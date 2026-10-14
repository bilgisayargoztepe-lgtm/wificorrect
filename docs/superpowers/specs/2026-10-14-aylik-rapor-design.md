# Aylık rapor — tasarım

Tarih: 2026-10-14 · Kullanıcı soruları atladı ("spec yaz"); aşağıdaki kararlar **benim varsayımlarımdır**, onayla kesinleşir.

## Neden

İşletme sahibi cihazı aldıktan sonra ne işe yaradığını göremiyor: kaç misafir bağlandı, yasal kayıtlar eksiksiz tutuldu mu,
cihaz ay boyunca çalıştı mı. Aylık rapor bunu tek sayfada gösterir. İki faydası var:

1. **Güven / uyum:** "Bu ay 31 günün 31'inde 5651 kaydınız tutuldu, mühürlendi ve yedeklendi" — işletme sahibinin olası bir
   denetimde göstereceği somut kanıt. Eksik gün varsa açıkça yazar (cihaz kapalıydı vb.).
2. **Değer / yenileme:** misafir sayıları ve yoğun saatler işletmeye faydalı bilgi; yıllık abonelik yenilenirken
   "bu ürün ne yaptı" sorusunun cevabı.

## Hedefler

1. **Yer:** müşteri paneli (panel.wificorrect.com) menüsünde **"Aylık rapor"**. Ay listesi (en yeni üstte; bu ay "sürüyor"
   etiketiyle, geçmiş aylar kesin). Yönetim (yonetim.wificorrect.com) müşteri sayfasında aynı rapora bağlantı.
2. **Kişisel veri yok:** raporda telefon, ad, MAC, IP, kimlik, kişi başına site **hiç yoktur**; yalnızca sayılar ve
   toplamlar. Bu yüzden kayıt görüntüleme gerekçesi (GEREKCE) istenmez, maskeleme gerekmez; yönetici de gerekçesiz açar.
3. **İçerik (ay için):**
   - **Misafirler:** farklı misafir sayısı, bu ay ilk kez gelen (yeni), daha önce de gelmiş (dönen), toplam giriş
     (OTURUM_BASLA), günlük ortalama farklı misafir, en yoğun gün. Giriş yöntemine göre dağılım: SMS / misafir kodu
     (otel-yurt) / izinli cihaz (portalsız).
   - **Yoğunluk:** saat dilimine göre (0–23) ve haftanın gününe göre farklı misafir; düz CSS çubuk grafik, JS yok.
   - **Kullanım:** toplam veri (indirme + yükleme, GB), en çok ziyaret edilen 10 alan adı (kaç farklı misafir ziyaret etti;
     alan adı kökü düzeyinde, ör. `instagram.com`), toplam doğrulama SMS'i (gönderilen kod sayısı).
   - **5651 kayıt durumu (en önemli bölüm):** ayın her günü için tek satır/kutu: kayıt geldi + mühürlü + merkezde yedekli +
     bütünlük (MANIFEST hash'leri ve zincir) doğrulandı ✔; ya da neyin eksik olduğu (kayıt yok — cihaz kapalı/bağlantısız,
     mühürsüz, yedeklenmedi, doğrulama hatası). Özet satırı: "31 günün 31'i tam".
   - **Cihaz:** ay içinde açılan sağlık sorunları (merkezdeki `sorun` tablosu: tür, açılış, kapanış, süre) ve toplam
     çevrimdışı süre (çevrimdışı sorunlarının süreleri toplamı); ay sonundaki yazılım sürümü.
   - **Engellenenler (varsa):** hazır listeler ve elle yasaklı sitelerle engellenen sorgu sayısı, kategori başına.
     Bu sayı cihazdaki DNS kaydından güvenilir çıkarılabiliyorsa eklenir; çıkarılamıyorsa ilk sürümde bu bölüm **yoktur**
     (uydurma sayı gösterilmez). Uygulayıcı karar verir ve gerekçesini yazar.
4. **Yazdırma:** sayfa yazdırmaya uygun (print CSS: menü gizli, A4, sayfa kırılmaları). "Yazdır / PDF kaydet" düğmesi
   tarayıcının yazdırma penceresini açar. Raporun üstünde işletme adı, unvan, müşteri no, ay, üretim zamanı.
5. **Hesaplama merkezde:** veri zaten merkezde (cihazın her gece gönderdiği mühürlü günlük arşiv + `wc-istatistik`'in
   ürettiği gün özetleri + `sorun` tablosu). Cihazda değişiklik yok, cihaz sürümü gerekmez. Aylık özet `wc-istatistik`
   çalıştıkça (root, mevcut zamanlayıcı) güncellenir; geçmiş ay, o ayın son günü arşive girdikten sonra kesinleşir.
   Panel yalnızca üretilmiş JSON'u okur (bugünkü düzen: root üretir, wcpanel okur).
6. **Birden çok arşiv:** müşterinin bağlı eski arşivleri de varsa (kayitlar.py'deki gibi) aynı gün iki arşivde ise biri
   sayılır; rapor müşteri bazındadır.
7. **Denetim:** rapor görüntülemesi müşteri hareketine yazılmaz (kişisel veri yok); yönetim tarafında da yazılmaz.

## Hedef dışı

- E-posta / SMS ile rapor gönderme (e-posta altyapısı yok; ileride "raporunuz hazır" SMS'i düşünülebilir).
- Sunucuda PDF üretimi (stdlib'de yok; tarayıcı yazdırması yeter).
- Cihaz panelinde rapor (cihaz panelinden müşteri paneline bağlantı yeterli olabilir; ilk sürümde yok).
- İşletmeler arası karşılaştırma, hizmet sağlayıcı için toplu "bütün müşteriler" raporu.
- Kişi bazında herhangi bir liste.

## Anti-hedefler

- **Kişisel veri sızdırmak:** raporda tek bir kişiye ait bilgi çıkmamalı. Çok az misafirli ayda bile alan adı listesi
  kişiyi ele verebilir → bir alan adı en az **3 farklı misafir** ziyaret etmediyse "en çok ziyaret edilenler"de çıkmaz.
- **Yanlış uyum beyanı:** bir gün "tam" gösterilip aslında eksikse bu ciddi bir hatadır. "Tam" yalnızca o günün
  MANIFEST'i ve zincir satırı merkezde doğrulandıysa yazılır; emin olunamayan gün "doğrulanamadı" olur.
- **Merkezi yavaşlatmak:** her sayfa açılışında arşiv taranmaz; hesap arka planda, sayfa hazır JSON okur.

## Kısıtlar ve kararlarım

- Merkez Python stdlib, mevcut panel görünümü ve kaçışlama kuralları (`e()`); JS yok.
- "Yeni misafir" = bu aydan önce bu müşterinin arşivlerinde hiç görülmemiş kişi anahtarı (telefon / TC / pasaport).
- Saat ve gün TR saatine göre; ay = TR takvim ayı.
- İlk rapor ayı: müşterinin arşivindeki ilk gün hangi aydaysa (Göztepe için Ekim 2026).
- Yönetici "Cihaz paneline git" gibi müşteri raporunu da açabilir.

## Uygulayıcıya bırakılanlar

Aylık özet JSON biçimi ve yolu, mevcut gün özetlerinden mi yoksa CSV'den mi hesaplanacağı, bütünlük doğrulamasının
merkezde nasıl yapılacağı (mevcut `tamam_mi` + zincir), engellenen sorgu sayımının mümkün olup olmadığı, sayfa yerleşimi
ve grafik biçimi, test kurgusu.
