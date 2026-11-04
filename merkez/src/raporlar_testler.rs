//! raporlar.rs testleri (Python tests/test_raporlar.py'nin uygulamadan bağımsız kısmı). Musteri/Yonetim uygulamasından
//! geçen testler uygulama görevlerinde taşınır: test_musteri_liste_sayfasi, test_musteri_menu_ve_rapor_sayfasi,
//! test_hizmet_oncesi_gunler, test_musteri_yollar_404_ve_baska_musteri, test_sayfa_arsiv_ve_ham_okumaz,
//! test_cihaz_bolumu_sorunlar_ve_cevrimdisi, test_yonetim_raporlar_gerekcesiz, test_uctan_uca_kisisel_veri_yok.
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{gecici_dizin, SAAT_BASLANGIC};

fn gunler(bas: &str, n: i64) -> Vec<Value> {
    let t = NaiveDate::parse_from_str(bas, "%Y-%m-%d").unwrap();
    (0..n).map(|i| json!({"gun": (t + Duration::days(i)).format("%Y-%m-%d").to_string(), "durum": "tam", "neden": "", "farkli": 2})).collect()
}

/// Python `r.update(degisik)`.
fn guncelle(mut r: Value, degisik: &[(&str, Value)]) -> Value {
    for (k, v) in degisik {
        r[*k] = v.clone();
    }
    r
}

/// 2026-09 (panel saati 21.09.2026 17:13: süren dönem).
fn ay_raporu(degisik: &[(&str, Value)]) -> Value {
    let mut g = gunler("2026-09-01", 30);
    for x in g[20..].iter_mut() {
        x["durum"] = json!("gelecek");
        x["farkli"] = Value::Null;
    }
    g[20]["durum"] = json!("bugun");
    g[19]["durum"] = json!("bekleniyor");
    g[19]["farkli"] = Value::Null;
    g[3]["durum"] = json!("dogrulanamadi");
    g[3]["neden"] = json!("zincir_yok");
    g[4]["durum"] = json!("kayit_yok");
    g[4]["farkli"] = Value::Null;
    g[5]["durum"] = json!("eksik");
    g[5]["farkli"] = Value::Null;
    let mut saat = vec![0; 9];
    saat.extend([5, 9]);
    saat.extend(vec![1; 13]);
    let r = json!({
        "bicim": 2, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30",
        "uretim": "2026-09-21T12:31:07+03:00", "kesin": false, "imza": "x".repeat(64),
        "misafir": {"farkli": 41, "yeni": 30, "donen": 11, "giris": 77, "giris_sms": 70, "giris_kod": 7, "izinli_cihaz": 2,
                    "gunluk_ortalama": 4.5, "en_yogun_gun": {"gun": "2026-09-12", "farkli": 9}},
        "saat": saat, "hafta_gunu": [1, 2, 3, 4, 5, 6, 7],
        "gunler": g, "kayit": {"gun": 20, "tam": 16},
        "kullanim": {"gonderilen": 1024u64.pow(3), "alinan": 3 * 1024u64.pow(3), "sms": 70,
                     // eski biçimden kalan alan adı listesi: hiç okunmaz, gösterilmez
                     "alanlar": [{"alan": "instagram.com", "misafir": 12}, {"alan": "<script>alert(1)</script>.com", "misafir": 3}]},
        "surum": "0.4.9", "surum_sonra_degisti": false
    });
    guncelle(r, degisik)
}

fn liste_ornegi() -> Vec<Value> {
    vec![
        json!({"tur": "hafta", "donem": "2026-W39", "baslangic": "2026-09-21", "bitis": "2026-09-27", "kesin": false, "farkli": 3, "kayit_gun": 0, "kayit_tam": 0}),
        json!({"tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20", "kesin": false, "farkli": 6, "kayit_gun": 7, "kayit_tam": 6}),
        json!({"tur": "hafta", "donem": "2026-W36", "baslangic": "2026-08-31", "bitis": "2026-09-06", "kesin": true, "farkli": 8, "kayit_gun": 7, "kayit_tam": 7}),
        json!({"tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30", "kesin": false, "farkli": 41, "kayit_gun": 20, "kayit_tam": 16}),
        json!({"tur": "ay", "donem": "2026-08", "baslangic": "2026-08-01", "bitis": "2026-08-31", "kesin": true, "farkli": 50, "kayit_gun": 31, "kayit_tam": 31}),
    ]
}

