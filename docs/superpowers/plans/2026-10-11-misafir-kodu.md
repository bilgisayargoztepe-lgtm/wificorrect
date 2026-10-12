# İşletme türü ve misafir kodu ile giriş — Uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** İşletme türü ayarı ve türe göre giriş yöntemleri; otel/yurtta personelin ürettiği 6 haneli misafir koduyla (TC ya da
pasaport + kod) portaldan giriş; TR/EN portal; kimliklerin 5651'de tam, ekranlarda maskeli, URL'lerde hiç görünmemesi.

**Architecture:** Cihaz (Rust): yeni `src/kimlik.rs` (TC/pasaport doğrulama, kişi anahtarı, maske, opak kimlik), `src/misafir.rs`
(kod deposu), `src/dil.rs` (TR/EN metinler); `ayar.rs`/`panel.rs`/`portal.rs`/`ortak.rs`/`kayit.rs`/`ctl.rs`/`merkez.rs` genişler.
Merkez (Python): `api.py` eşleşmede tür, `veri.py` sütun, `yonetim.py` listede tür, `detay.py`/`musteri.py` maskeleme + opak
kimlik + POST arama.

**Tech Stack:** Rust (mevcut ikili), Python stdlib, betiksiz HTML.

**Spec:** `docs/superpowers/specs/2026-10-11-isletme-turu-misafir-kodu-design.md` (onaylı).

## Global Constraints

- Cihaz testleri: `bash scripts/test.sh wificorrect <süzgeç>` (tamamı ~5 dk; aynı anda iki tane yok). Merkez testleri yerelde:
  `sh scripts/sunucu/merkez/tests/hepsi.sh /c/Users/hp/AppData/Local/Python/bin/python3.exe` (`WC_TEST=1`).
- İşletme türü değerleri `kafe | otel | yurt | diger`; etiketler "Kafe/Restoran", "Otel/Pansiyon", "Yurt", "Diğer"; varsayılan davranış
  `kafe`; ayarda anahtar yoksa "seçilmemiş" (Özet uyarısı "İşletme türünüzü seçin").
- Misafir kodu yalnızca `otel` ve `yurt`'ta görünür/açılabilir; tür başka bir şeye çekilince `[giris] misafir_kodu = false`.
- Geçerlilik seçenekleri (saat): 24 (varsayılan), 48, 72, 168 (1 hafta), 336 (2 hafta), 720 (1 ay). Uzatma mevcut bitişin üzerine.
- Kod: 6 hane, `getrandom`, aktif kodlar arasında tekil. Karşılaştırma sabit süreli (`hesap::ct_eq`).
- Kişi anahtarı: `TC:<11 hane>` / `PASAPORT:<ISO2>:<NO>`; dosya adı `:`→`_`; telefonlu anahtarlar (`905…`) aynen.
- Maske: TC `123******01` (ilk 3 + son 2), pasaport ilk 3 + `*` + son 2, uyruk açık. Telefon maskelenmez (bugünkü gibi).
- URL'de/çapada/sorgu dizgisinde TC/pasaport/telefon **yok**: kişi sayfası `?k=<opak>`; opak = HMAC-SHA256(sır, anahtar) ilk 16 hex.
  Cihaz sırrı `state_root/kisi_sir` (32 bayt, ilk kullanımda üretilir, 600); merkez sırrı `/var/lib/wificorrect/merkez/kisi_sir` (wcpanel 600).
  Kimlikle arama formları POST.
- Portal metinleri: kod alanı "Personelin size verdiği veya ilettiği kodu girin" / "Enter the code given or sent to you by our
  staff"; düğmeler Otel "Otel misafiriyim"/"Hotel guest", Yurt "Yurt sakiniyim"/"Dormitory resident"; tek hata metni
  "Bilgiler eşleşmedi ya da kodun süresi doldu." / "The details did not match or the code has expired." Hiçbir metinde "kupon" yok.
- Kilit: IP ve MAC başına 5 hata → 15 dk. Oturum `ek`: `giris=misafir_kodu oda=<no> kod_bitis=<iso> ureten=<hesap>`.
- Denetim: `PANEL_ISLETME_TURU eski= yeni=`, `PANEL_MISAFIR_KODU islem=uret|iptal|uzat oda= bitis= tur=tc|pasaport`
  (kod/TC/pasaport yazılmaz), `PANEL_MISAFIR_KODLARI` (kişisel veri bakışı), `OTURUM_BITIS neden=kod_iptal`.
