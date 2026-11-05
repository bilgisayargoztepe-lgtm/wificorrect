//! saglik.rs testleri: Python tests/test_saglik.py'nin karşılığı (43 test, altın kayıtlı). Gerçek SMS/NetGSM isteği
//! gitmez: gönderici sahte taşıyıcıdır (gövdeden mesajı ve numaraları kaydeder).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::*;
use crate::altin::Kayit;
use crate::ortak::{Curl, DisIstek};
use crate::test_yardim::{gecici_dizin, AgCevap, AgYok, SahteAg, SahteSaat};
use crate::veri::SmsSon;

const BUGUN: &str = "2026-10-11";
const SERVISLER: [&str; 5] = ["portal", "panel", "kaydedici", "dnsmasq", "guvenlik"];

fn ts(iso: &str) -> f64 {
    epoch(&format!("{iso}+03:00")).unwrap()
}

/// Python `T0`: 2026-10-11 20:15 (TR).
fn t0() -> f64 {
    ts("2026-10-11T20:15:00")
}

fn yeni_veri(yol: &std::path::Path, t: &SahteSaat) -> Veri {
    Veri::ac(yol, t.saat()).unwrap()
}

// --- Task 2 (veri) testleri ---
#[test]
fn veri_saglik_tek_satir() {
    let _k = Kayit::bas("test_saglik", "test_veri_saglik_tek_satir");
    let tmp = gecici_dizin();
    let t = SahteSaat::yeni(1_790_000_000.0);
    let v = yeni_veri(&tmp.join("m.db"), &t);
    assert!(v.saglik(7).unwrap().is_none());
    v.saglik_kaydet(7, &json!({"disk": 41, "servisler": ["a"]})).unwrap();
    let s1 = v.saglik(7).unwrap().unwrap();
    assert!(s1.rapor == json!({"disk": 41, "servisler": ["a"]}) && s1.zaman.starts_with("2026-"));
    t.ilerlet(300.0);
    v.saglik_kaydet(7, &json!({"disk": 42})).unwrap();
    let s2 = v.saglik(7).unwrap().unwrap();
    assert!(s2.rapor == json!({"disk": 42}) && s2.zaman > s1.zaman);
    let n: i64 = v.kilit().query_row("SELECT COUNT(*) n FROM saglik", [], |r| r.get("n")).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn sorun_ac_ayni_turu_tekrar_acmaz() {
    let _k = Kayit::bas("test_saglik", "test_sorun_ac_ayni_turu_tekrar_acmaz");
    let tmp = gecici_dizin();
    let v = yeni_veri(&tmp.join("m.db"), &SahteSaat::yeni(1_790_000_000.0));
    let a = v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%91)", "uyari").unwrap();
    assert_eq!(v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%95)", "uyari").unwrap(), a);
    assert_ne!(v.sorun_ac(1000001, "rapor_yok", "Rapor gelmiyor", "kritik").unwrap(), a);
    assert_ne!(v.sorun_ac(1000002, "disk_dolu", "Disk dolu (%91)", "uyari").unwrap(), a);
    assert!(v.acik_sorunlar(None).unwrap().len() == 3 && v.acik_sorunlar(Some(1000001)).unwrap().len() == 2);
    assert_eq!(v.acik_sorunlar(Some(1000001)).unwrap()[0].kapanis, "");
}

#[test]
fn sorun_kapat_ve_gecmis() {
    let _k = Kayit::bas("test_saglik", "test_sorun_kapat_ve_gecmis");
    let tmp = gecici_dizin();
    let t = SahteSaat::yeni(1_790_000_000.0);
    let v = yeni_veri(&tmp.join("m.db"), &t);
    assert!(v.son_kapanan(5, "disk_dolu").unwrap().is_none());
    let a = v.sorun_ac(5, "disk_dolu", "Disk dolu (%91)", "uyari").unwrap();
    v.sorun_sms(a, "acilis_sms", "mock").unwrap();
    t.ilerlet(600.0);
    v.sorun_kapat(a).unwrap();
    v.sorun_sms(a, "kapanis_sms", "hata:30").unwrap();
    assert!(v.acik_sorunlar(Some(5)).unwrap().is_empty());
    let k = v.son_kapanan(5, "disk_dolu").unwrap().unwrap();
    assert!(k.id == a && k.kapanis > k.acilis && k.acilis_sms == "mock" && k.kapanis_sms == "hata:30");
    let b = v.sorun_ac(5, "disk_dolu", "Disk dolu (%92)", "uyari").unwrap(); // kapandıktan sonra yenisi açılır
    assert_ne!(b, a);
    let g = v.sorun_gecmisi(5, 50).unwrap();
    assert_eq!(g.iter().map(|x| x.id).collect::<Vec<_>>(), vec![b, a]);
    assert_eq!(v.sorun_gecmisi(5, 1).unwrap().len(), 1);
    assert!(matches!(v.sorun_sms(a, "baska", "mock"), Err(Hata::Deger(_))));
}

#[test]
fn uyari_ayari_dogrulama() {
    let _k = Kayit::bas("test_saglik", "test_uyari_ayari_dogrulama");
    let tmp = gecici_dizin();
    let v = yeni_veri(&tmp.join("m.db"), &SahteSaat::yeni(1_790_000_000.0));
    assert_eq!(v.uyari_ayari().unwrap(), json!({"telefonlar": [], "acik": false, "gunluk_sinir": 20}));
    v.uyari_ayari_koy(&json!({"telefonlar": ["0533 455 31 32"], "acik": true, "gunluk_sinir": 5})).unwrap();
    assert_eq!(v.uyari_ayari().unwrap(), json!({"telefonlar": ["905334553132"], "acik": true, "gunluk_sinir": 5}));
    for kotu in [
        json!({"telefonlar": ["0533 455 31 32", "0533 455 31 33", "0533 455 31 34", "0533 455 31 35"]}),
        json!({"telefonlar": ["12345"]}),
    ] {
        assert!(matches!(v.uyari_ayari_koy(&kotu), Err(Hata::Deger(_))));
    }
    assert_eq!(v.uyari_ayari().unwrap()["telefonlar"], json!(["905334553132"])); // hatalı kayıt eskisini bozmaz
    for kotu in [json!(0), json!(1), json!(101), json!(true), json!(2.5), json!("x"), Value::Null] {
        // sınır 2–100 tamsayı (1: hiç uyarı gitmezdi)
        let r = v.uyari_ayari_koy(&json!({"telefonlar": [], "gunluk_sinir": kotu}));
        assert!(matches!(r, Err(Hata::Deger(_))), "{kotu}");
    }
    for (iyi, sayi) in [(json!(2), 2), (json!(100), 100), (json!("7"), 7)] {
        v.uyari_ayari_koy(&json!({"telefonlar": [], "gunluk_sinir": iyi})).unwrap();
        assert_eq!(v.uyari_ayari().unwrap()["gunluk_sinir"], json!(sayi));
    }
}

#[test]
fn sms_sayaci_gun_bazli() {
    let _k = Kayit::bas("test_saglik", "test_sms_sayaci_gun_bazli");
    let tmp = gecici_dizin();
    let v = yeni_veri(&tmp.join("m.db"), &SahteSaat::yeni(1_790_000_000.0));
    assert_eq!(v.sms_sayaci("2026-10-11").unwrap(), 0);
    assert!(v.sms_say("2026-10-11").unwrap() == 1 && v.sms_say("2026-10-11").unwrap() == 2);
    assert!(v.sms_sayaci("2026-10-11").unwrap() == 2 && v.sms_sayaci("2026-10-12").unwrap() == 0);
}

