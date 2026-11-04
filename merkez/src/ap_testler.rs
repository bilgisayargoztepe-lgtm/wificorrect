//! ap.rs testleri: Python tests/test_ap.py (21). Cihaz geçidi kurulumu `cihaz_testler.rs`ta.
use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::altin::Kayit;
use crate::cihaz::testler::{kur as kur_cihaz, musteri_ac, CIHAZ, IP, WG2};
use crate::cihaz::{self, Cihaz};
use crate::guvenlik::TekKullanimlik;
use crate::merkez::{Merkez, GUVENLIK_BASLIKLARI};
use crate::test_yardim::{self as y, Ortam};

const HOST_AP: (&str, &str) = ("Host", "ap.wificorrect.com");
const PANEL_GIRIS: &str = "https://panel.wificorrect.com/giris";
const AP_GIRIS: &str = "https://ap.wificorrect.com/_ap-giris?t=";
const CADDYFILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/sunucu/merkez/Caddyfile");

fn kur() -> (Ortam, Arc<Cihaz>, Ap, i64) {
    let (o, _m, c, n) = kur_cihaz();
    let a = Ap::new(c.clone(), o.saat.saat());
    (o, c, a, n)
}

fn wcc_ac(c: &Cihaz, o: &Ortam, n: i64, rol: &str) -> String {
    let ozet = o.veri.musteri(n).unwrap().unwrap().ozet;
    c.oturumlar.create(&n.to_string(), rol, o.saat.t(), &ozet, BTreeMap::new())
}

fn gecis_ip(c: &Cihaz, wcc: &str, ip: &str) -> Yanit {
    if wcc.is_empty() {
        y::istek(c, "GET", "/_ap-gecis", &[CIHAZ], "", "", ip)
    } else {
        y::istek(c, "GET", "/_ap-gecis", &[CIHAZ, ("Cookie", &format!("wcc={wcc}"))], "", "", ip)
    }
}

fn gecis(c: &Cihaz, wcc: &str) -> Yanit {
    gecis_ip(c, wcc, IP)
}

fn belirtec(r: &Yanit) -> String {
    assert!(r.durum == 303 && r.baslik("Location").is_some_and(|l| l.starts_with(AP_GIRIS)), "{} {:?}", r.durum, r.baslik("Location"));
    r.baslik("Location").unwrap()[AP_GIRIS.len()..].to_string()
}

fn giris(a: &Ap, t: &str, cerez: &str) -> Yanit {
    y::istek(a, "GET", "/_ap-giris", &[HOST_AP, ("Cookie", cerez)], "", &format!("t={t}"), IP)
}

/// Yeni AP oturumunun çerezi: `/_ap-giris`'in ilk Set-Cookie'si (ardından eski AP çerezlerinin silinmesi gelebilir).
fn wca(r: &Yanit) -> String {
    assert!(r.durum == 303 && r.baslik("Location") == Some("/"), "{} {:?}", r.durum, r.baslik("Location"));
    r.basliklar_hepsi("Set-Cookie")[0].split(';').next().unwrap().split_once('=').unwrap().1.to_string()
}

fn yetki(a: &Ap, wca_: &str) -> Yanit {
    if wca_.is_empty() {
        y::istek(a, "GET", "/ap-yetki", &[HOST_AP], "", "", IP)
    } else {
        y::istek(a, "GET", "/ap-yetki", &[HOST_AP, ("Cookie", &format!("wca={wca_}"))], "", "", IP)
    }
}

/// Cihaz paneli oturumu (wcc) + AP oturumu (wca).
fn ap_ac(c: &Cihaz, a: &Ap, o: &Ortam, n: i64, rol: &str) -> (String, String) {
    let w = wcc_ac(c, o, n, rol);
    let z = wca(&giris(a, &belirtec(&gecis(c, &w)), ""));
    (w, z)
}

fn getirme(site: &str, mod_: &str, hedef: &str) -> Vec<(String, String)> {
    vec![("Sec-Fetch-Site".into(), site.into()), ("Sec-Fetch-Mode".into(), mod_.into()), ("Sec-Fetch-Dest".into(), hedef.into())]
}

fn getirme_ilk(site: &str) -> Vec<(String, String)> {
    getirme(site, "navigate", "document")
}

/// Sabit başlıklar + ek başlıklar → `y::istek` için dilim.
fn basliklar<'a>(sabit: &[(&'a str, &'a str)], ek: &'a [(String, String)]) -> Vec<(&'a str, &'a str)> {
    sabit.iter().copied().chain(ek.iter().map(|(k, v)| (k.as_str(), v.as_str()))).collect()
}

fn hareket_sayisi(o: &Ortam, n: i64, olay: &str) -> usize {
    o.veri.hareketler(Some(n), 300, None).unwrap().iter().filter(|h| h.olay == olay).count()
}

// --- belirteç deposu, oturum temizliği ---
#[test]
fn tek_kullanimlik_belirtec() {
    let _k = Kayit::bas("test_ap", "test_tek_kullanimlik_belirtec");
    let b: TekKullanimlik<serde_json::Value> = TekKullanimlik::new(60.0);
    let t = b.uret(json!({"x": 1}), 1000.0);
    assert_eq!(b.tuket(&t, 1059.0), Some(json!({"x": 1})));
    assert_eq!(b.tuket(&t, 1059.0), None); // bir kez
    let t = b.uret(json!("d"), 1000.0);
    assert_eq!(b.tuket(&t, 1060.0), None); // 60 sn dolunca geçersiz
    assert_eq!(b.tuket("", 1000.0), None);
    assert_eq!(b.tuket("yok", 1000.0), None);
    // Python'daki `len(b._b) == 1` (süresi dolanlar her üretimde silinir) özel alan: guvenlik::testler::tek_kullanimlik_deger_ve_temizlik
}