- Panel ve portal betiksiz (CSP aynen). Gerçek SMS yok.

## Review Focus

- **TC kontrol basamağı uç durumları** (ilk hane 0, 10 hane, harfli, geçerli/geçersiz örnekler) → Task 1 testi `tc_kontrol_basamagi`.
- **Kaba kuvvet kilidi IP değiştirerek atlatılamaz** (MAC kilidi de sayar; kilitliyken doğru kod da reddedilir) → Task 6 testi `misafir_kilit_ip_ve_mac`.
- **İptal/uzatma açık oturuma yansır** (nft süresi ve sessions.json) → Task 4 testleri `iptal_oturumu_kapatir`, `uzatma_oturumu_uzatir`.
- **Hiçbir HTML/URL'de tam kimlik yok** (panel listeleri, kişi sayfası bağlantıları, merkez gün/kişi/arama) → Task 7 testi `tam_kimlik_sizmaz` ve Task 8 testi `merkezde_tam_kimlik_sizmaz`.
- **Eski telefonlu kayıtlar ve aramalar bozulmaz** → Task 7 testi `telefonlu_kayit_aramasi_aynen`.

---

### Task 1: `src/kimlik.rs` — doğrulama, kişi anahtarı, maske, opak kimlik
**Files:** Create `src/kimlik.rs` (+ `mod kimlik;`)
**Produces:** `pub fn tc_gecerli(s: &str) -> bool`; `pub fn pasaport_normal(s: &str) -> Option<String>` (5–20 `[A-Z0-9]`, büyük harf);
`pub enum Kimlik { Tc(String), Pasaport { uyruk: String, no: String } }` + `pub fn anahtar(&self) -> String`;
`pub fn coz(anahtar: &str) -> Option<Kimlik>`; `pub fn maskele(anahtar: &str) -> String` (telefon anahtarı aynen döner);
`pub fn dosya_adi(anahtar: &str) -> String`; `pub fn opak(sir: &[u8], anahtar: &str) -> String`; `pub fn sir_oku_ya_uret(state_root: &str) -> Vec<u8>`.
- [ ] Tests: `tc_kontrol_basamagi` (gerçek algoritmaya uyan 2 örnek geçerli; ilk hane 0, 10 hane, harf, bozuk kontrol hanesi geçersiz),
  `pasaport_normal_buyuk_harf_ve_sinirlar`, `anahtar_gidis_donus`, `maske_bicimi` (`TC:12345678901`→`123******01`,
  `PASAPORT:DE:X1234567`→`DE X12****67` biçiminde uyruk açık; telefon aynen), `dosya_adi_iki_nokta_yok`, `opak_sabit_ve_16_hex`.
- [ ] RED → implement (HMAC-SHA256 RFC 2104 elle, mevcut `sha2` ile; yeni bağımlılık yok; RFC 4231 test vektörü 1 ile sına) → GREEN (`kimlik::`) → full suite → commit.

### Task 2: İşletme türü, giriş yöntemleri ayarı, eşleşmede tür
**Files:** Modify `src/ayar.rs`, `src/panel.rs` (Ayarlar İşletme alanı, yeni "Giriş yöntemleri" bölümü, Özet uyarısı), `src/merkez.rs` (`eslesme` gövdesine `isletme_turu`)
**Produces:** `Config.main.isletme_turu: Option<String>`; `fn isletme_turu(&Config) -> &str` (yoksa `kafe`); `Config.giris.misafir_kodu: bool`;
`pub enum GirisYontemi { Sms, MisafirKodu }` + `fn gorunur(&self, tur: &str) -> bool`, `fn acik(&self, cfg) -> bool`;
`fn misafir_kodu_acik(&Config) -> bool` (açık ve tür otel/yurt).
- [ ] Tests: `tur_secilmemisse_ozet_uyarisi` (anahtar yok → uyarı; `kafe` yazılınca yok), `tur_degisimi_denetime` (`PANEL_ISLETME_TURU eski=kafe yeni=otel`),
  `misafir_kodu_yalniz_otel_yurtta_gorunur`, `turden_cikinca_misafir_kodu_kapanir`, `eslesme_govdesinde_tur`, eski ayar dosyası (anahtarsız) yüklenir.
- [ ] RED → implement → GREEN → full suite → commit.

