//! yonetim.rs testleri (Task 11a + 11b): Python tests/test_yonetim.py'nin 40 testi (dosyanın sonunda 11b'nin ayar sayfaları:
//! SMS, uyarı, sürümler), test_merkez'in `test_musteri_cerezi_yonetimde_gecmez`, test_bekleme'nin `test_yonetim_bekleyen_listesi`,
//! test_raporlar'ın `test_yonetim_raporlar_gerekcesiz`, test_sms_merkez'in üç yönetim testi, test_listeler'in iki yönetim testi ve
//! test_misafir_kodu'nun iki yönetim testi. Hepsi altın kayıtla (`Kayit::bas`); Python'da karşılığı olmayan iki Rust testi
//! (haftalık rapor yolu, gerçek `github_al` ile sürüm alma) kayıtsız.
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::api::testler as at;
use crate::{guvenlik, yonetici};
use crate::kayitlar::son_30_gun;
use crate::merkez::Merkez;
use crate::ortak::now_iso;
use crate::raporlar::testler as rr;
use crate::test_yardim::{self as y, Ortam};
use crate::veri::{Hareket, Veri};

const WG: &str = at::WG;
const WG2: &str = at::WG2;
const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c";
const PW: &str = "yonetici-parola-1";

struct K {
    app: Yonetim,
    o: Ortam,
    c: String,
    isler: Arc<Mutex<Vec<(String, Value)>>>,
}

impl K {
    fn v(&self) -> &Veri {
        &self.o.veri
    }

    fn al(&self, yol: &str) -> Yanit {
        y::al(&self.app, yol, &self.c)
    }

    fn gonder(&self, yol: &str, alanlar: &[(&str, &str)]) -> Yanit {
        y::gonder(&self.app, yol, &self.c, alanlar)
    }

    fn isler(&self) -> Vec<(String, Value)> {
        self.isler.lock().unwrap().clone()
    }
}

fn kayitlar_of(o: &Ortam) -> Kayitlar {
    Kayitlar::new(o.ist.clone(), o.detay.clone(), o.kisi_sir.clone())
}

fn kuyruk_kaydedici() -> (KuyrukEkle, Arc<Mutex<Vec<(String, Value)>>>) {
    let isler = Arc::new(Mutex::new(Vec::new()));
    let i2 = isler.clone();
    let k: KuyrukEkle = Box::new(move |islem, veri| {
        i2.lock().unwrap().push((islem.to_string(), veri.clone()));
        Ok("0".repeat(16))
    });
    (k, isler)
}

/// Python `kur(tmp, durum, **ek)`: yönetici dosyası, durum.json, uygulama ve giriş yapılmış çerez.
fn kur_dizinde(tmp: &Path, durum: Option<Value>, ek: impl FnOnce(Yonetim) -> Yonetim) -> K {
    let o = y::ortam_dizinde(tmp);
    let (yol, dyol) = (tmp.join("yonetici.json"), tmp.join("durum.json"));
    yonetici::yaz(&yol, "serkan", PW).unwrap();
    std::fs::write(&dyol, y::json_govde(&json!({"zaman": "", "arsivler": durum.unwrap_or(json!({}))}))).unwrap();
    let (kuyruk, isler) = kuyruk_kaydedici();
    let app = ek(Yonetim::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat(), yol, dyol, kuyruk));
    let c = y::cerez(&y::giris(&app, "serkan", PW));
    K { app, o, c, isler }
}

fn kur_ek(durum: Option<Value>, ek: impl FnOnce(Yonetim) -> Yonetim) -> K {
    kur_dizinde(&y::gecici_dizin(), durum, ek)
}

fn kur() -> K {
    kur_ek(None, |a| a)
}

fn har(v: &Veri, n: Option<i64>) -> Vec<Hareket> {
    v.hareketler(n, 300, None).unwrap()
}

fn olaylar(v: &Veri, n: Option<i64>) -> Vec<String> {
    har(v, n).into_iter().map(|h| h.olay).collect()
}

fn s(r: &Yanit) -> String {
    r.govde_metin()
}

fn konum(r: &Yanit) -> String {
    r.baslik("Location").unwrap_or("").to_string()
}

/// `re.findall(r'class="sir">([^<]+)<', h)`.
fn sir_degerleri(h: &str) -> Vec<String> {
    let ay = "class=\"sir\">";
    let mut out = Vec::new();
    let mut bas = 0;
    while let Some(i) = h[bas..].find(ay) {
        let d = bas + i + ay.len();
        let son = h[d..].find('<').map(|j| d + j).unwrap_or(h.len());
        if son > d {
            out.push(h[d..son].to_string());
        }
        bas = d;
    }
    out
}

/// `re.search(r"<tr><th>ETİKET</th><td>(.*?)</td></tr>", s, re.S).group(1)`.
fn hucre(h: &str, etiket: &str) -> String {
    let bas = format!("<tr><th>{etiket}</th><td>");
    let i = h.find(&bas).unwrap_or_else(|| panic!("{etiket} satırı yok")) + bas.len();
    let son = h[i..].find("</td></tr>").unwrap_or_else(|| panic!("{etiket} satırı kapanmıyor")) + i;
    h[i..son].to_string()
}

fn rapor(d: &[(&str, Value)]) -> Value {
    let mut r = json!({"surum": "1.0.<x>", "acik_sn": 90000, "servisler": {"portal": true, "panel": true, "kaydedici": false,
        "dnsmasq": true, "guvenlik": true}, "disk_yuzde": 91, "son_muhur": "2026-10-10", "kayit_boslugu_1s": false, "wan": true,
        "sms_sorunu": false, "bagli": 7, "hiz_hata": false});
    for (k, v) in d {
        r[*k] = v.clone();
    }
    r
}

fn surum_ekle(v: &Veri) {
    v.surum_ekle("1.2.0", &"ab".repeat(32), 2048, "github").unwrap();
}

// ======================================================================================================================
// test_yonetim.py
// ======================================================================================================================
#[test]
fn yonetici_dosyasi_ve_giris() {
    let _k = Kayit::bas("test_yonetim", "test_yonetici_dosyasi_ve_giris");
    let tmp = y::gecici_dizin();
    let yol = tmp.join("y.json");
    assert!(yonetici::yaz(&yol, "serkan", "kisa-parola").is_err());
    let sor = || Some(PW.to_string());
    let a = |x: &[&str]| x.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    assert_eq!(yonetici::main(&a(&["parola", "serkan"]), &yol, &sor, None, &|_| Ok(())), 0);
    assert_eq!(yonetici::oku(&yol).unwrap().kullanici, "serkan");
    assert_eq!(yonetici::main(&a(&["parola", "Kötü Ad"]), &yol, &sor, None, &|_| Ok(())), 2);
    let k = kur_dizinde(&tmp, None, |a| a);
    assert!(k.c.starts_with("wc_yonetim="));
    assert!(y::giris(&k.app, "serkan", "yanlis-parola-1").durum == 401 && y::giris(&k.app, "baska", PW).durum == 401);
    let (n, pw) = k.v().musteri_ekle("", None).unwrap();
    assert_eq!(y::giris(&k.app, &n.to_string(), &pw).durum, 401); // müşteri yönetime giremez
}

#[test]
fn yeni_musteri_parola_bir_kez() {
    let _k = Kayit::bas("test_yonetim", "test_yeni_musteri_parola_bir_kez");
    let k = kur();
    let v = k.v();
    let f = k.al("/yeni");
    assert!(s(&f).contains("name=\"parola\""));
    let t = y::csrf(&f);
    let r = k.gonder("/yeni", &[("csrf", &t), ("not", "x"), ("parola", "kisa"), ("parola2", "kisa")]);
    assert!(r.durum == 400 && v.musteriler().unwrap().is_empty()); // parola en az 10 karakter
    let r = k.gonder("/yeni", &[("csrf", &t), ("not", "x"), ("parola", "musteri-parola-1"), ("parola2", "baska-parola-1")]);
    assert!(r.durum == 400 && v.musteriler().unwrap().is_empty()); // iki parola aynı olmalı
    let r = k.gonder("/yeni", &[("csrf", &t), ("not", "Bocafe <Göztepe>"), ("parola", "musteri-parola-1"), ("parola2", "musteri-parola-1")]);
    let sd = sir_degerleri(&s(&r));
    assert_eq!(sd.len(), 2);
    let (numara, pw) = (sd[0].clone(), sd[1].clone());
    assert!(r.durum == 200 && guvenlik::numara_gecerli(&numara) && pw == "musteri-parola-1");
    let n: i64 = numara.parse().unwrap();
    assert!(v.parola_dogrula(n, &pw).unwrap().is_some());
    assert!(s(&k.al(&format!("/m/{numara}"))).contains(&pw)); // 2026-10-04: parola yönetimde görünür (kullanıcı isteği)
    assert!(s(&k.al("/")).contains("Bocafe &lt;Göztepe&gt;"));
    assert_eq!(har(v, Some(n))[0].olay, "MUSTERI_ACILDI");
}

#[test]
fn liste_durumlar_sorunlu_ustte_ve_arama() {
    let _k = Kayit::bas("test_yonetim", "test_liste_durumlar_sorunlu_ustte_ve_arama");
    let k = kur();
    let v = k.v();
    let (_n1, _) = v.musteri_ekle("Sağlam kafe", None).unwrap();
    let (n2, _) = v.musteri_ekle("Gecikmiş kafe", None).unwrap();
    std::fs::write(
        &k.app.durum_yolu,
        y::json_govde(&json!({"zaman": "", "arsivler": {n2.to_string(): {"tur": "cihaz", "son_gun": "2026-09-01", "boyut": 2048,
            "el_sikisma": 1_790_000_000i64 - 60}}})),
    )
    .unwrap();
    v.cihaz_bagla(n2, WG, SSH, "Kafe <B>", "", "").unwrap();
    let h = s(&k.al("/"));
    assert!(h.find("Gecikmiş kafe").unwrap() < h.find("Sağlam kafe").unwrap());
    assert!(h.contains("GECİKTİ") && h.contains("çevrimiçi") && h.contains("Kafe &lt;B&gt;") && h.contains("cihaz yok"));
    let h = s(&k.al("/?q=sa%C4%9Flam"));
    assert!(h.contains("Sağlam kafe") && !h.contains("Gecikmiş kafe"));
}

#[test]
fn liste_uyeligi_bitenler_gizli() {
    let _k = Kayit::bas("test_yonetim", "test_liste_uyeligi_bitenler_gizli");
    let k = kur();
    let v = k.v();
    let (n1, _) = v.musteri_ekle("Açık kafe", None).unwrap();
    let (n2, _) = v.musteri_ekle("Deneme", None).unwrap();
    v.uyelik(n2, "bitti").unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains(&format!("href=\"/m/{n1}\"")) && !h.contains(&format!("href=\"/m/{n2}\"")));
    assert!(h.contains("Üyeliği biten 1 müşteri listede gösterilmiyor") && h.contains("href=\"/?bitenler=1\""));
    let h = s(&k.al("/?bitenler=1"));
    assert!(h.find(&format!("href=\"/m/{n1}\"")).unwrap() < h.find(&format!("href=\"/m/{n2}\"")).unwrap()); // bitenler en altta
    assert!(h.contains("Üyeliği biten 1 müşteri de listede") && !h.contains("gösterilmiyor"));
    let h = s(&k.al(&format!("/?q={n2}")));
    assert!(h.contains(&format!("href=\"/m/{n2}\"")) && !h.contains(&format!("href=\"/m/{n1}\"")) && !h.contains("Üyeliği biten"));
    assert!(s(&k.al(&format!("/m/{n1}"))).contains("müşteri listede gizlenir")); // "Üyeliği bitir" formundaki not
    v.uyelik(n2, "aktif").unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains(&format!("href=\"/m/{n2}\"")) && !h.contains("Üyeliği biten"));
}

