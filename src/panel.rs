//! Yönetim paneli — 8a çekirdek (eski panel.py; RUST_YENIDEN_YAZIM.md A10, A14).
//! https://<cihaz>:8443, yalnızca dükkân ağından (güvenlik duvarı + burada müşteri ağı reddi).
//! İki rol (2026-10-03 kullanıcı kararı): işletme sahibi kayıtlar dahil işletmeyle ilgili her şeyi görür ve yönetir;
//! admin (hizmet sağlayıcı) ondan "Admin ayarları" sayfasıyla ayrılır: SMS sağlayıcıları ve sınırları, deneme modu,
//! kayıt saklama ve uzak yedek (sunucu), fabrika ayarları. İşletme sahibi sağlayıcı ve sunucu adlarını hiçbir yerde görmez.
//! 2026-10-09: ayrı admin kullanıcısı yok; herkes müşteri olarak girer, "Admin ayarları" (ve Panel hareketleri) merkezden
//! gelen admin parolasıyla oturumda 30 dk açılır (admin kilidi; açıkken etkin rol Hizmet).
//! 8b kayıtlar / kullanıcılar / resmi talep: panel/kayit_sayfalari.rs. 8c portlar ve Wi-Fi: panel/portlar.rs.

use crate::ayar::{Config, Device};
use crate::hesap::{self, Hesaplar, LoginGuard, Oturum, Oturumlar, Rol};
use crate::ortak::{self, Row, Runner};
use crate::portal::{html_escape as h, parse_query, substitute, Form};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::process::ExitCode;
use std::sync::Arc;

mod filtre_sayfasi;
mod gerekce;
mod hareketler;
mod kayit_sayfalari;
mod metinler;
mod portlar;

pub type Clock = dyn Fn() -> f64 + Send + Sync;
const TPL: &str = include_str!("sablon/panel.html");
const CERT: &str = "/etc/wificorrect/panel.crt";
const KEY: &str = "/etc/wificorrect/panel.key";

#[derive(Default)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub query: Form,
    pub form: Form,
    pub ip: String,
    pub token: Option<String>,
    /// X-WFC-Kullanici: merkezin doğruladığı müşteri numarası (yalnızca merkezin tünel adresinden gelirse geçerli)
    pub merkez_kullanici: Option<String>,
    /// X-Forwarded-For'un son öğesi: merkez üzerinden gelen gerçek istemci IP'si
    pub ileten_ip: Option<String>,
}

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
    /// İndirme: gövde yerine dosya akıtılır
    pub file: Option<std::fs::File>,
}

fn redirect(loc: &str, msg: Option<(&str, bool)>) -> Resp {
    // eski bölüm yolu (sorgusuz) → grup sayfasının o bölümü; sorgulu adres (arama, ayrıntı) olduğu gibi kalır
    let (loc, frag) = match BOLUMLER.iter().find(|(_, y, ..)| *y == loc) {
        Some((g, _, id, ..)) => (*g, format!("#{id}")),
        None => (loc, String::new()),
    };
    let loc = match msg {
        Some((m, err)) => format!("{loc}?m={}{}{frag}", pct(m), if err { "&e=1" } else { "" }),
        None => format!("{loc}{frag}"),
    };
    Resp { status: 303, body: String::new(), headers: vec![("Location".into(), loc)], file: None }
}

/// Merkezin tünel adresi: cihazın tünel adresinin (a.b.c.d) a.b.c.1'i; tünel adresi yoksa None.
fn merkez_adresi(cfg: &Config) -> Option<String> {
    let [a, b, c, _] = cfg.uzak.adres.trim().parse::<std::net::Ipv4Addr>().ok()?.octets();
    Some(format!("{a}.{b}.{c}.1"))
}

fn text(status: u16, msg: &str) -> Resp {
    Resp { status, body: msg.into(), headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())], file: None }
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn csrf_input(o: &Oturum) -> String {
    format!("<input type=\"hidden\" name=\"csrf\" value=\"{}\">", h(&o.csrf))
}

fn post_button(o: &Oturum, action: &str, label: &str, fields: &[(&str, &str)], cls: &str) -> String {
    let hidden: String = fields.iter().map(|(k, v)| format!("<input type=\"hidden\" name=\"{}\" value=\"{}\">", h(k), h(v))).collect();
    format!("<form class=\"ic\" method=\"post\" action=\"{}\">{}{hidden}<button class=\"{}\">{}</button></form>", h(action), csrf_input(o), h(cls), h(label))
}

fn table(headers: &[&str], rows: &[Vec<String>], empty: &str) -> String {
    if rows.is_empty() {
        return format!("<p class=\"not\">{}</p>", h(empty));
    }
    let head: String = headers.iter().map(|x| format!("<th>{}</th>", h(x))).collect();
    // hücreler zaten güvenli HTML (çağıran kaçışlar)
    let body: String = rows.iter().map(|r| format!("<tr>{}</tr>", r.iter().map(|c| format!("<td>{c}</td>")).collect::<String>())).collect();
    format!("<div class=\"tablo\"><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>")
}

fn facts(rows: &[(&str, String)]) -> String {
    let items: String = rows.iter().map(|(k, v)| format!("<div><dt>{}</dt><dd>{v}</dd></div>", h(k))).collect();
    format!("<dl class=\"bilgi\">{items}</dl>")
}

fn service_state(name: &str) -> String {
    let st = ortak::capture(&["systemctl", "is-active", name]).trim().to_string();
    let ok = st == "active";
    format!("<span class=\"durum{}\">{}</span>", if ok { "" } else { " kotu" }, if ok { "çalışıyor".into() } else { h(&format!("DURMUŞ ({st})")) })
}

fn disk_percent(path: &str) -> Option<u64> {
    let p = std::ffi::CString::new(path).ok()?;
    // SAFETY: statvfs yalnızca verilen yapıya yazar.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(p.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
        return None;
    }
    Some(100 - (st.f_bavail as u64 * 100 / st.f_blocks as u64))
}

// ---------------------------------------------------------------- ayar alanları
#[derive(Clone, Copy)]
enum Tur {
    Metin(fn(&str) -> bool),
    Sayi(u64, u64),
    Evet,
    Gizli(fn(&str) -> bool),
    Secim(&'static [(&'static str, &'static str)]),
}

struct Alan {
    key: &'static str,
    label: &'static str,
    tur: Tur,
    grup: &'static str,
    /// true: "Admin ayarları" sayfasında, yalnızca admin görür ve değiştirir (SMS sağlayıcısı, deneme modu)
    admin: bool,
}

fn chars_ok(s: &str, max: usize, allowed: fn(char) -> bool) -> bool {
    s.chars().count() <= max && s.chars().all(allowed)
}

fn prefixed_hex(s: &str, prefix: &str) -> bool {
    s.is_empty() || (s.len() == prefix.len() + 32 && s.starts_with(prefix) && s[prefix.len()..].bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Vergi levhası unvanı: 2-200 karakter; harf, rakam, boşluk ve . , - ' & ( ) / "
fn unvan_ok(s: &str) -> bool {
    let n = s.trim().chars().count();
    (2..=200).contains(&n) && s.chars().all(|c| c.is_alphanumeric() || " .,-'&()/\"".contains(c))
}

fn backup_target_ok(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let path_ok = |p: &str| p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b));
    if s.starts_with('/') {
        return s.len() > 1 && path_ok(s);
    }
    let Some((user_host, path)) = s.split_once(':') else { return false };
    let Some((user, host)) = user_host.split_once('@') else { return false };
    let name_ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    // '-' ile başlayan değer rsync seçeneği sanılmasın
    name_ok(user) && name_ok(host) && user.as_bytes()[0].is_ascii_alphanumeric() && path_ok(path)
}

/// Bağlı cihazda yönetim merkezinden gelen alanlar (spec 2026-10-05 §6): panelde salt okunur, formdan değişmez.
const MERKEZ_SMS: &[&str] = &["sms.mock", "sms.provider", "netgsm.usercode", "netgsm.msgheader", "netgsm.appkey", "netgsm.password"];

const ALANLAR: &[Alan] = &[
    Alan { key: "main.unvan", label: "İşletme unvanı (vergi levhasındaki unvan yazılmalıdır)", tur: Tur::Metin(unvan_ok), grup: "İşletme", admin: false },
    Alan { key: "main.site_name", label: "İşletme adı", tur: Tur::Metin(|s| !s.trim().is_empty() && chars_ok(s, 40, |c| c.is_alphanumeric() || " .-'&".contains(c))), grup: "İşletme", admin: false },
    Alan { key: "main.session_minutes", label: "Oturum süresi (dakika; 43200 = 30 gün)", tur: Tur::Sayi(30, 64800), grup: "İşletme", admin: false },
    Alan { key: "main.max_devices_per_phone", label: "Bir telefona en fazla cihaz", tur: Tur::Sayi(1, 10), grup: "İşletme", admin: false },
    Alan { key: "limits.sms_per_phone_15min", label: "Numara başına SMS / 15 dk", tur: Tur::Sayi(1, 20), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_per_phone_day", label: "Numara başına SMS / gün", tur: Tur::Sayi(1, 50), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_per_mac_hour", label: "Cihaz başına SMS / saat", tur: Tur::Sayi(1, 50), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_global_day", label: "Günlük toplam SMS tavanı", tur: Tur::Sayi(1, 100_000), grup: "SMS sınırları", admin: true },
    Alan { key: "sms.mock", label: "Deneme modu (SMS gerçekten gönderilmez)", tur: Tur::Evet, grup: "SMS sağlayıcısı", admin: true },
    Alan { key: "sms.provider", label: "Türk numaraları için sağlayıcı", tur: Tur::Secim(&[("netgsm", "NetGSM"), ("twilio", "Twilio")]), grup: "SMS sağlayıcısı", admin: true },
    Alan { key: "netgsm.usercode", label: "Kullanıcı kodu (abone no)", tur: Tur::Metin(|s| chars_ok(s, 32, |c| c.is_ascii_alphanumeric())), grup: "NetGSM", admin: true },
    Alan { key: "netgsm.msgheader", label: "Mesaj başlığı", tur: Tur::Metin(|s| chars_ok(s, 11, |c| c.is_ascii_alphanumeric() || " .-".contains(c))), grup: "NetGSM", admin: true },
    Alan { key: "netgsm.appkey", label: "Uygulama anahtarı (opsiyonel)", tur: Tur::Metin(|s| chars_ok(s, 64, |c| c.is_ascii_alphanumeric() || c == '-')), grup: "NetGSM", admin: true },
    Alan { key: "netgsm.password", label: "API şifresi", tur: Tur::Gizli(|s| (1..=64).contains(&s.len()) && s.bytes().all(|b| (0x21..=0x7e).contains(&b))), grup: "NetGSM", admin: true },
    Alan { key: "twilio.enabled", label: "Yabancı numaralara Twilio ile gönder", tur: Tur::Evet, grup: "Twilio", admin: true },
    Alan { key: "twilio.account_sid", label: "Account SID (AC…)", tur: Tur::Metin(|s| prefixed_hex(s, "AC")), grup: "Twilio", admin: true },
    Alan { key: "twilio.verify_sid", label: "Verify Service SID (VA…)", tur: Tur::Metin(|s| prefixed_hex(s, "VA")), grup: "Twilio", admin: true },
    Alan { key: "twilio.auth_token", label: "Auth Token", tur: Tur::Gizli(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())), grup: "Twilio", admin: true },
    Alan { key: "twilio.api_key_sid", label: "API Key SID (SK…, opsiyonel)", tur: Tur::Metin(|s| prefixed_hex(s, "SK")), grup: "Twilio", admin: true },
    Alan { key: "twilio.api_key_secret", label: "API Key Secret (opsiyonel)", tur: Tur::Gizli(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_alphanumeric())), grup: "Twilio", admin: true },
    Alan { key: "uzak.enabled", label: "Uzak erişim açık", tur: Tur::Evet, grup: "Uzak erişim", admin: true },
    Alan { key: "uzak.sunucu", label: "Sunucu (adres:port, ör. vpn.wificorrect.com:51820)", tur: Tur::Metin(|s| s.is_empty() || crate::uzak::endpoint_ok(s)), grup: "Uzak erişim", admin: true },
    Alan { key: "uzak.sunucu_anahtar", label: "Sunucunun açık anahtarı", tur: Tur::Metin(|s| s.is_empty() || crate::uzak::key_ok(s)), grup: "Uzak erişim", admin: true },
    Alan { key: "uzak.adres", label: "Bu cihazın tünel adresi (10.99.0.2–254)", tur: Tur::Metin(|s| s.is_empty() || crate::uzak::adres_ok(s)), grup: "Uzak erişim", admin: true },
    Alan { key: "main.retention_days", label: "Kayıtlar cihazda kaç gün saklansın (730 = 2 yıl)", tur: Tur::Sayi(30, 3650), grup: "Kayıt ve yedek", admin: true },
    Alan { key: "backup.enabled", label: "Uzak yedek açık", tur: Tur::Evet, grup: "Kayıt ve yedek", admin: true },
    Alan { key: "backup.target", label: "Yedek hedefi (kullanici@sunucu:)", tur: Tur::Metin(backup_target_ok), grup: "Kayıt ve yedek", admin: true },
    Alan { key: "backup.ssh", label: "SSH komutu", tur: Tur::Metin(|s| s.starts_with("ssh") && chars_ok(s, 120, |c| c.is_ascii_alphanumeric() || " ./_=-".contains(c))), grup: "Kayıt ve yedek", admin: true },
];

fn b(v: bool) -> String {
    if v { "1" } else { "0" }.into()
}

fn get_field(c: &Config, key: &str) -> String {
    match key {
        "main.site_name" => c.main.site_name.clone(),
        "main.unvan" => c.main.unvan.clone(),
        "uzak.enabled" => b(c.uzak.enabled),
        "uzak.sunucu" => c.uzak.sunucu.clone(),
        "uzak.sunucu_anahtar" => c.uzak.sunucu_anahtar.clone(),
        "uzak.adres" => c.uzak.adres.clone(),
        "main.session_minutes" => c.main.session_minutes.to_string(),
        "main.max_devices_per_phone" => c.main.max_devices_per_phone.to_string(),
        "main.retention_days" => c.main.retention_days.to_string(),
        "limits.sms_per_phone_15min" => c.limits.sms_per_phone_15min.to_string(),
        "limits.sms_per_phone_day" => c.limits.sms_per_phone_day.to_string(),
        "limits.sms_per_mac_hour" => c.limits.sms_per_mac_hour.to_string(),
        "limits.sms_global_day" => c.limits.sms_global_day.to_string(),
        "sms.mock" => b(c.sms.mock),
        "sms.provider" => c.sms.provider.clone(),
        "netgsm.usercode" => c.netgsm.usercode.clone(),
        "netgsm.msgheader" => c.netgsm.msgheader.clone(),
        "netgsm.appkey" => c.netgsm.appkey.clone(),
        "netgsm.password" => c.netgsm.password.clone(),
        "twilio.enabled" => b(c.twilio.enabled),
        "twilio.account_sid" => c.twilio.account_sid.clone(),
        "twilio.verify_sid" => c.twilio.verify_sid.clone(),
        "twilio.auth_token" => c.twilio.auth_token.clone(),
        "twilio.api_key_sid" => c.twilio.api_key_sid.clone(),
        "twilio.api_key_secret" => c.twilio.api_key_secret.clone(),
        "backup.enabled" => b(c.backup.enabled),
        "backup.target" => c.backup.target.clone(),
        "backup.ssh" => c.backup.ssh.clone(),
        _ => String::new(),
    }
}

fn set_field(c: &mut Config, key: &str, v: &str) {
    let n = || v.parse::<u64>().unwrap_or(0);
    match key {
        "main.site_name" => c.main.site_name = v.trim().into(),
        "main.unvan" => c.main.unvan = v.split_whitespace().collect::<Vec<_>>().join(" "),
        "uzak.enabled" => c.uzak.enabled = v == "1",
        "uzak.sunucu" => c.uzak.sunucu = v.into(),
        "uzak.sunucu_anahtar" => c.uzak.sunucu_anahtar = v.into(),
        "uzak.adres" => c.uzak.adres = v.into(),
        "main.session_minutes" => c.main.session_minutes = n(),
        "main.max_devices_per_phone" => c.main.max_devices_per_phone = n() as usize,
        "main.retention_days" => c.main.retention_days = n(),
        "limits.sms_per_phone_15min" => c.limits.sms_per_phone_15min = n() as usize,
        "limits.sms_per_phone_day" => c.limits.sms_per_phone_day = n() as usize,
        "limits.sms_per_mac_hour" => c.limits.sms_per_mac_hour = n() as usize,
        "limits.sms_global_day" => c.limits.sms_global_day = n(),
        "sms.mock" => c.sms.mock = v == "1",
        "sms.provider" => c.sms.provider = v.into(),
        "netgsm.usercode" => c.netgsm.usercode = v.into(),
        "netgsm.msgheader" => c.netgsm.msgheader = v.into(),
        "netgsm.appkey" => c.netgsm.appkey = v.into(),
        "netgsm.password" => c.netgsm.password = v.into(),
        "twilio.enabled" => c.twilio.enabled = v == "1",
        "twilio.account_sid" => c.twilio.account_sid = v.into(),
        "twilio.verify_sid" => c.twilio.verify_sid = v.into(),
        "twilio.auth_token" => c.twilio.auth_token = v.into(),
        "twilio.api_key_sid" => c.twilio.api_key_sid = v.into(),
        "twilio.api_key_secret" => c.twilio.api_key_secret = v.into(),
        "backup.enabled" => c.backup.enabled = v == "1",
        "backup.target" => c.backup.target = v.into(),
        "backup.ssh" => c.backup.ssh = v.into(),
        _ => {}
    }
}

/// Panelden değiştirilemeyen alan ve nedeni: merkezden yönetilen SMS alanları; bağlı cihazda uzak erişim (merkez bağlanırken
/// yazar; tünel üzerinden kapatılırsa panel kendi erişimini keserdi).
fn salt_okunur(a: &Alan, cfg: &Config, bagli: bool) -> Option<&'static str> {
    if cfg.sms.merkez && MERKEZ_SMS.contains(&a.key) {
        Some("merkezden yönetiliyor")
    } else if bagli && a.key.starts_with("uzak.") {
        Some("merkez bağlantısıyla kuruldu")
    } else {
        None
    }
}

/// Ayarlar sayfasının (admin=false) ya da Admin ayarları sayfasının (admin=true) alanları.
fn fields(admin: bool) -> impl Iterator<Item = &'static Alan> {
    ALANLAR.iter().filter(move |a| a.admin == admin)
}

