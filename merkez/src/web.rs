//! Yönetim merkezi web tabanı (Python web.py): CSS, sayfa çerçevesi, çerezler, giriş/çıkış/CSRF/gerekçe (spec §5–§6, §10).
//! Uygulamalar (yönetim, müşteri) `Panel` özelliğini uygular ve yalnızca kendi sayfalarını yazar; ortak akış `Panel`'in
//! varsayılan yöntemlerindedir (Python `Taban`'ın alt sınıf kancaları + ortak yöntemleri).
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::ops::Deref;
use std::sync::{Arc, LazyLock};

use crate::guvenlik::{self, GirisKilidi, Oturum, Oturumlar};
use crate::ortak::{py_bol, py_bosluk, py_repr, py_strip};
use crate::kayitlar::Kayitlar;
use crate::veri::{Saat, Sonuc, Veri};

pub use crate::detay::e;

pub const GEREKCE_SN: f64 = 1800.0;
pub const GEREKCE_ONERILERI: [&str; 4] =
    ["Emniyet / savcılık talebi", "Müşteri şikâyeti", "Müşterinin kendi talebi", "Teknik sorun incelemesi"];

pub const CSS: &str = r##"*{box-sizing:border-box}body{margin:0;font:16px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;background:#f5f7fa;color:#1d2330}
header{background:#fff;border-bottom:1px solid #e3e7ee;padding:12px 16px;display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}
.marka{font-weight:700;color:#0a6ebd;font-size:18px;text-decoration:none}nav{display:flex;gap:12px;align-items:center;flex-wrap:wrap}nav form{margin:0}
.marka svg.logo{display:block;width:166px;height:24px}
main{max-width:640px;margin:0 auto;padding:16px}main.genis{max-width:1100px}.kart{background:#fff;border:1px solid #e3e7ee;border-radius:12px;padding:16px;margin-bottom:16px}
h1{font-size:20px;margin:0 0 4px}.alt{color:#5b6475;font-size:14px;margin:0}
label{display:block;font-size:14px;margin:12px 0 4px}input,select{width:100%;padding:10px 12px;border:1px solid #c9d0db;border-radius:8px;font:inherit}
input[type=checkbox]{width:auto}button{background:#0a6ebd;color:#fff;border:0;border-radius:999px;padding:10px 18px;font:inherit;cursor:pointer;margin-top:16px}
button.ikincil{background:#fff;color:#0a6ebd;border:1px solid #0a6ebd;margin:0;padding:4px 14px}button.tehlike{background:#a4262c}
.hata{background:#fdecec;color:#a4262c;padding:10px 12px;border-radius:8px;margin:12px 0 0}.kotu{color:#a4262c}.uyarisi{color:#8a5a00;font-weight:600}
.tamam{background:#e7f6ec;color:#1e6b34;padding:10px 12px;border-radius:8px;margin:12px 0 0}
.sir{font:600 22px ui-monospace,Consolas,monospace;letter-spacing:1px;user-select:all}
table{width:100%;border-collapse:collapse;margin-bottom:8px}td,th{padding:6px 4px;border-bottom:1px solid #eef1f5;font-size:14px;text-align:left;vertical-align:top}
td.cubuk{width:100%}td.cubuk span{display:block;height:10px;background:#0a6ebd;border-radius:5px;min-width:2px}
td.n{text-align:right;font-weight:600;min-width:3ch}.bos{color:#9aa3b2}a{color:#0a6ebd}h2{font-size:17px;margin:0 0 8px}h3{font-size:15px;margin:12px 0 4px}
details{border:1px solid #e3e7ee;border-radius:10px;margin:8px 0;background:#fff}summary{cursor:pointer;padding:10px 12px}details>div{padding:0 12px 12px}
.kaydir{overflow-x:auto}table.detay th{font-size:13px;color:#5b6475}ul.gunler{list-style:none;margin:0;padding:0 12px 12px}ul.gunler li{padding:4px 0}
form.ara{display:flex;gap:8px;flex-wrap:wrap}form.ara input{flex:1;min-width:10em}form.ara button{margin:0}
form.ara select{width:auto}form.ara input.uyruk{flex:0 0 5.5em;min-width:0}.iyi{color:#1e6b34;font-weight:600}
td.cubuk span{-webkit-print-color-adjust:exact;print-color-adjust:exact}
.yerlesim{display:flex;align-items:stretch;min-height:100vh}.yerlesim main{flex:1;min-width:0}
nav.yan{display:block;flex:0 0 220px;background:#1d2330;padding:16px 0 24px}nav.yan .marka{display:block;color:#fff;font-size:18px;font-weight:700;padding:0 16px 12px}
nav.yan details{border:0;border-radius:0;margin:0;background:none}nav.yan summary::-webkit-details-marker{display:none}
nav.yan summary{display:flex;justify-content:space-between;align-items:center;list-style:none;padding:9px 16px;color:#fff;font-weight:600;font-size:15px;text-decoration:none}
nav.yan summary::after{content:"";width:7px;height:7px;border-right:2px solid #8a93a6;border-bottom:2px solid #8a93a6;transform:rotate(45deg);margin:-4px 2px 0 8px}
nav.yan details[open]>summary::after{transform:rotate(-135deg);margin-top:4px}
nav.yan a{display:block;padding:7px 16px 7px 28px;color:#e3e7ee;text-decoration:none;font-size:15px}
nav.yan summary:hover,nav.yan a:hover{background:#2a3242}nav.yan .marka:hover{background:none}nav.yan a.secili{background:#0a6ebd;color:#fff}
nav.yan form{margin:16px 16px 0}nav.yan button{margin:0;width:100%}nav.yan .marka svg.logo{width:180px;height:26px}
.gizli{position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);white-space:nowrap}.ust-mobil{display:none}
details.bolum{background:#fff;border:1px solid #e3e7ee;border-radius:12px;margin:0 0 12px}details.bolum>div{padding:0 16px 16px}
details.bolum>summary{display:flex;align-items:center;gap:4px 12px;flex-wrap:wrap;list-style:none;padding:14px 16px}
details.bolum>summary::-webkit-details-marker{display:none}details.bolum>summary h2{margin:0;flex:none}
details.bolum>summary .bolum-ozet{flex:1;min-width:0;text-align:right;color:#5b6475;font-size:14px}
details.bolum>summary::after{content:"";flex:none;width:7px;height:7px;border-right:2px solid #5b6475;border-bottom:2px solid #5b6475;transform:rotate(45deg);margin:-4px 2px 0 4px}
details.bolum[open]>summary::after{transform:rotate(-135deg);margin-top:4px}details.bolum[open]>summary{border-bottom:1px solid #eef1f5;margin-bottom:12px}
details.ic{border:0;border-radius:0;margin:6px 0 0;background:none}details.ic>summary{display:inline-block;padding:2px 0;color:#0a6ebd;font-size:14px}
details.ic>div{padding:4px 0 8px}details.ic.tehlikeli>summary{color:#a4262c}form.satir-ici{display:inline-block;margin:0 0 0 10px;vertical-align:middle}
h1 .alt-baslik{font-weight:400;color:#5b6475}.durumlar{display:flex;flex-wrap:wrap;gap:4px 20px;margin:8px 0 0;font-size:14px;color:#5b6475}
.ortak,.dugmeler{display:flex;flex-wrap:wrap;gap:8px;align-items:center}.ortak{margin-top:14px}.dugmeler{margin:12px 0}
.ortak form,.dugmeler form{margin:0}.ortak button,.dugmeler button{margin:0}
a.dugme{display:inline-block;background:#fff;color:#0a6ebd;border:1px solid #0a6ebd;border-radius:999px;padding:9px 18px;text-decoration:none}
details.bolum table:not(.detay) th{width:1%;white-space:nowrap;padding-right:24px}details.bolum form.ara input[type=date]{flex:0 1 13em}
@media(max-width:760px){details.bolum>summary::after{order:1;margin-left:auto}details.bolum>summary .bolum-ozet{order:2;flex-basis:100%;text-align:left}
details.bolum tr:has(form,details)>th,details.bolum tr:has(form,details)>td{display:block;width:auto}details.bolum tr:has(form,details)>th{border-bottom:0;padding-bottom:0}}
@media(max-width:760px){.ust-mobil{position:sticky;top:0;z-index:5;display:flex;align-items:center;gap:8px;min-height:52px;padding:0 12px 0 4px;background:#1d2330}
.ust-mobil .marka{color:#fff}.ust-mobil .marka svg.logo{width:139px;height:20px}.menu-dugme{display:inline-flex;align-items:center;justify-content:center;width:44px;height:44px;flex:none;margin:0;color:#fff;cursor:pointer}
.menu-dugme svg{width:24px;height:24px;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round}
.yerlesim{display:block;min-height:0}nav.yan{display:none;position:sticky;top:52px;z-index:4;max-height:calc(100vh - 52px);overflow-y:auto;padding:6px 0 14px}
nav.yan>.marka{display:none}nav.yan form{margin:10px 16px 0}#menu-ac:checked~.yerlesim nav.yan{display:block}
#menu-ac:focus-visible~.ust-mobil .menu-dugme{outline:2px solid #fff;outline-offset:-4px}}
@media print{header,nav,.ust-mobil,.yazdirma{display:none}body{background:#fff}main,main.genis{max-width:none;padding:0}
.kart{border:0;padding:0 0 8px;break-inside:avoid}.kaydir{overflow:visible}@page{size:A4;margin:12mm}}"##;

const SINIF_GENIS: &str = " class=\"genis\"";

// --- wificorrect_ logosu (2026-10-24, kullanıcının logo kiti; harfler yola çevrilmiş) ---
// Cihazla tek kopya: src/sablon/logo (test-merkez.sh cihaza gönderir). Sayfaya gömülür (satır içi SVG, data: simge).
const LOGO_ACIK_DOSYA: &str = include_str!("../../src/sablon/logo/logo-acik.svg");
const LOGO_KOYU_DOSYA: &str = include_str!("../../src/sablon/logo/logo-koyu.svg");
const FAVICON_SVG_DOSYA: &[u8] = include_bytes!("../../src/sablon/logo/favicon.svg");
const FAVICON_PNG_DOSYA: &[u8] = include_bytes!("../../src/sablon/logo/favicon-32.png");

pub fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for p in b.chunks(3) {
        let n = (p[0] as u32) << 16 | (*p.get(1).unwrap_or(&0) as u32) << 8 | *p.get(2).unwrap_or(&0) as u32;
        o.push(A[(n >> 18) as usize & 63] as char);
        o.push(A[(n >> 12) as usize & 63] as char);
        o.push(if p.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if p.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    o
}

/// Satır içi logo: sınıf verilir, ekran okuyucudan gizlenir (adı çevreleyen bağlantı verir).
pub fn logo_svg(dosya: &str) -> String {
    let s = py_strip(dosya);
    if s.starts_with("<svg ") {
        s.replacen("<svg ", "<svg class=\"logo\" aria-hidden=\"true\" focusable=\"false\" ", 1)
    } else {
        String::new()
    }
}

/// Sekme simgesi: W_ ikonu; 32 px PNG önce, SVG sonra (ikisini de tanıyan tarayıcı sonuncuyu seçer). CSP img-src data: verir.
pub fn favicon_html(png: &[u8], svg: &[u8]) -> String {
    let svg = svg.trim_ascii();
    let mut h = String::new();
    if !png.is_empty() {
        h += &format!("<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"data:image/png;base64,{}\">", base64(png));
    }
    if !svg.is_empty() {
        h += &format!("<link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,{}\">", base64(svg));
    }
    h
}

pub static LOGO_ACIK: LazyLock<String> = LazyLock::new(|| logo_svg(LOGO_ACIK_DOSYA));
pub static LOGO_KOYU: LazyLock<String> = LazyLock::new(|| logo_svg(LOGO_KOYU_DOSYA));
pub static FAVICON: LazyLock<String> = LazyLock::new(|| favicon_html(FAVICON_PNG_DOSYA, FAVICON_SVG_DOSYA));

fn marka(logo: &str) -> String {
    format!("<a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\">{}</a>", if logo.is_empty() { "WifiCorrect" } else { logo })
}

/// journal'a (stderr); fail2ban GIRIS_HATALI satırlarını okur.
pub fn log(msg: &str) {
    #[cfg(test)]
    {
        if YAKALANAN.with(|y| y.borrow_mut().as_mut().map(|v| v.push(msg.to_string())).is_some()) {
            return;
        }
    }
    crate::ortak::stderr_yaz(msg);
}

#[cfg(test)]
thread_local! {
    static YAKALANAN: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Testler: `f` çalışırken bu iş parçacığındaki `log` satırlarını stderr yerine toplar (Python `redirect_stderr`).
#[cfg(test)]
pub fn log_yakala<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    YAKALANAN.with(|y| *y.borrow_mut() = Some(Vec::new()));
    let r = f();
    let satirlar = YAKALANAN.with(|y| y.borrow_mut().take()).unwrap_or_default();
    (r, satirlar)
}

// --- İstek / Yanıt ---

/// Büyük/küçük harf duyarsız başlık listesi (sıra ve yinelenen adlar korunur).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Basliklar(pub Vec<(String, String)>);

impl Basliklar {
    #[cfg(test)]
    pub fn yeni(l: &[(&str, &str)]) -> Basliklar {
        Basliklar(l.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    /// İlk eşleşen başlığın değeri (Python `Message.get`).
    pub fn get(&self, ad: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(ad)).map(|(_, v)| v.as_str())
    }

    #[cfg(test)]
    pub fn ekle(&mut self, ad: &str, deger: &str) {
        self.0.push((ad.to_string(), deger.to_string()));
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Istek {
    pub yontem: String,
    pub yol: String,
    pub basliklar: Basliklar,
    pub govde: String,
    pub sorgu: String,
    pub ip: String,
}

impl Istek {
    #[cfg(test)]
    pub fn yeni(yontem: &str, yol: &str) -> Istek {
        Istek { yontem: yontem.into(), yol: yol.into(), ..Default::default() }
    }

    #[cfg(test)]
    pub fn baslik(mut self, ad: &str, deger: &str) -> Istek {
        self.basliklar.ekle(ad, deger);
        self
    }

    #[cfg(test)]
    pub fn govde(mut self, g: &str) -> Istek {
        self.govde = g.into();
        self
    }

    #[cfg(test)]
    pub fn sorgu(mut self, s: &str) -> Istek {
        self.sorgu = s.into();
        self
    }

    #[cfg(test)]
    pub fn ip(mut self, ip: &str) -> Istek {
        self.ip = ip.into();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Yanit {
    pub durum: u16,
    /// Sıralı; aynı ad birden çok satır olabilir (Set-Cookie).
    pub basliklar: Vec<(String, String)>,
    pub govde: Vec<u8>,
}

impl Yanit {
    pub fn yeni(durum: u16, basliklar: &[(&str, &str)], govde: Vec<u8>) -> Yanit {
        Yanit { durum, basliklar: basliklar.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), govde }
    }

    /// `text/plain; charset=utf-8` gövdeli yanıt.
    pub fn metin(durum: u16, metin: &str) -> Yanit {
        Yanit::yeni(durum, &[("Content-Type", "text/plain; charset=utf-8")], metin.as_bytes().to_vec())
    }

    #[cfg(test)]
    pub fn baslik(&self, ad: &str) -> Option<&str> {
        self.basliklar.iter().find(|(k, _)| k.eq_ignore_ascii_case(ad)).map(|(_, v)| v.as_str())
    }

    #[cfg(test)]
    pub fn basliklar_hepsi(&self, ad: &str) -> Vec<&str> {
        self.basliklar.iter().filter(|(k, _)| k.eq_ignore_ascii_case(ad)).map(|(_, v)| v.as_str()).collect()
    }

    #[cfg(test)]
    pub fn govde_metin(&self) -> String {
        String::from_utf8_lossy(&self.govde).into_owned()
    }
}

/// Bir uygulama hatası ya da paniği: ayrıntı journal'a, kullanıcıya genel mesaj (Python `merkez.py` işleyicisi).
pub fn hata_yaniti(yontem: &str, yol: &str, ayrinti: &str) -> Yanit {
    log(&format!("HATA {yontem} {yol}: {ayrinti}"));
    Yanit::metin(500, "Bir sorun oluştu")
}

// --- URL kodlaması ---

/// Python `urllib.parse.unquote` (UTF-8, geçersiz bayt U+FFFD; geçersiz `%` olduğu gibi kalır).
pub fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                o.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// Python `urllib.parse.quote(s, safe="")`: harf, rakam ve `_.-~` dışındaki her bayt `%XX`.
pub fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// Python `parse_qsl` (boş değerler atılır, `+` boşluk, `%XX`).
fn qsl(s: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for parca in s.split('&') {
        if parca.is_empty() {
            continue;
        }
        let Some((ad, deger)) = parca.split_once('=') else { continue };
        if deger.is_empty() {
            continue;
        }
        v.push((unquote(&ad.replace('+', " ")), unquote(&deger.replace('+', " "))));
    }
    v
}

/// `dict(parse_qsl(govde))`: aynı anahtarın tekrarında sonuncusu kalır.
pub fn form(govde: &str) -> HashMap<String, String> {
    qsl(govde).into_iter().collect()
}

/// `parse_qs(sorgu).get(anahtar)[0]`: anahtarın ilk (boş olmayan) değeri.
pub fn sorgu_ilk(sorgu: &str, anahtar: &str) -> Option<String> {
    qsl(sorgu).into_iter().find(|(k, _)| k == anahtar).map(|(_, v)| v)
}

/// Cookie başlığından `ad` çerezini okur (SimpleCookie ilk bozuk çerezde durduğu için elle).
pub fn cerez_degeri(baslik: Option<&str>, ad: &str) -> Option<String> {
    for parca in baslik.unwrap_or("").split(';') {
        let parca = py_strip(parca);
        let (k, v) = parca.split_once('=').unwrap_or((parca, ""));
        if k == ad {
            return Some(v.to_string());
        }
    }
    None
}

/// (çerez adı, belirteç); belirteç `None` ise çerez silinir.
pub type Cerez<'a> = (&'a str, Option<&'a str>);

fn cerez_basligi(c: Cerez) -> String {
    match c.1 {
        None => format!("{}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0", c.0),
        Some(t) => format!("{}={}; HttpOnly; Secure; SameSite=Strict; Path=/", c.0, t),
    }
}

pub fn yanit_html(govde: &str, durum: u16, cerez: Option<Cerez>) -> Yanit {
    let mut b = vec![("Content-Type".to_string(), "text/html; charset=utf-8".to_string()), ("Cache-Control".to_string(), "no-store".to_string())];
    if let Some(c) = cerez {
        b.push(("Set-Cookie".into(), cerez_basligi(c)));
    }
    Yanit { durum, basliklar: b, govde: govde.as_bytes().to_vec() }
}

pub fn yonlendir(yer: &str, cerez: Option<Cerez>) -> Yanit {
    let mut b = vec![("Location".to_string(), yer.to_string()), ("Cache-Control".to_string(), "no-store".to_string())];
    if let Some(c) = cerez {
        b.push(("Set-Cookie".into(), cerez_basligi(c)));
    }
    Yanit { durum: 303, basliklar: b, govde: Vec::new() }
}

/// Yalnızca bu sitedeki bir yol. Tarayıcılar sekme/satır sonunu silip "/\t/x"i "//x"e çevirir: boşluk ve denetim
/// karakteri, ters bölü ve "//" reddedilir.
pub fn guvenli_donus(p: &str) -> String {
    if !p.starts_with('/') || p.starts_with("//") {
        return "/".into();
    }
    if p.chars().any(|c| py_bosluk(c) || (c as u32) < 0x20 || c as u32 == 0x7f || c == '\\') {
        "/".into()
    } else {
        p.to_string()
    }
}

// --- HTML parçaları ---

pub fn kart(icerik: &str) -> String {
    format!("<div class=\"kart\">{icerik}</div>")
}

/// Başlık kaçışlanır, değer HTML olarak olduğu gibi.
pub fn bilgi_tablosu<K: Display, V: Display>(satirlar: &[(K, V)]) -> String {
    let mut o = String::from("<table>");
    for (k, v) in satirlar {
        o += &format!("<tr><th>{}</th><td>{}</td></tr>", e(k), v);
    }
    o + "</table>"
}

/// Açılır bölüm (betik yok: details/summary): kapalıyken başlık ve yanında kısa durum (ozet, HTML) görünür.
/// kimlik: sayfa içi çapa; işlemden dönüşte ?acik=<kimlik> ile bölüm açık gelir.
pub fn bolum(kimlik: &str, baslik: &str, ozet: &str, icerik: &str, acik: bool) -> String {
    format!(
        "<details class=\"bolum\" id=\"{kimlik}\"{}><summary><h2>{}</h2><span class=\"bolum-ozet\">{ozet}</span></summary><div>{icerik}</div></details>",
        if acik { " open" } else { "" },
        e(baslik)
    )
}

/// Bölüm içinde tıklayınca açılan kısım: uzun açıklama, geçmiş tablosu, nadir ya da tehlikeli işlem (sinif="tehlikeli").
pub fn acilir(baslik: &str, icerik: &str, sinif: &str) -> String {
    format!(
        "<details class=\"ic{}\"><summary>{}</summary><div>{icerik}</div></details>",
        if sinif.is_empty() { String::new() } else { format!(" {sinif}") },
        e(baslik)
    )
}

/// yenile: sayfa bu kadar saniyede kendini yeniler (meta refresh; JavaScript yok). yan: sol menünün içeriği; verilirse
/// üst şerit yerine sol menü çizilir (yönetim). Dar ekranda menü gizlenir, üstteki şeritte menü düğmesi açar/kapatır
/// (cihaz paneli gibi; betik yok (CSP): gizli onay kutusu + etiket, kutu .yerlesim'den önce gelmeli).
pub fn cerceve(baslik: &str, icerik: &str, nav: &str, genis: bool, yenile: Option<u32>, yan: &str) -> String {
    let yen = yenile.map(|n| format!("<meta http-equiv=\"refresh\" content=\"{n}\">")).unwrap_or_default();
    let ana = format!("<main{}>{icerik}</main>", if genis { SINIF_GENIS } else { "" });
    let govde = if !yan.is_empty() {
        format!(
            "<input type=\"checkbox\" id=\"menu-ac\" class=\"gizli\"><div class=\"ust-mobil\">\
             <label for=\"menu-ac\" class=\"menu-dugme\" title=\"Menü\"><span class=\"gizli\">Menüyü aç/kapat</span>\
             <svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M3 6h18M3 12h18M3 18h18\"/></svg></label>\
             {m}</div>\
             <div class=\"yerlesim\"><nav class=\"yan\">{m}{yan}</nav>{ana}</div>",
            m = marka(&LOGO_KOYU)
        )
    } else {
        format!("<header>{}{nav}</header>{ana}", marka(&LOGO_ACIK))
    };
    format!(
        "<!doctype html><html lang=\"tr\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">{yen}\
         <title>{} · WifiCorrect</title>{}<style>{CSS}</style></head><body>{govde}</body></html>",
        e(baslik),
        &*FAVICON
    )
}

// --- Oturum görünümü ve Taban ---

/// Python'daki `ot` sözlüğü: oturum kaydının kopyası + açık sayfanın yolu (sol menüde seçili bağlantı için).
#[derive(Clone, Debug)]
pub struct Ot {
    pub oturum: Oturum,
    pub yol: String,
}

impl Deref for Ot {
    type Target = Oturum;
    fn deref(&self) -> &Oturum {
        &self.oturum
    }
}

/// Alt sınıfların paylaştığı durum (Python `Taban.__init__`).
pub struct Taban {
    pub veri: Arc<Veri>,
    /// Müşteri ve yönetim panelleri tek örneği paylaşır (Python `merkez.py` tek `Kayitlar`): `kisi_sir` ilk üretimde yarışmaz.
    pub kayitlar: Arc<Kayitlar>,
    pub saat: Saat,
    /// haftalık/aylık rapor JSON dosyalarını okuyan nesne (Python `raporlar.Raporlar`; varsayılan /var/lib/wificorrect/rapor).
    pub raporlar: crate::raporlar::Raporlar,
    pub oturumlar: Oturumlar,
    pub kilit: GirisKilidi,
}

impl Taban {
    pub fn new(veri: Arc<Veri>, kayitlar: impl Into<Arc<Kayitlar>>, saat: Saat) -> Taban {
        Taban { veri, kayitlar: kayitlar.into(), saat, raporlar: crate::raporlar::Raporlar::default(), oturumlar: Oturumlar::default(), kilit: GirisKilidi::default() }
    }

    /// Rapor dosyalarının klasörü (Python `Taban(..., raporlar=Raporlar(dizin))`).
    #[cfg(test)]
    pub fn raporlar_ile(mut self, r: crate::raporlar::Raporlar) -> Taban {
        self.raporlar = r;
        self
    }

    pub fn simdi(&self) -> f64 {
        (self.saat)()
    }
}

/// Alt sınıfın yazdıkları (Python'da `Taban`'dan türeyen sınıf) + ortak akış (varsayılan yöntemler).
/// `Uygulama` (merkez.rs) her `Panel` için `isle` üzerinden uygulanır. Kancalar veritabanı hatasında `Err` verir
/// (Python'da istisna): cevapla bunu 500 "Bir sorun oluştu" yapar.
#[allow(clippy::too_many_arguments)]
pub trait Panel: Send + Sync {
    fn taban(&self) -> &Taban;
    /// "yonetici" | "musteri"
    fn rol(&self) -> &'static str;
    /// Oturum çerezinin adı.
    fn cerez(&self) -> &'static str;
    /// true: menü sol tarafta (`menu` grup grup yazar), değilse üst şeritte.
    fn yan_menu(&self) -> bool {
        false
    }
    fn kullanici_etiketi(&self) -> &'static str {
        "Kullanıcı adı"
    }
    fn giris_basligi(&self) -> &'static str {
        "Giriş"
    }
    fn giris_aciklama(&self) -> &'static str {
        ""
    }

    /// Kimlik (oturumdaki kullanıcı) ya da None.
    fn dogrula(&self, kul: &str, pw: &str) -> Sonuc<Option<String>>;
    /// Hesabın parola özeti; hesap yoksa None. Oturumdakiyle aynı değilse oturum düşer.
    fn hesap_surumu(&self, kimlik: &str) -> Sonuc<Option<String>>;
    fn girdi(&self, kimlik: &str, ip: &str) -> Sonuc<()>;
    fn kim(&self, ot: &Ot) -> String;
    fn hareket_musterisi(&self, ot: &Ot, yol: &str) -> Option<i64>;
    fn menu(&self, _ot: &Ot) -> String {
        String::new()
    }
    fn giris_sonrasi(&self, _kimlik: &str) -> Sonuc<String> {
        Ok("/".into())
    }
    /// Uygulamaya özel sayfalar; None: böyle bir sayfa yok (404).
    fn sayfalar(&self, _ot: &Ot, _token: &str, _yontem: &str, _yol: &str, _sorgu: &str, _form: &HashMap<String, String>, _ip: &str) -> Sonuc<Option<Yanit>> {
        Ok(None)
    }

    // --- ortak ---
    fn oturum_ac(&self, kimlik: &str, simdi: f64) -> Sonuc<String> {
        let surum = self.hesap_surumu(kimlik)?.unwrap_or_default();
        Ok(self.taban().oturumlar.create(kimlik, self.rol(), simdi, &surum, BTreeMap::new()))
    }

    fn sayfa(&self, baslik: &str, icerik: &str, ot: Option<&Ot>, genis: bool, yenile: Option<u32>) -> String {
        let (mut nav, mut yan) = (String::new(), String::new());
        if let Some(ot) = ot {
            let cikis = format!(
                "<form method=\"post\" action=\"/cikis\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                 <button class=\"ikincil\">Çıkış</button></form>",
                e(&ot.csrf)
            );
            if self.yan_menu() {
                yan = self.menu(ot) + &cikis;
            } else {
                nav = format!("<nav>{}{cikis}</nav>", self.menu(ot));
            }
        }
        cerceve(baslik, icerik, &nav, genis, yenile, &yan)
    }

    fn mesaj(&self, ot: Option<&Ot>, baslik: &str, metin: &str) -> String {
        self.sayfa(baslik, &kart(&format!("<h1>{}</h1><p>{}</p><p><a href=\"/\">Ana sayfa</a></p>", e(baslik), e(metin))), ot, false, None)
    }

    fn giris_html(&self, hata: &str) -> String {
        let h = if hata.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", e(hata)) };
        self.sayfa(
            "Giriş",
            &kart(&format!(
                "<h1>{}</h1><p class=\"alt\">{}</p><form method=\"post\" action=\"/giris\"><label for=\"k\">{}</label>\
                 <input id=\"k\" name=\"kullanici\" autocomplete=\"username\" autocapitalize=\"none\" spellcheck=\"false\" required>\
                 <label for=\"s\">Parola</label><input id=\"s\" name=\"sifre\" type=\"password\" autocomplete=\"current-password\" required>\
                 {h}<button>Giriş yap</button></form>",
                e(self.giris_basligi()),
                e(self.giris_aciklama()),
                e(self.kullanici_etiketi())
            )),
            None,
            false,
            None,
        )
    }

    /// Python `Taban.istek`: oturum, CSRF, giriş/çıkış/gerekçe, sonra uygulamanın sayfaları.
    fn isle(&self, i: &Istek) -> Sonuc<Yanit> {
        let t = self.taban();
        let simdi = t.simdi();
        let token = cerez_degeri(i.basliklar.get("Cookie"), self.cerez());
        let mut ot = match &token {
            Some(tk) => t.oturumlar.get(tk, simdi),
            None => None,
        };
        if let (Some(o), Some(tk)) = (&ot, &token) {
            // hesap silinmiş ya da parolası değişmiş
            if self.hesap_surumu(&o.user)?.unwrap_or_default() != o.surum {
                t.oturumlar.drop(tk);
                ot = None;
            }
        }
        let f = if i.yontem == "POST" { form(&i.govde) } else { HashMap::new() };
        if i.yol == "/giris" {
            if i.yontem == "POST" {
                return self.giris(&f, &i.ip, simdi);
            }
            // oturum varken giriş sayfası (ör. cihaz.wificorrect.com oturumsuz → panel girişi): giriş sonrası sayfaya
            return match ot {
                Some(o) => Ok(yonlendir(&self.giris_sonrasi(&o.user)?, None)),
                None => Ok(yanit_html(&self.giris_html(""), 200, None)),
            };
        }
        let (Some(o), Some(token)) = (ot, token) else {
            return Ok(yonlendir("/giris", None));
        };
        let ot = Ot { oturum: o, yol: i.yol.clone() }; // kopya: oturum kaydına yazılmaz
        if i.yontem == "POST" && !guvenlik::sabit_esit(f.get("csrf").map(String::as_str).unwrap_or("").as_bytes(), ot.csrf.as_bytes()) {
            return Ok(yanit_html(&self.mesaj(Some(&ot), "Geçersiz istek", "Sayfayı yenileyip tekrar deneyin."), 403, None));
        }
        if i.yontem == "POST" && i.yol == "/cikis" {
            t.oturumlar.drop(&token);
            return Ok(yonlendir("/giris", Some((self.cerez(), None))));
        }
        if i.yol == "/gerekce" {
            return self.gerekce(&ot, &token, &i.yontem, &i.sorgu, &f, &i.ip, simdi);
        }
        match self.sayfalar(&ot, &token, &i.yontem, &i.yol, &i.sorgu, &f, &i.ip)? {
            Some(r) => Ok(r),
            None => Ok(yanit_html(&self.mesaj(Some(&ot), "Bulunamadı", "Böyle bir sayfa yok."), 404, None)),
        }
    }

    fn giris(&self, f: &HashMap<String, String>, ip: &str, simdi: f64) -> Sonuc<Yanit> {
        let t = self.taban();
        let kul = py_strip(f.get("kullanici").map(String::as_str).unwrap_or("")).to_lowercase();
        if !t.kilit.attempt(ip, &kul, simdi) {
            log(&giris_satiri("GIRIS_KILITLI", self.rol(), &kul, ip));
            return Ok(yanit_html(&self.giris_html("Çok fazla deneme. 15 dakika sonra tekrar deneyin."), 429, None));
        }
        let Some(kimlik) = self.dogrula(&kul, f.get("sifre").map(String::as_str).unwrap_or(""))? else {
            log(&giris_satiri("GIRIS_HATALI", self.rol(), &kul, ip));
            return Ok(yanit_html(&self.giris_html(&format!("{} veya parola hatalı.", self.kullanici_etiketi())), 401, None));
        };
        t.kilit.succeed(ip, &kul, simdi);
        self.girdi(&kimlik, ip)?;
        let token = self.oturum_ac(&kimlik, simdi)?;
        Ok(yonlendir(&self.giris_sonrasi(&kimlik)?, Some((self.cerez(), Some(&token)))))
    }

    fn gerekce(&self, ot: &Ot, token: &str, yontem: &str, sorgu: &str, f: &HashMap<String, String>, ip: &str, simdi: f64) -> Sonuc<Yanit> {
        if yontem == "GET" {
            let donus = guvenli_donus(&sorgu_ilk(sorgu, "donus").unwrap_or_else(|| "/".into()));
            return Ok(yanit_html(&self.gerekce_html(ot, &donus, ""), 200, None));
        }
        let metin = py_bol(f.get("gerekce").map(String::as_str).unwrap_or("")).join(" ");
        let donus = guvenli_donus(f.get("donus").map(String::as_str).unwrap_or("/"));
        if !(5..=200).contains(&metin.chars().count()) {
            return Ok(yanit_html(&self.gerekce_html(ot, &donus, "Gerekçe 5–200 karakter olmalı."), 400, None));
        }
        let t = self.taban();
        t.oturumlar.set_gerekce(token, &metin, simdi + GEREKCE_SN);
        t.veri.hareket(&self.kim(ot), ip, "GEREKCE", self.hareket_musterisi(ot, &donus), &format!("{donus} · {metin}"))?;
        Ok(yonlendir(&donus, None))
    }

    fn gerekce_html(&self, ot: &Ot, donus: &str, hata: &str) -> String {
        let secenekler: String = GEREKCE_ONERILERI.iter().map(|x| format!("<option value=\"{}\">", e(x))).collect();
        let h = if hata.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", e(hata)) };
        self.sayfa(
            "Gerekçe",
            &kart(&format!(
                "<h1>Gerekçe</h1><p>Müşterilerin kişisel verisine bakmak için gerekçe yazın.</p>\
                 <p class=\"alt\">Gerekçe {} dakika geçerlidir; bu süredeki her görüntüleme gerekçeyle kaydedilir.</p>\
                 <form method=\"post\" action=\"/gerekce\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                 <input type=\"hidden\" name=\"donus\" value=\"{}\">\
                 <label for=\"g\">Gerekçe</label><input id=\"g\" name=\"gerekce\" list=\"oneriler\" minlength=\"5\" maxlength=\"200\" required>\
                 <datalist id=\"oneriler\">{secenekler}</datalist>{h}<button>Devam</button></form>",
                GEREKCE_SN as i64 / 60,
                e(&ot.csrf),
                e(donus)
            )),
            Some(ot),
            false,
            None,
        )
    }

    /// Geçerli gerekçe yoksa gerekçe sayfasına yönlendirir; varsa görüntülemeyi kaydeder ve None döner.
    fn gerekce_iste(&self, ot: &Ot, tam_yol: &str, musteri: Option<i64>, ip: &str) -> Sonuc<Option<Yanit>> {
        let t = self.taban();
        let g = match &ot.gerekce {
            Some(g) if g.1 > t.simdi() => g,
            _ => return Ok(Some(yonlendir(&format!("/gerekce?donus={}", quote(tam_yol)), None))),
        };
        t.veri.hareket(&self.kim(ot), ip, "GORUNTULEME", musteri, &format!("{tam_yol} · {}", g.0))?;
        Ok(None)
    }

    /// Python `Taban.kayit_sayfasi`. `alt`: "/", "/gun/<gün>", "/ara". Kişi verisi açan sayfalar gerekçe ister (sayılar ve boş
    /// arama formu istemez). Arama yalnızca POST'la yapılır (telefon/kimlik adrese, tarayıcı geçmişine ve hareket kaydına
    /// girmez). None: böyle bir sayfa yok.
    #[allow(clippy::too_many_arguments)]
    fn kayit_sayfasi(
        &self,
        ot: &Ot,
        numara: i64,
        alt: &str,
        sorgu: &str,
        onek: &str,
        ip: &str,
        baslik: &str,
        yontem: &str,
        form: Option<&HashMap<String, String>>,
    ) -> Sonuc<Option<Yanit>> {
        let t = self.taban();
        let hata = |m: String| crate::veri::Hata::Deger(format!("kayıtlar: {m}"));
        let arsivler = t.veri.arsivler(numara)?;
        if yontem == "POST" && alt != "/ara" {
            return Ok(None);
        }
        if alt == "/" {
            let h = t.kayitlar.ozet_html(baslik, &arsivler, t.simdi(), onek, &ot.csrf);
            return Ok(Some(yanit_html(&self.sayfa(baslik, &h, Some(ot), false, None), 200, None)));
        }
        if alt.starts_with("/gun/") || yontem == "POST" {
            let ek = if !sorgu.is_empty() && yontem == "GET" { format!("?{sorgu}") } else { String::new() };
            if let Some(r) = self.gerekce_iste(ot, &format!("{onek}{alt}{ek}"), Some(numara), ip)? {
                return Ok(Some(r));
            }
        }
        if let Some(gun) = alt.strip_prefix("/gun/") {
            return Ok(Some(match t.kayitlar.gun_html(&arsivler, gun, onek).map_err(hata)? {
                None => yanit_html(&self.mesaj(Some(ot), "Bulunamadı", "Bu gün için kayıt yok."), 404, None),
                Some(h) => yanit_html(&self.sayfa(&crate::detay::tarih(gun), &h, Some(ot), false, None), 200, None),
            }));
        }
        if alt == "/ara" {
            let h = t.kayitlar.ara_html(&arsivler, if yontem == "POST" { form } else { None }, onek, &ot.csrf).map_err(hata)?;
            return Ok(Some(yanit_html(&self.sayfa("Arama", &h, Some(ot), false, None), 200, None)));
        }
        Ok(None)
    }
}

/// fail2ban'ın okuduğu satır: `<OLAY> alan=<rol> kullanici=<repr> ip=<ip>` (satır `ip=<ip>` ile biter).
pub fn giris_satiri(olay: &str, rol: &str, kul: &str, ip: &str) -> String {
    format!("{olay} alan={rol} kullanici={} ip={ip}", py_repr(kul))
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::merkez::Uygulama;
    use std::path::Path;
    use std::sync::Mutex;

    fn saat_sabit(t: Arc<Mutex<f64>>) -> Saat {
        Arc::new(move || *crate::ortak::kilitle(&t))
    }

    fn sozluk(l: &[(&str, &str)]) -> HashMap<String, String> {
        l.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // --- form / sorgu çözme: Python parse_qsl + dict, parse_qs(...)[0] ile aynı (değerler Python'dan alındı) ---

    #[test]
    fn form_cozme_python_ile_ayni() {
        assert_eq!(form("a=1&a=2"), sozluk(&[("a", "2")]));
        assert_eq!(form("a=1+2&b=%C3%A7"), sozluk(&[("a", "1 2"), ("b", "ç")]));
        assert_eq!(form("x=%zz&y=%4"), sozluk(&[("x", "%zz"), ("y", "%4")]));
        assert_eq!(form("e=&f=1"), sozluk(&[("f", "1")])); // boş değer atılır
        assert_eq!(form("k&l=2"), sozluk(&[("l", "2")])); // '=' yok: atılır
        assert_eq!(form("=v"), sozluk(&[("", "v")]));
        assert_eq!(form("a=%FF%FE"), sozluk(&[("a", "\u{fffd}\u{fffd}")]));
        assert_eq!(form("a%20b=c%2Bd"), sozluk(&[("a b", "c+d")]));
        assert_eq!(form("a=1&&b=2&"), sozluk(&[("a", "1"), ("b", "2")]));
        assert_eq!(form("a=1;b=2"), sozluk(&[("a", "1;b=2")])); // ';' ayraç değil
        assert_eq!(form("a=%e2%82"), sozluk(&[("a", "\u{fffd}")]));
        assert_eq!(form("a=%E2%82%AC"), sozluk(&[("a", "€")]));
        assert_eq!(form("a=%2"), sozluk(&[("a", "%2")]));
        assert_eq!(form("a=%"), sozluk(&[("a", "%")]));
        assert_eq!(form("a=1=2"), sozluk(&[("a", "1=2")]));
        assert_eq!(form("%41=%42"), sozluk(&[("A", "B")]));
        assert_eq!(form("a=%C3%A7%zz%41"), sozluk(&[("a", "ç%zzA")]));
        assert_eq!(form("u=çç%41"), sozluk(&[("u", "ççA")]));
        assert_eq!(form(""), HashMap::new());
    }

    #[test]
    fn sorgu_ilk_degeri() {
        assert_eq!(sorgu_ilk("a=1&a=2", "a").as_deref(), Some("1"));
        assert_eq!(sorgu_ilk("a=&a=2", "a").as_deref(), Some("2"));
        assert_eq!(sorgu_ilk("a=", "a"), None);
        assert_eq!(sorgu_ilk("b=1", "a"), None);
        assert_eq!(sorgu_ilk("donus=%2Fm%2F1&x=2", "donus").as_deref(), Some("/m/1"));
        assert_eq!(sorgu_ilk("a=1;b=2", "a").as_deref(), Some("1;b=2"));
    }

    #[test]
    fn url_kodlama() {
        assert_eq!(quote("/m/1234567/kayit?x=1&y=ç ğ~._-"), "%2Fm%2F1234567%2Fkayit%3Fx%3D1%26y%3D%C3%A7%20%C4%9F~._-");
        assert_eq!(unquote(&quote("/a b/ç?x=%")), "/a b/ç?x=%");
    }

    #[test]
    fn cerez_okuma_ve_basliklar() {
        assert_eq!(cerez_degeri(Some("a=1; wc=tok; b=2"), "wc").as_deref(), Some("tok"));
        assert_eq!(cerez_degeri(Some("a=1;wc=tok"), "wc").as_deref(), Some("tok"));
        assert_eq!(cerez_degeri(Some("x=1; wc"), "wc").as_deref(), Some(""));
        assert_eq!(cerez_degeri(Some("a=1"), "wc"), None);
        assert_eq!(cerez_degeri(None, "wc"), None);
        let y = yanit_html("<b>", 200, Some(("wc", Some("t"))));
        assert_eq!(
            y.basliklar,
            vec![
                ("Content-Type".to_string(), "text/html; charset=utf-8".to_string()),
                ("Cache-Control".to_string(), "no-store".to_string()),
                ("Set-Cookie".to_string(), "wc=t; HttpOnly; Secure; SameSite=Strict; Path=/".to_string()),
            ]
        );
        let y = yonlendir("/giris", Some(("wc", None)));
        assert_eq!(y.durum, 303);
        assert_eq!(y.baslik("set-cookie"), Some("wc=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"));
        assert_eq!(y.baslik("Location"), Some("/giris"));
        assert!(y.govde.is_empty());
    }

    #[test]
    fn guvenli_donus_kurallari() {
        assert_eq!(guvenli_donus("/m/1"), "/m/1");
        for kotu in ["", "x", "//evil.com", "/\t/evil.com", "/a b", "/a\nb", "/a\\b", "/a\u{7f}", "http://x"] {
            assert_eq!(guvenli_donus(kotu), "/", "{kotu:?}");
        }
    }

    #[test]
    fn html_parcalari() {
        assert_eq!(kart("x"), "<div class=\"kart\">x</div>");
        assert_eq!(
            bilgi_tablosu(&[("A<b>", "<i>1</i>"), ("B", "2")]),
            "<table><tr><th>A&lt;b&gt;</th><td><i>1</i></td></tr><tr><th>B</th><td>2</td></tr></table>"
        );
        assert_eq!(
            bolum("cihaz", "Cihaz", "<b>ok</b>", "içerik", true),
            "<details class=\"bolum\" id=\"cihaz\" open><summary><h2>Cihaz</h2><span class=\"bolum-ozet\"><b>ok</b></span></summary><div>içerik</div></details>"
        );
        assert!(bolum("c", "C&", "", "", false).contains("<h2>C&amp;</h2>") && !bolum("c", "C", "", "", false).contains(" open"));
        assert_eq!(acilir("Aç", "x", ""), "<details class=\"ic\"><summary>Aç</summary><div>x</div></details>");
        assert_eq!(acilir("Aç", "x", "tehlikeli"), "<details class=\"ic tehlikeli\"><summary>Aç</summary><div>x</div></details>");
    }

    #[test]
    fn logo_ve_favicon_python_ile_ayni_bicimde() {
        assert!(LOGO_ACIK.starts_with("<svg class=\"logo\" aria-hidden=\"true\" focusable=\"false\" ") && LOGO_ACIK.ends_with("</svg>"));
        assert!(LOGO_KOYU.starts_with("<svg class=\"logo\" aria-hidden=\"true\" focusable=\"false\" "));
        assert!(FAVICON.starts_with("<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"data:image/png;base64,iVBOR"));
        assert!(FAVICON.contains("<link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,"));
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(logo_svg("  <svg viewBox=\"0\"></svg>\n"), "<svg class=\"logo\" aria-hidden=\"true\" focusable=\"false\" viewBox=\"0\"></svg>");
        assert_eq!(logo_svg("<?xml ?><svg ></svg>"), "");
        assert_eq!(favicon_html(&[], b""), "");
    }

    #[test]
    fn cerceve_ust_serit_ve_yan_menu() {
        let h = cerceve("Başlık <1>", "<p>x</p>", "<nav>n</nav>", false, None, "");
        assert!(h.starts_with("<!doctype html><html lang=\"tr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\""));
        assert!(h.contains("<title>Başlık &lt;1&gt; · WifiCorrect</title>") && h.contains("<style>*{box-sizing:border-box}"));
        assert!(h.contains("<body><header><a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\"><svg class=\"logo\""));
        assert!(h.ends_with("<nav>n</nav></header><main><p>x</p></main></body></html>"));
        let h = cerceve("B", "i", "", true, Some(5), "<a>m</a>");
        assert!(h.contains("<meta http-equiv=\"refresh\" content=\"5\">") && h.contains("<main class=\"genis\">i</main>"));
        assert!(h.contains("<input type=\"checkbox\" id=\"menu-ac\" class=\"gizli\"><div class=\"ust-mobil\">"));
        assert!(h.contains("<div class=\"yerlesim\"><nav class=\"yan\"><a class=\"marka\"") && h.contains("<a>m</a></nav><main class=\"genis\">"));
        assert_eq!(h.matches("aria-label=\"wificorrect_\"").count(), 2);
    }

    // --- Panel akışı: sahte panel, bellek içi veritabanı, ayarlanabilir saat ---

    struct Sahte {
        t: Taban,
        ozet: Mutex<String>,
        yan: bool,
    }

    impl Panel for Sahte {
        fn taban(&self) -> &Taban {
            &self.t
        }
        fn rol(&self) -> &'static str {
            "musteri"
        }
        fn cerez(&self) -> &'static str {
            "wc_deneme"
        }
        fn yan_menu(&self) -> bool {
            self.yan
        }
        fn kullanici_etiketi(&self) -> &'static str {
            "Müşteri numarası"
        }
        fn giris_basligi(&self) -> &'static str {
            "Müşteri girişi"
        }
        fn giris_aciklama(&self) -> &'static str {
            "Size verilen numarayla girin."
        }
        fn dogrula(&self, kul: &str, pw: &str) -> Sonuc<Option<String>> {
            Ok((kul == "1234567" && pw == "gizli").then(|| kul.to_string()))
        }
        fn hesap_surumu(&self, kimlik: &str) -> Sonuc<Option<String>> {
            Ok((kimlik == "1234567").then(|| crate::ortak::kilitle(&self.ozet).clone()))
        }
        fn girdi(&self, kimlik: &str, ip: &str) -> Sonuc<()> {
            self.t.veri.hareket(kimlik, ip, "GIRIS", Some(1234567), "deneme")
        }
        fn kim(&self, ot: &Ot) -> String {
            ot.user.clone()
        }
        fn hareket_musterisi(&self, _: &Ot, _: &str) -> Option<i64> {
            None
        }
        fn menu(&self, _: &Ot) -> String {
            "<a href=\"/\">Ana</a>".into()
        }
        fn giris_sonrasi(&self, _: &str) -> Sonuc<String> {
            Ok("/ana".into())
        }
        fn sayfalar(&self, ot: &Ot, _: &str, yontem: &str, yol: &str, _: &str, _: &HashMap<String, String>, ip: &str) -> Sonuc<Option<Yanit>> {
            Ok(match (yontem, yol) {
                ("GET", "/ana") => Some(yanit_html(&self.sayfa("Ana", "<p>merhaba</p>", Some(ot), false, None), 200, None)),
                ("GET", "/gizli") => match self.gerekce_iste(ot, "/gizli?x=1", Some(7), ip)? {
                    Some(y) => Some(y),
                    None => Some(yanit_html("kişisel veri", 200, None)),
                },
                _ => None,
            })
        }
    }

    fn sahte(yan: bool) -> (Arc<Sahte>, Arc<Mutex<f64>>) {
        let zaman = Arc::new(Mutex::new(1_000.0));
        let saat = saat_sabit(zaman.clone());
        let veri = Arc::new(Veri::ac(Path::new(":memory:"), saat.clone()).unwrap());
        (Arc::new(Sahte { t: Taban::new(veri, crate::kayitlar::Kayitlar::default(), saat), ozet: Mutex::new("ozet-1".into()), yan }), zaman)
    }

    fn gonder(p: &Sahte, yontem: &str, yol: &str, cerez: Option<&str>, govde: &str) -> Yanit {
        let mut i = Istek::yeni(yontem, yol).govde(govde).ip("203.0.113.5");
        if let Some(c) = cerez {
            i = i.baslik("Cookie", c);
        }
        p.istek(&i)
    }

    /// (çerez, csrf)
    fn giris_yap(p: &Sahte) -> (String, String) {
        let y = gonder(p, "POST", "/giris", None, "kullanici=1234567&sifre=gizli");
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/ana")));
        let sc = y.baslik("Set-Cookie").unwrap();
        assert!(sc.starts_with("wc_deneme=") && sc.ends_with("; HttpOnly; Secure; SameSite=Strict; Path=/"), "{sc}");
        let cerez = sc.split(';').next().unwrap().to_string();
        let h = gonder(p, "GET", "/ana", Some(&cerez), "").govde_metin();
        let ayrac = "name=\"csrf\" value=\"";
        let bas = h.find(ayrac).unwrap() + ayrac.len();
        (cerez, h[bas..bas + h[bas..].find('"').unwrap()].to_string())
    }

    #[test]
    fn oturumsuz_giris_sayfasina_yonlenir_giris_sayfasi() {
        let (p, _) = sahte(false);
        let y = gonder(&p, "GET", "/ana", None, "");
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/giris")));
        let y = gonder(&p, "GET", "/giris", None, "");
        assert_eq!(y.durum, 200);
        let h = y.govde_metin();
        assert!(h.contains("<h1>Müşteri girişi</h1><p class=\"alt\">Size verilen numarayla girin.</p>"));
        assert!(h.contains("<label for=\"k\">Müşteri numarası</label>") && !h.contains("class=\"hata\""));
        assert_eq!(y.baslik("Content-Type"), Some("text/html; charset=utf-8"));
    }

    #[test]
    fn giris_basarili_cikis_ve_csrf() {
        let (p, _) = sahte(false);
        let (cerez, csrf) = giris_yap(&p);
        // oturum varken giriş sayfası giriş sonrası sayfaya yönlenir
        let y = gonder(&p, "GET", "/giris", Some(&cerez), "");
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/ana")));
        // üst şerit: menü ve çıkış formu
        assert!(gonder(&p, "GET", "/ana", Some(&cerez), "").govde_metin().contains("<nav><a href=\"/\">Ana</a><form method=\"post\" action=\"/cikis\">"));
        // bilinmeyen sayfa 404, sayfa çerçevesiyle
        let y = gonder(&p, "GET", "/yok", Some(&cerez), "");
        assert_eq!(y.durum, 404);
        assert!(y.govde_metin().contains("<h1>Bulunamadı</h1><p>Böyle bir sayfa yok.</p>"));
        // csrf'siz POST
        let y = gonder(&p, "POST", "/cikis", Some(&cerez), "csrf=yanlis");
        assert_eq!(y.durum, 403);
        assert!(y.govde_metin().contains("Sayfayı yenileyip tekrar deneyin."));
        // doğru csrf ile çıkış: çerez silinir, oturum düşer
        let y = gonder(&p, "POST", "/cikis", Some(&cerez), &format!("csrf={csrf}"));
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/giris")));
        assert!(y.baslik("Set-Cookie").unwrap().starts_with("wc_deneme=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"));
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").baslik("Location"), Some("/giris"));
        // giriş hareketi yazıldı
        let h = p.t.veri.hareketler(None, 10, None).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].kim.as_str(), h[0].olay.as_str(), h[0].ip.as_str(), h[0].musteri), ("1234567", "GIRIS", "203.0.113.5", Some(1234567)));
    }

    #[test]
    fn hatali_giris_401_kilit_429() {
        let (p, zaman) = sahte(false);
        for _ in 0..5 {
            let y = gonder(&p, "POST", "/giris", None, "kullanici=%20%201234567%20&sifre=yanlis");
            assert_eq!(y.durum, 401);
            assert!(y.govde_metin().contains("<p class=\"hata\">Müşteri numarası veya parola hatalı.</p>"));
        }
        let y = gonder(&p, "POST", "/giris", None, "kullanici=1234567&sifre=gizli");
        assert_eq!(y.durum, 429);
        assert!(y.govde_metin().contains("Çok fazla deneme. 15 dakika sonra tekrar deneyin."));
        // pencere dolunca yeniden denenebilir
        *crate::ortak::kilitle(&zaman) += 901.0;
        assert_eq!(gonder(&p, "POST", "/giris", None, "kullanici=1234567&sifre=gizli").durum, 303);
    }

    #[test]
    fn parola_degisince_oturum_duser() {
        let (p, _) = sahte(false);
        let (cerez, _) = giris_yap(&p);
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").durum, 200);
        *crate::ortak::kilitle(&p.ozet) = "ozet-2".into();
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").baslik("Location"), Some("/giris"));
        *crate::ortak::kilitle(&p.ozet) = "ozet-1".into();
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").baslik("Location"), Some("/giris")); // oturum silindi
    }

    #[test]
    fn oturum_bosta_30_dakikada_biter() {
        let (p, zaman) = sahte(false);
        let (cerez, _) = giris_yap(&p);
        *crate::ortak::kilitle(&zaman) += 1801.0;
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").durum, 303);
    }

    #[test]
    fn gerekce_akisi() {
        let (p, zaman) = sahte(false);
        let (cerez, csrf) = giris_yap(&p);
        // gerekçe yok: gerekçe sayfasına
        let y = gonder(&p, "GET", "/gizli", Some(&cerez), "");
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/gerekce?donus=%2Fgizli%3Fx%3D1")));
        // gerekçe sayfası (GET): dönüş güvenli
        let al = |sorgu: &str| p.istek(&Istek::yeni("GET", "/gerekce").sorgu(sorgu).baslik("Cookie", &cerez));
        let h = al("donus=%2Fgizli").govde_metin();
        assert!(h.contains("<input type=\"hidden\" name=\"donus\" value=\"/gizli\">") && h.contains("Gerekçe 30 dakika geçerlidir"));
        assert!(h.contains("<option value=\"Müşteri şikâyeti\">"));
        assert!(al("donus=%2F%2Fevil.com").govde_metin().contains("name=\"donus\" value=\"/\""));
        // çok kısa gerekçe: 400
        let y = gonder(&p, "POST", "/gerekce", Some(&cerez), &format!("csrf={csrf}&donus=%2Fgizli&gerekce=+abc+"));
        assert_eq!(y.durum, 400);
        assert!(y.govde_metin().contains("Gerekçe 5–200 karakter olmalı."));
        // geçerli: boşluklar sadeleşir, hareket yazılır, dönüşe gider
        let govde = format!("csrf={csrf}&donus=%2Fgizli%3Fx%3D1&gerekce=Musteri++%C5%9Fikayeti%0A+incelemesi");
        let y = gonder(&p, "POST", "/gerekce", Some(&cerez), &govde);
        assert_eq!((y.durum, y.baslik("Location")), (303, Some("/gizli?x=1")));
        let y = gonder(&p, "GET", "/gizli", Some(&cerez), "");
        assert_eq!((y.durum, y.govde_metin().as_str()), (200, "kişisel veri"));
        let h = p.t.veri.hareketler(None, 10, None).unwrap();
        let olaylar: Vec<(&str, &str, Option<i64>)> = h.iter().map(|x| (x.olay.as_str(), x.ayrinti.as_str(), x.musteri)).collect();
        assert!(olaylar.contains(&("GEREKCE", "/gizli?x=1 · Musteri şikayeti incelemesi", None)), "{olaylar:?}");
        assert!(olaylar.contains(&("GORUNTULEME", "/gizli?x=1 · Musteri şikayeti incelemesi", Some(7))), "{olaylar:?}");
        // gerekçe 30 dk sonra biter (oturum boşta kalmasın diye araya istek)
        *crate::ortak::kilitle(&zaman) += 1000.0;
        assert_eq!(gonder(&p, "GET", "/ana", Some(&cerez), "").durum, 200);
        *crate::ortak::kilitle(&zaman) += 900.0;
        assert_eq!(gonder(&p, "GET", "/gizli", Some(&cerez), "").durum, 303);
    }

    #[test]
    fn yan_menu_sol_menu_cizer() {
        let (p, _) = sahte(true);
        let (cerez, _) = giris_yap(&p);
        let h = gonder(&p, "GET", "/ana", Some(&cerez), "").govde_metin();
        assert!(h.contains("<div class=\"yerlesim\"><nav class=\"yan\">") && h.contains("<a href=\"/\">Ana</a><form method=\"post\" action=\"/cikis\">"));
        assert!(!h.contains("<header>"));
    }

    #[test]
    fn fail2ban_satiri() {
        assert_eq!(giris_satiri("GIRIS_HATALI", "musteri", "100001", "1.2.3.4"), "GIRIS_HATALI alan=musteri kullanici='100001' ip=1.2.3.4");
        assert_eq!(giris_satiri("GIRIS_KILITLI", "yonetici", "o'neil", "1.2.3.4"), "GIRIS_KILITLI alan=yonetici kullanici=\"o'neil\" ip=1.2.3.4");
        assert_eq!(giris_satiri("GIRIS_HATALI", "yonetici", "a\nb\t\\", "::1"), "GIRIS_HATALI alan=yonetici kullanici='a\\nb\\t\\\\' ip=::1");
        assert_eq!(py_repr("é\u{200b}\u{7f}"), "'é\\u200b\\x7f'");
    }
}
