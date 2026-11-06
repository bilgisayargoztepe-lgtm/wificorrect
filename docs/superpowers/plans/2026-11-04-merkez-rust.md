# Merkezi Rust'a taşıma ve kod sadeleştirme — uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Yönetim merkezini (Python, `scripts/sunucu/merkez/`) aynı davranışla Rust'a taşımak; Python'u projeden çıkarmak;
merkezde ve cihazda ölü/eski kodu silmek.

**Architecture:** Ayrı Rust crate `merkez/` (ikili `wificorrect-merkez`, alt komutlar). Python modülleri 1:1 Rust modüllerine
geçer; uygulamalar saf `istek(&Istek) -> Yanit` arayüzüyle yazılır ve Python testleri birebir Rust testine taşınır. Python
testleri çalışırken her isteğin cevabı ve veritabanının son hâli "altın kayıt" olarak saklanır; Rust'taki aynı test aynı
cevapları vermelidir. Canlıya geçiş aynı port, aynı birim adları ve aynı veritabanıyla yapılır; geri dönüş tek komuttur.

**Tech Stack:** Rust 1.85 (Debian 13 cihazı ve CI), tiny_http 0.12, rusqlite (bundled), serde_json, chrono, sha2, pbkdf2,
hmac, flate2, getrandom, libc; dış istekler curl alt süreciyle. Geçici olarak Python 3 (yalnızca altın kayıt üretimi).

**Spec:** `docs/superpowers/specs/2026-11-04-merkez-rust-design.md`

## Global Constraints

- Rust sürümü **1.85**, `edition = "2021"`, `rust-version = "1.85"`; kök `.cargo/config.toml` (`incompatible-rust-versions =
  "fallback"`) `merkez/` için de geçerli. Crate: `merkez/Cargo.toml`, paket ve ikili adı `wificorrect-merkez`, kendi
  `merkez/Cargo.lock`'u (repoda tutulur). Release profili cihazınkiyle aynı (`opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `strip = true`).
- Cihaz kodu (`src/`, `deploy/`, `paket/`, `iso/`, kök `Cargo.toml`) **Task 15'e kadar değişmez**. Merkez `src/sablon/logo/`
  dosyalarını yalnızca okur (`include_str!`/`include_bytes!`).
- Veritabanı şeması, tablo/sütun adları, dosya yolları ve dosya biçimleri Python'la **aynı**. Sorgular sütunlara **adla**
  erişir: canlı veritabanında sonradan eklenen sütunlar sonda durur.
- Aynı alan adları, yollar, çerez adları ve öznitelikleri (`HttpOnly; Secure; SameSite=Strict; Path=/`), güvenlik
  başlıkları, istek gövdesi sınırı **10 000 bayt** (aşarsa 413), dinleme adresi **127.0.0.1:8081**.
- fail2ban günlük satırları: stderr'e `GIRIS_HATALI alan=<rol> kullanici=<repr> ip=<ip>` ve
  `GIRIS_KILITLI alan=<rol> kullanici=<repr> ip=<ip>`; satır `ip=<ip>` ile biter.
- Parola özeti: PBKDF2-HMAC-SHA256, tuz metninin baytları, hex; yineleme **120 000**. Python'un ürettiği özetler Rust'ta
  doğrulanmalı (ve tersi).
- Saat dilimi sabit **+03:00**. `now_iso` biçimi `2026-11-04T03:17:00+03:00` (saniye hassasiyeti).
- JSON dosyaları `save_json` gibi yazılır: geçici dosya + fsync + yer değiştirme, UTF-8 (kaçışsız), girinti 1.
- Testlerde ağ yok: gerçek SMS/NetGSM/Twilio/GitHub/liste kaynağı isteği gitmez (sahte taşıyıcı). `WC_TEST=1` ortamında curl
  taşıyıcısı istek göndermeden hata döner.
- Sırlar (NetGSM parolası, Twilio anahtarı, müşteri/admin/AP parolaları) komut satırına, günlüğe ve hata metnine yazılmaz.
  curl'e gövde ve kimlik stdin'den (`-K -` ya da `--data-binary @-`) verilir.
- Kod tanımlayıcıları ve yorumlar cihaz kodunun üslubunda: Türkçe adlar, kısa ve gerekli yorumlar.
- Cihazda test: `bash scripts/test-merkez.sh wificorrect [süzgeç]`. Aynı anda iki test (cihaz ya da merkez) çalıştırılmaz.
- Altın kayıtlar `merkez/tests/altin/` altında; yalnızca Python'dan üretilir (`py -3 -I scripts/sunucu/parite/kaydet.py`),
  elle düzenlenmez.
- Canlı sunucuya ve canlı cihazlara yalnızca Task 0, 14, 15 ve 16'da, kullanıcının çalıştırdığı sudo'lu adımlarla dokunulur.

## Review Focus

1. **Python'un oluşturduğu canlı veritabanı:** sütunlar `ALTER` sırasıyla sonda; eski satırlar var (üyeliği bitmiş Deneme,
   `bocafe-test` arşivi, boş isteğe bağlı alanlar). Rust bunları adla okumalı. Task 2, Python'a oluşturulup göç ettirilmiş bir
   veritabanı dosyasıyla test eder.
2. **Form ve sorgu çözme Python'la aynı olmalı** (`urllib.parse.parse_qsl` + `dict`, `parse_qs(...)[0]`):
   - `+` → boşluk, `%XX`, geçersiz `%`, aynı anahtarın tekrarında sonuncusu;
   - boş değer, UTF-8 olmayan bayt.
   Bunlar Task 3'te test edilir.
3. **Aynı veritabanına eşzamanlı erişim:** sunucu ile `saglik`/`listeler`/`surumler` süreçleri birlikte yazar. 5 sn bekleme
   (busy timeout) olmalı ve uzun yazma işlemi olmamalı. Task 2'de iki bağlantıyla test edilir.
4. **TR gün sınırları:** SMS sayacı, lisans tarihi, hafta ve ay raporu, sağlık "bugün" hesabı 23:59:59 ve 00:00:00 +03:00
   anlarında doğru gün vermeli. Task 1 ve 6'da test edilir.
5. **Geri dönüş uyumu:** Rust'ın yazdığı her dosya ve veritabanı satırı Python tarafından okunabilmeli. Task 13'teki denetim,
   Rust çıktısını Python okuyucularıyla açan bir test içerir.

---

## Çalışma düzeni

- **Dal ve CI:** iş `merkez-rust` dalında yürür. Her görev kendi commit'iyle biter; CI için dal push'lanır. `main`'e
  birleştirme yalnızca kullanıcının "birleştir" demesiyle olur.
