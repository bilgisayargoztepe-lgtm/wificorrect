//! netgsm.rs testleri: Python tests/test_sms_merkez.py → test_netgsm_otp_xml_ve_cevap, test_saglayici_cagrisi_bitmeden_donulmez
//! (NetGSM kısmı). Gerçek istek gitmez: sahte taşıyıcı (WC_TEST altında `Curl` reddi `ortak` testlerinde).
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{AgCevap, AgYok, SahteAg};

fn netgsm() -> Value {
    json!({"mock": false, "usercode": "8500000000", "password": "netgsm-gizli", "msgheader": "BASLIK", "appkey": ""})
}

fn s(a: &str, b: &str, c: &str) -> (String, String, String) {
    (a.to_string(), b.to_string(), c.to_string())
}

#[test]
fn otp_xml_ve_cevap() {
    let _k = Kayit::bas("test_sms_merkez", "test_netgsm_otp_xml_ve_cevap");
    let ag = SahteAg::yeni(vec![AgCevap::Govde("<?xml version='1.0'?><xml><main><code>0</code><jobID>17283946</jobID></main></xml>")]);
    let metin = "WiFi dogrulama kodunuz: 123456. Kod 3 dakika gecerlidir.";
    let mut ayar = netgsm();
    ayar["password"] = json!("p<&>");
    ayar["appkey"] = json!("k1");
    assert_eq!(otp_gonder(&ayar, "5334553132", metin, ag.as_ref()), s("gonderildi", "0", "17283946"));
    ag.son(|r| {
        assert!(r.url == "https://api.netgsm.com.tr/sms/send/otp" && r.yontem == "POST" && r.zaman_asimi == Duration::from_secs(12));
        assert_eq!(r.asama_sureleri, Some((Duration::from_secs(7), Duration::from_secs(5))));
        assert_eq!(r.basliklar, vec![("Content-Type".to_string(), "application/xml; charset=UTF-8".to_string())]);
        assert!(r.kullanici.is_none());
        assert_eq!(
            r.govde,
            format!(
                "<?xml version='1.0' encoding='utf-8'?>\n<mainbody><header><usercode>8500000000</usercode>\
                 <password>p&lt;&amp;&gt;</password><msgheader>BASLIK</msgheader><appkey>k1</appkey></header>\
                 <body><msg>{metin}</msg><no>5334553132</no></body></mainbody>"
            )
        );
    });
    assert!(!otp_xml("u", "p", "h", "", "m", "5").contains("<appkey>"));
    let tek = |c: AgCevap| otp_gonder(&netgsm(), "5334553132", metin, SahteAg::yeni(vec![c]).as_ref());
    assert_eq!(tek(AgCevap::Govde("00 998877")), s("gonderildi", "00", "998877"));
    assert_eq!(tek(AgCevap::Govde("<xml><main><code>30</code></main></xml>")), s("hata", "30", ""));
    assert_eq!(tek(AgCevap::Govde("<html>bozuk")), s("hata", "PARSE", ""));
    assert_eq!(tek(AgCevap::Hata), s("hata", "AG", ""));
    let mut mock = netgsm();
    mock["mock"] = json!(true);
    assert_eq!(otp_gonder(&mock, "5334553132", metin, &AgYok), s("mock", "0", ""));
    let mut parolasiz = netgsm();
    parolasiz["password"] = json!("");
    assert_eq!(otp_gonder(&parolasiz, "5334553132", metin, &AgYok), s("hata", "AYAR", ""));
    assert_eq!(otp_gonder(&Value::Null, "5334553132", metin, &AgYok), s("hata", "AYAR", ""));
}

#[test]
fn saglayici_cagrisi_netgsm_bitmeden_donulmez() {
    let _k = Kayit::bas("test_sms_merkez", "test_saglayici_cagrisi_bitmeden_donulmez");
    // Python soket işlemi başına 5 sn; Rust: bağlanma 7, boşta 5, toplam 12 (< cihazın 15 sn beklemesi)
    assert_eq!((OTP_BAGLANMA, OTP_BOSTA, ISLEM_SURE), (Duration::from_secs(7), Duration::from_secs(5), Duration::from_secs(12)));
    let metin = "WiFi dogrulama kodunuz: 123456. Kod 3 dakika gecerlidir.";
    let ag = SahteAg::yeni(vec![AgCevap::Govde("<xml><main><code>0</code><jobID>77</jobID></main></xml>")]);
    assert_eq!(otp_gonder(&netgsm(), "5334553132", metin, ag.as_ref()), s("gonderildi", "0", "77"));
    ag.son(|r| assert_eq!((r.zaman_asimi, r.asama_sureleri), (Duration::from_secs(12), Some((Duration::from_secs(7), Duration::from_secs(5))))));
    assert_eq!(otp_gonder(&netgsm(), "5334553132", metin, SahteAg::yeni(vec![AgCevap::Hata]).as_ref()), s("hata", "AG", ""));
}

