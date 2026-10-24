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
   rağmen bağlı kalır) ve tarayıcıya **`10.50.0.1:8080`** yazar (giriş sayfası; Servis'e doğrudan **`10.50.0.1:8080/servis`**).
   Sayfanın altında (hangi sayfa türü açılırsa açılsın) "Bu cihazın internet bağlantısı şu an yok" + neden ve **Servis**
   bağlantısını görür. Servis sayfasında AP yönetim parolası + PPPoE kullanıcı adı (boş bırakılırsa kayıtlı ad kalır), parola,
   VLAN girer; **PPPoE seçimi Açık olmalıdır** (Servis formunda, kayıtlı ayar kapalı olsa da Açık hazır gelir; panelde seçim
   kayıtlı değeri gösterir) → **Kaydet ve bağlan** → kayıt mesajı kaydedilen seçimi söyler ("PPPoE yeniden bağlanıyor" ya da
   "PPPoE kapalı") ve sayfa birkaç saniyede bir yenilenip durumu gösterir.
   - Neden adres elle yazılıyor: cihazın interneti yokken DNS hiçbir adı çözemez; telefonlar "ağda oturum açın" penceresini açmaz,
     yazılan alan adları da açılmaz. Yalnızca IP adresiyle girilir. (Değerlendirilip seçilmeyen yol: internet yokken DNS'in her adı
     yanıtlaması — her cihazda her kesintide bütün misafirlerin davranışını ve 5651 DNS kaydını değiştirirdi.)
   - Teknisyen notu ve cihaz etiketi: "İnternet yoksa: Wi-Fi'a bağlanın, tarayıcıya 10.50.0.1:8080/servis yazın (parola: AP
     yönetim parolası; PPPoE seçimi Açık olmalı)."

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
- **pppd ayrı bir systemd biriminde** (`wificorrect-pppoe.service`, `Restart=always`): yalnızca `pppoe.acik` iken etkin; geçici
  bir işin içinde başlatılmaz (2026-10-03 dersi: geçici işle başlatılan dhcpcd iş bitince öldü). Ayar değişince birim yeniden
  başlar. Bağlantı dosyası (`/etc/ppp/peers/wificorrect`, 600) ag.toml'dan üretilir: PPPoE eklentisi + arayüz (VLAN varsa
  `<wan>.<vlan>`), sabit arayüz adı (`wfcppp`), kullanıcı adı ve parola, varsayılan rota + metrik, operatör DNS'i, kopma
  algılama (LCP yankı), yeniden deneme (`persist`, `holdoff 10`, `maxfail 3`; aşağıda), IPv6 kapalı, MTU/MRU = `mtu`; kimlik
  `pap` ise CHAP/MS-CHAP/EAP reddedilir, `chap` ise PAP/EAP reddedilir; AC adı ve servis adı doluysa PPPoE eklentisinin ilgili
  seçenekleri (`pppoe-ac`, `pppoe-service`). Sistemin ortak `chap-secrets`/`pap-secrets` dosyalarına dokunulmaz.
