//! Merkezden uzaktan güncelleme (spec 2026-10-12): sağlık cevabındaki hedef sürüm, cihazdaki karar mantığı ve
//! güncelleme sonucunun uzlaştırılması. İndirme, kurma ve geri alma `ctl guncelle` ile denetim betiğindedir.

use crate::ortak::{self, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Yerel saat dilimi: Türkiye UTC+3, yaz saati yok (güncelleme penceresi TR saatiyle).
const TR: f64 = 3.0 * 3600.0;
/// Başarısız (hata) bir sürüm için `hemen` olmayan yeni deneme bekleme süresi.
const HATA_BEKLE_SN: f64 = 43200.0;

pub struct Yollar {
    pub kok: PathBuf,
}

// onceki_deb, betik ve kilit `ctl guncelle` ile denetim betiği tarafından kullanılacak
#[allow(dead_code)]
impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar { kok: "/var/lib/wificorrect/guncelleme".into() }
    }
    pub fn durum(&self) -> PathBuf {
        self.kok.join("durum.json")
    }
    pub fn hedef(&self) -> PathBuf {
        self.kok.join("hedef.json")
    }
    pub fn paketler(&self) -> PathBuf {
        self.kok.join("paketler")
    }
    pub fn bekleyen(&self) -> PathBuf {
        self.kok.join("bekleyen.json")
    }
    pub fn sonuc(&self) -> PathBuf {
        self.kok.join("sonuc")
    }
    pub fn onceki_deb(&self) -> PathBuf {
        self.kok.join("onceki.deb")
    }
    pub fn betik(&self) -> PathBuf {
        self.kok.join("denetle.sh")
    }
    pub fn saglik_basari(&self) -> PathBuf {
        self.kok.join("saglik_basari")
    }
    pub fn kilit(&self) -> PathBuf {
        self.kok.join("kilit")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hedef {
    pub surum: String,
    pub sha256: String,
    pub boyut: u64,
    #[serde(default)]
    pub hemen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sonuc {
    pub surum: String,
    /// tamam | geri_alindi | hata
    pub sonuc: String,
    pub zaman: u64,
    #[serde(default)]
    pub neden: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Durum {
    #[serde(default)]
    pub basarisiz: Vec<String>,
    #[serde(default)]
    pub son: Option<Sonuc>,
}

/// `X.Y.Z` (yalnızca rakamlar).
pub fn surum_gecerli(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit()))
}

/// Merkez cevabındaki `guncelleme` nesnesi; geçersiz/eksik → None.
pub fn hedef_coz(v: &Value) -> Option<Hedef> {
    let surum = v.get("surum")?.as_str()?;
    let sha = v.get("sha256")?.as_str()?;
    let boyut = v.get("boyut")?.as_u64()?;
    if !surum_gecerli(surum) || sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) || boyut == 0 {
        return None;
    }
    let hemen = v.get("hemen").and_then(Value::as_bool).unwrap_or(false);
    Some(Hedef { surum: surum.into(), sha256: sha.into(), boyut, hemen })
}

