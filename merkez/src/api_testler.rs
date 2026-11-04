//! api.rs testleri: Python tests/test_api.py (27'den 23'ü; raporlar uçları spec bilinçli fark 1 yüzünden, ALTER testi
//! test_misafir_kodu'nda taşınmadı), test_bekleme.py'nin yalnızca `api` ve `veri` ile çalışan 5 testi (öbürleri Musteri /
//! Yonetim uygulamasını kullanır: Task 10/11) ve test_misafir_kodu.py'nin `test_eslesme_turu_kaydeder`'i.
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{self as Y, SahteSaat};

pub(crate) const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
pub(crate) const WG2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
pub(crate) const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI cihaz";
const IP: &str = "198.51.100.7";

pub(crate) fn tamam() -> Value {
    json!({"durum": "tamam", "tunel_ip": "10.99.0.10", "sunucu_pub": format!("{}=", "S".repeat(43)), "uc_nokta": "vpn.wificorrect.com:51820"})
}

/// Python testlerindeki `Kuyruk` sahtesi: işleri kaydeder, `bekle` sabit sonucu verir.
pub(crate) struct Kuyruk {
    isler: Mutex<Vec<(String, Value)>>,
    sonuc: Mutex<Option<Value>>,
}

impl Kuyruk {
    pub(crate) fn isler(&self) -> Vec<(String, Value)> {
        self.isler.lock().unwrap().clone()
    }

    pub(crate) fn sonuc_koy(&self, s: Option<Value>) {
        *self.sonuc.lock().unwrap() = s;
    }
}

pub(crate) struct D {
    pub(crate) api: Api,
    pub(crate) v: Arc<Veri>,
    pub(crate) q: Arc<Kuyruk>,
    pub(crate) saat: SahteSaat,
    pub(crate) tmp: PathBuf,
    /// Python `rapor_yolla`'nın `_ip` sayacı (test başına).
    pub(crate) ip_sayac: Cell<u32>,
}

/// Python `kur(tmp, sonuc)`.
pub(crate) fn kur(sonuc: Option<Value>) -> D {
    let o = Y::ortam();
    let q = Arc::new(Kuyruk { isler: Mutex::new(Vec::new()), sonuc: Mutex::new(sonuc) });
    let (q1, q2) = (q.clone(), q.clone());
    let mut api = Api::new(
        o.veri.clone(),
        o.saat.saat(),
        Box::new(move |islem, alanlar| {
            let mut l = q1.isler.lock().unwrap();
            l.push((islem.to_string(), alanlar.clone()));
            Ok(format!("{:016x}", l.len()))
        }),
        Box::new(move |_| q2.sonuc.lock().unwrap().clone()),
    );
    api.liste_dizini = o.dizin.join("listeler");
    D { api, v: o.veri, q, saat: o.saat, tmp: o.dizin, ip_sayac: Cell::new(0) }
}

/// Python `baska(n)`: n'den farklı, geçerli biçimde ama kayıtlı olmayan numara.
fn baska(n: i64) -> i64 {
    if n != 1234567 {
        1234567
    } else {
        7654321
    }
}

fn post_ham(d: &D, yol: &str, govde: &str, ip: &str) -> (u16, Value) {
    let y = Y::istek(&d.api, "POST", yol, &[], govde, "", ip);
    (y.durum, serde_json::from_slice(&y.govde).expect("JSON cevap"))
}

pub(crate) fn post_ip(d: &D, yol: &str, govde: &Value, ip: &str) -> (u16, Value) {
    post_ham(d, yol, &Y::json_govde(govde), ip)
}

pub(crate) fn post(d: &D, yol: &str, govde: &Value) -> (u16, Value) {
    post_ip(d, yol, govde, IP)
}

fn giris_ip(d: &D, n: impl ToString, pw: &str, wg: &str, ip: &str) -> (u16, Value) {
    post_ip(
        d,
        "/api/giris",
        &json!({"numara": n.to_string(), "parola": pw, "wg_pub": wg, "ssh_pub": SSH, "isletme_adi": "Bocafe", "unvan": "Bocafe Ltd.", "surum": "1.0"}),
        ip,
    )
}

fn giris(d: &D, n: impl ToString, pw: &str) -> (u16, Value) {
    giris_ip(d, n, pw, WG, IP)
}

fn musteri(d: &D) -> (i64, String) {
    d.v.musteri_ekle("", None).unwrap()
}

pub(crate) fn mus(d: &D, n: i64) -> Musteri {
    d.v.musteri(n).unwrap().unwrap()
}

pub(crate) fn olaylar(d: &D, n: Option<i64>) -> Vec<String> {
    d.v.hareketler(n, 300, None).unwrap().into_iter().map(|h| h.olay).collect()
}

pub(crate) fn kod(j: &Value) -> &str {
    j["kod"].as_str().unwrap_or("")
}

fn anahtar(j: &Value) -> String {
    j["cihaz_anahtari"].as_str().unwrap().to_string()
}

fn rapor() -> Value {
    json!({"surum": "1.2.3", "acik_sn": 5000, "servisler": {"portal": true, "panel": true, "kaydedici": true, "dnsmasq": true, "guvenlik": false},
           "disk_yuzde": 42, "son_muhur": "2026-10-10", "kayit_boslugu_1s": false, "wan": true, "sms_sorunu": false, "bagli": 7, "hiz_hata": false})
}

/// Python `dict(RAPOR, alan=değer, ...)`.
fn rapor_ile(ek: &[(&str, Value)]) -> Value {
    let mut r = rapor();
    for (k, v) in ek {
        r.as_object_mut().unwrap().insert(k.to_string(), v.clone());
    }
    r
}

/// Python `rapor_yolla`: her çağrı ayrı IP'den (istek hız sınırına takılmasın).
fn rapor_yolla(d: &D, k: &str, ek: &[(&str, Value)]) -> (u16, Value) {
    d.ip_sayac.set(d.ip_sayac.get() + 1);
    post_ip(d, "/api/saglik", &json!({"cihaz_anahtari": k, "rapor": rapor_ile(ek)}), &format!("203.0.113.{}", d.ip_sayac.get()))
}

fn yaz(yol: &std::path::Path, icerik: &str) {
    std::fs::write(yol, icerik).unwrap();
}

#[test]
fn ilk_giris_baglar_ve_kuyruga_atar() {
    let _k = Kayit::bas("test_api", "test_ilk_giris_baglar_ve_kuyruga_atar");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (c, j) = giris(&d, n, &pw);
    assert!(c == 200 && j["tunel_ip"] == "10.99.0.10" && j["yedek_hedefi"] == format!("wfc-{n}@10.99.0.1:"));
    assert!(j["uc_nokta"] == "vpn.wificorrect.com:51820" && j["yineleme"] == 120000 && j["cihaz_anahtari"].as_str().unwrap().len() > 30);
    let m = mus(&d, n);
    assert_eq!((j["tuz"].as_str().unwrap(), j["ozet"].as_str().unwrap()), (m.tuz.as_str(), m.ozet.as_str()));
    assert_eq!(d.q.isler(), vec![("cihaz-ekle".to_string(), json!({"numara": n.to_string(), "wg_pub": WG, "ssh_pub": SSH}))]);
    let c = d.v.bagli_cihaz(n).unwrap().unwrap();
    assert!(c.tunel_ip == "10.99.0.10" && c.isletme_adi == "Bocafe");
    assert_eq!(olaylar(&d, Some(n)), ["CIHAZ_BAGLANDI"]);
}

