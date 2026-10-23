//! Port görevleri ve Wi-Fi yayını (eski ports.py + port-geri-al.sh; RUST_YENIDEN_YAZIM.md A2 #10, A10 Portlar).
//! Tek kaynak `/etc/wificorrect/ag.toml`; ağ dosyaları ondan üretilir (ifupdown, nft WAN adı, hostapd, konsol yazısı).
//! Değişiklik: geri alma zamanlayıcısı (3 dk) kurulur, sonra ağ değişir; panelden "Onayla" denmezse eski ayar döner.
//! Bekleyen değişiklik varken cihaz yeniden açılırsa açılışta eski ayar geri yüklenir (fişi çekmek de kurtarır).
//! İnternet alan portun PPPoE ayarı da burada (`[pppoe]`); pppd bağlantı dosyası ondan üretilir (src/pppoe.rs).

use crate::ayar::Config;
use crate::ortak::{self, Row, Runner};
use crate::pppoe;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const BRIDGE: &str = "br-hotspot";
pub const ONAY_SN: u64 = 180;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Wifi {
    /// true: müşterilere yayın yapar (köprüye girer)
    pub acik: bool,
    pub iface: String,
    pub ssid: String,
    /// boş: şifresiz ağ
    pub sifre: String,
    pub kanal: u8,
}

impl Default for Wifi {
    fn default() -> Self {
        Wifi { acik: false, iface: "wlp2s0".into(), ssid: String::new(), sifre: String::new(), kanal: 6 }
    }
}

/// İnternet alan portta PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md). DHCP hep açık kalır; açıksa yanında pppd
/// çalışır ve PPPoE rotası öne geçer. Kapalı (varsayılan) cihazda ve `[pppoe]` bölümü olmayan eski ag.toml'da PPPoE yoktur.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Pppoe {
    pub acik: bool,
    /// Tek alan: modemlerdeki "kullanıcı adı @ alan" birlikte yazılır (ör. 5321234567@turk.net)
    pub kullanici: String,
    /// Yalnızca yazılır: panelde ve servis sayfasında geri gösterilmez, boş gönderilirse eskisi kalır
    pub parola: String,
    /// 0: VLAN yok; 1-4094
    pub vlan: u16,
    /// VLAN 802.1p önceliği 0-7 (yalnızca VLAN varken anlamlı)
    pub oncelik: u8,
    /// MTU ve MRU, 1280-1492
    pub mtu: u16,
    /// otomatik | pap | chap
    pub kimlik: String,
    /// Boş: herhangi bir erişim yoğunlaştırıcı (AC)
    pub ac_adi: String,
    /// Boş: herhangi bir servis
    pub servis_adi: String,
}

