//! Doğrulama SMS'i (spec 2026-10-13 "SMS merkezden"): cihaz sağlayıcıya (NetGSM / Twilio) hiç gitmez; kodu yönetim
//! merkezine (`POST /api/sms`) gönderir, merkez kendi şablonu ve kendi sağlayıcı ayarıyla iletir. Twilio Verify'da kodu
//! Twilio üretir; doğrulama da merkezden (`POST /api/sms-dogrula`). Yeniden deneme yok (çift SMS = çift ücret).

use crate::merkez::{Http, Merkez};
use std::path::Path;

/// Cihaz → merkez zaman aşımı (sn).
const MERKEZ_SURE: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Sonuc {
    pub ok: bool,
    /// Sağlayıcı kodu ("0" başarılı), merkez kodu (`SINIR`, `YABANCI`, `AYAR` …) ya da "AG" (merkeze ulaşılamadı / bozuk cevap).
    pub kod: String,
    pub is: Option<String>,
    /// Merkezin seçtiği sağlayıcı (cevapta yoksa NetGSM: denetimde `netgsm=<kod>`).
    pub saglayici: Saglayici,
    /// Merkez deneme modunda: SMS gitmedi, kod yerelde doğrulanır (Twilio seçili olsa da).
    pub mock: bool,
}

impl Sonuc {
    fn hata(kod: &str) -> Sonuc {
        Sonuc { ok: false, kod: kod.into(), is: None, saglayici: Saglayici::Netgsm, mock: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Saglayici {
    Netgsm,
    Twilio,
}

impl Saglayici {
    pub fn ad(self) -> &'static str {
        match self {
            Saglayici::Netgsm => "netgsm",
            Saglayici::Twilio => "twilio",
        }
    }
}

pub const YABANCI_YOK: &str = "Yabancı numaralara şu an SMS gönderilemiyor, lütfen personele başvurun.";
pub const GONDERILEMIYOR: &str = "Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin.";

fn merkez_bagi() -> Option<Merkez> {
    crate::merkez::oku(Path::new(crate::merkez::PATH))
}

fn merkez_http(yol: &str, govde: &str) -> Result<(u16, String), String> {
    crate::merkez::curl_sure(yol, govde, MERKEZ_SURE)
}

/// Gerçek gönderim (portal): `merkez.json` + curl (10 sn).
pub fn send(phone: &str, code: &str) -> Sonuc {
    gonder(merkez_bagi().as_ref(), phone, code, &merkez_http)
}

/// Gerçek Twilio doğrulaması (portal): (onaylandı, hata). Hata "AG" ise misafirin deneme hakkı düşmez.
pub fn check(phone: &str, code: &str) -> (bool, Option<String>) {
    dogrula(merkez_bagi().as_ref(), phone, code, &merkez_http)
}

/// Kodu merkeze gönderir. Merkeze bağlı değilse, ulaşılamazsa ya da cevap bozuksa `AG`. Numara cihazda süzülmez:
/// hangi numaraya gönderilebileceğine merkez karar verir (`YABANCI`).
pub fn gonder(m: Option<&Merkez>, phone: &str, code: &str, http: &Http) -> Sonuc {
    let Some(m) = m else { return Sonuc::hata("AG") };
    let c = match crate::merkez::sms_gonder(m, phone, code, http) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SMS: {e}"); // kod ve cihaz anahtarı yazılmaz
            return Sonuc::hata("AG");
        }
    };
    let saglayici = if c.saglayici == "twilio" { Saglayici::Twilio } else { Saglayici::Netgsm };
    match c.sonuc.as_str() {
        "mock" => {
            eprintln!("MOCK OTP {phone} -> {code}");
            Sonuc { ok: true, kod: "0".into(), is: Some("MOCK".into()), saglayici, mock: true }
        }
        "gonderildi" => Sonuc { ok: true, kod: c.kod, is: c.is, saglayici, mock: false },
        _ => Sonuc { ok: false, kod: if c.kod.is_empty() { "AG".into() } else { c.kod }, is: None, saglayici, mock: false },
    }
}

