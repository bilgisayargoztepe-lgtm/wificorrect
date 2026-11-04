//! rapor.rs testleri (Python tests/test_rapor.py'nin karşılığı; `listeden_cikan_arsivin_eski_bicim_hamlari_silinir` spec
//! bilinçli fark 6 yüzünden, `kur_sh_ve_merkez_birimi` Python kurulum dosyalarını denetlediği için (Task 14) taşınmadı).
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeZone};
use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::gecici_dizin;

const BASLIK: [&str; 19] = [
    "zaman", "olay", "telefon", "ad", "soyad", "mac", "ic_ip", "protokol", "ic_port", "hedef_ip", "hedef_port", "nat_ip", "nat_port", "alan_adi", "gonderilen_bayt",
    "alinan_bayt", "sure_sn", "oturum_id", "ek",
];
const SIR: [u8; 32] = [b's'; 32];
const TEL1: &str = "905551112233";
const TEL2: &str = "905559998877";
const TEL3: &str = "905554443322";
const TC: &str = "TC:10000000146";
const PAS: &str = "PASAPORT:DE:X1234567";
const MAC_IZINLI: &str = "aa:bb:cc:dd:ee:01";
const MAC_MISAFIR: &str = "aa:bb:cc:dd:ee:02";

fn sha(b: &[u8]) -> String {
    sha256_hex(b)
}

fn satir(zaman: &str, olay: &str, alanlar: &[(&str, &str)]) -> String {
    let mut r: BTreeMap<&str, &str> = BASLIK.iter().map(|k| (*k, "")).collect();
    for (k, v) in alanlar {
        r.insert(k, v);
    }
    r.insert("zaman", zaman);
    r.insert("olay", olay);
    BASLIK.iter().map(|k| r[k]).collect::<Vec<_>>().join(";")
}

fn gz_yaz(yol: &Path, satirlar: &[String]) {
    fs::create_dir_all(yol.parent().unwrap()).unwrap();
    let mut m = format!("\u{feff}{}\r\n", BASLIK.join(";"));
    for s in satirlar {
        m.push_str(s);
        m.push_str("\r\n");
    }
    let mut e = flate2::write::GzEncoder::new(File::create(yol).unwrap(), flate2::Compression::default());
    e.write_all(m.as_bytes()).unwrap();
    e.finish().unwrap();
}