#[test]
fn parola_sifirla_serbest_birak_uyelik() {
    let _k = Kayit::bas("test_yonetim", "test_parola_sifirla_serbest_birak_uyelik");
    let k = kur();
    let v = k.v();
    let (n, pw) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    let f = k.al(&format!("/m/{n}"));
    let t = y::csrf(&f);
    let r = k.gonder(&format!("/m/{n}/parola-sifirla"), &[("csrf", &t), ("parola", "yeni-musteri-1"), ("parola2", "yeni-musteri-1")]);
    assert!(r.durum == 303 && v.parola_dogrula(n, "yeni-musteri-1").unwrap().is_some() && v.parola_dogrula(n, &pw).unwrap().is_none());
    assert_eq!(k.gonder(&format!("/m/{n}/serbest"), &[("csrf", &t), ("onay", "123")]).durum, 400); // numara yazılmadı
    assert_eq!(k.gonder(&format!("/m/{n}/serbest"), &[("csrf", &t), ("onay", &n.to_string())]).durum, 303);
    assert!(v.bagli_cihaz(n).unwrap().unwrap().durum == "serbest_birakiliyor" && k.isler().is_empty());
    assert_eq!(k.gonder(&format!("/m/{n}/serbest"), &[("csrf", &t), ("onay", &n.to_string()), ("zorla", "1")]).durum, 303);
    assert!(v.bagli_cihaz(n).unwrap().is_none());
    assert_eq!(k.isler(), vec![("cihaz-kapat".to_string(), json!({"numara": n.to_string(), "wg_pub": WG}))]);
    k.gonder(&format!("/m/{n}/uyelik"), &[("csrf", &t), ("islem", "bitir"), ("onay", &n.to_string())]);
    assert_eq!(v.musteri(n).unwrap().unwrap().uyelik, "bitti");
    k.gonder(&format!("/m/{n}/uyelik"), &[("csrf", &t), ("islem", "ac")]);
    assert_eq!(v.musteri(n).unwrap().unwrap().uyelik, "aktif");
    let ol = olaylar(v, Some(n));
    for o in ["PAROLA_SIFIRLANDI", "SERBEST_BIRAKMA_ISTENDI", "CIHAZ_ZORLA_AYRILDI", "UYELIK_BITTI", "UYELIK_ACILDI"] {
        assert!(ol.iter().any(|x| x == o), "{o}");
    }
    assert_eq!(k.al(if n != 1234567 { "/m/1234567" } else { "/m/7654321" }).durum, 404);
    assert!(k.al("/m/abc").durum == 404 && k.al(&format!("/m/{}", &n.to_string()[..6])).durum == 404);
}

#[test]
fn musteri_kayitlari_gerekce_ister() {
    let _k = Kayit::bas("test_yonetim", "test_musteri_kayitlari_gerekce_ister");
    let tmp = y::gecici_dizin();
    let k = kur_dizinde(&tmp, None, |a| a);
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let dun = son_30_gun(k.o.saat.t())[0].format("%Y-%m-%d").to_string();
    y::sayi_yaz(&tmp, &n.to_string(), &json!({ dun: 9 }));
    y::gun_yaz(&tmp, &n.to_string(), "2026-09-20", &[json!({"telefon": "5330000001", "ad": "Ayşe"})], None);
    assert!(s(&k.al(&format!("/m/{n}/kayitlar/"))).contains(">9<"));
    let r = k.al(&format!("/m/{n}/kayitlar/gun/2026-09-20"));
    assert!(r.durum == 303 && konum(&r).contains(&format!("donus=%2Fm%2F{n}%2Fkayitlar%2Fgun")));
    y::gerekce_ver(&k.app, &k.c);
    assert!(s(&k.al(&format!("/m/{n}/kayitlar/gun/2026-09-20"))).contains("Ayşe"));
    let h = har(v, Some(n));
    assert!(h[0].olay == "GORUNTULEME" && h[0].kim == "admin");
}

#[test]
fn eski_arsiv_baglama_ve_hesabim() {
    let _k = Kayit::bas("test_yonetim", "test_eski_arsiv_baglama_ve_hesabim");
    let tmp = y::gecici_dizin();
    let k = kur_dizinde(
        &tmp,
        Some(json!({"bocafe": {"tur": "eski", "son_gun": "2026-09-20", "boyut": 10, "el_sikisma": null},
                    "1234567": {"tur": "cihaz", "son_gun": null, "boyut": 0, "el_sikisma": null}})),
        |a| a,
    );
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let f = k.al("/eski-arsivler");
    let h = s(&f);
    assert!(h.contains("<td>bocafe</td>") && !h.contains("<td>1234567</td>")); // müşteri arşivi eski arşiv adayı değil
    let t = y::csrf(&f);
    assert_eq!(k.gonder("/eski-arsivler", &[("csrf", &t), ("ad", "yok-boyle"), ("musteri", &n.to_string())]).durum, 400);
    let baska = if n != 1234567 { "1234567" } else { "7654321" };
    assert_eq!(k.gonder("/eski-arsivler", &[("csrf", &t), ("ad", "bocafe"), ("musteri", baska)]).durum, 400);
    assert_eq!(k.gonder("/eski-arsivler", &[("csrf", &t), ("ad", "bocafe"), ("musteri", &n.to_string())]).durum, 303);
    assert_eq!(v.arsivler(n).unwrap(), vec![n.to_string(), "bocafe".to_string()]);
    assert!(!tmp.join("rapor-gruplari.json").exists()); // yol verilmediyse yazılmaz
    let hs = k.al("/hesabim");
    let t = y::csrf(&hs);
    assert_eq!(k.gonder("/hesabim", &[("csrf", &t), ("mevcut", PW), ("yeni", "kisa"), ("tekrar", "kisa")]).durum, 400);
    assert_eq!(k.gonder("/hesabim", &[("csrf", &t), ("mevcut", PW), ("yeni", "yeni-yonetici-12"), ("tekrar", "yeni-yonetici-12")]).durum, 200);
    assert_eq!(y::giris(&k.app, "serkan", "yeni-yonetici-12").durum, 303);
}

#[test]
fn eski_arsiv_baglaninca_rapor_gruplari_yazilir() {
    let _k = Kayit::bas("test_yonetim", "test_eski_arsiv_baglaninca_rapor_gruplari_yazilir");
    let tmp = y::gecici_dizin();
    let gy = tmp.join("rapor-gruplari.json");
    let gy2 = gy.clone();
    let k = kur_dizinde(
        &tmp,
        Some(json!({"bocafe": {"tur": "eski", "son_gun": "2026-09-20", "boyut": 10, "el_sikisma": null}})),
        move |a| a.rapor_gruplari_ile(gy2),
    );
    let (n, _) = k.v().musteri_ekle("", None).unwrap();
    let f = k.al("/eski-arsivler");
    let t = y::csrf(&f);
    assert_eq!(k.gonder("/eski-arsivler", &[("csrf", &t), ("ad", "bocafe"), ("musteri", &n.to_string())]).durum, 303);
    let oku = || -> Value { serde_json::from_str(&std::fs::read_to_string(&gy).unwrap()).unwrap() };
    assert_eq!(oku(), json!({ n.to_string(): ["bocafe"] }));
    assert_eq!(k.gonder("/eski-arsivler", &[("csrf", &t), ("ad", "bocafe"), ("musteri", "")]).durum, 303);
    assert_eq!(oku(), json!({})); // bağ kaldırıldı
}

#[test]
fn yonetici_parolasi_degisince_eski_oturum_duser() {
    let _k = Kayit::bas("test_yonetim", "test_yonetici_parolasi_degisince_eski_oturum_duser");
    let k = kur();
    let c2 = y::cerez(&y::giris(&k.app, "serkan", PW));
    let h = k.al("/hesabim");
    let r = k.gonder("/hesabim", &[("csrf", &y::csrf(&h)), ("mevcut", PW), ("yeni", "yeni-yonetici-12"), ("tekrar", "yeni-yonetici-12")]);
    assert!(r.durum == 200 && y::al(&k.app, "/", &y::cerez(&r)).durum == 200); // değiştiren yeni oturumla devam eder
    assert!(y::al(&k.app, "/", &c2).durum == 303 && k.al("/").durum == 303);
}

#[test]
fn uyelik_bitir_onay_ister_yeniden_acma_serbest_birakmayi_geri_alir() {
    let _k = Kayit::bas("test_yonetim", "test_uyelik_bitir_onay_ister_yeniden_acma_serbest_birakmayi_geri_alir");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    let t = y::csrf(&k.al(&format!("/m/{n}")));
    assert_eq!(k.gonder(&format!("/m/{n}/uyelik"), &[("csrf", &t), ("islem", "bitir")]).durum, 400);
    assert!(v.musteri(n).unwrap().unwrap().uyelik == "aktif" && v.bagli_cihaz(n).unwrap().unwrap().durum == "bagli");
    k.gonder(&format!("/m/{n}/uyelik"), &[("csrf", &t), ("islem", "bitir"), ("onay", &n.to_string())]);
    assert!(v.musteri(n).unwrap().unwrap().uyelik == "bitti" && v.bagli_cihaz(n).unwrap().unwrap().durum == "serbest_birakiliyor");
    k.gonder(&format!("/m/{n}/uyelik"), &[("csrf", &t), ("islem", "ac")]);
    assert!(v.musteri(n).unwrap().unwrap().uyelik == "aktif" && v.bagli_cihaz(n).unwrap().unwrap().durum == "bagli");
}

#[test]
fn parola_yetki_birakilmadan_once_sorulur() {
    let _k = Kayit::bas("test_yonetim", "test_parola_yetki_birakilmadan_once_sorulur");
    let tmp = y::gecici_dizin();
    let sira = std::cell::RefCell::new(Vec::<String>::new());
    let kod = yonetici::main(
        &["parola".to_string(), "serkan".to_string()],
        &tmp.join("y.json"),
        &|| {
            sira.borrow_mut().push("sor".into());
            Some(PW.to_string())
        },
        Some("wcpanel"),
        &|_| {
            sira.borrow_mut().push("birak".into());
            Ok(())
        },
    );
    assert!(kod == 0 && *sira.borrow() == ["sor", "birak"]); // wcpanel terminale erişemez: parola root iken sorulur
}

#[test]
fn parola_gorunur_ve_lisans_yonetimi() {
    let _k = Kayit::bas("test_yonetim", "test_parola_gorunur_ve_lisans_yonetimi");
    let k = kur();
    let v = k.v();
    let (n, pw) = v.musteri_ekle("Göztepe Bilgisayar", None).unwrap();
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains(&pw) && h.contains("21.09.2027") && h.contains("Lisans") && !h.contains("2027-09-21")); // ekranda gün.ay.yıl
    let t = y::csrf(&k.al(&format!("/m/{n}")));
    let lisans = |alanlar: &[(&str, &str)]| {
        let mut a = vec![("csrf", t.as_str())];
        a.extend_from_slice(alanlar);
        k.gonder(&format!("/m/{n}/lisans"), &a)
    };
    let bitis = || v.musteri(n).unwrap().unwrap().lisans_bitis;
    assert_eq!(lisans(&[("islem", "uzat")]).durum, 303);
    assert_eq!(bitis(), "2028-09-20");
    lisans(&[("islem", "askiya")]);
    assert_eq!(v.musteri(n).unwrap().unwrap().askida, 1);
    lisans(&[("islem", "ac")]);
    assert_eq!(v.musteri(n).unwrap().unwrap().askida, 0);
    lisans(&[("islem", "tarih"), ("tarih", "2026-09-01")]);
    assert_eq!(bitis(), "2026-09-01");
    lisans(&[("islem", "tarih"), ("tarih", "15.3.2027")]); // gün.ay.yıl yazımı da
    assert_eq!(bitis(), "2027-03-15"); // saklanan ISO
    lisans(&[("islem", "tarih"), ("tarih", "2026-09-01")]);
    assert_eq!(bitis(), "2026-09-01");
    assert_eq!(lisans(&[("islem", "tarih"), ("tarih", "bozuk")]).durum, 400);
    let liste = s(&k.al("/"));
    assert!(liste.contains("Lisans") && liste.contains("süresi doldu"));
    let ol = olaylar(v, Some(n));
    for o in ["LISANS_UZATILDI", "LISANS_ASKIYA_ALINDI", "LISANS_ACILDI", "LISANS_TARIHI"] {
        assert!(ol.iter().any(|x| x == o), "{o}");
    }
    let (v2, _) = v.musteri_ekle("", None).unwrap();
    v.kilit().execute("UPDATE musteri SET parola_acik = '' WHERE numara = ?", [v2]).unwrap();
    assert!(s(&k.al(&format!("/m/{v2}"))).contains("bilinmiyor"));
}

#[test]
fn sifirlama_sayfasi_ve_lisans_tarihi_kaydi() {
    let _k = Kayit::bas("test_yonetim", "test_sifirlama_sayfasi_ve_lisans_tarihi_kaydi");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let t = y::csrf(&k.al(&format!("/m/{n}")));
    let r = k.gonder(&format!("/m/{n}/parola-sifirla"), &[("csrf", &t), ("parola", "kisa"), ("parola2", "kisa")]);
    assert!(r.durum == 400 && s(&r).contains("10")); // kısa parola reddedilir, rastgele parola üretilmez
    k.gonder(&format!("/m/{n}/lisans"), &[("csrf", &t), ("islem", "tarih"), ("tarih", "20270101")]);
    assert_eq!(har(v, Some(n))[0].ayrinti, "bitis=2027-01-01"); // girilen değil, kaydedilen tarih
    assert!(s(&k.al(&format!("/m/{n}"))).contains("gününden sonra"));
}

