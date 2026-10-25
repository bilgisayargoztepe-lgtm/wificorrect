//! İnternet alan portta PPPoE (docs/superpowers/specs/2026-10-23-pppoe-design.md).
//! DHCP hep açık kalır; PPPoE açıksa yanında pppd çalışır (`wificorrect-pppoe.service`) ve PPPoE rotası öne geçer.
//! Ayar `ag.toml` `[pppoe]` (src/ag.rs); pppd bağlantı dosyası (`/etc/ppp/peers/wificorrect`, 600) ondan üretilir.

use crate::ag::{Ag, Pppoe};
use crate::ortak::Runner;
use crate::portal::html_escape as h;
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
/// Yeniden deneme: `persist` kurulmuş bağlantı düşünce yeniden dener (`holdoff 10` sonra). `maxfail 1`: her başarısız denemeden
/// sonra pppd çıkar ve birim (`Restart=always`, `RestartSteps=5`, `RestartMaxDelaySec=300`) onu 10 sn'den 5 dk'ya çıkan gecikmeyle
/// yeniden başlatır; yani deneme sınırsızdır ama başarısızken yavaşlar (systemd bu sayacı kendiliğinden yeniden başlatmalar arasında
/// sıfırlamaz; her başarılı bağlantıdan sonra `/etc/ppp/ip-up.d/wificorrect` sıfırlar, kablo internet portuna takılınca
/// `/etc/dhcpcd.exit-hook` sıfırlayıp hemen yeniden başlatır). pppd'nin kendi beklemesine güvenilmez: yanlış kimlik YÖNTEMİ seçilince
/// (`pap`/`chap`, operatörün istediğinden farklı) LCP hiç kalkmaz ve `holdoff` uygulanmaz (sahte operatörde ~3,3 sn'de bir yeni
/// oturum: saatte ~1000 deneme); gerçek operatör (Türk.net, 2026-10-24) yanlış parolada da PADT ile oturumu kapatıyor, pppd yine
/// beklemeden deniyor (dakikada 9 deneme). Hesap kilitlenmesin diye 0.4.20'de `maxfail 3`'ten 1'e indi.
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
         persist\nmaxfail 1\nholdoff 10\nlcp-echo-interval 10\nlcp-echo-failure 6\nmtu {mtu}\nmru {mtu}\nnoipv6\n",
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

/// `dosya_yenile` sonucu.
#[derive(Debug, PartialEq)]
pub enum Yenileme {
    /// Dosya zaten ayardan üretilen metinle aynı (izni 600): dokunulmadı.
    Ayni,
    /// Dosya ayardan yeniden yazıldı (ör. paket güncellemesi pppd seçeneklerini değiştirdi).
    Yazildi,
    /// Ayarda PPPoE kapalı: dosya kaldırıldı (pppd başlamamalı; birimin sonraki denemesi `ConditionPathExists`'e takılır).
    Kapali,
}

/// Birim başlarken (`ctl pppoe-hazirla`, `hazirla`'dan önce): bağlantı dosyasını ag.toml'daki ayardan yeniden üretir. Dosyayı ayar
/// uygulanınca `ag::write_files` yazar, paket güncellemesi yazmaz: yeni sürümün pppd seçenekleri (0.4.20'de `maxfail 3` → 1) ancak
/// bir sonraki ayar değişikliğinde gelirdi (2026-10-24, Göztepe: dosya elle yeniden üretildi). pppd seçenekleri yalnızca başlarken
/// okur: her başlangıçta yenilemek güncellemeyle gelen seçeneği bir sonraki pppd başlangıcında geçerli kılar (çalışan pppd'ye ve
/// bağlantıya dokunulmaz). Metin ve izin (600: kullanıcı adı ve parola içerir) aynıysa dosyaya dokunulmaz. PPPoE kapalıysa dosya
/// kaldırılır: ayar kapalıyken pppd çalışmaz. Ayar geçersizse (`ag::pppoe_dogrula`; panel ve Servis geçersizini kaydetmez, ancak elle
/// bozulmuş ag.toml) dosyaya dokunulmaz ve hata döner: pppd son geçerli dosyayla başlar. Hata metinleri kullanıcı adını ve parolayı
/// taşımaz.
pub fn dosya_yenile(ag: &Ag, peers: &Path) -> Result<Yenileme, String> {
    use std::os::unix::fs::PermissionsExt;
    if !ag.pppoe.acik {
        return match std::fs::remove_file(peers) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{} silinemedi: {e}", peers.display())),
            _ => Ok(Yenileme::Kapali),
        };
    }
    crate::ag::pppoe_dogrula(&ag.pppoe)?;
    let metin = peers_text(ag);
    let izin_600 = std::fs::metadata(peers).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o600);
    if izin_600 && std::fs::read(peers).is_ok_and(|eski| eski == metin.as_bytes()) {
        return Ok(Yenileme::Ayni);
    }
    crate::ortak::write_atomic(peers, metin.as_bytes()).map_err(|e| format!("{} yazılamadı: {e}", peers.display()))?;
    Ok(Yenileme::Yazildi)
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

