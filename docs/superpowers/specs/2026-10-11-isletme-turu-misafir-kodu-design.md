# İşletme türü ve misafir kodu ile giriş — tasarım

Tarih: 2026-10-11 · PMS entegrasyonu **yok** (kullanıcı kararı; PMS araştırması yapıldı, ertelendi).

## Neden

Otel, pansiyon ve yurtlarda misafirin telefon numarası çoğu zaman yok ya da SMS gelmiyor (yabancı numara, roaming). Misafir
zaten resepsiyonda kimliğiyle kayıt oluyor. Personel bu sırada misafire 6 haneli bir **misafir kodu** üretip verir; misafir
portalda kimlik numarası + kodla internete çıkar. 5651 kaydı, kimliği personelin girdiği kişiye bağlanır. Ürün işletme
türünü bilirse türe uygun giriş yöntemlerini açar; ileride başka yöntemler de aynı yerden eklenir.

## Hedefler

### 1. İşletme türü
- Cihaz paneli → Ayarlar → İşletme: **İşletme türü** seçimi: Kafe/Restoran, Otel/Pansiyon, Yurt, Diğer.
  Ayar anahtarı `main.isletme_turu` ∈ {`kafe`, `otel`, `yurt`, `diger`}; **varsayılan `kafe`** (mevcut cihazlar böyle kalır).
- Tür hiç seçilmemişse (ayar dosyasında anahtar yok) Özet'te "İşletme türünüzü seçin" uyarısı (Ayarlar'a bağlantı). Seçilince
  (kafe dahil) uyarı kalkar. *Benim kararım:* "seçilmemiş" = anahtar yok; varsayılan davranış kafe.
- Tür merkeze eşitlemede gider (`/api/eslesme` gövdesine `isletme_turu`); merkez `cihaz` tablosunda saklar; yonetim.wificorrect.com
  müşteri listesinde ve müşteri sayfasında görünür ("Kafe/Restoran", "Otel/Pansiyon", "Yurt", "Diğer", "—").
- Değişiklik denetime: `PANEL_ISLETME_TURU eski=… yeni=…`.

### 2. Giriş yöntemleri
- Ayarlar'da **Giriş yöntemleri** bölümü: **SMS** (her zaman açık, değiştirilemez, bilgi satırı) ve **Misafir kodu** (yalnızca tür
  Otel/Pansiyon ya da Yurt iken görünür; aç/kapat; varsayılan kapalı). Ayar `[giris] misafir_kodu = bool`.
- Tür otel/yurt dışına çekilirse misafir kodu **kendiliğinden kapanır** (ayar false; mevcut kodlar silinmez ama portalda girilemez,
  açık oturumlar sürelerini doldurur). *Benim kararım.*
- Tasarım: giriş yöntemleri bir liste (`enum GirisYontemi { Sms, MisafirKodu }`, her biri: kimlik, ad, hangi türlerde görünür,
  açılabilir mi); ileride kupon vb. yeni öğe eklenerek genişler.

### 3. Misafir kodu üretme (cihaz paneli)
- Menüde (yalnızca misafir kodu açıkken) **Misafir kodları** grubu; "Kod üret" formu + "Aktif kodlar" listesi.
- Form: ad, soyad, kimlik türü (**T.C. vatandaşı** / **Yabancı**), oda numarası (1–10 harf/rakam/`-`), geçerlilik:
  **24 saat (varsayılan), 48 saat, 72 saat, 1 hafta, 2 hafta, 1 ay (30 gün)**.
  - T.C.: TC kimlik no — 11 hane, ilk hane ≠ 0, resmi kontrol basamağı algoritması (10. ve 11. hane). NVİ sorgusu yok.
  - Yabancı: pasaport no (5–20 harf/rakam, büyük harfe çevrilir) + uyruk (`src/ulkeler.rs` listesinden, ISO kodu saklanır).
- "Kod üret" → 6 haneli kod (güvenli rastgele, `getrandom`), ekranda büyük; **bitiş tarihi ve saati** (TR saati, `GG.AA.YYYY SS:DD`)
  açıkça. Yazdırılabilir **fiş** sayfası: ad soyad, oda, kod, bitiş, portala giriş talimatı **TR + EN**. Panelde JS yok:
  fiş ayrı sayfa (`@media print` ile yalnızca fiş), "Yazdırmak için Ctrl+P" notu. *Benim kararım.*
