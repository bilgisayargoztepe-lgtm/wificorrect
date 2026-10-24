#!/usr/bin/env bash
# Sahte PPPoE operatörü (spec docs/superpowers/specs/2026-10-23-pppoe-design.md, "Test ve kabul"): cihazın kendi içinde,
# ağ ad alanlarında pppoe-server (operatör) ile gerçek pppd (cihaz tarafı; bağlantı dosyası `ctl pppoe-peers` üretir).
# Cihazda root olarak; ppp ve pppoe paketleri kurulu olmalı (pppoe yalnızca bu deneme için, paketin Depends'ine girmez):
#   scp scripts/pppoe-deneme.sh wificorrect:/root/ && ssh wificorrect bash /root/pppoe-deneme.sh   (hepsi ~4,5 dk)
#   Yalnızca bazı senaryolar: bash /root/pppoe-deneme.sh bagli vlan   (adlar: aşağıda TUMU)
#   Kurulu sürümde `ctl pppoe-peers` yoksa: WFC=/root/rza/target/debug/wificorrect bash /root/pppoe-deneme.sh
#   Güvenlik duvarı kuralları (nft_kurallari, mss) varsayılan olarak kurulu sürümden (/etc/wificorrect); dalınkiler için
#   (scripts/test.sh /root/rza'ya gönderir): KURALLAR=/root/rza/deploy/debian/etc/wificorrect
#   Yarıda kalmış bir denemenin artıkları: bash /root/pppoe-deneme.sh --temizle
# Canlı ağa dokunulmaz:
# - Her şey wfc-isp (operatör), wfc-cpe (cihaz) ve wfc-lan (MSS denemesinde misafir) ad alanlarında. Sanal kablolar (veth)
#   ad alanının içinde doğar, kök ad alanında hiç görünmez; cihazın arayüzlerine (enp*, br-hotspot, wfc) dokunulmaz.
#   Kök ad alanında yalnızca okuma (ip, nft list) ve ad alanı yönetimi (ip netns add/del/pids) yapılır; güvenlik duvarı
#   kurallarının denetimi (nft -c) de cihaz ad alanında.
# - /etc/ppp iki tarafta da yalıtılır: /etc/netns/<ad alanı>/ppp, `ip netns exec` ile /etc/ppp'nin üstüne bağlanır (sistemin
#   /etc/ppp'si, ip-up.d kancaları kullanılmaz); /etc/resolv.conf da aynı yolla örtülür, cihaz tarafının dosyasından
#   `usepeerdns` satırı çıkarılır. pppd ve pppoe-server /run'ın üstüne kendi tmpfs'ini bağlar (pid dosyaları, pppd
#   veritabanı, kilitler kök /run'a yazılmaz). Başlatmadan hemen önce ağ ad alanı, bağlama ad alanı ve iki örtü
#   denetlenir; biri eksikse süreç başlatılmaz.
# - Durdurulan süreçler yalnızca bu betiğin ad alanlarındakiler (`ip netns pids`) ve yalnızca bilinen adlar (pppd,
#   pppoe-server, timeout, bash…). Ad alanında pkill/killall yok: netns süreçleri ayırmaz (bkz. sahte-istemci.sh).
# - Başta ve sonda kök ad alanının ip addr/route/rule/link çıktısı, nft kural kümesi (misafirlerle değişen küme öğeleri
#   hariç), /etc/resolv.conf, /etc/ppp ve ip_forward karşılaştırılır (kok_ad_alani).
# Çıkış kodu: hepsi PASS → 0, en az bir FAIL → 1, kurulum hatası → 2. Günlükler /root/pppoe-deneme/ altında kalır (deneme
# değerleri; parolanın hiçbir günlükte olmadığı denetlenir). ornek/ altındakiler bağlantı kurulduğu / kimlik reddedildiği /
# PADO beklemesi bittiği anda alınan pppd günlükleridir (betiğin kendi durdurma satırları yok): tests/fixtures/pppoe/.
set -euo pipefail
export LC_ALL=C.UTF-8

WFC=${WFC:-/usr/local/bin/wificorrect}
KURALLAR=${KURALLAR:-/etc/wificorrect} # guvenlik.nft ve hiz.nft (yalnızca okunur; denetim cihaz ad alanında)
CIKTI=/root/pppoe-deneme
ISP=wfc-isp
CPE=wfc-cpe
LAN=wfc-lan
V_ISP=wfcisp0     # operatörün ucu (wfc-isp)
V_CPE=wfccpe0     # cihazın internet portunun yerine (wfc-cpe)
V_CPE_LAN=wfccpe1 # MSS denemesi: cihazın misafir tarafı (wfc-cpe)
V_LAN=wfclan0     # MSS denemesi: misafir (wfc-lan)
MAC_ISP=02:57:46:43:00:01
MAC_CPE=02:57:46:43:00:02
MAC_CPE_LAN=02:57:46:43:00:03
MAC_LAN=02:57:46:43:00:04
PPP=wfcppp # src/pppoe.rs ARAYUZ
# Deneme değerleri (gerçek hesap değil)
KULLANICI=wfcdeneme@deneme
PAROLA=Dogru-Parola-123
YANLIS=Yanlis-Parola-456
BRAS=198.51.100.1 # operatörün PPP adresi; RFC 5737 belgeleme adresleri
HAVUZ=203.0.113.10
LAN_AG=192.0.2 # MSS denemesinde misafir ağı (RFC 5737)
VLAN=35
ONCELIK=3
AC=wfc-ac
SERVIS=wfc-servis
# deploy/debian/etc/wificorrect/guvenlik.nft, chain forward'ın ilk kuralı (KURALLAR'daki dosyada varsa o kullanılır)
MSS_VARSAYILAN='tcp flags syn / syn,rst tcp option maxseg size set rt mtu'
ISARET=wfc-pppoe-deneme # yalıtım dosyalarının işareti

hata() {
    echo "HATA: $*" >&2
    exit 2
}

[ "$(id -u)" = 0 ] || hata "root olarak çalıştırın"
for k in ip nft pppd pppoe-server timeout flock awk sed grep diff md5sum mount readlink; do
    command -v "$k" >/dev/null || hata "$k bulunamadı (ppp ve pppoe paketleri kurulu mu?)"
done
exec 9>/run/wfc-pppoe-deneme.kilit
flock -n 9 || hata "başka bir pppoe-deneme çalışıyor"

ETC_ISP=/etc/netns/$ISP
ETC_CPE=/etc/netns/$CPE
IS= # geçici çalışma klasörü (bağlantı dosyası parolalı; çıkışta silinir)
ISTEMCI_PID=
SUNUCU_PID=
GUNLUK=
GECEN=0
KALAN=0
OZET=()