#[test]
fn oturumlar_ek_alan_ve_suresi_dolanlar_silinir() {
    let _k = Kayit::bas("test_ap", "test_oturumlar_ek_alan_ve_suresi_dolanlar_silinir");
    let s = Oturumlar::new(1800.0, 1800.0);
    let mut ek = BTreeMap::new();
    ek.insert("wcc".to_string(), Value::from("w"));
    ek.insert("user".to_string(), Value::from("baskasi"));
    let t = s.create("1234567", "musteri", 100.0, "", ek);
    let o = s.get(&t, 100.0).unwrap();
    assert!(o.ek["wcc"] == "w" && o.user == "1234567"); // ek alan asıl alanları ezemez
    assert!(s.get(&t, 1500.0).is_some() && s.get(&t, 1901.0).is_none()); // bakılsa da en çok 30 dk
    let (t1, t2) = (s.create("1", "musteri", 100.0, "", BTreeMap::new()), s.create("2", "musteri", 5000.0, "", BTreeMap::new()));
    // süresi dolan oturum yeni oturum açılırken silinir (Python `len(s._s) == 1`: özel alan; guvenlik::testler::create_suresi_dolmuslari_siler)
    assert!(s.get(&t1, 5000.0).is_none() && s.get(&t2, 5000.0).is_some());
}

// --- /_ap-gecis (cihaz.wificorrect.com, forward_auth dışı) ---
#[test]
fn gecis_belirteci_wca_cerezi_ve_yetki() {
    let _k = Kayit::bas("test_ap", "test_gecis_belirteci_wca_cerezi_ve_yetki");
    let (o, c, a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    let r = gecis(&c, &w);
    let t = belirtec(&r);
    assert!(r.baslik("Cache-Control") == Some("no-store") && r.baslik("Set-Cookie").is_none());
    let h = &o.veri.hareketler(Some(n), 300, None).unwrap()[0];
    assert_eq!((h.olay.as_str(), h.kim.as_str(), h.ip.as_str(), h.musteri, h.ayrinti.as_str()), ("AP_GECIS", n.to_string().as_str(), IP, Some(n), "rol=musteri"));
    let r = giris(&a, &t, "");
    let z = wca(&r);
    assert_eq!(r.basliklar_hepsi("Set-Cookie").len(), 1); // eski AP çerezi yoktu: yalnızca yeni wca
    let sc = r.basliklar_hepsi("Set-Cookie")[0];
    let parcalar: Vec<&str> = sc.split(';').map(str::trim).collect();
    // Strict (final review I2): başka sitenin üst düzey GET gezinmesi de AP'ye oturumlu istek gönderemez
    assert!(parcalar[0].starts_with("wca="));
    for p in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/", "Max-Age=1800"] {
        assert!(parcalar.contains(&p), "{p}");
    }
    assert!(!sc.to_lowercase().contains("domain")); // yalnızca ap.wificorrect.com (alt/üst alanlara gitmez)
    assert_eq!(r.baslik("Cache-Control"), Some("no-store"));
    // yeni AP oturumu (müşteri değişimi): kökün depolaması — Service Worker kayıtları dahil — silinir (final review C1).
    // "cookies" yok: kayıtlı alan adının (wificorrect.com) bütün çerezlerini, yeni wca'yı da siler. "cache" yok
    // (re-review m4): bu kökteki her cevap no-store, önbellekte bir şey yok; Chromium'da önbellek silme yönlendirmeyi
    // gözle görülür geciktirir
    assert_eq!(r.baslik("Clear-Site-Data"), Some("\"storage\""));
    let yt = yetki(&a, &z);
    assert!(yt.durum == 200 && yt.govde.is_empty());
    // AP kipinde kullanıcı ayrı başlıkta (final review M2): 0.4.15 öncesi cihaz X-WFC-Kullanici'li isteği oturumlu panel
    // isteği sayar ve paneli ap.wificorrect.com kökünde verirdi; yeni başlığı tanımaz, isteği oturumsuz sayar
    assert_eq!((yt.baslik("X-WFC-Cihaz"), yt.baslik("X-WFC-AP-Kullanici"), yt.baslik("X-WFC-AP")), (Some("10.99.0.11"), Some(n.to_string().as_str()), Some("1")));
    assert!(yt.baslik("X-WFC-Kullanici").is_none());
    assert_eq!(yt.baslik("Cache-Control"), Some("no-store"));
    // belirteç bir kez: ikinci kullanımda sade Türkçe sayfa, çerez yok
    let r = giris(&a, &t, "");
    assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none() && r.baslik("Content-Type").unwrap().contains("text/html"));
    let s = r.govde_metin();
    assert!(s.contains("geçersiz") && s.contains("https://cihaz.wificorrect.com/"));
    assert_eq!(giris(&a, "", "").durum, 403);
    assert_eq!(y::istek(&a, "GET", "/_ap-giris", &[HOST_AP], "", "", IP).durum, 403);
}

#[test]
fn gecis_oturumsuz_panel_girisine() {
    let _k = Kayit::bas("test_ap", "test_gecis_oturumsuz_panel_girisine");
    let (o, c, _a, n) = kur();
    for w in ["", "uydurma"] {
        let r = gecis(&c, w);
        assert!(r.durum == 303 && r.baslik("Location") == Some(PANEL_GIRIS), "{w}");
    }
    let w = wcc_ac(&c, &o, n, "musteri");
    o.veri.askiya_al(n, true).unwrap(); // lisans kapalı: cihaz paneli oturumu da geçersiz
    assert_eq!(gecis(&c, &w).baslik("Location"), Some(PANEL_GIRIS));
    assert_eq!(hareket_sayisi(&o, n, "AP_GECIS"), 0);
}

