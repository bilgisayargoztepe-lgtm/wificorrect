//! Uzak erişim (2026-10-03, kullanıcı kararı: kendi sunucumuz + WireGuard). Cihaz NAT arkasından sunucuya tünel açar ve
//! canlı tutar; admin aynı sunucuya bağlanıp cihazın tünel adresinden (10.99.0.x) panele ve SSH'a girer.
//! Tünel yalnızca VPN ağını taşır (internet trafiği tünelden geçmez); güvenlik duvarı tünelden yalnızca 22 ve 8443'ü açar.
//! Gizli anahtar cihazda üretilir, cihazdan çıkmaz; açık anahtar Admin ayarları'nda görünür, sunucuya o eklenir.

use crate::ag::Ag;
use crate::ayar::Config;
use crate::ortak::{self, Row, Runner};
use crate::pppoe;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const KEY: &str = "/etc/wificorrect/wg.key";
/// Yedek (rsync/SSH) anahtarı: cihazda üretilir, açık anahtarı sunucudaki yalnızca-yazma hesaba eklenir
pub const YEDEK_KEY: &str = "/etc/wificorrect/yedek_anahtar";
pub const CONF: &str = "/etc/wireguard/wfc.conf";
pub const IFACE: &str = "wfc";
pub const UNIT: &str = "wg-quick@wfc.service";
/// VPN ağı: sunucu .1, yönetici bilgisayarları ve cihazlar diğer adresler (ponytail: sabit; guvenlik.nft de bunu açar)
pub const AG: &str = "10.99.0.0/24";

pub fn key_ok(s: &str) -> bool {
    s.len() == 44 && s.ends_with('=') && s[..43].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

/// `vpn.ornek.com:51820` ya da `203.0.113.5:51820`
pub fn endpoint_ok(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else { return false };
    let host_ok = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|l| (1..=63).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-'));
    host_ok && port.parse::<u16>().is_ok_and(|p| p > 0)
}

/// Cihazın tünel adresi: 10.99.0.2 – 10.99.0.254 (.1 sunucu)
pub fn adres_ok(s: &str) -> bool {
    s.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ortak::in_subnet(s, AG) && (2..=254).contains(&ip.octets()[3]))
}

/// Gizli anahtar yoksa üretir (600). Dönen: gizli anahtar.
pub fn ensure_key(path: &Path) -> Result<String, String> {
    if let Ok(k) = std::fs::read_to_string(path) {
        if key_ok(k.trim()) {
            return Ok(k.trim().to_string());
        }
    }
    let out = Command::new("wg").arg("genkey").output().map_err(|e| format!("wg genkey: {e}"))?;
    let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !key_ok(&k) {
        return Err("anahtar üretilemedi".into());
    }
    ortak::write_atomic(path, format!("{k}\n").as_bytes()).map_err(|e| e.to_string())?;
    Ok(k)
}

pub fn public_key(private: &str) -> Result<String, String> {
    let mut child = Command::new("wg").arg("pubkey").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|e| format!("wg pubkey: {e}"))?;
    child.stdin.take().ok_or("stdin")?.write_all(format!("{private}\n").as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let k = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if key_ok(&k) { Ok(k) } else { Err("açık anahtar hesaplanamadı".into()) }
}

/// Yedek SSH anahtarı yoksa üretir; dönen: açık anahtar satırı (ssh-ed25519 …).
pub fn yedek_anahtari(path: &Path) -> Result<String, String> {
    if !path.exists() {
        let host = ortak::capture(&["hostname"]).trim().to_string();
        let ok = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", &format!("wificorrect-{host}"), "-f"])
            .arg(path)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err("yedek anahtarı üretilemedi (ssh-keygen)".into());
        }
    }
    let pub_path = format!("{}.pub", path.display());
    std::fs::read_to_string(&pub_path).map(|s| s.trim().to_string()).map_err(|e| format!("{pub_path}: {e}"))
}

/// İşletme adından sunucu kayıt adı: "Bocafe Kadıköy" → "bocafe-kadikoy" (küçük harf, rakam, tire; en çok 31).
pub fn kayit_adi(site: &str) -> String {
    let mut out = String::new();
    for c in site.chars() {
        let c = match c {
            'ç' | 'Ç' => 'c',
            'ğ' | 'Ğ' => 'g',
            'ı' | 'I' | 'İ' | 'i' => 'i',
            'ö' | 'Ö' => 'o',
            'ş' | 'Ş' => 's',
            'ü' | 'Ü' => 'u',
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            _ => '-',
        };
        if c != '-' || !out.ends_with('-') && !out.is_empty() {
            out.push(c);
        }
    }
    let s: String = out.trim_end_matches('-').chars().take(31).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "isletme".into() } else { s }
}