#[test]
fn hatali_parola_kilit_ve_gecersiz_girdi() {
    let _k = Kayit::bas("test_api", "test_hatali_parola_kilit_ve_gecersiz_girdi");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    assert_eq!(kod(&giris(&d, n, "yanlis-parola").1), "hatali");
    assert_eq!(kod(&giris(&d, baska(n), "yanlis-parola").1), "hatali");
    assert_eq!(kod(&giris(&d, 100001, "yanlis-parola").1), "gecersiz"); // 6 hane
    for _ in 0..4 {
        giris_ip(&d, n, "yanlis-parola", WG, "198.51.100.8");
    }
    assert_eq!(giris(&d, n, &pw).0, 429); // numara kilitli
    assert_eq!(post_ham(&d, "/api/giris", "{bozuk", IP).0, 400);
    assert_eq!(kod(&post(&d, "/api/giris", &json!({"numara": "abc", "parola": "x"})).1), "gecersiz");
    let (_, j) = post(&d, "/api/giris", &json!({"numara": baska(n).to_string(), "parola": "x".repeat(10), "wg_pub": "kotu", "ssh_pub": SSH}));
    assert_eq!(kod(&j), "hatali");
    assert_eq!(Y::istek(&d.api, "GET", "/api/giris", &[], "", "", "1.1.1.1").durum, 404);
    assert_eq!(Y::istek(&d.api, "POST", "/baska", &[], "{}", "", "1.1.1.1").durum, 404);
    assert_eq!(Y::istek(&d.api, "POST", "/api/giris", &[], &"x".repeat(5000), "", "1.1.1.1").durum, 413);
    assert!(d.q.isler().is_empty());
}

#[test]
fn baska_cihaz_409_ve_ayni_cihaz_yeniden_dener() {
    let _k = Kayit::bas("test_api", "test_baska_cihaz_409_ve_ayni_cihaz_yeniden_dener");
    let d = kur(None); // kuyruk zaman aşımı
    let (n, pw) = musteri(&d);
    let (c, j) = giris(&d, n, &pw);
    assert!(c == 503 && kod(&j) == "kayit");
    d.q.sonuc_koy(Some(tamam()));
    let (c, j1) = giris(&d, n, &pw); // aynı cihaz yeniden
    assert_eq!(c, 200);
    let (c, j2) = giris(&d, n, &pw);
    assert!(c == 200 && j2["cihaz_anahtari"] != j1["cihaz_anahtari"]);
    assert_eq!(kod(&post(&d, "/api/eslesme", &json!({"cihaz_anahtari": j1["cihaz_anahtari"]})).1), "taninmadi");
    let (c, j) = giris_ip(&d, n, &pw, WG2, IP);
    assert!(c == 409 && kod(&j) == "baska_cihaz");
}

#[test]
fn uyelik_bitti_giris_yok() {
    let _k = Kayit::bas("test_api", "test_uyelik_bitti_giris_yok");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    d.v.uyelik(n, "bitti").unwrap();
    assert_eq!(kod(&giris(&d, n, &pw).1), "uyelik");
    d.v.uyelik(n, "aktif").unwrap();
    assert_eq!(giris(&d, n, &pw).0, 200);
}

#[test]
fn eslesme_parola_ve_serbest_birakma() {
    let _k = Kayit::bas("test_api", "test_eslesme_parola_ve_serbest_birakma");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let a = anahtar(&giris(&d, n, &pw).1);
    let (c, j) = post(
        &d,
        "/api/eslesme",
        &json!({"cihaz_anahtari": a, "isletme_adi": "Bocafe Göztepe", "unvan": "<b>", "surum": "1.1", "adres": "Moda Cad. 1 Kadıköy"}),
    );
    assert!(c == 200 && j["durum"] == "bagli" && j["uyelik"] == "aktif");
    let c = d.v.bagli_cihaz(n).unwrap().unwrap();
    assert!(c.isletme_adi == "Bocafe Göztepe" && c.surum == "1.1" && !c.son_eslesme.is_empty());
    assert_eq!(c.adres, "Moda Cad. 1 Kadıköy");
    // adres göndermeyen (eski) cihaz kayıtlı adresi silmez; gönderilen bozuk adres boşalır
    post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "isletme_adi": "Bocafe Göztepe", "unvan": "<b>", "surum": "1.0"}));
    assert_eq!(d.v.bagli_cihaz(n).unwrap().unwrap().adres, "Moda Cad. 1 Kadıköy");
    post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "adres": "a\u{0}b"}));
    assert_eq!(d.v.bagli_cihaz(n).unwrap().unwrap().adres, "");
    assert_eq!(kod(&post(&d, "/api/parola", &json!({"cihaz_anahtari": a, "eski": "yanlis-1234", "yeni": "yeni-parola-1"})).1), "hatali");
    assert_eq!(post(&d, "/api/parola", &json!({"cihaz_anahtari": a, "eski": pw, "yeni": "kisa"})).0, 400);
    let (c, j) = post(&d, "/api/parola", &json!({"cihaz_anahtari": a, "eski": pw, "yeni": "yeni-parola-1"}));
    assert!(c == 200 && d.v.parola_dogrula(n, "yeni-parola-1").unwrap().is_some() && j["ozet"] == mus(&d, n).ozet);
    d.v.serbest_birak(n, false).unwrap();
    assert_eq!(post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a})).1["durum"], "serbest");
    assert_eq!(post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "temizlendi": true})).1["durum"], "serbest");
    assert_eq!(d.q.isler().last().unwrap(), &("cihaz-kapat".to_string(), json!({"numara": n.to_string(), "wg_pub": WG})));
    assert_eq!(kod(&post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a})).1), "taninmadi");
    assert!(d.v.bagli_cihaz(n).unwrap().is_none());
}

#[test]
fn metinler_kirpilir_ve_hiz_siniri() {
    let _k = Kayit::bas("test_api", "test_metinler_kirpilir_ve_hiz_siniri");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let a = anahtar(&giris(&d, n, &pw).1);
    post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "isletme_adi": "x".repeat(500), "unvan": "a\u{0}b", "surum": 5}));
    let c = d.v.bagli_cihaz(n).unwrap().unwrap();
    assert!(c.isletme_adi.is_empty() && c.unvan.is_empty() && c.surum.is_empty());
    let kodlar: Vec<u16> = (0..21).map(|_| post_ip(&d, "/api/eslesme", &json!({"cihaz_anahtari": a}), "203.0.113.99").0).collect();
    assert!(kodlar[20] == 429 && !kodlar[..20].contains(&429));
}

#[test]
fn unicode_rakam_kilidi_atlatamaz() {
    let _k = Kayit::bas("test_api", "test_unicode_rakam_kilidi_atlatamaz");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    for _ in 0..5 {
        giris(&d, n, "yanlis-parola");
    }
    assert_eq!(giris(&d, n, &pw).0, 429);
    let (c, j) = post(&d, "/api/giris", &json!({"numara": "1\u{ff10}0001", "parola": pw, "wg_pub": WG, "ssh_pub": SSH}));
    assert!(c == 400 && kod(&j) == "gecersiz" && d.q.isler().is_empty());
}

#[test]
fn lisans_bilgisi_ve_parola_tipi() {
    let _k = Kayit::bas("test_api", "test_lisans_bilgisi_ve_parola_tipi");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (c, j) = giris(&d, n, &pw);
    assert!(c == 200 && j["lisans"] == "aktif" && j["lisans_bitis"] == "2027-09-21");
    let a = anahtar(&j);
    d.v.askiya_al(n, true).unwrap();
    let j = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a})).1;
    assert!(j["durum"] == "bagli" && j["lisans"] == "askida");
    assert_eq!(post(&d, "/api/parola", &json!({"cihaz_anahtari": a, "eski": 5, "yeni": "yeni-parola-1"})).0, 400);
    assert_eq!(mus(&d, n).parola_acik, pw);
    post(&d, "/api/parola", &json!({"cihaz_anahtari": a, "eski": pw, "yeni": "yeni-parola-1"}));
    assert_eq!(mus(&d, n).parola_acik, "yeni-parola-1"); // cihazdan değişen parola da yönetimde görünür
}

