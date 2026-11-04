//! veri.rs testleri (Python tests/test_veri.py'nin karşılığı + Python'un oluşturduğu veritabanı, eşzamanlılık, gün sınırı).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::guvenlik;
use crate::ortak::tests::gecici_dizin;

const WG1: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const WG2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test@cihaz";
const SAAT: f64 = 1_790_000_000.0; // 2026-09-21

fn sabit_saat() -> Saat {
    Arc::new(|| SAAT)
}

fn yeni() -> (Veri, PathBuf) {
    let d = gecici_dizin();
    (Veri::ac(&d.join("m.db"), sabit_saat()).unwrap(), d)
}

fn deger(r: Result<(), Hata>) -> bool {
    matches!(r, Err(Hata::Deger(_)))
}

fn gocmus_kopya() -> PathBuf {
    let d = gecici_dizin();
    let kaynak = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/veri/python-gocmus.db");
    std::fs::copy(&kaynak, d.join("m.db")).unwrap();
    d.join("m.db")
}

#[test]
fn musteri_numara_sirasi_ve_parola() {
    let _k = Kayit::bas("test_veri", "test_musteri_numara_sirasi_ve_parola");
    let (v, _d) = yeni();
    let (n1, p1) = v.musteri_ekle("Bocafe, Göztepe", None).unwrap();
    let mut numaralar = vec![n1];
    for _ in 0..30 {
        numaralar.push(v.musteri_ekle("", None).unwrap().0);
    }
    assert!(numaralar.iter().all(|n| (1_000_000..=9_999_999).contains(n)));
    let tek: std::collections::HashSet<_> = numaralar.iter().collect();
    assert!(tek.len() == 31 && p1.len() == 12);
    let ardisik: Vec<i64> = (n1..n1 + 31).collect();
    assert!(numaralar != ardisik); // sıralı değil, rastgele
    assert_eq!(v.parola_dogrula(n1, &p1).unwrap().unwrap().not_, "Bocafe, Göztepe");
    assert!(v.parola_dogrula(n1, "yanlis").unwrap().is_none() && v.parola_dogrula(1, &p1).unwrap().is_none());
    assert!(matches!(v.parola_koy(n1, "kisa"), Err(Hata::Deger(_))));
    v.parola_koy(n1, "yeni-parola-1").unwrap();
    assert!(v.parola_dogrula(n1, "yeni-parola-1").unwrap().is_some());
    // parola_sifirla taşınmadı: parola_uret + parola_koy aynı sırayla
    let p = guvenlik::parola_uret(12);
    v.parola_koy(n1, &p).unwrap();
    assert!(v.parola_dogrula(n1, &p).unwrap().is_some() && v.parola_dogrula(n1, "yeni-parola-1").unwrap().is_none());
}

#[test]
fn cihaz_bagla_tek_cihaz_ve_yeniden_deneme() {
    let _k = Kayit::bas("test_veri", "test_cihaz_bagla_tek_cihaz_ve_yeniden_deneme");
    let (v, _d) = yeni();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let (cid, a1) = v.cihaz_bagla(n, WG1, SSH, "Bocafe", "Bocafe Ltd.", "1.0").unwrap();
    assert_eq!(v.cihaz_anahtarla(&a1).unwrap().unwrap().id, cid);
    let (cid2, a2) = v.cihaz_bagla(n, WG1, SSH, "", "", "").unwrap(); // aynı cihaz yeniden denedi
    assert!(cid2 == cid && v.cihaz_anahtarla(&a1).unwrap().is_none() && v.cihaz_anahtarla(&a2).unwrap().unwrap().id == cid);
    assert!(matches!(v.cihaz_bagla(n, WG2, SSH, "", "", ""), Err(Hata::BaskaCihaz))); // başka cihaz
    let (m2, _) = v.musteri_ekle("", None).unwrap();
    assert!(matches!(v.cihaz_bagla(m2, WG1, SSH, "", "", ""), Err(Hata::BaskaCihaz))); // bu cihaz başka müşteriye bağlı
}

#[test]
fn serbest_birakma_akisi() {
    let _k = Kayit::bas("test_veri", "test_serbest_birakma_akisi");
    let (v, _d) = yeni();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let (cid, a) = v.cihaz_bagla(n, WG1, SSH, "", "", "").unwrap();
    assert_eq!(v.serbest_birak(n, false).unwrap().unwrap().id, cid);
    assert!(v.bagli_cihaz(n).unwrap().unwrap().durum == "serbest_birakiliyor" && v.cihaz_anahtarla(&a).unwrap().is_some());
    assert!(matches!(v.cihaz_bagla(n, WG2, SSH, "", "", ""), Err(Hata::BaskaCihaz))); // eski cihaz temizlenmeden yenisi bağlanmaz
    v.temizlendi(cid).unwrap();
    assert!(v.bagli_cihaz(n).unwrap().is_none() && v.cihaz_anahtarla(&a).unwrap().is_none());
    let (cid3, _) = v.cihaz_bagla(n, WG2, SSH, "", "", "").unwrap();
    assert!(v.serbest_birak(n, true).unwrap().unwrap().id == cid3 && v.bagli_cihaz(n).unwrap().is_none());
    let durumlar: Vec<String> = v.cihaz_gecmisi(n).unwrap().into_iter().map(|c| c.durum).collect();
    assert_eq!(durumlar, ["serbest", "serbest"]);
    assert!(v.serbest_birak(n, false).unwrap().is_none());
}