/// Formdan değişiklikler: (anahtar, eski, yeni). Sayfada olmayan alanlar yok sayılır; boş gizli alan = değişmez.
fn validate(admin: bool, form: &Form, cfg: &Config, bagli: bool) -> Result<Vec<(&'static str, String, String)>, HashMap<&'static str, String>> {
    let mut changes = vec![];
    let mut errors = HashMap::new();
    for a in fields(admin) {
        if salt_okunur(a, cfg, bagli).is_some() {
            continue;
        }
        let old = get_field(cfg, a.key);
        let raw = form.get(a.key).map(|s| s.trim().to_string());
        let new = match a.tur {
            Tur::Evet => b(raw.as_deref() == Some("1")),
            Tur::Gizli(ok) => match raw {
                None => continue,
                Some(v) if v.is_empty() => continue,
                Some(v) if ok(&v) => v,
                Some(_) => {
                    errors.insert(a.key, "Geçersiz değer.".into());
                    continue;
                }
            },
            Tur::Metin(ok) => match raw {
                None => continue,
                Some(v) if ok(&v) => v,
                Some(_) => {
                    errors.insert(a.key, "Geçersiz değer.".into());
                    continue;
                }
            },
            Tur::Sayi(lo, hi) => match raw.as_deref().map(str::parse::<u64>) {
                None => continue,
                Some(Ok(n)) if (lo..=hi).contains(&n) => n.to_string(),
                Some(_) => {
                    errors.insert(a.key, format!("{lo} ile {hi} arasında bir sayı girin."));
                    continue;
                }
            },
            Tur::Secim(opts) => match raw {
                None => continue,
                Some(v) if opts.iter().any(|(k, _)| *k == v) => v,
                Some(_) => {
                    errors.insert(a.key, "Geçersiz seçim.".into());
                    continue;
                }
            },
        };
        if new != old {
            changes.push((a.key, old, new));
        }
    }
    if errors.is_empty() {
        Ok(changes)
    } else {
        Err(errors)
    }
}

fn describe(key: &str, old: &str, new: &str) -> String {
    let secret = ALANLAR.iter().any(|a| a.key == key && matches!(a.tur, Tur::Gizli(_)));
    if secret {
        format!("{key}: *** → ***")
    } else {
        format!("{key}: {old} → {new}")
    }
}

// ---------------------------------------------------------------- panel
pub struct Panel {
    pub cfg_path: String,
    pub hesaplar: Hesaplar,
    guard: LoginGuard,
    oturumlar: Oturumlar,
    runner: Box<Runner>,
    clock: Box<Clock>,
    ag_yollar: crate::ag::Yollar,
    /// /sys/class/net (testte geçici klasör)
    sys: std::path::PathBuf,
    /// dnsmasq yasaklı site dosyası (testte geçici)
    filtre_conf: std::path::PathBuf,
    /// WireGuard gizli anahtarı (testte geçici)
    uzak_key: std::path::PathBuf,
    /// Yedek SSH anahtarı (testte geçici)
    yedek_key: std::path::PathBuf,
    /// Yönetim merkezi bağı (testte geçici)
    pub merkez_path: std::path::PathBuf,
    /// Yönetim merkezi HTTP istemcisi (testte sahte)
    pub http: Box<crate::merkez::Http>,
    /// İlk bağlanma tek seferde (iki eşzamanlı giriş iki ayrı cihaz anahtarı almasın)
    baglan_kilit: std::sync::Mutex<()>,
}

const MENU: &[(&str, &str)] = &[("/", "Özet"), ("/cihazlar", "Kullanıcılar"), ("/kayitlar", "Kayıtlar"), ("/ayarlar", "Ayarlar"), ("/admin", "Admin")];

/// Menü grupları tek sayfadır; eski sayfalar bu sayfada alt alta bölüm olur (2026-10-08, kullanıcı isteği).
/// (grup, eski yol, bölüm id, başlık, yalnızca admin). Eski yollar ayrıca açılır (hata/arama sonucu, ayrıntı).
const BOLUMLER: &[(&str, &str, &str, &str, bool)] = &[
    ("/cihazlar", "/oturumlar", "oturumlar", "Bağlı kullanıcılar", false),
    ("/cihazlar", "/yasak", "yasak", "Yasaklı cihazlar", false),
    ("/cihazlar", "/izinli", "izinli", "İzinli cihazlar", false),
    ("/cihazlar", "/yasakli-siteler", "yasakli-siteler", "Yasaklı siteler", false),
    ("/kayitlar", "/kayitlar", "gunler", "Kayıtlar", false),
    ("/kayitlar", "/ara", "site-ara", "Site / IP arama", false),
    ("/kayitlar", "/kullanicilar", "kullanicilar", "Kullanıcılar", false),
    ("/kayitlar", "/talep", "talep", "Resmi talep", false),
    ("/ayarlar", "/ayarlar", "isletme", "İşletme", false),
    ("/ayarlar", "/portal-metinleri", "portal-metinleri", "Portal metinleri", false),
    ("/ayarlar", "/portlar", "portlar", "Portlar", false),
    ("/ayarlar", "/sistem", "sistem", "Sistem", false),
    ("/ayarlar", "/sifre", "sifre-degistir", "Şifremi değiştir", false),
    ("/admin", "/admin-ayarlari", "admin-ayarlari", "Admin ayarları", true),
    ("/admin", "/panel-hareketleri", "panel-hareketleri", "Panel hareketleri", true),
];

/// Yolun menü grubu: eski bölüm yolu ya da onun alt sayfası (/kayitlar/gun, /kullanici, /talep/paket …).
fn menu_grubu(path: &str) -> &'static str {
    match path {
        "/kullanici" => return "/kayitlar",
        "/admin-kilidi" => return "/admin",
        _ => {}
    }
    BOLUMLER
        .iter()
        .find(|(g, y, ..)| path == *g || path == *y || path.strip_prefix(y).is_some_and(|r| r.starts_with('/')))
        .map_or("/", |(g, ..)| g)
}

// çizgi simgeleri (24×24, stroke): menü ve Özet kısayolları
const SIMGE_EV: &str = "M3 11 12 3l9 8M5 9.5V21h5v-6h4v6h5V9.5";
const SIMGE_WIFI: &str = "M2 8.5a15 15 0 0 1 20 0M5 12a10 10 0 0 1 14 0M8.5 15.5a5 5 0 0 1 7 0M12 19h.01";
const SIMGE_KAYIT: &str = "M6 2h9l5 5v15H6zM14 2v6h6M9 13h8M9 17h8";
const SIMGE_AYAR: &str = "M4 6h16M4 12h16M4 18h16M9 4v4M15 10v4M7 16v4";
const SIMGE_KILIT: &str = "M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4";
const SIMGE_ARA: &str = "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM16 16l5 5";
const SIMGE_YASAK: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM5.6 5.6l12.8 12.8";
const SIMGE_MENU: &str = "M3 6h18M3 12h18M3 18h18";

fn simge(d: &str) -> String {
    format!("<svg class=\"simge\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"{d}\"/></svg>")
}

fn menu_simgesi(yol: &str) -> &'static str {
    match yol {
        "/" => SIMGE_EV,
        "/cihazlar" => SIMGE_WIFI,
        "/kayitlar" => SIMGE_KAYIT,
        "/ayarlar" => SIMGE_AYAR,
        _ => SIMGE_KILIT,
    }
}

const SIMGE_TAMAM: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM8 12l3 3 5-6";
const SIMGE_DUNYA: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3c3 3.5 3 14.5 0 18M12 3c-3 3.5-3 14.5 0 18";
const SIMGE_KISILER: &str = "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM2 21v-1a6 6 0 0 1 12 0v1M16 3.5a4 4 0 0 1 0 7.5M22 21v-1a6 6 0 0 0-4-5.6";
const SIMGE_MUHUR: &str = "M9 3h6v5l3 4H6l3-4zM4 16h16v3H4zM7 22h10";
const SIMGE_DUKKAN: &str = "M3 9l1.5-5h15L21 9M3 9h18v1a3 3 0 0 1-6 0 3 3 0 0 1-6 0 3 3 0 0 1-6 0zM5 13v8h14v-8M10 21v-5h4v5";
const SIMGE_METIN: &str = "M4 4h16v12H8l-4 4z";
const SIMGE_FIS: &str = "M9 2v6M15 2v6M6 8h12v4a6 6 0 0 1-12 0zM12 18v4";
const SIMGE_SUNUCU: &str = "M3 4h18v7H3zM3 13h18v7H3zM7 7.5h.01M7 16.5h.01";
const SIMGE_ANAHTAR: &str = "M8 11a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM11 13l9-9M16 8l3 3M18 6l2 2";
const SIMGE_GOZ: &str = "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z";

/// Grup sayfasının üstündeki bölüm kutucuklarının simgesi (bölüm id'sine göre)
fn bolum_simgesi(id: &str) -> &'static str {
    match id {
        "oturumlar" => SIMGE_WIFI,
        "yasak" => SIMGE_YASAK,
        "izinli" => SIMGE_TAMAM,
        "yasakli-siteler" => SIMGE_DUNYA,
        "gunler" => SIMGE_KAYIT,
        "site-ara" => SIMGE_ARA,
        "kullanicilar" => SIMGE_KISILER,
        "talep" => SIMGE_MUHUR,
        "isletme" => SIMGE_DUKKAN,
        "portal-metinleri" => SIMGE_METIN,
        "portlar" => SIMGE_FIS,
        "sistem" => SIMGE_SUNUCU,
        "sifre-degistir" => SIMGE_ANAHTAR,
        "admin-ayarlari" => SIMGE_AYAR,
        "panel-hareketleri" => SIMGE_GOZ,
        _ => SIMGE_KAYIT,
    }
}

/// Özet'in üstündeki kısayol kutuları: (bağlantı, simge, ad)
const KISAYOLLAR: &[(&str, &str, &str)] = &[
    ("/cihazlar#oturumlar", SIMGE_WIFI, "Bağlı kullanıcılar"),
    ("/kayitlar#site-ara", SIMGE_ARA, "Site / IP ara"),
    ("/cihazlar#yasakli-siteler", SIMGE_YASAK, "Yasaklı siteler"),
];

/// Özet listelerinde en çok bu kadar satır; fazlası "Tümü" bağlantısıyla
const OZET_SATIR: usize = 10;

/// "2026-10-10T14:02:11+03:00" → bugünse "14:02", değilse "09.10 14:02"
fn kisa_zaman(iso: &str, bugun: &str) -> String {
    let saat = iso.get(11..16).unwrap_or("");
    match (iso.get(..10), iso.get(5..7), iso.get(8..10)) {
        (Some(g), _, _) if g == bugun => saat.into(),
        (_, Some(ay), Some(gun)) => format!("{gun}.{ay} {saat}"),
        _ => String::new(),
    }
}

struct ParcaKipi;

impl ParcaKipi {
    fn ac() -> ParcaKipi {
        PARCA.with(|p| p.set(true));
        ParcaKipi
    }
}

impl Drop for ParcaKipi {
    fn drop(&mut self) {
        PARCA.with(|p| p.set(false));
    }
}