#[test]
fn giris_ve_eslesme_tek_admin_ozetini_tasir() {
    let _k = Kayit::bas("test_api", "test_giris_ve_eslesme_tek_admin_ozetini_tasir");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (c, j) = giris(&d, n, &pw);
    assert!(c == 200 && j.get("admin").is_none()); // yönetici belirlemediyse cihazdaki admin aynen kalır
    d.v.admin_parolasi_koy("benim-admin-parolam").unwrap();
    let (c, j2) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": j["cihaz_anahtari"]}));
    let a = d.v.admin_parolasi().unwrap().unwrap();
    assert!(c == 200 && j2["admin"] == json!({"tuz": a.tuz, "ozet": a.ozet, "yineleme": a.yineleme}));
    assert!(!j2.to_string().contains("benim-admin-parolam"));
    let (_, j3) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": j["cihaz_anahtari"]}));
    assert_eq!(j3["admin"], j2["admin"]); // her eşitlemede aynı özet: admin oturumları boşuna düşmez
}

/// AP yönetim parolası (final review I1; müşteriye özel, panel parolasından ayrı) cihaza bağlanırken ve her eşitlemede
/// gider: cihaz paneli AP kurulumunda işletme sahibine de gösterilir. Yenilenen parola sonraki eşitlemede iner.
#[test]
fn giris_ve_eslesme_ap_parolasini_tasir() {
    let _k = Kayit::bas("test_api", "test_giris_ve_eslesme_ap_parolasini_tasir");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (c, j) = giris(&d, n, &pw);
    let ap = mus(&d, n).ap_parola;
    assert!(c == 200 && j["ap_parola"] == ap && ap != pw);
    let yeni = d.v.ap_parolasi_yenile(n).unwrap();
    assert_eq!(post(&d, "/api/eslesme", &json!({"cihaz_anahtari": j["cihaz_anahtari"]})).1["ap_parola"], yeni);
    let (c, h) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": "yanlis"}));
    assert!(c == 401 && h.get("ap_parola").is_none());
}

/// Spec bilinçli fark 2: eşleşme/bağlama cevabındaki `sms` yalnızca `etkin_mock` taşır; NetGSM bilgisi (`mock`, `provider`,
/// `netgsm`) hiçbir cihaza gitmez. Python kaydı eski cihazlar için `mock` + `netgsm`'i de taşıyordu: bu yüzden altınsız.
#[test]
fn sms_ayari_yoksa_yalnizca_etkin_mock_varsa_gider() {
    let _k = Kayit::altinsiz("spec bilinçli fark 2");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    assert_eq!(j["sms"], json!({"etkin_mock": true}));
    d.v.sms_ayari_koy(&json!({"mock": false, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""})).unwrap();
    let (_, j2) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": j["cihaz_anahtari"]}));
    assert_eq!(j2["sms"], json!({"etkin_mock": false}));
    let govde = j2.to_string();
    assert!(!govde.contains("gizli-1") && !govde.contains("8503027084") && !govde.contains("netgsm") && !govde.contains("provider"));
    let (c, j3) = post(&d, "/api/eslesme", &json!({"cihaz_anahtari": "yanlis"}));
    assert!(c == 401 && j3.get("sms").is_none());
    // parolalı giriş (eski cihazlar dahil) de aynı: ortak NetGSM sırrı hiçbir cevapta yok
    let (n2, pw2) = musteri(&d);
    let (c, g2) = giris_ip(&d, n2, &pw2, WG2, "198.51.100.9");
    assert!(c == 200 && g2["sms"] == json!({"etkin_mock": false}) && !g2.to_string().contains("gizli-1"));
}

#[test]
fn saglik_raporu_kaydedilir() {
    let _k = Kayit::bas("test_api", "test_saglik_raporu_kaydedilir");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    let c = d.v.bagli_cihaz(n).unwrap().unwrap();
    let govde = json!({"cihaz_anahtari": j["cihaz_anahtari"], "rapor": rapor_ile(&[("ad", json!("Ali")), ("telefon", json!("5551112233"))])});
    let (cd, r) = post(&d, "/api/saglik", &govde);
    assert!(cd == 200 && r == json!({"tamam": true, "sms_etkin_mock": true, "listeler": {}, "ap_parola": mus(&d, n).ap_parola}));
    assert_eq!(d.v.saglik(c.id).unwrap().unwrap().rapor, rapor()); // bilinmeyen alanlar atıldı
    let govde = json!({"cihaz_anahtari": j["cihaz_anahtari"], "rapor": rapor_ile(&[("son_muhur", json!("")), ("bagli", json!(0))])});
    assert_eq!(post(&d, "/api/saglik", &govde).0, 200);
    let kayit = d.v.saglik(c.id).unwrap().unwrap().rapor;
    assert!(kayit["son_muhur"] == "" && kayit["bagli"] == 0);
    // serbest bırakılan cihaz: 200 ama yazılmaz
    d.v.saglik_kaydet(c.id, &json!({"eski": 1})).unwrap();
    d.v.serbest_birak(n, false).unwrap();
    let (cd, r) = post(&d, "/api/saglik", &json!({"cihaz_anahtari": j["cihaz_anahtari"], "rapor": rapor()}));
    // bırakılan cihaza liste sürümü bildirilmez (indiremeyeceği listeyi her raporda denemesin)
    assert!(cd == 200 && r == json!({"tamam": true, "sms_etkin_mock": true}) && d.v.saglik(c.id).unwrap().unwrap().rapor == json!({"eski": 1}));
}

#[test]
fn saglik_hizmet_kapaliyken_liste_surumu_yok() {
    let _k = Kayit::bas("test_api", "test_saglik_hizmet_kapaliyken_liste_surumu_yok");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    d.v.askiya_al(n, true).unwrap();
    let govde = json!({"cihaz_anahtari": j["cihaz_anahtari"], "rapor": rapor()});
    let (c, r) = post(&d, "/api/saglik", &govde);
    assert!(c == 200 && r.get("listeler").is_none());
    d.v.askiya_al(n, false).unwrap();
    let (c, r) = post(&d, "/api/saglik", &govde);
    assert!(c == 200 && r["listeler"] == json!({}));
}

#[test]
fn bozuk_rapor_reddedilir() {
    let _k = Kayit::bas("test_api", "test_bozuk_rapor_reddedilir");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    let (k, cid) = (anahtar(&j), d.v.bagli_cihaz(n).unwrap().unwrap().id);
    let yolla = |rp: &Value| post_ip(&d, "/api/saglik", &json!({"cihaz_anahtari": k, "rapor": rp}), "198.51.100.9");
    let mut eksik = rapor();
    eksik.as_object_mut().unwrap().remove("wan");
    let mut sv_1 = rapor()["servisler"].clone();
    sv_1["portal"] = json!(1);
    let kotu = vec![
        eksik,
        rapor_ile(&[("disk_yuzde", json!(101))]),
        rapor_ile(&[("disk_yuzde", json!(-1))]),
        rapor_ile(&[("servisler", json!(["portal"]))]),
        rapor_ile(&[("servisler", sv_1)]),
        rapor_ile(&[("servisler", json!({"portal": true}))]),
        rapor_ile(&[("acik_sn", json!(true))]),
        rapor_ile(&[("bagli", json!(true))]),
        rapor_ile(&[("disk_yuzde", json!(true))]),
        rapor_ile(&[("acik_sn", json!(-1))]),
        rapor_ile(&[("wan", json!(1))]),
        rapor_ile(&[("hiz_hata", json!("evet"))]),
        rapor_ile(&[("surum", json!("x".repeat(41)))]),
        rapor_ile(&[("surum", json!(5))]),
        rapor_ile(&[("son_muhur", json!("10.10.2026"))]),
        rapor_ile(&[("son_muhur", json!("2026-13-40"))]),
        rapor_ile(&[("acik_sn", json!(1.5))]),
        json!("metin"),
        Value::Null,
    ];
    for rp in &kotu {
        let (c, r) = yolla(rp);
        assert!(c == 400 && kod(&r) == "gecersiz", "{rp}");
    }
    assert_eq!(post(&d, "/api/saglik", &json!({"cihaz_anahtari": k})).0, 400); // rapor yok
    assert!(d.v.saglik(cid).unwrap().is_none());
    let (c, r) = post(&d, "/api/saglik", &json!({"cihaz_anahtari": "bilinmeyen", "rapor": rapor()}));
    assert!(c == 401 && kod(&r) == "taninmadi");
}