/// Merkez sunucuda bu cihazı tanıtan komut (panelde kopyalanmak üzere gösterilir).
pub fn sunucu_komutu(site: &str, wg_pub: &str, ssh_pub: &str) -> String {
    format!("sudo wificorrect-sunucu cihaz-ekle {} {wg_pub} \"{ssh_pub}\"", kayit_adi(site))
}

pub fn eksik(cfg: &Config) -> Option<&'static str> {
    let u = &cfg.uzak;
    if !endpoint_ok(&u.sunucu) {
        Some("sunucu adresi (adres:port)")
    } else if !key_ok(&u.sunucu_anahtar) {
        Some("sunucunun açık anahtarı")
    } else if !adres_ok(&u.adres) {
        Some("cihazın tünel adresi (10.99.0.2–254)")
    } else {
        None
    }
}

/// WireGuard'ın dış sarmalaması: IPv6 40 + UDP 8 + WireGuard 32 bayt. WireGuard'ın önerdiği tünel MTU'su = hat MTU'su − 80.
const SARMA: u16 = 80;

/// Tünelin MTU'su (spec 2026-10-23 "Cihazda bağlantı"): PPPoE açıksa hattın MTU'su − 80 (varsayılan 1492 → 1412), kapalıysa `None`:
/// yapılandırmaya MTU satırı yazılmaz, `wg-quick` kendi seçer (Ethernet'te 1420). PPPoE hattı Ethernet'ten dardır ve MTU'su
/// ayarlanabilir; tünel dış paketi hatta sığmazsa bölünür ve yavaşlar. Geçerli aralığın (1280-1492, `ag::pppoe_dogrula`) dışındaki
/// değer (elle bozulmuş ag.toml) taşma ya da `MTU = 0` üretmesin diye aralığa çekilir: tünel her durumda kalkar.
pub fn tunel_mtu(ag: &Ag) -> Option<u16> {
    ag.pppoe.acik.then(|| ag.pppoe.mtu.clamp(1280, pppoe::MTU as u16) - SARMA)
}

/// `mtu`: `[Interface]`'e `MTU = n` olarak yazılır (`tunel_mtu`); `None`: satır yok.
pub fn conf_text(cfg: &Config, private: &str, mtu: Option<u16>) -> String {
    let u = &cfg.uzak;
    let mtu_satiri = mtu.map_or(String::new(), |n| format!("MTU = {n}\n"));
    format!(
        "# WifiCorrect üretir (panel → Admin ayarları → Uzak erişim). Elle değiştirmeyin.\n\
         [Interface]\nPrivateKey = {private}\nAddress = {}/24\n{mtu_satiri}\n\
         [Peer]\nPublicKey = {}\nEndpoint = {}\nAllowedIPs = {AG}\nPersistentKeepalive = 25\n",
        u.adres, u.sunucu_anahtar, u.sunucu
    )
}

/// Yapılandırma metnindeki `MTU = n` satırı (yoksa `None`). `wg-quick` anahtar adında büyük/küçük harfe bakmaz ve boşluk serbesttir.
/// Satır var ama sayı okunamıyorsa `Some(0)`: hiçbir geçerli MTU'ya eşit olmaz, dosya yeniden yazılır.
fn okunan_mtu(conf: &str) -> Option<u16> {
    conf.lines().find_map(|satir| {
        let (anahtar, deger) = satir.split_once('=')?;
        anahtar.trim().eq_ignore_ascii_case("MTU").then(|| deger.trim().parse().unwrap_or(0))
    })
}

