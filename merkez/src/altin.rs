//! Altın kayıt (yalnızca testler): Python testlerinin cevapları, veritabanlarının son hâli ve geçici dizinlerindeki dosyalar
//! (scripts/sunucu/parite/kaydet.py → tests/altin/<dosya>/<test>.json[.gz]) Rust'taki aynı testinkiyle karşılaştırılır.
//! Kayıt biçimi (2) ve normalleştirme kaydet.py'nin başındaki tarifin aynısıdır.
//!
//! Kullanım: testin ilk satırı `let _k = Kayit::bas("test_yonetim", "test_x");` (Python dosyasının ve testinin adı).
//! Kayıt açıkken bu iş parçacığında rastgelelik belirlenimlidir; `test_yardim`'ın istek yardımcıları cevapları,
//! `test_yardim::gecici_dizin` geçici dizinleri, `Veri::ac` veritabanı dosyalarını bildirir. Kayıt düşünce (test panik
//! etmediyse) karşılaştırır, ilk farkı yazıp panik eder.
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::guvenlik::rastgele;
use crate::web::{self, Istek, Yanit};

const BICIM: u64 = 2;
/// Karşılaştırılan başlıklar (küçük harfle) ve `x-wfc-` ile başlayanlar. Uygulamanın başka başlık vermesi farktır
/// (Python kaydedicisi de reddeder): gerekiyorsa iki tarafta da listeye eklenir.
const SECILI_BASLIKLAR: [&str; 11] = [
    "location",
    "set-cookie",
    "content-type",
    "cache-control",
    "clear-site-data",
    "content-disposition",
    "x-content-type-options",
    "referrer-policy",
    "x-frame-options",
    "content-security-policy",
    "strict-transport-security",
];
/// Bundan kısa üretilen değerler belirteç sayılmaz (sıradan metinle karışmasın).
const BELIRTEC_EN_AZ: usize = 8;
const SQLITE_IMZA: &[u8] = b"SQLite format 3\0";
/// kaydet.py'nin iki koşusunda farklı çıkan yaprak (gz baytının ya da dosya zamanının özeti): yalnızca dosya içeriklerinde,
/// `kararsiz_izinli` yerlerde olabilir; karşılaştırılmaz (yine bir yaprak olmalı).
const KARARSIZ: &str = "<KARARSIZ>";
/// Dosya JSON'unun üst düzeyinde kararsız olabilecek alanlar (rapor `imza`, `manifest_sha`).
const KARARSIZ_JSON_ALANLARI: [&str; 2] = ["imza", "manifest_sha"];
/// Metni kararsız olabilecek dosyalar (gz baytlarının özetleri).
const KARARSIZ_METINLER: [&str; 2] = ["MANIFEST.sha256", "zincir.txt"];
const GUNLUK_EKLERI: [&str; 3] = ["-journal", "-wal", "-shm"];

#[derive(Default)]
struct Durum {
    /// (istek gösterimi, yanıt)
    cevaplar: Vec<(String, Yanit)>,
    /// Açılan veritabanı dosyaları, ilk açılış sırasıyla (yol başına bir kez).
    veritabanlari: Vec<PathBuf>,
    /// Testin geçici dizinleri, açılış sırasıyla.
    dizinler: Vec<PathBuf>,
}

thread_local! {
    static ETKIN: RefCell<Option<Durum>> = const { RefCell::new(None) };
}

/// Bir testin altın kaydı. Düşünce karşılaştırır (`altinsiz` değilse). İş parçacığına bağlıdır (Send değil).
pub struct Kayit {
    dosya: String,
    test: String,
    /// Yalnızca spec'teki bilinçli farklar: belirlenimli kip açılır, karşılaştırılmaz.
    altinsiz: Option<String>,
    _kip: rastgele::Belirlenimli,
    _is_parcacigi: PhantomData<*const ()>,
}

// Derleme anında: Kayit Send olsaydı iki uygulama da uyar, `<Kayit as SendIse<_>>` belirsiz kalır ve derleme durur
// (static_assertions'ın assert_not_impl_any! yöntemi).
trait SendIse<A> {
    fn yok() {}
}
impl<T: ?Sized> SendIse<()> for T {}
impl<T: ?Sized + Send> SendIse<u8> for T {}
const _: fn() = || {
    let _ = <Kayit as SendIse<_>>::yok;
};

impl Kayit {
    /// Python `tests/<dosya>.py` içindeki `<test>`'in kaydı: `Kayit::bas("test_yonetim", "test_yeni_musteri_parola_bir_kez")`.
    pub fn bas(dosya: &str, test: &str) -> Kayit {
        Kayit::yeni(dosya, test, None)
    }

    /// Altın kayıtla karşılaştırılmayan test (yalnızca spec'teki bilinçli farklar; neden defterine de yazılır).
    pub fn altinsiz(neden: &str) -> Kayit {
        assert!(!neden.trim().is_empty(), "altınsız kayıt için neden yazılmalı");
        Kayit::yeni("", "", Some(neden.to_string()))
    }

    fn yeni(dosya: &str, test: &str, altinsiz: Option<String>) -> Kayit {
        ETKIN.with(|e| {
            let mut e = e.borrow_mut();
            assert!(e.is_none(), "bu iş parçacığında zaten açık bir altın kayıt var");
            *e = Some(Durum::default());
        });
        Kayit { dosya: dosya.into(), test: test.into(), altinsiz, _kip: rastgele::belirlenimli(), _is_parcacigi: PhantomData }
    }
}

impl Drop for Kayit {
    fn drop(&mut self) {
        let durum = ETKIN.with(|e| e.borrow_mut().take());
        if std::thread::panicking() {
            return;
        }
        let Some(d) = durum else {
            panic!("altın kayıt durumu bu iş parçacığında yok ({}/{})", self.dosya, self.test);
        };
        if self.altinsiz.is_some() {
            return;
        }
        if let Err(m) = karsilastir(&self.dosya, &self.test, &d) {
            panic!("altın kayıt farkı ({}/{}): {m}", self.dosya, self.test);
        }
    }
}

/// Bu iş parçacığında kayıt açıksa yanıtı ekler (`test_yardim`'ın istek yardımcıları çağırır).
pub(crate) fn cevap_ekle(i: &Istek, y: &Yanit) {
    ETKIN.with(|e| {
        if let Some(d) = e.borrow_mut().as_mut() {
            let mut s = format!("{} {}", i.yontem, i.yol);
            if !i.sorgu.is_empty() {
                s.push('?');
                s.push_str(&i.sorgu);
            }
            d.cevaplar.push((s, y.clone()));
        }
    });
}

/// Bu iş parçacığında kayıt açıksa veritabanı dosyasını bildirir (`Veri::ac` çağırır).
pub(crate) fn veri_acildi(yol: &Path) {
    ETKIN.with(|e| {
        if let Some(d) = e.borrow_mut().as_mut() {
            let tam = if yol == Path::new(":memory:") { yol.to_path_buf() } else { std::fs::canonicalize(yol).unwrap_or_else(|_| yol.to_path_buf()) };
            if !d.veritabanlari.contains(&tam) {
                d.veritabanlari.push(tam);
            }
        }
    });
}

/// Bu iş parçacığında kayıt açıksa geçici dizini bildirir (`test_yardim::gecici_dizin` çağırır; Python'da
/// `tempfile.mkdtemp`/`TemporaryDirectory`).
pub(crate) fn dizin_acildi(yol: &Path) {
    ETKIN.with(|e| {
        if let Some(d) = e.borrow_mut().as_mut() {
            let tam = std::fs::canonicalize(yol).unwrap_or_else(|_| yol.to_path_buf());
            if !d.dizinler.contains(&tam) {
                d.dizinler.push(tam);
            }
        }
    });
}

