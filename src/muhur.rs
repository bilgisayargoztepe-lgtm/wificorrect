//! Mühürleme, doğrulama, saklama ve uzak yedek (eski ctl.py; docs/MASTER_ENGINEERING.md §15, RUST_YENIDEN_YAZIM.md A8).
//! 00:15 gun-kapat: önceki mühürsüz her gün → *.csv.gz + MANIFEST.sha256 + hash zinciri (zincir.txt), dosyalar salt okunur.
//! 02:00 yedekle (mühürlü, gönderilmemiş günler) + temizle (2 yılı aşmış ve yedeklenmiş günler).
//! RFC 3161 zaman damgası sağlayıcı seçilince eklenecek (eski sistemde de kapalıydı).

use crate::ayar::Config;
use crate::ortak::{self, Row, Runner};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const ZERO: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn chain_hash(prev: &str, man_sha: &str) -> String {
    hex(&Sha256::digest(format!("{prev}{man_sha}").as_bytes()))
}

/// Sıkıştır, sıkıştırılmışı baştan sona oku (bütünlük testi), sonra orijinali sil. Hata olursa orijinal kalır.
fn gzip_file(path: &Path) -> std::io::Result<()> {
    let gz = PathBuf::from(format!("{}.gz", path.display()));
    let tmp = PathBuf::from(format!("{}.gz.tmp", path.display()));
    {
        let mut src = fs::File::open(path)?;
        let mut enc = flate2::write::GzEncoder::new(fs::File::create(&tmp)?, flate2::Compression::best());
        std::io::copy(&mut src, &mut enc)?;
        enc.finish()?.sync_all()?;
    }
    std::io::copy(&mut flate2::read::GzDecoder::new(fs::File::open(&tmp)?), &mut std::io::sink())?;
    fs::rename(&tmp, &gz)?;
    fs::remove_file(path)
}

