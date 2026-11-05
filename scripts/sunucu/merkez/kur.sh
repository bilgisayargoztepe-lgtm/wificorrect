#!/bin/bash
# Yönetim merkezi (Rust ikilisi wificorrect-merkez) kurulumu/güncellemesi/geri dönüşü (Ubuntu, root).
# scripts/sunucu/merkez-kur.sh çağırır; boş sunucuda da, güncellemede de çalışır; tekrar çalıştırmak güvenli.
#   sudo bash kur.sh                    kur / güncelle (adım 1-6, aşağıda)
#   sudo bash kur.sh --yalniz-denetle   yalnız adım 1-2: veritabanı yedeği + yeni ikiliyle yedek kopyada denetim; canlıya dokunmaz
#   sudo bash kur.sh --geri-don [ZAMAN] en yeni (ya da adı verilen) geri dönüş dizinindeki birimleri/ikilileri geri koyar
# Kurulum başarılı olursa betiğin kendisi /var/lib/wificorrect/geri-donus/kur.sh olarak saklanır: checkout olmadan
#   sudo bash /var/lib/wificorrect/geri-donus/kur.sh --geri-don      (acil dönüş)
# Python merkezine (/usr/local/lib/wificorrect/*.py ve /usr/local/sbin/wc-* bağları) bu betik ne kurar ne dokunur;
# geri dönüş onlara dayanır. Silinmeleri ayrı bir iştir (Task 16).
set -euo pipefail
BU_BETIK=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
cd "$(dirname "$0")"
YALNIZ_DENETLE=0 GERI_DON=0 ZAMAN=
while [ $# -gt 0 ]; do
  case $1 in
    --yalniz-denetle) YALNIZ_DENETLE=1 ;;
    --geri-don) GERI_DON=1; if [ $# -gt 1 ] && [ "${2#--}" = "$2" ]; then ZAMAN=$2; shift; fi ;;
    *) echo "Kullanım: kur.sh [--yalniz-denetle | --geri-don [ZAMAN]]" >&2; exit 2 ;;
  esac
  shift
done
if [ -n "$ZAMAN" ] && ! [[ $ZAMAN =~ ^[0-9]{8}-[0-9]{6}$ ]]; then echo "HATA: ZAMAN YYYYmmdd-HHMMSS biçiminde olmalı: $ZAMAN" >&2; exit 2; fi
[ "$(id -u)" = 0 ] || { echo "HATA: kur.sh root olarak çalışmalı (sudo bash kur.sh)" >&2; exit 1; }

PAKET_IKILI=$PWD/wificorrect-merkez
SUNUCU_BETIGI=$PWD/../wificorrect-sunucu
IKILI=/usr/local/bin/wificorrect-merkez
SUNUCU=/usr/local/sbin/wificorrect-sunucu
BIRIMLER=/etc/systemd/system
VERI=/var/lib/wificorrect
DB=$VERI/merkez/merkez.db
GERI=$VERI/geri-donus
GERI_KUR=$GERI/kur.sh
GD= GERI_TUZAK=0 YEDEK_TMP=
SON_GERI=$GERI/son-geri-donus.txt
ZAMANLAYICILAR="wc-kuyruk.path wc-durum.timer wc-istatistik.timer wc-saglik.timer wc-listeler.timer wc-surumler.timer"

