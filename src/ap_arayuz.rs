//! AP arayüzüne panelden tek tıkla erişim (spec 2026-10-16-ap-arayuz). Panel "AP arayüzüne git" deyince hedef durumu
//! (`/run/wificorrect/ap-hedef.json`) yazılır; merkez (Caddy, ap.wificorrect.com) oturumu doğrulanan istekleri
//! `X-WFC-AP: 1` başlığıyla tünelden cihaz paneline (:8443) verir; panel bunları sayfa değil **aktarım** olarak işler
//! (`panel/ap_aktarim.rs`). Bu modül: hedef durumu ve denetimi, istek/cevap dönüştürme, AP'ye bağlantı.
//! - Hedef yalnızca açık yan odanın ağındaki adres (bağlantı yan oda aracısı üzerinden, SOCKS5) ya da kayıtlı AP adresi
//!   (misafir ağında, ana ad alanından). Misafir telefonlarına, cihazın kendisine, WAN'a, tünele aktarım yok: hedef her
//!   istekte yeniden denetlenir (`denetle`), bağlantı yalnızca hedefin adresine kurulur.
//! - AP'ye HTTP/1.0 (Connection: close, Accept-Encoding: identity); Host / Origin / Referer AP'nin kendi adresine
//!   çevrilir; X-WFC-*, X-Forwarded-*, wcc / wca / wfc çerezleri (merkezin "_" yaptığı parçalar dahil) ve koşullu istek
//!   başlıkları iletilmez.
//! - Cevap: başlıklardaki (Location, Refresh, CSP …) `http(s)://<hedef>` → `https://<gelen Host>` (merkezde
//!   ap.wificorrect.com); Set-Cookie'den Domain çıkar, wca / wcc / wfc adlı çerez yazılmaz (merkezin oturumunu ezmesin);
//!   metin içerikte (HTML/JS/CSS/JSON/XML, sıkıştırılmamış, en çok 2 MB) `http://<hedef>`, `//<hedef>` (JSON kaçışlı
//!   `\/\/` dahil) → `https://<Host>`, `//<Host>`; diğer içerik olduğu gibi akar. Her cevap `Cache-Control: no-store`:
//!   bütün müşterilerin AP'leri aynı kökende, bir AP'nin dosyası ötekine karışmasın. Panelin sıkı başlıkları (CSP,
//!   X-Frame-Options) eklenmez.
//! - AP aynı adreste başka şemaya/porta yönlendirirse (http → https://<hedef>/ ya da tersi) hedef güncellenir (döngü olmaz).
//! - HTTPS AP: TLS 1.2+ (rustls), sertifika doğrulamasız (AP'ler kendinden imzalı; bağlantı yan oda / misafir ağında).
//!   Yalnızca eski TLS'li AP'de ve arayüzünü JS ile sabit adresten kuran modelde yedek yol `ap-ac` betiği.
//! - Hedefin şeması (`web_sec`, 0.4.16): 443'te aynı TLS ayarıyla el sıkışma olursa HTTPS, olmazsa 80 (HTTP). Canlı
//!   ZyXEL: sayfa HTTPS'te açıldığı için AP'nin betiği istekleri şifrelemez; AP'ye 80'den gidilirse giriş döner. Panelde
//!   "HTTP ile aç" yalnızca 80'i dener (HTTPS arayüzü bozuk AP için).
//! - Bütün müşterilerin AP'leri tek kökte (final review 2026-10-16): Service Worker betiği, başka siteden gelen istek ve
//!   başka alt alandan gezinme dışı istek AP'ye gitmez (`istek_reddi`; merkez/Caddy de reddeder); kayıtlı AP adresine
//!   (misafir ağı) aktarım yalnızca komşu tablosundaki MAC kayıt anında sabitlenenle aynıysa (`baglan`).

use crate::ap::{self, Yollar};
use crate::ayar::Config;
use crate::ortak::{self, Row};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::time::Duration;

/// AP arayüzünün açıldığı alan adı (merkezde Caddy bloğu); gelen Host başlığı geçersizse
pub const ALAN: &str = "ap.wificorrect.com";

