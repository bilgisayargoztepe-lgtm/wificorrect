# AP arayüzüne panelden tek tıkla erişim — tasarım

Tarih: 2026-10-16 · Kullanıcıyla konuşularak, onaylandı ("onaylıyorum"). "(benim kararım)" işaretli yerler kullanıcıya sorulmadı.
Önceki tasarım: `2026-10-15-ap-kurulum-design.md` (yan oda, SOCKS aracı, `ap-ac` betiği).

## Neden

Bugün müşterinin taktığı AP'nin/modemin web arayüzüne yalnızca hizmet sağlayıcının bilgisayarından, SSH + `ap-ac.ps1` ile
girilebiliyor; cihaz müşterideyken bilgisayarın cihaza doğrudan ulaşması da gerekiyor (cihaz başına SSH ayarı / kısayol).
Kullanıcı isteği: "yan oda açılınca arayüz ekranına git butonu gelsin, her cihaz için kod yazmayacağımız bir senaryo".
Ayrıca müşteriye "modemin/AP'nin altındaki etikette yazan IP'yi buraya yazın" denebilsin; yan oda adresini kimse
hesaplamasın.

## Kullanıcı akışı

1. Cihaz paneli → Ayarlar → **AP kurulumu**: yeni **"Takılan cihaz"** bölümü:
   - **Etiketteki adres** (IP ya da bilinen ad, ör. `192.168.1.1`, `tplinkap.net`), isteğe bağlı **Marka / model**
     (bilgi için; `ayarlar.toml [ap]`'de saklanır). Not: "Modemin ya da access point'in altındaki etikette yazan IP
     adresini yazın. Etiketteki kullanıcı adı ve parolayı buraya yazmayın."
   - **Bağlan** düğmesi: adres misafir ağındaysa (10.50.0.0/24, ör. kayıtlı AP 10.50.0.2) yan oda gerekmez; değilse cihaz
     aynı /24'ten boş bir yan oda adresi seçer (önce .250, doluysa .249 … — "boş" = yan odadan ARP/ICMP ile cevap yok,
     hedefin kendisi değil) ve yan odayı açar (mevcut 30 dk kuralları), sonra hedefe TCP 80 (yoksa 443) bağlantısını dener.
     Sonuç sayfada: "Cihaz bulundu (HTTP)" ya da "Bu adreste cihaz yok — kabloyu ve adresi kontrol edin".
   - **Otomatik bul** (etiket bilinmiyorsa): yaygın fabrika adreslerini sırayla dener (192.168.1.1, 192.168.0.1,
     192.168.0.254, 192.168.1.254, 192.168.10.1, 192.168.2.1, 192.168.8.1, 192.168.100.1, 10.0.0.1, 10.0.0.138 …;
     liste kodda, uygulayıcı tamamlar), cevap veren ilk adreste yan odayı açık bırakır. Süre ~20 sn; arka plan işi olarak
     çalışır, sayfa kendini yeniler (JS yok).
   - Bilinen adlar tablosu (benim kararım, küçük): `tplinkap.net`, `tplinkrepeater.net` → 192.168.0.254;
     `tplinkwifi.net`, `tplinklogin.net` → 192.168.0.1; `routerlogin.net` → 192.168.1.1; `myrouter.local` → 192.168.1.1
     (uygulayıcı yaygın Türkiye modellerini ekleyebilir). Bilinmeyen ad → "IP adresini yazın".
2. Hedefe ulaşılınca **"AP arayüzüne git"** düğmesi → yeni sekmede **https://ap.wificorrect.com/** açılır; içinde
   doğrudan AP'nin kendi web arayüzü (giriş ekranı dahil). Kayıtlı AP adresleri ([ap] adresler) için de aynı düğme
   (yan odasız).
3. Erişim **30 dk** sürer (yan odayla aynı; misafir ağındaki hedef için de 30 dk); süre dolunca ya da "Kapat" ile biter.

## Mimari (sözleşme — merkez ve cihaz bu şekilde konuşur)