#[test]
fn saglik_govde_siniri_ve_hiz_siniri() {
    let _k = Kayit::bas("test_api", "test_saglik_govde_siniri_ve_hiz_siniri");
    let d = kur(Some(tamam()));
    assert_eq!(Y::istek(&d.api, "POST", "/api/saglik", &[], &"x".repeat(5000), "", "1.1.1.1").durum, 413);
    let (n, pw) = musteri(&d);
    let (_, j) = giris_ip(&d, n, &pw, WG, "198.51.100.20");
    let sonuc: Vec<u16> =
        (0..21).map(|_| post_ip(&d, "/api/saglik", &json!({"cihaz_anahtari": j["cihaz_anahtari"], "rapor": rapor()}), "203.0.113.5").0).collect();
    assert!(sonuc[..20].iter().all(|c| *c == 200) && sonuc[20] == 429);
}

/// Python `_surumlu(tmp, kanal, hedef)`: sur/ klasöründe 0.4.0, 0.4.1 ve 0.3.9 paketleri.
fn surumlu(kanal: &str, hedef: &str) -> (D, i64, String) {
    let mut d = kur(Some(tamam()));
    let dizin = d.tmp.join("sur");
    std::fs::create_dir(&dizin).unwrap();
    d.api.surum_dizini = dizin.clone();
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    for s in ["0.4.0", "0.4.1", "0.3.9"] {
        let b = format!("deb-{s}");
        std::fs::write(dizin.join(format!("{s}.deb")), &b).unwrap();
        d.v.surum_ekle(s, &"ab".repeat(32), b.len() as i64, "yerel").unwrap();
    }
    d.v.kanal_hedef_koy("test", Some(hedef)).unwrap();
    d.v.guncelleme_kanali_koy(n, kanal).unwrap();
    (d, n, anahtar(&j))
}

#[test]
fn rapor_paket_ve_guncelleme_alanlari() {
    let _k = Kayit::bas("test_api", "test_rapor_paket_ve_guncelleme_alanlari");
    let (d, n, k) = surumlu("test", "0.4.1");
    let cid = d.v.bagli_cihaz(n).unwrap().unwrap().id;
    assert_eq!(rapor_yolla(&d, &k, &[]).0, 200); // eski biçim
    let r = d.v.saglik(cid).unwrap().unwrap().rapor;
    assert!(r.get("paket").is_none() && r.get("guncelleme").is_none());
    let g = json!({"surum": "0.4.1", "sonuc": "geri_alindi", "zaman": 5});
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("0.4.0")), ("guncelleme", g.clone())]).0, 200);
    let r = d.v.saglik(cid).unwrap().unwrap().rapor;
    assert!(r["paket"] == "0.4.0" && r["guncelleme"] == g);
    let sonuclar = || d.v.hareketler(Some(n), 300, None).unwrap().into_iter().filter(|h| h.olay == "GUNCELLEME_SONUC").collect::<Vec<_>>();
    let olaylar = sonuclar();
    assert!(olaylar.len() == 1 && olaylar[0].ayrinti.contains("surum=0.4.1") && olaylar[0].ayrinti.contains("sonuc=geri_alindi"));
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("0.4.0")), ("guncelleme", g.clone())]).0, 200); // değişmedi: yeni hareket yok
    assert_eq!(sonuclar().len(), 1);
    let degis = |alan: &str, v: Value| {
        let mut x = g.clone();
        x[alan] = v;
        x
    };
    let mut ek1 = g.clone();
    ek1["ek"] = json!(1);
    for kotu in [degis("sonuc", json!("x")), degis("surum", json!("1.2")), degis("zaman", json!(-1)), degis("zaman", json!(true)), json!("x"), ek1] {
        assert_eq!(rapor_yolla(&d, &k, &[("guncelleme", kotu.clone())]).0, 400, "{kotu}");
    }
    for kotu in [json!("v".repeat(41)), json!(5), json!("1.2"), json!("../x")] {
        assert_eq!(rapor_yolla(&d, &k, &[("paket", kotu.clone())]).0, 400, "{kotu}");
    }
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("")), ("guncelleme", Value::Null)]).0, 200);
}

/// PPPoE (spec 2026-10-23, "AP yönetim parolası cihaza 5 dk içinde ulaşır"): müşterinin AP yönetim parolası bağlı cihazın
/// 5 dk'lık sağlık cevabında da gider; müşteride parola boşsa alan hiç yok. Bırakılan cihaza ve tanınmayan anahtara gitmez;
/// parola hareket satırına yazılmaz.
#[test]
fn saglik_cevabinda_ap_parola() {
    let _k = Kayit::bas("test_api", "test_saglik_cevabinda_ap_parola");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let (_, j) = giris(&d, n, &pw);
    let k = anahtar(&j);
    let ap = mus(&d, n).ap_parola;
    assert!(!ap.is_empty() && ap != pw);
    let (c, r) = rapor_yolla(&d, &k, &[]);
    assert!(c == 200 && r["ap_parola"] == ap);
    let yeni = d.v.ap_parolasi_yenile(n).unwrap(); // yönetimde "Yenile": sonraki rapor cevabında iner
    assert!(yeni != ap && rapor_yolla(&d, &k, &[]).1["ap_parola"] == yeni);
    // hizmet (lisans/üyelik) kapalı olsa da /api/eslesme gibi gider: cihazın bildiği parola merkezle aynı kalsın
    d.v.askiya_al(n, true).unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["ap_parola"], yeni);
    d.v.askiya_al(n, false).unwrap();
    // müşteride parola yok: alan hiç gönderilmez (boş dize de değil)
    d.v.kilit().execute("UPDATE musteri SET ap_parola = '' WHERE numara = ?", [n]).unwrap();
    let (c, r) = rapor_yolla(&d, &k, &[]);
    assert!(c == 200 && r.get("ap_parola").is_none() && r["tamam"] == true);
    d.v.kilit().execute("UPDATE musteri SET ap_parola = ? WHERE numara = ?", rusqlite::params![yeni, n]).unwrap();
    // parola hareket satırlarına ve kayıtlı rapora yazılmaz
    assert!(d.v.hareketler(None, 300, None).unwrap().iter().all(|h| !h.ayrinti.contains(&yeni) && !h.ayrinti.contains(&ap)));
    let cid = d.v.bagli_cihaz(n).unwrap().unwrap().id;
    assert!(!d.v.saglik(cid).unwrap().unwrap().rapor.to_string().contains(&yeni));
    // tanınmayan anahtar: 401, parola yok
    let (c, h) = post(&d, "/api/saglik", &json!({"cihaz_anahtari": "yanlis", "rapor": rapor()}));
    assert!(c == 401 && h.get("ap_parola").is_none());
    // bırakılan cihaz (bağlı değil): 200 ama parola gitmez
    d.v.serbest_birak(n, false).unwrap();
    let (c, r) = rapor_yolla(&d, &k, &[]);
    assert!(c == 200 && r.get("ap_parola").is_none());
}

