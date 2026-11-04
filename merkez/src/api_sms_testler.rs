//! /api/sms ve /api/sms-dogrula testleri: Python tests/test_sms_merkez.py (24 testten 21'i; yönetim sayfası kullanan
//! üçü Task 11'e). Sahte taşıyıcı (`SahteAg`): gerçek NetGSM/Twilio isteği asla gitmez.
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{self as Y, AgCevap, SahteAg, SahteSaat};

const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c";
const KOD: &str = "482913";
const TEL: &str = "905334553132";
const IP: &str = "198.51.100.7";
const BASARI: &str = "<xml><main><code>0</code><jobID>555</jobID></main></xml>";
const PENDING: &str = r#"{"status": "pending"}"#;

fn netgsm() -> Value {
    json!({"mock": false, "usercode": "8500000000", "password": "netgsm-gizli", "msgheader": "BASLIK", "appkey": ""})
}

fn tw() -> Value {
    json!({"acik": true, "account_sid": format!("AC{}", "a".repeat(32)), "auth_token": "tw-token-gizli", "api_key_sid": "",
           "api_key_secret": "", "verify_sid": format!("VA{}", "b".repeat(32))})
}

/// Python `dict(taban, **ek)`.
fn ile(taban: &Value, ek: Value) -> Value {
    let mut t = taban.clone();
    for (k, v) in ek.as_object().unwrap() {
        t[k] = v.clone();
    }
    t
}

struct D {
    api: Api,
    v: Arc<Veri>,
    n: i64,
    anahtar: String,
    saat: SahteSaat,
    dizin: PathBuf,
}

fn yeni_api(o: &Y::Ortam, ag: Option<&Arc<SahteAg>>) -> Api {
    let mut api = Api::new(
        o.veri.clone(),
        o.saat.saat(),
        Box::new(|_, _| panic!("kuyruk kullanılmamalı")),
        Box::new(|_| panic!("kuyruk kullanılmamalı")),
    );
    if let Some(a) = ag {
        api.sms_tasiyici = a.clone();
    }
    api
}

/// Python `kur(tmp, ag, sms, twilio, tr)`.
fn kur(ag: Option<&Arc<SahteAg>>, sms: Option<Value>, twilio: Option<Value>, tr: &str) -> D {
    let o = Y::ortam();
    if let Some(s) = sms {
        o.veri.sms_ayari_koy(&ile(&s, json!({"tr_saglayici": tr, "twilio": twilio.unwrap_or(json!({}))}))).unwrap();
    }
    let (n, _) = o.veri.musteri_ekle("", None).unwrap();
    let (_, anahtar) = o.veri.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    let api = yeni_api(&o, ag);
    D { api, v: o.veri, n, anahtar, saat: o.saat, dizin: o.dizin }
}

fn kur_basit(ag: Option<&Arc<SahteAg>>) -> D {
    kur(ag, Some(netgsm()), None, "netgsm")
}

fn post(d: &D, yol: &str, govde: &Value, ip: &str) -> (u16, Value) {
    let y = Y::istek(&d.api, "POST", yol, &[], &Y::json_govde(govde), "", ip);
    (y.durum, serde_json::from_slice(&y.govde).expect("JSON cevap"))
}

fn sms_ip(d: &D, anahtar: &str, tel: Value, kod: Value, ip: &str) -> (u16, Value) {
    post(d, "/api/sms", &json!({"cihaz_anahtari": anahtar, "telefon": tel, "kod": kod}), ip)
}

fn sms(d: &D, anahtar: &str, tel: &str) -> (u16, Value) {
    sms_ip(d, anahtar, json!(tel), json!(KOD), IP)
}

fn dogrula_ip(d: &D, anahtar: &str, tel: Value, kod: Value, ip: &str) -> (u16, Value) {
    post(d, "/api/sms-dogrula", &json!({"cihaz_anahtari": anahtar, "telefon": tel, "kod": kod}), ip)
}

fn dogrula(d: &D, anahtar: &str) -> (u16, Value) {
    dogrula_ip(d, anahtar, json!(TEL), json!(KOD), IP)
}

fn bugun(d: &D) -> String {
    d.v.bugun().to_string()
}

fn sms_otp_var(d: &D) -> bool {
    d.v.hareketler(None, 300, None).unwrap().iter().any(|h| h.olay == "SMS_OTP")
}

