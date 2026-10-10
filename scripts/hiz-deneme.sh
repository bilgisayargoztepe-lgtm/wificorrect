#!/usr/bin/env bash
# Kabul denemesi (spec §5): sahte istemciyle hız ölçümünün doğruluğu ve 5 Mb/sn yavaşlatma.
# Cihazda root olarak, MUTLAKA terminalle (pty) çalıştırın ki SSH koparsa SIGHUP gelsin ve temizlik çalışsın:
#   scp scripts/hiz-deneme.sh wificorrect:/root/ && ssh -t wificorrect bash /root/hiz-deneme.sh
#   ya da cihazda: systemd-run --pty --unit hiz-deneme bash /root/hiz-deneme.sh
# Temizlik çıktısı /root/hiz-deneme.log'a yazılır.
# Sahte oturum açılmaz (5651'e uydurma kişi girmez): test MAC'i geçici izinli cihaz olur ("Hız denemesi"),
# trafiği gerçek olduğu için kayıtlarda MAC'iyle görünür. Çıkışta (trap) ad alanı, izin ve sınır geri alınır;
# yarıda kalmış bir önceki denemenin artıkları da başta temizlenir.
# DİKKAT: netns süreçleri ayırmaz — ad alanında pkill kullanılmaz (bkz. sahte-istemci.sh); süreçler `ip netns pids` ile.
# Sessiz bir saatte çalıştırın: WAN'ı toplam ~100 sn doldurur (30 sn sınırsız + 40 sn 5 Mb/sn + 30 sn sınırsız).
# Çıkış kodu: hepsi PASS → 0, en az bir FAIL → 1, kurulum hatası → 2.
set -euo pipefail
export LC_ALL=C.UTF-8 # awk ondalık noktası

MAC=02:57:fc:00:00:01
AD='Hız denemesi'
NS=wfcdeneme
VH=wfcd0 # köprüdeki uç
VN=wfcd1 # ad alanındaki uç (test MAC'i)
KOPRU=br-hotspot
ROUTER=10.50.0.1
AYAR=${WFC_AYAR:-/etc/wificorrect/ayarlar.toml}
YEDEK=/root/ayarlar.toml.hiz-deneme
TRAFIK=/run/wificorrect/trafik.json
WFC=/usr/local/bin/wificorrect
URL='https://speed.cloudflare.com/__down?bytes='
SINIR_MB=5
# Aşama süreleri (sn). trafik.json hızı ≈ son 10 sn'nin ortalaması (5 sn'lik örnekler, 9 sn budama;
# bkz. src/trafik.rs PENCERE_SN); "kararlı" pencere indirmenin ilk KARARLI sn'sinden sonra başlar.
SURE1=30
SURE2=40
SURE3=30
KARARLI=10
GECICI=$(mktemp -d /tmp/hiz-deneme.XXXXXX)
GUNLUK=/root/hiz-deneme.log

hata() { echo "HATA: $*" >&2; exit 2; }

[ "$(id -u)" = 0 ] || hata "root olarak çalıştırın"
for k in ip nft tc curl dhcpcd awk flock systemctl timeout; do
    command -v "$k" >/dev/null || hata "$k bulunamadı"
done
[ -x "$WFC" ] || hata "$WFC yok"
[ -f "$AYAR" ] || hata "$AYAR yok"
ip link show "$KOPRU" >/dev/null 2>&1 || hata "$KOPRU yok"
exec 9>/run/hiz-deneme.kilit
flock -n 9 || hata "başka bir hiz-deneme çalışıyor"

