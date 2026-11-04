//! Haftalık ve aylık raporların root tarafı (Python rapor.py; spec 2026-10-14-aylik-rapor).
//! 1. arşiv günü başına **ham özet** (`rapor-ham/<arşiv>/<gün>.json`, root 700/600) ve günün bütünlük doğrulaması (MANIFEST
//!    hash'leri + cihazdaki `muhur::verify` ile birebir hash zinciri). Ham özetteki misafir/izinli kimlikleri root'a özel
//!    sırla HMAC'lenmiş takma kimliktir (tekil sayım için); telefon, TC, pasaport, MAC, IP, ad hiç yazılmaz. Hata metinleri
//!    yalnızca gün + istisna türü taşır (dosya adı `kullanicilar/<telefon>.csv.gz` kişisel veridir).
//! 2. müşteri grupları, gün durumu ("tam" yalnızca doğrulanmışsa) ve haftalık/aylık dönem raporları `rapor/<numara>/`
//!    (root:wcpanel 750/640; yalnızca sayılar, takma kimlik yok). Alan adı (DNS) verisi tutulmaz: dns.csv okunmaz.
//!
//! Python'dan farklar: biçim 1 (alan adı listeli) ham/rapor dosyaları okunmaz ve listeden çıkan arşivlerin eski ham
//! klasörleri temizlenmez (spec bilinçli fark 6); önbellekteki dosyanın biçimi güncelden farklıysa yeniden hesaplanır.
//! Hata iletileri Python'daki istisna türü adıdır (`OSError`, `ValueError`, ...): işlevler `Result<_, String>` döner.
//! Arşiv CSV'leri akışla okunur (`istatistik::satirlar`, alan sınırı 131072 karakter): bellekte bir satır.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use hmac::{Hmac, Mac};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::detay::gun_coz;
use crate::istatistik::{self, arsiv_adi_mi, gun_adi_mi, Satir, SatirHata};
use crate::ortak::{self, py_bol, py_strip};
use crate::veri::py_json;


/// Kimliği takma adlandırma sırrının yolu: `<ham>/.sir` (nokta: hiçbir arşiv adı noktayla başlamaz, çakışmaz).
pub const SIR_ADI: &str = ".sir";
/// Ham ve rapor dosyalarının biçimi (2: alan adı verisi yok).
pub const BICIM: u64 = 2;
/// zincir.txt ve eski/zincir.txt.* başına; büyüğü atlanır.
pub const ZINCIR_SINIRI: usize = 8 * 1024 * 1024;
pub const MANIFEST_SINIRI: usize = 16 * 1024 * 1024;
const PARCA: usize = 1 << 20;
const SIFIR: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const GRUP_SINIRI: usize = 64 * 1024;
const HAFTA_EN_COK: usize = 26;
const AY_EN_COK: usize = 24;
/// Bitişten sonraki gün gelmese de bu kadar gün sonra rapor kesinleşir.
const KESIN_GUN: i64 = 7;

/// Arşiv adı → kaynak klasörleri (Python `{ad: {"kaynaklar": [...]}}`; yeni arşiv önce).
pub type Arsivler = BTreeMap<String, Vec<PathBuf>>;
/// `{gün: (manifest_sha, zincir satırı doğru mu)}`.
pub type Zincir = BTreeMap<String, (String, bool)>;

