//! kuyruk.rs testleri (Python tests/test_kuyruk.py'nin 8 testi). Gerçek `wificorrect-sunucu` hiçbir yerde çalıştırılmaz:
//! komut çalıştırıcı kayit.csv'yi gerçek komut gibi değiştiren bir sahtedir.
use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::gecici_dizin;

const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const WG2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test";

/// `wificorrect-sunucu` taklidi: kayit.csv'yi gerçek komut gibi değiştirir.
struct Sunucu {
    kayit: PathBuf,
    wg: PathBuf,
    komutlar: Vec<Vec<String>>,
}

impl Sunucu {
    fn yeni(tmp: &Path) -> Sunucu {
        let (kayit, wg) = (tmp.join("kayit.csv"), tmp.join("wg"));
        fs::create_dir(&wg).unwrap();
        for (ad, icerik) in [("sunucu.pub", format!("{}=\n", "S".repeat(43))), ("uc-nokta", "vpn.wificorrect.com:51820\n".to_string())] {
            fs::write(wg.join(ad), icerik).unwrap();
        }
        fs::write(&kayit, "").unwrap();
        Sunucu { kayit, wg, komutlar: Vec::new() }
    }

    fn cagri(&mut self, komut: &[String]) -> (i32, String, String) {
        self.komutlar.push(komut[1..].to_vec());
        let ham = fs::read_to_string(&self.kayit).unwrap();
        let mut satirlar: Vec<String> = ham.lines().map(String::from).collect();
        if komut[1] == "cihaz-ekle" {
            satirlar.push(format!("cihaz;{};10.99.0.{};{};wfc-{}", komut[2], 10 + satirlar.len(), komut[3], komut[2]));
        } else if komut[1] == "cihaz-kapat" {
            let onek = format!("cihaz;{};", komut[2]);
            satirlar = satirlar.into_iter().map(|s| if s.starts_with(&onek) { format!("{};2026-10-04", s.replacen("cihaz;", "bitti;", 1)) } else { s }).collect();
        }
        fs::write(&self.kayit, format!("{}\n", satirlar.join("\n"))).unwrap();
        (0, String::new(), String::new())
    }

    fn komut_adlari(&self) -> Vec<&str> {
        self.komutlar.iter().map(|k| k[0].as_str()).collect()
    }
}

fn isle_s(s: &mut Sunucu, is_: Value) -> Value {
    let (kayit, wg) = (s.kayit.clone(), s.wg.clone());
    isle(&is_, &mut |c: &[String]| s.cagri(c), &kayit, &wg)
}

fn calistir_s(s: &mut Sunucu, kq: &Path, sd: &Path) -> i32 {
    let (kayit, wg) = (s.kayit.clone(), s.wg.clone());
    calistir(kq, sd, &mut |c: &[String]| s.cagri(c), &kayit, &wg, None, None)
}

fn zaman_koy(yol: &Path, sn: u64) {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(sn);
    let f = fs::OpenOptions::new().write(true).open(yol).unwrap();
    f.set_times(fs::FileTimes::new().set_accessed(t).set_modified(t)).unwrap();
}

fn ekle_is(numara: &str, wg: &str) -> Value {
    json!({"islem": "cihaz-ekle", "numara": numara, "wg_pub": wg, "ssh_pub": SSH})
}

fn iki_dizin(tmp: &Path) -> (PathBuf, PathBuf) {
    let (kq, sd) = (tmp.join("kuyruk"), tmp.join("sonuc"));
    fs::create_dir(&kq).unwrap();
    fs::create_dir(&sd).unwrap();
    (kq, sd)
}