#[test]
fn serbest_iptal_bagliya_doner() {
    // iptal: serbest_birakiliyor → bagli
    let (v, _d) = yeni();
    let (m, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(m, "C".repeat(43).as_str(), SSH, "", "", "").unwrap();
    v.serbest_birak(m, false).unwrap();
    v.serbest_iptal(m).unwrap();
    assert_eq!(v.bagli_cihaz(m).unwrap().unwrap().durum, "bagli");
}

#[test]
fn uyelik_hareket_eski_arsiv() {
    let _k = Kayit::bas("test_veri", "test_uyelik_hareket_eski_arsiv");
    let (v, _d) = yeni();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.uyelik(n, "bitti").unwrap();
    let m = v.musteri(n).unwrap().unwrap();
    assert!(m.uyelik == "bitti" && !m.bitis.is_empty());
    assert!(deger(v.uyelik(n, "bozuk")));
    v.uyelik(n, "aktif").unwrap();
    assert_eq!(v.musteri(n).unwrap().unwrap().bitis, "");
    v.hareket("admin", "1.2.3.4", "MUSTERI_ACILDI", Some(n), &"x".repeat(1000)).unwrap();
    v.hareket("admin", "1.2.3.4", "GIRIS", None, "").unwrap();
    let olaylar: Vec<String> = v.hareketler(None, 300, None).unwrap().into_iter().map(|h| h.olay).collect();
    assert_eq!(olaylar, ["GIRIS", "MUSTERI_ACILDI"]);
    let h = v.hareketler(Some(n), 300, None).unwrap();
    assert!(h.len() == 1 && h[0].ayrinti.len() == 500 && h[0].musteri == Some(n));
    assert_eq!(v.hareketler(None, 300, Some("GIRIS")).unwrap().len(), 1);
    assert!(v.hareketler(None, 300, Some("GIRIS")).unwrap()[0].musteri.is_none());
    v.eski_arsiv_bagla("bocafe", Some(n)).unwrap();
    v.eski_arsiv_bagla("bocafe-test", None).unwrap();
    assert_eq!(v.eski_arsivler().unwrap(), [("bocafe".to_string(), Some(n)), ("bocafe-test".to_string(), None)]);
    assert_eq!(v.arsivler(n).unwrap(), [n.to_string(), "bocafe".to_string()]);
}

#[test]
fn eski_arsiv_bagi_degisir() {
    let (v, _d) = yeni();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.eski_arsiv_bagla("bocafe", Some(n)).unwrap();
    v.eski_arsiv_bagla("bocafe", None).unwrap(); // ON CONFLICT: bağ değişir
    assert_eq!(v.arsivler(n).unwrap(), [n.to_string()]);
}

#[test]
fn rapor_gruplari_yaz() {
    let _k = Kayit::bas("test_veri", "test_rapor_gruplari_yaz");
    let (v, d) = yeni();
    let (n1, _) = v.musteri_ekle("", None).unwrap();
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    let yol = d.join("rapor-gruplari.json");
    v.rapor_gruplari_yaz(&yol).unwrap();
    let oku = |y: &Path| -> Value { serde_json::from_str(&std::fs::read_to_string(y).unwrap()).unwrap() };
    assert_eq!(oku(&yol), json!({}));
    v.eski_arsiv_bagla("zkafe", Some(n1)).unwrap();
    v.eski_arsiv_bagla("akafe", Some(n1)).unwrap();
    v.eski_arsiv_bagla("bocafe", Some(n2)).unwrap();
    v.eski_arsiv_bagla("bagsiz", None).unwrap();
    v.rapor_gruplari_yaz(&yol).unwrap();
    assert_eq!(oku(&yol), json!({ n1.to_string(): ["akafe", "zkafe"], n2.to_string(): ["bocafe"] }));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&yol).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn kalici() {
    let _k = Kayit::bas("test_veri", "test_kalici");
    let d = gecici_dizin();
    let yol = d.join("m.db");
    let (n, p) = {
        let v = Veri::ac(&yol, sabit_saat()).unwrap();
        v.musteri_ekle("", None).unwrap()
    };
    assert!(Veri::ac(&yol, sabit_saat()).unwrap().parola_dogrula(n, &p).unwrap().is_some());
}

#[test]
fn parola_acik_ve_lisans() {
    let _k = Kayit::bas("test_veri", "test_parola_acik_ve_lisans");
    let (v, _d) = yeni(); // saat: 2026-09-21
    let (n, p) = v.musteri_ekle("", None).unwrap();
    let m = v.musteri(n).unwrap().unwrap();
    assert!(m.parola_acik == p && m.lisans_bitis == "2027-09-21" && m.askida == 0);
    v.parola_koy(n, "yeni-parola-1").unwrap();
    assert_eq!(v.musteri(n).unwrap().unwrap().parola_acik, "yeni-parola-1");
    let p2 = guvenlik::parola_uret(12);
    v.parola_koy(n, &p2).unwrap();
    assert_eq!(v.musteri(n).unwrap().unwrap().parola_acik, p2);
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()), ("aktif", "2027-09-21".to_string()));
    assert_eq!(v.lisans_uzat(n, LISANS_GUN).unwrap(), "2028-09-20"); // bitişin üstüne 1 yıl
    v.lisans_ayarla(n, "2026-09-01").unwrap();
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()), ("bitti", "2026-09-01".to_string()));
    assert_eq!(v.lisans_uzat(n, LISANS_GUN).unwrap(), "2027-09-21"); // süresi geçmişse bugünden 1 yıl
    v.askiya_al(n, true).unwrap();
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()).0, "askida");
    v.askiya_al(n, false).unwrap();
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()).0, "aktif");
    assert!(matches!(v.lisans_ayarla(n, "bozuk"), Err(Hata::Deger(_))));
}