# ------------------------------------------------------------------ temizlik
# Bu betiğin ad alanındaki süreçler: önce TERM (en çok 10 sn), kalan KILL. Yalnızca bilinen adlar; başka bir süreç
# bulunursa (olmamalı) dokunulmaz, yazılır.
ns_surec_durdur() {
    local ns=$1 isaret p c i
    for isaret in TERM KILL; do
        local var=0
        for p in $(ip netns pids "$ns" 2>/dev/null); do
            c=$(cat "/proc/$p/comm" 2>/dev/null) || continue
            case $c in
                pppd | pppoe-server | pppoe | timeout | bash | sh | sleep)
                    kill "-$isaret" "$p" 2>/dev/null || true
                    var=1
                    ;;
                *) echo "UYARI: $ns içinde beklenmeyen süreç $p ($c); dokunulmadı" ;;
            esac
        done
        [ "$var" = 1 ] || return 0
        for ((i = 0; i < 50; i++)); do
            [ -n "$(ip netns pids "$ns" 2>/dev/null)" ] || return 0
            sleep 0.2
        done
    done
}

ad_alani_var() { ip netns list 2>/dev/null | awk '{ print $1 }' | grep -qx "$1"; }

# Kendi başlattığımız süreç hâlâ çalışıyor mu (bitmiş ama beklenmemiş çocuk "Z" görünür)
yasiyor() {
    local s
    s=$(sed 's/.*) //' "/proc/$1/stat" 2>/dev/null | cut -d' ' -f1) || return 1
    [ -n "$s" ] && [ "$s" != Z ]
}

temizle() {
    set +e
    local ns p
    for ns in "$LAN" "$CPE" "$ISP"; do
        if ad_alani_var "$ns"; then
            ns_surec_durdur "$ns"
            # PPP arayüzü pppd ile gider; kalmışsa ad alanı silinmeden söylenir (kök ad alanına taşınmamalı)
            if [ -n "$(ip -n "$ns" -o link show type ppp 2>/dev/null)" ]; then
                echo "UYARI: $ns içinde PPP arayüzü kaldı: $(ip -n "$ns" -o link show type ppp | cut -d: -f2 | tr -d ' ')"
            fi
            ip netns del "$ns"
        fi
        rm -rf "/etc/netns/$ns"
    done
    # ad alanıyla birlikte bitmiş olmalılar; yalnızca bitmişse beklenir (asılı kalınmasın)
    for p in $ISTEMCI_PID $SUNUCU_PID; do
        yasiyor "$p" || wait "$p" 2>/dev/null
    done
    ISTEMCI_PID=
    SUNUCU_PID=
    if [ -n "$IS" ] && [ -d "$IS" ]; then
        rm -rf "$IS"
    fi
    IS=
    set -e
}

if [ "${1:-}" = --temizle ]; then
    temizle
    echo "Temizlendi: $LAN $CPE $ISP ad alanları, /etc/netns/wfc-*."
    exit 0