#[test]
fn ap_yonetim_parolasi_musteri_sayfasinda() {
    let _k = Kayit::bas("test_yonetim", "test_ap_yonetim_parolasi_musteri_sayfasinda");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", Some("musteri-parola-1")).unwrap();
    let ap = v.musteri(n).unwrap().unwrap().ap_parola;
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("AP yönetim parolası") && h.contains(&format!("<span class=\"sir\">{ap}</span>")) && h.contains(&format!("action=\"/m/{n}/ap-parola\"")));
    assert!(k.gonder(&format!("/m/{n}/ap-parola"), &[]).durum == 403 && v.musteri(n).unwrap().unwrap().ap_parola == ap); // CSRF'siz
    let r = k.gonder(&format!("/m/{n}/ap-parola"), &[("csrf", &y::csrf(&k.al(&format!("/m/{n}"))))]);
    let yeni = v.musteri(n).unwrap().unwrap().ap_parola;
    assert!(r.durum == 303 && konum(&r) == format!("/m/{n}?acik=parolalar#parolalar")); // Parolalar bölümü açık döner
    assert!(yeni != ap && s(&k.al(&format!("/m/{n}"))).contains(&yeni));
    let h = &har(v, Some(n))[0];
    assert!((h.olay.as_str(), h.kim.as_str()) == ("AP_PAROLASI_YENILENDI", "admin") && !h.ayrinti.contains(&yeni) && !h.ayrinti.contains(&ap));
}

#[test]
fn ap_parolasi_yenile_notu() {
    let _k = Kayit::bas("test_yonetim", "test_ap_parolasi_yenile_notu");
    let k = kur();
    let (n, _) = k.v().musteri_ekle("", Some("musteri-parola-1")).unwrap();
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Cihaz çevrimdışıyken yenilerseniz cihaz yeniden bağlanana kadar eski parolayı kullanır."));
    let c = hucre(&h, "AP yönetim parolası");
    assert!(c.contains(&format!("action=\"/m/{n}/ap-parola\"")) && c.contains("Cihaz çevrimdışıyken yenilerseniz")); // Yenile ile aynı hücrede
    assert!(c.contains("cihaza 5 dk içinde ulaşır (0.4.17'den eski cihazlarda her gün 06:00'da)")); // eski cihazlar için de doğru
}

/// Python `_bolumler`: kimlik → (açık mı, başlık satırı, bölümün HTML'i), sayfadaki sırayla.
fn bolumler(h: &str) -> Vec<(String, (bool, String, String))> {
    let mut out = Vec::new();
    for p in h.split("<details class=\"bolum\" id=\"").skip(1) {
        let (kimlik, geri) = p.split_once('"').unwrap();
        let ozet = geri.split_once("<summary>").unwrap().1.split_once("</summary>").unwrap().0;
        out.push((kimlik.to_string(), (geri.starts_with(" open>"), ozet.to_string(), geri.split("</main>").next().unwrap().to_string())));
    }
    out
}

fn bolum<'a>(b: &'a [(String, (bool, String, String))], kimlik: &str) -> &'a (bool, String, String) {
    &b.iter().find(|(k, _)| k == kimlik).unwrap_or_else(|| panic!("{kimlik} bölümü yok")).1
}

#[test]
fn musteri_sayfasi_acilir_bolumler() {
    let _k = Kayit::bas("test_yonetim", "test_musteri_sayfasi_acilir_bolumler");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("Ahmet Bey", Some("musteri-parola-1")).unwrap();
    v.cihaz_bagla(n, WG, SSH, "Kafe Örnek", "", "").unwrap();
    let f = k.al(&format!("/m/{n}"));
    let (h, t) = (s(&f), y::csrf(&f));
    let ust = h.split("<details class=\"bolum\"").next().unwrap().to_string();
    assert!(ust.contains("Kafe Örnek") && ust.contains("Not: Ahmet Bey") && ust.contains("Lisans: <b>Aktif</b> · bitiş"));
    for yol in ["kayitlar/", "raporlar", "hareketler"] {
        assert!(ust.contains(&format!("<a class=\"dugme\" href=\"/m/{n}/{yol}\">")), "{yol}");
    }
    let b = bolumler(&h);
    assert_eq!(b.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["parolalar", "lisans", "cihaz", "saglik", "guncelleme", "sorunlar"]);
    assert!(!b.iter().any(|(_, (acik, _, _))| *acik)); // hepsi kapalı gelir
    assert!(bolum(&b, "lisans").1.contains("<h2>Lisans ve üyelik</h2>") && bolum(&b, "lisans").1.contains("<b>Aktif</b> · bitiş"));
    assert!(bolum(&b, "saglik").1.contains("henüz rapor yok") && bolum(&b, "sorunlar").1.contains("kayıt yok"));
    // uzun açıklamalar ve nadir/tehlikeli işlemler bölümün içinde ayrıca açılır
    assert!(!bolum(&b, "parolalar").2.contains("<details class=\"ic\"><summary>Açıklama</summary><div><p class=\"alt\">AP&#x27;nin"));
    assert!(bolum(&b, "parolalar").2.contains("<details class=\"ic\"><summary>Açıklama</summary><div><p class=\"alt\">AP'nin (modemin)"));
    assert!(bolum(&b, "parolalar").2.contains("<details class=\"ic\"><summary>Parolayı değiştir</summary>"));
    let (ana, tehlike) = bolum(&b, "lisans").2.split_once("<details class=\"ic tehlikeli\"><summary>Askıya al / Üyeliği bitir</summary>").unwrap();
    assert!(tehlike.contains("value=\"askiya\"") && tehlike.contains("value=\"bitir\"") && !ana.contains("value=\"askiya\"") && ana.contains("value=\"uzat\""));
    let (ana, tehlike) = bolum(&b, "cihaz").2.split_once("<details class=\"ic tehlikeli\"><summary>Cihazı serbest bırak</summary>").unwrap();
    assert!(tehlike.contains(&format!("action=\"/m/{n}/serbest\"")) && !ana.contains(&format!("action=\"/m/{n}/serbest\"")));
    // ?acik= ile yalnızca o bölüm açık gelir; bilinmeyen değer hiçbirini açmaz
    let b = bolumler(&s(&k.al(&format!("/m/{n}?acik=lisans"))));
    assert_eq!(b.iter().filter(|(_, (a, _, _))| *a).map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["lisans"]);
    assert!(!bolumler(&s(&k.al(&format!("/m/{n}?acik=x%22%3E")))).iter().any(|(_, (a, _, _))| *a));

    // işlemden dönüşte işlemin bölümü açık gelir (çapa sayfayı oraya kaydırır)
    let don = |yol: &str, alanlar: &[(&str, &str)]| -> String {
        let mut a = alanlar.to_vec();
        a.push(("csrf", &t));
        let r = k.gonder(&format!("/m/{n}/{yol}"), &a);
        assert_eq!(r.durum, 303, "{yol}");
        konum(&r)
    };
    assert_eq!(don("lisans", &[("islem", "uzat")]), format!("/m/{n}?acik=lisans#lisans"));
    assert_eq!(don("lisans", &[("islem", "askiya")]), format!("/m/{n}?acik=lisans#lisans"));
    let b = bolumler(&s(&k.al(&format!("/m/{n}"))));
    let (ana, tehlike) = bolum(&b, "lisans").2.split_once("<details class=\"ic tehlikeli\">").unwrap();
    assert!(ana.contains("Askıdan çıkar (cihazı aç)") && tehlike.contains("<summary>Üyeliği bitir</summary>") && !bolum(&b, "lisans").2.contains("value=\"askiya\""));
    assert!(bolum(&b, "lisans").1.contains("<b class=\"kotu\">Askıda</b>"));
    assert_eq!(don("lisans", &[("islem", "ac")]), format!("/m/{n}?acik=lisans#lisans"));
    assert_eq!(don("parola-sifirla", &[("parola", "yeni-musteri-1"), ("parola2", "yeni-musteri-1")]), format!("/m/{n}?acik=parolalar#parolalar"));
    assert_eq!(don("guncelleme", &[("kanal", "test")]), format!("/m/{n}?acik=guncelleme#guncelleme"));
    assert!(bolum(&bolumler(&s(&k.al(&format!("/m/{n}")))), "guncelleme").1.contains("kanal test"));
    v.sorun_ac(n, "disk_dolu", "Disk dolu (%91)", "uyari").unwrap();
    assert!(bolum(&bolumler(&s(&k.al(&format!("/m/{n}")))), "sorunlar").1.contains("1 kayıt · <b class=\"kotu\">1 açık</b>"));
    assert_eq!(don("serbest", &[("onay", &n.to_string())]), format!("/m/{n}?acik=cihaz#cihaz"));
    assert_eq!(don("uyelik", &[("islem", "bitir"), ("onay", &n.to_string())]), format!("/m/{n}?acik=lisans#lisans"));
    let h = s(&k.al(&format!("/m/{n}")));
    let b = bolumler(&h);
    assert!(h.split("<details class=\"bolum\"").next().unwrap().contains("<b class=\"kotu\">Üyelik bitti</b>") && bolum(&b, "lisans").1.contains("üyelik bitti"));
    assert!(bolum(&b, "lisans").2.split("<details class=\"ic").next().unwrap().contains("Üyeliği yeniden aç")); // ana kısımda, tehlikeli değil
    assert_eq!(don("uyelik", &[("islem", "ac")]), format!("/m/{n}?acik=lisans#lisans"));
}

#[test]
fn cihaz_kartinda_internet() {
    let _k = Kayit::bas("test_yonetim", "test_cihaz_kartinda_internet");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let (cid, _) = v.cihaz_bagla(n, WG, SSH, "Kafe", "", "").unwrap();
    let internet = |r: &Value| -> String {
        v.saglik_kaydet(cid, r).unwrap();
        hucre(&s(&k.al(&format!("/m/{n}"))), "İnternet")
    };
    let kotu = |m: &str| format!("<b class=\"kotu\">{m}</b>");
    let tur = |w: Value, p: Value| rapor(&[("wan_tur", w), ("pppoe", p)]);
    let (tm, pp) = (|x: &str| json!(x), |x: &str| json!(x));
    // spec örneği: parola reddedildi → kötü işaretli
    assert_eq!(internet(&tur(tm("pppoe"), pp("kimlik_hatasi"))), kotu("PPPoE: parola reddedildi"));
    assert_eq!(internet(&tur(tm("dhcp"), pp("kapali"))), "Modem (DHCP)");
    assert_eq!(internet(&rapor(&[])), "—"); // eski cihaz: alan yok
    // öteki durumlar (plandaki kart metinleri birebir)
    assert_eq!(internet(&tur(tm("pppoe"), pp("bagli"))), "PPPoE bağlı");
    assert_eq!(internet(&tur(tm("pppoe"), pp("baglaniyor"))), "PPPoE: bağlanıyor");
    assert_eq!(internet(&tur(tm("pppoe"), pp("sunucu_yok"))), kotu("PPPoE: sunucu yanıt vermiyor"));
    // PPPoE açık ama internet modemden (0.4.18): sorun değil, kötü işaretli değil
    assert_eq!(internet(&tur(tm("pppoe"), pp("modemden"))), "Modemden bağlı (DHCP); PPPoE ONT&#x27;ye takılınca bağlanır");
    assert_eq!(internet(&tur(tm("pppoe"), pp("kablo_yok"))), kotu("kablo yok"));
    assert_eq!(internet(&tur(tm("dhcp"), pp("kablo_yok"))), kotu("kablo yok"));
    assert_eq!(internet(&tur(tm("dhcp"), pp("dhcp_yok"))), kotu("modemden adres alınamadı"));
    // adres var ama etkin yoklama internete çıkamadı (0.4.20): kötü işaretli
    assert_eq!(internet(&tur(tm("dhcp"), pp("internet_yok"))), kotu("adres var ama internet yok"));
    // durum alanı yok → "—": tür tek başına gösterilmez; tür atılmış olsa da durum gösterilir
    assert!(internet(&rapor(&[("wan_tur", json!("dhcp"))])) == "—" && internet(&rapor(&[("wan_tur", json!("pppoe"))])) == "—");
    assert_eq!(internet(&rapor(&[("pppoe", json!("bagli"))])), "PPPoE bağlı");
    for bozuk in [json!("x"), json!("<b>"), json!(5), Value::Null, json!(["bagli"]), json!({"bagli": 1})] {
        // DB'deki bozuk değer sayfayı çökertmez, sayfaya sızmaz
        assert_eq!(internet(&tur(bozuk.clone(), bozuk.clone())), "—", "{bozuk}");
    }
    // uçtan uca: yeni cihaz sürümünün bilinmeyen durum kodu saglik_temizle'den geçer (rapor kabul, alan atılır) → satır "—"
    let temiz = |r: Value| crate::api::saglik_temizle(&r).unwrap();
    assert_eq!(internet(&temiz(tur(json!("pppoe"), json!("yeni_durum")))), "—");
    assert_eq!(internet(&temiz(tur(json!("x"), json!("bagli")))), "PPPoE bağlı");
    assert_eq!(internet(&temiz(tur(json!("x"), json!("?")))), "—");
    // WAN satırı olduğu gibi kalır (adres var/yok); İnternet hemen ondan sonra gelir
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.find("<th>WAN</th>").unwrap() < h.find("<th>İnternet</th>").unwrap() && h.find("<th>İnternet</th>").unwrap() < h.find("<th>SMS</th>").unwrap());
}

