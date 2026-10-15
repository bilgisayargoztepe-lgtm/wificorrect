//! AP kurulum modu (spec 2026-10-15-ap-kurulum): hizmet sağlayıcı ya da işletme sahibi, müşterinin misafir portuna
//! taktığı erişim noktasını (AP) cihaz üzerinden uzaktan kurar. Burada saf işlevler ve komut listeleri var (komutlar
//! `Runner` ile, canlı çıktı `ip_oku` ile verilir; testler gerçek ağa dokunmaz):
//! - ağdaki cihazlar: dnsmasq kiraları + `ip -4 neigh` (MAC, IP, ad),
//! - yan oda ("Ek: yan oda"): fabrika adresli AP'ye ulaşmak için misafir köprüsüne macvlan ile bağlı ayrı bir ağ ad
//!   alanı + 127.0.0.1'de SOCKS5 aracı (`yan_oda.rs`), geçici `wfc-ap-araci` systemd birimi olarak. Ana ağ ad alanına
//!   adres/rota eklenmez (WAN'la, tünelle çakışma yok; 192.168.1.250/24 de olur). Önce `systemd-run` kapatma
//!   zamanlayıcısı kurulur (kurulamazsa hiçbir şey değişmez), sonra aracı; birimin `RuntimeMaxSec`'i yedek emniyettir
//!   (aracı süreci bitince yan oda çekirdekçe silinir). Durum `/var/lib/wificorrect/ap-gecici.json`: açılışta dosya
//!   kalmış ama aracı yoksa (yeniden başlatma) temizlenir (`neden=acilis`),
//! - AP adresi kaydı (`[ap] adresler`): yalnızca kayıt ve gösterim, güvenlik duvarı kuralı yok. Köprü portları arası
//!   iletim `kapi.nft`'teki yalıtımla zaten düşer; AP'ye kablosuz bağlı misafirin trafiği cihazdan hiç geçmez (asıl
//!   koruma AP'nin kendi ayarı, docs/AP_KURULUM.md),
//! - yabancı DHCP denetimi: `ctl ap-dhcp-denetle` çıktısının ayrıştırılması.

use crate::ayar::Config;
use crate::ortak::{self, Runner};
use crate::yan_oda::ARACI_BIRIM;
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::PathBuf;

/// Yan oda süresi (sn)
pub const SURE_SN: u64 = 1800;
/// DHCP havuzu (son bayt; dnsmasq `dhcp-range` ile aynı)
pub const HAVUZ: (u8, u8) = (20, 249);
/// Kayıtlı AP adresi sınırı
pub const EN_COK: usize = 8;

const BIRIM_ONEK: &str = "wfc-ap-gecici-";

pub struct Yollar {
    pub gecici: PathBuf,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar { gecici: "/var/lib/wificorrect/ap-gecici.json".into() }
    }
}

/// Canlı komut çıktısı okuyucu (gerçekte `ortak::capture`; testte sahte)
pub type IpOku = dyn Fn(&[&str]) -> String;

// ---------------------------------------------------------------- CIDR
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cidr {
    pub ip: Ipv4Addr,
    pub onek: u8,
}

fn maske(onek: u8) -> u32 {
    if onek == 0 { 0 } else { u32::MAX << (32 - u32::from(onek)) }
}

impl Cidr {
    /// Yalnızca `a.b.c.d/p` (p 0–32); boşluk ve fazlası kabul edilmez.
    pub fn parse(s: &str) -> Option<Cidr> {
        let (ip, p) = s.split_once('/')?;
        if p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let onek: u8 = p.parse().ok().filter(|p| *p <= 32)?;
        Some(Cidr { ip: ip.parse().ok()?, onek })
    }

    pub fn ag(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.ip) & maske(self.onek))
    }

    pub fn yayin(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.ip) | !maske(self.onek))
    }

    /// Biri diğerinin ağını kapsıyor mu (geniş olanın önekiyle karşılaştırılır)
    pub fn cakisir(&self, o: &Cidr) -> bool {
        let m = maske(self.onek.min(o.onek));
        u32::from(self.ip) & m == u32::from(o.ip) & m
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.ip, self.onek)
    }
}


fn ozel(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
}

/// Yan oda adresi: `a.b.c.d/önek`, önek 16–30, özel aralık, ağ/yayın adresi değil, misafir ağıyla çakışmıyor (aynı
/// bağlantı katmanında misafirlerle adres çakışması olur). Ana ağ ad alanına eklenmediği için WAN/tünel ağıyla
/// çakışma serbesttir (ör. modem 192.168.1.1, AP de 192.168.1.1 → 192.168.1.250/24).
pub fn gecici_dogrula(girdi: &str, cfg: &Config) -> Result<Cidr, String> {
    let c = Cidr::parse(girdi.trim()).ok_or("Adres a.b.c.d/önek biçiminde olmalı (ör. 192.168.1.250/24).")?;
    if !(16..=30).contains(&c.onek) {
        return Err("Önek 16 ile 30 arasında olmalı (/16'dan geniş alt ağ kabul edilmez).".into());
    }
    if !ozel(c.ip) {
        return Err("Yalnızca özel ağ adresi kullanılabilir (10.x, 172.16-31.x, 192.168.x).".into());
    }
    if c.ip == c.ag() || c.ip == c.yayin() {
        return Err("Bu, alt ağın ağ ya da yayın adresi; başka bir adres seçin.".into());
    }
    if Cidr::parse(&cfg.main.subnet).is_none_or(|m| m.cakisir(&c)) {
        return Err("Bu adres misafir ağının içinde; AP zaten bu ağdaysa yan oda gerekmez.".into());
    }
    Ok(c)
}