/// PPPoE (spec 2026-10-23 "Durum ve teşhis"): rapora `wan_tur` (dhcp|pppoe) ve `pppoe` (durum kodu) girer. İkisi de isteğe
/// bağlı ve ileriye dönük uyumlu: bilinmeyen ya da bozuk değer RAPORU REDDETMEZ, yalnızca o alan atılır. Öteki alanlar katı.
#[test]
fn saglik_temizle_pppoe() {
    let _k = Kayit::bas("test_api", "test_saglik_temizle_pppoe");
    let t = saglik_temizle(&rapor_ile(&[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli"))])).unwrap();
    assert_eq!(t, rapor_ile(&[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli"))]));
    let eski = saglik_temizle(&rapor()).unwrap(); // alanlar yok: eskisi gibi, anahtarlar da eklenmez
    assert!(eski == rapor() && eski.get("wan_tur").is_none() && eski.get("pppoe").is_none());
    for tur in ["dhcp", "pppoe"] {
        assert_eq!(saglik_temizle(&rapor_ile(&[("wan_tur", json!(tur))])).unwrap()["wan_tur"], tur);
    }
    for durum in ["kapali", "kablo_yok", "dhcp_yok", "internet_yok", "sunucu_yok", "modemden", "kimlik_hatasi", "baglaniyor", "bagli"] {
        assert_eq!(saglik_temizle(&rapor_ile(&[("pppoe", json!(durum))])).unwrap()["pppoe"], durum);
    }
    assert!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("dhcp"))])).unwrap().get("pppoe").is_none()); // tek alan gelebilir
    // bilinmeyen alan hâlâ atılır
    assert!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("dhcp")), ("pppoe", json!("kapali")), ("ek", json!(1))])).unwrap().get("ek").is_none());
    // bilinmeyen / bozuk değer: rapor KABUL edilir, yalnızca o alan yoktur (öteki alanlar aynen)
    for kotu in [json!("x"), json!(""), json!("PPPoE"), json!("pppoe "), json!("ppp"), json!(5), Value::Null, json!(true), json!(["pppoe"]), json!({"pppoe": 1})] {
        assert_eq!(saglik_temizle(&rapor_ile(&[("wan_tur", kotu.clone())])), Some(rapor()), "{kotu}");
    }
    for kotu in [
        json!("?"),
        json!(""),
        json!("Bagli"),
        json!("bagli "),
        json!("kablo yok"),
        json!("yeni_durum"),
        json!(5),
        Value::Null,
        json!(false),
        json!(["bagli"]),
        json!({"bagli": 1}),
    ] {
        assert_eq!(saglik_temizle(&rapor_ile(&[("pppoe", kotu.clone())])), Some(rapor()), "{kotu}");
    }
    // biri geçerli, öteki bozuk: geçerli olan saklanır, bozuk olan atılır (ikisi birden atılmaz)
    assert_eq!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("pppoe")), ("pppoe", json!("?"))])), Some(rapor_ile(&[("wan_tur", json!("pppoe"))])));
    assert_eq!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("x")), ("pppoe", json!("bagli"))])), Some(rapor_ile(&[("pppoe", json!("bagli"))])));
    assert_eq!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("x")), ("pppoe", json!("?"))])), Some(rapor()));
    // öteki alanlar katı kalır
    assert!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli")), ("disk_yuzde", json!(101))])).is_none());
    assert!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("x")), ("pppoe", json!("?")), ("disk_yuzde", json!(101))])).is_none());
    assert!(saglik_temizle(&rapor_ile(&[("wan_tur", json!("x")), ("paket", json!(5))])).is_none());
    let mut wan_siz = rapor_ile(&[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli"))]);
    wan_siz.as_object_mut().unwrap().remove("wan");
    assert!(saglik_temizle(&wan_siz).is_none()); // zorunlu alan eksik
    // uçtan uca: geçerli rapor saklanır; yeni/bozuk değerli rapor 200 ile kabul edilir ve o alan olmadan saklanır
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let k = anahtar(&giris(&d, n, &pw).1);
    let cid = d.v.bagli_cihaz(n).unwrap().unwrap().id;
    assert_eq!(rapor_yolla(&d, &k, &[]).0, 200);
    let r = d.v.saglik(cid).unwrap().unwrap().rapor;
    assert!(r.get("wan_tur").is_none() && r.get("pppoe").is_none());
    assert_eq!(rapor_yolla(&d, &k, &[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli"))]).0, 200);
    let r = d.v.saglik(cid).unwrap().unwrap().rapor;
    assert!(r["wan_tur"] == "pppoe" && r["pppoe"] == "bagli");
    let tuhaflar: [(Vec<(&str, Value)>, Vec<(&str, Value)>); 3] = [
        (vec![("wan_tur", json!("pppoe")), ("pppoe", json!("yeni_durum"))], vec![("wan_tur", json!("pppoe"))]), // gelecekteki yeni durum kodu
        (vec![("wan_tur", json!("l2tp")), ("pppoe", json!("bagli"))], vec![("pppoe", json!("bagli"))]),
        (vec![("wan_tur", json!(5)), ("pppoe", Value::Null)], vec![]),
    ];
    for (tuhaf, beklenen) in &tuhaflar {
        let (c, h) = rapor_yolla(&d, &k, tuhaf);
        assert!(c == 200 && h["tamam"] == true);
        assert_eq!(d.v.saglik(cid).unwrap().unwrap().rapor, rapor_ile(beklenen)); // yeni rapor kaydedildi; öncekinin alanı kalmaz
    }
    // başka bir alan bozuksa rapor hâlâ reddedilir ve öncekinin üzerine yazmaz
    let onceki = d.v.saglik(cid).unwrap().unwrap().rapor;
    let (c, h) = rapor_yolla(&d, &k, &[("wan_tur", json!("pppoe")), ("pppoe", json!("bagli")), ("disk_yuzde", json!(101))]);
    assert!(c == 400 && kod(&h) == "gecersiz" && d.v.saglik(cid).unwrap().unwrap().rapor == onceki);
}

#[test]
fn guncelleme_hedefi_kanal_lisans_ve_hemen() {
    let _k = Kayit::bas("test_api", "test_guncelleme_hedefi_kanal_lisans_ve_hemen");
    let (d, n, k) = surumlu("test", "0.4.1");
    let (_, r) = rapor_yolla(&d, &k, &[]);
    assert_eq!(
        r,
        json!({"tamam": true, "sms_etkin_mock": true, "listeler": {}, "ap_parola": mus(&d, n).ap_parola,
               "guncelleme": {"surum": "0.4.1", "sha256": "ab".repeat(32), "boyut": 9, "hemen": false}})
    );
    d.v.guncelleme_hemen_koy(n, 100.0, "0.4.1").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["hemen"], true);
    let g = |sonuc: &str, zaman: u64| ("guncelleme", json!({"surum": "0.4.1", "sonuc": sonuc, "zaman": zaman}));
    assert_eq!(rapor_yolla(&d, &k, &[g("hata", 50)]).1["guncelleme"]["hemen"], true);
    assert_eq!(rapor_yolla(&d, &k, &[g("tamam", 200)]).1["guncelleme"]["hemen"], false);
    assert_eq!(mus(&d, n).guncelleme_hemen, 0.0);
    d.v.guncelleme_kanali_koy(n, "kapali").unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    d.v.guncelleme_kanali_koy(n, "kararli").unwrap(); // kararlı kanalın hedefi yok
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    d.v.kanal_hedef_koy("kararli", Some("0.4.0")).unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["surum"], "0.4.0");
    d.v.kanal_hedef_koy("kararli", Some("9.9.9")).unwrap(); // sürüm kaydı yok
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    d.v.kanal_hedef_koy("kararli", Some("0.4.0")).unwrap();
    d.v.askiya_al(n, true).unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    d.v.askiya_al(n, false).unwrap();
    d.v.lisans_ayarla(n, "2020-01-01").unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    d.v.kanal_hedef_koy("kararli", None).unwrap();
    assert!(d.v.kanal_hedef("kararli").unwrap().is_none());
}

#[test]
fn hemen_bayragi_hedef_surume_bagli() {
    let _k = Kayit::bas("test_api", "test_hemen_bayragi_hedef_surume_bagli");
    let (d, n, k) = surumlu("test", "0.4.1");
    // bayrak başka bir hedef için konmuştu: gönderilmez, sağlık raporunda temizlenir
    d.v.guncelleme_hemen_koy(n, 100.0, "0.4.0").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["hemen"], false);
    assert!(mus(&d, n).guncelleme_hemen == 0.0 && mus(&d, n).guncelleme_hemen_surum.is_empty());
    // cihaz zaten hedefte: sonuç gelmeyecek, bayrak temizlenir
    d.v.guncelleme_hemen_koy(n, 100.0, "0.4.1").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("0.4.0"))]).1["guncelleme"]["hemen"], true);
    assert_eq!(mus(&d, n).guncelleme_hemen, 100.0);
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("0.4.1"))]).1["guncelleme"]["hemen"], false);
    assert_eq!(mus(&d, n).guncelleme_hemen, 0.0);
    // hedef kalktı (kanal kapalı): bayrak temizlenir, kanal yeniden açılınca eski bayrak geri gelmez
    d.v.guncelleme_hemen_koy(n, 100.0, "0.4.1").unwrap();
    d.v.guncelleme_kanali_koy(n, "kapali").unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none());
    assert_eq!(mus(&d, n).guncelleme_hemen, 0.0);
    d.v.guncelleme_kanali_koy(n, "test").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["hemen"], false);
}