// --- Task 3: netgsm + kurallar + SMS kararı ---
#[test]
fn netgsm_xml_ve_yanit() {
    let _k = Kayit::bas("test_saglik", "test_netgsm_xml_ve_yanit");
    let x = crate::netgsm::xml_govde("u&1", "p<2", "BAŞLIK", "a]]>b ç", &["905334553132", "5334553133", "12"]);
    assert!(x.contains("<usercode>u&amp;1</usercode>") && x.contains("<password>p&lt;2</password>"));
    assert!(x.contains("<type>1:n</type>") && x.contains("<msgheader>BAŞLIK</msgheader>"));
    assert!(x.contains("<![CDATA[a]]]]><![CDATA[>b ç]]>"), "{x}"); // CDATA'dan taşmaz
    assert!(x.contains("<no>5334553132</no><no>5334553133</no>") && !x.contains("<no>12</no>"));
    use crate::netgsm::{gonder, yanit_kodu};
    assert!(yanit_kodu("00 123456") == "00" && yanit_kodu("0") == "0" && yanit_kodu("30\n") == "30");
    assert!(yanit_kodu("<xml><main><code>40</code></main></xml>") == "40" && yanit_kodu("").is_empty());
    let ayar = json!({"mock": false, "usercode": "u", "password": "gizli", "msgheader": "H"});
    let numara = ["905334553132"];
    let mut mock = ayar.clone();
    mock["mock"] = json!(true);
    assert_eq!(gonder(&mock, "m", &numara, &AgYok), "mock");
    assert_eq!(gonder(&Value::Null, "m", &numara, &AgYok), "hata:AYAR");
    let mut parolasiz = ayar.clone();
    parolasiz["password"] = json!("");
    assert_eq!(gonder(&parolasiz, "m", &numara, &AgYok), "hata:AYAR");
    assert_eq!(gonder(&ayar, "m", &["123"], &AgYok), "hata:NUMARA");
    let ag = SahteAg::yeni(vec![AgCevap::Govde("00 987654")]);
    assert_eq!(gonder(&ayar, "merhaba", &numara, ag.as_ref()), "gonderildi");
    ag.son(|r| {
        assert!(r.url == "https://api.netgsm.com.tr/sms/send/xml" && r.yontem == "POST" && r.zaman_asimi == Duration::from_secs(25));
        assert_eq!(r.basliklar, vec![("Content-Type".to_string(), "application/xml; charset=UTF-8".to_string())]);
        assert!(r.govde.contains("<no>5334553132</no>") && r.govde.contains("merhaba"));
    });
    let tek = |c: AgCevap| gonder(&ayar, "m", &numara, SahteAg::yeni(vec![c]).as_ref());
    assert_eq!(tek(AgCevap::Govde("30")), "hata:30");
    assert_eq!(tek(AgCevap::Govde("<html>bozuk")), "hata:BILINMIYOR");
    assert_eq!(tek(AgCevap::Hata), "hata:AG");
}

fn m_ornek() -> Musteri {
    Musteri {
        numara: 1537344,
        tuz: String::new(),
        ozet: String::new(),
        yineleme: 0,
        not_: String::new(),
        uyelik: "aktif".into(),
        bitis: String::new(),
        olusturma: String::new(),
        son_giris: String::new(),
        parola_acik: String::new(),
        lisans_bitis: "2027-10-11".into(),
        askida: 0,
        guncelleme_kanali: "kararli".into(),
        guncelleme_hemen: 0.0,
        guncelleme_hemen_surum: String::new(),
        ap_parola: String::new(),
    }
}

fn c_ornek() -> Cihaz {
    Cihaz {
        id: 1,
        musteri: 1537344,
        durum: "bagli".into(),
        tunel_ip: String::new(),
        wg_pub: String::new(),
        ssh_pub: String::new(),
        anahtar_ozet: String::new(),
        isletme_adi: "Göztepe".into(),
        unvan: String::new(),
        surum: String::new(),
        son_eslesme: String::new(),
        baglanma: String::new(),
        admin_parola: String::new(),
        admin_tuz: String::new(),
        admin_ozet: String::new(),
        admin_yineleme: 0,
        ayrilma: String::new(),
        isletme_turu: String::new(),
        adres: String::new(),
        otomatik: 0,
    }
}

/// Python `rapor(**d)`.
fn rapor(d: &[(&str, Value)]) -> Value {
    let mut r = json!({"surum": "1.0", "acik_sn": 100,
        "servisler": {"portal": true, "panel": true, "kaydedici": true, "dnsmasq": true, "guvenlik": true},
        "disk_yuzde": 40, "son_muhur": "2026-10-10", "kayit_boslugu_1s": false, "wan": true, "sms_sorunu": false,
        "bagli": 3, "hiz_hata": false});
    for (k, v) in d {
        if SERVISLER.contains(k) {
            r["servisler"][*k] = v.clone();
        } else {
            r[*k] = v.clone();
        }
    }
    r
}

/// Python `turler(m, c, r, rz, el, sg, simdi)` varsayılanları.
struct G {
    m: Musteri,
    c: Option<Cihaz>,
    r: Option<Value>,
    rz: Option<f64>,
    el: Option<f64>,
    sg: Option<String>,
    simdi: f64,
}

fn g() -> G {
    G {
        m: m_ornek(),
        c: Some(c_ornek()),
        r: Some(rapor(&[])),
        rz: Some(t0() - 60.0),
        el: Some(t0() - 60.0),
        sg: Some(BUGUN.into()),
        simdi: t0(),
    }
}

impl G {
    fn sorunlar(&self) -> Vec<SorunSatiri> {
        sorunlar(&self.m, self.c.as_ref(), self.r.as_ref(), self.rz, self.el, self.sg.as_deref(), self.simdi, true, None, None)
    }
}

fn turler(f: impl FnOnce(&mut G)) -> Vec<String> {
    let mut x = g();
    f(&mut x);
    x.sorunlar().into_iter().map(|s| s.0).collect()
}

fn sr(tur: &str, metin: &str, onem: &str) -> SorunSatiri {
    (tur.into(), metin.into(), onem.into())
}

#[test]
fn kural_saglam_cihazda_sorun_yok() {
    let _k = Kayit::bas("test_saglik", "test_kural_saglam_cihazda_sorun_yok");
    assert!(turler(|_| {}).is_empty());
}

#[test]
fn kural_ulasilamiyor_diger_kritikleri_bastirir() {
    let _k = Kayit::bas("test_saglik", "test_kural_ulasilamiyor_diger_kritikleri_bastirir");
    let t = t0();
    let kotu = rapor(&[("kaydedici", json!(false)), ("portal", json!(false)), ("disk_yuzde", json!(95))]);
    let k = kotu.clone();
    assert_eq!(turler(|g| { g.r = Some(k); g.rz = Some(t - 2000.0); g.el = Some(t - 601.0); }), ["ulasilamiyor", "disk_dolu"]);
    let k = kotu.clone();
    assert_eq!(turler(|g| { g.r = Some(k); g.rz = Some(t - 2000.0); g.el = None; }), ["ulasilamiyor", "disk_dolu"]); // hiç el sıkışmamış
    let k = kotu.clone();
    assert_eq!(
        turler(|g| { g.r = Some(k); g.rz = Some(t - 2000.0); g.el = Some(t - 60.0); }),
        ["rapor_yok", "kayit_durdu", "servis_durdu", "disk_dolu"]
    );
    let mut x = g();
    x.r = Some(kotu);
    let s: HashMap<String, (String, String)> = x.sorunlar().into_iter().map(|(a, b, c)| (a, (b, c))).collect();
    assert_eq!(s["servis_durdu"], ("Servis durdu: portal".to_string(), "kritik".to_string()));
    assert_eq!(s["kayit_durdu"], ("5651 kaydı durdu".to_string(), "kritik".to_string()));
    assert_eq!(turler(|g| g.r = Some(rapor(&[("kayit_boslugu_1s", json!(true))]))), ["kayit_durdu"]);
    assert_eq!(
        turler(|g| g.r = Some(rapor(&[("panel", json!(false)), ("dnsmasq", json!(false)), ("guvenlik", json!(false))]))),
        ["servis_durdu"]
    );
    let mut x = g();
    x.r = Some(rapor(&[("panel", json!(false)), ("dnsmasq", json!(false))]));
    assert_eq!(x.sorunlar(), vec![sr("servis_durdu", "Servis durdu: panel, dnsmasq", "kritik")]);
    let mut x = g();
    x.el = Some(t - 700.0);
    assert_eq!(x.sorunlar(), vec![sr("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik")]);
}

#[test]
fn kural_esikleri() {
    let _k = Kayit::bas("test_saglik", "test_kural_esikleri");
    let t = t0();
    assert!(turler(|g| g.r = Some(rapor(&[("disk_yuzde", json!(89))]))).is_empty());
    let mut x = g();
    x.r = Some(rapor(&[("disk_yuzde", json!(90))]));
    assert_eq!(x.sorunlar(), vec![sr("disk_dolu", "Disk dolu (%90)", "uyari")]);
    assert!(turler(|g| g.m.lisans_bitis = "2026-10-27".into()).is_empty());
    let mut x = g();
    x.m.lisans_bitis = "2026-10-26".into();
    assert_eq!(x.sorunlar(), vec![sr("lisans_bitiyor", "Lisans bitiyor (26.10.2026)", "bilgi")]);
    assert!(turler(|g| g.rz = Some(t - 899.0)).is_empty());
    assert_eq!(turler(|g| g.rz = Some(t - 901.0)), ["rapor_yok"]);
    assert!(turler(|g| g.el = Some(t - 600.0)).is_empty());
    assert_eq!(turler(|g| g.el = Some(t - 601.0)), ["ulasilamiyor"]);
    assert!(turler(|g| g.sg = Some("2026-10-09".into())).is_empty());
    assert_eq!(turler(|g| g.sg = Some("2026-10-08".into())), ["yedek_gecikti"]);
    assert_eq!(turler(|g| g.sg = None), ["yedek_gecikti"]);
    let mut x = g();
    x.r = Some(rapor(&[("sms_sorunu", json!(true))]));
    assert_eq!(x.sorunlar(), vec![sr("sms_calismiyor", "SMS çalışmıyor", "uyari")]);
    // lisansı bitmiş: yalnızca lisans bilgisi
    let mut x = g();
    x.m.lisans_bitis = "2026-10-10".into();
    x.r = Some(rapor(&[("disk_yuzde", json!(99))]));
    x.rz = Some(t - 5000.0);
    x.el = Some(t - 5000.0);
    x.sg = None;
    assert_eq!(x.sorunlar(), vec![sr("lisans_bitti", "Lisans bitti (10.10.2026)", "bilgi")]);
}