- **Yeniden deneme sınırsızdır ama başarısızken yavaşlar** (uygulamada netleşti; ilk taslak "sınırsız yeniden deneme" diyordu):
  pppd arka arkaya 3 başarısız denemeden sonra çıkar (`maxfail 3`); birim (`RestartSec=10`, `RestartSteps=5`,
  `RestartMaxDelaySec=300`, systemd 254+) onu 10 sn'den 5 dk'ya çıkan gecikmeyle yeniden başlatır. Neden: yanlış kimlik YÖNTEMİ
  (PAP/CHAP, operatörün istediğinden farklı) seçilince LCP hiç kalkmaz, `holdoff` uygulanmaz ve pppd ~3,3 sn'de bir yeni oturum
  açar (saatte ~1000 deneme; operatörler hesabı kilitleyebilir); yanlış parolada `holdoff` zaten ~13 sn'de bir sınırlar (sahte
  operatörle ölçüldü). Bedeli: uzun bir operatör kesintisinden sonra yeniden bağlanma 5 dk'ya kadar sürebilir.
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
  yavaşlatma **o an varsayılan rotanın arayüzüne** kurulur: en düşük metrikli varsayılan rota `wfcppp` ise `wfcppp`, yalnız DHCP
  rotası varsa ya da rota okunamıyorsa internet alan Ethernet (kaydedici ve panelin anında uygulaması `ctl hiz-uygula` aynı
  kuralı kullanır). `wfcppp` her yeniden bağlanmada yeniden doğduğu için kök kaybolur; kaydedicinin mevcut "kök yoksa yeniden
  kur" turu bunu düzeltir. Çıkış arayüzü değişince (PPPoE bağlandı / düştü) planın eski çıkışta kalan HTB kökü de silinir (önceki
  planın izinden okunur); kök denetimi yalnızca uygulanmış plandaki arayüzlere bakar (taze doğmuş `wfcppp`'nin kökü yoktur, planı
  silmemeli).
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
- **Sağlık raporu** (`/api/saglik`, 5 dk): `wan_tur` (`dhcp`|`pppoe`; ag.toml'da PPPoE açık mı — bağlı olması gerekmez) ve `pppoe`
  (yukarıdaki durum kodu; DHCP'li cihaz da gönderir) eklenir; merkez müşteri sayfasında sağlık kartında "İnternet" satırı gösterir
  ("Modem (DHCP)", "PPPoE bağlı", sorunlu durumlar kırmızı). Yeni sağlık kuralı yok: PPPoE bağlanamazsa cihaz zaten "ulaşılamıyor"
  olur ve mevcut uyarı SMS'i gider.
  - **Merkez bilinmeyen ya da bozuk `wan_tur`/`pppoe` değerini reddetmez**, yalnızca o alanı atar (rapor kabul edilir): ileride
    eklenecek yeni bir durum kodu eski merkezde raporu bozmasın; reddedilseydi çalışan cihaz için "Rapor gelmiyor" uyarısı/SMS'i
    çıkardı. Satır yalnızca `pppoe` koduna bakar; kod yoksa (eski cihaz ya da atılmış değer) "—".

## Panel → Portlar
- İnternet alan portun satırının altında **PPPoE** kutusu: Açık/Kapalı, Kullanıcı adı (örnek: `5321234567@turk.net`), Parola
  (yalnızca yazılır; boş = aynı kalır, yani kayıtlı parola panelden silinemez), VLAN (boş = yok), durum satırı, dükkân notu; altta
  **Gelişmiş**: VLAN önceliği, MTU, kimlik doğrulama (Otomatik/PAP/CHAP), AC adı, servis adı (çoğu hatta boş bırakılır; varsayılandan
  farklı bir değer varsa bölüm açık gelir). Form parola dışında kayıtlı değerlerle dolu gelir: "Kapalı" gönderince kullanıcı adı ve
  ileri alanlar silinmez. Açık/Kapalı seçimi de kayıtlı değeri gösterir (hiç ayarlanmamış cihazda Kapalı gelir; açmak için Açık
  seçilir). Servis formunda bu farklıdır: orada seçim Açık gelir (aşağıda).
- Kaydetme **mevcut port değişikliği akışından** geçer (yeni ayar dosyası → geri alma zamanlayıcısı → geçiş → 3 dk içinde
  "Çalışıyor, onayla"; onaylanmazsa eski ayar döner). Böylece müşterideyken PPPoE bilgisini uzaktan yanlış değiştirmek cihazı
  kalıcı olarak koparmaz.
- Denetim: `PANEL_PORT` satırına `pppoe=acik|kapali vlan=… kullanici_degisti=0|1 parola_degisti=0|1` (kullanıcı adı ve parola
  yazılmaz).

## Giriş sayfası: internet yokken uyarı ve Servis
- **Koşul:** cihazın varsayılan rotası yoksa (ne DHCP ne PPPoE) — **PPPoE açık da kapalı da olsa** (bütün cihazlarda). İnternet
  varken giriş sayfası bugünkü gibidir; Servis bölümü görünmez ve `/servis` adresi (GET, `?izle=1` ve POST) giriş sayfasına
  yönlendirir, form hiç işlenmez. Rota `ip -o route show default` ile bakılır (en çok 10 sn önbellek); `ip` çalışmazsa ya da süresi
  dolarsa internet var sayılır (emin olunamayınca Servis açılmaz); rota yokken internet alan portun durumu okunur.
- **Uyarı (herkese):** sayfanın altında "Bu cihazın internet bağlantısı şu an yok" + kısa neden (yukarıdaki durum metni) +
  **Servis** bağlantısı. **Her sayfa türünde** görünür: giriş formu (hatalı yeniden çizimi dahil), kod sayfası, oturumu açık
  telefonun "Bağlandınız" sayfası (internet yokken 1 sn'lik yönlendirmesi atlanır, uyarı kaybolmasın), lisans kapalı ve MAC
  tanınmadı hata sayfaları, misafir kodu sayfası; yalnızca Servis sayfasının kendisinde yoktur. Misafir giriş akışı (SMS) bu
  durumda zaten çalışmaz; mevcut hata metinleri aynen kalır. (İngilizce sayfada uyarı İngilizce, neden metni Türkçe.)
- **Servis sayfası** (`/servis`, giriş sayfasının kendi sunucusunda, yalnızca Türkçe): durum satırı + **tek form**: Servis parolası,
  PPPoE Açık/Kapalı, kullanıcı adı, parola, VLAN ve paneldeki Gelişmiş alanların aynısı → **Kaydet ve bağlan**. Parola her
  gönderimde birlikte yazılır (oturum, çerez yok). **Kayıtlı kullanıcı adı hiçbir sayfada gösterilmez** (çoğu kez abonenin telefon
  numarasıdır ve sayfa parola doğrulanmadan misafire açıktır): alan boş gelir ("Boş bırakılırsa kayıtlı kullanıcı adı aynı kalır"
  notu); boş gönderilirse parola doğrulandıktan sonra kayıtlı ad kullanılır; hata sayfalarında forma yalnızca o istekte yazılan ad
  döner. VLAN ve gelişmiş alanlar kayıtlı değerlerle dolu gelir; parola alanı hep boştur (boş = kayıtlı parola kalır).
  **PPPoE Açık/Kapalı seçimi, kayıtlı ayar kapalı olsa da Açık gelir:** Servis'e PPPoE'yi açmak için gelinir ve kayıtlı ayar çoğu
  kez kapalıdır (PPPoE hiç ayarlanmamış cihazda varsayılan); seçim kayıtlı değerle gelseydi formu olduğu gibi gönderen teknisyen
  bilgileri PPPoE kapalıyken kaydeder ve internet gelmezdi. Kapatmak için Kapalı elle seçilir. Reddedilen bir kayıttan sonra
  (yanlış ya da boş Servis parolası, kilit, doğrulama hatası) form gönderilen seçimi korur; hiçbir şey gönderilmediyse (eksik
  form) kayıtlı ayardan, yani ilk çizim gibi Açık kurulur. Kaydedince ag.toml güncellenir, bağlantı dosyası yeniden üretilir,
  güvenlik duvarı kuralları yeniden yüklenir, PPPoE birimi yeniden başlar (Kapalı kaydedilmişse durur);
  **geri alma zamanlayıcısı kurulmaz** (cihaz zaten çevrimdışı; geri almak yanlış eski bilgiye döndürürdü). Kayıttan sonra sayfa
  `/servis?izle=1` durum sayfasına geçer ve birkaç saniyede bir kendini yeniler (meta, JS yok); internet gelince giriş sayfası açılır.
  Kayıt mesajı kaydedilen seçimi söyler (kullanıcı adı ve parola hiçbirinde yok): Açık → "Ayar kaydedildi; PPPoE yeniden bağlanıyor. …";
  Kapalı → "Ayar kaydedildi; PPPoE kapalı. Cihaz internet portundan modemin adres vermesini bekliyor (PPPoE hattı için Ayarları
  değiştir bağlantısından PPPoE seçimini Açık yapın). …".
  Panelden yapılmış, onay bekleyen bir port değişikliği varken (en çok 3 dk) Servis kaydetmeyi reddeder.
- **Servis parolası = müşteriye özel AP yönetim parolası** (`merkez.json` → `ap_parola`; yönetimde ve cihaz panelinde görünür).
  Merkeze sormadan cihazda doğrulanır (sabit sürede karşılaştırma). `ap_parola` yoksa (cihaz bağlanmamış ya da eski merkez) ya da
  merkezin kuralından (8 karakter) kısaysa: "Servis parolası tanımlı değil" ve form kapalı.
- **AP yönetim parolası cihaza 5 dk içinde ulaşır:** bugün yalnızca bağlanırken ve günlük 06:00 eşitlemesinde gidiyor. Merkez onu
  5 dk'lık sağlık cevabında da gönderir (`api.py` cihaz ekleri), cihaz değiştiyse `merkez.json`'a yazar. Böylece yönetimde
  "Yenile"den 5 dk sonra cihaz yeni parolayı bilir. Yönetimde "Yenile"nin yanında not: "Cihaz çevrimdışıyken yenilerseniz cihaz,
  yeniden bağlanana kadar eski parolayı kullanır."
- **Deneme sınırı:** MAC (MAC bilinmiyorsa IP) başına 15 dk'da 5 yanlış → 15 dk kilit; ayrıca cihaz genelinde saatte 20 yanlış →
  1 saat kilit (MAC değiştirerek sınırsız deneme olmasın; parola 12 rastgele karakter olduğu için zaten tahmin edilemez). Yalnızca
  yanlış Servis parolası sayılır (boş parola, eksik form ve doğru paroladan sonraki doğrulama hataları sayılmaz); kilit sürerken
  parolaya bakılmaz. Sayaçlar bellektedir (portal yeniden başlayınca ya da cihaz açılınca sıfırlanır). Bir misafir cihaz genelindeki
  kilidi kesinti sırasında tetikleyebilir (tasarım bedeli; teknisyen en çok 1 saat gecikir).
- **Denetim:** `SERVIS_GIRIS sonuc=hatali|kilit` (MAC, IP; `kilit` her kilit dönemi için bir kez yazılır), `SERVIS_PPPOE pppoe=acik|kapali
  vlan=… kullanici_degisti=… parola_degisti=…` (MAC, IP; değerler yazılmaz; iş başlatılamazsa sonuna `hata=yazilamadi|baslatilamadi`),
  uygulayan komut ayrıca `PPPOE_SERVIS_UYGULANDI pppoe=acik|kapali vlan=…`.

## Paket ve güncelleme sırası
- Depends'e `ppp`. Uzaktan güncelleme `apt-get install ./paket.deb` ile bağımlılığı Debian deposundan çeker.
- Merkez değişiklikleri: sağlık raporundaki `wan_tur`/`pppoe` alanlarını saklayıp müşteri sayfasında göstermek ve `ap_parola`'yı
  sağlık cevabına eklemek. Sıra önemli değil: `saglik_temizle` bilinmeyen alanları yok sayıyor (eski merkez yeni cihazın raporunu
  reddetmez), eski cihaz da cevaptaki yeni alanı yok sayar.
- Yeni sürümün PPPoE kapalı cihazlardaki etkisi: `define WAN` kümesi, MSS kuralı, yeni (devre dışı) birim, internet yokken giriş
  sayfasındaki uyarı ve Servis, sağlık cevabıyla gelen AP yönetim parolası. Yükseltme güvenlik duvarını yeniden yüklemez (postinst
  nft'ye dokunmaz): MSS kuralı (`guvenlik.nft`) bir sonraki açılışta, WAN kümesi (`arayuzler.nft`, pakette yok, program yazar) ilk
  PPPoE ayarında ya da bir port değişikliğinde devreye girer; PPPoE ayarını uygulayan iki yol (panel geçişi ve Servis) dosyaları
  yazdıktan sonra `nft -f` ile kuralları yeniden yükler. PPPoE kapalıyken ikisinin de etkisi yoktur (MSS kuralı 1500 MTU'lu
  yolda mesajı değiştirmez).

## Test ve kabul
- **Birim testleri:** ag.toml okuma/yazma ve doğrulama, bağlantı dosyası ve `arayuzler.nft` üretimi, durum çıkarımı (örnek pppd
  günlük satırlarıyla), giriş sayfasında uyarı/servis görünürlüğü ve deneme sınırı.
- **Cihazda (sahte operatör):** cihazın kendi içinde ayrı ağ ad alanında `pppoe-server` (Debian `pppoe` paketi; yalnızca testte
  kurulur, Depends'e girmez) + sanal kablo: doğru parola → `bagli`; yanlış parola → `kimlik_hatasi`; sunucu kapalı → `sunucu_yok`;
  VLAN'lı sunucu; bağlantı kopup geri gelince kendiliğinden yeniden bağlanma; MSS düzeltmesi. Canlı ağa dokunulmaz (mevcut sahte
  istemci yöntemi gibi; dhcpcd'ye sinyal gönderilmez). Uygulandı: `scripts/pppoe-deneme.sh` (18 kontrol; canlı Göztepe'de 18/18
  PASS, kök ad alanı denemeden önce/sonra aynı).
- **Canlı kabul** (ilk PPPoE'li müşteride, yanında yedek modemle): ONT'ye takınca bağlanma, tünel ve merkez, misafir girişi,
  5651 kaydında dış IP, yavaşlatma, PPPoE kopunca yeniden bağlanma; yanlış parolayla Servis sayfasından düzeltme. Ek olarak
  gerçek hatta bir kez `journalctl -u wificorrect-pppoe.service -o cat` (pppd'nin systemd birimi altındaki gerçek çıktısı —
  satırların çift gelmesi, başlangıç satırları — sahte operatör denemesinde gözlenmedi; durum çözümleyici alt dizgiyle eşler) ve
  bağlanma/kopma sonrası `tc -s class show` (yavaşlatma doğru çıkış arayüzünde). Tam liste: `docs/KURULUM_GUNLUGU.md` "PPPoE".

## Bilinen riskler
- **Servis parolası şifresiz ağdan gider:** giriş sayfası HTTP, Wi-Fi şifresiz. Yakındaki biri o anki AP yönetim parolasını
  yakalayabilir; bu parola o müşterinin AP'sini de korur. Etki o müşteriyle sınırlı; servis yalnızca cihaz çevrimdışıyken kullanılır.
  Servis kullanıldıktan sonra yönetimde AP yönetim parolası yenilenip AP'ye de yazılabilir.
- **Tek oturum sınırı:** bazı operatörler aynı hesapla tek PPPoE oturumuna izin verir; müşterinin eski modemi de aynı hatta
  bağlıysa biri düşer. Kurulumda eski modem sökülür.
- **VLAN bilinmezse** cihaz `sunucu_yok`'ta kalır; operatörden sorulup Servis sayfasından girilir.
- **Operatörün hesabı kilitlemesi:** yanlış kullanıcı adı/parola/kimlik yöntemiyle cihaz operatörü yoklamayı sürdürür (sınırsız
  deneme); yavaşlatma yukarıdaki geri çekilmeyle yapılır (3 başarısız denemelik turlar; turlar arasındaki bekleme 10 sn'den 5 dk'ya
  çıkar). Bazı operatörler yine de kilitleyebilir; teknisyen yanlış bilgiyi hemen düzeltmelidir. systemd'in
  `RestartSteps` sayacının sağlıklı çalışmadan sonra sıfırlanıp sıfırlanmadığı doğrulanmadı (en kötü durum: her kesintide 5 dk'ya
  kadar gecikme, elle yeniden başlatmaya ya da açılışa kadar).
- **Cihaz geneli Servis kilidi** misafir tarafından tetiklenebilir (kesinti sırasında 20 yanlış deneme → 1 saat); portal yeniden
  başlayınca ya da cihaz açılınca kalkar.
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

Uygulamada netleşenler (2026-10-23/24; kullanıcı kontrol etsin; ayrıntı `docs/KURULUM_GUNLUGU.md` "PPPoE"):
- Yeniden deneme: `maxfail 3` + birim gecikmesi 10 sn → 5 dk (yukarıda "Yeniden deneme sınırsızdır ama başarısızken yavaşlar").
- Servis sayfası kayıtlı kullanıcı adını hiç göstermez; boş bırakılırsa parola doğrulandıktan sonra kayıtlı ad kullanılır.
- "İnternet yok" uyarısı ve Servis bağlantısı her sayfa türünde görünür; teknisyen adresi `10.50.0.1:8080/servis`.
- Servis formunda PPPoE Açık/Kapalı seçimi, kayıtlı ayar kapalı olsa da **Açık** gelir (teknisyen Servis'e PPPoE'yi açmak için gelir;
  panelde seçim kayıtlı değeri gösterir ve hiç ayarlanmamış cihazda Kapalı gelir). Reddedilen kayıtta gönderilen seçim korunur; kayıt
  mesajı kaydedilen seçimi söyler ("PPPoE yeniden bağlanıyor" / "PPPoE kapalı"). Teknisyen notu bunu söyler.
- Merkez bilinmeyen/bozuk `wan_tur`/`pppoe` değerini reddetmez, yalnızca o alanı atar; "İnternet" satırı yalnızca `pppoe` koduna bakar.
- `SERVIS_GIRIS sonuc=kilit` kilit dönemi başına bir kez yazılır; Servis'te boş parola ve doğru paroladan sonraki doğrulama hataları
  deneme sınırına sayılmaz.
- Panel formu ve Servis formu aynı ayrıştırmayı kullanır (`pppoe::formdan`, `pppoe::form_html`); panel formu kayıtlı değerlerle
  (parola hariç) dolu gelir.
- Servis kaydı güvenlik duvarı kurallarını yeniden yükler; ag.toml yoksa ya da onay bekleyen bir port değişikliği varsa uygulanmaz.
- Onaylanmamış bir port değişikliği PPPoE'yi kapatmış ve cihaz 3 dk dolmadan yeniden açılmışsa, açılışta geri alma pppd birimini
  etkinleştirir ve beklemeden başlatır.
- Yavaşlatma etkin çıkış arayüzüne kurulur ve çıkış değişince eski çıkıştaki HTB kökü silinir (yukarıda).
