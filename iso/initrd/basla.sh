#!/bin/sh
# preseed/early_command (initrd'den; cdrom-detect'ten ÖNCE çalışır): list-devices'ı sarmalayıcıyla değiştirir,
# böylece cdrom-detect /cdrom'a yalnızca bu ISO'yu bağlar (bkz. list-devices). Disk.sh ayrıca doğrular.
. /wificorrect/ortak.sh
g=$(command -v list-devices) || { seri "list-devices bulunamadi"; exit 0; }
[ -e /wificorrect/list-devices.gercek ] && exit 0
cp "$g" /wificorrect/list-devices.gercek && chmod 755 /wificorrect/list-devices.gercek || exit 0
cp /wificorrect/list-devices "$g.wfc" && chmod 755 "$g.wfc" && mv -f "$g.wfc" "$g"
seri "KURULUM BASLADI: $(cat "$KIMLIK" 2>/dev/null)"
