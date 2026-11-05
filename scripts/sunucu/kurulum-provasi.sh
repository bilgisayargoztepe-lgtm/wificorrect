#!/bin/bash
# Merkez kurulum/geçiş/geri dönüş provası: GERÇEK systemd'li atılabilir bir Ubuntu makinesinde (GitHub Actions ubuntu-24.04)
# scripts/sunucu/merkez/kur.sh'nin canlı geçişte yapacaklarını uçtan uca sınar (Task 13c). root olarak çalışır:
#   sudo IKILI=merkez/target/release/wificorrect-merkez bash scripts/sunucu/kurulum-provasi.sh
# Her adım "PROVA ADIM <n> <ad>: TAMAM|HATA <neden>|ATLANDI <neden>" yazar; ilk HATA'da çıkış 1.
# CANLI SUNUCUYA/CİHAZA BAĞLANMAZ. Dış yan etki yok: bütün wc-* hizmetlerinde WC_TEST=1 (gerçek SMS/istek gitmez), caddy
# hizmeti sahte (ACME'ye çıkmaz). Atılabilir makine dışında ÇALIŞTIRMAYIN (sistem birimlerini ve /var/lib/wificorrect'i değiştirir).
set -uo pipefail
[ "$(id -u)" = 0 ] || { echo "HATA: root olmalı" >&2; exit 2; }
[ "${PROVA_ATILABILIR:-}" = 1 ] || { echo "HATA: bu betik makineyi değiştirir; yalnız atılabilir makinede PROVA_ATILABILIR=1 ile çalışır" >&2; exit 2; }

DEPO=$(cd "$(dirname "$0")/../.." && pwd)
IKILI_KAYNAK=$(readlink -f "${IKILI:?IKILI=derlenmiş wificorrect-merkez yolu gerekli}")
LOG=${PROVA_LOG:-/tmp/prova-log}
CALISMA=/opt/prova
GERI=/var/lib/wificorrect/geri-donus
VERI=/var/lib/wificorrect
DB=$VERI/merkez/merkez.db
BIRIMLER=/etc/systemd/system
ZAMANLAYICILAR="wc-kuyruk.path wc-durum.timer wc-istatistik.timer wc-saglik.timer wc-listeler.timer wc-surumler.timer"
# (birim, alt komut, kullanıcı; boş = root) — birim testleriyle aynı beklenti
HIZMETLER="wc-merkez.service|sunucu|wcpanel
wc-surumler.service|surumler al|wcpanel
wc-saglik.service|saglik|wcpanel
wc-listeler.service|listeler|wcpanel
wc-durum.service|durum|
wc-istatistik.service|istatistik|
wc-kuyruk.service|kuyruk|"
mkdir -p "$LOG" "$CALISMA"
# wcpanel de çalıştırabilsin diye ikili herkesin okuyabildiği bir yere kopyalanır
GERCEK_IKILI=$CALISMA/gercek-ikili
install -m 755 "$IKILI_KAYNAK" "$GERCEK_IKILI"
exec > >(tee -a "$LOG/prova.log") 2>&1

ADIM_NO=0 ADIM_AD=
adim() { ADIM_NO=$1; ADIM_AD=$2; echo "--- adım $1: $2 ($(date +%T))"; }
tamam() { echo "PROVA ADIM $ADIM_NO $ADIM_AD: TAMAM${1:+ ($1)}"; }
atlandi() { echo "PROVA ADIM $ADIM_NO $ADIM_AD: ATLANDI $*"; }
hata() { echo "PROVA ADIM $ADIM_NO $ADIM_AD: HATA $*"; exit 1; }

son_dok() { # tanı: çıkışta birim durumu ve günlükler
  set +e
  { systemctl status 'wc-*' caddy fail2ban --no-pager -l; systemctl list-units 'wc-*' --all --no-pager; } >"$LOG/systemctl-status.txt" 2>&1
  journalctl -u 'wc-*' --no-pager -o short-iso >"$LOG/journal-wc.txt" 2>&1
  ls -laR "$GERI" >"$LOG/geri-donus-agaci.txt" 2>&1
  chmod -R a+rX "$LOG" 2>/dev/null
}
trap son_dok EXIT

