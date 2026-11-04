//! musteri.rs testleri: Python tests/test_musteri.py (8), test_misafir_kodu.py'nin Musteri'li 3'ü
//! (`test_merkezde_tam_kimlik_sizmaz`, `test_merkez_tc_pasaport_oda_aramasi`, `test_telefonlu_eski_arama_aynen`),
//! test_raporlar.py'nin Musteri'li 7'si ve test_bekleme.py'nin müşteri `/baglan` akışını kullanan 14'ü.
//! (`test_musteri_cerezi_yonetimde_gecmez` ve `test_yonetim_bekleyen_listesi` Yonetim ister: Görev 11.)
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

use super::*;
use crate::altin::Kayit;
use crate::api::testler as at;
use crate::guvenlik::Oturum;
use crate::kayitlar::son_30_gun;
use crate::ozet::testler::{gz_yaz, manifest};
use crate::rapor::testler as rt;
use crate::raporlar::testler as rr;
use crate::test_yardim::{self as y, Ortam, SAAT_BASLANGIC};
use crate::web::Yanit;

const WG: &str = at::WG;
const SSH: &str = at::SSH;
/// test_musteri.py ve test_raporlar.py'nin SSH sabiti (test_bekleme'ninkinden farklı).
const SSH_C: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c";

fn kur() -> (Musteri, Ortam) {
    let o = y::ortam();
    (Musteri::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat()), o)
}

fn kayitlar_of(o: &Ortam) -> Kayitlar {
    Kayitlar::new(o.ist.clone(), o.detay.clone(), o.kisi_sir.clone())
}

fn govde(r: &Yanit) -> String {
    r.govde_metin()
}

fn obj(k: &str, v: Value) -> Value {
    let mut m = Map::new();
    m.insert(k.to_string(), v);
    Value::Object(m)
}

fn olaylar(v: &Veri, n: i64) -> Vec<String> {
    v.hareketler(Some(n), 300, None).unwrap().into_iter().map(|h| h.olay).collect()
}

// ======================================================================================================================
// test_musteri.py
// ======================================================================================================================
#[test]
fn giris_ve_kendi_kayitlari() {
    let _k = Kayit::bas("test_musteri", "test_giris_ve_kendi_kayitlari");
    let (app, o) = kur();
    let v = &o.veri;
    let (n, pw) = v.musteri_ekle("", None).unwrap();
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    let dun = son_30_gun(o.saat.t())[0].format("%Y-%m-%d").to_string();
    y::sayi_yaz(&o.dizin, &n.to_string(), &obj(&dun, json!(14)));
    y::sayi_yaz(&o.dizin, &n2.to_string(), &obj(&dun, json!(777)));
    assert_eq!(y::giris(&app, &n.to_string(), "yanlis").durum, 401);
    assert_eq!(y::giris(&app, "abc", &pw).durum, 401);
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    assert!(c.starts_with("wc_musteri="));
    let s = govde(&y::al(&app, "/", &c));
    assert!(s.contains(">14<") && !s.contains("777") && s.contains(&format!("Müşteri {n}")));
    assert!(s.contains("<header>") && !s.contains("id=\"menu-ac\"")); // müşteri paneli üst şeritte (sol menü yalnızca yönetimde)
    assert!(s.contains("<header><a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\"><svg class=\"logo\"") && s.contains("rel=\"icon\""));
    v.cihaz_bagla(n, WG, SSH_C, "Bocafe Göztepe", "", "").unwrap();
    assert!(govde(&y::al(&app, "/", &c)).contains("Bocafe Göztepe"));
    assert!(!v.musteri(n).unwrap().unwrap().son_giris.is_empty() && olaylar(v, n) == ["GIRIS"]);
}

#[test]
fn gerekce_olmadan_kisi_verisi_yok() {
    let _k = Kayit::bas("test_musteri", "test_gerekce_olmadan_kisi_verisi_yok");
    let (app, o) = kur();
    let v = &o.veri;
    let (n, pw) = v.musteri_ekle("", None).unwrap();
    y::gun_yaz(&o.dizin, &n.to_string(), "2026-09-20", &[json!({"telefon": "5330000001", "ad": "Ayşe"})], None);
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    let r = y::al(&app, "/gun/2026-09-20", &c);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/gerekce?donus=%2Fgun%2F2026-09-20"));
    assert!(y::al(&app, "/ara", &c).durum == 200 && !govde(&y::al(&app, "/ara?q=ay", &c)).contains("Ayşe")); // GET aramaz
    let r = y::gonder(&app, "/ara", &c, &[("csrf", &y::csrf(&y::al(&app, "/ara", &c))), ("q", "ay")]);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/gerekce?donus=%2Fara")); // POST arama gerekçe ister
    assert_eq!(y::gerekce_ver_metin(&app, &c, "x").durum, 400);
    assert_eq!(y::gerekce_ver(&app, &c).durum, 303);
    assert!(govde(&y::al(&app, "/gun/2026-09-20", &c)).contains("Ayşe"));
    let ol = olaylar(v, n);
    assert_eq!(ol[..2], ["GORUNTULEME", "GEREKCE"]);
    o.saat.ilerlet(1801.0);
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    assert_eq!(y::al(&app, "/gun/2026-09-20", &c).durum, 303); // yeni oturum, gerekçe yok
}

#[test]
fn parola_degistirme_ve_uyelik_bitti_girer() {
    let _k = Kayit::bas("test_musteri", "test_parola_degistirme_ve_uyelik_bitti_girer");
    let (app, o) = kur();
    let v = &o.veri;
    let (n, pw) = v.musteri_ekle("", None).unwrap();
    v.uyelik(n, "bitti").unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw)); // üyelik bitse de kayıtlarına bakar
    let f = y::al(&app, "/parola", &c);
    let csrf = y::csrf(&f);
    let g = |mevcut: &str, tekrar: &str| y::gonder(&app, "/parola", &c, &[("csrf", &csrf), ("mevcut", mevcut), ("yeni", "yeni-parola-1"), ("tekrar", tekrar)]);
    assert_eq!(g("yanlis", "yeni-parola-1").durum, 400);
    assert_eq!(g(&pw, "baska").durum, 400);
    let r = g(&pw, "yeni-parola-1");
    assert!(r.durum == 200 && govde(&r).contains("06:00") && v.parola_dogrula(n, "yeni-parola-1").unwrap().is_some());
    assert_eq!(y::al(&app, "/", &c).durum, 303); // eski oturum kapandı
    assert_eq!(y::al(&app, "/", &y::cerez(&r)).durum, 200);
}

#[test]
fn baska_musterinin_yollari_yok() {
    let _k = Kayit::bas("test_musteri", "test_baska_musterinin_yollari_yok");
    let (app, o) = kur();
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    for yol in ["/m/100002/", "/m/100001/kayitlar/", "/hareketler", "/yeni"] {
        assert_eq!(y::al(&app, yol, &c).durum, 404, "{yol}");
    }
}

#[test]
fn unicode_rakamli_numara_reddedilir() {
    let _k = Kayit::bas("test_musteri", "test_unicode_rakamli_numara_reddedilir");
    let (app, o) = kur();
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    assert_eq!(y::giris(&app, "1\u{ff10}0001", &pw).durum, 401);
    assert_eq!(y::giris(&app, &"\u{00b2}".repeat(7), &pw).durum, 401);
    assert_eq!(y::giris(&app, &n.to_string()[..6], &pw).durum, 401); // 6 hane
}