/// Yönetimden cihaz paneline geçen yönetici (wcc rolü "yonetici"): hareket "admin"; cihaza giden kullanıcı yine müşteri
/// numarası (cihaz X-WFC-Kullanici'yi kendi bağındaki numarayla karşılaştırır, cihaz-yetki ile aynı).
#[test]
fn yonetici_gecisi() {
    let _k = Kayit::bas("test_ap", "test_yonetici_gecisi");
    let (o, c, a, n) = kur();
    let (_, z) = ap_ac(&c, &a, &o, n, "yonetici");
    let h = &o.veri.hareketler(Some(n), 300, None).unwrap()[0];
    assert_eq!((h.olay.as_str(), h.kim.as_str(), h.ayrinti.as_str()), ("AP_GECIS", "admin", "rol=yonetici"));
    let yt = yetki(&a, &z);
    assert!(yt.durum == 200 && yt.baslik("X-WFC-AP-Kullanici") == Some(n.to_string().as_str()) && yt.baslik("X-WFC-AP") == Some("1"));
}

/// AP arayüzü (ap.wificorrect.com) panel ve cihazla aynı "site": kötü niyetli bir AP'nin betiği çerçeve/fetch/yeni
/// pencereyle /_ap-gecis'i tetikleyip tarayıcıdaki BAŞKA müşterinin cihaz paneli oturumuyla (wcc, aynı-site isteklerde
/// gönderilir) wca aldıramamalı. Fetch Metadata: yalnızca cihaz.wificorrect.com sayfasından (same-origin) ya da elle yazılan
/// adresle (none) üst düzey gezinme; başlıkları göndermeyen eski tarayıcı geçer.
#[test]
fn gecis_yalnizca_cihaz_panelinden_ust_duzey_gezinmeyle() {
    let _k = Kayit::bas("test_ap", "test_gecis_yalnizca_cihaz_panelinden_ust_duzey_gezinmeyle");
    let (o, c, a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    let cookie = format!("wcc={w}");
    let kotuler = [
        getirme_ilk("same-site"),
        getirme_ilk("cross-site"),
        getirme("same-origin", "navigate", "iframe"),
        getirme("same-origin", "no-cors", "empty"),
        getirme("same-origin", "cors", "empty"),
        getirme("same-origin", "navigate", "image"),
    ];
    for kotu in &kotuler {
        let h = basliklar(&[CIHAZ, ("Cookie", &cookie)], kotu);
        let r = y::istek(&*c, "GET", "/_ap-gecis", &h, "", "", IP);
        assert!(r.durum == 403 && r.baslik("Location").is_none() && r.govde_metin().contains("cihaz panelinden"), "{kotu:?}");
    }
    // (Python `c.ap_belirtecleri._b == {}`: özel alan; reddedilen istekte belirteç üretilmediği aşağıda hareket sayısıyla da görülür)
    assert_eq!(hareket_sayisi(&o, n, "AP_GECIS"), 0);
    for iyi in [getirme_ilk("same-origin"), getirme_ilk("none"), Vec::new()] {
        let h = basliklar(&[CIHAZ, ("Cookie", &cookie)], &iyi);
        belirtec(&y::istek(&*c, "GET", "/_ap-gecis", &h, "", "", IP));
    }
    // /_ap-giris: yalnızca cihaz alanından yönlenen üst düzey gezinme (same-site; final review C1). Çerçeve/fetch/başka
    // site, AP sayfasının kendisinin gezinmesi (same-origin: A'nın AP'si A'nın belirtecini tarayıcıya ekip B sekmesini
    // kendine çevirirdi — oturum açtırma CSRF'i) ve elle yazılan adres (none) reddedilir; belirteç harcanmaz
    let t = belirtec(&gecis(&c, &w));
    let kotuler = [
        getirme_ilk("cross-site"),
        getirme("same-site", "navigate", "iframe"),
        getirme("same-site", "no-cors", "empty"),
        getirme_ilk("same-origin"),
        getirme_ilk("none"),
    ];
    for kotu in &kotuler {
        let h = basliklar(&[HOST_AP], kotu);
        let r = y::istek(&a, "GET", "/_ap-giris", &h, "", &format!("t={t}"), IP);
        assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none(), "{kotu:?}");
    }
    let ek = getirme_ilk("same-site");
    wca(&y::istek(&a, "GET", "/_ap-giris", &basliklar(&[HOST_AP], &ek), "", &format!("t={t}"), IP));
    // Sec-Fetch-* göndermeyen eski tarayıcı geçer (asıl koruma SameSite=Strict + IP)
    wca(&giris(&a, &belirtec(&gecis(&c, &w)), ""));
}

/// Belirtecin adres bağı: IPv4'te /24, IPv6'da /64 (IPv4'e eşlenmiş IPv6 yazımı IPv4 sayılır); adres olmayan değer
/// yalnızca kendisiyle aynı (eski tam eşitlik).
#[test]
fn ayni_ag_testi() {
    let _k = Kayit::bas("test_ap", "test_ayni_ag");
    let tablo: [(Option<&str>, Option<&str>, bool); 12] = [
        (Some("203.0.113.5"), Some("203.0.113.5"), true),
        (Some("203.0.113.5"), Some("203.0.113.254"), true),
        (Some("203.0.113.5"), Some("203.0.112.5"), false),
        (Some("10.0.0.1"), Some("10.0.1.1"), false),
        (Some("2001:db8:1:2::5"), Some("2001:db8:1:2:ffff:ffff:ffff:ffff"), true),
        (Some("2001:db8:1:2::5"), Some("2001:db8:1:3::5"), false),
        (Some("::ffff:203.0.113.5"), Some("203.0.113.77"), true),
        (Some("203.0.113.5"), Some("2001:db8::5"), false),
        (Some("203.0.113.5"), Some("::cb00:7105"), false),
        (Some("yerel"), Some("yerel"), true),
        (Some("yerel"), Some("203.0.113.5"), false),
        (None, Some("203.0.113.5"), false),
    ];
    for (x, y_, beklenen) in tablo {
        assert_eq!(ayni_ag(x, y_), beklenen, "{x:?} {y_:?}");
        assert_eq!(ayni_ag(y_, x), beklenen, "{y_:?} {x:?}");
    }
    // kapsam kimlikli IPv6 (%eth0) sorun olmaz
    assert!(ayni_ag(Some("2001:db8:1:2::5%eth0"), Some("2001:db8:1:2::9")));
}

