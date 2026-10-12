# Merkezden uzaktan güncelleme — tasarım

Tarih: 2026-10-12 · Kullanıcı soruları atladı ("gerek yok spec yaz"); aşağıdaki kararlar **benim varsayımlarımdır**,
onayla kesinleşir.

## Neden

Bugün yeni sürüm bir cihaza ancak elle girilerek kurulabiliyor: geliştirmede `gelistir.sh`, sahada `.deb` + SSH. Müşteri
sayısı arttıkça bu ölçeklenmez; hata düzeltmesi ve yasal (5651) değişiklikler cihazlara günlerce ulaşmaz. Amaç:
hizmet sağlayıcı Yönetim'den bir sürümü seçsin, cihazlar onu **kendiliğinden, misafiri rahatsız etmeden ve
bozulursa kendiliğinden geri dönerek** kursun.

## Hedefler

1. **Sürümler merkeze GitHub'dan gelir.** Yönetim → **Sürümler** sayfası, "GitHub'dan yeni sürümleri al" düğmesi:
   merkez herkese açık repodaki release'lerden (`release.yml`'in ürettiği) `wificorrect_<sürüm>_amd64.deb` ve
   `SHA256SUMS`'u indirir, hash'i denetler, paketi `/var/lib/wificorrect/merkez/surumler/`'e koyar. Liste: sürüm, tarih,
   boyut, sha256, kaç cihazda kurulu.
2. **Kanallar ve hedef sürüm.** İki kanal: **test** ve **kararlı**. Yönetici her kanal için bir hedef sürüm seçer.
   Her müşterinin bir güncelleme ayarı vardır: `kararlı` (varsayılan) / `test` / `kapalı`. Göztepe Bilgisayar (1537344)
   = `test`. Yeni sürüm önce test kanalına verilir, sorunsuzsa kararlıya.
3. **Cihaz hedefi öğrenir.** Merkez, cihazın 5 dk'lık sağlık raporuna verdiği cevapta (`/api/saglik`) ve günlük
   eşitlemede (`/api/eslesme`) `guncelleme: {surum, sha256, boyut, hemen}` döner. Cihazın kurulu paket sürümü
   (`dpkg-query`) hedeften farklıysa güncelleme başlar (eski sürüme dönüş dahil: `--allow-downgrades`).
4. **İndirme ve kurulum.** Cihaz paketi `POST /api/paket` (cihaz anahtarıyla kimlik, `api.wificorrect.com`, HTTPS)
   ile indirir, sha256'yı merkezden gelen değerle karşılaştırır, `apt-get install ./paket.deb` ile kurar (yeni bağımlılıklar
   Debian deposundan gelir). Kurulum ayrı bir systemd işinde (`systemd-run … ctl guncelle`) koşar; kaydedici/portal
   döngüsünü beklemez.
5. **Zaman penceresi.** Cihaz yeni hedefi yalnızca **03:00–06:00** arasında kurar. Yönetici müşteri sayfasındaki
   "Şimdi güncelle" ile `hemen` işaretini verirse sonraki sağlık raporunda (≤ 5 dk) kurar.
6. **Kendiliğinden geri dönüş (ölü adam).** Kurulumdan önce kurulu sürümün paketi saklanır
   (`/var/lib/wificorrect/paket/`, son 2 sürüm). Kurulum bitince 10 dk sonra `ctl guncelleme-denetle` çalışır:
   portal, panel, kaydedici, dnsmasq, güvenlik servisi aktif **ve** merkeze bir sağlık raporu başarıyla gitmiş olmalı.
   Değilse önceki paket geri kurulur; o sürüm cihazda "başarısız" diye işaretlenir ve merkez aynı sürümü yeniden
   istese de **kurulmaz** (yönetici "yeniden dene" diyene kadar).
7. **Görünürlük.** Yönetim müşteri listesinde sürüm sütunu (hedeften farklıysa sarı), müşteri sayfasında güncelleme
   geçmişi. Sağlık izlemeye yeni sorun: **Güncelleme başarısız** (uyarı önemi; mevcut SMS kuralları geçerli).
   Cihaz denetimi: `GUNCELLEME_BASLADI`, `GUNCELLEME_TAMAM`, `GUNCELLEME_HATA`, `GUNCELLEME_GERI_ALINDI`
   (sürüm, sebep). Merkez denetimi: `SURUM_ALINDI`, `KANAL_HEDEF`, `GUNCELLEME_HEMEN`.