#[test]
fn http_hatasi_ag_sayilir() {
    let metin = "m";
    let ag = SahteAg::yeni(vec![AgCevap::Durum(503, "<xml><main><code>0</code></main></xml>")]);
    assert_eq!(otp_gonder(&netgsm(), "5334553132", metin, ag.as_ref()), s("hata", "AG", ""));
    let ag = SahteAg::yeni(vec![AgCevap::Durum(400, "00")]);
    assert_eq!(gonder(&netgsm(), "m", &["5334553132"], ag.as_ref()), "hata:AG");
}

#[test]
fn genel_sms_gonder() {
    let ag = SahteAg::yeni(vec![]);
    assert_eq!(gonder(&netgsm(), "Uyarı <test>", &["+90 533 455 31 32", "05334553132", "kötü"], ag.as_ref()), "gonderildi");
    ag.son(|r| {
        assert!(r.url == URL && r.zaman_asimi == GENEL_SURE && r.kullanici.is_none());
        assert_eq!(GENEL_SURE, Duration::from_secs(25));
        assert_eq!(r.asama_sureleri, Some((Duration::from_secs(10), Duration::from_secs(10))));
        assert_eq!(
            r.govde,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><mainbody><header><company dil=\"TR\">Netgsm</company>\
             <usercode>8500000000</usercode><password>netgsm-gizli</password><type>1:n</type><msgheader>BASLIK</msgheader></header>\
             <body><msg><![CDATA[Uyarı <test>]]></msg><no>5334553132</no></body></mainbody>"
        );
    });
    let tek = |c: AgCevap| gonder(&netgsm(), "m", &["5334553132"], SahteAg::yeni(vec![c]).as_ref());
    assert_eq!(tek(AgCevap::Govde("00 123456")), "gonderildi");
    assert_eq!(tek(AgCevap::Govde("<xml><code> 40 </code></xml>")), "hata:40");
    assert_eq!(tek(AgCevap::Govde("<html>bozuk")), "hata:BILINMIYOR");
    assert_eq!(tek(AgCevap::Govde("")), "hata:BILINMIYOR");
    assert_eq!(tek(AgCevap::Hata), "hata:AG");
    let mut mock = netgsm();
    mock["mock"] = json!(true);
    assert_eq!(gonder(&mock, "m", &["5334553132"], &AgYok), "mock");
    assert_eq!(gonder(&Value::Null, "m", &["5334553132"], &AgYok), "hata:AYAR");
    assert_eq!(gonder(&json!({}), "m", &["5334553132"], &AgYok), "hata:AYAR");
    assert_eq!(gonder(&netgsm(), "m", &["123", "4321234567"], &AgYok), "hata:NUMARA");
    assert_eq!(gonder(&netgsm(), "m", &[] as &[&str], &AgYok), "hata:NUMARA");
}

#[test]
fn yardimcilar() {
    assert_eq!(numaralar_10(&["905334553132", "0533 455 31 32", "5334553132", "x", "5443332211"]), ["5334553132", "5443332211"]);
    assert_eq!(yanit_kodu("  <a><code>\n 30\n</code> "), "30");
    assert_eq!(yanit_kodu("00 123456"), "00");
    assert_eq!(yanit_kodu(""), "");
    assert_eq!(yanit_kodu("<code>a b</code> 77"), "<code>a");
    assert_eq!(yanit_kodu("<code>1 2</code><code>5</code>"), "5");
    assert_eq!(yanit_kodu("<code></code>"), "");
    // CDATA ve denetim karakterleri
    let x = xml_govde("u&", "p<", "h>", "a]]>b\u{1}c\u{9}", &["5334553132"]);
    assert!(x.contains("<usercode>u&amp;</usercode><password>p&lt;</password>") && x.contains("<msgheader>h&gt;</msgheader>"));
    assert!(x.contains("<![CDATA[a]]]]><![CDATA[>bc\u{9}]]>"));
    assert_eq!(otp_yaniti("00 12"), ("00".into(), "12".into()));
    assert_eq!(otp_yaniti("00"), ("00".into(), String::new()));
    assert_eq!(otp_yaniti("<code>0</code><jobID>a b</jobID>"), ("0".into(), String::new()));
    assert_eq!(otp_yaniti("<code>0123456789abcdefg</code>"), ("PARSE".into(), String::new()));
    assert_eq!(otp_yaniti("<code></code>"), ("PARSE".into(), String::new()));
    assert_eq!(otp_yaniti("ab 1"), ("PARSE".into(), String::new()));
}
