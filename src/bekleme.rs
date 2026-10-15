//! Kolay bağlama (spec 2026-10-16): bir müşteriye bağlı olmayan cihaz internete çıkınca 2 dk'da bir merkeze
//! "bağlanmayı bekliyorum" der (`/api/bekliyor`). Müşteri panel.wificorrect.com'a (ya da cihaz.wificorrect.com'a) cihazla
//! aynı internet adresinden girince merkez cihazı ona ayırır; cihaz sonraki bildiriminde tek kullanımlık bağlama
//! belirteci alır ve bugünkü bağlamayı (tünel, yedek, admin parolası) parolasız tamamlar (`/api/bagla`).
//! Kimlik kalıcıdır (`bekleme.json`, 600); gizli bekleme anahtarı (sır) yalnızca cihazda ve TLS içinde merkeze gider,
//! merkez yalnızca özetini tutar: kimliği bilen başka biri cihazın yerine bağlanamaz. Sır ve belirteç loglanmaz.

use crate::ayar::Config;
use crate::merkez::{Giris, GirisHata, Http};
use crate::ortak::{self, Row, Runner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const PATH: &str = "/etc/wificorrect/bekleme.json";
/// son bildirim sonucu ("bekle" | "ulasilamadi" (ulaşılamadı ya da reddetti) | "baglandi"): denetime yalnızca değişince yazılır
pub const DURUM: &str = "/var/lib/wificorrect/bekleme-durum";
pub const ARALIK_SN: f64 = 120.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Kimlik {
    pub kimlik: String,
    pub sir: String,
}

fn kucuk_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn gecerli(k: &Kimlik) -> bool {
    kucuk_hex(&k.kimlik, 32) && kucuk_hex(&k.sir, 64)
}

/// Kayıtlı kimliği okur; yoksa ya da bozuksa yenisini üretir (600). Fabrika dönüşünde de kalır.
pub fn yukle_ya_da_uret(path: &Path) -> Result<Kimlik, String> {
    if let Some(k) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Kimlik>(&b).ok()).filter(gecerli) {
        return Ok(k);
    }
    let k = Kimlik { kimlik: ortak::random_hex(16), sir: ortak::random_hex(32) };
    let veri = serde_json::to_vec_pretty(&k).map_err(|e| e.to_string())?;
    ortak::write_atomic(path, &veri).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(k)
}

#[derive(Debug, PartialEq)]
pub enum Cevap {
    Bekle,
    Baglan(String),
}

/// Bekleme bildirimi. Err: merkeze ulaşılamadı ya da reddetti (mesaj; sır ve belirteç girmez).
pub fn bildir(k: &Kimlik, wan_mac: &str, wg_pub: &str, ssh_pub: &str, http: &Http) -> Result<Cevap, String> {
    let govde = json!({"kimlik": k.kimlik, "sir": k.sir, "surum": env!("CARGO_PKG_VERSION"), "wan_mac": wan_mac,
                       "wg_pub": wg_pub, "ssh_pub": ssh_pub});
    let (d, v) = crate::merkez::cagir(http, "/api/bekliyor", &govde)?;
    if d != 200 {
        return Err(crate::merkez::hata(&v).map_or(format!("HTTP {d}"), |(m, kod)| format!("{m} ({kod})")));
    }
    match v.get("durum").and_then(Value::as_str) {
        Some("bekle") => Ok(Cevap::Bekle),
        Some("baglan") => match v.get("belirtec").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => Ok(Cevap::Baglan(t.to_string())),
            _ => Err("merkezden geçersiz yanıt".into()),
        },
        _ => Err("merkezden geçersiz yanıt".into()),
    }
}

/// Belirteçle parolasız bağlama: cevap `/api/giris`inkiyle aynı, ayrıca müşteri numarası (cihaz numarayı bilmez).
#[allow(clippy::too_many_arguments)]
pub fn bagla(k: &Kimlik, belirtec: &str, wg_pub: &str, ssh_pub: &str, isletme_adi: &str, unvan: &str, http: &Http, now: f64) -> Result<Giris, GirisHata> {
    let govde = json!({"kimlik": k.kimlik, "sir": k.sir, "belirtec": belirtec, "wg_pub": wg_pub, "ssh_pub": ssh_pub,
                       "isletme_adi": isletme_adi, "unvan": unvan, "surum": env!("CARGO_PKG_VERSION")});
    let (d, v) = crate::merkez::cagir(http, "/api/bagla", &govde).map_err(|_| GirisHata::Baglanti)?;
    let numara = v.get("numara").and_then(Value::as_str).filter(|n| crate::merkez::numara_gecerli(n)).unwrap_or_default();
    if d == 200 && numara.is_empty() {
        return Err(GirisHata::Baglanti);
    }
    crate::merkez::giris_cevabi(d, &v, numara, now)
}

