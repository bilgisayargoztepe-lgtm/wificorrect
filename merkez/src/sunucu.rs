//! HTTP sunucusu (Python `ThreadingHTTPServer` + `Isleyici`): bağlantı başına bir iş parçacığı, soket başına zaman aşımı,
//! HTTP/1.0 biçiminde tek istek/tek cevap (Python `BaseHTTPRequestHandler` varsayılanı; Caddy her istekte yeni bağlantı açar).
//!
//! tiny_http kullanılmadı: zaman aşımı yalnızca dinleyici sokete konabiliyor (kabul edilen bağlantılara geçiyor), ama
//! `accept()` de aynı SO_RCVTIMEO ile zaman aşımına uğruyor ve tiny_http kabul döngüsü ilk hatada kapanıyor (cihazda
//! görüldü: boşta kalan sunucu `Connection refused` verdi). Bu yüzden kabul edilen her sokete zaman aşımı elle konur.
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(test)]
use std::net::SocketAddr;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::merkez::{self, Merkez};
use crate::ortak::{py_repr, py_strip};
use crate::web::{self, Basliklar, Istek, Yanit};

/// Python `Isleyici.timeout`: yavaş istemci iş parçacığını süresiz tutmasın (her okuma için).
pub const SOKET_ZAMAN_ASIMI: Duration = Duration::from_secs(20);
/// İsteği (başlıklar + gövde) okumaya TOPLAM son tarih. Okuma başına 20 sn tek başına yetmez: her gelen bayt süreyi yeniden
/// başlatır, 19 sn'de bir bayt gönderen istemci bir bağlantı yuvasını günlerce tutabilir (256 yuva dolunca yeni misafir
/// girişi durur). Meşru istekler küçüktür (gövde en çok 10.000 bayt, Caddy'den tek seferde gelir). Cevap yazma bu son
/// tarihin dışındadır (`/api/paket` 50 MB ilerleme bazlı gider).
pub const ISTEK_TOPLAM_SURE: Duration = Duration::from_secs(30);
/// Bağlantı sınırına gelindiğinde günlük satırı en çok bu aralıkla yazılır.
const SINIR_GUNLUK_ARALIGI: Duration = Duration::from_secs(60);
/// Python `http.server` satır sınırı (`readline(65537)`) ve en çok başlık sayısı.
const SATIR_SINIRI: usize = 65_536;
const BASLIK_SINIRI: usize = 100;
/// 413 sonrası istemcinin gönderdiği kalan gövdeyi atmak için (bağlantı sıfırlanıp cevap kaybolmasın).
const ATMA_SINIRI: u64 = 65_536;

/// Python `urlsplit(self.path)` (+ `http.server`'ın `//` düzeltmesi): (yol, sorgu); `#` sonrası atılır, mutlak biçimde
/// (`http://ev/yol`) şema ve ağ konumu atılır.
pub fn url_ayir(url: &str) -> (String, String) {
    let mut url = url.to_string();
    if url.starts_with("//") {
        url = format!("/{}", url.trim_start_matches('/'));
    }
    let url = url.split('#').next().unwrap_or("");
    let (yol, sorgu) = url.split_once('?').unwrap_or((url, ""));
    let mut yol = yol;
    if let Some((sema, geri)) = yol.split_once("://") {
        let gecerli = !sema.is_empty()
            && sema.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && sema.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        if gecerli {
            yol = geri.find('/').map(|n| &geri[n..]).unwrap_or("");
        }
    }
    (yol.to_string(), sorgu.to_string())
}

fn neden(durum: u16) -> &'static str {
    match durum {
        200 => "OK",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Request Entity Too Large",
        414 => "Request-URI Too Long",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

fn aciklama(durum: u16) -> &'static str {
    match durum {
        400 => "Bad request syntax or unsupported method",
        414 => "URI is too long",
        431 => "The server is unwilling to process the request because its header fields are too large",
        501 => "Server does not support this operation",
        505 => "Cannot fulfill request.",
        _ => "",
    }
}