#[test]
fn parola_sifirlaninca_eski_oturum_duser() {
    let _k = Kayit::bas("test_musteri", "test_parola_sifirlaninca_eski_oturum_duser");
    let (app, o) = kur();
    let v = &o.veri;
    let (n, pw) = v.musteri_ekle("", None).unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    assert_eq!(y::al(&app, "/", &c).durum, 200);
    v.parola_koy(n, &guvenlik::parola_uret(12)).unwrap(); // Python parola_sifirla(n)
    assert_eq!(y::al(&app, "/", &c).durum, 303);
}

#[test]
fn gerekce_donusu_baska_siteye_gitmez() {
    let _k = Kayit::bas("test_musteri", "test_gerekce_donusu_baska_siteye_gitmez");
    let (app, o) = kur();
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    let f = y::al(&app, "/gerekce", &c);
    for kotu in ["/\t/evil.example", "//evil.example", "/\\evil.example", "/\n/x", "https://evil.example", "/ /evil.example"] {
        let r = y::gonder(&app, "/gerekce", &c, &[("csrf", &y::csrf(&f)), ("gerekce", "Müşteri şikâyeti"), ("donus", kotu)]);
        assert_eq!(r.baslik("Location"), Some("/"), "{kotu}");
    }
}

/// Çıkış CSRF'li POST'la: oturum kapanır, çerez silinir; aynı çerezle sayfa giriş sayfasına döner.
#[test]
fn cikis_oturumu_kapatir() {
    let _k = Kayit::bas("test_musteri", "test_cikis_oturumu_kapatir");
    let (app, o) = kur();
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    let f = y::al(&app, "/parola", &c);
    assert_eq!(y::gonder(&app, "/cikis", &c, &[("csrf", "yanlis")]).durum, 403);
    let r = y::gonder(&app, "/cikis", &c, &[("csrf", &y::csrf(&f))]);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/giris"));
    assert_eq!(r.baslik("Set-Cookie"), Some("wc_musteri=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"));
    assert_eq!(y::al(&app, "/parola", &c).baslik("Location"), Some("/giris"));
}

// ======================================================================================================================
// test_misafir_kodu.py (Musteri ile)
// ======================================================================================================================
const TC: &str = "TC:10000000146";
const PAS: &str = "PASAPORT:DE:X1234567";
const TEL: &str = "905330000001";
const GUN: &str = "2026-09-20";
const TAM: [&str; 3] = ["10000000146", "11111111110", "X1234567"];

/// Bir telefonlu, bir TC'li (oda 101) ve bir pasaportlu (oda 202) kişi.
fn arsiv_yaz(kok: &Path) {
    let d = kok.join("gunluk").join(GUN);
    let z = |s: &str| format!("{GUN}T{s}+03:00");
    let misafir = |oda: &str| format!("sozlesme=1 riza=1 giris=misafir_kodu oda={oda} kod_bitis=2026-09-21T12:00:00+03:00 ureten=mudur");
    let (z0900, z1000, z1100, z1005, z1106) = (z("09:00:00"), z("10:00:00"), z("11:00:00"), z("10:05:00"), z("11:06:00"));
    let (m101, m202) = (misafir("101"), misafir("202"));
    gz_yaz(
        &d.join("oturum.csv.gz"),
        &[
            vec![("zaman", z0900.as_str()), ("olay", "OTURUM_BASLA"), ("telefon", TEL), ("ad", "Ayşe"), ("soyad", "Yılmaz"), ("mac", "aa:aa:aa:aa:aa:01"), ("ic_ip", "10.50.0.21"), ("ek", "sozlesme=1 riza=1")],
            vec![("zaman", z1000.as_str()), ("olay", "OTURUM_BASLA"), ("telefon", TC), ("ad", "Deniz"), ("soyad", "Kara"), ("mac", "aa:aa:aa:aa:aa:02"), ("ic_ip", "10.50.0.22"), ("ek", m101.as_str())],
            vec![("zaman", z1100.as_str()), ("olay", "OTURUM_BASLA"), ("telefon", PAS), ("ad", "Hans"), ("soyad", "Muller"), ("mac", "aa:aa:aa:aa:aa:03"), ("ic_ip", "10.50.0.23"), ("ek", m202.as_str())],
        ],
    );
    gz_yaz(&d.join("dns.csv.gz"), &[vec![("zaman", z1005.as_str()), ("olay", "DNS"), ("telefon", TC), ("alan_adi", "ornek.com")]]);
    gz_yaz(
        &d.join("trafik.csv.gz"),
        &[vec![("zaman", z1106.as_str()), ("olay", "BAGLANTI_BITIS"), ("telefon", PAS), ("protokol", "tcp"), ("hedef_ip", "1.1.1.1"), ("hedef_port", "443"), ("gonderilen_bayt", "10"), ("alinan_bayt", "90"), ("sure_sn", "6")]],
    );
    gz_yaz(&d.join("kullanicilar").join("TC_10000000146.csv.gz"), &[vec![("zaman", z1000.as_str()), ("olay", "KAYIT"), ("telefon", TC), ("ad", "Deniz"), ("soyad", "Kara")]]);
    manifest(&d, &["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz", "kullanicilar/TC_10000000146.csv.gz"]);
}

/// Python `kur(tmp)` (test_misafir_kodu): Musteri + arşiv + özetler + giriş + gerekçe.
fn kur_misafir() -> (Musteri, Ortam, i64, String) {
    let (app, o) = kur();
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    arsiv_yaz(&o.dizin.join("arsiv"));
    let hatalar = crate::ozet::detay_guncelle(&o.dizin.join("arsiv"), &o.dizin.join("detay").join(n.to_string()), None).unwrap();
    assert!(hatalar.is_empty(), "{hatalar:?}");
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    y::gerekce_ver(&app, &c);
    (app, o, n, c)
}

fn ara(app: &Musteri, c: &str, q: &str, tur: &str, uyruk: &str) -> String {
    let f = y::al(app, "/ara", c);
    let r = y::gonder(app, "/ara", c, &[("csrf", &y::csrf(&f)), ("q", q), ("tur", tur), ("uyruk", uyruk)]);
    assert_eq!(r.durum, 200);
    govde(&r)
}

/// `(?:href|action|id)="([^"]*)"` eşleşmeleri.
fn niteliklar(h: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < h.len() {
        let kalan = &h[i..];
        let on = ["href=\"", "action=\"", "id=\""].iter().find(|p| kalan.starts_with(**p));
        match on {
            Some(p) => {
                let bas = i + p.len();
                match h[bas..].find('"') {
                    Some(son) => {
                        out.push(h[bas..bas + son].to_string());
                        i = bas + son + 1;
                    }
                    None => break,
                }
            }
            None => i += kalan.chars().next().map(char::len_utf8).unwrap_or(1),
        }
    }
    out
}