#[cfg(test)]
thread_local! {
    /// `gun_ham` çağrı sayısı (Python testi `R.gun_ham`'ı sarar).
    pub static GUN_HAM_CAGRI: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// `oku_sinirli` çağrılarının dosya adları (Python testi `R._oku_sinirli`'yi sarar).
    pub static OKUNAN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// `donem_hamlari` çağrıları: (baş, bit, yüklenen günler).
    pub static DONEM_HAMLARI: std::cell::RefCell<Vec<(NaiveDate, NaiveDate, Vec<String>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn io_hata(_: io::Error) -> String {
    "OSError".into()
}

fn satir_hata(h: &SatirHata) -> String {
    match h {
        SatirHata::Deger(_) => "ValueError".into(),
        SatirHata::Diger(m) if m.starts_with("_csv.Error") => "Error".into(),
        SatirHata::Diger(_) => "OSError".into(),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256_hex(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

// --- güvenli okuma (cihazdan gelen sembolik bağ/FIFO root'u takmasın, başka dosyayı okutmasın) ---

/// Normal dosyayı bağ izlemeden ve beklemeden açar; yoksa ya da normal dosya değilse None.
fn ac(yol: &Path) -> Option<File> {
    let f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(yol).ok()?;
    f.metadata().ok()?.is_file().then_some(f)
}

pub fn oku_sinirli(yol: &Path, sinir: usize) -> Option<Vec<u8>> {
    #[cfg(test)]
    OKUNAN.with(|o| o.borrow_mut().push(yol.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default()));
    let mut f = ac(yol)?;
    if f.metadata().ok()?.len() > sinir as u64 {
        return None;
    }
    let mut b = Vec::new();
    Read::by_ref(&mut f).take(sinir as u64 + 1).read_to_end(&mut b).ok()?;
    (b.len() <= sinir).then_some(b)
}

fn dosya_sha(mut f: File) -> io::Result<String> {
    let mut h = Sha256::new();
    let mut tampon = vec![0u8; PARCA];
    loop {
        let n = f.read(&mut tampon)?;
        if n == 0 {
            return Ok(hex(&h.finalize()));
        }
        h.update(&tampon[..n]);
    }
}

// --- zincir ---

/// Tek zincir dosyası: cihazdaki `chain_lines` + `verify` ile birebir. Aynı gün → son satır.
fn zincir_dosyasi(metin: &str) -> Zincir {
    let (mut out, mut prev) = (Zincir::new(), SIFIR.to_string());
    for satir in metin.split('\n') {
        let p: Vec<&str> = py_strip(satir).split(';').collect();
        if p.len() != 3 || p[1].chars().count() != 64 || p[2].chars().count() != 64 {
            continue; // yarım/bozuk satır cihazda da sayılmaz (prev'i değiştirmez)
        }
        let (gun, man, zincir) = (p[0], p[1], p[2]);
        let ok = sha256_hex(format!("{prev}{man}").as_bytes()) == zincir;
        prev = zincir.to_string(); // bir önceki satırın YAZILI değeri (cihazdaki verify gibi)
        if gun_adi_mi(gun) {
            out.insert(gun.to_string(), (man.to_string(), ok));
        }
    }
    out
}

pub fn zincir_oku(kaynak: &Path, gerekli: Option<&BTreeSet<String>>) -> Zincir {
    zincir_oku_sinirli(kaynak, gerekli, ZINCIR_SINIRI)
}

/// `kaynak/zincir.txt` + `kaynak/eski/zincir.txt*` (yeniden kurulumda eski zincir; cihaz her yedekte bir kopya bırakır).
/// Bir gün herhangi bir dosyada doğrulanıyorsa ok (o satırın manifest_sha'sı); hiçbirinde değilse ilk bulunan. Normal
/// olmayan ya da büyük dosya okunmaz. Eski kopyalar en yeniden eskiye okunur; `gerekli` (gün kümesi) verilirse hepsi
/// doğrulandığında durulur (boş küme: hiçbir şey okunmaz).
/// Not: "ok kazanır" yüzünden güncel zincirde bozuk ama eski bir kopyada sağlam olan gün `tam` görünebilir (kabul edildi).
pub fn zincir_oku_sinirli(kaynak: &Path, gerekli: Option<&BTreeSet<String>>, sinir: usize) -> Zincir {
    let mut yollar = vec![kaynak.join("zincir.txt")];
    let eski = kaynak.join("eski");
    if fs::symlink_metadata(&eski).map(|m| m.is_dir()).unwrap_or(false) {
        if let Ok(d) = fs::read_dir(&eski) {
            let mut adlar: Vec<String> = d.filter_map(|x| x.ok().and_then(|x| x.file_name().into_string().ok())).filter(|a| a.starts_with("zincir.txt")).collect();
            adlar.sort();
            adlar.reverse();
            yollar.extend(adlar.into_iter().map(|a| eski.join(a)));
        }
    }
    let mut out = Zincir::new();
    for yol in yollar {
        if let Some(g) = gerekli {
            if g.iter().all(|x| out.get(x).map(|v| v.1).unwrap_or(false)) {
                break;
            }
        }
        let Some(b) = oku_sinirli(&yol, sinir) else { continue };
        let Ok(metin) = String::from_utf8(b) else { continue }; // cihazdaki read_to_string de bozuk UTF-8'de boş döner
        for (gun, (man, ok)) in zincir_dosyasi(&metin) {
            match out.get(&gun) {
                Some(v) if !(ok && !v.1) => {}
                _ => {
                    out.insert(gun, (man, ok));
                }
            }
        }
    }
    out
}

// --- gün bütünlüğü ---

fn ad_gecerli(ad: &str) -> bool {
    if ad.is_empty() || ad.contains('\\') || ad.contains('\0') || ad.starts_with('/') || (ad.chars().count() > 1 && ad.chars().nth(1) == Some(':')) {
        return false;
    }
    ad.split('/').all(|p| !matches!(p, "" | "." | ".."))
}

fn hex64_mi(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `(sha256(MANIFEST.sha256), MANIFEST'teki her dosya yerinde ve hash'i tutuyor mu)`. MANIFEST yoksa `(None, false)`.
/// Hangi dosyanın tutmadığı söylenmez (kişi dosyasının adı telefondur). Dosya okuma hatası `Err` (günü düşürür).
pub fn gun_dogrula(gun_dizini: &Path) -> io::Result<(Option<String>, bool)> {
    let Some(b) = oku_sinirli(&gun_dizini.join("MANIFEST.sha256"), MANIFEST_SINIRI) else { return Ok((None, false)) };
    let man_sha = sha256_hex(&b);
    let Ok(metin) = String::from_utf8(b) else { return Ok((Some(man_sha), false)) };
    let Ok(kok) = fs::canonicalize(gun_dizini) else { return Ok((Some(man_sha), false)) };
    let kok_onek = format!("{}/", kok.to_string_lossy());
    for satir in metin.split('\n') {
        let satir = satir.trim_end_matches('\r');
        if satir.starts_with('#') || py_strip(satir).is_empty() {
            continue;
        }
        let Some((ozet, ad)) = satir.split_once("  ") else { return Ok((Some(man_sha), false)) };
        if !hex64_mi(ozet) || !ad_gecerli(ad) {
            return Ok((Some(man_sha), false));
        }
        let mut yol = gun_dizini.to_path_buf();
        ad.split('/').for_each(|p| yol.push(p));
        // realpath gün klasörünün dışına çıkıyorsa (sembolik bağ, ..) ya da yoksa tutmaz
        match fs::canonicalize(&yol) {
            Ok(g) if g.to_string_lossy().starts_with(&kok_onek) => {}
            _ => return Ok((Some(man_sha), false)),
        }
        let Some(f) = ac(&yol) else { return Ok((Some(man_sha), false)) };
        if dosya_sha(f)? != ozet {
            return Ok((Some(man_sha), false));
        }
    }
    Ok((Some(man_sha), true))
}

fn imza_yuru(kok: &Path, d: &Path, girdiler: &mut Vec<(String, u64, i128)>) {
    let Ok(okuyucu) = fs::read_dir(d) else { return }; // os.walk hataları yutar
    for x in okuyucu.flatten() {
        let yol = x.path();
        let Ok(md) = fs::symlink_metadata(&yol) else { continue };
        if md.is_dir() {
            imza_yuru(kok, &yol, girdiler); // gerçek klasör (sembolik bağ md.is_dir() değildir: girdi olarak sayılır)
            continue;
        }
        let goreli = yol.strip_prefix(kok).map(|g| g.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        girdiler.push((goreli, md.size(), md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128));
    }
}

/// Önbellek anahtarı: MANIFEST içeriği + gün klasöründeki her girdinin (göreli ad, boyut, mtime_ns), sıralı.
/// MANIFEST ya da herhangi bir dosya değişince değişir; dosyaların kendisi okunmaz.
pub fn imza(gun_dizini: &Path) -> String {
    let mut h = Sha256::new();
    match oku_sinirli(&gun_dizini.join("MANIFEST.sha256"), MANIFEST_SINIRI) {
        Some(b) => h.update(&b),
        None => h.update(b"\0yok"),
    }
    let mut girdiler = Vec::new();
    imza_yuru(gun_dizini, gun_dizini, &mut girdiler);
    girdiler.sort();
    for (ad, boy, mt) in girdiler {
        h.update(format!("\n{ad}\0{boy}\0{mt}").as_bytes());
    }
    hex(&h.finalize())
}

// --- ham özet ---

/// HMAC-SHA256(sır, anahtar) ilk 16 hex (Python `takma`).
pub fn takma(sir: &[u8], anahtar: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(sir).expect("HMAC her uzunlukta anahtar alır");
    m.update(anahtar.as_bytes());
    hex(&m.finalize().into_bytes())[..16].to_string()
}

/// Sırrın kısa parmak izi (ham'a yazılır): sır yeniden üretilirse eski takma kimlikli hamlar geçersiz sayılır, eski ve yeni
/// kimlikler karışıp herkes "yeni" görünmez. Takma kimliklerle çakışmaz (ayrı ön ek).
pub fn sir_izi(sir: &[u8]) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(sir).expect("HMAC her uzunlukta anahtar alır");
    m.update(b"\0wificorrect-rapor-sir-izi");
    hex(&m.finalize().into_bytes())[..8].to_string()
}

fn alan<'a>(r: &'a Satir, k: &str) -> &'a str {
    r.get(k).map(String::as_str).unwrap_or("")
}

/// `\d{4}-\d{2}-\d{2}T([01]\d|2[0-3]):[0-5]\d:[0-5]\d` (başta eşleşme, ASCII): saat ya da None.
fn saat_of(r: &Satir) -> Option<u32> {
    let b = alan(r, "zaman").as_bytes();
    if b.len() < 19 {
        return None;
    }
    let rakam = |i: usize| b[i].is_ascii_digit();
    let tarih = (0..4).all(rakam) && b[4] == b'-' && (5..7).all(rakam) && b[7] == b'-' && (8..10).all(rakam) && b[10] == b'T';
    let saat = rakam(11) && rakam(12) && (b[11] <= b'1' || (b[11] == b'2' && b[12] <= b'3'));
    let dk_sn = b[13] == b':' && (b'0'..=b'5').contains(&b[14]) && rakam(15) && b[16] == b':' && (b'0'..=b'5').contains(&b[17]) && rakam(18);
    (tarih && saat && dk_sn).then(|| ((b[11] - b'0') * 10 + (b[12] - b'0')) as u32)
}

/// Python `int(s) if s.isascii() and s.isdigit() else 0` (taşarsa en büyük değere doyar).
fn sayi(s: &str) -> u64 {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    s.parse::<u64>().unwrap_or(u64::MAX)
}

pub fn surum_gecerli(v: &str) -> bool {
    (1..=32).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'~' | b'-'))
}

fn ek_deger(ek: &str, anahtar: &str) -> Option<String> {
    let onek = format!("{anahtar}=");
    for t in py_bol(ek) {
        if let Some(v) = t.strip_prefix(&onek) {
            return surum_gecerli(v).then(|| v.to_string());
        }
    }
    None
}

fn satirlar(gun_dizini: &Path, ad: &str, f: &mut dyn FnMut(Satir)) -> Result<(), String> {
    istatistik::satirlar(&gun_dizini.join(ad), &mut |r| {
        f(r);
        true
    })
    .map_err(|h| satir_hata(&h))
}

/// Tamam (`istatistik::tamam_mi`) bir günün ham özeti. Bozuk gz `Err` verir (günü düşürür).
pub fn gun_ham(gun_dizini: &Path, sir: &[u8]) -> Result<Value, String> {
    #[cfg(test)]
    GUN_HAM_CAGRI.with(|c| c.set(c.get() + 1));
    let imz = imza(gun_dizini);
    let (man_sha, dosya_ok) = gun_dogrula(gun_dizini).map_err(io_hata)?;
    let (mut misafir, mut izinli) = (BTreeSet::new(), BTreeSet::new());
    let mut saat: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let (mut sms_giris, mut kod_giris) = (0u64, 0u64);
    let (mut gonderilen, mut alinan, mut sms) = (0u64, 0u64, 0u64);
    let (mut surum, mut surum_zaman): (Option<String>, String) = (None, String::new());

    satirlar(gun_dizini, "trafik.csv.gz", &mut |r| {
        let Some(s) = saat_of(&r) else { return };
        let tel = alan(&r, "telefon");
        if !tel.is_empty() {
            let k = takma(sir, tel);
            misafir.insert(k.clone());
            saat.entry(s).or_default().insert(k);
        } else if alan(&r, "ek").contains("izinli=") && !alan(&r, "mac").is_empty() {
            izinli.insert(takma(sir, &format!("mac:{}", alan(&r, "mac"))));
        }
        if alan(&r, "olay") == "BAGLANTI_BITIS" {
            gonderilen = gonderilen.saturating_add(sayi(alan(&r, "gonderilen_bayt")));
            alinan = alinan.saturating_add(sayi(alan(&r, "alinan_bayt")));
        }
    })?;

    satirlar(gun_dizini, "oturum.csv.gz", &mut |r| {
        if alan(&r, "olay") == "OTURUM_BASLA" && saat_of(&r).is_some() {
            if py_bol(alan(&r, "ek")).contains(&"giris=misafir_kodu") {
                kod_giris += 1;
            } else {
                sms_giris += 1;
            }
        }
    })?;

    satirlar(gun_dizini, "denetim.csv.gz", &mut |r| {
        let olay = alan(&r, "olay");
        if saat_of(&r).is_none() {
            return;
        }
        if olay == "OTP_GONDERILDI" {
            sms += 1;
        } else if olay == "GUNCELLEME_TAMAM" || olay == "GUNCELLEME_GERI_ALINDI" {
            let v = ek_deger(alan(&r, "ek"), if olay == "GUNCELLEME_TAMAM" { "surum" } else { "onceki" });
            let z = alan(&r, "zaman");
            if let Some(v) = v {
                if z >= surum_zaman.as_str() {
                    surum = Some(v);
                    surum_zaman = z.to_string();
                }
            }
        }
    })?;

    let mut saat_obj = Map::new();
    for (s, k) in &saat {
        saat_obj.insert(s.to_string(), json!(k));
    }
    Ok(json!({
        "bicim": BICIM, "imza": imz, "sir_izi": sir_izi(sir), "manifest_sha": man_sha, "dosya_ok": dosya_ok,
        "misafir": misafir, "saat": saat_obj, "giris": {"sms": sms_giris, "kod": kod_giris}, "izinli": izinli,
        "gonderilen": gonderilen, "alinan": alinan, "sms": sms, "surum": surum
    }))
}

fn json_oku(yol: &Path) -> Option<Value> {
    let b = fs::read(yol).ok()?;
    match serde_json::from_str::<Value>(std::str::from_utf8(&b).ok()?).ok()? {
        v @ Value::Object(_) => Some(v),
        _ => None,
    }
}

/// Python `deger == BICIM` (2 == 2.0 doğru).
fn bicim_guncel(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64) == Some(BICIM as f64)
}

/// Geçici dosya + fsync + yer değiştirme; hata olursa geçici dosya kalmaz. `yaz` içeriği yazar.
fn yaz_atomik_f(yol: &Path, kip: u32, grup: Option<&str>, yaz: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let mut g = yol.as_os_str().to_owned();
    g.push(format!(".{}.tmp", std::process::id()));
    let gecici = PathBuf::from(g);
    fn is(gecici: &Path, yol: &Path, kip: u32, grup: Option<&str>, yaz: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(kip).open(gecici)?;
        yaz(&mut f)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        fs::set_permissions(gecici, fs::Permissions::from_mode(kip))?;
        if let Some(g) = grup {
            chown_grup(gecici, g)?;
        }
        fs::rename(gecici, yol)
    }
    let r = is(&gecici, yol, kip, grup, yaz);
    if r.is_err() {
        let _ = fs::remove_file(&gecici);
    }
    r
}

/// `shutil.chown(yol, group=ad)`.
fn chown_grup(yol: &Path, grup: &str) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let ad = CString::new(grup).map_err(|_| io::Error::other("grup adı"))?;
    // SAFETY: getgrnam kendi statik alanına işaretçi döner; hemen okunur, başka iş parçacığı bu işlevi çağırmaz
    let gid = unsafe {
        let g = libc::getgrnam(ad.as_ptr());
        if g.is_null() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "LookupError"));
        }
        (*g).gr_gid
    };
    let y = CString::new(yol.as_os_str().as_bytes()).map_err(|_| io::Error::other("yol"))?;
    // SAFETY: y geçerli, NUL sonlu bir C dizisi
    if unsafe { libc::chown(y.as_ptr(), u32::MAX, gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn json_yaz(f: &mut File, v: &Value) -> io::Result<()> {
    f.write_all(&serde_json::to_vec(v).map_err(io::Error::other)?)
}

fn yaz_600(yol: &Path, v: &Value) -> io::Result<()> {
    yaz_atomik_f(yol, 0o600, None, |f| json_yaz(f, v))
}

/// root:wcpanel 640, atomik.
fn yaz_panel(yol: &Path, v: &Value, grup: Option<&str>) -> io::Result<()> {
    yaz_atomik_f(yol, 0o640, grup, |f| json_yaz(f, v))
}

fn sil(yol: &Path) -> io::Result<()> {
    match fs::remove_file(yol) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn klasor_mu(yol: &Path) -> bool {
    fs::symlink_metadata(yol).map(|m| m.is_dir()).unwrap_or(false)
}

/// Kaynak(lar)daki her tamam gün için `ham_dizini/<arşiv>/<gün>.json`'u yoksa ya da imza/biçim değiştiyse yeniden hesaplar;
/// değişmeyen gün okunmaz. Aynı gün iki kaynakta ise tamam olan ilki geçerli. Bozuk gün atlanır (eski ham'ı silinir, sonraki
/// çalıştırmada yeniden denenir); artık tamam olmayan günün ham'ı silinir. Dönüş: `["<gün>: <istisna türü>", …]`.
pub fn ham_guncelle(arsiv_adi: &str, kaynaklar: &[PathBuf], ham_dizini: &Path, sir: &[u8]) -> Result<Vec<String>, String> {
    use std::os::unix::fs::DirBuilderExt;
    if !arsiv_adi_mi(arsiv_adi) {
        return Err("ValueError".into());
    }
    let hedef = ham_dizini.join(arsiv_adi);
    fs::DirBuilder::new().recursive(true).mode(0o700).create(&hedef).map_err(io_hata)?;
    fs::set_permissions(ham_dizini, fs::Permissions::from_mode(0o700)).map_err(io_hata)?;
    fs::set_permissions(&hedef, fs::Permissions::from_mode(0o700)).map_err(io_hata)?;

    let mut gunler: BTreeMap<String, PathBuf> = BTreeMap::new();
    for k in kaynaklar {
        let g = k.join("gunluk");
        if !klasor_mu(&g) {
            continue;
        }
        let mut adlar: Vec<String> = fs::read_dir(&g).map_err(io_hata)?.filter_map(|d| d.ok().and_then(|d| d.file_name().into_string().ok())).collect();
        adlar.sort();
        for gun in adlar {
            let d = g.join(&gun);
            if gunler.contains_key(&gun) || !gun_adi_mi(&gun) || !klasor_mu(&d) {
                continue;
            }
            if istatistik::tamam_mi(&d) {
                // bozuk MANIFEST (UTF-8 değil, klasör, okunamaz) yalnızca o günü düşürür
                gunler.insert(gun, d);
            }
        }
    }

    let (mut hatalar, iz) = (Vec::new(), sir_izi(sir));
    for (gun, d) in &gunler {
        let yol = hedef.join(format!("{gun}.json"));
        let sonuc = (|| -> Result<(), String> {
            if let Some(m) = json_oku(&yol) {
                if bicim_guncel(m.get("bicim")) && m.get("sir_izi").and_then(Value::as_str) == Some(iz.as_str()) && m.get("imza").and_then(Value::as_str) == Some(imza(d).as_str()) {
                    return Ok(());
                }
            }
            yaz_600(&yol, &gun_ham(d, sir)?).map_err(io_hata)
        })();
        if let Err(tur) = sonuc {
            // bozuk/kesik gz, normal olmayan dosya: yalnızca o gün düşer
            sil(&yol).map_err(io_hata)?;
            hatalar.push(format!("{gun}: {tur}"));
        }
    }

    for x in fs::read_dir(&hedef).map_err(io_hata)?.flatten() {
        let a = x.file_name().to_string_lossy().into_owned();
        if let Some(gun) = a.strip_suffix(".json") {
            if gun_adi_mi(gun) && !gunler.contains_key(gun) {
                sil(&x.path()).map_err(io_hata)?;
            }
        }
    }
    Ok(hatalar)
}

/// Root'a özel takma kimlik sırrı (panelin kisi_sir'i değil).
pub fn sir_al(yol: &Path) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(ust) = yol.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(ust)?;
        fs::set_permissions(ust, fs::Permissions::from_mode(0o700))?;
    }
    crate::detay::sir_oku_ya_uret(yol)
}

// ======================================================================================================================
// Görev 2: gruplar, gün durumu, dönem raporları (rapor/<numara>/, root:wcpanel 750/640, KİŞİSEL VERİ YOK)
// ======================================================================================================================

/// `[1-9][0-9]{6}`.
pub fn numara_mi(s: &str) -> bool {
    s.len() == 7 && s.as_bytes()[0] != b'0' && s.bytes().all(|b| b.is_ascii_digit())
}

/// `(?:hafta-\d{4}-W\d{2}|ay-\d{4}-\d{2})\.json` (tam eşleşme, ASCII).
fn donem_dosyasi_mi(a: &str) -> bool {
    let Some(g) = a.strip_suffix(".json") else { return false };
    let rakamlar = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if let Some(r) = g.strip_prefix("hafta-") {
        return r.len() == 8 && r.is_char_boundary(4) && r.is_char_boundary(6) && rakamlar(&r[..4]) && &r[4..6] == "-W" && rakamlar(&r[6..]);
    }
    if let Some(r) = g.strip_prefix("ay-") {
        return r.len() == 7 && r.is_char_boundary(4) && r.is_char_boundary(5) && rakamlar(&r[..4]) && &r[4..5] == "-" && rakamlar(&r[5..]);
    }
    false
}

fn tarih(gun: &str) -> Option<NaiveDate> {
    gun_coz(gun)
}

/// Panelin yazdığı `{"<numara>": [eski arşiv adı, …]}`; güvenilmez: normal dosya, ≤ 64 KB, JSON sözlük. Bozuksa boş.
pub fn grup_dosyasi_oku(yol: &Path) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(b) = oku_sinirli(yol, GRUP_SINIRI) else { return out };
    let Ok(metin) = String::from_utf8(b) else { return out };
    let Ok(Value::Object(d)) = serde_json::from_str::<Value>(&metin) else { return out };
    for (k, v) in d {
        if let (true, Value::Array(l)) = (numara_mi(&k), v) {
            out.insert(k, l.into_iter().filter_map(|a| a.as_str().map(str::to_string)).collect());
        }
    }
    out
}

