//! Yönetim komutları (eski ctl.py). Bu adımda: dhcp-olay, yukle, oturumlar. Diğerleri (gun-kapat, yedekle, ara...) sonraki adımlarda.

use crate::ayar::Config;
use crate::ortak::{self, Row, Runner};
use std::process::ExitCode;

/// dnsmasq'ın betiğe verdiği kira bitişi (epoch): `DNSMASQ_LEASE_EXPIRES`, yoksa şimdi + `DNSMASQ_TIME_REMAINING`.
pub fn kira_bitisi(env: &dyn Fn(&str) -> Option<String>, now: f64) -> Option<f64> {
    let sayi = |k: &str| env(k).and_then(|v| v.trim().parse::<u64>().ok()).map(|v| v as f64);
    sayi("DNSMASQ_LEASE_EXPIRES").or_else(|| sayi("DNSMASQ_TIME_REMAINING").map(|r| now + r))
}

/// Son kaydedilen DHCP olayı (MAC başına): dnsmasq'ın SIGHUP/yeniden başlatmada bütün kiralar için `old` ile yeniden
/// çağırdığı betiği gerçek yenilemeden ayırmak için.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
struct DhcpSon {
    ip: String,
    bitis: f64,
    /// Temizlenmiş cihaz adı; eski durum dosyasında yok (None: karşılaştırılmaz, sonraki olayda tamamlanır).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
}

const DHCP_SON: &str = "dhcp_son.json";
/// `DNSMASQ_TIME_REMAINING`'den hesaplanan bitiş çağrı anına göre birkaç saniye oynar.
const BITIS_TOLERANS: f64 = 5.0;

/// `old` olayı son kaydedilenle aynı mı (IP ve kira bitişi değişmemiş: yeniden okutma, gerçek yenileme değil). Durum
/// dosyası her olayda güncellenir; `del` kaydı siler; bitişinden bir gün geçen kayıtlar atılır. dnsmasq betiği tek tek
/// (sırayla) çalıştırır, ayrıca kilit gerekmez.
fn dhcp_tekrar_mi(state_root: &str, action: &str, mac: &str, ip: &str, host: &str, bitis: Option<f64>, now: f64) -> bool {
    let yol = std::path::Path::new(state_root).join(DHCP_SON);
    let mut son: std::collections::BTreeMap<String, DhcpSon> = std::fs::read(&yol).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let once = son.clone();
    let tekrar = action == "old"
        && bitis.is_some_and(|b| {
            son.get(mac).is_some_and(|s| s.ip == ip && (s.bitis - b).abs() <= BITIS_TOLERANS && s.host.as_deref().is_none_or(|h| h == host))
        });
    match (action, bitis) {
        ("del", _) => {
            son.remove(mac);
        }
        (_, Some(b)) if !tekrar => {
            son.insert(mac.to_string(), DhcpSon { ip: ip.to_string(), bitis: b, host: Some(host.to_string()) });
        }
        ("old", Some(_)) => {
            // eski durum dosyasından gelen adsız kayıt: ad tamamlanır (satır yazılmaz)
            if let Some(s) = son.get_mut(mac).filter(|s| s.host.is_none()) {
                s.host = Some(host.to_string());
            }
        }
        _ => {}
    }
    son.retain(|_, s| s.bitis + 86_400.0 > now);
    if son != once {
        let _ = ortak::mkdirs(std::path::Path::new(state_root));
        if let Ok(b) = serde_json::to_vec(&son) {
            if let Err(e) = ortak::write_atomic(&yol, &b) {
                eprintln!("dhcp-olay: {} yazılamadı: {e}", yol.display());
            }
        }
    }
    tekrar
}

/// dnsmasq `dhcp-script` çağrısı: `<add|old|del> <mac> <ip> [hostname]` (Debian dnsmasq biçimi).
/// 5651 iç IP dağıtım kaydını (`dhcp.csv`) yazar; atama/yenilemede kayıtlı cihazın oturumunu yeni IP'ye taşır.
/// `bitis` (kira bitişi, bkz. `kira_bitisi`) biliniyorsa ve `old` olayında IP ile bitiş son kaydedilenle aynıysa hiçbir
/// şey yazılmaz ("tekrar"): dnsmasq SIGHUP (hazır liste) ve yeniden başlatmada bütün kiralar için betiği yeniden çağırır.
#[allow(clippy::too_many_arguments)]
pub fn dhcp_olay(cfg: &Config, action: &str, mac: &str, ip: &str, host: &str, bitis: Option<f64>, now: f64, runner: &Runner) -> &'static str {
    let olay = match action {
        "add" => "DHCP_ATAMA",
        "old" => "DHCP_YENILEME",
        "del" => "DHCP_BIRAKMA",
        _ => return "yok sayildi", // tftp, init vb.
    };
    let (Some(mac), true) = (ortak::norm_mac(mac), ortak::in_subnet(ip, &cfg.main.subnet)) else { return "gecersiz" };
    let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).take(63).collect();
    if dhcp_tekrar_mi(&cfg.main.state_root, action, &mac, ip, &host, bitis, now) {
        return "tekrar";
    }
    let zaman = ortak::now_iso(now);
    let row = Row::new(olay, &zaman).set("mac", &mac).set("ic_ip", ip).set("alan_adi", host);
    let path = ortak::day_file(&cfg.main.log_root, ortak::day_of(&zaman), "dhcp.csv");
    if let Err(e) = ortak::append_rows(&path, &[row]) {
        eprintln!("dhcp-olay: {} yazılamadı: {e}", path.display());
    }
    if action == "del" {
        return "kaydedildi";
    }
    // Cihaz eski IP'sinde ısrar etmez; havuzdan ne aldıysa oturum oraya taşınır, IP'yi tutan başka oturum beklemeye alınır.
    let m = &cfg.main;
    let Ok(_g) = ortak::state_lock(&m.state_root) else { return "kilit hatasi" };
    let mut ses = ortak::load_sessions(&m.state_root);
    let movable = ses.get(&mac).is_some_and(|s| s.expires_epoch > now && s.ip != ip);
    let result = if movable {
        if !ortak::move_session(&m.log_root, &mut ses, &mac, ip, now, runner) {
            return "nft hatasi";
        }
        "tasindi"
    } else if ses.iter().any(|(k, v)| *k != mac && v.ip == ip) {
        ortak::park_ip_holders(&m.log_root, &mut ses, &mac, ip, now, runner);
        "bosaltildi"
    } else {
        return "degisiklik yok";
    };
    if let Err(e) = ortak::save_sessions(&m.state_root, &ses) {
        eprintln!("dhcp-olay: oturum dosyası yazılamadı: {e}");
    }
    result
}

