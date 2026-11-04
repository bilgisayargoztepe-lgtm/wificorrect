//! durum.rs testleri: Python tests/test_durum.py'nin karşılığı (altın kayıtlı).
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::gecici_dizin;

fn pub_() -> String {
    format!("{}=", "A".repeat(43))
}

/// Python `ortam(tmp)` → (kayit, yeni, eski).
fn ortam(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let kayit = tmp.join("kayit.csv");
    let b = format!("{}=", "B".repeat(43));
    std::fs::write(
        &kayit,
        format!(
            "yonetici;pc;10.99.0.2;{p};\ncihaz;100001;10.99.0.10;{p};wfc-100001\nbitti;bocafe-test;10.99.0.11;{b};wfc-bocafe-test;2026-10-04\n",
            p = pub_()
        ),
    )
    .unwrap();
    let (yeni, eski) = (tmp.join("yeni"), tmp.join("eski"));
    std::fs::create_dir_all(yeni.join("100001/veri/gunluk/2026-10-03")).unwrap();
    std::fs::create_dir_all(yeni.join("bocafe-test/veri/gunluk/2026-10-01")).unwrap();
    std::fs::create_dir_all(yeni.join("..kotu/veri")).unwrap();
    std::fs::create_dir_all(eski.join("bocafe/gunluk/2026-09-20")).unwrap();
    std::fs::write(yeni.join("100001/veri/zincir.txt"), "x".repeat(100)).unwrap();
    (kayit, yeni, eski)
}

fn adlar(a: &[Arsiv]) -> Vec<&str> {
    let mut v: Vec<&str> = a.iter().map(|x| x.ad.as_str()).collect();
    v.sort();
    v
}

#[test]
fn arsivler_listesi() {
    let _k = Kayit::bas("test_durum", "test_arsivler");
    let tmp = gecici_dizin();
    let (kayit, yeni, eski) = ortam(&tmp);
    let a = arsivler(&kayit, &yeni, &eski).unwrap();
    assert_eq!(adlar(&a), ["100001", "bocafe", "bocafe-test"]);
    let bir = a.iter().find(|x| x.ad == "100001").unwrap();
    assert_eq!(
        *bir,
        Arsiv { ad: "100001".into(), tur: "cihaz".into(), wg_pub: pub_(), kaynaklar: vec![yeni.join("100001").join("veri")] }
    );
    let tur = |ad: &str| a.iter().find(|x| x.ad == ad).unwrap().tur.clone();
    assert!(tur("bocafe-test") == "bitti" && tur("bocafe") == "eski");
    // ekleme sırası: yeni arşiv sıralı, eski yalnızca-eski adlar sonda
    let sira: Vec<&str> = a.iter().map(|x| x.ad.as_str()).collect();
    assert_eq!(sira, ["100001", "bocafe-test", "bocafe"]);
}

#[test]
fn yardimcilar() {
    let _k = Kayit::bas("test_durum", "test_yardimcilar");
    let b = format!("{}=", "B".repeat(43));
    let beklenen: BTreeMap<String, u64> = [(pub_(), 1_790_000_000u64)].into_iter().collect();
    assert_eq!(el_sikismalari(&format!("{}\t1790000000\nbozuk\n{b}\t0\n", pub_())), beklenen);
    // spec bilinçli fark 5: yalnızca ASCII rakam (Python str.isdigit Unicode rakamı da kabul ederdi)
    assert!(el_sikismalari(&format!("{}\t١٢٣\n{}\t12\u{b2}\n{}\t+5\n", pub_(), pub_(), pub_())).is_empty());
    let tmp = gecici_dizin();
    let (_kayit, yeni, _eski) = ortam(&tmp);
    let k = vec![yeni.join("100001").join("veri")];
    assert_eq!(son_gun(&k).unwrap().as_deref(), Some("2026-10-03"));
    assert_eq!(son_gun(&[tmp.join("yok")]).unwrap(), None);
    assert_eq!(boyut(&k), 100);
}

#[test]
fn durum_json_yazilir() {
    let _k = Kayit::bas("test_durum", "test_main");
    let tmp = gecici_dizin();
    let (kayit, yeni, eski) = ortam(&tmp);
    let cikti = tmp.join("durum.json");
    let y = Yollar { kayit, yeni, eski, cikti: cikti.clone(), grup: None };
    let metin = format!("{}\t1790000000\n", pub_());
    assert_eq!(calistir(&y, &|| Some(metin.clone()), 1_790_000_100.0), 0);
    let d: Value = serde_json::from_slice(&std::fs::read(&cikti).unwrap()).unwrap();
    assert_eq!(d["arsivler"]["100001"], json!({"tur": "cihaz", "son_gun": "2026-10-03", "boyut": 100, "el_sikisma": 1790000000}));
    assert!(d["arsivler"]["bocafe"]["el_sikisma"].is_null() && d["zaman"].as_str().unwrap().starts_with("2026-"));
    assert_eq!(d["wg_ok"], json!(true));
    assert_eq!(std::fs::metadata(&cikti).unwrap().permissions().mode() & 0o777, 0o640);
    // `wg show` başarısız
    assert_eq!(calistir(&y, &|| None, 1_790_000_100.0), 0);
    let d: Value = serde_json::from_slice(&std::fs::read(&cikti).unwrap()).unwrap();
    assert!(d["wg_ok"] == json!(false) && d["arsivler"]["100001"]["el_sikisma"].is_null());
}

#[test]
fn wg_komutu_sonucu() {
    let _k = Kayit::bas("test_durum", "test_wg_basarisi");
    let metin = format!("{}\t1790000000\n", pub_());
    assert_eq!(wg_oku(&|| Ok((0, metin.clone()))), Some(metin.clone()));
    assert_eq!(wg_oku(&|| Ok((0, String::new()))), Some(String::new())); // çalıştı, hiç eş yok
    assert_eq!(wg_oku(&|| Ok((1, String::new()))), None); // arayüz yok / yetki yok
    assert_eq!(wg_oku(&|| Err("OSError: wg yok".into())), None);
}

#[test]
fn boyut_sembolik_baglar_python_walk_gibi() {
    let tmp = crate::ortak::tests::gecici_dizin();
    let k = tmp.join("k");
    std::fs::create_dir_all(k.join("alt")).unwrap();
    std::fs::write(k.join("a"), "12345").unwrap();
    std::fs::write(k.join("alt/b"), "123").unwrap();
    std::os::unix::fs::symlink(&tmp, k.join("dizin_bagi")).unwrap(); // klasöre bağ: girilmez, sayılmaz
    std::os::unix::fs::symlink("a", k.join("dosya_bagi")).unwrap(); // dosyaya bağ: lstat = bağın uzunluğu (1)
    std::os::unix::fs::symlink("yok", k.join("kirik")).unwrap(); // kırık bağ: lstat = 3
    assert_eq!(boyut(&[k.clone()]), 5 + 3 + 1 + 3);
    assert_eq!(boyut(&[tmp.join("yok")]), 0);
}
