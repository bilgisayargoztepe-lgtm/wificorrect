//! Hazır engel listeleri (spec 2026-10-14): merkez kategori başına temizlenmiş alan adı listesi üretir; sağlık raporu
//! cevabında (`listeler`) her kategorinin sürümü (düz metnin sha256'sı) gelir. Açık kategorinin sürümü değiştiyse
//! `ctl liste-guncelle` (ayrı systemd işi) listeyi `/api/liste`ten gzip olarak indirir, açılmış metnin sha256'sını
//! doğrular (tutmazsa eski dosya kalır) ve birleşik dnsmasq dosyasını yazar. dnsmasq bu dosyayı `servers-file` ile
//! okur; `server=/alan/` (adres yok) alan adı ve bütün alt adları için yerelde NXDOMAIN döndürür, SIGHUP ile yeniden
//! okunur (yeniden başlatma yok). Sorgular 5651 DNS kaydına bugünkü gibi düşer. Elle girilen siteler
//! `yasak-siteler.conf`'ta kalır (src/filtre.rs); işletmenin istisnaları (alt dize) hazır listeden de çıkarır.
//! Hata misafirin internetini kesmez: eski liste kalır.

use crate::ayar::Config;
use crate::merkez::{Indir, Merkez};
use crate::ortak::{self, Row, Runner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// (kimlik, ekranda görünen ad). Merkezdeki `listeler.KATEGORILER` ile aynı kimlikler.
pub const KATEGORILER: [(&str, &str); 3] = [("kumar", "Kumar ve bahis"), ("yetiskin", "Yetişkin içerik"), ("zararli", "Zararlı ve dolandırıcı siteler")];
/// dnsmasq `servers-file` (deploy/debian/etc/dnsmasq.d/wificorrect.conf); dosya yoksa dnsmasq açılmaz → hep yazılır.
pub const CONF: &str = "/etc/wificorrect/hazir-liste.conf";
pub const DIZIN: &str = "/var/lib/wificorrect/liste";
/// Açılmış liste en çok bu kadar bayt (merkez kategori başına 300.000 alan adıyla sınırlar, ~10 MB).
const EN_COK_BAYT: u64 = 64 << 20;

/// Bir kategorinin sürümü: düz metnin sha256'sı, alan adı sayısı, merkezde üretildiği an.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Meta {
    pub sha256: String,
    pub sayi: u64,
    pub zaman: String,
}

/// Cihazdaki yollar (testte geçici klasör).
pub struct Yollar {
    pub dizin: PathBuf,
    pub conf: PathBuf,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar { dizin: DIZIN.into(), conf: CONF.into() }
    }
    fn gz(&self, k: &str) -> PathBuf {
        self.dizin.join(format!("{k}.txt.gz"))
    }
    fn meta(&self, k: &str) -> PathBuf {
        self.dizin.join(format!("{k}.json"))
    }
    /// Son sağlık cevabındaki sürümler (panel yeni açılan kategoriyi indirtirken de kullanır).
    fn hedef(&self) -> PathBuf {
        self.dizin.join("hedef.json")
    }
    /// Son başarısız indirme (kategori → sürüm, zaman): aynı sürüm `HATA_BEKLE` boyunca yeniden denenmez.
    fn hata(&self) -> PathBuf {
        self.dizin.join("hata.json")
    }
    /// Dosya yazıldı ama dnsmasq yeniden okuyamadı: sonraki `uygula` içerik aynı olsa da yeniden okutur.
    fn hup_bekliyor(&self) -> PathBuf {
        self.dizin.join("hup-bekliyor")
    }
}

/// Başarısız indirmeden sonra aynı sürümün yeniden denenmesi için beklenen süre (sn).
const HATA_BEKLE: f64 = 3600.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Hata {
    sha256: String,
    zaman: f64,
}