#[test]
fn tek_admin_parolasi_hesabimda() {
    let _k = Kayit::bas("test_yonetim", "test_tek_admin_parolasi_hesabimda");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    assert!(!s(&k.al(&format!("/m/{n}"))).contains(&format!("/m/{n}/admin-parola"))); // cihaz başına admin parolası yok
    let r = k.al("/admin-parolasi");
    assert_eq!((r.durum, konum(&r)), (303, "/hesabim".to_string()));
    let ana = s(&k.al("/"));
    assert!(!ana.contains("href=\"/admin-parolasi\"") && ana.contains("href=\"/hesabim\""));
    let h = k.al("/hesabim");
    let h_ = s(&h);
    assert!(h_.contains("Admin parolası (bütün cihazlar)") && h_.contains("tanımsız") && h_.contains("action=\"/admin-parolasi\""));
    assert!(h_.find("Yönetici parolası").unwrap() < h_.find("Admin parolası (bütün cihazlar)").unwrap());
    let t = y::csrf(&h);
    let r = k.gonder("/admin-parolasi", &[("csrf", &t), ("parola", "benim-admin-parolam"), ("parola2", "baska")]);
    assert!(r.durum == 400 && v.admin_parolasi().unwrap().is_none());
    let r = k.gonder("/admin-parolasi", &[("csrf", &t), ("parola", "benim-admin-parolam"), ("parola2", "benim-admin-parolam")]);
    assert_eq!((r.durum, konum(&r)), (303, "/hesabim".to_string()));
    assert!(v.admin_parolasi().unwrap().is_some() && s(&k.al("/hesabim")).contains("tanımlı"));
    assert!(har(v, None).iter().all(|h| !h.ayrinti.contains("benim-admin-parolam")));
    assert!(har(v, None).iter().any(|h| h.olay == "ADMIN_PAROLASI"));
}

/// `re.findall(r'<a href="([^"]+)" class="secili"', h)`.
fn secili(h: &str) -> Vec<String> {
    let mut out = Vec::new();
    for p in h.split("<a href=\"").skip(1) {
        if let Some((href, geri)) = p.split_once('"') {
            if !href.is_empty() && geri.starts_with(" class=\"secili\"") {
                out.push(href.to_string());
            }
        }
    }
    out
}

/// `re.findall(r'<details class="grup"( open)?><summary>([^<]+)</summary>', h)`: (açık mı, başlık).
fn gruplar(h: &str) -> Vec<(bool, String)> {
    let mut out = Vec::new();
    for p in h.split("<details class=\"grup\"").skip(1) {
        let (acik, geri) = match p.strip_prefix(" open>") {
            Some(g) => (true, g),
            None => match p.strip_prefix('>') {
                Some(g) => (false, g),
                None => continue,
            },
        };
        if let Some(b) = geri.strip_prefix("<summary>") {
            if let Some((baslik, _)) = b.split_once("</summary>") {
                out.push((acik, baslik.to_string()));
            }
        }
    }
    out
}

#[test]
fn yonetim_sol_menu() {
    let _k = Kayit::bas("test_yonetim", "test_yonetim_sol_menu");
    let k = kur();
    let (n, _) = k.v().musteri_ekle("", None).unwrap();
    let h = s(&k.al("/"));
    assert!(!h.contains("<header>"));
    // telefonda menü gizli, üst şeritteki düğme açar (cihaz paneli gibi; betik yok): onay kutusu menüden önce gelmeli
    let (a, b, c) = (h.find("<input type=\"checkbox\" id=\"menu-ac\"").unwrap(), h.find("<div class=\"ust-mobil\">").unwrap(), h.find("<div class=\"yerlesim\">").unwrap());
    assert!(a < b && b < c);
    assert!(h.contains("<label for=\"menu-ac\" class=\"menu-dugme\"") && h.contains("#menu-ac:checked~.yerlesim nav.yan{display:block}"));
    let menu = &h[h.find("<nav class=\"yan\">").unwrap()..h.find("</nav>").unwrap()];
    assert!(menu.starts_with("<nav class=\"yan\"><a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\"><svg class=\"logo\"")); // koyu logo
    let sira = [
        ">Müşteriler<", ">Müşteri listesi<", ">Yeni müşteri<", ">Bekleyen cihazlar<", ">Kayıtlar<", ">Hareketler<", ">Eski arşivler<", ">Ayarlar<",
        ">Engel listeleri<", ">SMS ayarları<", ">Uyarı ayarları<", ">Sürümler<", ">Hesabım<", "action=\"/cikis\"",
    ];
    let konumlar: Vec<usize> = sira.iter().map(|x| menu.find(x).unwrap_or_else(|| panic!("{x}"))).collect();
    let mut sirali = konumlar.clone();
    sirali.sort();
    assert_eq!(konumlar, sirali);
    for href in ["/", "/yeni", "/bekleyen", "/surumler", "/hareketler", "/eski-arsivler", "/listeler", "/sms", "/uyari", "/hesabim"] {
        assert!(menu.contains(&format!("<a href=\"{href}\"")), "{href}");
    }
    assert!(!menu.contains("/admin-parolasi"));
    let sec = |yol: &str| secili(&s(&k.al(yol)));
    assert!(sec("/") == ["/"] && sec(&format!("/m/{n}")) == ["/"] && sec("/yeni") == ["/yeni"]);
    assert!(sec("/sms") == ["/sms"] && sec("/hesabim") == ["/hesabim"] && sec("/surumler") == ["/surumler"]);
    assert!(sec("/eski-arsivler") == ["/eski-arsivler"] && sec("/yok-boyle-sayfa").is_empty());
    assert!(secili_menu("/admin-parolasi") == Some("/hesabim") && secili_menu("/uyari/deneme") == Some("/uyari"));
    assert!(secili_menu("/smsx").is_none() && secili_menu("/m/") == Some("/"));
    let gr = |yol: &str| gruplar(&s(&k.al(yol)));
    let kisa = |l: &[(bool, &str)]| l.iter().map(|(a, b)| (*a, b.to_string())).collect::<Vec<_>>();
    assert_eq!(gr("/"), kisa(&[(true, "Müşteriler"), (false, "Kayıtlar"), (false, "Ayarlar")]));
    // Python zincirli karşılaştırma: ortadaki işlenen (/bekleyen) bir kez istenir
    let (g_musteri, g_bekleyen) = (gr(&format!("/m/{n}")), gr("/bekleyen"));
    assert!(g_musteri == g_bekleyen && g_bekleyen == gr("/"));
    assert_eq!(gr("/eski-arsivler"), kisa(&[(false, "Müşteriler"), (true, "Kayıtlar"), (false, "Ayarlar")]));
    let ayarlar = kisa(&[(false, "Müşteriler"), (false, "Kayıtlar"), (true, "Ayarlar")]);
    assert!(gr("/sms") == ayarlar && gr("/hesabim") == ayarlar && gr("/surumler") == ayarlar);
    assert!(!s(&k.al("/")).contains("class=\"grup tek\"")); // başlıksız grup kalmadı
}

#[test]
fn acik_sorunlar_karti() {
    let _k = Kayit::bas("test_yonetim", "test_acik_sorunlar_karti");
    let k = kur();
    let v = k.v();
    let (n1, _) = v.musteri_ekle("a", None).unwrap();
    let (n2, _) = v.musteri_ekle("b", None).unwrap();
    assert!(!s(&k.al("/")).contains("Açık sorunlar")); // sorun yoksa kart yok
    v.cihaz_bagla(n1, WG, SSH, "Kafe <B>", "", "").unwrap();
    let t0 = k.o.saat.t();
    k.o.saat.koy(t0 - 7200.0); // sorunlar iki saat önce açıldı (oturum bozulmasın diye saati geri alıp kaydet)
    let i1 = v.sorun_ac(n1, "kayit_durdu", "5651 kaydı durdu <i>", "kritik").unwrap();
    v.sorun_ac(n2, "lisans_bitti", "Lisans bitti (01.10.2026)", "bilgi").unwrap();
    v.sorun_sms(i1, "acilis_sms", "gonderildi").unwrap();
    k.o.saat.koy(t0);
    let h = s(&k.al("/"));
    assert!(h.contains("Açık sorunlar") && h.contains(&format!("href=\"/m/{n1}\"")) && h.contains(&format!("href=\"/m/{n2}\"")));
    assert!(h.contains("Kafe &lt;B&gt;") && h.contains("5651 kaydı durdu &lt;i&gt;") && !h.contains("durdu <i>"));
    assert!(h.contains("2 saat önce") && h.contains("09:00&#x27;da gidecek") && h.contains("SMS gitti") && h.contains("kritik"));
    let satir = &h[h.find(&format!("<a href=\"/m/{n1}\">{n1}</a></td><td>Kafe")).unwrap()..];
    assert!(satir[..satir.find("</tr>").unwrap()].contains("5651 kaydı durdu")); // liste satırındaki sorunlu rozeti
    v.sorun_kapat(i1).unwrap();
    assert!(!s(&k.al("/")).contains("5651 kaydı durdu"));
    v.sorun_ac(0, "liste_guncellenemiyor", "Engel listeleri güncellenemiyor: Kumar ve bahis", "uyari").unwrap(); // merkezin satırı
    let h = s(&k.al("/"));
    assert!(h.contains("<a href=\"/listeler\">Merkez</a>") && h.contains("Engel listeleri güncellenemiyor: Kumar ve bahis"));
    assert!(!h.contains("href=\"/m/0\""));
}

#[test]
fn sms_durum_etiketleri() {
    let _k = Kayit::bas("test_yonetim", "test_sms_durum_etiketleri");
    let f = sms_etiketi;
    assert!(f("", "kritik") == "SMS bekleniyor" && f("", "uyari") == "SMS bekleniyor" && f("", "bilgi") == "09:00'da gidecek");
    assert!(f("gonderildi", "kritik") == "SMS gitti" && f("mock", "uyari") == "deneme modu");
    assert!(f("hata:30", "kritik") == "SMS gönderilemedi (30)" && f("sinir", "uyari") == "günlük SMS sınırı");
    assert_eq!(f("yok", "kritik"), "SMS gönderilmedi (uyarılar kapalı)");
    assert!(f("sinir:2026-10-11", "kritik") == "günlük SMS sınırı" && f("sinir:2026-10-11", "bilgi") == "günlük SMS sınırı");
}

#[test]
fn gecmis_sms_etiketleri_testi() {
    let _k = Kayit::bas("test_yonetim", "test_gecmis_sms_etiketleri");
    let t = 1_790_000_000.0;
    let iso = now_iso;
    let f = |onem: &str, acilis_sms: &str, kapanis_sms: &str, kapanis: String| {
        let x = Sorun {
            id: 0,
            musteri: 0,
            tur: String::new(),
            metin: String::new(),
            onem: onem.into(),
            acilis: String::new(),
            kapanis,
            acilis_sms: acilis_sms.into(),
            kapanis_sms: kapanis_sms.into(),
        };
        gecmis_sms_etiketleri(&x, t)
    };
    let e2 = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(f("kritik", "", "", String::new()), e2("SMS bekleniyor", "—"));
    assert_eq!(f("bilgi", "", "", String::new()), e2("09:00'da gidecek", "—")); // açık
    // kapanmış, hiç SMS'siz: sonsuza dek "bekleniyor" demez
    assert_eq!(f("kritik", "", "", iso(t - 5000.0)), e2("—", "—"));
    assert_eq!(f("bilgi", "", "", iso(t - 5000.0)), e2("—", "—"));
    assert_eq!(f("kritik", "gonderildi", "", iso(t - 100.0)), e2("SMS gitti", "düzeldi bekletiliyor"));
    assert_eq!(f("kritik", "mock", "", iso(t - 100.0)), e2("deneme modu", "düzeldi bekletiliyor"));
    assert_eq!(f("kritik", "gonderildi", "", iso(t - 700.0)), e2("SMS gitti", "—"));
    assert_eq!(f("kritik", "sinir:2026-09-21", "", iso(t - 100.0)), e2("günlük SMS sınırı", "—")); // açılış gitmedi: düzeldi de yok
    assert_eq!(f("kritik", "gonderildi", "gonderildi", iso(t - 5000.0)), e2("SMS gitti", "SMS gitti"));
    assert_eq!(f("kritik", "gonderildi", "sinir:2026-09-21", iso(t - 5000.0)), e2("SMS gitti", "günlük SMS sınırı"));
}

