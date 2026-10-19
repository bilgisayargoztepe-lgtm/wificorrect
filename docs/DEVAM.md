# Devir notu — kaldığın yerden devam (2026-10-19)

Bu dosya, projeye yeni bir oturumda/hesapta devam eden ajan içindir. Önce bunu, sonra `CLAUDE.md`'deki belgeleri oku.

## Kullanıcıyla çalışma kuralları (zorunlu)
- **Her zaman Türkçe** konuş. Kullanıcı kısa ve doğrudan yanıt ister; "devam edeyim mi" diye sık sorma, uygun varsayılanla ilerle.
- Cihaz komutlarını **sen çalıştırırsın**: `ssh wificorrect` (root, anahtarla; cihaz 192.168.1.110, modem ağı).
  Sunucu: `ssh -i ~/.ssh/gbserver gbserver@192.168.1.109` (sudo parolası **yalnızca kullanıcıda**; sudo gereken işleri
  kullanıcıya terminalde çalıştırtırsın — cmd için: `"C:\Program Files\Git\bin\bash.exe" <betik>`). Claude'un terminal
  panelindeki sudo istemini kullanıcı görmüyor: sudo'lu işi görünür bir PowerShell penceresinde başlat
  (`Start-Process powershell -ArgumentList '-NoExit','-ExecutionPolicy','Bypass','-File','<betik.ps1>'`; betik repoya geçip
  `& "C:\Program Files\Git\bin\bash.exe" scripts/sunucu/merkez-guncelle.sh` çalıştırır; satır içi `-Command` tırnakları bozuyor).
- İkinci cihaz (Visavis, tünel 10.99.0.10) dükkân ağında değil; merkez üzerinden atlanır:
  `ssh -o HostKeyAlias=192.168.1.115 -o ProxyCommand="ssh -i ~/.ssh/gbserver -W %h:%p gbserver@192.168.1.109" -i ~/.ssh/bocafe root@10.99.0.10`
  (ana makine anahtarı dükkândaki ilk adresiyle kayıtlı).
- Yönetimde "Şimdi güncelle" gibi canlı cihazı etkileyen düğmelere kullanıcı basar (ajanın tıklaması izin denetimine takılıyor).
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
- Repo: `git clone https://github.com/bilgisayargoztepe-lgtm/wificorrect.git` (çalışma dalı `main`).
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
`cihaz.wificorrect.com` (cihaz paneline geçit), `ap.wificorrect.com` (AP arayüzü). Cihaz ↔ merkez WireGuard (10.99.0.0/24;
merkez .1, Visavis .10, Göztepe .11).
Canlı müşteriler: **Göztepe Bilgisayar = 1537344** (cihaz 192.168.1.110, test kanalı) ve **Visavis** (tünel 10.99.0.10,
kararlı kanal; numarası yönetimde; aşağıda "Visavis").