#[test]
fn ap_yonetim_parolasi() {
    let _k = Kayit::bas("test_veri", "test_ap_yonetim_parolasi");
    let (v, _d) = yeni();
    let (n, pw) = v.musteri_ekle("", Some("musteri-parola-1")).unwrap();
    let ap = v.musteri(n).unwrap().unwrap().ap_parola;
    assert!(ap.len() == 12 && ap.chars().all(|c| guvenlik::HARFLER.contains(c)) && ap != pw);
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    let ap2 = v.musteri(n2).unwrap().unwrap().ap_parola;
    assert!(!ap2.is_empty() && ap2 != ap);
    let yeni_ap = v.ap_parolasi_yenile(n).unwrap();
    assert!(yeni_ap != ap && yeni_ap.len() == 12 && v.musteri(n).unwrap().unwrap().ap_parola == yeni_ap);
    v.parola_koy(n, "baska-parola-12").unwrap();
    assert_eq!(v.musteri(n).unwrap().unwrap().ap_parola, yeni_ap); // müşteri parolası değişince AP parolası değişmez
}

#[test]
fn silinen_veri_sayfalarda_kalmaz() {
    let _k = Kayit::bas("test_veri", "test_silinen_veri_sayfalarda_kalmaz");
    let (v, _d) = yeni();
    let c = v.kilit();
    let (sd, fk): (i64, i64) = (
        c.query_row("PRAGMA secure_delete", [], |r| r.get(0)).unwrap(),
        c.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap(),
    );
    assert_eq!((sd, fk), (1, 1)); // eski açık parolalar dosyada kalmasın
}

#[test]
fn tek_admin_parolasi_ve_belirlenen_musteri_parolasi() {
    let _k = Kayit::bas("test_veri", "test_tek_admin_parolasi_ve_belirlenen_musteri_parolasi");
    let (v, _d) = yeni();
    assert!(v.admin_parolasi().unwrap().is_none());
    for kotu in ["kisa", ""] {
        assert!(deger(v.admin_parolasi_koy(kotu)), "{kotu}");
    }
    v.admin_parolasi_koy("benim-admin-parolam").unwrap();
    let a = v.admin_parolasi().unwrap().unwrap();
    assert!(a.yineleme == 120_000 && guvenlik::dogru("benim-admin-parolam", &a.tuz, &a.ozet, a.yineleme));
    let ham: String = v.kilit().query_row("SELECT deger FROM ayar WHERE anahtar = 'admin'", [], |r| r.get(0)).unwrap();
    assert!(!ham.contains("benim-admin-parolam")); // yalnızca özet saklanır
    assert!(ham.contains("\"tuz\": ") && ham.contains(", \"ozet\": ")); // Python json.dumps biçimi
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG1, SSH, "", "", "").unwrap();
    assert!(v.admin_parolasi().unwrap().unwrap() == a && v.bagli_cihaz(n).unwrap().unwrap().admin_ozet.is_empty());
    let (n2, pw) = v.musteri_ekle("not", Some("musteri-parola-1")).unwrap();
    assert!(pw == "musteri-parola-1" && v.parola_dogrula(n2, "musteri-parola-1").unwrap().is_some());
    assert_eq!(v.musteri(n2).unwrap().unwrap().parola_acik, pw);
    assert!(matches!(v.musteri_ekle("", Some("kisa")), Err(Hata::Deger(_))));
}

