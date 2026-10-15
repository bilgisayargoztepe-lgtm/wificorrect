# Müşterinin taktığı AP'yi uzaktan kurmak — tasarım

Tarih: 2026-10-15 · Kullanıcıyla konuşularak; "(benim kararım)" işaretli yerler kullanıcıya sorulmadı.

## Neden ve akış

Müşteri (işletme sahibi) erişim noktasını (AP) kendisi alır, paketinden çıkarıp WifiCorrect cihazının misafir portuna
takar ve bize "taktım" diye haber verir: **marka, model ve etiketteki bilgiler** (varsayılan adres ya da alan adı,
fabrika kullanıcı adı/parolası, MAC). Biz zaten cihaza merkez → WireGuard tüneli üzerinden erişebiliyoruz; AP'nin web
arayüzüne de cihaz üzerinden girip ayarlarını uzaktan yaparız. Müşteri hiçbir ayar yapmaz.

Bu iş **ara sıra, çoğu müşteride bir kez** yapılır ve yalnızca hizmet sağlayıcı (biz) yapar. Bu yüzden yönetimden tek
tıkla açılan bir web aracısı **yapılmaz**; bugün de çalıştığı gösterilmiş SOCKS yöntemi kullanılır ve kolaylaştırılır.

Sorun: kutudan çıkan AP misafir ağımızda (ör. 10.50.0.0/24) değildir. Ya DHCP'den adres ister (yeni modellerin çoğu AP
modunda böyle; o zaman cihazımızın DHCP'sinden 10.50.0.x alır ve görünür), ya da sabit fabrika adresi vardır
(192.168.0.254, 192.168.1.1 …) ve cihaz o ağa ulaşamaz.

## Hedefler

1. **AP kurulum modu (cihaz paneli → Ayarlar → AP kurulumu; hizmet sağlayıcı ve işletme sahibi):**
   **Karar (2026-10-15, kullanıcı):** işletme sahibi de bu bölümü admin gibi kullanır (ağdaki cihazlar, geçici adres
   ekle/kaldır, AP adresi kaydı); admin kilidi yoktur. CSRF, denetim (kim + rol) ve bütün çakışma/doğrulama kuralları
   aynen geçerlidir. Bölüm Admin'den Ayarlar grup sayfasına taşındı. (İlk taslakta "yalnızca admin" idi.)
   - **Ağdaki cihazlar:** misafir portunda son görülen cihazlar — DHCP kiraları ve ARP/komşu tablosu: MAC, IP, cihaz adı,
     (biliniyorsa) üretici. Müşterinin verdiği MAC'le eşleştirmeye yarar.
   - **Geçici adres:** kullanıcı bir alt ağ adresi girer (ör. `192.168.0.250/24`; müşterinin bildirdiği fabrika adresine göre);
     cihaz bunu misafir köprüsüne **ikinci adres** olarak ekler, böylece fabrika adresli AP'ye ulaşır. Adres **30 dk sonra
     kendiliğinden kalkar** (açılışta da kalmaz); "Şimdi kaldır" düğmesi vardır. Misafir ağının kendi adresiyle, WAN'ın
     ağıyla ya da tünel ağıyla (10.99.0.0/24) çakışan, özel ağ aralığı dışındaki ya da /16'dan geniş alt ağ reddedilir.
     Geçici adres açıkken bile misafir trafiği o alt ağa yönlendirilmez (misafirden özel ağlara erişim zaten kapalı;
     bu kural bozulmaz).
   - Denetim: `AP_KURULUM_ADRES eklendi|kaldirildi adres=… neden=elle|sure|acilis`.
