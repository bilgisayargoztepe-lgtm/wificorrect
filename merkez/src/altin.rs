//! Altın kayıt (yalnızca testler): Python testlerinin cevapları ve veritabanlarının son hâli
//! (scripts/sunucu/parite/kaydet.py → tests/altin/<dosya>/<test>.json[.gz]) Rust'taki aynı testinkiyle karşılaştırılır.
//! Kayıt biçimi ve normalleştirme kaydet.py'nin başındaki tarifin aynısıdır.
//!
//! Kullanım: testin ilk satırı `let _k = Kayit::bas("test_yonetim", "test_x");` (Python dosyasının ve testinin adı).
//! Kayıt açıkken bu iş parçacığında rastgelelik belirlenimlidir; `test_yardim`'ın istek yardımcıları cevapları,
//! `Veri::ac` veritabanı dosyalarını bildirir. Kayıt düşünce (test panik etmediyse) karşılaştırır, ilk farkı yazıp panik eder.
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::guvenlik::rastgele;
use crate::web::{self, Istek, Yanit};

/// Karşılaştırılan başlıklar (küçük harfle) ve `x-wfc-` ile başlayanlar; diğerleri (ör. sunucunun eklediği) karşılaştırılmaz.
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

#[derive(Default)]
struct Durum {
    /// (istek gösterimi, yanıt)
    cevaplar: Vec<(String, Yanit)>,
    /// Açılan veritabanı dosyaları, ilk açılış sırasıyla (yol başına bir kez).
    veritabanlari: Vec<PathBuf>,
}

thread_local! {
    static ETKIN: RefCell<Option<Durum>> = const { RefCell::new(None) };
}

/// Bir testin altın kaydı. Düşünce karşılaştırır (`altinsiz` değilse).
pub struct Kayit {
    dosya: String,
    test: String,
    /// Yalnızca spec'teki bilinçli farklar: belirlenimli kip açılır, karşılaştırılmaz.
    altinsiz: Option<String>,
    _kip: rastgele::Belirlenimli,
}

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
        Kayit { dosya: dosya.into(), test: test.into(), altinsiz, _kip: rastgele::belirlenimli() }
    }
}

