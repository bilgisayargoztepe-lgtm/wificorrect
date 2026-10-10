# Bağlı kullanıcılarda anlık hız, bugünkü kullanım ve yavaşlatma — tasarım

Tarih: 2026-10-10 · Dal: `hiz-olcer` (`ozet-tasarim` üzerine; yeni panel görünümünü kullanır)

## Neden

Dükkânda internet yavaşladığında işletme sahibi **kimin yüklendiğini** görmek ve o kişiyi ya **bağlantısını kesip** ya da
**atmadan yavaşlatarak** durumu düzeltmek istiyor (kullanıcının sözü: "internet yavaşladığında"). Şu an panelde kimin ne kadar
trafik yaptığı hiç görünmüyor; elde yalnızca "Bağlantıyı kes" var.

Başarı: yavaşlık anında Kullanıcılar → Bağlı kullanıcılar açılınca en çok yükleneni en üstte, gerçeğe yakın Mb/sn ile görmek;
tek tıkla (Yavaşlat → hız) onu sınırlamak ve sınırın gerçekten tuttuğunu hissetmek.

## Hedefler

1. **Bağlı kullanıcılar tablosu** (`/cihazlar#oturumlar`, ayrıca `/oturumlar`) her satırda:
   - **↓ İndirme Mb/sn** ve **↑ Yükleme Mb/sn**: son ~10 saniyenin ortalaması (megabit/saniye, 1 ondalık).
   - **Bugün MB**: o gün 00:00'dan (Türkiye saati) beri indirilen + yüklenen toplam (MB = 10⁶ bayt, tam sayı).
     Gün değişince sıfırlanır.
