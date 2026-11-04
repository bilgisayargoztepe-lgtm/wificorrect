//! wc-durum (Python durum.py; root, 5 dk'da bir): arşivlerin ve tünelin durumu → /var/lib/wificorrect/durum.json
//! (root:wcpanel 640). Panel /srv'yi ve WireGuard'ı göremez; müşteri listesindeki son gelen gün, arşiv boyutu ve tünel
//! bilgisi buradan gelir. Root, panelin veritabanına dokunmaz: arşivler klasörlerden ve kayit.csv'den bulunur.
//!
//! Python'dan fark (spec bilinçli fark 5): gün adı ve el sıkışma sayısı ASCII rakamla doğrulanır (`\d`, `str.isdigit`
//! Unicode rakamı da kabul ederdi). Okunamayan klasör/dosya Python'da istisnaydı: burada `Err` → süreç 1 ile çıkar.
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Map};

use crate::istatistik::{arsiv_adi_mi, gun_adi_mi, sembolik_mi};
use crate::ortak;

pub const KAYIT: &str = "/etc/wireguard/wificorrect/kayit.csv";
pub const YENI: &str = "/srv/wificorrect-arsiv";
pub const ESKI: &str = "/srv/hotspot-arsiv";
pub const DURUM: &str = "/var/lib/wificorrect/durum.json";
/// `wg show` zaman aşımı.
const WG_SURE: Duration = Duration::from_secs(10);

/// Python `arsivler()` sözlüğünün bir girdisi: `{tur, pub, kaynaklar}` (ad ayrı).
#[derive(Clone, Debug, PartialEq)]
pub struct Arsiv {
    pub ad: String,
    /// "cihaz" | "bitti" | "eski"
    pub tur: String,
    /// WireGuard açık anahtarı (eski arşivde ve kaydı olmayanda boş)
    pub wg_pub: String,
    pub kaynaklar: Vec<PathBuf>,
}

fn io(h: std::io::Error) -> String {
    format!("OSError({h})")
}

/// kayit.csv → {arşiv adı: (tur, wg açık anahtarı)} (yalnızca cihaz/bitti satırları); dosya yoksa boş.
fn kayit_oku(kayit: &Path) -> Result<BTreeMap<String, (String, String)>, String> {
    let mut out = BTreeMap::new();
    let ham = match std::fs::read(kayit) {
        Ok(h) => h,
        Err(h) if h.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(h) => return Err(io(h)),
    };
    let metin = String::from_utf8(ham).map_err(|_| "UnicodeDecodeError: kayit.csv UTF-8 değil".to_string())?;
    for s in crate::kuyruk::satirlar(&metin) {
        let p: Vec<&str> = s.split(';').collect();
        if p.len() >= 5 && (p[0] == "cihaz" || p[0] == "bitti") && p[4].starts_with("wfc-") {
            out.insert(p[4][4..].to_string(), (p[0].to_string(), p[3].to_string()));
        }
    }
    Ok(out)
}

/// `sorted(os.listdir(kok))`.
fn adlar(kok: &Path) -> Result<Vec<String>, String> {
    let mut v: Vec<String> = Vec::new();
    for d in std::fs::read_dir(kok).map_err(io)? {
        if let Ok(ad) = d.map_err(io)?.file_name().into_string() {
            v.push(ad); // UTF-8 olmayan ad hiçbir arşiv adı kalıbına uymaz
        }
    }
    v.sort();
    Ok(v)
}

/// {ad: arşiv}: yeni arşiv (cihaz yedekleri) önce; aynı ad eski arşivde de varsa ikinci kaynak olur. Sıra: ekleme sırası.
pub fn arsivler(kayit: &Path, yeni: &Path, eski: &Path) -> Result<Vec<Arsiv>, String> {
    let k = kayit_oku(kayit)?;
    let mut out: Vec<Arsiv> = Vec::new();
    if yeni.is_dir() {
        for ad in adlar(yeni)? {
            let d = yeni.join(&ad).join("veri");
            if arsiv_adi_mi(&ad) && !sembolik_mi(&yeni.join(&ad)) && d.is_dir() {
                let (tur, wg_pub) = k.get(&ad).cloned().unwrap_or(("bitti".into(), String::new()));
                out.push(Arsiv { ad, tur, wg_pub, kaynaklar: vec![d] });
            }
        }
    }
    if eski.is_dir() {
        for ad in adlar(eski)? {
            let d = eski.join(&ad);
            if arsiv_adi_mi(&ad) && !sembolik_mi(&d) && d.is_dir() {
                match out.iter_mut().find(|a| a.ad == ad) {
                    Some(a) => a.kaynaklar.push(d),
                    None => out.push(Arsiv { ad, tur: "eski".into(), wg_pub: String::new(), kaynaklar: vec![d] }),
                }
            }
        }
    }
    Ok(out)
}

/// `wg show wg0 latest-handshakes` çıktısı: {açık anahtar: epoch} (yalnızca 0'dan büyük el sıkışmalar).
pub fn el_sikismalari(metin: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for s in crate::kuyruk::satirlar(metin) {
        let p: Vec<&str> = s.split('\t').collect();
        if p.len() == 2 && !p[1].is_empty() && p[1].bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = p[1].parse::<u64>() {
                if n > 0 {
                    out.insert(p[0].to_string(), n);
                }
            }
        }
    }
    out
}

