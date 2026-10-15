# Kolay bağlama — tasarım

Tarih: 2026-10-16 · Kullanıcıyla konuşularak (seçenek B: kendiliğinden bağlansın). "(benim kararım)" işaretli yerler sorulmadı.

## Neden ve akış

Bugün ISO'dan kurulmuş, bağlanmamış cihaz yalnızca yerel ağdan `https://<ip>:8443` bağlama sayfasıyla müşteriye bağlanır.
Müşteri cihazın adresini bilmez (monitör yok) ve kendinden imzalı sertifika uyarısıyla karşılaşır. İstenen akış:

1. Müşteri cihazı modemine takar (Ethernet 1 → modem). Cihaz internete çıkar çıkmaz merkeze **"bağlanmayı bekliyorum"**
   bildirir; birkaç dakikada bir tekrarlar.
2. Müşteri bize "taktım" der; biz yönetimde **Yeni müşteri** ile numara + parola oluşturup müşteriye iletiriz.
3. Müşteri **cihaz.wificorrect.com**'u açar, numarası ve parolasıyla girer. Müşterinin bağlı cihazı yoksa ve **müşterinin
   tarayıcısının internet adresiyle aynı adresten** bekleyen tek cihaz varsa cihaz **kendiliğinden** bu müşteriye bağlanır;
   müşteri "Cihazınız bağlanıyor…" sayfasını görür (kendini yeniler), bağlanınca doğrudan cihaz paneline geçer.
4. Müşteri Ayarlar'dan işletme adı, unvan, tür, adres vb.'yi kendisi girer. AP takılınca biz AP kurulumu ile ayarlarız.

## Hedefler

1. **Cihaz tarafı (bağlanmamışken):** kalıcı rastgele cihaz kimliği + yalnızca cihazda duran gizli bekleme anahtarı
   üretir (ilk açılışta, `/etc/wificorrect/`, 600). Bağlanmamış ve merkeze ulaşılabilirken **2 dk'da bir** merkeze
   `bekliyor` bildirimi: kimlik, gizli anahtarın kanıtı, yazılım sürümü, WAN MAC'i, WireGuard ve yedek açık anahtarları
   (bağlama için gerekenler — bugünkü bağlama akışının istediği alanlar). Cevapta merkez "bekle" ya da "bağlan" der;
   "bağlan" gelince cihaz bugünkü bağlama işlemini (tünel, yedek hesabı, admin parolası, eşleşme) **parolasız**, merkezin
   bu cihaza özel verdiği tek kullanımlık bağlama belirteciyle tamamlar. Yerel bağlama sayfası yedek yol olarak kalır.
2. **Merkez:** bekleyen cihazlar tablosu (kimlik, son görülme, internet adresi, sürüm, WAN MAC, açık anahtarlar); 30 dk
   bildirmeyen kayıt "eski" sayılır, 7 gün sonra silinir (benim kararım).