#[test]
fn sms_ayari() {
    let _k = Kayit::bas("test_veri", "test_sms_ayari");
    let (v, _d) = yeni();
    assert!(v.sms_ayari().unwrap().is_none());
    let h = v.sms_ayari_koy(&json!({"mock": false, "usercode": "850", "password": "", "msgheader": "BASLIK", "appkey": ""}));
    assert!(matches!(&h, Err(Hata::Deger(m)) if m.contains("password")));
    v.sms_ayari_koy(&json!({"mock": true, "usercode": "", "password": "", "msgheader": "", "appkey": ""})).unwrap();
    assert_eq!(v.sms_ayari().unwrap().unwrap()["mock"], json!(true));
    v.sms_ayari_koy(&json!({"mock": false, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""})).unwrap();
    assert_eq!(
        v.sms_ayari().unwrap().unwrap(),
        json!({"mock": false, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": "",
               "tr_saglayici": "netgsm", "twilio": twilio_varsayilan()})
    );
    assert!(!v.sms_etkin_mock().unwrap());
    // Twilio'dan önce kaydedilmiş ayar satırı varsayılanlarla okunur
    v.kilit()
        .execute("UPDATE ayar SET deger = ? WHERE anahtar = 'sms'", [r#"{"mock": true, "usercode": "", "password": "", "msgheader": "", "appkey": ""}"#])
        .unwrap();
    let s = v.sms_ayari().unwrap().unwrap();
    assert!(s["tr_saglayici"] == "netgsm" && s["twilio"] == twilio_varsayilan());
    assert!(v.sms_etkin_mock().unwrap());
}

#[test]
fn sms_ayari_twilio_dogrulamasi() {
    let (v, _d) = yeni();
    let sid = |on: &str| format!("{on}{}", "0".repeat(32));
    let temel = |tw: Value, saglayici: &str| json!({"mock": true, "tr_saglayici": saglayici, "twilio": tw});
    // geçersiz SID
    let h = v.sms_ayari_koy(&temel(json!({"acik": false, "account_sid": "AC123"}), "netgsm"));
    assert!(matches!(&h, Err(Hata::Deger(m)) if m == "Twilio account_sid biçimi geçersiz."));
    // Türk numaraları için Twilio seçili ama kapalı
    let h = v.sms_ayari_koy(&temel(json!({"acik": false}), "twilio"));
    assert!(matches!(&h, Err(Hata::Deger(m)) if m.contains("Twilio açık olmalı")));
    assert!(deger(v.sms_ayari_koy(&temel(json!({}), "bozuk"))));
    // açık ama kimlik eksik
    let h = v.sms_ayari_koy(&temel(json!({"acik": true, "account_sid": sid("AC"), "verify_sid": sid("VA")}), "netgsm"));
    assert!(matches!(&h, Err(Hata::Deger(m)) if m.contains("auth_token")));
    let tam = json!({"acik": true, "account_sid": sid("AC"), "verify_sid": sid("VA"), "auth_token": "t"});
    v.sms_ayari_koy(&temel(tam, "twilio")).unwrap();
    assert!(v.sms_etkin_mock().unwrap()); // deneme modu açık
    v.sms_ayari_koy(&json!({"mock": false, "tr_saglayici": "twilio", "twilio": {"acik": true, "account_sid": sid("AC"),
        "verify_sid": sid("VA"), "auth_token": "t"}}))
        .unwrap();
    assert!(!v.sms_etkin_mock().unwrap());
    assert!(!sms_hazir(&json!({"tr_saglayici": "twilio", "twilio": {"acik": false}})));
    assert!(sms_hazir(&json!({"usercode": "a", "password": "b", "msgheader": "c"})));
    assert!(!sms_hazir(&json!({"usercode": "a", "password": "", "msgheader": "c"})));
}

#[test]
fn python_gocmus_veritabani_okunur() {
    let yol = gocmus_kopya();
    let v = Veri::ac(&yol, Arc::new(|| 1_790_000_000.0)).unwrap();
    sema_denetle(&v.kilit()).unwrap();
    let ms = v.musteriler().unwrap();
    assert_eq!(ms.iter().map(|m| m.numara).collect::<Vec<_>>(), [1000001, 1000002, 1000003]);
    let m1 = &ms[0];
    assert!(m1.not_ == "Deneme Kafe, Ornek" && m1.parola_acik == "Deneme-parola-1" && m1.guncelleme_kanali == "test");
    assert!(m1.lisans_bitis == "2027-01-15" && m1.ap_parola.len() == 12 && m1.askida == 0 && m1.uyelik == "aktif");
    assert!(ms[1].askida == 1 && ms[1].uyelik == "bitti" && ms[1].guncelleme_kanali == "kapali" && !ms[1].bitis.is_empty());
    assert!(ms[2].guncelleme_hemen == 1_790_000_000.0 && ms[2].guncelleme_hemen_surum == "0.4.28" && ms[2].guncelleme_kanali == "kararli");
    assert_eq!(v.lisans_durumu(&ms[1]), ("askida", "2027-02-01".to_string()));
    for m in &ms {
        assert!(v.musteri(m.numara).unwrap().is_some());
        v.cihaz_gecmisi(m.numara).unwrap();
        v.sorun_gecmisi(m.numara, 50).unwrap();
        v.arsivler(m.numara).unwrap();
        v.sms_son(m.numara).unwrap();
        v.bekleyen_musterinin(m.numara).unwrap();
    }
    let c1 = v.bagli_cihaz(1000001).unwrap().unwrap();
    assert!(c1.isletme_turu == "otel" && c1.adres == "Deneme Mah. 1" && c1.otomatik == 1 && c1.tunel_ip == "10.99.0.50");
    assert!(c1.admin_parola.is_empty() && c1.admin_yineleme == 0 && c1.ayrilma.is_empty());
    assert!(v.bagli_cihaz(1000002).unwrap().is_none());
    let gecmis = v.cihaz_gecmisi(1000002).unwrap();
    assert!(gecmis.len() == 1 && gecmis[0].durum == "serbest" && !gecmis[0].ayrilma.is_empty());
    assert_eq!(v.cihazlar().unwrap().len(), 2);
    assert_eq!(v.cihaz_anahtarla("yok").unwrap().map(|c| c.id), None);
    let hs = v.hareketler(None, 300, None).unwrap();
    assert!(hs.len() == 4 && hs[0].musteri.is_none() && hs[1].ayrinti == "Türkçe ayrıntı ğüşıöç" && hs[3].olay == "MUSTERI_ACILDI");
    assert_eq!(v.hareketler(Some(1000001), 300, None).unwrap().len(), 2);
    assert_eq!(v.eski_arsivler().unwrap(), [("bocafe-test".to_string(), None), ("deneme-eski".to_string(), Some(1000001))]);
    assert_eq!(v.rapor_gruplari().unwrap(), json!({"1000001": ["deneme-eski"]}));
    // ayar: JSON olmayan düz metin hedefler
    assert_eq!(v.kanal_hedef("kararli").unwrap().as_deref(), Some("0.4.28"));
    assert_eq!(v.kanal_hedef("test").unwrap().as_deref(), Some(EN_SON));
    assert_eq!(v.kanal_hedef("yok").unwrap(), None);
    assert_eq!(v.en_son_surum().unwrap().as_deref(), Some("0.4.28")); // 0.10.0 yerel: girmez
    assert_eq!(v.kanal_surumu("test").unwrap().as_deref(), Some("0.4.28"));
    let sr = v.surumler().unwrap();
    assert_eq!(sr.iter().map(|s| s.surum.as_str()).collect::<Vec<_>>(), ["0.10.0", "0.4.28", "0.4.9"]);
    assert!(sr[1].boyut == 2_000_000 && sr[1].kaynak == "github" && v.surum("0.4.9").unwrap().unwrap().sha256 == "a".repeat(64));
    let hedef = v.cihaz_hedefi(&ms[0]).unwrap().unwrap(); // kanal test → en-son
    assert!(hedef.surum == "0.4.28" && hedef.boyut == 2_000_000 && !hedef.hemen);
    assert!(v.cihaz_hedefi(&ms[1]).unwrap().is_none()); // kapalı + askıda
    let h3 = v.cihaz_hedefi(&ms[2]).unwrap().unwrap(); // kararlı → 0.4.28, hemen bayrağı aynı sürümle
    assert!(h3.surum == "0.4.28" && h3.hemen);
    let a = v.admin_parolasi().unwrap().unwrap();
    assert!(a.tuz == "atuz" && a.ozet == "aozet" && a.yineleme == 120_000);
    let s = v.sms_ayari().unwrap().unwrap();
    assert!(s["mock"] == json!(true) && s["tr_saglayici"] == "netgsm" && s["twilio"] == twilio_varsayilan());
    assert!(v.sms_etkin_mock().unwrap());
    let u = v.uyari_ayari().unwrap();
    assert!(u["telefonlar"] == json!(["905550000001"]) && u["acik"] == json!(true) && u["gunluk_sinir"] == json!(20));
    assert_eq!(v.saglik_son_tur().unwrap().as_deref(), Some("2026-11-03T20:00:00+03:00"));
    assert_eq!(v.sms_sayaci("2026-11-03").unwrap(), 7);
    assert_eq!(v.sms_sayac(1000001, "2026-11-03").unwrap(), 4);
    assert_eq!(v.sms_sayac(1000001, "2026-11-03/yabanci").unwrap(), 1);
    let son = v.sms_son(1000001).unwrap().unwrap();
    assert!(son.saglayici == "netgsm" && son.sonuc == "gonderildi" && son.kod == "0");
    let sg = v.saglik(1).unwrap().unwrap();
    assert!(sg.rapor["disk"] == json!(41) && sg.rapor["not"] == json!("ğ"));
    assert_eq!(v.acik_sorunlar(None).unwrap().len(), 1);
    let so = v.sorun_gecmisi(1000001, 50).unwrap();
    assert!(so.len() == 1 && so[0].tur == "disk" && so[0].acilis_sms == "gonderildi");
    assert!(v.son_kapanan(1000001, "disk").unwrap().is_some() && v.son_kapanan(1000003, "rapor").unwrap().is_none());
    assert_eq!(v.sorunlar_aralik(1000001, "2026-10-01T00:00:00+03:00", "2026-11-01T00:00:00+03:00").unwrap().len(), 1);
    assert_eq!(v.kapanis_bekleyenler(1000001).unwrap().len(), 1);
    let bs = v.bekleyenler().unwrap();
    assert_eq!(bs.iter().map(|b| b.kimlik.chars().next().unwrap()).collect::<Vec<_>>(), ['m', 'k']);
    assert!(bs[0].musteri == Some(1000003) && bs[0].cihaz == Some(3) && bs[0].baglandi == 1_790_000_660.5 && bs[1].musteri.is_none());
    assert!(v.bekleyen_var(&"k".repeat(32)).unwrap());
    assert_eq!(v.bekleyen_adaylar("203.0.113.9").unwrap().len(), 2);
    // yeni yazım eski dosyayla uyumlu mu: yazıp tekrar oku
    v.hareket("admin", "1.1.1.1", "DENEME", Some(1000001), "").unwrap();
    assert_eq!(v.hareketler(Some(1000001), 300, None).unwrap().len(), 3);
}

#[test]
fn eksik_sutun_sema_denetiminde_hata() {
    let yol = gocmus_kopya();
    {
        let db = rusqlite::Connection::open(&yol).unwrap();
        db.execute("ALTER TABLE musteri DROP COLUMN ap_parola", []).unwrap();
    }
    let h = Veri::ac(&yol, sabit_saat()).err().expect("eksik sütun hata vermeli");
    assert!(h.contains("ap_parola") && h.contains("musteri"), "{h}");
    let yol2 = gocmus_kopya();
    {
        let db = rusqlite::Connection::open(&yol2).unwrap();
        db.execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE sms_son;").unwrap();
    }
    // eksik tablo CREATE IF NOT EXISTS ile yeniden kurulur, denetim yine sütunları doğrular
    Veri::ac(&yol2, sabit_saat()).unwrap();
    let yol3 = gocmus_kopya();
    {
        let db = rusqlite::Connection::open(&yol3).unwrap();
        db.execute("ALTER TABLE sms_son RENAME COLUMN kod TO kod_eski", []).unwrap();
    }
    let h = Veri::ac(&yol3, sabit_saat()).err().unwrap();
    assert!(h.contains("sms_son") && h.contains("kod"), "{h}");
    // doğrudan işlev: tablo yoksa tablo adı geçer
    let bos = rusqlite::Connection::open_in_memory().unwrap();
    let h = sema_denetle(&bos).unwrap_err();
    assert!(h.contains("musteri"), "{h}");
}

#[test]
fn sema_sutun_listesi_taze_veritabaniyla_ayni() {
    let (v, _d) = yeni();
    let c = v.kilit();
    sema_denetle(&c).unwrap();
    for (tablo, sutunlar) in SUTUNLAR {
        let mut st = c.prepare(&format!("PRAGMA table_info({tablo})")).unwrap();
        let mut var: Vec<String> = st.query_map([], |r| r.get::<_, String>(1)).unwrap().map(|x| x.unwrap()).collect();
        let mut beklenen: Vec<String> = sutunlar.iter().map(|s| s.to_string()).collect();
        var.sort();
        beklenen.sort();
        assert_eq!(var, beklenen, "{tablo}");
    }
}

#[test]
fn eszamanli_iki_baglanti() {
    let d = gecici_dizin();
    let yol = d.join("m.db");
    let a = Arc::new(Veri::ac(&yol, sabit_saat()).unwrap());
    let b = Arc::new(Veri::ac(&yol, sabit_saat()).unwrap());
    // sırayla
    for i in 0..20 {
        a.hareket("a", "1", "SIRA", None, &i.to_string()).unwrap();
        b.hareket("b", "2", "SIRA", None, &i.to_string()).unwrap();
    }
    // aynı anda
    let isler: Vec<_> = [a.clone(), b.clone()]
        .into_iter()
        .map(|v| {
            std::thread::spawn(move || {
                for i in 0..40 {
                    v.hareket("t", "3", "AYNI_ANDA", None, &i.to_string()).unwrap();
                    v.sms_sayac_artir(1, "2026-11-04").unwrap();
                }
            })
        })
        .collect();
    for i in isler {
        i.join().unwrap();
    }
    assert_eq!(a.hareketler(None, 1000, Some("SIRA")).unwrap().len(), 40);
    assert_eq!(b.hareketler(None, 1000, Some("AYNI_ANDA")).unwrap().len(), 80);
    assert_eq!(a.sms_sayac(1, "2026-11-04").unwrap(), 80);
}

#[test]
fn bugun_tr_gun_siniri() {
    let t = Arc::new(AtomicU64::new(1_793_825_999.0f64.to_bits()));
    let t2 = t.clone();
    let d = gecici_dizin();
    let v = Veri::ac(&d.join("m.db"), Arc::new(move || f64::from_bits(t2.load(Ordering::SeqCst)))).unwrap();
    assert_eq!(v.bugun().to_string(), "2026-11-04"); // 23:59:59 +03:00
    let (n, _) = v.musteri_ekle("", None).unwrap();
    assert_eq!(v.musteri(n).unwrap().unwrap().lisans_bitis, "2027-11-04");
    v.lisans_ayarla(n, "2026-11-04").unwrap();
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()).0, "aktif");
    t.store(1_793_826_000.0f64.to_bits(), Ordering::SeqCst);
    assert_eq!(v.bugun().to_string(), "2026-11-05"); // 00:00:00 +03:00 (UTC'de hâlâ 4 Kasım 21:00)
    assert_eq!(v.lisans_durumu(&v.musteri(n).unwrap().unwrap()).0, "bitti");
    assert_eq!(v.lisans_uzat(n, 1).unwrap(), "2026-11-06");
}

