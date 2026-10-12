#!/bin/bash
# Yönetim merkezi kurulumu/güncellemesi (Ubuntu, root). scripts/sunucu/dagit.sh çağırır; tekrar çalıştırmak güvenli.
set -euo pipefail
cd "$(dirname "$0")"
sed -i 's/\r$//' ./*.py ./*.service ./*.timer ./*.path ./*.conf Caddyfile hotspot-arsiv-saklama
L=/usr/local/lib/wificorrect
command -v caddy >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q caddy >/dev/null
command -v fail2ban-client >/dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y -q fail2ban >/dev/null
id wcpanel >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin wcpanel
install -d -m 755 $L /var/lib/wificorrect /var/lib/wificorrect/istatistik
install -d -o root -g wcpanel -m 750 /var/lib/wificorrect/detay /var/lib/wificorrect/kuyruk-sonuc
install -d -o wcpanel -g wcpanel -m 700 /var/lib/wificorrect/merkez /var/lib/wificorrect/kuyruk
rm -f $L/panel.py $L/panel_auth.py $L/hesap.py   # eski kafe paneli
install -m 644 common.py ozet.py detay.py guvenlik.py veri.py kayitlar.py web.py musteri.py yonetim.py api.py cihaz.py netgsm.py surumler.py $L/
install -m 755 merkez.py istatistik.py durum.py kuyruk.py yonetici.py saglik.py $L/
ln -sf $L/istatistik.py /usr/local/sbin/wc-istatistik
ln -sf $L/durum.py /usr/local/sbin/wc-durum
ln -sf $L/saglik.py /usr/local/sbin/wc-saglik
ln -sf $L/kuyruk.py /usr/local/sbin/wc-kuyruk
ln -sf $L/yonetici.py /usr/local/sbin/wc-yonetici
rm -f /usr/local/sbin/wc-hesap
install -m 644 wc-merkez.service wc-kuyruk.path wc-kuyruk.service wc-durum.service wc-durum.timer \
  wc-istatistik.service wc-istatistik.timer wc-saglik.service wc-saglik.timer /etc/systemd/system/
# geçersiz Caddyfile canlı siteleri düşürmesin: önce doğrula, canlı dosyaya dokunmadan dur
caddy validate --adapter caddyfile --config Caddyfile >/dev/null 2>&1 || { echo "HATA: Caddyfile geçersiz, kurulum durdu (canlı Caddyfile değişmedi):" >&2; caddy validate --adapter caddyfile --config Caddyfile >&2 || true; exit 1; }
[ -f /etc/caddy/Caddyfile ] && cp -n /etc/caddy/Caddyfile /etc/caddy/Caddyfile.eski
[ -f /etc/caddy/Caddyfile ] && cp -p /etc/caddy/Caddyfile "/etc/caddy/Caddyfile.yedek-$(date +%Y%m%d-%H%M%S)"
install -m 644 Caddyfile /etc/caddy/Caddyfile
install -m 755 hotspot-arsiv-saklama /etc/cron.daily/
install -m 644 fail2ban-wc-merkez-filtre.conf /etc/fail2ban/filter.d/wc-merkez.conf
install -m 644 fail2ban-wc-merkez.conf /etc/fail2ban/jail.d/wc-merkez.local
# eski kafe paneli: durdur, hesap dosyasını silmeden kenara al
if systemctl list-unit-files wc-panel.service >/dev/null 2>&1; then systemctl disable -q --now wc-panel 2>/dev/null || true; rm -f /etc/systemd/system/wc-panel.service; fi
[ -f /etc/wificorrect/hesaplar.json ] && mv -n /etc/wificorrect/hesaplar.json /etc/wificorrect/hesaplar.json.eski
systemctl daemon-reload
systemctl enable -q --now wc-kuyruk.path wc-durum.timer wc-istatistik.timer wc-saglik.timer
systemctl enable -q wc-merkez
systemctl restart wc-merkez
systemctl start wc-durum.service
systemctl reload caddy 2>/dev/null || systemctl restart caddy
systemctl reload fail2ban 2>/dev/null || systemctl restart fail2ban
[ -s /var/lib/wificorrect/merkez/yonetici.json ] || echo "Yönetici hesabı yok: sudo wificorrect-sunucu yonetici-parola <kullanıcı-adı>"
echo "kurulum tamam: wc-merkez $(systemctl is-active wc-merkez), kuyruk $(systemctl is-active wc-kuyruk.path)"