/// Açılışta geri almada (`ag::geri_al("acilis")`) birimi ayara getirir. Onaylanmamış bir değişiklik PPPoE'yi kapatmış (`disable --now`)
/// ve cihaz 3 dk dolmadan yeniden açılmışsa, geri yazılan eski ayarda PPPoE açıktır ama birimin etkinliği kalkmıştır: etkinleştirilmezse
/// ve başlatılmazsa müşteri bir sonraki elle ayara kadar internetsiz kalır. Bu iş `wificorrect-ag-acilis.service`ten (oneshot, sysinit,
/// `Before=networking.service`) çalışır: engelleyen bir `start`/`restart`, `After=network.target` olan pppd birimini bekler ve ağ bu iş
/// bitmeden kalkmadığı için kilitlenir (oneshot'ın başlatma zaman aşımı kapalıdır, `TimeoutStartUSec=infinity`: açılış sonsuza dek
/// takılırdı). Bu yüzden başlatma BEKLEMEDEN kuyruğa alınır (`--no-block`); ağ kalkınca birim kendiliğinden başlar. Kapalıysa
/// dokunulmaz: bağlantı dosyası da silinmiştir, birim `ConditionPathExists` ile atlanır. Hata metni dönerse çağıran hata listesine ekler.
pub fn birim_acilista(ag: &Ag, runner: &Runner) -> Option<String> {
    if !ag.pppoe.acik {
        return None;
    }
    let etkin = runner(&cmd(&["systemctl", "enable", BIRIM]));
    if !runner(&cmd(&["systemctl", "start", "--no-block", BIRIM])) {
        return Some("PPPoE başlatılamadı".into());
    }
    // başlayacak ama sonraki açılışlarda başlamayacak (etkinleştirilemedi): sessiz kalmasın
    (!etkin).then(|| "PPPoE açılışta kendiliğinden başlamayabilir (birim etkinleştirilemedi)".into())
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
    /// PPPoE açık değil; internet portu DHCP'den adres almış ama internete çıkılamıyor (etkin yoklama, `ag::internet_yoklamasi`):
    /// modemin interneti yok ya da kablo PPPoE isteyen bir hatta (ONT'nin VLAN'sız ağı adres verir ama internete çıkarmaz; 0.4.20)
    InternetYok,
    /// PPPoE sunucusu yanıt vermiyor (keşif zaman aşımı)
    SunucuYok,
    /// PPPoE açık ama bağlı değil (sunucu yanıt vermiyor ya da deneme sürüyor), internet portu modemden DHCP adresi almış ve
    /// internete çıkılıyor: modemli hatta internet modemden gelir, ONT'ye takılınca PPPoE bağlanır (sorun değil; 0.4.18; 0.4.20'den
    /// beri yalnızca etkin yoklama internete çıkılabildiğini gösterirse)
    Modemden,
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
            Durum::InternetYok => "internet_yok",
            Durum::SunucuYok => "sunucu_yok",
            Durum::Modemden => "modemden",
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
            Durum::InternetYok => "Adres alındı ama internete çıkılamıyor (modemin interneti yok ya da hat PPPoE istiyor)".into(),
            Durum::SunucuYok => "PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)".into(),
            Durum::Modemden => "Modemden bağlı (DHCP); PPPoE ONT'ye takılınca bağlanır".into(),
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
/// `dhcp_adres`: internet portunun DHCP'den aldığı IPv4 adresi var. `internet`: o adresle internete çıkılıyor (etkin yoklama;
/// yalnızca DHCP adresi varken anlamlı). `gunluk`: pppd birim günlüğünün son dakikaları (`journalctl -o cat`).
/// Sıra: kablo yoksa her şeyden önce `KabloYok` (kablo yeni çekildiyse pppd ~70 sn daha eski adresi tutar); kapalıysa DHCP (adres
/// var ama internet yoksa `InternetYok`); açıkta PPP adresi varsa `Bagli` (günlükteki eski hatalar geçersiz); yoksa günlük: kimlik
/// reddi DHCP çalışsa da gösterilir (bir PPPoE sunucusu yanıt verdi, bilgi yanlış); sunucu yoksa ya da deneme sürüyorsa ve DHCP
/// adresiyle internete çıkılıyorsa `Modemden` (modemli hat: internet modemden gelir, ONT'ye takılınca PPPoE bağlanır; 0.4.18'den
/// önce bu durumda kırmızı `SunucuYok` görünüyordu). Adres var ama internet yoksa (ONT'nin VLAN'sız ağı; 0.4.20) günlüğün durumu.
pub fn durum_coz(acik: bool, kablo: bool, ppp_ip: Option<&str>, dhcp_adres: bool, internet: bool, gunluk: &str) -> Durum {
    if !kablo {
        return Durum::KabloYok;
    }
    if !acik {
        return match (dhcp_adres, internet) {
            (false, _) => Durum::DhcpYok,
            (true, true) => Durum::Kapali,
            (true, false) => Durum::InternetYok,
        };
    }
    match (ppp_ip, gunluk_durumu(gunluk)) {
        (Some(ip), _) => Durum::Bagli(ip.to_string()),
        (None, Durum::KimlikHatasi) => Durum::KimlikHatasi,
        (None, _) if dhcp_adres && internet => Durum::Modemden,
        (None, d) => d,
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
/// `internet`: etkin internet yoklaması (sistemde `ag::internet_yoklamasi`; çağıran internetin olmadığını zaten biliyorsa `|| false`).
/// Yalnızca kararı etkileyecek veri okunur: kapalıyken günlük, bağlıyken ya da günlükte kimlik reddi varken DHCP adresi, kablo
/// yokken hiçbiri okunmaz; yoklama yalnızca DHCP adresi varken ve karar ona bağlıyken yapılır.
pub fn durum(ag: &Ag, sys: &Path, komut: &dyn Fn(&[&str]) -> Option<String>, internet: &dyn Fn() -> bool) -> Durum {
    let adres = |arayuz: &str| komut(&["ip", "-4", "-o", "addr", "show", "dev", arayuz]).and_then(|o| adres_coz(&o));
    let (acik, kablo) = (ag.pppoe.acik, crate::ag::link(sys, &ag.wan).0);
    let ppp_ip = if acik && kablo { adres(ARAYUZ) } else { None };
    let gunluk = if acik && kablo && ppp_ip.is_none() {
        komut(&["journalctl", "-u", BIRIM, "--since", "-10min", "-o", "cat", "--no-pager", "-q"]).unwrap_or_default()
    } else {
        String::new()
    };
    // DHCP adresi kapalıyken `Kapali`/`DhcpYok`, açıkken (bağlı değil, kimlik reddi yok) `Modemden` için gerekir
    let dhcp_sor = kablo && (!acik || (ppp_ip.is_none() && gunluk_durumu(&gunluk) != Durum::KimlikHatasi));
    let dhcp_adres = dhcp_sor && adres(&ag.wan).is_some();
    // adres internete çıkarıyor mu: ONT'nin VLAN'sız ağı adres ve rota verir ama internete çıkarmaz (2026-10-24, Göztepe)
    let internet = dhcp_adres && internet();
    durum_coz(acik, kablo, ppp_ip.as_deref(), dhcp_adres, internet, &gunluk)
}

// ---------------------------------------------------------------- form (panel → Portlar, giriş sayfası → Servis)

/// Sayı alanı: boş → `bos`; sayı değilse (ya da türe sığmıyorsa) `gecersiz`, yani `ag::pppoe_dogrula`nın reddedeceği aralık dışı bir değer.
fn sayi<T: std::str::FromStr>(s: &str, bos: T, gecersiz: T) -> T {
    if s.is_empty() {
        bos
    } else {
        s.parse().unwrap_or(gecersiz)
    }
}

/// Formdan PPPoE ayarı. Panel (Portlar) ve giriş sayfasındaki Servis aynı alan adlarını kullanır: `pppoe_acik`, `pppoe_kullanici`,
/// `pppoe_parola`, `pppoe_vlan`, `pppoe_oncelik`, `pppoe_mtu`, `pppoe_kimlik`, `pppoe_ac_adi`, `pppoe_servis_adi`.
/// `alan`: alan adı → gönderilen değer (alan hiç gelmediyse boş). Ayar burada doğrulanmaz: çağıran `ag::pppoe_dogrula` ile denetler.
/// - Açık yalnızca `pppoe_acik=acik` (`form_html`'in seçim kutusu) ya da `1` (panelin onay kutusu geleneği: başka bir form ya da
///   betik öyle gönderirse "kapalı" sayılıp sessizce kaydedilmesin) ise. Kullanıcı adı, AC adı ve servis adı baştan/sondan kırpılır;
///   parola olduğu gibi alınır (boşluk parolanın parçası olabilir).
/// - Boş parola = `onceki.parola`: parola hiçbir yerde geri gösterilmez, boş bırakmak "aynı kalsın" demektir. `ag::gecis` boş
///   parolayı birleştirmez; Portlar'da hazırlanan dosyaya eski parola buradan girer. Kullanıcı adı ve ileri alanlar `form_html`
///   ile kayıtlı değerlerle dolu gelir, "kapalı" gönderilince de korunur.
/// - Boş VLAN/öncelik = 0, boş MTU = 1492, boş kimlik = otomatik. Sayı olmayan ya da sığmayan değer doğrulamanın reddedeceği bir
///   değere çevrilir (VLAN 65535, öncelik 255, MTU 0): sessizce varsayılana dönüp yanlış ayar kaydetmez.
pub fn formdan(onceki: &Pppoe, alan: &dyn Fn(&str) -> String) -> Pppoe {
    let d = Pppoe::default();
    let metin = |k: &str| alan(k).trim().to_string();
    let parola = alan("pppoe_parola");
    let kimlik = metin("pppoe_kimlik");
    Pppoe {
        acik: matches!(metin("pppoe_acik").as_str(), "acik" | "1"),
        kullanici: metin("pppoe_kullanici"),
        parola: if parola.is_empty() { onceki.parola.clone() } else { parola },
        vlan: sayi(&metin("pppoe_vlan"), d.vlan, u16::MAX),
        oncelik: sayi(&metin("pppoe_oncelik"), d.oncelik, u8::MAX),
        mtu: sayi(&metin("pppoe_mtu"), d.mtu, 0),
        kimlik: if kimlik.is_empty() { d.kimlik } else { kimlik },
        ac_adi: metin("pppoe_ac_adi"),
        servis_adi: metin("pppoe_servis_adi"),
    }
}

/// Form alanlarının HTML'i: `<label>` + giriş çiftleri; ileri ayarlar `<details class="gelismis">` içinde (varsayılandan farklı bir
/// değer varsa açık gelir: gizli kalıp sessizce uygulanmasın). Form etiketi, CSRF, düğme, durum satırı ve notlar çağıranın.
/// Alanlar `p`nin kayıtlı değerleriyle dolu gelir ki "kapalı" gönderince kullanıcı adı ve ileri alanlar silinmesin; VLAN yokken boş.
/// Parolanın değeri hiçbir zaman yazılmaz (alan boş; yalnızca kayıtlı parola varsa "aynı kalır" ipucu). Yazılan her değer kaçışlanır.
pub fn form_html(p: &Pppoe) -> String {
    alanlar_html(p, &p.kullanici, "")
}

/// Servis formunda kullanıcı adının altındaki not.
const SERVIS_KULLANICI_NOTU: &str = "Boş bırakılırsa kayıtlı kullanıcı adı aynı kalır.";

/// Giriş sayfasındaki Servis formu: `form_html`'in alanları, ama kayıtlı kullanıcı adı yazılmaz (sayfa misafire açık, parola
/// doğrulanmadan açılır; ad çoğu kez abonenin telefon numarasıdır). Alana yalnızca `kullanici` (bu istekte yazılan; yoksa boş) gelir,
/// hemen altında "boş = aynı kalır" notu: boş gönderilirse kayıtlı adı çağıran (parola doğrulandıktan sonra) birleştirir.
pub fn servis_form_html(p: &Pppoe, kullanici: &str) -> String {
    alanlar_html(p, kullanici, &format!("<p class=\"not kucuk\">{SERVIS_KULLANICI_NOTU}</p>"))
}

/// `form_html` ve `servis_form_html`'in ortak gövdesi: kullanıcı adı alanının değeri `kullanici` (kaçışlanır), hemen ardından
/// `kullanici_notu` (güvenilir sabit HTML).
fn alanlar_html(p: &Pppoe, kullanici: &str, kullanici_notu: &str) -> String {
    let d = Pppoe::default();
    let secim = |ad: &str, secenekler: &[(&str, &str)], mevcut: &str| {
        let o: String = secenekler.iter().map(|(k, l)| format!("<option value=\"{k}\"{}>{l}</option>", if *k == mevcut { " selected" } else { "" })).collect();
        format!("<select id=\"{ad}\" name=\"{ad}\">{o}</select>")
    };
    let metin = |ad: &str, deger: &str, ek: &str| format!("<input type=\"text\" id=\"{ad}\" name=\"{ad}\" value=\"{}\"{ek}>", h(deger));
    let sayisal = " inputmode=\"numeric\" autocomplete=\"off\"";
    let adi = " maxlength=\"64\" autocomplete=\"off\" autocapitalize=\"none\" spellcheck=\"false\"";
    let acik = secim("pppoe_acik", &[("acik", "Açık"), ("kapali", "Kapalı")], if p.acik { "acik" } else { "kapali" });
    let kullanici_alani = metin("pppoe_kullanici", kullanici, " placeholder=\"5321234567@turk.net\" maxlength=\"128\" autocomplete=\"off\" autocapitalize=\"none\" spellcheck=\"false\"");
    let parola_ipucu = if p.parola.is_empty() { "" } else { " placeholder=\"Kayıtlı parola aynı kalır\"" };
    let vlan = metin("pppoe_vlan", &if p.vlan == 0 { String::new() } else { p.vlan.to_string() }, &format!(" placeholder=\"yok\"{sayisal}"));
    let oncelik = metin("pppoe_oncelik", &p.oncelik.to_string(), sayisal);
    let mtu = metin("pppoe_mtu", &p.mtu.to_string(), sayisal);
    let kimlik = secim("pppoe_kimlik", &[("otomatik", "Otomatik"), ("pap", "PAP"), ("chap", "CHAP")], &p.kimlik);
    let ac_adi = metin("pppoe_ac_adi", &p.ac_adi, adi);
    let servis_adi = metin("pppoe_servis_adi", &p.servis_adi, adi);
    let ileri_acik = if (p.oncelik, p.mtu, &p.kimlik, &p.ac_adi, &p.servis_adi) != (d.oncelik, d.mtu, &d.kimlik, &d.ac_adi, &d.servis_adi) { " open" } else { "" };
    format!(
        "<label for=\"pppoe_acik\">PPPoE</label>{acik}\
         <label for=\"pppoe_kullanici\">Kullanıcı adı</label>{kullanici_alani}{kullanici_notu}\
         <label for=\"pppoe_parola\">Parola (yalnızca yazılır; boş = aynı kalır)</label>\
         <input type=\"password\" id=\"pppoe_parola\" name=\"pppoe_parola\" value=\"\"{parola_ipucu} maxlength=\"128\" autocomplete=\"new-password\">\
         <label for=\"pppoe_vlan\">VLAN (boş = yok)</label>{vlan}\
         <details class=\"gelismis\"{ileri_acik}><summary>Gelişmiş</summary>\
         <label for=\"pppoe_oncelik\">VLAN önceliği (802.1p, 0-7)</label>{oncelik}\
         <label for=\"pppoe_mtu\">MTU (1280-1492)</label>{mtu}\
         <label for=\"pppoe_kimlik\">Kimlik doğrulama</label>{kimlik}\
         <label for=\"pppoe_ac_adi\">AC adı (çoğu hatta boş bırakılır)</label>{ac_adi}\
         <label for=\"pppoe_servis_adi\">Servis adı (çoğu hatta boş bırakılır)</label>{servis_adi}\
         </details>"
    )
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
            // her başarısız denemeden sonra pppd çıkar, birim artan gecikmeyle yeniden başlatır: gerçek operatör reddi PADT ile
            // kapatınca pppd'nin holdoff'u işlemiyor (maxfail 3 ile dakikada 9 deneme oldu, 2026-10-24)
            "maxfail 1",
            "holdoff 10",
            "lcp-echo-interval 10",
            "lcp-echo-failure 6",
            "mtu 1492",
            "mru 1492",
            "noipv6",
        ] {
            assert!(l.contains(&beklenen), "{beklenen:?} satırı yok:\n{t}");
        }
        assert!(!t.contains("maxfail 0") && !t.contains("maxfail 3"), "hızlı yeniden deneme operatörün hesabı kilitlemesine yol açabilir:\n{t}");
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
    fn dosya_ayardan_yenilenir() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let d = kanca_klasoru("peers");
        let peers = d.join("peers");
        let izin = || std::fs::metadata(&peers).unwrap().permissions().mode() & 0o777;
        let a = ag(acik());
        let yeni = peers_text(&a);
        // önceki sürümün yazdığı dosya (0.4.19: maxfail 3), yanlış izinle: ayardan yeniden yazılır, izni 600
        std::fs::write(&peers, yeni.replace("maxfail 1\n", "maxfail 3\n")).unwrap();
        std::fs::set_permissions(&peers, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(dosya_yenile(&a, &peers), Ok(Yenileme::Yazildi));
        assert_eq!((std::fs::read_to_string(&peers).unwrap(), izin()), (yeni.clone(), 0o600));
        // metin ve izin aynı: dosyaya dokunulmaz (aynı inode)
        let inode = std::fs::metadata(&peers).unwrap().ino();
        assert_eq!(dosya_yenile(&a, &peers), Ok(Yenileme::Ayni));
        assert_eq!(std::fs::metadata(&peers).unwrap().ino(), inode);
        // yalnızca izin farklı: yeniden yazılır
        std::fs::set_permissions(&peers, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(dosya_yenile(&a, &peers), Ok(Yenileme::Yazildi));
        assert_eq!(izin(), 0o600);
        // ayar değişti (VLAN): yeni arayüzle
        assert_eq!(dosya_yenile(&ag(Pppoe { vlan: 7, ..acik() }), &peers), Ok(Yenileme::Yazildi));
        assert!(std::fs::read_to_string(&peers).unwrap().contains("\nnic-enp3s0.7\n"));
        // geçersiz ayar (elle bozulmuş ag.toml): dosyaya dokunulmaz, hata metninde kullanıcı adı ve parola yok
        let once = std::fs::read(&peers).unwrap();
        let kotu = ag(Pppoe { kullanici: "kotu\nconnect /bin/sh".into(), parola: "sirli-parola".into(), ..acik() });
        let e = dosya_yenile(&kotu, &peers).unwrap_err();
        assert!(!e.contains("kotu") && !e.contains("connect") && !e.contains("sirli-parola"), "{e}");
        assert_eq!(std::fs::read(&peers).unwrap(), once);
        // PPPoE kapalı: dosya kaldırılır; zaten yoksa da hata değil
        assert_eq!(dosya_yenile(&ag(Pppoe { acik: false, ..acik() }), &peers), Ok(Yenileme::Kapali));
        assert!(!peers.exists());
        assert_eq!(dosya_yenile(&ag(Pppoe::default()), &peers), Ok(Yenileme::Kapali));
        // dosya yokken açık ayar: yazılır
        assert_eq!(dosya_yenile(&a, &peers), Ok(Yenileme::Yazildi));
        assert_eq!((std::fs::read_to_string(&peers).unwrap(), izin()), (yeni, 0o600));
        let _ = std::fs::remove_dir_all(&d);
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

    #[test]
    fn birim_acilista_beklemeden() {
        // açık: açılışta başlasın diye etkinleştirilir, başlatma BEKLEMEDEN kuyruğa alınır (ağ-acilis networking'den önce çalışan bir
        // oneshot; After=network.target olan birimi bekleyen start/restart onu sonsuza dek kilitler)
        let (r, c) = sahte(&[]);
        assert_eq!(birim_acilista(&ag(acik()), &r), None);
        assert_eq!(cagrilar(&c), ["systemctl enable wificorrect-pppoe.service", "systemctl start --no-block wificorrect-pppoe.service"]);
        // kapalı (bilgiler dolu olsa da): bağlantı dosyası yok, birim ConditionPathExists ile atlanır → hiçbir komut
        let (r, c) = sahte(&[]);
        assert_eq!(birim_acilista(&ag(Pppoe { acik: false, ..acik() }), &r), None);
        assert_eq!(birim_acilista(&ag(Pppoe::default()), &r), None);
        assert!(cagrilar(&c).is_empty());
        // başlatma kuyruğa alınamazsa: hata (enable hatası start'ı engellemez)
        let (r, c) = sahte(&["systemctl start"]);
        assert_eq!(birim_acilista(&ag(acik()), &r), Some("PPPoE başlatılamadı".into()));
        assert_eq!(cagrilar(&c).len(), 2);
        let (r, _) = sahte(&["systemctl"]);
        assert_eq!(birim_acilista(&ag(acik()), &r), Some("PPPoE başlatılamadı".into()));
        // çalışacak ama açılışta başlamayacak: ayrı uyarı
        let (r, _) = sahte(&["systemctl enable"]);
        assert_eq!(birim_acilista(&ag(acik()), &r), Some("PPPoE açılışta kendiliğinden başlamayabilir (birim etkinleştirilemedi)".into()));
    }

    // Birim dosyası, güvenlik duvarı kuralları ve kancalar kodla birlikte değişmeli: test.sh bu dosyaları da cihaza gönderir.
    const BIRIM_DOSYASI: &str = include_str!("../deploy/debian/etc/systemd/system/wificorrect-pppoe.service");
    const GUVENLIK_NFT: &str = include_str!("../deploy/debian/etc/wificorrect/guvenlik.nft");
    const HIZ_NFT: &str = include_str!("../deploy/debian/etc/wificorrect/hiz.nft");

    /// Kural dosyasının yorumsuz, boş olmayan satırları (baştaki/sondaki boşluk kırpılmış)
    fn nft_kurallari(nft: &str) -> Vec<&str> {
        nft.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
    }

    #[test]
    fn guvenlik_nft_yonetim_pppoe_arayuzunde_kapali() {
        // Yönetim (SSH 22, panel 8443, ping) özel kaynak adreslerinden yalnızca internet alan Ethernet'te; PPPoE arayüzünde hiç:
        // arkasında dükkân ağı yok, yalnızca operatör var (CGNAT ya da operatörün kendi adresleri özel aralıkta olabilir).
        // `$WAN` iki biçimde de gelir: eski tek ad (`iifname "enp3s0" iifname != "wfcppp"`) ve küme
        // (`iifname { "enp3s0", "wfcppp" } iifname != "wfcppp"`); kural metni ikisinde de aynıdır.
        let haric = format!("iifname $WAN iifname != \"{ARAYUZ}\" ip saddr {{ 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16 }} ");
        let kurallar = nft_kurallari(GUVENLIK_NFT);
        let yonetim: Vec<&str> = kurallar.iter().copied().filter(|l| l.starts_with("iifname $WAN ") && l.ends_with(" accept")).collect();
        assert_eq!(yonetim, [format!("{haric}tcp dport {{ 22, 8443 }} accept"), format!("{haric}icmp type echo-request accept")]);
        // `$WAN` kullanan her kural gözden geçirildi: misafir iletimi, NAT ve hız sayaçları PPPoE arayüzünü kapsamalı (misafir
        // trafiği wfcppp'den çıkar), yönetim kapsamamalı. Yeni bir `$WAN` kuralı eklenirse buraya ve PPPoE'deki anlamına bakılmalı.
        let wan: Vec<&str> = kurallar.iter().copied().chain(nft_kurallari(HIZ_NFT)).filter(|l| l.contains("$WAN")).collect();
        assert_eq!(
            wan,
            [
                yonetim[0],
                yonetim[1],
                "iifname \"br-hotspot\" oifname $WAN meta mark 0x5651 accept",
                "oifname $WAN ip saddr 10.50.0.0/24 masquerade",
                "iifname \"br-hotspot\" oifname $WAN update @yukle { ip saddr }",
                "iifname $WAN oifname \"br-hotspot\" update @indir { ip daddr }",
            ]
        );
        // PPPoE'de yönetim tünelden: tünel kuralları aynen
        assert!(kurallar.contains(&"iifname \"wfc\" ip saddr 10.99.0.0/24 tcp dport { 22, 8443 } accept"));
        assert!(kurallar.contains(&"iifname \"wfc\" ip saddr 10.99.0.0/24 icmp type echo-request accept"));
        assert!(kurallar.contains(&"iifname != \"wfc\" ip saddr 10.99.0.0/24 drop")); // tünel adresi başka arayüzden (wfcppp dahil) taklit edilemez
    }

    #[test]
    fn birim_dosyasi() {
        let satir = |s: &str| BIRIM_DOSYASI.lines().any(|l| l == s);
        let peers = crate::ag::Yollar::sistem().peers;
        // bağlantı dosyası yoksa (PPPoE kapalı) birim başlamaz; pppd o dosyayı adıyla okur
        assert!(satir(&format!("ConditionPathExists={}", peers.display())), "{BIRIM_DOSYASI}");
        assert!(satir(&format!("ExecStart=/usr/sbin/pppd call {} nodetach", peers.file_name().unwrap().to_str().unwrap())), "{BIRIM_DOSYASI}");
        assert!(satir("ExecStartPre=/usr/local/bin/wificorrect ctl pppoe-hazirla") && satir("ExecStopPost=/usr/local/bin/wificorrect ctl pppoe-temizle"));
        assert!(satir("Restart=always") && satir("RestartSec=10") && satir("After=network.target") && satir("WantedBy=multi-user.target"));
        // pppd her başarısız denemeden sonra çıkar (maxfail 1); yeniden başlatma aralığı 10 sn'den 5 adımda 5 dk'ya çıkar
        // (systemd 254+; Debian 13'te 257): yanlış kimlik bilgisiyle operatörü saniyede bir yoklayıp hesabı kilitletmeyelim
        assert!(satir("RestartSteps=5") && satir("RestartMaxDelaySec=300"), "{BIRIM_DOSYASI}");
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

    // ---------------------------------------------------------------- kancalar: dhcpcd (DNS) ve pppd ip-up (yeniden deneme sayacı)
    const DHCPCD_KANCASI: &str = include_str!("../deploy/debian/etc/dhcpcd.exit-hook");
    const IP_UP_KANCASI: &str = include_str!("../deploy/debian/etc/ppp/ip-up.d/wificorrect");

    /// Testin kendi geçici klasörü (boş başlar)
    fn kanca_klasoru(ad: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wfc-pppoe-kanca-{ad}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ozet(yol: &str) -> Option<Vec<u8>> {
        use sha2::{Digest, Sha256};
        std::fs::read(yol).ok().map(|b| Sha256::digest(&b).to_vec())
    }

    // dhcpcd'nin DNS'siz bir olaydan (IPv4LL, NOCARRIER) sonra yazdığı dosya ve pppd'nin `usepeerdns` ile yazdığı sunucular
    const DHCPCD_BOS: &str = "# Generated by dhcpcd\n# /etc/resolv.conf.head can replace this line\n# /etc/resolv.conf.tail can replace this line\n";
    const PPP_SUNUCULARI: &str = "nameserver 203.0.113.53\nnameserver 198.51.100.53\n";

    /// dhcpcd-run-hooks gibi kancayı "source" eden kabuk (`secenek`: `-c`, `-ec`, `-euc`); yollar `d` içinde. Dönen: çıktı (kancadan
    /// sonra `SONRA=<kod>` ve `set`: kancanın bıraktığı değişkenler)
    fn dhcpcd_kancasi(d: &std::path::Path, secenek: &str) -> String {
        let cikti = std::process::Command::new("sh")
            .args([secenek, ". \"$1\"; echo \"SONRA=$?\"; set", "sh"])
            .arg(d.join("kanca"))
            .env("WFC_NET", d.join("net"))
            .env("WFC_PPP_RESOLV", d.join("ppp-resolv.conf"))
            .env("WFC_RESOLV", d.join("resolv.conf"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&cikti.stdout).into_owned()
    }

    #[test]
    fn dhcpcd_kancasi_pppoe_dns_ini_geri_yazar() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let gercek = ozet("/etc/resolv.conf"); // sınama gerçek dosyaya dokunmamalı
        assert!(DHCPCD_KANCASI.contains(&format!("$_wfc_net/{ARAYUZ}/flags"))); // pppd'nin arayüzünün sabit adı
        let d = kanca_klasoru("dhcpcd");
        std::fs::write(d.join("kanca"), DHCPCD_KANCASI).unwrap();
        let rc = d.join("resolv.conf");
        let bayrak = |b: Option<&str>| {
            let _ = std::fs::remove_dir_all(d.join("net"));
            if let Some(b) = b {
                std::fs::create_dir_all(d.join("net/wfcppp")).unwrap();
                std::fs::write(d.join("net/wfcppp/flags"), format!("{b}\n")).unwrap();
            }
        };
        let oku = || std::fs::read_to_string(&rc).unwrap_or_default();
        // kanca her durumda: exit kullanmaz (sonraki komut çalışır), 0 döner, `set -e`/`-u` altında da durmaz, değişken bırakmaz
        let calistir = || {
            let mut son = String::new();
            for secenek in ["-c", "-ec", "-euc"] {
                son = dhcpcd_kancasi(&d, secenek);
                assert!(son.lines().any(|l| l == "SONRA=0"), "{secenek}: {son}");
                assert!(!son.lines().any(|l| l.starts_with("_wfc")), "{secenek}: değişken kaldı: {son}");
            }
            son
        };
        let beklenen = format!(
            "# PPP: This file was written by /etc/dhcpcd.exit-hook (WifiCorrect), as /etc/ppp/ip-up.d/0000usepeerdns does.\n\
             # PPP: The following entries were obtained using PPP:\n{PPP_SUNUCULARI}\
             # PPP: End of section rewritten by /etc/dhcpcd.exit-hook.\n{DHCPCD_BOS}"
        );

        // PPPoE bağlı (wfcppp: UP|POINTOPOINT|RUNNING|NOARP|MULTICAST), dhcpcd dosyayı sunucusuz yazmış: PPP sunucuları geri gelir,
        // dhcpcd'nin satırları kalır, dosya 644, geçici dosya kalmaz
        bayrak(Some("0x10d1"));
        std::fs::write(d.join("ppp-resolv.conf"), PPP_SUNUCULARI).unwrap();
        std::fs::write(&rc, DHCPCD_BOS).unwrap();
        calistir();
        assert_eq!(oku(), beklenen);
        assert_eq!(std::fs::metadata(&rc).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(!d.join("resolv.conf.wfc-yeni").exists());
        // sunucular zaten aynı: dosyaya dokunulmaz (aynı inode, aynı içerik)
        let inode = std::fs::metadata(&rc).unwrap().ino();
        calistir();
        assert_eq!((oku(), std::fs::metadata(&rc).unwrap().ino()), (beklenen.clone(), inode));
        // 0000usepeerdns'in kendi yazdığı dosya (PPP sunucuları aynı): dokunulmaz
        let usepeerdns = format!(
            "# PPP: This file was written by /etc/ppp/ip-up.d/0000usepeerdns.\n# PPP: The following entries were obtained using PPP:\n\
             {PPP_SUNUCULARI}# PPP: End of section rewritten by 0000usepeerdns.\n{DHCPCD_BOS}"
        );
        std::fs::write(&rc, &usepeerdns).unwrap();
        calistir();
        assert_eq!(oku(), usepeerdns);
        // dhcpcd modemin DNS'iyle yazmış (DHCP ve PPPoE ikisi de bağlı): PPP sunucuları geçer (PPPoE rotası önde), DHCP sunucusu
        // gider, `search` gibi öteki satırlar kalır
        std::fs::write(&rc, "# Generated by dhcpcd from enp3s0.dhcp\n# /etc/resolv.conf.head can replace this line\nsearch lan\nnameserver 192.168.1.1\n# /etc/resolv.conf.tail can replace this line\n").unwrap();
        calistir();
        let t = oku();
        assert!(t.starts_with("# PPP: This file was written by /etc/dhcpcd.exit-hook") && t.contains(PPP_SUNUCULARI), "{t}");
        assert!(t.contains("\nsearch lan\n") && t.contains("# Generated by dhcpcd from enp3s0.dhcp\n") && !t.contains("192.168.1.1"), "{t}");
        // /etc/resolv.conf yoksa oluşturulur
        std::fs::remove_file(&rc).unwrap();
        calistir();
        assert!(oku().contains(PPP_SUNUCULARI));

        // dokunulmayan durumlar: PPPoE arayüzü yok, var ama kapalı (pppd kurdu, IPCP bitmedi), bayraklar okunamıyor; PPP sunucusu yok
        for (b, ppp) in [
            (None, Some(PPP_SUNUCULARI)),
            (Some("0x1090"), Some(PPP_SUNUCULARI)),
            (Some(""), Some(PPP_SUNUCULARI)),
            (Some("bozuk"), Some(PPP_SUNUCULARI)),
            (Some("0x10d1"), None),
            (Some("0x10d1"), Some("")),
            (Some("0x10d1"), Some("# sunucu yok\n")),
        ] {
            bayrak(b);
            match ppp {
                Some(p) => std::fs::write(d.join("ppp-resolv.conf"), p).unwrap(),
                None => {
                    let _ = std::fs::remove_file(d.join("ppp-resolv.conf"));
                }
            }
            std::fs::write(&rc, DHCPCD_BOS).unwrap();
            calistir();
            assert_eq!(oku(), DHCPCD_BOS, "bayrak {b:?}, ppp {ppp:?}");
        }
        // /etc/resolv.conf bir bağlantı (resolvconf / systemd-resolved): dokunulmaz, bağlantı kalır
        bayrak(Some("0x10d1"));
        std::fs::write(d.join("ppp-resolv.conf"), PPP_SUNUCULARI).unwrap();
        std::fs::write(d.join("baska.conf"), DHCPCD_BOS).unwrap();
        std::fs::remove_file(&rc).unwrap();
        std::os::unix::fs::symlink(d.join("baska.conf"), &rc).unwrap();
        calistir();
        assert!(std::fs::symlink_metadata(&rc).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(d.join("baska.conf")).unwrap(), DHCPCD_BOS);
        // yazılamayan yer (klasör yok): hata vermez, geçici dosya bırakmaz
        let cikti = std::process::Command::new("sh")
            .args(["-euc", ". \"$1\"; echo \"SONRA=$?\"", "sh"])
            .arg(d.join("kanca"))
            .env("WFC_NET", d.join("net"))
            .env("WFC_PPP_RESOLV", d.join("ppp-resolv.conf"))
            .env("WFC_RESOLV", d.join("yok/resolv.conf"))
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&cikti.stdout).lines().any(|l| l == "SONRA=0"));
        assert!(!d.join("yok").exists());

        assert_eq!(ozet("/etc/resolv.conf"), gercek, "gerçek /etc/resolv.conf değişti");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ip_up_kancasi_yeniden_deneme_sayacini_sifirlar() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
        let d = kanca_klasoru("ip-up");
        let kanca = d.join("wificorrect");
        std::fs::write(&kanca, IP_UP_KANCASI).unwrap();
        std::fs::set_permissions(&kanca, std::fs::Permissions::from_mode(0o755)).unwrap();
        // sahte systemctl (PATH'te önde): argümanlarını kayıt dosyasına yazar; istenirse bekler ya da hata koduyla çıkar
        std::fs::create_dir_all(d.join("bin")).unwrap();
        let sahte_systemctl = d.join("bin/systemctl");
        std::fs::write(&sahte_systemctl, "#!/bin/sh\nsleep \"${WFC_BEKLE:-0}\"\necho \"$*\" >>\"$WFC_KAYIT\"\nexit \"${WFC_KOD:-0}\"\n").unwrap();
        std::fs::set_permissions(&sahte_systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", d.join("bin").display());
        // /etc/ppp/ip-up gibi: ortam temiz, PATH ve PPP_IFACE verilir; run-parts altı argüman geçirir. Dönen: (kod, süre)
        let calistir = |ppp_iface: Option<&str>, arayuz: &str, kayit: &str, ek: &[(&str, &str)]| {
            let mut c = std::process::Command::new(&kanca);
            c.env_clear().env("PATH", &path).env("WFC_KAYIT", d.join(kayit)).env("WFC_RESOLV", d.join("resolv.conf"));
            c.args([arayuz, "", "0", "203.0.113.10", "198.51.100.1", ""]);
            if let Some(p) = ppp_iface {
                c.env("PPP_IFACE", p);
            }
            for (k, v) in ek {
                c.env(k, v);
            }
            let t0 = Instant::now();
            let cikti = c.output().unwrap(); // arka plandaki komut çıktı borusunu tutsaydı burada beklenirdi
            (cikti.status.code(), t0.elapsed())
        };
        let bekle = |kayit: &str| {
            for _ in 0..60 {
                if let Ok(t) = std::fs::read_to_string(d.join(kayit)) {
                    if t.ends_with('\n') {
                        return t;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            String::new()
        };
        const BEKLENEN: &str = "reset-failed wificorrect-pppoe.service\n";
        assert_eq!(BEKLENEN, format!("reset-failed {BIRIM}\n")); // kanca birimin ve arayüzün sabit adlarını kullanır
        assert!(IP_UP_KANCASI.lines().any(|l| l == format!("{ARAYUZ})")));
        // başka bir PPP arayüzü (ör. elle kurulmuş ppp0): dokunulmaz
        assert_eq!(calistir(Some("ppp0"), "ppp0", "kayit-ppp0", &[]).0, Some(0));
        // wfcppp: sayaç sıfırlanır (tam olarak bu komut, bir kez)
        assert_eq!(calistir(Some("wfcppp"), "wfcppp", "kayit-wfcppp", &[]).0, Some(0));
        assert_eq!(bekle("kayit-wfcppp"), BEKLENEN);
        // PPP_IFACE yoksa ilk argüman
        assert_eq!(calistir(None, "wfcppp", "kayit-arg", &[]).0, Some(0));
        assert_eq!(bekle("kayit-arg"), BEKLENEN);
        // systemctl hata verse de kanca 0 ile çıkar (pppd'nin ip-up betiği hiç başarısız olmaz)
        assert_eq!(calistir(Some("wfcppp"), "wfcppp", "kayit-hata", &[("WFC_KOD", "1")]).0, Some(0));
        assert_eq!(bekle("kayit-hata"), BEKLENEN);
        // systemctl hiç yoksa da
        let mut c = std::process::Command::new(&kanca);
        c.env_clear().env("PATH", "/yok/bin").env("PPP_IFACE", "wfcppp").env("WFC_RESOLV", d.join("resolv.conf")).arg("wfcppp");
        assert_eq!(c.output().unwrap().status.code(), Some(0));
        // systemctl yavaş olsa da (PID 1 meşgul) pppd beklemez: kanca hemen döner, komut arkada biter
        let (kod, sure) = calistir(Some("wfcppp"), "wfcppp", "kayit-yavas", &[("WFC_BEKLE", "2")]);
        assert_eq!(kod, Some(0));
        assert!(sure < Duration::from_millis(1500), "kanca bekledi: {sure:?}");
        assert_eq!(bekle("kayit-yavas"), BEKLENEN);
        // ppp0 için (en başta) hiç çağrılmadı
        assert!(!d.join("kayit-ppp0").exists());

        // 0000usepeerdns'in yedeği silinir (koparken bayat DNS geri gelmesin; 2026-10-24); bağlantıysa gerçek dosyanın yanındaki
        let rc = d.join("resolv.conf");
        let yedek = d.join("resolv.conf.pppd-backup.wfcppp");
        std::fs::write(&rc, "nameserver 203.0.113.53\n").unwrap();
        for ad in ["resolv.conf.pppd-backup.wfcppp", "resolv.conf.pppd-backup.ppp0", "resolv.conf.wfc-yeni"] {
            std::fs::write(d.join(ad), "nameserver 10.98.153.1\n").unwrap();
        }
        assert_eq!(calistir(Some("ppp0"), "ppp0", "kayit-ppp0-yedek", &[]).0, Some(0));
        assert!(yedek.exists(), "ppp0'da wfcppp'nin yedeğine dokunulmaz");
        assert_eq!(calistir(Some("wfcppp"), "wfcppp", "kayit-yedek", &[]).0, Some(0));
        assert!(!yedek.exists() && d.join("resolv.conf.pppd-backup.ppp0").exists() && d.join("resolv.conf.wfc-yeni").exists());
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), "nameserver 203.0.113.53\n"); // resolv.conf'un kendisine dokunulmaz
        assert_eq!(bekle("kayit-yedek"), BEKLENEN);
        // /etc/resolv.conf bir bağlantıysa yedek gerçek dosyanın yanındadır (0000usepeerdns `readlink --canonicalize` kullanır)
        std::fs::create_dir_all(d.join("gercek")).unwrap();
        std::fs::write(d.join("gercek/resolv.conf"), "").unwrap();
        std::fs::write(d.join("gercek/resolv.conf.pppd-backup.wfcppp"), "").unwrap();
        std::fs::remove_file(&rc).unwrap();
        std::os::unix::fs::symlink(d.join("gercek/resolv.conf"), &rc).unwrap();
        assert_eq!(calistir(Some("wfcppp"), "wfcppp", "kayit-bag", &[]).0, Some(0));
        assert!(!d.join("gercek/resolv.conf.pppd-backup.wfcppp").exists());
        // yedek yoksa da sorun yok
        assert_eq!(calistir(Some("wfcppp"), "wfcppp", "kayit-yedeksiz", &[]).0, Some(0));
        let _ = std::fs::remove_dir_all(&d);
    }

    const IP_DOWN_KANCASI: &str = include_str!("../deploy/debian/etc/ppp/ip-down.d/wificorrect");

    #[test]
    fn ip_down_kancasi_dns_i_dhcp_den_yeniden_kurdurur() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
        let d = kanca_klasoru("ip-down");
        let kanca = d.join("wificorrect");
        std::fs::write(&kanca, IP_DOWN_KANCASI).unwrap();
        std::fs::set_permissions(&kanca, std::fs::Permissions::from_mode(0o755)).unwrap();
        // sahte dhcpcd (PATH'te önde): argümanlarını kayıt dosyasına yazar; istenirse bekler ya da hata koduyla çıkar
        std::fs::create_dir_all(d.join("bin")).unwrap();
        let sahte = d.join("bin/dhcpcd");
        std::fs::write(&sahte, "#!/bin/sh\nsleep \"${WFC_BEKLE:-0}\"\necho \"$*\" >>\"$WFC_KAYIT\"\nexit \"${WFC_KOD:-0}\"\n").unwrap();
        std::fs::set_permissions(&sahte, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", d.join("bin").display());
        // internet portunun çalışan dhcpcd'si: denetim soketi (gerçek UNIX soketi) ve yaşayan süreç (bu test)
        let run = d.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let soket = std::os::unix::net::UnixListener::bind(run.join("enp3s0.sock")).unwrap();
        std::fs::write(run.join("enp3s0.pid"), format!("{}\n", std::process::id())).unwrap();
        // /etc/ppp/ip-down gibi: PPP_IFACE ve PPP_TTY (pppd'nin aygıtı: PPPoE'de Ethernet ya da VLAN arayüzü) ve altı argüman
        let calistir = |iface: &str, tty: &str, kayit: &str, ek: &[(&str, &str)]| {
            let mut c = std::process::Command::new(&kanca);
            c.env_clear().env("PATH", &path).env("WFC_KAYIT", d.join(kayit)).env("WFC_DHCPCD_RUN", &run);
            c.env("PPP_IFACE", iface).env("PPP_TTY", tty).args([iface, tty, "0", "203.0.113.10", "198.51.100.1", ""]);
            for (k, v) in ek {
                c.env(k, v);
            }
            let t0 = Instant::now();
            let cikti = c.output().unwrap(); // arka plandaki komut çıktı borusunu tutsaydı burada beklenirdi
            (cikti.status.code(), t0.elapsed())
        };
        let bekle = |kayit: &str| {
            for _ in 0..30 {
                if let Ok(t) = std::fs::read_to_string(d.join(kayit)) {
                    if t.ends_with('\n') {
                        return t;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            String::new()
        };
        assert!(IP_DOWN_KANCASI.lines().any(|l| l == format!("{ARAYUZ})")));
        // wfcppp koptu: internet portunun dhcpcd'sine "yeniden bağlan" denir (VLAN'lı aygıt da, `nic-` önekli yazım da)
        for (tty, kayit) in [("enp3s0", "k-duz"), ("enp3s0.35", "k-vlan"), ("nic-enp3s0", "k-nic")] {
            assert_eq!(calistir("wfcppp", tty, kayit, &[]).0, Some(0), "{tty}");
            assert_eq!(bekle(kayit), "-n enp3s0\n", "{tty}");
        }
        // dhcpcd yavaş olsa da pppd beklemez
        let (kod, sure) = calistir("wfcppp", "enp3s0", "k-yavas", &[("WFC_BEKLE", "2")]);
        assert_eq!(kod, Some(0));
        assert!(sure < Duration::from_millis(1500), "kanca bekledi: {sure:?}");
        assert_eq!(bekle("k-yavas"), "-n enp3s0\n");
        // dhcpcd hata verse de kanca 0 ile çıkar
        assert_eq!(calistir("wfcppp", "enp3s0", "k-hata", &[("WFC_KOD", "1")]).0, Some(0));
        assert_eq!(bekle("k-hata"), "-n enp3s0\n");
        // dokunulmayanlar (dhcpcd çağrılmaz): başka PPP arayüzü; o portun dhcpcd'si yok (soket yok: `dhcpcd -n` yeni bir dhcpcd
        // başlatırdı); aygıt adı boş ya da tuhaf
        let mut hic = vec![calistir("ppp0", "enp3s0", "k-ppp0", &[]), calistir("wfcppp", "enp1s0", "k-baska", &[])];
        hic.push(calistir("wfcppp", "", "k-bos", &[]));
        hic.push(calistir("wfcppp", "../enp3s0", "k-tuhaf", &[]));
        hic.push(calistir("wfcppp", "enp3s0 x", "k-bosluk", &[]));
        // pid dosyası yok ya da süreç ölü (soket bayat)
        std::fs::remove_file(run.join("enp3s0.pid")).unwrap();
        hic.push(calistir("wfcppp", "enp3s0", "k-pidsiz", &[]));
        std::fs::write(run.join("enp3s0.pid"), "999999999\n").unwrap();
        hic.push(calistir("wfcppp", "enp3s0", "k-olu", &[]));
        // soket yerine düz dosya
        std::fs::write(run.join("enp3s0.pid"), format!("{}\n", std::process::id())).unwrap();
        drop(soket);
        std::fs::remove_file(run.join("enp3s0.sock")).unwrap();
        std::fs::write(run.join("enp3s0.sock"), "").unwrap();
        hic.push(calistir("wfcppp", "enp3s0", "k-dosya", &[]));
        assert!(hic.iter().all(|(k, _)| *k == Some(0)), "{hic:?}");
        std::thread::sleep(Duration::from_millis(300));
        for k in ["k-ppp0", "k-baska", "k-bos", "k-tuhaf", "k-bosluk", "k-pidsiz", "k-olu", "k-dosya"] {
            assert!(!d.join(k).exists(), "{k}: dhcpcd çağrılmamalıydı");
        }
        // dhcpcd hiç yoksa da 0
        let mut c = std::process::Command::new(&kanca);
        c.env_clear().env("PATH", "/yok/bin").env("PPP_IFACE", "wfcppp").env("PPP_TTY", "enp3s0").env("WFC_DHCPCD_RUN", &run);
        assert_eq!(c.args(["wfcppp", "enp3s0"]).output().unwrap().status.code(), Some(0));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dhcpcd_kancasi_kablo_takilinca_pppoe_yi_hemen_dener() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let d = kanca_klasoru("carrier");
        std::fs::write(d.join("kanca"), DHCPCD_KANCASI).unwrap();
        // sahte systemctl: her çağrının argümanları bir satır
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/systemctl"), "#!/bin/sh\necho \"$*\" >>\"$WFC_KAYIT\"\n").unwrap();
        std::fs::set_permissions(d.join("bin/systemctl"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/usr/bin:/bin", d.join("bin").display());
        let peers = d.join("peers");
        let bayrak = |b: Option<&str>| {
            let _ = std::fs::remove_dir_all(d.join("net"));
            if let Some(b) = b {
                std::fs::create_dir_all(d.join("net/wfcppp")).unwrap();
                std::fs::write(d.join("net/wfcppp/flags"), format!("{b}\n")).unwrap();
            }
        };
        // dhcpcd-run-hooks gibi "source" eder (set -eu altında da durmamalı); reason ve interface dhcpcd'den
        let calistir = |reason: &str, interface: &str, kayit: &str| {
            let cikti = std::process::Command::new("sh")
                .args(["-euc", ". \"$1\"; echo \"SONRA=$?\"", "sh"])
                .arg(d.join("kanca"))
                .env_clear()
                .env("PATH", &path)
                .env("WFC_KAYIT", d.join(kayit))
                .env("WFC_NET", d.join("net"))
                .env("WFC_PPP_RESOLV", d.join("ppp-resolv.conf"))
                .env("WFC_RESOLV", d.join("resolv.conf"))
                .env("WFC_PEERS", &peers)
                .env("reason", reason)
                .env("interface", interface)
                .output()
                .unwrap();
            let t = String::from_utf8_lossy(&cikti.stdout).into_owned();
            assert!(t.lines().any(|l| l == "SONRA=0"), "{reason} {interface}: {t}");
        };
        let bekle = |kayit: &str| {
            for _ in 0..30 {
                if let Ok(t) = std::fs::read_to_string(d.join(kayit)) {
                    if t.lines().count() == 2 {
                        return t;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            std::fs::read_to_string(d.join(kayit)).unwrap_or_default()
        };
        const BEKLENEN: &str = "reset-failed wificorrect-pppoe.service\nrestart --no-block wificorrect-pppoe.service\n";
        assert_eq!(BEKLENEN, format!("reset-failed {BIRIM}\nrestart --no-block {BIRIM}\n"));
        // PPPoE açık (bağlantı dosyası var), bağlı değil (wfcppp yok ya da kapalı), kablo internet portuna takıldı: hemen yeniden dener
        let mut a = ag(acik());
        a.pppoe.vlan = 0;
        std::fs::write(&peers, peers_text(&a)).unwrap();
        assert!(peers_text(&a).lines().any(|l| l == "nic-enp3s0"));
        bayrak(None);
        calistir("CARRIER", "enp3s0", "k-takildi");
        assert_eq!(bekle("k-takildi"), BEKLENEN);
        bayrak(Some("0x1090")); // arayüz var ama kapalı
        calistir("CARRIER", "enp3s0", "k-kapali-arayuz");
        assert_eq!(bekle("k-kapali-arayuz"), BEKLENEN);
        // VLAN'lı hatta da (pppd enp3s0.35'te, dhcpcd enp3s0'da)
        std::fs::write(&peers, peers_text(&ag(acik()))).unwrap();
        assert!(peers_text(&ag(acik())).lines().any(|l| l == "nic-enp3s0.35"));
        calistir("CARRIER", "enp3s0", "k-vlan");
        assert_eq!(bekle("k-vlan"), BEKLENEN);
        // dokunulmayanlar: PPPoE zaten bağlı; başka olay; başka arayüz; PPPoE kapalı (bağlantı dosyası yok)
        bayrak(Some("0x10d1"));
        calistir("CARRIER", "enp3s0", "k-bagli");
        bayrak(None);
        const OLAYLAR: [&str; 7] = ["NOCARRIER", "BOUND", "RENEW", "REBOOT", "IPV4LL", "PREINIT", "STOPPED"];
        for olay in OLAYLAR {
            calistir(olay, "enp3s0", &format!("k-{olay}"));
        }
        calistir("CARRIER", "enp1s0", "k-baska");
        calistir("CARRIER", "", "k-bos");
        std::fs::remove_file(&peers).unwrap();
        calistir("CARRIER", "enp3s0", "k-kapali");
        std::thread::sleep(Duration::from_millis(300));
        let mut olmamali = vec!["k-bagli".to_string(), "k-baska".into(), "k-bos".into(), "k-kapali".into()];
        olmamali.extend(OLAYLAR.map(|o| format!("k-{o}")));
        for k in olmamali {
            assert!(!d.join(&k).exists(), "{k}: systemctl çağrılmamalıydı");
        }
        let _ = std::fs::remove_dir_all(&d);
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
        durum_coz(true, true, None, false, false, gunluk)
    }

    #[test]
    fn durum_kapali() {
        assert_eq!(durum_coz(false, false, None, false, false, ""), Durum::KabloYok);
        assert_eq!(durum_coz(false, false, None, true, true, ""), Durum::KabloYok); // kablo yoksa eski adres önemsiz
        assert_eq!(durum_coz(false, true, None, false, false, ""), Durum::DhcpYok);
        assert_eq!(durum_coz(false, true, None, true, true, ""), Durum::Kapali);
        // adres var ama internete çıkılamıyor (modemin interneti yok ya da kablo PPPoE isteyen hatta; 0.4.20)
        assert_eq!(durum_coz(false, true, None, true, false, ""), Durum::InternetYok);
        // PPPoE kapalıyken eski günlük ve kalıntı PPP adresi durumu değiştirmez
        assert_eq!(durum_coz(false, true, None, true, true, KIMLIK_CHAP), Durum::Kapali);
        assert_eq!(durum_coz(false, true, Some("85.1.2.3"), true, true, SUNUCU_YOK), Durum::Kapali);
        assert_eq!(durum_coz(false, true, Some("85.1.2.3"), true, false, SUNUCU_YOK), Durum::InternetYok);
        assert_eq!(durum_coz(false, true, None, false, false, SUNUCU_YOK), Durum::DhcpYok);
    }

    #[test]
    fn durum_acik_fixture() {
        let coz = |kablo: bool, ip: Option<&str>, gunluk: &str| durum_coz(true, kablo, ip, false, false, gunluk);
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
        assert_eq!(durum_coz(true, true, Some("85.1.2.3"), true, true, ""), Durum::Bagli("85.1.2.3".into()));
        assert_eq!(durum_coz(true, true, Some("85.1.2.3"), true, true, KIMLIK_CHAP), Durum::Bagli("85.1.2.3".into()));
        // modemli hat (dükkân ya da modemi olan müşteri): PPPoE sunucusu yok ya da deneme sürüyor, internet modemden geliyor
        assert_eq!(durum_coz(true, true, None, true, true, SUNUCU_YOK), Durum::Modemden);
        assert_eq!(durum_coz(true, true, None, true, true, ""), Durum::Modemden);
        assert_eq!(durum_coz(true, true, None, true, true, &format!("{BAGLI}{DUSTU}")), Durum::Modemden); // ONT'den modeme geçildi
        assert_eq!(durum_coz(true, true, None, true, true, &format!("{KIMLIK_CHAP}{SUNUCU_YOK}")), Durum::Modemden); // en son olay: sunucu yok
        // adres var ama internete çıkılamıyor (ONT'nin VLAN'sız ağı: 2026-10-24, Göztepe): yeşil "Modemden" değil, PPPoE'nin
        // kendi durumu (VLAN gerekli olabilir)
        assert_eq!(durum_coz(true, true, None, true, false, SUNUCU_YOK), Durum::SunucuYok);
        assert_eq!(durum_coz(true, true, None, true, false, ""), Durum::Baglaniyor);
        // kimlik reddi DHCP çalışsa da gösterilir (bir PPPoE sunucusu yanıt verdi, bilgi yanlış)
        assert_eq!(durum_coz(true, true, None, true, true, KIMLIK_CHAP), Durum::KimlikHatasi);
        assert_eq!(durum_coz(true, true, None, true, true, KIMLIK_YONTEMI), Durum::KimlikHatasi);
        // DHCP adresi yoksa PPPoE'nin kendi durumu (açıkken "DHCP yok" durumu yoktur)
        assert_eq!(durum_coz(true, true, None, false, false, SUNUCU_YOK), Durum::SunucuYok);
        assert_eq!(durum_coz(true, true, None, false, false, ""), Durum::Baglaniyor);
        // kablo yoksa DHCP adresi önemsiz
        assert_eq!(durum_coz(true, false, None, true, true, SUNUCU_YOK), Durum::KabloYok);
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
            (Durum::InternetYok, "internet_yok", "Adres alındı ama internete çıkılamıyor (modemin interneti yok ya da hat PPPoE istiyor)"),
            (Durum::SunucuYok, "sunucu_yok", "PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)"),
            (Durum::Modemden, "modemden", "Modemden bağlı (DHCP); PPPoE ONT'ye takılınca bağlanır"),
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

    /// Sahte etkin yoklamanın çağrı kaydındaki adı
    const YOKLAMA: &str = "YOKLAMA";

    /// `durum`u sahte komut çıktılarıyla çalıştırır (tabloda olmayan komut `None` = çalışmadı/zaman aşımı); çağrılan komutlar
    /// (ve etkin yoklama, `YOKLAMA`) sırayla döner. Yoklama internete çıkıldığını söyler (`durum_dene_net`: verilen sonuç).
    fn durum_dene(a: &Ag, sys: &Path, cikti: &[(&str, &str)]) -> (Durum, Vec<String>) {
        durum_dene_net(a, sys, cikti, true)
    }

    fn durum_dene_net(a: &Ag, sys: &Path, cikti: &[(&str, &str)], internet: bool) -> (Durum, Vec<String>) {
        let cagrilar = std::cell::RefCell::new(Vec::<String>::new());
        let komut = |c: &[&str]| {
            let s = c.join(" ");
            cagrilar.borrow_mut().push(s.clone());
            cikti.iter().find(|(k, _)| *k == s).map(|(_, v)| v.to_string())
        };
        let yokla = || {
            cagrilar.borrow_mut().push(YOKLAMA.to_string());
            internet
        };
        let d = durum(a, sys, &komut, &yokla);
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
        // kablo var, DHCP adresi var: yalnızca internet portunun adresi sorulur (wfcppp ve günlük okunmaz), sonra etkin yoklama
        let sys = sys_dizini("k-dhcp", Some("1\n"));
        let (d, c) = durum_dene(&kapali, &sys, &[(IP_WAN, DHCP_ADRES)]);
        assert_eq!((d, c), (Durum::Kapali, vec![IP_WAN.to_string(), YOKLAMA.to_string()]));
        // adres var ama yoklama internete çıkamadı: modemin interneti yok ya da hat PPPoE istiyor
        let (d, c) = durum_dene_net(&kapali, &sys, &[(IP_WAN, DHCP_ADRES)], false);
        assert_eq!((d, c), (Durum::InternetYok, vec![IP_WAN.to_string(), YOKLAMA.to_string()]));
        // DHCP yanıt vermiyor: adres yok, ya da yalnızca dhcpcd'nin 169.254 adresi, ya da komut çalışmadı (yoklama yapılmaz)
        let (d, c) = durum_dene(&kapali, &sys, &[(IP_WAN, "")]);
        assert_eq!((d, c), (Durum::DhcpYok, vec![IP_WAN.to_string()]));
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
        // adres yok: pppd birim günlüğünün son 10 dakikası okunur; kimlik reddi yoksa internet portunun (DHCP) adresi de sorulur,
        // adres varsa internete çıkıyor mu diye etkin yoklama yapılır
        let yok = "Device \"wfcppp\" does not exist.\n";
        for (gunluk, dhcp, internet, beklenen) in [
            (KIMLIK_CHAP, DHCP_ADRES, true, Durum::KimlikHatasi),
            (KIMLIK_PAP, DHCP_ADRES, true, Durum::KimlikHatasi),
            (KIMLIK_YONTEMI, "", true, Durum::KimlikHatasi),
            (SUNUCU_YOK, "", true, Durum::SunucuYok),
            (SUNUCU_YOK, YEREL_ADRES, true, Durum::SunucuYok), // dhcpcd'nin 169.254 adresi modem sayılmaz (ONT DHCP vermiyor)
            (SUNUCU_YOK, DHCP_ADRES, true, Durum::Modemden),
            // ONT'nin VLAN'sız ağı: adres var ama internet yok (2026-10-24, Göztepe): yeşil "Modemden" değil
            (SUNUCU_YOK, DHCP_ADRES, false, Durum::SunucuYok),
            ("", "", true, Durum::Baglaniyor),
            ("", DHCP_ADRES, true, Durum::Modemden),
            ("", DHCP_ADRES, false, Durum::Baglaniyor),
        ] {
            let (d, c) = durum_dene_net(&a, &sys, &[(IP_PPP, yok), (GUNLUK_KOMUTU, gunluk), (IP_WAN, dhcp)], internet);
            assert_eq!(d, beklenen, "{gunluk:?} {dhcp:?} {internet}");
            let sorulan = match (&beklenen, dhcp) {
                (Durum::KimlikHatasi, _) => vec![IP_PPP, GUNLUK_KOMUTU],
                (_, DHCP_ADRES) => vec![IP_PPP, GUNLUK_KOMUTU, IP_WAN, YOKLAMA],
                _ => vec![IP_PPP, GUNLUK_KOMUTU, IP_WAN],
            };
            assert_eq!(c, sorulan, "{gunluk:?} {dhcp:?} {internet}");
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
        assert_eq!((d, c), (Durum::Kapali, vec!["ip -4 -o addr show dev enp1s0".to_string(), YOKLAMA.to_string()]));
    }

    // ---------------------------------------------------------------- form (Portlar ve Servis aynı alanları kullanır)

    /// Sahte form: verilen çiftler dışındaki alanlar boş gelir
    fn form_alani<'a>(f: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> String + 'a {
        move |k: &str| f.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string()).unwrap_or_default()
    }

    /// `name="…"` taşıyan etiket (`<input …>` ya da `<select …>` açılışı)
    fn etiket<'a>(html: &'a str, ad: &str) -> &'a str {
        let i = html.find(&format!("name=\"{ad}\"")).unwrap_or_else(|| panic!("{ad} alanı yok:\n{html}"));
        let bas = html[..i].rfind('<').unwrap();
        &html[bas..i + html[i..].find('>').unwrap() + 1]
    }

    /// Alanın tarayıcıda görünen değeri (ham HTML'den): giriş → value, seçim → seçili seçeneğin değeri
    fn gorunen(html: &str, ad: &str) -> String {
        let i = html.find(&format!("name=\"{ad}\"")).unwrap_or_else(|| panic!("{ad} alanı yok"));
        let bas = html[..i].rfind('<').unwrap();
        if html[bas..].starts_with("<select") {
            let secenekler = &html[i..i + html[i..].find("</select>").unwrap()];
            let j = secenekler.find("\" selected>").unwrap_or_else(|| panic!("{ad}: seçili seçenek yok"));
            secenekler[secenekler[..j].rfind("value=\"").unwrap() + 7..j].to_string()
        } else {
            let tag = &html[bas..i + html[i..].find('>').unwrap()];
            let v = tag.find(" value=\"").unwrap() + 8;
            tag[v..v + tag[v..].find('"').unwrap()].to_string()
        }
    }

    #[test]
    fn formdan_bos_alanlar() {
        // hiçbir alan gelmedi: kapalı, bilgiler boş, varsayılanlar; parola yalnızca önceki varsa kalır
        let onceki = Pppoe { parola: "eski".into(), ..acik() };
        assert_eq!(formdan(&onceki, &form_alani(&[])), Pppoe { parola: "eski".into(), ..Pppoe::default() });
        // boş VLAN/öncelik → 0, boş MTU → 1492, boş kimlik → otomatik; kayıtlı (varsayılandan farklı) değerler korunmaz
        let onceki = Pppoe { oncelik: 3, mtu: 1400, kimlik: "pap".into(), ac_adi: "AC1".into(), servis_adi: "s".into(), ..acik() };
        let f = [
            ("pppoe_acik", "acik"),
            ("pppoe_kullanici", "k"),
            ("pppoe_parola", "p"),
            ("pppoe_vlan", ""),
            ("pppoe_oncelik", " "),
            ("pppoe_mtu", ""),
            ("pppoe_kimlik", ""),
            ("pppoe_ac_adi", ""),
            ("pppoe_servis_adi", ""),
        ];
        assert_eq!(formdan(&onceki, &form_alani(&f)), Pppoe { acik: true, kullanici: "k".into(), parola: "p".into(), ..Pppoe::default() });
    }

    #[test]
    fn formdan_degerler_ve_kirpma() {
        let f = [
            ("pppoe_acik", " acik "),
            ("pppoe_kullanici", "  5321234567@turk.net \t"),
            ("pppoe_parola", " bir parola "),
            ("pppoe_vlan", " 35 "),
            ("pppoe_oncelik", "7"),
            ("pppoe_mtu", "1280"),
            ("pppoe_kimlik", "chap"),
            ("pppoe_ac_adi", " AC-1 "),
            ("pppoe_servis_adi", "srv.2_x"),
        ];
        let p = formdan(&Pppoe::default(), &form_alani(&f));
        // kullanıcı adı, AC adı ve servis adı kırpılır; parola olduğu gibi (boşluk parolanın parçası)
        assert_eq!(p.kullanici, "5321234567@turk.net");
        assert_eq!(p.parola, " bir parola ");
        assert_eq!((p.acik, p.vlan, p.oncelik, p.mtu, p.kimlik.as_str()), (true, 35, 7, 1280, "chap"));
        assert_eq!((p.ac_adi.as_str(), p.servis_adi.as_str()), ("AC-1", "srv.2_x"));
        assert_eq!(crate::ag::pppoe_dogrula(&p), Ok(()));
        // açık yalnızca "acik" (seçim kutusu) ve "1" (onay kutusu geleneği); başka her değer kapalı
        for v in ["acik", "1"] {
            assert!(formdan(&Pppoe::default(), &form_alani(&[("pppoe_acik", v)])).acik, "{v:?}");
        }
        for v in ["", "kapali", "0", "evet", "on", "true", "ACIK"] {
            assert!(!formdan(&Pppoe::default(), &form_alani(&[("pppoe_acik", v)])).acik, "{v:?}");
        }
    }

    #[test]
    fn formdan_parola_bos_eskisi() {
        let onceki = Pppoe { parola: "eski-parola".into(), ..acik() };
        assert_eq!(formdan(&onceki, &form_alani(&[("pppoe_parola", "")])).parola, "eski-parola");
        assert_eq!(formdan(&onceki, &form_alani(&[])).parola, "eski-parola"); // alan hiç gelmedi
        assert_eq!(formdan(&onceki, &form_alani(&[("pppoe_parola", "yeni")])).parola, "yeni");
        assert_eq!(formdan(&onceki, &form_alani(&[("pppoe_parola", " ")])).parola, " "); // yalnızca boşluk da yazılmış parola
        assert_eq!(formdan(&Pppoe::default(), &form_alani(&[])).parola, ""); // eskisi de yoksa boş (açıkken doğrulama reddeder)
    }

    #[test]
    fn formdan_sayilar_dogrulamayla_yakalanir() {
        let dene = |alan: &str, deger: &str| {
            let p = formdan(&acik(), &form_alani(&[("pppoe_acik", "acik"), ("pppoe_kullanici", "k"), ("pppoe_parola", "p"), (alan, deger)]));
            crate::ag::pppoe_dogrula(&p)
        };
        // sınır değerleri geçerli
        for (alan, deger) in [("pppoe_vlan", "1"), ("pppoe_vlan", "4094"), ("pppoe_vlan", "0"), ("pppoe_oncelik", "0"), ("pppoe_oncelik", "7"), ("pppoe_mtu", "1280"), ("pppoe_mtu", "1492")] {
            assert_eq!(dene(alan, deger), Ok(()), "{alan}={deger}");
        }
        // sayı olmayan, aralık dışı ve türe sığmayan değerler doğrulamada reddedilir (sessizce varsayılana dönmez)
        for (alan, deger) in [
            ("pppoe_vlan", "abc"),
            ("pppoe_vlan", "-1"),
            ("pppoe_vlan", "4095"),
            ("pppoe_vlan", "70000"),
            ("pppoe_vlan", "3.5"),
            ("pppoe_vlan", "3 5"),
            ("pppoe_oncelik", "8"),
            ("pppoe_oncelik", "-1"),
            ("pppoe_oncelik", "300"),
            ("pppoe_oncelik", "x"),
            ("pppoe_mtu", "1279"),
            ("pppoe_mtu", "1493"),
            ("pppoe_mtu", "abc"),
            ("pppoe_mtu", "70000"),
            ("pppoe_mtu", "-5"),
        ] {
            assert!(dene(alan, deger).is_err(), "{alan}={deger:?} geçerli sayılmış");
        }
    }

    #[test]
    fn form_html_alanlar_ve_parola() {
        let html = form_html(&Pppoe { kullanici: "5321234567@turk.net".into(), parola: "gizli-parola-9".into(), ..acik() });
        for ad in ["pppoe_acik", "pppoe_kullanici", "pppoe_parola", "pppoe_vlan", "pppoe_oncelik", "pppoe_mtu", "pppoe_kimlik", "pppoe_ac_adi", "pppoe_servis_adi"] {
            assert_eq!(html.matches(&format!("name=\"{ad}\"")).count(), 1, "{ad}");
            assert!(html.contains(&format!("<label for=\"{ad}\">")) && html.contains(&format!("id=\"{ad}\"")), "{ad}: etiket ve kimlik");
        }
        // parola: türü parola, değeri boş; kayıtlı parola hiçbir yerde yok, yalnızca "aynı kalır" ipucu
        let parola = etiket(&html, "pppoe_parola");
        assert!(parola.contains("type=\"password\"") && parola.contains("value=\"\"") && parola.contains("autocomplete=\"new-password\""), "{parola}");
        assert!(parola.contains("placeholder=\"Kayıtlı parola aynı kalır\"") && !html.contains("gizli-parola-9"));
        assert!(!etiket(&form_html(&Pppoe::default()), "pppoe_parola").contains("placeholder"), "kayıtlı parola yokken ipucu olmaz");
        // kullanıcı adı: örnek yer tutucu + kayıtlı değer
        let kullanici = etiket(&html, "pppoe_kullanici");
        assert!(kullanici.contains("placeholder=\"5321234567@turk.net\"") && kullanici.contains("value=\"5321234567@turk.net\""), "{kullanici}");
        // yalnızca kimlik doğrulama seçeneklerinin üçü
        let kimlik = &html[html.find("<select id=\"pppoe_kimlik\"").unwrap()..];
        assert!(kimlik.contains("<option value=\"otomatik\" selected>Otomatik</option><option value=\"pap\">PAP</option><option value=\"chap\">CHAP</option></select>"), "{kimlik}");
        // ileri ayarlar Gelişmiş'in altında, temel alanlar üstünde
        let (once, sonra) = html.split_once("<summary>Gelişmiş</summary>").unwrap();
        for ad in ["pppoe_acik", "pppoe_kullanici", "pppoe_parola", "pppoe_vlan"] {
            assert!(once.contains(&format!("name=\"{ad}\"")) && !sonra.contains(&format!("name=\"{ad}\"")), "{ad}");
        }
        for ad in ["pppoe_oncelik", "pppoe_mtu", "pppoe_kimlik", "pppoe_ac_adi", "pppoe_servis_adi"] {
            assert!(sonra.contains(&format!("name=\"{ad}\"")) && !once.contains(&format!("name=\"{ad}\"")), "{ad}");
        }
    }

    #[test]
    fn form_html_varsayilan_ve_ileri_ayarlar() {
        // varsayılan ayar: kapalı, VLAN boş (yok), öncelik 0, MTU 1492, otomatik; Gelişmiş kapalı gelir
        let html = form_html(&Pppoe::default());
        assert_eq!(gorunen(&html, "pppoe_acik"), "kapali");
        assert_eq!((gorunen(&html, "pppoe_vlan").as_str(), gorunen(&html, "pppoe_oncelik").as_str(), gorunen(&html, "pppoe_mtu").as_str()), ("", "0", "1492"));
        assert_eq!(gorunen(&html, "pppoe_kimlik"), "otomatik");
        assert!(html.contains("<details class=\"gelismis\"><summary>Gelişmiş</summary>"), "{html}");
        // varsayılandan farklı bir ileri değer varsa Gelişmiş açık gelir (gizli kalıp sessizce uygulanmasın)
        for p in [
            Pppoe { oncelik: 1, ..Pppoe::default() },
            Pppoe { mtu: 1400, ..Pppoe::default() },
            Pppoe { kimlik: "pap".into(), ..Pppoe::default() },
            Pppoe { ac_adi: "AC".into(), ..Pppoe::default() },
            Pppoe { servis_adi: "S".into(), ..Pppoe::default() },
        ] {
            assert!(form_html(&p).contains("<details class=\"gelismis\" open><summary>"));
        }
        // kullanıcı adı, parola ve VLAN ileri ayar değildir
        assert!(form_html(&acik()).contains("<details class=\"gelismis\"><summary>"));
    }

    #[test]
    fn form_html_kacislar() {
        let kotu = "\"><script>alert(1)</script>&'";
        let html = form_html(&Pppoe { kullanici: kotu.into(), ac_adi: kotu.into(), servis_adi: kotu.into(), ..Pppoe::default() });
        assert!(!html.contains("<script") && !html.contains("alert(1)</script>"), "{html}");
        assert_eq!(html.matches("value=\"&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;&amp;&#x27;\"").count(), 3, "{html}");
    }

    #[test]
    fn servis_form_html_kayitli_kullanici_adi_yok() {
        // giriş sayfasındaki Servis formu (parola doğrulanmadan açılır): kayıtlı kullanıcı adı yazılmaz; alan boş ya da yalnızca bu
        // istekte yazılan, hemen altında not; diğer alanlar panel formuyla aynı
        const NOT: &str = "<p class=\"not kucuk\">Boş bırakılırsa kayıtlı kullanıcı adı aynı kalır.</p>";
        let p = Pppoe { kullanici: "abone99@isp".into(), vlan: 35, mtu: 1400, ..acik() };
        let html = servis_form_html(&p, "");
        assert!(!html.contains("abone99@isp") && etiket(&html, "pppoe_kullanici").contains("value=\"\""), "{html}");
        assert!(html.contains(&format!("{}{NOT}", etiket(&html, "pppoe_kullanici"))), "not alanın hemen altında olmalı: {html}");
        assert_eq!(html.replace(NOT, ""), form_html(&Pppoe { kullanici: String::new(), ..p.clone() }));
        // bu istekte yazılan ad geri yazılır (kaçışlı)
        assert!(etiket(&servis_form_html(&p, "yeni\"<x"), "pppoe_kullanici").contains("value=\"yeni&quot;&lt;x\""));
        // panel formu değişmedi: kayıtlı ad dolu, not yok
        let panel = form_html(&p);
        assert!(etiket(&panel, "pppoe_kullanici").contains("value=\"abone99@isp\"") && !panel.contains(NOT), "{panel}");
    }

    #[test]
    fn form_html_formdan_gidis_donus() {
        // form kayıtlı değerlerle dolu gelir ve parola alanı hep boştur: olduğu gibi gönderilince ayar (parola dahil) değişmez.
        // "Değişiklik yok" kararı ve "kapalı" gönderince kullanıcı adının silinmemesi buna dayanır.
        for p in [
            Pppoe::default(),
            acik(),
            Pppoe { acik: false, ..acik() },
            Pppoe { vlan: 0, oncelik: 5, mtu: 1280, kimlik: "chap".into(), ac_adi: "AC.1".into(), servis_adi: "s_2-x".into(), ..acik() },
            Pppoe { vlan: 4094, kimlik: "pap".into(), ..acik() },
        ] {
            let html = form_html(&p);
            let alan = |k: &str| if k == "pppoe_parola" { String::new() } else { gorunen(&html, k) };
            assert!(formdan(&p, &alan) == p, "form kendi değerlerini geri vermedi (VLAN {})", p.vlan);
        }
    }
}
