# Müşterinin AP'sini uzaktan kurmak (teknisyen rehberi)

Müşteri erişim noktasını (AP) ya da modemi kendisi alır, WifiCorrect cihazının **misafir portuna** (Ethernet 2) takar ve
bize haber verir. Biz AP'nin web arayüzüne **cihaz panelinden** gireriz: arayüz yeni sekmede
**https://ap.wificorrect.com/** adresinde açılır, bilgisayarda ayar ya da SSH gerekmez; müşteri hiçbir ayar yapmaz.
Panel bölümü cihaz panelinde **Ayarlar → AP kurulumu**'dur; hizmet sağlayıcı ve işletme sahibi ikisi de kullanabilir
(admin kilidi gerekmez; 2026-10-15 kullanıcı kararı). Her işlem kim + rolle denetime yazılır. Panelden açılmayan
modellerde yedek yol Windows betiğidir (4. bölüm). Tasarım: `docs/superpowers/specs/2026-10-16-ap-arayuz-design.md`
(önceki: `2026-10-15-ap-kurulum-design.md`, yan oda).

> **Kutudan yeni çıkmış AP'nin DHCP sunucusu çoğu zaman açıktır** (ZyXEL gibi modem/AP'lerde her zaman): misafirlere
> kendi adresini dağıtır, o misafirler giriş sayfasını göremez ve internete çıkamaz (5651 kaydı da dışında kalırlar).
> AP takılır takılmaz kurulmalı ve **ilk iş DHCP kapatılmalı**; mümkünse misafir yoğun saatte yapılmasın.

> **AP arayüzünü her müşteri için yeni bir gizli (incognito) pencerede açın; işiniz bitince bütün gizli pencereleri
> kapatın.** Bütün müşterilerin AP'leri aynı adreste (ap.wificorrect.com) açılır: tarayıcının bu adres için sakladığı AP
> çerezleri ve AP parolası (HTTP Basic) başka müşterinin AP'sine gidebilir. Gizli pencerede yönetime (ya da
> panel.wificorrect.com'a) girip oradan cihaz paneline geçin; pencereler kapanınca hepsi silinir. Merkez ve cihaz ayrıca
> korur (yeni AP oturumunda eski AP çerezleri ve depolaması silinir, Service Worker kaydı engellenir), ama tarayıcıda kalan
> HTTP Basic parolasını yalnızca pencereyi kapatmak siler.

## 1. Müşteriden istenecekler