impl Drop for Kayit {
    fn drop(&mut self) {
        let durum = ETKIN.with(|e| e.borrow_mut().take());
        if self.altinsiz.is_some() || std::thread::panicking() {
            return;
        }
        if let Some(d) = durum {
            if let Err(m) = karsilastir(&self.dosya, &self.test, &d) {
                panic!("altın kayıt farkı ({}/{}): {m}", self.dosya, self.test);
            }
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

// --- kayıt biçimi (kaydet.py ile birebir) ---

fn secili(ad: &str) -> bool {
    let a = ad.to_ascii_lowercase();
    SECILI_BASLIKLAR.contains(&a.as_str()) || a.starts_with("x-wfc-")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Ortak CSS, logolar ve sekme simgesi yer tutucuya.
fn krom(s: &str) -> String {
    let mut s = s.to_string();
    for (ad, deger) in [("CSS", web::CSS), ("LOGO_ACIK", web::LOGO_ACIK.as_str()), ("LOGO_KOYU", web::LOGO_KOYU.as_str()), ("FAVICON", web::FAVICON.as_str())] {
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
    json!({ "ikili": { "sha256": hex(&Sha256::digest(govde)), "uzunluk": govde.len() } })
}

/// Belirteçlerin `<Tn>` olması: en soldaki-en uzun eşleşme, n ilk görünüş sırası.
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
                out.push_str(&format!("<T{n}>"));
            } else {
                out.push_str("<T?>");
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
}

/// kaydet.py `normallestir`: istekler (numarasız), sonra cevaplar (başlıklar, gövde), sonra veritabanları (tablo adı,
/// rowid, sütun adı sırasıyla).
fn normallestir(cevaplar: &mut [Value], vts: &mut [Value], uretilen: &[String]) {
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
        let g = &mut c["govde"];
        if let Some(Value::String(m)) = g.get_mut("metin") {
            *m = b.degistir(m, true);
        } else if let Some(j) = g.get("json") {
            let n = b.json(j);
            g["json"] = n;
        }
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
    kayit_oku_dizinden(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/altin"), dosya, test)
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

/// Rust testinin kaydı (kaydet.py biçiminde, normalleştirilmiş).
fn gelen_kayit(d: &Durum) -> Result<(Vec<Value>, Vec<Value>), String> {
    let mut cevaplar: Vec<Value> = d
        .cevaplar
        .iter()
        .map(|(istek, y)| {
            let basliklar: Vec<Value> = y.basliklar.iter().filter(|(k, _)| secili(k)).map(|(k, v)| json!([k, v])).collect();
            json!({ "istek": istek, "durum": y.durum, "basliklar": basliklar, "govde": govde_bicimi(&y.basliklar, &y.govde) })
        })
        .collect();
    let mut vts = Vec::new();
    for yol in &d.veritabanlari {
        let ad = yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_else(|| yol.display().to_string());
        vts.push(json!({ "ad": ad, "tablolar": tablolar(yol)? }));
    }
    normallestir(&mut cevaplar, &mut vts, &rastgele::uretilenler());
    Ok((cevaplar, vts))
}

fn karsilastir(dosya: &str, test: &str, d: &Durum) -> Result<(), String> {
    let beklenen = kayit_oku(dosya, test)?;
    let (cevaplar, vts) = gelen_kayit(d)?;
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
    Ok(())
}

fn cevap_farki(i: usize, b: &Value, g: &Value) -> Result<(), String> {
    let yer = format!("cevap #{} ({})", i + 1, b["istek"].as_str().unwrap_or("?"));
    if b["istek"] != g["istek"] {
        return Err(format!("{yer}: istek farklı; Rust: {}", g["istek"]));
    }
    if b["durum"] != g["durum"] {
        return Err(format!("{yer}: durum Python {}, Rust {}", b["durum"], g["durum"]));
    }
    let bos = Vec::new();
    let (bb, gb) = (b["basliklar"].as_array().unwrap_or(&bos), g["basliklar"].as_array().unwrap_or(&bos));
    let baslik_esit = bb.len() == gb.len()
        && bb.iter().zip(gb).all(|(x, y)| {
            x[0].as_str().unwrap_or("").eq_ignore_ascii_case(y[0].as_str().unwrap_or("")) && x[1] == y[1]
        });
    if !baslik_esit {
        let goster = |l: &[Value]| l.iter().map(|x| format!("{}: {}", x[0].as_str().unwrap_or(""), x[1].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n    ");
        return Err(format!("{yer}: başlıklar farklı\n  Python:\n    {}\n  Rust:\n    {}", goster(bb), goster(gb)));
    }
    let (bg, gg) = (&b["govde"], &g["govde"]);
    match (bg.get("metin"), gg.get("metin")) {
        (Some(Value::String(x)), Some(Value::String(y))) => {
            if x != y {
                return Err(format!("{yer}: gövde farklı, {}", metin_farki(x, y)));
            }
            return Ok(());
        }
        (Some(_), _) | (_, Some(_)) => return Err(format!("{yer}: gövde türü farklı\n  Python: {}\n  Rust:   {}", kisalt(&bg.to_string()), kisalt(&gg.to_string()))),
        _ => {}
    }
    if let (Some(x), Some(y)) = (bg.get("json"), gg.get("json")) {
        return match json_farki("$", x, y) {
            Some(m) => Err(format!("{yer}: JSON gövde farklı, {m}")),
            None => Ok(()),
        };
    }
    if bg != gg {
        return Err(format!("{yer}: gövde farklı\n  Python: {}\n  Rust:   {}", kisalt(&bg.to_string()), kisalt(&gg.to_string())));
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

/// İlk farkın yolu ($.a.b[3]) ve iki değer; aynıysa None. Nesnelerde anahtar sırası önemsiz.
fn json_farki(yol: &str, b: &Value, g: &Value) -> Option<String> {
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
    fn belirtecler_ilk_gorunus_sirasiyla_en_uzun_eslesme() {
        let t1 = "AAAAAAAAAAAAAAAA".to_string();
        let t2 = "AAAAAAAAAAAAAAAABBBB".to_string(); // t1 ile başlıyor: en uzunu seçilir
        let t3 = "cccccccc".to_string();
        let kisa = "abc".to_string(); // 8'den kısa: belirteç değil
        let mut b = Belirtecler::yeni(&[t1.clone(), t2.clone(), t3.clone(), kisa, t1.clone()]);
        assert_eq!(b.degistir("x cccccccc y AAAAAAAAAAAAAAAABBBB z AAAAAAAAAAAAAAAA abc", true), "x <T1> y <T2> z <T3> abc");
        assert_eq!(b.degistir("AAAAAAAAAAAAAAAA/cccccccc", true), "<T3>/<T1>");
        assert_eq!(b.degistir("t=cccccccc", false), "t=<T?>");
        // JSON: anahtarlar sıralı dolaşılır, nesnenin sırası korunur
        let mut b = Belirtecler::yeni(&[t1.clone(), t3.clone()]);
        let v = b.json(&json!({"z": "cccccccc", "a": ["AAAAAAAAAAAAAAAA", 1], "cccccccc": true}));
        assert_eq!(v.to_string(), r#"{"z":"<T2>","a":["<T1>",1],"<T2>":true}"#); // kaydet.py ile aynı
    }

    #[test]
    fn govde_bicimleri() {
        let b = |ct: &str| vec![("Content-Type".to_string(), ct.to_string())];
        assert_eq!(govde_bicimi(&b("application/json; charset=utf-8"), br#"{"a": 1}"#), json!({"json": {"a": 1}}));
        assert_eq!(govde_bicimi(&b("application/json"), b"{bozuk"), json!({"metin": "{bozuk"}));
        assert_eq!(govde_bicimi(&[], b"Y / 1.1.1.1"), json!({"metin": "Y / 1.1.1.1"}));
        assert_eq!(govde_bicimi(&b("application/octet-stream"), b""), json!({"metin": ""}));
        let ikili = govde_bicimi(&b("application/gzip"), b"abc");
        assert_eq!(ikili["ikili"]["uzunluk"], 3);
        assert_eq!(ikili["ikili"]["sha256"], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(govde_bicimi(&b("text/plain"), &[0xff, 0xfe]).get("ikili").is_some());
        let sayfa = format!("<style>{}</style>{}{}", web::CSS, &*web::LOGO_ACIK, &*web::FAVICON);
        assert_eq!(govde_bicimi(&b("text/html; charset=utf-8"), sayfa.as_bytes()), json!({"metin": "<style>{CSS}</style>{LOGO_ACIK}{FAVICON}"}));
        assert!(secili("X-WFC-Cihaz") && secili("set-cookie") && !secili("Content-Length") && !secili("Server"));
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
        gz.write_all(br#"{"bicim": 1, "cevaplar": [], "veritabanlari": []}"#).unwrap();
        std::fs::write(d.join("test_x/test_y.json.gz"), gz.finish().unwrap()).unwrap();
        assert_eq!(kayit_oku_dizinden(&d, "test_x", "test_y").unwrap()["bicim"], 1);
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
    fn fark_tabloyu_yazar() {
        let p = std::panic::catch_unwind(|| {
            let _k = Kayit::bas("test_veri", "test_silinen_veri_sayfalarda_kalmaz");
            let o = crate::test_yardim::ortam();
            o.veri.hareket("admin", "1.2.3.4", "FAZLA", None, "").unwrap();
        })
        .unwrap_err();
        let m = panik_metni(p);
        assert!(m.contains("veritabanı #1 (m.db), tablo hareket: satır sayısı Python 0, Rust 1"), "{m}");
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
}