fn csv_gz_adlari(kok: &Path, d: &Path, out: &mut Vec<String>) {
    for x in fs::read_dir(d).unwrap().flatten() {
        let yol = x.path();
        if yol.is_dir() {
            csv_gz_adlari(kok, &yol, out);
        } else if x.file_name().to_string_lossy().ends_with(".csv.gz") {
            out.push(yol.strip_prefix(kok).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Cihazın biçimi: başlık yorumu + '<sha>  <göreli ad>' (sıralı, kullanicilar/ dahil).
fn manifest_yaz(d: &Path, gun: &str) {
    let mut adlar = Vec::new();
    csv_gz_adlari(d, d, &mut adlar);
    adlar.sort();
    let mut metin = format!("# gun={gun} olusturma={gun}T00:15:00+03:00 onceki_zincir=x\n");
    for a in adlar {
        metin.push_str(&format!("{}  {a}\n", sha(&fs::read(d.join(&a)).unwrap())));
    }
    fs::write(d.join("MANIFEST.sha256"), metin).unwrap();
}

#[derive(Default)]
struct G {
    trafik: Vec<String>,
    oturum: Vec<String>,
    denetim: Vec<String>,
    kisi: Vec<String>,
    manifest_yok: bool,
}

fn gun_yaz(kok: &Path, gun: &str, g: G) -> PathBuf {
    let d = kok.join("gunluk").join(gun);
    fs::create_dir_all(&d).unwrap();
    for (ad, satirlar) in [("trafik", &g.trafik), ("oturum", &g.oturum), ("denetim", &g.denetim)] {
        if !satirlar.is_empty() {
            gz_yaz(&d.join(format!("{ad}.csv.gz")), satirlar);
        }
    }
    for tel in &g.kisi {
        match tel.split_once(':') {
            // Altın kayıt Windows'ta alındı: "TC:123.csv.gz" yazımı NTFS'te ':' öncesi adlı boş dosya bıraktı (kayıtta
            // `kullanicilar/TC`, boş metin). Linux'ta Python gerçek adlı gz yazardı; kayıt bu yüzden böyle karşılaştırılır.
            Some((onek, _)) => {
                fs::create_dir_all(d.join("kullanicilar")).unwrap();
                fs::write(d.join("kullanicilar").join(onek), b"").unwrap();
            }
            None => gz_yaz(&d.join("kullanicilar").join(format!("{tel}.csv.gz")), &[]),
        }
    }
    if !g.manifest_yok {
        manifest_yaz(&d, gun);
    }
    d
}

fn man_sha(kok: &Path, gun: &str) -> String {
    sha(&fs::read(kok.join("gunluk").join(gun).join("MANIFEST.sha256")).unwrap())
}

fn zincir_satirlari(gunler_sha: &[(&str, String)]) -> Vec<String> {
    let mut prev = SIFIR.to_string();
    let mut out = Vec::new();
    for (gun, m) in gunler_sha {
        let z = sha(format!("{prev}{m}").as_bytes());
        out.push(format!("{gun};{m};{z}"));
        prev = z;
    }
    out
}

/// Cihazdaki append_chain gibi zincir.txt üretir.
fn muhurle(kok: &Path, gunler: &[&str]) {
    let l: Vec<(&str, String)> = gunler.iter().map(|g| (*g, man_sha(kok, g))).collect();
    let metin: String = zincir_satirlari(&l).iter().map(|s| format!("{s}\n")).collect();
    fs::write(kok.join("zincir.txt"), metin).unwrap();
}

fn z(gun: &str, saat: &str) -> String {
    format!("{gun}T{saat}+03:00")
}

fn basit_gun(kok: &Path, gun: &str, tel: &str) -> PathBuf {
    gun_yaz(
        kok,
        gun,
        G { trafik: vec![satir(&z(gun, "10:00:00"), "BAGLANTI_BASLA", &[("telefon", tel), ("mac", MAC_MISAFIR), ("ic_ip", "10.50.0.23")])], kisi: vec![tel.into()], ..Default::default() },
    )
}

fn yaz_zincir(yol: &Path, satirlar: &[String], son: &str) {
    fs::write(yol, format!("{}{son}", satirlar.join("\n"))).unwrap();
}

fn s(x: &str) -> String {
    x.to_string()
}

// --- zincir ---
#[test]
fn zincir_dogru() {
    let _k = Kayit::bas("test_rapor", "test_zincir_dogru");
    let kok = gecici_dizin();
    let gunler = ["2026-10-01", "2026-10-02", "2026-10-03"];
    for g in gunler {
        basit_gun(&kok, g, TEL1);
    }
    muhurle(&kok, &gunler);
    let sonuc = zincir_oku(&kok, None);
    let beklenen: Zincir = gunler.iter().map(|g| (s(g), (man_sha(&kok, g), true))).collect();
    assert_eq!(sonuc, beklenen);
}

#[test]
fn zincir_ortadaki_manifest_sha_degisti() {
    let _k = Kayit::bas("test_rapor", "test_zincir_ortadaki_manifest_sha_degisti");
    let kok = gecici_dizin();
    let mut sl = zincir_satirlari(&[("2026-10-01", "a".repeat(64)), ("2026-10-02", "b".repeat(64)), ("2026-10-03", "c".repeat(64))]);
    let p: Vec<String> = sl[1].split(';').map(s).collect();
    sl[1] = format!("{};{};{}", p[0], "d".repeat(64), p[2]);
    yaz_zincir(&kok.join("zincir.txt"), &sl, "\n");
    let sonuc = zincir_oku(&kok, None);
    assert_eq!(sonuc["2026-10-01"], ("a".repeat(64), true));
    assert_eq!(sonuc["2026-10-02"], ("d".repeat(64), false));
    assert_eq!(sonuc["2026-10-03"], ("c".repeat(64), true)); // cihazdaki verify gibi: prev = yazılı zincir değeri
}

#[test]
fn zincir_sha_degisti_sonrakiler_bozuk() {
    let _k = Kayit::bas("test_rapor", "test_zincir_sha_degisti_sonrakiler_bozuk");
    let kok = gecici_dizin();
    let mut sl = zincir_satirlari(&[("2026-10-01", "a".repeat(64)), ("2026-10-02", "b".repeat(64)), ("2026-10-03", "c".repeat(64))]);
    let p: Vec<String> = sl[1].split(';').map(s).collect();
    sl[1] = format!("{};{};{}", p[0], p[1], "e".repeat(64));
    yaz_zincir(&kok.join("zincir.txt"), &sl, "\n");
    let sonuc = zincir_oku(&kok, None);
    assert!(sonuc["2026-10-01"].1);
    assert!(!sonuc["2026-10-02"].1);
    assert!(!sonuc["2026-10-03"].1);
}

#[test]
fn zincir_yarim_satir_ve_cift_gun() {
    let _k = Kayit::bas("test_rapor", "test_zincir_yarim_satir_ve_cift_gun");
    let kok = gecici_dizin();
    let sl = zincir_satirlari(&[("2026-10-01", "a".repeat(64)), ("2026-10-02", "b".repeat(64))]);
    let yarim = sl[1][..40].to_string();
    yaz_zincir(&kok.join("zincir.txt"), &[sl[0].clone(), yarim], "");
    assert_eq!(zincir_oku(&kok, None), Zincir::from([(s("2026-10-01"), ("a".repeat(64), true))]));
    // aynı gün iki satır: son satır geçerli
    let s2 = zincir_satirlari(&[("2026-10-01", "a".repeat(64)), ("2026-10-01", "f".repeat(64))]);
    yaz_zincir(&kok.join("zincir.txt"), &s2, "\n");
    assert_eq!(zincir_oku(&kok, None), Zincir::from([(s("2026-10-01"), ("f".repeat(64), true))]));
    let s3 = vec![s2[0].clone(), format!("2026-10-01;{};{}", "f".repeat(64), "0".repeat(64))];
    yaz_zincir(&kok.join("zincir.txt"), &s3, "\n");
    assert_eq!(zincir_oku(&kok, None), Zincir::from([(s("2026-10-01"), ("f".repeat(64), false))]));
}

#[test]
fn zincir_eski_dosya_yeniden_kurulum() {
    let _k = Kayit::bas("test_rapor", "test_zincir_eski_dosya_yeniden_kurulum");
    let kok = gecici_dizin();
    fs::create_dir(kok.join("eski")).unwrap();
    yaz_zincir(&kok.join("eski").join("zincir.txt.2026-10-01"), &zincir_satirlari(&[("2026-09-29", "a".repeat(64)), ("2026-09-30", "b".repeat(64))]), "\n");
    yaz_zincir(&kok.join("zincir.txt"), &zincir_satirlari(&[("2026-10-02", "c".repeat(64))]), "\n"); // sıfırdan başlıyor
    let sonuc = zincir_oku(&kok, None);
    assert_eq!(
        sonuc,
        Zincir::from([(s("2026-09-29"), ("a".repeat(64), true)), (s("2026-09-30"), ("b".repeat(64), true)), (s("2026-10-02"), ("c".repeat(64), true))])
    );
}

#[test]
fn zincir_birlesim_ok_kazanir() {
    let _k = Kayit::bas("test_rapor", "test_zincir_birlesim_ok_kazanir");
    let kok = gecici_dizin();
    fs::create_dir(kok.join("eski")).unwrap();
    yaz_zincir(&kok.join("eski").join("zincir.txt.1"), &[format!("2026-10-01;{};{}", "a".repeat(64), "1".repeat(64))], "\n"); // bozuk
    yaz_zincir(&kok.join("zincir.txt"), &zincir_satirlari(&[("2026-10-01", "a".repeat(64))]), "\n");
    assert_eq!(zincir_oku(&kok, None), Zincir::from([(s("2026-10-01"), ("a".repeat(64), true))]));
    yaz_zincir(&kok.join("zincir.txt"), &[format!("2026-10-01;{};{}", "b".repeat(64), "1".repeat(64))], "\n");
    yaz_zincir(&kok.join("eski").join("zincir.txt.1"), &zincir_satirlari(&[("2026-10-01", "a".repeat(64))]), "\n");
    assert_eq!(zincir_oku(&kok, None), Zincir::from([(s("2026-10-01"), ("a".repeat(64), true))]));
}

#[test]
fn zincir_yok_ve_buyuk_dosya() {
    let _k = Kayit::bas("test_rapor", "test_zincir_yok_ve_buyuk_dosya");
    let kok = gecici_dizin();
    assert!(zincir_oku(&kok, None).is_empty());
    yaz_zincir(&kok.join("zincir.txt"), &zincir_satirlari(&[("2026-10-01", "a".repeat(64))]), "\n");
    assert!(zincir_oku_sinirli(&kok, None, 10).is_empty());
}

#[test]
fn zincir_sembolik_bag_okunmaz() {
    let _k = Kayit::bas("test_rapor", "test_zincir_sembolik_bag_okunmaz");
    let tmp = gecici_dizin();
    let (kok, baska) = (tmp.join("a"), tmp.join("b"));
    fs::create_dir(&kok).unwrap();
    fs::create_dir(&baska).unwrap();
    yaz_zincir(&baska.join("zincir.txt"), &zincir_satirlari(&[("2026-10-01", "a".repeat(64))]), "\n");
    symlink(baska.join("zincir.txt"), kok.join("zincir.txt")).unwrap();
    assert!(zincir_oku(&kok, None).is_empty());
}

// --- gun_dogrula ---
#[test]
fn gun_dogrula_dogru_ve_bozuk_bayt() {
    let _k = Kayit::bas("test_rapor", "test_gun_dogrula_dogru_ve_bozuk_bayt");
    let kok = gecici_dizin();
    let d = basit_gun(&kok, "2026-10-01", TEL1);
    assert_eq!(gun_dogrula(&d).unwrap(), (Some(man_sha(&kok, "2026-10-01")), true));
    let yol = d.join("kullanicilar").join(format!("{TEL1}.csv.gz"));
    let mut b = fs::read(&yol).unwrap();
    *b.last_mut().unwrap() ^= 1;
    fs::write(&yol, b).unwrap();
    let (m, ok) = gun_dogrula(&d).unwrap();
    assert!(m == Some(man_sha(&kok, "2026-10-01")) && !ok);
}

#[test]
fn gun_dogrula_yol_hilesi() {
    let _k = Kayit::bas("test_rapor", "test_gun_dogrula_yol_hilesi");
    let kok = gecici_dizin();
    let d = basit_gun(&kok, "2026-10-01", TEL1);
    fs::write(kok.join("gunluk").join("x"), b"x").unwrap();
    for ad in ["../x", "/etc/passwd", "..\\x", "kullanicilar/../../x"] {
        fs::write(d.join("MANIFEST.sha256"), format!("{}  {ad}\n", sha(b"x"))).unwrap();
        assert!(!gun_dogrula(&d).unwrap().1, "{ad}");
    }
    fs::write(d.join("MANIFEST.sha256"), "bozuk satir\n").unwrap();
    assert!(!gun_dogrula(&d).unwrap().1);
    fs::remove_file(d.join("MANIFEST.sha256")).unwrap();
    assert_eq!(gun_dogrula(&d).unwrap(), (None, false));
}

#[test]
fn gun_dogrula_sembolik_bag() {
    let _k = Kayit::bas("test_rapor", "test_gun_dogrula_sembolik_bag");
    let kok = gecici_dizin();
    let d = basit_gun(&kok, "2026-10-01", TEL1);
    let hedef = kok.join("disari.csv.gz");
    fs::write(&hedef, b"x").unwrap();
    symlink(&hedef, d.join("bag.csv.gz")).unwrap();
    let mut f = fs::OpenOptions::new().append(true).open(d.join("MANIFEST.sha256")).unwrap();
    write!(f, "{}  bag.csv.gz\n", sha(b"x")).unwrap();
    assert!(!gun_dogrula(&d).unwrap().1);
}

// --- gun_ham ---
fn zengin_gun(kok: &Path, gun: &str) -> PathBuf {
    let t = |saat: &str| z(gun, saat);
    let trafik = vec![
        satir(&t("09:10:00"), "BAGLANTI_BASLA", &[("telefon", TEL1), ("ad", "Ali"), ("soyad", "Veli"), ("mac", MAC_MISAFIR), ("ic_ip", "10.50.0.23"), ("hedef_ip", "142.250.187.110"), ("oturum_id", "oturum-abc")]),
        satir(&t("09:40:00"), "BAGLANTI_BITIS", &[("telefon", TEL1), ("mac", MAC_MISAFIR), ("ic_ip", "10.50.0.23"), ("gonderilen_bayt", "100"), ("alinan_bayt", "1000")]),
        satir(&t("21:05:00"), "BAGLANTI_BASLA", &[("telefon", TC), ("ad", "Ayşe"), ("mac", "aa:bb:cc:dd:ee:03"), ("ic_ip", "10.50.0.24")]),
        satir(&t("21:06:00"), "BAGLANTI_BITIS", &[("telefon", PAS), ("mac", "aa:bb:cc:dd:ee:04"), ("ic_ip", "10.50.0.25"), ("gonderilen_bayt", "5"), ("alinan_bayt", "x")]),
        satir(&t("11:00:00"), "BAGLANTI_BITIS", &[("telefon", ""), ("mac", MAC_IZINLI), ("ic_ip", "10.50.0.2"), ("ek", "ct=1 izinli=AP"), ("gonderilen_bayt", "7"), ("alinan_bayt", "3")]),
        satir("bozuk-zaman", "BAGLANTI_BITIS", &[("telefon", TEL3), ("mac", MAC_MISAFIR), ("gonderilen_bayt", "999999")]),
        satir(&t("25:00:00"), "BAGLANTI_BASLA", &[("telefon", TEL3)]),
    ];
    let oturum = vec![
        satir(&t("09:09:00"), "OTURUM_BASLA", &[("telefon", TEL1), ("ek", "bitis=x sozlesme=1 riza=1")]),
        satir(&t("21:04:00"), "OTURUM_BASLA", &[("telefon", TC), ("ek", "bitis=x sozlesme=1 riza=1 giris=misafir_kodu oda=101")]),
        satir(&t("21:05:00"), "OTURUM_BASLA", &[("telefon", PAS), ("ek", "giris=misafir_kodu")]),
        satir(&t("22:00:00"), "OTURUM_UZATILDI", &[("telefon", TC), ("ek", "giris=misafir_kodu")]),
        satir(&t("23:00:00"), "OTURUM_BITIS", &[("telefon", TEL1)]),
    ];
    let denetim = vec![
        satir(&t("09:08:00"), "OTP_GONDERILDI", &[("telefon", TEL1)]),
        satir(&t("09:08:30"), "OTP_GONDERILDI", &[("telefon", TEL2)]),
        satir(&t("09:08:40"), "OTP_ISTEK", &[("telefon", TEL2)]),
        satir(&t("03:10:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.4.8 onceki=0.4.7")]),
        satir(&t("03:20:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.4.9 onceki=0.4.8")]),
    ];
    // dns.csv Python'da da yazılır (alan adı verisi okunmaz): dosya kümesi aynı olsun
    let d = gun_yaz(kok, gun, G { trafik, oturum, denetim, kisi: vec![TEL1.into(), TC.into()], manifest_yok: true });
    let dns = vec![
        satir(&t("09:11:00"), "DNS", &[("telefon", TEL1), ("alan_adi", "www.instagram.com")]),
        satir(&t("09:12:00"), "DNS", &[("telefon", TEL1), ("alan_adi", "i.instagram.com")]),
        satir(&t("21:07:00"), "DNS", &[("telefon", TC), ("alan_adi", "instagram.com.")]),
        satir(&t("21:08:00"), "DNS", &[("telefon", ""), ("alan_adi", "facebook.com")]),
        satir(&t("21:09:00"), "DNS", &[("telefon", TEL1), ("alan_adi", "10.0.0.1")]),
        satir(&t("21:10:00"), "DNSX", &[("telefon", TEL1), ("alan_adi", "twitter.com")]),
    ];
    gz_yaz(&d.join("dns.csv.gz"), &dns);
    manifest_yaz(&d, gun);
    d
}

fn dizi(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

#[test]
fn gun_ham_sayilar() {
    let _k = Kayit::bas("test_rapor", "test_gun_ham_sayilar");
    let kok = gecici_dizin();
    let d = zengin_gun(&kok, "2026-10-05");
    let h = gun_ham(&d, &SIR).unwrap();
    assert_eq!(h["bicim"], BICIM);
    assert!(h["manifest_sha"] == man_sha(&kok, "2026-10-05").as_str() && h["dosya_ok"] == true);
    let kim = |k: &str| takma(&SIR, k);
    let mut tum = vec![kim(TEL1), kim(TC), kim(PAS)];
    tum.sort();
    assert_eq!(dizi(&h["misafir"]), tum);
    let mut iki = vec![kim(TC), kim(PAS)];
    iki.sort();
    assert_eq!(h["saat"], json!({"9": [kim(TEL1)], "21": iki}));
    assert_eq!(h["giris"], json!({"sms": 1, "kod": 2}));
    assert_eq!(h["izinli"], json!([takma(&SIR, &format!("mac:{MAC_IZINLI}"))]));
    assert!(h["gonderilen"] == 100 + 5 + 7 && h["alinan"] == 1000 + 3);
    assert_eq!(h["sms"], 2);
    assert!(h.get("alan").is_none() && !serde_json::to_string(&h).unwrap().contains("instagram")); // alan adı verisi tutulmaz
    assert_eq!(h["surum"], "0.4.9");
    assert_eq!(h["imza"].as_str().unwrap().len(), 64);
}

#[test]
fn gun_ham_surum_geri_alindi_ve_yok() {
    let _k = Kayit::bas("test_rapor", "test_gun_ham_surum_geri_alindi_ve_yok");
    let kok = gecici_dizin();
    let g = "2026-10-06";
    let d = gun_yaz(
        &kok,
        g,
        G {
            denetim: vec![
                satir(&z(g, "03:10:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.4.9 onceki=0.4.8")]),
                satir(&z(g, "03:30:00"), "GUNCELLEME_GERI_ALINDI", &[("ek", "surum=0.4.9 onceki=0.4.8 neden=x")]),
            ],
            ..Default::default()
        },
    );
    assert_eq!(gun_ham(&d, &SIR).unwrap()["surum"], "0.4.8");
    let g2 = "2026-10-07";
    let d2 = gun_yaz(&kok, g2, G { denetim: vec![satir(&z(g2, "10:00:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=<b>")])], ..Default::default() });
    assert_eq!(gun_ham(&d2, &SIR).unwrap()["surum"], Value::Null);
    let d3 = gun_yaz(&kok, "2026-10-08", G::default()); // hiç dosya yok, yalnızca MANIFEST
    let h = gun_ham(&d3, &SIR).unwrap();
    assert!(h["surum"].is_null() && h["misafir"] == json!([]) && h["saat"] == json!({}) && h["dosya_ok"] == true);
}

#[test]
fn gun_ham_kisisel_veri_yok() {
    let _k = Kayit::bas("test_rapor", "test_gun_ham_kisisel_veri_yok");
    let kok = gecici_dizin();
    let d = zengin_gun(&kok, "2026-10-05");
    let metin = serde_json::to_string(&gun_ham(&d, &SIR).unwrap()).unwrap();
    for gizli in [TEL1, TEL2, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", MAC_MISAFIR, MAC_IZINLI, "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "AP"] {
        assert!(!metin.contains(gizli), "{gizli}");
    }
}

// --- ham_guncelle ---
fn oku(yol: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(yol).unwrap()).unwrap()
}