# [main] içindeki anahtar (tek/çift tırnaklı); yoksa $2
ana_ayar() {
    local v
    v=$(awk -v k="$1" -v q="'" '
        /^[ \t]*\[/ { m = ($0 ~ /^[ \t]*\[main\][ \t]*$/) }
        m && $0 ~ "^[ \t]*" k "[ \t]*=" { sub(/^[^=]*=[ \t]*/, ""); gsub("^[\"" q "]|[\"" q "][ \t]*$", ""); print; exit }' "$AYAR")
    echo "${v:-$2}"
}
STATE=$(ana_ayar state_root /srv/hotspot/state)
SINIR=$STATE/hiz_sinir.json
KIRA=$(ana_ayar leases_file /var/lib/misc/dnsmasq.leases)
# yükleme yönünün HTB'si WAN'da (ag.toml üst düzey `wan`)
WAN=$(awk -v q="'" '
    /^[ \t]*\[/ { exit }
    $0 ~ "^[ \t]*wan[ \t]*=" { sub(/^[^=]*=[ \t]*/, ""); gsub("^[\"" q "]|[\"" q "][ \t]*$", ""); print; exit }' \
    /etc/wificorrect/ag.toml 2>/dev/null) || true
[ -n "$WAN" ] || hata "/etc/wificorrect/ag.toml içinde wan yok"
SINIR_VARDI=0
[ -e "$SINIR" ] && SINIR_VARDI=1

# $1 hedef, sonrası üretici komut: çıktısı atomik olarak $1'in yerine geçer (izin/sahip korunur).
# Komut başarısızsa ya da çıktı boşsa hedefe dokunulmaz (boş ayarlar.toml varsayılanlarla açılır, boş sınır dosyası herkesi serbest bırakır).
yaz() {
    local h=$1 t="$1.hiz-deneme.tmp"
    shift
    if [ -e "$h" ]; then cp -p "$h" "$t" || return 1; else (umask 077 && : >"$t") || return 1; fi
    if "$@" >"$t" && [ -s "$t" ] && mv -f "$t" "$h"; then return 0; fi
    rm -f "$t"
    echo "HATA: $h yazılamadı, dokunulmadı" >&2
    return 1
}

ek_metni() { printf "\n[[allow]]\nmac = '%s'\nname = '%s'\n" "$MAC" "$AD"; }
# Panelin yazdığı dosyada üst düzey `allow = []` olabilir; yanına [[allow]] tablosu eklemek anahtarı yeniden tanımlar
# (geçersiz TOML). Boş dizi satırı çıkarılır (Config #[serde(default)]: anahtar yoksa boş liste).
# $1 dosyanın test bloğu eklenmiş hâli
ekli_hali() { awk '!/^allow = \[\][[:space:]]*$/' "$1" && ek_metni; }
ayar_ekli() { ekli_hali "$AYAR"; }
# Dolu satır içi dizi (`allow = [ {...} ]`) elle düzenlenmez
inline_allow_dolu() { grep -v '^allow = \[\][[:space:]]*$' "$AYAR" | grep -Eq '^[[:space:]]*allow[[:space:]]*='; }

# ayarlar.toml'dan test MAC'inin [[allow]] tablosu çıkmış hâli (panel dosyayı yeniden yazmış olsa da)
ayar_temiz_hali() {
    awk -v mac="$MAC" -v q="'" '
        BEGIN { re = "^[ \t]*mac[ \t]*=[ \t]*[\"" q "]" mac "[\"" q "]" }
        function bosalt() { if (!(baslik ~ /^[ \t]*\[\[allow\]\]/ && sil)) printf "%s", tampon; tampon = ""; sil = 0 }
        /^[ \t]*\[/ { bosalt(); baslik = $0 }
        { tampon = tampon $0 "\n"; if (tolower($0) ~ re) sil = 1 }
        END { bosalt() }' "$AYAR"
}

# Yedek + eklediğimiz blok dışında değişiklik yoksa yedeği aynen geri koyar; varsa yalnızca bloğu çıkarır
ayar_geri() {
    if [ -f "$YEDEK" ] && cmp -s <(ekli_hali "$YEDEK") "$AYAR"; then
        yaz "$AYAR" cat "$YEDEK"
    else
        echo "UYARI: $AYAR deneme sırasında değişmiş; yalnızca test bloğu çıkarılıyor (yedek: $YEDEK)" >&2
        yaz "$AYAR" ayar_temiz_hali
    fi
}

# Bozuk ayar kaydediciyi (5651) durdurur: ctl bilinmeyen komutta ayar okunabildiyse 2, okunamadıysa 1 döner
ayar_gecerli() {
    local rc=0
    "$WFC" ctl __ayar-denetimi >/dev/null 2>&1 || rc=$?
    [ "$rc" -eq 2 ]
}

# hiz_sinir.json (serde pretty) içine / içinden test MAC'inin kaydı
sinir_ekli_hali() {
    local z blok
    z=$(TZ=Etc/GMT-3 date +%Y-%m-%dT%H:%M:%S+03:00)
    blok=$(printf '  "%s": {\n    "hiz": %s,\n    "ad": "%s",\n    "zaman": "%s",\n    "kim": "hiz-deneme"\n  }' "$MAC" "$SINIR_MB" "$AD" "$z")
    if [ -s "$SINIR" ] && grep -q '"' "$SINIR"; then
        BLOK=$blok awk 'NR == 1 && sub(/^[ \t]*\{/, "") { print "{"; print ENVIRON["BLOK"] ","; if ($0 ~ /[^ \t]/) print; next } { print }' "$SINIR"
    else
        printf '{\n%s\n}\n' "$blok"
    fi
}
sinir_ekle() { yaz "$SINIR" sinir_ekli_hali; }

sinir_temiz_hali() {
    awk -v mac="$MAC" '
        { l[NR] = $0 }
        END {
            for (i = 1; i <= NR; i++) {
                if (!atla && index(l[i], "\"" mac "\": {")) { atla = 1; continue }
                if (atla) {
                    if (l[i] ~ /^  \},?[ \t]*$/) { atla = 0; if (l[i] !~ /,/ && n) sub(/,[ \t]*$/, "", o[n]) }
                    continue
                }
                o[++n] = l[i]
            }
            for (i = 1; i <= n; i++) print o[i]
        }' "$SINIR"
}
sinir_kaldir() {
    yaz "$SINIR" sinir_temiz_hali || return 1
    if grep -qi "\"$MAC\"" "$SINIR"; then
        echo "UYARI: $SINIR içinden test sınırı çıkarılamadı, elle silin" >&2
        return 1
    elif [ "$SINIR_VARDI" = 0 ] && ! grep -q '"' "$SINIR"; then
        rm -f "$SINIR"
    fi
}

# Her şeyi geri alır; durum kontrolüyle çalışır (idempotent). Başta artık temizliği, sonda trap kullanır.
TEMIZLENDI=0
temizle() {
    [ "$TEMIZLENDI" = 1 ] && return 0
    set +e
    local yeniden=0
    if [ -n "${INDIR_PID:-}" ]; then kill "$INDIR_PID" 2>/dev/null; fi
    if ip netns list | grep -qw "$NS"; then
        ip netns pids "$NS" | xargs -r kill 2>/dev/null
        sleep 1
        ip netns pids "$NS" | xargs -r kill -9 2>/dev/null
        if [ -n "${IP:-}" ] && command -v dhcp_release >/dev/null; then dhcp_release "$KOPRU" "$IP" "$MAC"; fi
        ip netns del "$NS"
    fi
    ip link del "$VH" 2>/dev/null
    rm -rf "/etc/netns/$NS"
    if [ -f "$SINIR" ] && grep -qi "\"$MAC\"" "$SINIR"; then
        sinir_kaldir
        "$WFC" ctl hiz-uygula
    fi
    nft delete element inet hotspot allow_mac "{ $MAC }" 2>/dev/null
    if grep -qi "$MAC" "$AYAR"; then
        ayar_geri
        yeniden=1
    fi
    if [ "$yeniden" = 1 ]; then
        # Bozuk ayarla kaydedici (5651) açılmaz: önce yedek denenir, okunamıyorsa kaydediciye dokunulmaz
        if ! ayar_gecerli && [ -f "$YEDEK" ]; then
            echo "UYARI: geri alınan $AYAR okunamıyor; yedek ($YEDEK) geri konuyor" >&2
            yaz "$AYAR" cat "$YEDEK"
        fi
        if ayar_gecerli; then
            systemctl restart wificorrect-kaydedici
        else
            echo "!!! HATA: $AYAR okunamıyor; wificorrect-kaydedici YENİDEN BAŞLATILMADI (eski ayarla çalışıyor). Yedek: $YEDEK" >&2
        fi
        grep -qi "$MAC" "$AYAR" && echo "!!! HATA: test MAC'i hâlâ $AYAR içinde; elle çıkarıp kaydediciyi yeniden başlatın" >&2
    fi
    rm -rf "$GECICI"
    set -e
}
cikis() {
    # Temizlik ikinci bir Ctrl-C ya da kopan SSH (SIGPIPE/SIGHUP) ile yarıda kalmasın; çıktı günlüğe
    trap '' INT TERM HUP PIPE
    echo "Temizleniyor (günlük: $GUNLUK)" 2>/dev/null || true
    exec >>"$GUNLUK" 2>&1
    echo "--- $(date '+%F %T') temizlik ---"
    temizle
    TEMIZLENDI=1
    echo "Temizlendi: ad alanı, izin, test sınırı."
}

# --- trafik.json okuma (sıkıştırılmış JSON, cihaz nesneleri iç içe değil) ---
trafik() { cat "$TRAFIK" 2>/dev/null || true; }
zaman_of() { grep -o '"zaman":[0-9.]*' <<<"$1" | cut -d: -f2 || true; }
alan() { # $1 json, $2 alan → test MAC'inin değeri (yoksa 0)
    local v
    v=$(grep -o "\"$MAC\":{[^}]*}" <<<"$1" | grep -o "\"$2\":[0-9]*" | head -1 | cut -d: -f2) || true
    echo "${v:-0}"
}
diger_en() { # $1 json → "bps mac": test MAC'i dışındaki en yüksek indirme
    local v
    v=$(grep -o '"[0-9a-f:]\{17\}":{[^}]*}' <<<"$1" | grep -v "^\"$MAC\"" |
        sed 's/^"\([^"]*\)".*"indir_bps":\([0-9]*\).*/\2 \1/' | sort -n | tail -1) || true
    echo "${v:-0 -}"
}
mb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1e6 }'; }

# trafik.json'un $1 kez yenilenmesini bekler (her biri en çok 30 sn)
yenilenme_bekle() {
    local i son z t
    for ((i = 0; i < $1; i++)); do
        son=$(zaman_of "$(trafik)")
        for ((t = 0; t < 30; t++)); do
            sleep 1
            z=$(zaman_of "$(trafik)")
            [ -n "$z" ] && [ "$z" != "$son" ] && break
        done
        [ "$t" -lt 30 ] || hata "$TRAFIK 30 sn'dir yenilenmiyor (kaydedici çalışıyor mu?)"
    done
}

# Köprü ucunun sayaçları: "tx rx" (tx = istemcinin indirdiği, rx = yüklediği; Ethernet başlığı dahil).
# trafik.json'dan bağımsız ikinci bayt sayımı.
veth() {
    local tx rx
    tx=$(cat "/sys/class/net/$VH/statistics/tx_bytes" 2>/dev/null) || tx=0
    rx=$(cat "/sys/class/net/$VH/statistics/rx_bytes" 2>/dev/null) || rx=0
    echo "${tx:-0} ${rx:-0}"
}

# $1 etiket, $2 süre bütçesi (sn). 99 MB'lık istekler arka arkaya (Cloudflare ≥100 MB'ı 403'le reddediyor).
# trafik.json her yenilendiğinde bir satır: z o indir_bps veth_tx veth_rx diğer_bps diğer_mac (z: ölçüm, o: okuma zamanı)
# Sonuç: ORNEK, BASLA, BITIS, CURL_BAYT, CURL_MB (toplam bayt × 8 / toplam süre; yalnızca bilgi)
INDIR_PID=
indir() {
    local istek="$GECICI/istek.$1" son="" z j d v o bitis
    ORNEK="$GECICI/ornek.$1"
    : >"$ORNEK"
    : >"$istek"
    echo "== $1: $2 sn indirme (99 MB'lık istekler) =="
    BASLA=$(date +%s.%N)
    bitis=$(awk -v b="$BASLA" -v s="$2" 'BEGIN { printf "%.3f", b + s }')
    (
        while :; do
            kalan=$(awk -v s="$bitis" -v n="$(date +%s.%N)" 'BEGIN { printf "%d", s - n }')
            [ "$kalan" -ge 3 ] || break
            rc=0
            r=$(ip netns exec "$NS" curl -4 -s -o /dev/null --max-time "$kalan" \
                -w '%{http_code} %{size_download} %{time_total}' "${URL}99000000") || rc=$?
            echo "${r:-000 0 0} $rc" >>"$istek"
            [ "$rc" = 0 ] || break
        done
    ) &
    INDIR_PID=$!
    while kill -0 "$INDIR_PID" 2>/dev/null; do
        sleep 0.5
        j=$(trafik)
        z=$(zaman_of "$j")
        if [ -z "$z" ] || [ "$z" = "$son" ]; then continue; fi
        son=$z
        o=$(date +%s.%N)
        v=$(veth)
        d=$(diger_en "$j")
        echo "$z $o $(alan "$j" indir_bps) $v $d" >>"$ORNEK"
        printf '  %s  test ↓ %6s Mb/sn | diğerlerinin en yükseği %s Mb/sn (%s)\n' \
            "$(date +%T)" "$(mb "$(alan "$j" indir_bps)")" "$(mb "${d%% *}")" "${d#* }"
    done
    wait "$INDIR_PID" || true
    INDIR_PID=
    BITIS=$(date +%s.%N)
    d=$(istek_hatasi "$istek")
    [ -z "$d" ] || hata "$1: indirme isteği başarısız ($d)"
    read -r CURL_BAYT CURL_MB < <(curl_ozet "$istek")
    [ "$CURL_BAYT" -gt 0 ] || hata "$1: hiç bayt indirilmedi"
    echo "  curl: $(wc -l <"$istek") istek, $(mb "$CURL_BAYT") MB, ortalama $CURL_MB Mb/sn"
}

# istek satırları "http_kodu bayt süre curl_çıkışı" → ilk hatalı istek (yoksa boş). Bütçe sonundaki süre aşımı (28)
# hata sayılmaz; başlık gelmeden kesildiyse kod 000 olur.
istek_hatasi() {
    awk '!($4 == 0 && $1 == 200) && !($4 == 28 && ($1 == 200 || $1 == "000")) { print "HTTP " $1 ", curl çıkışı " $4; exit }' "$1"
}
# istek satırları → "toplam_bayt Mb/sn"
curl_ozet() { awk '{ b += $2; t += $3 } END { printf "%d %.1f\n", b, (t > 0 ? b * 8 / t / 1e6 : 0) }' "$1"; }

# $1 örnek dosyası, $2 pencere indirme başından en az kaç sn sonra başlasın.
# trafik.json hızı ≈ son iki adımın (≈10 sn) ortalaması: her örnek iki önceki örnekle eşlenir; veth tx hızı aynı
# aralıkta, kendi okuma zamanlarımızla hesaplanır. Yalnızca [BASLA+$2, BITIS] içinde kalan pencereler sayılır;
# atlanmış yenilemeli çift (aralık 9–11 sn dışında) eşlenmez. Çıktı: "adet trafik_Mb veth_Mb ort_mutlak_fark_%"
analiz() {
    awk -v b="$BASLA" -v e="$BITIS" -v ofs="$2" '
        { z[NR] = $1; o[NR] = $2; t[NR] = $3; tx[NR] = $4 }
        END {
            for (i = 3; i <= NR; i++) {
                k = i - 2
                if (z[k] < b + ofs || z[i] > e || z[i] - z[k] < 9 || z[i] - z[k] > 11 || o[i] <= o[k]) continue
                vr = (tx[i] - tx[k]) * 8 / (o[i] - o[k])
                if (vr <= 0) continue
                d = (t[i] - vr) / vr * 100
                if (d < 0) d = -d
                st += t[i]; sv += vr; sd += d; n++
            }
            if (n) printf "%d %.1f %.1f %.1f\n", n, st / n / 1e6, sv / n / 1e6, sd / n
            else print "0 0 0 0"
        }' "$1"
}

# awk koşulu, değişkenler a=… biçiminde → 1/0. Boş ya da sayı olmayan girdi → 0 (awk sözdizimi hatası olmaz).
kosul() {
    local ifade=$1 x
    local -a arg=()
    shift
    for x in "$@"; do
        [[ ${x#*=} =~ ^-?[0-9]+([.][0-9]+)?$ ]] || { echo 0; return 0; }
        arg+=(-v "$x")
    done
    awk "${arg[@]}" "BEGIN { print (($ifade) ? 1 : 0) }"
}
# awk ifadesinin değeri (%.3f); girdi sayı değilse "-"
hesap() {
    local ifade=$1 x
    local -a arg=()
    shift
    for x in "$@"; do
        [[ ${x#*=} =~ ^-?[0-9]+([.][0-9]+)?$ ]] || { echo -; return 0; }
        arg+=(-v "$x")
    done
    awk "${arg[@]}" "BEGIN { printf \"%.3f\", $ifade }"
}

# Yavaşlatma sınıflandırması: "yön ip hız" satırları, sıralı (lan = indirme, wan = yükleme).
# nft `inet wfc_hiz sinif` zincirindeki IP → sınıf, sınıfın hızı ilgili arayüzün `tc class show` çıktısından.
siniflar() {
    nft list chain inet wfc_hiz sinif >"$GECICI/nft" 2>/dev/null || : >"$GECICI/nft"
    tc class show dev "$KOPRU" >"$GECICI/tc.lan" 2>/dev/null || : >"$GECICI/tc.lan"
    tc class show dev "$WAN" >"$GECICI/tc.wan" 2>/dev/null || : >"$GECICI/tc.wan"
    awk '
        FILENAME ~ /tc\.(lan|wan)$/ { y = substr(FILENAME, length(FILENAME) - 2); for (i = 1; i < NF; i++) if ($i == "rate") r[y " " $3] = $(i + 1); next }
        /meta priority set/ {
            ip = ""
            for (i = 1; i < NF; i++) {
                if ($i == "daddr") { y = "lan"; ip = $(i + 1) }
                if ($i == "saddr") { y = "wan"; ip = $(i + 1) }
                if ($i == "set") c = $(i + 1)
            }
            if (ip != "") print y, ip, ((y " " c) in r ? r[y " " c] : "?")
        }' "$GECICI/tc.lan" "$GECICI/tc.wan" "$GECICI/nft" | sort
}

# --- Başlangıç: önceki denemenin artıkları ---
temizle
GECICI=$(mktemp -d /tmp/hiz-deneme.XXXXXX)
systemctl is-active -q wificorrect-kaydedici || hata "wificorrect-kaydedici çalışmıyor"
[ -f "$TRAFIK" ] || hata "$TRAFIK yok (hız ölçümü kurulu mu?)"
trap cikis EXIT
trap 'exit 130' INT TERM HUP PIPE

# --- Sahte istemci: netns + veth, köprüye bağlı, DHCP ---
echo "== Sahte istemci kuruluyor ($MAC) =="
ip netns add "$NS"
ip link add "$VH" type veth peer name "$VN"
ip link set "$VN" netns "$NS"
ip -n "$NS" link set "$VN" address "$MAC"
# Köprünün MAC'i sabitlenmemişse en küçük port MAC'ini alır: büyük bir MAC verilir ve değişmediği doğrulanır
# (ağ geçidi MAC'i değişirse misafirlerin trafiği kara deliğe düşer)
ip link set "$VH" address fe:57:fc:00:00:00
KOPRU_MAC=$(cat "/sys/class/net/$KOPRU/address")
ip link set "$VH" master "$KOPRU" up
[ "$(cat "/sys/class/net/$KOPRU/address")" = "$KOPRU_MAC" ] || hata "$KOPRU MAC'i değişti ($KOPRU_MAC → $(cat "/sys/class/net/$KOPRU/address")); veth çıkarılıyor"
ip -n "$NS" link set lo up
ip -n "$NS" link set "$VN" up
mkdir -p "/etc/netns/$NS"
echo "nameserver $ROUTER" >"/etc/netns/$NS/resolv.conf"
# -1: kira alınca çıkar; -C resolv.conf/hostname: cihazın kendi dosyalarına dokunmaz
timeout 30 ip netns exec "$NS" dhcpcd -1 -4 -B -C resolv.conf -C hostname -h hiz-deneme "$VN" >/dev/null 2>&1 || true
IP=$(ip -n "$NS" -4 -o addr show dev "$VN" | awk '{ split($4, a, "/"); print a[1]; exit }')
[ -n "$IP" ] || hata "DHCP'den IP alınamadı"
grep -qi "$MAC" "$KIRA" || hata "$KIRA içinde $MAC kirası yok"
echo "  IP: $IP"

# --- Geçici izin: ayarlar.toml (tabloda görünsün) + allow_mac ---
inline_allow_dolu && hata "$AYAR içinde dolu satır içi 'allow = [...]' var; test bloğu eklenemez (önce panelden kaydedin ya da elle [[allow]] tablolarına çevirin)"
cp -p "$AYAR" "$YEDEK"
yaz "$AYAR" ayar_ekli
ayar_gecerli || hata "test bloğu eklenince $AYAR okunamadı"
nft add element inet hotspot allow_mac "{ $MAC }"
systemctl restart wificorrect-kaydedici
sleep 2
systemctl is-active -q wificorrect-kaydedici || hata "wificorrect-kaydedici yeniden başlamadı"
ip netns exec "$NS" curl -4 -s -o /dev/null --max-time 15 "${URL}1000" || hata "sahte istemciden internete çıkılamadı"
# IP sayaç setine girsin ve ölçer onu bir kez görsün (ilk örnek fark sayılmaz)
yenilenme_bekle 2
j=$(trafik)
grep -q "\"$MAC\"" <<<"$j" || hata "test MAC'i $TRAFIK içinde görünmüyor"
if grep -q '"hiz_hata":"' <<<"$j"; then echo "  UYARI: $(grep -o '"hiz_hata":"[^"]*"' <<<"$j")"; fi

# --- 1. Sınırsız indirme: ölçüm doğruluğu ---
ONCE_BAYT=$(alan "$j" bugun_bayt)
read -r V0_TX V0_RX < <(veth)
echo "  bugün (önce): $(mb "$ONCE_BAYT") MB"
indir "1-sinirsiz" "$SURE1"
read -r A1_N A1_T A1_V A1_FARK < <(analiz "$ORNEK" 0)
read -r K1_N K1_T _ _ < <(analiz "$ORNEK" "$KARARLI")
C1_MB=$CURL_MB

# --- 2. 5 Mb/sn sınır ---
echo "== $SINIR_MB Mb/sn sınır yazılıyor =="
siniflar >"$GECICI/s0"
printf 'lan %s %sMbit\nwan %s %sMbit\n' "$IP" "$SINIR_MB" "$IP" "$SINIR_MB" | cat - "$GECICI/s0" | sort >"$GECICI/beklenen"
echo "  önceden sınırlı: $(awk '$1 == "lan"' "$GECICI/s0" | wc -l) IP"
sinir_ekle
"$WFC" ctl hiz-uygula
siniflar >"$GECICI/s2a"
if tc class show dev "$KOPRU" | grep -q "rate ${SINIR_MB}Mbit"; then
    echo "  tc: $KOPRU üzerinde ${SINIR_MB}Mbit sınıfı var"
else
    echo "  UYARI: $KOPRU üzerinde ${SINIR_MB}Mbit sınıfı görünmüyor"
fi
sleep 2
indir "2-sinirli" "$SURE2"
read -r K2_N K2_T K2_V _ < <(analiz "$ORNEK" "$KARARLI")
siniflar >"$GECICI/s2b"
C2_MB=$CURL_MB
D2=$(awk '$6 > m { m = $6; a = $7 } END { printf "%.1f %s", m / 1e6, (a ? a : "-") }' "$ORNEK")

# --- 3. Sınır kaldırıldı ---
echo "== Sınır kaldırılıyor =="
sinir_kaldir
"$WFC" ctl hiz-uygula
siniflar >"$GECICI/s3"
sleep 2
indir "3-kaldirildi" "$SURE3"
read -r K3_N K3_T _ _ < <(analiz "$ORNEK" "$KARARLI")
C3_MB=$CURL_MB
B3=$BITIS

# Bugün MB: indirmeler bittikten sonraki ilk trafik.json okuması tüm baytları içerir
for ((t = 0; t < 30; t++)); do
    j=$(trafik)
    [ "$(kosul 'z > b + 1' z="$(zaman_of "$j")" b="$B3")" = 1 ] && break
    sleep 1
done
SONRA_BAYT=$(alan "$j" bugun_bayt)
read -r V1_TX V1_RX < <(veth)
echo "  bugün (sonra): $(mb "$SONRA_BAYT") MB"

# --- Özet ---
SONUC=0
sonuc() { # $1 açıklama, $2 1 = PASS
    if [ "$2" = 1 ]; then echo "PASS  $1"; else echo "FAIL  $1"; SONUC=1; fi
}
echo
echo "== Özet (Mb/sn; kararlı = penceresi indirmenin ilk $KARARLI sn'sinden sonra başlayan örnekler) =="
sonuc "1. Ölçüm: trafik.json $A1_T, aynı pencerelerde veth $A1_V, ort. mutlak fark %$A1_FARK ($A1_N pencere; ≤%10)" \
    "$(kosul 'n >= 1 && f <= 10' n="$A1_N" f="$A1_FARK")"
if [ "$(kosul 'n >= 1 && t < 2 * s' n="$K1_N" t="$K1_T" s="$SINIR_MB")" = 1 ]; then
    echo "UYARI: sınırsız hız ($K1_T Mb/sn) sınırın iki katından düşük; yavaşlatma denemesi anlamlı değil"
fi
sonuc "2. Yavaşlatma: kararlı trafik.json $K2_T, veth $K2_V ($K2_N pencere), hedef $SINIR_MB ±%15" \
    "$(kosul 'n >= 1 && t >= s * 0.85 && t <= s * 1.15 && v >= s * 0.85 && v <= s * 1.15' n="$K2_N" t="$K2_T" v="$K2_V" s="$SINIR_MB")"
sonuc "3. Kaldırınca: kararlı trafik.json $K3_T ($K3_N pencere) ≥ 0,8 × sınırsız $K1_T ($K1_N pencere) ve > $((SINIR_MB * 2))" \
    "$(kosul 'n3 >= 1 && n1 >= 1 && t3 >= 0.8 * t1 && t3 > 2 * s' n3="$K3_N" n1="$K1_N" t3="$K3_T" t1="$K1_T" s="$SINIR_MB")"
# veth Ethernet başlığını da sayar, trafik.json IP baytını → oran 1'in biraz altında olabilir
ORAN=-
VARTIS=$(hesap 'c + r - d - x' c="$V1_TX" r="$V1_RX" d="$V0_TX" x="$V0_RX")
if [ "$(kosul 'v > 0' v="$VARTIS")" = 1 ]; then
    ORAN=$(hesap '(a - b) / v' a="$SONRA_BAYT" b="$ONCE_BAYT" v="$VARTIS")
fi
BARTIS=$(hesap 'a - b' a="$SONRA_BAYT" b="$ONCE_BAYT")
if [ "$ORAN" = - ]; then
    sonuc "4. Bugün MB: girdi eksik (bugün önce='$ONCE_BAYT' sonra='$SONRA_BAYT', veth artışı='$VARTIS')" 0
else
    sonuc "4. Bugün MB: +$(mb "$BARTIS") MB, veth (indir+yükle) +$(mb "$VARTIS") MB, oran $ORAN (0,97–1,15)" \
        "$(kosul 'o >= 0.97 && o <= 1.15' o="$ORAN")"
fi
echo "Bilgi: curl ortalaması (TCP/TLS açılışı dahil) — sınırsız $C1_MB, sınırlı $C2_MB, kaldırınca $C3_MB Mb/sn"
# 5. Diğer müşteriler: sınırlı küme yalnızca test IP'si kadar büyür, önceden sınırlıların hızı aynı kalır, kaldırınca eskiye döner
K5=1
grep -q " $IP " "$GECICI/s0" && K5=0
for s in s2a s2b; do
    if ! cmp -s "$GECICI/$s" "$GECICI/beklenen"; then
        K5=0
        echo "  $s beklenenden farklı:"
        diff "$GECICI/beklenen" "$GECICI/$s" | sed 's/^/    /' || true
    fi
done
if ! cmp -s "$GECICI/s3" "$GECICI/s0"; then
    K5=0
    echo "  kaldırınca sınıflar eskiye dönmedi:"
    diff "$GECICI/s0" "$GECICI/s3" | sed 's/^/    /' || true
fi
sonuc "5. Diğerleri etkilenmedi: önceden $(awk '$1 == "lan"' "$GECICI/s0" | wc -l) sınırlı IP aynı hızda, yalnızca test IP'si ($IP) eklendi ve kaldırıldı" "$K5"
echo "Bilgi: 2. aşamada diğer cihazların en yüksek indirmesi: ${D2% *} Mb/sn (${D2#* })"
exit "$SONUC"