fn ap_giris_ip(a: &Ap, t: &str, ip: &str) -> Yanit {
    let ek = getirme_ilk("same-site");
    y::istek(a, "GET", "/_ap-giris", &basliklar(&[HOST_AP], &ek), "", &format!("t={t}"), ip)
}

/// Belirteç /_ap-gecis'teki istemci adresinin ağına bağlı (final review C1 "bind the token", re-review I-1): kötü
/// niyetli bir AP betiğinin tarayıcıdan sızdırdığı belirteç başka ağdan kullanılamaz, denenen belirteç yanar. cihaz.* ve
/// ap.* ayrı bağlantılar: aynı ağdan (IPv4 /24, IPv6 /64) başka çıkış adresi (taşıyıcı NAT havuzu, çift hat) geçer. Ağ
/// farklıysa (iCloud Özel Geçiş, VPN) genel "geçersiz" değil, sebebi ve çözümü söyleyen ayrı sayfa (yeniden denemek
/// aynı yerde takılırdı).
#[test]
fn belirtec_veren_tarayicinin_agina_bagli() {
    let _k = Kayit::bas("test_ap", "test_belirtec_veren_tarayicinin_agina_bagli");
    let (o, c, a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    for (g_ip, c_ip) in [("203.0.113.5", "203.0.113.200"), ("2001:db8:1:2::5", "2001:db8:1:2:aaaa:bbbb:cccc:dddd"), ("203.0.113.5", "::ffff:203.0.113.9")] {
        wca(&ap_giris_ip(&a, &belirtec(&gecis_ip(&c, &w, g_ip)), c_ip));
    }
    for (g_ip, c_ip) in [
        ("203.0.113.5", "198.51.100.66"),
        ("203.0.113.5", "203.0.114.5"),
        ("2001:db8:1:2::5", "2001:db8:1:3::5"),
        ("203.0.113.5", "2001:db8:1:2::5"),
        ("203.0.113.5", "yerel"),
    ] {
        let t = belirtec(&gecis_ip(&c, &w, g_ip));
        let (r, gunluk) = web::log_yakala(|| ap_giris_ip(&a, &t, c_ip));
        let s = r.govde_metin().replace("&#x27;", "'").replace("&#39;", "'").replace("&amp;", "&");
        assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none() && r.baslik("Content-Type").unwrap().contains("text/html"), "{c_ip}");
        assert_eq!(r.baslik("Cache-Control"), Some("no-store"), "{c_ip}");
        assert!(
            s.contains("Bağlantınız farklı bir internet adresinden geldi (ör. iCloud Özel Geçiş, VPN). Özel Geçiş/VPN'i kapatıp cihaz panelinden yeniden deneyin.")
                && !s.contains("geçersiz"),
            "{c_ip}"
        );
        assert!(s.contains("href=\"https://cihaz.wificorrect.com/\""), "{c_ip}");
        // teşhis için iki adres de günlükte (merkez journal)
        assert!(gunluk.contains(&format!("AP_GIRIS_RED adres_farkli ip={c_ip} gecis_ip={g_ip}")), "{c_ip}: {gunluk:?}");
        // belirteç yandı: aynı ağdan da artık genel "geçersiz" sayfası
        let r = ap_giris_ip(&a, &t, g_ip);
        assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none() && r.govde_metin().contains("geçersiz"), "{c_ip}");
    }
    wca(&giris(&a, &belirtec(&gecis(&c, &w)), "")); // aynı tarayıcıdan (adresten) yenisi açılır
}