### Task 3: `src/misafir.rs` — kod deposu
**Files:** Create `src/misafir.rs`
**Produces:** `pub struct Kod { pub kod: String, pub kimlik: String /*anahtar*/, pub ad: String, pub soyad: String, pub oda: String, pub uretim: f64, pub bitis: f64, pub ureten: String, pub iptal: bool }`;
`pub fn oku(state_root) -> Vec<Kod>`, `kaydet(state_root, &[Kod])` (`misafir_kodlari.json`, 600, atomik);
`pub fn uret(kodlar: &mut Vec<Kod>, kimlik, ad, soyad, oda, sure_saat: u64, ureten, now) -> (Kod, Option<Kod> /*iptal edilen eski*/)`;
`pub fn iptal(kodlar, kod_id) -> Option<Kod>`; `pub fn uzat(kodlar, kod_id, sure_saat, now) -> Option<f64 /*yeni bitis*/>`;
`pub fn dogrula(kodlar, kimlik, kod, now) -> Option<&Kod>` (sabit süreli karşılaştırma; iptal/süresi dolmuşsa None);
`pub fn temizle(kodlar, now)` (bitişten 30 gün sonra sil); `pub const SURELER: [u64; 6] = [24, 48, 72, 168, 336, 720]`.
Kod kimliği (`kod_id`): kodun kendisi değil, ayrı rastgele 8 hex (panel bağlantılarında kod görünmesin).
- [ ] Tests: `uret_alti_hane_tekil`, `ayni_kimlik_eskisini_iptal`, `uzat_bitis_uzerine_ekler`, `bir_hafta_bitis_dogru`, `dogrula_iptal_ve_suresi_dolmus_red`, `temizle_30_gun`, gidiş-dönüş.
- [ ] RED → implement → GREEN → commit.

### Task 4: Panel — Misafir kodları sayfaları ve oturum etkileri
**Files:** Modify `src/panel.rs` (menü, rotalar), Create `src/panel/misafir_sayfasi.rs`; `src/ortak.rs` (gerekirse `oturum_uzat(state, mac, yeni_bitis, runner)`)
**Produces:** GET `/misafir-kodlari` (form + aktif kodlar, maskeli), POST `/misafir-kodlari/uret`, POST `/misafir-kodlari/iptal` (`id`), POST `/misafir-kodlari/uzat` (`id`, `sure`), GET `/misafir-kodlari/fis?id=` (yazdırılabilir; `@media print`).
Menü öğesi yalnızca `misafir_kodu_acik`. Üretimde form doğrulaması (TC algoritması, pasaport+uyruk listeden, oda `[A-Za-z0-9-]{1,10}`, ad/soyad portaldaki kurallarla).
İptal → o kimlikli açık oturumlar `close_session(.., "kod_iptal")`; uzatma → o kimlikli oturumların `expires_epoch` ve nft süresi yeni bitişe.
Fiş: ad soyad, oda, büyük kod, bitiş `GG.AA.YYYY SS:DD`, TR + EN talimat ("Wi-Fi'ye bağlanın, açılan sayfada 'Otel misafiriyim'e dokunun…").
- [ ] Tests: `kod_uret_tc_ve_pasaport` (geçersiz TC 400), `liste_maskeli_ve_url_de_kimlik_yok`, `iptal_oturumu_kapatir`, `uzatma_oturumu_uzatir` (nft komutu yeni süreyle), `ayni_kimlik_yeni_kod_eskiyi_iptal_eder`,
  `denetimde_kod_ve_kimlik_yok`, `misafir_kodu_kapaliyken_404`, `fis_bitis_ve_iki_dil`.
- [ ] RED → implement → GREEN → full suite → commit.

### Task 5: Portal TR/EN
**Files:** Create `src/dil.rs`; Modify `src/portal.rs`, `src/sablon/*.html`, `src/ayar.rs` (`PortalMetin` + `aydinlatma_en`, `acik_riza_en`, `sozlesme_en`), `src/panel.rs` (Portal metinleri: İngilizce alanlar)
**Produces:** `pub enum Dil { Tr, En }`, `pub fn sec(sorgu, cerez, accept_language) -> Dil`, `pub fn m(d: Dil, anahtar: &str) -> &'static str`; portal üstünde `TR | EN` bağlantısı (`?dil=tr|en`), çerez `dil` (SameSite=Lax, 1 yıl).
- [ ] Tests: `dil_secimi_oncelik` (sorgu > çerez > Accept-Language > tr), `en_formu_ve_hatalar_ingilizce`, `hukuki_metin_en_bossa_turkce`, mevcut portal testleri TR'de aynen.
- [ ] RED → implement → GREEN → full suite → commit.