#[test]
fn kural_merkezdeki_son_dogrulama_sms_sonucu() {
    let _k = Kayit::bas("test_saglik", "test_kural_merkezdeki_son_dogrulama_sms_sonucu");
    // Merkezin son doğrulama SMS'i (sms_son) son 60 dk içinde misafirden kaynaklanmayan bir hatayla bittiyse SMS çalışmıyor.
    let t = t0();
    let son = |sonuc: &str, kod: &str, once: f64| SmsSon {
        musteri: 1537344,
        zaman: ortak::now_iso(t - once),
        saglayici: "netgsm".into(),
        sonuc: sonuc.into(),
        kod: kod.into(),
    };
    let tur = |s: Option<&SmsSon>, el: f64| -> Vec<String> {
        let x = g();
        sorunlar(&x.m, x.c.as_ref(), x.r.as_ref(), x.rz, Some(t - el), x.sg.as_deref(), t, true, None, s).into_iter().map(|s| s.0).collect()
    };
    assert!(tur(None, 60.0).is_empty());
    for kod in ["30", "AG", "AYAR", "PARSE", "SINIR", "100", "20003"] {
        assert_eq!(tur(Some(&son("hata", kod, 60.0)), 60.0), ["sms_calismiyor"], "{kod}");
    }
    for kod in ["YABANCI", "50", "60200", "21211", "60205", "60203", "80"] {
        // misafirden kaynaklanan
        assert!(tur(Some(&son("hata", kod, 60.0)), 60.0).is_empty(), "{kod}");
    }
    assert!(tur(Some(&son("gonderildi", "0", 60.0)), 60.0).is_empty()); // son gönderim başarılı: kapanır
    assert!(tur(Some(&son("hata", "30", 3601.0)), 60.0).is_empty()); // 60 dk'dan eski sonuç
    assert_eq!(tur(Some(&son("hata", "30", 3599.0)), 60.0), ["sms_calismiyor"]);
    // rapordaki sms_sorunu ile birlikte tek sorun
    let mut x = g();
    x.r = Some(rapor(&[("sms_sorunu", json!(true))]));
    let sm = son("hata", "30", 60.0);
    let s = sorunlar(&x.m, x.c.as_ref(), x.r.as_ref(), x.rz, x.el, x.sg.as_deref(), t, true, None, Some(&sm));
    assert_eq!(s, vec![sr("sms_calismiyor", "SMS çalışmıyor", "uyari")]);
    // ulaşılamıyorken de (merkezin kendi bilgisi)
    assert!(tur(Some(&son("hata", "30", 60.0)), 700.0).contains(&"sms_calismiyor".to_string()));
}

#[test]
fn hic_rapor_yoksa_rapor_yok_acilmaz() {
    let _k = Kayit::bas("test_saglik", "test_hic_rapor_yoksa_rapor_yok_acilmaz");
    assert!(turler(|g| { g.r = None; g.rz = None; }).is_empty());
    assert_eq!(turler(|g| { g.r = None; g.rz = None; g.el = Some(t0() - 700.0); }), ["ulasilamiyor"]);
}

#[test]
fn serbest_askida_kuralsiz() {
    let _k = Kayit::bas("test_saglik", "test_serbest_askida_kuralsiz");
    assert!(turler(|g| { g.m.askida = 1; g.el = None; g.sg = None; }).is_empty());
    assert!(turler(|g| { g.c.as_mut().unwrap().durum = "serbest_birakiliyor".into(); g.el = None; g.sg = None; }).is_empty());
    assert!(turler(|g| { g.c = None; g.el = None; g.sg = None; }).is_empty());
    assert!(turler(|g| { g.m.uyelik = "bitti".into(); g.el = None; g.sg = None; }).is_empty());
}

#[test]
fn kural_guncelleme_basarisiz() {
    let _k = Kayit::bas("test_saglik", "test_kural_guncelleme_basarisiz");
    let t = |r: Value, hedef: Option<&str>| -> Vec<String> {
        let x = g();
        sorunlar(&x.m, x.c.as_ref(), Some(&r), x.rz, x.el, x.sg.as_deref(), x.simdi, true, hedef, None).into_iter().map(|s| s.0).collect()
    };
    let h = Some("1.2.0");
    let gun = json!({"surum": "1.2.0", "sonuc": "geri_alindi", "zaman": 1});
    let gun_hata = json!({"surum": "1.2.0", "sonuc": "hata", "zaman": 1});
    assert_eq!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", gun.clone())]), h), ["guncelleme_basarisiz"]);
    assert_eq!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", gun_hata)]), h), ["guncelleme_basarisiz"]);
    let x = g();
    let r = rapor(&[("paket", json!("1.1.0")), ("guncelleme", gun.clone())]);
    let s = sorunlar(&x.m, x.c.as_ref(), Some(&r), x.rz, x.el, x.sg.as_deref(), x.simdi, true, h, None);
    assert_eq!(s, vec![sr("guncelleme_basarisiz", "Güncelleme başarısız (1.2.0)", "uyari")]);
    assert!(t(rapor(&[("paket", json!("1.2.0")), ("guncelleme", gun.clone())]), h).is_empty()); // paket hedefe eşit
    assert!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", json!({"surum": "1.2.0", "sonuc": "tamam", "zaman": 1}))]), h).is_empty());
    assert!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", json!({"surum": "1.1.5", "sonuc": "geri_alindi", "zaman": 1}))]), h).is_empty()); // başka sürümün denemesi
    assert!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", gun)]), None).is_empty());
    assert!(t(rapor(&[("paket", json!("1.1.0"))]), h).is_empty());
    assert!(t(rapor(&[("paket", json!("1.1.0")), ("guncelleme", Value::Null)]), h).is_empty());
}

// --- DB'li tur testleri ---
/// Sahte NetGSM: hiçbir ağ isteği yok; gövdeden mesajı ve numaraları ayıklar.
struct Sahte {
    giden: Mutex<Vec<String>>,
    sonuc: Mutex<String>,
}

impl Sahte {
    fn yeni() -> Arc<Sahte> {
        Arc::new(Sahte { giden: Mutex::new(Vec::new()), sonuc: Mutex::new("gonderildi".into()) })
    }

    fn giden(&self) -> Vec<String> {
        self.giden.lock().unwrap().clone()
    }

    fn sonuc_koy(&self, s: &str) {
        *self.sonuc.lock().unwrap() = s.into();
    }
}

impl Tasiyici for Sahte {
    fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String> {
        let g = String::from_utf8_lossy(i.govde.as_deref().expect("gövde")).into_owned();
        assert!(g.contains("<usercode>u</usercode>"), "{g}");
        let nolar: Vec<&str> = g.split("<no>").skip(1).map(|p| p.split("</no>").next().unwrap_or("")).collect();
        assert_eq!(nolar, ["5334553132"]);
        let bas = g.find("<![CDATA[").expect("CDATA") + "<![CDATA[".len();
        let son = g.find("]]></msg>").expect("msg");
        self.giden.lock().unwrap().push(g[bas..son].to_string());
        let sonuc = self.sonuc.lock().unwrap().clone();
        match sonuc.as_str() {
            "gonderildi" => Ok((200, b"00 123".to_vec())),
            s if s.starts_with("hata:") => Ok((200, s["hata:".len()..].as_bytes().to_vec())),
            _ => Err("ağ".into()),
        }
    }
}

/// Her zaman ağ hatası veren taşıyıcı (Python `patlayan`).
struct Patlayan;

impl Tasiyici for Patlayan {
    fn gonder(&self, _: &DisIstek) -> Result<(u16, Vec<u8>), String> {
        Err("OSError: ağ".into())
    }
}

#[derive(Clone, Copy)]
enum ElYas {
    /// durum.json'da el sıkışma yaşı (sn)
    Yas(f64),
    /// None: hiç el sıkışmamış
    Hic,
    /// "yok": durum.json'da kaydı yok
    Yok,
}