#[test]
fn izleme_satiri_ana_sayfada() {
    let _k = Kayit::bas("test_yonetim", "test_izleme_satiri_ana_sayfada");
    let k = kur();
    let v = k.v();
    let h = s(&k.al("/"));
    assert!(h.contains("<b class=\"kotu\">Sağlık izleme: hiç tur yok — izleme çalışmıyor olabilir</b>")); // hiç çalışmadı
    v.saglik_son_tur_koy(&now_iso(k.o.saat.t() - 180.0)).unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains("Sağlık izleme: son tur 3 dk önce") && !h.contains("izleme çalışmıyor olabilir"));
    v.saglik_son_tur_koy(&now_iso(k.o.saat.t() - 16.0 * 60.0)).unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains("<b class=\"kotu\">Sağlık izleme: son tur 16 dk önce — izleme çalışmıyor olabilir</b>"));
}

#[test]
fn musteri_saglik_ve_gecmis() {
    let _k = Kayit::bas("test_yonetim", "test_musteri_saglik_ve_gecmis");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Henüz rapor yok") && h.contains("Sorun geçmişi"));
    let (cid, _) = v.cihaz_bagla(n, WG, SSH, "Kafe", "", "").unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Henüz rapor yok"));
    let t0 = k.o.saat.t();
    k.o.saat.koy(t0 - 600.0);
    v.saglik_kaydet(cid, &rapor(&[])).unwrap();
    k.o.saat.koy(t0);
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(!h.contains("Henüz rapor yok") && h.contains("10 dk önce"));
    assert!(h.contains("1.0.&lt;x&gt;") && !h.contains("1.0.<x>")); // cihaz verisi kaçırılır
    assert!(h.contains("kaydedici") && h.contains("%91") && h.contains("10.10.2026") && h.contains(">7<"));
    k.o.saat.koy(t0 - 3600.0);
    let i = v.sorun_ac(n, "disk_dolu", "Disk dolu (%91)", "uyari").unwrap();
    k.o.saat.koy(t0 - 3600.0 + 1800.0);
    v.sorun_kapat(i).unwrap();
    k.o.saat.koy(t0);
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Disk dolu (%91)"));
    for i in 0..60 {
        v.sorun_ac(n, &format!("t{i}"), &format!("Sorun {i}"), "uyari").unwrap();
    }
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Sorun 59") && !h.contains("Sorun 5<") && !h.contains("Disk dolu")); // son 50
}

#[test]
fn sorun_gecmisi_kapanmis_satir_etiketleri() {
    let _k = Kayit::bas("test_yonetim", "test_sorun_gecmisi_kapanmis_satir_etiketleri");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let t0 = k.o.saat.t();
    k.o.saat.koy(t0 - 3600.0);
    let i = v.sorun_ac(n, "rapor_yok", "Rapor gelmiyor", "kritik").unwrap();
    let j = v.sorun_ac(n, "lisans_bitiyor", "Lisans bitiyor (01.10.2026)", "bilgi").unwrap();
    let kk = v.sorun_ac(n, "disk_dolu", "Disk dolu (%93)", "uyari").unwrap();
    v.sorun_sms(kk, "acilis_sms", "gonderildi").unwrap();
    k.o.saat.koy(t0 - 3600.0 - 1800.0);
    v.sorun_kapat(i).unwrap();
    v.sorun_kapat(j).unwrap(); // SMS'siz kapandı
    k.o.saat.koy(t0 - 100.0);
    v.sorun_kapat(kk).unwrap(); // açılışı gitmiş, 100 sn önce kapandı
    k.o.saat.koy(t0);
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(!h.contains("SMS bekleniyor") && !h.contains("09:00&#x27;da gidecek") && h.contains("düzeldi bekletiliyor"));
}

#[test]
fn yonetimden_cihaz_paneline_dogrudan_gecis() {
    let _k = Kayit::bas("test_yonetim", "test_yonetimden_cihaz_paneline_dogrudan_gecis");
    let tmp = y::gecici_dizin();
    let o = y::ortam_dizinde(&tmp);
    let (yol, dyol) = (tmp.join("yonetici.json"), tmp.join("durum.json"));
    yonetici::yaz(&yol, "serkan", PW).unwrap();
    std::fs::write(&dyol, y::json_govde(&json!({"zaman": "", "arsivler": {}}))).unwrap();
    let gecis = Arc::new(GecisBelirtecleri::new());
    let mu = Arc::new(Musteri::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat()).gecis_ile(gecis.clone()));
    let cz = Arc::new(crate::cihaz::Cihaz::new(o.veri.clone(), gecis.clone(), mu.clone(), o.saat.saat()));
    let (kuyruk, _) = kuyruk_kaydedici();
    let app = Yonetim::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat(), yol, dyol, kuyruk).gecis_ile(gecis, mu);
    let c = y::cerez(&y::giris(&app, "serkan", PW));
    let v = &o.veri;
    let (n, _) = v.musteri_ekle("", Some("musteri-parola-1")).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    // tünel adresi yok → düğme yok, geçiş reddedilir
    let f = y::al(&app, &format!("/m/{n}"), &c);
    let h = s(&f);
    assert!(!h.contains("Cihaz paneline git") && h.contains("şu an ulaşılamıyor"));
    let t = y::csrf(&f);
    assert_eq!(y::gonder(&app, &format!("/m/{n}/cihaz-paneli"), &c, &[("csrf", &t)]).durum, 409);
    v.cihaz_guncelle(v.bagli_cihaz(n).unwrap().unwrap().id, &[("tunel_ip", "10.99.0.11")]).unwrap();
    let h = s(&y::al(&app, &format!("/m/{n}"), &c));
    assert!(h.contains(&format!("action=\"/m/{n}/cihaz-paneli\"")) && h.contains("Cihaz paneline git"));
    assert_eq!(y::gonder(&app, &format!("/m/{n}/cihaz-paneli"), &c, &[("csrf", "yanlis")]).durum, 403);
    let r = y::gonder(&app, &format!("/m/{n}/cihaz-paneli"), &c, &[("csrf", &t)]);
    assert!(r.durum == 303 && konum(&r).starts_with("https://cihaz.wificorrect.com/_giris?t="));
    assert_eq!(har(v, Some(n))[0].olay, "YONETICI_CIHAZ_PANELI");
    let hh = ("Host", "cihaz.wificorrect.com");
    let r = y::istek(&*cz, "GET", "/_giris", &[hh], "", &format!("t={}", konum(&r).split_once("t=").unwrap().1), "1.2.3.4");
    let wcc = r.baslik("Set-Cookie").unwrap().split(';').next().unwrap().split_once('=').unwrap().1.to_string();
    let wc = format!("wcc={wcc}");
    let yt = y::istek(&*cz, "GET", "/cihaz-yetki", &[hh, ("Cookie", &wc)], "", "", "1.2.3.4");
    assert!(yt.durum == 200 && yt.baslik("X-WFC-Cihaz") == Some("10.99.0.11") && yt.baslik("X-WFC-Kullanici") == Some(n.to_string().as_str()));
    // müşterinin kendi cihaz oturumu yöneticinin çıkışıyla düşmez; yönetici müşteri sayfasına döner
    let ozet = v.musteri(n).unwrap().unwrap().ozet;
    let mw = cz.oturumlar.create(&n.to_string(), "musteri", o.saat.t(), &ozet, Default::default());
    let r = y::istek(&*cz, "GET", "/_cikis", &[hh, ("Cookie", &wc)], "", "", "1.2.3.4");
    assert!(r.durum == 303 && konum(&r) == format!("https://yonetim.wificorrect.com/m/{n}"));
    assert_eq!(y::istek(&*cz, "GET", "/cihaz-yetki", &[hh, ("Cookie", &wc)], "", "", "1.2.3.4").durum, 302);
    assert_eq!(y::istek(&*cz, "GET", "/cihaz-yetki", &[hh, ("Cookie", &format!("wcc={mw}"))], "", "", "1.2.3.4").durum, 200);
}

#[test]
fn musteri_guncelleme_karti_kanal_ve_hemen() {
    let _k = Kayit::bas("test_yonetim", "test_musteri_guncelleme_karti_kanal_ve_hemen");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    surum_ekle(v);
    v.kanal_hedef_koy("kararli", Some("1.2.0")).unwrap();
    let f = k.al(&format!("/m/{n}"));
    let (h, t) = (s(&f), y::csrf(&f));
    assert!(h.contains("Güncelleme") && h.contains(&format!("action=\"/m/{n}/guncelleme\"")) && h.contains(&format!("action=\"/m/{n}/guncelleme-hemen\"")));
    assert!(h.contains("Hedef sürüm") && h.contains("1.2.0"));
    // CSRF'siz POST reddedilir
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme"), &[("kanal", "test")]).durum, 403);
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme-hemen"), &[("csrf", "yanlis")]).durum, 403);
    let m = || v.musteri(n).unwrap().unwrap();
    assert!(m().guncelleme_kanali == "kararli" && m().guncelleme_hemen == 0.0);
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme"), &[("csrf", &t), ("kanal", "bilinmez")]).durum, 400);
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme"), &[("csrf", &t), ("kanal", "test")]).durum, 303);
    assert_eq!(m().guncelleme_kanali, "test");
    assert!(v.hareketler(Some(n), 300, None).unwrap().iter().any(|h| h.olay == "GUNCELLEME_KANALI" && h.musteri == Some(n) && h.ayrinti.contains("kanal=test")));
    v.guncelleme_kanali_koy(n, "kararli").unwrap();
    assert!(!s(&k.al(&format!("/m/{n}"))).contains("bekleniyor"));
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme-hemen"), &[("csrf", &t)]).durum, 303);
    assert!(m().guncelleme_hemen == k.o.saat.t() && m().guncelleme_hemen_surum == "1.2.0");
    assert!(olaylar(v, Some(n)).iter().any(|o| o == "GUNCELLEME_HEMEN"));
    assert!(s(&k.al(&format!("/m/{n}"))).contains("bekleniyor"));
}

#[test]
fn simdi_guncelle_hedef_yoksa_reddedilir() {
    let _k = Kayit::bas("test_yonetim", "test_simdi_guncelle_hedef_yoksa_reddedilir");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    surum_ekle(v);
    let t = y::csrf(&k.al(&format!("/m/{n}")));
    // kararlı kanalda hedef yok
    let r = k.gonder(&format!("/m/{n}/guncelleme-hemen"), &[("csrf", &t)]);
    assert!(r.durum == 400 && s(&r).contains("hedef"));
    assert_eq!(v.musteri(n).unwrap().unwrap().guncelleme_hemen, 0.0);
    // kanal kapalı
    v.kanal_hedef_koy("kararli", Some("1.2.0")).unwrap();
    v.guncelleme_kanali_koy(n, "kapali").unwrap();
    assert_eq!(k.gonder(&format!("/m/{n}/guncelleme-hemen"), &[("csrf", &t)]).durum, 400);
    assert_eq!(v.musteri(n).unwrap().unwrap().guncelleme_hemen, 0.0);
    assert!(!olaylar(v, Some(n)).iter().any(|o| o == "GUNCELLEME_HEMEN"));
}

#[test]
fn musteri_guncelleme_karti_paket_sonuc_gecmis() {
    let _k = Kayit::bas("test_yonetim", "test_musteri_guncelleme_karti_paket_sonuc_gecmis");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let (cid, _) = v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    assert!(!s(&k.al(&format!("/m/{n}"))).contains("Paketsiz kurulum"));
    v.saglik_kaydet(cid, &json!({"paket": ""})).unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Paketsiz kurulum — uzaktan güncellenmez"));
    v.saglik_kaydet(cid, &json!({"paket": "1.1.0", "guncelleme": {"surum": "1.2.0", "sonuc": "geri_alindi", "zaman": 1790000000}})).unwrap();
    for i in 0..12 {
        v.hareket(&format!("cihaz:{cid}"), "10.99.0.11", "GUNCELLEME_SONUC", Some(n), &format!("surum=1.0.{i} sonuc=hata")).unwrap();
    }
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Kurulu paket") && h.contains("1.1.0") && h.contains("Geri alındı"));
    assert!(h.matches("surum=1.0.").count() == 10 && h.contains("surum=1.0.11 ") && !h.contains("surum=1.0.1 "));
}