- **DEVAM.md:** bekleyen değişikliğine bu iş dokunmaz. Belgeler Task 16'da güncellenir.
- **Altın kayıt farkları:** yalnızca spec'teki "Bilinçli farklar" listesi `altinsiz` olabilir. Başka her fark ya düzeltilir ya
  da denetleyici ruling'iyle listeye eklenir (defterde gerekçesiyle).
- **Eski kod silindiği için çıkan fark hatadır.** Örnek: `_gocur` ya da common.py silinince değişen bir sayfa.
- **Task 0 sonucu:** Task 2'den önce alınır. Sonuç yoksa varsayılan, eski veri düzeltmelerini ve okuyucuları **taşımaktır**
  (fazla kod zararsız, eksik kod canlıda hata verir).

### Task 0: Canlı envanter (denetleyici + kullanıcı)

Ajan görevi değil. Task 2'den önce yapılır. Denetleyici yazar, kullanıcı sudo ile çalıştırır; sonuç defterine yazılır, Task 2
ve 6'daki kararları belirler.

**Files:**
- Create: `scripts/sunucu/envanter.sh` (geçici; Task 16'da silinir)

- [ ] **Step 1: Betiği yaz.** Salt okunur. Yazdıkları:
  - `merkez.db`'nin `PRAGMA table_info` sütun listeleri ve tablo başına satır sayısı;
  - boş `lisans_bitis`, boş `ap_parola` ve boş `parola_acik` müşteri sayısı;
  - `ayar` tablosundaki anahtar adları (değer yok);
  - `/var/lib/wificorrect/rapor/*/*.json` ve `rapor-ham/*/*.json` içinde `"bicim"` değerlerinin sayımı;
  - `detay`, `istatistik` ve `kuyruk*` dizinlerindeki dosya sayısı;
  - `/srv/hotspot-arsiv` ve `/srv/wificorrect-arsiv` gün sayısı.
  Kişisel veri, parola ve değer basmaz.
- [ ] **Step 2: Kullanıcı çalıştırır.** Görünür PowerShell penceresinde: `ssh -t ... 'sudo bash /tmp/envanter.sh'`. Çıktı
  defterine (`.superpowers/sdd/.../progress.md`) yazılır.
- [ ] **Step 3: Kararlar.**
  - Boş `lisans_bitis`/`ap_parola` > 0 ise Task 2 `_gocur` veri düzeltmesini taşır, yoksa taşımaz.
  - Biçim 1 rapor dosyası > 0 ise Task 6 biçim 1 okumasını taşır, yoksa taşımaz.

### Task 1: İskelet, ortak yardımcılar, güvenlik

**Files:**
- Create: `merkez/Cargo.toml`, `merkez/Cargo.lock`, `merkez/src/main.rs`, `merkez/src/ortak.rs`, `merkez/src/guvenlik.rs`
- Create: `scripts/test-merkez.sh`
- Modify: `.github/workflows/test.yml` (yeni iş `merkez-rust`; Python işi kalır)
- Test: `merkez/src/guvenlik.rs` ve `merkez/src/ortak.rs` içindeki `#[cfg(test)] mod tests`
- Taşınan Python testleri: `tests/test_guvenlik.py` (6)

**Interfaces:**
- Produces:
  - `ortak::TZ: chrono::FixedOffset` (+03:00), `ortak::simdi() -> f64`, `ortak::now_iso(ts: f64) -> String`.
  - `ortak::tarih_goster(iso: &str, saniye: bool) -> String`, `ortak::tarih_saat_goster(iso: &str) -> String`,
    `ortak::tarih_girdi(s: &str) -> String`.
  - `ortak::save_json(yol: &Path, v: &serde_json::Value) -> io::Result<()>`.
  - `ortak::read_rows(yol: &Path) -> Vec<BTreeMap<String, String>>` (`;` ayraçlı, BOM'lu, `.gz` ise açar, dosya yoksa boş).
  - `ortak::normalize_phone(raw: &str) -> Option<String>`.
  - `ortak::{DisIstek, Tasiyici, Curl}`:
    - `trait Tasiyici: Send + Sync { fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String>; }`;
    - `DisIstek { yontem, url, basliklar: Vec<(String, String)>, govde: Option<Vec<u8>>, kullanici: Option<(String, String)>,
      zaman_asimi: Duration, okuma_siniri: usize }`;
    - HTTP 4xx/5xx hata değil `(durum, gövde)` döner (`saglayici_istegi` anlamı).
  - `guvenlik::{YINELEME = 120_000, EN_AZ = 10, HARFLER, numara_gecerli(&str) -> bool}`.
  - `guvenlik::{ozet(pw, tuz, yineleme) -> String, yeni_kayit(pw) -> (String, String, u32),
    dogru(pw, tuz, oz, yineleme) -> bool, bos_dogrulama(pw), parola_uret(n) -> String}`.
  - `guvenlik::{Oturumlar, Oturum, GecisBelirtecleri, TekKullanimlik<T>, GirisKilidi, HizSiniri}`: Python sınıflarının aynı
    yöntemleri ve süreleri (boşta 1800 sn, en çok 43 200 sn, geçiş 60 sn, kilit 5/900 sn, hız 20/60 sn).
  - `guvenlik::rastgele::{token_urlsafe(n) -> String, token_hex(n) -> String, token_bytes(n) -> Vec<u8>,
    randbelow(n: u64) -> u64, choice(alfabe: &str) -> char}`: bütün rastgelelik buradan geçer.
  - Yalnızca `cfg(test)`: `guvenlik::rastgele::belirlenimli()`. Bu kip açıkken her işlev kendi sayacıyla şu baytları üretir:
    `sha256("<tür>:<sayaç>:<blok>")` blokları birleştirilip `n` bayta kırpılır. Türler: `urlsafe`, `hex`, `bytes`, `below`,
    `choice`. Dönüşümler:
    - urlsafe: base64url, dolgusuz;
    - hex: küçük harf;
    - below: ilk 8 bayt büyük sonlu `u64 % n`;
    - choice: `alfabe[below(len)]`.
    Sayaçlar 1'den başlar. Task 4'teki Python kaydedicisi aynı algoritmayı kullanır.

- [ ] **Step 1: Crate'i kur.**
  - `merkez/Cargo.toml`: Global Constraints'teki paket ayarları; bağımlılıklar tiny_http 0.12, serde (derive), serde_json,
    rusqlite (bundled), chrono (yalnız gerekli özellikler), sha2, pbkdf2 (hmac), hmac, flate2, getrandom 0.2, libc.
  - `main.rs`: `surum` komutu ve bilinmeyen komutta kullanım metni.
  - `scripts/test-merkez.sh`: `scripts/test.sh` kalıbında. Tar'a girenler: `merkez`, `.cargo`, `src/sablon/logo`. Cihazdaki
    dizin: `/root/rza-merkez`. Sonra `cd merkez && cargo test -j 2 $süzgeç`; `MemoryMax=1200M`, `nice -n 10`,
    `timeout 1500`. `merkez/Cargo.lock` cihazdan geri alınır.
- [ ] **Step 2: Testleri yaz.**
  - `test_guvenlik.py`'nin 6 testini birebir taşı.
  - Ek testler:
    - Python'un `hashlib.pbkdf2_hmac` ile ürettiği bir özet (testte sabit değer) Rust'ta `dogru` olmalı.
    - `tarih_goster`/`tarih_girdi` docstring örnekleri.
    - `now_iso(1_790_000_000.0)`, 23:59:59 ve 00:00:00 +03:00 sınırları.
    - `normalize_phone`: `+90 5xx`, `05xx`, `5xx`; geçersiz → `None`.
    - `read_rows`: BOM, `.gz`, eksik dosya.
    - `save_json`: girinti 1, Türkçe karakter kaçışsız.
    - Belirlenimli kipte `token_urlsafe(32)` iki kez farklı değer, uzunluk 43.
- [ ] **Step 3: Çalıştır, başarısız olsun.** `bash scripts/test-merkez.sh wificorrect` → derleme ya da test hatası.
- [ ] **Step 4: `ortak.rs` ve `guvenlik.rs`'yi yaz.** Python karşılıkları: `common.py` (yalnızca listedeki 9 yardımcı;
  `saglayici_istegi` → `Tasiyici`) ve `guvenlik.py`. `Curl`:
  - `curl -sS --max-time <sn> -K -` kullanır; başlıklar, kullanıcı ve URL stdin'deki yapılandırmadan, gövde
    `--data-binary @-` ile ya da yapılandırma içinde verilir;
  - `-w '%{http_code}'` ile durum alınır, cevap `okuma_siniri` baytla kesilir;
  - `WC_TEST=1` ise çalıştırmadan `Err("WC_TEST")` döner.
- [ ] **Step 5: Çalıştır, geçsin.** `bash scripts/test-merkez.sh wificorrect` → `test result: ok`, uyarı yok.
- [ ] **Step 6: CI.** `test.yml`'ye `merkez-rust` işini ekle (ubuntu-24.04, `dtolnay/rust-toolchain@1.85`,
  `Swatinem/rust-cache@v2` ile `workspaces: merkez`, `cd merkez && cargo test --locked`).
