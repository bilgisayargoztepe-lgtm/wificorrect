//! İnternet alan portta PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md).
//! DHCP hep açık kalır; PPPoE açıksa yanında pppd çalışır (`wificorrect-pppoe.service`) ve PPPoE rotası öne geçer.
//! Ayar `ag.toml` `[pppoe]` (src/ag.rs); pppd bağlantı dosyası (`/etc/ppp/peers/wificorrect`, 600) ondan üretilir.

use crate::ag::Ag;
use crate::ortak::Runner;

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
}