pub struct Yollar {
    pub merkez: PathBuf,
    pub bekleme: PathBuf,
    pub durum: PathBuf,
    /// ayar dosyası (bağlanınca tünel + yedek yazılır)
    pub ayar: String,
    /// port görevleri (WAN arayüzü) ve /sys/class/net (WAN MAC)
    pub ag: PathBuf,
    pub sys: PathBuf,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar {
            merkez: crate::merkez::PATH.into(),
            bekleme: PATH.into(),
            durum: DURUM.into(),
            ayar: std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string()),
            ag: crate::ag::Yollar::sistem().ag,
            sys: "/sys/class/net".into(),
        }
    }
}

/// İnternet alan portun MAC'i (küçük harf, iki nokta); okunamazsa boş.
pub fn wan_mac(y: &Yollar) -> String {
    let wan = crate::ag::load(&y.ag).wan;
    std::fs::read_to_string(y.sys.join(wan).join("address")).ok().and_then(|s| ortak::norm_mac(s.trim())).unwrap_or_default()
}

/// Durum değiştiyse yazar ve true döner (denetime yalnızca değişimde).
fn durum_degisti(y: &Yollar, durum: &str) -> bool {
    if std::fs::read_to_string(&y.durum).is_ok_and(|s| s.trim() == durum) {
        return false;
    }
    if let Err(e) = ortak::write_atomic(&y.durum, durum.as_bytes()) {
        eprintln!("bekleme durumu yazılamadı: {e}");
    }
    true
}