- [ ] **Step 7: Commit.** `git add merkez scripts/test-merkez.sh .github/workflows/test.yml && git commit -m "merkez: Rust
  iskeleti, ortak yardımcılar, güvenlik"`

### Task 2: Veritabanı

**Files:**
- Create: `merkez/src/veri.rs`, `merkez/src/twilio.rs` (yalnızca `SID` doğrulayıcıları ve `kimlik(ayar)`; ağ kısmı Task 8)
- Create: `merkez/tests/veri/python-gocmus.db`. Python'un `veri.Veri` ile oluşturup bütün `YENI_SUTUNLAR`'ı `ALTER` ile
  eklettiği küçük bir veritabanıdır; örnek satırlar uydurma. Üreten betik `scripts/sunucu/parite/eski_vt.py` (geçici).
- Taşınan Python testleri: `tests/test_veri.py` (13'ten 11'i)

**Interfaces:**
- Consumes: Task 1 `guvenlik::*`, `ortak::now_iso`, `ortak::TZ`.
- Produces:
  - `veri::Saat = Arc<dyn Fn() -> f64 + Send + Sync>`.
  - `veri::Veri::ac(yol: &Path, saat: Saat) -> Result<Veri, String>`:
    - `PRAGMA foreign_keys = ON` ve `PRAGMA secure_delete = ON`;
    - tam şema: `SEMA` artı Python'da yalnızca `ALTER` ile gelen bütün sütunlar `CREATE TABLE` içinde;
    - `sema_denetle`; günlük modu değişmez;
    - tek bağlantı `Mutex` içinde.
  - `veri::sema_denetle(&rusqlite::Connection) -> Result<(), String>`: gerekli her tablo ve sütun var mı (Task 13'te de
    kullanılır).
  - Yöntemler `veri.py`'deki **aynı adlarla** (Task 2'nin defter kaydındaki liste), satır türleriyle: `Musteri`, `Cihaz`,
    `Hareket`, `Sorun`, `SurumKaydi`, `Bekleyen`, `SmsSon`. Alanlar tablo sütunlarının adları ve türleri.
  - `veri::{sms_hazir(&Value) -> bool, anahtar_ozeti(&str) -> String}`.
  - Sabitler: `NUMARA_ARALIGI`, `BEKLEYEN_*`, `AYRILMA_SN`, `BELIRTEC_SN`, `CIHAZ_ALANLARI`, `ISLETME_TURLERI`,
    `AP_PAROLA_UZUNLUK`, `GUNCELLEME_KANALLARI`, `EN_SON`, `UYARI_*`, `SMS_ALANLARI`, `TR_SAGLAYICILAR`, `TWILIO_*`,
    `LISANS_GUN`, `RAPOR_GRUPLARI`.
  - `veri.py`'den farklar:
    - `parola_sifirla` taşınmaz (testler `parola_koy` + `parola_uret` kullanır);
    - `ALTER` geçişleri taşınmaz;
    - `_gocur`'un veri düzeltmeleri Task 0'ın kararına göre.

- [ ] **Step 1: Eski veritabanı örneğini üret.** `py -3 -I scripts/sunucu/parite/eski_vt.py merkez/tests/veri/python-gocmus.db`.
  Betik `veri.SEMA`'yı `YENI_SUTUNLAR`'sız bir sürümle önce oluşturur, sonra `veri.Veri` açtırıp `ALTER`'ları uygulatır ve
  her tabloya uydurma satır ekler.
- [ ] **Step 2: Testleri yaz.**
  - `test_veri.py`'nin testlerini birebir taşı. Taşınmayanlar ve yerine konanlar:
    - `test_eski_veritabani_gocu` ve `test_eski_cihaz_tablosuna_admin_sutunlari_eklenir` taşınmaz.
    - Yerine `python_gocmus_veritabani_okunur`: `python-gocmus.db` kopyası açılır, bütün okuyucular her satırı okur,
      `sema_denetle` tamam.
    - Yerine `eksik_sutun_sema_denetiminde_hata`.
  - Ek testler:
    - `eszamanli_iki_baglanti`: iki `Veri` aynı dosyada sırayla yazar, beşinci saniyeden önce kilit hatası yok.
    - TR gün sınırında `bugun()`.
