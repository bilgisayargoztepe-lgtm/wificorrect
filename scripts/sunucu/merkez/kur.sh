#!/bin/bash
# Yönetim merkezi (Rust ikilisi wificorrect-merkez) kurulumu/güncellemesi/geri dönüşü (Ubuntu, root).
# scripts/sunucu/merkez-kur.sh çağırır; boş sunucuda da, güncellemede de çalışır; tekrar çalıştırmak güvenli.
#   sudo bash kur.sh                    kur / güncelle (adım 1-6, aşağıda)
#   sudo bash kur.sh --yalniz-denetle   yalnız adım 1-2: veritabanı yedeği + yeni ikiliyle yedek kopyada denetim; canlıya dokunmaz
#   sudo bash kur.sh --geri-don [ZAMAN] en yeni (ya da adı verilen) geri dönüş dizinindeki birimleri/ikilileri geri koyar
# Python merkezine (/usr/local/lib/wificorrect/*.py ve /usr/local/sbin/wc-* bağları) bu betik ne kurar ne dokunur;
# geri dönüş onlara dayanır. Silinmeleri ayrı bir iştir (Task 16).
set -euo pipefail
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
[ "$(id -u)" = 0 ] || { echo "HATA: kur.sh root olarak çalışmalı (sudo bash kur.sh)" >&2; exit 1; }

PAKET_IKILI=$PWD/wificorrect-merkez
SUNUCU_BETIGI=$PWD/../wificorrect-sunucu
IKILI=/usr/local/bin/wificorrect-merkez
SUNUCU=/usr/local/sbin/wificorrect-sunucu
BIRIMLER=/etc/systemd/system
VERI=/var/lib/wificorrect
DB=$VERI/merkez/merkez.db
GERI=$VERI/geri-donus
ZAMANLAYICILAR="wc-kuyruk.path wc-durum.timer wc-istatistik.timer wc-saglik.timer wc-listeler.timer wc-surumler.timer"

# --- yardımcılar -----------------------------------------------------------------------------------------------------
# yönetim sayfası 200 vermeli (en çok ~20 sn yeniden denenir)
saglik_denetle() {
  local k=000 i
  for i in 1 2 3 4 5 6 7 8 9 10; do
    k=$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: yonetim.wificorrect.com' http://127.0.0.1:8081/giris 2>/dev/null || true)
    [ "$k" = 200 ] && break
    sleep 2
  done
  echo "sağlık denetimi: GET /giris -> HTTP $k"
  [ "$k" = 200 ]
}

# dosyayı yerine yeni bir dosya olarak koyar (çalışan ikilinin üzerine yazmadan): dosya_koy MOD KAYNAK HEDEF
dosya_koy() {
  install -m "$1" "$2" "$3.yeni" && mv -f "$3.yeni" "$3"
}

