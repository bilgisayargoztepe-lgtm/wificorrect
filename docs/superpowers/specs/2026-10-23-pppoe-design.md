# PPPoE ile internet — tasarım (2026-10-23)

## Amaç ve neden
Bazı işletmelerde araya modem girmeyecek: WifiCorrect'in internet alan portuna doğrudan **ONT'den** ya da **binanın
merkezinden gelen Ethernet kablosu** takılacak. Bu hatlar adresi DHCP ile değil **PPPoE** ile (operatörün verdiği kullanıcı adı
ve parolayla) verir. Bugün cihazın internet alan portu yalnızca DHCP biliyor; bu hatlarda internete çıkamıyor.

İstenen (kullanıcı, 2026-10-23): **yalnızca internet alan Ethernet portu PPPoE tanısın, başka bir şeye gerek yok.**

## Hedefler
- İnternet alan port, panelde girilen PPPoE bilgileriyle bağlanır; hat düşerse kendiliğinden yeniden bağlanır.
- PPPoE bilgileri **dükkânda, cihaz modemle (DHCP) çalışırken panelden** girilir. Cihaz dükkânda DHCP ile çalışmaya devam eder;
  müşteride ONT'ye / bina kablosuna takılınca kendiliğinden PPPoE ile bağlanır. Aynı cihaz iki hatta da ayar değiştirmeden çalışır.
- PPPoE bağlanamazsa (parola yanlış, VLAN gerekli…) teknisyen **müşteride, internet ve SMS olmadan** sorunu görüp düzeltebilir:
  telefonla misafir Wi-Fi'ına bağlanır, giriş sayfasındaki **Servis** bölümüne **müşteriye özel AP yönetim parolasıyla** girer.
- PPPoE varsayılan kapalı. PPPoE kapalı cihazlarda tek görünür fark: **internet yokken** giriş sayfasındaki uyarı ve Servis
  bağlantısı (dükkânda hazırlanmadan PPPoE hattına gelen cihaz da yerinde açılabilsin; modem arızasında misafir de nedeni görsün).

## Kapsam dışı / anti-hedefler
- İşletme ağı, personel Wi-Fi'ı, kasa/kamera için port: **yok** (kullanıcı: işletmenin kendi cihazı olmayacak).
- ONT'nin yerine geçmek (SFP/GPON): yok. IPv6 (PPPoE üzerinden): yok, kapalı.
- Kurulum ISO'sunda / ilk açılışta PPPoE sormak: yok (cihazlar dükkânda DHCP ile kurulup bağlanır).
- **Anti-hedef:** misafire açık giriş sayfasında, internet varken herhangi bir ayar ya da yönetim yüzeyi açmak. Servis bölümü
  yalnızca cihazın interneti yokken görünür ve yalnızca PPPoE bilgilerini değiştirir.
- **Anti-hedef:** bütün cihazlarda ortak olan admin parolasının şifresiz (HTTP) ağdan yazılması. Servis için admin parolası
  kullanılmaz.

## Kullanım akışı
1. **Dükkânda hazırlık:** cihaz ISO'dan kurulur, modeme takılı (DHCP), müşteriye bağlanır (kolay bağlama). Panel → Portlar →
   İnternet alan port → **PPPoE: Açık**, operatörün verdiği kullanıcı adı, parola, gerekirse VLAN → Uygula → 3 dk içinde Onayla
   (mevcut onay/geri alma akışı). Dükkânda PPPoE sunucusu olmadığı için durum "PPPoE sunucusu yanıt vermiyor" görünür; cihaz DHCP
   ile çalışmaya devam eder. Panelde bunu açıklayan not: "Dükkânda PPPoE bağlanmaz; ONT'ye takılınca bağlanır."
2. **Müşteride:** internet alan port ONT'ye / bina kablosuna takılır. PPPoE bağlanır, varsayılan rota PPPoE'den gider, tünel ve
   merkez bağlantısı gelir. Panelde durum "PPPoE bağlı (dış IP …)".
