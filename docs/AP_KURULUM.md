# Müşterinin AP'sini uzaktan kurmak (teknisyen rehberi)

Müşteri erişim noktasını (AP) kendisi alır, WifiCorrect cihazının **misafir portuna** (Ethernet 2) takar ve bize haber
verir. Biz AP'nin web arayüzüne cihaz üzerinden (SSH SOCKS tüneli) gireriz; müşteri hiçbir ayar yapmaz. Bu iş yalnızca
hizmet sağlayıcıdadır: cihaz panelinde **Admin → AP kurulumu** (admin kilidi açık olmalı). Tasarım:
`docs/superpowers/specs/2026-10-15-ap-kurulum-design.md`.

> Fabrika ayarındaki AP'nin DHCP sunucusu açık olabilir ve misafirlere yanlış adres dağıtabilir. AP takılır takılmaz
> kurulmalı; kurulum bitene kadar mümkünse misafir yoğun saatte yapılmasın.

## 1. Müşteriden istenecekler

- AP'nin **marka ve modeli**,
- etiketteki **varsayılan adres ya da alan adı** (ör. `192.168.0.254`, `tplinkap.net`),
- etiketteki **fabrika kullanıcı adı ve parolası**,
- etiketteki **MAC adresi**.

## 2. AP'yi ağda bul

1. Cihaz panelini merkezden açın (Yönetim → müşteri → "Cihaz paneline git") → **Admin** → **AP kurulumu**.
2. **Ağdaki cihazları göster**: misafir portunda son görülen cihazlar (DHCP kiraları + ARP tablosu). Müşterinin verdiği
   MAC'i arayın. "rastgele MAC" notlu satırlar telefondur.
3. İki durum vardır:
   - **AP DHCP'den 10.50.0.x almış** (yeni modellerin çoğu AP modunda böyledir): o adresi kullanın, geçici adres gerekmez.
   - **AP sabit fabrika adresinde** (ör. 192.168.0.254; listede görünmeyebilir — cihaz ona hiç paket göndermediyse):
     **Geçici adres** ekleyin: aynı alt ağdan, AP'nin kullanmadığı bir adres, ör. `192.168.0.250/24`. Adres 30 dk sonra
     (ve cihaz yeniden başlayınca) kendiliğinden kalkar; misafirler etkilenmez, misafirden bu alt ağa erişim kapalıdır.
     Panel çakışan adresi reddeder: AP'nin fabrika adresi **modemin ağındaysa** (ör. modem 192.168.1.1, AP 192.168.1.254)
     geçici adres eklenemez — müşteriden AP'yi sıfırlayıp DHCP'ye almasını isteyin.

## 3. AP arayüzünü kendi bilgisayarınızda açın

Repodaki betik SSH SOCKS tüneli açar, ayrı (geçici profilli) bir Chrome penceresini bu tünelle açar; pencere kapanınca
tünel kapanır ve geçici profil silinir. Normal Chrome profilinize dokunmaz.

```
scripts\ap-ac.cmd -Adres 10.50.0.2                              (dükkân ağındayken; ~/.ssh/config'teki "wificorrect")
scripts\ap-ac.cmd -Adres 192.168.0.254 -Cihaz root@10.99.0.11   (dışarıdan: WireGuard yönetici bağlantısı açık)
scripts\ap-ac.cmd -Adres tplinkap.net -Anahtar C:\Users\ben\.ssh\bocafe
scripts\ap-ac.cmd -Adres 10.50.0.2 -Kuru                        (yalnızca komutları yazar)
```

- `-Port` varsayılan 1080; doluysa 1081–1099 arasından boş olanı seçer.
- SSH parola sormaz (`BatchMode`); anahtar parolalıysa önce ssh-agent'a ekleyin. Hata olursa önce `ssh <cihaz>` elle denenir.
- Alan adları (ör. `tplinkap.net`) SOCKS5 üzerinden cihazda çözülür; ek ayar gerekmez.
- Chrome yoksa Edge kullanılır. Windows PowerShell 5.1 ile çalışır.

## 4. AP ayarları (bu sırayla)

1. **Çalışma modu:** Access Point.
2. **DHCP sunucusu kapalı**; "Smart IP" / "Auto DHCP" gibi "gerekirse kendin dağıt" modları da kapalı.
3. **Ağ adı (SSID):** işletmenin adı; **şifresiz**.
4. **İstemci yalıtımı (AP isolation / Client isolation)** açık.
5. **Kanal** 1, 6 ya da 11; **20 MHz**; **iletim gücü yüksek**.
6. **Yönetim parolası = işletme sahibinin panel parolası** (Yönetim → müşteri sayfasında görünür). Model en çok 15
   karakter kabul ediyorsa parola kısaltılır ve **kullanılan parola müşteri sayfasına not yazılır**.
7. **Yönetim erişimi kısıtı** (ağ şifresiz; misafir AP'ye kablosuz bağlıyken AP'nin kendi sayfasına giden trafiği
   cihazımızdan hiç geçmez, onu yalnızca AP korur):
   - modelde varsa **"kablosuzdan yönetim" kapalı** ("Wireless management", "Management via wireless", "Web access
     from wireless" …), ya da
   - **"yönetime yalnızca şu IP/MAC izin ver"** = `10.50.0.1` / panelde AP kurulumu → "Cihazın misafir portu"ndaki MAC.
   - İkisi de yoksa koruma yalnızca parolaya kalır; bunu müşteri sayfasına not edin.
8. **En son yönetim adresi:** sabit `10.50.0.2/24` (birden çok AP'de .3, .4 …; DHCP havuzu .20–.249 dışında), ağ geçidi
   ve DNS `10.50.0.1`. Kaydedince bağlantı kopar: betiği yeni adresle yeniden açıp ayarların kaldığını kontrol edin.

## 5. Bitirme

1. Panelde geçici adres açıksa **Şimdi kaldır**.
2. **AP adresleri** kutusuna kalıcı yönetim adresini yazıp kaydedin (ör. `10.50.0.2`; en çok 8). Kayıt bilgi içindir:
   panelde ve ağdaki cihazlar listesinde "kayıtlı AP adresi" olarak görünür. Başka porttaki misafirlerden AP'ye giden
   trafik cihazın köprü yalıtımıyla zaten düşer; ayrı güvenlik duvarı kuralı yoktur.
3. Kontrol: telefonla SSID'ye bağlanın, giriş sayfası açılıyor mu; panelde "Ağdaki cihazlar"da AP `10.50.0.2` görünüyor mu.

## Bilinen tuzaklar

- **TP-Link + `ssh -L` (127.0.0.1 port yönlendirmesi):** giriş Referer/Host yüzünden reddedilir. SOCKS yönteminde
  (bu betik) olmaz.
- **Fabrika modunda DHCP sunucusu açık AP** misafirlere kendi adresini dağıtabilir → takar takmaz kurulmalı.
- **Port değişikliği** (panel → Portlar) ağı yeniden kurar ve geçici adresi siler; panel "artık yok" der, yeniden ekleyin.
- AP'nin parolası müşteri parolası değişince kendiliğinden değişmez; gerekirse elle güncellenir.