fn kodsuz(d: &D) -> bool {
    d.v.hareketler(None, 300, None).unwrap().iter().all(|h| !h.ayrinti.contains(KOD))
}

#[test]
fn netgsm_gonderir_ve_sayar() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_netgsm_gonderir_ve_sayar");
    let ag = SahteAg::yeni(vec![AgCevap::Govde(BASARI)]);
    let d = kur_basit(Some(&ag));
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j == json!({"sonuc": "gonderildi", "kod": "0", "is": "555", "saglayici": "netgsm"}), "{j}");
    ag.son(|r| {
        assert!(
            r.govde.contains(&format!("<msg>WiFi dogrulama kodunuz: {KOD}. Kod 3 dakika gecerlidir.</msg>"))
                && r.govde.contains("<no>5334553132</no>")
        );
    });
    let gun = bugun(&d);
    assert_eq!(d.v.sms_sayac(d.n, &gun).unwrap(), 1);
    assert!(!sms_otp_var(&d)); // SMS başına hareket satırı yok (geçmişi boğmasın)
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert!((son.saglayici.as_str(), son.sonuc.as_str(), son.kod.as_str()) == ("netgsm", "gonderildi", "0") && son.zaman.starts_with("2026-"));
    assert!(kodsuz(&d));
    // veritabanı dosyasında da kod yok
    let ham = std::fs::read(d.dizin.join("m.db")).unwrap();
    assert!(!ham.windows(KOD.len()).any(|w| w == KOD.as_bytes()));
    ag.ekle(AgCevap::Govde("<xml><main><code>50</code></main></xml>"));
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j == json!({"sonuc": "hata", "kod": "50", "saglayici": "netgsm"}));
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert_eq!((son.sonuc.as_str(), son.kod.as_str()), ("hata", "50"));
    assert!(!sms_otp_var(&d));
}

#[test]
fn mock_istek_gondermez() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_mock_istek_gondermez");
    let d = kur(None, Some(ile(&netgsm(), json!({"mock": true}))), None, "netgsm");
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j["sonuc"] == "mock" && j["saglayici"] == "netgsm");
    assert!(d.v.sms_son(d.n).unwrap().is_none() && !sms_otp_var(&d)); // deneme modu son sonuca yazılmaz
    let d = kur(None, Some(ile(&netgsm(), json!({"mock": true}))), Some(tw()), "twilio");
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j["sonuc"] == "mock" && j["saglayici"] == "twilio");
}

#[test]
fn gecersiz_girdi_400() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_gecersiz_girdi_400");
    // Python'da taşıyıcı `ag_yok` (istisna → AG): son geçerli istek ağa çıkamaz, cevap hata/AG olur
    let ag = SahteAg::yeni(vec![AgCevap::Hata]);
    let d = kur_basit(Some(&ag));
    let ciftler: Vec<(Value, Value)> = vec![
        (json!(TEL), json!("12345")),
        (json!(TEL), json!("1234567")),
        (json!(TEL), json!("12a456")),
        (json!(TEL), json!(123456)),
        (json!(TEL), json!("١٢٣٤٥٦")),
        (json!("+905334553132"), json!(KOD)),
        (json!("90533455313x"), json!(KOD)),
        (json!("1234567"), json!(KOD)),
        (json!("9".repeat(16)), json!(KOD)),
        (json!(905334553132_i64), json!(KOD)),
        (Value::Null, json!(KOD)),
    ];
    for (i, (tel, kod)) in ciftler.into_iter().enumerate() {
        let ip = format!("198.51.100.{}", i + 10); // IP başına dakikada 20 istek sınırına takılmasın
        assert_eq!(sms_ip(&d, &d.anahtar, tel.clone(), kod.clone(), &ip).0, 400, "{tel} {kod}");
        assert_eq!(dogrula_ip(&d, &d.anahtar, tel.clone(), kod.clone(), &ip).0, 400, "{tel} {kod}");
    }
    assert_eq!(ag.sayi(), 0);
    // serbest metin alanı yok sayılır, şablon kullanılır
    let (c, _) = post(&d, "/api/sms", &json!({"cihaz_anahtari": d.anahtar, "telefon": TEL, "kod": KOD, "metin": "x"}), IP);
    assert_eq!(c, 200);
    assert_eq!(d.v.sms_sayac(d.n, &bugun(&d)).unwrap(), 1);
    ag.son(|r| assert!(r.govde.contains("WiFi dogrulama kodunuz") && !r.govde.contains(">x<")));
}