**Neden ayrı alt alan adı:** AP arayüzü kökte (`/`) açıldığı için AP'nin mutlak yolları (`/userRpm/...`, `/cgi-bin/...`)
değişmeden çalışır; yol önekli (/ap/...) aktarım çoğu AP'yi bozar.

1. **DNS:** `ap.wificorrect.com` A → merkezin dış IP'si (kullanıcı ekler). Caddy sertifikayı kendi alır.
2. **Geçiş belirteci:** Panelde "AP arayüzüne git" → cihaz hedefi doğrular ve **hedef durumunu** yazar
   (`/run/wificorrect/ap-hedef.json`: adres, port, şema http|https, yol yan_oda|misafir, bitiş, kim/rol) → tarayıcıyı
   `https://cihaz.wificorrect.com/_ap-gecis` adresine yönlendirir (Caddy bu yolu forward_auth'suz merkeze verir).
   Merkez `wcc` çerezinden (cihaz paneli oturumu: müşteri ya da yönetici geçişi) kimi/hangi cihazı bulur, 60 sn'lik tek
   kullanımlık belirteç üretir → `https://ap.wificorrect.com/_ap-giris?t=…` → merkez belirteci `wca` çerezine çevirir
   (yalnızca ap.wificorrect.com, HttpOnly, Secure, SameSite=Lax, 30 dk; `wcc` oturumu geçersizleşince — parola, lisans,
   bağ — `wca` da geçersiz) → `/`'a yönlendirir. `/_ap-cikis` çerezi siler.
3. **Caddy `ap.wificorrect.com` bloğu:** gelen `X-WFC-*` başlıkları silinir; `/_ap-giris`, `/_ap-cikis` merkeze;
   diğer her şey `forward_auth` → merkez `/ap-yetki` (`wca` geçerliyse 200 + `X-WFC-Cihaz: <tünel IP>`,
   `X-WFC-Kullanici: <numara ya da yönetici>`, `X-WFC-AP: 1`) → `reverse_proxy {X-WFC-Cihaz}:8443` (cihaz sertifikası
   kendinden imzalı: `tls_insecure_skip_verify`, cihaz.wificorrect.com bloğuyla aynı). `wca` çerezi cihaza iletilmez
   (cihaz bloğundaki `wcc` temizliği gibi). Host başlığı `ap.wificorrect.com` kalır. Caddy 2.6.2 kısıtları (yer tutuculu
   üst akışta şema yazılamaz) KURULUM_GUNLUGU "Uzak panel"de.
4. **Cihaz (panel, :8443):** `X-WFC-AP: 1` başlığı yalnızca bugünkü güven kuralıyla (tünel `wfc` + merkez adresi,
   `X-WFC-Kullanici` gibi) kabul edilir; başka kaynaktan gelirse yok sayılır. AP kipindeki istek panel sayfası değil
   **aktarım**dır:
   - Hedef durumu yoksa/süresi dolmuşsa: sade bir Türkçe sayfa ("AP erişimi kapalı; cihaz panelinden yeniden açın").
   - Hedef: yalnızca **açık yan odanın alt ağı** ya da **kayıtlı AP adresleri** (misafir ağı); başka adres asla
     (misafir telefonlarına, cihazın kendisine, WAN'a, tünele aktarım yok). Yan oda hedefine bağlantı yan odadan
     (mevcut aracı ya da setns), misafir ağı hedefine ana ad alanından.
   - İstek: AP'ye **HTTP/1.0** (Connection: close; parçalı aktarımla uğraşmamak için), `Accept-Encoding: identity`;
     `Host`, `Origin`, `Referer` AP'nin kendi adresine (`http://192.168.1.1`) çevrilir; `X-WFC-*`, `X-Forwarded-*`,
     `wcc`/`wca` çerezleri iletilmez; gövde en çok birkaç MB.
   - Cevap: `Location` içindeki `http(s)://<hedef>` → `https://ap.wificorrect.com`; `Set-Cookie`'den `Domain=` çıkarılır;
     metin içerikte (HTML/JS/CSS, sıkıştırılmamış, boyut sınırlı) `http://<hedef>` ve `//<hedef>` geçenleri
     `https://ap.wificorrect.com` / `//ap.wificorrect.com` yapılır; diğer içerik olduğu gibi akar. HTTP Basic kimlik
     doğrulaması (eski TP-Link) olduğu gibi geçer. Zaman aşımları, eşzamanlı bağlantı sınırı.
   - Denetim: `AP_ARAYUZ acildi hedef=… yol=… sure=…` (kim + rol), `AP_ARAYUZ kapandi neden=…`; tek tek istekler yazılmaz.
5. **Bilinen sınır:** arayüzünde kendi adresini JS ile kuran (ör. `location.host` yerine sabit IP birleştiren, yeniden
   yazmanın yakalayamadığı) modellerde bazı sayfalar bozulabilir; yedek yol `ap-ac` betiği.

## Aynı sürüme (0.4.15) giren küçükler

- **Panel sertifikası:** `/etc/wificorrect/panel.{crt,key}` boş ya da okunamıyorsa panel yeniden üretir; üretim geçici
  dosya + atomik taşıma (yarım dosya kalmaz). (Visavis cihazında kurulumda 0 baytlık sertifika oluştu, panel açılamadı.)
- **Monitörsüz açılış denemesi:** bu J1900 kutularında (AMI Aptio CRB, BIOS 2015) NVRAM'de hiç önyükleme kaydı yok
  (`efibootmgr`: "No BootOrder is set"); monitör takılı değilken firmware yerleşik UEFI Shell'e düşüyor. EFI bölümüne
  `startup.nsh` (+ `\EFI\BOOT\startup.nsh`) ve işaret `\EFI\wificorrect\kimlik`: Shell işareti taşıyan fs'yi bulup
  `\EFI\BOOT\BOOTX64.EFI`'yi başlatır. Paket (postinst, /boot/efi bağlıysa) ve ISO (`son.sh`) yazar. **Etkisi gerçek
  cihazda doğrulanmadı** (ilk deneme monitörle yapıldı); zararsız: monitör varken Shell'e hiç düşülmüyor.
- Portal metinleri sayfasındaki örnek ("İŞLETMECİ’nin → Göztepe Bilgisayar’nin") gerçek ek hesabıyla yazılsın.
- `ap-ac.ps1`: müşteri başına ayar gerekmesin diye `-Tunel <10.99.0.x>` (yönetimde müşteri sayfasında görünen tünel
  adresi) + merkez üzerinden atlama (`-Merkez gbserver@192.168.1.109`, anahtar `~/.ssh/gbserver`; SSH ProxyJump/ProxyCommand),
  cihaz ana makine anahtarı tünel IP'si yeniden kullanılabildiği için `HostKeyAlias`/ayrı known_hosts ile karışmasın.

## Hedef dışı

- AP'yi programla ayarlamak (markaya özel sürücü).
- AP aktarımında HTTP dışı protokoller (telnet/ssh); WebSocket (gerekirse sonra).
- Etiketteki fabrika kullanıcı adı/parolasını saklamak.

## Anti-hedefler

- **Açık röle:** ap.wificorrect.com yalnızca oturumu olan müşteri/yöneticiye ve yalnızca izinli hedefe aktarır;
  misafir cihazlarına, cihazın kendisine, WAN/tünel ağlarına asla.
- **Misafir ağını bozmak:** yan oda kuralları aynen (ana ağ ad alanına adres/rota eklenmez).
- **Oturum karışması:** bir müşterinin `wca` çerezi başka müşterinin cihazına gidemez (merkez cihazı çerezden bulur).

## Kurulum sırası

1. Kullanıcı DNS'e `ap.wificorrect.com` ekler. 2. Merkez (`merkez-guncelle.sh`; Caddyfile doğrulanır). 3. Cihazlar 0.4.15
(uzaktan güncelleme). Merkez önce gelmezse düğme "AP erişimi şu an kullanılamıyor" der.
