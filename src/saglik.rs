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
    pub wan: bool,
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
    let wan_if = crate::ag::load(&k.ag).wan;
    let wan = !wan_if.is_empty() && (k.komut)(&["ip", "-4", "-o", "addr", "show", "dev", &wan_if]).unwrap_or_default().contains(" inet ");
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
        sms_sorunu: cfg.sms.mock || !cfg.sms_missing().is_empty(),
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

/// Raporu gönderir; merkezin cevabındaki güncelleme hedefi (varsa ve geçerliyse) döner.
pub fn gonder(m: &Merkez, rapor: &Value, http: &Http) -> Result<Option<Hedef>, String> {
    let (d, v) = crate::merkez::cagir(http, "/api/saglik", &json!({"cihaz_anahtari": m.cihaz_anahtari, "rapor": rapor}))?;
    if d != 200 {
        return Err(crate::merkez::hata(&v).map_or(format!("HTTP {d}"), |(m, _)| m));
    }
    Ok(v.get("guncelleme").and_then(guncelleme::hedef_coz))
}

/// `ctl saglik`: cihaz bağlı değilse bir şey yapılmaz; bağlıysa önce önceki güncellemenin sonucu uzlaştırılır, rapor
/// (paket sürümü ve son güncelleme sonucuyla) gönderilir; başarıda `saglik_basari` dosyasının zamanı yenilenir
/// (denetim betiği mtime'a bakar) ve merkezin hedef sürümü döner.
pub fn calistir(cfg: &Config, merkez_path: &Path, k: &Kaynak, http: &Http, now: f64) -> Result<Option<Hedef>, String> {
    let Some(m) = crate::merkez::oku(merkez_path) else { return Ok(None) };
    let kurulu = guncelleme::kurulu(&*k.komut);
    guncelleme::uzlas(&k.guncelleme, &cfg.main.log_root, &kurulu, now);
    let mut r = rapor(&topla(cfg, now, k)?);
    let (paket, g) = guncelleme::rapor_alanlari(&k.guncelleme, &kurulu);
    r["paket"] = Value::String(paket);
    r["guncelleme"] = g;
    let hedef = gonder(&m, &r, http)?;
    let _ = ortak::write_atomic(&k.guncelleme.saglik_basari(), format!("{}\n", now as u64).as_bytes());
    Ok(hedef)
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
        assert_eq!(anahtar, ["acik_sn", "bagli", "disk_yuzde", "hiz_hata", "kayit_boslugu_1s", "servisler", "sms_sorunu", "son_muhur", "surum", "wan"]);
        for a in ["surum", "son_muhur"] {
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
        // sınırlar: disk en çok 100, sürüm en çok 40 karakter
        let mut g = ornek();
        g.disk_yuzde = 130;
        g.surum = "x".repeat(60);
        let r = rapor(&g);
        assert_eq!((r["disk_yuzde"].as_u64(), r["surum"].as_str().map(str::len)), (Some(100), Some(40)));
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
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(None));
        assert!(c.lock().unwrap().is_empty());
        crate::merkez::kaydet(&p, &Merkez { numara: "4511643".into(), cihaz_anahtari: "gizli-anahtar".into(), ..Default::default() }).unwrap();
        assert!(!k.guncelleme.saglik_basari().exists());
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(None));
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
        let hedef = calistir(&cfg, &p, &k, &*h, 1000.0).unwrap().unwrap();
        assert_eq!((hedef.surum.as_str(), hedef.boyut, hedef.hemen), ("0.4.1", 12345, false));
        // bozuk hedef yok sayılır
        let (h, _) = sahte(Ok((200, r#"{"tamam":true,"guncelleme":{"surum":"1.2","sha256":"x","boyut":0}}"#)));
        assert_eq!(calistir(&cfg, &p, &k, &*h, 1000.0), Ok(None));
    }

    #[test]
    fn istek_10_sn_zaman_asimli() {
        let a = crate::merkez::curl_args_sure("https://api.wificorrect.com/api/saglik", 10);
        assert!(a.windows(2).any(|w| w[0] == "--max-time" && w[1] == "10") && a.windows(2).any(|w| w[0] == "--connect-timeout" && w[1] == "10"));
    }
}
