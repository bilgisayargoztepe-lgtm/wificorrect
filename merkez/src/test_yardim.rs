//! Uygulama testlerinin ortak kurulumu (Python tests/yardim.py): geçici veritabanı, özet klasörleri, sahte saat, istek
//! yardımcıları. İstek yardımcıları cevabı açık altın kayda (`altin::Kayit`) ekler; Python'da kaydedilen her üst düzey
//! `istek` çağrısı Rust testinde bu yardımcılardan biriyle yapılmalıdır (kayda girmeyecek ek istek: `app.istek(...)`).
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

use crate::merkez::Uygulama;
use crate::veri::{Saat, Veri};
use crate::web::{Istek, Yanit};

/// Python `yardim.Saat.t`: 2026-09-21 ~ 17:13 (+03).
pub const SAAT_BASLANGIC: f64 = 1_790_000_000.0;
/// Yardımcıların varsayılan istemci adresi (Python `ip="203.0.113.5"`).
pub const IP: &str = "203.0.113.5";

/// Python `yardim.Saat`: ayarlanabilir sahte saat (`saat.t += 1801` → `saat.ilerlet(1801.0)`).
#[derive(Clone)]
pub struct SahteSaat(Arc<Mutex<f64>>);

impl SahteSaat {
    pub fn yeni(t: f64) -> SahteSaat {
        SahteSaat(Arc::new(Mutex::new(t)))
    }

    pub fn t(&self) -> f64 {
        *crate::ortak::kilitle(&self.0)
    }

    pub fn koy(&self, t: f64) {
        *crate::ortak::kilitle(&self.0) = t;
    }

    pub fn ilerlet(&self, sn: f64) {
        *crate::ortak::kilitle(&self.0) += sn;
    }

    /// `Veri`'ye ve uygulamalara verilecek saat.
    pub fn saat(&self) -> Saat {
        let s = self.clone();
        Arc::new(move || s.t())
    }
}

impl Default for SahteSaat {
    fn default() -> Self {
        SahteSaat::yeni(SAAT_BASLANGIC)
    }
}

/// Python `ortam(tmp)` → (v, k, saat). `kayitlar::Kayitlar` (Task 5) `ist`, `detay` ve `kisi_sir` yollarıyla kurulur.
pub struct Ortam {
    /// Python `tmp`.
    pub dizin: PathBuf,
    pub veri: Arc<Veri>,
    pub saat: SahteSaat,
    /// tmp/ist (günlük sayılar, `sayi_yaz`)
    pub ist: PathBuf,
    /// tmp/detay (gün özetleri, `gun_yaz`)
    pub detay: PathBuf,
    /// tmp/kisi_sir
    pub kisi_sir: PathBuf,
}

/// Yeni, boş geçici dizin (Python `tempfile.TemporaryDirectory()` / `mkdtemp()`); açık altın kayda bildirilir: testin
/// sonunda içindeki dosyalar Python'unkilerle karşılaştırılır. Python testi kaç geçici dizin açtıysa o kadar, aynı sırayla.
pub fn gecici_dizin() -> PathBuf {
    let d = crate::ortak::tests::gecici_dizin();
    crate::altin::dizin_acildi(&d);
    d
}

/// Yeni geçici dizinde ortam (Python `with tempfile.TemporaryDirectory() as tmp: ... Y.ortam(tmp)`).
pub fn ortam() -> Ortam {
    ortam_dizinde(&gecici_dizin())
}

/// Python `ortam(tmp)`: tmp/m.db, tmp/ist, tmp/detay.
pub fn ortam_dizinde(tmp: &Path) -> Ortam {
    let saat = SahteSaat::default();
    let veri = Arc::new(Veri::ac(&tmp.join("m.db"), saat.saat()).expect("veritabanı"));
    let (ist, detay) = (tmp.join("ist"), tmp.join("detay"));
    std::fs::create_dir(&ist).expect("ist");
    std::fs::create_dir(&detay).expect("detay");
    Ortam { dizin: tmp.to_path_buf(), veri, saat, ist, detay, kisi_sir: tmp.join("kisi_sir") }
}