#[test]
fn liste_surum_sutunu_hedefle_farkli_sari() {
    let _k = Kayit::bas("test_yonetim", "test_liste_surum_sutunu_hedefle_farkli_sari");
    let k = kur();
    let v = k.v();
    let (n1, _) = v.musteri_ekle("", None).unwrap();
    let (c1, _) = v.cihaz_bagla(n1, WG, SSH, "", "", "").unwrap();
    let (n2, _) = v.musteri_ekle("", None).unwrap();
    let (c2, _) = v.cihaz_bagla(n2, WG2, SSH, "", "", "").unwrap();
    let (n3, _) = v.musteri_ekle("", None).unwrap();
    surum_ekle(v);
    v.kanal_hedef_koy("kararli", Some("1.2.0")).unwrap();
    v.saglik_kaydet(c1, &json!({"paket": "1.1.0"})).unwrap();
    v.saglik_kaydet(c2, &json!({"paket": "1.2.0"})).unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains("<th>Sürüm</th>"));
    let sat = |n: i64| {
        let i = h.find(&format!("<tr><td><a href=\"/m/{n}\">")).unwrap();
        h[i..i + h[i..].find("</tr>").unwrap()].to_string()
    };
    let (s1, s2, s3) = (sat(n1), sat(n2), sat(n3));
    assert!(s1.contains("class=\"uyarisi\">1.1.0") && !s2.contains("class=\"uyarisi\"") && s2.contains("1.2.0"));
    assert!(s3.contains('—') && !s3.contains("class=\"uyarisi\""));
}

#[test]
fn giris_sayfasi_hareketler_kayitlar_ve_cikis() {
    let _k = Kayit::bas("test_yonetim", "test_giris_sayfasi_hareketler_kayitlar_ve_cikis");
    let k = kur();
    let v = k.v();
    let r = y::al(&k.app, "/giris", "");
    assert!(r.durum == 200 && s(&r).contains("action=\"/giris\""));
    assert_eq!(konum(&y::al(&k.app, "/giris", &k.c)), "/");
    let (n, _) = v.musteri_ekle("Bocafe", None).unwrap();
    v.hareket("admin", "203.0.113.5", "DENEME", Some(n), "ayrıntı <1>").unwrap();
    let h = s(&k.al("/hareketler"));
    assert!(h.contains("DENEME") && h.contains("ayrıntı &lt;1&gt;"));
    let h = s(&k.al(&format!("/m/{n}/hareketler")));
    assert!(h.contains(&format!("Müşteri {n} · hareketler")) && h.contains("DENEME"));
    let r = k.al(&format!("/m/{n}/kayitlar"));
    assert!(r.durum == 200 && s(&r).contains(&format!("action=\"/m/{n}/kayitlar/ara\"")));
    let f = k.al("/hesabim");
    let r = k.gonder("/cikis", &[("csrf", &y::csrf(&f))]);
    assert!(r.durum == 303 && konum(&r) == "/giris" && r.baslik("Set-Cookie").unwrap().starts_with("wc_yonetim=; "));
    assert_eq!(konum(&k.al("/")), "/giris");
}

// ======================================================================================================================
// başka Python dosyalarındaki yönetim testleri
// ======================================================================================================================
#[test]
fn musteri_cerezi_yonetimde_gecmez() {
    let _k = Kayit::bas("test_merkez", "test_musteri_cerezi_yonetimde_gecmez");
    let tmp = y::gecici_dizin();
    let o = y::ortam_dizinde(&tmp);
    yonetici::yaz(&tmp.join("y.json"), "serkan", "yonetici-parola-1").unwrap();
    let (kuyruk, _) = kuyruk_kaydedici();
    let yonetim = Yonetim::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat(), tmp.join("y.json"), tmp.join("d.json"), kuyruk);
    let musteri = Musteri::new(o.veri.clone(), kayitlar_of(&o), o.saat.saat());
    let mut m = Merkez::new();
    m.ekle("yonetim", Arc::new(yonetim));
    m.ekle("musteri", Arc::new(musteri));
    let (n, pw) = o.veri.musteri_ekle("", None).unwrap();
    let r = y::istek(&m, "POST", "/giris", &[("Host", "panel.wificorrect.com")], &y::urlencode(&[("kullanici", &n.to_string()), ("sifre", &pw)]), "", "1.1.1.1");
    let c = y::cerez(&r);
    assert!(r.baslik("Set-Cookie").unwrap().starts_with("wc_musteri=") && r.baslik("Set-Cookie").unwrap().ends_with("; HttpOnly; Secure; SameSite=Strict; Path=/"));
    assert_eq!(y::istek(&m, "GET", "/", &[("Host", "panel.wificorrect.com"), ("Cookie", &c)], "", "", "1.1.1.1").durum, 200);
    let yn = y::istek(&m, "GET", "/", &[("Host", "yonetim.wificorrect.com"), ("Cookie", &c)], "", "", "1.1.1.1");
    assert_eq!((yn.durum, konum(&yn)), (303, "/giris".to_string()));
}

fn bekleme_kur() -> (at::D, Musteri) {
    let d = at::kur(Some(at::tamam()));
    let m = Musteri::new(d.v.clone(), Kayitlar::new(d.tmp.join("ist"), d.tmp.join("detay"), d.tmp.join("kisi_sir")), d.saat.saat());
    (d, m)
}

#[test]
fn yonetim_bekleyen_listesi() {
    let _k = Kayit::bas("test_bekleme", "test_yonetim_bekleyen_listesi");
    let (d, m) = bekleme_kur();
    let v = &d.v;
    let (n, pw) = v.musteri_ekle("", Some("musteri-parola-1")).unwrap();
    at::bildir_tam(&d, at::K1, at::S1, WG, at::IP1, "00:0e:c4:ce:a0:9b", "<b>");
    at::bildir_tam(&d, at::K2, at::S2, WG2, at::IP2, "00:0e:c4:ce:a0:9b", "0.4.13");
    let cm = y::cerez(&y::giris(&m, &n.to_string(), &pw));
    y::al(&m, "/baglan", &cm);
    d.saat.ilerlet(1801.0);
    at::bildir_tam(&d, at::K2, at::S2, WG2, at::IP2, "00:0e:c4:ce:a0:9b", "0.4.13");
    yonetici::yaz(&d.tmp.join("yonetici.json"), "admin", "yonetici-parola-1").unwrap();
    let (kuyruk, _) = kuyruk_kaydedici();
    let yn = Yonetim::new(v.clone(), Kayitlar::new(d.tmp.join("ist"), d.tmp.join("detay"), d.tmp.join("kisi_sir")), d.saat.saat(), d.tmp.join("yonetici.json"), d.tmp.join("durum.json"), kuyruk);
    let cz = y::cerez(&y::giris(&yn, "admin", "yonetici-parola-1"));
    let h = s(&y::al(&yn, "/bekleyen", &cz));
    assert!(h.contains("Bekleyen cihazlar") && h.contains(at::IP1) && h.contains(at::IP2) && h.contains("00:0e:c4:ce:a0:9b"));
    assert!(h.contains("&lt;b&gt;") && !h.contains("<b>")); // cihazın bildirdiği sürüm kaçışlı
    assert!(h.contains(&format!("/m/{n}")) && h.contains("eski"));
    assert!(s(&y::al(&yn, "/", &cz)).contains("Bekleyen cihazlar")); // menüde
}

fn raporlu_yonetim(tmp: &Path) -> K {
    let rp = Raporlar::new(tmp.join("rapor"));
    kur_dizinde(tmp, None, move |a| a.raporlar_ile(rp))
}

#[test]
fn yonetim_raporlar_gerekcesiz() {
    let _k = Kayit::bas("test_raporlar", "test_yonetim_raporlar_gerekcesiz");
    let tmp = y::gecici_dizin();
    let k = raporlu_yonetim(&tmp);
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains(&format!("href=\"/m/{n}/raporlar\"")));
    rr::yaz(&tmp, &n.to_string(), "liste.json", &Value::Array(rr::liste_ornegi()));
    rr::yaz(&tmp, &n.to_string(), "ay-2026-09.json", &rr::ay_raporu(&[]));
    let r = k.al(&format!("/m/{n}/raporlar"));
    assert!(r.durum == 200 && s(&r).contains(&format!("href=\"/m/{n}/rapor/ay/2026-09\"")));
    let r = k.al(&format!("/m/{n}/rapor/ay/2026-09"));
    assert!(r.durum == 200 && s(&r).contains(&format!("Müşteri {n}")) && s(&r).contains(&format!("href=\"/m/{n}/raporlar\"")));
    assert_eq!(k.al(&format!("/m/{n}/rapor/ay/2026-13")).durum, 404);
    if v.musteri(1000000).unwrap().is_none() {
        assert_eq!(k.al("/m/1000000/raporlar").durum, 404);
    }
    assert!(!har(v, Some(n)).iter().any(|h| h.olay == "GORUNTULEME" || h.olay == "GEREKCE"));
}

#[test]
fn yonetim_musteri_sayfasi_sms_sayaci() {
    let _k = Kayit::bas("test_sms_merkez", "test_yonetim_musteri_sayfasi_sms_sayaci");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    for _ in 0..7 {
        v.sms_sayac_artir(n, &v.bugun().to_string()).unwrap();
    }
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Bugün doğrulama SMS") && h.contains("<td>7</td>"));
}

#[test]
fn yonetim_musteri_sayfasi_son_sms() {
    let _k = Kayit::bas("test_sms_merkez", "test_yonetim_musteri_sayfasi_son_sms");
    let k = kur();
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Son doğrulama SMS"));
    v.sms_son_kaydet(n, "netgsm", "hata", "30").unwrap();
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("NetGSM · hata (30)"));
    v.sms_son_kaydet(n, "twilio", "gonderildi", "0").unwrap();
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains("Twilio · gönderildi") && !h.contains("(30)"));
}

#[test]
fn yonetim_listesinde_tur() {
    let _k = Kayit::bas("test_misafir_kodu", "test_yonetim_listesinde_tur");
    let k = kur();
    let v = k.v();
    let (n1, _) = v.musteri_ekle("Otel müşterisi", None).unwrap();
    let (n2, _) = v.musteri_ekle("Türsüz", None).unwrap();
    let (cid, _) = v.cihaz_bagla(n1, WG, SSH, "Deniz Otel", "", "").unwrap();
    v.eslesme_kaydet(cid, "Deniz Otel", "", "1.0", Some("otel"), None).unwrap();
    v.cihaz_bagla(n2, WG2, SSH, "Kafe X", "", "").unwrap();
    let h = s(&k.al("/"));
    assert!(h.contains("<th>Tür</th>") && h.contains("Otel/Pansiyon"));
    assert!(s(&k.al(&format!("/m/{n1}"))).contains("Otel/Pansiyon"));
    let s2 = s(&k.al(&format!("/m/{n2}")));
    assert!(s2.contains("İşletme türü") && !s2.contains("Otel/Pansiyon"));
    for (tur, etiket) in [("kafe", "Kafe/Restoran"), ("yurt", "Yurt"), ("diger", "Diğer")] {
        v.eslesme_kaydet(cid, "Deniz Otel", "", "1.0", Some(tur), None).unwrap();
        assert!(s(&k.al(&format!("/m/{n1}"))).contains(etiket));
    }
}

#[test]
fn yonetimde_de_post_arama_maskeli() {
    let _k = Kayit::bas("test_misafir_kodu", "test_yonetimde_de_post_arama_maskeli");
    let tmp = y::gecici_dizin();
    let k = kur_dizinde(&tmp, None, |a| a);
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    crate::musteri::testler::arsiv_yaz(&tmp.join("arsiv"));
    crate::ozet::detay_guncelle(&tmp.join("arsiv"), &tmp.join("detay").join(n.to_string()), None).unwrap();
    y::gerekce_ver(&k.app, &k.c);
    let on = format!("/m/{n}/kayitlar");
    let f = k.al(&format!("{on}/ara"));
    let h = s(&f);
    assert!(h.contains(&format!("action=\"{on}/ara\"")) && h.contains("method=\"post\""));
    let r = k.gonder(&format!("{on}/ara"), &[("csrf", &y::csrf(&f)), ("q", "10000000146")]);
    let h = s(&r);
    assert!(r.durum == 200 && h.contains("100******46") && h.contains("Deniz Kara"));
    crate::musteri::testler::sizmaz(&h.replace(&n.to_string(), ""));
    crate::musteri::testler::sizmaz(&s(&k.al(&format!("{on}/gun/{}", crate::musteri::testler::GUN))).replace(&n.to_string(), ""));
}