/// Yeniden yazmada kullanılan genel ad: gelen Host başlığı (yalnızca ad[:port] karakterleri), yoksa `ALAN`.
pub fn kamu_adi(host: Option<&str>) -> String {
    let h = host.map(str::trim).unwrap_or("").to_ascii_lowercase();
    let (ad, port) = match h.rsplit_once(':') {
        Some((a, p)) => (a, Some(p)),
        None => (h.as_str(), None),
    };
    let ad_ok = !ad.is_empty() && ad.len() <= 253 && ad.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    let port_ok = port.is_none_or(|p| (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
    if ad_ok && port_ok { h } else { ALAN.to_string() }
}
/// Merkezin geçiş adresi: `wcc` oturumundan kısa ömürlü belirteç üretip ap.wificorrect.com'a yönlendirir
pub const GECIS: &str = "https://cihaz.wificorrect.com/_ap-gecis";
/// Erişim süresi (sn; yan odada en çok yan odanın kalan süresi)
pub const SURE_SN: f64 = 1800.0;
/// İstek gövdesi sınırı (ör. ayar yedeği; büyük ürün yazılımı yüklemesi yedek yoldan)
pub const GOVDE_EN_COK: u64 = 8 * 1024 * 1024;
/// Yeniden yazılan metin gövdesi sınırı (aşan gövde değiştirilmeden akar)
const YAZ_EN_COK: usize = 2 * 1024 * 1024;
const BASLIK_EN_COK: usize = 64 * 1024;
const BASLIK_SAYI: usize = 100;
const BAGLANMA: Duration = Duration::from_secs(5);
/// Aracı hedefe en çok 5 sn bekler; SOCKS cevabı için biraz daha
const ARACI_BEKLEME: Duration = Duration::from_secs(8);
const BEKLEME: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hedef {
    pub adres: String,
    pub port: u16,
    /// http | https
    pub sema: String,
    /// yan_oda | misafir
    pub yol: String,
    pub bitis: f64,
    /// Açan (müşteri numarası) ve rolü (sahip | hizmet): denetim için
    pub kim: String,
    pub rol: String,
    /// Yan odanın kimliği: yan oda kapanıp yenisi açılınca eski hedef geçmez
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yan_oda: Option<String>,
    /// Misafir ağındaki (kayıtlı) AP'nin sabitlenmiş MAC'i (`[ap] mac`): bağlantıda komşu tablosundaki MAC bu olmalı
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}

impl Hedef {
    pub fn ip(&self) -> Option<Ipv4Addr> {
        self.adres.parse().ok()
    }

    fn varsayilan_port(&self) -> bool {
        (self.sema == "http" && self.port == 80) || (self.sema == "https" && self.port == 443)
    }

    /// Host başlığı: "192.168.1.1" ya da "192.168.1.1:8080"
    pub fn konak(&self) -> String {
        if self.varsayilan_port() { self.adres.clone() } else { format!("{}:{}", self.adres, self.port) }
    }

    /// "http://192.168.1.1" (Origin / Referer)
    pub fn koken(&self) -> String {
        format!("{}://{}", self.sema, self.konak())
    }
}

// ---------------------------------------------------------------- hedef durumu
/// Hedef denetimi: geçerli, yok ya da (süre doldu / yan oda kapandı / AP kaydı silindi …) kapandı. Kapanan hedefin
/// dosyası silinir; `Kapandi` yalnızca dosyayı silen çağırana döner (denetim satırı bir kez yazılır).
#[derive(Debug, PartialEq)]
pub enum Durum {
    Yok,
    Gecerli(Hedef),
    Kapandi(Hedef, &'static str),
}

fn gecersiz_neden(h: &Hedef, y: &Yollar, cfg: &Config, now: f64) -> Option<&'static str> {
    if h.bitis <= now {
        return Some("sure");
    }
    let Some(ip) = h.ip() else { return Some("bozuk") };
    if h.port == 0 || !matches!(h.sema.as_str(), "http" | "https") {
        return Some("bozuk");
    }
    match h.yol.as_str() {
        "yan_oda" => {
            let g = ap::gecici_oku(y);
            let ayni = g.as_ref().is_some_and(|g| h.yan_oda.as_deref() == Some(g.kimlik.as_str()) && ap::yan_odada(ip, g));
            (!ayni).then_some("yan_oda")
        }
        // kayıtlı adres ve kayıtta sabitlenen MAC aynı (MAC yeniden sabitlenir ya da silinirse açık erişim kapanır)
        "misafir" => (!ap::kayitli_ap_mi(ip, cfg) || h.mac.is_none() || cfg.ap.mac.get(&h.adres) != h.mac.as_ref()).then_some("adres_kaydi"),
        _ => Some("bozuk"),
    }
}

pub fn oku(y: &Yollar) -> Option<Hedef> {
    serde_json::from_slice(&std::fs::read(&y.hedef).ok()?).ok()
}

/// Her aktarımda ve panel bakışında: hedef hâlâ geçerli mi. Okunamayan dosya `Yok` sayılır ama silinmez (final review
/// M1: yazılırken okunan dosya geçerli bir hedefi sildirmesin; bozuksa yeni "AP arayüzüne git" ya da "Erişimi kapat" ezer).
pub fn denetle(y: &Yollar, cfg: &Config, now: f64) -> Durum {
    let Ok(b) = std::fs::read(&y.hedef) else { return Durum::Yok };
    let Ok(h) = serde_json::from_slice::<Hedef>(&b) else { return Durum::Yok };
    match gecersiz_neden(&h, y, cfg, now) {
        None => Durum::Gecerli(h),
        Some(n) if std::fs::remove_file(&y.hedef).is_ok() => Durum::Kapandi(h, n),
        Some(_) => Durum::Yok, // başka bir iş parçacığı az önce sildi (denetim satırını o yazar)
    }
}

/// Hedef dosyasını değiştiren işlemler (panel "AP arayüzüne git", aktarımda şema/port değişimi, MAC değişince kapatma)
/// bu süreçte sıraya girer (final review M1): aynı geçici dosyaya iki yazıcı birbirinin içeriğini kesmesin,
/// okuma-değiştirme-yazma araya giren yeni hedefi ezmesin. Hedefi yalnızca panel süreci yazar.
static HEDEF_KILIDI: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn hedef_kilidi() -> std::sync::MutexGuard<'static, ()> {
    HEDEF_KILIDI.lock().unwrap_or_else(|e| e.into_inner())
}

fn yaz_(y: &Yollar, h: &Hedef) -> Result<(), String> {
    let v = serde_json::to_vec(h).map_err(|e| e.to_string())?;
    ortak::write_atomic_mode(&y.hedef, &v, 0o600).map_err(|e| format!("AP hedefi yazılamadı: {e}"))
}

pub fn yaz(y: &Yollar, h: &Hedef) -> Result<(), String> {
    let _k = hedef_kilidi();
    yaz_(y, h)
}

/// AP aynı adreste başka şemaya/porta yönlendirdi: dosyadaki hedef hâlâ bu hedefse (adres, yol, bitiş) güncellenir.
pub fn sema_guncelle(y: &Yollar, h: &Hedef, sema: &str, port: u16) {
    let _k = hedef_kilidi();
    if let Some(mut g) = oku(y).filter(|g| g.adres == h.adres && g.yol == h.yol && g.bitis == h.bitis) {
        (g.sema, g.port) = (sema.to_string(), port);
        let _ = yaz_(y, &g);
    }
}

/// Dosyadaki hedef tam olarak `h` ise kapatır (aktarımda MAC değişti); arada yenisi yazıldıysa dokunmaz. Kapattıysa true.
pub fn kapat_eger(y: &Yollar, h: &Hedef) -> bool {
    let _k = hedef_kilidi();
    oku(y).is_some_and(|g| g == *h) && std::fs::remove_file(&y.hedef).is_ok()
}

/// Erişimi kapatır (panel "Kapat", yan oda kapanınca, fabrika). `yalniz_yan_oda`: yalnızca yan odadan giden hedef.
pub fn kapat(y: &Yollar, yalniz_yan_oda: bool) -> Option<Hedef> {
    let _k = hedef_kilidi();
    let b = std::fs::read(&y.hedef).ok()?;
    let Ok(h) = serde_json::from_slice::<Hedef>(&b) else {
        let _ = std::fs::remove_file(&y.hedef);
        return None;
    };
    if yalniz_yan_oda && h.yol != "yan_oda" {
        return None;
    }
    std::fs::remove_file(&y.hedef).ok()?;
    Some(h)
}

/// Açılacak hedefin yolu: yan oda (kimliği, bitişi) ya da misafir ağı (kayıtlı AP adresi)
#[derive(Debug, PartialEq)]
pub enum Yol {
    YanOda(String, f64),
    Misafir,
}

/// Panel "AP arayüzüne git": adres yalnızca açık yan odanın ağında (`yan_oda`: aracı çalışan yan oda) ya da kayıtlı AP
/// adresi (misafir ağında, havuz dışı, cihazın adresi değil) olabilir.
pub fn hedef_dogrula(girdi: &str, cfg: &Config, yan_oda: Option<&ap::Gecici>) -> Result<(Ipv4Addr, Yol), String> {
    let ip: Ipv4Addr = girdi.trim().parse().map_err(|_| "AP adresi geçerli bir IPv4 adresi değil.".to_string())?;
    if ap::misafir_aginda(ip, cfg) {
        if ap::kayitli_ap_mi(ip, cfg) {
            return Ok((ip, Yol::Misafir));
        }
        return Err(format!(
            "{ip} kayıtlı AP adresi değil: misafir ağındaki AP'ye gitmek için önce AP adresleri'ne kaydedin (DHCP havuzu dışında bir adres; \
             AP havuzdan adres aldıysa önce sabit adrese alın ya da betiği kullanın)."
        ));
    }
    match yan_oda {
        Some(g) if ap::yan_odada(ip, g) => Ok((ip, Yol::YanOda(g.kimlik.clone(), g.bitis))),
        Some(g) => Err(format!("{ip} açık yan odanın ({}) ağında değil.", g.adres)),
        None => Err("Bu adres için açık yan oda yok; önce \"Bağlan\" ile yan odayı açın.".into()),
    }
}

/// Denetim satırı (AP_ARAYUZ): panel dışından (kendiliğinden kapanma) kim + rol hedef dosyasından
pub fn denetim(log_root: &str, now: f64, h: &Hedef, ek: &str) {
    let satir = format!("kullanici={} rol={} ip=- {ek}", h.kim, h.rol);
    ortak::audit(log_root, Row::new("AP_ARAYUZ", &ortak::now_iso(now)).set("ek", satir));
}

pub fn kapandi_ek(h: &Hedef, neden: &str) -> String {
    format!("kapandi hedef={}:{} yol={} neden={neden}", h.adres, h.port, h.yol)
}

// ---------------------------------------------------------------- yeniden yazma
/// Hedefin metinde geçebilecek adları: adresi ve bu adrese giden bilinen adlar (ör. tplinkap.net)
pub fn konaklar(h: &Hedef) -> Vec<String> {
    let mut v = vec![h.adres.clone()];
    if let Some(ip) = h.ip() {
        v.extend(ap::BILINEN_ADLAR.iter().filter(|(_, o)| Ipv4Addr::from(*o) == ip).map(|(a, _)| a.to_string()));
    }
    v
}

fn ad_karakteri(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// `g[j..]` konaklardan biriyle (büyük/küçük harf fark etmez) başlıyor ve ardından ad devam etmiyorsa eşleşmenin sonu;
/// ardından `:port` geliyorsa o da eşleşmeye girer.
fn konak_eslesir(g: &[u8], j: usize, konaklar: &[String]) -> Option<usize> {
    for k in konaklar {
        let k = k.as_bytes();
        let Some(aday) = g.get(j..j + k.len()) else { continue };
        if !aday.eq_ignore_ascii_case(k) {
            continue;
        }
        let mut e = j + k.len();
        match g.get(e) {
            Some(b) if ad_karakteri(*b) => continue, // 192.168.1.10, tplinkap.network
            Some(b'.') if g.get(e + 1).is_some_and(|b| ad_karakteri(*b)) => continue, // 192.168.1.1.5
            Some(b':') => {
                let n = g[e + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
                if (1..=5).contains(&n) {
                    e += 1 + n;
                }
            }
            _ => {}
        }
        return Some(e);
    }
    None
}

/// Metindeki `http://H`, `https://H`, `//H` ve JSON kaçışlı `http:\/\/H` → `https://KAMU`, `//KAMU` (`:port` gider;
/// KAMU gelen Host başlığı, merkezde ap.wificorrect.com). Başka adreslere (192.168.1.10, başka siteler) dokunulmaz.
pub fn yeniden_yaz(g: &[u8], konaklar: &[String], kamu: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(g.len() + 64);
    let mut i = 0;
    while i < g.len() {
        let egik = if g[i..].starts_with(b"//") {
            2
        } else if g[i..].starts_with(b"\\/\\/") {
            4
        } else {
            0
        };
        if egik > 0 {
            if let Some(son) = konak_eslesir(g, i + egik, konaklar) {
                for sema in [&b"https:"[..], &b"http:"[..]] {
                    if out.len() >= sema.len() && out[out.len() - sema.len()..].eq_ignore_ascii_case(sema) {
                        out.truncate(out.len() - sema.len());
                        out.extend_from_slice(b"https:");
                        break;
                    }
                }
                out.extend_from_slice(&g[i..i + egik]);
                out.extend_from_slice(kamu.as_bytes());
                i = son;
                continue;
            }
        }
        out.push(g[i]);
        i += 1;
    }
    out
}

fn on_ek_sil<'a>(s: &'a str, on: &str) -> Option<&'a str> {
    (s.len() >= on.len() && s.is_char_boundary(on.len()) && s[..on.len()].eq_ignore_ascii_case(on)).then(|| &s[on.len()..])
}

/// Yönlendirme hedefin kendisine mi (başka şema / port olabilir): (şema, port)
pub fn konum_hedefi(deger: &str, konaklar: &[String]) -> Option<(String, u16)> {
    let d = deger.trim();
    let (sema, kalan) = match (on_ek_sil(d, "https://"), on_ek_sil(d, "http://")) {
        (Some(k), _) => ("https", k),
        (_, Some(k)) => ("http", k),
        _ => return None,
    };
    let yetki = &kalan[..kalan.find(['/', '?', '#']).unwrap_or(kalan.len())];
    let (konak, port) = match yetki.rsplit_once(':') {
        Some((k, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (k, p.parse::<u16>().ok().filter(|p| *p != 0)?),
        _ => (yetki, if sema == "https" { 443 } else { 80 }),
    };
    konaklar.iter().any(|k| k.eq_ignore_ascii_case(konak)).then(|| (sema.to_string(), port))
}

/// AP'nin yazmak istediği çerez merkezin (wca, wcc) ya da panelin (wfc) adını taşıyor mu: yazılmaz.
fn ayrilmis_cerez(set_cookie: &str) -> bool {
    let ad = set_cookie.split(';').next().unwrap_or("").split('=').next().unwrap_or("").trim();
    matches!(ad.to_ascii_lowercase().as_str(), "wca" | "wcc" | "wfc")
}

/// Set-Cookie: Domain özniteliği çıkar (çerez ap.wificorrect.com'a yazılsın). Tarayıcı öznitelik adının çevresindeki
/// boşluğu yok sayar (RFC 6265 §5.2): `Domain =x` de Domain'dir (final review M4).
pub fn domain_sil(deger: &str) -> String {
    deger
        .split(';')
        .enumerate()
        .filter(|(i, p)| *i == 0 || !p.split('=').next().unwrap_or("").trim().eq_ignore_ascii_case("domain"))
        .map(|(_, p)| p)
        .collect::<Vec<_>>()
        .join(";")
}

/// Cookie: merkezin (wcc, wca) ve panelin (wfc) çerezleri ve adsız ("_", merkezin temizlediği) parçalar gitmez.
pub fn cerez_suz(deger: &str) -> Option<String> {
    let v: Vec<&str> = deger
        .split(';')
        .map(str::trim)
        .filter(|c| c.split_once('=').is_some_and(|(a, _)| !a.trim().is_empty() && !matches!(a.trim(), "wcc" | "wca" | "wfc")))
        .collect();
    (!v.is_empty()).then(|| v.join("; "))
}

fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// AP kipinde AP'ye hiç gitmeyecek tarayıcı isteği (final review C1, I2; Caddy ap bloğu ve merkez de reddeder — eski ya
/// da yanlış kurulmuş merkeze karşı ikinci kat). Dönen: ret nedeni.
/// - Service Worker betiği (`Service-Worker` başlığı ya da `Sec-Fetch-Dest: serviceworker`): bütün müşterilerin AP'leri
///   tek kökte; bir AP'nin kaydettiği SW sonraki müşterilerin AP oturumlarını dinlerdi.
/// - Başka siteden (`Sec-Fetch-Site: cross-site`): GET ile ayar değiştiren AP'lere başka siteden istek.
/// - Aynı sitenin başka alt alanından (`same-site`) yalnızca üst düzey GET/HEAD gezinme geçer: cihaz panelinden gelen
///   yönlendirme zinciri (cihaz.wificorrect.com → /_ap-gecis → /_ap-giris → /, AP'nin kendi yönlendirmeleri dahil).
///   Başka alt alandan form, fetch, çerçeve AP'ye gitmez.
/// Sec-Fetch-* göndermeyen eski tarayıcı geçer (bu başlıkları sayfa değil tarayıcı yazar).
pub fn istek_reddi(yontem: &str, gelen: &[(String, String)]) -> Option<&'static str> {
    let b = |ad: &str| gelen.iter().find(|(a, _)| a.eq_ignore_ascii_case(ad)).map(|(_, d)| d.trim().to_ascii_lowercase());
    if gelen.iter().any(|(a, _)| a.eq_ignore_ascii_case("Service-Worker")) || b("Sec-Fetch-Dest").as_deref() == Some("serviceworker") {
        return Some("service_worker");
    }
    let gezinme = matches!(yontem, "GET" | "HEAD") && b("Sec-Fetch-Mode").as_deref() == Some("navigate") && b("Sec-Fetch-Dest").as_deref() == Some("document");
    match b("Sec-Fetch-Site").as_deref() {
        Some("cross-site") => Some("baska_site"),
        Some("same-site") if !gezinme => Some("alt_alan"),
        _ => None,
    }
}

/// `istek_reddi` nedeninin sayfadaki açıklaması
pub fn red_mesaji(neden: &str) -> &'static str {
    match neden {
        "service_worker" => "Tarayıcının arka plan betiği (Service Worker) AP arayüzünde kullanılamaz; istek AP'ye iletilmedi.",
        _ => "Başka bir siteden gelen istek AP arayüzüne iletilmez. AP arayüzünü cihaz panelinden açın.",
    }
}

fn deger_ok(s: &str) -> bool {
    !s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0)
}