#[test]
fn yetkisiz_cihaz_reddi() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_yetkisiz_cihaz_reddi");
    let d = kur_basit(None);
    assert_eq!(sms(&d, "yanlis-anahtar", TEL).0, 401);
    d.v.askiya_al(d.n, true).unwrap();
    assert!(sms(&d, &d.anahtar, TEL).0 == 403 && dogrula(&d, &d.anahtar).0 == 403);
    d.v.askiya_al(d.n, false).unwrap();
    d.v.lisans_ayarla(d.n, "2026-01-01").unwrap();
    assert_eq!(sms(&d, &d.anahtar, TEL).0, 403);
    d.v.lisans_uzat(d.n, crate::veri::LISANS_GUN).unwrap();
    d.v.uyelik(d.n, "bitti").unwrap();
    assert_eq!(sms(&d, &d.anahtar, TEL).0, 403);
    d.v.uyelik(d.n, "aktif").unwrap();
    d.v.serbest_birak(d.n, false).unwrap();
    assert!(sms(&d, &d.anahtar, TEL).0 == 403 && dogrula(&d, &d.anahtar).0 == 403);
    assert_eq!(d.v.sms_sayac(d.n, &bugun(&d)).unwrap(), 0);
}

#[test]
fn yabanci_numara_twilio_kapali() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_yabanci_numara_twilio_kapali");
    let d = kur_basit(None);
    let (c, j) = sms(&d, &d.anahtar, "4915123456789");
    assert!(c == 200 && j["sonuc"] == "hata" && j["kod"] == "YABANCI");
    let (_, j) = sms(&d, &d.anahtar, "902161234567"); // 90 ama cep değil: Türk cep numarası sayılmaz
    assert_eq!(j["kod"], "YABANCI");
    assert_eq!(d.v.sms_sayac(d.n, &bugun(&d)).unwrap(), 0);
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert_eq!((son.saglayici.as_str(), son.sonuc.as_str(), son.kod.as_str()), ("", "hata", "YABANCI"));
}

#[test]
fn yabanci_numara_twilio_acik() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_yabanci_numara_twilio_acik");
    let ag = SahteAg::yeni(vec![AgCevap::Durum(201, r#"{"status": "pending", "sid": "VE1"}"#)]);
    let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "netgsm");
    let (c, j) = sms(&d, &d.anahtar, "4915123456789");
    assert!(c == 200 && j == json!({"sonuc": "gonderildi", "kod": "0", "saglayici": "twilio"}));
    ag.son(|r| assert!(r.url.contains("verify.twilio.com") && !r.govde.contains(KOD))); // kodu Twilio üretir
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert_eq!((son.saglayici.as_str(), son.sonuc.as_str()), ("twilio", "gonderildi"));
    ag.ekle(AgCevap::Govde("<xml><main><code>0</code><jobID>9</jobID></main></xml>"));
    let (_, j) = sms(&d, &d.anahtar, TEL); // Türk numarası hâlâ NetGSM'den
    assert!(j["saglayici"] == "netgsm");
    ag.son(|r| assert!(r.url.contains("netgsm")));
}

#[test]
fn tr_saglayici_twilio() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_tr_saglayici_twilio");
    let ag = SahteAg::yeni(vec![AgCevap::Durum(201, PENDING)]);
    let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "twilio");
    let (_, j) = sms(&d, &d.anahtar, TEL);
    assert_eq!(j, json!({"sonuc": "gonderildi", "kod": "0", "saglayici": "twilio"}));
    ag.son(|r| assert!(r.url.contains("verify.twilio.com")));
    assert_eq!(ag.sayi(), 1);
    ag.ekle(AgCevap::Durum(429, r#"{"code": 60203}"#));
    assert_eq!(sms(&d, &d.anahtar, TEL).1, json!({"sonuc": "hata", "kod": "60203", "saglayici": "twilio"}));
}

#[test]
fn ayar_yok() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_ayar_yok");
    let d = kur(None, None, None, "netgsm");
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j["sonuc"] == "hata" && j["kod"] == "AYAR");
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert_eq!((son.sonuc.as_str(), son.kod.as_str()), ("hata", "AYAR"));
}