#[test]
fn en_son_hedefi_github_surumlerinin_en_yuksegi() {
    let _k = Kayit::bas("test_api", "test_en_son_hedefi_github_surumlerinin_en_yuksegi");
    let (d, n, k) = surumlu("test", "0.4.1"); // depoda yalnızca elle eklenmiş (yerel) 0.3.9, 0.4.0, 0.4.1
    d.v.kanal_hedef_koy("test", Some("en-son")).unwrap();
    assert!(d.v.kanal_hedef("test").unwrap().as_deref() == Some("en-son") && d.v.en_son_surum().unwrap().is_none());
    assert!(rapor_yolla(&d, &k, &[]).1.get("guncelleme").is_none()); // GitHub sürümü yok: yerel deneme paketleri "en son" sayılmaz
    d.v.surum_ekle("0.4.0", &"cd".repeat(32), 9, "github").unwrap(); // 0.4.0 artık GitHub'dan
    assert!(d.v.en_son_surum().unwrap().as_deref() == Some("0.4.0") && d.v.kanal_surumu("test").unwrap().as_deref() == Some("0.4.0"));
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["surum"], "0.4.0"); // yerel 0.4.1 daha yüksek ama seçilmez
    d.v.surum_ekle("0.10.0", &"ef".repeat(32), 9, "github").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["surum"], "0.10.0"); // sayısal sıra (0.10.0 > 0.4.0)
    // "şimdi güncelle" basıldığı andaki sürüm içindir: en son ilerlerse bayrak yeni sürüme geçmez, temizlenir
    d.v.guncelleme_hemen_koy(n, 100.0, "0.10.0").unwrap();
    assert_eq!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["hemen"], true);
    d.v.surum_ekle("0.11.0", &"aa".repeat(32), 9, "github").unwrap();
    let g = rapor_yolla(&d, &k, &[]).1["guncelleme"].clone();
    assert!(g["surum"] == "0.11.0" && g["hemen"] == false && mus(&d, n).guncelleme_hemen == 0.0);
    // elle seçilen sürüm "en son"dan bağımsız kalır
    d.v.kanal_hedef_koy("test", Some("0.4.1")).unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1["guncelleme"]["surum"] == "0.4.1" && d.v.kanal_surumu("test").unwrap().as_deref() == Some("0.4.1"));
}

#[test]
fn onceki_paketin_sha256_si() {
    let _k = Kayit::bas("test_api", "test_onceki_paketin_sha256_si");
    let (d, _n, k) = surumlu("test", "0.4.1");
    d.v.surum_ekle("0.4.0", &"cd".repeat(32), 9, "yerel").unwrap();
    assert!(rapor_yolla(&d, &k, &[]).1["guncelleme"].get("onceki_sha256").is_none()); // paket bildirilmedi
    assert_eq!(rapor_yolla(&d, &k, &[("paket", json!("0.4.0"))]).1["guncelleme"]["onceki_sha256"], "cd".repeat(32));
    assert!(rapor_yolla(&d, &k, &[("paket", json!("0.2.0"))]).1["guncelleme"].get("onceki_sha256").is_none()); // depoda yok
    assert!(rapor_yolla(&d, &k, &[("paket", json!(""))]).1["guncelleme"].get("onceki_sha256").is_none());
}

#[test]
fn api_paket_yetkisi() {
    let _k = Kayit::bas("test_api", "test_api_paket_yetkisi");
    let (d, n, k) = surumlu("test", "0.4.1");
    let paket = |surum: &str, anahtar: &str| {
        Y::istek(&d.api, "POST", "/api/paket", &[], &Y::json_govde(&json!({"cihaz_anahtari": anahtar, "surum": surum})), "", "198.51.100.77")
    };
    let y = paket("0.4.1", &k);
    assert!(y.durum == 200 && y.govde == b"deb-0.4.1" && y.baslik("Content-Type") == Some("application/octet-stream"));
    assert_eq!(y.baslik("Cache-Control"), Some("no-store"));
    let y = paket("0.4.0", &k);
    assert!(y.durum == 403 && kod(&serde_json::from_slice(&y.govde).unwrap()) == "yetki");
    rapor_yolla(&d, &k, &[("paket", json!("0.4.0"))]); // geri dönüş paketi: raporlanan sürüm
    let y = paket("0.4.0", &k);
    assert!(y.durum == 200 && y.govde == b"deb-0.4.0");
    assert_eq!(paket("0.3.9", &k).durum, 403);
    assert_eq!(paket("../x", &k).durum, 403);
    assert_eq!(paket("0.4.1", "bilinmeyen").durum, 401);
    d.v.askiya_al(n, true).unwrap();
    assert!(paket("0.4.1", &k).durum == 403 && paket("0.4.0", &k).durum == 403);
    d.v.askiya_al(n, false).unwrap();
    assert_eq!(paket("0.4.0", &k).durum, 200);
    d.v.lisans_ayarla(n, "2020-01-01").unwrap();
    assert!(paket("0.4.1", &k).durum == 403 && paket("0.4.0", &k).durum == 403);
    d.v.lisans_uzat(n, crate::veri::LISANS_GUN).unwrap();
    d.v.uyelik(n, "bitti").unwrap();
    assert_eq!(paket("0.4.0", &k).durum, 403);
    d.v.uyelik(n, "aktif").unwrap();
    d.v.serbest_birak(n, false).unwrap();
    assert!(paket("0.4.1", &k).durum == 403 && paket("0.4.0", &k).durum == 403);
    d.v.serbest_iptal(n).unwrap();
    std::fs::remove_file(d.api.surum_dizini.join("0.4.1.deb")).unwrap();
    assert_eq!(paket("0.4.1", &k).durum, 404);
}

