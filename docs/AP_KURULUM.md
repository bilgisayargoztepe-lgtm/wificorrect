# Müşterinin AP'sini uzaktan kurmak (teknisyen rehberi)

Müşteri erişim noktasını (AP) kendisi alır, WifiCorrect cihazının **misafir portuna** (Ethernet 2) takar ve bize haber
verir. Biz AP'nin web arayüzüne cihaz üzerinden (SSH tüneli + SOCKS) gireriz; müşteri hiçbir ayar yapmaz. Panel bölümü
cihaz panelinde **Ayarlar → AP kurulumu**'dur; hizmet sağlayıcı ve işletme sahibi ikisi de kullanabilir (admin kilidi
gerekmez; 2026-10-15 kullanıcı kararı). Her işlem kim + rolle denetime yazılır. Windows betiği (3. bölüm) cihaza SSH
anahtarı gerektirdiği için AP arayüzüne girmek pratikte hizmet sağlayıcıdadır. Tasarım:
`docs/superpowers/specs/2026-10-15-ap-kurulum-design.md` (Ek: yan oda).

> **Kutudan yeni çıkmış AP'nin DHCP sunucusu çoğu zaman açıktır** (ZyXEL gibi modem/AP'lerde her zaman): misafirlere
> kendi adresini dağıtır, o misafirler giriş sayfasını göremez ve internete çıkamaz (5651 kaydı da dışında kalırlar).
> AP takılır takılmaz kurulmalı ve **ilk iş DHCP kapatılmalı**; mümkünse misafir yoğun saatte yapılmasın.

## 1. Müşteriden istenecekler

- AP'nin **marka ve modeli**,
- etiketteki **varsayılan adres ya da alan adı** (ör. `192.168.1.1`, `192.168.0.254`, `tplinkap.net`),
- etiketteki **fabrika kullanıcı adı ve parolası**,
- etiketteki **MAC adresi**.

## 2. AP'yi ağda bul

1. Cihaz panelini merkezden açın (Yönetim → müşteri → "Cihaz paneline git") → **Ayarlar** → **AP kurulumu**.
2. **Yabancı DHCP sunucusu var mı?** düğmesi: misafir portunda ~3 sn DHCP teklifi dinler. Kırmızı uyarı çıkarsa
   listedeki MAC müşterinin AP'sininkiyle aynı mı bakın; o AP'nin DHCP'si 4. bölümde **ilk iş** kapatılır.
3. **Ağdaki cihazları göster**: misafir portunda son görülen cihazlar (DHCP kiraları + ARP tablosu). Müşterinin verdiği
   MAC'i arayın. "rastgele MAC" notlu satırlar telefondur.
4. İki durum vardır:
   - **AP DHCP'den 10.50.0.x almış** (yeni modellerin çoğu AP modunda böyledir): o adresi kullanın, yan oda gerekmez.
   - **AP sabit fabrika adresinde** (ör. 192.168.1.1; listede görünmeyebilir — cihaz ona hiç paket göndermediyse):
     **Yan oda** açın: aynı alt ağdan, AP'nin kullanmadığı bir adres, ör. AP `192.168.1.1` ise `192.168.1.250/24`.
     Adres cihazın kendi ağına değil, misafir portuna bağlı ayrı bir ağ ad alanına ("yan oda") verilir: modemin ağıyla
     (ör. 192.168.1.0/24) aynı olsa da sorun olmaz, cihazın interneti, tüneli ve misafirler etkilenmez. Yan oda 30 dk
     sonra (ve cihaz yeniden başlayınca) kendiliğinden kapanır; erken bitirmek için **Şimdi kapat**. Misafir ağının
     içindeki (10.50.0.x), genel ya da /16'dan geniş adresler reddedilir.

## 3. AP arayüzünü kendi bilgisayarınızda açın

Repodaki betik SSH tüneli açar, ayrı (geçici profilli) bir Chrome penceresini SOCKS proxy ile ve AP'nin kendi adresiyle
açar; pencere kapanınca tünel kapanır ve geçici profil silinir. Normal Chrome profilinize dokunmaz.

```
scripts\ap-ac.cmd -Adres 10.50.0.2                              (dükkân ağındayken; ~/.ssh/config'teki "wificorrect")
scripts\ap-ac.cmd -Adres 192.168.1.1 -Cihaz root@10.99.0.11     (dışarıdan: WireGuard yönetici bağlantısı açık)
scripts\ap-ac.cmd -Adres tplinkap.net -Anahtar C:\Users\ben\.ssh\bocafe
scripts\ap-ac.cmd -Adres 192.168.1.1 -Kuru                      (yalnızca yöntemi ve komutları yazar)
```

- **Yöntemi betik seçer:** adres misafir ağındaysa (`-MisafirAg`, varsayılan `10.50.0.0/24`) ya da bir alan adıysa
  `ssh -D` (cihazın kendisi SOCKS proxy olur, ad çözümü cihazda). Misafir ağı dışında bir IPv4 ise (fabrika adresi)
  `ssh -L <yerel port>:127.0.0.1:1085` ile cihazdaki **yan oda aracısına** bağlanır; önce panelde yan oda açılmış
  olmalı, yoksa betik "Cihazda yan oda açık değil" der. Aracı yalnızca yan odanın alt ağındaki adreslere izin verir ve
  alan adı çözmez: fabrika adresli AP'ye **IP adresiyle** girin (AP arayüzü bir ada yönlendirirse adresi elle yazın).