/// Açılışta / güvenlik duvarı yeniden yüklenince: izinli ve yasaklı kümeleri doldurur, süresi dolmamış oturumları
/// kalan süreyle nft'ye geri ekler (müşteri tekrar SMS istemez), süresi dolanları kapatır.
pub fn yukle(cfg: &Config, now: f64, runner: &Runner) -> usize {
    for mac in Config::devices(&cfg.allow).keys() {
        ortak::run_opt(runner, ortak::nft_set_cmd("add", "allow_mac", mac));
    }
    for mac in Config::devices(&cfg.ban).keys() {
        ortak::run_opt(runner, ortak::nft_set_cmd("add", "ban_mac", mac));
    }
    let m = &cfg.main;
    let Ok(_g) = ortak::state_lock(&m.state_root) else { return 0 };
    let mut ses = ortak::load_sessions(&m.state_root);
    let mut restored = 0;
    for mac in ses.keys().cloned().collect::<Vec<_>>() {
        let s = ses[&mac].clone();
        let left = s.expires_epoch - now;
        if left <= 5.0 {
            ortak::close_session(&m.log_root, &mut ses, &mac, "sure_doldu", now, runner);
        } else if s.ip.is_empty() {
            continue; // IP'siz beklemede: cihaz dönünce DHCP olayı yeni IP'ye taşır
        } else if ortak::run_opt(runner, ortak::nft_add_cmd(&mac, &s.ip, left)) {
            restored += 1;
        } else {
            ortak::close_session(&m.log_root, &mut ses, &mac, "yukleme_hatasi", now, runner);
        }
    }
    if let Err(e) = ortak::save_sessions(&m.state_root, &ses) {
        eprintln!("yukle: oturum dosyası yazılamadı: {e}");
    }
    restored
}

fn ap_denetim(cfg: &Config, now: f64, ek: String) {
    ortak::audit(&cfg.main.log_root, Row::new("AP_KURULUM_ADRES", &ortak::now_iso(now)).set("ek", ek));
}

/// Açılışta (`yukle`): yeniden başlatmayla giden AP geçici adresinin durum dosyası temizlenir (`neden=acilis`).
fn ap_acilis(cfg: &Config, y: &crate::ap::Yollar, ip_oku: &crate::ap::IpOku, now: f64) -> Option<String> {
    let a = crate::ap::acilis_temizle(cfg, y, ip_oku)?;
    ap_denetim(cfg, now, format!("kaldirildi adres={a} neden=acilis"));
    Some(a)
}

