//! wc-istatistik (Python istatistik.py): yönetim merkezi sayıları ve gün özetleri.
//! Her arşivin (wc-durum'un listesi: cihaz yedekleri + eski arşivler) eksiksiz gelmiş her günü için o gün internete çıkan
//! farklı telefon sayısını `/var/lib/wificorrect/istatistik/<arşiv>.json`'a yazar. Root çalışır; panel yalnızca bu dosyaları okur.
//!
//! Python'dan farklar: istisnalar `Result`'tır (Python `repr(e)` metni yaklaşık `Tür("ileti")`); arşiv listesi
//! (`durum.arsivler`) Task 12'de `durum` modülü gelene kadar burada yalnızca kaynak klasörleri için yeniden yazıldı.
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::ortak::{self, py_bosluk, py_strip, OkumaHata};

pub const CIKTI: &str = "/var/lib/wificorrect/istatistik";
pub const DETAY: &str = "/var/lib/wificorrect/detay";
pub const RAPOR: &str = "/var/lib/wificorrect/rapor";
pub const RAPOR_HAM: &str = "/var/lib/wificorrect/rapor-ham";
pub const RAPOR_GRUPLARI: &str = "/var/lib/wificorrect/merkez/rapor-gruplari.json";
/// Özetler root:wcpanel 640; testlerde None (sahiplik değişmez).
pub const PANEL_GRUBU: &str = "wcpanel";
pub const KAYIT: &str = "/etc/wireguard/wificorrect/kayit.csv";
pub const YENI: &str = "/srv/wificorrect-arsiv";
pub const ESKI: &str = "/srv/hotspot-arsiv";

pub type Satir = BTreeMap<String, String>;

/// Python istisnası: `Deger` bir `ValueError` (normal olmayan dosya, geçersiz UTF-8), `Diger` öteki (bozuk gzip, G/Ç).
#[derive(Debug)]
pub enum SatirHata {
    Deger(String),
    Diger(String),
}

impl fmt::Display for SatirHata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SatirHata::Deger(m) => write!(f, "ValueError({m:?})"),
            SatirHata::Diger(m) => write!(f, "OSError({m:?})"),
        }
    }
}

/// `\d{4}-\d{2}-\d{2}` (tam eşleşme, ASCII).
pub fn gun_adi_mi(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// MANIFEST var ve listelediği her dosya yerinde mi (rsync yarıdayken hayır). MANIFEST normal dosya değilse (sembolik bağ,
/// FIFO, klasör), okunamıyorsa ya da UTF-8 değilse hayır: root beklemesin, bozuk bir gün diğerlerini durdurmasın.
pub fn tamam_mi(gun_dizini: &Path) -> bool {
    let yol = gun_dizini.join("MANIFEST.sha256");
    let Ok(mut f) = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&yol) else {
        return false;
    };
    if !f.metadata().map(|m| m.is_file()).unwrap_or(false) {
        return false;
    }
    let mut ham = Vec::new();
    if f.read_to_end(&mut ham).is_err() {
        return false;
    }
    let Ok(metin) = String::from_utf8(ham) else { return false };
    let metin = metin.replace("\r\n", "\n").replace('\r', "\n"); // metin kipindeki satır sonu çevirisiyle aynı
    for s in metin.split('\n') {
        if py_strip(s).is_empty() || s.starts_with('#') {
            continue;
        }
        // Python `s.split(None, 1)`: iki parça yoksa (tek sözcük) `all(len(p) == 2)` yanlış
        let t = s.trim_start_matches(py_bosluk);
        let Some(i) = t.find(py_bosluk) else { return false };
        let kalan = t[i..].trim_start_matches(py_bosluk);
        if kalan.is_empty() || !gun_dizini.join(py_strip(kalan)).exists() {
            return false;
        }
    }
    true
}

/// Yol yoksa None; varsa ve normal dosya değilse (cihazdan gelmiş sembolik bağ/FIFO) `Deger`:
/// başka arşivi okutmasın, root görevini takmasın.
fn normal_dosya(yol: &Path) -> Result<Option<&Path>, SatirHata> {
    match std::fs::symlink_metadata(yol) {
        Err(_) => Ok(None),
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            Err(SatirHata::Deger(format!("{} normal dosya değil", yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default())))
        }
        Ok(_) => Ok(Some(yol)),
    }
}

