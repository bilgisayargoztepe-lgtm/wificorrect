//! Müşteri kayıtlarının görünümü (Python kayitlar.py; eski kafe paneli ekranları): son 30 gün sayıları, gün → kişi, arama.
//! Bir müşterinin birden çok arşivi olabilir (kendi numarası + bağlı eski arşivler); aynı gün iki arşivde varsa ilki geçerli.
//! Yalnızca root'un ürettiği özetleri okur (`istatistik/<arşiv>.json`, `detay/<arşiv>/`).
//! Kişiler ekranda maskeli, adreslerde opak kimlikle (sır: `kisi_sir`, wcpanel 600, ilk kullanımda üretilir).
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use serde_json::{Map, Value};

use crate::detay::{self, e, Dizin};
use crate::ortak::{self, kilitle, py_strip};

pub const ISTATISTIK: &str = "/var/lib/wificorrect/istatistik";
pub const DETAY: &str = "/var/lib/wificorrect/detay";
pub const SIR: &str = "/var/lib/wificorrect/merkez/kisi_sir";

/// JSON dosyası; yoksa, klasör değilse, JSON değilse ya da okunamıyorsa None.
fn json_oku(yol: &Path) -> Option<Value> {
    let ham = std::fs::read(yol).ok()?;
    serde_json::from_slice(&ham).ok()
}

/// `simdi`'den (TR günü) dünden başlayıp geriye 30 gün: [dün, dünden önceki gün, …].
pub fn son_30_gun(simdi: f64) -> Vec<NaiveDate> {
    let bugun = ortak::TZ.timestamp_opt(simdi.floor() as i64, 0).single().map(|t| t.date_naive()).unwrap_or_default();
    let dun = bugun - Duration::days(1);
    (0..30).map(|i| dun - Duration::days(i)).collect()
}

pub struct Kayitlar {
    pub ist_dizini: PathBuf,
    pub detay_dizini: PathBuf,
    pub sir_yolu: PathBuf,
    sir: Mutex<Option<Vec<u8>>>,
}

impl Default for Kayitlar {
    fn default() -> Self {
        Kayitlar::new(ISTATISTIK.into(), DETAY.into(), SIR.into())
    }
}

impl Kayitlar {
    pub fn new(ist_dizini: PathBuf, detay_dizini: PathBuf, sir_yolu: PathBuf) -> Kayitlar {
        Kayitlar { ist_dizini, detay_dizini, sir_yolu, sir: Mutex::new(None) }
    }

    /// Kişi kimliği sırrı (ilk kullanımda dosyadan okunur ya da üretilir). Okunamazsa/yazılamazsa `Err` (500).
    pub fn sir(&self) -> Result<Vec<u8>, String> {
        let mut s = kilitle(&self.sir);
        if s.is_none() {
            *s = Some(detay::sir_oku_ya_uret(&self.sir_yolu).map_err(|h| format!("kisi_sir: {h}"))?);
        }
        Ok(s.clone().expect("yukarıda dolduruldu"))
    }