- `-Port` varsayılan 1080; doluysa 1081–1099 arasından boş olanı seçer. `-AraciPort` (1085) panelde gösterilenle aynı.
- SSH parola sormaz (`BatchMode`); anahtar parolalıysa önce ssh-agent'a ekleyin. Hata olursa önce `ssh <cihaz>` elle denenir.
- Chrome yoksa Edge kullanılır. Windows PowerShell 5.1 ile çalışır.
- Adreste `&` varsa tırnak içinde yazın (`-Adres "http://10.50.0.2/?a=1&b=2"`); `.cmd` argümanları cmd'den geçirir.
- Tünel penceresi (ssh) simge durumunda açılır. Betiğin konsol penceresi X ile kapatılırsa tünel kapanmayabilir: o simge
  durumundaki ssh penceresini de kapatın (ya da Görev Yöneticisi'nden `ssh.exe`'yi sonlandırın).

### Örnek: fabrika ayarındaki ZyXEL (192.168.1.1, DHCP açık)

1. Panel → AP kurulumu → **Yabancı DHCP sunucusu var mı?** → kırmızı uyarıda `192.168.1.1` ve ZyXEL'in MAC'i görünür.
2. **Yan oda**: `192.168.1.250/24` → Aç.
3. `scripts\ap-ac.cmd -Adres 192.168.1.1` (dışarıdan `-Cihaz root@10.99.0.11`) → ZyXEL giriş sayfası açılır, etiketteki
   kullanıcı adı/parola ile girilir.
4. **İlk iş:** LAN / DHCP Setup → DHCP **None / Off** → Apply. (Sonra 4. bölümdeki sıra.)
5. Panelde denetimi tekrarlayın: "Yabancı DHCP sunucusu bulunmadı" görülmeli.
6. AP'yi `10.50.0.2/24` adresine aldıktan sonra betiği `-Adres 10.50.0.2` ile yeniden açın (artık misafir ağında:
   `ssh -D`); yan odayı **Şimdi kapat**.

## 4. AP ayarları (bu sırayla)

1. **DHCP sunucusu kapalı** — ilk iş; "Smart IP" / "Auto DHCP" gibi "gerekirse kendin dağıt" modları da kapalı.
2. **Çalışma modu:** Access Point (modem/router cihazlarda: köprü modu, WAN/PPPoE kapalı, LAN portları misafir portuna).
3. **Ağ adı (SSID):** işletmenin adı; **şifresiz**.
4. **İstemci yalıtımı (AP isolation / Client isolation)** açık.
5. **Kanal** 1, 6 ya da 11; **20 MHz**; **iletim gücü yüksek**.
6. **Yönetim parolası = işletme sahibinin panel parolası** (Yönetim → müşteri sayfasında görünür). Model en çok 15
   karakter kabul ediyorsa parola kısaltılır ve **kullanılan parola müşteri sayfasına not yazılır**.
   Dikkat: bu parola panel.wificorrect.com'da bütün 5651 kayıtlarını açar; ucuz AP'ler parolayı zayıf saklayabilir ya
   da şifresiz HTTP'den gönderebilir. Bu yüzden 8. adımdaki kablosuzdan yönetim kapatma özellikle önemlidir.
7. **Yönetim adresi:** sabit `10.50.0.2/24` (birden çok AP'de .3, .4 …; DHCP havuzu .20–.249 dışında), ağ geçidi
   ve DNS `10.50.0.1`. Kaydedince bağlantı kopar: betiği yeni adresle yeniden açın
   (`scripts\ap-ac.cmd -Adres 10.50.0.2`) ve ayarların kaldığını kontrol edin.
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

## 5. Bitirme

1. Panelde yan oda açıksa **Şimdi kapat**.
2. **Yabancı DHCP sunucusu var mı?** → "bulunmadı" olmalı.
3. **AP adresleri** kutusuna kalıcı yönetim adresini yazıp kaydedin (ör. `10.50.0.2`; en çok 8). Kayıt bilgi içindir:
   panelde ve ağdaki cihazlar listesinde "kayıtlı AP adresi" olarak görünür. Başka porttaki misafirlerden AP'ye giden
   trafik cihazın köprü yalıtımıyla zaten düşer; ayrı güvenlik duvarı kuralı yoktur.
4. Kontrol: telefonla SSID'ye bağlanın, giriş sayfası açılıyor mu; panelde "Ağdaki cihazlar"da AP `10.50.0.2` görünüyor mu.

## Bilinen tuzaklar

- **TP-Link + `ssh -L` doğrudan AP'ye (127.0.0.1 port yönlendirmesi):** giriş Referer/Host yüzünden reddedilir. Betik
  hiçbir yöntemde bunu yapmaz: Chrome her zaman SOCKS ile AP'nin kendi adresini açar (`ssh -L` yalnızca cihazdaki
  aracıya gider).
- **Fabrika modunda DHCP sunucusu açık AP** misafirlere kendi adresini dağıtır → takar takmaz kurulmalı, ilk iş DHCP.
- **Yan odadan alan adı açılmaz** (AP arayüzü `http://zyxel.home/` gibi bir ada yönlendirirse adresi IP ile yazın).
- **Port değişikliği** (panel → Portlar) köprüyü yeniden kurarsa yan odanın bağı kopar; yan odayı kapatıp yeniden açın.
- AP'nin parolası müşteri parolası değişince kendiliğinden değişmez; gerekirse elle güncellenir.