/// `html.escape(s, quote=False)`
fn kacis(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Python `BaseHTTPRequestHandler.send_error` cevabı (istek ayrıştırılamadı ya da yöntem desteklenmiyor).
fn hata_yaniti(durum: u16, mesaj: &str) -> Yanit {
    let govde = format!(
        "<!DOCTYPE HTML>\n<html lang=\"en\">\n    <head>\n        <meta charset=\"utf-8\">\n        <title>Error response</title>\n    </head>\n    \
         <body>\n        <h1>Error response</h1>\n        <p>Error code: {durum}</p>\n        <p>Message: {}.</p>\n        \
         <p>Error code explanation: {durum} - {}.</p>\n    </body>\n</html>\n",
        kacis(mesaj),
        kacis(aciklama(durum))
    );
    Yanit::yeni(durum, &[("Connection", "close"), ("Content-Type", "text/html;charset=utf-8")], govde.into_bytes())
}

/// `\n` ile biten bir satır (en çok `SATIR_SINIRI` bayt; aşarsa `Ok(Err(()))`); bağlantı kapanmışsa `None`.
fn satir_oku<R: BufRead>(r: &mut R) -> std::io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut tampon = Vec::new();
    let n = r.by_ref().take(SATIR_SINIRI as u64 + 1).read_until(b'\n', &mut tampon)?;
    if n == 0 {
        return Ok(None);
    }
    if tampon.len() > SATIR_SINIRI {
        return Ok(Some(Err(())));
    }
    Ok(Some(Ok(tampon)))
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

/// Ayrıştırılmış istek ya da hemen gönderilecek hata cevabı; `None`: cevapsız kapat (boş istek).
enum Okunan {
    Istek(Istek),
    Hata(Yanit),
    Kapat,
}

/// Okuma başına zaman aşımını kalan toplam süreyle sınırlayan okuyucu: her okumadan önce `set_read_timeout(min(okuma_basi,
/// kalan))`; son tarih dolunca `TimedOut` (bağlantı Python'daki okuma zaman aşımı gibi cevapsız kapanır).
struct SonTarihli<'a> {
    akim: &'a TcpStream,
    son: Instant,
    okuma_basi: Duration,
}

impl Read for SonTarihli<'_> {
    fn read(&mut self, tampon: &mut [u8]) -> std::io::Result<usize> {
        let kalan = self.son.saturating_duration_since(Instant::now());
        if kalan.is_zero() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "istek okuma son tarihi doldu"));
        }
        self.akim.set_read_timeout(Some(kalan.min(self.okuma_basi)))?;
        let mut akim = self.akim;
        akim.read(tampon)
    }
}

fn istek_oku<R: BufRead>(akim: &mut R) -> std::io::Result<Okunan> {
    let satir = match satir_oku(akim)? {
        None => return Ok(Okunan::Kapat),
        Some(Err(())) => return Ok(Okunan::Hata(hata_yaniti(414, "Request-URI Too Long"))),
        Some(Ok(s)) => s,
    };
    let metin = latin1(&satir);
    let metin = metin.trim_end_matches(['\r', '\n']);
    let kelimeler: Vec<&str> = metin.split_whitespace().collect();
    if kelimeler.is_empty() {
        return Ok(Okunan::Kapat);
    }
    if kelimeler.len() != 3 {
        return Ok(Okunan::Hata(hata_yaniti(400, &format!("Bad request syntax ({})", py_repr(metin)))));
    }
    let surum = kelimeler[2];
    let sayilar: Option<Vec<u32>> = surum
        .strip_prefix("HTTP/")
        .map(|s| s.split('.').map(|p| if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) { None } else { p.parse().ok() }).collect())
        .unwrap_or(None);
    match sayilar.as_deref() {
        Some([ana, _]) if *ana >= 2 => {
            return Ok(Okunan::Hata(hata_yaniti(505, &format!("Invalid HTTP version ({})", &surum[5..]))));
        }
        Some([_, _]) => {}
        _ => return Ok(Okunan::Hata(hata_yaniti(400, &format!("Bad request version ({})", py_repr(surum))))),
    }
    // başlıklar
    // Python http.client._read_headers: her ham satır (boş bitiş satırı dahil) sayılır, 100'ü aşarsa 431 (en çok 99 başlık).
    // Ayrıştırma iki nokta içermeyen ilk satırda durur (sonrakiler yok sayılır ama okunup sayılmaya devam eder).
    let mut basliklar = Basliklar::default();
    let (mut satir_sayisi, mut ayristirma_bitti) = (0usize, false);
    loop {
        let s = match satir_oku(akim)? {
            None => break,
            Some(Err(())) => return Ok(Okunan::Hata(hata_yaniti(431, "Line too long"))),
            Some(Ok(s)) => s,
        };
        satir_sayisi += 1;
        if satir_sayisi > BASLIK_SINIRI {
            return Ok(Okunan::Hata(hata_yaniti(431, "Too many headers")));
        }
        let s = latin1(&s);
        let s = s.trim_end_matches(['\r', '\n']);
        if s.is_empty() {
            break;
        }
        if ayristirma_bitti {
            continue;
        }
        if s.starts_with([' ', '\t']) {
            // eski katlanmış başlık satırı: öncekinin devamı
            if let Some(son) = basliklar.0.last_mut() {
                son.1.push(' ');
                son.1.push_str(py_strip(s));
            }
            continue;
        }
        match s.split_once(':') {
            Some((ad, deger)) => basliklar.0.push((ad.to_string(), py_strip(deger).to_string())),
            None => ayristirma_bitti = true,
        }
    }
    let yontem = kelimeler[0];
    if yontem != "GET" && yontem != "POST" {
        return Ok(Okunan::Hata(hata_yaniti(501, &format!("Unsupported method ({})", py_repr(yontem)))));
    }
    let (yol, sorgu) = url_ayir(kelimeler[1]);
    let ip = merkez::istemci_ip(&basliklar);
    // Content-Length yoksa gövde boş; sınırı aşan/bozuk uzunlukta gövde okunmaz (cevapla 413 verir)
    let uzunluk = merkez::govde_uzunlugu(&basliklar).unwrap_or(0);
    let mut ham = Vec::with_capacity(uzunluk);
    if uzunluk > 0 {
        akim.by_ref().take(uzunluk as u64).read_to_end(&mut ham)?;
    }
    Ok(Okunan::Istek(Istek { yontem: yontem.to_string(), yol, basliklar, govde: String::from_utf8_lossy(&ham).into_owned(), sorgu, ip }))
}

