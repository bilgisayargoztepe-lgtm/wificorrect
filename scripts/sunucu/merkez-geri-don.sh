#!/usr/bin/env bash
# Yönetim merkezini önceki kuruluma döndürür (kur.sh'nin aldığı geri dönüş kopyasından). Kullanıcının bilgisayarında, Git Bash'te
# çalışır; sudo parolasını kullanıcı terminalde girer. Python dosyalarına kurulum hiç dokunmadığı için Python merkezine dönüş de
# bununla yapılır: birim dosyaları geri konur, wc-merkez yeniden başlar, sağlık (HTTP 200) denetlenir.
#
#   scripts/sunucu/merkez-geri-don.sh [hedef] [ZAMAN] [--kuru]
#
#   hedef   ssh hedefi (varsayılan gbserver@192.168.1.109); anahtar ~/.ssh/gbserver (WFC_SSH_KEY ile değişir)
#   ZAMAN   geri dönüş dizininin adı (YYYYmmdd-HHMMSS); verilmezse en yenisi. Rust iki kez kurulduysa en yeni dizin önceki
#           Rust'ın birimleridir; Python'a dönmek için ilk (en eski) dizinin adı verilmelidir (komut dizinleri listeler).
#   --kuru  sunucuya bağlanmaz, yalnız çalışacak komutları yazar
set -euo pipefail
host=gbserver@192.168.1.109
zaman=
kuru=0
konum=0
while [ $# -gt 0 ]; do
  case $1 in
    --kuru) kuru=1 ;;
    -*) echo "HATA: bilinmeyen seçenek: $1" >&2; exit 2 ;;
    *) konum=$((konum + 1))
       case $konum in 1) host=$1 ;; 2) zaman=$1 ;; *) echo "HATA: fazla argüman: $1" >&2; exit 2 ;; esac ;;
  esac
  shift
done
[[ $host =~ ^[A-Za-z0-9._@-]+$ ]] || { echo "HATA: geçersiz ssh hedefi: $host" >&2; exit 2; }
[ -z "$zaman" ] || [[ $zaman =~ ^[0-9]{8}-[0-9]{6}$ ]] || { echo "HATA: ZAMAN YYYYmmdd-HHMMSS biçiminde olmalı: $zaman" >&2; exit 2; }
anahtar=${WFC_SSH_KEY:-$HOME/.ssh/gbserver}
cd "$(dirname "$0")/../.."
kur=scripts/sunucu/merkez/kur.sh
[ -f "$kur" ] || { echo "HATA: $kur yok" >&2; exit 1; }
uzak_kur="set -o pipefail; sudo bash /tmp/wfc-geri/kur.sh --geri-don${zaman:+ $zaman} 2>&1 | tee /tmp/wfc-geri/sonuc.txt; k=\$?; sudo chmod 644 /tmp/wfc-geri/sonuc.txt; exit \$k"
if [ "$kuru" = 1 ]; then
  echo "--kuru: sunucuya bağlanılmıyor. Çalışacak komutlar ($host):"
  echo "  ssh -i $anahtar $host 'rm -rf /tmp/wfc-geri && mkdir /tmp/wfc-geri && cat > /tmp/wfc-geri/kur.sh'   ($kur gönderilir)"
  echo "  ssh -t -i $anahtar $host '$uzak_kur'"
  exit 0
fi
tr -d '\r' <"$kur" | ssh -i "$anahtar" "$host" 'rm -rf /tmp/wfc-geri && mkdir /tmp/wfc-geri && cat > /tmp/wfc-geri/kur.sh'
echo "geri dönüş çalıştırılıyor (sudo parolası istenecek)..."
ssh -t -i "$anahtar" "$host" "$uzak_kur"