/// `re.sub(r"k[0-9a-f]{16}", "", s)`.
fn capa_sil(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'k' && i + 17 <= b.len() && b[i + 1..i + 17].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c)) {
            i += 17;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn bes_rakam(s: &str) -> bool {
    s.as_bytes().windows(5).any(|w| w.iter().all(u8::is_ascii_digit))
}

fn sizmaz(h: &str) {
    for t in TAM {
        assert!(!h.contains(t), "{t}");
    }
    for nitelik in niteliklar(h) {
        // adres, çapa, form hedefi: kimlik ve telefon yok
        let temiz = capa_sil(&nitelik).replace("2026-09-20", ""); // opak çapada rastgele rakam olabilir
        assert!(!bes_rakam(&temiz), "{nitelik}");
        assert!(!nitelik.contains("X123") && !nitelik.contains(TEL), "{nitelik}");
    }
}

#[test]
fn merkezde_tam_kimlik_sizmaz() {
    let _k = Kayit::bas("test_misafir_kodu", "test_merkezde_tam_kimlik_sizmaz");
    let (app, o, n, c) = kur_misafir();
    let sayfalar = vec![
        govde(&y::al(&app, "/", &c)),
        govde(&y::al(&app, &format!("/gun/{GUN}"), &c)),
        govde(&y::al(&app, "/ara", &c)),
        ara(&app, &c, "10000000146", "kisi", ""),
        ara(&app, &c, "100 000 001 46", "kisi", ""),
        ara(&app, &c, "X1234567", "kisi", "DE"),
        ara(&app, &c, "x1234567", "kisi", ""),
        ara(&app, &c, "101", "oda", ""),
        ara(&app, &c, "202", "oda", ""),
        ara(&app, &c, "Deniz", "kisi", ""),
        ara(&app, &c, "0533 000 00 01", "kisi", ""),
    ];
    let gun = &sayfalar[1];
    assert!(gun.contains("100******46") && gun.contains("DE X12***67") && gun.contains("Deniz Kara") && gun.contains("Hans Muller"));
    assert!(gun.contains("oda=101")); // oturum notu açık (kimlik içermez)
    for h in &sayfalar {
        sizmaz(h);
    }
    for h in &sayfalar[3..5] {
        assert!(h.contains("100******46") && h.contains("Deniz Kara"));
    }
    assert!(sayfalar[5].contains("DE X12***67") && sayfalar[6].contains("DE X12***67"));
    // çapalar opak ve gün sayfasındaki kişiye götürür
    let ayrac = "href=\"/gun/2026-09-20#";
    let bas = sayfalar[3].find(ayrac).expect("çapa") + ayrac.len();
    let capa = &sayfalar[3][bas..bas + 17];
    assert!(capa.starts_with('k') && capa[1..].bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    assert_eq!(sayfalar[3].as_bytes()[bas + 17], b'"');
    assert!(gun.contains(&format!("id=\"{capa}\"")));
    // hareket kaydına kimlik girmez
    for h in o.veri.hareketler(Some(n), 300, None).unwrap() {
        for t in TAM {
            assert!(!h.ayrinti.contains(t), "{}", h.ayrinti);
        }
    }
    // GET ile sorgu dizgisindeki değer aranmaz, sayfaya yansımaz
    let h = govde(&y::al(&app, "/ara?q=10000000146", &c));
    assert!(!h.contains("10000000146") && !h.contains("Deniz Kara"));
}

#[test]
fn merkez_tc_pasaport_oda_aramasi() {
    let _k = Kayit::bas("test_misafir_kodu", "test_merkez_tc_pasaport_oda_aramasi");
    let (app, _o, _n, c) = kur_misafir();
    let a = |q: &str, tur: &str, uyruk: &str| ara(&app, &c, q, tur, uyruk);
    assert!(a("10000000146", "kisi", "").contains("Deniz Kara") && a("100-000-001.46", "kisi", "").contains("Deniz Kara"));
    assert!(!a("10000000146", "kisi", "").contains("Hans Muller"));
    assert!(!a("10000000147", "kisi", "").contains("Deniz Kara")); // kontrol hanesi bozuk: kimlik araması yok
    assert!(!a("000000", "kisi", "").contains("Deniz Kara")); // kimlik numarasında parça araması yok
    assert!(a("x1234567", "kisi", "").contains("Hans Muller") && a("X1234567", "kisi", "de").contains("Hans Muller"));
    assert!(!a("X1234567", "kisi", "FR").contains("Hans Muller"));
    assert!(!a("X12345", "kisi", "").contains("Hans Muller")); // pasaportta da parça araması yok
    let h = a("101", "oda", "");
    assert!(h.contains("Deniz Kara") && !h.contains("Hans Muller") && !h.contains("Ayşe"));
    assert!(a("202", "oda", "").contains("Hans Muller"));
    assert!(a("1010", "oda", "").contains("Sonuç yok") && a("10", "oda", "").contains("Sonuç yok"));
}

#[test]
fn telefonlu_eski_arama_aynen() {
    let _k = Kayit::bas("test_misafir_kodu", "test_telefonlu_eski_arama_aynen");
    let dizin: crate::detay::Dizin = vec![
        (TEL.to_string(), json!({"ad": "Ayşe", "soyad": "Yılmaz", "gunler": [GUN]})),
        (TC.to_string(), json!({"ad": "Deniz", "soyad": "Kara", "gunler": [GUN]})),
    ];
    let teller = |q: &str| -> Vec<String> { crate::detay::ara_sonuclari(&dizin, q, "kisi", "").into_iter().map(|(t, _)| t).collect() };
    for sorgu in [TEL, "0533 000 0001", "+90 533 000 00 01", "05330000001", "533", "0533", "0001"] {
        assert_eq!(teller(sorgu), [TEL], "{sorgu}");
    }
    assert_eq!(teller("ayşe"), [TEL]);
    assert!(teller("0544").is_empty());
    assert_eq!(teller("kara"), [TC]);
    let (app, _o, _n, c) = kur_misafir();
    let h = ara(&app, &c, "0533 000 00 01", "kisi", "");
    assert!(h.contains("Ayşe Yılmaz") && h.contains(TEL) && !h.contains("Deniz"));
    assert!(h.contains("value=\"0533 000 00 01\"")); // telefon/ad araması formda kalır
    assert!(ara(&app, &c, "ayşe", "kisi", "").contains("Ayşe"));
}

// ======================================================================================================================
// test_raporlar.py (müşteri paneli)
// ======================================================================================================================
fn hafta_raporu() -> Value {
    let mut g = rr::gunler("2026-09-14", 7);
    g.truncate(6);
    g.push(json!({"gun": "2026-09-20", "durum": "bekleniyor", "neden": "", "farkli": null}));
    json!({
        "bicim": 2, "tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20",
        "uretim": "2026-09-21T02:31:07+03:00", "kesin": false, "imza": "y".repeat(64),
        "misafir": {"farkli": 6, "yeni": 1, "donen": 5, "giris": 9, "giris_sms": 9, "giris_kod": 0, "izinli_cihaz": 1,
                    "gunluk_ortalama": 2.0, "en_yogun_gun": null},
        "saat": vec![0; 24], "hafta_gunu": vec![0; 7], "gunler": g,
        "kayit": {"gun": 7, "tam": 6},
        "kullanim": {"gonderilen": 0, "alinan": 0, "sms": 9}, "surum": null, "surum_sonra_degisti": false
    })
}

fn liste() -> Value {
    Value::Array(rr::liste_ornegi())
}

/// Python `kur_musteri(tmp)`: Raporlar(tmp/rapor) ile Musteri, bir müşteri, girişli çerez.
fn kur_musteri() -> (Musteri, Ortam, i64, String) {
    let o = y::ortam();
    let app = Musteri::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat()).raporlar_ile(Raporlar::new(o.dizin.join("rapor")));
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    let c = y::cerez(&y::giris(&app, &n.to_string(), &pw));
    (app, o, n, c)
}

fn rapor_yaz(o: &Ortam, n: i64, ad: &str, icerik: &Value) {
    rr::yaz(&o.dizin, &n.to_string(), ad, icerik);
}

#[test]
fn musteri_liste_sayfasi() {
    let _k = Kayit::bas("test_raporlar", "test_musteri_liste_sayfasi");
    let (app, o, n, c) = kur_musteri();
    let s = govde(&y::al(&app, "/raporlar", &c));
    assert!(s.contains("Henüz rapor yok"));
    rapor_yaz(&o, n, "liste.json", &liste());
    let r = y::al(&app, "/raporlar", &c);
    let s = govde(&r);
    let i = |x: &str| s.find(x).unwrap_or_else(|| panic!("{x}"));
    assert!(r.durum == 200 && i("Haftalık raporlar") < i("Aylık raporlar"));
    assert!(i("21–27.09.2026") < i("14–20.09.2026") && i("14–20.09.2026") < i("31.08–06.09.2026"));
    assert!(i("Eylül 2026") < i("Ağustos 2026"));
    assert_eq!(s.matches("sürüyor").count(), 2); // W39 ve Eylül
    assert!(s.contains("7 günün 6'sı tam") && s.contains("31 günün 31'i tam") && s.contains("20 günün 16'sı tam"));
    assert!(s.contains("href=\"/rapor/hafta/2026-W38\"") && s.contains("href=\"/rapor/ay/2026-08\""));
    assert!(!s.contains("<script"));
}

#[test]
fn musteri_menu_ve_rapor_sayfasi() {
    let _k = Kayit::bas("test_raporlar", "test_musteri_menu_ve_rapor_sayfasi");
    let (app, o, n, c) = kur_musteri();
    let v = &o.veri;
    v.cihaz_bagla(n, WG, SSH_C, "Bocafe Göztepe", "Bocafe Gıda Ltd. Şti.", "0.5.0").unwrap();
    rapor_yaz(&o, n, "ay-2026-09.json", &rr::ay_raporu(&[]));
    let r = y::al(&app, "/rapor/ay/2026-09", &c);
    let s = govde(&r);
    assert_eq!(r.durum, 200);
    let menu = &s[s.find("<nav>").unwrap()..s.find("</nav>").unwrap()];
    assert!(menu.find("Yedek arşiv").unwrap() < menu.find("Raporlar").unwrap() && menu.find("Raporlar").unwrap() < menu.find("Parola").unwrap());
    for b in ["5651 kayıt durumu", "Misafirler", "Yoğunluk", "Kullanım", "Cihaz"] {
        assert!(s.contains(&format!("<h2>{b}</h2>")), "{b}");
    }
    assert!(s.find("5651 kayıt durumu").unwrap() < s.find("<h2>Misafirler").unwrap());
    assert!(!s.contains("Engellen"));
    assert!(s.contains("Bocafe Göztepe") && s.contains("Bocafe Gıda Ltd. Şti.") && s.contains(&n.to_string()) && s.contains("Eylül 2026"));
    assert!(s.contains("21.09.2026 12:31") && s.contains("sürüyor") && s.contains("02:30"));
    assert!(s.contains("20 günün 16'sı tam"));
    assert!(s.contains("gün mühür zincirinde yok") && s.contains("Kayıt gelmedi") && s.contains("Eksik geldi"));
    assert!(!s.contains("alert(1)") && !s.contains("<script"));
    assert!(!s.contains("instagram") && !s.to_lowercase().contains("alan adı") && !s.contains("en az 3 farklı misafir"));
    assert!(s.contains("4,0 GB") && s.contains("4,5") && s.contains("12.09.2026"));
    assert!(s.contains("@media print") && s.contains("class=\"yazdirma") && s.contains("Ctrl+P") && s.contains("href=\"/raporlar\""));
    assert!(s.contains("0.4.9"));
    // gerekçe istenmez, hareket yazılmaz
    assert_eq!(olaylar(v, n), ["GIRIS"]);
    rapor_yaz(&o, n, "hafta-2026-W38.json", &hafta_raporu());
    let s = govde(&y::al(&app, "/rapor/hafta/2026-W38", &c));
    assert!(s.contains("14–20.09.2026") && s.contains("7 günün 6'sı tam"));
    assert!(s.contains("0.5.0")); // sürüm null ve sonra değişmedi: cihazın bugünkü sürümü
    assert_eq!(olaylar(v, n), ["GIRIS"]);
}

/// Cihaz kurulmadan önceki günler 'Kayıt gelmedi' değil, nötr 'Hizmet öncesi' görünür.
#[test]
fn hizmet_oncesi_gunler() {
    let _k = Kayit::bas("test_raporlar", "test_hizmet_oncesi_gunler");
    let (app, o, n, c) = kur_musteri();
    o.veri.cihaz_bagla(n, WG, SSH_C, "Bocafe Göztepe", "Bocafe Gıda Ltd. Şti.", "0.5.0").unwrap();
    let mut r = rr::ay_raporu(&[]);
    r["gunler"][4]["durum"] = json!("tam");
    r["gunler"][4]["farkli"] = json!(2);
    for i in 0..3 {
        r["gunler"][i]["durum"] = json!("hizmet_oncesi");
        r["gunler"][i]["neden"] = json!("");
        r["gunler"][i]["farkli"] = Value::Null;
    }
    rapor_yaz(&o, n, "ay-2026-09.json", &r);
    let okunan = Raporlar::new(o.dizin.join("rapor")).rapor(n, "ay", "2026-09").unwrap();
    assert!(okunan.gunler[..3].iter().all(|x| x.durum == "hizmet_oncesi"));
    let s = govde(&y::al(&app, "/rapor/ay/2026-09", &c));
    assert!(s.matches("Hizmet öncesi").count() == 3 && !s.contains("Kayıt gelmedi"));
}

#[test]
fn musteri_yollar_404_ve_baska_musteri() {
    let _k = Kayit::bas("test_raporlar", "test_musteri_yollar_404_ve_baska_musteri");
    let (app, o, n, c) = kur_musteri();
    let (n2, _) = o.veri.musteri_ekle("", None).unwrap();
    rapor_yaz(&o, n2, "ay-2026-09.json", &rr::ay_raporu(&[]));
    rapor_yaz(&o, n2, "liste.json", &liste());
    assert_eq!(y::al(&app, "/rapor/ay/2026-09", &c).durum, 404); // yalnızca kendi klasörü
    assert!(!govde(&y::al(&app, "/raporlar", &c)).contains("2026-W38"));
    for yol in [
        "/rapor/ay/2026-13".to_string(),
        "/rapor/hafta/../x".to_string(),
        "/rapor/yil/2026".to_string(),
        "/rapor/hafta/2026-W54".to_string(),
        "/rapor/ay/".to_string(),
        format!("/rapor/ay/2026-09/../../{n2}"),
        "/rapor/hafta/2026-w38".to_string(),
    ] {
        let r = y::al(&app, &yol, &c);
        assert!(r.durum == 404 && govde(&r).contains("Bu dönem için rapor yok"), "{yol}");
    }
    rapor_yaz(&o, n, "ay-2026-09.json", &json!("{bozuk"));
    assert_eq!(y::al(&app, "/rapor/ay/2026-09", &c).durum, 404);
    assert_eq!(y::al(&app, "/raporlar", "").durum, 303); // giriş ister
}

/// Python'da `builtins.open` izlenir; Rust'ta dosya açma izlenemez: yalnızca `rapor-ham` klasörünün hiç oluşmadığı denetlenir
/// (okunan/yazılan dosya kümesini altın kayıt da karşılaştırır). Kalan fark: arşive/ham klasöre bakıp bakmadığı kodda (raporlar.rs
/// yalnızca `Raporlar.dizin/<numara>/` altını açar) görülür.
#[test]
fn sayfa_arsiv_ve_ham_okumaz() {
    let _k = Kayit::bas("test_raporlar", "test_sayfa_arsiv_ve_ham_okumaz");
    let (app, o, n, c) = kur_musteri();
    rapor_yaz(&o, n, "liste.json", &liste());
    rapor_yaz(&o, n, "ay-2026-09.json", &rr::ay_raporu(&[]));
    assert!(y::al(&app, "/raporlar", &c).durum == 200 && y::al(&app, "/rapor/ay/2026-09", &c).durum == 200);
    assert!(!o.dizin.join("rapor-ham").exists());
}

fn ep(iso: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(iso).unwrap().timestamp() as f64
}

#[test]
fn cihaz_bolumu_sorunlar_ve_cevrimdisi() {
    let _k = Kayit::bas("test_raporlar", "test_cihaz_bolumu_sorunlar_ve_cevrimdisi");
    let (app, o, n, c) = kur_musteri();
    let v = &o.veri;
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    let sorun = |tur: &str, metin: &str, onem: &str, acilis: &str, kapanis: Option<&str>, musteri: i64| {
        o.saat.koy(ep(acilis));
        let i = v.sorun_ac(musteri, tur, metin, onem).unwrap();
        if let Some(k) = kapanis {
            o.saat.koy(ep(k));
            v.sorun_kapat(i).unwrap();
        }
    };
    sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-08-25T10:00:00+03:00", Some("2026-08-26T10:00:00+03:00"), n);
    sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-08-31T22:00:00+03:00", Some("2026-09-01T02:00:00+03:00"), n);
    sorun("servis", "Servis durdu: <portal>", "kritik", "2026-09-10T08:00:00+03:00", Some("2026-09-10T09:30:00+03:00"), n);
    sorun("lisans", "Lisans bitiyor", "bilgi", "2026-09-12T08:00:00+03:00", None, n);
    sorun("disk", "Disk dolu", "uyari", "2026-09-15T08:00:00+03:00", Some("2026-09-15T08:20:00+03:00"), n2);
    sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-09-21T15:13:20+03:00", None, n);
    o.saat.koy(SAAT_BASLANGIC); // 21.09.2026 17:13:20
    let ar = v.sorunlar_aralik(n, "2026-09-01T00:00:00+03:00", "2026-10-01T00:00:00+03:00").unwrap();
    assert_eq!(ar.iter().map(|x| x.tur.as_str()).collect::<Vec<_>>(), ["ulasilamiyor", "servis", "lisans", "ulasilamiyor"]);
    rapor_yaz(&o, n, "ay-2026-09.json", &rr::ay_raporu(&[]));
    let s = govde(&y::al(&app, "/rapor/ay/2026-09", &c));
    let cihaz = &s[s.find("<h2>Cihaz</h2>").unwrap()..];
    assert!(cihaz.contains("Servis durdu: &lt;portal&gt;") && cihaz.contains("1 sa 30 dk"));
    assert!(!cihaz.contains("Lisans bitiyor") && !cihaz.contains("Disk dolu"));
    assert!(!cihaz.contains("31.08.2026")); // dönemden önce açılan listelenmez ama süresi sayılır
    assert!(cihaz.contains("açık") && cihaz.contains("en az 4 sa 0 dk"));
    assert_eq!(raporlar::cevrimdisi_sn(&ar, "2026-09-01", "2026-09-30", SAAT_BASLANGIC), 4 * 3600);
    // panel ve cihaz API'si aynı veriyi kullanır
    let vr = raporlar::rapor_verisi(v, &app.taban().raporlar, n, "ay", "2026-09", SAAT_BASLANGIC).unwrap().unwrap();
    assert_eq!(vr.cihaz.cevrimdisi_sn, 4 * 3600);
    assert_eq!(vr.isletme, raporlar::Isletme { numara: n.to_string(), ad: format!("Müşteri {n}"), unvan: String::new() });
    let ozet: Vec<_> = vr.cihaz.sorunlar.iter().map(|x| (x.tur.as_str(), x.onem.as_str(), x.kapanis.is_none(), x.sure_sn)).collect();
    assert_eq!(ozet, [("servis", "kritik", false, Some(5400)), ("ulasilamiyor", "kritik", true, Some(7200))]);
    assert_eq!(
        vr.cihaz.sorunlar[0],
        raporlar::CihazSorun {
            tur: "servis".into(),
            onem: "kritik".into(),
            metin: "Servis durdu: <portal>".into(),
            acilis: "2026-09-10T08:00:00+03:00".into(),
            kapanis: Some("2026-09-10T09:30:00+03:00".into()),
            sure_sn: Some(5400),
        }
    );
    assert!(raporlar::rapor_verisi(v, &app.taban().raporlar, n, "ay", "2026-13", SAAT_BASLANGIC).unwrap().is_none());
    assert!(raporlar::rapor_verisi(v, &app.taban().raporlar, n, "ay", "2026-08", SAAT_BASLANGIC).unwrap().is_none());
    assert_eq!(raporlar::cevrimdisi_sn(&ar, "2026-09-14", "2026-09-20", SAAT_BASLANGIC), 0);
}

/// rapor.py çıktısı → sayfa; kişisel veri yok.
#[test]
fn uctan_uca_kisisel_veri_yok() {
    let _k = Kayit::bas("test_raporlar", "test_uctan_uca_kisisel_veri_yok");
    let (app, o, n, c) = kur_musteri();
    let k = o.dizin.join("a");
    for g in ["2026-09-05", "2026-09-06", "2026-09-07"] {
        rt::zengin_gun(&k, g);
    }
    rt::misafir_gun_dns(&k, "2026-09-08", &[rt::TEL1, rt::TEL2, rt::TEL3], "www.ornek.com.tr");
    rt::muhurle(&k, &["2026-09-05", "2026-09-06", "2026-09-07", "2026-09-08"]);
    let hatalar = crate::rapor::calistir(
        &rt::arsivler(vec![(&n.to_string(), rt::ars(&[&k]))]),
        &o.dizin.join("yok.json"),
        &o.dizin.join("ham"),
        &o.dizin.join("rapor"),
        None,
        Some(rt::ts("2026-09-21", 12)),
        Some(&rt::SIR),
    )
    .unwrap();
    assert!(hatalar.is_empty(), "{hatalar:?}");
    let mut s = govde(&y::al(&app, "/raporlar", &c)) + &govde(&y::al(&app, "/rapor/ay/2026-09", &c));
    s += &govde(&y::al(&app, "/rapor/hafta/2026-W36", &c));
    // ilk arşiv günü 05.09: 1–4 Eylül hizmet öncesi, sayılmaz (16 gün = 05–20 Eylül)
    assert!(!s.contains("ornek") && s.contains("16 günün 4'ü tam") && s.contains("Hizmet öncesi"));
    assert!(s.contains("Kayıt gelmedi")); // 09–19 Eylül: hizmet içinde, kayıt yok
    let mut gizli: Vec<String> = [rt::TEL1, rt::TEL2, rt::TEL3, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", rt::MAC_MISAFIR, rt::MAC_IZINLI, "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "csv", "kullanicilar"]
        .iter()
        .map(|x| x.to_string())
        .collect();
    let mac_izinli = format!("mac:{}", rt::MAC_IZINLI);
    gizli.extend([rt::TEL1, rt::TC, rt::PAS, mac_izinli.as_str()].iter().map(|x| crate::rapor::takma(&rt::SIR, x)));
    for x in gizli {
        assert!(!s.contains(&x), "{x}");
    }
}

// ======================================================================================================================
// test_bekleme.py (müşteri /baglan akışı)
// ======================================================================================================================
const K1: &str = at::K1;
const S1: &str = at::S1;
const K2: &str = at::K2;
const S2: &str = at::S2;
const IP1: &str = at::IP1;
const IP2: &str = at::IP2;
const MAC0: &str = "00:0e:c4:ce:a0:9b";
const PW1: &str = "musteri-parola-1";
const PW2: &str = "musteri-parola-2";

/// Python `kur(tmp, sonuc)` (test_bekleme): api + Musteri aynı ortamda.
fn kur_bekleme(sonuc: Option<Value>) -> (at::D, Musteri) {
    let d = at::kur(sonuc);
    let k = Kayitlar::new(d.tmp.join("ist"), d.tmp.join("detay"), d.tmp.join("kisi_sir"));
    let m = Musteri::new(d.v.clone(), k, d.saat.saat());
    (d, m)
}

fn bildir_ip(d: &at::D, ip: &str) -> (u16, Value) {
    at::bildir_tam(d, K1, S1, WG, ip, MAC0, "0.4.13")
}

fn bildir_k(d: &at::D, k: &str, s: &str, wg: &str, ip: &str, mac: &str) -> (u16, Value) {
    at::bildir_tam(d, k, s, wg, ip, mac, "0.4.13")
}

fn musteri_gir(m: &Musteri, n: i64, pw: &str, ip: &str) -> (Yanit, String) {
    let r = y::giris_ip(m, &n.to_string(), pw, ip);
    let c = y::cerez(&r);
    (r, c)
}

fn belirtec(j: &Value) -> String {
    j["belirtec"].as_str().unwrap().to_string()
}

fn musteri_eki(d: &at::D, pw: &str) -> i64 {
    d.v.musteri_ekle("", Some(pw)).unwrap().0
}

#[test]
fn tek_aday_kendiliginden_baglanir_uctan_uca() {
    let _k = Kayit::bas("test_bekleme", "test_tek_aday_kendiliginden_baglanir_uctan_uca");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let v = &d.v;
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    let (r, c) = musteri_gir(&m, n, PW1, IP1);
    assert_eq!(r.baslik("Location"), Some("/baglan"));
    let s = y::al_ip(&m, "/baglan", &c, IP1);
    assert!(s.durum == 200 && govde(&s).contains("bağlanıyor") && govde(&s).contains("http-equiv=\"refresh\""));
    assert_eq!(olaylar(v, n)[0], "CIHAZ_OTOMATIK_AYRILDI");
    assert_eq!(v.bekleyenler().unwrap()[0].musteri, Some(n));
    d.saat.ilerlet(120.0);
    let (dk, j) = bildir_ip(&d, IP1);
    assert!(dk == 200 && j["durum"] == "baglan" && belirtec(&j).len() > 30);
    let (dk, g) = at::bagla_tam(&d, &belirtec(&j), K1, S1, WG, IP1);
    assert!(dk == 200 && g["numara"] == n.to_string() && g["tunel_ip"] == "10.99.0.10" && g["yedek_hedefi"] == format!("wfc-{n}@10.99.0.1:"));
    let mu = v.musteri(n).unwrap().unwrap();
    assert!(g["cihaz_anahtari"].as_str().unwrap().len() > 30 && g["tuz"] == mu.tuz.as_str() && g["ozet"] == mu.ozet.as_str());
    assert_eq!(d.q.isler(), vec![("cihaz-ekle".to_string(), json!({"numara": n.to_string(), "wg_pub": WG, "ssh_pub": SSH}))]);
    let cz = v.bagli_cihaz(n).unwrap().unwrap();
    assert!(cz.tunel_ip == "10.99.0.10" && cz.wg_pub == WG);
    let h = v.hareketler(Some(n), 300, Some("CIHAZ_BAGLANDI")).unwrap();
    assert!(h.len() == 1 && h[0].ayrinti.contains("otomatik"));
    assert_eq!(at::kod(&at::bagla_tam(&d, &belirtec(&j), K1, S1, WG, IP1).1), "taninmadi"); // belirteç tek kullanımlık
    // bağlandıktan hemen sonra panel hazırlanıyor; 30 sn sonra cihaz paneline
    let s = y::al_ip(&m, "/baglan", &c, IP1);
    assert!(s.durum == 200 && govde(&s).contains("hazırlanıyor"));
    d.saat.ilerlet(31.0);
    assert_eq!(y::al_ip(&m, "/baglan", &c, IP1).baslik("Location"), Some("/cihaz"));
    assert!(y::al_ip(&m, "/cihaz", &c, IP1).baslik("Location").unwrap().starts_with("https://cihaz.wificorrect.com/_giris?t="));
}

#[test]
fn cevabi_kaybeden_cihaz_yeniden_baglan_alir() {
    let _k = Kayit::bas("test_bekleme", "test_cevabi_kaybeden_cihaz_yeniden_baglan_alir");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    d.saat.ilerlet(120.0);
    let j = bildir_ip(&d, IP1).1;
    assert_eq!(at::bagla(&d, &belirtec(&j)).0, 200);
    d.saat.ilerlet(120.0); // cihaz cevabı alamadı, yine bildiriyor
    let (dk, j) = bildir_ip(&d, IP1);
    assert!(dk == 200 && j["durum"] == "baglan");
    let (dk, g) = at::bagla(&d, &belirtec(&j));
    assert!(dk == 200 && g["numara"] == n.to_string());
    // başka wg anahtarıyla gelen (cihaz anahtarını değiştirmiş) aynı kimlik bağa dokunamaz
    d.saat.ilerlet(120.0);
    assert_eq!(bildir_k(&d, K1, S1, at::WG2, IP1, MAC0).1, json!({"durum": "bekle"}));
    assert_eq!(d.v.bagli_cihaz(n).unwrap().unwrap().wg_pub, WG);
}

#[test]
fn hic_aday_yoksa_bulunamadi_baska_ipden_bulunmaz() {
    let _k = Kayit::bas("test_bekleme", "test_hic_aday_yoksa_bulunamadi_baska_ipden_bulunmaz");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP2);
    let c = musteri_gir(&m, n, PW1, IP1).1;
    let s = y::al_ip(&m, "/baglan", &c, IP1);
    assert!(s.durum == 200 && govde(&s).contains("bulunamadı") && govde(&s).contains("mobil veri"));
    assert_eq!(d.v.bekleyenler().unwrap()[0].musteri, None);
    // aynı IP ama 10 dk'dır bildirmeyen cihaz aday değil
    bildir_k(&d, K2, S2, WG, IP1, MAC0);
    d.saat.ilerlet(601.0);
    assert!(govde(&y::al_ip(&m, "/baglan", &c, IP1)).contains("bulunamadı"));
    bildir_k(&d, K2, S2, WG, IP1, MAC0);
    assert!(govde(&y::al_ip(&m, "/baglan", &c, IP1)).contains("bağlanıyor"));
}