/// `ctl bekliyor` (kaydedici 2 dk'da bir başlatır). Bağlıysa hiçbir şey yapmaz. `anahtarlar`: (WireGuard açık anahtarı,
/// yedek SSH açık anahtarı) — yoksa üretilir (bugünkü bağlamanın istediği alanlar). Dönen: kısa sonuç ya da hata.
#[allow(clippy::too_many_arguments)]
pub fn calistir(cfg: &Config, y: &Yollar, hesaplar: &crate::hesap::Hesaplar, anahtarlar: &dyn Fn() -> Result<(String, String), String>, http: &Http, runner: &Runner, now: f64) -> Result<String, String> {
    if crate::merkez::oku(&y.merkez).is_some() {
        return Ok("cihaz bir müşteriye bağlı".into());
    }
    let denetim = |olay: &str, ek: String| ortak::audit(&cfg.main.log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
    let k = yukle_ya_da_uret(&y.bekleme)?;
    let (wg, ssh) = anahtarlar().map_err(|e| format!("cihaz anahtarı üretilemedi: {e}"))?;
    let kisa = &k.kimlik[..8];
    let belirtec = match bildir(&k, &wan_mac(y), &wg, &ssh, http) {
        Ok(Cevap::Bekle) => {
            if durum_degisti(y, "bekle") {
                denetim("MERKEZ_BEKLIYOR", format!("durum=bekle kimlik={kisa}"));
            }
            return Ok("bağlanmayı bekliyor".into());
        }
        Ok(Cevap::Baglan(t)) => t,
        Err(e) => {
            if durum_degisti(y, "ulasilamadi") {
                denetim("MERKEZ_BEKLIYOR", format!("durum=ulasilamadi hata={e}"));
            }
            return Err(e);
        }
    };
    // yerel panelden bağlamayla yarışmasın: kilit altında yeniden bakılır
    let _kilit = crate::merkez::kilit(&y.merkez);
    if crate::merkez::oku(&y.merkez).is_some() {
        return Ok("cihaz bir müşteriye bağlı".into());
    }
    let g = match bagla(&k, &belirtec, &wg, &ssh, &cfg.main.site_name, &cfg.main.unvan, http, now) {
        Ok(g) => g,
        Err(h) => {
            let m = match h {
                GirisHata::Mesaj(m, _) => m,
                GirisHata::Baglanti => "merkeze ulaşılamadı".into(),
            };
            denetim("MERKEZ_BAGLAMA_HATA", format!("hata={m}"));
            return Err(m);
        }
    };
    let (yeni, admin, sms) = match crate::merkez::kur(cfg, &y.ayar, &y.merkez, hesaplar, &g, runner) {
        Ok(x) => x,
        Err(e) => {
            denetim("MERKEZ_BAGLAMA_HATA", format!("hata={e}"));
            return Err(e);
        }
    };
    let denetim = |olay: &str, ek: String| ortak::audit(&yeni.main.log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
    denetim("MERKEZ_BAGLANDI", format!("numara={} tunel={} yol=otomatik kimlik={kisa}", g.merkez.numara, g.tunel_ip));
    match admin {
        Ok(true) => denetim("ADMIN_PAROLA_MERKEZ", String::new()),
        Ok(false) => {}
        Err(e) => denetim("MERKEZ_EK_HATA", e),
    }
    match sms {
        Ok(true) => denetim("SMS_AYARI_MERKEZ", format!("mock={}", u8::from(yeni.sms.mock))),
        Ok(false) => {}
        Err(e) => denetim("MERKEZ_EK_HATA", e),
    }
    durum_degisti(y, "baglandi");
    Ok(format!("müşteri {} ile bağlandı", g.merkez.numara))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn tmp(ad: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wfc-bekleme-{}-{ad}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    type Cagrilar = Arc<Mutex<Vec<(String, Value)>>>;

    fn sahte(yanitlar: Vec<Result<(u16, String), &'static str>>) -> (Box<Http>, Cagrilar) {
        let cagrilar = Arc::new(Mutex::new(vec![]));
        let (c2, y) = (cagrilar.clone(), Arc::new(Mutex::new(yanitlar.into_iter())));
        let f = move |yol: &str, govde: &str| {
            c2.lock().unwrap().push((yol.to_string(), serde_json::from_str(govde).unwrap()));
            match y.lock().unwrap().next().expect("beklenmeyen çağrı") {
                Ok((d, g)) => Ok((d, g)),
                Err(e) => Err(e.to_string()),
            }
        };
        (Box::new(f), cagrilar)
    }

    const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const SSH: &str = "ssh-ed25519 AAAAC3Nza wificorrect-test";

    fn bagla_ok() -> String {
        r#"{"numara":"4511643","tuz":"a1b2c3d4","ozet":"ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495","yineleme":120000,
            "cihaz_anahtari":"k-123","tunel_ip":"10.99.0.12","sunucu_pub":"S","uc_nokta":"vpn.wificorrect.com:51820",
            "yedek_hedefi":"wfc-4511643@10.99.0.1:","lisans":"aktif","lisans_bitis":"2027-10-04","sms":{"etkin_mock":true}}"#.into()
    }

    struct Ortam {
        d: PathBuf,
        y: Yollar,
        cfg: Config,
        hs: crate::hesap::Hesaplar,
    }

    fn ortam(ad: &str) -> Ortam {
        let d = tmp(ad);
        std::fs::create_dir_all(d.join("sys/enp3s0")).unwrap();
        std::fs::write(d.join("sys/enp3s0/address"), "00:0E:C4:CE:A0:9B\n").unwrap();
        std::fs::write(d.join("ag.toml"), "wan = \"enp3s0\"\nlan = [\"enp1s0\"]\n").unwrap();
        let mut cfg = Config::default();
        cfg.main.log_root = d.join("5651").display().to_string();
        cfg.main.site_name = "Bocafe".into();
        let y = Yollar {
            merkez: d.join("merkez.json"),
            bekleme: d.join("bekleme.json"),
            durum: d.join("bekleme-durum"),
            ayar: d.join("ayarlar.toml").display().to_string(),
            ag: d.join("ag.toml"),
            sys: d.join("sys"),
        };
        let hs = crate::hesap::Hesaplar::new(d.join("hesaplar.json"));
        Ortam { d, y, cfg, hs }
    }

    fn denetim(o: &Ortam) -> String {
        let mut s = String::new();
        for e in std::fs::read_dir(o.d.join("5651/gunluk")).into_iter().flatten().flatten() {
            s += &std::fs::read_to_string(e.path().join("denetim.csv")).unwrap_or_default();
        }
        s
    }

    fn anahtarlar() -> Result<(String, String), String> {
        Ok((WG.into(), SSH.into()))
    }

    #[test]
    fn kimlik_kalici_ve_600() {
        let d = tmp("kimlik");
        let p = d.join("bekleme.json");
        let k = yukle_ya_da_uret(&p).unwrap();
        assert!(gecerli(&k) && k.kimlik != k.sir[..32]);
        assert_eq!(yukle_ya_da_uret(&p).unwrap(), k); // kalıcı
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&p, r#"{"kimlik":"XYZ","sir":"1"}"#).unwrap();
        let k2 = yukle_ya_da_uret(&p).unwrap(); // bozuk: yenisi
        assert!(gecerli(&k2) && k2 != k);
        assert!(!gecerli(&Kimlik { kimlik: "A".repeat(32), sir: "a".repeat(64) }));
    }

    #[test]
    fn bildirim_govdesi_ve_cevaplar() {
        let k = Kimlik { kimlik: "1".repeat(32), sir: "a".repeat(64) };
        let (h, c) = sahte(vec![
            Ok((200, r#"{"durum":"bekle"}"#.into())),
            Ok((200, r#"{"durum":"baglan","belirtec":"t-1"}"#.into())),
            Ok((200, r#"{"durum":"baglan"}"#.into())),
            Ok((401, r#"{"hata":"Cihaz tanınmadı.","kod":"taninmadi"}"#.into())),
            Err("ağ"),
        ]);
        assert_eq!(bildir(&k, "00:0e:c4:ce:a0:9b", WG, SSH, &*h), Ok(Cevap::Bekle));
        let g = c.lock().unwrap()[0].clone();
        assert_eq!(g.0, "/api/bekliyor");
        assert_eq!((g.1["kimlik"].as_str(), g.1["sir"].as_str(), g.1["wan_mac"].as_str()), (Some(&*"1".repeat(32)), Some(&*"a".repeat(64)), Some("00:0e:c4:ce:a0:9b")));
        assert_eq!((g.1["wg_pub"].as_str(), g.1["ssh_pub"].as_str(), g.1["surum"].as_str()), (Some(WG), Some(SSH), Some(env!("CARGO_PKG_VERSION"))));
        assert_eq!(bildir(&k, "", WG, SSH, &*h), Ok(Cevap::Baglan("t-1".into())));
        assert!(bildir(&k, "", WG, SSH, &*h).is_err()); // belirteçsiz "bağlan"
        let e = bildir(&k, "", WG, SSH, &*h).unwrap_err();
        assert!(e.contains("taninmadi") && !e.contains(&"a".repeat(64)));
        assert!(bildir(&k, "", WG, SSH, &*h).is_err());
    }

    #[test]
    fn bagla_numarayi_cevaptan_alir() {
        let k = Kimlik { kimlik: "1".repeat(32), sir: "a".repeat(64) };
        let (h, c) = sahte(vec![Ok((200, bagla_ok())), Ok((200, bagla_ok().replace("4511643\",\"tuz", "045\",\"tuz"))), Ok((409, r#"{"hata":"Bu numara başka bir cihazda kullanılıyor.","kod":"baska_cihaz"}"#.into()))]);
        let g = bagla(&k, "t-1", WG, SSH, "Bocafe", "", &*h, 1000.0).unwrap();
        assert_eq!((g.merkez.numara.as_str(), g.tunel_ip.as_str(), g.merkez.cihaz_anahtari.as_str()), ("4511643", "10.99.0.12", "k-123"));
        let b = c.lock().unwrap()[0].clone();
        assert_eq!((b.0.as_str(), b.1["belirtec"].as_str(), b.1["isletme_adi"].as_str()), ("/api/bagla", Some("t-1"), Some("Bocafe")));
        assert!(b.1.get("parola").is_none() && b.1.get("numara").is_none());
        assert!(matches!(bagla(&k, "t-1", WG, SSH, "", "", &*h, 0.0), Err(GirisHata::Baglanti))); // geçersiz numara
        assert!(matches!(bagla(&k, "t-1", WG, SSH, "", "", &*h, 0.0), Err(GirisHata::Mesaj(m, false)) if m.contains("başka")));
    }

    #[test]
    fn bekle_durumu_yalnizca_degisince_denetime() {
        let o = ortam("bekle");
        let r: Box<Runner> = Box::new(|_: &[String]| true);
        let (h, c) = sahte(vec![Ok((200, r#"{"durum":"bekle"}"#.into())), Ok((200, r#"{"durum":"bekle"}"#.into())), Err("ağ"), Err("ağ"), Ok((200, r#"{"durum":"bekle"}"#.into()))]);
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_000.0).is_ok());
        assert_eq!(c.lock().unwrap()[0].1["wan_mac"].as_str(), Some("00:0e:c4:ce:a0:9b")); // WAN portunun MAC'i
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_120.0).is_ok());
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_240.0).is_err());
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_360.0).is_err());
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_480.0).is_ok());
        let d = denetim(&o);
        assert_eq!(d.matches("MERKEZ_BEKLIYOR").count(), 3, "{d}"); // bekle, ulasilamadi, bekle
        let sir = yukle_ya_da_uret(&o.y.bekleme).unwrap().sir;
        assert!(d.contains("durum=ulasilamadi") && !d.contains(&sir), "{d}"); // sır denetime girmez
        // bağlı cihaz hiç bildirmez
        crate::merkez::kaydet(&o.y.merkez, &crate::merkez::Merkez { numara: "4511643".into(), ..Default::default() }).unwrap();
        let (h, c) = sahte(vec![]);
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_600.0).is_ok());
        assert!(c.lock().unwrap().is_empty());
    }

    #[test]
    fn baglan_gelince_parolasiz_baglanir() {
        let o = ortam("baglan");
        let cagrilan = Arc::new(Mutex::new(Vec::<String>::new()));
        let c2 = cagrilan.clone();
        let r: Box<Runner> = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            true
        });
        let (h, c) = sahte(vec![Ok((200, r#"{"durum":"baglan","belirtec":"t-9"}"#.into())), Ok((200, bagla_ok()))]);
        let s = calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_000.0).unwrap();
        assert!(s.contains("4511643"), "{s}");
        let m = crate::merkez::oku(&o.y.merkez).unwrap();
        assert_eq!((m.numara.as_str(), m.cihaz_anahtari.as_str(), m.lisans.as_str()), ("4511643", "k-123", "aktif"));
        let cfg = Config::load(&o.y.ayar).unwrap();
        assert!(cfg.uzak.enabled && cfg.backup.enabled && cfg.uzak.adres == "10.99.0.12" && cfg.backup.target == "wfc-4511643@10.99.0.1:");
        assert_eq!(cfg.main.site_name, "Bocafe"); // diğer ayarlar korunur
        assert_eq!(c.lock().unwrap()[1].1["belirtec"].as_str(), Some("t-9"));
        assert!(cagrilan.lock().unwrap().iter().any(|x| x.contains("ctl uzak-uygula")));
        let d = denetim(&o);
        assert!(d.contains("MERKEZ_BAGLANDI") && d.contains("yol=otomatik") && !d.contains("t-9") && !d.contains("k-123"), "{d}");
        // bağlandıktan sonra bildirim yok
        let (h, c) = sahte(vec![]);
        calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_120.0).unwrap();
        assert!(c.lock().unwrap().is_empty());
    }

    #[test]
    fn baglama_hatasinda_bag_yazilmaz() {
        let o = ortam("hata");
        let r: Box<Runner> = Box::new(|_: &[String]| true);
        let (h, _) = sahte(vec![Ok((200, r#"{"durum":"baglan","belirtec":"t-9"}"#.into())), Ok((401, r#"{"hata":"Bağlama belirteci geçersiz.","kod":"taninmadi"}"#.into()))]);
        assert!(calistir(&o.cfg, &o.y, &o.hs, &anahtarlar, &*h, &*r, 1_790_000_000.0).is_err());
        assert!(crate::merkez::oku(&o.y.merkez).is_none());
        assert!(denetim(&o).contains("MERKEZ_BAGLAMA_HATA"));
        let bozuk = || -> Result<(String, String), String> { Err("wg yok".into()) };
        let (h, c) = sahte(vec![]);
        assert!(calistir(&o.cfg, &o.y, &o.hs, &bozuk, &*h, &*r, 1_790_000_100.0).unwrap_err().contains("wg yok"));
        assert!(c.lock().unwrap().is_empty());
    }
}
