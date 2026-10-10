# Bağlı kullanıcılarda hız, bugünkü kullanım ve yavaşlatma — Uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kullanıcılar → Bağlı kullanıcılar tablosunda her cihazın anlık indirme/yükleme hızı ve bugünkü MB'si; tek tıkla
yavaşlatma (1/2/5/10/20 Mb/sn, kaldırılana kadar) ve Yavaşlatılmış cihazlar bölümü.

**Architecture:** nft `inet wfc_hiz` tablosu IP başına dinamik set + counter ile sayar ve `sinif` zincirinde
`meta priority` ile sınırlı cihazları HTB sınıfına yollar. Kaydedici 5 sn'de bir setleri okur (`trafik.rs`), hızı ve günlük
toplamı hesaplayıp `/run/wificorrect/trafik.json`'a yazar, sınırları (`hiz.rs`, `state_root/hiz_sinir.json`) tc + nft'ye
uzlaştırır. Panel yalnızca dosyaları okur/yazar ve `ctl hiz-uygula` tetikler.

**Tech Stack:** Rust (mevcut ikili `wificorrect`, serde_json), nftables 1.1.3, tc/HTB + fq_codel, betiksiz HTML.

**Spec:** `docs/superpowers/specs/2026-10-10-hiz-ve-kullanim-design.md` (onaylı; renk vurgusu yok).

## Global Constraints

- Testler cihazda: `bash scripts/test.sh wificorrect <süzgeç>` (aynı anda iki tane çalıştırma; tamamı ~5 dk).
- Hız: son ~10 sn ortalaması, **megabit/saniye**, 1 ondalık, Türkçe biçim (`38,4`). Bugün: **MB = 10⁶ bayt**, tam sayı, binlik nokta (`4.820`). Gün Türkiye saatiyle (`ortak::now_iso` / `day_of`).
- Hız seçenekleri tam olarak `[1, 2, 5, 10, 20]` Mb/sn; hem indirme hem yükleme.
- Sınır **MAC'e bağlı**, kullanıcı kaldırana kadar sürer.
- Arayüz adları sabit yazılmaz: LAN = `cfg.main.iface` (`br-hotspot`), WAN = `ag.toml`'daki `wan` (`crate::ag` okuyucusu; `hiz.nft` için `/etc/wificorrect/arayuzler.nft`'deki `$WAN`).
- `wificorrect-guvenlik.service` güncellemede **yeniden başlatılmaz** (`conntrack -F` herkesi düşürür).
- Sayfa betiksiz (CSP aynen); yenileme `<meta http-equiv="refresh" content="10">`.
- Anlık hız için renk vurgusu **yok**.
- Metinler: düğme "Yavaşlat", seçenek başlığı "Hız sınırı seçin", seçenekler "N Mb/sn", rozet "≤ N Mb/sn", düğme "Yavaşlatmayı kaldır", izinli satır adı "İzinli cihaz" (notu varsa "İzinli cihaz · <not>").
- Denetim: her bakış `PANEL_OTURUMLAR` (bugünkü gibi); yenileme açılınca `PANEL_OTURUMLAR_YENILE` "Otomatik yenilemeyi açtı" (kişisel veri listesinde); `oto=1` istekleri pencere (900 sn) içindeyse kaydedilmez; `PANEL_YAVASLAT` "Yavaşlattı", `PANEL_YAVASLATMA_KALDIR` "Yavaşlatmayı kaldırdı", `PANEL_YAVASLATILMIS` "Yavaşlatılmış cihazlara baktı" (kişisel veri listesinde).
- Canlı cihaza ilk tc/nft kurulumu **ölü adam anahtarıyla** (yedek + `systemd-run --on-active=300` geri alma + yeni bağlantıyla doğrula + iptal).

## Review Focus

