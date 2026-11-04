//! NetGSM (Python netgsm.py): genel SMS ucu (XML, 1:n; merkezden hizmet sağlayıcıya uyarı SMS'i) ve OTP ucu (misafir
//! doğrulama kodu). Yeniden deneme yok (çift SMS = çift ücret). Parola ve istek gövdesi hiçbir yere yazılmaz: gövde curl'e
//! yalnızca standart girdiden gider (`ortak::Curl`). Taşıyıcı dışarıdan verilir; testte sahte, üretimde `Curl`.
use std::time::Duration;

use serde_json::Value;

use crate::ortak::{normalize_phone, py_bol, py_dogru, py_str, py_strip, DisIstek, Tasiyici};

pub const URL: &str = "https://api.netgsm.com.tr/sms/send/xml";
pub const OTP_URL: &str = "https://api.netgsm.com.tr/sms/send/otp";
pub const BASARI: [&str; 2] = ["00", "0"];
/// Genel (uyarı) SMS için işlem zaman aşımı.
pub const GENEL_SURE: Duration = Duration::from_secs(10);
/// OTP için işlem başına zaman aşımı: bağlan + oku (TLS dahil) en kötü ~10-11 sn, cihaz merkezi 15 sn bekler. Çağrı bitmeden
/// cevap dönülmez (SMS gitmiş ama kod atılmış olmasın).
pub const ISLEM_SURE: Duration = Duration::from_secs(5);
const OKUMA_SINIRI: usize = 4096;