#[test]
fn birden_cok_aday_liste_ve_secim() {
    let _k = Kayit::bas("test_bekleme", "test_birden_cok_aday_liste_ve_secim");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_k(&d, K1, S1, WG, IP1, "00:0e:c4:ce:a0:9b");
    bildir_k(&d, K2, S2, at::WG2, IP1, "00:0e:c4:11:22:33");
    let c = musteri_gir(&m, n, PW1, IP1).1;
    let s = y::al(&m, "/baglan", &c);
    let h = govde(&s);
    assert!(s.durum == 200 && h.contains("A0:9B") && h.contains("22:33") && !h.to_lowercase().contains("00:0e:c4")); // yalnızca son 4 hane
    assert!(d.v.bekleyenler().unwrap().iter().all(|r| r.musteri.is_none())); // kendiliğinden ayrılmaz
    assert_eq!(y::gonder(&m, "/baglan", &c, &[("csrf", "yanlis"), ("kimlik", K2)]).durum, 403);
    assert_eq!(y::gonder_ip(&m, "/baglan", &c, &[("csrf", &y::csrf(&s)), ("kimlik", K2)], IP2).durum, 400); // başka IP'den seçilemez
    let r = y::gonder(&m, "/baglan", &c, &[("csrf", &y::csrf(&s)), ("kimlik", K2)]);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/baglan"));
    let atamalar: BTreeMap<String, Option<i64>> = d.v.bekleyenler().unwrap().into_iter().map(|r| (r.kimlik, r.musteri)).collect();
    assert_eq!(atamalar, BTreeMap::from([(K1.to_string(), None), (K2.to_string(), Some(n))]));
    let h = d.v.hareketler(Some(n), 300, Some("CIHAZ_OTOMATIK_AYRILDI")).unwrap();
    assert!(h.len() == 1 && h[0].ayrinti.contains("secim=liste"));
    assert_eq!(bildir_ip(&d, IP1).1, json!({"durum": "bekle"}));
    assert_eq!(bildir_k(&d, K2, S2, at::WG2, IP1, MAC0).1["durum"], "baglan");
}