/// Spec bilinçli fark 1: `/api/raporlar` ve `/api/rapor` yok (cihaz artık merkezden rapor almıyor): 404, hız sınırına girmez.
#[test]
fn api_rapor_uclari_yok_404() {
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let a = anahtar(&giris(&d, n, &pw).1);
    for yol in ["/api/raporlar", "/api/rapor"] {
        for _ in 0..25 {
            let (c, j) = post_ip(&d, yol, &json!({"cihaz_anahtari": a, "tur": "ay", "donem": "2026-09"}), "203.0.113.9");
            assert!(c == 404 && kod(&j) == "yok" && j["hata"] == "Bulunamadı.");
        }
    }
    // 404'ler ortak hız sınırını tüketmedi
    assert_eq!(post_ip(&d, "/api/eslesme", &json!({"cihaz_anahtari": a}), "203.0.113.9").0, 200);
}

#[test]
fn liste_ucu_yetki_ve_icerik() {
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let a = anahtar(&giris(&d, n, &pw).1);
    std::fs::create_dir(d.tmp.join("listeler")).unwrap();
    yaz(&d.tmp.join("listeler/kumar.txt.gz"), "gz-bayt");
    yaz(&d.tmp.join("listeler/kumar.json"), &json!({"sha256": "ab".repeat(32), "sayi": 3, "zaman": "2026-09-21T03:00:00+03:00"}).to_string());
    yaz(&d.tmp.join("listeler/yetiskin.json"), &json!({"sha256": "XY", "sayi": 3, "zaman": "z"}).to_string()); // sha geçersiz: özette yok
    yaz(&d.tmp.join("listeler/zararli.json"), &json!({"sha256": "ab".repeat(32), "sayi": 3.5, "zaman": "z"}).to_string()); // sayı int değil
    let liste = |kategori: &Value, anahtar: &str| Y::istek(&d.api, "POST", "/api/liste", &[], &Y::json_govde(&json!({"cihaz_anahtari": anahtar, "kategori": kategori})), "", "198.51.100.88");
    let y = liste(&json!("kumar"), &a);
    assert!(y.durum == 200 && y.govde == b"gz-bayt" && y.baslik("Content-Type") == Some("application/gzip"));
    assert_eq!(liste(&json!("kumar"), "x").durum, 401);
    for kotu in [json!("../x"), json!(5), json!(["kumar"]), json!("")] {
        assert_eq!(liste(&kotu, &a).durum, 400, "{kotu}");
    }
    assert_eq!(liste(&json!("yetiskin"), &a).durum, 404); // dosya yok
    let (_, r) = post(&d, "/api/saglik", &json!({"cihaz_anahtari": a, "rapor": rapor()}));
    assert_eq!(r["listeler"], json!({"kumar": {"sha256": "ab".repeat(32), "sayi": 3, "zaman": "2026-09-21T03:00:00+03:00"}}));
    d.v.askiya_al(n, true).unwrap();
    assert_eq!(liste(&json!("kumar"), &a).durum, 403);
}

#[test]
fn bozuk_json_ve_tur_hatalari_cokmez() {
    let d = kur(Some(tamam()));
    let mut sayac = 0;
    for govde in ["[]", "5", "null", "\"x\"", "{\"cihaz_anahtari\": 5}", "{\"cihaz_anahtari\": [1]}"] {
        for yol in ["/api/eslesme", "/api/parola", "/api/saglik", "/api/paket", "/api/liste", "/api/bekliyor", "/api/bagla", "/api/giris"] {
            sayac += 1;
            let (c, _) = post_ham(&d, yol, govde, &format!("203.0.113.{sayac}"));
            assert!(c == 400 || c == 401, "{yol} {govde} {c}");
        }
    }
}

// --- test_misafir_kodu.py ---
#[test]
fn eslesme_turu_kaydeder() {
    let _k = Kayit::bas("test_misafir_kodu", "test_eslesme_turu_kaydeder");
    let d = kur(Some(tamam()));
    let (n, pw) = musteri(&d);
    let a = anahtar(&giris(&d, n, &pw).1);
    let tur = |d: &D| d.v.bagli_cihaz(n).unwrap().unwrap().isletme_turu;
    assert_eq!(tur(&d), "");
    assert_eq!(post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "isletme_turu": "otel"})).0, 200);
    assert_eq!(tur(&d), "otel");
    for bozuk in [json!("hastane"), json!("OTEL"), json!(""), json!(5), Value::Null, json!(["yurt"])] {
        // izinli olmayan değer saklanmaz
        assert_eq!(post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "isletme_turu": bozuk})).0, 200);
        assert_eq!(tur(&d), "otel", "{bozuk}");
    }
    post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a})); // gönderilmezse eski değer kalır
    assert_eq!(tur(&d), "otel");
    for t in ["yurt", "diger", "kafe"] {
        post(&d, "/api/eslesme", &json!({"cihaz_anahtari": a, "isletme_turu": t}));
        assert_eq!(tur(&d), t);
    }
}

// --- test_bekleme.py (yalnızca api + veri ile çalışan testler) ---
pub(crate) const K1: &str = "11111111111111111111111111111111";
pub(crate) const S1: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) const K2: &str = "22222222222222222222222222222222";
pub(crate) const S2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub(crate) const IP1: &str = "203.0.113.5";
pub(crate) const IP2: &str = "198.51.100.7";

pub(crate) fn bildir_tam(d: &D, k: &str, s: &str, wg: &str, ip: &str, mac: &str, surum: &str) -> (u16, Value) {
    post_ip(d, "/api/bekliyor", &json!({"kimlik": k, "sir": s, "surum": surum, "wan_mac": mac, "wg_pub": wg, "ssh_pub": SSH}), ip)
}

