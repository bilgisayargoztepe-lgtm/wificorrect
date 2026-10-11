# Merkezde cihaz sağlık izleme ve uyarı — tasarım

Tarih: 2026-10-11 · Kullanıcı soruları atladı ("gerek yok spec yaz"); aşağıdaki kararlar **benim varsayımlarımdır**,
onayla kesinleşir.

## Neden

J1900 cihazlar zaman zaman donuyor; internet, tünel ya da bir servis düşünce bunu hizmet sağlayıcı ancak müşteri arayınca
öğreniyor. Bu sırada misafir interneti ve 5651 kaydı (yasal yükümlülük) durmuş olabilir. Amaç: sorun **müşteri fark etmeden**
hizmet sağlayıcının telefonuna SMS ile gelsin, Yönetim'de neyin bozuk olduğu tek bakışta görünsün.

Bugün olan: Yönetim müşteri listesi WireGuard el sıkışmasından "çevrimiçi / son eşitleme …" gösteriyor (`wc-durum`, 5 dk),
yedek ("son gün … GECİKTİ") ve eşitleme gecikmesini kırmızıyla işaretliyor. Eksik: kimseye haber gitmiyor; cihazın içi
(servisler, disk, 5651 kaydı, SMS) merkezde görünmüyor.

## Hedefler

1. **Cihaz sağlık raporu**: her cihaz 5 dk'da bir merkeze küçük bir rapor gönderir (`POST /api/saglik`, mevcut cihaz anahtarı
   kimliğiyle, `api.wificorrect.com`). İçerik:
   - sürüm, açık kalma süresi (uptime)
   - servisler: portal, panel, kaydedici, dnsmasq, wificorrect-guvenlik (active/değil)
   - kayıt diski doluluk %
   - 5651: son mühürlenen gün, son 1 saatte kayıt boşluğu/ENOBUFS olayı var mı
   - internet (WAN) IP'si var mı
   - SMS: deneme modu/eksik ayar
   - bağlı misafir sayısı
   - yavaşlatma hatası (`hiz_hata`) var mı
2. **Kurallar** (merkez, 5 dk'da bir değerlendirir); her biri bir **sorun** üretir:

   | Sorun | Koşul | Önem |
   |---|---|---|
   | Cihaz ulaşılamıyor | WireGuard el sıkışması > 10 dk (donma, elektrik, internet ya da tünel) | kritik |
   | Rapor gelmiyor | Tünel ayakta ama son rapor > 15 dk (yazılım takılmış) | kritik |
   | 5651 kaydı durdu | kaydedici aktif değil ya da son 1 saatte kayıt boşluğu | kritik |
   | Servis durdu | portal / panel / dnsmasq / güvenlik aktif değil | kritik |
   | Yedek gecikti | Mevcut kural (son gün > 2 gün önce) | uyarı |
   | Disk dolu | ≥ %90 | uyarı |
   | SMS çalışmıyor | Deneme modu ya da eksik ayar (misafir kod alamaz) | uyarı |
   | Lisans bitiyor | ≤ 15 gün kaldı | bilgi |

3. **SMS uyarısı**:
   - Bir sorun **10 dk boyunca sürerse** hizmet sağlayıcının telefon(lar)ına SMS gider. Örnek: "WifiCorrect 1537344 Göztepe Bilgisayar: Cihaz ulaşılamıyor (20:15'ten beri)".
   - Sorun düzelince tek SMS daha gider: "… düzeldi (35 dk sürdü)".
   - Aynı sorun için tekrar SMS gitmez.
   - "Bilgi" önemindekiler yalnızca günde bir kez, sabah 09:00'da toplu gider.
4. **Yönetim**:
   - ana sayfanın üstünde **Açık sorunlar** kutusu (müşteri, sorun, ne zamandan beri, önem);
   - müşteri listesinde sorun rozeti;
   - müşteri sayfasında son sağlık raporu ve sorun geçmişi;
   - **Uyarı ayarları** sayfası: telefon numaraları (en çok 3), SMS açık/kapalı, günlük SMS üst sınırı (varsayılan 20) ve bir deneme SMS'i gönderme düğmesi.

## Hedef dışı

- İşletme sahibine (müşteriye) uyarı göndermek — sonraki iş.
- E-posta, Telegram, WhatsApp — yalnızca SMS (NetGSM zaten var).
- Donanım bekçisi (watchdog) ile cihazı otomatik yeniden başlatmak — kullanıcı bu listeden çıkardı.
- Grafikler, uzun dönem metrik saklama (yalnızca son rapor + sorun geçmişi).
- Erişim noktası (AP) izleme.

## Anti-hedefler

- **SMS yağmuru**: titreyen bağlantı her dakika SMS atarsa başarısızlık sayılır. 10 dk bekleme, tekrar yok ve günlük üst sınır bunu önler. Üst sınıra ulaşılınca son bir "üst sınıra ulaşıldı" SMS'i gider ve gün boyunca başka SMS gitmez.
- Rapor göndermenin cihazda misafir trafiğini ya da 5651 kaydını yavaşlatması: gönderim kaydedici döngüsünü beklemez, ayrı süreçte koşar (`ctl merkez-eslesme` gibi).
- Raporda kişisel veri olması: rapor yalnızca sayılar ve durumlar içerir. Ad, telefon ve MAC adresi taşımaz.
- Merkez kapalıyken cihazın etkilenmesi: rapor gönderilemezse sessizce düşer, cihaz çalışmaya devam eder.

## Kısıtlar

- Merkez: Python stdlib, SQLite (`veri.py`), mevcut `api.py` kimlik doğrulaması ve hız sınırı, `yonetim.py` sayfa kalıbı.
  Değerlendirme işi: `wc-durum` gibi systemd timer (5 dk). wcpanel kullanıcısıyla çalışır, el sıkışmalarını `durum.json`'dan okur.
- SMS: merkezde **henüz NetGSM gönderici yok**. Cihazlara giden `sms_ayari` (usercode/password/msgheader) ile Python'da
  NetGSM gönderimi yazılır. `mock` açıksa SMS gönderilmez, yalnızca kayıt tutulur.
- Cihaz: Rust ikilisi; kaydedici 5 dk'da bir `systemd-run … ctl saglik` başlatır, istek 10 sn zaman aşımlıdır. Mevcut `crate::merkez::Http` kullanılır.
- Merkez kurulumu kullanıcıda (sudo, `merkez-guncelle.sh`). Cihaz kurulumu `gelistir.sh` ile yapılır, ağa dokunmadığı için ölü adam anahtarı gerekmez.

## Benim kararlarım (onayınıza)

- Kanal yalnızca SMS. Bekleme süresi 10 dk. Önem sınıfları tablodaki gibi. Günlük üst sınır 20.
- Rapor aralığı 5 dk. "Rapor gelmiyor" eşiği 15 dk. "Ulaşılamıyor" eşiği 10 dk (mevcut `CEVRIMICI_SN` ile aynı).
- Sorun kaydı ve geçmiş merkez veritabanında tutulur (`sorun` tablosu: müşteri, tür, başlangıç, bitiş, SMS gönderildi mi).
  Son rapor `saglik` tablosunda (cihaz başına tek satır).
- Serbest bırakılmış ya da askıdaki müşteriler için uyarı gitmez. Lisansı bitmiş müşteri için yalnızca "Lisans bitti" bilgisi gider.

## Uygulayıcıya bırakılanlar

Tablo ve alan adları, rapor JSON alan adları, SMS metinlerinin tam hâli, sayfa yerleşimi, test kurgusu.
