//! `ortak::Curl` yönlendirme izleme testleri (Task 12a, ruling madde 5): yalnızca kimliksiz GET indirmeleri (sürüm ve liste
//! dosyaları) yönlendirme izler; sınırlı, yalnızca http/https; SMS istekleri (gövdeli/kimlikli) izlemez.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::Duration;

use super::*;

fn istek(url: &str, yonlendirme: u32) -> DisIstek {
    DisIstek {
        yontem: "GET".into(),
        url: url.into(),
        basliklar: vec![("User-Agent".into(), "wificorrect-merkez".into())],
        govde: None,
        kullanici: None,
        zaman_asimi: Duration::from_secs(10),
        okuma_siniri: 1 << 20,
        yonlendirme,
        indirme_sureleri: None,
    }
}

#[test]
fn yapilandirma_yonlendirmeyi_yalniz_istenince_ve_sinirli_acar() {
    let y = Curl::yapilandirma(&istek("https://x/", 0)).unwrap();
    assert!(!y.contains("location") && !y.contains("max-redirs") && !y.contains("proto-redir"));
    let y = Curl::yapilandirma(&istek("https://x/", 5)).unwrap();
    assert!(y.contains("location\n") && y.contains("max-redirs = 5\n") && y.contains("proto-redir = \"=http,https\"\n"));
    assert!(!y.contains("location-trusted"));
    // gövdeli ya da kimlikli istek (SMS) yönlendirmeye hiç girmez
    let mut i = istek("https://x/", 5);
    i.yontem = "POST".into();
    assert!(Curl::yapilandirma(&i).is_err());
    let mut i = istek("https://x/", 5);
    i.govde = Some(b"x".to_vec());
    assert!(Curl::yapilandirma(&i).is_err());
    let mut i = istek("https://x/", 5);
    i.kullanici = Some(("k".into(), "p".into()));
    assert!(Curl::yapilandirma(&i).is_err());
}

#[test]
fn yapilandirma_indirme_sureleri_python_bosta_anlamiyla() {
    // SMS: connect-timeout = zaman_asimi, hız sınırı yok
    let y = Curl::yapilandirma(&istek("https://x/", 0)).unwrap();
    assert!(y.contains("connect-timeout = 10\n") && !y.contains("speed-"));
    // indirme: bağlanma 60, boşta 60 sn'de 1 bayt/sn altı kesilir; toplam sınır yalnızca --max-time (zaman_asimi)
    let mut i = istek("https://x/", 5);
    i.zaman_asimi = Duration::from_secs(1800);
    i.indirme_sureleri = Some((Duration::from_secs(60), Duration::from_secs(60)));
    let y = Curl::yapilandirma(&i).unwrap();
    assert!(y.contains("connect-timeout = 60\n") && y.contains("speed-limit = 1\n") && y.contains("speed-time = 60\n"), "{y}");
    assert!(!y.contains("connect-timeout = 1800"));
}

/// Bir bağlantıyı kabul eder, isteğin yolunu okur, `cevap(yol)` ile cevaplar.
fn sun(l: &TcpListener, cevap: &dyn Fn(&str) -> String) -> String {
    let (mut s, _) = l.accept().unwrap();
    let mut ham = Vec::new();
    let mut tmp = [0u8; 2048];
    while !ham.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut tmp).unwrap();
        assert!(n > 0, "istek eksik");
        ham.extend_from_slice(&tmp[..n]);
    }
    let ilk = String::from_utf8_lossy(&ham).lines().next().unwrap().to_string();
    let yol = ilk.split(' ').nth(1).unwrap().to_string();
    s.write_all(cevap(&yol).as_bytes()).unwrap();
    yol
}

fn curl_var() -> bool {
    Command::new("curl").arg("--version").output().is_ok()
}