struct Ortam {
    tmp: PathBuf,
    yol: PathBuf,
    t: SahteSaat,
    v: Veri,
    g: Arc<Sahte>,
    el: Vec<(i64, ElYas)>,
    sg: HashMap<i64, String>,
    rd: HashMap<i64, Vec<(&'static str, Value)>>,
    /// durum.json'daki `wg show` başarısı; None: alan yok (eski wc-durum)
    wg_ok: Option<bool>,
    ld: PathBuf,
}

impl Ortam {
    fn yeni() -> Ortam {
        Ortam::ozel(true, 20, true, t0())
    }

    fn ozel(acik: bool, sinir: i64, telefon: bool, t: f64) -> Ortam {
        let tmp = gecici_dizin();
        let yol = tmp.join("m.db");
        let saat = SahteSaat::yeni(t);
        let v = yeni_veri(&yol, &saat);
        let tel = if telefon { json!(["0533 455 31 32"]) } else { json!([]) };
        v.uyari_ayari_koy(&json!({"telefonlar": tel, "acik": acik, "gunluk_sinir": sinir})).unwrap();
        v.sms_ayari_koy(&json!({"mock": false, "usercode": "u", "password": "p", "msgheader": "H"})).unwrap();
        let ld = tmp.join("listeler");
        Ortam { tmp, yol, t: saat, v, g: Sahte::yeni(), el: Vec::new(), sg: HashMap::new(), rd: HashMap::new(), wg_ok: Some(true), ld }
    }

    fn musteri(&mut self, ad: &str, lisans: &str, el_yas: ElYas) -> (i64, i64) {
        let (n, _) = self.v.musteri_ekle("", None).unwrap();
        let (cid, _) = self.v.cihaz_bagla(n, &format!("{n:043}="), "ssh-ed25519 AAAA", ad, "", "").unwrap();
        self.v.lisans_ayarla(n, lisans).unwrap();
        self.el.push((n, el_yas));
        (n, cid)
    }

    /// Varsayılan müşteri: Göztepe, lisans 2027-10-11, el sıkışma 60 sn önce.
    fn musteri_v(&mut self) -> (i64, i64) {
        self.musteri("Göztepe", "2027-10-11", ElYas::Yas(60.0))
    }

    fn musteri_el(&mut self, el: ElYas) -> (i64, i64) {
        self.musteri("Göztepe", "2027-10-11", el)
    }

    fn el_koy(&mut self, n: i64, y: ElYas) {
        match self.el.iter_mut().find(|e| e.0 == n) {
            Some(e) => e.1 = y,
            None => self.el.push((n, y)),
        }
    }

    /// `ileri` sn sonra bir wc-saglik turu; cihazlar raporlarını yeni göndermiş olur.
    fn tur(&mut self, ileri: f64) {
        self.tur_r(ileri, true);
    }

    fn tur_r(&mut self, ileri: f64, rapor_yaz: bool) {
        self.t.ilerlet(ileri);
        let simdi = self.t.t();
        let mut ars = Map::new();
        for (n, yas) in self.el.clone() {
            let c = self.v.bagli_cihaz(n).unwrap();
            if rapor_yaz {
                if let Some(c) = &c {
                    let ek = self.rd.get(&n).cloned().unwrap_or_default();
                    self.v.saglik_kaydet(c.id, &rapor(&ek)).unwrap();
                }
            }
            let gun = ortak::now_iso(simdi)[..10].to_string();
            if matches!(yas, ElYas::Yok) {
                continue; // durum.json'da kaydı yok (arşiv henüz oluşmadı)
            }
            let el_sikisma = match yas {
                ElYas::Yas(y) => json!(simdi - y),
                _ => Value::Null,
            };
            ars.insert(n.to_string(), json!({"tur": "cihaz", "son_gun": self.sg.get(&n).cloned().unwrap_or(gun), "boyut": 0, "el_sikisma": el_sikisma}));
        }
        let mut d = json!({"zaman": ortak::now_iso(simdi), "arsivler": ars});
        if let Some(w) = self.wg_ok {
            d["wg_ok"] = json!(w);
        }
        calistir(&self.v, &d, &*self.g, simdi, &self.ld).unwrap();
    }

    fn calistir(&self, d: &Value, simdi: f64) {
        calistir(&self.v, d, &*self.g, simdi, &self.ld).unwrap();
    }

    fn acik(&self, n: i64) -> Vec<Sorun> {
        self.v.acik_sorunlar(Some(n)).unwrap()
    }

    fn acik_turler(&self, n: i64) -> Vec<String> {
        self.acik(n).into_iter().map(|s| s.tur).collect()
    }

    fn sorun(&self, n: i64, tur: &str) -> Sorun {
        self.acik(n).into_iter().find(|s| s.tur == tur).unwrap_or_else(|| panic!("{tur} açık değil"))
    }

    fn uyari_hareketleri(&self, n: Option<i64>) -> Vec<crate::veri::Hareket> {
        self.v.hareketler(n, 300, None).unwrap().into_iter().filter(|r| r.olay == "UYARI_SMS").collect()
    }

    fn kapanis_sms(&self, n: i64, tur: &str) -> String {
        self.v.son_kapanan(n, tur).unwrap().unwrap().kapanis_sms
    }
}

#[test]
fn sms_10_dk_sonra_ve_duzeldi() {
    let _k = Kayit::bas("test_saglik", "test_sms_10_dk_sonra_ve_duzeldi");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    assert!(o.acik_turler(n) == ["ulasilamiyor"] && o.g.giden().is_empty());
    o.tur(300.0);
    assert!(o.g.giden().is_empty());
    o.tur(299.0);
    assert!(o.g.giden().is_empty());
    o.tur(1.0); // 600 sn açık kaldı
    assert_eq!(o.g.giden(), [format!("WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (20:15'ten beri)")]);
    assert_eq!(o.sorun(n, "ulasilamiyor").acilis_sms, "gonderildi");
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1);
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0); // 1200 sn sonra düzeldi; "düzeldi" 600 sn temiz kalınca
    assert!(o.acik(n).is_empty() && o.g.giden().len() == 1);
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1);
    o.tur(300.0);
    assert_eq!(o.g.giden()[1], format!("WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor düzeldi (20 dk sürdü)"));
    assert_eq!(o.kapanis_sms(n, "ulasilamiyor"), "gonderildi");
    let h = o.uyari_hareketleri(Some(n));
    assert!(h.len() == 2 && h.iter().all(|r| r.kim == "saglik" && r.ip == "-"));
    o.tur(300.0);
    assert!(o.g.giden().len() == 2 && o.v.sms_sayaci(BUGUN).unwrap() == 2);
}

#[test]
fn titreyen_baglanti_tek_sms() {
    let _k = Kayit::bas("test_saglik", "test_titreyen_baglanti_tek_sms");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_v();
    // bir saat 9–11 dk arası gidip gelen el sıkışma: her kopukluk 600 sn temiz kalmadan döner → bildirilmemiş olsa da
    // aynı olay sürer (aynı satır, ilk açılış), 600 sn'yi bulunca tek SMS
    for _ in 0..6 {
        for yas in [660.0, 540.0] {
            o.el_koy(n, ElYas::Yas(yas));
            o.tur(300.0);
        }
    }
    assert_eq!(o.g.giden(), [format!("WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (20:20'ten beri)")]);
    assert_eq!(o.v.sorun_gecmisi(n, 50).unwrap().len(), 1);
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1);
    o.tur(300.0); // son kapanıştan (T0+3600) 600 sn temiz: tek "düzeldi", toplam süreyle
    assert_eq!(o.g.giden()[1], format!("WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor düzeldi (55 dk sürdü)"));
    o.tur(300.0);
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 2);
}

#[test]
fn bildirilmemis_kisa_kopma_sms_atmaz() {
    let _k = Kayit::bas("test_saglik", "test_bildirilmemis_kisa_kopma_sms_atmaz");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0);
    for _ in 0..4 {
        o.tur(300.0); // tek seferlik 300 sn'lik kopukluk: SMS yok
    }
    o.el_koy(n, ElYas::Yas(700.0));
    o.tur(300.0);
    o.tur(300.0); // 600 sn'den sonra yeniden: yeni olay, 600 sn dolmadı
    assert!(o.g.giden().is_empty() && o.v.sorun_gecmisi(n, 50).unwrap().len() == 2);
}

