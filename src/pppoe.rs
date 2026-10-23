//! İnternet alan portta PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md).
//! DHCP hep açık kalır; PPPoE açıksa yanında pppd çalışır (`wificorrect-pppoe.service`) ve PPPoE rotası öne geçer.
//! Ayar `ag.toml` `[pppoe]` (src/ag.rs); pppd bağlantı dosyası (`/etc/ppp/peers/wificorrect`, 600) ondan üretilir.

use crate::ag::Ag;
use crate::ortak::Runner;
use std::path::Path;

/// pppd'nin açtığı PPP arayüzünün sabit adı (güvenlik duvarı `define WAN` kümesi, yavaşlatma ve durum bunu bilir).
pub const ARAYUZ: &str = "wfcppp";
/// pppd'yi çalıştıran systemd birimi: yalnızca `pppoe.acik` iken etkin.
pub const BIRIM: &str = "wificorrect-pppoe.service";
/// PPPoE varsayılan rotasının metriği; DHCP rotasının (bugün 1002) önünde.
pub const METRIK: u32 = 100;
/// PPPoE'de en büyük (ve varsayılan) MTU/MRU: Ethernet 1500 − PPPoE başlığı 8 (RFC 2516).
pub const MTU: u32 = 1492;

/// pppd'nin PPPoE eklentisine verilen Ethernet arayüzü: VLAN varsa `<internet portu>.<vlan>`, yoksa internet portunun kendisi.
pub fn alt_arayuz(ag: &Ag) -> String {
    match ag.pppoe.vlan {
        0 => ag.wan.clone(),
        v => format!("{}.{v}", ag.wan),
    }
}

/// pppd bağlantı dosyasının (`pppd call wificorrect`) içeriği. Açık/kapalıya bakmaz: yazıp yazmamaya `ag::write_files` karar verir.
/// Kullanıcı adı ve parola tırnaklı yazılır; `ag::pppoe_dogrula` tırnak, ters eğik çizgi ve kontrol karakterini zaten reddeder
/// (çağıran doğrulamalı). Sistemin ortak chap-secrets/pap-secrets dosyalarına dokunulmaz: kimlik bilgisi yalnızca bu dosyada.
/// Eklentinin seçenekleri (`nic-…`, `pppoe-ac`, `pppoe-service`) `plugin` satırından sonra gelmeli: eklenti yüklenince tanınırlar.
pub fn peers_text(ag: &Ag) -> String {
    peers_text_nic(ag, &alt_arayuz(ag))
}

/// `peers_text`, PPPoE eklentisinin Ethernet arayüzü `nic` olarak verilir (VLAN soneki eklenmez, ayarın VLAN'ına bakılmaz).
/// Yalnızca `ctl pppoe-peers` kullanır (ağ ad alanındaki sahte operatör denemesi: arayüz adı betikten gelir).
pub fn peers_text_nic(ag: &Ag, nic: &str) -> String {
    let p = &ag.pppoe;
    let mut s = format!(
        "# WifiCorrect üretir (ag.toml [pppoe]); {BIRIM} bunu okur. Elle değiştirmeyin.\n\
         plugin pppoe.so\nnic-{nic}\nifname {ARAYUZ}\nuser \"{user}\"\npassword \"{pass}\"\n\
         noauth\nhide-password\nnoipdefault\ndefaultroute\ndefaultroute-metric {METRIK}\nusepeerdns\n\
         persist\nmaxfail 0\nholdoff 10\nlcp-echo-interval 10\nlcp-echo-failure 6\nmtu {mtu}\nmru {mtu}\nnoipv6\n",
        user = p.kullanici,
        pass = p.parola,
        mtu = p.mtu,
    );
    match p.kimlik.as_str() {
        // yalnızca PAP: CHAP ailesi (MS-CHAP dahil) ve EAP reddedilir
        "pap" => s.push_str("refuse-chap\nrefuse-mschap\nrefuse-mschap-v2\nrefuse-eap\n"),
        // yalnızca CHAP ailesi: PAP ve EAP reddedilir
        "chap" => s.push_str("refuse-pap\nrefuse-eap\n"),
        _ => {} // otomatik: sunucunun istediği yöntem kullanılır
    }
    if !p.ac_adi.is_empty() {
        s.push_str(&format!("pppoe-ac \"{}\"\n", p.ac_adi));
    }
    if !p.servis_adi.is_empty() {
        s.push_str(&format!("pppoe-service \"{}\"\n", p.servis_adi));
    }
    s
}

fn cmd(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Birim başlarken (`ExecStartPre`, `ctl pppoe-hazirla`): internet alan Ethernet'i açar; VLAN varsa `<port>.<vlan>` arayüzünü
/// kurar (öncelik > 0 ise giden çerçevelere 802.1p önceliği: varsayılan sınıf 0 → öncelik) ve açar. pppd bu arayüze bağlanır.
/// Hata dönerse birim başlamaz (systemd 10 sn sonra yeniden dener). VLAN dururken `temizle` ile kaldırılır.
pub fn hazirla(ag: &Ag, runner: &Runner) -> Result<(), String> {
    let wan = ag.wan.as_str();
    if !runner(&cmd(&["ip", "link", "set", "dev", wan, "up"])) {
        return Err(format!("{wan} arayüzü açılamadı"));
    }
    let vlan = ag.pppoe.vlan;
    if vlan == 0 {
        return Ok(());
    }
    let alt = alt_arayuz(ag);
    let mut ekle = cmd(&["ip", "link", "add", "link", wan, "name", &alt, "type", "vlan", "id", &vlan.to_string()]);
    if ag.pppoe.oncelik > 0 {
        ekle.extend(cmd(&["egress-qos-map", &format!("0:{}", ag.pppoe.oncelik)]));
    }
    if !runner(&ekle) {
        return Err(format!("VLAN arayüzü ({alt}) kurulamadı"));
    }
    if !runner(&cmd(&["ip", "link", "set", "dev", &alt, "up"])) {
        return Err(format!("VLAN arayüzü ({alt}) açılamadı"));
    }
    Ok(())
}

/// pppd durunca (`ExecStopPost`, `ctl pppoe-temizle`): internet alan Ethernet'in bütün VLAN alt arayüzleri (`<port>.<rakam>`)
/// silinir. Ayardaki VLAN'a değil adın biçimine bakılır: ayar değişince (VLAN 35 → 7, ya da PPPoE kapatıldı) eskisi kalmasın.
/// Başka portların VLAN'larına dokunulmaz. `ip_link`: `ip -o link show type vlan` çıktısı. Silme hataları yok sayılır.
pub fn temizle(ag: &Ag, ip_link: &str, runner: &Runner) {
    for satir in ip_link.lines() {
        // "5: enp3s0.35@enp3s0: <…>": sıra numarası, sonra "ad@üst:" (-o'suz çıktıda da arayüz satırı böyle başlar)
        let mut parca = satir.split_whitespace();
        let (Some(sira), Some(ad)) = (parca.next(), parca.next()) else { continue };
        if !sira.strip_suffix(':').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        let Some((ad, ust)) = ad.strip_suffix(':').and_then(|a| a.split_once('@')) else { continue };
        let vlan_adi = ad.strip_prefix(ag.wan.as_str()).and_then(|k| k.strip_prefix('.')).is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
        if ust == ag.wan && vlan_adi {
            runner(&cmd(&["ip", "link", "del", "dev", ad]));
        }
    }
}

/// Ayar uygulanınca birimi ayara getirir: açıksa açılışta da başlasın diye etkinleştirir ve yeniden başlatır (yeni bağlantı
/// dosyasını okusun), kapalıysa durdurur ve etkinliğini kaldırır. Hata metni dönerse çağıran hata listesine ekler.
/// Birimi paket etkinleştirmez (postinst listesinde yok): açık/kapalıyı yalnızca bu işlev yönetir.
pub fn birim_uygula(ag: &Ag, runner: &Runner) -> Option<String> {
    if ag.pppoe.acik {
        let etkin = runner(&cmd(&["systemctl", "enable", BIRIM]));
        if !runner(&cmd(&["systemctl", "restart", BIRIM])) {
            return Some("PPPoE başlatılamadı".into());
        }
        // çalışıyor ama açılışta başlamayacak (etkinleştirilemedi): sessiz kalmasın
        (!etkin).then(|| "PPPoE açılışta kendiliğinden başlamayabilir (birim etkinleştirilemedi)".into())
    } else {
        (!runner(&cmd(&["systemctl", "disable", "--now", BIRIM]))).then(|| "PPPoE durdurulamadı".into())
    }
}

// ---------------------------------------------------------------- durum (spec "Durum ve teşhis")

/// İnternet alan portun durumu: panel → Portlar, sağlık raporu (`pppoe`) ve giriş sayfasındaki "internet yok" uyarısı bunu gösterir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Durum {
    /// PPPoE açık değil; internet portu modemden DHCP adresi almış
    Kapali,
    /// internet portunda kablo yok
    KabloYok,
    /// PPPoE açık değil ve DHCP'den adres alınamıyor
    DhcpYok,
    /// PPPoE sunucusu yanıt vermiyor (keşif zaman aşımı)
    SunucuYok,
    /// operatör kullanıcı adını/parolayı (ya da kimlik doğrulama yöntemini) reddetti
    KimlikHatasi,
    /// pppd deniyor
    Baglaniyor,
    /// bağlı; dış IP (`wfcppp`'nin adresi)
    Bagli(String),
}