3. **Bağlanamazsa:** cihazın varsayılan rotası yoktur. Teknisyen telefonla misafir Wi-Fi'ına bağlanır ("internet yok" uyarısına
   rağmen bağlı kalır) ve tarayıcıya **`10.50.0.1:8080`** yazar (giriş sayfası). Sayfanın altında "Bu cihazın internet bağlantısı
   yok — <neden>" ve **Servis** bağlantısını görür. Servis sayfasında AP yönetim parolası + PPPoE kullanıcı adı, parola, VLAN girer
   → **Kaydet ve bağlan** → sayfa birkaç saniyede bir yenilenip durumu gösterir.
   - Neden adres elle yazılıyor: cihazın interneti yokken DNS hiçbir adı çözemez; telefonlar "ağda oturum açın" penceresini açmaz,
     yazılan alan adları da açılmaz. Yalnızca IP adresiyle girilir. (Değerlendirilip seçilmeyen yol: internet yokken DNS'in her adı
     yanıtlaması — her cihazda her kesintide bütün misafirlerin davranışını ve 5651 DNS kaydını değiştirirdi.)
   - Teknisyen notu ve cihaz etiketi: "İnternet yoksa: Wi-Fi'a bağlanın, tarayıcıya 10.50.0.1:8080 yazın → Servis."

## Ayar modeli
`/etc/wificorrect/ag.toml` (zaten 600, root) yeni bölüm:

```toml
[pppoe]
acik = false        # varsayılan kapalı (eski dosyada bölüm yoksa kapalı)
kullanici = ""      # tek alan: modemlerdeki "kullanıcı adı @ alan" birlikte yazılır (ör. 5321234567@turk.net)
parola = ""         # panelde yalnızca yazılır; boş gönderilirse eskisi kalır
vlan = 0            # 0 = VLAN yok; 1–4094 (Türk Telekom altyapısındaki hatlarda genelde 35)
oncelik = 0         # VLAN 802.1p önceliği 0–7 (yalnızca VLAN varken anlamlı)
mtu = 1492          # 1280–1492 (modemlerdeki "MRU")
kimlik = "otomatik" # otomatik | pap | chap
ac_adi = ""         # boş = herhangi bir erişim yoğunlaştırıcı
servis_adi = ""     # boş = herhangi bir servis
```

Doğrulama: açıkken kullanıcı adı ve parola boş olamaz; kullanıcı adı ve parola 1–128 bayt, kontrol karakteri, tırnak ve ters
eğik çizgi içermez (pppd dosyasına güvenle yazılabilsin); VLAN 0 ya da 1–4094; öncelik 0–7; MTU 1280–1492; kimlik üç değerden
biri; AC adı ve servis adı boş ya da en çok 64 karakter `A-Z a-z 0-9 . _ -`.