## Dallar ve durum (2026-10-19)
Repo `bilgisayargoztepe-lgtm/wificorrect` (avsharbeyi'den devredildi). Her şey **`main`**'de (PR #3–#26 birleşti; #2 kapatıldı).
**v0.4.16** yayında (2026-10-18; tarihler gün.ay.yıl; ISO + .deb, `release.yml`; `WFC_SSH_PUB` = `~/.ssh/bocafe.pub`). İki canlı
cihaz da 0.4.16 (merkezden uzaktan güncellemeyle); merkez main'i çalıştırıyor.
**Cihaza kurulum `gelistir.sh` ile değil**: yeni sürüm → `v*` etiketi (release ~20–25 dk) → Yönetim → Sürümler → "GitHub'dan
yeni sürümleri al" → kanal hedefi (Göztepe **test**, Visavis **kararlı**: ikisinin de hedefini seç) → müşteri sayfasında
"Şimdi güncelle" (kullanıcı basar) ya da 03:00–06:00 penceresi. Acil yerel deneme: cihazda `scripts/paket.sh` ile .deb, sunucuda
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
- **AP kurulumu → AP arayüzü** (PR #21–#26, 0.4.11–0.4.16; ayrıntı KURULUM_GUNLUGU "AP kurulumu", "AP yan odası",
  "AP arayüzü — merkez/cihaz"; teknisyen rehberi `docs/AP_KURULUM.md`): panel → Ayarlar → AP kurulumu: etiketteki adres →
  "Bağlan" (ARP yoklaması, yan oda = macvlan + ayrı ağ ad alanı + SOCKS aracı) ya da "Otomatik bul", yabancı DHCP denetimi,
  AP adresleri (kaydederken MAC sabitlenir), "AP arayüzüne git" → ap.wificorrect.com (merkez belirteci + `wca` çerezi + cihazda
  aktarım). 0.4.16: AP'nin HTTPS'i tercih edilir (modern ZyXEL girişi HTTP'den aktarılınca `/login`'e dönüyordu), "HTTP ile aç".
  Müşteriye özel **AP yönetim parolası** merkezde üretilir (yönetimde ve cihaz panelinde görünür); AP'ye panel parolası yazılmaz.
- **KVKK veri sorumlusu bilgileri** (0.4.12): Ayarlar → İşletme'de adres, telefon, e-posta, KEP; aydınlatma metni işletmeden bağımsız.
  Mobilde kullanıcılar tablosu sığar (Son görülme önde, Bugün MB sonra).
- **Kolay bağlama** (PR #23, 0.4.13): bağlanmamış cihaz merkeze 2 dk'da bir bildirir; müşteri aynı internet adresinden
  panel.wificorrect.com'a girince cihaz kendiliğinden bağlanır. Visavis böyle bağlandı.
- **ISO gerçek donanımda** (PR #24, 0.4.14): preseed initrd'de, kurulum belleği kimlikle seçilir (takılı başka kurulum diski
  karışmaz), `firmware-realtek`, iç disk yoksa boş USB belleğe kurulum. 0.4.15: boş/bozuk panel sertifikası yeniden üretilir
  (Visavis'te 0 baytlık `panel.crt/key` görüldü), monitörsüz açılış için EFI `startup.nsh`.

## Visavis (ikinci canlı müşteri, 2026-10-16'dan)
- J1900; ISO 0.4.13 ile 7,5 GB USB belleğe kuruldu, kolay bağlamayla bağlandı; tünel 10.99.0.10, **kararlı** kanal, 0.4.16.
- Portlar bu kutuda Göztepe'ninkinden farklı: internet **sol** port (`enp1s0`), misafir **sağ** port (`enp3s0`). Kurulumda
  internete çıkan port internet portu olur (`ag-ilk`); taşırken kablolar aynı porta takılmalı.
- Şu an geçici olarak dükkân dışında, ev fiber hattında (modem arkasında DHCP). Tünel bu dış sabit hatta günlerce sorunsuz
  (8–9 ms): 2026-10-13'te planlanan "dış sabit hatta tünel denemesi" tamam. Dükkândayken (merkezle aynı modem) dış adresten
  tünel kurulmamıştı (hairpin NAT); o zaman elle `wg set wfc peer … endpoint 192.168.1.109:51820` yapıldı.
- Misafir tarafında AP: **ZyXEL VMG1312-B10D** (fabrika adresi 192.168.1.1; yan odayla bulundu, HTTPS'ten yönetildi). Ayarlar
  (2026-10-19): LAN 10.50.0.2/24, DHCP kapalı, IPv6 kapalı (RA/DHCPv6 misafirlere gitmesin), Wi-Fi "VISAVIS" şifresiz, yönetim
  parolası = AP yönetim parolası (kullanıcı girdi); panelde AP adresi 10.50.0.2 kayıtlı, MAC 8c:59:73:a4:14:14 sabit; yan oda
  kapatıldı; yabancı DHCP denetimi temiz. Bu modelde ana ağ için **istemci yalıtımı yok** (yalnızca "Guest WLAN" kipinde var,
  o kip ZyXEL'in kendi adres dağıtımına geçtiği için kullanılamaz): misafirler birbirini görebilir; kalıcı çözüm yalıtımlı AP.
- İlk gerçek misafir SMS girişi 2026-10-19 00:44; 24 saatlik rapor temiz (servisler, hata, yedek, bütünlük).
- **Monitörsüz açılış (2026-10-19, Göztepe'de denendi):** bu BIOS (AMI Aptio CRB, CLBTM210 06/01/2015; iki kutuda aynı) UEFI
  açılış kayıtlarını saklamıyor: kurulumda yazılan `Boot0000 debian` sonra yok, çalışırken BootOrder/ConOut gibi hiçbir genel
  değişken görünmüyor; cihaz her açılışta `\EFI\BOOT\BOOTX64.EFI` yedek yolundan açılıyor. **Düzgün kapanıştan sonra ekransız
  açılış çalışıyor** (36 sn). **Elektrik kesintisinden (fiş çekme) ya da düğmeye basılı tutup kapatmadan sonra ekransız açılış
  çalışmıyor:** UEFI kabuğunda kalıyor ve `startup.nsh` hiç çalışmıyor (betiğin diske yazdığı iz oluşmadı; büyük ihtimalle kabuk
  diski görmüyor) → yazılım güncellemesiyle çözülmez. Sahte ekran (çevirici; Linux'ta `HDMI-A-1 connected`) takılıyken her
  durumda açılıyor. Çözüm: her kutuya sahte ekran fişi/çevirici + BIOS'ta "Restore on AC Power Loss = Power On" (şu an kapalı:
  elektrik gelince kendiliğinden açılmıyor; bu BIOS ayarları saklıyor mu, ayar sonrası fiş çekerek denenmeli). Tanı betiği
  (ekrana ve `\EFI\wificorrect\kabuk.txt`'ye iz yazar, GRUB'u doğrudan başlatır): Göztepe'de `/root/wfc-startup-tani.nsh`;
  ESP'de şu an paketteki `startup.nsh` var.
- Kurulum sırasında bulunan: kötü kablo sağ portu 100 Mb/yarım çift yönlü ve rx=0 gösterdi; kablo değişince düzeldi.

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
**Haftalık ve aylık raporlar — BİTTİ (2026-10-14/15, PR #19 + #20; alan adı listesi kaldırıldı, cihaz panelinde Ayarlar → Raporlar, v0.4.10; canlıda):**
spec `docs/superpowers/specs/2026-10-14-aylik-rapor-design.md`, plan `docs/superpowers/plans/2026-10-14-raporlar.md`.
Müşteri paneli menüsünde "Raporlar" (önce haftalar, sonra aylar), yönetimde müşteri sayfasında bağlantı; gerekçe/hareket yok
(kişisel veri yok). Engellenenler bölümü yok (kayıtta engel işareti yok), yazdır düğmesi yok (JS yok; Ctrl+P notu + print CSS).
Kurulum: `merkez-guncelle.sh` (kullanıcı, sudo), sonra hemen görmek için `sudo systemctl start wc-istatistik` (ilk çalıştırma
bütün arşiv günlerini bir kez hash'ler).
**Merkez yedeği YOK (2026-10-15 tespiti):** merkez.db, WireGuard sunucu anahtarı/kayıt listesi ve arşivler yalnızca
gbservers diskinde; `tasi-paketle` yalnızca sunucu çalışırken işe yarar. **Kullanıcı kararı (2026-10-19): "yedekler önemli
değil"** — gece yedeği yapılmayacak. Bilinen sonucu: merkez.db ve WireGuard sunucu anahtarı kaybolursa yeni merkez cihazları
tanımaz, tüneller kurulamaz, her cihaz yeniden bağlanmalı (tünel olmadığı için büyük ihtimalle yerinde). Cihazlar kendini
silmez (`MERKEZ_TANINMADI`, `src/ctl.rs`), lisans tarihi dolana kadar bağlı misafirlere hizmet verir ve 5651 kayıtlarını tutar;
ama SMS merkezden gittiği için yeni misafir giriş yapamaz. Merkezin Rust'a taşınması ertelendi (fayda küçük, risk büyük;
merkez ~36 MB). Sunucuda ayrıca Emek ERP, Kurye API, OSRM/Nominatim, PostgreSQL, Samba, xrdp ve başka işletmelerin web
siteleri çalışıyor — müşteri artınca merkezi ayrı sunucuya al.
**Ertelenenler (2026-10-19, kullanıcı: "sonra yapacağız"):**
- **Sunucu tek komutla kurulum:** `wificorrect-sunucu` .deb + `sunucu-kur.sh` (boş Ubuntu/Debian'da tek komut; Caddy, fail2ban,
  WireGuard, merkez, servisler; yalnızca yönetici kullanıcı adı/parolasını sorar; sonunda DNS, port, sertifika ve NetGSM IP'si
  kontrol listesi) + eski sunucudan "taşı" (müşteriler + tünel anahtarı; yoksa bütün cihazlar yeniden bağlanır). Alan adları
  sabit (cihaz programında 100'den fazla yerde `wificorrect.com`). Dikkat: gbservers'ta Caddy ortak — `caddy.service`
  `Caddyfile.main`'i çalıştırır, o da `/etc/caddy/Caddyfile`'ı (bizim; `kur.sh` bütünüyle yazar) ve `siteler*.caddy`'yi
  (Emek Koltuk, paketatmaca, 128bitbilgisayar … 10'dan fazla site) içe alır; paket yalnızca kendi dosyasını eklemeli.
  Windows sunucu: doğrudan değil (systemd, WireGuard + nftables, rrsync yedek hesapları, fail2ban Linux'a özgü); gerekirse
  Hyper-V'de Ubuntu sanal makine.
- **PPPoE / router kipi:** şimdi gerek yok (müşteride ONT + fiber modem; cihaz modemin arkasında DHCP ile çalışıyor; cihazın
  internet portu yalnızca DHCP bilir). İleride ürün seçeneği: PPPoE (+ VLAN, MTU/MSS) + işletme ağı (3. port) + personel
  Wi-Fi'ı; ONT'nin yerine geçmek yok (SFP yuvası yok, operatör ONT'yi seri numarasıyla yetkilendiriyor).
**Bulgular 2026-10-13:** cihaz telefon (mobil) hattına alınınca HTTPS/sağlık raporu çalışıyor ama WireGuard 1–2 dk sonra kesiliyor
(büyük olasılıkla mobil operatör engeli). Dış sabit hatta tünel denemesi yapıldı (Visavis, ev fiber hattı, günlerce sorunsuz);
mobil hat gerekirse tüneli HTTPS/443 kılıfına almak. NetGSM izinsiz-IP reddi (dinamik IP) SMS'i merkeze taşıma sebebi.
Monitörsüz açılış: yukarıda "Visavis". Uzaktan AP yönetimi yapıldı (0.4.11–0.4.16, "Bitenler"); yedek yol `scripts/ap-ac.ps1`
(SOCKS üzerinden ayrı Chrome).
**Güvenlik notları (2026-10-19 bakıldı):** misafir tarafında cihaza yalnızca DNS 53, DHCP 67 ve giriş sayfası 8080 açık; özel
ağlara (modem ağı, tünel) iletim kapalı. Giriş sayfası HTTP: açık Wi-Fi'de ad, telefon ve kod havada açık gider (kod isteyen
cihazın MAC'ine bağlı, başka cihazda işe yaramaz); MAC kopyalayan biri giriş yapmış misafirin adına çıkabilir (MAC tabanlı
bütün sistemlerde var). Misafirler AP'nin kendi yönetim sayfasına ulaşabilir (trafik cihazdan geçmez) → AP parolası şart.
gbservers: sshd parolayı yalnızca 192.168.1.0/24 (modem .1 hariç), Tailscale 100.64/10 ve localhost'tan kabul ediyor, dışarıdan
yalnızca anahtar; Samba (139/445), xrdp (3389) ve 9090 bütün arayüzlerde dinliyor → dükkân modeminin Wi-Fi şifresi
müşterilere verilmemeli.
Kullanıcının onayladığı sıra (2026-10-11; donanım bekçisi hariç): ~~1 küçük işler~~ · ~~2 sağlık izleme~~ ·
~~3 merkezden uzaktan güncelleme~~ · ~~4 hazır engel listeleri~~ · ~~5 aylık rapor~~ · **6 personel kodu ile giriş** ·
7 imzalı resmi talep raporu · 8 online ödeme (`veri.lisans_uzat`) · 9 **merkezi Rust'a taşıma** (2026-10-13 kararı;
~4.300 satır Python + 3.600 satır test, 9 işe bölünür: veritabanı, güvenlik, web tabanı, cihaz API, yönetim, müşteri paneli,
arka plan işleri, kurulum/yan yana geçiş, uçtan uca + Python'u kaldırma; aynı SQLite şeması ve parola biçimi korunur). Zaman damgası: kontör alınınca. İki aşamalı giriş istenmedi.
Ertelenmiş küçük bulgular: `.superpowers/sdd/*/progress.md` (yalnızca bu makinede).
Bekleyen kullanıcı işleri: iki J1900'de BIOS → "Restore on AC Power Loss = Power On" (monitör + klavyeyle; sonra fiş çekip
kendiliğinden açıldığını dene) ve yeni kutulara sahte ekran fişi; Yönetim → Uyarı ayarları'na telefon (SMS uyarıları hiç gitmiyor); Göztepe'nin TP-Link'inin yönetim
parolasını AP yönetim parolasına çevirmek (2026-10-16'ya kadarki kural panel parolasıydı); aydınlatma metni (misafir kodu
cümlesi ve yeni veri sorumlusu kalıbı) için hukukçu onayı; AP canlı kabul listesinin kalanı (KURULUM_GUNLUGU "AP arayüzü —
cihaz": Göztepe TP-Link'in yeni yoldan açılması, iPhone/Özel Geçiş, süre dolumu sayfası, `ap-ac.ps1 -Tunel`).

## Açık küçük notlar
- AP (TP-Link TL-WA901ND, 10.50.0.2) menzili zayıf; kullanıcı sonra ilgilenecek (iletim gücü High, kanal 1/6/11, 20 MHz).
- Monitörsüz açılış ve J1900 donması için GRUB ayarı `deploy/debian/etc/default/grub.d/wificorrect.cfg` (cihazda uygulandı);
  ekransız açılmayan J1900'lerde sahte ekran (çevirici) işe yarıyor; ayrıntı yukarıda "Visavis → Monitörsüz açılış".
- Ertelenmiş küçük bulgular ve verilen bütün kararlar: `.superpowers/sdd/*/progress.md` (git'e girmez, yalnızca bu makinede).