impl Durum {
    /// Makine kodu (sağlık raporu, merkez): spec tablosu.
    pub fn kod(&self) -> &'static str {
        match self {
            Durum::Kapali => "kapali",
            Durum::KabloYok => "kablo_yok",
            Durum::DhcpYok => "dhcp_yok",
            Durum::SunucuYok => "sunucu_yok",
            Durum::KimlikHatasi => "kimlik_hatasi",
            Durum::Baglaniyor => "baglaniyor",
            Durum::Bagli(_) => "bagli",
        }
    }

    /// Ekrandaki metin: spec tablosu birebir.
    pub fn metin(&self) -> String {
        match self {
            Durum::Kapali => "PPPoE kapalı (modemden DHCP)".into(),
            Durum::KabloYok => "Kablo takılı değil".into(),
            Durum::DhcpYok => "Modemden adres alınamadı (modem kapalı ya da kablo yanlış porta takılı)".into(),
            Durum::SunucuYok => "PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)".into(),
            Durum::KimlikHatasi => "Kullanıcı adı ya da parola reddedildi".into(),
            Durum::Baglaniyor => "Bağlanıyor…".into(),
            Durum::Bagli(ip) => format!("PPPoE bağlı (dış IP {ip})"),
        }
    }
}

/// pppd günlüğünde (küçük harfe çevrilmiş satır) aranan izler. Metinler pppd 2.5.2'nin gerçek çıktısından (scripts/pppoe-deneme.sh;
/// örnekler tests/fixtures/pppoe/ ve testlerdeki `KIMLIK_YONTEMI`):
/// - kimlik reddi: `CHAP authentication failed: <operatörün mesajı>` (kimlik_hatasi.log) / `PAP authentication failed`
///   (kimlik_hatasi_pap.log); mesaj operatöre göre değişir, kök sabit. Yanlış kimlik YÖNTEMİ seçilince (ör. kimlik=chap, operatör
///   yalnız PAP ister) LCP hiç kalkmaz ve operatör `LCP terminated by peer (peer refused to authenticate)` der.
/// - sunucu yok: `Timeout waiting for PADO packets` + `Unable to complete PPPoE Discovery phase 1` (sunucu_yok.log; PADS
///   aşamasında son satır `phase 2` olur).
/// - başarı: `CHAP authentication succeeded` (bagli.log) / `PAP authentication succeeded` (sonra düşen oturumun öncesindeki
///   hatalar artık geçerli değildir).
const IZ_KIMLIK: [&str; 2] = ["authentication failed", "refused to authenticate"];
const IZ_SUNUCU: [&str; 2] = ["timeout waiting for pado", "unable to complete pppoe discovery"];
const IZ_BASARI: &str = "authentication succeeded";

/// Günlüğü sondan tarar; kararı veren ilk iz belirler (en son olay). systemd altında pppd satırları çift (syslog + stdout) ve
/// birimin/pppd'nin başlangıç satırlarıyla birlikte gelebilir: satırlar alt dizgiyle eşlenir, tekrar ve fazla satır sonucu değiştirmez.
/// İz yoksa (birim yeni başladı, keşif sürüyor) ya da son iz bir başarıysa (oturum düştü, pppd yeniden deniyor) bağlanıyordur.
/// Ayar değişince (birim yeniden başlar) önceki sonuç, yenisi gelene kadar (en çok ~35 sn) gösterilir.
fn gunluk_durumu(gunluk: &str) -> Durum {
    for satir in gunluk.lines().rev() {
        let s = satir.to_ascii_lowercase();
        if IZ_KIMLIK.iter().any(|iz| s.contains(iz)) {
            return Durum::KimlikHatasi;
        }
        if IZ_SUNUCU.iter().any(|iz| s.contains(iz)) {
            return Durum::SunucuYok;
        }
        if s.contains(IZ_BASARI) {
            break;
        }
    }
    Durum::Baglaniyor
}