fn liste(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(d).unwrap().map(|x| x.unwrap().file_name().into_string().unwrap()).collect();
    v.sort();
    v
}

fn bos() -> Vec<String> {
    Vec::new()
}

#[test]
fn ham_guncelle_artimli_ve_manifest_degisimi() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_artimli_ve_manifest_degisimi");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    zengin_gun(&kok, "2026-10-05");
    basit_gun(&kok, "2026-10-06", TEL2);
    gun_yaz(&kok, "2026-10-07", G { trafik: vec![satir(&z("2026-10-07", "10:00:00"), "BAGLANTI_BASLA", &[("telefon", TEL1)])], manifest_yok: true, ..Default::default() });
    assert_eq!(ham_guncelle("1537344", &[kok.clone()], &ham, &SIR).unwrap(), bos());
    assert_eq!(liste(&ham.join("1537344")), vec!["2026-10-05.json", "2026-10-06.json"]);
    let ilk = oku(&ham.join("1537344").join("2026-10-05.json"));
    assert!(!ilk["misafir"].as_array().unwrap().is_empty() && ilk["dosya_ok"] == true);

    // imza aynıyken dosyalar yeniden okunmaz/hash'lenmez: içerik aynı boyutta bozulur, mtime geri konur
    let yol = kok.join("gunluk/2026-10-05/trafik.csv.gz");
    let md = fs::metadata(&yol).unwrap();
    let mut f = fs::OpenOptions::new().write(true).open(&yol).unwrap();
    f.write_all(&vec![0u8; md.len() as usize]).unwrap();
    f.set_times(fs::FileTimes::new().set_accessed(md.accessed().unwrap()).set_modified(md.modified().unwrap())).unwrap();
    drop(f);
    GUN_HAM_CAGRI.with(|c| c.set(0));
    assert_eq!(ham_guncelle("1537344", &[kok.clone()], &ham, &SIR).unwrap(), bos());
    assert_eq!(GUN_HAM_CAGRI.with(|c| c.get()), 0);
    assert_eq!(oku(&ham.join("1537344").join("2026-10-05.json")), ilk);

    // MANIFEST değişince (yeniden gönderim) yeniden hesaplanır
    let d6 = kok.join("gunluk/2026-10-06");
    gz_yaz(
        &d6.join("trafik.csv.gz"),
        &[satir(&z("2026-10-06", "10:00:00"), "BAGLANTI_BASLA", &[("telefon", TEL1)]), satir(&z("2026-10-06", "10:00:00"), "BAGLANTI_BASLA", &[("telefon", TEL3)])],
    );
    manifest_yaz(&d6, "2026-10-06");
    assert_eq!(ham_guncelle("1537344", &[kok.clone()], &ham, &SIR).unwrap(), bos());
    assert_eq!(oku(&ham.join("1537344").join("2026-10-06.json"))["misafir"].as_array().unwrap().len(), 2);
}

#[test]
fn ham_guncelle_dosya_degisince_imza_degisir() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_dosya_degisince_imza_degisir");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    let d = basit_gun(&kok, "2026-10-05", TEL1);
    ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap();
    assert_eq!(oku(&ham.join("a/2026-10-05.json"))["dosya_ok"], true);
    let mut f = fs::OpenOptions::new().append(true).open(d.join("kullanicilar").join(format!("{TEL1}.csv.gz"))).unwrap();
    f.write_all(b"x").unwrap(); // MANIFEST aynı, dosya değişti → önbellek "tam"ı eskitmez
    drop(f);
    ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap();
    assert_eq!(oku(&ham.join("a/2026-10-05.json"))["dosya_ok"], false);
}

#[test]
fn ham_guncelle_bozuk_gz_ve_silinen_gun() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_bozuk_gz_ve_silinen_gun");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    basit_gun(&kok, "2026-10-05", TEL1);
    let d = basit_gun(&kok, "2026-10-06", TEL1);
    fs::write(d.join("trafik.csv.gz"), b"\x1f\x8b\x08\x00bozuk-veri").unwrap();
    manifest_yaz(&d, "2026-10-06");
    basit_gun(&kok, "2026-10-07", TEL1);
    let hatalar = ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap();
    assert!(hatalar.len() == 1 && hatalar[0].starts_with("2026-10-06: "), "{hatalar:?}");
    assert!(!hatalar[0].contains(TEL1) && !hatalar[0].contains("kullanicilar") && !hatalar[0].contains("trafik"));
    assert_eq!(liste(&ham.join("a")), vec!["2026-10-05.json", "2026-10-07.json"]);
    assert_eq!(ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap().len(), 1); // sonraki çalıştırmada yeniden denenir
    fs::remove_dir_all(kok.join("gunluk/2026-10-05")).unwrap();
    ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap();
    assert_eq!(liste(&ham.join("a")), vec!["2026-10-07.json"]);
}