## Hedef dışı

- Debian işletim sistemi güncellemeleri (zaten `unattended-upgrades`, yalnızca güvenlik).
- Merkezin kendisinin güncellenmesi (kullanıcı `merkez-guncelle.sh` ile yapmaya devam eder).
- İşletme sahibinin güncelleme zamanını seçmesi; cihaz panelinde güncelleme düğmesi.
- Ağ ayarı değiştiren güncellemeler için özel geçiş adımı (bugünkü postinst ne yapıyorsa o).

## Anti-hedefler

- **Cihazı merkezden koparmak**: güncelleme tüneli ya da merkez bağlantısını bozup cihaz bir daha ulaşılamaz olursa
  başarısızlıktır. 6. maddedeki 10 dk denetim + geri dönüş bunu önler.
- **Misafirin düşmesi**: güncelleme açık oturumları kapatmamalı (oturumlar nft'de ve `sessions.json`'da kalır;
  servisler yalnızca yeniden başlar). Kaydedici yeniden başlarken birkaç saniyelik `LOG_BOSLUK` kabul edilir.
- **Bütün cihazların aynı anda bozulması**: test kanalı + pencere + tek tek geri dönüş bunu önler.
- **Doğrulanmamış paket kurmak**: sha256 tutmazsa kurulmaz.

## Kısıtlar ve kararlarım

- **apt deposu yerine tek paket + hash** (benim kararım): depo kurmak GPG anahtarı, imzalama ve ek bir sunucu ayarı
  gerektirir. Bunun yerine güven zinciri şöyle: cihaz merkezi HTTPS (genel CA, `api.wificorrect.com`) ile tanır, hash
  bu kanaldan gelir, paket hash'le doğrulanır. Aynı güvenlik, daha az parça.
- Paket indirme mevcut API kimliğiyle (`cihaz_anahtari`); merkez yalnızca bağlı ve lisansı açık cihaza verir.
- Merkezde yeni tablolar: `surum` (sürüm, sha256, boyut, tarih), `kanal` (test/kararlı → hedef), müşteri satırında
  `guncelleme_kanali`; cihazın bildirdiği kurulu sürüm ve son güncelleme sonucu sağlık raporuna eklenir.
- GitHub'dan indirme `wcpanel` kullanıcısıyla (yazma yeri `/var/lib/wificorrect/merkez`), yalnızca
  `github.com/bilgisayargoztepe-lgtm/wificorrect` release'leri; sürüm adı `^\d+\.\d+\.\d+$`.
- **Paketsiz cihaz**: `dpkg-query` paketi bulamazsa (geliştirme kurulumu, `gelistir.sh`) cihaz güncellemez ve raporda
  `paketsiz` der. Canlı cihaz (1537344) bugün paketsizdir; ilk geçiş **bir kez elle** yapılır: ölü adam anahtarıyla
  `.deb` kurulur (ayar dosyaları paket tarafından ezilmez; 2026-10-08 karşılaştırmasına göre repodaki dosyalar cihazdakiyle aynı).
  Geliştirme sırasında `gelistir.sh` kullanılırsa müşteri `kapalı` kanala alınır.
- Yeni sürüm üretmek: `vX.Y.Z` etiketi → `release.yml` (bugünkü gibi). Cargo sürümü etiketle aynı olmalı.
- Testler: birim testleri (cihaz: hedef karşılaştırma, pencere, hash red, başarısız sürüm hatırlama, geri dönüş kararı;
  merkez: GitHub cevabı sahte, kanal/hedef, paket uç noktası yetkisi). Uçtan uca: canlı cihaz test kanalında
  önce 0.3.x → yeni sürüm, sonra bilerek bozuk bir sürümle (ör. servis başlamıyor) geri dönüşün çalıştığı gösterilir.

## Uygulayıcıya bırakılanlar

Tablo/alan adları, JSON alan adları, sayfa yerleşimi, saklanan eski paket sayısı, deneme sürümünün nasıl bozulacağı.