fn hata_oku(y: &Yollar) -> BTreeMap<String, Hata> {
    std::fs::read(y.hata()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn hata_yaz(y: &Yollar, h: &BTreeMap<String, Hata>) {
    if h.is_empty() {
        let _ = std::fs::remove_file(y.hata());
    } else {
        let _ = ortak::write_atomic(&y.hata(), &serde_json::to_vec(h).unwrap_or_default());
    }
}

pub fn ad(k: &str) -> Option<&'static str> {
    KATEGORILER.iter().find(|(id, _)| *id == k).map(|(_, a)| *a)
}

/// Ayarda anahtar yoksa açık (varsayılan: üç kategori de açık).
pub fn acik(cfg: &Config, k: &str) -> bool {
    cfg.filtre.hazir.get(k).copied().unwrap_or(true)
}

pub fn acik_harita(cfg: &Config) -> BTreeMap<String, bool> {
    KATEGORILER.iter().map(|(k, _)| (k.to_string(), acik(cfg, k))).collect()
}

fn sha_gecerli(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn sha256(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// dnsmasq dosyasına yazılabilir alan adı: küçük harf, rakam, nokta, tire; en az iki etiket, etiket 1-63, toplam ≤ 253.
/// Son etiketi yalnızca rakam olan (IPv4 adresi; gerçek bir üst alan adı rakamdan oluşmaz) atılır.
fn alan_gecerli(d: &str) -> bool {
    d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| (1..=63).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
        && !d.rsplit('.').next().is_some_and(|t| t.bytes().all(|b| b.is_ascii_digit()))
}

pub fn meta_oku(y: &Yollar, k: &str) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(y.meta(k)).ok()?).ok()
}

/// Sağlık cevabındaki `listeler`: yalnızca bilinen kategoriler ve geçerli sha256.
pub fn hedef_coz(v: &Value) -> BTreeMap<String, Meta> {
    let Some(o) = v.as_object() else { return BTreeMap::new() };
    o.iter()
        .filter(|(k, _)| ad(k).is_some())
        .filter_map(|(k, m)| serde_json::from_value::<Meta>(m.clone()).ok().filter(|m| sha_gecerli(&m.sha256)).map(|m| (k.clone(), m)))
        .collect()
}

pub fn hedef_oku(y: &Yollar) -> BTreeMap<String, Meta> {
    std::fs::read(y.hedef()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// İndirilmesi gereken kategoriler: açık ve eldeki sürümü merkezinkinden farklı (ya da hiç yok); aynı sürüm son
/// `HATA_BEKLE` saniyede hata verdiyse (ör. sha tutmadı, merkez bozuk dosya veriyor) beklenir.
pub fn eksikler(y: &Yollar, hedef: &BTreeMap<String, Meta>, acik: &BTreeMap<String, bool>, now: f64) -> Vec<String> {
    let hata = hata_oku(y);
    hedef
        .iter()
        .filter(|(k, h)| acik.get(*k).copied().unwrap_or(true) && meta_oku(y, k).is_none_or(|m| m.sha256 != h.sha256))
        .filter(|(k, h)| !hata.get(*k).is_some_and(|e| e.sha256 == h.sha256 && (now - e.zaman).abs() < HATA_BEKLE))
        .map(|(k, _)| k.clone())
        .collect()
}

/// `ctl saglik`: merkezin sürümleri kaydedilir; açık bir kategori eksik ya da eskiyse `baslat` (indirme işi) çağrılır.
/// Dönen: iş başlatıldı mı.
pub fn hedefi_isle(y: &Yollar, hedef: &BTreeMap<String, Meta>, acik: &BTreeMap<String, bool>, now: f64, baslat: &dyn Fn()) -> bool {
    if hedef_oku(y) != *hedef {
        if let Ok(b) = serde_json::to_vec_pretty(hedef) {
            let _ = ortak::write_atomic(&y.hedef(), &b);
        }
    }
    let eksik = !eksikler(y, hedef, acik, now).is_empty();
    if eksik {
        baslat();
    }
    eksik
}

/// gzip'i açar (boyut sınırlı).
fn ac(gz: &Path) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(gz).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(f).take(EN_COK_BAYT + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > EN_COK_BAYT {
        return Err("çok büyük".into());
    }
    Ok(out)
}

fn denetim(log_root: &str, now: f64, olay: &str, ek: String) {
    ortak::audit(log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
}

/// Eksik açık kategorileri indirir; açılmış metnin sha256'sı merkezin bildirdiğiyle aynı değilse (merkez listeyi o
/// arada yenilemiş olabilir) dosya yazılmaz, eski liste kalır, sonraki raporda yeniden denenir. Dönen: indirilenler.
pub fn guncelle(y: &Yollar, log_root: &str, m: &Merkez, hedef: &BTreeMap<String, Meta>, acik: &BTreeMap<String, bool>, indir: &Indir, now: f64) -> Vec<String> {
    let mut alinan = vec![];
    if ortak::mkdirs(&y.dizin).is_err() {
        return alinan;
    }
    let mut hata = hata_oku(y);
    for k in eksikler(y, hedef, acik, now) {
        let h = &hedef[&k];
        let gecici = y.dizin.join(format!("{k}.indir"));
        let govde = json!({"cihaz_anahtari": m.cihaz_anahtari, "kategori": k}).to_string();
        let sonuc = match indir("/api/liste", &govde, &gecici) {
            Ok(200) => ac(&gecici).map_err(|_| "gzip".to_string()),
            Ok(d) => Err(format!("http_{d}")),
            Err(_) => Err("indirme".into()),
        }
        .and_then(|metin| if sha256(&metin) == h.sha256 { Ok(metin) } else { Err("sha".into()) })
        .and_then(|metin| {
            let sayi = metin.split(|b| *b == b'\n').filter(|l| !l.trim_ascii().is_empty()).count() as u64;
            std::fs::rename(&gecici, y.gz(&k)).map_err(|_| "yazma".to_string())?;
            let meta = Meta { sha256: h.sha256.clone(), sayi, zaman: h.zaman.clone() };
            ortak::write_atomic(&y.meta(&k), &serde_json::to_vec_pretty(&meta).unwrap_or_default()).map_err(|_| "yazma".to_string())?;
            Ok(sayi)
        });
        let _ = std::fs::remove_file(&gecici);
        match sonuc {
            Ok(sayi) => {
                denetim(log_root, now, "HAZIR_LISTE_GUNCEL", format!("kategori={k} surum={} sayi={sayi}", &h.sha256[..8]));
                hata.remove(&k);
                alinan.push(k);
            }
            Err(neden) => {
                denetim(log_root, now, "HAZIR_LISTE_HATA", format!("kategori={k} neden={neden}"));
                hata.insert(k, Hata { sha256: h.sha256.clone(), zaman: now });
            }
        }
    }
    hata_yaz(y, &hata);
    alinan
}

/// Bir kategorinin geçerli alan adları (dosya yoksa ya da bozuksa boş).
fn alanlar(y: &Yollar, k: &str) -> Vec<String> {
    ac(&y.gz(k))
        .map(|b| String::from_utf8_lossy(&b).lines().map(str::trim).filter(|d| alan_gecerli(d)).map(String::from).collect())
        .unwrap_or_default()
}

fn istisnada(cfg: &Config, d: &str) -> bool {
    cfg.filtre.istisnalar.iter().any(|i| d.contains(i.as_str()))
}

/// Birleşik dnsmasq dosyasının metni: açık kategoriler, istisna geçen alan adları çıkarılmış.
pub fn metin(cfg: &Config, y: &Yollar) -> (String, usize) {
    let mut set = BTreeSet::new();
    for (k, _) in KATEGORILER {
        if acik(cfg, k) {
            set.extend(alanlar(y, k).into_iter().filter(|d| !istisnada(cfg, d)));
        }
    }
    let mut s = String::from("# WifiCorrect üretir (panel → Yasaklı siteler → Hazır listeler). Alan adı ve alt adları NXDOMAIN döner.\n");
    for d in &set {
        s.push_str("server=/");
        s.push_str(d);
        s.push_str("/\n");
    }
    (s, set.len())
}

/// Birleşik dosyayı atomik yazar (her zaman var olur; dnsmasq dosya yoksa açılmaz); değiştiyse (ya da önceki yeniden
/// okutma başarısız kaldıysa) ve dnsmasq çalışıyorsa SIGHUP ile yeniden okutur (yalnızca ana süreç). dnsmasq çalışmıyorsa
/// açılırken dosyayı okur. Dönen: engellenen alan adı sayısı.
pub fn uygula(cfg: &Config, y: &Yollar, runner: &Runner) -> Result<usize, String> {
    let _kilit = ortak::state_lock(&y.dizin.to_string_lossy()).map_err(|e| format!("{}: {e}", y.dizin.display()))?;
    let (text, n) = metin(cfg, y);
    let changed = std::fs::read(&y.conf).ok().as_deref() != Some(text.as_bytes());
    if changed {
        ortak::write_atomic(&y.conf, text.as_bytes()).map_err(|e| format!("{} yazılamadı: {e}", y.conf.display()))?;
        // dnsmasq SIGHUP'ta dosyayı yetkisini bırakmış kullanıcıyla okur
        let _ = std::fs::set_permissions(&y.conf, std::os::unix::fs::PermissionsExt::from_mode(0o644));
    }
    if changed || y.hup_bekliyor().exists() {
        let c = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        if runner(&c(&["systemctl", "is-active", "--quiet", "dnsmasq.service"])) && !runner(&c(&["systemctl", "kill", "--kill-whom=main", "-s", "HUP", "dnsmasq.service"])) {
            let _ = std::fs::write(y.hup_bekliyor(), b"");
            return Err("dnsmasq listeyi yeniden okuyamadı".into());
        }
        let _ = std::fs::remove_file(y.hup_bekliyor());
    }
    Ok(n)
}

/// `ctl liste-uygula` / `liste-guncelle` (ayrı işler; panel ayarı kaydedip işi başlatır ve hemen döner): ayar dosyası
/// uygulamadan hemen önce yeniden okunur ve uygulama sürerken hazır liste ayarı ya da istisnalar değiştiyse yeniden
/// uygulanır (iş zaten sürerken gelen ikinci panel değişikliği kaybolmasın; aynı anda iki iş en sonda güncel ayarı yazar).
pub fn uygula_guncel(cfg_path: &str, yedek: &Config, y: &Yollar, runner: &Runner) -> Result<usize, String> {
    let oku = |onceki: &Config| Config::load(cfg_path).unwrap_or_else(|_| onceki.clone());
    let anahtar = |c: &Config| (c.filtre.hazir.clone(), c.filtre.istisnalar.clone());
    let mut c = oku(yedek);
    let mut r = uygula(&c, y, runner);
    for _ in 0..3 {
        let yeni = oku(&c);
        if anahtar(&yeni) == anahtar(&c) {
            break;
        }
        c = yeni;
        r = uygula(&c, y, runner);
    }
    r
}

/// Listede satır satır arar (bütün listeyi belleğe almadan), ilk eşleşmede durur.
fn listede_var(y: &Yollar, k: &str, bul: &dyn Fn(&str) -> bool) -> bool {
    use std::io::BufRead;
    let Ok(f) = std::fs::File::open(y.gz(k)) else { return false };
    std::io::BufReader::new(flate2::read::GzDecoder::new(f).take(EN_COK_BAYT)).split(b'\n').map_while(Result::ok).any(|l| {
        let d = String::from_utf8_lossy(&l);
        let d = d.trim();
        alan_gecerli(d) && bul(d)
    })
}

/// "Bu adres engelli mi?": alan adını (ya da üst adlarından birini) içeren açık kategorinin adı; istisnaya takılan
/// liste girdisi saymaz (dnsmasq dosyasında da yoktur).
pub fn bul(cfg: &Config, y: &Yollar, d: &str) -> Option<&'static str> {
    let ustler: BTreeSet<&str> = d.match_indices('.').map(|(i, _)| &d[i + 1..]).chain([d]).collect();
    KATEGORILER
        .iter()
        .filter(|(k, _)| acik(cfg, k))
        .find(|(k, _)| listede_var(y, k, &|e| ustler.contains(e) && !istisnada(cfg, e)))
        .map(|(_, a)| *a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn kur() -> (Yollar, PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-liste-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (Yollar { dizin: root.join("liste"), conf: root.join("hazir.conf") }, root)
    }

    fn gz(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    fn meta(metin: &str) -> Meta {
        Meta { sha256: sha256(metin.as_bytes()), sayi: metin.lines().count() as u64, zaman: "2026-10-14T03:00:00+03:00".into() }
    }

    /// Listeyi doğrudan cihaz dosyalarına koyar (indirilmiş gibi).
    fn koy(y: &Yollar, k: &str, metin: &str) {
        std::fs::create_dir_all(&y.dizin).unwrap();
        std::fs::write(y.gz(k), gz(metin.as_bytes())).unwrap();
        std::fs::write(y.meta(k), serde_json::to_vec(&meta(metin)).unwrap()).unwrap();
    }

    fn cfg(extra: &str) -> Config {
        Config::parse(&format!("[filtre]\n{extra}")).unwrap()
    }

    fn harita(c: &Config) -> BTreeMap<String, bool> {
        acik_harita(c)
    }

    #[test]
    fn varsayilan_acik() {
        let c = cfg("");
        assert!(KATEGORILER.iter().all(|(k, _)| acik(&c, k)));
        let c = cfg("[filtre.hazir]\nkumar = false\n");
        assert!(!acik(&c, "kumar") && acik(&c, "yetiskin") && acik(&c, "zararli"));
        // kayıt-okuma: anahtar korunur; boş harita dosyaya yazılmaz
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.filtre.hazir.get("kumar"), Some(&false));
        assert!(!toml::to_string_pretty(&cfg("")).unwrap().contains("hazir"));
    }

    #[test]
    fn hedef_cozumleme() {
        let s = sha256(b"x");
        let v = json!({"kumar": {"sha256": s, "sayi": 3, "zaman": "z"}, "vpn": {"sha256": s, "sayi": 1, "zaman": "z"},
                       "yetiskin": {"sha256": "kisa", "sayi": 1, "zaman": "z"}, "zararli": {"sayi": 1}});
        let h = hedef_coz(&v);
        assert_eq!(h.keys().collect::<Vec<_>>(), vec!["kumar"]);
        assert_eq!(h["kumar"].sayi, 3);
        assert!(hedef_coz(&json!(null)).is_empty());
    }

    #[test]
    fn surum_degisince_is_baslar_ayniysa_baslamaz() {
        let (y, root) = kur();
        let c = cfg("");
        let sayac = std::cell::Cell::new(0);
        let b = || sayac.set(sayac.get() + 1);
        let hedef: BTreeMap<String, Meta> = [("kumar".to_string(), meta("bet365.com\n"))].into();
        assert!(hedefi_isle(&y, &hedef, &harita(&c), T, &b)); // hiç yok → indir
        assert_eq!(sayac.get(), 1);
        assert_eq!(hedef_oku(&y), hedef);
        koy(&y, "kumar", "bet365.com\n");
        assert!(!hedefi_isle(&y, &hedef, &harita(&c), T, &b)); // aynı sürüm → başlamaz
        assert_eq!(sayac.get(), 1);
        let yeni: BTreeMap<String, Meta> = [("kumar".to_string(), meta("bet365.com\nbets10.com\n"))].into();
        assert!(hedefi_isle(&y, &yeni, &harita(&c), T, &b));
        assert_eq!(sayac.get(), 2);
        // kapalı kategori indirilmez
        let kapali = cfg("[filtre.hazir]\nkumar = false\n");
        assert!(!hedefi_isle(&y, &yeni, &harita(&kapali), T, &b));
        assert_eq!(sayac.get(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn indirme_sha_dogrulanir_tutmazsa_eski_kalir() {
        let (y, root) = kur();
        let log = root.join("5651").display().to_string();
        let c = cfg("");
        let m = Merkez { cihaz_anahtari: "anahtar".into(), ..Default::default() };
        let eski = "eski.com\n";
        koy(&y, "kumar", eski);
        let yeni = "bet365.com\nbets10.com\n";
        let hedef: BTreeMap<String, Meta> = [("kumar".to_string(), meta(yeni))].into();
        let istekler: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let gonderilen = Arc::new(Mutex::new(gz(b"baska.com\n"))); // merkez o arada yenilemiş: sha tutmaz
        let (i2, g2) = (istekler.clone(), gonderilen.clone());
        let indir = move |yol: &str, govde: &str, cikti: &Path| -> Result<u16, String> {
            i2.lock().unwrap().push((yol.to_string(), serde_json::from_str(govde).unwrap()));
            std::fs::write(cikti, &*g2.lock().unwrap()).unwrap();
            Ok(200)
        };
        assert!(guncelle(&y, &log, &m, &hedef, &harita(&c), &indir, 1_790_705_134.0).is_empty());
        assert_eq!(istekler.lock().unwrap()[0], ("/api/liste".to_string(), json!({"cihaz_anahtari": "anahtar", "kategori": "kumar"})));
        assert_eq!(String::from_utf8(ac(&y.gz("kumar")).unwrap()).unwrap(), eski); // eski dosya yerinde
        assert_eq!(meta_oku(&y, "kumar").unwrap().sha256, sha256(eski.as_bytes()));
        assert!(!y.dizin.join("kumar.indir").exists());
        // doğru içerik → yazılır (aynı sürüm hatadan bir saat sonra yeniden denenir)
        *gonderilen.lock().unwrap() = gz(yeni.as_bytes());
        assert_eq!(guncelle(&y, &log, &m, &hedef, &harita(&c), &indir, 1_790_705_134.0 + 3601.0), vec!["kumar".to_string()]);
        assert_eq!(String::from_utf8(ac(&y.gz("kumar")).unwrap()).unwrap(), yeni);
        assert_eq!(meta_oku(&y, "kumar").unwrap(), Meta { sha256: hedef["kumar"].sha256.clone(), sayi: 2, zaman: hedef["kumar"].zaman.clone() });
        // merkez 404 / ulaşılamaz → eski kalır
        let h2: BTreeMap<String, Meta> = [("kumar".to_string(), meta("x.com\n"))].into();
        assert!(guncelle(&y, &log, &m, &h2, &harita(&c), &|_: &str, _: &str, _: &Path| Ok(404), 1_790_705_134.0).is_empty());
        assert!(guncelle(&y, &log, &m, &h2, &harita(&c), &|_: &str, _: &str, _: &Path| Err("merkeze ulaşılamadı".into()), 1_790_705_134.0 + 3601.0).is_empty());
        assert_eq!(String::from_utf8(ac(&y.gz("kumar")).unwrap()).unwrap(), yeni);
        let audit = std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("HAZIR_LISTE_HATA") && audit.contains("kategori=kumar neden=sha"));
        assert!(audit.contains(&format!("kategori=kumar surum={} sayi=2", &hedef["kumar"].sha256[..8])));
        assert!(audit.contains("neden=http_404") && audit.contains("neden=indirme"));
        let _ = std::fs::remove_dir_all(&root);
    }

    const T: f64 = 1_790_705_134.0;

    /// Aynı sürüm hata verdiyse (ör. sha tutmadı) bir saat yeniden denenmez; sağlık raporu her 5 dk'da işi başlatmaz.
    /// Merkezde yeni sürüm çıkınca hemen denenir.
    #[test]
    fn hata_sonrasi_ayni_surum_bir_saat_beklenir() {
        let (y, root) = kur();
        let log = root.join("5651").display().to_string();
        let c = cfg("");
        let m = Merkez { cihaz_anahtari: "anahtar".into(), ..Default::default() };
        let hedef: BTreeMap<String, Meta> = [("kumar".to_string(), meta("bet365.com\n"))].into();
        let sayac = Arc::new(AtomicUsize::new(0));
        let s2 = sayac.clone();
        let bozuk = move |_: &str, _: &str, c: &Path| -> Result<u16, String> {
            s2.fetch_add(1, Ordering::SeqCst);
            std::fs::write(c, gz(b"baska.com\n")).unwrap();
            Ok(200)
        };
        assert!(guncelle(&y, &log, &m, &hedef, &harita(&c), &bozuk, T).is_empty());
        assert_eq!(sayac.load(Ordering::SeqCst), 1);
        let bas = std::cell::Cell::new(0);
        assert!(!hedefi_isle(&y, &hedef, &harita(&c), T + 300.0, &|| bas.set(bas.get() + 1)));
        assert!(guncelle(&y, &log, &m, &hedef, &harita(&c), &bozuk, T + 3000.0).is_empty());
        assert_eq!((sayac.load(Ordering::SeqCst), bas.get()), (1, 0));
        // bir saat sonra yeniden
        assert!(hedefi_isle(&y, &hedef, &harita(&c), T + 3601.0, &|| bas.set(bas.get() + 1)));
        guncelle(&y, &log, &m, &hedef, &harita(&c), &bozuk, T + 3601.0);
        assert_eq!(sayac.load(Ordering::SeqCst), 2);
        // yeni sürüm: beklemeden
        let yeni: BTreeMap<String, Meta> = [("kumar".to_string(), meta("bets10.com\n"))].into();
        let iyi = |_: &str, _: &str, c: &Path| -> Result<u16, String> {
            std::fs::write(c, gz(b"bets10.com\n")).unwrap();
            Ok(200)
        };
        assert_eq!(guncelle(&y, &log, &m, &yeni, &harita(&c), &iyi, T + 3700.0), vec!["kumar".to_string()]);
        // başarıdan sonra hata kaydı temizlenir: eski sürüm tekrar hedef olursa beklemeden denenir
        guncelle(&y, &log, &m, &hedef, &harita(&c), &bozuk, T + 3800.0);
        assert_eq!(sayac.load(Ordering::SeqCst), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Dosya yazıldı ama dnsmasq yeniden okumadıysa (HUP başarısız) bir sonraki uygula içerik aynı olsa da yeniden dener.
    #[test]
    fn basarisiz_hup_sonraki_uygulamada_yinelenir() {
        let (y, root) = kur();
        koy(&y, "kumar", "bet365.com\n");
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let hup_ok = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (c2, h2) = (calls.clone(), hup_ok.clone());
        let runner = move |a: &[String]| {
            c2.lock().unwrap().push(a.join(" "));
            a[1] != "kill" || h2.load(Ordering::SeqCst)
        };
        let c = cfg("");
        assert!(uygula(&c, &y, &runner).is_err());
        assert!(std::fs::read_to_string(&y.conf).unwrap().contains("server=/bet365.com/"));
        calls.lock().unwrap().clear();
        assert!(uygula(&c, &y, &runner).is_err()); // içerik aynı ama okutulmadı → yine dener
        assert!(calls.lock().unwrap().iter().any(|x| x.contains("HUP")));
        hup_ok.store(true, Ordering::SeqCst);
        assert_eq!(uygula(&c, &y, &runner), Ok(1));
        calls.lock().unwrap().clear();
        assert_eq!(uygula(&c, &y, &runner), Ok(1)); // okutuldu → bir daha gerekmez
        assert!(calls.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// İş ayarı uygulama anında okur; uygularken ayar değişirse (panelde ikinci tıklama) son ayar yazılır.
    #[test]
    fn uygula_guncel_son_ayari_yazar() {
        let (y, root) = kur();
        koy(&y, "kumar", "bet365.com\n");
        koy(&y, "yetiskin", "porn.example\n");
        let yol = root.join("ayarlar.toml");
        std::fs::write(&yol, "[filtre.hazir]\nkumar = false\n").unwrap();
        let yol_s = yol.display().to_string();
        let ilk = std::sync::atomic::AtomicBool::new(true);
        let y2 = yol.clone();
        let runner = move |a: &[String]| {
            // ilk yeniden okutmada panel ayarı değiştirmiş olsun
            if a[1] == "kill" && ilk.swap(false, Ordering::SeqCst) {
                std::fs::write(&y2, "[filtre.hazir]\nkumar = true\nyetiskin = false\n").unwrap();
            }
            true
        };
        let eski = cfg(""); // işi başlatanın elindeki (eski) ayar kullanılmaz
        assert_eq!(uygula_guncel(&yol_s, &eski, &y, &runner), Ok(1));
        let text = std::fs::read_to_string(&y.conf).unwrap();
        assert!(text.contains("server=/bet365.com/") && !text.contains("porn.example"), "{text}");
        // ayar dosyası okunamazsa elindeki ayarla
        assert_eq!(uygula_guncel(&root.join("yok.toml").display().to_string(), &eski, &y, &runner), Ok(2));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// "Bu adres engelli mi?" listeyi satır satır tarar (bütün alan adlarını belleğe almaz); anlam aynı.
    #[test]
    fn bul_buyuk_listede_akarak_arar() {
        let (y, root) = kur();
        let mut metin = String::new();
        for i in 0..50_000 {
            metin.push_str(&format!("site{i}.com\n"));
        }
        metin.push_str("1.2.3.4\nson-site.com\n");
        koy(&y, "zararli", &metin);
        let c = cfg("");
        assert_eq!(bul(&c, &y, "a.b.son-site.com"), Some("Zararlı ve dolandırıcı siteler"));
        assert_eq!(bul(&c, &y, "site49999.com"), Some("Zararlı ve dolandırıcı siteler"));
        assert_eq!(bul(&c, &y, "1.2.3.4"), None); // IP adresi liste girdisi sayılmaz
        assert_eq!(bul(&c, &y, "yok.com"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn birlesik_dosya_ve_uygula() {
        let (y, root) = kur();
        koy(&y, "kumar", "bet365.com\nbetter-bet.com\nkotu satir\nx.com/../y\n1.2.3.4\n10.0.0.1\n");
        koy(&y, "yetiskin", "porn.example\nbet365.com\n");
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let runner = move |a: &[String]| {
            c2.lock().unwrap().push(a.join(" "));
            true
        };
        let c = cfg("istisnalar = ['better']\n");
        assert_eq!(uygula(&c, &y, &runner), Ok(2));
        let text = std::fs::read_to_string(&y.conf).unwrap();
        let satirlar: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(satirlar, vec!["server=/bet365.com/", "server=/porn.example/"]); // istisna alt dizesi çıkar, geçersiz satır yok, tekrar yok
        assert!(calls.lock().unwrap().iter().any(|x| x == "systemctl kill --kill-whom=main -s HUP dnsmasq.service"));
        // aynı içerik: yeniden okutulmaz
        calls.lock().unwrap().clear();
        assert_eq!(uygula(&c, &y, &runner), Ok(2));
        assert!(calls.lock().unwrap().is_empty());
        // kumar kapatılınca onun alan adları hemen çıkar
        let kapali = cfg("istisnalar = ['better']\n[filtre.hazir]\nkumar = false\nyetiskin = true\n");
        assert_eq!(uygula(&kapali, &y, &runner), Ok(2)); // bet365.com yetişkin listesinde de var
        let kapali2 = cfg("[filtre.hazir]\nyetiskin = false\nkumar = false\n");
        assert_eq!(uygula(&kapali2, &y, &runner), Ok(0));
        let text = std::fs::read_to_string(&y.conf).unwrap();
        assert!(!text.contains("server=")); // boş ama var (dnsmasq dosya yoksa açılmaz)
        assert!(calls.lock().unwrap().iter().any(|x| x.contains("HUP")));
        // dnsmasq çalışmıyorsa (açılış) yeniden okutma denenmez, hata değil
        let durgun = |a: &[String]| a[1] != "is-active" && a[1] != "kill";
        assert_eq!(uygula(&c, &y, &durgun), Ok(2));
        // hiç liste yokken de dosya yazılır
        let (bos, root2) = kur();
        assert_eq!(uygula(&cfg(""), &bos, &runner), Ok(0));
        assert!(bos.conf.exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&root2);
    }

    #[test]
    fn engelli_mi_hangi_listeden() {
        let (y, root) = kur();
        koy(&y, "kumar", "bet365.com\nbetter-bet.com\n");
        let c = cfg("istisnalar = ['better']\n");
        assert_eq!(bul(&c, &y, "m.bet365.com"), Some("Kumar ve bahis"));
        assert_eq!(bul(&c, &y, "bet365.com"), Some("Kumar ve bahis"));
        assert_eq!(bul(&c, &y, "notbet365.com"), None);
        assert_eq!(bul(&c, &y, "www.better-bet.com"), None); // istisna
        assert_eq!(bul(&cfg("[filtre.hazir]\nkumar = false\n"), &y, "bet365.com"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