#[test]
fn ham_guncelle_bozuk_onbellek_yeniden_yazilir() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_bozuk_onbellek_yeniden_yazilir");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    basit_gun(&kok, "2026-10-05", TEL1);
    ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap();
    let yol = ham.join("a/2026-10-05.json");
    fs::write(&yol, "{bozuk").unwrap();
    assert_eq!(ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap(), bos());
    assert_eq!(oku(&yol)["dosya_ok"], true);
}

#[test]
fn ham_guncelle_tamam_olmayan_gun_yazilmaz() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_tamam_olmayan_gun_yazilmaz");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    let d = basit_gun(&kok, "2026-10-05", TEL1);
    fs::remove_file(d.join("kullanicilar").join(format!("{TEL1}.csv.gz"))).unwrap(); // MANIFEST'teki dosya henüz gelmemiş
    assert_eq!(ham_guncelle("a", &[kok.clone()], &ham, &SIR).unwrap(), bos());
    assert!(liste(&ham.join("a")).is_empty());
}

#[test]
fn ham_guncelle_gecersiz_arsiv_adi() {
    let _k = Kayit::bas("test_rapor", "test_ham_guncelle_gecersiz_arsiv_adi");
    let tmp = gecici_dizin();
    let (kok, ham) = (tmp.join("arsiv"), tmp.join("ham"));
    basit_gun(&kok, "2026-10-05", TEL1);
    for ad in ["../x", "", ".", "a/b"] {
        assert!(ham_guncelle(ad, &[kok.clone()], &ham, &SIR).is_err(), "{ad}");
    }
}

// ======================================================================================================================
// Görev 2: gruplar, gün durumu, dönem raporları
// ======================================================================================================================
fn ts(gun: &str, saat: u32) -> f64 {
    let d = NaiveDate::parse_from_str(gun, "%Y-%m-%d").unwrap();
    ortak::TZ.from_local_datetime(&d.and_hms_opt(saat, 0, 0).unwrap()).single().unwrap().timestamp() as f64
}

fn tarih_(y: i32, a: u32, g: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, a, g).unwrap()
}

fn ars(kaynaklar: &[&Path]) -> Vec<PathBuf> {
    kaynaklar.iter().map(|p| p.to_path_buf()).collect()
}

fn arsivler(l: Vec<(&str, Vec<PathBuf>)>) -> Arsivler {
    l.into_iter().map(|(a, k)| (a.to_string(), k)).collect()
}

fn misafir_gun_g(kok: &Path, gun: &str, tels: &[&str], saat: &str, oturum: Vec<String>, denetim: Vec<String>) -> PathBuf {
    let trafik = tels.iter().map(|t| satir(&z(gun, saat), "BAGLANTI_BASLA", &[("telefon", t), ("mac", MAC_MISAFIR), ("ic_ip", "10.50.0.23")])).collect();
    gun_yaz(kok, gun, G { trafik, oturum, denetim, kisi: tels.iter().filter(|t| t.bytes().all(|b| b.is_ascii_digit())).map(|t| t.to_string()).collect(), manifest_yok: false })
}

fn misafir_gun(kok: &Path, gun: &str, tels: &[&str]) -> PathBuf {
    misafir_gun_g(kok, gun, tels, "10:00:00", vec![], vec![])
}

fn calis_saat(tmp: &Path, a: &Arsivler, gun: &str, grup_icerik: Option<Value>, saat: u32) -> Vec<String> {
    let gd = tmp.join("gruplar.json");
    if let Some(v) = grup_icerik {
        fs::write(&gd, serde_json::to_string(&v).unwrap()).unwrap();
    }
    calistir(a, &gd, &tmp.join("ham"), &tmp.join("rapor"), None, Some(ts(gun, saat)), Some(&SIR)).unwrap()
}

fn calis(tmp: &Path, a: &Arsivler, gun: &str, grup_icerik: Option<Value>) -> Vec<String> {
    calis_saat(tmp, a, gun, grup_icerik, 12)
}

fn rp(tmp: &Path, numara: &str, ad: &str) -> Value {
    oku(&tmp.join("rapor").join(numara).join(format!("{ad}.json")))
}

