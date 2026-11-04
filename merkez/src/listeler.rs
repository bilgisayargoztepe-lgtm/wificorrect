//! Hazır engel listeleri (Python listeler.py): kaynakları indirir, temizler, kategori başına tek alan adı listesi üretir
//! (`<k>.txt.gz` + `<k>.json`); cihaz API'si için okuma tarafı (`liste_yolu`, `ozet`, `meta_oku`) da burada.
//!
//! Çıktı (DIZIN): `<kategori>.txt.gz` (sıralı, satır başına bir alan adı, gzip mtime=0 → aynı içerik aynı dosya) ve
//! `<kategori>.json` (meta). sha256 gzip'in değil düz metnin özetidir (cihaz açıp doğrular). İsteğe bağlı elle ek:
//! `<kategori>.ek.txt`. Kaynak başına son başarılı temiz kopya `kaynak/<kategori>-<url özeti>.txt.gz`: bir kaynak
//! inmezse ya da boş/HTML dönerse onun dünkü kopyası kullanılır; hiçbir kaynak bugün inmediyse eski çıktı ve meta
//! korunur, meta'ya `hata` yazılır. Engelleme dnsmasq'ta alan adı + bütün alt adlarıdır; üst adı listede olan alt adlar
//! atılır.
use std::collections::{BTreeSet, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::ortak::{py_bol, py_bosluk, py_dogru, py_str, py_strip, Tasiyici};
use crate::veri::Veri;

pub const DIZIN: &str = "/var/lib/wificorrect/merkez/listeler";
/// Kategori başına; fazlası kesilir (önce elle ek, sonra kaynak sırası öncelikli).
pub const EN_COK: usize = 300_000;
pub const KAYNAK_SINIRI: usize = 64 * 1024 * 1024;
/// Python `KATEGORILER` (sıra korunur: sağlık cevabındaki `listeler` bu sırayla yazılır).
pub const KATEGORILER: [(&str, &str); 3] =
    [("kumar", "Kumar ve bahis"), ("yetiskin", "Yetişkin içerik"), ("zararli", "Zararlı ve dolandırıcı siteler")];
const EK_AD: &str = "Elle eklenenler";
/// Küçük liste tabanı: önceki kopya `TABAN_ESIK`'ten büyükken yeni liste onun `TABAN_ORAN`ından küçükse hata.
const TABAN_ESIK: usize = 1000;
const TABAN_ORAN: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bicim {
    Hosts,
    Duz,
    Adblock,
    Usom,
}

#[derive(Clone, Copy, Debug)]
pub struct Kaynak {
    pub ad: &'static str,
    pub url: &'static str,
    pub bicim: Bicim,
}

// Kaynaklar (adresler 2026-10-14'te doğrulandı). Lisanslar:
// - HaGeZi DNS Blocklists (GPL-3.0). "wildcard/*-onlydomains": satır başına alan adı, alt adları kapsar. "doh-vpn-proxy-bypass"
//   listesi BİLEREK kullanılmaz (iş VPN'lerini kırar; kullanıcı kararı 2026-10-14).
// - StevenBlack/hosts "porn" eki (MIT; eki oluşturan kaynakların kendi lisansları var).
// - USOM zararlı bağlantılar (kamuya açık). 2026-10-14'te adres siberguvenlik.gov.tr'ye yönleniyor; HTML dönerse kaynak
//   hatalı sayılır ve son başarılı kopyası kullanılır.
pub fn varsayilan_kaynaklar() -> Vec<(&'static str, Vec<Kaynak>)> {
    let k = |ad, url, bicim| Kaynak { ad, url, bicim };
    vec![
        (
            "kumar",
            vec![k(
                "HaGeZi Gambling (medium)",
                "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/gambling.medium-onlydomains.txt",
                Bicim::Duz,
            )],
        ),
        (
            "yetiskin",
            vec![
                k("HaGeZi NSFW", "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/nsfw-onlydomains.txt", Bicim::Duz),
                k(
                    "StevenBlack porn",
                    "https://raw.githubusercontent.com/StevenBlack/hosts/master/alternates/porn-only/hosts",
                    Bicim::Hosts,
                ),
            ],
        ),
        (
            "zararli",
            vec![
                k("USOM zararlı bağlantılar", "https://www.usom.gov.tr/url-list.txt", Bicim::Usom),
                k(
                    "HaGeZi Threat Intelligence (mini)",
                    "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/tif.mini-onlydomains.txt",
                    Bicim::Duz,
                ),
            ],
        ),
    ]
}

fn kume(a: &[&'static str]) -> HashSet<&'static str> {
    a.iter().copied().collect()
}

/// Asla engellenmez (ve bütün alt adları): sistemin, giriş akışının, telefonların bağlantı denetiminin ve kamu/eğitim
/// sitelerinin ihtiyacı. Bir kaynak listelese de çıkar.
static ASLA: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    kume(&[
        "wificorrect.com", "netgsm.com.tr", "twilio.com", "google.com", "googleapis.com", "gstatic.com", "android.com",
        "apple.com", "icloud.com", "microsoft.com", "windowsupdate.com", "msftconnecttest.com", "msftncsi.com",
        "connectivitycheck.gstatic.com", "captive.apple.com", "detectportal.firefox.com", "pool.ntp.org", "ntp.org",
        "gov.tr", "edu.tr",
        // büyük ortak altyapılar: bir alt adı engellemek bile milyonlarca meşru kullanıcıyı ve sayfayı bozar
        "googleusercontent.com", "azureedge.net", "windows.net", "cloudflare.com", "trycloudflare.com", "facebook.com",
        "fbcdn.net", "instagram.com", "youtube.com", "ytimg.com", "akamaihd.net", "live.com", "office.com", "outlook.com",
        "whatsapp.com", "whatsapp.net", "wa.me",
        // telefon üreticileri, sosyal medya, alışveriş, ödeme ve bankalar: yanlış pozitif misafiri doğrudan etkiler
        "hicloud.com", "miui.com", "xiaomi.com", "oppomobile.com", "samsung.com", "samsungcloud.com", "x.com", "twitter.com",
        "tiktok.com", "amazon.com", "trendyol.com", "hepsiburada.com", "papara.com", "iyzico.com", "garantibbva.com.tr",
        "isbank.com.tr", "ziraatbank.com.tr", "akbank.com", "yapikredi.com.tr", "vakifbank.com.tr", "halkbank.com.tr",
        "qnb.com.tr", "denizbank.com", "enpara.com", "turkiye.gov.tr",
    ])
});

/// Paylaşımlı platformlar: zararlı listelerde bir kullanıcının dosyası yüzünden bütün platform listelenebilir. Bunlar ve
/// alt adları engellenmez.
static PAYLASIMLI: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    kume(&[
        "github.com", "githubusercontent.com", "discordapp.com", "discord.com", "discord.gg", "dropbox.com",
        "dropboxusercontent.com", "bit.ly", "t.me", "telegram.org", "ipfs.io", "mediafire.com", "pastebin.com",
        "onedrive.live.com", "1drv.ms", "sharepoint.com", "wetransfer.com", "mega.nz", "tinyurl.com",
    ])
});