/// AP'ye giden başlıklar (Host ve bağlantı başlıkları hariç; onları `aktar` yazar). `kamu`: tarayıcının gördüğü ad
/// (Origin / Referer onunla başlıyorsa AP'nin kendi adresine çevrilir, başkası AP'ye gitmez).
pub fn istek_basliklari(gelen: &[(String, String)], h: &Hedef, kamu: &str) -> Vec<(String, String)> {
    let kok = format!("https://{kamu}");
    let mut v = vec![];
    for (ad, deger) in gelen {
        if !token(ad) || !deger_ok(deger) {
            continue;
        }
        let k = ad.to_ascii_lowercase();
        if k.starts_with("x-wfc-") || k.starts_with("x-forwarded-") || k.starts_with("proxy-") {
            continue;
        }
        match k.as_str() {
            "host" | "connection" | "keep-alive" | "te" | "trailer" | "transfer-encoding" | "upgrade" | "content-length"
            | "accept-encoding" | "expect" | "forwarded" | "via" | "x-real-ip" | "cdn-loop" | "if-none-match"
            | "if-modified-since" | "if-match" | "if-unmodified-since" | "if-range" => {}
            "cookie" => {
                if let Some(c) = cerez_suz(deger) {
                    v.push(("Cookie".to_string(), c));
                }
            }
            "origin" => {
                if deger.trim().eq_ignore_ascii_case(&kok) {
                    v.push(("Origin".to_string(), h.koken()));
                }
            }
            "referer" => {
                if let Some(r) = on_ek_sil(deger.trim(), &kok).filter(|r| r.is_empty() || r.starts_with('/')) {
                    v.push(("Referer".to_string(), format!("{}{}", h.koken(), if r.is_empty() { "/" } else { r })));
                }
            }
            _ => v.push((ad.clone(), deger.clone())),
        }
    }
    v
}

// ---------------------------------------------------------------- AP'ye bağlantı
pub trait Akis: Read + Write + Send {}
impl<T: Read + Write + Send> Akis for T {}

pub type Baglanti = dyn Fn(&Hedef) -> io::Result<Box<dyn Akis>> + Send + Sync;

#[derive(Debug)]
struct Dogrulamasiz;

impl rustls::client::ServerCertVerifier for Dogrulamasiz {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

/// TLS'te gönderilen ad (SNI): AP IP adresiyle açılır; ad gönderilmesi gerekmez ama rustls bir ad ister (AP'ler SNI'ye
/// bakmaz)
pub const TLS_ADI: &str = "ap.wificorrect.local";

/// AP'ye TLS istemcisi: aktarım (`tls`) ve web arayüzü seçimi (`tls_el_sikis`) aynı ayarla — TLS 1.2+ (rustls güvenli
/// varsayılanları), sertifika doğrulamasız, SNI `TLS_ADI`.
fn tls_baglantisi() -> io::Result<rustls::ClientConnection> {
    use std::sync::{Arc, OnceLock};
    static AYAR: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let ayar = AYAR
        .get_or_init(|| {
            Arc::new(rustls::ClientConfig::builder().with_safe_defaults().with_custom_certificate_verifier(Arc::new(Dogrulamasiz)).with_no_client_auth())
        })
        .clone();
    let ad = rustls::ServerName::try_from(TLS_ADI).map_err(|_| io::Error::other("TLS adı"))?;
    rustls::ClientConnection::new(ayar, ad).map_err(|e| io::Error::other(format!("TLS: {e}")))
}

fn tls(tcp: TcpStream) -> io::Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
    let mut s = rustls::StreamOwned::new(tls_baglantisi()?, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock)?;
    }
    Ok(s)
}

/// Web arayüzü seçiminde TLS el sıkışmasının en uzun süresi (yavaş gömülü AP'de RSA imzası birkaç saniye sürebilir)
const TLS_DENEME: Duration = Duration::from_secs(5);
/// Misafir ağındaki hedefe TCP denemesi
const TCP_DENEME: Duration = Duration::from_secs(3);

/// 443'teki sunucu aktarımın TLS'iyle konuşabiliyor mu: aynı ayarla (`tls_baglantisi`) el sıkışma en çok `sure` içinde
/// biterse Ok (bağlantı close_notify ile kapatılır). Süre bütün el sıkışmaya uygulanır (her okumada kalan süre): parça
/// parça cevap veren cihaz da süreyi uzatamaz. TLS 1.0/1.1'li eski AP, 443'te düz HTTP, uyarı ya da kapanma: hata.
pub fn tls_el_sikis(mut tcp: TcpStream, sure: Duration) -> io::Result<()> {
    let son = std::time::Instant::now() + sure;
    let mut c = tls_baglantisi()?;
    let sonuc = el_sikis(&mut c, &mut tcp, son);
    if sonuc.is_ok() {
        c.send_close_notify();
    }
    // kuyruktaki son kayıt (close_notify ya da rustls'in hata uyarısı) gönderilir; sonucu değiştirmez
    let _ = c.write_tls(&mut tcp);
    sonuc
}

fn el_sikis(c: &mut rustls::ClientConnection, tcp: &mut TcpStream, son: std::time::Instant) -> io::Result<()> {
    loop {
        let kalan = son.saturating_duration_since(std::time::Instant::now());
        if kalan.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "TLS el sıkışması süre içinde bitmedi"));
        }
        tcp.set_read_timeout(Some(kalan))?;
        tcp.set_write_timeout(Some(kalan))?;
        while c.wants_write() {
            c.write_tls(tcp)?;
        }
        if !c.is_handshaking() {
            return Ok(());
        }
        if !c.wants_read() {
            return Err(io::Error::other("TLS el sıkışması ilerlemiyor"));
        }
        if c.read_tls(tcp)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        c.process_new_packets().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    }
}

/// Web arayüzü seçimi kuralı (canlı ZyXEL bulgusu, 2026-10-18): arayüz tarayıcıda hep https://ap.wificorrect.com'da
/// açılır; `location.protocol` "https:" olduğu için bazı AP'lerin betikleri (ZyXEL, Vue, 2020) girişi ve cgi-bin
/// isteklerini şifrelemez, oysa aynı AP 80'de şifreli istek bekler (giriş döngüye girer). Bu yüzden önce HTTPS:
/// `https()` = 443'te aktarımın TLS'iyle el sıkışma başarılı → 443; değilse `http()` = 80'e TCP bağlantısı → 80.
/// Dönen port: 443 = https, 80 = http. Yalnızca TCP'si açık 443 yetmez (eski TLS'li AP 80'e düşer).
pub fn web_sec(https: impl FnOnce() -> bool, http: impl FnOnce() -> bool) -> Option<u16> {
    if https() {
        Some(443)
    } else if http() {
        Some(80)
    } else {
        None
    }
}

/// `web_sec` gerçek bağlantılarla. `baglan(port)`: hedefin o portuna TCP bağlantısı (misafir ağında doğrudan, yan odada
/// aracı üzerinden); `sure`: TLS el sıkışmasının en uzun süresi.
pub fn web_bul(baglan: &dyn Fn(u16) -> io::Result<TcpStream>, sure: Duration) -> Option<u16> {
    web_sec(|| baglan(443).and_then(|t| tls_el_sikis(t, sure)).is_ok(), || baglan(80).is_ok())
}

/// Hedefin portuna TCP: misafir ağında doğrudan, yan odada aracı üzerinden
fn tcp_baglan(a: SocketAddrV4, yan_oda: bool) -> io::Result<TcpStream> {
    if yan_oda {
        crate::yan_oda::socks_baglan(a, ARACI_BEKLEME)
    } else {
        TcpStream::connect_timeout(&a.into(), TCP_DENEME)
    }
}

/// Kayıtlı AP adresinde (misafir ağı) başka MAC: aktarım reddedilir (final review I1). `simdi`: komşu tablosundaki MAC.
#[derive(Debug)]
pub struct MacFarkli {
    pub kayitli: String,
    pub simdi: Option<String>,
}

impl std::fmt::Display for MacFarkli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MAC adresi değişmiş: kayıtlı {}, şu an {}", self.kayitli, self.simdi.as_deref().unwrap_or("okunamadı"))
    }
}

impl std::error::Error for MacFarkli {}

/// Misafir ağındaki hedefe bağlandıktan sonra (adres çözülmüş olur): komşu tablosundaki MAC sabitlenen MAC mi. Misafir
/// ağında AP'nin adresini elle alan bir cihaz AP yerine geçip yazılan AP parolasını toplamasın.
pub fn mac_denetle(h: &Hedef, ip: Ipv4Addr, arp: &str) -> io::Result<()> {
    let Some(kayitli) = &h.mac else {
        return Err(io::Error::other(MacFarkli { kayitli: "yok".into(), simdi: ap::arp_mac(arp, ip) }));
    };
    match ap::arp_mac(arp, ip) {
        Some(m) if m == *kayitli => Ok(()),
        simdi => Err(io::Error::other(MacFarkli { kayitli: kayitli.clone(), simdi })),
    }
}

/// Gerçek bağlantı: yan oda hedefi aracı (127.0.0.1:1085) üzerinden, misafir ağı hedefi ana ad alanından (bağlandıktan
/// sonra MAC denetimi); bağlanılan adres yalnızca hedefin adresi. HTTPS'te TLS.
pub fn baglan(h: &Hedef) -> io::Result<Box<dyn Akis>> {
    let ip = h.ip().ok_or_else(|| io::Error::other("hedef adresi bozuk"))?;
    let adres = SocketAddrV4::new(ip, h.port);
    let tcp = if h.yol == "yan_oda" {
        crate::yan_oda::socks_baglan(adres, ARACI_BEKLEME)?
    } else {
        let t = TcpStream::connect_timeout(&adres.into(), BAGLANMA)?;
        mac_denetle(h, ip, &std::fs::read_to_string("/proc/net/arp").unwrap_or_default())?;
        t
    };
    tcp.set_read_timeout(Some(BEKLEME))?;
    tcp.set_write_timeout(Some(BEKLEME))?;
    if h.sema == "https" {
        Ok(Box::new(tls(tcp)?))
    } else {
        Ok(Box::new(tcp))
    }
}

