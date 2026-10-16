#!/bin/sh
# deploy/debian + ikili → .deb (dpkg-deb, ek araç yok). Kullanım: scripts/paket.sh target/release/wificorrect 0.3.0 dist
set -eu
ikili=$(realpath "$1") surum=$2 cikti=$(realpath -m "$3")
kok=$(mktemp -d)
cd "$(dirname "$0")/.."
cp -a deploy/debian/. "$kok/"
# panelin / programın ürettiği dosyalar pakete girmez (güncellemede ezilmesin)
rm -f "$kok/etc/network/interfaces.d/wificorrect" "$kok/etc/wificorrect/arayuzler.nft" \
      "$kok/etc/issue.d/wificorrect.issue" "$kok/etc/wificorrect/yasak-siteler.conf"
install -D -m 755 "$ikili" "$kok/usr/local/bin/wificorrect"
chmod 755 "$kok/usr/local/lib/wificorrect/dhcp-script"  # dnsmasq doğrudan çalıştırır; git/Windows modu ne olursa olsun
chmod 755 "$kok/usr/local/lib/wificorrect/efi-baslangic"  # postinst ve kurucu sh ile çağırır; elle de çalışsın
mkdir -p "$kok/DEBIAN"
sed "s/@SURUM@/$surum/" paket/control.in > "$kok/DEBIAN/control"
install -m 755 paket/postinst "$kok/DEBIAN/postinst"
# conffiles yok: /etc altındaki ürün dosyaları her güncellemede repodaki haline döner (kural: kalıp yalnızca repodan)
mkdir -p "$cikti"
dpkg-deb --root-owner-group --build "$kok" "$cikti/wificorrect_${surum}_amd64.deb"
rm -rf "$kok"