fi
TUMU=(bagli kimlik_hatasi kimlik_hatasi_pap sunucu_yok vlan ac_servis kimlik_yontemi yeniden_baglanma hat_kopmasi mss nft_kurallari)
SECILEN=("$@")
[ $# -gt 0 ] || SECILEN=("${TUMU[@]}")
for s in "${SECILEN[@]}"; do
    [[ " ${TUMU[*]} " == *" $s "* ]] || hata "kullanım: $0 [--temizle | senaryo…]; senaryolar: ${TUMU[*]}"
done

# ------------------------------------------------------------------ kök ad alanı (yalnızca okuma)
# nft: misafir girişleri ve hız sayaçları küme öğelerini (elements) değiştirir; karşılaştırmada öğe satırları atlanır.
nft_kurallar() { nft -s list ruleset | awk '/elements = \{/ { if ($0 !~ /\}/) atla = 1; next } atla { if ($0 ~ /\}/) atla = 0; next } { print }'; }
kok_goruntu() {
    echo "## ip -br addr"
    ip -br addr
    echo "## ip route"
    ip route
    echo "## ip rule"
    ip rule
    echo "## ip link"
    ip link
    echo "## nft -s list ruleset (küme öğeleri hariç)"
    nft_kurallar
    echo "## /etc/resolv.conf"
    md5sum /etc/resolv.conf
    echo "## /etc/ppp"
    find /etc/ppp -printf '%p %m %s %T@\n' 2>/dev/null | sort | md5sum
    echo "## ip_forward"
    cat /proc/sys/net/ipv4/ip_forward
}

# ------------------------------------------------------------------ sonuç
sonuc() { # $1 PASS|FAIL, $2 ad, $3 açıklama
    echo "$1 $2: $3"
    OZET+=("$1 $2")
    if [ "$1" = PASS ]; then GECEN=$((GECEN + 1)); else KALAN=$((KALAN + 1)); fi
}

KARSILASTIRILDI=0
kok_karsilastir() {
    KARSILASTIRILDI=1
    kok_goruntu >"$CIKTI/kok-sonra.txt" 2>&1 || true
    if diff -u "$CIKTI/kok-once.txt" "$CIKTI/kok-sonra.txt" >"$CIKTI/kok-fark.txt"; then
        sonuc PASS kok_ad_alani "ip addr/route/rule/link, nft kuralları, resolv.conf, /etc/ppp, ip_forward önce/sonra aynı"
    else
        sonuc FAIL kok_ad_alani "kök ad alanı değişti (fark: $CIKTI/kok-fark.txt)"
        cat "$CIKTI/kok-fark.txt"
    fi
}

cikis() {
    local rc=$?
    trap '' INT TERM HUP PIPE
    temizle
    if [ -f "$CIKTI/kok-once.txt" ] && [ "$KARSILASTIRILDI" = 0 ]; then
        echo "Yarıda kaldı (çıkış $rc); temizlendi."
        kok_karsilastir || true
    fi
}

# ------------------------------------------------------------------ ad alanında süreç başlatma
# $1 ad alanı, $2 çıktı dosyası, sonrası komut: arka planda, PID → SON_PID. Komuttan hemen önce yalıtım denetlenir: ağ ve
# bağlama ad alanı kökünkü değil, /etc/ppp ve /etc/resolv.conf bu ad alanının kopyası. Sonra /run'ın üstüne tmpfs.
ns_baslat() {
    local ns=$1 cikti=$2
    shift 2
    ip netns exec "$ns" bash -c '
        ns=$1 isaret=$2
        shift 2
        [ "$(readlink /proc/self/ns/net)" != "$(readlink /proc/1/ns/net)" ] || { echo "YALITIM: kök ağ ad alanı" >&2; exit 97; }
        [ "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)" ] || { echo "YALITIM: kök bağlama ad alanı" >&2; exit 97; }
        [ "$(cat /etc/ppp/.yalitim 2>/dev/null)" = "$isaret $ns" ] || { echo "YALITIM: /etc/ppp örtülmemiş" >&2; exit 97; }
        grep -qx "# $isaret $ns" /etc/resolv.conf || { echo "YALITIM: /etc/resolv.conf örtülmemiş" >&2; exit 97; }
        mount -t tmpfs -o mode=0755,size=8m wfc-run /run || exit 97
        mkdir -p /run/lock /run/pppd
        exec "$@"' wfc "$ns" "$ISARET" "$@" </dev/null >"$cikti" 2>&1 &
    SON_PID=$!
}

# Kendi başlattığımız süreç (timeout sarmalayıcısı; TERM'i pppd'ye/sunucuya iletir, 8 sn sonra KILL eder).
surec_durdur() {
    local p=$1 i
    [ -n "$p" ] || return 0
    if yasiyor "$p"; then
        kill -TERM "$p" 2>/dev/null || true
        for ((i = 0; i < 75; i++)); do
            yasiyor "$p" || break
            sleep 0.2
        done
        if yasiyor "$p"; then kill -KILL "$p" 2>/dev/null || true; fi
    fi
    wait "$p" 2>/dev/null || true
}

# ------------------------------------------------------------------ bekleme ve durum
simdi_ms() { echo $(($(date +%s%N) / 1000000)); }
# $1 süre (sn), sonrası koşul komutu: doğru olunca 0, süre dolarsa 1 (0,2 sn arayla)
bekle() {
    local son=$(($(simdi_ms) + $1 * 1000))
    shift
    until "$@"; do
        [ "$(simdi_ms)" -lt "$son" ] || return 1
        sleep 0.2
    done
}
ppp_ip() { { ip -n "$CPE" -4 -o addr show dev "$PPP" 2>/dev/null | awk '{ split($4, a, "/"); print a[1]; exit }'; } || true; }
ip_var() { [ -n "$(ppp_ip)" ]; }
ip_yok() { [ -z "$(ppp_ip)" ]; }
gunlukte() { grep -Eq -- "$1" "$GUNLUK" 2>/dev/null; }
sure() { awk -v ms="$1" 'BEGIN { printf "%.1f sn", ms / 1000 }'; }
gunluk_goster() { # son satırlar (FAIL ayrıntısı)
    echo "  --- pppd günlüğü ($GUNLUK), son 15 satır:"
    tail -n 15 "$GUNLUK" 2>/dev/null | sed 's/^/  | /'
    if [ -s "${GUNLUK%.log}.cikti" ]; then
        echo "  --- pppd çıktısı:"
        sed 's/^/  | /' "${GUNLUK%.log}.cikti"
    fi
}
# Bağlantı kurulmadan kalınması beklenen denemelerde pppd'nin seçenek hatasıyla çıkmadığı da denetlenir
istemci_calisiyor() { [ -n "$ISTEMCI_PID" ] && yasiyor "$ISTEMCI_PID"; }

# ------------------------------------------------------------------ operatör (pppoe-server)
# $1 chap|pap, $2 arayüz, $3 AC adı (boş: varsayılan), $4 servis adı (boş: hepsi)
sunucu_baslat() {
    local kimlik=$1 arayuz=$2 ac=${3:-} servis=${4:-}
    local -a ek=()
    [ -z "$ac" ] || ek+=(-C "$ac")
    [ -z "$servis" ] || ek+=(-S "$servis")
    # Sunucunun pppd seçenekleri (yalıtılmış /etc/ppp'de; kimlik bilgisi chap-secrets/pap-secrets'te)
    cat >"$ETC_ISP/ppp/wfc-sunucu" <<EOF
name wfc-bras
require-$kimlik
noipv6
lcp-echo-interval 10
lcp-echo-failure 3
mtu 1492
mru 1492
logfile $CIKTI/sunucu-pppd.log
EOF
    echo "  operatör: pppoe-server ($arayuz, $kimlik${ac:+, AC $ac}${servis:+, servis $servis})" >>"$CIKTI/sunucu.cikti"
    ns_baslat "$ISP" "$IS/sunucu.cikti" timeout -k 8 600 pppoe-server -F -k -g "$EKLENTI" -I "$arayuz" \
        -L "$BRAS" -R "$HAVUZ" -N 4 -O /etc/ppp/wfc-sunucu "${ek[@]}"
    SUNUCU_PID=$SON_PID
    sleep 0.5
    if ! yasiyor "$SUNUCU_PID"; then
        cat "$IS/sunucu.cikti" >>"$CIKTI/sunucu.cikti" 2>/dev/null || true
        echo "  pppoe-server başlamadı: $(head -3 "$IS/sunucu.cikti" 2>/dev/null)"
        SUNUCU_PID=
        return 1
    fi
}
sunucu_durdur() {
    surec_durdur "$SUNUCU_PID"
    SUNUCU_PID=
    [ -z "$IS" ] || cat "$IS/sunucu.cikti" >>"$CIKTI/sunucu.cikti" 2>/dev/null || true
    # oturum pppd'leri de bitmeli (sunucu TERM'de onları da durdurur; kalmışsa bizim ad alanımızda, bizim süreçlerimiz)
    ns_surec_durdur "$ISP"
}

# ------------------------------------------------------------------ cihaz tarafı (gerçek pppd)
# ag.toml'un [pppoe] tablosu; $1 parola, sonrası ek satırlar ("vlan = 35" …)
ag_toml() {
    local p=$1 s
    shift
    printf '[pppoe]\nacik = true\nkullanici = "%s"\nparola = "%s"\n' "$KULLANICI" "$p"
    for s in "$@"; do printf '%s\n' "$s"; done
}
# $1 günlük adı, $2 PPPoE eklentisinin Ethernet arayüzü (nic-…), $3 ag.toml metni
istemci_baslat() {
    local ad=$1 nic=$2
    GUNLUK=$CIKTI/$ad.log
    : >"$GUNLUK"
    (umask 077 && printf '%s\n' "$3" >"$IS/ag.toml")
    if ! "$WFC" ctl pppoe-peers "$IS/ag.toml" "$nic" >"$IS/peers.uretilen" 2>"$IS/peers.hata"; then
        echo "  ctl pppoe-peers başarısız: $(head -3 "$IS/peers.hata")"
        return 1
    fi
    # Ek önlem: operatörün DNS'i istenmez (yalıtılmış /etc/ppp'de ip-up.d kancası da yok, /etc/resolv.conf örtülü)
    (umask 077 && grep -vx 'usepeerdns' "$IS/peers.uretilen" >"$ETC_CPE/ppp/peers/wificorrect") || true
    if ! grep -qx 'plugin pppoe.so' "$ETC_CPE/ppp/peers/wificorrect" || ! grep -qx "nic-$nic" "$ETC_CPE/ppp/peers/wificorrect" ||
        grep -q 'usepeerdns' "$ETC_CPE/ppp/peers/wificorrect"; then
        echo "  bağlantı dosyası beklenmedik"
        return 1
    fi
    # sistemdeki birim gibi: pppd call wificorrect nodetach (+ günlük dosyası)
    ns_baslat "$CPE" "$CIKTI/$ad.cikti" timeout -k 8 300 pppd call wificorrect nodetach logfile "$GUNLUK"
    ISTEMCI_PID=$SON_PID
    # Günlüğün zamanlı kopyası (pppd'nin dosyasında zaman yok): <ad>.zaman, pppd başlangıcından saniye. Kök ad alanında
    # yalnızca dosya okur; pppd'nin sarmalayıcısı bitince kendiliğinden biter.
    local t0
    t0=$(simdi_ms)
    tail --pid="$ISTEMCI_PID" -s 0.2 -n +1 -F "$GUNLUK" 2>/dev/null | while IFS= read -r l; do
        ms=$((${EPOCHREALTIME/[.,]/} / 1000 - t0))
        printf '%4d.%d %s\n' $((ms / 1000)) $((ms % 1000 / 100)) "$l"
    done >"$CIKTI/$ad.zaman" &
}
iki_red() {
    local n
    n=$(grep -cE 'authentication failed$' "$GUNLUK" 2>/dev/null || true)
    [ "${n:-0}" -ge 2 ]
}
# <ad>.zaman'da $1 kalıbına uyan $2. satırın zamanı (sn; yoksa boş)
zaman() { { grep -E -- "$1" "${GUNLUK%.log}.zaman" 2>/dev/null | sed -n "${2}p" | awk '{ print $1 }'; } || true; }
# $1, $2 zaman (sn) → "b − a" (biri boşsa "?")
fark() { if [ -n "$1" ] && [ -n "$2" ]; then awk -v a="$1" -v b="$2" 'BEGIN { printf "%.1f", b - a }'; else echo "?"; fi; }
istemci_durdur() {
    surec_durdur "$ISTEMCI_PID"
    ISTEMCI_PID=
    ns_surec_durdur "$CPE"
    [ -s "${GUNLUK%.log}.cikti" ] || rm -f "${GUNLUK%.log}.cikti"
}
# O anki günlük, betiğin kendi durdurma satırlarından önce: tests/fixtures/pppoe/ örneği
ornek_al() { cp "$GUNLUK" "$CIKTI/ornek/$1.log"; }

# $1 ad, $2 süre (sn), $3 ag.toml metni, $4 PASS açıklaması: pppd başlar; o süre içinde bağlanmamalı ve çalışmaya
# devam etmeli (seçenek hatasıyla çıkan pppd de "bağlanmadı" olurdu)
baglanmamali() {
    local ad=$1 sn=$2
    if ! istemci_baslat "$ad" "$V_CPE" "$3"; then
        sonuc FAIL "$ad" "pppd başlatılamadı"
    elif bekle "$sn" ip_var; then
        sonuc FAIL "$ad" "bağlanmamalıydı ama $(ppp_ip) aldı"
    elif ! istemci_calisiyor; then
        sonuc FAIL "$ad" "pppd erken çıktı"
        gunluk_goster
    else
        sonuc PASS "$ad" "$4"
    fi
    istemci_durdur
}
# $1 ad, $2 nic, $3 ag.toml metni, $4 PASS açıklamasının başı: pppd başlar, 15 sn içinde IPv4 almalı
baglanmali() {
    local ad=$1 t0 ms secenek
    if ! istemci_baslat "$ad" "$2" "$3"; then
        sonuc FAIL "$ad" "pppd başlatılamadı"
    else
        t0=$(simdi_ms)
        if bekle 15 ip_var; then
            ms=$(($(simdi_ms) - t0))
            secenek=$(grep -E '^(refuse-|pppoe-)' "$ETC_CPE/ppp/peers/wificorrect" | paste -sd' ' - || true)
            sonuc PASS "$ad" "$4${secenek:+ [$secenek]} → $PPP $(ppp_ip) ($(sure "$ms"))"
        else
            sonuc FAIL "$ad" "15 sn içinde IPv4 yok"
            gunluk_goster
        fi
    fi
    istemci_durdur
}

# ------------------------------------------------------------------ senaryolar
senaryo_bagli() {
    sunucu_baslat chap "$V_ISP" || { sonuc FAIL bagli "operatör başlamadı"; return; }
    istemci_baslat bagli "$V_CPE" "$(ag_toml "$PAROLA")" || { sonuc FAIL bagli "pppd başlatılamadı"; sunucu_durdur; return; }
    echo "  bağlantı dosyası (parola gizli):"
    sed "s/$PAROLA/***/" "$ETC_CPE/ppp/peers/wificorrect" | sed 's/^/  | /'
    local t0 ms ip rota mtu
    t0=$(simdi_ms)
    if bekle 15 ip_var; then
        ms=$(($(simdi_ms) - t0))
        sleep 1 # IPCP'nin kalan satırları
        ornek_al bagli
        ip=$(ppp_ip)
        rota=$(ip -n "$CPE" route show default | sed 's/[[:space:]]*$//' || true)
        mtu=$(ip -n "$CPE" -o link show dev "$PPP" | grep -o 'mtu [0-9]*' | cut -d' ' -f2 || true)
        if ! grep -Eq "^default dev $PPP .*metric 100( |$)" <<<"$rota"; then
            sonuc FAIL bagli "$ip ama varsayılan rota beklenmedik: $rota"
        elif [ "$mtu" != 1492 ]; then
            sonuc FAIL bagli "$ip ama MTU $mtu"
        else
            sonuc PASS bagli "$PPP $ip ($(sure "$ms")), rota '$rota', MTU $mtu"
        fi
    else
        sonuc FAIL bagli "15 sn içinde IPv4 yok"
        gunluk_goster
    fi
    istemci_durdur
    sunucu_durdur
}

# $1 ad (ornek adı), $2 chap|pap
senaryo_kimlik_hatasi() {
    local ad=$1 kimlik=$2 satir
    sunucu_baslat "$kimlik" "$V_ISP" || { sonuc FAIL "$ad" "operatör başlamadı"; return; }
    istemci_baslat "$ad" "$V_CPE" "$(ag_toml "$YANLIS")" || { sonuc FAIL "$ad" "pppd başlatılamadı"; sunucu_durdur; return; }
    if bekle 20 gunlukte 'authentication failed'; then
        sleep 2 # ardından gelen kapanış satırları
        ornek_al "$ad"
        satir=$(grep -m1 'authentication failed' "$GUNLUK" || true)
        # Bilgi: pppd yanlış parolada ne sıklıkla yeniden dener (operatörde hesap kilidi riski)
        local tekrar
        if bekle 20 iki_red; then
            sleep 0.5 # zamanlı kopya yetişsin
            tekrar="ikinci deneme $(fark "$(zaman 'authentication failed$' 1)" "$(zaman 'authentication failed$' 2)") sn sonra yine reddedildi"
        else
            tekrar="20 sn içinde yeniden denemedi"
        fi
        if ip_var; then
            sonuc FAIL "$ad" "yanlış parolayla $(ppp_ip) aldı"
        else
            sonuc PASS "$ad" "operatör ($kimlik) reddetti: '$satir'; $tekrar"
        fi
    else
        sonuc FAIL "$ad" "20 sn içinde 'authentication failed' yok"
        gunluk_goster
    fi
    istemci_durdur
    sunucu_durdur
}

senaryo_sunucu_yok() {
    local t0 ms
    istemci_baslat sunucu_yok "$V_CPE" "$(ag_toml "$PAROLA")" || { sonuc FAIL sunucu_yok "pppd başlatılamadı"; return; }
    t0=$(simdi_ms)
    # PADI 3 kez (5 + 10 + 20 sn) yanıtsız kalınca
    if bekle 60 gunlukte 'Timeout waiting for PADO|Unable to complete PPPoE Discovery'; then
        ms=$(($(simdi_ms) - t0))
        sleep 2
        ornek_al sunucu_yok
        if ip_var; then
            sonuc FAIL sunucu_yok "operatör yokken $(ppp_ip) aldı"
        else
            sonuc PASS sunucu_yok "$(sure "$ms") sonra: $(grep -E 'PADO|Discovery' "$GUNLUK" | head -2 | sed "s/^/'/; s/$/'/" | paste -sd' ')"
        fi
    else
        sonuc FAIL sunucu_yok "60 sn içinde PADO zaman aşımı satırı yok"
        gunluk_goster
    fi
    istemci_durdur
}

senaryo_vlan() {
    local alt_isp=$V_ISP.$VLAN alt_cpe=$V_CPE.$VLAN ms t0 qos
    ip -n "$ISP" link add link "$V_ISP" name "$alt_isp" type vlan id "$VLAN"
    ip -n "$ISP" link set dev "$alt_isp" up
    # Cihaz tarafı: pppoe::hazirla'nın kurduğu (src/pppoe.rs): <port>.<vlan>, öncelik > 0 ise egress-qos-map 0:<öncelik>
    ip -n "$CPE" link set dev "$V_CPE" up
    ip -n "$CPE" link add link "$V_CPE" name "$alt_cpe" type vlan id "$VLAN" egress-qos-map "0:$ONCELIK"
    ip -n "$CPE" link set dev "$alt_cpe" up
    if sunucu_baslat chap "$alt_isp"; then
        # Operatör yalnızca VLAN 35'te: VLAN'sız bağlantı dosyası bağlanmamalı (deneme gerçekten VLAN istiyor)
        baglanmamali vlan_gerekli 10 "$(ag_toml "$PAROLA")" "VLAN'sız 10 sn'de bağlanmadı (operatör yalnızca VLAN $VLAN'te)"
        if istemci_baslat vlan "$alt_cpe" "$(ag_toml "$PAROLA" "vlan = $VLAN" "oncelik = $ONCELIK")"; then
            t0=$(simdi_ms)
            if bekle 15 ip_var; then
                ms=$(($(simdi_ms) - t0))
                qos=$(ip -n "$CPE" -d link show dev "$alt_cpe" | grep -o 'egress-qos-map { [^}]*}' || true)
                if grep -q "0:$ONCELIK" <<<"$qos"; then
                    sonuc PASS vlan "nic-$alt_cpe: $PPP $(ppp_ip) ($(sure "$ms")), $qos"
                else
                    sonuc FAIL vlan "bağlandı ama öncelik eşlemesi yok: '$qos'"
                fi
            else
                sonuc FAIL vlan "VLAN $VLAN ile 15 sn içinde IPv4 yok"
                gunluk_goster
            fi
        else
            sonuc FAIL vlan "pppd başlatılamadı"
        fi
        istemci_durdur
        sunucu_durdur
    else
        sonuc FAIL vlan "operatör başlamadı"
    fi
    ip -n "$CPE" link del dev "$alt_cpe" 2>/dev/null || true
    ip -n "$ISP" link del dev "$alt_isp" 2>/dev/null || true
}

senaryo_ac_servis() {
    sunucu_baslat chap "$V_ISP" "$AC" "$SERVIS" || { sonuc FAIL ac_servis "operatör başlamadı"; return; }
    baglanmamali ac_yanlis 10 "$(ag_toml "$PAROLA" 'ac_adi = "yanlis-ac"' "servis_adi = \"$SERVIS\"")" \
        "pppoe-ac \"yanlis-ac\" ile 10 sn'de bağlanmadı (operatör AC'si $AC)"
    baglanmamali servis_yanlis 10 "$(ag_toml "$PAROLA" "ac_adi = \"$AC\"" 'servis_adi = "yanlis-servis"')" \
        "pppoe-service \"yanlis-servis\" ile 10 sn'de bağlanmadı (operatör servisi $SERVIS)"
    baglanmali ac_servis "$V_CPE" "$(ag_toml "$PAROLA" "ac_adi = \"$AC\"" "servis_adi = \"$SERVIS\"")" \
        "pppoe-ac \"$AC\", pppoe-service \"$SERVIS\""
    sunucu_durdur
}