// --- kayıt biçimi (kaydet.py ile birebir) ---

fn secili(ad: &str) -> bool {
    let a = ad.to_ascii_lowercase();
    SECILI_BASLIKLAR.contains(&a.as_str()) || a.starts_with("x-wfc-")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn ikili(b: &[u8]) -> Value {
    json!({ "ikili": { "sha256": hex(&Sha256::digest(b)), "uzunluk": b.len() } })
}

fn kromlar() -> [(&'static str, &'static str); 4] {
    [("CSS", web::CSS), ("LOGO_ACIK", web::LOGO_ACIK.as_str()), ("LOGO_KOYU", web::LOGO_KOYU.as_str()), ("FAVICON", web::FAVICON.as_str())]
}

/// Ortak CSS, logolar ve sekme simgesi yer tutucuya (iki tarafın bu metinleri aynı mı: `_krom.json` testi).
fn krom(s: &str) -> String {
    let mut s = s.to_string();
    for (ad, deger) in kromlar() {
        if !deger.is_empty() {
            s = s.replace(deger, &format!("{{{ad}}}"));
        }
    }
    s
}

/// {"json": …} | {"metin": …} | {"ikili": {"sha256", "uzunluk"}}; basliklar: yanıtın bütün başlıkları.
fn govde_bicimi(basliklar: &[(String, String)], govde: &[u8]) -> Value {
    let ct = basliklar.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.as_str()).unwrap_or("");
    let tur = ct.split(';').next().unwrap_or("").trim().to_lowercase();
    let metin = std::str::from_utf8(govde).ok();
    if tur == "application/json" {
        if let Some(v) = metin.and_then(|m| serde_json::from_str::<Value>(m).ok()) {
            return json!({ "json": v });
        }
    }
    if govde.is_empty() {
        return json!({ "metin": "" });
    }
    if tur.starts_with("text/") || tur.is_empty() || tur == "application/json" {
        if let Some(m) = metin {
            return json!({ "metin": krom(m) });
        }
    }
    if govde.starts_with(&[0x1f, 0x8b]) {
        // gzip (ör. /api/liste): sıkıştırılmış baytlar uygulamaya göre değişir, içerik değişmez
        let mut acik = Vec::new();
        if flate2::read::MultiGzDecoder::new(govde).read_to_end(&mut acik).is_err() {
            return ikili(govde);
        }
        let mut out = match std::str::from_utf8(&acik) {
            Ok(m) => json!({ "metin": m }),
            Err(_) => ikili(&acik),
        };
        out["gz"] = Value::Bool(true);
        return out;
    }
    ikili(govde)
}

/// Dosya içeriği: .gz açılır ("gz": true; açılamazsa ham bayt ikili); .json → JSON değeri; UTF-8 → metin; değilse ikili.
fn dosya_bicimi(ad: &str, bayt: &[u8]) -> Value {
    let (mut ad, mut gz, mut acik) = (ad, false, Vec::new());
    let mut bayt = bayt;
    if ad.ends_with(".gz") && !bayt.is_empty() {
        if flate2::read::MultiGzDecoder::new(bayt).read_to_end(&mut acik).is_err() {
            return ikili(bayt);
        }
        (ad, gz, bayt) = (&ad[..ad.len() - 3], true, &acik[..]);
    }
    let metin = std::str::from_utf8(bayt).ok();
    let deger = if ad.ends_with(".json") { metin.and_then(|m| serde_json::from_str::<Value>(m).ok()) } else { None };
    let mut out = match (deger, metin) {
        (Some(v), _) => json!({ "json": v }),
        (None, Some(m)) => json!({ "metin": m }),
        (None, None) => ikili(bayt),
    };
    if gz {
        out["gz"] = Value::Bool(true);
    }
    out
}

/// {göreli yol (/ ile): içerik}, yola göre sıralı. Dizinler "yol/". Veritabanları, günlükleri, bağlar ve özel dosyalar alınmaz.
fn dizin_dok(kok: &Path) -> Result<BTreeMap<String, Value>, String> {
    fn yuru(kok: &Path, d: &Path, out: &mut BTreeMap<String, Value>) -> Result<(), String> {
        let hata = |e: std::io::Error| format!("{}: {e}", d.display());
        for e in std::fs::read_dir(d).map_err(hata)? {
            let e = e.map_err(hata)?;
            let yol = e.path();
            let md = std::fs::symlink_metadata(&yol).map_err(hata)?;
            let goreli = yol.strip_prefix(kok).map(|g| g.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                out.insert(format!("{goreli}/"), json!({ "dizin": true }));
                yuru(kok, &yol, out)?;
                continue;
            }
            let ad = e.file_name().to_string_lossy().into_owned();
            if !md.is_file() || GUNLUK_EKLERI.iter().any(|g| ad.ends_with(g)) {
                continue;
            }
            let bayt = std::fs::read(&yol).map_err(hata)?;
            if bayt.starts_with(SQLITE_IMZA) {
                continue;
            }
            out.insert(goreli, dosya_bicimi(&ad, &bayt));
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    yuru(kok, kok, &mut out)?;
    Ok(out)
}

/// Belirteçlerin `<Tn:UZUNLUK>` olması: en soldaki-en uzun eşleşme, n ilk görünüş sırası.
struct Belirtecler {
    liste: Vec<String>,
    no: HashMap<String, usize>,
}

impl Belirtecler {
    fn yeni(uretilen: &[String]) -> Belirtecler {
        let mut liste: Vec<String> = uretilen.iter().filter(|t| t.chars().count() >= BELIRTEC_EN_AZ).cloned().collect();
        liste.sort();
        liste.dedup();
        Belirtecler { liste, no: HashMap::new() }
    }

    fn degistir(&mut self, s: &str, numarali: bool) -> String {
        let mut adaylar: Vec<(usize, usize, usize)> = Vec::new(); // (konum, uzunluk, belirteç)
        for (k, t) in self.liste.iter().enumerate() {
            let mut bas = 0;
            while let Some(i) = s[bas..].find(t.as_str()) {
                adaylar.push((bas + i, t.len(), k));
                bas += i + 1; // belirteç ASCII: bir sonraki bayt karakter sınırı
            }
        }
        if adaylar.is_empty() {
            return s.to_string();
        }
        adaylar.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        let (mut out, mut konum) = (String::with_capacity(s.len()), 0);
        for (i, uzunluk, k) in adaylar {
            if i < konum {
                continue;
            }
            out.push_str(&s[konum..i]);
            if numarali {
                let yeni = self.no.len() + 1;
                let n = *self.no.entry(self.liste[k].clone()).or_insert(yeni);
                out.push_str(&format!("<T{n}:{uzunluk}>"));
            } else {
                out.push_str(&format!("<T?:{uzunluk}>"));
            }
            konum = i + uzunluk;
        }
        out.push_str(&s[konum..]);
        out
    }

    /// JSON: metinler ve anahtarlar; numaralama nesne anahtarlarının sıralı düzeniyle, nesnenin kendi sırası korunur.
    fn json(&mut self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.degistir(s, true)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.json(x)).collect()),
            Value::Object(o) => {
                let mut anahtarlar: Vec<&String> = o.keys().collect();
                anahtarlar.sort();
                let mut yeni: HashMap<&String, (String, Value)> = HashMap::new();
                for k in anahtarlar {
                    let nk = self.degistir(k, true);
                    let nv = self.json(&o[k.as_str()]);
                    yeni.insert(k, (nk, nv));
                }
                Value::Object(o.keys().map(|k| yeni.remove(k).expect("anahtar")).collect())
            }
            x => x.clone(),
        }
    }

    /// Gövde ya da dosya içeriği: {"metin"} ve {"json"} normalleşir, diğerleri olduğu gibi.
    fn icerik(&mut self, d: &mut Value) {
        if let Some(Value::String(m)) = d.get_mut("metin") {
            *m = self.degistir(m, true);
        } else if let Some(j) = d.get("json") {
            let n = self.json(j);
            d["json"] = n;
        }
    }
}

