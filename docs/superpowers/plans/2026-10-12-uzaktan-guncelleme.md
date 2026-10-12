# Merkezden uzaktan güncelleme — uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Yöneticinin Yönetim'de seçtiği `.deb` sürümünü cihazların kendiliğinden, pencere içinde kurması; bozulursa eski sürüme kendiliğinden dönmesi.

**Architecture:** Merkez sürümleri GitHub release'lerinden (ya da yerel CLI ile) `/var/lib/wificorrect/merkez/surumler/`'e alır;
kanal (test/kararlı) hedefini `/api/saglik` cevabında `guncelleme` olarak verir, paketi `/api/paket` ile sunar. Cihazda `ctl saglik`
cevabı değerlendirir, gerekiyorsa `systemd-run … ctl guncelle` başlatır; kurulumdan önce eski paketi ve **eski ikilinin yazdığı**
denetim betiğini bırakır; paketle gelen periyodik zamanlayıcı betiği çalıştırır, 10 dk sonra sağlıksızsa eski paketi geri kurar.

**Tech Stack:** Rust (cihaz; serde_json, sha2, curl, systemd-run, apt-get, dpkg-query), Python stdlib + SQLite (merkez), sh.

**Spec:** `docs/superpowers/specs/2026-10-12-uzaktan-guncelleme-design.md`

## Global Constraints

- Kanallar: `test`, `kararli`; müşteri ayarı `kararli` (varsayılan) | `test` | `kapali`.
- Sürüm adı `^\d+\.\d+\.\d+$`; paket adı `wificorrect_<sürüm>_amd64.deb`; GitHub repo `bilgisayargoztepe-lgtm/wificorrect`, etiket `v<sürüm>`.
- Kurulum penceresi **03:00–06:00 TR** (UTC+3 sabit, `merkez.rs` `TR`); `hemen` işaretiyle pencere beklenmez.
- Denetim kurulumdan **600 sn** sonra; servisler `wificorrect-portal wificorrect-panel wificorrect-kaydedici dnsmasq wificorrect-guvenlik`.
- Cihazda sürüm kararları **dpkg sürümüyle** (`dpkg-query -W -f='${Version}' wificorrect`); paket yoksa `paket=""` (paketsiz) → hiç güncellemez.
- sha256 tutmazsa ya da boyut farklıysa kurulmaz. Paket yalnızca bağlı, üyeliği ve lisansı açık cihaza verilir.
- Cihaz denetim olayları: `GUNCELLEME_BASLADI`, `GUNCELLEME_TAMAM`, `GUNCELLEME_HATA`, `GUNCELLEME_GERI_ALINDI` (ek: `surum=… onceki=… neden=…`).
  Merkez hareketleri: `SURUM_ALINDI`, `KANAL_HEDEF`, `GUNCELLEME_KANALI`, `GUNCELLEME_HEMEN`, `GUNCELLEME_SONUC`.
- Kullanıcıya görünen metin Türkçe; tanımlayıcılar mevcut kod gibi (Türkçe ASCII adlar).
- Misafir oturumları güncellemede kapanmaz (postinst yalnızca `try-restart`).

## Review Focus

- Yeni ikili hiç çalışmıyor (ör. `exit 1`): geri dönüş yine olmalı → denetim betiği eski ikilinin bıraktığı dosyadır, Rust'a dayanmaz (Task 4 testi + Task 6 e2e).
- Cihaz kurulumdan sonraki 10 dk içinde yeniden açılıyor: periyodik zamanlayıcı açılışta da çalışır, betik süresi dolunca karar verir (Task 4).
- Eski cihaz (bu sürümden önceki) `paket`/`guncelleme` alanı göndermiyor: merkez raporu reddetmemeli (Task 1 testi).
- Aynı başarısız sürüm her 5 dk'da yeniden indirilmemeli: `basarisiz` listesi ve `hata` sonrası 12 saat bekleme (Task 3 testi).
- İki `ctl saglik` aynı anda kurulum başlatmamalı: sabit birim adı `wfc-guncelle` + kurulum kilidi (Task 4 testi).

---

### Task 1: Merkez — sürüm deposu, kanallar, API