#[test]
fn kisa_kapanip_acilma_tek_olay() {
    let _k = Kayit::bas("test_saglik", "test_kisa_kapanip_acilma_tek_olay");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    o.tur(600.0);
    assert_eq!(o.g.giden().len(), 1);
    let ilk = o.sorun(n, "ulasilamiyor");
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0); // 300 sn temiz
    o.el_koy(n, ElYas::Yas(700.0));
    o.tur(300.0);
    o.tur(300.0);
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1); // düzeldi yok, yeni açılış yok
    let s = o.sorun(n, "ulasilamiyor");
    assert!(s.id == ilk.id && s.acilis == ilk.acilis && o.v.sorun_gecmisi(n, 50).unwrap().len() == 1);
}

#[test]
fn duzeldi_sonrasi_uzun_kesinti_sms_atar() {
    let _k = Kayit::bas("test_saglik", "test_duzeldi_sonrasi_uzun_kesinti_sms_atar");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    o.tur(600.0);
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0);
    o.tur(300.0);
    o.tur(300.0); // 600 sn temiz → düzeldi
    assert!(o.g.giden().len() == 2 && o.g.giden()[1].contains("düzeldi"));
    o.el_koy(n, ElYas::Yas(700.0));
    o.tur(300.0); // yeniden düştü (artık yeni olay)
    assert_eq!(o.g.giden().len(), 2);
    o.tur(300.0);
    o.tur(300.0); // 600 sn sürdü → yeni SMS
    assert!(o.g.giden().len() == 3 && o.g.giden()[2].starts_with(&format!("WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (")));
    for _ in 0..12 {
        o.tur(300.0); // saatlerce sürse de tek SMS
    }
    assert_eq!(o.g.giden().len(), 3);
}

#[test]
fn yeniden_calistirmada_cift_sms_yok() {
    let _k = Kayit::bas("test_saglik", "test_yeniden_calistirmada_cift_sms_yok");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    o.tur(600.0);
    assert_eq!(o.g.giden().len(), 1);
    o.tur(0.0); // aynı anda ikinci tur (timer kaçtı/çift tetiklendi)
    o.v = yeni_veri(&o.yol, &o.t); // merkez yeniden başladı
    o.tur(0.0);
    o.tur(300.0);
    assert!(o.g.giden().len() == 1 && o.v.acik_sorunlar(Some(n)).unwrap().len() == 1);
}

#[test]
fn gunluk_sinir() {
    let _k = Kayit::bas("test_saglik", "test_gunluk_sinir");
    let mut o = Ortam::ozel(true, 3, true, t0());
    let mut ns: Vec<i64> = (0..5).map(|i| o.musteri(&format!("K{i}"), "2027-10-11", ElYas::Yas(700.0)).0).collect(); // müşteri sırası numaraya göre
    ns.sort();
    o.musteri("Sağlam", "2027-10-11", ElYas::Yas(60.0)); // hepsi birden düşmüş olmasın (o, izleme arızası sayılır)
    o.tur(0.0);
    o.tur(600.0);
    assert!(o.g.giden().len() == 3 && o.v.sms_sayaci(BUGUN).unwrap() == 3);
    assert_eq!(o.g.giden()[2], "WifiCorrect: günlük SMS üst sınırına (3) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek.");
    let durumlar: Vec<String> = ns.iter().map(|n| o.sorun(*n, "ulasilamiyor").acilis_sms).collect();
    assert_eq!(durumlar, ["gonderildi", "gonderildi", "sinir:2026-10-11", "sinir:2026-10-11", "sinir:2026-10-11"]);
    for n in &ns {
        o.el_koy(*n, ElYas::Yas(60.0));
    }
    o.tur(300.0);
    o.tur(300.0);
    o.tur(300.0); // düzeldi: ilk ikisinin kapanışı sınıra takılır, üst sınır SMS'i ikinci kez gitmez
    assert_eq!(o.g.giden().len(), 3);
    // sınıra takılmış açılışın "düzeldi"si hiç gitmez (operatör açılışı almadı)
    let kapanislar: Vec<String> = ns.iter().map(|n| o.kapanis_sms(*n, "ulasilamiyor")).collect();
    assert_eq!(kapanislar, ["sinir:2026-10-11", "sinir:2026-10-11", "", "", ""]);
    assert_eq!(o.uyari_hareketleri(None).len(), 3);
    // ertesi gün sayaç sıfırdan
    o.el_koy(ns[0], ElYas::Yas(700.0));
    o.tur(86400.0);
    o.tur(600.0);
    assert!(o.g.giden().len() == 4 && o.v.sms_sayaci("2026-10-12").unwrap() == 1);
}

#[test]
fn bilgi_09da_toplu() {
    let _k = Kayit::bas("test_saglik", "test_bilgi_09da_toplu");
    let mut o = Ortam::yeni();
    let (x, _) = o.musteri("A Kafe", "2026-10-20", ElYas::Yas(60.0));
    let (y, _) = o.musteri("B Kafe", "2026-10-21", ElYas::Yas(60.0));
    let mut ikili = vec![(x, "A Kafe", "20.10.2026"), (y, "B Kafe", "21.10.2026")];
    ikili.sort();
    let [(a, ad_a, t_a), (b, ad_b, t_b)] = [ikili[0], ikili[1]];
    o.tur(0.0);
    let acik = o.v.acik_sorunlar(None).unwrap();
    assert!(acik.iter().all(|s| s.tur == "lisans_bitiyor") && acik.len() == 2);
    o.tur(3600.0);
    assert!(o.g.giden().is_empty()); // bilgi 600 sn sonra gitmez
    o.t.koy(ts("2026-10-12T08:55:00"));
    o.tur(0.0);
    assert!(o.g.giden().is_empty());
    o.tur(300.0); // 09:00 turu
    assert_eq!(o.g.giden().len(), 1);
    assert_eq!(
        o.g.giden()[0],
        format!("WifiCorrect bilgi: {a} {ad_a}: Lisans bitiyor ({t_a}); {b} {ad_b}: Lisans bitiyor ({t_b})")
    );
    assert!(o.v.acik_sorunlar(None).unwrap().iter().all(|s| s.acilis_sms == "gonderildi"));
    assert_eq!(o.uyari_hareketleri(None).len(), 2); // müşteri başına kayıt
    o.tur(300.0);
    o.tur(86400.0);
    assert_eq!(o.g.giden().len(), 1); // aynı sorun için tekrar yok
    o.v.lisans_ayarla(a, "2027-10-20").unwrap();
    o.tur(300.0);
    let kalan: Vec<i64> = o.v.acik_sorunlar(None).unwrap().iter().map(|s| s.musteri).collect();
    assert!(o.g.giden().len() == 1 && kalan == [b]); // bilgi kapanışı SMS'siz
    // b'nin lisansı bitti: "bitiyor" sessizce kapanır, "bitti" ayrı sorun olarak ertesi 09:00 özetine girer
    let ertesi = chrono::NaiveDate::parse_from_str(t_b, "%d.%m.%Y").unwrap() + chrono::Duration::days(1);
    o.t.koy(ts(&format!("{}T09:00:00", ertesi.format("%Y-%m-%d"))));
    o.tur(0.0);
    let bs: Vec<(String, String)> = o.v.acik_sorunlar(Some(b)).unwrap().into_iter().map(|s| (s.tur, s.metin)).collect();
    assert_eq!(bs, [("lisans_bitti".to_string(), format!("Lisans bitti ({t_b})"))]);
    assert!(o.g.giden().len() == 2 && o.g.giden()[1] == format!("WifiCorrect bilgi: {b} {ad_b}: Lisans bitti ({t_b})"));
}

#[test]
fn sms_hatasi_kaydedilir() {
    let _k = Kayit::bas("test_saglik", "test_sms_hatasi_kaydedilir");
    let mut o = Ortam::yeni();
    o.g.sonuc_koy("hata:30");
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.tur(0.0);
    o.tur(600.0);
    assert!(o.g.giden().len() == 1 && o.sorun(n, "ulasilamiyor").acilis_sms == "hata:30");
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1); // yeniden deneme yok
    let h = o.uyari_hareketleri(Some(n));
    assert!(h.len() == 1 && h[0].ayrinti.contains("hata:30"));
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0);
    assert!(o.g.giden().len() == 1 && o.kapanis_sms(n, "ulasilamiyor").is_empty());
}

#[test]
fn serbest_askida_uyarisiz() {
    let _k = Kayit::bas("test_saglik", "test_serbest_askida_uyarisiz");
    let mut o = Ortam::yeni();
    let (a, _) = o.musteri_el(ElYas::Yas(700.0));
    let (b, _) = o.musteri_el(ElYas::Yas(700.0));
    let (c, _) = o.musteri_el(ElYas::Yas(700.0));
    o.v.askiya_al(b, true).unwrap();
    o.v.serbest_birak(c, false).unwrap();
    o.tur(0.0);
    o.tur(600.0);
    assert!(o.g.giden().len() == 1 && o.v.acik_sorunlar(Some(b)).unwrap().is_empty() && o.v.acik_sorunlar(Some(c)).unwrap().is_empty());
    o.v.askiya_al(a, true).unwrap(); // bildirilmiş sorun askıya alınınca sessizce kapanır
    o.tur(300.0);
    assert!(o.v.acik_sorunlar(Some(a)).unwrap().is_empty() && o.g.giden().len() == 1);
}

