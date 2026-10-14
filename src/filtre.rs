//! Yasaklı siteler ve kelimeler (eski dns-filtre.sh; RUST_YENIDEN_YAZIM.md "yasaklı kelimeler ve siteler" modülü).
//! - Site (ör. bet365.com): alan adı ve bütün alt adları → dnsmasq `address=/site/` (NXDOMAIN; sorgu DNS kaydına düşer).
//! - Kelime (ör. bet): alan adının herhangi bir parçasında geçerse → müşteri DNS sorgusu güvenlik duvarında reddedilir
//!   (iptables-nft + xt_string; dnsmasq kelime ortası eşleştiremez). İstisna kelimesi geçen sorgu kelime filtresinden geçer.
//! Bütün DNS zaten cihaza yönlendirildiği için başka DNS yazmak filtreyi atlatmaz; şifreli DNS (DoH) atlatır.

use crate::ayar::Config;
use crate::ortak::{self, Runner};
use std::collections::BTreeMap;
use std::path::Path;

pub const DNSMASQ_CONF: &str = "/etc/wificorrect/yasak-siteler.conf";
const CHAIN: &str = "WFC_DNS";
const IFACE: &str = "br-hotspot";

/// Kullanıcının yazdığı adresten alan adı: `https://www.Bet365.com/tr` → `www.bet365.com`.
pub fn normalize_site(raw: &str) -> Option<String> {
    let s = raw.trim().to_lowercase();
    let s = s.split_once("://").map_or(s.as_str(), |(_, r)| r);
    let host = s.split(['/', '?', '#']).next()?.split(':').next()?.trim_end_matches('.');
    let ok = host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| (1..=63).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && !l.starts_with('-') && !l.ends_with('-'));
    ok.then(|| host.to_string())
}