- [ ] **Step 3: Çalıştır, başarısız olsun.** `bash scripts/test-merkez.sh wificorrect veri::`
- [ ] **Step 4: `veri.rs`'yi yaz.** `veri.py`'yi sorgu sorgu taşı (aynı SQL, aynı işlem sınırları: `_islem` = tek kilit +
  `BEGIN`/`COMMIT`).
- [ ] **Step 5: Çalıştır, geçsin.** Aynı komut → `test result: ok`.
- [ ] **Step 6: Commit.** `git commit -m "merkez: veritabani (veri.rs)"`

### Task 3: Web tabanı, alan adı dağıtıcısı, HTTP sunucusu

**Files:**
- Create: `merkez/src/web.rs`, `merkez/src/merkez.rs`, `merkez/src/sunucu.rs`, `merkez/src/detay.rs` (bu görevde
  yalnızca `e()`; kalanı Task 5)
- Taşınan Python testleri: `tests/test_merkez.py` (8; `test_soket_zaman_asimi` aşağıda)

**Interfaces:**
- Consumes: Task 1, Task 2 (`Veri`, `Saat`).
- Produces:
  - `web::Istek { yontem: String, yol: String, basliklar: Basliklar, govde: String, sorgu: String, ip: String }`;
    `Basliklar` büyük/küçük harf duyarsız `get(&str) -> Option<&str>`.
  - `web::Yanit { durum: u16, basliklar: Vec<(String, String)>, govde: Vec<u8> }`; aynı ad birden çok satır olabilir
    (Set-Cookie).
  - `web::form(govde: &str) -> HashMap<String, String>` (parse_qsl + dict) ve `web::sorgu_ilk(sorgu, anahtar) -> Option<String>`
    (parse_qs[0]).
  - `web::{e, kart, bilgi_tablosu, bolum, acilir, yanit_html(govde, durum, cerez), yonlendir(yer, cerez), guvenli_donus,
    cerez_degeri, cerceve, log, CSS, LOGO_ACIK, LOGO_KOYU, FAVICON}`: `web.py`'nin birebiri. Logo dosyaları
    `../src/sablon/logo/`'dan gömülür.
  - Alt sınıf kancaları için `web::Panel` özelliği: `rol`, `cerez`, `yan_menu`, `kullanici_etiketi`, `giris_basligi`,
    `giris_aciklama`, `dogrula`, `hesap_surumu`, `girdi`, `kim`, `hareket_musterisi`, `menu`, `giris_sonrasi`, `sayfalar`.
  - `web::Taban`: `oturumlar`, `kilit`, `veri`, `saat`, isteğe bağlı `raporlar`. Python'daki ortak akış: `istek`, `_giris`,
    `_gerekce`, `gerekce_iste`, `sayfa`, `mesaj`. `kayit_sayfasi` Task 5'te eklenir.
  - `merkez::Uygulama` özelliği: `fn istek(&self, i: &Istek) -> Yanit`.
  - `merkez::Merkez` (alan adı → uygulama; `ALANLAR` aynı), `merkez::istemci_ip(&Basliklar) -> String`.
  - `merkez::cevapla(&Merkez, Istek) -> Yanit`: 413 sınırı, panik → 500 "Bir sorun oluştu" + `log`, varsayılan güvenlik
    başlıkları uygulamanınkiyle birleşir.
  - `sunucu::calistir(merkez: Arc<Merkez>, adres: &str, isci: usize)`: tiny_http, `umask(0o077)`. Content-Length yoksa gövde
    boş sayılır (Python gibi).

- [ ] **Step 1: Testleri yaz.**
  - `test_merkez.py`'yi birebir taşı.
  - `test_soket_zaman_asimi`: Python'daki gibi okuma ve yazma zaman aşımı **20 sn**. `sunucu` dinleyiciyi kendisi açar ve
    `SO_RCVTIMEO`/`SO_SNDTIMEO`'yu dinleyici sokete koyar (Linux'ta kabul edilen bağlantılara geçer). Sonra
    `tiny_http::Server::from_listener` çağrılır.
    - Test kısa süreyle (1 sn) çalışır: yarım başlık gönderen bağlantı kapanır, bu sırada başka istek cevaplanır.
    - Geçiş cihazda doğrulanamazsa ruling: istek okuma bir iş parçacığında süre sınırıyla yapılır. Zaman aşımsız sunucuya
      izin yok.
  - Ek testler:
    - Form çözme: `a=1&a=2` → `2`, `+` → boşluk, `%C3%A7` → `ç`, geçersiz `%zz` aynen, boş değer.
    - 10 001 baytlık gövde → 413.
    - Bilinmeyen alan adı → 404 `Bulunamadı`.
    - Panik yapan uygulama → 500.
- [ ] **Step 2: Çalıştır, başarısız olsun.** `bash scripts/test-merkez.sh wificorrect merkez::`
- [ ] **Step 3: Yaz.** `web.rs`, `merkez.rs`, `sunucu.rs`. `main.rs`'ye `sunucu` alt komutu eklenir: `Veri::ac`,
  `rapor_gruplari_yaz` ve henüz taşınmış uygulamalarla `Merkez`. Uygulamalar sonraki görevlerde eklenir.
- [ ] **Step 4: Çalıştır, geçsin.** Aynı komut → `ok`.
- [ ] **Step 5: Commit.** `git commit -m "merkez: web tabani, alan adi dagiticisi, HTTP sunucusu"`

### Task 4: Altın kayıt düzeneği