ip_var_ya_da_cikti() { ip_var || ! istemci_calisiyor; }
# Yanlış kimlik YÖNTEMİ (operatör öbürünü ister): LCP hiç kalkmaz, operatör "peer refused to authenticate" ile kapatır, holdoff
# işlemez (~3,3 sn'de bir yeni oturum). Bağlantı dosyasındaki maxfail 3 ile pppd üç başarısız denemeden sonra kendiliğinden çıkar
# (sistemde birim onu gecikmeyle yeniden başlatır); "hâlâ çalışıyor" şartı bu yüzden yok. PASS: süre boyunca IP yok, günlükte
# operatörün reddi ve pppd ya hâlâ deniyor ya da kendiliğinden çıktı. Seçenek hatasıyla çıkan pppd PASS sayılmaz: o oturum açmaz
# (günlükte ret olmaz) ve 1-4 koduyla çıkar (ölümcül/seçenek hatası; 5 ve üstü bağlantı sonuçlarıdır, pppd(8) EXIT STATUS).
# $1 ad, $2 en çok bekleme (sn), $3 ag.toml metni, $4 PASS açıklamasının başı
yontem_reddi() {
    local ad=$1 sn=$2 n rc=0 t0 ms
    if ! istemci_baslat "$ad" "$V_CPE" "$3"; then
        sonuc FAIL "$ad" "pppd başlatılamadı"
        return
    fi
    t0=$(simdi_ms)
    bekle "$sn" ip_var_ya_da_cikti || true
    ms=$(($(simdi_ms) - t0))
    n=$(grep -c 'peer refused to authenticate' "$GUNLUK" 2>/dev/null || true)
    if ip_var; then
        sonuc FAIL "$ad" "bağlanmamalıydı ama $(ppp_ip) aldı"
    elif [ "${n:-0}" = 0 ]; then
        sonuc FAIL "$ad" "günlükte operatörün reddi ('peer refused to authenticate') yok"
        gunluk_goster
    elif istemci_calisiyor; then
        sonuc PASS "$ad" "$4: $n ret, pppd $(sure "$ms") sonra hâlâ deniyor"
    else
        # timeout sarmalayıcısı pppd'nin çıkış kodunu geçirir (124 ve üstü: zaman aşımı ya da sinyal)
        wait "$ISTEMCI_PID" 2>/dev/null || rc=$?
        if [ "$rc" -lt 124 ] && ! [[ $rc =~ ^[1-4]$ ]]; then
            sonuc PASS "$ad" "$4: $n ret, pppd $(sure "$ms") sonra kendiliğinden çıktı (kod $rc, maxfail 3)"
        else
            sonuc FAIL "$ad" "pppd hatayla çıktı (kod $rc, $n ret)"
            gunluk_goster
        fi
    fi
    istemci_durdur
}