/// Aşırı geniş girdiler — yalnızca KENDİSİ atılır, alt adları listelenebilir: kamu son ekleri ve kullanıcıya alt ad veren
/// barındırma platformları (kotu.github.io listelenebilir, github.io değil).
static KAMU_SON_EK: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    kume(&[
        "com", "net", "org", "com.tr", "gov.tr", "edu.tr", "bel.tr", "k12.tr", "org.tr", "net.tr", "gen.tr", "av.tr",
        "tv.tr", "info.tr", "biz.tr", "web.tr", "name.tr", "tel.tr", "pol.tr", "tsk.tr", "dr.tr", "bbs.tr", "nc.tr", "kep.tr",
        "co.uk", "org.uk", "ac.uk", "gov.uk", "com.br", "com.au", "net.au", "org.au", "co.jp", "ne.jp", "co.in", "co.za",
        "com.cn", "com.mx", "co.kr", "com.ar", "co.nz", "com.sg", "com.ua", "com.pl", "co.id", "com.hk", "com.tw", "eu.org",
        "blogspot.com", "github.io", "gitlab.io", "wordpress.com", "weebly.com", "wixsite.com", "firebaseapp.com", "web.app",
        "pages.dev", "workers.dev", "vercel.app", "netlify.app", "herokuapp.com", "azurewebsites.net", "cloudfront.net",
        "amazonaws.com", "s3.amazonaws.com", "appspot.com", "glitch.me", "repl.co", "ngrok.io", "ngrok-free.app",
        "duckdns.org", "no-ip.com", "ddns.net", "000webhostapp.com", "r2.dev", "onrender.com", "fly.dev", "surge.sh",
    ])
});

