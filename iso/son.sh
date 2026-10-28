#!/bin/sh
# Kurucunun son adımı: WifiCorrect paketi (bağımlılıklar internetten) + hizmet sağlayıcının açık SSH anahtarı.
set -e
# Tanı: kurucu günlüğü (apt/dpkg çıktısı dahil) seri porta; QEMU duman testi kurulum.log'da görür. Arka planda, tüm
# tanımlayıcılar kapalı: seri port yoksa açılamaz, taşıyıcı beklerse yalnızca bu süreç bekler — kurulum etkilenmez.
( tail -n 100 -f /var/log/syslog </dev/null >/dev/ttyS0 2>/dev/null 3>&- 4>&- 5>&- 6>&- 7>&- 8>&- 9>&- & )
cp /cdrom/wificorrect/wificorrect_*_amd64.deb /target/tmp/
if [ -s /cdrom/wificorrect/authorized_keys ]; then
  mkdir -p /target/root/.ssh && chmod 700 /target/root/.ssh
  cp /cdrom/wificorrect/authorized_keys /target/root/.ssh/authorized_keys && chmod 600 /target/root/.ssh/authorized_keys
fi
# </dev/null: bir paket soru sorarsa (dpkg conffile vb.) sonsuza dek beklemek yerine hata verir
in-target sh -c 'DEBIAN_FRONTEND=noninteractive apt-get install -y -q /tmp/wificorrect_*_amd64.deb </dev/null && rm -f /tmp/wificorrect_*_amd64.deb'
# Ağ rolleri belirlenemediyse (tek Ethernet vb.) paket ag.toml yazmaz → cihaz ilk açılışta ağsız ve kök kilitli:
# erişilemez. "Kurulum bitti" demek yerine KALICI olarak dur (preseed komut hatasından sonra devam eder, exit 1 yetmez).
if [ ! -s /target/etc/wificorrect/ag.toml ]; then
  logger -t wificorrect "AG AYARI YAZILAMADI"
  cat >/tmp/wificorrect-ag.templates <<'EOF'
Template: wificorrect/ag-yok
Type: error
Description: Ağ ayarı yapılamadı
 WifiCorrect en az iki Ethernet kartı ister ve Ethernet 1 modeme takılı olmalıdır. Disk kuruldu ama cihaz bu haliyle
 ağsız açılır ve erişilemez.
 .
 Cihazı kapatın, iki Ethernet'in de takılı olduğunu ve Ethernet 1'in modemde olduğunu denetleyin, kurulumu yeniden başlatın.
EOF
  debconf-loadtemplate wificorrect /tmp/wificorrect-ag.templates || true
  . /usr/share/debconf/confmodule
  while :; do
    db_fset wificorrect/ag-yok seen false || true
    db_input critical wificorrect/ag-yok || true
    db_go || true
    sleep 1
  done
fi
# Saat (2026-10-26; Visavis'te ilk açılış 2026-04-13 günü yazdı, KURULUM_GUNLUGU "Saat doğrulanmadan kayıt"): kurucu
# saati NTP ile düzeltir (clock-setup) ama RTC'ye yazamıyordu (hedefte hwclock yoktu; artık util-linux-extra
# pkgsel/include'da). Burada da yazılır ve geri okunur (duman testi "RTC YAZILDI" arar). timesyncd'nin zaman dosyası
# kurulum anıyla oluşturulur: systemd açılışta saati en az bunun zamanına getirir (yoksa kendi paket tarihine; RTC yine
# bozuksa ilk açılış Nisan 2026'da başlıyordu). "saat-dogru" işareti ilk açılışın RTC'ye güvenmesini sağlar (src/saat.rs).
# Kurucunun saati kurulum belleğindeki paketten eskiyse NTP tutmamıştır: hiçbiri yazılmaz, ilk açılışta 5651 satırları
# saat NTP ile doğrulanana kadar bekletilir.
touch /tmp/wfc-simdi
if [ /tmp/wfc-simdi -nt "$(ls /cdrom/wificorrect/wificorrect_*_amd64.deb | head -n 1)" ]; then
  dev_bagli=
  if [ ! -e /target/dev/rtc0 ] && mount --bind /dev /target/dev; then dev_bagli=1; fi
  chroot /target hwclock --systohc --utc >/tmp/wfc-hwclock.log 2>&1 || true
  [ -z "$dev_bagli" ] || umount /target/dev || true
  rtc=$(cat /sys/class/rtc/rtc0/since_epoch 2>/dev/null || true)
  case "$rtc" in '' | *[!0-9]*) rtc=0 ;; esac
  simdi=$(date +%s)
  fark=$((simdi - rtc))
  if [ "$fark" -ge -5 ] && [ "$fark" -le 5 ]; then
    logger -t wificorrect "RTC YAZILDI ($rtc)"
    printf 'dogru kurulum %s\n' "$simdi" >/target/var/lib/wificorrect/saat-dogru || true
  else
    logger -t wificorrect "RTC YAZILAMADI (rtc=$rtc saat=$simdi): $(tr '\n' ' ' </tmp/wfc-hwclock.log)"
  fi
  { mkdir -p /target/var/lib/systemd/timesync && touch /target/var/lib/systemd/timesync/clock; } || true