# ---- yardımcılar ----------------------------------------------------------------------------------------------------
http() { curl -s -m 5 --connect-timeout 2 -o /dev/null -w '%{http_code}' -H 'Host: yonetim.wificorrect.com' http://127.0.0.1:8081/giris 2>/dev/null || true; }
http_bekle() { # 200 gelene kadar en çok $1 sn bekler
  local bitis=$((SECONDS + ${1:-40})) k
  while :; do k=$(http); [ "$k" = 200 ] && return 0; [ "$SECONDS" -ge "$bitis" ] && { echo "son HTTP kodu: $k"; return 1; }; sleep 1; done
}
exec_start() { systemctl show -p ExecStart --value wc-merkez; }
dinleyen() { # 8081'i dinleyen sürecin komut satırı
  local pid; pid=$(ss -H -ltnp 'sport = :8081' 2>/dev/null | grep -o 'pid=[0-9]*' | head -n1 | cut -d= -f2)
  [ -n "$pid" ] && tr '\0' ' ' </proc/"$pid"/cmdline
}
python_servis() { exec_start | grep -q 'python3' && http_bekle 40 && dinleyen | grep -q 'python3.*merkez\.py'; }
rust_servis() { exec_start | grep -q '/usr/local/bin/wificorrect-merkez sunucu' && http_bekle 40 && dinleyen | grep -q 'wificorrect-merkez sunucu'; }
geri_sayisi() { find "$GERI" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l; }
python_dizini() { grep -qsE 'python3|/usr/local/sbin/wc-' "$GERI/$1"/wc-*.service; }
denetle_tamam() { "$GERCEK_IKILI" veritabani-denetle "$1" --dosyalar "$VERI" 2>&1 | tail -n 1 | grep -qx TAMAM; }

# paket hazırla: HEAD'deki scripts/sunucu (merkez-kur.sh gibi: *.py, tests, parite, logo, __pycache__ hariç) + ikili.
# paket_yap AD [sarmalayici]  -> $CALISMA/AD/merkez/kur.sh
paket_yap() {
  local ad=$1 mod=${2:-gercek} p=$CALISMA/$1
  rm -rf "$p"; mkdir -p "$p"
  tar -C "$CALISMA/kaynak/scripts/sunucu" -cf - --exclude='*.py' --exclude=tests --exclude=parite --exclude=logo --exclude=__pycache__ \
    wificorrect-sunucu merkez | tar -x -C "$p"
  case $mod in
    gercek) install -m 755 "$GERCEK_IKILI" "$p/merkez/wificorrect-merkez" ;;
    denetim_red) # veritabani-denetle 1 döner; öteki her şey gerçek ikiliye
      printf '#!/bin/bash\nif [ "$1" = veritabani-denetle ]; then echo "prova: sahte denetim reddi" >&2; exit 1; fi\nexec %s "$@"\n' "$GERCEK_IKILI" >"$p/merkez/wificorrect-merkez" ;;
    asili) # sunucu: 8081'i açar, bağlantıyı kabul eder ama hiç cevap vermez; öteki her şey gerçek ikiliye
      printf '#!/bin/bash\nif [ "$1" = sunucu ]; then exec python3 -I -c %s; fi\nexec %s "$@"\n' \
        "'import socket;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind((\"127.0.0.1\",8081));s.listen(8);c=[]
while True: c.append(s.accept()[0])'" "$GERCEK_IKILI" >"$p/merkez/wificorrect-merkez" ;;
    *) hata "bilinmeyen paket kipi: $mod" ;;
  esac
  chmod 755 "$p/merkez/wificorrect-merkez"
}

kur() { # kur AD LOGAD [argümanlar]: paketteki kur.sh'yi çalıştırır, çıktıyı $LOG/kur-LOGAD.log'a da yazar; çıkış kodunu döndürür
  local p=$CALISMA/$1 l=$LOG/kur-$2.log; shift 2
  bash "$p/merkez/kur.sh" "$@" >"$l" 2>&1; local k=$?
  echo "kur.sh çıkış kodu: $k (günlük: $l)"; sed 's/^/    | /' "$l" | tail -n 40
  return $k
}