/// `ctl ap-adres-kaldir <elle|sure> [KIMLIK]` (zamanlayıcı `sure KIMLIK` ile çağırır).
fn ap_adres_kaldir(cfg: &Config, y: &crate::ap::Yollar, args: &[String], ip_oku: &crate::ap::IpOku, runner: &Runner, now: f64) -> ExitCode {
    let (Some(neden @ ("elle" | "sure")), kimlik) = (args.get(1).map(String::as_str), args.get(2).map(String::as_str)) else {
        eprintln!("Kullanım: wificorrect ctl ap-adres-kaldir <elle|sure> [KIMLIK]");
        return ExitCode::from(2);
    };
    match crate::ap::kaldir(cfg, y, neden, kimlik, ip_oku, runner) {
        Ok(Some(a)) => {
            ap_denetim(cfg, now, format!("kaldirildi adres={a} neden={neden}"));
            println!("{a} kaldırıldı");
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("açık geçici adres yok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// `ctl ap-adres-ekle ADRES [--sure SN]` (SN 60–1800): panelin konsol eşdeğeri.
fn ap_adres_ekle(cfg: &Config, y: &crate::ap::Yollar, args: &[String], ip_oku: &crate::ap::IpOku, runner: &Runner, now: f64) -> ExitCode {
    let sure = match args.iter().position(|a| a == "--sure").map(|i| args.get(i + 1).and_then(|v| v.parse::<u64>().ok())) {
        None => Some(crate::ap::SURE_SN),
        Some(v) => v.filter(|v| (60..=crate::ap::SURE_SN).contains(v)),
    };
    let (Some(adres), Some(sure)) = (args.get(1).filter(|a| !a.starts_with("--")), sure) else {
        eprintln!("Kullanım: wificorrect ctl ap-adres-ekle a.b.c.d/önek [--sure SN]  (SN 60-1800)");
        return ExitCode::from(2);
    };
    match crate::ap::ekle(cfg, y, adres, sure, ip_oku, runner, now) {
        Ok(g) => {
            ap_denetim(cfg, now, format!("eklendi adres={} sure={sure} kullanici=konsol", g.adres));
            println!("{} eklendi, {sure} sn sonra kalkar", g.adres);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

fn oturumlar(cfg: &Config, now: f64) {
    let ses = ortak::load_sessions(&cfg.main.state_root);
    if ses.is_empty() {
        println!("Aktif oturum yok");
        return;
    }
    println!("{:<11} {:<24} {:<17} {:<15} {:<16} KALAN", "TELEFON", "AD SOYAD", "MAC", "IP", "BASLANGIC");
    let mut list: Vec<_> = ses.iter().collect();
    list.sort_by(|a, b| a.1.start_epoch.total_cmp(&b.1.start_epoch));
    for (mac, s) in list {
        let left = ((s.expires_epoch - now).max(0.0) as u64) / 60;
        let name: String = format!("{} {}", s.ad, s.soyad).chars().take(24).collect();
        let start = s.start.get(..16).unwrap_or("").replace('T', " ");
        let ip = if s.ip.is_empty() { "(beklemede)" } else { &s.ip };
        println!("{:<11} {name:<24} {mac:<17} {ip:<15} {start:<16} {}g {}sa {}dk", s.phone, left / 1440, left / 60 % 24, left % 60);
    }
}

/// Panel yavaşlatma ekleyip kaldırınca: kaydediciyi beklemeden bir kez uzlaştırır (ip: oturumlar + kirada izinli cihazlar).
fn hiz_uygula(cfg: &Config, wan: &str, runner: &Runner, nft_f: &dyn Fn(&str) -> bool, uygulanan: &std::path::Path) -> ExitCode {
    let m = &cfg.main;
    let ses = ortak::load_sessions(&m.state_root);
    let kiralar = ortak::parse_leases(&std::fs::read_to_string(&m.leases_file).unwrap_or_default());
    let ip_mac = crate::trafik::ip_mac(ses.iter().map(|(mac, s)| (&s.ip, mac)), &Config::devices(&cfg.allow), &kiralar);
    let ip_of = ip_mac.into_iter().map(|(ip, mac)| (mac, ip)).collect();
    match crate::hiz::uygula(cfg, wan, &ip_of, runner, nft_f, uygulanan) {
        Ok(degisti) => {
            println!("{}", if degisti { "uygulandı" } else { "değişiklik yok" });
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// Parolayı ekrana yazdırmadan okur (terminal değilse düz satır: kurulum betikleri için).
fn read_secret(prompt: &str) -> String {
    use std::io::Write;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    // SAFETY: termios yalnızca stdin için okunur/yazılır, okuma bitince eski hali geri konur.
    let mut old: libc::termios = unsafe { std::mem::zeroed() };
    let tty = unsafe { libc::tcgetattr(0, &mut old) } == 0;
    if tty {
        let mut t = old;
        t.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &t) };
    }
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    if tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &old) };
        eprintln!();
    }
    line.trim_end_matches(['\r', '\n']).to_string()
}

/// `admin` (hizmet sağlayıcı) parolası: yalnızca cihaz konsolundan / SSH'tan, root olarak.
fn admin_parola(path: &str) -> ExitCode {
    let pw = read_secret("Yeni admin parolası: ");
    if let Some(e) = crate::hesap::password_problem(&pw) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    if read_secret("Tekrar: ") != pw {
        eprintln!("Parolalar aynı değil.");
        return ExitCode::from(1);
    }
    match crate::hesap::Hesaplar::new(path).set_admin(&pw) {
        Ok(()) => {
            println!("admin parolası kaydedildi.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// Fabrika dönüşü (panel ya da merkezin serbest bırakması): kayıtlar teslim edilince merkeze `temizlendi` bildirilir.
/// Müşteriye bağlı cihaz elle sıfırlanmaz (merkezde sahipsiz bağlı kalırdı): önce yönetim merkezinden serbest bırakılır.
fn fabrika_calistir(cfg: &Config, now: f64, runner: &Runner, serbest: bool) -> ExitCode {
    if !serbest && crate::merkez::oku(std::path::Path::new(crate::merkez::PATH)).is_some() {
        eprintln!("Cihaz bir müşteriye bağlı: fabrika ayarına dönmek için önce yönetim merkezinden serbest bırakın.");
        return ExitCode::from(1);
    }
    let path = std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string());
    let bildir = |m: &crate::merkez::Merkez| {
        crate::merkez::temizlendi_bildir(m, &crate::merkez::curl, 3, &|| std::thread::sleep(std::time::Duration::from_secs(5)))
    };
    match crate::fabrika::fabrika(
        cfg,
        &path,
        crate::hesap::PATH,
        std::path::Path::new(crate::merkez::PATH),
        &bildir,
        std::path::Path::new(crate::filtre::DNSMASQ_CONF),
        &crate::liste::Yollar::sistem(),
        &crate::ag::Yollar::sistem(),
        &crate::ap::Yollar::sistem(),
        &|c: &[&str]| ortak::capture(c),
        now,
        runner,
    ) {
        Ok(errs) if errs.is_empty() => {
            println!("Fabrika ayarlarına dönüldü.");
            ExitCode::SUCCESS
        }
        Ok(errs) => {
            eprintln!("Fabrika ayarlarına dönüldü, uyarılar: {}", errs.join("; "));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// `ctl ara` seçeneği → (`kayit::ara` türü, değer). Pasaportta `--uyruk XX` varsa değer `XX:NO` olur.
fn ara_secenegi(args: &[String]) -> Option<(&'static str, String)> {
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (tur, deger) = ["ic-ip", "nat-port", "hedef-ip", "telefon", "mac", "tc", "pasaport", "oda"]
        .iter()
        .find_map(|t| opt(&format!("--{t}")).map(|v| (*t, v)))?;
    match (tur, opt("--uyruk")) {
        ("pasaport", Some(u)) => Some((tur, format!("{u}:{deger}"))),
        _ => Some((tur, deger)),
    }
}

/// Merkezden gelen deneme modu `ek_uygula` ile `yeni`ye uygulandıysa ve değiştiyse ayar dosyasına (atomik) yazılır.
/// Portal yeniden başlatılmaz: deneme modu yalnızca Özet ve sağlık raporunda kullanılır (bekleyen kodlar düşmesin).
fn sms_sonucu_kaydet(sms: Result<bool, String>, yeni: &Config, cfg_path: &str, denetim: &dyn Fn(&str, String)) {
    match sms {
        Ok(true) => match yeni.save(cfg_path) {
            Ok(()) => denetim("SMS_AYARI_MERKEZ", format!("mock={}", u8::from(yeni.sms.mock))),
            Err(e) => denetim("MERKEZ_EK_HATA", e),
        },
        Ok(false) => {}
        Err(e) => denetim("MERKEZ_EK_HATA", e),
    }
}

/// `ayar::Config::eski_sirlari_temizle` (postinst ve açılış): hata tek satır, çıkış etkilenmez.
fn eski_sirlari_temizle() {
    let cfg_path = std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string());
    match Config::eski_sirlari_temizle(&cfg_path) {
        Ok(true) => println!("ayar dosyasındaki eski SMS bilgileri silindi"),
        Ok(false) => {}
        Err(e) => eprintln!("eski SMS bilgileri silinemedi: {e}"),
    }
}

pub fn run(cfg: Config, args: &[String]) -> ExitCode {
    let runner = |c: &[String]| ortak::run(c);
    let now = ortak::wall();
    match args.first().map(String::as_str) {
        Some("dhcp-olay") if args.len() >= 4 => {
            let host = args.get(4).map(String::as_str).unwrap_or("");
            let bitis = kira_bitisi(&|k| std::env::var(k).ok(), now);
            println!("{}", dhcp_olay(&cfg, &args[1], &args[2], &args[3], host, bitis, now, &runner));
            ExitCode::SUCCESS
        }
        Some("yukle") => {
            eski_sirlari_temizle(); // paket kurulumunda yapılamadıysa açılışta
            println!("{} oturum geri yüklendi", yukle(&cfg, now, &runner));
            if let Some(a) = ap_acilis(&cfg, &crate::ap::Yollar::sistem(), &|c: &[&str]| ortak::capture(c), now) {
                println!("AP geçici adresi {a} artık yok; durum temizlendi");
            }
            ExitCode::SUCCESS
        }
        // Panel → Admin → AP kurulumu eşdeğeri (konsol) ve kaldırma zamanlayıcısı
        Some("ap-adres-ekle") => ap_adres_ekle(&cfg, &crate::ap::Yollar::sistem(), args, &|c: &[&str]| ortak::capture(c), &runner, now),
        Some("ap-adres-kaldir") => ap_adres_kaldir(&cfg, &crate::ap::Yollar::sistem(), args, &|c: &[&str]| ortak::capture(c), &runner, now),
        // Paket kurulumu (postinst): 0.4.8 öncesinden ayar dosyasında kalan SMS sırları
        Some("ayar-temizle") => {
            eski_sirlari_temizle();
            ExitCode::SUCCESS
        }
        Some("oturumlar") => {
            oturumlar(&cfg, now);
            ExitCode::SUCCESS
        }
        Some("gun-kapat") => {
            let day = args.iter().skip(1).find(|a| !a.starts_with("--")).map(String::as_str);
            let out = crate::muhur::gun_kapat(&cfg, day, now, args.iter().any(|a| a == "--zorla"));
            println!("{}", if out.is_empty() { "Kapatılacak gün yok".to_string() } else { out.join("\n") });
            if out.iter().any(|l| l.contains("HATA")) { ExitCode::from(1) } else { ExitCode::SUCCESS }
        }
        Some("dogrula") => {
            let last = args.iter().position(|a| a == "--son").and_then(|i| args.get(i + 1)).and_then(|n| n.parse().ok());
            let r = crate::muhur::verify(&cfg, last);
            println!("{}", crate::muhur::format_verify(&r));
            if r.iter().any(|(_, p, _)| !p.is_empty()) { ExitCode::from(1) } else { ExitCode::SUCCESS }
        }
        Some("temizle") => {
            let kuru = args.iter().any(|a| a == "--kuru");
            let mut out = crate::muhur::prune(&cfg, now, kuru);
            // bitişinden 30 gün geçmiş misafir kodları (tam kimlik) her gece silinir; kod üretilmese de birikmesin
            if !kuru {
                if let Ok(_g) = ortak::state_lock(&cfg.main.state_root) {
                    if let Ok(mut k) = crate::misafir::oku_kesin(&cfg.main.state_root) {
                        let once = k.len();
                        crate::misafir::temizle(&mut k, now);
                        if k.len() < once && crate::misafir::kaydet(&cfg.main.state_root, &k).is_ok() {
                            out.push(format!("{} eski misafir kodu silindi", once - k.len()));
                        }
                    }
                }
            }
            println!("{}", if out.is_empty() { "Silinecek kayıt yok".to_string() } else { out.join("\n") });
            ExitCode::SUCCESS
        }
        Some("admin-parola") => admin_parola(crate::hesap::PATH),
        // Panel → Admin ayarları → Fabrika ayarları (ayrı systemd işinde; ağ yeniden kurulur)
        Some("fabrika") => fabrika_calistir(&cfg, now, &runner, false),
        // Kaydedici zamanı gelince başlatır (her gün 06:00 sonrası; hata olursa 30 dk sonra yeniden)
        Some("merkez-eslesme") => {
            let p = std::path::Path::new(crate::merkez::PATH);
            let Some(mut m) = crate::merkez::oku(p) else {
                println!("cihaz bir müşteriye bağlı değil");
                return ExitCode::SUCCESS;
            };
            m.deneme = now;
            let _ = crate::merkez::kaydet(p, &m);
            let denetim = |olay: &str, ek: String| ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
            match crate::merkez::eslesme(&mut m, &cfg.main.site_name, &cfg.main.unvan, crate::ayar::isletme_turu_secili(&cfg).unwrap_or(""), false, &crate::merkez::curl, now) {
                Ok(crate::merkez::Eslesme::Bagli { uyelik, ek }) => {
                    if let Err(e) = crate::merkez::kaydet(p, &m) {
                        eprintln!("{e}");
                        return ExitCode::from(1);
                    }
                    denetim("MERKEZ_ESLESME", format!("uyelik={uyelik}"));
                    let cfg_path = std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string());
                    let mut yeni = cfg.clone();
                    let (admin, sms) = crate::merkez::ek_uygula(&mut yeni, &crate::hesap::Hesaplar::new(crate::hesap::PATH), &ek);
                    match admin {
                        Ok(true) => denetim("ADMIN_PAROLA_MERKEZ", String::new()),
                        Ok(false) => {}
                        Err(e) => denetim("MERKEZ_EK_HATA", e),
                    }
                    sms_sonucu_kaydet(sms, &yeni, &cfg_path, &denetim);
                    ExitCode::SUCCESS
                }
                Ok(crate::merkez::Eslesme::Serbest) => {
                    denetim("MERKEZ_SERBEST", "fabrika=basladi".into());
                    fabrika_calistir(&cfg, now, &runner, true) // teslim olmazsa bağ kalır, 30 dk sonra yeniden denenir
                }
                Ok(crate::merkez::Eslesme::Taninmadi) => {
                    // cihaz silinmez: merkezdeki bir hata bütün cihazları sıfırlamasın; admin yönetim merkezinden bakar
                    denetim("MERKEZ_TANINMADI", String::new());
                    ExitCode::from(1)
                }
                Err(e) => {
                    denetim("MERKEZ_ESLESME_HATA", format!("hata={e}"));
                    ExitCode::from(1)
                }
            }
        }
        // Kaydedici 5 dk'da bir başlatır: bağlı değilse sessiz; gönderilemezse tek satır (merkez raporun gelmediğini görür)
        Some("saglik") => {
            let http = |y: &str, g: &str| crate::merkez::curl_sure(y, g, 10);
            let k = crate::saglik::Kaynak::sistem();
            let mut liste_isi = false;
            match crate::saglik::calistir(&cfg, std::path::Path::new(crate::merkez::PATH), &k, &http, now) {
                Ok(cevap) => {
                    // merkezin etkin deneme modu (Özet uyarısı, sms_sorunu): yalnızca değiştiyse kaydedilir
                    if cevap.sms_mock.is_some() {
                        let cfg_path = std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string());
                        let mut yeni = cfg.clone();
                        let ek = crate::merkez::Ek { admin: None, sms_mock: cevap.sms_mock };
                        let (_, sms) = crate::merkez::ek_uygula(&mut yeni, &crate::hesap::Hesaplar::new(crate::hesap::PATH), &ek);
                        let denetim = |olay: &str, ek: String| ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
                        sms_sonucu_kaydet(sms, &yeni, &cfg_path, &denetim);
                    }
                    if let Some(h) = cevap.hedef {
                        // karar Kur ise hedef.json yazılır ve güncelleme ayrı systemd işinde başlar (birim varsa systemd reddeder)
                        let kurulu = crate::guncelleme::kurulu(&*k.komut);
                        let baslat = || {
                            let c: Vec<String> = ["systemd-run", "--collect", "--unit", "wfc-guncelle", "/usr/local/bin/wificorrect", "ctl", "guncelle"].map(String::from).to_vec();
                            ortak::run(&c);
                        };
                        crate::guncelleme::hedefi_isle(&k.guncelleme, &h, &kurulu, now, &baslat);
                    }
                    if let Some(l) = cevap.listeler {
                        // hazır engel listeleri: açık bir kategorinin sürümü değiştiyse ayrı işte indirilir
                        let baslat = || {
                            let c: Vec<String> = ["systemd-run", "--collect", "--unit", "wfc-liste", "/usr/local/bin/wificorrect", "ctl", "liste-guncelle"].map(String::from).to_vec();
                            ortak::run(&c);
                        };
                        liste_isi = crate::liste::hedefi_isle(&crate::liste::Yollar::sistem(), &l, &crate::liste::acik_harita(&cfg), now, &baslat);
                    }
                }
                Err(e) => eprintln!("sağlık raporu gönderilemedi: {e}"),
            }
            // dnsmasq hazır listeyi yeniden okuyamadıysa (HUP başarısız) merkeze ulaşılamasa da yeniden denenir
            let baslat = || {
                let c: Vec<String> = ["systemd-run", "--collect", "--unit", "wfc-liste-uygula", "/usr/local/bin/wificorrect", "ctl", "liste-uygula"].map(String::from).to_vec();
                ortak::run(&c);
            };
            crate::liste::hup_yeniden(&crate::liste::Yollar::sistem(), liste_isi, &baslat);
            ExitCode::SUCCESS
        }
        // `ctl saglik` ya da panel başlatır (systemd-run --unit wfc-liste): eksik/eski açık hazır listeleri indirir, uygular
        Some("liste-guncelle") => {
            let Some(m) = crate::merkez::oku(std::path::Path::new(crate::merkez::PATH)) else {
                eprintln!("cihaz merkeze bağlı değil");
                return ExitCode::from(1);
            };
            let y = crate::liste::Yollar::sistem();
            let indir = |yol: &str, g: &str, c: &std::path::Path| crate::merkez::indir(yol, g, c, 300);
            let alinan = crate::liste::guncelle(&y, &cfg.main.log_root, &m, &crate::liste::hedef_oku(&y), &crate::liste::acik_harita(&cfg), &indir, now);
            let cfg_path = std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string());
            match crate::liste::uygula_guncel(&cfg_path, &cfg, &y, &runner) {
                Ok(n) => {
                    println!("{} liste indirildi; {n} alan adı engelli", alinan.len());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        // Panel (systemd-run --unit wfc-liste-uygula; aç/kapat, istisna), fabrika dönüşü, paket kurulumu ve elle: birleşik
        // hazır liste dosyasını güncel ayarla yeniden yazar (dnsmasq SIGHUP; önceki okutma başarısızsa yeniden dener)
        Some("liste-uygula") => match crate::liste::uygula_guncel(
            &std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string()),
            &cfg,
            &crate::liste::Yollar::sistem(),
            &runner,
        ) {
            Ok(n) => {
                println!("{n} alan adı engelli");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        // `ctl saglik` başlatır (systemd-run --unit wfc-guncelle): hedef sürümü indirir, doğrular, kurar
        Some("guncelle") => {
            let Some(m) = crate::merkez::oku(std::path::Path::new(crate::merkez::PATH)) else {
                eprintln!("cihaz merkeze bağlı değil");
                return ExitCode::from(1);
            };
            let indir = |y: &str, g: &str, c: &std::path::Path| crate::merkez::indir(y, g, c, 300);
            // apt-get uzun sürebilir (bağımlılıklar Debian deposundan iner)
            let komut = |c: &[&str]| ortak::run_timeout(&c.iter().map(|s| s.to_string()).collect::<Vec<_>>(), 3600);
            let kurulu = crate::guncelleme::kurulu(&|c: &[&str]| ortak::capture_timeout(c, 30));
            match crate::guncelleme::calistir_guncelle(&cfg, &crate::guncelleme::Yollar::sistem(), &m, &indir, &komut, &kurulu, now) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        // Panel → Admin ayarları → Uzak erişim kaydedilince (ayrı systemd işinde)
        Some("uzak-uygula") => {
            let r = crate::uzak::uygula(&cfg, std::path::Path::new(crate::uzak::KEY), std::path::Path::new(crate::uzak::CONF), &runner);
            crate::uzak::audit(&cfg, now, &r);
            match r {
                Ok(m) => {
                    println!("{m}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        // Panel → Bağlı kullanıcılar → Yavaşlat / Yavaşlatmayı kaldır
        Some("hiz-uygula") => {
            let wan = crate::ag::load(&crate::ag::Yollar::sistem().ag).wan;
            hiz_uygula(&cfg, &wan, &runner, &crate::hiz::nft_dosya, std::path::Path::new(crate::hiz::UYGULANAN_YOLU))
        }
        // Açılışta (wificorrect-guvenlik) ve panelden: yasaklı site / kelime listelerini uygula
        Some("filtre-uygula") => match crate::filtre::uygula(&cfg, std::path::Path::new(crate::filtre::DNSMASQ_CONF), &runner) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        // Kurulum paketi (postinst): ilk açılıştan önce ağ rolleri ve dosyaları; ag.toml varsa dokunmaz
        Some("ag-ilk") => {
            let rota = std::process::Command::new("ip").args(["-o", "route", "show", "default"]).output()
                .ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).and_then(|s| crate::ag::rota_arayuzu(&s));
            match crate::ag::ilk(&cfg, &crate::ag::Yollar::sistem(), std::path::Path::new("/sys/class/net"), rota.as_deref()) {
                Ok(yazildi) => {
                    println!("{}", if yazildi { "ağ rolleri yazıldı" } else { "ag.toml var, dokunulmadı" });
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Some(c @ ("ag-uygula" | "ag-gecis" | "ag-onayla" | "ag-geri-al" | "ag-acilis")) => {
            use crate::ag;
            let y = ag::Yollar::sistem();
            let report = |r: Result<Vec<String>, String>| match r {
                Ok(errs) if errs.is_empty() => ExitCode::SUCCESS,
                Ok(errs) => {
                    eprintln!("{}", errs.join("; "));
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            };
            match c {
                // Kurulumda / elle: ag.toml'dan dosyaları üret ve ağı geçir (geri alma zamanlayıcısı yok)
                "ag-uygula" => report(Ok(ag::switch(&cfg, &ag::load(&y.ag), &y, &runner))),
                "ag-gecis" => match args.get(1) {
                    Some(f) => report(ag::gecis(&cfg, &y, std::path::Path::new(f), now, &runner)),
                    None => report(Err("Kullanım: wificorrect ctl ag-gecis <yeni-ag.toml>".into())),
                },
                "ag-onayla" => report(ag::onayla(&cfg, &y, &runner).map(|_| vec![])),
                "ag-geri-al" => report(ag::geri_al(&cfg, &y, args.get(1).map_or("elle", String::as_str), &runner)),
                _ => report(ag::geri_al(&cfg, &y, "acilis", &runner)),
            }
        }
        Some("ara") => {
            let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str);
            let Some((tur, deger)) = ara_secenegi(args) else {
                eprintln!("Kullanım: wificorrect ctl ara (--ic-ip IP | --nat-port PORT | --hedef-ip IP) --zaman \"2026-09-24 14:30\" [--tolerans SN]");
                eprintln!("          wificorrect ctl ara --telefon NUMARA | --mac MAC [--zaman ...]");
                eprintln!("          wificorrect ctl ara --tc TCKN | --pasaport NO [--uyruk XX] | --oda NO [--zaman ...]");
                return ExitCode::from(2);
            };
            let deger = deger.as_str();
            let tol = opt("--tolerans").and_then(|v| v.parse().ok()).unwrap_or(120.0);
            match crate::kayit::ara(&cfg, tur, deger, opt("--zaman").unwrap_or(""), tol, now) {
                Ok(sections) => {
                    for (title, rows) in sections {
                        println!("{title}:\n{}\n", crate::kayit::format_rows(&rows));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(2)
                }
            }
        }
        Some("disa-aktar") => {
            let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str);
            let (Some(bas), Some(bit), Some(out)) = (opt("--baslangic"), opt("--bitis"), opt("--cikti")) else {
                eprintln!("Kullanım: wificorrect ctl disa-aktar --baslangic YYYY-AA-GG --bitis YYYY-AA-GG --cikti /tmp/talep.tar");
                return ExitCode::from(2);
            };
            match crate::kayit::talep_paketi(&cfg, bas, bit, std::path::Path::new(out)) {
                Ok(n) => {
                    println!("Paket: {out} ({n} gün)");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(1)
                }
            }
        }
        Some("yedekle") => {
            // ponytail: bir gün 1 saatte gitmezse YEDEK_HATA; ertesi gece kaldığı yerden devam eder
            // sonuç denetime: panel → Admin ayarları "Son yedek" bunu gösterir
            let r = crate::muhur::backup(&cfg, now, &|c: &[String]| ortak::run_timeout(c, 3600));
            let (ek, code) = match &r {
                Ok(m) => (format!("sonuc={m}"), ExitCode::SUCCESS),
                Err(m) => (format!("hata={m}"), ExitCode::from(1)),
            };
            ortak::audit(&cfg.main.log_root, Row::new("YEDEK_SONUC", &ortak::now_iso(now)).set("ek", ek));
            match r {
                Ok(m) => println!("{m}"),
                Err(m) => eprintln!("YEDEK HATASI: {m}"),
            }
            code
        }
        _ => {
            eprintln!(
                "Kullanım: wificorrect ctl <komut>\n  dhcp-olay <add|old|del> <mac> <ip> [ad]\n  yukle\n  oturumlar\n  \
                 gun-kapat [YYYY-AA-GG] [--zorla]\n  dogrula [--son N]\n  temizle [--kuru]\n  yedekle\n  ara ...  (ayrıntı: ctl ara)\n  disa-aktar --baslangic G --bitis G --cikti DOSYA\n  ag-uygula | ag-gecis DOSYA | ag-onayla | ag-geri-al | ag-ilk\n  ayar-temizle\n  filtre-uygula\n  liste-uygula | liste-guncelle\n  hiz-uygula\n  admin-parola\n  merkez-eslesme\n  saglik\n  guncelle\n  ap-adres-ekle a.b.c.d/önek [--sure SN]\n  ap-adres-kaldir <elle|sure> [KIMLIK]"
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn ara_secenekleri_kimlik_ve_oda() {
        let v = |a: &[&str]| ara_secenegi(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["ara", "--tc", "10000000146"]), Some(("tc", "10000000146".into())));
        assert_eq!(v(&["ara", "--pasaport", "X1234567", "--uyruk", "DE"]), Some(("pasaport", "DE:X1234567".into())));
        assert_eq!(v(&["ara", "--pasaport", "X1234567"]), Some(("pasaport", "X1234567".into())));
        assert_eq!(v(&["ara", "--oda", "101", "--zaman", "2026-09-27 12:00"]), Some(("oda", "101".into())));
        assert_eq!(v(&["ara", "--telefon", "05334553132"]), Some(("telefon", "05334553132".into())));
        assert_eq!(v(&["ara", "--uyruk", "DE"]), None);
    }

    const M1: &str = "aa:bb:cc:dd:ee:01";
    const M2: &str = "aa:bb:cc:dd:ee:02";

    fn cfg() -> (Config, std::path::PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-ctl-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        let text = format!(
            "[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\nleases_file = '{0}/leases'\n[[allow]]\nmac = 'aa:bb:cc:dd:ee:99'\n[[ban]]\nmac = 'aa:bb:cc:dd:ee:66'\n",
            root.display()
        );
        (Config::parse(&text).unwrap(), root)
    }

    fn sess(ip: &str, expires: f64) -> ortak::Session {
        ortak::Session {
            phone: "5334553132".into(),
            ad: "A".into(),
            soyad: "B".into(),
            ip: ip.into(),
            session_id: "s1".into(),
            start: ortak::now_iso(1000.0),
            start_epoch: 1000.0,
            expires_epoch: expires,
        }
    }

    const NOW: f64 = 1_790_705_134.0;

    #[test]
    fn dhcp_event_logs_and_moves_session() {
        let (c, root) = cfg();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let c2 = calls.clone();
        let runner = move |x: &[String]| {
            c2.lock().unwrap().push(x.to_vec());
            true
        };
        let mut ses = ortak::Sessions::new();
        ses.insert(M1.into(), sess("10.50.0.23", NOW + 3600.0));
        ses.insert(M2.into(), sess("10.50.0.30", NOW + 3600.0));
        ortak::save_sessions(&c.main.state_root, &ses).unwrap();
        assert_eq!(dhcp_olay(&c, "add", "AA:BB:CC:DD:EE:01", "10.50.0.30", "iPhone;=x", None, NOW, &runner), "tasindi");
        let ses = ortak::load_sessions(&c.main.state_root);
        assert_eq!((ses[M1].ip.as_str(), ses[M2].ip.as_str()), ("10.50.0.30", "")); // eski sahibi beklemeye
        let dhcp = std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/dhcp.csv")).unwrap();
        assert!(dhcp.contains("2026-09-29T21:05:34+03:00;DHCP_ATAMA;;;;aa:bb:cc:dd:ee:01;10.50.0.30;;;;;;;iPhonex;"));
        assert!(calls.lock().unwrap().iter().any(|c| c[1] == "add" && c[6].contains("10.50.0.30")));
        assert_eq!(dhcp_olay(&c, "old", M1, "10.50.0.30", "", None, NOW, &runner), "degisiklik yok");
        assert_eq!(dhcp_olay(&c, "del", M1, "10.50.0.30", "", None, NOW, &runner), "kaydedildi");
        assert_eq!(dhcp_olay(&c, "add", "bozuk", "10.50.0.30", "", None, NOW, &runner), "gecersiz");
        assert_eq!(dhcp_olay(&c, "add", M1, "192.168.1.5", "", None, NOW, &runner), "gecersiz");
        assert_eq!(dhcp_olay(&c, "tftp", M1, "10.50.0.30", "", None, NOW, &runner), "yok sayildi");
        let dhcp = std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/dhcp.csv")).unwrap();
        assert!(dhcp.contains(";DHCP_YENILEME;") && dhcp.contains(";DHCP_BIRAKMA;"));
    }

    /// dnsmasq SIGHUP'ta (hazır liste yeniden okutma) ve yeniden başlatmada bütün kiralar için betiği `old` ile yeniden
    /// çağırır: kira bitişi ve IP aynıysa 5651 kaydına sahte DHCP_YENILEME yazılmaz, oturuma dokunulmaz.
    #[test]
    fn dhcp_tekrar_eden_old_yazilmaz() {
        let (c, root) = cfg();
        let runner = |_: &[String]| true;
        let satir = || {
            std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/dhcp.csv")).unwrap_or_default().lines().filter(|l| l.contains(";DHCP_")).count()
        };
        let bitis = Some(NOW + 7200.0);
        assert_ne!(dhcp_olay(&c, "add", M1, "10.50.0.30", "", bitis, NOW, &runner), "tekrar");
        assert_eq!(satir(), 1);
        // HUP / yeniden başlatma: aynı IP, aynı kira bitişi → satır yok
        assert_eq!(dhcp_olay(&c, "old", M1, "10.50.0.30", "", bitis, NOW + 60.0, &runner), "tekrar");
        assert_eq!(dhcp_olay(&c, "old", M1, "10.50.0.30", "", bitis, NOW + 90.0, &runner), "tekrar");
        assert_eq!(satir(), 1);
        // DNSMASQ_TIME_REMAINING'den hesaplanan bitiş birkaç saniye oynayabilir
        assert_eq!(dhcp_olay(&c, "old", M1, "10.50.0.30", "", Some(NOW + 7201.0), NOW + 95.0, &runner), "tekrar");
        // gerçek yenileme: bitiş uzar → yazılır
        assert_ne!(dhcp_olay(&c, "old", M1, "10.50.0.30", "", Some(NOW + 3600.0 + 7200.0), NOW + 3600.0, &runner), "tekrar");
        assert_eq!(satir(), 2);
        // aynı bitiş ama farklı IP → yazılır
        assert_ne!(dhcp_olay(&c, "old", M1, "10.50.0.31", "", Some(NOW + 3600.0 + 7200.0), NOW + 3601.0, &runner), "tekrar");
        assert_eq!(satir(), 3);
        // add ve del her zaman yazılır (aynı bitişle bile)
        dhcp_olay(&c, "add", M1, "10.50.0.31", "", Some(NOW + 3600.0 + 7200.0), NOW + 3602.0, &runner);
        dhcp_olay(&c, "del", M1, "10.50.0.31", "", Some(NOW + 3600.0 + 7200.0), NOW + 3603.0, &runner);
        dhcp_olay(&c, "del", M1, "10.50.0.31", "", Some(NOW + 3600.0 + 7200.0), NOW + 3604.0, &runner);
        assert_eq!(satir(), 6);
        // bırakmadan sonra aynı bitişle gelen old artık tekrar değildir (kayıt silindi)
        assert_ne!(dhcp_olay(&c, "old", M1, "10.50.0.31", "", Some(NOW + 3600.0 + 7200.0), NOW + 3605.0, &runner), "tekrar");
        assert_eq!(satir(), 7);
        // bitiş bilinmiyorsa (ortam değişkeni yok) bugünkü gibi her zaman yazılır
        assert_ne!(dhcp_olay(&c, "old", M2, "10.50.0.40", "", None, NOW, &runner), "tekrar");
        assert_ne!(dhcp_olay(&c, "old", M2, "10.50.0.40", "", None, NOW, &runner), "tekrar");
        assert_eq!(satir(), 9);
        // aynı IP ve bitiş ama cihaz adı değişti (temizlenmiş adla karşılaştırılır) → yazılır
        dhcp_olay(&c, "add", M1, "10.50.0.50", "iPhone;", Some(NOW + 9000.0), NOW + 3700.0, &runner);
        let n = satir();
        assert_eq!(dhcp_olay(&c, "old", M1, "10.50.0.50", "iPhone", Some(NOW + 9000.0), NOW + 3701.0, &runner), "tekrar");
        assert_ne!(dhcp_olay(&c, "old", M1, "10.50.0.50", "Galaxy", Some(NOW + 9000.0), NOW + 3702.0, &runner), "tekrar");
        assert_eq!(satir(), n + 1);
        // eski durum dosyası (cihaz adı yok): ad karşılaştırılmaz, tamamlanır
        let yol = root.join("state/dhcp_son.json");
        std::fs::write(&yol, format!(r#"{{"{M2}":{{"ip":"10.50.0.60","bitis":{}}}}}"#, NOW + 9000.0)).unwrap();
        assert_eq!(dhcp_olay(&c, "old", M2, "10.50.0.60", "pc", Some(NOW + 9000.0), NOW + 3703.0, &runner), "tekrar");
        assert!(std::fs::read_to_string(&yol).unwrap().contains(r#""host":"pc""#));
        assert_ne!(dhcp_olay(&c, "old", M2, "10.50.0.60", "laptop", Some(NOW + 9000.0), NOW + 3704.0, &runner), "tekrar");
        // eski kayıtlar birikmez: bitişinden bir gün sonra durum dosyasından düşer
        dhcp_olay(&c, "add", M2, "10.50.0.40", "", Some(NOW + 100.0), NOW, &runner);
        dhcp_olay(&c, "add", M1, "10.50.0.31", "", Some(NOW + 200_000.0), NOW + 100_000.0, &runner);
        let son = std::fs::read_to_string(root.join("state/dhcp_son.json")).unwrap();
        assert!(son.contains(M1) && !son.contains(M2), "{son}");
    }

    #[test]
    fn dhcp_kira_bitisi_ortamdan() {
        let e = |a: &[(&str, &str)]| {
            let m: std::collections::HashMap<String, String> = a.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            move |k: &str| m.get(k).cloned()
        };
        assert_eq!(kira_bitisi(&e(&[("DNSMASQ_LEASE_EXPIRES", "1790712334")]), NOW), Some(1_790_712_334.0));
        assert_eq!(kira_bitisi(&e(&[("DNSMASQ_TIME_REMAINING", "7200")]), NOW), Some(NOW + 7200.0));
        assert_eq!(kira_bitisi(&e(&[("DNSMASQ_LEASE_EXPIRES", "bozuk")]), NOW), None);
        assert_eq!(kira_bitisi(&e(&[]), NOW), None);
    }

    #[test]
    fn restore_sessions_and_sets() {
        let (c, _root) = cfg();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let c2 = calls.clone();
        let runner = move |x: &[String]| {
            c2.lock().unwrap().push(x.to_vec());
            !x.iter().any(|a| a.contains("10.50.0.99"))
        };
        let mut ses = ortak::Sessions::new();
        ses.insert("aa:bb:cc:dd:ee:01".into(), sess("10.50.0.21", NOW + 7200.0)); // geri yüklenir
        ses.insert("aa:bb:cc:dd:ee:02".into(), sess("10.50.0.22", NOW + 3.0)); // süresi doldu
        ses.insert("aa:bb:cc:dd:ee:03".into(), sess("", NOW + 7200.0)); // IP'siz beklemede
        ses.insert("aa:bb:cc:dd:ee:04".into(), sess("10.50.0.99", NOW + 7200.0)); // nft hatası
        ortak::save_sessions(&c.main.state_root, &ses).unwrap();
        assert_eq!(yukle(&c, NOW, &runner), 1);
        let ses = ortak::load_sessions(&c.main.state_root);
        assert_eq!(ses.keys().cloned().collect::<Vec<_>>(), vec!["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:03"]);
        let calls = calls.lock().unwrap();
        assert!(calls.iter().any(|c| c[5] == "allow_mac" && c[6] == "{ aa:bb:cc:dd:ee:99 }"));
        assert!(calls.iter().any(|c| c[5] == "ban_mac" && c[6] == "{ aa:bb:cc:dd:ee:66 }"));
        assert!(calls.iter().any(|c| c[6] == "{ aa:bb:cc:dd:ee:01 . 10.50.0.21 timeout 7200s }"));
    }

    #[test]
    fn ctl_hiz_uygula_cikis_kodu() {
        let (c, root) = cfg();
        let mut ses = ortak::Sessions::new();
        ses.insert(M1.into(), sess("10.50.0.23", NOW + 3600.0));
        ortak::save_sessions(&c.main.state_root, &ses).unwrap();
        std::fs::write(root.join("leases"), "1 aa:bb:cc:dd:ee:99 10.50.0.2 ap 01\n").unwrap(); // izinli cihaz
        let mut s = crate::hiz::Sinirlar::new();
        for mac in [M1, "aa:bb:cc:dd:ee:99"] {
            s.insert(mac.into(), crate::hiz::Sinir { hiz: 2, ad: String::new(), zaman: String::new(), kim: "y".into() });
        }
        crate::hiz::kaydet(&c.main.state_root, &s).unwrap();
        let tc_ok = Arc::new(Mutex::new(false));
        let t2 = tc_ok.clone();
        let runner = move |x: &[String]| x[0] != "tc" || x[2] == "del" || *t2.lock().unwrap();
        let son = std::cell::RefCell::new(String::new());
        let nft_f = |t: &str| {
            *son.borrow_mut() = t.to_string();
            true
        };
        let yol = root.join("uygulanan");
        let kod = |k: ExitCode| format!("{k:?}");
        assert_eq!(kod(hiz_uygula(&c, "enp3s0", &runner, &nft_f, &yol)), kod(ExitCode::from(1)));
        *tc_ok.lock().unwrap() = true;
        assert_eq!(kod(hiz_uygula(&c, "enp3s0", &runner, &nft_f, &yol)), kod(ExitCode::SUCCESS));
        assert!(son.borrow().contains("ip daddr 10.50.0.23 ") && son.borrow().contains("ip daddr 10.50.0.2 "));
        assert_eq!(kod(hiz_uygula(&c, "enp3s0", &runner, &nft_f, &yol)), kod(ExitCode::SUCCESS)); // değişiklik yok
    }

    #[test]
    fn ap_gecici_adres_acilis_ve_komutlar() {
        let (c, root) = cfg();
        let y = crate::ap::Yollar { gecici: root.join("ap-gecici.json") };
        let denetim = || std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        let canli = "4: br-hotspot    inet 192.168.0.250/24 scope global br-hotspot:ap\\       valid_lft 100sec preferred_lft 100sec\n";
        let ip_var = |a: &[&str]| if a.len() == 7 && a[5] == "dev" { canli.to_string() } else { String::new() };
        let ip_yok = |_: &[&str]| String::new();
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let c2 = calls.clone();
        let runner = move |x: &[String]| {
            c2.lock().unwrap().push(x.to_vec());
            true
        };
        let kod = |k: ExitCode| format!("{k:?}");
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // konsoldan ekleme: --sure sınırı (60-1800)
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.0.250/24", "--sure", "30"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.0.250/24", "--sure", "1801"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "--sure", "120"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.0.250/24", "--sure", "120"]), &ip_yok, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(denetim().contains("eklendi adres=192.168.0.250/24 sure=120 kullanici=konsol"));
        assert!(calls.lock().unwrap().iter().any(|x| x.contains(&"130".to_string()) && x[0] == "ip")); // valid_lft = süre + 10
        let g: crate::ap::Gecici = serde_json::from_slice(&std::fs::read(&y.gecici).unwrap()).unwrap();
        // servis yeniden başladı (adres canlıda): açılış temizliği dokunmaz
        assert_eq!(ap_acilis(&c, &y, &ip_var, NOW), None);
        assert!(y.gecici.exists() && !denetim().contains("neden=acilis"));
        // eski zamanlayıcı (başka kimlik) silemez
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "sure", "ffffffff"]), &ip_var, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(y.gecici.exists());
        // zamanlayıcı kendi kimliğiyle: kaldırır, neden=sure
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "sure", &g.kimlik]), &ip_var, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(!y.gecici.exists() && denetim().contains("kaldirildi adres=192.168.0.250/24 neden=sure"));
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "fabrika"]), &ip_var, &runner, NOW)), kod(ExitCode::from(2)));
        // yeniden başlatma: dosya kaldı, adres yok → yukle temizler, neden=acilis
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.0.250/24"]), &ip_yok, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert_eq!(ap_acilis(&c, &y, &ip_yok, NOW).as_deref(), Some("192.168.0.250/24"));
        assert!(!y.gecici.exists() && denetim().contains("kaldirildi adres=192.168.0.250/24 neden=acilis"));
    }
}
