# Devir notu — kaldığın yerden devam (2026-10-12, akşam)

Bu dosya, projeye yeni bir oturumda/hesapta devam eden ajan içindir. Önce bunu, sonra `CLAUDE.md`'deki belgeleri oku.

## Kullanıcıyla çalışma kuralları (zorunlu)
- **Her zaman Türkçe** konuş. Kullanıcı kısa ve doğrudan yanıt ister; "devam edeyim mi" diye sık sorma, uygun varsayılanla ilerle.
- Cihaz komutlarını **sen çalıştırırsın**: `ssh wificorrect` (root, anahtarla; cihaz 192.168.1.110, modem ağı).
  Sunucu: `ssh -i ~/.ssh/gbserver gbserver@192.168.1.109` (sudo parolası **yalnızca kullanıcıda**; sudo gereken işleri
  kullanıcıya terminalde çalıştırtırsın — cmd için: `"C:\Program Files\Git\bin\bash.exe" <betik>`).
- **Asla** parola, API anahtarı, belirteç girme/kullanma — kullanıcı sohbete yazsa bile. `openwrt/serverp` dosyasını okuma.
- Gerçek SMS yalnızca kullanıcı açıkça onaylarsa. `/srv/5651` dosyalarını silme. Ağ/güvenlik duvarı değişikliği yalnızca
  ölü adam anahtarıyla (yedek + `systemd-run --on-active=300` geri alma + YENİ bağlantıyla doğrula + iptal).
- Sırlar repoya girmez (repo **herkese açık**: github.com/bilgisayargoztepe-lgtm/wificorrect).
- Cihaz testleri **cihazda** koşar: `bash scripts/test.sh wificorrect [süzgeç]` (~5 dk; aynı anda iki tane çalıştırma).
  Cihaza kurulum: `bash scripts/gelistir.sh` + `ssh wificorrect 'systemctl restart wificorrect-panel wificorrect-portal wificorrect-kaydedici'`.
  Merkez testleri: `sh scripts/sunucu/merkez/tests/hepsi.sh python`.
- Merkez kurulumu: `scripts/sunucu/merkez-guncelle.sh` (şu anki daldan; önce veritabanı yedeği, sonra `kur.sh` —
  Caddyfile doğrulanmadan kurulmaz). Kullanıcı çalıştırır (sudo).

## Yeni bilgisayar kurulumu (2026-10-10'da başka PC'ye geçildi)
- Gerekenler: Git for Windows (Git Bash), Python 3, GitHub CLI (`gh auth login`, hesap bilgisayargoztepe-lgtm), Claude Code.
  Rust gerekmez (derleme ve testler cihazda).
- Repo: `git clone https://github.com/bilgisayargoztepe-lgtm/wificorrect.git` → `git checkout uzak-panel`.
- SSH: `~/.ssh/config`'e
  ```
  Host wificorrect
      HostName 192.168.1.110
      User root
      IdentityFile ~/.ssh/bocafe
  ```
  Anahtarlar: `~/.ssh/bocafe` (cihaz root) ve `~/.ssh/gbserver` (sunucu). Ya eski PC'den güvenli yolla kopyalanır ya da yeni
  PC'de yeni anahtar üretilip açık anahtar cihazın `/root/.ssh/authorized_keys`'ine ve sunucuda
  `gbserver`'ın `~/.ssh/authorized_keys`'ine eklenir.
- Ağ: cihaz (192.168.1.110) ve sunucu (192.168.1.109) SSH'ı yalnızca dükkânın modem ağından. Dışarıdan çalışılıyorsa
  WireGuard yönetici bağlantısı (`wificorrect-disaridan.conf`) açılır; o zaman cihaz `10.99.0.11`, sunucu `10.99.0.1`.

## Ürün özeti
WifiCorrect: işletmelere yıllık abonelikle satılan 5651 uyumlu, SMS (NetGSM) doğrulamalı misafir internet cihazı.
Cihaz: Debian 13 + Rust ikilisi `wificorrect` (`src/`). Merkez: Python stdlib (`scripts/sunucu/merkez/`), Caddy 2.6.2 arkasında,
`yonetim.wificorrect.com` (hizmet sağlayıcı), `panel.wificorrect.com` (müşteri), `api.wificorrect.com` (cihazlar),
`cihaz.wificorrect.com` (cihaz paneline geçit). Cihaz ↔ merkez WireGuard (10.99.0.0/24; merkez .1, cihaz 1537344 = .11).
Canlı müşteri: **Göztepe Bilgisayar = 1537344**, cihaz 192.168.1.110.