#[test]
fn yonlendirme_izlenir_sinirlanir_ve_http_disina_gitmez() {
    if !curl_var() {
        return;
    }
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let taban = format!("http://127.0.0.1:{port}");
    let sunucu = std::thread::spawn({
        let taban = taban.clone();
        move || {
            let yonlendir = |hedef: String| format!("HTTP/1.0 302 Found\r\nLocation: {hedef}\r\nContent-Length: 0\r\n\r\n");
            let t = taban.clone();
            let mut yollar = Vec::new();
            // 1) /a → /b → /son (200)
            yollar.push(sun(&l, &|y| if y == "/a" { yonlendir(format!("{t}/b")) } else { String::new() }));
            yollar.push(sun(&l, &|_| yonlendir(format!("{t}/son"))));
            yollar.push(sun(&l, &|_| "HTTP/1.0 200 OK\r\nContent-Length: 4\r\n\r\nson!".to_string()));
            // 2) yönlendirme kapalı: 302 olduğu gibi döner (tek bağlantı)
            yollar.push(sun(&l, &|_| yonlendir(format!("{t}/b"))));
            // 3) file:// hedefi reddedilir (tek bağlantı, ikincisi gelmez)
            yollar.push(sun(&l, &|_| yonlendir("file:///etc/passwd".to_string())));
            // 4) sonsuz döngü: max-redirs 2 → ilk istek + 2 yönlendirme = 3 bağlantı
            for _ in 0..3 {
                yollar.push(sun(&l, &|_| yonlendir(format!("{t}/dongu"))));
            }
            yollar
        }
    });
    let (kod, g) = Curl.calistir(&istek(&format!("{taban}/a"), 5)).unwrap();
    assert_eq!((kod, g.as_slice()), (200, &b"son!"[..]));
    // yönlendirme kapalı: 302 cevabı olduğu gibi
    let (kod, _) = Curl.calistir(&istek(&format!("{taban}/a"), 0)).unwrap();
    assert_eq!(kod, 302);
    // file:// hedefi: hata, yerel dosya okunmaz
    let h = Curl.calistir(&istek(&format!("{taban}/f"), 5));
    assert!(h.is_err(), "{h:?}");
    // döngü: sınır aşılınca hata
    let h = Curl.calistir(&istek(&format!("{taban}/dongu"), 2));
    assert!(h.is_err(), "{h:?}");
    let yollar = sunucu.join().unwrap();
    assert_eq!(yollar, ["/a", "/b", "/son", "/a", "/f", "/dongu", "/dongu", "/dongu"]);
}

#[test]
fn yapilandirma_sms_asama_sureleri_ve_globoff() {
    // SMS biçimi: bağlanma 7, boşta 5 (hız sınırı), toplam 12 (--max-time `calistir`de, zaman_asimi'ndan). Anlam: aşama
    // süreleri verilince `zaman_asimi` TOPLAM sınırdır, bağlanma süresi olarak kullanılmaz.
    let mut i = istek("https://x/", 0);
    i.yontem = "POST".into();
    i.govde = Some(b"x".to_vec());
    i.zaman_asimi = Duration::from_secs(12);
    i.indirme_sureleri = Some((Duration::from_secs(7), Duration::from_secs(5)));
    let y = Curl::yapilandirma(&i).unwrap();
    assert!(y.contains("connect-timeout = 7\n"), "{y}");
    assert!(y.contains("speed-limit = 1\n") && y.contains("speed-time = 5\n"), "{y}");
    assert!(!y.contains("connect-timeout = 12") && !y.contains("max-time"), "{y}");
    assert!(y.starts_with("globoff\n"), "{y}");
}

/// Sınırı aşan indirme erken biter: sunucu sınırdan fazlasını gönderip bağlantıyı açık tutar; çocuk süreç hemen öldürülür.
#[test]
fn indirme_siniri_asilinca_curl_hemen_oldurulur() {
    if !curl_var() {
        return;
    }
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let sunucu = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut tmp = [0u8; 2048];
        let mut ham = Vec::new();
        while !ham.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = s.read(&mut tmp).unwrap();
            assert!(n > 0);
            ham.extend_from_slice(&tmp[..n]);
        }
        let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\n");
        let _ = s.write_all(&vec![b'a'; 64 * 1024]);
        // bağlantı açık kalır: curl kendiliğinden bitmez, ancak öldürülürse istemci kapanır
        std::thread::sleep(Duration::from_secs(8));
    });
    let mut i = istek(&format!("http://127.0.0.1:{port}/b"), 5);
    i.okuma_siniri = 1001;
    i.zaman_asimi = Duration::from_secs(30);
    i.indirme_sureleri = Some((Duration::from_secs(5), Duration::from_secs(20)));
    let basla = std::time::Instant::now();
    let (_, g) = Curl.calistir(&i).unwrap();
    assert_eq!(g.len(), 1001, "sınır+1 bayt okunmuş olmalı");
    assert!(basla.elapsed() < Duration::from_secs(4), "sınır aşımı beklemeden bitmeli: {:?}", basla.elapsed());
    drop(sunucu); // uyuyan iş parçacığı beklenmez
}