/// Twilio kodunu merkez üzerinden doğrular: (onaylandı, hata). Onay/ret dışındaki her şey "AG" (deneme hakkı düşmez).
pub fn dogrula(m: Option<&Merkez>, phone: &str, code: &str, http: &Http) -> (bool, Option<String>) {
    let Some(m) = m else { return (false, Some("AG".into())) };
    match crate::merkez::sms_dogrula(m, phone, code, http) {
        Ok(Some(onay)) => (onay, None),
        Ok(None) => (false, Some("AG".into())),
        Err(e) => {
            eprintln!("SMS doğrulama: {e}");
            (false, Some("AG".into()))
        }
    }
}

/// Misafire gösterilen ileti (Türkçe; `dil::cevir` ile çevrilir).
pub fn message_for(p: Saglayici, kod: &str) -> &'static str {
    match kod {
        "YABANCI" => YABANCI_YOK,
        _ => match p {
            Saglayici::Netgsm => user_message(kod),
            Saglayici::Twilio => twilio_message(kod),
        },
    }
}

/// NetGSM kodları (merkez sağlayıcının kodunu olduğu gibi döner).
pub fn user_message(kod: &str) -> &'static str {
    match kod {
        "50" => "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.",
        "80" => "Çok fazla deneme yapıldı, lütfen biraz sonra tekrar deneyin.",
        "100" => "SMS servisi geçici olarak yanıt vermiyor, lütfen biraz sonra tekrar deneyin.",
        "AG" | "SINIR" | "PARSE" => GONDERILEMIYOR,
        _ => "SMS gönderilemedi, lütfen personele başvurun.",
    }
}

