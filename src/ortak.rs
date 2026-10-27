//! Ortak yardımcılar (eski common.py): doğrulayıcılar, zaman, 5651 CSV yazımı, oturum durumu, nft komutları, IP→MAC.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- zaman
/// Türkiye sabit UTC+3 (yaz saati yok). Kayıtlarda ISO 8601, saniye hassasiyeti, `+03:00`.
pub fn now_iso(ts: f64) -> String {
    let t = ts.floor() as i64 + 3 * 3600;
    let (days, sod) = (t.div_euclid(86400), t.rem_euclid(86400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+03:00", sod / 3600, sod % 3600 / 60, sod % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant, "chrono-compatible low-level date algorithms"
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn day_of(iso: &str) -> &str {
    &iso[..10]
}

pub fn wall() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Süreç içi monoton saat (sn). NTP sıçramalarından etkilenmez; OTP süreleri bununla ölçülür.
pub fn monotonic() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

// ---------------------------------------------------------------- doğrulayıcılar
/// Ad/soyad: boşluklar teke, 2–40 karakter, harfle başlar; yalnızca harf, boşluk, `-`, `'`, `.`.
pub fn clean_name(raw: &str) -> Option<String> {
    let s = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = s.chars().count();
    let first_ok = s.chars().next().is_some_and(char::is_alphabetic);
    let all_ok = s.chars().all(|c| c.is_alphabetic() || " -'.".contains(c));
    ((2..=40).contains(&n) && first_ok && all_ok).then_some(s)
}

pub fn norm_mac(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase().replace('-', ":");
    let ok = s.len() == 17
        && s.split(':').count() == 6
        && s.split(':').all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()));
    ok.then_some(s)
}

/// Köprünün iletim tablosundaki (`/sys/class/net/<köprü>/brforward`) misafir MAC'leri: son trafikten sonra
/// köprünün yaşlanma süresi (varsayılan 5 dk) boyunca kalır, yani "o an bağlı". Kayıt 16 bayt: MAC (6), port (1),
/// yerel mi (1), yaş (4), (4); köprünün kendi portları (yerel) sayılmaz.
pub fn kopruye_bagli(brforward: &[u8]) -> std::collections::BTreeSet<String> {
    brforward
        .chunks_exact(16)
        .filter(|k| k[7] == 0)
        .map(|k| k[..6].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"))
        .collect()
}

/// Klasörün bulunduğu diskin doluluğu (%), statvfs; okunamazsa `None`.
pub fn disk_percent(path: &str) -> Option<u64> {
    let p = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs yalnızca verilen yapıya yazar.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(p.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    Some(100 - (st.f_bavail as u64 * 100 / st.f_blocks as u64))
}

pub fn in_subnet(ip: &str, subnet: &str) -> bool {
    let (Ok(ip), Some((net, bits))) = (ip.parse::<std::net::Ipv4Addr>(), subnet.split_once('/')) else {
        return false;
    };
    let (Ok(net), Ok(bits)) = (net.parse::<std::net::Ipv4Addr>(), bits.parse::<u32>()) else {
        return false;
    };
    if bits > 32 {
        return false;
    }
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    u32::from(ip) & mask == u32::from(net) & mask
}

// ---------------------------------------------------------------- CSV (§14)
pub const CSV_HEADER: [&str; 19] = [
    "zaman", "olay", "telefon", "ad", "soyad", "mac", "ic_ip", "protokol", "ic_port", "hedef_ip",
    "hedef_port", "nat_ip", "nat_port", "alan_adi", "gonderilen_bayt", "alinan_bayt", "sure_sn",
    "oturum_id", "ek",
];
const BOM: &str = "\u{feff}"; // Türkçe Excel dosyayı doğru kodlamayla açsın

/// Ayraç ve satır kırıcıları temizler, formül enjeksiyonunu (=,+,-,@,TAB) etkisizleştirir (§14.6).
pub fn csv_cell(v: &str) -> String {
    let s: String = v
        .chars()
        .filter(|&c| c != '"')
        .map(|c| if matches!(c, '\r' | '\n' | ';') { ' ' } else { c })
        .collect();
    if s.starts_with(['=', '+', '-', '@', '\t']) {
        format!("'{s}")
    } else {
        s
    }
}

#[derive(Clone, Debug)]
pub struct Row(pub [String; 19]);

impl Row {
    pub fn new(olay: &str, zaman: &str) -> Row {
        let mut r = Row(Default::default());
        r.0[0] = zaman.to_string();
        r.0[1] = olay.to_string();
        r
    }

    /// Bilinmeyen sütun programlama hatasıdır.
    pub fn set(mut self, key: &str, val: impl Into<String>) -> Row {
        let i = CSV_HEADER.iter().position(|k| *k == key).unwrap_or_else(|| panic!("bilinmeyen sütun {key}"));
        self.0[i] = val.into();
        self
    }

    pub fn get(&self, key: &str) -> &str {
        CSV_HEADER.iter().position(|k| *k == key).map(|i| self.0[i].as_str()).unwrap_or("")
    }

    /// `ek` sütununun sonuna `anahtar=değer` ekler.
    pub fn ek_ekle(mut self, s: &str) -> Row {
        let ek = &mut self.0[18];
        if !ek.is_empty() {
            ek.push(' ');
        }
        ek.push_str(s);
        self
    }

    fn line(&self) -> String {
        let mut s = self.0.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(";");
        s.push_str("\r\n");
        s
    }
}

pub fn mkdirs(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

fn flock(f: &File) {
    // SAFETY: geçerli dosya tanıtıcısı; kilit dosya kapanınca bırakılır.
    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
}

/// Kilitli ekleme. Dosya yoksa 600 izinle BOM + başlıkla oluşur. Birden çok süreç aynı dosyaya güvenle yazar.
/// 5651 gün dosyasıysa ve saat bu açılışta doğrulanmadıysa satırlar bekletilir (src/saat.rs); doğrulanınca zamanları
/// düzeltilip doğru güne yazılır.
pub fn append_rows(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    if rows.is_empty() || crate::saat::biriktir_gerekirse(path, rows)? {
        return Ok(());
    }
    gun_dosyasina_ekle(path, rows)
}

/// `YYYY-MM-DD` biçiminde bir gün klasörü adı mı.
pub fn gun_adi(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })
}

/// `<kök>/gunluk/<gün>/<göreli>` (göreli: `dhcp.csv`, `kullanicilar/<kişi>.csv`).
pub struct GunHedefi {
    pub kok: PathBuf,
    pub gun: String,
    pub goreli: String,
}

pub fn gun_hedefi(path: &Path) -> Option<GunHedefi> {
    let p: Vec<String> = path.iter().map(|c| c.to_string_lossy().into_owned()).collect();
    let i = (0..p.len().saturating_sub(2)).rev().find(|&i| p[i] == "gunluk" && gun_adi(&p[i + 1]))?;
    Some(GunHedefi { kok: path.iter().take(i).collect(), gun: p[i + 1].clone(), goreli: p[i + 2..].join("/") })
}

/// Saati doğrulanmış yazım. Mühürlü günün klasörüne dosya açılmaz (MANIFEST'te olmayan dosya zincirin dışında kalırdı):
/// satır bugüne yazılır, asıl zamanı `ek`'te `asil_zaman=`.
pub fn gun_dosyasina_ekle(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    if let Some(h) = gun_hedefi(path) {
        if h.kok.join("gunluk").join(&h.gun).join("MANIFEST.sha256").exists() {
            let simdi = now_iso(wall());
            let rows: Vec<Row> = rows
                .iter()
                .map(|r| {
                    let mut r = r.clone().ek_ekle(&format!("asil_zaman={}", r.0[0]));
                    r.0[0] = simdi.clone();
                    r
                })
                .collect();
            return dosyaya_ekle(&h.kok.join("gunluk").join(day_of(&simdi)).join(&h.goreli), &rows);
        }
    }
    dosyaya_ekle(path, rows)
}

fn dosyaya_ekle(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        mkdirs(dir)?;
    }
    let mut f = OpenOptions::new().append(true).create(true).mode(0o600).open(path)?;
    flock(&f);
    let mut out = String::new();
    if f.metadata()?.len() == 0 {
        out.push_str(BOM);
        out.push_str(&CSV_HEADER.join(";"));
        out.push_str("\r\n");
    }
    for r in rows {
        out.push_str(&r.line());
    }
    f.write_all(out.as_bytes())
}

pub fn day_file(root: &str, day: &str, name: &str) -> PathBuf {
    Path::new(root).join("gunluk").join(day).join(name)
}

/// Kişinin günlük dosyası; kimlik anahtarında `:` → `_` (`TC_…csv`, `PASAPORT_DE_…csv`), telefon aynen.
pub fn user_file(root: &str, day: &str, phone: &str) -> PathBuf {
    Path::new(root).join("gunluk").join(day).join("kullanicilar").join(format!("{}.csv", crate::kimlik::dosya_adi(phone)))
}

/// Güne hâlâ satır eklenebilir mi: bugün ya da geleceği, ya da klasörü duran ve mühürlenmemiş geçmiş gün.
pub fn day_open(root: &str, day: &str, now: f64) -> bool {
    let d = Path::new(root).join("gunluk").join(day);
    day >= day_of(&now_iso(now)) || (d.is_dir() && !d.join("MANIFEST.sha256").exists())
}

/// Satırları zamanlarının gününe göre kişinin günlük dosyasına yazar; dosya o gün ilk kez açılıyorsa
/// başa KAYIT (ad, soyad, MAC) koyar — her günlük dosya kendi başına okunabilsin.
pub fn append_user_rows(root: &str, s: &Session, mac: &str, rows: &[Row]) -> std::io::Result<()> {
    let mut by_day: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for r in rows {
        by_day.entry(day_of(r.get("zaman")).to_string()).or_default().push(r.clone());
    }
    for (day, mut list) in by_day {
        let path = user_file(root, &day, &s.phone);
        // saat doğrulanmadıysa satırların düşeceği gün henüz bilinmez: KAYIT kişi başına bir kez bekletilir (src/saat.rs)
        if crate::saat::kayit_bekletmesi(&path).unwrap_or_else(|| !path.exists()) {
            // ponytail: iki süreç aynı anda açarsa KAYIT iki kez yazılabilir; zararsız
            let kayit = Row::new("KAYIT", &list[0].0[0].clone())
                .set("telefon", &s.phone)
                .set("ad", &s.ad)
                .set("soyad", &s.soyad)
                .set("mac", mac)
                .set("ic_ip", &s.ip)
                .set("oturum_id", &s.session_id);
            list.insert(0, kayit);
        }
        append_rows(&path, &list)?;
    }
    Ok(())
}

pub fn audit(root: &str, row: Row) {
    let path = day_file(root, day_of(row.get("zaman")), "denetim.csv");
    if let Err(e) = append_rows(&path, &[row]) {
        eprintln!("denetim kaydı yazılamadı: {e}");
    }
}

// ---------------------------------------------------------------- kişi listesi (çağıran state_lock tutar)
const INDEX_HEADER: [&str; 5] = ["telefon", "ad", "soyad", "ilk_kayit", "son_oturum"];

fn index_path(root: &str) -> PathBuf {
    Path::new(root).join("kullanicilar").join("index.csv")
}

pub fn read_index(root: &str) -> BTreeMap<String, [String; 5]> {
    let text = fs::read_to_string(index_path(root)).unwrap_or_default();
    let mut out = BTreeMap::new();
    for line in text.trim_start_matches(BOM).lines().skip(1) {
        let v: Vec<&str> = line.split(';').collect();
        if v.len() == 5 {
            out.insert(v[0].to_string(), [0, 1, 2, 3, 4].map(|i| v[i].to_string()));
        }
    }
    out
}

fn write_index(root: &str, rows: &BTreeMap<String, [String; 5]>) -> std::io::Result<()> {
    let mut out = format!("{BOM}{}\r\n", INDEX_HEADER.join(";"));
    for r in rows.values() {
        out.push_str(&r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(";"));
        out.push_str("\r\n");
    }
    write_atomic(&index_path(root), out.as_bytes())
}

pub fn upsert_index(root: &str, phone: &str, ad: &str, soyad: &str, zaman: &str) -> std::io::Result<()> {
    let mut rows = read_index(root);
    let first = rows.get(phone).map(|r| r[3].clone()).unwrap_or_else(|| zaman.to_string());
    rows.insert(phone.into(), [phone.into(), ad.into(), soyad.into(), first, zaman.into()]);
    write_index(root, &rows)
}

/// Saat doğrulanınca (src/saat.rs): bekletilen girişin zamanı kişi listesinde de düzeltilir. (telefon, ham, düzeltilmiş)
pub fn index_zaman_duzelt(root: &str, duzeltmeler: &[(String, String, String)]) -> std::io::Result<()> {
    if duzeltmeler.is_empty() {
        return Ok(());
    }
    let mut rows = read_index(root);
    let mut degisti = false;
    for (tel, ham, yeni) in duzeltmeler {
        if let Some(r) = rows.get_mut(tel) {
            for i in [3, 4] {
                if r[i] == *ham {
                    r[i] = yeni.clone();
                    degisti = true;
                }
            }
        }
    }
    if degisti {
        write_index(root, &rows)?;
    }
    Ok(())
}

pub fn remove_from_index(root: &str, phones: &[String]) -> std::io::Result<()> {
    let mut rows = read_index(root);
    for p in phones {
        rows.remove(p);
    }
    write_index(root, &rows)
}

// ---------------------------------------------------------------- durum dosyaları
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub phone: String,
    pub ad: String,
    pub soyad: String,
    /// Boşsa oturum IP'siz bekliyor (IP başka cihaza verildi; cihaz dönünce yeni IP'sine taşınır).
    pub ip: String,
    pub session_id: String,
    pub start: String,
    pub start_epoch: f64,
    pub expires_epoch: f64,
}