- Aynı kimlik (aynı TC ya da aynı uyruk+pasaport) için yeni kod üretilirse eskisi **iptal** (açık oturumları kapanır, neden=kod_iptal).
- **Aktif kodlar** listesi: ad soyad, oda, kimlik türü, kimlik **maskeli** (`123******01`, `X12****67`), bitiş; "Fişi göster",
  "Süre uzat" (aynı seçenekler; mevcut bitişin üzerine eklenir), "İptal et". Süresi dolan kodlar listeden düşer.
- Kodlar cihazda `state_root/misafir_kodlari.json` (600, atomik yazma). Kod **açık** saklanır (fişin yeniden gösterilebilmesi için;
  dosya yalnızca root, panel erişimi denetimli). *Benim kararım.*
- Denetim: `PANEL_MISAFIR_KODU islem=uret|iptal|uzat oda=… bitis=… tur=tc|pasaport` — **kodun kendisi, TC ve pasaport yazılmaz**.
  Panel ekranındaki kod üretimi ve listeye bakış kişisel veri bakışıdır (`PANEL_MISAFIR_KODLARI`, kişisel veri listesinde).

### 4. Portal
- **TR | EN dil seçimi** bütün portalda (kullanıcı kararı): sayfanın üstünde iki bağlantı (`?dil=en`), seçim çerezde; ilk açılışta
  `Accept-Language` İngilizce ise EN. Form etiketleri, düğmeler, hata mesajları, başarı sayfası çevrilir. Hukuki metinler
  (aydınlatma, açık rıza, sözleşme) için Portal metinleri sayfasına **İngilizce alanlar** eklenir; boşsa Türkçesi gösterilir.
- Misafir kodu açıksa giriş sayfasında türe göre ikinci yol: Otel → "Otel misafiriyim" / "Hotel guest"; Yurt → "Yurt sakiniyim"
  / "Dormitory resident". Betiksiz: `<details>` ya da ayrı sayfa (`/misafir`). *Benim kararım:* ayrı sayfa `/misafir`.
- Form: kimlik türü (T.C. / Pasaport), TC no ya da pasaport no (+ uyruk), 6 haneli kod, açık rıza + sözleşme onayları (SMS formuyla aynı).
  Kod alanı: "Personelin size verdiği veya ilettiği kodu girin" / "Enter the code given or sent to you by our staff".
  **"kupon" kelimesi hiçbir metinde geçmez.**
- Kimlik + kod eşleşir ve süre dolmamışsa internete çıkar: oturum **kodun bitişinde** biter (nft auth öğesinin süresi = kalan süre).
  Kod uzatılırsa açık oturumların bitişi ve nft süresi de uzar. **Kod başına cihaz sınırı** = `main.max_devices_per_phone`
  (aşılırsa en eski oturum kapanır, bugünkü SMS kuralı gibi; neden=cihaz_limiti).
- Kaba kuvvet: IP ve MAC başına **5 hatalı deneme → 15 dk kilit**; sabit süreli karşılaştırma; kimlik bulunamadı / kod yanlış /
  süre dolmuş ayrımı kullanıcıya **tek mesajla** ("Bilgiler eşleşmedi ya da kodun süresi doldu") — tahmin ipucu verilmez.
- Kod iptal edilirse o koda bağlı açık oturumlar hemen kapanır (`OTURUM_BITIS neden=kod_iptal`).

### 5. 5651 ve KVKK
- **Kişi anahtarı** (bugünkü telefon alanının yerini alan genel kimlik): `TC:<11 hane>` ya da `PASAPORT:<ISO2>:<NO>` (örn.
  `PASAPORT:DE:X1234567`). CSV `telefon` sütununa bu değer yazılır (şema değişmez, 19 sütun); kişi dosyası adı `:` → `_`
  (`TC_12345678901.csv`, `PASAPORT_DE_X1234567.csv`); kişi listesi (index.csv) aynı anahtarla. *Benim kararım* (kullanıcının
  önerdiği biçimin dosya adı güvenli hâli).