2. **AP adresi kaydı (aynı sayfa):** AP ayarlandıktan sonra kullanıcı AP'nin kalıcı yönetim adresini yazar (ör. 10.50.0.2;
   yalnızca misafir alt ağı içinde ve DHCP havuzu dışında kabul edilir). Ayar `ayarlar.toml`'da.
   **Önemli sınır:** AP'ye kablosuz bağlı bir misafirin AP'nin kendi adresine giden paketi cihazımızdan **hiç geçmez**
   (AP onu kendi içinde alır); cihazın güvenlik duvarı bunu göremez. Bu yüzden asıl koruma **AP'nin kendi ayarıdır**
   (aşağıda 4: kablosuzdan yönetimi kapatma / yönetimi yalnızca 10.50.0.1'e izin verme + güçlü parola). Cihazın kuralı
   yalnızca **köprüden geçen** trafiği kapatır: başka bir porttaki (kablolu ya da ikinci AP'deki) misafirden AP adresine
   giden bütün trafik, köprü katmanında (bridge ailesi, `kapi.nft`'teki köprü yalıtımıyla aynı yer) düşürülür; ARP serbest
   kalır (yoksa AP ağda kaybolur), IP trafiğinin tamamı (TCP/UDP/ICMP) düşer (benim kararım). Cihazın kendisi (10.50.0.1)
   erişmeye devam eder, dolayısıyla tünelden SOCKS ile biz de. Birden çok AP için liste (benim kararım: en çok 8 adres). Denetim `PANEL_AP_ADRES`.
   Güvenlik duvarı değişikliği mevcut kural dosyası düzeniyle, açılışta da uygulanır (kapalıyken güvenli ilkesi bozulmaz).
   **Karar (2026-10-15, uygulama sırasında): ayrı güvenlik duvarı kuralı YAPILMADI.** `kapi.nft`'teki köprü yalıtımı
   (`table bridge hotspot` → `isolate`) köprü portları arası bütün iletimi zaten düşürüyor (başka porttaki misafir →
   AP adresi bugün de geçmiyor); AP'ye kablosuz bağlı misafirin AP'ye giden trafiği ise cihazdan hiç geçmez. İkinci kat
   kural (ayrı tablo, `ap.nft`, postinst/servis değişikliği) canlı güvenlik duvarına risk ekleyip koruma eklemediği için
   bırakıldı. AP adresi kaydı yalnızca kayıttır (`[ap] adresler`, en çok 8, aynı doğrulama; panelde gösterilir, ağdaki
   cihazlar listesinde işaretlenir; denetim `PANEL_AP_ADRES`). Yalıtım ileride gevşetilirse bu kural yeniden ele alınmalı.
3. **SOCKS kısayolu (bizim Windows bilgisayarımız için, repoda `scripts/ap-ac.ps1` ya da `.cmd`):** tek komutla
   - cihaza SSH ile SOCKS tüneli açar (`ssh -N -D <port>`; hedef: dükkân ağındaysa `wificorrect`, dışarıdan tünel adresi —
     cihaz parametre olarak verilir, ör. `10.99.0.11`),
   - ayrı profilli bir Chrome penceresini bu proxy ile ve verilen AP adresiyle açar
     (`--proxy-server=socks5://127.0.0.1:<port> --user-data-dir=<geçici>`; böylece normal Chrome etkilenmez),
   - Chrome kapanınca tüneli kapatır.
   Sır içermez; SSH anahtarı kullanıcının mevcut anahtarıdır.
4. **AP ayar listesi (belgede, `docs/AP_KURULUM.md`):** her markada uygulanacak liste ve sıra:
   çalışma modu Access Point · DHCP sunucusu ve "Smart IP" benzeri otomatik modlar kapalı · ağ adı (SSID) işletmenin adı,
   şifresiz · istemci yalıtımı (AP isolation) açık · kanal 1/6/11, 20 MHz, iletim gücü yüksek · **yönetim parolası =
   işletme sahibinin panel parolası** (merkezde müşteri sayfasında yöneticiye zaten görünür) · en son yönetim adresi sabit
   10.50.0.2/24, ağ geçidi ve DNS 10.50.0.1 (adres değişince bağlantı kopar; yeni adresten yeniden girilir) · **en son,
   AP 10.50.0.2'deyken ve yeni adresten girildikten sonra yönetim erişimi kısıtı**: modelde varsa "kablosuzdan yönetimi
   kapat" ("Wireless management" / "Management via wireless" kapalı) ya da "yönetime yalnızca şu IP/MAC izin ver" =
   10.50.0.1 / cihazın misafir portu MAC'i (yoksa yalnızca güçlü parolaya kalır; bu not edilir) · sonra adım 2.
   **Sıra neden böyle:** fabrika adresli AP'ye geçici adresten (ör. 192.168.0.250) girilir; AP istekleri 10.50.0.1'den
   değil oradan görür. IP kısıtı adres değişikliğinden önce yazılırsa kaydedince oturum kesilir, adres değiştirilemez ve
   AP'ye cihazdan ulaşılamaz (müşteri yerinde sıfırlar). MAC kısıtı (köprü MAC'i aynı) erken de güvenlidir.
   Bilinen tuzaklar: TP-Link'te 127.0.0.1 port yönlendirmesiyle giriş Referer yüzünden reddedilir (SOCKS'ta olmaz); bazı
   modellerde parola en çok 15 karakter — müşteri parolası uzunsa kısaltılır ve kullanılan parola müşteri sayfasına not
   olarak yazılır.

   - **Bilinen sınır:** AP'nin fabrika adresi modemin ağındaysa (ör. modem 192.168.1.1, AP de 192.168.1.1/254) geçici
     adres eklenemez (cihazın internet yolu bozulur). İlk sürümde panel bunu açıkça söyler; çözüm yolu (uygulayıcı
     isterse ve küçükse): ayrı bir ağ ad alanında (netns) köprüye bağlı geçici arayüz. Yoksa müşteriden AP'yi DHCP'ye
     alması / sıfırlaması istenir (çoğu yeni AP AP modunda zaten DHCP istemcisidir).
   - Geçici adres durumu `/run` altında (açılışta kendiliğinden yok), süre `systemd-run --on-active` zamanlayıcısıyla;
     kayıtlı AP adresleri kuralı panel kaydında hemen ve açılışta/güvenlik duvarı yeniden yüklenince yeniden uygulanır
     (uygulamada: durum `/var/lib/wificorrect/ap-gecici.json`, açılışta temizlenir; AP adresi kuralı yok — yukarıdaki karar).
5. **Kabul:** cihazda sahte istemciyle (`scripts/sahte-istemci.sh`, ayrı köprü portu) köprüden AP adresine erişimin
   kapandığı, 10.50.0.1'den açık olduğu ve geçici adresin 30 dk sonra kalktığı doğrulanır; canlıda Göztepe AP'si
   (10.50.0.2) kaydedilir ve AP'deki kablosuz yönetim ayarı elle kontrol edilir.

## Hedef dışı

- Yönetimden tek tıkla AP arayüzü aracısı (cihaz üzerinden HTTP proxy, Host/Referer çevirme).
- Markaya özel otomatik ayar sürücüleri; AP'yi programla ayarlamak.
- AP'nin parolasını müşteri parolası değişince kendiliğinden güncellemek (elle yeniden girilir).
- İşletme sahibine AP'nin kendi web arayüzüne giriş aracı (SOCKS betiği cihaza SSH anahtarı ister; panel bölümü ise
  2026-10-15 kararıyla işletme sahibine de açık).