pub type Sessions = BTreeMap<String, Session>;

/// Durum klasörü kilidi; düşünce bırakılır.
pub struct StateLock(#[allow(dead_code)] File);

pub fn state_lock(state_root: &str) -> std::io::Result<StateLock> {
    mkdirs(Path::new(state_root))?;
    let f = OpenOptions::new().append(true).create(true).mode(0o600).open(Path::new(state_root).join(".lock"))?;
    flock(&f);
    Ok(StateLock(f))
}

/// Herhangi bir kilit dosyası (600); düşünce bırakılır.
pub fn dosya_kilidi(path: &Path) -> std::io::Result<StateLock> {
    if let Some(d) = path.parent() {
        mkdirs(d)?;
    }
    let f = OpenOptions::new().append(true).create(true).mode(0o600).open(path)?;
    flock(&f);
    Ok(StateLock(f))
}

pub fn sessions_path(state_root: &str) -> PathBuf {
    Path::new(state_root).join("sessions.json")
}

/// Eksik, boş veya bozuk dosya → boş (elektrik kesintisinde servisler çökmesin).
pub fn load_sessions(state_root: &str) -> Sessions {
    fs::read(sessions_path(state_root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_sessions(state_root: &str, s: &Sessions) -> std::io::Result<()> {
    write_atomic(&sessions_path(state_root), &serde_json::to_vec_pretty(s).unwrap_or_default())
}

/// Geçici dosyaya yaz + fsync + yeniden adlandır (yarım dosya kalmaz).
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_atomic_mode(path, data, 0o600)
}

/// `write_atomic`, dosya izni `mode` (geçici dosya bu izinle yazılır, yeniden adlandırmadan sonra izin değişmez:
/// okuyan süreç dosyayı hiçbir an yanlış izinle görmez).
pub fn write_atomic_mode(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        mkdirs(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)
}

// ---------------------------------------------------------------- bağlı süresi (özet)
/// 15 dk'dan uzun ağda görülmeyen cihazın bağlı süresi sıfırlanır; daha kısa kopmada kaldığı yerden sürer.
pub const VARLIK_ARA_SN: f64 = 900.0;
/// Son görülme bu kadar süre saklanır (oturum süresi en çok 45 gün).
pub const VARLIK_SAKLA_SN: f64 = 45.0 * 86_400.0;

/// Son görülme: bugünse "14:02", değilse "12.10.2026" (Türkiye saati).
pub fn son_gorulme(son: f64, now: f64) -> String {
    let (iso, bugun) = (now_iso(son), now_iso(now));
    if iso[..10] == bugun[..10] {
        iso[11..16].to_string()
    } else {
        tarih_goster(&iso[..10])
    }
}

/// Ekranda gösterilen tarih (gün.ay.yıl): "2026-10-12" → "12.10.2026"; "2026-10-12T17:38:05+03:00" ya da
/// "2026-10-12 17:38" → "12.10.2026 17:38". Çözülemeyen değer aynen döner. Kayıtlar, adresler ve dosyalar ISO kalır.
pub fn tarih_goster(iso: &str) -> String {
    tarih_goster_(iso, false)
}

/// Saniyeli gösterim: "2026-10-12T17:38:05+03:00" → "12.10.2026 17:38:05" (saniye yoksa dakikaya kadar).
pub fn tarih_saat_goster(iso: &str) -> String {
    tarih_goster_(iso, true)
}

fn rakamlar(s: &str, r: std::ops::Range<usize>) -> bool {
    s.get(r).is_some_and(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit()))
}

fn tarih_goster_(iso: &str, saniye: bool) -> String {
    let s = iso.trim();
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' || !rakamlar(s, 0..4) || !rakamlar(s, 5..7) || !rakamlar(s, 8..10) {
        return iso.to_string();
    }
    let gun = format!("{}.{}.{}", &s[8..10], &s[5..7], &s[..4]);
    if b.len() == 10 {
        return gun;
    }
    if !matches!(b[10], b'T' | b' ') || b.len() < 16 || b[13] != b':' || !rakamlar(s, 11..13) || !rakamlar(s, 14..16) {
        return iso.to_string();
    }
    if saniye && b.len() >= 19 && b[16] == b':' && rakamlar(s, 17..19) {
        format!("{gun} {}", &s[11..19])
    } else {
        format!("{gun} {}", &s[11..16])
    }
}

/// Kişinin yazdığı tarih: "12.10.2026" / "1.9.2026" / "12.10.2026 17:38[:05]" → ISO "2026-10-12[ 17:38[:05]]".
/// Başka her şey (eski ISO yazımı dahil) kırpılıp aynen döner; geçerliliği çağıran denetler.
pub fn tarih_girdi(s: &str) -> String {
    let s = s.trim();
    let (gun, saat) = match s.split_once(' ') {
        Some((g, z)) => (g, Some(z.trim())),
        None => (s, None),
    };
    let p: Vec<&str> = gun.split('.').collect();
    let iki = |x: &str| (1..=2).contains(&x.len()) && rakamlar(x, 0..x.len());
    if p.len() != 3 || !iki(p[0]) || !iki(p[1]) || p[2].len() != 4 || !rakamlar(p[2], 0..4) {
        return s.to_string();
    }
    let iso = format!("{}-{:0>2}-{:0>2}", p[2], p[1], p[0]);
    match saat {
        Some(z) => format!("{iso} {z}"),
        None => iso,
    }
}

/// Cihazın (MAC) ağda kesintisiz sayılan görünümü: başlangıç ve son görülme (epoch).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Varlik {
    pub baslangic: f64,
    pub son: f64,
}