/// `{numara: [(arşiv adı, kaynak), …]}`: müşterinin numara arşivi önce, bağlı eski arşivler ada göre. Eski arşiv 7 haneli
/// numara olamaz (başka müşterinin cihaz arşivi çekilemez), var olmalı ve tek gruba girer (numara sırasıyla ilk kazanır).
/// Bağsız eski arşiv rapor almaz.
pub fn gruplar(arsivler: &Arsivler, grup_dosyasi: &Path) -> BTreeMap<String, Vec<(String, PathBuf)>> {
    let bag = grup_dosyasi_oku(grup_dosyasi);
    let (mut verilen, mut out) = (BTreeSet::new(), BTreeMap::new());
    let mut numaralar: BTreeSet<&String> = arsivler.keys().filter(|a| numara_mi(a)).collect();
    numaralar.extend(bag.keys());
    for n in numaralar {
        let mut adlar: Vec<&String> = if arsivler.contains_key(n) { vec![n] } else { Vec::new() };
        let eski: BTreeSet<&String> = bag.get(n).map(|l| l.iter().collect()).unwrap_or_default();
        for ad in eski {
            if arsiv_adi_mi(ad) && !numara_mi(ad) && arsivler.contains_key(ad) && !verilen.contains(ad) {
                verilen.insert(ad);
                adlar.push(ad);
            }
        }
        if !adlar.is_empty() {
            out.insert(n.clone(), adlar.iter().flat_map(|ad| arsivler[*ad].iter().map(move |k| ((*ad).clone(), k.clone()))).collect());
        }
    }
    out
}