/// VPN/proxy/DoH: VPN kategorisi yok ve iş VPN'leri çalışmalı; tehdit listelerinde geçse de hiçbir listeden gelmez.
/// Bilinen sağlayıcıların alan adları (ve alt adları) + ilk etiketi tam olarak vpn/doh/dot/dns/proxy.
static VPN_DOH: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    kume(&[
        "nordvpn.com", "nordcdn.com", "expressvpn.com", "protonvpn.com", "proton.me", "surfshark.com", "cyberghostvpn.com",
        "privateinternetaccess.com", "mullvad.net", "windscribe.com", "tunnelbear.com", "hotspotshield.com", "ipvanish.com",
        "vyprvpn.com", "purevpn.com", "hide.me", "psiphon.ca", "openvpn.net", "wireguard.com", "tailscale.com", "zerotier.com",
        "cloudflare-dns.com", "cloudflareclient.com", "one.one.one.one", "dns.google", "quad9.net", "nextdns.io",
        "adguard-dns.com", "adguard-dns.io", "opendns.com", "dns.sb", "controld.com", "mullvad-dns.net",
    ])
});
const VPN_ETIKET: [&str; 5] = ["vpn", "doh", "dot", "dns", "proxy"];
const ATLA: [&str; 5] = ["localhost.localdomain", "ip6-localhost", "ip6-loopback", "local", "broadcasthost"];

/// ad kümedeki bir adın kendisi ya da alt adı mı.
fn ust_mu(ad: &str, k: &HashSet<&'static str>) -> bool {
    let mut p = ad;
    loop {
        if k.contains(p) {
            return true;
        }
        match p.split_once('.') {
            Some((_, kalan)) => p = kalan,
            None => return false,
        }
    }
}

