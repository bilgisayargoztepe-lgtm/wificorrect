#!/bin/sh
# partman/early_command (initrd'de /lib/wificorrect/disk.sh). Hiçbir diske dokunmadan önce:
#  1) /cdrom gerçekten bu ISO mu (kimlik dosyası)? Değilse dur: başka bir kurulum belleğinden paket kurulmasın.
#  2) İnternet var mı (varsayılan rota)? Yoksa dur: paketler internetten iner, ağsız kurulum yarım kalırdı.
#  3) Kurulacak disk: kurulum ortamının kendisi (/cdrom'daki aygıt) hiçbir zaman; bağlı (mount) diskler ve 7 GB'tan
#     küçükler de değil. Önce iç disk (SSD, USB'ye bağlı olmayan), yoksa ilk USB disk/bellek — başka bir kurulum
#     belleği (ISO 9660 içeren) değilse. Hiç aday yoksa dur.
# Duruşlar KALICIDIR (ortak.sh dur), hiçbir disk silinmez. Seçim seri porta da yazılır (duman testi) ve
# /tmp/wificorrect-hedef'e (ic|usb; son.sh) yazılır.
. /lib/wificorrect/ortak.sh

if ! bizim_ortam /cdrom; then
  seri "KURULUM ORTAMI YANLIS: /cdrom = $(awk '$2=="/cdrom"{print $1}' /proc/mounts)"
  dur ortam-yanlis "Kurulum belleği okunamadı" \
    "Kurucu, WifiCorrect kurulum belleği yerine başka bir diski açtı. Hiçbir disk silinmedi." \
    "." \
    "Cihazı kapatın. WifiCorrect kurulum belleği ve kurulacak boş bellek dışındaki bütün USB bellek ve diskleri çıkarın, kurulumu yeniden başlatın."
fi

if ! awk '$2=="00000000"{v=1} END{exit !v}' /proc/net/route 2>/dev/null; then
  seri "INTERNET YOK"
  dur internet-yok "İnternet bağlantısı yok" \
    "Kurulum paketleri internetten indirir; cihaz IP alamadı. Hiçbir disk silinmedi." \
    "." \
    "Cihazı kapatın. Ethernet 1'in (sağdaki port) modeme takılı olduğunu ve modemin çalıştığını denetleyin, kurulumu yeniden başlatın."
fi

ortam=$(awk '$2=="/cdrom"{print $1}' /proc/mounts | head -1)
ortam=${ortam##*/}
seri "KURULUM ORTAMI: /dev/$ortam"
# en az 7 GB (512 baytlık kesim): 8 GB'lık bellekler gerçekte ~7,5 GB'tır; tarif ~5,6 GB ister
EN_AZ=13671875
m=/tmp/wificorrect-disk-dene
mkdir -p "$m"
# iso_mu <ad>: diskte ya da bölümlerinden birinde ISO 9660 var mı (başka bir kurulum belleği)? Salt okunur denenir.
iso_mu() {
  for b in "/dev/$1" $(cd "/sys/block/$1" 2>/dev/null && for p in "$1"*; do [ -e "$p/partition" ] && echo "/dev/$p"; done); do
    if mount -t iso9660 -o ro "$b" "$m" 2>/dev/null; then umount "$m" 2>/dev/null || true; return 0; fi
  done
  return 1
}
modprobe isofs 2>/dev/null || true
ic= usb=
for d in $(list-devices disk); do
  n=${d#/dev/}
  # kurulum ortamı: diskin kendisi ya da bir bölümü (/sys/block/sdb/sdb1)
  if [ -n "$ortam" ] && { [ "$n" = "$ortam" ] || [ -e "/sys/block/$n/$ortam" ]; }; then continue; fi
  if grep -Eq "^/dev/$n(p?[0-9]+)? " /proc/mounts; then seri "BAGLI DISK ATLANDI: $d"; continue; fi
  boyut=$(cat "/sys/block/$n/size" 2>/dev/null || echo 0)
  if [ "${boyut:-0}" -lt $EN_AZ ]; then seri "KUCUK DISK ATLANDI: $d"; continue; fi
  case $(readlink -f "/sys/block/$n") in
    */usb*)
      [ -z "$usb" ] || continue
      if iso_mu "$n"; then seri "BASKA KURULUM BELLEGI ATLANDI: $d"; continue; fi
      usb=$d ;;
    *) ic=$d; break ;;
  esac
done
if [ -n "$ic" ]; then hedef=$ic tur=ic; else hedef=$usb tur=usb; fi
if [ -n "$hedef" ]; then
  debconf-set partman-auto/disk "$hedef"
  debconf-set grub-installer/bootdev "$hedef"
  echo "$tur" >/tmp/wificorrect-hedef
  seri "KURULUM DISKI: $hedef ($tur)"
  exit 0
fi
seri "KURULACAK DISK YOK"
dur disk-yok "Kurulacak disk bulunamadı" \
  "Kurulum belleği dışında en az 8 GB'lık bir disk (SSD) ya da boş USB bellek yok. Hiçbir disk silinmedi." \
  "." \
  "Cihazı kapatın. Kendi diskini (SSD) kontrol edin ya da kurulum için ikinci, boş bir USB bellek takın (en az 8 GB; SSD önerilir), yeniden deneyin."