# Kimlik yöntemi seçimi: kimlik=pap → refuse-chap/mschap/mschap-v2/eap, kimlik=chap → refuse-pap/eap.
# $1 operatörün istediği (pap|chap), $2 cihazda uyan seçim, $3 uymayan seçim
senaryo_kimlik_yontemi() {
    local op=$1 uyan=$2 uymayan=$3
    sunucu_baslat "$op" "$V_ISP" || { sonuc FAIL "kimlik_$uyan" "operatör başlamadı"; return; }
    baglanmali "kimlik_$uyan" "$V_CPE" "$(ag_toml "$PAROLA" "kimlik = \"$uyan\"")" "operatör $op, kimlik=$uyan"
    yontem_reddi "kimlik_${uymayan}_red" 20 "$(ag_toml "$PAROLA" "kimlik = \"$uymayan\"")" \
        "operatör yalnızca $op isterken kimlik=$uymayan bağlanmadı"
    sunucu_durdur
}

senaryo_yeniden_baglanma() {
    local t0 ms ilk
    sunucu_baslat chap "$V_ISP" || { sonuc FAIL yeniden_baglanma "operatör başlamadı"; return; }
    istemci_baslat yeniden_baglanma "$V_CPE" "$(ag_toml "$PAROLA")" || { sonuc FAIL yeniden_baglanma "pppd başlatılamadı"; sunucu_durdur; return; }
    if ! bekle 15 ip_var; then
        sonuc FAIL yeniden_baglanma "ilk bağlantı 15 sn içinde kurulmadı"
        gunluk_goster
        istemci_durdur
        sunucu_durdur
        return
    fi
    ilk=$(ppp_ip)
    sunucu_durdur # operatör durdu (oturumunu kapatarak)
    if ! bekle 20 ip_yok; then
        sonuc FAIL yeniden_baglanma "operatör durunca 20 sn içinde bağlantı düşmedi"
        gunluk_goster
        istemci_durdur
        return
    fi
    sleep 2
    sunucu_baslat chap "$V_ISP" || { sonuc FAIL yeniden_baglanma "operatör yeniden başlamadı"; istemci_durdur; return; }
    t0=$(simdi_ms)
    if bekle 45 ip_var; then
        ms=$(($(simdi_ms) - t0))
        sleep 0.5 # zamanlı kopya yetişsin
        sonuc PASS yeniden_baglanma "$ilk → operatör durdu, bağlantı düştü → operatör açıldıktan $(sure "$ms") sonra $(ppp_ip) (pppd: düşüşten yeni oturuma $(fark "$(zaman 'Connection terminated' 1)" "$(zaman 'PPP session is' 2)") sn)"
    else
        sonuc FAIL yeniden_baglanma "operatör açıldıktan sonra 45 sn içinde yeniden bağlanmadı"
        gunluk_goster
    fi
    istemci_durdur
    sunucu_durdur
}