/// Kelime / istisna: 2-63 karakter, küçük harf, rakam, tire (alan adı parçası; nokta içeremez).
pub fn normalize_word(raw: &str) -> Option<String> {
    let w = raw.trim().to_lowercase();
    ((2..=63).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')).then_some(w)
}

/// Engelli mi, hangi kuraldan. Güvenlik duvarı DNS paketinde ararken alan adı parçaları ayrı ayrı durur (aralarında
/// uzunluk baytı) → kelime ve istisna her parçada ayrı aranır, bu da aynısını yapar. Hazır listeler (src/liste.rs)
/// elle girilen sitelerden sonra bakılır.
pub fn check(cfg: &Config, hazir: &crate::liste::Yollar, domain: &str) -> Option<String> {
    let d = normalize_site(domain).or_else(|| normalize_word(domain))?;
    let f = &cfg.filtre;
    if let Some(s) = f.siteler.iter().find(|s| d == **s || d.ends_with(&format!(".{s}"))) {
        return Some(format!("yasaklı site: {s}"));
    }
    if let Some(ad) = crate::liste::bul(cfg, hazir, &d) {
        return Some(format!("hazır liste: {ad}"));
    }
    let labels: Vec<&str> = d.split('.').collect();
    let has = |w: &String| labels.iter().any(|l| l.contains(w.as_str()));
    if f.istisnalar.iter().any(has) {
        return None;
    }
    f.kelimeler.iter().find(|w| has(w)).map(|w| format!("yasaklı kelime: {w}"))
}

pub fn dnsmasq_text(cfg: &Config) -> String {
    let mut s = String::from("# WifiCorrect üretir (panel → Yasaklı siteler). Alan adı ve alt adları NXDOMAIN döner.\n");
    for site in &cfg.filtre.siteler {
        s.push_str(&format!("address=/{site}/\n"));
    }
    s
}

pub fn iptables_text(cfg: &Config) -> String {
    let f = &cfg.filtre;
    let mut s = format!("*filter\n:{CHAIN} - [0:0]\n");
    for w in &f.istisnalar {
        s.push_str(&format!("-A {CHAIN} -m string --string \"{w}\" --algo bm --icase -j RETURN\n"));
    }
    for w in &f.kelimeler {
        // UDP: "port ulaşılamaz" → tarayıcı anında "site bulunamadı" der; TCP: bağlantı sıfırlanır
        s.push_str(&format!("-A {CHAIN} -p udp -m string --string \"{w}\" --algo bm --icase -j REJECT --reject-with icmp-port-unreachable\n"));
        s.push_str(&format!("-A {CHAIN} -p tcp -m string --string \"{w}\" --algo bm --icase -j REJECT --reject-with tcp-reset\n"));
    }
    s.push_str("COMMIT\n");
    s
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Listeleri uygular: dnsmasq dosyası (değiştiyse dnsmasq yeniden başlar), kelime zinciri ve müşteri DNS'inin zincire yönlendirilmesi.
pub fn uygula(cfg: &Config, conf: &Path, runner: &Runner) -> Result<(), String> {
    let text = dnsmasq_text(cfg);
    let changed = std::fs::read_to_string(conf).ok().as_deref() != Some(text.as_str());
    if changed {
        ortak::write_atomic(conf, text.as_bytes()).map_err(|e| format!("{} yazılamadı: {e}", conf.display()))?;
        let _ = std::fs::set_permissions(conf, std::os::unix::fs::PermissionsExt::from_mode(0o644));
    }
    let rules = std::env::temp_dir().join(format!("wfc-filtre-{}.rules", ortak::random_hex(4)));
    ortak::write_atomic(&rules, iptables_text(cfg).as_bytes()).map_err(|e| e.to_string())?;
    let ok = runner(&cmd(&["iptables-restore", "--noflush", &rules.to_string_lossy()]));
    let _ = std::fs::remove_file(&rules);
    if !ok {
        return Err("kelime filtresi yüklenemedi (iptables)".into());
    }
    for proto in ["udp", "tcp"] {
        let spec = ["-i", IFACE, "-p", proto, "--dport", "53", "-j", CHAIN];
        let with = |head: &[&str]| cmd(&[head, &spec[..]].concat());
        if !runner(&with(&["iptables", "-C", "INPUT"])) && !runner(&with(&["iptables", "-I", "INPUT", "1"])) {
            return Err("kelime filtresi bağlanamadı (iptables)".into());
        }
    }
    // açılışta dnsmasq henüz başlamamışsa başlatılmaz (try-restart)
    if changed && !runner(&cmd(&["systemctl", "try-restart", "dnsmasq"])) {
        return Err("dnsmasq yeniden başlatılamadı".into());
    }
    Ok(())
}

/// `iptables -L WFC_DNS -v -n -x` çıktısından kelime / istisna başına eşleşen sorgu sayısı.
pub fn counters(listing: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for line in listing.lines() {
        let Some(pkts) = line.split_whitespace().next().and_then(|p| p.parse::<u64>().ok()) else { continue };
        let Some(rest) = line.split_once("STRING match").map(|(_, r)| r.trim_start()) else { continue };
        if let Some(w) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
            *out.entry(w.to_string()).or_insert(0) += pkts;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn cfg(extra: &str) -> Config {
        Config::parse(&format!("[filtre]\n{extra}")).unwrap()
    }

    #[test]
    fn normalize_and_check() {
        assert_eq!(normalize_site("https://www.Bet365.com/tr?x=1").as_deref(), Some("www.bet365.com"));
        assert_eq!(normalize_site("bet365.com.").as_deref(), Some("bet365.com"));
        for bad in ["bet365", "a..com", "-a.com", "şans.com", "x.com\naddress=/y/1.2.3.4", ""] {
            assert!(normalize_site(bad).is_none(), "{bad}");
        }
        assert_eq!(normalize_word(" Bet ").as_deref(), Some("bet"));
        assert!(normalize_word("b").is_none() && normalize_word("bet.com").is_none() && normalize_word("b\"et").is_none());
        let yok = crate::liste::Yollar { dizin: "/yok/wfc-liste".into(), conf: "/yok/wfc-hazir.conf".into() }; // hazır liste yok
        let c = cfg("siteler = ['bet365.com']\nkelimeler = ['bet', 'porn']\nistisnalar = ['alphabet', 'better']\n");
        assert_eq!(check(&c, &yok, "m.bet365.com").as_deref(), Some("yasaklı site: bet365.com"));
        assert_eq!(check(&c, &yok, "superbet.com.tr").as_deref(), Some("yasaklı kelime: bet"));
        assert_eq!(check(&c, &yok, "WWW.PoRnHuB.CoM").as_deref(), Some("yasaklı kelime: porn"));
        assert_eq!(check(&c, &yok, "abc.alphabet.com"), None); // istisna
        assert_eq!(check(&c, &yok, "betterhelp.com"), None);
        assert_eq!(check(&c, &yok, "google.com"), None);
        assert_eq!(check(&c, &yok, "notbet365.com").as_deref(), Some("yasaklı kelime: bet")); // site değil (alt ad değil), kelime
        assert_eq!(check(&cfg(""), &yok, "bet365.com"), None); // boş liste: hiçbir şey engellenmez
    }

    #[test]
    fn generated_rules_and_apply() {
        let c = cfg("siteler = ['bet365.com']\nkelimeler = ['bet']\nistisnalar = ['alphabet']\n");
        assert_eq!(dnsmasq_text(&c).lines().nth(1), Some("address=/bet365.com/"));
        let r = iptables_text(&c);
        assert!(r.starts_with("*filter\n:WFC_DNS - [0:0]\n-A WFC_DNS -m string --string \"alphabet\" --algo bm --icase -j RETURN\n"));
        assert!(r.contains("-p udp -m string --string \"bet\"") && r.contains("tcp-reset") && r.ends_with("COMMIT\n"));
        assert_eq!(iptables_text(&cfg("")), "*filter\n:WFC_DNS - [0:0]\nCOMMIT\n");

        let dir = std::env::temp_dir().join(format!("wfc-filtre-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("yasak.conf");
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let jump_exists = Arc::new(Mutex::new(false));
        let j2 = jump_exists.clone();
        let runner = move |a: &[String]| {
            c2.lock().unwrap().push(a.join(" "));
            if a.get(1).is_some_and(|x| x == "-C") { *j2.lock().unwrap() } else { true }
        };
        uygula(&c, &conf, &runner).unwrap();
        {
            let v = calls.lock().unwrap();
            assert!(v.iter().any(|x| x == "iptables -I INPUT 1 -i br-hotspot -p udp --dport 53 -j WFC_DNS"));
            assert!(v.iter().any(|x| x == "systemctl try-restart dnsmasq"));
        }
        calls.lock().unwrap().clear();
        *jump_exists.lock().unwrap() = true;
        uygula(&c, &conf, &runner).unwrap(); // aynı liste: dnsmasq yeniden başlamaz, bağlantı iki kez eklenmez
        let v = calls.lock().unwrap();
        assert!(!v.iter().any(|x| x.contains("dnsmasq") || x.contains(" -I ")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn counter_parsing() {
        let out = "Chain WFC_DNS (2 references)\n    pkts      bytes target     prot opt in     out     source               destination\n\
                   \x20      3      180 RETURN     all  --  *      *       0.0.0.0/0            0.0.0.0/0            STRING match  \"alphabet\" ALGO name bm ICASE\n\
                   \x20     12      900 REJECT     udp  --  *      *       0.0.0.0/0            0.0.0.0/0            STRING match  \"bet\" ALGO name bm ICASE reject-with icmp-port-unreachable\n\
                   \x20      1       60 REJECT     tcp  --  *      *       0.0.0.0/0            0.0.0.0/0            STRING match  \"bet\" ALGO name bm ICASE reject-with tcp-reset\n";
        let c = counters(out);
        assert_eq!((c["bet"], c["alphabet"]), (13, 3));
    }
}
