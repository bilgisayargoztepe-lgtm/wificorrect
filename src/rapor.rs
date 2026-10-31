//! Raporlar (cihaz paneli → Ayarlar → Raporlar, 0.4.28): gün başına kişi başına kullanılan veri ve bağlı kalma süresi.
//!
//! - **Veri:** 5651 trafik kaydında kapanan bağlantıların baytları (gönderilen + alınan). Özetteki "Bugün kullanılan veri"
//!   (kaydedicinin nft sayaçları) ile aynı trafiği sayar; açık bağlantılar kapanınca eklenir.
//! - **Bağlı kalma:** kaydedici 30 sn'lik bakım turunda oturumu süren ve köprüde görülen cihazların sahibine geçen süreyi ekler
//!   (`<state>/varlik_gun/<gün>.json`; özetteki "N dk" ile aynı kaynak, bir kişinin iki cihazı aynı anda bağlıysa süre bir kez).
//!   Bu kayıt 0.4.28'den önceki günlerde yok, güncellemenin yapıldığı gün eksik: 5651 kaydındaki etkinlikten (yeni bağlantı ve DNS
//!   sorgusu; 30 dk'dan kısa aralar birleşir) de tahmin edilir, ikisinden büyüğü alınır.
//! - Kişi anahtarı 5651 `telefon` sütunu (telefon ya da `TC:…`/`PASAPORT:…`); kişisiz satır (izinli cihaz) `mac:<MAC>`.
//! - Mühürlü günün özeti bir kez hesaplanıp `<state>/rapor_ozet/<gün>.json`'a yazılır (gün-kapat mühürledikten sonra da yazar);
//!   açık gün (bugün) her istekte hesaplanır (panel kısa süre bellekte tutar). Temizlik ve fabrika dönüşü ikisini de siler.

use crate::ayar::Config;
use crate::{kayit, ortak};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const VARLIK: &str = "varlik_gun";
const OZET: &str = "rapor_ozet";
/// Önbellek biçimi: hesap değişirse artar, eski özetler yeniden hesaplanır
const SURUM: u32 = 1;
/// Tahminde birleşen en uzun etkinliksiz ara (sn)
const TAHMIN_ARA_SN: f64 = 1800.0;
const GUN_SN: u64 = 86_400;

/// Kişinin bir gündeki ya da dönemdeki kullanımı
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Kisi {
    pub bayt: u64,
    pub sn: u64,
}

/// Kişi anahtarı → kullanım
pub type GunOzeti = BTreeMap<String, Kisi>;

#[derive(Serialize, Deserialize, Default)]
struct Varlik {
    kisiler: BTreeMap<String, f64>,
}

#[derive(Serialize, Deserialize)]
struct Onbellek {
    surum: u32,
    kisiler: GunOzeti,
}

fn yol(state_root: &str, dizin: &str, gun: &str) -> PathBuf {
    Path::new(state_root).join(dizin).join(format!("{gun}.json"))
}

/// Bağlı kalma kaydı: `kisiler`'in her birine `sn` ekler (kaydedici, 30 sn'de bir).
pub fn varlik_ekle(state_root: &str, gun: &str, kisiler: &BTreeSet<String>, sn: f64) -> std::io::Result<()> {
    if kisiler.is_empty() || !(sn > 0.0) || !kayit::valid_day(gun) {
        return Ok(());
    }
    let p = yol(state_root, VARLIK, gun);
    let mut v: Varlik = fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for k in kisiler {
        *v.kisiler.entry(k.clone()).or_default() += sn;
    }
    fs::create_dir_all(p.parent().unwrap_or(Path::new(state_root)))?;
    ortak::write_atomic(&p, &serde_json::to_vec(&v).unwrap_or_default())
}

fn varlik_oku(state_root: &str, gun: &str) -> BTreeMap<String, f64> {
    fs::read(yol(state_root, VARLIK, gun)).ok().and_then(|b| serde_json::from_slice::<Varlik>(&b).ok()).map(|v| v.kisiler).unwrap_or_default()
}

