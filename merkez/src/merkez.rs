//! Alan adı dağıtıcısı ve istek işleyici (Python merkez.py): yonetim.wificorrect.com (hizmet sağlayıcı), panel (müşteri),
//! api (cihazlar), cihaz (cihaz paneline geçit), ap (AP arayüzüne geçit). Tek süreç, Caddy arkasında 127.0.0.1:8081.
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::ortak::py_strip;
use crate::web::{hata_yaniti, Basliklar, Istek, Panel, Yanit};

pub const DB: &str = "/var/lib/wificorrect/merkez/merkez.db";
pub const ALANLAR: [(&str, &str); 5] = [
    ("yonetim.wificorrect.com", "yonetim"),
    ("panel.wificorrect.com", "musteri"),
    ("api.wificorrect.com", "api"),
    ("cihaz.wificorrect.com", "cihaz"),
    ("ap.wificorrect.com", "ap"),
];
pub const GOVDE_SINIRI: i64 = 10_000;
pub const GUVENLIK_BASLIKLARI: [(&str, &str); 4] = [
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("X-Frame-Options", "DENY"),
    ("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:"),
];

/// Yalnızca 127.0.0.1 dinlenir; Caddy gelen X-Forwarded-For'a gerçek istemciyi SONA ekler.
pub fn istemci_ip(basliklar: &Basliklar) -> String {
    let son = basliklar.get("X-Forwarded-For").unwrap_or("").rsplit(',').next().unwrap_or("");
    let son = py_strip(son);
    if son.is_empty() { "yerel".into() } else { son.to_string() }
}