thread_local! {
    // ponytail: grup sayfası, bölüm işleyicilerini çağırıp yalnızca gövdelerini alır (her işleyici page() ile biter).
    // İşleyicileri gövde/sayfa diye ikiye bölmek yerine iş parçacığına özel bir bayrak; istek tek iş parçacığında işlenir.
    static PARCA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Panel {
    pub fn new(cfg_path: &str, hesap_path: &str, runner: Box<Runner>, clock: Box<Clock>) -> Panel {
        Panel {
            cfg_path: cfg_path.into(),
            hesaplar: Hesaplar::new(hesap_path),
            guard: LoginGuard::default(),
            oturumlar: Oturumlar::default(),
            runner,
            clock,
            ag_yollar: crate::ag::Yollar::sistem(),
            sys: "/sys/class/net".into(),
            filtre_conf: crate::filtre::DNSMASQ_CONF.into(),
            uzak_key: crate::uzak::KEY.into(),
            yedek_key: crate::uzak::YEDEK_KEY.into(),
            merkez_path: crate::merkez::PATH.into(),
            http: Box::new(crate::merkez::curl),
            baglan_kilit: std::sync::Mutex::new(()),
        }
    }

    fn page(&self, cfg: &Config, req: &Req, o: Option<&Oturum>, title: &str, body: &str) -> Resp {
        if PARCA.with(std::cell::Cell::get) {
            return Resp { status: 200, body: body.into(), headers: vec![], file: None };
        }
        let (menu, ust) = match o {
            Some(o) => {
                let grup = menu_grubu(&req.path);
                let links: String = MENU
                    .iter()
                    .map(|(p, l)| {
                        let act = if *p == grup { " class=\"aktif\" aria-current=\"page\"" } else { "" };
                        format!("<a href=\"{p}\"{act}>{}{}</a>", simge(menu_simgesi(p)), h(l))
                    })
                    .collect();
                (
                    format!("<nav aria-label=\"Menü\">{links}</nav>"),
                    format!(
                        "<div class=\"ust-sag\"><div class=\"kim\">{}<span>{}</span></div>\
                         <form method=\"post\" action=\"/cikis\">{}<button class=\"ikincil\">Çıkış</button></form></div>",
                        h(&o.user),
                        h(o.rol.ad()),
                        csrf_input(o)
                    ),
                )
            }
            None => (String::new(), String::new()),
        };
        // telefonda menü: betik yok (CSP), gizli onay kutusu + etiket açar/kapatır
        let (dugme, etiket) = if o.is_some() {
            (
                "<input type=\"checkbox\" id=\"menu-ac\" class=\"gizli\">".to_string(),
                format!("<label for=\"menu-ac\" class=\"menu-dugme\" title=\"Menü\"><span class=\"gizli\">Menüyü aç/kapat</span>{}</label>", simge(SIMGE_MENU)),
            )
        } else {
            (String::new(), String::new())
        };
        let msg = req.query.get("m").map(|m| {
            let cls = if req.query.get("e").is_some_and(|e| e == "1") { "mesaj hata" } else { "mesaj" };
            format!("<div class=\"{cls}\">{}</div>", h(m))
        });
        let mut v: HashMap<&str, String> = HashMap::new();
        v.insert("baslik", h(title));
        // cihaz bir müşteriye bağlanmadan işletme adı yok (eski/örnek ayardaki ad görünmesin)
        v.insert("site", h(if crate::merkez::oku(&self.merkez_path).is_none() { "WifiCorrect" } else { &cfg.main.site_name }));
        v.insert("govde", if o.is_some() { String::new() } else { "yalin".into() });
        v.insert("menu_html", menu);
        v.insert("ust_html", ust);
        v.insert("menu_dugme_html", dugme);
        v.insert("menu_etiket_html", etiket);
        let mut uyari = String::new();
        if o.is_some() {
            let now = (self.clock)();
            match crate::merkez::oku(&self.merkez_path) {
                None => uyari.push_str("<div class=\"mesaj hata\">Cihaz bir müşteriye bağlı değil: misafirlere internet verilmiyor. Müşteri numarasıyla giriş yapılınca açılır.</div>"),
                Some(m) if !crate::merkez::lisans_aktif(&m, now) => uyari.push_str(&format!(
                    "<div class=\"mesaj hata\">{}: misafirlere internet verilmiyor. Hizmet sağlayıcınıza başvurun.</div>",
                    if m.lisans == "askida" { "Lisans askıya alındı".to_string() } else { format!("Lisans süresi doldu ({})", h(&m.lisans_bitis)) }
                )),
                Some(_) => {}
            }
            if cfg.main.unvan.trim().is_empty() && crate::merkez::oku(&self.merkez_path).is_some() {
                uyari.push_str("<div class=\"mesaj hata\">Unvan girilmemiş: misafirlerin gördüğü açık rıza metninde yer tutucu görünüyor. Ayarlar'dan işletme unvanını girin.</div>");
            }
        }
        v.insert("mesaj_html", uyari + &msg.unwrap_or_default());
        v.insert("icerik_html", body.into());
        Resp { status: 200, body: substitute(TPL, &v), headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())], file: None }
    }

    /// Menü grubu sayfası (/cihazlar, /kayitlar, /ayarlar): bölümler alt alta, üstte bölümlere atlama bağlantıları.
    fn grup_sayfasi(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let hizmet = o.rol == Rol::Hizmet;
        let gerekce = hizmet || o.gerekce.as_ref().is_some_and(|(_, until)| *until > now);
        let bolumler: Vec<_> = BOLUMLER.iter().filter(|(g, .., yalniz_admin)| *g == req.path && (hizmet || !yalniz_admin)).collect();
        let parca = ParcaKipi::ac(); // bir bölüm panik yapsa da bayrak iş parçacığında kalmasın
        let govdeler: Vec<String> = bolumler
            .iter()
            .map(|(_, yol, ..)| match *yol {
                "/oturumlar" => self.oturumlar_sayfa(cfg, req, o, now).body,
                "/yasak" => self.liste(cfg, req, o, true).body,
                "/izinli" => self.liste(cfg, req, o, false).body,
                "/yasakli-siteler" => self.filtre_sayfa(cfg, req, o).body,
                "/kayitlar" => self.kayitlar(cfg, req, o).body,
                "/panel-hareketleri" => self.hareketler(cfg, req, o, now).body,
                // kişisel veri: işletme sahibi gerekçe yazmadan liste gösterilmez (gerekçe sayfası /kullanicilar'da açılır)
                "/kullanicilar" if !gerekce => "<p>Müşterilerin kişisel verisini görmek için önce gerekçe yazmanız gerekir; \
                     her görüntüleme gerekçesiyle kaydedilir.</p><p><a class=\"dugme\" href=\"/kullanicilar\">Kullanıcıları göster</a></p>"
                    .into(),
                "/kullanicilar" => self.kullanicilar(cfg, req, o).body,
                "/talep" => self.talep(cfg, req, o, now).body,
                "/ara" => self.site_ara(cfg, req, o, now).body,
                "/ayarlar" => self.ayarlar(cfg, req, o, &HashMap::new(), false).body,
                "/portal-metinleri" => self.metinler(cfg, req, o).body,
                "/portlar" => self.portlar(cfg, req, o, now).body,
                "/sistem" => self.sistem(cfg, req, o, "").body,
                "/admin-ayarlari" => self.ayarlar(cfg, req, o, &HashMap::new(), true).body,
                _ => self.sifre(cfg, req, o, "").body,
            })
            .collect();
        drop(parca);
        let atla: String = bolumler.iter().map(|(_, _, id, baslik, _)| format!("<a href=\"#{id}\">{}{}</a>", simge(bolum_simgesi(id)), h(baslik))).collect();
        let mut body = format!("<nav class=\"kisayol\" aria-label=\"Bu sayfadaki bölümler\">{atla}</nav>");
        for ((_, _, id, baslik, _), govde) in bolumler.iter().zip(&govdeler) {
            body.push_str(&format!(
                "<section class=\"bolum\" id=\"{id}\" aria-labelledby=\"b-{id}\"><h2 class=\"bolum-baslik\" id=\"b-{id}\">{}</h2>{govde}</section>",
                h(baslik)
            ));
        }
        let baslik = MENU.iter().find(|(p, _)| *p == req.path).map_or("", |(_, l)| l);
        self.page(cfg, req, Some(o), baslik, &body)
    }

    fn audit(&self, cfg: &Config, req: &Req, o: Option<&Oturum>, olay: &str, ek: &str) {
        let who = o.map_or("-".to_string(), |o| format!("{} rol={}", o.user, if o.rol == Rol::Hizmet { "hizmet" } else { "sahip" }));
        let mut ek = format!("kullanici={who} ip={}{}{ek}", req.ip, if ek.is_empty() { "" } else { " " });
        if let Some((g, until)) = o.and_then(|o| o.gerekce.as_ref()) {
            if *until > (self.clock)() && olay != "PANEL_GEREKCE" {
                ek.push_str(&format!(" gerekce={g}"));
            }
        }
        ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso((self.clock)())).set("ek", ek));
    }

    pub fn handle(&self, req: &Req) -> Resp {
        let cfg = match Config::load(&self.cfg_path) {
            Ok(c) => c,
            Err(e) => return text(500, &e),
        };
        if ortak::in_subnet(&req.ip, &cfg.main.subnet) {
            return text(403, "Yasak"); // müşteri ağından panel yok
        }
        let now = (self.clock)();
        // Kimlik başlıkları yalnızca açık tünelden, merkezin tünel adresinden gelirse geçerli; başka kaynakta yok sayılır.
        // Tünel kapalıysa (ayar ya da "wfc" arayüzü yok) 10.99.0.1'e yanıt varsayılan yoldan gider: kaynak taklit edilebilir.
        let tunel = cfg.uzak.enabled && crate::uzak::adres_ok(cfg.uzak.adres.trim()) && self.sys.join("wfc").exists();
        let merkezden = tunel && merkez_adresi(&cfg).is_some_and(|m| m == req.ip);
        let ileten = req.ileten_ip.as_deref().filter(|s| s.parse::<std::net::IpAddr>().is_ok());
        let req = &Req {
            ip: if merkezden { ileten.unwrap_or(&req.ip).to_string() } else { req.ip.clone() },
            merkez_kullanici: if merkezden { req.merkez_kullanici.clone() } else { None },
            ileten_ip: None,
            ..clone_req(req)
        };
        if req.path == "/kurulum" {
            return text(404, "Sayfa bulunamadı");
        }
        let Some(bag) = crate::merkez::oku(&self.merkez_path) else {
            // bağlı değil: yalnızca eşleştirme (müşteri numarasıyla ilk giriş cihazı merkeze bağlar)
            return match (req.method.as_str(), req.path.as_str()) {
                ("GET", "/giris") => self.giris(&cfg, req, ""),
                ("POST", "/giris") => self.giris_post(&cfg, req, now),
                _ => redirect("/giris", None),
            };
        };
        // bağlıyken panel yalnızca merkez üzerinden (2026-10-09, kullanıcı kararı)
        if !merkezden {
            return Self::yerel_kapali();
        }
        if req.merkez_kullanici.as_ref().is_some_and(|k| *k != bag.numara) {
            return text(403, "Bu cihaz bu müşteriye ait değil.");
        }
        if req.path == "/giris" {
            return Self::yerel_kapali();
        }
        // geçerli oturum korunur; yoksa merkezin doğruladığı müşteriye oturum açılır
        let mut yeni = None;
        let mut o = match self.gecerli_oturum(req, now) {
            Some(o) => o,
            None if req.merkez_kullanici.is_some() => {
                let t = self.oturumlar.create(&bag.numara, Rol::Sahip, now, &bag.ozet);
                let o = self.oturumlar.get(&t, now).expect("yeni oturum");
                self.audit(&cfg, req, Some(&o), "PANEL_GIRIS", "kaynak=merkez");
                yeni = Some(t);
                o
            }
            None => return redirect("/giris", None),
        };
        // admin kilidi açık ve admin parolası o zamandan beri değişmedi: etkin rol hizmet sağlayıcı
        let admin = o.admin_kadar > now && self.hesaplar.ozet(hesap::ADMIN).is_some_and(|z| hesap::ct_eq(&z, &o.admin_surum));
        o.rol = if admin { Rol::Hizmet } else { Rol::Sahip };
        let req = &Req { token: yeni.clone().or_else(|| req.token.clone()), ..clone_req(req) };
        let mut r = self.oturumlu(cfg, req, o, now);
        if let Some(t) = yeni {
            r.headers.push(("Set-Cookie".into(), format!("wfc={t}; Path=/; Secure; HttpOnly; SameSite=Lax")));
        }
        r
    }

    /// Çerezdeki oturum hâlâ geçerliyse (bağ ve parola özeti aynı); değilse silinir.
    fn gecerli_oturum(&self, req: &Req, now: f64) -> Option<Oturum> {
        let t = req.token.as_deref()?;
        let o = self.oturumlar.get(t, now)?;
        let gecerli = self.sahip_oturumu_gecerli(&o);
        if !gecerli {
            self.oturumlar.remove(t);
        }
        gecerli.then_some(o)
    }

    /// Bağlı cihazda dükkân içinden (merkez dışından) gelen her isteğe: yönlendirme notu.
    /// Bağlı cihaz merkez dışından gelen isteğe hiçbir şey göstermez (2026-10-10, kullanıcı kararı: not sayfası da yok).
    fn yerel_kapali() -> Resp {
        Resp { status: 404, body: String::new(), headers: vec![], file: None }
    }

    /// Admin kilidi formu (Admin ayarları ve Panel hareketleri kilitliyken). Admin özeti hiç gelmediyse form yok.
    fn admin_kilidi_formu(&self, cfg: &Config, req: &Req, o: &Oturum, err: &str) -> Resp {
        let body = if self.hesaplar.ozet(hesap::ADMIN).is_none() {
            "<div class=\"kart dar\"><p>Admin parolası belirlenmemiş; hizmet sağlayıcınız merkezden belirler.</p></div>".to_string()
        } else {
            let err = if err.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", h(err)) };
            format!(
                "<div class=\"kart dar\"><p>Admin ayarları ve Panel hareketleri admin parolasıyla 30 dakika açılır.</p>{err}\
                 <form method=\"post\" action=\"/admin-kilidi\">{}<input type=\"hidden\" name=\"donus\" value=\"{}\">\
                 <label for=\"ap\">Admin parolası</label><input type=\"password\" id=\"ap\" name=\"parola\" autocomplete=\"current-password\" required autofocus>\
                 <div style=\"margin-top:20px\"><button>Aç</button></div></form></div>",
                csrf_input(o),
                admin_donus(req.form.get("donus").unwrap_or(&req.path))
            )
        };
        self.page(cfg, req, Some(o), "Admin", &body)
    }

    fn admin_kilidi(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let anahtar = format!("admin-kilidi:{}", o.user);
        if !self.guard.allowed(&req.ip, &anahtar, now) {
            self.audit(cfg, req, Some(o), "PANEL_ADMIN_HATALI", "kilit=1");
            return self.admin_kilidi_formu(cfg, req, o, "Çok fazla hatalı deneme. 15 dakika sonra tekrar deneyin.");
        }
        let pw = req.form.get("parola").map_or("", String::as_str);
        match (self.hesaplar.verify(hesap::ADMIN, pw), self.hesaplar.ozet(hesap::ADMIN), req.token.as_deref()) {
            (Some(Rol::Hizmet), Some(ozet), Some(t)) => {
                self.guard.succeeded(&req.ip, &anahtar);
                self.oturumlar.set_admin(t, now + ADMIN_SURE, &ozet);
                self.audit(cfg, req, Some(o), "PANEL_ADMIN_ACILDI", "");
                redirect(admin_donus(req.form.get("donus").map_or("", String::as_str)), None)
            }
            _ => {
                self.guard.failed(&req.ip, &anahtar, now);
                self.audit(cfg, req, Some(o), "PANEL_ADMIN_HATALI", "");
                self.admin_kilidi_formu(cfg, req, o, "Admin parolası hatalı.")
            }
        }
    }

    /// Oturumlu istek. Bağlı cihazda buraya yalnızca merkez üzerinden gelinir.
    fn oturumlu(&self, cfg: Config, req: &Req, o: Oturum, now: f64) -> Resp {
        if req.method == "POST" && !hesap::ct_eq(req.form.get("csrf").map_or("", String::as_str), &o.csrf) {
            return text(403, "Geçersiz form (CSRF). Sayfayı yenileyip tekrar deneyin.");
        }
        let hizmet = o.rol == Rol::Hizmet;
        if let Some(r) = self.reason_gate(&cfg, req, &o, now) {
            return r;
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/gerekce") => self.gerekce_post(&cfg, req, &o, now),
            ("POST", "/cikis") => {
                self.audit(&cfg, req, Some(&o), "PANEL_CIKIS", "");
                if let Some(t) = &req.token {
                    self.oturumlar.remove(t);
                }
                // bağlı cihazda oturum merkezden açılmıştır: merkez iki oturumu da kapatır
                let bagli = crate::merkez::oku(&self.merkez_path).is_some();
                let mut r = redirect(if bagli { "/_cikis" } else { "/giris" }, None);
                r.headers.push(("Set-Cookie".into(), "wfc=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict".into()));
                r
            }
            ("GET", "/") => self.ozet(&cfg, req, &o, now),
            ("GET", "/cihazlar" | "/kayitlar" | "/ayarlar") => self.grup_sayfasi(&cfg, req, &o, now),
            ("GET", "/oturumlar") => self.oturumlar_sayfa(&cfg, req, &o, now),
            ("POST", "/oturumlar/at") => self.at(&cfg, req, &o, now),
            ("GET", "/yasak") => self.liste(&cfg, req, &o, true),
            ("GET", "/izinli") => self.liste(&cfg, req, &o, false),
            ("POST", "/yasak/ekle") => self.liste_ekle(cfg, req, &o, true, now),
            ("POST", "/izinli/ekle") => self.liste_ekle(cfg, req, &o, false, now),
            ("POST", "/yasak/kaldir") => self.liste_kaldir(cfg, req, &o, true),
            ("POST", "/izinli/kaldir") => self.liste_kaldir(cfg, req, &o, false),
            ("GET", "/admin") if hizmet => self.grup_sayfasi(&cfg, req, &o, now),
            ("GET", "/panel-hareketleri") if hizmet => self.hareketler(&cfg, req, &o, now),
            ("GET", "/admin-ayarlari") if hizmet => self.ayarlar(&cfg, req, &o, &HashMap::new(), true),
            ("GET", "/admin" | "/admin-ayarlari" | "/panel-hareketleri") => self.admin_kilidi_formu(&cfg, req, &o, ""),
            ("POST", "/admin-kilidi") => self.admin_kilidi(&cfg, req, &o, now),
            ("POST", "/admin-kilidi/kapat") => {
                if let Some(t) = &req.token {
                    self.oturumlar.set_admin(t, 0.0, "");
                }
                self.audit(&cfg, req, Some(&o), "PANEL_ADMIN_KAPANDI", "");
                redirect("/", None)
            }
            ("GET", "/kayitlar/gun") => self.kayit_gun(&cfg, req, &o),
            ("GET", "/kayitlar/dosya") => self.kayit_dosya(&cfg, req, &o),
            ("GET", "/kayitlar/indir") => self.kayit_indir(&cfg, req, &o),
            ("GET", "/kayitlar/gun-indir") => self.kayit_gun_indir(&cfg, req, &o),
            ("GET", "/kullanicilar") => self.kullanicilar(&cfg, req, &o),
            ("GET", "/kullanici") => self.kullanici(&cfg, req, &o),
            ("GET", "/talep") => self.talep(&cfg, req, &o, now),
            ("GET", "/ara") => self.site_ara(&cfg, req, &o, now),
            ("GET", "/talep/paket") => self.talep_paket(&cfg, req, &o),
            ("GET", "/yasakli-siteler") => self.filtre_sayfa(&cfg, req, &o),
            ("POST", "/yasakli-siteler/ekle") => self.filtre_degistir(cfg, req, &o, true),
            ("POST", "/yasakli-siteler/kaldir") => self.filtre_degistir(cfg, req, &o, false),
            ("GET", "/portal-metinleri") => self.metinler(&cfg, req, &o),
            ("POST", "/portal-metinleri") => self.metinler_kaydet(cfg, req, &o),
            ("GET", "/portlar") => self.portlar(&cfg, req, &o, now),
            ("POST", "/portlar") => self.portlar_uygula(&cfg, req, &o),
            ("POST", "/portlar/onayla") => self.portlar_onayla(&cfg, req, &o),
            ("POST", "/portlar/geri-al") => self.portlar_geri_al(&cfg, req, &o),
            ("POST", "/portlar/etiket") => self.portlar_etiket(&cfg, req, &o),
            ("POST", "/ayarlar") => self.ayarlar_post(cfg, req, &o, false),
            ("POST", "/admin-ayarlari") if hizmet => self.ayarlar_post(cfg, req, &o, true),
            ("POST", "/admin-ayarlari/fabrika") if hizmet => self.fabrika(&cfg, req, &o),
            ("POST", "/admin-ayarlari/yedekle") if hizmet => self.yedekle(&cfg, req, &o),
            ("POST", "/admin-ayarlari" | "/admin-ayarlari/fabrika" | "/admin-ayarlari/yedekle") => text(403, "Admin kilidi kapalı."),
            ("GET", "/sistem") => self.sistem(&cfg, req, &o, ""),
            ("POST", "/sistem/gun-kapat") => {
                let out = crate::muhur::gun_kapat(&cfg, None, now, false);
                self.audit(&cfg, req, Some(&o), "PANEL_GUN_KAPAT", "");
                let msg = if out.is_empty() { "Kapatılacak gün yok.".to_string() } else { out.join(" · ") };
                redirect("/sistem", Some((&msg, msg.contains("HATA"))))
            }
            ("POST", "/sistem/dogrula") => {
                let r = crate::muhur::format_verify(&crate::muhur::verify(&cfg, None));
                self.sistem(&cfg, req, &o, &r)
            }
            ("POST", "/sistem/yeniden-baslat") => {
                let ok = (self.runner)(&cmd(&["systemctl", "restart", "wificorrect-portal", "wificorrect-kaydedici"]));
                self.audit(&cfg, req, Some(&o), "PANEL_YENIDEN_BASLAT", "");
                redirect("/sistem", Some((if ok { "Portal ve kaydedici yeniden başlatıldı." } else { "Yeniden başlatılamadı!" }, !ok)))
            }
            ("GET", "/sifre") => self.sifre(&cfg, req, &o, ""),
            ("POST", "/sifre") => self.sifre_post(&cfg, req, &o),
            _ => text(404, "Sayfa bulunamadı"),
        }
    }

    // --- giriş
    /// Sahip oturumu: cihaz hâlâ bu numaraya bağlı ve parola özeti girişteki gibi (merkezden sıfırlanmadı).
    fn sahip_oturumu_gecerli(&self, o: &Oturum) -> bool {
        crate::merkez::oku(&self.merkez_path).is_some_and(|m| m.numara == o.user && hesap::ct_eq(&m.ozet, &o.surum))
    }

    fn giris(&self, cfg: &Config, req: &Req, err: &str) -> Resp {
        let err = if err.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", h(err)) };
        let body = format!(
            "<div class=\"kart dar\"><h2>Cihazı eşleştir — müşteri numarası ve parola</h2>{err}<form method=\"post\" action=\"/giris\"><label for=\"k\">Müşteri numarası</label>\
             <input type=\"text\" id=\"k\" name=\"kullanici\" autocomplete=\"username\" required autofocus>\
             <label for=\"p\">Parola</label><input type=\"password\" id=\"p\" name=\"parola\" autocomplete=\"current-password\" required>\
             <div style=\"margin-top:20px\"><button>Giriş</button></div></form></div>"
        );
        self.page(cfg, req, None, "Cihazı eşleştir", &body)
    }

    fn giris_post(&self, cfg: &Config, req: &Req, now: f64) -> Resp {
        let user = req.form.get("kullanici").map_or("", String::as_str).trim().to_lowercase();
        let pw = req.form.get("parola").map_or("", String::as_str);
        if !self.guard.allowed(&req.ip, &user, now) {
            self.audit(cfg, req, None, "PANEL_GIRIS_KILIT", &format!("kullanici_adi={user}"));
            return self.giris(cfg, req, "Çok fazla hatalı deneme. 15 dakika sonra tekrar deneyin.");
        }
        let hatali = |e: &str| {
            self.guard.failed(&req.ip, &user, now);
            self.audit(cfg, req, None, "PANEL_GIRIS_HATA", &format!("kullanici_adi={user}"));
            self.giris(cfg, req, e)
        };
        let surum = {
            // bağ kontrolü ve ilk bağlanma kilit altında: aynı anda iki giriş iki ayrı cihaz anahtarı almasın
            let _k = self.baglan_kilit.lock().unwrap_or_else(|e| e.into_inner());
            match crate::merkez::oku(&self.merkez_path) {
                Some(m) => {
                    if user != m.numara {
                        return hatali(if crate::merkez::numara_gecerli(&user) { "Bu cihaz başka bir müşteriye ait." } else { "Kullanıcı adı veya parola hatalı." });
                    }
                    if !crate::merkez::dogrula(&m, pw) {
                        return hatali("Kullanıcı adı veya parola hatalı.");
                    }
                    m.ozet
                }
                None if !crate::merkez::numara_gecerli(&user) => return hatali("Kullanıcı adı veya parola hatalı."),
                None => match self.merkeze_baglan(cfg, req, &user, pw, now) {
                    Ok(ozet) => ozet,
                    Err((mesaj, kilit)) => return if kilit { hatali(&mesaj) } else { self.giris(cfg, req, &mesaj) },
                },
            }
        };
        self.guard.succeeded(&req.ip, &user);
        let token = self.oturumlar.create(&user, Rol::Sahip, now, &surum);
        let o = self.oturumlar.get(&token, now);
        self.audit(cfg, req, o.as_ref(), "PANEL_GIRIS", "");
        // sonuç bir kez gösterilir; bundan sonra cihaz yerelden erişimsiz (oturum çerezi verilmez)
        let _ = token;
        self.page(cfg, req, None, "Cihaz eşleşti",
                  "<div class=\"kart dar\"><p><b>Cihaz eşleşti.</b> Bundan sonra panele <a href=\"https://panel.wificorrect.com\">panel.wificorrect.com</a> \
                   üzerinden müşteri numaranız ve parolanızla girin.</p></div>")
    }

    /// Bağlı olmayan cihazda ilk müşteri girişi: merkez parolayı doğrular, cihazı bağlar; tünel ve yedek ayarlanır.
    /// Dönen: parola özeti (oturum sürümü) ya da (mesaj, kilit sayacına yazılsın mı).
    fn merkeze_baglan(&self, cfg: &Config, req: &Req, numara: &str, pw: &str, now: f64) -> Result<String, (String, bool)> {
        let wg = crate::uzak::ensure_key(&self.uzak_key).and_then(|k| crate::uzak::public_key(&k));
        let ssh = crate::uzak::yedek_anahtari(&self.yedek_key);
        let (wg, ssh) = match (wg, ssh) {
            (Ok(w), Ok(s)) => (w, s),
            (Err(e), _) | (_, Err(e)) => return Err((format!("Cihaz anahtarı üretilemedi: {e}"), false)),
        };
        let g = match crate::merkez::giris(numara, pw, &wg, &ssh, &cfg.main.site_name, &cfg.main.unvan, &*self.http, now) {
            Ok(g) => g,
            Err(crate::merkez::GirisHata::Baglanti) => return Err(("Merkeze ulaşılamadı, internet bağlantısını kontrol edin. Modem kablosunun Ethernet 1'e takılı olduğunu kontrol edin.".into(), false)),
            Err(crate::merkez::GirisHata::Mesaj(m, kilit)) => return Err((m, kilit)),
        };
        // önce ayarlar (tünel + yedek), sonra bağ: ayar yazılamazsa cihaz "bağlı ama tünelsiz" kalmasın
        let mut yeni = cfg.clone();
        crate::merkez::uygula(&mut yeni, &g);
        let (admin, sms) = crate::merkez::ek_uygula(&mut yeni, &self.hesaplar, &g.ek);
        yeni.save(&self.cfg_path).map_err(|e| (e, false))?;
        crate::merkez::kaydet(&self.merkez_path, &g.merkez).map_err(|e| (e, false))?;
        // güncellenen eski cihazdaki yerel sahip hesapları artık geçersiz: yalnızca admin kalır
        if let Err(e) = self.hesaplar.keep_only_admin() {
            eprintln!("panel: {e}");
        }
        let unit = format!("wfc-uzak-{}", ortak::random_hex(4));
        (self.runner)(&cmd(&["systemd-run", "--collect", "--unit", &unit, "--on-active", "1s", "/usr/local/bin/wificorrect", "ctl", "uzak-uygula"]));
        self.audit(&yeni, req, None, "PANEL_MERKEZ_BAGLANDI", &format!("numara={numara} tunel={}", g.tunel_ip));
        match admin {
            Ok(true) => self.audit(&yeni, req, None, "ADMIN_PAROLA_MERKEZ", ""),
            Ok(false) => {}
            Err(e) => self.audit(&yeni, req, None, "MERKEZ_EK_HATA", &e),
        }
        match sms {
            Ok(true) => {
                self.audit(&yeni, req, None, "SMS_AYARI_MERKEZ", ""); // şifre yazılmaz
                (self.runner)(&cmd(&["systemctl", "restart", "wificorrect-portal"]));
            }
            Ok(false) => {}
            Err(e) => self.audit(&yeni, req, None, "MERKEZ_EK_HATA", &e),
        }
        Ok(g.merkez.ozet)
    }

    // --- özet ve oturumlar
    fn ozet(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        // ad soyad gösterir: Bağlı cihazlar sayfası gibi denetim kaydına girer
        self.audit(cfg, req, Some(o), "PANEL_OZET", "");
        let m = &cfg.main;
        let today = ortak::day_of(&ortak::now_iso(now)).to_string();
        // bağlı = oturumu süren ve son 5 dakikada ağda görülen cihaz (Cihazlar sayfasındaki yeşil "Bağlı" ile aynı)
        let agda = std::fs::read(self.sys.join(&m.iface).join("brforward")).map(|b| ortak::kopruye_bagli(&b)).unwrap_or_default();
        let mut bagli: Vec<ortak::Session> = ortak::load_sessions(&m.state_root)
            .into_iter()
            .filter(|(mac, s)| !s.ip.is_empty() && s.expires_epoch > now && agda.contains(mac))
            .map(|(_, s)| s)
            .collect();
        bagli.sort_by(|a, b| b.start_epoch.total_cmp(&a.start_epoch)); // son bağlanan üstte
        let mut yeni: Vec<[String; 5]> = ortak::read_index(&m.log_root).into_values().filter(|r| r[3].starts_with(&today)).collect();
        yeni.sort_by(|a, b| b[3].cmp(&a[3]));
        let kisiler: BTreeSet<String> = std::fs::read_to_string(ortak::day_file(&m.log_root, &today, "oturum.csv"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(';').collect();
                (f.len() > 2 && f[1] == "OTURUM_BASLA").then(|| f[2].to_string())
            })
            .collect();
        let sms = ortak::sms_count(&m.state_root, &today);
        let disk = disk_percent(&m.log_root);
        let mut uyarilar = vec![];
        let sms_down = cfg.sms.mock || !cfg.sms_missing().is_empty();
        if o.rol == Rol::Hizmet {
            if cfg.sms.mock {
                uyarilar.push("SMS deneme modunda: müşterilere gerçek SMS gitmiyor.".to_string());
            } else if !cfg.sms_missing().is_empty() {
                uyarilar.push("SMS sağlayıcı bilgileri eksik: müşteriler kod alamaz.".to_string());
            }
            if !cfg.backup.enabled {
                uyarilar.push("Uzak yedek kapalı: kayıtlar yalnızca bu cihazda.".to_string());
            }
        } else if sms_down {
            // işletme sahibi sağlayıcı adını / ayrıntıyı görmez
            uyarilar.push("Müşterilere doğrulama SMS'i şu an gönderilmiyor; hizmet sağlayıcınıza başvurun.".to_string());
        }
        if sms as f64 >= cfg.limits.sms_global_day as f64 * 0.8 {
            uyarilar.push(format!("Günlük SMS tavanına yaklaşıldı ({sms} / {}).", cfg.limits.sms_global_day));
        }
        let uyari_html = if uyarilar.is_empty() {
            String::new()
        } else {
            format!(
                "<section class=\"kutu ozet-uyari\"><h2>Uyarılar</h2><ul class=\"uyarilar\">{}</ul></section>",
                uyarilar.iter().map(|u| format!("<li>{}</li>", h(u))).collect::<String>()
            )
        };
        let kisayol: String = KISAYOLLAR.iter().map(|(yol, d, ad)| format!("<a href=\"{yol}\">{}{}</a>", simge(d), h(ad))).collect();
        // telefon ve MAC yok: yalnızca ad soyad, IP ve bağlanma saati
        let liste = |satirlar: Vec<String>, toplam: usize, bos: &str, tumu: &str| -> String {
            if satirlar.is_empty() {
                return format!("<p class=\"not\">{}</p>", h(bos));
            }
            let fazla = if toplam > OZET_SATIR { format!("<a class=\"tumu\" href=\"{tumu}\">Tümü ({toplam}) →</a>") } else { String::new() };
            format!("<ul class=\"liste\">{}</ul>{fazla}", satirlar.concat())
        };
        let bagli_html = liste(
            bagli
                .iter()
                .take(OZET_SATIR)
                .map(|s| format!("<li><b>{}</b><span>{} · {}</span></li>", h(&format!("{} {}", s.ad, s.soyad)), h(&s.ip), h(&kisa_zaman(&s.start, &today))))
                .collect(),
            bagli.len(),
            "Şu an bağlı kullanıcı yok.",
            "/cihazlar#oturumlar",
        );
        let yeni_html = liste(
            yeni.iter().take(OZET_SATIR).map(|r| format!("<li><b>{}</b><span>{}</span></li>", h(&format!("{} {}", r[1], r[2])), h(&kisa_zaman(&r[3], &today)))).collect(),
            yeni.len(),
            "Bugün yeni üye yok.",
            "/kayitlar#kullanicilar",
        );
        let sinir = cfg.limits.sms_global_day.max(1);
        let sms_p = (sms * 100 / sinir).min(100);
        let halka = |p: u64, deger: &str, ad: &str, alt: &str| {
            format!(
                "<div class=\"halka{}\"><b style=\"--p:{p}\"><span>{}</span></b>{}<small>{}</small></div>",
                if p >= 80 { " dolu" } else { "" },
                h(deger),
                h(ad),
                h(alt)
            )
        };
        let body = format!(
            "<nav class=\"kisayol\" aria-label=\"Kısayollar\">{kisayol}</nav>{uyari_html}\
             <div class=\"ozet-izgara\">\
             <section class=\"kutu\"><h2>Bağlı Kullanıcılar</h2>{bagli_html}</section>\
             <div class=\"ozet-sag\">\
             <section class=\"kutu\"><dl class=\"sayilar\"><div><dt>Bağlı kullanıcı</dt><dd>{}</dd></div>\
             <div><dt>Bugün farklı kullanıcı</dt><dd>{}</dd></div><div><dt>Bugün yeni üye</dt><dd>{}</dd></div></dl></section>\
             <section class=\"kutu\"><h2>Bugün Yeni Üyeler</h2>{yeni_html}</section>\
             </div></div>\
             <section class=\"kutu\"><h2>SMS ve Disk</h2><div class=\"halkalar\">{}{}</div></section>",
            bagli.len(),
            kisiler.len(),
            yeni.len(),
            halka(sms_p, &format!("{sms}"), "Bugün SMS", &format!("günlük sınır {}", cfg.limits.sms_global_day)),
            match disk {
                Some(p) => halka(p, &format!("%{p}"), "Kayıt diski", "dolu"),
                None => halka(0, "?", "Kayıt diski", "okunamadı"),
            },
        );
        self.page(cfg, req, Some(o), "Özet", &body)
    }

    fn oturumlar_sayfa(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        self.audit(cfg, req, Some(o), "PANEL_OTURUMLAR", "");
        let mut list: Vec<_> = ortak::load_sessions(&cfg.main.state_root).into_iter().collect();
        list.sort_by(|a, b| a.1.start_epoch.total_cmp(&b.1.start_epoch));
        let bagli = std::fs::read(self.sys.join(&cfg.main.iface).join("brforward")).map(|b| ortak::kopruye_bagli(&b)).unwrap_or_default();
        let rows: Vec<Vec<String>> = list
            .iter()
            .map(|(mac, s)| {
                let (tel, ad) = (format!("+{}", s.phone), format!("{} {}", s.ad, s.soyad));
                let left = ((s.expires_epoch - now).max(0.0) as u64) / 60;
                vec![
                    if bagli.contains(mac) { "<span class=\"rozet bagli\">Bağlı</span>".into() } else { "<span class=\"rozet\">Bağlı değil</span>".into() },
                    h(&tel),
                    h(&ad),
                    format!("<code>{}</code>", h(mac)),
                    if s.ip.is_empty() { "<span class=\"not\">beklemede</span>".into() } else { h(&s.ip) },
                    h(&s.start.get(..16).unwrap_or("").replace('T', " ")),
                    format!("{}g {}sa", left / 1440, left / 60 % 24),
                    post_button(o, "/oturumlar/at", "Bağlantıyı kes", &[("mac", mac)], "tehlike"),
                ]
            })
            .collect();
        let body = format!(
            "<p class=\"not\">Bağlantısı kesilen cihaz internete çıkamaz; yeniden SMS ile giriş yapması gerekir. \
             <b>Bağlı</b>: cihaz son 5 dakikada ağda görüldü.</p><p class=\"not\">Bu sayfadaki her görüntüleme, arama ve indirme kimin yaptığıyla birlikte kaydedilir ve hizmet sağlayıcı tarafından denetlenir.</p>{}",
            table(&["Durum", "Telefon", "Ad soyad", "MAC", "IP", "Başlangıç", "Kalan", ""], &rows, "Bağlı kullanıcı yok")
        );
        self.page(cfg, req, Some(o), "Bağlı kullanıcılar", &body)
    }

    fn at(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let Some(mac) = req.form.get("mac").and_then(|m| ortak::norm_mac(m)) else { return redirect("/oturumlar", Some(("Geçersiz MAC.", true))) };
        let m = &cfg.main;
        let Ok(_g) = ortak::state_lock(&m.state_root) else { return redirect("/oturumlar", Some(("Kilit alınamadı.", true))) };
        let mut ses = ortak::load_sessions(&m.state_root);
        let closed = ortak::close_session(&m.log_root, &mut ses, &mac, "yonetici", now, &*self.runner).is_some();
        if closed {
            let _ = ortak::save_sessions(&m.state_root, &ses);
            self.audit(cfg, req, Some(o), "PANEL_AT", &format!("mac={mac}"));
        }
        redirect("/oturumlar", Some(if closed { ("Bağlantı kesildi.", false) } else { ("Oturum bulunamadı.", true) }))
    }

    // --- yasaklı / izinli
    fn liste(&self, cfg: &Config, req: &Req, o: &Oturum, ban: bool) -> Resp {
        let (list, base, title, note) = if ban {
            (&cfg.ban, "/yasak", "Yasaklı cihazlar", "Yasaklı cihaz ağdan hiçbir şey alamaz: internet, giriş sayfası, DNS, DHCP. Bağlıysa bağlantısı hemen kesilir.")
        } else {
            (&cfg.allow, "/izinli", "İzinli cihazlar", "İzinli cihaz SMS'siz internete çıkar (AP, kasa, personel). Trafiği yine kaydedilir.")
        };
        let rows: Vec<Vec<String>> = list
            .iter()
            .map(|d| vec![format!("<code>{}</code>", h(&d.mac)), h(&d.name), post_button(o, &format!("{base}/kaldir"), "Kaldır", &[("mac", &d.mac)], "ikincil")])
            .collect();
        let body = format!(
            "<p class=\"not\">{}</p><form class=\"satir kart\" method=\"post\" action=\"{base}/ekle\">{}\
             <div><label for=\"{k}-mac\">MAC adresi</label><input type=\"text\" id=\"{k}-mac\" name=\"mac\" placeholder=\"aa:bb:cc:dd:ee:ff\" required></div>\
             <div><label for=\"{k}-ad\">Not</label><input type=\"text\" id=\"{k}-ad\" name=\"ad\" maxlength=\"40\"></div><button>Ekle</button></form>{}",
            h(note),
            csrf_input(o),
            table(&["MAC", "Not", ""], &rows, "Liste boş"),
            k = &base[1..], // yasaklı ve izinli listesi aynı sayfada: alan kimlikleri ayrı
        );
        self.page(cfg, req, Some(o), title, &body)
    }

    fn liste_ekle(&self, mut cfg: Config, req: &Req, o: &Oturum, ban: bool, now: f64) -> Resp {
        let base = if ban { "/yasak" } else { "/izinli" };
        let Some(mac) = req.form.get("mac").and_then(|m| ortak::norm_mac(m)) else { return redirect(base, Some(("Geçersiz MAC adresi.", true))) };
        let name: String = req.form.get("ad").map_or("", String::as_str).chars().filter(|c| !c.is_control()).take(40).collect();
        let (set, other) = if ban { ("ban_mac", "allow_mac") } else { ("allow_mac", "ban_mac") };
        // bir cihaz iki listede birden olmaz
        let (list, other_list) = if ban { (&mut cfg.ban, &mut cfg.allow) } else { (&mut cfg.allow, &mut cfg.ban) };
        if list.iter().any(|d| ortak::norm_mac(&d.mac).as_deref() == Some(mac.as_str())) {
            return redirect(base, Some(("Bu cihaz zaten listede.", true)));
        }
        let was_other = other_list.len();
        other_list.retain(|d| ortak::norm_mac(&d.mac).as_deref() != Some(mac.as_str()));
        if other_list.len() != was_other {
            ortak::run_opt(&*self.runner, ortak::nft_set_cmd("delete", other, &mac));
        }
        list.push(Device { mac: mac.clone(), name: name.clone() });
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect(base, Some((&e, true)));
        }
        let ok = ortak::run_opt(&*self.runner, ortak::nft_set_cmd("add", set, &mac));
        if ban {
            let m = &cfg.main;
            if let Ok(_g) = ortak::state_lock(&m.state_root) {
                let mut ses = ortak::load_sessions(&m.state_root);
                if ortak::close_session(&m.log_root, &mut ses, &mac, "yasaklandi", now, &*self.runner).is_some() {
                    let _ = ortak::save_sessions(&m.state_root, &ses);
                }
            }
        }
        self.audit(&cfg, req, Some(o), if ban { "PANEL_YASAK_EKLE" } else { "PANEL_IZIN_EKLE" }, &format!("mac={mac} not={name}"));
        redirect(base, Some(if ok { ("Eklendi.", false) } else { ("Listeye eklendi ama güvenlik duvarına yazılamadı; servisleri yeniden başlatın.", true) }))
    }

    fn liste_kaldir(&self, mut cfg: Config, req: &Req, o: &Oturum, ban: bool) -> Resp {
        let base = if ban { "/yasak" } else { "/izinli" };
        let Some(mac) = req.form.get("mac").and_then(|m| ortak::norm_mac(m)) else { return redirect(base, Some(("Geçersiz MAC adresi.", true))) };
        let list = if ban { &mut cfg.ban } else { &mut cfg.allow };
        let before = list.len();
        list.retain(|d| ortak::norm_mac(&d.mac).as_deref() != Some(mac.as_str()));
        if list.len() == before {
            return redirect(base, Some(("Listede yok.", true)));
        }
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect(base, Some((&e, true)));
        }
        ortak::run_opt(&*self.runner, ortak::nft_set_cmd("delete", if ban { "ban_mac" } else { "allow_mac" }, &mac));
        self.audit(&cfg, req, Some(o), if ban { "PANEL_YASAK_KALDIR" } else { "PANEL_IZIN_KALDIR" }, &format!("mac={mac}"));
        redirect(base, Some(("Kaldırıldı.", false)))
    }

    // --- ayarlar
    fn ayarlar(&self, cfg: &Config, req: &Req, o: &Oturum, errors: &HashMap<&str, String>, admin: bool) -> Resp {
        let form = if errors.is_empty() { None } else { Some(&req.form) };
        let mut groups: Vec<(&str, String)> = vec![];
        let bagli = crate::merkez::oku(&self.merkez_path).is_some();
        for a in fields(admin) {
            let cur = form.and_then(|f| f.get(a.key).cloned()).unwrap_or_else(|| get_field(cfg, a.key));
            let id = a.key.replace('.', "-");
            let err = errors.get(a.key).map_or(String::new(), |e| format!("<div class=\"hata\">{}</div>", h(e)));
            if let Some(neden) = salt_okunur(a, cfg, bagli) {
                let deger = if matches!(a.tur, Tur::Gizli(_)) { (if cur.is_empty() { "tanımsız" } else { "tanımlı" }).to_string() }
                            else if matches!(a.tur, Tur::Evet) { (if cur == "1" { "açık" } else { "kapalı" }).to_string() } else { cur.clone() };
                let html = format!("<p class=\"not\">{}: <b>{}</b> ({neden})</p>", h(a.label), h(&deger));
                match groups.iter_mut().find(|(g, _)| *g == a.grup) {
                    Some((_, s)) => s.push_str(&html),
                    None => groups.push((a.grup, html)),
                }
                continue;
            }
            let html = match a.tur {
                Tur::Evet => {
                    let on = if form.is_some() { req.form.get(a.key).is_some_and(|v| v == "1") } else { cur == "1" };
                    format!("<label class=\"secim\"><input type=\"checkbox\" name=\"{}\" value=\"1\"{}> {}</label>", a.key, if on { " checked" } else { "" }, h(a.label))
                }
                Tur::Secim(opts) => {
                    let o: String = opts.iter().map(|(k, l)| format!("<option value=\"{k}\"{}>{}</option>", if *k == cur { " selected" } else { "" }, h(l))).collect();
                    format!("<label for=\"{id}\">{}</label><select id=\"{id}\" name=\"{}\">{o}</select>{err}", h(a.label), a.key)
                }
                Tur::Gizli(_) => {
                    let state = if get_field(cfg, a.key).is_empty() { "tanımsız" } else { "tanımlı" };
                    format!(
                        "<label for=\"{id}\">{} <span class=\"not\">({state})</span></label><input type=\"password\" id=\"{id}\" name=\"{}\" \
                         placeholder=\"değiştirmek için yazın (boş = aynı kalır)\" autocomplete=\"new-password\">{err}",
                        h(a.label),
                        a.key
                    )
                }
                Tur::Metin(_) | Tur::Sayi(..) => {
                    let kind = if matches!(a.tur, Tur::Sayi(..)) { "number" } else { "text" };
                    format!("<label for=\"{id}\">{}</label><input type=\"{kind}\" id=\"{id}\" name=\"{}\" value=\"{}\" autocomplete=\"off\">{err}", h(a.label), a.key, h(&cur))
                }
            };
            match groups.iter_mut().find(|(g, _)| *g == a.grup) {
                Some((_, s)) => s.push_str(&html),
                None => groups.push((a.grup, html)),
            }
        }
        let mut body = format!("<form method=\"post\" action=\"{}\">{}", if admin { "/admin-ayarlari" } else { "/ayarlar" }, csrf_input(o));
        // tek grup (Ayarlar → İşletme): grup adı bölüm başlığıyla aynı, tekrar yazılmaz
        let tek = groups.len() == 1;
        for (g, inner) in &groups {
            if tek {
                body.push_str(&format!("<section class=\"kart\"><div>{inner}</div></section>"));
            } else {
                body.push_str(&format!("<section class=\"kart grup\"><h2>{}</h2><div>{inner}</div></section>", h(g)));
            }
        }
        if admin {
            let (yedek, fabrika) = self.son_durum(cfg);
            if let Some(f) = &fabrika {
                body.insert_str(0, f);
            }
            body.insert_str(0, &format!(
                "<div class=\"kart satir\"><p>Admin kilidi 30 dk açık.</p>{}</div>",
                post_button(o, "/admin-kilidi/kapat", "Kapat", &[], "ikincil")
            ));
            let st = |missing: bool| if missing { "eksik" } else { "tanımlı" };
            let twilio_missing = !cfg.twilio_missing().is_empty();
            let mut durum = vec![
                ("Mod", if cfg.sms.mock { "Deneme (SMS gönderilmiyor)".into() } else { "Gerçek SMS".into() }),
                ("NetGSM bilgileri", st(cfg.netgsm.password.is_empty() || cfg.netgsm.usercode.is_empty()).into()),
                ("Twilio (yabancı numaralar)", if cfg.twilio.enabled { st(twilio_missing).into() } else { "kapalı".into() }),
                ("Son yedek", yedek),
                ("Uzak erişim", h(&crate::uzak::durum(cfg, (self.clock)()))),
            ];
            // bağlı cihazda uzak erişim ve yedek merkez bağlanırken kurulur: elle tanıtma komutu gereksiz (alanlar salt okunur)
            if !bagli {
                durum.push((
                    "Sunucuya tanıtma komutu",
                    match (
                        crate::uzak::ensure_key(&self.uzak_key).and_then(|k| crate::uzak::public_key(&k)),
                        crate::uzak::yedek_anahtari(&self.yedek_key),
                    ) {
                        (Ok(wg), Ok(ssh)) => format!(
                            "<code style=\"user-select:all;overflow-wrap:anywhere\">{}</code><br><span class=\"not\">Merkez sunucuda bir kez \
                             çalıştırın; çıkan değerleri Uzak erişim ve Yedek hedefi alanlarına girin.</span>",
                            h(&crate::uzak::sunucu_komutu(&cfg.main.site_name, &wg, &ssh))
                        ),
                        (Err(e), _) | (_, Err(e)) => format!("<span class=\"durum kotu\">{}</span>", h(&e)),
                    },
                ));
            }
            body.push_str(&format!("<section class=\"kart grup\"><h2>Durum</h2><div>{}</div></section>", facts(&durum)));
        }
        body.push_str(if tek { "<div class=\"kaydet tek\"><button>" } else { "<div class=\"kaydet\"><button>" });
        body.push_str("Kaydet</button><p class=\"not\">Kaydedince giriş sayfası yeni ayarlarla yeniden başlatılır; bağlı müşteriler düşmez.</p></div></form>");
        if admin {
            body.push_str(&format!(
                "<section class=\"kart\" style=\"margin-top:48px\"><h2>Uzak yedek</h2><p class=\"not\">Mühürlenmiş, henüz \
                 gönderilmemiş günleri ve zincir dosyasını sunucuya şimdi gönderir (gece 02:00'de kendiliğinden de çalışır). Gönderilecek \
                 yeni gün yoksa da zincir gönderilir; sunucu bağlantısı böylece denenmiş olur. Sonuç yukarıdaki Durum'da görünür.</p>{}</section>",
                post_button(o, "/admin-ayarlari/yedekle", "Şimdi yedekle", &[], "ikincil")
            ));
            if bagli {
                // panelden bağlı cihazda fabrika yok (merkezde sahipsiz bağlı kalırdı); serbest bırakma ctl fabrika'yı çalıştırır
                body.push_str(
                    "<section class=\"kart\" style=\"margin-top:48px\"><h2>Fabrika ayarları</h2><p class=\"not\">Fabrika ayarına dönmek için \
                     cihazı yönetim merkezinden serbest bırakın; cihaz kayıtlarını gönderip kendiliğinden fabrika ayarına döner.</p></section>",
                );
            } else {
                body.push_str(&format!(
                    "<section class=\"kart\" style=\"margin-top:48px\"><h2>Fabrika ayarları</h2><p class=\"not\">Cihazı ISO'dan kurulduktan hemen \
                     sonraki haline döndürür: bütün ayarlar (işletme adı ve unvanı, SMS bilgileri, yedek, metinler, yasaklı listeler) ürün \
                     varsayılanına döner, admin dışındaki hesaplar silinir ve açılışta giriş ekranı gelir, portlar varsayılana döner \
                     (Ethernet 1 internet alır, Ethernet 2 verir, Wi-Fi kapalı), bağlı müşterilerin oturumları kapanır. 5651 kayıtları önce \
                     (bugün dahil) mühürlenip uzak sunucuya gönderilir, sonra cihazdan silinir; işletme sahibi kayıtlarını sunucudaki panelden \
                     görmeye devam eder. Uzak yedek kapalıysa ya da bir gün gönderilemezse işlem iptal olur ve hiçbir şey silinmez. \
                     Geri alınamaz.</p><form class=\"satir\" method=\"post\" action=\"/admin-ayarlari/fabrika\">{}\
                     <div><label for=\"fp\">Admin parolası</label><input type=\"password\" id=\"fp\" name=\"parola\" autocomplete=\"current-password\" required></div>\
                     <label class=\"secim\"><input type=\"checkbox\" name=\"onay\" value=\"1\" required> Bütün ayarların silineceğini anladım</label>\
                     <button class=\"tehlike\">Fabrika ayarlarına döndür</button></form></section>",
                    csrf_input(o)
                ));
            }
        }
        self.page(cfg, req, Some(o), if admin { "Admin ayarları" } else { "Ayarlar" }, &body)
    }

    /// Admin ayarları durumu (denetim kaydından): son yedek sonucu ve son fabrika dönüşü (sürüyor / iptal oldu).
    fn son_durum(&self, cfg: &Config) -> (String, Option<String>) {
        let root = &cfg.main.log_root;
        let mut rows: Vec<crate::kayit::Rec> = vec![];
        for day in crate::kayit::days_desc(root).into_iter().take(3) {
            rows.extend(crate::kayit::day_rows(root, &day, "denetim.csv"));
        }
        rows.sort_by(|a, b| a[0].cmp(&b[0]));
        let when = |r: &crate::kayit::Rec| r[0].get(..16).unwrap_or("").replace('T', " ");
        let ek = |r: &crate::kayit::Rec| crate::kayit::col(r, "ek").to_string();
        let yedek = match rows.iter().rev().find(|r| crate::kayit::col(r, "olay") == "YEDEK_SONUC") {
            None => "henüz çalışmadı".to_string(),
            Some(r) => match ek(r).strip_prefix("hata=") {
                Some(e) => format!("<span class=\"durum kotu\">{} — HATA: {}</span>", h(&when(r)), h(e)),
                None => format!("<span class=\"durum\">{} — {}</span>", h(&when(r)), h(ek(r).trim_start_matches("sonuc="))),
            },
        };
        let last = rows.iter().rev().find(|r| matches!(crate::kayit::col(r, "olay"), "FABRIKA_IPTAL" | "FABRIKA_AYARI" | "PANEL_FABRIKA"));
        let fabrika = last.and_then(|r| match crate::kayit::col(r, "olay") {
            "FABRIKA_IPTAL" => Some(format!(
                "<div class=\"mesaj hata\">Son fabrika dönüşü <b>iptal oldu</b> ({}): {}. Hiçbir kayıt silinmedi, ayarlar yerinde.</div>",
                h(&when(r)),
                h(ek(r).trim_start_matches("neden="))
            )),
            "PANEL_FABRIKA" => Some(format!(
                "<div class=\"mesaj\">Fabrika dönüşü {} tarihinde başlatıldı; kayıtlar sunucuya gönderiliyor. Birkaç dakika sonra sayfayı yenileyin.</div>",
                h(&when(r))
            )),
            _ => None,
        });
        (yedek, fabrika)
    }

    fn yedekle(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        if !cfg.backup.enabled || cfg.backup.target.trim().is_empty() {
            return redirect("/admin-ayarlari", Some(("Uzak yedek kapalı ya da hedef boş; önce yukarıdan açıp kaydedin.", true)));
        }
        self.audit(cfg, req, Some(o), "PANEL_YEDEKLE", "");
        let unit = format!("wfc-yedekle-{}", ortak::random_hex(4));
        let ok = (self.runner)(&cmd(&["systemd-run", "--unit", &unit, "--on-active", "1s", "/usr/local/bin/wificorrect", "ctl", "yedekle"]));
        redirect("/admin-ayarlari", Some(if ok { ("Yedekleme başladı; birkaç saniye sonra sayfayı yenileyip Durum'daki \"Son yedek\" satırına bakın.", false) } else { ("Yedekleme başlatılamadı.", true) }))
    }

    fn fabrika(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let pw = req.form.get("parola").map_or("", String::as_str);
        if req.form.get("onay").is_none_or(|v| v != "1") || self.hesaplar.verify(hesap::ADMIN, pw) != Some(Rol::Hizmet) {
            self.audit(cfg, req, Some(o), "PANEL_FABRIKA_RED", "");
            return redirect("/admin-ayarlari", Some(("Parola hatalı ya da onay işaretlenmedi; hiçbir şey değişmedi.", true)));
        }
        if let Some(e) = crate::fabrika::engel(cfg) {
            return redirect("/admin-ayarlari", Some((e, true)));
        }
        if crate::merkez::oku(&self.merkez_path).is_some() {
            return redirect("/admin-ayarlari", Some((
                "Cihaz bir müşteriye bağlı. Fabrika ayarına dönmek için önce yönetim merkezinden cihazı serbest bırakın; cihaz kayıtlarını teslim edip kendini temizler.",
                true,
            )));
        }
        self.audit(cfg, req, Some(o), "PANEL_FABRIKA", "");
        // yanıt tarayıcıya ulaşsın diye 2 sn sonra, panelden bağımsız işte (panel ve ağ yeniden başlar)
        let unit = format!("wfc-fabrika-{}", ortak::random_hex(4));
        if !(self.runner)(&cmd(&["systemd-run", "--unit", &unit, "--on-active", "2s", "/usr/local/bin/wificorrect", "ctl", "fabrika"])) {
            return redirect("/admin-ayarlari", Some(("Fabrika ayarları başlatılamadı.", true)));
        }
        let body = "<div class=\"kart\"><p>Cihaz fabrika ayarlarına dönüyor: önce kayıtlar sunucuya gönderiliyor (birkaç dakika \
                    sürebilir), sonra ağ ve panel yeniden başlıyor. Biraz sonra bu adresi yenileyin; giriş ekranı gelip menüde işletme adı yerine WifiCorrect yazıyorsa işlem tamamdır. \
                    Panel açılıp ayarlar yerindeyse kayıtlar gönderilemediği için işlem iptal olmuştur (Panel hareketleri / denetim: \
                    FABRIKA_IPTAL). İnternet kablosu Ethernet 1'de (sağ) değilse oraya takın.</p></div>";
        self.page(cfg, req, None, "Fabrika ayarları", body)
    }

    fn ayarlar_post(&self, mut cfg: Config, req: &Req, o: &Oturum, admin: bool) -> Resp {
        let back = if admin { "/admin-ayarlari" } else { "/ayarlar" };
        let changes = match validate(admin, &req.form, &cfg, crate::merkez::oku(&self.merkez_path).is_some()) {
            Ok(c) => c,
            Err(errors) => {
                let req2 = Req { query: [("m".to_string(), "Geçersiz değerler var, düzeltip tekrar kaydedin.".to_string()), ("e".to_string(), "1".to_string())].into(), ..clone_req(req) };
                return self.ayarlar(&cfg, &req2, o, &errors, admin);
            }
        };
        if changes.is_empty() {
            return redirect(back, Some(("Değişiklik yok.", false)));
        }
        for (k, _, new) in &changes {
            set_field(&mut cfg, k, new);
        }
        if !cfg.sms.mock {
            let eksik = cfg.sms_missing();
            if !eksik.is_empty() {
                // portal eksik ayarla açılmaz; misafirler giriş sayfasını hiç göremezdi
                return redirect(back, Some((&format!("SMS deneme modunu kapatmak için şu alanlar gerekli: {}; hiçbir şey kaydedilmedi.", eksik.join(", ")), true)));
            }
        }
        if cfg.uzak.enabled {
            if let Some(e) = crate::uzak::eksik(&cfg) {
                return redirect(back, Some((&format!("Uzak erişimi açmak için {e} gerekli; hiçbir şey kaydedilmedi."), true)));
            }
        }
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect(back, Some((&e, true)));
        }
        for (k, old, new) in &changes {
            self.audit(&cfg, req, Some(o), "PANEL_AYAR", &describe(k, old, new));
        }
        if changes.iter().any(|(k, _, _)| k.starts_with("uzak.")) {
            // tünel ayrı işte kurulur (/etc/wireguard panelden yazılamaz; sunucu adı çözülürken panel beklemesin)
            let unit = format!("wfc-uzak-{}", ortak::random_hex(4));
            (self.runner)(&cmd(&["systemd-run", "--unit", &unit, "--on-active", "1s", "/usr/local/bin/wificorrect", "ctl", "uzak-uygula"]));
        }
        let ok = (self.runner)(&cmd(&["systemctl", "restart", "wificorrect-portal"]));
        redirect(back, Some(if ok { ("Ayarlar kaydedildi. Giriş sayfası yeniden başlatıldı.", false) } else { ("Ayarlar kaydedildi ama giriş sayfası yeniden başlatılamadı!", true) }))
    }

    // --- sistem
    fn sistem(&self, cfg: &Config, req: &Req, o: &Oturum, verify_out: &str) -> Resp {
        let chain = crate::muhur::chain_lines(&cfg.main.log_root);
        let mut rows: Vec<(&str, String)> = vec![
            ("Güvenlik duvarı", service_state("wificorrect-guvenlik")),
            ("DHCP / DNS", service_state("dnsmasq")),
            ("Giriş sayfası", service_state("wificorrect-portal")),
            ("Kaydedici", service_state("wificorrect-kaydedici")),
            ("Son mühürlenen gün", h(&chain.last().map_or("henüz yok".to_string(), |l| l[0].clone()))),
            ("Saat", h(&ortak::now_iso((self.clock)()))),
            ("Sürüm", h(&format!("wificorrect {}", env!("CARGO_PKG_VERSION")))),
        ];
        if o.rol == Rol::Hizmet {
            rows.insert(5, ("Uzak yedek", if cfg.backup.enabled { h(&cfg.backup.target) } else { "kapalı".into() }));
        }
        let verify = if verify_out.is_empty() { String::new() } else { format!("<div class=\"kart\"><h2>Bütünlük doğrulaması</h2><pre>{}</pre></div>", h(verify_out)) };
        let body = format!(
            "<div class=\"kart\">{}</div>{verify}<div class=\"kart\"><h2>İşlemler</h2><div class=\"eylemler\">{}{}{}</div>\
             <p class=\"not\">Günler her gece 00:15'te kendiliğinden mühürlenir; buradan beklemeden çalıştırılabilir.</p></div>",
            facts(&rows),
            post_button(o, "/sistem/gun-kapat", "Günü kapat (mühürle)", &[], ""),
            post_button(o, "/sistem/dogrula", "Bütünlüğü doğrula", &[], "ikincil"),
            post_button(o, "/sistem/yeniden-baslat", "Servisleri yeniden başlat", &[], "ikincil"),
        );
        self.page(cfg, req, Some(o), "Sistem", &body)
    }

    // --- parola ve hesaplar
    fn sifre(&self, cfg: &Config, req: &Req, o: &Oturum, err: &str) -> Resp {
        let err = if err.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", h(err)) };
        let body = format!(
            "<div class=\"kart dar\">{err}<form method=\"post\" action=\"/sifre\">{}\
             <label for=\"a\">Şu anki parola</label><input type=\"password\" id=\"a\" name=\"eski\" autocomplete=\"current-password\" required>\
             <label for=\"b\">Yeni parola (en az 10 karakter)</label><input type=\"password\" id=\"b\" name=\"yeni\" autocomplete=\"new-password\" required>\
             <label for=\"c\">Yeni parola (tekrar)</label><input type=\"password\" id=\"c\" name=\"yeni2\" autocomplete=\"new-password\" required>\
             <div style=\"margin-top:20px\"><button>Değiştir</button></div></form></div>",
            csrf_input(o)
        );
        self.page(cfg, req, Some(o), "Şifremi değiştir", &body)
    }

    fn sifre_post(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let g = |k: &str| req.form.get(k).cloned().unwrap_or_default();
        // müşteri parolası merkezde (admin kilidi açık olsa da; admin parolası yalnızca merkezden değişir): değişiklik merkeze
        // gider, yeni özet cihaza yazılır (internetsiz değişmez)
        let Some(mut m) = crate::merkez::oku(&self.merkez_path) else { return redirect("/giris", None) };
        if !crate::merkez::dogrula(&m, &g("eski")) {
            return self.sifre(cfg, req, o, "Şu anki parola hatalı.");
        }
        if let Some(e) = hesap::password_problem(&g("yeni")) {
            return self.sifre(cfg, req, o, e);
        }
        if g("yeni") != g("yeni2") {
            return self.sifre(cfg, req, o, "Yeni parolalar aynı değil.");
        }
        if let Err(e) = crate::merkez::parola(&mut m, &g("eski"), &g("yeni"), &*self.http) {
            return self.sifre(cfg, req, o, &e);
        }
        if let Err(e) = crate::merkez::kaydet(&self.merkez_path, &m) {
            return self.sifre(cfg, req, o, &e);
        }
        self.oturumlar.remove_user(&o.user, None);
        let token = self.oturumlar.create(&o.user, Rol::Sahip, (self.clock)(), &m.ozet);
        self.audit(cfg, req, Some(o), "PANEL_SIFRE", "merkez");
        let mut r = redirect("/sifre", Some(("Parola değiştirildi.", false)));
        r.headers.push(("Set-Cookie".into(), format!("wfc={token}; Path=/; Secure; HttpOnly; SameSite=Strict")));
        r
    }
}