## Dallar ve durum (2026-10-12)
Repo `bilgisayargoztepe-lgtm/wificorrect` (avsharbeyi'den devredildi). Her şey **`main`**'de (PR #3–#12 birleşti; #2 kapatıldı).
**v0.4.4** yayında (2026-10-12; tarihler gün.ay.yıl, PR #13; ISO + .deb, `release.yml`; `WFC_SSH_PUB` = `~/.ssh/bocafe.pub`). Canlı cihaz 0.4.4'e
**merkezden uzaktan güncellemeyle** geçti (GitHub → Sürümler → test kanalı → Şimdi güncelle; ilk gerçek kullanım). Cihaz **paketli**
(dpkg `wificorrect` 0.4.3, yerelde derlenmiş; elle geçiş ölü adam anahtarıyla) ve merkez main'i çalıştırıyor.
**Cihaza kurulum artık `gelistir.sh` ile değil**: yeni sürüm → `v*` etiketi → Yönetim → Sürümler → "GitHub'dan al" → test kanalı
hedefi + Göztepe (test kanalında) "Şimdi güncelle". Acil yerel deneme: cihazda `scripts/paket.sh` ile .deb, sunucuda
`sudo -u wcpanel python3 /usr/local/lib/wificorrect/surumler.py ekle DOSYA SURUM` (gelistir.sh paketli cihazı paketten saptırır).
Merkez her güncellemede yeniden başlar ve bellekteki oturumlar düşer (yönetime yeniden giriş gerekir).

Bitenler (canlıda):
- ISO (Debian netinst + preseed + .deb), GitHub'da QEMU duman testi (`iso.yml`), `release.yml` yalnızca `v*` etiketinde.
- Parolalar: müşteri parolasını yönetici yazar; tek admin parolası (merkezden bütün cihazlara).
- Uzak panel (spec 2026-10-08): giriş yalnızca panel.wificorrect.com'dan → `cihaz.wificorrect.com`; telefondan denendi, çalışıyor.
- **Panel görünümü** (örnek görsel: koyu üst şerit + koyu sol menü, kırmızı vurgu): menü Özet, Kullanıcılar, Kayıtlar,
  Ayarlar, Admin; grup sayfalarında simgeli kutucuklar + kırmızı başlıklı bölüm kutuları. **Özet**: kısayollar (Bağlı
  kullanıcılar, Site/IP ara, Yasaklı siteler), Bağlı Kullanıcılar, sayılar, Bugün Yeni Üyeler, SMS ve Disk halkaları
  (telefon/MAC yok; her bakış `PANEL_OZET` denetimi).
- **Hız ve yavaşlatma** (spec `docs/superpowers/specs/2026-10-10-hiz-ve-kullanim-design.md`): nft `inet wfc_hiz` IP başına
  sayaç (`/etc/wificorrect/hiz.nft`), kaydedici 5 sn'de `/run/wificorrect/trafik.json`; Bağlı kullanıcılar tablosunda
  ↓/↑ Mb/sn, Bugün MB, sıralama, 10 sn yenileme; Yavaşlat 1/2/5/10/20 Mb/sn (HTB + meta priority, MAC'e bağlı, kaldırılana
  kadar), Yavaşlatılmış cihazlar bölümü; izinli cihazlar da görünür. Kabul: `scripts/hiz-deneme.sh` (sahte istemci; WAN'ı
  ~100 sn doldurur, sakin saatte) 2026-10-10'da 5/5 PASS. Canlıya kurulum ölü adam anahtarıyla yapıldı.

- **Merkezde sağlık izleme + SMS uyarısı** (spec `docs/superpowers/specs/2026-10-11-saglik-izleme-design.md`): cihaz 5 dk'da
  bir `/api/saglik`; merkezde `wc-saglik.timer` kurallar + NetGSM genel SMS (`netgsm.py`); Yönetim'de "Sağlık izleme: son tur",
  Açık sorunlar, müşteri sağlık kartı, sorun geçmişi, **Uyarı ayarları** (telefon + deneme SMS'i; kullanıcı henüz girmedi).
  Merkez testleri `WC_TEST` ile gerçek SMS'i reddeder.
- Yönetim → müşteri sayfası **"Cihaz paneline git"** (yönetici geçiş belirteci; çıkış yönetime döner, müşteri oturumu düşmez).
- Caddy düzeltmeleri (PR #8): `header_up Cookie "…" ""` Caddy 2.6.2'de Cookie'yi EZİYORDU (cihaz oturumu her istekte
  yenileniyor, formlar CSRF'ye takılıyordu) → `"wcc=[^;]*" "_"`; CSP `form-action`'a panel ve yönetim eklendi (çıkış).
  Caddyfile değişikliğini kurmadan önce sunucuda `caddy adapt --config … --adapter caddyfile` ile denetle.
- **İşletme türü + misafir kodu** (PR #9, spec `docs/superpowers/specs/2026-10-11-isletme-turu-misafir-kodu-design.md`):
  Ayarlar'da tür (kafe/otel/yurt/diğer; seçilmemişse Özet uyarısı, merkeze eşitlenir); otel/yurtta **Misafir kodları**
  (ad, soyad, TC ya da pasaport+uyruk, oda, 24 sa–1 ay; 6 haneli kod, TR+EN fiş, iptal/uzat). Portal TR/EN, "Otel misafiriyim".
  5651'de kimlik `telefon` sütununda (`TC:…`, `PASAPORT:UY:NO`), ekranlarda maskeli, URL'de opak kimlik. Cihazda sahte
  istemciyle uçtan uca geçti (2026-10-12). Canlı cihazda tür **seçilmemiş**, misafir kodu kapalı. Aydınlatma metni cümlesi
  hukukçu onayı bekliyor. Bilinen: seçilen tür panelden "seçilmemiş"e geri alınamaz.
- **Uzaktan güncelleme** (PR #12, spec `docs/superpowers/specs/2026-10-12-uzaktan-guncelleme-design.md`; ayrıntı
  KURULUM_GUNLUGU "Uzaktan güncelleme"): kanallar test/kararlı, müşteri başına kanal (Göztepe = test), 03:00–06:00 penceresi,
  sha doğrulama, eski paket saklanır, eski ikilinin yazdığı `denetle.sh` + 5 dk zamanlayıcı 10 dk sonra sağlıksızsa geri döner,
  başarısız sürüm tekrar kurulmaz. Canlıda uçtan uca geçti (0.4.0→0.4.1 tamam, bozuk 0.4.2 geri alındı, 0.4.1→0.4.0 tamam).
  Kanal hedefleri şu an boş. Bilinen küçükler: hedef kaydında mesaj yok; müşteri sayfası "Cihaz → Sürüm" günlük eşitlemeye
  kadar eski; ilk güncellemede eski sürümün paketi merkezde yoksa `onceki_paket_yok` (0.4.3 GitHub'dan alınmalı).
- **Sade özet + bağlı kullanıcılar** (PR #10, #11): özet kısayolsuz, SMS/Disk yok; bağlı kullanıcıda yeşil nokta + bağlı dakika
  (15 dk'dan kısa kopmada sürer). Kullanıcılar tablosu: Durum · Telefon · Ad · Bugün MB · Son görülme · ↓ · ↑ · Yavaşlat · Kes ·
  MAC · IP. Kaynak: kaydedici 30 sn'de `state/varlik.json` (45 gün).
- Küçük işler (PR #6): bozuk `hiz_sinir.json`'un üzerine yazılmaz; 10 sn yenileme yalnızca `/oturumlar`; panel tam genişlik.

## Sıradaki işler
**SMS merkezden — BİTTİ (2026-10-14, PR #17, v0.4.8):** misafir doğrulama SMS'i (NetGSM OTP + Twilio Verify) merkezden
(`/api/sms`, `/api/sms-dogrula`; sabit şablon, 6 hane, cihaz başına 300/gün, yabancı 20/gün; merkeze ulaşılamazsa "Şu an kod
gönderilemiyor…"). Cihazda NetGSM/Twilio sırrı yok (eski dosyadan temizlendi). Merkez SMS ayarları Yönetim → SMS ayarları'nda
(NetGSM girildi, deneme modu kapalı, Twilio kapalı). Canlı: merkezden gerçek OTP (kullanıcı onayı, kendi numarası) ulaştı;
cihaz 0.4.8'e uzaktan güncellendi; deneme modunda sahte misafir uçtan uca giriş yaptı, deneme modu geri kapatıldı.
Ertelenmiş küçükler: SMS ayarlarında Twilio sırrı silinemiyor; `ctl saglik` ile panel aynı anda kaydederse küçük yarış;
5 sn/işlem zaman aşımı DNS'i kapsamaz.
**Hazır engel listeleri — BİTTİ (2026-10-14, PR #18, v0.4.9):** kumar (HaGeZi gambling), yetişkin (HaGeZi nsfw + StevenBlack),
zararlı (HaGeZi tif.mini + USOM; USOM şu an HTML döndürüyor, kaynak hatalı sayılıyor, diğeri yetiyor). VPN/proxy kategorisi yok
(kullanıcı kararı). Merkez `wc-listeler.timer` 04:30, Yönetim → Engel listeleri; kategori hiç güncellenemez ya da 48 sa eskirse
"Merkez" satırıyla sağlık uyarısı. Cihaz dnsmasq `servers-file` + SIGHUP; panel Yasaklı siteler → Hazır listeler (varsayılan açık).
Canlı ölçüm (561.821 alan): dnsmasq RSS ~3 MB → ~47 MB; SIGHUP'ta DNS ~4 sn duruyor (yalnızca liste değişince, günde ~1);
bet365/pornhub NXDOMAIN, google/netgsm/banka/e-Devlet/bağlantı denetimi açık. `wc-listeler.service` USOM hatası yüzünden
çıkış 1 veriyor (zararsız). Ertelenmiş küçükler: `.superpowers/sdd/2026-10-14-hazir-engel-listeleri/progress.md`.
**Haftalık ve aylık raporlar — kod bitti (2026-10-14, dal `aylik-rapor`, yalnızca merkez; birleşme ve kurulum bekliyor):**
spec `docs/superpowers/specs/2026-10-14-aylik-rapor-design.md`, plan `docs/superpowers/plans/2026-10-14-raporlar.md`.
Müşteri paneli menüsünde "Raporlar" (önce haftalar, sonra aylar), yönetimde müşteri sayfasında bağlantı; gerekçe/hareket yok
(kişisel veri yok). Engellenenler bölümü yok (kayıtta engel işareti yok), yazdır düğmesi yok (JS yok; Ctrl+P notu + print CSS).
Kurulum: `merkez-guncelle.sh` (kullanıcı, sudo), sonra hemen görmek için `sudo systemctl start wc-istatistik` (ilk çalıştırma
bütün arşiv günlerini bir kez hash'ler).
**Bulgular 2026-10-13:** cihaz telefon (mobil) hattına alınınca HTTPS/sağlık raporu çalışıyor ama WireGuard 1–2 dk sonra kesiliyor
(büyük olasılıkla mobil operatör engeli); müşteriler sabit hatlı (TT/Superonline) olacağı için ilk müşteri kurulumundan önce
gerçek bir dış sabit hatta 15 dk tünel denemesi yapılacak, engel çıkarsa tüneli HTTPS/443 kılıfına almak. NetGSM izinsiz-IP
reddi (dinamik IP) SMS'i merkeze taşıma sebebi. Monitörsüz açılış hâlâ sorunlu (BIOS'ta takılıyor olabilir; teşhis adımı
kullanıcıya anlatıldı, HDMI sahte fiş önerildi). Uzaktan AP yönetimi fikri: 1) merkez → tünel → cihaz → AP web arayüzü
aktarımı (Referer/Host AP'nin adresine çevrilir), 2) bilinen modellere sürücü, 3) standart AP — kullanıcıya "AP'yi siz mi
vereceksiniz?" soruldu, cevap bekleniyor. AP'ye bugün erişim: `ssh -N -D 1080 wificorrect` + ayrı Chrome
`--proxy-server=socks5://127.0.0.1:1080` → http://10.50.0.2/ (TP-Link 127.0.0.1 üzerinden girişte Referer yüzünden reddeder).
Canlı cihaz 0.4.9 (Admin, Ayarlar'ın son kutucuğu; kullanıcılar 10'arlı sayfa; tarihler gün.ay.yıl).
Kullanıcının onayladığı sıra (2026-10-11; donanım bekçisi hariç): ~~1 küçük işler~~ · ~~2 sağlık izleme~~ ·
~~3 merkezden uzaktan güncelleme~~ · ~~4 hazır engel listeleri~~ · ~~5 aylık rapor~~ (birleşme/kurulum bekliyor) · 6 personel kodu ile giriş ·
7 imzalı resmi talep raporu · 8 online ödeme (`veri.lisans_uzat`) · 9 **merkezi Rust'a taşıma** (2026-10-13 kararı;
~4.300 satır Python + 3.600 satır test, 9 işe bölünür: veritabanı, güvenlik, web tabanı, cihaz API, yönetim, müşteri paneli,
arka plan işleri, kurulum/yan yana geçiş, uçtan uca + Python'u kaldırma; aynı SQLite şeması ve parola biçimi korunur). Zaman damgası: kontör alınınca. İki aşamalı giriş istenmedi.
Ertelenmiş küçük bulgular: `.superpowers/sdd/*/progress.md` (yalnızca bu makinede).
Bekleyen kullanıcı işleri: Yönetim → Uyarı ayarları'na telefon (SMS uyarıları hiç gitmiyor); 0.4.3'ü "GitHub'dan al";
aydınlatma metni misafir kodu cümlesi için hukukçu onayı; `docs/superpowers/specs/2026-10-12-harici-ap-design.md` (izlenmeyen,
başka oturumdan) — kullanıcıya sorulmadı.

## Açık küçük notlar
- AP (TP-Link TL-WA901ND, 10.50.0.2) menzili zayıf; kullanıcı sonra ilgilenecek (iletim gücü High, kanal 1/6/11, 20 MHz).
- Monitörsüz açılış ve J1900 donması için GRUB ayarı `deploy/debian/etc/default/grub.d/wificorrect.cfg` (cihazda uygulandı).
- Ertelenmiş küçük bulgular ve verilen bütün kararlar: `.superpowers/sdd/*/progress.md` (git'e girmez, yalnızca bu makinede).