/// Python `json.dumps(v)` (varsayılan: ", " ve ": " ayraçları, ASCII dışı `\uXXXX`).
pub fn json_govde(v: &Value) -> String {
    crate::veri::py_json(v, true)
}

/// Python `sayi_yaz(tmp, ad, sayilar)`: tmp/ist/<ad>.json; değişiklik zamanı sahte saatin başlangıcı ("Son güncelleme").
pub fn sayi_yaz(tmp: &Path, ad: &str, sayilar: &Value) {
    let yol = tmp.join("ist").join(format!("{ad}.json"));
    std::fs::write(&yol, json_govde(sayilar)).expect("sayılar");
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs_f64(SAAT_BASLANGIC);
    let f = std::fs::OpenOptions::new().write(true).open(&yol).expect("sayılar");
    f.set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t)).expect("zaman");
}

/// Python `gun_yaz(tmp, ad, gun, kisiler=(), dizin=None)`: tmp/detay/<ad>/<gün>.json (+ kisiler.json).
pub fn gun_yaz(tmp: &Path, ad: &str, gun: &str, kisiler: &[Value], dizin: Option<&Value>) {
    let d = tmp.join("detay").join(ad);
    std::fs::create_dir_all(&d).expect("detay");
    std::fs::write(d.join(format!("{gun}.json")), json_govde(&json!({ "gun": gun, "kisiler": kisiler }))).expect("gün");
    if let Some(dz) = dizin {
        std::fs::write(d.join("kisiler.json"), json_govde(dz)).expect("kişiler");
    }
}

/// Python `cerez(yanit)`: Set-Cookie'nin ilk `;`'a kadarı ("ad=değer").
pub fn cerez(y: &Yanit) -> String {
    let sc = y.baslik("Set-Cookie").unwrap_or_else(|| panic!("Set-Cookie yok (durum {})", y.durum));
    sc.split(';').next().unwrap_or("").to_string()
}

/// Python `csrf(yanit)`: gövdedeki ilk boş olmayan `name="csrf" value="..."`.
pub fn csrf(y: &Yanit) -> String {
    let h = y.govde_metin();
    let ayrac = "name=\"csrf\" value=\"";
    let mut bas = 0;
    while let Some(i) = h[bas..].find(ayrac) {
        let d = bas + i + ayrac.len();
        let son = h[d..].find('"').map(|j| d + j).unwrap_or(h.len());
        if son > d {
            return h[d..son].to_string();
        }
        bas = d;
    }
    panic!("gövdede csrf yok (durum {})", y.durum)
}

/// Python `urllib.parse.urlencode(alanlar)` (quote_plus: boşluk `+`, harf/rakam ve `_.-~` dışı `%XX`).
pub fn urlencode(alanlar: &[(&str, &str)]) -> String {
    fn kodla(s: &str) -> String {
        let mut o = String::with_capacity(s.len());
        for &b in s.as_bytes() {
            match b {
                b' ' => o.push('+'),
                b if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') => o.push(b as char),
                b => o.push_str(&format!("%{b:02X}")),
            }
        }
        o
    }
    alanlar.iter().map(|(k, v)| format!("{}={}", kodla(k), kodla(v))).collect::<Vec<_>>().join("&")
}

/// Uygulamaya istek (Python `app.istek(yontem, yol, basliklar, govde, sorgu, ip)`); cevap altın kayda eklenir.
pub fn istek(app: &dyn Uygulama, yontem: &str, yol: &str, basliklar: &[(&str, &str)], govde: &str, sorgu: &str, ip: &str) -> Yanit {
    let mut i = Istek::yeni(yontem, yol).govde(govde).sorgu(sorgu).ip(ip);
    for (k, v) in basliklar {
        i = i.baslik(k, v);
    }
    let y = app.istek(&i);
    crate::altin::cevap_ekle(&i, &y);
    y
}

/// Python `al(app, yol, c="")`: GET; yol `?`'dan bölünür (sorgu).
pub fn al(app: &dyn Uygulama, yol: &str, c: &str) -> Yanit {
    al_ip(app, yol, c, IP)
}