else
  logger -t wificorrect "SAAT DOGRULANMADI: kurucunun saati $(date -u +%Y-%m-%dT%H:%M:%SZ), RTC yazilmadi"
fi
# Realtek ağ kartlarının aygıt yazılımı kurulan sistemde olmalı (pkgsel/include); duman testi bu satırı arar.
if chroot /target dpkg-query -W -f='${Status}' firmware-realtek 2>/dev/null | grep -q 'install ok installed'; then
  logger -t wificorrect "FIRMWARE-REALTEK KURULU"
else
  logger -t wificorrect "FIRMWARE-REALTEK YOK"
fi
# Kök dosya sistemi noatime: her okuma diske yazmasın (USB bellekte aşınmayı azaltır; SSD'de de zararsız).
sed -i -E 's|^([^#[:space:]]+[[:space:]]+/[[:space:]]+ext4[[:space:]]+)([^[:space:]]+)|\1noatime,\2|' /target/etc/fstab || true
# Monitörsüz açılış: NVRAM'inde önyükleme kaydı olmayan kutu UEFI Shell'e düşerse startup.nsh sistemi başlatır. Paketin
# postinst'i de yazar; kurucunun içinde /boot/efi'nin bağlı görünmesine güvenmemek için burada da (yalnızca UEFI).
if grep -q ' /target/boot/efi ' /proc/mounts; then
  if sh /target/usr/local/lib/wificorrect/efi-baslangic /target/boot/efi; then
    logger -t wificorrect "EFI BASLANGIC YAZILDI"
  else
    logger -t wificorrect "EFI BASLANGIC YAZILAMADI"
  fi
fi
# Bitti mesajı (bekletmez: 10 sn görünür, sonra kurucu kendi son adımlarını yapıp cihazı kapatır). USB belleğe
# kurulduysa (disk.sh: /tmp/wificorrect-hedef) hangi belleğin çıkarılacağı ayrıca söylenir.
if [ "$(cat /tmp/wificorrect-hedef 2>/dev/null)" = usb ]; then
  bitti="Kurulum bitti. Cihaz birazdan kapanacak; kapandıktan sonra yalnızca kurulum belleğini çıkarın. WifiCorrect'in kurulduğu USB bellek takılı kalmalı."
else
  bitti="Kurulum bitti. Cihaz birazdan kapanacak; kapandıktan sonra USB belleği çıkarın."
fi
printf 'Template: wificorrect/bitti\nType: text\nDescription: %s\n' "$bitti" >/tmp/wificorrect-bitti.templates
logger -t wificorrect "KURULUM BITTI ($(cat /tmp/wificorrect-hedef 2>/dev/null)) fstab: $(grep -E '[[:space:]]/[[:space:]]' /target/etc/fstab | grep -v '^#')"
if debconf-loadtemplate wificorrect /tmp/wificorrect-bitti.templates; then
  . /usr/share/debconf/confmodule
  db_progress INFO wificorrect/bitti || true
  sleep 10
fi
