//! AP kurulum modu (spec 2026-10-15-ap-kurulum): hizmet sağlayıcı, müşterinin misafir portuna taktığı erişim noktasını
//! (AP) cihaz üzerinden uzaktan kurar. Burada yalnızca saf işlevler ve komut listeleri var (komutlar `Runner` ile,
//! canlı `ip` çıktısı `ip_oku` ile verilir; testler gerçek ağa dokunmaz):
//! - ağdaki cihazlar: dnsmasq kiraları + `ip -4 neigh` (MAC, IP, ad),
//! - geçici adres: fabrika adresli AP'ye ulaşmak için misafir köprüsüne 30 dk'lık ikinci adres. Önce `systemd-run`
//!   zamanlayıcısı kurulur (kurulamazsa hiçbir şey değişmez), sonra adres; çekirdek ömrü (`valid_lft`) yedek emniyettir.
//!   Durum `/var/lib/wificorrect/ap-gecici.json`: açılışta dosya kalmışsa adresin yeniden başlatmayla gittiği anlaşılır
//!   (`neden=acilis`); adres hiçbir ağ dosyasına yazılmaz.

use crate::ayar::Config;
use crate::ortak::{self, Runner};
use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::path::PathBuf;

/// Geçici adresin `ip` etiketi (≤15 karakter, arayüz adıyla başlamalı); varsayılan köprü için. Kod `<köprü>:ap` üretir.
#[allow(dead_code)] // testlerde ve belgede
pub const ETIKET: &str = "br-hotspot:ap";
/// Geçici adres süresi (sn)
pub const SURE_SN: u64 = 1800;
/// DHCP havuzu (son bayt; dnsmasq `dhcp-range` ile aynı)
#[allow(dead_code)] // AP adresi kaydı (plan Task 2) kullanır
pub const HAVUZ: (u8, u8) = (20, 249);
/// Kayıtlı AP adresi sınırı
#[allow(dead_code)] // AP adresi kaydı (plan Task 2) kullanır
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

/// Canlı `ip` çıktısı okuyucu (gerçekte `ortak::capture`; testte sahte)
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

// ---------------------------------------------------------------- canlı adres ve rotalar
/// `ip -o -4 addr show`: (arayüz, adres/önek); `lo` atlanır.
pub fn adresler(ip_addr: &str) -> Vec<(String, Cidr)> {
    ip_addr
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            let ifc = p.get(1)?.split('@').next()?.trim_end_matches(':');
            let i = p.iter().position(|t| *t == "inet")?;
            let c = Cidr::parse(p.get(i + 1)?)?;
            (ifc != "lo").then(|| (ifc.to_string(), c))
        })
        .collect()
}

const ROTA_TURLERI: [&str; 9] = ["unicast", "unreachable", "blackhole", "prohibit", "throw", "local", "broadcast", "multicast", "nat"];

/// `ip -4 route show`: (arayüz, hedef ağ); `default` atlanır, öneksiz hedef /32.
pub fn rotalar(ip_route: &str) -> Vec<(String, Cidr)> {
    ip_route
        .lines()
        .filter_map(|l| {
            let mut p: Vec<&str> = l.split_whitespace().collect();
            if p.first().is_some_and(|t| ROTA_TURLERI.contains(t)) {
                p.remove(0);
            }
            let hedef = *p.first()?;
            if hedef == "default" {
                return None;
            }
            let c = if hedef.contains('/') { Cidr::parse(hedef)? } else { Cidr { ip: hedef.parse().ok()?, onek: 32 } };
            let ifc = p.iter().position(|t| *t == "dev").and_then(|i| p.get(i + 1)).map_or("", |s| *s);
            Some((ifc.to_string(), c))
        })
        .collect()
}

fn ozel(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
}