/// Admin kilidi açık kalma süresi (sn)
const ADMIN_SURE: f64 = 1800.0;

/// Admin kilidi açılınca dönülecek yer: yalnızca kilitli sayfalar (başka siteye yönlendirme yok).
fn admin_donus(yol: &str) -> &str {
    match yol {
        "/admin-ayarlari" | "/panel-hareketleri" => yol,
        _ => "/admin",
    }
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn clone_req(r: &Req) -> Req {
    Req {
        method: r.method.clone(),
        path: r.path.clone(),
        query: r.query.clone(),
        form: r.form.clone(),
        ip: r.ip.clone(),
        token: r.token.clone(),
        merkez_kullanici: r.merkez_kullanici.clone(),
        ileten_ip: r.ileten_ip.clone(),
    }
}

// ---------------------------------------------------------------- HTTPS
/// Öz-imzalı sertifika ilk açılışta cihazda üretilir (kalıba girmez).
fn ensure_cert() -> Result<(Vec<u8>, Vec<u8>), String> {
    if !std::path::Path::new(CERT).exists() || !std::path::Path::new(KEY).exists() {
        let host = ortak::capture(&["hostname"]).trim().to_string();
        let ok = ortak::run_timeout(
            &cmd(&[
                "openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "3650",
                "-subj", &format!("/CN={}", if host.is_empty() { "wificorrect" } else { &host }), "-keyout", KEY, "-out", CERT,
            ]),
            60,
        );
        if !ok {
            return Err("panel sertifikası üretilemedi (openssl)".into());
        }
        let _ = std::fs::set_permissions(KEY, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    }
    Ok((std::fs::read(CERT).map_err(|e| e.to_string())?, std::fs::read(KEY).map_err(|e| e.to_string())?))
}

fn respond(req: tiny_http::Request, mut r: Resp) {
    match r.file.take() {
        Some(f) => send(req, tiny_http::Response::from_file(f).with_status_code(r.status), r.headers),
        None => send(req, tiny_http::Response::from_string(r.body).with_status_code(r.status), r.headers),
    }
}

fn send<R: Read>(req: tiny_http::Request, mut resp: tiny_http::Response<R>, headers: Vec<(String, String)>) {
    let base = [
        ("Cache-Control", "no-store"),
        ("X-Frame-Options", "DENY"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Content-Type-Options", "nosniff"),
        ("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action 'self'; frame-ancestors 'none'"),
        ("Server", "wificorrect"),
    ];
    for (k, v) in base.iter().map(|(k, v)| (k.to_string(), v.to_string())).chain(headers) {
        if let Ok(hd) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(hd);
        }
    }
    let _ = req.respond(resp);
}

fn handle(p: &Panel, mut req: tiny_http::Request) {
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let token = req
        .headers()
        .iter()
        .find(|hd| hd.field.equiv("Cookie"))
        .and_then(|hd| hd.value.as_str().split(';').find_map(|c| c.trim().strip_prefix("wfc=").map(String::from)))
        .filter(|t| !t.is_empty());
    let baslik = |ad: &'static str| req.headers().iter().find(|hd| hd.field.equiv(ad)).map(|hd| hd.value.as_str().to_string());
    let merkez_kullanici = baslik("X-WFC-Kullanici").map(|v| v.trim().to_string());
    let ileten_ip = baslik("X-Forwarded-For").and_then(|v| v.rsplit(',').next().map(|s| s.trim().to_string())).filter(|s| !s.is_empty());
    let method = req.method().as_str().to_uppercase();
    let mut form = Form::new();
    if method == "POST" {
        if req.body_length().unwrap_or(0) > 16_384 {
            return respond(req, text(413, "İstek çok büyük."));
        }
        let mut raw = String::new();
        if req.as_reader().take(16_384).read_to_string(&mut raw).is_err() {
            return respond(req, text(400, "Geçersiz istek."));
        }
        form = parse_query(&raw);
    }
    let r = p.handle(&Req { method, path: path.to_string(), query: parse_query(query), form, ip, token, merkez_kullanici, ileten_ip });
    respond(req, r);
}

pub fn run(cfg_path: &str) -> ExitCode {
    let (cert, key) = match ensure_cert() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("panel: {e}");
            return ExitCode::from(1);
        }
    };
    let server = match tiny_http::Server::https("0.0.0.0:8443", tiny_http::SslConfig { certificate: cert, private_key: key }) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("panel: 8443 dinlenemedi: {e}");
            return ExitCode::from(1);
        }
    };
    let panel = Arc::new(Panel::new(cfg_path, hesap::PATH, Box::new(|c: &[String]| ortak::run(c)), Box::new(ortak::wall)));
    eprintln!("panel: https://0.0.0.0:8443 dinleniyor{}", if crate::merkez::oku(std::path::Path::new(crate::merkez::PATH)).is_none() { " (müşteri girişi bekleniyor)" } else { "" });
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (server, panel) = (server.clone(), panel.clone());
            std::thread::spawn(move || loop {
                match server.recv() {
                    Ok(req) => {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&panel, req))).is_err() {
                            eprintln!("panel: istek işlenirken beklenmeyen hata");
                        }
                    }
                    Err(e) => {
                        eprintln!("panel: bağlantı hatası: {e}");
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct Env {
        p: Panel,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        root: std::path::PathBuf,
    }

    const IP: &str = "192.168.1.50";

    fn env() -> Env {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-panel-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = format!("[main]\nsite_name = 'Bocafe'\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n[uzak]\nenabled = true\nsunucu = 'vpn.wificorrect.com:51820'\nsunucu_anahtar = '{UZAK_KEY}'\nadres = '10.99.0.11'\n", root.display());
        std::fs::write(root.join("ayarlar.toml"), cfg).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let c2 = calls.clone();
        let p = Panel::new(
            &root.join("ayarlar.toml").to_string_lossy(),
            &root.join("hesaplar.json").to_string_lossy(),
            Box::new(move |c: &[String]| {
                c2.lock().unwrap().push(c.to_vec());
                true
            }),
            Box::new(|| 1_790_705_134.0),
        );
        let mut p = p;
        p.uzak_key = root.join("wg.key"); // testler gerçek anahtarlara dokunmasın
        p.yedek_key = root.join("yedek_anahtar");
        p.filtre_conf = root.join("yasak.conf");
        p.merkez_path = root.join("merkez.json");
        p.http = merkez_yok();
        p.sys = root.join("sys"); // testler gerçek arayüzlere bakmasın; "wfc" = tünel arayüzü var
        std::fs::create_dir_all(p.sys.join("wfc")).unwrap();
        Env { p, calls, root }
    }

    fn req(method: &str, path: &str, form: &[(&str, &str)], token: Option<&str>) -> Req {
        Req {
            method: method.into(),
            path: path.into(),
            query: Form::new(),
            form: form.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            // bağlı cihazda yerel istek reddedilir: testler varsayılan olarak merkezden (tünel) gelir
            ip: "10.99.0.1".into(),
            token: token.map(String::from),
            merkez_kullanici: Some(MUSTERI.into()),
            ileten_ip: Some(IP.into()),
        }
    }

    /// Merkezden ama kimlik başlığı olmadan (çerez geçerliliğini sınamak için)
    fn basliksiz(mut r: Req) -> Req {
        r.merkez_kullanici = None;
        r
    }

    fn merkez_req(path: &str, kullanici: &str) -> Req {
        Req { method: "GET".into(), path: path.into(), ip: "10.99.0.1".into(), merkez_kullanici: Some(kullanici.into()),
              ileten_ip: Some("203.0.113.9".into()), ..Default::default() }
    }

    const UZAK_KEY: &str = "aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789+/abcdE=";

    /// Tünel açık: ayar (açık, eksiksiz, adres 10.99.0.11) ve "wfc" arayüzü
    fn uzak_adresli(e: &Env) {
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.uzak.enabled = true;
        c.uzak.sunucu = "vpn.wificorrect.com:51820".into();
        c.uzak.sunucu_anahtar = UZAK_KEY.into();
        c.uzak.adres = "10.99.0.11".into();
        c.save(&e.p.cfg_path).unwrap();
        std::fs::create_dir_all(e.p.sys.join("wfc")).unwrap();
    }

    fn loc(r: &Resp) -> &str {
        r.headers.iter().find(|(k, _)| k == "Location").map_or("", |(_, v)| v.as_str())
    }

    const MUSTERI: &str = "4511643";

    /// Sahte merkez: yalnızca bağ testlerinde çağrılır; varsayılan "ulaşılamadı".
    fn merkez_yok() -> Box<crate::merkez::Http> {
        Box::new(|_: &str, _: &str| Err("ağ yok".into()))
    }

    /// Cihaz müşteriye bağlı (merkez.json) ve admin parolası kurulu; merkez başlığıyla (müşteri numarası) oturum açılır.
    /// `user` "admin" ise ardından admin kilidi `pw` ile açılır (etkin rol hizmet sağlayıcı).
    fn setup_and_login(e: &Env, user: &str, pw: &str) -> (String, String) {
        if crate::merkez::oku(&e.p.merkez_path).is_none() {
            e.p.hesaplar.set_admin("hizmet-parola-1").unwrap();
            let tuz = "0011aabb".to_string();
            let m = crate::merkez::Merkez { numara: MUSTERI.into(), ozet: crate::hesap::digest("sahip-parola-12", &tuz, 120_000), tuz,
                                            yineleme: 120_000, cihaz_anahtari: "k".into(), son_eslesme: 1_790_705_134.0, deneme: 0.0, ..Default::default() };
            crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
            let mut c = Config::load(&e.p.cfg_path).unwrap();
            (c.main.site_name, c.main.unvan) = ("Bocafe Göztepe".into(), "Boca Gıda Tic. Ltd. Şti.".into());
            c.save(&e.p.cfg_path).unwrap();
        }
        uzak_adresli(e);
        let r = e.p.handle(&merkez_req("/", MUSTERI));
        let cookie = r.headers.iter().find(|(k, _)| k == "Set-Cookie").map(|(_, v)| v.clone()).expect("çerez");
        let token = cookie.trim_start_matches("wfc=").split(';').next().unwrap().to_string();
        let page = e.p.handle(&req("GET", "/sifre", &[], Some(&token))).body;
        let csrf = page.split("name=\"csrf\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        if user == "admin" {
            let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", pw), ("donus", "/admin")], Some(&token)));
            assert_eq!(loc(&r), "/admin", "admin kilidi açılmadı");
        }
        (token, csrf)
    }

    #[test]
    fn baslik_yalniz_merkezden() {
        let e = env();
        setup_and_login(&e, "mudur", "sahip-parola-12"); // cihazı MUSTERI'ye bağlar
        uzak_adresli(&e);
        let r = e.p.handle(&merkez_req("/cihazlar", MUSTERI));
        assert_eq!(r.status, 200, "{}", r.body);
        let cerez = r.headers.iter().find(|(k, _)| k == "Set-Cookie").expect("oturum çerezi").1.clone();
        assert!(cerez.starts_with("wfc="));
        // aynı başlık modem ağından: yok sayılır
        let mut r2 = merkez_req("/cihazlar", MUSTERI);
        r2.ip = "192.168.1.50".into();
        let y = e.p.handle(&r2);
        assert!(y.status == 404 && y.body.is_empty()); // yerel erişim: hiçbir şey
        // başka müşteri numarası: red
        assert_ne!(e.p.handle(&merkez_req("/cihazlar", "7654321")).status, 200);
        // denetimde gerçek istemci IP'si
        let gun = ortak::day_of(&ortak::now_iso((e.p.clock)())).to_string();
        let root = Config::load(&e.p.cfg_path).unwrap().main.log_root;
        assert!(std::fs::read_to_string(ortak::day_file(&root, &gun, "denetim.csv")).unwrap().contains("ip=203.0.113.9"));
    }

    #[test]
    fn yerel_erisim_bagliyken_kapali() {
        let e = env();
        // bağlı değil: yerelden yalnızca eşleştirme (giriş) ekranı
        let g = e.p.handle(&req("GET", "/giris", &[], None)).body;
        assert!(g.contains("action=\"/giris\"") && g.contains("Müşteri numarası"));
        assert_eq!(loc(&e.p.handle(&req("GET", "/cihazlar", &[], None))), "/giris");
        setup_and_login(&e, "mudur", "sahip-parola-12");
        uzak_adresli(&e);
        // bağlı: giriş formu da not da yok (2026-10-10: tamamen erişimsiz)
        let g = e.p.handle(&req("GET", "/giris", &[], None));
        assert!(g.status == 404 && g.body.is_empty());
        assert_ne!(e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "sahip-parola-12")], None)).status, 303);
    }

    #[test]
    fn merkezden_cikis_merkeze_yonlenir_ve_arsiv_baglantisi() {
        let e = env();
        setup_and_login(&e, "mudur", "sahip-parola-12");
        uzak_adresli(&e);
        let r = e.p.handle(&merkez_req("/kayitlar", MUSTERI));
        assert!(r.body.contains("https://panel.wificorrect.com/") && r.body.contains("yedek arşiv"));
        let tok = r.headers.iter().find(|(k, _)| k == "Set-Cookie").unwrap().1.trim_start_matches("wfc=").split(';').next().unwrap().to_string();
        let csrf = r.body.split("name=\"csrf\" value=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
        let mut c = merkez_req("/cikis", MUSTERI);
        (c.method, c.token, c.form) = ("POST".into(), Some(tok), [("csrf".to_string(), csrf)].into_iter().collect());
        assert_eq!(loc(&e.p.handle(&c)), "/_cikis");
    }

    /// Tünel kapalıyken (ayar ya da arayüz yok) 10.99.0.1 kaynağı dükkân ağından taklit edilebilir: başlığa güvenilmez.
    #[test]
    fn tunel_yokken_merkez_basligi_yok_sayilir() {
        let e = env();
        setup_and_login(&e, "mudur", "sahip-parola-12");
        let reddedildi = |r: Resp| {
            r.status == 404 && r.body.is_empty() && !r.headers.iter().any(|(k, _)| k == "Set-Cookie")
        };
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.uzak.enabled = false;
        c.save(&e.p.cfg_path).unwrap();
        assert!(reddedildi(e.p.handle(&merkez_req("/cihazlar", MUSTERI))));
        assert!(reddedildi(e.p.handle(&merkez_req("/cihazlar", "7654321")))); // numara sorgulanamaz
        uzak_adresli(&e);
        std::fs::remove_dir(e.p.sys.join("wfc")).unwrap(); // wg-quick başlamadı
        assert!(reddedildi(e.p.handle(&merkez_req("/cihazlar", MUSTERI))));
        assert!(reddedildi(e.p.handle(&merkez_req("/cihazlar", "7654321"))));
        uzak_adresli(&e);
        assert_eq!(e.p.handle(&merkez_req("/cihazlar", MUSTERI)).status, 200);
    }

    #[test]
    fn bagliyken_yerel_kaynak_reddedilir() {
        let e = env();
        let yerel = |mut r: Req| {
            (r.ip, r.merkez_kullanici, r.ileten_ip) = (IP.into(), None, None);
            r
        };
        let denetim = || std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        // bağlı değil: yerel kaynakta X-Forwarded-For denetime girmez
        let mut r = yerel(req("POST", "/giris", &[("kullanici", "admin"), ("parola", "yanlis-parola")], None));
        r.ileten_ip = Some("198.51.100.7".into());
        e.p.handle(&r);
        assert!(denetim().contains(&format!("ip={IP}")) && !denetim().contains("198.51.100.7"));
        let (atok, _) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let (mtok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        // (a) doğru parolalı yerel admin girişi: oturum yok
        let r = e.p.handle(&yerel(req("POST", "/giris", &[("kullanici", "admin"), ("parola", "hizmet-parola-1")], None)));
        assert!(r.status != 303 && !r.headers.iter().any(|(k, _)| k == "Set-Cookie"));
        // (b) daha önce verilmiş geçerli çerez yerelden işe yaramaz
        for t in [&atok, &mtok] {
            let r = e.p.handle(&yerel(req("GET", "/cihazlar", &[], Some(t))));
            assert!(r.status == 404 && r.body.is_empty());
        }
        // (c) yerel istekteki X-Forwarded-For hiçbir yere yazılmaz
        let mut r = yerel(req("POST", "/oturumlar/at", &[], Some(&mtok)));
        r.ileten_ip = Some("198.51.100.7".into());
        e.p.handle(&r);
        assert!(!denetim().contains("198.51.100.7"));
        // merkezden de IP olmayan X-Forwarded-For kabul edilmez: kaynak adres yazılır
        let mut r = merkez_req("/cihazlar", MUSTERI);
        r.ileten_ip = Some("kotu-deger".into());
        e.p.handle(&r);
        assert!(!denetim().contains("kotu-deger") && denetim().contains("ip=10.99.0.1"));
    }

    /// Sayfadaki id'ler: aynı id iki kez olursa etiketler yanlış alana bağlanır.
    fn tekrar_eden_idler(html: &str) -> Vec<String> {
        let mut gorulen = BTreeSet::new();
        html.split(" id=\"").skip(1).filter_map(|s| s.split('"').next()).filter(|id| !gorulen.insert(id.to_string())).map(String::from).collect()
    }

    fn menu_linkleri(html: &str) -> Vec<String> {
        let nav = html.split("<nav aria-label=\"Menü\">").nth(1).unwrap().split("</nav>").next().unwrap();
        nav.split("href=\"").skip(1).map(|s| s.split('"').next().unwrap().to_string()).collect()
    }

    #[test]
    fn menu_dort_grup_ve_bolumler_tek_sayfada() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ozet = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert_eq!(menu_linkleri(&ozet), ["/", "/cihazlar", "/kayitlar", "/ayarlar", "/admin"]);
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok)));
        assert_eq!(s.status, 200);
        for (id, form) in [("oturumlar", ""), ("yasak", "/yasak/ekle"), ("izinli", "/izinli/ekle"), ("yasakli-siteler", "/yasakli-siteler/ekle")] {
            assert!(s.body.contains(&format!("<section class=\"bolum\" id=\"{id}\"")), "{id}");
            assert!(s.body.contains(&format!("href=\"#{id}\"")), "atlama bağlantısı {id}");
            assert!(s.body.contains(form), "{form}");
        }
        assert!(s.body.contains("href=\"/cihazlar\" class=\"aktif\""));
        assert!(tekrar_eden_idler(&s.body).is_empty(), "{:?}", tekrar_eden_idler(&s.body));
        // kayıtlar: işletme sahibi panel hareketlerini görmez; kullanıcılar gerekçe ister (gerekçesiz içerik yok)
        let k = e.p.handle(&req("GET", "/kayitlar", &[], Some(&tok))).body;
        for id in ["gunler", "kullanicilar", "talep"] {
            assert!(k.contains(&format!("id=\"{id}\"")), "{id}");
        }
        assert!(!k.contains("id=\"panel-hareketleri\"") && k.contains("href=\"/kullanicilar\""));
        assert!(tekrar_eden_idler(&k).is_empty(), "{:?}", tekrar_eden_idler(&k));
        // ayarlar: şifre değiştirme de burada; admin ayarları yalnızca admin'e
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        for id in ["isletme", "portal-metinleri", "portlar", "sistem", "sifre-degistir"] {
            assert!(a.contains(&format!("<section class=\"bolum\" id=\"{id}\"")), "{id}");
        }
        assert!(!a.contains("id=\"admin-ayarlari\"") && a.contains("action=\"/sifre\""));
        assert!(tekrar_eden_idler(&a).is_empty(), "{:?}", tekrar_eden_idler(&a));
        // bölüm sayfasından ayrı açılan ayrıntı sayfası kendi grubunu seçili gösterir
        let gun = e.p.handle(&req("GET", "/kayitlar/gun", &[], Some(&tok))).body;
        assert!(gun.contains("href=\"/kayitlar\" class=\"aktif\"") || gun.contains("gerekçe"));
    }

    #[test]
    fn site_ip_arama_kimin_girdigini_gosterir() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let root = Config::load(&e.p.cfg_path).unwrap().main.log_root;
        let gun = ortak::day_of(&ortak::now_iso((e.p.clock)())).to_string();
        ortak::append_rows(&ortak::day_file(&root, &gun, "dns.csv"), &[Row::new("DNS", &format!("{gun}T10:00:00+03:00"))
            .set("telefon", "905334553132").set("mac", "aa:bb:cc:dd:ee:01").set("ic_ip", "10.50.0.23").set("alan_adi", "www.bet365.com")]).unwrap();
        std::fs::create_dir_all(std::path::Path::new(&root).join("kullanicilar")).unwrap();
        std::fs::write(std::path::Path::new(&root).join("kullanicilar/index.csv"), "telefon;ad;soyad;ilk_kayit;son_oturum\n905334553132;Ayşe;Yılmaz;x;y\n").unwrap();
        // Kayıtlar sayfasında arama formu; varsayılan aralık son 7 gün
        let k = e.p.handle(&req("GET", "/kayitlar", &[], Some(&tok))).body;
        assert!(k.contains("<section class=\"bolum\" id=\"site-ara\"") && k.contains("action=\"/ara\"") && k.contains(&format!("value=\"{gun}\"")));
        // kişisel veri: işletme sahibi önce gerekçe yazar
        let mut r = req("GET", "/ara", &[], Some(&tok));
        r.query.insert("q".into(), "bet365.com".into());
        let sonuc = e.p.handle(&r).body;
        assert!(sonuc.contains("action=\"/gerekce\"") && !sonuc.contains("Ayşe Yılmaz"));
        let (tok, _) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let mut r = req("GET", "/ara", &[], Some(&tok));
        r.query.insert("q".into(), "https://bet365.com/spor".into());
        let sonuc = e.p.handle(&r);
        assert!(sonuc.body.contains("Ayşe Yılmaz") && sonuc.body.contains("+905334553132") && sonuc.body.contains("www.bet365.com"), "{}", sonuc.body);
        assert!(sonuc.body.contains("href=\"/kayitlar\" class=\"aktif\""));
        let denetim = std::fs::read_to_string(ortak::day_file(&root, &gun, "denetim.csv")).unwrap();
        assert!(denetim.contains("PANEL_SITE_ARA") && denetim.contains("aranan=https://bet365.com/spor"));
    }

    #[test]
    fn o_an_bagli_cihaz_yesil_rozetle() {
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let mut ses = ortak::Sessions::new();
        for (mac, ad) in [("aa:bb:cc:dd:ee:01", "Ayşe"), ("aa:bb:cc:dd:ee:02", "Mehmet")] {
            ses.insert(mac.into(), ortak::Session {
                phone: "905334553132".into(), ad: ad.into(), soyad: "Y".into(), ip: "10.50.0.23".into(),
                session_id: "s".into(), start: "2026-09-29T20:00:00+03:00".into(), start_epoch: 1.0, expires_epoch: 9e9,
            });
        }
        ortak::save_sessions(&cfg.main.state_root, &ses).unwrap();
        // köprü iletim tablosu: yalnızca ...:01 son 5 dakikada görüldü; köprünün kendi (yerel) MAC'i sayılmaz
        let sys = e.root.join("sys-bagli");
        std::fs::create_dir_all(sys.join(&cfg.main.iface)).unwrap();
        let mut fdb = vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x01, 1, 0, 0x3d, 0x21, 0, 0, 0, 0, 0, 0];
        fdb.extend([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x02, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(sys.join(&cfg.main.iface).join("brforward"), fdb).unwrap();
        std::fs::create_dir_all(sys.join("wfc")).unwrap();
        e.p.sys = sys;
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        let satir = |ad: &str| s.split("<tr>").find(|r| r.contains(ad)).unwrap().to_string();
        assert!(satir("Ayşe").contains("<span class=\"rozet bagli\">Bağlı</span>"));
        assert!(!satir("Mehmet").contains("rozet bagli") && satir("Mehmet").contains("Bağlı değil"));
        assert_eq!(ortak::kopruye_bagli(&[1, 2, 3]), BTreeSet::new()); // yarım kayıt yok sayılır
    }

    #[test]
    fn ozet_bagli_cihazlar_ve_bugun_yeni_uyeler() {
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let mut ses = ortak::Sessions::new();
        for (mac, ad, ip) in [("aa:bb:cc:dd:ee:01", "Ayşe", "10.50.0.23"), ("aa:bb:cc:dd:ee:02", "Mehmet", "10.50.0.41")] {
            ses.insert(mac.into(), ortak::Session {
                phone: "905334553132".into(), ad: ad.into(), soyad: "Yılmaz".into(), ip: ip.into(),
                session_id: "s".into(), start: "2026-09-29T20:14:00+03:00".into(), start_epoch: 1.0, expires_epoch: 9e9,
            });
        }
        ortak::save_sessions(&cfg.main.state_root, &ses).unwrap();
        // yalnızca ...:01 ağda (son 5 dk)
        let sys = e.root.join("sys-ozet");
        std::fs::create_dir_all(sys.join(&cfg.main.iface)).unwrap();
        std::fs::write(sys.join(&cfg.main.iface).join("brforward"), [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x01, 1, 0, 0x3d, 0x21, 0, 0, 0, 0, 0, 0]).unwrap();
        std::fs::create_dir_all(sys.join("wfc")).unwrap();
        e.p.sys = sys;
        ortak::upsert_index(&cfg.main.log_root, "905551112233", "Zeynep", "Ak", "2026-09-29T19:40:00+03:00").unwrap();
        ortak::upsert_index(&cfg.main.log_root, "905551114455", "Eski", "Üye", "2026-08-01T10:00:00+03:00").unwrap();

        let s = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        let kutu = |baslik: &str| s.split(&format!("<h2>{baslik}</h2>")).nth(1).unwrap().split("</section>").next().unwrap().to_string();
        let bagli = kutu("Bağlı Kullanıcılar");
        assert!(bagli.contains("<b>Ayşe Yılmaz</b><span>10.50.0.23 · 20:14</span>") && !bagli.contains("Mehmet"), "{bagli}");
        let yeni = kutu("Bugün Yeni Üyeler");
        assert!(yeni.contains("<b>Zeynep Ak</b><span>19:40</span>") && !yeni.contains("Eski"), "{yeni}");
        assert!(s.contains("<dt>Bağlı kullanıcı</dt><dd>1</dd>") && s.contains("<dt>Bugün yeni üye</dt><dd>1</dd>"));
        assert!(!s.contains("905334553132") && !s.contains("aa:bb:cc"), "özette telefon ve MAC yok");
        for k in ["/cihazlar#oturumlar", "/kayitlar#site-ara", "/cihazlar#yasakli-siteler"] {
            assert!(s.contains(&format!("<a href=\"{k}\">")), "{k}");
        }
        assert!(!s.contains("href=\"/kayitlar#gunler\"") && !s.contains("href=\"/ayarlar#isletme\""), "menüde olanlar kısayolda yok");
        assert!(kutu("SMS ve Disk").contains("Bugün SMS") && s.contains("for=\"menu-ac\""));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_OZET"));
        assert_eq!(kisa_zaman("2026-09-28T08:05:00+03:00", "2026-09-29"), "28.09 08:05");
    }

    #[test]
    fn admin_bolumleri_ve_yonlendirmeler() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let k = e.p.handle(&req("GET", "/kayitlar", &[], Some(&tok))).body;
        assert!(!k.contains("id=\"panel-hareketleri\"") && k.contains("action=\"/kullanicilar\""));
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("<section class=\"bolum\" id=\"admin-ayarlari\"") && a.contains("name=\"netgsm.usercode\""));
        assert!(a.contains("<section class=\"bolum\" id=\"panel-hareketleri\"") && a.contains("href=\"/admin\" class=\"aktif\""));
        assert!(!e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("id=\"admin-ayarlari\""));
        assert!(tekrar_eden_idler(&a).is_empty(), "{:?}", tekrar_eden_idler(&a));
        // form gönderince aynı sayfanın o bölümüne dönülür; mesaj korunur
        let r = e.p.handle(&req("POST", "/yasak/ekle", &[("csrf", &csrf), ("mac", "aa:bb:cc:dd:ee:01"), ("ad", "x")], Some(&tok)));
        assert!(loc(&r).starts_with("/cihazlar?m=") && loc(&r).ends_with("#yasak"), "{}", loc(&r));
        let r = e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("limits.sms_global_day", "250")], Some(&tok)));
        assert!(loc(&r).starts_with("/admin?m=") && loc(&r).ends_with("#admin-ayarlari"), "{}", loc(&r));
        // sorgulu adresler (arama sonucu) olduğu gibi kalır
        assert_eq!(redirect("/kullanicilar?q=x", None).headers[0].1, "/kullanicilar?q=x");
    }

    #[test]
    fn merkezden_sms_alanlari_salt_okunur() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        (c.sms.merkez, c.sms.mock, c.netgsm.usercode, c.netgsm.password, c.netgsm.msgheader) =
            (true, false, "8503027084".into(), "gizli-1".into(), "gztp.blgsyr".into());
        c.save(&e.p.cfg_path).unwrap();
        let page = e.p.handle(&req("GET", "/admin-ayarlari", &[], Some(&tok))).body;
        assert!(page.contains("merkezden yönetiliyor") && !page.contains("name=\"netgsm.usercode\"") && !page.contains("gizli-1"));
        e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("netgsm.usercode", "999"), ("limits.sms_global_day", "250")], Some(&tok)));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.netgsm.usercode.as_str(), c.sms.mock, c.limits.sms_global_day), ("8503027084", false, 250)); // sms.mock kutusu yok sayıldı
    }



    #[test]
    fn giris_ekrani_ilk_giris_merkezden() {
        let mut e = env();
        e.p.hesaplar.set_admin("hizmet-parola-1").unwrap();
        // güncellenen eski cihaz: hesaplar.json'da eski yerel sahip hesabı kalmış
        let ham = std::fs::read_to_string(e.root.join("hesaplar.json")).unwrap();
        let mut j: serde_json::Value = serde_json::from_str(&ham).unwrap();
        let mut eski = j["admin"].clone();
        eski["rol"] = "sahip".into();
        j["mudur"] = eski;
        std::fs::write(e.root.join("hesaplar.json"), j.to_string()).unwrap();
        assert_eq!(loc(&e.p.handle(&req("GET", "/", &[], None))), "/giris"); // kurulum ekranı yok
        assert_eq!(e.p.handle(&req("GET", "/kurulum", &[], None)).status, 404);
        let page = e.p.handle(&req("GET", "/giris", &[], None)).body;
        assert!(page.contains("WifiCorrect") && !page.contains("müşteriye bağlanmadı"));
        // merkeze ulaşılamıyor: müşteri giremez; admin kullanıcı girişi yok
        let r = e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "sahip-parola-12")], None));
        assert!(r.body.contains("Merkeze ulaşılamadı") && r.body.contains("Ethernet 1"));
        let r = e.p.handle(&req("POST", "/giris", &[("kullanici", "admin"), ("parola", "hizmet-parola-1")], None));
        assert!(r.body.contains("hatalı") && !r.headers.iter().any(|(k, _)| k == "Set-Cookie"));
        // numara biçimsizse merkeze sorulmaz
        assert!(e.p.handle(&req("POST", "/giris", &[("kullanici", "mudur"), ("parola", "x")], None)).body.contains("hatalı"));
        // merkez: önce hatalı parola, sonra başarı
        let cagri = Arc::new(Mutex::new(0));
        let c2 = cagri.clone();
        e.p.http = Box::new(move |yol: &str, _: &str| {
            assert_eq!(yol, "/api/giris");
            *c2.lock().unwrap() += 1;
            let n = *c2.lock().unwrap();
            Ok(if n == 1 {
                (401, r#"{"hata":"Müşteri numarası veya parola hatalı.","kod":"hatali"}"#.to_string())
            } else {
                (200, format!(r#"{{"tuz":"0011aabb","ozet":"{}","yineleme":120000,"cihaz_anahtari":"k-1","tunel_ip":"10.99.0.12","sunucu_pub":"S","uc_nokta":"vpn.wificorrect.com:51820","yedek_hedefi":"wfc-{MUSTERI}@10.99.0.1:"}}"#,
                              crate::hesap::digest("sahip-parola-12", "0011aabb", 120_000)))
            })
        });
        assert!(e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "yanlis-parola")], None)).body.contains("hatalı"));
        let r = e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "sahip-parola-12")], None));
        // eşleşme sonucu bir kez gösterilir (yerel oturum çerezi yok); sonra yerel erişim kapalı
        assert!(r.status == 200 && r.body.contains("Cihaz eşleşti") && r.body.contains("panel.wificorrect.com"), "{}", r.body);
        assert!(!r.headers.iter().any(|(k, _)| k == "Set-Cookie"));
        let m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        assert_eq!((m.numara.as_str(), m.cihaz_anahtari.as_str()), (MUSTERI, "k-1"));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert!(c.uzak.enabled && c.uzak.adres == "10.99.0.12" && c.backup.target == format!("wfc-{MUSTERI}@10.99.0.1:"));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl uzak-uygula") && c.contains(&"--collect".to_string())));
        assert_eq!(e.p.hesaplar.load().unwrap().len(), 1); // eski yerel sahip hesabı silindi
        // bağlandıktan sonra: müşteri girişi yalnızca panel.wificorrect.com'dan (cihazda form yok)
        e.p.http = merkez_yok();
        let r = e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "sahip-parola-12")], None));
        assert!(r.status == 404 && r.body.is_empty());
        // müşteri ağı
        let mut r = req("GET", "/giris", &[], None);
        r.ip = "10.50.0.23".into();
        assert_eq!(e.p.handle(&r).status, 403);
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_MERKEZ_BAGLANDI") && audit.contains("PANEL_GIRIS_HATA"));
        assert!(!audit.contains("k-1")); // cihaz anahtarı denetime yazılmaz
        // eşleştirme girişinde kilit: 5 hatadan sonra doğru parola da beklemeli (merkeze sorulmaz)
        let e = env();
        for _ in 0..5 {
            e.p.handle(&req("POST", "/giris", &[("kullanici", "mudur"), ("parola", "yanlis-parola")], None));
        }
        assert!(e.p.handle(&req("POST", "/giris", &[("kullanici", MUSTERI), ("parola", "sahip-parola-12")], None)).body.contains("Çok fazla"));
        assert!(std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap().contains("PANEL_GIRIS_KILIT"));
    }

    #[test]
    fn merkezden_yeni_ozet_gelince_eski_oturum_duser() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        assert_eq!(e.p.handle(&req("GET", "/", &[], Some(&tok))).status, 200);
        let mut m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        m.ozet = crate::hesap::digest("baska-parola-1", &m.tuz, m.yineleme); // 06:00 eşitlemesi sıfırlanmış parolayı getirdi
        crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
        assert_eq!(loc(&e.p.handle(&basliksiz(req("GET", "/", &[], Some(&tok))))), "/giris");
        let r = e.p.handle(&req("GET", "/", &[], Some(&tok))); // merkez müşteriyi doğruladı: yeni oturum
        assert!(r.status == 200 && r.headers.iter().any(|(k, _)| k == "Set-Cookie"));
        crate::merkez::sil(&e.p.merkez_path); // fabrika: bağ silindi
        let (tok2, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        crate::merkez::sil(&e.p.merkez_path);
        assert_eq!(loc(&e.p.handle(&req("GET", "/", &[], Some(&tok2)))), "/giris");
    }

    #[test]
    fn csrf_required_and_roles() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        assert_eq!(e.p.handle(&req("POST", "/ayarlar", &[("main.site_name", "X")], Some(&tok))).status, 403); // CSRF yok
        let r = e.p.handle(&req("GET", "/admin-ayarlari", &[], Some(&tok))); // Admin ayarları admin kilidiyle
        assert!(r.body.contains("action=\"/admin-kilidi\"") && !r.body.contains("name=\"sms.mock\""));
        assert_eq!(e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("sms.mock", "0")], Some(&tok))).status, 403);
        let page = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(!page.contains("href=\"/admin-ayarlari\"") && !page.contains("href=\"/hesaplar\""));
        assert_eq!(e.p.handle(&req("GET", "/hesaplar", &[], Some(&tok))).status, 404); // yerel ek hesap yok
        let page = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        for gizli in ["netgsm", "NetGSM", "Twilio", "twilio", "sms.mock", "backup.target", "Yedek", "sms_global_day", "Fabrika"] {
            assert!(!page.contains(gizli), "{gizli}"); // işletme sahibi sağlayıcı / sunucu / sınır görmez
        }
        assert!(page.contains("main.unvan") && page.contains("main.session_minutes"));
        // API alanları Ayarlar formundan elle gönderilse de yok sayılır
        let r = e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.session_minutes", "1440"), ("netgsm.password", "kotu-sifre"), ("sms.mock", "0")], Some(&tok)));
        assert_eq!(r.status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.main.session_minutes, c.netgsm.password.as_str(), c.sms.mock), (1440, "", true));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ") == "systemctl restart wificorrect-portal"));
    }

    #[test]
    fn provider_sets_secrets_write_only_and_validates() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let page = e.p.handle(&req("GET", "/admin-ayarlari", &[], Some(&tok))).body;
        assert!(page.contains("name=\"netgsm.password\"") && page.contains("sms.mock") && !page.contains("main.session_minutes"));
        let r = e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("netgsm.password", "gizli-sifre-1"), ("netgsm.usercode", "8503027084"), ("sms.mock", "1"), ("netgsm.msgheader", "COK-UZUN-BASLIK-OLMAZ")], Some(&tok)));
        assert!(r.status == 200 && r.body.contains("Geçersiz değer")); // geçersiz → hiçbir şey kaydedilmez
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().netgsm.password, "");
        // bağlı cihazda uzak erişim kutusu formda yok: gönderilmemesi uzak erişimi kapatmaz
        e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("netgsm.password", "gizli-sifre-1"), ("netgsm.usercode", "8503027084"), ("sms.mock", "1")], Some(&tok)));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.netgsm.password.as_str(), c.netgsm.usercode.as_str(), c.uzak.enabled), ("gizli-sifre-1", "8503027084", true));
        let page = e.p.handle(&req("GET", "/admin-ayarlari", &[], Some(&tok))).body;
        assert!(!page.contains("gizli-sifre-1") && page.contains("(tanımlı)")); // şifre geri gösterilmez
        e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("netgsm.password", ""), ("sms.mock", "1")], Some(&tok))); // boş = aynı
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().netgsm.password, "gizli-sifre-1");
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("netgsm.password: *** → ***") && !audit.contains("gizli-sifre-1"));
    }

    #[test]
    fn mock_cannot_be_turned_off_with_missing_sms_settings() {
        // eksik ayarla deneme modu kapanırsa portal açılmaz (çöker-yeniden başlar), misafirler giriş sayfasını göremez
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let f = |extra: &[(&str, &str)]| {
            let mut v = vec![("csrf", csrf.as_str()), ("netgsm.usercode", "8503027084"), ("netgsm.password", "gizli-sifre-1")];
            v.extend_from_slice(extra);
            e.p.handle(&req("POST", "/admin-ayarlari", &v, Some(&tok)))
        };
        let r = f(&[("sms.mock", "0")]);
        assert!(r.headers.iter().any(|(_, v)| v.contains("msgheader")) || r.body.contains("msgheader"));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert!(c.sms.mock && c.netgsm.password.is_empty()); // hiçbir şey kaydedilmedi
        f(&[("sms.mock", "0"), ("netgsm.msgheader", "GOZTEPE")]);
        assert!(!Config::load(&e.p.cfg_path).unwrap().sms.mock);
    }

    #[test]
    fn ban_closes_session_and_owner_sees_people() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let state = Config::load(&e.p.cfg_path).unwrap().main.state_root;
        let mut ses = ortak::Sessions::new();
        ses.insert("aa:bb:cc:dd:ee:01".into(), ortak::Session {
            phone: "905334553132".into(), ad: "Ayşe".into(), soyad: "Yılmaz".into(), ip: "10.50.0.23".into(),
            session_id: "s1".into(), start: "2026-09-29T20:00:00+03:00".into(), start_epoch: 1.0, expires_epoch: 9e9,
        });
        ortak::save_sessions(&state, &ses).unwrap();
        let page = e.p.handle(&req("GET", "/oturumlar", &[], Some(&tok))).body;
        assert!(page.contains("+905334553132") && page.contains("Ayşe Yılmaz")); // kafe sahibi de açık görür
        let r = e.p.handle(&req("POST", "/yasak/ekle", &[("csrf", &csrf), ("mac", "AA-BB-CC-DD-EE-01"), ("ad", "sorunlu")], Some(&tok)));
        assert_eq!(r.status, 303);
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().ban[0].mac, "aa:bb:cc:dd:ee:01");
        assert!(ortak::load_sessions(&state).is_empty()); // bağlantısı kesildi
        let calls = e.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.join(" ") == "nft add element inet hotspot ban_mac { aa:bb:cc:dd:ee:01 }"));
        assert!(calls.iter().any(|c| c[0] == "conntrack"));
        e.p.handle(&req("POST", "/yasak/kaldir", &[("csrf", &csrf), ("mac", "aa:bb:cc:dd:ee:01")], Some(&tok)));
        assert!(Config::load(&e.p.cfg_path).unwrap().ban.is_empty());
    }

    #[test]
    fn password_change_via_center_logs_out_other_sessions() {
        let mut e = env();
        let (t1, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let (t2, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let post = |e: &Env, eski: &str| {
            e.p.handle(&req("POST", "/sifre", &[("csrf", &csrf), ("eski", eski), ("yeni", "yepyeni-parola"), ("yeni2", "yepyeni-parola")], Some(&t1)))
        };
        assert!(post(&e, "yanlis-parola").body.contains("hatalı")); // yerel kontrol, merkeze gitmez
        assert!(post(&e, "sahip-parola-12").body.contains("internet gerekli")); // merkez yok
        let yeni = crate::hesap::digest("yepyeni-parola", "cc", 120_000);
        e.p.http = Box::new(move |yol: &str, govde: &str| {
            assert!(yol == "/api/parola" && govde.contains("\"eski\":\"sahip-parola-12\""));
            Ok((200, format!(r#"{{"tuz":"cc","ozet":"{yeni}","yineleme":120000}}"#)))
        });
        let r = post(&e, "sahip-parola-12");
        assert_eq!(r.status, 303);
        assert!(crate::merkez::dogrula(&crate::merkez::oku(&e.p.merkez_path).unwrap(), "yepyeni-parola"));
        assert_eq!(loc(&e.p.handle(&basliksiz(req("GET", "/", &[], Some(&t2))))), "/giris"); // diğer oturum düştü
        let cookie = r.headers.iter().find(|(k, _)| k == "Set-Cookie").map(|(_, v)| v.clone()).expect("yeni çerez");
        let t3 = cookie.trim_start_matches("wfc=").split(';').next().unwrap().to_string();
        assert_eq!(e.p.handle(&req("GET", "/", &[], Some(&t3))).status, 200); // değiştiren yeni oturumla devam eder
        // admin kilidi açıkken de Şifremi değiştir müşteri parolasıdır; admin parolası panelden değişmez (merkezden gelir)
        e.p.http = merkez_yok();
        let (a1, acsrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let r = e.p.handle(&req("POST", "/sifre", &[("csrf", &acsrf), ("eski", "hizmet-parola-1"), ("yeni", "yeni-admin-12"), ("yeni2", "yeni-admin-12")], Some(&a1)));
        assert!(r.body.contains("Şu anki parola hatalı"));
        assert!(e.p.hesaplar.verify("admin", "yeni-admin-12").is_none() && e.p.hesaplar.verify("admin", "hizmet-parola-1").is_some());
    }

    fn get(path: &str, query: &[(&str, &str)], token: &str) -> Req {
        let mut r = req("GET", path, &[], Some(token));
        r.query = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        r
    }

    #[test]
    fn owner_reads_records_people_and_official_requests() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let root = cfg.main.log_root.clone();
        let ses = |olay: &str, z: &str| {
            Row::new(olay, z).set("telefon", "905334553132").set("ad", "Ayşe").set("soyad", "Yılmaz").set("mac", "aa:bb:cc:dd:ee:01").set("ic_ip", "10.50.0.23").set("oturum_id", "s1")
        };
        let d = "2026-09-27";
        ortak::append_rows(&ortak::day_file(&root, d, "oturum.csv"), &[ses("OTURUM_BASLA", "2026-09-27T10:00:00+03:00")]).unwrap();
        ortak::append_rows(&ortak::day_file(&root, d, "dns.csv"), &[
            Row::new("DNS", "2026-09-27T10:01:00+03:00").set("telefon", "905334553132").set("alan_adi", "www.ornek.com"),
            Row::new("DNS", "2026-09-27T10:02:00+03:00").set("telefon", "905334553132").set("alan_adi", "haber.example"),
        ]).unwrap();
        ortak::append_rows(&ortak::user_file(&root, d, "905334553132"), &[ses("OTURUM_BASLA", "2026-09-27T10:00:00+03:00")]).unwrap();
        ortak::upsert_index(&root, "905334553132", "Ayşe", "Yılmaz", "2026-09-27T10:00:00+03:00").unwrap();
        crate::muhur::close_day(&cfg, d, 1_790_550_000.0, true).unwrap(); // mühürlü gün
        ortak::append_rows(&ortak::day_file(&root, "2026-09-29", "dhcp.csv"), &[Row::new("DHCP_ATAMA", "2026-09-29T09:00:00+03:00").set("ic_ip", "10.50.0.30")]).unwrap();

        let page = e.p.handle(&get("/kayitlar", &[], &tok)).body;
        assert!(page.contains("2026-09-29") && page.contains("açık") && page.contains("mühürlü"));
        let page = e.p.handle(&get("/kayitlar/gun", &[("gun", d)], &tok)).body;
        assert!(page.contains("dns.csv.gz") && page.contains("Ayşe Yılmaz")); // kişi dosyasının sahibi
        let page = e.p.handle(&get("/kayitlar/dosya", &[("gun", d), ("dosya", "dns.csv.gz"), ("q", "ORNEK")], &tok)).body;
        assert!(page.contains("www.ornek.com") && !page.contains("haber.example") && page.contains("1 satır"));
        for bad in ["../../zincir.txt", "../2026-09-29/dhcp.csv", "MANIFEST.sha256"] {
            assert_eq!(e.p.handle(&get("/kayitlar/indir", &[("gun", d), ("dosya", bad)], &tok)).status, 404, "{bad}");
        }
        let r = e.p.handle(&get("/kayitlar/indir", &[("gun", d), ("dosya", "dns.csv.gz")], &tok));
        assert!(r.body.starts_with("\u{feff}zaman;olay") && r.body.contains("haber.example")); // açılmış, Excel'de açılır
        assert!(r.headers.iter().any(|(_, v)| v.contains("2026-09-27_dns.csv\"")));
        let r = e.p.handle(&get("/kayitlar/indir", &[("gun", "2026-09-29"), ("dosya", "dhcp.csv")], &tok));
        assert!(r.file.is_some() && r.status == 200);
        let r = e.p.handle(&get("/kayitlar/gun-indir", &[("gun", d)], &tok));
        assert!(r.file.is_some() && r.headers.iter().any(|(_, v)| v.contains("kayit_2026-09-27.tar")));

        let page = e.p.handle(&get("/kullanicilar", &[("q", "ayşe")], &tok)).body;
        assert!(page.contains("+905334553132") && page.contains("1 kişi"));
        let page = e.p.handle(&get("/kullanici", &[("tel", "905334553132")], &tok)).body;
        assert!(page.contains("OTURUM_BASLA") && page.contains("2026-09-27"));
        assert_eq!(e.p.handle(&get("/kullanici", &[("tel", "../x")], &tok)).status, 404);

        let page = e.p.handle(&get("/talep", &[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "2026-09-27 11:00")], &tok)).body;
        assert!(page.contains("O anda açık oturum") && page.contains("905334553132"));
        let page = e.p.handle(&get("/talep", &[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "dün")], &tok)).body;
        assert!(page.contains("Zaman geçersiz"));
        let r = e.p.handle(&get("/talep/paket", &[("bas", "2026-09-27"), ("bit", "2026-09-29")], &tok));
        assert!(r.file.is_some());
        assert!(loc(&e.p.handle(&get("/talep/paket", &[("bas", "x"), ("bit", "y")], &tok))).contains("e=1"));
        assert_eq!(e.p.handle(&basliksiz(req("GET", "/kayitlar", &[], None))).status, 303); // girişsiz yok

        let audit = std::fs::read_to_string(ortak::day_file(&root, "2026-09-29", "denetim.csv")).unwrap();
        for olay in ["PANEL_KAYIT_GORUNTULE", "PANEL_KAYIT_INDIR", "PANEL_KULLANICI", "PANEL_TALEP_ARA", "PANEL_TALEP_PAKET"] {
            assert!(audit.contains(olay), "{olay}");
        }
    }

    #[test]
    fn ports_page_change_confirm_and_rollback() {
        let mut e = env();
        let sys = e.root.join("sys");
        for (n, mac, up) in [("enp3s0", "00:0e:c4:ce:a0:9b", "1"), ("enp1s0", "00:0e:c4:ce:a0:9a", "0"), ("wlp2s0", "90:00:4e:b5:b3:c5", "0")] {
            std::fs::create_dir_all(sys.join(n).join("device")).unwrap();
            std::fs::write(sys.join(n).join("address"), format!("{mac}\n")).unwrap();
            std::fs::write(sys.join(n).join("carrier"), up).unwrap();
            std::fs::write(sys.join(n).join("speed"), "1000").unwrap();
        }
        std::fs::create_dir_all(sys.join("wlp2s0").join("wireless")).unwrap();
        e.p.sys = sys;
        e.p.ag_yollar = crate::ag::Yollar {
            ag: e.root.join("ag.toml"),
            interfaces: e.root.join("interfaces"),
            nft: e.root.join("arayuzler.nft"),
            hostapd: e.root.join("hostapd.conf"),
            issue: e.root.join("issue"),
            durum: e.root.join("durum"),
        };
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12"); // kafe sahibi de port değiştirebilir
        let page = e.p.handle(&req("GET", "/portlar", &[], Some(&tok))).body;
        assert!(page.contains("Ethernet 1 (sağ)") && page.contains("bağlı, 1000 Mb/s") && page.contains("Ethernet 2 (sol)") && !page.contains("wlp2s0</span>"));
        let post = |form: &[(&str, &str)]| {
            let mut f = vec![("csrf", csrf.as_str())];
            f.extend_from_slice(form);
            e.p.handle(&req("POST", "/portlar", &f, Some(&tok)))
        };
        // iki port birden internet alamaz; hiçbiri vermiyorsa olmaz
        assert!(loc(&post(&[("rol_enp3s0", "alir"), ("rol_enp1s0", "alir"), ("wifi", "kapali")])).contains("e=1"));
        assert!(loc(&post(&[("rol_enp3s0", "alir"), ("rol_enp1s0", "kapali"), ("wifi", "kapali")])).contains("e=1"));
        assert!(loc(&post(&[("rol_enp3s0", "alir"), ("rol_enp1s0", "verir"), ("wifi", "kapali"), ("kanal", "6")])).contains("%20yok."));
        // portları değiştir + Wi-Fi aç
        let r = post(&[("rol_enp3s0", "verir"), ("rol_enp1s0", "alir"), ("wifi", "verir"), ("ssid", "Bocafe Misafir"), ("sifre", ""), ("kanal", "11")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let staged = crate::ag::load(&e.p.ag_yollar.yeni());
        assert_eq!((staged.wan.as_str(), staged.wan_mac.as_str(), staged.wifi.ssid.as_str()), ("enp1s0", "00:0e:c4:ce:a0:9b", "Bocafe Misafir")); // MAC korunur
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl ag-gecis")));
        // işi elle çalıştır (systemd-run'ın yapacağı), sonra panel onay ister
        let runner = |_: &[String]| true;
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        crate::ag::gecis(&cfg, &e.p.ag_yollar, &e.p.ag_yollar.yeni(), 1_790_705_134.0, &runner).unwrap();
        let page = e.p.handle(&req("GET", "/portlar", &[], Some(&tok))).body;
        assert!(page.contains("180 saniye") && page.contains("Hemen geri al"));
        assert!(loc(&post(&[("rol_enp3s0", "alir"), ("rol_enp1s0", "verir"), ("wifi", "kapali")])).contains("e=1")); // beklerken yeni değişiklik yok
        assert!(loc(&e.p.handle(&req("POST", "/portlar/geri-al", &[("csrf", &csrf)], Some(&tok)))).contains("geri%20y"));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ag-geri-al elle")));
        let r = e.p.handle(&req("POST", "/portlar/onayla", &[("csrf", &csrf)], Some(&tok)));
        assert!(!loc(&r).contains("e=1") && crate::ag::pending(&e.p.ag_yollar).is_none());
        assert_eq!(crate::ag::load(&e.p.ag_yollar.ag).wan, "enp1s0");
        e.p.handle(&req("POST", "/portlar/etiket", &[("csrf", &csrf)], Some(&tok)));
        assert!(e.p.handle(&req("GET", "/portlar", &[], Some(&tok))).body.contains("Ethernet 1 (sol)"));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_PORT;") || audit.contains(";PANEL_PORT"));
        assert!(audit.contains("PANEL_PORT_ONAY") && audit.contains("PANEL_PORT_GERI_AL") && audit.contains("Ethernet 2 (sol) alır"));
    }

    #[test]
    fn owner_manages_blocked_sites_and_words() {
        let mut e = env();
        e.p.filtre_conf = e.root.join("yasak.conf");
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&req("GET", "/yasakli-siteler", &[], Some(&tok))).body;
        assert!(page.matches("Liste boş").count() == 3); // boş başlar
        let add = |liste: &str, v: &str| e.p.handle(&req("POST", "/yasakli-siteler/ekle", &[("csrf", &csrf), ("liste", liste), ("deger", v)], Some(&tok)));
        assert!(!loc(&add("site", "https://www.Bet365.com/tr")).contains("e=1"));
        assert!(!loc(&add("kelime", "Bet")).contains("e=1"));
        assert!(!loc(&add("istisna", "alphabet")).contains("e=1"));
        assert!(loc(&add("kelime", "bet")).contains("e=1")); // aynısı iki kez yok
        assert!(loc(&add("kelime", "şans")).contains("e=1"));
        assert!(loc(&add("site", "x.com\naddress=/y/1.2.3.4")).contains("e=1")); // dosyaya satır sokulamaz
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.filtre.siteler.clone(), c.filtre.kelimeler.clone(), c.filtre.istisnalar.clone()), (vec!["www.bet365.com".to_string()], vec!["bet".to_string()], vec!["alphabet".to_string()]));
        assert!(std::fs::read_to_string(&e.p.filtre_conf).unwrap().contains("address=/www.bet365.com/"));
        assert!(e.calls.lock().unwrap().iter().any(|c| c[0] == "iptables-restore"));
        let page = e.p.handle(&get("/yasakli-siteler", &[("ad", "superbet.com.tr")], &tok)).body;
        assert!(page.contains("engelli (yasaklı kelime: bet)"));
        assert!(e.p.handle(&get("/yasakli-siteler", &[("ad", "alphabet.com")], &tok)).body.contains("engelli değil"));
        e.p.handle(&req("POST", "/yasakli-siteler/kaldir", &[("csrf", &csrf), ("liste", "kelime"), ("deger", "bet")], Some(&tok)));
        assert!(Config::load(&e.p.cfg_path).unwrap().filtre.kelimeler.is_empty());
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_FILTRE_EKLE") && audit.contains("liste=site deger=www.bet365.com") && audit.contains("PANEL_FILTRE_KALDIR"));
    }

    #[test]
    fn owner_activity_is_logged_and_only_admin_sees_it() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        e.p.handle(&get("/oturumlar", &[], &tok));
        e.p.handle(&get("/kullanicilar", &[], &tok));
        e.p.handle(&get("/kullanicilar", &[("q", "ayşe")], &tok));
        assert!(e.p.handle(&get("/kullanicilar", &[], &tok)).body.contains("hizmet sağlayıcı tarafından denetlenir"));
        let r = e.p.handle(&get("/panel-hareketleri", &[], &tok)); // sahip kendi izini admin parolası olmadan göremez/silemez
        assert!(r.body.contains("action=\"/admin-kilidi\"") && !r.body.contains("Bağlı kullanıcılara baktı"));
        assert!(!e.p.handle(&get("/", &[], &tok)).body.contains("/panel-hareketleri"));
        e.p.handle(&req("POST", "/cikis", &[("csrf", &csrf)], Some(&tok)));
        let (atok, _) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let page = e.p.handle(&get("/panel-hareketleri", &[("kim", MUSTERI), ("kisisel", "1")], &atok)).body;
        assert!(page.contains("Bağlı kullanıcılara baktı") && page.contains("Kullanıcı listesine baktı") && page.contains("ara=ayşe"));
        assert!(!page.contains(">Giriş<")); // yalnızca kişisel veri filtresi
        let page = e.p.handle(&get("/panel-hareketleri", &[], &atok)).body;
        assert!(page.contains("İşletme sahibi") && page.contains(">7<") && page.contains("Çıkış")); // müşteri: 7 kişisel veri bakışı (bağlı cihazlar + 2 liste + arama + 3 özet: iki girişte ve yukarıda)
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains(&format!("kullanici={MUSTERI} rol=sahip ip=")));
    }

    fn give_reason(e: &Env, tok: &str, csrf: &str) {
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", csrf), ("gerekce", "Müşteri şikâyeti incelemesi"), ("donus", "/")], Some(tok)));
        assert_eq!(loc(&r), "/");
    }

    #[test]
    fn owner_must_give_reason_before_personal_data() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let r = e.p.handle(&get("/kullanicilar", &[("q", "ayşe")], &tok));
        assert!(r.body.contains("Gerekçe gerekli") && r.body.contains("value=\"/kullanicilar?q=ay%C5%9Fe\"")); // dönüş adresi korunur
        assert!(e.p.handle(&get("/kullanici", &[("tel", "905334553132")], &tok)).body.contains("Gerekçe gerekli"));
        assert!(!e.p.handle(&get("/talep", &[], &tok)).body.contains("Gerekçe gerekli")); // boş talep formu açılır
        assert!(e.p.handle(&get("/talep", &[("tur", "telefon"), ("deger", "05334553132")], &tok)).body.contains("Gerekçe gerekli"));
        assert!(!e.p.handle(&get("/oturumlar", &[], &tok)).body.contains("Gerekçe gerekli")); // bağlı cihazlar gerekçesiz
        // kısa gerekçe olmaz; başka siteye dönüş olmaz
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "bakıyorum"), ("donus", "/kullanicilar")], Some(&tok)));
        assert!(r.body.contains("10-200 karakter"));
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "Emniyet / savcılık talebi"), ("donus", "//kotu.example/x")], Some(&tok)));
        assert_eq!(loc(&r), "/");
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "Emniyet / savcılık talebi"), ("donus", "/kullanicilar?q=ay%C5%9Fe")], Some(&tok)));
        assert_eq!(loc(&r), "/kullanicilar?q=ay%C5%9Fe");
        assert!(!e.p.handle(&get("/kullanicilar", &[("q", "ayşe")], &tok)).body.contains("Gerekçe gerekli"));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_GEREKCE") && audit.contains("ara=ayşe gerekce=Emniyet / savcılık talebi")); // her bakışa eklenir
        // süre dolunca yeniden sorulur
        e.p.oturumlar.set_gerekce(&tok, "eski gerekçe metni", 1_790_705_134.0 - 1.0);
        assert!(e.p.handle(&get("/kullanicilar", &[], &tok)).body.contains("Gerekçe gerekli"));
        // admin muaf
        let (atok, _) = setup_and_login(&e, "admin", "hizmet-parola-1");
        assert!(!e.p.handle(&get("/kullanicilar", &[], &atok)).body.contains("Gerekçe gerekli"));
    }

    #[test]
    fn owner_edits_portal_texts() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&get("/portal-metinleri", &[], &tok)).body;
        assert!(page.contains("KVKK Aydınlatma Metni") && page.contains("Açık Rıza Metni") && page.contains("İnternet Kullanıcı Sözleşmesi"));
        let r = e.p.handle(&req("POST", "/portal-metinleri", &[("csrf", &csrf), ("aydinlatma", "Veri sorumlusu: <Bocafe>\r\n\r\nİkinci paragraf"), ("sozlesme", "Kurallar"), ("acik_riza", "Onay metni")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.portal.aydinlatma.as_str(), c.portal.sozlesme.as_str(), c.portal.acik_riza.as_str()), ("Veri sorumlusu: <Bocafe>\n\nİkinci paragraf", "Kurallar", "Onay metni"));
        assert!(e.p.handle(&get("/portal-metinleri", &[], &tok)).body.contains("Veri sorumlusu: &lt;Bocafe&gt;")); // kaçışlı
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ") == "systemctl restart wificorrect-portal"));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_PORTAL_METIN") && audit.contains("aydinlatma(41 karakter)") && audit.contains("acik_riza(10 karakter)"));
    }

    #[test]
    fn admin_settings_page_and_factory_reset() {
        let e = env();
        let (otok, ocsrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ozet = e.p.handle(&get("/", &[], &otok)).body; // varsayılan: SMS deneme modu
        assert!(ozet.contains("hizmet sağlayıcınıza başvurun") && !ozet.contains("deneme modu") && !ozet.contains("Uzak yedek"));
        assert!(!e.p.handle(&get("/sistem", &[], &otok)).body.contains("Uzak yedek"));
        assert_eq!(e.p.handle(&req("POST", "/admin-ayarlari/fabrika", &[("csrf", &ocsrf), ("parola", "sahip-parola-12"), ("onay", "1")], Some(&otok))).status, 403);
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let page = e.p.handle(&get("/admin-ayarlari", &[], &tok)).body;
        for gorunur in ["backup.target", "main.retention_days", "limits.sms_global_day", "netgsm.usercode", "twilio.enabled", "Fabrika ayarları"] {
            assert!(page.contains(gorunur), "{gorunur}");
        }
        assert!(!page.contains("main.unvan")); // işletme ayarları Ayarlar sayfasında
        assert!(e.p.handle(&get("/sistem", &[], &tok)).body.contains("Uzak yedek"));
        let fab = |pw: &str, onay: &str| e.p.handle(&req("POST", "/admin-ayarlari/fabrika", &[("csrf", &csrf), ("parola", pw), ("onay", onay)], Some(&tok)));
        assert!(loc(&fab("yanlis-parola", "1")).contains("e=1"));
        assert!(loc(&fab("hizmet-parola-1", "")).contains("e=1"));
        assert!(!e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl fabrika")));
        // şimdi yedekle: kapalıyken reddedilir
        assert!(page.contains("Son yedek") && page.contains("henüz çalışmadı"));
        assert!(loc(&e.p.handle(&req("POST", "/admin-ayarlari/yedekle", &[("csrf", &csrf)], Some(&tok)))).contains("e=1"));
        // denetimdeki sonuçlar durumda görünür
        let cfgd = Config::load(&e.p.cfg_path).unwrap();
        ortak::audit(&cfgd.main.log_root, Row::new("YEDEK_SONUC", "2026-09-29T02:00:05+03:00").set("ek", "hata=zincir.txt sunucuya gönderilemedi"));
        ortak::audit(&cfgd.main.log_root, Row::new("FABRIKA_IPTAL", "2026-09-29T03:00:00+03:00").set("ek", "neden=2026-09-28 sunucuya gönderilemedi"));
        let page = e.p.handle(&get("/admin-ayarlari", &[], &tok)).body;
        assert!(page.contains("2026-09-29 02:00 — HATA: zincir.txt sunucuya gönderilemedi"));
        assert!(page.contains("Son fabrika dönüşü <b>iptal oldu</b> (2026-09-29 03:00): 2026-09-28 sunucuya gönderilemedi"));
        // uzak yedek kapalıyken reddedilir (kayıtlar sunucuya gitmeden olmaz)
        let r = fab("hizmet-parola-1", "1");
        assert!(loc(&r).contains("e=1") && !e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl fabrika")));
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.backup.enabled = true;
        c.backup.target = "kafe-x@sunucu:".into();
        c.save(&e.p.cfg_path).unwrap();
        // cihaz müşteriye bağlıyken elle fabrika yok (merkezde sahipsiz bağlı kalırdı): önce serbest bırakılır
        let r = fab("hizmet-parola-1", "1");
        assert!(loc(&r).contains("e=1") && !e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl fabrika")));
        assert!(!loc(&e.p.handle(&req("POST", "/admin-ayarlari/yedekle", &[("csrf", &csrf)], Some(&tok)))).contains("e=1"));
        crate::merkez::sil(&e.p.merkez_path);
        // bağsız cihazda panel yalnızca eşleştirme: Admin ayarları da açılmaz
        assert_eq!(loc(&e.p.handle(&get("/admin-ayarlari", &[], &tok))), "/giris");
        // işleyicinin kendisi: admin parolası doğru, cihaz bağsız → başlar (panelden artık ulaşılmıyor; bkz. Task 3 raporu)
        let o = e.p.oturumlar.get(&tok, 1_790_705_134.0).unwrap();
        let cfgd = Config::load(&e.p.cfg_path).unwrap();
        let r = e.p.fabrika(&cfgd, &req("POST", "/admin-ayarlari/fabrika", &[("parola", "hizmet-parola-1"), ("onay", "1")], Some(&tok)), &o);
        assert!(r.body.contains("fabrika ayarlarına dönüyor"));
        let (tok, _) = setup_and_login(&e, "admin", "hizmet-parola-1"); // yeniden bağlandı
        assert!(e.p.handle(&get("/admin-ayarlari", &[], &tok)).body.contains("kayıtlar sunucuya gönderiliyor")); // sürüyor
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").ends_with("wificorrect ctl yedekle")));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").ends_with("wificorrect ctl fabrika")));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_FABRIKA_RED") && audit.contains("PANEL_FABRIKA;") || audit.contains(";PANEL_FABRIKA;"));
    }

    #[test]
    fn admin_remote_access_settings() {
        let e = env();
        let (otok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        assert!(!e.p.handle(&get("/ayarlar", &[], &otok)).body.contains("uzak.")); // işletme sahibi görmez
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let page = e.p.handle(&get("/admin-ayarlari", &[], &tok)).body;
        assert!(!page.contains("tanıtma") && !page.contains("sudo wificorrect-sunucu")); // bağlıyken elle tanıtma yok
        // bağlı cihazda uzak erişim salt okunur (merkez bağlanırken yazar; tünelden kapatan panele erişimini keserdi)
        assert!(!page.contains("name=\"uzak.") && page.contains("10.99.0.11</b> (merkez bağlantısıyla kuruldu)"));
        let r = e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("uzak.sunucu", ""), ("uzak.adres", "10.99.0.17"), ("sms.mock", "1")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert!(c.uzak.enabled && c.uzak.adres == "10.99.0.11" && c.uzak.sunucu == "vpn.wificorrect.com:51820");
        assert!(!e.calls.lock().unwrap().iter().any(|c| c.join(" ").ends_with("wificorrect ctl uzak-uygula")));
    }

    #[test]
    fn admin_kilidi_ve_menu() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ozet = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert_eq!(menu_linkleri(&ozet), ["/", "/cihazlar", "/kayitlar", "/ayarlar", "/admin"]);
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("action=\"/admin-kilidi\"") && !a.contains("name=\"netgsm.usercode\""));
        assert!(e.p.handle(&req("GET", "/panel-hareketleri", &[], Some(&tok))).body.contains("action=\"/admin-kilidi\""));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "yanlis-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert!(r.body.contains("hatalı"));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert_eq!(loc(&r), "/admin");
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("name=\"netgsm.usercode\"") && a.contains("id=\"panel-hareketleri\""));
        // yerel admin kullanıcı girişi yok
        assert!(!e.p.handle(&req("POST", "/giris", &[("kullanici", "admin"), ("parola", "hizmet-parola-1")], None)).headers.iter().any(|(k, _)| k == "Set-Cookie"));
        // kapat: kilit hemen kapanır; başka siteye dönüş yok
        let r = e.p.handle(&req("POST", "/admin-kilidi/kapat", &[("csrf", &csrf)], Some(&tok)));
        assert_eq!(r.status, 303);
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("action=\"/admin-kilidi\""));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("donus", "//kotu.example/x")], Some(&tok)));
        assert_eq!(loc(&r), "/admin");
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_ADMIN_HATALI") && audit.contains("PANEL_ADMIN_ACILDI"));
    }

    #[test]
    fn admin_kilidi_parola_degisince_kapanir_ve_sure_dolunca() {
        let mut e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("name=\"netgsm.usercode\""));
        let oz = crate::hesap::digest("merkez-parola-1", "ab12", 120_000);
        e.p.hesaplar.set_admin_ozet("ab12", &oz, 120_000).unwrap();
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("action=\"/admin-kilidi\""));
        // yeni parolayla açılır, 30 dk sonra kendiliğinden kapanır (müşteri oturumu sürer)
        e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "merkez-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("name=\"limits.sms_global_day\""));
        e.p.clock = Box::new(|| 1_790_705_134.0 + 1801.0);
        let r = e.p.handle(&req("GET", "/admin", &[], Some(&tok)));
        assert!(r.status == 200 && r.body.contains("action=\"/admin-kilidi\"") && !r.body.contains("name=\"limits.sms_global_day\""));
    }

    #[test]
    fn admin_kilidi_hatali_deneme_kilidi() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ac = |pw: &str| e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", pw), ("donus", "/admin")], Some(&tok)));
        for _ in 0..5 {
            assert!(ac("yanlis-parola-1").body.contains("hatalı"));
        }
        let r = ac("hizmet-parola-1");
        assert!(r.body.contains("Çok fazla") && loc(&r).is_empty());
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("action=\"/admin-kilidi\""));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.lines().any(|l| l.contains("PANEL_ADMIN_HATALI") && l.contains("kilit=1")) && !audit.contains("PANEL_ADMIN_ACILDI"));
    }

    #[test]
    fn admin_ayarlari_bagliyken_fabrika_formu_yok() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(!a.contains("action=\"/admin-ayarlari/fabrika\"") && a.contains("serbest bırakın") && !a.contains("tanıtma"));
        // elle gönderilse de bağlıyken reddedilir
        let r = e.p.handle(&req("POST", "/admin-ayarlari/fabrika", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("onay", "1")], Some(&tok)));
        assert!(loc(&r).contains("e=1") && !e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl fabrika")));
    }

    #[test]
    fn admin_parolasi_yoksa_kilit_acilmaz() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let mut j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.root.join("hesaplar.json")).unwrap()).unwrap();
        j.as_object_mut().unwrap().remove("admin");
        std::fs::write(e.root.join("hesaplar.json"), j.to_string()).unwrap();
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("Admin parolası belirlenmemiş") && !a.contains("action=\"/admin-kilidi\""));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert!(r.status == 200 && !e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("name=\"netgsm.usercode\""));
    }

    #[test]
    fn lisans_ve_unvan_uyarisi() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(!page.contains("Lisans askıya") && !page.contains("Unvan girilmemiş"));
        let mut m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
        let page = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(page.contains("Lisans askıya alındı") && page.contains("misafirlere internet verilmiyor"));
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.unvan = String::new();
        c.save(&e.p.cfg_path).unwrap();
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("Unvan girilmemiş"));
    }

}