#[test]
fn gunluk_sinir() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_gunluk_sinir");
    let ag = SahteAg::yeni(vec![]);
    let d = kur_basit(Some(&ag));
    let gun = bugun(&d);
    for _ in 0..299 {
        d.v.sms_sayac_artir(d.n, &gun).unwrap();
    }
    assert!(sms(&d, &d.anahtar, TEL).1["sonuc"] == "gonderildi" && ag.sayi() == 1);
    assert_eq!(d.v.sms_sayac(d.n, &gun).unwrap(), 300);
    let (c, j) = sms(&d, &d.anahtar, TEL);
    assert!(c == 200 && j["sonuc"] == "hata" && j["kod"] == "SINIR" && ag.sayi() == 1);
    assert_eq!(d.v.sms_sayac(d.n, &gun).unwrap(), 300);
    let son = d.v.sms_son(d.n).unwrap().unwrap();
    assert_eq!((son.sonuc.as_str(), son.kod.as_str()), ("hata", "SINIR"));
    let (n2, _) = d.v.musteri_ekle("", None).unwrap(); // sınır müşteri başına
    let (_, a2) = d.v.cihaz_bagla(n2, &format!("{}=", "B".repeat(43)), SSH, "", "", "").unwrap();
    assert!(sms(&d, &a2, TEL).1["sonuc"] == "gonderildi");
    d.saat.ilerlet(86400.0); // ertesi gün sayaç sıfırdan
    assert!(sms(&d, &d.anahtar, TEL).1["sonuc"] == "gonderildi" && d.v.sms_sayac(d.n, &gun).unwrap() == 300);
}

#[test]
fn yabanci_gunluk_sinir() {
    // Yabancı numaralara ayrı, daha düşük günlük sınır (20); yabancılar 300'lük toplamdan da düşer.
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_yabanci_gunluk_sinir");
    assert_eq!(SMS_YABANCI_SINIR, 20);
    let pend = |n: usize| (0..n).map(|_| AgCevap::Durum(201, PENDING)).collect::<Vec<_>>();
    let yabanci = |d: &D, ip: &str| sms_ip(d, &d.anahtar, json!("4915123456789"), json!(KOD), ip);
    {
        let ag = SahteAg::yeni(pend(25));
        let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "netgsm");
        let gun = bugun(&d);
        for i in 0..20 {
            assert!(yabanci(&d, &format!("198.51.100.{}", i % 5)).1["sonuc"] == "gonderildi");
        }
        assert!(d.v.sms_sayac(d.n, &gun).unwrap() == 20 && ag.sayi() == 20);
        let (c, j) = yabanci(&d, "198.51.100.9");
        assert!(c == 200 && j == json!({"sonuc": "hata", "kod": "SINIR", "saglayici": "twilio"}) && ag.sayi() == 20);
        assert_eq!(d.v.sms_sayac(d.n, &gun).unwrap(), 20); // reddedilen sayılmaz
        ag.basa_ekle(AgCevap::Govde("<xml><main><code>0</code><jobID>1</jobID></main></xml>"));
        // Türk numarası etkilenmez
        assert!(sms_ip(&d, &d.anahtar, json!(TEL), json!(KOD), "198.51.100.9").1["sonuc"] == "gonderildi");
        assert_eq!(d.v.sms_sayac(d.n, &gun).unwrap(), 21);
    }
    {
        // toplam dolunca yabancı da duramaz
        let ag = SahteAg::yeni(pend(1));
        let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "netgsm");
        for _ in 0..300 {
            d.v.sms_sayac_artir(d.n, &bugun(&d)).unwrap();
        }
        assert_eq!(yabanci(&d, IP).1["kod"], "SINIR");
    }
    {
        // TR sağlayıcısı Twilio: Türk numarası yabancı sayılmaz
        let ag = SahteAg::yeni(pend(22));
        let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "twilio");
        for i in 0..22 {
            let ip = format!("198.51.100.{}", i % 5);
            assert!(sms_ip(&d, &d.anahtar, json!(TEL), json!(KOD), &ip).1["sonuc"] == "gonderildi");
        }
    }
}