/// kaydet.py `normallestir`: istekler (numarasız), sonra cevaplar (başlıklar, gövde), veritabanları (tablo adı, rowid,
/// sütun adı sırasıyla), dosyalar (dizin sırası, yol sırası; önce yol, sonra içerik).
fn normallestir(cevaplar: &mut [Value], vts: &mut [Value], dizinler: &mut [Value], uretilen: &[String]) {
    let mut b = Belirtecler::yeni(uretilen);
    for c in cevaplar.iter_mut() {
        let s = c["istek"].as_str().unwrap_or("").to_string();
        c["istek"] = Value::String(b.degistir(&s, false));
    }
    for c in cevaplar.iter_mut() {
        if let Some(Value::Array(bs)) = c.get_mut("basliklar") {
            for satir in bs.iter_mut() {
                if let Some(Value::String(v)) = satir.get_mut(1) {
                    *v = b.degistir(v, true);
                }
            }
        }
        b.icerik(&mut c["govde"]);
    }
    for vt in vts.iter_mut() {
        let Some(Value::Object(tablolar)) = vt.get_mut("tablolar") else { continue };
        let mut adlar: Vec<String> = tablolar.keys().cloned().collect();
        adlar.sort();
        for ad in adlar {
            let Some(Value::Array(satirlar)) = tablolar.get_mut(&ad) else { continue };
            for satir in satirlar.iter_mut() {
                let Value::Object(s) = satir else { continue };
                let mut sutunlar: Vec<String> = s.keys().cloned().collect();
                sutunlar.sort();
                for k in sutunlar {
                    if let Some(Value::String(x)) = s.get_mut(&k) {
                        *x = b.degistir(x, true);
                    }
                }
            }
        }
    }
    for dz in dizinler.iter_mut() {
        let Value::Object(eski) = std::mem::take(dz) else { continue };
        let mut yollar: Vec<(String, Value)> = eski.into_iter().collect();
        yollar.sort_by(|a, b| a.0.cmp(&b.0));
        let mut yeni = Map::new();
        for (yol, mut icerik) in yollar {
            let ny = b.degistir(&yol, true);
            b.icerik(&mut icerik);
            yeni.insert(ny, icerik);
        }
        *dz = Value::Object(yeni);
    }
}

/// {tablo: [satır]} (tablo adı, rowid, sütun adı sırasıyla), salt okunur ayrı bağlantıyla.
fn tablolar(yol: &Path) -> Result<Value, String> {
    if yol == Path::new(":memory:") {
        return Err("bellek içi veritabanı (:memory:) karşılaştırılamaz; Python testi gibi dosya kullanın".into());
    }
    let hata = |e: rusqlite::Error| format!("{}: {e}", yol.display());
    let c = Connection::open_with_flags(yol, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(hata)?;
    let mut adlar: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(hata)?;
    adlar.sort();
    let mut out = Map::new();
    for ad in adlar {
        let mut st = c.prepare(&format!("SELECT * FROM \"{ad}\" ORDER BY rowid")).map_err(hata)?;
        let sutunlar: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
        let mut sira: Vec<usize> = (0..sutunlar.len()).collect();
        sira.sort_by(|a, b| sutunlar[*a].cmp(&sutunlar[*b]));
        let mut satirlar = Vec::new();
        let mut q = st.query([]).map_err(hata)?;
        while let Some(r) = q.next().map_err(hata)? {
            let mut m = Map::new();
            for &i in &sira {
                let v = match r.get_ref(i).map_err(hata)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(f) => Value::from(f),
                    ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
                    ValueRef::Blob(b) => json!({ "blob": hex(b) }),
                };
                m.insert(sutunlar[i].clone(), v);
            }
            satirlar.push(Value::Object(m));
        }
        out.insert(ad, Value::Array(satirlar));
    }
    Ok(Value::Object(out))
}