# ---- hazırlık -------------------------------------------------------------------------------------------------------
adim 0 hazirlik
set -e
echo "sistem python3: $(/usr/bin/python3 --version)  | ikili sürümü: $("$GERCEK_IKILI" surum)"
# 1) eski paket: main dalındaki Python merkezi
git -C "$DEPO" rev-parse --verify -q origin/main >/dev/null || { echo "HATA: origin/main yok (checkout fetch-depth: 0 olmalı)"; exit 1; }
rm -rf "$CALISMA/eski" "$CALISMA/kaynak"; mkdir -p "$CALISMA/eski" "$CALISMA/kaynak"
git -C "$DEPO" archive origin/main scripts/sunucu | tar -x -C "$CALISMA/eski"
git -C "$DEPO" archive HEAD scripts/sunucu | tar -x -C "$CALISMA/kaynak"
# 2) dış yan etki yok: caddy sahte, wc-* hizmetlerine WC_TEST=1 (birimler ilk kez başlamadan ÖNCE)
mkdir -p "$BIRIMLER/caddy.service.d"
printf '[Service]\nType=simple\nExecStart=\nExecStart=/bin/sleep infinity\nExecReload=\nExecReload=/bin/true\n' >"$BIRIMLER/caddy.service.d/prova.conf"
while IFS='|' read -r u _ _; do
  mkdir -p "$BIRIMLER/$u.d"
  printf '[Service]\nEnvironment=WC_TEST=1\n' >"$BIRIMLER/$u.d/prova.conf"
done <<<"$HIZMETLER"
systemctl daemon-reload
# 3) paketler (kur.sh'nin kendisi de ister; burada önceden, caddy drop-in'i varken kurulur)
export DEBIAN_FRONTEND=noninteractive
apt-get update -q >/dev/null
apt-get install -y -q caddy fail2ban wireguard-tools curl >/dev/null
command -v caddy fail2ban-client wg >/dev/null || { echo "HATA: caddy/fail2ban/wg kurulamadı"; exit 1; }
echo "caddy: $(caddy version | head -n1)"
systemctl is-active caddy >/dev/null && echo "caddy (sahte) etkin"
# 4) gerçekçi veritabanı eski kurulumdan ÖNCE; sahibi wcpanel, izin 600
id wcpanel >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin wcpanel
install -d -o wcpanel -g wcpanel -m 700 $VERI/merkez
install -o wcpanel -g wcpanel -m 600 "$DEPO/merkez/tests/veri/python-gocmus.db" "$DB"
# wc-durum/istatistik arşiv ve WireGuard kaydı arar (canlıda var)
mkdir -p /srv/wificorrect-arsiv /etc/wireguard/wificorrect
set +e
echo "hazırlık tamam"

# ---- 1. eski Python kurulumu ----------------------------------------------------------------------------------------
adim 1 eski_python_kurulumu
bash "$CALISMA/eski/scripts/sunucu/merkez/kur.sh" >"$LOG/kur-eski-python.log" 2>&1; k=$?
tail -n 25 "$LOG/kur-eski-python.log" | sed 's/^/    | /'
[ "$k" = 0 ] || hata "eski kur.sh çıkış kodu $k (bkz. kur-eski-python.log)"
install -m 755 "$CALISMA/eski/scripts/sunucu/wificorrect-sunucu" /usr/local/sbin/wificorrect-sunucu
python_servis || { journalctl -u wc-merkez --no-pager -n 30; hata "Python wc-merkez 200 vermiyor (python3 $(/usr/bin/python3 --version))"; }
tamam "python3: $(/usr/bin/python3 --version), sistem yorumlayıcısı değiştirilmedi, drop-in gerekmedi"
[ -f /usr/local/lib/wificorrect/merkez.py ] && [ -L /usr/local/sbin/wc-durum ] || hata "Python dosyaları/bağları yok"

py_yerinde() { [ -f /usr/local/lib/wificorrect/merkez.py ] && [ -L /usr/local/sbin/wc-durum ] && [ -L /usr/local/sbin/wc-kuyruk ]; }