/// Gün haritasındaki seçilen kaynak.
#[derive(Clone, Debug)]
pub struct Girdi {
    pub arsiv: String,
    pub kaynak: PathBuf,
    pub dizin: PathBuf,
    pub tamam: bool,
}

/// `{gün: Girdi}`: aynı gün birden çok kaynakta ise ilk tamam olan, yoksa ilki.
pub fn gun_haritasi(grup: &[(String, PathBuf)]) -> BTreeMap<String, Girdi> {
    let mut out: BTreeMap<String, Girdi> = BTreeMap::new();
    for (ad, kaynak) in grup {
        let g = kaynak.join("gunluk");
        if !klasor_mu(&g) {
            continue;
        }
        let Ok(d) = fs::read_dir(&g) else { continue };
        let mut adlar: Vec<String> = d.filter_map(|x| x.ok().and_then(|x| x.file_name().into_string().ok())).collect();
        adlar.sort();
        for gun in adlar {
            let d = g.join(&gun);
            if tarih(&gun).is_none() || !klasor_mu(&d) {
                continue;
            }
            if out.get(&gun).map(|o| o.tamam).unwrap_or(false) {
                continue;
            }
            let tamam = istatistik::tamam_mi(&d);
            if !out.contains_key(&gun) || tamam {
                out.insert(gun, Girdi { arsiv: ad.clone(), kaynak: kaynak.clone(), dizin: d, tamam });
            }
        }
    }
    out
}