**Files:**
- Create: `scripts/sunucu/merkez/surumler.py`
- Modify: `scripts/sunucu/merkez/veri.py` (tablo `surum`, `musteri.guncelleme_kanali` TEXT DEFAULT 'kararli', `musteri.guncelleme_hemen` REAL DEFAULT 0; `_gocur`)
- Modify: `scripts/sunucu/merkez/api.py` (`saglik_temizle`, `saglik`, yeni `/api/paket`)
- Test: `scripts/sunucu/merkez/tests/test_surumler.py` (yeni, `hepsi.sh`'a ekle), `tests/test_api.py`

**Interfaces:**
- Produces (veri.Veri):
  - `surum_ekle(surum, sha256, boyut, kaynak)`; `surumler() -> [dict(surum, sha256, boyut, kaynak, zaman)]` (yeniden eskiye, sürüm sırasıyla); `surum(surum) -> dict|None`
  - `kanal_hedef(kanal) -> str|None` / `kanal_hedef_koy(kanal, surum_or_None)` (ayar tablosu, anahtar `kanal_<ad>`)
  - `guncelleme_kanali_koy(numara, kanal)`; `guncelleme_hemen_koy(numara, epoch)`
  - `cihaz_hedefi(musteri_row) -> dict|None`: `{surum, sha256, boyut, hemen: bool}`; None: kanal `kapali`, kanal hedefi yok, sürüm kaydı yok, üyelik/lisans kapalı (`lisans_durumu` "aktif" değil) ya da askıda
- Produces (surumler.py):
  - `DIZIN = "/var/lib/wificorrect/merkez/surumler"`; `SURUM_RE`
  - `github_al(veri, dizin, getir) -> list[str]`: eklenen sürümler. `getir(url, sinir_bayt) -> bytes` (gerçek: urllib, 60 sn, `User-Agent: wificorrect-merkez`); release listesi `https://api.github.com/repos/bilgisayargoztepe-lgtm/wificorrect/releases?per_page=10`; her `tag_name` `v<sürüm>` için `wificorrect_<s>_amd64.deb` + `SHA256SUMS` varlıkları (`browser_download_url`); deb ≤ 50 MB; SHA256SUMS'taki satırla tutmazsa atlanır (hata listesine), dosya `<dizin>/<s>.deb.tmp` → `os.replace` → `<s>.deb`, izin 644.
  - `yerel_ekle(veri, dizin, dosya, surum)`: aynı doğrulama olmadan sha hesaplayıp ekler (kaynak `yerel`); `python3 surumler.py ekle DOSYA SURUM` CLI (`sudo -u wcpanel` ile).
  - `paket_yolu(dizin, surum) -> str`
- Produces (api): `/api/saglik` cevabı `{"tamam": true, "guncelleme": cihaz_hedefi(...)}` (None ise alan yok); `/api/paket` `{cihaz_anahtari, surum}` → 200 `application/octet-stream` gövde; izin yalnızca `surum` == cihaz hedefi **ya da** == cihazın son raporundaki `paket` (geri dönüş paketi için); aksi 403 `{"kod":"yetki"}`, dosya yoksa 404. Api kurucusuna `surum_dizini=surumler.DIZIN` parametresi.
- `saglik_temizle`: isteğe bağlı `paket` (str ≤ 40, `""` ya da SURUM_RE) ve `guncelleme` (`null` ya da `{surum: SURUM_RE, sonuc: "tamam"|"geri_alindi"|"hata", zaman: int ≥ 0}`); yoksa eklenmez, bozuksa rapor reddedilir. Rapor saklanırken `guncelleme.zaman > musteri.guncelleme_hemen` ise `guncelleme_hemen` 0 yapılır; son raporun `guncelleme`'si değiştiyse `hareket(..., "GUNCELLEME_SONUC", ayrinti="surum=… sonuc=…")`.

- [ ] **Step 1: Testleri yaz** — `test_surumler.py`: sahte `getir` iki release döndürür (biri SHA tutmuyor) → yalnız doğru olan eklenir, dosya diskte 644, ikinci çağrıda yeniden indirilmez; etiket `v1.2` / `vx` atlanır; 50 MB üstü reddedilir. `test_api.py`: (a) eski biçim rapor (paket/guncelleme yok) 200; (b) `paket: "0.4.0", guncelleme: {"surum":"0.4.1","sonuc":"geri_alindi","zaman":5}` kabul, `sonuc:"x"` 400; (c) kanal `test` hedef `0.4.1` + müşteri `test` → cevapta `guncelleme == {"surum":"0.4.1","sha256":…,"boyut":…,"hemen":False}`; müşteri `kapali` → alan yok; lisans bitmiş → alan yok; (d) `hemen` koyulunca `True`, sonra `guncelleme.zaman` daha yeni rapor → `False`; (e) `/api/paket` hedef sürüm 200 ve gövde = dosya; rapordaki `paket` sürümü 200; başka sürüm 403; tanınmayan anahtar 401.
- [ ] **Step 2:** `sh scripts/sunucu/merkez/tests/hepsi.sh python` → yeni testler FAIL.
- [ ] **Step 3:** Uygula (yukarıdaki imzalar). `Api.istek` ikili gövdeyi `_json` dışında doğrudan döndürür (`Content-Type: application/octet-stream`, `Cache-Control: no-store`).
- [ ] **Step 4:** Testler → `TUM TESTLER GECTI`.
- [ ] **Step 5: Commit** `merkez: surum deposu, kanal hedefi, /api/paket`

### Task 2: Merkez — Yönetim sayfaları ve sağlık kuralı

**Files:**
- Modify: `scripts/sunucu/merkez/yonetim.py` (menüye **Sürümler** `/surumler`; müşteri listesine **Sürüm** sütunu; müşteri sayfasına güncelleme kartı)
- Modify: `scripts/sunucu/merkez/saglik.py` (`sorunlar(..., hedef=None)`)
- Test: `tests/test_yonetim.py`, `tests/test_saglik.py`

**Interfaces:**
- Consumes: Task 1 `veri` işlevleri, `surumler.github_al`, `surumler.DIZIN`.
- `/surumler` (yalnız yönetici, POST'lar CSRF'li): tablo (sürüm, kaynak, tarih, boyut, sha256 ilk 12, kurulu cihaz sayısı — son raporlardaki `paket`); "GitHub'dan yeni sürümleri al" (POST `/surumler/al` → `github_al`, sonuç mesajı, hareket `SURUM_ALINDI surum=…`); kanal başına hedef seçimi (POST `/surumler/hedef` `kanal`, `surum` ya da boş = hedef yok; hareket `KANAL_HEDEF`).
- Müşteri sayfası kartı "Güncelleme": kanal seçimi (POST `/m/<n>/guncelleme` `kanal`; hareket `GUNCELLEME_KANALI`), "Şimdi güncelle" (POST `/m/<n>/guncelleme-hemen`; hareket `GUNCELLEME_HEMEN`; `hemen` bekliyorsa "bekleniyor" yazar), kurulu paket (`paketsiz` ise "Paketsiz kurulum — uzaktan güncellenmez"), hedef sürüm, son sonuç, `GUNCELLEME_SONUC` hareketlerinden geçmiş (son 10).
- Liste sütunu: kurulu `paket` (yoksa "—"); hedeften farklıysa mevcut uyarı sınıfıyla (sarı).
- `saglik.sorunlar`: `hedef` verilmiş ve `rapor.guncelleme.sonuc in ("geri_alindi","hata")` ve `rapor.guncelleme.surum == hedef` ve `rapor.paket != hedef` → `("guncelleme_basarisiz", f"Güncelleme başarısız ({hedef})", "uyari")`. Çağıran (`saglik.py`'deki değerlendirme) `veri.cihaz_hedefi(m)`'den sürümü geçirir.

- [ ] **Step 1: Testleri yaz** — yönetim: `/surumler` yönetici dışına 403/yönlendirme; sahte `github_al` ile POST mesajı; hedef seçimi kaydedilir ve hareket yazılır; geçersiz sürüm reddedilir; müşteri kanal POST'u CSRF'siz 403; liste sütununda farklı sürüm sarı. sağlık: kural üç durumda (açılır / paket hedefe eşit → açılmaz / sonuc tamam → açılmaz).
- [ ] **Step 2:** testler FAIL.
- [ ] **Step 3:** Uygula; sayfa kalıbı mevcut `sms`/`uyari` sayfaları gibi.
- [ ] **Step 4:** `TUM TESTLER GECTI`.
- [ ] **Step 5: Commit** `merkez: Surumler sayfasi, musteri guncelleme karti, guncelleme_basarisiz sorunu`

### Task 3: Cihaz — güncelleme kararı ve sağlık raporu alanları

**Files:**
- Create: `src/guncelleme.rs` (`main.rs`'e `mod guncelleme;`)
- Modify: `src/saglik.rs` (rapora `paket`, `guncelleme`; `gonder` cevaptaki hedefi döner; başarıda `saglik_basari` dosyasına dokunur), `src/ctl.rs` (`saglik` dalı)
- Test: `src/guncelleme.rs`, `src/saglik.rs` içi testler

**Interfaces:**
- Produces (`guncelleme.rs`):
  - `pub struct Yollar { pub kok: PathBuf }` + `Yollar::sistem()` (`/var/lib/wificorrect/guncelleme`); yöntemler: `durum()` (`durum.json`), `hedef()` (`hedef.json`), `paketler()` (`paketler/`), `bekleyen()`, `sonuc()`, `onceki_deb()` (`onceki.deb`), `betik()` (`denetle.sh`), `saglik_basari()`, `kilit()`.
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct Hedef { pub surum: String, pub sha256: String, pub boyut: u64, #[serde(default)] pub hemen: bool }`; `pub fn hedef_coz(v: &Value) -> Option<Hedef>` (sürüm `^\d+\.\d+\.\d+$`, sha 64 küçük hex, boyut > 0, yoksa None).
  - `#[derive(Serialize, Deserialize, Default)] pub struct Durum { pub basarisiz: Vec<String>, pub son: Option<Sonuc> }`, `pub struct Sonuc { pub surum: String, pub sonuc: String /* tamam|geri_alindi|hata */, pub zaman: u64, #[serde(default)] pub neden: String }`; `durum_oku(&Yollar) -> Durum` (yok/bozuk → varsayılan), `durum_yaz` (atomik).
  - `pub fn kurulu(komut: &dyn Fn(&[&str]) -> Option<String>) -> String`: `dpkg-query -W -f=${Version} wificorrect`; hata/boş → `""`.
  - `pub enum Karar { Kur, Bekle(&'static str) }`; `pub fn karar(kurulu: &str, h: &Hedef, d: &Durum, bekleyen_var: bool, now: f64) -> Karar`. Sırayla: kurulu `""` → `Bekle("paketsiz")`; `kurulu == h.surum` → `Bekle("guncel")`; `bekleyen_var` → `Bekle("denetim")`; `!h.hemen && basarisiz.contains` → `Bekle("basarisiz")`; `!h.hemen && son.sonuc=="hata" && son.surum==h.surum && now - son.zaman < 43200` → `Bekle("hata_bekle")`; `!h.hemen && !pencere(now)` → `Bekle("pencere")`; yoksa `Kur`.
  - `pub fn pencere(now: f64) -> bool` (TR saat 03:00 ≤ t < 06:00).
  - `pub fn uzlas(y: &Yollar, log_root: &str, now: f64)`: `bekleyen` (JSON `{yeni, onceki, zaman}`) ve `sonuc` (`tamam`|`geri`) varsa: tamam → denetim `GUNCELLEME_TAMAM`, `son={yeni,tamam}`; geri → `GUNCELLEME_GERI_ALINDI`, `basarisiz`'a `yeni` (tekil), `son={yeni,geri_alindi}`; `hemen` ile tekrar başarı gelirse `basarisiz`'dan çıkar; ardından `bekleyen`, `sonuc` silinir. `paketler/`'de en yeni 2 dosya dışındakileri siler (kurulu ve önceki korunur).
  - `pub fn rapor_alanlari(y: &Yollar, kurulu: &str) -> (String, Value)`: `("paket", kurulu)`, `guncelleme` = `son` (yoksa `null`) — `{surum, sonuc, zaman}`.
- `saglik::gonder(...) -> Result<Option<Hedef>, String>`; `saglik::calistir` önce `uzlas`, rapora alanları ekler, başarıda `saglik_basari`'ya şimdiki zamanı yazar (mtime denetim betiği içindir) ve hedefi döner.
- `ctl saglik`: hedef `Some` ve `karar == Kur` ise `hedef.json`'u yazar ve `systemd-run --collect --unit wfc-guncelle /usr/local/bin/wificorrect ctl guncelle` (birim zaten varsa systemd reddeder; sorun değil). Karar `Bekle` iken sessiz.

- [ ] **Step 1: Testleri yaz** — `karar` her dal (tablo testi; pencere için TR 02:59 → pencere dışı, 03:00 → içinde, 05:59 → içinde, 06:00 → dışında); `hedef_coz` bozuk sha / `1.2` / boyut 0 → None; `uzlas` tamam ve geri yolları (denetim satırı, durum, dosyalar silindi), aynı sürüm iki kez geri → `basarisiz`'da tek; `kurulu` boş çıktı → `""`; saglik rapor JSON'unda `paket` ve `guncelleme` alanları (yokken `null`); `gonder` cevapta `guncelleme` varsa `Some(Hedef)`.
- [ ] **Step 2:** `bash scripts/test.sh wificorrect guncelleme` ve `saglik` → FAIL.
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** İki süzgeç PASS.
- [ ] **Step 5: Commit** `cihaz: guncelleme karari, saglik raporunda paket ve son sonuc`

### Task 4: Cihaz — `ctl guncelle`, denetim betiği, zamanlayıcı

**Files:**
- Modify: `src/guncelleme.rs` (`calistir_guncelle`, `BETIK`), `src/merkez.rs` (`indir`), `src/ctl.rs` (`guncelle`)
- Create: `deploy/debian/etc/systemd/system/wificorrect-guncelleme-denetle.service`, `…/wificorrect-guncelleme-denetle.timer`
- Modify: `paket/postinst` (timer'ı `enable`, çalışan sistemde `start`)
- Test: `src/guncelleme.rs` içi; `scripts/paket-test.sh` (timer dosyaları pakette)

**Interfaces:**
- Consumes: Task 3 `Yollar`, `Hedef`, `Durum`, `kurulu`, `uzlas`.
- Produces:
  - `merkez::indir(yol: &str, govde: &str, cikti: &Path, max_sn: u32) -> Result<u16, String>` (curl `-o cikti -w %{http_code}`, gövde stdin'den, 300 sn); `pub type Indir = dyn Fn(&str, &str, &Path) -> Result<u16, String>`.
  - `pub fn calistir_guncelle(cfg: &Config, y: &Yollar, m: &Merkez, indir: &Indir, komut: &dyn Fn(&[&str]) -> bool, kurulu: &str, now: f64) -> Result<String, String>`. Adımlar: kilit (`kilit()` dosyası, `flock` LOCK_NB; alınamazsa `Err("zaten sürüyor")`); `uzlas`; `hedef.json` oku; `karar` yeniden (Kur değilse `Ok("gerek yok")`); denetim `GUNCELLEME_BASLADI surum= onceki=`; `paketler/<kurulu>.deb` yoksa `/api/paket {surum: kurulu}` ile indir (gelmezse `GUNCELLEME_HATA neden=onceki_paket_yok`, durum `hata`, dur); yeni paketi `paketler/<s>.deb.tmp`'ye indir, boyut + sha256 doğrula (tutmazsa sil, `neden=sha`), yeniden adlandır; `onceki.deb` = kurulu paketin kopyası; `denetle.sh` = `BETIK` (755); `sonuc` silinir; `bekleyen` = `{yeni, onceki, zaman}` (mtime = şimdi); sonra `DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades <paketler/s.deb>` (`komut`). apt başarısızsa hemen `apt-get install -y --allow-downgrades onceki.deb` ve `sonuc` = `geri` (uzlaşma sonraki raporda yazar). Başarıda `Ok("kuruldu, 10 dk sonra denetlenecek")`.
  - `pub const BETIK: &str` — aşağıdaki betik (sabit metin; test bunu `sh -n` ile denetler):

```sh
#!/bin/sh
# Güncelleme denetimi: kurulumdan ÖNCE eski ikili yazdı; yeni ikiliye güvenmez. Zamanlayıcı 5 dk'da bir çalıştırır.
D=/var/lib/wificorrect/guncelleme
[ -f "$D/bekleyen" ] || exit 0
[ -f "$D/sonuc" ] && exit 0
[ $(( $(date +%s) - $(stat -c %Y "$D/bekleyen") )) -ge 600 ] || exit 0
ok=1
for s in wificorrect-portal wificorrect-panel wificorrect-kaydedici dnsmasq wificorrect-guvenlik; do
  systemctl is-active -q "$s" || ok=0
done
[ -f "$D/saglik_basari" ] && [ "$D/saglik_basari" -nt "$D/bekleyen" ] || ok=0
if [ "$ok" = 1 ]; then echo tamam > "$D/sonuc"; exit 0; fi
echo geri > "$D/sonuc"
DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades "$D/onceki.deb"
```
  - Birim: `wificorrect-guncelleme-denetle.service` (`Type=oneshot`, `ExecStart=/bin/sh -c '[ -x /var/lib/wificorrect/guncelleme/denetle.sh ] && exec /var/lib/wificorrect/guncelleme/denetle.sh || true'`); `.timer` `OnBootSec=5min`, `OnUnitActiveSec=5min`, `WantedBy=timers.target`.
- `ctl guncelle`: gerçek `indir`/`komut`/`kurulu` ile `calistir_guncelle`; çıktıyı yazar; hata çıkış kodu 1.

- [ ] **Step 1: Testleri yaz** — sahte `indir` (dosyaya bayt yazar) ve kayıt tutan `komut`: (a) başarı: önceki paket indirildi, `onceki.deb`, `denetle.sh` 755 ve `BETIK`'e eşit, `bekleyen` JSON, apt komutu `--allow-downgrades …/paketler/0.4.1.deb`; (b) sha tutmuyor → apt çağrılmaz, durum `hata`/`sha`, `.tmp` silindi; (c) önceki paket alınamıyor → apt çağrılmaz, `onceki_paket_yok`; (d) apt başarısız → ikinci apt `onceki.deb`, `sonuc == "geri"`; (e) kilit tutuluyken ikinci çağrı `Err("zaten sürüyor")`; (f) `sh -n` ile `BETIK` sözdizimi (`Command::new("sh").arg("-n")`); (g) betik davranışı: geçici `D` ile (betikteki yolu `sed` yerine testte `D=` önüne yazılmış kopya), `bekleyen` mtime 11 dk önce, `saglik_basari` yok → `sonuc == geri` (apt/systemctl yerine PATH'e konmuş sahte `apt-get`, `systemctl` betikleri). `paket-test.sh`: timer dosyası pakette ve `systemctl is-enabled` (kapsayıcıda dosya varlığı yeterli).
- [ ] **Step 2:** `bash scripts/test.sh wificorrect guncelleme` → FAIL.
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** Süzgeç PASS; ardından tam takım `bash scripts/test.sh wificorrect` PASS.
- [ ] **Step 5: Commit** `cihaz: ctl guncelle, denetim betigi ve zamanlayici`

### Task 5: Sürüm numarası ve belgeler

**Files:**
- Modify: `Cargo.toml` (`version = "0.4.0"`), `Cargo.lock` (cihazda derlenince), `docs/KURULUM_GUNLUGU.md` (kısa bölüm: akış, dosyalar, ilk elle geçiş, yerel sürüm CLI, deneme adımları)

- [ ] **Step 1:** Sürüm 0.4.0; `bash scripts/gelistir.sh` derler (kurulum değil, yalnızca `cargo build` başarısı görülür) ve `Cargo.lock` geri gelir.
- [ ] **Step 2:** Belge bölümü.
- [ ] **Step 3: Commit** `surum 0.4.0, uzaktan guncelleme belgesi`

### Task 6: Kurulum ve uçtan uca deneme (denetleyici + kullanıcı)

- [ ] Merkez: dal itilir, kullanıcı `merkez-guncelle.sh`. Yönetim → Sürümler → "GitHub'dan al": v0.2.0/v0.3.0 gelir (indirme yolunun kanıtı).
- [ ] Cihazda (paket.sh ile, `dpkg-deb` cihazda var) üç yerel paket: `0.4.0` (bu dal), `0.4.1` (aynı ikili), `0.4.2` (ikili yerine `exit 1` betiği). Sunucuya kopyalanır, kullanıcı `sudo -u wcpanel python3 /usr/local/lib/wificorrect/surumler.py ekle …` çalıştırır.
- [ ] Canlı cihaza ilk elle geçiş: `/etc/wificorrect` ve `/etc/default` yedeği + ölü adam anahtarı (`systemd-run --on-active=300` geri alma) → `apt-get install ./wificorrect_0.4.0_amd64.deb` → yeni SSH ile erişim, servisler aktif, misafir oturumları duruyor → geri alma iptal. `dpkg-query` 0.4.0.
- [ ] Göztepe = `test`; test hedefi `0.4.1` + "Şimdi güncelle" → ≤ 5 dk `GUNCELLEME_BASLADI`, 10–15 dk içinde `GUNCELLEME_TAMAM`, Yönetim'de 0.4.1.
- [ ] Test hedefi `0.4.2` + "Şimdi güncelle" → kurulur, servisler düşer, ≤ 15 dk içinde betik `0.4.1`'i geri kurar; `GUNCELLEME_GERI_ALINDI`, Yönetim'de "Güncelleme başarısız (0.4.2)" sorunu; sonraki raporlarda 0.4.2 yeniden indirilmez.
- [ ] Test hedefi `0.4.0` + hemen → eski sürüme dönüş (`--allow-downgrades`) tamam. Hedef temizlenir; kanal Göztepe için `test` kalır. Sonuçlar kullanıcıya; PR.