fn varlik_yolu(state_root: &str) -> PathBuf {
    Path::new(state_root).join("varlik.json")
}

/// Eksik ya da bozuk dosya → boş.
pub fn varlik_oku(state_root: &str) -> BTreeMap<String, Varlik> {
    fs::read(varlik_yolu(state_root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Kaydedici her bakımda çağırır: `gorulen` (oturumu süren ve köprüde görülen MAC'ler) için son görülme ilerler;
/// 15 dk'dan uzun ara verilmişse başlangıç şimdi olur. 45 gündür görülmeyenler unutulur (oturum en çok 45 gün; son görülme).
pub fn varlik_guncelle(state_root: &str, gorulen: &std::collections::BTreeSet<String>, now: f64) -> std::io::Result<()> {
    let mut v = varlik_oku(state_root);
    for mac in gorulen {
        let e = v.entry(mac.clone()).or_insert(Varlik { baslangic: now, son: now });
        if now - e.son > VARLIK_ARA_SN || e.son > now {
            e.baslangic = now;
        }
        e.son = now;
    }
    v.retain(|_, e| now - e.son <= VARLIK_SAKLA_SN);
    write_atomic(&varlik_yolu(state_root), &serde_json::to_vec(&v).unwrap_or_default())
}

/// Özette gösterilen bağlı dakika: kayıt yoksa ya da 15 dk'dan uzun görülmemişse 0.
pub fn bagli_dakika(v: Option<&Varlik>, now: f64) -> u64 {
    match v {
        Some(e) if now - e.son <= VARLIK_ARA_SN && now >= e.baslangic => ((now - e.baslangic) / 60.0) as u64,
        _ => 0,
    }
}

#[derive(Serialize, Deserialize)]
struct SmsCounter {
    tarih: String,
    adet: u64,
}

fn sms_path(state_root: &str) -> PathBuf {
    Path::new(state_root).join("sms_sayac.json")
}

/// Günlük toplam SMS sayacı diskte: süreç yeniden başlasa da tavan korunur.
pub fn sms_count(state_root: &str, day: &str) -> u64 {
    fs::read(sms_path(state_root))
        .ok()
        .and_then(|b| serde_json::from_slice::<SmsCounter>(&b).ok())
        .filter(|c| c.tarih == day)
        .map_or(0, |c| c.adet)
}

pub fn sms_count_inc(state_root: &str, day: &str) {
    let c = SmsCounter { tarih: day.into(), adet: sms_count(state_root, day) + 1 };
    if let Err(e) = write_atomic(&sms_path(state_root), &serde_json::to_vec(&c).unwrap_or_default()) {
        eprintln!("SMS sayacı yazılamadı: {e}");
    }
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------- nft / sistem komutları
pub type Cmd = Vec<String>;

fn checked(mac: &str, ip: &str) -> bool {
    norm_mac(mac).as_deref() == Some(mac) && ip.parse::<std::net::Ipv4Addr>().is_ok()
}

pub fn nft_add_cmd(mac: &str, ip: &str, seconds: f64) -> Option<Cmd> {
    checked(mac, ip).then(|| {
        let secs = (seconds as i64).max(1);
        ["nft", "add", "element", "inet", "hotspot", "auth"]
            .iter()
            .map(|s| s.to_string())
            .chain([format!("{{ {mac} . {ip} timeout {secs}s }}")])
            .collect()
    })
}

pub fn nft_del_cmd(mac: &str, ip: &str) -> Option<Cmd> {
    checked(mac, ip).then(|| {
        ["nft", "delete", "element", "inet", "hotspot", "auth"]
            .iter()
            .map(|s| s.to_string())
            .chain([format!("{{ {mac} . {ip} }}")])
            .collect()
    })
}

/// `allow_mac` / `ban_mac` kümesine ekle (`add`) ya da çıkar (`delete`).
pub fn nft_set_cmd(verb: &str, set: &str, mac: &str) -> Option<Cmd> {
    (norm_mac(mac).as_deref() == Some(mac) && matches!(set, "allow_mac" | "ban_mac") && matches!(verb, "add" | "delete")).then(|| {
        ["nft", verb, "element", "inet", "hotspot", set].iter().map(|s| s.to_string()).chain([format!("{{ {mac} }}")]).collect()
    })
}

/// Komutu kabuksuz çalıştırır, 15 sn içinde bitmezse öldürür. Başarı = çıkış kodu 0.
pub fn run(cmd: &[String]) -> bool {
    run_timeout(cmd, 15)
}

/// Komutun standart çıktısı (kabuksuz; hata → boş).
pub fn capture(cmd: &[&str]) -> String {
    let Some((prog, args)) = cmd.split_first() else { return String::new() };
    Command::new(prog).args(args).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// `capture`, süre sınırlı: komut `secs` içinde bitmezse öldürülür → `None` (çalıştırılamadı da `None`). Çıkış koduna bakmaz
/// (hata koduyla biten komutun çıktısı da döner); bakması gerekene `capture_timeout_basarili`.
pub fn capture_timeout(cmd: &[&str], secs: u64) -> Option<String> {
    capture_timeout_durum(cmd, secs).map(|(_, cikti)| cikti)
}

/// `capture_timeout`, ama komut sıfırdan farklı kodla çıkarsa da `None`: "çalıştı ama sonuç alınamadı" ile "çıktı boş" ayrılsın.
/// Ör. `ip -o route show default` netlink hatasında boş çıktı ve hata koduyla biter; bu "varsayılan rota yok" sayılmamalı
/// (giriş sayfası internet varken Servis'i açardı).
pub fn capture_timeout_basarili(cmd: &[&str], secs: u64) -> Option<String> {
    capture_timeout_durum(cmd, secs).and_then(|(basarili, cikti)| basarili.then_some(cikti))
}

/// İkisinin ortak gövdesi: (çıkış kodu 0 mı, standart çıktı). Çıktı ayrı iş parçacığında okunur (büyük çıktı boruyu doldurup
/// komutu bekletmesin).
fn capture_timeout_durum(cmd: &[&str], secs: u64) -> Option<(bool, String)> {
    use std::io::Read;
    let (prog, args) = cmd.split_first()?;
    let mut child = Command::new(prog).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let okuyucu = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    let basarili = loop {
        match child.try_wait() {
            Ok(Some(durum)) => break durum.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = okuyucu.join();
                return None;
            }
        }
    };
    okuyucu.join().ok().map(|b| (basarili, String::from_utf8_lossy(&b).into_owned()))
}

pub fn run_timeout(cmd: &[String], secs: u64) -> bool {
    let Some((prog, args)) = cmd.split_first() else { return false };
    let Ok(mut child) = Command::new(prog).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub type Runner = dyn Fn(&[String]) -> bool + Send + Sync;

pub fn run_opt(runner: &Runner, cmd: Option<Cmd>) -> bool {
    cmd.is_some_and(|c| runner(&c))
}

// ---------------------------------------------------------------- oturum olayları (çağıran state_lock tutar, sessions'ı kaydeder)
const KEEP_CONNS: [&str; 3] = ["sure_doldu", "yeniden_giris", "yukleme_hatasi"];

fn session_row(olay: &str, zaman: &str, s: &Session, mac: &str) -> Row {
    Row::new(olay, zaman).set("telefon", &s.phone).set("mac", mac).set("ic_ip", &s.ip).set("oturum_id", &s.session_id)
}

fn write_session_row(root: &str, s: &Session, mac: &str, row: Row) {
    let day = day_of(row.get("zaman")).to_string();
    if let Err(e) = append_user_rows(root, s, mac, &[row.clone()])
        .and_then(|_| append_rows(&day_file(root, &day, "oturum.csv"), &[row]))
    {
        eprintln!("oturum kaydı yazılamadı: {e}");
    }
}

/// Oturumu kapatır ve OTURUM_BITIS yazar.
pub fn close_session(root: &str, sessions: &mut Sessions, mac: &str, reason: &str, now: f64, runner: &Runner) -> Option<Session> {
    let s = sessions.remove(mac)?;
    let end = if reason == "sure_doldu" { now.min(s.expires_epoch) } else { now };
    if !s.ip.is_empty() {
        run_opt(runner, nft_del_cmd(mac, &s.ip));
        if !KEEP_CONNS.contains(&reason) {
            runner(&["conntrack".into(), "-D".into(), "-s".into(), s.ip.clone()]);
        }
    }
    let (mut zaman, mut ek) = (now_iso(end), format!("neden={reason}"));
    if !day_open(root, day_of(&zaman), now) {
        // o gün mühürlendi: bugüne yaz, gerçek bitişi ek'te ver
        ek = format!("{ek} bitis={zaman}");
        zaman = now_iso(now);
    }
    let row = session_row("OTURUM_BITIS", &zaman, &s, mac)
        .set("ad", &s.ad)
        .set("soyad", &s.soyad)
        .set("sure_sn", ((end - s.start_epoch) as i64).to_string())
        .set("ek", ek);
    write_session_row(root, &s, mac, row);
    Some(s)
}

fn ip_row(root: &str, s: &Session, mac: &str, old_ip: &str, now: f64) {
    let row = session_row("OTURUM_IP_DEGISTI", &now_iso(now), s, mac).set("ek", format!("eski_ip={old_ip}"));
    write_session_row(root, s, mac, row);
}

/// `ip`'yi tutan başka oturum varsa o cihaz artık o IP'de değildir: nft kaydı silinir, oturum IP'siz beklemeye alınır.
pub fn park_ip_holders(root: &str, sessions: &mut Sessions, mac: &str, ip: &str, now: f64, runner: &Runner) {
    for (other, s) in sessions.iter_mut() {
        if other != mac && s.ip == ip {
            run_opt(runner, nft_del_cmd(other, ip));
            s.ip.clear();
            ip_row(root, s, other, ip, now);
        }
    }
}

/// Süren oturumu cihazın yeni IP'sine taşır (SMS istemeden). nft başarısızsa hiçbir şey değişmez.
pub fn move_session(root: &str, sessions: &mut Sessions, mac: &str, new_ip: &str, now: f64, runner: &Runner) -> bool {
    let Some(left) = sessions.get(mac).map(|s| s.expires_epoch - now) else { return false };
    if !run_opt(runner, nft_add_cmd(mac, new_ip, left)) {
        return false;
    }
    park_ip_holders(root, sessions, mac, new_ip, now, runner);
    let s = sessions.get_mut(mac).expect("oturum var");
    let old_ip = std::mem::replace(&mut s.ip, new_ip.to_string());
    if !old_ip.is_empty() {
        run_opt(runner, nft_del_cmd(mac, &old_ip));
    }
    let s = s.clone();
    ip_row(root, &s, mac, &old_ip, now);
    true
}

/// Süren oturumun bitişini değiştirir (misafir kodu uzatıldı). nft öğesinin süresi yerinde değiştirilemez
/// (`add` var olan öğeye dokunmaz, `update` yalnızca kural içinde): sil + yeniden ekle **tek nft çağrısında** (`;` ile
/// ayrılmış komutlar tek işlem/batch olarak uygulanır: ya ikisi birden ya hiçbiri; arada boşluk yok). Başarısızsa eski öğe
/// yerinde kalır, oturum değişmez → false. IP'siz oturumda yalnızca kayıt. Süre = yeni bitiş − şimdi (koddan uzun olamaz).
pub fn oturum_uzat(sessions: &mut Sessions, mac: &str, yeni_bitis: f64, now: f64, runner: &Runner) -> bool {
    let Some(s) = sessions.get_mut(mac) else { return false };
    if !s.ip.is_empty() && !run_opt(runner, nft_uzat_cmd(mac, &s.ip, yeni_bitis - now)) {
        return false;
    }
    s.expires_epoch = yeni_bitis;
    true
}

/// Uzatılan oturum için `OTURUM_UZATILDI` (oturum.csv + kişi dosyası). Resmi talep IP araması oturumun hâlâ açık
/// olduğunu bu satırdan da anlar: geriye bakış en son uzatmaya ulaşsa yeter.
pub fn oturum_uzatildi_yaz(root: &str, s: &Session, mac: &str, now: f64, ureten: &str) {
    let row = session_row("OTURUM_UZATILDI", &now_iso(now), s, mac)
        .set("ad", &s.ad)
        .set("soyad", &s.soyad)
        .set("ek", format!("yeni_bitis={} ureten={}", now_iso(s.expires_epoch), ureten.replace(char::is_whitespace, "_")));
    write_session_row(root, s, mac, row);
}

/// `nft delete element … ; add element … timeout Ns` — tek çağrı, tek işlem (atomik).
pub fn nft_uzat_cmd(mac: &str, ip: &str, seconds: f64) -> Option<Cmd> {
    let mut c = nft_del_cmd(mac, ip)?;
    c.push(";".into());
    c.extend(nft_add_cmd(mac, ip, seconds)?.into_iter().skip(1)); // "nft" bir kez
    Some(c)
}

// ---------------------------------------------------------------- IP → MAC
/// dnsmasq kira dosyası: `bitis_epoch mac ip hostname clientid`.
pub fn parse_leases(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 3).then(|| norm_mac(p[1]).map(|m| (p[2].to_string(), m))).flatten()
        })
        .collect()
}

/// /proc/net/arp; yalnızca tamamlanmış (0x2) girdiler.
pub fn parse_arp(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 4 && p[2] == "0x2").then(|| norm_mac(p[3]).map(|m| (p[0].to_string(), m))).flatten()
        })
        .collect()
}

