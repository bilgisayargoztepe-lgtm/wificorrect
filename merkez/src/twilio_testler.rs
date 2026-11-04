//! twilio.rs ağ kısmı testleri: Python tests/test_sms_merkez.py → test_twilio_gonder_ve_dogrula,
//! test_twilio_dogrula_durum_eslemesi, test_saglayici_cagrisi_bitmeden_donulmez (Twilio kısmı). Gerçek istek gitmez.
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{AgCevap, AgYok, SahteAg};

fn tw() -> Value {
    json!({"acik": true, "account_sid": format!("AC{}", "a".repeat(32)), "auth_token": "tw-token-gizli", "api_key_sid": "",
           "api_key_secret": "", "verify_sid": format!("VA{}", "b".repeat(32))})
}

const KOD: &str = "482913";
const TEL: &str = "905334553132";

fn g(a: &str, b: &str) -> (String, String) {
    (a.to_string(), b.to_string())
}

#[test]
fn gonder_ve_dogrula() {
    let _k = Kayit::bas("test_sms_merkez", "test_twilio_gonder_ve_dogrula");
    let ag = SahteAg::yeni(vec![AgCevap::Durum(201, r#"{"status": "pending", "sid": "VE123"}"#)]);
    assert_eq!(gonder(&tw(), TEL, ag.as_ref()), g("gonderildi", "0"));
    ag.son(|r| {
        assert_eq!(r.url, format!("https://verify.twilio.com/v2/Services/VA{}/Verifications", "b".repeat(32)));
        assert!(r.yontem == "POST" && r.zaman_asimi == Duration::from_secs(5));
        assert_eq!(r.govde, "To=%2B905334553132&Channel=sms&Locale=tr");
        // Basic kimlik curl'e `user =` olarak (stdin) gider; başlıkta ve komut satırında değil
        assert_eq!(r.kullanici, Some((format!("AC{}", "a".repeat(32)), "tw-token-gizli".to_string())));
        assert_eq!(r.basliklar, vec![("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())]);
    });
    let ag = SahteAg::yeni(vec![AgCevap::Durum(201, r#"{"status": "pending"}"#)]);
    let mut anahtarli = tw();
    anahtarli["api_key_sid"] = json!(format!("SK{}", "c".repeat(32)));
    anahtarli["api_key_secret"] = json!("sk-gizli");
    assert_eq!(gonder(&anahtarli, "4915123456789", ag.as_ref()), g("gonderildi", "0"));
    ag.son(|r| {
        assert!(r.govde.ends_with("Locale=en"));
        assert_eq!(r.kullanici, Some((format!("SK{}", "c".repeat(32)), "sk-gizli".to_string())));
    });
    let tek = |c: AgCevap| gonder(&tw(), TEL, SahteAg::yeni(vec![c]).as_ref());
    assert_eq!(tek(AgCevap::Durum(400, r#"{"code": 60200, "message": "Invalid"}"#)), g("hata", "60200"));
    assert_eq!(tek(AgCevap::Durum(503, "bozuk")), g("hata", "503"));
    assert_eq!(tek(AgCevap::Hata), g("hata", "AG"));
    let mut kotu_sid = tw();
    kotu_sid["verify_sid"] = json!("../x");
    assert_eq!(gonder(&kotu_sid, TEL, &AgYok), g("hata", "AYAR"));
    let mut tokensiz = tw();
    tokensiz["auth_token"] = json!("");
    assert_eq!(gonder(&tokensiz, TEL, &AgYok), g("hata", "AYAR"));
    // doğrulama
    let ag = SahteAg::yeni(vec![AgCevap::Durum(200, r#"{"status": "approved"}"#)]);
    assert_eq!(dogrula(&tw(), TEL, KOD, ag.as_ref()), (Some(true), "0".to_string()));
    ag.son(|r| {
        assert!(r.url.ends_with("/VerificationCheck"));
        assert_eq!(r.govde, "To=%2B905334553132&Code=482913");
    });
    let dtek = |c: AgCevap| dogrula(&tw(), TEL, KOD, SahteAg::yeni(vec![c]).as_ref());
    assert_eq!(dtek(AgCevap::Durum(200, r#"{"status": "pending"}"#)), (Some(false), "0".to_string()));
    assert_eq!(dtek(AgCevap::Durum(404, r#"{"code": 20404}"#)), (Some(false), "0".to_string())); // süresi dolmuş
    assert_eq!(dtek(AgCevap::Hata), (None, "AG".to_string()));
}

#[test]
fn dogrula_durum_eslemesi() {
    let _k = Kayit::bas("test_sms_merkez", "test_twilio_dogrula_durum_eslemesi");
    let ag = (None, "AG");
    let ayar = (None, "AYAR");
    let onay = (Some(true), "0");
    let yanlis = (Some(false), "0");
    for (cevap, beklenen) in [
        (AgCevap::Durum(500, "{}"), ag),
        (AgCevap::Durum(503, "bozuk"), ag),
        (AgCevap::Durum(429, r#"{"code": 20429}"#), ag),
        (AgCevap::Durum(401, r#"{"code": 20003}"#), ayar),
        (AgCevap::Durum(403, "{}"), ayar),
        (AgCevap::Durum(200, r#"{"status": "approved"}"#), onay),
        (AgCevap::Durum(201, r#"{"status": "approved"}"#), onay),
        (AgCevap::Durum(200, r#"{"status": "pending"}"#), yanlis),
        (AgCevap::Durum(200, r#"{"status": "canceled"}"#), yanlis),
        (AgCevap::Durum(200, "bozuk"), yanlis),
        (AgCevap::Durum(404, r#"{"code": 20404}"#), yanlis),
        (AgCevap::Durum(400, r#"{"code": 60200}"#), yanlis),
        (AgCevap::Durum(409, "{}"), yanlis),
    ] {
        let r = dogrula(&tw(), TEL, KOD, SahteAg::yeni(vec![cevap]).as_ref());
        assert_eq!((r.0, r.1.as_str()), beklenen);
    }
}

#[test]
fn saglayici_cagrisi_twilio_bitmeden_donulmez() {
    let _k = Kayit::bas("test_sms_merkez", "test_saglayici_cagrisi_bitmeden_donulmez");
    assert_eq!(ISLEM_SURE, Duration::from_secs(5));
    let ag = SahteAg::yeni(vec![AgCevap::Govde(r#"{"status": "pending"}"#), AgCevap::Govde(r#"{"status": "approved"}"#)]);
    assert_eq!(gonder(&tw(), TEL, ag.as_ref()), g("gonderildi", "0"));
    assert_eq!(dogrula(&tw(), TEL, KOD, ag.as_ref()), (Some(true), "0".to_string()));
    assert!(ag.istekler.lock().unwrap().iter().all(|r| r.zaman_asimi == Duration::from_secs(5)));
}

#[test]
fn tr_mi_ve_mock() {
    assert!(tr_mi("905334553132") && !tr_mi("902161234567") && !tr_mi("90533455313") && !tr_mi("4915123456789"));
    let mut mock = tw();
    mock["mock"] = json!(true);
    assert_eq!(gonder(&mock, TEL, &AgYok), g("mock", "0"));
    assert_eq!(gonder(&Value::Null, TEL, &AgYok), g("hata", "AYAR"));
    // kod sayı değilse (bool, kayan nokta, metin) HTTP durumu yazılır
    for kod in [json!(true), json!(1.5), json!("60200")] {
        let govde = json!({"code": kod}).to_string();
        let ag = SahteAg::yeni(vec![]);
        ag.ekle(AgCevap::Durum(400, Box::leak(govde.into_boxed_str())));
        assert_eq!(gonder(&tw(), TEL, ag.as_ref()), g("hata", "400"));
    }
}