#[test]
fn ayrilan_15_dk_baglanmazsa_serbest_ve_baska_musteri_kapamaz() {
    let _k = Kayit::bas("test_bekleme", "test_ayrilan_15_dk_baglanmazsa_serbest_ve_baska_musteri_kapamaz");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    let n2 = musteri_eki(&d, PW2);
    bildir_ip(&d, IP1);
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    let c2 = musteri_gir(&m, n2, PW2, IP1).1;
    assert!(govde(&y::al(&m, "/baglan", &c2)).contains("bulunamadı")); // n'e ayrılmış: n2 göremez
    d.saat.ilerlet(901.0);
    bildir_ip(&d, IP1); // süre doldu: bekle (ayrılma düşer)
    assert_eq!(d.v.bekleyenler().unwrap()[0].musteri, None);
    assert!(govde(&y::al(&m, "/baglan", &c2)).contains("bağlanıyor"));
    assert_eq!(d.v.bekleyenler().unwrap()[0].musteri, Some(n2));
}

#[test]
fn belirtec_kurallari() {
    let _k = Kayit::bas("test_bekleme", "test_belirtec_kurallari");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    let t = belirtec(&bildir_ip(&d, IP1).1);
    assert_eq!(at::kod(&at::bagla_tam(&d, &t, K1, &"c".repeat(64), WG, IP1).1), "taninmadi"); // sırrı bilmeyen
    assert_eq!(at::kod(&at::bagla(&d, &"x".repeat(43)).1), "taninmadi");
    assert_eq!(at::kod(&at::bagla_tam(&d, &t, K2, S1, WG, IP1).1), "taninmadi");
    let (_, j) = at::post(&d, "/api/bagla", &json!({"kimlik": K1, "sir": S1, "belirtec": t, "wg_pub": "kotu", "ssh_pub": SSH}));
    assert_eq!(at::kod(&j), "gecersiz");
    d.saat.ilerlet(601.0); // belirteç 10 dk
    assert_eq!(at::kod(&at::bagla(&d, &t).1), "taninmadi");
    assert!(d.q.isler().is_empty() && d.v.bagli_cihaz(n).unwrap().is_none());
}