/// `iz` verilirse ham aynı sırla yazılmış olmalı (sır izi tutmalı).
fn ham_gecerli(h: &Value, iz: Option<&str>) -> bool {
    let liste = |k: &str| h.get(k).map(Value::is_array).unwrap_or(false);
    let nesne = |k: &str| h.get(k).map(Value::is_object).unwrap_or(false);
    h.is_object()
        && bicim_guncel(h.get("bicim"))
        && h.get("imza").map(Value::is_string).unwrap_or(false)
        && liste("misafir")
        && liste("izinli")
        && nesne("saat")
        && nesne("giris")
        && iz.map(|i| h.get("sir_izi").and_then(Value::as_str) == Some(i)).unwrap_or(true)
}

fn ham_yolu(ham_dizini: &Path, g: &Girdi) -> PathBuf {
    ham_dizini.join(&g.arsiv).join(format!("{}.json", g.dizin.file_name().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default()))
}

/// Seçilen kaynağın ham özeti; yoksa, bozuksa, başka sırla yazılmışsa ya da gün klasörü ham yazıldıktan sonra değiştiyse None.
fn ham_yukle(ham_dizini: &Path, g: &Girdi, iz: Option<&str>) -> Option<Value> {
    if !g.tamam {
        return None;
    }
    let h = json_oku(&ham_yolu(ham_dizini, g))?;
    (ham_gecerli(&h, iz) && h["imza"].as_str() == Some(&imza(&g.dizin))).then_some(h)
}

/// Genel özetteki bir günün küçük bilgileri.
#[derive(Clone, Debug)]
pub struct GunMeta {
    pub imza: String,
    /// Yalnızca geçerli sürüm metni.
    pub surum: Option<String>,
    pub farkli: usize,
    pub dosya_ok: bool,
    /// Boş olmayan metin; yoksa None.
    pub manifest_sha: Option<String>,
}

/// Grubun bütün günlerinden yalnızca küçük bilgiler: `meta` {gün: ...} ve `ilk` {takma kimlik: ilk görüldüğü gün} ("yeni"
/// misafir için). Ham özetler tek tek okunup bırakılır: bellek bir günün hamı + farklı misafir sayısı kadar.
#[derive(Default)]
pub struct Ozet {
    pub meta: BTreeMap<String, GunMeta>,
    pub ilk: HashMap<String, String>,
    pub iz: String,
}

fn kume(v: Option<&Value>) -> impl Iterator<Item = &str> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str)
}

fn ozet_ekle(ozet: &mut Ozet, g: &str, h: &Value) {
    let m: BTreeSet<&str> = kume(h.get("misafir")).collect();
    ozet.meta.insert(
        g.to_string(),
        GunMeta {
            imza: h["imza"].as_str().unwrap_or_default().to_string(),
            surum: h.get("surum").and_then(Value::as_str).filter(|v| surum_gecerli(v)).map(str::to_string),
            farkli: m.len(),
            dosya_ok: h.get("dosya_ok") == Some(&Value::Bool(true)),
            manifest_sha: h.get("manifest_sha").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
        },
    );
    for k in m {
        match ozet.ilk.get(k) {
            Some(i) if g >= i.as_str() => {}
            _ => {
                ozet.ilk.insert(k.to_string(), g.to_string());
            }
        }
    }
}

pub fn genel_ozet(ham_dizini: &Path, harita: &BTreeMap<String, Girdi>, iz: Option<&str>) -> Ozet {
    let mut ozet = Ozet { iz: iz.unwrap_or("").to_string(), ..Default::default() };
    for (g, girdi) in harita {
        if let Some(h) = ham_yukle(ham_dizini, girdi, iz) {
            ozet_ekle(&mut ozet, g, &h);
        }
    }
    ozet
}

/// Yalnızca bu dönemin bugünden önceki, genel özette yer alan günlerinin ham özetleri (bellek ≤ bir ayın hamları).
pub fn donem_hamlari(ham_dizini: &Path, harita: &BTreeMap<String, Girdi>, ozet: &Ozet, bas: NaiveDate, bit: NaiveDate, bugun: NaiveDate) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let son = bit.min(bugun.pred_opt().unwrap_or(bugun));
    for t in gunler(bas, son) {
        let g = t.format("%Y-%m-%d").to_string();
        let (Some(meta), Some(girdi)) = (ozet.meta.get(&g), harita.get(&g)) else { continue };
        if let Some(h) = json_oku(&ham_yolu(ham_dizini, girdi)) {
            let iz = (!ozet.iz.is_empty()).then_some(ozet.iz.as_str());
            if ham_gecerli(&h, iz) && h["imza"].as_str() == Some(&meta.imza) {
                out.insert(g, h);
            }
        }
    }
    #[cfg(test)]
    DONEM_HAMLARI.with(|c| c.borrow_mut().push((bas, bit, out.keys().cloned().collect())));
    out
}

/// `(durum, neden)`. `tam`'ın tek yolu: klasör tamam + ham var + dosya hash'leri tutuyor + zincir satırı doğru + zincirdeki
/// manifest_sha MANIFEST'in sha'sına eşit. Emin olunamayan her şey `tam` değildir. `ilk`: grubun ilk arşiv günü; ondan önceki
/// günler `hizmet_oncesi` (cihaz henüz yoktu; kayıt sayımına girmez).
pub fn gun_durumu(gun: &str, bugun: NaiveDate, girdi: Option<&Girdi>, ham: Option<&GunMeta>, zincir: &Zincir, ilk: Option<NaiveDate>) -> (&'static str, &'static str) {
    let t = tarih(gun).unwrap_or(bugun);
    if t > bugun {
        return ("gelecek", "");
    }
    if t == bugun {
        return ("bugun", "");
    }
    if ilk.map(|i| t < i).unwrap_or(false) {
        return ("hizmet_oncesi", "");
    }
    let Some(girdi) = girdi else {
        return (if t >= bugun - Duration::days(1) { "bekleniyor" } else { "kayit_yok" }, "");
    };
    if !girdi.tamam {
        return ("eksik", "");
    }
    let Some(ham) = ham else { return ("dogrulanamadi", "okunamadi") };
    if !ham.dosya_ok {
        return ("dogrulanamadi", "dosya");
    }
    let Some(z) = zincir.get(gun) else { return ("dogrulanamadi", "zincir_yok") };
    if !z.1 {
        return ("dogrulanamadi", "zincir");
    }
    match &ham.manifest_sha {
        Some(m) if *m == z.0 => ("tam", ""),
        _ => ("dogrulanamadi", "manifest"),
    }
}