- **Sayaç geri gidiyor** (set öğesi zaman aşımıyla düşüp yeniden eklendi, ya da tablo yeniden yüklendi): hız negatif/dev çıkmamalı, o aralık 0; bugün toplamı geri gitmemeli → Task 1 testi `sayac_azalinca_hiz_sifir_bugun_gerilemez`.
- **IP başka cihaza geçti** (DHCP): eski MAC'in bugünkü toplamı yeni sahibine yazılmamalı → Task 1 testi `ip_sahibi_degisince_toplam_dogru_maca`.
- **trafik.json yok / bozuk / 30 sn'den eski** (kaydedici durmuş): sayfa açılmalı, hız ve MB "—", altta "Ölçüm alınamıyor" → Task 4 testi `olcum_yoksa_tire_ve_not`.
- **Sahte/bozuk form değeri** (`hiz=3`, `hiz=abc`, tanınmayan MAC): sınır yazılmamalı, hata mesajıyla dönmeli → Task 5 testi `yavaslat_gecersiz_deger_reddedilir`.
- **tc başarısız** (modül yok, arayüz yok): kapı ve sayım sürmeli; panelde "Yavaşlatma uygulanamadı" uyarısı → Task 3 testi `tc_hatasi_trafik_jsona_yazilir` + Task 5 testi `hiz_hatasi_panelde_gorunur`.

---

### Task 1: `trafik.rs` — set ayrıştırma, hız ve günlük toplam

**Files:**
- Create: `src/trafik.rs` (+ `mod trafik;` in `src/main.rs`)

**Interfaces:**
- Produces:
  - `pub const DURUM_YOLU: &str = "/run/wificorrect/trafik.json";`
  - `pub fn parse_set(json: &str) -> BTreeMap<String, u64>` — `nft -j list set` çıktısından IP → bayt; bozuk girdi → boş harita.
  - `#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)] pub struct Cihaz { pub ip: String, pub indir_bps: u64, pub yukle_bps: u64, pub bugun_bayt: u64 }`
  - `#[derive(Serialize, Deserialize, Clone, Default, Debug)] pub struct Durum { pub zaman: f64, pub cihazlar: BTreeMap<String /*mac*/, Cihaz>, #[serde(default)] pub hiz_hata: Option<String> }`
  - `pub struct Olcer` + `pub fn Olcer::new(gun: GunToplam) -> Olcer` + `pub fn ornek(&mut self, t: f64, gun: &str, indir: &BTreeMap<String,u64>, yukle: &BTreeMap<String,u64>, ip_mac: &BTreeMap<String,String>) -> Durum` + `pub fn gun_toplam(&self) -> &GunToplam`
  - `#[derive(Serialize, Deserialize, Clone, Default, Debug)] pub struct GunToplam { pub gun: String, pub bayt: BTreeMap<String /*mac*/, u64> }`; `pub fn gun_oku(state_root: &str) -> GunToplam`, `pub fn gun_yaz(state_root: &str, g: &GunToplam) -> std::io::Result<()>` (dosya `trafik_gun.json`, `ortak::write_atomic`).
  - `pub fn oku(path: &Path) -> Option<Durum>`, `pub fn yaz(path: &Path, d: &Durum) -> std::io::Result<()>` (dizini oluşturur, atomik).