## Anti-hedefler

- **Misafirin AP yönetimine ulaşması:** ağ şifresiz; AP adresi kaydedildikten sonra misafir AP'nin hiçbir kapısına
  ulaşamamalı.
- **Kalıcı yan etki:** geçici adres unutulursa kendiliğinden kalkmalı; misafir ağının, DHCP'nin ya da internet erişiminin
  bozulmaması. Ağ/güvenlik duvarı kodu değiştiği için canlıya kurulum ve deneme ölü adam anahtarıyla yapılır.
- **Misafirlerin internetini kesmek:** kurulum modu sırasında misafirler normal çalışmaya devam eder.

## Kısıtlar

- Cihaz Rust, mevcut panel ve güvenlik duvarı (`kapi.nft` / `guvenlik.nft`) düzeni; merkezde değişiklik gerekmez.
- Yeni cihaz sürümü uzaktan güncellemeyle kurulur.
- Mevcut AP (Göztepe, TP-Link 10.50.0.2) ilk kullanıcı: adresi kaydedilerek misafir erişimi kapatılır.

## Uygulayıcıya bırakılanlar

Sayfa yerleşimi, geçici adresin zamanlayıcı biçimi (`systemd-run --on-active` gibi), güvenlik duvarı kuralının nereye
konacağı, üretici adı için küçük OUI tablosu olup olmayacağı, kısayol betiğinin biçimi.