// --- dönemler (ISO hafta Pzt–Paz, TR takvim ayı) ---

/// `(anahtar, başlangıç, bitiş)`: ISO yılı ile `YYYY-Www`, Pazartesi başlar.
pub fn hafta(t: NaiveDate) -> (String, NaiveDate, NaiveDate) {
    let w = t.iso_week();
    let bas = t - Duration::days(t.weekday().num_days_from_monday() as i64);
    (format!("{}-W{:02}", w.year(), w.week()), bas, bas + Duration::days(6))
}

pub fn ay(t: NaiveDate) -> (String, NaiveDate, NaiveDate) {
    let bas = t.with_day(1).unwrap_or(t);
    let sonraki = (bas + Duration::days(32)).with_day(1).unwrap_or(bas);
    (format!("{}-{:02}", bas.year(), bas.month()), bas, sonraki - Duration::days(1))
}

/// `[(tur, anahtar, baş, bit)]`: haftalar sonra aylar, her biri yeniden eskiye; en çok 26 hafta ve 24 ay.
pub fn donemler(en_eski: NaiveDate, bugun: NaiveDate) -> Vec<(&'static str, String, NaiveDate, NaiveDate)> {
    type DonemFn = fn(NaiveDate) -> (String, NaiveDate, NaiveDate);
    let mut out = Vec::new();
    for (tur, f, en_cok) in [("hafta", hafta as DonemFn, HAFTA_EN_COK), ("ay", ay as DonemFn, AY_EN_COK)] {
        let mut t = bugun;
        for _ in 0..en_cok {
            let (anahtar, bas, bit) = f(t);
            out.push((tur, anahtar, bas, bit));
            if bas <= en_eski {
                break;
            }
            t = bas - Duration::days(1);
        }
    }
    out
}

fn gunler(bas: NaiveDate, bit: NaiveDate) -> impl Iterator<Item = NaiveDate> {
    std::iter::successors(Some(bas), |t| t.succ_opt()).take_while(move |t| *t <= bit)
}

fn gun_s(t: NaiveDate) -> String {
    t.format("%Y-%m-%d").to_string()
}

/// Python `_int(v)`: negatif olmayan, bool olmayan tamsayı; yoksa 0.
fn int_of(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

/// Python `f"{x:.1f}"` (ikili değerin kendisi üzerinde, yarıya-çifte yuvarlama). `x` sonlu ve ≥ 0.
pub fn ondalik1(x: f64) -> String {
    let s = format!("{:.120}", x.abs());
    let (tam, kesir) = s.split_once('.').unwrap_or((&s, ""));
    let kesir = kesir.as_bytes();
    let mut r: Vec<u8> = tam.bytes().chain(std::iter::once(*kesir.first().unwrap_or(&b'0'))).map(|b| b - b'0').collect();
    let ikinci = kesir.get(1).map(|b| b - b'0').unwrap_or(0);
    let gerisi_sifir = kesir.iter().skip(2).all(|&b| b == b'0');
    let tek = r.last().map(|d| d % 2 == 1).unwrap_or(false);
    if ikinci > 5 || (ikinci == 5 && (!gerisi_sifir || tek)) {
        let mut i = r.len();
        loop {
            if i == 0 {
                r.insert(0, 1);
                break;
            }
            i -= 1;
            if r[i] == 9 {
                r[i] = 0;
            } else {
                r[i] += 1;
                break;
            }
        }
    }
    let son = r.pop().unwrap_or(0);
    let tam: String = r.iter().map(|d| (d + b'0') as char).collect();
    format!("{}{}.{}", if tam.is_empty() { "0" } else { "" }, tam, son)
}

/// Python `round(x, 1)`.
fn yuvarla1(x: f64) -> f64 {
    ondalik1(x).parse().unwrap_or(0.0)
}

/// Dönemin ham özet okumadan hesaplanan kısmı: gün durumları, kayıt sayımı, sürüm, kesinlik ve imza. İmza aynıysa dönem
/// raporu (ve dönemin hamları) yeniden okunmaz.
pub struct DonemOn {
    pub imza: String,
    pub kesin: bool,
    pub surum: Option<String>,
    pub sonra: bool,
    pub gunler: Vec<(String, &'static str, &'static str)>,
    pub kayit_gun: u64,
    pub kayit_tam: u64,
}

pub type Durumlar = BTreeMap<String, (&'static str, &'static str)>;

fn donem_on(bas: NaiveDate, bit: NaiveDate, bugun: NaiveDate, grup_adlari: &[String], harita: &BTreeMap<String, Girdi>, durumlar: &Durumlar, ozet: &Ozet) -> DonemOn {
    let meta = &ozet.meta;
    let (bas_s, bit_s) = (gun_s(bas), gun_s(bit));
    let (mut gun_listesi, mut kayit_gun, mut kayit_tam) = (Vec::new(), 0u64, 0u64);
    for t in gunler(bas, bit) {
        let g = gun_s(t);
        let (durum, neden) = durumlar.get(&g).copied().unwrap_or(("bilinmiyor", ""));
        if t < bugun && durum != "hizmet_oncesi" {
            // cihaz kurulmadan önceki günler kayıt sayımına girmez
            kayit_gun += 1;
            kayit_tam += (durum == "tam") as u64;
        }
        gun_listesi.push((g, durum, neden));
    }

    let (mut surum, mut sonra) = (None, false);
    for (g, m) in meta {
        let Some(v) = &m.surum else { continue };
        if *g <= bit_s {
            surum = Some(v.clone());
        } else {
            sonra = true;
        }
    }
    let kesin = bit < bugun && (harita.keys().any(|g| *g > bit_s) || bugun >= bit + Duration::days(KESIN_GUN));

    // imza: dönemin günleri + dönemden önceki günler ("yeni" ve sürüm onlara bağlı) + sonraki sürüm bilgisi + kesin + sır
    let h_imza = |g: &str| meta.get(g).map(|m| m.imza.as_str()).unwrap_or("");
    let onceki: Vec<Value> = meta.keys().filter(|g| **g < bas_s).map(|g| json!([g, h_imza(g)])).collect();
    let onceki_ozet = sha256_hex(py_json(&Value::Array(onceki), true).as_bytes());
    let gunler_json: Vec<Value> =
        gun_listesi.iter().map(|(g, durum, neden)| json!([g, harita.get(g).map(|x| x.arsiv.as_str()).unwrap_or(""), h_imza(g), durum, neden])).collect();
    // anahtarlar sıralı (Python `sort_keys=True`)
    let imza_v = json!({"bicim": BICIM, "grup": grup_adlari, "gunler": gunler_json, "kesin": kesin, "onceki": onceki_ozet, "sir": ozet.iz, "sonra_surum": sonra});
    DonemOn { imza: sha256_hex(py_json(&imza_v, true).as_bytes()), kesin, surum, sonra, gunler: gun_listesi, kayit_gun, kayit_tam }
}

/// Bir dönemin raporu (yalnızca sayılar). `hamlar`: {gün: geçerli ham özet}, en az dönemin günleri; `ozet`: genel özet;
/// `durumlar`: {gün: (durum, neden)}. İstatistik yalnızca bugünden önceki, ham özeti olan günlerden.
#[allow(clippy::too_many_arguments)]
pub fn donem_raporu(
    tur: &str,
    anahtar: &str,
    bas: NaiveDate,
    bit: NaiveDate,
    bugun: NaiveDate,
    simdi: f64,
    grup_adlari: &[String],
    harita: &BTreeMap<String, Girdi>,
    hamlar: &BTreeMap<String, Value>,
    durumlar: &Durumlar,
    ozet: &Ozet,
) -> Value {
    let on = donem_on(bas, bit, bugun, grup_adlari, harita, durumlar, ozet);
    let (mut gun_satirlari, mut s_gunler) = (Vec::new(), Vec::new());
    for (g, durum, neden) in &on.gunler {
        let t = tarih(g).unwrap_or(bugun);
        let h = if t < bugun && ozet.meta.contains_key(g) { hamlar.get(g) } else { None };
        if let Some(h) = h {
            s_gunler.push((t, h));
        }
        let farkli = h.map(|h| Value::from(kume(h.get("misafir")).collect::<BTreeSet<_>>().len())).unwrap_or(Value::Null);
        gun_satirlari.push(json!({"gun": g, "durum": durum, "neden": neden, "farkli": farkli}));
    }

    let (mut farkli, mut izinli): (BTreeSet<&str>, BTreeSet<&str>) = (BTreeSet::new(), BTreeSet::new());
    let mut saat: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); 24];
    let mut hafta_gunu: Vec<BTreeSet<&str>> = vec![BTreeSet::new(); 7];
    let (mut giris_sms, mut giris_kod, mut gonderilen, mut alinan, mut sms) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut gunluk: Vec<(String, usize)> = Vec::new();
    for (t, h) in &s_gunler {
        let m: BTreeSet<&str> = kume(h.get("misafir")).collect();
        farkli.extend(m.iter().copied());
        hafta_gunu[t.weekday().num_days_from_monday() as usize].extend(m.iter().copied());
        izinli.extend(kume(h.get("izinli")));
        if let Some(Value::Object(sa)) = h.get("saat") {
            for (k, v) in sa {
                if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(n) = k.parse::<u64>() {
                        if n < 24 {
                            saat[n as usize].extend(kume(Some(v)));
                        }
                    }
                }
            }
        }
        let giris = h.get("giris");
        giris_sms = giris_sms.saturating_add(int_of(giris.and_then(|g| g.get("sms"))));
        giris_kod = giris_kod.saturating_add(int_of(giris.and_then(|g| g.get("kod"))));
        gonderilen = gonderilen.saturating_add(int_of(h.get("gonderilen")));
        alinan = alinan.saturating_add(int_of(h.get("alinan")));
        sms = sms.saturating_add(int_of(h.get("sms")));
        gunluk.push((gun_s(*t), m.len()));
    }

    let (bas_s, bit_s) = (gun_s(bas), gun_s(bit));
    let donen = farkli.iter().copied().filter(|k| ozet.ilk.get(*k).map(String::as_str).unwrap_or(bas_s.as_str()) < bas_s.as_str()).count();
    let yeni = farkli.len() - donen;

    let mut en_yogun: Option<(&str, usize)> = None;
    for (g, n) in &gunluk {
        // eşitlikte erken gün
        if en_yogun.map(|e| *n > e.1).unwrap_or(true) {
            en_yogun = Some((g, *n));
        }
    }
    let ortalama = if gunluk.is_empty() { 0.0 } else { yuvarla1(gunluk.iter().map(|g| g.1 as f64).sum::<f64>() / gunluk.len() as f64) };

    json!({
        "bicim": BICIM, "tur": tur, "donem": anahtar, "baslangic": bas_s, "bitis": bit_s,
        "uretim": ortak::now_iso(simdi), "kesin": on.kesin, "imza": on.imza,
        "misafir": {
            "farkli": farkli.len(), "yeni": yeni, "donen": donen,
            "giris": giris_sms.saturating_add(giris_kod), "giris_sms": giris_sms, "giris_kod": giris_kod,
            "izinli_cihaz": izinli.len(), "gunluk_ortalama": ortalama,
            "en_yogun_gun": en_yogun.map(|(g, n)| json!({"gun": g, "farkli": n})).unwrap_or(Value::Null)
        },
        "saat": saat.iter().map(BTreeSet::len).collect::<Vec<_>>(), "hafta_gunu": hafta_gunu.iter().map(BTreeSet::len).collect::<Vec<_>>(),
        "gunler": gun_satirlari, "kayit": {"gun": on.kayit_gun, "tam": on.kayit_tam},
        "kullanim": {"gonderilen": gonderilen, "alinan": alinan, "sms": sms},
        "surum": on.surum, "surum_sonra_degisti": on.sonra
    })
}