/// CSV (ya da .gz) satırlarını akışla `f`'ye verir (Python üreteci gibi bellekte yalnızca bir satır); `f` false dönerse
/// okuma durur. Dosya yoksa hiçbir şey. Normal dosya değilse, gzip bozuksa ya da UTF-8 değilse `Deger`/`Diger`; alan
/// sınırı (`_csv.Error`) `ValueError` değildir: `Diger`.
pub fn satirlar(yol: &Path, f: &mut dyn FnMut(Satir) -> bool) -> Result<(), SatirHata> {
    let Some(yol) = normal_dosya(yol)? else { return Ok(()) };
    ortak::rows_akis(yol, f).map_err(|h| match h {
        OkumaHata::Utf8 => SatirHata::Deger("geçersiz UTF-8".into()),
        OkumaHata::Io(m) => SatirHata::Diger(m),
        OkumaHata::Csv(m) => SatirHata::Diger(format!("_csv.Error: {m}")),
    })
}

/// Günün trafik.csv.gz'sindeki farklı, boş olmayan telefon sayısı (trafik dosyası yoksa 0).
pub fn farkli_kisi(gun_dizini: &Path) -> Result<usize, SatirHata> {
    let mut kisiler: std::collections::HashSet<String> = std::collections::HashSet::new();
    satirlar(&gun_dizini.join("trafik.csv.gz"), &mut |r| {
        if let Some(t) = r.get("telefon").filter(|t| !t.is_empty()) {
            kisiler.insert(t.clone());
        }
        true
    })?;
    Ok(kisiler.len())
}

/// `<kafe>/gunluk` gerçek bir klasörse içindeki gün klasörü adları (sıralı); değilse boş.
pub fn gun_klasorleri(kafe_dizini: &Path) -> Result<Vec<String>, String> {
    let g = kafe_dizini.join("gunluk");
    let gercek = std::fs::symlink_metadata(&g).map(|m| m.is_dir()).unwrap_or(false);
    if !gercek {
        return Ok(Vec::new());
    }
    let mut adlar: Vec<String> = std::fs::read_dir(&g)
        .map_err(|h| format!("OSError({h})"))?
        .filter_map(|d| d.ok().and_then(|d| d.file_name().into_string().ok()))
        .filter(|a| gun_adi_mi(a))
        .collect();
    adlar.sort();
    Ok(adlar)
}

pub fn sembolik_mi(yol: &Path) -> bool {
    std::fs::symlink_metadata(yol).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

/// Henüz hesaplanmamış tamam günleri ekler. Bozuk gün atlanır ve sonraki çalıştırmada yeniden denenir.
/// Dönen: hata iletileri (gün başına); `Err`: G/Ç hatası (Python'da istisna).
pub fn guncelle(kafe_dizini: &Path, json_yolu: &Path) -> Result<Vec<String>, String> {
    let mut sayilar: BTreeMap<String, Value> = BTreeMap::new();
    match std::fs::read(json_yolu) {
        Ok(ham) => {
            if let Some(Value::Object(o)) = std::str::from_utf8(&ham).ok().and_then(|m| serde_json::from_str::<Value>(m).ok()) {
                sayilar = o.into_iter().collect();
            }
        }
        Err(h) if h.kind() == std::io::ErrorKind::NotFound => {}
        Err(h) => return Err(format!("OSError({h})")),
    }
    let mut hatalar = Vec::new();
    for gun in gun_klasorleri(kafe_dizini)? {
        let d = kafe_dizini.join("gunluk").join(&gun);
        if sayilar.contains_key(&gun) || sembolik_mi(&d) || !tamam_mi(&d) {
            continue;
        }
        match farkli_kisi(&d) {
            Ok(n) => {
                sayilar.insert(gun, Value::from(n));
            }
            Err(h) => hatalar.push(format!("{gun}: {h}")), // bozuk/kesik gz: yalnızca o gün düşer
        }
    }
    if let Some(ust) = json_yolu.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(ust).map_err(|h| format!("OSError({h})"))?;
    }
    ortak::save_json(json_yolu, &Value::Object(sayilar.into_iter().collect())).map_err(|h| format!("OSError({h})"))?;
    std::fs::set_permissions(json_yolu, std::fs::Permissions::from_mode(0o644)).map_err(|h| format!("OSError({h})"))?;
    Ok(hatalar)
}

