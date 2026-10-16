#!/bin/sh
# ISO'yu QEMU'da (UEFI, KVM) dener. Kurulum ortamı gerçek donanımdaki gibi USB bellek olarak takılır (salt okunur,
# Rufus DD gibi ham ISO). Ayrıca hep takılı küçük bir yabancı USB bellek (64 MB, bilinen içerik) değişmemeli:
#  0) Yalnız kurulum belleği (+ küçük bellek): kurucu "KURULACAK DISK YOK" deyip durmalı (partman başlamamalı).
#  1) + boş 8 GB virtio disk (iç disk): menü kendiliğinden başlamamalı; /cdrom = kurulum belleği (kimlik); kurulum
#     /dev/vda'ya (iç disk USB'ye tercih edilir); "Kurulum bitti" mesajı; kapanır.
#  2) Diskten açılış: panel servisi + IP'li yönetim adresi (seri konsol); WFC_TEST_SSH_KEY (özel anahtar dosyası)
#     verildiyse o anahtarla root SSH.
#  3) İç disk yok, ikinci boş USB bellek (8 GB) var: kurulum o belleğe; sonra kurulum belleği çıkarılmış ve UEFI
#     kayıtları sıfır (yalnızca çıkarılabilir ortam yolu) makinede o bellekten açılış + panel.
# Her aşamadan sonra küçük USB belleğin sha256'sı değişmemiş olmalı.
# KVM yoksa atlar (rapora yazılır). Seri günlükler + ekran görüntüleri ./iso-test-gunluk/ altına (her durumda).
# Kullanım: scripts/iso-test.sh dist/wificorrect-kurulum-X.iso
set -eu
iso=$(realpath "$1") d=$(mktemp -d) g=$PWD/iso-test-gunluk
[ -w /dev/kvm ] || { echo "ISO DUMAN TESTI ATLANDI: KVM yok"; exit 0; }
mkdir -p "$g"
qemu-img create -f qcow2 "$d/disk.qcow2" 8G >/dev/null
qemu-img create -f qcow2 "$d/hedef-usb.qcow2" 8G >/dev/null
yes WIFICORRECT-USB-TEST | head -c 67108864 >"$d/usb.img"
usb_sha=$(sha256sum <"$d/usb.img")
iso_sha=$(sha256sum <"$iso")
# Kurulum belleği: ham ISO, USB, salt okunur, ilk açılış aygıtı
KUR="-drive if=none,id=kur,format=raw,readonly=on,file=$iso -device usb-storage,bus=xhci.0,drive=kur,bootindex=0"
# vm <seri-günlük> [ek argümanlar]: arka planda başlatır, $q = süreç
vm() {
  log=$1; shift
  rm -f "$d/mon"
  qemu-system-x86_64 -machine q35 -enable-kvm -m 2048 -smp 2 -display none \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd \
    -drive if=pflash,format=raw,file="$d/vars.fd" \
    -device qemu-xhci,id=xhci -drive if=none,id=usb,format=raw,file="$d/usb.img" -device usb-storage,bus=xhci.0,drive=usb \
    -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2222-:22 -device virtio-net-pci,netdev=n0 \
    -netdev user,id=n1,restrict=on,hostfwd=tcp:127.0.0.1:2223-:22 -device virtio-net-pci,netdev=n1 \
    -monitor unix:"$d/mon",server,nowait -serial file:"$d/$log" "$@" &
  q=$!
}
# HMP: komutu gönder, yanıtı (sonraki "(qemu)" istemine kadar) mon.log'a ekle
mon() { python3 -c 'import socket,sys,time
s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.settimeout(15); s.sendall(sys.argv[2].encode()+b"\n")
b=b""; t=time.time()
while b.count(b"(qemu)") < 2 and time.time()-t < 15:
    try: b+=s.recv(4096) or b"(qemu)(qemu)"
    except socket.timeout: break
sys.stdout.write(b.decode(errors="replace"))' "$d/mon" "$1" >>"$d/mon.log" 2>&1 || true; }
ekran() { mon "screendump $d/ekran.ppm"; }
# P6 PPM → PNG (yalnızca stdlib; çalıştırıcıda ImageMagick olmayabilir)
png() { python3 -c 'import sys,zlib,struct
b=open(sys.argv[1],"rb").read(); m,wh,mx,r=b.split(b"\n",3); w,h=map(int,wh.split())
c=lambda t,x: struct.pack(">I",len(x))+t+x+struct.pack(">I",zlib.crc32(t+x))
raw=b"".join(b"\0"+r[y*w*3:(y+1)*w*3] for y in range(h))
open(sys.argv[2],"wb").write(b"\x89PNG\r\n\x1a\n"+c(b"IHDR",struct.pack(">IIBBBBB",w,h,8,2,0,0,0))+c(b"IDAT",zlib.compress(raw))+c(b"IEND",b""))' "$1" "$2" || cp "$1" "$g/"; }
gunluk() { cp "$d"/*.log "$g/" 2>/dev/null || true; }
son=
hata() {
  ekran; kill "$q" 2>/dev/null || true
  [ -n "$son" ] && [ -e "$d/$son" ] && { echo "--- $son (son 60 satır)"; tail -60 "$d/$son" | tr -d '\033'; }
  gunluk; [ -e "$d/ekran.ppm" ] && png "$d/ekran.ppm" "$g/ekran.png"
  echo "ISO DUMAN TESTI BASARISIZ: $1"; exit 1
}
usb_ayni() {
  [ "$(sha256sum <"$d/usb.img")" = "$usb_sha" ] || hata "küçük USB bellek değişti ($1)"
  [ "$(sha256sum <"$iso")" = "$iso_sha" ] || hata "kurulum belleği (ISO) değişti ($1)"
}
# Menü (GRUB, OVMF seri konsola da yazar) kendiliğinden başlamamalı: 20 sn seri çıktı değişmemeli, sonra Enter.
menu() {
  n=0
  until grep -aq "WifiCorrect kur" "$d/$1" 2>/dev/null; do
    sleep 2; n=$((n + 2)); [ $n -lt 180 ] || hata "kurulum menüsü seri konsolda görünmedi"
  done
  sleep 3; once=$(wc -c <"$d/$1"); sleep 20
  [ "$(wc -c <"$d/$1")" = "$once" ] || hata "menü kendiliğinden başladı (seri çıktı değişti)"
  mon "sendkey ret"
}
# kur <seri-günlük> [ek argümanlar]: kurulumu baştan sona yürütür (kurucu bitince makineyi kapatır).
# Ekran 10 dk hiç değişmediyse kurucu bir soruda/hatada takılmıştır → 40 dk beklemeden dur.
kur() {
  log=$1; shift; son=$log
  cp /usr/share/OVMF/OVMF_VARS_4M.fd "$d/vars.fd"
  vm "$log" -no-reboot $KUR "$@"
  menu "$log"
  n=0 ayni=0 once= bitti=
  while kill -0 "$q" 2>/dev/null; do
    sleep 2; n=$((n + 2))
    if [ -z "$bitti" ] && grep -aq "KURULUM BITTI" "$d/$log"; then
      bitti=1; sleep 3; ekran; png "$d/ekran.ppm" "$g/ekran-bitti-${log%.log}.png"
    fi
    [ $((n % 30)) = 0 ] || continue
    ekran
    simdi=$(cksum <"$d/ekran.ppm" 2>/dev/null || true)
    if [ -n "$simdi" ] && [ "$simdi" = "$once" ]; then ayni=$((ayni + 1)); else ayni=0; fi
    once=$simdi
    [ $ayni -lt 20 ] || hata "kurucu ekranı 10 dk değişmedi (takıldı; son ekran: iso-test-gunluk/ekran.png)"
    [ $n -lt 2400 ] || hata "kurulum 40 dk içinde bitmedi (son ekran: iso-test-gunluk/ekran.png)"
  done
  grep -a "WIFICORRECT: KURULUM" "$d/$log" | tr -d '\r'
  grep -aq "WIFICORRECT: KURULUM ORTAMI ADAYI: /dev/sd" "$d/$log" || hata "kurulum belleği kimlikle seçilmedi"
  [ -n "$bitti" ] || hata "'Kurulum bitti' mesajı görünmedi"
  grep -aq "FIRMWARE-REALTEK KURULU" "$d/$log" || hata "firmware-realtek kurulan sisteme kurulmadı"
  grep -aq "KURULUM BITTI .* noatime," "$d/$log" || hata "kök dosya sistemi noatime değil"
  usb_ayni "kurulumda ($log)"
}
# acilis <seri-günlük> [ek argümanlar]: systemd durum satırı (açıklama konsol genişliğine kırpılır → birim adıyla
# aranır) + getty'nin yazdığı yönetim adresi (seri konsol ttyS0). Makine açık kalır ($q).
acilis() {
  log=$1; shift; son=$log
  vm "$log" "$@"
  n=0
  until grep -aq "Started.*wificorrect-panel.service" "$d/$log" && grep -aq "WifiCorrect panel: https://panel.wificorrect.com (eslestirme: https://[0-9]" "$d/$log"; do
    sleep 5; n=$((n + 5)); [ $n -lt 300 ] || { grep -a "FAILED" "$d/$log" | tr -d '\033' || true; hata "açılışta panel/yönetim adresi görünmedi ($log)"; }
  done
  grep -a "WifiCorrect panel: https://panel.wificorrect.com" "$d/$log" | tail -1 | tr -d '\033'
}

# 0) Kurulum belleğinden başka (yeterli) disk yok → kurucu disk.sh hatasında durur. Kurucu monitörde (tty)
#    çalışır; disk.sh seriye yazar.
son=kurulum0.log
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$d/vars.fd"
vm kurulum0.log $KUR
menu kurulum0.log
n=0
until grep -aq "WIFICORRECT: KURULACAK DISK YOK" "$d/kurulum0.log"; do
  sleep 5; n=$((n + 5)); usb_ayni "yalnız kurulum belleği varken"
  grep -aq "WIFICORRECT: KURULUM DISKI" "$d/kurulum0.log" && hata "yalnız kurulum belleği varken disk seçildi"
  kill -0 "$q" 2>/dev/null || hata "yalnız kurulum belleği varken kurucu kapandı"
  [ $n -lt 900 ] || hata "yalnız kurulum belleği varken 'disk yok' hatası 15 dk içinde gelmedi"
done
sleep 60   # durduğunu gör: partman başlamış olsaydı bu sürede diske yazardı
ekran; png "$d/ekran.ppm" "$g/ekran-disk-yok.png"
kill -0 "$q" 2>/dev/null || hata "'disk yok'tan sonra kurucu kapandı"
kill "$q"; wait "$q" 2>/dev/null || true
usb_ayni "yalnız kurulum belleği varken"
grep -a "WIFICORRECT:" "$d/kurulum0.log" | tr -d '\r'
grep -aq "WIFICORRECT: KURULUM ORTAMI ADAYI: /dev/sd" "$d/kurulum0.log" || hata "kurulum belleği kimlikle seçilmedi"
echo "disk yok: kurucu durdu, USB'ler değişmedi"

# 1) iç disk (virtio) + kurulum belleği → iç diske
kur kurulum.log -drive file="$d/disk.qcow2",if=virtio
grep -aq "WIFICORRECT: KURULUM DISKI: /dev/vda (ic)" "$d/kurulum.log" || hata "kurulum /dev/vda'ya gitmedi"

# 2) iç diskten açılış (kurulum belleği çıkarılmış)
acilis acilis.log -drive file="$d/disk.qcow2",if=virtio
if [ -n "${WFC_TEST_SSH_KEY:-}" ]; then
  n=0
  until for p in 2222 2223; do
      ssh -q -i "$WFC_TEST_SSH_KEY" -p $p -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=no \
        -o UserKnownHostsFile=/dev/null root@127.0.0.1 'test -s /root/.ssh/authorized_keys && echo "SSH ANAHTARI CALISIYOR (port '$p')"' && break
    done | grep -a "SSH ANAHTARI CALISIYOR"; do
    sleep 5; n=$((n + 5)); [ $n -lt 120 ] || hata "SSH anahtarıyla root girişi olmadı"
  done
fi
kill "$q" 2>/dev/null || true; wait "$q" 2>/dev/null || true
usb_ayni "açılışta"

# 3) iç disk yok, ikinci boş USB bellek → o belleğe; sonra yalnız o bellekten, boş UEFI kayıtlarıyla açılış
HEDEF="-drive if=none,id=hedef,format=qcow2,file=$d/hedef-usb.qcow2 -device usb-storage,bus=xhci.0,drive=hedef"
kur kurulum-usb.log $HEDEF
grep -aq "WIFICORRECT: KURULUM DISKI: /dev/sd[a-z]* (usb)" "$d/kurulum-usb.log" || hata "kurulum ikinci USB belleğe gitmedi"
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$d/vars.fd"
acilis acilis-usb.log $HEDEF
kill "$q" 2>/dev/null || true; wait "$q" 2>/dev/null || true
usb_ayni "USB bellekten açılışta"
gunluk; rm -rf "$d"
echo "ISO DUMAN TESTI GECTI"