fn etiket_gecerli(x: &str) -> bool {
    (1..=63).contains(&x.len()) && x.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Normalize edilmiş, engellenebilir alan adı ya da None.
pub fn gecerli(ad: &str) -> Option<String> {
    let ad = py_strip(ad).to_lowercase();
    let ad = ad.trim_end_matches('.');
    if ad.is_empty() || ad.chars().count() > 253 || ATLA.contains(&ad) {
        return None;
    }
    let p: Vec<&str> = ad.split('.').collect();
    if p.len() < 2 || !p.iter().all(|x| etiket_gecerli(x)) {
        return None;
    }
    if p.iter().all(|x| x.bytes().all(|b| b.is_ascii_digit())) {
        return None; // IP adresi
    }
    if KAMU_SON_EK.contains(ad) || ust_mu(ad, &ASLA) || ust_mu(ad, &PAYLASIMLI) || ust_mu(ad, &VPN_DOH) || VPN_ETIKET.contains(&p[0]) {
        return None;
    }
    Some(ad.to_string())
}

/// USOM satırından ana bilgisayar adı; yol ya da sorgu içeren URL'ler için None. Bir URL çoğu kez paylaşımlı bir sitedeki
/// tek dosyayı gösterir; bütün siteyi engellemek yanlış olur. Yalnızca çıplak ana bilgisayar girdileri alınır.
fn usom_ad(s: &str) -> Option<String> {
    let s = s.split_once("://").map_or(s, |(_, k)| k);
    let i = s.find(['/', '?', '#']).unwrap_or(s.len());
    if !matches!(&s[i..], "" | "/") {
        return None;
    }
    let s = s[..i].rsplit('@').next().unwrap_or("");
    let s = s.split(':').next().unwrap_or("");
    Some(s.to_string())
}

/// Python `_ADBLOCK.fullmatch` (`\|\|([^\^/$|]+)\^(\$.*)?`): `||ad^` ya da `||ad^$seçenekler`.
fn adblock_ad(s: &str) -> Option<&str> {
    let g = s.strip_prefix("||")?;
    let n = g.find(['^', '/', '$', '|'])?;
    if n == 0 || !g[n..].starts_with('^') {
        return None;
    }
    let kalan = &g[n + 1..];
    (kalan.is_empty() || (kalan.starts_with('$') && !kalan.contains('\n'))).then_some(&g[..n])
}

fn adaylar(satir: &str, bicim: Bicim) -> Vec<String> {
    let s = if bicim != Bicim::Adblock { py_strip(satir.split('#').next().unwrap_or("")) } else { py_strip(satir) };
    if s.is_empty() || s.starts_with('!') {
        return vec![];
    }
    match bicim {
        Bicim::Hosts => py_bol(s).into_iter().skip(1).map(String::from).collect(),
        Bicim::Duz => vec![py_bol(s)[0].to_string()],
        Bicim::Adblock => adblock_ad(s).map(|a| vec![a.to_string()]).unwrap_or_default(),
        Bicim::Usom => usom_ad(py_bol(s)[0]).into_iter().collect(),
    }
}

/// Kaynak satırlarından geçerli, engellenebilir alan adı kümesi.
pub fn temizle<'a>(satirlar: impl IntoIterator<Item = &'a str>, bicim: Bicim) -> BTreeSet<String> {
    let mut sonuc = BTreeSet::new();
    for satir in satirlar {
        for a in adaylar(satir, bicim) {
            if let Some(ad) = gecerli(&a) {
                sonuc.insert(ad);
            }
        }
    }
    sonuc
}

pub fn kategori_gecerli(k: &str) -> bool {
    KATEGORILER.iter().any(|(a, _)| *a == k)
}

/// Python `liste_yolu`: bilinmeyen kategori `ValueError("Bilinmeyen kategori.")`.
pub fn liste_yolu(dizin: &Path, kategori: &str) -> Result<PathBuf, String> {
    if !kategori_gecerli(kategori) {
        return Err("Bilinmeyen kategori.".into());
    }
    Ok(dizin.join(format!("{kategori}.txt.gz")))
}

fn meta_yolu(dizin: &Path, kategori: &str) -> PathBuf {
    dizin.join(format!("{kategori}.json"))
}

/// gzip, mtime=0 ve dosya adı yok: aynı içerik aynı dosya.
pub fn gz(metin: &str) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(metin.as_bytes()).expect("bellekte yazım");
    e.finish().expect("bellekte yazım")
}

/// Geçici dosya + yer değiştirme (fsync yok: Python `_atomik`).
fn atomik(yol: &Path, icerik: &[u8]) -> Result<(), String> {
    let hata = |e: std::io::Error| format!("{}: {e}", yol.display());
    if let Some(ust) = yol.parent() {
        std::fs::create_dir_all(ust).map_err(hata)?;
    }
    let mut g = yol.as_os_str().to_owned();
    g.push(".tmp");
    let g = PathBuf::from(g);
    std::fs::write(&g, icerik).map_err(hata)?;
    std::fs::rename(&g, yol).map_err(hata)
}