/// Durumu girdilerden çıkarır (saf işlev; `durum` girdileri toplar).
/// `acik`: ag.toml `pppoe.acik`. `kablo`: internet portunda taşıyıcı var. `ppp_ip`: `wfcppp`'nin IPv4 adresi (varsa).
/// `dhcp_adres`: internet portunun DHCP'den aldığı IPv4 adresi var (yalnız PPPoE kapalıyken anlamlı: açıkken DHCP bağlı olsa da
/// durum PPPoE'ninkidir). `gunluk`: pppd birim günlüğünün son dakikaları (`journalctl -o cat`).
/// Sıra: kablo yoksa her şeyden önce `KabloYok` (kablo yeni çekildiyse pppd ~70 sn daha eski adresi tutar); kapalıysa DHCP; açıkta
/// PPP adresi varsa `Bagli` (günlükteki eski hatalar geçersiz); yoksa günlük.
pub fn durum_coz(acik: bool, kablo: bool, ppp_ip: Option<&str>, dhcp_adres: bool, gunluk: &str) -> Durum {
    if !kablo {
        return Durum::KabloYok;
    }
    if !acik {
        return if dhcp_adres { Durum::Kapali } else { Durum::DhcpYok };
    }
    match ppp_ip {
        Some(ip) => Durum::Bagli(ip.to_string()),
        None => gunluk_durumu(gunluk),
    }
}