**Files:**
- Create: `scripts/sunucu/parite/kaydet.py` (geçici, Task 16'da silinir), `scripts/sunucu/parite/yollar.py` (yol kapsamı)
- Create: `merkez/src/altin.rs` (`#[cfg(test)]`), `merkez/src/test_yardim.rs` (`#[cfg(test)]`; `tests/yardim.py`'nin karşılığı)
- Create: `merkez/tests/altin/<python_dosyasi>/<test_adi>.json` (üretilir)
- Modify: gerekirse `scripts/sunucu/merkez/tests/test_*.py`. Kapsanmayan her yol için Python'a küçük test eklenir; davranış
  değişmez.

**Interfaces:**
- Consumes: Task 1 belirlenimli rastgelelik algoritması, Task 3 `Istek`/`Yanit`.
- Produces:
  - `kaydet.py [test_dosyasi...]`: Python testlerini tek tek çalıştırır. Kendini `PYTHONHASHSEED=0` ile yeniden başlatır.
    Python'da küme (set) sırasına bağlı bir çıktı çıkarsa o alan iki tarafta da sıralanıp karşılaştırılır; durum defterine
    yazılır. Kurduğu kancalar:
    - `secrets`'in `token_urlsafe`, `token_hex`, `token_bytes`, `randbelow`, `choice` işlevlerini Task 1'deki belirlenimli
      algoritmayla değiştirir; sayaçlar her testin başında sıfırlanır.
    - `time.time`'a dokunmaz (testler zaten sahte saat verir).
    - Her `*.istek(...)` çağrısının `(durum, seçili başlıklar, gövde)` sonucunu sırayla kaydeder. Seçili başlıklar:
      `Location`, `Set-Cookie`, `Content-Type`, `Cache-Control`, güvenlik başlıkları.
    - Testte oluşturulan her `veri.Veri`'nin test sonundaki bütün tablolarını kaydeder.
    - Çıktı `merkez/tests/altin/<dosya>/<test>.json`.
    - Gövde normalleştirmesi: ortak CSS, logo ve sekme simgesi `{CSS}`, `{LOGO_ACIK}`, `{LOGO_KOYU}`, `{FAVICON}` yer
      tutucularıyla değişir. JSON gövdeler değer olarak saklanır.
  - `yollar.py`: Python kodundaki bütün yolları (dağıtıcı ve uygulama `istek` dalları) çıkarır; altın kayıtlarda hiç
    görülmeyenleri listeler. Sonuç boş olmalı.
  - Rust `altin::Kayit::bas(dosya, test)`: testin başında belirlenimli kipi açar ve sayaçları sıfırlar. `test_yardim`'ın
    `al`/`gonder`/`giris`/`gerekce_ver` yardımcıları cevapları ona ekler, `Veri` oluşturan `ortam` örnekleri ona bildirir.
    `Drop` (panik yoksa) cevapları sırayla ve tabloları değer olarak altın kayıtla karşılaştırır. Belirteçler her iki tarafta
    ilk görünüş sırasıyla `<T1>`, `<T2>`… olarak normalleşir. Farkta ilk farklı cevabın yolunu ve satırını yazar.
  - `Kayit::altinsiz(neden)`: yalnızca spec'teki bilinçli farklar için; neden defterine yazılır.

- [ ] **Step 1: `kaydet.py` ve `yollar.py`'yi yaz.** Bütün testleri kaydet: `py -3 -I scripts/sunucu/parite/kaydet.py` → 365
  testin hepsi geçer (Python davranışı değişmedi), kayıtlar oluşur.
- [ ] **Step 2: Kapsama.** `py -3 -I scripts/sunucu/parite/yollar.py` → kapsanmayan yollar. Her biri için Python'a test ekle,
  kaydı yeniden üret. Hedef: liste boş.
- [ ] **Step 3: Boyut.** Kayıtlar toplam 15 MB'ı aşarsa dosya başına gzip (`.json.gz`); iki taraf da okur.
- [ ] **Step 4: Rust tarafı.** `test_yardim.rs` (`ortam`, `sayi_yaz`, `gun_yaz`, `cerez`, `csrf`, `al`, `gonder`, `giris`,
  `gerekce_ver`) ve `altin.rs`. Task 2 ve 3'te taşınan testlere `Kayit::bas` ekle, altın kayıtla karşılaştır.
- [ ] **Step 5: Çalıştır, geçsin.** `bash scripts/test-merkez.sh wificorrect` → `ok`.
- [ ] **Step 6: Commit.** `git commit -m "merkez: altin kayit duzenegi (Python cevaplari + Rust karsilastirmasi)"`

> Sonraki her görev aynı kalıbı izler:
> - Python testlerini birebir taşı, her teste `Kayit::bas` ekle.
> - Testler başarısız → modülü yaz → testler ve altın kayıt geçer.
> - Taşınmayan testi adı ve nedeniyle defterine yaz.

### Task 5: Arşiv özetleri ve Kayıtlar (detay, ozet, istatistik, kayitlar)

**Files:**
- Create: `merkez/src/{detay,ozet,istatistik,kayitlar}.rs`
- Modify: `merkez/src/web.rs` (`Taban::kayit_sayfasi`), `merkez/src/main.rs` (`istatistik` alt komutu; rapor adımı Task 6)
- Taşınan Python testleri: `test_wc_detay.py` (6), `test_wc_ozet.py` (8), `test_wc_istatistik.py` (12),
  `test_kayitlar.py` (2), `test_misafir_kodu.py`'nin `test_maske_cihazla_ayni`, `test_opak_ve_sir_dosyasi`,
  `test_telefonlu_eski_arama_aynen`

**Interfaces:**
- Produces:
  - `detay::*` (`e`, `gecerli_gun`, `boyut`, `saat`, `sure`, `tarih`, `tr_kucuk`, `tc_gecerli`, `pasaport_normal`, `ayracsiz`,
    `maskele`, `opak`, `sir_oku_ya_uret`, `arama_formu`, `gunler_html`, `kisi_html`, `gun_icerik`, `telefon_sorgusu`,
    `oda_normal`, `ara_sonuclari`, `kimlik_sorgusu_mu`, `ara_icerik`).
  - `ozet::{detay_guncelle, kisi_dizini_guncelle, gun_ozeti, ...}`.
  - `istatistik::{tamam_mi, satirlar, farkli_kisi, guncelle, calistir(Yollar, simdi)}`.
  - `kayitlar::Kayitlar { ist_dizini, detay_dizini, sir_yolu }` (`sayilar`, `gunler`, `ozet_html`, `gun_html`, `ara_html`).
- Kurallar: dosya biçimleri ve izinler Python'la aynı (`0o640`, grup `wcpanel` sahipliği root çalışırken).

- [ ] **Step 1:** Testleri taşı. **Step 2:** Başarısız olsun (`istatistik:: ozet:: detay:: kayitlar::`).
- [ ] **Step 3:** Yaz. **Step 4:** Testler ve altın kayıt geçsin.
- [ ] **Step 5: Commit.** `git commit -m "merkez: arsiv ozetleri ve kayitlar"`

### Task 6: Raporlar (rapor, raporlar)

**Files:**
- Create: `merkez/src/{rapor,raporlar}.rs`
- Modify: `merkez/src/istatistik.rs` (rapor adımı)
- Taşınan Python testleri: `test_rapor.py` (46), `test_raporlar.py` (12). Biçim 1 ile ilgili testler yalnızca Task 0 biçim 1
  dosya bulduysa taşınır.

**Interfaces:**
- Produces:
  - `rapor::calistir(arsivler, grup_dosyasi, ham_dizini, cikti, panel_grubu, simdi, sir)`;
  - `rapor::{hafta, ay, donemler, takma}`;
  - `raporlar::Raporlar { dizin }` (`liste`, `rapor`);
  - `raporlar::istek(panel, ot, numara, alt, onek) -> Option<Yanit>` (müşteri ve yönetim kullanır);
  - `raporlar::rapor_verisi`.
- Kurallar:
  - HMAC takma kimliği Python'la aynı bayt dizisi: `hmac(sir, anahtar)`; sır dosyası `rapor-ham/.sir`.
  - Hafta ISO hafta (`2026-W38`), ay `YYYY-MM`.
  - En çok 26 hafta + 24 ay.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`rapor:: raporlar:: istatistik::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: haftalik ve aylik raporlar"`

### Task 7: Cihaz API'si, kuyruk, liste ve paket okuma

**Files:**
- Create: `merkez/src/{api,kuyruk}.rs`
- Create: `merkez/src/listeler.rs` (bu görevde okuma: `liste_yolu`, `meta_oku`, `ozet`), `merkez/src/surumler.rs` (bu
  görevde `paket_yolu`)
- Modify: `merkez/src/main.rs` (`kuyruk` alt komutu)
- Taşınan Python testleri:
  - `test_api.py` (27'den 23'ü);
  - `test_bekleme.py` (20);
  - `test_kuyruk.py` (8);
  - `test_misafir_kodu.py`'nin `test_eslesme_turu_kaydeder`'i.
- Taşınmayan testler:
  - `test_api_raporlar_liste_ve_donem`, `test_api_rapor_dogrulama_ve_yetki`, `test_rapor_hiz_siniri_sms_butcesinden_ayri`,
    `test_api_rapor_liste_ya_da_sozluk_tur_400` (uçlar kaldırıldı). Yerine `api_rapor_uclari_yok_404`.
  - `test_eski_veritabanina_tur_sutunu_eklenir` (ALTER).
- Değişen test: `test_sms_ayari_yoksa_yalnizca_etkin_mock_varsa_gider` artık eşleşmede `sms` olarak yalnızca `etkin_mock`
  bekler. `Kayit::altinsiz("spec bilinçli fark 2")` yalnızca eşleşme cevaplarına uygulanır.

**Interfaces:**
- Produces:
  - `api::Api` (`Uygulama`): `/api/{giris,bekliyor,bagla,eslesme,parola,saglik,paket,liste}`. `/api/sms` ve
    `/api/sms-dogrula` Task 8'de eklenir.
  - `cihaz.otomatik` artık eşleşme cevabını değiştirmez (NetGSM eki zaten yok); sütun şemada ve yönetim sayfasında kalır.
  - `api::saglik_temizle(&Value) -> Option<Value>`.
  - `kuyruk::{ekle, sonuc, bekle, is_oku, kayit_satiri, anahtar_satiri, isle, calistir}`: dosya ve komut biçimleri Python'la
    aynı.
- Kural: eşleşme eki `sms` yalnızca `{"etkin_mock": bool}`. `mock`, `provider` ve `netgsm` hiç gönderilmez.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`api:: kuyruk::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: cihaz API'si ve root kuyrugu"`

### Task 8: Doğrulama SMS'i (netgsm, twilio, /api/sms, /api/sms-dogrula)

**Files:**
- Create: `merkez/src/netgsm.rs`
- Modify: `merkez/src/twilio.rs` (ağ kısmı), `merkez/src/api.rs` (iki uç)
- Taşınan Python testleri: `test_sms_merkez.py` (24)

**Interfaces:**
- Produces:
  - `netgsm::{numaralar_10, xml_govde, yanit_kodu, gonder(ayar, metin, numaralar, &dyn Tasiyici), otp_xml, otp_yaniti,
    otp_gonder}`;
  - `twilio::{tr_mi, kimlik, gonder, dogrula}`.
- Kurallar:
  - Hata kodları ve misafir metinleri aynı.
  - Yeniden deneme yok.
  - Zaman aşımı işlem başına 5 sn (`ISLEM_SURE`).
  - Şablon `WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir.`
  - Sırlar curl'e yalnızca stdin'den gider.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`netgsm:: twilio:: api::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: dogrulama SMS'i (NetGSM, Twilio)"`

### Task 9: Geçitler (cihaz.wificorrect.com, ap.wificorrect.com)

**Files:**
- Create: `merkez/src/{cihaz,ap}.rs`
- Taşınan Python testleri: `test_cihaz.py` (6), `test_ap.py` (21)

**Interfaces:**
- Produces: `cihaz::Cihaz` (`Uygulama`: `/_giris`, `/_cikis`, `/cihaz-yetki`, `/_ap-gecis`) ve `ap::Ap` (`Uygulama`:
  `/_ap-giris`, `/_ap-cikis`, `/ap-yetki`).
- Kurallar:
  - Fetch Metadata kuralları, `/24`–`/64` ağ bağı, `wca` 30 dk, `Clear-Site-Data` ve 403 sayfaları birebir.
  - `X-WFC-*` başlık adları aynı.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`cihaz:: ap::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: cihaz paneli ve AP gecitleri"`

### Task 10: Müşteri paneli (panel.wificorrect.com)

**Files:**
- Create: `merkez/src/musteri.rs`
- Taşınan Python testleri: `test_musteri.py` (7), `test_misafir_kodu.py`'nin `test_merkezde_tam_kimlik_sizmaz`,
  `test_merkez_tc_pasaport_oda_aramasi`

**Interfaces:**
- Produces: `musteri::Musteri` (`Uygulama` + `web::Panel`). Yollar: `/`, `/gun/<gün>`, `/ara`, `/baglan`, `/cihaz`, `/parola`,
  `/raporlar`, `/rapor/...`, `/giris`, `/cikis`, `/gerekce`.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`musteri::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: musteri paneli"`

### Task 11: Yönetim paneli (yonetim.wificorrect.com)

**Files:**
- Create: `merkez/src/yonetim.rs`, `merkez/src/yonetici.rs` (`oku`, `yaz`; CLI Task 12)
- Taşınan Python testleri: `test_yonetim.py` (39), `test_misafir_kodu.py`'nin `test_yonetim_listesinde_tur`,
  `test_yonetimde_de_post_arama_maskeli`

**Interfaces:**
- Produces: `yonetim::Yonetim` (`Uygulama` + `web::Panel`). Python'daki bütün yollar: `/`, `/m/<numara>[...]`, `/yeni`,
  `/bekleyen`, `/hareketler`, `/eski-arsivler`, `/listeler`, `/sms`, `/uyari`, `/surumler`, `/hesabim`, `/admin-parolasi`,
  `/giris`, `/cikis`, `/gerekce` ve POST işlemleri. Sol menü (`secili_menu`) birebir.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`yonetim::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: yonetim paneli"`

### Task 12: Arka plan işleri (durum, saglik, listeler, surumler, yonetici)

**Files:**
- Create: `merkez/src/{durum,saglik}.rs`
- Modify: `merkez/src/listeler.rs` (üretim: `gecerli`, `temizle`, `uret`, `calistir`), `merkez/src/surumler.rs`
  (`github_al`, `yerel_ekle`, `otomatik_al`), `merkez/src/yonetici.rs` (CLI), `merkez/src/main.rs`
- Taşınan Python testleri: `test_durum.py` (4), `test_saglik.py` (43), `test_listeler.py` (32), `test_surumler.py` (7)

**Interfaces:**
- Produces alt komutlar:
  - `durum`;
  - `saglik`: gönderici = NetGSM genel SMS, `Tasiyici` ile;
  - `listeler`;
  - `surumler al`, `surumler ekle DOSYA SURUM`;
  - `yonetici parola AD`: `sudo wificorrect-sunucu yonetici-parola AD` (root) çağırır.
    - Python `yonetici.py`'yle birebir: dosya yolu (`/var/lib/wificorrect/merkez/yonetici.json`), sahibi ve izni (`birak` ile
      `wcpanel`), istem metinleri, en az uzunluk ve çıkış kodları.
    - Testle sabitlenir: geçici dizinde yazılan dosyayı Python `yonetici.oku` ve Rust `yonetim` girişi okur.
  - Her komut Python `main()`'inin yollarını, kullanıcısını ve çıkış kodunu korur.

- [ ] **Step 1–4:** Taşı → başarısız → yaz → geçsin (`durum:: saglik:: listeler:: surumler:: yonetici::`).
- [ ] **Step 5: Commit.** `git commit -m "merkez: arka plan isleri"`

### Task 13: Canlı denetim, kurulum, CI ve Python'suz ISO denemesi

**Files:**
- Create: `merkez/src/denetle.rs` (`veritabani-denetle VT [--dosyalar KOK]`)
- Create: `scripts/sunucu/merkez-kur.sh` (boş sunucu ve güncelleme), `scripts/sunucu/merkez-geri-don.sh`
- Modify: `scripts/sunucu/merkez/kur.sh` (Rust ikilisi, birimler; eski kafe paneli satırları, sbin bağları ve
  `PYTHONDONTWRITEBYTECODE` gider), `scripts/sunucu/merkez/wc-*.service` (ExecStart → `/usr/local/bin/wificorrect-merkez <komut>`),
  `scripts/sunucu/wificorrect-sunucu` (`yonetici-parola` → Rust)
- Delete: `scripts/sunucu/dagit.sh`, `scripts/sunucu/merkez-guncelle.sh` (yerlerine `merkez-kur.sh`)
- Modify: `.github/workflows/test.yml` (`merkez-rust` release derlemesi + yapıt `wificorrect-merkez-<sha>`),
  `.github/workflows/release.yml` (ikili sunucu paketine)
- Modify: `scripts/iso-test.sh`:
  - QEMU izleme soketine `nc -U` (yoksa `socat`);
  - ekran görüntüsü QEMU'nun kendi PNG'siyle (`screendump <dosya>.png -f png`); desteklenmezse PPM yapıt olarak kalır;
  - satır içi Python yok.

**Interfaces:**
- Produces:
  - `wificorrect-merkez veritabani-denetle <vt> [--dosyalar /var/lib/wificorrect]`:
    - salt okunur açar (`?mode=ro`);
    - `sema_denetle`'den geçirir;
    - her tablonun her satırını satır türüne okur;
    - JSON sütunlarını (`saglik.rapor`, `ayar.deger`) ve verilen kökteki JSON dosyalarını ayrıştırır.
    - Çıktı: `TABLO musteri 3`… ve `TAMAM` ya da `HATA <tablo|dosya> <kimlik> <neden>`; çıkış kodu 0/1. Değer basmaz.
    - Okuma sıkı. NULL olabilen sütunlar `Option`: `hareket.musteri`, `eski_arsiv.musteri`, `bekleyen.musteri`,
      `bekleyen.cihaz`. Bildirilen türden farklı saklama sınıfı (INTEGER sütunda metin) `HATA` verir. Hata çıkarsa sayılara
      bakılıp ruling verilir; sessiz varsayılan yok.
  - `merkez-kur.sh [hedef]`. Kullanıcının bilgisayarında, Git Bash'te çalışır (`gh` burada oturum açmış durumda):
    - acil durum seçeneği `--ikili DOSYA`: CI denetimini atlar, sha256'yı ekrana yazar, kalan adımlar aynı;
    - git ağacı temiz ve HEAD push'lanmış olmalı;
    - `gh run list --commit <HEAD> --workflow test` başarılı olmalı;
    - `gh run download -n wificorrect-merkez-<sha>`, sha256 doğrulaması;
    - sunucuya kopyalar ve `sudo bash kur.sh`'yi kullanıcının terminalinde çalıştırır.
  - `kur.sh` sırası:
    1. veritabanı varsa yedek;
    2. ikili varsa `veritabani-denetle` (hata → dur, canlıya dokunma);
    3. eski birim dosyalarını `/var/lib/wificorrect/geri-donus/<zaman>/`'a kopyala;
    4. ikiliyi ve birimleri kur;
    5. başlat;
    6. `curl -s -o /dev/null -w '%{http_code}' -H 'Host: yonetim.wificorrect.com' http://127.0.0.1:8081/giris` → 200.
  - `merkez-geri-don.sh [hedef]`: son geri dönüş birimlerini geri koyar ve yeniden başlatır. Python dosyalarına kurulum hiç
    dokunmadığı için yeterlidir.
- Test:
  - Python'un yazdığı `python-gocmus.db` ve Task 5–12 testlerinin ürettiği örnek dosya ağacı `denetle` ile `TAMAM` verir.
  - **Geri okuma testi** (`scripts/sunucu/parite/geri_oku.py`, yerelde; canlıya geçişin ön koşulu):
    - Rust testleri bir örnek ağaç üretir (`WFC_ORNEK_AGAC=<dizin> bash scripts/test-merkez.sh … denetle::ornek_agac`):
      veritabanı ile `durum.json`, `istatistik`, `detay`, `rapor`, `rapor-ham`, `kuyruk`, `kuyruk-sonuc`, `listeler` ve
      `yonetici.json` dosyaları.
    - Python bu ağaçta `veri.Veri` ile bütün okuma yöntemlerini, `kayitlar.Kayitlar`'ın sayfalarını, `raporlar.Raporlar`'ı,
      `kuyruk.sonuc`'u, `listeler.meta_oku`'yu ve `yonetici.oku`'yu çalıştırır.
    - Geçme ölçütü: hiçbir istisna yok ve sayılar Rust'ın yazdıklarıyla aynı.
  - `bash -n` bütün betiklere.
  - Birimler cihazda `systemd-analyze verify`.
  - `iso.yml` `workflow_dispatch` ile bir kez çalıştırılıp duman testinin geçtiği görülür.

- [ ] **Step 1–4:** Test → başarısız → yaz → geçsin (`denetle::`, betikler, CI).
- [ ] **Step 5: Commit.** `git commit -m "merkez: canli denetim, kurulum/geri donus, CI; iso-test Python'suz"`

### Task 14: Canlı geçiş (denetleyici + kullanıcı)

Ajan görevi değil. Önce bütün görevler birleşmiş ve son inceleme bitmiş olmalı.

- [ ] **Step 1: Hazırlık.** Ağaç temiz, push'lanmış, CI yeşil. Kullanıcıya özet ve geri dönüş komutu verilir.
- [ ] **Step 2: Kurulum.** Kullanıcı çalıştırır: `scripts/sunucu/merkez-kur.sh` (görünür PowerShell penceresi, sudo).
  Beklenenler: `veritabani-denetle … TAMAM`, `kurulum tamam: wc-merkez active`, `/giris 200`.
- [ ] **Step 3: Doğrulama.**
  - Denetleyici:
    - `systemctl is-active` bütün wc-* birimleri;
    - iki cihazın sağlık raporu 5 dk içinde (`saglik` tablosu zamanı, yalnızca sayı);
    - zamanlayıcıların bir sonraki turu;
    - fail2ban'ın `wc-merkez` filtresi.
  - Kullanıcı: yönetime giriş, müşteri sayfası, Sürümler, cihaz paneline geçiş, AP geçişi.
  - Gerçek SMS gönderilmez.
- [ ] **Step 4: Sorun olursa.** `scripts/sunucu/merkez-geri-don.sh` (kullanıcı). Sorun ruling'iyle görev açılır.
- [ ] **Step 5: Bir hafta izleme.** Günde bir kez denetleyici bakar: birimler, sağlık turları, `hareket` sayısı. Defterine
  yazılır.

### Task 15: Cihaz sadeleştirme (0.4.30)

**Files:**
- Modify: `src/logo.rs` (`LOGO_KOYU`, `marka_koyu` sil), `src/merkez.rs` (`curl_args` sil), `src/ulkeler.rs` (`is_tr` sil
  ya da yalnızca test ise teste taşı)
- Modify: `src/ayar.rs` (`eski_sirlari_temizle`, `eski_varsayilani_guncelle`, `ESKI_AYDINLATMA_SHA` sil), `src/ctl.rs`
  (`ayar-temizle` komutu ve kullanım satırı, `ap-adres-kaldir`'daki 0.4.11 temizliği), `paket/postinst` (`ayar-temizle` satırı)
- Delete: `tests/fixtures/aydinlatma-0.4.11.txt`
- Modify: `src/panel.rs`, `src/panel/ap_aktarim.rs` (merkez eski sürümdeyken `/_ap-gecis` sayfası)
- Modify: modül başlık yorumları (`src/*.rs` ilk satırları: "eski ….py", "Bu adımda … sonraki adımlarda") → bugünkü görevi
  anlatan tek cümle
- Modify: `src/kaydedici.rs` testi: OpenWrt dnsmasq satırı yalnızca biçim farkı içinse fixture ve döngü kalkar; kodda
  OpenWrt'ye özel dal varsa o da silinir.
- Modify: `Cargo.toml`/`Cargo.lock` sürümü 0.4.30

**Interfaces:**
- Davranış aynı. Yalnızca silinen yollar değişir: `ctl ayar-temizle` yok; cihazdan `/_ap-gecis` doğrudan istenirse 404.

- [ ] **Step 1: Kanıtı tazele.** İki cihazda yalnızca sayı okuyan denetimi (bu planın 2026-11-04 sayımı) yeniden çalıştır.
  Hepsi 0/hayır olmalı.
  - Eski ISO'yla (0.4.8 öncesi) kurulup güncellenen bir cihazda bile `ayar-temizle`'nin kalkması zararsızdır:
    - Rust merkez eşleşmede NetGSM bilgisi göndermez, dosyaya yeni sır girmez;
    - `Config` bilinmeyen bölümleri okurken yok sayar, ilk kayıtta yazmaz.
- [ ] **Step 2: Sil.** Silinen koda bağlı testleri de sil; derleyici uyarısız (`cargo check` cihazda, test dışı).
- [ ] **Step 3: Test.** `bash scripts/test.sh wificorrect` → bütün testler geçer, uyarı yok; CI `cihaz` ve `paket` yeşil.
- [ ] **Step 4: Commit.** `git commit -m "cihaz: olu ve eski kod temizligi (0.4.30)"`
- [ ] **Step 5: Yayın.** Birleştirme ve `v0.4.30` etiketi kullanıcı onayıyla. Sırası: test kanalında Göztepe, 24 saat sonra
  kararlı.

### Task 16: Python'u kaldırma ve belgeler (kullanıcı onayıyla, Task 14'ten bir hafta sonra)

**Files:**
- Delete: `scripts/sunucu/merkez/*.py`, `scripts/sunucu/merkez/tests/` (bütün Python testleri ve `hepsi.sh`),
  `scripts/sunucu/merkez/logo/`, `scripts/sunucu/parite/`, `scripts/sunucu/envanter.sh`
- Modify: `.github/workflows/test.yml`, `release.yml` (Python işi ve `setup-python` adımları gider)
- Modify: `scripts/sunucu/merkez/kur.sh` (sunucudaki `/usr/local/lib/wificorrect/*.py`, `logo/` ve geri dönüş birimlerini
  kaldırır)
- Modify: `docs/DEVAM.md`, `docs/KURULUM_GUNLUGU.md`, `CLAUDE.md` (merkez artık Rust; komutlar)
- Altın kayıtlar (`merkez/tests/altin/`) kalır: Rust testleri onlara karşı çalışmayı sürdürür.

- [ ] **Step 1:** `git grep -nE "python|\.py\b"` → kalan her satır gerekçeli olmalı (yalnızca geçmişi anlatan belge
  satırları).
- [ ] **Step 2:** CI yeşil, `bash scripts/test-merkez.sh wificorrect` yeşil.
- [ ] **Step 3:** Kullanıcı `merkez-kur.sh`'yi çalıştırır (sunucudaki Python dosyaları silinir); birimler çalışıyor.
- [ ] **Step 4: Commit.** `git commit -m "Python kaldirildi: merkez tamamen Rust"`