/// Geçici adres girdisini denetler. `ip_addr` / `ip_route`: canlı `ip -o -4 addr show` ve `ip -4 route show` çıktısı.
pub fn gecici_dogrula(girdi: &str, cfg: &Config, ip_addr: &str, ip_route: &str) -> Result<Cidr, String> {
    let c = Cidr::parse(girdi.trim()).ok_or("Adres a.b.c.d/önek biçiminde olmalı (ör. 192.168.0.250/24).")?;
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
        return Err("Bu adres misafir ağının içinde; AP zaten bu ağdaysa geçici adres gerekmez.".into());
    }
    if Cidr::parse(crate::uzak::AG).is_some_and(|t| t.cakisir(&c)) {
        return Err("Bu adres uzak erişim tünelinin ağıyla (10.99.0.0/24) çakışıyor.".into());
    }
    if let Some((ifc, a)) = adresler(ip_addr).into_iter().chain(rotalar(ip_route)).find(|(_, a)| a.cakisir(&c)) {
        let ag = Cidr { ip: a.ag(), onek: a.onek };
        let ifc = if ifc.is_empty() { "bir".to_string() } else { ifc };
        return Err(format!(
            "Bu adres cihazın {ifc} ağıyla ({ag}) çakışıyor; eklenirse cihazın bağlantısı bozulur. AP'nin fabrika adresi modemin \
             ağındaysa müşteriden AP'yi sıfırlayıp DHCP'den adres almasını isteyin (docs/AP_KURULUM.md)."
        ));
    }
    Ok(c)
}

// ---------------------------------------------------------------- geçici adres
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

/// (zamanlayıcı, adres ekleme) komutları. Çekirdek ömrü süreden 10 sn uzun: zamanlayıcı önce çalışıp denetimi yazar,
/// çalışmasa da adres düşer.
pub fn ekle_komutlari(c: &Cidr, iface: &str, kimlik: &str, sure: u64) -> (Vec<String>, Vec<String>) {
    let birim = format!("{BIRIM_ONEK}{kimlik}");
    let zaman = format!("{sure}s");
    let omur = (sure + 10).to_string();
    let adres = c.to_string();
    let etiket = format!("{iface}:ap");
    (
        s(&["systemd-run", "--collect", "--unit", &birim, "--on-active", &zaman, "--timer-property=AccuracySec=1s",
            "/usr/local/bin/wificorrect", "ctl", "ap-adres-kaldir", "sure", kimlik]),
        s(&["ip", "addr", "add", &adres, "dev", iface, "label", &etiket, "valid_lft", &omur, "preferred_lft", &omur]),
    )
}

fn zamanlayici_durdur(kimlik: &str) -> Vec<String> {
    s(&["systemctl", "stop", &format!("{BIRIM_ONEK}{kimlik}.timer")])
}

fn oku(y: &Yollar) -> Option<Gecici> {
    let g: Gecici = serde_json::from_slice(&std::fs::read(&y.gecici).ok()?).ok()?;
    (Cidr::parse(&g.adres).is_some() && kimlik_ok(&g.kimlik)).then_some(g)
}

fn canli_adresler(cfg: &Config, ip_oku: &IpOku) -> Vec<Cidr> {
    let iface = cfg.main.iface.as_str();
    adresler(&ip_oku(&["ip", "-o", "-4", "addr", "show", "dev", iface])).into_iter().filter(|(i, _)| i == iface).map(|(_, c)| c).collect()
}