/// Python `yaz(tmp, numara, ad, icerik)`: metin olduğu gibi, değilse JSON.
fn yaz(tmp: &Path, numara: &str, ad: &str, icerik: &Value) {
    let d = tmp.join("rapor").join(numara);
    fs::create_dir_all(&d).unwrap();
    let m = match icerik {
        Value::String(s) => s.clone(),
        v => serde_json::to_string(v).unwrap(),
    };
    fs::write(d.join(ad), m).unwrap();
}

fn veri_(r: &Rapor, cihaz_surumu: &str) -> RaporVerisi {
    RaporVerisi { rapor: r.clone(), isletme: isletme_bilgisi(None, 1234567), cihaz: cihaz_bolumu(r, &[], cihaz_surumu, SAAT_BASLANGIC) }
}

fn normal(r: &Value) -> Rapor {
    normal_rapor(r, "ay", "2026-09").unwrap()
}

// --- Raporlar sınıfı ---
#[test]
fn liste_ve_rapor_okuma_dogrulama() {
    let _k = Kayit::bas("test_raporlar", "test_liste_ve_rapor_okuma_dogrulama");
    let tmp = gecici_dizin();
    let rp = Raporlar::new(tmp.join("rapor"));
    assert!(rp.liste(1234567).is_empty() && rp.rapor(1234567, "ay", "2026-09").is_none()); // klasör yok
    let mut l = liste_ornegi();
    l.extend([json!({"tur": "yil", "donem": "2026"}), json!("x"), json!({"tur": "ay", "donem": "../x"})]);
    yaz(&tmp, "1234567", "liste.json", &Value::Array(l));
    let donemler: Vec<String> = rp.liste(1234567).into_iter().map(|x| x.donem).collect();
    assert_eq!(donemler, vec!["2026-W39", "2026-W38", "2026-W36", "2026-09", "2026-08"]);
    yaz(&tmp, "1234567", "ay-2026-09.json", &ay_raporu(&[]));
    assert_eq!(rp.rapor(1234567, "ay", "2026-09").unwrap().misafir.farkli, 41);
    for (tur, donem) in [("ay", "2026-13"), ("ay", "2026-00"), ("hafta", "2026-W54"), ("hafta", "2026-W00"), ("hafta", "../x"), ("yil", "2026"), ("ay", "2026-9"), ("ay", "２０２６-09")] {
        assert!(rp.rapor(1234567, tur, donem).is_none(), "{tur} {donem}");
    }
    for (tur, donem) in [("ay", "2026-13"), ("ay", "2026-00"), ("hafta", "2026-W54"), ("hafta", "2026-W00"), ("yil", "2026")] {
        assert!(!donem_gecerli(tur, donem), "{tur} {donem}");
    }
    assert!(donem_gecerli("hafta", "2026-W53") && donem_gecerli("ay", "2026-12"));
    assert!(rp.rapor(7654321, "ay", "2026-09").is_none() && rp.rapor("../1234567", "ay", "2026-09").is_none());
    yaz(&tmp, "1234567", "ay-2026-08.json", &json!("{bozuk"));
    assert!(rp.rapor(1234567, "ay", "2026-08").is_none());
    yaz(&tmp, "1234567", "ay-2026-08.json", &ay_raporu(&[("bicim", json!(3)), ("donem", json!("2026-08"))]));
    assert!(rp.rapor(1234567, "ay", "2026-08").is_none());
    // eski biçim (1, alan adı listeli): Python root yenileyene kadar okurdu; Rust'ta okunmaz (spec bilinçli fark 6)
    yaz(&tmp, "1234567", "ay-2026-08.json", &ay_raporu(&[("bicim", json!(1)), ("donem", json!("2026-08"))]));
    assert!(rp.rapor(1234567, "ay", "2026-08").is_none());
    yaz(&tmp, "1234567", "ay-2026-08.json", &ay_raporu(&[])); // dosya adı ile içerik farklı dönem
    assert!(rp.rapor(1234567, "ay", "2026-08").is_none());
    yaz(&tmp, "1234567", "liste.json", &json!("[bozuk"));
    assert!(rp.liste(1234567).is_empty());
}