#[test]
fn bekleyen_akisi() {
    let t = Arc::new(AtomicU64::new(SAAT.to_bits()));
    let t2 = t.clone();
    let d = gecici_dizin();
    let v = Veri::ac(&d.join("m.db"), Arc::new(move || f64::from_bits(t2.load(Ordering::SeqCst)))).unwrap();
    let (kimlik, sir, ip) = ("k".repeat(32), "s".repeat(64), "203.0.113.9");
    let r = v.bekleyen_bildir(&kimlik, &sir, ip, "0.4.28", "aa:bb", WG1, SSH).unwrap();
    assert_eq!(r, ("bekle", true, None));
    assert_eq!(v.bekleyen_bildir(&kimlik, &sir, ip, "0.4.28", "aa:bb", WG1, SSH).unwrap(), ("bekle", false, None));
    assert_eq!(v.bekleyen_bildir(&kimlik, "baska", ip, "0.4.28", "aa:bb", WG1, SSH).unwrap().0, "taninmadi");
    assert!(v.bekleyen_var(&kimlik).unwrap() && !v.bekleyen_var("yok").unwrap());
    assert_eq!(v.bekleyen_adaylar(ip).unwrap().len(), 1);
    assert!(v.bekleyen_adaylar("1.2.3.4").unwrap().is_empty());
    let (n, _) = v.musteri_ekle("", None).unwrap();
    assert!(!v.bekleyen_ayir(&kimlik, n, "1.2.3.4").unwrap()); // başka IP
    assert!(v.bekleyen_ayir(&kimlik, n, ip).unwrap());
    assert!(v.bekleyen_adaylar(ip).unwrap().is_empty()); // ayrıldı: aday değil
    assert_eq!(v.bekleyen_musterinin(n).unwrap().unwrap().atama, "ayrildi");
    let (durum, yeni_mi, bel) = v.bekleyen_bildir(&kimlik, &sir, ip, "0.4.28", "aa:bb", WG1, SSH).unwrap();
    assert!(durum == "baglan" && !yeni_mi);
    let bel = bel.unwrap();
    assert!(v.bekleyen_belirtec_tuket(&kimlik, &sir, "yanlis").unwrap().is_none()); // yanlış belirteç de tüketir
    assert!(v.bekleyen_belirtec_tuket(&kimlik, &sir, &bel).unwrap().is_none());
    let (_, _, bel) = v.bekleyen_bildir(&kimlik, &sir, ip, "0.4.28", "aa:bb", WG1, SSH).unwrap();
    let bel = bel.unwrap();
    assert_eq!(v.bekleyen_belirtec_tuket(&kimlik, &sir, &bel).unwrap().unwrap().musteri, Some(n));
    assert!(v.bekleyen_belirtec_tuket(&kimlik, &sir, &bel).unwrap().is_none()); // tek kullanımlık
    let (cid, _) = v.cihaz_bagla(n, WG1, SSH, "", "", "").unwrap();
    v.bekleyen_baglandi(&kimlik, cid).unwrap();
    assert_eq!(v.bekleyen_musterinin(n).unwrap().unwrap().atama, "bagli");
    assert_eq!(v.bekleyen_bildir(&kimlik, &sir, ip, "0.4.28", "aa:bb", WG1, SSH).unwrap().0, "baglan"); // cevap kaybolduysa yeniden
    // süresi geçen ayrılma düşer
    let (k2, ip2) = ("m".repeat(32), "198.51.100.1");
    v.bekleyen_bildir(&k2, &sir, ip2, "", "", WG2, SSH).unwrap();
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    assert!(v.bekleyen_ayir(&k2, n2, ip2).unwrap());
    t.store((SAAT + AYRILMA_SN + 1.0).to_bits(), Ordering::SeqCst);
    assert_eq!(v.bekleyen_bildir(&k2, &sir, ip2, "", "", WG2, SSH).unwrap().0, "bekle");
    assert!(v.bekleyen_musterinin(n2).unwrap().is_none());
    // 7 günden eski kayıt silinir
    t.store((SAAT + BEKLEYEN_SIL_SN + 5000.0).to_bits(), Ordering::SeqCst);
    assert!(v.bekleyenler().unwrap().is_empty());
}