2. **Sıralama** (JS'siz, bağlantı/düğme): *Anlık hız* (indirme + yükleme, varsayılan, büyükten küçüğe), *Bugün MB*,
   *Bağlanma saati* (bugünkü sıra).
3. **10 sn'de bir kendini yenileme**: aç/kapa düğmesi; açıkken sayfa `<meta http-equiv="refresh">` ile 10 sn'de bir yenilenir.
   JS yok (CSP `default-src 'none'` aynen kalır).
4. **Yavaşlat**: satırda tek **Yavaşlat** düğmesi. Basınca (JS'siz açılır, `<details>`) altında *1 / 2 / 5 / 10 / 20 Mb/sn*
   düğmeleri çıkar; birine basmak o cihazı hem indirmede hem yüklemede o hızla sınırlar.
   - Yavaşlatılmış satırda turuncu **"≤ N Mb/sn"** rozeti ve **"Yavaşlatmayı kaldır"** düğmesi görünür (Yavaşlat düğmesi yerine).
   - Sınır **siz kaldırana kadar** sürer (kullanıcı kararı). Kişi çıkıp yeniden bağlansa, IP'si değişse, cihaz yeniden başlasa
     da geçerlidir.
5. **Bağlantıyı kes** bugünkü gibi kalır.
5a. **Portalsız (İzinli) cihazlar da tabloda** (kullanıcı kararı): `allow_mac`'teki ve şu an IP'si olan cihazlar
   (MAC → IP, DHCP kiralarından) ad soyad yerine **"İzinli cihaz"** yazısı ve MAC ile listelenir; hız/MB görünür,
   yavaşlatılabilir. Telefonları ve oturumları olmadığı için "Bağlantıyı kes" yerine boş hücre (izinliden çıkarma
   İzinli cihazlar bölümünden yapılır).
5b. **Yavaşlatılmış cihazlar bölümü** (kullanıcı kararı): Kullanıcılar sayfasına, Yasaklı/İzinli cihazlar gibi yeni bölüm
   (`/yavaslatilmis`, id `yavaslatilmis`, kutucuğu da var): bütün sınırlar (MAC, varsa son bilinen ad, hız, ne zaman/kim
   koydu) listelenir, her satırda **Yavaşlatmayı kaldır**. Böylece o an bağlı olmayan kişinin sınırı da kaldırılabilir.
6. **Ölçüm yükü düşük**: conntrack'e ek yük yok; sayaçlar nft'de IP başına dinamik set + counter; kaydedici 5 sn'de bir okur.

## Hedef dışı (non-goals)

- Erişim noktasından (TP-Link AP) bilgi almak, Wi-Fi sinyali ölçmek — **yapılmayacak** (kullanıcı kararı). Yalnızca cihazdan
  internete geçen trafik sayılır.
- Cihazın kendisine giden trafik (DNS, portal, panel) sayılmaz; yalnızca `br-hotspot ↔ WAN` iletimi.
- Geçmiş grafikler, günlük/aylık kullanım raporu, kota (MB sınırı), otomatik yavaşlatma — sonraki işler.
- Özet sayfasına hız eklemek — bu işte yok.
- IPv6 — misafir IPv6'sı zaten düşürülüyor (`gate` zinciri).

## Anti-hedefler (çalışsa bile başarısızlık sayılır)

- Ölçüm ya da yavaşlatma yüzünden **kimsenin interneti kesilirse** veya yavaşlatılmamış kullanıcılar yavaşlarsa.
- Yavaşlatma kuralı yüklenemediğinde güvenlik duvarının **açık kalması** ya da 5651 kaydının bozulması.
- Sayfa yenilemesinin denetim kaydını şişirip gerçek bakışları gömmesi (bkz. açık soru 1).
- Panelin kişisel veri kurallarını gevşetmek.

## Kısıtlar

- Cihaz: J1900, Debian 13, çekirdek 6.12, nftables 1.1.3, `sch_htb` mevcut. Misafir köprüsü `cfg.main.iface`
  (`br-hotspot`), internet portu `ag.toml`'daki `wan` (`enp3s0`); adlar sabit yazılmaz, ayardan okunur.
- Misafir ağı 10.50.0.0/24, NAT `inet wfc` postrouting'de. İndirme yönünde paket `forward`'da hedef IP'si zaten misafir IP'si
  (de-NAT sonrası); hedef MAC bu noktada yoktur → sayaç **IP başına**, MAC eşlemesi oturumlardan (`sessions`) yapılır.
- Panel betiksiz (CSP `default-src 'none'; style-src 'unsafe-inline'`), mevcut şablon ve `ozet-tasarim` görünümü.
- **Ağ/güvenlik duvarı değişikliği canlı cihazda yalnızca ölü adam anahtarıyla** (yedek + `systemd-run --on-active=300`
  geri alma + yeni bağlantıyla doğrula + iptal). Bu, ilk kurulum için de geçerli.
- Gerekçe ve denetim kuralları **aynen** geçerli: Bağlı kullanıcılar gerekçesiz açılır (bağlantı kesmek için gerekli) ama her
  bakış `PANEL_OTURUMLAR` olarak kaydedilir (`src/panel/gerekce.rs`, `hareketler.rs`).

## Nasıl (kararlaştırılanlar ve benim kararlarım)

### 1. Sayaç (nft) — *kullanıcı: nft dinamik set + counter*

Yeni tablo `inet wfc_hiz` (kapi.nft yanında ayrı dosya `hiz.nft`, `wificorrect-guvenlik.service` yükler):

```
table inet wfc_hiz {
  set indir { type ipv4_addr; flags dynamic, timeout; timeout 1d; counter; }
  set yukle { type ipv4_addr; flags dynamic, timeout; timeout 1d; counter; }
  chain say {
    type filter hook forward priority filter - 10; policy accept;
    iifname $LAN oifname $WAN update @yukle { ip saddr }
    iifname $WAN oifname $LAN update @indir { ip daddr }
  }
}
```

*Benim kararım:* zincir `gate`'ten (priority filter − 5) **önce** çalışır ve yalnızca sayar (accept/drop yok) → mevcut
kapı mantığına dokunmaz; reddedilen giriş öncesi paketler de sayılabilir ama yalnızca oturumu olan IP'ler gösterilir.
`timeout 1d`: ayrılan IP'lerin öğesi kendiliğinden düşer. Tablo yüklenemezse yalnızca hız sütunları "—" olur; kapı etkilenmez.

### 2. Okuma ve hesap (kaydedici) — *kullanıcı: kaydedici 5 sn'de bir okur, /run'a küçük JSON yazar*

- Kaydedici ana döngüsüne 5 sn'lik bir adım: `nft -j list set inet wfc_hiz indir` ve `yukle` (iki alt komut, ~ms).
- Her IP için son 3 örnek (≈10 sn) tutulur; **hız = (son bayt − 10 sn önceki bayt) × 8 / geçen süre**. Sayaç azalırsa
  (öğe düşüp yeniden eklendi) o aralık sıfır sayılır.
- **IP → MAC**: önce oturumlar, yoksa DHCP kiraları (izinli cihazlar için; kaydedici `leases`'i zaten tutuyor).
  İkisinde de olmayan IP'ler yok sayılır.
- **Bugün bayt**: MAC başına biriktirilir (IP değişse de kişi aynı kalır). Gün değişince sıfırlanır.
  *Benim kararım:* günlük toplam dakikada bir `state_root/trafik_gun.json`'a da yazılır ki kaydedici/cihaz yeniden başlarsa
  bugünkü MB kaybolmasın (`/run` tmpfs'tir).
- Çıktı: `/run/wificorrect/trafik.json` (atomik yaz), örnek:
  `{"zaman": 1791640000, "cihazlar": {"aa:bb:..": {"ip":"10.50.0.93","indir_bps":38400000,"yukle_bps":2100000,"bugun_bayt":4820000000}}}`
- Panel bu dosyayı okur; dosya yoksa ya da 30 sn'den eskiyse hız sütunları "—", altta "Ölçüm alınamıyor" notu.

### 3. Yavaşlatma — *kullanıcı: Yavaşlat → 1/2/5/10/20 Mb/sn, siz kaldırana kadar; benim kararım: HTB + nft `meta priority`*

- Kalıcı kayıt: `state_root/hiz_sinir.json` = `{ "<mac>": {"hiz": <Mb/sn>, "ad": "<son bilinen ad soyad ya da İzinli cihaz>", "zaman": "<iso>", "kim": "<panel hesabı>"} }`. **MAC'e bağlı** (benim kararım) ki IP/oturum
  değişince sınır kişiyle gitsin.
- Uygulama: tek uygulayıcı **kaydedici** (5 sn adımında uzlaştırır): sınırlı her MAC'in güncel IP'si için
  - `tc`: `$LAN` (indirme) ve `$WAN` (yükleme) çıkışlarında kök HTB; varsayılan sınıf sınırsız (1 Gbit, altında fq_codel —
    bugünkü davranış); her sınırlı cihaz için bir sınıf (rate = ceil = N Mbit, altında fq_codel).
  - `nft`: `inet wfc_hiz` içinde `meta priority set` ile paketi o sınıfa yollayan kurallar (`ip daddr X` → indirme sınıfı,
    `ip saddr X` → yükleme sınıfı; NAT sonrası WAN'da da skb önceliği korunur). Sınıflandırma için tc filtresi gerekmez.
  - Kural seti her değişiklikte **bütün olarak** yeniden yazılır (`nft -f` ile atomik), artık kalmaz.
- **Güncellenen cihazlarda kurulum** (benim kararım, gözden geçirme sonrası): `wificorrect-guvenlik.service` yeniden
  başlatılmaz (`conntrack -F` herkesi düşürür). Kaydedici açılışta ve her uzlaştırmada `inet wfc_hiz` tablosu yoksa
  `nft -f /etc/wificorrect/hiz.nft` ile ekler (yalnızca o tablo; diğer tablolara dokunmaz) ve HTB köklerini kurar; böylece
  `.deb` güncellemesinden sonraki kaydedici yeniden başlatması yeterli. Açılışta da guvenlik servisi aynı dosyayı yükler.
  tc kurulamazsa (hata) yavaşlatma "uygulanamadı" uyarısıyla panelde görünür, sayım ve kapı çalışmaya devam eder.
  Ölü adam anahtarı üründe kod değildir; canlı cihaza (1537344) **ilk kurulumda elle** uygulanır.
- *Neden HTB (benim kararım)*: nft `limit … drop` (policer) TCP'yi düzensiz ve hedefin altında yavaşlatır; HTB kuyruğa alarak
  düzgün şekillendirir, yavaşlatılmamışlar etkilenmez.
- Panel: `POST /oturumlar/yavaslat` (`mac`, `hiz` ∈ {1,2,5,10,20}) ve `POST /oturumlar/yavaslatma-kaldir` (`mac`), CSRF'li,
  dosyayı yazar ve kaydediciyi bekletmemek için hemen bir uzlaştırma tetikler (*benim kararım:* `wificorrect ctl hiz-uygula`).
  Denetim: `PANEL_YAVASLAT` ("Yavaşlattı", `mac=… hiz=…`), `PANEL_YAVASLATMA_KALDIR` ("Yavaşlatmayı kaldırdı").
  Bunlar kişisel veri bakışı değil, işlem kaydıdır (Bağlantı kesti gibi).

### 4. Sayfa

- Taslak (onaylandı): bölüm açıklama notu → üstte *Sırala: Anlık hız / Bugün MB / Bağlanma saati* düğmeleri ve sağda
  *"⟳ 10 sn'de bir yenile: açık/kapalı"* → tablo: Durum, Telefon, Ad soyad, MAC, IP, ↓ Mb/sn, ↑ Mb/sn, Bugün MB, Başlangıç,
  Kalan, Yavaşlat, Bağlantıyı kes → altta "Ölçüm N sn önce".
- Anlık hız için renk vurgusu yok (kullanıcı kararı); sıralama en yükleneni zaten üste getirir.
- Sıralama ve yenileme sorgu parametresiyle: `/cihazlar?sira=hiz|mb|saat&yenile=1#oturumlar`. Yenileme açıkken
  sayfa başına `<meta http-equiv="refresh" content="10">` (şablona `$bas_ek` yer tutucusu).
- **Denetim ve yenileme** (kullanıcı kararı): yenileme düğmesiyle açılınca bir kez `PANEL_OTURUMLAR_YENILE`
  ("Otomatik yenilemeyi açtı") yazılır; açık kaldıkça kendiliğinden gelen yenilemeler tekrar yazılmaz. Ayrım için
  (*benim kararım*) yenileme bağlantısı `yenile=1` ile, meta yenilemesinin URL'si ek bir `oto=1` ile gider; `oto=1` taşıyan
  istek, aynı panel oturumunda son 15 dk içinde yenileme açılmışsa kaydedilmez, değilse normal bakış gibi kaydedilir
  (elle yazılmış/yer imi URL kuralı delemez). Panel oturumunda yenileme açılış zamanı tutulur.
  Diğer her bakış (ilk açılış, sıralama değiştirme) bugünkü gibi `PANEL_OTURUMLAR`.
- "Yavaşlatılmış cihazlar" bölümüne bakış kişisel veri sayılır (son bilinen ad gösterir) → `PANEL_YAVASLATILMIS`
  ("Yavaşlatılmış cihazlara baktı"), kişisel veri listesinde.
- Telefonda tablo yatay kayar (bugünkü gibi).

### 5. Gösterim (kabul)

Cihazda **sahte istemci**: `br-hotspot`'a bağlı bir ağ ad alanı (veth, sabit test MAC'i, DHCP'den 10.50.0.x),
geçici **`allow_mac`** öğesiyle (izinli cihaz olarak tabloda görünür; sahte oturum açılmaz, 5651 kayıtlarına uydurma kişi
girmez — trafiği gerçek olduğu için kayıtta MAC'iyle doğru görünür). İnternetteki bir test dosyasını indirir
(kapı misafirin yerel ağa, ör. 192.168.1.109'a çıkmasını engeller; o yol kullanılmaz). Göster:
1. `curl`'ün ölçtüğü hız ile panelin / `trafik.json`'un gösterdiği indirme Mb/sn'si ±%10 içinde.
2. 5 Mb/sn yavaşlatınca `curl` hızı ≈ 5 Mb/sn'ye (±%15) iner; kaldırınca geri çıkar; diğer gerçek müşteriler etkilenmez.
3. Bugün MB, indirilen dosya boyutu kadar artar.
Deneme bitince ad alanı, test öğeleri ve test sınırı silinir. Canlıya ilk tc/nft kurulumu ölü adam anahtarıyla.

## Açık sorular

Yok (gözden geçirmede çıkan üç soru kullanıcıyla kapatıldı: yenilemede tek kayıt, izinli cihazlar tabloda,
Yavaşlatılmış cihazlar bölümü). Spec 2026-10-10 kullanıcı tarafından onaylandı (hız vurgusu çıkarılarak).

## Uygulayıcıya bırakılanlar

- Modül/dosya adları, kaydedicideki 5 sn adımının yeri, JSON alan adları, testlerin kurgusu.
- HTB sınıf numaralandırması, `burst` değerleri (hedef ±%15'i tutturacak şekilde).
- Bölüm notlarının tam metni.