/// Python `int(metin)` (ASCII): boşluk kırpılır, işaret, rakamlar arası tek alt çizgi. Taşan değerler sınırı aşar.
pub(crate) fn py_int(s: &str) -> Option<i64> {
    let s = py_strip(s);
    let (eksi, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() || !b[b.len() - 1].is_ascii_digit() {
        return None;
    }
    let mut n: i64 = 0;
    let mut onceki_alt = false;
    for &c in b {
        if c == b'_' {
            if onceki_alt {
                return None;
            }
            onceki_alt = true;
            continue;
        }
        if !c.is_ascii_digit() {
            return None;
        }
        onceki_alt = false;
        n = n.saturating_mul(10).saturating_add((c - b'0') as i64);
    }
    Some(if eksi { -n } else { n })
}

/// İstek gövdesinin okunacak bayt sayısı (`Content-Length`): yoksa ya da boşsa 0 (Python gibi gövde boş);
/// sayı değilse, eksi ya da `GOVDE_SINIRI`'ndan büyükse `Err` (413).
pub fn govde_uzunlugu(basliklar: &Basliklar) -> Result<usize, ()> {
    let ham = basliklar.get("Content-Length").unwrap_or("");
    if ham.is_empty() {
        return Ok(0);
    }
    match py_int(ham) {
        Some(n) if (0..=GOVDE_SINIRI).contains(&n) => Ok(n as usize),
        _ => Err(()),
    }
}

/// Bir alan adının uygulaması.
pub trait Uygulama: Send + Sync {
    fn istek(&self, i: &Istek) -> Yanit;
}

/// Her `Panel` (yönetim, müşteri) bir uygulamadır: ortak akış `isle`, veritabanı hatası 500.
impl<T: Panel> Uygulama for T {
    fn istek(&self, i: &Istek) -> Yanit {
        match self.isle(i) {
            Ok(y) => y,
            Err(h) => hata_yaniti(&i.yontem, &i.yol, &format!("{h:?}")),
        }
    }
}

#[derive(Default)]
pub struct Merkez {
    uygulamalar: HashMap<&'static str, Arc<dyn Uygulama>>,
}

impl Merkez {
    pub fn new() -> Merkez {
        Merkez::default()
    }

    /// `ad`: `ALANLAR`'daki uygulama adı ("yonetim", "musteri", "api", "cihaz", "ap").
    pub fn ekle(&mut self, ad: &'static str, uygulama: Arc<dyn Uygulama>) {
        self.uygulamalar.insert(ad, uygulama);
    }

    pub fn istek(&self, i: &Istek) -> Yanit {
        let host = py_strip(i.basliklar.get("Host").unwrap_or("").split(':').next().unwrap_or("")).to_lowercase();
        let ad = ALANLAR.iter().find(|(h, _)| *h == host).map(|(_, a)| *a).unwrap_or("");
        match self.uygulamalar.get(ad) {
            Some(app) => app.istek(i),
            None => Yanit::metin(404, "Bulunamadı"),
        }
    }
}

/// Dağıtıcının kendisi de bir uygulama (testler Python'daki gibi uygulamaya ya da dağıtıcıya aynı yoldan istek yapar).
impl Uygulama for Merkez {
    fn istek(&self, i: &Istek) -> Yanit {
        Merkez::istek(self, i)
    }
}

fn panik_mesaji(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panik".into())
}

/// HTTP işleyicinin çekirdeği (Python `Isleyici._cevapla`): gövde sınırı (413), uygulamadaki panik (500, ayrıntı
/// journal'a), varsayılan güvenlik başlıkları (uygulamanın verdiği aynı adlı başlık varsayılanın yerine geçer).
pub fn cevapla(m: &Merkez, i: Istek) -> Yanit {
    let mut y = if govde_uzunlugu(&i.basliklar).is_err() {
        Yanit::metin(413, "İstek çok büyük")
    } else {
        match catch_unwind(AssertUnwindSafe(|| m.istek(&i))) {
            Ok(y) => y,
            Err(p) => hata_yaniti(&i.yontem, &i.yol, &panik_mesaji(p.as_ref())),
        }
    };
    // cevabın kendi verdiği güvenlik başlığı (ap.sayfa: aynı kökenden çerçevelenebilir) varsayılanın yerine geçer: iki
    // farklı X-Frame-Options / CSP satırı tarayıcıda belirsiz ya da en sıkısı olurdu
    for (k, v) in GUVENLIK_BASLIKLARI {
        if !y.basliklar.iter().any(|(ad, _)| ad.eq_ignore_ascii_case(k)) {
            y.basliklar.push((k.to_string(), v.to_string()));
        }
    }
    y
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::altin::Kayit;
    use crate::test_yardim as Y;
    use crate::veri::{Saat, Sonuc};
    use crate::web::{self, Ot, Taban};
    use std::collections::HashMap;

    /// Python test `Sahte`: "<ad> <yol> <ip>" döner.
    struct Sahte(&'static str);

    impl Uygulama for Sahte {
        fn istek(&self, i: &Istek) -> Yanit {
            Yanit::yeni(200, &[], format!("{} {} {}", self.0, i.yol, i.ip).into_bytes())
        }
    }

    fn istek(yontem: &str, host: &str) -> Istek {
        Istek::yeni(yontem, "/").baslik("Host", host).ip("1.1.1.1")
    }

    #[test]
    fn host_yonlendirme() {
        let _k = Kayit::bas("test_merkez", "test_host_yonlendirme");
        let mut m = Merkez::new();
        m.ekle("yonetim", Arc::new(Sahte("Y")));
        m.ekle("musteri", Arc::new(Sahte("M")));
        m.ekle("api", Arc::new(Sahte("A")));
        let host = |yontem: &str, yol: &str, h: &str| Y::istek(&m, yontem, yol, &[("Host", h)], "", "", "1.1.1.1");
        assert_eq!(host("GET", "/", "yonetim.wificorrect.com").govde, b"Y / 1.1.1.1");
        assert!(host("GET", "/", "PANEL.wificorrect.com:443").govde.starts_with(b"M"));
        assert!(host("POST", "/api/giris", "api.wificorrect.com").govde.starts_with(b"A"));
        for h in ["wificorrect.com", "evil.com", "", "yonetim.wificorrect.com.evil.com"] {
            assert_eq!(host("GET", "/", h).durum, 404, "{h}");
        }
        // Host başlığı hiç yoksa (Python testinde yok: kayda girmez)
        assert_eq!(m.istek(&Istek::yeni("GET", "/")).durum, 404);
    }

    #[test]
    fn bilinmeyen_alan_adi_404_bulunamadi() {
        let m = Merkez::new();
        let y = cevapla(&m, istek("GET", "yonetim.wificorrect.com"));
        assert_eq!(y.durum, 404);
        assert_eq!(y.govde_metin(), "Bulunamadı");
        assert_eq!(y.baslik("Content-Type"), Some("text/plain; charset=utf-8"));
        // uygulaması kayıtlı olmayan ama bilinen alan adı da aynı
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Sahte("A")));
        assert_eq!(cevapla(&m, istek("GET", "ap.wificorrect.com")).durum, 404);
    }

    #[test]
    fn istemci_ip_son_xff() {
        let _k = Kayit::bas("test_merkez", "test_istemci_ip_son_xff");
        let b = Basliklar::yeni(&[("X-Forwarded-For", "6.6.6.6, 203.0.113.5")]);
        assert_eq!(istemci_ip(&b), "203.0.113.5");
        assert_eq!(istemci_ip(&Basliklar::default()), "yerel");
        assert_eq!(istemci_ip(&Basliklar::yeni(&[("x-forwarded-for", "  ")])), "yerel");
        assert_eq!(istemci_ip(&Basliklar::yeni(&[("X-Forwarded-For", "1.2.3.4,")])), "yerel");
    }

    #[test]
    fn fail2ban_hatali_ve_kilitli_satirlari_yakalar() {
        let _k = Kayit::bas("test_merkez", "test_fail2ban_hatali_ve_kilitli_satirlari_yakalar");
        // failregex = "GIRIS_HATALI .* ip=<HOST>$" / "GIRIS_KILITLI .* ip=<HOST>$"
        let filtre = include_str!("../../scripts/sunucu/merkez/fail2ban-wc-merkez-filtre.conf");
        assert!(filtre.contains("GIRIS_HATALI .* ip=<HOST>$") && filtre.contains("GIRIS_KILITLI .* ip=<HOST>$"));
        for (olay, kul) in [("GIRIS_HATALI", "100001"), ("GIRIS_KILITLI", "serkan")] {
            let satir = web::giris_satiri(olay, "musteri", kul, "203.0.113.5");
            assert!(satir.starts_with(&format!("{olay} alan=musteri kullanici='")), "{satir}");
            assert!(satir.ends_with(" ip=203.0.113.5"), "{satir}");
        }
        // satır sonu enjeksiyonu: kullanıcı adı kaçışlanır, satır tek kalır ve ip ile biter
        let satir = web::giris_satiri("GIRIS_HATALI", "yonetici", "x\nGIRIS_HATALI alan=y kullanici='z' ip=6.6.6.6", "203.0.113.5");
        assert!(!satir.contains('\n') && satir.ends_with(" ip=203.0.113.5"), "{satir}");
        let jail = include_str!("../../scripts/sunucu/merkez/fail2ban-wc-merkez.conf");
        assert!(jail.contains("maxretry = 5") && jail.contains("192.168.1.0/24") && jail.contains("10.99.0.0/24"));
    }

    #[test]
    fn govde_sinirlari() {
        let ok = |l: Option<&str>| govde_uzunlugu(&match l {
            Some(v) => Basliklar::yeni(&[("content-length", v)]),
            None => Basliklar::default(),
        });
        assert_eq!(ok(None), Ok(0));
        assert_eq!(ok(Some("")), Ok(0));
        assert_eq!(ok(Some("0")), Ok(0));
        assert_eq!(ok(Some("10000")), Ok(10_000));
        assert_eq!(ok(Some(" 12 ")), Ok(12));
        assert_eq!(ok(Some("1_0")), Ok(10)); // Python int("1_0")
        assert_eq!(ok(Some("10001")), Err(()));
        assert_eq!(ok(Some("-1")), Err(()));
        assert_eq!(ok(Some("abc")), Err(()));
        assert_eq!(ok(Some("12abc")), Err(()));
        assert_eq!(ok(Some("1__0")), Err(()));
        assert_eq!(ok(Some("99999999999999999999999")), Err(()));
    }

    #[test]
    fn govde_10001_bayt_413() {
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Sahte("A")));
        let govde = "x".repeat(10_001);
        let i = Istek::yeni("POST", "/x").baslik("Host", "api.wificorrect.com").baslik("Content-Length", "10001").govde(&govde);
        let y = cevapla(&m, i);
        assert_eq!(y.durum, 413);
        assert_eq!(y.govde_metin(), "İstek çok büyük");
        // tam sınırda uygulamaya gider
        let govde = "x".repeat(10_000);
        let i = Istek::yeni("POST", "/x").baslik("Host", "api.wificorrect.com").baslik("Content-Length", "10000").govde(&govde);
        assert_eq!(cevapla(&m, i).durum, 200);
        // sayı olmayan Content-Length
        let i = Istek::yeni("POST", "/x").baslik("Host", "api.wificorrect.com").baslik("Content-Length", "abc");
        assert_eq!(cevapla(&m, i).durum, 413);
    }

    struct Panikli;

    impl Uygulama for Panikli {
        fn istek(&self, _: &Istek) -> Yanit {
            panic!("deneme paniği");
        }
    }

    #[test]
    fn panik_500_ve_sonraki_istek_calisir() {
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Panikli));
        m.ekle("ap", Arc::new(Sahte("P")));
        let y = cevapla(&m, istek("GET", "api.wificorrect.com"));
        assert_eq!((y.durum, y.govde_metin().as_str()), (500, "Bir sorun oluştu"));
        assert_eq!(y.baslik("X-Frame-Options"), Some("DENY"));
        assert_eq!(cevapla(&m, istek("GET", "ap.wificorrect.com")).durum, 200);
    }

    #[test]
    fn kilit_panikten_sonra_kullanilabilir() {
        // panik anında tutulan kilit sonraki istekleri düşürmemeli (ortak::kilitle zehirlenmeye dayanıklı)
        let m = Arc::new(std::sync::Mutex::new(1));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("zehirle");
        })
        .join();
        assert!(m.is_poisoned());
        assert_eq!(*crate::ortak::kilitle(&m), 1);
    }

    fn saat() -> Saat {
        Arc::new(|| 1_000.0)
    }

    #[test]
    fn guvenlik_basliklari_ve_uygulama_basligi_oncelikli() {
        let _k = Kayit::bas("test_merkez", "test_uygulamanin_verdigi_guvenlik_basligi_varsayilanin_yerine");
        struct Cerceveli;
        impl Uygulama for Cerceveli {
            fn istek(&self, _: &Istek) -> Yanit {
                Yanit::yeni(
                    403,
                    &[
                        ("Content-Type", "text/html; charset=utf-8"),
                        ("x-frame-options", "SAMEORIGIN"),
                        ("Content-Security-Policy", "default-src 'self'; frame-ancestors 'self'"),
                        ("Set-Cookie", "a=1; Path=/"),
                        ("Set-Cookie", "b=; Path=/; Max-Age=0"),
                    ],
                    b"x".to_vec(),
                )
            }
        }
        let mut m = Merkez::new();
        m.ekle("ap", Arc::new(Cerceveli));
        let y = cevapla(&m, istek("GET", "ap.wificorrect.com"));
        assert_eq!(y.basliklar_hepsi("X-Frame-Options"), vec!["SAMEORIGIN"]);
        assert_eq!(y.basliklar_hepsi("Content-Security-Policy"), vec!["default-src 'self'; frame-ancestors 'self'"]);
        assert_eq!(y.basliklar_hepsi("Referrer-Policy"), vec!["no-referrer"]);
        assert_eq!(y.basliklar_hepsi("X-Content-Type-Options"), vec!["nosniff"]);
        assert_eq!(y.basliklar_hepsi("Set-Cookie"), vec!["a=1; Path=/", "b=; Path=/; Max-Age=0"]);
        // normal yanıtta dört varsayılan başlık, uygulamanın başlıklarından sonra
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Sahte("A")));
        let y = cevapla(&m, istek("GET", "api.wificorrect.com"));
        let adlar: Vec<&str> = y.basliklar.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(adlar, ["X-Content-Type-Options", "Referrer-Policy", "X-Frame-Options", "Content-Security-Policy"]);
    }

    // --- iki panel, çerezler ayrı (Python test_musteri_cerezi_yonetimde_gecmez; gerçek Yonetim/Musteri sonraki görevlerde) ---

    struct Deneme {
        t: Taban,
        rol: &'static str,
        cerez: &'static str,
    }

    impl Panel for Deneme {
        fn taban(&self) -> &Taban {
            &self.t
        }
        fn rol(&self) -> &'static str {
            self.rol
        }
        fn cerez(&self) -> &'static str {
            self.cerez
        }
        fn dogrula(&self, kul: &str, pw: &str) -> Sonuc<Option<String>> {
            Ok((kul == "100001" && pw == "dogru-parola").then(|| kul.to_string()))
        }
        fn hesap_surumu(&self, kimlik: &str) -> Sonuc<Option<String>> {
            Ok((kimlik == "100001").then(|| "ozet-1".to_string()))
        }
        fn girdi(&self, _: &str, _: &str) -> Sonuc<()> {
            Ok(())
        }
        fn kim(&self, ot: &Ot) -> String {
            ot.user.clone()
        }
        fn hareket_musterisi(&self, _: &Ot, _: &str) -> Option<i64> {
            None
        }
        fn sayfalar(&self, ot: &Ot, _: &str, _: &str, yol: &str, _: &str, _: &HashMap<String, String>, _: &str) -> Sonuc<Option<Yanit>> {
            Ok((yol == "/").then(|| web::yanit_html(&self.sayfa("Ana", "merhaba", Some(ot), false, None), 200, None)))
        }
    }

    fn deneme(rol: &'static str, cerez: &'static str) -> Arc<Deneme> {
        let veri = Arc::new(crate::veri::Veri::ac(std::path::Path::new(":memory:"), saat()).unwrap());
        Arc::new(Deneme { t: Taban::new(veri, crate::kayitlar::Kayitlar::default(), saat()), rol, cerez })
    }

    #[test]
    fn musteri_cerezi_yonetimde_gecmez() {
        let mut m = Merkez::new();
        m.ekle("yonetim", deneme("yonetici", "wc_yonetim"));
        m.ekle("musteri", deneme("musteri", "wc_musteri"));
        let r = m.istek(
            &Istek::yeni("POST", "/giris")
                .baslik("Host", "panel.wificorrect.com")
                .govde("kullanici=100001&sifre=dogru-parola")
                .ip("1.1.1.1"),
        );
        assert_eq!(r.durum, 303);
        let sc = r.baslik("Set-Cookie").unwrap().to_string();
        assert!(sc.starts_with("wc_musteri=") && sc.ends_with("; HttpOnly; Secure; SameSite=Strict; Path=/"), "{sc}");
        let cerez = sc.split(';').next().unwrap().to_string();
        let panel = m.istek(&Istek::yeni("GET", "/").baslik("Host", "panel.wificorrect.com").baslik("Cookie", &cerez));
        assert_eq!(panel.durum, 200);
        let yonetim = m.istek(&Istek::yeni("GET", "/").baslik("Host", "yonetim.wificorrect.com").baslik("Cookie", &cerez));
        assert_eq!(yonetim.durum, 303);
        assert_eq!(yonetim.baslik("Location"), Some("/giris"));
    }
}