#[test]
fn gruplar_testi() {
    let _k = Kayit::bas("test_rapor", "test_gruplar");
    let tmp = gecici_dizin();
    let a = arsivler(vec![
        ("1234567", ars(&[Path::new("k1"), Path::new("k1b")])),
        ("7654321", ars(&[Path::new("k2")])),
        ("bocafe", ars(&[Path::new("k3")])),
        ("eski-b", ars(&[Path::new("k4")])),
        ("eski-c", ars(&[Path::new("k5")])),
        ("eski-d", ars(&[Path::new("k6")])),
        ("0123456", ars(&[Path::new("k7")])),
    ]);
    let gd = tmp.join("g.json");
    let p = |ad: &str, k: &str| (s(ad), PathBuf::from(k));
    let tek: BTreeMap<String, Vec<(String, PathBuf)>> =
        BTreeMap::from([(s("1234567"), vec![p("1234567", "k1"), p("1234567", "k1b")]), (s("7654321"), vec![p("7654321", "k2")])]);
    assert_eq!(gruplar(&a, &gd), tek); // dosya yok: her numara tek başına; bağsız eski arşiv rapor almaz
    fs::write(
        &gd,
        serde_json::to_string(&json!({
            "1234567": ["eski-c", "bocafe", "7654321", "yok", "../x", 5, "bocafe"],
            "7654321": ["bocafe", "eski-b"], "abc": ["eski-d"], "3333333": ["eski-c"],
            "4444444": ["eski-d", "0123456"], "5555555": "bocafe"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        gruplar(&a, &gd),
        BTreeMap::from([
            (s("1234567"), vec![p("1234567", "k1"), p("1234567", "k1b"), p("bocafe", "k3"), p("eski-c", "k5")]),
            (s("7654321"), vec![p("7654321", "k2"), p("eski-b", "k4")]), // bocafe ilk müşteride; başka numara çekilemez
            (s("4444444"), vec![p("0123456", "k7"), p("eski-d", "k6")]), // numara arşivi henüz yok ama eski arşivi bağlı
        ])
    );
    let buyuk = serde_json::to_string(&json!({"1234567": ["bocafe"], "x": "y".repeat(70000)})).unwrap();
    for bozuk in ["{bozuk".to_string(), "[1, 2]".to_string(), buyuk] {
        fs::write(&gd, &bozuk).unwrap();
        assert_eq!(gruplar(&a, &gd), tek, "{}", &bozuk[..10]);
    }
    fs::remove_file(&gd).unwrap();
    let baska = tmp.join("baska.json");
    fs::write(&baska, serde_json::to_string(&json!({"1234567": ["bocafe"]})).unwrap()).unwrap();
    symlink(&baska, &gd).unwrap();
    assert_eq!(gruplar(&a, &gd), tek);
}

#[test]
fn ayni_gun_iki_arsivde_bir_kez_sayilir() {
    let _k = Kayit::bas("test_rapor", "test_ayni_gun_iki_arsivde_bir_kez_sayilir");
    let tmp = gecici_dizin();
    let (k1, k1b, k2) = (tmp.join("yeni"), tmp.join("yeni-eski-klasor"), tmp.join("bocafe"));
    let g = "2026-10-05";
    let bas = satir(&z(g, "10:00:00"), "OTURUM_BASLA", &[("telefon", TEL1), ("ek", "sozlesme=1 riza=1")]);
    misafir_gun_g(&k1, g, &[TEL1, TEL2], "10:00:00", vec![bas.clone()], vec![]);
    misafir_gun_g(&k2, g, &[TEL1, TEL2, TEL3], "10:00:00", vec![bas.clone(), bas.clone(), bas.clone()], vec![]);
    misafir_gun(&k2, "2026-10-04", &[TC]);
    misafir_gun(&k1b, "2026-10-06", &[PAS]); // aynı arşivin ikinci kaynağı (eski klasör): tek ham_guncelle çağrısı
    muhurle(&k1, &[g]);
    muhurle(&k2, &["2026-10-04", g]);
    muhurle(&k1b, &["2026-10-06"]);
    let a = arsivler(vec![("1234567", ars(&[&k1, &k1b])), ("bocafe", ars(&[&k2]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-08", None), bos());
    let r = rp(&tmp, "1234567", "hafta-2026-W41");
    assert!(r["misafir"]["farkli"] == 3 && r["misafir"]["giris"] == 1); // bağsız: 10-05 (2) + 10-06 (PAS)
    assert_eq!(json!([r["gunler"][0]["farkli"], r["gunler"][1]["farkli"], r["gunler"][2]["farkli"]]), json!([2, 1, null]));
    assert!(r["gunler"][0]["durum"] == "tam" && r["gunler"][1]["durum"] == "tam");
    assert!(!tmp.join("rapor/bocafe").exists());
    let eski_imza = r["imza"].clone();

    assert_eq!(calis(&tmp, &a, "2026-10-08", Some(json!({"1234567": ["bocafe"]}))), bos());
    let r = rp(&tmp, "1234567", "hafta-2026-W41");
    assert_ne!(r["imza"], eski_imza); // grup değişti → yeniden üretildi
    assert!(r["misafir"]["farkli"] == 3 && r["misafir"]["giris"] == 1); // 10-05 bir kez: yeni arşivinki (iki kat yok)
    assert_eq!(r["gunler"][0]["farkli"], 2);
    let w40 = rp(&tmp, "1234567", "hafta-2026-W40");
    let son = w40["gunler"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(son, json!({"gun": "2026-10-04", "durum": "tam", "neden": "", "farkli": 1}));

    fs::remove_file(k1.join("gunluk").join(g).join("kullanicilar").join(format!("{TEL1}.csv.gz"))).unwrap(); // yeni arşivde 10-05 eksik
    assert_eq!(calis(&tmp, &a, "2026-10-08", Some(json!({"1234567": ["bocafe"]}))), bos());
    let r = rp(&tmp, "1234567", "hafta-2026-W41");
    assert!(r["gunler"][0]["farkli"] == 3 && r["gunler"][0]["durum"] == "tam"); // ilk TAMAM kaynak kazanır
    assert!(r["misafir"]["farkli"] == 4 && r["misafir"]["giris"] == 3);
}

fn durum_haritasi(r: &Value) -> BTreeMap<String, (String, String)> {
    r["gunler"].as_array().unwrap().iter().map(|x| (x["gun"].as_str().unwrap().to_string(), (x["durum"].as_str().unwrap().to_string(), x["neden"].as_str().unwrap().to_string()))).collect()
}

fn dn(g: &str, d: &str, n: &str) -> (String, (String, String)) {
    (s(g), (s(d), s(n)))
}

#[test]
fn gun_durumlari() {
    let _k = Kayit::bas("test_rapor", "test_gun_durumlari");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    for g in ["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-02", "2026-10-03", "2026-10-04", "2026-10-05", "2026-10-07", "2026-10-08", "2026-10-10"] {
        misafir_gun(&k, g, &[TEL1]);
    }
    let d1 = gun_yaz(&k, "2026-10-01", G::default()); // trafik.csv.gz bozuk ama MANIFEST'te doğru hash'le → okunamadı
    fs::write(d1.join("trafik.csv.gz"), b"bozuk-gz").unwrap();
    manifest_yaz(&d1, "2026-10-01");
    fs::remove_file(k.join("gunluk/2026-10-07/kullanicilar").join(format!("{TEL1}.csv.gz"))).unwrap(); // eksik
    let mut f = fs::OpenOptions::new().append(true).open(k.join("gunluk/2026-10-08/kullanicilar").join(format!("{TEL1}.csv.gz"))).unwrap();
    f.write_all(b"x").unwrap(); // hash tutmuyor → dosya
    drop(f);
    let mut gunler: Vec<(&str, String)> =
        ["2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04", "2026-10-05", "2026-10-08", "2026-10-10"].iter().map(|g| (*g, man_sha(&k, g))).collect();
    gunler[1] = ("2026-09-30", "e".repeat(64)); // zincir satırı doğru ama MANIFEST başka → manifest
    let mut satirlar = zincir_satirlari(&gunler);
    let p: Vec<String> = satirlar[0].split(';').map(s).collect();
    satirlar[0] = format!("{};{};{}", p[0], "f".repeat(64), p[2]); // 09-29 satırı bozuk → zincir (sonrakiler etkilenmez)
    yaz_zincir(&k.join("zincir.txt"), &satirlar, "\n"); // 09-28 zincirde yok → zincir_yok (sahte "tam" yok)

    let hatalar = calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-10", None);
    assert!(hatalar.len() == 1 && hatalar[0].starts_with("1234567: 2026-10-01: "), "{hatalar:?}");
    let w40 = durum_haritasi(&rp(&tmp, "1234567", "hafta-2026-W40"));
    assert_eq!(
        w40,
        BTreeMap::from([
            dn("2026-09-28", "dogrulanamadi", "zincir_yok"),
            dn("2026-09-29", "dogrulanamadi", "zincir"),
            dn("2026-09-30", "dogrulanamadi", "manifest"),
            dn("2026-10-01", "dogrulanamadi", "okunamadi"),
            dn("2026-10-02", "tam", ""),
            dn("2026-10-03", "tam", ""),
            dn("2026-10-04", "tam", ""),
        ])
    );
    let w41 = rp(&tmp, "1234567", "hafta-2026-W41");
    let uc: BTreeMap<String, (String, String, Value)> =
        w41["gunler"].as_array().unwrap().iter().map(|x| (x["gun"].as_str().unwrap().to_string(), (x["durum"].as_str().unwrap().to_string(), x["neden"].as_str().unwrap().to_string(), x["farkli"].clone()))).collect();
    let u = |g: &str, d: &str, n: &str, f: Value| (s(g), (s(d), s(n), f));
    assert_eq!(
        uc,
        BTreeMap::from([
            u("2026-10-05", "tam", "", json!(1)),
            u("2026-10-06", "kayit_yok", "", Value::Null),
            u("2026-10-07", "eksik", "", Value::Null),
            u("2026-10-08", "dogrulanamadi", "dosya", json!(1)),
            u("2026-10-09", "bekleniyor", "", Value::Null),
            u("2026-10-10", "bugun", "", Value::Null),
            u("2026-10-11", "gelecek", "", Value::Null),
        ])
    );
    assert!(w41["kayit"] == json!({"gun": 5, "tam": 1}) && w41["misafir"]["farkli"] == 1); // bugün sayılmaz
    assert_eq!(rp(&tmp, "1234567", "hafta-2026-W40")["kayit"], json!({"gun": 7, "tam": 3}));
    let ay = rp(&tmp, "1234567", "ay-2026-10");
    assert!(ay["gunler"].as_array().unwrap().len() == 31 && ay["kayit"] == json!({"gun": 9, "tam": 4}));
    assert_eq!(ay["gunler"].as_array().unwrap().last().unwrap()["durum"], "gelecek");
}

#[test]
fn iso_hafta_ay_siniri_ve_saat() {
    let _k = Kayit::bas("test_rapor", "test_iso_hafta_ay_siniri_ve_saat");
    assert_eq!(hafta(tarih_(2026, 12, 28)), (s("2026-W53"), tarih_(2026, 12, 28), tarih_(2027, 1, 3)));
    assert!(hafta(tarih_(2027, 1, 3)).0 == "2026-W53" && hafta(tarih_(2027, 1, 4)).0 == "2027-W01");
    assert_eq!(ay(tarih_(2026, 2, 10)), (s("2026-02"), tarih_(2026, 2, 1), tarih_(2026, 2, 28)));
    assert_eq!(ay(tarih_(2026, 12, 31)), (s("2026-12"), tarih_(2026, 12, 1), tarih_(2026, 12, 31)));
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun_g(&k, "2026-09-30", &[TEL1], "23:30:00", vec![], vec![]);
    misafir_gun_g(&k, "2026-10-01", &[TEL2], "00:10:00", vec![], vec![]);
    muhurle(&k, &["2026-09-30", "2026-10-01"]);
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-20", None), bos());
    let (eylul, ekim) = (rp(&tmp, "1234567", "ay-2026-09"), rp(&tmp, "1234567", "ay-2026-10"));
    assert!(eylul["misafir"]["farkli"] == 1 && eylul["gunler"].as_array().unwrap().len() == 30 && eylul["gunler"].as_array().unwrap().last().unwrap()["farkli"] == 1);
    assert!(eylul["saat"][23] == 1 && eylul["saat"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).sum::<u64>() == 1);
    assert!(ekim["misafir"]["farkli"] == 1 && ekim["saat"][0] == 1 && ekim["gunler"][0]["farkli"] == 1);
    let w40 = rp(&tmp, "1234567", "hafta-2026-W40");
    assert!(w40["baslangic"] == "2026-09-28" && w40["bitis"] == "2026-10-04");
    assert!(w40["misafir"]["farkli"] == 2 && w40["hafta_gunu"] == json!([0, 0, 1, 1, 0, 0, 0])); // Pzt…Paz
    assert_eq!(w40["misafir"]["en_yogun_gun"], json!({"gun": "2026-09-30", "farkli": 1})); // eşitlikte erken gün
    assert_eq!(w40["misafir"]["gunluk_ortalama"], json!(1.0));
}

#[test]
fn yeni_donen() {
    let _k = Kayit::bas("test_rapor", "test_yeni_donen");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-09-10", &[TEL1]);
    misafir_gun(&k, "2026-10-05", &[TEL1, TEL2]);
    misafir_gun(&k, "2026-10-13", &[TEL2]);
    muhurle(&k, &["2026-09-10", "2026-10-05", "2026-10-13"]);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    let m = |ad: &str| rp(&tmp, "1234567", ad)["misafir"].clone();
    assert!(m("ay-2026-09")["yeni"] == 1 && m("ay-2026-09")["donen"] == 0);
    assert!(m("ay-2026-10")["farkli"] == 2 && m("ay-2026-10")["yeni"] == 1 && m("ay-2026-10")["donen"] == 1);
    assert!(m("hafta-2026-W41")["yeni"] == 1 && m("hafta-2026-W41")["donen"] == 1);
    assert!(m("hafta-2026-W42")["farkli"] == 1 && m("hafta-2026-W42")["yeni"] == 0 && m("hafta-2026-W42")["donen"] == 1);
    assert_eq!(m("ay-2026-10")["gunluk_ortalama"], json!(1.5)); // (2 + 1) / 2 gün
    misafir_gun(&k, "2026-09-15", &[TEL2]); // dönemden ÖNCEKİ gün geç geldi: "yeni" değişir, rapor yeniden yazılır
    muhurle(&k, &["2026-09-10", "2026-09-15", "2026-10-05", "2026-10-13"]);
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    assert!(m("ay-2026-10")["yeni"] == 0 && m("ay-2026-10")["donen"] == 2);
}

fn mtime(yol: &Path) -> std::time::SystemTime {
    fs::metadata(yol).unwrap().modified().unwrap()
}

#[test]
fn kesin_ve_imza() {
    let _k = Kayit::bas("test_rapor", "test_kesin_ve_imza");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-10-05", &[TEL1]);
    muhurle(&k, &["2026-10-05"]);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-08", None), bos());
    assert_eq!(rp(&tmp, "1234567", "hafta-2026-W41")["kesin"], false); // süren dönem
    assert_eq!(calis(&tmp, &a, "2026-10-14", None), bos());
    let yol = tmp.join("rapor/1234567/hafta-2026-W41.json");
    assert_eq!(oku(&yol)["kesin"], false); // bitti ama sonraki gün yok ve 7 gün geçmedi
    let eski = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    fs::OpenOptions::new().write(true).open(&yol).unwrap().set_times(fs::FileTimes::new().set_accessed(eski).set_modified(eski)).unwrap();
    assert_eq!(calis_saat(&tmp, &a, "2026-10-14", None, 13), bos());
    assert_eq!(mtime(&yol), eski); // değişiklik yok → yeniden yazılmaz
    misafir_gun(&k, "2026-10-07", &[TEL2]); // geç gelen gün
    muhurle(&k, &["2026-10-05", "2026-10-07"]);
    assert_eq!(calis(&tmp, &a, "2026-10-14", None), bos());
    let r = oku(&yol);
    assert!(mtime(&yol) != eski && r["misafir"]["farkli"] == 2 && r["kesin"] == false);
    fs::create_dir_all(k.join("gunluk/2026-10-13")).unwrap(); // bitişten sonraki gün geldi (henüz tamam değil)
    assert_eq!(calis(&tmp, &a, "2026-10-14", None), bos());
    assert_eq!(oku(&yol)["kesin"], true);
    assert_eq!(rp(&tmp, "1234567", "hafta-2026-W42")["kesin"], false);
    fs::remove_dir(k.join("gunluk/2026-10-13")).unwrap();
    assert!(calis(&tmp, &a, "2026-10-18", None) == bos() && oku(&yol)["kesin"] == true); // 7 gün geçti
    assert!(oku(&yol)["surum"].is_null() && oku(&yol)["surum_sonra_degisti"] == false);
}

#[test]
fn surum() {
    let _k = Kayit::bas("test_rapor", "test_surum");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun_g(&k, "2026-09-20", &[TEL1], "10:00:00", vec![], vec![satir(&z("2026-09-20", "10:00:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.4.8")])]);
    misafir_gun(&k, "2026-10-05", &[TEL1]);
    misafir_gun_g(&k, "2026-10-14", &[TEL1], "10:00:00", vec![], vec![satir(&z("2026-10-14", "10:00:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.4.9")])]);
    muhurle(&k, &["2026-09-20", "2026-10-05", "2026-10-14"]);
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-20", None), bos());
    let w41 = rp(&tmp, "1234567", "hafta-2026-W41");
    assert!(w41["surum"] == "0.4.8" && w41["surum_sonra_degisti"] == true); // dönem öncesi günler dahil
    assert_eq!(rp(&tmp, "1234567", "hafta-2026-W42")["surum"], "0.4.9");
    let ekim = rp(&tmp, "1234567", "ay-2026-10");
    assert!(ekim["surum"] == "0.4.9" && ekim["surum_sonra_degisti"] == false);
}

#[test]
fn pencere_ve_liste() {
    let _k = Kayit::bas("test_rapor", "test_pencere_ve_liste");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2025-09-01", &[TEL1]);
    misafir_gun(&k, "2026-10-05", &[TEL1, TEL2]);
    muhurle(&k, &["2025-09-01", "2026-10-05"]);
    let hedef = tmp.join("rapor/1234567");
    fs::create_dir_all(&hedef).unwrap();
    for ad in ["hafta-2020-W01.json", "ay-2020-01.json", "baska.txt"] {
        fs::write(hedef.join(ad), "{}").unwrap();
    }
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-20", None), bos());
    let adlar = liste(&hedef);
    assert!(!adlar.contains(&s("hafta-2020-W01.json")) && !adlar.contains(&s("ay-2020-01.json")) && adlar.contains(&s("baska.txt")));
    assert_eq!(adlar.iter().filter(|a| a.starts_with("hafta-")).count(), 26);
    assert_eq!(adlar.iter().filter(|a| a.starts_with("ay-")).count(), 14); // 2025-09 … 2026-10
    let l = oku(&hedef.join("liste.json"));
    let l = l.as_array().unwrap();
    let haftalar: Vec<&Value> = l.iter().filter(|x| x["tur"] == "hafta").collect();
    let aylar: Vec<&Value> = l.iter().filter(|x| x["tur"] == "ay").collect();
    let turler: Vec<&str> = l.iter().map(|x| x["tur"].as_str().unwrap()).collect();
    let mut bek = vec!["hafta"; 26];
    bek.extend(vec!["ay"; 14]);
    assert_eq!(turler, bek); // önce haftalar, sonra aylar
    assert!(haftalar[0]["donem"] == "2026-W43" && haftalar[0]["kesin"] == false);
    let donemler: Vec<&str> = haftalar.iter().map(|x| x["donem"].as_str().unwrap()).collect();
    let mut sirali = donemler.clone();
    sirali.sort();
    sirali.reverse();
    assert_eq!(donemler, sirali);
    assert!(aylar[0]["donem"] == "2026-10" && aylar.last().unwrap()["donem"] == "2025-09");
    let ekim = aylar.iter().find(|x| x["donem"] == "2026-10").unwrap();
    assert_eq!(**ekim, json!({"tur": "ay", "donem": "2026-10", "baslangic": "2026-10-01", "bitis": "2026-10-31", "kesin": false, "farkli": 2, "kayit_gun": 19, "kayit_tam": 1}));
}

#[test]
fn gunu_olmayan_donem_ve_bos_grup() {
    let _k = Kayit::bas("test_rapor", "test_gunu_olmayan_donem_ve_bos_grup");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-09-01", &[TEL1]);
    muhurle(&k, &["2026-09-01"]);
    fs::create_dir_all(tmp.join("bos/gunluk")).unwrap();
    let bos_k = tmp.join("bos");
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k])), ("2345678", ars(&[&bos_k]))]), "2026-10-20", None), bos());
    let r = rp(&tmp, "1234567", "hafta-2026-W42");
    assert!(r["misafir"]["farkli"] == 0 && r["misafir"]["en_yogun_gun"].is_null());
    assert!(r["saat"] == json!(vec![0; 24]) && r["kullanim"].get("alanlar").is_none() && r["misafir"]["gunluk_ortalama"] == json!(0.0));
    assert_eq!(oku(&tmp.join("rapor/2345678/liste.json")), json!([]));
}