/// Panel "AP arayüzüne git", AP adresi kaydı ve arama işi: hedefin web arayüzü (`web_bul`; yan odada aracı üzerinden).
/// `yalniz_http`: yalnızca 80. Dönen: port.
pub fn web_portu(ip: Ipv4Addr, yan_oda: bool, yalniz_http: bool) -> Option<u16> {
    let baglan = |p: u16| tcp_baglan(SocketAddrV4::new(ip, p), yan_oda);
    if yalniz_http {
        return baglan(80).is_ok().then_some(80);
    }
    web_bul(&baglan, TLS_DENEME)
}

// ---------------------------------------------------------------- aktarım
#[derive(Debug, PartialEq)]
pub enum Hata {
    /// İstek kabul edilmez (400)
    Gecersiz(&'static str),
    /// Gövde sınırı aşıldı (413)
    Buyuk,
    /// AP'ye bağlanılamadı (502)
    Ulasilamadi(String),
    /// AP geçersiz / yarım cevap verdi (502)
    Bozuk(String),
    /// Kayıtlı AP adresinde başka MAC (403): (kayıtlı, şu an)
    MacDegisti(String, Option<String>),
}

impl Hata {
    pub fn durum(&self) -> u16 {
        match self {
            Hata::Gecersiz(_) => 400,
            Hata::Buyuk => 413,
            Hata::MacDegisti(..) => 403,
            _ => 502,
        }
    }

    pub fn baslik(&self) -> &'static str {
        match self {
            Hata::MacDegisti(..) => "AP'nin MAC adresi değişmiş",
            _ => "AP arayüzü",
        }
    }

    pub fn mesaj(&self) -> String {
        match self {
            Hata::Gecersiz(m) => format!("İstek AP'ye iletilemez: {m}."),
            Hata::Buyuk => format!("İstek çok büyük (en çok {} MB); büyük dosya yüklemek için betiği kullanın.", GOVDE_EN_COK / 1024 / 1024),
            Hata::Ulasilamadi(e) => format!("AP'ye ulaşılamadı ({e}). AP açık ve misafir portuna bağlı mı, yan oda hâlâ açık mı?"),
            Hata::Bozuk(e) => format!("AP geçerli bir cevap vermedi ({e})."),
            Hata::MacDegisti(k, s) => format!("İstek AP'ye iletilmedi, AP arayüzü erişimi kapatıldı. {}", mac_degisti_mesaji(k, s.as_deref())),
        }
    }
}

/// Panel ("AP arayüzüne git") ve aktarım aynı açıklamayı gösterir
pub fn mac_degisti_mesaji(kayitli: &str, simdi: Option<&str>) -> String {
    format!(
        "Bu adreste kayıtlı AP'den başka bir cihaz cevap veriyor (kayıtlı MAC {kayitli}, şu an {}). AP değiştirildiyse cihaz \
         panelinde Ayarlar → AP kurulumu → AP adresleri'nde Kaydet'e basarak yeni MAC'i sabitleyin; değiştirilmediyse misafir \
         ağında bu adresi başka bir cihaz kullanıyor olabilir (Ağdaki cihazlar).",
        simdi.unwrap_or("okunamadı")
    )
}

pub struct Cevap {
    pub durum: u16,
    pub basliklar: Vec<(String, String)>,
    pub govde: Box<dyn Read + Send>,
    /// Bilinen gövde boyu (yoksa sunucu parçalı gönderir)
    pub boy: Option<u64>,
}

fn satir_oku<R: BufRead>(r: &mut R, kalan: &mut usize) -> Result<Option<String>, String> {
    let mut l = Vec::new();
    let n = r.take(*kalan as u64 + 1).read_until(b'\n', &mut l).map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > *kalan {
        return Err("başlıklar çok büyük".into());
    }
    *kalan -= n;
    while l.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        l.pop();
    }
    Ok(Some(String::from_utf8_lossy(&l).into_owned()))
}

/// Cevabın durum satırı ve başlıkları (devam satırları birleşir; boyut ve sayı sınırlı)
pub fn bas_oku<R: BufRead>(r: &mut R) -> Result<(u16, Vec<(String, String)>), String> {
    let mut kalan = BASLIK_EN_COK;
    let ilk = loop {
        match satir_oku(r, &mut kalan)? {
            None => return Err("AP cevap vermeden bağlantıyı kapattı".into()),
            Some(l) if l.trim().is_empty() => continue,
            Some(l) => break l,
        }
    };
    let mut p = ilk.split_whitespace();
    let (Some(surum), Some(kod)) = (p.next(), p.next()) else { return Err("durum satırı bozuk".into()) };
    if !surum.starts_with("HTTP/") {
        return Err("HTTP cevabı değil".into());
    }
    let kod: u16 = kod.parse().ok().filter(|k| (100..=599).contains(k)).ok_or("durum kodu bozuk")?;
    let mut basliklar: Vec<(String, String)> = vec![];
    while let Some(l) = satir_oku(r, &mut kalan)? {
        if l.is_empty() {
            break;
        }
        if l.starts_with([' ', '\t']) {
            if let Some(son) = basliklar.last_mut() {
                son.1.push(' ');
                son.1.push_str(l.trim());
            }
            continue;
        }
        let Some((ad, deger)) = l.split_once(':') else { continue };
        if basliklar.len() >= BASLIK_SAYI {
            return Err("çok fazla başlık".into());
        }
        basliklar.push((ad.trim().to_string(), deger.trim().to_string()));
    }
    Ok((kod, basliklar))
}

/// Parçalı (chunked) gövde çözücü: HTTP/1.0 isteğe parçalı cevap veren AP'ler için
pub struct Parcali<R> {
    r: R,
    kalan: u64,
    bitti: bool,
}

impl<R: BufRead> Parcali<R> {
    pub fn new(r: R) -> Parcali<R> {
        Parcali { r, kalan: 0, bitti: false }
    }
}

impl<R: BufRead> Read for Parcali<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.bitti || buf.is_empty() {
            return Ok(0);
        }
        if self.kalan == 0 {
            let mut l = String::new();
            if (&mut self.r).take(1024).read_line(&mut l)? == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let boy = u64::from_str_radix(l.split(';').next().unwrap_or("").trim(), 16).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            if boy == 0 {
                loop {
                    let mut t = String::new();
                    if (&mut self.r).take(8192).read_line(&mut t)? == 0 || t.trim().is_empty() {
                        break;
                    }
                }
                self.bitti = true;
                return Ok(0);
            }
            self.kalan = boy;
        }
        let en_cok = buf.len().min(usize::try_from(self.kalan).unwrap_or(usize::MAX));
        let n = self.r.read(&mut buf[..en_cok])?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.kalan -= n as u64;
        if self.kalan == 0 {
            let mut crlf = String::new();
            (&mut self.r).take(2).read_line(&mut crlf)?;
        }
        Ok(n)
    }
}

/// Bağlantı kapanınca biten gövde: TLS'te close_notify göndermeden kapanan AP'nin "beklenmeyen son"u gövdenin sonudur.
struct SessizSon<R>(R);

impl<R: Read> Read for SessizSon<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
            r => r,
        }
    }
}

/// Yeniden yazılacak metin türü mü (tür yazmayan eski sunucular: gövde ikili değilse metin sayılır)
fn metin_turu(tur: Option<&str>) -> Option<bool> {
    let t = tur?.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Some(t.starts_with("text/") || t.contains("javascript") || t.contains("ecmascript") || t.contains("json") || t.contains("xml"))
}