/// Ayara göre tüneli açar ya da kapatır. `mtu`: yapılandırmaya yazılacak tünel MTU'su (`tunel_mtu`).
pub fn uygula(cfg: &Config, key: &Path, conf: &Path, mtu: Option<u16>, runner: &Runner) -> Result<String, String> {
    let cmd = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    if !cfg.uzak.enabled {
        runner(&cmd(&["systemctl", "disable", "--now", UNIT]));
        let _ = std::fs::remove_file(conf);
        return Ok("uzak erişim kapalı".into());
    }
    if let Some(e) = eksik(cfg) {
        return Err(format!("eksik ya da geçersiz: {e}"));
    }
    let private = ensure_key(key)?;
    if let Some(d) = conf.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    ortak::write_atomic(conf, conf_text(cfg, &private, mtu).as_bytes()).map_err(|e| format!("{}: {e}", conf.display()))?;
    runner(&cmd(&["systemctl", "enable", UNIT]));
    if !runner(&cmd(&["systemctl", "restart", UNIT])) {
        return Err("tünel başlatılamadı (sunucu adresi çözülemedi ya da ayar hatalı olabilir)".into());
    }
    let mtu = mtu.map_or("otomatik".to_string(), |n| n.to_string());
    Ok(format!("tünel kuruldu: {} → {} (MTU {mtu})", cfg.uzak.adres, cfg.uzak.sunucu))
}

/// PPPoE açılıp kapanınca (ya da hattın MTU'su değişince) çalışan tüneli yeni MTU'ya getirir. Uzak erişim kapalıysa ya da yapılandırma
/// dosyasındaki `MTU` satırı istenenle aynıysa (ikisi de yoksa dahil) `None`: tünel boşuna kesilmez (çoğu cihaz PPPoE'siz; onlarda
/// hiçbir şey olmaz). Değilse `uygula` (yapılandırmayı yazar, tüneli yeniden başlatır). Sonucu çağıran denetime yazar (`audit`).
/// Yeniden başlatma başarısız olsa da dosya günceldir: tünel gözcüsü (kaydedici) arayüz yoksa tüneli yeniden başlatır ve MTU'yu dosyadan alır.
pub fn mtu_esitle(cfg: &Config, key: &Path, conf: &Path, mtu: Option<u16>, runner: &Runner) -> Option<Result<String, String>> {
    if !cfg.uzak.enabled {
        return None;
    }
    let mevcut = std::fs::read_to_string(conf).ok().and_then(|metin| okunan_mtu(&metin));
    (mevcut != mtu).then(|| uygula(cfg, key, conf, mtu, runner))
}

/// `wg show wfc latest-handshakes` çıktısından son el sıkışmanın zamanı (epoch).
pub fn last_handshake(wg_show: &str) -> Option<f64> {
    wg_show.lines().filter_map(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()).filter(|&t| t > 0.0).reduce(f64::max)
}

/// Panelde gösterilen durum.
pub fn durum(cfg: &Config, now: f64) -> String {
    if !cfg.uzak.enabled {
        return "kapalı".into();
    }
    match last_handshake(&ortak::capture(&["wg", "show", IFACE, "latest-handshakes"])) {
        Some(t) if now - t < 180.0 => format!("bağlı (son el sıkışma {} sn önce)", (now - t).max(0.0) as u64),
        Some(t) => format!("BAĞLANTI YOK (son el sıkışma {} dk önce)", ((now - t) / 60.0) as u64),
        None => "BAĞLANTI YOK (sunucuyla hiç el sıkışılmadı)".into(),
    }
}