#[test]
fn bagli_cihazi_olan_musteri_ve_bagli_cihaz_kaydi() {
    let _k = Kayit::bas("test_bekleme", "test_bagli_cihazi_olan_musteri_ve_bagli_cihaz_kaydi");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    let n2 = musteri_eki(&d, PW2);
    d.v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap(); // yerel sayfadan bağlanmış cihaz (WG), bağ dosyasını kaybetmiş, bildiriyor
    bildir_ip(&d, IP1);
    assert!(govde(&y::al(&m, "/baglan", &musteri_gir(&m, n2, PW2, IP1).1)).contains("bulunamadı")); // bağlı cihaz aday değil
    bildir_k(&d, K2, S2, at::WG2, IP1, MAC0);
    let (r, c) = musteri_gir(&m, n, PW1, IP1);
    assert_eq!(r.baslik("Location"), Some("/")); // cihazı var (tünelsiz): bağlama yok
    assert_eq!(y::al(&m, "/baglan", &c).baslik("Location"), Some("/cihaz"));
    assert!(d.v.bekleyenler().unwrap().iter().all(|x| x.musteri != Some(n)));
}

#[test]
fn ayrilmis_ama_musteri_baska_cihaz_baglarsa_bekle() {
    let _k = Kayit::bas("test_bekleme", "test_ayrilmis_ama_musteri_baska_cihaz_baglarsa_bekle");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    let t = belirtec(&bildir_ip(&d, IP1).1);
    d.v.cihaz_bagla(n, at::WG2, SSH, "", "", "").unwrap(); // bu arada müşteri başka cihazı yerelden bağladı
    let (dk, j) = at::bagla(&d, &t);
    assert!(dk == 409 && at::kod(&j) == "baska_cihaz" && d.q.isler().is_empty());
    assert_eq!(bildir_ip(&d, IP1).1, json!({"durum": "bekle"}));
}

