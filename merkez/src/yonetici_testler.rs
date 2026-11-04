//! yonetici.rs testleri: Python `yonetici.main` assert'leri (test_yonetim.py 45–51, 215) Task 11'de o test işlevleriyle altınla
//! gelir; burada aynı davranış kayıtsız birim testleriyle sabitlenir (çıkış kodları, kısa parola, geçersiz ad, dosya izni,
//! sorma/inme sırası, Python'un yazdığı dosyayı okuma).
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::test_yardim as Y;

const PW: &str = "yonetici-parola-1";

fn s(a: &[&str]) -> Vec<String> {
    a.iter().map(|x| x.to_string()).collect()
}

#[test]
fn yaz_gecersiz_ad_ve_kisa_parola_reddeder() {
    let yol = Y::gecici_dizin().join("y.json");
    assert_eq!(yaz(&yol, "serkan", "kisa-parola"), Err("Parola en az 12 karakter olmalı.".to_string()));
    assert_eq!(yaz(&yol, "Kötü Ad", PW), Err("Kullanıcı adı 3-32 karakter: küçük harf, rakam, . _ -".to_string()));
    assert_eq!(yaz(&yol, "ab", PW).unwrap_err(), "Kullanıcı adı 3-32 karakter: küçük harf, rakam, . _ -");
    assert!(yaz(&yol, &"a".repeat(33), PW).is_err() && yaz(&yol, "a.b_c-1", PW).is_ok());
    assert!(!yol.with_extension("json.tmp").exists());
    // 12 karakter: bayt değil karakter sayılır
    assert!(yaz(&yol, "serkan", "ğğğğğğğğğğğğ").is_ok());
    assert!(yaz(&yol, "serkan", "ğğğğğğğğğğğ").is_err());
}

#[test]
fn main_yazar_okur_ve_dogrular() {
    let yol = Y::gecici_dizin().join("y.json");
    let sor = || Some(PW.to_string());
    assert_eq!(main(&s(&["parola", "serkan"]), &yol, &sor, None, &|_| Ok(())), 0);
    let h = oku(&yol).unwrap();
    assert_eq!((h.kullanici.as_str(), h.yineleme), ("serkan", 120_000));
    assert!(guvenlik::dogru(PW, &h.tuz, &h.ozet, h.yineleme) && !guvenlik::dogru("yanlis-parola-1", &h.tuz, &h.ozet, h.yineleme));
    assert_eq!(std::fs::metadata(&yol).unwrap().permissions().mode() & 0o777, 0o600);
    // Python common.save_json biçimi: girinti 1, anahtar sırası kullanici, tuz, ozet, yineleme
    let ham = std::fs::read_to_string(&yol).unwrap();
    assert!(ham.starts_with("{\n \"kullanici\": \"serkan\",\n \"tuz\": \"") && ham.contains("\",\n \"ozet\": \"") && ham.ends_with("\",\n \"yineleme\": 120000\n}"), "{ham}");
    // geçersiz ad: kullanım, 2
    assert_eq!(main(&s(&["parola", "Kötü Ad"]), &yol, &sor, None, &|_| Ok(())), 2);
    assert_eq!(main(&s(&["parola"]), &yol, &sor, None, &|_| Ok(())), 2);
    assert_eq!(main(&s(&["baska", "serkan"]), &yol, &sor, None, &|_| Ok(())), 2);
    assert_eq!(main(&s(&[]), &yol, &sor, None, &|_| Ok(())), 2);
    // parolalar farklı: 1, dosya değişmez
    assert_eq!(main(&s(&["parola", "baska"]), &yol, &|| None, None, &|_| Ok(())), 1);
    assert_eq!(oku(&yol).unwrap().kullanici, "serkan");
    // kısa parola: 1
    assert_eq!(main(&s(&["parola", "serkan"]), &yol, &|| Some("kisa".into()), None, &|_| Ok(())), 1);
    assert_eq!(oku(&yol).unwrap(), h);
}

#[test]
fn parola_yetki_birakilmadan_once_sorulur() {
    // wcpanel terminale erişemez: parola root iken sorulur
    let yol = Y::gecici_dizin().join("y.json");
    let sira = RefCell::new(Vec::<String>::new());
    let kod = main(
        &s(&["parola", "serkan"]),
        &yol,
        &|| {
            sira.borrow_mut().push("sor".into());
            Some(PW.into())
        },
        Some("wcpanel"),
        &|k| {
            sira.borrow_mut().push(format!("birak:{k}"));
            Ok(())
        },
    );
    assert!(kod == 0 && *sira.borrow() == ["sor", "birak:wcpanel"]);
    // inilemezse dosyaya dokunulmaz
    let yol2 = Y::gecici_dizin().join("y.json");
    assert_eq!(main(&s(&["parola", "serkan"]), &yol2, &|| Some(PW.into()), Some("wcpanel"), &|_| Err("inilemedi".into())), 1);
    assert!(!yol2.exists());
}

#[test]
fn oku_eksik_bozuk_ve_python_dosyasi() {
    let d = Y::gecici_dizin();
    assert_eq!(oku(&d.join("yok.json")), None);
    std::fs::write(d.join("b.json"), "{bozuk").unwrap();
    assert_eq!(oku(&d.join("b.json")), None);
    std::fs::write(d.join("e.json"), r#"{"kullanici": "x", "tuz": "t", "ozet": "o"}"#).unwrap();
    assert_eq!(oku(&d.join("e.json")), None); // alan eksik
    std::fs::write(d.join("l.json"), "[1]").unwrap();
    assert_eq!(oku(&d.join("l.json")), None);
    // Python yonetici.yaz(yol, "serkan", "yonetici-parola-1") ile üretilmiş dosya (common.save_json)
    std::fs::write(
        d.join("p.json"),
        "{\n \"kullanici\": \"serkan\",\n \"tuz\": \"ac645042424d87246c8d5439d34d349d\",\n \"ozet\": \"51a9b7a9f14cf006a946de08ee4c2030417161f06d692f371e84c816889154c4\",\n \"yineleme\": 120000\n}",
    )
    .unwrap();
    let h = oku(&d.join("p.json")).unwrap();
    assert_eq!(h.kullanici, "serkan");
    assert!(guvenlik::dogru(PW, &h.tuz, &h.ozet, h.yineleme));
    // fazladan alan zararsız (Python yalnızca dört anahtarın varlığına bakar)
    std::fs::write(d.join("f.json"), r#"{"kullanici": "x", "tuz": "t", "ozet": "o", "yineleme": 5, "ek": 1}"#).unwrap();
    assert_eq!(oku(&d.join("f.json")).unwrap().yineleme, 5);
}

#[test]
fn kullanici_adi_kurali() {
    for iyi in ["abc", "a.b_c-d", "serkan", &"a".repeat(32)] {
        assert!(kullanici_gecerli(iyi), "{iyi}");
    }
    for kotu in ["ab", "", "Serkan", "a b", "a/b", "ş.ş.ş", &"a".repeat(33), "abc\n"] {
        assert!(!kullanici_gecerli(kotu), "{kotu:?}");
    }
}