pub fn audit(cfg: &Config, now: f64, r: &Result<String, String>) {
    let ek = match r {
        Ok(m) => format!("sonuc={m}"),
        Err(e) => format!("hata={e}"),
    };
    ortak::audit(&cfg.main.log_root, Row::new("UZAK_ERISIM", &ortak::now_iso(now)).set("ek", ek));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const K: &str = "aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789+/abcdE=";

    #[test]
    fn validation_and_config() {
        assert!(key_ok(K) && !key_ok("kısa") && !key_ok(&K.replace('=', "A")));
        assert!(endpoint_ok("vpn.wificorrect.com:51820") && endpoint_ok("203.0.113.5:51820"));
        assert!(!endpoint_ok("vpn.wificorrect.com") && !endpoint_ok(":51820") && !endpoint_ok("a b:1") && !endpoint_ok("x.com:0"));
        assert!(adres_ok("10.99.0.17") && !adres_ok("10.99.0.1") && !adres_ok("10.98.0.17") && !adres_ok("10.99.0.255"));
        let mut cfg = Config::default();
        cfg.uzak.enabled = true;
        assert_eq!(eksik(&cfg), Some("sunucu adresi (adres:port)"));
        cfg.uzak.sunucu = "vpn.wificorrect.com:51820".into();
        cfg.uzak.sunucu_anahtar = K.into();
        cfg.uzak.adres = "10.99.0.17".into();
        assert_eq!(eksik(&cfg), None);
        let t = conf_text(&cfg, "GIZLI", None);
        assert!(t.contains("PrivateKey = GIZLI\nAddress = 10.99.0.17/24") && t.contains("AllowedIPs = 10.99.0.0/24") && t.contains("PersistentKeepalive = 25"));
        assert_eq!(last_handshake(&format!("{K}\t1791000000\n{K}\t0\n")), Some(1_791_000_000.0));
        assert_eq!(last_handshake(&format!("{K}\t0\n")), None);
        assert_eq!(kayit_adi("Bocafe Kadıköy Şubesi"), "bocafe-kadikoy-subesi");
        assert_eq!(kayit_adi("  Çay & Ötesi!! "), "cay-otesi");
        assert_eq!(kayit_adi("İşletme"), "isletme");
        assert_eq!(kayit_adi("???"), "isletme");
        assert_eq!(kayit_adi(&"a".repeat(40)).len(), 31);
        assert_eq!(sunucu_komutu("Bocafe", "W=", "ssh-ed25519 AAAA x"), "sudo wificorrect-sunucu cihaz-ekle bocafe W= \"ssh-ed25519 AAAA x\"");
    }

    #[test]
    fn apply_enable_disable() {
        let dir = std::env::temp_dir().join(format!("wfc-uzak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (key, conf) = (dir.join("wg.key"), dir.join("wireguard/wfc.conf"));
        std::fs::write(&key, format!("{K}\n")).unwrap(); // var olan anahtar korunur
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let runner = move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            true
        };
        let mut cfg = Config::default();
        cfg.uzak.enabled = true;
        assert!(uygula(&cfg, &key, &conf, None, &runner).unwrap_err().contains("eksik"));
        cfg.uzak.sunucu = "vpn.wificorrect.com:51820".into();
        cfg.uzak.sunucu_anahtar = K.into();
        cfg.uzak.adres = "10.99.0.17".into();
        let ok = uygula(&cfg, &key, &conf, None, &runner).unwrap();
        assert!(ok.contains("MTU otomatik"), "{ok}");
        assert!(std::fs::read_to_string(&conf).unwrap().contains(&format!("PrivateKey = {K}")));
        assert!(!std::fs::read_to_string(&conf).unwrap().contains("MTU"));
        assert!(calls.lock().unwrap().iter().any(|c| c == "systemctl restart wg-quick@wfc.service"));
        // PPPoE açıkken çağıran MTU verir: [Interface]'e yazılır
        let ok = uygula(&cfg, &key, &conf, Some(1412), &runner).unwrap();
        assert!(ok.contains("MTU 1412"), "{ok}");
        assert!(std::fs::read_to_string(&conf).unwrap().contains(&format!("PrivateKey = {K}\nAddress = 10.99.0.17/24\nMTU = 1412\n")));
        cfg.uzak.enabled = false;
        uygula(&cfg, &key, &conf, None, &runner).unwrap();
        assert!(!conf.exists() && calls.lock().unwrap().iter().any(|c| c == "systemctl disable --now wg-quick@wfc.service"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------- tünel MTU'su (PPPoE: spec 2026-10-23 "Cihazda bağlantı")
    fn acik_ayar() -> Config {
        let mut cfg = Config::default();
        cfg.uzak.enabled = true;
        cfg.uzak.sunucu = "vpn.wificorrect.com:51820".into();
        cfg.uzak.sunucu_anahtar = K.into();
        cfg.uzak.adres = "10.99.0.17".into();
        cfg
    }

    /// Geçici klasör, içinde var olan gizli anahtar (testler gerçek anahtara ve tünele dokunmasın): (klasör, anahtar, yapılandırma)
    fn gecici(ad: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("wfc-uzak-{ad}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("wg.key");
        std::fs::write(&key, format!("{K}\n")).unwrap();
        let conf = dir.join("wireguard/wfc.conf");
        (dir, key, conf)
    }

    /// Sahte komut çalıştırıcı: çağrıları sırayla toplar; `basarisiz` önekiyle başlayan komut false döner.
    fn sahte(basarisiz: &'static [&'static str]) -> (impl Fn(&[String]) -> bool + Send + Sync, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let r = move |c: &[String]| {
            let s = c.join(" ");
            c2.lock().unwrap().push(s.clone());
            !basarisiz.iter().any(|b| s.starts_with(b))
        };
        (r, calls)
    }

    /// Şimdiye kadarki çağrıları alır ve listeyi boşaltır.
    fn al(c: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        std::mem::take(&mut *c.lock().unwrap())
    }

    #[test]
    fn tunel_mtu_hatta_gore() {
        use crate::ag::{Ag, Pppoe};
        let ag = |acik: bool, mtu: u16| Ag { pppoe: Pppoe { acik, mtu, ..Pppoe::default() }, ..Ag::default() };
        assert_eq!(tunel_mtu(&Ag::default()), None); // PPPoE kapalı (varsayılan): yapılandırmaya MTU yazılmaz
        assert_eq!(tunel_mtu(&ag(false, 1400)), None); // kapalıyken MTU ayarı önemsiz
        assert_eq!(tunel_mtu(&ag(true, 1492)), Some(1412)); // hattın MTU'su − 80 (WireGuard sarmalaması)
        assert_eq!(tunel_mtu(&ag(true, 1400)), Some(1320));
        assert_eq!(tunel_mtu(&ag(true, 1280)), Some(1200));
        // geçerli aralığın (1280-1492) dışı, elle bozulmuş ag.toml: taşma ya da `MTU = 0` üretmez, tünel yine kalkar
        assert_eq!(tunel_mtu(&ag(true, 10)), Some(1200));
        assert_eq!(tunel_mtu(&ag(true, 0)), Some(1200));
        assert_eq!(tunel_mtu(&ag(true, 1500)), Some(1412));
        assert_eq!(tunel_mtu(&ag(true, u16::MAX)), Some(1412));
    }

    #[test]
    fn conf_mtu() {
        let cfg = acik_ayar();
        // MTU [Interface] bölümünde, [Peer]'den önce
        let t = conf_text(&cfg, "GIZLI", Some(1412));
        let (arayuz, es) = t.split_once("[Peer]").unwrap();
        assert!(arayuz.contains("PrivateKey = GIZLI\nAddress = 10.99.0.17/24\nMTU = 1412\n\n"), "{t}");
        assert!(!es.contains("MTU"), "{t}");
        assert!(conf_text(&cfg, "GIZLI", Some(1320)).contains("\nMTU = 1320\n"));
        // MTU yoksa dosya bugünküyle birebir aynı: PPPoE'siz cihazlarda yükseltme tüneli yeniden yazdırmaz
        let t = conf_text(&cfg, "GIZLI", None);
        assert!(!t.contains("MTU"), "{t}");
        assert_eq!(
            t,
            format!(
                "# WifiCorrect üretir (panel → Admin ayarları → Uzak erişim). Elle değiştirmeyin.\n[Interface]\nPrivateKey = GIZLI\nAddress = 10.99.0.17/24\n\n\
                 [Peer]\nPublicKey = {K}\nEndpoint = vpn.wificorrect.com:51820\nAllowedIPs = 10.99.0.0/24\nPersistentKeepalive = 25\n"
            )
        );
    }

    #[test]
    fn okunan_mtu_ayrisimi() {
        let cfg = acik_ayar();
        assert_eq!(okunan_mtu(&conf_text(&cfg, "G", Some(1412))), Some(1412));
        assert_eq!(okunan_mtu(&conf_text(&cfg, "G", None)), None);
        // elle yazılmış biçimler: wg-quick anahtar adında büyük/küçük harfe bakmaz, boşluk serbest
        assert_eq!(okunan_mtu("[Interface]\nmtu=1320\n"), Some(1320));
        assert_eq!(okunan_mtu("[Interface]\n  MTU   =   1320  \n"), Some(1320));
        // yorum satırı ve başka anahtarlar MTU sayılmaz
        assert_eq!(okunan_mtu("[Interface]\n# MTU = 1200\nPersistentKeepalive = 25\n"), None);
        // satır var ama sayı okunamıyor: hiçbir geçerli MTU'ya eşit olmasın (dosya yeniden yazılsın)
        assert_eq!(okunan_mtu("[Interface]\nMTU = abc\n"), Some(0));
        assert_eq!(okunan_mtu(""), None);
    }

    #[test]
    fn mtu_esitle_gereksiz() {
        let (dir, key, conf) = gecici("mtu-gereksiz");
        let cfg = acik_ayar();
        let (r, c) = sahte(&[]);
        // uzak erişim kapalı: tünel yok, dosyaya dokunulmaz
        let mut kapali = cfg.clone();
        kapali.uzak.enabled = false;
        assert_eq!(mtu_esitle(&kapali, &key, &conf, Some(1412), &r), None);
        assert!(!conf.exists() && al(&c).is_empty());
        // dosya zaten istenen MTU'da: tünel yeniden başlamaz, dosya aynı kalır
        uygula(&cfg, &key, &conf, Some(1412), &r).unwrap();
        al(&c);
        let once = std::fs::read(&conf).unwrap();
        assert_eq!(mtu_esitle(&cfg, &key, &conf, Some(1412), &r), None);
        assert_eq!((std::fs::read(&conf).unwrap(), al(&c)), (once, Vec::<String>::new()));
        // ikisi de yok: PPPoE'siz cihaz, bugünkü dosya (yükseltmede hiçbir tünel kesilmez)
        uygula(&cfg, &key, &conf, None, &r).unwrap();
        al(&c);
        assert_eq!(mtu_esitle(&cfg, &key, &conf, None, &r), None);
        assert!(al(&c).is_empty());
        // dosya hiç yokken MTU de istenmiyorsa bir şey yapılmaz (tüneli kuran `uygula`dır, eşitleme değil)
        std::fs::remove_file(&conf).unwrap();
        assert_eq!(mtu_esitle(&cfg, &key, &conf, None, &r), None);
        assert!(!conf.exists() && al(&c).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mtu_esitle_gerekli() {
        let (dir, key, conf) = gecici("mtu-gerekli");
        let cfg = acik_ayar();
        let (r, c) = sahte(&[]);
        let oku = || std::fs::read_to_string(&conf).unwrap();
        // PPPoE'siz cihazın tüneli (MTU satırı yok) → PPPoE açıldı: 1412 yazılır, tünel yeniden başlar
        uygula(&cfg, &key, &conf, None, &r).unwrap();
        al(&c);
        let sonuc = mtu_esitle(&cfg, &key, &conf, Some(1412), &r);
        assert!(matches!(&sonuc, Some(Ok(m)) if m.contains("MTU 1412")), "{sonuc:?}");
        assert!(oku().contains("Address = 10.99.0.17/24\nMTU = 1412\n"), "{}", oku());
        assert_eq!(al(&c), ["systemctl enable wg-quick@wfc.service", "systemctl restart wg-quick@wfc.service"]);
        // hattın MTU'su değişti (1400): 1412 → 1320
        assert!(matches!(mtu_esitle(&cfg, &key, &conf, Some(1320), &r), Some(Ok(_))));
        assert!(oku().contains("\nMTU = 1320\n") && !oku().contains("1412"), "{}", oku());
        assert!(al(&c).contains(&"systemctl restart wg-quick@wfc.service".to_string()));
        // PPPoE kapandı: satır kalkar, dosya bugünküyle aynı
        assert!(matches!(mtu_esitle(&cfg, &key, &conf, None, &r), Some(Ok(_))));
        assert_eq!(oku(), conf_text(&cfg, K, None));
        assert!(al(&c).contains(&"systemctl restart wg-quick@wfc.service".to_string()));
        // yapılandırma hiç yokken (uzak erişim açık ama tünel kurulmamış) MTU isteniyorsa tünel kurulur
        std::fs::remove_file(&conf).unwrap();
        assert!(matches!(mtu_esitle(&cfg, &key, &conf, Some(1412), &r), Some(Ok(_))));
        assert!(oku().contains("\nMTU = 1412\n"));
        // tünel yeniden başlatılamazsa hata döner; dosya yine güncel (tünel gözcüsü arayüz yoksa yeniden başlatır, MTU'yu dosyadan alır)
        let (r2, _) = sahte(&["systemctl restart"]);
        let sonuc = mtu_esitle(&cfg, &key, &conf, Some(1320), &r2);
        assert!(matches!(&sonuc, Some(Err(e)) if e.contains("tünel başlatılamadı")), "{sonuc:?}");
        assert!(oku().contains("\nMTU = 1320\n"));
        // ayar eksikse (sunucu adresi yok) hata, dosyaya dokunulmaz
        let mut eksik_ayar = cfg.clone();
        eksik_ayar.uzak.sunucu.clear();
        let once = oku();
        assert!(matches!(mtu_esitle(&eksik_ayar, &key, &conf, None, &r), Some(Err(e)) if e.contains("eksik")));
        assert_eq!(oku(), once);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