### Task 6: Portal misafir girişi
**Files:** Modify `src/portal.rs` (+ şablon)
**Produces:** giriş sayfasında türe göre düğme (`misafir_kodu_acik` iken) → GET `/misafir`, POST `/misafir/dogrula` (kimlik türü, TC ya da pasaport+uyruk, kod, onaylar);
eşleşince `open_session` benzeri akış: kişi anahtarı = misafirin kimliği, `secs = kod.bitis - now`, cihaz sınırı aynı kimlik (kod) başına `max_devices_per_phone`, `ek` alanı, kişi dosyası/index;
5 hata → 15 dk (IP ve MAC ayrı sayaç); tek hata metni.
- [ ] Tests: `misafir_tc_ile_giris`, `misafir_pasaport_ile_giris`, `misafir_yanlis_kod_tek_mesaj`, `misafir_kilit_ip_ve_mac`, `misafir_oturum_kod_bitisinde_biter`, `kod_basina_cihaz_siniri`, `kafede_misafir_yolu_yok`, `kupon_kelimesi_yok`.
- [ ] RED → implement → GREEN → full suite → commit.

### Task 7: Cihaz kayıtları, arama, maskeleme, opak kimlik
**Files:** Modify `src/ortak.rs` (`user_file` → `kimlik::dosya_adi`), `src/kayit.rs` (`ara` türleri `tc`, `pasaport`, `oda`), `src/ctl.rs` (`--tc`, `--pasaport [--uyruk]`, `--oda`), `src/panel/kayit_sayfalari.rs` + `src/panel.rs` (Kullanıcılar listesi/kişi sayfası `?k=<opak>`, arama POST, Resmi talep yeni alanlar POST, Özet/Bağlı kullanıcılar maskeli)
- [ ] Tests: `tam_kimlik_sizmaz` (TC/pasaportlu oturum + kayıt → Özet, Bağlı kullanıcılar, Kullanıcılar, kişi sayfası HTML'inde tam değer ve URL'de kimlik yok), `talep_tc_pasaport_oda`,
  `telefonlu_kayit_aramasi_aynen`, `kisi_sayfasi_opak_kimlikle`, `kayit_dosyasinda_tam_deger` (görüntüleyici tam gösterir).
- [ ] RED → implement → GREEN → full suite → commit.

### Task 8: Merkez — tür, maskeleme, opak kimlik, arama
**Files:** Modify `scripts/sunucu/merkez/api.py` (`eslesme` `isletme_turu` ∈ izinli değerler), `veri.py` (`cihaz.isletme_turu` sütunu, göç), `yonetim.py` (liste ve müşteri sayfasında tür), `detay.py` + `musteri.py` (gün/kişi/arama maskeli, çapalar opak, arama POST + TC/pasaport/oda), yeni sır dosyası okuma/üretme
- [ ] Tests (merkez): `eslesme_turu_kaydeder`, `yonetim_listesinde_tur`, `merkezde_tam_kimlik_sizmaz`, `merkez_tc_pasaport_oda_aramasi`, `telefonlu_eski_arama_aynen`.
- [ ] RED → implement → GREEN (`hepsi.sh`) → commit.

### Task 9: Kurulum ve uçtan uca deneme (denetleyici + kullanıcı)
- [ ] Cihaz: `gelistir.sh` + panel/portal/kaydedici yeniden başlatma (ağa dokunmaz). Merkez: kullanıcı `merkez-guncelle.sh`.
- [ ] Panelden tür Otel, Misafir kodu açık.
- [ ] Sahte istemci (`scripts/sahte-istemci.sh` kalıbı; dhcpcd'ye sinyal yok) ile: TC'li kod üret → `/misafir/dogrula` → internet; pasaportlu kod → giriş; 1 haftalık kod → nft süresi ≈ 7 gün;
  süre uzat → nft süresi uzadı; iptal → oturum düştü, `OTURUM_BITIS neden=kod_iptal`. Deneme verileri: geçerli algoritmalı uydurma TC ve uydurma pasaport; denemeden sonra kodlar iptal.
- [ ] Sonuçları kullanıcıya göster; tür ve misafir kodu ayarını kullanıcının istediği hâle döndür.
