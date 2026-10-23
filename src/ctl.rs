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
    ortak::audit(&cfg.main.log_root, Row::new("AP_YAN_ODA", &ortak::now_iso(now)).set("ek", ek));
}

/// Açılışta (`yukle`): yeniden başlatmayla kapanan AP yan odasının durum dosyası temizlenir (`neden=acilis`).
fn ap_acilis(cfg: &Config, y: &crate::ap::Yollar, ip_oku: &crate::ap::IpOku, now: f64) -> Option<String> {
    let a = crate::ap::acilis_temizle(y, ip_oku)?;
    ap_denetim(cfg, now, format!("kapandi adres={a} neden=acilis"));
    Some(a)
}

/// `ctl ap-adres-kaldir <elle|sure> [KIMLIK]` (zamanlayıcı `sure KIMLIK` ile çağırır): yan odayı kapatır.
fn ap_adres_kaldir(cfg: &Config, y: &crate::ap::Yollar, args: &[String], ip_oku: &crate::ap::IpOku, runner: &Runner, now: f64) -> ExitCode {
    let (Some(neden @ ("elle" | "sure")), kimlik) = (args.get(1).map(String::as_str), args.get(2).map(String::as_str)) else {
        eprintln!("Kullanım: wificorrect ctl ap-adres-kaldir <elle|sure> [KIMLIK]");
        return ExitCode::from(2);
    };
    runner(&crate::ap::eski_adres_temizle(&cfg.main.iface)); // 0.4.11'den kalan zamanlayıcı (ana ad alanındaki adres)
    match crate::ap::kaldir(y, neden, kimlik, ip_oku, runner) {
        Ok(Some(k)) => {
            let hata = k.hata.map_or(String::new(), |h| format!(" hata={h}"));
            ap_denetim(cfg, now, format!("kapandi adres={} neden={neden}{hata}", k.adres));
            ap_arayuz_kapat(cfg, y, now);
            println!("yan oda {} kapandı{hata}", k.adres);
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("açık yan oda yok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// Yan oda kapanınca onun AP arayüzü erişimi de biter (`AP_ARAYUZ kapandi … neden=yan_oda`; kim + rol hedef dosyasından).
fn ap_arayuz_kapat(cfg: &Config, y: &crate::ap::Yollar, now: f64) {
    if let Some(h) = crate::ap_arayuz::kapat(y, true) {
        crate::ap_arayuz::denetim(&cfg.main.log_root, now, &h, &crate::ap_arayuz::kapandi_ek(&h, "yan_oda"));
    }
}

const AP_BUL_KULLANIM: &str = "Kullanım: wificorrect ctl ap-bul KIM <sahip|hizmet|konsol> IP [HEDEF]  (panelin arama işi; HEDEF yoksa otomatik)";

/// `ctl ap-bul KIM ROL IP [HEDEF]` argümanları: (kim, rol, ip, hedef). Kim yalnızca harf/rakam/._-, IP geçerli ya da "-".
fn ap_bul_argumanlari(args: &[String]) -> Option<(&str, &str, &str, Option<std::net::Ipv4Addr>)> {
    let kim = args.get(1).filter(|k| !k.is_empty() && k.len() <= 64 && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)))?;
    let rol = args.get(2).filter(|r| matches!(r.as_str(), "sahip" | "hizmet" | "konsol"))?;
    let ip = args.get(3).filter(|i| *i == "-" || i.parse::<std::net::IpAddr>().is_ok())?;
    let hedef = match args.get(4) {
        None => None,
        Some(h) => Some(h.parse().ok()?),
    };
    (args.len() <= 5).then_some((kim.as_str(), rol.as_str(), ip.as_str(), hedef))
}

/// `ctl ap-bul`: panelin "Bağlan" / "Otomatik bul" arka plan işi (`systemd-run --unit wfc-ap-bul`, src/ap_bul.rs).
/// Denetim satırları panelinki gibi kim + rol + IP ile.
fn ap_bul(cfg: &Config, args: &[String]) -> ExitCode {
    use std::net::Ipv4Addr;
    use std::time::Duration;
    let Some((kim, rol, ip, hedef)) = ap_bul_argumanlari(args) else {
        eprintln!("{AP_BUL_KULLANIM}");
        return ExitCode::from(2);
    };
    let y = crate::ap::Yollar::sistem();
    let ip_oku = |c: &[&str]| ortak::capture(c);
    let runner = |c: &[String]| ortak::run(c);
    let log_root = &cfg.main.log_root;
    let denetim = |olay: &str, ek: String| {
        ortak::audit(log_root, Row::new(olay, &ortak::now_iso(ortak::wall())).set("ek", format!("kullanici={kim} rol={rol} ip={ip} {ek}")));
    };
    let arp = |istek: &[(Ipv4Addr, Ipv4Addr)]| crate::yan_oda::arp_tara(&cfg.main.iface, istek, Duration::from_millis(1200));
    let yan_oda = || crate::ap::durum(&y, &ip_oku).filter(|(_, canli)| *canli).map(|(g, _)| g);
    let ac = |c: &str| {
        // durum dosyası kalmış ama aracısı ölmüş yan oda "Zaten bir yan oda açık" demesin (final review M6): önce kapanır
        if let Ok(Some(k)) = crate::ap::kaldir(&y, "araci_yok", None, &ip_oku, &runner) {
            denetim("AP_YAN_ODA", format!("kapandi adres={} neden=araci_yok", k.adres));
            ap_arayuz_kapat(cfg, &y, ortak::wall());
        }
        crate::ap::ekle(cfg, &y, c, crate::ap::SURE_SN, &ip_oku, &runner, ortak::wall())
    };
    let kapat = |_neden: &str| -> Result<Option<String>, String> {
        let k = crate::ap::kaldir(&y, "elle", None, &ip_oku, &runner)?;
        ap_arayuz_kapat(cfg, &y, ortak::wall());
        Ok(k.map(|k| k.adres))
    };
    // web arayüzü: panelin "AP arayüzüne git"iyle aynı seçim (443'te TLS el sıkışması olursa HTTPS, yoksa 80)
    let web = |adres: Ipv4Addr, yan: bool| crate::ap_arayuz::web_portu(adres, yan, false);
    let kaydet = |d: &crate::ap_bul::Durum| {
        if let Err(e) = crate::ap_bul::kaydet(&y, d) {
            eprintln!("{e}");
        }
    };
    let a = crate::ap_bul::Araclar { arp: &arp, yan_oda: &yan_oda, ac: &ac, kapat: &kapat, web: &web, denetim: &denetim, kaydet: &kaydet };
    let d = crate::ap_bul::calistir(cfg, hedef, kim, rol, &a, &ortak::wall);
    if d.mesaj.is_empty() {
        println!("{}: {}:{} ({})", d.durum, d.adres, d.port, d.yol);
    } else {
        println!("{}: {}", d.durum, d.mesaj);
    }
    ExitCode::SUCCESS
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
            ap_denetim(cfg, now, format!("acildi adres={} sure={sure} kullanici=konsol", g.adres));
            println!("yan oda {} açıldı ({sure} sn); aracı 127.0.0.1:{}", g.adres, crate::yan_oda::ARACI_PORT);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// `ctl ap-araci ADRES`: yalnızca `ap::ekle`'nin geçici systemd birimi çalıştırır (yan oda + SOCKS5 aracı).
fn ap_araci(cfg: &Config, args: &[String]) -> ExitCode {
    let Some(adres) = args.get(1) else {
        eprintln!("Kullanım: wificorrect ctl ap-araci a.b.c.d/önek  (yalnızca wfc-ap-araci birimi)");
        return ExitCode::from(2);
    };
    let sonuc = crate::ap::gecici_dogrula(adres, cfg).and_then(|c| crate::yan_oda::araci_calistir(&cfg.main.iface, c));
    if let Err(e) = sonuc {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

/// `ctl ap-dhcp-denetle`: misafir köprüsünde (kısa ömürlü yan odadan) DHCP teklifleri. Çıktı panel için:
/// `TEKLIF <sunucu> <mac> <adres>` satırları, sonda `TAMAM <n>`.
fn ap_dhcp_denetle(cfg: &Config) -> ExitCode {
    match crate::yan_oda::dhcp_denetle(&cfg.main.iface) {
        Ok(v) => {
            for t in &v {
                println!("TEKLIF {} {} {}", t.sunucu, t.mac, t.adres);
            }
            println!("TAMAM {}", v.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

/// `ctl pppoe-peers AG.TOML ARAYUZ` (ağ ad alanındaki sahte operatör denemesi, `scripts/pppoe-deneme.sh`): AG.TOML'daki PPPoE
/// ayarından pppd bağlantı dosyasının metnini üretir; yalnızca Ethernet arayüzü ARAYUZ'dür (VLAN soneki eklenmez). Dosya yazmaz.
/// Üretilen dosya kullanılabilir olmalı: ayar açıkmış gibi doğrulanır (boş / satır sokan kullanıcı adı ve parola reddedilir).
fn pppoe_peers_metni(ag_toml: &std::path::Path, arayuz: &str) -> Result<String, String> {
    if arayuz.is_empty() || arayuz.len() > 15 || !arayuz.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
        return Err("Arayüz adı geçersiz (en çok 15 karakter: harf, rakam, nokta, alt çizgi, tire).".into());
    }
    let metin = std::fs::read_to_string(ag_toml).map_err(|e| format!("{} okunamadı: {e}", ag_toml.display()))?;
    // ayrıştırma hatasının metni hatalı satırı (parola olabilir) içerir: gösterilmez
    let ag: crate::ag::Ag = toml::from_str(&metin).map_err(|_| format!("{} ayrıştırılamadı", ag_toml.display()))?;
    crate::ag::pppoe_dogrula(&crate::ag::Pppoe { acik: true, ..ag.pppoe.clone() })?;
    Ok(crate::pppoe::peers_text_nic(&ag, arayuz))
}

/// `ctl pppoe-servis DOSYA` (giriş sayfasındaki Servis bölümü `systemd-run` ile başlatır): DOSYA, ag.toml'daki `[pppoe]` ile aynı
/// alanlara sahip bir `[pppoe]` tablosu içeren TOML'dur (boş parola = eskisi kalır). Parola içerdiği için okunur okunmaz, sonuç
/// ne olursa olsun silinir. Uygulama `ag::pppoe_uygula` (geri alma zamanlayıcısı yok). Denetime kullanıcı adı ve parola yazılmaz.
fn pppoe_servis(cfg: &Config, y: &crate::ag::Yollar, dosya: &std::path::Path, runner: &Runner, now: f64) -> ExitCode {
    #[derive(serde::Deserialize)]
    struct Servis {
        pppoe: crate::ag::Pppoe, // tablo zorunlu: eksikse PPPoE kendiliğinden kapanmasın
    }
    let okunan = std::fs::read_to_string(dosya);
    let _ = std::fs::remove_file(dosya);
    let yeni = match okunan {
        // ayrıştırma hatasının metni hatalı satırı (parola olabilir) içerir: gösterilmez
        Ok(t) => match toml::from_str::<Servis>(&t) {
            Ok(s) => s.pppoe,
            Err(_) => {
                eprintln!("PPPoE servis dosyası geçersiz: [pppoe] tablosu okunamadı.");
                return ExitCode::from(1);
            }
        },
        Err(e) => {
            eprintln!("{} okunamadı: {e}", dosya.display());
            return ExitCode::from(1);
        }
    };
    let sonuc = crate::ag::pppoe_uygula(cfg, y, &yeni, runner);
    let hata = match &sonuc {
        Ok(errs) if errs.is_empty() => String::new(),
        Ok(errs) => format!(" hata={}", errs.join(",")),
        Err(e) => format!(" hata={e}"),
    };
    let ek = format!("pppoe={} vlan={}{hata}", if yeni.acik { "acik" } else { "kapali" }, yeni.vlan);
    ortak::audit(&cfg.main.log_root, Row::new("PPPOE_SERVIS_UYGULANDI", &ortak::now_iso(now)).set("ek", ek));
    match sonuc {
        Ok(errs) if errs.is_empty() => {
            println!("PPPoE ayarı uygulandı");
            ExitCode::SUCCESS
        }
        Ok(errs) => {
            eprintln!("{}", errs.join("; "));
            ExitCode::from(1)
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
                println!("AP yan odası {a} artık yok; durum temizlendi");
            }
            ExitCode::SUCCESS
        }
        // Panel → Ayarlar → AP kurulumu eşdeğeri (konsol), kapatma zamanlayıcısı, aracı birimi, DHCP denetimi
        Some("ap-adres-ekle") => ap_adres_ekle(&cfg, &crate::ap::Yollar::sistem(), args, &|c: &[&str]| ortak::capture(c), &runner, now),
        Some("ap-adres-kaldir") => ap_adres_kaldir(&cfg, &crate::ap::Yollar::sistem(), args, &|c: &[&str]| ortak::capture(c), &runner, now),
        Some("ap-araci") => ap_araci(&cfg, args),
        Some("ap-dhcp-denetle") => ap_dhcp_denetle(&cfg),
        // Panel → AP kurulumu → Takılan cihaz: "Bağlan" / "Otomatik bul" (systemd-run --unit wfc-ap-bul)
        Some("ap-bul") => ap_bul(&cfg, args),
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
            match crate::merkez::eslesme(&mut m, &cfg.main.site_name, &cfg.main.unvan, &cfg.main.adres, crate::ayar::isletme_turu_secili(&cfg).unwrap_or(""), false, &crate::merkez::curl, now) {
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
        // Kolay bağlama (spec 2026-10-16): kaydedici bağlı değilken 2 dk'da bir başlatır (systemd-run --unit wfc-bekliyor)
        Some("bekliyor") => {
            let anahtarlar = || -> Result<(String, String), String> {
                let wg = crate::uzak::ensure_key(std::path::Path::new(crate::uzak::KEY)).and_then(|k| crate::uzak::public_key(&k))?;
                Ok((wg, crate::uzak::yedek_anahtari(std::path::Path::new(crate::uzak::YEDEK_KEY))?))
            };
            let hs = crate::hesap::Hesaplar::new(crate::hesap::PATH);
            match crate::bekleme::calistir(&cfg, &crate::bekleme::Yollar::sistem(), &hs, &anahtarlar, &crate::merkez::curl, &runner, now) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("bekleme bildirimi: {e}");
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
        // PPPoE (spec 2026-10-23): wificorrect-pppoe.service pppd'den önce ve sonra çalıştırır
        Some("pppoe-hazirla") => match crate::pppoe::hazirla(&crate::ag::load(&crate::ag::Yollar::sistem().ag), &runner) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some("pppoe-temizle") => {
            let ip_link = ortak::capture(&["ip", "-o", "link", "show", "type", "vlan"]);
            crate::pppoe::temizle(&crate::ag::load(&crate::ag::Yollar::sistem().ag), &ip_link, &runner);
            ExitCode::SUCCESS
        }
        // Sahte operatör denemesi (scripts/pppoe-deneme.sh): ag.toml'daki PPPoE ayarından pppd bağlantı dosyası metnini stdout'a yazar
        Some("pppoe-peers") if args.len() == 3 => match pppoe_peers_metni(std::path::Path::new(&args[1]), &args[2]) {
            Ok(t) => {
                print!("{t}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Some("pppoe-peers") => {
            eprintln!("Kullanım: wificorrect ctl pppoe-peers <ag.toml> <arayuz>");
            ExitCode::from(2)
        }
        // Giriş sayfası → Servis (systemd-run ile): DOSYA'daki [pppoe] ayarını uygular, dosyayı siler
        Some("pppoe-servis") if args.len() == 2 => pppoe_servis(&cfg, &crate::ag::Yollar::sistem(), std::path::Path::new(&args[1]), &runner, now),
        Some("pppoe-servis") => {
            eprintln!("Kullanım: wificorrect ctl pppoe-servis <hazir.toml>");
            ExitCode::from(2)
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
                 gun-kapat [YYYY-AA-GG] [--zorla]\n  dogrula [--son N]\n  temizle [--kuru]\n  yedekle\n  ara ...  (ayrıntı: ctl ara)\n  disa-aktar --baslangic G --bitis G --cikti DOSYA\n  ag-uygula | ag-gecis DOSYA | ag-onayla | ag-geri-al | ag-ilk\n  pppoe-hazirla | pppoe-temizle | pppoe-peers AG.TOML ARAYUZ | pppoe-servis DOSYA\n  ayar-temizle\n  filtre-uygula\n  liste-uygula | liste-guncelle\n  hiz-uygula\n  admin-parola\n  merkez-eslesme\n  saglik\n  guncelle\n  ap-adres-ekle a.b.c.d/önek [--sure SN]  (AP yan odası)\n  ap-adres-kaldir <elle|sure> [KIMLIK]\n  ap-dhcp-denetle\n  ap-bul KIM ROL IP [HEDEF]"
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

    // ---------------------------------------------------------------- PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md)
    fn pppoe_yollar(root: &std::path::Path) -> crate::ag::Yollar {
        crate::ag::Yollar {
            ag: root.join("ag.toml"),
            interfaces: root.join("interfaces"),
            nft: root.join("arayuzler.nft"),
            hostapd: root.join("hostapd.conf"),
            issue: root.join("issue"),
            peers: root.join("peers"),
            durum: root.join("durum"),
            hiz_uygulanan: root.join("hiz_uygulanan"),
        }
    }

    #[test]
    fn pppoe_peers_komutu() {
        let (_, root) = cfg();
        std::fs::create_dir_all(&root).unwrap();
        let f = root.join("ag.toml");
        std::fs::write(&f, "wan = \"enp1s0\"\n[pppoe]\nacik = true\nkullanici = \"deneme@isp\"\nparola = \"gizli1\"\nvlan = 35\nmtu = 1400\n").unwrap();
        let once = std::fs::read_dir(&root).unwrap().count();
        let t = pppoe_peers_metni(&f, "vethcpe").unwrap();
        // yalnızca arayüz değişir: VLAN soneki eklenmez, internet portunun adı yazılmaz; gerisi ayardan
        assert!(t.lines().any(|l| l == "nic-vethcpe") && t.contains("user \"deneme@isp\"\n") && t.contains("password \"gizli1\"\n") && t.contains("mtu 1400\n"), "{t}");
        assert!(!t.contains("enp1s0") && !t.contains("vethcpe.35"), "{t}");
        assert!(pppoe_peers_metni(&f, "vethcpe.35").unwrap().lines().any(|l| l == "nic-vethcpe.35")); // betik kendi VLAN arayüzünü verebilir
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), once); // hiçbir dosya yazılmaz (yalnızca stdout)
        // geçersiz arayüz adı (peers dosyasına satır sokulamaz) ve eksik dosya
        let uzun = "a".repeat(16);
        for kotu in ["", "veth cpe", "veth\ncpe", uzun.as_str(), "veth;x", "veth/cpe", "veth\"x"] {
            assert!(pppoe_peers_metni(&f, kotu).is_err(), "{kotu:?}");
        }
        assert!(pppoe_peers_metni(&root.join("yok.toml"), "vethcpe").is_err());
        // bozuk ayar: hata, hata metni parolayı göstermez
        std::fs::write(&f, "[pppoe]\nacik = true\nkullanici = \"k\"\nparola = \"gizli1\nvlan =\n").unwrap();
        let e = pppoe_peers_metni(&f, "vethcpe").unwrap_err();
        assert!(!e.contains("gizli1"), "{e}");
        // kullanılamaz peers üretilmez: boş parola, satır sokan değer
        std::fs::write(&f, "[pppoe]\nacik = true\nkullanici = \"k\"\n").unwrap();
        assert!(pppoe_peers_metni(&f, "vethcpe").is_err());
        std::fs::write(&f, "[pppoe]\nkullanici = \"k\\nconnect /bin/sh\"\nparola = \"p\"\n").unwrap();
        assert!(pppoe_peers_metni(&f, "vethcpe").is_err());
        // acik = false olsa da bilgiler doluysa peers üretilir (betik yalnızca kullanıcı adı, parola, kimlik doğrulama verebilir)
        std::fs::write(&f, "[pppoe]\nkullanici = \"k\"\nparola = \"p\"\n").unwrap();
        assert!(pppoe_peers_metni(&f, "vethcpe").unwrap().contains("user \"k\"\n"));
    }

    #[test]
    fn pppoe_komutlari_arguman_sayisi() {
        // run(): yanlış argüman sayısı kullanım hatasıdır (2) ve hiçbir şeye dokunmaz; doğru sayıda argüman komuta gider
        let (c, root) = cfg();
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let kod = |k: ExitCode| format!("{k:?}");
        let kullanim = kod(ExitCode::from(2));
        for v in [&["pppoe-peers"][..], &["pppoe-peers", "ag.toml"], &["pppoe-peers", "a", "b", "c"], &["pppoe-servis"], &["pppoe-servis", "a", "b"]] {
            assert_eq!(kod(run(c.clone(), &a(v))), kullanim, "{v:?}");
        }
        let yok = root.join("yok.toml").to_string_lossy().to_string();
        assert_eq!(kod(run(c.clone(), &a(&["pppoe-peers", &yok, "vethcpe"]))), kod(ExitCode::from(1))); // komuta gitti: dosya yok
        assert_eq!(kod(run(c.clone(), &a(&["pppoe-servis", &yok]))), kod(ExitCode::from(1)));
    }

    #[test]
    fn pppoe_servis_komutu() {
        let (c, root) = cfg();
        let y = pppoe_yollar(&root);
        let eski = crate::ag::Ag { pppoe: crate::ag::Pppoe { acik: true, kullanici: "eski@isp".into(), parola: "eski-parola".into(), vlan: 35, ..Default::default() }, ..Default::default() };
        crate::ag::save(&y.ag, &eski).unwrap();
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        let runner = move |x: &[String]| {
            c2.lock().unwrap().push(x.join(" "));
            true
        };
        let kod = |k: ExitCode| format!("{k:?}");
        let denetim = || std::fs::read_to_string(ortak::day_file(&c.main.log_root, ortak::day_of(&ortak::now_iso(NOW)), "denetim.csv")).unwrap_or_default();
        let dosya = root.join("servis.toml");

        // boş parola = eskisi kalır; hazırlık dosyası (parola içerir) okununca silinir; kullanıcı adı / parola denetime yazılmaz
        std::fs::write(&dosya, "[pppoe]\nacik = true\nkullanici = \"yeni@isp\"\nparola = \"\"\nvlan = 0\n").unwrap();
        assert_eq!(kod(pppoe_servis(&c, &y, &dosya, &runner, NOW)), kod(ExitCode::SUCCESS));
        let k = crate::ag::load(&y.ag);
        assert_eq!((k.pppoe.acik, k.pppoe.kullanici.as_str(), k.pppoe.parola.as_str(), k.pppoe.vlan), (true, "yeni@isp", "eski-parola", 0));
        assert!(!dosya.exists());
        let calls_simdi = calls.lock().unwrap().clone();
        assert!(calls_simdi.iter().any(|x| x == "systemctl restart wificorrect-pppoe.service") && !calls_simdi.iter().any(|x| x.contains("systemd-run")), "{calls_simdi:?}");
        assert_eq!(std::fs::read_to_string(&y.peers).unwrap(), crate::pppoe::peers_text(&k));
        let d = denetim();
        let satir = d.lines().find(|l| l.contains("PPPOE_SERVIS_UYGULANDI")).unwrap_or_else(|| panic!("{d}"));
        assert!(satir.contains("pppoe=acik vlan=0") && !satir.contains("hata="), "{satir}");
        assert!(!d.contains("yeni@isp") && !d.contains("eski@isp") && !d.contains("parola"), "{d}");

        // geçersiz bilgi (satır sokan kullanıcı adı): çıkış 1, ayar ve bağlantı dosyası aynı, dosya yine silinir, denetimde hata var ama bilgi yok
        let ag_once = std::fs::read(&y.ag).unwrap();
        let peers_once = std::fs::read(&y.peers).unwrap();
        calls.lock().unwrap().clear();
        std::fs::write(&dosya, "[pppoe]\nacik = true\nkullanici = \"kotu\\nconnect /bin/sh\"\nparola = \"sirli-parola\"\nvlan = 7\n").unwrap();
        assert_eq!(kod(pppoe_servis(&c, &y, &dosya, &runner, NOW + 1.0)), kod(ExitCode::from(1)));
        assert!(!dosya.exists());
        assert_eq!((std::fs::read(&y.ag).unwrap(), std::fs::read(&y.peers).unwrap()), (ag_once.clone(), peers_once));
        assert!(calls.lock().unwrap().is_empty());
        let d = denetim();
        let hatali = d.lines().filter(|l| l.contains("PPPOE_SERVIS_UYGULANDI")).nth(1).unwrap_or_else(|| panic!("{d}"));
        assert!(hatali.contains("pppoe=acik vlan=7") && hatali.contains("hata="), "{hatali}");
        assert!(!d.contains("sirli-parola") && !d.contains("connect"), "{d}");

        // [pppoe] tablosu yok ya da dosya bozuk: çıkış 1 (PPPoE kendiliğinden kapanmaz), ayar aynı, dosya silinir, parola hiçbir yere sızmaz
        for icerik in ["acik = false\n", "[pppoe]\nacik = true\nparola = \"sirli-parola\nvlan =\n", "[pppoe]\nvlan = \"sirli-parola\"\n"] {
            std::fs::write(&dosya, icerik).unwrap();
            assert_eq!(kod(pppoe_servis(&c, &y, &dosya, &runner, NOW + 2.0)), kod(ExitCode::from(1)), "{icerik:?}");
            assert!(!dosya.exists());
            assert_eq!(std::fs::read(&y.ag).unwrap(), ag_once);
        }
        assert!(!denetim().contains("sirli-parola"));
        assert!(calls.lock().unwrap().is_empty());
        // dosya yok: çıkış 1
        assert_eq!(kod(pppoe_servis(&c, &y, &root.join("yok.toml"), &runner, NOW + 3.0)), kod(ExitCode::from(1)));

        // kapatma: denetimde pppoe=kapali; bağlantı dosyası gider, birim durur
        std::fs::write(&dosya, "[pppoe]\nacik = false\n").unwrap();
        assert_eq!(kod(pppoe_servis(&c, &y, &dosya, &runner, NOW + 4.0)), kod(ExitCode::SUCCESS));
        assert!(!y.peers.exists() && calls.lock().unwrap().iter().any(|x| x == "systemctl disable --now wificorrect-pppoe.service"));
        assert!(denetim().lines().any(|l| l.contains("PPPOE_SERVIS_UYGULANDI") && l.contains("pppoe=kapali vlan=0")));
        // birim başlamazsa: çıkış 1, hata denetimde
        std::fs::write(&dosya, "[pppoe]\nacik = true\nkullanici = \"k\"\nparola = \"p\"\n").unwrap();
        let hata_runner = |x: &[String]| !x.join(" ").starts_with("systemctl restart");
        assert_eq!(kod(pppoe_servis(&c, &y, &dosya, &hata_runner, NOW + 5.0)), kod(ExitCode::from(1)));
        assert!(denetim().contains("hata=PPPoE başlatılamadı"));
    }

    #[test]
    fn ap_yan_oda_acilis_ve_komutlar() {
        let (c, root) = cfg();
        let y = crate::ap::Yollar::kok(&root);
        let denetim = || std::fs::read_to_string(root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        // sahte `systemctl is-active wfc-ap-araci.service`
        let ip_var = |a: &[&str]| if a.first() == Some(&"systemctl") { "active\n".to_string() } else { String::new() };
        let ip_yok = |a: &[&str]| if a.first() == Some(&"systemctl") { "inactive\n".to_string() } else { String::new() };
        let calls = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let c2 = calls.clone();
        let runner = move |x: &[String]| {
            c2.lock().unwrap().push(x.to_vec());
            true
        };
        let kod = |k: ExitCode| format!("{k:?}");
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // konsoldan açma: --sure sınırı (60-1800)
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.1.250/24", "--sure", "30"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.1.250/24", "--sure", "1801"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "--sure", "120"]), &ip_yok, &runner, NOW)), kod(ExitCode::from(2)));
        assert!(calls.lock().unwrap().is_empty());
        // modemin ağıyla aynı alt ağ (yan oda ana ad alanına dokunmaz)
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.1.250/24", "--sure", "120"]), &ip_yok, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(denetim().contains("AP_YAN_ODA") && denetim().contains("acildi adres=192.168.1.250/24 sure=120 kullanici=konsol"));
        assert!(calls.lock().unwrap().iter().any(|x| x.contains(&"--property=RuntimeMaxSec=130".to_string()))); // süre + 10
        assert!(!calls.lock().unwrap().iter().any(|x| x[0] == "ip"));
        let g: crate::ap::Gecici = serde_json::from_slice(&std::fs::read(&y.gecici).unwrap()).unwrap();
        // güvenlik duvarı servisi yeniden başladı (aracı çalışıyor): açılış temizliği dokunmaz
        assert_eq!(ap_acilis(&c, &y, &ip_var, NOW), None);
        assert!(y.gecici.exists() && !denetim().contains("neden=acilis"));
        // eski zamanlayıcı (başka kimlik) kapatamaz
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "sure", "ffffffff"]), &ip_var, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(y.gecici.exists());
        // zamanlayıcı kendi kimliğiyle: kapatır, neden=sure; yan odanın AP arayüzü erişimi de biter
        let hedef = crate::ap_arayuz::Hedef { adres: "192.168.1.1".into(), port: 80, sema: "http".into(), yol: "yan_oda".into(), bitis: NOW + 600.0,
                                              kim: "4511643".into(), rol: "sahip".into(), yan_oda: Some(g.kimlik.clone()), mac: None };
        crate::ap_arayuz::yaz(&y, &hedef).unwrap();
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "sure", &g.kimlik]), &ip_var, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert!(!y.gecici.exists() && denetim().contains("kapandi adres=192.168.1.250/24 neden=sure"));
        assert!(!y.hedef.exists());
        assert!(denetim().lines().any(|l| l.contains("AP_ARAYUZ") && l.contains("kullanici=4511643 rol=sahip") && l.contains("kapandi hedef=192.168.1.1:80 yol=yan_oda neden=yan_oda")), "{}", denetim());
        let eski = a(&["ip", "addr", "flush", "dev", "br-hotspot", "label", "br-hotspot:ap"]);
        assert!(calls.lock().unwrap().iter().any(|x| *x == eski)); // 0.4.11'in köprüdeki adresi de kalkar
        assert_eq!(kod(ap_adres_kaldir(&c, &y, &a(&["ap-adres-kaldir", "fabrika"]), &ip_var, &runner, NOW)), kod(ExitCode::from(2)));
        // yeniden başlatma: dosya kaldı, aracı yok → yukle temizler, neden=acilis
        assert_eq!(kod(ap_adres_ekle(&c, &y, &a(&["ap-adres-ekle", "192.168.1.250/24"]), &ip_yok, &runner, NOW)), kod(ExitCode::SUCCESS));
        assert_eq!(ap_acilis(&c, &y, &ip_yok, NOW).as_deref(), Some("192.168.1.250/24"));
        assert!(!y.gecici.exists() && denetim().contains("kapandi adres=192.168.1.250/24 neden=acilis"));
        // aracı birimi geçersiz adresi reddeder (yan oda açılmaz)
        assert_eq!(kod(ap_araci(&c, &a(&["ap-araci", "10.50.0.9/24"]))), kod(ExitCode::from(1)));
        assert_eq!(kod(ap_araci(&c, &a(&["ap-araci"]))), kod(ExitCode::from(2)));
        // arama işi: argümanlar sıkı (ağa dokunmadan reddedilir)
        let arg = |v: &[&str]| ap_bul_argumanlari(&a(v)).map(|(k, r, i, h)| (k.to_string(), r.to_string(), i.to_string(), h));
        assert_eq!(arg(&["ap-bul", "4511643", "sahip", "192.168.1.50", "192.168.1.1"]),
                   Some(("4511643".into(), "sahip".into(), "192.168.1.50".into(), Some("192.168.1.1".parse().unwrap()))));
        assert_eq!(arg(&["ap-bul", "konsol", "konsol", "-"]).map(|x| x.3), Some(None));
        assert!(arg(&["ap-bul", "2001:db8::1", "hizmet", "2001:db8::1"]).is_none(), "kimde ':' yok");
        assert!(arg(&["ap-bul", "4511643", "hizmet", "2001:db8::1"]).is_some());
        for kotu in [&["ap-bul"][..], &["ap-bul", "4511643", "kral", "1.2.3.4"], &["ap-bul", "a b", "sahip", "1.2.3.4"],
                     &["ap-bul", "4511643", "sahip", "x"], &["ap-bul", "4511643", "sahip", "1.2.3.4", "tplinkap.net"],
                     &["ap-bul", "4511643", "sahip", "1.2.3.4", "1.2.3.5", "fazla"]] {
            assert!(ap_bul_argumanlari(&a(kotu)).is_none(), "{kotu:?}");
            assert_eq!(kod(ap_bul(&c, &a(kotu))), kod(ExitCode::from(2)));
        }
    }
}