/// Başlık adı/değeri `latin-1`'e sığmıyor ya da satır sonu içeriyorsa yazılamaz (Python `send_header` de reddeder).
fn baslik_yaz(cikti: &mut Vec<u8>, k: &str, v: &str) -> bool {
    let uygun = |s: &str| s.chars().all(|c| (c as u32) < 0x100 && c != '\r' && c != '\n');
    if k.is_empty() || k.contains(':') || !uygun(k) || !uygun(v) {
        web::log(&format!("HATA başlık yazılamadı: {k}"));
        return false;
    }
    cikti.extend(k.chars().map(|c| c as u8));
    cikti.extend_from_slice(b": ");
    cikti.extend(v.chars().map(|c| c as u8));
    cikti.extend_from_slice(b"\r\n");
    true
}

fn tarih() -> String {
    let sn = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    chrono::DateTime::from_timestamp(sn, 0).map(|t| t.format("%a, %d %b %Y %H:%M:%S GMT").to_string()).unwrap_or_default()
}

/// Bundan büyük gövde başlıkla tek tamponda birleştirilmez (`/api/paket` 50 MB'a kadar .deb verir): başlık bloğu ve gövde
/// ayrı yazılır, gövde bir kez daha kopyalanmaz.
const TEK_TAMPON_SINIRI: usize = 64 * 1024;

/// Python `send_response` + `send_header` + `end_headers` + gövde. Hata cevapları (`hata_yaniti`) güvenlik başlıksız gider.
fn yanit_yaz(mut akim: &TcpStream, y: &Yanit) -> std::io::Result<()> {
    let tek = y.govde.len() <= TEK_TAMPON_SINIRI;
    let mut b = Vec::with_capacity(512 + if tek { y.govde.len() } else { 0 });
    b.extend_from_slice(format!("HTTP/1.0 {} {}\r\nServer: wificorrect\r\nDate: {}\r\n", y.durum, neden(y.durum), tarih()).as_bytes());
    for (k, v) in &y.basliklar {
        baslik_yaz(&mut b, k, v);
    }
    baslik_yaz(&mut b, "Content-Length", &y.govde.len().to_string());
    b.extend_from_slice(b"\r\n");
    if tek {
        b.extend_from_slice(&y.govde);
        akim.write_all(&b)?;
    } else {
        // iki yazma arasında Nagle gecikmesi olmasın
        let _ = akim.set_nodelay(true);
        akim.write_all(&b)?;
        akim.write_all(&y.govde)?;
    }
    akim.flush()
}

fn baglanti_isle(m: &Merkez, akim: TcpStream, okuma_basi: Duration, toplam: Duration) {
    let son = Instant::now() + toplam;
    let okunan = match istek_oku(&mut BufReader::new(SonTarihli { akim: &akim, son, okuma_basi })) {
        Ok(o) => o,
        Err(_) => return, // zaman aşımı/kopma: Python'da istisna, bağlantı cevapsız kapanır
    };
    let (y, atilacak) = match okunan {
        Okunan::Kapat => return,
        Okunan::Hata(y) => (y, false),
        Okunan::Istek(i) => {
            let govde_buyuk = merkez::govde_uzunlugu(&i.basliklar).is_err();
            (merkez::cevapla(m, i), govde_buyuk)
        }
    };
    if yanit_yaz(&akim, &y).is_err() {
        return;
    }
    let _ = akim.shutdown(Shutdown::Write);
    if atilacak {
        // okunmamış gövde varken kapatmak bağlantıyı sıfırlayıp cevabı silebilir: kalanı kısa sürede at
        let _ = akim.set_read_timeout(Some(Duration::from_secs(2)));
        let a = &akim;
        let _ = std::io::copy(&mut a.take(ATMA_SINIRI), &mut std::io::sink());
    }
}