// ---------------------------------------------------------------- AP adresi kaydı
/// Kayıtlı AP adresleri girdisi (satır, virgül ya da boşlukla ayrılmış). Her biri IPv4, misafir alt ağında, köprü
/// adresi / ağ / yayın adresi değil, DHCP havuzu dışında (ağ adresinden uzaklık `HAVUZ` aralığında değil); tekrarlar
/// atılır; en çok `EN_COK`. Boş girdi = liste boşaltılır.
pub fn kayit_dogrula(girdi: &str, cfg: &Config) -> Result<Vec<String>, String> {
    let ag = Cidr::parse(&cfg.main.subnet).ok_or("Misafir ağı ayarı (main.subnet) okunamadı.")?;
    let kopru: Option<Ipv4Addr> = cfg.main.router_ip.parse().ok();
    let mut v: Vec<String> = vec![];
    for p in girdi.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()) {
        let ip: Ipv4Addr = p.parse().map_err(|_| format!("\"{p}\" geçerli bir IPv4 adresi değil."))?;
        let c = Cidr { ip, onek: ag.onek };
        if !ag.cakisir(&c) {
            return Err(format!("{ip} misafir ağının ({ag}) dışında; AP'nin yönetim adresi misafir ağında olmalı."));
        }
        if Some(ip) == kopru {
            return Err(format!("{ip} cihazın kendi adresi; AP'ye başka bir adres verin."));
        }
        if ip == c.ag() || ip == c.yayin() {
            return Err(format!("{ip} misafir ağının ağ ya da yayın adresi."));
        }
        let uzaklik = u32::from(ip) - u32::from(c.ag());
        if (u32::from(HAVUZ.0)..=u32::from(HAVUZ.1)).contains(&uzaklik) {
            let o = c.ag().octets();
            return Err(format!(
                "{ip} DHCP havuzunda ({}.{}.{}.{}–{}); AP'ye havuz dışında bir adres verin.",
                o[0], o[1], o[2], u32::from(o[3]) + u32::from(HAVUZ.0), u32::from(o[3]) + u32::from(HAVUZ.1)
            ));
        }
        let s = ip.to_string();
        if !v.contains(&s) {
            v.push(s);
        }
    }
    if v.len() > EN_COK {
        return Err(format!("En çok {EN_COK} AP adresi kaydedilebilir."));
    }
    Ok(v)
}


// ---------------------------------------------------------------- yan oda
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Gecici {
    pub adres: String,
    pub kimlik: String,
    pub bitis: f64,
}