# Hat kopması (ONT sökülmüş ya da hat kesik): iki yönde de her çerçeve düşer; operatörün LCP kapanışı ve PADT'si de
# gelmez. pppd bunu kendi LCP yankısıyla (10 sn × 6) fark etmeli, hat gelince yeniden bağlanmalı. Arayüzler açık kalır:
# çekirdek PPPoE oturumunu taşıyıcı kaybında da kapatmaz (yalnızca arayüz kapanınca), pppd için ikisi aynı.
kesik_tablo() { # $1 aygıt
    printf 'table netdev wfckesik {\n\tchain giris {\n\t\ttype filter hook ingress device "%s" priority 0; policy drop;\n\t}\n}\n' "$1"
}
hat_kes() {
    kesik_tablo "$V_ISP" | ip netns exec "$ISP" nft -f - && kesik_tablo "$V_CPE" | ip netns exec "$CPE" nft -f -
}
hat_bagla() {
    ip netns exec "$ISP" nft delete table netdev wfckesik
    ip netns exec "$CPE" nft delete table netdev wfckesik
}
senaryo_hat_kopmasi() {
    local t0 ms ms2
    sunucu_baslat chap "$V_ISP" || { sonuc FAIL hat_kopmasi "operatör başlamadı"; return; }
    istemci_baslat hat_kopmasi "$V_CPE" "$(ag_toml "$PAROLA")" || { sonuc FAIL hat_kopmasi "pppd başlatılamadı"; sunucu_durdur; return; }
    if ! bekle 15 ip_var; then
        sonuc FAIL hat_kopmasi "ilk bağlantı 15 sn içinde kurulmadı"
        gunluk_goster
        istemci_durdur
        sunucu_durdur
        return
    fi
    if ! hat_kes; then
        sonuc FAIL hat_kopmasi "hat kesilemedi (nft)"
        istemci_durdur
        sunucu_durdur
        return
    fi
    t0=$(simdi_ms)
    if ! bekle 90 ip_yok; then
        hat_bagla || true
        sonuc FAIL hat_kopmasi "hat koptuktan sonra 90 sn içinde bağlantı düşmedi"
        gunluk_goster
        istemci_durdur
        sunucu_durdur
        return
    fi
    ms=$(($(simdi_ms) - t0))
    hat_bagla || true
    t0=$(simdi_ms)
    if ! gunlukte 'echo-requests'; then
        # kopmayı pppd'nin kendisi değil başka bir şey (operatörün kapanışı) bildirdi: hat tam kesilmemiş
        sonuc FAIL hat_kopmasi "bağlantı $(sure "$ms") sonra düştü ama LCP yankısı satırı yok"
        gunluk_goster
    elif bekle 60 ip_var; then
        ms2=$(($(simdi_ms) - t0))
        sonuc PASS hat_kopmasi "kopma $(sure "$ms") sonra fark edildi ('$(grep -m1 'echo-requests' "$GUNLUK" || true)'), hat gelince $(sure "$ms2") sonra $(ppp_ip)"
    else
        sonuc FAIL hat_kopmasi "hat geldikten sonra 60 sn içinde yeniden bağlanmadı"
        gunluk_goster
    fi
    istemci_durdur
    sunucu_durdur
}

