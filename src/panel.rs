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

mod ap_aktarim;
mod ap_sayfasi;
mod filtre_sayfasi;
mod gerekce;
mod hareketler;
mod hiz_sayfasi;
mod kayit_sayfalari;
mod metinler;
mod misafir_sayfasi;
mod portlar;
mod rapor_sayfalari;

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

// ---------------------------------------------------------------- ayar alanları
#[derive(Clone, Copy)]
enum Tur {
    Metin(fn(&str) -> bool),
    Sayi(u64, u64),
    Evet,
    Secim(&'static [(&'static str, &'static str)]),
}

struct Alan {
    key: &'static str,
    label: &'static str,
    tur: Tur,
    grup: &'static str,
    /// true: "Admin ayarları" sayfasında, yalnızca admin görür ve değiştirir (SMS sınırları, uzak erişim, yedek)
    admin: bool,
}

fn chars_ok(s: &str, max: usize, allowed: fn(char) -> bool) -> bool {
    s.chars().count() <= max && s.chars().all(allowed)
}

/// Vergi levhası unvanı: 2-200 karakter; harf, rakam, boşluk ve . , - ' & ( ) / "
fn unvan_ok(s: &str) -> bool {
    let n = s.trim().chars().count();
    (2..=200).contains(&n) && s.chars().all(|c| c.is_alphanumeric() || " .,-'&()/\"".contains(c))
}

/// Veri sorumlusu adresi (KVKK aydınlatma metni): boş (Özet uyarır) ya da 5-200 karakter; denetim karakteri ve köşeli
/// parantez yok (yer tutucu sanılmasın).
fn adres_ok(s: &str) -> bool {
    let n = s.trim().chars().count();
    n == 0 || ((5..=200).contains(&n) && s.chars().all(|c| !c.is_control() && !"[]<>".contains(c)))
}

/// İrtibat telefonu (isteğe bağlı): boş ya da rakam, boşluk, + ( ) - ; 7-15 rakam.
fn irtibat_tel_ok(s: &str) -> bool {
    let rakam = s.chars().filter(char::is_ascii_digit).count();
    s.is_empty() || (s.chars().count() <= 25 && (7..=15).contains(&rakam) && s.chars().all(|c| c.is_ascii_digit() || " +()-".contains(c)))
}

/// E-posta ya da KEP adresi (isteğe bağlı): boş ya da tek @, iki yanı dolu, alan adında nokta; boşluk ve köşeli parantez yok.
fn eposta_ok(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let Some((ad, alan)) = s.split_once('@') else { return false };
    s.len() <= 100
        && !ad.is_empty()
        && alan.contains('.')
        && !alan.starts_with('.')
        && !alan.ends_with('.')
        && !alan.contains('@')
        && s.chars().all(|c| c.is_ascii_graphic() && !"[]<>\"\\,;".contains(c))
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

const ALANLAR: &[Alan] = &[
    Alan { key: "main.unvan", label: "İşletme unvanı (vergi levhasındaki unvan yazılmalıdır)", tur: Tur::Metin(unvan_ok), grup: "İşletme", admin: false },
    Alan { key: "main.adres", label: "Veri sorumlusunun adresi (KVKK aydınlatma metninde başvuru adresi)", tur: Tur::Metin(adres_ok), grup: "İşletme", admin: false },
    Alan { key: "main.telefon", label: "İrtibat telefonu (isteğe bağlı; aydınlatma metninde)", tur: Tur::Metin(irtibat_tel_ok), grup: "İşletme", admin: false },
    Alan { key: "main.eposta", label: "E-posta (isteğe bağlı; aydınlatma metninde)", tur: Tur::Metin(eposta_ok), grup: "İşletme", admin: false },
    Alan { key: "main.kep", label: "KEP adresi (isteğe bağlı; aydınlatma metninde)", tur: Tur::Metin(eposta_ok), grup: "İşletme", admin: false },
    Alan { key: "main.site_name", label: "İşletme adı", tur: Tur::Metin(|s| !s.trim().is_empty() && chars_ok(s, 40, |c| c.is_alphanumeric() || " .-'&".contains(c))), grup: "İşletme", admin: false },
    Alan { key: "main.isletme_turu", label: "İşletme türü", tur: Tur::Secim(crate::ayar::ISLETME_TURLERI), grup: "İşletme", admin: false },
    Alan { key: "main.session_minutes", label: "Oturum süresi (dakika; 43200 = 30 gün)", tur: Tur::Sayi(30, 64800), grup: "İşletme", admin: false },
    Alan { key: "main.max_devices_per_phone", label: "Bir telefona en fazla cihaz", tur: Tur::Sayi(1, 10), grup: "İşletme", admin: false },
    Alan { key: "giris.misafir_kodu", label: "Misafir kodu ile giriş (personelin ürettiği kodla; T.C. kimlik ya da pasaport)", tur: Tur::Evet, grup: "Giriş yöntemleri", admin: false },
    Alan { key: "limits.sms_per_phone_15min", label: "Numara başına SMS / 15 dk", tur: Tur::Sayi(1, 20), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_per_phone_day", label: "Numara başına SMS / gün", tur: Tur::Sayi(1, 50), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_per_mac_hour", label: "Cihaz başına SMS / saat", tur: Tur::Sayi(1, 50), grup: "SMS sınırları", admin: true },
    Alan { key: "limits.sms_global_day", label: "Günlük toplam SMS tavanı", tur: Tur::Sayi(1, 100_000), grup: "SMS sınırları", admin: true },
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

/// Binlik nokta ("12.345.678").
fn binlik(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// bayt → MB (10⁶), tam sayı, binlik nokta ("4.820")
fn mb(bayt: u64) -> String {
    binlik((bayt + 500_000) / 1_000_000)
}

/// bayt → GB (10⁹), üç anlamlı basamak, ondalık virgül ("0,04", "3,46", "12,3", "125", "1.235")
fn gb(bayt: u64) -> String {
    if bayt == 0 {
        return "0".into();
    }
    // ondalık basamak yuvarlanmış değere göre: 9,995 GB → "10,0", 99,95 GB → "100"
    let basamak: u32 = if bayt < 9_995_000_000 {
        2
    } else if bayt < 99_950_000_000 {
        1
    } else {
        0
    };
    let birim = 10u64.pow(9 - basamak);
    let n = (bayt + birim / 2) / birim;
    let kat = 10u64.pow(basamak);
    if basamak == 0 { binlik(n) } else { format!("{},{:0w$}", binlik(n / kat), n % kat, w = basamak as usize) }
}

/// Ölçüm bundan eskiyse kaydedici durmuş sayılır (5 sn'de bir yazar).
const OLCUM_ESKI_SN: f64 = 30.0;
/// Yenileme açıldıktan sonra kendiliğinden gelen (oto=1) bakışlar bu süre denetime yazılmaz.
const YENILE_PENCERE_SN: f64 = 900.0;

/// Bağlı kullanıcılar sıralaması: son görülme (varsayılan), bugün MB. Eski `saat` ve bilinmeyen değer varsayılana düşer.
fn sira(req: &Req) -> &'static str {
    match req.query.get("sira").map(String::as_str) {
        Some("mb") => "mb",
        _ => "son",
    }
}

/// Bağlı kullanıcılar tablosunda sayfa başına satır.
const OTURUM_SAYFA: usize = 10;

/// İstenen sayfa (1'den başlar); geçersizse 1. Üst sınır tabloyu çizerken uygulanır.
fn sayfa_no(req: &Req) -> usize {
    req.query.get("sayfa").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1)
}

fn sorgu_bir(req: &Req, k: &str) -> bool {
    req.query.get(k).is_some_and(|v| v == "1")
}

/// Bağlı kullanıcılar adresi; grup sayfasında bölüme atlar.
/// Yenileme açıkken yalnızca bağlı kullanıcılar bölümü tek başına açılır (/oturumlar): 10 sn'lik yenileme aynı sayfadaki
/// Yasaklı/İzinli cihaz formlarına yazılanı silmesin. Yenileme kapalıyken tam Kullanıcılar sayfası.
/// 1. sayfada `sayfa` yazılmaz.
fn oturumlar_url(sira: &str, sayfa: usize, yenile: bool, oto: bool) -> String {
    let s = if sayfa > 1 { format!("&sayfa={sayfa}") } else { String::new() };
    if yenile {
        format!("/oturumlar?sira={sira}{s}&yenile=1{}", if oto { "&oto=1" } else { "" })
    } else {
        format!("/cihazlar?sira={sira}{s}#oturumlar")
    }
}

/// Yenileme açıkken yalnızca bağlı kullanıcılar sayfası 10 sn'de bir kendini yeniler (betik yok; CSP).
fn yenile_meta(req: &Req) -> String {
    // AP kurulumu → takılan cihaz aranırken (arka plan işi); bitince sayfa sonuca yönlendirir
    if req.path == "/ap-kurulum/bul" {
        return "<meta http-equiv=\"refresh\" content=\"2;url=/ap-kurulum/bul\">".into();
    }
    if !matches!(req.path.as_str(), "/cihazlar" | "/oturumlar") || !sorgu_bir(req, "yenile") {
        return String::new();
    }
    format!("<meta http-equiv=\"refresh\" content=\"10;url={}\">", h(&oturumlar_url(sira(req), sayfa_no(req), true, true)))
}

/// Bugün MB hücresi; ölçüm yoksa tire.
fn olcum_hucreleri(c: Option<&crate::trafik::Cihaz>) -> String {
    c.map_or_else(|| "—".to_string(), |c| mb(c.bugun_bayt))
}
fn get_field(c: &Config, key: &str) -> String {
    match key {
        "main.site_name" => c.main.site_name.clone(),
        "main.unvan" => c.main.unvan.clone(),
        "main.adres" => c.main.adres.clone(),
        "main.telefon" => c.main.telefon.clone(),
        "main.eposta" => c.main.eposta.clone(),
        "main.kep" => c.main.kep.clone(),
        // seçilmemiş (yok ya da tanınmayan) tür boş döner: formda "Seçin" görünür, kaydetmek türü kendiliğinden yazmaz
        "main.isletme_turu" => crate::ayar::isletme_turu_secili(c).unwrap_or_default().to_string(),
        "giris.misafir_kodu" => b(c.giris.misafir_kodu),
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
        "main.adres" => c.main.adres = v.split_whitespace().collect::<Vec<_>>().join(" "),
        "main.telefon" => c.main.telefon = v.trim().into(),
        "main.eposta" => c.main.eposta = v.trim().into(),
        "main.kep" => c.main.kep = v.trim().into(),
        "main.isletme_turu" => c.main.isletme_turu = Some(v.into()),
        "giris.misafir_kodu" => c.giris.misafir_kodu = v == "1",
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
        "backup.enabled" => c.backup.enabled = v == "1",
        "backup.target" => c.backup.target = v.into(),
        "backup.ssh" => c.backup.ssh = v.into(),
        _ => {}
    }
}

/// Panelden değiştirilemeyen alan ve nedeni: bağlı cihazda uzak erişim (merkez bağlanırken yazar; tünel üzerinden kapatılırsa
/// panel kendi erişimini keserdi).
fn salt_okunur(a: &Alan, bagli: bool) -> Option<&'static str> {
    if bagli && a.key.starts_with("uzak.") {
        Some("merkez bağlantısıyla kuruldu")
    } else {
        None
    }
}

/// Alan bu işletme türünde sayfada var mı (giriş yöntemleri türe göre görünür). Görünmeyen alan formdan değişmez.
fn gorunur(a: &Alan, cfg: &Config) -> bool {
    let tur = crate::ayar::isletme_turu(cfg);
    crate::ayar::GirisYontemi::HEPSI.iter().find(|y| a.key.strip_prefix("giris.") == Some(y.kimlik())).is_none_or(|y| y.gorunur(tur))
}

/// Ayarlar sayfasının (admin=false) ya da Admin ayarları sayfasının (admin=true) alanları.
fn fields(admin: bool) -> impl Iterator<Item = &'static Alan> {
    ALANLAR.iter().filter(move |a| a.admin == admin)
}