#[test]
fn bekleyen_tablo_tavani() {
    let (v, _d) = yeni();
    {
        let c = v.kilit();
        c.execute_batch("BEGIN").unwrap();
        for i in 0..BEKLEYEN_EN_COK {
            c.execute(
                "INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, wg_pub, ssh_pub) VALUES (?, 'x', ?, ?, '1.1.1.1', 'w', 's')",
                rusqlite::params![format!("id{i:04}"), SAAT - 10.0, SAAT - 10.0],
            )
            .unwrap();
        }
        c.execute_batch("COMMIT").unwrap();
    }
    // hepsi 10 sn önce bildirdi: aday sayılır, yer açılmaz
    assert_eq!(v.bekleyen_bildir("yeni", "sir", "2.2.2.2", "", "", WG1, SSH).unwrap().0, "dolu");
    v.kilit().execute("UPDATE bekleyen SET son = ? WHERE kimlik = 'id0007'", [SAAT - BEKLEYEN_ADAY_SN - 1.0]).unwrap();
    assert_eq!(v.bekleyen_bildir("yeni", "sir", "2.2.2.2", "", "", WG1, SSH).unwrap(), ("bekle", true, None));
    assert!(!v.bekleyen_var("id0007").unwrap());
}

#[test]
fn surum_ve_kanal() {
    let (v, _d) = yeni();
    v.surum_ekle("0.4.9", &"a".repeat(64), 100, "github").unwrap();
    v.surum_ekle("0.10.0", &"b".repeat(64), 200, "github").unwrap();
    v.surum_ekle("0.11.0", &"c".repeat(64), 300, "yerel").unwrap();
    v.surum_ekle("0.4.9", &"d".repeat(64), 150, "github").unwrap(); // INSERT OR REPLACE
    let s: Vec<_> = v.surumler().unwrap().into_iter().map(|r| r.surum).collect();
    assert_eq!(s, ["0.11.0", "0.10.0", "0.4.9"]);
    assert_eq!(v.surum("0.4.9").unwrap().unwrap().boyut, 150);
    assert!(v.surum("9.9.9").unwrap().is_none());
    assert_eq!(v.en_son_surum().unwrap().as_deref(), Some("0.10.0"));
    assert_eq!(v.kanal_surumu("kararli").unwrap(), None);
    v.kanal_hedef_koy("kararli", Some(EN_SON)).unwrap();
    v.kanal_hedef_koy("test", Some("0.11.0")).unwrap();
    assert_eq!(v.kanal_surumu("kararli").unwrap().as_deref(), Some("0.10.0"));
    assert_eq!(v.kanal_surumu("test").unwrap().as_deref(), Some("0.11.0"));
    assert!(deger(v.kanal_hedef_koy("kapali", Some("0.4.9"))) && deger(v.kanal_hedef_koy("bozuk", None)));
    v.kanal_hedef_koy("test", None).unwrap();
    assert_eq!(v.kanal_hedef("test").unwrap(), None);
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let hedef = v.cihaz_hedefi(&v.musteri(n).unwrap().unwrap()).unwrap().unwrap();
    assert!(hedef.surum == "0.10.0" && hedef.sha256 == "b".repeat(64) && hedef.boyut == 200 && !hedef.hemen);
    v.guncelleme_hemen_koy(n, 1_790_000_001.0, "0.9.0").unwrap(); // başka hedef içinse geçmez
    assert!(!v.cihaz_hedefi(&v.musteri(n).unwrap().unwrap()).unwrap().unwrap().hemen);
    v.guncelleme_hemen_koy(n, 1_790_000_001.0, "0.10.0").unwrap();
    assert!(v.cihaz_hedefi(&v.musteri(n).unwrap().unwrap()).unwrap().unwrap().hemen);
    v.guncelleme_hemen_koy(n, 0.0, "0.10.0").unwrap();
    let m = v.musteri(n).unwrap().unwrap();
    assert!(m.guncelleme_hemen == 0.0 && m.guncelleme_hemen_surum.is_empty());
    assert!(deger(v.guncelleme_kanali_koy(n, "bozuk")));
    v.guncelleme_kanali_koy(n, "kapali").unwrap();
    assert!(v.cihaz_hedefi(&v.musteri(n).unwrap().unwrap()).unwrap().is_none());
    v.guncelleme_kanali_koy(n, "kararli").unwrap();
    v.askiya_al(n, true).unwrap(); // lisans/üyelik kapalı: hedef yok
    assert!(!v.hizmet_acik(&v.musteri(n).unwrap().unwrap()));
    assert!(v.cihaz_hedefi(&v.musteri(n).unwrap().unwrap()).unwrap().is_none());
}