#[test]
fn uyeligi_biten_musteri_baglanmaz() {
    let _k = Kayit::bas("test_bekleme", "test_uyeligi_biten_musteri_baglanmaz");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    d.v.uyelik(n, "bitti").unwrap();
    bildir_ip(&d, IP1);
    let (r, c) = musteri_gir(&m, n, PW1, IP1);
    assert_eq!(r.baslik("Location"), Some("/"));
    assert!(govde(&y::al(&m, "/baglan", &c)).contains("sona ermiş"));
    assert_eq!(d.v.bekleyenler().unwrap()[0].musteri, None);
}

#[test]
fn cihaz_sitesinden_gelen_oturumlu_musteri_giris_sonrasina() {
    let _k = Kayit::bas("test_bekleme", "test_cihaz_sitesinden_gelen_oturumlu_musteri_giris_sonrasina");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    let c = musteri_gir(&m, n, PW1, IP1).1;
    // cihaz.wificorrect.com oturumsuz → panel girişi; oturum varsa doğrudan bağlama/cihaz adımına
    assert_eq!(y::al(&m, "/giris", &c).baslik("Location"), Some("/baglan"));
    assert_eq!(y::al(&m, "/cihaz", &c).baslik("Location"), Some("/baglan"));
    d.v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    let id = d.v.bagli_cihaz(n).unwrap().unwrap().id;
    d.v.cihaz_guncelle(id, &[("tunel_ip", "10.99.0.11")]).unwrap();
    assert_eq!(y::al(&m, "/giris", &c).baslik("Location"), Some("/cihaz"));
}