/// İşlenecek yollar (Python `main`'in anahtar sözcükleri).
pub struct Yollar {
    pub kayit: PathBuf,
    pub yeni: PathBuf,
    pub eski: PathBuf,
    pub cikti: PathBuf,
    pub detay: PathBuf,
    /// `shutil.chown(group=)` adı; None: sahiplik değişmez (testler).
    pub grup: Option<String>,
    pub rapor_ham: PathBuf,
    pub rapor_cikti: PathBuf,
    pub rapor_gruplari: PathBuf,
}

impl Yollar {
    pub fn uretim() -> Yollar {
        Yollar {
            kayit: KAYIT.into(),
            yeni: YENI.into(),
            eski: ESKI.into(),
            cikti: CIKTI.into(),
            detay: DETAY.into(),
            grup: Some(PANEL_GRUBU.into()),
            rapor_ham: RAPOR_HAM.into(),
            rapor_cikti: RAPOR.into(),
            rapor_gruplari: RAPOR_GRUPLARI.into(),
        }
    }
}

/// `[a-z0-9][a-z0-9._-]{0,31}` (tam eşleşme).
pub fn arsiv_adi_mi(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 32 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

fn gercek_klasor(yol: &Path) -> bool {
    std::fs::symlink_metadata(yol).map(|m| m.is_dir()).unwrap_or(false)
}

/// Python `durum.arsivler` (yalnızca `kaynaklar`): yeni arşiv (cihaz yedekleri) önce; aynı ad eski arşivde de varsa ikinci
/// kaynak olur. Sıra: ekleme sırası.
/// TODO(Task 12): `durum::arsivler` gelince oradan alınır (tur/pub rapor adımında gerekir).
pub fn arsiv_kaynaklari(yeni: &Path, eski: &Path) -> Vec<(String, Vec<PathBuf>)> {
    let mut out: Vec<(String, Vec<PathBuf>)> = Vec::new();
    let adlar = |kok: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(kok).map(|d| d.filter_map(|x| x.ok().and_then(|x| x.file_name().into_string().ok())).collect()).unwrap_or_default();
        v.sort();
        v
    };
    if yeni.is_dir() {
        for ad in adlar(yeni) {
            let d = yeni.join(&ad).join("veri");
            if arsiv_adi_mi(&ad) && !sembolik_mi(&yeni.join(&ad)) && d.is_dir() {
                out.push((ad, vec![d]));
            }
        }
    }
    if eski.is_dir() {
        for ad in adlar(eski) {
            let d = eski.join(&ad);
            if arsiv_adi_mi(&ad) && !sembolik_mi(&d) && gercek_klasor(&d) {
                match out.iter_mut().find(|(a, _)| *a == ad) {
                    Some((_, k)) => k.push(d),
                    None => out.push((ad, vec![d])),
                }
            }
        }
    }
    out
}

/// Arşiv işleme adımı: `(arşiv adı, hata iletisi)` listesi (Python `main`'in `hatalar`ı; yeni arşiv önce).
pub fn arsiv_hatalari(y: &Yollar, arsivler: &[(String, Vec<PathBuf>)]) -> Vec<(String, String)> {
    let mut tum = Vec::new();
    for (ad, kaynaklar) in arsivler {
        let mut hatalar: Vec<String> = Vec::new();
        for kaynak in kaynaklar {
            // bir arşivin bozuk klasörü diğerlerini durdurmasın; Python: `hatalar += guncelle(...)` sonraki adım
            // patlasa da korunur
            let mut sonuc = || -> Result<(), String> {
                hatalar.extend(guncelle(kaynak, &y.cikti.join(format!("{ad}.json")))?);
                hatalar.extend(crate::ozet::detay_guncelle(kaynak, &y.detay.join(ad), y.grup.as_deref())?);
                Ok(())
            };
            if let Err(h) = sonuc() {
                hatalar.push(h);
            }
        }
        tum.extend(hatalar.into_iter().map(|h| (ad.clone(), h)));
    }
    tum
}