// ======================================================================================================================
// Kayıtsız (Python'da karşılığı yok): yönetimde haftalık rapor yolu
// ======================================================================================================================
#[test]
fn yonetimde_haftalik_rapor_yolu() {
    let tmp = y::gecici_dizin();
    let k = raporlu_yonetim(&tmp);
    let v = k.v();
    let (n, _) = v.musteri_ekle("", None).unwrap();
    rr::yaz(&tmp, &n.to_string(), "liste.json", &Value::Array(rr::liste_ornegi()));
    let hafta = rr::guncelle(
        rr::ay_raporu(&[]),
        &[("tur", json!("hafta")), ("donem", json!("2026-W38")), ("baslangic", json!("2026-09-14")), ("bitis", json!("2026-09-20")), ("gunler", Value::Array(rr::gunler("2026-09-14", 7)))],
    );
    rr::yaz(&tmp, &n.to_string(), "hafta-2026-W38.json", &hafta);
    // müşteri sayfasından liste, listeden hafta raporu
    let h = s(&k.al(&format!("/m/{n}")));
    assert!(h.contains(&format!("<a class=\"dugme\" href=\"/m/{n}/raporlar\">Raporlar</a>")));
    let r = k.al(&format!("/m/{n}/raporlar"));
    assert!(r.durum == 200 && s(&r).contains(&format!("href=\"/m/{n}/rapor/hafta/2026-W38\"")));
    let r = k.al(&format!("/m/{n}/rapor/hafta/2026-W38"));
    assert!(r.durum == 200 && s(&r).contains(&format!("Müşteri {n}")) && s(&r).contains(&format!("href=\"/m/{n}/raporlar\"")));
    // gerekçe istenmez, hareket yazılmaz
    assert!(!har(v, Some(n)).iter().any(|h| h.olay == "GORUNTULEME" || h.olay == "GEREKCE"));
    // 404'ler: geçersiz dönem, var olmayan dönem, bilinmeyen tür, POST, yok müşteri
    for yol in [
        format!("/m/{n}/rapor/hafta/2026-W54"),
        format!("/m/{n}/rapor/hafta/2026-W01"),
        format!("/m/{n}/rapor/hafta/../x"),
        format!("/m/{n}/rapor/yil/2026"),
        format!("/m/{n}/rapor/hafta/"),
    ] {
        assert_eq!(k.al(&yol).durum, 404, "{yol}");
    }
    assert_eq!(k.gonder(&format!("/m/{n}/rapor/hafta/2026-W38"), &[("csrf", &y::csrf(&k.al("/hesabim")))]).durum, 404);
    if v.musteri(1000000).unwrap().is_none() {
        assert_eq!(k.al("/m/1000000/rapor/hafta/2026-W38").durum, 404);
    }
}

// ======================================================================================================================
// Task 11b: ayar sayfaları (SMS, uyarı, sürümler, engel listeleri). Dış istek yok: SahteAg / SahteIndirme / sahte github_al.
// ======================================================================================================================

/// Python `dict(taban, **ek)`: aynı anahtarlar `ek`'tekiyle değişir.
fn birlestir<'a>(taban: &[(&'a str, &'a str)], ek: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut v: Vec<(&str, &str)> = taban.iter().filter(|(k, _)| !ek.iter().any(|(e, _)| e == k)).cloned().collect();
    v.extend_from_slice(ek);
    v
}

/// Python `re.search(r'<select id="k-KANAL".*?</select>', s, re.S).group(0)`.
fn secim_kutusu(h: &str, kanal: &str) -> String {
    let bas = h.find(&format!("<select id=\"k-{kanal}\"")).unwrap();
    h[bas..bas + h[bas..].find("</select>").unwrap() + "</select>".len()].to_string()
}

#[test]
fn sms_ayarlari_sayfasi() {
    let _k = Kayit::bas("test_yonetim", "test_sms_ayarlari_sayfasi");
    let k = kur();
    let v = k.v();
    let h = s(&k.al("/sms"));
    assert!(h.contains("name=\"usercode\"") && h.contains("name=\"password\""));
    let t = y::csrf(&k.al("/sms"));
    // deneme modu kapalı, şifre yok
    let r = k.gonder("/sms", &[("csrf", &t), ("usercode", "850"), ("msgheader", "BASLIK")]);
    assert!(r.durum == 400 && s(&r).contains("password") && v.sms_ayari().unwrap().is_none());
    k.gonder("/sms", &[("csrf", &t), ("usercode", "8503027084"), ("password", "gizli-1"), ("msgheader", "gztp.blgsyr")]);
    let a = v.sms_ayari().unwrap().unwrap();
    assert!(a["password"] == "gizli-1" && a["mock"] == false);
    let h = s(&k.al("/sms"));
    assert!(!h.contains("gizli-1") && h.contains("tanımlı")); // şifre geri gösterilmez
    k.gonder("/sms", &[("csrf", &t), ("usercode", "8503027084"), ("password", ""), ("msgheader", "gztp.blgsyr"), ("mock", "1")]);
    let a = v.sms_ayari().unwrap().unwrap();
    assert!(a["password"] == "gizli-1" && a["mock"] == true); // boş şifre = aynı kalır
    assert!(har(v, None).iter().any(|h| h.olay == "SMS_AYARI" && !h.ayrinti.contains("gizli-1")));
}

#[test]
fn yonetim_twilio_ayarlari() {
    let _k = Kayit::bas("test_sms_merkez", "test_yonetim_twilio_ayarlari");
    let k = kur();
    let v = k.v();
    let h = s(&k.al("/sms"));
    for alan in ["tr_saglayici", "twilio_acik", "account_sid", "auth_token", "api_key_sid", "api_key_secret", "verify_sid"] {
        assert!(h.contains(&format!("name=\"{alan}\"")), "{alan}");
    }
    let t = y::csrf(&k.al("/sms"));
    let (hesap, dogrulama) = (format!("AC{}", "a".repeat(32)), format!("VA{}", "b".repeat(32)));
    let form: Vec<(&str, &str)> = vec![
        ("csrf", &t),
        ("usercode", "850"),
        ("password", "netgsm-gizli"),
        ("msgheader", "B"),
        ("tr_saglayici", "twilio"),
        ("twilio_acik", "1"),
        ("account_sid", &hesap),
        ("auth_token", "tw-token-gizli"),
        ("api_key_sid", ""),
        ("api_key_secret", ""),
        ("verify_sid", &dogrulama),
    ];
    assert_eq!(k.gonder("/sms", &form).durum, 303);
    let a = v.sms_ayari().unwrap().unwrap();
    assert!(a["tr_saglayici"] == "twilio" && a["twilio"]["acik"] == true && a["twilio"]["auth_token"] == "tw-token-gizli");
    let sayfa = s(&k.al("/sms"));
    assert!(!sayfa.contains("tw-token-gizli") && !sayfa.contains("netgsm-gizli") && sayfa.contains(&hesap));
    let r = k.gonder("/sms", &birlestir(&form, &[("auth_token", ""), ("password", "")])); // boş sır = aynı kalır
    assert!(r.durum == 303 && v.sms_ayari().unwrap().unwrap()["twilio"]["auth_token"] == "tw-token-gizli");
    assert_eq!(v.sms_ayari().unwrap().unwrap()["password"], "netgsm-gizli");
    let sk = format!("SK{}", "d".repeat(32));
    let r = k.gonder("/sms", &birlestir(&form, &[("api_key_secret", "sk-yeni-gizli"), ("api_key_sid", &sk)]));
    assert!(r.durum == 303 && v.sms_ayari().unwrap().unwrap()["twilio"]["api_key_secret"] == "sk-yeni-gizli");
    assert!(!s(&k.al("/sms")).contains("sk-yeni-gizli"));
    assert!(har(v, None).iter().all(|h| ["tw-token-gizli", "sk-yeni-gizli", "netgsm-gizli"].iter().all(|z| !h.ayrinti.contains(z))));
    let onceki = v.sms_ayari().unwrap();
    let csrfsiz: Vec<(&str, &str)> = form.iter().filter(|(k, _)| *k != "csrf").cloned().collect();
    for f in [birlestir(&csrfsiz, &[("auth_token", "baska")]), birlestir(&csrfsiz, &[("csrf", "yanlis"), ("auth_token", "baska")])] {
        assert_eq!(k.gonder("/sms", &f).durum, 403);
    }
    assert_eq!(v.sms_ayari().unwrap(), onceki);
    let r = k.gonder("/sms", &birlestir(&form, &[("verify_sid", "kotu")]));
    assert!(r.durum == 400 && v.sms_ayari().unwrap() == onceki);
}

#[test]
fn uyari_ayarlari_kaydet_ve_dogrula() {
    let _k = Kayit::bas("test_yonetim", "test_uyari_ayarlari_kaydet_ve_dogrula");
    let k = kur();
    let v = k.v();
    let h = s(&k.al("/uyari"));
    assert!(h.contains("name=\"tel1\"") && h.contains("name=\"tel3\"") && h.contains("name=\"acik\"") && h.contains("name=\"gunluk_sinir\""));
    assert!(h.contains("href=\"/uyari\"") && h.contains("Uyarı ayarları"));
    let t = y::csrf(&k.al("/uyari"));
    let r = k.gonder("/uyari", &[("csrf", &t), ("tel1", "0533 111 22 33"), ("tel2", ""), ("tel3", "05441112244"), ("acik", "1"), ("gunluk_sinir", "15")]);
    assert_eq!(r.durum, 303);
    assert_eq!(v.uyari_ayari().unwrap(), json!({"telefonlar": ["905331112233", "905441112244"], "acik": true, "gunluk_sinir": 15}));
    let olay: Vec<Hareket> = har(v, None).into_iter().filter(|h| h.olay == "UYARI_AYAR").collect();
    assert!(!olay.is_empty() && !olay[0].ayrinti.contains("5331112233"));
    let h = s(&k.al("/uyari"));
    assert!(h.contains("5331112233") && h.contains("checked") && h.contains("value=\"15\""));
    let r = k.gonder("/uyari", &[("csrf", &t), ("tel1", "123"), ("gunluk_sinir", "15")]); // geçersiz telefon
    assert!(r.durum == 400 && s(&r).contains("Geçersiz telefon") && v.uyari_ayari().unwrap()["gunluk_sinir"] == 15);
    for kotu in ["0", "1", "101", "abc", "", "2.5"] {
        let r = k.gonder("/uyari", &[("csrf", &t), ("tel1", "05331112233"), ("gunluk_sinir", kotu)]);
        assert_eq!(r.durum, 400, "{kotu}");
    }
    assert_eq!(v.uyari_ayari().unwrap()["gunluk_sinir"], 15);
    let r = k.gonder("/uyari", &[("csrf", &t), ("tel1", "05331112233"), ("gunluk_sinir", "20")]); // onay kutusu işaretsiz = kapalı
    assert!(r.durum == 303 && v.uyari_ayari().unwrap()["acik"] == false);
    let r = k.gonder("/uyari", &[("csrf", &t), ("tel1", "05331112233"), ("gunluk_sinir", "2")]); // en az 2
    assert!(r.durum == 303 && v.uyari_ayari().unwrap()["gunluk_sinir"] == 2);
}

#[test]
fn deneme_sms_sahte_gondericiyle() {
    let _k = Kayit::bas("test_yonetim", "test_deneme_sms_sahte_gondericiyle");
    let ag = y::SahteAg::yeni(vec![]);
    let k = kur_ek(None, |a| a.sms_tasiyici_ile(ag.clone()));
    let v = k.v();
    let t = y::csrf(&k.al("/uyari"));
    v.uyari_ayari_koy(&json!({"telefonlar": ["05331112233", "05441112244"], "acik": true, "gunluk_sinir": 20})).unwrap();
    let r = k.gonder("/uyari/deneme", &[("csrf", &t)]); // SMS ayarı yok
    assert!(r.durum == 200 && s(&r).contains("SMS ayarları girilmemiş") && ag.sayi() == 0);
    v.sms_ayari_koy(&json!({"mock": false, "usercode": "u", "password": "p", "msgheader": "h"})).unwrap();
    let r = k.gonder("/uyari/deneme", &[("csrf", &t)]);
    assert!(r.durum == 200 && s(&r).contains("gönderildi"));
    assert_eq!(ag.sayi(), 1);
    // yalnızca ilk telefona, sabit metinle, kayıtlı kullanıcı koduyla
    ag.son(|i| {
        assert!(i.govde.contains("WifiCorrect deneme uyarısı") && i.govde.contains("<no>5331112233</no>") && !i.govde.contains("5441112244"));
        assert!(i.govde.contains("<usercode>u</usercode>"));
    });
    let gun = now_iso(k.o.saat.t())[..10].to_string();
    assert_eq!(v.sms_sayaci(&gun).unwrap(), 1);
    assert_eq!(har(v, None)[0].olay, "UYARI_DENEME");
    v.uyari_ayari_koy(&json!({"telefonlar": [], "acik": true, "gunluk_sinir": 20})).unwrap();
    let r = k.gonder("/uyari/deneme", &[("csrf", &t)]);
    assert!(s(&r).contains("telefon") && ag.sayi() == 1);
}