fn tum_dosya_metni(d: &Path, metin: &mut String) {
    let mut adlar = liste(d);
    adlar.sort();
    for a in adlar {
        let y = d.join(&a);
        if y.is_dir() {
            tum_dosya_metni(&y, metin);
        } else {
            metin.push_str(&fs::read_to_string(&y).unwrap());
        }
    }
}

#[test]
fn raporlarda_kisisel_veri_yok() {
    let _k = Kayit::bas("test_rapor", "test_raporlarda_kisisel_veri_yok");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    for g in ["2026-10-05", "2026-10-06", "2026-10-07"] {
        zengin_gun(&k, g);
    }
    let d = gun_yaz(&k, "2026-10-08", G::default());
    fs::write(d.join("trafik.csv.gz"), b"bozuk").unwrap();
    manifest_yaz(&d, "2026-10-08");
    muhurle(&k, &["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08"]);
    let hatalar = calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-20", None);
    assert_eq!(hatalar.len(), 1);
    let mut metin = hatalar.join("\n");
    tum_dosya_metni(&tmp.join("rapor"), &mut metin);
    assert!(!metin.contains("instagram.com")); // yalnızca 2 misafir
    let mut gizli: Vec<String> = [TEL1, TEL2, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", MAC_MISAFIR, MAC_IZINLI, "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "csv", "kullanicilar"]
        .iter()
        .map(|x| x.to_string())
        .collect();
    gizli.extend([TEL1, TC, PAS].iter().map(|x| takma(&SIR, x)));
    gizli.push(takma(&SIR, &format!("mac:{MAC_IZINLI}")));
    for x in gizli {
        assert!(!metin.contains(&x), "{x}");
    }
    let r = rp(&tmp, "1234567", "ay-2026-10");
    for g in r["gunler"].as_array().unwrap() {
        let n = g["neden"].as_str().unwrap();
        assert!(["", "okunamadi", "dosya", "zincir_yok", "zincir", "manifest"].contains(&n), "{n}");
    }
}