/// Python `main`: her arşivin kaynaklarını işler (yeni arşiv önce), hataları stderr'e yazar. Çıkış kodu: 0, hata varsa 1.
pub fn calistir(y: &Yollar, simdi: Option<f64>) -> i32 {
    let mut kod = 0;
    let arsivler = arsiv_kaynaklari(&y.yeni, &y.eski);
    for (ad, hata) in arsiv_hatalari(y, &arsivler) {
        eprintln!("{ad}: {hata}");
        kod = 1;
    }
    // haftalık/aylık raporlar (ham özet önbelleği + müşteri başına dönem raporları); hatası yukarıdakileri etkilemez
    let harita: crate::rapor::Arsivler = arsivler.into_iter().collect();
    match crate::rapor::calistir(&harita, &y.rapor_gruplari, &y.rapor_ham, &y.rapor_cikti, y.grup.as_deref(), simdi, None) {
        Ok(hatalar) => {
            for hata in hatalar {
                eprintln!("rapor: {hata}");
                kod = 1;
            }
        }
        Err(tur) => {
            eprintln!("rapor: {tur}");
            kod = 1;
        }
    }
    kod
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::altin::Kayit;
    use crate::test_yardim::gecici_dizin;
    use std::fs;
    use std::io::Write;

    const BASLIK: &str = "zaman;olay;telefon;ad;soyad;mac;ic_ip;protokol;ic_port;hedef_ip;hedef_port;nat_ip;nat_port;alan_adi;gonderilen_bayt;alinan_bayt;sure_sn;oturum_id;ek";

    /// Python `gzip.open(yol, "wt", ...)`: BOM + başlık + satırlar.
    fn gz_yaz(yol: &Path, metin: &str) {
        let mut e = flate2::write::GzEncoder::new(fs::File::create(yol).unwrap(), flate2::Compression::default());
        e.write_all(metin.as_bytes()).unwrap();
        e.finish().unwrap();
    }

    /// Python istatistik testlerinin `gun_yaz`'ı.
    fn gun_yaz(kok: &Path, gun: &str, telefonlar: &[&str], manifest: Option<&[&str]>, bozuk: bool) -> PathBuf {
        let d = kok.join("gunluk").join(gun);
        fs::create_dir_all(&d).unwrap();
        let yol = d.join("trafik.csv.gz");
        if bozuk {
            fs::write(&yol, b"\x1f\x8b\x08\x00bozuk-veri").unwrap();
        } else {
            let mut m = format!("\u{feff}{BASLIK}\n");
            for t in telefonlar {
                m.push_str(&format!("{gun}T10:00:00+03:00;BAGLANTI_BASLA;{t}{}\n", ";".repeat(16)));
            }
            gz_yaz(&yol, &m);
        }
        if let Some(man) = manifest {
            let mut m = format!("# gun={gun} onceki_zincir=0\n");
            for p in man {
                m.push_str(&format!("abc123  {p}\n"));
            }
            fs::write(d.join("MANIFEST.sha256"), m).unwrap();
        }
        d
    }

    fn oku(yol: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(yol).unwrap()).unwrap()
    }

    fn bos() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn farkli_kisi_bos_telefon_sayilmaz() {
        let _k = Kayit::bas("test_wc_istatistik", "test_farkli_kisi_bos_telefon_sayilmaz");
        let tmp = gecici_dizin();
        let d = gun_yaz(&tmp, "2026-09-27", &["5551112233", "5551112233", "5559998877", ""], Some(&["trafik.csv.gz"]), false);
        assert_eq!(farkli_kisi(&d).unwrap(), 2);
    }

    #[test]
    fn trafiksiz_tamam_gun_sifirdir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_trafiksiz_tamam_gun_sifirdir");
        let tmp = gecici_dizin();
        let d = gun_yaz(&tmp, "2026-09-27", &[], Some(&[]), false);
        fs::remove_file(d.join("trafik.csv.gz")).unwrap();
        assert!(tamam_mi(&d));
        assert_eq!(farkli_kisi(&d).unwrap(), 0);
    }

    #[test]
    fn guncelle_artimli() {
        let _k = Kayit::bas("test_wc_istatistik", "test_guncelle_artimli");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("ist").join("bocafe.json"));
        gun_yaz(&kafe, "2026-09-26", &["5551112233"], Some(&["trafik.csv.gz"]), false);
        assert_eq!(guncelle(&kafe, &js).unwrap(), bos());
        assert_eq!(oku(&js), serde_json::json!({"2026-09-26": 1}));
        fs::write(&js, r#"{"2026-09-26": 99}"#).unwrap(); // hesaplanmış gün yeniden hesaplanmaz
        gun_yaz(&kafe, "2026-09-27", &["5551112233", "5559998877"], Some(&["trafik.csv.gz"]), false);
        assert_eq!(guncelle(&kafe, &js).unwrap(), bos());
        assert_eq!(oku(&js), serde_json::json!({"2026-09-26": 99, "2026-09-27": 2}));
    }

    #[test]
    fn yarim_gun_atlanir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_yarim_gun_atlanir");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("bocafe.json"));
        let d = gun_yaz(&kafe, "2026-09-27", &["5551112233"], Some(&["trafik.csv.gz", "kullanicilar/5551112233.csv.gz"]), false);
        gun_yaz(&kafe, "2026-09-28", &["5551112233"], None, false); // MANIFEST henüz gelmemiş
        assert!(!tamam_mi(&d));
        assert_eq!(guncelle(&kafe, &js).unwrap(), bos());
        assert_eq!(oku(&js), serde_json::json!({}));
        fs::create_dir_all(d.join("kullanicilar")).unwrap();
        fs::File::create(d.join("kullanicilar").join("5551112233.csv.gz")).unwrap();
        guncelle(&kafe, &js).unwrap();
        assert_eq!(oku(&js), serde_json::json!({"2026-09-27": 1}));
    }

    #[test]
    fn bozuk_gz_gunu_atlar() {
        let _k = Kayit::bas("test_wc_istatistik", "test_bozuk_gz_gunu_atlar");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("bocafe.json"));
        gun_yaz(&kafe, "2026-09-26", &["5551112233"], Some(&["trafik.csv.gz"]), true);
        gun_yaz(&kafe, "2026-09-27", &["5551112233"], Some(&["trafik.csv.gz"]), false);
        let hatalar = guncelle(&kafe, &js).unwrap();
        assert!(hatalar.len() == 1 && hatalar[0].starts_with("2026-09-26"), "{hatalar:?}");
        assert_eq!(oku(&js), serde_json::json!({"2026-09-27": 1}));
    }

    #[test]
    fn bozuk_json_yeniden_hesaplanir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_bozuk_json_yeniden_hesaplanir");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("bocafe.json"));
        gun_yaz(&kafe, "2026-09-27", &["5551112233"], Some(&["trafik.csv.gz"]), false);
        fs::write(&js, r#"{"2026-09-27": "#).unwrap();
        assert_eq!(guncelle(&kafe, &js).unwrap(), bos());
        assert_eq!(oku(&js), serde_json::json!({"2026-09-27": 1}));
    }

    #[test]
    fn gun_olmayan_klasorler_yok_sayilir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_gun_olmayan_klasorler_yok_sayilir");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("bocafe.json"));
        gun_yaz(&kafe, "2026-09-27", &["5551112233"], Some(&["trafik.csv.gz"]), false);
        fs::create_dir_all(kafe.join("gunluk").join("eski")).unwrap();
        assert_eq!(guncelle(&kafe, &js).unwrap(), bos());
        assert_eq!(oku(&js), serde_json::json!({"2026-09-27": 1}));
        assert_eq!(guncelle(&tmp.join("olmayan"), &tmp.join("olmayan.json")).unwrap(), bos());
        assert_eq!(oku(&tmp.join("olmayan.json")), serde_json::json!({}));
    }

    #[test]
    fn sembolik_bag_gun_ve_dosya_atlanir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_sembolik_bag_gun_ve_dosya_atlanir");
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("bocafe"), tmp.join("bocafe.json"));
        let baska = gun_yaz(&tmp.join("baska"), "2026-09-26", &["5551112233", "5559998877"], Some(&["trafik.csv.gz"]), false);
        gun_yaz(&kafe, "2026-09-27", &["5551112233"], Some(&["trafik.csv.gz"]), false);
        std::os::unix::fs::symlink(&baska, kafe.join("gunluk").join("2026-09-26")).unwrap();
        let d = gun_yaz(&kafe, "2026-09-28", &[], Some(&[]), false);
        fs::remove_file(d.join("trafik.csv.gz")).unwrap();
        std::os::unix::fs::symlink(baska.join("trafik.csv.gz"), d.join("trafik.csv.gz")).unwrap();
        let hatalar = guncelle(&kafe, &js).unwrap();
        assert_eq!(oku(&js), serde_json::json!({"2026-09-27": 1}));
        assert!(hatalar.len() == 1 && hatalar[0].starts_with("2026-09-28"), "{hatalar:?}");
    }

    /// `test_gunluk_sembolik_bag_ise_arsiv_okunmaz` I.main çağırmaz (yalnızca guncelle + ozet.detay_guncelle): burada.
    #[test]
    fn gunluk_sembolik_bag_ise_arsiv_okunmaz() {
        let _k = Kayit::bas("test_wc_istatistik", "test_gunluk_sembolik_bag_ise_arsiv_okunmaz");
        let tmp = gecici_dizin();
        let baska = tmp.join("baska");
        gun_yaz(&baska, "2026-10-01", &["5334553132"], Some(&["trafik.csv.gz"]), false);
        let kok = tmp.join("kotu");
        fs::create_dir_all(&kok).unwrap();
        std::os::unix::fs::symlink(baska.join("gunluk"), kok.join("gunluk")).unwrap();
        let (js, dd) = (tmp.join("k.json"), tmp.join("dd"));
        guncelle(&kok, &js).unwrap();
        assert_eq!(oku(&js), serde_json::json!({}));
        crate::ozet::detay_guncelle(&kok, &dd, None).unwrap();
        let kalan: Vec<String> = fs::read_dir(&dd).unwrap().map(|d| d.unwrap().file_name().into_string().unwrap()).filter(|a| a != "kisiler.json").collect();
        assert!(kalan.is_empty(), "{kalan:?}");
    }

    #[test]
    fn manifest_cozumleme_kenarlari() {
        let tmp = gecici_dizin();
        let d = tmp.join("gun");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.gz"), b"x").unwrap();
        let dene = |metin: &str| {
            fs::write(d.join("MANIFEST.sha256"), metin).unwrap();
            tamam_mi(&d)
        };
        assert!(dene("# baslik\r\nabc  a.gz\r\n\r\n"));
        assert!(dene("abc\t a.gz  \n"));
        assert!(!dene("abc  yok.gz\n"));
        assert!(!dene("tekkelime\n")); // iki parça yok
        assert!(dene("# yalniz yorum\n"));
        fs::write(d.join("MANIFEST.sha256"), b"\xff\xfe").unwrap();
        assert!(!tamam_mi(&d)); // UTF-8 değil
        fs::remove_file(d.join("MANIFEST.sha256")).unwrap();
        std::os::unix::fs::symlink(d.join("a.gz"), d.join("MANIFEST.sha256")).unwrap();
        assert!(!tamam_mi(&d)); // sembolik bağ
    }

    fn rapor_yollari(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
        (tmp.join("rapor-ham"), tmp.join("rapor"), tmp.join("yok-gruplar.json"))
    }

    fn yollar(tmp: &Path, kayit: PathBuf, yeni: PathBuf, eski: PathBuf, ham_rapor_gruplar: (PathBuf, PathBuf, PathBuf)) -> Yollar {
        Yollar {
            kayit,
            yeni,
            eski,
            cikti: tmp.join("ist"),
            detay: tmp.join("detay"),
            grup: None,
            rapor_ham: ham_rapor_gruplar.0,
            rapor_cikti: ham_rapor_gruplar.1,
            rapor_gruplari: ham_rapor_gruplar.2,
        }
    }

    #[test]
    fn main_arsivleri_isler_yeni_once() {
        let _k = Kayit::bas("test_wc_istatistik", "test_main_arsivleri_isler_yeni_once");
        let tmp = gecici_dizin();
        let (yeni, eski, kayit) = (tmp.join("yeni"), tmp.join("eski"), tmp.join("kayit.csv"));
        fs::write(&kayit, format!("cihaz;bocafe;10.99.0.10;{}=;wfc-bocafe
", "A".repeat(43))).unwrap();
        gun_yaz(&yeni.join("bocafe").join("veri"), "2026-10-01", &["5334553132"], Some(&["trafik.csv.gz"]), false);
        gun_yaz(&eski.join("bocafe"), "2026-09-20", &["5334553132", "5551112233"], Some(&["trafik.csv.gz"]), false);
        gun_yaz(&eski.join("bocafe"), "2026-10-01", &["5550000000", "5551111111", "5552222222"], Some(&["trafik.csv.gz"]), false);
        let y = yollar(&tmp, kayit, yeni, eski, rapor_yollari(&tmp));
        assert_eq!(calistir(&y, None), 0);
        assert_eq!(oku(&y.cikti.join("bocafe.json")), serde_json::json!({"2026-09-20": 2, "2026-10-01": 1}));
    }

    #[test]
    fn main_raporlari_uretir() {
        let _k = Kayit::bas("test_wc_istatistik", "test_main_raporlari_uretir");
        let tmp = gecici_dizin();
        let (yeni, eski, kayit) = (tmp.join("yeni"), tmp.join("eski"), tmp.join("kayit.csv"));
        fs::write(&kayit, "").unwrap();
        gun_yaz(&yeni.join("1234567").join("veri"), "2026-10-01", &["905334553132"], Some(&["trafik.csv.gz"]), false);
        gun_yaz(&eski.join("1234567"), "2026-09-30", &["905551112233"], Some(&["trafik.csv.gz"]), false); // aynı adın eski klasörü
        gun_yaz(&eski.join("bocafe"), "2026-09-29", &["905550000000"], Some(&["trafik.csv.gz"]), false);
        let (ham, rapor, gd) = rapor_yollari(&tmp);
        fs::write(&gd, r#"{"1234567": ["bocafe"]}"#).unwrap();
        let y = yollar(&tmp, kayit, yeni, eski, (ham.clone(), rapor.clone(), gd));
        let simdi = 1_791_000_000.0; // 2026-10-03
        assert_eq!(calistir(&y, Some(simdi)), 0);
        let mut ham_gunleri: Vec<String> = fs::read_dir(ham.join("1234567")).unwrap().map(|d| d.unwrap().file_name().into_string().unwrap()).collect();
        ham_gunleri.sort();
        assert_eq!(ham_gunleri, vec!["2026-09-30.json", "2026-10-01.json"]);
        let rapor_klasorleri: Vec<String> = fs::read_dir(&rapor).unwrap().map(|d| d.unwrap().file_name().into_string().unwrap()).collect();
        assert_eq!(rapor_klasorleri, vec!["1234567"]);
        let w40 = oku(&rapor.join("1234567").join("hafta-2026-W40.json"));
        assert!(w40["misafir"]["farkli"] == 3);
        let g = w40["gunler"].as_array().unwrap();
        assert_eq!([g[1]["farkli"].clone(), g[2]["farkli"].clone(), g[3]["farkli"].clone()], [1, 1, 1].map(Value::from));
        assert_eq!(g[1]["durum"], "dogrulanamadi"); // zincir.txt yok: "tam" değil
        assert!(ham.join(".sir").exists() && w40["bicim"] == crate::rapor::BICIM);
    }

    #[test]
    fn main_bozuk_arsiv_digerlerini_durdurmaz() {
        let _k = Kayit::bas("test_wc_istatistik", "test_main_bozuk_arsiv_digerlerini_durdurmaz");
        let tmp = gecici_dizin();
        let (yeni, eski, kayit) = (tmp.join("yeni"), tmp.join("eski"), tmp.join("kayit.csv"));
        fs::write(&kayit, "").unwrap();
        fs::create_dir_all(eski.join("a-kafe")).unwrap();
        fs::write(eski.join("a-kafe").join("gunluk"), "x").unwrap(); // klasör olması gereken yerde dosya
        gun_yaz(&eski.join("b-kafe"), "2026-10-01", &["5334553132"], Some(&["trafik.csv.gz"]), false);
        let y = yollar(&tmp, kayit, yeni, eski, rapor_yollari(&tmp));
        calistir(&y, None);
        assert_eq!(oku(&y.cikti.join("b-kafe.json")), serde_json::json!({"2026-10-01": 1}));
    }

    #[test]
    fn arsiv_kaynaklari_sirasi() {
        let tmp = gecici_dizin();
        let (yeni, eski) = (tmp.join("yeni"), tmp.join("eski"));
        for a in ["bocafe", "1234567", "Buyuk"] {
            fs::create_dir_all(yeni.join(a).join("veri")).unwrap();
        }
        fs::create_dir_all(yeni.join("yok-veri")).unwrap();
        for a in ["bocafe", "eski-kafe"] {
            fs::create_dir_all(eski.join(a)).unwrap();
        }
        let r = arsiv_kaynaklari(&yeni, &eski);
        let adlar: Vec<&str> = r.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(adlar, vec!["1234567", "bocafe", "eski-kafe"]);
        assert_eq!(r[1].1, vec![yeni.join("bocafe").join("veri"), eski.join("bocafe")]);
    }
}

#[cfg(test)]
mod duzeltme_testleri {
    use super::*;
    use crate::test_yardim::gecici_dizin;
    use std::fs;
    use std::io::Write;

    fn gz(yol: &Path, metin: &str) {
        fs::create_dir_all(yol.parent().unwrap()).unwrap();
        let mut e = flate2::write::GzEncoder::new(fs::File::create(yol).unwrap(), flate2::Compression::fast());
        e.write_all(metin.as_bytes()).unwrap();
        e.finish().unwrap();
    }

    fn gun(kok: &Path, ad: &str, trafik: &str) -> PathBuf {
        let d = kok.join("gunluk").join(ad);
        gz(&d.join("trafik.csv.gz"), trafik);
        fs::write(d.join("MANIFEST.sha256"), "abc  trafik.csv.gz\n").unwrap();
        d
    }

    #[test]
    fn alan_siniri_asan_gun_hata_diger_gunler_surer() {
        let tmp = gecici_dizin();
        let (kafe, js) = (tmp.join("k"), tmp.join("k.json"));
        gun(&kafe, "2026-09-26", &format!("telefon;ad\n5551112233;{}\n", "x".repeat(ortak::ALAN_SINIRI + 1)));
        gun(&kafe, "2026-09-27", "telefon;ad\n5551112233;a\n");
        let hatalar = guncelle(&kafe, &js).unwrap();
        // Python: csv.Error `Exception`, o gün düşer ve iletisi kaydedilir; sonraki gün hesaplanır
        assert!(hatalar.len() == 1 && hatalar[0].starts_with("2026-09-26") && hatalar[0].contains("field larger"), "{hatalar:?}");
        assert_eq!(fs::read_to_string(&js).unwrap().contains("2026-09-27"), true);
        assert!(!fs::read_to_string(&js).unwrap().contains("2026-09-26"));
        // sınırdaki alan geçer
        let d = gun(&kafe, "2026-09-28", &format!("telefon;ad\n5551112233;{}\n", "x".repeat(ortak::ALAN_SINIRI)));
        assert_eq!(farkli_kisi(&d).unwrap(), 1);
    }

    #[test]
    fn guncelle_hatalari_detay_basarisiz_olsa_da_korunur() {
        let tmp = gecici_dizin();
        let (yeni, eski) = (tmp.join("yeni"), tmp.join("eski"));
        let kafe = yeni.join("ar").join("veri");
        gun(&kafe, "2026-09-26", "telefon\n5551112233\n");
        // bozuk gün: guncelle hata listesine yazar
        fs::write(kafe.join("gunluk/2026-09-26/trafik.csv.gz"), b"\x1f\x8b\x08\x00bozuk").unwrap();
        // detay hedefi dosya: detay_guncelle klasör açamayıp Err verir
        fs::create_dir_all(tmp.join("detay")).unwrap();
        fs::write(tmp.join("detay/ar"), "x").unwrap();
        let y = Yollar {
            kayit: tmp.join("kayit.csv"),
            yeni,
            eski,
            cikti: tmp.join("ist"),
            detay: tmp.join("detay"),
            grup: None,
            rapor_ham: tmp.join("ham"),
            rapor_cikti: tmp.join("rapor"),
            rapor_gruplari: tmp.join("g.json"),
        };
        let h = arsiv_hatalari(&y, &arsiv_kaynaklari(&y.yeni, &y.eski));
        assert_eq!(h.len(), 2, "{h:?}");
        assert!(h[0].1.starts_with("2026-09-26") && h[0].0 == "ar", "{h:?}");
    }
}