/// Geçici adres ekler (panel ve `ctl ap-adres-ekle`). Sıra: denetim → zamanlayıcı → durum dosyası → `ip addr add`.
pub fn ekle(cfg: &Config, y: &Yollar, girdi: &str, sure: u64, ip_oku: &IpOku, runner: &Runner, now: f64) -> Result<Gecici, String> {
    let iface = cfg.main.iface.as_str();
    let etiket = format!("{iface}:ap");
    if y.gecici.exists() || !ip_oku(&["ip", "-o", "-4", "addr", "show", "dev", iface, "label", &etiket]).trim().is_empty() {
        return Err("Zaten bir geçici adres açık; önce kaldırın.".into());
    }
    let c = gecici_dogrula(girdi, cfg, &ip_oku(&["ip", "-o", "-4", "addr", "show"]), &ip_oku(&["ip", "-4", "route", "show"]))?;
    let kimlik = ortak::random_hex(4);
    let (zamanlayici, ip) = ekle_komutlari(&c, iface, &kimlik, sure);
    if !runner(&zamanlayici) {
        return Err("Kaldırma zamanlayıcısı kurulamadı; adres eklenmedi.".into());
    }
    let g = Gecici { adres: c.to_string(), kimlik: kimlik.clone(), bitis: now + sure as f64 };
    let geri = |e: String| -> Result<Gecici, String> {
        runner(&zamanlayici_durdur(&kimlik));
        let _ = std::fs::remove_file(&y.gecici);
        Err(e)
    };
    let veri = serde_json::to_vec(&g).map_err(|e| e.to_string());
    if let Err(e) = veri.and_then(|v| ortak::write_atomic_mode(&y.gecici, &v, 0o600).map_err(|e| e.to_string())) {
        return geri(format!("Durum dosyası yazılamadı ({e}); adres eklenmedi."));
    }
    if !runner(&ip) {
        return geri("Adres eklenemedi (ip addr add başarısız).".into());
    }
    Ok(g)
}

/// Geçici adresi kaldırır. `kimlik` (zamanlayıcı yolu) dosyadakiyle aynı değilse hiçbir şey yapmaz: eski bir zamanlayıcı
/// yeni adresi erken silemez. `neden` "sure" değilse zamanlayıcı durdurulur. Dönen: kaldırılan adres (denetim için).
pub fn kaldir(cfg: &Config, y: &Yollar, neden: &str, kimlik: Option<&str>, ip_oku: &IpOku, runner: &Runner) -> Result<Option<String>, String> {
    let Some(g) = oku(y) else {
        if y.gecici.exists() && kimlik.is_none() {
            let _ = std::fs::remove_file(&y.gecici); // bozuk dosya: elle kaldırmada temizlenir
        }
        return Ok(None);
    };
    if kimlik.is_some_and(|k| k != g.kimlik) {
        return Ok(None);
    }
    let c = Cidr::parse(&g.adres).expect("oku doğruladı");
    let iface = cfg.main.iface.as_str();
    if canli_adresler(cfg, ip_oku).contains(&c) && !runner(&s(&["ip", "addr", "del", &g.adres, "dev", iface])) {
        return Err("Adres kaldırılamadı (ip addr del başarısız).".into());
    }
    if neden != "sure" {
        runner(&zamanlayici_durdur(&g.kimlik));
    }
    std::fs::remove_file(&y.gecici).map_err(|e| format!("Durum dosyası silinemedi: {e}"))?;
    Ok(Some(g.adres))
}

/// Panel gösterimi: açık geçici adres ve canlıda hâlâ var mı (ör. port değişikliği adresi silmiş olabilir).
pub fn durum(y: &Yollar, cfg: &Config, ip_oku: &IpOku) -> Option<(Gecici, bool)> {
    let g = oku(y)?;
    let c = Cidr::parse(&g.adres)?;
    let canli = canli_adresler(cfg, ip_oku).contains(&c);
    Some((g, canli))
}