#[test]
fn sorun_saglik_ve_sayaclar() {
    let (v, _d) = yeni();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let a = v.sorun_ac(n, "disk", "Disk %91", "uyari").unwrap();
    assert_eq!(v.sorun_ac(n, "disk", "baska", "kritik").unwrap(), a); // açık sorun varsa yenisi açılmaz
    v.sorun_metin(a, &"m".repeat(300)).unwrap();
    assert_eq!(v.acik_sorunlar(Some(n)).unwrap()[0].metin.chars().count(), 200);
    v.sorun_sms(a, "acilis_sms", "gonderildi").unwrap();
    assert!(deger(v.sorun_sms(a, "metin", "x")));
    v.sorun_kapat(a).unwrap();
    assert!(v.acik_sorunlar(None).unwrap().is_empty());
    assert_eq!(v.kapanis_bekleyenler(n).unwrap().len(), 1);
    v.sorun_sms(a, "kapanis_sms", "yok").unwrap();
    assert!(v.kapanis_bekleyenler(n).unwrap().is_empty());
    assert_eq!(v.son_kapanan(n, "disk").unwrap().unwrap().id, a);
    v.sorun_yeniden_ac(a).unwrap();
    assert_eq!(v.acik_sorunlar(Some(n)).unwrap().len(), 1);
    // sağlık
    assert!(v.saglik(7).unwrap().is_none());
    v.saglik_kaydet(7, &json!({"disk": 50, "ad": "ğ"})).unwrap();
    v.saglik_kaydet(7, &json!({"disk": 60})).unwrap();
    assert_eq!(v.saglik(7).unwrap().unwrap().rapor, json!({"disk": 60}));
    // sayaçlar ve bayrak
    assert_eq!(v.sms_sayac(n, "g").unwrap(), 0);
    assert_eq!(v.sms_sayac_artir(n, "g").unwrap(), 1);
    assert_eq!(v.sms_sayac_artir(n, "g").unwrap(), 2);
    assert_eq!(v.sms_sayaci("g").unwrap(), 0);
    assert_eq!((v.sms_say("g").unwrap(), v.sms_say("g").unwrap(), v.sms_sayaci("g").unwrap()), (1, 2, 2));
    assert!(v.gunluk_bayrak("rapor", "2026-11-04").unwrap() && !v.gunluk_bayrak("rapor", "2026-11-04").unwrap());
    v.sms_son_kaydet(n, "netgsm", "gonderildi", &"9".repeat(40)).unwrap();
    assert_eq!(v.sms_son(n).unwrap().unwrap().kod.len(), 16);
    v.saglik_son_tur_koy("2026-11-04T10:00:00+03:00").unwrap();
    assert_eq!(v.saglik_son_tur().unwrap().as_deref(), Some("2026-11-04T10:00:00+03:00"));
    // cihaz alanları
    let (cid, _) = v.cihaz_bagla(n, WG1, SSH, "", "", "").unwrap();
    assert!(deger(v.cihaz_guncelle(cid, &[("durum", "x")])) && deger(v.cihaz_guncelle(cid, &[])));
    v.eslesme_kaydet(cid, "Ad", "Unvan", "0.4.29", Some("otel"), Some("Adres 1")).unwrap();
    let c = v.bagli_cihaz(n).unwrap().unwrap();
    assert!(c.isletme_adi == "Ad" && c.isletme_turu == "otel" && c.adres == "Adres 1" && !c.son_eslesme.is_empty());
    v.eslesme_kaydet(cid, "Ad", "Unvan", "0.4.29", Some("bozuk"), None).unwrap(); // geçersiz tür/adres: eskisi kalır
    let c = v.bagli_cihaz(n).unwrap().unwrap();
    assert!(c.isletme_turu == "otel" && c.adres == "Adres 1");
    v.cihaz_otomatik_isaretle(cid).unwrap();
    assert_eq!(v.bagli_cihaz(n).unwrap().unwrap().otomatik, 1);
}