#[test]
fn uyari_kapaliyken_sms_yok_kayit_var() {
    let _k = Kayit::bas("test_saglik", "test_uyari_kapaliyken_sms_yok_kayit_var");
    for (acik, telefon) in [(false, true), (true, false)] {
        let mut o = Ortam::ozel(acik, 20, telefon, t0());
        let (n, _) = o.musteri_el(ElYas::Yas(700.0));
        o.tur(0.0);
        o.tur(600.0);
        o.tur(300.0);
        assert!(o.g.giden().is_empty() && o.sorun(n, "ulasilamiyor").acilis_sms.is_empty());
        assert_eq!(o.v.sms_sayaci(BUGUN).unwrap(), 0);
    }
}

#[test]
fn ulasilamiyorken_rapor_kritikleri_duzeldi_sayilmaz() {
    let _k = Kayit::bas("test_saglik", "test_ulasilamiyorken_rapor_kritikleri_duzeldi_sayilmaz");
    let mut o = Ortam::yeni();
    let (n, cid) = o.musteri_v();
    o.v.saglik_kaydet(cid, &rapor(&[("kaydedici", json!(false))])).unwrap();
    o.tur_r(0.0, false);
    o.tur_r(600.0, false);
    assert!(o.acik_turler(n) == ["kayit_durdu"] && o.g.giden().len() == 1);
    o.el_koy(n, ElYas::Yas(700.0));
    o.tur_r(300.0, false);
    let mut turler = o.acik_turler(n);
    turler.sort();
    assert!(turler == ["kayit_durdu", "ulasilamiyor"] && o.g.giden().len() == 1);
}

#[test]
fn bayat_durumda_degerlendirme_yok_gunde_bir_sms() {
    let _k = Kayit::bas("test_saglik", "test_bayat_durumda_degerlendirme_yok_gunde_bir_sms");
    let t = t0();
    {
        let mut o = Ortam::yeni();
        o.musteri_el(ElYas::Hic);
        for d in [
            json!({}),
            json!({"zaman": "bozuk", "arsivler": {}}),
            json!({"zaman": ortak::now_iso(t - 1000.0), "arsivler": {}}),
            json!({"zaman": ortak::now_iso(t - 2000.0), "arsivler": {}}),
        ] {
            o.calistir(&d, t);
        }
        // neden türü başına günde bir: aynı neden ikinci kez gitmez, farklı neden ötekini yutmaz
        assert!(o.v.acik_sorunlar(None).unwrap().is_empty());
        assert_eq!(o.g.giden(), ["WifiCorrect: izleme çalışmıyor (durum.json yok)", "WifiCorrect: izleme çalışmıyor (durum.json bayat)"]);
        o.calistir(&json!({"zaman": ortak::now_iso(t), "arsivler": {}}), t + 86400.0 + 1000.0);
        assert_eq!(o.g.giden()[2..], ["WifiCorrect: izleme çalışmıyor (durum.json bayat)"]); // ertesi gün yine bir kez
        assert!(o.uyari_hareketleri(None).len() == 3 && o.v.sms_sayaci(BUGUN).unwrap() == 2);
    }
    let o = Ortam::ozel(false, 20, true, t0());
    o.calistir(&json!({}), t);
    assert!(o.g.giden().is_empty());
}

#[test]
fn el_sikisma_yasi_durum_zamanina_gore() {
    let _k = Kayit::bas("test_saglik", "test_el_sikisma_yasi_durum_zamanina_gore");
    let mut o = Ortam::yeni();
    let (n, cid) = o.musteri_v();
    o.v.saglik_kaydet(cid, &rapor(&[])).unwrap();
    let dz = t0() - 500.0; // durum.json 500 sn önce yazılmış, el sıkışma ondan 120 sn önce
    let mut d = json!({"zaman": ortak::now_iso(dz), "arsivler": {n.to_string(): {"son_gun": BUGUN, "el_sikisma": dz - 120.0}}});
    o.calistir(&d, t0());
    assert!(o.v.acik_sorunlar(Some(n)).unwrap().is_empty());
    d["arsivler"][n.to_string()]["el_sikisma"] = json!(dz - 601.0);
    o.calistir(&d, t0());
    assert_eq!(o.acik_turler(n), ["ulasilamiyor"]);
}

#[test]
fn tunel_bilgisi_okunamadi_toplu_ulasilamiyor_acilmaz() {
    let _k = Kayit::bas("test_saglik", "test_tunel_bilgisi_okunamadi_toplu_ulasilamiyor_acilmaz");
    let mut o = Ortam::yeni();
    o.wg_ok = None; // eski wc-durum: wg_ok yok, izlenen hepsinin el sıkışması yok → okunamadı sayılır
    let (a, _) = o.musteri_el(ElYas::Hic);
    let (b, _) = o.musteri_el(ElYas::Hic);
    o.tur(0.0);
    o.tur(600.0);
    o.tur(300.0);
    assert!(o.v.acik_sorunlar(None).unwrap().is_empty());
    assert_eq!(o.g.giden(), ["WifiCorrect: izleme çalışmıyor (tünel bilgisi okunamadı)"]);
    o.el_koy(b, ElYas::Yas(60.0)); // biri ayakta: ötekinin düşmesi gerçek
    o.tur(300.0);
    assert!(o.acik_turler(a) == ["ulasilamiyor"] && o.v.acik_sorunlar(Some(b)).unwrap().is_empty());
    o.el_koy(a, ElYas::Hic); // aynı gün durum.json bayatlarsa o ayrı bir neden: yine bildirilir
    o.el_koy(b, ElYas::Hic);
    o.tur(300.0);
    o.tur(300.0);
    let simdi = o.t.t();
    o.calistir(&json!({"zaman": ortak::now_iso(simdi - 1000.0), "arsivler": {}}), simdi);
    assert_eq!(o.g.giden()[1..], ["WifiCorrect: izleme çalışmıyor (durum.json bayat)"]);
}

#[test]
fn tunel_kaydi_olmayan_cihaz_toplu_kopmaya_sayilmaz() {
    let _k = Kayit::bas("test_saglik", "test_tunel_kaydi_olmayan_cihaz_toplu_kopmaya_sayilmaz");
    {
        // (a) iki yeni cihaz, ikisinin de kaydı yok: kurallar normal çalışır, tünel SMS'i yok
        let mut o = Ortam::yeni();
        let (a, cid) = o.musteri_el(ElYas::Yok);
        o.musteri_el(ElYas::Yok);
        o.v.saglik_kaydet(cid, &rapor(&[("kaydedici", json!(false))])).unwrap();
        o.tur_r(0.0, false);
        o.tur_r(600.0, false);
        assert!(o.acik_turler(a) == ["kayit_durdu"] && o.g.giden().len() == 1 && o.g.giden()[0].contains("5651 kaydı durdu"));
        assert!(!o.g.giden().iter().any(|g| g.contains("izleme")));
    }
    {
        // (b) biri yeni (kayıt yok), öteki gerçekten düşmüş: düşen için ulaşılamıyor açılır
        let mut o = Ortam::yeni();
        o.musteri_el(ElYas::Yok);
        let (b, _) = o.musteri_el(ElYas::Yas(700.0));
        o.tur(0.0);
        o.tur(600.0);
        assert!(o.acik_turler(b) == ["ulasilamiyor"]);
        assert_eq!(o.g.giden(), [format!("WifiCorrect {b} Göztepe: Cihaz ulaşılamıyor (20:15'ten beri)")]);
    }
}

#[test]
fn arsiv_kaydi_yoksa_ulasilamiyor_ve_yedek_acilmaz() {
    let _k = Kayit::bas("test_saglik", "test_arsiv_kaydi_yoksa_ulasilamiyor_ve_yedek_acilmaz");
    {
        let mut o = Ortam::yeni();
        let (n, _) = o.musteri_el(ElYas::Yok);
        o.tur(0.0);
        o.tur(600.0);
        o.tur(86400.0);
        assert!(o.v.acik_sorunlar(Some(n)).unwrap().is_empty() && o.g.giden().is_empty());
        o.tur(2.0 * 86400.0); // bağlanalı 3 gün: yedek gecikti
        assert_eq!(o.acik_turler(n), ["yedek_gecikti"]);
    }
    let t = t0();
    let m = m_ornek();
    let r = rapor(&[]);
    let mut c = c_ornek();
    c.baglanma = "2026-10-09T10:00:00+03:00".into();
    assert!(sorunlar(&m, Some(&c), Some(&r), Some(t - 60.0), None, None, t, false, None, None).is_empty());
    c.baglanma = "2026-10-08T23:00:00+03:00".into();
    assert_eq!(sorunlar(&m, Some(&c), Some(&r), Some(t - 60.0), None, None, t, false, None, None), vec![sr("yedek_gecikti", "Yedek gecikti", "uyari")]);
    let r = rapor(&[("kaydedici", json!(false))]);
    let turler: Vec<String> = sorunlar(&m, Some(&c), Some(&r), Some(t - 5000.0), None, None, t, false, None, None).into_iter().map(|s| s.0).collect();
    assert_eq!(turler, ["kayit_durdu", "yedek_gecikti"]); // rapor kuralları yine çalışır; tünel bilinmediğinden rapor_yok yok
}

