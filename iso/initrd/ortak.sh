# Kurucu betiklerinin ortak işlevleri (initrd'de /wificorrect/ortak.sh; ". " ile okunur).
# seri <metin>: sistem günlüğüne ve seri porta (QEMU duman testi). Arka planda, tanımlayıcılar kapalı: seri port
# taşıyıcı beklerse yalnızca bu süreç bekler.
seri() {
  logger -t wificorrect "$*"
  ( echo "WIFICORRECT: $*" </dev/null >/dev/ttyS0 2>/dev/null 3>&- 4>&- 5>&- 6>&- 7>&- 8>&- 9>&- & )
}
# Bu kurulum ortamının kimliği (ISO'da /.wificorrect, initrd'de /wificorrect-kimlik; her ISO'da farklı).
KIMLIK=/wificorrect-kimlik
# bizim_ortam <dizin>: dizine bağlı ortam bu ISO mu?
bizim_ortam() {
  [ -s "$KIMLIK" ] && [ -s "$1/.wificorrect" ] && [ "$(cat "$1/.wificorrect")" = "$(cat "$KIMLIK")" ]
}
# dur <ad> <başlık> <metin...>: Türkçe hata ekranını KALICI olarak gösterir, asla dönmez. preseed komutu başarısız
# olunca kurucu devam ettiğinden "exit 1" yetmez. Metin satırları ayrı argümanlar; "." boş satırdır.
dur() {
  ad=$1 baslik=$2; shift 2
  {
    echo "Template: wificorrect/$ad"
    echo "Type: error"
    echo "Description: $baslik"
    for s in "$@"; do echo " $s"; done
  } >"/tmp/wificorrect-$ad.templates"
  debconf-loadtemplate wificorrect "/tmp/wificorrect-$ad.templates" || true
  . /usr/share/debconf/confmodule
  while :; do
    db_fset "wificorrect/$ad" seen false || true
    db_input critical "wificorrect/$ad" || true
    db_go || true
    sleep 1
  done
}