#[test]
fn dogrula_twilio() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_dogrula");
    let ag = SahteAg::yeni(vec![
        AgCevap::Durum(200, r#"{"status": "approved"}"#),
        AgCevap::Durum(200, PENDING),
        AgCevap::Durum(404, r#"{"code": 20404}"#),
        AgCevap::Hata,
    ]);
    let d = kur(Some(&ag), Some(netgsm()), Some(tw()), "twilio");
    assert_eq!(dogrula(&d, &d.anahtar), (200, json!({"onay": true})));
    assert_eq!(dogrula(&d, &d.anahtar), (200, json!({"onay": false})));
    assert_eq!(dogrula(&d, &d.anahtar), (200, json!({"onay": false})));
    assert_eq!(dogrula(&d, &d.anahtar), (200, json!({"sonuc": "hata", "kod": "AG"})));
    assert!(ag.istekler.lock().unwrap()[0].govde.ends_with(&format!("Code={KOD}")));
    assert!(kodsuz(&d));
    assert_eq!(dogrula(&d, "yanlis").0, 401);
}

#[test]
fn dogrula_twilio_kapali() {
    let _k = Kayit::bas("test_sms_merkez", "test_api_sms_dogrula_twilio_kapali");
    let d = kur_basit(None);
    assert_eq!(dogrula(&d, &d.anahtar), (200, json!({"sonuc": "hata", "kod": "AYAR"})));
}

fn rapor() -> Value {
    json!({"surum": "0.4.8", "acik_sn": 10, "servisler": {"portal": true, "panel": true, "kaydedici": true, "dnsmasq": true, "guvenlik": true},
           "disk_yuzde": 10, "son_muhur": "", "kayit_boslugu_1s": false, "wan": true, "sms_sorunu": false, "bagli": 0, "hiz_hata": false})
}

/// Spec bilinçli fark 2: eşleşme cevabındaki `sms` yalnızca `etkin_mock` taşır (Python kaydı eski cihazlar için `mock`,
/// `provider` ve `netgsm` bloğunu da taşıyordu; ortak NetGSM bilgisi hiçbir cihaza gitmez). Twilio sırrı gitmez.
#[test]
fn eski_cihaz_eki_degismez() {
    let _k = Kayit::altinsiz("spec bilinçli fark 2");
    let d = kur(None, Some(netgsm()), Some(tw()), "twilio");
    let (c, j) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": d.anahtar}), IP);
    assert!(c == 200 && j["sms"] == json!({"etkin_mock": false}), "{j}");
    assert!(!j.to_string().contains("tw-token-gizli") && !j.to_string().contains("netgsm-gizli"));
    let s = d.v.sms_ayari().unwrap().unwrap();
    assert!(s["tr_saglayici"] == "twilio" && s["twilio"]["auth_token"] == "tw-token-gizli");
}

/// Cihaza giden etkin deneme modu: merkez deneme modunda YA DA seçili Türk sağlayıcısının ayarı eksikse true. Spec bilinçli
/// fark 2: `mock` ve `netgsm` bloğu artık gitmez (altınsız).
#[test]
fn etkin_deneme_modu_eki() {
    let _k = Kayit::altinsiz("spec bilinçli fark 2");
    use crate::veri::sms_hazir;
    let tr = |e: Value| ile(&netgsm(), ile(&json!({"tr_saglayici": "netgsm", "twilio": {"acik": false}}), e));
    assert!(sms_hazir(&tr(json!({}))));
    for eksik in ["usercode", "password", "msgheader"] {
        let mut e = serde_json::Map::new();
        e.insert(eksik.into(), json!(""));
        assert!(!sms_hazir(&tr(Value::Object(e))));
    }
    assert!(sms_hazir(&ile(&netgsm(), json!({"password": "", "tr_saglayici": "twilio", "twilio": tw()}))));
    for kotu in [json!({"acik": false}), json!({"verify_sid": "VAkisa"}), json!({"auth_token": ""})] {
        assert!(!sms_hazir(&ile(&netgsm(), json!({"tr_saglayici": "twilio", "twilio": ile(&tw(), kotu)}))));
    }
    let d = kur_basit(None);
    let es = |d: &D| post(d, "/api/eslesme", &json!({"cihaz_anahtari": d.anahtar}), IP).1;
    let sg = |d: &D| post(d, "/api/saglik", &json!({"cihaz_anahtari": d.anahtar, "rapor": rapor()}), IP).1;
    assert_eq!(es(&d)["sms"], json!({"etkin_mock": false}));
    assert_eq!(sg(&d)["sms_etkin_mock"], json!(false));
    d.v.sms_ayari_koy(&ile(&netgsm(), json!({"mock": true}))).unwrap();
    assert_eq!(es(&d)["sms"], json!({"etkin_mock": true}));
    assert_eq!(sg(&d)["sms_etkin_mock"], json!(true));
    // deneme modu kapalı ama seçili sağlayıcı eksik (veritabanına eksik ayar elle yazılmış): etkin deneme modu
    let eksik = ile(&netgsm(), json!({"password": ""})).to_string();
    d.v.kilit().execute("UPDATE ayar SET deger = ? WHERE anahtar = 'sms'", [eksik]).unwrap();
    assert_eq!(es(&d)["sms"], json!({"etkin_mock": true}));
    // hiç kaydedilmemiş: yalnızca etkin_mock
    let d = kur(None, None, None, "netgsm");
    assert_eq!(es(&d)["sms"], json!({"etkin_mock": true}));
    assert_eq!(sg(&d)["sms_etkin_mock"], json!(true));
}

#[test]
fn hareketler_olay_suzgeci() {
    let _k = Kayit::bas("test_sms_merkez", "test_hareketler_olay_suzgeci");
    let o = Y::ortam();
    let v = &o.veri;
    for i in 0..5 {
        v.hareket("x", "-", if i % 2 == 1 { "GUNCELLEME_SONUC" } else { "BASKA" }, Some(1234567), &i.to_string()).unwrap();
    }
    v.hareket("x", "-", "GUNCELLEME_SONUC", Some(7654321), "baska-musteri").unwrap();
    let ay = |h: Vec<crate::veri::Hareket>| h.into_iter().map(|h| h.ayrinti).collect::<Vec<_>>();
    assert_eq!(ay(v.hareketler(Some(1234567), 300, Some("GUNCELLEME_SONUC")).unwrap()), ["3", "1"]);
    assert_eq!(ay(v.hareketler(None, 2, Some("GUNCELLEME_SONUC")).unwrap()), ["baska-musteri", "3"]);
    assert_eq!(v.hareketler(Some(1234567), 300, None).unwrap().len(), 5);
}

#[test]
fn sms_ayari_dogrulama() {
    let _k = Kayit::bas("test_sms_merkez", "test_sms_ayari_dogrulama");
    let o = Y::ortam();
    let v = &o.veri;
    v.sms_ayari_koy(&netgsm()).unwrap();
    let s = v.sms_ayari().unwrap().unwrap();
    assert!(s["tr_saglayici"] == "netgsm" && s["twilio"]["acik"] == json!(false));
    let kotular = [
        json!({"tr_saglayici": "baska"}),
        json!({"tr_saglayici": "twilio"}), // Twilio kapalıyken TR'yi Twilio'ya veremez
        json!({"twilio": ile(&tw(), json!({"verify_sid": "VA../x"}))}),
        json!({"twilio": ile(&tw(), json!({"account_sid": "XX1"}))}),
        json!({"twilio": ile(&tw(), json!({"auth_token": ""}))}),
    ];
    for kotu in kotular {
        let h = v.sms_ayari_koy(&ile(&netgsm(), kotu.clone()));
        assert!(matches!(h, Err(Hata::Deger(_))), "{kotu}");
    }
    v.sms_ayari_koy(&ile(&netgsm(), json!({"password": "", "usercode": "", "tr_saglayici": "twilio", "twilio": tw()}))).unwrap(); // NetGSM gerekmez
    let t = ile(&tw(), json!({"auth_token": "", "api_key_sid": format!("SK{}", "1".repeat(32)), "api_key_secret": "s"}));
    v.sms_ayari_koy(&ile(&netgsm(), json!({"twilio": t}))).unwrap();
}

#[test]
fn sayac() {
    let _k = Kayit::bas("test_sms_merkez", "test_sayac");
    let o = Y::ortam();
    let v = &o.veri;
    assert_eq!(v.sms_sayac(1234567, "2026-09-21").unwrap(), 0);
    assert!(v.sms_sayac_artir(1234567, "2026-09-21").unwrap() == 1 && v.sms_sayac_artir(1234567, "2026-09-21").unwrap() == 2);
    assert!(v.sms_sayac(1234567, "2026-09-22").unwrap() == 0 && v.sms_sayac(7654321, "2026-09-21").unwrap() == 0);
}