- [ ] **Step 1: Failing tests** (in `src/trafik.rs` `mod tests`)
  - `parse_set_nft_json`: fixture (cihazda 2026-10-10 alınan gerçek biçim):
    `{"nftables": [{"metainfo": {"version": "1.1.3", "json_schema_version": 1}}, {"set": {"family": "inet", "name": "indir", "table": "wfc_hiz", "type": "ipv4_addr", "flags": ["timeout", "dynamic"], "timeout": 86400, "elem": [{"elem": {"val": "10.50.0.93", "expires": 86399, "counter": {"packets": 3, "bytes": 252}}}, {"elem": {"val": "10.50.0.57", "expires": 86399, "counter": {"packets": 1, "bytes": 84}}}]}}]}`
    → `{"10.50.0.93": 252, "10.50.0.57": 84}`; `parse_set("bozuk")` boş; `elem` yoksa boş.
  - `hiz_on_saniye_ortalamasi`: örnekler t=0 (0 B), t=5 (6_250_000 B), t=10 (12_500_000 B) indir, ip_mac `{10.50.0.93: aa:..:01}` → t=10'daki `indir_bps == 10_000_000` (10 Mb/sn); t=15 (18_750_000) → hâlâ 10_000_000 (pencere 10 sn kayar).
  - `bugun_toplam_birikir_gun_degisince_sifir`: aynı örneklerle `bugun_bayt == indir+yükle farklarının toplamı`; `gun` "2026-10-11" olunca ilk örnekte `bugun_bayt == 0`, `gun_toplam().gun == "2026-10-11"`.
  - `sayac_azalinca_hiz_sifir_bugun_gerilemez`: 1_000_000 → 400 B: `indir_bps == 0`, `bugun_bayt` önceki değer + 400 (yeni öğenin baytı).
  - `ip_sahibi_degisince_toplam_dogru_maca`: 10.50.0.93 önce mac A, sonra ip_mac'te mac B: B'nin `bugun_bayt`'ı yalnızca B dönemindeki farkla başlar; A'nınki korunur (A artık `cihazlar`'da yok ama `gun_toplam().bayt[A]` duruyor).
  - `ip_mac_te_olmayan_ip_yok_sayilir`.
  - `durum_yaz_oku_gidis_donus` (geçici dizinde).
- [ ] **Step 2: Run** `bash scripts/test.sh wificorrect trafik::` → derleme hatası/FAIL.
- [ ] **Step 3: Implement.** Olcer IP başına son örnekleri `VecDeque<(f64, u64)>` olarak tutar (indir ve yükle ayrı); hız = (son − en eski örnek ki `t_son − t ≥ 9.0`'ı sağlayan en yeni; yoksa en eski) × 8 / Δt; Δt < 1 → 0; aralıkta azalma varsa 0. Günlük fark: `yeni ≥ eski ? yeni − eski : yeni`, IP'nin o örnekteki MAC'ine eklenir; IP'nin MAC'i değişince o IP'nin geçmiş örnekleri silinir (yeni sahibe eski bayt yazılmaz). Yalnızca `ip_mac`'te olan IP'ler `cihazlar`'a girer.
- [ ] **Step 4: Run** `bash scripts/test.sh wificorrect trafik::` → PASS.
- [ ] **Step 5: Commit** `git commit -m "trafik: nft set sayaçlarından hız (10 sn) ve günlük MB"`

### Task 2: `hiz.rs` — sınır kaydı, nft zinciri ve tc komutları

**Files:**
- Create: `src/hiz.rs` (+ `mod hiz;`)

**Interfaces:**
- Produces:
  - `pub const HIZLAR: [u32; 5] = [1, 2, 5, 10, 20];`
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct Sinir { pub hiz: u32, pub ad: String, pub zaman: String, pub kim: String }`
  - `pub type Sinirlar = BTreeMap<String /*mac*/, Sinir>;` `pub fn oku(state_root: &str) -> Sinirlar` (dosya `hiz_sinir.json`; yok/bozuk → boş), `pub fn kaydet(state_root: &str, s: &Sinirlar) -> std::io::Result<()>`
  - `pub struct Plan { pub nft: String, pub tc: Vec<Vec<String>> }`; `pub fn plan(lan: &str, wan: &str, sinirli: &[(String /*ip*/, u32 /*Mb/sn*/)]) -> Plan` — `sinirli` IP'ye göre sıralı gelir; sınıf numarası `0x10 + sıra`.
  - `pub fn parmak_izi(p: &Plan) -> String` (nft + tc'nin birleşik metni; uygulanmış durumu karşılaştırmak için).
  - `pub const UYGULANAN_YOLU: &str = "/run/wificorrect/hiz_uygulanan";`
- [ ] **Step 1: Failing tests**
  - `plan_bos_yalniz_kok_ve_varsayilan`: `plan("br-hotspot","enp3s0",&[])` → `nft == "flush chain inet wfc_hiz sinif\n"`; tc her iki arayüz için tam olarak: `tc qdisc replace dev X root handle 1: htb default 1`, `tc class replace dev X parent 1: classid 1:1 htb rate 1gbit`, `tc qdisc replace dev X parent 1:1 handle 2: fq_codel`.
  - `plan_iki_sinir`: `[("10.50.0.57",5),("10.50.0.93",2)]` → nft satırları
    `add rule inet wfc_hiz sinif oifname "br-hotspot" ip daddr 10.50.0.57 meta priority set 1:10`,
    `add rule inet wfc_hiz sinif oifname "enp3s0" ip saddr 10.50.0.57 meta priority set 1:10`, aynı ikili `10.50.0.93 … 1:11`;
    tc'de her arayüzde (kök + varsayılan üç komuttan sonra) `tc class replace dev X parent 1: classid 1:10 htb rate 5mbit ceil 5mbit` + `tc qdisc replace dev X parent 1:10 handle 10: fq_codel`, `1:11 … 2mbit` + `handle 11:`.
    `plan` `tc qdisc del` içermez; eski sınıfları temizlemek için kökü silmek Task 3'teki `uygula`'nın işidir.
  - `oku_kaydet_gidis_donus`, `bozuk_dosya_bos`.
- [ ] **Step 2: Run** `bash scripts/test.sh wificorrect hiz::` → FAIL.
- [ ] **Step 3: Implement** (saf metin üretimi; IO yalnızca `oku`/`kaydet`).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `git commit -m "hiz: yavaşlatma kaydı, nft sınıf zinciri ve HTB komutları"`

### Task 3: nft dosyası, kaydedicide 5 sn adımı, `ctl hiz-uygula`

**Files:**
- Create: `deploy/debian/etc/wificorrect/hiz.nft`
- Modify: `deploy/debian/etc/systemd/system/wificorrect-guvenlik.service` (kapi.nft'den sonra `ExecStart=-/usr/sbin/nft -f /etc/wificorrect/hiz.nft`; başarısızlık açılışı durdurmaz)
- Modify: `src/kaydedici.rs` (5 sn adımı), `src/ctl.rs` (`hiz-uygula`), `src/hiz.rs` (uygulayıcı)

**Interfaces:**
- Consumes: Task 1 (`parse_set`, `Olcer`, `gun_oku/gun_yaz`, `yaz`, `DURUM_YOLU`), Task 2 (`oku`, `plan`, `parmak_izi`, `UYGULANAN_YOLU`).
- Produces:
  - `hiz.nft`: `include "/etc/wificorrect/arayuzler.nft"`; `table inet wfc_hiz` (önce `table inet wfc_hiz` + `delete table inet wfc_hiz` YOK — sayaçlar korunmalı; dosya yalnızca tablo yokken yüklenir). Setler `indir`/`yukle` (`type ipv4_addr; flags dynamic, timeout; timeout 1d; counter`), zincir `say` (`hook forward priority filter - 10; policy accept;` `iifname "br-hotspot" oifname $WAN update @yukle { ip saddr }`, `iifname $WAN oifname "br-hotspot" update @indir { ip daddr }`), boş zincir `sinif` (`hook forward priority filter - 9; policy accept;`). Yalnızca sayar/işaretler; accept/drop yok.
  - `pub fn hiz::uygula(cfg: &Config, wan: &str, ip_of: &BTreeMap<String /*mac*/, String /*ip*/>, runner: &Runner, nft_f: &dyn Fn(&str) -> bool) -> Result<bool /*değişti*/, String>` — `oku` → sınırlı MAC'lerin güncel IP'si (yoksa atlanır) → `plan` → `parmak_izi` `UYGULANAN_YOLU`'ndakiyle aynıysa `Ok(false)`; değilse her arayüzde `tc qdisc del dev X root` (hata yok sayılır) + plan tc komutları + `nft_f(&plan.nft)`; hepsi başarılıysa parmak izini yazar, `Ok(true)`; tc/nft hatası → `Err("Yavaşlatma uygulanamadı: …")`.
  - `Kaydedici::trafik_adimi(&mut self, now: f64, indir_json: &str, yukle_json: &str) -> trafik::Durum` — ip_mac = oturumlardan (`ip` boş olmayan) + `cfg.allow` ∩ `self.leases`; `Olcer::ornek`; `hiz_hata` alanını son `uygula` sonucundan doldurur; `trafik::yaz(DURUM_YOLU)`; 60 sn'de bir `gun_yaz`.
  - `run` döngüsü: açılışta ve her adımda `nft list table inet wfc_hiz` başarısızsa `nft -f /etc/wificorrect/hiz.nft`; 5 sn'de bir iki `nft -j list set inet wfc_hiz indir|yukle` (`ortak::capture`) → `trafik_adimi` → `hiz::uygula`. Kaydedici yolları testte değiştirilebilir alan olarak tutar (`trafik_yolu`, `uygulanan_yolu`).
  - `wificorrect ctl hiz-uygula` → `hiz::uygula` bir kez (ip_of: oturumlar + izinli ∩ kiralar), çıktı "uygulandı"/"değişiklik yok"/hata metni, hata → çıkış kodu 1.
- [ ] **Step 1: Failing tests** (`src/kaydedici.rs` ve `src/hiz.rs` testleri)
  - `trafik_adimi_json_yazar`: setup() + bir oturum (mac M1, ip 10.50.0.23) + `cfg.allow` MAC M3 ve kirada 10.50.0.30; iki çağrı (t, t+10) sabit JSON'larla → yazılan dosyada M1 ve M3 var, M1 `indir_bps` beklenen, oturumsuz/izinsiz IP yok.
  - `uygula_degismeyince_komut_yok`: aynı sınırlarla iki `uygula` → ikincisinde runner/nft_f çağrılmaz, `Ok(false)`.
  - `uygula_ip_degisince_yeniden`: sınırlı MAC'in IP'si değişince yeni IP'yle nft metni.
  - `tc_hatasi_trafik_jsona_yazilir`: runner `tc` için false döner → `uygula` Err; ardından `trafik_adimi` çıktısında `hiz_hata == Some("Yavaşlatma uygulanamadı: …")`.
  - `ctl_hiz_uygula_cikis_kodu` (ctl testlerinin kalıbıyla).
- [ ] **Step 2: Run** `bash scripts/test.sh wificorrect "kaydedici:: hiz::"` → FAIL. (süzgeç tek kelime alır; iki ayrı çalıştırma yapılabilir)
- [ ] **Step 3: Implement** yukarıdaki imzalarla. `nft -f` için stdin'den besleme: `nft_f` gerçek hâlinde geçici dosyaya (`/run/wificorrect/hiz.nft.tmp`) yazıp `nft -f` çalıştırır.
- [ ] **Step 4: Run** tüm testler `bash scripts/test.sh wificorrect` → PASS; `sh scripts/paket-test.sh` CI'da `hiz.nft`'nin pakette olduğunu da denetlesin (paket-test.sh'e `dpkg -L wificorrect | grep -q /etc/wificorrect/hiz.nft` satırı).
- [ ] **Step 5: Commit** `git commit -m "kaydedici: 5 sn'de bir trafik ölçümü ve yavaşlatma uzlaştırması; hiz.nft; ctl hiz-uygula"`

### Task 4: Bağlı kullanıcılar tablosu — hız, MB, sıralama, yenileme, izinli cihazlar

**Files:**
- Modify: `src/panel.rs` (`oturumlar_sayfa`, `page`, `Panel` alanı `trafik_path: PathBuf` = `trafik::DURUM_YOLU`, testte `env()` içinde geçici yol), `src/hesap.rs` (`Oturum.yenile_acildi: f64`, `Oturumlar::set_yenile(token, t)`), `src/sablon/panel.html` (`$bas_ek` `<head>` içinde; `.arac`, `.ayrac` stilleri), `src/panel/hareketler.rs` (etiketler, KISISEL)

**Interfaces:**
- Consumes: `trafik::{oku, Durum, Cihaz}` (Task 1), `hiz::oku` (Task 2, rozet için).
- Produces: `fn mbps(bps: u64) -> String` ("38,4"), `fn mb(bayt: u64) -> String` ("4.820"); sorgu parametreleri `sira ∈ {hiz (vars.), mb, saat}`, `yenile=1`, `oto=1`; `fn oto_yenileme(&self, req: &Req, o: &Oturum, now: f64) -> bool` (Task 5 de kullanır).
- Sütunlar (sırayla): Durum, Telefon, Ad soyad, MAC, IP, "↓ İndirme Mb/sn", "↑ Yükleme Mb/sn", "Bugün MB", Başlangıç, Kalan, Yavaşlat (Task 5 doldurur; bu taskta boş hücre), "" (Bağlantıyı kes; izinli satırda boş).
- [ ] **Step 1: Failing tests** (`src/panel.rs` tests; `o_an_bagli_cihaz_yesil_rozetle` kurgusunu örnek al, `trafik.json` geçici yola yazılır, `zaman` = test saati)
  - `hiz_ve_mb_sutunlari`: Ayşe `indir_bps 38_400_000, yukle_bps 2_100_000, bugun_bayt 4_820_000_000` → satırda `38,4`, `2,1`, `4.820`; başlıklar metinleri birebir.
  - `varsayilan_siralama_anlik_hiz`: iki kişi, hızı yüksek olan önce; `?sira=mb` bugün MB büyük olan önce; `?sira=saat` başlangıca göre.
  - `izinli_cihaz_satiri`: `cfg.allow` (not "Kasa") + kira → satırda "İzinli cihaz · Kasa", MAC, hız; telefon boş; "Bağlantıyı kes" yok.
  - `olcum_yoksa_tire_ve_not`: dosya yok → hız/MB hücreleri "—", sayfada "Ölçüm alınamıyor"; `zaman` 31 sn eski → aynı; 5 sn eski → "Ölçüm 5 sn önce".
  - `yenileme_meta_ve_denetim`: `?yenile=1` → `<meta http-equiv="refresh" content="10;url=/cihazlar?sira=hiz&amp;yenile=1&amp;oto=1#oturumlar">` ve denetimde bir `PANEL_OTURUMLAR_YENILE`; ardından `?yenile=1&oto=1` üç kez → yeni denetim satırı yok; saat 901 sn ileri → `oto=1` isteği bir `PANEL_OTURUMLAR` yazar ve pencereyi yeniler; `oto=1` ile ama yenileme hiç açılmamış oturum → `PANEL_OTURUMLAR` yazılır.
  - mevcut `o_an_bagli_cihaz_yesil_rozetle` ve `owner_activity_is_logged…` geçmeye devam etmeli (sayılar değişirse nedenini yorumla düzelt).
- [ ] **Step 2: Run** `bash scripts/test.sh wificorrect panel::` → FAIL.
- [ ] **Step 3: Implement.** Sırala/yenile çubuğu taslaktaki gibi (`.eylemler.arac`; seçili sıralama `dugme`, diğerleri `dugme ikincil`; sağda "⟳ 10 sn'de bir yenile: kapalı|açık" bağlantısı, açıkken `yenile`'siz URL'ye götürür). Yenileme yalnızca `/cihazlar` ve `/oturumlar` sayfasında `bas_ek`'e meta yazar (`page` için yeni parametre yerine `PanelBas` benzeri iş parçacığı yerel değişken ya da `grup_sayfasi`'ndan geçen `bas_ek: String`; uygulayıcı seçer, diğer sayfalarda boş).
- [ ] **Step 4: Run** `bash scripts/test.sh wificorrect` → PASS.
- [ ] **Step 5: Commit** `git commit -m "panel: bağlı kullanıcılarda anlık hız, bugün MB, sıralama, 10 sn yenileme, izinli cihazlar"`

### Task 5: Yavaşlat / Yavaşlatmayı kaldır ve Yavaşlatılmış cihazlar bölümü

**Files:**
- Modify: `src/panel.rs` (rotalar, `BOLUMLER`'e `("/cihazlar", "/yavaslatilmis", "yavaslatilmis", "Yavaşlatılmış cihazlar", false)` — Yasaklı/İzinli'den sonra, Yasaklı siteler'den önce; `bolum_simgesi("yavaslatilmis")` yeni simge), `src/panel/hareketler.rs`, `src/sablon/panel.html` (`details.yavas`, `.yavas-menu`, `.rozet.sinirli` — taslak v3 stilleri)

**Interfaces:**
- Consumes: `hiz::{HIZLAR, Sinir, oku, kaydet}` (Task 2), `oto_yenileme` (Task 4), `trafik::Durum.hiz_hata`.
- Produces: `POST /oturumlar/yavaslat` (`mac`, `hiz`, `csrf`), `POST /oturumlar/yavaslatma-kaldir` (`mac`, `donus ∈ {oturumlar, yavaslatilmis}`, `csrf`); `GET /yavaslatilmis`.
- [ ] **Step 1: Failing tests**
  - `yavaslat_kaydeder_ve_uygular`: Ayşe satırında `<details class="yavas"><summary class="dugme ikincil">Yavaşlat</summary>` ve beş `N Mb/sn` formu (`action="/oturumlar/yavaslat"`, gizli `hiz`); `hiz=5` POST → `hiz_sinir.json`'da `{hiz:5, ad:"Ayşe Y", kim: MUSTERI}`, runner `["wificorrect","ctl","hiz-uygula"]` çağrıldı (`/usr/local/bin/wificorrect` yolu, mevcut kalıp), yönlendirme `/cihazlar#oturumlar`, denetimde `PANEL_YAVASLAT … mac=… hiz=5`; sayfada satırda `≤ 5 Mb/sn` rozeti + "Yavaşlatmayı kaldır", Yavaşlat düğmesi yok.
  - `yavaslat_gecersiz_deger_reddedilir`: `hiz=3`, `hiz=abc`, bilinmeyen MAC, CSRF'siz → dosya değişmez, hata mesajı (`e=1`) / 403.
  - `yavaslatilmis_bolumu_ve_kaldir`: bağlı olmayan sınırlı MAC → bölümde MAC, son ad, "5 Mb/sn", zaman, kim ve "Yavaşlatmayı kaldır"; kaldır POST (`donus=yavaslatilmis`) → kayıt silinir, runner çağrıldı, yönlendirme `/cihazlar#yavaslatilmis`, denetimde `PANEL_YAVASLATMA_KALDIR`; bölüme bakış `PANEL_YAVASLATILMIS` (oto yenilemede yazılmaz).
  - `hiz_hatasi_panelde_gorunur`: `trafik.json` `hiz_hata` dolu → bölüm başında `mesaj hata` içinde o metin.
  - `menu_dort_grup_ve_bolumler_tek_sayfada` id listesine `("yavaslatilmis", "/oturumlar/yavaslatma-kaldir")` değil yalnızca `("yavaslatilmis", "")` ekle (boş liste "Yavaşlatılmış cihaz yok").
- [ ] **Step 2: Run** `bash scripts/test.sh wificorrect panel::` → FAIL.
- [ ] **Step 3: Implement.** MAC doğrulaması: oturumlarda ya da izinli + kirada olmalı (yavaşlat); kaldır için kayıtta olmalı. `ad`: oturumdan "Ad Soyad", izinliden "İzinli cihaz · not".
- [ ] **Step 4: Run** `bash scripts/test.sh wificorrect` → PASS (tamamı).
- [ ] **Step 5: Commit** `git commit -m "panel: yavaşlat (1/2/5/10/20 Mb/sn), yavaşlatmayı kaldır, Yavaşlatılmış cihazlar bölümü"`

### Task 6: Canlı cihaza kurulum (ölü adam anahtarı) ve sahte istemciyle ölçüm gösterimi

**Files:**
- Create: `scripts/hiz-deneme.sh` (cihazda çalışır; ad alanı kurar, ölçer, temizler)

**Interfaces:**
- Consumes: Task 3–5'in kurulu hâli; `wificorrect ctl hiz-uygula`.

- [ ] **Step 1: Yedek + ölü adam anahtarı.** Cihazda `nft list ruleset > /root/yedek-nft-$(date +%s).nft`, `tc qdisc show > /root/yedek-tc.txt`; geri alma birimi:
  `systemd-run --unit wfc-geri-al --on-active=300 sh -c 'nft delete table inet wfc_hiz; tc qdisc del dev br-hotspot root; tc qdisc del dev enp3s0 root; tc qdisc replace dev enp3s0 root fq_codel'`.
- [ ] **Step 2: Kur.** `bash scripts/gelistir.sh` + `ssh wificorrect 'systemctl restart wificorrect-panel wificorrect-portal wificorrect-kaydedici'`. Kaydedici `inet wfc_hiz`'i kendisi yükler.
- [ ] **Step 3: Doğrula (YENİ SSH bağlantısıyla).** `nft list table inet wfc_hiz` var; `tc qdisc show dev br-hotspot` ve `dev enp3s0` HTB; `/run/wificorrect/trafik.json` 5 sn'de bir güncelleniyor ve gerçek müşteriler listede; panel.wificorrect.com → Kullanıcılar açılıyor; misafir internet çalışıyor (portal). Hepsi tamamsa `systemctl stop wfc-geri-al.timer`; değilse bekle, 5 dk'da geri alınır.
- [ ] **Step 4: `scripts/hiz-deneme.sh`** — ad alanı `wfcdeneme`, veth `wfcd0`↔`wfcd1`, `wfcd0` `br-hotspot`'a bağlanır, test MAC `02:57:fc:00:00:01`, ad alanında `dhclient`/sabit kira ile 10.50.0.x; `wificorrect ctl` ya da `nft add element inet hotspot allow_mac { 02:57:fc:00:00:01 }` ile geçici izin (cfg.allow'a "Hız denemesi" notuyla eklenir ki tabloda görünsün; deneme sonunda kaldırılır); internetteki test dosyasını (`https://speed.cloudflare.com/__down?bytes=200000000`) `curl -o /dev/null -w '%{speed_download}'` ile indirir; aynı anda 5 sn'de bir `trafik.json`'dan test MAC'inin `indir_bps`'ini yazar. Sonra `hiz_sinir.json`'a 5 Mb/sn ekleyip `ctl hiz-uygula`, yeniden indirir; sonra kaldırır. Çıkışta (trap) ad alanı, veth, izin ve sınır silinir.
- [ ] **Step 5: Çalıştır ve raporla.** Beklenen: (1) curl ortalaması ile trafik.json indirme hızı ±%10; (2) 5 Mb/sn sınırda curl ≈ 5 Mb/sn (±%15), kaldırınca geri yükselir; (3) bugün MB indirilen boyut kadar arttı; (4) deneme sırasında diğer müşterilerin `trafik.json` hızları sınırdan etkilenmedi. Sonuçları (sayılarla) kullanıcıya göster; panelde sahte istemcinin satırının ekran görüntüsü.
- [ ] **Step 6: Commit** `git commit -m "scripts: hiz-deneme.sh — sahte istemciyle hız ölçümü ve yavaşlatma doğrulaması"`; dalı it, PR (`hiz-olcer` → `main`, `ozet-tasarim` birleştikten sonra) kullanıcı onayıyla.
