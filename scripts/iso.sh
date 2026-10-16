#!/bin/sh
# Debian 13 netinst (imzası doğrulanır) + preseed + .deb → kurulum ISO'su. Kullanım: scripts/iso.sh <deb> <sürüm> <çıktı>
# Ortam: WFC_SSH_PUB (opsiyonel) → kurulan sistemde /root/.ssh/authorized_keys.
# preseed.cfg ve iso/initrd/ betikleri kurucunun initrd'sine eklenir (/preseed.cfg, /lib/wificorrect): yüklenmeleri
# hangi diskin /cdrom'a bağlandığına bağlı olmaz. ISO'ya ve initrd'ye aynı, her derlemede yeni kimlik konur
# (/.wificorrect, /wificorrect-kimlik); kurucu /cdrom'a yalnızca bu kimliği tutan ortamı bağlar.
set -eu
deb=$(realpath "$1") surum=$2 cikti=$(realpath -m "$3")
cd "$(dirname "$0")/.."
u=https://cdimage.debian.org/debian-cd/current/amd64/iso-cd
d=$(mktemp -d)
curl -fsSL "$u/SHA256SUMS" -o "$d/SHA256SUMS"
curl -fsSL "$u/SHA256SUMS.sign" -o "$d/SHA256SUMS.sign"
gpgv --keyring /usr/share/keyrings/debian-role-keys.gpg "$d/SHA256SUMS.sign" "$d/SHA256SUMS"
ad=$(grep -o 'debian-13[^ ]*-amd64-netinst\.iso' "$d/SHA256SUMS" | head -1)
curl -fsSL "$u/$ad" -o "$d/$ad"
(cd "$d" && grep " $ad\$" SHA256SUMS | sha256sum -c -)
mkdir -p "$d/ek/wificorrect" "$cikti"
cp "$deb" iso/son.sh "$d/ek/wificorrect/"
if [ -n "${WFC_SSH_PUB:-}" ]; then printf '%s\n' "$WFC_SSH_PUB" > "$d/ek/wificorrect/authorized_keys"; fi
printf 'WifiCorrect %s %s\n' "$surum" "$(cat /proc/sys/kernel/random/uuid)" > "$d/kimlik"
# initrd: Debian'ınkinin arkasına ikinci bir (gzip'li newc cpio) arşiv eklenir; çekirdek ikisini sırayla açar.
k=$d/initrd-ek
mkdir -p "$k/lib/wificorrect"
cp iso/preseed.cfg "$k/preseed.cfg"
cp "$d/kimlik" "$k/wificorrect-kimlik"
cp iso/initrd/* "$k/lib/wificorrect/"
chmod 644 "$k/preseed.cfg" "$k/wificorrect-kimlik" && chmod 755 "$k/lib/wificorrect"/*
xorriso -osirrox on -indev "$d/$ad" -extract /install.amd/initrd.gz "$d/initrd.gz"
chmod u+w "$d/initrd.gz"
(cd "$k" && find . | LC_ALL=C sort | cpio -o -H newc -R 0:0 --quiet | gzip -9) >> "$d/initrd.gz"
rm -f "$cikti/wificorrect-kurulum-$surum.iso"
xorriso -indev "$d/$ad" -outdev "$cikti/wificorrect-kurulum-$surum.iso" \
  -map "$d/kimlik" /.wificorrect -map "$d/ek/wificorrect" /wificorrect -map "$d/initrd.gz" /install.amd/initrd.gz \
  -map iso/grub.cfg /boot/grub/grub.cfg -map iso/isolinux.cfg /isolinux/isolinux.cfg \
  -boot_image any replay
rm -rf "$d"
