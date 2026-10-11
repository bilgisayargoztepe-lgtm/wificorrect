//! Müşteri giriş portalı (eski portal.py; docs/MASTER_ENGINEERING.md §11, RUST_YENIDEN_YAZIM.md A4).
//! Akış: form → hız sınırları → SMS kodu → doğrulama → nft yetkisi + oturum + 5651 kayıtları.

use crate::ayar::Config;
use crate::dil::{self, Dil};
use crate::ortak::{self, Row, Runner, Session};
use crate::sms::{self, Saglayici, Sonuc};
use crate::{kimlik, misafir, ulkeler};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

const AFTER_LOGIN_URL: &str = "http://www.google.com/"; // istenen sayfa bilinmiyorsa başarıdan sonra

pub type Form = HashMap<String, String>;
pub type Clock = dyn Fn() -> f64 + Send + Sync;
/// (ayar, sağlayıcı, numara E.164, kod) → sonuç. Twilio gerçek modda kodu kendisi üretir.
pub type SmsFn = dyn Fn(&Config, Saglayici, &str, &str) -> Sonuc + Send + Sync;
/// Twilio Verify uzaktan doğrulama: (onaylandı, ağ hatası).
pub type CheckFn = dyn Fn(&Config, &str, &str) -> (bool, Option<String>) + Send + Sync;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner()) // bir istekteki panik diğerlerini kilitlemesin
}

/// Yalnızca http(s) ve ≤ 2048 karakter (açık yönlendirme koruması).
pub fn safe_dst(dst: &str) -> String {
    let d = dst.trim();
    if d.len() <= 2048 && (d.starts_with("http://") || d.starts_with("https://")) {
        d.to_string()
    } else {
        String::new()
    }
}

/// Kayan pencere sayacı. ponytail: anahtarlar bellekten silinmez; kafe ölçeğinde önemsiz, gerekirse periyodik temizlik.
#[derive(Default)]
pub struct RateLimiter(Mutex<HashMap<String, VecDeque<f64>>>);

impl RateLimiter {
    pub fn count(&self, key: &str, window: f64, now: f64) -> usize {
        let mut m = lock(&self.0);
        let q = m.entry(key.to_string()).or_default();
        while q.front().is_some_and(|&t| t <= now - window) {
            q.pop_front();
        }
        q.len()
    }

    pub fn hit(&self, key: &str, now: f64) {
        lock(&self.0).entry(key.to_string()).or_default().push_back(now);
    }
}

#[derive(Debug)]
pub struct Page {
    pub tpl: &'static str,
    pub ctx: Vec<(&'static str, String)>,
    pub dil: Dil,
}

impl Page {
    fn new(tpl: &'static str, ctx: Vec<(&'static str, String)>, dil: Dil) -> Page {
        Page { tpl, ctx, dil }
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> &str {
        self.ctx.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[derive(Clone, Debug)]
struct Pending {
    phone: String,
    ad: String,
    soyad: String,
    code: String,
    dst: String,
    expires: f64,
    attempts: u32,
    sent_at: f64,
    provider: Saglayici,
    /// Açık rıza kutusu işaretli miydi (oturum kaydına yazılır)
    riza: bool,
    /// Uzaktan doğrulama sürerken kod yenilendiyse sonuç eski koda uygulanmasın
    nonce: u64,
}

/// Oturum açacak giriş: kişi anahtarı (telefon ya da kimlik), ad, soyad; `bitis` verilmişse oturum o anda biter
/// (misafir kodu), yoksa ayardaki oturum süresi. `ek` OTURUM_BASLA'nın `ek` alanına `bitis=…`'ten sonra eklenir.
struct Giris<'a> {
    kisi: &'a str,
    ad: &'a str,
    soyad: &'a str,
    bitis: Option<f64>,
    ek: String,
}

/// Misafir kodu kaba kuvvet koruması: IP ve MAC başına bu kadar hata → kilit (SMS kod sayaçlarından ayrı).
const MISAFIR_HATA_SINIRI: usize = 5;
const MISAFIR_KILIT_SN: f64 = 15.0 * 60.0;

pub struct Portal {
    pub cfg: Config,
    pending: Mutex<HashMap<String, Pending>>,
    rl: RateLimiter,
    clock: Box<Clock>,
    wall: Box<Clock>,
    runner: Box<Runner>,
    send_sms: Box<SmsFn>,
    check_sms: Box<CheckFn>,
    nonce: AtomicU64,
    pub host: String,
    pub base_url: String,
    /// Yönetim merkezi bağı: bağlı değilse ya da lisans kapalıysa misafire internet verilmez (testte geçici)
    pub merkez_path: std::path::PathBuf,
}

fn template(name: &str) -> &'static str {
    match name {
        "_sayfa" => include_str!("sablon/_sayfa.html"),
        "giris" => include_str!("sablon/giris.html"),
        "kod" => include_str!("sablon/kod.html"),
        "basarili" => include_str!("sablon/basarili.html"),
        "hata" => include_str!("sablon/hata.html"),
        "misafir" => include_str!("sablon/misafir.html"),
        _ => "",
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// `$ad` yer tutucularını doldurur; bilinmeyeni olduğu gibi bırakır (Python string.Template.safe_substitute).
pub fn substitute(tpl: &str, values: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(stripped) = after.strip_prefix('$') {
            out.push('$');
            rest = stripped;
            continue;
        }
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let name = &after[..n];
        match values.get(name) {
            Some(v) if !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) => out.push_str(v),
            _ => {
                out.push('$');
                out.push_str(name);
            }
        }
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

/// Şablondaki `$m_<anahtar>` → sayfa dilindeki metin (dil.rs; güvenilir sabit, kaçışsız). Metnin içindeki `$ad`
/// yer tutucuları ikinci geçişte (substitute) dolar.
fn yerel(tpl: &str, d: Dil) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find("$m_") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 3..];
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        out.push_str(dil::m(d, &after[..n]));
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub fn metin_html(text: &str) -> String {
    metin_html_dil(text, Dil::Tr)
}

/// Panelden yazılan düz metin → güvenli HTML: boş satır paragraf, tek satır sonu <br>. Boşsa bilgi notu (sayfa dilinde).
pub fn metin_html_dil(text: &str, d: Dil) -> String {
    let text = text.replace("\r\n", "\n");
    let paras: Vec<String> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            // "* " / "• " / "- " ile başlayan satırlar madde listesi; tamamı büyük harf satırlar (başlıklar) kalın
            let mut html = String::new();
            let (mut lines, mut items): (Vec<String>, Vec<String>) = (vec![], vec![]);
            let flush = |html: &mut String, lines: &mut Vec<String>, items: &mut Vec<String>| {
                if !lines.is_empty() {
                    html.push_str(&format!("<p>{}</p>", lines.join("<br>")));
                    lines.clear();
                }
                if !items.is_empty() {
                    html.push_str(&format!("<ul>{}</ul>", items.iter().map(|i| format!("<li>{i}</li>")).collect::<String>()));
                    items.clear();
                }
            };
            for l in p.lines().map(str::trim) {
                if let Some(item) = l.strip_prefix("* ").or_else(|| l.strip_prefix("• ")).or_else(|| l.strip_prefix("- ")) {
                    if !lines.is_empty() {
                        flush(&mut html, &mut lines, &mut vec![]);
                    }
                    items.push(html_escape(item.trim()));
                } else {
                    if !items.is_empty() {
                        flush(&mut html, &mut vec![], &mut items);
                    }
                    let e = html_escape(l);
                    let heading = l.chars().any(char::is_alphabetic) && !l.chars().any(char::is_lowercase);
                    lines.push(if heading { format!("<strong>{e}</strong>") } else { e });
                }
            }
            flush(&mut html, &mut lines, &mut items);
            html
        })
        .collect();
    if paras.is_empty() { format!("<p class=\"not\">{}</p>", dil::m(d, "metin_yok")) } else { paras.concat() }
}

fn tr_lower(s: &str) -> String {
    s.chars().map(|c| match c { 'I' => "ı".into(), 'İ' => "i".into(), c => c.to_lowercase().to_string() }).collect()
}

fn tr_upper(s: &str) -> String {
    s.chars().map(|c| match c { 'i' => "İ".into(), 'ı' => "I".into(), c => c.to_uppercase().to_string() }).collect()
}

/// Ada uygun ek (ünlü uyumu): 'y' yönelme (’ye/’ya), 'n' tamlayan (’nin/’ın/’un/’ün), 'd' ayrılma (’den/’tan).
/// ponytail: yazımdan çıkarılır; okunuşu farklı yabancı adlarda (Starbucks) ek yazıma göre olur.
fn ek(name: &str, kind: char) -> String {
    const UNLU: &str = "aeıioöuü";
    let l: Vec<char> = tr_lower(name).chars().filter(|c| c.is_alphabetic()).collect();
    let last_v = l.iter().rev().find(|c| UNLU.contains(**c)).copied().unwrap_or('e');
    let ends_vowel = l.last().is_some_and(|c| UNLU.contains(*c));
    let a2 = if "aıou".contains(last_v) { 'a' } else { 'e' };
    let i4 = match last_v {
        'a' | 'ı' => 'ı',
        'o' | 'u' => 'u',
        'ö' | 'ü' => 'ü',
        _ => 'i',
    };
    match kind {
        'y' => format!("{}{a2}", if ends_vowel { "y" } else { "" }),
        'n' => format!("{}{i4}n", if ends_vowel { "n" } else { "" }),
        _ => format!("{}{a2}n", if l.last().is_some_and(|c| "çfhkpsşt".contains(*c)) { 't' } else { 'd' }),
    }
}

/// Metindeki "İŞLETMECİ" → kafe adı; kesme işaretinden sonraki ek ada göre yeniden kurulur, büyük harfli yerde ad da büyük.
pub fn isletme_yerlestir(text: &str, name: &str) -> String {
    const KELIME: &str = "İŞLETMECİ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(KELIME) {
        out.push_str(&rest[..i]);
        rest = &rest[i + KELIME.len()..];
        let apos = rest.chars().next().filter(|c| *c == '’' || *c == '\'');
        let suffix: String = apos.map_or(String::new(), |a| rest[a.len_utf8()..].chars().take_while(|c| c.is_alphabetic()).collect());
        let upper = !suffix.is_empty() && !suffix.chars().any(char::is_lowercase);
        let kind = match tr_lower(&suffix).chars().next() {
            Some('y') => Some('y'),
            Some('n') => Some('n'),
            Some('d') | Some('t') => Some('d'),
            _ => None,
        };
        match (apos, kind) {
            (Some(a), Some(k)) => {
                let e = ek(name, k);
                out.push_str(&if upper { format!("{}{a}{}", tr_upper(name), tr_upper(&e)) } else { format!("{name}{a}{e}") });
                rest = &rest[a.len_utf8() + suffix.len()..];
            }
            _ => out.push_str(name),
        }
    }
    out.push_str(rest);
    out
}