/// 90XXXXXXXXXX / 0XXXXXXXXXX / 5XXXXXXXXX → 5XXXXXXXXX; geçersizler ve tekrarlar atlanır.
pub fn numaralar_10<S: AsRef<str>>(numaralar: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in numaralar {
        if let Some(d) = normalize_phone(n.as_ref()) {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

/// XML'de geçersiz denetim karakterleri (`[\x00-\x08\x0b\x0c\x0e-\x1f]`).
fn denetim_mi(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}')
}

/// `xml.sax.saxutils.escape`: yalnızca & < >.
fn sax_kacis(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn xml_govde<S: AsRef<str>>(usercode: &str, password: &str, msgheader: &str, metin: &str, numaralar: &[S]) -> String {
    let temiz: String = metin.chars().filter(|c| !denetim_mi(*c)).collect();
    let msg = temiz.replace("]]>", "]]]]><![CDATA[>"); // metin CDATA'dan taşamaz
    let nolar: String = numaralar_10(numaralar).iter().map(|n| format!("<no>{n}</no>")).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><mainbody><header><company dil=\"TR\">Netgsm</company>\
         <usercode>{}</usercode><password>{}</password>\
         <type>1:n</type><msgheader>{}</msgheader></header>\
         <body><msg><![CDATA[{msg}]]></msg>{nolar}</body></mainbody>",
        sax_kacis(usercode),
        sax_kacis(password),
        sax_kacis(msgheader)
    )
}

/// `<ad>…</ad>` içi (ilk eşleşme, satır sonları dahil), boşlukları kırpılmış.
fn etiket(metin: &str, ad: &str) -> Option<String> {
    let acilis = format!("<{ad}>");
    let kapanis = format!("</{ad}>");
    let bas = metin.find(&acilis)? + acilis.len();
    let son = bas + metin[bas..].find(&kapanis)?;
    Some(py_strip(&metin[bas..son]).to_string())
}

/// `<code>\s*([^<\s]*)\s*</code>` (ilk eşleşme).
fn code_etiketi(metin: &str) -> Option<String> {
    const ACILIS: &str = "<code>";
    let mut konum = 0;
    while let Some(i) = metin[konum..].find(ACILIS) {
        let bas = konum + i + ACILIS.len();
        let kalan = metin[bas..].trim_start_matches(crate::ortak::py_bosluk);
        let govde_son = kalan.find(|c: char| c == '<' || crate::ortak::py_bosluk(c)).unwrap_or(kalan.len());
        let sonra = kalan[govde_son..].trim_start_matches(crate::ortak::py_bosluk);
        if sonra.starts_with("</code>") {
            return Some(kalan[..govde_son].to_string());
        }
        konum = bas;
    }
    None
}

/// "00 123456" → "00"; XML yanıtta `<code>`; boşsa "".
pub fn yanit_kodu(metin: &str) -> String {
    let metin = py_strip(metin);
    if let Some(k) = code_etiketi(metin) {
        return k;
    }
    py_bol(metin).first().map(|s| s.to_string()).unwrap_or_default()
}

fn rakam_mi(s: &str, en_cok: usize) -> bool {
    !s.is_empty() && s.len() <= en_cok && s.bytes().all(|b| b.is_ascii_digit())
}

/// Genel SMS. Dönen: `gonderildi` | `mock` | `hata:<kod>` (AYAR, NUMARA, AG, BILINMIYOR ya da NetGSM kodu).
pub fn gonder<S: AsRef<str>>(ayar: &Value, metin: &str, numaralar: &[S], t: &dyn Tasiyici) -> String {
    if !py_dogru(ayar) {
        return "hata:AYAR".into();
    }
    if ayar.get("mock").is_some_and(py_dogru) {
        return "mock".into();
    }
    if !["usercode", "password", "msgheader"].iter().all(|k| ayar.get(*k).is_some_and(py_dogru)) {
        return "hata:AYAR".into();
    }
    let nolar = numaralar_10(numaralar);
    if nolar.is_empty() {
        return "hata:NUMARA".into();
    }
    let govde = xml_govde(&py_str(&ayar["usercode"]), &py_str(&ayar["password"]), &py_str(&ayar["msgheader"]), metin, &nolar);
    let istek = DisIstek {
        yontem: "POST".into(),
        url: URL.into(),
        basliklar: vec![("Content-Type".into(), "application/xml; charset=UTF-8".into())],
        govde: Some(govde.into_bytes()),
        kullanici: None,
        zaman_asimi: GENEL_SURE,
        okuma_siniri: OKUMA_SINIRI,
        yonlendirme: 0, // SMS: yönlendirme izlenmez
        indirme_sureleri: None,
    };
    let yanit = match t.gonder(&istek) {
        Ok((durum, g)) if durum < 400 => String::from_utf8_lossy(&g).into_owned(),
        _ => return "hata:AG".into(), // ağ, zaman aşımı, TLS, HTTP hatası: hepsi aynı sonuç
    };
    let kod = yanit_kodu(&yanit);
    if BASARI.contains(&kod.as_str()) {
        return "gonderildi".into();
    }
    if rakam_mi(&kod, 8) {
        format!("hata:{kod}")
    } else {
        "hata:BILINMIYOR".into()
    }
}

// --- Doğrulama (OTP) SMS'i: cihazların misafir kodu merkezden gider. Cihazdaki sms::build_xml / parse_response ile aynı
// biçim. Yeniden deneme yok; kod ve parola hiçbir yere yazılmaz.

fn xml_kacis(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

pub fn otp_xml(usercode: &str, password: &str, msgheader: &str, appkey: &str, metin: &str, telefon_10: &str) -> String {
    let ak = if appkey.is_empty() { String::new() } else { format!("<appkey>{}</appkey>", xml_kacis(appkey)) };
    format!(
        "<?xml version='1.0' encoding='utf-8'?>\n<mainbody><header><usercode>{}</usercode><password>{}</password>\
         <msgheader>{}</msgheader>{ak}</header><body><msg>{}</msg><no>{}</no></body></mainbody>",
        xml_kacis(usercode),
        xml_kacis(password),
        xml_kacis(msgheader),
        xml_kacis(metin),
        xml_kacis(telefon_10)
    )
}

fn alnum_mi(s: &str, en_cok: usize, ek: &[u8]) -> bool {
    !s.is_empty() && s.len() <= en_cok && s.bytes().all(|b| b.is_ascii_alphanumeric() || ek.contains(&b))
}

/// XML (`<code>`, `<jobID>`) ya da düz metin ("00 123456") → (kod, iş no); anlaşılmazsa ("PARSE", "").
pub fn otp_yaniti(metin: &str) -> (String, String) {
    let metin = py_strip(metin);
    let (kod, is_no) = match etiket(metin, "code") {
        Some(k) => (k, etiket(metin, "jobID").unwrap_or_default()),
        None => {
            let p = py_bol(metin);
            match p.first() {
                Some(ilk) if rakam_mi(ilk, usize::MAX) => (ilk.to_string(), p.get(1).map(|s| s.to_string()).unwrap_or_default()),
                _ => return ("PARSE".into(), String::new()),
            }
        }
    };
    if !alnum_mi(&kod, 16, b"") {
        return ("PARSE".into(), String::new());
    }
    let is_no = if alnum_mi(&is_no, 64, b"_-") { is_no } else { String::new() };
    (kod, is_no)
}

/// Dönen (sonuc, kod, is): sonuc `gonderildi` | `mock` | `hata`; kod NetGSM kodu ya da AYAR | AG | PARSE. Zaman aşımı
/// işlem başına (aşılırsa AG).
pub fn otp_gonder(ayar: &Value, telefon_10: &str, metin: &str, t: &dyn Tasiyici) -> (String, String, String) {
    let s = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
    if !py_dogru(ayar) {
        return s("hata", "AYAR", "");
    }
    if ayar.get("mock").is_some_and(py_dogru) {
        return s("mock", "0", "");
    }
    if !["usercode", "password", "msgheader"].iter().all(|k| ayar.get(*k).is_some_and(py_dogru)) {
        return s("hata", "AYAR", "");
    }
    let appkey = ayar.get("appkey").filter(|v| py_dogru(v)).map(py_str).unwrap_or_default();
    let govde = otp_xml(&py_str(&ayar["usercode"]), &py_str(&ayar["password"]), &py_str(&ayar["msgheader"]), &appkey, metin, telefon_10);
    let istek = DisIstek {
        yontem: "POST".into(),
        url: OTP_URL.into(),
        basliklar: vec![("Content-Type".into(), "application/xml; charset=UTF-8".into())],
        govde: Some(govde.into_bytes()),
        kullanici: None,
        zaman_asimi: ISLEM_SURE,
        okuma_siniri: OKUMA_SINIRI,
        yonlendirme: 0, // SMS: yönlendirme izlenmez
        indirme_sureleri: None,
    };
    let yanit = match t.gonder(&istek) {
        Ok((durum, g)) if durum < 400 => String::from_utf8_lossy(&g).into_owned(),
        _ => return s("hata", "AG", ""), // ağ, zaman aşımı, TLS, HTTP hatası
    };
    let (kod, is_no) = otp_yaniti(&yanit);
    if BASARI.contains(&kod.as_str()) {
        return ("gonderildi".into(), kod, is_no);
    }
    ("hata".into(), kod, String::new())
}

#[cfg(test)]
#[path = "netgsm_testler.rs"]
mod testler;