#[test]
fn rapor_turleri_denetlenir() {
    let _k = Kayit::bas("test_raporlar", "test_rapor_turleri_denetlenir");
    let tmp = gecici_dizin();
    let rp = Raporlar::new(tmp.join("rapor"));
    let r = ay_raporu(&[
        ("misafir", json!({"farkli": "41", "yeni": -3, "donen": true, "giris": 1.5, "gunluk_ortalama": "x", "en_yogun_gun": {"gun": "<b>", "farkli": 3}})),
        ("saat", json!(vec!["a"; 30])),
        ("hafta_gunu", json!("x")),
        ("gunler", json!([{"gun": "2026-09-01", "durum": "<i>", "neden": 5, "farkli": -1}, "bozuk", {"gun": "x"}])),
        ("kayit", json!([])),
        ("kullanim", json!({"gonderilen": "1", "alanlar": [{"alan": 5, "misafir": 3}, "x", {"alan": "a.com", "misafir": "9"}]})),
        ("surum", json!("<script>")),
        ("kesin", json!("evet")),
    ]);
    yaz(&tmp, "1234567", "ay-2026-09.json", &r);
    let n = rp.rapor(1234567, "ay", "2026-09").unwrap();
    assert!(n.misafir.farkli == 0 && n.misafir.yeni == 0 && n.misafir.donen == 0);
    assert!(n.misafir.giris == 0 && n.misafir.gunluk_ortalama == 0.0 && n.misafir.en_yogun_gun.is_none());
    assert!(n.saat == vec![0; 24] && n.hafta_gunu == vec![0; 7] && (n.kayit_gun, n.kayit_tam) == (0, 0));
    assert_eq!(n.gunler, vec![GunSatiri { gun: "2026-09-01".into(), durum: "bilinmiyor", neden: "", farkli: None }]);
    assert_eq!((n.gonderilen, n.alinan, n.sms), (0, 0, 0));
    assert!(n.surum.is_none() && !n.kesin);
    let s = rapor_html(&veri_(&n, ""), SAAT_BASLANGIC, ""); // çökmez
    assert!(!s.contains("<script"));
}

#[test]
fn surum_kurali() {
    let _k = Kayit::bas("test_raporlar", "test_surum_kurali");
    let t = SAAT_BASLANGIC;
    let r = normal(&ay_raporu(&[("surum", Value::Null), ("surum_sonra_degisti", json!(true))]));
    assert!(cihaz_bolumu(&r, &[], "0.5.0", t).surum.is_none());
    assert!(!rapor_html(&veri_(&r, "0.5.0"), t, "").contains("0.5.0"));
    let r = normal(&ay_raporu(&[("surum", Value::Null)]));
    assert_eq!(cihaz_bolumu(&r, &[], "0.5.0", t).surum.as_deref(), Some("0.5.0"));
    assert!(cihaz_bolumu(&r, &[], "<b>", t).surum.is_none()); // cihazın bildirdiği biçimsiz sürüm alınmaz
    assert!(rapor_html(&veri_(&r, "0.5.0"), t, "").contains("0.5.0"));
    let r = normal(&ay_raporu(&[("surum", json!("0.4.7")), ("surum_sonra_degisti", json!(true))]));
    let s = rapor_html(&veri_(&r, "0.5.0"), t, "");
    assert!(s.contains("0.4.7") && !s.contains("0.5.0"));
}

#[test]
fn turkce_ek() {
    let _k = Kayit::bas("test_raporlar", "test_turkce_ek");
    let beklenen = ["ı", "i", "si", "ü", "ü", "i", "sı", "si", "i", "u", "u", "si", "u", "i", "ı", "si", "ı", "ü"];
    let girdi = [0u64, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 31, 40, 50, 60, 100];
    assert_eq!(girdi.iter().map(|x| iyelik(*x)).collect::<Vec<_>>(), beklenen);
}