/// Sabit sürede karşılaştırma (zamanlama ile kod tahminini önler).
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn otp_code() -> String {
    loop {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
        let x = u32::from_le_bytes(b);
        if x < 4_294_000_000 {
            // eşit dağılım için 10^6'nın katının üstü atılır
            return format!("{:06}", x % 1_000_000);
        }
    }
}

impl Portal {
    pub fn new(cfg: Config, clock: Box<Clock>, wall: Box<Clock>, runner: Box<Runner>, send_sms: Box<SmsFn>, check_sms: Box<CheckFn>) -> Portal {
        let host = format!("{}:{}", cfg.main.router_ip, cfg.main.portal_port);
        Portal {
            base_url: format!("http://{host}"),
            host,
            cfg,
            pending: Mutex::default(),
            rl: RateLimiter::default(),
            clock,
            wall,
            runner,
            send_sms,
            check_sms,
            nonce: AtomicU64::new(1),
            merkez_path: crate::merkez::PATH.into(),
        }
    }

    /// Bekleyen koddan formu yeniden doldurmak için (numara `+` ile, ülke seçimi yok sayılır).
    fn prefill(p: &Pending, d: Dil) -> Form {
        let riza = if p.riza { "1" } else { "" };
        [("ad", p.ad.clone()), ("soyad", p.soyad.clone()), ("telefon", format!("+{}", p.phone)), ("sozlesme", "1".into()), ("riza", riza.into()), ("dst", p.dst.clone()), ("dil", d.kod().into())]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    // --- sayfalar
    pub fn render(&self, page: &Page) -> String {
        let d = page.dil;
        let mut values: HashMap<&str, String> = HashMap::new();
        values.insert("dil_kod", d.kod().into());
        values.insert("site", self.cfg.main.site_name.clone());
        // "Bocafe'ye", "Starbucks'a": ek ünlü uyumuyla, kesme işaretiyle ayrılır
        values.insert("site_ye", format!("{}'{}", self.cfg.main.site_name, ek(&self.cfg.main.site_name, 'y')));
        values.insert("portal", self.base_url.clone());
        for (k, v) in &page.ctx {
            values.insert(k, v.clone());
        }
        let safe: HashMap<&str, String> =
            values.into_iter().map(|(k, v)| (k, if k.ends_with("_html") { v } else { html_escape(&v) })).collect();
        let body = substitute(&yerel(template(page.tpl), d), &safe);
        let mut outer = HashMap::new();
        outer.insert("icerik_html", body);
        outer.insert("site", safe["site"].clone());
        outer.insert("bas_html", safe.get("bas_html").cloned().unwrap_or_default());
        outer.insert("dil_kod", d.kod().into());
        outer.insert("dil_secim_html", self.dil_secimi(page));
        substitute(&yerel(template("_sayfa"), d), &outer)
    }

    /// Sayfanın üstündeki "TR | EN": istenen adres (`dst`) korunur; kod sayfasındaysa `sayfa=kod` ile kod sayfası
    /// yeni dilde yeniden açılır (bekleyen kod bellekte).
    fn dil_secimi(&self, page: &Page) -> String {
        let yol = if page.tpl == "misafir" { "/misafir" } else { "/" };
        let mut ek = String::new();
        if page.tpl == "kod" {
            ek.push_str("&amp;sayfa=kod");
        }
        let dst = page.ctx.iter().find(|(k, _)| *k == "dst").map_or("", |(_, v)| v.as_str());
        if !dst.is_empty() {
            ek.push_str(&format!("&amp;dst={}", pct_encode(dst)));
        }
        let a = |x: Dil, ad: &str| {
            let cur = if x == page.dil { " aria-current=\"true\"" } else { "" };
            format!("<a href=\"{}{yol}?dil={}{ek}\" lang=\"{}\"{cur}>{ad}</a>", html_escape(&self.base_url), x.kod(), x.kod())
        };
        format!("<nav class=\"dil\" aria-label=\"{}\">{} | {}</nav>", dil::m(page.dil, "dil_secimi"), a(Dil::Tr, "TR"), a(Dil::En, "EN"))
    }

    #[cfg(test)]
    fn form_page(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str) -> Page {
        self.form_page_dil(form, errors, dst, Dil::Tr)
    }

    fn form_page_dil(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str, d: Dil) -> Page {
        let f = |k: &str| form.get(k).cloned().unwrap_or_default();
        let pm = &self.cfg.portal;
        let mut ctx = vec![
            ("ad", f("ad")),
            ("soyad", f("soyad")),
            ("telefon", f("telefon")),
            ("sozlesme_checked", if f("sozlesme") == "1" { "checked".into() } else { String::new() }),
            ("riza_checked", if f("riza") == "1" { "checked".into() } else { String::new() }),
            // açık rıza metni onay kutusunun yanında (kısa metin; satır sonları korunur)
            ("riza_metni_html", html_escape(&self.hukuki(d, &pm.acik_riza, &pm.acik_riza_en)).replace('\n', "<br>")),
            ("sozlesme_html", metin_html_dil(&self.hukuki(d, &pm.sozlesme, &pm.sozlesme_en), d)),
            ("aydinlatma_html", metin_html_dil(&self.hukuki(d, &pm.aydinlatma, &pm.aydinlatma_en), d)),
            ("dst", dst.to_string()),
            ("ulke_secenekleri_html", ulkeler::options_html(form.get("ulke").map_or("TR", String::as_str))),
            ("misafir_html", self.misafir_dugmesi(dst, d)),
        ];
        for k in ["hata_genel", "hata_ad", "hata_soyad", "hata_telefon", "hata_riza", "hata_sozlesme"] {
            let v = errors.iter().find(|(e, _)| *e == k).map(|(_, v)| v.clone()).unwrap_or_default();
            ctx.push((k, v));
        }
        Page::new("giris", ctx, d)
    }

    /// Metinlerdeki İŞLETMECİ → işletme adı, [vergi levhası unvanı] → unvan (girilmemişse yer tutucu kalır).
    fn yerlestir(&self, text: &str) -> String {
        self.unvan_yerlestir(isletme_yerlestir(text, &self.cfg.main.site_name))
    }

    fn unvan_yerlestir(&self, t: String) -> String {
        let unvan = self.cfg.main.unvan.trim();
        if unvan.is_empty() { t } else { t.replace("[vergi levhası unvanı]", unvan) }
    }

    /// Hukuki metin sayfa dilinde: İngilizcesi yazılmamışsa Türkçesi. İngilizcede İŞLETMECİ yalnızca adla değişir (ek yok).
    fn hukuki(&self, d: Dil, tr: &str, en: &str) -> String {
        if d == Dil::En && !en.trim().is_empty() {
            self.unvan_yerlestir(en.replace("İŞLETMECİ", &self.cfg.main.site_name))
        } else {
            self.yerlestir(tr)
        }
    }

    fn code_page(&self, p: &Pending, note: &str, error: &str, d: Dil) -> Page {
        let dakika = (self.cfg.limits.otp_ttl_sec / 60).max(1);
        Page::new(
            "kod",
            vec![
                ("telefon_maskeli", ulkeler::mask_phone(&p.phone)),
                ("dakika", dakika.to_string()),
                ("not", note.into()),
                ("hata", error.into()),
            ],
            d,
        )
    }

    fn error_page(&self, msg: &str, d: Dil) -> Page {
        Page::new("hata", vec![("mesaj", msg.into())], d)
    }

    /// JS yok (CSP): 1 sn sonra yönlen. Telefonların giriş penceresi interneti görünce kendini kapatır.
    fn success_page(&self, dst: &str, d: Dil) -> Page {
        let target = html_escape(if dst.is_empty() { AFTER_LOGIN_URL } else { dst });
        Page::new("basarili", vec![("bas_html", format!("<meta http-equiv=\"refresh\" content=\"1;url={target}\">"))], d)
    }

    fn audit(&self, olay: &str, ip: &str, mac: &str, phone: &str, ek: &str) {
        let row = Row::new(olay, &ortak::now_iso((self.wall)()))
            .set("telefon", phone)
            .set("mac", mac)
            .set("ic_ip", ip)
            .set("ek", ek);
        ortak::audit(&self.cfg.main.log_root, row);
    }

    fn session_for(&self, ip: &str, mac: &str) -> bool {
        ortak::load_sessions(&self.cfg.main.state_root)
            .get(mac)
            .is_some_and(|s| s.ip == ip && s.expires_epoch > (self.wall)())
    }

    // --- akış
    /// Lisans kapalı / cihaz bağlı değil: misafir giremez (yeni kod gönderilmez).
    fn hizmet_kapali(&self, d: Dil) -> Option<Page> {
        (!crate::merkez::hizmet_acik(&self.merkez_path, (self.wall)())).then(|| self.error_page(dil::m(d, "hizmet_kapali"), d))
    }

    #[cfg(test)]
    pub fn home(&self, ip: &str, mac: Option<&str>, dst: &str) -> Page {
        self.home_dil(ip, mac, dst, Dil::Tr, false)
    }

    /// `kod_sayfasi`: kod sayfasındaki dil bağlantısından gelindi; süresi dolmamış bekleyen kod varsa kod sayfası açılır.
    pub fn home_dil(&self, ip: &str, mac: Option<&str>, dst: &str, d: Dil, kod_sayfasi: bool) -> Page {
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        if self.session_for(ip, mac) || self.rehome(ip, mac) {
            return self.success_page(dst, d);
        }
        if kod_sayfasi {
            let now = (self.clock)();
            let p = lock(&self.pending).get(mac).filter(|p| now < p.expires).cloned();
            if let Some(p) = p {
                return self.code_page(&p, "", "", d);
            }
        }
        self.form_page_dil(&Form::new(), vec![], dst, d)
    }

    /// Oturumu süren cihaz DHCP'den yeni IP ile döndü: SMS istemeden oturumu yeni IP'ye taşır.
    fn rehome(&self, ip: &str, mac: &str) -> bool {
        let root = &self.cfg.main;
        let Ok(_g) = ortak::state_lock(&root.state_root) else { return false };
        let mut ses = ortak::load_sessions(&root.state_root);
        let now = (self.wall)();
        let movable = ses.get(mac).is_some_and(|s| s.expires_epoch > now && s.ip != ip);
        if !movable || !ortak::move_session(&root.log_root, &mut ses, mac, ip, now, &*self.runner) {
            return false;
        }
        ortak::save_sessions(&root.state_root, &ses).is_ok()
    }

    fn refusal(&self, pending: &HashMap<String, Pending>, mac: &str, phone: &str, now: f64, day: &str, d: Dil) -> Option<(&'static str, String)> {
        let l = &self.cfg.limits;
        let cooldown = l.resend_cooldown_sec as f64;
        if let Some(p) = pending.get(mac) {
            if now - p.sent_at < cooldown {
                let wait = (cooldown - (now - p.sent_at)) as i64 + 1;
                return Some(("bekleme", dil::f(d, "bekleme", &[&wait.to_string()])));
            }
        }
        let tel = format!("tel:{phone}");
        if self.rl.count(&tel, 900.0, now) >= l.sms_per_phone_15min {
            return Some(("tel_15dk", dil::m(d, "tel_15dk").into()));
        }
        if self.rl.count(&tel, 86400.0, now) >= l.sms_per_phone_day {
            return Some(("tel_gun", dil::m(d, "tel_gun").into()));
        }
        if self.rl.count(&format!("mac:{mac}"), 3600.0, now) >= l.sms_per_mac_hour {
            return Some(("mac_saat", dil::m(d, "mac_saat").into()));
        }
        if ortak::sms_count(&self.cfg.main.state_root, day) >= l.sms_global_day {
            return Some(("tavan", dil::m(d, "tavan").into()));
        }
        None
    }

    /// Formdaki `dil` alanı (HTTP katmanı seçilen dili yazar); yoksa Türkçe.
    fn form_dil(form: &Form) -> Dil {
        form.get("dil").and_then(|v| Dil::coz(v)).unwrap_or_default()
    }

    pub fn send_code(&self, ip: &str, mac: Option<&str>, form: &Form) -> Page {
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let dst = safe_dst(get("dst"));
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        let ulke = if get("ulke").is_empty() { "TR" } else { get("ulke") };
        let (ad, soyad, phone) = (ortak::clean_name(get("ad")), ortak::clean_name(get("soyad")), ulkeler::normalize_phone(ulke, get("telefon")));
        let mut errors = vec![];
        if ad.is_none() {
            errors.push(("hata_ad", dil::m(d, "hata_ad").to_string()));
        }
        if soyad.is_none() {
            errors.push(("hata_soyad", dil::m(d, "hata_soyad").to_string()));
        }
        if phone.is_none() {
            let k = if ulke == "TR" && !get("telefon").trim_start().starts_with('+') { "hata_telefon_tr" } else { "hata_telefon" };
            errors.push(("hata_telefon", dil::m(d, k).to_string()));
        }
        if get("sozlesme") != "1" {
            errors.push(("hata_sozlesme", dil::m(d, "hata_sozlesme").to_string()));
        }
        if get("riza") != "1" {
            errors.push(("hata_riza", dil::m(d, "hata_riza").to_string()));
        }
        let (Some(ad), Some(soyad), Some(phone), true) = (ad, soyad, phone, errors.is_empty()) else {
            return self.form_page_dil(form, errors, &dst, d);
        };
        let provider = match sms::route(&self.cfg, &phone) {
            Ok(p) => p,
            Err(msg) => {
                self.audit("OTP_ISTEK", ip, mac, &phone, "red=yabanci_numara");
                return self.form_page_dil(form, vec![("hata_genel", dil::cevir(d, msg))], &dst, d);
            }
        };
        let now = (self.clock)();
        let day = ortak::day_of(&ortak::now_iso((self.wall)())).to_string();
        let cooldown = self.cfg.limits.resend_cooldown_sec as f64;
        let code = {
            let pending = lock(&self.pending);
            if let Some(p) = pending.get(mac) {
                if p.phone == phone && now - p.sent_at < cooldown && now < p.expires {
                    let wait = (cooldown - (now - p.sent_at)) as i64 + 1;
                    return self.code_page(p, &dil::f(d, "kod_az_once", &[&wait.to_string()]), "", d);
                }
            }
            if let Some((kind, msg)) = self.refusal(&pending, mac, &phone, now, &day, d) {
                drop(pending);
                let olay = if kind == "tavan" { "SMS_TAVAN" } else { "OTP_ISTEK" };
                self.audit(olay, ip, mac, &phone, &format!("red={kind}"));
                return self.form_page_dil(form, vec![("hata_genel", msg)], &dst, d);
            }
            self.rl.hit(&format!("tel:{phone}"), now);
            self.rl.hit(&format!("mac:{mac}"), now);
            ortak::sms_count_inc(&self.cfg.main.state_root, &day);
            otp_code()
        };
        let r = (self.send_sms)(&self.cfg, provider, &phone, &code); // ağ çağrısı kilit dışında
        if !r.ok {
            self.audit("OTP_HATA", ip, mac, &phone, &format!("{}={}", provider.ad(), r.kod));
            return self.form_page_dil(form, vec![("hata_genel", dil::cevir(d, sms::message_for(provider, &r.kod)))], &dst, d);
        }
        let p = Pending {
            phone: phone.clone(),
            ad,
            soyad,
            code,
            dst,
            expires: now + self.cfg.limits.otp_ttl_sec as f64,
            attempts: 0,
            sent_at: now,
            provider,
            riza: get("riza") == "1",
            nonce: self.nonce.fetch_add(1, Ordering::Relaxed),
        };
        let page = self.code_page(&p, "", "", d);
        lock(&self.pending).insert(mac.to_string(), p);
        let job = r.is.as_deref().unwrap_or("None");
        self.audit("OTP_GONDERILDI", ip, mac, &phone, &format!("{}={job}", provider.ad()));
        page
    }

    #[cfg(test)]
    pub fn resend(&self, ip: &str, mac: Option<&str>) -> Page {
        self.resend_dil(ip, mac, Dil::Tr)
    }

    pub fn resend_dil(&self, ip: &str, mac: Option<&str>, d: Dil) -> Page {
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let p = mac.and_then(|m| lock(&self.pending).get(m).cloned());
        let Some(p) = p else {
            return self.form_page_dil(&Form::new(), vec![("hata_genel", dil::m(d, "kod_yok_bilgi").into())], "", d);
        };
        self.send_code(ip, mac, &Self::prefill(&p, d))
    }

    pub fn verify(&self, ip: &str, mac: Option<&str>, form: &Form) -> Page {
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        let now = (self.clock)();
        let fail_lock = self.cfg.limits.verify_fail_lock_min * 60;
        let fail_key = format!("hata:{mac}");
        if self.rl.count(&fail_key, fail_lock as f64, now) >= 10 {
            return self.error_page(&dil::f(d, "kilit", &[&(fail_lock / 60).to_string()]), d);
        }
        let not_found = || self.form_page_dil(&Form::new(), vec![("hata_genel", dil::m(d, "kod_yok").into())], "", d);
        let code: String = form.get("kod").map(|k| k.chars().filter(char::is_ascii_digit).collect()).unwrap_or_default();
        // 1) bekleyen kod
        let p = {
            let mut pending = lock(&self.pending);
            let Some(p) = pending.get(mac).cloned() else { return not_found() };
            if now >= p.expires {
                pending.remove(mac);
                return self.form_page_dil(&Self::prefill(&p, d), vec![("hata_genel", dil::m(d, "kod_suresi").into())], &p.dst, d);
            }
            p
        };
        // 2) karşılaştır: Twilio gerçek modda kodu Twilio doğrular (ağ çağrısı kilit dışında); diğerlerinde yerelde
        let matched = if p.provider == Saglayici::Twilio && !self.cfg.sms.mock {
            let (ok, err) = (self.check_sms)(&self.cfg, &p.phone, &code);
            if err.is_some() {
                return self.code_page(&p, "", dil::m(d, "dogrulanamadi"), d);
            }
            ok
        } else {
            ct_eq(&code, &p.code)
        };
        // 3) sonucu uygula — bu arada kod yenilendiyse eski sonuç uygulanmaz
        let wrong_page = {
            let mut pending = lock(&self.pending);
            let Some(cur) = pending.get_mut(mac).filter(|c| c.nonce == p.nonce) else { return not_found() };
            if matched {
                pending.remove(mac);
                None
            } else {
                cur.attempts += 1;
                self.rl.hit(&fail_key, now);
                let left = self.cfg.limits.otp_max_attempts.saturating_sub(cur.attempts);
                Some(if left == 0 {
                    pending.remove(mac);
                    self.form_page_dil(&Self::prefill(&p, d), vec![("hata_genel", dil::m(d, "cok_hatali").into())], &p.dst, d)
                } else {
                    self.code_page(&p, "", &dil::f(d, "kod_hatali", &[&left.to_string()]), d)
                })
            }
        };
        if let Some(page) = wrong_page {
            self.audit("OTP_HATALI_KOD", ip, mac, &p.phone, "");
            return page;
        }
        let g = Giris { kisi: &p.phone, ad: &p.ad, soyad: &p.soyad, bitis: None, ek: format!("sozlesme=1 riza={}", u8::from(p.riza)) };
        let Some(s) = self.open_session(ip, mac, &g) else {
            return self.error_page(dil::m(d, "baglanti_acilamadi"), d);
        };
        self.audit("OTP_BASARILI", ip, mac, &p.phone, &format!("oturum={}", s.session_id));
        self.success_page(&p.dst, d)
    }

    // --- misafir kodu (otel / yurt)
    /// Giriş sayfasındaki ikinci yol; misafir kodu kapalıysa (ya da tür otel/yurt değilse) boş.
    fn misafir_dugmesi(&self, dst: &str, d: Dil) -> String {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return String::new();
        }
        let dst = if dst.is_empty() { String::new() } else { format!("&amp;dst={}", pct_encode(dst)) };
        format!("<a class=\"dugme\" href=\"{}/misafir?dil={}{dst}\">{}</a>", html_escape(&self.base_url), d.kod(), html_escape(self.misafir_adi(d)))
    }