# ---- 2. denetim reddi -----------------------------------------------------------------------------------------------
adim 2 denetim_reddi
paket_yap p2 denetim_red
[ ! -d "$GERI" ] || hata "ön koşul: geri dönüş dizini bu noktada olmamalı"
kur p2 denetim-reddi; k=$?
[ "$k" != 0 ] || hata "kur.sh denetim reddinde 0 döndü"
exec_start | grep -q python3 || hata "ExecStart değişti: $(exec_start)"
python_servis || hata "Python artık 200 vermiyor"
[ ! -d "$GERI" ] || [ "$(geri_sayisi)" = 0 ] || hata "denetim reddinde geri dönüş dizini oluştu: $(ls "$GERI")"
grep -q 'veritabanı denetimi başarısız' "$LOG/kur-denetim-reddi.log" || hata "çıktıda denetim hatası iletisi yok"
ls "$DB".yedek-* >/dev/null 2>&1 || hata "denetim reddinde de yedek incelenebilsin diye bırakılmalı"
py_yerinde || hata "Python dosyaları bozuldu"
tamam "kur.sh çıkış $k, canlı Python kurulumu değişmedi, geri dönüş dizini yok"

# ---- 3. sağlık reddi ve kendiliğinden dönüş ------------------------------------------------------------------------
adim 3 saglik_reddi_otomatik_donus
paket_yap p3 asili
t0=$SECONDS
kur p3 saglik-reddi; k=$?
sure=$((SECONDS - t0))
[ "$k" != 0 ] || hata "kur.sh asılı sunucuda 0 döndü"
[ "$sure" -le 120 ] || hata "kur.sh $sure sn sürdü (≈60 sn beklenirdi)"
grep -q 'GERİ DÖNÜLDÜ' "$LOG/kur-saglik-reddi.log" || hata "'GERİ DÖNÜLDÜ' iletisi yok"
python_servis || hata "geri dönüşten sonra Python 200 vermiyor: $(exec_start)"
[ "$(geri_sayisi)" -ge 1 ] || hata "geri dönüş dizini yok"
[ -s "$GERI/son-geri-donus.txt" ] || hata "son-geri-donus.txt yok"
bash "$CALISMA/p3/merkez/kur.sh" --geri-don >"$LOG/geri-don-liste.log" 2>&1  # en yeni (Python) dizine yeniden döner: zararsız; liste çıktısı denetlenir
grep -qE '^  [0-9]{8}-[0-9]{6} \(Python\)$' "$LOG/geri-don-liste.log" || { cat "$LOG/geri-don-liste.log"; hata "--geri-don listesinde (Python) işaretli dizin yok"; }
python_servis || hata "--geri-don sonrası Python 200 vermiyor"
py_yerinde || hata "Python dosyaları bozuldu"
tamam "kur.sh çıkış $k, $sure sn, Python'a dönüldü"

# ---- 4. sağlam ikiliyle geçiş ---------------------------------------------------------------------------------------
adim 4 saglam_gecis
paket_yap p4 gercek
durum_once=$(systemctl show -p InactiveEnterTimestampMonotonic --value wc-durum.service)
kur p4 gecis; k=$?
[ "$k" = 0 ] || hata "kur.sh çıkış $k"
rust_servis || hata "Rust 200 vermiyor: $(exec_start) | dinleyen: $(dinleyen)"
for z in $ZAMANLAYICILAR; do
  [ "$(systemctl is-enabled "$z" 2>&1)" = enabled ] && [ "$(systemctl is-active "$z" 2>&1)" = active ] || hata "$z etkin/aktif değil"
done
while IFS='|' read -r u kom kul; do
  es=$(systemctl show -p ExecStart --value "$u"); us=$(systemctl show -p User --value "$u")
  case $es in *"path=/usr/local/bin/wificorrect-merkez ;"*"argv[]=/usr/local/bin/wificorrect-merkez $kom ;"*) ;; *) hata "$u ExecStart beklenenden farklı: $es" ;; esac
  [ "$us" = "$kul" ] || hata "$u User='$us' beklenen='$kul'"
done <<<"$HIZMETLER"
bekle=$((SECONDS + 90))
prop() { systemctl show -p "$1" --value wc-durum.service; }
while [ "$SECONDS" -lt "$bekle" ]; do
  [ "$(prop ActiveState)" = inactive ] && [ "$(prop Result)" = success ] && [ "$(prop InactiveEnterTimestampMonotonic)" != "$durum_once" ] && break
  sleep 2