/// Kullanımdaki bağlantı yuvası: iş parçacığı nasıl biterse bitsin (panik dahil) Drop ile geri verilir.
struct Yuva(Arc<AtomicUsize>);

impl Yuva {
    fn al(aktif: &Arc<AtomicUsize>) -> Yuva {
        aktif.fetch_add(1, Ordering::SeqCst);
        Yuva(aktif.clone())
    }
}

impl Drop for Yuva {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Bağlantı sınırına gelindiğinde kapatılan bağlantıları sayar; dakikada en çok bir satır üretir. Satır `ip=` ile bitmez:
/// fail2ban filtresine girmez.
struct SinirGunlugu {
    son: Option<Instant>,
    kapatilan: u64,
}

impl SinirGunlugu {
    fn yeni() -> SinirGunlugu {
        SinirGunlugu { son: None, kapatilan: 0 }
    }

    /// Bir bağlantı kapatıldı; günlük satırı yazma zamanıysa satırı verir (`kapatilan`: son satırdan beri sayı).
    fn kaydet(&mut self, simdi: Instant, sinir: usize) -> Option<String> {
        self.kapatilan += 1;
        if self.son.is_some_and(|s| simdi.saturating_duration_since(s) < SINIR_GUNLUK_ARALIGI) {
            return None;
        }
        self.son = Some(simdi);
        let n = std::mem::take(&mut self.kapatilan);
        Some(format!("BAGLANTI_SINIRI sinir={sinir} kapatilan={n}"))
    }
}

/// Çalışan sunucu (testler için durdurulabilir).
pub struct Calisan {
    #[cfg(test)]
    adres: SocketAddr,
    #[cfg(test)]
    dur: Arc<AtomicBool>,
    kabul: JoinHandle<()>,
}

impl Calisan {
    #[cfg(test)]
    pub fn adres(&self) -> SocketAddr {
        self.adres
    }

    /// Kabul döngüsü bitene kadar bekler (üretimde sonsuza dek).
    pub fn bekle(self) {
        let _ = self.kabul.join();
    }

    #[cfg(test)]
    /// Yeni bağlantı kabul etmeyi bırakır (süren bağlantılar kendi sürelerinde biter).
    pub fn durdur(self) {
        self.dur.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.adres); // kabul döngüsünü uyandır
        self.bekle();
    }
}