pub(crate) fn bildir(d: &D) -> (u16, Value) {
    bildir_tam(d, K1, S1, WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13")
}

pub(crate) fn bagla_tam(d: &D, belirtec: &str, k: &str, s: &str, wg: &str, ip: &str) -> (u16, Value) {
    post_ip(
        d,
        "/api/bagla",
        &json!({"kimlik": k, "sir": s, "belirtec": belirtec, "wg_pub": wg, "ssh_pub": SSH, "isletme_adi": "", "unvan": "", "surum": "0.4.13"}),
        ip,
    )
}

pub(crate) fn bagla(d: &D, belirtec: &str) -> (u16, Value) {
    bagla_tam(d, belirtec, K1, S1, WG, IP1)
}

#[test]
fn bildirim_kaydeder_sir_tutmazsa_reddeder() {
    let _k = Kayit::bas("test_bekleme", "test_bildirim_kaydeder_sir_tutmazsa_reddeder");
    let d = kur(Some(tamam()));
    assert_eq!(bildir(&d), (200, json!({"durum": "bekle"})));
    let r = d.v.bekleyenler().unwrap();
    assert!(r.len() == 1 && r[0].kimlik == K1 && r[0].ip == IP1 && r[0].sir_ozet != S1);
    assert_eq!(olaylar(&d, None), ["CIHAZ_BEKLIYOR"]);
    d.saat.ilerlet(120.0);
    assert_eq!(bildir_tam(&d, K1, S1, WG, IP2, "00:0e:c4:ce:a0:9b", "0.4.13").0, 200);
    assert!(d.v.bekleyenler().unwrap()[0].ip == IP2 && olaylar(&d, None) == ["CIHAZ_BEKLIYOR"]); // yalnızca ilk görülme
    let (c, j) = bildir_tam(&d, K1, &"c".repeat(64), WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13"); // kimliği bilen ama sırrı bilmeyen
    assert!(c == 401 && kod(&j) == "taninmadi" && d.v.bekleyenler().unwrap()[0].ip == IP2);
}

#[test]
fn bildirim_girdi_dogrulama() {
    let _k = Kayit::bas("test_bekleme", "test_bildirim_girdi_dogrulama");
    let d = kur(Some(tamam()));
    let ayni = "00:11:22:33:44:55";
    let bozuklar = [
        ("x".repeat(32), S1.to_string(), WG, ayni),
        (K1.to_string(), "kisa".to_string(), WG, ayni),
        (K1.to_string(), S1.to_string(), "kotu", ayni),
        (K1.to_string(), S1.to_string(), WG, "<b>"),
        ("A".repeat(32), S1.to_string(), WG, ""),
    ];
    for (k, s, wg, mac) in &bozuklar {
        assert_eq!(kod(&bildir_tam(&d, k, s, wg, IP1, mac, "0.4.13").1), "gecersiz", "{k} {s} {wg} {mac}");
    }
    assert_eq!(bildir_tam(&d, K1, S1, WG, IP1, "", "0.4.13").0, 200); // WAN MAC bilinmiyorsa boş gelebilir
    assert_eq!(d.v.bekleyenler().unwrap()[0].wan_mac, "");
}

#[test]
fn bildirim_hiz_ve_yeni_kimlik_siniri() {
    let _k = Kayit::bas("test_bekleme", "test_bildirim_hiz_ve_yeni_kimlik_siniri");
    let d = kur(Some(tamam()));
    let kodlar: Vec<u16> = (0..4).map(|_| bildir(&d).0).collect();
    assert_eq!(kodlar, [200, 200, 200, 429]); // kimlik başına dakikada 3
    d.saat.ilerlet(61.0);
    assert_eq!(bildir(&d).0, 200);
    for i in 0..9 {
        // aynı IP'den saatte 10 yeni kimlik (K1 dahil)
        d.saat.ilerlet(4.0);
        assert_eq!(bildir_tam(&d, &format!("{:032x}", i + 3), S1, WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13").0, 200, "{i}");
    }
    d.saat.ilerlet(4.0);
    assert_eq!(bildir_tam(&d, &"f".repeat(32), S1, WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13").0, 429);
    assert_eq!(bildir_tam(&d, &"f".repeat(32), S1, WG, IP2, "00:0e:c4:ce:a0:9b", "0.4.13").0, 200); // başka IP
    assert_eq!(d.v.bekleyenler().unwrap().len(), 11);
}

#[test]
fn eski_kayit_7_gunde_silinir() {
    let _k = Kayit::bas("test_bekleme", "test_eski_kayit_7_gunde_silinir");
    let d = kur(Some(tamam()));
    bildir(&d);
    d.saat.ilerlet(7.0 * 86400.0 + 1.0);
    bildir_tam(&d, K2, S2, WG, IP2, "00:0e:c4:ce:a0:9b", "0.4.13");
    let kimlikler: Vec<String> = d.v.bekleyenler().unwrap().into_iter().map(|r| r.kimlik).collect();
    assert_eq!(kimlikler, [K2]);
    assert_eq!(bildir_tam(&d, K1, &"c".repeat(64), WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13").0, 200); // silinen kimlik yeniden kayıt olur (yeni sır)
}

#[test]
fn reddedilen_bildirim_ve_baglama_iz_birakir() {
    let _k = Kayit::bas("test_bekleme", "test_reddedilen_bildirim_ve_baglama_iz_birakir");
    let d = kur(Some(tamam()));
    bildir(&d);
    let yanlis = |s: &str| bildir_tam(&d, K1, &s.repeat(64), WG, IP1, "00:0e:c4:ce:a0:9b", "0.4.13").0;
    assert_eq!(yanlis("c"), 401);
    assert_eq!(yanlis("d"), 401); // aynı kimlikte saatte bir satır
    assert_eq!(bagla(&d, &"x".repeat(43)).0, 401);
    assert_eq!(bagla(&d, &"y".repeat(43)).0, 401);
    let mut h: Vec<(String, String)> =
        d.v.hareketler(None, 300, None).unwrap().into_iter().filter(|x| x.olay.ends_with("_RED")).map(|x| (x.olay, x.ayrinti)).collect();
    h.sort();
    let k8 = &K1[..8];
    assert_eq!(h, [("CIHAZ_BAGLA_RED".to_string(), format!("kimlik={k8}")), ("CIHAZ_BEKLIYOR_RED".to_string(), format!("kimlik={k8}"))]);
    assert!(d.v.hareketler(None, 300, None).unwrap().iter().all(|x| !x.ayrinti.contains(S1) && !x.ayrinti.contains(&"c".repeat(64))));
    d.saat.ilerlet(3601.0);
    yanlis("c");
    assert_eq!(olaylar(&d, None).iter().filter(|o| *o == "CIHAZ_BEKLIYOR_RED").count(), 2);
}

// --- Rust'a özgü yedek: kolay_baglama_uctan_uca_api_tarafi yalnızca `cihaz.otomatik == 1` denetimini taşır (Musteri'li karşılığı
// musteri_testler.rs'te test_bekleme ile taşındı; öbür üç yedek test orada tamamen karşılandığı için silindi) ---

#[test]
fn kolay_baglama_uctan_uca_api_tarafi() {
    let d = kur(Some(tamam()));
    let (n, _pw) = d.v.musteri_ekle("", Some("musteri-parola-1")).unwrap();
    bildir(&d);
    assert!(d.v.bekleyen_ayir(K1, n, IP1).unwrap());
    d.saat.ilerlet(120.0);
    let (c, j) = bildir(&d);
    assert!(c == 200 && j["durum"] == "baglan" && j["belirtec"].as_str().unwrap().len() > 30);
    let (c, g) = bagla(&d, j["belirtec"].as_str().unwrap());
    assert!(c == 200 && g["numara"] == n.to_string() && g["tunel_ip"] == "10.99.0.10" && g["yedek_hedefi"] == format!("wfc-{n}@10.99.0.1:"));
    assert!(g["cihaz_anahtari"].as_str().unwrap().len() > 30);
    let m = mus(&d, n);
    assert_eq!((g["tuz"].as_str().unwrap(), g["ozet"].as_str().unwrap()), (m.tuz.as_str(), m.ozet.as_str()));
    assert_eq!(d.q.isler(), vec![("cihaz-ekle".to_string(), json!({"numara": n.to_string(), "wg_pub": WG, "ssh_pub": SSH}))]);
    let cz = d.v.bagli_cihaz(n).unwrap().unwrap();
    assert!(cz.tunel_ip == "10.99.0.10" && cz.wg_pub == WG && cz.otomatik == 1);
    let h: Vec<_> = d.v.hareketler(Some(n), 300, Some("CIHAZ_BAGLANDI")).unwrap();
    assert!(h.len() == 1 && h[0].ayrinti.contains("otomatik"));
    assert_eq!(kod(&bagla(&d, j["belirtec"].as_str().unwrap()).1), "taninmadi"); // belirteç tek kullanımlık
    // cihaz cevabı alamadı, yine bildiriyor: aynı wg anahtarıyla yeniden baglan
    d.saat.ilerlet(120.0);
    let (c, j) = bildir(&d);
    assert!(c == 200 && j["durum"] == "baglan");
    assert!(bagla(&d, j["belirtec"].as_str().unwrap()).0 == 200);
    // başka wg anahtarıyla gelen aynı kimlik bağa dokunamaz
    d.saat.ilerlet(120.0);
    assert_eq!(bildir_tam(&d, K1, S1, WG2, IP1, "00:0e:c4:ce:a0:9b", "0.4.13").1, json!({"durum": "bekle"}));
    assert_eq!(d.v.bagli_cihaz(n).unwrap().unwrap().wg_pub, WG);
}