/// Gün klasöründeki (kullanicilar/ dahil) dosyalar: `/` ayraçlı göreli yollar, sıralı, gizliler hariç.
pub fn day_files(d: &Path, suffix: &str) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, suffix: &str, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                walk(base, &p, suffix, out);
            } else if name.ends_with(suffix) && !name.starts_with('.') {
                out.push(p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = vec![];
    walk(d, d, suffix, &mut out);
    out.sort();
    out
}

pub fn days(root: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(Path::new(root).join("gunluk"))
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// zincir.txt satırları (gün, manifest_sha, zincir_sha). Yarım yazılmış satır (elektrik kesintisi) sayılmaz.
pub fn chain_lines(root: &str) -> Vec<[String; 3]> {
    fs::read_to_string(Path::new(root).join("zincir.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.trim().split(';').collect();
            (p.len() == 3 && p[1].len() == 64 && p[2].len() == 64).then(|| [p[0].to_string(), p[1].to_string(), p[2].to_string()])
        })
        .collect()
}

fn append_sync(path: &Path, text: &str) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().append(true).create(true).open(path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()
}

fn append_chain(root: &str, day: &str, man_sha: &str) -> std::io::Result<String> {
    let prev = chain_lines(root).last().map(|l| l[2].clone()).unwrap_or_else(|| ZERO.into());
    let chain = chain_hash(&prev, man_sha);
    let path = Path::new(root).join("zincir.txt");
    // önceki satır yarım kaldıysa yeni satır ona yapışmasın
    let torn = fs::read(&path).map(|b| b.last().is_some_and(|&c| c != b'\n')).unwrap_or(false);
    append_sync(&path, &format!("{}{day};{man_sha};{chain}\n", if torn { "\n" } else { "" }))?;
    Ok(chain)
}

fn audit(cfg: &Config, olay: &str, now: f64, ek: &str) {
    ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
}

/// Bir günü mühürler. Son 5 dk'da yazılmış dosya varsa erteler (zorla hariç). Tekrar çalıştırmak güvenlidir.
pub fn close_day(cfg: &Config, day: &str, now: f64, force: bool) -> std::io::Result<String> {
    let root = &cfg.main.log_root;
    let d = Path::new(root).join("gunluk").join(day);
    let man = d.join("MANIFEST.sha256");
    if !d.is_dir() {
        return Ok(format!("{day}: kayıt yok"));
    }
    if !man.exists() {
        let csvs = day_files(&d, ".csv");
        let wall = std::time::SystemTime::now();
        let recent = csvs.iter().any(|f| {
            fs::metadata(d.join(f)).and_then(|m| m.modified()).ok().and_then(|t| wall.duration_since(t).ok()).is_some_and(|age| age.as_secs() < 300)
        });
        if recent && !force {
            audit(cfg, "GUN_KAPAT_ERTELENDI", now, &format!("gun={day}"));
            return Ok(format!("{day}: dosyalar hâlâ yazılıyor, daha sonra tekrar deneyin"));
        }
        for f in &csvs {
            gzip_file(&d.join(f))?;
        }
        let prev = chain_lines(root).last().map(|l| l[2].clone()).unwrap_or_else(|| ZERO.into());
        let mut out = format!("# gun={day} olusturma={} onceki_zincir={prev}\n", ortak::now_iso(now));
        for f in day_files(&d, ".csv.gz") {
            out.push_str(&format!("{}  {f}\n", sha256_file(&d.join(&f))?));
        }
        let tmp = d.join("MANIFEST.sha256.tmp");
        fs::write(&tmp, &out)?;
        fs::File::open(&tmp)?.sync_all()?;
        fs::rename(&tmp, &man)?;
    }
    if !chain_lines(root).iter().any(|l| l[0] == day) {
        append_chain(root, day, &sha256_file(&man)?)?;
    }
    for f in day_files(&d, "") {
        fs::set_permissions(d.join(f), fs::Permissions::from_mode(0o400))?;
    }
    Ok(format!("{day}: kapatıldı"))
}

/// Bugünden önceki mühürsüz bütün günler (cihaz 00:15'te kapalıysa kaçanlar da) ya da verilen gün.
/// Saat bu açılışta doğrulanmadıysa (src/saat.rs) "bugün" bilinmez, bekletilen satırlar da yazılmamıştır: ertelenir
/// (zorla da), saat doğrulanınca kaydedici gün-kapat'ı yeniden çalıştırır.
pub fn gun_kapat(cfg: &Config, day: Option<&str>, now: f64, force: bool) -> Vec<String> {
    if let Err(e) = crate::saat::hazir(cfg) {
        audit(cfg, "GUN_KAPAT_ERTELENDI", now, &format!("neden=saat hata={e}"));
        crate::saat::gun_kapat_ertele();
        return vec![format!("saat doğrulanmadı, mühürleme ertelendi: {e}")];
    }
    let today = ortak::day_of(&ortak::now_iso(now)).to_string();
    let root = &cfg.main.log_root;
    let list: Vec<String> = match day {
        Some(d) => vec![d.to_string()],
        None => days(root).into_iter().filter(|d| *d < today && !Path::new(root).join("gunluk").join(d).join("MANIFEST.sha256").exists()).collect(),
    };
    list.iter()
        .map(|d| match close_day(cfg, d, now, force) {
            Ok(s) => {
                crate::rapor::onbellege_al(cfg, d); // mühürlendiyse raporun gün özeti şimdi (ertelenen güne dokunmaz)
                s
            }
            Err(e) => {
                audit(cfg, "GUN_KAPAT_HATA", now, &format!("gun={d} hata={e}"));
                format!("{d}: HATA {e}")
            }
        })
        .collect()
}

/// Zincir her zaman baştan; dosya hash'leri son `last` gün için (None = hepsi). (gün, sorunlar, not)
pub fn verify(cfg: &Config, last: Option<usize>) -> Vec<(String, Vec<String>, String)> {
    let root = &cfg.main.log_root;
    let lines = chain_lines(root);
    let check_from = last.map_or(0, |n| lines.len().saturating_sub(n));
    let mut prev = ZERO.to_string();
    let mut out = vec![];
    for (i, [day, man_sha, chain]) in lines.iter().enumerate() {
        let mut problems = vec![];
        if chain_hash(&prev, man_sha) != *chain {
            problems.push("zincir".to_string());
        }
        prev = chain.clone();
        if i < check_from {
            if !problems.is_empty() {
                out.push((day.clone(), problems, String::new()));
            }
            continue;
        }
        let d = Path::new(root).join("gunluk").join(day);
        let man = d.join("MANIFEST.sha256");
        let Ok(text) = fs::read_to_string(&man) else {
            out.push((day.clone(), problems, "saklama süresi nedeniyle silinmiş".into()));
            continue;
        };
        if sha256_file(&man).ok().as_deref() != Some(man_sha.as_str()) {
            problems.push("MANIFEST.sha256".into());
        }
        for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let Some((digest, name)) = line.split_once("  ") else { continue };
            if sha256_file(&d.join(name)).ok().as_deref() != Some(digest) {
                problems.push(name.to_string());
            }
        }
        out.push((day.clone(), problems, String::new()));
    }
    out
}

pub fn format_verify(results: &[(String, Vec<String>, String)]) -> String {
    if results.is_empty() {
        return "Kapatılmış gün yok".into();
    }
    results
        .iter()
        .map(|(day, p, note)| {
            if p.is_empty() {
                format!("{day} ✔ hash ✔ zincir {note}").trim_end().to_string()
            } else {
                format!("{day} ✘ {}", p.join(", "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Saklama süresini aşmış ve yedeklenmiş (.yedeklendi) günleri siler; yedeksiz gün asla silinmez.
/// Kişi listesinden son oturumu süreyi aşanlar çıkarılır (KVKK). `dry`: yalnızca listeler.
pub fn prune(cfg: &Config, now: f64, dry: bool) -> Vec<String> {
    // ileri kaymış saat yedeklenmiş günleri vaktinden önce silerdi
    if let Err(e) = crate::saat::hazir(cfg) {
        return vec![format!("saat doğrulanmadı, temizlik yapılmadı: {e}")];
    }
    let root = &cfg.main.log_root;
    let cutoff = ortak::day_of(&ortak::now_iso(now - cfg.main.retention_days as f64 * 86400.0)).to_string();
    let mut actions = vec![];
    for day in days(root).into_iter().filter(|d| *d < cutoff) {
        let d = Path::new(root).join("gunluk").join(&day);
        if !d.join(".yedeklendi").exists() {
            actions.push(format!("YEDEKSİZ, silinmedi: {day}"));
            continue;
        }
        actions.push(format!("gün silindi: {day}"));
        if !dry {
            for f in day_files(&d, "") {
                let _ = fs::set_permissions(d.join(f), fs::Permissions::from_mode(0o600));
            }
            if let Err(e) = fs::remove_dir_all(&d) {
                eprintln!("temizle: {day} silinemedi: {e}");
            }
        }
    }
    let stale: Vec<String> = ortak::read_index(root).into_iter().filter(|(_, r)| r[4].get(..10).unwrap_or("") < cutoff.as_str()).map(|(p, _)| p).collect();
    if !stale.is_empty() {
        actions.push(format!("index: {} eski kişi çıkarıldı", stale.len()));
    }
    if !dry {
        if !stale.is_empty() {
            if let Ok(_g) = ortak::state_lock(&cfg.main.state_root) {
                if let Err(e) = ortak::remove_from_index(root, &stale) {
                    eprintln!("temizle: kişi listesi yazılamadı: {e}");
                }
            }
        }
        // raporun kayıtlardan türettiği veri (bağlı kalma, gün özetleri) kayıtlardan uzun tutulmaz
        crate::rapor::temizle(&cfg.main.state_root, &cutoff);
        for a in &actions {
            audit(cfg, if a.starts_with("YEDEKSİZ") { "YEDEKSIZ_GUN" } else { "SAKLAMA_SILME" }, now, a);
        }
    }
    actions
}

/// Mühürlenmiş ama gönderilmemiş her günü sunucuya gönderir; başarılı olana .yedeklendi yazar.
/// Sunucudaki dosya ezilmez: günler --ignore-existing, zincir/index'in önceki sürümü eski/ altına.
/// Mühürlü, gönderilmemiş günleri ve zincir.txt / index.csv'yi gönderir. Gönderilecek gün yoksa da zincir gönderilir
/// (sunucu bağlantısı her çalıştırmada denenmiş olur). Err: yedek kapalı ya da bir şey gönderilemedi.
pub fn backup(cfg: &Config, now: f64, runner: &Runner) -> Result<String, String> {
    let b = &cfg.backup;
    if !b.enabled {
        return Err("yedekleme kapalı".into());
    }
    let target = b.target.trim_end_matches('/');
    if target.is_empty() {
        return Err("yedek hedefi boş".into());
    }
    let remote = target.contains(':') && !target.starts_with('/');
    if !remote && !Path::new(target).is_dir() {
        return Err("hedef dizin yok".into()); // bağlı olmayan diski kök dosya sistemine doldurmasın
    }
    let mut rsync: Vec<String> = vec!["rsync".into(), "-a".into(), "--timeout=120".into()];
    if remote {
        rsync.extend(["-e".to_string(), b.ssh.clone()]);
    }
    let root = &cfg.main.log_root;
    let g = Path::new(root).join("gunluk");
    let pending: Vec<String> =
        days(root).into_iter().filter(|d| g.join(d).join("MANIFEST.sha256").exists() && !g.join(d).join(".yedeklendi").exists()).collect();
    for day in &pending {
        let mut cmd = rsync.clone();
        cmd.extend(["--ignore-existing".to_string(), g.join(day).display().to_string(), format!("{target}/gunluk/")]);
        if !runner(&cmd) {
            audit(cfg, "YEDEK_HATA", now, &format!("gun={day}"));
            return Err(format!("{day} sunucuya gönderilemedi"));
        }
        if let Err(e) = fs::write(g.join(day).join(".yedeklendi"), format!("{}\n", ortak::now_iso(now))) {
            eprintln!("yedekle: işaret yazılamadı: {e}");
        }
        audit(cfg, "YEDEK", now, &format!("gun={day}"));
    }
    let extras: Vec<String> = [Path::new(root).join("zincir.txt"), Path::new(root).join("kullanicilar/index.csv")]
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .collect();
    if !extras.is_empty() {
        let mut cmd = rsync.clone();
        cmd.extend(["--backup".into(), "--backup-dir=eski".into(), format!("--suffix=.{}", ortak::day_of(&ortak::now_iso(now)))]);
        cmd.extend(extras.iter().cloned());
        cmd.push(format!("{target}/"));
        if !runner(&cmd) {
            // zincir sunucuya ulaşmadan yerel kopyaya güvenilmesin (fabrika dönüşü bunu bekler)
            audit(cfg, "YEDEK_HATA", now, "dosya=zincir.txt");
            return Err("zincir.txt sunucuya gönderilemedi (sunucuya ulaşılamıyor olabilir)".into());
        }
    }
    Ok(match (pending.is_empty(), extras.is_empty()) {
        (true, true) => "gönderilecek kayıt yok".into(),
        (true, false) => "yeni gün yok; zincir gönderildi (sunucu bağlantısı sağlam)".into(),
        _ => format!("{} gün gönderildi: {}", pending.len(), pending.join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const NOW: f64 = 1_790_705_134.0; // 2026-09-29

    fn setup(extra: &str) -> (Config, PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-muhur-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        let text = format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n{extra}", root.display());
        (Config::parse(&text).unwrap(), root)
    }

    fn write_day(cfg: &Config, day: &str) {
        let r = Row::new("DHCP_ATAMA", &format!("{day}T10:00:00+03:00")).set("mac", "aa:bb:cc:dd:ee:01");
        ortak::append_rows(&ortak::day_file(&cfg.main.log_root, day, "dhcp.csv"), &[r.clone()]).unwrap();
        ortak::append_rows(&ortak::user_file(&cfg.main.log_root, day, "5334553132"), &[r]).unwrap();
    }

    #[test]
    fn seal_postpone_chain_and_tamper_detection() {
        let (cfg, root) = setup("");
        write_day(&cfg, "2026-09-27");
        write_day(&cfg, "2026-09-28");
        // az önce yazıldı → ertelenir
        assert_eq!(gun_kapat(&cfg, None, NOW, false), vec!["2026-09-27: dosyalar hâlâ yazılıyor, daha sonra tekrar deneyin", "2026-09-28: dosyalar hâlâ yazılıyor, daha sonra tekrar deneyin"]);
        assert_eq!(gun_kapat(&cfg, None, NOW, true), vec!["2026-09-27: kapatıldı", "2026-09-28: kapatıldı"]);
        let d = root.join("5651/gunluk/2026-09-28");
        assert!(d.join("dhcp.csv.gz").exists() && !d.join("dhcp.csv").exists());
        let man = std::fs::read_to_string(d.join("MANIFEST.sha256")).unwrap();
        assert!(man.contains("  dhcp.csv.gz") && man.contains("  kullanicilar/5334553132.csv.gz"));
        assert_eq!(std::fs::metadata(d.join("dhcp.csv.gz")).unwrap().permissions().mode() & 0o777, 0o400);
        let chain = chain_lines(&cfg.main.log_root);
        assert_eq!(chain.len(), 2);
        assert!(gun_kapat(&cfg, None, NOW, true).is_empty()); // tekrar: yapılacak yok
        assert_eq!(chain_lines(&cfg.main.log_root).len(), 2);
        assert_eq!(format_verify(&verify(&cfg, None)), "2026-09-27 ✔ hash ✔ zincir\n2026-09-28 ✔ hash ✔ zincir");
        // bir bayt değiştir → hangi dosya olduğu söylenir
        let gz = d.join("dhcp.csv.gz");
        std::fs::set_permissions(&gz, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut b = std::fs::read(&gz).unwrap();
        let last = b.len() - 1;
        b[last] ^= 1;
        std::fs::write(&gz, b).unwrap();
        assert_eq!(format_verify(&verify(&cfg, Some(1))), "2026-09-28 ✘ dhcp.csv.gz");
        // zincir satırını değiştir → zincir kırılır
        let z = root.join("5651/zincir.txt");
        let mut lines: Vec<String> = std::fs::read_to_string(&z).unwrap().lines().map(String::from).collect();
        let p: Vec<&str> = lines[0].split(';').collect();
        lines[0] = format!("{};{};{}", p[0], "a".repeat(64), p[2]);
        std::fs::write(&z, lines.join("\n") + "\n").unwrap();
        assert!(format_verify(&verify(&cfg, None)).starts_with("2026-09-27 ✘ zincir"));
    }

    #[test]
    fn prune_only_backed_up_old_days_and_stale_people() {
        let (mut cfg, root) = setup("");
        cfg.main.retention_days = 2;
        let now = ortak::wall();
        let day = |back: f64| ortak::day_of(&ortak::now_iso(now - back * 86400.0)).to_string();
        let (old_backed, old_unbacked, recent) = (day(5.0), day(4.0), day(1.0));
        for d in [&old_backed, &old_unbacked, &recent] {
            write_day(&cfg, d);
        }
        std::fs::write(root.join(format!("5651/gunluk/{old_backed}/.yedeklendi")), "x").unwrap();
        ortak::upsert_index(&cfg.main.log_root, "5334553132", "A", "B", &format!("{recent}T10:00:00+03:00")).unwrap();
        ortak::upsert_index(&cfg.main.log_root, "5550000000", "C", "D", &format!("{old_backed}T10:00:00+03:00")).unwrap();
        let dry = prune(&cfg, now, true);
        assert!(dry.contains(&format!("gün silindi: {old_backed}")) && dry.contains(&format!("YEDEKSİZ, silinmedi: {old_unbacked}")));
        assert!(root.join(format!("5651/gunluk/{old_backed}")).exists());
        prune(&cfg, now, false);
        assert!(!root.join(format!("5651/gunluk/{old_backed}")).exists());
        assert!(root.join(format!("5651/gunluk/{old_unbacked}")).exists() && root.join(format!("5651/gunluk/{recent}")).exists());
        assert_eq!(ortak::read_index(&cfg.main.log_root).keys().cloned().collect::<Vec<_>>(), vec!["5334553132"]);
    }

    #[test]
    fn backup_sends_sealed_unsent_days_and_stops_on_error() {
        let (mut cfg, root) = setup("");
        write_day(&cfg, "2026-09-27");
        write_day(&cfg, "2026-09-28");
        gun_kapat(&cfg, None, NOW, true);
        let runner = |_: &[String]| true;
        assert_eq!(backup(&cfg, NOW, &runner), Err("yedekleme kapalı".into()));
        cfg.backup.enabled = true;
        cfg.backup.target = "kafe-x@192.168.1.109:".into();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let c2 = calls.clone();
        let ok = Arc::new(Mutex::new(true));
        let ok2 = ok.clone();
        let runner = move |c: &[String]| {
            c2.lock().unwrap().push(c.to_vec());
            *ok2.lock().unwrap()
        };
        *ok.lock().unwrap() = false;
        assert_eq!(backup(&cfg, NOW, &runner), Err("2026-09-27 sunucuya gönderilemedi".into()));
        assert!(!root.join("5651/gunluk/2026-09-27/.yedeklendi").exists());
        *ok.lock().unwrap() = true;
        assert_eq!(backup(&cfg, NOW, &runner), Ok("2 gün gönderildi: 2026-09-27, 2026-09-28".into()));
        assert!(root.join("5651/gunluk/2026-09-28/.yedeklendi").exists());
        assert_eq!(backup(&cfg, NOW, &runner), Ok("yeni gün yok; zincir gönderildi (sunucu bağlantısı sağlam)".into()));
        *ok.lock().unwrap() = false;
        assert!(backup(&cfg, NOW, &runner).unwrap_err().contains("zincir.txt")); // yeni gün yokken de bağlantı denenir
        *ok.lock().unwrap() = true;
        let calls = calls.lock().unwrap();
        let day_cmd = &calls[1];
        assert_eq!(&day_cmd[..5], ["rsync", "-a", "--timeout=120", "-e", "ssh -i /etc/wificorrect/yedek_anahtar -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"]);
        assert!(day_cmd.contains(&"--ignore-existing".to_string()) && day_cmd.last().unwrap() == "kafe-x@192.168.1.109:/gunluk/");
        assert!(calls.iter().any(|c| c.contains(&"--backup-dir=eski".to_string()) && c.last().unwrap() == "kafe-x@192.168.1.109:/"));
    }
}