/// `adres`'te dinler; kabul edilen her sokete `zaman_asimi` (okuma ve yazma başına) konur, isteğin okunması ise `toplam`
/// içinde bitmelidir. `en_cok`: eşzamanlı bağlantı sınırı (Python sınırsız iş parçacığı açardı; dolunca yeni bağlantı
/// hemen kapatılır ve dakikada bir `BAGLANTI_SINIRI` satırı yazılır).
pub fn baslat(merkez: Arc<Merkez>, adres: &str, en_cok: usize, zaman_asimi: Duration, toplam: Duration) -> Result<Calisan, String> {
    let dinleyici = TcpListener::bind(adres).map_err(|e| format!("{adres} dinlenemiyor: {e}"))?;
    #[cfg(test)]
    let yerel = dinleyici.local_addr().map_err(|e| e.to_string())?;
    let dur = Arc::new(AtomicBool::new(false));
    let aktif = Arc::new(AtomicUsize::new(0));
    let (dur2, en_cok) = (dur.clone(), en_cok.max(1));
    let kabul = std::thread::Builder::new()
        .name("kabul".into())
        .spawn(move || {
            let mut sinir_gunlugu = SinirGunlugu::yeni();
            for akim in dinleyici.incoming() {
                if dur2.load(Ordering::SeqCst) {
                    break;
                }
                let akim = match akim {
                    Ok(a) => a,
                    Err(e) => {
                        // geçici hata (ör. EMFILE) kabul döngüsünü öldürmesin
                        web::log(&format!("HATA kabul: {e}"));
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                };
                if aktif.load(Ordering::SeqCst) >= en_cok {
                    if let Some(satir) = sinir_gunlugu.kaydet(Instant::now(), en_cok) {
                        web::log(&satir);
                    }
                    continue; // akim düşer, bağlantı kapanır
                }
                if akim.set_read_timeout(Some(zaman_asimi)).is_err() || akim.set_write_timeout(Some(zaman_asimi)).is_err() {
                    continue;
                }
                // yuva Drop ile geri verilir: iş parçacığı panikle bitse ya da hiç başlamasa (closure düşer) de sızmaz
                let (m, yuva) = (merkez.clone(), Yuva::al(&aktif));
                let _ = std::thread::Builder::new().spawn(move || {
                    let _yuva = yuva;
                    baglanti_isle(&m, akim, zaman_asimi, toplam);
                });
            }
        })
        .map_err(|e| format!("kabul iş parçacığı başlatılamadı: {e}"))?;
    Ok(Calisan {
        #[cfg(test)]
        adres: yerel,
        #[cfg(test)]
        dur,
        kabul,
    })
}

/// Üretim: umask 077 (Python `main`), okuma başına 20 sn, istek başına toplam 30 sn; sonsuza dek çalışır. `en_cok`:
/// eşzamanlı bağlantı sınırı. Kabul iş parçacığı biterse (panik dahil) `Err` döner: süreç sıfırdan farklı çıkar ve
/// `Restart=on-failure` yeniden başlatır (sessizce 0 ile çıkıp hizmeti durdurmaz).
pub fn calistir(merkez: Arc<Merkez>, adres: &str, en_cok: usize) -> Result<(), String> {
    // SAFETY: umask çağrısı yan etkisiz bir sistem çağrısıdır
    unsafe {
        libc::umask(0o077);
    }
    baslat(merkez, adres, en_cok, SOKET_ZAMAN_ASIMI, ISTEK_TOPLAM_SURE)?.bekle();
    Err("kabul iş parçacığı beklenmedik biçimde bitti".into())
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::merkez::Uygulama;
    use std::time::Instant;

    struct Yansit;

    impl Uygulama for Yansit {
        fn istek(&self, i: &Istek) -> Yanit {
            if i.yol == "/buyuk" {
                return Yanit::yeni(200, &[("Content-Type", "application/octet-stream")], vec![7u8; 300_000]);
            }
            if i.yol == "/coklu" {
                return Yanit::yeni(303, &[("Location", "/"), ("Set-Cookie", "a=1; Path=/"), ("Set-Cookie", "b=; Path=/; Max-Age=0")], vec![]);
            }
            Yanit::yeni(200, &[], format!("{} {} q={} ip={} g={}", i.yontem, i.yol, i.sorgu, i.ip, i.govde).into_bytes())
        }
    }

    fn baslat_deneme(zaman_asimi: Duration) -> Calisan {
        baslat_toplamli(zaman_asimi, ISTEK_TOPLAM_SURE)
    }

    fn baslat_toplamli(zaman_asimi: Duration, toplam: Duration) -> Calisan {
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Yansit));
        baslat(Arc::new(m), "127.0.0.1:0", 64, zaman_asimi, toplam).unwrap()
    }

    /// Ham istek gönderir, bağlantı kapanana kadar okur.
    fn al(adres: SocketAddr, istek: &str) -> Vec<u8> {
        let mut s = TcpStream::connect(adres).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(istek.as_bytes()).unwrap();
        let mut o = Vec::new();
        let _ = s.read_to_end(&mut o);
        o
    }

    fn metin(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    #[test]
    fn url_ayirma_python_urlsplit_gibi() {
        assert_eq!(url_ayir("/a/b?x=1&y=2"), ("/a/b".into(), "x=1&y=2".into()));
        assert_eq!(url_ayir("/a#parca"), ("/a".into(), "".into()));
        assert_eq!(url_ayir("/a?x=1#p?q"), ("/a".into(), "x=1".into()));
        assert_eq!(url_ayir("/"), ("/".into(), "".into()));
        assert_eq!(url_ayir("//ev.il/yol?z=1"), ("/ev.il/yol".into(), "z=1".into())); // http.server: '/' + lstrip('/')
        assert_eq!(url_ayir("///x"), ("/x".into(), "".into()));
        assert_eq!(url_ayir("http://ev.il/yol?z=1"), ("/yol".into(), "z=1".into()));
        assert_eq!(url_ayir("/yol?"), ("/yol".into(), "".into()));
        assert_eq!(url_ayir("/a?u=http://x/y"), ("/a".into(), "u=http://x/y".into()));
    }

    #[test]
    fn istek_cevap_basliklar_ve_ip() {
        let c = baslat_deneme(Duration::from_secs(5));
        let ham = al(
            c.adres(),
            "POST /x/y?k=v HTTP/1.1\r\nHost: api.wificorrect.com\r\nX-Forwarded-For: 6.6.6.6, 203.0.113.5\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
        );
        let t = metin(&ham);
        assert!(t.starts_with("HTTP/1.0 200 OK\r\nServer: wificorrect\r\nDate: "), "{t}");
        assert!(t.contains("\r\nX-Frame-Options: DENY\r\n") && t.contains("\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:\r\n"), "{t}");
        assert!(t.contains("\r\nContent-Length: 36\r\n\r\nPOST /x/y q=k=v ip=203.0.113.5 g=abc"), "{t}");
        // Content-Length yok: gövde boş; başlık adları büyük/küçük harf duyarsız
        let t = metin(&al(c.adres(), "POST /z HTTP/1.1\r\nhost: api.wificorrect.com\r\nx-forwarded-for:  9.9.9.9 \r\n\r\n"));
        assert!(t.ends_with("POST /z q= ip=9.9.9.9 g="), "{t}");
        c.durdur();
    }

    #[test]
    fn liste_degerli_baslik_ayri_satirlar() {
        let _k = crate::altin::Kayit::bas("test_merkez", "test_liste_degerli_baslik_ayri_satirlar");
        let c = baslat_deneme(Duration::from_secs(5));
        let t = metin(&al(c.adres(), "GET /coklu HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 303 See Other\r\n"), "{t}");
        assert!(t.contains("\r\nSet-Cookie: a=1; Path=/\r\nSet-Cookie: b=; Path=/; Max-Age=0\r\n"), "{t}");
        assert!(t.contains("\r\nX-Frame-Options: DENY\r\n") && t.contains("\r\nContent-Length: 0\r\n\r\n"));
        c.durdur();
    }

    #[test]
    fn buyuk_govde_tamam_yazilir() {
        let _k = crate::altin::Kayit::bas("test_merkez", "test_buyuk_govde_parca_parca_yazilir");
        let c = baslat_deneme(Duration::from_secs(5));
        let ham = al(c.adres(), "GET /buyuk HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n");
        let ayrac = ham.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(metin(&ham[..ayrac]).contains("\r\nContent-Length: 300000\r\n"));
        assert_eq!(ham.len() - ayrac, 300_000);
        assert!(ham[ayrac..].iter().all(|b| *b == 7));
        c.durdur();
    }

    #[test]
    fn govde_10001_bayt_413_sunucuda() {
        let c = baslat_deneme(Duration::from_secs(5));
        let govde = "x".repeat(10_001);
        let t = metin(&al(c.adres(), &format!("POST /x HTTP/1.1\r\nHost: api.wificorrect.com\r\nContent-Length: 10001\r\n\r\n{govde}")));
        assert!(t.starts_with("HTTP/1.0 413 "), "{}", &t[..t.len().min(200)]);
        assert!(t.ends_with("İstek çok büyük") && t.contains("\r\nX-Frame-Options: DENY\r\n"));
        // tam sınır uygulamaya gider
        let govde = "y".repeat(10_000);
        let t = metin(&al(c.adres(), &format!("POST /x HTTP/1.1\r\nHost: api.wificorrect.com\r\nContent-Length: 10000\r\n\r\n{govde}")));
        assert!(t.starts_with("HTTP/1.0 200 "));
        // sayı olmayan Content-Length
        let t = metin(&al(c.adres(), "POST /x HTTP/1.1\r\nHost: api.wificorrect.com\r\nContent-Length: abc\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 413 "), "{t}");
        c.durdur();
    }

    /// Python 3.14 `http.client._read_headers`: boş bitiş satırı dahil her satır sayılır (99 başlık geçer, 100 başlık 431);
    /// katlanmış ve iki noktasız satırlar da sayılır; iki noktasız satırdan sonrası yok sayılır.
    #[test]
    fn baslik_satiri_siniri_python_gibi() {
        let c = baslat_deneme(Duration::from_secs(5));
        let n = |k: usize| -> String { (0..k).map(|i| format!("H{i}: v\r\n")).collect() };
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\n{}\r\n", n(98))));
        assert!(t.starts_with("HTTP/1.0 200 "), "{}", &t[..t.len().min(80)]);
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\n{}\r\n", n(99))));
        assert!(t.starts_with("HTTP/1.0 431 "), "{}", &t[..t.len().min(80)]);
        let katli: String = (0..150).map(|_| " devam\r\n").collect();
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\nX: a\r\n{katli}\r\n")));
        assert!(t.starts_with("HTTP/1.0 431 "), "{}", &t[..t.len().min(80)]);
        let kolonsuz: String = (0..150).map(|_| "kolonsuz\r\n").collect();
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\nX: a\r\n{kolonsuz}\r\n")));
        assert!(t.starts_with("HTTP/1.0 431 "), "{}", &t[..t.len().min(80)]);
        // iki noktasız satırda ayrıştırma durur: sonraki Host yok sayılır -> bilinmeyen alan 404
        let t = metin(&al(c.adres(), "GET /k HTTP/1.1\r\nX: a\r\nkolonsuz\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 404 "), "{}", &t[..t.len().min(80)]);
        c.durdur();
    }

    #[test]
    fn bozuk_istekler_python_gibi_hata_sayfasi() {
        let c = baslat_deneme(Duration::from_secs(5));
        let t = metin(&al(c.adres(), "PUT / HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 501 Not Implemented\r\n") && t.contains("Message: Unsupported method ('PUT')."), "{t}");
        assert!(t.contains("Error code explanation: 501 - Server does not support this operation."));
        let t = metin(&al(c.adres(), "GARIP\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 400 ") && t.contains("Bad request syntax ('GARIP')"), "{t}");
        let t = metin(&al(c.adres(), "GET / HTTP/x.y\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 400 ") && t.contains("Bad request version ('HTTP/x.y')"), "{t}");
        let t = metin(&al(c.adres(), "GET / HTTP/2.0\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 505 "), "{t}");
        let uzun = "a".repeat(70_000);
        let t = metin(&al(c.adres(), &format!("GET /{uzun} HTTP/1.1\r\n\r\n")));
        assert!(t.starts_with("HTTP/1.0 414 "), "{}", &t[..t.len().min(100)]);
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\nX: {uzun}\r\n\r\n")));
        assert!(t.starts_with("HTTP/1.0 431 "), "{}", &t[..t.len().min(100)]);
        let cok: String = (0..101).map(|n| format!("H{n}: v\r\n")).collect();
        let t = metin(&al(c.adres(), &format!("GET / HTTP/1.1\r\n{cok}\r\n")));
        assert!(t.starts_with("HTTP/1.0 431 "), "{}", &t[..t.len().min(100)]);
        // boş bağlantı cevapsız kapanır; sunucu yaşar
        assert!(al(c.adres(), "").is_empty());
        assert!(metin(&al(c.adres(), "GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n")).starts_with("HTTP/1.0 200 "));
        c.durdur();
    }

    /// Python `test_soket_zaman_asimi` (Isleyici.timeout = 20): yarım istek satırı gönderen bağlantı kapanır, bu sırada
    /// başka istek cevaplanır. Sunucu zaman aşımından sonra da çalışmaya devam eder (kabul döngüsü zaman aşımıyla ölmemeli).
    #[test]
    fn soket_zaman_asimi() {
        let _k = crate::altin::Kayit::bas("test_merkez", "test_soket_zaman_asimi");
        assert_eq!(SOKET_ZAMAN_ASIMI, Duration::from_secs(20));
        let c = baslat_deneme(Duration::from_secs(1));
        let baslangic = Instant::now();
        let mut yavas = TcpStream::connect(c.adres()).unwrap();
        yavas.write_all(b"GET /yavas HT").unwrap(); // yarım istek satırı
        yavas.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        // bu sırada başka istek cevaplanır
        let t = metin(&al(c.adres(), "GET /hizli HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 200 ") && t.ends_with("GET /hizli q= ip=yerel g="), "{t}");
        // yavaş bağlantı sunucu tarafından kapatılır (cevapsız EOF), istemci okuma zaman aşımına düşmeden
        let mut o = Vec::new();
        match yavas.read_to_end(&mut o) {
            Ok(_) => assert!(o.is_empty(), "{}", metin(&o)),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(e) => panic!("bağlantı kapanmadı (zaman aşımı konmamış): {e}"),
        }
        let gecen = baslangic.elapsed();
        assert!(gecen >= Duration::from_millis(900) && gecen < Duration::from_secs(6), "{gecen:?}");
        // zaman aşımı süresi geçtikten sonra da yeni bağlantılar kabul edilir
        std::thread::sleep(Duration::from_millis(1500));
        let t = metin(&al(c.adres(), "GET /sonra HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 200 ") && t.ends_with("GET /sonra q= ip=yerel g="), "{t}");
        c.durdur();
    }

    #[test]
    fn govdesi_yarim_kalan_baglanti_kapanir() {
        let c = baslat_deneme(Duration::from_secs(1));
        let mut s = TcpStream::connect(c.adres()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        s.write_all(b"POST /x HTTP/1.1\r\nHost: api.wificorrect.com\r\nContent-Length: 50\r\n\r\nyarim").unwrap();
        let basla = Instant::now();
        let mut o = Vec::new();
        match s.read_to_end(&mut o) {
            Ok(_) => assert!(o.is_empty(), "{}", metin(&o)),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(e) => panic!("bağlantı kapanmadı: {e}"),
        }
        assert!(basla.elapsed() < Duration::from_secs(6));
        let t = metin(&al(c.adres(), "GET /s HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n"));
        assert!(t.starts_with("HTTP/1.0 200 "));
        c.durdur();
    }

    #[test]
    fn eszamanli_baglanti_siniri() {
        let mut m = Merkez::new();
        m.ekle("api", Arc::new(Yansit));
        let c = baslat(Arc::new(m), "127.0.0.1:0", 2, Duration::from_secs(1), ISTEK_TOPLAM_SURE).unwrap();
        // iki yavaş bağlantı sınırı doldurur: üçüncü hemen kapanır (cevapsız)
        let _a = TcpStream::connect(c.adres()).unwrap();
        let _b = TcpStream::connect(c.adres()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(al(c.adres(), "GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n").is_empty());
        // yavaşlar zaman aşımıyla düşünce yeniden cevaplanır
        std::thread::sleep(Duration::from_millis(1300));
        assert!(metin(&al(c.adres(), "GET / HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n")).starts_with("HTTP/1.0 200 "));
        c.durdur();
    }

    /// Yavaş gövde: her okuma okuma başı sınırın (burada 5 sn) altında kalsa da toplam son tarihle kapanır, cevap
    /// yazılmaz. (Eski davranış: her bayt süreyi yeniden başlatır, yuva günlerce tutulurdu.)
    #[test]
    fn yavas_govde_toplam_sureyle_kapanir() {
        let c = baslat_toplamli(Duration::from_secs(5), Duration::from_millis(1500));
        let mut s = TcpStream::connect(c.adres()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(b"POST /x HTTP/1.1\r\nHost: api.wificorrect.com\r\nContent-Length: 100\r\n\r\n").unwrap();
        let basla = Instant::now();
        let mut kapandi = false;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(400)); // her aralık okuma başı sınırın çok altında
            if s.write_all(b"a").is_err() {
                kapandi = true;
                break;
            }
        }
        let mut o = Vec::new();
        match s.read_to_end(&mut o) {
            Ok(_) => assert!(o.is_empty(), "cevap yazılmamalı: {}", metin(&o)),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(e) => panic!("bağlantı kapanmadı: {e}"),
        }
        let gecen = basla.elapsed();
        assert!(gecen >= Duration::from_millis(1400) && gecen < Duration::from_secs(6), "{gecen:?} (yazma hatası görüldü: {kapandi})");
        // sunucu yaşıyor; hızlı istek normal cevaplanır
        assert!(metin(&al(c.adres(), "GET /s HTTP/1.1\r\nHost: api.wificorrect.com\r\n\r\n")).starts_with("HTTP/1.0 200 "));
        c.durdur();
    }

    #[test]
    fn sinir_gunlugu_dakikada_bir_satir() {
        let t0 = Instant::now();
        let mut g = SinirGunlugu::yeni();
        // ilk kapatma hemen satır yazar
        assert_eq!(g.kaydet(t0, 256).as_deref(), Some("BAGLANTI_SINIRI sinir=256 kapatilan=1"));
        // dakika dolmadan satır yok, sayaç birikir
        assert_eq!(g.kaydet(t0 + Duration::from_secs(1), 256), None);
        assert_eq!(g.kaydet(t0 + Duration::from_secs(59), 256), None);
        // dakika dolunca tek satır, son satırdan beri sayı (2 + bu kapatma)
        assert_eq!(g.kaydet(t0 + Duration::from_secs(60), 256).as_deref(), Some("BAGLANTI_SINIRI sinir=256 kapatilan=3"));
        assert_eq!(g.kaydet(t0 + Duration::from_secs(61), 256), None);
        // fail2ban filtresine girmez: ip= ile bitmez
        assert!(!"BAGLANTI_SINIRI sinir=256 kapatilan=3".ends_with("ip="));
    }

    /// Yuva, iş parçacığı panikle bitse de geri gelir.
    #[test]
    fn yuva_panikte_geri_verilir() {
        let aktif = Arc::new(AtomicUsize::new(0));
        let yuva = Yuva::al(&aktif);
        assert_eq!(aktif.load(Ordering::SeqCst), 1);
        let t = std::thread::spawn(move || {
            let _yuva = yuva;
            panic!("deneme paniği");
        });
        assert!(t.join().is_err());
        assert_eq!(aktif.load(Ordering::SeqCst), 0);
        // normal bitiş de geri verir
        drop(Yuva::al(&aktif));
        assert_eq!(aktif.load(Ordering::SeqCst), 0);
    }
}
