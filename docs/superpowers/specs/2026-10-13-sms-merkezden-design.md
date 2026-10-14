# Doğrulama SMS'i merkezden — tasarım

Tarih: 2026-10-13 · Kullanıcı kararları: SMS merkeze taşınır (**NetGSM ve Twilio ikisi de**, 2026-10-13); merkeze ulaşılamazsa misafire "şu an kod gönderilemiyor"
denir (cihaz doğrudan NetGSM'i **denemez**). Diğer kararlar **benim varsayımımdır**, onayla kesinleşir.

## Neden

NetGSM API'si yalnızca izinli IP'lerden gelen istekleri kabul ediyor. Müşteriler normal sabit hatlarla (Türk Telekom,
Superonline …) bağlanacak ve **IP'leri dinamik** olacak. Bugün misafir kodu cihazdan doğrudan NetGSM'e gidiyor; dükkânın
sabit IP'si dışındaki her yerde NetGSM isteği reddeder ve misafir kod alamaz. 2026-10-13'te cihaz telefon hattındayken bu
yaşandı: misafirin isteği `OTP_HATA netgsm=30` ile düştü, NetGSM "izinsiz IP" e-postası gönderdi. Merkezin IP'si sabit ve
NetGSM'de tanımlı; SMS merkezden giderse cihaz hangi hatta olursa olsun çalışır.

## Hedefler

1. **Yeni merkez ucu `POST /api/sms`** (api.wificorrect.com, mevcut cihaz anahtarı kimliği): gövde
   `{cihaz_anahtari, telefon, kod}`. `telefon` E.164 rakamları (`905…`, yalnızca Türk numarası), `kod` 6 rakam.
   Merkez mesajı **kendi şablonundan** kurar (`WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir.`), cihazın serbest
   metin göndermesine izin yoktur. NetGSM **OTP** ucu (`/sms/send/otp`, cihazın bugün kullandığı XML biçimi) ve merkezdeki
   tek SMS ayarı (Yönetim → SMS ayarları: kullanıcı kodu, şifre, başlık, uygulama anahtarı, deneme modu) kullanılır.
   Cevap: `{sonuc: "gonderildi"|"mock"|"hata", kod: "<NetGSM kodu ya da AYAR|AG|SINIR|…>", is: "<jobID>"?}`.
2. **Yetki ve sınırlar (merkez):** yalnızca bağlı, üyeliği ve lisansı açık cihaz (`veri.hizmet_acik`). Cihaz başına
   günlük üst sınır = merkezde tutulan sayaç, varsayılan **300** (cihazdaki sistem tavanıyla aynı); aşılınca `hata/SINIR`.
   Yeniden deneme yok (çift SMS = çift ücret).
3. **Cihaz:** NetGSM yolu (`sms::send` → `send_netgsm`) merkeze istek atar. Zaman aşımı **10 sn**. Merkeze ulaşılamazsa,
   cevap bozuksa ya da `hata/AG`: misafire **"Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin."** — cihaz
   doğrudan NetGSM'i denemez. NetGSM hata kodları (50, 80, 100 …) bugünkü Türkçe karşılıklarıyla gösterilir.
   Merkez `mock` dönerse cihaz bugünkü gibi kodu günlüğe yazar (`MOCK OTP …`) ve yerelde doğrular.
6. **Twilio da merkezde (kullanıcı kararı):** sağlayıcı seçimi merkezde. Türk numaraları merkezdeki "Türk numaraları için
   sağlayıcı" ayarına göre (NetGSM varsayılan, ya da Twilio), yabancı numaralar Twilio açıksa Twilio'dan, değilse
   `hata/YABANCI` (cihaz bugünkü "Yabancı numaralara şu an SMS gönderilemiyor" metnini gösterir). Twilio Verify'da kodu
   Twilio üretir ve doğrular: `/api/sms` Twilio'ya giderse cevapta `saglayici: "twilio"` döner, cihaz kendi kodunu kullanmaz;
   doğrulama `POST /api/sms-dogrula {cihaz_anahtari, telefon, kod}` → `{onay: true|false}` ya da `hata/AG` (ağ hatasında
   misafirin deneme hakkı düşmez, bugünkü davranış). Twilio bilgileri (account SID, auth token ya da API key, Verify
   service SID, açık/kapalı) Yönetim → SMS ayarları'na taşınır; cihazdan `[twilio]` ve `[sms] provider` kalkar.
4. **Kayıtlar:** cihazdaki denetim olayları aynı (`OTP_ISTEK`, `OTP_GONDERILDI is=<jobID>`, `OTP_HATA netgsm=<kod>`;
   merkeze ulaşılamazsa `OTP_HATA netgsm=AG`). Merkez **kodu hiçbir yere yazmaz**; müşteri başına günlük sayaç ve
   `SMS_OTP` hareketi (telefonun son 2 hanesi + sonuç) tutar. Yönetim müşteri sayfasında "Bugün gönderilen doğrulama SMS'i".
5. **Sırların cihazdan çekilmesi:** cihaz NetGSM/Twilio bilgilerini artık kullanmaz; cihazdaki Admin ayarları'nda NetGSM
   ve Twilio alanları ve sağlayıcı seçimi kalkar, yerine "Doğrulama SMS'leri yönetim merkezinden gönderilir" notu.
   Deneme modu bilgisi merkezden gelmeye devam eder (Özet uyarısı ve sağlık raporu `sms_sorunu` için).

## Hedef dışı

- Merkez kapalıyken yedek yol (kullanıcı kararı: yok).
- Toplu/pazarlama SMS'i, müşteriye fatura.

## Anti-hedefler

- **Açık SMS rölesi:** çalınmış bir cihaz anahtarıyla merkezden istenen metin, istenen numaraya gönderilememeli →
  sabit şablon, yalnızca 6 haneli kod, yalnızca Türk numarası, cihaz başına günlük sınır, mevcut IP hız sınırı.
- **Kodun sızması:** doğrulama kodu merkezde günlüğe, veritabanına ya da harekete yazılmaz.
- **Çift SMS:** ne cihaz ne merkez yeniden dener.
- **Sessiz bozulma:** merkez SMS gönderemiyorsa (ayar eksik, NetGSM hatası) bu Yönetim'de görünmeli; sağlık izlemenin
  mevcut "SMS çalışmıyor" kuralı merkezdeki son NetGSM sonucunu da dikkate alır.

## Kısıtlar

- Merkez: Python stdlib (`api.py`, `veri.py`, `netgsm.py`); NetGSM isteği **WC_TEST** altında gerçekten gönderilmez (mevcut kural).
- Cihaz: Rust; merkez istemcisi `crate::merkez::cagir`/`curl_sure`.
- Geçiş sırası: önce merkez kurulur (yeni uç eski cihazları bozmaz), sonra cihaz yeni sürümü uzaktan güncellemeyle alır.
  Eski sürümlü cihazlar için merkez NetGSM bilgisini göndermeye bir süre daha devam eder; **tüm cihazlar yeni sürüme
  geçince** gönderim kapatılır (bu dalda: cihaz tarafı alanları kullanmaz; merkezde gönderimi kapatan ayar ayrı küçük adım).
- Gerçek SMS denemesi yalnızca kullanıcının açık onayıyla ve verdiği numarayla.

## Benim kararlarım (onayınıza)

- Cihaz başına günlük sınır 300; cihazdaki mevcut sınırlar (numara, MAC, sistem tavanı) aynen kalır.
- Merkez cevabı NetGSM kodunu olduğu gibi döner; Türkçe mesaja çeviri cihazda (bugünkü tablo).
- Eski cihazlara NetGSM sırlarını gönderme bu dalda kapatılmaz (canlı cihaz güncellenince ayrı adım).

## Uygulayıcıya bırakılanlar

Merkez sayaç tablosu adı, hareket ayrıntı biçimi, test kurgusu, Admin ayarları notunun tam yeri.