done
if ! { [ "$(prop ActiveState)" = inactive ] && [ "$(prop Result)" = success ] && [ "$(prop ExecMainStatus)" = 0 ] && [ "$(prop InactiveEnterTimestampMonotonic)" != "$durum_once" ]; }; then
  journalctl -u wc-durum --no-pager -n 20; hata "wc-durum bu kurulumda başarıyla bitmedi: $(prop ActiveState) $(prop Result) $(prop ExecMainStatus)"
fi
yedek=$(ls -1 "$DB".yedek-* 2>/dev/null | tail -n1)
[ -n "$yedek" ] || hata "veritabanı yedeği yok"
[ "$(stat -c '%U:%G %a' "$yedek")" = "$(stat -c '%U:%G %a' "$DB")" ] || hata "yedek sahip/izin farklı: $(stat -c '%U:%G %a' "$yedek") <> $(stat -c '%U:%G %a' "$DB")"
denetle_tamam "$yedek" || hata "yedek denetimi TAMAM değil"
[ -f "$GERI/kur.sh" ] || hata "$GERI/kur.sh saklanmadı"
py_yerinde || hata "Python dosyaları/bağları silinmiş"
denetle_tamam "$DB" || hata "Rust, Python'un bıraktığı canlı veritabanını okuyamadı/TAMAM değil"
GD4=$(grep -o "$GERI/[0-9]\{8\}-[0-9]\{6\}" "$LOG/kur-gecis.log" | head -n1)
[ -d "$GD4" ] && python_dizini "$(basename "$GD4")" || hata "geçişin geri dönüş dizini Python dönemi değil: $GD4"
tamam "Rust sunucu, 7 hizmet/6 zamanlayıcı, wc-durum bitti, yedek $(basename "$yedek")"

# Rust çalışırken veritabanına yaz (kolay bağlama bildirimi); adım 5'te Python bunu okumalı
bildir() {
  curl -s -m 10 -o /dev/null -w '%{http_code}' -H 'Host: api.wificorrect.com' -H 'Content-Type: application/json' -H 'X-Forwarded-For: 203.0.113.5' \
    -d "{\"kimlik\":\"$(printf '1%.0s' $(seq 32))\",\"sir\":\"$(printf 'a%.0s' $(seq 64))\",\"surum\":\"0.4.29\",\"wan_mac\":\"00:0e:c4:ce:a0:9b\",\"wg_pub\":\"$(printf 'A%.0s' $(seq 43))=\",\"ssh_pub\":\"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI prova\"}" \
    http://127.0.0.1:8081/api/bekliyor
}
satir_say() { /usr/bin/python3 -I -c 'import sqlite3,sys;print(sqlite3.connect(sys.argv[1]).execute("select count(*) from bekleyen").fetchone()[0])' "$DB" 2>&1; }
once=$(satir_say)
b=$(bildir)
sonra=$(satir_say)
echo "Rust'a bekliyor bildirimi: HTTP $b; bekleyen satırı $once -> $sonra"

# ---- 5. elle geri dönüş ---------------------------------------------------------------------------------------------
adim 5 elle_geri_donus
[ "$b" = 200 ] && [ "$sonra" -gt "$once" ] || hata "Rust veritabanına yazamadı (HTTP $b, $once -> $sonra); bekleyen satırı Python'a okutulamaz"
bash "$GERI/kur.sh" --geri-don "$(basename "$GD4")" >"$LOG/geri-don-elle.log" 2>&1; k=$?
sed 's/^/    | /' "$LOG/geri-don-elle.log" | tail -n 25
[ "$k" = 0 ] || hata "--geri-don çıkış $k"
python_servis || hata "elle geri dönüşten sonra Python 200 vermiyor: $(exec_start)"
[ "$(satir_say)" = "$sonra" ] || hata "veritabanı satırı değişti"
if journalctl -u wc-merkez --no-pager --since "-2min" 2>/dev/null | grep -qE 'Traceback|OperationalError|DatabaseError'; then
  journalctl -u wc-merkez --no-pager --since "-2min" | tail -n 30; hata "Python, Rust'ın yazdığı veritabanında hata verdi"