# MSS: misafir (wfc-lan) → cihaz (wfc-cpe: iletim + MSS kuralı + NAT) → PPPoE → operatör (wfc-isp). Operatör tarafında
# gelen SYN'in MSS'i sayaçlarla ölçülür (tcpdump yok). Önce kuralsız (karşılaştırma: 1460), sonra kurallı (≤ 1452).
mss_sayac() { # $1 yorum → paket sayısı
    { ip netns exec "$ISP" nft list chain inet wfcolcum gelen |
        awk -v y="comment \"$1\"" 'index($0, y) { for (i = 1; i < NF; i++) if ($i == "packets") { print $(i + 1); exit } }'; } || true
}
syn_gonder() { # $1 hedef port; bağlantı reddedilir (dinleyen yok), yalnızca SYN gitsin
    ip netns exec "$LAN" timeout 3 bash -c "exec 3<>/dev/tcp/$BRAS/$1" </dev/null >/dev/null 2>&1 || true
}
senaryo_mss() {
    local kural s
    kural=$(sed -n 's/^[[:space:]]*\([^#]*maxseg size set[^#]*[^#[:space:]]\)[[:space:]]*$/\1/p' "$KURALLAR/guvenlik.nft" 2>/dev/null | head -1 || true)
    if [ -n "$kural" ]; then echo "  MSS kuralı ($KURALLAR/guvenlik.nft): $kural"; else
        kural=$MSS_VARSAYILAN
        echo "  MSS kuralı (betikteki kopya; $KURALLAR/guvenlik.nft'te yok): $kural"
    fi
    ip netns add "$LAN"
    ip -n "$LAN" link set lo up
    ip -n "$CPE" link add "$V_CPE_LAN" address "$MAC_CPE_LAN" type veth peer name "$V_LAN" address "$MAC_LAN" netns "$LAN"
    ip -n "$CPE" addr add "$LAN_AG.1/24" dev "$V_CPE_LAN"
    ip -n "$CPE" link set dev "$V_CPE_LAN" up
    ip -n "$LAN" addr add "$LAN_AG.2/24" dev "$V_LAN"
    ip -n "$LAN" link set dev "$V_LAN" up
    ip -n "$LAN" route add default via "$LAN_AG.1"
    # cihaz: iletim + NAT (sistemdeki gibi); MSS kuralı karşılaştırma SYN'inden sonra forward'ın başına eklenir
    if ! ip netns exec "$CPE" sh -c 'echo 1 >/proc/sys/net/ipv4/ip_forward' ||
        ! ip netns exec "$CPE" nft -f - <<EOF; then
table inet wfcdeneme {
	chain forward {
		type filter hook forward priority filter; policy accept;
	}
	chain postrouting {
		type nat hook postrouting priority srcnat; policy accept;
		oifname "$PPP" masquerade
	}
}
EOF
        sonuc FAIL mss "cihaz ad alanında iletim/NAT kurulamadı"
        return
    fi
    # operatör: gelen SYN'lerin MSS'i (yönlendirmeden önce)
    if ! ip netns exec "$ISP" nft -f - <<'EOF'; then
table inet wfcolcum {
	chain gelen {
		type filter hook prerouting priority raw; policy accept;
		tcp dport 7001 tcp flags syn / syn,ack counter comment "s7001"
		tcp dport 7001 tcp flags syn / syn,ack tcp option maxseg size 1460 counter comment "s7001_1460"
		tcp dport 7001 tcp flags syn / syn,ack tcp option maxseg size 0-1452 counter comment "s7001_le1452"
		tcp dport 7002 tcp flags syn / syn,ack counter comment "s7002"
		tcp dport 7002 tcp flags syn / syn,ack tcp option maxseg size 1452 counter comment "s7002_1452"
		tcp dport 7002 tcp flags syn / syn,ack tcp option maxseg size 0-1452 counter comment "s7002_le1452"
	}
}
EOF
        sonuc FAIL mss "operatör ad alanında sayaçlar kurulamadı"
        return
    fi
    if ! sunucu_baslat chap "$V_ISP"; then
        sonuc FAIL mss "operatör başlamadı"
        return
    fi
    if ! istemci_baslat mss "$V_CPE" "$(ag_toml "$PAROLA")" || ! bekle 15 ip_var; then
        sonuc FAIL mss "PPPoE bağlantısı kurulamadı"
        gunluk_goster
        istemci_durdur
        sunucu_durdur
        return
    fi
    syn_gonder 7001 # kural yokken
    if ! ip netns exec "$CPE" nft insert rule inet wfcdeneme forward "$kural"; then
        sonuc FAIL mss "MSS kuralı yüklenemedi: $kural"
        istemci_durdur
        sunucu_durdur
        return
    fi
    syn_gonder 7002 # kural varken
    local a a1460 ale b b1452 ble
    a=$(mss_sayac s7001)
    a1460=$(mss_sayac s7001_1460)
    ale=$(mss_sayac s7001_le1452)
    b=$(mss_sayac s7002)
    b1452=$(mss_sayac s7002_1452)
    ble=$(mss_sayac s7002_le1452)
    s="kuralsız: $a SYN, MSS 1460: $a1460; kurallı: $b SYN, MSS 1452: $b1452, MSS ≤ 1452: $ble"
    if [ "${a:-0}" -ge 1 ] && [ "$a1460" = "$a" ] && [ "${ale:-0}" = 0 ] && [ "${b:-0}" -ge 1 ] && [ "$ble" = "$b" ]; then
        sonuc PASS mss "$s"
    else
        sonuc FAIL mss "$s"
    fi
    istemci_durdur
    sunucu_durdur
}

# Güvenlik duvarı ve hız kuralları (KURALLAR'daki guvenlik.nft, hiz.nft) iki WAN tanımıyla da geçerli mi: eski tek ad
# (0.4.16'nın yazdığı arayuzler.nft; yükseltmeden sonra ilk PPPoE ayarına ya da port değişikliğine kadar diskte o durur, açılışta
# guvenlik.nft onunla yüklenir) ve küme (internet portu + wfcppp). Yalnızca denetim (nft -c), cihaz ad alanında (wfc-cpe; kök ad
# alanının kurallarına dokunulmaz). Karşı sınama: bilerek bozulmuş bir kopya reddedilmeli (denetim gerçekten bir şey denetliyor).
# $1 kaynak, $2 tanım (tek|kume), $3 çıktı: include satırı geçici WAN tanımına çevrilmiş kopya (satır yoksa başarısız)
nft_kopya() {
    sed "s|^include \"/etc/wificorrect/arayuzler.nft\"|include \"$IS/arayuzler-$2.nft\"|" "$1" >"$3" &&
        grep -qxF "include \"$IS/arayuzler-$2.nft\"" "$3"
}
senaryo_nft_kurallari() {
    local f tanim hata= gecen=() yonetim
    for f in guvenlik.nft hiz.nft; do
        if [ ! -f "$KURALLAR/$f" ]; then
            sonuc FAIL nft_kurallari "$KURALLAR/$f yok"
            return
        fi
    done
    printf 'define WAN = "%s"\n' "$V_CPE" >"$IS/arayuzler-tek.nft"
    printf 'define WAN = { "%s", "%s" }\n' "$V_CPE" "$PPP" >"$IS/arayuzler-kume.nft"
    for tanim in tek kume; do
        for f in guvenlik.nft hiz.nft; do
            if ! nft_kopya "$KURALLAR/$f" "$tanim" "$IS/$tanim-$f"; then
                hata="$f: include satırı bulunamadı"
            elif ip netns exec "$CPE" nft -c -f "$IS/$tanim-$f" >"$IS/nft.cikti" 2>&1; then
                gecen+=("$tanim/$f")
                continue
            else
                hata="$tanim/$f: $(head -3 "$IS/nft.cikti" | paste -sd' ' -)"
            fi
            break 2
        done
    done
    if [ -z "$hata" ]; then
        # karşı sınama: guvenlik.nft'nin bozuk kopyası (giriş zincirinin politikası) reddedilmeli
        sed 's/policy drop;/policy dorp;/' "$IS/kume-guvenlik.nft" >"$IS/bozuk-guvenlik.nft"
        if ! grep -q 'policy dorp;' "$IS/bozuk-guvenlik.nft"; then
            hata="karşı sınama kurulamadı (policy drop; yok)"
        elif ip netns exec "$CPE" nft -c -f "$IS/bozuk-guvenlik.nft" >"$IS/nft.cikti" 2>&1; then
            hata="bozuk kopya da kabul edildi: denetim çalışmıyor"
        fi
    fi
    if [ -n "$hata" ]; then
        sonuc FAIL nft_kurallari "$hata"
    else
        yonetim=$(grep -cF "iifname \$WAN iifname != \"$PPP\" " "$KURALLAR/guvenlik.nft" || true)
        sonuc PASS nft_kurallari "${gecen[*]} nft -c ile geçerli, bozuk kopya reddedildi; yönetim kurallarında $PPP hariç: ${yonetim:-0} satır"
    fi
}