    /// Düğme / sayfa başlığı: Otel "Otel misafiriyim", Yurt "Yurt sakiniyim".
    fn misafir_adi(&self, d: Dil) -> &'static str {
        dil::m(d, if crate::ayar::isletme_turu(&self.cfg) == "yurt" { "misafir_dugme_yurt" } else { "misafir_dugme_otel" })
    }

    fn misafir_form(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str, d: Dil) -> Page {
        let f = |k: &str| form.get(k).cloned().unwrap_or_default();
        let pm = &self.cfg.portal;
        let tur = f("tur");
        let uyruk = f("uyruk");
        let secili = |a: &str| if tur == a || (a == "tc" && tur.is_empty()) { "checked".to_string() } else { String::new() };
        let uyruklar: String = std::iter::once(format!("<option value=\"\">{}</option>", dil::m(d, "uyruk_secin")))
            .chain(ulkeler::ULKELER.iter().map(|(iso, _, ad)| {
                let sel = if *iso == uyruk { " selected" } else { "" };
                format!("<option value=\"{iso}\"{sel}>{} {iso} · {}</option>", ulkeler::flag(iso), html_escape(ad))
            }))
            .collect();
        let mut ctx = vec![
            ("baslik", self.misafir_adi(d).to_string()),
            ("tur_tc", secili("tc")),
            ("tur_pasaport", secili("pasaport")),
            ("tc", f("tc")),
            ("pasaport", f("pasaport")),
            ("uyruk_secenekleri_html", uyruklar),
            ("sozlesme_checked", if f("sozlesme") == "1" { "checked".into() } else { String::new() }),
            ("riza_checked", if f("riza") == "1" { "checked".into() } else { String::new() }),
            ("riza_metni_html", html_escape(&self.hukuki(d, &pm.acik_riza, &pm.acik_riza_en)).replace('\n', "<br>")),
            ("sozlesme_html", metin_html_dil(&self.hukuki(d, &pm.sozlesme, &pm.sozlesme_en), d)),
            ("aydinlatma_html", metin_html_dil(&self.hukuki(d, &pm.aydinlatma, &pm.aydinlatma_en), d)),
            ("dst", dst.to_string()),
        ];
        for k in ["hata_genel", "hata_tur", "hata_tc", "hata_pasaport", "hata_uyruk", "hata_kod", "hata_riza", "hata_sozlesme"] {
            let v = errors.iter().find(|(e, _)| *e == k).map(|(_, v)| v.clone()).unwrap_or_default();
            ctx.push((k, v));
        }
        Page::new("misafir", ctx, d)
    }

    /// GET /misafir. `None`: misafir kodu bu işletmede yok (HTTP katmanı ana sayfaya yönlendirir).
    pub fn misafir_sayfasi(&self, ip: &str, mac: Option<&str>, dst: &str, d: Dil) -> Option<Page> {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return None;
        }
        if let Some(pg) = self.hizmet_kapali(d) {
            return Some(pg);
        }
        let Some(mac) = mac else { return Some(self.error_page(dil::m(d, "mac_yok"), d)) };
        if self.session_for(ip, mac) || self.rehome(ip, mac) {
            return Some(self.success_page(dst, d));
        }
        Some(self.misafir_form(&Form::new(), vec![], dst, d))
    }

    /// POST /misafir/dogrula: kimlik + kod eşleşirse kodun bitişine kadar internet. Kimlik yok / kod yanlış / süresi
    /// dolmuş / iptal: tek ileti, IP ve MAC başına hata sayılır (5 → 15 dk; kilitliyken doğru kod da reddedilir).
    /// Denetime kimlik ve kod yazılmaz (başarıda yalnızca maskeli kimlik).
    pub fn misafir_dogrula(&self, ip: &str, mac: Option<&str>, form: &Form) -> Option<Page> {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return None;
        }
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return Some(pg);
        }
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let dst = safe_dst(get("dst"));
        let Some(mac) = mac else { return Some(self.error_page(dil::m(d, "mac_yok"), d)) };
        // girilen kod forma geri yazılmaz
        let mut geri = form.clone();
        geri.remove("kod");
        let now = (self.clock)();
        let (ip_key, mac_key) = (format!("misafir_ip:{ip}"), format!("misafir_mac:{mac}"));
        let kilitli = self.rl.count(&ip_key, MISAFIR_KILIT_SN, now) >= MISAFIR_HATA_SINIRI
            || self.rl.count(&mac_key, MISAFIR_KILIT_SN, now) >= MISAFIR_HATA_SINIRI;
        if kilitli {
            self.audit("MISAFIR_HATALI", ip, mac, "", "red=kilit");
            let msg = dil::f(d, "kilit", &[&((MISAFIR_KILIT_SN / 60.0) as u64).to_string()]);
            return Some(self.misafir_form(&geri, vec![("hata_genel", msg)], &dst, d));
        }
        // biçim hataları (kodun var olup olmadığını söylemez, hata sayılmaz)
        let mut errors = vec![];
        let anahtar = match get("tur") {
            "tc" => {
                let t: String = get("tc").chars().filter(|c| !c.is_whitespace()).collect();
                if kimlik::tc_gecerli(&t) {
                    Some(kimlik::Kimlik::Tc(t).anahtar())
                } else {
                    errors.push(("hata_tc", dil::m(d, "hata_tc").to_string()));
                    None
                }
            }
            "pasaport" => {
                let no = kimlik::pasaport_normal(get("pasaport"));
                let uyruk = get("uyruk");
                let uyruk_ok = ulkeler::ULKELER.iter().any(|(i, ..)| *i == uyruk);
                if no.is_none() {
                    errors.push(("hata_pasaport", dil::m(d, "hata_pasaport").to_string()));
                }
                if !uyruk_ok {
                    errors.push(("hata_uyruk", dil::m(d, "hata_uyruk").to_string()));
                }
                no.filter(|_| uyruk_ok).map(|no| kimlik::Kimlik::Pasaport { uyruk: uyruk.to_string(), no }.anahtar())
            }
            _ => {
                errors.push(("hata_tur", dil::m(d, "hata_kimlik_turu").to_string()));
                None
            }
        };
        let kod: String = get("kod").chars().filter(|c| !c.is_whitespace()).collect();
        if kod.len() != 6 || !kod.bytes().all(|b| b.is_ascii_digit()) {
            errors.push(("hata_kod", dil::m(d, "hata_misafir_kod").to_string()));
        }
        if get("riza") != "1" {
            errors.push(("hata_riza", dil::m(d, "hata_riza").to_string()));
        }
        if get("sozlesme") != "1" {
            errors.push(("hata_sozlesme", dil::m(d, "hata_sozlesme").to_string()));
        }
        let (Some(anahtar), true) = (anahtar, errors.is_empty()) else {
            return Some(self.misafir_form(&geri, errors, &dst, d));
        };
        let wall = (self.wall)();
        let kodlar = misafir::oku(&self.cfg.main.state_root);
        let Some(k) = misafir::dogrula(&kodlar, &anahtar, &kod, wall).cloned() else {
            self.rl.hit(&ip_key, now);
            self.rl.hit(&mac_key, now);
            self.audit("MISAFIR_HATALI", ip, mac, "", "");
            return Some(self.misafir_form(&geri, vec![("hata_genel", dil::m(d, "misafir_eslesmedi").into())], &dst, d));
        };
        let ek = format!(
            "sozlesme=1 riza=1 giris=misafir_kodu oda={} kod_bitis={} ureten={}",
            k.oda,
            ortak::now_iso(k.bitis),
            k.ureten.replace(char::is_whitespace, "_")
        );
        let g = Giris { kisi: &k.kimlik, ad: &k.ad, soyad: &k.soyad, bitis: Some(k.bitis), ek };
        let Some(s) = self.open_session(ip, mac, &g) else {
            return Some(self.error_page(dil::m(d, "baglanti_acilamadi"), d));
        };
        self.audit("MISAFIR_BASARILI", ip, mac, &kimlik::maskele(&k.kimlik), &format!("oturum={}", s.session_id));
        Some(self.success_page(&dst, d))
    }

    /// §11.7 sırası. nft başarısızsa yeni oturum yazılmaz. SMS ve misafir kodu aynı adımları kullanır; cihaz sınırı
    /// kişi anahtarı (telefon ya da `TC:…`/`PASAPORT:…`) başına.
    fn open_session(&self, ip: &str, mac: &str, g: &Giris) -> Option<Session> {
        let m = &self.cfg.main;
        let runner = &*self.runner;
        let _g = ortak::state_lock(&m.state_root).ok()?;
        let mut ses = ortak::load_sessions(&m.state_root);
        let now = (self.wall)();
        let secs = g.bitis.map_or((m.session_minutes * 60) as f64, |b| b - now);
        if secs < 1.0 {
            return None; // kodun süresi bu arada doldu
        }
        if ses.contains_key(mac) {
            ortak::close_session(&m.log_root, &mut ses, mac, "yeniden_giris", now, runner);
        }
        let mut same: Vec<(f64, String)> =
            ses.iter().filter(|(_, s)| s.phone == g.kisi).map(|(k, s)| (s.start_epoch, k.clone())).collect();
        same.sort_by(|a, b| a.0.total_cmp(&b.0));
        while !same.is_empty() && same.len() >= m.max_devices_per_phone {
            let (_, old) = same.remove(0);
            ortak::close_session(&m.log_root, &mut ses, &old, "cihaz_limiti", now, runner);
        }
        if !ortak::nft_add_cmd(mac, ip, secs).is_some_and(|c| runner(&c)) {
            let _ = ortak::save_sessions(&m.state_root, &ses);
            return None;
        }
        ortak::park_ip_holders(&m.log_root, &mut ses, mac, ip, now, runner);
        let zaman = ortak::now_iso(now);
        let s = Session {
            phone: g.kisi.into(),
            ad: g.ad.into(),
            soyad: g.soyad.into(),
            ip: ip.into(),
            session_id: ortak::random_hex(6),
            start: zaman.clone(),
            start_epoch: now,
            expires_epoch: now + secs,
        };
        ses.insert(mac.into(), s.clone());
        if let Err(e) = ortak::save_sessions(&m.state_root, &ses) {
            eprintln!("oturum dosyası yazılamadı: {e}");
        }
        let basla = Row::new("OTURUM_BASLA", &zaman)
            .set("telefon", &s.phone)
            .set("ad", &s.ad)
            .set("soyad", &s.soyad)
            .set("mac", mac)
            .set("ic_ip", ip)
            .set("oturum_id", &s.session_id)
            .set("ek", format!("bitis={} {}", ortak::now_iso(now + secs), g.ek));
        let day = ortak::day_of(&zaman).to_string();
        let written = ortak::append_user_rows(&m.log_root, &s, mac, &[basla.clone()])
            .and_then(|_| ortak::append_rows(&ortak::day_file(&m.log_root, &day, "oturum.csv"), &[basla]))
            .and_then(|_| ortak::upsert_index(&m.log_root, &s.phone, &s.ad, &s.soyad, &zaman));
        if let Err(e) = written {
            eprintln!("oturum kaydı yazılamadı: {e}");
        }
        Some(s)
    }
}