# --- yardımcılar -----------------------------------------------------------------------------------------------------
# yönetim sayfası 200 vermeli (~20 sn yeniden denenir; her deneme en çok 5 sn sürer)
saglik_denetle() {
  local k=000 bitis=$((SECONDS + 20))
  while :; do
    k=$(curl -s -m 5 --connect-timeout 2 -o /dev/null -w '%{http_code}' -H 'Host: yonetim.wificorrect.com' http://127.0.0.1:8081/giris 2>/dev/null || true)
    [ "$k" = 200 ] && break
    [ "$SECONDS" -ge "$bitis" ] && break
    sleep 2
  done
  echo "sağlık denetimi: GET /giris -> HTTP $k"
  [ "$k" = 200 ]
}

# geri dönüş dizinleri (kur.sh dosyası ve başka dosyalar sayılmaz), eskiden yeniye
geri_dizinleri() { find "$GERI" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort || true; }
# Python dönemi dizini: birimleri python3 ya da /usr/local/sbin/wc-* bağlarını çağırır (hiç budanmaz)
python_donemi() { grep -qsE 'python3|/usr/local/sbin/wc-' "$GERI/$1"/wc-*.service; }

# dosyayı yerine yeni bir dosya olarak koyar (çalışan ikilinin üzerine yazmadan): dosya_koy MOD KAYNAK HEDEF
dosya_koy() {
  install -m "$1" "$2" "$3.yeni" && mv -f "$3.yeni" "$3"
}

# bir geri dönüş dizinindeki dosyaları yerlerine koyar, servisleri yeniden başlatır, sağlığı denetler.
# Dosya/komut başına hatalar sayılır ve sonunda bildirilir (set -e'ye güvenilmez: EXIT tuzağından da çağrılır).
geri_don_uygula() {
  local dizin=$1 f ad hata=0
  [ -d "$dizin" ] || { echo "HATA: geri dönüş dizini yok: $dizin" >&2; return 1; }
  echo "geri dönüş: $dizin"
  for f in "$dizin"/*; do
    [ -f "$f" ] || continue
    ad=$(basename "$f")
    case $ad in
      wc-*.service|wc-*.timer|wc-*.path) dosya_koy 644 "$f" "$BIRIMLER/$ad" || { echo "HATA: $ad geri konamadı" >&2; hata=$((hata + 1)); continue; } ;;
      wificorrect-sunucu) dosya_koy 755 "$f" "$SUNUCU" || { echo "HATA: $ad geri konamadı" >&2; hata=$((hata + 1)); continue; } ;;
      wificorrect-merkez) dosya_koy 755 "$f" "$IKILI" || { echo "HATA: $ad geri konamadı" >&2; hata=$((hata + 1)); continue; } ;;
      *) echo "uyarı: tanınmayan geri dönüş dosyası atlandı: $ad" >&2; continue ;;
    esac
    echo "  geri konan: $ad"
  done
  # dizin "önceki kurulumun tamamı" demektir: Rust ikilisi yoksa (Python dönemi) başarısız kurulumun ikilisi yerinde kalmasın
  # (sonraki kurulum onu "önceki kurulum" sanıp geri dönüş dizinine kopyalardı). wificorrect-sunucu'ya dokunulmaz.
  if [ ! -e "$dizin/wificorrect-merkez" ] && [ -e "$IKILI" ]; then
    if rm -f "$IKILI"; then echo "  kaldırılan: wificorrect-merkez (dizinde yok)"
    else echo "HATA: $IKILI silinemedi" >&2; hata=$((hata + 1)); fi
  fi
  systemctl daemon-reload || { echo "HATA: daemon-reload" >&2; hata=$((hata + 1)); }
  # shellcheck disable=SC2086
  systemctl enable -q --now $ZAMANLAYICILAR || { echo "HATA: zamanlayıcılar etkinleştirilemedi" >&2; hata=$((hata + 1)); }
  systemctl enable -q wc-merkez || { echo "HATA: wc-merkez enable" >&2; hata=$((hata + 1)); }
  systemctl restart wc-merkez || { echo "HATA: wc-merkez restart" >&2; hata=$((hata + 1)); }
  echo "wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path)"
  saglik_denetle || hata=$((hata + 1))
  [ "$hata" -eq 0 ] || echo "HATA: geri dönüşte $hata hata oldu" >&2
  [ "$hata" -eq 0 ]
}

# EXIT tuzağı: geri dönüş dizini oluştuktan sonra 200 denetimine kadar her çıkış (hata, set -e, Ctrl-C, SSH kopması, SIGTERM)
# önceki kuruluma döner. Başarı yolu önce GERI_TUZAK=0 yapar, bu yüzden GERI_TUZAK=1 iken "başarılı çıkış" yoktur ($? sinyalde 0 olabilir).
# SIGPIPE/HUP/INT/TERM yok sayılır (tee ölse de geri dönüş yarım kalmaz); çıktı kalıcı olarak $SON_GERI dosyasına da yazılır.
kurtar() {
  local k=$? rc
  trap - EXIT
  trap '' PIPE HUP INT TERM
  set +e
  [ -z "$YEDEK_TMP" ] || rm -rf "$YEDEK_TMP"
  if [ "$GERI_TUZAK" = 1 ] && [ -n "$GD" ]; then
    # son savunma hattı: sonuç dosyası açılamazsa (salt okunur dosya sistemi, inode tükenmesi) geri dönüş yine çalışır
    DOSYALI=1
    if ! ( umask 077; : >>"$SON_GERI" ) 2>/dev/null; then DOSYALI=0; SON_GERI=/dev/null; echo "sonuç dosyası açılamadı, çıktı yalnız terminale" >&2; fi
    echo "kurulum kesildi/başarısız; geri dönülüyor (~30 sn sürebilir); sonuç: $SON_GERI" >&2
    ( umask 077; echo "kurulum başarısız ya da kesildi; önceki kuruluma geri dönülüyor: $GD" >"$SON_GERI" ) 2>/dev/null
    if [ "$DOSYALI" = 1 ]; then geri_don_uygula "$GD" >>"$SON_GERI" 2>&1; else geri_don_uygula "$GD" >&2 2>&1; fi
    rc=$?
    if [ "$rc" -eq 0 ]; then son="GERİ DÖNÜLDÜ: önceki kurulum çalışıyor; yeni sürüm kurulmadı (veritabanı yedeği: ${yedek:-yok})."
    else son="HATA: geri dönüş de tam olmadı; elle bakın (kur.sh --geri-don)."; fi
    echo "$son" >>"$SON_GERI" 2>/dev/null
    if [ "$DOSYALI" = 1 ]; then cat "$SON_GERI" >&2 2>/dev/null || true; else echo "$son" >&2; fi
    # sinyalle kesildiyse (k > 128: 143 TERM, 141 PIPE, 129 HUP, 130 INT) geleneksel sinyal kodu korunur, aksi halde 1
    [ "$k" -gt 128 ] && exit "$k"
    exit 1
  fi
  exit "$k"
}
trap kurtar EXIT

# Caddyfile geçersizse canlı siteleri düşürmesin: önce doğrula, canlı dosyaya dokunmadan dur
caddy_dogrula() {
  caddy validate --adapter caddyfile --config Caddyfile >/dev/null 2>&1 && return 0
  echo "HATA: Caddyfile geçersiz, kurulum durdu (canlı Caddyfile değişmedi):" >&2
  caddy validate --adapter caddyfile --config Caddyfile >&2 || true
  return 1
}

# ANA AKIŞ
# --- geri dönüş kipi -------------------------------------------------------------------------------------------------
if [ "$GERI_DON" = 1 ]; then
  if [ -z "$ZAMAN" ]; then ZAMAN=$(geri_dizinleri | tail -n 1); fi
  [ -n "$ZAMAN" ] || { echo "HATA: geri dönüş dizini yok ($GERI boş); geri dönülecek bir şey yok" >&2; exit 1; }
  echo "Mevcut geri dönüş dizinleri (eskiden yeniye):"
  for d in $(geri_dizinleri); do if python_donemi "$d"; then echo "  $d (Python)"; else echo "  $d"; fi; done
  echo "Not: Rust iki kez kurulduysa en yeni dizin önceki Rust'ın birimleridir; Python merkezine dönmek için '(Python)' işaretli"
  echo "dizinin adını verin (kur.sh --geri-don <ad>)."
  geri_don_uygula "$GERI/$ZAMAN" || { echo "HATA: geri dönüşten sonra sağlık denetimi geçmedi" >&2; exit 1; }
  echo "geri dönüş tamam: $ZAMAN"
  exit 0
fi

[ -x "$PAKET_IKILI" ] || chmod 755 "$PAKET_IKILI" 2>/dev/null || true
[ -x "$PAKET_IKILI" ] || { echo "HATA: paketteki wificorrect-merkez ikilisi yok ya da çalıştırılamıyor: $PAKET_IKILI" >&2; exit 1; }
echo "yeni ikili sürümü: $("$PAKET_IKILI" surum)"

# ADIM 1: veritabanı yedeği. Önce root'a ait gizli geçici dizinde üretilir (wcpanel'in yazabildiği $VERI/merkez'de sahip/izin
# verilmez: yol üzerinden chown, bağlantı yarışına açık olurdu), sahip/izin orada verilir, sonra rename ile son adına taşınır.
yedek=
if [ -f "$DB" ]; then
  yedek=$DB.yedek-$(date +%Y%m%d-%H%M%S)
  YEDEK_TMP=$(mktemp -d "$VERI/.yedek.XXXXXX")
  yedek_gecici=$YEDEK_TMP/yedek.db
  # tutarlı anlık görüntü (çalışan merkez yazarken de): paketteki ikili çalışmazsa kurulum durur, canlıya dokunulmaz
  if ! "$PAKET_IKILI" veritabani-yedekle "$DB" "$yedek_gecici"; then
    echo "HATA: veritabanı yedeği alınamadı (ya da ikili çalıştırılamadı). Canlıya dokunulmadı." >&2
    exit 1
  fi
  chown --reference="$DB" "$yedek_gecici"
  chmod --reference="$DB" "$yedek_gecici"
  echo "ADIM 1: veritabanı yedeği alındı (root'a ait geçici dizinde): $yedek"
else
  echo "ADIM 1: veritabanı yok (boş sunucu), yedek atlandı"
fi

# ADIM 2: yeni ikili, taşımadan ÖNCEKİ root'a ait geçici kopyada canlı veriyi denetler (hata -> canlıya dokunulmaz; yedek yine de
# son adına taşınır, incelenebilsin)
if [ -n "$yedek" ]; then
  cikti=$(mktemp)
  if "$PAKET_IKILI" veritabani-denetle "$yedek_gecici" --dosyalar "$VERI" >"$cikti" 2>&1; then
    tail -n 1 "$cikti"
    mv -f -T "$yedek_gecici" "$yedek"
    echo "ADIM 2: veritabanı denetimi TAMAM; yedek: $yedek"
    rm -f "$cikti"
  else
    cat "$cikti" >&2; rm -f "$cikti"
    mv -f -T "$yedek_gecici" "$yedek"
    echo "HATA: veritabanı denetimi başarısız (ya da ikili çalıştırılamadı). Canlıya dokunulmadı; yedek: $yedek" >&2
    exit 1
  fi
else
  echo "ADIM 2: veritabanı yok, denetim atlandı"
fi
# Caddy kuruluysa Caddyfile'ı da şimdi doğrula (bir şey değişmeden)
if command -v caddy >/dev/null 2>&1; then caddy_dogrula || exit 1; fi
if [ "$YALNIZ_DENETLE" = 1 ]; then
  echo "--yalniz-denetle: adım 1-2 tamam; canlıya dokunulmadı."
  exit 0
fi

# ADIM 3: geri dönüş kopyası (Python dönemi dizinleri hiç budanmaz; kalanlardan en yeni 5 tutulur; hiçbir şey yoksa dizin oluşmaz)
for f in "$BIRIMLER"/wc-*.service "$BIRIMLER"/wc-*.timer "$BIRIMLER"/wc-*.path "$SUNUCU" "$IKILI"; do
  [ -f "$f" ] || continue
  if [ -z "$GD" ]; then GD=$GERI/$(date +%Y%m%d-%H%M%S); install -d -m 700 "$GERI" "$GD"; GERI_TUZAK=1; fi
  cp -p "$f" "$GD/"
done
if [ -n "$GD" ]; then
  echo "ADIM 3: geri dönüş kopyası: $GD"
  diger=()
  for d in $(geri_dizinleri); do python_donemi "$d" || diger+=("$d"); done
  if [ "${#diger[@]}" -gt 5 ]; then
    for d in "${diger[@]:0:${#diger[@]}-5}"; do rm -rf "${GERI:?}/$d"; echo "  budandı (eski geri dönüş dizini): $d"; done
  fi
else
  echo "ADIM 3: kopyalanacak eski birim/ikili yok (boş sunucu)"
fi

# ADIM 4: ikiliyi ve birimleri kur
sed -i 's/\r$//' ./*.service ./*.timer ./*.path ./*.conf Caddyfile hotspot-arsiv-saklama "$SUNUCU_BETIGI"
command -v caddy >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q caddy >/dev/null
command -v fail2ban-client >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q fail2ban >/dev/null
command -v curl >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q curl >/dev/null
caddy_dogrula || { GERI_TUZAK=0; [ -z "$GD" ] || rm -rf "$GD"; exit 1; }
id wcpanel >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin wcpanel
install -d -m 755 $VERI $VERI/istatistik
install -d -o root -g wcpanel -m 750 $VERI/detay $VERI/kuyruk-sonuc $VERI/rapor
install -d -m 700 $VERI/rapor-ham   # takma kimlikli ham özetler + sır: yalnızca root
install -d -o wcpanel -g wcpanel -m 700 $VERI/merkez $VERI/kuyruk
dosya_koy 755 "$PAKET_IKILI" "$IKILI"
dosya_koy 755 "$SUNUCU_BETIGI" "$SUNUCU"
install -m 644 wc-merkez.service wc-kuyruk.path wc-kuyruk.service wc-durum.service wc-durum.timer \
  wc-istatistik.service wc-istatistik.timer wc-saglik.service wc-saglik.timer \
  wc-listeler.service wc-listeler.timer wc-surumler.service wc-surumler.timer $BIRIMLER/
if [ -f /etc/caddy/Caddyfile ]; then
  [ -e /etc/caddy/Caddyfile.eski ] || cp -p /etc/caddy/Caddyfile /etc/caddy/Caddyfile.eski
  cp -p /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.yedek-$(date +%Y%m%d-%H%M%S)"
fi
install -m 644 Caddyfile /etc/caddy/Caddyfile
install -m 755 hotspot-arsiv-saklama /etc/cron.daily/
install -m 644 fail2ban-wc-merkez-filtre.conf /etc/fail2ban/filter.d/wc-merkez.conf
install -m 644 fail2ban-wc-merkez.conf /etc/fail2ban/jail.d/wc-merkez.local
echo "ADIM 4: ikili ($IKILI), wificorrect-sunucu ve birimler kuruldu"

# ADIM 5: başlat
systemctl daemon-reload
# shellcheck disable=SC2086
systemctl enable -q --now $ZAMANLAYICILAR
systemctl enable -q wc-merkez
systemctl restart wc-merkez
echo "ADIM 5: wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path)"

# ADIM 6: sağlık; geçmezse (geri dönüş kopyası varsa) EXIT tuzağı kendiliğinden geri döner
if ! saglik_denetle; then
  echo "HATA: yönetim sayfası 200 vermedi." >&2
  systemctl status wc-merkez --no-pager -l 2>&1 | tail -n 15 >&2 || true
  [ -n "$GD" ] || echo "Geri dönülecek önceki kurulum yok (boş sunucu); günlük: journalctl -u wc-merkez" >&2
  exit 1
fi
GERI_TUZAK=0
# sağlık geçti: ikincil işler (hata kurulumu geri çevirmez); betik kalıcı saklanır (checkout olmadan acil dönüş için)
if [ -n "$GD" ]; then install -m 700 "$BU_BETIK" "$GERI_KUR" || echo "uyarı: kur.sh $GERI_KUR olarak saklanamadı" >&2; fi
systemctl start --no-block wc-durum.service || true
# ilk kurulumda listeler hemen üretilsin (cihazlar beklemesin); arka planda, kurulumu bekletmez
[ -s $VERI/merkez/listeler/kumar.json ] || systemctl start --no-block wc-listeler.service || true
systemctl reload caddy 2>/dev/null || systemctl restart caddy
systemctl reload fail2ban 2>/dev/null || systemctl restart fail2ban
[ -s $VERI/merkez/yonetici.json ] || echo "Yönetici hesabı yok: sudo wificorrect-sunucu yonetici-parola <kullanıcı-adı>"
echo "kurulum tamam: wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path), geri dönüş dizini: ${GD:-yok}"
[ -z "$GD" ] || echo "Acil dönüş (checkout gerekmez): sudo bash $GERI_KUR --geri-don"