/// Tünel/yedek kaydı (kuyruk) bir kez başarısız olsa da cihaz sonraki bildirimde yeniden bağlan alır; müşteri "ulaşılamıyor"a
/// düşmez, "bağlanıyor" görür.
#[test]
fn kuyruk_hatasinda_cihaz_takilmaz() {
    let _k = Kayit::bas("test_bekleme", "test_kuyruk_hatasinda_cihaz_takilmaz");
    let (d, m) = kur_bekleme(Some(json!({"durum": "hata", "hata": "wg"})));
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    let c = musteri_gir(&m, n, PW1, IP1).1;
    y::al(&m, "/baglan", &c);
    d.saat.ilerlet(120.0);
    let (dk, j) = at::bagla(&d, &belirtec(&bildir_ip(&d, IP1).1));
    assert!(dk == 503 && at::kod(&j) == "kayit");
    assert!(olaylar(&d.v, n).contains(&"CIHAZ_KAYIT_HATA".to_string()));
    d.saat.ilerlet(31.0);
    let s = y::al(&m, "/baglan", &c);
    assert!(s.durum == 200 && govde(&s).contains("bağlanıyor")); // tünel yok: cihaz paneline gönderilmez
    d.q.sonuc_koy(None); // kuyruk zaman aşımı: yine kurtulur
    d.saat.ilerlet(120.0);
    let (dk, j) = bildir_ip(&d, IP1);
    assert!(dk == 200 && j["durum"] == "baglan");
    assert_eq!(at::bagla(&d, &belirtec(&j)).0, 503);
    d.q.sonuc_koy(Some(at::tamam()));
    d.saat.ilerlet(120.0);
    let (dk, j) = bildir_ip(&d, IP1);
    assert!(dk == 200 && j["durum"] == "baglan");
    let (dk, g) = at::bagla(&d, &belirtec(&j));
    assert!(dk == 200 && g["numara"] == n.to_string() && d.v.bagli_cihaz(n).unwrap().unwrap().tunel_ip == "10.99.0.10");
    d.saat.ilerlet(31.0);
    assert_eq!(y::al(&m, "/baglan", &c).baslik("Location"), Some("/cihaz"));
}

/// Kolay bağlamada (hesapsız, aynı IP) cevap yalnızca 0.4.13+ cihazın ihtiyacı olanı taşır: ortak NetGSM bilgisi ne bağlamada
/// ne sonraki eşitlemede gider. Parolalı `/api/giris` de ortak sırları taşımaz (spec bilinçli fark 2: yalnızca `etkin_mock`);
/// Python bu cevapta `sms.netgsm.password` taşıyordu, altın kayıt bu yüzden karşılaştırılamaz.
#[test]
fn bagla_cevabi_ortak_sms_sirlarini_tasimaz() {
    let _k = Kayit::altinsiz("spec bilinçli fark 2: /api/giris cevabının sms alanı yalnızca etkin_mock taşır");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let v = &d.v;
    v.sms_ayari_koy(&json!({"mock": false, "usercode": "8500000000", "password": "netgsm-gizli", "msgheader": "BASLIK"})).unwrap();
    v.admin_parolasi_koy("admin-parola-123").unwrap();
    let n = musteri_eki(&d, PW1);
    bildir_ip(&d, IP1);
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    let (dk, g) = at::bagla(&d, &belirtec(&bildir_ip(&d, IP1).1));
    let s = g.to_string();
    assert!(dk == 200 && !s.contains("netgsm-gizli") && !s.contains("8500000000"));
    assert_eq!(g["sms"], json!({"etkin_mock": false}));
    let mut admin: Vec<&String> = g["admin"].as_object().unwrap().keys().collect();
    admin.sort();
    assert_eq!(admin, ["ozet", "tuz", "yineleme"]);
    let (dk, e) = at::post(&d, "/api/eslesme", &json!({"cihaz_anahtari": g["cihaz_anahtari"], "surum": "0.4.7"}));
    assert!(dk == 200 && !e.to_string().contains("netgsm-gizli") && e["sms"] == json!({"etkin_mock": false}));
    // müşteriye özel AP yönetim parolası (ortak sır değil) bu cihaza da gider: cihaz panelinde gösterilir
    let ap = v.musteri(n).unwrap().unwrap().ap_parola;
    assert!(g["ap_parola"] == e["ap_parola"] && g["ap_parola"] == ap.as_str());
    // parolalı giriş (eski cihazlar): ortak NetGSM bilgisi yok, yalnızca deneme modu bayrağı
    let n2 = musteri_eki(&d, PW2);
    let (dk, g2) = at::post(&d, "/api/giris", &json!({"numara": n2.to_string(), "parola": PW2, "wg_pub": at::WG2, "ssh_pub": SSH}));
    assert!(dk == 200 && g2["sms"] == json!({"etkin_mock": false}) && !g2.to_string().contains("netgsm-gizli"));
    let (_, e2) = at::post(&d, "/api/eslesme", &json!({"cihaz_anahtari": g2["cihaz_anahtari"]}));
    assert!(e2["sms"] == json!({"etkin_mock": false}) && !e2.to_string().contains("netgsm-gizli"));
}

#[test]
fn tavan_doluysa_bayat_kayit_yer_acar() {
    let _k = Kayit::bas("test_bekleme", "test_tavan_doluysa_bayat_kayit_yer_acar");
    // Python V.BEKLEYEN_EN_COK = 3 (test iş parçacığında tavan düşürülür)
    let _s = crate::veri::bekleyen_sinirini_dusur(3);
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    bildir_k(&d, &"3".repeat(32), S1, WG, IP1, MAC0); // en eski; müşteriye ayrılacak, silinmemeli
    y::al(&m, "/baglan", &musteri_gir(&m, n, PW1, IP1).1);
    d.saat.ilerlet(1.0);
    bildir_k(&d, &"4".repeat(32), S1, WG, IP2, MAC0);
    d.saat.ilerlet(1.0);
    bildir_k(&d, &"5".repeat(32), S1, WG, IP2, MAC0);
    assert_eq!(at::kod(&bildir_k(&d, &"6".repeat(32), S1, WG, IP2, MAC0).1), "dolu"); // hepsi taze: yer yok
    d.saat.ilerlet(601.0); // 10 dk'dır bildirmeyen, kimseye ayrılmamış en eski kayıt yer açar
    assert_eq!(bildir_k(&d, &"6".repeat(32), S1, WG, IP2, MAC0), (200, json!({"durum": "bekle"})));
    let mut kimlikler: Vec<String> = d.v.bekleyenler().unwrap().into_iter().map(|r| r.kimlik).collect();
    kimlikler.sort();
    assert_eq!(kimlikler, ["3".repeat(32), "5".repeat(32), "6".repeat(32)]);
}

#[test]
fn silinmis_musteri_baglan_ve_cihaz_hata_vermez() {
    let _k = Kayit::bas("test_bekleme", "test_silinmis_musteri_baglan_ve_cihaz_hata_vermez");
    let (d, m) = kur_bekleme(Some(at::tamam()));
    let n = musteri_eki(&d, PW1);
    let ot = Ot {
        oturum: Oturum {
            user: n.to_string(),
            role: "musteri".into(),
            csrf: "x".into(),
            created: 0.0,
            last: 0.0,
            gerekce: None,
            surum: String::new(),
            ek: BTreeMap::new(),
        },
        yol: String::new(),
    };
    d.v.kilit().execute("DELETE FROM musteri WHERE numara = ?", [n]).unwrap();
    for yol in ["/baglan", "/cihaz"] {
        let r = m.sayfalar(&ot, "t", "GET", yol, "", &HashMap::new(), IP1).unwrap().expect(yol);
        assert!(r.durum == 200 || r.durum == 303, "{yol}");
    }
    let _ = fs::metadata(&d.tmp);
}