fn istek_yaz<W: Write + ?Sized>(akis: &mut W, bas: &[u8], tampon: Option<&[u8]>, govde: &mut dyn Read, boy: u64) -> io::Result<()> {
    akis.write_all(bas)?;
    match tampon {
        Some(b) => akis.write_all(b)?,
        None if boy > 0 => {
            if io::copy(&mut govde.take(boy), akis)? != boy {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        None => {}
    }
    akis.flush()
}

/// Bir isteği AP'ye aktarır. `kamu`: tarayıcının gördüğü ad (gelen Host, `kamu_adi`); `govde_boy`: istek gövdesinin
/// bilinen boyu (yoksa en çok `GOVDE_EN_COK` okunur); `sema_degisti`: AP aynı adreste başka şemaya/porta yönlendirdi
/// (hedef dosyası güncellenir).
#[allow(clippy::too_many_arguments)]
pub fn aktar(
    h: &Hedef,
    kamu: &str,
    yontem: &str,
    yol: &str,
    gelen: &[(String, String)],
    govde_boy: Option<u64>,
    govde: &mut dyn Read,
    baglanti: &Baglanti,
    sema_degisti: &dyn Fn(&str, u16),
) -> Result<Cevap, Hata> {
    if yontem.is_empty() || yontem.len() > 16 || !yontem.bytes().all(|b| b.is_ascii_uppercase()) || matches!(yontem, "CONNECT" | "TRACE") {
        return Err(Hata::Gecersiz("yöntem desteklenmiyor"));
    }
    if !yol.starts_with('/') || yol.len() > 8192 || !yol.bytes().all(|b| (0x21..0x7f).contains(&b)) {
        return Err(Hata::Gecersiz("adres geçersiz"));
    }
    // gövde: boyu biliniyorsa akar, bilinmiyorsa (parçalı) sınıra kadar okunur
    let (boy, tampon) = match govde_boy {
        Some(n) if n > GOVDE_EN_COK => return Err(Hata::Buyuk),
        Some(n) => (n, None),
        None => {
            let mut b = Vec::new();
            (&mut *govde).take(GOVDE_EN_COK + 1).read_to_end(&mut b).map_err(|e| Hata::Gecersiz(if e.kind() == io::ErrorKind::TimedOut { "gövde gelmedi" } else { "gövde okunamadı" }))?;
            if b.len() as u64 > GOVDE_EN_COK {
                return Err(Hata::Buyuk);
            }
            (b.len() as u64, Some(b))
        }
    };
    let mut bas = format!("{yontem} {yol} HTTP/1.0\r\nHost: {}\r\n", h.konak());
    for (ad, deger) in istek_basliklari(gelen, h, kamu) {
        bas.push_str(&format!("{ad}: {deger}\r\n"));
    }
    bas.push_str("Accept-Encoding: identity\r\nConnection: close\r\n");
    if boy > 0 || !matches!(yontem, "GET" | "HEAD" | "DELETE" | "OPTIONS") {
        bas.push_str(&format!("Content-Length: {boy}\r\n"));
    }
    bas.push_str("\r\n");
    let mut akis = baglanti(h).map_err(|e| match e.get_ref().and_then(|x| x.downcast_ref::<MacFarkli>()) {
        Some(m) => Hata::MacDegisti(m.kayitli.clone(), m.simdi.clone()),
        None => Hata::Ulasilamadi(e.to_string()),
    })?;
    istek_yaz(&mut *akis, bas.as_bytes(), tampon.as_deref(), govde, boy).map_err(|e| Hata::Ulasilamadi(format!("istek gönderilemedi: {e}")))?;
    let mut br = BufReader::with_capacity(16 * 1024, akis);
    let (kod, basliklar) = loop {
        let (k, b) = bas_oku(&mut br).map_err(Hata::Bozuk)?;
        if (100..200).contains(&k) && k != 101 {
            continue; // 100 Continue, 103 Early Hints …
        }
        break (k, b);
    };
    if kod == 101 {
        return Err(Hata::Bozuk("protokol değişimi (WebSocket) desteklenmiyor".into()));
    }
    let konak_adlari = konaklar(h);
    let mut cikis: Vec<(String, String)> = vec![];
    let (mut uzunluk, mut parcali, mut kodlama, mut tur) = (None::<u64>, false, None::<String>, None::<String>);
    for (ad, deger) in basliklar {
        if !token(&ad) || !deger_ok(&deger) {
            continue;
        }
        let k = ad.to_ascii_lowercase();
        if k.starts_with("proxy-") {
            continue;
        }
        match k.as_str() {
            "content-length" => uzunluk = deger.trim().parse().ok(),
            "transfer-encoding" => parcali = deger.to_ascii_lowercase().contains("chunked"),
            // service-worker-allowed: SW kapsamını genişletir (SW zaten aktarılmaz; yine de bu kökte hiç yazılmaz)
            "connection" | "keep-alive" | "trailer" | "upgrade" | "te" | "alt-svc" | "cache-control" | "expires" | "etag"
            | "last-modified" | "pragma" | "strict-transport-security" | "service-worker-allowed" => {}
            "set-cookie" if ayrilmis_cerez(&deger) => {}
            "set-cookie" => cikis.push((ad, domain_sil(&deger))),
            _ => {
                if k == "location" {
                    if let Some((s, p)) = konum_hedefi(&deger, &konak_adlari).filter(|(s, p)| *s != h.sema || *p != h.port) {
                        sema_degisti(&s, p);
                    }
                }
                if k == "content-encoding" {
                    kodlama = Some(deger.clone());
                }
                if k == "content-type" {
                    tur = Some(deger.clone());
                }
                let yeni = String::from_utf8_lossy(&yeniden_yaz(deger.as_bytes(), &konak_adlari, kamu)).into_owned();
                cikis.push((ad, yeni));
            }
        }
    }
    cikis.push(("Cache-Control".into(), "no-store".into()));
    let bos = yontem == "HEAD" || kod < 200 || kod == 204 || kod == 304;
    // Akan gövdenin boyu bildirilmez (final review M3): AP bildirdiğinden kısa gövdeyle kapatırsa Content-Length'li cevap
    // Caddy ile bağlantıyı kaydırır (istek asılı kalır); boysuz cevabı tiny_http parçalı gönderir ve kısa da olsa düzgün
    // bitirir. Boy yalnızca HEAD'de (gövdesiz) ve yeniden yazılan (bellekteki) gövdede bildirilir.
    let (mut govde, mut boy): (Box<dyn Read + Send>, Option<u64>) = if bos {
        (Box::new(io::empty()), if yontem == "HEAD" { uzunluk } else { Some(0) })
    } else if parcali {
        (Box::new(Parcali::new(br)), None)
    } else if let Some(n) = uzunluk {
        (Box::new(br.take(n)), None)
    } else {
        (Box::new(SessizSon(br)), None)
    };
    let sikistirilmis = kodlama.is_some_and(|k| !k.trim().eq_ignore_ascii_case("identity"));
    if !bos && !sikistirilmis && metin_turu(tur.as_deref()) != Some(false) {
        let mut on = Vec::new();
        (&mut govde).take(YAZ_EN_COK as u64 + 1).read_to_end(&mut on).map_err(|e| Hata::Bozuk(format!("gövde okunamadı: {e}")))?;
        let ikili = tur.is_none() && on.iter().take(1024).any(|b| *b == 0);
        if on.len() <= YAZ_EN_COK && !ikili {
            let yeni = yeniden_yaz(&on, &konak_adlari, kamu);
            boy = Some(yeni.len() as u64);
            govde = Box::new(Cursor::new(yeni));
        } else {
            govde = Box::new(Cursor::new(on).chain(govde)); // büyük ya da ikili: olduğu gibi
        }
    }
    Ok(Cevap { durum: kod, basliklar: cikis, govde, boy })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn hedef(adres: &str, yol: &str) -> Hedef {
        Hedef { adres: adres.into(), port: 80, sema: "http".into(), yol: yol.into(), bitis: 2000.0, kim: "4511643".into(), rol: "sahip".into(),
                yan_oda: None, mac: None }
    }

    fn k(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn metin_yeniden_yazma() {
        let ks = k(&["192.168.1.1", "tplinkap.net"]);
        let y = |s: &str| String::from_utf8(yeniden_yaz(s.as_bytes(), &ks, "ap.wificorrect.com")).unwrap();
        assert_eq!(y("<a href=\"http://192.168.1.1/x.htm\">"), "<a href=\"https://ap.wificorrect.com/x.htm\">");
        assert_eq!(y("HTTPS://192.168.1.1:443/a"), "https://ap.wificorrect.com/a");
        assert_eq!(y("src='//192.168.1.1:80/js/a.js'"), "src='//ap.wificorrect.com/js/a.js'");
        assert_eq!(y("{\"u\":\"http:\\/\\/192.168.1.1\\/cgi\"}"), "{\"u\":\"https:\\/\\/ap.wificorrect.com\\/cgi\"}");
        assert_eq!(y("location.href='http://TPLINKAP.NET/'"), "location.href='https://ap.wificorrect.com/'");
        assert_eq!(y("http://192.168.1.1"), "https://ap.wificorrect.com");
        assert_eq!(y("url(http://192.168.1.1/a.png);"), "url(https://ap.wificorrect.com/a.png);");
        // başka adresler: değişmez
        for s in ["http://192.168.1.10/", "http://192.168.1.1.5/", "http://tplinkap.network/", "192.168.1.1/x", "http://example.com/192.168.1.1",
                  "http://10.50.0.1/", "ftp:/192.168.1.1", "//192.168.1.1-x/"] {
            assert_eq!(y(s), s, "{s}");
        }
        // cümle sonu nokta ve tırnak sınırdır
        assert_eq!(y("Girin: http://192.168.1.1."), "Girin: https://ap.wificorrect.com.");
        assert_eq!(y("a //192.168.1.1\" b //192.168.1.1"), "a //ap.wificorrect.com\" b //ap.wificorrect.com");
        assert_eq!(yeniden_yaz(b"\xff\x00http://192.168.1.1/\xfe", &ks, "ap.wificorrect.com"), b"\xff\x00https://ap.wificorrect.com/\xfe");
        // genel ad gelen Host başlığından
        assert_eq!(yeniden_yaz(b"http://192.168.1.1/a", &ks, "ap.ornek.test:8443"), b"https://ap.ornek.test:8443/a");
        assert_eq!(kamu_adi(Some(" AP.WifiCorrect.com ")), "ap.wificorrect.com");
        assert_eq!(kamu_adi(Some("ap.ornek.test:8443")), "ap.ornek.test:8443");
        for kotu in [None, Some(""), Some("a b"), Some("x\"y"), Some("ap.wificorrect.com:"), Some("ap.wificorrect.com:123456"), Some("[::1]")] {
            assert_eq!(kamu_adi(kotu), ALAN, "{kotu:?}");
        }
    }

    #[test]
    fn konum_cerez_ve_basliklar() {
        let ks = k(&["192.168.1.1"]);
        assert_eq!(konum_hedefi("https://192.168.1.1/login", &ks), Some(("https".into(), 443)));
        assert_eq!(konum_hedefi("HTTP://192.168.1.1:8080", &ks), Some(("http".into(), 8080)));
        assert_eq!(konum_hedefi("http://192.168.1.1?a=1", &ks), Some(("http".into(), 80)));
        for d in ["/login", "http://192.168.1.10/", "https://example.com/", "http://192.168.1.1:0/", "http://192.168.1.1:99999/"] {
            assert_eq!(konum_hedefi(d, &ks), None, "{d}");
        }
        assert_eq!(domain_sil("SID=abc; Domain=192.168.1.1; Path=/; HttpOnly"), "SID=abc; Path=/; HttpOnly");
        assert_eq!(domain_sil("domain=x; path=/"), "domain=x; path=/"); // ilk parça çerezin kendisi
        // tarayıcı öznitelik adının çevresindeki boşluğu yok sayar: "Domain =x" de üst alana yazardı (final review M4)
        assert_eq!(domain_sil("a=1; Domain =wificorrect.com; Path=/"), "a=1; Path=/");
        assert_eq!(domain_sil("a=1;DOMAIN= .wificorrect.com ;Secure; domain"), "a=1;Secure");
        assert_eq!(domain_sil("a=1; Domainx=y"), "a=1; Domainx=y");
        assert_eq!(cerez_suz("wcc=1; SID=abc; _; wca=2; wfc=3; lang=tr"), Some("SID=abc; lang=tr".into()));
        assert_eq!(cerez_suz("wca=2; _"), None);
        let h = hedef("192.168.1.1", "yan_oda");
        let gelen: Vec<(String, String)> = [
            ("Host", "ap.wificorrect.com"), ("X-WFC-AP", "1"), ("X-WFC-Kullanici", "4511643"), ("X-Forwarded-For", "1.2.3.4"),
            ("X-Forwarded-Proto", "https"), ("Cookie", "wca=gizli; SID=abc"), ("Origin", "https://ap.wificorrect.com"),
            ("Referer", "https://ap.wificorrect.com/userRpm/Index.htm"), ("Authorization", "Basic YWRtaW46YWRtaW4="),
            ("Accept-Encoding", "gzip, br"), ("If-None-Match", "\"x\""), ("Connection", "keep-alive"), ("Content-Length", "5"),
            ("Accept", "*/*"), ("Kötü Ad", "x"), ("Via", "2.0 Caddy"),
        ].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let v = istek_basliklari(&gelen, &h, "ap.wificorrect.com");
        let ad = |n: &str| v.iter().find(|(a, _)| a.eq_ignore_ascii_case(n)).map(|(_, d)| d.as_str());
        assert_eq!(ad("Cookie"), Some("SID=abc"));
        assert_eq!(ad("Origin"), Some("http://192.168.1.1"));
        assert_eq!(ad("Referer"), Some("http://192.168.1.1/userRpm/Index.htm"));
        assert_eq!(ad("Authorization"), Some("Basic YWRtaW46YWRtaW4="), "eski TP-Link: HTTP Basic olduğu gibi");
        assert_eq!(ad("Accept"), Some("*/*"));
        for yok in ["Host", "X-WFC-AP", "X-WFC-Kullanici", "X-Forwarded-For", "X-Forwarded-Proto", "Accept-Encoding", "If-None-Match", "Connection", "Content-Length", "Kötü Ad", "Via"] {
            assert_eq!(ad(yok), None, "{yok}");
        }
        // başka sitenin Referer'ı / Origin'i AP'ye gitmez; https hedefinde port yazılır
        let mut h2 = h.clone();
        (h2.sema, h2.port) = ("https".into(), 8443);
        let v = istek_basliklari(&[("Referer".into(), "https://cihaz.wificorrect.com/ayarlar".into()), ("Origin".into(), "https://ap.wificorrect.com".into())], &h2, "ap.wificorrect.com");
        assert_eq!(v, vec![("Origin".to_string(), "https://192.168.1.1:8443".to_string())]);
        assert_eq!((h2.konak(), h.konak()), ("192.168.1.1:8443".to_string(), "192.168.1.1".to_string()));
    }

    #[test]
    fn bas_ve_parcali_govde() {
        let mut r = Cursor::new(b"\r\nHTTP/1.1 302 Found\r\nLocation: /a\r\nX-Uzun: bir\r\n  iki\r\nbozuk satir\r\n\r\ngovde".to_vec());
        let (kod, b) = bas_oku(&mut r).unwrap();
        assert_eq!((kod, b), (302, vec![("Location".into(), "/a".into()), ("X-Uzun".into(), "bir iki".into())]));
        let mut kalan = String::new();
        r.read_to_string(&mut kalan).unwrap();
        assert_eq!(kalan, "govde");
        assert!(bas_oku(&mut Cursor::new(b"".to_vec())).unwrap_err().contains("kapattı"));
        assert!(bas_oku(&mut Cursor::new(b"SSH-2.0-dropbear\r\n\r\n".to_vec())).is_err());
        assert!(bas_oku(&mut Cursor::new(b"HTTP/1.0 999 X\r\n\r\n".to_vec())).is_err());
        let mut buyuk = b"HTTP/1.0 200 OK\r\n".to_vec();
        buyuk.extend(vec![b'a'; BASLIK_EN_COK + 10]);
        assert!(bas_oku(&mut Cursor::new(buyuk)).unwrap_err().contains("büyük"));
        let mut p = Parcali::new(Cursor::new(b"5;x=1\r\nmerha\r\n2\r\nba\r\n0\r\nX-Son: 1\r\n\r\nfazla".to_vec()));
        let mut v = String::new();
        p.read_to_string(&mut v).unwrap();
        assert_eq!(v, "merhaba");
        let mut yarim = Parcali::new(Cursor::new(b"a\r\nkisa".to_vec()));
        assert!(yarim.read_to_string(&mut String::new()).is_err());
    }

    /// Sahte AP: tek bağlantı, isteği (başlık + Content-Length kadar gövde) kaydeder, verilen cevabı yazar ve kapatır.
    fn sahte_ap(cevap: Vec<u8>) -> (std::net::SocketAddr, Arc<Mutex<Vec<u8>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let adres = l.local_addr().unwrap();
        let gelen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let g2 = gelen.clone();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut br = BufReader::new(s.try_clone().unwrap());
            let mut istek = Vec::new();
            let mut boy = 0usize;
            loop {
                let mut satir = Vec::new();
                if br.read_until(b'\n', &mut satir).unwrap() == 0 {
                    break;
                }
                let t = String::from_utf8_lossy(&satir).to_ascii_lowercase();
                if let Some(n) = t.strip_prefix("content-length:") {
                    boy = n.trim().parse().unwrap();
                }
                istek.extend(&satir);
                if satir == b"\r\n" {
                    break;
                }
            }
            let mut govde = vec![0u8; boy];
            br.read_exact(&mut govde).unwrap();
            istek.extend(govde);
            *g2.lock().unwrap() = istek;
            s.write_all(&cevap).unwrap();
        });
        (adres, gelen)
    }

    fn baglanti(adres: std::net::SocketAddr) -> Box<Baglanti> {
        Box::new(move |_: &Hedef| Ok(Box::new(TcpStream::connect(adres)?) as Box<dyn Akis>))
    }

    fn basliklar(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn deger<'a>(c: &'a Cevap, ad: &str) -> Vec<&'a str> {
        c.basliklar.iter().filter(|(a, _)| a.eq_ignore_ascii_case(ad)).map(|(_, d)| d.as_str()).collect()
    }

    fn govde(c: Cevap) -> Vec<u8> {
        let mut v = vec![];
        let mut g = c.govde;
        g.read_to_end(&mut v).unwrap();
        v
    }

    #[test]
    fn aktarim_istek_ve_cevap() {
        let html = "<html><a href=\"http://192.168.1.1/b.htm\">b</a><script src=\"//192.168.1.1/a.js\"></script>192.168.1.1</html>";
        let cevap = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=gb2312\r\nContent-Length: {}\r\nSet-Cookie: SID=1; Domain=192.168.1.1; Path=/\r\n\
             Set-Cookie: wca=ap-yazdi; Path=/\r\nSet-Cookie: WCC=x\r\n\
             Cache-Control: max-age=600\r\nETag: \"e\"\r\nConnection: keep-alive\r\nX-Frame-Options: SAMEORIGIN\r\n\r\n{html}",
            html.len()
        );
        let (adres, gelen) = sahte_ap(cevap.into_bytes());
        let h = hedef("192.168.1.1", "yan_oda");
        let degisti = Mutex::new(None);
        let istek = basliklar(&[("Cookie", "_; SID=1"), ("X-WFC-AP", "1"), ("Referer", "https://ap.wificorrect.com/"), ("Content-Type", "application/x-www-form-urlencoded"),
                                ("X-Forwarded-Host", "ap.wificorrect.com"), ("Host", "ap.wificorrect.com")]);
        let mut g: &[u8] = b"a=1&b=2";
        let c = aktar(&h, "ap.wificorrect.com", "POST", "/userRpm/LoginRpm.htm?Save=Save", &istek, Some(7), &mut g, &*baglanti(adres), &|s: &str, p: u16| *degisti.lock().unwrap() = Some((s.to_string(), p))).unwrap();
        let giden = String::from_utf8(gelen.lock().unwrap().clone()).unwrap();
        assert!(giden.starts_with("POST /userRpm/LoginRpm.htm?Save=Save HTTP/1.0\r\nHost: 192.168.1.1\r\n"), "{giden}");
        assert!(giden.contains("\r\nCookie: SID=1\r\n") && giden.contains("\r\nReferer: http://192.168.1.1/\r\n") && giden.contains("\r\nContent-Length: 7\r\n"));
        assert!(giden.contains("\r\nAccept-Encoding: identity\r\nConnection: close\r\n") && giden.ends_with("\r\n\r\na=1&b=2"));
        assert!(!giden.contains("wca") && !giden.contains("X-WFC") && !giden.contains("X-Forwarded") && !giden.contains("_;"));
        assert_eq!(giden.matches("Host:").count(), 1, "yalnızca AP'nin kendi adresi");
        assert_eq!(c.durum, 200);
        assert_eq!(deger(&c, "Set-Cookie"), ["SID=1; Path=/"]);
        assert_eq!(deger(&c, "Cache-Control"), ["no-store"]);
        assert_eq!(deger(&c, "X-Frame-Options"), ["SAMEORIGIN"], "AP'nin kendi çerçeve kuralı geçer");
        assert!(deger(&c, "ETag").is_empty() && deger(&c, "Connection").is_empty() && deger(&c, "Content-Length").is_empty());
        let beklenen = "<html><a href=\"https://ap.wificorrect.com/b.htm\">b</a><script src=\"//ap.wificorrect.com/a.js\"></script>192.168.1.1</html>";
        assert_eq!(c.boy, Some(beklenen.len() as u64));
        assert_eq!(String::from_utf8(govde(c)).unwrap(), beklenen);
        assert_eq!(*degisti.lock().unwrap(), None);
    }

    #[test]
    fn aktarim_yonlendirme_ikili_parcali() {
        let h = hedef("192.168.1.1", "misafir");
        // http → https yönlendirmesi: hedef güncellenir, Location ap.wificorrect.com'a
        let (adres, _) = sahte_ap(b"HTTP/1.0 302 Found\r\nLocation: https://192.168.1.1/webpages/login.html\r\n\r\n".to_vec());
        let degisti = Mutex::new(None);
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|s: &str, p: u16| *degisti.lock().unwrap() = Some((s.to_string(), p))).unwrap();
        assert_eq!((c.durum, deger(&c, "Location")), (302, vec!["https://ap.wificorrect.com/webpages/login.html"]));
        assert_eq!(*degisti.lock().unwrap(), Some(("https".to_string(), 443)));
        // ters yön: HTTPS'i yalnızca yönlendiren AP (https → http://<hedef>/): hedef http/80'e döner
        let hs = Hedef { sema: "https".into(), port: 443, ..h.clone() };
        let (adres, _) = sahte_ap(b"HTTP/1.0 301 Moved\r\nLocation: http://192.168.1.1/index.htm\r\n\r\n".to_vec());
        let c = aktar(&hs, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|s: &str, p: u16| *degisti.lock().unwrap() = Some((s.to_string(), p))).unwrap();
        assert_eq!((c.durum, deger(&c, "Location")), (301, vec!["https://ap.wificorrect.com/index.htm"]));
        assert_eq!(*degisti.lock().unwrap(), Some(("http".to_string(), 80)));
        // aynı şema ve port: güncelleme yok
        *degisti.lock().unwrap() = None;
        let (adres, _) = sahte_ap(b"HTTP/1.0 302 Found\r\nLocation: https://192.168.1.1/login\r\n\r\n".to_vec());
        aktar(&hs, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|s: &str, p: u16| *degisti.lock().unwrap() = Some((s.to_string(), p))).unwrap();
        assert_eq!(*degisti.lock().unwrap(), None);
        // ikili içerik (uzunluksuz, bağlantı kapanınca biter) olduğu gibi akar
        let mut ikili = b"HTTP/1.0 200 OK\r\nContent-Type: image/png\r\n\r\n".to_vec();
        let resim: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).chain(b"http://192.168.1.1/".iter().copied()).collect();
        ikili.extend(&resim);
        let (adres, _) = sahte_ap(ikili);
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/a.png", &[], None, &mut io::empty(), &*baglanti(adres), &|_: &str, _: u16| {}).unwrap();
        assert_eq!(c.boy, None);
        assert_eq!(govde(c), resim);
        // parçalı cevap (HTTP/1.0 isteğe rağmen) çözülür; sıkıştırılmış metin yeniden yazılmaz
        let (adres, _) = sahte_ap(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/html\r\n\r\n13\r\nhttp://192.168.1.1/\r\n0\r\n\r\n".to_vec());
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|_: &str, _: u16| {}).unwrap();
        assert_eq!(String::from_utf8(govde(c)).unwrap(), "https://ap.wificorrect.com/");
        // akan gövdenin boyu bildirilmez (final review M3): AP eksik gönderse de cevap parçalı ve kendi içinde tutarlı
        let (adres, _) = sahte_ap(b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\nContent-Length: 19\r\nService-Worker-Allowed: /\r\n\r\nhttp://192.168.1.1/".to_vec());
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|_: &str, _: u16| {}).unwrap();
        assert!(deger(&c, "Service-Worker-Allowed").is_empty(), "SW kapsamı genişletilemez");
        assert_eq!((c.boy, govde(c)), (None, b"http://192.168.1.1/".to_vec()));
        let (adres, _) = sahte_ap(b"HTTP/1.0 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 50\r\n\r\nkisa".to_vec());
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/a.gif", &[], None, &mut io::empty(), &*baglanti(adres), &|_: &str, _: u16| {}).unwrap();
        assert_eq!((c.boy, govde(c)), (None, b"kisa".to_vec()));
        // HEAD: gövde yok, uzunluk korunur
        let (adres, gelen) = sahte_ap(b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1234\r\n\r\n".to_vec());
        let c = aktar(&h, "ap.wificorrect.com", "HEAD", "/", &[], None, &mut io::empty(), &*baglanti(adres), &|_: &str, _: u16| {}).unwrap();
        assert_eq!(c.boy, Some(1234));
        assert!(govde(c).is_empty() && !String::from_utf8(gelen.lock().unwrap().clone()).unwrap().contains("Content-Length"));
    }

    #[test]
    fn aktarim_retleri() {
        let h = hedef("192.168.1.1", "yan_oda");
        let hic: Box<Baglanti> = Box::new(|_: &Hedef| -> io::Result<Box<dyn Akis>> { panic!("bağlanılmamalı") });
        let yok = |_: &str, _: u16| {};
        for (yontem, yol) in [("CONNECT", "/"), ("get", "/"), ("GET", "http://evil/"), ("GET", "/a b"), ("TRACE", "/")] {
            assert!(matches!(aktar(&h, "ap.wificorrect.com", yontem, yol, &[], None, &mut io::empty(), &*hic, &yok), Err(Hata::Gecersiz(_))), "{yontem} {yol}");
        }
        assert_eq!(aktar(&h, "ap.wificorrect.com", "POST", "/", &[], Some(GOVDE_EN_COK + 1), &mut io::empty(), &*hic, &yok).err(), Some(Hata::Buyuk));
        let mut cok = io::repeat(b'a').take(GOVDE_EN_COK + 2);
        assert_eq!(aktar(&h, "ap.wificorrect.com", "POST", "/", &[], None, &mut cok, &*hic, &yok).err(), Some(Hata::Buyuk));
        let red: Box<Baglanti> = Box::new(|_: &Hedef| Err(io::Error::from(io::ErrorKind::ConnectionRefused)));
        let e = aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*red, &yok).err().unwrap();
        assert!(matches!(e, Hata::Ulasilamadi(_)) && e.durum() == 502 && e.mesaj().contains("AP'ye ulaşılamadı"));
        let (adres, _) = sahte_ap(b"".to_vec());
        assert!(matches!(aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*baglanti(adres), &yok), Err(Hata::Bozuk(_))));
    }

    fn gecici_dizin(ad: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("wfc-{ad}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Deneme HTTPS sunucusu (AP'nin 443'ü yerine): tiny_http + rustls (TLS 1.2/1.3). Sertifika testte openssl ile üretilir
    /// (depoda gizli anahtar yok); el sıkışma tiny_http'nin kendi iş parçacığında olur. Sunucu düşerse dinleme biter.
    fn tls_sunucusu() -> (tiny_http::Server, std::net::SocketAddr) {
        let d = gecici_dizin("tls");
        let (c, k) = (d.join("c.pem"), d.join("k.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "1", "-subj", "/CN=ap-deneme", "-keyout"])
            .arg(&k)
            .arg("-out")
            .arg(&c)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "openssl ile deneme sertifikası üretilemedi");
        let ssl = tiny_http::SslConfig { certificate: std::fs::read(&c).unwrap(), private_key: std::fs::read(&k).unwrap() };
        let _ = std::fs::remove_dir_all(&d);
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        (tiny_http::Server::https(("127.0.0.1", port), ssl).unwrap(), ([127, 0, 0, 1], port).into())
    }

    /// Sahte 443 (aktarımın TLS'iyle konuşamayan cihaz): her bağlantıda ClientHello'yu okur, sonra `cevap`ı yazar ve
    /// istemci kapatana kadar bekler; boş cevapta hemen kapatır, `None`'da hiç cevap vermez.
    fn sahte_443(cevap: Option<Vec<u8>>) -> std::net::SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let adres = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let cevap = cevap.clone();
                std::thread::spawn(move || {
                    let mut s = s;
                    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                    let _ = s.read(&mut [0u8; 4096]);
                    match cevap {
                        Some(c) if c.is_empty() => {}
                        Some(c) => {
                            let _ = s.write_all(&c);
                            let _ = s.read_to_end(&mut vec![]);
                        }
                        None => {
                            let _ = s.read_to_end(&mut vec![]);
                        }
                    }
                });
            }
        });
        adres
    }

    /// Sahte 443: ClientHello'dan sonra 16.000 baytlık bir kaydı 100 ms'de bir bayt yollar (hiç bitmeyen el sıkışma; her
    /// okuma süre dolmadan veri alır)
    fn sahte_443_damla() -> std::net::SocketAddr {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let adres = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut s = s;
                    let _ = s.read(&mut [0u8; 4096]);
                    let mut ok = s.write_all(&[0x16, 0x03, 0x03, 0x3e, 0x80]).is_ok();
                    for _ in 0..200 {
                        if !ok {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                        ok = s.write_all(&[0]).is_ok();
                    }
                });
            }
        });
        adres
    }

    /// Yalnızca TLS 1.0 bilen eski AP'nin ServerHello'su (TLS_RSA_WITH_AES_128_CBC_SHA): rustls TLS 1.2+ ister
    fn tls10_server_hello() -> Vec<u8> {
        let mut govde = vec![0x03, 0x01];
        govde.extend([0x5a; 32]);
        govde.extend([0x00, 0x00, 0x2f, 0x00]); // oturum kimliği yok, şifre takımı, sıkıştırma yok
        let mut el = vec![0x02, 0x00, 0x00, govde.len() as u8];
        el.extend(govde);
        let mut kayit = vec![0x16, 0x03, 0x01, 0x00, el.len() as u8];
        kayit.extend(el);
        kayit
    }

    #[test]
    fn tls_el_sikismasi_aktarim_ayariyla() {
        let (_sunucu, iyi) = tls_sunucusu();
        let dene_sure = |a: std::net::SocketAddr, sure: Duration| tls_el_sikis(TcpStream::connect(a).unwrap(), sure);
        let sonuc = dene_sure(iyi, TLS_DENEME);
        assert!(sonuc.is_ok(), "{sonuc:?}");
        // başarısızlar çabuk biter; yalnızca hiç cevap vermeyen süreyi bekler (kısa süreyle)
        let dene = |a: std::net::SocketAddr| dene_sure(a, Duration::from_millis(800));
        // TCP'si açık ama aktarımın konuşamadığı 443'ler: yalnızca TLS 1.0, 443'te düz HTTP, TLS uyarısı (protocol_version),
        // hemen kapatan, hiç cevap vermeyen (süre sınırında biter)
        for (ad, cevap) in [
            ("tls1.0", Some(tls10_server_hello())),
            ("düz http", Some(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec())),
            ("uyarı", Some(vec![0x15, 0x03, 0x01, 0x00, 0x02, 0x02, 0x46])),
            ("kapatan", Some(vec![])),
            ("sessiz", None),
        ] {
            let a = sahte_443(cevap);
            let t = std::time::Instant::now();
            assert!(dene(a).is_err(), "{ad}");
            assert!(t.elapsed() < Duration::from_secs(3), "{ad}: {:?}", t.elapsed());
        }
        // süre bütün el sıkışmaya: damla damla cevap veren cihaz süreyi uzatamaz
        let t = std::time::Instant::now();
        let e = dene(sahte_443_damla()).unwrap_err();
        // son okuma süresi dolarsa Linux EAGAIN (WouldBlock) verir, döngü başı denetimi TimedOut
        assert!(matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) && t.elapsed() < Duration::from_secs(3), "{e:?} {:?}", t.elapsed());
    }

    #[test]
    fn web_arayuzu_https_ancak_tls_calisirsa() {
        // Canlı ZyXEL (2026-10-18): sayfa https://ap.wificorrect.com'da açıldığı için AP'nin betiği istekleri şifrelemez;
        // AP'ye 80'den gidilirse AP şifreli istek bekler, giriş döner. 443'te aktarımın TLS'iyle el sıkışma olursa HTTPS
        // (80 açık olsa da); olmazsa (eski TLS) 80. Yalnızca TCP'si açık 443 HTTPS sayılmaz.
        let (_sunucu, tls) = tls_sunucusu();
        let eski = sahte_443(Some(tls10_server_hello()));
        let duz = TcpListener::bind("127.0.0.1:0").unwrap(); // 80: TCP açık yeter
        let http = duz.local_addr().unwrap();
        let kapali = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let sec = |p443: std::net::SocketAddr, p80: std::net::SocketAddr| {
            web_bul(&|p: u16| TcpStream::connect(if p == 443 { p443 } else { p80 }), TLS_DENEME)
        };
        assert_eq!(sec(tls, http), Some(443), "TLS el sıkışması başarılı: HTTPS");
        assert_eq!(sec(eski, http), Some(80), "443 açık ama TLS 1.0: HTTP");
        assert_eq!(sec(kapali, http), Some(80), "yalnızca 80");
        assert_eq!(sec(tls, kapali), Some(443), "yalnızca 443, TLS çalışıyor");
        assert_eq!(sec(eski, kapali), None, "aktarımın konuşamadığı 443 tek başına web arayüzü değil");
        assert_eq!(sec(kapali, kapali), None);
        // kural: önce 443 (TLS); başarılıysa 80 hiç denenmez
        let sira = std::cell::RefCell::new(vec![]);
        assert_eq!(web_sec(|| { sira.borrow_mut().push(443); true }, || { sira.borrow_mut().push(80); true }), Some(443));
        assert_eq!(web_sec(|| { sira.borrow_mut().push(443); false }, || { sira.borrow_mut().push(80); true }), Some(80));
        assert_eq!(*sira.borrow(), [443, 443, 80]);
        drop(duz);
    }

    #[test]
    fn https_aktarimi_gercek_tls() {
        // HTTPS hedefe aktarım gerçek TLS üzerinden: istek gider, cevap (ZyXEL'in HTTPS'teki JSON'u gibi) yeniden yazılır
        let (sunucu, a) = tls_sunucusu();
        let t = std::thread::spawn(move || {
            let rq = sunucu.recv().unwrap();
            let govde = format!("{{\"RSAPublicKey\":\"None\",\"yol\":\"{}\",\"u\":\"https://127.0.0.1/x\"}}", rq.url());
            let tur = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            rq.respond(tiny_http::Response::from_string(govde).with_header(tur)).unwrap();
        });
        let mut h = hedef("127.0.0.1", "yan_oda");
        (h.sema, h.port) = ("https".into(), a.port());
        let b: Box<Baglanti> = Box::new(move |_: &Hedef| Ok(Box::new(tls(TcpStream::connect(a)?)?) as Box<dyn Akis>));
        let c = aktar(&h, "ap.wificorrect.com", "GET", "/getRSAPublickKey", &[], None, &mut io::empty(), &*b, &|_: &str, _: u16| {}).unwrap();
        assert_eq!(c.durum, 200);
        assert_eq!(String::from_utf8(govde(c)).unwrap(), "{\"RSAPublicKey\":\"None\",\"yol\":\"/getRSAPublickKey\",\"u\":\"https://ap.wificorrect.com/x\"}");
        t.join().unwrap();
    }

    #[test]
    fn tarayici_istek_reddi() {
        // final review C1: Service Worker betiği (kayıt ya da güncelleme) hiçbir koşulda AP'ye gitmez
        assert_eq!(istek_reddi("GET", &basliklar(&[("Service-Worker", "script")])), Some("service_worker"));
        assert_eq!(istek_reddi("GET", &basliklar(&[("service-worker", "")])), Some("service_worker"));
        assert_eq!(istek_reddi("GET", &basliklar(&[("Sec-Fetch-Site", "same-origin"), ("Sec-Fetch-Dest", "serviceworker")])), Some("service_worker"));
        // final review I2: başka siteden hiçbir istek
        let g = |site: &str, mod_: &str, hedef: &str| basliklar(&[("Sec-Fetch-Site", site), ("Sec-Fetch-Mode", mod_), ("Sec-Fetch-Dest", hedef)]);
        assert_eq!(istek_reddi("GET", &g("cross-site", "navigate", "document")), Some("baska_site"));
        assert_eq!(istek_reddi("GET", &g(" Cross-Site ", "navigate", "document")), Some("baska_site"));
        // aynı sitenin başka alt alanından yalnızca üst düzey GET/HEAD gezinme (cihaz panelinden gelen yönlendirme zinciri)
        assert_eq!((istek_reddi("GET", &g("same-site", "navigate", "document")), istek_reddi("HEAD", &g("same-site", "navigate", "document"))), (None, None));
        for (y, m, d) in [("POST", "navigate", "document"), ("GET", "navigate", "iframe"), ("GET", "no-cors", "image"), ("GET", "cors", "empty")] {
            assert_eq!(istek_reddi(y, &g("same-site", m, d)), Some("alt_alan"), "{y} {m} {d}");
        }
        // AP'nin kendi sayfaları (aynı köken), elle yazılan adres ve başlık göndermeyen eski tarayıcı geçer
        for s in ["same-origin", "none"] {
            assert_eq!(istek_reddi("POST", &g(s, "cors", "empty")), None, "{s}");
        }
        assert_eq!(istek_reddi("POST", &[]), None);
        assert!(red_mesaji("service_worker").contains("Service Worker") && red_mesaji("baska_site").contains("cihaz panelinden"));
    }

    #[test]
    fn kayitli_ap_mac_denetimi() {
        // final review I1: misafir ağındaki kayıtlı AP adresinde başka MAC (adresi elle alan cihaz) → istek gitmez
        let arp = "IP address       HW type     Flags       HW address            Mask     Device\n\
                   10.50.0.2        0x1         0x2         70:4f:57:dd:7a:e8     *        br-hotspot\n";
        let ip: Ipv4Addr = "10.50.0.2".parse().unwrap();
        let mac_hatasi = |r: io::Result<()>| r.unwrap_err().get_ref().and_then(|x| x.downcast_ref::<MacFarkli>()).map(|m| (m.kayitli.clone(), m.simdi.clone()));
        let mut h = hedef("10.50.0.2", "misafir");
        h.mac = Some("70:4f:57:dd:7a:e8".into());
        assert!(mac_denetle(&h, ip, arp).is_ok());
        h.mac = Some("aa:bb:cc:dd:ee:ff".into());
        assert_eq!(mac_hatasi(mac_denetle(&h, ip, arp)), Some(("aa:bb:cc:dd:ee:ff".into(), Some("70:4f:57:dd:7a:e8".into()))));
        assert_eq!(mac_hatasi(mac_denetle(&h, ip, "")), Some(("aa:bb:cc:dd:ee:ff".into(), None)), "okunamayan MAC geçmez");
        h.mac = None;
        assert!(mac_denetle(&h, ip, arp).is_err(), "MAC'i sabitlenmemiş hedef geçmez");
        // aktarımda: 403 "AP'nin MAC adresi değişmiş" (bağlantı kurulmuş olsa da istek yazılmadan)
        let farkli: Box<Baglanti> =
            Box::new(|_: &Hedef| Err(io::Error::other(MacFarkli { kayitli: "aa:bb:cc:dd:ee:ff".into(), simdi: Some("11:22:33:44:55:66".into()) })));
        let e = aktar(&h, "ap.wificorrect.com", "GET", "/", &[], None, &mut io::empty(), &*farkli, &|_: &str, _: u16| {}).err().unwrap();
        assert_eq!(e, Hata::MacDegisti("aa:bb:cc:dd:ee:ff".into(), Some("11:22:33:44:55:66".into())));
        assert!(e.durum() == 403 && e.baslik() == "AP'nin MAC adresi değişmiş" && e.mesaj().contains("11:22:33:44:55:66") && e.mesaj().contains("Kaydet"));
        assert_eq!(Hata::Bozuk("x".into()).baslik(), "AP arayüzü");
    }

    #[test]
    fn hedef_esanli_yazilirken_gecerli_kalir() {
        // final review M1: aynı anda yazan iş parçacıkları (aktarımda şema değişimi, yeni hedef) dosyayı yarım bırakmaz;
        // denetim geçerli hedefi hiçbir an kaybetmez
        let (mut cfg, y) = cfg_ve_yollar();
        cfg.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        let mut m = hedef("10.50.0.2", "misafir");
        (m.mac, m.bitis) = (Some("70:4f:57:dd:7a:e8".into()), 9e9);
        yaz(&y, &m).unwrap();
        std::thread::scope(|t| {
            for i in 0..4 {
                let (y, m) = (&y, &m);
                t.spawn(move || {
                    for j in 0..50 {
                        if (i + j) % 2 == 0 { yaz(y, m).unwrap() } else { sema_guncelle(y, m, "http", 80) }
                    }
                });
            }
            for _ in 0..200 {
                assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Gecerli(m.clone()));
            }
        });
    }

    fn cfg_ve_yollar() -> (Config, Yollar) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-aparayuz-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = Config::parse(&format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n[ap]\nadresler = ['10.50.0.2']\n", root.display())).unwrap();
        (cfg, Yollar::kok(&root))
    }

    #[test]
    fn hedef_dogrulama_ve_denetim() {
        let (mut cfg, y) = cfg_ve_yollar();
        let g = ap::Gecici { adres: "192.168.1.250/24".into(), kimlik: "0a1b2c3d".into(), bitis: 5000.0 };
        assert_eq!(hedef_dogrula("192.168.1.1", &cfg, Some(&g)).unwrap(), ("192.168.1.1".parse().unwrap(), Yol::YanOda("0a1b2c3d".into(), 5000.0)));
        assert_eq!(hedef_dogrula(" 10.50.0.2 ", &cfg, None).unwrap().1, Yol::Misafir);
        // misafir telefonu (havuz), cihazın kendisi, kayıtsız adres, yan oda dışı, WAN/tünel, yan odanın kendisi: hayır
        for (a, yo) in [("10.50.0.30", Some(&g)), ("10.50.0.1", Some(&g)), ("10.50.0.3", Some(&g)), ("192.168.2.1", Some(&g)),
                        ("10.99.0.1", Some(&g)), ("192.168.1.250", Some(&g)), ("192.168.1.1", None), ("abc", Some(&g)), ("192.168.1.255", Some(&g))] {
            assert!(hedef_dogrula(a, &cfg, yo).is_err(), "{a}");
        }
        // denetim: geçerli, süre, yan oda değişti, kayıt silindi, bozuk
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Yok);
        let mut h = hedef("192.168.1.1", "yan_oda");
        h.yan_oda = Some("0a1b2c3d".into());
        yaz(&y, &h).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&y.hedef).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(matches!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(_, "yan_oda")), "yan oda yokken geçmez ve silinir");
        assert!(!y.hedef.exists());
        ortak::write_atomic(&y.gecici, &serde_json::to_vec(&g).unwrap()).unwrap();
        yaz(&y, &h).unwrap();
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Gecerli(h.clone()));
        assert_eq!(denetle(&y, &cfg, 2000.0), Durum::Kapandi(h.clone(), "sure"));
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Yok);
        yaz(&y, &Hedef { yan_oda: Some("ffffffff".into()), ..h.clone() }).unwrap();
        assert!(matches!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(_, "yan_oda")));
        // misafir ağı: kayıtlı adres + kayıtta sabitlenen MAC (final review I1)
        let mut m = hedef("10.50.0.2", "misafir");
        m.mac = Some("70:4f:57:dd:7a:e8".into());
        yaz(&y, &m).unwrap();
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(m.clone(), "adres_kaydi"), "MAC'i sabitlenmemiş kayıt geçmez");
        cfg.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        yaz(&y, &m).unwrap();
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Gecerli(m.clone()));
        cfg.ap.mac.insert("10.50.0.2".into(), "aa:bb:cc:dd:ee:ff".into()); // MAC yeniden sabitlendi: açık erişim kapanır
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(m.clone(), "adres_kaydi"));
        cfg.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        yaz(&y, &Hedef { mac: None, ..m.clone() }).unwrap(); // MAC'siz misafir hedefi geçmez
        assert!(matches!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(_, "adres_kaydi")));
        yaz(&y, &m).unwrap();
        cfg.ap.adresler.clear();
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Kapandi(m.clone(), "adres_kaydi"));
        cfg.ap.adresler = vec!["10.50.0.2".into()];
        // okunamayan dosya: Yok, ama silinmez (final review M1: yazılırken okunan dosya geçerli hedefi sildirmesin)
        std::fs::write(&y.hedef, "bozuk").unwrap();
        assert_eq!(denetle(&y, &cfg, 1000.0), Durum::Yok);
        assert!(y.hedef.exists());
        // kapatma: yalnızca yan oda hedefi istenirse misafir hedefine dokunulmaz
        yaz(&y, &m).unwrap();
        assert_eq!(kapat(&y, true), None);
        assert_eq!(kapat(&y, false), Some(m.clone()));
        assert_eq!(kapat(&y, false), None);
        // şema/port güncellemesi yalnızca aynı hedefe; kapat_eger yalnızca dosyadaki hedef tam olarak buysa
        yaz(&y, &m).unwrap();
        sema_guncelle(&y, &Hedef { bitis: 1.0, ..m.clone() }, "https", 443);
        assert_eq!(oku(&y), Some(m.clone()));
        sema_guncelle(&y, &m, "https", 443);
        let g = oku(&y).unwrap();
        assert_eq!((g.sema.as_str(), g.port), ("https", 443));
        // ters yön (https → http)
        sema_guncelle(&y, &g, "http", 80);
        assert_eq!(oku(&y).map(|x| (x.sema, x.port)), Some(("http".to_string(), 80)));
        sema_guncelle(&y, &g, "https", 443);
        assert!(!kapat_eger(&y, &m) && y.hedef.exists());
        assert!(kapat_eger(&y, &g) && !y.hedef.exists());
        assert!(!kapat_eger(&y, &g));
        assert_eq!(konaklar(&hedef("192.168.0.254", "yan_oda")), k(&["192.168.0.254", "tplinkap.net", "tplinkrepeater.net", "tplinkeap.net"]));
    }
}
