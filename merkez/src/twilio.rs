//! Twilio Verify (Python twilio.py; cihazdaki twilio.rs'in merkez karşılığı): kodu Twilio üretir ve doğrular; merkez
//! yalnızca başlatır ve sorar. API: POST /v2/Services/{verify_sid}/Verifications (To, Channel=sms, Locale) ve
//! /VerificationCheck (To, Code). Kimlik: API anahtarı (sid + secret) ikisi de girilmişse o, değilse account SID + auth
//! token (Basic; curl'e `user =` olarak stdin'den gider). Yeniden deneme yok; kimlik bilgisi, kod ve istek gövdesi hiçbir
//! yere yazılmaz.
use std::time::Duration;

use serde_json::Value;

use crate::ortak::{py_dogru, py_str, DisIstek, Tasiyici};

pub const BASE: &str = "https://verify.twilio.com/v2/Services";
/// İşlem başına zaman aşımı; çağrı bitmeden dönülmez, cihaz 15 sn bekler.
pub const ISLEM_SURE: Duration = Duration::from_secs(5);
const OKUMA_SINIRI: usize = 16384;

/// Alan adı → SID öneki (Python `SID_RE`, bu sırayla): önek + 32 onaltılık hane, tam eşleşme.
pub const SID_ALANLARI: [(&str, &str); 3] = [("account_sid", "AC"), ("api_key_sid", "SK"), ("verify_sid", "VA")];

/// Python `SID_RE[alan].fullmatch(s)`.
pub fn sid_gecerli(alan: &str, s: &str) -> bool {
    let Some((_, onek)) = SID_ALANLARI.iter().find(|(a, _)| *a == alan) else {
        return false;
    };
    s.len() == 34 && s.starts_with(onek) && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// (kullanıcı, sır) ya da None (eksik ayar): API anahtarı ikilisi önce, sonra account SID + auth token.
pub fn kimlik(ayar: &Value) -> Option<(String, String)> {
    let al = |k: &str| ayar.get(k).filter(|v| py_dogru(v)).map(py_str);
    if let (Some(a), Some(b)) = (al("api_key_sid"), al("api_key_secret")) {
        return Some((a, b));
    }
    if let (Some(a), Some(b)) = (al("account_sid"), al("auth_token")) {
        return Some((a, b));
    }
    None
}

/// Türk cep numarası: 90 + 5 ile başlayan 12 hane.
pub fn tr_mi(telefon: &str) -> bool {
    telefon.chars().count() == 12 && telefon.starts_with("905")
}

/// `urllib.parse.urlencode` (quote_plus).
fn urlencode(alanlar: &[(&str, &str)]) -> String {
    let kac = |s: &str| {
        let mut o = String::new();
        for &b in s.as_bytes() {
            match b {
                b if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'~') => o.push(b as char),
                b' ' => o.push('+'),
                b => o.push_str(&format!("%{b:02X}")),
            }
        }
        o
    };
    alanlar.iter().map(|(k, v)| format!("{}={}", kac(k), kac(v))).collect::<Vec<_>>().join("&")
}

enum Istek {
    /// Eksik/bozuk ayar.
    Ayar,
    /// Ağ hatası, zaman aşımı.
    Ag,
    /// (HTTP durumu, JSON gövde; nesne değilse ya da çözülemezse boş nesne).
    Yanit(u16, Value),
}

fn istek(ayar: &Value, yol: &str, alanlar: &[(&str, &str)], t: &dyn Tasiyici) -> Istek {
    let bos = Value::Null;
    let ayar = if py_dogru(ayar) { ayar } else { &bos };
    let verify = ayar.get("verify_sid").filter(|v| py_dogru(v)).map(py_str).unwrap_or_default();
    let Some(k) = kimlik(ayar).filter(|_| sid_gecerli("verify_sid", &verify)) else {
        return Istek::Ayar;
    };
    let i = DisIstek {
        yontem: "POST".into(),
        url: format!("{BASE}/{verify}/{yol}"),
        basliklar: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())],
        govde: Some(urlencode(alanlar).into_bytes()),
        kullanici: Some(k),
        zaman_asimi: ISLEM_SURE,
        okuma_siniri: OKUMA_SINIRI,
        yonlendirme: 0, // SMS: yönlendirme izlenmez
        indirme_sureleri: None,
    };
    let Ok((durum, govde)) = t.gonder(&i) else { return Istek::Ag };
    let j = match serde_json::from_slice::<Value>(&govde) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(Default::default()),
    };
    Istek::Yanit(durum, j)
}

