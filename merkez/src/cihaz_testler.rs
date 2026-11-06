//! cihaz.rs testleri: Python tests/test_cihaz.py (6). Ortak kurulum `ap_testler.rs`ta da kullanılır.
use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::altin::Kayit;
use crate::kayitlar::Kayitlar;
use crate::musteri::Musteri;
use crate::test_yardim::{self as y, Ortam};

pub(crate) const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
pub(crate) const WG2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
pub(crate) const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c";
pub(crate) const CIHAZ: (&str, &str) = ("Host", "cihaz.wificorrect.com");
pub(crate) const IP: &str = "203.0.113.5";
pub(crate) const PW1: &str = "musteri-parola-1";

/// Python `kur(tmp)` (test_cihaz ve test_ap): ortam, ortak geçiş belirteçleri, müşteri paneli, cihaz geçidi ve bağlı cihazlı
/// bir müşteri (tünel 10.99.0.11).
pub(crate) fn kur() -> (Ortam, Arc<Musteri>, Arc<Cihaz>, i64) {
    let o = y::ortam();
    let gecis = Arc::new(GecisBelirtecleri::new());
    let k = Kayitlar::new(o.ist.clone(), o.detay.clone(), o.kisi_sir.clone());
    let m = Arc::new(Musteri::new(o.veri.clone(), k, o.saat.saat()).gecis_ile(gecis.clone()));
    let c = Arc::new(Cihaz::new(o.veri.clone(), gecis, m.clone(), o.saat.saat()));
    let n = musteri_ac(&o, PW1, WG, "10.99.0.11");
    (o, m, c, n)
}

pub(crate) fn musteri_ac(o: &Ortam, pw: &str, wg: &str, tunel: &str) -> i64 {
    let (n, _) = o.veri.musteri_ekle("", Some(pw)).unwrap();
    o.veri.cihaz_bagla(n, wg, SSH, "", "", "").unwrap();
    let id = o.veri.bagli_cihaz(n).unwrap().unwrap().id;
    o.veri.cihaz_guncelle(id, &[("tunel_ip", tunel)]).unwrap();
    n
}

/// Python `cerez(yanit, ad)`: Set-Cookie'nin değeri (ad= sonrası, ilk `;`'a kadar).
fn cerez_degeri_of(r: &Yanit) -> String {
    r.baslik("Set-Cookie").unwrap().split(';').next().unwrap().split_once('=').unwrap().1.to_string()
}

fn yetki(c: &Cihaz, wcc: &str, ip: &str) -> Yanit {
    y::istek(c, "GET", "/cihaz-yetki", &[CIHAZ, ("Cookie", &format!("wcc={wcc}"))], "", "", ip)
}

#[test]
fn belirtec_tek_kullanim() {
    let _k = Kayit::bas("test_cihaz", "test_belirtec_tek_kullanim");
    let g = GecisBelirtecleri::new();
    let t = g.uret("1234567", 1000.0, "oz", "musteri");
    assert_eq!(g.tuket(&t, 1030.0), Some(("1234567".into(), "oz".into(), "musteri".into())));
    assert_eq!(g.tuket(&t, 1031.0), None);
    let t2 = g.uret("1234567", 1000.0, "", "musteri");
    assert_eq!(g.tuket(&t2, 1061.0), None);
    assert_eq!(g.tuket("yok", 1000.0), None);
}

#[test]
fn giris_cihaza_gecirir_ve_yetki_baslik_verir() {
    let _k = Kayit::bas("test_cihaz", "test_giris_cihaza_gecirir_ve_yetki_baslik_verir");
    let (_o, m, c, n) = kur();
    let r = y::giris(&*m, &n.to_string(), PW1);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/cihaz"));
    let cz = y::cerez(&r);
    let r = y::istek(&*m, "GET", "/cihaz", &[("Cookie", &cz)], "", "", IP);
    assert!(r.durum == 303 && r.baslik("Location").unwrap().starts_with("https://cihaz.wificorrect.com/_giris?t="));
    let t = r.baslik("Location").unwrap().split_once("t=").unwrap().1.to_string();
    let r = y::istek(&*c, "GET", "/_giris", &[CIHAZ], "", &format!("t={t}"), IP);
    assert!(r.durum == 303 && r.baslik("Location") == Some("/") && r.baslik("Set-Cookie").unwrap().contains("SameSite=Lax"));
    let wcc = cerez_degeri_of(&r);
    let r = yetki(&c, &wcc, IP);
    assert!(r.durum == 200 && r.baslik("X-WFC-Cihaz") == Some("10.99.0.11") && r.baslik("X-WFC-Kullanici") == Some(n.to_string().as_str()));
    // ikinci kez: giriş sayfasına
    assert_eq!(y::istek(&*c, "GET", "/_giris", &[CIHAZ], "", &format!("t={t}"), IP).durum, 303);
    let r = y::istek(&*c, "GET", "/cihaz-yetki", &[CIHAZ], "", "", IP);
    assert!(r.durum == 302 && r.baslik("Location") == Some("https://panel.wificorrect.com/giris"));
}