- AP'nin **marka ve modeli**,
- etiketteki **varsayılan adres ya da alan adı** (ör. `192.168.1.1`, `192.168.0.254`, `tplinkap.net`),
- etiketteki **fabrika kullanıcı adı ve parolası** (AP'nin kendi giriş ekranında kullanılır; panele yazılmaz),
- etiketteki **MAC adresi**.

## 2. AP'yi bul ve arayüzünü aç (panel)

1. Cihaz panelini açın (müşteri panel.wificorrect.com'dan; biz Yönetim → müşteri → "Cihaz paneline git") →
   **Ayarlar** → **AP kurulumu** → **Takılan cihaz**.
2. **Etiketteki adres**: modemin ya da AP'nin altındaki etikette yazan IP adresi ya da bilinen ad (`tplinkap.net`,
   `tplinkwifi.net`, `routerlogin.net` … IP'ye çevrilir). İsteğe bağlı **Marka / model** (bilgi için). Etiketteki
   kullanıcı adını ve parolayı buraya **yazmayın**. **Bağlan**:
   - Adres misafir ağındaysa (10.50.0.x) yan oda gerekmez, cihaz doğrudan dener.
   - Değilse cihaz aynı ağdan boş bir adresle (önce .250, doluysa .249 …) 30 dk'lık **yan oda** açar (misafir portuna
     bağlı ayrı bir ağ; modemin ağıyla aynı olsa da cihazın interneti, tüneli ve misafirler etkilenmez) ve AP'nin web
     arayüzüne bağlanmayı dener: 443'te HTTPS çalışıyorsa HTTPS, yoksa HTTP (80). Sayfa birkaç saniye kendini yeniler;
     sonuç bölümde görünür: **"Cihaz bulundu (HTTPS)"** / **"(HTTP)"** ya da **"Bu adreste cihaz yok — kabloyu ve adresi
     kontrol edin"**.
   - Etiket okunmuyorsa **Otomatik bul**: yaygın fabrika adreslerini (192.168.1.1, 192.168.0.1, 192.168.0.254,
     192.168.1.254, 192.168.2.254 AirTies, 192.168.8.1 / 192.168.100.1 Huawei, 192.168.88.1 MikroTik …) tarar; cevap
     veren ilk cihazda yan oda açık kalır.
3. **AP arayüzüne git**: yeni sekmede ap.wificorrect.com açılır, içinde AP'nin kendi giriş ekranı (HTTPS çalışıyorsa
   AP'ye HTTPS'ten gidilir: bazı modemler, ör. ZyXEL, ancak böyle giriş yapar; HTTPS arayüzü düzgün çalışmayan cihazda
   yanındaki **HTTP ile aç**). Etiketteki fabrika kullanıcı adı/parolasıyla girin. Erişim **30 dk** sürer (yan odada en
   çok yan odanın kalan süresi); bölümde **Erişimi kapat** ile erken biter. Süre dolunca ap.wificorrect.com "AP erişimi kapalı" der: panelden yeniden açın
   (aynı sayfadaki "AP oturumunu kapat ve bu tarayıcıdaki AP verisini sil" bağlantısı tarayıcıdaki AP çerezlerini ve
   depolamayı da siler, saklanan AP parolasını da çoğu tarayıcıda siler — tarayıcıya bağlı; wificorrect.com'daki
   panel/yönetim oturumları da kapanır).
   Yan oda açıkken o ağdaki başka bir adrese de gidilebilir (Yan oda bölümündeki "AP'nin adresi" formu).
4. **Yabancı DHCP sunucusu var mı?** düğmesi: misafir portunda ~3 sn DHCP teklifi dinler. Kırmızı uyarı çıkarsa
   listedeki MAC müşterinin AP'sininkiyle aynı mı bakın; o AP'nin DHCP'si 3. bölümde **ilk iş** kapatılır.
5. AP DHCP'den 10.50.0.x almışsa (yeni modellerin çoğu AP modunda böyledir) **Ağdaki cihazları göster**'den adresini
   bulun (müşterinin verdiği MAC; "rastgele MAC" notlu satırlar telefondur). Havuzdaki (.20–.249) adrese panelden
   gidilmez — misafir telefonlarının adresleri; AP'ye ya betikle girip (4. bölüm) sabit adrese alın ya da AP'yi
   sıfırlatıp fabrika adresinden "Bağlan" deyin.

## 3. AP ayarları (bu sırayla)