fn dizin_panel(yol: &Path, grup: Option<&str>) -> io::Result<()> {
    fs::create_dir_all(yol)?;
    fs::set_permissions(yol, fs::Permissions::from_mode(0o750))?;
    if let Some(g) = grup {
        chown_grup(yol, g)?;
    }
    Ok(())
}

/// liste.json satırı; rapor bozuk/eksikse None (yeniden üretilir).
fn liste_girdisi(tur: &str, anahtar: &str, r: &Value) -> Option<Value> {
    let (m, k) = (r.get("misafir")?, r.get("kayit")?);
    if !(m.is_object() && k.is_object()) {
        return None;
    }
    let (bas, bit, kesin) = (r.get("baslangic")?, r.get("bitis")?, r.get("kesin")?);
    let tamsayi = |v: Option<&Value>| v.filter(|x| x.is_i64() || x.is_u64()).cloned();
    let (farkli, kg, kt) = (tamsayi(m.get("farkli"))?, tamsayi(k.get("gun"))?, tamsayi(k.get("tam"))?);
    if !(bas.is_string() && bit.is_string() && kesin.is_boolean()) {
        return None;
    }
    Some(json!({"tur": tur, "donem": anahtar, "baslangic": bas, "bitis": bit, "kesin": kesin, "farkli": farkli, "kayit_gun": kg, "kayit_tam": kt}))
}