#[test]
fn main_beklenmeyen_hatada_sms() {
    let _k = Kayit::bas("test_saglik", "test_main_beklenmeyen_hatada_sms");
    let o = Ortam::yeni();
    // main'in açtığı veritabanı ve uyarısı da sahte saatle (altın kayıt gerçek saate bağlı olmasın)
    let saat: Saat = Arc::new(t0);
    let bozuk = |_: &Veri, _: &Value, _: &dyn Tasiyici, _: f64| -> Result<(), CalHata> { Err(CalHata::yeni("RuntimeError", "deneme")) };
    assert_eq!(main_ile(&o.yol, &o.tmp.join("yok.json"), &*o.g, saat.clone(), &bozuk), 1);
    assert_eq!(o.g.giden(), ["WifiCorrect: izleme çalışmıyor (hata: RuntimeError)"]);
    // SMS denemesi de patlasa main düşmez
    assert_eq!(main_ile(&o.tmp.join("b.db"), &o.tmp.join("yok.json"), &Patlayan, saat, &bozuk), 1);
}

#[test]
fn sinira_takilan_acilis_ertesi_gun_bir_kez_gider() {
    let _k = Kayit::bas("test_saglik", "test_sinira_takilan_acilis_ertesi_gun_bir_kez_gider");
    let mut o = Ortam::ozel(true, 2, true, t0());
    let ad: HashMap<i64, &str> = ["A", "B"].iter().map(|a| (o.musteri(a, "2027-10-11", ElYas::Yas(700.0)).0, *a)).collect();
    o.musteri("Sağlam", "2027-10-11", ElYas::Yas(60.0));
    let mut sirali: Vec<i64> = ad.keys().copied().collect();
    sirali.sort();
    let (x, y) = (sirali[0], sirali[1]);
    o.tur(0.0);
    o.tur(600.0);
    assert_eq!(
        o.g.giden(),
        [
            format!("WifiCorrect {x} {}: Cihaz ulaşılamıyor (20:15'ten beri)", ad[&x]),
            "WifiCorrect: günlük SMS üst sınırına (2) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek.".to_string()
        ]
    );
    assert_eq!(o.sorun(y, "ulasilamiyor").acilis_sms, "sinir:2026-10-11");
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 2); // aynı gün yeniden denenmez
    o.t.koy(ts("2026-10-12T00:05:00"));
    o.tur(0.0); // ertesi gün y hâlâ açık: tam bir SMS
    assert_eq!(o.g.giden()[2..], [format!("WifiCorrect {y} {}: Cihaz ulaşılamıyor (20:15'ten beri)", ad[&y])]);
    o.tur(300.0);
    o.tur(300.0);
    assert!(o.g.giden().len() == 3 && o.sorun(y, "ulasilamiyor").acilis_sms == "gonderildi");
}

#[test]
fn sinira_takilan_acilisin_duzeldisi_gitmez() {
    let _k = Kayit::bas("test_saglik", "test_sinira_takilan_acilisin_duzeldisi_gitmez");
    let mut o = Ortam::ozel(true, 2, true, t0());
    let ad: HashMap<i64, &str> = ["A", "B"].iter().map(|a| (o.musteri(a, "2027-10-11", ElYas::Yas(700.0)).0, *a)).collect();
    o.musteri("Sağlam", "2027-10-11", ElYas::Yas(60.0));
    let mut sirali: Vec<i64> = ad.keys().copied().collect();
    sirali.sort();
    let y = sirali[1];
    o.tur(0.0);
    o.tur(600.0);
    o.el_koy(y, ElYas::Yas(60.0));
    o.tur(300.0);
    o.tur(300.0);
    o.tur(300.0);
    o.t.koy(ts("2026-10-12T00:05:00"));
    o.tur(0.0);
    o.tur(300.0);
    o.tur(300.0);
    assert!(o.g.giden().len() == 2 && !o.g.giden().iter().any(|g| g.contains(&format!("{y} "))));
    assert!(o.kapanis_sms(y, "ulasilamiyor").is_empty());
}

#[test]
fn ust_sinir_bildirimi_deneme_sayaci_doldurdugunda_da_gider() {
    let _k = Kayit::bas("test_saglik", "test_ust_sinir_bildirimi_deneme_sayaci_doldurdugunda_da_gider");
    let mut o = Ortam::ozel(true, 2, true, t0());
    o.v.sms_say(BUGUN).unwrap();
    o.v.sms_say(BUGUN).unwrap(); // deneme SMS'leri sayacı doldurdu
    let (n, _) = o.musteri_el(ElYas::Yas(700.0));
    o.musteri("Sağlam", "2027-10-11", ElYas::Yas(60.0));
    o.tur(0.0);
    o.tur(600.0); // ilk bastırılan uyarıda üst sınır bildirimi bir kez
    assert_eq!(o.g.giden(), ["WifiCorrect: günlük SMS üst sınırına (2) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."]);
    assert_eq!(o.sorun(n, "ulasilamiyor").acilis_sms, "sinir:2026-10-11");
    o.el_koy(n, ElYas::Yas(60.0));
    o.tur(300.0);
    o.tur(300.0);
    o.tur(300.0);
    assert_eq!(o.g.giden().len(), 1);
}

#[test]
fn son_tur_zamani_yazilir() {
    let _k = Kayit::bas("test_saglik", "test_son_tur_zamani_yazilir");
    let mut o = Ortam::yeni();
    assert!(o.v.saglik_son_tur().unwrap().is_none());
    o.musteri_v();
    o.tur(0.0);
    assert_eq!(o.v.saglik_son_tur().unwrap(), Some(ortak::now_iso(t0())));
    o.calistir(&json!({}), t0() + 300.0); // değerlendirme yapılamadı: tamamlanmış tur sayılmaz
    assert_eq!(o.v.saglik_son_tur().unwrap(), Some(ortak::now_iso(t0())));
    o.tur(600.0);
    assert_eq!(o.v.saglik_son_tur().unwrap(), Some(ortak::now_iso(t0() + 600.0)));
}

#[test]
fn wg_ok_kucuk_filoda_gercek_kopma() {
    let _k = Kayit::bas("test_saglik", "test_wg_ok_kucuk_filoda_gercek_kopma");
    {
        // `wg show` çalıştı: biri çoktan kapalı (hiç el sıkışmamış), öteki şimdi düştü → ikisi de gerçek
        let mut o = Ortam::yeni();
        let (a, _) = o.musteri_el(ElYas::Hic);
        let (b, _) = o.musteri_el(ElYas::Yas(700.0));
        o.tur(0.0);
        o.tur(600.0);
        assert!(o.acik_turler(a) == ["ulasilamiyor"] && o.acik_turler(b) == ["ulasilamiyor"]);
        assert!(o.g.giden().len() == 2 && !o.g.giden().iter().any(|g| g.contains("izleme")));
    }
    {
        // `wg show` okunamadı: tek cihaz olsa da değerlendirme yok, izleme SMS'i
        let mut o = Ortam::yeni();
        o.wg_ok = Some(false);
        o.musteri_el(ElYas::Hic);
        o.tur(0.0);
        o.tur(600.0);
        assert!(o.v.acik_sorunlar(None).unwrap().is_empty());
        assert_eq!(o.g.giden(), ["WifiCorrect: izleme çalışmıyor (tünel bilgisi okunamadı)"]);
    }
}

#[test]
fn acik_sorun_metni_guncellenir() {
    let _k = Kayit::bas("test_saglik", "test_acik_sorun_metni_guncellenir");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_v();
    o.rd.insert(n, vec![("disk_yuzde", json!(91))]);
    o.tur(0.0);
    let ilk = o.sorun(n, "disk_dolu");
    assert_eq!(ilk.metin, "Disk dolu (%91)");
    o.rd.insert(n, vec![("disk_yuzde", json!(95))]);
    o.tur(300.0);
    let s = o.sorun(n, "disk_dolu");
    assert!(s.id == ilk.id && s.metin == "Disk dolu (%95)" && o.g.giden().is_empty()); // SMS yok
    o.rd.insert(n, vec![("disk_yuzde", json!(97))]);
    o.tur(300.0); // 600 sn: SMS güncel metinle
    assert_eq!(o.g.giden(), [format!("WifiCorrect {n} Göztepe: Disk dolu (%97) (20:15'ten beri)")]);
}