/// Kategorinin üretim özeti (`<k>.json`); okunamıyor ya da sözlük değilse None.
pub fn meta_oku(dizin: &Path, kategori: &str) -> Option<Map<String, Value>> {
    let ham = std::fs::read(meta_yolu(dizin, kategori)).ok()?;
    match serde_json::from_slice::<Value>(&ham).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn sha_gecerli(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Sağlık cevabı için `{kategori: {sha256, sayi, zaman}}`; meta okunamayan ya da hiç liste üretilmemiş kategori yok.
pub fn ozet(dizin: &Path) -> Value {
    let mut o = Map::new();
    for (k, _) in KATEGORILER {
        let Some(m) = meta_oku(dizin, k) else { continue };
        let sha = m.get("sha256").and_then(Value::as_str).filter(|s| sha_gecerli(s));
        // Python `type(x) is int`: bool ve kayan nokta değil
        let sayi = m.get("sayi").filter(|v| v.is_i64() || v.is_u64());
        let zaman = m.get("zaman").filter(|v| v.is_string());
        if let (Some(sha), Some(sayi), Some(zaman)) = (sha, sayi, zaman) {
            let mut e = Map::new();
            e.insert("sha256".into(), Value::String(sha.to_string()));
            e.insert("sayi".into(), sayi.clone());
            e.insert("zaman".into(), zaman.clone());
            o.insert(k.to_string(), Value::Object(e));
        }
    }
    Value::Object(o)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn onbellek_yolu(dizin: &Path, kategori: &str, url: &str) -> PathBuf {
    let h = hex(&Sha256::digest(url.as_bytes()));
    dizin.join("kaynak").join(format!("{kategori}-{}.txt.gz", &h[..12]))
}

/// Eski sürümün yazdığı kopya bugünkü kurallarla (asla listesi, geçerlilik) yeniden süzülür; okunamıyorsa None.
fn onbellek_oku(yol: &Path) -> Option<BTreeSet<String>> {
    let f = std::fs::File::open(yol).ok()?;
    let mut ham = Vec::new();
    flate2::read::MultiGzDecoder::new(f).read_to_end(&mut ham).ok()?;
    let metin = String::from_utf8(ham).ok()?;
    Some(py_bol(&metin).into_iter().filter_map(gecerli).collect())
}

/// Liste yerine bir web sayfası (yönlendirme, hata ya da giriş sayfası) mı döndü.
pub fn html_mi(ham: &str) -> bool {
    let ham = ham.trim_start_matches('\u{feff}').trim_start_matches(py_bosluk);
    let bas: String = ham.chars().take(4096).collect::<String>().to_lowercase();
    bas.starts_with('<') || bas.contains("<html") || bas.contains("<!doctype")
}

/// Python `str(h) or type(h).__name__` (200 karaktere kırpılır).
fn hata_metni(h: &str) -> String {
    let m = if h.is_empty() { "Exception" } else { h };
    m.chars().take(200).collect()
}

fn durum_satiri(ad: &str, sayi: usize, hata: Option<&str>, eski: bool) -> Value {
    let mut d = Map::new();
    d.insert("ad".into(), json!(ad));
    d.insert("sayi".into(), json!(sayi));
    if let Some(h) = hata {
        d.insert("hata".into(), json!(h));
    }
    if eski {
        d.insert("eski".into(), json!(true));
    }
    Value::Object(d)
}

fn elle_ek(dizin: &Path, kategori: &str) -> Option<BTreeSet<String>> {
    let ham = std::fs::read(dizin.join(format!("{kategori}.ek.txt"))).ok()?;
    let metin = String::from_utf8(ham).ok()?;
    Some(temizle(crate::kuyruk::satirlar(&metin), Bicim::Duz))
}

/// Başka bir güncelleme sürüyor (`bekle=false`).
#[derive(Debug, PartialEq, Eq)]
pub struct Mesgul;

/// Zamanlayıcı ve yönetimdeki "Şimdi güncelle" aynı anda yazmasın (flock). `bekle=false`: kilit tutuluyorsa beklemeden
/// `Mesgul` (web isteği asılı kalmasın). Düşünce (dosya kapanınca) bırakılır.
struct Kilit(#[allow(dead_code)] std::fs::File);

impl Kilit {
    fn al(dizin: &Path, bekle: bool) -> Result<Kilit, Mesgul> {
        use std::os::fd::AsRawFd;
        let _ = std::fs::create_dir_all(dizin);
        let f = std::fs::OpenOptions::new().create(true).append(true).open(dizin.join(".kilit")).map_err(|_| Mesgul)?;
        // SAFETY: fd geçerli ve bize ait
        let k = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | if bekle { 0 } else { libc::LOCK_NB }) };
        if k != 0 {
            return Err(Mesgul);
        }
        Ok(Kilit(f))
    }
}

/// Üretici ayarları: kaynak listesi ve kategori sınırı (testlerde Python'daki `L.KAYNAKLAR`/`L.EN_COK` değişikliğinin yeri).
pub struct Uretici {
    pub kaynaklar: Vec<(&'static str, Vec<Kaynak>)>,
    pub en_cok: usize,
}

impl Uretici {
    pub fn varsayilan() -> Uretici {
        Uretici { kaynaklar: varsayilan_kaynaklar(), en_cok: EN_COK }
    }

    fn kategori_kaynaklari(&self, kategori: &str) -> &[Kaynak] {
        self.kaynaklar.iter().find(|(k, _)| *k == kategori).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    /// (alan adları, durum satırı, bugün indi mi). Hatada son başarılı kopya kullanılır ve kopyanın üzerine yazılmaz.
    /// Taban: önceki kopya `TABAN_ESIK`'ten büyükken yeni liste onun `TABAN_ORAN`ından küçükse (kaynak yarım ya da bozuk
    /// yayımlamış olabilir) hata sayılır. Kaynak gerçekten kalıcı küçüldüyse yönetici `kaynak/` altındaki kopyayı silerek
    /// yeni listeyi kabul ettirir.
    fn kaynak_al(&self, kategori: &str, k: &Kaynak, t: &dyn Tasiyici, dizin: &Path) -> Result<(BTreeSet<String>, Value, bool), String> {
        let yol = onbellek_yolu(dizin, kategori, k.url);
        let eski = onbellek_oku(&yol);
        let deneme = (|| -> Result<BTreeSet<String>, String> {
            let ham = crate::surumler::getir(t, k.url, KAYNAK_SINIRI)?;
            let ham = String::from_utf8_lossy(&ham);
            let ham = ham.trim_start_matches('\u{feff}');
            if html_mi(ham) {
                return Err("liste yerine HTML sayfası döndü".into());
            }
            let alanlar = temizle(crate::kuyruk::satirlar(ham), k.bicim);
            if alanlar.is_empty() {
                return Err("liste boş".into());
            }
            if let Some(e) = eski.as_ref().filter(|e| e.len() > TABAN_ESIK) {
                if (alanlar.len() as f64) < e.len() as f64 * TABAN_ORAN {
                    return Err(format!("liste beklenenden çok küçük: {} / eski {}", alanlar.len(), e.len()));
                }
            }
            Ok(alanlar)
        })();
        match deneme {
            Err(h) => {
                let kopya = eski.filter(|e| !e.is_empty());
                let sayi = kopya.as_ref().map_or(0, BTreeSet::len);
                let durum = durum_satiri(k.ad, sayi, Some(&hata_metni(&h)), kopya.is_some());
                Ok((kopya.unwrap_or_default(), durum, false))
            }
            Ok(alanlar) => {
                let metin: String = alanlar.iter().map(|a| format!("{a}\n")).collect();
                atomik(&yol, &gz(&metin))?;
                let durum = durum_satiri(k.ad, alanlar.len(), None, false);
                Ok((alanlar, durum, true))
            }
        }
    }

    /// Kategorinin listesini üretir; meta döner (ayrıca `<kategori>.json`).
    pub fn uret(&self, kategori: &str, t: &dyn Tasiyici, dizin: &Path, simdi: f64) -> Result<Map<String, Value>, String> {
        std::fs::create_dir_all(dizin).map_err(|e| format!("{}: {e}", dizin.display()))?;
        let (mut kumeler, mut durumlar, mut inen) = (Vec::<BTreeSet<String>>::new(), Vec::<Value>::new(), 0usize);
        let ek = elle_ek(dizin, kategori);
        for k in self.kategori_kaynaklari(kategori) {
            let (alanlar, durum, indi) = self.kaynak_al(kategori, k, t, dizin)?;
            kumeler.push(alanlar);
            durumlar.push(durum);
            inen += usize::from(indi);
        }
        if let Some(ek) = ek {
            durumlar.push(durum_satiri(EK_AD, ek.len(), None, false));
            kumeler.insert(0, ek);
        }
        let hepsi: BTreeSet<String> = kumeler.iter().flatten().cloned().collect();
        let zaman = crate::ortak::now_iso(simdi);
        if inen == 0 || hepsi.is_empty() {
            // hiçbir kaynak bugün inmedi: eski liste ve sürümü kalır
            let mut meta = meta_oku(dizin, kategori).unwrap_or_else(|| {
                let mut m = Map::new();
                m.insert("kategori".into(), json!(kategori));
                m
            });
            meta.insert("hata".into(), json!("Hiçbir kaynak indirilemedi; önceki liste kullanılıyor."));
            meta.insert("hata_zaman".into(), json!(zaman));
            meta.insert("kaynaklar".into(), Value::Array(durumlar));
            atomik(&meta_yolu(dizin, kategori), crate::veri::py_json(&Value::Object(meta.clone()), false).as_bytes())?;
            return Ok(meta);
        }
        // üst adı listede olan alt ad gereksiz (dnsmasq alt adları da engeller); son noktadan sonrası (uzantı) bakılmaz
        let mut sade: BTreeSet<String> = hepsi
            .iter()
            .filter(|a| {
                let noktalar: Vec<usize> = a.match_indices('.').map(|(i, _)| i).collect();
                let son = noktalar.len().saturating_sub(1);
                !noktalar[..son].iter().any(|&i| hepsi.contains(&a[i + 1..]))
            })
            .cloned()
            .collect();
        let kesildi = sade.len() > self.en_cok;
        if kesildi {
            let mut secilen = BTreeSet::new();
            for k in &kumeler {
                for a in k.iter().filter(|a| sade.contains(*a)) {
                    if secilen.len() >= self.en_cok {
                        break;
                    }
                    secilen.insert(a.clone());
                }
                if secilen.len() >= self.en_cok {
                    break;
                }
            }
            sade = secilen;
        }
        let metin: String = sade.iter().map(|a| format!("{a}\n")).collect();
        // Önce liste, sonra meta: arada (ya da meta yazılamazsa) cihaz eski sha ile yeni dosyayı alabilir; cihaz sha'yı
        // doğruladığı için yazmaz, eskisini tutar ve sonraki raporda yeniden dener. Zararsız pencere.
        atomik(&liste_yolu(dizin, kategori)?, &gz(&metin))?;
        let mut meta = Map::new();
        meta.insert("kategori".into(), json!(kategori));
        meta.insert("sha256".into(), json!(hex(&Sha256::digest(metin.as_bytes()))));
        meta.insert("sayi".into(), json!(sade.len()));
        meta.insert("zaman".into(), json!(zaman));
        meta.insert("kesildi".into(), json!(kesildi));
        meta.insert("kaynaklar".into(), Value::Array(durumlar));
        atomik(&meta_yolu(dizin, kategori), crate::veri::py_json(&Value::Object(meta.clone()), false).as_bytes())?;
        Ok(meta)
    }

    /// Üç kategoriyi üretir, hareket yazar. Dönen: 0 hepsi temiz, 1 bir kaynak ya da kategori hatalı. `bekle=false` iken
    /// başka güncelleme sürüyorsa `Mesgul`. Hareket yazılamazsa (veritabanı) o satır günlüğe düşer ve sonuç 1 olur.
    #[allow(clippy::too_many_arguments)]
    pub fn calistir(&self, veri: &Veri, t: &dyn Tasiyici, dizin: &Path, simdi: f64, kim: &str, ip: &str, bekle: bool) -> Result<i32, Mesgul> {
        let _kilit = Kilit::al(dizin, bekle)?;
        let mut sonuc = 0;
        let yaz = |olay: &str, ayrinti: String, sonuc: &mut i32| {
            if let Err(h) = veri.hareket(kim, ip, olay, None, &ayrinti) {
                eprintln!("hareket yazılamadı ({olay}): {h}");
                *sonuc = 1;
            }
        };
        for (k, _) in KATEGORILER {
            let onceki = meta_oku(dizin, k).and_then(|m| m.get("sha256").cloned());
            let meta = match self.uret(k, t, dizin, simdi) {
                Ok(m) => m,
                Err(h) => {
                    // bir kategorinin hatası diğerlerini durdurmaz
                    yaz("LISTE_HATA", format!("kategori={k} neden=OSError: {h}"), &mut sonuc);
                    sonuc = 1;
                    continue;
                }
            };
            let kaynaklar: Vec<Value> = meta.get("kaynaklar").and_then(Value::as_array).cloned().unwrap_or_default();
            let hatali = |d: &&Value| d.get("hata").is_some_and(py_dogru);
            if meta.get("hata").is_some_and(py_dogru) {
                let neden = kaynaklar
                    .iter()
                    .filter(hatali)
                    .map(|d| format!("{}: {}", py_str(&d["ad"]), py_str(&d["hata"])))
                    .collect::<Vec<_>>()
                    .join("; ");
                let neden = if neden.is_empty() { py_str(&meta["hata"]) } else { neden };
                yaz("LISTE_HATA", format!("kategori={k} neden={neden}"), &mut sonuc);
                sonuc = 1;
                continue;
            }
            for d in kaynaklar.iter().filter(hatali) {
                yaz("LISTE_HATA", format!("kategori={k} kaynak={} neden={}", py_str(&d["ad"]), py_str(&d["hata"])), &mut sonuc);
                sonuc = 1;
            }
            if meta.get("sha256") != onceki.as_ref() {
                let sha = meta["sha256"].as_str().unwrap_or("");
                let ek = if meta.get("kesildi").is_some_and(py_dogru) { " kesildi" } else { "" };
                yaz("LISTE_GUNCEL", format!("kategori={k} surum={} sayi={}{ek}", &sha[..sha.len().min(8)], meta["sayi"]), &mut sonuc);
            }
        }
        Ok(sonuc)
    }
}

pub fn uret(kategori: &str, t: &dyn Tasiyici, dizin: &Path, simdi: f64) -> Result<Map<String, Value>, String> {
    Uretici::varsayilan().uret(kategori, t, dizin, simdi)
}

#[allow(clippy::too_many_arguments)]
pub fn calistir(veri: &Veri, t: &dyn Tasiyici, dizin: &Path, simdi: f64, kim: &str, ip: &str, bekle: bool) -> Result<i32, Mesgul> {
    Uretici::varsayilan().calistir(veri, t, dizin, simdi, kim, ip, bekle)
}

/// `wificorrect-merkez listeler` (wc-listeler.timer, 04:30): Python `main()`: umask 077, canlı veritabanı, gerçek taşıyıcı.
pub fn main_uretim() -> i32 {
    // SAFETY: umask çağrısı yan etkisiz bir sistem çağrısıdır
    unsafe {
        libc::umask(0o077);
    }
    let v = match Veri::ac(Path::new(crate::merkez::DB), std::sync::Arc::new(crate::ortak::simdi)) {
        Ok(v) => v,
        Err(h) => {
            eprintln!("{h}");
            return 1;
        }
    };
    calistir(&v, &crate::ortak::Curl, Path::new(DIZIN), crate::ortak::simdi(), "listeler", "", true).unwrap_or(1)
}

#[cfg(test)]
#[path = "listeler_testler.rs"]
mod testler;