/// Yok ya da bozuk dosya → varsayılan.
pub fn durum_oku(y: &Yollar) -> Durum {
    std::fs::read(y.durum()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn durum_yaz(y: &Yollar, d: &Durum) -> std::io::Result<()> {
    ortak::write_atomic(&y.durum(), &serde_json::to_vec_pretty(d).unwrap_or_default())
}

/// Kurulu paket sürümü (`dpkg-query`); hata ya da boş çıktı → "".
pub fn kurulu(komut: &dyn Fn(&[&str]) -> Option<String>) -> String {
    komut(&["dpkg-query", "-W", "-f=${Version}", "wificorrect"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

#[derive(Debug, PartialEq)]
pub enum Karar {
    Kur,
    Bekle(&'static str),
}

/// TR saati 03:00 ≤ t < 06:00.
pub fn pencere(now: f64) -> bool {
    let sn = (now + TR).rem_euclid(86400.0);
    (3.0 * 3600.0..6.0 * 3600.0).contains(&sn)
}

pub fn karar(kurulu: &str, h: &Hedef, d: &Durum, bekleyen_var: bool, now: f64) -> Karar {
    if kurulu.is_empty() {
        return Karar::Bekle("paketsiz");
    }
    if kurulu == h.surum {
        return Karar::Bekle("guncel");
    }
    if bekleyen_var {
        return Karar::Bekle("denetim");
    }
    if !h.hemen {
        if d.basarisiz.contains(&h.surum) {
            return Karar::Bekle("basarisiz");
        }
        if d.son.as_ref().is_some_and(|s| s.sonuc == "hata" && s.surum == h.surum && now - (s.zaman as f64) < HATA_BEKLE_SN) {
            return Karar::Bekle("hata_bekle");
        }
        if !pencere(now) {
            return Karar::Bekle("pencere");
        }
    }
    Karar::Kur
}

/// Denetim betiğinin bıraktığı sonucu işler (`bekleyen.json` + `sonuc`), sonra eski paket dosyalarını temizler.
pub fn uzlas(y: &Yollar, log_root: &str, now: f64) {
    if y.bekleyen().exists() && y.sonuc().exists() {
        let b: Value = std::fs::read(y.bekleyen()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        let yeni = b.get("yeni").and_then(Value::as_str).unwrap_or("").to_string();
        let onceki = b.get("onceki").and_then(Value::as_str).unwrap_or("").to_string();
        let sonuc = std::fs::read_to_string(y.sonuc()).unwrap_or_default();
        let sonuc = sonuc.trim();
        let tamam = sonuc == "tamam";
        if !yeni.is_empty() && (tamam || sonuc == "geri") {
            let mut d = durum_oku(y);
            let zaman = now.max(0.0) as u64;
            let ek = format!("yeni={yeni} onceki={onceki}");
            if tamam {
                ortak::audit(log_root, Row::new("GUNCELLEME_TAMAM", &ortak::now_iso(now)).set("ek", ek));
                d.basarisiz.retain(|s| *s != yeni);
            } else {
                ortak::audit(log_root, Row::new("GUNCELLEME_GERI_ALINDI", &ortak::now_iso(now)).set("ek", ek));
                if !d.basarisiz.contains(&yeni) {
                    d.basarisiz.push(yeni.clone());
                }
            }
            d.son = Some(Sonuc { surum: yeni, sonuc: if tamam { "tamam" } else { "geri_alindi" }.into(), zaman, neden: String::new() });
            let _ = durum_yaz(y, &d);
        }
        let _ = std::fs::remove_file(y.bekleyen());
        let _ = std::fs::remove_file(y.sonuc());
    }
    paket_temizle(y);
}

/// `paketler/` içinde en yeni 2 dosya kalır (kurulu ve önceki sürüm).
fn paket_temizle(y: &Yollar) {
    let Ok(rd) = std::fs::read_dir(y.paketler()) else { return };
    let mut f: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
        .collect();
    f.sort_by(|a, b| b.cmp(a));
    for (_, p) in f.into_iter().skip(2) {
        let _ = std::fs::remove_file(p);
    }
}

/// Rapora eklenecek alanlar: kurulu paket sürümü (merkezin biçimine uymuyorsa "") ve son sonuç (yoksa null).
pub fn rapor_alanlari(y: &Yollar, kurulu: &str) -> (String, Value) {
    let paket = if surum_gecerli(kurulu) { kurulu.to_string() } else { String::new() };
    let g = durum_oku(y).son.map_or(Value::Null, |s| json!({"surum": s.surum, "sonuc": s.sonuc, "zaman": s.zaman}));
    (paket, g)
}

/// Merkezin hedefini işler: karar `Kur` ise `hedef.json` yazılır ve `baslat` çağrılır.
pub fn hedefi_isle(y: &Yollar, h: &Hedef, kurulu: &str, now: f64, baslat: &dyn Fn()) -> Karar {
    let k = karar(kurulu, h, &durum_oku(y), y.bekleyen().exists(), now);
    if k == Karar::Kur && ortak::write_atomic(&y.hedef(), &serde_json::to_vec_pretty(h).unwrap_or_default()).is_ok() {
        baslat();
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn kur() -> Yollar {
        static N: AtomicUsize = AtomicUsize::new(0);
        let kok = std::env::temp_dir().join(format!("wfc-gunc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&kok);
        std::fs::create_dir_all(&kok).unwrap();
        Yollar { kok }
    }

    fn hedef(hemen: bool) -> Hedef {
        Hedef { surum: "0.4.1".into(), sha256: "a".repeat(64), boyut: 100, hemen }
    }

    // 2026-10-12 00:00 UTC = TR 03:00
    const T0300: f64 = 1_791_763_200.0 + 0.0;

    fn tr_saat(s: f64) -> f64 {
        // TR gün başı (00:00) + s saat; 1_791_763_200 UTC = TR 03:00 → TR 00:00 = -3 saat
        T0300 - 3.0 * 3600.0 + s * 3600.0
    }

    #[test]
    fn pencere_sinirlari() {
        assert!(!pencere(tr_saat(3.0) - 60.0)); // 02:59
        assert!(pencere(tr_saat(3.0)));
        assert!(pencere(tr_saat(6.0) - 60.0)); // 05:59
        assert!(!pencere(tr_saat(6.0)));
    }

    #[test]
    fn karar_tablosu() {
        let now = tr_saat(4.0);
        let gece = now;
        let gunduz = tr_saat(14.0);
        let bos = Durum::default();
        assert_eq!(karar("", &hedef(false), &bos, false, gece), Karar::Bekle("paketsiz"));
        assert_eq!(karar("0.4.1", &hedef(false), &bos, false, gece), Karar::Bekle("guncel"));
        assert_eq!(karar("0.4.0", &hedef(false), &bos, true, gece), Karar::Bekle("denetim"));
        let basarisiz = Durum { basarisiz: vec!["0.4.1".into()], son: None };
        assert_eq!(karar("0.4.0", &hedef(false), &basarisiz, false, gece), Karar::Bekle("basarisiz"));
        assert_eq!(karar("0.4.0", &hedef(true), &basarisiz, false, gunduz), Karar::Kur); // hemen: başarısızı da dener
        let hata = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: gece as u64 - 3600, neden: String::new() }) };
        assert_eq!(karar("0.4.0", &hedef(false), &hata, false, gece), Karar::Bekle("hata_bekle"));
        assert_eq!(karar("0.4.0", &hedef(true), &hata, false, gece), Karar::Kur);
        let eski_hata = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: gece as u64 - 50000, neden: String::new() }) };
        assert_eq!(karar("0.4.0", &hedef(false), &eski_hata, false, gece), Karar::Kur);
        let baska = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.0".into(), sonuc: "hata".into(), zaman: gece as u64 - 10, neden: String::new() }) };
        assert_eq!(karar("0.3.0", &hedef(false), &baska, false, gece), Karar::Kur);
        assert_eq!(karar("0.4.0", &hedef(false), &bos, false, gunduz), Karar::Bekle("pencere"));
        assert_eq!(karar("0.4.0", &hedef(true), &bos, false, gunduz), Karar::Kur);
        assert_eq!(karar("0.4.0", &hedef(false), &bos, false, gece), Karar::Kur);
    }

    #[test]
    fn hedef_coz_dogrular() {
        let iyi = json!({"surum": "0.4.1", "sha256": "a".repeat(64), "boyut": 12345, "hemen": true});
        assert_eq!(hedef_coz(&iyi), Some(Hedef { surum: "0.4.1".into(), sha256: "a".repeat(64), boyut: 12345, hemen: true }));
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "0".repeat(64), "boyut": 1})).map(|h| h.hemen), Some(false));
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "A".repeat(64), "boyut": 1})), None); // büyük harf
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "a".repeat(63), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "g".repeat(64), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2", "sha256": "a".repeat(64), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2.3", "sha256": "a".repeat(64), "boyut": 0})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2.3", "sha256": "a".repeat(64)})), None);
        assert_eq!(hedef_coz(&Value::Null), None);
    }

    #[test]
    fn kurulu_bos_cikti() {
        assert_eq!(kurulu(&|_: &[&str]| Some("0.4.0\n".into())), "0.4.0");
        assert_eq!(kurulu(&|_: &[&str]| Some(String::new())), "");
        assert_eq!(kurulu(&|_: &[&str]| None), "");
        let gorulen = std::cell::RefCell::new(vec![]);
        kurulu(&|c: &[&str]| {
            gorulen.borrow_mut().extend(c.iter().map(|s| s.to_string()));
            None
        });
        assert_eq!(gorulen.borrow()[..], ["dpkg-query", "-W", "-f=${Version}", "wificorrect"]);
    }

    fn bekleyen_yaz(y: &Yollar, sonuc: &str) {
        std::fs::write(y.bekleyen(), r#"{"yeni":"0.4.1","onceki":"0.4.0","zaman":1}"#).unwrap();
        std::fs::write(y.sonuc(), format!("{sonuc}\n")).unwrap();
    }

    fn denetim(log_root: &str, now: f64) -> String {
        std::fs::read_to_string(ortak::day_file(log_root, ortak::day_of(&ortak::now_iso(now)), "denetim.csv")).unwrap_or_default()
    }

    #[test]
    fn uzlas_tamam_ve_geri() {
        let y = kur();
        let lr = y.kok.join("5651").display().to_string();
        let now = tr_saat(4.0);
        // tamam: durum + denetim, dosyalar silinir, başarısız listesinden çıkar
        durum_yaz(&y, &Durum { basarisiz: vec!["0.4.1".into(), "0.3.9".into()], son: None }).unwrap();
        bekleyen_yaz(&y, "tamam");
        uzlas(&y, &lr, now);
        let d = durum_oku(&y);
        assert_eq!(d.basarisiz, ["0.3.9"]);
        let s = d.son.unwrap();
        assert_eq!((s.surum.as_str(), s.sonuc.as_str(), s.zaman), ("0.4.1", "tamam", now as u64));
        assert!(!y.bekleyen().exists() && !y.sonuc().exists());
        let a = denetim(&lr, now);
        assert!(a.contains("GUNCELLEME_TAMAM") && a.contains("yeni=0.4.1") && a.contains("onceki=0.4.0"), "{a}");
        // geri iki kez: başarısız'da tek
        for _ in 0..2 {
            bekleyen_yaz(&y, "geri");
            uzlas(&y, &lr, now + 10.0);
        }
        let d = durum_oku(&y);
        assert_eq!(d.basarisiz.iter().filter(|s| *s == "0.4.1").count(), 1);
        assert_eq!(d.son.unwrap().sonuc, "geri_alindi");
        assert!(denetim(&lr, now).contains("GUNCELLEME_GERI_ALINDI"));
        assert!(!y.bekleyen().exists() && !y.sonuc().exists());
        // yalnız bekleyen (sonuç yok): dokunulmaz
        std::fs::write(y.bekleyen(), r#"{"yeni":"0.4.2","onceki":"0.4.1","zaman":1}"#).unwrap();
        uzlas(&y, &lr, now);
        assert!(y.bekleyen().exists());
    }

    #[test]
    fn uzlas_paketleri_temizler() {
        let y = kur();
        std::fs::create_dir_all(y.paketler()).unwrap();
        for (i, ad) in ["a.deb", "b.deb", "c.deb", "d.deb"].iter().enumerate() {
            let p = y.paketler().join(ad);
            std::fs::write(&p, "x").unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64 * 100);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        uzlas(&y, &y.kok.join("5651").display().to_string(), 1000.0);
        let mut kalan: Vec<String> = std::fs::read_dir(y.paketler()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        kalan.sort();
        assert_eq!(kalan, ["c.deb", "d.deb"]);
    }

    #[test]
    fn rapor_alanlari_paket_ve_son() {
        let y = kur();
        let (p, g) = rapor_alanlari(&y, "0.4.0");
        assert_eq!((p.as_str(), g), ("0.4.0", Value::Null));
        durum_yaz(&y, &Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: 77, neden: "x".into() }) }).unwrap();
        let (_, g) = rapor_alanlari(&y, "");
        assert_eq!(g, json!({"surum": "0.4.1", "sonuc": "hata", "zaman": 77})); // neden gitmez
        assert_eq!(rapor_alanlari(&y, "0.4.1-1").0, ""); // merkezin biçimine uymayan sürüm gönderilmez
    }

    #[test]
    fn hedefi_isle_yazar_ve_baslatir() {
        let y = kur();
        let sayac = std::cell::Cell::new(0);
        let b = || sayac.set(sayac.get() + 1);
        let gece = tr_saat(4.0);
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.1", gece, &b), Karar::Bekle("guncel"));
        assert_eq!((sayac.get(), y.hedef().exists()), (0, false));
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.0", tr_saat(14.0), &b), Karar::Bekle("pencere"));
        assert_eq!(sayac.get(), 0);
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.0", gece, &b), Karar::Kur);
        assert_eq!(sayac.get(), 1);
        let yazilan: Hedef = serde_json::from_slice(&std::fs::read(y.hedef()).unwrap()).unwrap();
        assert_eq!(yazilan, hedef(false));
    }
}