/// `ip -4 -o addr show dev X` çıktısındaki ilk kullanılabilir IPv4 adresi: `inet 192.168.1.110/24 brd …` (Ethernet) ya da
/// `inet 203.0.113.10 peer 198.51.100.1/32 …` (PPP: yerel adres `peer` ile birlikte). Bağlantı-yerel adres (169.254/16: dhcpcd'nin
/// DHCP yanıtı yokken kendi verdiği IPv4LL) gerçek bir adres değildir, sayılmaz.
fn adres_coz(ip_addr: &str) -> Option<String> {
    ip_addr.lines().find_map(|satir| {
        let mut it = satir.split_whitespace();
        it.find(|w| *w == "inet")?;
        let ip: std::net::Ipv4Addr = it.next()?.split('/').next()?.parse().ok()?;
        (!ip.is_link_local() && !ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
    })
}

/// Canlı durum. Kablo `sys/<internet portu>/carrier`; adresler `ip -4 -o addr show dev …`; PPPoE açık ve bağlı değilse pppd birim
/// günlüğünün son 10 dakikası. `komut`: süre sınırlı komut çıktısı (`None`: çalışmadı / zaman aşımı → adres/günlük yok sayılır).
/// Yalnızca kararı etkileyecek veri okunur: kapalıyken günlük, bağlıyken DHCP adresi, kablo yokken hiçbiri okunmaz.
pub fn durum(ag: &Ag, sys: &Path, komut: &dyn Fn(&[&str]) -> Option<String>) -> Durum {
    let adres = |arayuz: &str| komut(&["ip", "-4", "-o", "addr", "show", "dev", arayuz]).and_then(|o| adres_coz(&o));
    let (acik, kablo) = (ag.pppoe.acik, crate::ag::link(sys, &ag.wan).0);
    let ppp_ip = if acik && kablo { adres(ARAYUZ) } else { None };
    let dhcp_adres = !acik && kablo && adres(&ag.wan).is_some();
    let gunluk = if acik && kablo && ppp_ip.is_none() {
        komut(&["journalctl", "-u", BIRIM, "--since", "-10min", "-o", "cat", "--no-pager", "-q"]).unwrap_or_default()
    } else {
        String::new()
    };
    durum_coz(acik, kablo, ppp_ip.as_deref(), dhcp_adres, &gunluk)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ag::Pppoe;

    fn ag(p: Pppoe) -> Ag {
        Ag { pppoe: p, ..Ag::default() }
    }

    fn acik() -> Pppoe {
        Pppoe { acik: true, kullanici: "kullanici1".into(), parola: "parola1".into(), vlan: 35, ..Pppoe::default() }
    }

    #[test]
    fn sabitler() {
        assert_eq!((ARAYUZ, BIRIM, METRIK, MTU), ("wfcppp", "wificorrect-pppoe.service", 100, 1492));
    }

    #[test]
    fn alt_arayuz_vlanli_ve_vlansiz() {
        assert_eq!(alt_arayuz(&ag(Pppoe::default())), "enp3s0");
        assert_eq!(alt_arayuz(&ag(acik())), "enp3s0.35");
        assert_eq!(alt_arayuz(&ag(Pppoe { vlan: 0, ..acik() })), "enp3s0"); // VLAN yoksa çıplak arayüz (açık da olsa)
        // internet alan port başka olabilir
        let a = Ag { wan: "enp1s0".into(), pppoe: Pppoe { vlan: 4094, ..acik() }, ..Ag::default() };
        assert_eq!(alt_arayuz(&a), "enp1s0.4094");
    }

    #[test]
    fn peers_metni() {
        let t = peers_text(&ag(acik()));
        let l: Vec<&str> = t.lines().collect();
        for beklenen in [
            "plugin pppoe.so",
            "nic-enp3s0.35",
            "ifname wfcppp",
            "user \"kullanici1\"",
            "password \"parola1\"",
            "noauth",
            "hide-password",
            "noipdefault",
            "defaultroute",
            "defaultroute-metric 100",
            "usepeerdns",
            "persist",
            "maxfail 0",
            "holdoff 10",
            "lcp-echo-interval 10",
            "lcp-echo-failure 6",
            "mtu 1492",
            "mru 1492",
            "noipv6",
        ] {
            assert!(l.contains(&beklenen), "{beklenen:?} satırı yok:\n{t}");
        }
        assert!(!t.contains("refuse-"), "kimlik otomatik: reddedilen yöntem olmamalı:\n{t}");
        assert!(!t.contains("pppoe-ac") && !t.contains("pppoe-service"), "AC/servis adı boşken satırı olmamalı:\n{t}");
        // eklenti önce yüklenir; arayüz seçeneği (eklentinin) ondan sonra gelir
        let yer = |s: &str| l.iter().position(|x| *x == s).unwrap();
        assert!(yer("plugin pppoe.so") < yer("nic-enp3s0.35"));
        // her seçenek tek satırda, tekrar ve boş satır yok
        let mut sirali = l.clone();
        sirali.sort();
        sirali.dedup();
        assert_eq!(sirali.len(), l.len(), "tekrarlı satır:\n{t}");
        assert!(l.iter().all(|x| !x.is_empty()) && t.ends_with('\n'));
        // VLAN yoksa çıplak arayüz
        let t = peers_text(&ag(Pppoe { vlan: 0, ..acik() }));
        assert!(t.lines().any(|x| x == "nic-enp3s0") && !t.contains("nic-enp3s0."), "{t}");
    }

    #[test]
    fn peers_gelismis() {
        let t = peers_text(&ag(Pppoe { mtu: 1400, kimlik: "pap".into(), ac_adi: "AC1".into(), servis_adi: "srv".into(), ..acik() }));
        let l: Vec<&str> = t.lines().collect();
        for beklenen in ["mtu 1400", "mru 1400", "refuse-chap", "refuse-mschap", "refuse-mschap-v2", "refuse-eap", "pppoe-ac \"AC1\"", "pppoe-service \"srv\""] {
            assert!(l.contains(&beklenen), "{beklenen:?} satırı yok:\n{t}");
        }
        assert!(!l.contains(&"refuse-pap") && !l.contains(&"mtu 1492") && !l.contains(&"mru 1492"), "{t}");
        // eklentinin seçenekleri eklenti satırından sonra
        let yer = |s: &str| l.iter().position(|x| *x == s).unwrap();
        assert!(yer("plugin pppoe.so") < yer("pppoe-ac \"AC1\"") && yer("plugin pppoe.so") < yer("pppoe-service \"srv\""));

        // CHAP: PAP ve EAP reddedilir; CHAP ailesi (MS-CHAP dahil) serbest
        let t = peers_text(&ag(Pppoe { kimlik: "chap".into(), ..acik() }));
        let l: Vec<&str> = t.lines().collect();
        assert!(l.contains(&"refuse-pap") && l.contains(&"refuse-eap"), "{t}");
        assert!(!l.contains(&"refuse-chap") && !l.contains(&"refuse-mschap") && !l.contains(&"refuse-mschap-v2"), "{t}");

        // yalnızca biri doluysa yalnızca onun satırı
        let t = peers_text(&ag(Pppoe { ac_adi: "AC1".into(), ..acik() }));
        assert!(t.contains("pppoe-ac \"AC1\"\n") && !t.contains("pppoe-service"), "{t}");
        let t = peers_text(&ag(Pppoe { servis_adi: "srv".into(), ..acik() }));
        assert!(t.contains("pppoe-service \"srv\"\n") && !t.contains("pppoe-ac"), "{t}");
    }

    #[test]
    fn peers_arayuzu_disaridan() {
        // ctl pppoe-peers: arayüz adı olduğu gibi yazılır (VLAN soneki eklenmez); kalan her satır peers_text ile aynı
        let a = ag(acik()); // vlan 35
        let t = peers_text_nic(&a, "vethcpe");
        assert!(t.lines().any(|l| l == "nic-vethcpe") && !t.contains("nic-enp3s0") && !t.contains("vethcpe.35"), "{t}");
        assert_eq!(t.replace("nic-vethcpe", "nic-enp3s0.35"), peers_text(&a));
        assert!(peers_text_nic(&a, "vethcpe.35").lines().any(|l| l == "nic-vethcpe.35"));
    }

    // ---------------------------------------------------------------- VLAN arayüzü ve systemd birimi
    use std::sync::{Arc, Mutex};

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

    fn cagrilar(c: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        c.lock().unwrap().clone()
    }

    #[test]
    fn hazirla_vlan() {
        let (r, c) = sahte(&[]);
        assert_eq!(hazirla(&ag(acik()), &r), Ok(()));
        assert_eq!(
            cagrilar(&c),
            ["ip link set dev enp3s0 up", "ip link add link enp3s0 name enp3s0.35 type vlan id 35", "ip link set dev enp3s0.35 up"]
        );
        // 802.1p önceliği > 0: giden çerçevelere
        let (r, c) = sahte(&[]);
        assert_eq!(hazirla(&ag(Pppoe { oncelik: 3, ..acik() }), &r), Ok(()));
        assert_eq!(
            cagrilar(&c),
            [
                "ip link set dev enp3s0 up",
                "ip link add link enp3s0 name enp3s0.35 type vlan id 35 egress-qos-map 0:3",
                "ip link set dev enp3s0.35 up"
            ]
        );
        // VLAN yok: yalnızca internet portu açılır (öncelik VLAN'sız anlamsız)
        let (r, c) = sahte(&[]);
        assert_eq!(hazirla(&ag(Pppoe { vlan: 0, oncelik: 5, ..acik() }), &r), Ok(()));
        assert_eq!(cagrilar(&c), ["ip link set dev enp3s0 up"]);
        // internet alan port başka olabilir
        let (r, c) = sahte(&[]);
        let b = Ag { wan: "enp1s0".into(), pppoe: Pppoe { vlan: 4094, ..acik() }, ..Ag::default() };
        assert_eq!(hazirla(&b, &r), Ok(()));
        assert_eq!(cagrilar(&c)[1], "ip link add link enp1s0 name enp1s0.4094 type vlan id 4094");
    }

    #[test]
    fn hazirla_hatalari() {
        // internet portu açılamazsa gerisi denenmez
        let (r, c) = sahte(&["ip link set dev enp3s0 up"]);
        assert_eq!(hazirla(&ag(acik()), &r), Err("enp3s0 arayüzü açılamadı".into()));
        assert_eq!(cagrilar(&c).len(), 1);
        // VLAN arayüzü kurulamazsa ya da açılamazsa birim başlamaz (ExecStartPre hata verir)
        let (r, c) = sahte(&["ip link add"]);
        assert_eq!(hazirla(&ag(acik()), &r), Err("VLAN arayüzü (enp3s0.35) kurulamadı".into()));
        assert_eq!(cagrilar(&c).len(), 2);
        let (r, _) = sahte(&["ip link set dev enp3s0.35"]);
        assert_eq!(hazirla(&ag(acik()), &r), Err("VLAN arayüzü (enp3s0.35) açılamadı".into()));
    }

    #[test]
    fn temizle_butun_vlanlar() {
        // `ip -o link show type vlan`: önceki ayarlardan kalmış VLAN'lar da (ayardaki VLAN 35 olsa bile) silinir
        let ip_link = "\
5: enp3s0.35@enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP mode DEFAULT group default qlen 1000\\    link/ether 00:0e:c4:ce:a0:9b brd ff:ff:ff:ff:ff:ff promiscuity 0 minmtu 0 maxmtu 65535 \\    vlan protocol 802.1Q id 35 <REORDER_HDR> addrgenmode eui64 numtxqueues 1 numrxqueues 1
6: enp3s0.7@enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP mode DEFAULT group default qlen 1000\\    link/ether 00:0e:c4:ce:a0:9b brd ff:ff:ff:ff:ff:ff promiscuity 0 minmtu 0 maxmtu 65535 \\    vlan protocol 802.1Q id 7 <REORDER_HDR> addrgenmode eui64 numtxqueues 1 numrxqueues 1
7: enp1s0.5@enp1s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP mode DEFAULT group default qlen 1000\\    link/ether 00:0e:c4:ce:a0:9a brd ff:ff:ff:ff:ff:ff promiscuity 0 minmtu 0 maxmtu 65535 \\    vlan protocol 802.1Q id 5 <REORDER_HDR> addrgenmode eui64 numtxqueues 1 numrxqueues 1
";
        let (r, c) = sahte(&[]);
        temizle(&ag(acik()), ip_link, &r);
        assert_eq!(cagrilar(&c), ["ip link del dev enp3s0.35", "ip link del dev enp3s0.7"]); // enp1s0.5'e dokunulmaz
        // PPPoE ya da VLAN kapatılmış olsa da (ayar değişince birim durur) internet portunun bütün VLAN'ları gider
        let (r, c) = sahte(&[]);
        temizle(&ag(Pppoe::default()), ip_link, &r);
        assert_eq!(cagrilar(&c), ["ip link del dev enp3s0.35", "ip link del dev enp3s0.7"]);
        // internet alan port başkaysa onun VLAN'ları
        let (r, c) = sahte(&[]);
        temizle(&Ag { wan: "enp1s0".into(), ..Ag::default() }, ip_link, &r);
        assert_eq!(cagrilar(&c), ["ip link del dev enp1s0.5"]);
        // -o'suz çıktıda arayüz satırı aynı biçimde başlar, devam satırları atlanır
        let uzun = "5: enp3s0.35@enp3s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc noqueue state UP\n    link/ether 00:0e:c4:ce:a0:9b brd ff:ff:ff:ff:ff:ff\n    vlan protocol 802.1Q id 35 <REORDER_HDR>\n";
        let (r, c) = sahte(&[]);
        temizle(&ag(acik()), uzun, &r);
        assert_eq!(cagrilar(&c), ["ip link del dev enp3s0.35"]);
        // VLAN olmayan, rakamsız ya da başka üst arayüze bağlı adlar ve boş çıktı: dokunulmaz
        let (r, c) = sahte(&[]);
        for satir in [
            "8: enp3s0.abc@enp3s0: <UP> mtu 1500",
            "9: enp3s0.@enp3s0: <UP> mtu 1500",
            "10: enp3s0.35@enp1s0: <UP> mtu 1500",
            "11: enp3s0x35@enp3s0: <UP> mtu 1500",
            "12: enp3s0.35: <UP> mtu 1500",
            "13: br-hotspot@enp3s0: <UP> mtu 1500",
            "enp3s0.35@enp3s0: <UP> mtu 1500",
            "",
        ] {
            temizle(&ag(acik()), satir, &r);
        }
        assert!(cagrilar(&c).is_empty(), "{:?}", cagrilar(&c));
    }

    #[test]
    fn birim_acik_kapali() {
        // açık: açılışta başlasın + şimdi (yeniden) başlasın
        let (r, c) = sahte(&[]);
        assert_eq!(birim_uygula(&ag(acik()), &r), None);
        assert_eq!(cagrilar(&c), ["systemctl enable wificorrect-pppoe.service", "systemctl restart wificorrect-pppoe.service"]);
        // kapalı (bilgiler dolu olsa da): durur ve açılışta başlamaz
        let (r, c) = sahte(&[]);
        assert_eq!(birim_uygula(&ag(Pppoe { acik: false, ..acik() }), &r), None);
        assert_eq!(cagrilar(&c), ["systemctl disable --now wificorrect-pppoe.service"]);
        assert_eq!(birim_uygula(&ag(Pppoe::default()), &sahte(&[]).0), None);
        // restart başarısızsa: hata (başka komut da başarısızsa aynı metin)
        let (r, _) = sahte(&["systemctl restart"]);
        assert_eq!(birim_uygula(&ag(acik()), &r), Some("PPPoE başlatılamadı".into()));
        let (r, c) = sahte(&["systemctl"]);
        assert_eq!(birim_uygula(&ag(acik()), &r), Some("PPPoE başlatılamadı".into()));
        assert_eq!(cagrilar(&c).len(), 2); // enable hatası restart'ı engellemez
        // çalışıyor ama açılışta başlamayacak: ayrı uyarı
        let (r, _) = sahte(&["systemctl enable"]);
        assert_eq!(birim_uygula(&ag(acik()), &r), Some("PPPoE açılışta kendiliğinden başlamayabilir (birim etkinleştirilemedi)".into()));
        // kapatılamazsa uyarı
        let (r, _) = sahte(&["systemctl disable"]);
        assert_eq!(birim_uygula(&ag(Pppoe::default()), &r), Some("PPPoE durdurulamadı".into()));
    }

    // Birim dosyası ve güvenlik duvarı kuralı kodla birlikte değişmeli: test.sh bu iki dosyayı da cihaza gönderir.
    const BIRIM_DOSYASI: &str = include_str!("../deploy/debian/etc/systemd/system/wificorrect-pppoe.service");
    const GUVENLIK_NFT: &str = include_str!("../deploy/debian/etc/wificorrect/guvenlik.nft");

    #[test]
    fn birim_dosyasi() {
        let satir = |s: &str| BIRIM_DOSYASI.lines().any(|l| l == s);
        let peers = crate::ag::Yollar::sistem().peers;
        // bağlantı dosyası yoksa (PPPoE kapalı) birim başlamaz; pppd o dosyayı adıyla okur
        assert!(satir(&format!("ConditionPathExists={}", peers.display())), "{BIRIM_DOSYASI}");
        assert!(satir(&format!("ExecStart=/usr/sbin/pppd call {} nodetach", peers.file_name().unwrap().to_str().unwrap())), "{BIRIM_DOSYASI}");
        assert!(satir("ExecStartPre=/usr/local/bin/wificorrect ctl pppoe-hazirla") && satir("ExecStopPost=/usr/local/bin/wificorrect ctl pppoe-temizle"));
        assert!(satir("Restart=always") && satir("RestartSec=10") && satir("After=network.target") && satir("WantedBy=multi-user.target"));
        assert!(satir("SuccessExitStatus=5")); // pppd SIGTERM'e 5 koduyla çıkar: durdurma birimi "failed" yapmasın
        assert!(!BIRIM_DOSYASI.contains("ProtectSystem") && !BIRIM_DOSYASI.contains("PrivateTmp")); // pppd /etc/ppp, /run/ppp ve çekirdeğe yazar
    }

    #[test]
    fn guvenlik_nft_mss_kurali() {
        // iletilen TCP SYN (ve SYN-ACK) MSS'i çıkış rotasının MTU'suna iner (PPPoE 1492); zincirin İLK kuralı olmalı
        let zincir = GUVENLIK_NFT.split("chain forward {").nth(1).unwrap().split('}').next().unwrap();
        let kurallar: Vec<&str> = zincir.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        assert_eq!(kurallar[0], "type filter hook forward priority filter; policy drop;");
        assert_eq!(kurallar[1], "tcp flags syn / syn,rst tcp option maxseg size set rt mtu");
        assert_eq!(kurallar[2], "ct state established,related accept");
        assert!(GUVENLIK_NFT.contains("oifname $WAN ip saddr 10.50.0.0/24 masquerade")); // WAN kümesi masquerade'i de kapsar
    }

    // ---------------------------------------------------------------- durum (spec "Durum ve teşhis")
    // Gerçek pppd 2.5.2 günlükleri: scripts/pppoe-deneme.sh (sahte operatör, ağ ad alanı) → tests/fixtures/pppoe/
    const BAGLI: &str = include_str!("../tests/fixtures/pppoe/bagli.log");
    const KIMLIK_CHAP: &str = include_str!("../tests/fixtures/pppoe/kimlik_hatasi.log");
    const KIMLIK_PAP: &str = include_str!("../tests/fixtures/pppoe/kimlik_hatasi_pap.log");
    const SUNUCU_YOK: &str = include_str!("../tests/fixtures/pppoe/sunucu_yok.log");
    // Yanlış kimlik YÖNTEMİ (kimlik=chap iken operatör yalnız PAP ister): LCP hiç kalkmaz, kimlik doğrulama başlamaz, pppd ~3,3 sn'de
    // bir yeniden dener. Gerçek günlük: cihazda /root/pppoe-deneme/kimlik_chap_red.log (pppoe-deneme.sh kimlik_chap_red), tek deneme.
    const KIMLIK_YONTEMI: &str = "PPP session is 2\n\
        Connected to 02:57:46:43:00:01 via interface wfccpe0\n\
        Using interface wfcppp\n\
        Connect: wfcppp <--> wfccpe0\n\
        peer from calling number 02:57:46:43:00:01 authorized\n\
        LCP terminated by peer (peer refused to authenticate)\n\
        Modem hangup\n\
        Connection terminated.\n\
        Sent PADT\n";
    // PAP ile bağlanma: kimlik satırları CHAP'inkinden farklı. Gerçek günlük (cihazda /root/pppoe-deneme/kimlik_pap.log, kimlik_pap senaryosu)
    const BAGLI_PAP: &str = "PPP session is 1\n\
        Connected to 02:57:46:43:00:01 via interface wfccpe0\n\
        Using interface wfcppp\n\
        Connect: wfcppp <--> wfccpe0\n\
        Remote message: Login ok\n\
        PAP authentication succeeded\n\
        peer from calling number 02:57:46:43:00:01 authorized\n\
        local  IP address 203.0.113.10\n\
        remote IP address 198.51.100.1\n";
    // Bağlıyken oturumun düşmesi (operatör PADT'si): gerçek günlük, yeniden_baglanma senaryosu
    const DUSTU: &str = "Modem hangup\nConnect time 1.2 minutes.\nSent 0 bytes, received 0 bytes.\nConnection terminated.\nSent PADT\n";

    fn acik_coz(gunluk: &str) -> Durum {
        durum_coz(true, true, None, false, gunluk)
    }

    #[test]
    fn durum_kapali() {
        assert_eq!(durum_coz(false, false, None, false, ""), Durum::KabloYok);
        assert_eq!(durum_coz(false, false, None, true, ""), Durum::KabloYok); // kablo yoksa eski adres önemsiz
        assert_eq!(durum_coz(false, true, None, false, ""), Durum::DhcpYok);
        assert_eq!(durum_coz(false, true, None, true, ""), Durum::Kapali);
        // PPPoE kapalıyken eski günlük ve kalıntı PPP adresi durumu değiştirmez
        assert_eq!(durum_coz(false, true, None, true, KIMLIK_CHAP), Durum::Kapali);
        assert_eq!(durum_coz(false, true, Some("85.1.2.3"), true, SUNUCU_YOK), Durum::Kapali);
        assert_eq!(durum_coz(false, true, None, false, SUNUCU_YOK), Durum::DhcpYok);
    }

    #[test]
    fn durum_acik_fixture() {
        let coz = |kablo: bool, ip: Option<&str>, gunluk: &str| durum_coz(true, kablo, ip, false, gunluk);
        assert_eq!(coz(true, None, KIMLIK_CHAP), Durum::KimlikHatasi);
        assert_eq!(coz(true, None, KIMLIK_PAP), Durum::KimlikHatasi);
        assert_eq!(coz(true, None, KIMLIK_YONTEMI), Durum::KimlikHatasi);
        assert_eq!(coz(true, None, SUNUCU_YOK), Durum::SunucuYok);
        assert_eq!(coz(true, None, ""), Durum::Baglaniyor);
        // adres varsa bağlı (günlükte eski bir hata olsa da)
        assert_eq!(coz(true, Some("85.1.2.3"), ""), Durum::Bagli("85.1.2.3".into()));
        assert_eq!(coz(true, Some("85.1.2.3"), KIMLIK_CHAP), Durum::Bagli("85.1.2.3".into()));
        assert_eq!(coz(true, Some("85.1.2.3"), SUNUCU_YOK), Durum::Bagli("85.1.2.3".into()));
        assert_eq!(coz(true, Some("203.0.113.10"), BAGLI), Durum::Bagli("203.0.113.10".into()));
        // kablo yoksa her şeyden önce (kablo yeni çekildiyse pppd ~70 sn daha eski adresi tutar)
        assert_eq!(coz(false, None, ""), Durum::KabloYok);
        assert_eq!(coz(false, None, SUNUCU_YOK), Durum::KabloYok);
        assert_eq!(coz(false, Some("85.1.2.3"), BAGLI), Durum::KabloYok);
    }

    #[test]
    fn durum_dhcp_ve_pppoe_birlikte() {
        // DHCP adresi de var, PPPoE de bağlı: PPPoE tercih edilir
        assert_eq!(durum_coz(true, true, Some("85.1.2.3"), true, ""), Durum::Bagli("85.1.2.3".into()));
        // DHCP çalışıyor ama PPPoE bağlanamıyor (dükkân: modemde PPPoE sunucusu yok): durum PPPoE'nindir, DHCP'ninki değil
        assert_eq!(durum_coz(true, true, None, true, SUNUCU_YOK), Durum::SunucuYok);
        assert_eq!(durum_coz(true, true, None, true, ""), Durum::Baglaniyor);
        assert_eq!(durum_coz(true, true, None, false, ""), Durum::Baglaniyor); // açıkken "DHCP yok" durumu yoktur
    }

    #[test]
    fn durum_gunlugu_sondan_taranir() {
        // en son olay karar verir
        assert_eq!(acik_coz(&format!("{SUNUCU_YOK}{KIMLIK_CHAP}")), Durum::KimlikHatasi); // sunucu bulundu, kimlik reddedildi
        assert_eq!(acik_coz(&format!("{KIMLIK_CHAP}{SUNUCU_YOK}")), Durum::SunucuYok); // sunucu gitti
        assert_eq!(acik_coz(&format!("{KIMLIK_CHAP}{KIMLIK_PAP}{KIMLIK_YONTEMI}")), Durum::KimlikHatasi);
        // yeni deneme sürerken (henüz sonuç yok) son sonuç gösterilir
        let yeni = "PPP session is 1\nConnected to 02:57:46:43:00:01 via interface wfccpe0\nUsing interface wfcppp\n";
        assert_eq!(acik_coz(&format!("{KIMLIK_CHAP}{yeni}")), Durum::KimlikHatasi);
        // hata düzeldi, bağlandı, sonra oturum düştü (adres yok, pppd yeniden deniyor): eski hata geçersiz
        assert_eq!(acik_coz(&format!("{SUNUCU_YOK}{KIMLIK_CHAP}{BAGLI}{DUSTU}")), Durum::Baglaniyor);
        assert_eq!(acik_coz(&format!("{KIMLIK_PAP}{BAGLI_PAP}{DUSTU}")), Durum::Baglaniyor);
        // düştükten sonra yeni bir hata: yine en sonuncusu
        assert_eq!(acik_coz(&format!("{BAGLI}{DUSTU}{SUNUCU_YOK}")), Durum::SunucuYok);
        assert_eq!(acik_coz(&format!("{BAGLI}{DUSTU}{KIMLIK_CHAP}")), Durum::KimlikHatasi);
        // hata olmayan sonlanmalar durumu kimlik/sunucu hatasına çevirmez (bunlarda pppd zaten yeniden dener)
        assert_eq!(acik_coz("No response to 6 echo-requests\nSerial link appears to be disconnected.\nConnection terminated.\nSent PADT\nModem hangup\n"), Durum::Baglaniyor);
        assert_eq!(acik_coz("LCP terminated by peer (Peer not responding)\nModem hangup\nConnection terminated.\nSent PADT\n"), Durum::Baglaniyor);
        assert_eq!(acik_coz("Terminating on signal 15\nConnection terminated.\nSent PADT\n"), Durum::Baglaniyor);
        // operatörün kendi mesajı değişir, kök sabit; büyük/küçük harf önemsiz; MS-CHAP ve satır ortasındaki ize de bakılır
        assert_eq!(acik_coz("CHAP authentication failed: E=691 R=1 C=0123 V=3 M=Kullanici bilinmiyor\n"), Durum::KimlikHatasi);
        assert_eq!(acik_coz("MS-CHAP-v2 AUTHENTICATION FAILED: E=691\n"), Durum::KimlikHatasi);
        assert_eq!(acik_coz("Unable to complete PPPoE Discovery phase 2\n"), Durum::SunucuYok); // PADS aşaması
        // boş satırlar ve CRLF
        assert_eq!(acik_coz("\n\r\nTimeout waiting for PADO packets\r\n\n"), Durum::SunucuYok);
    }

    #[test]
    fn durum_systemd_gunlugu() {
        // systemd altında `journalctl -u … -o cat`: birimin kendi satırları ("Started …", "…: Deactivated successfully." gibi; geçici bir
        // birimle görülen gerçek biçim) araya girer; pppd hem syslog'a hem stdout'a yazdığından (pppd(8) logfd) satırlar çift gelebilir ve
        // sırası karışabilir; pppd'nin başlangıç satırları (Plugin … loaded., pppd … started) ve bağlıyken operatör DNS'i (usepeerdns) eklenir
        let cift = |g: &str| g.lines().map(|l| format!("{l}\n{l}\n")).collect::<String>();
        let basla = "Started wificorrect-pppoe.service - WifiCorrect PPPoE baglantisi (pppd).\nPlugin pppoe.so loaded.\npppd 2.5.2 started by root, uid 0\n";
        for (g, beklenen) in [
            (KIMLIK_CHAP, Durum::KimlikHatasi),
            (KIMLIK_PAP, Durum::KimlikHatasi),
            (KIMLIK_YONTEMI, Durum::KimlikHatasi),
            (SUNUCU_YOK, Durum::SunucuYok),
        ] {
            assert_eq!(acik_coz(&format!("{basla}{}", cift(g))), beklenen, "satır satır çift: {g}");
            assert_eq!(acik_coz(&format!("{basla}{g}{g}")), beklenen, "blok blok çift: {g}");
        }
        assert_eq!(acik_coz(basla), Durum::Baglaniyor); // yalnızca başlangıç satırları
        // bağlıydı (DNS satırları var), düştü: eski hata yok
        let dns = "primary   DNS address 203.0.113.53\nsecondary DNS address 203.0.113.54\n";
        let bagli = format!("{basla}{}{dns}{dns}", cift(BAGLI));
        assert_eq!(acik_coz(&format!("{KIMLIK_CHAP}{bagli}{}", cift(DUSTU))), Durum::Baglaniyor);
        // birim yeniden başladı (pppd hata koduyla çıktı): systemd'nin satırları arada; "Failed" geçse de kimlik hatası sayılmaz
        let yeniden = "wificorrect-pppoe.service: Main process exited, code=exited, status=1/FAILURE\nwificorrect-pppoe.service: Failed with result 'exit-code'.\nwificorrect-pppoe.service: Scheduled restart job, restart counter is at 3.\nStopped wificorrect-pppoe.service - WifiCorrect PPPoE baglantisi (pppd).\n";
        assert_eq!(acik_coz(&format!("{}{yeniden}{basla}{}", cift(SUNUCU_YOK), cift(KIMLIK_CHAP))), Durum::KimlikHatasi);
        assert_eq!(acik_coz(&format!("{}{yeniden}{basla}", cift(SUNUCU_YOK))), Durum::SunucuYok); // yeni deneme henüz sonuç vermedi
    }

    #[test]
    fn durum_metinleri() {
        // spec "Durum ve teşhis" tablosu birebir
        let tablo = [
            (Durum::Kapali, "kapali", "PPPoE kapalı (modemden DHCP)"),
            (Durum::KabloYok, "kablo_yok", "Kablo takılı değil"),
            (Durum::DhcpYok, "dhcp_yok", "Modemden adres alınamadı (modem kapalı ya da kablo yanlış porta takılı)"),
            (Durum::SunucuYok, "sunucu_yok", "PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)"),
            (Durum::KimlikHatasi, "kimlik_hatasi", "Kullanıcı adı ya da parola reddedildi"),
            (Durum::Baglaniyor, "baglaniyor", "Bağlanıyor\u{2026}"),
            (Durum::Bagli("85.1.2.3".into()), "bagli", "PPPoE bağlı (dış IP 85.1.2.3)"),
        ];
        for (d, kod, metin) in tablo {
            assert_eq!((d.kod(), d.metin().as_str()), (kod, metin), "{d:?}");
        }
        assert_eq!(Durum::Bagli("203.0.113.10".into()).kod(), "bagli"); // kod IP'den bağımsız
        assert_eq!(Durum::Bagli("203.0.113.10".into()).metin(), "PPPoE bağlı (dış IP 203.0.113.10)");
    }

    // `ip -4 -o addr show dev …` çıktıları (cihazdaki gerçek biçim; PPP arayüzünde yerel adres `peer` ile birlikte yazılır)
    const PPP_ADRES: &str = "12: wfcppp    inet 203.0.113.10 peer 198.51.100.1/32 scope global wfcppp\\       valid_lft forever preferred_lft forever\n";
    const DHCP_ADRES: &str = "3: enp3s0    inet 192.168.1.110/24 brd 192.168.1.255 scope global dynamic noprefixroute enp3s0\\       valid_lft 3101sec preferred_lft 2651sec\n";
    // DHCP yanıtı yokken dhcpcd'nin (varsayılan ayarlarla) verdiği IPv4LL adresi: gerçek bir adres değildir
    const YEREL_ADRES: &str = "3: enp3s0    inet 169.254.77.5/16 brd 169.254.255.255 scope link noprefixroute enp3s0\\       valid_lft forever preferred_lft forever\n";
    const IP_PPP: &str = "ip -4 -o addr show dev wfcppp";
    const IP_WAN: &str = "ip -4 -o addr show dev enp3s0";
    const GUNLUK_KOMUTU: &str = "journalctl -u wificorrect-pppoe.service --since -10min -o cat --no-pager -q";

    #[test]
    fn adres_cozumleme() {
        assert_eq!(adres_coz(PPP_ADRES).as_deref(), Some("203.0.113.10"));
        assert_eq!(adres_coz(DHCP_ADRES).as_deref(), Some("192.168.1.110"));
        assert_eq!(adres_coz("5: ppp0    inet 10.64.64.64/32 scope global ppp0\n").as_deref(), Some("10.64.64.64")); // önek yazımı
        assert_eq!(adres_coz(YEREL_ADRES), None);
        assert_eq!(adres_coz(&format!("{YEREL_ADRES}{DHCP_ADRES}")).as_deref(), Some("192.168.1.110")); // ilk kullanılabilir olan
        assert_eq!(adres_coz(""), None);
        assert_eq!(adres_coz("Device \"wfcppp\" does not exist.\n"), None);
        assert_eq!(adres_coz("5: ppp0    inet 999.1.1.1/32 scope global ppp0\n"), None); // bozuk adres
        assert_eq!(adres_coz("5: ppp0    inet 0.0.0.0/0 scope global ppp0\n"), None);
        assert_eq!(adres_coz("1: lo    inet 127.0.0.1/8 scope host lo\n"), None);
    }

    /// Geçici `/sys/class/net`: internet portunun `carrier` dosyası (`None`: dosya yok, ör. arayüz kapalı).
    fn sys_dizini(ad: &str, carrier: Option<&str>) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wfc-pppoe-durum-{}-{ad}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("enp3s0")).unwrap();
        if let Some(c) = carrier {
            std::fs::write(d.join("enp3s0").join("carrier"), c).unwrap();
        }
        d
    }

    /// `durum`u sahte komut çıktılarıyla çalıştırır (tabloda olmayan komut `None` = çalışmadı/zaman aşımı); çağrılan komutlar sırayla döner.
    fn durum_dene(a: &Ag, sys: &Path, cikti: &[(&str, &str)]) -> (Durum, Vec<String>) {
        let cagrilar = std::cell::RefCell::new(Vec::<String>::new());
        let komut = |c: &[&str]| {
            let s = c.join(" ");
            cagrilar.borrow_mut().push(s.clone());
            cikti.iter().find(|(k, _)| *k == s).map(|(_, v)| v.to_string())
        };
        let d = durum(a, sys, &komut);
        (d, cagrilar.into_inner())
    }

    #[test]
    fn durum_islevi_kapali() {
        let kapali = ag(Pppoe::default());
        // kablo yok: hiçbir komut çalışmaz
        let (d, c) = durum_dene(&kapali, &sys_dizini("k-kablo-yok", Some("0\n")), &[(IP_WAN, DHCP_ADRES)]);
        assert_eq!((d, c.len()), (Durum::KabloYok, 0));
        // carrier okunamıyor (arayüz kapalı → EINVAL / dosya yok): kablo yok sayılır
        assert_eq!(durum_dene(&kapali, &sys_dizini("k-carrier-yok", None), &[]).0, Durum::KabloYok);
        // kablo var, DHCP adresi var: yalnızca internet portunun adresi sorulur (wfcppp ve günlük okunmaz)
        let sys = sys_dizini("k-dhcp", Some("1\n"));
        let (d, c) = durum_dene(&kapali, &sys, &[(IP_WAN, DHCP_ADRES)]);
        assert_eq!((d, c), (Durum::Kapali, vec![IP_WAN.to_string()]));
        // DHCP yanıt vermiyor: adres yok, ya da yalnızca dhcpcd'nin 169.254 adresi, ya da komut çalışmadı
        assert_eq!(durum_dene(&kapali, &sys, &[(IP_WAN, "")]).0, Durum::DhcpYok);
        assert_eq!(durum_dene(&kapali, &sys, &[(IP_WAN, YEREL_ADRES)]).0, Durum::DhcpYok);
        assert_eq!(durum_dene(&kapali, &sys, &[]).0, Durum::DhcpYok);
        // PPPoE bilgileri dolu ama kapalı: aynı
        let dolu_kapali = ag(Pppoe { acik: false, ..acik() });
        assert_eq!(durum_dene(&dolu_kapali, &sys, &[(IP_WAN, DHCP_ADRES)]).0, Durum::Kapali);
    }

    #[test]
    fn durum_islevi_acik() {
        let a = ag(acik()); // VLAN 35: pppd enp3s0.35'e bağlanır; kablo ve DHCP adresi yine internet portunda (enp3s0)
        let sys = sys_dizini("a-kablo", Some("1\n"));
        // wfcppp'nin adresi var: bağlı; günlük okunmaz, DHCP sorulmaz
        let (d, c) = durum_dene(&a, &sys, &[(IP_PPP, PPP_ADRES), (IP_WAN, DHCP_ADRES)]);
        assert_eq!((d, c), (Durum::Bagli("203.0.113.10".into()), vec![IP_PPP.to_string()]));
        // adres yok: pppd birim günlüğünün son 10 dakikası okunur
        for (gunluk, beklenen) in [
            (KIMLIK_CHAP, Durum::KimlikHatasi),
            (KIMLIK_PAP, Durum::KimlikHatasi),
            (KIMLIK_YONTEMI, Durum::KimlikHatasi),
            (SUNUCU_YOK, Durum::SunucuYok),
            ("", Durum::Baglaniyor),
        ] {
            let (d, c) = durum_dene(&a, &sys, &[(IP_PPP, "Device \"wfcppp\" does not exist.\n"), (GUNLUK_KOMUTU, gunluk)]);
            assert_eq!(d, beklenen);
            assert_eq!(c, [IP_PPP, GUNLUK_KOMUTU]);
        }
        // komutlar çalışmadı (zaman aşımı): bağlanıyor
        assert_eq!(durum_dene(&a, &sys, &[]).0, Durum::Baglaniyor);
        // kablo yok: hiçbir komut çalışmaz
        let (d, c) = durum_dene(&a, &sys_dizini("a-kablo-yok", Some("0\n")), &[(IP_PPP, PPP_ADRES), (GUNLUK_KOMUTU, BAGLI)]);
        assert_eq!((d, c.len()), (Durum::KabloYok, 0));
        // internet alan port başka olabilir: carrier ve komutlar o portun adıyla
        let b = Ag { wan: "enp1s0".into(), pppoe: Pppoe { acik: false, ..acik() }, ..Ag::default() };
        let sys = sys_dizini("b", Some("0\n"));
        std::fs::create_dir_all(sys.join("enp1s0")).unwrap();
        std::fs::write(sys.join("enp1s0").join("carrier"), "1\n").unwrap();
        let (d, c) = durum_dene(&b, &sys, &[("ip -4 -o addr show dev enp1s0", DHCP_ADRES)]);
        assert_eq!((d, c), (Durum::Kapali, vec!["ip -4 -o addr show dev enp1s0".to_string()]));
    }
}