fn kayit_oku(dosya: &str, test: &str) -> Result<Value, String> {
    let k = kayit_oku_dizinden(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/altin"), dosya, test)?;
    kayit_dogrula(&k, dosya, test)?;
    Ok(k)
}

/// <altin>/<dosya>/<test>.json ya da .json.gz (kayıtlar 15 MB'ı aşınca kaydet.py hepsini gzip'ler).
fn kayit_oku_dizinden(altin: &Path, dosya: &str, test: &str) -> Result<Value, String> {
    let kok = altin.join(dosya);
    let (duz, gz) = (kok.join(format!("{test}.json")), kok.join(format!("{test}.json.gz")));
    let bayt = if duz.exists() {
        std::fs::read(&duz).map_err(|e| format!("{}: {e}", duz.display()))?
    } else if gz.exists() {
        let ham = std::fs::read(&gz).map_err(|e| format!("{}: {e}", gz.display()))?;
        let mut acik = Vec::new();
        flate2::read::GzDecoder::new(&ham[..]).read_to_end(&mut acik).map_err(|e| format!("{}: {e}", gz.display()))?;
        acik
    } else {
        return Err(format!(
            "kayıt yok: tests/altin/{dosya}/{test}.json (Python testinin adı doğru mu? kayıtlar: py -3 -I scripts/sunucu/parite/kaydet.py)"
        ));
    };
    serde_json::from_slice(&bayt).map_err(|e| format!("tests/altin/{dosya}/{test}: bozuk JSON: {e}"))
}

/// Biçim ve zorunlu alanlar: bozuk ya da eski biçimli kayıt boş sayılmaz.
fn kayit_dogrula(k: &Value, dosya: &str, test: &str) -> Result<(), String> {
    let yer = format!("tests/altin/{dosya}/{test}");
    if k["bicim"].as_u64() != Some(BICIM) {
        return Err(format!("{yer}: kayıt biçimi {} (beklenen {BICIM}); kaydet.py ile yeniden üretin", k["bicim"]));
    }
    if k["dosya"].as_str() != Some(dosya) || k["test"].as_str() != Some(test) {
        return Err(format!("{yer}: kayıt başka teste ait ({}/{})", k["dosya"], k["test"]));
    }
    for alan in ["cevaplar", "veritabanlari", "dosyalar", "platform_farki"] {
        if !k[alan].is_array() {
            return Err(format!("{yer}: `{alan}` alanı yok ya da dizi değil"));
        }
    }
    if !k["platform_farki"].as_array().is_some_and(|p| p.iter().all(Value::is_string)) {
        return Err(format!("{yer}: `platform_farki` metin dizisi değil"));
    }
    for alan in ["cevaplar", "veritabanlari"] {
        if let Some(y) = kararsiz_yerleri(&k[alan]).first() {
            return Err(format!("{yer}: {KARARSIZ} yalnızca dosya içeriklerinde olabilir ({alan}/{})", y.join("/")));
        }
    }
    for (i, dz) in k["dosyalar"].as_array().into_iter().flatten().enumerate() {
        let Some(dz) = dz.as_object() else {
            return Err(format!("{yer}: dosyalar[{i}] nesne değil"));
        };
        for (dosya, icerik) in dz {
            if let Some(y) = kararsiz_yerleri(icerik).iter().find(|y| !kararsiz_izinli(dosya, y)) {
                return Err(format!("{yer}: dosyalar[{i}]/{dosya}/{}: {KARARSIZ} burada olamaz", y.join("/")));
            }
        }
    }
    Ok(())
}

/// Değeri tam olarak KARARSIZ olan yaprakların yolları (nesne anahtarları, dizi sırası).
fn kararsiz_yerleri(v: &Value) -> Vec<Vec<String>> {
    fn gez(v: &Value, yol: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        match v {
            Value::String(s) if s == KARARSIZ => out.push(yol.clone()),
            Value::Object(o) => {
                for (k, x) in o {
                    yol.push(k.clone());
                    gez(x, yol, out);
                    yol.pop();
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    yol.push(i.to_string());
                    gez(x, yol, out);
                    yol.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    gez(v, &mut Vec::new(), &mut out);
    out
}

/// kaydet.py `kararsiz_izinli`: dosya geçici dizindeki göreli yol, alan içerikteki yol.
fn kararsiz_izinli(dosya: &str, alan: &[String]) -> bool {
    let ad = dosya.rsplit('/').next().unwrap_or(dosya);
    let alan: Vec<&str> = alan.iter().map(String::as_str).collect();
    (alan == ["metin"] && KARARSIZ_METINLER.contains(&ad))
        || (alan.len() == 2 && alan[0] == "json" && KARARSIZ_JSON_ALANLARI.contains(&alan[1]))
        || (dosya.ends_with(".gz") && (alan == ["ikili", "sha256"] || alan == ["ikili", "uzunluk"]))
}

/// Rust testinin kaydı (kaydet.py biçiminde, normalleştirilmiş): (cevaplar, veritabanları, dosyalar).
fn gelen_kayit(d: &Durum) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>), String> {
    let mut cevaplar = Vec::new();
    for (i, (istek, y)) in d.cevaplar.iter().enumerate() {
        if let Some((k, _)) = y.basliklar.iter().find(|(k, _)| !secili(k)) {
            return Err(format!(
                "cevap #{} ({istek}): seçili olmayan başlık `{k}` (Python kaydı bunu reddeder; gerekiyorsa iki tarafta SECILI_BASLIKLAR'a ekleyin)",
                i + 1
            ));
        }
        let basliklar: Vec<Value> = y.basliklar.iter().map(|(k, v)| json!([k, v])).collect();
        cevaplar.push(json!({ "istek": istek, "durum": y.durum, "basliklar": basliklar, "govde": govde_bicimi(&y.basliklar, &y.govde) }));
    }
    let mut vts = Vec::new();
    for yol in &d.veritabanlari {
        let ad = yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|| yol.display().to_string());
        vts.push(json!({ "ad": ad, "tablolar": tablolar(yol)? }));
    }
    let mut dizinler = Vec::new();
    for yol in &d.dizinler {
        dizinler.push(Value::Object(dizin_dok(yol)?.into_iter().collect()));
    }
    normallestir(&mut cevaplar, &mut vts, &mut dizinler, &rastgele::uretilenler());
    Ok((cevaplar, vts, dizinler))
}

fn karsilastir(dosya: &str, test: &str, d: &Durum) -> Result<(), String> {
    let beklenen = kayit_oku(dosya, test)?;
    let (cevaplar, vts, dizinler) = gelen_kayit(d)?;
    let bos = Vec::new();
    let b_cevap = beklenen["cevaplar"].as_array().unwrap_or(&bos);
    for i in 0..b_cevap.len().max(cevaplar.len()) {
        match (b_cevap.get(i), cevaplar.get(i)) {
            (Some(b), Some(g)) => cevap_farki(i, b, g)?,
            (Some(b), None) => {
                return Err(format!("Rust {} cevap verdi, Python {}: ilk eksik cevap #{} {}", cevaplar.len(), b_cevap.len(), i + 1, b["istek"]))
            }
            (None, Some(g)) => {
                return Err(format!("Rust {} cevap verdi, Python {}: ilk fazla cevap #{} {}", cevaplar.len(), b_cevap.len(), i + 1, g["istek"]))
            }
            (None, None) => {}
        }
    }
    let b_vt = beklenen["veritabanlari"].as_array().unwrap_or(&bos);
    if b_vt.len() != vts.len() {
        let adlar = |l: &[Value]| l.iter().map(|v| v["ad"].as_str().unwrap_or("?").to_string()).collect::<Vec<_>>().join(", ");
        return Err(format!("veritabanı sayısı: Python {} [{}], Rust {} [{}]", b_vt.len(), adlar(b_vt), vts.len(), adlar(&vts)));
    }
    for (i, (b, g)) in b_vt.iter().zip(&vts).enumerate() {
        let yer = format!("veritabanı #{} ({})", i + 1, b["ad"].as_str().unwrap_or("?"));
        if b["ad"] != g["ad"] {
            return Err(format!("{yer}: Rust'ta adı {}", g["ad"]));
        }
        tablo_farki(&yer, &b["tablolar"], &g["tablolar"])?;
    }
    // Python testi Windows'ta bir Linux adımını atladıysa (sembolik bağ, FIFO; kaydet.py PLATFORM_TESTLERI) Rust testi
    // Linux'ta o adımı da yapar: Python'un dizin ve dosyaları Rust'ta aynı içerikle bulunmalı, fazlası olabilir
    let alt_kume = beklenen["platform_farki"].as_array().is_some_and(|p| !p.is_empty());
    dosya_farki(beklenen["dosyalar"].as_array().unwrap_or(&bos), &dizinler, alt_kume)
}

/// Geçici dizinler: aynı dizinler, aynı göreli yollar, aynı içerik. alt_kume: Python'unkiler Rust'ta bulunmalı (platform farkı).
fn dosya_farki(b: &[Value], g: &[Value], alt_kume: bool) -> Result<(), String> {
    if b.len() != g.len() && !(alt_kume && b.len() < g.len()) {
        return Err(format!("geçici dizin sayısı: Python {}, Rust {} (Rust testi dizinlerini test_yardim::gecici_dizin ile açmalı)", b.len(), g.len()));
    }
    let bos = Map::new();
    for (i, (x, y)) in b.iter().zip(g).enumerate() {
        let (x, y) = (x.as_object().unwrap_or(&bos), y.as_object().unwrap_or(&bos));
        let yer = format!("geçici dizin #{}", i + 1);
        let eksik: Vec<&String> = x.keys().filter(|k| !y.contains_key(k.as_str())).collect();
        let fazla: Vec<&String> = if alt_kume { Vec::new() } else { y.keys().filter(|k| !x.contains_key(k.as_str())).collect() };
        if !eksik.is_empty() || !fazla.is_empty() {
            return Err(format!("{yer}: dosyalar farklı; Rust'ta eksik {eksik:?}, Rust'ta fazla {fazla:?}"));
        }
        let mut yollar: Vec<&String> = x.keys().collect();
        yollar.sort();
        for yol in yollar {
            let (p, q) = (&x[yol.as_str()], &y[yol.as_str()]);
            if let (Some(Value::String(m)), Some(Value::String(n))) = (p.get("metin"), q.get("metin")) {
                if m != KARARSIZ && m != n {
                    return Err(format!("{yer}, dosya {yol}: içerik farklı, {}", metin_farki(m, n)));
                }
            }
            if let Some(m) = json_farki("$", p, q) {
                return Err(format!("{yer}, dosya {yol}: {m}"));
            }
        }
    }
    Ok(())
}

fn cevap_farki(i: usize, b: &Value, g: &Value) -> Result<(), String> {
    let yer = format!("cevap #{} ({})", i + 1, b["istek"].as_str().unwrap_or("?"));
    if b["istek"] != g["istek"] {
        return Err(format!("{yer}: istek farklı; Rust: {}", g["istek"]));
    }
    if json_farki("", &b["durum"], &g["durum"]).is_some() {
        return Err(format!("{yer}: durum Python {}, Rust {}", b["durum"], g["durum"]));
    }
    let bos = Vec::new();
    let (bb, gb) = (b["basliklar"].as_array().unwrap_or(&bos), g["basliklar"].as_array().unwrap_or(&bos));
    let baslik_esit = bb.len() == gb.len()
        && bb.iter().zip(gb).all(|(x, y)| {
            x[0].as_str().unwrap_or("").eq_ignore_ascii_case(y[0].as_str().unwrap_or("")) && json_farki("", &x[1], &y[1]).is_none()
        });
    if !baslik_esit {
        let goster = |l: &[Value]| l.iter().map(|x| format!("{}: {}", x[0].as_str().unwrap_or(""), x[1].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n    ");
        return Err(format!("{yer}: başlıklar farklı\n  Python:\n    {}\n  Rust:\n    {}", goster(bb), goster(gb)));
    }
    // metin ve JSON farkı ayrıntılı iletiyle; sonra gövde nesnesinin tamamı (tür, "gz" işareti, ikili özeti)
    let (bg, gg) = (&b["govde"], &g["govde"]);
    if let (Some(Value::String(x)), Some(Value::String(y))) = (bg.get("metin"), gg.get("metin")) {
        if x != y {
            return Err(format!("{yer}: gövde farklı, {}", metin_farki(x, y)));
        }
    }
    if let (Some(x), Some(y)) = (bg.get("json"), gg.get("json")) {
        if let Some(m) = json_farki("$", x, y) {
            return Err(format!("{yer}: JSON gövde farklı, {m}"));
        }
    }
    if let Some(m) = json_farki("gövde", bg, gg) {
        return Err(format!("{yer}: {m}"));
    }
    Ok(())
}

fn kisalt(s: &str) -> String {
    if s.chars().count() > 300 {
        s.chars().take(300).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// İlk farklı karakterin satırı, sütunu ve çevresi.
fn metin_farki(b: &str, g: &str) -> String {
    let (bc, gc): (Vec<char>, Vec<char>) = (b.chars().collect(), g.chars().collect());
    let i = bc.iter().zip(&gc).position(|(x, y)| x != y).unwrap_or(bc.len().min(gc.len()));
    let satir = bc[..i].iter().filter(|c| **c == '\n').count() + 1;
    let sutun = i - bc[..i].iter().rposition(|c| *c == '\n').map(|p| p + 1).unwrap_or(0) + 1;
    let parca = |c: &[char]| -> String {
        let bas = i.saturating_sub(80);
        let son = (i + 80).min(c.len());
        let s: String = c[bas.min(c.len())..son].iter().collect();
        format!("{}{}{}", if bas > 0 { "…" } else { "" }, s, if son < c.len() { "…" } else { "" })
    };
    format!(
        "satır {satir}, sütun {sutun} (Python {} karakter, Rust {})\n  Python: {}\n  Rust:   {}",
        bc.len(),
        gc.len(),
        parca(&bc),
        parca(&gc)
    )
}

/// İlk farkın yolu ($.a.b[3]) ve iki değer; aynıysa None. Nesnelerde anahtar sırası önemsiz; beklenen KARARSIZ ise yalnızca
/// bir yaprak (metin, sayı, mantıksal, null) kabul.
fn json_farki(yol: &str, b: &Value, g: &Value) -> Option<String> {
    if b.as_str() == Some(KARARSIZ) && !g.is_object() && !g.is_array() {
        return None;
    }
    match (b, g) {
        (Value::Object(x), Value::Object(y)) => {
            let mut anahtarlar: Vec<&String> = x.keys().chain(y.keys()).collect();
            anahtarlar.sort();
            anahtarlar.dedup();
            for k in anahtarlar {
                match (x.get(k.as_str()), y.get(k.as_str())) {
                    (Some(p), Some(q)) => {
                        if let Some(m) = json_farki(&format!("{yol}.{k}"), p, q) {
                            return Some(m);
                        }
                    }
                    (p, q) => {
                        let goster = |v: Option<&Value>| v.map(|v| kisalt(&v.to_string())).unwrap_or_else(|| "(yok)".into());
                        return Some(format!("{yol}.{k}\n  Python: {}\n  Rust:   {}", goster(p), goster(q)));
                    }
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            x.iter().zip(y).enumerate().find_map(|(i, (p, q))| json_farki(&format!("{yol}[{i}]"), p, q))
        }
        _ if b == g => None,
        _ => Some(format!("{yol}\n  Python: {}\n  Rust:   {}", kisalt(&b.to_string()), kisalt(&g.to_string()))),
    }
}

/// Anahtarları sıralı, tek satırlık JSON (satırların sırasız karşılaştırılması için).
fn kanonik(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut k: Vec<(&String, &Value)> = o.iter().collect();
            k.sort_by(|a, b| a.0.cmp(b.0));
            format!("{{{}}}", k.iter().map(|(a, b)| format!("{}:{}", Value::String(a.to_string()), kanonik(b))).collect::<Vec<_>>().join(","))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(kanonik).collect::<Vec<_>>().join(",")),
        x => x.to_string(),
    }
}

/// Tablolar değer olarak: aynı tablolar; satırlar rowid sırasıyla aynı ya da (yalnızca sıra farklıysa) aynı küme.
fn tablo_farki(yer: &str, b: &Value, g: &Value) -> Result<(), String> {
    let bos = Map::new();
    let (bt, gt) = (b.as_object().unwrap_or(&bos), g.as_object().unwrap_or(&bos));
    let adlar = |m: &Map<String, Value>| m.keys().cloned().collect::<Vec<_>>().join(", ");
    let mut b_ad: Vec<&String> = bt.keys().collect();
    let mut g_ad: Vec<&String> = gt.keys().collect();
    b_ad.sort();
    g_ad.sort();
    if b_ad != g_ad {
        return Err(format!("{yer}: tablolar farklı\n  Python: {}\n  Rust:   {}", adlar(bt), adlar(gt)));
    }
    let bos_l = Vec::new();
    for ad in b_ad {
        let (bs, gs) = (bt[ad.as_str()].as_array().unwrap_or(&bos_l), gt[ad.as_str()].as_array().unwrap_or(&bos_l));
        if bs.len() == gs.len() && bs.iter().zip(gs).all(|(x, y)| json_farki("", x, y).is_none()) {
            continue;
        }
        // yalnızca sıra farkı (ör. ayar satırlarının yazılış sırası) kabul
        if bs.len() == gs.len() {
            let mut x: Vec<String> = bs.iter().map(kanonik).collect();
            let mut y: Vec<String> = gs.iter().map(kanonik).collect();
            x.sort();
            y.sort();
            if x == y {
                continue;
            }
        }
        for (i, (x, y)) in bs.iter().zip(gs).enumerate() {
            if let Some(m) = json_farki("", x, y) {
                return Err(format!("{yer}, tablo {ad}, satır {} (rowid sırası; Python {} satır, Rust {}): sütun {}", i + 1, bs.len(), gs.len(), m.trim_start_matches('.')));
            }
        }
        let fazla = if bs.len() > gs.len() { ("Python", &bs[gs.len()]) } else { ("Rust", &gs[bs.len()]) };
        return Err(format!(
            "{yer}, tablo {ad}: satır sayısı Python {}, Rust {}; ilk fazla satır ({}): {}",
            bs.len(),
            gs.len(),
            fazla.0,
            kisalt(&fazla.1.to_string())
        ));
    }
    Ok(())
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn python_belirlenimli_degerleriyle_ayni() {
        // kaydet.py'nin işlevleriyle (sayaçlar sıfırken) üretilen değerler
        let _k = rastgele::belirlenimli();
        assert_eq!(rastgele::token_urlsafe(32), "ow9GSpUzurqeOa4WJgoViVplwDVeOQN8w2aULmJvri4");
        assert_eq!(rastgele::token_urlsafe(24), "NxGdjwocdOGcLgEXaaEL9jUEq1O3R6UW");
        assert_eq!(rastgele::token_hex(16), "b0b98249afed054d2391c43341da71fb");
        assert_eq!(rastgele::token_hex(8), "42839926a31de4b1");
        assert_eq!(rastgele::randbelow(9_000_000), 4_721_885);
        assert_eq!(rastgele::randbelow(10), 9);
        assert_eq!(crate::guvenlik::parola_uret(12), "6bnuKUG4dyZ7");
        assert_eq!(hex(&rastgele::token_bytes(3)), "445b31");
        // yalnızca urlsafe ve hex belirteç sayılır
        assert_eq!(
            rastgele::uretilenler(),
            ["ow9GSpUzurqeOa4WJgoViVplwDVeOQN8w2aULmJvri4", "NxGdjwocdOGcLgEXaaEL9jUEq1O3R6UW", "b0b98249afed054d2391c43341da71fb", "42839926a31de4b1"]
        );
    }

    #[test]
    fn krom_python_ile_ayni() {
        // kaydet.py `_krom.json`: Python web.CSS/LOGO_ACIK/LOGO_KOYU/FAVICON'un sha256'sı; yer tutucular ancak aynıysa güvenli
        let yol = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/altin/_krom.json");
        let k: Value = serde_json::from_str(&std::fs::read_to_string(&yol).expect("_krom.json")).expect("json");
        for (ad, deger) in kromlar() {
            assert!(!deger.is_empty(), "{ad} boş");
            assert_eq!(k[ad].as_str(), Some(hex(&Sha256::digest(deger.as_bytes())).as_str()), "{ad} Python'unkinden farklı");
        }
    }

    #[test]
    fn belirtecler_ilk_gorunus_sirasiyla_en_uzun_eslesme() {
        let t1 = "AAAAAAAAAAAAAAAA".to_string();
        let t2 = "AAAAAAAAAAAAAAAABBBB".to_string(); // t1 ile başlıyor: en uzunu seçilir
        let t3 = "cccccccc".to_string();
        let kisa = "abc".to_string(); // 8'den kısa: belirteç değil
        let mut b = Belirtecler::yeni(&[t1.clone(), t2.clone(), t3.clone(), kisa, t1.clone()]);
        assert_eq!(b.degistir("x cccccccc y AAAAAAAAAAAAAAAABBBB z AAAAAAAAAAAAAAAA abc", true), "x <T1:8> y <T2:20> z <T3:16> abc");
        assert_eq!(b.degistir("AAAAAAAAAAAAAAAA/cccccccc", true), "<T3:16>/<T1:8>");
        assert_eq!(b.degistir("t=cccccccc", false), "t=<T?:8>");
        // JSON: anahtarlar sıralı dolaşılır, nesnenin sırası korunur (kaydet.py ile aynı)
        let mut b = Belirtecler::yeni(&[t1.clone(), t3.clone()]);
        let v = b.json(&json!({"z": "cccccccc", "a": ["AAAAAAAAAAAAAAAA", 1], "cccccccc": true}));
        assert_eq!(v.to_string(), r#"{"z":"<T2:8>","a":["<T1:16>",1],"<T2:8>":true}"#);
    }

    #[test]
    fn govde_bicimleri() {
        let b = |ct: &str| vec![("Content-Type".to_string(), ct.to_string())];
        assert_eq!(govde_bicimi(&b("application/json; charset=utf-8"), br#"{"a": 1}"#), json!({"json": {"a": 1}}));
        assert_eq!(govde_bicimi(&b("application/json"), b"{bozuk"), json!({"metin": "{bozuk"}));
        assert_eq!(govde_bicimi(&[], b"Y / 1.1.1.1"), json!({"metin": "Y / 1.1.1.1"}));
        assert_eq!(govde_bicimi(&b("application/octet-stream"), b""), json!({"metin": ""}));
        // gzip gövde açılır (sıkıştırma uygulamaya göre değişir)
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut e, b"a.com\n").unwrap();
        assert_eq!(govde_bicimi(&b("application/gzip"), &e.finish().unwrap()), json!({"metin": "a.com\n", "gz": true}));
        let ikili = govde_bicimi(&b("application/gzip"), b"abc");
        assert_eq!(ikili["ikili"]["uzunluk"], 3);
        assert_eq!(ikili["ikili"]["sha256"], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(govde_bicimi(&b("text/plain"), &[0xff, 0xfe]).get("ikili").is_some());
        let sayfa = format!("<style>{}</style>{}{}", web::CSS, &*web::LOGO_ACIK, &*web::FAVICON);
        assert_eq!(govde_bicimi(&b("text/html; charset=utf-8"), sayfa.as_bytes()), json!({"metin": "<style>{CSS}</style>{LOGO_ACIK}{FAVICON}"}));
        assert!(secili("X-WFC-Cihaz") && secili("set-cookie") && !secili("Content-Length") && !secili("Server"));
    }

    #[test]
    fn dosya_bicimleri_ve_dizin_dokumu() {
        use std::io::Write;
        let gz = |b: &[u8]| {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(b).unwrap();
            e.finish().unwrap()
        };
        assert_eq!(dosya_bicimi("a.json", br#"{"x": [1]}"#), json!({"json": {"x": [1]}}));
        assert_eq!(dosya_bicimi("a.json", b"bozuk"), json!({"metin": "bozuk"}));
        assert_eq!(dosya_bicimi("a.txt", "ç\n".as_bytes()), json!({"metin": "ç\n"}));
        assert_eq!(dosya_bicimi("a.txt.gz", &gz(b"x\n")), json!({"metin": "x\n", "gz": true}));
        assert_eq!(dosya_bicimi("k.json.gz", &gz(br#"{"a": 1}"#)), json!({"json": {"a": 1}, "gz": true}));
        assert_eq!(dosya_bicimi("b.gz", b"bozuk gz"), ikili(b"bozuk gz"));
        assert_eq!(dosya_bicimi("b.gz", b""), json!({"metin": ""}));
        assert_eq!(dosya_bicimi("b.bin", &[0xff, 0, 1]), ikili(&[0xff, 0, 1]));
        // dizin: göreli yollar sıralı, dizinler "/", veritabanı ve günlüğü, bağ ve FIFO alınmaz
        let d = crate::ortak::tests::gecici_dizin();
        std::fs::create_dir_all(d.join("b/c")).unwrap();
        std::fs::write(d.join("b/c/z.json"), "[1]").unwrap();
        std::fs::write(d.join("a.txt"), "x").unwrap();
        crate::veri::Veri::ac(&d.join("m.db"), std::sync::Arc::new(|| 0.0)).unwrap();
        std::fs::write(d.join("m.db-journal"), "j").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join("a.txt"), d.join("bag")).unwrap();
            let fifo = std::ffi::CString::new(d.join("fifo").to_string_lossy().as_bytes()).unwrap();
            // SAFETY: geçerli, NUL ile biten yol
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        }
        let dok = dizin_dok(&d).unwrap();
        assert_eq!(dok.keys().collect::<Vec<_>>(), ["a.txt", "b/", "b/c/", "b/c/z.json"]);
        assert_eq!(dok["b/c/z.json"], json!({"json": [1]}));
    }

    #[test]
    fn kararsiz_yalnizca_yapraga_karsi() {
        let b = json!({"imza": KARARSIZ, "sayi": 3});
        for gelen in [json!("abc"), json!(7), json!(null), json!(false)] {
            assert!(json_farki("$", &b, &json!({"imza": gelen, "sayi": 3})).is_none());
        }
        for gelen in [json!({"x": 1}), json!([1])] {
            assert!(json_farki("$", &b, &json!({"imza": gelen, "sayi": 3})).unwrap().starts_with("$.imza"));
        }
        assert!(json_farki("$", &b, &json!({"imza": "abc", "sayi": 4})).unwrap().starts_with("$.sayi"));
        assert!(json_farki("$", &b, &json!({"sayi": 3})).is_some()); // anahtar yine olmalı
        let dz = |v: Value| vec![v];
        assert!(dosya_farki(&dz(json!({"g/MANIFEST.sha256": {"metin": KARARSIZ}})), &dz(json!({"g/MANIFEST.sha256": {"metin": "x"}})), false).is_ok());
    }

    #[test]
    fn kararsiz_yalnizca_izinli_yerde() {
        let k = |cevaplar: Value, vt: Value, dosyalar: Value| {
            json!({"bicim": 2, "dosya": "d", "test": "t", "platform_farki": [], "cevaplar": cevaplar, "veritabanlari": vt, "dosyalar": dosyalar})
        };
        let bos = json!([]);
        let izinli = json!([{
            "ham/1/2026-10-05.json": {"json": {"imza": KARARSIZ, "manifest_sha": KARARSIZ, "n": 1}},
            "g/MANIFEST.sha256": {"metin": KARARSIZ}, "zincir.txt": {"metin": KARARSIZ},
            "g/t.csv.gz": {"ikili": {"sha256": KARARSIZ, "uzunluk": KARARSIZ}}}]);
        assert!(kayit_dogrula(&k(bos.clone(), bos.clone(), izinli), "d", "t").is_ok());
        let e = kayit_dogrula(&k(json!([{"istek": "GET /", "govde": {"metin": KARARSIZ}}]), bos.clone(), bos.clone()), "d", "t").unwrap_err();
        assert!(e.contains("yalnızca dosya içeriklerinde olabilir (cevaplar/0/govde/metin)"), "{e}");
        let e = kayit_dogrula(&k(bos.clone(), json!([{"ad": "m.db", "tablolar": {"t": [{"z": KARARSIZ}]}}]), bos.clone()), "d", "t").unwrap_err();
        assert!(e.contains("(veritabanlari/0/tablolar/t/0/z)"), "{e}");
        for (dosya, icerik) in [
            ("notlar.txt", json!({"metin": KARARSIZ})),
            ("x.json", json!({"json": {"a": {"imza": KARARSIZ}}})),
            ("x.json", json!({"json": {"liste": [KARARSIZ]}})),
            ("t.bin", json!({"ikili": {"sha256": KARARSIZ, "uzunluk": 1}})),
            ("x.json", json!(KARARSIZ)),
        ] {
            let e = kayit_dogrula(&k(bos.clone(), bos.clone(), json!([{ dosya: icerik }])), "d", "t").unwrap_err();
            assert!(e.contains("burada olamaz"), "{dosya}: {e}");
        }
    }

    #[test]
    fn govde_nesnesinin_tamami_karsilastirilir() {
        let c = |govde: Value| json!({"istek": "GET /api/liste", "durum": 200, "basliklar": [], "govde": govde});
        let e = cevap_farki(0, &c(json!({"metin": "a.com\n", "gz": true})), &c(json!({"metin": "a.com\n"}))).unwrap_err();
        assert!(e.contains("gövde.gz"), "{e}");
        let e = cevap_farki(0, &c(json!({"metin": "a"})), &c(json!({"json": "a"}))).unwrap_err();
        assert!(e.contains("gövde."), "{e}");
        assert!(cevap_farki(0, &c(json!({"metin": "a", "gz": true})), &c(json!({"metin": "a", "gz": true}))).is_ok());
    }

    #[test]
    fn platform_farkinda_python_dosyalari_alt_kume() {
        let dz = |v: Value| v.as_array().unwrap().clone();
        let python = dz(json!([{"gun/": {"dizin": true}, "gun/a.txt": {"metin": "x"}}]));
        // Rust Linux'ta fazlasını da yapar: fazla dosya ve fazla dizin olabilir
        let rust = dz(json!([{"gun/": {"dizin": true}, "gun/a.txt": {"metin": "x"}, "gun/b": {"metin": ""}}, {}]));
        assert!(dosya_farki(&python, &rust, true).is_ok());
        assert!(dosya_farki(&python, &rust, false).is_err());
        let e = dosya_farki(&python, &dz(json!([{"gun/": {"dizin": true}}])), true).unwrap_err();
        assert!(e.contains("Rust'ta eksik [\"gun/a.txt\"]"), "{e}");
        let e = dosya_farki(&python, &dz(json!([{"gun/": {"dizin": true}, "gun/a.txt": {"metin": "y"}}])), true).unwrap_err();
        assert!(e.contains("dosya gun/a.txt: içerik farklı"), "{e}");
        let e = dosya_farki(&python, &dz(json!([{"gun/a.txt": {"metin": "x"}}])), true).unwrap_err();
        assert!(e.contains("Rust'ta eksik [\"gun/\"]"), "{e}"); // dizin girdileri de
        assert!(dosya_farki(&python, &[], true).unwrap_err().starts_with("geçici dizin sayısı: Python 1, Rust 0"));
    }

    #[test]
    fn farklar_okunur_yazilir() {
        let m = metin_farki("ab\ncdef", "ab\ncxef");
        assert!(m.starts_with("satır 2, sütun 2"), "{m}");
        assert!(json_farki("$", &json!({"a": [1, {"b": 2}]}), &json!({"a": [1, {"b": 3}]})).unwrap().starts_with("$.a[1].b"));
        assert!(json_farki("$", &json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1})).is_none());
        assert!(json_farki("$", &json!(1), &json!(1.0)).is_some()); // tamsayı ile kayan nokta ayrı
        let t = |v: Value| json!({"t": v});
        assert!(tablo_farki("vt", &t(json!([{"k": "a"}, {"k": "b"}])), &t(json!([{"k": "b"}, {"k": "a"}]))).is_ok());
        let e = tablo_farki("vt", &t(json!([{"k": "a"}])), &t(json!([{"k": "x"}]))).unwrap_err();
        assert!(e.contains("tablo t, satır 1") && e.contains("sütun k"), "{e}");
        assert!(tablo_farki("vt", &t(json!([{"k": "a"}])), &t(json!([]))).unwrap_err().contains("satır sayısı Python 1, Rust 0"));
        let dz = |v: Value| vec![v];
        let e = dosya_farki(&dz(json!({"a.txt": {"metin": "x\ny"}})), &dz(json!({"a.txt": {"metin": "x\nz"}})), false).unwrap_err();
        assert!(e.starts_with("geçici dizin #1, dosya a.txt: içerik farklı, satır 2, sütun 1"), "{e}");
        let e = dosya_farki(&dz(json!({"a/": {"dizin": true}})), &dz(json!({"b": {"metin": ""}})), false).unwrap_err();
        assert!(e.contains("Rust'ta eksik [\"a/\"], Rust'ta fazla [\"b\"]"), "{e}");
        assert!(dosya_farki(&[], &dz(json!({})), false).unwrap_err().starts_with("geçici dizin sayısı: Python 0, Rust 1"));
    }

    #[test]
    fn kayit_bicimi_dogrulanir() {
        let iyi = json!({"bicim": 2, "dosya": "d", "test": "t", "cevaplar": [], "veritabanlari": [], "dosyalar": [], "platform_farki": []});
        assert!(kayit_dogrula(&iyi, "d", "t").is_ok());
        let mut eski = iyi.clone();
        eski["bicim"] = json!(1);
        assert!(kayit_dogrula(&eski, "d", "t").unwrap_err().contains("kayıt biçimi 1 (beklenen 2)"));
        let mut eksik = iyi.clone();
        eksik.as_object_mut().unwrap().remove("dosyalar");
        assert!(kayit_dogrula(&eksik, "d", "t").unwrap_err().contains("`dosyalar` alanı yok"));
        assert!(kayit_dogrula(&iyi, "d", "baska").unwrap_err().contains("kayıt başka teste ait"));
        assert!(kayit_dogrula(&json!([]), "d", "t").is_err());
    }

    #[test]
    fn butun_kayitlar_gecerli() {
        // her kayıt biçim 2, zorunlu alanlar, <KARARSIZ> yalnızca izinli yerlerde (Rust testi henüz yazılmamış olanlar da)
        let kok = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/altin");
        let mut sayi = 0;
        for d in std::fs::read_dir(&kok).unwrap() {
            let d = d.unwrap().path();
            if !d.is_dir() {
                continue;
            }
            let dosya = d.file_name().unwrap().to_string_lossy().into_owned();
            for f in std::fs::read_dir(&d).unwrap() {
                let ad = f.unwrap().file_name().to_string_lossy().into_owned();
                let test = ad.trim_end_matches(".gz").trim_end_matches(".json");
                let k = kayit_oku_dizinden(&kok, &dosya, test).unwrap();
                kayit_dogrula(&k, &dosya, test).unwrap();
                sayi += 1;
            }
        }
        assert!(sayi >= 367, "{sayi}");
    }

    #[test]
    fn kayitsiz_test_kayit_dosyasi_ister() {
        let e = kayit_oku("test_yok", "test_yok").unwrap_err();
        assert!(e.contains("kayıt yok"), "{e}");
    }

    #[test]
    fn gzipli_kayit_okunur() {
        use std::io::Write;
        let d = crate::ortak::tests::gecici_dizin();
        std::fs::create_dir(d.join("test_x")).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(br#"{"bicim": 2, "cevaplar": [], "veritabanlari": []}"#).unwrap();
        std::fs::write(d.join("test_x/test_y.json.gz"), gz.finish().unwrap()).unwrap();
        assert_eq!(kayit_oku_dizinden(&d, "test_x", "test_y").unwrap()["bicim"], 2);
    }

    fn panik_metni(p: Box<dyn std::any::Any + Send>) -> String {
        p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
    }

    #[test]
    fn fark_ilk_farkli_cevabi_yazar() {
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_merkez", "test_host_yonlendirme");
            // Python'da ilk istek "Y / 1.1.1.1" döner; burada uygulama kayıtlı değil: 404
            let m = crate::merkez::Merkez::new();
            crate::test_yardim::istek(&m, "GET", "/", &[("Host", "yonetim.wificorrect.com")], "", "", "1.1.1.1");
        })
        .unwrap_err();
        let m = panik_metni(p);
        assert!(m.contains("(test_merkez/test_host_yonlendirme): cevap #1 (GET /): durum Python 200, Rust 404"), "{m}");
        ETKIN.with(|e| assert!(e.borrow().is_none())); // panikten sonra kayıt kapalı
        assert!(rastgele::uretilenler().is_empty());
    }

    #[test]
    fn secili_olmayan_baslik_farktir() {
        struct Fazla;
        impl crate::merkez::Uygulama for Fazla {
            fn istek(&self, _: &Istek) -> Yanit {
                Yanit::yeni(200, &[("X-Baska", "1")], b"Y / 1.1.1.1".to_vec())
            }
        }
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_merkez", "test_host_yonlendirme");
            crate::test_yardim::istek(&Fazla, "GET", "/", &[], "", "", "1.1.1.1");
        })
        .unwrap_err();
        assert!(panik_metni(p).contains("cevap #1 (GET /): seçili olmayan başlık `X-Baska`"));
    }

    #[test]
    fn fark_tabloyu_ve_dosyayi_yazar() {
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_veri", "test_silinen_veri_sayfalarda_kalmaz");
            let o = crate::test_yardim::ortam();
            o.veri.hareket("admin", "1.2.3.4", "FAZLA", None, "").unwrap();
        })
        .unwrap_err();
        let m = panik_metni(p);
        assert!(m.contains("veritabanı #1 (m.db), tablo hareket: satır sayısı Python 0, Rust 1"), "{m}");
        // tablolar aynı, dosya fazla: Python'un geçici dizininde yalnızca veritabanı var
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_veri", "test_silinen_veri_sayfalarda_kalmaz");
            let d = crate::test_yardim::gecici_dizin();
            crate::veri::Veri::ac(&d.join("m.db"), std::sync::Arc::new(|| 0.0)).unwrap();
            std::fs::write(d.join("fazla.txt"), "x").unwrap();
        })
        .unwrap_err();
        let m = panik_metni(p);
        assert!(m.contains("geçici dizin #1: dosyalar farklı; Rust'ta eksik [], Rust'ta fazla [\"fazla.txt\"]"), "{m}");
    }

    #[test]
    fn eksik_cevap_ve_altinsiz() {
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_merkez", "test_host_yonlendirme");
        })
        .unwrap_err();
        assert!(panik_metni(p).contains("Rust 0 cevap verdi, Python 7: ilk eksik cevap #1 \"GET /\""));
        // altınsız: karşılaştırılmaz, belirlenimli kip yine açık
        let _k = Kayit::altinsiz("deneme: bilinçli fark");
        assert_eq!(rastgele::token_hex(16), "b0b98249afed054d2391c43341da71fb");
    }

    #[test]
    fn platform_farkli_kayitta_fazla_dizin_olabilir() {
        // Python testi Windows'ta FIFO adımını atladı (geçici dizin hiç açmadı); Rust Linux'ta açar
        let k = kayit_oku("test_rapor", "test_tamam_mi_fifo_beklemez").unwrap();
        assert_eq!(k["platform_farki"], json!(["os.mkfifo yok"]));
        let _k = Kayit::bas("test_rapor", "test_tamam_mi_fifo_beklemez");
        std::fs::create_dir(crate::test_yardim::gecici_dizin().join("gun")).unwrap();
    }
}
