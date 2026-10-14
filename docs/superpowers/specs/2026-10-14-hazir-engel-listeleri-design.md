# Hazır engel listeleri — tasarım

Tarih: 2026-10-14 · Kullanıcı soruları atladı ("gerek yok spec yaz"); aşağıdaki kararlar **benim varsayımlarımdır**,
onayla kesinleşir.

## Neden

Cihaz panelindeki **Yasaklı siteler** sayfasında işletme sahibi siteleri ("bet365.com") ve kelimeleri ("bet") tek tek
yazıyor; listeler boş başlıyor. Çoğu işletme sahibi ne yazacağını bilmez ya da hiç uğraşmaz. Yasa dışı bahis siteleri
sürekli yeni adla açılıyor; elle yazılan liste hep geride kalır. Amaç: tek tıkla açılan, merkezden güncel tutulan hazır
kategoriler. Misafirin bu siteleri işletmenin internetinden açması engellenir; işletme 5651 açısından korunur; satışta
somut bir özellik olur.

## Hedefler

1. **Kategoriler** (ilk sürüm):
   - **Kumar ve bahis**
   - **Yetişkin içerik**
   - **Zararlı ve dolandırıcı siteler** (kimlik avı, zararlı yazılım; USOM'un yayımladığı zararlı bağlantılar dahil)
   - **VPN ve proxy siteleri** (engelleri atlatmayı kolaylaştıran siteler)
2. **Cihaz paneli → Yasaklı siteler** sayfasının en üstünde "Hazır listeler" bölümü: her kategori için aç/kapa,
   kaç alan adı içerdiği ve son güncelleme tarihi. Değişiklik hemen geçerli, denetime `PANEL_HAZIR_LISTE kategori=… durum=acik|kapali`.
   Elle girilen siteler, kelimeler ve **istisnalar aynen kalır**; istisna listesine yazılan alan adı hazır listede olsa da
   açılır (yanlış engellenen meşru siteler için çıkış yolu). "Bu adres engelli mi?" denemesi hangi listeden engellendiğini söyler.
3. **Listeler merkezde hazırlanır:** merkez her gün açık kaynaklı listeleri indirir, kategori başına tek, temizlenmiş alan
   adı listesi üretir (geçersiz satırlar ve aşırı geniş girdiler — ör. `com`, `com.tr`, tek etiketli adlar — atılır;
   merkezdeki "asla engelleme" listesi uygulanır: wificorrect.com, netgsm.com.tr, twilio.com, google.com gibi sistemin ve
   giriş akışının ihtiyaç duyduğu adlar). Her kategori bir sürüm (sha256) taşır.
4. **Cihaz listeleri merkezden alır:** sağlık raporu cevabında (5 dk) kategori sürümleri gelir; açık olan bir kategorinin
   sürümü değiştiyse cihaz yeni listeyi indirir (cihaz anahtarıyla, sıkıştırılmış), doğrular ve uygular. Merkeze ulaşılamazsa
   eldeki son liste kullanılmaya devam eder. Kapalı kategoriler indirilmez.
5. **Uygulama DNS düzeyinde:** bugünkü site engeli gibi, alan adı ve bütün alt adları "bulunamadı" döner; sorgular 5651 DNS
   kaydına bugünkü gibi düşer. Liste değişince DNS kesintisiz yenilenmeli (dnsmasq'ı yeniden başlatmadan yeniden okutulabilen
   bir biçim tercih edilir; olmazsa yeniden başlatma en çok birkaç saniye sürmeli).
6. **Yönetim (merkez):** "Engel listeleri" sayfası: kategori başına kaynaklar, alan adı sayısı, son başarılı güncelleme,
   son hata. Bir kaynak indirilemezse eski liste kalır ve sağlık izlemede uyarı açılır.

## Hedef dışı

- HTTPS içeriğini inceleyen filtre (SNI/URL düzeyi); yalnızca DNS.
- DoH/şifreli DNS'i tamamen kapatmak (bilinen sınır; "VPN ve proxy" kategorisi bunu bir ölçüde zorlaştırır).
- İşletme başına özel kategori tanımlamak; yalnızca aç/kapa ve mevcut elle liste/istisna.
- Müşteriye (işletme sahibine) merkezden zorla açtırmak.

## Anti-hedefler

- **Meşru siteleri kırmak:** giriş akışı (portal, merkez, SMS sağlayıcıları), banka, devlet siteleri, büyük platformlar
  yanlışlıkla engellenirse başarısızlıktır → merkezde "asla engelleme" listesi + aşırı geniş girdi süzgeci + işletme
  istisnası.
- **Cihazı yavaşlatmak:** yüz binlerce satır DNS'i yavaşlatmamalı, belleği şişirmemeli (J1900, ~1,9 GB). Toplam alan adı
  sayısı ve dnsmasq bellek kullanımı ölçülür; kategori başına üst sınır konur.
- **İnternetsiz kalmak:** liste indirme ya da uygulama hatası misafirin internetini kesmemeli; hata olursa eski liste kalır.

## Kısıtlar ve kararlarım

- **Varsayılan:** dört kategori de **açık** gelir — yeni kurulumlarda ve güncellemeyle mevcut cihazlarda (canlı cihaz dahil).
  İşletme sahibi istediğini kapatabilir. (Gerekçe: yasal koruma ve "hiç uğraşmayan" işletme sahibi; ters tercih kolay.)
- **Kaynaklar** (açık lisanslı, düzenli güncellenen; lisans ve atıf belgeye yazılır):
  - kumar: HaGeZi "gambling" + Türkiye'ye özel ek (merkezde elle de eklenebilen küçük liste),
  - yetişkin: HaGeZi "nsfw" ya da StevenBlack "porn" eki,
  - zararlı: USOM zararlı bağlantılar listesi + HaGeZi "threat intelligence",
  - VPN/proxy: HaGeZi "doh-vpn-proxy-bypass".
  Uygulayıcı kaynakların güncel adreslerini ve lisanslarını doğrular; uygun olmayan kaynak yerine benzeri seçilir.
- Kategori başına üst sınır başlangıçta **300.000** alan adı; ölçüme göre değiştirilebilir.
- Merkez Python stdlib; cihaz Rust; liste aktarımı mevcut API kimliği ve sağlık raporu mekanizmasıyla. İstisnalar ve elle
  liste mevcut `ayarlar.toml [filtre]` ile aynı.
- Cihaza kurulum uzaktan güncellemeyle (yeni sürüm), merkeze kullanıcı `merkez-guncelle.sh`.

## Uygulayıcıya bırakılanlar

Dosya biçimleri ve yolları, dnsmasq yükleme yöntemi, indirme ucu adı, merkezde zamanlayıcı, sayfa yerleşimi, test kurgusu,
kaynakların tam adresleri.