/// Bütün müşterilerin AP'leri tek kökte: önceki AP'nin çerezleri (ör. TP-Link'in betikle yazdığı Authorization = Basic
/// kullanıcı:parola) sonraki müşterinin AP'sine gitmesin (final review I1). /_ap-giris tarayıcının bu kökte gönderdiği
/// (Path=/) çerezleri siler; yalnızca geçerli çerez adları (başlığa enjeksiyon yok), en çok ESKI_CEREZ_EN_COK.
#[test]
fn yeni_ap_oturumu_eski_ap_cerezlerini_siler() {
    let _k = Kayit::bas("test_ap", "test_yeni_ap_oturumu_eski_ap_cerezlerini_siler");
    let (o, c, a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    let r = giris(&a, &belirtec(&gecis(&c, &w)), "wca=eski; SID=abc; Authorization=Basic%20YWRtaW46eA%3D%3D; kötü ad=1; SID=2");
    let sc = r.basliklar_hepsi("Set-Cookie");
    assert!(sc[0].starts_with("wca=") && !sc[0].starts_with("wca=eski;"));
    assert_eq!(sc[1..], ["SID=; Path=/; Max-Age=0; Secure", "Authorization=; Path=/; Max-Age=0; Secure"]);
    let cok: Vec<String> = (0..ESKI_CEREZ_EN_COK + 10).map(|i| format!("c{i}=1")).collect();
    let r = giris(&a, &belirtec(&gecis(&c, &w)), &cok.join("; "));
    assert_eq!(r.basliklar_hepsi("Set-Cookie").len(), 1 + ESKI_CEREZ_EN_COK);
}

#[test]
fn gecis_hiz_siniri() {
    let _k = Kayit::bas("test_ap", "test_gecis_hiz_siniri");
    let (o, c, _a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    for _ in 0..cihaz::AP_GECIS_SINIRI {
        belirtec(&gecis(&c, &w));
    }
    let r = gecis(&c, &w);
    assert!(r.durum == 429 && r.baslik("Location").is_none() && r.govde_metin().contains("dakika"));
    assert_eq!(hareket_sayisi(&o, n, "AP_GECIS"), cihaz::AP_GECIS_SINIRI); // hareket şişmez
    o.saat.ilerlet(61.0);
    belirtec(&gecis(&c, &w));
}

// --- /_ap-giris: belirteç süresi, altındaki oturum ---
#[test]
fn belirtec_suresi_ve_cihaz_oturumu() {
    let _k = Kayit::bas("test_ap", "test_belirtec_suresi_ve_cihaz_oturumu");
    let (o, c, a, n) = kur();
    let w = wcc_ac(&c, &o, n, "musteri");
    let t = belirtec(&gecis(&c, &w));
    o.saat.ilerlet(61.0);
    assert_eq!(giris(&a, &t, "").durum, 403); // 60 sn geçti
    let t = belirtec(&gecis(&c, &w));
    y::istek(&*c, "GET", "/_cikis", &[CIHAZ, ("Cookie", &format!("wcc={w}"))], "", "", IP); // cihaz panelinden çıkış
    let r = giris(&a, &t, "");
    assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none());
    let w = wcc_ac(&c, &o, n, "musteri");
    let t = belirtec(&gecis(&c, &w));
    o.veri.parola_koy(n, "yeni-parola-12").unwrap(); // belirteç alındıktan sonra parola değişti
    assert_eq!(giris(&a, &t, "").durum, 403);
}

#[test]
fn yeni_giris_eski_ap_oturumunu_kapatir() {
    let _k = Kayit::bas("test_ap", "test_yeni_giris_eski_ap_oturumunu_kapatir");
    let (o, c, a, n) = kur();
    let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
    let z2 = wca(&giris(&a, &belirtec(&gecis(&c, &w)), &format!("wca={z}")));
    assert!(z2 != z && yetki(&a, &z).durum == 403 && yetki(&a, &z2).durum == 200);
}

// --- /ap-yetki: wca + altındaki wcc ---
#[test]
fn wca_en_cok_30_dakika_ve_cihaz_oturumunu_canli_tutar() {
    let _k = Kayit::bas("test_ap", "test_wca_en_cok_30_dakika_ve_cihaz_oturumunu_canli_tutar");
    let (o, c, a, n) = kur();
    let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
    let bas = o.saat.t();
    o.saat.koy(bas + 1500.0);
    assert_eq!(yetki(&a, &z).durum, 200); // AP kullanımı cihaz paneli oturumunu da canlı tutar (boşta 30 dk)
    o.saat.koy(bas + 1799.0);
    assert_eq!(yetki(&a, &z).durum, 200);
    o.saat.koy(bas + 1801.0);
    assert_eq!(yetki(&a, &z).durum, 403); // 30 dk mutlak: wcc hâlâ geçerli olsa da
    assert_eq!(y::istek(&*c, "GET", "/cihaz-yetki", &[CIHAZ, ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 200);
}

#[test]
fn wca_cihaz_oturumu_dusunce_gecersiz() {
    type Bozan = fn(&Ortam, &Cihaz, i64, &str);
    let durumlar: [(&str, Bozan); 6] = [
        ("cikis", |_, c, _, w| {
            y::istek(c, "GET", "/_cikis", &[CIHAZ, ("Cookie", &format!("wcc={w}"))], "", "", IP);
        }),
        ("parola", |o, _, n, _| {
            o.veri.parola_koy(n, "yeni-parola-12").unwrap();
        }),
        ("lisans", |o, _, n, _| o.veri.askiya_al(n, true).unwrap()),
        ("uyelik", |o, _, n, _| o.veri.uyelik(n, "bitti").unwrap()),
        ("bag", |o, _, n, _| {
            o.veri.serbest_birak(n, true).unwrap();
        }),
        ("tunel", |o, _, n, _| {
            let id = o.veri.bagli_cihaz(n).unwrap().unwrap().id;
            o.veri.cihaz_guncelle(id, &[("tunel_ip", "10.99.0.77")]).unwrap();
        }),
    ];
    let _k = Kayit::bas("test_ap", "test_wca_cihaz_oturumu_dusunce_gecersiz");
    for (ad, bozan) in durumlar {
        let (o, c, a, n) = kur();
        let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
        assert_eq!(yetki(&a, &z).durum, 200, "{ad}");
        bozan(&o, &c, n, &w);
        let yt = yetki(&a, &z);
        assert!(yt.durum == 403 && !yt.basliklar.iter().any(|(k, _)| k.starts_with("X-WFC")), "{ad}");
    }
}

#[test]
fn cihaz_degisince_gecersiz_ayni_tunel_adresi_olsa_da() {
    let _k = Kayit::bas("test_ap", "test_cihaz_degisince_gecersiz_ayni_tunel_adresi_olsa_da");
    let (o, c, a, n) = kur();
    let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
    o.veri.serbest_birak(n, true).unwrap();
    let yeni_cihaz = o.veri.cihaz_bagla(n, WG2, crate::cihaz::testler::SSH, "", "", "").unwrap().0;
    o.veri.cihaz_guncelle(yeni_cihaz, &[("tunel_ip", "10.99.0.11")]).unwrap(); // yeni cihaz, aynı tünel adresi
    assert_eq!(y::istek(&*c, "GET", "/cihaz-yetki", &[CIHAZ, ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 200);
    assert_eq!(yetki(&a, &z).durum, 403); // AP oturumu açıldığı cihaza bağlı
}

#[test]
fn baska_musterinin_cihazina_gidemez() {
    let _k = Kayit::bas("test_ap", "test_baska_musterinin_cihazina_gidemez");
    let (o, c, a, n) = kur();
    let n2 = musteri_ac(&o, "musteri-parola-2", WG2, "10.99.0.12");
    let (_, z) = ap_ac(&c, &a, &o, n, "musteri");
    let (_, z2) = ap_ac(&c, &a, &o, n2, "musteri");
    let (yt, yt2) = (yetki(&a, &z), yetki(&a, &z2));
    assert_eq!((yt.baslik("X-WFC-Cihaz"), yt.baslik("X-WFC-AP-Kullanici")), (Some("10.99.0.11"), Some(n.to_string().as_str())));
    assert_eq!((yt2.baslik("X-WFC-Cihaz"), yt2.baslik("X-WFC-AP-Kullanici")), (Some("10.99.0.12"), Some(n2.to_string().as_str())));
    // müşteri 1 cihazsız kalınca onun AP oturumu hiçbir cihaza gitmez (başkasınınkine de)
    o.veri.serbest_birak(n, true).unwrap();
    assert!(yetki(&a, &z).durum == 403 && yetki(&a, &z2).baslik("X-WFC-Cihaz") == Some("10.99.0.12"));
}

#[test]
fn yetkisiz_sayfa() {
    let _k = Kayit::bas("test_ap", "test_yetkisiz_sayfa");
    let (o, c, a, n) = kur();
    let yt = yetki(&a, "");
    assert!(yt.durum == 403 && yt.baslik("Content-Type").unwrap().contains("text/html") && yt.baslik("Set-Cookie").is_none());
    assert!(!yt.basliklar.iter().any(|(k, _)| k.starts_with("X-WFC")));
    let s = yt.govde_metin();
    assert!(s.contains("AP erişimi kapalı") && s.contains("AP kurulumu") && s.contains("https://cihaz.wificorrect.com/"));
    // temizleyerek çıkış bağlantısı (aynı kökte; tarayıcıdaki AP verisi silinir)
    assert!(s.contains("href=\"/_ap-cikis\""));
    // çerçeveli AP arayüzlerinde (TP-Link userRpm frameset) sayfa çerçevenin içinde görünür ve bağlantılar üst pencereyi
    // açar (re-review m1): çerçevedeki gezinme Sec-Fetch-Dest: frame olur, /_ap-cikis onu reddeder; cihaz paneli
    // (frame-ancestors 'none') çerçevede boş görünür
    assert!(s.contains("<a href=\"/_ap-cikis\" target=\"_top\">") && s.contains("<a href=\"https://cihaz.wificorrect.com/\" target=\"_top\">"));
    // yalnızca aynı kökenden çerçevelenebilir (aynı kökendeki AP betiği bu adrese zaten gidebilir; sayfada sır yok);
    // merkezin diğer cevaplarında DENY kalır (test_merkez)
    assert_eq!(yt.baslik("X-Frame-Options"), Some("SAMEORIGIN"));
    let csp = GUVENLIK_BASLIKLARI.iter().find(|(k, _)| *k == "Content-Security-Policy").unwrap().1;
    assert_eq!(yt.baslik("Content-Security-Policy"), Some(format!("{csp}; frame-ancestors 'self'").as_str()));
    let yt = yetki(&a, "uydurma");
    // bozuk/eskimiş çerez silinir
    assert!(yt.durum == 403 && yt.baslik("Set-Cookie").unwrap().contains("Max-Age=0") && yt.baslik("Set-Cookie").unwrap().contains("SameSite=Strict"));
    // cihaz paneli çerezi (wcc) AP alanında geçmez
    let w = wcc_ac(&c, &o, n, "musteri");
    assert_eq!(y::istek(&a, "GET", "/ap-yetki", &[HOST_AP, ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 403);
}

#[test]
fn cikis() {
    let _k = Kayit::bas("test_ap", "test_cikis");
    let (o, c, a, n) = kur();
    let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
    // başka siteden (ya da çerçeve/resim olarak) çıkış yok: her şeyi silen çıkış oturum kapatma CSRF'i olmasın
    let cookie = format!("wca={z}");
    let kotuler = [getirme_ilk("cross-site"), getirme("same-origin", "no-cors", "image"), getirme("same-site", "navigate", "iframe")];
    for kotu in &kotuler {
        let r = y::istek(&a, "GET", "/_ap-cikis", &basliklar(&[HOST_AP, ("Cookie", &cookie)], kotu), "", "", IP);
        assert!(r.durum == 403 && r.baslik("Set-Cookie").is_none() && r.baslik("Clear-Site-Data").is_none(), "{kotu:?}");
    }
    assert_eq!(yetki(&a, &z).durum, 200);
    let ek = getirme_ilk("same-origin");
    let r = y::istek(&a, "GET", "/_ap-cikis", &basliklar(&[HOST_AP, ("Cookie", &cookie)], &ek), "", "", IP);
    assert!(r.durum == 303 && r.baslik("Location") == Some("https://cihaz.wificorrect.com/"));
    assert!(r.baslik("Set-Cookie").unwrap().starts_with("wca=;") && r.baslik("Set-Cookie").unwrap().contains("Max-Age=0"));
    // çıkış tarayıcıdaki AP verisini de siler (final review C1/I1): AP'lerin çerezleri, HTTP kimlik bilgileri, depolama ve
    // Service Worker'lar. "cookies" kayıtlı alan adının bütün çerezlerini siler: panel/yönetim/cihaz oturumları da kapanır
    assert_eq!(r.baslik("Clear-Site-Data"), Some("\"cache\", \"cookies\", \"storage\""));
    assert_eq!(yetki(&a, &z).durum, 403);
    // sunucudaki cihaz paneli oturumu düşmez (tarayıcı Clear-Site-Data'yı uygulamazsa panel açık kalır)
    assert_eq!(y::istek(&*c, "GET", "/cihaz-yetki", &[CIHAZ, ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 200);
    // dönüş yalnızca cihaz.wificorrect.com'da bir yol
    for (donus, yer) in [
        ("/ap-kurulum", "https://cihaz.wificorrect.com/ap-kurulum"),
        ("//kotu.example", "https://cihaz.wificorrect.com/"),
        ("https://kotu.example/", "https://cihaz.wificorrect.com/"),
        ("/\\kotu.example", "https://cihaz.wificorrect.com/"),
        ("/%0d%0aSet-Cookie:%20x=1", "https://cihaz.wificorrect.com/"), // başlık enjeksiyonu yok
        ("/%C5%9Fube", "https://cihaz.wificorrect.com/%C5%9Fube"),       // Location latin-1'e sığmalı
    ] {
        let r = y::istek(&a, "GET", "/_ap-cikis", &[HOST_AP], "", &format!("donus={donus}"), IP);
        assert_eq!(r.baslik("Location"), Some(yer), "{donus}");
    }
}

#[test]
fn alan_adi_ayrimi() {
    let _k = Kayit::bas("test_ap", "test_alan_adi_ayrimi");
    let (o, c, a, n) = kur();
    let a = Arc::new(a);
    let mut mz = Merkez::new();
    mz.ekle("cihaz", c.clone());
    mz.ekle("ap", a.clone());
    let (w, z) = ap_ac(&c, &a, &o, n, "musteri");
    let yt = y::istek(&mz, "GET", "/ap-yetki", &[("Host", "ap.wificorrect.com"), ("Cookie", &format!("wca={z}"))], "", "", IP);
    assert!(yt.durum == 200 && yt.baslik("X-WFC-AP") == Some("1"));
    // AP uçları cihaz alanında, cihaz uçları AP alanında yok
    assert_eq!(y::istek(&mz, "GET", "/ap-yetki", &[("Host", "cihaz.wificorrect.com"), ("Cookie", &format!("wca={z}"))], "", "", IP).durum, 404);
    assert_eq!(y::istek(&mz, "GET", "/_ap-gecis", &[("Host", "ap.wificorrect.com"), ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 404);
    assert_eq!(y::istek(&mz, "GET", "/cihaz-yetki", &[("Host", "ap.wificorrect.com"), ("Cookie", &format!("wcc={w}"))], "", "", IP).durum, 404);
    assert_eq!(y::istek(&mz, "GET", "/_giris", &[("Host", "ap.wificorrect.com")], "", "t=x", IP).durum, 404);
    assert_eq!(crate::merkez::ALANLAR.iter().find(|(h, _)| *h == "ap.wificorrect.com").map(|(_, a)| *a), Some("ap"));
}

// --- Caddyfile ---
fn yorumsuz(metin: &str) -> String {
    metin.lines().filter(|s| !s.trim().starts_with('#')).collect::<Vec<_>>().join("\n")
}

/// Üst düzey `ad {` bloğunun içi (yorumlar atılmış; {yer.tutucu}lar aynı satırda dengeli).
fn caddy_blogu(metin: &str, ad: &str) -> String {
    let metin = yorumsuz(metin);
    let bas = format!("{ad} {{");
    let konum = metin
        .match_indices(&bas)
        .map(|(i, _)| i)
        .find(|&i| (i == 0 || metin.as_bytes()[i - 1] == b'\n') && metin[i + bas.len()..].starts_with('\n'))
        .unwrap_or_else(|| panic!("{ad}"));
    let acilis = konum + bas.len() - 1;
    let mut derinlik = 0;
    for (i, ch) in metin[acilis..].char_indices() {
        match ch {
            '{' => derinlik += 1,
            '}' => {
                derinlik -= 1;
                if derinlik == 0 {
                    return metin[acilis + 1..acilis + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("{ad}: kapanmayan blok");
}

/// Python `re.search(a\n\s*b\n\s*c…)`: parçalar sırayla, aralarında boşluk (yeni satır dahil) olabilir; ilk parçanın başlangıcı.
fn dizi(metin: &str, parcalar: &[&str]) -> Option<usize> {
    metin.match_indices(parcalar[0]).map(|(i, _)| i).find(|&bas| {
        let mut kalan = &metin[bas + parcalar[0].len()..];
        parcalar[1..].iter().all(|p| {
            kalan = kalan.trim_start();
            match kalan.strip_prefix(p) {
                Some(k) => {
                    kalan = k;
                    true
                }
                None => false,
            }
        })
    })
}

/// `header_up Cookie "<ifade>" "<değiştirme>"`: ifade (Caddyfile'daki yazımıyla), değiştirme "_" olmalı (boş değiştirme
/// Caddy 2.6.2'de Cookie'yi ezer; KURULUM_GUNLUGU "Uzak panel").
fn cerez_ifadesi(blok: &str) -> String {
    let i = blok.find("header_up Cookie \"").expect("header_up Cookie") + "header_up Cookie \"".len();
    let j = i + blok[i..].find('"').unwrap();
    let kalan = &blok[j..];
    assert!(kalan.starts_with("\" \"_\""), "değiştirme _ olmalı: {kalan:?}");
    blok[i..j].to_string()
}

/// `\b(?:ad1|ad2)=[^;]*` ifadesinin (RE2) ASCII çerez başlığındaki anlamı: sözcük sınırında `ad=` ve `;`'a kadar olan kısım "_" olur.
fn cerez_degistir(baslik: &str, adlar: &[&str]) -> String {
    let b = baslik.as_bytes();
    let sozcuk = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut o = String::new();
    let mut i = 0;
    while i < b.len() {
        let eslesen = adlar.iter().find(|ad| baslik[i..].starts_with(&format!("{ad}=")) && (i == 0 || !sozcuk(b[i - 1])));
        if let Some(ad) = eslesen {
            let son = baslik[i + ad.len() + 1..].find(';').map(|k| i + ad.len() + 1 + k).unwrap_or(b.len());
            o.push('_');
            i = son;
        } else {
            let ch = baslik[i..].chars().next().unwrap();
            o.push(ch);
            i += ch.len_utf8();
        }
    }
    o
}

#[test]
fn caddyfile_ap_blogu() {
    let _k = Kayit::bas("test_ap", "test_caddyfile_ap_blogu");
    let metin = std::fs::read_to_string(CADDYFILE).expect("Caddyfile");
    let yorumsuz_ = yorumsuz(&metin);
    assert_eq!(yorumsuz_.matches('{').count(), yorumsuz_.matches('}').count());
    let ap = caddy_blogu(&metin, "ap.wificorrect.com");
    for b in ["X-WFC-Kullanici", "X-WFC-AP-Kullanici", "X-WFC-Cihaz", "X-WFC-AP"] {
        assert!(ap.contains(&format!("request_header -{b}\n")), "{b}"); // istemcinin gönderdiği kimlik başlıkları geçmez
    }
    // Service Worker betiği (kayıt ve güncelleme isteği "Service-Worker: script" taşır) hiçbir yere gitmez (final review C1);
    // adlı eşleyicili handle'lar yazıldığı sırayla denenir: ilk o
    let sw = dizi(&ap, &["@sw header Service-Worker script\n", "handle @sw {\n", "respond 403\n", "}"]).expect("@sw");
    assert!(sw < ap.find("handle @merkez {").unwrap() && ap.find("handle @merkez {").unwrap() < ap.find("handle {").unwrap());
    assert!(dizi(&ap, &["@merkez path /_ap-giris /_ap-cikis\n", "handle @merkez {\n", "reverse_proxy 127.0.0.1:8081\n"]).is_some());
    assert!(dizi(&ap, &["forward_auth 127.0.0.1:8081 {\n", "uri /ap-yetki\n", "copy_headers X-WFC-AP-Kullanici X-WFC-Cihaz X-WFC-AP\n"]).is_some());
    assert!(ap.contains("reverse_proxy {http.request.header.X-WFC-Cihaz}:8443 {"));
    assert!(dizi(&ap, &["transport http {\n", "tls\n", "tls_insecure_skip_verify\n", "}"]).is_some());
    assert!(ap.contains("header_up -X-WFC-Cihaz\n"));
    assert!(!ap.contains("header_up Host")); // Host ap.wificorrect.com kalır
    // AP'nin kendi arayüzünü bozacak başlıklar bilerek yok (satır içi betik, çerçeve, yanlış MIME türü, Referer denetimi)
    for b in ["Content-Security-Policy", "X-Frame-Options", "X-Content-Type-Options", "Referrer-Policy"] {
        assert!(!ap.contains(b), "{b}");
    }
    assert!(ap.contains("Strict-Transport-Security \"max-age=31536000\"") && ap.contains("-Server"));
    assert_eq!(cerez_ifadesi(&ap), r"\b(?:wca|wcc)=[^;]*");
    for (once, sonra) in [
        ("wca=abc; SessionID=xyz", "_; SessionID=xyz"),
        ("SessionID=xyz; wca=abc", "SessionID=xyz; _"),
        ("wcc=1; wca=2; Authorization=Basic%20YWRtaW46YWRtaW4%3D", "_; _; Authorization=Basic%20YWRtaW46YWRtaW4%3D"),
        ("xwca=1; wcax=2; _wca=3", "xwca=1; wcax=2; _wca=3"),
    ] {
        assert_eq!(cerez_degistir(once, &["wca", "wcc"]), sonra, "{once}");
    }
}

#[test]
fn caddyfile_cihaz_blogu_ap_gecisi() {
    let _k = Kayit::bas("test_ap", "test_caddyfile_cihaz_blogu_ap_gecisi");
    let metin = std::fs::read_to_string(CADDYFILE).expect("Caddyfile");
    let cz = caddy_blogu(&metin, "cihaz.wificorrect.com");
    assert!(cz.contains("@merkez path /_giris /_cikis /_ap-gecis\n")); // forward_auth'suz merkeze
    for b in ["X-WFC-Kullanici", "X-WFC-AP-Kullanici", "X-WFC-Cihaz", "X-WFC-AP"] {
        assert!(cz.contains(&format!("request_header -{b}\n")), "{b}"); // AP kipi başlıkları cihaz alanından cihaza asla ulaşmaz
    }
    assert!(cz.contains("copy_headers X-WFC-Kullanici X-WFC-Cihaz\n")); // cihaz-yetki AP başlığı vermez
    let i = cz.find("header_down Content-Security-Policy \"").unwrap() + "header_down Content-Security-Policy \"".len();
    let csp = &cz[i..i + cz[i..].find('"').unwrap()]; // cihaz panelinin sıkı CSP'si yerinde
    assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
    // form-action yönlendirmelere de uygulanır: "AP arayüzüne git" formu → /_ap-gecis → ap.wificorrect.com
    let j = csp.find("form-action ").unwrap() + "form-action ".len();
    let form_action: std::collections::BTreeSet<&str> = csp[j..j + csp[j..].find(';').unwrap()].split_whitespace().collect();
    let beklenen: std::collections::BTreeSet<&str> =
        ["'self'", "https://panel.wificorrect.com", "https://yonetim.wificorrect.com", "https://ap.wificorrect.com"].into_iter().collect();
    assert_eq!(form_action, beklenen);
    assert_eq!(cerez_ifadesi(&cz), r"\bwcc=[^;]*");
    assert_eq!(cerez_degistir("wcc=abc; wfc=def", &["wcc"]), "_; wfc=def");
}