#[test]
fn uyari_csrf_siz_post_reddi() {
    let _k = Kayit::bas("test_yonetim", "test_uyari_csrf_siz_post_reddi");
    let ag = y::SahteAg::yeni(vec![]);
    let k = kur_ek(None, |a| a.sms_tasiyici_ile(ag.clone()));
    let v = k.v();
    v.uyari_ayari_koy(&json!({"telefonlar": ["05331112233"], "acik": true, "gunluk_sinir": 20})).unwrap();
    v.sms_ayari_koy(&json!({"mock": false, "usercode": "u", "password": "p", "msgheader": "h"})).unwrap();
    let uyari: Vec<(&str, &str)> = vec![("tel1", "05441112244"), ("gunluk_sinir", "5")];
    for (yol, alan) in [("/uyari", uyari), ("/uyari/deneme", vec![])] {
        for form in [alan.clone(), birlestir(&alan, &[("csrf", "yanlis")])] {
            assert_eq!(k.gonder(yol, &form).durum, 403, "{yol}");
        }
    }
    assert!(ag.sayi() == 0 && v.uyari_ayari().unwrap()["gunluk_sinir"] == 20 && v.sms_sayaci("2026-09-21").unwrap() == 0);
}

#[test]
fn surumler_yonetici_disina_kapali_ve_tablo() {
    let _k = Kayit::bas("test_yonetim", "test_surumler_yonetici_disina_kapali_ve_tablo");
    let k = kur();
    let v = k.v();
    assert!([302, 303, 401, 403].contains(&y::al(&k.app, "/surumler", "").durum));
    let (n, _) = v.musteri_ekle("", None).unwrap();
    let (cid, _) = v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    surum_ekle(v);
    v.saglik_kaydet(cid, &json!({"paket": "1.2.0"})).unwrap();
    let h = s(&k.al("/surumler"));
    assert!(h.contains("1.2.0") && h.contains("github") && h.contains(&"ab".repeat(6)) && !h.contains(&"ab".repeat(7)));
    assert!(h.contains("href=\"/surumler\""));
    let i = h.find("<tr><td>1.2.0</td>").unwrap();
    let satir = &h[i..i + h[i..].find("</tr>").unwrap()];
    assert!(satir.contains("<td>1</td>")); // kurulu cihaz sayısı
}

#[test]
fn surumler_github_al_mesaji_ve_hareket() {
    let _k = Kayit::bas("test_yonetim", "test_surumler_github_al_mesaji_ve_hareket");
    let tmp = y::gecici_dizin();
    let sd = tmp.join("sd");
    let cagri: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let c2 = cagri.clone();
    // Python `sahte(v_, dizin, getir)`: depoya 1.3.0'ı ekler, bir sürümü alamaz
    let k = kur_dizinde(&tmp, None, |a| {
        a.surum_dizini_ile(sd.clone()).github_al_ile(Box::new(move |v, dizin, _| {
            c2.lock().unwrap().push(dizin.to_path_buf());
            v.surum_ekle("1.3.0", &"cd".repeat(32), 10, "github").map_err(|h| h.to_string())?;
            Ok((vec!["1.3.0".to_string()], vec!["1.2.9: SHA256 tutmuyor".to_string()]))
        }))
    });
    let v = k.v();
    let t = y::csrf(&k.al("/surumler"));
    assert!(k.gonder("/surumler/al", &[("csrf", "yanlis")]).durum == 403 && cagri.lock().unwrap().is_empty());
    let r = k.gonder("/surumler/al", &[("csrf", &t)]);
    let h = s(&r);
    assert!(r.durum == 200 && h.contains("1.3.0") && h.contains("SHA256 tutmuyor") && *cagri.lock().unwrap() == vec![sd.clone()]);
    assert!(v.surum("1.3.0").unwrap().is_some());
    assert!(har(v, None).iter().any(|h| h.olay == "SURUM_ALINDI" && h.ayrinti.contains("surum=1.3.0")));
}

#[test]
fn surumler_github_listesi_alinamazsa_hata_mesaji() {
    let _k = Kayit::bas("test_yonetim", "test_surumler_github_listesi_alinamazsa_hata_mesaji");
    let k = kur_ek(None, |a| a.github_al_ile(Box::new(|_, _, _| Err("ag yok".to_string()))));
    let t = y::csrf(&k.al("/surumler"));
    let r = k.gonder("/surumler/al", &[("csrf", &t)]);
    assert!(r.durum == 200 && s(&r).to_lowercase().contains("alınamadı"));
}

#[test]
fn surumler_hedef_secimi() {
    let _k = Kayit::bas("test_yonetim", "test_surumler_hedef_secimi");
    let k = kur();
    let v = k.v();
    surum_ekle(v);
    let t = y::csrf(&k.al("/surumler"));
    assert_eq!(k.gonder("/surumler/hedef", &[("csrf", "yanlis"), ("kanal", "test"), ("surum", "1.2.0")]).durum, 403);
    let r = k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "test"), ("surum", "1.2.0")]);
    assert!(r.durum == 303 && v.kanal_hedef("test").unwrap().as_deref() == Some("1.2.0"));
    assert!(har(v, None).iter().any(|h| h.olay == "KANAL_HEDEF" && h.ayrinti.contains("kanal=test") && h.ayrinti.contains("surum=1.2.0")));
    assert!(s(&k.al("/surumler")).contains("value=\"1.2.0\" selected"));
    // geçersiz sürüm / kanal reddedilir, değişmez
    assert_eq!(k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "test"), ("surum", "9.9.9")]).durum, 400);
    assert_eq!(k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "kapali"), ("surum", "1.2.0")]).durum, 400);
    assert_eq!(k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "kararli"), ("surum", "../x")]).durum, 400);
    assert!(v.kanal_hedef("test").unwrap().as_deref() == Some("1.2.0") && v.kanal_hedef("kararli").unwrap().is_none());
    // boş = hedef yok
    assert_eq!(k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "test"), ("surum", "")]).durum, 303);
    assert!(v.kanal_hedef("test").unwrap().is_none());
}

#[test]
fn surumler_her_zaman_en_son() {
    let _k = Kayit::bas("test_yonetim", "test_surumler_her_zaman_en_son");
    let k = kur();
    let v = k.v();
    v.surum_ekle("1.3.0", &"cd".repeat(32), 10, "yerel").unwrap(); // elle eklenen deneme paketi "en son" sayılmaz
    let h = s(&k.al("/surumler"));
    assert!(h.contains("<option value=\"en-son\">Her zaman en son sürüm (GitHub sürümü yok)</option>"));
    surum_ekle(v); // GitHub'dan 1.2.0
    let f = k.al("/surumler");
    let t = y::csrf(&f);
    assert!(s(&f).contains("Her zaman en son sürüm (şu an 1.2.0)"));
    let r = k.gonder("/surumler/hedef", &[("csrf", &t), ("kanal", "kararli"), ("surum", "en-son")]);
    assert!(r.durum == 303 && v.kanal_hedef("kararli").unwrap().as_deref() == Some("en-son") && v.kanal_hedef("test").unwrap().is_none());
    assert!(har(v, None).iter().any(|h| h.olay == "KANAL_HEDEF" && h.ayrinti.contains("kanal=kararli surum=en-son")));
    let h = s(&k.al("/surumler"));
    let kararli = secim_kutusu(&h, "kararli");
    assert!(kararli.contains("<option value=\"en-son\" selected>") && !kararli.replace("value=\"en-son\" selected", "").contains("selected"));
    assert!(!secim_kutusu(&h, "test").contains("<option value=\"en-son\" selected>"));
    // müşteri sayfası çözülmüş sürümü gösterir; yeni GitHub sürümü gelince hedef kendiliğinden ona geçer
    let (n, _) = v.musteri_ekle("", None).unwrap();
    v.cihaz_bagla(n, WG, SSH, "", "", "").unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Hedef sürüm</th><td>1.2.0<"));
    v.surum_ekle("1.4.0", &"ef".repeat(32), 10, "github").unwrap();
    assert!(s(&k.al(&format!("/m/{n}"))).contains("Hedef sürüm</th><td>1.4.0<"));
}

#[test]
fn yonetim_listeler_sayfasi_ve_simdi_guncelle() {
    let _k = Kayit::bas("test_listeler", "test_yonetim_listeler_sayfasi_ve_simdi_guncelle");
    let tmp = y::gecici_dizin();
    let d = tmp.join("listeler");
    let uretici = Uretici {
        kaynaklar: vec![
            ("kumar", vec![listeler::Kaynak { ad: "HaGeZi kumar", url: "https://a", bicim: listeler::Bicim::Duz }]),
            ("yetiskin", vec![listeler::Kaynak { ad: "B", url: "https://b", bicim: listeler::Bicim::Duz }]),
            ("zararli", vec![listeler::Kaynak { ad: "USOM", url: "https://c", bicim: listeler::Bicim::Usom }]),
        ],
        en_cok: listeler::EN_COK,
    };
    let ind = Arc::new(y::SahteIndirme::yeni(
        [
            ("https://a".to_string(), Ok(b"a.com\nb.com\n".to_vec())),
            ("https://b".to_string(), Ok(b"x.com\n".to_vec())),
            ("https://c".to_string(), Err("ag yok".to_string())),
        ]
        .into_iter()
        .collect(),
    ));
    let (ind2, d2) = (ind.clone(), d.clone());
    let k = kur_dizinde(&tmp, None, move |a| a.liste_uretici_ile(uretici).liste_dizini_ile(d2).indirme_tasiyici_ile(ind2));
    let v = k.v();
    assert!([302, 303, 401, 403].contains(&y::al(&k.app, "/listeler", "").durum));
    let h = s(&k.al("/listeler"));
    assert!(h.contains("href=\"/listeler\"") && h.contains("Kumar ve bahis") && h.contains("Zararlı ve dolandırıcı siteler"));
    let t = y::csrf(&k.al("/listeler"));
    assert!(k.gonder("/listeler/guncelle", &[("csrf", "yanlis")]).durum == 403 && ind.cagrilar().is_empty());
    let r = k.gonder("/listeler/guncelle", &[("csrf", &t)]);
    let mut cagri = ind.cagrilar();
    cagri.sort();
    assert!([200, 303].contains(&r.durum) && cagri == ["https://a", "https://b", "https://c"]);
    assert!(d.join("kumar.txt.gz").exists());
    let h = s(&k.al("/listeler"));
    assert!(h.contains("HaGeZi kumar") && h.contains("ag yok") && h.contains("USOM"));
    let ol: Vec<String> = olaylar(v, None);
    assert!(ol.iter().any(|o| o == "LISTE_GUNCEL") && ol.iter().any(|o| o == "LISTE_HATA"));
}

#[test]
fn simdi_guncelle_mesgulse_beklemez() {
    let _k = Kayit::bas("test_listeler", "test_simdi_guncelle_mesgulse_beklemez");
    // Python kilit sınıfını değiştirir; burada gerçek flock tutulur. Kilit dosyası altın kayıttaki geçici dizinlerin dışında
    // (Python'da liste dizini hiç oluşmaz), yoksa dosya listesi farklı çıkardı.
    let kilit_dizini = crate::ortak::tests::gecici_dizin();
    let kilit = std::fs::OpenOptions::new().create(true).append(true).open(kilit_dizini.join(".kilit")).unwrap();
    // SAFETY: fd geçerli ve bize ait
    assert_eq!(unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&kilit), libc::LOCK_EX | libc::LOCK_NB) }, 0);
    let ind = Arc::new(y::SahteIndirme::yeni(Default::default()));
    let (ind2, d2) = (ind.clone(), kilit_dizini.clone());
    let k = kur_ek(None, move |a| a.liste_dizini_ile(d2).indirme_tasiyici_ile(ind2));
    let t = y::csrf(&k.al("/listeler"));
    let r = k.gonder("/listeler/guncelle", &[("csrf", &t)]);
    assert!(r.durum == 200 && s(&r).contains("zaten sürüyor") && ind.cagrilar().is_empty());
    drop(kilit);
    let _ = std::fs::remove_dir_all(&kilit_dizini);
}

/// Python karşılığı yok: gerçek `surumler::github_al` Yonetim üzerinden sahte indirmeyle (liste alınamıyor → hata satırı).
#[test]
fn surum_al_gercek_islevle_sahte_indirme() {
    let ind = Arc::new(y::SahteIndirme::yeni([(surumler::GITHUB_LISTE.to_string(), Err("ag yok".to_string()))].into_iter().collect()));
    let k = kur_ek(None, |a| a.indirme_tasiyici_ile(ind.clone()));
    let t = y::csrf(&k.al("/surumler"));
    let r = k.gonder("/surumler/al", &[("csrf", &t)]);
    assert!(r.durum == 200 && s(&r).contains("GitHub sürüm listesi alınamadı: ag yok"));
    assert_eq!(ind.cagrilar(), [surumler::GITHUB_LISTE]);
    let h = har(k.v(), None);
    assert!(h.iter().any(|h| h.olay == "SURUM_AL_HATA" && h.ayrinti == "ag yok"));
}
