#!/bin/sh
# .deb'i temiz Debian 13 kapsayıcısında kurar ve denetler (CI). Kullanım: scripts/paket-test.sh dist/wificorrect_X_amd64.deb
set -eu
deb=$1
docker run --rm -v "$PWD/$(dirname "$deb"):/p:ro" debian:trixie sh -euc "
  apt-get update -q >/dev/null
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q /p/$(basename "$deb") >/dev/null
  test -x /usr/local/bin/wificorrect
  /usr/local/bin/wificorrect surum
  test -f /etc/wificorrect/ayarlar.toml && test \"\$(stat -c %a /etc/wificorrect/ayarlar.toml)\" = 600
  grep -q GRUB_TERMINAL=console /etc/default/grub.d/wificorrect.cfg
  test -d /srv/5651 && test \"\$(stat -c %a /srv/5651)\" = 700
  test -L /etc/systemd/system/multi-user.target.wants/wificorrect-panel.service
  test -L /etc/systemd/system/timers.target.wants/wificorrect-guncelleme-denetle.timer
  dpkg -L wificorrect | grep -q /etc/systemd/system/wificorrect-guncelleme-denetle.service
  dpkg -L wificorrect | grep -q /etc/systemd/system/wificorrect-pppoe.service
  test ! -L /etc/systemd/system/multi-user.target.wants/wificorrect-pppoe.service
  test -x /usr/sbin/pppd
  dpkg -L wificorrect | grep -qx /etc/ppp/ip-up.d/wificorrect && test \"\$(stat -c %a /etc/ppp/ip-up.d/wificorrect)\" = 755
  run-parts --test /etc/ppp/ip-up.d | grep -qx /etc/ppp/ip-up.d/wificorrect
  PPP_IFACE=wfcppp /etc/ppp/ip-up.d/wificorrect wfcppp && PPP_IFACE=ppp0 /etc/ppp/ip-up.d/wificorrect ppp0
  dpkg -L wificorrect | grep -qx /etc/dhcpcd.exit-hook && test \"\$(stat -c %a /etc/dhcpcd.exit-hook)\" = 644
  test -f /usr/lib/dhcpcd/dhcpcd-run-hooks && grep -q /etc/dhcpcd.exit-hook /usr/lib/dhcpcd/dhcpcd-run-hooks
  sh -euc '. /etc/dhcpcd.exit-hook; echo kanca-tamam' | grep -qx kanca-tamam
  ! dpkg -L wificorrect | grep -E 'interfaces.d/wificorrect|arayuzler.nft|issue.d/wificorrect.issue|yasak-siteler.conf|hazir-liste.conf'
  test -f /etc/wificorrect/hazir-liste.conf && grep -q servers-file=/etc/wificorrect/hazir-liste.conf /etc/dnsmasq.d/wificorrect.conf
  grep -q \"ExecStartPre=.*hazir-liste.conf\" /etc/systemd/system/dnsmasq.service.d/wificorrect.conf
  test -f /usr/local/lib/wificorrect/dnsmasq-ozet
  dpkg -L wificorrect | grep -q /etc/wificorrect/hiz.nft
  test -x /usr/local/lib/wificorrect/dhcp-script
  test -x /usr/local/lib/wificorrect/efi-baslangic && test ! -e /boot/efi/startup.nsh
  mkdir -p /tmp/esp && sh /usr/local/lib/wificorrect/efi-baslangic /tmp/esp
  test -s /tmp/esp/startup.nsh && cmp -s /tmp/esp/startup.nsh /tmp/esp/EFI/BOOT/startup.nsh
  grep -q 'fs0:.EFI.wificorrect.kimlik' /tmp/esp/startup.nsh && grep -q WifiCorrect /tmp/esp/EFI/wificorrect/kimlik
  mkdir -p /tmp/esp2 && printf 'echo baska\r\n' > /tmp/esp2/startup.nsh && sh /usr/local/lib/wificorrect/efi-baslangic /tmp/esp2
  grep -q baska /tmp/esp2/startup.nsh && grep -q 'EFI.wificorrect.kimlik' /tmp/esp2/EFI/BOOT/startup.nsh
  command -v iptables-restore
  echo 'site_name = \"Kalsin\"' >> /etc/wificorrect/ayarlar.toml
  echo 'elle' > /etc/wificorrect/yasak-siteler.conf
  DEBIAN_FRONTEND=noninteractive dpkg -i /p/$(basename "$deb") >/dev/null
  grep -q Kalsin /etc/wificorrect/ayarlar.toml
  grep -q elle /etc/wificorrect/yasak-siteler.conf
  echo PAKET TESTI GECTI
"
