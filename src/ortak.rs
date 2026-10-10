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
pub fn append_rows(path: &Path, rows: &[Row]) -> std::io::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
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

pub fn user_file(root: &str, day: &str, phone: &str) -> PathBuf {
    Path::new(root).join("gunluk").join(day).join("kullanicilar").join(format!("{phone}.csv"))
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
        if !path.exists() {
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
    if let Some(dir) = path.parent() {
        mkdirs(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)
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

/// `capture`, süre sınırlı: komut `secs` içinde bitmezse öldürülür → `None` (çalıştırılamadı da `None`).
/// Çıktı ayrı iş parçacığında okunur (büyük çıktı boruyu doldurup komutu bekletmesin).
pub fn capture_timeout(cmd: &[&str], secs: u64) -> Option<String> {
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
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = okuyucu.join();
                return None;
            }
        }
    }
    okuyucu.join().ok().map(|b| String::from_utf8_lossy(&b).into_owned())
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
    }
}