fi
# Python bu veriyi API'den okuyabiliyor mu: aynı bildirimi tekrar et, "var olan kayıt" cevabı 200 gelmeli
b2=$(bildir); [ "$b2" = 200 ] || hata "Python aynı bildirime HTTP $b2 verdi"
tamam "Python, Rust'ın yazdığı veritabanıyla açıldı (bekleyen=$sonra), bildirime 200"

# ---- 6. yeniden Rust ve budama ---------------------------------------------------------------------------------------
adim 6 yeniden_rust_ve_budama
# Not: brifte 6 kez; ilk koşu Python dönemi birimlerini yedeklediği için budama sayacına girmez, bu yüzden 7 koşu (budama görülsün)
BUDANDI=
for i in 1 2 3 4 5 6 7; do
  kur p4 "gecis-$i"; k=$?
  [ "$k" = 0 ] || hata "$i. koşu: kur.sh çıkış $k"
  rust_servis || hata "$i. koşu: Rust 200 vermiyor"
  BUDANDI="$BUDANDI$(grep 'budandı' "$LOG/kur-gecis-$i.log")"$'\n'
done
[ -d "$GERI/$(basename "$GD4")" ] || hata "Python dizini budandı: $GD4"
diger=0 py=0
for d in $(find "$GERI" -mindepth 1 -maxdepth 1 -type d -printf '%f\n'); do if python_dizini "$d"; then py=$((py + 1)); else diger=$((diger + 1)); fi; done
echo "geri dönüş dizinleri: Python dönemi $py, diğer $diger"
[ "$diger" -le 5 ] || hata "Python olmayan dizin sayısı $diger > 5"
[ -n "$(echo "$BUDANDI" | tr -d '\n ')" ] || hata "hiç dizin budanmadı (çıktıda 'budandı' yok)"
echo "budanan:"; echo "$BUDANDI" | sed 's/^/    /'
for d in $(echo "$BUDANDI" | grep -o '[0-9]\{8\}-[0-9]\{6\}'); do [ ! -e "$GERI/$d" ] || hata "budandı denen dizin duruyor: $d"; done
py_yerinde || hata "Python dosyaları bozuldu"
tamam "Python dizini duruyor, diğer=$diger, budanan: $(echo "$BUDANDI" | grep -c budandı)"

# ---- 7. kurulum adımı hatası -> kendiliğinden geri dönüş -------------------------------------------------------------
adim 7 adim_hatasi_otomatik_donus
HEDEF=/etc/cron.daily/hotspot-arsiv-saklama
[ -f "$HEDEF" ] || hata "ön koşul: $HEDEF yok"
# geri dönüşün gerçekten yapıldığını görmek için: mevcut birime işaret koy (yeni kurulum ezer, geri dönüş işareti geri getirir)
echo "# prova-isareti" >>"$BIRIMLER/wc-durum.service"; systemctl daemon-reload
CHATTR_OK=0
if chattr +i "$HEDEF" 2>/dev/null && ! (echo x >>"$HEDEF") 2>/dev/null; then CHATTR_OK=1; fi
if [ "$CHATTR_OK" = 0 ]; then
  chattr -i "$HEDEF" 2>/dev/null
  sed -i '/prova-isareti/d' "$BIRIMLER/wc-durum.service"; systemctl daemon-reload
  atlandi "chattr +i bu dosya sisteminde çalışmıyor; ADIM 4 hatası enjekte edilemedi"
else
  kur p4 adim-hatasi; k=$?
  chattr -i "$HEDEF"
  [ "$k" != 0 ] || hata "kur.sh başarısız adımda 0 döndü"
  grep -q 'GERİ DÖNÜLDÜ' "$LOG/kur-adim-hatasi.log" || hata "'GERİ DÖNÜLDÜ' iletisi yok"
  grep -q 'prova-isareti' "$BIRIMLER/wc-durum.service" || hata "birim geri konmadı (işaret yok)"
  [ -s "$GERI/son-geri-donus.txt" ] || hata "son-geri-donus.txt yok"
  cmp -s "$GERCEK_IKILI" /usr/local/bin/wificorrect-merkez || hata "ikili geri konan sürüm değil"
  rust_servis || hata "geri dönüşten sonra Rust 200 vermiyor"
  sed -i '/prova-isareti/d' "$BIRIMLER/wc-durum.service"; systemctl daemon-reload
  tamam "kur.sh çıkış $k; ERR/EXIT tuzağı önceki kuruluma döndü"