# bir geri dönüş dizinindeki dosyaları yerlerine koyar, servisleri yeniden başlatır, sağlığı denetler
geri_don_uygula() {
  local dizin=$1 f ad
  [ -d "$dizin" ] || { echo "HATA: geri dönüş dizini yok: $dizin" >&2; return 1; }
  echo "geri dönüş: $dizin"
  for f in "$dizin"/*; do
    [ -f "$f" ] || continue
    ad=$(basename "$f")
    case $ad in
      wc-*.service|wc-*.timer|wc-*.path) dosya_koy 644 "$f" "$BIRIMLER/$ad" ;;
      wificorrect-sunucu) dosya_koy 755 "$f" "$SUNUCU" ;;
      wificorrect-merkez) dosya_koy 755 "$f" "$IKILI" ;;
      *) echo "uyarı: tanınmayan geri dönüş dosyası atlandı: $ad" >&2; continue ;;
    esac
    echo "  geri konan: $ad"
  done
  systemctl daemon-reload
  # shellcheck disable=SC2086
  systemctl enable -q --now $ZAMANLAYICILAR
  systemctl enable -q wc-merkez
  systemctl restart wc-merkez
  echo "wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path)"
  saglik_denetle
}

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
  if [ -z "$ZAMAN" ]; then ZAMAN=$(ls -1 "$GERI" 2>/dev/null | sort | tail -n 1 || true); fi
  [ -n "$ZAMAN" ] || { echo "HATA: geri dönüş dizini yok ($GERI boş); geri dönülecek bir şey yok" >&2; exit 1; }
  echo "Mevcut geri dönüş dizinleri (eskiden yeniye):"
  ls -1 "$GERI" | sort | sed 's/^/  /'
  echo "Not: Rust iki kez kurulduysa en yeni dizin önceki Rust'ın birimleridir; Python merkezine dönmek için en ESKİ dizinin"
  echo "adını verin (kur.sh --geri-don <ad>)."
  geri_don_uygula "$GERI/$ZAMAN" || { echo "HATA: geri dönüşten sonra sağlık denetimi geçmedi" >&2; exit 1; }
  echo "geri dönüş tamam: $ZAMAN"
  exit 0
fi

[ -x "$PAKET_IKILI" ] || chmod 755 "$PAKET_IKILI" 2>/dev/null || true
[ -x "$PAKET_IKILI" ] || { echo "HATA: paketteki wificorrect-merkez ikilisi yok ya da çalıştırılamıyor: $PAKET_IKILI" >&2; exit 1; }
echo "yeni ikili sürümü: $("$PAKET_IKILI" surum)"

# ADIM 1: veritabanı yedeği
yedek=
if [ -f "$DB" ]; then
  yedek=$DB.yedek-$(date +%Y%m%d-%H%M%S)
  cp -p "$DB" "$yedek"
  echo "ADIM 1: veritabanı yedeği: $yedek"
else
  echo "ADIM 1: veritabanı yok (boş sunucu), yedek atlandı"
fi

# ADIM 2: yeni ikili yedek kopyada canlı veriyi denetler (hata -> canlıya dokunulmaz)
if [ -n "$yedek" ]; then
  cikti=$(mktemp)
  if "$PAKET_IKILI" veritabani-denetle "$yedek" --dosyalar "$VERI" >"$cikti" 2>&1; then
    tail -n 1 "$cikti"
    echo "ADIM 2: veritabanı denetimi TAMAM"
    rm -f "$cikti"
  else
    cat "$cikti" >&2; rm -f "$cikti"
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

# ADIM 3: geri dönüş kopyası (en yeni 5 dizin tutulur; hiçbir şey yoksa dizin oluşmaz)
GD=
for f in "$BIRIMLER"/wc-*.service "$BIRIMLER"/wc-*.timer "$BIRIMLER"/wc-*.path "$SUNUCU" "$IKILI"; do
  [ -f "$f" ] || continue
  if [ -z "$GD" ]; then GD=$GERI/$(date +%Y%m%d-%H%M%S); install -d -m 700 "$GERI" "$GD"; fi
  cp -p "$f" "$GD/"
done
if [ -n "$GD" ]; then
  echo "ADIM 3: geri dönüş kopyası: $GD"
  ls -1 "$GERI" | sort | head -n -5 | while read -r eski; do rm -rf "${GERI:?}/$eski"; done
else
  echo "ADIM 3: kopyalanacak eski birim/ikili yok (boş sunucu)"
fi

# ADIM 4: ikiliyi ve birimleri kur
sed -i 's/\r$//' ./*.service ./*.timer ./*.path ./*.conf Caddyfile hotspot-arsiv-saklama "$SUNUCU_BETIGI"
command -v caddy >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q caddy >/dev/null
command -v fail2ban-client >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q fail2ban >/dev/null
command -v curl >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q curl >/dev/null
caddy_dogrula || { [ -z "$GD" ] || rm -rf "$GD"; exit 1; }
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
[ -f /etc/caddy/Caddyfile ] && cp -n /etc/caddy/Caddyfile /etc/caddy/Caddyfile.eski
[ -f /etc/caddy/Caddyfile ] && cp -p /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.yedek-$(date +%Y%m%d-%H%M%S)"
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

# ADIM 6: sağlık; geçmezse (geri dönüş kopyası varsa) kendiliğinden geri dön
if ! saglik_denetle; then
  echo "HATA: yönetim sayfası 200 vermedi." >&2
  systemctl status wc-merkez --no-pager -l 2>&1 | tail -n 15 >&2 || true
  if [ -n "$GD" ]; then
    echo "Önceki kuruluma geri dönülüyor..." >&2
    if geri_don_uygula "$GD"; then echo "GERİ DÖNÜLDÜ: önceki kurulum çalışıyor; yeni sürüm kurulmadı (veritabanı yedeği: ${yedek:-yok})." >&2
    else echo "HATA: geri dönüş de sağlık denetimini geçemedi; elle bakın (kur.sh --geri-don)." >&2; fi
  else
    echo "Geri dönülecek önceki kurulum yok (boş sunucu); günlük: journalctl -u wc-merkez" >&2
  fi
  exit 1
fi
# sağlık geçti: ikincil işler (hata kurulumu geri çevirmez)
systemctl start --no-block wc-durum.service || true
# ilk kurulumda listeler hemen üretilsin (cihazlar beklemesin); arka planda, kurulumu bekletmez
[ -s $VERI/merkez/listeler/kumar.json ] || systemctl start --no-block wc-listeler.service || true
systemctl reload caddy 2>/dev/null || systemctl restart caddy
systemctl reload fail2ban 2>/dev/null || systemctl restart fail2ban
[ -s $VERI/merkez/yonetici.json ] || echo "Yönetici hesabı yok: sudo wificorrect-sunucu yonetici-parola <kullanıcı-adı>"
echo "kurulum tamam: wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path), geri dönüş dizini: ${GD:-yok}"