1. **DHCP sunucusu kapalı** — ilk iş; "Smart IP" / "Auto DHCP" gibi "gerekirse kendin dağıt" modları da kapalı.
2. **Çalışma modu:** Access Point (modem/router cihazlarda: köprü modu, WAN/PPPoE kapalı, LAN portları misafir portuna).
3. **Ağ adı (SSID):** işletmenin adı; **şifresiz**.
4. **İstemci yalıtımı (AP isolation / Client isolation)** açık.
5. **Kanal** 1, 6 ya da 11; **20 MHz**; **iletim gücü yüksek**.
6. **Yönetim parolası = AP yönetim parolası** (merkezin müşteri başına ürettiği 12 karakterlik rastgele parola; Yönetim →
   müşteri sayfasında "AP yönetim parolası", cihaz panelinde AP kurulumu → Takılan cihaz'da da görünür). **İşletme
   sahibinin panel parolasını AP'ye yazmayın:** o parola panel.wificorrect.com'da bütün 5651 kayıtlarını açar; bütün
   AP'ler aynı adreste açıldığı için tarayıcının sakladığı AP parolası başka müşterinin AP'sine gidebilir, ucuz AP'ler
   parolayı zayıf saklayabilir ya da şifresiz HTTP'den gönderebilir. AP parolası yönetimde **Yenile** ile değişirse AP'de
   elle değiştirilir (cihaza 5 dk içinde ulaşır ve cihaz panelinde görünür; 0.4.17'den eski cihazlarda sonraki eşitlemeden
   — her gün 06:00 — sonra). Aynı parola cihazın giriş sayfasındaki **Servis** için de anahtardır (aşağıda, 5. bölüm).
7. **Yönetim adresi:** sabit `10.50.0.2/24` (birden çok AP'de .3, .4 …; DHCP havuzu .20–.249 dışında), ağ geçidi
   ve DNS `10.50.0.1`. Kaydedince bağlantı kopar: panelde **AP adresleri** kutusuna `10.50.0.2` yazıp **AP açıkken**
   kaydedin — cihaz o anki MAC'i (ör. `10.50.0.2 (MAC 70:4f:57:dd:7a:e8)`) sabitler ve panel bu adrese yalnızca bu
   MAC'teki cihaz için gider (misafir ağında adresi elle alan biri AP yerine geçip yazılan parolayı toplamasın). "MAC
   adresi okunamadı" derse AP'nin açık ve misafir portuna bağlı olduğunu kontrol edip yeniden Kaydet'e basın. Sonra
   kutunun altındaki **10.50.0.2 arayüzüne git** ile yeniden girin (yan odasız, misafir ağından) ve ayarların kaldığını
   kontrol edin. Yan odayı **Şimdi kapat**. AP değiştirilirse (yeni cihaz) panel "AP'nin MAC adresi değişmiş" der: aynı
   adresi yeniden Kaydet'e basarak yeni MAC'i sabitleyin.
8. **Yönetim erişimi kısıtı — en son, AP 10.50.0.2'deyken ve bu adresten yeniden girdikten sonra** (ağ şifresiz;
   misafir AP'ye kablosuz bağlıyken AP'nin kendi sayfasına giden trafiği cihazımızdan hiç geçmez, onu yalnızca AP korur):
   - modelde varsa **"kablosuzdan yönetim" kapalı** ("Wireless management", "Management via wireless", "Web access
     from wireless" …), ya da
   - **"yönetime yalnızca şu IP/MAC izin ver"** = `10.50.0.1` / panelde AP kurulumu → "Cihazın misafir portu"ndaki MAC.
   - İkisi de yoksa koruma yalnızca parolaya kalır; bunu müşteri sayfasına not edin.

   **Neden bu sıra:** AP fabrika adresindeyken (ör. 192.168.1.1) ona yan odadan gireriz; AP isteği `10.50.0.1`'den değil
   yan odanın adresinden (ör. 192.168.1.250) ve yan odanın kendi (rastgele) MAC'inden görür. IP ya da MAC kısıtı önce
   yazılırsa çoğu modelde kaydedince hemen geçerli olur, oturum kesilir, adres değişikliği (7. adım) yapılamaz ve AP'ye
   cihazdan bir daha ulaşılamaz — müşteri AP'yi yerinde sıfırlamak zorunda kalır. AP 10.50.0.2'ye alınıp yeni adresten
   girildikten sonra istekler gerçekten `10.50.0.1`'den ve köprünün MAC'inden gelir ve kısıt güvenlidir.

### Örnek: fabrika ayarındaki ZyXEL (192.168.1.1, DHCP açık)

1. Panel → AP kurulumu → **Yabancı DHCP sunucusu var mı?** → kırmızı uyarıda `192.168.1.1` ve ZyXEL'in MAC'i görünür.
2. **Takılan cihaz**: etiketteki adres `192.168.1.1`, marka `ZyXEL P-660HN-T1H` → **Bağlan** → "Cihaz bulundu (HTTP):
   192.168.1.1 — yan oda 192.168.1.250/24" (telefonlardan biri ZyXEL'den .250 aldıysa .249 …).
3. **AP arayüzüne git** → ZyXEL giriş sayfası açılır, etiketteki kullanıcı adı/parola ile girilir.
4. **İlk iş:** LAN / DHCP Setup → DHCP **None / Off** → Apply. (Sonra 3. bölümdeki sıra.)
5. Panelde denetimi tekrarlayın: "Yabancı DHCP sunucusu bulunmadı" görülmeli.
6. AP'yi `10.50.0.2/24` adresine aldıktan sonra **AP adresleri**'ne `10.50.0.2` kaydedip (AP açıkken; MAC sabitlenir)
   **10.50.0.2 arayüzüne git**; yan odayı **Şimdi kapat**. AP'nin yönetim parolası: AP yönetim parolası (3.6).

## 4. Yedek yol: AP arayüzünü kendi bilgisayarınızda açın (betik)

Panelden açılmayan modellerde (aşağıdaki "Bilinen tuzaklar") ya da AP'nin başka bir portu / büyük dosya yükleme
gerektiğinde. Betik SSH tüneli açar, ayrı (geçici profilli) bir Chrome penceresini SOCKS proxy ile ve AP'nin kendi
adresiyle açar; pencere kapanınca tünel kapanır ve geçici profil silinir. Normal Chrome profilinize dokunmaz.

```
scripts\ap-ac.cmd -Adres 192.168.1.1 -Tunel 10.99.0.11          (müşteri başına ayar gerekmez; merkez üzerinden)
scripts\ap-ac.cmd -Adres 10.50.0.2                              (dükkân ağındayken; ~/.ssh/config'teki "wificorrect")
scripts\ap-ac.cmd -Adres 192.168.1.1 -Cihaz root@10.99.0.11     (dışarıdan: WireGuard yönetici bağlantısı açık)
scripts\ap-ac.cmd -Adres tplinkap.net -Anahtar C:\Users\ben\.ssh\bocafe
scripts\ap-ac.cmd -Adres 192.168.1.1 -Tunel 10.99.0.11 -Kuru     (yalnızca yöntemi, ssh ayarını ve komutları yazar)
```

- **-Tunel 10.99.0.x** (önerilen): yönetimde müşteri sayfasında görünen tünel adresi. Bağlantı merkez sunucu üzerinden
  atlanır (`-Merkez`, varsayılan `gbserver@192.168.1.109` — dükkân ağı; dışarıdan WireGuard açıkken
  `-Merkez gbserver@10.99.0.1`; merkez anahtarı `-MerkezAnahtar`, varsayılan `~/.ssh/gbserver`). Cihaz anahtarı
  `-Anahtar`, verilmezse `~/.ssh/bocafe`. Kullanıcının `~/.ssh/config`'i bu kipte okunmaz (geçici ayar dosyası).
  Cihazın ana makine anahtarı ayrı dosyada (`~/.ssh/known_hosts_wificorrect`) tünel adresine özel adla tutulur: ilk
  bağlantıda kaydedilir; tünel adresi müşteri değişince başka cihaza verilip anahtar değişirse bağlantı kurulmaz ve
  betik eski kaydı silen `ssh-keygen -R wificorrect-tunel-10.99.0.x -f …` satırını yazar (adres doğruysa çalıştırın).
- **Yöntemi betik seçer:** adres misafir ağındaysa (`-MisafirAg`, varsayılan `10.50.0.0/24`) ya da bir alan adıysa
  `ssh -D` (cihazın kendisi SOCKS proxy olur, ad çözümü cihazda). Misafir ağı dışında bir IPv4 ise (fabrika adresi)
  `ssh -L <yerel port>:127.0.0.1:1085` ile cihazdaki **yan oda aracısına** bağlanır; önce panelde yan oda açılmış
  olmalı ("Bağlan"), yoksa betik "Cihazda yan oda açık değil" der. Aracı yalnızca yan odanın alt ağındaki adreslere izin
  verir ve alan adı çözmez: fabrika adresli AP'ye **IP adresiyle** girin.
- `-Port` varsayılan 1080; doluysa 1081–1099 arasından boş olanı seçer. `-AraciPort` (1085) panelde gösterilenle aynı.
- SSH parola sormaz (`BatchMode`); anahtar parolalıysa önce ssh-agent'a ekleyin.
- Chrome yoksa Edge kullanılır. Windows PowerShell 5.1 ile çalışır.
- Adreste `&` varsa tırnak içinde yazın (`-Adres "http://10.50.0.2/?a=1&b=2"`); `.cmd` argümanları cmd'den geçirir.
- Tünel penceresi (ssh) simge durumunda açılır. Betiğin konsol penceresi X ile kapatılırsa tünel kapanmayabilir: o simge
  durumundaki ssh penceresini de kapatın (ya da Görev Yöneticisi'nden `ssh.exe`'yi sonlandırın).

## 5. Bitirme

1. Panelde yan oda açıksa **Şimdi kapat**; AP arayüzü erişimi açıksa **Erişimi kapat** (30 dk sonra kendiliğinden de
   kapanır). Bütün gizli pencereleri kapatın.
2. **Yabancı DHCP sunucusu var mı?** → "bulunmadı" olmalı.
3. **AP adresleri** kutusunda kalıcı yönetim adresi kayıtlı olsun (ör. `10.50.0.2`; en çok 8). Panelde ve ağdaki
   cihazlar listesinde "kayıtlı AP adresi" olarak görünür; sonraki bakımlarda yan odasız "arayüzüne git" düğmesi
   buradadır. Başka porttaki misafirlerden AP'ye giden trafik cihazın köprü yalıtımıyla zaten düşer; ayrı güvenlik
   duvarı kuralı yoktur.
4. Kontrol: telefonla SSID'ye bağlanın, giriş sayfası açılıyor mu; panelde "Ağdaki cihazlar"da AP `10.50.0.2` görünüyor mu.
5. **İnternet yoksa** (PPPoE'li hatta ONT'ye takılı cihaz bağlanamıyorsa ya da modem arızalıysa): telefonla misafir Wi-Fi'ına bağlanın
   ("internet yok" uyarısına rağmen bağlı kalın), tarayıcıya **`10.50.0.1:8080/servis`** yazın → **Servis** (parola: bu müşterinin
   AP yönetim parolası, 3.6). **PPPoE seçiminin Açık olduğundan emin olun** (Servis'te Açık hazır gelir; Kapalı seçerseniz PPPoE
   kapanır ve cihaz modemden adres bekler), PPPoE kullanıcı adını, parolayı ve gerekirse VLAN'ı girip **Kaydet ve bağlan**. Kayıttan sonra
   sayfa "Ayar kaydedildi; PPPoE yeniden bağlanıyor" der ("PPPoE kapalı" derse seçim Kapalı kaydedilmiştir: Ayarları değiştir bağlantısından
   Açık yapıp yeniden kaydedin), birkaç saniyede bir durumu gösterir ve internet gelince giriş sayfası açılır. Servis yalnızca cihazın interneti
   yokken açılır; yanlış parola 5 denemeden sonra 15 dakika kilitler. (Cihaz panelindeki Ayarlar → Portlar → PPPoE kutusu ise kayıtlı değeri
   gösterir: PPPoE hiç ayarlanmamış cihazda Kapalı gelir, orada Açık'ı kendiniz seçin.) Ayrıntı: `docs/KURULUM_GUNLUGU.md` "PPPoE".

## Bilinen tuzaklar

- **ap.wificorrect.com'da bozulan sayfa:** arayüzünü JavaScript'le sabit adresten kuran modellerde (ör. `location`
  yerine `"http://" + ip` birleştiren) bazı sayfalar açılmaz; AP'nin yalnızca eski TLS'li (1.0/1.1) HTTPS arayüzü,
  WebSocket ve 8 MB'tan büyük dosya yükleme (ürün yazılımı) de panelden olmaz → 4. bölüm (betik).
- **ap.wificorrect.com bütün müşterilerin AP'leri için tek adres:** her müşteri için yeni gizli pencere (yukarıdaki not).
  Yeni AP oturumu (panelden "AP arayüzüne git") bu adresin depolamasını ve önceki AP'lerin çerezlerini siler; tarayıcının
  sakladığı HTTP Basic parolasını silemez. AP parolası müşteriye özel olduğu için sızsa da yalnızca o müşterinin AP'sini
  ilgilendirir.
- **"AP'nin MAC adresi değişmiş":** kayıtlı AP adresinde (ör. 10.50.0.2) sabitlenen MAC'ten başka bir cihaz cevap
  veriyor; istek gitmez, erişim kapanır (denetimde `AP_ARAYUZ … neden=mac`). AP değiştirildiyse AP adresleri'nde
  Kaydet'e basın; değiştirilmediyse "Ağdaki cihazlar"da o adresi kimin kullandığına bakın.
- **Aynı sitenin başka sayfasından ya da başka siteden AP'ye istek gitmez** (AP'ye yalnızca cihaz panelinden gelen
  gezinme ve AP'nin kendi sayfaları); Service Worker kullanan AP arayüzleri (çok nadir) panelden çalışmaz → betik.
- **"Farklı internet adresi":** "AP arayüzüne git" bağlantısı onu isteyen ağdan (aynı /24) açılmalı; iCloud Özel Geçiş
  (iPhone Safari) ya da VPN her bağlantıyı ayrı adresten çıkarabilir. Özel Geçiş'i/VPN'i kapatıp cihaz panelinden
  yeniden açın (iPhone: Ayarlar → adınız → iCloud → Özel Geçiş); bilgisayarda betik de olur.
- **Merkez henüz güncellenmediyse** "AP arayüzüne git" "AP erişimi şu an kullanılamıyor" der → betik.
- **TP-Link + `ssh -L` doğrudan AP'ye (127.0.0.1 port yönlendirmesi):** giriş Referer/Host yüzünden reddedilir. Betik
  hiçbir yöntemde bunu yapmaz: Chrome her zaman SOCKS ile AP'nin kendi adresini açar (`ssh -L` yalnızca cihazdaki
  aracıya gider). Panel aktarımı da Host/Origin/Referer'ı AP'nin kendi adresine çevirir.
- **Fabrika modunda DHCP sunucusu açık AP** misafirlere kendi adresini dağıtır → takar takmaz kurulmalı, ilk iş DHCP.
- **Yan odadan alan adı açılmaz** (AP arayüzü `http://zyxel.home/` gibi bir ada yönlendirirse adresi IP ile yazın).
- **Port değişikliği** (panel → Portlar) köprüyü yeniden kurarsa yan oda ve onun AP arayüzü erişimi kapanır; yeniden
  "Bağlan" deyin.
- AP yönetim parolası müşterinin panel parolasından bağımsızdır (panel parolası değişince AP'ye dokunulmaz); yönetimde
  yenilenirse AP'de elle güncellenir.
- **Servis'e yazılan AP yönetim parolası şifresiz Wi-Fi'dan (HTTP) gider:** yakındaki biri yakalayabilir. Servis'i kullandıktan sonra
  yönetimde bu müşterinin AP yönetim parolasını **Yenile** deyip AP'ye de elle yazın (cihaz çevrimdışıyken yenilerseniz cihaz yeniden
  bağlanana kadar eski parolayı kullanır).