/// Etkinlik zamanlarından bağlı kalma tahmini: aralar 30 dk'dan kısa olan olaylar bir ziyaret; ziyaret = ilk → son olay + 1 dk.
fn tahmin(zamanlar: &mut [f64]) -> u64 {
    zamanlar.sort_by(f64::total_cmp);
    let Some((&ilk, kalan)) = zamanlar.split_first() else { return 0 };
    let (mut bas, mut son, mut toplam) = (ilk, ilk, 0.0);
    for &z in kalan {
        if z - son > TAHMIN_ARA_SN {
            toplam += son - bas + 60.0;
            bas = z;
        }
        son = z;
    }
    toplam += son - bas + 60.0;
    (toplam as u64).min(GUN_SN)
}

/// Günün kişi başına kullanımı, kayıtlardan (önbelleksiz).
pub fn gun_hesapla(log_root: &str, state_root: &str, gun: &str) -> GunOzeti {
    let mut bayt: HashMap<String, u64> = HashMap::new();
    let mut olaylar: HashMap<String, Vec<f64>> = HashMap::new();
    for ad in ["trafik.csv", "dns.csv"] {
        let p = ortak::day_file(log_root, gun, ad);
        // mühürlü .gz + (varsa) mühürden sonra düşmüş .csv
        for metin in [kayit::read_text(&PathBuf::from(format!("{}.gz", p.display()))), kayit::read_text(&p)] {
            for satir in metin.lines() {
                let mut f = satir.split(';');
                let (Some(zaman), Some(olay), Some(tel)) = (f.next(), f.next(), f.next()) else { continue };
                let mac = f.nth(2).unwrap_or("");
                match olay {
                    "BAGLANTI_BITIS" => {
                        let n = |s: Option<&str>| s.and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                        let (gon, al) = (n(f.nth(8)), n(f.next()));
                        let k = if !tel.is_empty() { tel.to_string() } else if !mac.is_empty() { format!("mac:{mac}") } else { continue };
                        let b = bayt.entry(k).or_default();
                        *b = b.saturating_add(gon.saturating_add(al));
                    }
                    "BAGLANTI_BASLA" | "DNS" if !tel.is_empty() => {
                        if let Some(t) = kayit::parse_time(zaman) {
                            olaylar.entry(tel.to_string()).or_default().push(t);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let kayitli = varlik_oku(state_root, gun);
    let mut out = GunOzeti::new();
    for (k, b) in bayt {
        out.entry(k).or_default().bayt = b;
    }
    for (k, mut z) in olaylar {
        let sn = tahmin(&mut z);
        let e = out.entry(k).or_default();
        e.sn = e.sn.max(sn);
    }
    for (k, sn) in kayitli {
        let e = out.entry(k).or_default();
        e.sn = e.sn.max((sn.max(0.0).round() as u64).min(GUN_SN));
    }
    out.retain(|_, k| k.bayt > 0 || k.sn > 0);
    out
}

fn muhurlu(log_root: &str, gun: &str) -> bool {
    Path::new(log_root).join("gunluk").join(gun).join("MANIFEST.sha256").exists()
}

fn onbellek_oku(state_root: &str, gun: &str) -> Option<GunOzeti> {
    let o: Onbellek = serde_json::from_slice(&fs::read(yol(state_root, OZET, gun)).ok()?).ok()?;
    (o.surum == SURUM).then_some(o.kisiler)
}

fn onbellek_yaz(state_root: &str, gun: &str, kisiler: &GunOzeti) {
    let p = yol(state_root, OZET, gun);
    let govde = serde_json::to_vec(&Onbellek { surum: SURUM, kisiler: kisiler.clone() }).unwrap_or_default();
    if let Err(e) = fs::create_dir_all(p.parent().unwrap_or(Path::new(state_root))).and_then(|_| ortak::write_atomic(&p, &govde)) {
        eprintln!("rapor: {} yazılamadı: {e}", p.display());
    }
}

/// Günün özeti ve kesin mi (mühürlü gün: değişmez, önbellekten). Açık gün her çağrıda kayıtlardan hesaplanır.
pub fn gun_ozeti(cfg: &Config, gun: &str) -> (GunOzeti, bool) {
    let m = &cfg.main;
    if !kayit::valid_day(gun) {
        return (GunOzeti::new(), true);
    }
    if !muhurlu(&m.log_root, gun) {
        return (gun_hesapla(&m.log_root, &m.state_root, gun), false);
    }
    if let Some(o) = onbellek_oku(&m.state_root, gun) {
        return (o, true);
    }
    let o = gun_hesapla(&m.log_root, &m.state_root, gun);
    onbellek_yaz(&m.state_root, gun, &o);
    (o, true)
}

/// Gün-kapat mühürledikten sonra: özet önceden yazılır (rapor ilk açıldığında hesap beklemesin).
pub fn onbellege_al(cfg: &Config, gun: &str) {
    if muhurlu(&cfg.main.log_root, gun) && onbellek_oku(&cfg.main.state_root, gun).is_none() {
        gun_ozeti(cfg, gun);
    }
}

/// Saklama süresini aşan günlerin bağlı kalma kayıtları ve özetleri silinir (5651 temizliğiyle; yedek beklenmez, kayıtlardan
/// türetilmiş veridir).
pub fn temizle(state_root: &str, sinir_gun: &str) {
    for dizin in [VARLIK, OZET] {
        let Ok(rd) = fs::read_dir(Path::new(state_root).join(dizin)) else { continue };
        for e in rd.flatten() {
            let ad = e.file_name().to_string_lossy().to_string();
            if ad.strip_suffix(".json").is_some_and(|g| kayit::valid_day(g) && g < sinir_gun) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

/// Fabrika dönüşü: önceki işletmenin rapor verisi silinir.
pub fn hepsini_sil(state_root: &str) {
    for dizin in [VARLIK, OZET] {
        let _ = fs::remove_dir_all(Path::new(state_root).join(dizin));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gecici() -> (PathBuf, String, String) {
        let kok = std::env::temp_dir().join(format!("wfc-rapor-{}", ortak::random_hex(6)));
        let (log, st) = (kok.join("5651"), kok.join("state"));
        fs::create_dir_all(&st).unwrap();
        (kok, log.display().to_string(), st.display().to_string())
    }

    fn satir(zaman: &str, olay: &str, tel: &str, mac: &str, gon: u64, al: u64) -> String {
        format!("{zaman};{olay};{tel};;;{mac};10.50.0.23;tcp;50000;1.2.3.4;443;192.168.1.110;50000;;{gon};{al};5;s;\r\n")
    }

    fn yaz(log: &str, gun: &str, ad: &str, satirlar: &[String]) {
        let p = ortak::day_file(log, gun, ad);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, format!("\u{feff}{}\r\n{}", ortak::CSV_HEADER.join(";"), satirlar.concat())).unwrap();
    }

    #[test]
    fn bayt_kapanan_baglantilardan_kisi_yoksa_mac() {
        let (_k, log, st) = gecici();
        let g = "2026-10-31";
        yaz(&log, g, "trafik.csv", &[
            satir("2026-10-31T10:00:00+03:00", "BAGLANTI_BASLA", "905551112233", "aa:bb:cc:dd:ee:01", 0, 0),
            satir("2026-10-31T10:05:00+03:00", "BAGLANTI_BITIS", "905551112233", "aa:bb:cc:dd:ee:01", 1000, 9000),
            satir("2026-10-31T10:06:00+03:00", "BAGLANTI_BITIS", "905551112233", "aa:bb:cc:dd:ee:02", 500, 500),
            satir("2026-10-31T11:00:00+03:00", "BAGLANTI_BITIS", "", "aa:bb:cc:dd:ee:09", 7, 3),
            "bozuk satır\r\n".into(),
        ]);
        let o = gun_hesapla(&log, &st, g);
        assert_eq!(o["905551112233"].bayt, 11_000); // iki cihaz bir kişi
        assert_eq!(o["mac:aa:bb:cc:dd:ee:09"], Kisi { bayt: 10, sn: 0 }); // izinli cihaz: süre tahmini yok
        assert_eq!(o.len(), 2);
    }

    #[test]
    fn sure_kayittan_ya_da_etkinlikten_buyugu() {
        let (_k, log, st) = gecici();
        let g = "2026-10-31";
        // etkinlik: 10:00-10:20 (DNS ve yeni bağlantı, aralar < 30 dk) → 21 dk; 12:00 tek olay → 1 dk
        yaz(&log, g, "trafik.csv", &[
            satir("2026-10-31T10:00:00+03:00", "BAGLANTI_BASLA", "905551112233", "m1", 0, 0),
            satir("2026-10-31T10:20:00+03:00", "BAGLANTI_BASLA", "905551112233", "m1", 0, 0),
            satir("2026-10-31T12:00:00+03:00", "BAGLANTI_BASLA", "905551112233", "m1", 0, 0),
        ]);
        yaz(&log, g, "dns.csv", &[satir("2026-10-31T10:10:00+03:00", "DNS", "905551112233", "m1", 0, 0)]);
        assert_eq!(gun_hesapla(&log, &st, g)["905551112233"].sn, 22 * 60);
        // kayıt daha uzunsa kayıt (telefon cebinde, ağda ama etkinliksiz)
        let k = BTreeSet::from(["905551112233".to_string(), "905559998877".to_string()]);
        varlik_ekle(&st, g, &k, 3600.0).unwrap();
        varlik_ekle(&st, g, &BTreeSet::from(["905551112233".to_string()]), 30.0).unwrap();
        let o = gun_hesapla(&log, &st, g);
        assert_eq!((o["905551112233"].sn, o["905559998877"].sn), (3630, 3600));
        // boş küme, sıfır süre, bozuk gün yazılmaz
        varlik_ekle(&st, g, &BTreeSet::new(), 30.0).unwrap();
        varlik_ekle(&st, g, &k, 0.0).unwrap();
        varlik_ekle(&st, "../x", &k, 30.0).unwrap();
        assert_eq!(gun_hesapla(&log, &st, g)["905551112233"].sn, 3630);
        assert!(!Path::new(&st).join("x.json").exists());
    }

    #[test]
    fn tahmin_ara_30_dakika() {
        let t = |dk: &[f64]| tahmin(&mut dk.iter().map(|d| d * 60.0).collect::<Vec<_>>());
        assert_eq!(t(&[]), 0);
        assert_eq!(t(&[5.0]), 60);
        assert_eq!(t(&[0.0, 29.0]), 30 * 60); // 29 dk ara birleşir
        assert_eq!(t(&[0.0, 31.0]), 2 * 60); // 31 dk ara: iki ayrı ziyaret
        assert_eq!(t(&[0.0, 2000.0, 1000.0]), 3 * 60); // sırasız
        assert_eq!(t(&(0..2000).map(|i| i as f64).collect::<Vec<_>>()), GUN_SN); // günden uzun olmaz
    }

    #[test]
    fn muhurlu_gun_onbellekten_acik_gun_her_seferinde() {
        let (k, log, st) = gecici();
        let mut cfg = Config::default();
        cfg.main.log_root = log.clone();
        cfg.main.state_root = st.clone();
        let g = "2026-10-30";
        yaz(&log, g, "trafik.csv", &[satir("2026-10-30T10:05:00+03:00", "BAGLANTI_BITIS", "905551112233", "m1", 100, 0)]);
        let (o, kesin) = gun_ozeti(&cfg, g);
        assert_eq!((o["905551112233"].bayt, kesin), (100, false));
        assert!(!yol(&st, OZET, g).exists(), "açık gün önbelleğe yazılmaz");
        // mühürlenince bir kez hesaplanır, sonra dosya değişse de önbellek
        fs::write(ortak::day_file(&log, g, "MANIFEST.sha256"), "# gun\n").unwrap();
        onbellege_al(&cfg, g);
        assert!(yol(&st, OZET, g).exists());
        yaz(&log, g, "trafik.csv", &[]);
        let (o, kesin) = gun_ozeti(&cfg, g);
        assert_eq!((o["905551112233"].bayt, kesin), (100, true));
        // eski biçimli önbellek yeniden hesaplanır
        fs::write(yol(&st, OZET, g), "{\"surum\":0,\"kisiler\":{}}").unwrap();
        assert!(gun_ozeti(&cfg, g).0.is_empty());
        // temizlik ve fabrika
        varlik_ekle(&st, g, &BTreeSet::from(["905551112233".to_string()]), 30.0).unwrap();
        varlik_ekle(&st, "2026-10-31", &BTreeSet::from(["905551112233".to_string()]), 30.0).unwrap();
        temizle(&st, "2026-10-31");
        assert!(!yol(&st, OZET, g).exists() && !yol(&st, VARLIK, g).exists() && yol(&st, VARLIK, "2026-10-31").exists());
        hepsini_sil(&st);
        assert!(!Path::new(&st).join(VARLIK).exists());
        let _ = fs::remove_dir_all(k);
    }
}