    /// Arşivlerin günlük farklı kişi sayıları (aynı gün birden çoksa ilk arşivinki) ve en yeni dosya değişiklik zamanı.
    pub fn sayilar(&self, arsivler: &[String]) -> (BTreeMap<String, Value>, Option<f64>) {
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        let mut guncel: Option<f64> = None;
        for ad in arsivler {
            let yol = self.ist_dizini.join(format!("{ad}.json"));
            if let Some(Value::Object(v)) = json_oku(&yol) {
                for (g, n) in v {
                    out.entry(g).or_insert(n);
                }
                let mtime = std::fs::metadata(&yol).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64());
                if let Some(m) = mtime {
                    guncel = Some(guncel.unwrap_or(0.0).max(m));
                }
            }
        }
        (out, guncel)
    }

    /// {gün: özeti bulunan ilk arşiv}.
    pub fn gunler(&self, arsivler: &[String]) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for ad in arsivler {
            let Ok(d) = std::fs::read_dir(self.detay_dizini.join(ad)) else { continue };
            let mut adlar: Vec<String> = d.filter_map(|x| x.ok().and_then(|x| x.file_name().into_string().ok())).collect();
            adlar.sort();
            for a in adlar {
                if let Some(gun) = a.strip_suffix(".json") {
                    if detay::gecerli_gun(gun) {
                        out.entry(gun.to_string()).or_insert_with(|| ad.clone());
                    }
                }
            }
        }
        out
    }

    pub fn ozet_html(&self, baslik: &str, arsivler: &[String], simdi: f64, onek: &str, csrf: &str) -> String {
        let (sayilar, guncel) = self.sayilar(arsivler);
        let tablo = if sayilar.is_empty() {
            "<p class=\"alt\">Henüz veri yok. Cihazdan ilk gün geldiğinde burada görünecek.</p>".to_string()
        } else {
            let gunler = son_30_gun(simdi);
            let deger = |g: &NaiveDate| sayilar.get(&g.format("%Y-%m-%d").to_string()).filter(|v| !v.is_null());
            let enbuyuk = gunler.iter().map(|g| detay::int_of(deger(g))).chain(std::iter::once(1)).max().unwrap_or(1);
            let mut satirlar = String::new();
            for g in &gunler {
                let iso = g.format("%Y-%m-%d").to_string();
                let tarih = format!("<td>{}</td><td>{}</td>", ortak::tarih_goster(&iso, false), detay::KISA_GUN[g.weekday().num_days_from_monday() as usize]);
                match deger(g) {
                    None => satirlar.push_str(&format!("<tr>{tarih}<td class=\"cubuk\"></td><td class=\"n bos\">—</td></tr>")),
                    Some(v) => {
                        let n = detay::int_of(Some(v));
                        satirlar.push_str(&format!(
                            "<tr>{tarih}<td class=\"cubuk\"><span style=\"width:{}%\"></span></td><td class=\"n\">{n}</td></tr>",
                            n.saturating_mul(100).div_euclid(enbuyuk)
                        ));
                    }
                }
            }
            format!("<table>{satirlar}</table>")
        };
        let son = match guncel {
            Some(g) if g != 0.0 => format!("<p class=\"alt\">Son güncelleme: {}</p>", ortak::tarih_goster(&ortak::now_iso(g), false)),
            _ => String::new(),
        };
        let gun_listesi: Vec<String> = self.gunler(arsivler).into_keys().collect();
        format!(
            "<div class=\"kart\"><h1>{}</h1>\
             <p class=\"alt\">Son 30 gün · her gün internete çıkan farklı kişi sayısı</p></div>\
             <div class=\"kart\">{tablo}{son}</div>\
             <div class=\"kart\"><h2>Kişi ara</h2>{}</div>\
             {}",
            e(baslik),
            detay::arama_formu("", onek, csrf, "kisi", ""),
            detay::gunler_html(&gun_listesi, &sayilar, onek)
        )
    }

    /// Gün sayfası; gün geçersizse ya da özeti yoksa None.
    pub fn gun_html(&self, arsivler: &[String], gun: &str, onek: &str) -> Result<Option<String>, String> {
        if !detay::gecerli_gun(gun) {
            return Ok(None);
        }
        let ad = self.gunler(arsivler).get(gun).cloned();
        let ozet = ad.and_then(|ad| json_oku(&self.detay_dizini.join(ad).join(format!("{gun}.json"))));
        match ozet {
            Some(o) if o.is_object() => Ok(Some(detay::gun_icerik(gun, &o, &self.sir()?, onek))),
            _ => Ok(None),
        }
    }

    /// form None: yalnızca boş arama formu (GET). Değilse POST gövdesi: q, tur (kisi|oda), uyruk.
    pub fn ara_html(&self, arsivler: &[String], form: Option<&HashMap<String, String>>, onek: &str, csrf: &str) -> Result<String, String> {
        let Some(form) = form else {
            return Ok(detay::ara_icerik("", None, b"", onek, csrf, "kisi", "", ""));
        };
        let ilk = |s: &str, n: usize| -> String { py_strip(s).chars().take(n).collect() };
        let q = ilk(form.get("q").map(String::as_str).unwrap_or(""), 50);
        let tur = if form.get("tur").map(String::as_str) == Some("oda") { "oda" } else { "kisi" };
        let uyruk = ilk(form.get("uyruk").map(String::as_str).unwrap_or(""), 2);
        let mut dizin: Dizin = Vec::new();
        let mut konum: HashMap<String, usize> = HashMap::new();
        for ad in arsivler {
            let Some(Value::Object(d)) = json_oku(&self.detay_dizini.join(ad).join("kisiler.json")) else { continue };
            for (tel, k) in d {
                let Value::Object(k) = k else { continue };
                let odalar: Map<String, Value> = match k.get("odalar") {
                    Some(Value::Object(o)) => o.clone(),
                    _ => Map::new(),
                };
                let gunler_of = |v: Option<&Value>| -> Vec<String> {
                    match v {
                        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                        _ => Vec::new(),
                    }
                };
                let metinler = |l: Vec<String>| Value::Array(l.into_iter().map(Value::String).collect());
                if let Some(&i) = konum.get(&tel) {
                    // aynı kişi başka arşivde de: günler birleşir (yeni → eski), odalar günleriyle birleşir
                    let Value::Object(x) = &mut dizin[i].1 else { continue };
                    let mut g = gunler_of(x.get("gunler"));
                    g.extend(gunler_of(k.get("gunler")));
                    g.sort_by(|a, b| b.cmp(a));
                    g.dedup();
                    x.insert("gunler".into(), metinler(g));
                    let mut xo = match x.get("odalar") {
                        Some(Value::Object(o)) => o.clone(),
                        _ => Map::new(),
                    };
                    for (oda, gunler) in &odalar {
                        let mut l = gunler_of(xo.get(oda));
                        l.extend(gunler_of(Some(gunler)));
                        l.sort();
                        l.dedup();
                        xo.insert(oda.clone(), metinler(l));
                    }
                    x.insert("odalar".into(), Value::Object(xo));
                } else {
                    let mut y = k.clone();
                    y.insert("gunler".into(), metinler(gunler_of(k.get("gunler"))));
                    y.insert("odalar".into(), Value::Object(odalar.iter().map(|(o, g)| (o.clone(), metinler(gunler_of(Some(g))))).collect()));
                    konum.insert(tel.clone(), dizin.len());
                    dizin.push((tel, Value::Object(y)));
                }
            }
        }
        let yeterli = q.chars().count() >= if tur == "oda" { 1 } else { 2 };
        let sonuclar = if yeterli { Some(detay::ara_sonuclari(&dizin, &q, tur, &uyruk)) } else { None };
        let geri = if tur == "kisi" && detay::kimlik_sorgusu_mu(&q, &dizin) { "" } else { q.as_str() };
        Ok(detay::ara_icerik(geri, sonuclar.as_deref(), &self.sir()?, onek, csrf, tur, &uyruk, if yeterli { "" } else { "En az 2 karakter yazın." }))
    }
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::altin::Kayit;
    use crate::test_yardim::{gun_yaz, ortam, sayi_yaz, Ortam};
    use serde_json::json;

    fn kayitlar(o: &Ortam) -> Kayitlar {
        Kayitlar::new(o.ist.clone(), o.detay.clone(), o.kisi_sir.clone())
    }

    fn adlar(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn tek(anahtar: &str, v: Value) -> Value {
        let mut m = Map::new();
        m.insert(anahtar.into(), v);
        Value::Object(m)
    }

    #[test]
    fn birden_cok_arsiv_birlesir_ilki_gecerli() {
        let _k = Kayit::bas("test_kayitlar", "test_birden_cok_arsiv_birlesir_ilki_gecerli");
        let o = ortam();
        let k = kayitlar(&o);
        let dun = son_30_gun(o.saat.t())[0].format("%Y-%m-%d").to_string();
        let mut s1 = Map::new();
        s1.insert(dun.clone(), json!(5));
        sayi_yaz(&o.dizin, "100001", &Value::Object(s1));
        let mut s2 = Map::new();
        s2.insert(dun.clone(), json!(99));
        s2.insert("2026-09-01".into(), json!(7));
        sayi_yaz(&o.dizin, "bocafe", &Value::Object(s2));
        let (sayilar, _) = k.sayilar(&adlar(&["100001", "bocafe"]));
        let beklenen: BTreeMap<String, Value> = [(dun.clone(), json!(5)), ("2026-09-01".to_string(), json!(7))].into_iter().collect();
        assert_eq!(sayilar, beklenen);
        gun_yaz(&o.dizin, "100001", "2026-09-20", &[], Some(&json!({"5330000001": {"ad": "Ayşe", "gunler": ["2026-09-20"]}})));
        gun_yaz(&o.dizin, "bocafe", "2026-09-20", &[], Some(&json!({"5330000001": {"ad": "Ayşe", "gunler": ["2026-09-01"]}})));
        gun_yaz(&o.dizin, "bocafe", "2026-09-01", &[], None);
        let gunler = k.gunler(&adlar(&["100001", "bocafe"]));
        let beklenen_gunler: BTreeMap<String, String> = [("2026-09-20".to_string(), "100001".to_string()), ("2026-09-01".to_string(), "bocafe".to_string())].into_iter().collect();
        assert_eq!(gunler, beklenen_gunler);
        let form: HashMap<String, String> = [("q".to_string(), "ayşe".to_string())].into_iter().collect();
        let s = k.ara_html(&adlar(&["100001", "bocafe"]), Some(&form), "/m/100001/kayitlar", "").unwrap();
        let capa = format!("k{}", detay::opak(&k.sir().unwrap(), "5330000001"));
        assert!(s.contains(&format!("href=\"/m/100001/kayitlar/gun/2026-09-20#{capa}\"")) && s.contains(&format!("/gun/2026-09-01#{capa}")));
        let bos = k.ara_html(&adlar(&["100001"]), None, "", "").unwrap();
        assert!(bos.contains("method=\"post\"") && !bos.contains("Ayşe"));
    }

    #[test]
    fn ozet_gun_ve_bilinmeyen() {
        let _k = Kayit::bas("test_kayitlar", "test_ozet_gun_ve_bilinmeyen");
        let o = ortam();
        let k = kayitlar(&o);
        let dun = son_30_gun(o.saat.t())[0].format("%Y-%m-%d").to_string();
        sayi_yaz(&o.dizin, "100001", &tek(&dun, json!(14)));
        gun_yaz(&o.dizin, "100001", "2026-09-20", &[json!({"telefon": "5330000001", "ad": "<b>x</b>"})], None);
        let s = k.ozet_html("Bocafe <Göztepe>", &adlar(&["100001"]), o.saat.t(), "", "");
        assert!(s.contains("Bocafe &lt;Göztepe&gt;") && s.contains(">14<") && s.contains("href=\"/gun/2026-09-20\""));
        assert!(k.gun_html(&adlar(&["100001"]), "2026-09-20", "").unwrap().unwrap().contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(k.gun_html(&adlar(&["100001"]), "2026-09-21", "").unwrap().is_none());
        assert!(k.gun_html(&adlar(&["100001"]), "../../etc", "").unwrap().is_none());
        assert!(k.ozet_html("X", &adlar(&["yok"]), o.saat.t(), "", "").contains("Henüz veri yok"));
    }

    #[test]
    fn son_30_gun_tr_sinirlari() {
        // 2026-11-04 23:59:59 +03:00 → dün 11-03; 00:00:00 +03:00 (11-05) → dün 11-04
        let g = son_30_gun(1_793_825_999.0);
        assert_eq!((g[0].to_string(), g[29].to_string(), g.len()), ("2026-11-03".to_string(), "2026-10-05".to_string(), 30));
        assert_eq!(son_30_gun(1_793_826_000.0)[0].to_string(), "2026-11-04");
    }
}