// ---------------------------------------------------------------- HTTP
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` → ilk değer kazanır; boş değerler atlanır (Python parse_qs gibi).
pub fn parse_query(s: &str) -> Form {
    let mut out = Form::new();
    for pair in s.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (pct_decode(k), pct_decode(v));
        if !k.is_empty() && !v.is_empty() {
            out.entry(k).or_insert(v);
        }
    }
    out
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn respond(req: tiny_http::Request, code: u16, body: String, ctype: &str, extra: Vec<(&'static str, String)>, base_url: &str) {
    let csp = format!("default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action {base_url}");
    let mut headers = vec![
        ("Content-Type", ctype.to_string()),
        ("Cache-Control", "no-store".into()),
        ("X-Frame-Options", "DENY".into()),
        ("Content-Security-Policy", csp),
        ("Server", "hotspot".into()),
    ];
    headers.extend(extra);
    let mut resp = tiny_http::Response::from_string(body).with_status_code(code);
    for (k, v) in headers {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(h);
        }
    }
    let _ = req.respond(resp);
}

fn baslik(req: &tiny_http::Request, ad: &'static str) -> String {
    req.headers().iter().find(|h| h.field.equiv(ad)).map(|h| h.value.as_str().to_string()).unwrap_or_default()
}

fn handle(p: &Portal, per_ip: &RateLimiter, mut req: tiny_http::Request) {
    let base = p.base_url.clone();
    let text = |req, code, msg: &str| respond(req, code, msg.to_string(), "text/plain; charset=utf-8", vec![], &base);
    let git = |req, to: String| respond(req, 302, String::new(), "text/html; charset=utf-8", vec![("Location", to)], &base);
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    if !ortak::in_subnet(&ip, &p.cfg.main.subnet) {
        return text(req, 403, "Yasak");
    }
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let q = parse_query(query);
    // dil: sorgu (GET) ya da form alanı (POST) > çerez > Accept-Language > Türkçe
    let (cerez, al) = (baslik(&req, "Cookie"), baslik(&req, "Accept-Language"));
    let dil_sec = |acik: Option<&str>| dil::sec(acik, dil::cerezden(&cerez), Some(al.as_str()).filter(|a| !a.is_empty()));
    let d = dil_sec(q.get("dil").map(String::as_str));
    let now = ortak::monotonic();
    if per_ip.count(&ip, 60.0, now) >= 30 {
        return text(req, 429, dil::m(d, "cok_istek"));
    }
    per_ip.hit(&ip, now);
    let host = baslik(&req, "Host");
    if host != p.host {
        // nft'nin 80 → 8080 yönlendirdiği yabancı istek: portala gönder, gitmek istediği adresi hatırla
        let dst = if host.is_empty() { String::new() } else { safe_dst(&format!("http://{host}{url}")) };
        let q = if dst.is_empty() { String::new() } else { format!("?dst={}", pct_encode(&dst)) };
        return git(req, format!("{base}/{q}"));
    }
    let mac = ortak::lookup_mac(&ip, &p.cfg.main.leases_file);
    let mac = mac.as_deref();
    let method = req.method().clone();
    let mut extra = vec![];
    let page = match method {
        tiny_http::Method::Get | tiny_http::Method::Head => match path {
            "/" => {
                if q.get("dil").and_then(|v| dil::Dil::coz(v)).is_some() {
                    extra.push(("Set-Cookie", dil::cerez_basligi(d))); // seçim hatırlansın
                }
                let dst = safe_dst(q.get("dst").map(String::as_str).unwrap_or(""));
                p.home_dil(&ip, mac, &dst, d, q.get("sayfa").is_some_and(|s| s == "kod"))
            }
            "/misafir" => {
                if q.get("dil").and_then(|v| dil::Dil::coz(v)).is_some() {
                    extra.push(("Set-Cookie", dil::cerez_basligi(d)));
                }
                let dst = safe_dst(q.get("dst").map(String::as_str).unwrap_or(""));
                match p.misafir_sayfasi(&ip, mac, &dst, d) {
                    Some(pg) => pg,
                    None => return git(req, format!("{base}/")),
                }
            }
            _ => return git(req, format!("{base}/")),
        },
        tiny_http::Method::Post => {
            if req.body_length().unwrap_or(0) > 4096 {
                return text(req, 413, dil::m(d, "istek_buyuk"));
            }
            let mut raw = String::new();
            if req.as_reader().take(4096).read_to_string(&mut raw).is_err() {
                return text(req, 400, dil::m(d, "istek_gecersiz"));
            }
            let mut form = parse_query(&raw);
            let d = dil_sec(form.get("dil").map(String::as_str));
            form.insert("dil".into(), d.kod().into());
            match path {
                "/kod-gonder" => p.send_code(&ip, mac, &form),
                "/dogrula" => p.verify(&ip, mac, &form),
                "/tekrar-gonder" => p.resend_dil(&ip, mac, d),
                "/misafir/dogrula" => match p.misafir_dogrula(&ip, mac, &form) {
                    Some(pg) => pg,
                    None => return git(req, format!("{base}/")),
                },
                _ => return git(req, format!("{base}/")),
            }
        }
        _ => return text(req, 405, dil::m(d, "istek_desteklenmez")),
    };
    let html = p.render(&page);
    respond(req, 200, html, "text/html; charset=utf-8", extra, &base);
}

pub fn run(cfg: Config) -> ExitCode {
    let missing = cfg.sms_missing();
    if !missing.is_empty() {
        eprintln!("portal: SMS ayarları eksik: {}", missing.join(", "));
        return ExitCode::from(1);
    }
    let addr = format!("{}:{}", cfg.main.router_ip, cfg.main.portal_port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("portal: {addr} dinlenemedi: {e}");
            return ExitCode::from(1);
        }
    };
    let portal = Arc::new(Portal::new(
        cfg,
        Box::new(ortak::monotonic),
        Box::new(ortak::wall),
        Box::new(|c: &[String]| ortak::run(c)),
        Box::new(sms::send),
        Box::new(crate::twilio::check),
    ));
    portal.audit("SERVIS_BASLADI", "", "", "", "portal");
    eprintln!("portal: {addr} dinleniyor (SMS deneme modu: {})", if portal.cfg.sms.mock { "açık" } else { "kapalı" });
    let per_ip = Arc::new(RateLimiter::default());
    // ponytail: 8 işçi + gövde ≤ 4 KB; yavaş istemci bir işçiyi en çok gövde okuma süresince tutar. Kafe ölçeğinde yeterli.
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let (server, portal, per_ip) = (server.clone(), portal.clone(), per_ip.clone());
            std::thread::spawn(move || loop {
                match server.recv() {
                    Ok(req) => {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&portal, &per_ip, req)));
                        if r.is_err() {
                            eprintln!("portal: istek işlenirken beklenmeyen hata");
                        }
                    }
                    Err(e) => {
                        eprintln!("portal: bağlantı hatası: {e}");
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

    struct T {
        p: Portal,
        mono: Arc<Mutex<f64>>,
        wall: Arc<Mutex<f64>>,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        sms: Arc<Mutex<Vec<(String, String, &'static str)>>>,
        root: std::path::PathBuf,
    }

    const MAC: &str = "aa:bb:cc:dd:ee:01";
    const IP: &str = "10.50.0.23";
    const WALL0: f64 = 1_790_705_134.0; // 2026-09-29 21:05:34 +03

    fn setup(tweak: impl FnOnce(&mut Config)) -> T {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-portal-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        let mut cfg = Config::default();
        cfg.main.site_name = "Bocafe".into();
        cfg.main.log_root = root.join("5651").to_string_lossy().into();
        cfg.main.state_root = root.join("state").to_string_lossy().into();
        tweak(&mut cfg);
        let (mono, wall) = (Arc::new(Mutex::new(100.0)), Arc::new(Mutex::new(WALL0)));
        let calls = Arc::new(Mutex::new(vec![]));
        let sms_log = Arc::new(Mutex::new(vec![]));
        let (m2, w2, c2, s2) = (mono.clone(), wall.clone(), calls.clone(), sms_log.clone());
        let p = Portal::new(
            cfg,
            Box::new(move || *m2.lock().unwrap()),
            Box::new(move || *w2.lock().unwrap()),
            Box::new(move |c: &[String]| {
                c2.lock().unwrap().push(c.to_vec());
                !c.iter().any(|a| a.contains("10.50.0.99")) // .99'a nft ekleme başarısız sayılır
            }),
            Box::new(move |_c: &Config, prov: Saglayici, phone: &str, code: &str| {
                s2.lock().unwrap().push((phone.into(), code.into(), prov.ad()));
                Sonuc { ok: true, kod: "0".into(), is: Some("J1".into()) }
            }),
            // Twilio uzaktan doğrulama: 424242 onaylı, 999999 ağ hatası
            Box::new(|_c: &Config, _phone: &str, code: &str| match code {
                "424242" => (true, None),
                "999999" => (false, Some("AG".into())),
                _ => (false, None),
            }),
        );
        let mut p = p;
        p.merkez_path = root.join("merkez.json");
        std::fs::create_dir_all(&root).unwrap();
        crate::merkez::kaydet(&p.merkez_path, &crate::merkez::Merkez { numara: "4511643".into(), ..Default::default() }).unwrap();
        T { p, mono, wall, calls, sms: sms_log, root }
    }

    impl T {
        fn advance(&self, secs: f64) {
            *self.mono.lock().unwrap() += secs;
            *self.wall.lock().unwrap() += secs;
        }
        fn last_code(&self) -> String {
            self.sms.lock().unwrap().last().unwrap().1.clone()
        }
        fn sms_count(&self) -> usize {
            self.sms.lock().unwrap().len()
        }
        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.root.join(rel)).unwrap_or_default()
        }
        fn audit_text(&self) -> String {
            self.read("5651/gunluk/2026-09-29/denetim.csv")
        }
    }

    fn form(pairs: &[(&str, &str)]) -> Form {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn good(phone: &str) -> Form {
        form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", phone), ("sozlesme", "1"), ("riza", "1"), ("dst", "http://neverssl.com/")])
    }

    #[test]
    fn form_errors_send_nothing() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "A"), ("soyad", "Ali1"), ("telefon", "212 455 31 32")]));
        assert_eq!(pg.tpl, "giris");
        for k in ["hata_ad", "hata_soyad", "hata_telefon", "hata_sozlesme"] {
            assert!(!pg.get(k).is_empty(), "{k}");
        }
        assert_eq!(pg.get("telefon"), "212 455 31 32"); // girilen değer korunur
        assert_eq!(t.sms_count(), 0);
        assert_eq!(t.p.send_code(IP, None, &good("5334553132")).tpl, "hata"); // MAC bilinmiyor
    }

    #[test]
    fn full_flow_opens_session_and_writes_records() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &good("0533 455 31 32"));
        assert_eq!(pg.tpl, "kod");
        assert_eq!(pg.get("telefon_maskeli"), "+90 5XX XXX XX 32");
        assert_eq!(t.sms.lock().unwrap()[0].0, "905334553132");
        assert_eq!(t.sms.lock().unwrap()[0].2, "netgsm");
        let code = t.last_code();
        assert_eq!(code.len(), 6);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)]));
        assert_eq!((pg.tpl, pg.get("hata")), ("kod", "Kod hatalı. Kalan deneme hakkı: 4"));
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", &format!(" {code} "))]));
        assert_eq!(pg.tpl, "basarili");
        assert!(pg.get("bas_html").contains("url=http://neverssl.com/"));
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout 2592000s }}")));
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses[MAC].ip, IP);
        assert_eq!(ses[MAC].phone, "905334553132");
        let user = t.read("5651/gunluk/2026-09-29/kullanicilar/905334553132.csv");
        assert!(user.starts_with('\u{feff}'));
        assert!(user.contains(";KAYIT;905334553132;Ayşe;Yılmaz;aa:bb:cc:dd:ee:01;10.50.0.23;"));
        assert!(user.contains(";OTURUM_BASLA;"));
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("OTURUM_BASLA"));
        assert!(t.read("5651/kullanicilar/index.csv").contains("905334553132;Ayşe;Yılmaz;2026-09-29T21:05:34+03:00"));
        let a = t.audit_text();
        for olay in ["OTP_GONDERILDI", "OTP_HATALI_KOD", "OTP_BASARILI"] {
            assert!(a.contains(olay), "{olay}");
        }
        assert!(!a.contains(&code), "kod kayda yazılmaz");
        assert_eq!(t.p.home(IP, Some(MAC), "").tpl, "basarili"); // zaten bağlı
    }

    #[test]
    fn double_submit_does_not_resend() {
        let t = setup(|_| {});
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        t.advance(5.0);
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert_eq!(pg.tpl, "kod");
        assert!(pg.get("not").starts_with("Kod az önce gönderildi"));
        assert_eq!(t.sms_count(), 1);
        assert!(t.p.resend(IP, Some(MAC)).get("not").starts_with("Kod az önce gönderildi"));
        t.advance(61.0);
        assert_eq!(t.p.resend(IP, Some(MAC)).tpl, "kod");
        assert_eq!(t.sms_count(), 2);
    }

    #[test]
    fn five_wrong_codes_cancel_and_expiry() {
        let t = setup(|_| {});
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        let wrong = if t.last_code() == "000000" { "111111" } else { "000000" };
        for _ in 0..4 {
            assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)])).tpl, "kod");
        }
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)]));
        assert_eq!((pg.tpl, pg.get("hata_genel")), ("giris", "Çok fazla hatalı deneme. Yeni kod isteyin."));
        assert_eq!(pg.get("telefon"), "+905334553132");
        assert!(t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)])).get("hata_genel").contains("Kod bulunamadı"));

        t.advance(61.0);
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        let code = t.last_code();
        t.advance(181.0);
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", &code)]));
        assert_eq!(pg.get("hata_genel"), "Kodun süresi doldu. Yeni kod isteyin.");
    }

    #[test]
    fn rate_limits_and_global_cap() {
        let t = setup(|_| {});
        for i in 0..3 {
            assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod", "gönderim {i}");
            t.advance(61.0);
        }
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert!(pg.get("hata_genel").contains("15 dakika"));
        assert!(t.audit_text().contains("red=tel_15dk"));
        assert_eq!(t.sms_count(), 3);

        let t = setup(|c| c.limits.sms_global_day = 1);
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        let pg = t.p.send_code("10.50.0.24", Some("aa:bb:cc:dd:ee:02"), &good("5321112233"));
        assert_eq!(pg.get("hata_genel"), "Şu an SMS gönderilemiyor, lütfen personele başvurun.");
        assert!(t.audit_text().contains("SMS_TAVAN"));
    }

    fn login(t: &T, mac: &str, ip: &str, phone: &str) -> Page {
        t.p.send_code(ip, Some(mac), &good(phone));
        let code = t.last_code();
        let pg = t.p.verify(ip, Some(mac), &form(&[("kod", &code)]));
        t.advance(61.0);
        pg
    }

    #[test]
    fn ip_change_moves_session_without_sms() {
        let t = setup(|_| {});
        login(&t, MAC, IP, "5334553132");
        let before = t.sms_count();
        assert_eq!(t.p.home("10.50.0.50", Some(MAC), "").tpl, "basarili");
        assert_eq!(t.sms_count(), before);
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c[1] == "add" && c.last().unwrap().contains("10.50.0.50")));
        assert!(calls.iter().any(|c| c[1] == "delete" && c.last().unwrap().contains(IP)));
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].ip, "10.50.0.50");
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("OTURUM_IP_DEGISTI"));
    }

    #[test]
    fn device_limit_closes_oldest() {
        let t = setup(|c| c.main.max_devices_per_phone = 2);
        login(&t, "aa:bb:cc:dd:ee:01", "10.50.0.21", "5334553132");
        login(&t, "aa:bb:cc:dd:ee:02", "10.50.0.22", "5334553132");
        login(&t, "aa:bb:cc:dd:ee:03", "10.50.0.23", "5334553132");
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses.keys().cloned().collect::<Vec<_>>(), vec!["aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:03"]);
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("neden=cihaz_limiti"));
    }

    #[test]
    fn same_ip_holder_is_parked() {
        let t = setup(|_| {});
        login(&t, "aa:bb:cc:dd:ee:01", IP, "5334553132");
        login(&t, "aa:bb:cc:dd:ee:02", IP, "5321112233"); // DHCP aynı IP'yi yeni cihaza verdi
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses["aa:bb:cc:dd:ee:01"].ip, "");
        assert_eq!(ses["aa:bb:cc:dd:ee:02"].ip, IP);
    }

    #[test]
    fn nft_failure_writes_no_session() {
        let t = setup(|_| {});
        let pg = login(&t, MAC, "10.50.0.99", "5334553132");
        assert_eq!((pg.tpl, pg.get("mesaj")), ("hata", "Bağlantı açılamadı, lütfen tekrar deneyin."));
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
    }

    #[test]
    fn rendering_escapes_and_safe_redirects() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "<script>x</script>"), ("dst", "javascript:alert(1)")]));
        let html = t.p.render(&pg);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>x"));
        assert!(html.contains("<h1>Bocafe&#x27;ye hoş geldiniz</h1>")); // ek ada göre, kesme işaretiyle
        assert!(html.contains("name=\"dst\" value=\"\""));
        assert_eq!(safe_dst("https://a.com/x"), "https://a.com/x");
        assert_eq!(safe_dst(&format!("http://{}", "a".repeat(2050))), "");
        assert_eq!(parse_query("a=1&a=2&b=&c=%C5%9F+x&d=%zz"), form(&[("a", "1"), ("c", "ş x"), ("d", "%zz")]));
        assert_eq!(pct_encode("http://a.com/?q=1"), "http%3A%2F%2Fa.com%2F%3Fq%3D1");
        let mut v = HashMap::new();
        v.insert("site", "X".to_string());
        assert_eq!(substitute("$site'ye $bilinmeyen $$5", &v), "X'ye $bilinmeyen $5");
    }

    #[test]
    fn foreign_numbers_and_twilio() {
        // Twilio kapalı: yabancı numaraya SMS yok
        let t = setup(|_| {});
        let f = form(&[("ad", "John"), ("soyad", "Smith"), ("ulke", "DE"), ("telefon", "0151 2345 6789"), ("sozlesme", "1"), ("riza", "1")]);
        let pg = t.p.send_code(IP, Some(MAC), &f);
        assert_eq!(pg.get("hata_genel"), sms::YABANCI_YOK);
        assert!(t.p.render(&pg).contains("<option value=\"DE\" selected>")); // seçim korunur
        assert_eq!(t.sms_count(), 0);
        // Twilio açık, deneme modu: yabancı numara Twilio'dan, kod yerelde doğrulanır
        let t = setup(|c| c.twilio.enabled = true);
        let pg = t.p.send_code(IP, Some(MAC), &f);
        assert_eq!((pg.tpl, pg.get("telefon_maskeli")), ("kod", "+49 XXXXXXXXX89"));
        let code = t.last_code();
        assert_eq!(t.sms.lock().unwrap()[0], ("4915123456789".to_string(), code.clone(), "twilio"));
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", &code)])).tpl, "basarili");
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].phone, "4915123456789");
        // Türk numarası Twilio açıkken de NetGSM'den
        let t = setup(|c| c.twilio.enabled = true);
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert_eq!(t.sms.lock().unwrap()[0].2, "netgsm");
    }

    #[test]
    fn twilio_real_mode_checks_remotely() {
        let t = setup(|c| {
            c.sms.mock = false;
            c.sms.provider = "twilio".into();
        });
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        // ağ hatası: deneme hakkı düşmez
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", "999999")]));
        assert_eq!(pg.get("hata"), "Kod şu an doğrulanamadı, lütfen birkaç saniye sonra tekrar deneyin.");
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", "111111")]));
        assert_eq!(pg.get("hata"), "Kod hatalı. Kalan deneme hakkı: 4");
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", "424242")])).tpl, "basarili");
    }

    #[test]
    fn consent_texts_and_recording() {
        assert_eq!(metin_html(""), "<p class=\"not\">Metin henüz eklenmedi.</p>");
        assert_eq!(metin_html("Birinci <b>\r\nsatır\r\n\r\nİkinci"), "<p>Birinci &lt;b&gt;<br>satır</p><p>İkinci</p>");
        assert_eq!(metin_html("BAŞLIK\ngiriş:\n* bir\n* <iki>\nson"), "<p><strong>BAŞLIK</strong><br>giriş:</p><ul><li>bir</li><li>&lt;iki&gt;</li></ul><p>son</p>");
        let t = setup(|c| c.main.unvan = "Boca Gıda Tic. Ltd. Şti.".into());
        let page = t.p.render(&t.p.form_page(&Form::new(), vec![], ""));
        // açık rıza metni onay kutusunun yanında, unvan yerleşmiş, kutu zorunlu
        assert!(page.contains("name=\"riza\" value=\"1\"  required><span>Şirketinizin Wi-Fi internet hizmetini"));
        assert!(page.contains("ile paylaştığım kişisel verilerimin (ad, soyad, cep telefonu numarası) Boca Gıda Tic. Ltd. Şti. ile paylaşılmasına onay veriyorum."));
        assert!(!page.contains("İsteğe bağlı") && !page.contains("Metin henüz eklenmedi") && !page.contains("[vergi levhası unvanı]"));
        assert!(page.contains("<summary>KVKK Aydınlatma Metni</summary>") && page.contains("<li>Haberleşme ve İletişim Bilgileri: Telefon numarası</li>"));
        assert!(page.contains("<strong>SON KULLANICI LİSANS SÖZLEŞMESİ</strong>") && page.contains("Bocafe’nin uhdesindeki") && !page.contains("İŞLETMECİ"));
        assert!(page.contains("<strong>9. BOCAFE’NİN YETKİ VE İMKÂNLARI</strong>"));
        // unvan girilmemişse yer tutucu görünür (kurulum yapılmamış)
        let t0 = setup(|_| {});
        assert!(t0.p.render(&t0.p.form_page(&Form::new(), vec![], "")).contains("[vergi levhası unvanı]"));
        // rıza ya da sözleşme işaretlenmeden kod gönderilmez
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", "05334553132"), ("sozlesme", "1")]));
        assert!(!pg.get("hata_riza").is_empty() && t.sms_count() == 0);
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", "05334553132"), ("riza", "1")]));
        assert!(!pg.get("hata_sozlesme").is_empty() && t.sms_count() == 0);
        // ikisi de işaretli: giriş, kayda yazılır
        let pg = t.p.send_code(IP, Some(MAC), &good("05334553132"));
        assert_eq!(pg.tpl, "kod");
        let code = t.last_code();
        t.p.verify(IP, Some(MAC), &form(&[("kod", &code)]));
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("sozlesme=1 riza=1"));
    }

    #[test]
    fn business_name_with_turkish_suffixes() {
        let t = "İŞLETMECİ’ye, İŞLETMECİ’nin, İŞLETMECİ’den; İŞLETMECİ, 9. İŞLETMECİ’NİN";
        assert_eq!(isletme_yerlestir(t, "Bocafe"), "Bocafe’ye, Bocafe’nin, Bocafe’den; Bocafe, 9. BOCAFE’NİN");
        assert_eq!(isletme_yerlestir(t, "Starbucks"), "Starbucks’a, Starbucks’un, Starbucks’tan; Starbucks, 9. STARBUCKS’UN");
        assert_eq!(isletme_yerlestir(t, "Kahvecim"), "Kahvecim’e, Kahvecim’in, Kahvecim’den; Kahvecim, 9. KAHVECİM’İN");
        // ponytail: iyelik ekli adlarda doğrusu ’na/’ndan; yazımdan ayırt edilemez, genel kural uygulanır
        assert_eq!(isletme_yerlestir(t, "Kahve Dünyası"), "Kahve Dünyası’ya, Kahve Dünyası’nın, Kahve Dünyası’dan; Kahve Dünyası, 9. KAHVE DÜNYASI’NIN");
        assert_eq!(isletme_yerlestir("Göztepe Kafe’ye", "X"), "Göztepe Kafe’ye"); // başka kelimeye dokunmaz
        assert_eq!((ek("Hilton Otel", 'y'), ek("Starbucks", 'y'), ek("Kahvecim", 'y'), ek("Boca", 'y')), ("e".into(), "a".into(), "e".into(), "ya".into()));
    }

    #[test]
    fn lisans_kapaliyken_portal_internet_vermez() {
        let t = setup(|_| {});
        let mut m = crate::merkez::oku(&t.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&t.p.merkez_path, &m).unwrap();
        let pg = t.p.home(IP, Some(MAC), "");
        assert_eq!(pg.tpl, "hata");
        assert!(t.p.render(&pg).contains("internet hizmeti şu an kapalı"));
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "hata");
        assert_eq!(t.sms_count(), 0);
        crate::merkez::sil(&t.p.merkez_path); // bağlı değil: kapalı
        assert_eq!(t.p.home(IP, Some(MAC), "").tpl, "hata");
    }

    fn en(pairs: &[(&str, &str)]) -> Form {
        let mut f = form(pairs);
        f.insert("dil".into(), "en".into());
        f
    }

    #[test]
    fn en_formu_ve_hatalar_ingilizce() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &en(&[("ad", "A"), ("soyad", "Ali1"), ("telefon", "212 455 31 32")]));
        assert_eq!((pg.tpl, pg.dil), ("giris", Dil::En));
        assert_eq!(pg.get("hata_ad"), "Enter your first name using letters (2-40 characters).");
        assert_eq!(pg.get("hata_telefon"), "Enter a valid mobile number (5XX XXX XX XX).");
        assert_eq!(pg.get("hata_sozlesme"), "Accept the Internet User Agreement to continue.");
        let html = t.p.render(&pg);
        assert!(html.contains("<html lang=\"en\">") && html.contains("<h1>Welcome to Bocafe</h1>"));
        assert!(html.contains(">First name</label>") && html.contains(">Send code</button>") && html.contains("Bocafe · Guest Wi-Fi"));
        assert!(html.contains("I have read and accept the Internet User Agreement.") && html.contains("All rights reserved."));
        assert!(html.contains("<input type=\"hidden\" name=\"dil\" value=\"en\">"));
        assert!(!html.contains("hoş geldiniz") && !html.contains("Kod Gönder") && !html.contains("$m_"));
        // sağlayıcı iletisi de çevrilir
        let f = en(&[("ad", "John"), ("soyad", "Smith"), ("ulke", "DE"), ("telefon", "0151 2345 6789"), ("sozlesme", "1"), ("riza", "1")]);
        assert_eq!(t.p.send_code(IP, Some(MAC), &f).get("hata_genel"), "SMS cannot be sent to foreign numbers right now, please ask the staff.");
        // kod sayfası, tekrar gönderme, yanlış ve doğru kod
        let mut g = good("5334553132");
        g.insert("dil".into(), "en".into());
        let html = t.p.render(&t.p.send_code(IP, Some(MAC), &g));
        assert!(html.contains("We sent a 6-digit code to +90 5XX XXX XX 32. The code is valid for 3 minutes."));
        assert!(html.contains(">Resend code</button>") && html.contains(">Change number</a>") && !html.contains("Doğrula"));
        assert!(t.p.resend_dil(IP, Some(MAC), Dil::En).get("not").starts_with("A code was just sent. Please wait"));
        let code = t.last_code();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert_eq!(t.p.verify(IP, Some(MAC), &en(&[("kod", wrong)])).get("hata"), "Incorrect code. Attempts left: 4");
        let html = t.p.render(&t.p.verify(IP, Some(MAC), &en(&[("kod", &code)])));
        assert!(html.contains("<h1>You are connected</h1>") && html.contains("You are connected to Bocafe Wi-Fi") && html.contains("<b>Done</b>"));
        // hata sayfası
        let html = t.p.render(&t.p.home_dil(IP, None, "", Dil::En, false));
        assert!(html.contains("Your device could not be identified") && html.contains(">Start over</a>") && html.contains("Something went wrong"));
    }

    #[test]
    fn dil_baglantilari_dst_ve_kod_sayfasini_korur() {
        let t = setup(|_| {});
        let base = t.p.base_url.clone();
        let html = t.p.render(&t.p.home(IP, Some(MAC), "http://neverssl.com/a?b=1"));
        assert!(html.contains("<html lang=\"tr\">"));
        assert!(html.contains(&format!("href=\"{base}/?dil=en&amp;dst=http%3A%2F%2Fneverssl.com%2Fa%3Fb%3D1\"")));
        assert!(html.contains(&format!("href=\"{base}/?dil=tr&amp;dst=http%3A%2F%2Fneverssl.com%2Fa%3Fb%3D1\" lang=\"tr\" aria-current=\"true\">TR</a>")));
        assert!(html.contains("<input type=\"hidden\" name=\"dil\" value=\"tr\">"));
        // kod sayfasında dil değişince kod sayfası yeni dilde gelir; "Numarayı değiştir" (sayfa yok) formu açar
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert!(t.p.render(&pg).contains(&format!("href=\"{base}/?dil=en&amp;sayfa=kod\"")));
        let pg = t.p.home_dil(IP, Some(MAC), "", Dil::En, true);
        assert_eq!((pg.tpl, pg.dil), ("kod", Dil::En));
        assert_eq!(t.p.home_dil(IP, Some(MAC), "", Dil::En, false).tpl, "giris");
        t.advance(181.0); // kod süresi dolduysa form
        assert_eq!(t.p.home_dil(IP, Some(MAC), "", Dil::En, true).tpl, "giris");
    }

    // --- misafir kodu
    const TC: &str = "10000000146"; // algoritmaya uyan uydurma numara
    const TC_KEY: &str = "TC:10000000146";
    const PAS_KEY: &str = "PASAPORT:DE:X1234567";

    fn otel(tur: &'static str) -> impl FnOnce(&mut Config) {
        move |c: &mut Config| {
            c.main.isletme_turu = Some(tur.into());
            c.giris.misafir_kodu = true;
        }
    }

    /// Personelin panelden ürettiği kod (depoya yazılır).
    fn kod_uret(t: &T, kimlik: &str, saat: u64, uretim: f64) -> misafir::Kod {
        let mut k = misafir::oku(&t.p.cfg.main.state_root);
        let (yeni, _) = misafir::uret(&mut k, kimlik, "Ali", "Veli", "101", saat, "mudur", uretim);
        misafir::kaydet(&t.p.cfg.main.state_root, &k).unwrap();
        yeni
    }

    fn mform(tur: &str, kimlik: &str, kod: &str) -> Form {
        let mut f = form(&[("tur", tur), ("kod", kod), ("sozlesme", "1"), ("riza", "1"), ("dst", "http://neverssl.com/")]);
        match tur {
            "tc" => {
                f.insert("tc".into(), kimlik.into());
            }
            _ => {
                f.insert("pasaport".into(), kimlik.into());
                f.insert("uyruk".into(), "DE".into());
            }
        }
        f
    }

    fn baska(kod: &str) -> String {
        if kod == "000000" { "111111".into() } else { "000000".into() }
    }

    #[test]
    fn misafir_tc_ile_giris() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let pg = t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap();
        assert_eq!((pg.tpl, pg.get("baslik")), ("misafir", "Otel misafiriyim"));
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", "100 0000 0146", &k.kod)).unwrap();
        assert_eq!(pg.tpl, "basarili");
        assert!(pg.get("bas_html").contains("url=http://neverssl.com/"));
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!((ses[MAC].phone.as_str(), ses[MAC].ad.as_str(), ses[MAC].soyad.as_str()), (TC_KEY, "Ali", "Veli"));
        assert_eq!(ses[MAC].expires_epoch, k.bitis);
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout 86400s }}")));
        // 5651: oturum satırı tam kimlikle, ek alanı; kişi dosyası güvenli adla; kişi listesi
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        let bitis = ortak::now_iso(k.bitis);
        let satir = oturum.lines().find(|l| l.contains("OTURUM_BASLA")).unwrap();
        assert!(satir.contains(TC_KEY), "{satir}");
        assert!(satir.ends_with(&format!("bitis={bitis} sozlesme=1 riza=1 giris=misafir_kodu oda=101 kod_bitis={bitis} ureten=mudur")), "{satir}");
        let user = t.read("5651/gunluk/2026-09-29/kullanicilar/TC_10000000146.csv");
        assert!(user.contains(";KAYIT;TC:10000000146;Ali;Veli;") && user.contains(";OTURUM_BASLA;"));
        assert!(t.read("5651/kullanicilar/index.csv").contains("TC:10000000146;Ali;Veli;"));
        // denetim: kimlik ve kod yok, maskeli kimlik var
        let a = t.audit_text();
        assert!(a.contains("MISAFIR_BASARILI") && a.contains("100******46"), "{a}");
        assert!(!a.contains(TC) && !a.contains(&k.kod), "{a}");
        assert_eq!(t.sms_count(), 0);
        // zaten bağlı: sayfa başarı
        assert_eq!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap().tpl, "basarili");
    }

    #[test]
    fn misafir_pasaport_ile_giris() {
        let t = setup(otel("yurt"));
        let k = kod_uret(&t, PAS_KEY, 48, WALL0);
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("pasaport", "x123-4567", &k.kod)).unwrap();
        assert_eq!(pg.tpl, "basarili");
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].phone, PAS_KEY);
        assert!(t.read("5651/gunluk/2026-09-29/kullanicilar/PASAPORT_DE_X1234567.csv").contains(";KAYIT;PASAPORT:DE:X1234567;"));
        let a = t.audit_text();
        assert!(a.contains("DE X12***67") && !a.contains("X1234567"), "{a}");
        // uyruk farklıysa eşleşmez
        let mut f = mform("pasaport", "X1234567", &k.kod);
        f.insert("uyruk".into(), "FR".into());
        assert_eq!(t.p.misafir_dogrula("10.50.0.24", Some("aa:bb:cc:dd:ee:02"), &f).unwrap().get("hata_genel"), "Bilgiler eşleşmedi ya da kodun süresi doldu.");
    }

    #[test]
    fn misafir_yanlis_kod_tek_mesaj() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let iptal = kod_uret(&t, PAS_KEY, 24, WALL0);
        let mut kodlar = misafir::oku(&t.p.cfg.main.state_root);
        misafir::iptal(&mut kodlar, &iptal.kod_id);
        misafir::kaydet(&t.p.cfg.main.state_root, &kodlar).unwrap();
        let eski = kod_uret(&t, "TC:20000000046", 24, WALL0 - 25.0 * 3600.0); // süresi dolmuş
        const TEK: &str = "Bilgiler eşleşmedi ya da kodun süresi doldu.";
        let denemeler = [
            mform("tc", TC, &baska(&k.kod)),          // kod yanlış
            mform("tc", "11111111110", &k.kod),       // kimlik yok
            mform("tc", "20000000046", &eski.kod),     // süresi dolmuş
            mform("pasaport", "X1234567", &iptal.kod), // iptal edilmiş
        ];
        for (i, f) in denemeler.iter().enumerate() {
            let pg = t.p.misafir_dogrula(IP, Some(MAC), f).unwrap();
            assert_eq!((pg.tpl, pg.get("hata_genel")), ("misafir", TEK), "{i}");
            assert!(pg.ctx.iter().all(|(k2, _)| *k2 != "kod"), "kod forma geri yazılmaz");
        }
        // İngilizce
        let mut f = mform("tc", "20000000046", &eski.kod);
        f.insert("dil".into(), "en".into());
        let pg = t.p.misafir_dogrula("10.50.0.25", Some("aa:bb:cc:dd:ee:03"), &f).unwrap();
        assert_eq!(pg.get("hata_genel"), "The details did not match or the code has expired.");
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        // biçim hataları alan iletisi; hata sayılmaz
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", "10000000147", "12")).unwrap();
        assert!(pg.get("hata_genel").is_empty() && !pg.get("hata_tc").is_empty() && !pg.get("hata_kod").is_empty());
        let a = t.audit_text();
        assert_eq!(a.matches("MISAFIR_HATALI").count(), 5, "{a}");
        assert!(!a.contains(TC) && !a.contains("11111111110") && !a.contains("20000000046") && !a.contains("X1234567"), "{a}");
    }

    #[test]
    fn misafir_kilit_ip_ve_mac() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let yanlis = mform("tc", TC, &baska(&k.kod));
        let dogru = mform("tc", TC, &k.kod);
        const KILIT: &str = "Çok fazla hatalı deneme yapıldı. Lütfen 15 dakika sonra tekrar deneyin.";
        // aynı IP, farklı MAC'ler: 5 hata → IP kilitli, doğru kod da reddedilir
        for i in 0..5 {
            t.p.misafir_dogrula(IP, Some(&format!("aa:bb:cc:dd:ee:1{i}")), &yanlis);
        }
        assert_eq!(t.p.misafir_dogrula(IP, Some("aa:bb:cc:dd:ee:20"), &dogru).unwrap().get("hata_genel"), KILIT);
        // aynı MAC, farklı IP'ler: 5 hata → MAC kilitli
        for i in 0..5 {
            t.p.misafir_dogrula(&format!("10.50.0.{}", 30 + i), Some("aa:bb:cc:dd:ee:30"), &yanlis);
        }
        assert_eq!(t.p.misafir_dogrula("10.50.0.40", Some("aa:bb:cc:dd:ee:30"), &dogru).unwrap().get("hata_genel"), KILIT);
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        // SMS doğrulama sayacından ayrı: SMS akışı kilitlenmez
        assert_eq!(t.p.send_code(IP, Some("aa:bb:cc:dd:ee:20"), &good("5334553132")).tpl, "kod");
        // başka IP ve MAC etkilenmez
        assert_eq!(t.p.misafir_dogrula("10.50.0.41", Some("aa:bb:cc:dd:ee:41"), &dogru).unwrap().tpl, "basarili");
        // 15 dk sonra kilit kalkar
        t.advance(901.0);
        assert_eq!(t.p.misafir_dogrula(IP, Some("aa:bb:cc:dd:ee:20"), &dogru).unwrap().tpl, "basarili");
        assert!(t.audit_text().contains("red=kilit"));
    }

    #[test]
    fn misafir_oturum_kod_bitisinde_biter() {
        let t = setup(otel("otel"));
        // kod 4 saat önce 1 haftalık üretildi: kalan süre 164 saat
        let k = kod_uret(&t, TC_KEY, 168, WALL0 - 4.0 * 3600.0);
        assert_eq!(t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", TC, &k.kod)).unwrap().tpl, "basarili");
        let s = &ortak::load_sessions(&t.p.cfg.main.state_root)[MAC];
        assert_eq!(s.expires_epoch, WALL0 + 164.0 * 3600.0);
        let kalan = (164 * 3600).to_string();
        assert!(t.calls.lock().unwrap().iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout {kalan}s }}")));
        // ayardaki 30 günlük oturum süresi uygulanmaz
        assert!(!t.calls.lock().unwrap().iter().any(|c| c.last().unwrap().contains("2592000s")));
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        assert!(oturum.contains(&format!("bitis={} ", ortak::now_iso(k.bitis))));
    }

    #[test]
    fn kod_basina_cihaz_siniri() {
        let t = setup(|c| {
            otel("otel")(c);
            c.main.max_devices_per_phone = 2;
        });
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let diger = kod_uret(&t, PAS_KEY, 24, WALL0);
        assert_eq!(t.p.misafir_dogrula("10.50.0.29", Some("aa:bb:cc:dd:ee:09"), &mform("pasaport", "X1234567", &diger.kod)).unwrap().tpl, "basarili");
        for i in 1..=3 {
            t.advance(1.0);
            let pg = t.p.misafir_dogrula(&format!("10.50.0.2{i}"), Some(&format!("aa:bb:cc:dd:ee:0{i}")), &mform("tc", TC, &k.kod));
            assert_eq!(pg.unwrap().tpl, "basarili", "{i}");
        }
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses.keys().cloned().collect::<Vec<_>>(), vec!["aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:03", "aa:bb:cc:dd:ee:09"]);
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        assert!(oturum.lines().any(|l| l.contains("OTURUM_BITIS") && l.contains("aa:bb:cc:dd:ee:01") && l.contains("neden=cihaz_limiti")));
    }

    #[test]
    fn kafede_misafir_yolu_yok() {
        // kafe (misafir_kodu ayarı açık kalmış olsa da) ve otel ama kapalı: yol yok, düğme yok
        for tweak in [otel("kafe"), otel("diger")] {
            let t = setup(tweak);
            kod_uret(&t, TC_KEY, 24, WALL0);
            assert!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).is_none());
            assert!(t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", TC, "123456")).is_none());
            assert!(!t.p.render(&t.p.home(IP, Some(MAC), "")).contains("/misafir"));
        }
        let t = setup(|c| c.main.isletme_turu = Some("otel".into()));
        assert!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).is_none());
        assert!(!t.p.render(&t.p.home(IP, Some(MAC), "")).contains("/misafir"));
        // otel / yurt açık: türe göre düğme, dst ve dil korunur
        let t = setup(otel("otel"));
        let base = t.p.base_url.clone();
        let html = t.p.render(&t.p.home(IP, Some(MAC), "http://a.com/"));
        assert!(html.contains(&format!("<a class=\"dugme\" href=\"{base}/misafir?dil=tr&amp;dst=http%3A%2F%2Fa.com%2F\">Otel misafiriyim</a>")), "{html}");
        assert!(t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false)).contains(">Hotel guest</a>"));
        let t = setup(otel("yurt"));
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains(">Yurt sakiniyim</a>"));
        let html = t.p.render(&t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::En).unwrap());
        assert!(html.contains("<h1>Dormitory resident</h1>") && html.contains("Enter the code given or sent to you by our staff"));
        assert!(html.contains(&format!("href=\"{base}/misafir?dil=tr\"")) && html.contains(&format!("action=\"{base}/misafir/dogrula\"")));
    }

    #[test]
    fn kupon_kelimesi_yok() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        for d in [Dil::Tr, Dil::En] {
            let mut sayfalar = vec![
                t.p.home_dil(IP, Some(MAC), "", d, false),
                t.p.misafir_sayfasi(IP, Some(MAC), "", d).unwrap(),
            ];
            let mut f = mform("tc", TC, &baska(&k.kod));
            f.insert("dil".into(), d.kod().into());
            sayfalar.push(t.p.misafir_dogrula(IP, Some(MAC), &f).unwrap());
            for pg in &sayfalar {
                let html = t.p.render(pg).to_lowercase();
                assert!(!html.contains("kupon") && !html.contains("coupon"), "{}", pg.tpl);
            }
        }
        let html = t.p.render(&t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap());
        assert!(html.contains("Personelin size verdiği veya ilettiği kodu girin") && !html.contains("$m_"));
        for tpl in ["giris", "misafir", "_sayfa", "basarili", "hata", "kod"] {
            assert!(!template(tpl).to_lowercase().contains("kupon"), "{tpl}");
        }
    }

    #[test]
    fn hukuki_metin_en_bossa_turkce() {
        // İngilizce alanlar boş: Türkçe metinler (eklerle) görünür, başlıklar İngilizce
        let t = setup(|c| c.main.unvan = "Boca Ltd.".into());
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("Bocafe’nin uhdesindeki") && html.contains("Şirketinizin Wi-Fi internet hizmetini"));
        assert!(html.contains("<summary>Privacy Notice (KVKK)</summary>") && html.contains("<summary>Internet User Agreement</summary>"));
        // İngilizce yazılmışsa: İŞLETMECİ yalnızca adla, unvan yer tutucusu unvanla değişir
        let t = setup(|c| {
            c.main.unvan = "Boca Ltd.".into();
            c.portal.aydinlatma_en = "Data controller: İŞLETMECİ".into();
            c.portal.acik_riza_en = "I consent to sharing with [vergi levhası unvanı].".into();
            c.portal.sozlesme_en = "Rules of İŞLETMECİ’s network".into();
        });
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("Data controller: Bocafe") && html.contains("I consent to sharing with Boca Ltd.") && html.contains("Rules of Bocafe’s network"));
        assert!(!html.contains("uhdesindeki") && !html.contains("Şirketinizin"));
        // Türkçe sayfa İngilizce metinlerden etkilenmez
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(html.contains("Bocafe’nin uhdesindeki") && !html.contains("Data controller"));
        // ikisi de boşsa bilgi notu sayfanın dilinde
        let t = setup(|c| c.portal.aydinlatma = String::new());
        assert!(t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false)).contains("This text has not been added yet."));
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains("Metin henüz eklenmedi."));
    }
}