impl Default for Pppoe {
    fn default() -> Self {
        Pppoe {
            acik: false,
            kullanici: String::new(),
            parola: String::new(),
            vlan: 0,
            oncelik: 0,
            mtu: pppoe::MTU as u16,
            kimlik: "otomatik".into(),
            ac_adi: String::new(),
            servis_adi: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Ag {
    /// İnternet alan Ethernet (modemden DHCP)
    pub wan: String,
    /// İnternet veren Ethernet'ler (müşteri köprüsü)
    pub lan: Vec<String>,
    /// İnternet alan porta yazılan MAC: WAN başka porta geçse de modemdeki IP rezervasyonu korunur. Boş: dokunulmaz.
    pub wan_mac: String,
    /// Sağ/sol etiketi cihazla ters çıkarsa
    pub etiket_ters: bool,
    pub wifi: Wifi,
    /// İnternet alan portta PPPoE (kapalı: yalnızca DHCP)
    pub pppoe: Pppoe,
}

impl Default for Ag {
    fn default() -> Self {
        // ponytail: aynı donanım (J1900, sağ port enp3s0, sol port enp1s0); farklı kutuda kurulum sihirbazı belirleyecek
        Ag { wan: "enp3s0".into(), lan: vec!["enp1s0".into()], wan_mac: String::new(), etiket_ters: false, wifi: Wifi::default(), pppoe: Pppoe::default() }
    }
}

/// Dosya yolları (testte geçici klasör).
pub struct Yollar {
    pub ag: PathBuf,
    pub interfaces: PathBuf,
    pub nft: PathBuf,
    pub hostapd: PathBuf,
    pub issue: PathBuf,
    /// pppd bağlantı dosyası (kullanıcı adı ve parola içerir, 600); PPPoE kapalıyken bulunmaz
    pub peers: PathBuf,
    /// Kalıcı bekleme durumu (yeniden açılışta da geri alınabilsin)
    pub durum: PathBuf,
    /// Yavaşlatmanın uygulanmış plan izi: arayüzler yeniden kurulunca silinir (tc kökleri gitti)
    pub hiz_uygulanan: PathBuf,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar {
            ag: "/etc/wificorrect/ag.toml".into(),
            interfaces: "/etc/network/interfaces.d/wificorrect".into(),
            nft: "/etc/wificorrect/arayuzler.nft".into(),
            hostapd: "/etc/wificorrect/hostapd.conf".into(),
            issue: "/etc/issue.d/wificorrect.issue".into(),
            peers: "/etc/ppp/peers/wificorrect".into(),
            durum: "/var/lib/wificorrect".into(),
            hiz_uygulanan: crate::hiz::UYGULANAN_YOLU.into(),
        }
    }
    fn yedek(&self) -> PathBuf {
        self.durum.join("ag-yedek.toml")
    }
    fn bekliyor(&self) -> PathBuf {
        self.durum.join("ag-bekliyor")
    }
    pub fn yeni(&self) -> PathBuf {
        self.durum.join("ag-yeni.toml")
    }
}

pub fn load(path: &Path) -> Ag {
    fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, ag: &Ag) -> Result<(), String> {
    let text = toml::to_string_pretty(ag).map_err(|e| e.to_string())?;
    ortak::write_atomic(path, text.as_bytes()).map_err(|e| format!("{} yazılamadı: {e}", path.display()))
}

// ---------------------------------------------------------------- donanım
/// Cihazdaki Ethernet'ler: Ethernet 1 = adı büyük olan (bu kutuda sağdaki enp3s0).
pub fn ethernets(sys: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(sys)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("en") && sys.join(n).join("device").exists() && !sys.join(n).join("wireless").exists())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// "Ethernet 1 (sağ)" — iki portlu kutuda yan bilgisiyle.
pub fn label(ag: &Ag, eths: &[String], iface: &str) -> String {
    let i = eths.iter().position(|e| e == iface).unwrap_or(0);
    let side = match (eths.len(), i, ag.etiket_ters) {
        (2, 0, false) | (2, 1, true) => " (sağ)",
        (2, _, _) => " (sol)",
        _ => "",
    };
    format!("Ethernet {}{side}", i + 1)
}

/// Canlı kablo durumu: (bağlı mı, hız Mb/s, MAC).
pub fn link(sys: &Path, iface: &str) -> (bool, Option<u32>, String) {
    let read = |f: &str| fs::read_to_string(sys.join(iface).join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
    let up = read("carrier") == "1";
    (up, read("speed").parse::<i64>().ok().filter(|&s| up && s > 0).map(|s| s as u32), read("address"))
}

// ---------------------------------------------------------------- doğrulama
pub fn validate(ag: &Ag, eths: &[String]) -> Result<(), String> {
    if !eths.contains(&ag.wan) {
        return Err("İnternet alan port olarak tam bir Ethernet seçilmeli.".into());
    }
    if ag.lan.iter().any(|l| !eths.contains(l) || *l == ag.wan) {
        return Err("Bir port hem internet alıp hem veremez.".into());
    }
    if ag.lan.is_empty() && !ag.wifi.acik {
        return Err("En az bir port (Ethernet ya da Wi-Fi) internet vermeli.".into());
    }
    if !ag.wan_mac.is_empty() && ortak::norm_mac(&ag.wan_mac).as_deref() != Some(ag.wan_mac.as_str()) {
        return Err("MAC adresi geçersiz.".into());
    }
    let w = &ag.wifi;
    if w.acik {
        let n = w.ssid.chars().count();
        if !(1..=32).contains(&n) || w.ssid.len() > 32 || w.ssid.chars().any(char::is_control) || w.ssid.trim() != w.ssid {
            return Err("Wi-Fi adı 1-32 karakter olmalı (başta/sonda boşluk olmadan).".into());
        }
        if !w.sifre.is_empty() && (!(8..=63).contains(&w.sifre.len()) || !w.sifre.bytes().all(|b| (0x20..=0x7e).contains(&b))) {
            return Err("Wi-Fi şifresi 8-63 karakter olmalı (Türkçe harf olmadan) ya da boş (şifresiz).".into());
        }
        if !(1..=13).contains(&w.kanal) {
            return Err("Kanal 1-13 olmalı.".into());
        }
    }
    if w.iface.is_empty() || !w.iface.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Wi-Fi arayüzü geçersiz.".into());
    }
    pppoe_dogrula(&ag.pppoe)
}

/// PPPoE ayarının doğrulaması. Kullanıcı adı ve parola pppd seçenek dosyasına tırnak içinde yazılır: dosyaya satır sokulamasın
/// diye kontrol karakteri, tırnak ve ters eğik çizgi reddedilir. Boş bilgi yalnızca açıkken hatadır; yazılmış bilgi kapalıyken
/// de biçimli olmalı (ag.toml'a bozuk değer girmesin).
pub fn pppoe_dogrula(p: &Pppoe) -> Result<(), String> {
    if p.acik && (p.kullanici.is_empty() || p.parola.is_empty()) {
        return Err("PPPoE kullanıcı adı ve parolası boş olamaz.".into());
    }
    let temiz = |s: &str| s.len() <= 128 && !s.chars().any(|c| c.is_control() || c == '"' || c == '\\');
    if !temiz(&p.kullanici) || !temiz(&p.parola) {
        return Err("PPPoE kullanıcı adı ve parolası en çok 128 karakter olmalı; tırnak, ters eğik çizgi ve kontrol karakteri içeremez.".into());
    }
    if p.vlan > 4094 {
        return Err("VLAN 1-4094 arasında olmalı (boş: VLAN yok).".into());
    }
    if p.oncelik > 7 {
        return Err("VLAN önceliği 0-7 arasında olmalı.".into());
    }
    if !(1280..=pppoe::MTU).contains(&u32::from(p.mtu)) {
        return Err(format!("PPPoE MTU değeri 1280-{} arasında olmalı.", pppoe::MTU));
    }
    if !["otomatik", "pap", "chap"].contains(&p.kimlik.as_str()) {
        return Err("Kimlik doğrulama yöntemi otomatik, PAP ya da CHAP olmalı.".into());
    }
    // AC ve servis adı pppd'ye bir seçenek değeri olarak gider: boş ya da en çok 64 karakter A-Z a-z 0-9 . _ -
    let ad_gecerli = |s: &str| s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if !ad_gecerli(&p.ac_adi) {
        return Err("PPPoE AC adı en çok 64 karakter olmalı ve yalnızca harf, rakam, nokta, alt çizgi ve tire içerebilir.".into());
    }
    if !ad_gecerli(&p.servis_adi) {
        return Err("PPPoE servis adı en çok 64 karakter olmalı ve yalnızca harf, rakam, nokta, alt çizgi ve tire içerebilir.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------- üretilen dosyalar
fn prefix(subnet: &str) -> &str {
    subnet.split_once('/').map_or("24", |(_, p)| p)
}

pub fn interfaces_text(cfg: &Config, ag: &Ag) -> String {
    let mut s = String::from("# WifiCorrect üretir (panel → Portlar, /etc/wificorrect/ag.toml). Elle değiştirmeyin.\n\n");
    s.push_str(&format!("# İnternet alan port\nallow-hotplug {0}\niface {0} inet dhcp\n", ag.wan));
    if !ag.wan_mac.is_empty() {
        s.push_str(&format!("\thwaddress ether {}\n", ag.wan_mac));
    }
    let ports = if ag.lan.is_empty() { "none".to_string() } else { ag.lan.join(" ") };
    let v6: String = std::iter::once(BRIDGE).chain(ag.lan.iter().map(String::as_str)).map(|i| format!(" net.ipv6.conf.{i}.disable_ipv6=1")).collect();
    s.push_str(&format!(
        "\n# Müşteri köprüsü (internet veren Ethernet'ler; Wi-Fi açıksa hostapd ekler)\nauto {BRIDGE}\niface {BRIDGE} inet static\n\
         \taddress {}/{}\n\tbridge_ports {ports}\n\tbridge_stp off\n\tbridge_fd 0\n\tbridge_waitport 0\n\tpost-up sysctl -qw{v6}\n",
        cfg.main.router_ip,
        prefix(&cfg.main.subnet)
    ));
    s
}

/// WAN bir küme: internet alan Ethernet ve pppd'nin arayüzü. PPPoE kapalıyken de yazılır (olmayan arayüz adı eşleşmez, zararsız);
/// böylece guvenlik.nft'teki yönetim, iletim ve masquerade kuralları ile hiz.nft sayaçları değişmeden ikisini de kapsar.
pub fn nft_text(ag: &Ag) -> String {
    format!("# WifiCorrect üretir (ag.toml). guvenlik.nft bunu okur.\ndefine WAN = {{ \"{}\", \"{}\" }}\n", ag.wan, pppoe::ARAYUZ)
}

pub fn issue_text(ag: &Ag) -> String {
    let b = '\\';
    format!("WifiCorrect panel: https://panel.wificorrect.com (eslestirme: https://{b}4{{{0}}}:8443)   SSH: {b}4{{{0}}}\n\n", ag.wan)
}

pub fn hostapd_text(ag: &Ag) -> Option<String> {
    let w = &ag.wifi;
    if !w.acik {
        return None;
    }
    let mut s = format!(
        "# WifiCorrect üretir (ag.toml). Müşteri yayını; köprüye girer, istemciler birbirini göremez.\n\
         interface={}\nbridge={BRIDGE}\ndriver=nl80211\nssid={}\nutf8_ssid=1\ncountry_code=TR\nieee80211d=1\n\
         hw_mode=g\nchannel={}\nieee80211n=1\nwmm_enabled=1\nap_isolate=1\nauth_algs=1\n",
        w.iface, w.ssid, w.kanal
    );
    if !w.sifre.is_empty() {
        s.push_str(&format!("wpa=2\nwpa_key_mgmt=WPA-PSK\nrsn_pairwise=CCMP\nwpa_passphrase={}\n", w.sifre));
    }
    Some(s)
}

fn write_600(path: &Path, text: &str) -> Result<(), String> {
    ortak::write_atomic(path, text.as_bytes()).map_err(|e| format!("{} yazılamadı: {e}", path.display()))
}

/// Dosyayı siler; zaten yoksa hata değildir.
fn sil(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{} silinemedi: {e}", path.display())),
        _ => Ok(()),
    }
}

pub fn write_files(cfg: &Config, ag: &Ag, y: &Yollar) -> Result<(), String> {
    // pppd seçenek dosyasındaki her satır root'a bir seçenek verir: geçersiz (satır sokan) bilgi hiçbir dosya yazılmadan reddedilir
    if ag.pppoe.acik {
        pppoe_dogrula(&ag.pppoe)?;
    }
    write_600(&y.interfaces, &interfaces_text(cfg, ag))?;
    let _ = fs::set_permissions(&y.interfaces, std::os::unix::fs::PermissionsExt::from_mode(0o644));
    write_600(&y.nft, &nft_text(ag))?;
    write_600(&y.issue, &issue_text(ag))?;
    let _ = fs::set_permissions(&y.issue, std::os::unix::fs::PermissionsExt::from_mode(0o644));
    match hostapd_text(ag) {
        Some(t) => write_600(&y.hostapd, &t)?, // Wi-Fi şifresi içerir: 600
        None => sil(&y.hostapd)?,
    }
    if ag.pppoe.acik {
        write_600(&y.peers, &pppoe::peers_text(ag)) // kullanıcı adı ve parola içerir: 600
    } else {
        sil(&y.peers) // kapalıyken dosya hiç bulunmaz (pppd birimi başlamaz)
    }
}

/// `ip -o route show default` çıktısından arayüz adı.
pub fn rota_arayuzu(ip_route: &str) -> Option<String> {
    let mut it = ip_route.split_whitespace();
    while let Some(w) = it.next() {
        if w == "dev" {
            return it.next().map(String::from);
        }
    }
    None
}

/// Kurulum (paket postinst): ag.toml yoksa rolleri belirler ve dosyaları yazar; ağı geçirmez (kurucunun içinde çalışır).
/// İnternet alan: kurulumun internete çıktığı arayüz (varsayılan rota) → yoksa bu kutunun varsayılanı → yoksa Ethernet 1.
pub fn ilk(cfg: &Config, y: &Yollar, sys: &Path, rota: Option<&str>) -> Result<bool, String> {
    if y.ag.exists() {
        return Ok(false);
    }
    let eths = ethernets(sys);
    let v = Ag::default();
    let wan = match rota {
        Some(r) if eths.iter().any(|e| e == r) => r.to_string(),
        _ if eths.contains(&v.wan) && v.lan.iter().all(|l| eths.contains(l)) => v.wan.clone(),
        _ => eths.first().cloned().ok_or("Ethernet bulunamadı")?,
    };
    let ag = Ag { lan: eths.iter().filter(|e| **e != wan).cloned().collect(), wan, ..Ag::default() };
    if ag.lan.is_empty() {
        return Err("En az iki Ethernet gerekli (biri internet alır, biri misafirlere verir).".into());
    }
    // ag.toml en son: "kurulum tamam" işareti; yazım yarıda kalırsa yeniden deneme (dpkg --configure -a) boşa gitmez
    write_files(cfg, &ag, y)?;
    save(&y.ag, &ag)?;
    Ok(true)
}

/// `ip link show` değiştirilmiş MAC'te kalıcı adresi `permaddr` olarak yazar.
pub fn parse_permaddr(ip_link: &str) -> Option<String> {
    let mut it = ip_link.split_whitespace();
    it.find(|w| *w == "permaddr").and(it.next()).and_then(ortak::norm_mac)
}

/// WAN'ın DHCP istemcisini tutan birim (Debian ifupdown, udev ile allow-hotplug arayüzler için).
pub fn wan_unit(wan: &str) -> String {
    format!("ifup@{wan}.service")
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Ağı yeni ayara geçirir: eski ayarla arayüzleri indir, dosyaları yaz, kaldır, güvenlik duvarı / DHCP / Wi-Fi.
/// Dönen: kalan hatalar (boşsa sorunsuz).
pub fn switch(cfg: &Config, ag: &Ag, y: &Yollar, runner: &Runner) -> Vec<String> {
    let mut errs = vec![];
    runner(&cmd(&["ifdown", "-a", "--exclude=lo"])); // eski dosyalarla; başarısızlık önemsiz
    // İnternet almaktan müşteri tarafına dönen port WAN'ın MAC'ini taşımasın (ifupdown hwaddress'i geri almaz)
    for l in &ag.lan {
        if let Some(m) = parse_permaddr(&ortak::capture(&["ip", "link", "show", "dev", l])) {
            runner(&cmd(&["ip", "link", "set", "dev", l, "down"]));
            runner(&cmd(&["ip", "link", "set", "dev", l, "address", &m]));
        }
    }
    if let Err(e) = write_files(cfg, ag, y) {
        errs.push(e);
    }
    runner(&cmd(&["ifup", "-a"]));
    // WAN (allow-hotplug; ifup -a getirmez) kendi biriminde: dhcpcd buradan (geçici bir systemd işinden) başlatılsaydı
    // iş bitince systemd onu da öldürürdü ve IP kira sonunda (modemde 1 saat) düşerdi (2026-10-03'te yaşandı)
    runner(&cmd(&["systemctl", "restart", &wan_unit(&ag.wan)]));
    if !runner(&cmd(&["nft", "-f", "/etc/wificorrect/guvenlik.nft"])) {
        errs.push("güvenlik duvarı yüklenemedi".into());
    }
    // Hız sayaçları WAN adını yükleme anında alır → yeniden yükle (idempotent, sayaçlar kalır); köprü yeniden kurulduğu için
    // tc kökleri gitti → plan izi silinir, kaydedici sonraki turda yavaşlatmaları yeniden kurar. Hata ağ geçişini durdurmaz.
    runner(&cmd(&["nft", "-f", "/etc/wificorrect/hiz.nft"]));
    let _ = fs::remove_file(&y.hiz_uygulanan);
    if !runner(&cmd(&["systemctl", "restart", "dnsmasq"])) {
        errs.push("dnsmasq başlatılamadı".into());
    }
    let wifi = if ag.wifi.acik { "restart" } else { "stop" };
    if !runner(&cmd(&["systemctl", wifi, "wificorrect-wifi"])) && ag.wifi.acik {
        errs.push("Wi-Fi yayını başlatılamadı".into());
    }
    // PPPoE: pppd ayrı birimde. Açıksa yeni bağlantı dosyasıyla yeniden başlar; kapalıysa durur ve açılışta başlamaz.
    // switch'i kullanan her yol (panel geçişi, geri alma, fabrika dönüşü) pppd'yi de ayara getirir.
    errs.extend(pppoe::birim_uygula(ag, runner));
    errs
}

fn audit(cfg: &Config, olay: &str, ek: &str) {
    ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso(ortak::wall())).set("ek", ek));
}

/// Bekleyen değişiklik: (son onay anı, geri alma birimi).
pub fn pending(y: &Yollar) -> Option<(f64, String)> {
    let t = fs::read_to_string(y.bekliyor()).ok()?;
    let (d, unit) = t.trim().split_once(' ')?;
    Some((d.parse().ok()?, unit.to_string()))
}

/// `ctl ag-gecis <yeni.toml>`: önce geri alma zamanlayıcısı, sonra geçiş.
pub fn gecis(cfg: &Config, y: &Yollar, new_path: &Path, now: f64, runner: &Runner) -> Result<Vec<String>, String> {
    if pending(y).is_some() {
        return Err("Onay bekleyen bir değişiklik var.".into());
    }
    let new = fs::read_to_string(new_path).map_err(|e| e.to_string())?;
    let ag: Ag = toml::from_str(&new).map_err(|e| e.to_string())?;
    ortak::mkdirs(&y.durum).map_err(|e| e.to_string())?;
    save(&y.yedek(), &load(&y.ag))?;
    let unit = format!("wfc-ag-geri-al-{}", ortak::random_hex(4));
    let timer = cmd(&["systemd-run", "--unit", &unit, "--on-active", &format!("{ONAY_SN}s"), "--timer-property=AccuracySec=1s", "/usr/local/bin/wificorrect", "ctl", "ag-geri-al", "sure"]);
    if !runner(&timer) {
        let _ = fs::remove_file(y.yedek());
        return Err("Geri alma zamanlayıcısı kurulamadı; değişiklik uygulanmadı.".into());
    }
    write_600(&y.bekliyor(), &format!("{} {unit}\n", now + ONAY_SN as f64))?;
    save(&y.ag, &ag)?;
    let _ = fs::remove_file(new_path);
    let errs = switch(cfg, &ag, y, runner);
    audit(cfg, "AG_UYGULANDI", &format!("wan={} lan={} wifi={}{}", ag.wan, ag.lan.join(","), ag.wifi.acik, if errs.is_empty() { String::new() } else { format!(" hata={}", errs.join(",")) }));
    Ok(errs)
}

/// Giriş sayfasındaki Servis bölümü (`ctl pppoe-servis`): yalnızca PPPoE ayarını yeni değerle değiştirir ve uygular.
/// **Geri alma zamanlayıcısı kurulmaz**: Servis cihaz internetsizken kullanılır; geri almak teknisyenin düzelttiği bilgiyi yanlış
/// eskisine döndürürdü. Boş parola = eskisi kalır (parola hiçbir yerde geri gösterilmez). Geçersiz ayar (ya da onay bekleyen bir
/// port değişikliği: süresi dolunca geri alma yedeği bu ayarı ezerdi) `Err`, hiçbir şey değişmez. Dönen: kalan hatalar.
/// Ağı yeniden kurmaz (`switch` değil): ag.toml, bağlantı dosyası, güvenlik duvarı kuralları, pppd birimi.
pub fn pppoe_uygula(cfg: &Config, y: &Yollar, yeni: &Pppoe, runner: &Runner) -> Result<Vec<String>, String> {
    if pending(y).is_some() {
        return Err("Bir port değişikliği onay bekliyor (en çok 3 dakika); onaylanınca ya da geri alınınca yeniden deneyin.".into());
    }
    let mut ag = load(&y.ag);
    let mut p = yeni.clone();
    if p.parola.is_empty() {
        p.parola = ag.pppoe.parola.clone();
    }
    pppoe_dogrula(&p)?;
    ag.pppoe = p;
    save(&y.ag, &ag)?;
    let mut errs = vec![];
    if let Err(e) = write_files(cfg, &ag, y) {
        errs.push(e);
    }
    // Yükseltilmiş cihazda çekirdeğe yüklü kurallar eski tek adlı WAN ile kuruldu (arayuzler.nft ancak şimdi küme oldu): yeniden
    // yüklenmezse wfcppp'den gelen/giden trafik iletilmez ve NAT'lanmaz (switch ile aynı yükleme; hız sayaçları da WAN adını alır)
    if !runner(&cmd(&["nft", "-f", "/etc/wificorrect/guvenlik.nft"])) {
        errs.push("güvenlik duvarı yüklenemedi".into());
    }
    runner(&cmd(&["nft", "-f", "/etc/wificorrect/hiz.nft"]));
    errs.extend(pppoe::birim_uygula(&ag, runner));
    Ok(errs)
}

/// `ctl ag-onayla`: zamanlayıcıyı durdur, yedeği sil.
pub fn onayla(cfg: &Config, y: &Yollar, runner: &Runner) -> Result<(), String> {
    let (_, unit) = pending(y).ok_or("Onay bekleyen değişiklik yok.")?;
    runner(&cmd(&["systemctl", "stop", &format!("{unit}.timer")]));
    let _ = fs::remove_file(y.bekliyor());
    let _ = fs::remove_file(y.yedek());
    audit(cfg, "AG_ONAYLANDI", "");
    Ok(())
}

/// `ctl ag-geri-al <neden>`: yedekteki ayara dön ve ağı ona geçir. `acilis`: ağ henüz kalkmadı, yalnızca dosyalar.
pub fn geri_al(cfg: &Config, y: &Yollar, neden: &str, runner: &Runner) -> Result<Vec<String>, String> {
    let Some((_, unit)) = pending(y) else { return Ok(vec![]) };
    let old = load(&y.yedek());
    if neden != "sure" {
        runner(&cmd(&["systemctl", "stop", &format!("{unit}.timer")]));
    }
    save(&y.ag, &old)?;
    let errs = if neden == "acilis" { write_files(cfg, &old, y).err().into_iter().collect() } else { switch(cfg, &old, y, runner) };
    let _ = fs::remove_file(y.bekliyor());
    let _ = fs::remove_file(y.yedek());
    audit(cfg, "AG_GERI_ALINDI", &format!("neden={neden}{}", if errs.is_empty() { String::new() } else { format!(" hata={}", errs.join(",")) }));
    Ok(errs)
}

/// Değişiklik özeti (denetim ve panel mesajı için).
pub fn describe(ag: &Ag, eths: &[String]) -> String {
    let role = |e: &String| {
        let r = if *e == ag.wan { "alır" } else if ag.lan.contains(e) { "verir" } else { "kapalı" };
        format!("{} {r}", label(ag, eths, e))
    };
    let wifi = if ag.wifi.acik { format!("Wi-Fi verir ({}{})", ag.wifi.ssid, if ag.wifi.sifre.is_empty() { ", şifresiz" } else { "" }) } else { "Wi-Fi kapalı".into() };
    let mut s = eths.iter().map(role).chain(std::iter::once(wifi)).collect::<Vec<_>>().join(", ");
    if ag.pppoe.acik {
        // kullanıcı adı ve parola denetime yazılmaz
        s.push_str(&match ag.pppoe.vlan {
            0 => ", PPPoE açık".to_string(),
            v => format!(", PPPoE açık (VLAN {v})"),
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn setup() -> (Config, Yollar, PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-ag-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cfg = Config::parse(&format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n", root.display())).unwrap();
        let y = Yollar {
            ag: root.join("ag.toml"),
            interfaces: root.join("interfaces"),
            nft: root.join("arayuzler.nft"),
            hostapd: root.join("hostapd.conf"),
            issue: root.join("issue"),
            peers: root.join("peers"),
            durum: root.join("durum"),
            hiz_uygulanan: root.join("hiz_uygulanan"),
        };
        (cfg, y, root)
    }

    fn eths() -> Vec<String> {
        vec!["enp3s0".into(), "enp1s0".into()]
    }

    #[test]
    fn validation_and_labels() {
        let e = eths();
        let mut ag = Ag::default();
        assert!(validate(&ag, &e).is_ok());
        assert_eq!((label(&ag, &e, "enp3s0"), label(&ag, &e, "enp1s0")), ("Ethernet 1 (sağ)".into(), "Ethernet 2 (sol)".into()));
        ag.etiket_ters = true;
        assert_eq!(label(&ag, &e, "enp3s0"), "Ethernet 1 (sol)");
        ag.lan = vec![];
        assert!(validate(&ag, &e).unwrap_err().contains("En az bir"));
        ag.wifi = Wifi { acik: true, ssid: "Bocafe Misafir".into(), ..Wifi::default() };
        assert!(validate(&ag, &e).is_ok()); // yalnızca Wi-Fi verir, şifresiz
        ag.wifi.sifre = "kisa".into();
        assert!(validate(&ag, &e).is_err());
        ag.wifi.sifre = "şifreşifre".into();
        assert!(validate(&ag, &e).is_err());
        ag.wifi.sifre = "guclu-sifre".into();
        ag.wifi.ssid = "ad\nkanal=1".into();
        assert!(validate(&ag, &e).is_err()); // dosyaya satır sokulamaz
        ag.wifi.ssid = "Ad".into();
        ag.wan = "enp9s9".into();
        assert!(validate(&ag, &e).is_err());
        ag.wan = "enp1s0".into();
        ag.lan = vec!["enp1s0".into()];
        assert!(validate(&ag, &e).is_err());
    }

    #[test]
    fn permanent_mac() {
        let out = "2: enp1s0: <UP> mtu 1500\n    link/ether 00:0e:c4:ce:a0:9b brd ff:ff:ff:ff:ff:ff permaddr 00:0E:C4:CE:A0:9A\n";
        assert_eq!(parse_permaddr(out).as_deref(), Some("00:0e:c4:ce:a0:9a"));
        assert_eq!(parse_permaddr("link/ether 00:0e:c4:ce:a0:9a brd ff:ff:ff:ff:ff:ff"), None);
    }

    #[test]
    fn generated_files() {
        let (cfg, y, _) = setup();
        let mut ag = Ag { wan: "enp1s0".into(), lan: vec!["enp3s0".into()], wan_mac: "00:0e:c4:ce:a0:9b".into(), ..Ag::default() };
        let t = interfaces_text(&cfg, &ag);
        assert!(t.contains("allow-hotplug enp1s0\niface enp1s0 inet dhcp\n\thwaddress ether 00:0e:c4:ce:a0:9b"));
        assert!(t.contains("address 10.50.0.1/24\n\tbridge_ports enp3s0\n") && t.contains("net.ipv6.conf.enp3s0.disable_ipv6=1"));
        assert_eq!(nft_text(&ag), "# WifiCorrect üretir (ag.toml). guvenlik.nft bunu okur.\ndefine WAN = { \"enp1s0\", \"wfcppp\" }\n");
        assert!(issue_text(&ag).starts_with("WifiCorrect panel: https://panel.wificorrect.com (eslestirme: https://\\4{enp1s0}:8443)"));
        assert!(hostapd_text(&ag).is_none());
        ag.lan.clear();
        ag.wifi = Wifi { acik: true, ssid: "Bocafe".into(), sifre: "guclu-sifre".into(), kanal: 11, ..Wifi::default() };
        assert!(interfaces_text(&cfg, &ag).contains("bridge_ports none"));
        let h = hostapd_text(&ag).unwrap();
        assert!(h.contains("bridge=br-hotspot\n") && h.contains("ssid=Bocafe\n") && h.contains("channel=11\n") && h.contains("ap_isolate=1") && h.contains("wpa_passphrase=guclu-sifre"));
        ag.wifi.sifre.clear();
        assert!(!hostapd_text(&ag).unwrap().contains("wpa"));
        write_files(&cfg, &ag, &y).unwrap();
        assert!(y.hostapd.exists());
        ag.wifi.acik = false;
        ag.lan = vec!["enp3s0".into()];
        write_files(&cfg, &ag, &y).unwrap();
        assert!(!y.hostapd.exists() && y.interfaces.exists() && y.nft.exists());
    }

    #[test]
    fn change_confirm_and_rollback() {
        let (cfg, y, root) = setup();
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let runner = move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            true
        };
        let old = Ag::default();
        save(&y.ag, &old).unwrap();
        let new = Ag { wan: "enp1s0".into(), lan: vec!["enp3s0".into()], wan_mac: "00:0e:c4:ce:a0:9b".into(), ..Ag::default() };
        let staged = root.join("yeni.toml");
        save(&staged, &new).unwrap();
        fs::write(&y.hiz_uygulanan, "eski plan").unwrap();
        assert_eq!(gecis(&cfg, &y, &staged, 1000.0, &runner), Ok(vec![]));
        assert_eq!(load(&y.ag), new);
        assert!(!y.hiz_uygulanan.exists()); // tc kökleri yeniden kurulsun
        let (deadline, unit) = pending(&y).unwrap();
        assert_eq!(deadline, 1180.0);
        {
            let c = calls.lock().unwrap();
            let timer = c.iter().position(|x| x.starts_with("systemd-run --unit wfc-ag-geri-al-")).unwrap();
            let down = c.iter().position(|x| x == "ifdown -a --exclude=lo").unwrap();
            assert!(timer < down); // önce zamanlayıcı, sonra ağ
            assert!(c.iter().any(|x| x == "systemctl restart ifup@enp1s0.service") && c.iter().any(|x| x == "systemctl stop wificorrect-wifi"));
            assert!(!c.iter().any(|x| x == "ifup enp1s0")); // dhcpcd geçici işin içinde başlamasın
            let guv = c.iter().position(|x| x == "nft -f /etc/wificorrect/guvenlik.nft").unwrap();
            let hiz = c.iter().position(|x| x == "nft -f /etc/wificorrect/hiz.nft").unwrap();
            assert!(guv < hiz); // yeni WAN adıyla sayaçlar
        }
        assert!(fs::read_to_string(&y.nft).unwrap().contains("\"enp1s0\""));
        save(&staged, &old).unwrap();
        assert!(gecis(&cfg, &y, &staged, 1001.0, &runner).is_err()); // onay beklerken yeni değişiklik yok

        // süre doldu: eski ayar geri gelir
        assert_eq!(geri_al(&cfg, &y, "sure", &runner), Ok(vec![]));
        assert_eq!(load(&y.ag), old);
        assert!(pending(&y).is_none() && fs::read_to_string(&y.nft).unwrap().contains("\"enp3s0\""));

        // onaylanan değişiklik kalır, zamanlayıcı durur
        save(&staged, &new).unwrap();
        gecis(&cfg, &y, &staged, 2000.0, &runner).unwrap();
        let (_, unit2) = pending(&y).unwrap();
        assert_ne!(unit, unit2);
        onayla(&cfg, &y, &runner).unwrap();
        assert!(calls.lock().unwrap().iter().any(|x| *x == format!("systemctl stop {unit2}.timer")));
        assert!(pending(&y).is_none() && load(&y.ag) == new);
        assert_eq!(geri_al(&cfg, &y, "elle", &runner), Ok(vec![])); // bekleyen yoksa bir şey yapmaz
        assert_eq!(load(&y.ag), new);

        // açılışta bekleyen değişiklik: yalnızca dosyalar geri yazılır, ağ komutu çalışmaz
        save(&staged, &old).unwrap();
        gecis(&cfg, &y, &staged, 3000.0, &runner).unwrap();
        calls.lock().unwrap().clear();
        geri_al(&cfg, &y, "acilis", &runner).unwrap();
        assert_eq!(load(&y.ag), new);
        assert!(calls.lock().unwrap().iter().all(|x| !x.starts_with("if")));
        let audit = fs::read_to_string(ortak::day_file(&cfg.main.log_root, ortak::day_of(&ortak::now_iso(ortak::wall())), "denetim.csv")).unwrap();
        assert!(audit.contains("AG_UYGULANDI") && audit.contains("AG_GERI_ALINDI") && audit.contains("AG_ONAYLANDI") && audit.contains("neden=acilis"));
    }

    impl Yollar {
        fn test(d: &Path) -> Yollar {
            Yollar { ag: d.join("ag.toml"), interfaces: d.join("interfaces"), nft: d.join("arayuzler.nft"), hostapd: d.join("hostapd.conf"), issue: d.join("issue"), peers: d.join("peers"), durum: d.join("durum"), hiz_uygulanan: d.join("hiz_uygulanan") }
        }
    }

    #[test]
    fn ilk_rota_ve_varsayilan() {
        fn sys_kur(ad: &str, adlar: &[&str]) -> std::path::PathBuf {
            let d = std::env::temp_dir().join(format!("wfc-ag-ilk-{}-{ad}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            for n in adlar {
                std::fs::create_dir_all(d.join("net").join(n).join("device")).unwrap();
            }
            d
        }
        assert_eq!(rota_arayuzu("default via 192.168.1.1 dev eno1 proto dhcp src 192.168.1.50 metric 100
").as_deref(), Some("eno1"));
        assert_eq!(rota_arayuzu(""), None);
        let cfg = Config::default();
        // farklı donanım: kurulumun internete çıktığı arayüz internet alır
        let d = sys_kur("farkli", &["eno1", "enp2s0"]);
        let y = Yollar::test(&d);
        assert!(ilk(&cfg, &y, &d.join("net"), Some("eno1")).unwrap());
        let ag = load(&y.ag);
        assert_eq!((ag.wan.as_str(), ag.lan.clone()), ("eno1", vec!["enp2s0".to_string()]));
        assert!(std::fs::read_to_string(&y.interfaces).unwrap().contains("bridge_ports enp2s0"));
        assert!(!ilk(&cfg, &y, &d.join("net"), Some("enp2s0")).unwrap()); // ag.toml var: dokunulmaz
        // rota yok, J1900 adları var: varsayılan
        let d = sys_kur("j1900", &["enp1s0", "enp3s0"]);
        let y = Yollar::test(&d);
        assert!(ilk(&cfg, &y, &d.join("net"), None).unwrap());
        assert_eq!(load(&y.ag).wan, "enp3s0");
        // tek Ethernet: hata
        let d = sys_kur("tek", &["eno1"]);
        assert!(ilk(&cfg, &Yollar::test(&d), &d.join("net"), Some("eno1")).is_err());
    }

    #[test]
    fn ilk_yazim_hatasinda_ag_toml_kalmaz() {
        let d = std::env::temp_dir().join(format!("wfc-ag-ilk-{}-hata", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for n in ["eno1", "enp2s0", "br-hotspot"] {
            fs::create_dir_all(d.join("net").join(n).join("device")).unwrap();
        }
        let cfg = Config::default();
        // interfaces yolunun üstü sıradan dosya: üst dizin oluşturulamaz, write_files başarısız
        fs::write(d.join("engel"), "x").unwrap();
        let mut y = Yollar::test(&d);
        let iyi = y.interfaces.clone();
        y.interfaces = d.join("engel").join("interfaces");
        assert!(ilk(&cfg, &y, &d.join("net"), Some("eno1")).is_err());
        assert!(!y.ag.exists()); // ag.toml "tamamlandı" işareti: yazım başarısızsa kalmaz, yeniden deneme çalışır
        y.interfaces = iyi;
        assert!(ilk(&cfg, &y, &d.join("net"), Some("eno1")).unwrap());
        assert!(y.ag.exists() && y.interfaces.exists());
        // rota Ethernet değil (köprü): varsayılan koldan geçer, ilk Ethernet internet alır
        let d2 = d.join("kopru");
        let y2 = Yollar::test(&d2);
        assert!(ilk(&cfg, &y2, &d.join("net"), Some("br-hotspot")).unwrap());
        assert_eq!(load(&y2.ag).wan, "enp2s0");
    }

    // ---------------------------------------------------------------- PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md)
    fn pppoe_acik() -> Pppoe {
        Pppoe { acik: true, kullanici: "kullanici1".into(), parola: "parola1".into(), vlan: 35, ..Pppoe::default() }
    }

    #[test]
    fn eski_ag_toml_pppoe_kapali() {
        let (_, y, _) = setup();
        // 0.4.16'nın yazdığı ag.toml: [pppoe] bölümü yok
        fs::write(&y.ag, "wan = \"enp1s0\"\nlan = [\"enp3s0\"]\nwan_mac = \"\"\netiket_ters = false\n\n[wifi]\nacik = false\niface = \"wlp2s0\"\nssid = \"\"\nsifre = \"\"\nkanal = 6\n").unwrap();
        let ag = load(&y.ag);
        assert_eq!(ag.wan, "enp1s0"); // dosya okundu (varsayılana düşmedi)
        assert_eq!(ag.pppoe, Pppoe::default());
        assert!(!ag.pppoe.acik && ag.pppoe.mtu == 1492 && ag.pppoe.kimlik == "otomatik" && ag.pppoe.vlan == 0 && ag.pppoe.oncelik == 0);
        // yeniden yazılınca bölüm varsayılanlarla eklenir
        save(&y.ag, &ag).unwrap();
        let t = fs::read_to_string(&y.ag).unwrap();
        let bolum = t.split("[pppoe]").nth(1).unwrap_or_else(|| panic!("[pppoe] bölümü yazılmadı:\n{t}"));
        assert!(bolum.contains("acik = false") && bolum.contains("mtu = 1492") && bolum.contains("kimlik = \"otomatik\""), "{t}");
        assert_eq!(load(&y.ag), ag);
        // bölüm kısmi yazılmışsa yazılmayan alanlar varsayılan kalır
        let k: Ag = toml::from_str("[pppoe]\nacik = true\nkullanici = \"k\"\nparola = \"p\"\n").unwrap();
        assert!(k.pppoe.acik && k.pppoe.mtu == 1492 && k.pppoe.kimlik == "otomatik" && k.pppoe.vlan == 0 && k.wan == "enp3s0");
    }

    #[test]
    fn pppoe_dogrulama() {
        let e = eths();
        let dene = |p: Pppoe| validate(&Ag { pppoe: p, ..Ag::default() }, &e);
        let gecerli = || Pppoe { acik: true, kullanici: "5321234567@turk.net".into(), parola: "parola-1".into(), ..Pppoe::default() };
        assert_eq!(dene(gecerli()), Ok(()));
        assert_eq!(dene(Pppoe { vlan: 35, oncelik: 0, mtu: 1492, kimlik: "pap".into(), ..gecerli() }), Ok(()));
        // kapalıyken boş bilgiler sorun değil (varsayılan hâl)
        assert_eq!(dene(Pppoe::default()), Ok(()));

        // açıkken kullanıcı adı ve parola boş olamaz
        let bos = "PPPoE kullanıcı adı ve parolası boş olamaz.";
        assert_eq!(dene(Pppoe { kullanici: String::new(), ..gecerli() }), Err(bos.into()));
        assert_eq!(dene(Pppoe { parola: String::new(), ..gecerli() }), Err(bos.into()));

        // en çok 128 bayt; tırnak, ters eğik çizgi ve kontrol karakteri yok (pppd seçenek dosyasına satır sokulamaz)
        let karakter = "PPPoE kullanıcı adı ve parolası en çok 128 karakter olmalı; tırnak, ters eğik çizgi ve kontrol karakteri içeremez.";
        assert_eq!(dene(Pppoe { kullanici: "k".repeat(128), parola: "p".repeat(128), ..gecerli() }), Ok(()));
        assert_eq!(dene(Pppoe { parola: "p".repeat(129), ..gecerli() }), Err(karakter.into()));
        assert_eq!(dene(Pppoe { kullanici: "k".repeat(129), ..gecerli() }), Err(karakter.into()));
        assert_eq!(dene(Pppoe { kullanici: "ş".repeat(65), ..gecerli() }), Err(karakter.into())); // 130 bayt
        for kotu in ["a\"b", "a\\b", "a\nb", "a\rb", "a\tb", "a\0b", "a\u{7f}b", "\"", "\\"] {
            assert_eq!(dene(Pppoe { kullanici: kotu.into(), ..gecerli() }), Err(karakter.into()), "kullanıcı {kotu:?}");
            assert_eq!(dene(Pppoe { parola: kotu.into(), ..gecerli() }), Err(karakter.into()), "parola {kotu:?}");
        }
        assert_eq!(dene(Pppoe { kullanici: "ş@alan çok.net".into(), parola: "boşluk # ve 'tek tırnak' olur".into(), ..gecerli() }), Ok(()));
        // kapalıyken boş bilgi serbest, ama yazılmış bilgi yine de biçimli olmalı (ag.toml'a bozuk değer girmez)
        assert_eq!(dene(Pppoe { acik: false, kullanici: "a\"b".into(), ..Pppoe::default() }), Err(karakter.into()));

        // VLAN: 0 (yok) ya da 1-4094
        let vlan = "VLAN 1-4094 arasında olmalı (boş: VLAN yok).";
        for v in [0, 1, 35, 4094] {
            assert_eq!(dene(Pppoe { vlan: v, ..gecerli() }), Ok(()), "vlan {v}");
        }
        for v in [4095, 5000, u16::MAX] {
            assert_eq!(dene(Pppoe { vlan: v, ..gecerli() }), Err(vlan.into()), "vlan {v}");
        }
        // 802.1p önceliği 0-7
        assert_eq!(dene(Pppoe { vlan: 35, oncelik: 7, ..gecerli() }), Ok(()));
        assert_eq!(dene(Pppoe { vlan: 35, oncelik: 8, ..gecerli() }), Err("VLAN önceliği 0-7 arasında olmalı.".into()));
        // MTU 1280-1492
        let mtu = "PPPoE MTU değeri 1280-1492 arasında olmalı.";
        for m in [1280, 1400, 1492] {
            assert_eq!(dene(Pppoe { mtu: m, ..gecerli() }), Ok(()), "mtu {m}");
        }
        for m in [0, 1279, 1493, 1500] {
            assert_eq!(dene(Pppoe { mtu: m, ..gecerli() }), Err(mtu.into()), "mtu {m}");
        }
        // kimlik doğrulama: otomatik | pap | chap
        for k in ["otomatik", "pap", "chap"] {
            assert_eq!(dene(Pppoe { kimlik: k.into(), ..gecerli() }), Ok(()), "kimlik {k}");
        }
        for k in ["mschap", "PAP", "", "eap"] {
            assert_eq!(dene(Pppoe { kimlik: k.into(), ..gecerli() }), Err("Kimlik doğrulama yöntemi otomatik, PAP ya da CHAP olmalı.".into()), "kimlik {k:?}");
        }
        // AC adı ve servis adı: boş ya da en çok 64 karakter A-Z a-z 0-9 . _ -
        let ac = "PPPoE AC adı en çok 64 karakter olmalı ve yalnızca harf, rakam, nokta, alt çizgi ve tire içerebilir.";
        let servis = "PPPoE servis adı en çok 64 karakter olmalı ve yalnızca harf, rakam, nokta, alt çizgi ve tire içerebilir.";
        assert_eq!(dene(Pppoe { ac_adi: "AC-1.x_y".into(), servis_adi: "internet".into(), ..gecerli() }), Ok(()));
        assert_eq!(dene(Pppoe { ac_adi: "a".repeat(64), servis_adi: "s".repeat(64), ..gecerli() }), Ok(()));
        assert_eq!(dene(Pppoe { ac_adi: "a".repeat(65), ..gecerli() }), Err(ac.into()));
        assert_eq!(dene(Pppoe { servis_adi: "s".repeat(65), ..gecerli() }), Err(servis.into()));
        for kotu in ["a b", "a\"b", "a\nb", "ağ", "a/b", "a;b", "a#b"] {
            assert_eq!(dene(Pppoe { ac_adi: kotu.into(), ..gecerli() }), Err(ac.into()), "ac {kotu:?}");
            assert_eq!(dene(Pppoe { servis_adi: kotu.into(), ..gecerli() }), Err(servis.into()), "servis {kotu:?}");
        }
    }

    #[test]
    fn nft_wan_kumesi() {
        // PPPoE kapalıyken de küme: olmayan arayüz adı eşleşmez, zararsız (guvenlik.nft ve hiz.nft değişmeden iki arayüzü kapsar)
        assert_eq!(nft_text(&Ag::default()), "# WifiCorrect üretir (ag.toml). guvenlik.nft bunu okur.\ndefine WAN = { \"enp3s0\", \"wfcppp\" }\n");
        let acik = Ag { wan: "eno1".into(), pppoe: pppoe_acik(), ..Ag::default() };
        assert_eq!(nft_text(&acik), "# WifiCorrect üretir (ag.toml). guvenlik.nft bunu okur.\ndefine WAN = { \"eno1\", \"wfcppp\" }\n");
    }

    #[test]
    fn peers_dosyasi_yalniz_acikken() {
        use std::os::unix::fs::PermissionsExt;
        let (cfg, y, _) = setup();
        let mut ag = Ag { pppoe: pppoe_acik(), ..Ag::default() };
        write_files(&cfg, &ag, &y).unwrap();
        assert_eq!(fs::metadata(&y.peers).unwrap().permissions().mode() & 0o777, 0o600); // parola içerir
        let t = fs::read_to_string(&y.peers).unwrap();
        assert_eq!(t, crate::pppoe::peers_text(&ag));
        assert!(t.contains("password \"parola1\"\n") && t.contains("nic-enp3s0.35\n"));
        assert!(y.interfaces.exists() && y.nft.exists()); // diğer dosyalar yine yazılır
        // ayar değişince dosya yeniden yazılır
        ag.pppoe.vlan = 0;
        write_files(&cfg, &ag, &y).unwrap();
        assert!(fs::read_to_string(&y.peers).unwrap().contains("nic-enp3s0\n"));
        // kapatınca dosya silinir; zaten yokken de hata vermez
        ag.pppoe.acik = false;
        write_files(&cfg, &ag, &y).unwrap();
        assert!(!y.peers.exists());
        write_files(&cfg, &ag, &y).unwrap();
        assert!(!y.peers.exists());
        // PPPoE hiç açılmamış cihaz: dosya yok, hata yok
        let (cfg2, y2, _) = setup();
        write_files(&cfg2, &Ag::default(), &y2).unwrap();
        assert!(!y2.peers.exists());
    }

    #[test]
    fn peers_dosyasina_gecersiz_ayar_yazilmaz() {
        // pppd seçenek dosyasındaki her satır root'a bir seçenek verir: geçersiz (satır sokan) bilgi dosyaya hiç girmemeli
        let (cfg, y, _) = setup();
        let kotu = Ag { pppoe: Pppoe { kullanici: "kullanici1\nconnect /bin/sh".into(), ..pppoe_acik() }, ..Ag::default() };
        assert!(write_files(&cfg, &kotu, &y).is_err());
        assert!(!y.peers.exists() && !y.interfaces.exists()); // hiçbir dosya yazılmadan reddedilir
    }

    #[test]
    fn describe_pppoe() {
        let e = eths();
        let kapali = describe(&Ag::default(), &e);
        assert!(!kapali.contains("PPPoE"), "{kapali}");
        let ag = Ag { pppoe: Pppoe { kullanici: "5321234567@turk.net".into(), parola: "gizli-parola".into(), ..pppoe_acik() }, ..Ag::default() };
        let d = describe(&ag, &e);
        assert_eq!(d, format!("{kapali}, PPPoE açık (VLAN 35)"));
        assert!(!d.contains("turk.net") && !d.contains("gizli-parola")); // kullanıcı adı ve parola denetime yazılmaz
        // VLAN yoksa yalnızca "PPPoE açık"
        assert_eq!(describe(&Ag { pppoe: Pppoe { vlan: 0, ..pppoe_acik() }, ..Ag::default() }, &e), format!("{kapali}, PPPoE açık"));
        // kapalıyken (bilgiler dolu olsa da) çıktı değişmez
        assert_eq!(describe(&Ag { pppoe: Pppoe { acik: false, ..pppoe_acik() }, ..Ag::default() }, &e), kapali);
    }

    // ---------------------------------------------------------------- PPPoE birimi ve Servis yolu
    /// Sahte komut çalıştırıcı: çağrıları sırayla toplar; `basarisiz` listesindeki önekle başlayan komut false döner.
    fn sahte(basarisiz: &'static [&'static str]) -> (impl Fn(&[String]) -> bool + Send + Sync, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::default();
        let c2 = calls.clone();
        let r = move |c: &[String]| {
            let s = c.join(" ");
            c2.lock().unwrap().push(s.clone());
            !basarisiz.iter().any(|b| s.starts_with(b))
        };
        (r, calls)
    }

    /// `ara` listesinde, her biri öncekinden sonra gelen komutların yerleri; bulunamayan komut panik (mesajda tüm liste).
    fn sirayla(calls: &[String], ara: &[&str]) -> Vec<usize> {
        let mut yerler: Vec<usize> = vec![];
        for a in ara {
            let basla = yerler.last().map_or(0, |y| y + 1);
            let y = calls[basla..].iter().position(|x| x == a).unwrap_or_else(|| panic!("{a:?} (sırayla) yok: {calls:?}")) + basla;
            yerler.push(y);
        }
        yerler
    }

    #[test]
    fn switch_pppoe_birimi() {
        let (cfg, y, _) = setup();
        let acik = Ag { pppoe: pppoe_acik(), ..Ag::default() };
        // açık: bağlantı dosyası yazılır, Wi-Fi adımından sonra birim açılışta başlayacak biçimde etkinleştirilip yeniden başlatılır
        let (r, c) = sahte(&[]);
        assert_eq!(switch(&cfg, &acik, &y, &r), Vec::<String>::new());
        let calls = c.lock().unwrap().clone();
        sirayla(&calls, &["systemctl restart dnsmasq", "systemctl stop wificorrect-wifi", "systemctl enable wificorrect-pppoe.service", "systemctl restart wificorrect-pppoe.service"]);
        assert!(!calls.iter().any(|x| x.contains("disable")), "{calls:?}");
        assert!(y.peers.exists());
        // kapalı: birim durur ve açılışta başlamaz (pppd'nin eski oturumu kalmaz); bağlantı dosyası gider
        let (r, c) = sahte(&[]);
        assert_eq!(switch(&cfg, &Ag::default(), &y, &r), Vec::<String>::new());
        let calls = c.lock().unwrap().clone();
        sirayla(&calls, &["systemctl stop wificorrect-wifi", "systemctl disable --now wificorrect-pppoe.service"]);
        assert!(!calls.iter().any(|x| x.contains("restart wificorrect-pppoe")) && !y.peers.exists(), "{calls:?}");
        // birim başlatılamazsa geçişin geri kalanı yine yapılmıştır; hata listede
        let (r, c) = sahte(&["systemctl restart wificorrect-pppoe"]);
        assert_eq!(switch(&cfg, &acik, &y, &r), vec!["PPPoE başlatılamadı".to_string()]);
        sirayla(&c.lock().unwrap(), &["systemctl restart dnsmasq", "systemctl restart wificorrect-pppoe.service"]);
    }

    #[test]
    fn pppoe_birimi_gecis_ve_geri_alma_yolunda() {
        // switch'i kullanan her yol (panel geçişi, geri alma, fabrika dönüşü) pppd birimini de yönetir
        let (cfg, y, root) = setup();
        let (r, c) = sahte(&[]);
        let acik = Ag { pppoe: pppoe_acik(), ..Ag::default() };
        save(&y.ag, &acik).unwrap();
        let staged = root.join("yeni.toml");
        save(&staged, &Ag::default()).unwrap(); // panelden PPPoE kapatıldı
        assert_eq!(gecis(&cfg, &y, &staged, 1000.0, &r), Ok(vec![]));
        assert!(c.lock().unwrap().iter().any(|x| x == "systemctl disable --now wificorrect-pppoe.service"));
        assert!(!y.peers.exists());
        // 3 dk içinde onaylanmadı: eski ayar (PPPoE açık) geri gelir, pppd yeniden başlar
        c.lock().unwrap().clear();
        assert_eq!(geri_al(&cfg, &y, "sure", &r), Ok(vec![]));
        assert_eq!(load(&y.ag), acik);
        sirayla(&c.lock().unwrap(), &["systemctl enable wificorrect-pppoe.service", "systemctl restart wificorrect-pppoe.service"]);
        assert!(y.peers.exists());
    }

    #[test]
    fn pppoe_uygula_bos_parola_korunur() {
        let (cfg, y, _) = setup();
        let eski = Ag { pppoe: Pppoe { acik: true, kullanici: "k1".into(), parola: "p1".into(), vlan: 35, mtu: 1400, kimlik: "chap".into(), ..Pppoe::default() }, ..Ag::default() };
        save(&y.ag, &eski).unwrap();
        let (r, c) = sahte(&[]);
        let yeni = Pppoe { acik: true, kullanici: "k2".into(), parola: String::new(), vlan: 0, ..Pppoe::default() };
        assert_eq!(pppoe_uygula(&cfg, &y, &yeni, &r), Ok(vec![]));
        // boş parola = eskisi kalır; gönderilen öbür alanlar (boş AC adı, varsayılan MTU/kimlik dahil) aynen yazılır
        let k = load(&y.ag);
        assert_eq!(k.pppoe, Pppoe { acik: true, kullanici: "k2".into(), parola: "p1".into(), vlan: 0, ..Pppoe::default() });
        assert_eq!((&k.wan, &k.lan, &k.wifi, &k.wan_mac), (&eski.wan, &eski.lan, &eski.wifi, &eski.wan_mac)); // portlara ve Wi-Fi'a dokunulmaz
        // bağlantı dosyası yeniden yazıldı (yeni kullanıcı adı, eski parola, VLAN'sız arayüz)
        let peers = fs::read_to_string(&y.peers).unwrap();
        assert_eq!(peers, crate::pppoe::peers_text(&k));
        assert!(peers.contains("user \"k2\"\n") && peers.contains("password \"p1\"\n") && peers.contains("nic-enp3s0\n"), "{peers}");
        assert!(y.interfaces.exists() && y.nft.exists());
        // güvenlik duvarı (yükseltilmiş cihazda yüklü olan eski tek adlı WAN wfcppp trafiğini iletmez) ve hız sayaçları yeniden
        // yüklenir, sonra pppd birimi; geri alma zamanlayıcısı yok (cihaz zaten çevrimdışı olabilir) ve ağ yeniden kurulmaz
        let calls = c.lock().unwrap().clone();
        sirayla(&calls, &["nft -f /etc/wificorrect/guvenlik.nft", "nft -f /etc/wificorrect/hiz.nft", "systemctl enable wificorrect-pppoe.service", "systemctl restart wificorrect-pppoe.service"]);
        assert!(!calls.iter().any(|x| x.contains("systemd-run") || x.contains("ag-geri-al") || x.starts_with("if") || x.contains("dnsmasq")), "{calls:?}");
        assert!(pending(&y).is_none() && !y.yedek().exists());

        // parola yazılırsa değişir
        let (r, _) = sahte(&[]);
        assert_eq!(pppoe_uygula(&cfg, &y, &Pppoe { parola: "p2".into(), ..yeni.clone() }, &r), Ok(vec![]));
        assert_eq!(load(&y.ag).pppoe.parola, "p2");
        assert!(fs::read_to_string(&y.peers).unwrap().contains("password \"p2\"\n"));

        // kapatma: bağlantı dosyası gider, birim durur; kullanıcı adı ve parola ag.toml'da kalır (yeniden açınca yazmak gerekmez)
        let (r, c) = sahte(&[]);
        assert_eq!(pppoe_uygula(&cfg, &y, &Pppoe { acik: false, ..Pppoe::default() }, &r), Ok(vec![]));
        let k = load(&y.ag);
        assert_eq!((k.pppoe.acik, k.pppoe.kullanici.as_str(), k.pppoe.parola.as_str()), (false, "", "p2")); // kullanıcı adı gönderildiği gibi (boş), parola kalır
        assert!(!y.peers.exists());
        let calls = c.lock().unwrap().clone();
        sirayla(&calls, &["nft -f /etc/wificorrect/guvenlik.nft", "systemctl disable --now wificorrect-pppoe.service"]);
        assert!(!calls.iter().any(|x| x.contains("restart wificorrect-pppoe")), "{calls:?}");
    }

    #[test]
    fn pppoe_uygula_gecersiz_ayar_hicbir_sey_degistirmez() {
        let (cfg, y, _) = setup();
        let eski = Ag { pppoe: Pppoe { acik: true, kullanici: "k1".into(), parola: "p1".into(), ..Pppoe::default() }, ..Ag::default() };
        save(&y.ag, &eski).unwrap();
        write_files(&cfg, &eski, &y).unwrap();
        let ag_once = fs::read(&y.ag).unwrap();
        let peers_once = fs::read(&y.peers).unwrap();
        let (r, c) = sahte(&[]);
        let dene = |p: Pppoe| pppoe_uygula(&cfg, &y, &p, &r);
        let gecerli = || Pppoe { acik: true, kullanici: "k2".into(), parola: "p2".into(), ..Pppoe::default() };
        // pppd dosyasına satır sokan kullanıcı adı / parola / AC adı, aralık dışı değerler
        for kotu in [
            Pppoe { kullanici: "k2\nconnect /bin/sh".into(), ..gecerli() },
            Pppoe { parola: "p\"2".into(), ..gecerli() },
            Pppoe { vlan: 4095, ..gecerli() },
            Pppoe { mtu: 1500, ..gecerli() },
            Pppoe { kimlik: "eap".into(), ..gecerli() },
            Pppoe { ac_adi: "a b".into(), ..gecerli() },
            Pppoe { kullanici: String::new(), ..gecerli() },
        ] {
            assert!(dene(kotu.clone()).is_err(), "{kotu:?}");
        }
        // açık ve parola hiç yok (ne gönderildi ne kayıtlı): boş parolayla açılmaz
        save(&y.ag, &Ag::default()).unwrap();
        let ag_bos = fs::read(&y.ag).unwrap();
        assert_eq!(dene(Pppoe { parola: String::new(), ..gecerli() }), Err("PPPoE kullanıcı adı ve parolası boş olamaz.".to_string()));
        assert_eq!(fs::read(&y.ag).unwrap(), ag_bos);
        // ilk durumda ag.toml ve bağlantı dosyası da aynıydı: hiçbir şey değişmedi, komut çalışmadı
        fs::write(&y.ag, &ag_once).unwrap();
        assert!(dene(Pppoe { vlan: 5000, ..gecerli() }).is_err());
        assert_eq!((fs::read(&y.ag).unwrap(), fs::read(&y.peers).unwrap()), (ag_once.clone(), peers_once));
        assert!(c.lock().unwrap().is_empty(), "{:?}", c.lock().unwrap());
        // onay bekleyen port değişikliği varken: geri alma yedeği Servis'in yazdığını ezerdi → reddedilir
        ortak::mkdirs(&y.durum).unwrap();
        write_600(&y.bekliyor(), "9999999999 wfc-ag-geri-al-x\n").unwrap();
        let e = dene(gecerli()).unwrap_err();
        assert!(e.contains("onay bekliyor"), "{e}");
        assert_eq!(fs::read(&y.ag).unwrap(), ag_once);
        assert!(c.lock().unwrap().is_empty());
    }

    #[test]
    fn pppoe_uygula_hatalari_listelenir() {
        let (cfg, y, _) = setup();
        save(&y.ag, &Ag::default()).unwrap();
        let yeni = Pppoe { acik: true, kullanici: "k".into(), parola: "p".into(), ..Pppoe::default() };
        // güvenlik duvarı yüklenemezse hata listede, birim yine de yönetilir (kapı kapalı kalır: kapalıyken güvenli)
        let (r, c) = sahte(&["nft -f /etc/wificorrect/guvenlik.nft"]);
        assert_eq!(pppoe_uygula(&cfg, &y, &yeni, &r), Ok(vec!["güvenlik duvarı yüklenemedi".to_string()]));
        sirayla(&c.lock().unwrap(), &["nft -f /etc/wificorrect/hiz.nft", "systemctl restart wificorrect-pppoe.service"]);
        // hız sayaçları yüklenemezse sessiz (switch gibi: yalnızca hız sütunları boş kalır)
        let (r, _) = sahte(&["nft -f /etc/wificorrect/hiz.nft"]);
        assert_eq!(pppoe_uygula(&cfg, &y, &yeni, &r), Ok(vec![]));
        // birim başlamazsa
        let (r, _) = sahte(&["systemctl restart"]);
        assert_eq!(pppoe_uygula(&cfg, &y, &yeni, &r), Ok(vec!["PPPoE başlatılamadı".to_string()]));
    }
}
