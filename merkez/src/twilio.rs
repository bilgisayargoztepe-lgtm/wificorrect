//! Twilio Verify (Python twilio.py). Bu görevde yalnızca ayar doğrulayıcıları; ağ kısmı Task 8'de eklenir.
use serde_json::Value;

use crate::ortak::{py_dogru, py_str};

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