fi

# ---- 7b. sinyalle kesilme ve ölü boru ----------------------------------------------------------------------------------
adim 7b sinyal_ve_olu_boru
paket_yap p7 asili
sinyal_dene() { # sinyal_dene AD KİP: kur.sh arka planda; ADIM 5 çıktısından sonra kesilir; sonra birimler geri dönmeli
  local ad=$1 kip=$2 l=$LOG/kur-$1.log pid
  : >"$l"; rm -f "$GERI/son-geri-donus.txt"; echo "# prova-isareti" >>"$BIRIMLER/wc-durum.service"; systemctl daemon-reload
  if [ "$kip" = sigterm ]; then
    bash "$CALISMA/p7/merkez/kur.sh" >"$l" 2>&1 & pid=$!
  else
    ( bash "$CALISMA/p7/merkez/kur.sh" 2>&1 | tee "$l" >/dev/null ) & pid=$!
  fi
  local bekle=$((SECONDS + 60))
  until grep -q '^ADIM 5:' "$l"; do [ "$SECONDS" -lt "$bekle" ] || { kill -9 "$pid" 2>/dev/null; hata "$ad: ADIM 5'e gelinmedi"; }; sleep 0.5; done
  sleep 3   # sağlık denetimi (200 beklemesi) sürüyor, tuzak etkin
  if [ "$kip" = sigterm ]; then
    kill -TERM "$pid"
  else
    pkill -9 -f "tee $l" || hata "$ad: tee bulunamadı"
  fi
  wait "$pid" 2>/dev/null
  # kur.sh geri dönüşü bitirene kadar bekle (sonuç dosyası tamamlanır)
  local b2=$((SECONDS + 100))
  until grep -qE 'GERİ DÖNÜLDÜ|geri dönüş de tam olmadı' "$GERI/son-geri-donus.txt" 2>/dev/null && ! pgrep -f "p7/merkez/kur.sh" >/dev/null; do
    [ "$SECONDS" -lt "$b2" ] || hata "$ad: geri dönüş sonucu gelmedi: $(tail -n 5 "$GERI/son-geri-donus.txt" 2>&1)"; sleep 1
  done
  grep -q 'GERİ DÖNÜLDÜ' "$GERI/son-geri-donus.txt" || hata "$ad: geri dönüş tam olmadı: $(tail -n 5 "$GERI/son-geri-donus.txt")"
  grep -q 'prova-isareti' "$BIRIMLER/wc-durum.service" || hata "$ad: birimler geri konmadı"
  cmp -s "$GERCEK_IKILI" /usr/local/bin/wificorrect-merkez || hata "$ad: ikili geri konan sürüm değil"
  rust_servis || hata "$ad: geri dönüşten sonra Rust 200 vermiyor"
  sed -i '/prova-isareti/d' "$BIRIMLER/wc-durum.service"; systemctl daemon-reload
}
sinyal_dene sinyal-sigterm sigterm
sinyal_dene sinyal-olu-boru borulu
tamam "SIGTERM ve ölü boru: iki durumda da önceki kuruluma dönüldü"

# ---- 8. son durum --------------------------------------------------------------------------------------------------
adim 8 son_durum
systemctl is-active wc-merkez >/dev/null || hata "wc-merkez etkin değil"
rust_servis || hata "wc-merkez 200 vermiyor"
denetle_tamam "$DB" || hata "veritabani-denetle TAMAM değil"
if journalctl -u wc-merkez --no-pager 2>/dev/null | grep -qi 'panicked'; then journalctl -u wc-merkez --no-pager | grep -i -B2 -A5 panicked | head -n 30; hata "wc-merkez günlüğünde panic var"; fi
[ "$(for d in $(find "$GERI" -mindepth 1 -maxdepth 1 -type d -printf '%f\n'); do python_dizini "$d" || echo x; done | wc -l)" -le 5 ] || hata "Python olmayan geri dönüş dizini > 5"
tamam
echo "PROVA BITTI: bütün adımlar geçti"