# ------------------------------------------------------------------ hazırlık
temizle # önceki yarıda kalmış denemenin artıkları
[ -d /etc/ppp ] || hata "/etc/ppp yok (ppp paketi kurulu değil); yalıtım bağlaması için gerekli"
[ -x "$WFC" ] || hata "$WFC yok"
EKLENTI=$(ls /usr/lib/pppd/*/pppoe.so 2>/dev/null | sort -V | tail -1)
[ -n "$EKLENTI" ] || hata "pppd PPPoE eklentisi (pppoe.so) bulunamadı"
rm -rf "$CIKTI"
mkdir -p "$CIKTI/ornek"
trap cikis EXIT
trap 'exit 130' INT TERM HUP PIPE
IS=$(mktemp -d /tmp/wfc-pppoe-deneme.XXXXXX)
printf '[pppoe]\nacik = true\nkullanici = "k"\nparola = "p"\n' >"$IS/ag.toml"
"$WFC" ctl pppoe-peers "$IS/ag.toml" "$V_CPE" 2>/dev/null | grep -qx 'plugin pppoe.so' ||
    hata "$WFC ctl pppoe-peers çalışmıyor (WFC=… ile bu dalın ikilisini verin)"
kok_goruntu >"$CIKTI/kok-once.txt" 2>&1

echo "== pppoe-deneme $(date '+%F %T'): $(pppd --version 2>&1 | head -1), $(pppoe-server -h 2>&1 | grep -m1 -o 'Version [0-9.]*'), çekirdek $(uname -r)"
echo "   ikili: $WFC ($("$WFC" surum 2>/dev/null)), eklenti: $EKLENTI, günlükler: $CIKTI"

# Ad alanları ve sanal kablo (veth ad alanında doğar, eşi doğrudan öbür ad alanına)
ip netns add "$ISP"
ip netns add "$CPE"
ip -n "$ISP" link set lo up
ip -n "$CPE" link set lo up
ip -n "$CPE" link add "$V_CPE" address "$MAC_CPE" type veth peer name "$V_ISP" address "$MAC_ISP" netns "$ISP"
ip -n "$CPE" link set dev "$V_CPE" up
ip -n "$ISP" link set dev "$V_ISP" up

# Yalıtılmış /etc/ppp ve /etc/resolv.conf
for ns in "$ISP" "$CPE"; do
    mkdir -p "/etc/netns/$ns/ppp/peers"
    chmod 700 "/etc/netns/$ns" "/etc/netns/$ns/ppp"
    echo "$ISARET $ns" >"/etc/netns/$ns/ppp/.yalitim"
    echo "# $ISARET $ns" >"/etc/netns/$ns/resolv.conf"
done
# Cihaz tarafı sistemdeki gibi Debian'ın /etc/ppp/options'ını da okur (auth, lock, lcp-echo… — bağlantı dosyası ezer)
cp /etc/ppp/options "$ETC_CPE/ppp/options"
: >"$ETC_ISP/ppp/options"
(
    umask 077
    printf '"%s" * "%s" *\n' "$KULLANICI" "$PAROLA" >"$ETC_ISP/ppp/chap-secrets"
    cp "$ETC_ISP/ppp/chap-secrets" "$ETC_ISP/ppp/pap-secrets"
)
# Yalıtım çalışıyor mu (pppd başlatmadan): ad alanında /etc/ppp bizimki, kökte değil
[ "$(ip netns exec "$CPE" cat /etc/ppp/.yalitim 2>/dev/null)" = "$ISARET $CPE" ] || hata "/etc/ppp yalıtımı çalışmıyor ($CPE)"
[ "$(ip netns exec "$ISP" cat /etc/ppp/.yalitim 2>/dev/null)" = "$ISARET $ISP" ] || hata "/etc/ppp yalıtımı çalışmıyor ($ISP)"
[ ! -e /etc/ppp/.yalitim ] || hata "kök /etc/ppp'de yalıtım işareti var"

# ------------------------------------------------------------------ çalıştır
BASLA=$(simdi_ms)
for s in "${SECILEN[@]}"; do
    case $s in
        bagli)
            echo "-- doğru parola"
            senaryo_bagli
            ;;
        kimlik_hatasi)
            echo "-- yanlış parola (CHAP)"
            senaryo_kimlik_hatasi kimlik_hatasi chap
            ;;
        kimlik_hatasi_pap)
            echo "-- yanlış parola (PAP)"
            senaryo_kimlik_hatasi kimlik_hatasi_pap pap
            ;;
        sunucu_yok)
            echo "-- operatör yok (PADI yanıtsız, ~35 sn)"
            senaryo_sunucu_yok
            ;;
        vlan)
            echo "-- VLAN $VLAN (öncelik $ONCELIK)"
            senaryo_vlan
            ;;
        ac_servis)
            echo "-- AC ve servis adı"
            senaryo_ac_servis
            ;;
        kimlik_yontemi)
            echo "-- kimlik yöntemi"
            senaryo_kimlik_yontemi pap pap chap
            senaryo_kimlik_yontemi chap chap pap
            ;;
        yeniden_baglanma)
            echo "-- operatör durup açılınca yeniden bağlanma"
            senaryo_yeniden_baglanma
            ;;
        hat_kopmasi)
            echo "-- hat kopması (LCP yankısı 10 sn × 6, ~70 sn)"
            senaryo_hat_kopmasi
            ;;
        mss)
            echo "-- MSS"
            senaryo_mss
            ;;
        nft_kurallari)
            echo "-- güvenlik duvarı kuralları (eski tek adlı ve yeni küme WAN tanımıyla, nft -c, cihaz ad alanında)"
            senaryo_nft_kurallari
            ;;
    esac
done

echo "-- toparlama"
temizle
if grep -rlF -e "$PAROLA" -e "$YANLIS" "$CIKTI" >/dev/null 2>&1; then
    sonuc FAIL parola_gizli "parola günlükte: $(grep -rlF -e "$PAROLA" -e "$YANLIS" "$CIKTI" | paste -sd' ' -)"
else
    sonuc PASS parola_gizli "$CIKTI altındaki hiçbir dosyada parola yok"
fi
kok_karsilastir
echo "== $GECEN PASS, $KALAN FAIL ($(sure $(($(simdi_ms) - BASLA)))); günlükler: $CIKTI"
if [ "$KALAN" != 0 ]; then
    echo "   başarısız: $(printf '%s\n' "${OZET[@]}" | awk '$1 == "FAIL" { print $2 }' | paste -sd' ' -)"
    exit 1
fi