/// Açılışta (`ctl yukle`): dosya var ama adres canlıda yoksa (yeniden başlatma) dosya silinir, adres döner
/// (`neden=acilis`). Adres canlıdaysa (yalnızca servis yeniden başladı) dokunulmaz.
pub fn acilis_temizle(cfg: &Config, y: &Yollar, ip_oku: &IpOku) -> Option<String> {
    if !y.gecici.exists() {
        return None;
    }
    match durum(y, cfg, ip_oku) {
        Some((_, true)) => None,
        Some((g, false)) => std::fs::remove_file(&y.gecici).ok().map(|_| g.adres),
        None => {
            let _ = std::fs::remove_file(&y.gecici); // bozuk dosya
            None
        }
    }
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

    const IP_ADDR: &str = "1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever
2: enp3s0    inet 192.168.1.110/24 brd 192.168.1.255 scope global dynamic enp3s0\\       valid_lft 3221sec preferred_lft 3221sec
4: br-hotspot    inet 10.50.0.1/24 brd 10.50.0.255 scope global br-hotspot\\       valid_lft forever preferred_lft forever
7: wfc    inet 10.99.0.11/24 scope global wfc\\       valid_lft forever preferred_lft forever
";
    const IP_ROUTE: &str = "default via 192.168.1.1 dev enp3s0 proto dhcp src 192.168.1.110 metric 1002
10.50.0.0/24 dev br-hotspot proto kernel scope link src 10.50.0.1
10.99.0.0/24 dev wfc proto kernel scope link src 10.99.0.11
172.20.0.0/16 dev docker0 proto kernel scope link src 172.20.0.1 linkdown
192.168.1.0/24 dev enp3s0 proto dhcp scope link src 192.168.1.110 metric 1002
unreachable 192.168.77.0/24
";

    fn cfg_ve_yol() -> (Config, Yollar, PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-ap-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = Config::parse(&format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n", root.display())).unwrap();
        (cfg, Yollar { gecici: root.join("ap-gecici.json") }, root)
    }

    /// Sahte `ip`: canlı adres listesi + köprüdeki ek adresler (geçici adres eklenmiş gibi)
    fn ip_sahte(kopru_ek: &'static str) -> impl Fn(&[&str]) -> String {
        move |a: &[&str]| match a {
            ["ip", "-o", "-4", "addr", "show"] => format!("{IP_ADDR}{kopru_ek}"),
            ["ip", "-4", "route", "show"] => IP_ROUTE.into(),
            ["ip", "-o", "-4", "addr", "show", "dev", "br-hotspot", "label", "br-hotspot:ap"] => kopru_ek.into(),
            ["ip", "-o", "-4", "addr", "show", "dev", d] => format!("{IP_ADDR}{kopru_ek}").lines().filter(|l| l.contains(&format!(" {d} "))).map(|l| format!("{l}\n")).collect(),
            _ => String::new(),
        }
    }

    const ACIK: &str = "4: br-hotspot    inet 192.168.0.250/24 scope global br-hotspot:ap\\       valid_lft 1700sec preferred_lft 1700sec\n";

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
    fn adres_ve_rota_okuma() {
        let a = adresler(IP_ADDR);
        assert_eq!(a.len(), 3); // lo atlandı
        assert_eq!(a[0], ("enp3s0".to_string(), Cidr::parse("192.168.1.110/24").unwrap()));
        let r = rotalar(IP_ROUTE);
        assert!(!r.iter().any(|(_, c)| c.onek == 0)); // default yok
        assert!(r.contains(&("docker0".to_string(), Cidr::parse("172.20.0.0/16").unwrap())));
        assert!(r.contains(&(String::new(), Cidr::parse("192.168.77.0/24").unwrap())));
        assert_eq!(rotalar("8.8.8.8 via 192.168.1.1 dev enp3s0\n"), vec![("enp3s0".to_string(), Cidr::parse("8.8.8.8/32").unwrap())]);
    }

    #[test]
    fn gecici_dogrula_retler_ve_kabul() {
        let (cfg, _, _) = cfg_ve_yol();
        let d = |g: &str| gecici_dogrula(g, &cfg, IP_ADDR, IP_ROUTE);
        assert!(d("192.168.0.250").unwrap_err().contains("biçiminde"));
        assert!(d("192.168.0.250/15").unwrap_err().contains("16 ile 30"));
        assert!(d("192.168.0.250/31").unwrap_err().contains("16 ile 30"));
        assert!(d("8.8.8.1/24").unwrap_err().contains("özel ağ"));
        assert!(d("192.168.0.0/24").unwrap_err().contains("ağ ya da yayın"));
        assert!(d("192.168.0.255/24").unwrap_err().contains("ağ ya da yayın"));
        assert!(d("10.50.0.9/24").unwrap_err().contains("misafir ağının"));
        assert!(d("10.50.200.9/16").unwrap_err().contains("misafir ağının")); // misafir ağını kapsayan geniş alt ağ
        assert!(d("10.99.0.50/16").unwrap_err().contains("tünel"));
        // tünel kapalıyken de (canlıda wfc yok)
        assert!(gecici_dogrula("10.99.0.50/24", &cfg, "", "").unwrap_err().contains("tünel"));
        let wan = d("192.168.1.250/24").unwrap_err();
        assert!(wan.contains("enp3s0") && wan.contains("192.168.1.0/24") && wan.contains("DHCP"), "{wan}");
        assert!(d("172.20.5.1/24").unwrap_err().contains("docker0"));
        assert!(d("192.168.77.5/24").unwrap_err().contains("çakışıyor"));
        assert_eq!(d(" 192.168.0.250/24 ").unwrap(), Cidr::parse("192.168.0.250/24").unwrap());
        assert!(d("172.16.5.1/24").is_ok());
    }

    #[test]
    fn ekle_komutlari_birebir() {
        let (z, ip) = ekle_komutlari(&Cidr::parse("192.168.0.250/24").unwrap(), "br-hotspot", "0a1b2c3d", 1800);
        assert_eq!(z, s(&["systemd-run", "--collect", "--unit", "wfc-ap-gecici-0a1b2c3d", "--on-active", "1800s", "--timer-property=AccuracySec=1s",
                          "/usr/local/bin/wificorrect", "ctl", "ap-adres-kaldir", "sure", "0a1b2c3d"]));
        assert_eq!(ip, s(&["ip", "addr", "add", "192.168.0.250/24", "dev", "br-hotspot", "label", ETIKET, "valid_lft", "1810", "preferred_lft", "1810"]));
    }

    #[test]
    fn ekle_sirasi_ve_geri_alma() {
        let (cfg, y, _) = cfg_ve_yol();
        let ip = ip_sahte("");
        // başarılı: zamanlayıcı ip'den önce, dosya 600
        let (calls, r) = kayitci(|_| true);
        let g = ekle(&cfg, &y, "192.168.0.250/24", SURE_SN, &ip, &*r, 1000.0).unwrap();
        assert_eq!((g.adres.as_str(), g.bitis, g.kimlik.len()), ("192.168.0.250/24", 2800.0, 8));
        {
            let v = calls.lock().unwrap();
            assert_eq!(v.len(), 2);
            assert_eq!((v[0][0].as_str(), v[1][0].as_str()), ("systemd-run", "ip"));
            assert!(v[0].contains(&format!("wfc-ap-gecici-{}", g.kimlik)));
        }
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&y.gecici).unwrap().permissions().mode() & 0o777, 0o600);
        // zaten açık (dosya): ret, komut yok
        let (calls, r) = kayitci(|_| true);
        assert!(ekle(&cfg, &y, "172.16.5.1/24", SURE_SN, &ip, &*r, 1000.0).unwrap_err().contains("Zaten"));
        assert!(calls.lock().unwrap().is_empty());
        std::fs::remove_file(&y.gecici).unwrap();
        // zaten açık (dosya yok ama canlıda etiketli adres): ret
        let (calls, r) = kayitci(|_| true);
        assert!(ekle(&cfg, &y, "172.16.5.1/24", SURE_SN, &ip_sahte(ACIK), &*r, 1000.0).unwrap_err().contains("Zaten"));
        assert!(calls.lock().unwrap().is_empty());
        // zamanlayıcı kurulamazsa ip çağrılmaz, dosya yok
        let (calls, r) = kayitci(|c| c[0] != "systemd-run");
        assert!(ekle(&cfg, &y, "192.168.0.250/24", SURE_SN, &ip, &*r, 1000.0).is_err());
        assert!(!calls.lock().unwrap().iter().any(|c| c[0] == "ip") && !y.gecici.exists());
        // ip başarısızsa zamanlayıcı durdurulur, dosya silinir
        let (calls, r) = kayitci(|c| c[0] != "ip");
        assert!(ekle(&cfg, &y, "192.168.0.250/24", SURE_SN, &ip, &*r, 1000.0).unwrap_err().contains("ip addr add"));
        let v = calls.lock().unwrap();
        assert_eq!(&v.last().unwrap()[..2], &s(&["systemctl", "stop"])[..]);
        assert!(v.last().unwrap()[2].starts_with("wfc-ap-gecici-") && v.last().unwrap()[2].ends_with(".timer"));
        assert!(!y.gecici.exists());
        // çakışan adres: hiçbir komut yok
        let (calls, r) = kayitci(|_| true);
        assert!(ekle(&cfg, &y, "192.168.1.250/24", SURE_SN, &ip, &*r, 1000.0).is_err());
        assert!(calls.lock().unwrap().is_empty() && !y.gecici.exists());
    }

    fn yaz(y: &Yollar, kimlik: &str) {
        let g = Gecici { adres: "192.168.0.250/24".into(), kimlik: kimlik.into(), bitis: 2800.0 };
        ortak::write_atomic_mode(&y.gecici, &serde_json::to_vec(&g).unwrap(), 0o600).unwrap();
    }

    #[test]
    fn kaldir_kimlik_ve_neden() {
        let (cfg, y, _) = cfg_ve_yol();
        // dosya yoksa hiçbir şey
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&cfg, &y, "elle", None, &ip_sahte(ACIK), &*r).unwrap(), None);
        assert!(calls.lock().unwrap().is_empty());
        // eski zamanlayıcı (başka kimlik) yeni adresi silemez
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&cfg, &y, "sure", Some("ffffffff"), &ip_sahte(ACIK), &*r).unwrap(), None);
        assert!(calls.lock().unwrap().is_empty() && y.gecici.exists());
        // süre doldu: ip addr del, zamanlayıcı durdurulmaz
        let (calls, r) = kayitci(|_| true);
        assert_eq!(kaldir(&cfg, &y, "sure", Some("0a1b2c3d"), &ip_sahte(ACIK), &*r).unwrap().as_deref(), Some("192.168.0.250/24"));
        assert_eq!(*calls.lock().unwrap(), vec![s(&["ip", "addr", "del", "192.168.0.250/24", "dev", "br-hotspot"])]);
        assert!(!y.gecici.exists());
        // elle: zamanlayıcı da durdurulur
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert!(kaldir(&cfg, &y, "elle", None, &ip_sahte(ACIK), &*r).unwrap().is_some());
        assert_eq!(calls.lock().unwrap().last().unwrap(), &s(&["systemctl", "stop", "wfc-ap-gecici-0a1b2c3d.timer"]));
        // canlıda yoksa ip addr del yok ama dosya silinir
        yaz(&y, "0a1b2c3d");
        let (calls, r) = kayitci(|_| true);
        assert!(kaldir(&cfg, &y, "elle", None, &ip_sahte(""), &*r).unwrap().is_some());
        assert!(!calls.lock().unwrap().iter().any(|c| c[0] == "ip") && !y.gecici.exists());
        // ip addr del başarısız: hata, dosya kalır
        yaz(&y, "0a1b2c3d");
        let (_, r) = kayitci(|c| c[0] != "ip");
        assert!(kaldir(&cfg, &y, "elle", None, &ip_sahte(ACIK), &*r).is_err() && y.gecici.exists());
    }

    #[test]
    fn durum_ve_acilis_temizligi() {
        let (cfg, y, _) = cfg_ve_yol();
        assert!(durum(&y, &cfg, &ip_sahte(ACIK)).is_none() && acilis_temizle(&cfg, &y, &ip_sahte("")).is_none());
        yaz(&y, "0a1b2c3d");
        assert!(durum(&y, &cfg, &ip_sahte(ACIK)).unwrap().1);
        assert!(!durum(&y, &cfg, &ip_sahte("")).unwrap().1);
        // servis yeniden başladı, adres canlıda: dokunulmaz
        assert_eq!(acilis_temizle(&cfg, &y, &ip_sahte(ACIK)), None);
        assert!(y.gecici.exists());
        // yeniden başlatma: adres yok → dosya silinir
        assert_eq!(acilis_temizle(&cfg, &y, &ip_sahte("")).as_deref(), Some("192.168.0.250/24"));
        assert!(!y.gecici.exists());
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
        assert_eq!(ETIKET, format!("{}:ap", Config::default().main.iface));
        assert!(ETIKET.len() <= 15);
    }
}