/// Formdan değişiklikler: (anahtar, eski, yeni). Sayfada olmayan alanlar yok sayılır.
fn validate(admin: bool, form: &Form, cfg: &Config, bagli: bool) -> Result<Vec<(&'static str, String, String)>, HashMap<&'static str, String>> {
    let mut changes = vec![];
    let mut errors = HashMap::new();
    for a in fields(admin) {
        if salt_okunur(a, bagli).is_some() || !gorunur(a, cfg) {
            continue;
        }
        let old = get_field(cfg, a.key);
        let raw = form.get(a.key).map(|s| s.trim().to_string());
        let new = match a.tur {
            Tur::Evet => b(raw.as_deref() == Some("1")),
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
                // hiç seçilmemiş alan ("Seçin") seçilmemiş kalabilir
                Some(v) if v.is_empty() && old.is_empty() => continue,
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
    format!("{key}: {old} → {new}")
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
    /// Hazır engel listeleri: indirilen listeler ve birleşik dnsmasq dosyası (testte geçici)
    liste_yollar: crate::liste::Yollar,
    /// WireGuard gizli anahtarı (testte geçici)
    uzak_key: std::path::PathBuf,
    /// Yedek SSH anahtarı (testte geçici)
    yedek_key: std::path::PathBuf,
    /// Yönetim merkezi bağı (testte geçici)
    pub merkez_path: std::path::PathBuf,
    /// Kaydedicinin 5 sn'de bir yazdığı hız / bugünkü kullanım ölçümü (testte geçici)
    trafik_path: std::path::PathBuf,
    /// Yönetim merkezi HTTP istemcisi (testte sahte)
    pub http: Box<crate::merkez::Http>,
    /// İlk bağlanma tek seferde (iki eşzamanlı giriş iki ayrı cihaz anahtarı almasın)
    baglan_kilit: std::sync::Mutex<()>,
    /// Raporlar için merkez istemcisi (kısa süreli; testte sahte)
    pub rapor_http: Box<crate::merkez::Http>,
    /// Merkezden alınan rapor cevapları: anahtar → (geçerlilik sonu, cevap)
    rapor_onbellek: std::sync::Mutex<HashMap<String, (f64, rapor_sayfalari::Sonuc)>>,
    /// AP kurulumu yan oda durum dosyası (testte geçici)
    ap_yollar: crate::ap::Yollar,
    /// Canlı komut çıktısı (komşu tablosu, aracı birimi durumu, DHCP denetimi; testte sahte)
    ip_oku: Box<dyn Fn(&[&str]) -> String + Send + Sync>,
    /// Süre sınırlı canlı komut çıktısı (PPPoE durumu: `ip`, `journalctl`; `None`: çalışmadı / zaman aşımı; testte sahte)
    komut: Box<dyn Fn(&[&str]) -> Option<String> + Send + Sync>,
    /// Etkin internet yoklaması (PPPoE durumu: adres var ama internete çıkılıyor mu; `ag::internet_yoklamasi`, testte sahte)
    internet: Box<dyn Fn() -> bool + Send + Sync>,
    /// Yabancı DHCP denetimi sürüyor: aynı anda tek denetim (panelin iş parçacıkları bağlanmasın)
    dhcp_denetim: std::sync::atomic::AtomicBool,
    /// "Bağlan" / "Otomatik bul" başlatma tek seferde (çift tıklamada ikinci istek koşan aramanın durumunu ezmesin)
    ap_bul_kilit: std::sync::Mutex<()>,
    /// AP arayüzü: hedefte web arayüzü var mı (adres, yan odadan mı, yalnızca HTTP mi) → port (443'te TLS el sıkışması
    /// olursa 443, yoksa 80; `ap_arayuz::web_portu`); testte sahte
    ap_dene: Box<dyn Fn(std::net::Ipv4Addr, bool, bool) -> Option<u16> + Send + Sync>,
    /// AP arayüzü aktarımında AP'ye bağlantı (yan oda aracısı ya da misafir ağı; testte sahte)
    ap_baglan: Box<crate::ap_arayuz::Baglanti>,
}

const MENU: &[(&str, &str)] = &[("/", "Özet"), ("/cihazlar", "Kullanıcılar"), ("/misafir-kodlari", "Misafir kodları"), ("/kayitlar", "Kayıtlar"), ("/ayarlar", "Ayarlar")];

/// Admin sol menüde değil, Ayarlar sayfasının en altında kutucuk (2026-10-13, kullanıcı isteği); /admin sayfası aynen kalır.
const ADMIN_YOLU: &str = "/admin";

/// Menü grupları tek sayfadır; eski sayfalar bu sayfada alt alta bölüm olur (2026-10-08, kullanıcı isteği).
/// (grup, eski yol, bölüm id, başlık, yalnızca admin). Eski yollar ayrıca açılır (hata/arama sonucu, ayrıntı).
const BOLUMLER: &[(&str, &str, &str, &str, bool)] = &[
    ("/cihazlar", "/oturumlar", "oturumlar", "Bağlı kullanıcılar", false),
    ("/cihazlar", "/yasak", "yasak", "Yasaklı cihazlar", false),
    ("/cihazlar", "/izinli", "izinli", "İzinli cihazlar", false),
    ("/cihazlar", "/yavaslatilmis", "yavaslatilmis", "Yavaşlatılmış cihazlar", false),
    ("/cihazlar", "/yasakli-siteler", "yasakli-siteler", "Yasaklı siteler", false),
    ("/kayitlar", "/kayitlar", "gunler", "Kayıtlar", false),
    ("/kayitlar", "/ara", "site-ara", "Site / IP arama", false),
    ("/kayitlar", "/kullanicilar", "kullanicilar", "Kullanıcılar", false),
    ("/kayitlar", "/talep", "talep", "Resmi talep", false),
    ("/ayarlar", "/ayarlar", "isletme", "İşletme", false),
    ("/ayarlar", "/portal-metinleri", "portal-metinleri", "Portal metinleri", false),
    ("/ayarlar", "/portlar", "portlar", "Portlar", false),
    ("/ayarlar", "/ap-kurulum", "ap-kurulum", "AP kurulumu", false),
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
        "/raporlar" => return "/ayarlar",
        p if p.starts_with("/rapor/") => return "/ayarlar",
        p if p == "/misafir-kodlari" || p.starts_with("/misafir-kodlari/") => return "/misafir-kodlari",
        _ => {}
    }
    BOLUMLER
        .iter()
        .find(|(g, y, ..)| path == *g || path == *y || path.strip_prefix(y).is_some_and(|r| r.starts_with('/')))
        .map_or("/", |(g, ..)| g)
}

// çizgi simgeleri (24×24, stroke): sol menü (2026-10-29 yeni tasarım; önce yönetimdeki gibi yalnızca yazıydı), telefondaki
// menü düğmesi ve grup sayfalarının bölüm düğmeleri
const SIMGE_PANO: &str = "M3 3h7v9H3zM14 3h7v5h-7zM14 12h7v9h-7zM3 16h7v5H3z";
const SIMGE_BILET: &str = "M4 6h16a1 1 0 0 1 1 1v3a2 2 0 0 0 0 4v3a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-3a2 2 0 0 0 0-4V7a1 1 0 0 1 1-1zM14 7v2M14 11v2M14 15v2";
const SIMGE_ARSIV: &str = "M3 4h18v4H3zM5 8v12h14V8M10 12h4";
const SIMGE_WIFI: &str = "M2 8.5a15 15 0 0 1 20 0M5 12a10 10 0 0 1 14 0M8.5 15.5a5 5 0 0 1 7 0M12 19h.01";
const SIMGE_KAYIT: &str = "M6 2h9l5 5v15H6zM14 2v6h6M9 13h8M9 17h8";
const SIMGE_AYAR: &str = "M4 6h16M4 12h16M4 18h16M9 4v4M15 10v4M7 16v4";
const SIMGE_KILIT: &str = "M6 11h12v10H6zM8 11V7a4 4 0 0 1 8 0v4";
const SIMGE_ARA: &str = "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM16 16l5 5";
const SIMGE_YASAK: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM5.6 5.6l12.8 12.8";
const SIMGE_CIKIS: &str = "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9";
const SIMGE_GERI: &str = "M19 12H5M12 19l-7-7 7-7";
const SIMGE_OK: &str = "M6 9l6 6 6-6";

fn simge(d: &str) -> String {
    format!("<svg class=\"simge\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"{d}\"/></svg>")
}

/// Fotoğrafı olmayan kişinin yuvarlak avatarındaki dolu silüet (Özet listeleri; WhatsApp'taki gibi gri zeminde beyaz)
const KISI_AVATAR: &str = "<span class=\"avatar kisi\" aria-hidden=\"true\"><svg viewBox=\"0 0 40 40\"><circle cx=\"20\" cy=\"15.5\" r=\"7.5\"/><path d=\"M5.5 40c.8-9.3 6.8-14.5 14.5-14.5S33.7 30.7 34.5 40z\"/></svg></span>";

/// Bağlı kullanıcılar listesinde bir kişi (2026-10-30 kullanıcı: "liste şeklinde olsun yine ama açılır kartlar olsun ok ile"):
/// kapalıyken ad, son görülme, bugünkü MB ve durum rozetleri; ok ile açılınca bilgiler (telefon, MAC, IP) ve düğmeler. JS yok.
/// `durum`, `bilgi` değerleri ve `eylem` güvenli HTML (çağıran kaçışlar); `ad`, `son`, `bugun` düz metin.
fn kisi_ogesi(ad: &str, son: &str, bugun: &str, durum: &str, bilgi: &[(&str, String)], eylem: &str) -> String {
    let bugun = if bugun == "—" { "—".to_string() } else { format!("{bugun} MB") };
    format!(
        "<li class=\"kisi\"><details><summary>{KISI_AVATAR}<span class=\"satir-metin\"><b>{}</b><small>Son görülme {} · Bugün {}</small></span>\
         <span class=\"kisi-durum\">{durum}</span><span class=\"ok\">{}</span></summary>\
         <div class=\"kisi-ic\">{}<div class=\"eylemler\">{eylem}</div></div></details></li>",
        h(ad),
        h(son),
        h(&bugun),
        simge(SIMGE_OK),
        facts(bilgi)
    )
}

/// Sayfanın içerik düzeni (2026-10-30 WhatsApp Web benzeri panel)
enum Duzen<'a> {
    /// başlık çubuklu tek panel (ayrıntı sayfaları, formlar)
    Tek(&'a str),
    /// solda liste paneli (başlığı kendinde), sağda ayrıntı paneli; `sinif` dış kutuya eklenir (ör. "bolumlu", "ozet")
    Ikili { liste: &'a str, ayrinti: &'a str, sinif: &'a str },
}

/// İşletme adının baş harfleri (şeritteki avatar): ilk iki sözcüğün ilk harfleri, Türkçe büyük harfle ("Bocafe Göztepe" → "BG")
fn bas_harfler(ad: &str) -> String {
    ad.split_whitespace()
        .filter_map(|s| s.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .map(|c| match c {
            'i' => "İ".to_string(),
            'ı' => "I".to_string(),
            c => c.to_uppercase().collect(),
        })
        .collect()
}

/// Grup sayfasının bölüm listesinde başlığın altındaki kısa açıklama (bölüm id'sine göre)
fn bolum_aciklamasi(id: &str) -> &'static str {
    match id {
        "oturumlar" => "Ağdaki misafirler, hız ve bağlantı",
        "yasak" => "Ağa hiç giremeyen cihazlar",
        "izinli" => "SMS'siz internete çıkan cihazlar",
        "yavaslatilmis" => "Hızı sınırlanan cihazlar",
        "yasakli-siteler" => "Engellenen siteler ve kelimeler",
        "gunler" => "Günlük 5651 kayıtları ve indirme",
        "site-ara" => "Bir siteye kimin girdiğini bulun",
        "kullanicilar" => "Misafirler ve kişi kayıtları",
        "talep" => "Resmi talep için kayıt arama",
        "isletme" => "Unvan, adres, oturum süresi",
        "portal-metinleri" => "Giriş sayfasındaki yasal metinler",
        "portlar" => "İnternet, PPPoE ve Wi-Fi",
        "ap-kurulum" => "Erişim noktasını bulun ve yönetin",
        "sistem" => "Servisler, mühür ve doğrulama",
        "sifre-degistir" => "Panel parolanız",
        "admin-ayarlari" => "SMS sınırları, yedek, uzak erişim",
        "panel-hareketleri" => "Panel hesaplarının yaptıkları",
        _ => "",
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
const SIMGE_HIZ: &str = "M4 15a8 8 0 0 1 16 0M12 15l4-5";
const SIMGE_RAPOR: &str = "M4 20h16M7 16v-5M12 16V6M17 16v-8";

/// Sol menü bağlantısının simgesi (menü yoluna göre)
fn menu_simgesi(yol: &str) -> &'static str {
    match yol {
        "/cihazlar" => SIMGE_KISILER,
        "/misafir-kodlari" => SIMGE_BILET,
        "/kayitlar" => SIMGE_ARSIV,
        "/ayarlar" => SIMGE_AYAR,
        _ => SIMGE_PANO,
    }
}

/// Grup sayfasının üstündeki bölüm kutucuklarının simgesi (bölüm id'sine göre)
fn bolum_simgesi(id: &str) -> &'static str {
    match id {
        "oturumlar" => SIMGE_WIFI,
        "yasak" => SIMGE_YASAK,
        "izinli" => SIMGE_TAMAM,
        "yavaslatilmis" => SIMGE_HIZ,
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
        "ap-kurulum" => SIMGE_WIFI,
        "panel-hareketleri" => SIMGE_GOZ,
        _ => SIMGE_KAYIT,
    }
}

/// Özet listelerinde en çok bu kadar satır; fazlası "Tümü" bağlantısıyla
const OZET_SATIR: usize = 10;

/// "2026-10-10T14:02:11+03:00" → bugünse "14:02", değilse "09.10.2026 14:02"
fn kisa_zaman(iso: &str, bugun: &str) -> String {
    match (iso.get(..10), iso.get(11..16)) {
        (Some(g), Some(saat)) if g == bugun => saat.into(),
        (Some(_), Some(_)) => ortak::tarih_goster(iso),
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
            liste_yollar: crate::liste::Yollar::sistem(),
            uzak_key: crate::uzak::KEY.into(),
            yedek_key: crate::uzak::YEDEK_KEY.into(),
            merkez_path: crate::merkez::PATH.into(),
            trafik_path: crate::trafik::DURUM_YOLU.into(),
            http: Box::new(crate::merkez::curl),
            baglan_kilit: std::sync::Mutex::new(()),
            rapor_http: Box::new(|yol: &str, govde: &str| crate::merkez::curl_sure(yol, govde, rapor_sayfalari::MERKEZ_SURE)),
            rapor_onbellek: std::sync::Mutex::new(HashMap::new()),
            ap_yollar: crate::ap::Yollar::sistem(),
            ip_oku: Box::new(|c: &[&str]| ortak::capture(c)),
            komut: Box::new(|c: &[&str]| ortak::capture_timeout(c, 5)),
            internet: Box::new(crate::ag::internet_yoklamasi),
            dhcp_denetim: std::sync::atomic::AtomicBool::new(false),
            ap_bul_kilit: std::sync::Mutex::new(()),
            ap_dene: Box::new(crate::ap_arayuz::web_portu),
            ap_baglan: Box::new(crate::ap_arayuz::baglan),
        }
    }

    fn page(&self, cfg: &Config, req: &Req, o: Option<&Oturum>, title: &str, body: &str) -> Resp {
        self.page_ek(cfg, req, o, title, body, "")
    }

    /// `page`, başa (head) ek: ör. AP arayüzüne geçişin meta yenilemesi
    fn page_ek(&self, cfg: &Config, req: &Req, o: Option<&Oturum>, title: &str, body: &str, bas: &str) -> Resp {
        self.cerceve(cfg, req, o, title, Duzen::Tek(body), bas)
    }

    /// Sayfa çerçevesi (2026-10-30, kullanıcı isteği: "arayüzü WhatsApp Web gibi yap"): solda dar simge şeridi (üstte W_ işareti,
    /// altta işletmenin baş harfleri ve çıkış), içerik tek panel ya da liste + ayrıntı. Telefonda şerit alt sekme çubuğu olur,
    /// üstte logo ve çıkışlı ince şerit. Oturumsuz sayfada beyaz şeritte logo, şerit yok.
    fn cerceve(&self, cfg: &Config, req: &Req, o: Option<&Oturum>, title: &str, duzen: Duzen, bas: &str) -> Resp {
        if PARCA.with(std::cell::Cell::get) {
            let body = match duzen {
                Duzen::Tek(b) => b.to_string(),
                Duzen::Ikili { liste, ayrinti, .. } => format!("{liste}{ayrinti}"),
            };
            return Resp { status: 200, body, headers: vec![], file: None };
        }
        // cihaz bir müşteriye bağlanmadan işletme adı yok (eski/örnek ayardaki ad görünmesin): sekme başlığında "WifiCorrect",
        // şeritte ad yok (müşteri numarası hiç gösterilmez)
        let bagli = crate::merkez::oku(&self.merkez_path).is_some();
        let (ust, yan) = match o {
            Some(o) => {
                let grup = match menu_grubu(&req.path) {
                    ADMIN_YOLU => "/ayarlar",
                    g => g,
                };
                let links: String = MENU
                    .iter()
                    .filter(|(p, _)| *p != "/misafir-kodlari" || crate::ayar::misafir_kodu_acik(cfg))
                    .map(|(p, l)| {
                        let act = if *p == grup { " class=\"aktif\" aria-current=\"page\"" } else { "" };
                        format!("<a href=\"{p}\"{act} title=\"{l}\">{}<span>{l}</span></a>", simge(menu_simgesi(p)), l = h(l))
                    })
                    .collect();
                let ad = cfg.main.site_name.trim();
                let isletme = if bagli && !ad.is_empty() {
                    format!("<div class=\"yan-isletme\" title=\"{a}\"><span class=\"avatar\" aria-hidden=\"true\">{}</span><span class=\"gizli\">{a}</span></div>", h(&bas_harfler(ad)), a = h(ad))
                } else {
                    String::new()
                };
                let cikis = format!(
                    "<form method=\"post\" action=\"/cikis\">{}<button class=\"cikis\" title=\"Çıkış\">{}<span>Çıkış</span></button></form>",
                    csrf_input(o),
                    simge(SIMGE_CIKIS)
                );
                let ust_ad = if bagli && !ad.is_empty() { format!("<span class=\"ust-isletme\">{}</span>", h(ad)) } else { String::new() };
                (
                    format!("<header class=\"ust-mobil\">{}{ust_ad}{cikis}</header>", crate::logo::marka_acik()),
                    format!(
                        "<aside class=\"yan\">{}<nav aria-label=\"Menü\">{links}</nav><div class=\"yan-alt\">{isletme}{cikis}</div></aside>\n",
                        crate::logo::isaret()
                    ),
                )
            }
            None => (format!("<header class=\"ust-acik\">{}</header>", crate::logo::marka_acik()), String::new()),
        };
        let msg = req.query.get("m").map(|m| {
            let cls = if req.query.get("e").is_some_and(|e| e == "1") { "mesaj hata" } else { "mesaj" };
            format!("<div class=\"{cls}\">{}</div>", h(m))
        });
        let mut v: HashMap<&str, String> = HashMap::new();
        v.insert("baslik", h(title));
        v.insert("favicon_html", crate::logo::favicon_html().to_string());
        v.insert("bas_ek", if o.is_some() { yenile_meta(req) + bas } else { bas.to_string() });
        v.insert("site", h(if bagli { &cfg.main.site_name } else { "WifiCorrect" }));
        v.insert("govde", if o.is_some() { String::new() } else { "yalin".into() });
        v.insert("ust_html", ust);
        v.insert("yan_html", yan);
        let mut uyari = String::new();
        if o.is_some() {
            let now = (self.clock)();
            match crate::merkez::oku(&self.merkez_path) {
                None => uyari.push_str("<div class=\"mesaj hata\">Cihaz bir müşteriye bağlı değil: misafirlere internet verilmiyor. Müşteri numarasıyla giriş yapılınca açılır.</div>"),
                Some(m) if !crate::merkez::lisans_aktif(&m, now) => uyari.push_str(&format!(
                    "<div class=\"mesaj hata\">{}: misafirlere internet verilmiyor. Hizmet sağlayıcınıza başvurun.</div>",
                    if m.lisans == "askida" { "Lisans askıya alındı".to_string() } else { format!("Lisans süresi doldu ({})", h(&ortak::tarih_goster(&m.lisans_bitis))) }
                )),
                Some(_) => {}
            }
            let eksik: Vec<&str> = [("unvan", &cfg.main.unvan), ("adres", &cfg.main.adres)].iter().filter(|(_, v)| v.trim().is_empty()).map(|(k, _)| *k).collect();
            if !eksik.is_empty() && crate::merkez::oku(&self.merkez_path).is_some() {
                uyari.push_str(&format!(
                    "<div class=\"mesaj hata\">KVKK metninde veri sorumlusu bilgileri eksik ({}): misafirlerin gördüğü aydınlatma ve açık rıza metinlerinde yer tutucu görünüyor. Ayarlar → İşletme'den girin.</div>",
                    eksik.join(", ")
                ));
            }
        }
        let mesaj = uyari + &msg.unwrap_or_default();
        // tek panel: başlık çubuğu + gövde (mesajlar gövdenin başında); iki panel: solda liste (başlığı kendinde), sağda ayrıntı
        // (mesajlar ayrıntının başında)
        let icerik = match duzen {
            Duzen::Tek(body) => format!(
                "<div class=\"bolme tek\"><header class=\"bolme-ust\"><h1>{}</h1></header><div class=\"bolme-govde\">{mesaj}<div class=\"icerik-kart\">{body}</div></div></div>",
                h(title)
            ),
            Duzen::Ikili { liste, ayrinti, sinif } => {
                format!("<div class=\"ikili {sinif}\"><div class=\"liste-bolme\">{liste}</div><div class=\"ayrinti-bolme\">{mesaj}{ayrinti}</div></div>")
            }
        };
        v.insert("icerik_html", icerik);
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
                "/yavaslatilmis" => self.yavaslatilmis(cfg, req, o, now).body,
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
                "/ap-kurulum" => self.ap_kurulum(cfg, req, o, now).body,
                _ => self.sifre(cfg, req, o, "").body,
            })
            .collect();
        drop(parca);
        // WhatsApp Web gibi (2026-10-30): solda bölümler sohbet listesi gibi (simge, başlık, kısa açıklama), sağda seçilen bölüm
        // başlık çubuğuyla. Seçim adres çubuğundaki #bölüm'le (CSS :target, betik yok); hedef yoksa ilk bölüm. Form dönüşleri ve
        // sıralama bağlantıları zaten #bölüm taşır. Telefonda önce liste, bölüme girince ayrıntı ve geri oku.
        let satir = |href: &str, ikon: &str, baslik: &str, aciklama: &str| {
            format!(
                "<a class=\"sohbet\" href=\"{href}\"><span class=\"avatar\">{}</span><span class=\"satir-metin\"><b>{}</b><small>{}</small></span></a>",
                simge(ikon),
                h(baslik),
                h(aciklama)
            )
        };
        let mut satirlar: String = bolumler.iter().map(|(_, _, id, baslik, _)| satir(&format!("#{id}"), bolum_simgesi(id), baslik, bolum_aciklamasi(id))).collect();
        if req.path == "/ayarlar" {
            satirlar.push_str(&satir("/raporlar", SIMGE_RAPOR, "Raporlar", "Haftalık ve aylık raporlar"));
            satirlar.push_str(&satir(ADMIN_YOLU, SIMGE_KILIT, "Admin", "Hizmet sağlayıcının ayarları"));
        }
        let baslik = if req.path == ADMIN_YOLU { "Admin" } else { MENU.iter().find(|(p, _)| *p == req.path).map_or("", |(_, l)| l) };
        let liste = format!("<header class=\"bolme-ust\"><h1>{}</h1></header><nav class=\"kisayol\" aria-label=\"Bu sayfadaki bölümler\">{satirlar}</nav>", h(baslik));
        // seçili satır: hedeflenen bölüm ya da içinde hedeflenen bir alan olan bölüm (ör. #main-isletme_turu)
        let mut ayrinti = String::from("<style>");
        for (_, _, id, ..) in &bolumler {
            ayrinti.push_str(&format!(".ikili:has(#{id}:target,#{id} :target) .sohbet[href=\"#{id}\"]{{background:var(--secili)}}"));
        }
        ayrinti.push_str("</style>");
        for ((_, _, id, baslik, _), govde) in bolumler.iter().zip(&govdeler) {
            ayrinti.push_str(&format!(
                "<section class=\"bolum\" id=\"{id}\" aria-labelledby=\"b-{id}\"><h2 class=\"bolum-baslik\" id=\"b-{id}\"><a class=\"geri\" href=\"#\" title=\"Bölümler\">{}<span class=\"gizli\">Bölümlere dön</span></a><span class=\"avatar\">{}</span>{}</h2><div class=\"bolum-govde\">{govde}</div></section>",
                simge(SIMGE_GERI),
                simge(bolum_simgesi(id)),
                h(baslik)
            ));
        }
        self.cerceve(cfg, req, Some(o), baslik, Duzen::Ikili { liste: &liste, ayrinti: &ayrinti, sinif: "bolumlu" }, "")
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
        let merkezden = self.merkezden(&cfg, &req.ip);
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
        // AP arayüzü geçişi merkezindir; merkez eski sürümdeyse istek buraya gelir (spec 2026-10-16-ap-arayuz)
        if req.path == "/_ap-gecis" {
            return ap_aktarim::gecis_yok();
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

    /// Kimlik başlıkları (X-WFC-Kullanici, X-WFC-AP, X-WFC-AP-Kullanici) yalnızca açık tünelden, merkezin tünel adresinden gelirse geçerli;
    /// başka kaynakta yok sayılır. Tünel kapalıysa (ayar ya da "wfc" arayüzü yok) 10.99.0.1'e yanıt varsayılan yoldan
    /// gider: kaynak taklit edilebilir.
    fn merkezden(&self, cfg: &Config, ip: &str) -> bool {
        let tunel = cfg.uzak.enabled && crate::uzak::adres_ok(cfg.uzak.adres.trim()) && self.sys.join("wfc").exists();
        tunel && merkez_adresi(cfg).is_some_and(|m| m == ip)
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
            ("POST", "/oturumlar/yavaslat") => self.yavaslat(&cfg, req, &o, now),
            ("POST", "/oturumlar/yavaslatma-kaldir") => self.yavaslatma_kaldir(&cfg, req, &o),
            ("GET", "/yavaslatilmis") => self.yavaslatilmis(&cfg, req, &o, now),
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
            // AP kurulumu: işletme sahibi de kullanır (2026-10-15 kullanıcı kararı; admin kilidi yok, CSRF ve denetim var)
            ("GET", "/ap-kurulum") => self.ap_kurulum(&cfg, req, &o, now),
            ("GET", "/ap-kurulum/cihazlar") => self.ap_cihazlar(&cfg, req, &o),
            ("POST", "/ap-kurulum/gecici-ekle") => self.ap_gecici_ekle(&cfg, req, &o, now),
            ("POST", "/ap-kurulum/gecici-kaldir") => self.ap_gecici_kaldir(&cfg, req, &o),
            ("POST", "/ap-kurulum/adresler") => self.ap_adres_kaydet(&cfg, req, &o),
            ("POST", "/ap-kurulum/dhcp-denetle") => self.ap_dhcp_denetle(&cfg, req, &o),
            ("POST", "/ap-kurulum/baglan") => self.ap_baglan(&cfg, req, &o, now),
            ("POST", "/ap-kurulum/otomatik") => self.ap_bul_baslat(&cfg, req, &o, None, now),
            ("GET", "/ap-kurulum/bul") => self.ap_bul_sayfa(&cfg, req, &o),
            ("POST", "/ap-kurulum/arayuz") => self.ap_arayuz_ac(&cfg, req, &o, now),
            ("POST", "/ap-kurulum/arayuz-kapat") => self.ap_arayuz_kapat(&cfg, req, &o),
            ("POST", "/admin-kilidi") => self.admin_kilidi(&cfg, req, &o, now),
            ("POST", "/admin-kilidi/kapat") => {
                if let Some(t) = &req.token {
                    self.oturumlar.set_admin(t, 0.0, "");
                }
                self.audit(&cfg, req, Some(&o), "PANEL_ADMIN_KAPANDI", "");
                redirect("/", None)
            }
            ("GET", "/kayitlar/gun") => self.kayit_gun(&cfg, req, &o),
            ("GET" | "POST", "/kayitlar/dosya") => self.kayit_dosya(&cfg, req, &o),
            ("GET", "/kayitlar/indir") => self.kayit_indir(&cfg, req, &o),
            ("GET", "/kayitlar/gun-indir") => self.kayit_gun_indir(&cfg, req, &o),
            ("GET" | "POST", "/kullanicilar") => self.kullanicilar(&cfg, req, &o),
            ("GET", "/kullanici") => self.kullanici(&cfg, req, &o),
            ("GET" | "POST", "/talep") => self.talep(&cfg, req, &o, now),
            ("GET", "/ara") => self.site_ara(&cfg, req, &o, now),
            ("GET", "/talep/paket") => self.talep_paket(&cfg, req, &o),
            ("GET", "/yasakli-siteler") => self.filtre_sayfa(&cfg, req, &o),
            ("POST", "/yasakli-siteler/ekle") => self.filtre_degistir(cfg, req, &o, true),
            ("POST", "/yasakli-siteler/kaldir") => self.filtre_degistir(cfg, req, &o, false),
            ("POST", "/yasakli-siteler/hazir") => self.hazir_degistir(cfg, req, &o),
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
            (_, p) if p == "/misafir-kodlari" || p.starts_with("/misafir-kodlari/") => self.misafir_kodlari(&cfg, req, &o, now),
            ("GET", "/sifre") => self.sifre(&cfg, req, &o, ""),
            ("GET", "/raporlar") => self.raporlar(&cfg, req, &o, now),
            ("GET", p) if p.starts_with("/rapor/") => self.rapor(&cfg, req, &o, now),
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
             <div style=\"margin-top:20px\"><button>Giriş</button></div></form>\
             <p class=\"alt\">Kolay yol: bu cihazla aynı internetten <a href=\"https://cihaz.wificorrect.com\">cihaz.wificorrect.com</a> \
             adresine müşteri numaranız ve parolanızla girin; cihaz birkaç dakikada kendiliğinden bağlanır.</p></div>"
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
            // bağ kontrolü ve ilk bağlanma kilit altında: aynı anda iki giriş iki ayrı cihaz anahtarı almasın; dosya kilidi
            // kolay bağlamayla (`ctl bekliyor`, ayrı süreç) yarışmasın diye
            let _k = self.baglan_kilit.lock().unwrap_or_else(|e| e.into_inner());
            let _d = crate::merkez::kilit(&self.merkez_path);
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
        let (yeni, admin, sms) = crate::merkez::kur(cfg, &self.cfg_path, &self.merkez_path, &self.hesaplar, &g, &*self.runner).map_err(|e| (e, false))?;
        self.audit(&yeni, req, None, "PANEL_MERKEZ_BAGLANDI", &format!("numara={numara} tunel={}", g.tunel_ip));
        match admin {
            Ok(true) => self.audit(&yeni, req, None, "ADMIN_PAROLA_MERKEZ", ""),
            Ok(false) => {}
            Err(e) => self.audit(&yeni, req, None, "MERKEZ_EK_HATA", &e),
        }
        match sms {
            Ok(true) => self.audit(&yeni, req, None, "SMS_AYARI_MERKEZ", ""), // şifre yazılmaz; portal kur'da yeniden başladı
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
        // bağlı süresi kaydedicinin varlık kaydından (15 dk'dan kısa kopma süreyi sıfırlamaz)
        let varlik = ortak::varlik_oku(&m.state_root);
        let mut bagli: Vec<(ortak::Session, u64)> = ortak::load_sessions(&m.state_root)
            .into_iter()
            .filter(|(mac, s)| !s.ip.is_empty() && s.expires_epoch > now && agda.contains(mac))
            .map(|(mac, s)| {
                let dk = ortak::bagli_dakika(varlik.get(&mac), now);
                (s, dk)
            })
            .collect();
        bagli.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.start_epoch.total_cmp(&a.0.start_epoch))); // son bağlanan üstte
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
        // bütün cihazların bugün kullandığı veri (indirme + yükleme; kaydedici dakikada bir yazar); bugünün kaydı yoksa tire
        let veri = crate::trafik::bugun_toplam(&crate::trafik::gun_oku(&m.state_root), &today).map_or_else(|| "—".to_string(), |b| format!("{} GB", gb(b)));
        let sms = ortak::sms_count(&m.state_root, &today);
        let mut uyarilar = vec![];
        // SMS merkezden gider (spec 2026-10-13); deneme modu bilgisi merkezden gelir
        let sms_down = cfg.sms.mock;
        if o.rol == Rol::Hizmet {
            if cfg.sms.mock {
                uyarilar.push("SMS deneme modunda (yönetim merkezi): müşterilere gerçek SMS gitmiyor.".to_string());
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
        let mut uyari_li: String = uyarilar.iter().map(|u| format!("<li>{}</li>", h(u))).collect();
        if crate::ayar::isletme_turu_secili(cfg).is_none() {
            // seçilmemiş = anahtar yok (eski cihazlar) ya da tanınmayan değer; seçilince (kafe dahil) kalkar
            uyari_li.insert_str(0, "<li>İşletme türünüzü seçin: <a href=\"/ayarlar#main-isletme_turu\">Ayarlar → İşletme türü</a></li>");
        }
        let uyari_html = if uyari_li.is_empty() {
            String::new()
        } else {
            format!("<section class=\"kutu ozet-uyari\"><h2>Uyarılar</h2><ul class=\"uyarilar\">{uyari_li}</ul></section>")
        };
        // telefon, MAC ve IP yok: yalnızca ad soyad ve bağlı olduğu dakika
        let liste = |satirlar: Vec<String>, toplam: usize, bos: &str, tumu: &str| -> String {
            if satirlar.is_empty() {
                return format!("<p class=\"not\">{}</p>", h(bos));
            }
            let fazla = if toplam > OZET_SATIR { format!("<a class=\"tumu\" href=\"{tumu}\">Tümünü göster ({toplam})</a>") } else { String::new() };
            format!("<ul class=\"liste\">{}</ul>{fazla}", satirlar.concat())
        };
        // satırlar sohbet listesi gibi (2026-10-30 WhatsApp Web benzeri): avatar, ad, sağ üstte süre/saat, altında kısa durum
        let kisi = |ad: &str, sag: &str, alt: &str| {
            format!(
                "<li>{KISI_AVATAR}<span class=\"satir-metin\"><span class=\"satir-ust\"><b>{}</b><span class=\"zaman\">{}</span></span><small>{alt}</small></span></li>",
                h(ad),
                h(sag)
            )
        };
        let bagli_html = liste(
            bagli
                .iter()
                .take(OZET_SATIR)
                .map(|(s, dk)| kisi(&format!("{} {}", s.ad, s.soyad), &format!("{dk} dk"), "<i class=\"nokta\"></i>şu an bağlı"))
                .collect(),
            bagli.len(),
            "Şu an bağlı kullanıcı yok.",
            "/cihazlar#oturumlar",
        );
        let yeni_html = liste(
            yeni.iter().take(OZET_SATIR).map(|r| kisi(&format!("{} {}", r[1], r[2]), &kisa_zaman(&r[3], &today), "bugün ilk kez bağlandı")).collect(),
            yeni.len(),
            "Bugün yeni üye yok.",
            "/kayitlar#kullanicilar",
        );
        // solda bağlı kullanıcılar (sohbet listesi gibi); sağda başlık çubuğu, günün sayıları, uyarılar ve bugün yeni üyeler;
        // telefonda tek sütun (CSS): sayılar ve uyarılardan sonra bağlı kullanıcılar, en altta bugün yeni üyeler
        let sol = format!(
            "<header class=\"bolme-ust\"><h1>Özet</h1></header><section class=\"kutu sohbetler\"><h2>Bağlı kullanıcılar</h2>{bagli_html}</section>"
        );
        let sag = format!(
            "<header class=\"bolum-baslik\"><span class=\"avatar\">{}</span>Bugün<span class=\"ust-not\">{}</span></header>\
             <div class=\"bolum-govde\"><section class=\"pano\" aria-label=\"Bugünün özeti\"><dl class=\"sayilar\">\
             <div class=\"canli\"><dt>Bağlı kullanıcı</dt><dd>{}</dd></div>\
             <div><dt>Bugün farklı kullanıcı</dt><dd>{}</dd></div><div><dt>Bugün yeni üye</dt><dd>{}</dd></div>\
             <div><dt>Bugün kullanılan veri</dt><dd>{veri}</dd></div></dl></section>\
             {uyari_html}\
             <section class=\"kutu ozet-yeni\"><h2>Bugün yeni üyeler</h2>{yeni_html}</section></div>",
            simge(SIMGE_PANO),
            h(&ortak::tarih_goster(&today)),
            bagli.len(),
            kisiler.len(),
            yeni.len(),
        );
        self.cerceve(cfg, req, Some(o), "Özet", Duzen::Ikili { liste: &sol, ayrinti: &sag, sinif: "ozet" }, "")
    }

    /// Meta yenilemesinden (oto=1) gelen istek ve bu oturumda yenileme 15 dk içinde açılmış/yenilenmiş: bakış kaydedilmez.
    /// Elle yazılmış / yer imi oto=1 (yenilemeyi açmamış oturum) normal bakış sayılır.
    fn oto_yenileme(&self, req: &Req, o: &Oturum, now: f64) -> bool {
        sorgu_bir(req, "oto") && (0.0..YENILE_PENCERE_SN).contains(&(now - o.yenile_acildi))
    }

    fn oturumlar_sayfa(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let (yenile, oto, sira) = (sorgu_bir(req, "yenile"), sorgu_bir(req, "oto"), sira(req));
        if !self.oto_yenileme(req, o, now) {
            // düğmeyle açılış bir kez "Otomatik yenilemeyi açtı" (pencere zaten açıksa — yenileme açıkken sıralama, F5 — normal bakış);
            // pencere dışındaki oto=1 normal bakış ve pencereyi yeniler
            let acik = (0.0..YENILE_PENCERE_SN).contains(&(now - o.yenile_acildi));
            self.audit(cfg, req, Some(o), if yenile && !oto && !acik { "PANEL_OTURUMLAR_YENILE" } else { "PANEL_OTURUMLAR" }, "");
            if let (true, Some(t)) = (yenile, &req.token) {
                self.oturumlar.set_yenile(t, now);
            }
        }
        let ses = ortak::load_sessions(&cfg.main.state_root);
        let sinirlar = crate::hiz::oku(&cfg.main.state_root);
        let bagli = std::fs::read(self.sys.join(&cfg.main.iface).join("brforward")).map(|b| ortak::kopruye_bagli(&b)).unwrap_or_default();
        let rozet = |mac: &str| -> String {
            if bagli.contains(mac) { "<span class=\"rozet bagli\">Bağlı</span>".into() } else { "<span class=\"rozet\">Bağlı değil</span>".into() }
        };
        let durum = crate::trafik::oku(&self.trafik_path).filter(|d| now - d.zaman <= OLCUM_ESKI_SN);
        // son görülme kaydedicinin varlık kaydından (oturumu süren ve köprüde görülen cihazlar)
        let varlik = ortak::varlik_oku(&cfg.main.state_root);
        let son = |mac: &str| varlik.get(mac).map_or("—".to_string(), |v| ortak::son_gorulme(v.son, now));
        let olcum = |mac: &str| durum.as_ref().and_then(|d| d.cihazlar.get(mac));
        struct Satir {
            bagli: bool,
            bayt: u64,
            son: Option<f64>,
            bas: f64,
            oge: String,
        }
        let mut satirlar: Vec<Satir> = vec![];
        for (mac, s) in &ses {
            let c = olcum(mac);
            let sinir = sinirlar.get(mac);
            let ip = if s.ip.is_empty() { "<span class=\"not\">beklemede</span>".into() } else { h(&s.ip) };
            let oge = kisi_ogesi(
                &format!("{} {}", s.ad, s.soyad),
                &son(mac),
                &olcum_hucreleri(c),
                &format!("{}{}", hiz_sayfasi::yavas_rozeti(sinir), rozet(mac)),
                &[("Telefon / Kimlik", h(&crate::kimlik::goster(&s.phone))), ("MAC", format!("<code>{}</code>", h(mac))), ("IP", ip)],
                &format!(
                    "{}{}",
                    hiz_sayfasi::yavas_dugmesi(o, mac, sinir),
                    post_button(o, "/oturumlar/at", "Bağlantıyı kes", &[("mac", mac)], "tehlike")
                ),
            );
            satirlar.push(Satir {
                bagli: bagli.contains(mac),
                bayt: c.map_or(0, |c| c.bugun_bayt),
                son: varlik.get(mac).map(|v| v.son),
                bas: s.start_epoch,
                oge,
            });
        }
        // izinli cihazlar portal açmaz (oturum yok): ağda kirası ya da ölçümü varsa listede; IP kiradan
        let izinli = Config::devices(&cfg.allow);
        let kiralar = ortak::parse_leases(&std::fs::read_to_string(&cfg.main.leases_file).unwrap_or_default());
        let ip_of: std::collections::BTreeMap<String, String> =
            crate::trafik::ip_mac(std::iter::empty(), &izinli, &kiralar).into_iter().map(|(ip, mac)| (mac, ip)).collect();
        for (mac, not) in izinli.iter().filter(|(mac, _)| !ses.contains_key(*mac)) {
            let c = olcum(mac);
            let Some(ip) = ip_of.get(mac).or(c.map(|c| &c.ip)).filter(|ip| !ip.is_empty()) else { continue };
            let ad = if not.trim().is_empty() { "İzinli cihaz".to_string() } else { format!("İzinli cihaz · {}", not.trim()) };
            let sinir = sinirlar.get(mac);
            // oturumu yok: telefon ve bağlantıyı kes yok
            let oge = kisi_ogesi(
                &ad,
                &son(mac),
                &olcum_hucreleri(c),
                &format!("{}{}", hiz_sayfasi::yavas_rozeti(sinir), rozet(mac)),
                &[("MAC", format!("<code>{}</code>", h(mac))), ("IP", h(ip))],
                &hiz_sayfasi::yavas_dugmesi(o, mac, sinir),
            );
            satirlar.push(Satir {
                bagli: bagli.contains(mac),
                bayt: c.map_or(0, |c| c.bugun_bayt),
                son: varlik.get(mac).map(|v| v.son),
                bas: f64::NEG_INFINITY, // oturumu yok: kaydı olmayanların sonunda
                oge,
            });
        }
        // varsayılan: bağlı olanlar önce, sonra son görülme (yeni üstte); kaydı olmayanlar sonda, oturum başlangıcına göre (yeni üstte)
        satirlar.sort_by(|a, b| {
            b.bagli
                .cmp(&a.bagli)
                .then_with(|| match (a.son, b.son) {
                    (Some(x), Some(y)) => y.total_cmp(&x),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                })
                .then_with(|| b.bas.total_cmp(&a.bas))
        });
        // MB: yine bağlı olanlar önce; eşitlikte varsayılan sıra kalır (kararlı sıralama)
        match sira {
            "mb" => satirlar.sort_by_key(|s| (std::cmp::Reverse(s.bagli), std::cmp::Reverse(s.bayt))),
            _ => {}
        }
        let toplam = satirlar.len();
        let sayfa_say = toplam.div_ceil(OTURUM_SAYFA).max(1);
        let sayfa = sayfa_no(req).min(sayfa_say);
        let ogeler: String = satirlar.into_iter().skip((sayfa - 1) * OTURUM_SAYFA).take(OTURUM_SAYFA).map(|s| s.oge).collect();
        let sayfalama = if toplam == 0 {
            String::new()
        } else {
            let onceki = if sayfa > 1 {
                format!("<a class=\"dugme ikincil\" href=\"{}\">‹ Önceki</a>", h(&oturumlar_url(sira, sayfa - 1, yenile, false)))
            } else {
                String::new()
            };
            let sonraki = if sayfa < sayfa_say {
                format!("<a class=\"dugme ikincil\" href=\"{}\">Sonraki ›</a>", h(&oturumlar_url(sira, sayfa + 1, yenile, false)))
            } else {
                String::new()
            };
            format!("<div class=\"eylemler sayfalama\">{onceki}<span class=\"not\">Sayfa {sayfa} / {sayfa_say} · toplam {toplam} kişi</span>{sonraki}</div>")
        };
        // sıralama 1. sayfaya döner
        let secenek = |k: &str, ad: &str| {
            format!("<a class=\"{}\" href=\"{}\">{ad}</a>", if k == sira { "dugme" } else { "dugme ikincil" }, h(&oturumlar_url(k, 1, yenile, false)))
        };
        let arac = format!(
            "<div class=\"eylemler arac\"><span class=\"not\">Sırala:</span>{}{}<span class=\"ayrac\"></span>\
             <a class=\"{}\" href=\"{}\">⟳ 10 sn'de bir yenile: {}</a></div>",
            secenek("son", "Son görülme"),
            secenek("mb", "Bugün MB"),
            if yenile { "dugme" } else { "dugme ikincil" },
            h(&oturumlar_url(sira, sayfa, !yenile, false)),
            if yenile { "açık" } else { "kapalı" },
        );
        let olcum_notu = match &durum {
            Some(d) => format!("Ölçüm {} sn önce", (now - d.zaman).max(0.0).round() as u64),
            None => "Ölçüm alınamıyor: bugünkü kullanım şu an gösterilemiyor.".into(),
        };
        // yavaşlatma uygulanamıyorsa listedeki "≤ N Mb/sn" rozetleri yanıltmasın (Yavaşlatılmış bölümündeki uyarının aynısı)
        let hata = durum.as_ref().and_then(|d| d.hiz_hata.as_ref()).map(|e| format!("<div class=\"mesaj hata\">{}</div>", h(e))).unwrap_or_default();
        let liste = if ogeler.is_empty() { "<p class=\"not\">Bağlı kullanıcı yok</p>".to_string() } else { format!("<ul class=\"kisiler\">{ogeler}</ul>") };
        // liste başlığın hemen altında; sıralama/yenileme, ölçüm zamanı ve açıklamalar listenin altında (2026-10-27, kullanıcı isteği)
        let body = format!(
            "{hata}{liste}{sayfalama}{arac}<p class=\"not\">{}</p>\
             <p class=\"not\">Bağlantısı kesilen cihaz internete çıkamaz; yeniden SMS ile giriş yapması gerekir. \
             <b>Bağlı</b>: cihaz son 5 dakikada ağda görüldü. \
             Bugün: gece 00:00'dan beri indirilen + yüklenen (MB).</p>\
             <p class=\"not\">Bu sayfadaki her görüntüleme, arama ve indirme kimin yaptığıyla birlikte kaydedilir ve hizmet sağlayıcı tarafından denetlenir.</p>",
            h(&olcum_notu)
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
            if !gorunur(a, cfg) {
                continue;
            }
            let cur = form.and_then(|f| f.get(a.key).cloned()).unwrap_or_else(|| get_field(cfg, a.key));
            let id = a.key.replace('.', "-");
            let err = errors.get(a.key).map_or(String::new(), |e| format!("<div class=\"hata\">{}</div>", h(e)));
            if let Some(neden) = salt_okunur(a, bagli) {
                let deger = if matches!(a.tur, Tur::Evet) { (if cur == "1" { "açık" } else { "kapalı" }).to_string() } else { cur.clone() };
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
                    let mut o: String = opts.iter().map(|(k, l)| format!("<option value=\"{k}\"{}>{}</option>", if *k == cur { " selected" } else { "" }, h(l))).collect();
                    if cur.is_empty() {
                        o.insert_str(0, "<option value=\"\" selected>— Seçin —</option>");
                    }
                    format!("<label for=\"{id}\">{}</label><select id=\"{id}\" name=\"{}\">{o}</select>{err}", h(a.label), a.key)
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
        if !admin {
            // değiştirilemeyen yöntemler (SMS) bilgi satırı olarak, bölümün başında
            let bilgi: String = crate::ayar::GirisYontemi::HEPSI
                .iter()
                .filter(|y| !y.degistirilebilir())
                .map(|y| format!("<p class=\"not\">{}: <b>her zaman açık</b></p>", h(y.ad())))
                .collect();
            match groups.iter_mut().find(|(g, _)| *g == "Giriş yöntemleri") {
                Some((_, s)) => s.insert_str(0, &bilgi),
                None => groups.push(("Giriş yöntemleri", bilgi)),
            }
        }
        let mut body = format!("<form method=\"post\" action=\"{}\">{}", if admin { "/admin-ayarlari" } else { "/ayarlar" }, csrf_input(o));
        // Ayarlar → İşletme: "İşletme" grubunun adı bölüm başlığıyla aynı, tekrar yazılmaz
        let tek = groups.len() == 1;
        for (g, inner) in &groups {
            if tek || (!admin && *g == "İşletme") {
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
            let mut durum = vec![
                ("Doğrulama SMS'i", "Doğrulama SMS'leri yönetim merkezinden gönderilir (NetGSM ve Twilio ayarları Yönetim → SMS ayarları'nda).".into()),
                ("Mod", if cfg.sms.mock { "Deneme (SMS gönderilmiyor)".into() } else { "Gerçek SMS".into() }),
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
        let when = |r: &crate::kayit::Rec| ortak::tarih_goster(&r[0]);
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
        let mut changes = match validate(admin, &req.form, &cfg, crate::merkez::oku(&self.merkez_path).is_some()) {
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
        // türde görünmeyen giriş yöntemi aynı kayıtta kapanır (ör. otel → kafe: misafir kodu kapalı)
        let tur = crate::ayar::isletme_turu(&cfg).to_string();
        if cfg.giris.misafir_kodu && !crate::ayar::GirisYontemi::MisafirKodu.gorunur(&tur) {
            cfg.giris.misafir_kodu = false;
            match changes.iter_mut().find(|(k, _, _)| *k == "giris.misafir_kodu") {
                Some(c) => c.2 = b(false),
                None => changes.push(("giris.misafir_kodu", b(true), b(false))),
            }
            changes.retain(|(_, old, new)| old != new);
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
            if *k == "main.isletme_turu" {
                // ayrı olay: tür seçimi (spec §1); hiç seçilmemişse eski=secilmemis
                let eski = if old.is_empty() { "secilmemis" } else { old.as_str() };
                self.audit(&cfg, req, Some(o), "PANEL_ISLETME_TURU", &format!("eski={eski} yeni={new}"));
            } else {
                self.audit(&cfg, req, Some(o), "PANEL_AYAR", &describe(k, old, new));
            }
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
            ("Son mühürlenen gün", h(&chain.last().map_or("henüz yok".to_string(), |l| ortak::tarih_goster(&l[0])))),
            ("Saat", h(&ortak::tarih_saat_goster(&ortak::now_iso((self.clock)())))),
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
/// PEM bloğu (verilen türlerden biri) var ve gövdesi base64 mü. tiny_http boş/bozuk sertifikada açılamaz, boş anahtar
/// dosyasında panik eder (yalnızca PKCS#8 "PRIVATE KEY" ve "RSA PRIVATE KEY" okur).
fn pem_ok(veri: &[u8], turler: &[&str]) -> bool {
    let Ok(s) = std::str::from_utf8(veri) else { return false };
    turler.iter().any(|t| {
        let (bas, son) = (format!("-----BEGIN {t}-----"), format!("-----END {t}-----"));
        let govde = s.find(&bas).and_then(|i| {
            let g = &s[i + bas.len()..];
            g.find(&son).map(|j| &g[..j])
        });
        govde.is_some_and(|g| {
            let b: Vec<u8> = g.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
            b.len() >= 64 && b.len() % 4 == 0 && b.iter().all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(c))
        })
    })
}

fn sertifika_gecerli(cert: &[u8], key: &[u8]) -> bool {
    pem_ok(cert, &["CERTIFICATE"]) && pem_ok(key, &["PRIVATE KEY", "RSA PRIVATE KEY"])
}

/// Öz-imzalı sertifika cihazda üretilir (kalıba girmez). Eksik, boş ya da okunamıyorsa (`zorla`: her durumda) yeniden
/// üretilir: geçici dosyalara (anahtar baştan 600), doğrulanınca yerlerine taşınır; yarım ya da boş dosya kalmaz
/// (Visavis cihazında kurulumda 0 baytlık sertifika oluştu, panel açılamadı — 2026-10-16).
fn sertifika_hazirla(cert: &std::path::Path, key: &std::path::Path, zorla: bool, uret: &dyn Fn(&std::path::Path, &std::path::Path) -> bool) -> Result<(Vec<u8>, Vec<u8>), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let (c, k) = (std::fs::read(cert).unwrap_or_default(), std::fs::read(key).unwrap_or_default());
    if !zorla && sertifika_gecerli(&c, &k) {
        return Ok((c, k));
    }
    let (yc, yk) = (cert.with_extension("crt.yeni"), key.with_extension("key.yeni"));
    let temizle = || {
        let _ = std::fs::remove_file(&yc);
        let _ = std::fs::remove_file(&yk);
    };
    temizle();
    // openssl var olan dosyanın iznini değiştirmez: gizli anahtar hiçbir an başkasınca okunamaz
    std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&yk).map_err(|e| format!("panel anahtarı için dosya açılamadı: {e}"))?;
    if !uret(&yc, &yk) {
        temizle();
        return Err("panel sertifikası üretilemedi (openssl)".into());
    }
    let (c, k) = (std::fs::read(&yc).unwrap_or_default(), std::fs::read(&yk).unwrap_or_default());
    if !sertifika_gecerli(&c, &k) {
        temizle();
        return Err("üretilen panel sertifikası okunamadı".into());
    }
    for f in [&yc, &yk] {
        if let Ok(d) = std::fs::File::open(f) {
            let _ = d.sync_all();
        }
    }
    if let Err(e) = std::fs::rename(&yk, key).and_then(|_| std::fs::rename(&yc, cert)) {
        temizle();
        return Err(format!("panel sertifikası yerine konamadı: {e}"));
    }
    Ok((c, k))
}

fn openssl_uret(c: &std::path::Path, k: &std::path::Path) -> bool {
    let host = ortak::capture(&["hostname"]).trim().to_string();
    let host = if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') { "wificorrect".to_string() } else { host };
    ortak::run_timeout(
        &cmd(&[
            "openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "3650",
            "-subj", &format!("/CN={host}"), "-keyout", &k.to_string_lossy(), "-out", &c.to_string_lossy(),
        ]),
        60,
    )
}

/// tiny_http HTTPS sunucusu; sertifika/anahtar okunamazsa (tiny_http bazen panik eder) hata döner. Dinleme hatası
/// (port dolu vb.) sertifika hatasından ayrılır: o durumda sertifika yeniden üretilmez. Dönen hata: (sertifika mı, metin).
fn sunucu_ac(cert: &[u8], key: &[u8]) -> Result<tiny_http::Server, (bool, String)> {
    let ssl = tiny_http::SslConfig { certificate: cert.to_vec(), private_key: key.to_vec() };
    match std::panic::catch_unwind(|| tiny_http::Server::https("0.0.0.0:8443", ssl)) {
        Ok(Ok(s)) => Ok(s),
        Ok(Err(e)) => {
            let dinleme = e.downcast_ref::<std::io::Error>().is_some_and(|e| {
                matches!(e.kind(), std::io::ErrorKind::AddrInUse | std::io::ErrorKind::AddrNotAvailable | std::io::ErrorKind::PermissionDenied)
            });
            Err((!dinleme, format!("8443 dinlenemedi: {e}")))
        }
        Err(_) => Err((true, "sertifika okunamadı".into())),
    }
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

/// AP aktarım iş parçacıklarının kuyruğu
type ApKuyrugu = std::sync::mpsc::SyncSender<(tiny_http::Request, crate::ap_arayuz::Hedef)>;

fn handle(p: &Panel, mut req: tiny_http::Request, ap: &ApKuyrugu) {
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    // AP arayüzü aktarımı (spec 2026-10-16-ap-arayuz): başlık yalnızca merkezden geçerli (ap_karar denetler). AP kipinde
    // müşteri numarası ayrı başlıkta (X-WFC-AP-Kullanici; final review M2): istek panele düşse de oturum açtırmaz
    let ap_basligi = req.headers().iter().find(|hd| hd.field.equiv("X-WFC-AP")).is_some_and(|hd| hd.value.as_str().trim() == "1");
    if ap_basligi {
        let kullanici = req.headers().iter().find(|hd| hd.field.equiv("X-WFC-AP-Kullanici")).map(|hd| hd.value.as_str().to_string());
        match p.ap_karar(&ip, kullanici.as_deref()) {
            ap_aktarim::ApKarar::Panel => {}
            ap_aktarim::ApKarar::Cevap(r) => return ap_aktarim::ap_cevapla(req, r),
            ap_aktarim::ApKarar::Aktar(h) => {
                if let Err(std::sync::mpsc::TrySendError::Full((req, _)) | std::sync::mpsc::TrySendError::Disconnected((req, _))) = ap.try_send((req, h)) {
                    let r = ap_aktarim::sade_sayfa(503, "AP arayüzü", "Şu an çok fazla istek var; biraz sonra yeniden deneyin.", true);
                    ap_aktarim::ap_cevapla(req, r);
                }
                return;
            }
        }
    }
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
    let (cert, key) = (std::path::Path::new(CERT), std::path::Path::new(KEY));
    let acik = sertifika_hazirla(cert, key, false, &openssl_uret).and_then(|(c, k)| match sunucu_ac(&c, &k) {
        Ok(s) => Ok(s),
        // dosyalar PEM gibi görünüyor ama açılmıyor (bozuk anahtar vb.): bir kez yeniden üretilir
        Err((true, e)) => {
            eprintln!("panel: {e}; sertifika yeniden üretiliyor");
            let (c, k) = sertifika_hazirla(cert, key, true, &openssl_uret)?;
            sunucu_ac(&c, &k).map_err(|(_, e)| e)
        }
        Err((false, e)) => Err(e),
    });
    let server = match acik {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("panel: {e}");
            return ExitCode::from(1);
        }
    };
    let panel = Arc::new(Panel::new(cfg_path, hesap::PATH, Box::new(|c: &[String]| ortak::run(c)), Box::new(ortak::wall)));
    eprintln!("panel: https://0.0.0.0:8443 dinleniyor{}", if crate::merkez::oku(std::path::Path::new(crate::merkez::PATH)).is_none() { " (müşteri girişi bekleniyor)" } else { "" });
    // AP arayüzü aktarımları ayrı iş parçacıklarında (panelin 4 iş parçacığı uzun aktarımlarda beklemesin)
    let (ap_tx, ap_rx) = std::sync::mpsc::sync_channel::<(tiny_http::Request, crate::ap_arayuz::Hedef)>(ap_aktarim::AP_KUYRUK);
    let ap_rx = Arc::new(std::sync::Mutex::new(ap_rx));
    for _ in 0..ap_aktarim::AP_ISCI {
        let (rx, panel) = (ap_rx.clone(), panel.clone());
        std::thread::spawn(move || loop {
            let is = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
            let Ok((req, h)) = is else { break };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ap_aktarim::ap_isle(&panel, req, h))).is_err() {
                eprintln!("panel: AP aktarımında beklenmeyen hata");
            }
        });
    }
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let (server, panel, ap) = (server.clone(), panel.clone(), ap_tx.clone());
            std::thread::spawn(move || loop {
                match server.recv() {
                    Ok(req) => {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&panel, req, &ap))).is_err() {
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
        /// Sahte komut çıktısı: "araci" satırı → aracı birimi çalışıyor; `TEKLIF`/`TAMAM` satırları → DHCP denetimi çıktısı
        ip_ek: Arc<Mutex<String>>,
    }

    const IP: &str = "192.168.1.50";

    const NEIGH: &str = "10.50.0.2 lladdr 70:4f:57:dd:7a:e8 REACHABLE\n10.50.0.44 lladdr da:11:22:33:44:55 STALE\n10.50.0.45 FAILED\n";

    fn sahte_ip(a: &[&str], ek: &str) -> String {
        match a {
            ["ip", "-4", "neigh", "show", "dev", "br-hotspot"] => NEIGH.into(),
            ["systemctl", "is-active", "wfc-ap-araci.service"] => if ek.lines().any(|l| l == "araci") { "active\n" } else { "inactive\n" }.into(),
            ["systemctl", "is-active", "wfc-ap-bul.service"] => if ek.lines().any(|l| l == "bul") { "active\n" } else { "inactive\n" }.into(),
            ["timeout", "25", "/usr/local/bin/wificorrect", "ctl", "ap-dhcp-denetle"] => {
                ek.lines().filter(|l| l.starts_with("TEKLIF") || l.starts_with("TAMAM")).map(|l| format!("{l}\n")).collect()
            }
            _ => String::new(),
        }
    }

    fn env() -> Env {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-panel-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = format!("[main]\nsite_name = 'Bocafe'\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\nleases_file = '{0}/leases'\n[uzak]\nenabled = true\nsunucu = 'vpn.wificorrect.com:51820'\nsunucu_anahtar = '{UZAK_KEY}'\nadres = '10.99.0.11'\n", root.display());
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
        p.liste_yollar = crate::liste::Yollar { dizin: root.join("liste"), conf: root.join("hazir-liste.conf") };
        p.merkez_path = root.join("merkez.json");
        p.trafik_path = root.join("trafik.json");
        p.http = merkez_yok();
        p.rapor_http = merkez_yok();
        p.sys = root.join("sys"); // testler gerçek arayüzlere bakmasın; "wfc" = tünel arayüzü var
        std::fs::create_dir_all(p.sys.join("wfc")).unwrap();
        // AP kurulumu: durum dosyası geçici klasörde, `ip` çıktısı sahte (testler gerçek ip çalıştırmaz)
        p.ap_yollar = crate::ap::Yollar::kok(&root);
        let ip_ek: Arc<Mutex<String>> = Arc::default();
        let ek2 = ip_ek.clone();
        p.ip_oku = Box::new(move |a: &[&str]| sahte_ip(a, &ek2.lock().unwrap()));
        // PPPoE durumu: testler gerçek ip / journalctl çalıştırmaz (hiçbir komut çıktı vermez: adres yok, pppd günlüğü yok)
        p.komut = Box::new(|_: &[&str]| None);
        p.internet = Box::new(|| true); // testler gerçek ağa yoklama yapmaz
        // AP arayüzü: testler gerçek ağa bağlanmaz (web arayüzü 80'de var sayılır; aktarım bağlantısı yok)
        p.ap_dene = Box::new(|_, _, _| Some(80));
        p.ap_baglan = Box::new(|_: &crate::ap_arayuz::Hedef| Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        Env { p, calls, root, ip_ek }
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
            c.main.adres = "Moda Cad. No 1 Kadıköy/İstanbul".into();
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
        // oturumsuz sayfa yönetimin giriş sayfası gibi: beyaz üst şeritte açık zemin logosu, menü ve menü düğmesi yok; sekme simgesi
        assert!(g.contains(&format!("<header class=\"ust-acik\">{}</header>", crate::logo::marka_acik())) && g.contains("<link rel=\"icon\""), "{g}");
        assert!(!g.contains("<aside") && !g.contains("id=\"menu-ac\"") && !g.contains("Çıkış"), "{g}");
        assert!(g.contains(" · WifiCorrect</title>"), "bağlanmamış cihazda sekme başlığı: {g}");
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
        assert_eq!(menu_linkleri(&ozet), ["/", "/cihazlar", "/kayitlar", "/ayarlar"]);
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok)));
        assert_eq!(s.status, 200);
        for (id, form) in [("oturumlar", ""), ("yasak", "/yasak/ekle"), ("izinli", "/izinli/ekle"), ("yavaslatilmis", ""), ("yasakli-siteler", "/yasakli-siteler/ekle")] {
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
        assert!(satir_of(&s, "Ayşe").contains("<span class=\"rozet bagli\">Bağlı</span>"));
        assert!(!satir_of(&s, "Mehmet").contains("rozet bagli") && satir_of(&s, "Mehmet").contains("Bağlı değil"));
        assert_eq!(ortak::kopruye_bagli(&[1, 2, 3]), BTreeSet::new()); // yarım kayıt yok sayılır
    }

    const SAAT: f64 = 1_790_705_134.0;

    /// Sahte oturumlar (mac, ad, başlangıç) ve trafik.json (mac → indir, yükle, bugün; zaman = test saati - yaş; yaş yoksa dosya yok).
    fn trafikli(e: &Env, kisiler: &[(&str, &str, f64)], olcum: &[(&str, u64, u64, u64)], yas: Option<f64>) {
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let mut ses = ortak::Sessions::new();
        for (mac, ad, bas) in kisiler {
            ses.insert(mac.to_string(), ortak::Session {
                phone: "905334553132".into(), ad: ad.to_string(), soyad: "Y".into(), ip: "10.50.0.23".into(),
                session_id: "s".into(), start: "2026-09-29T20:00:00+03:00".into(), start_epoch: *bas, expires_epoch: 9e9,
            });
        }
        ortak::save_sessions(&cfg.main.state_root, &ses).unwrap();
        let _ = std::fs::remove_file(&e.p.trafik_path);
        if let Some(yas) = yas {
            let cihazlar = olcum
                .iter()
                .map(|(mac, i, y, b)| (mac.to_string(), crate::trafik::Cihaz { ip: "10.50.0.23".into(), indir_bps: *i, yukle_bps: *y, bugun_bayt: *b }))
                .collect();
            crate::trafik::yaz(&e.p.trafik_path, &crate::trafik::Durum { zaman: SAAT - yas, cihazlar, hiz_hata: None }).unwrap();
        }
    }

    /// Bağlı kullanıcılar listesinde adı (ya da MAC'i) geçen kişinin kartı (`<li class="kisi">` içi)
    fn satir_of(s: &str, ad: &str) -> String {
        s.split("<li class=\"kisi\">")
            .skip(1)
            .map(|r| r.split("</li>").next().unwrap())
            .find(|r| r.contains(ad))
            .unwrap_or_else(|| panic!("{ad} satırı yok"))
            .to_string()
    }

    #[test]
    fn hiz_ve_mb_sutunlari() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        trafikli(&e, &[("aa:bb:cc:dd:ee:01", "Ayşe", 1.0)], &[("aa:bb:cc:dd:ee:01", 38_400_000, 2_100_000, 4_820_000_000)], Some(2.0));
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        let r = satir_of(&s, "Ayşe");
        assert!(r.contains("Bugün 4.820 MB") && !r.contains("38,4") && !r.contains("2,1"), "{r}");
        // liste: kişi başına açılır kart (2026-10-30); kapalı satırda ad, son görülme, MB ve durum, ok ile açılınca telefon, MAC, IP
        // ve düğmeler
        assert!(s.contains("<ul class=\"kisiler\"><li class=\"kisi\"><details><summary>") && !s.contains("kullanici-tablo"), "liste");
        assert!(r.contains(&format!("<span class=\"ok\">{}</span></summary>", simge(SIMGE_OK))), "ok: {r}");
        let ad = r.find("<b>Ayşe Y</b>").unwrap();
        let sira_ok = [r.find("Son görülme — · Bugün 4.820 MB"), r.find("<span class=\"rozet"), r.find("</summary>"),
                       r.find("<dt>Telefon / Kimlik</dt><dd>+90"), r.find("<dt>MAC</dt><dd><code>aa:bb:cc:dd:ee:01</code>"),
                       r.find("<dt>IP</dt><dd>10.50.0.23</dd>"), r.find(">Yavaşlat</summary>"), r.find("Bağlantıyı kes")]
            .iter()
            .try_fold(ad, |onceki, x| x.filter(|&x| x > onceki));
        assert!(sira_ok.is_some(), "sıra: {r}");
        // son görülme: kayıt yoksa "—", bugünse saat, değilse gün.ay.yıl
        assert!(r.contains("Son görülme — ·") && !r.contains("2026-09-29 20:00"), "{r}");
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let g = std::collections::BTreeSet::from(["aa:bb:cc:dd:ee:01".to_string()]);
        ortak::varlik_guncelle(&cfg.main.state_root, &g, SAAT - 86_400.0 * 3.0).unwrap();
        let s2 = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(satir_of(&s2, "Ayşe").contains("Son görülme 26.09.2026 ·"), "{}", satir_of(&s2, "Ayşe"));
        ortak::varlik_guncelle(&cfg.main.state_root, &g, SAAT - 120.0).unwrap();
        let s3 = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(satir_of(&s3, "Ayşe").contains("Son görülme 21:03 ·"), "{}", satir_of(&s3, "Ayşe"));
        assert!(!s.contains("İndirme") && !s.contains("Yükleme") && !s.contains("Mb/sn</th>"), "hız sütunları yok");
        assert_eq!((mb(0), mb(999_999_999), mb(12_345_678_000_000)), ("0".into(), "1.000".into(), "12.345.678".into()));
    }

    /// Köprü iletim tablosu: verilen MAC'ler son 5 dakikada görüldü ("Bağlı").
    fn kopru_bagli(e: &mut Env, macs: &[&str]) {
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let sys = e.root.join("sys-sayfa");
        std::fs::create_dir_all(sys.join(&cfg.main.iface)).unwrap();
        let mut fdb = vec![];
        for mac in macs {
            fdb.extend(mac.split(':').map(|x| u8::from_str_radix(x, 16).unwrap()));
            fdb.extend([1, 0, 0x3d, 0x21, 0, 0, 0, 0, 0, 0]);
        }
        std::fs::write(sys.join(&cfg.main.iface).join("brforward"), fdb).unwrap();
        std::fs::create_dir_all(sys.join("wfc")).unwrap();
        e.p.sys = sys;
    }

    #[test]
    fn varsayilan_siralama_bagli_once_son_gorulme() {
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        // Ayşe ve Zeynep bağlı; Mehmet bağlı değil ama görülmüş; Veli ve Ali hiç görülmemiş (Veli daha yeni oturum)
        trafikli(
            &e,
            &[("aa:bb:cc:dd:ee:01", "Ayşe", 1.0), ("aa:bb:cc:dd:ee:02", "Mehmet", 2.0), ("aa:bb:cc:dd:ee:03", "Zeynep", 3.0),
              ("aa:bb:cc:dd:ee:04", "Ali", 4.0), ("aa:bb:cc:dd:ee:05", "Veli", 5.0)],
            &[("aa:bb:cc:dd:ee:01", 1_000_000, 0, 9_000_000_000), ("aa:bb:cc:dd:ee:02", 50_000_000, 0, 5_000_000_000),
              ("aa:bb:cc:dd:ee:03", 2_000_000, 0, 1_000_000), ("aa:bb:cc:dd:ee:04", 40_000_000, 0, 0)],
            Some(3.0),
        );
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        for (mac, yas) in [("aa:bb:cc:dd:ee:02", 3600.0), ("aa:bb:cc:dd:ee:01", 600.0), ("aa:bb:cc:dd:ee:03", 60.0)] {
            ortak::varlik_guncelle(&cfg.main.state_root, &BTreeSet::from([mac.to_string()]), SAAT - yas).unwrap();
        }
        kopru_bagli(&mut e, &["aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:03"]);
        let sira = |q: &[(&str, &str)], beklenen: &[&str]| {
            let s = e.p.handle(&get("/cihazlar", q, &tok)).body;
            let yer: Vec<usize> = beklenen.iter().map(|ad| s.find(&format!("{ad} Y")).unwrap_or_else(|| panic!("{ad} yok"))).collect();
            assert!(yer.windows(2).all(|w| w[0] < w[1]), "{q:?}: beklenen {beklenen:?}");
            s
        };
        let s = sira(&[], &["Zeynep", "Ayşe", "Mehmet", "Veli", "Ali"]);
        // seçili sıralama dolu düğme, diğerleri ikincil; düğme sırası Son görülme, Bugün MB; yenileme kapalı
        let son = s.find("<a class=\"dugme\" href=\"/cihazlar?sira=son#oturumlar\">Son görülme</a>").expect("seçili sıralama");
        let mb = s.find("<a class=\"dugme ikincil\" href=\"/cihazlar?sira=mb#oturumlar\">Bugün MB</a>").unwrap();
        assert!(son < mb && !s.contains("Bağlanma saati") && !s.contains("Anlık hız") && !s.contains("sira=hiz"));
        assert!(s.contains("href=\"/oturumlar?sira=son&amp;yenile=1\">⟳ 10 sn'de bir yenile: kapalı</a>"), "yenile tek bölüm sayfasına götürür");
        sira(&[("sira", "son")], &["Zeynep", "Ayşe", "Mehmet", "Veli", "Ali"]);
        // MB: bağlılar yine önce, sonra ölçü büyükten küçüğe; kalkan "hiz" sıralaması varsayılana düşer
        let s = sira(&[("sira", "hiz")], &["Zeynep", "Ayşe", "Mehmet", "Veli", "Ali"]);
        assert!(s.contains("<a class=\"dugme\" href=\"/cihazlar?sira=son#oturumlar\">Son görülme</a>"));
        sira(&[("sira", "mb")], &["Ayşe", "Zeynep", "Mehmet", "Ali"]);
        // eski "saat" ve bilinmeyen değer: varsayılan
        let s = sira(&[("sira", "saat")], &["Zeynep", "Ayşe", "Mehmet", "Veli", "Ali"]);
        assert!(s.contains("<a class=\"dugme\" href=\"/cihazlar?sira=son#oturumlar\">Son görülme</a>"));
        sira(&[("sira", "<script>")], &["Zeynep", "Ayşe", "Mehmet", "Veli", "Ali"]);
    }

    #[test]
    fn bagli_kullanicilar_sayfalama() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let kisiler: Vec<(String, String, f64)> =
            (1..=25).map(|i| (format!("aa:bb:cc:dd:ee:{i:02x}"), format!("Kişi{i:02}"), i as f64)).collect();
        let k: Vec<(&str, &str, f64)> = kisiler.iter().map(|(m, a, b)| (m.as_str(), a.as_str(), *b)).collect();
        trafikli(&e, &k, &[], None);
        let bak = |q: &[(&str, &str)]| e.p.handle(&get("/cihazlar", q, &tok)).body;
        // kayıt yok: oturum başlangıcına göre yeni üstte → 1. sayfa Kişi25..Kişi16
        let s = bak(&[]);
        assert_eq!(s.matches("Kişi").count(), 10, "1. sayfa 10 satır");
        assert!(s.contains("Kişi25 Y") && s.contains("Kişi16 Y") && !s.contains("Kişi15 Y"));
        assert!(s.contains("Sayfa 1 / 3 · toplam 25 kişi") && !s.contains("‹ Önceki"));
        assert!(s.contains("<a class=\"dugme ikincil\" href=\"/cihazlar?sira=son&amp;sayfa=2#oturumlar\">Sonraki ›</a>"), "sonraki");
        let s = bak(&[("sayfa", "2")]);
        assert!(s.contains("Kişi15 Y") && s.contains("Kişi06 Y") && s.matches("Kişi").count() == 10 && s.contains("Sayfa 2 / 3 · toplam 25 kişi"));
        assert!(s.contains("href=\"/cihazlar?sira=son#oturumlar\">‹ Önceki</a>") && s.contains("href=\"/cihazlar?sira=son&amp;sayfa=3#oturumlar\">Sonraki ›</a>"));
        let s = bak(&[("sayfa", "3")]);
        assert_eq!(s.matches("Kişi").count(), 5, "3. sayfa 5 satır");
        assert!(s.contains("Kişi05 Y") && s.contains("Kişi01 Y") && s.contains("Sayfa 3 / 3 · toplam 25 kişi") && !s.contains("Sonraki ›"));
        assert!(s.contains("href=\"/cihazlar?sira=son&amp;sayfa=2#oturumlar\">‹ Önceki</a>"));
        // sıralama düğmeleri 1. sayfaya döner; yenileme düğmesi sayfayı korur
        assert!(s.contains("href=\"/cihazlar?sira=mb#oturumlar\">Bugün MB</a>") && s.contains("href=\"/oturumlar?sira=son&amp;sayfa=3&amp;yenile=1\">⟳"));
        // aralık dışı → en yakın geçerli sayfa
        assert!(bak(&[("sayfa", "99")]).contains("Sayfa 3 / 3"));
        for v in ["0", "abc", "-2"] {
            assert!(bak(&[("sayfa", v)]).contains("Sayfa 1 / 3"), "{v}");
        }
        // yenileme açıkken bağlantılar sıra ve yenilemeyi, kendiliğinden yenileme sayfayı korur
        let s = e.p.handle(&get("/oturumlar", &[("sira", "mb"), ("sayfa", "2"), ("yenile", "1")], &tok)).body;
        assert!(s.contains("<meta http-equiv=\"refresh\" content=\"10;url=/oturumlar?sira=mb&amp;sayfa=2&amp;yenile=1&amp;oto=1\">"), "meta");
        assert!(s.contains("href=\"/oturumlar?sira=mb&amp;yenile=1\">‹ Önceki</a>") && s.contains("href=\"/oturumlar?sira=mb&amp;sayfa=3&amp;yenile=1\">Sonraki ›</a>"));
        assert!(!s.contains("Anlık hız") && s.contains("href=\"/cihazlar?sira=mb&amp;sayfa=2#oturumlar\">⟳"));
        // tek sayfa: bağlantı yok
        trafikli(&e, &k[..3], &[], None);
        let s = bak(&[]);
        assert!(s.contains("Sayfa 1 / 1 · toplam 3 kişi") && !s.contains("‹ Önceki") && !s.contains("Sonraki ›"));
    }

    #[test]
    fn izinli_cihaz_satiri() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.allow = vec![Device { mac: "aa:bb:cc:dd:ee:99".into(), name: "Kasa".into() }, Device { mac: "aa:bb:cc:dd:ee:98".into(), name: String::new() }];
        c.save(&e.p.cfg_path).unwrap();
        std::fs::write(&c.main.leases_file, "1 aa:bb:cc:dd:ee:99 10.50.0.7 kasa 01\n1 aa:bb:cc:dd:ee:98 10.50.0.8 * 01\n").unwrap();
        trafikli(&e, &[("aa:bb:cc:dd:ee:01", "Ayşe", 1.0)], &[("aa:bb:cc:dd:ee:99", 5_500_000, 300_000, 2_000_000)], Some(1.0));
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        let r = satir_of(&s, "İzinli cihaz · Kasa");
        assert!(r.contains("<code>aa:bb:cc:dd:ee:99</code>") && r.contains("10.50.0.7") && r.contains("Bugün 2 MB") && !r.contains("5,5"), "{r}");
        assert!(r.contains("<b>İzinli cihaz · Kasa</b>") && r.contains("<span class=\"rozet"), "ad ve durum: {r}");
        assert!(!r.contains("Telefon / Kimlik") && !r.contains("Bağlantıyı kes"), "bağlantı kes ve telefon yok: {r}");
        let r = satir_of(&s, "aa:bb:cc:dd:ee:98");
        assert!(r.contains("<b>İzinli cihaz</b>") && r.contains("10.50.0.8") && r.contains("Bugün —</small>"), "notsuz, ölçümsüz: {r}");
        assert!(satir_of(&s, "Ayşe").contains("Bağlantıyı kes"));
    }

    #[test]
    fn olcum_yoksa_tire_ve_not() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ayse = [("aa:bb:cc:dd:ee:01", "Ayşe", 1.0)];
        let olcum = [("aa:bb:cc:dd:ee:01", 38_400_000, 2_100_000, 4_820_000_000)];
        for yas in [None, Some(31.0)] {
            trafikli(&e, &ayse, &olcum, yas);
            let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
            let r = satir_of(&s, "Ayşe");
            assert!(r.contains("Son görülme — · Bugün —</small>"), "{yas:?}: {r}"); // bugün MB + son görülme yok
            assert!(!r.contains("4.820") && s.contains("Ölçüm alınamıyor") && !s.contains("sn önce"), "{yas:?}");
        }
        trafikli(&e, &ayse, &olcum, Some(5.0));
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(satir_of(&s, "Ayşe").contains("Bugün 4.820 MB") && s.contains("Ölçüm 5 sn önce") && !s.contains("Ölçüm alınamıyor"));
    }

    #[test]
    fn bagli_kullanicilar_tablo_ustte_aciklamalar_altta() {
        // 2026-10-27 kullanıcı: açıklamalar, denetim notu ve Sırala/yenile düğmeleri tablonun altında
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        trafikli(&e, &[("aa:bb:cc:dd:ee:01", "Ayşe", 1.0)], &[("aa:bb:cc:dd:ee:01", 38_400_000, 2_100_000, 4_820_000_000)], Some(5.0));
        for yol in ["/cihazlar", "/oturumlar"] {
            let s = e.p.handle(&req("GET", yol, &[], Some(&tok))).body;
            let yer: Vec<usize> = [
                "<ul class=\"kisiler\">",
                "<span class=\"not\">Sırala:</span>",
                "Ölçüm 5 sn önce",
                "Bağlantısı kesilen cihaz internete çıkamaz",
                "kimin yaptığıyla birlikte kaydedilir",
            ]
            .iter()
            .map(|m| s.find(m).unwrap_or_else(|| panic!("{yol}: {m} yok")))
            .collect();
            assert!(yer.windows(2).all(|w| w[0] < w[1]), "{yol}: {yer:?}");
        }
    }

    #[test]
    fn yenileme_meta_ve_denetim() {
        let mut e = env();
        let saat = Arc::new(std::sync::atomic::AtomicU64::new(SAAT.to_bits()));
        let s2 = saat.clone();
        e.p.clock = Box::new(move || f64::from_bits(s2.load(Ordering::SeqCst)));
        let ilerle = |sn: f64| saat.store((f64::from_bits(saat.load(Ordering::SeqCst)) + sn).to_bits(), Ordering::SeqCst);
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let say = |olay: &str| {
            let csv = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
            csv.lines().filter(|l| l.split(';').nth(1) == Some(olay)).count()
        };
        const META: &str = "<meta http-equiv=\"refresh\" content=\"10;url=/oturumlar?sira=son&amp;yenile=1&amp;oto=1\">";
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        assert!(!s.contains("http-equiv=\"refresh\"") && say("PANEL_OTURUMLAR") == 1 && say("PANEL_OTURUMLAR_YENILE") == 0);
        let s = e.p.handle(&get("/cihazlar", &[("yenile", "1")], &tok)).body;
        assert!(s.contains(META), "meta");
        assert!(s.contains("href=\"/cihazlar?sira=son#oturumlar\">⟳ 10 sn'de bir yenile: açık</a>"), "açıkken yenilesiz bağlantı");
        assert!(s.contains("href=\"/oturumlar?sira=mb&amp;yenile=1\""), "sıralama yenilemeyi korur");
        // yenilenen sayfa tek bölüm: Yasaklı/İzinli formları yok (yazılan silinmesin)
        let t = e.p.handle(&get("/oturumlar", &[("sira", "son"), ("yenile", "1"), ("oto", "1")], &tok)).body;
        assert!(t.contains(META) && !t.contains("action=\"/yasak/ekle\"") && !t.contains("action=\"/izinli/ekle\""), "tek bölüm");
        assert_eq!((say("PANEL_OTURUMLAR_YENILE"), say("PANEL_OTURUMLAR")), (1, 1));
        for _ in 0..3 {
            ilerle(10.0);
            assert!(e.p.handle(&get("/cihazlar", &[("sira", "son"), ("yenile", "1"), ("oto", "1")], &tok)).body.contains(META));
        }
        assert_eq!((say("PANEL_OTURUMLAR_YENILE"), say("PANEL_OTURUMLAR")), (1, 1), "pencere içinde kayıt yok");
        // yenileme açıkken sıralama tıklaması (yenile=1, oto yok): normal bakış, ikinci "yenilemeyi açtı" değil
        let s = e.p.handle(&get("/cihazlar", &[("sira", "mb"), ("yenile", "1")], &tok)).body;
        assert!(s.contains("http-equiv=\"refresh\""));
        assert_eq!((say("PANEL_OTURUMLAR_YENILE"), say("PANEL_OTURUMLAR")), (1, 2), "sıralama tıklaması");
        // /oturumlar sayfası da kendine yeniler; başka sayfa yenilemez
        let s = e.p.handle(&get("/oturumlar", &[("sira", "mb"), ("yenile", "1"), ("oto", "1")], &tok)).body;
        assert!(s.contains("<meta http-equiv=\"refresh\" content=\"10;url=/oturumlar?sira=mb&amp;yenile=1&amp;oto=1\">"), "oturumlar meta");
        assert!(!e.p.handle(&get("/kayitlar", &[("yenile", "1")], &tok)).body.contains("http-equiv=\"refresh\""));
        assert_eq!(say("PANEL_OTURUMLAR"), 2);
        ilerle(901.0);
        e.p.handle(&get("/cihazlar", &[("sira", "son"), ("yenile", "1"), ("oto", "1")], &tok));
        assert_eq!(say("PANEL_OTURUMLAR"), 3, "pencere doldu: normal bakış");
        ilerle(10.0);
        e.p.handle(&get("/cihazlar", &[("sira", "son"), ("yenile", "1"), ("oto", "1")], &tok));
        assert_eq!(say("PANEL_OTURUMLAR"), 3, "pencere yenilendi");
        // yenilemeyi hiç açmamış oturum (yer imi / elle yazılmış oto=1) kuralı delemez
        let (tok2, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        e.p.handle(&get("/cihazlar", &[("sira", "son"), ("yenile", "1"), ("oto", "1")], &tok2));
        assert_eq!((say("PANEL_OTURUMLAR"), say("PANEL_OTURUMLAR_YENILE")), (4, 1));
        assert_eq!(hareketler::olay_adi("PANEL_OTURUMLAR_YENILE"), "Otomatik yenilemeyi açtı");
    }

    #[test]
    fn gb_uc_anlamli_basamak() {
        let ornekler = [
            (0, "0"),
            (40_000_000, "0,04"),
            (3_456_000_000, "3,46"),
            (9_994_999_999, "9,99"),
            (9_995_000_000, "10,0"),
            (12_345_000_000, "12,3"),
            (99_949_999_999, "99,9"),
            (99_950_000_000, "100"),
            (1_234_500_000_000, "1.235"),
        ];
        for (bayt, beklenen) in ornekler {
            assert_eq!(gb(bayt), beklenen, "{bayt}");
        }
    }

    #[test]
    fn ozet_bugun_kullanilan_veri() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let veri = || {
            let s = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
            s.split("<dt>Bugün kullanılan veri</dt><dd>").nth(1).expect("satır yok").split("</dd>").next().unwrap().to_string()
        };
        assert_eq!(veri(), "—", "kaydedici bugün hiç yazmadı");
        // bütün cihazların bugünkü toplamı (indirme + yükleme; oturumu kapanmış cihazlar da)
        let mut g = crate::trafik::GunToplam {
            gun: "2026-09-29".into(),
            bayt: std::collections::BTreeMap::from([("aa:bb:cc:dd:ee:01".into(), 2_500_000_000), ("aa:bb:cc:dd:ee:02".into(), 960_000_000)]),
        };
        crate::trafik::gun_yaz(&cfg.main.state_root, &g).unwrap();
        assert_eq!(veri(), "3,46 GB");
        g.gun = "2026-09-28".into(); // dünün toplamı bugün sayılmaz
        crate::trafik::gun_yaz(&cfg.main.state_root, &g).unwrap();
        assert_eq!(veri(), "—");
    }

    #[test]
    fn ozet_telefonda_bagli_kullanicilar_yeni_uyelerin_ustunde() {
        // kullanıcı (2026-10-30): telefonda bağlı kullanıcılar bugün yeni üyelerin üstünde; iki bölme açılır, sıra CSS order ile
        let tel = TPL.split("@media (max-width:820px){").nth(1).expect("telefon bloğu");
        let sira = |secici: &str| -> i32 {
            let kural = tel.split(&format!("
{secici}{{")).nth(1).and_then(|k| k.split('}').next()).unwrap_or_else(|| panic!("{secici} yok"));
            kural.split(';').find_map(|d| d.strip_prefix("order:")).and_then(|o| o.parse().ok()).unwrap_or_else(|| panic!("{secici}: order yok"))
        };
        assert!(tel.contains("
.ozet .ayrinti-bolme,.ozet .bolum-govde{display:contents}"), "bölmeler açılmalı");
        assert!(sira(".ozet .pano") < sira(".ozet .liste-bolme") && sira(".ozet .ozet-uyari") < sira(".ozet .liste-bolme"));
        assert!(sira(".ozet .liste-bolme") < sira(".ozet .ozet-yeni"));
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
        // ...:01 25 dk'dır bağlı (kaydedicinin varlık kaydı)
        let now = 1_790_705_134.0;
        let g = std::collections::BTreeSet::from(["aa:bb:cc:dd:ee:01".to_string()]);
        ortak::varlik_guncelle(&cfg.main.state_root, &g, now - 25.0 * 60.0).unwrap();
        ortak::varlik_guncelle(&cfg.main.state_root, &g, now - 800.0).unwrap(); // aralar 15 dk'dan kısa
        ortak::varlik_guncelle(&cfg.main.state_root, &g, now - 30.0).unwrap();
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
        let bagli = kutu("Bağlı kullanıcılar");
        assert!(bagli.contains("<b>Ayşe Yılmaz</b><span class=\"zaman\">25 dk</span></span><small><i class=\"nokta\"></i>şu an bağlı</small>") && !bagli.contains("Mehmet"), "{bagli}");
        assert!(!bagli.contains("10.50.0.23") && !bagli.contains("20:14"), "özette IP ve tarih yok: {bagli}");
        let yeni = kutu("Bugün yeni üyeler");
        assert!(yeni.contains("<b>Zeynep Ak</b><span class=\"zaman\">19:40</span>") && !yeni.contains("Eski"), "{yeni}");
        assert!(s.contains("<dt>Bağlı kullanıcı</dt><dd>1</dd>") && s.contains("<dt>Bugün yeni üye</dt><dd>1</dd>"));
        // WhatsApp Web gibi (2026-10-30): solda bağlı kullanıcılar sohbet listesi gibi, sağda başlık çubuğu, sayılar (bağlı sayısı
        // canlı), uyarılar ve yeni üyeler
        let (sol, sag) = (s.find("<div class=\"liste-bolme\">").expect("liste"), s.find("<div class=\"ayrinti-bolme\">").expect("ayrıntı"));
        assert!(sol < s.find("<h2>Bağlı kullanıcılar</h2>").unwrap() && s.find("<h2>Bağlı kullanıcılar</h2>").unwrap() < sag, "{s}");
        assert!(sag < s.find("<section class=\"pano\"").unwrap() && s.find("<section class=\"pano\"").unwrap() < s.find("<h2>Bugün yeni üyeler</h2>").unwrap(), "{s}");
        assert!(s.contains("<div class=\"canli\"><dt>Bağlı kullanıcı</dt>") && s.contains("<div class=\"ikili ozet\">"), "{s}");
        // telefonda sıralanan kutular (ozet_telefonda_bagli_kullanicilar_yeni_uyelerin_ustunde)
        assert!(s.contains("<section class=\"kutu sohbetler\"><h2>Bağlı kullanıcılar</h2>") && s.contains("<section class=\"kutu ozet-yeni\"><h2>Bugün yeni üyeler</h2>"), "{s}");
        assert!(!s.contains("905334553132") && !s.contains("aa:bb:cc"), "özette telefon ve MAC yok");
        // sade özet: kısayol kutuları ve SMS/Disk halkaları yok (menüde zaten var)
        assert!(!s.contains("aria-label=\"Kısayollar\"") && !s.contains("SMS ve Disk") && !s.contains("class=\"halka"), "{s}");
        // sol şerit: üstte W_ işareti, simgeli menü, altta işletmenin baş harfleri (adı ipucunda) ve çıkış; oturumlu sayfada beyaz
        // logo şeridi yok, telefondaki ince şeritte logo ve çıkış
        assert!(!s.contains("<header class=\"ust-acik\">") && !s.contains("class=\"wc-logo\""), "{s}");
        let yan = s.split("<aside class=\"yan\">").nth(1).unwrap().split("</aside>").next().unwrap();
        let sira = [crate::logo::isaret(), "<nav aria-label=\"Menü\">", "<div class=\"yan-isletme\" title=\"Bocafe Göztepe\"><span class=\"avatar\" aria-hidden=\"true\">BG</span>", "<button class=\"cikis\" title=\"Çıkış\">"]
            .map(|p| yan.find(p).unwrap_or_else(|| panic!("{p} yok: {yan}")));
        assert!(sira.is_sorted() && yan.ends_with("</form></div>"), "üstte işaret, sonra menü, en altta işletme ve çıkış: {yan}");
        let nav = yan.split("<nav aria-label=\"Menü\">").nth(1).unwrap().split("</nav>").next().unwrap();
        assert!(nav.split("<a ").skip(1).all(|a| a.contains("<svg class=\"simge\"")), "{nav}");
        assert!(nav.contains(&format!("<a href=\"/\" class=\"aktif\" aria-current=\"page\" title=\"Özet\">{}<span>Özet</span></a>", simge(SIMGE_PANO))), "{nav}");
        let mobil = s.split("<header class=\"ust-mobil\">").nth(1).unwrap().split("</header>").next().unwrap();
        assert!(mobil.contains(crate::logo::marka_acik()) && mobil.contains("<span class=\"ust-isletme\">Bocafe Göztepe</span>") && mobil.contains("action=\"/cikis\""), "{mobil}");
        assert_eq!(bas_harfler("visavis"), "V");
        assert_eq!(bas_harfler("ilker ışık kafe"), "İI");
        assert!(!s.contains("class=\"kim\"") && !yan.contains("mudur") && !yan.contains("İşletme sahibi") && !yan.contains(MUSTERI), "müşteri numarası/hesap gösterilmez: {yan}");
        // menüde ve sekme başlığında yalnızca işletme adı ("Yönetim" eki yok; 2026-10-24, kullanıcı isteği)
        assert!(!yan.contains("Yönetim") && !s.contains("Yönetim</title>") && s.contains("<title>Özet · Bocafe Göztepe</title>"), "{yan}");
        let bas = s.split("</head>").next().unwrap();
        assert!(bas.contains("<link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,"), "{bas}");
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_OZET"));
        assert_eq!(kisa_zaman("2026-09-28T08:05:00+03:00", "2026-09-29"), "28.09.2026 08:05");
    }

    #[test]
    fn grup_sayfasi_liste_ve_ayrinti() {
        // WhatsApp Web gibi (2026-10-30): solda bölümler sohbet listesi gibi, sağda seçilen bölüm (CSS :target)
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(s.contains("<div class=\"ikili bolumlu\"><div class=\"liste-bolme\"><header class=\"bolme-ust\"><h1>Kullanıcılar</h1></header>"), "{s}");
        let satir = format!(
            "<a class=\"sohbet\" href=\"#yasak\"><span class=\"avatar\">{}</span><span class=\"satir-metin\"><b>Yasaklı cihazlar</b><small>Ağa hiç giremeyen cihazlar</small></span></a>",
            simge(SIMGE_YASAK)
        );
        let ayrinti = s.find("<div class=\"ayrinti-bolme\">").unwrap();
        assert!(s.find(&satir).is_some_and(|i| i < ayrinti), "{s}");
        assert!(s[ayrinti..].contains(".ikili:has(#yasak:target,#yasak :target) .sohbet[href=\"#yasak\"]{background:var(--secili)}"), "seçili satır kuralı");
        assert!(s[ayrinti..].contains("<section class=\"bolum\" id=\"yasak\""), "{s}");
        // Ayarlar'da Raporlar ve Admin satırları başka sayfaya gider
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<a class=\"sohbet\" href=\"/raporlar\">") && a.contains("<a class=\"sohbet\" href=\"/admin\">"), "{a}");
        // tek panelli sayfa: başlık çubuğu, mesaj ve içerik kartı
        let r = e.p.handle(&get("/oturumlar", &[("m", "Tamam.")], &tok)).body;
        assert!(r.contains("<div class=\"bolme tek\"><header class=\"bolme-ust\"><h1>") && r.contains("<div class=\"bolme-govde\"><div class=\"mesaj\">Tamam.</div><div class=\"icerik-kart\">"), "{r}");
    }

    #[test]
    fn admin_bolumleri_ve_yonlendirmeler() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let k = e.p.handle(&req("GET", "/kayitlar", &[], Some(&tok))).body;
        assert!(!k.contains("id=\"panel-hareketleri\"") && k.contains("action=\"/kullanicilar\""));
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("<section class=\"bolum\" id=\"admin-ayarlari\"") && a.contains("name=\"limits.sms_global_day\""));
        assert!(a.contains("<section class=\"bolum\" id=\"panel-hareketleri\"") && a.contains("href=\"/ayarlar\" class=\"aktif\""));
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
    fn sms_alanlari_cihazda_yok() {
        // SMS merkezden gider (spec 2026-10-13): Admin ayarları'nda NetGSM / Twilio / sağlayıcı / deneme modu alanı yok, not var
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let page = e.p.handle(&req("GET", "/admin-ayarlari", &[], Some(&tok))).body;
        assert!(page.contains("Doğrulama SMS'leri yönetim merkezinden gönderilir"));
        for yok in ["name=\"netgsm.", "name=\"twilio.", "name=\"sms.mock\"", "name=\"sms.provider\""] {
            assert!(!page.contains(yok), "{yok}");
        }
        // formdan elle gönderilse de yazılmaz
        e.p.handle(&req("POST", "/admin-ayarlari", &[("csrf", &csrf), ("netgsm.password", "gizli-1"), ("twilio.auth_token", "gizli-2"), ("sms.mock", "0"), ("limits.sms_global_day", "250")], Some(&tok)));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.sms.mock, c.limits.sms_global_day), (true, 250));
        let dosya = std::fs::read_to_string(&e.p.cfg_path).unwrap();
        assert!(!dosya.contains("gizli-1") && !dosya.contains("gizli-2"));
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
        for gizli in ["netgsm", "NetGSM", "Twilio", "twilio", "sms.mock", "backup.target", "Yedek", "sms_global_day", "Fabrika ayar"] {
            assert!(!page.contains(gizli), "{gizli}"); // işletme sahibi sağlayıcı / sunucu / sınır görmez
        }
        assert!(page.contains("main.unvan") && page.contains("main.session_minutes"));
        // API alanları Ayarlar formundan elle gönderilse de yok sayılır
        let r = e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.session_minutes", "1440"), ("sms.mock", "0")], Some(&tok)));
        assert_eq!(r.status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.main.session_minutes, c.sms.mock), (1440, true));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ") == "systemctl restart wificorrect-portal"));
    }

    #[test]
    fn veri_sorumlusu_iletisim_alanlari() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        for k in ["main.adres", "main.telefon", "main.eposta", "main.kep"] {
            assert!(page.contains(&format!("name=\"{k}\"")), "{k}");
        }
        let kaydet = |alanlar: &[(&str, &str)]| {
            let mut f = vec![("csrf", csrf.as_str())];
            f.extend_from_slice(alanlar);
            e.p.handle(&req("POST", "/ayarlar", &f, Some(&tok)))
        };
        let r = kaydet(&[("main.adres", "  Moda   Cad. No 2  Kadıköy "), ("main.telefon", "+90 (216) 000 00 00"), ("main.eposta", "kvkk@boca.example"), ("main.kep", "boca@hs01.kep.tr")]);
        assert_eq!(r.status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.main.adres.as_str(), c.main.telefon.as_str(), c.main.eposta.as_str(), c.main.kep.as_str()),
                   ("Moda Cad. No 2 Kadıköy", "+90 (216) 000 00 00", "kvkk@boca.example", "boca@hs01.kep.tr"));
        // isteğe bağlılar boşaltılabilir; adres de boş kaydedilebilir (Özet uyarır)
        assert_eq!(kaydet(&[("main.telefon", ""), ("main.kep", ""), ("main.adres", "")]).status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert!(c.main.telefon.is_empty() && c.main.kep.is_empty() && c.main.adres.is_empty() && c.main.eposta == "kvkk@boca.example");
        // geçersizler kaydedilmez (yer tutucu gibi köşeli parantez, bozuk e-posta, harfli telefon)
        for (k, v) in [("main.adres", "[KEP adresi]"), ("main.adres", "kısa"), ("main.eposta", "a@b"), ("main.eposta", "a b@c.d"), ("main.kep", "x@@y.z"), ("main.telefon", "0216 ABC")] {
            let r = kaydet(&[(k, v)]);
            assert_eq!(r.status, 200, "{k}={v}");
            assert!(r.body.contains("Geçersiz değer."), "{k}={v}");
        }
        assert!(Config::load(&e.p.cfg_path).unwrap().main.adres.is_empty());
        assert!(eposta_ok("") && irtibat_tel_ok("") && adres_ok(""));
    }

    #[test]
    fn tur_secilmemisse_ozet_uyarisi() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let uyari = "İşletme türünüzü seçin";
        let ozet = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(ozet.contains(uyari) && ozet.contains("href=\"/ayarlar#main-isletme_turu\""));
        let page = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(page.contains("name=\"main.isletme_turu\"") && page.contains("<option value=\"\" selected>— Seçin —</option>"));
        // başka ayarı kaydetmek (tür "Seçin"de) türü kendiliğinden yazmaz
        let r = e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", ""), ("main.session_minutes", "1440")], Some(&tok)));
        assert_eq!(r.status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.main.session_minutes, c.main.isletme_turu.as_deref()), (1440, None));
        assert!(!std::fs::read_to_string(&e.p.cfg_path).unwrap().contains("isletme_turu"));
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains(uyari));
        // geçersiz tür reddedilir
        let r = e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel'")], Some(&tok)));
        assert!(r.status == 200 && r.body.contains("Geçersiz seçim"));
        // kafe seçilince (varsayılanla aynı olsa da) uyarı kalkar
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "kafe")], Some(&tok)));
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().main.isletme_turu.as_deref(), Some("kafe"));
        assert!(!e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains(uyari));
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("<option value=\"kafe\" selected>Kafe/Restoran</option>"));
        assert!(denetim_satirlari(&e, "PANEL_ISLETME_TURU")[0].contains("eski=secilmemis yeni=kafe"));
    }

    #[test]
    fn taninmayan_tur_secilmemis_sayilir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.isletme_turu = Some("x".into()); // elle bozulmuş değer
        c.save(&e.p.cfg_path).unwrap();
        assert_eq!(crate::ayar::isletme_turu(&c), "kafe"); // davranış kafe
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("İşletme türünüzü seçin"));
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("<option value=\"\" selected>— Seçin —</option>"));
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel")], Some(&tok)));
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().main.isletme_turu.as_deref(), Some("otel"));
        assert!(denetim_satirlari(&e, "PANEL_ISLETME_TURU")[0].contains("eski=secilmemis yeni=otel"));
        assert!(!e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("İşletme türünüzü seçin"));
    }

    #[test]
    fn tur_degisimi_denetime() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "kafe")], Some(&tok)));
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel")], Some(&tok)));
        let satirlar = denetim_satirlari(&e, "PANEL_ISLETME_TURU");
        assert_eq!(satirlar.len(), 2);
        assert!(satirlar[1].contains("eski=kafe yeni=otel") && satirlar[1].contains("kullanici="), "{}", satirlar[1]);
        // tür değişimi ayrıca PANEL_AYAR olarak yazılmaz
        assert!(!denetim_satirlari(&e, "PANEL_AYAR").iter().any(|l| l.contains("isletme_turu")));
        assert_eq!(crate::ayar::isletme_turu(&Config::load(&e.p.cfg_path).unwrap()), "otel");
    }

    #[test]
    fn misafir_kodu_yalniz_otel_yurtta_gorunur() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let ayarlar = || e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        let kutu = "name=\"giris.misafir_kodu\"";
        // seçilmemiş (kafe gibi): yalnızca SMS bilgi satırı
        let page = ayarlar();
        assert!(page.contains("Giriş yöntemleri") && page.contains("SMS: <b>her zaman açık</b>") && !page.contains(kutu));
        // türde görünmeyen kutu elle gönderilse de yok sayılır
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("giris.misafir_kodu", "1"), ("main.session_minutes", "1440")], Some(&tok)));
        assert!(!Config::load(&e.p.cfg_path).unwrap().giris.misafir_kodu);
        for (tur, gorunur) in [("kafe", false), ("diger", false), ("yurt", true), ("otel", true)] {
            e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", tur)], Some(&tok)));
            assert_eq!(ayarlar().contains(kutu), gorunur, "{tur}");
        }
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel"), ("giris.misafir_kodu", "1")], Some(&tok)));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert!(c.giris.misafir_kodu && crate::ayar::misafir_kodu_acik(&c));
        assert!(ayarlar().contains("value=\"1\" checked> Misafir kodu"));
        // kutu işaretsiz gönderilirse kapanır
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel")], Some(&tok)));
        assert!(!Config::load(&e.p.cfg_path).unwrap().giris.misafir_kodu);
    }

    #[test]
    fn turden_cikinca_misafir_kodu_kapanir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "yurt")], Some(&tok)));
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "yurt"), ("giris.misafir_kodu", "1")], Some(&tok)));
        assert!(crate::ayar::misafir_kodu_acik(&Config::load(&e.p.cfg_path).unwrap()));
        // aynı kayıtta tür kafe'ye çekilir, kutu işaretli gelse de misafir kodu kapanır
        let r = e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "kafe"), ("giris.misafir_kodu", "1")], Some(&tok)));
        assert_eq!(r.status, 303);
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.main.isletme_turu.as_deref(), c.giris.misafir_kodu), (Some("kafe"), false));
        let ayar = denetim_satirlari(&e, "PANEL_AYAR");
        assert!(ayar.last().is_some_and(|l| l.contains("giris.misafir_kodu: 1 → 0")), "{ayar:?}");
        // yeniden otel: kod kendiliğinden açılmaz
        e.p.handle(&req("POST", "/ayarlar", &[("csrf", &csrf), ("main.isletme_turu", "otel")], Some(&tok)));
        assert!(!crate::ayar::misafir_kodu_acik(&Config::load(&e.p.cfg_path).unwrap()));
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
        assert!(page.contains("29.09.2026") && page.contains("açık") && page.contains("mühürlü"));
        assert!(page.contains("href=\"/kayitlar/gun?gun=2026-09-29\"") && !page.contains(">2026-09-29<"), "bağlantı ISO, görünen gün.ay.yıl");
        let page = e.p.handle(&get("/kayitlar/gun", &[("gun", d)], &tok)).body;
        assert!(page.contains("dns.csv.gz") && page.contains("Ayşe Yılmaz")); // kişi dosyasının sahibi
        assert!(page.contains("27.09.2026"));
        let page = e.p.handle(&req("POST", "/kayitlar/dosya", &[("csrf", &csrf), ("gun", d), ("dosya", "dns.csv.gz"), ("q", "ORNEK")], Some(&tok))).body;
        assert!(page.contains("www.ornek.com") && !page.contains("haber.example") && page.contains("1 satır"));
        assert!(page.contains("<td>27.09.2026 10:01:00</td>") && !page.contains("2026-09-27T10:01"), "zaman sütunu gün.ay.yıl");
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

        let page = e.p.handle(&req("POST", "/kullanicilar", &[("csrf", &csrf), ("q", "ayşe")], Some(&tok))).body;
        assert!(page.contains("+905334553132") && page.contains("1 kişi"));
        let k = crate::kimlik::opak(&crate::kimlik::sir_oku_ya_uret(&cfg.main.state_root), "905334553132");
        assert!(page.contains(&format!("href=\"/kullanici?k={k}\"")));
        let page = e.p.handle(&get("/kullanici", &[("k", &k)], &tok)).body;
        assert!(page.contains("OTURUM_BASLA") && page.contains("2026-09-27") && page.contains("905334553132"));
        assert!(page.contains("27.09.2026 10:00:00") && page.contains(">27.09.2026<"), "ilk kayıt ve gün listesi gün.ay.yıl");
        for bad in ["../x", "905334553132", "0000000000000000"] {
            assert_eq!(e.p.handle(&get("/kullanici", &[("k", bad)], &tok)).status, 404, "{bad}");
        }

        let talep = |f: &[(&str, &str)]| {
            let mut v = vec![("csrf", csrf.as_str())];
            v.extend_from_slice(f);
            e.p.handle(&req("POST", "/talep", &v, Some(&tok))).body
        };
        let page = talep(&[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "2026-09-27 11:00")]);
        assert!(page.contains("O anda açık oturum") && page.contains("905334553132"));
        let page = talep(&[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "27.09.2026 11:00")]); // ekrandaki yazım
        assert!(page.contains("O anda açık oturum") && page.contains("905334553132"));
        assert!(page.contains("value=\"29.09.2026\"") && page.contains("placeholder=\"GG.AA.YYYY SS:DD\""), "paket günleri gün.ay.yıl dolu");
        // GET adreste değer taşımaz: arama yapmaz
        assert!(!e.p.handle(&get("/talep", &[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "2026-09-27 11:00")], &tok)).body.contains("O anda açık oturum"));
        let page = talep(&[("tur", "ic-ip"), ("deger", "10.50.0.23"), ("zaman", "dün")]);
        assert!(page.contains("Zaman geçersiz"));
        let r = e.p.handle(&get("/talep/paket", &[("bas", "2026-09-27"), ("bit", "2026-09-29")], &tok));
        assert!(r.file.is_some());
        let r = e.p.handle(&get("/talep/paket", &[("bas", "27.09.2026"), ("bit", "29.09.2026")], &tok));
        assert!(r.file.is_some() && r.headers.iter().any(|(_, v)| v.contains("talep_2026-09-27_2026-09-29.tar")), "gün.ay.yıl kabul, dosya adı ISO");
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
            peers: e.root.join("peers"),
            durum: e.root.join("durum"),
            hiz_uygulanan: e.root.join("hiz_uygulanan"),
            uzak_key: e.root.join("wg.key"),
            uzak_conf: e.root.join("wireguard/wfc.conf"),
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
        // açık yan oda varken: port değişikliği önce yan odayı kapatır (köprü yeniden kurulunca macvlan gider)
        std::fs::write(&e.p.ap_yollar.gecici, r#"{"adres":"192.168.1.250/24","kimlik":"0a1b2c3d","bitis":1790706934.0}"#).unwrap();
        *e.ip_ek.lock().unwrap() = "araci\n".into();
        // portları değiştir + Wi-Fi aç
        let r = post(&[("rol_enp3s0", "verir"), ("rol_enp1s0", "alir"), ("wifi", "verir"), ("ssid", "Bocafe Misafir"), ("sifre", ""), ("kanal", "11")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(!e.p.ap_yollar.gecici.exists() && e.calls.lock().unwrap().iter().any(|c| c.join(" ") == "systemctl stop wfc-ap-araci.service"));
        let dn = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        assert!(dn.contains("kapandi adres=192.168.1.250/24 neden=port"));
        e.ip_ek.lock().unwrap().clear();
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

    // ---------------------------------------------------------------- Portlar → PPPoE kutusu

    /// Portlar sayfası ortamı: geçici /sys (Ethernet 1 kablolu, Ethernet 2 kablosuz değil), geçici ağ yolları, işletme sahibi oturumu.
    /// `pppoe`: ag.toml'daki kayıtlı PPPoE ayarı (None: ag.toml yok, varsayılan ayar).
    fn portlar_ortami(pppoe: Option<crate::ag::Pppoe>) -> (Env, String, String) {
        let mut e = env();
        let sys = e.root.join("sys");
        for (n, mac, up) in [("enp3s0", "00:0e:c4:ce:a0:9b", "1"), ("enp1s0", "00:0e:c4:ce:a0:9a", "0")] {
            std::fs::create_dir_all(sys.join(n).join("device")).unwrap();
            std::fs::write(sys.join(n).join("address"), format!("{mac}\n")).unwrap();
            std::fs::write(sys.join(n).join("carrier"), up).unwrap();
            std::fs::write(sys.join(n).join("speed"), "1000").unwrap();
        }
        e.p.sys = sys;
        e.p.ag_yollar = crate::ag::Yollar {
            ag: e.root.join("ag.toml"),
            interfaces: e.root.join("interfaces"),
            nft: e.root.join("arayuzler.nft"),
            hostapd: e.root.join("hostapd.conf"),
            issue: e.root.join("issue"),
            peers: e.root.join("peers"),
            durum: e.root.join("durum"),
            hiz_uygulanan: e.root.join("hiz_uygulanan"),
            uzak_key: e.root.join("wg.key"),
            uzak_conf: e.root.join("wireguard/wfc.conf"),
        };
        if let Some(p) = pppoe {
            crate::ag::save(&e.p.ag_yollar.ag, &crate::ag::Ag { pppoe: p, ..Default::default() }).unwrap();
        }
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        (e, tok, csrf)
    }

    /// Her alanı varsayılandan farklı, açık bir PPPoE ayarı (parola dahil)
    fn pppoe_kayitli() -> crate::ag::Pppoe {
        crate::ag::Pppoe {
            acik: true,
            kullanici: "5321234567@turk.net".into(),
            parola: "gizli-parola-9".into(),
            vlan: 35,
            oncelik: 3,
            mtu: 1400,
            kimlik: "chap".into(),
            ac_adi: "AC1".into(),
            servis_adi: "srv1".into(),
        }
    }

    /// Portlar formunu gönderir: portlar aynı kalır (Ethernet 1 alır, Ethernet 2 verir, Wi-Fi kapalı) + verilen PPPoE alanları
    fn portlar_gonder(e: &Env, tok: &str, csrf: &str, pppoe: &[(&str, &str)]) -> Resp {
        let mut f = vec![("csrf", csrf), ("rol_enp3s0", "alir"), ("rol_enp1s0", "verir"), ("wifi", "kapali"), ("kanal", "6")];
        f.extend_from_slice(pppoe);
        e.p.handle(&req("POST", "/portlar", &f, Some(tok)))
    }

    /// `name="…"` taşıyan giriş etiketinin tamamı (`<input …>`, `<select …>` açılışı)
    fn etiket<'a>(html: &'a str, ad: &str) -> &'a str {
        let i = html.find(&format!("name=\"{ad}\"")).unwrap_or_else(|| panic!("{ad} alanı yok:\n{html}"));
        let bas = html[..i].rfind('<').unwrap();
        &html[bas..i + html[i..].find('>').unwrap() + 1]
    }

    /// `name="…"` taşıyan seçim kutusunun tamamı (`<select …>…</select>`)
    fn secim_html<'a>(html: &'a str, ad: &str) -> &'a str {
        let i = html.find(&format!("name=\"{ad}\"")).unwrap_or_else(|| panic!("{ad} alanı yok:\n{html}"));
        let bas = html[..i].rfind("<select").unwrap_or_else(|| panic!("{ad} seçim kutusu değil"));
        &html[bas..i + html[i..].find("</select>").unwrap() + "</select>".len()]
    }

    /// Günün denetim kaydındaki son `olay` satırı (yoksa boş)
    fn denetim_satiri(e: &Env, olay: &str) -> String {
        let t = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        t.lines().filter(|l| l.contains(&format!(";{olay};"))).last().unwrap_or("").to_string()
    }

    fn systemd_run_var(e: &Env) -> bool {
        e.calls.lock().unwrap().iter().any(|c| c[0] == "systemd-run")
    }

    #[test]
    fn portlar_pppoe_kutusu() {
        let (e, tok, _) = portlar_ortami(Some(pppoe_kayitli()));
        let page = e.p.handle(&req("GET", "/portlar", &[], Some(&tok))).body;
        // Açık/Kapalı, kullanıcı adı (örnek yer tutucu, kayıtlı değer), parola (yalnızca yazılır), VLAN
        assert!(secim_html(&page, "pppoe_acik").contains("<option value=\"acik\" selected>"), "{page}");
        let k = etiket(&page, "pppoe_kullanici");
        assert!(k.contains("placeholder=\"5321234567@turk.net\"") && k.contains("value=\"5321234567@turk.net\""), "{k}");
        let p = etiket(&page, "pppoe_parola");
        assert!(p.contains("type=\"password\"") && p.contains("value=\"\""), "{p}");
        assert!(etiket(&page, "pppoe_vlan").contains("value=\"35\""));
        // Gelişmiş'in altında: öncelik, MTU, kimlik doğrulama (Otomatik/PAP/CHAP), AC adı, servis adı
        let (once, sonra) = page.split_once("Gelişmiş").expect("Gelişmiş bölümü");
        for ad in ["pppoe_acik", "pppoe_kullanici", "pppoe_parola", "pppoe_vlan"] {
            assert!(once.contains(&format!("name=\"{ad}\"")), "{ad} Gelişmiş'in üstünde olmalı");
        }
        for ad in ["pppoe_oncelik", "pppoe_mtu", "pppoe_kimlik", "pppoe_ac_adi", "pppoe_servis_adi"] {
            assert!(sonra.contains(&format!("name=\"{ad}\"")) && !once.contains(&format!("name=\"{ad}\"")), "{ad} Gelişmiş'in altında olmalı");
        }
        assert!(etiket(sonra, "pppoe_oncelik").contains("value=\"3\"") && etiket(sonra, "pppoe_mtu").contains("value=\"1400\""));
        let kimlik = secim_html(sonra, "pppoe_kimlik");
        assert!(kimlik.contains(">Otomatik<") && kimlik.contains(">PAP<") && kimlik.contains(">CHAP<") && kimlik.contains("value=\"chap\" selected"), "{kimlik}");
        assert!(etiket(sonra, "pppoe_ac_adi").contains("value=\"AC1\"") && etiket(sonra, "pppoe_servis_adi").contains("value=\"srv1\""));
        // durum satırı (sahte komutlar boş: adres de pppd günlüğü de yok) ve dükkân notu
        assert!(page.contains("Bağlanıyor\u{2026}"), "{page}");
        assert!(page.contains("Dükkânda PPPoE bağlanmaz; ONT'ye takılınca bağlanır."), "{page}");
        // kayıtlı parola sayfada hiçbir yerde yok
        assert!(!page.contains("gizli-parola-9"), "parola sayfaya yazılmış");
        // menüden gelinen yol: Ayarlar sayfasında Portlar bölümü olarak aynı kutu; parola orada da yok
        let ayarlar = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(ayarlar.contains("name=\"pppoe_kullanici\"") && ayarlar.contains("Dükkânda PPPoE bağlanmaz") && !ayarlar.contains("gizli-parola-9"));
    }

    #[test]
    fn portlar_pppoe_durum_satiri() {
        const DHCP: &str = "3: enp3s0    inet 192.168.1.110/24 brd 192.168.1.255 scope global dynamic noprefixroute enp3s0\\       valid_lft 3101sec preferred_lft 2651sec\n";
        const PPP: &str = "12: wfcppp    inet 203.0.113.10 peer 198.51.100.1/32 scope global wfcppp\\       valid_lft forever preferred_lft forever\n";
        // sahte komutlar: wfcppp adresi, internet portunun adresi, pppd birim günlüğü (komut çalışmazsa None)
        let sahte = |ppp: &'static str, dhcp: &'static str, gunluk: &'static str| -> Box<dyn Fn(&[&str]) -> Option<String> + Send + Sync> {
            Box::new(move |c: &[&str]| match c {
                ["ip", "-4", "-o", "addr", "show", "dev", "wfcppp"] => Some(ppp.to_string()),
                ["ip", "-4", "-o", "addr", "show", "dev", "enp3s0"] => Some(dhcp.to_string()),
                ["journalctl", ..] => Some(gunluk.to_string()),
                _ => None,
            })
        };
        let hicbiri: Box<dyn Fn(&[&str]) -> Option<String> + Send + Sync> = Box::new(|_: &[&str]| None);
        let (mut e, tok, _) = portlar_ortami(None);
        for (acik, kablo, komut, beklenen) in [
            // iyi durumlar yeşil nokta
            (false, "1", sahte("", DHCP, ""), "<span class=\"durum\">PPPoE kapalı (modemden DHCP)</span>"),
            (true, "1", sahte(PPP, DHCP, ""), "<span class=\"durum\">PPPoE bağlı (dış IP 203.0.113.10)</span>"),
            // PPPoE açık, modemde (sunucu yok ya da deneme sürüyor) ama internet modemden geliyor: sorun değil
            (true, "1", sahte("", DHCP, "Timeout waiting for PADO packets\nUnable to complete PPPoE Discovery phase 1\n"), "<span class=\"durum\">Modemden bağlı (DHCP); PPPoE ONT&#x27;ye takılınca bağlanır</span>"),
            (true, "1", sahte("", DHCP, ""), "<span class=\"durum\">Modemden bağlı (DHCP); PPPoE ONT&#x27;ye takılınca bağlanır</span>"),
            // bağlanma sürerken düz metin
            (true, "1", hicbiri, "<span>Bağlanıyor\u{2026}</span>"),
            (true, "1", sahte("", "", ""), "<span>Bağlanıyor\u{2026}</span>"),
            // sorunlar kırmızı
            (false, "1", sahte("", "", ""), "<span class=\"durum kotu\">Modemden adres alınamadı (modem kapalı ya da kablo yanlış porta takılı)</span>"),
            (false, "0", sahte("", DHCP, ""), "<span class=\"durum kotu\">Kablo takılı değil</span>"),
            (true, "0", sahte(PPP, DHCP, ""), "<span class=\"durum kotu\">Kablo takılı değil</span>"),
            (true, "1", sahte("", "", "Timeout waiting for PADO packets\nUnable to complete PPPoE Discovery phase 1\n"), "<span class=\"durum kotu\">PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)</span>"),
            (true, "1", sahte("", "", "CHAP authentication failed: Access denied\n"), "<span class=\"durum kotu\">Kullanıcı adı ya da parola reddedildi</span>"),
            (true, "1", sahte("", DHCP, "CHAP authentication failed: Access denied\n"), "<span class=\"durum kotu\">Kullanıcı adı ya da parola reddedildi</span>"),
        ] {
            // aynı oturumda: her vakada kayıtlı ayar, kablo ve sahte komutlar değişir (giriş parolası özeti yavaş, bir kez yeter)
            crate::ag::save(&e.p.ag_yollar.ag, &crate::ag::Ag { pppoe: crate::ag::Pppoe { acik, ..pppoe_kayitli() }, ..Default::default() }).unwrap();
            std::fs::write(e.p.sys.join("enp3s0/carrier"), kablo).unwrap();
            e.p.komut = komut;
            let page = e.p.handle(&req("GET", "/portlar", &[], Some(&tok))).body;
            assert!(page.contains(&format!("Durum: {beklenen}")), "açık={acik} kablo={kablo}: {beklenen}");
        }
    }

    #[test]
    fn portlar_pppoe_bos_alanlar() {
        // kayıtlı ayarda VLAN, öncelik ve MTU varsayılandan farklı: boş gönderilince varsayılana döner (eskisi kalmaz)
        let (e, tok, csrf) = portlar_ortami(Some(pppoe_kayitli()));
        let r = portlar_gonder(
            &e,
            &tok,
            &csrf,
            &[
                ("pppoe_acik", "acik"),
                ("pppoe_kullanici", "5321234567@turk.net"),
                ("pppoe_parola", ""),
                ("pppoe_vlan", ""),
                ("pppoe_oncelik", ""),
                ("pppoe_mtu", ""),
                ("pppoe_kimlik", ""),
                ("pppoe_ac_adi", ""),
                ("pppoe_servis_adi", ""),
            ],
        );
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let p = crate::ag::load(&e.p.ag_yollar.yeni()).pppoe;
        assert!(p.acik, "hazırlanan ayar okunamadı");
        assert_eq!((p.vlan, p.oncelik, p.mtu), (0, 0, 1492));
        assert_eq!((p.kimlik.as_str(), p.ac_adi.as_str(), p.servis_adi.as_str()), ("otomatik", "", ""));
        assert_eq!(p.parola, "gizli-parola-9"); // parola alanı da boştu: eskisi kalır
    }

    #[test]
    fn portlar_pppoe_bos_parola_korunur() {
        let eski = crate::ag::Pppoe { acik: true, kullanici: "k1".into(), parola: "p1".into(), ..Default::default() };
        let (e, tok, csrf) = portlar_ortami(Some(eski));
        // boş parola: hazırlanan ag-yeni.toml'a ESKİ parola yazılır (ag::gecis boş parolayı birleştirmez)
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "k2"), ("pppoe_parola", ""), ("pppoe_vlan", "35")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let yeni = std::fs::read_to_string(e.p.ag_yollar.yeni()).unwrap();
        assert!(yeni.contains("parola = \"p1\"") && yeni.contains("kullanici = \"k2\""), "{yeni}");
        // yazılan parola eskisinin yerine geçer
        std::fs::remove_file(e.p.ag_yollar.yeni()).unwrap();
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "k2"), ("pppoe_parola", "p2"), ("pppoe_vlan", "35")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let yeni = std::fs::read_to_string(e.p.ag_yollar.yeni()).unwrap();
        assert!(yeni.contains("parola = \"p2\"") && !yeni.contains("parola = \"p1\""), "{yeni}");
    }

    #[test]
    fn portlar_pppoe_gecersiz() {
        let (e, tok, csrf) = portlar_ortami(None);
        let gecerli = [("pppoe_acik", "acik"), ("pppoe_kullanici", "k1"), ("pppoe_parola", "p1")];
        let bos = "PPPoE kullanıcı adı ve parolası boş olamaz.";
        let ad = "PPPoE AC adı en çok 64 karakter olmalı ve yalnızca harf, rakam, nokta, alt çizgi ve tire içerebilir.";
        let vlan = "VLAN 1-4094 arasında olmalı (boş: VLAN yok).";
        let karakter = "PPPoE kullanıcı adı ve parolası en çok 128 karakter olmalı; tırnak, ters eğik çizgi ve kontrol karakteri içeremez.";
        let mut vakalar: Vec<(Vec<(&str, &str)>, &str)> = vec![
            // açık + boş kullanıcı adı / parola (kayıtlı parola da yok)
            (vec![("pppoe_acik", "acik"), ("pppoe_kullanici", ""), ("pppoe_parola", "p1")], bos),
            (vec![("pppoe_acik", "acik"), ("pppoe_kullanici", "k1"), ("pppoe_parola", "")], bos),
            (vec![("pppoe_acik", "acik"), ("pppoe_kullanici", "   "), ("pppoe_parola", "p1")], bos), // yalnızca boşluk: kırpılınca boş
            (vec![("pppoe_acik", "acik"), ("pppoe_kullanici", "k\"1"), ("pppoe_parola", "p1")], karakter),
        ];
        for (alan, deger, mesaj) in [
            ("pppoe_vlan", "4095", vlan),
            ("pppoe_vlan", "abc", vlan),
            ("pppoe_vlan", "-1", vlan),
            ("pppoe_oncelik", "8", "VLAN önceliği 0-7 arasında olmalı."),
            ("pppoe_oncelik", "x", "VLAN önceliği 0-7 arasında olmalı."),
            ("pppoe_mtu", "1000", "PPPoE MTU değeri 1280-1492 arasında olmalı."),
            ("pppoe_mtu", "1493", "PPPoE MTU değeri 1280-1492 arasında olmalı."),
            ("pppoe_mtu", "abc", "PPPoE MTU değeri 1280-1492 arasında olmalı."),
            ("pppoe_kimlik", "radius", "Kimlik doğrulama yöntemi otomatik, PAP ya da CHAP olmalı."),
            ("pppoe_ac_adi", "bir ad", ad),
        ] {
            let mut f = gecerli.to_vec();
            f.push((alan, deger));
            vakalar.push((f, mesaj));
        }
        for (alanlar, mesaj) in &vakalar {
            let r = portlar_gonder(&e, &tok, &csrf, alanlar);
            assert!(loc(&r).contains("e=1") && loc(&r).contains(&pct(mesaj)), "{alanlar:?}: {}", loc(&r));
        }
        // hiçbiri uygulanmaya başlamadı
        assert!(!systemd_run_var(&e) && !e.p.ag_yollar.yeni().exists());
    }

    #[test]
    fn portlar_pppoe_denetim() {
        let (e, tok, csrf) = portlar_ortami(None);
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "5321234567@turk.net"), ("pppoe_parola", "gizli-parola-9"), ("pppoe_vlan", "35")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let satir = denetim_satiri(&e, "PANEL_PORT");
        assert!(satir.contains("pppoe=acik vlan=35 kullanici_degisti=1 parola_degisti=1"), "{satir}");
        assert!(satir.contains("PPPoE açık (VLAN 35)"), "mevcut özet metni korunur: {satir}");
        for gizli in ["5321234567", "turk.net", "gizli-parola-9"] {
            assert!(!satir.contains(gizli), "denetime {gizli} yazılmış: {satir}");
        }
        // değişiklik uygulanmış sayılsın (gecis ag.toml'u yazar); bu kez kullanıcı adı aynı, parola boş, yalnızca VLAN değişir
        std::fs::remove_file(e.p.ag_yollar.yeni()).unwrap();
        crate::ag::save(&e.p.ag_yollar.ag, &crate::ag::Ag { pppoe: crate::ag::Pppoe { acik: true, kullanici: "5321234567@turk.net".into(), parola: "gizli-parola-9".into(), vlan: 35, ..Default::default() }, ..Default::default() }).unwrap();
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "5321234567@turk.net"), ("pppoe_parola", ""), ("pppoe_vlan", "7")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(denetim_satiri(&e, "PANEL_PORT").contains("pppoe=acik vlan=7 kullanici_degisti=0 parola_degisti=0"));
        // kapatma: kullanıcı adı ve ileri alanlar kalır (form kayıtlı değerlerle dolu gelir)
        std::fs::remove_file(e.p.ag_yollar.yeni()).unwrap();
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "kapali"), ("pppoe_kullanici", "5321234567@turk.net"), ("pppoe_parola", ""), ("pppoe_vlan", "35")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let satir = denetim_satiri(&e, "PANEL_PORT");
        assert!(satir.contains("pppoe=kapali vlan=35 kullanici_degisti=0 parola_degisti=0") && !satir.contains("PPPoE açık"), "{satir}");
        let kapali = crate::ag::load(&e.p.ag_yollar.yeni()).pppoe;
        assert_eq!((kapali.acik, kapali.kullanici.as_str(), kapali.parola.as_str()), (false, "5321234567@turk.net", "gizli-parola-9"));
    }

    #[test]
    fn portlar_pppoe_alani_olmayan_gonderim_pppoe_ya_dokunmaz() {
        // PPPoE alanı taşımayan bir Portlar gönderimi (betik, ileride başka bir form): kayıtlı PPPoE ayarı (açık, kullanıcı adı,
        // ileri alanlar) aynen kalır; yalnızca gönderilen port/Wi-Fi değişikliği uygulanır
        let (e, tok, csrf) = portlar_ortami(Some(pppoe_kayitli()));
        let r = portlar_gonder(&e, &tok, &csrf, &[]);
        assert!(!loc(&r).contains("e=1") && loc(&r).contains("%20yok."), "değişiklik yok sayılmalı: {}", loc(&r));
        assert!(!systemd_run_var(&e) && !e.p.ag_yollar.yeni().exists());
        // aynı gönderimde Wi-Fi açılırsa: değişiklik uygulanır, hazırlanan ayarda PPPoE kayıtlı haliyle durur
        let r = portlar_gonder(&e, &tok, &csrf, &[("wifi", "verir"), ("ssid", "Misafir")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let yeni = crate::ag::load(&e.p.ag_yollar.yeni());
        assert!(yeni.wifi.acik && yeni.wifi.ssid == "Misafir");
        assert_eq!(yeni.pppoe, pppoe_kayitli());
        assert!(denetim_satiri(&e, "PANEL_PORT").contains("pppoe=acik vlan=35 kullanici_degisti=0 parola_degisti=0"));
        // PPPoE seçimi gönderilince (sayfadaki form hep gönderir) alanlar formdan okunur: Kapalı seçilirse kapanır
        std::fs::remove_file(e.p.ag_yollar.yeni()).unwrap();
        let r = portlar_gonder(&e, &tok, &csrf, &[("pppoe_acik", "kapali"), ("pppoe_kullanici", "5321234567@turk.net"), ("pppoe_vlan", "35")]);
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(!crate::ag::load(&e.p.ag_yollar.yeni()).pppoe.acik);
    }

    #[test]
    fn portlar_pppoe_degisiklik_tespiti() {
        let (e, tok, csrf) = portlar_ortami(Some(pppoe_kayitli()));
        // sayfadaki form kayıtlı değerlerle dolu gelir (parola boş): hiçbir şey değişmemiş, hiçbir iş başlamaz
        let ayni = [
            ("pppoe_acik", "acik"),
            ("pppoe_kullanici", "5321234567@turk.net"),
            ("pppoe_parola", ""),
            ("pppoe_vlan", "35"),
            ("pppoe_oncelik", "3"),
            ("pppoe_mtu", "1400"),
            ("pppoe_kimlik", "chap"),
            ("pppoe_ac_adi", "AC1"),
            ("pppoe_servis_adi", "srv1"),
        ];
        let r = portlar_gonder(&e, &tok, &csrf, &ayni);
        assert!(!loc(&r).contains("e=1") && loc(&r).contains("%20yok."), "{}", loc(&r));
        assert!(!systemd_run_var(&e) && !e.p.ag_yollar.yeni().exists());
        // yalnızca bir PPPoE alanı değişse de değişikliktir: mevcut 3 dk onay akışına girer
        let mut vlanli = ayni.to_vec();
        vlanli[3] = ("pppoe_vlan", "36");
        let r = portlar_gonder(&e, &tok, &csrf, &vlanli);
        assert!(!loc(&r).contains("e=1") && !loc(&r).contains("%20yok."), "{}", loc(&r));
        assert!(e.calls.lock().unwrap().iter().any(|c| c.join(" ").contains("ctl ag-gecis")));
        let yeni = crate::ag::load(&e.p.ag_yollar.yeni());
        assert_eq!(yeni.pppoe.vlan, 36);
        assert_eq!(crate::ag::Ag { pppoe: crate::ag::Pppoe { vlan: 35, ..yeni.pppoe.clone() }, ..yeni.clone() }, crate::ag::load(&e.p.ag_yollar.ag), "başka hiçbir ayar değişmemeli");
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
    fn hazir_listeler_ac_kapa() {
        use std::io::Write;
        let e = env();
        let y = &e.p.liste_yollar;
        std::fs::create_dir_all(&y.dizin).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"bet365.com\nbets10.com\n").unwrap();
        std::fs::write(y.dizin.join("kumar.txt.gz"), gz.finish().unwrap()).unwrap();
        std::fs::write(y.dizin.join("kumar.json"), r#"{"sha256":"x","sayi":2,"zaman":"2026-10-14T03:00:00+03:00"}"#).unwrap();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&req("GET", "/yasakli-siteler", &[], Some(&tok))).body;
        assert!(page.contains("Hazır listeler") && page.contains("Kumar ve bahis") && page.contains("Yetişkin içerik") && page.contains("Zararlı ve dolandırıcı siteler"));
        assert!(page.contains("14.10.2026") && page.contains("Henüz indirilmedi") && page.matches("<b>Açık</b>").count() == 3); // varsayılan açık
        let page = e.p.handle(&get("/yasakli-siteler", &[("ad", "m.bet365.com")], &tok)).body;
        assert!(page.contains("engelli (hazır liste: Kumar ve bahis)"));
        // CSRF'siz istek reddedilir
        assert_eq!(e.p.handle(&req("POST", "/yasakli-siteler/hazir", &[("kategori", "kumar"), ("durum", "kapali")], Some(&tok))).status, 403);
        assert!(Config::load(&e.p.cfg_path).unwrap().filtre.hazir.is_empty());
        let hazir = |k: &str, d: &str| e.p.handle(&req("POST", "/yasakli-siteler/hazir", &[("csrf", &csrf), ("kategori", k), ("durum", d)], Some(&tok)));
        assert!(loc(&hazir("vpn", "kapali")).contains("e=1"));
        assert!(loc(&hazir("kumar", "belki")).contains("e=1"));
        // panel isteği birleşik listeyi kendisi kurmaz (~1 milyon satır): ayarı yazar, sabit adlı işi başlatır, hemen döner
        let uygula_isi = "systemd-run --collect --unit wfc-liste-uygula /usr/local/bin/wificorrect ctl liste-uygula";
        let cagrildi = |x: &str| e.calls.lock().unwrap().iter().any(|c| c.join(" ") == x);
        let is = || crate::liste::uygula_guncel(&e.p.cfg_path, &Config::load(&e.p.cfg_path).unwrap(), y, &|_: &[String]| true).unwrap(); // systemd-run'ın yapacağı
        assert!(!loc(&hazir("kumar", "kapali")).contains("e=1"));
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().filtre.hazir.get("kumar"), Some(&false));
        assert!(!y.conf.exists() && cagrildi(uygula_isi));
        is();
        assert!(!std::fs::read_to_string(&y.conf).unwrap().contains("server=")); // kapatınca iş hemen açar
        assert!(e.p.handle(&get("/yasakli-siteler", &[("ad", "bet365.com")], &tok)).body.contains("engelli değil"));
        assert!(!e.calls.lock().unwrap().iter().any(|c| c.contains(&"liste-guncelle".to_string())));
        e.calls.lock().unwrap().clear();
        assert!(!loc(&hazir("kumar", "acik")).contains("e=1"));
        assert!(!std::fs::read_to_string(&y.conf).unwrap().contains("server="));
        // eldeki liste hemen (uygulama işi), merkezdeki daha yenisi ayrı işte
        assert!(cagrildi(uygula_isi) && cagrildi("systemd-run --collect --unit wfc-liste /usr/local/bin/wificorrect ctl liste-guncelle"));
        is();
        assert!(std::fs::read_to_string(&y.conf).unwrap().contains("server=/bet365.com/\nserver=/bets10.com/\n"));
        // istisna eklenince hazır listeden de çıkar (yine iş üzerinden)
        e.calls.lock().unwrap().clear();
        e.p.handle(&req("POST", "/yasakli-siteler/ekle", &[("csrf", &csrf), ("liste", "istisna"), ("deger", "bets10")], Some(&tok)));
        assert!(cagrildi(uygula_isi) && std::fs::read_to_string(&y.conf).unwrap().contains("bets10"));
        is();
        let conf = std::fs::read_to_string(&y.conf).unwrap();
        assert!(conf.contains("server=/bet365.com/") && !conf.contains("bets10"));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_HAZIR_LISTE") && audit.contains("kategori=kumar durum=kapali") && audit.contains("kategori=kumar durum=acik"));
    }

    #[test]
    fn owner_activity_is_logged_and_only_admin_sees_it() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        e.p.handle(&get("/oturumlar", &[], &tok));
        e.p.handle(&get("/kullanicilar", &[], &tok));
        e.p.handle(&req("POST", "/kullanicilar", &[("csrf", &csrf), ("q", "ayşe")], Some(&tok)));
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
        assert!(page.contains("<td>29.09.2026 21:05:34</td>"), "hareket zamanı saniyeli gün.ay.yıl");
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
        let r = e.p.handle(&get("/kayitlar/gun", &[("gun", "2026-09-29")], &tok));
        assert!(r.body.contains("Gerekçe gerekli") && r.body.contains("value=\"/kayitlar/gun?gun=2026-09-29\"")); // dönüş adresi korunur
        let r = e.p.handle(&req("POST", "/kullanicilar", &[("csrf", &csrf), ("q", "ayşe")], Some(&tok)));
        assert!(r.body.contains("Gerekçe gerekli") && r.body.contains("value=\"/kullanicilar\"")); // aranan değer dönüş adresine girmez
        assert!(e.p.handle(&get("/kullanici", &[("k", "0123456789abcdef")], &tok)).body.contains("Gerekçe gerekli"));
        assert!(!e.p.handle(&get("/talep", &[], &tok)).body.contains("Gerekçe gerekli")); // boş talep formu açılır
        let r = e.p.handle(&req("POST", "/talep", &[("csrf", &csrf), ("tur", "tc"), ("deger", "10000000146")], Some(&tok)));
        assert!(r.body.contains("Gerekçe gerekli") && !r.body.contains("10000000146"));
        assert!(!e.p.handle(&get("/oturumlar", &[], &tok)).body.contains("Gerekçe gerekli")); // bağlı cihazlar gerekçesiz
        // kısa gerekçe olmaz; başka siteye dönüş olmaz
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "bakıyorum"), ("donus", "/kullanicilar")], Some(&tok)));
        assert!(r.body.contains("10-200 karakter"));
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "Emniyet / savcılık talebi"), ("donus", "//kotu.example/x")], Some(&tok)));
        assert_eq!(loc(&r), "/");
        let r = e.p.handle(&req("POST", "/gerekce", &[("csrf", &csrf), ("gerekce", "Emniyet / savcılık talebi"), ("donus", "/kullanicilar")], Some(&tok)));
        assert_eq!(loc(&r), "/kayitlar#kullanicilar"); // eski yol grup sayfasındaki bölüme gider
        assert!(!e.p.handle(&req("POST", "/kullanicilar", &[("csrf", &csrf), ("q", "ayşe")], Some(&tok))).body.contains("Gerekçe gerekli"));
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
    fn portal_metinleri_ingilizce_alanlar() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&get("/portal-metinleri", &[], &tok)).body;
        assert!(page.contains("KVKK Aydınlatma Metni (İngilizce)") && page.contains("name=\"acik_riza_en\"") && page.contains("name=\"sozlesme_en\""));
        let c0 = Config::load(&e.p.cfg_path).unwrap();
        let (a, r, s) = (c0.portal.aydinlatma.clone(), c0.portal.acik_riza.clone(), c0.portal.sozlesme.clone());
        let f = [("csrf", csrf.as_str()), ("aydinlatma", a.as_str()), ("acik_riza", r.as_str()), ("sozlesme", s.as_str()), ("aydinlatma_en", "Data controller: <X>"), ("acik_riza_en", "I consent"), ("sozlesme_en", "")];
        assert!(!loc(&e.p.handle(&req("POST", "/portal-metinleri", &f, Some(&tok)))).contains("e=1"));
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.portal.aydinlatma_en.as_str(), c.portal.acik_riza_en.as_str(), c.portal.sozlesme_en.as_str()), ("Data controller: <X>", "I consent", ""));
        assert_eq!(c.portal.aydinlatma, a); // Türkçeler değişmedi
        assert!(e.p.handle(&get("/portal-metinleri", &[], &tok)).body.contains("Data controller: &lt;X&gt;"));
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_PORTAL_METIN") && audit.contains("aydinlatma_en(20 karakter) acik_riza_en(9 karakter)") && !audit.contains("sozlesme_en("));
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
        for gorunur in ["backup.target", "main.retention_days", "limits.sms_global_day", "Doğrulama SMS'leri yönetim merkezinden gönderilir", "Fabrika ayarları"] {
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
        assert!(page.contains("29.09.2026 02:00 — HATA: zincir.txt sunucuya gönderilemedi"));
        assert!(page.contains("Son fabrika dönüşü <b>iptal oldu</b> (29.09.2026 03:00): 2026-09-28 sunucuya gönderilemedi"));
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
        assert_eq!(menu_linkleri(&ozet), ["/", "/cihazlar", "/kayitlar", "/ayarlar"]);
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("action=\"/admin-kilidi\"") && !a.contains("name=\"limits.sms_global_day\""));
        // Admin sol menüde yok: Ayarlar'ın en son kutucuğu; admin sayfasında menüde Ayarlar seçili
        let ay = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        let kutular = ay.split("aria-label=\"Bu sayfadaki bölümler\">").nth(1).unwrap().split("</nav>").next().unwrap().to_string();
        assert!(kutular.ends_with("<b>Admin</b><small>Hizmet sağlayıcının ayarları</small></span></a>") && kutular.contains("href=\"/admin\""), "{kutular}");
        assert!(a.contains("href=\"/ayarlar\" class=\"aktif\""), "admin sayfasında Ayarlar vurgulu");
        assert!(!e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body.contains("href=\"/admin\">"), "başka grupta Admin kutucuğu yok");
        assert!(e.p.handle(&req("GET", "/panel-hareketleri", &[], Some(&tok))).body.contains("action=\"/admin-kilidi\""));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "yanlis-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert!(r.body.contains("hatalı"));
        let r = e.p.handle(&req("POST", "/admin-kilidi", &[("csrf", &csrf), ("parola", "hizmet-parola-1"), ("donus", "/admin")], Some(&tok)));
        assert_eq!(loc(&r), "/admin");
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("name=\"limits.sms_global_day\"") && a.contains("id=\"panel-hareketleri\""));
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
        assert!(e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("name=\"limits.sms_global_day\""));
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
        assert!(r.status == 200 && !e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body.contains("name=\"limits.sms_global_day\""));
    }

    #[test]
    fn lisans_ve_unvan_uyarisi() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let page = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(!page.contains("Lisans askıya") && !page.contains("veri sorumlusu bilgileri eksik"));
        let mut m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
        let page = e.p.handle(&req("GET", "/", &[], Some(&tok))).body;
        assert!(page.contains("Lisans askıya alındı") && page.contains("misafirlere internet verilmiyor"));
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.unvan = String::new();
        c.save(&e.p.cfg_path).unwrap();
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("KVKK metninde veri sorumlusu bilgileri eksik (unvan)"));
        // adres de boş: ikisi birden; isteğe bağlı alanlar (telefon, e-posta, KEP) uyarı vermez
        c.main.adres = String::new();
        c.save(&e.p.cfg_path).unwrap();
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("veri sorumlusu bilgileri eksik (unvan, adres)"));
        c.main.unvan = "Boca Ltd.".into();
        c.save(&e.p.cfg_path).unwrap();
        assert!(e.p.handle(&req("GET", "/", &[], Some(&tok))).body.contains("veri sorumlusu bilgileri eksik (adres)"));
    }

    // --- yavaşlatma

    const AYSE: &str = "aa:bb:cc:dd:ee:01";
    const HIZ_UYGULA: [&str; 3] = ["/usr/local/bin/wificorrect", "ctl", "hiz-uygula"];

    fn sinirlar(e: &Env) -> crate::hiz::Sinirlar {
        crate::hiz::oku(&Config::load(&e.p.cfg_path).unwrap().main.state_root)
    }

    fn denetim_satirlari(e: &Env, olay: &str) -> Vec<String> {
        let csv = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        csv.lines().filter(|l| l.split(';').nth(1) == Some(olay)).map(String::from).collect()
    }

    fn hiz_uygulandi(e: &Env) -> usize {
        let beklenen: Vec<String> = HIZ_UYGULA.iter().map(|s| s.to_string()).collect();
        e.calls.lock().unwrap().iter().filter(|c| **c == beklenen).count()
    }

    fn bolum_of(s: &str, id: &str) -> String {
        s.split(&format!("<section class=\"bolum\" id=\"{id}\"")).nth(1).unwrap_or_else(|| panic!("{id} bölümü yok")).split("</section>").next().unwrap().to_string()
    }

    #[test]
    fn yavaslat_kaydeder_ve_uygular() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        trafikli(&e, &[(AYSE, "Ayşe", 1.0)], &[], None);
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        let r = satir_of(&s, "Ayşe");
        assert!(r.contains("<details class=\"yavas\"><summary class=\"dugme ikincil\">Yavaşlat</summary><div class=\"yavas-menu\"><span class=\"not\">Hız sınırı seçin</span>"), "{r}");
        for n in [1, 2, 5, 10, 20] {
            assert!(r.contains(&format!("<input type=\"hidden\" name=\"mac\" value=\"{AYSE}\"><input type=\"hidden\" name=\"hiz\" value=\"{n}\"><button class=\"ikincil\">{n} Mb/sn</button>")), "{n}: {r}");
        }
        assert_eq!(r.matches("action=\"/oturumlar/yavaslat\"").count(), 5);
        assert!(!r.contains("rozet sinirli"));

        let y = e.p.handle(&req("POST", "/oturumlar/yavaslat", &[("csrf", &csrf), ("mac", "AA-BB-CC-DD-EE-01"), ("hiz", "5")], Some(&tok)));
        assert!(loc(&y).starts_with("/cihazlar?m=") && loc(&y).ends_with("#oturumlar") && !loc(&y).contains("e=1"), "{}", loc(&y));
        let k = sinirlar(&e);
        let sn = k.get(AYSE).expect("kayıt");
        assert_eq!((sn.hiz, sn.ad.as_str(), sn.kim.as_str()), (5, "Ayşe Y", MUSTERI));
        assert!(sn.zaman.starts_with("2026-09-29T"), "{}", sn.zaman);
        assert_eq!(hiz_uygulandi(&e), 1);
        let d = denetim_satirlari(&e, "PANEL_YAVASLAT");
        assert!(d.len() == 1 && d[0].contains(&format!("mac={AYSE} hiz=5")), "{d:?}");
        assert_eq!(hareketler::olay_adi("PANEL_YAVASLAT"), "Yavaşlattı");

        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        let r = satir_of(&s, "Ayşe");
        assert!(r.contains("<span class=\"rozet sinirli\">≤ 5 Mb/sn</span>"), "{r}");
        assert!(r.contains("action=\"/oturumlar/yavaslatma-kaldir\"") && r.contains("name=\"donus\" value=\"oturumlar\"") && r.contains(">Yavaşlatmayı kaldır</button>"), "{r}");
        assert!(!r.contains("<details class=\"yavas\">") && !r.contains(">Yavaşlat</summary>"), "{r}");
        // rozet kartın kapalı satırında, kaldır düğmesi açılınca
        let (ozet, ic) = r.split_once("</summary>").unwrap();
        assert!(ozet.contains("≤ 5 Mb/sn") && ic.contains(">Yavaşlatmayı kaldır</button>") && !ic.contains("rozet sinirli"), "{r}");

        // kaldır (satırdan): oturumlara döner
        let k = e.p.handle(&req("POST", "/oturumlar/yavaslatma-kaldir", &[("csrf", &csrf), ("mac", AYSE), ("donus", "oturumlar")], Some(&tok)));
        assert!(loc(&k).starts_with("/cihazlar?m=") && loc(&k).ends_with("#oturumlar") && !loc(&k).contains("e=1"), "{}", loc(&k));
        assert!(sinirlar(&e).is_empty());
        assert_eq!(hiz_uygulandi(&e), 2);
    }

    #[test]
    fn yavaslat_gecersiz_deger_reddedilir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        trafikli(&e, &[(AYSE, "Ayşe", 1.0)], &[], None);
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.allow = vec![Device { mac: "aa:bb:cc:dd:ee:99".into(), name: "Kasa".into() }, Device { mac: "aa:bb:cc:dd:ee:98".into(), name: String::new() }];
        c.save(&e.p.cfg_path).unwrap();
        std::fs::write(&c.main.leases_file, "1 aa:bb:cc:dd:ee:99 10.50.0.7 kasa 01\n").unwrap(); // ...:98 ağda değil
        for (mac, hiz) in [(AYSE, "3"), (AYSE, "abc"), (AYSE, ""), ("aa:bb:cc:dd:ee:77", "5"), ("aa:bb:cc:dd:ee:98", "5"), ("bozuk", "5")] {
            let r = e.p.handle(&req("POST", "/oturumlar/yavaslat", &[("csrf", &csrf), ("mac", mac), ("hiz", hiz)], Some(&tok)));
            assert!(loc(&r).contains("e=1") && loc(&r).ends_with("#oturumlar"), "{mac} {hiz}: {}", loc(&r));
        }
        let r = e.p.handle(&req("POST", "/oturumlar/yavaslat", &[("mac", AYSE), ("hiz", "5")], Some(&tok)));
        assert_eq!(r.status, 403);
        let r = e.p.handle(&req("POST", "/oturumlar/yavaslatma-kaldir", &[("csrf", &csrf), ("mac", AYSE), ("donus", "oturumlar")], Some(&tok)));
        assert!(loc(&r).contains("e=1"), "kayıtta olmayan kaldırılamaz");
        assert!(!std::path::Path::new(&c.main.state_root).join("hiz_sinir.json").exists() && hiz_uygulandi(&e) == 0);
        assert!(denetim_satirlari(&e, "PANEL_YAVASLAT").is_empty() && denetim_satirlari(&e, "PANEL_YAVASLATMA_KALDIR").is_empty());

        // izinli + kirada: olur; adı "İzinli cihaz · not"; satırda menü
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(satir_of(&s, "İzinli cihaz · Kasa").contains("<details class=\"yavas\">"));
        let r = e.p.handle(&req("POST", "/oturumlar/yavaslat", &[("csrf", &csrf), ("mac", "aa:bb:cc:dd:ee:99"), ("hiz", "2")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert_eq!(sinirlar(&e).get("aa:bb:cc:dd:ee:99").map(|s| (s.hiz, s.ad.clone())), Some((2, "İzinli cihaz · Kasa".to_string())));
        let s = e.p.handle(&req("GET", "/cihazlar", &[], Some(&tok))).body;
        assert!(satir_of(&s, "İzinli cihaz · Kasa").contains("<span class=\"rozet sinirli\">≤ 2 Mb/sn</span>"));
    }

    #[test]
    fn yavaslatilmis_bolumu_ve_kaldir() {
        let mut e = env();
        let saat = Arc::new(std::sync::atomic::AtomicU64::new(SAAT.to_bits()));
        let s2 = saat.clone();
        e.p.clock = Box::new(move || f64::from_bits(s2.load(Ordering::SeqCst)));
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let root = Config::load(&e.p.cfg_path).unwrap().main.state_root;
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        assert!(bolum_of(&s, "yavaslatilmis").contains("Yavaşlatılmış cihaz yok"));
        assert!(s.contains("href=\"#yavaslatilmis\""));
        assert_eq!(denetim_satirlari(&e, "PANEL_YAVASLATILMIS").len(), 1);
        assert_eq!(hareketler::olay_adi("PANEL_YAVASLATILMIS"), "Yavaşlatılmış cihazlara baktı");

        let mut k = crate::hiz::Sinirlar::new();
        k.insert("aa:bb:cc:dd:ee:55".into(), crate::hiz::Sinir { hiz: 5, ad: "Zeynep A".into(), zaman: "2026-09-29T21:04:00+03:00".into(), kim: "mudur".into() });
        crate::hiz::kaydet(&root, &k).unwrap();
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        let b = bolum_of(&s, "yavaslatilmis");
        for p in ["<code>aa:bb:cc:dd:ee:55</code>", "Zeynep A", "<td>5 Mb/sn</td>", "<td>29.09.2026 21:04</td>", "<td>mudur</td>", ">Yavaşlatmayı kaldır</button>", "name=\"donus\" value=\"yavaslatilmis\""] {
            assert!(b.contains(p), "{p}: {b}");
        }
        assert_eq!(denetim_satirlari(&e, "PANEL_YAVASLATILMIS").len(), 2);
        // oto yenilemede bakış yazılmaz
        e.p.handle(&get("/cihazlar", &[("yenile", "1")], &tok));
        assert_eq!(denetim_satirlari(&e, "PANEL_YAVASLATILMIS").len(), 3);
        saat.store((SAAT + 10.0).to_bits(), Ordering::SeqCst);
        e.p.handle(&get("/cihazlar", &[("sira", "hiz"), ("yenile", "1"), ("oto", "1")], &tok));
        assert_eq!(denetim_satirlari(&e, "PANEL_YAVASLATILMIS").len(), 3, "oto yenileme");

        let r = e.p.handle(&req("POST", "/oturumlar/yavaslatma-kaldir", &[("csrf", &csrf), ("mac", "aa:bb:cc:dd:ee:55"), ("donus", "yavaslatilmis")], Some(&tok)));
        assert!(loc(&r).starts_with("/cihazlar?m=") && loc(&r).ends_with("#yavaslatilmis") && !loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(crate::hiz::oku(&root).is_empty());
        assert_eq!(hiz_uygulandi(&e), 1);
        let d = denetim_satirlari(&e, "PANEL_YAVASLATMA_KALDIR");
        assert!(d.len() == 1 && d[0].contains("mac=aa:bb:cc:dd:ee:55"), "{d:?}");
        assert_eq!(hareketler::olay_adi("PANEL_YAVASLATMA_KALDIR"), "Yavaşlatmayı kaldırdı");
        // bölüm sayfası ayrıca açılır
        let s = e.p.handle(&get("/yavaslatilmis", &[], &tok));
        assert!(s.status == 200 && s.body.contains("Yavaşlatılmış cihaz yok") && s.body.contains("href=\"/cihazlar\" class=\"aktif\""));
    }

    #[test]
    fn bozuk_sinir_dosyasi_uzerine_yazilmaz() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let mut ses = ortak::Sessions::new();
        ses.insert("aa:bb:cc:dd:ee:01".into(), ortak::Session {
            phone: "905334553132".into(), ad: "Ayşe".into(), soyad: "Y".into(), ip: "10.50.0.23".into(),
            session_id: "s".into(), start: "2026-09-29T20:00:00+03:00".into(), start_epoch: 1.0, expires_epoch: 9e9,
        });
        ortak::save_sessions(&cfg.main.state_root, &ses).unwrap();
        let yol = std::path::Path::new(&cfg.main.state_root).join("hiz_sinir.json");
        std::fs::write(&yol, "{\"aa:bb:cc:dd:ee:55\": {\"hiz\": 5, \"ad\"").unwrap(); // yarım dosya
        for (yol_, alanlar) in [
            ("/oturumlar/yavaslat", vec![("mac", "aa:bb:cc:dd:ee:01"), ("hiz", "5")]),
            ("/oturumlar/yavaslatma-kaldir", vec![("mac", "aa:bb:cc:dd:ee:55"), ("donus", "yavaslatilmis")]),
        ] {
            let mut f = vec![("csrf", csrf.as_str())];
            f.extend(alanlar);
            let r = e.p.handle(&req("POST", yol_, &f, Some(&tok)));
            assert!(loc(&r).contains("e=1") && loc(&r).contains("okunamad"), "{yol_}: {}", loc(&r));
        }
        assert_eq!(std::fs::read_to_string(&yol).unwrap(), "{\"aa:bb:cc:dd:ee:55\": {\"hiz\": 5, \"ad\"", "dosya değişmedi");
        assert_eq!(hiz_uygulandi(&e), 0);
        let b = bolum_of(&e.p.handle(&get("/cihazlar", &[], &tok)).body, "yavaslatilmis");
        assert!(b.contains("<div class=\"mesaj hata\">Yavaşlatma kayıtları okunamadı (dosya bozuk). Hizmet sağlayıcınıza başvurun.</div>"), "{b}");
    }

    #[test]
    fn hiz_hatasi_panelde_gorunur() {
        let mut e = env();
        e.p.clock = Box::new(|| SAAT);
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        assert!(!bolum_of(&s, "yavaslatilmis").contains("mesaj hata") && !bolum_of(&s, "oturumlar").contains("mesaj hata"));
        let d = crate::trafik::Durum { zaman: SAAT - 2.0, cihazlar: Default::default(), hiz_hata: Some("Yavaşlatma uygulanamadı: <tc> yok".into()) };
        crate::trafik::yaz(&e.p.trafik_path, &d).unwrap();
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        let b = bolum_of(&s, "yavaslatilmis");
        // bölüm başlık çubuğu (2026-10-30 WhatsApp Web benzeri: telefonda geri oku, avatar) ve hemen altında uyarı
        assert!(b.starts_with(" aria-labelledby=\"b-yavaslatilmis\"><h2 class=\"bolum-baslik\" id=\"b-yavaslatilmis\"><a class=\"geri\" href=\"#\""), "{b}");
        assert!(b.contains("Yavaşlatılmış cihazlar</h2><div class=\"bolum-govde\"><div class=\"mesaj hata\">Yavaşlatma uygulanamadı: &lt;tc&gt; yok</div>"), "{b}");
        // Bağlı kullanıcılar tablosunun üstünde de (ölçüm tazeyken)
        let b = bolum_of(&s, "oturumlar");
        let uyari = "<div class=\"mesaj hata\">Yavaşlatma uygulanamadı: &lt;tc&gt; yok</div>";
        assert!(b.contains(uyari) && b.find(uyari) < b.find("Bağlı kullanıcı yok"), "{b}");
        let d = crate::trafik::Durum { zaman: SAAT - 100.0, ..d };
        crate::trafik::yaz(&e.p.trafik_path, &d).unwrap();
        let s = e.p.handle(&get("/cihazlar", &[], &tok)).body;
        assert!(!bolum_of(&s, "oturumlar").contains("mesaj hata")); // eski ölçüm: tablo da ölçüm göstermez
    }

    // --- misafir kodları

    const TC1: &str = "10000000146";
    const TC2: &str = "11111111110";
    const MISAFIR_MAC: &str = "aa:bb:cc:dd:ee:07";

    /// İşletme türü ve misafir kodu ayarı (doğrudan ayar dosyasına)
    fn misafir_ac(e: &Env, tur: &str, acik: bool) {
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.isletme_turu = Some(tur.into());
        c.giris.misafir_kodu = acik;
        c.save(&e.p.cfg_path).unwrap();
    }

    fn kodlar(e: &Env) -> Vec<crate::misafir::Kod> {
        crate::misafir::oku(&Config::load(&e.p.cfg_path).unwrap().main.state_root)
    }

    fn uret_tc(e: &Env, tok: &str, csrf: &str, tc: &str, oda: &str, sure: &str) -> Resp {
        e.p.handle(&req("POST", "/misafir-kodlari/uret",
            &[("csrf", csrf), ("ad", "Ali"), ("soyad", "Veli"), ("tur", "tc"), ("tc", tc), ("oda", oda), ("sure", sure)], Some(tok)))
    }

    /// Kişi anahtarıyla açık misafir oturumu (portalın açacağı gibi)
    fn misafir_oturumu(e: &Env, mac: &str, anahtar: &str, ip: &str, bitis: f64) {
        let state = Config::load(&e.p.cfg_path).unwrap().main.state_root;
        let mut ses = ortak::load_sessions(&state);
        ses.insert(mac.into(), ortak::Session {
            phone: anahtar.into(), ad: "Ali".into(), soyad: "Veli".into(), ip: ip.into(), session_id: format!("s-{mac}"),
            start: "2026-09-29T21:00:00+03:00".into(), start_epoch: 1_790_705_000.0, expires_epoch: bitis,
        });
        ortak::save_sessions(&state, &ses).unwrap();
    }

    fn oturumlar_of(e: &Env) -> ortak::Sessions {
        ortak::load_sessions(&Config::load(&e.p.cfg_path).unwrap().main.state_root)
    }

    #[test]
    fn misafir_kodu_kapaliyken_404() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let yollar = |e: &Env| {
            [
                e.p.handle(&get("/misafir-kodlari", &[], &tok)).status,
                uret_tc(e, &tok, &csrf, TC1, "101", "24").status,
                e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", "x")], Some(&tok))).status,
                e.p.handle(&req("POST", "/misafir-kodlari/uzat", &[("csrf", &csrf), ("id", "x"), ("sure", "24")], Some(&tok))).status,
                e.p.handle(&get("/misafir-kodlari/fis", &[("id", "x")], &tok)).status,
            ]
        };
        for (tur, acik) in [("otel", false), ("kafe", true), ("diger", true)] {
            misafir_ac(&e, tur, acik);
            assert_eq!(yollar(&e), [404; 5], "{tur} {acik}");
            assert!(!e.p.handle(&get("/", &[], &tok)).body.contains("href=\"/misafir-kodlari\""));
        }
        assert!(kodlar(&e).is_empty());
        misafir_ac(&e, "yurt", true);
        let ozet = e.p.handle(&get("/", &[], &tok)).body;
        assert_eq!(menu_linkleri(&ozet), ["/", "/cihazlar", "/misafir-kodlari", "/kayitlar", "/ayarlar"]);
        let s = e.p.handle(&get("/misafir-kodlari", &[], &tok));
        assert_eq!(s.status, 200);
        assert!(s.body.contains("href=\"/misafir-kodlari\" class=\"aktif\"") && s.body.contains("Kod üret") && s.body.contains("Aktif kod yok"));
        assert_eq!(e.p.handle(&get("/misafir-kodlari/yok", &[], &tok)).status, 404);
    }

    #[test]
    fn kod_uret_tc_ve_pasaport() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        // geçersiz TC: 400, form geri dolar, hiçbir şey yazılmaz
        let r = uret_tc(&e, &tok, &csrf, "10000000145", "101", "24");
        assert_eq!(r.status, 400);
        assert!(r.body.contains("T.C. kimlik numarası geçersiz.") && r.body.contains("value=\"Ali\""));
        assert!(kodlar(&e).is_empty());
        for (alan, deger, hata) in [("oda", "1 01", "Oda numarası 1-10"), ("oda", "", "Oda numarası 1-10"), ("sure", "25", "Geçerlilik süresini seçin"), ("ad", "A", "Ad 2-40")] {
            let mut f = vec![("csrf", csrf.as_str()), ("ad", "Ali"), ("soyad", "Veli"), ("tur", "tc"), ("tc", TC1), ("oda", "101"), ("sure", "24")];
            f.retain(|(k, _)| *k != alan);
            f.push((alan, deger));
            let r = e.p.handle(&req("POST", "/misafir-kodlari/uret", &f, Some(&tok)));
            assert!(r.status == 400 && r.body.contains(hata), "{alan}={deger}");
        }
        assert!(kodlar(&e).is_empty());
        // geçerli TC (boşluklu yazım da)
        let r = uret_tc(&e, &tok, &csrf, " 1000000014 6", "101", "48");
        assert_eq!(r.status, 303);
        let k = kodlar(&e);
        assert_eq!(k.len(), 1);
        assert_eq!((k[0].kimlik.as_str(), k[0].oda.as_str(), k[0].ad.as_str(), k[0].ureten.as_str()), ("TC:10000000146", "101", "Ali", MUSTERI));
        assert_eq!(k[0].bitis, 1_790_705_134.0 + 48.0 * 3600.0);
        assert!(loc(&r).starts_with(&format!("/misafir-kodlari/fis?id={}&m=", k[0].kod_id)), "{}", loc(&r));
        // pasaport: geçersiz uyruk ve numara reddedilir; geçerlisi büyük harfe çevrilir
        let pas = |no: &str, uyruk: &str| {
            e.p.handle(&req("POST", "/misafir-kodlari/uret",
                &[("csrf", &csrf), ("ad", "John"), ("soyad", "Smith"), ("tur", "pasaport"), ("pasaport", no), ("uyruk", uyruk), ("oda", "A-12"), ("sure", "168")], Some(&tok)))
        };
        let r = pas("x1234567", "ZZ");
        assert!(r.status == 400 && r.body.contains("Uyruğu seçin."));
        let r = pas("x12", "DE");
        assert!(r.status == 400 && r.body.contains("Pasaport numarası"));
        assert_eq!(kodlar(&e).len(), 1);
        assert_eq!(pas("x1234567", "DE").status, 303);
        let k = kodlar(&e);
        assert_eq!((k[1].kimlik.as_str(), k[1].oda.as_str()), ("PASAPORT:DE:X1234567", "A-12"));
        // tür seçilmemiş
        let r = e.p.handle(&req("POST", "/misafir-kodlari/uret", &[("csrf", &csrf), ("ad", "Ali"), ("soyad", "Veli"), ("oda", "1"), ("sure", "24")], Some(&tok)));
        assert!(r.status == 400 && r.body.contains("Kimlik türünü seçin"));
        // CSRF'siz üretilmez
        let r = e.p.handle(&req("POST", "/misafir-kodlari/uret", &[("ad", "Ali"), ("soyad", "Veli"), ("tur", "tc"), ("tc", TC2), ("oda", "1"), ("sure", "24")], Some(&tok)));
        assert_eq!(r.status, 403);
        assert_eq!(kodlar(&e).len(), 2);
    }

    #[test]
    fn liste_maskeli_ve_url_de_kimlik_yok() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        let r1 = uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        let r2 = e.p.handle(&req("POST", "/misafir-kodlari/uret",
            &[("csrf", &csrf), ("ad", "John"), ("soyad", "Smith"), ("tur", "pasaport"), ("pasaport", "X1234567"), ("uyruk", "DE"), ("oda", "202"), ("sure", "72")], Some(&tok)));
        for r in [&r1, &r2] {
            assert!(!loc(r).contains(TC1) && !loc(r).contains("X1234567"), "{}", loc(r));
        }
        let k = kodlar(&e);
        let s = e.p.handle(&get("/misafir-kodlari", &[], &tok)).body;
        assert!(s.contains("<code>100******46</code>") && s.contains("<code>DE X12***67</code>"), "maskeli");
        assert!(s.contains("Ali Veli") && s.contains("John Smith") && s.contains("T.C.") && s.contains("Yabancı"));
        assert!(!s.contains(TC1) && !s.contains("X1234567"), "tam kimlik sayfada yok");
        for c in &k {
            assert!(!s.contains(&format!(">{}<", c.kod)), "kod listede görünmez");
            assert!(s.contains(&format!("href=\"/misafir-kodlari/fis?id={}\"", c.kod_id)));
        }
        // bütün adresler (bağlantı ve form hedefi) yalnızca kod_id taşır
        for parca in s.split("href=\"").skip(1).chain(s.split("action=\"").skip(1)) {
            let adres = parca.split('"').next().unwrap();
            assert!(!adres.contains(TC1) && !adres.contains("X1234567") && k.iter().all(|c| !adres.contains(&c.kod)), "{adres}");
        }
        // bakış kişisel veri olarak denetimde; Panel hareketleri etiketi
        assert!(!denetim_satirlari(&e, "PANEL_MISAFIR_KODLARI").is_empty());
        assert!(hareketler::olay_adi("PANEL_MISAFIR_KODLARI").contains("Misafir kodlarına baktı"));
        assert!(hareketler::olay_adi("PANEL_MISAFIR_KODU").contains("üret/iptal/uzat"));
    }

    #[test]
    fn iptal_oturumu_kapatir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        let k = kodlar(&e)[0].clone();
        misafir_oturumu(&e, MISAFIR_MAC, "TC:10000000146", "10.50.0.31", k.bitis);
        misafir_oturumu(&e, AYSE, "905334553132", "10.50.0.23", 9e9); // başkası: dokunulmaz
        let r = e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", &k.kod_id)], Some(&tok)));
        assert!(loc(&r).starts_with("/misafir-kodlari?m=") && !loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(kodlar(&e)[0].iptal);
        let ses = oturumlar_of(&e);
        assert!(!ses.contains_key(MISAFIR_MAC) && ses.contains_key(AYSE));
        let calls: Vec<String> = e.calls.lock().unwrap().iter().map(|c| c.join(" ")).collect();
        assert!(calls.contains(&format!("nft delete element inet hotspot auth {{ {MISAFIR_MAC} . 10.50.0.31 }}")), "{calls:?}");
        assert!(calls.contains(&"conntrack -D -s 10.50.0.31".to_string()));
        let oturum = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/oturum.csv")).unwrap();
        assert!(oturum.lines().any(|l| l.contains("OTURUM_BITIS") && l.contains("TC:10000000146") && l.contains("neden=kod_iptal")), "{oturum}");
        // listeden düştü; ikinci iptal hata
        assert!(e.p.handle(&get("/misafir-kodlari", &[], &tok)).body.contains("Aktif kod yok"));
        let r = e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", &k.kod_id)], Some(&tok)));
        assert!(loc(&r).contains("e=1"));
        assert_eq!(denetim_satirlari(&e, "PANEL_MISAFIR_KODU").iter().filter(|l| l.contains("islem=iptal")).count(), 1);
    }

    #[test]
    fn uzatma_oturumu_uzatir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        let k = kodlar(&e)[0].clone();
        misafir_oturumu(&e, MISAFIR_MAC, "TC:10000000146", "10.50.0.31", k.bitis);
        misafir_oturumu(&e, "aa:bb:cc:dd:ee:08", "TC:10000000146", "", k.bitis); // IP'siz bekleyen
        e.calls.lock().unwrap().clear();
        let r = e.p.handle(&req("POST", "/misafir-kodlari/uzat", &[("csrf", &csrf), ("id", &k.kod_id), ("sure", "48")], Some(&tok)));
        assert!(loc(&r).starts_with("/misafir-kodlari?m=") && !loc(&r).contains("e=1"), "{}", loc(&r));
        let yeni = 1_790_705_134.0 + 72.0 * 3600.0;
        assert_eq!(kodlar(&e)[0].bitis, yeni);
        let ses = oturumlar_of(&e);
        assert_eq!(ses[MISAFIR_MAC].expires_epoch, yeni);
        assert_eq!(ses["aa:bb:cc:dd:ee:08"].expires_epoch, yeni);
        let calls: Vec<String> = e.calls.lock().unwrap().iter().map(|c| c.join(" ")).collect();
        // sil + ekle tek nft çağrısında (tek işlem): arada öğesiz an yok
        assert_eq!(calls, [format!(
            "nft delete element inet hotspot auth {{ {MISAFIR_MAC} . 10.50.0.31 }} ; add element inet hotspot auth {{ {MISAFIR_MAC} . 10.50.0.31 timeout 259200s }}"
        )]);
        // geçersiz süre
        let r = e.p.handle(&req("POST", "/misafir-kodlari/uzat", &[("csrf", &csrf), ("id", &k.kod_id), ("sure", "5")], Some(&tok)));
        assert!(loc(&r).contains("e=1"));
        assert_eq!(kodlar(&e)[0].bitis, yeni);
        let d = denetim_satirlari(&e, "PANEL_MISAFIR_KODU");
        assert!(d.iter().any(|l| l.contains("islem=uzat") && l.contains("sure=48") && l.contains("uzayan=2")), "{d:?}");
        // her uzayan oturum için 5651 kaydı (resmi talep IP araması oturumu buradan da bulur)
        let oturum = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/oturum.csv")).unwrap();
        let uz: Vec<&str> = oturum.lines().filter(|l| l.contains(";OTURUM_UZATILDI;")).collect();
        assert_eq!(uz.len(), 2, "{oturum}");
        let beklenen = format!("yeni_bitis={} ureten={}", ortak::now_iso(yeni), kodlar(&e)[0].ureten); // kodu üreten hesapla aynı
        assert!(uz.iter().all(|l| l.contains("TC:10000000146") && l.ends_with(&beklenen)), "{uz:?}");
        assert!(uz.iter().any(|l| l.contains(MISAFIR_MAC) && l.contains("10.50.0.31")));
    }

    #[test]
    fn uzatma_nft_hatasinda_oturum_degismez() {
        let mut ses = ortak::Sessions::new();
        ses.insert(MISAFIR_MAC.into(), ortak::Session {
            phone: "TC:10000000146".into(), ad: "A".into(), soyad: "B".into(), ip: "10.50.0.31".into(), session_id: "s".into(),
            start: String::new(), start_epoch: 0.0, expires_epoch: 2000.0,
        });
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c2 = calls.clone();
        let runner = move |c: &[String]| {
            c2.lock().unwrap().push(c.to_vec());
            false // işlem reddedildi: nft hiçbirini uygulamaz, eski öğe yerinde
        };
        assert!(!ortak::oturum_uzat(&mut ses, MISAFIR_MAC, 10_000.0, 1000.0, &runner));
        assert_eq!(ses[MISAFIR_MAC].expires_epoch, 2000.0);
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "tek çağrı, geri koyma yok: {calls:?}");
        let c = &calls[0];
        assert_eq!(c[0], "nft");
        assert_eq!(c.iter().filter(|x| x.as_str() == "nft").count(), 1);
        assert!(c.join(" ").ends_with(&format!("; add element inet hotspot auth {{ {MISAFIR_MAC} . 10.50.0.31 timeout 9000s }}")));
        // geçersiz MAC/IP: hiçbir komut yok
        assert!(ortak::nft_uzat_cmd("bozuk", "10.50.0.31", 10.0).is_none() && ortak::nft_uzat_cmd(MISAFIR_MAC, "1.2.3", 10.0).is_none());
        assert!(ortak::oturum_uzat(&mut ses, MISAFIR_MAC, 5000.0, 1000.0, &|_: &[String]| true));
        assert_eq!(ses[MISAFIR_MAC].expires_epoch, 5000.0);
    }

    /// sessions.json yazılamasın: geçici dosya yolu klasör (root için de açılamaz)
    fn oturum_kaydini_boz(e: &Env) {
        let state = Config::load(&e.p.cfg_path).unwrap().main.state_root;
        std::fs::create_dir_all(std::path::Path::new(&state).join("sessions.tmp")).unwrap();
    }

    #[test]
    fn oturum_kaydi_yazilamazsa_basari_denmez() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        uret_tc(&e, &tok, &csrf, TC2, "102", "24");
        let k = kodlar(&e);
        misafir_oturumu(&e, MISAFIR_MAC, "TC:10000000146", "10.50.0.31", k[0].bitis);
        misafir_oturumu(&e, AYSE, "TC:11111111110", "10.50.0.23", k[1].bitis);
        oturum_kaydini_boz(&e);
        // uzatma: hata mesajı, denetimde hatali
        let r = e.p.handle(&req("POST", "/misafir-kodlari/uzat", &[("csrf", &csrf), ("id", &k[1].kod_id), ("sure", "24")], Some(&tok)));
        assert!(loc(&r).contains("e=1") && loc(&r).contains(&pct("Oturum kaydı yazılamadı")), "{}", loc(&r));
        assert!(denetim_satirlari(&e, "PANEL_MISAFIR_KODU").last().unwrap().contains("uzayan=0 hatali=1"));
        // iptal
        let r = e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", &k[0].kod_id)], Some(&tok)));
        assert!(loc(&r).contains("e=1") && loc(&r).contains(&pct("Oturum kaydı yazılamadı; servisleri yeniden başlatın.")), "{}", loc(&r));
        assert!(denetim_satirlari(&e, "PANEL_MISAFIR_KODU").last().unwrap().contains("kapanan=1 hatali=1"));
        // aynı kimliğe yeni kod (eskisi iptal, oturum kapanır): yine hata
        let r = uret_tc(&e, &tok, &csrf, TC2, "103", "24");
        assert!(loc(&r).starts_with("/misafir-kodlari/fis?id=") && loc(&r).ends_with("&e=1"), "{}", loc(&r));
        assert!(denetim_satirlari(&e, "PANEL_MISAFIR_KODU").last().unwrap().ends_with("hatali=1"));
    }

    #[test]
    fn ayni_kimlik_yeni_kod_eskiyi_iptal_eder() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        uret_tc(&e, &tok, &csrf, TC2, "102", "24");
        let ilk = kodlar(&e)[0].clone();
        misafir_oturumu(&e, MISAFIR_MAC, "TC:10000000146", "10.50.0.31", ilk.bitis);
        misafir_oturumu(&e, AYSE, "TC:11111111110", "10.50.0.23", ilk.bitis);
        let r = uret_tc(&e, &tok, &csrf, TC1, "105", "48");
        assert!(loc(&r).contains("&m=") && !loc(&r).contains("e=1"));
        let k = kodlar(&e);
        assert_eq!(k.len(), 3);
        assert!(k[0].iptal && !k[1].iptal && !k[2].iptal && k[2].oda == "105");
        let ses = oturumlar_of(&e);
        assert!(!ses.contains_key(MISAFIR_MAC) && ses.contains_key(AYSE), "yalnızca aynı kimliğin oturumu kapanır");
        let oturum = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/oturum.csv")).unwrap();
        assert!(oturum.lines().any(|l| l.contains("TC:10000000146") && l.contains("neden=kod_iptal")));
        let s = e.p.handle(&get("/misafir-kodlari", &[], &tok)).body;
        assert_eq!(s.matches("<code>100******46</code>").count(), 1, "aynı kişinin tek aktif kodu");
        let d = denetim_satirlari(&e, "PANEL_MISAFIR_KODU");
        assert!(d.last().unwrap().contains(&format!("onceki_iptal={} kapanan=1", ilk.kod_id)), "{d:?}");
    }

    #[test]
    fn denetimde_kod_ve_kimlik_yok() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        uret_tc(&e, &tok, &csrf, TC1, "101", "24");
        e.p.handle(&req("POST", "/misafir-kodlari/uret",
            &[("csrf", &csrf), ("ad", "John"), ("soyad", "Smith"), ("tur", "pasaport"), ("pasaport", "X1234567"), ("uyruk", "DE"), ("oda", "202"), ("sure", "72")], Some(&tok)));
        let k = kodlar(&e);
        e.p.handle(&get("/misafir-kodlari/fis", &[("id", &k[0].kod_id)], &tok));
        e.p.handle(&get("/misafir-kodlari", &[], &tok));
        e.p.handle(&req("POST", "/misafir-kodlari/uzat", &[("csrf", &csrf), ("id", &k[0].kod_id), ("sure", "24")], Some(&tok)));
        e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", &k[1].kod_id)], Some(&tok)));
        uret_tc(&e, &tok, &csrf, "10000000145", "101", "24"); // hatalı form da yazmaz
        let denetim = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        let d = denetim_satirlari(&e, "PANEL_MISAFIR_KODU");
        for islem in ["uret", "uzat", "iptal"] {
            assert!(d.iter().any(|l| l.contains(&format!("islem={islem}"))), "{islem}");
        }
        assert!(d.iter().any(|l| l.contains("oda=101") && l.contains("tur=tc") && l.contains("bitis=2026-09-30T21:05:34+03:00")), "{d:?}");
        assert!(d.iter().any(|l| l.contains("oda=202") && l.contains("tur=pasaport")));
        assert!(!denetim.contains(TC1) && !denetim.contains("10000000145") && !denetim.contains("X1234567"), "{denetim}");
        for c in &k {
            // kod 6 hane: başka bir alanın parçası olabilir, ayrı bir değer olarak geçmemeli
            assert!(!denetim.split([';', ' ', '=']).any(|t| t == c.kod), "kod denetimde: {denetim}");
        }
    }

    #[test]
    fn fis_bitis_ve_iki_dil() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        misafir_ac(&e, "otel", true);
        let r = uret_tc(&e, &tok, &csrf, TC1, "101", "168");
        assert!(loc(&r).starts_with("/misafir-kodlari/fis?id="));
        let k = kodlar(&e)[0].clone();
        let mut q = get("/misafir-kodlari/fis", &[("id", &k.kod_id)], &tok);
        q.query.insert("m".into(), "Kod üretildi.".into());
        let s = e.p.handle(&q);
        assert_eq!(s.status, 200);
        let b = &s.body;
        assert!(b.contains(&format!("<div class=\"kod\">{}</div>", k.kod)), "kod büyük");
        assert!(b.contains("<b>06.10.2026 21:05</b>"), "1 hafta sonrası, TR saati");
        assert!(b.contains("Ali Veli") && b.contains("<b>101</b>") && b.contains("Bocafe Göztepe"));
        assert!(b.contains("'Otel misafiriyim'e dokunun") && b.contains("tap 'Hotel guest'"));
        assert!(b.contains("@media print") && b.contains("Yazdırmak için Ctrl+P") && b.contains("Kod üretildi."));
        assert!(!b.contains(TC1) && !b.to_lowercase().contains("kupon"));
        assert_eq!(misafir_sayfasi::tarih_saat(1_790_705_134.0), "29.09.2026 21:05");
        // süresi dolmuş (iptal edilmemiş) kodun fişi açılır ama kod gösterilmez
        let state = Config::load(&e.p.cfg_path).unwrap().main.state_root;
        let mut dolmus = kodlar(&e);
        dolmus[0].bitis = 1_790_705_000.0;
        crate::misafir::kaydet(&state, &dolmus).unwrap();
        let b = e.p.handle(&q).body;
        assert!(!b.contains(&format!(">{}<", k.kod)) && b.contains("Süresi doldu / Expired") && b.contains("Bu kodun süresi doldu."), "{b}");
        crate::misafir::kaydet(&state, &[k.clone()]).unwrap();
        misafir_ac(&e, "yurt", true);
        let b = e.p.handle(&q).body;
        assert!(b.contains("'Yurt sakiniyim'e dokunun") && b.contains("tap 'Dormitory resident'"));
        // bilinmeyen ya da iptal edilmiş kodun fişi yok
        assert!(loc(&e.p.handle(&get("/misafir-kodlari/fis", &[("id", "yok")], &tok))).contains("e=1"));
        e.p.handle(&req("POST", "/misafir-kodlari/iptal", &[("csrf", &csrf), ("id", &k.kod_id)], Some(&tok)));
        assert!(loc(&e.p.handle(&q)).contains("e=1"));
    }

    // --- T7: kimlik maskeleme ve opak kişi kimliği
    const KIMLIK_TC: &str = "TC:10000000146";
    const KIMLIK_PAS: &str = "PASAPORT:DE:X1234567";

    /// TC'li ve pasaportlu misafir: açık oturum + oturum.csv + kişi dosyası + kişi listesi + DNS satırı (bugün)
    fn kimlikli_kayitlar(e: &Env) -> Config {
        let cfg = Config::load(&e.p.cfg_path).unwrap();
        let root = cfg.main.log_root.clone();
        for (i, k) in [KIMLIK_TC, KIMLIK_PAS].into_iter().enumerate() {
            let (mac, ip) = (format!("aa:bb:cc:dd:ee:3{i}"), format!("10.50.0.3{i}"));
            misafir_oturumu(e, &mac, k, &ip, 9e9);
            let row = Row::new("OTURUM_BASLA", "2026-09-29T21:00:00+03:00").set("telefon", k).set("ad", "Ali").set("soyad", "Veli")
                .set("mac", &mac).set("ic_ip", &ip).set("oturum_id", &format!("s-{mac}")).set("ek", "giris=misafir_kodu oda=101");
            ortak::append_rows(&ortak::day_file(&root, "2026-09-29", "oturum.csv"), &[row.clone()]).unwrap();
            ortak::append_rows(&ortak::user_file(&root, "2026-09-29", k), &[row]).unwrap();
            ortak::upsert_index(&root, k, "Ali", "Veli", "2026-09-29T21:00:00+03:00").unwrap();
            ortak::append_rows(&ortak::day_file(&root, "2026-09-29", "dns.csv"), &[Row::new("DNS", "2026-09-29T21:01:00+03:00")
                .set("telefon", k).set("mac", &mac).set("ic_ip", &ip).set("alan_adi", "www.ornek.com")]).unwrap();
        }
        cfg
    }

    /// Sayfadaki bütün adresler: href, action ve Location
    fn adresler(r: &Resp) -> Vec<String> {
        let mut v: Vec<String> = ["href=\"", "action=\""]
            .iter()
            .flat_map(|a| r.body.split(a).skip(1).map(|x| x.split('"').next().unwrap_or("").to_string()))
            .collect();
        v.push(loc(r).to_string());
        v
    }

    fn kimlik_yok(ad: &str, metin: &str) {
        for t in ["10000000146", "X1234567", "TC_", "TC:", "TC%3A", "PASAPORT"] {
            assert!(!metin.contains(t), "{ad}: {t} sızdı: {metin}");
        }
    }

    #[test]
    fn tam_kimlik_sizmaz() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        let cfg = kimlikli_kayitlar(&e);
        let sir = crate::kimlik::sir_oku_ya_uret(&cfg.main.state_root);
        let mut sayfalar = vec![
            ("Özet", e.p.handle(&get("/", &[], &tok))),
            ("Bağlı kullanıcılar", e.p.handle(&get("/oturumlar", &[], &tok))),
            ("Kullanıcılar grubu", e.p.handle(&get("/cihazlar", &[], &tok))),
            ("Kayıtlar grubu", e.p.handle(&get("/kayitlar", &[], &tok))),
            ("Kullanıcılar", e.p.handle(&get("/kullanicilar", &[], &tok))),
            ("Kayıt günü", e.p.handle(&get("/kayitlar/gun", &[("gun", "2026-09-29")], &tok))),
            ("Site arama", e.p.handle(&get("/ara", &[("q", "ornek"), ("bas", "2026-09-29"), ("bit", "2026-09-29")], &tok))),
        ];
        for k in [KIMLIK_TC, KIMLIK_PAS] {
            sayfalar.push(("Kişi sayfası", e.p.handle(&get("/kullanici", &[("k", &crate::kimlik::opak(&sir, k))], &tok))));
        }
        for (ad, r) in &sayfalar {
            assert_eq!(r.status, 200, "{ad}");
            kimlik_yok(ad, &r.body);
            for a in adresler(r) {
                kimlik_yok(ad, &a);
            }
        }
        // maskeli hâlleri görünür
        let liste = &sayfalar[4].1.body;
        assert!(liste.contains("100******46") && liste.contains("DE X12***67") && liste.contains("Ali Veli"), "{liste}");
        assert!(sayfalar[1].1.body.contains("100******46") && sayfalar[1].1.body.contains("DE X12***67"));
        assert!(sayfalar[5].1.body.contains("kullanicilar/100******46.csv"));
        assert!(sayfalar[6].1.body.contains("100******46") && sayfalar[7].1.body.contains("100******46"));
        // kimlikle arama POST: sonuç maskeli; aranan değer yalnızca arama kutusunda (kullanıcının yazdığı) durur
        let r = e.p.handle(&req("POST", "/kullanicilar", &[("csrf", &csrf), ("q", "10000000146")], Some(&tok)));
        assert!(r.body.contains("1 kişi") && r.body.contains("100******46"));
        kimlik_yok("Kullanıcı araması", &r.body.replace("value=\"10000000146\"", ""));
        for a in adresler(&r) {
            kimlik_yok("Kullanıcı araması adresi", &a);
        }
        // resmi talep: sonuç 5651 kaydıdır, tam; form POST
        let r = e.p.handle(&req("POST", "/talep", &[("csrf", &csrf), ("tur", "tc"), ("deger", "10000000146")], Some(&tok)));
        assert!(r.body.contains("TC:10000000146 oturumları") && r.body.contains("method=\"post\" action=\"/talep\""), "{}", r.body);
        let r = e.p.handle(&req("POST", "/talep", &[("csrf", &csrf), ("tur", "pasaport"), ("deger", "x1234567"), ("uyruk", "de")], Some(&tok)));
        assert!(r.body.contains("PASAPORT:DE:X1234567 oturumları"));
        let r = e.p.handle(&req("POST", "/talep", &[("csrf", &csrf), ("tur", "oda"), ("deger", "101")], Some(&tok)));
        assert!(r.body.contains("Oda 101: misafir oturumları") && r.body.contains("TC:10000000146"));
        // yalnız rakamlı pasaport ve boşluklu TC de denetime maskeli girer (talep, kullanıcı ve satır araması)
        let post = |yol: &str, f: &[(&str, &str)]| {
            let mut v = vec![("csrf", csrf.as_str())];
            v.extend_from_slice(f);
            e.p.handle(&req("POST", yol, &v, Some(&tok)))
        };
        post("/talep", &[("tur", "pasaport"), ("deger", "123456789")]);
        post("/talep", &[("tur", "tc"), ("deger", "100 000 001 46")]);
        post("/kullanicilar", &[("q", "100 000 001 46")]);
        post("/kullanicilar", &[("q", "123456789")]);
        post("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv"), ("q", "123456789")]);
        // önekli / küçük harfli / uyruk:no / başka ayraçlı yazımlar da (düzeltme turu 2)
        post("/kullanicilar", &[("q", "TC:10000000146")]);
        post("/kullanicilar", &[("q", "pasaport:de:x1234567")]);
        post("/kullanicilar", &[("q", "100.000.001.46")]);
        post("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv"), ("q", "10000000146,")]);
        post("/talep", &[("tur", "pasaport"), ("deger", "de:x1234567")]);
        // düzeltme turu 3: serbest metinde 5+ rakam varsa her rakam yıldız (hangi ayraçla yazılırsa yazılsın)
        let saldiri = [
            "ayşe 100 000 001 46", "(10000000146)", "\"10000000146\"", "TC=10000000146", "TC_10000000146", "100_000_001_46",
            "１００００００００１４６", "TC：10000000146", "100–000–001–46", "100·000·001·46", "10000000146\u{200B}",
        ];
        for q in saldiri {
            post("/kullanicilar", &[("q", q)]);
            post("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv"), ("q", q)]);
        }
        post("/kullanicilar", &[("q", "101")]);
        post("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv"), ("q", "bet")]);
        post("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv"), ("q", "10.50.0.23:44321")]);
        // denetim kaydına kimlik tam girmez
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("PANEL_TALEP_ARA") && audit.contains("deger=100******46") && audit.contains("deger=DE X12***67"));
        assert!(audit.contains("PANEL_KULLANICI_ARA") && audit.contains("ara=***********"));
        // her serbest arama değerinde 5 ve daha uzun rakam dizisi yok; kısa arama ve IP'nin noktalaması aynen
        let degerler: Vec<&str> = audit
            .lines()
            .filter_map(|l| l.split("ara=").nth(1))
            .map(|v| v.split(" gerekce=").next().unwrap_or(v))
            .collect();
        assert!(degerler.len() >= 2 * 11 + 3, "{degerler:?}");
        for v in &degerler {
            let mut dizi = 0;
            for c in v.chars() {
                dizi = if c.is_numeric() { dizi + 1 } else { 0 };
                assert!(dizi < 5, "denetimde rakam dizisi: {v}");
            }
        }
        assert!(degerler.contains(&"101") && degerler.contains(&"bet") && degerler.contains(&"**.**.*.**:*****"), "{degerler:?}");
        assert!(audit.contains("kisi=100******46")); // kişi sayfası bakışı
        assert!(!audit.contains("10000000146") && !audit.to_uppercase().contains("X1234567") && !audit.contains("100.000.001.46"), "{audit}");
        assert!(!audit.contains("123456789") && !audit.contains("000 001") && audit.contains("deger=?? 123****89"), "{audit}");
    }

    #[test]
    fn site_aramasi_denetimde_yalniz_site_ve_ip_aynen() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        kimlikli_kayitlar(&e);
        let aynen = ["https://bet365.com/spor", "10.50.0.23", "bet365.com"];
        let yildizli = [
            ("10000000146", "***********"),
            ("ayşe 100 000 001 46", "ayşe *** *** *** **"),
            ("(10000000146)", "(***********)"),
            ("100.000.001.46", "***.***.***.**"),
        ];
        // "TC:10000000146": site terimi "tc" (3 harften kısa) olduğu için arama reddedilir, denetime hiç yazılmaz
        for q in aynen.iter().copied().chain(yildizli.iter().map(|(q, _)| *q)).chain(["TC:10000000146"]) {
            let r = e.p.handle(&get("/ara", &[("q", q), ("bas", "2026-09-29"), ("bit", "2026-09-29")], &tok));
            assert_eq!(r.status, 200, "{q}");
        }
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        let degerler: Vec<&str> = audit
            .lines()
            .filter(|l| l.contains("PANEL_SITE_ARA"))
            .filter_map(|l| l.split("aranan=").nth(1))
            .map(|v| v.split(" bas=").next().unwrap_or(v))
            .collect();
        assert_eq!(degerler.len(), aynen.len() + yildizli.len(), "{degerler:?}");
        for q in aynen {
            assert!(degerler.contains(&q), "{q} aynen yazılmalı: {degerler:?}");
        }
        for (_, m) in yildizli {
            assert!(degerler.contains(&m), "{m} bekleniyordu: {degerler:?}");
        }
        assert!(!audit.contains("10000000146") && !audit.contains("100.000.001.46") && !audit.contains("000 001"), "{audit}");
    }

    #[test]
    fn kisi_sayfasi_opak_kimlikle() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        let cfg = kimlikli_kayitlar(&e);
        let sir = crate::kimlik::sir_oku_ya_uret(&cfg.main.state_root);
        let k = crate::kimlik::opak(&sir, KIMLIK_TC);
        assert_eq!(k.len(), 16);
        // listedeki bağlantı opak kimlikle
        let liste = e.p.handle(&get("/kullanicilar", &[], &tok)).body;
        assert!(liste.contains(&format!("href=\"/kullanici?k={k}\"")));
        let r = e.p.handle(&get("/kullanici", &[("k", &k)], &tok));
        assert_eq!(r.status, 200);
        assert!(r.body.contains("100******46") && r.body.contains("OTURUM_BASLA") && r.body.contains("Ali Veli") && r.body.contains("2026-09-29"));
        // kişi dosyası bağlantısı da opak
        assert!(r.body.contains(&format!("dosya=kullanicilar%2F{k}.csv")), "{}", r.body);
        // ham anahtar, başka sırla üretilmiş kimlik ya da eski ?tel= adresi açılmaz
        let baska = crate::kimlik::opak(b"baska-sir", KIMLIK_TC);
        for bad in [KIMLIK_TC, "10000000146", baska.as_str(), ""] {
            assert_eq!(e.p.handle(&get("/kullanici", &[("k", bad)], &tok)).status, 404, "{bad}");
        }
        assert_eq!(e.p.handle(&get("/kullanici", &[("tel", "10000000146")], &tok)).status, 404);
        // kişi listesinden çıkmış kişi de günlerin dosyalarından bulunur
        std::fs::remove_file(std::path::Path::new(&cfg.main.log_root).join("kullanicilar/index.csv")).unwrap();
        assert_eq!(e.p.handle(&get("/kullanici", &[("k", &k)], &tok)).status, 200);
    }

    #[test]
    fn kayit_dosyasinda_tam_deger() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        give_reason(&e, &tok, &csrf);
        let cfg = kimlikli_kayitlar(&e);
        let sir = crate::kimlik::sir_oku_ya_uret(&cfg.main.state_root);
        let opak = crate::kimlik::opak(&sir, KIMLIK_TC);
        let u = format!("kullanicilar/{opak}.csv");
        // görüntüleyici: 5651 kaydının kendisi, tam değer; başlıkta maskeli
        let r = e.p.handle(&get("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", &u)], &tok));
        assert_eq!(r.status, 200);
        assert!(r.body.contains("TC:10000000146") && r.body.contains("100******46"), "{}", r.body);
        // satır araması POST; sonuç tam
        let r = e.p.handle(&req("POST", "/kayitlar/dosya", &[("csrf", &csrf), ("gun", "2026-09-29"), ("dosya", &u), ("q", "10000000146")], Some(&tok)));
        assert!(r.body.contains("TC:10000000146") && r.body.contains("1 satır"));
        // genel dosyada da tam
        let r = e.p.handle(&get("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "oturum.csv")], &tok));
        assert!(r.body.contains("TC:10000000146") && r.body.contains("PASAPORT:DE:X1234567"));
        // indirme: içerik tam, dosya adı (indirme geçmişi) opak
        let r = e.p.handle(&get("/kayitlar/indir", &[("gun", "2026-09-29"), ("dosya", &u)], &tok));
        assert_eq!(r.status, 200);
        let mut icerik = String::new();
        r.file.as_ref().unwrap().try_clone().unwrap().read_to_string(&mut icerik).unwrap();
        assert!(icerik.contains("TC:10000000146"));
        let ad = r.headers.iter().find(|(k, _)| k == "Content-Disposition").unwrap().1.clone();
        assert!(!ad.contains("10000000146") && ad.contains(&opak), "{ad}");
        // gerçek dosya adıyla (eski yer imi) gelinirse opak adrese yönlenir; sayfadaki hiçbir adreste gerçek ad yok
        let r = e.p.handle(&get("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "kullanicilar/TC_10000000146.csv"), ("s", "1")], &tok));
        assert_eq!(r.status, 303);
        assert_eq!(loc(&r), format!("/kayitlar/dosya?gun=2026-09-29&dosya=kullanicilar%2F{opak}.csv&s=1"));
        let r = e.p.handle(&get("/kayitlar/indir", &[("gun", "2026-09-29"), ("dosya", "kullanicilar/TC_10000000146.csv")], &tok));
        assert_eq!((r.status, loc(&r).to_string()), (303, format!("/kayitlar/indir?gun=2026-09-29&dosya=kullanicilar%2F{opak}.csv")));
        // POST (satır araması) gerçek adla gelse de sayfadaki bağlantılar, gizli alan ve form opak
        let r = e.p.handle(&req("POST", "/kayitlar/dosya", &[("csrf", &csrf), ("gun", "2026-09-29"), ("dosya", "kullanicilar/TC_10000000146.csv"), ("q", "oturum")], Some(&tok)));
        assert_eq!(r.status, 200);
        for a in adresler(&r) {
            assert!(!a.contains("TC_") && !a.contains("PASAPORT_") && !a.contains("10000000146"), "{a}");
        }
        assert!(!r.body.contains("value=\"kullanicilar/TC_") && r.body.contains(&format!("value=\"kullanicilar/{opak}.csv\"")));
        assert_eq!(e.p.handle(&get("/kayitlar/dosya", &[("gun", "2026-09-29"), ("dosya", "kullanicilar/0123456789abcdef.csv")], &tok)).status, 404);
        let audit = std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap();
        assert!(audit.contains("dosya=kullanicilar/100******46.csv") && !audit.contains("10000000146"), "{audit}");
    }

    /// Sahte merkez (raporlar): çağrı sayısı ve gövde denetimi; dönem 2026-09 var, başkası 404.
    fn rapor_merkezi(e: &mut Env, durum: u16) -> Arc<AtomicUsize> {
        let n = Arc::new(AtomicUsize::new(0));
        let n2 = n.clone();
        e.p.rapor_http = Box::new(move |yol: &str, govde: &str| {
            n2.fetch_add(1, Ordering::SeqCst);
            let v: serde_json::Value = serde_json::from_str(govde).unwrap();
            assert_eq!(v["cihaz_anahtari"], "k");
            if durum != 200 {
                return Ok((durum, "{\"hata\":\"x\",\"kod\":\"yetki\"}".into()));
            }
            let liste = serde_json::json!({"raporlar": [
                {"tur": "hafta", "donem": "2026-W40", "baslangic": "2026-09-28", "bitis": "2026-10-04", "kesin": false, "farkli": 3, "kayit_gun": 1, "kayit_tam": 1},
                {"tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30", "kesin": false, "farkli": 4, "kayit_gun": 20, "kayit_tam": 16}],
                "bugun": "2026-09-29"});
            let rapor = serde_json::json!({"rapor": {"bicim": 2, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30",
                "uretim": "2026-09-29T12:31:00+03:00", "kesin": false, "misafir": {"farkli": 4}, "saat": [], "hafta_gunu": [],
                "gunler": [{"gun": "2026-09-01", "durum": "tam", "neden": "", "farkli": 2}], "kayit": {"gun": 20, "tam": 16},
                "kullanim": {"gonderilen": 0, "alinan": 0, "sms": 3}, "surum": "0.4.10", "surum_sonra_degisti": false},
                "isletme": {"numara": MUSTERI, "ad": "Bocafe <i>", "unvan": ""},
                "cihaz": {"surum": "0.4.10", "cevrimdisi_sn": 0, "sorunlar": []}, "bugun": "2026-09-29"});
            match (yol, v["donem"].as_str()) {
                ("/api/raporlar", _) => Ok((200, liste.to_string())),
                ("/api/rapor", Some("2026-09")) => Ok((200, rapor.to_string())),
                ("/api/rapor", _) => Ok((404, "{\"hata\":\"yok\",\"kod\":\"yok\"}".into())),
                _ => Err("?".into()),
            }
        });
        n
    }

    #[test]
    fn raporlar_ayarlarda_merkezden_onbellekli_denetimsiz() {
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let n = rapor_merkezi(&mut e, 200);
        let ay = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        let kutular = ay.split("aria-label=\"Bu sayfadaki bölümler\">").nth(1).unwrap().split("</nav>").next().unwrap().to_string();
        assert!(kutular.contains("href=\"/raporlar\"") && kutular.ends_with("<b>Admin</b><small>Hizmet sağlayıcının ayarları</small></span></a>"), "{kutular}");
        let denetim = e.root.join("5651/gunluk/2026-09-29/denetim.csv");
        let once = std::fs::read_to_string(&denetim).unwrap_or_default();
        let l = e.p.handle(&req("GET", "/raporlar", &[], Some(&tok)));
        assert_eq!(l.status, 200);
        assert!(l.body.contains("<a href=\"/rapor/ay/2026-09\">Eylül 2026</a> <b class=\"uyarisi\">sürüyor</b>"), "{}", l.body);
        assert!(l.body.contains("href=\"/ayarlar\" class=\"aktif\""), "menüde Ayarlar seçili");
        e.p.handle(&req("GET", "/raporlar", &[], Some(&tok)));
        assert_eq!(n.load(Ordering::SeqCst), 1, "liste önbellekten");
        let r = e.p.handle(&req("GET", "/rapor/ay/2026-09", &[], Some(&tok)));
        assert_eq!(r.status, 200);
        assert!(r.body.contains("Aylık rapor · Eylül 2026") && r.body.contains("Bocafe &lt;i&gt;") && r.body.contains("20 günün 16'sı tam"));
        assert!(r.body.contains("href=\"/ayarlar\" class=\"aktif\""));
        e.p.handle(&req("GET", "/rapor/ay/2026-09", &[], Some(&tok)));
        assert_eq!(n.load(Ordering::SeqCst), 2, "rapor önbellekten");
        let r = e.p.handle(&req("GET", "/rapor/ay/2026-08", &[], Some(&tok)));
        assert!(r.status == 404 && r.body.contains("Bu dönem için rapor yok."));
        for yol in ["/rapor/ay/..%2Fx", "/rapor/ay/2026-13", "/rapor/hafta/2026-w40", "/rapor/yil/2026", "/rapor/ay/2026-09/x"] {
            let r = e.p.handle(&req("GET", yol, &[], Some(&tok)));
            assert_eq!(r.status, 404, "{yol}");
        }
        assert_eq!(n.load(Ordering::SeqCst), 3, "geçersiz dönem merkeze sorulmaz");
        assert_eq!(std::fs::read_to_string(&denetim).unwrap_or_default(), once, "görüntüleme denetime yazılmaz");
    }

    #[test]
    fn raporlar_alinamiyor_ve_lisans_kapali() {
        // merkeze ulaşılamıyor (varsayılan sahte merkez); bir kez sorulur, kısa süre önbellekte
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let r = e.p.handle(&req("GET", "/raporlar", &[], Some(&tok)));
        assert!(r.status == 200 && r.body.contains("Raporlar şu an alınamıyor"), "{}", r.body);
        assert!(e.p.handle(&req("GET", "/rapor/hafta/2026-W40", &[], Some(&tok))).body.contains("Raporlar şu an alınamıyor"));
        // merkez 403 (üyelik/lisans kapalı); admin kilidi açıkken de aynı sayfa
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "admin", "hizmet-parola-1");
        rapor_merkezi(&mut e, 403);
        assert!(e.p.handle(&req("GET", "/raporlar", &[], Some(&tok))).body.contains("lisans kapalı"));
        // yerelde lisans askıda: merkeze hiç sorulmaz
        let mut e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let n = rapor_merkezi(&mut e, 200);
        let mut m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
        assert!(e.p.handle(&req("GET", "/raporlar", &[], Some(&tok))).body.contains("lisans kapalı"));
        assert!(e.p.handle(&req("GET", "/rapor/ay/2026-09", &[], Some(&tok))).body.contains("lisans kapalı"));
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ap_kurulum_isletme_sahibi_de() {
        // 2026-10-15 kullanıcı kararı: işletme sahibi de AP kurulumunu kullanır (admin kilidi yok); CSRF ve denetim aynen
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let denetim = || std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        // bölüm Ayarlar grup sayfasında (Admin'de değil); sayfalar kilit formu değil
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<section class=\"bolum\" id=\"ap-kurulum\"") && a.contains("href=\"#ap-kurulum\"") && a.contains("action=\"/ap-kurulum/gecici-ekle\""));
        for yol in ["/ap-kurulum", "/ap-kurulum/cihazlar"] {
            let r = e.p.handle(&req("GET", yol, &[], Some(&tok)));
            assert!(r.status == 200 && !r.body.contains("action=\"/admin-kilidi\""), "{yol}");
        }
        assert!(denetim().lines().any(|l| l.contains("PANEL_AP_CIHAZLAR") && l.contains("rol=sahip")), "{}", denetim());
        // CSRF'siz POST: 403, hiçbir şey değişmez
        for yol in ["/ap-kurulum/gecici-ekle", "/ap-kurulum/gecici-kaldir", "/ap-kurulum/adresler", "/ap-kurulum/dhcp-denetle"] {
            let r = e.p.handle(&req("POST", yol, &[("adres", "192.168.0.250/24"), ("adresler", "10.50.0.2")], Some(&tok)));
            assert_eq!(r.status, 403, "{yol}");
        }
        assert!(Config::load(&e.p.cfg_path).unwrap().ap.adresler.is_empty());
        assert!(e.calls.lock().unwrap().iter().all(|c| c[0] != "ip" && c[0] != "systemd-run"));
        // doğrulama sahipte de: misafir ağı reddedilir
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("csrf", &csrf), ("adres", "10.50.0.250/24")], Some(&tok)));
        assert!(loc(&r).starts_with("/ayarlar?m=") && loc(&r).contains("e=1") && loc(&r).ends_with("#ap-kurulum"), "{}", loc(&r));
        assert!(e.calls.lock().unwrap().iter().all(|c| c[0] != "ip" && c[0] != "systemd-run"));
        // geçerli açma (modemin ağıyla aynı alt ağ) ve kayıt: denetimde kim + rol
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("csrf", &csrf), ("adres", "192.168.1.250/24")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert!(denetim().lines().any(|l| l.contains("AP_YAN_ODA") && l.contains("rol=sahip") && l.contains("acildi adres=192.168.1.250/24")));
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.30")], Some(&tok)));
        assert!(loc(&r).contains("e=1"), "havuz reddedilir");
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        assert_eq!(Config::load(&e.p.cfg_path).unwrap().ap.adresler, ["10.50.0.2"]);
        assert!(denetim().lines().any(|l| l.contains("PANEL_AP_ADRES") && l.contains("rol=sahip")));
        // DHCP denetimi sahipte de
        let r = e.p.handle(&req("POST", "/ap-kurulum/dhcp-denetle", &[("csrf", &csrf)], Some(&tok)));
        assert_eq!(r.status, 200);
        assert!(denetim().lines().any(|l| l.contains("AP_DHCP_DENETIM") && l.contains("rol=sahip")));
        // Admin sayfası hâlâ kilitli ve AP bölümünü içermez
        let a = e.p.handle(&req("GET", "/admin", &[], Some(&tok))).body;
        assert!(a.contains("action=\"/admin-kilidi\"") && !a.contains("id=\"ap-kurulum\""));
    }

    #[test]
    fn ap_kurulum_bozuk_durum_dosyasi_kaldirilabilir() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        std::fs::write(&e.p.ap_yollar.gecici, "bozuk").unwrap();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("okunamadı") && a.contains("action=\"/ap-kurulum/gecici-kaldir\"") && !a.contains("action=\"/ap-kurulum/gecici-ekle\""));
        e.p.handle(&req("POST", "/ap-kurulum/gecici-kaldir", &[("csrf", &csrf)], Some(&tok)));
        assert!(!e.p.ap_yollar.gecici.exists());
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("action=\"/ap-kurulum/gecici-ekle\""));
    }

    #[test]
    fn ap_kurulum_yan_oda_ve_cihazlar() {
        let e = env();
        std::fs::create_dir_all(e.p.sys.join("br-hotspot")).unwrap();
        std::fs::write(e.p.sys.join("br-hotspot/address"), "00:0e:c4:ce:a0:9a\n").unwrap();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let denetim = || std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        // Ayarlar grup sayfasında bölüm: misafir portu adresi + köprü MAC'i, form, DHCP denetimi düğmesi
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<section class=\"bolum\" id=\"ap-kurulum\"") && a.contains("href=\"#ap-kurulum\""));
        assert!(a.contains("10.50.0.1") && a.contains("00:0e:c4:ce:a0:9a") && a.contains("action=\"/ap-kurulum/gecici-ekle\""));
        assert!(a.contains("action=\"/ap-kurulum/dhcp-denetle\""));
        assert_eq!(e.p.handle(&req("GET", "/ap-kurulum", &[], Some(&tok))).status, 200);
        // CSRF'siz POST reddedilir
        assert_eq!(e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("adres", "192.168.1.250/24")], Some(&tok))).status, 403);
        // geçersiz adres (genel IP): mesaj, komut yok, denetim yok
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("csrf", &csrf), ("adres", "8.8.8.1/24")], Some(&tok)));
        assert!(loc(&r).starts_with("/ayarlar?m=") && loc(&r).contains("e=1") && loc(&r).ends_with("#ap-kurulum"), "{}", loc(&r));
        assert!(e.calls.lock().unwrap().iter().all(|c| c[0] != "ip" && c[0] != "systemd-run"));
        assert!(!denetim().contains("AP_YAN_ODA"));
        // açma: önce kapatma zamanlayıcısı, sonra aracı; ana ad alanına ip komutu yok; denetim; yönlendirme
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("csrf", &csrf), ("adres", "192.168.1.250/24")], Some(&tok)));
        assert!(loc(&r).starts_with("/ayarlar?m=") && !loc(&r).contains("e=1") && loc(&r).ends_with("#ap-kurulum"), "{}", loc(&r));
        {
            let v = e.calls.lock().unwrap();
            let i = v.iter().position(|c| c[0] == "systemd-run" && c.iter().any(|x| x == "ap-adres-kaldir")).expect("zamanlayıcı");
            let j = v.iter().position(|c| c[0] == "systemd-run" && c.iter().any(|x| x == "ap-araci")).expect("aracı");
            assert!(i < j && v[j].contains(&"192.168.1.250/24".to_string()) && v[j].contains(&"--service-type=notify".to_string()));
            assert!(!v.iter().any(|c| c[0] == "ip"));
        }
        assert!(denetim().contains("AP_YAN_ODA") && denetim().contains("acildi adres=192.168.1.250/24 sure=1800"));
        // açık yan oda: kalan süre, aracı bilgisi + Şimdi kapat
        *e.ip_ek.lock().unwrap() = "araci\n".into();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("192.168.1.250/24") && a.contains("(30 dk)") && a.contains("127.0.0.1:1085") && a.contains("192.168.1.0/24"));
        assert!(a.contains("action=\"/ap-kurulum/gecici-kaldir\"") && !a.contains("action=\"/ap-kurulum/gecici-ekle\""));
        // ikinci açma reddedilir
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-ekle", &[("csrf", &csrf), ("adres", "172.16.5.1/24")], Some(&tok)));
        assert!(loc(&r).contains("e=1"));
        // aracı durmuş
        e.ip_ek.lock().unwrap().clear();
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("artık yok"));
        *e.ip_ek.lock().unwrap() = "araci\n".into();
        // kapatma: aracı + zamanlayıcı durdurulur, denetim
        e.calls.lock().unwrap().clear();
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-kaldir", &[("csrf", &csrf)], Some(&tok)));
        assert!(!loc(&r).contains("e=1"));
        {
            let v = e.calls.lock().unwrap();
            assert!(v.iter().any(|c| *c == cmd(&["systemctl", "stop", "wfc-ap-araci.service"])));
            assert!(v.iter().any(|c| c.len() > 2 && c[..2] == ["systemctl", "stop"] && c[2].starts_with("wfc-ap-gecici-")));
        }
        assert!(denetim().contains("kapandi adres=192.168.1.250/24 neden=elle"));
        assert!(!e.p.ap_yollar.gecici.exists());
        // durum kaydı olmadan çalışan aracı: Aç formu yerine Kapat; kapatma aracıyı durdurur
        e.calls.lock().unwrap().clear();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("durum kaydı yok") && a.contains("action=\"/ap-kurulum/gecici-kaldir\"") && !a.contains("action=\"/ap-kurulum/gecici-ekle\""));
        let r = e.p.handle(&req("POST", "/ap-kurulum/gecici-kaldir", &[("csrf", &csrf)], Some(&tok)));
        assert!(!loc(&r).contains("e=1"));
        assert!(e.calls.lock().unwrap().iter().any(|c| *c == cmd(&["systemctl", "stop", "wfc-ap-araci.service"])));
        e.ip_ek.lock().unwrap().clear();
        // ağdaki cihazlar: kira + komşu tablosu, adlar kaçışlı, her açılış denetimde
        std::fs::write(e.root.join("leases"), "1790712334 70:4f:57:dd:7a:e8 10.50.0.2 <b>AP</b> *\n").unwrap();
        let c = e.p.handle(&req("GET", "/ap-kurulum/cihazlar", &[], Some(&tok))).body;
        assert!(c.contains("70:4f:57:dd:7a:e8") && c.contains("&lt;b&gt;AP&lt;/b&gt;") && !c.contains("<b>AP</b>"));
        assert!(c.contains("ARP REACHABLE") && c.contains("rastgele MAC") && !c.contains("10.50.0.45"));
        assert!(denetim().contains("PANEL_AP_CIHAZLAR") && denetim().contains("sayi=2"));
        std::fs::remove_file(e.root.join("leases")).unwrap();
    }

    #[test]
    fn ap_yabanci_dhcp_denetimi() {
        let e = env();
        std::fs::create_dir_all(e.p.sys.join("br-hotspot")).unwrap();
        std::fs::write(e.p.sys.join("br-hotspot/address"), "00:0e:c4:ce:a0:9a\n").unwrap();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let denetim = || std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        let denetle = || e.p.handle(&req("POST", "/ap-kurulum/dhcp-denetle", &[("csrf", &csrf)], Some(&tok)));
        // denetim çalışmadı (çıktı yok)
        let r = denetle();
        assert!(r.status == 200 && r.body.contains("Denetim yapılamadı") && denetim().contains("sonuc=hata"), "{}", r.body);
        // yalnızca cihazın kendi sunucusu
        *e.ip_ek.lock().unwrap() = "TEKLIF 10.50.0.1 00:0e:c4:ce:a0:9a 10.50.0.77\nTAMAM 1\n".into();
        let r = denetle();
        assert!(r.body.contains("bulunmadı") && !r.body.contains("kapatılmalı") && denetim().contains("sonuc=yok"));
        // yabancı sunucu: kırmızı uyarı + adres ve MAC; denetimde sunucular
        *e.ip_ek.lock().unwrap() = "TEKLIF 192.168.1.1 aa:bb:cc:dd:ee:ff 192.168.1.33\nTEKLIF 10.50.0.1 00:0e:c4:ce:a0:9a 10.50.0.77\nTAMAM 2\n".into();
        let r = denetle();
        assert!(r.body.contains("mesaj hata") && r.body.contains("DHCP sunucusu kapatılmalı"));
        assert!(r.body.contains("192.168.1.1") && r.body.contains("aa:bb:cc:dd:ee:ff") && r.body.contains("192.168.1.33"));
        assert!(denetim().contains("AP_DHCP_DENETIM") && denetim().contains("sonuc=var sunucular=192.168.1.1/aa:bb:cc:dd:ee:ff"));
        // aynı anda ikinci denetim başlamaz (komut çalışmaz, denetime yazılmaz); bitince yeniden açılır
        let satir = denetim().lines().count();
        e.p.dhcp_denetim.store(true, std::sync::atomic::Ordering::SeqCst);
        let r = denetle();
        assert!(r.status == 200 && r.body.contains("zaten sürüyor") && denetim().lines().count() == satir);
        e.p.dhcp_denetim.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(denetle().body.contains("kapatılmalı") && !e.p.dhcp_denetim.load(std::sync::atomic::Ordering::SeqCst));
        e.ip_ek.lock().unwrap().clear();
    }

    #[test]
    fn ap_adresleri_kaydi() {
        let mut e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let (kok, ayar) = (e.root.clone(), e.p.cfg_path.clone());
        let denetim = move || std::fs::read_to_string(kok.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default();
        let ayar2 = ayar.clone();
        let kayitli = move || Config::load(&ayar2).unwrap().ap.adresler;
        let mac = move || Config::load(&ayar).unwrap().ap.mac;
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("action=\"/ap-kurulum/adresler\"") && a.contains("Kayıtlı AP adresi yok"));
        // CSRF'siz: 403, ayar değişmez
        assert_eq!(e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("adresler", "10.50.0.2")], Some(&tok))).status, 403);
        // geçersiz (havuz): mesaj, ayar ve denetim değişmez
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2\n10.50.0.30")], Some(&tok)));
        assert!(loc(&r).contains("e=1") && loc(&r).ends_with("#ap-kurulum"), "{}", loc(&r));
        assert!(kayitli().is_empty() && !denetim().contains("PANEL_AP_ADRES"));
        // geçerli: ayar dosyası + denetim; güvenlik duvarına komut gitmez. Her adresin o anki MAC'i sabitlenir (final review
        // I1; komşu tablosu: 10.50.0.2 var, 10.50.0.3 yok → bağlantı denemesinden sonra da yok: uyarı)
        e.calls.lock().unwrap().clear();
        let denenen: Arc<Mutex<Vec<String>>> = Arc::default();
        let d2 = denenen.clone();
        e.p.ap_dene = Box::new(move |ip, yan, yalniz_http| {
            d2.lock().unwrap().push(format!("{ip} {yan} {yalniz_http}"));
            None
        });
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2, 10.50.0.3 10.50.0.2")], Some(&tok)));
        assert!(loc(&r).contains("e=1") && loc(&r).contains(&pct("MAC adresi okunamadı: 10.50.0.3")), "{}", loc(&r));
        assert_eq!(kayitli(), ["10.50.0.2", "10.50.0.3"]);
        assert_eq!(mac().into_iter().collect::<Vec<_>>(), vec![("10.50.0.2".to_string(), "70:4f:57:dd:7a:e8".to_string())]);
        // yalnızca tabloda olmayan adres denenir; adres çözülsün diye tek TCP bağlantısı yeter (TLS denemesi yok)
        assert_eq!(*denenen.lock().unwrap(), ["10.50.0.3 false true"]);
        assert!(denetim().contains("PANEL_AP_ADRES") && denetim().contains("adresler=10.50.0.2,10.50.0.3 eski=- mac=10.50.0.2/70:4f:57:dd:7a:e8 eski_mac=-"));
        assert!(e.calls.lock().unwrap().is_empty());
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<code>10.50.0.2</code> (MAC <code>70:4f:57:dd:7a:e8</code>), <code>10.50.0.3</code> (MAC sabitlenmedi)"), "{a}");
        // yalnızca MAC'i sabitlenmiş adrese "arayüzüne git"
        assert!(a.contains("10.50.0.2 arayüzüne git") && !a.contains("10.50.0.3 arayüzüne git") && a.contains("MAC'i sabitlenmemiş adrese panelden gidilmez"));
        // aynı liste yeniden: MAC değişmediyse "Değişiklik yok"; AP değişti (yeni MAC): yeniden sabitlenir. Okunamayan adreste
        // önceki sabit MAC kalır
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"), "{}", loc(&r));
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2")], Some(&tok)));
        assert!(loc(&r).contains(&pct("Değişiklik yok.")), "{}", loc(&r));
        e.p.ip_oku = Box::new(|a: &[&str]| if a[0] == "ip" { "10.50.0.2 lladdr aa:bb:cc:dd:ee:01 REACHABLE\n".into() } else { String::new() });
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2")], Some(&tok)));
        assert!(!loc(&r).contains("e=1") && mac()["10.50.0.2"] == "aa:bb:cc:dd:ee:01", "{}", loc(&r));
        assert!(denetim().contains("adresler=10.50.0.2 eski=10.50.0.2 mac=10.50.0.2/aa:bb:cc:dd:ee:01 eski_mac=10.50.0.2/70:4f:57:dd:7a:e8"));
        e.p.ip_oku = Box::new(|_: &[&str]| String::new());
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "10.50.0.2")], Some(&tok)));
        assert!(loc(&r).contains("e=1") && mac()["10.50.0.2"] == "aa:bb:cc:dd:ee:01", "{}", loc(&r));
        // ağdaki cihazlar listesinde işaretlenir
        std::fs::write(e.root.join("leases"), "1790712334 70:4f:57:dd:7a:e8 10.50.0.2 * *\n").unwrap();
        assert!(e.p.handle(&req("GET", "/ap-kurulum/cihazlar", &[], Some(&tok))).body.contains("kayıtlı AP adresi"));
        std::fs::remove_file(e.root.join("leases")).unwrap();
        // boşaltma: MAC'ler de gider
        let r = e.p.handle(&req("POST", "/ap-kurulum/adresler", &[("csrf", &csrf), ("adresler", "")], Some(&tok)));
        assert!(!loc(&r).contains("e=1"));
        assert!(kayitli().is_empty() && mac().is_empty() && denetim().contains("adresler=- eski=10.50.0.2 mac=- eski_mac=10.50.0.2/aa:bb:cc:dd:ee:01"));
    }

    // ---------------------------------------------------------------- AP arayüzü (spec 2026-10-16-ap-arayuz)
    fn denetim_metni(e: &Env) -> String {
        std::fs::read_to_string(e.root.join("5651/gunluk/2026-09-29/denetim.csv")).unwrap_or_default()
    }

    /// Açık yan oda: durum dosyası + aracı çalışıyor
    fn yan_oda_ac(e: &Env, adres: &str, bitis: f64) -> crate::ap::Gecici {
        let g = crate::ap::Gecici { adres: adres.into(), kimlik: "0a1b2c3d".into(), bitis };
        ortak::write_atomic(&e.p.ap_yollar.gecici, &serde_json::to_vec(&g).unwrap()).unwrap();
        e.ip_ek.lock().unwrap().push_str("araci\n");
        g
    }

    fn git(e: &Env, tok: &str, csrf: &str, hedef: &str) -> Resp {
        e.p.handle(&req("POST", "/ap-kurulum/arayuz", &[("csrf", csrf), ("hedef", hedef)], Some(tok)))
    }

    #[test]
    fn ap_takili_cihaz_baglan_ve_sonuc() {
        let e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("Takılan cihaz") && a.contains("action=\"/ap-kurulum/baglan\"") && a.contains("action=\"/ap-kurulum/otomatik\""));
        assert!(a.contains("Etiketteki kullanıcı adı ve parolayı buraya yazmayın") && tekrar_eden_idler(&a).is_empty(), "{:?}", tekrar_eden_idler(&a));
        // CSRF'siz: 403; geçersiz etiket: mesaj, iş başlamaz
        assert_eq!(e.p.handle(&req("POST", "/ap-kurulum/baglan", &[("etiket", "192.168.1.1")], Some(&tok))).status, 403);
        assert_eq!(e.p.handle(&req("POST", "/ap-kurulum/otomatik", &[], Some(&tok))).status, 403);
        for kotu in ["8.8.8.8", "bilinmeyen.example", "10.50.0.1"] {
            let r = e.p.handle(&req("POST", "/ap-kurulum/baglan", &[("csrf", &csrf), ("etiket", kotu)], Some(&tok)));
            assert!(loc(&r).contains("e=1") && loc(&r).ends_with("#ap-kurulum"), "{kotu}: {}", loc(&r));
        }
        assert!(e.calls.lock().unwrap().iter().all(|c| c[0] != "systemd-run") && !e.p.ap_yollar.bul.exists());
        // bilinen ad + marka: [ap]'ye yazılır, iş başlar (kim + rol + istemci IP'si), bekleme sayfasına
        let r = e.p.handle(&req("POST", "/ap-kurulum/baglan", &[("csrf", &csrf), ("etiket", "TPLinkAP.net"), ("marka", "TP-Link TL-WA901ND")], Some(&tok)));
        assert_eq!(loc(&r), "/ap-kurulum/bul");
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!((c.ap.etiket.as_str(), c.ap.marka.as_str()), ("tplinkap.net", "TP-Link TL-WA901ND"));
        assert!(denetim_metni(&e).contains("PANEL_AP_ETIKET") && denetim_metni(&e).contains("etiket=tplinkap.net marka=TP-Link TL-WA901ND"));
        assert!(e.calls.lock().unwrap().contains(&crate::ap_bul::komut(MUSTERI, "sahip", IP, Some("192.168.0.254".parse().unwrap()))));
        let d = crate::ap_bul::oku(&e.p.ap_yollar).unwrap();
        assert_eq!((d.tur.as_str(), d.durum.as_str(), d.hedefler.clone()), ("baglan", "suruyor", vec!["192.168.0.254".to_string()]));
        // sürerken: bekleme sayfası kendini yeniler; ikinci istek yeni iş başlatmaz
        e.ip_ek.lock().unwrap().push_str("bul\n");
        let b = e.p.handle(&req("GET", "/ap-kurulum/bul", &[], Some(&tok)));
        assert!(b.status == 200 && b.body.contains("<meta http-equiv=\"refresh\" content=\"2;url=/ap-kurulum/bul\">") && b.body.contains("192.168.0.254 aranıyor"), "{}", b.body);
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("href=\"/ap-kurulum/bul\""));
        let n = e.calls.lock().unwrap().len();
        let r = e.p.handle(&req("POST", "/ap-kurulum/otomatik", &[("csrf", &csrf)], Some(&tok)));
        assert!(loc(&r) == "/ap-kurulum/bul" && e.calls.lock().unwrap().len() == n);
        // bitti: bekleme sayfası sonuca yönlendirir; bulunan cihaza (yan oda açık) "AP arayüzüne git", yeni sekmede
        e.ip_ek.lock().unwrap().clear();
        let g = yan_oda_ac(&e, "192.168.0.250/24", 1_790_706_000.0);
        let mut d = d;
        (d.durum, d.adres, d.port, d.sema, d.yol, d.yan_oda, d.bitis) =
            ("bulundu".into(), "192.168.0.254".into(), 80, "http".into(), "yan_oda".into(), g.adres.clone(), 1_790_705_140.0);
        crate::ap_bul::kaydet(&e.p.ap_yollar, &d).unwrap();
        assert_eq!(loc(&e.p.handle(&req("GET", "/ap-kurulum/bul", &[], Some(&tok)))), "/ayarlar#ap-kurulum");
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("Cihaz bulundu (HTTP):</b> <code>192.168.0.254</code>"), "{a}");
        assert!(a.contains("action=\"/ap-kurulum/arayuz\" target=\"_blank\"") && a.contains("name=\"hedef\" value=\"192.168.0.254\""));
        assert!(a.contains("value=\"tplinkap.net\"") && a.contains("value=\"TP-Link TL-WA901ND\"") && tekrar_eden_idler(&a).is_empty());
        assert!(!a.contains("name=\"sema\" value=\"http\""), "HTTP'den bulunan cihazda \"HTTP ile aç\" gerekmez");
        // HTTPS'ten bulunduysa (443'te TLS el sıkışması) şema görünür; yanında "HTTP ile aç" (HTTPS arayüzü bozuk AP için)
        (d.port, d.sema) = (443, "https".into());
        crate::ap_bul::kaydet(&e.p.ap_yollar, &d).unwrap();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("Cihaz bulundu (HTTPS):</b> <code>192.168.0.254</code>"), "{a}");
        assert!(a.contains("<button>AP arayüzüne git</button><button class=\"ikincil\" name=\"sema\" value=\"http\">HTTP ile aç</button>"), "{a}");
        // yan oda kapandıysa düğme yok
        e.ip_ek.lock().unwrap().clear();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("Yan oda kapandı") && !a.contains("name=\"hedef\" value=\"192.168.0.254\""));
        // otomatik bul: hedefsiz komut
        std::fs::remove_file(&e.p.ap_yollar.gecici).unwrap();
        let r = e.p.handle(&req("POST", "/ap-kurulum/otomatik", &[("csrf", &csrf)], Some(&tok)));
        assert_eq!(loc(&r), "/ap-kurulum/bul");
        assert!(e.calls.lock().unwrap().contains(&crate::ap_bul::komut(MUSTERI, "sahip", IP, None)));
        assert_eq!(crate::ap_bul::oku(&e.p.ap_yollar).unwrap().tur, "otomatik");
        // iş bitti ama sonucu yazmadı (öldü): yarıda kaldı
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("Son arama yarıda kaldı"));
        // sonuç: bulunamadı
        let mut d = crate::ap_bul::oku(&e.p.ap_yollar).unwrap();
        (d.durum, d.mesaj) = ("yok".into(), "Yaygın fabrika adreslerinde cihaz bulunamadı.".into());
        crate::ap_bul::kaydet(&e.p.ap_yollar, &d).unwrap();
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("<p class=\"hata\">Yaygın fabrika adreslerinde cihaz bulunamadı. (yaygın fabrika adresleri"));
    }

    #[test]
    fn ap_yonetim_parolasi_ve_gizli_pencere_notu() {
        // final review I1: AP'ye panel parolası değil merkezin ürettiği AP yönetim parolası yazılır; işletme sahibi de görür
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("AP yönetim parolası henüz merkezden gelmedi") && a.contains("gizli (incognito) pencerede açın"), "{a}");
        let mut m = crate::merkez::oku(&e.p.merkez_path).unwrap();
        m.ap_parola = "Kp7mQx2Rtz4W".into();
        crate::merkez::kaydet(&e.p.merkez_path, &m).unwrap();
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<b>AP yönetim parolası:</b> <code>Kp7mQx2Rtz4W</code>") && a.contains("panel parolanızı AP'ye yazmayın"), "{a}");
    }

    #[test]
    fn ap_bul_baslatilamazsa_kosan_aramaya_dokunmaz() {
        // final review M6: başlatma kilit altında; systemd-run başarısız ama arama çalışıyorsa (birim başka yerden başladı)
        // durumu "başlatılamadı" ile ezilmez. Çalışmıyorsa hata yazılır.
        let mut e = env();
        let (tok, csrf) = setup_and_login(&e, "mudur", "sahip-parola-12");
        e.p.runner = Box::new(|_: &[String]| false);
        let r = e.p.handle(&req("POST", "/ap-kurulum/otomatik", &[("csrf", &csrf)], Some(&tok)));
        assert!(loc(&r).contains("e=1") && crate::ap_bul::oku(&e.p.ap_yollar).unwrap().durum == "hata", "{}", loc(&r));
        let sayac = Arc::new(AtomicUsize::new(0));
        let s2 = sayac.clone();
        e.p.ip_oku = Box::new(move |a: &[&str]| match a {
            ["systemctl", "is-active", "wfc-ap-bul.service"] => if s2.fetch_add(1, Ordering::SeqCst) == 0 { "inactive\n" } else { "active\n" }.into(),
            _ => String::new(),
        });
        let r = e.p.handle(&req("POST", "/ap-kurulum/otomatik", &[("csrf", &csrf)], Some(&tok)));
        assert_eq!(loc(&r), "/ap-kurulum/bul");
        assert_eq!(crate::ap_bul::oku(&e.p.ap_yollar).unwrap().durum, "suruyor");
        assert_eq!(sayac.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn ap_arayuz_git_ve_kapat() {
        let mut e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        // CSRF'siz: 403
        assert_eq!(e.p.handle(&req("POST", "/ap-kurulum/arayuz", &[("hedef", "10.50.0.2")], Some(&tok))).status, 403);
        // yan oda yok, kayıtsız misafir adresi, havuz (telefon), cihazın kendisi: hedef yazılmaz
        for kotu in ["192.168.1.1", "10.50.0.2", "10.50.0.30", "10.50.0.1", "abc"] {
            let r = git(&e, &tok, &csrf, kotu);
            assert!(r.status == 200 && r.body.contains("<p class=\"hata\">") && !r.body.contains("http-equiv=\"refresh\""), "{kotu}: {}", r.body);
        }
        assert!(!e.p.ap_yollar.hedef.exists() && !denetim_metni(&e).contains("AP_ARAYUZ"));
        // yan odanın ağındaki adres: hedef (yan oda kimliği; bitiş en çok yan odanınki), denetim, merkezin geçişine meta yenileme
        let now = 1_790_705_134.0;
        let g = yan_oda_ac(&e, "192.168.1.250/24", now + 600.0);
        let r = git(&e, &tok, &csrf, "192.168.1.1");
        assert!(r.body.contains("<meta http-equiv=\"refresh\" content=\"0;url=https://cihaz.wificorrect.com/_ap-gecis\">"), "{}", r.body);
        assert!(r.body.contains("href=\"https://cihaz.wificorrect.com/_ap-gecis\""));
        let hd = crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap();
        assert_eq!((hd.adres.as_str(), hd.port, hd.sema.as_str(), hd.yol.as_str(), hd.bitis, hd.yan_oda.as_deref(), hd.kim.as_str(), hd.rol.as_str()),
                   ("192.168.1.1", 80, "http", "yan_oda", now + 600.0, Some(g.kimlik.as_str()), MUSTERI, "hizmet"));
        assert!(denetim_metni(&e).lines().any(|l| l.contains("AP_ARAYUZ") && l.contains("rol=hizmet") && l.contains("acildi hedef=192.168.1.1:80 yol=yan_oda sure=600")), "{}", denetim_metni(&e));
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("AP arayüzü erişimi açık:</b> <code>192.168.1.1</code>") && a.contains("action=\"/ap-kurulum/arayuz-kapat\""), "{a}");
        // yan odanın kendi adresi ve ağı dışı reddedilir
        assert!(git(&e, &tok, &csrf, "192.168.1.250").body.contains("class=\"hata\"") && git(&e, &tok, &csrf, "192.168.2.1").body.contains("class=\"hata\""));
        // kayıtlı ama MAC'i sabitlenmemiş adres (final review I1): gidilmez, AP'ye bağlanılmaz
        e.p.ap_dene = Box::new(|_, _, _| panic!("MAC'siz kayıtta AP'ye bağlanılmamalı"));
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.ap.adresler = vec!["10.50.0.2".into()];
        c.save(&e.p.cfg_path).unwrap();
        assert!(git(&e, &tok, &csrf, "10.50.0.2").body.contains("MAC&#x27;i sabitlenmemiş"));
        // web arayüzü cevap vermiyor: hedef değişmez
        e.p.ap_dene = Box::new(|_, _, _| None);
        c.ap.mac.insert("10.50.0.2".into(), "aa:bb:cc:dd:ee:ff".into());
        c.save(&e.p.cfg_path).unwrap();
        assert!(git(&e, &tok, &csrf, "10.50.0.2").body.contains("cevap vermedi"));
        assert_eq!(crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap().adres, "192.168.1.1");
        // komşu tablosundaki MAC (70:4f:…) sabitlenenden farklı: hedef yazılmaz, red denetimde
        e.p.ap_dene = Box::new(|ip, yan, _| (ip.to_string() == "10.50.0.2" && !yan).then_some(443));
        let r = git(&e, &tok, &csrf, "10.50.0.2");
        assert!(r.body.contains("kayıtlı MAC aa:bb:cc:dd:ee:ff, şu an 70:4f:57:dd:7a:e8") && !r.body.contains("http-equiv=\"refresh\""), "{}", r.body);
        assert_eq!(crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap().adres, "192.168.1.1");
        assert!(denetim_metni(&e).contains("red hedef=10.50.0.2 neden=mac mac=aa:bb:cc:dd:ee:ff simdi=70:4f:57:dd:7a:e8"));
        // kayıtlı AP adresi (HTTPS, MAC aynı): misafir ağından, 30 dk; hedefte sabit MAC; önceki hedef kapandı (yeni_hedef)
        c.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        c.save(&e.p.cfg_path).unwrap();
        assert!(git(&e, &tok, &csrf, "10.50.0.2").body.contains("http-equiv=\"refresh\""));
        let hd = crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap();
        assert_eq!((hd.yol.as_str(), hd.sema.as_str(), hd.port, hd.bitis, hd.yan_oda), ("misafir", "https", 443, now + 1800.0, None));
        assert_eq!(hd.mac.as_deref(), Some("70:4f:57:dd:7a:e8"));
        let d = denetim_metni(&e);
        assert!(d.contains("kapandi hedef=192.168.1.1:80 yol=yan_oda neden=yeni_hedef") && d.contains("acildi hedef=10.50.0.2:443 yol=misafir sure=1800"), "{d}");
        // AP adresleri kartında da git düğmesi
        assert!(e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body.contains("10.50.0.2 arayüzüne git"));
        // kapat
        let r = e.p.handle(&req("POST", "/ap-kurulum/arayuz-kapat", &[("csrf", &csrf)], Some(&tok)));
        assert!(!loc(&r).contains("e=1") && !e.p.ap_yollar.hedef.exists());
        assert!(denetim_metni(&e).contains("kapandi hedef=10.50.0.2:443 yol=misafir neden=elle"));
        // yan oda kapanınca onun hedefi de kapanır
        e.p.ap_dene = Box::new(|_, _, _| Some(80));
        git(&e, &tok, &csrf, "192.168.1.1");
        assert!(e.p.ap_yollar.hedef.exists());
        e.p.handle(&req("POST", "/ap-kurulum/gecici-kaldir", &[("csrf", &csrf)], Some(&tok)));
        assert!(!e.p.ap_yollar.hedef.exists() && denetim_metni(&e).contains("kapandi hedef=192.168.1.1:80 yol=yan_oda neden=yan_oda"));
    }

    #[test]
    fn ap_arayuz_https_ve_http_ile_ac() {
        // 0.4.16 (canlı ZyXEL, 2026-10-18): "AP arayüzüne git" HTTPS'i (443'te TLS el sıkışması) tercih eder; "HTTP ile aç"
        // yalnızca 80'i dener ve hedefi http/80 yazar (HTTPS arayüzü bozuk AP için). Doğrulama ve denetim aynı.
        let mut e = env();
        let (tok, csrf) = setup_and_login(&e, "admin", "hizmet-parola-1");
        let denenen: Arc<Mutex<Vec<bool>>> = Arc::default();
        let d2 = denenen.clone();
        e.p.ap_dene = Box::new(move |_, _, yalniz_http| {
            d2.lock().unwrap().push(yalniz_http);
            Some(if yalniz_http { 80 } else { 443 })
        });
        let http_ile = |hedef: &str, csrf: Option<&str>| {
            let mut f = vec![("hedef", hedef), ("sema", "http")];
            f.extend(csrf.map(|c| ("csrf", c)));
            e.p.handle(&req("POST", "/ap-kurulum/arayuz", &f, Some(&tok)))
        };
        let now = 1_790_705_134.0;
        yan_oda_ac(&e, "192.168.1.250/24", now + 600.0);
        assert!(git(&e, &tok, &csrf, "192.168.1.1").body.contains("http-equiv=\"refresh\""));
        let hd = crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap();
        assert_eq!((hd.sema.as_str(), hd.port), ("https", 443));
        assert!(denetim_metni(&e).contains("acildi hedef=192.168.1.1:443 yol=yan_oda sure=600"));
        // panelde şema; HTTPS erişiminin yanında "HTTP ile aç"
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("AP arayüzü erişimi açık:</b> <code>192.168.1.1</code> (HTTPS, yan odadan)"), "{a}");
        assert!(a.contains("name=\"sema\" value=\"http\">HTTP ile aç</button>"), "{a}");
        // HTTP ile aç: yalnızca 80, hedef http/80 (aynı adres ve yol)
        let r = http_ile("192.168.1.1", Some(&csrf));
        assert!(r.body.contains("http-equiv=\"refresh\""), "{}", r.body);
        let hd = crate::ap_arayuz::oku(&e.p.ap_yollar).unwrap();
        assert_eq!((hd.sema.as_str(), hd.port, hd.yol.as_str(), hd.bitis), ("http", 80, "yan_oda", now + 600.0));
        assert_eq!(*denenen.lock().unwrap(), [false, true]);
        let d = denetim_metni(&e);
        assert!(d.contains("acildi hedef=192.168.1.1:80 yol=yan_oda sure=600") && !d.contains("neden=yeni_hedef"), "{d}");
        // açık erişim satırında "HTTP ile aç" kalır: HTTP ile açılan erişim aynı şemayla yeniden açılabilir ("AP arayüzüne
        // git" şemayı yeniden seçer)
        let a = e.p.handle(&req("GET", "/ayarlar", &[], Some(&tok))).body;
        assert!(a.contains("<code>192.168.1.1</code> (HTTP, yan odadan)") && a.contains("name=\"sema\" value=\"http\">HTTP ile aç</button>"), "{a}");
        // doğrulama aynı: CSRF'siz 403, yan oda dışı adres reddedilir (AP'ye bağlanılmaz)
        assert_eq!(http_ile("192.168.1.1", None).status, 403);
        let r = http_ile("192.168.2.1", Some(&csrf));
        assert!(r.body.contains("class=\"hata\"") && !r.body.contains("http-equiv=\"refresh\"") && denenen.lock().unwrap().len() == 2, "{}", r.body);
        // 80 cevap vermiyor: hedef değişmez
        e.p.ap_dene = Box::new(|_, _, yalniz_http| (!yalniz_http).then_some(443));
        let r = e.p.handle(&req("POST", "/ap-kurulum/arayuz", &[("csrf", &csrf), ("hedef", "192.168.1.1"), ("sema", "http")], Some(&tok)));
        assert!(r.body.contains("HTTP (80) web arayüzü cevap vermedi") && !r.body.contains("http-equiv=\"refresh\""), "{}", r.body);
        assert_eq!(crate::ap_arayuz::oku(&e.p.ap_yollar).map(|h| h.port), Some(80));
    }

    #[test]
    fn ap_aktarim_karari_ve_eski_merkez() {
        let e = env();
        // bağlı değil: başlık yok sayılır (normal panel isteği)
        assert!(matches!(e.p.ap_karar("10.99.0.1", Some(MUSTERI)), ap_aktarim::ApKarar::Panel));
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        // merkezden değil (modem ağı, başka tünel adresi, yerel): yok sayılır
        for ip in ["192.168.1.50", "10.99.0.7", "127.0.0.1"] {
            assert!(matches!(e.p.ap_karar(ip, Some(MUSTERI)), ap_aktarim::ApKarar::Panel), "{ip}");
        }
        // tünel arayüzü yokken merkez adresi taklit edilebilir: yok sayılır
        std::fs::remove_dir(e.p.sys.join("wfc")).unwrap();
        assert!(matches!(e.p.ap_karar("10.99.0.1", Some(MUSTERI)), ap_aktarim::ApKarar::Panel));
        std::fs::create_dir_all(e.p.sys.join("wfc")).unwrap();
        let durum = |k: Option<&str>| match e.p.ap_karar("10.99.0.1", k) {
            ap_aktarim::ApKarar::Cevap(r) => r.status,
            ap_aktarim::ApKarar::Aktar(_) => 1,
            ap_aktarim::ApKarar::Panel => 0,
        };
        // başka müşteri / numarasız: 403
        assert_eq!((durum(Some("1234567")), durum(None)), (403, 403));
        // hedef yok: kapalı sayfası (panele bağlantı; /_ap-gecis'e bağlantı yok: merkez aynı siteden geleni reddeder)
        match e.p.ap_karar("10.99.0.1", Some(MUSTERI)) {
            ap_aktarim::ApKarar::Cevap(r) => assert!(r.status == 403 && r.body.contains("AP erişimi kapalı") && !r.body.contains("_ap-gecis"), "{}", r.body),
            _ => panic!("kapalı sayfası bekleniyor"),
        }
        // kapalı sayfasında temizleyerek çıkış (merkezin /_ap-cikis'i, aynı kökte)
        match e.p.ap_karar("10.99.0.1", Some(MUSTERI)) {
            ap_aktarim::ApKarar::Cevap(r) => assert!(r.body.contains("href=\"/_ap-cikis\"") && r.body.contains("oturumlarınız da kapanır"), "{}", r.body),
            _ => panic!("kapalı sayfası bekleniyor"),
        }
        // kayıtlı AP adresine (MAC'i sabitlenmiş) hedef: aktarım
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.ap.adresler = vec!["10.50.0.2".into()];
        c.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        c.save(&e.p.cfg_path).unwrap();
        let hd = crate::ap_arayuz::Hedef {
            adres: "10.50.0.2".into(), port: 80, sema: "http".into(), yol: "misafir".into(), bitis: 1_790_705_200.0,
            kim: MUSTERI.into(), rol: "sahip".into(), yan_oda: None, mac: Some("70:4f:57:dd:7a:e8".into()),
        };
        crate::ap_arayuz::yaz(&e.p.ap_yollar, &hd).unwrap();
        assert!(matches!(e.p.ap_karar("10.99.0.1", Some(MUSTERI)), ap_aktarim::ApKarar::Aktar(h) if h == hd));
        // süre doldu: kapalı + denetim (kim hedeften)
        crate::ap_arayuz::yaz(&e.p.ap_yollar, &crate::ap_arayuz::Hedef { bitis: 1_790_705_000.0, ..hd.clone() }).unwrap();
        assert_eq!(durum(Some(MUSTERI)), 403);
        assert!(denetim_metni(&e).contains(&format!("kullanici={MUSTERI} rol=sahip ip=- kapandi hedef=10.50.0.2:80 yol=misafir neden=sure")));
        assert!(!e.p.ap_yollar.hedef.exists());
        // merkez eski sürümdeyse /_ap-gecis cihaza panel isteği olarak gelir
        let r = e.p.handle(&req("GET", "/_ap-gecis", &[], Some(&tok)));
        assert!(r.status == 503 && r.body.contains("AP erişimi şu an kullanılamıyor"), "{}", r.body);
    }

    /// tiny_http üzerinden uçtan uca: istek (yerel, sahte) AP'ye gider, cevap panel başlıkları olmadan akar.
    #[test]
    fn ap_aktarimi_tiny_http_uzerinden() {
        use std::io::Write;
        use std::net::{TcpListener, TcpStream};
        let ap = TcpListener::bind("127.0.0.1:0").unwrap();
        let ap_adres = ap.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in ap.incoming().flatten().take(1) {
                let mut s = s;
                let mut b = vec![];
                let mut tek = [0u8; 1];
                while !b.ends_with(b"\r\n\r\n") && s.read(&mut tek).unwrap_or(0) == 1 {
                    b.push(tek[0]);
                }
                let istek = String::from_utf8_lossy(&b).to_string();
                let govde = format!("<a href=\"http://192.168.1.1/x\">{}</a>", istek.lines().next().unwrap_or(""));
                let _ = s.write_all(format!("HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nX-Frame-Options: SAMEORIGIN\r\n\r\n{govde}").as_bytes());
            }
        });
        let mut e = env();
        e.p.ap_baglan = Box::new(move |_: &crate::ap_arayuz::Hedef| Ok(Box::new(TcpStream::connect(ap_adres)?) as Box<dyn crate::ap_arayuz::Akis>));
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let sunucu = tiny_http::Server::http(("127.0.0.1", port)).unwrap();
        let istemci = |istek: &'static [u8]| {
            std::thread::spawn(move || {
                let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.write_all(istek).unwrap();
                let mut v = String::new();
                c.read_to_string(&mut v).unwrap();
                v
            })
        };
        let hd = crate::ap_arayuz::Hedef {
            adres: "192.168.1.1".into(), port: 80, sema: "http".into(), yol: "yan_oda".into(), bitis: 9e9,
            kim: MUSTERI.into(), rol: "sahip".into(), yan_oda: Some("0a1b2c3d".into()), mac: None,
        };
        // Service Worker betiği ve başka siteden istek AP'ye hiç gitmez (final review C1, I2): bağlantı kurulmadan 403
        for istek in [&b"GET /sw.js HTTP/1.0\r\nHost: ap.wificorrect.com\r\nService-Worker: script\r\n\r\n"[..],
                      &b"GET /userRpm/WlanSecurityRpm.htm?Save=Save HTTP/1.0\r\nHost: ap.wificorrect.com\r\nSec-Fetch-Site: cross-site\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n\r\n"[..]] {
            let t = istemci(istek);
            ap_aktarim::ap_isle(&e.p, sunucu.recv().unwrap(), hd.clone());
            let cevap = t.join().unwrap();
            assert!(cevap.starts_with("HTTP/1.0 403") && cevap.contains("iletil") && !cevap.contains("Content-Security-Policy"), "{cevap}");
        }
        let t = istemci(b"GET /userRpm/Index.htm HTTP/1.0\r\nHost: ap.wificorrect.com\r\nX-WFC-AP: 1\r\nCookie: _; SID=1\r\n\r\n");
        ap_aktarim::ap_isle(&e.p, sunucu.recv().unwrap(), hd.clone());
        let cevap = t.join().unwrap();
        assert!(cevap.starts_with("HTTP/1.0 200"), "{cevap}");
        assert!(cevap.contains("<a href=\"https://ap.wificorrect.com/x\">GET /userRpm/Index.htm HTTP/1.0</a>"), "{cevap}");
        assert!(cevap.contains("X-Frame-Options: SAMEORIGIN") && cevap.contains("Cache-Control: no-store"), "{cevap}");
        assert!(!cevap.contains("Content-Security-Policy") && !cevap.contains("DENY"), "panel başlıkları yok: {cevap}");
        // AP'ye ulaşılamıyor: sade 502, panel başlıkları yok
        e.p.ap_baglan = Box::new(|_: &crate::ap_arayuz::Hedef| Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        let t = istemci(b"GET / HTTP/1.0\r\nHost: ap.wificorrect.com\r\n\r\n");
        ap_aktarim::ap_isle(&e.p, sunucu.recv().unwrap(), hd.clone());
        let cevap = t.join().unwrap();
        assert!(cevap.starts_with("HTTP/1.0 502") && cevap.contains("AP&#x27;ye ulaşılamadı"), "{cevap}");
        assert!(!cevap.contains("Content-Security-Policy") && !cevap.contains("X-Frame-Options"), "{cevap}");
        // kayıtlı AP adresinde başka MAC (final review I1): 403, erişim kapanır (bir kez denetim), AP'ye istek yazılmaz
        crate::ap_arayuz::yaz(&e.p.ap_yollar, &hd).unwrap();
        e.p.ap_baglan = Box::new(|_: &crate::ap_arayuz::Hedef| {
            Err(std::io::Error::other(crate::ap_arayuz::MacFarkli { kayitli: "70:4f:57:dd:7a:e8".into(), simdi: Some("de:ad:be:ef:00:01".into()) }))
        });
        for _ in 0..2 {
            let t = istemci(b"GET / HTTP/1.0\r\nHost: ap.wificorrect.com\r\n\r\n");
            ap_aktarim::ap_isle(&e.p, sunucu.recv().unwrap(), hd.clone());
            let cevap = t.join().unwrap();
            assert!(cevap.starts_with("HTTP/1.0 403") && cevap.contains("MAC adresi değişmiş") && cevap.contains("de:ad:be:ef:00:01"), "{cevap}");
        }
        assert!(!e.p.ap_yollar.hedef.exists());
        let satirlar: Vec<String> = denetim_metni(&e).lines().filter(|l| l.contains("neden=mac")).map(String::from).collect();
        assert!(satirlar.len() == 1 && satirlar[0].contains("kapandi hedef=192.168.1.1:80 yol=yan_oda neden=mac mac=70:4f:57:dd:7a:e8 simdi=de:ad:be:ef:00:01"), "{satirlar:?}");
    }

    #[test]
    fn panel_sertifikasi_bozuksa_yeniden_uretilir() {
        use std::os::unix::fs::PermissionsExt;
        let e = env();
        let (c, k) = (e.root.join("panel.crt"), e.root.join("panel.key"));
        let pem = |t: &str| format!("-----BEGIN {t}-----\n{}\n-----END {t}-----\n", "QUJD".repeat(20));
        let sayac = AtomicUsize::new(0);
        let uret = |yc: &std::path::Path, yk: &std::path::Path| {
            sayac.fetch_add(1, Ordering::SeqCst);
            // anahtar dosyası openssl'den önce 600 açılmış olmalı
            assert_eq!(std::fs::metadata(yk).unwrap().permissions().mode() & 0o777, 0o600);
            std::fs::write(yc, pem("CERTIFICATE")).is_ok() && std::fs::write(yk, pem("PRIVATE KEY")).is_ok()
        };
        let n = || sayac.load(Ordering::SeqCst);
        // yok: üretilir; geçerliyse dokunulmaz
        let (cc, kk) = sertifika_hazirla(&c, &k, false, &uret).unwrap();
        assert!(sertifika_gecerli(&cc, &kk) && n() == 1);
        assert_eq!(std::fs::metadata(&k).unwrap().permissions().mode() & 0o777, 0o600);
        sertifika_hazirla(&c, &k, false, &uret).unwrap();
        assert_eq!(n(), 1);
        // 0 baytlık sertifika (Visavis), okunamayan anahtar türü, zorla: yeniden
        std::fs::write(&c, "").unwrap();
        sertifika_hazirla(&c, &k, false, &uret).unwrap();
        std::fs::write(&k, "-----BEGIN EC PRIVATE KEY-----\nQUJD\n-----END EC PRIVATE KEY-----\n").unwrap();
        sertifika_hazirla(&c, &k, false, &uret).unwrap();
        sertifika_hazirla(&c, &k, true, &uret).unwrap();
        assert!(n() == 4 && !std::fs::read(&c).unwrap().is_empty());
        // üretim başarısız ya da bozuk: eski dosyalar yerinde, geçici dosya kalmaz
        std::fs::write(&c, "").unwrap();
        assert!(sertifika_hazirla(&c, &k, false, &|_: &std::path::Path, _: &std::path::Path| false).is_err());
        assert!(sertifika_hazirla(&c, &k, false, &|yc: &std::path::Path, _: &std::path::Path| std::fs::write(yc, "x").is_ok()).is_err());
        assert!(std::fs::read(&c).unwrap().is_empty() && !c.with_extension("crt.yeni").exists() && !k.with_extension("key.yeni").exists());
        assert!(!pem_ok(b"", &["CERTIFICATE"]) && !pem_ok(b"-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----", &["CERTIFICATE"]));
    }

    #[test]
    fn portal_metinleri_ornek_ek_gercek() {
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.site_name = "Göztepe Bilgisayar".into();
        c.save(&e.p.cfg_path).unwrap();
        let b = e.p.handle(&req("GET", "/portal-metinleri", &[], Some(&tok))).body;
        assert!(b.contains("İŞLETMECİ’nin → Göztepe Bilgisayar’ın gibi") && !b.contains("Bilgisayar’nin"), "{b}");
    }

    #[test]
    fn portal_metinleri_isletme_bilgileriyle_dolu_gorunur() {
        // işletme adı, unvanı ve adresi yazılınca metinler panelde de giriş sayfasındaki gibi dolu görünür (2026-10-26: Göztepe ve
        // Visavis'te panel şablonu gösteriyordu); düzenleme kutusunda şablon kalır: bilgiler değişince metin de değişsin
        let e = env();
        let (tok, _) = setup_and_login(&e, "mudur", "sahip-parola-12");
        let mut c = Config::load(&e.p.cfg_path).unwrap();
        c.main.site_name = "Göztepe Bilgisayar".into();
        c.main.unvan = "Boca Gıda Tic. Ltd. Şti.".into();
        c.main.adres = "Örnek Mah. 1. Sok. No 2 İzmir".into();
        c.main.telefon = String::new(); // isteğe bağlı bilgi boş: geçtiği satır gösterilmez
        c.save(&e.p.cfg_path).unwrap();
        let b = e.p.handle(&get("/portal-metinleri", &[], &tok)).body;
        let parcalar = |ac: &str, kapa: &str| b.split(ac).skip(1).map(|p| p.split(kapa).next().unwrap().to_string()).collect::<Vec<_>>();
        let onizleme = parcalar("<div class=\"metin-onizleme\">", "</div>");
        let kutular = parcalar("<textarea", "</textarea>");
        assert_eq!((onizleme.len(), kutular.len()), (6, 6), "{b}");
        let dolu = onizleme.concat();
        assert!(dolu.contains("Boca Gıda Tic. Ltd. Şti.") && dolu.contains("Örnek Mah. 1. Sok. No 2 İzmir") && dolu.contains("Göztepe Bilgisayar’ın"), "{dolu}");
        for y in ["[vergi levhası unvanı]", "[başvuru adresi]", "[irtibat numarası]", "İŞLETMECİ"] {
            assert!(!dolu.contains(y), "{y}: {dolu}");
        }
        // giriş sayfasındakiyle aynı metin (aynı doldurma işlevi ve biçim)
        let c = Config::load(&e.p.cfg_path).unwrap();
        assert_eq!(onizleme[0], crate::portal::metin_html_dil(&crate::portal::metin_doldur(&c.portal.aydinlatma, &c.main, false), crate::dil::Dil::Tr));
        // boş İngilizce metin: giriş sayfası Türkçesini gösterir
        assert!(onizleme[3].contains("Türkçesi gösterilir"), "{}", onizleme[3]);
        // düzenleme kutusunda şablon duruyor: dokunmadan kaydetmek yer tutucuları korur
        assert!(kutular[0].contains("[vergi levhası unvanı]") && kutular[0].contains("[irtibat numarası]") && kutular[2].contains("İŞLETMECİ"));
    }
}