3. **Kendiliğinden eşleştirme (müşteri girişinde):** müşterinin bağlı cihazı yoksa, son **10 dk** içinde müşterinin
   tarayıcısıyla **aynı internet adresinden** bildirim yapan, henüz kimseye ayrılmamış bekleyen cihazlara bakılır:
   - **Tek cihaz** → bu müşteriye ayrılır (hareket `CIHAZ_OTOMATIK_AYRILDI`); cihaz bir sonraki bildiriminde "bağlan" alır.
     Müşteriye "Cihazınız bağlanıyor, bu sayfa kendini yeniler" sayfası; bağlanınca cihaz paneline geçer.
   - **Hiç yok** → "Cihazınız bulunamadı. Cihazın modeme takılı ve açık olduğundan, bu sayfayı cihazla aynı internetten
     (işletmenin Wi-Fi'ı ya da kablosu; mobil veri değil) açtığınızdan emin olun." + yeniden dene.
   - **Birden çok** → (benim kararım) yanlış eşleşme riski olduğu için kendiliğinden bağlanmaz; liste (ilk görülme saati,
     WAN MAC'in son 4 hanesi, sürüm) gösterilir, müşteri seçer.
   - Ayrılmış ama 15 dk içinde bağlanmamış cihaz serbest kalır (benim kararım).
4. **cihaz.wificorrect.com'dan doğrudan giriş:** müşteri cihaz.wificorrect.com'u açınca giriş ekranı gelir (bugünkü
   panel.wificorrect.com girişi); girince eşleştirme adımı (3) ve ardından cihaz paneli. panel.wificorrect.com girişi
   aynen çalışmaya devam eder.
5. **Yönetim:** "Bekleyen cihazlar" listesi (salt okunur: son görülme, internet adresi, sürüm, WAN MAC, ayrıldığı müşteri)
   — kurulumda neyin olduğunu görmek için. Elle bağlama düğmesi yok (benim kararım; gerekirse sonra).
6. **Denetim:** cihaz `MERKEZ_BEKLIYOR` (yalnızca durum değişince), bağlanınca bugünkü bağlanma olayları; merkez
   `CIHAZ_BEKLIYOR` (ilk görülme), `CIHAZ_OTOMATIK_AYRILDI`, `CIHAZ_BAGLANDI`.

## Hedef dışı

- Yönetimden elle "bu müşteriye bağla" (seçenek A).
- Cihaz ekranında kod gösterme / QR.
- Bağlı cihazı başka müşteriye taşıma (bugünkü "zorla ayır" yeterli).

## Anti-hedefler / güvenlik

- **Başkasının cihazını kapmak:** eşleşme yalnızca aynı internet adresinden ve yalnızca bekleme durumundaki cihaz için;
  "bağlan" cevabı yalnızca o cihazın gizli bekleme anahtarını kanıtlayan bildirime verilir (kimliği bilen sahte istemci
  cihazın yerine bağlanamaz). Bekleme bildirimleri hız sınırlı.
- **Yanlış müşteriye bağlanmak:** birden çok aday varsa kendiliğinden bağlanmaz.
- **Bağlı cihazı bozmak:** bağlı cihaz bildirim göndermez; merkez bağlı bir cihaz kimliğini yeniden ayırmaz.
- **Kalan risk (bilerek kabul; ürün kararı: kendiliğinden bağlama kalır):** güven sınırı "aynı internet adresi"dir. Aynı
  dış adresi paylaşan biri (müşterilere açık modem Wi-Fi'ı, kurulum sırasında; aynı CGNAT çıkışı) hesabı olmadan sahte bir
  "cihaz" bildirebilir; sahip, gerçek cihaz bildirmeden önce girerse ve tek aday sahteyse sahte cihaz müşterinin hesabına
  bağlanır. Sahte cihazın alabildikleri en aza indirildi: `/api/bagla` cevabı ve bu yolla bağlanmış cihazın sonraki
  eşitlemeleri (`/api/eslesme`) bütün cihazlarda ortak NetGSM bilgisini (`mock`/`provider`/`netgsm`) **taşımaz** — 0.4.13+
  cihaz yalnızca `etkin_mock` okur (cihaz satırında `otomatik=1`). Kalanlar bağlamanın kendisi için gerekir: müşteri parola
  özeti (cihaz cevabı onsuz kabul etmez, yerel panel girişi bununla doğrulanır; zaten `/api/eslesme` cihaz anahtarıyla aynı
  özeti döner), ortak admin özeti (bağlanır bağlanmaz admin girişi; eşitlemede de gelir), cihaz anahtarı, tünel ve yedek
  hesabı (müşterinin `/api/sms` kotası, raporları, `wfc-<numara>` arşivine yazma). Parolalı `/api/giris` (eski cihazlar)
  değişmedi. **Azaltma:** Yönetim → Bekleyen cihazlar (adres, WAN MAC, sürüm, durum) ve hareketler (`CIHAZ_BEKLIYOR`,
  `CIHAZ_OTOMATIK_AYRILDI`, `CIHAZ_BAGLANDI … otomatik kimlik=…`, reddedilen denemeler `CIHAZ_BEKLIYOR_RED` /
  `CIHAZ_BAGLA_RED`) neyin bağlandığını gösterir; gerçek cihaz hep `bekle` alıyorsa yönetimde "zorla ayır" sahte bağı
  kaldırır, müşteri yeniden girince gerçek cihaz bağlanır. Tek aday için onay düğmesi (sessiz kapmayı önler) ileride ürün
  kararıyla eklenebilir.

## Kısıtlar

- Merkez Python stdlib, cihaz Rust; mevcut `api/giris` bağlama akışının yan etkileri (kuyruk: tünel + yedek hesabı, admin
  parolası, eşleşme eki) aynen yeniden kullanılır.
- Müşterinin internet adresi Caddy arkasından gelir (`X-Forwarded-For` / Caddy'nin verdiği gerçek istemci adresi; bugünkü
  fail2ban/hız sınırı hangi adresi kullanıyorsa o).
- Sürüm 0.4.13 (0.4.12'nin üstüne). Kurulum sırası: önce merkez, sonra cihaz. ISO yeniden üretilir (yeni cihazlar
  kutudan çıkınca bildirsin).

## Uygulayıcıya bırakılanlar

Uç nokta adları ve biçimleri, gizli anahtar kanıtının biçimi (ör. HMAC'li zaman damgası), bağlama belirtecinin
biçimi/süresi, cihazda bildirimi hangi servisin yaptığı (kaydedici/zamanlayıcı), Caddy değişikliği, sayfa metinleri.