**Modemle karşılaştırma (2026-10-23, dükkândaki Aidata modemin "Ethernet WAN" sayfası):** orada kullanıcı adı (@ alan), şifre,
VLAN'ı etkinleştir + VLAN ID, 802.1p işareti, MRU (1280–1892), kimlik doğrulama yöntemi (AUTO), AC adı, servis adı, bağlantı
türü (Sürekli), varsayılan rota, NAT, IP protokolü, MAC klon, köprü, port eşleştirme var. İlk yedisi yukarıda. Bilerek
olmayanlar: bağlantı türü (her zaman sürekli), varsayılan rota ve NAT (her zaman açık), IP protokolü (yalnız IPv4), köprü ve
port eşleştirme (tek internet portu), MAC klon (Portlar'daki mevcut "internet portu MAC'i" ayarı), 1492'den büyük MRU
(RFC 4638 "baby jumbo" — gerekirse ileride).

## Cihazda bağlantı
- **DHCP her zaman açık kalır.** İnternet alan Ethernet'te dhcpcd bugünkü gibi çalışır (mevcut WAN DHCP bekçisi dahil). PPPoE
  açıksa yanında **pppd** çalışır (PPPoE çerçeveleri ile DHCP aynı kabloda birlikte yaşar). Hangisi bağlıysa o kullanılır; ikisi de
  bağlıysa **PPPoE tercih edilir** (PPPoE varsayılan rotası DHCP'ninkinden düşük metrikli; bugün DHCP rotası metrik 1002).
- **Yeni paket bağımlılığı:** `ppp` (Debian 13: 2.5.2, PPPoE eklentisi içinde; iki canlı cihazda Debian deposundan kurulabildiği
  doğrulandı). `paket/control.in` Depends'e eklenir.
- **pppd ayrı bir systemd biriminde** (ör. `wificorrect-pppoe.service`, `Restart=always`): yalnızca `pppoe.acik` iken etkin; geçici
  bir işin içinde başlatılmaz (2026-10-03 dersi: geçici işle başlatılan dhcpcd iş bitince öldü). Ayar değişince birim yeniden
  başlar. Bağlantı dosyası (ör. `/etc/ppp/peers/wificorrect`, 600) ag.toml'dan üretilir: PPPoE eklentisi + arayüz (VLAN varsa
  `<wan>.<vlan>`), sabit arayüz adı (ör. `wfcppp`), kullanıcı adı ve parola, varsayılan rota + metrik, operatör DNS'i, kopma
  algılama (LCP yankı), sınırsız yeniden deneme, IPv6 kapalı, MTU/MRU = `mtu`; kimlik `pap` ise CHAP/MS-CHAP/EAP reddedilir,
  `chap` ise PAP/EAP reddedilir; AC adı ve servis adı doluysa PPPoE eklentisinin ilgili seçenekleri. Sistemin ortak
  `chap-secrets`/`pap-secrets` dosyalarına dokunulmaz.
- **VLAN:** `vlan > 0` ise birim başlarken `<wan>.<vlan>` arayüzünü kurar (öncelik > 0 ise giden çerçevelere 802.1p önceliği
  `egress-qos-map 0:<oncelik>`), dururken kaldırır (ek paket gerekmez, `ip link`).
- **DNS:** PPPoE açıkken operatörün DNS'i kullanılır (Debian'ın ppp kancası `/etc/resolv.conf`'u yazar); dnsmasq yukarı akış olarak
  `/etc/resolv.conf`'u zaten okuyor. DHCP ile PPPoE aynı anda bağlıysa son yazan geçerli olur; ikisi de çalışan DNS'tir.
- **Tünel MTU'su:** PPPoE açıkken WireGuard yapılandırmasına `MTU = <mtu − 80>` (varsayılanda 1412) yazılır; kapalıyken bugünkü
  gibi (yazılmaz).

## Güvenlik duvarı, NAT, yavaşlatma
- `arayuzler.nft` bugün `define WAN = "<eth>"`. Yeni hali **küme**: `define WAN = { "<eth>", "wfcppp" }` (PPPoE kapalıyken de;
  olmayan arayüz adı eşleşmez, zararsız). Böylece `guvenlik.nft`'teki yönetim, iletim (`oifname $WAN … mark 0x5651`), masquerade
  ve `hiz.nft` sayaçları değişmeden iki arayüzü de kapsar.
- **Yönetim:** PPPoE'de cihaz genel IP alır; yönetim kuralı yalnızca özel kaynak adreslerini kabul ettiği için internetten panel
  ve SSH kapalı kalır (bugünkü "genel IP alırsa yönetim kapalı" davranışı). Yönetim tünelden.
- **MSS düzeltmesi:** iletilen TCP SYN'lerde MSS rota MTU'suna indirilir (PPPoE 1492; yapılmazsa bazı siteler açılmaz).
- **Yavaşlatma (HTB):** yükleme yönü bugün WAN Ethernet'ine kuruluyor. PPPoE bağlıyken trafiğin çıktığı arayüz `wfcppp`'dir;
  yavaşlatma **o an varsayılan rotanın arayüzüne** kurulur. `wfcppp` her yeniden bağlanmada yeniden doğduğu için kök kaybolur;
  kaydedicinin mevcut "kök yoksa yeniden kur" turu bunu düzeltir.
- **5651:** NAT kaydındaki `nat_ip`/`nat_port` artık doğrudan operatörün gördüğü adres olur (modemin ikinci NAT'ı yok). Şema aynı.

## Durum ve teşhis
Cihaz PPPoE durumunu şu değerlerle bilir (nasıl çıkarılacağı — pppd günlüğü, çıkış kodları, kanca dosyaları — kurucuya bırakıldı):

| Durum | Anlamı | Ekrandaki metin |
|---|---|---|
| `kapali` | PPPoE açık değil | PPPoE kapalı (modemden DHCP) |
| `kablo_yok` | internet portunda kablo yok | Kablo takılı değil |
| `dhcp_yok` | PPPoE kapalı ve DHCP'den adres alınamıyor | Modemden adres alınamadı (modem kapalı ya da kablo yanlış porta takılı) |
| `sunucu_yok` | PPPoE sunucusu yanıt vermiyor | PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?) |
| `kimlik_hatasi` | operatör kullanıcı adını/parolayı reddetti | Kullanıcı adı ya da parola reddedildi |
| `baglaniyor` | deneniyor | Bağlanıyor… |
| `bagli` | bağlı | PPPoE bağlı (dış IP …) |

- **Panel → Portlar:** internet alan portun altında durum satırı.
- **Sağlık raporu** (`/api/saglik`, 5 dk): `wan_tur` (`dhcp`|`pppoe`) ve `pppoe` (yukarıdaki durum) eklenir; merkez müşteri sayfasında
  cihaz kartında "İnternet: DHCP" / "İnternet: PPPoE bağlı" gösterir. Yeni sağlık kuralı yok: PPPoE bağlanamazsa cihaz zaten
  "ulaşılamıyor" olur ve mevcut uyarı SMS'i gider.

## Panel → Portlar
- İnternet alan portun satırının altında **PPPoE** kutusu: Açık/Kapalı, Kullanıcı adı (örnek: `5321234567@turk.net`), Parola
  (yalnızca yazılır; boş = aynı kalır), VLAN (boş = yok), durum satırı, dükkân notu; altta **Gelişmiş**: VLAN önceliği, MTU,
  kimlik doğrulama (Otomatik/PAP/CHAP), AC adı, servis adı (çoğu hatta boş bırakılır).
- Kaydetme **mevcut port değişikliği akışından** geçer (yeni ayar dosyası → geri alma zamanlayıcısı → geçiş → 3 dk içinde
  "Çalışıyor, onayla"; onaylanmazsa eski ayar döner). Böylece müşterideyken PPPoE bilgisini uzaktan yanlış değiştirmek cihazı
  kalıcı olarak koparmaz.
- Denetim: `PANEL_PORT` satırına `pppoe=acik|kapali vlan=… kullanici_degisti=0|1 parola_degisti=0|1` (kullanıcı adı ve parola
  yazılmaz).

## Giriş sayfası: internet yokken uyarı ve Servis
- **Koşul:** cihazın varsayılan rotası yoksa (ne DHCP ne PPPoE) — **PPPoE açık da kapalı da olsa** (bütün cihazlarda). İnternet
  varken giriş sayfası bugünkü gibidir; Servis bölümü görünmez ve `/servis` adresi giriş sayfasına yönlendirir.
- **Uyarı (herkese):** sayfanın altında "Bu cihazın internet bağlantısı şu an yok" + kısa neden (yukarıdaki durum metni) +
  **Servis** bağlantısı. Misafir giriş akışı (SMS) bu durumda zaten çalışmaz; mevcut hata metinleri aynen kalır.
- **Servis sayfası** (`/servis`, giriş sayfasının kendi sunucusunda, Türkçe): durum satırı + **tek form**: Servis parolası,
  PPPoE Açık/Kapalı, kullanıcı adı, parola, VLAN ve paneldeki Gelişmiş alanların aynısı → **Kaydet ve bağlan**. Parola her gönderimde birlikte yazılır (oturum,
  çerez yok). Kaydedince ag.toml güncellenir, bağlantı dosyası yeniden üretilir, PPPoE birimi yeniden başlar; **geri alma
  zamanlayıcısı kurulmaz** (cihaz zaten çevrimdışı; geri almak yanlış eski bilgiye döndürürdü). Sayfa birkaç saniyede bir kendini
  yenileyip (meta, JS yok) durumu gösterir.
- **Servis parolası = müşteriye özel AP yönetim parolası** (`merkez.json` → `ap_parola`; yönetimde ve cihaz panelinde görünür).
  Merkeze sormadan cihazda doğrulanır (sabit sürede karşılaştırma). `ap_parola` yoksa (cihaz bağlanmamış ya da eski merkez):
  "Servis parolası tanımlı değil" ve form kapalı.
- **AP yönetim parolası cihaza 5 dk içinde ulaşır:** bugün yalnızca bağlanırken ve günlük 06:00 eşitlemesinde gidiyor. Merkez onu
  5 dk'lık sağlık cevabında da gönderir (`api.py` cihaz ekleri), cihaz değiştiyse `merkez.json`'a yazar. Böylece yönetimde
  "Yenile"den 5 dk sonra cihaz yeni parolayı bilir. Yönetimde "Yenile"nin yanında not: "Cihaz çevrimdışıyken yenilerseniz cihaz,
  yeniden bağlanana kadar eski parolayı kullanır."
- **Deneme sınırı:** MAC (ve IP) başına 15 dk'da 5 yanlış → 15 dk kilit; ayrıca cihaz genelinde saatte 20 yanlış → 1 saat kilit
  (MAC değiştirerek sınırsız deneme olmasın; parola 12 rastgele karakter olduğu için zaten tahmin edilemez).
- **Denetim:** `SERVIS_GIRIS sonuc=hatali|kilit` (MAC, IP), `SERVIS_PPPOE pppoe=acik|kapali vlan=… kullanici_degisti=… parola_degisti=…`
  (MAC, IP; değerler yazılmaz).

## Paket ve güncelleme sırası
- Depends'e `ppp`. Uzaktan güncelleme `apt-get install ./paket.deb` ile bağımlılığı Debian deposundan çeker.
- Merkez değişiklikleri: sağlık raporundaki `wan_tur`/`pppoe` alanlarını saklayıp müşteri sayfasında göstermek ve `ap_parola`'yı
  sağlık cevabına eklemek. Sıra önemli değil: `saglik_temizle` bilinmeyen alanları yok sayıyor (eski merkez yeni cihazın raporunu
  reddetmez), eski cihaz da cevaptaki yeni alanı yok sayar.
- Yeni sürümün PPPoE kapalı cihazlardaki etkisi: `define WAN` kümesi, MSS kuralı, yeni (devre dışı) birim, internet yokken giriş
  sayfasındaki uyarı ve Servis, sağlık cevabıyla gelen AP yönetim parolası.

## Test ve kabul
- **Birim testleri:** ag.toml okuma/yazma ve doğrulama, bağlantı dosyası ve `arayuzler.nft` üretimi, durum çıkarımı (örnek pppd
  günlük satırlarıyla), giriş sayfasında uyarı/servis görünürlüğü ve deneme sınırı.
- **Cihazda (sahte operatör):** cihazın kendi içinde ayrı ağ ad alanında `pppoe-server` (Debian `pppoe` paketi; yalnızca testte
  kurulur, Depends'e girmez) + sanal kablo: doğru parola → `bagli`; yanlış parola → `kimlik_hatasi`; sunucu kapalı → `sunucu_yok`;
  VLAN'lı sunucu; bağlantı kopup geri gelince kendiliğinden yeniden bağlanma; MSS düzeltmesi. Canlı ağa dokunulmaz (mevcut sahte
  istemci yöntemi gibi; dhcpcd'ye sinyal gönderilmez).
- **Canlı kabul** (ilk PPPoE'li müşteride, yanında yedek modemle): ONT'ye takınca bağlanma, tünel ve merkez, misafir girişi,
  5651 kaydında dış IP, yavaşlatma, PPPoE kopunca yeniden bağlanma; yanlış parolayla Servis sayfasından düzeltme.

## Bilinen riskler
- **Servis parolası şifresiz ağdan gider:** giriş sayfası HTTP, Wi-Fi şifresiz. Yakındaki biri o anki AP yönetim parolasını
  yakalayabilir; bu parola o müşterinin AP'sini de korur. Etki o müşteriyle sınırlı; servis yalnızca cihaz çevrimdışıyken kullanılır.
  Servis kullanıldıktan sonra yönetimde AP yönetim parolası yenilenip AP'ye de yazılabilir.
- **Tek oturum sınırı:** bazı operatörler aynı hesapla tek PPPoE oturumuna izin verir; müşterinin eski modemi de aynı hatta
  bağlıysa biri düşer. Kurulumda eski modem sökülür.
- **VLAN bilinmezse** cihaz `sunucu_yok`'ta kalır; operatörden sorulup Servis sayfasından girilir.
- **Ekransız açılış** (ayrı konu): PPPoE'li müşteride de sahte ekran fişi ve BIOS "Restore on AC Power Loss" gerekir.

## Kararlar
Kullanıcının verdiği (2026-10-23): kapsam yalnızca internet alan portta PPPoE; bilgiler dükkânda panelden girilir; müşteride
kendiliğinden bağlanır; yerinde düzeltme ekranı giriş sayfasında, yalnızca internet yokken; servis parolası AP yönetim parolası;
ayar seçenekleri dükkândaki modemin PPPoE sayfasındaki gibi (yukarıdaki karşılaştırma).

Benim kararlarım (kullanıcı kontrol etsin): teknisyenin giriş sayfasına tarayıcıya `10.50.0.1:8080` yazarak girmesi (internet
yokken "oturum açın" penceresi çıkmaz); internet yokken uyarı ve Servis'in PPPoE kapalı cihazlarda da görünmesi; DHCP'nin PPPoE
açıkken de çalışması ve PPPoE'nin tercih edilmesi; panelden değişikliğin 3 dk onay akışından geçmesi, Servis'ten değişikliğin
geçmemesi; Servis formunun oturumsuz tek form olması; AP yönetim parolasının 5 dk'lık sağlık cevabıyla da gitmesi; cihaz genelinde
ek deneme sınırı; arayüz adı `wfcppp` ve `define WAN` kümesi; tünel MTU'su `mtu − 80`; Gelişmiş alanların varsayılanları;
internet yok koşulunun
"varsayılan rota yok" olması; merkezde yeni sağlık kuralı yok (yalnızca gösterim).

Kurucuya bırakılanlar: pppd seçeneklerinin tam listesi, durumun nasıl çıkarılacağı, birim/dosya adları, panel ve giriş sayfası
yerleşimi, deneme sınırının saklanma yeri, sahte operatör betiğinin ayrıntısı.