pub fn al_ip(app: &dyn Uygulama, yol: &str, c: &str, ip: &str) -> Yanit {
    let (y, s) = yol.split_once('?').unwrap_or((yol, ""));
    istek(app, "GET", y, &[("Cookie", c)], "", s, ip)
}

/// Python `gonder(app, yol, c, alanlar)`: form gövdeli POST (yol bölünmez).
pub fn gonder(app: &dyn Uygulama, yol: &str, c: &str, alanlar: &[(&str, &str)]) -> Yanit {
    gonder_ip(app, yol, c, alanlar, IP)
}

pub fn gonder_ip(app: &dyn Uygulama, yol: &str, c: &str, alanlar: &[(&str, &str)], ip: &str) -> Yanit {
    istek(app, "POST", yol, &[("Cookie", c)], &urlencode(alanlar), "", ip)
}

/// Python `giris(app, kul, pw)`: çerezsiz POST /giris.
pub fn giris(app: &dyn Uygulama, kul: &str, pw: &str) -> Yanit {
    giris_ip(app, kul, pw, IP)
}

pub fn giris_ip(app: &dyn Uygulama, kul: &str, pw: &str, ip: &str) -> Yanit {
    istek(app, "POST", "/giris", &[], &urlencode(&[("kullanici", kul), ("sifre", pw)]), "", ip)
}

/// Python `gerekce_ver(app, c)`: gerekçe sayfası + "Müşteri şikâyeti" (iki istek).
pub fn gerekce_ver(app: &dyn Uygulama, c: &str) -> Yanit {
    gerekce_ver_metin(app, c, "Müşteri şikâyeti")
}

pub fn gerekce_ver_metin(app: &dyn Uygulama, c: &str, metin: &str) -> Yanit {
    let f = al(app, "/gerekce?donus=%2F", c);
    gonder(app, "/gerekce", c, &[("csrf", &csrf(&f)), ("gerekce", metin), ("donus", "/")])
}