/// Bir müşterinin dönem raporları + liste.json. İmzası aynı rapor yeniden yazılmaz (hamları da okunmaz); pencere dışı
/// dosyalar silinir. Bellek: genel özet + en çok bir dönemin hamları. Bugünden sonraki tarihli günler (cihaz saati yanlış)
/// hiç kullanılmaz.
#[allow(clippy::too_many_arguments)]
pub fn grup_raporla(numara: &str, grup: &[(String, PathBuf)], ham_dizini: &Path, cikti: &Path, panel_grubu: Option<&str>, bugun: NaiveDate, simdi: f64, sir: Option<&[u8]>) -> Result<(), String> {
    let bugun_s = gun_s(bugun);
    let harita: BTreeMap<String, Girdi> = gun_haritasi(grup).into_iter().filter(|(g, _)| *g <= bugun_s).collect();
    let mut adlar: Vec<String> = Vec::new();
    for (ad, _) in grup {
        if !adlar.contains(ad) {
            adlar.push(ad.clone());
        }
    }
    let iz = sir.map(sir_izi);
    let ozet = genel_ozet(ham_dizini, &harita, iz.as_deref());
    let ilk = harita.keys().next().and_then(|g| tarih(g)); // grubun ilk arşiv günü; öncesi "hizmet öncesi"
    let pencere = ilk.map(|i| donemler(i, bugun)).unwrap_or_default();
    let hedef = cikti.join(numara);
    dizin_panel(&hedef, panel_grubu).map_err(io_hata)?;

    // zincir: kaynak başına bir kez; yalnızca "tam" olabilecek pencere günleri için eski kopyalara inilir
    let mut gerekli: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
    for (_, _, bas, bit) in &pencere {
        for t in gunler(*bas, *bit) {
            let g = gun_s(t);
            if let (Some(girdi), Some(meta)) = (harita.get(&g), ozet.meta.get(&g)) {
                if meta.dosya_ok {
                    gerekli.entry(girdi.kaynak.clone()).or_default().insert(g);
                }
            }
        }
    }

    let (mut zincirler, mut durumlar, mut liste, mut isteniyor): (HashMap<PathBuf, Zincir>, Durumlar, Vec<Value>, BTreeSet<String>) = Default::default();
    let bos_zincir = Zincir::new();
    for (tur, anahtar, bas, bit) in &pencere {
        for t in gunler(*bas, *bit) {
            let g = gun_s(t);
            if durumlar.contains_key(&g) {
                continue;
            }
            let girdi = harita.get(&g);
            let zincir = match girdi {
                Some(girdi) => {
                    if !zincirler.contains_key(&girdi.kaynak) {
                        let bos = BTreeSet::new();
                        zincirler.insert(girdi.kaynak.clone(), zincir_oku(&girdi.kaynak, Some(gerekli.get(&girdi.kaynak).unwrap_or(&bos))));
                    }
                    &zincirler[&girdi.kaynak]
                }
                None => &bos_zincir,
            };
            let d = gun_durumu(&g, bugun, girdi, ozet.meta.get(&g), zincir, ilk);
            durumlar.insert(g, d);
        }
        let ad = format!("{tur}-{anahtar}.json");
        isteniyor.insert(ad.clone());
        let yol = hedef.join(&ad);
        let on = donem_on(*bas, *bit, bugun, &adlar, &harita, &durumlar, &ozet);
        let mevcut = json_oku(&yol);
        let mut satir = match &mevcut {
            // biçimi güncelden farklı dosya (önceki sürümün yazdığı) imzası tutsa da yeniden hesaplanır
            Some(m) if bicim_guncel(m.get("bicim")) && m.get("imza").and_then(Value::as_str) == Some(on.imza.as_str()) => liste_girdisi(tur, anahtar, m),
            _ => None,
        };
        if satir.is_none() {
            let hamlar = donem_hamlari(ham_dizini, &harita, &ozet, *bas, *bit, bugun);
            let r = donem_raporu(tur, anahtar, *bas, *bit, bugun, simdi, &adlar, &harita, &hamlar, &durumlar, &ozet);
            drop(hamlar);
            yaz_panel(&yol, &r, panel_grubu).map_err(io_hata)?;
            satir = liste_girdisi(tur, anahtar, &r);
        }
        liste.extend(satir);
    }

    let liste = Value::Array(liste);
    let liste_yolu = hedef.join("liste.json");
    let eski_liste = fs::read(&liste_yolu).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    if eski_liste.as_ref() != Some(&liste) {
        yaz_panel(&liste_yolu, &liste, panel_grubu).map_err(io_hata)?;
    }
    for x in fs::read_dir(&hedef).map_err(io_hata)?.flatten() {
        let a = x.file_name().to_string_lossy().into_owned();
        if donem_dosyasi_mi(&a) && !isteniyor.contains(&a) {
            sil(&x.path()).map_err(io_hata)?;
        }
    }
    Ok(())
}

/// Artık grubu olmayan numaraların (eski arşiv bağı kaldırıldı, numara arşivi silindi) raporları ve listesi silinir; başka
/// dosya varsa klasör kalır. Yanlış bağlanmış eski arşivin verisi o müşteride görünmeye devam etmesin.
fn grubu_kalkan_temizle(cikti: &Path, numaralar: &BTreeMap<String, Vec<(String, PathBuf)>>) -> Result<Vec<String>, String> {
    let mut hatalar = Vec::new();
    let mut adlar: Vec<String> = fs::read_dir(cikti).map_err(io_hata)?.filter_map(|d| d.ok().and_then(|d| d.file_name().into_string().ok())).collect();
    adlar.sort();
    for n in adlar {
        let yol = cikti.join(&n);
        if !numara_mi(&n) || numaralar.contains_key(&n) || !klasor_mu(&yol) {
            continue;
        }
        let sonuc = (|| -> io::Result<()> {
            for x in fs::read_dir(&yol)?.flatten() {
                let a = x.file_name().to_string_lossy().into_owned();
                if donem_dosyasi_mi(&a) || a == "liste.json" {
                    sil(&x.path())?;
                }
            }
            let _ = fs::remove_dir(&yol); // bizim olmayan dosya var: dokunulmaz
            Ok(())
        })();
        if sonuc.is_err() {
            hatalar.push(format!("rapor {n}: OSError"));
        }
    }
    Ok(hatalar)
}

/// wc-istatistik'ten: önce her arşivin ham özetleri (bütün kaynakları TEK çağrıda: ayrı çağrılar birbirinin günlerini
/// silerdi), sonra her müşterinin raporları; grubu kalmayan numaraların raporları silinir. Bir arşivin/grubun hatası
/// diğerlerini durdurmaz. Dönüş: hata metinleri (arşiv/numara + gün + istisna türü; dosya adı yok). `Err`: işlemi bütünüyle
/// durduran istisna türü adı (Python'da `calistir`'dan fırlayan istisna).
pub fn calistir(arsivler: &Arsivler, grup_dosyasi: &Path, ham_dizini: &Path, cikti: &Path, panel_grubu: Option<&str>, simdi: Option<f64>, sir: Option<&[u8]>) -> Result<Vec<String>, String> {
    let simdi = simdi.unwrap_or_else(ortak::simdi);
    let bugun = ortak::TZ.timestamp_opt(simdi.floor() as i64, 0).single().map(|t| t.date_naive()).ok_or("ValueError")?;
    let sir: Vec<u8> = match sir {
        Some(s) => s.to_vec(),
        // sır yoksa takma kimlik yok: hiçbir şey yazılmaz
        None => match sir_al(&ham_dizini.join(SIR_ADI)) {
            Ok(s) => s,
            Err(_) => return Ok(vec!["sir: OSError".into()]),
        },
    };
    let mut hatalar = Vec::new();
    for (ad, kaynaklar) in arsivler {
        match ham_guncelle(ad, kaynaklar, ham_dizini, &sir) {
            Ok(h) => hatalar.extend(h.into_iter().map(|h| format!("{ad}: {h}"))),
            Err(t) => hatalar.push(format!("{ad}: {t}")),
        }
    }
    dizin_panel(cikti, panel_grubu).map_err(io_hata)?;
    let gr = gruplar(arsivler, grup_dosyasi);
    for (numara, grup) in &gr {
        if let Err(t) = grup_raporla(numara, grup, ham_dizini, cikti, panel_grubu, bugun, simdi, Some(&sir)) {
            hatalar.push(format!("rapor {numara}: {t}"));
        }
    }
    hatalar.extend(grubu_kalkan_temizle(cikti, &gr)?);
    Ok(hatalar)
}

#[cfg(test)]
#[path = "rapor_testler.rs"]
mod testler;