#[test]
fn ekle_ve_bekle() {
    let _k = Kayit::bas("test_kuyruk", "test_ekle_ve_bekle");
    let tmp = gecici_dizin();
    let (kq, sd) = iki_dizin(&tmp);
    let kimlik = ekle("cihaz-ekle", &json!({"numara": "1234567"}), &kq).unwrap();
    let is_: Value = serde_json::from_str(&fs::read_to_string(kq.join(format!("{kimlik}.json"))).unwrap()).unwrap();
    assert_eq!(is_, json!({"numara": "1234567", "islem": "cihaz-ekle"}));
    assert!(fs::read_dir(&kq).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    let mut uykular: Vec<f64> = Vec::new();
    assert!(bekle(&kimlik, 1.0, 0.5, &sd, &mut |s| uykular.push(s)).is_none() && uykular.len() == 3);
    fs::write(sd.join(format!("{kimlik}.json")), r#"{"durum": "tamam"}"#).unwrap();
    assert_eq!(bekle(&kimlik, 15.0, 0.5, &sd, &mut |s| uykular.push(s)), Some(json!({"durum": "tamam"})));
}

#[test]
fn cihaz_ekle_yeniden_ve_degisen_cihaz() {
    let _k = Kayit::bas("test_kuyruk", "test_cihaz_ekle_yeniden_ve_degisen_cihaz");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    let r = isle_s(&mut s, ekle_is("1234567", WG));
    assert_eq!(r, json!({"durum": "tamam", "tunel_ip": "10.99.0.10", "sunucu_pub": format!("{}=", "S".repeat(43)), "uc_nokta": "vpn.wificorrect.com:51820"}));
    let r2 = isle_s(&mut s, ekle_is("1234567", WG));
    assert!(r2["tunel_ip"] == "10.99.0.10" && s.komutlar.len() == 1); // aynı cihaz: komut çalışmaz
    isle_s(&mut s, ekle_is("1234567", WG2));
    assert_eq!(s.komut_adlari(), ["cihaz-ekle", "cihaz-kapat", "cihaz-ekle"]);
    assert_eq!(isle_s(&mut s, json!({"islem": "cihaz-kapat", "numara": "1234567"})), json!({"durum": "tamam"}));
    assert_eq!(isle_s(&mut s, json!({"islem": "cihaz-kapat", "numara": "1234567"})), json!({"durum": "tamam"})); // zaten kapalı
    assert!(s.komut_adlari().last() == Some(&"cihaz-kapat") && s.komutlar.len() == 4);
}

#[test]
fn gecersiz_girdiler_komut_calistirmaz() {
    let _k = Kayit::bas("test_kuyruk", "test_gecersiz_girdiler_komut_calistirmaz");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    let girdiler = [
        json!({"islem": "cihaz-ekle", "numara": "1; rm -rf /", "wg_pub": WG, "ssh_pub": SSH}),
        json!({"islem": "cihaz-ekle", "numara": 1234567, "wg_pub": WG, "ssh_pub": SSH}),
        json!({"islem": "cihaz-ekle", "numara": "1234567", "wg_pub": format!("{WG}\n[Peer]"), "ssh_pub": SSH}),
        json!({"islem": "cihaz-ekle", "numara": "1234567", "wg_pub": WG, "ssh_pub": format!("{SSH}\ncommand=x")}),
        json!({"islem": "kabuk", "numara": "1234567"}),
        json!({"islem": "cihaz-kapat", "numara": "1\u{ff12}34567"}),
        json!({"islem": "cihaz-kapat", "numara": "123456"}),
    ];
    for is_ in girdiler {
        assert_eq!(isle_s(&mut s, is_.clone())["durum"], "hata", "{is_}");
    }
    assert!(s.komutlar.is_empty());
}

#[test]
fn main_isler_siler_ve_cop_dosyada_donmez() {
    let _k = Kayit::bas("test_kuyruk", "test_main_isler_siler_ve_cop_dosyada_donmez");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    let (kq, sd) = iki_dizin(&tmp);
    let iyi = ekle("cihaz-ekle", &json!({"numara": "1234567", "wg_pub": WG, "ssh_pub": SSH}), &kq).unwrap();
    fs::write(kq.join("0123456789abcdef.json"), "{bozuk").unwrap();
    fs::write(kq.join("baska-ad.json"), "{}").unwrap(); // .path birimi bunu da görür: silinmeli
    fs::write(kq.join("fedcba9876543210.json"), "x".repeat(5000)).unwrap();
    assert_eq!(calistir_s(&mut s, &kq, &sd), 0);
    assert_eq!(fs::read_dir(&kq).unwrap().count(), 0);
    assert_eq!(sonuc(&iyi, &sd).unwrap()["tunel_ip"], "10.99.0.10");
    assert!(sonuc("0123456789abcdef", &sd).unwrap()["durum"] == "hata" && sonuc("fedcba9876543210", &sd).unwrap()["durum"] == "hata");
    zaman_koy(&sd.join(format!("{iyi}.json")), 1); // 1 günden eski sonuç temizlenir
    calistir_s(&mut s, &kq, &sd);
    assert!(sonuc(&iyi, &sd).is_none());
}

#[test]
fn sembolik_bag_ve_fifo_izlenmez() {
    let _k = Kayit::bas("test_kuyruk", "test_sembolik_bag_ve_fifo_izlenmez");
    let tmp = gecici_dizin();
    let hedef = tmp.join("gizli");
    fs::write(&hedef, r#"{"islem": "cihaz-kapat", "numara": "1234567"}"#).unwrap();
    std::os::unix::fs::symlink(&hedef, tmp.join("a.json")).unwrap();
    assert!(is_oku(&tmp.join("a.json")).is_none());
    let fifo = CString::new(tmp.join("b.json").as_os_str().as_bytes()).unwrap();
    // SAFETY: geçerli, NUL sonlu bir yol; yeni FIFO oluşturur
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(is_oku(&tmp.join("b.json")).is_none());
    // düz dosya okunur
    assert_eq!(is_oku(&hedef), Some(json!({"islem": "cihaz-kapat", "numara": "1234567"})));
}

#[test]
fn eski_kapatma_isi_yeni_cihazi_kapatmaz() {
    let _k = Kayit::bas("test_kuyruk", "test_eski_kapatma_isi_yeni_cihazi_kapatmaz");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    isle_s(&mut s, ekle_is("1234567", WG2));
    assert_eq!(isle_s(&mut s, json!({"islem": "cihaz-kapat", "numara": "1234567", "wg_pub": WG})), json!({"durum": "tamam"}));
    assert_eq!(s.komut_adlari(), ["cihaz-ekle"]);
    isle_s(&mut s, json!({"islem": "cihaz-kapat", "numara": "1234567", "wg_pub": WG2}));
    assert_eq!(s.komut_adlari(), ["cihaz-ekle", "cihaz-kapat"]);
    assert_eq!(isle_s(&mut s, json!({"islem": "cihaz-kapat", "numara": "1234567", "wg_pub": "kotu"}))["durum"], "hata");
}

#[test]
fn isler_yazilis_sirasiyla_islenir() {
    let _k = Kayit::bas("test_kuyruk", "test_isler_yazilis_sirasiyla_islenir");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    isle_s(&mut s, ekle_is("1234567", WG));
    let (kq, sd) = iki_dizin(&tmp);
    for (ad, is_, zaman) in [
        ("ffffffffffffffff", json!({"islem": "cihaz-kapat", "numara": "1234567", "wg_pub": WG}), 1000),
        ("0000000000000000", ekle_is("1234567", WG), 2000),
    ] {
        let yol = kq.join(format!("{ad}.json"));
        fs::write(&yol, is_.to_string()).unwrap();
        zaman_koy(&yol, zaman);
    }
    calistir_s(&mut s, &kq, &sd);
    // önce kapat, sonra ekle: cihaz açık kalır
    assert!(kayit_satiri("1234567", &s.kayit).is_some());
}

#[test]
fn ayni_anahtar_eski_kayitta_ise_once_o_kapanir() {
    let _k = Kayit::bas("test_kuyruk", "test_ayni_anahtar_eski_kayitta_ise_once_o_kapanir");
    let tmp = gecici_dizin();
    let mut s = Sunucu::yeni(&tmp);
    fs::write(&s.kayit, format!("cihaz;bocafe-test;10.99.0.10;{WG};wfc-bocafe-test\n")).unwrap();
    let r = isle_s(&mut s, ekle_is("1234567", WG));
    let ilk_ikisi: Vec<Vec<&str>> = s.komutlar.iter().map(|k| k.iter().take(2).map(String::as_str).collect()).collect();
    assert!(r["durum"] == "tamam" && ilk_ikisi == [vec!["cihaz-kapat", "bocafe-test"], vec!["cihaz-ekle", "1234567"]]);
    let mut f = fs::OpenOptions::new().append(true).open(&s.kayit).unwrap();
    std::io::Write::write_all(&mut f, format!("cihaz;7654321;10.99.0.20;{WG2};wfc-7654321\n").as_bytes()).unwrap();
    drop(f);
    let r = isle_s(&mut s, ekle_is("2345678", WG2));
    assert!(r["durum"] == "hata" && r["hata"].as_str().unwrap().contains("başka müşteri")); // başka müşterinin cihazı kapatılmaz
}

#[test]
fn ssh_ve_wg_desenleri() {
    for ok in [WG.to_string(), format!("{}+/=", "a".repeat(41))] {
        assert!(wg_gecerli(&ok), "{ok}");
    }
    for kotu in [String::new(), WG[..43].to_string(), format!("{WG}
"), format!("{}==", "A".repeat(42)), format!("{}-=", "A".repeat(42))] {
        assert!(!wg_gecerli(&kotu), "{kotu}");
    }
    for ok in [SSH, "ssh-ed25519 AAAA", "ssh-ed25519 AAAA==", "ssh-ed25519 AAAA== a@b.c_d-e", "ssh-ed25519 A+/9=  ".trim_end()] {
        assert!(ssh_gecerli(ok), "{ok}");
    }
    for kotu in [
        "",
        "ssh-rsa AAAA",
        "ssh-ed25519 ",
        "ssh-ed25519 =",
        "ssh-ed25519 AAAA ",
        "ssh-ed25519 AAAA a b",
        "ssh-ed25519 AAAA\ncommand=x",
        "ssh-ed25519 AAAA a\n",
        "ssh-ed25519 AA=A",
    ] {
        assert!(!ssh_gecerli(kotu), "{kotu:?}");
    }
    assert!(is_adi_gecerli("0123456789abcdef.json") && !is_adi_gecerli("0123456789ABCDEF.json") && !is_adi_gecerli("0123456789abcde.json"));
}

#[test]
fn satirlar_python_splitlines_gibi() {
    assert_eq!(satirlar(""), Vec::<&str>::new());
    assert_eq!(satirlar("a\nb\n"), ["a", "b"]);
    assert_eq!(satirlar("\n"), [""]);
    assert_eq!(satirlar("a\r\nb\rc"), ["a", "b", "c"]);
    assert_eq!(satirlar("a\n\nb"), ["a", "", "b"]);
}