#[test]
fn bilgi_toplu_en_cok_bes_musteri() {
    let _k = Kayit::bas("test_saglik", "test_bilgi_toplu_en_cok_bes_musteri");
    let mut o = Ortam::yeni();
    for i in 0..7 {
        o.musteri(&format!("K{i}"), "2026-10-20", ElYas::Yas(60.0));
    }
    o.tur(0.0);
    o.t.koy(ts("2026-10-12T09:00:00"));
    o.tur(0.0);
    assert_eq!(o.g.giden().len(), 1);
    let g = &o.g.giden()[0];
    assert!(g.starts_with("WifiCorrect bilgi: ") && g.matches("Lisans bitiyor").count() == 5 && g.ends_with("; +2 diğer"), "{g}");
    let acik = o.v.acik_sorunlar(None).unwrap();
    assert!(acik.iter().all(|s| s.acilis_sms == "gonderildi") && acik.len() == 7);
}

#[test]
fn netgsm_test_ortaminda_gercek_istek_yok() {
    let _k = Kayit::bas("test_saglik", "test_netgsm_test_ortaminda_gercek_istek_yok");
    // WC_TEST altında gerçek taşıyıcı (`Curl`) isteği curl'e hiç vermeden reddeder; netgsm bunu hata:AG sayar
    let istek = DisIstek {
        yontem: "POST".into(),
        url: crate::netgsm::URL.into(),
        basliklar: Vec::new(),
        govde: Some(b"x".to_vec()),
        kullanici: None,
        zaman_asimi: Duration::from_secs(10),
        okuma_siniri: 4096,
        yonlendirme: 0,
        indirme_sureleri: None,
    };
    assert_eq!(Curl.gonder_ortamla(&istek, Some("1".into())), Err("WC_TEST".to_string()));
    struct TestOrtami;
    impl Tasiyici for TestOrtami {
        fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String> {
            Curl.gonder_ortamla(i, Some("1".into()))
        }
    }
    let ayar = json!({"mock": false, "usercode": "u", "password": "p", "msgheader": "H"});
    assert_eq!(crate::netgsm::gonder(&ayar, "m", &["905334553132"], &TestOrtami), "hata:AG");
}

fn liste_meta(dizin: &std::path::Path, kategori: &str, alanlar: Value) {
    std::fs::create_dir_all(dizin).unwrap();
    let mut m = Map::new();
    m.insert("kategori".into(), json!(kategori));
    for (k, v) in alanlar.as_object().unwrap() {
        m.insert(k.clone(), v.clone());
    }
    std::fs::write(dizin.join(format!("{kategori}.json")), serde_json::to_vec(&Value::Object(m)).unwrap()).unwrap();
}

#[test]
fn liste_sorunu_saf() {
    let _k = Kayit::bas("test_saglik", "test_liste_sorunu_saf");
    let tmp = gecici_dizin();
    let d = tmp.join("l");
    let t = t0();
    assert!(liste_sorunu(&d, t).is_none()); // dizin yok (merkez yeni kuruldu, liste hiç üretilmedi)
    let taze = ortak::now_iso(t - 3600.0);
    let (a, b, c) = ("a".repeat(64), "b".repeat(64), "c".repeat(64));
    liste_meta(&d, "kumar", json!({"sha256": a, "sayi": 5, "zaman": taze, "kaynaklar": [{"ad": "HaGeZi", "sayi": 5}]}));
    // tek kaynağın hatası (USOM HTML döndü) kategori başarılıysa sorun değildir
    liste_meta(
        &d,
        "zararli",
        json!({"sha256": b, "sayi": 5, "zaman": taze, "kaynaklar": [{"ad": "USOM", "sayi": 0, "hata": "liste yerine HTML sayfası döndü"}, {"ad": "HaGeZi", "sayi": 5}]}),
    );
    assert!(liste_sorunu(&d, t).is_none());
    liste_meta(&d, "yetiskin", json!({"sha256": c, "sayi": 5, "zaman": taze, "hata": "Hiçbir kaynak indirilemedi; önceki liste kullanılıyor."}));
    assert_eq!(liste_sorunu(&d, t).as_deref(), Some("Engel listeleri güncellenemiyor: Yetişkin içerik"));
    liste_meta(&d, "kumar", json!({"sha256": a, "sayi": 5, "zaman": ortak::now_iso(t - 48.0 * 3600.0 - 1.0)})); // 48 saatten eski
    assert_eq!(liste_sorunu(&d, t).as_deref(), Some("Engel listeleri güncellenemiyor: Kumar ve bahis, Yetişkin içerik"));
    liste_meta(&d, "kumar", json!({"sha256": a, "sayi": 5, "zaman": ortak::now_iso(t - 48.0 * 3600.0 + 60.0)}));
    liste_meta(&d, "yetiskin", json!({"hata": "Hiçbir kaynak indirilemedi; önceki liste kullanılıyor."})); // hiç başarılı olmadı
    assert_eq!(liste_sorunu(&d, t).as_deref(), Some("Engel listeleri güncellenemiyor: Yetişkin içerik"));
    std::fs::write(d.join("yetiskin.json"), "{bozuk").unwrap();
    assert!(liste_sorunu(&d, t).is_none()); // okunamayan meta: liste yok sayılır (yönetim sayfası gösterir)
}

#[test]
fn liste_sorunu_merkez_satiri_ve_sms() {
    let _k = Kayit::bas("test_saglik", "test_liste_sorunu_merkez_satiri_ve_sms");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_v();
    let sha = "a".repeat(64);
    liste_meta(&o.ld, "kumar", json!({"sha256": sha, "sayi": 5, "zaman": ortak::now_iso(t0() - 3600.0), "hata": "Hiçbir kaynak indirilemedi."}));
    o.tur(0.0);
    let a = o.v.acik_sorunlar(Some(MERKEZ)).unwrap();
    assert!(a.len() == 1 && a[0].tur == "liste_guncellenemiyor" && a[0].onem == "uyari");
    assert!(o.acik(n).is_empty() && o.g.giden().is_empty());
    o.tur(600.0);
    assert_eq!(o.g.giden(), ["WifiCorrect merkez: Engel listeleri güncellenemiyor: Kumar ve bahis (20:15'ten beri)"]);
    for _ in 0..5 {
        o.tur(3600.0);
    }
    assert_eq!(o.g.giden().len(), 1); // açık kaldıkça yeni SMS yok
    liste_meta(&o.ld, "kumar", json!({"sha256": sha, "sayi": 5, "zaman": ortak::now_iso(o.t.t())}));
    o.tur(300.0);
    assert!(o.v.acik_sorunlar(Some(MERKEZ)).unwrap().is_empty() && o.g.giden().len() == 1);
    o.tur(600.0);
    assert!(o.g.giden().len() == 2 && o.g.giden()[1].starts_with("WifiCorrect merkez: Engel listeleri güncellenemiyor: Kumar ve bahis düzeldi"));
}

#[test]
fn liste_denetimi_durum_bozukken_de_calisir() {
    let _k = Kayit::bas("test_saglik", "test_liste_denetimi_durum_bozukken_de_calisir");
    let o = Ortam::yeni();
    liste_meta(&o.ld, "kumar", json!({"hata": "Hiçbir kaynak indirilemedi."}));
    o.calistir(&json!({}), o.t.t()); // durum.json yok
    let turler: Vec<String> = o.v.acik_sorunlar(Some(MERKEZ)).unwrap().into_iter().map(|s| s.tur).collect();
    assert_eq!(turler, ["liste_guncellenemiyor"]);
}

#[test]
fn merkez_sms_hatasi_sorun_acar_ve_kapatir() {
    let _k = Kayit::bas("test_saglik", "test_merkez_sms_hatasi_sorun_acar_ve_kapatir");
    let mut o = Ortam::yeni();
    let (n, _) = o.musteri_v();
    o.tur(0.0);
    assert!(o.acik(n).is_empty());
    o.v.sms_son_kaydet(n, "netgsm", "hata", "30").unwrap();
    o.tur(60.0);
    assert_eq!(o.acik_turler(n), ["sms_calismiyor"]);
    o.v.sms_son_kaydet(n, "netgsm", "gonderildi", "0").unwrap();
    o.tur(60.0);
    assert!(o.acik(n).is_empty());
}