fn kimlik_ok(k: &str) -> bool {
    k.len() == 8 && k.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// (kapatma zamanlayıcısı, aracı) komutları. Aracı `Type=notify`: `systemd-run` yan oda kurulana kadar bekler, kurulamazsa
/// başarısız döner (`TimeoutStartSec` `ortak::run`'ın 15 sn'sinden kısa). `RuntimeMaxSec` süreden 10 sn uzun: zamanlayıcı
/// önce çalışıp denetimi yazar, çalışmasa da aracı (ve yan oda) durur.
/// Aracı kısıtlı çalışır (`ARACI_KISIT`): yalnızca CAP_NET_ADMIN (macvlan, yan oda sysctl) ve CAP_SYS_ADMIN (unshare,
/// setns); dosya sistemi salt okunur. `PrivateNetwork`, `RestrictNamespaces` ve `ProtectKernelTunables` bilerek yok
/// (aracı ad alanı açıp girer ve yan odanın /proc/sys/net değerlerini yazar).
pub const ARACI_KISIT: [&str; 10] = [
    "--property=NoNewPrivileges=yes",
    "--property=ProtectSystem=strict",
    "--property=ProtectHome=yes",
    "--property=PrivateTmp=yes",
    "--property=CapabilityBoundingSet=CAP_NET_ADMIN CAP_SYS_ADMIN",
    "--property=RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK",
    "--property=TasksMax=128",
    "--property=MemoryMax=64M",
    "--property=ProtectControlGroups=yes",
    "--property=LockPersonality=yes",
];

pub fn ekle_komutlari(c: &Cidr, kimlik: &str, sure: u64) -> (Vec<String>, Vec<String>) {
    let birim = format!("{BIRIM_ONEK}{kimlik}");
    let zaman = format!("{sure}s");
    let omur = format!("--property=RuntimeMaxSec={}", sure + 10);
    let adres = c.to_string();
    (
        s(&["systemd-run", "--collect", "--unit", &birim, "--on-active", &zaman, "--timer-property=AccuracySec=1s",
            "/usr/local/bin/wificorrect", "ctl", "ap-adres-kaldir", "sure", kimlik]),
        ["systemd-run", "--collect", "--unit", ARACI_BIRIM, "--service-type=notify", "--property=TimeoutStartSec=12", &omur]
            .into_iter()
            .chain(ARACI_KISIT)
            .chain(["/usr/local/bin/wificorrect", "ctl", "ap-araci", &adres])
            .map(String::from)
            .collect(),
    )
}

/// 0.4.11'in geçici adresi ana ad alanında köprüdeydi (etiket `<köprü>:ap`); o sürümün zamanlayıcısı güncellemeden sonra
/// çalışırsa adres de kalkar. Yalnızca bu etiketli adresleri siler; yoksa bir şey yapmaz.
pub fn eski_adres_temizle(kopru: &str) -> Vec<String> {
    s(&["ip", "addr", "flush", "dev", kopru, "label", &format!("{kopru}:ap")])
}

fn zamanlayici_durdur(kimlik: &str) -> Vec<String> {
    s(&["systemctl", "stop", &format!("{BIRIM_ONEK}{kimlik}.timer")])
}

fn araci_durdur() -> Vec<String> {
    s(&["systemctl", "stop", &format!("{ARACI_BIRIM}.service")])
}

/// Aracı birimi çalışıyor mu (yan oda yalnızca aracı süreci yaşadıkça var)
pub fn araci_canli(ip_oku: &IpOku) -> bool {
    matches!(ip_oku(&["systemctl", "is-active", &format!("{ARACI_BIRIM}.service")]).trim(), "active" | "activating" | "reloading" | "deactivating")
}

fn oku(y: &Yollar) -> Option<Gecici> {
    let g: Gecici = serde_json::from_slice(&std::fs::read(&y.gecici).ok()?).ok()?;
    (Cidr::parse(&g.adres).is_some() && kimlik_ok(&g.kimlik)).then_some(g)
}

/// Yan oda işlemlerini sıraya sokar (panelin iş parçacıkları, `ctl`, zamanlayıcı): durum dosyasının yanında `.kilit`
/// dosyası, flock; dosya kapanınca bırakılır.
fn kilit(y: &Yollar) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    if let Some(d) = y.gecici.parent() {
        let _ = ortak::mkdirs(d);
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(y.gecici.with_extension("kilit"))
        .map_err(|e| format!("Kilit dosyası açılamadı: {e}"))?;
    // SAFETY: geçerli dosya tanıtıcısı; kilit dosya kapanınca bırakılır.
    unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    Ok(f)
}

/// Yan oda açar (panel ve `ctl ap-adres-ekle`). Sıra: denetim → kapatma zamanlayıcısı → durum dosyası → aracı.
pub fn ekle(cfg: &Config, y: &Yollar, girdi: &str, sure: u64, ip_oku: &IpOku, runner: &Runner, now: f64) -> Result<Gecici, String> {
    let _kilit = kilit(y)?;
    if y.gecici.exists() {
        return Err("Zaten bir yan oda açık; önce kapatın.".into());
    }
    let c = gecici_dogrula(girdi, cfg)?;
    if araci_canli(ip_oku) {
        runner(&araci_durdur()); // durum dosyası olmadan kalmış aracı (yarıda kalmış işlem)
    }
    let kimlik = ortak::random_hex(4);
    let (zamanlayici, araci) = ekle_komutlari(&c, &kimlik, sure);
    if !runner(&zamanlayici) {
        return Err("Kapatma zamanlayıcısı kurulamadı; yan oda açılmadı.".into());
    }
    let g = Gecici { adres: c.to_string(), kimlik: kimlik.clone(), bitis: now + sure as f64 };
    let geri = |e: String| -> Result<Gecici, String> {
        runner(&araci_durdur());
        runner(&zamanlayici_durdur(&kimlik));
        let _ = std::fs::remove_file(&y.gecici);
        Err(e)
    };
    let veri = serde_json::to_vec(&g).map_err(|e| e.to_string());
    if let Err(e) = veri.and_then(|v| ortak::write_atomic_mode(&y.gecici, &v, 0o600).map_err(|e| e.to_string())) {
        return geri(format!("Durum dosyası yazılamadı ({e}); yan oda açılmadı."));
    }
    if !runner(&araci) {
        return geri("Yan oda açılamadı (ağ ad alanı ya da aracı başlatılamadı; ayrıntı: journalctl -u wfc-ap-araci).".into());
    }
    Ok(g)
}

/// Kapatılan yan oda (denetim için); `hata`: süre dolumunda aracı durdurulamadı ama durum dosyası yine silindi
/// (`RuntimeMaxSec` aracıyı 10 sn sonra durdurur).
#[derive(Debug, Clone, PartialEq)]
pub struct Kaldirilan {
    pub adres: String,
    pub hata: Option<&'static str>,
}

/// Yan odayı kapatır (aracı durur → ad alanı ve macvlan çekirdekçe silinir). `kimlik` (zamanlayıcı yolu) dosyadakiyle
/// aynı değilse hiçbir şey yapmaz: eski bir zamanlayıcı yeni yan odayı erken kapatamaz. `neden` "sure" değilse
/// zamanlayıcı durdurulur; "sure" değilken aracı durdurulamazsa hata döner ve durum dosyası kalır.
pub fn kaldir(y: &Yollar, neden: &str, kimlik: Option<&str>, ip_oku: &IpOku, runner: &Runner) -> Result<Option<Kaldirilan>, String> {
    let _kilit = kilit(y)?;
    let Some(g) = oku(y) else {
        if kimlik.is_none() {
            if y.gecici.exists() {
                let _ = std::fs::remove_file(&y.gecici); // bozuk dosya: elle kapatmada temizlenir
            }
            // durum dosyası olmadan kalmış aracı (yarıda kalmış işlem): elle kapatmada durdurulur
            if araci_canli(ip_oku) {
                if !runner(&araci_durdur()) {
                    return Err("Yan oda kapatılamadı (aracı durdurulamadı).".into());
                }
                return Ok(Some(Kaldirilan { adres: "-".into(), hata: None }));
            }
        }
        return Ok(None);
    };
    if kimlik.is_some_and(|k| k != g.kimlik) {
        return Ok(None);
    }
    let sure = neden == "sure";
    let hata = (araci_canli(ip_oku) && !runner(&araci_durdur())).then_some("araci_durmadi");
    if hata.is_some() && !sure {
        return Err("Yan oda kapatılamadı (aracı durdurulamadı).".into());
    }
    if !sure {
        runner(&zamanlayici_durdur(&g.kimlik));
    }
    std::fs::remove_file(&y.gecici).map_err(|e| format!("Durum dosyası silinemedi: {e}"))?;
    Ok(Some(Kaldirilan { adres: g.adres, hata }))
}

/// Panel gösterimi: açık yan oda ve aracı hâlâ çalışıyor mu.
pub fn durum(y: &Yollar, ip_oku: &IpOku) -> Option<(Gecici, bool)> {
    let g = oku(y)?;
    Some((g, araci_canli(ip_oku)))
}

/// Açılışta (`ctl yukle`): dosya var ama aracı yoksa (yeniden başlatma) dosya silinir, adres döner (`neden=acilis`).
/// Aracı çalışıyorsa (yalnızca güvenlik duvarı servisi yeniden başladı) dokunulmaz.
pub fn acilis_temizle(y: &Yollar, ip_oku: &IpOku) -> Option<String> {
    if !y.gecici.exists() {
        return None;
    }
    let _kilit = kilit(y).ok();
    match durum(y, ip_oku) {
        Some((_, true)) => None,
        Some((g, false)) => std::fs::remove_file(&y.gecici).ok().map(|_| g.adres),
        None => {
            let _ = std::fs::remove_file(&y.gecici); // bozuk dosya
            None
        }
    }
}

// ---------------------------------------------------------------- yabancı DHCP denetimi
/// Yabancı DHCP sunucusu (sunucu IP'si, MAC, teklif ettiği adres)
#[derive(Debug, Clone, PartialEq)]
pub struct YabanciDhcp {
    pub sunucu: Ipv4Addr,
    pub mac: String,
    pub adres: Ipv4Addr,
}

/// `ctl ap-dhcp-denetle` çıktısı (`TEKLIF <sunucu> <mac> <adres>` satırları + `TAMAM <n>`). `TAMAM` yoksa denetim
/// yapılamamıştır (None). Cihazın kendi sunucusu (köprü adresi + köprü MAC'i; MAC okunamadıysa yalnızca adres) atılır.
pub fn dhcp_cikti_coz(cikti: &str, kopru_ip: &str, kopru_mac: &str) -> Option<Vec<YabanciDhcp>> {
    if !cikti.lines().any(|l| l.starts_with("TAMAM ")) {
        return None;
    }
    let kendi_mac = ortak::norm_mac(kopru_mac);
    let mut v: Vec<YabanciDhcp> = vec![];
    for l in cikti.lines() {
        let p: Vec<&str> = l.split_whitespace().collect();
        let ["TEKLIF", sunucu, mac, adres] = p[..] else { continue };
        let (Ok(sunucu), Some(mac), Ok(adres)) = (sunucu.parse::<Ipv4Addr>(), ortak::norm_mac(mac), adres.parse::<Ipv4Addr>()) else { continue };
        if sunucu.to_string() == kopru_ip && kendi_mac.as_ref().is_none_or(|k| *k == mac) {
            continue;
        }
        let t = YabanciDhcp { sunucu, mac, adres };
        if !v.contains(&t) {
            v.push(t);
        }
    }
    Some(v)
}

// ---------------------------------------------------------------- ağdaki cihazlar
#[derive(Debug, Clone, PartialEq)]
pub struct Komsu {
    pub mac: String,
    pub ipler: BTreeSet<Ipv4Addr>,
    pub ad: String,
    pub dhcp: bool,
    /// ARP/komşu durumu (REACHABLE, STALE …); komşu tablosunda yoksa None
    pub arp: Option<String>,
    /// Yerel yönetilen (rastgele) MAC: telefonlar; AP'ler genelde değil
    pub rastgele: bool,
}

/// dnsmasq kira dosyası (`bitis mac ip ad istemci-kimliği`) + `ip -4 neigh show dev <köprü>`; MAC'e göre birleşir,
/// IP'ye göre sıralanır. `lladdr`'sız (FAILED / INCOMPLETE) komşu satırları atlanır.
pub fn komsular(kiralar: &str, neigh: &str) -> Vec<Komsu> {
    fn al(m: &mut BTreeMap<String, Komsu>, mac: String) -> &mut Komsu {
        let rastgele = u8::from_str_radix(&mac[..2], 16).is_ok_and(|b| b & 0x02 != 0);
        m.entry(mac.clone()).or_insert(Komsu { mac, ipler: BTreeSet::new(), ad: String::new(), dhcp: false, arp: None, rastgele })
    }
    let mut m: BTreeMap<String, Komsu> = BTreeMap::new();
    for l in kiralar.lines() {
        let p: Vec<&str> = l.split_whitespace().collect();
        let (Some(mac), Some(ip)) = (p.get(1).and_then(|x| ortak::norm_mac(x)), p.get(2).and_then(|x| x.parse::<Ipv4Addr>().ok())) else { continue };
        let k = al(&mut m, mac);
        k.ipler.insert(ip);
        k.dhcp = true;
        if let Some(ad) = p.get(3).filter(|a| **a != "*") {
            k.ad = ad.chars().filter(|c| !c.is_control()).take(63).collect();
        }
    }
    for l in neigh.lines() {
        let p: Vec<&str> = l.split_whitespace().collect();
        let Some(ip) = p.first().and_then(|x| x.parse::<Ipv4Addr>().ok()) else { continue };
        let Some(mac) = p.iter().position(|t| *t == "lladdr").and_then(|i| p.get(i + 1)).and_then(|x| ortak::norm_mac(x)) else { continue };
        let durum = p.last().filter(|t| t.chars().all(|c| c.is_ascii_uppercase())).map(|t| t.to_string());
        let k = al(&mut m, mac);
        k.ipler.insert(ip);
        k.arp = durum.or(Some(String::new()));
    }
    let mut v: Vec<Komsu> = m.into_values().collect();
    v.sort_by_key(|k| k.ipler.iter().next().copied());
    v
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn cfg_ve_yol() -> (Config, Yollar, PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-ap-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = Config::parse(&format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n", root.display())).unwrap();
        (cfg, Yollar { gecici: root.join("ap-gecici.json") }, root)
    }

    /// Sahte `systemctl is-active`: aracı çalışıyor mu
    fn canli(acik: bool) -> impl Fn(&[&str]) -> String {
        move |a: &[&str]| match a {
            ["systemctl", "is-active", "wfc-ap-araci.service"] => if acik { "active\n" } else { "inactive\n" }.into(),
            _ => String::new(),
        }
    }

    fn kayitci(sonuc: impl Fn(&[String]) -> bool + Send + Sync + 'static) -> (Arc<Mutex<Vec<Vec<String>>>>, Box<Runner>) {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
        let c2 = calls.clone();
        (calls, Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.to_vec());
            sonuc(c)
        }))
    }

    #[test]
    fn cidr_ayristirma_ag_yayin_cakisma() {
        let c = Cidr::parse("192.168.0.250/24").unwrap();
        assert_eq!((c.ag(), c.yayin()), ("192.168.0.0".parse().unwrap(), "192.168.0.255".parse().unwrap()));
        assert_eq!(c.to_string(), "192.168.0.250/24");
        for kotu in ["192.168.0.250", "192.168.0.250/33", "192.168.0.250/", "192.168.0.256/24", " 1.2.3.4/24", "1.2.3.4/+8", "1.2.3.4/024", "a/24"] {
            assert!(Cidr::parse(kotu).is_none(), "{kotu}");
        }
        let genis = Cidr::parse("10.0.0.0/8").unwrap();
        assert!(genis.cakisir(&Cidr::parse("10.50.0.1/24").unwrap()) && Cidr::parse("10.50.0.1/24").unwrap().cakisir(&genis));
        assert!(!c.cakisir(&Cidr::parse("192.168.1.110/24").unwrap()));
        assert!(Cidr::parse("192.168.0.250/16").unwrap().cakisir(&Cidr::parse("192.168.1.110/24").unwrap()));
    }

    #[test]
    fn gecici_dogrula_retler_ve_kabul() {
        let (cfg, _, _) = cfg_ve_yol();
        let d = |g: &str| gecici_dogrula(g, &cfg);
        assert!(d("192.168.0.250").unwrap_err().contains("biçiminde"));
        assert!(d("192.168.0.250/15").unwrap_err().contains("16 ile 30"));
        assert!(d("192.168.0.250/31").unwrap_err().contains("16 ile 30"));
        assert!(d("8.8.8.1/24").unwrap_err().contains("özel ağ"));
        assert!(d("192.168.0.0/24").unwrap_err().contains("ağ ya da yayın"));
        assert!(d("192.168.0.255/24").unwrap_err().contains("ağ ya da yayın"));
        assert!(d("10.50.0.9/24").unwrap_err().contains("misafir ağının"));
        assert!(d("10.50.200.9/16").unwrap_err().contains("misafir ağının")); // misafir ağını kapsayan geniş alt ağ
        // yan oda ana ağ ad alanına dokunmaz: modemin ağı (WAN), tünel ağı, başka rotalar artık serbest
        assert_eq!(d(" 192.168.1.250/24 ").unwrap(), Cidr::parse("192.168.1.250/24").unwrap());
        assert!(d("10.99.0.50/24").is_ok() && d("172.16.5.1/24").is_ok() && d("192.168.0.250/16").is_ok());
    }

    #[test]
    fn kayit_dogrula_kurallar() {
        let (cfg, _, _) = cfg_ve_yol();
        let d = |g: &str| kayit_dogrula(g, &cfg);
        assert_eq!(d("10.50.0.2").unwrap(), ["10.50.0.2"]);
        assert_eq!(d("10.50.0.2, 10.50.0.3\n10.50.0.2 10.50.0.250").unwrap(), ["10.50.0.2", "10.50.0.3", "10.50.0.250"]);
        assert!(d("").unwrap().is_empty() && d(" \n ").unwrap().is_empty());
        assert!(d("10.50.0.1").unwrap_err().contains("kendi adresi"));
        assert!(d("10.50.0.0").unwrap_err().contains("ağ ya da yayın"));
        assert!(d("10.50.0.255").unwrap_err().contains("ağ ya da yayın"));
        let havuz = d("10.50.0.30").unwrap_err();
        assert!(havuz.contains("10.50.0.30 DHCP havuzunda (10.50.0.20–249)"), "{havuz}");
        assert!(d("10.50.0.20").is_err() && d("10.50.0.249").is_err() && d("10.50.0.19").is_ok());
        assert!(d("192.168.0.2").unwrap_err().contains("dışında"));
        assert!(d("abc").unwrap_err().contains("\"abc\""));
        assert!(d("10.50.0.2/24").is_err() && d("10.50.0.2;rm").is_err());
        let dokuz: Vec<String> = (2..=10).map(|i| format!("10.50.0.{i}")).collect();
        assert!(d(&dokuz.join(" ")).unwrap_err().contains("En çok 8"));
        assert_eq!(d(&dokuz[..8].join(",")).unwrap().len(), 8);
    }

    #[test]
    fn ekle_komutlari_birebir() {
        let (z, a) = ekle_komutlari(&Cidr::parse("192.168.1.250/24").unwrap(), "0a1b2c3d", 1800);
        assert_eq!(z, s(&["systemd-run", "--collect", "--unit", "wfc-ap-gecici-0a1b2c3d", "--on-active", "1800s", "--timer-property=AccuracySec=1s",
                          "/usr/local/bin/wificorrect", "ctl", "ap-adres-kaldir", "sure", "0a1b2c3d"]));
        assert_eq!(a[..7], s(&["systemd-run", "--collect", "--unit", "wfc-ap-araci", "--service-type=notify", "--property=TimeoutStartSec=12",
                               "--property=RuntimeMaxSec=1810"])[..]);
        assert_eq!(a[7..17], ARACI_KISIT.map(String::from)[..]);
        assert_eq!(a[17..], s(&["/usr/local/bin/wificorrect", "ctl", "ap-araci", "192.168.1.250/24"])[..]);
        // kısıt: yetkiler yalnızca ağ ve ad alanı; ad alanı/sysctl engelleyen ayarlar yok
        assert!(a.iter().any(|x| x == "--property=CapabilityBoundingSet=CAP_NET_ADMIN CAP_SYS_ADMIN") && a.iter().any(|x| x == "--property=NoNewPrivileges=yes"));
        assert!(!a.iter().any(|x| x.contains("PrivateNetwork") || x.contains("RestrictNamespaces") || x.contains("ProtectKernelTunables")));
    }

    #[test]
    fn ekle_sirasi_ve_geri_alma() {
        let (cfg, y, _) = cfg_ve_yol();
        let kapali = canli(false);
        // başarılı: zamanlayıcı aracıdan önce, dosya 600; ana ad alanında ip komutu yok
        let (calls, r) = kayitci(|_| true);
        let g = ekle(&cfg, &y, "192.168.1.250/24", SURE_SN, &kapali, &*r, 1000.0).unwrap();
        assert_eq!((g.adres.as_str(), g.bitis, g.kimlik.len()), ("192.168.1.250/24", 2800.0, 8));
        {
            let v = calls.lock().unwrap();
            assert_eq!(v.len(), 2);
            assert!(v[0].contains(&format!("wfc-ap-gecici-{}", g.kimlik)) && v[1].contains(&"ap-araci".to_string()));
            assert!(!v.iter().any(|c| c[0] == "ip"));
        }
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&y.gecici).unwrap().permissions().mode() & 0o777, 0o600);
        // zaten açık (dosya): ret, komut yok
        let (calls, r) = kayitci(|_| true);
        assert!(ekle(&cfg, &y, "172.16.5.1/24", SURE_SN, &kapali, &*r, 1000.0).unwrap_err().contains("Zaten"));
        assert!(calls.lock().unwrap().is_empty());
        std::fs::remove_file(&y.gecici).unwrap();
        // dosyasız kalmış aracı önce durdurulur
        let (calls, r) = kayitci(|_| true);
        ekle(&cfg, &y, "172.16.5.1/24", SURE_SN, &canli(true), &*r, 1000.0).unwrap();
        assert_eq!(calls.lock().unwrap()[0], s(&["systemctl", "stop", "wfc-ap-araci.service"]));
        std::fs::remove_file(&y.gecici).unwrap();
        // zamanlayıcı kurulamazsa aracı başlatılmaz, dosya yok
        let (calls, r) = kayitci(|c| !c.iter().any(|x| x == "ap-adres-kaldir"));
        assert!(ekle(&cfg, &y, "192.168.1.250/24", SURE_SN, &kapali, &*r, 1000.0).is_err());
        assert!(!calls.lock().unwrap().iter().any(|c| c.iter().any(|x| x == "ap-araci")) && !y.gecici.exists());
        // aracı açılamazsa: aracı + zamanlayıcı durdurulur, dosya silinir
        let (calls, r) = kayitci(|c| !c.iter().any(|x| x == "ap-araci"));
        assert!(ekle(&cfg, &y, "192.168.1.250/24", SURE_SN, &kapali, &*r, 1000.0).unwrap_err().contains("Yan oda açılamadı"));
        let v = calls.lock().unwrap();
        assert!(v.contains(&s(&["systemctl", "stop", "wfc-ap-araci.service"])));
        assert!(v.last().unwrap()[2].starts_with("wfc-ap-gecici-") && v.last().unwrap()[2].ends_with(".timer"));
        assert!(!y.gecici.exists());
        // geçersiz adres: hiçbir komut yok
        let (calls, r) = kayitci(|_| true);
        assert!(ekle(&cfg, &y, "10.50.0.9/24", SURE_SN, &kapali, &*r, 1000.0).is_err());
        assert!(calls.lock().unwrap().is_empty() && !y.gecici.exists());
    }

    #[test]
    fn ekle_esanli_tek() {
        let (cfg, y, _) = cfg_ve_yol();
        // iki eşzamanlı ekleme (çift tıklama): biri açılır, öteki "Zaten" ile reddedilir
        let (calls, r) = kayitci(|_| true);
        let r: &Runner = &*r;
        let ip = canli(false);
        let sonuc: Vec<bool> = std::thread::scope(|t| {
            let h: Vec<_> = (0..2).map(|_| t.spawn(|| ekle(&cfg, &y, "192.168.1.250/24", SURE_SN, &ip, r, 1000.0).is_ok())).collect();
            h.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(sonuc.iter().filter(|b| **b).count(), 1, "{sonuc:?}");
        assert_eq!(calls.lock().unwrap().iter().filter(|c| c.iter().any(|x| x == "ap-araci")).count(), 1);
    }

    fn yaz(y: &Yollar, kimlik: &str) {
        let g = Gecici { adres: "192.168.1.250/24".into(), kimlik: kimlik.into(), bitis: 2800.0 };
        ortak::write_atomic_mode(&y.gecici, &serde_json::to_vec(&g).unwrap(), 0o600).unwrap();
    }

    #[test]
    fn kaldir_kimlik_ve_neden() {
        let (_, y, _) = cfg_ve_yol();
        let acik = canli(true);
        // dosya ve aracı yoksa hiçbir şey
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&y, "elle", None, &canli(false), &*r).unwrap(), None);
        assert!(calls.lock().unwrap().is_empty());
        // dosyasız kalmış aracı: elle kapatma durdurur; zamanlayıcı (kimlikli) dokunmaz; durdurulamazsa hata
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&y, "sure", Some("0a1b2c3d"), &acik, &*r).unwrap(), None);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(kaldir(&y, "elle", None, &acik, &*r).unwrap(), Some(Kaldirilan { adres: "-".into(), hata: None }));
        assert_eq!(*calls.lock().unwrap(), vec![s(&["systemctl", "stop", "wfc-ap-araci.service"])]);
        let (_, r) = kayitci(|_| false);
        assert!(kaldir(&y, "elle", None, &acik, &*r).is_err());
        // eski zamanlayıcı (başka kimlik) yeni yan odayı kapatamaz
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&y, "sure", Some("ffffffff"), &acik, &*r).unwrap(), None);
        assert!(calls.lock().unwrap().is_empty() && y.gecici.exists());
        // süre doldu: aracı durur, zamanlayıcı durdurulmaz
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&y, "sure", Some("0a1b2c3d"), &acik, &*r).unwrap(), Some(Kaldirilan { adres: "192.168.1.250/24".into(), hata: None }));
        assert_eq!(*calls.lock().unwrap(), vec![s(&["systemctl", "stop", "wfc-ap-araci.service"])]);
        assert!(!y.gecici.exists());
        // elle: zamanlayıcı da durdurulur
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert!(kaldir(&y, "elle", None, &acik, &*r).unwrap().is_some());
        assert_eq!(calls.lock().unwrap().last().unwrap(), &s(&["systemctl", "stop", "wfc-ap-gecici-0a1b2c3d.timer"]));
        // aracı zaten yoksa durdurma yok ama dosya silinir
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert!(kaldir(&y, "elle", None, &canli(false), &*r).unwrap().is_some());
        assert!(!calls.lock().unwrap().iter().any(|c| c.contains(&"wfc-ap-araci.service".to_string())) && !y.gecici.exists());
        // aracı durdurulamadı: elle → hata, dosya kalır; süre → dosya yine silinir, hata denetime
        yaz(&y, "0a1b2c3d");
        let (_, r) = kayitci(|c| !c.contains(&"wfc-ap-araci.service".to_string()));
        assert!(kaldir(&y, "elle", None, &acik, &*r).is_err() && y.gecici.exists());
        assert_eq!(kaldir(&y, "sure", Some("0a1b2c3d"), &acik, &*r).unwrap().and_then(|k| k.hata), Some("araci_durmadi"));
        assert!(!y.gecici.exists());
        // bozuk dosya: elle kapatma temizler
        std::fs::write(&y.gecici, "bozuk").unwrap();
        let (_, r) = kayitci(|_| true);
        assert_eq!(kaldir(&y, "elle", None, &canli(false), &*r).unwrap(), None);
        assert!(!y.gecici.exists());
    }

    #[test]
    fn durum_ve_acilis_temizligi() {
        let (_, y, _) = cfg_ve_yol();
        assert!(durum(&y, &canli(true)).is_none() && acilis_temizle(&y, &canli(false)).is_none());
        yaz(&y, "0a1b2c3d");
        assert!(durum(&y, &canli(true)).unwrap().1);
        assert!(!durum(&y, &canli(false)).unwrap().1);
        // güvenlik duvarı servisi yeniden başladı, aracı çalışıyor: dokunulmaz
        assert_eq!(acilis_temizle(&y, &canli(true)), None);
        assert!(y.gecici.exists());
        // yeniden başlatma: aracı yok → dosya silinir
        assert_eq!(acilis_temizle(&y, &canli(false)).as_deref(), Some("192.168.1.250/24"));
        assert!(!y.gecici.exists());
    }

    #[test]
    fn dhcp_ciktisi() {
        let kopru_mac = "00:0E:C4:CE:A0:9A";
        assert_eq!(dhcp_cikti_coz("", "10.50.0.1", kopru_mac), None); // denetim çalışmadı
        assert_eq!(dhcp_cikti_coz("TEKLIF 192.168.1.1 aa:bb:cc:dd:ee:ff 192.168.1.33\n", "10.50.0.1", kopru_mac), None);
        assert_eq!(dhcp_cikti_coz("TAMAM 0\n", "10.50.0.1", kopru_mac), Some(vec![]));
        let c = "TEKLIF 192.168.1.1 AA:BB:CC:DD:EE:FF 192.168.1.33\nTEKLIF 10.50.0.1 00:0e:c4:ce:a0:9a 10.50.0.77\n\
                 TEKLIF 10.50.0.1 11:22:33:44:55:66 10.50.0.80\nTEKLIF bozuk <b> x\nTEKLIF 192.168.1.1 aa:bb:cc:dd:ee:ff 192.168.1.33\nTAMAM 5\n";
        let v = dhcp_cikti_coz(c, "10.50.0.1", kopru_mac).unwrap();
        assert_eq!(v, vec![
            YabanciDhcp { sunucu: "192.168.1.1".parse().unwrap(), mac: "aa:bb:cc:dd:ee:ff".into(), adres: "192.168.1.33".parse().unwrap() },
            // 10.50.0.1'i taklit eden başka MAC yabancıdır
            YabanciDhcp { sunucu: "10.50.0.1".parse().unwrap(), mac: "11:22:33:44:55:66".into(), adres: "10.50.0.80".parse().unwrap() },
        ]);
        // köprü MAC'i okunamadıysa köprü adresinden gelen her teklif kendimiz sayılır
        assert_eq!(dhcp_cikti_coz(c, "10.50.0.1", "").unwrap().len(), 1);
    }

    #[test]
    fn komsular_birlesir() {
        let kira = "1790712334 70:4F:57:DD:7A:E8 10.50.0.2 * 01:70:4f:57:dd:7a:e8\n1790712334 da:11:22:33:44:55 10.50.0.31 iPhone-Ayse *\nbozuk\n";
        let neigh = "10.50.0.2 lladdr 70:4f:57:dd:7a:e8 REACHABLE\n10.50.0.40 FAILED\n10.50.0.7 lladdr 00:0e:c4:aa:bb:cc router STALE\n10.50.0.31 INCOMPLETE\n";
        let k = komsular(kira, neigh);
        assert_eq!(k.iter().map(|x| x.mac.as_str()).collect::<Vec<_>>(), vec!["70:4f:57:dd:7a:e8", "00:0e:c4:aa:bb:cc", "da:11:22:33:44:55"]);
        assert_eq!((k[0].ad.as_str(), k[0].dhcp, k[0].arp.as_deref(), k[0].rastgele), ("", true, Some("REACHABLE"), false));
        assert_eq!((k[1].dhcp, k[1].arp.as_deref()), (false, Some("STALE")));
        assert_eq!((k[2].ad.as_str(), k[2].arp.as_deref(), k[2].rastgele), ("iPhone-Ayse", None, true));
        assert!(!k.iter().any(|x| x.ipler.contains(&"10.50.0.40".parse().unwrap())));
    }

    #[test]
    fn havuz_dnsmasq_ile_ayni() {
        let conf = include_str!("../deploy/debian/etc/dnsmasq.d/wificorrect.conf");
        assert!(conf.lines().any(|l| l.starts_with(&format!("dhcp-range=10.50.0.{},10.50.0.{},", HAVUZ.0, HAVUZ.1))), "{conf}");
    }
}