#[test]
fn belirtec_parola_degisirse_giris_sayfasina() {
    let _k = Kayit::bas("test_cihaz", "test_belirtec_parola_degisirse_giris_sayfasina");
    let (o, m, c, n) = kur();
    let r = y::istek(&*m, "GET", "/cihaz", &[("Cookie", &y::cerez(&y::giris(&*m, &n.to_string(), PW1)))], "", "", IP);
    let t = r.baslik("Location").unwrap().split_once("t=").unwrap().1.to_string();
    o.veri.parola_koy(n, "yeni-parola-12").unwrap(); // belirteç alındıktan sonra
    let r = y::istek(&*c, "GET", "/_giris", &[CIHAZ], "", &format!("t={t}"), IP);
    assert!(r.durum == 303 && r.baslik("Location") == Some("https://panel.wificorrect.com/giris") && r.baslik("Set-Cookie").is_none());
}

#[test]
fn cihaz_yok_sorgusu_yonlendirmez() {
    let _k = Kayit::bas("test_cihaz", "test_cihaz_yok_sorgusu_yonlendirmez");
    let (_o, m, _c, n) = kur();
    let r = y::istek(&*m, "GET", "/cihaz", &[("Cookie", &y::cerez(&y::giris(&*m, &n.to_string(), PW1)))], "", "yok=1", IP);
    assert!(r.durum == 200 && r.baslik("Location").is_none() && r.govde_metin().contains("ulaşılamıyor"));
}

#[test]
fn cihaz_oturumu_parola_lisans() {
    let _k = Kayit::bas("test_cihaz", "test_cihaz_oturumu_parola_lisans");
    let (o, _m, c, n) = kur();
    let ozet = |o: &Ortam| o.veri.musteri(n).unwrap().unwrap().ozet;
    let wcc = c.oturumlar.create(&n.to_string(), "musteri", o.saat.t(), &ozet(&o), BTreeMap::new());
    assert_eq!(yetki(&c, &wcc, "1.2.3.4").durum, 200);
    o.veri.askiya_al(n, true).unwrap();
    assert_eq!(yetki(&c, &wcc, "1.2.3.4").durum, 302); // lisans kapalı
    o.veri.askiya_al(n, false).unwrap();
    o.veri.parola_koy(n, "yeni-parola-12").unwrap();
    assert_eq!(yetki(&c, &wcc, "1.2.3.4").durum, 302); // parola değişti: oturum düştü
    let wcc = c.oturumlar.create(&n.to_string(), "musteri", o.saat.t(), &ozet(&o), BTreeMap::new());
    o.veri.serbest_birak(n, true).unwrap();
    assert_eq!(yetki(&c, &wcc, "1.2.3.4").durum, 302); // cihaz bağlı değil
}

#[test]
fn cikis_iki_oturumu_kapatir_ve_cihazsiz_musteri_merkezde_kalir() {
    let _k = Kayit::bas("test_cihaz", "test_cikis_iki_oturumu_kapatir_ve_cihazsiz_musteri_merkezde_kalir");
    let (o, m, c, n) = kur();
    let cz = y::cerez(&y::giris(&*m, &n.to_string(), PW1));
    let ozet = o.veri.musteri(n).unwrap().unwrap().ozet;
    let wcc = c.oturumlar.create(&n.to_string(), "musteri", o.saat.t(), &ozet, BTreeMap::new());
    let r = y::istek(&*c, "GET", "/_cikis", &[CIHAZ, ("Cookie", &format!("wcc={wcc}"))], "", "", "1.2.3.4");
    assert!(r.durum == 303 && r.baslik("Location") == Some("https://panel.wificorrect.com/giris") && r.baslik("Set-Cookie").unwrap().contains("Max-Age=0"));
    assert_eq!(yetki(&c, &wcc, "1.2.3.4").durum, 302);
    // merkez oturumu da kapandı → giriş
    assert_eq!(y::istek(&*m, "GET", "/", &[("Cookie", &cz)], "", "", "1.2.3.4").durum, 303);
    let (n2, _) = o.veri.musteri_ekle("", Some("musteri-parola-2")).unwrap(); // cihazı yok
    let r = y::giris(&*m, &n2.to_string(), "musteri-parola-2");
    assert_eq!(r.baslik("Location"), Some("/baglan")); // kolay bağlama (spec 2026-10-16)
    assert_eq!(y::istek(&*m, "GET", "/cihaz", &[("Cookie", &y::cerez(&r))], "", "", "1.2.3.4").baslik("Location"), Some("/baglan"));
    o.veri.uyelik(n2, "bitti").unwrap(); // üyeliği biten cihazsız müşteri merkezde (yedek arşiv) kalır
    let r = y::giris(&*m, &n2.to_string(), "musteri-parola-2");
    assert_eq!(r.baslik("Location"), Some("/"));
    let s = y::istek(&*m, "GET", "/cihaz", &[("Cookie", &y::cerez(&r))], "", "", "1.2.3.4").govde_metin();
    assert!(s.contains("ulaşılamıyor"));
}
