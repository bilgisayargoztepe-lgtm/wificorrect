//! Cihaz sağlık raporu (spec 2026-10-11): kaydedici 5 dk'da bir `ctl saglik` başlatır; rapor toplanıp merkeze
//! (`POST /api/saglik`) gönderilir. Kişisel veri yok (ad, telefon, MAC gitmez). Başarısızlık sessiz: merkez raporun
//! gelmediğini kendisi fark eder.

use crate::ayar::Config;
use crate::guncelleme::{self, Hedef};
use crate::merkez::{Http, Merkez};
use crate::ortak;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Rapordaki servis adları ve systemd birimleri (merkezdeki `SAGLIK_SERVISLER` ile aynı sıra).
pub const SERVISLER: [(&str, &str); 5] = [
    ("portal", "wificorrect-portal"),
    ("panel", "wificorrect-panel"),
    ("kaydedici", "wificorrect-kaydedici"),
    ("dnsmasq", "dnsmasq"),
    ("guvenlik", "wificorrect-guvenlik"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Girdi {
    pub surum: String,
    pub acik_sn: u64,
    pub servisler: [(&'static str, bool); 5],
    pub disk_yuzde: u64,
    /// en son mühürlenen gün (YYYY-AA-GG) ya da ""
    pub son_muhur: String,
    pub kayit_boslugu_1s: bool,
    /// internet portunda IPv4 adresi var: Ethernet'te DHCP'den; PPPoE açıksa ayrıca PPP arayüzünde (`wfcppp`; PPPoE'de
    /// Ethernet'in kendisinde adres olmaz). Kablo/bağlantı ayrıntısı `pppoe` alanında.
    pub wan: bool,
    /// internet portunun türü: `dhcp` (modemden DHCP) | `pppoe` (ag.toml'da PPPoE açık; bağlı olması gerekmez)
    pub wan_tur: &'static str,
    /// internet portunun durumu (`pppoe::Durum::kod()`, DHCP'li cihazda da gelir): kapali | kablo_yok | dhcp_yok | internet_yok
    /// (≥ 0.4.20) | sunucu_yok | modemden (≥ 0.4.18) | kimlik_hatasi | baglaniyor | bagli
    pub pppoe: &'static str,
    pub sms_sorunu: bool,
    pub bagli: u64,
    pub hiz_hata: bool,
}

/// Merkezin beklediği biçim (api.py `saglik_temizle`): servisler nesne, tamsayılar negatif değil.
pub fn rapor(g: &Girdi) -> Value {
    let servisler: serde_json::Map<String, Value> = g.servisler.iter().map(|(a, d)| (a.to_string(), Value::Bool(*d))).collect();
    json!({
        "surum": g.surum.chars().take(40).collect::<String>(),
        "acik_sn": g.acik_sn,
        "servisler": servisler,
        "disk_yuzde": g.disk_yuzde.min(100),
        "son_muhur": g.son_muhur,
        "kayit_boslugu_1s": g.kayit_boslugu_1s,
        "wan": g.wan,
        "wan_tur": g.wan_tur,
        "pppoe": g.pppoe,
        "sms_sorunu": g.sms_sorunu,
        "bagli": g.bagli,
        "hiz_hata": g.hiz_hata,
    })
}

/// Okunan sistem yolları ve komut çalıştırıcı (testte geçici klasör / sahte).
pub struct Kaynak {
    pub ag: PathBuf,
    pub sys: PathBuf,
    pub trafik: PathBuf,
    pub uptime: PathBuf,
    /// güncelleme durumu klasörü (`/var/lib/wificorrect/guncelleme`)
    pub guncelleme: guncelleme::Yollar,
    /// komut çıktısı (systemctl is-active, ip addr); süre sınırlı; None: çalışmadı/zaman aşımı
    pub komut: Box<dyn Fn(&[&str]) -> Option<String>>,
    /// etkin internet yoklaması (`pppoe` durumu: adres var ama internete çıkılıyor mu; en çok 1,5 sn)
    pub internet: Box<dyn Fn() -> bool>,
}

impl Kaynak {
    pub fn sistem() -> Kaynak {
        Kaynak {
            ag: crate::ag::Yollar::sistem().ag,
            sys: "/sys/class/net".into(),
            trafik: crate::trafik::DURUM_YOLU.into(),
            uptime: "/proc/uptime".into(),
            guncelleme: guncelleme::Yollar::sistem(),
            komut: Box::new(|c: &[&str]| ortak::capture_timeout(c, 5)),
            internet: Box::new(crate::ag::internet_yoklamasi),
        }
    }
}

/// Raporu toplar (gerçek cihazda `Kaynak::sistem()`). systemctl yanıt vermezse (zaman aşımı) rapor yok: "durdu"
/// sanılıp yanlış kritik uyarı açılmasın; merkez raporun gelmediğini kendisi görür.
pub fn topla(cfg: &Config, now: f64, k: &Kaynak) -> Result<Girdi, String> {
    let m = &cfg.main;
    let mut servisler = SERVISLER.map(|(ad, _)| (ad, false));
    for (i, (_, birim)) in SERVISLER.iter().enumerate() {
        let Some(c) = (k.komut)(&["systemctl", "is-active", birim]) else { return Err(format!("systemctl yanıt vermedi ({birim})")) };
        servisler[i].1 = c.trim() == "active";
    }
    let acik_sn = std::fs::read_to_string(&k.uptime)
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map_or(0, |s| s.max(0.0) as u64);
    // en yeni geçerli gün (zincirin son satırı bozuk ya da eski olabilir)
    let son_muhur = crate::muhur::chain_lines(&m.log_root).into_iter().map(|l| l[0].clone()).filter(|g| tarih_mi(g)).max().unwrap_or_default();
    // internet portu (spec 2026-10-23): DHCP'de Ethernet'in adresi; PPPoE açıkken PPP arayüzünün adresi de sayılır (Ethernet'te
    // adres olmaz). Kapalıyken `wfcppp` kalıntısı sayılmaz. PPPoE bağlanamasa da modem DHCP veriyorsa yedek rota çalışır: `wan` true.
    let ag = crate::ag::load(&k.ag);
    let adresli = |arayuz: &str| !arayuz.is_empty() && (k.komut)(&["ip", "-4", "-o", "addr", "show", "dev", arayuz]).unwrap_or_default().contains(" inet ");
    let wan = adresli(&ag.wan) || (ag.pppoe.acik && adresli(crate::pppoe::ARAYUZ));
    let wan_tur = if ag.pppoe.acik { "pppoe" } else { "dhcp" };
    let pppoe = crate::pppoe::durum(&ag, &k.sys, &*k.komut, &*k.internet).kod();
    // bağlı = oturumu süren ve köprüde görülen cihaz (panel Özet ile aynı)
    let agda = std::fs::read(k.sys.join(&m.iface).join("brforward")).map(|b| ortak::kopruye_bagli(&b)).unwrap_or_default();
    let bagli = ortak::load_sessions(&m.state_root).iter().filter(|(mac, s)| !s.ip.is_empty() && s.expires_epoch > now && agda.contains(*mac)).count() as u64;
    Ok(Girdi {
        surum: env!("CARGO_PKG_VERSION").into(),
        acik_sn,
        servisler,
        disk_yuzde: ortak::disk_percent(&m.log_root).unwrap_or(100).min(100), // okunamazsa dolu say: gizlemesin
        son_muhur,
        kayit_boslugu_1s: kayit_boslugu(&m.log_root, now),
        wan,
        wan_tur,
        pppoe,
        // SMS merkezden gider (spec 2026-10-13): sorun = merkez deneme modunda (rapor yalnızca bağlı cihazdan gider)
        sms_sorunu: cfg.sms.mock,
        bagli,
        hiz_hata: crate::trafik::oku(&k.trafik).and_then(|d| d.hiz_hata).is_some_and(|h| !h.is_empty()),
    })
}

fn tarih_mi(g: &str) -> bool {
    let b = g.as_bytes();
    b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Son 1 saatte denetim kaydında `LOG_BOSLUK` var mı. Gece yarısını aşan saat için dünkü dosyaya da bakılır.
/// Zamanlar aynı biçimde (`+03:00`) yazıldığından metin karşılaştırması yeter.
pub fn kayit_boslugu(log_root: &str, now: f64) -> bool {
    let esik = ortak::now_iso(now - 3600.0);
    let simdi = ortak::now_iso(now);
    let mut gunler = vec![ortak::day_of(&esik).to_string(), ortak::day_of(&simdi).to_string()];
    gunler.dedup();
    gunler.iter().any(|gun| {
        std::fs::read_to_string(ortak::day_file(log_root, gun, "denetim.csv")).unwrap_or_default().lines().any(|l| {
            let mut f = l.split(';');
            let (z, olay) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
            olay == "LOG_BOSLUK" && z >= esik.as_str() && z <= simdi.as_str()
        })
    })
}

/// Merkezin rapor cevabı: güncelleme hedefi (varsa ve geçerliyse) ve etkin SMS deneme modu (bu alandan önceki merkez
/// göndermez: None). Deneme modu 5 dk'da bir gelir: Özet ve `sms_sorunu` 06:00 eşitlemesini beklemez.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cevap {
    pub hedef: Option<Hedef>,
    pub sms_mock: Option<bool>,
    /// Hazır engel listelerinin sürümleri (src/liste.rs); bu alandan önceki merkez göndermez: None.
    pub listeler: Option<std::collections::BTreeMap<String, crate::liste::Meta>>,
    /// Müşterinin AP yönetim parolası (spec 2026-10-23: merkezde "Yenile"den 5 dk sonra cihaza inmesi için sağlık cevabında da
    /// gelir; Servis sayfasının parolası). `merkez::ap_parola_oku` kuralına uymayan (8–64 yazdırılabilir ASCII) değer ya da bu
    /// alandan önceki merkez: None. Bir sır: denetime/günlüğe/hata metnine yazılmaz, `{:?}` ile biçimlenmez.
    pub ap_parola: Option<String>,
}

/// Raporu gönderir; merkezin cevabını döner.
pub fn gonder(m: &Merkez, rapor: &Value, http: &Http) -> Result<Cevap, String> {
    let (d, v) = crate::merkez::cagir(http, "/api/saglik", &json!({"cihaz_anahtari": m.cihaz_anahtari, "rapor": rapor}))?;
    if d != 200 {
        return Err(crate::merkez::hata(&v).map_or(format!("HTTP {d}"), |(m, _)| m));
    }
    Ok(Cevap {
        hedef: v.get("guncelleme").and_then(guncelleme::hedef_coz),
        sms_mock: v.get("sms_etkin_mock").and_then(Value::as_bool),
        listeler: v.get("listeler").map(crate::liste::hedef_coz),
        ap_parola: crate::merkez::ap_parola_oku(&v),
    })
}

/// `ctl saglik`: cihaz bağlı değilse bir şey yapılmaz; bağlıysa önce önceki güncellemenin sonucu uzlaştırılır, rapor
/// (paket sürümü ve son güncelleme sonucuyla) gönderilir; başarıda `saglik_basari` dosyasının zamanı yenilenir
/// (denetim betiği mtime'a bakar) ve merkezin cevabı (hedef sürüm, etkin deneme modu) döner.
pub fn calistir(cfg: &Config, merkez_path: &Path, k: &Kaynak, http: &Http, now: f64) -> Result<Cevap, String> {
    let Some(m) = crate::merkez::oku(merkez_path) else { return Ok(Cevap::default()) };
    let kurulu = guncelleme::kurulu(&*k.komut);
    guncelleme::uzlas(&k.guncelleme, &cfg.main.log_root, &kurulu, now);
    let mut r = rapor(&topla(cfg, now, k)?);
    let (paket, g) = guncelleme::rapor_alanlari(&k.guncelleme, &kurulu);
    r["paket"] = Value::String(paket);
    r["guncelleme"] = g;
    let cevap = gonder(&m, &r, http)?;
    let _ = ortak::write_atomic(&k.guncelleme.saglik_basari(), format!("{}\n", now as u64).as_bytes());
    Ok(cevap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ortak::{Row, Session};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn kur() -> (Config, PathBuf, Kaynak) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-saglik-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let text = format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\nleases_file = '{0}/leases'\n", root.display());
        let k = Kaynak {
            ag: root.join("yok-ag.toml"), // varsayılan: WAN enp3s0
            sys: root.join("sys"),
            trafik: root.join("trafik.json"),
            uptime: root.join("uptime"),
            guncelleme: guncelleme::Yollar { kok: root.join("guncelleme") },
            komut: Box::new(|_: &[&str]| Some(String::new())),
            internet: Box::new(|| true), // testler gerçek ağa yoklama yapmaz
        };
        (Config::parse(&text).unwrap(), root, k)
    }

    fn ornek() -> Girdi {
        Girdi {
            surum: "1.2.3".into(),
            acik_sn: 3600,
            servisler: [("portal", true), ("panel", true), ("kaydedici", false), ("dnsmasq", true), ("guvenlik", true)],
            disk_yuzde: 42,
            son_muhur: "2026-10-10".into(),
            kayit_boslugu_1s: false,
            wan: true,
            wan_tur: "dhcp",
            pppoe: "kapali",
            sms_sorunu: false,
            bagli: 3,
            hiz_hata: false,
        }
    }

    #[test]
    fn rapor_alanlari_birebir() {
        let r = rapor(&ornek());
        let o = r.as_object().unwrap();
        let mut anahtar: Vec<&str> = o.keys().map(String::as_str).collect();
        anahtar.sort();
        assert_eq!(anahtar, ["acik_sn", "bagli", "disk_yuzde", "hiz_hata", "kayit_boslugu_1s", "pppoe", "servisler", "sms_sorunu", "son_muhur", "surum", "wan", "wan_tur"]);
        for a in ["surum", "son_muhur", "wan_tur", "pppoe"] {
            assert!(o[a].is_string(), "{a}");
        }
        for a in ["acik_sn", "disk_yuzde", "bagli"] {
            assert!(o[a].is_u64(), "{a}"); // negatif olmayan tamsayı, bool değil
        }
        for a in ["kayit_boslugu_1s", "wan", "sms_sorunu", "hiz_hata"] {
            assert!(o[a].is_boolean(), "{a}");
        }
        let sv = o["servisler"].as_object().unwrap(); // dizi değil nesne
        let mut adlar: Vec<&str> = sv.keys().map(String::as_str).collect();
        adlar.sort();
        assert_eq!(adlar, ["dnsmasq", "guvenlik", "kaydedici", "panel", "portal"]);
        assert!(sv.values().all(Value::is_boolean));
        assert_eq!((sv["kaydedici"].as_bool(), sv["portal"].as_bool()), (Some(false), Some(true)));
        assert_eq!((o["acik_sn"].as_u64(), o["bagli"].as_u64(), o["son_muhur"].as_str()), (Some(3600), Some(3), Some("2026-10-10")));
        assert_eq!((o["wan_tur"].as_str(), o["pppoe"].as_str()), (Some("dhcp"), Some("kapali")));
        // sınırlar: disk en çok 100, sürüm en çok 40 karakter
        let mut g = ornek();
        g.disk_yuzde = 130;
        g.surum = "x".repeat(60);
        let r = rapor(&g);
        assert_eq!((r["disk_yuzde"].as_u64(), r["surum"].as_str().map(str::len)), (Some(100), Some(40)));
        // PPPoE'li cihaz: aynı anahtarlar, farklı değerler
        let mut g = ornek();
        (g.wan_tur, g.pppoe) = ("pppoe", "bagli");
        let r = rapor(&g);
        assert_eq!((r["wan_tur"].as_str(), r["pppoe"].as_str(), r.as_object().unwrap().len()), (Some("pppoe"), Some("bagli"), 12));
    }

    #[test]
    fn kayit_boslugu_son_saat() {
        let (cfg, _root, _) = kur();
        let lr = cfg.main.log_root.clone();
        let now = 1_791_072_000.0 + 12.0 * 3600.0; // gün ortası
        assert!(!kayit_boslugu(&lr, now)); // dosya yok
        ortak::audit(&lr, Row::new("LOG_BOSLUK", &ortak::now_iso(now - 4000.0)).set("ek", "kaynak=ct"));
        ortak::audit(&lr, Row::new("PANEL_OZET", &ortak::now_iso(now - 100.0)));
        assert!(!kayit_boslugu(&lr, now)); // 1 saatten eski boşluk ve başka olay sayılmaz
        ortak::audit(&lr, Row::new("LOG_BOSLUK", &ortak::now_iso(now - 600.0)).set("ek", "kaynak=dns"));
        assert!(kayit_boslugu(&lr, now));
        assert!(!kayit_boslugu(&lr, now + 3600.0));
        // gece yarısını aşan saat: dünkü dosyadaki boşluk da sayılır
        let gece = 1_791_072_000.0 + 21.0 * 3600.0; // TR 00:00 (ertesi gün)
        ortak::audit(&lr, Row::new("LOG_BOSLUK", &ortak::now_iso(gece - 300.0)).set("ek", "kaynak=ct"));
        assert_ne!(ortak::day_of(&ortak::now_iso(gece - 300.0)), ortak::day_of(&ortak::now_iso(gece + 600.0)));
        assert!(kayit_boslugu(&lr, gece + 600.0));
    }

    #[test]
    fn topla_sistemden_okur() {
        let (mut cfg, root, mut k) = kur();
        let now = 1_791_072_000.0;
        let lr = cfg.main.log_root.clone();
        std::fs::write(&k.uptime, "12345.67 9999.00\n").unwrap();
        k.komut = Box::new(|c: &[&str]| Some(match c {
            ["systemctl", "is-active", "dnsmasq"] => "failed\n".into(),
            ["systemctl", "is-active", _] => "active\n".into(),
            ["ip", "-4", "-o", "addr", "show", "dev", "enp3s0"] => "2: enp3s0    inet 192.168.1.10/24 brd 192.168.1.255 scope global dynamic enp3s0\n".into(),
            _ => String::new(),
        }));
        std::fs::create_dir_all(&lr).unwrap();
        std::fs::write(Path::new(&lr).join("zincir.txt"), format!("2026-10-09;{0};{0}\n2026-10-10;{0};{0}\n2026-10-08;{0};{0}\nbozuk-gun;{0};{0}\n", "a".repeat(64))).unwrap();
        // oturumlar: M1 köprüde görülüyor, M2 görülmüyor, M3 süresi dolmuş
        let s = |exp: f64| Session { phone: "5334553132".into(), ad: "A".into(), soyad: "B".into(), ip: "10.50.0.23".into(), session_id: "s".into(),
                                    start: ortak::now_iso(now - 60.0), start_epoch: now - 60.0, expires_epoch: exp };
        let mut ses = ortak::Sessions::new();
        ses.insert("aa:bb:cc:dd:ee:01".into(), s(now + 600.0));
        ses.insert("aa:bb:cc:dd:ee:02".into(), s(now + 600.0));
        ses.insert("aa:bb:cc:dd:ee:03".into(), s(now - 1.0));
        ortak::save_sessions(&cfg.main.state_root, &ses).unwrap();
        let br = k.sys.join(&cfg.main.iface);
        std::fs::create_dir_all(&br).unwrap();
        let mut fdb = vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x01, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        fdb.extend([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x03, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        std::fs::write(br.join("brforward"), fdb).unwrap();
        crate::trafik::yaz(&k.trafik, &crate::trafik::Durum { zaman: now, hiz_hata: Some("tc: hata".into()), ..Default::default() }).unwrap();
        cfg.sms.mock = true;
        let g = topla(&cfg, now, &k).unwrap(); // son_muhur: son satır değil en yeni geçerli gün
        assert_eq!((g.acik_sn, g.son_muhur.as_str(), g.bagli, g.wan, g.hiz_hata, g.sms_sorunu, g.kayit_boslugu_1s), (12345, "2026-10-10", 1, true, true, true, false));
        assert_eq!(g.servisler, [("portal", true), ("panel", true), ("kaydedici", true), ("dnsmasq", false), ("guvenlik", true)]);
        assert_eq!(g.surum, env!("CARGO_PKG_VERSION"));
        assert!(g.disk_yuzde <= 100);
        // hiçbir şey yokken: boş/yanlış değerler, panik yok
        let (cfg2, _r, k2) = kur();
        let g = topla(&cfg2, now, &k2).unwrap();
        assert_eq!((g.acik_sn, g.son_muhur.as_str(), g.bagli, g.wan, g.hiz_hata), (0, "", 0, false, false));
        assert!(g.servisler.iter().all(|(_, d)| !d));
        assert_eq!(g.disk_yuzde, 100); // log_root yok: statvfs okunamadı → dolu say, gizleme
        let _ = root;
    }

    const ETH_ADRES: &str = "2: enp3s0    inet 192.168.1.10/24 brd 192.168.1.255 scope global dynamic enp3s0\n";
    const PPP_ADRES: &str = "7: wfcppp    inet 203.0.113.10 peer 198.51.100.1/32 scope global wfcppp\n";
    const KIMLIK_HATASI: &str = include_str!("../tests/fixtures/pppoe/kimlik_hatasi.log");
    const SUNUCU_YOK: &str = include_str!("../tests/fixtures/pppoe/sunucu_yok.log");

    /// Sahte internet portu: ag.toml'da PPPoE açık/kapalı, internet portunda kablo (taşıyıcı), `ip addr` çıktıları ve pppd birim
    /// günlüğü verilir; merkeze giden rapor (JSON) döner. Etkin yoklama internete çıkıldığını söyler (`wan_raporu_net`: verilen).
    fn wan_raporu(acik: bool, kablo: bool, eth: &'static str, ppp: &'static str, gunluk: &'static str) -> Value {
        wan_raporu_net(acik, kablo, eth, ppp, gunluk, true)
    }

    fn wan_raporu_net(acik: bool, kablo: bool, eth: &'static str, ppp: &'static str, gunluk: &'static str, internet: bool) -> Value {
        let (cfg, _root, mut k) = kur();
        k.internet = Box::new(move || internet);
        k.ag = k.ag.with_file_name("ag.toml");
        let ag = crate::ag::Ag { pppoe: crate::ag::Pppoe { acik, ..Default::default() }, ..Default::default() };
        crate::ag::save(&k.ag, &ag).unwrap();
        std::fs::create_dir_all(k.sys.join("enp3s0")).unwrap();
        std::fs::write(k.sys.join("enp3s0/carrier"), if kablo { "1\n" } else { "0\n" }).unwrap();
        k.komut = Box::new(move |c: &[&str]| {
            Some(match c {
                ["ip", "-4", "-o", "addr", "show", "dev", "enp3s0"] => eth.into(),
                ["ip", "-4", "-o", "addr", "show", "dev", "wfcppp"] => ppp.into(),
                ["journalctl", ..] => gunluk.into(),
                _ => String::new(),
            })
        });
        rapor(&topla(&cfg, 1_791_072_000.0, &k).unwrap())
    }

    /// Merkezin gördüğü üç alan tek satırda: internet var mı, internet türü, durum kodu.
    fn wan_ozeti(acik: bool, kablo: bool, eth: &'static str, ppp: &'static str, gunluk: &'static str) -> String {
        wan_ozeti_net(acik, kablo, eth, ppp, gunluk, true)
    }

    fn wan_ozeti_net(acik: bool, kablo: bool, eth: &'static str, ppp: &'static str, gunluk: &'static str, internet: bool) -> String {
        let r = wan_raporu_net(acik, kablo, eth, ppp, gunluk, internet);
        format!("wan={} tur={} pppoe={}", r["wan"], r["wan_tur"].as_str().unwrap(), r["pppoe"].as_str().unwrap())
    }

    #[test]
    fn wan_ppp_ile() {
        // PPPoE'de internet portunun Ethernet'inde adres yok (DHCP'den alınmaz), PPP arayüzü `wfcppp`'de var: internet var
        let r = wan_raporu(true, true, "", PPP_ADRES, "");
        assert_eq!((r["wan"].as_bool(), r["wan_tur"].as_str(), r["pppoe"].as_str()), (Some(true), Some("pppoe"), Some("bagli")));
        // taşıyıcı bilgisi okunamasa da (sys boş) `wan` yalnızca adrese bakar, DHCP'deki gibi; kablo durumu `pppoe` alanında
        let (cfg, _root, mut k) = kur();
        k.ag = k.ag.with_file_name("ag.toml");
        crate::ag::save(&k.ag, &crate::ag::Ag { pppoe: crate::ag::Pppoe { acik: true, ..Default::default() }, ..Default::default() }).unwrap();
        k.komut = Box::new(|c: &[&str]| Some(if c.contains(&"wfcppp") { PPP_ADRES.to_string() } else { String::new() }));
        let r = rapor(&topla(&cfg, 1_791_072_000.0, &k).unwrap());
        assert_eq!((r["wan"].as_bool(), r["wan_tur"].as_str(), r["pppoe"].as_str()), (Some(true), Some("pppoe"), Some("kablo_yok")));
    }

    #[test]
    fn wan_tur_ve_pppoe_durumlari() {
        // PPPoE kapalı (modemden DHCP): tür dhcp; durum kapali | dhcp_yok | kablo_yok
        assert_eq!(wan_ozeti(false, true, ETH_ADRES, "", ""), "wan=true tur=dhcp pppoe=kapali");
        assert_eq!(wan_ozeti(false, true, "", "", ""), "wan=false tur=dhcp pppoe=dhcp_yok");
        assert_eq!(wan_ozeti(false, false, "", "", ""), "wan=false tur=dhcp pppoe=kablo_yok");
        // kapalıyken `wfcppp` kalıntısı internet sayılmaz (PPPoE birimi kapanırken arayüz bir süre kalabilir)
        assert_eq!(wan_ozeti(false, true, "", PPP_ADRES, ""), "wan=false tur=dhcp pppoe=dhcp_yok");
        // PPPoE açık: tür her zaman pppoe (bağlı olması gerekmez); durum pppd'nin son haline göre
        assert_eq!(wan_ozeti(true, true, "", PPP_ADRES, ""), "wan=true tur=pppoe pppoe=bagli");
        assert_eq!(wan_ozeti(true, true, "", "", ""), "wan=false tur=pppoe pppoe=baglaniyor");
        assert_eq!(wan_ozeti(true, true, "", "", KIMLIK_HATASI), "wan=false tur=pppoe pppoe=kimlik_hatasi");
        assert_eq!(wan_ozeti(true, true, "", "", SUNUCU_YOK), "wan=false tur=pppoe pppoe=sunucu_yok");
        assert_eq!(wan_ozeti(true, false, "", "", ""), "wan=false tur=pppoe pppoe=kablo_yok");
        // PPPoE bağlanamıyor ama modem DHCP veriyor (modemli hat): internet modemden, durum `modemden` (sorun değil; 0.4.18)
        assert_eq!(wan_ozeti(true, true, ETH_ADRES, "", SUNUCU_YOK), "wan=true tur=pppoe pppoe=modemden");
        assert_eq!(wan_ozeti(true, true, ETH_ADRES, "", ""), "wan=true tur=pppoe pppoe=modemden");
        // kimlik reddi DHCP çalışsa da görünür (bir PPPoE sunucusu yanıt verdi, bilgi yanlış)
        assert_eq!(wan_ozeti(true, true, ETH_ADRES, "", KIMLIK_HATASI), "wan=true tur=pppoe pppoe=kimlik_hatasi");
        // adres var ama etkin yoklama internete çıkamıyor (0.4.20): PPPoE kapalıyken `internet_yok`; açıkken (ONT'nin VLAN'sız ağı,
        // 2026-10-24) yeşil `modemden` değil, PPPoE'nin kendi durumu. `wan` yalnızca adrese bakar (eskisi gibi)
        assert_eq!(wan_ozeti_net(false, true, ETH_ADRES, "", "", false), "wan=true tur=dhcp pppoe=internet_yok");
        assert_eq!(wan_ozeti_net(true, true, ETH_ADRES, "", SUNUCU_YOK, false), "wan=true tur=pppoe pppoe=sunucu_yok");
        assert_eq!(wan_ozeti_net(true, true, ETH_ADRES, "", "", false), "wan=true tur=pppoe pppoe=baglaniyor");
        // bağlıyken yoklamaya bakılmaz
        assert_eq!(wan_ozeti_net(true, true, ETH_ADRES, PPP_ADRES, "", false), "wan=true tur=pppoe pppoe=bagli");
    }

    #[test]
    fn systemctl_zaman_asiminda_rapor_gitmez() {
        let (cfg, root, mut k) = kur();
        k.komut = Box::new(|c: &[&str]| match c {
            ["systemctl", "is-active", "wificorrect-panel"] => None, // zaman aşımı
            ["systemctl", "is-active", _] => Some("active\n".into()),
            _ => Some(String::new()),
        });
        assert!(topla(&cfg, 1000.0, &k).is_err());
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        let (h, c) = sahte(Ok((200, r#"{"tamam":true}"#)));
        let e = calistir(&cfg, &p, &k, &*h, 1000.0).unwrap_err();
        assert!(e.contains("systemctl") && !e.contains('\n'), "{e}"); // ctl tek satır yazar, çıkış 0
        assert!(c.lock().unwrap().is_empty());
    }

    fn sahte(yanit: Result<(u16, &'static str), &'static str>) -> (Box<Http>, Arc<Mutex<Vec<(String, Value)>>>) {
        let cagrilar = Arc::new(Mutex::new(vec![]));
        let c2 = cagrilar.clone();
        let f = move |yol: &str, govde: &str| {
            c2.lock().unwrap().push((yol.to_string(), serde_json::from_str(govde).unwrap()));
            yanit.map(|(d, g)| (d, g.to_string())).map_err(String::from)
        };
        (Box::new(f), cagrilar)
    }

    #[test]
    fn bagli_degilken_gonderilmez() {
        let (cfg, root, k) = kur();
        let p = root.join("merkez.json");
        let (h, c) = sahte(Ok((200, r#"{"tamam":true}"#)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(Cevap::default()));
        assert!(c.lock().unwrap().is_empty());
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "gizli-anahtar".into(), ..Default::default() }).unwrap();
        assert!(!k.guncelleme.saglik_basari().exists());
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(Cevap::default())); // eski merkez: deneme modu bilgisi yok
        assert!(k.guncelleme.saglik_basari().exists()); // başarıda dokunulur
        let (yol, govde) = c.lock().unwrap()[0].clone();
        assert_eq!((yol.as_str(), govde["cihaz_anahtari"].as_str()), ("/api/saglik", Some("gizli-anahtar")));
        assert!(govde["rapor"]["servisler"].is_object() && govde.as_object().unwrap().len() == 2);
        // gönderilemezse hata (ctl tek satır yazar, çıkış 0)
        let (h, _) = sahte(Err("bağlanılamadı"));
        let _ = std::fs::remove_file(k.guncelleme.saglik_basari());
        assert!(calistir(&cfg, &p, &k, &*h, 1000.0).is_err());
        assert!(!k.guncelleme.saglik_basari().exists()); // hatada dokunulmaz
        let (h, _) = sahte(Ok((401, r#"{"hata":"Cihaz tanınmadı.","kod":"taninmadi"}"#)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Err("Cihaz tanınmadı.".into()));
    }

    #[test]
    fn rapora_paket_ve_guncelleme_eklenir() {
        let (cfg, root, mut k) = kur();
        k.komut = Box::new(|c: &[&str]| Some(match c {
            ["dpkg-query", ..] => "0.4.0
".into(),
            _ => String::new(),
        }));
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        let (h, c) = sahte(Ok((200, r#"{"tamam":true}"#)));
        calistir(&cfg, &p, &k, &*h, 1000.0).unwrap();
        let r = c.lock().unwrap()[0].1["rapor"].clone();
        assert_eq!((r["paket"].as_str(), r["guncelleme"].is_null()), (Some("0.4.0"), true));
        guncelleme::durum_yaz(&k.guncelleme, &guncelleme::Durum { basarisiz: vec![], son: Some(guncelleme::Sonuc { surum: "0.4.0".into(), sonuc: "tamam".into(), zaman: 900, neden: String::new() }), ..Default::default() }).unwrap();
        calistir(&cfg, &p, &k, &*h, 1000.0).unwrap();
        let r = c.lock().unwrap()[1].1["rapor"].clone();
        assert_eq!(r["guncelleme"], json!({"surum": "0.4.0", "sonuc": "tamam", "zaman": 900}));
    }

    #[test]
    fn cevaptaki_hedef_doner() {
        let (cfg, root, k) = kur();
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        let govde: &'static str = Box::leak(format!(r#"{{"tamam":true,"guncelleme":{{"surum":"0.4.1","sha256":"{}","boyut":12345,"hemen":false}}}}"#, "b".repeat(64)).into_boxed_str());
        let (h, _) = sahte(Ok((200, govde)));
        let hedef = calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().hedef.unwrap();
        assert_eq!((hedef.surum.as_str(), hedef.boyut, hedef.hemen), ("0.4.1", 12345, false));
        // bozuk hedef yok sayılır
        let (h, _) = sahte(Ok((200, r#"{"tamam":true,"guncelleme":{"surum":"1.2","sha256":"x","boyut":0}}"#)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0).map(|c| c.hedef), Ok(None));
    }

    #[test]
    fn cevaptaki_etkin_deneme_modu_doner() {
        let (cfg, root, k) = kur();
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        for (govde, beklenen) in [(r#"{"tamam":true,"sms_etkin_mock":true}"#, Some(true)), (r#"{"tamam":true,"sms_etkin_mock":false}"#, Some(false)),
                                  (r#"{"tamam":true,"sms_etkin_mock":"x"}"#, None), (r#"{"tamam":true}"#, None)] {
            let (h, _) = sahte(Ok((200, govde)));
            assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().sms_mock, beklenen, "{govde}");
        }
    }

    #[test]
    fn cevapta_ap_parola() {
        let (cfg, root, k) = kur();
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        // gövdeyi json! kurar (kaçışlar doğru); sahte() 'static metin ister
        let ap = |parola: Value| {
            let govde: &'static str = Box::leak(json!({"tamam": true, "ap_parola": parola}).to_string().into_boxed_str());
            let (h, _) = sahte(Ok((200, govde)));
            calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().ap_parola
        };
        let sin = |s: &str| Some(s.to_string());
        assert_eq!(ap(json!("Kp7mQx2Rtz4W")), sin("Kp7mQx2Rtz4W"));
        // sınırlar: 8–64 bayt
        assert_eq!(ap(json!("Kp7mQx2R")), sin("Kp7mQx2R"));
        assert_eq!(ap(json!("A".repeat(64))), Some("A".repeat(64)));
        assert_eq!(ap(json!("Kp7mQx2")), None); // 7 karakter
        assert_eq!(ap(json!("A".repeat(65))), None);
        assert_eq!(ap(json!("")), None);
        // yalnızca yazdırılabilir ASCII, boşluksuz
        for kotu in ["Kp7m Qx2Rtz4W", "Kp7mQx2\tRtz4W", "Kp7mQx2\nRtz4W", "Şifre1234567", "Kp7mQx2Rtz4W\u{0}"] {
            assert_eq!(ap(json!(kotu)), None, "{kotu:?}");
        }
        // dize olmayan değerler ve eski merkez (alan yok)
        for kotu in [json!(12345678901u64), json!(null), json!(true), json!(["Kp7mQx2Rtz4W"])] {
            assert_eq!(ap(kotu.clone()), None, "{kotu}");
        }
        let (h, _) = sahte(Ok((200, r#"{"tamam":true}"#)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(Cevap::default()));
        // öteki cevap alanlarını bozmaz; cevap tek başına parolayı dosyaya yazmaz (uygulamak ctl'nin işi)
        let govde: &'static str = r#"{"tamam":true,"ap_parola":"Kp7mQx2Rtz4W","sms_etkin_mock":true}"#;
        let (h, _) = sahte(Ok((200, govde)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(Cevap { sms_mock: Some(true), ap_parola: sin("Kp7mQx2Rtz4W"), ..Default::default() }));
        assert_eq!(crate::merkez::oku(&p).unwrap().ap_parola, "");
    }

    #[test]
    fn cevaptaki_liste_surumleri_doner() {
        let (cfg, root, k) = kur();
        let p = root.join("merkez.json");
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "a".into(), ..Default::default() }).unwrap();
        let govde: &'static str = Box::leak(
            format!(r#"{{"tamam":true,"listeler":{{"kumar":{{"sha256":"{}","sayi":12,"zaman":"2026-10-14T03:00:00+03:00"}},"vpn":{{}}}}}}"#, "c".repeat(64)).into_boxed_str(),
        );
        let (h, _) = sahte(Ok((200, govde)));
        let l = calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().listeler.unwrap();
        assert_eq!(l.keys().collect::<Vec<_>>(), vec!["kumar"]);
        assert_eq!(l["kumar"].sayi, 12);
        let (h, _) = sahte(Ok((200, r#"{"tamam":true}"#))); // eski merkez
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().listeler, None);
    }

    #[test]
    fn istek_10_sn_zaman_asimli() {
        let a = crate::merkez::curl_args_sure("https://api.wificorrect.com/api/saglik", 10);
        assert!(a.windows(2).any(|w| w[0] == "--max-time" && w[1] == "10") && a.windows(2).any(|w| w[0] == "--connect-timeout" && w[1] == "10"));
    }
}
