# Kolay bağlama — uygulama planı

Spec: `docs/superpowers/specs/2026-10-16-kolay-baglama-design.md`. Sürüm 0.4.13. Kurulum sırası: merkez, sonra cihaz.

## Sözleşme (cihaz ↔ merkez)

- Cihazda `/etc/wificorrect/bekleme.json` (600): `{"kimlik": 32 hex, "sir": 64 hex}`; ilk `ctl bekliyor`da üretilir,
  fabrika dönüşünde de kalır (kalıcı kimlik). Sır yalnızca TLS içinde merkeze gider; merkez yalnızca SHA-256'sını tutar
  (ilk bildirim kaydeder = TOFU; kimlik rastgele 128 bit, cihazdan önce kimse bilmez).
- `POST /api/bekliyor` `{kimlik, sir, surum, wan_mac, wg_pub, ssh_pub}` → `{"durum":"bekle"}` ya da
  `{"durum":"baglan","belirtec":…}` (belirteç tek kullanımlık, 10 dk, merkezde özeti). Sır tutmazsa 401 `taninmadi`
  (kayıt değişmez). Hız: IP başına genel sınır (dakikada 20) + kimlik başına dakikada 3 + IP başına saatte 10 yeni kimlik;
  tabloda en çok 2000 kayıt.
- `POST /api/bagla` `{kimlik, sir, belirtec, wg_pub, ssh_pub, isletme_adi, unvan, surum}` → `/api/giris` cevabının
  aynısı + `numara`. Yan etkiler `/api/giris` ile ortak (`_bagla`: `cihaz_bagla`, kuyruk `cihaz-ekle`, tünel IP,
  `CIHAZ_BAGLANDI` (ayrıntıda `otomatik`), admin parolası ve SMS eki).

## Merkez

1. `veri.py`: `bekleyen` tablosu (kimlik, sir_ozet, ilk, son, ip, surum, wan_mac, wg_pub, ssh_pub, musteri, ayrilma,
   belirtec_ozet, belirtec_bitis, cihaz, baglandi; zamanlar epoch). İşlevler: `bekleyen_bildir`, `bekleyen_adaylar`,
   `bekleyen_ayir`, `bekleyen_musterinin`, `bekleyen_belirtec_tuket`, `bekleyen_baglandi`, `bekleyenler`.
   Kurallar: 7 gün bildirmeyen silinir; 30 dk bildirmeyen yönetimde "eski"; ayrılmış ama 15 dk'da bağlanmamış serbest;
   bağlı bir cihazın (wg_pub'u bağlı cihaz satırında) kaydı aday olmaz; bu kayıttan bağlanmış cihaz cevabı kaybettiyse
   (bağ duruyor, cihaz yine bildiriyor) yeniden "bağlan" alır (aynı müşteri, aynı wg_pub).
2. `api.py`: `/api/bekliyor`, `/api/bagla`; `giris`in bağlama kuyruğu `_bagla`ya. Hareket `CIHAZ_BEKLIYOR` (ilk görülme).
3. `musteri.py`: giriş sonrası cihazı yoksa (üyelik aktif) `/baglan`: müşterinin ayrılmış cihazı varsa "bağlanıyor"
   (meta refresh 5 sn); yoksa aynı IP'den son 10 dk'da bildiren adaylar: tek → ayır (`CIHAZ_OTOMATIK_AYRILDI`),
   hiç → "bulunamadı" + yeniden dene, çok → liste (ilk görülme, WAN MAC son 4, sürüm) + POST seçimi. Bağlandıktan
   sonra 30 sn "panel hazırlanıyor" (tünel kalksın), sonra `/cihaz`. `/cihaz` cihazsızken `/baglan`a. Oturum varken
   `/giris` → giriş sonrası sayfası (cihaz.wificorrect.com → panel girişi → doğrudan cihaz ya da bağlama).
4. `yonetim.py`: "Bekleyen cihazlar" (salt okunur).
5. Caddy: değişiklik yok (cihaz.wificorrect.com oturumsuzken zaten panel girişine yönlenir; IP `X-Forwarded-For` son
   öğesi, `api` ve `panel` aynı Caddy'den).

## Cihaz

1. `src/bekleme.rs`: kimlik dosyası, `bildir`, `bagla`, `calistir` (bekle/bağlan akışı, `MERKEZ_BEKLIYOR` yalnızca durum
   değişince — durum `/var/lib/wificorrect/bekleme-durum`), bağlanınca `MERKEZ_BAGLANDI yol=otomatik` + ek olayları.
2. `src/merkez.rs`: giriş cevabı ayrıştırma ortak (`giris_cevabi`), `kur` (ayar + bağ + hesaplar + uzak-uygula + portal):
   panelin yerel bağlaması da bunu kullanır; panel ve `ctl bekliyor` aynı dosya kilidini (`/etc/wificorrect/merkez.kilit`) alır.
3. `ctl bekliyor`; kaydedici bağlı değilken 2 dk'da bir `systemd-run --unit wfc-bekliyor … ctl bekliyor`.
4. Sürüm 0.4.13 (Cargo.toml + Cargo.lock).

## Testler

Merkez: `test_bekleme.py` (bildir/TOFU/hız/sınır, adaylar, ayırma, 15 dk serbest, belirteç tek kullanım, bağla uçtan
uca, kayıp cevapta yeniden bağlan, bağlı cihaz aday değil, müşteri sayfaları tek/hiç/çok, yönetim listesi). Cihaz:
`bekleme` birim testleri (sahte HTTP), kaydedici zamanlaması. Sonda tam merkez + tam cihaz takımı.
