//! İnternet alan portta PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md).
//! DHCP hep açık kalır; PPPoE açıksa yanında pppd çalışır (`wificorrect-pppoe.service`) ve PPPoE rotası öne geçer.
//! Ayar `ag.toml` `[pppoe]` (src/ag.rs); pppd bağlantı dosyası (`/etc/ppp/peers/wificorrect`, 600) ondan üretilir.

use crate::ag::Ag;

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
    let p = &ag.pppoe;
    let mut s = format!(
        "# WifiCorrect üretir (ag.toml [pppoe]); {BIRIM} bunu okur. Elle değiştirmeyin.\n\
         plugin pppoe.so\nnic-{nic}\nifname {ARAYUZ}\nuser \"{user}\"\npassword \"{pass}\"\n\
         noauth\nhide-password\nnoipdefault\ndefaultroute\ndefaultroute-metric {METRIK}\nusepeerdns\n\
         persist\nmaxfail 0\nholdoff 10\nlcp-echo-interval 10\nlcp-echo-failure 6\nmtu {mtu}\nmru {mtu}\nnoipv6\n",
        nic = alt_arayuz(ag),
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
}