- **Maskeleme kapsamı (kullanıcı kararı):** bütün listeler ve özetler **her yerde maskeli** — cihaz panelinde (Özet "Bugün Yeni
  Üyeler", Bağlı kullanıcılar, Kullanıcılar listesi ve kişi sayfası, kod listesi) ve **panel.wificorrect.com müşteri ekranlarında**
  (gün ve kişi görünümleri, arama sonuçları; `detay.py`, `ozet.py`). Tam değer yalnızca 5651 kaydının kendisinde: Kayıtlar dosya
  görüntüleyici ve indirme, Resmi talep sonuçları ve talep paketi. Maske: TC `123******01`, pasaport ilk 3 + son 2 (`X12****67`),
  uyruk açık.
- **URL'de kimlik yok (kullanıcı kararı):** TC/pasaport hiçbir adrese, çapaya (`#…`) ya da sorgu dizgisine girmez (tarayıcı geçmişi,
  erişim kayıtları). Kişi sayfaları opak kimlikle açılır (*benim kararım:* kişi anahtarının cihaz/merkez gizli tuzuyla HMAC-SHA256'sının
  ilk 16 hex'i); kimlikle arama formları **POST**. Telefonlu eski kayıtlar da aynı opak kimliğe geçer (davranış aynı, adres değişir).
- **Merkez araması (kullanıcı kararı):** panel.wificorrect.com müşteri aramasına TC, pasaport (+uyruk) ve oda numarası ile arama
  eklenir; aranan değer POST ile gider, sonuçlar maskeli.
- Ad/soyad personelin girdiği. Oturum `ek` alanı: `giris=misafir_kodu oda=<no> kod_bitis=<iso> ureten=<panel hesabı>`.
- **Saklama (kullanıcı kararı):** TC ve pasaport **5651 kayıtlarında ve resmi talep aramasında tam**; panel ekranlarında
  (kod listesi, bağlı kullanıcılar, kullanıcılar listesi) **maskeli**. Süre: 5651 kayıtlarıyla aynı (`retention_days`, bugün 730 gün).
  Kod dosyasında süresi dolup 30 gün geçen kayıt silinir. *Benim kararım* (30 gün).
- Resmi talep / `ctl ara`: `--tc`, `--pasaport [--uyruk XX]`, `--oda` arama türleri; panel Resmi talep formuna aynı alanlar.
  Oda araması `ek` alanındaki `oda=` üzerinden.
- **Aydınlatma metnine önerilen ek** (hukukçuya onaylatılmalı):
  - TR: "Konaklama/yurt hizmeti kapsamında internet erişimi için misafir koduyla giriş yapmanız hâlinde; adınız, soyadınız,
    T.C. kimlik numaranız ya da pasaport numaranız ve uyruğunuz, oda numaranız ve kodun geçerlilik süresi tesis personelince
    kaydedilir ve 5651 sayılı Kanun kapsamındaki trafik kayıtlarıyla birlikte kanuni süre boyunca saklanır."
  - EN: "If you sign in with a guest code for internet access as part of your accommodation, your name, surname, Turkish ID
    number or passport number and nationality, room number and the code's validity period are recorded by our staff and kept
    together with the traffic logs required by Turkish Law No. 5651 for the legally required period."

## Hedef dışı
- PMS entegrasyonu; NVİ (kimlik) doğrulaması; kodu SMS/e-posta ile misafire gönderme; personel hesapları (ayrı iş: "personel
  kodu ile giriş"); kupon/ücretli kod; Kafe/Diğer türlerinde misafir kodu.

## Anti-hedefler
- TC/pasaport/kodun denetim kaydına, panel çıktısına (maskesiz) ya da repoya girmesi.
- Kod tahminiyle (kaba kuvvet) internete çıkılabilmesi.
- Mevcut SMS akışının ve telefonlu 5651 kayıtlarının bozulması (eski kayıtlar ve arama aynen çalışır).
- Kod iptal/uzatmanın açık oturumlara yansımaması.

## Kısıtlar
- Panel ve portal betiksiz (CSP), mevcut tasarım dili. Rust, mevcut modüller (`portal.rs`, `panel.rs`, `ortak.rs`, `ayar.rs`, `ctl.rs`).
- Merkez değişikliği: `/api/eslesme` alanı + `cihaz.isletme_turu` sütunu + liste/müşteri sayfası; müşteri ekranlarında maskeleme, opak kişi kimliği ve TC/pasaport/oda araması (POST). Kurulumu kullanıcı yapar.
- Testler: birim testleri (cihazda) + merkez testleri; **cihazda sahte istemciyle uçtan uca** (kullanıcı şartı): TC ile kod üret →
  portaldan gir; pasaportla kod üret → gir; 1 haftalık kod → bitiş doğru; süre uzat → oturum uzar; iptal → oturum düşer.
  Uçtan uca denemede sahte istemci (`scripts/sahte-istemci.sh` kalıbı), gerçek SMS yok, deneme kodları sonunda silinir.

## Uygulayıcıya bırakılanlar
Modül/dosya adları, panel sayfa yerleşimi, fiş biçimi, çeviri metinlerinin tam hâli (yukarıda sabitlenenler hariç), test kurgusu.