/// Kaynaklardaki `gunluk/` altındaki en büyük gün klasörü adı.
pub fn son_gun(kaynaklar: &[PathBuf]) -> Result<Option<String>, String> {
    let mut gunler: Vec<String> = Vec::new();
    for k in kaynaklar {
        let g = k.join("gunluk");
        if std::fs::symlink_metadata(&g).map(|m| m.is_dir()).unwrap_or(false) {
            gunler.extend(adlar(&g)?.into_iter().filter(|d| gun_adi_mi(d)));
        }
    }
    Ok(gunler.into_iter().max())
}

fn boyut_dizin(dizin: &Path, toplam: &mut u64) {
    // os.walk: okunamayan klasör sessizce atlanır
    let Ok(girdiler) = std::fs::read_dir(dizin) else { return };
    for g in girdiler.flatten() {
        let yol = g.path();
        let Ok(tur) = g.file_type() else { continue };
        if tur.is_dir() {
            boyut_dizin(&yol, toplam);
            continue;
        }
        // sembolik bağ: hedefi klasörse os.walk dirnames'e koyar ve girmez (dosya sayılmaz); öteki lstat boyutu
        if tur.is_symlink() && std::fs::metadata(&yol).map(|m| m.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Ok(m) = std::fs::symlink_metadata(&yol) {
            *toplam += std::os::unix::fs::MetadataExt::size(&m);
        }
    }
}

/// Dosyaların (`lstat`) toplam bayt sayısı. Her çalıştırmada tüm arşivi tarar (yüzlerce müşteride günde bire indirilir).
pub fn boyut(kaynaklar: &[PathBuf]) -> u64 {
    let mut toplam = 0;
    for k in kaynaklar {
        boyut_dizin(k, &mut toplam);
    }
    toplam
}

/// `wg show` çalıştırıcısı: (çıkış kodu, stdout) ya da başlatılamadı/zaman aşımı.
pub fn wg_calistir() -> Result<(i32, String), String> {
    let mut cocuk = Command::new("wg")
        .args(["show", "wg0", "latest-handshakes"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(io)?;
    let mut boru = cocuk.stdout.take();
    let okuyucu = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(b) = boru.as_mut() {
            let _ = std::io::Read::read_to_end(b, &mut v);
        }
        v
    });
    let bas = Instant::now();
    let kod = loop {
        match cocuk.try_wait() {
            Ok(Some(d)) => break d.code().unwrap_or(-1),
            Ok(None) if bas.elapsed() > WG_SURE => {
                let _ = cocuk.kill();
                let _ = cocuk.wait();
                return Err("TimeoutExpired".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(h) => return Err(io(h)),
        }
    };
    let cikti = okuyucu.join().unwrap_or_default();
    Ok((kod, String::from_utf8_lossy(&cikti).into_owned()))
}

/// `wg show` çıktısı; çalışmadıysa (komut yok, arayüz yok, zaman aşımı) None.
pub fn wg_oku(calistir: &dyn Fn() -> Result<(i32, String), String>) -> Option<String> {
    match calistir() {
        Ok((0, cikti)) => Some(cikti),
        _ => None,
    }
}

/// İşlenecek yollar (Python `main`'in anahtar sözcükleri).
pub struct Yollar {
    pub kayit: PathBuf,
    pub yeni: PathBuf,
    pub eski: PathBuf,
    pub cikti: PathBuf,
    /// `shutil.chown(group=)`; None: sahiplik değişmez (testler).
    pub grup: Option<String>,
}

impl Yollar {
    pub fn uretim() -> Yollar {
        Yollar { kayit: KAYIT.into(), yeni: YENI.into(), eski: ESKI.into(), cikti: DURUM.into(), grup: Some("wcpanel".into()) }
    }
}

fn yaz(y: &Yollar, wg: &dyn Fn() -> Option<String>, simdi: f64) -> Result<(), String> {
    let metin = wg();
    let el = el_sikismalari(metin.as_deref().unwrap_or(""));
    let mut out = Map::new();
    for a in arsivler(&y.kayit, &y.yeni, &y.eski)? {
        let el_sikisma = if a.wg_pub.is_empty() { None } else { el.get(&a.wg_pub) };
        out.insert(
            a.ad.clone(),
            json!({"tur": a.tur, "son_gun": son_gun(&a.kaynaklar)?, "boyut": boyut(&a.kaynaklar), "el_sikisma": el_sikisma}),
        );
    }
    // wg_ok: tünel bilgisi okundu mu (wc-saglik "hepsi ulaşılamıyor"u "okunamadı"dan böyle ayırır)
    ortak::save_json(&y.cikti, &json!({"zaman": ortak::now_iso(simdi), "wg_ok": metin.is_some(), "arsivler": out})).map_err(io)?;
    std::fs::set_permissions(&y.cikti, std::fs::Permissions::from_mode(0o640)).map_err(io)?;
    if let Some(g) = &y.grup {
        crate::ozet::chown_grup(&y.cikti, g)?;
    }
    Ok(())
}

/// Python `main`: 0; hata (Python'da istisna) stderr'e yazılır, çıkış 1.
pub fn calistir(y: &Yollar, wg: &dyn Fn() -> Option<String>, simdi: f64) -> i32 {
    match yaz(y, wg, simdi) {
        Ok(()) => 0,
        Err(h) => {
            eprintln!("wc-durum: {h}");
            1
        }
    }
}

pub fn main_uretim() -> i32 {
    calistir(&Yollar::uretim(), &|| wg_oku(&wg_calistir), ortak::simdi())
}

#[cfg(test)]
#[path = "durum_testler.rs"]
mod testler;