#[test]
fn alan_adi_verisi_yok_ve_eski_bicim_yenilenir() {
    let _k = Kayit::bas("test_rapor", "test_alan_adi_verisi_yok_ve_eski_bicim_yenilenir");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    for g in ["2026-10-05", "2026-10-06", "2026-10-07"] {
        zengin_gun(&k, g);
    }
    muhurle(&k, &["2026-10-05", "2026-10-06", "2026-10-07"]);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    assert!(BICIM >= 2);
    // önceki sürümün yazdığı dosyalar elle kurulur (Python testi BICIM=1 ile üretiyordu): biçim 1 + alan adı listesi
    let (ham_d, rap_d) = (tmp.join("ham/1234567"), tmp.join("rapor/1234567"));
    for a_ in liste(&ham_d) {
        let mut h = oku(&ham_d.join(&a_));
        h["bicim"] = json!(1);
        h["alan"] = json!({"instagram.com": h["misafir"].clone()});
        fs::write(ham_d.join(&a_), serde_json::to_string(&h).unwrap()).unwrap();
    }
    for a_ in liste(&rap_d) {
        if a_ != "liste.json" {
            let mut r = oku(&rap_d.join(&a_));
            r["bicim"] = json!(1);
            r["kullanim"]["alanlar"] = json!([{"alan": "instagram.com", "misafir": 3}]);
            fs::write(rap_d.join(&a_), serde_json::to_string(&r).unwrap()).unwrap();
        }
    }
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    let mut metin = String::new();
    for d in [&ham_d, &rap_d] {
        for a_ in liste(d) {
            let v = oku(&d.join(&a_));
            if v.is_object() {
                assert_eq!(v["bicim"], BICIM, "{a_}");
                assert!(v.get("alan").is_none() && v["kullanim"].get("alanlar").is_none() && v.get("alanlar").is_none(), "{a_}");
            }
            metin.push_str(&serde_json::to_string(&v).unwrap());
        }
    }
    assert!(!metin.contains("instagram") && !metin.replace("alan_", "").contains("alan"));
    assert_eq!(rp(&tmp, "1234567", "ay-2026-10")["kullanim"], json!({"gonderilen": 3 * 112, "alinan": 3 * 1003, "sms": 6}));
}

#[test]
fn sir_arsiv_adiyla_cakismaz() {
    let _k = Kayit::bas("test_rapor", "test_sir_arsiv_adiyla_cakismaz");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    basit_gun(&k, "2026-10-05", TEL1);
    muhurle(&k, &["2026-10-05"]);
    let ham = tmp.join("ham");
    let hatalar = calistir(&arsivler(vec![("sir", ars(&[&k]))]), &tmp.join("yok.json"), &ham, &tmp.join("rapor"), None, Some(ts("2026-10-07", 12)), None).unwrap();
    assert_eq!(hatalar, bos());
    assert!(ham.join(".sir").is_file() && ham.join("sir/2026-10-05.json").is_file());
    assert!(SIR_ADI == ".sir" && !arsiv_adi_mi(".sir"));
}

// ======================================================================================================================
// Son inceleme düzeltmeleri (I1–I4, küçükler)
// ======================================================================================================================
#[test]
fn bozuk_manifest_sonraki_gunleri_durdurmaz() {
    let _k = Kayit::bas("test_rapor", "test_bozuk_manifest_sonraki_gunleri_durdurmaz");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-10-05", &[TEL1]);
    let d6 = misafir_gun(&k, "2026-10-06", &[TEL1]);
    misafir_gun(&k, "2026-10-07", &[TEL2]);
    misafir_gun(&k, "2026-10-08", &[TEL2]);
    muhurle(&k, &["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08"]);
    fs::write(d6.join("MANIFEST.sha256"), b"\xff\xfe\x00bozuk").unwrap();
    let d8 = k.join("gunluk/2026-10-08");
    fs::remove_file(d8.join("MANIFEST.sha256")).unwrap();
    fs::create_dir(d8.join("MANIFEST.sha256")).unwrap();
    assert_eq!(ham_guncelle("1234567", &[k.clone()], &tmp.join("ham"), &SIR).unwrap(), bos());
    assert_eq!(liste(&tmp.join("ham/1234567")), vec!["2026-10-05.json", "2026-10-07.json"]);
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-10", None), bos());
    let w41 = durum_haritasi(&rp(&tmp, "1234567", "hafta-2026-W41"));
    assert!(w41["2026-10-05"].0 == "tam" && w41["2026-10-07"].0 == "tam");
    assert!(w41["2026-10-06"].0 == "eksik" && w41["2026-10-08"].0 == "eksik");
}

#[test]
fn hizmet_oncesi_gunler_sayilmaz() {
    let _k = Kayit::bas("test_rapor", "test_hizmet_oncesi_gunler_sayilmaz");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    for g in ["2026-10-05", "2026-10-06", "2026-10-07"] {
        misafir_gun(&k, g, &[TEL1]);
    }
    muhurle(&k, &["2026-10-05", "2026-10-06", "2026-10-07"]);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-09", None), bos());
    let ay_ = rp(&tmp, "1234567", "ay-2026-10");
    let durum: BTreeMap<String, (String, Value)> =
        ay_["gunler"].as_array().unwrap().iter().map(|x| (x["gun"].as_str().unwrap().to_string(), (x["durum"].as_str().unwrap().to_string(), x["farkli"].clone()))).collect();
    for g in ["2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"] {
        assert_eq!(durum[g], (s("hizmet_oncesi"), Value::Null), "{g}");
    }
    assert_eq!(durum["2026-10-08"].0, "bekleniyor");
    assert_eq!(ay_["kayit"], json!({"gun": 4, "tam": 3})); // 10-05..10-08; 1-4 Ekim sayılmaz
    let l = oku(&tmp.join("rapor/1234567/liste.json"));
    let ekim = l.as_array().unwrap().iter().find(|x| x["donem"] == "2026-10").unwrap();
    assert!(ekim["kayit_gun"] == 4 && ekim["kayit_tam"] == 3);
    assert!(!tmp.join("rapor/1234567/hafta-2026-W40.json").exists());
    // geç gelen daha eski gün ilk günü geri çeker: 10-03 artık sayılır, 10-04 kayıt gelmedi olur
    misafir_gun(&k, "2026-10-03", &[TEL2]);
    muhurle(&k, &["2026-10-03", "2026-10-05", "2026-10-06", "2026-10-07"]);
    assert_eq!(calis(&tmp, &a, "2026-10-09", None), bos());
    let ay_ = rp(&tmp, "1234567", "ay-2026-10");
    let durum = durum_haritasi(&ay_);
    assert!(durum["2026-10-02"].0 == "hizmet_oncesi" && durum["2026-10-03"].0 == "tam");
    assert_eq!(durum["2026-10-04"].0, "kayit_yok");
    assert_eq!(ay_["kayit"], json!({"gun": 6, "tam": 4}));
}

#[test]
fn gun_durumu_hizmet_oncesi_ve_gelecek() {
    let _k = Kayit::bas("test_rapor", "test_gun_durumu_hizmet_oncesi_ve_gelecek");
    let b = tarih_(2026, 10, 9);
    let ilk = tarih_(2026, 10, 5);
    let bos_z = Zincir::new();
    assert_eq!(gun_durumu("2026-10-04", b, None, None, &bos_z, Some(ilk)), ("hizmet_oncesi", ""));
    assert_eq!(gun_durumu("2026-10-06", b, None, None, &bos_z, Some(ilk)), ("kayit_yok", ""));
    assert_eq!(gun_durumu("2026-10-10", b, None, None, &bos_z, Some(ilk)), ("gelecek", ""));
    assert_eq!(gun_durumu("2026-10-04", b, None, None, &bos_z, None), ("kayit_yok", "")); // ilk gün verilmezse eski davranış
}