/// Sahte dış taşıyıcının sıradaki cevabı (Python testlerindeki `Ag`: bytes, (durum, bytes) ya da istisna).
pub enum AgCevap {
    /// 200 + gövde.
    Govde(&'static str),
    /// Durum + gövde (4xx/5xx de hata değil, cevap).
    Durum(u16, &'static str),
    /// Ağ hatası / zaman aşımı.
    Hata,
}

/// Taşıyıcıya giden isteğin test kaydı.
pub struct GidenIstek {
    pub yontem: String,
    pub url: String,
    pub basliklar: Vec<(String, String)>,
    pub govde: String,
    pub kullanici: Option<(String, String)>,
    pub zaman_asimi: Duration,
}

/// Python testlerindeki sahte `urlopen`: her isteği kaydeder, sıradaki cevabı verir; sıra boşsa NetGSM başarısı.
#[derive(Default)]
pub struct SahteAg {
    pub cevaplar: Mutex<std::collections::VecDeque<AgCevap>>,
    pub istekler: Mutex<Vec<GidenIstek>>,
}

impl SahteAg {
    pub fn yeni(cevaplar: Vec<AgCevap>) -> Arc<SahteAg> {
        Arc::new(SahteAg { cevaplar: Mutex::new(cevaplar.into()), istekler: Mutex::new(Vec::new()) })
    }

    pub fn ekle(&self, c: AgCevap) {
        self.cevaplar.lock().unwrap().push_back(c);
    }

    /// Sıranın başına (Python `cevaplar.insert(0, ...)`).
    pub fn basa_ekle(&self, c: AgCevap) {
        self.cevaplar.lock().unwrap().push_front(c);
    }

    pub fn sayi(&self) -> usize {
        self.istekler.lock().unwrap().len()
    }

    /// Son isteği `f`'ye verir.
    pub fn son<R>(&self, f: impl FnOnce(&GidenIstek) -> R) -> R {
        f(self.istekler.lock().unwrap().last().expect("hiç istek gitmedi"))
    }
}

impl crate::ortak::Tasiyici for SahteAg {
    fn gonder(&self, i: &crate::ortak::DisIstek) -> Result<(u16, Vec<u8>), String> {
        self.istekler.lock().unwrap().push(GidenIstek {
            yontem: i.yontem.clone(),
            url: i.url.clone(),
            basliklar: i.basliklar.clone(),
            govde: i.govde.as_ref().map(|g| String::from_utf8_lossy(g).into_owned()).unwrap_or_default(),
            kullanici: i.kullanici.clone(),
            zaman_asimi: i.zaman_asimi,
        });
        match self.cevaplar.lock().unwrap().pop_front() {
            None => Ok((200, b"<xml><main><code>0</code><jobID>111</jobID></main></xml>".to_vec())),
            Some(AgCevap::Govde(g)) => Ok((200, g.as_bytes().to_vec())),
            Some(AgCevap::Durum(d, g)) => Ok((d, g.as_bytes().to_vec())),
            Some(AgCevap::Hata) => Err("ağ".into()),
        }
    }
}

/// Python `ag_yok`: ağ isteği gitmemeliydi.
pub struct AgYok;

impl crate::ortak::Tasiyici for AgYok {
    fn gonder(&self, _: &crate::ortak::DisIstek) -> Result<(u16, Vec<u8>), String> {
        panic!("ağ isteği gitmemeliydi")
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn urlencode_python_ile_ayni() {
        // py: urllib.parse.urlencode({"kullanici": " 1234567 ", "sifre": "ş&=+/?%~._-", "x": ""})
        assert_eq!(urlencode(&[("kullanici", " 1234567 "), ("sifre", "ş&=+/?%~._-"), ("x", "")]), "kullanici=+1234567+&sifre=%C5%9F%26%3D%2B%2F%3F%25~._-&x=");
        assert_eq!(urlencode(&[("gerekce", "Müşteri şikâyeti"), ("donus", "/")]), "gerekce=M%C3%BC%C5%9Fteri+%C5%9Fik%C3%A2yeti&donus=%2F");
    }

    #[test]
    fn json_govde_python_dumps_gibi() {
        // py: json.dumps({"gun": "2026-09-20", "kisiler": [{"ad": "Ayşe", "n": 1.5}]})
        assert_eq!(json_govde(&json!({"gun": "2026-09-20", "kisiler": [{"ad": "Ayşe", "n": 1.5}]})), r#"{"gun": "2026-09-20", "kisiler": [{"ad": "Ay\u015fe", "n": 1.5}]}"#);
    }

    #[test]
    fn cerez_ve_csrf() {
        let y = crate::web::yonlendir("/", Some(("wc", Some("tok"))));
        assert_eq!(cerez(&y), "wc=tok");
        let y = crate::web::yanit_html("<input name=\"csrf\" value=\"\"><input name=\"csrf\" value=\"abc\">", 200, None);
        assert_eq!(csrf(&y), "abc");
    }

    #[test]
    fn ortam_ve_saat() {
        let o = ortam();
        assert!(o.dizin.join("m.db").exists() && o.ist.is_dir() && o.detay.is_dir());
        assert_eq!(o.saat.t(), SAAT_BASLANGIC);
        o.saat.ilerlet(1801.0);
        assert_eq!((o.saat.saat())(), SAAT_BASLANGIC + 1801.0);
        sayi_yaz(&o.dizin, "100001", &json!({"2026-09-20": 5}));
        let yol = o.ist.join("100001.json");
        assert_eq!(std::fs::read_to_string(&yol).unwrap(), r#"{"2026-09-20": 5}"#);
        let mt = std::fs::metadata(&yol).unwrap().modified().unwrap();
        assert_eq!(mt.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs_f64(), SAAT_BASLANGIC);
        gun_yaz(&o.dizin, "100001", "2026-09-20", &[json!({"telefon": "5330000001", "ad": "Ayşe"})], Some(&json!({})));
        assert!(o.detay.join("100001/2026-09-20.json").exists() && o.detay.join("100001/kisiler.json").exists());
    }
}