pub fn lookup_mac(ip: &str, leases_file: &str) -> Option<String> {
    let leases = fs::read_to_string(leases_file).unwrap_or_default();
    parse_leases(&leases)
        .remove(ip)
        .or_else(|| parse_arp(&fs::read_to_string("/proc/net/arp").unwrap_or_default()).remove(ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bagli_suresi_15_dk_kurali() {
        let d = std::env::temp_dir().join(format!("wfc-varlik-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let root = d.to_string_lossy().to_string();
        let a = std::collections::BTreeSet::from(["aa:bb:cc:dd:ee:01".to_string()]);
        let bos = std::collections::BTreeSet::new();
        let t0 = 1_790_000_000.0;
        varlik_guncelle(&root, &a, t0).unwrap();
        varlik_guncelle(&root, &a, t0 + 600.0).unwrap();
        assert_eq!(bagli_dakika(varlik_oku(&root).get("aa:bb:cc:dd:ee:01"), t0 + 600.0), 10);
        // 14 dk görülmedi (listeden düştü), döndü: kaldığı yerden sürer
        varlik_guncelle(&root, &bos, t0 + 1000.0).unwrap();
        varlik_guncelle(&root, &a, t0 + 600.0 + 840.0).unwrap();
        assert_eq!(bagli_dakika(varlik_oku(&root).get("aa:bb:cc:dd:ee:01"), t0 + 1440.0), 24);
        // 16 dk görülmedi: sayaç sıfırlanır
        varlik_guncelle(&root, &a, t0 + 1440.0 + 960.0).unwrap();
        assert_eq!(bagli_dakika(varlik_oku(&root).get("aa:bb:cc:dd:ee:01"), t0 + 2400.0 + 120.0), 2);
        // kayıt yok ya da 15 dk'dan eski kayıt: 0
        assert_eq!(bagli_dakika(None, t0), 0);
        assert_eq!(bagli_dakika(varlik_oku(&root).get("aa:bb:cc:dd:ee:01"), t0 + 2400.0 + 1000.0), 0);
        // 45 gündür görülmeyen unutulur; bozuk dosya boş sayılır
        varlik_guncelle(&root, &bos, t0 + 2400.0 + 86_401.0).unwrap();
        assert_eq!(varlik_oku(&root).len(), 1, "bir gün sonra hâlâ var (son görülme)");
        varlik_guncelle(&root, &bos, t0 + 2400.0 + VARLIK_SAKLA_SN + 1.0).unwrap();
        assert!(varlik_oku(&root).is_empty());
        fs::write(d.join("varlik.json"), "bozuk").unwrap();
        assert!(varlik_oku(&root).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn son_gorulme_bicimi() {
        let now = 1_790_705_134.0; // 2026-09-29 21:05 TR
        assert_eq!(son_gorulme(now - 3600.0, now), "20:05");
        assert_eq!(son_gorulme(now - 86_400.0, now), "28.09.2026");
        assert_eq!(son_gorulme(now - 21.0 * 3600.0, now), "00:05");
    }

    #[test]
    fn tarih_gosterimi() {
        assert_eq!(tarih_goster("2026-10-12"), "12.10.2026");
        assert_eq!(tarih_goster("2026-10-12T17:38:05+03:00"), "12.10.2026 17:38");
        assert_eq!(tarih_goster("2026-10-12 17:38"), "12.10.2026 17:38");
        assert_eq!(tarih_saat_goster("2026-10-12T17:38:05+03:00"), "12.10.2026 17:38:05");
        assert_eq!(tarih_saat_goster("2026-10-12 17:38"), "12.10.2026 17:38");
        assert_eq!(tarih_saat_goster("2026-10-12"), "12.10.2026");
        // çözülemeyen aynen
        for bozuk in ["", "dün", "2026-1-12", "12.10.2026", "2026-10-12X17:38", "2026-10-12T17", "2026-10-1ü", "abcd-ef-gh"] {
            assert_eq!(tarih_goster(bozuk), bozuk);
            assert_eq!(tarih_saat_goster(bozuk), bozuk);
        }
    }

    #[test]
    fn tarih_girdisi_iki_bicim() {
        assert_eq!(tarih_girdi("12.10.2026"), "2026-10-12");
        assert_eq!(tarih_girdi(" 1.9.2026 "), "2026-09-01");
        assert_eq!(tarih_girdi("12.10.2026 17:38"), "2026-10-12 17:38");
        assert_eq!(tarih_girdi("12.10.2026 17:38:05"), "2026-10-12 17:38:05");
        assert_eq!(tarih_girdi("2026-10-12 17:38"), "2026-10-12 17:38"); // eski yazım aynen
        assert_eq!(tarih_girdi("2026-10-12"), "2026-10-12");
        for bozuk in ["dün", "12.10.26", "123.10.2026", "12.10", "a.b.cdef", ""] {
            assert_eq!(tarih_girdi(bozuk), bozuk);
        }
    }

    #[test]
    fn time_format() {
        assert_eq!(now_iso(0.0), "1970-01-01T03:00:00+03:00");
        assert_eq!(now_iso(1_790_705_134.9), "2026-09-29T21:05:34+03:00");
        assert_eq!(now_iso(951_782_400.0), "2000-02-29T03:00:00+03:00"); // artık yıl
        assert_eq!(day_of("2026-09-29T21:05:34+03:00"), "2026-09-29");
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("  Ayşe  ").as_deref(), Some("Ayşe"));
        assert_eq!(clean_name("İsmail   Hakkı").as_deref(), Some("İsmail Hakkı"));
        assert_eq!(clean_name("O'Neil").as_deref(), Some("O'Neil"));
        for bad in ["A", "=cmd", "Ali1", "-Ali", &"a".repeat(41)] {
            assert_eq!(clean_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn macs_and_subnets() {
        assert_eq!(norm_mac("AA-BB-cc-dd-ee-ff").as_deref(), Some("aa:bb:cc:dd:ee:ff"));
        assert_eq!(norm_mac("aa:bb:cc:dd:ee"), None);
        assert_eq!(norm_mac("aa:bb:cc:dd:ee:fg"), None);
        assert!(in_subnet("10.50.0.23", "10.50.0.0/24"));
        assert!(!in_subnet("10.50.1.23", "10.50.0.0/24"));
        assert!(!in_subnet("bozuk", "10.50.0.0/24"));
    }

    #[test]
    fn csv_cells() {
        assert_eq!(csv_cell("=HYPERLINK(\"x\")"), "'=HYPERLINK(x)");
        assert_eq!(csv_cell("a;b\nc\r\"d"), "a b c d");
        assert_eq!(csv_cell("-1"), "'-1");
        assert_eq!(csv_cell(""), "");
    }

    #[test]
    fn gun_hedefi_cozumlenir() {
        let h = gun_hedefi(Path::new("/srv/5651/gunluk/2026-10-26/kullanicilar/905334553132.csv")).unwrap();
        assert_eq!((h.kok, h.gun.as_str(), h.goreli.as_str()), (PathBuf::from("/srv/5651"), "2026-10-26", "kullanicilar/905334553132.csv"));
        let h = gun_hedefi(&day_file("/srv/5651", "2026-04-13", "denetim.csv")).unwrap();
        assert_eq!((h.kok, h.goreli.as_str()), (PathBuf::from("/srv/5651"), "denetim.csv"));
        assert!(gun_hedefi(Path::new("/srv/5651/kullanicilar/index.csv")).is_none());
        assert!(gun_hedefi(Path::new("/srv/5651/gunluk/bekleyen/x.csv")).is_none());
        assert!(gun_adi("2026-10-26") && !gun_adi("2026-1-026") && !gun_adi("2026/10/26") && !gun_adi("bekleyen"));
    }

    #[test]
    fn muhurlu_gune_dosya_acilmaz_bugune_yazilir() {
        let root = std::env::temp_dir().join(format!("wfc-ortak-muhur-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let kok = root.to_string_lossy().to_string();
        let d = root.join("gunluk/2026-09-27");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("MANIFEST.sha256"), "# gun=2026-09-27\n").unwrap();
        let r = Row::new("OTURUM_BITIS", "2026-09-27T23:59:59+03:00").set("ek", "neden=sure_doldu");
        append_rows(&day_file(&kok, "2026-09-27", "oturum.csv"), &[r]).unwrap();
        assert!(!d.join("oturum.csv").exists());
        let bugun = fs::read_to_string(day_file(&kok, day_of(&now_iso(wall())), "oturum.csv")).unwrap();
        assert!(bugun.contains("neden=sure_doldu asil_zaman=2026-09-27T23:59:59+03:00"), "{bugun}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nft_commands_validate() {
        let c = nft_add_cmd("aa:bb:cc:dd:ee:ff", "10.50.0.23", 2_592_000.0).unwrap();
        assert_eq!(c.last().unwrap(), "{ aa:bb:cc:dd:ee:ff . 10.50.0.23 timeout 2592000s }");
        assert!(nft_add_cmd("aa:bb:cc:dd:ee:ff; reboot", "10.50.0.23", 1.0).is_none());
        assert!(nft_del_cmd("aa:bb:cc:dd:ee:ff", "10.50.0.23 }").is_none());
    }

    #[test]
    fn leases_and_arp() {
        let leases = "1790952863 F4:3B:D8:55:FD:3E 10.50.0.147 DESKTOP 01:f4\nbozuk\n";
        assert_eq!(parse_leases(leases).get("10.50.0.147").map(String::as_str), Some("f4:3b:d8:55:fd:3e"));
        let arp = "IP address HW type Flags HW address Mask Device\n10.50.0.9 0x1 0x2 aa:bb:cc:dd:ee:01 * br-hotspot\n10.50.0.8 0x1 0x0 00:00:00:00:00:00 * br-hotspot\n";
        let m = parse_arp(arp);
        assert_eq!(m.get("10.50.0.9").map(String::as_str), Some("aa:bb:cc:dd:ee:01"));
        assert!(!m.contains_key("10.50.0.8"));
    }

    #[test]
    fn capture_timeout_suresi_dolunca_none() {
        let t0 = Instant::now();
        assert_eq!(capture_timeout(&["sleep", "3"], 1), None); // asılı komut döngüyü bekletmez
        assert!(t0.elapsed() < Duration::from_millis(2500), "{:?}", t0.elapsed());
        assert_eq!(capture_timeout(&["echo", "merhaba"], 5).as_deref(), Some("merhaba\n"));
        assert_eq!(capture_timeout(&["/yok/komut"], 5), None);
        // çıkış koduna bakmaz (öbür çağıranlar için davranış aynı): hata koduyla biten komutun çıktısı da döner
        assert_eq!(capture_timeout(&["sh", "-c", "echo yarim; exit 2"], 5).as_deref(), Some("yarim\n"));
        assert_eq!(capture_timeout(&["sh", "-c", "exit 1"], 5).as_deref(), Some(""));
    }

    #[test]
    fn capture_timeout_basarili_hata_kodunda_none() {
        // çalıştı ama hata koduyla bitti (ör. netlink hatasında `ip`: boş çıktı, kod 2): sonuç yok, "çıktı boş" sayılmaz
        assert_eq!(capture_timeout_basarili(&["sh", "-c", "exit 2"], 5), None);
        assert_eq!(capture_timeout_basarili(&["sh", "-c", "echo 'default dev wfcppp scope link metric 100'; exit 1"], 5), None);
        assert_eq!(capture_timeout_basarili(&["false"], 5), None);
        // başarılı: boş çıktı da bir sonuçtur
        assert_eq!(capture_timeout_basarili(&["true"], 5).as_deref(), Some(""));
        assert_eq!(capture_timeout_basarili(&["echo", "merhaba"], 5).as_deref(), Some("merhaba\n"));
        // çalıştırılamadı / süresi doldu
        assert_eq!(capture_timeout_basarili(&["/yok/komut"], 5), None);
        let t0 = Instant::now();
        assert_eq!(capture_timeout_basarili(&["sleep", "3"], 1), None);
        assert!(t0.elapsed() < Duration::from_millis(2500), "{:?}", t0.elapsed());
    }
}