/// Doğrulamayı başlatır. `telefon_e164`: + olmadan. Dönen (sonuc, kod): gonderildi/0, mock/0, hata/<Twilio kodu|HTTP|AYAR|AG>.
pub fn gonder(ayar: &Value, telefon_e164: &str, t: &dyn Tasiyici) -> (String, String) {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    if ayar.get("mock").is_some_and(py_dogru) {
        return s("mock", "0");
    }
    let yerel = if tr_mi(telefon_e164) { "tr" } else { "en" };
    let to = format!("+{telefon_e164}");
    match istek(ayar, "Verifications", &[("To", &to), ("Channel", "sms"), ("Locale", yerel)], t) {
        Istek::Ayar => s("hata", "AYAR"),
        Istek::Ag => s("hata", "AG"),
        Istek::Yanit(durum, j) => {
            if durum < 300 && j.get("status").and_then(Value::as_str) == Some("pending") {
                return s("gonderildi", "0");
            }
            // `type(kod) is int`: bool ve kayan nokta int değildir
            match j.get("code") {
                Some(Value::Number(n)) if n.is_i64() || n.is_u64() => ("hata".into(), n.to_string()),
                _ => ("hata".into(), durum.to_string()),
            }
        }
    }
}

/// (onay, kod): onay Some(true/false); Twilio'ya ulaşılamazsa ya da Twilio geçici hata veriyorsa (5xx, 429) (None, "AG"),
/// ayar eksik ya da kimlik reddedildiyse (401/403) (None, "AYAR") — ikisinde de misafirin deneme hakkı düşmez. Yanlış,
/// süresi dolmuş ya da iptal edilmiş kod (pending/canceled, 404 ve diğer 4xx) → (Some(false), "0").
pub fn dogrula(ayar: &Value, telefon_e164: &str, kod: &str, t: &dyn Tasiyici) -> (Option<bool>, String) {
    let to = format!("+{telefon_e164}");
    match istek(ayar, "VerificationCheck", &[("To", &to), ("Code", kod)], t) {
        Istek::Ayar => (None, "AYAR".into()),
        Istek::Ag => (None, "AG".into()),
        Istek::Yanit(durum, j) => {
            if durum >= 500 || durum == 429 {
                return (None, "AG".into());
            }
            if durum == 401 || durum == 403 {
                return (None, "AYAR".into());
            }
            (Some(durum < 300 && j.get("status").and_then(Value::as_str) == Some("approved")), "0".into())
        }
    }
}

#[cfg(test)]
#[path = "twilio_testler.rs"]
mod net_testler;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sid_desenleri() {
        let ok = format!("AC{}", "aF09".repeat(8));
        assert!(sid_gecerli("account_sid", &ok));
        assert!(!sid_gecerli("verify_sid", &ok));
        assert!(!sid_gecerli("account_sid", &format!("{ok}0")));
        assert!(!sid_gecerli("account_sid", &format!("AC{}g", "a".repeat(31))));
        assert!(!sid_gecerli("account_sid", &format!("{ok}\n")));
        assert!(sid_gecerli("verify_sid", &format!("VA{}", "0".repeat(32))));
        assert!(!sid_gecerli("bilinmeyen", &ok));
    }

    #[test]
    fn kimlik_sirasi() {
        assert_eq!(kimlik(&json!({})), None);
        assert_eq!(kimlik(&json!({"account_sid": "AC1", "auth_token": "t"})), Some(("AC1".into(), "t".into())));
        assert_eq!(
            kimlik(&json!({"account_sid": "AC1", "auth_token": "t", "api_key_sid": "SK1", "api_key_secret": "s"})),
            Some(("SK1".into(), "s".into()))
        );
        assert_eq!(kimlik(&json!({"api_key_sid": "SK1", "account_sid": "AC1", "auth_token": "t"})), Some(("AC1".into(), "t".into())));
        assert_eq!(kimlik(&json!({"account_sid": "AC1", "auth_token": ""})), None);
    }
}