/// Twilio Verify hata kodları (merkez Twilio'nun kodunu olduğu gibi döner).
pub fn twilio_message(kod: &str) -> &'static str {
    match kod {
        "60200" | "21211" | "60205" => "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.",
        "60203" | "20429" => "Çok fazla deneme yapıldı, lütfen biraz sonra tekrar deneyin.",
        "AG" | "SINIR" | "PARSE" => GONDERILEMIYOR,
        _ => "SMS gönderilemedi, lütfen personele başvurun.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Cagrilar = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    /// Sahte merkez: tek cevap; çağrıları kaydeder.
    fn sahte(cevap: Result<(u16, &'static str), &'static str>) -> (Box<Http>, Cagrilar) {
        let c = Arc::new(Mutex::new(vec![]));
        let c2 = c.clone();
        let f = move |yol: &str, govde: &str| {
            c2.lock().unwrap().push((yol.to_string(), serde_json::from_str(govde).unwrap()));
            cevap.map(|(d, g)| (d, g.to_string())).map_err(String::from)
        };
        (Box::new(f), c)
    }

    fn bag() -> Merkez {
        Merkez { numara: "4511643".into(), cihaz_anahtari: "k-1".into(), ..Default::default() }
    }

    #[test]
    fn sms_basari_is_numarasi_ve_istek() {
        let (h, c) = sahte(Ok((200, r#"{"sonuc":"gonderildi","kod":"0","is":"J9","saglayici":"netgsm"}"#)));
        let r = gonder(Some(&bag()), "905334553132", "123456", &*h);
        assert_eq!(r, Sonuc { ok: true, kod: "0".into(), is: Some("J9".into()), saglayici: Saglayici::Netgsm, mock: false });
        let c = c.lock().unwrap();
        assert_eq!(c[0].0, "/api/sms");
        assert_eq!(c[0].1, serde_json::json!({"cihaz_anahtari": "k-1", "telefon": "905334553132", "kod": "123456"}));
    }

    #[test]
    fn sms_mock_twilio_olsa_da_yerel() {
        let (h, _) = sahte(Ok((200, r#"{"sonuc":"mock","kod":"0","saglayici":"twilio"}"#)));
        let r = gonder(Some(&bag()), "4915123456789", "123456", &*h);
        assert_eq!(r, Sonuc { ok: true, kod: "0".into(), is: Some("MOCK".into()), saglayici: Saglayici::Twilio, mock: true });
    }

    #[test]
    fn sms_hatalari() {
        for (cevap, kod, saglayici) in [
            (Err("ağ"), "AG", Saglayici::Netgsm),
            (Ok((200, "<html>bozuk</html>")), "AG", Saglayici::Netgsm),
            (Ok((500, r#"{"hata":"x"}"#)), "AG", Saglayici::Netgsm),
            (Ok((403, r#"{"hata":"hizmet kapali"}"#)), "AG", Saglayici::Netgsm),
            (Ok((200, r#"{"sonuc":"bilinmeyen","kod":"0"}"#)), "AG", Saglayici::Netgsm),
            (Ok((200, r#"{"sonuc":"hata","kod":"SINIR","saglayici":"netgsm"}"#)), "SINIR", Saglayici::Netgsm),
            (Ok((200, r#"{"sonuc":"hata","kod":"YABANCI"}"#)), "YABANCI", Saglayici::Netgsm),
            (Ok((200, r#"{"sonuc":"hata","kod":"50","saglayici":"netgsm"}"#)), "50", Saglayici::Netgsm),
            (Ok((200, r#"{"sonuc":"hata","kod":"60200","saglayici":"twilio"}"#)), "60200", Saglayici::Twilio),
        ] {
            let (h, _) = sahte(cevap);
            let r = gonder(Some(&bag()), "905334553132", "123456", &*h);
            assert_eq!((r.ok, r.kod.as_str(), r.saglayici, r.mock), (false, kod, saglayici, false), "{cevap:?}");
        }
        // merkeze bağlı değil: istek yok
        let (h, c) = sahte(Err("çağrılmamalı"));
        assert_eq!(gonder(None, "905334553132", "123456", &*h).kod, "AG");
        assert!(c.lock().unwrap().is_empty());
    }

    #[test]
    fn sms_dogrulama_merkezden() {
        let (h, c) = sahte(Ok((200, r#"{"onay":true}"#)));
        assert_eq!(dogrula(Some(&bag()), "4915123456789", "424242", &*h), (true, None));
        assert_eq!(c.lock().unwrap()[0].0, "/api/sms-dogrula");
        assert_eq!(c.lock().unwrap()[0].1, serde_json::json!({"cihaz_anahtari": "k-1", "telefon": "4915123456789", "kod": "424242"}));
        let (h, _) = sahte(Ok((200, r#"{"onay":false}"#)));
        assert_eq!(dogrula(Some(&bag()), "4915123456789", "111111", &*h), (false, None));
        for cevap in [Ok((200, r#"{"sonuc":"hata","kod":"AG"}"#)), Ok((200, r#"{"sonuc":"hata","kod":"AYAR"}"#)), Ok((200, "bozuk")), Err("ağ")] {
            let (h, _) = sahte(cevap);
            assert_eq!(dogrula(Some(&bag()), "4915123456789", "111111", &*h), (false, Some("AG".into())), "{cevap:?}");
        }
        let (h, _) = sahte(Err("çağrılmamalı"));
        assert_eq!(dogrula(None, "4915123456789", "111111", &*h), (false, Some("AG".into())));
    }

    #[test]
    fn sms_iletileri() {
        for k in ["AG", "SINIR", "PARSE"] {
            assert_eq!(message_for(Saglayici::Netgsm, k), "Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin.");
            assert_eq!(message_for(Saglayici::Twilio, k), GONDERILEMIYOR);
        }
        assert_eq!(message_for(Saglayici::Netgsm, "YABANCI"), YABANCI_YOK);
        assert_eq!(message_for(Saglayici::Netgsm, "50"), "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.");
        assert_eq!(message_for(Saglayici::Netgsm, "30"), "SMS gönderilemedi, lütfen personele başvurun.");
        assert_eq!(message_for(Saglayici::Twilio, "60200"), "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.");
        assert_eq!(message_for(Saglayici::Twilio, "x"), "SMS gönderilemedi, lütfen personele başvurun.");
    }

    #[test]
    fn sms_saglayiciya_dogrudan_istek_yok() {
        // cihaz kodunda sağlayıcı HTTP istemcisi ve adresleri kalmadı (ölü kod / sırların cihazda kullanılması)
        let kaynaklar = [include_str!("sms.rs"), include_str!("portal.rs"), include_str!("merkez.rs"), include_str!("panel.rs"), include_str!("ayar.rs")];
        for k in kaynaklar {
            for yasak in [concat!("ureq", "::"), concat!("api.netgsm", ".com.tr"), concat!("verify.twilio", ".com")] {
                assert!(!k.contains(yasak), "{yasak}");
            }
        }
    }
}