#[test]
fn grup_kalkinca_raporlari_silinir() {
    let _k = Kayit::bas("test_rapor", "test_grup_kalkinca_raporlari_silinir");
    let tmp = gecici_dizin();
    let (k1, kb) = (tmp.join("a"), tmp.join("bocafe"));
    misafir_gun(&k1, "2026-10-05", &[TEL1]);
    misafir_gun(&kb, "2026-10-06", &[TEL2]);
    muhurle(&k1, &["2026-10-05"]);
    muhurle(&kb, &["2026-10-06"]);
    let a = arsivler(vec![("1234567", ars(&[&k1])), ("bocafe", ars(&[&kb]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-08", Some(json!({"4511643": ["bocafe"]}))), bos());
    let yanlis = tmp.join("rapor/4511643");
    assert!(!oku(&yanlis.join("liste.json")).as_array().unwrap().is_empty() && rp(&tmp, "4511643", "hafta-2026-W41")["misafir"]["farkli"] == 1);
    fs::write(yanlis.join("baska.txt"), "x").unwrap(); // bizim olmayan dosyaya dokunulmaz
    assert_eq!(calis(&tmp, &a, "2026-10-08", Some(json!({"1234567": ["bocafe"]}))), bos());
    assert_eq!(liste(&yanlis), vec!["baska.txt"]);
    assert_eq!(rp(&tmp, "1234567", "hafta-2026-W41")["misafir"]["farkli"], 2);
    fs::remove_file(yanlis.join("baska.txt")).unwrap();
    assert_eq!(calis(&tmp, &arsivler(vec![("bocafe", ars(&[&kb]))]), "2026-10-08", Some(json!({}))), bos()); // numara arşivi de kalktı
    assert!(!yanlis.exists());
    assert!(!tmp.join("rapor/1234567").exists());
}

#[test]
fn donem_yalnizca_kendi_gunlerinin_hamini_yukler() {
    let _k = Kayit::bas("test_rapor", "test_donem_yalnizca_kendi_gunlerinin_hamini_yukler");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    let gunler = ["2026-08-20", "2026-09-10", "2026-10-05", "2026-10-13"];
    for g in gunler {
        misafir_gun(&k, g, if g == "2026-10-05" { &[TEL1, TEL2][..] } else { &[TEL1][..] });
    }
    muhurle(&k, &gunler);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    DONEM_HAMLARI.with(|c| c.borrow_mut().clear());
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    let cagri = DONEM_HAMLARI.with(|c| c.borrow().clone());
    assert!(!cagri.is_empty());
    for (bas, bit, gs) in &cagri {
        for g in gs {
            assert!(gun_s(*bas).as_str() <= g.as_str() && g.as_str() <= gun_s(*bit).as_str());
        }
    }
    assert!(cagri.iter().map(|c| c.2.len()).max().unwrap() <= 2);
    DONEM_HAMLARI.with(|c| c.borrow_mut().clear());
    assert_eq!(calis_saat(&tmp, &a, "2026-10-20", None, 13), bos());
    assert!(DONEM_HAMLARI.with(|c| c.borrow().is_empty())); // hiçbir şey değişmedi: dönem hamları yüklenmez
    let m = rp(&tmp, "1234567", "ay-2026-10")["misafir"].clone();
    assert!(m["farkli"] == 2 && m["yeni"] == 1 && m["donen"] == 1);
    assert_eq!(rp(&tmp, "1234567", "ay-2026-08")["misafir"]["yeni"], 1);
    assert_eq!(rp(&tmp, "1234567", "ay-2026-09")["misafir"]["donen"], 1);
}

#[test]
fn sir_degisince_hamlar_yeniden_hesaplanir() {
    let _k = Kayit::bas("test_rapor", "test_sir_degisince_hamlar_yeniden_hesaplanir");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-09-10", &[TEL1]);
    misafir_gun(&k, "2026-10-05", &[TEL1, TEL2]);
    muhurle(&k, &["2026-09-10", "2026-10-05"]);
    let a = arsivler(vec![("1234567", ars(&[&k]))]);
    assert_eq!(calis(&tmp, &a, "2026-10-20", None), bos());
    let yeni_sir = [b't'; 32];
    let hatalar = calistir(&a, &tmp.join("yok.json"), &tmp.join("ham"), &tmp.join("rapor"), None, Some(ts("2026-10-20", 12)), Some(&yeni_sir)).unwrap();
    assert_eq!(hatalar, bos());
    let h = oku(&tmp.join("ham/1234567/2026-09-10.json"));
    assert_eq!(h["misafir"], json!([takma(&yeni_sir, TEL1)]));
    let m = rp(&tmp, "1234567", "ay-2026-10")["misafir"].clone();
    assert!(m["yeni"] == 1 && m["donen"] == 1);
}

#[test]
fn sir_izi_tutmayan_ham_kullanilmaz() {
    let _k = Kayit::bas("test_rapor", "test_sir_izi_tutmayan_ham_kullanilmaz");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-10-05", &[TEL1]);
    muhurle(&k, &["2026-10-05"]);
    ham_guncelle("1234567", &[k.clone()], &tmp.join("ham"), &[b't'; 32]).unwrap(); // başka sırla
    grup_raporla("1234567", &[(s("1234567"), k.clone())], &tmp.join("ham"), &tmp.join("rapor"), None, tarih_(2026, 10, 8), ts("2026-10-08", 12), Some(&SIR)).unwrap();
    let r = rp(&tmp, "1234567", "hafta-2026-W41");
    assert!(r["gunler"][0]["durum"] == "dogrulanamadi" && r["misafir"]["farkli"] == 0);
}

#[test]
fn gelecek_tarihli_gun_kesinlestirmez() {
    let _k = Kayit::bas("test_rapor", "test_gelecek_tarihli_gun_kesinlestirmez");
    let tmp = gecici_dizin();
    let k = tmp.join("a");
    misafir_gun(&k, "2026-10-05", &[TEL1]);
    misafir_gun_g(&k, "2026-10-16", &[TEL1], "10:00:00", vec![], vec![satir(&z("2026-10-16", "10:00:00"), "GUNCELLEME_TAMAM", &[("ek", "surum=0.9.9")])]);
    muhurle(&k, &["2026-10-05", "2026-10-16"]);
    assert_eq!(calis(&tmp, &arsivler(vec![("1234567", ars(&[&k]))]), "2026-10-14", None), bos());
    let r = rp(&tmp, "1234567", "hafta-2026-W41");
    assert!(r["kesin"] == false && r["surum_sonra_degisti"] == false);
    assert!(rp(&tmp, "1234567", "hafta-2026-W42")["surum"].is_null());
}

#[test]
fn tamam_mi_fifo_beklemez() {
    let _k = Kayit::bas("test_rapor", "test_tamam_mi_fifo_beklemez");
    let tmp = gecici_dizin();
    let d = tmp.join("gun");
    fs::create_dir(&d).unwrap();
    let yol = std::ffi::CString::new(d.join("MANIFEST.sha256").to_str().unwrap()).unwrap();
    // SAFETY: yol geçerli, NUL sonlu bir C dizisi
    assert_eq!(unsafe { libc::mkfifo(yol.as_ptr(), 0o600) }, 0);
    assert!(!istatistik::tamam_mi(&d));
}

#[test]
fn tamam_mi_bozuk_manifest() {
    let _k = Kayit::bas("test_rapor", "test_tamam_mi_bozuk_manifest");
    let tmp = gecici_dizin();
    let d = basit_gun(&tmp, "2026-10-05", TEL1);
    assert!(istatistik::tamam_mi(&d));
    fs::write(d.join("MANIFEST.sha256"), b"\xff\xfe bozuk").unwrap();
    assert!(!istatistik::tamam_mi(&d));
    fs::remove_file(d.join("MANIFEST.sha256")).unwrap();
    fs::create_dir(d.join("MANIFEST.sha256")).unwrap();
    assert!(!istatistik::tamam_mi(&d));
}

#[test]
fn zincir_eski_dosyalar_gerekmedikce_okunmaz() {
    let _k = Kayit::bas("test_rapor", "test_zincir_eski_dosyalar_gerekmedikce_okunmaz");
    let kok = gecici_dizin();
    fs::create_dir(kok.join("eski")).unwrap();
    for i in 0..5 {
        yaz_zincir(&kok.join("eski").join(format!("zincir.txt.2026-09-2{i}")), &zincir_satirlari(&[("2026-09-01", "a".repeat(64))]), "\n");
    }
    yaz_zincir(&kok.join("zincir.txt"), &zincir_satirlari(&[("2026-10-02", "c".repeat(64))]), "\n");
    OKUNAN.with(|o| o.borrow_mut().clear());
    let g1 = BTreeSet::from([s("2026-10-02")]);
    assert_eq!(zincir_oku(&kok, Some(&g1)), Zincir::from([(s("2026-10-02"), ("c".repeat(64), true))]));
    assert_eq!(OKUNAN.with(|o| o.borrow().clone()), vec!["zincir.txt"]);
    OKUNAN.with(|o| o.borrow_mut().clear());
    let g2 = BTreeSet::from([s("2026-10-02"), s("2026-09-01")]);
    assert_eq!(zincir_oku(&kok, Some(&g2))["2026-09-01"], ("a".repeat(64), true));
    assert_eq!(OKUNAN.with(|o| o.borrow().clone()), vec!["zincir.txt", "zincir.txt.2026-09-24"]); // en yeni eski kopya yetti
    let hepsi: Vec<String> = zincir_oku(&kok, None).keys().cloned().collect(); // gerekli verilmezse hepsi
    assert_eq!(hepsi, vec!["2026-09-01", "2026-10-02"]);
}

#[test]
fn gecici_dosya_hata_sonrasi_kalmaz() {
    let _k = Kayit::bas("test_rapor", "test_gecici_dosya_hata_sonrasi_kalmaz");
    let tmp = gecici_dizin();
    // Python: json.dump TypeError; burada yazma kapanışı hata verir (aynı yol: geçici dosya açıldıktan sonra hata)
    for kip in [0o600, 0o640] {
        let r = yaz_atomik_f(&tmp.join("a.json"), kip, None, |f| {
            f.write_all(b"yarim")?;
            Err(io::Error::other("serileştirilemez"))
        });
        assert!(r.is_err());
        assert!(liste(&tmp).is_empty());
    }
}

// --- Rust'a özgü ---
#[test]
fn ondalik_python_gibi_yuvarlar() {
    // Python: f"{x:.1f}" ikili değerin kendisi üzerinde, yarıya-çifte
    assert_eq!(ondalik1(0.25), "0.2");
    assert_eq!(ondalik1(0.35), "0.3"); // 0.35 ikilide biraz küçük
    assert_eq!(ondalik1(0.15), "0.1");
    assert_eq!(ondalik1(2.25), "2.2");
    assert_eq!(ondalik1(2.75), "2.8");
    assert_eq!(ondalik1(1.0), "1.0");
    assert_eq!(ondalik1(9.96), "10.0");
    assert_eq!(ondalik1(0.0), "0.0");
    assert_eq!(yuvarla1(4.55), 4.5);
}

#[test]
fn takma_python_ile_ayni_bayt_dizisi() {
    // py: hmac.new(b"s"*32, b"905551112233", sha256).hexdigest()[:16]
    assert_eq!(takma(&SIR, TEL1).len(), 16);
    assert_eq!(takma(&SIR, TEL1), takma(&SIR, TEL1));
    assert_ne!(takma(&SIR, TEL1), takma(&SIR, TEL2));
    assert_eq!(sir_izi(&SIR).len(), 8);
}