#[test]
fn uyari_ayari() {
    let (v, _d) = yeni();
    assert_eq!(v.uyari_ayari().unwrap(), json!({"telefonlar": [], "acik": false, "gunluk_sinir": 20}));
    v.uyari_ayari_koy(&json!({"telefonlar": ["0555 000 00 01", "905550000001", "+90 555 000 00 02"], "acik": true, "gunluk_sinir": "30"})).unwrap();
    assert_eq!(v.uyari_ayari().unwrap(), json!({"telefonlar": ["905550000001", "905550000002"], "acik": true, "gunluk_sinir": 30}));
    let hata = |d: Value| v.uyari_ayari_koy(&d).unwrap_err().to_string();
    assert!(hata(json!({"telefonlar": ["123"]})).contains("Geçersiz telefon: 123"));
    assert!(hata(json!({"telefonlar": ["5550000001", "5550000002", "5550000003", "5550000004"]})).contains("En çok 3 telefon"));
    assert!(hata(json!({"gunluk_sinir": 1})).contains("2 ile 100 arasında"));
    assert!(hata(json!({"gunluk_sinir": 101})).contains("2 ile 100 arasında"));
    for kotu in [json!(true), json!(5.5), json!("x"), json!(null)] {
        assert!(hata(json!({"gunluk_sinir": kotu})).contains("tamsayı"));
    }
}

#[test]
fn anahtar_ozeti_sha256() {
    assert_eq!(anahtar_ozeti("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}
