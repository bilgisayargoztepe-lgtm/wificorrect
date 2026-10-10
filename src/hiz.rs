//! Cihaz başına hız sınırı: MAC → sınır kaydı, nft sınıf zinciri metni ve HTB tc komutları.
//! Metin üretimi saf; `uygula` kaydı okuyup tc/nft'yi uzlaştırır (kaydedici 5 sn'de bir, panel `ctl hiz-uygula` ile).

use crate::ayar::Config;
use crate::ortak::{self, Runner};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const HIZLAR: [u32; 5] = [1, 2, 5, 10, 20];
/// Uygulanmış planın parmak izi (yeniden başlatmada silinir; bkz. `uygula`).
pub const UYGULANAN_YOLU: &str = "/run/wificorrect/hiz_uygulanan";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sinir {
    /// Mb/sn
    pub hiz: u32,
    pub ad: String,
    pub zaman: String,
    pub kim: String,
}

/// mac → sınır
pub type Sinirlar = BTreeMap<String, Sinir>;

fn yol(state_root: &str) -> PathBuf {
    Path::new(state_root).join("hiz_sinir.json")
}

/// Dosya yok/bozuk → boş.
pub fn oku(state_root: &str) -> Sinirlar {
    std::fs::read_to_string(yol(state_root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn kaydet(state_root: &str, s: &Sinirlar) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
    ortak::write_atomic(&yol(state_root), &data)
}

pub struct Plan {
    pub nft: String,
    pub tc: Vec<Vec<String>>,
}

fn komut(s: String) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

/// `sinirli`: IP'ye göre sıralı (ip, Mb/sn). Sınıf numarası 0x10 + sıra (nft/tc'de onaltılık).
pub fn plan(lan: &str, wan: &str, sinirli: &[(String, u32)]) -> Plan {
    let mut nft = String::from("flush chain inet wfc_hiz sinif\n");
    for (i, (ip, _)) in sinirli.iter().enumerate() {
        let n = 0x10 + i;
        nft += &format!("add rule inet wfc_hiz sinif oifname \"{lan}\" ip daddr {ip} meta priority set 1:{n:x}\n");
        nft += &format!("add rule inet wfc_hiz sinif oifname \"{wan}\" ip saddr {ip} meta priority set 1:{n:x}\n");
    }
    let mut tc = Vec::new();
    for d in [lan, wan] {
        tc.push(komut(format!("tc qdisc replace dev {d} root handle 1: htb default 1")));
        tc.push(komut(format!("tc class replace dev {d} parent 1: classid 1:1 htb rate 1gbit")));
        tc.push(komut(format!("tc qdisc replace dev {d} parent 1:1 handle 2: fq_codel")));
        for (i, (_, mb)) in sinirli.iter().enumerate() {
            let n = 0x10 + i;
            tc.push(komut(format!("tc class replace dev {d} parent 1: classid 1:{n:x} htb rate {mb}mbit ceil {mb}mbit")));
            tc.push(komut(format!("tc qdisc replace dev {d} parent 1:{n:x} handle {n:x}: fq_codel")));
        }
    }
    Plan { nft, tc }
}

/// nft + tc'nin birleşik metni; uygulanmış durumla karşılaştırmak için.
pub fn parmak_izi(p: &Plan) -> String {
    let mut s = p.nft.clone();
    for c in &p.tc {
        s += &c.join(" ");
        s.push('\n');
    }
    s
}

/// Hata anında: hiç kimse sınıflandırılmasın.
pub const SINIF_BOSALT: &str = "flush chain inet wfc_hiz sinif\n";

/// `nft -f` için geçici dosya (gerçek `nft_f`).
const NFT_GECICI: &str = "/run/wificorrect/hiz.nft.tmp";

/// Gerçek `nft_f`: metni geçici dosyaya yazıp `nft -f` ile tek seferde (atomik) yükler.
pub fn nft_dosya(metin: &str) -> bool {
    let yol = Path::new(NFT_GECICI);
    ortak::write_atomic(yol, metin.as_bytes()).is_ok() && ortak::run(&["nft".into(), "-f".into(), NFT_GECICI.into()])
}

/// Kayıttaki sınırları güncel IP'lere uygular. `ip_of`: mac → ip (IP'si olmayan sınırlı cihaz atlanır).
/// Plan son uygulananla aynıysa hiçbir komut çalışmaz → `Ok(false)`. Hata olursa parmak izi silinir: sonraki tur yeniden dener.
pub fn uygula(
    cfg: &Config,
    wan: &str,
    ip_of: &BTreeMap<String, String>,
    runner: &Runner,
    nft_f: &dyn Fn(&str) -> bool,
    uygulanan: &Path,
) -> Result<bool, String> {
    // panel (`ctl hiz-uygula`) ile kaydedici aynı anda tc komutlarını karıştırmasın
    let kilit_dizini = uygulanan.parent().unwrap_or(Path::new(".")).to_string_lossy().into_owned();
    let _kilit = ortak::state_lock(&kilit_dizini).map_err(|e| format!("Yavaşlatma uygulanamadı: kilit alınamadı: {e}"))?;
    let lan = cfg.main.iface.as_str();
    let mut sinirli: Vec<(String, u32)> =
        oku(&cfg.main.state_root).iter().filter_map(|(mac, s)| ip_of.get(mac).map(|ip| (ip.clone(), s.hiz))).collect();
    sinirli.sort();
    sinirli.dedup_by(|a, b| a.0 == b.0);
    let p = plan(lan, wan, &sinirli);
    let iz = parmak_izi(&p);
    if std::fs::read_to_string(uygulanan).is_ok_and(|eski| eski == iz) {
        return Ok(false);
    }
    let _ = std::fs::remove_file(uygulanan);
    // Yarım kalırsa güvenli tarafa: eski sınıf kuralları eski IP'leri yeni plandaki (başkasının) sınıfına yollayabilir;
    // sınıf zinciri boşaltılır → kimse yavaşlatılmaz (yavaşlatılmamış biri asla yavaşlamasın).
    let acik_birak = |hata: String| {
        nft_f(SINIF_BOSALT);
        Err(format!("Yavaşlatma uygulanamadı: {hata}"))
    };
    for d in [lan, wan] {
        runner(&["tc".into(), "qdisc".into(), "del".into(), "dev".into(), d.into(), "root".into()]); // kök yoksa hata: önemsiz
    }
    for c in &p.tc {
        if !runner(c) {
            return acik_birak(c.join(" "));
        }
    }
    if !nft_f(&p.nft) {
        return acik_birak("nft sınıf kuralları yüklenemedi".into());
    }
    ortak::write_atomic(uygulanan, iz.as_bytes()).map_err(|e| format!("Yavaşlatma uygulandı, kaydı yazılamadı: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc_say(p: &Plan) -> Vec<String> {
        p.tc.iter().map(|c| c.join(" ")).collect()
    }

    #[test]
    fn plan_bos_yalniz_kok_ve_varsayilan() {
        let p = plan("br-hotspot", "enp3s0", &[]);
        assert_eq!(p.nft, "flush chain inet wfc_hiz sinif\n");
        let beklenen: Vec<String> = ["br-hotspot", "enp3s0"]
            .iter()
            .flat_map(|d| {
                [
                    format!("tc qdisc replace dev {d} root handle 1: htb default 1"),
                    format!("tc class replace dev {d} parent 1: classid 1:1 htb rate 1gbit"),
                    format!("tc qdisc replace dev {d} parent 1:1 handle 2: fq_codel"),
                ]
            })
            .collect();
        assert_eq!(tc_say(&p), beklenen);
    }

    #[test]
    fn plan_iki_sinir() {
        let s = vec![("10.50.0.57".to_string(), 5), ("10.50.0.93".to_string(), 2)];
        let p = plan("br-hotspot", "enp3s0", &s);
        let nft: Vec<&str> = p.nft.lines().collect();
        assert_eq!(
            nft,
            [
                "flush chain inet wfc_hiz sinif",
                r#"add rule inet wfc_hiz sinif oifname "br-hotspot" ip daddr 10.50.0.57 meta priority set 1:10"#,
                r#"add rule inet wfc_hiz sinif oifname "enp3s0" ip saddr 10.50.0.57 meta priority set 1:10"#,
                r#"add rule inet wfc_hiz sinif oifname "br-hotspot" ip daddr 10.50.0.93 meta priority set 1:11"#,
                r#"add rule inet wfc_hiz sinif oifname "enp3s0" ip saddr 10.50.0.93 meta priority set 1:11"#,
            ]
        );
        assert!(p.nft.ends_with('\n'));
        let tc = tc_say(&p);
        for d in ["br-hotspot", "enp3s0"] {
            let kendi: Vec<&String> = tc.iter().filter(|c| c.contains(&format!("dev {d} "))).collect();
            assert_eq!(kendi.len(), 7);
            assert_eq!(kendi[3], &format!("tc class replace dev {d} parent 1: classid 1:10 htb rate 5mbit ceil 5mbit"));
            assert_eq!(kendi[4], &format!("tc qdisc replace dev {d} parent 1:10 handle 10: fq_codel"));
            assert_eq!(kendi[5], &format!("tc class replace dev {d} parent 1: classid 1:11 htb rate 2mbit ceil 2mbit"));
            assert_eq!(kendi[6], &format!("tc qdisc replace dev {d} parent 1:11 handle 11: fq_codel"));
        }
        assert!(!tc.iter().any(|c| c.contains(" del ")));
    }

    #[test]
    fn sinif_numarasi_onaltilik() {
        let s: Vec<(String, u32)> = (0..8).map(|i| (format!("10.50.0.{}", 10 + i), 1)).collect();
        let p = plan("a", "b", &s);
        assert!(p.nft.contains("ip daddr 10.50.0.17 meta priority set 1:17\n"));
        let s: Vec<(String, u32)> = (0..11).map(|i| (format!("10.50.0.{}", 10 + i), 1)).collect();
        let p = plan("a", "b", &s);
        assert!(p.nft.contains("ip daddr 10.50.0.20 meta priority set 1:1a\n"));
        assert!(tc_say(&p).contains(&"tc qdisc replace dev a parent 1:1a handle 1a: fq_codel".to_string()));
    }

    #[test]
    fn parmak_izi_planla_degisir() {
        let a = plan("a", "b", &[]);
        let b = plan("a", "b", &[("10.50.0.5".to_string(), 1)]);
        assert_ne!(parmak_izi(&a), parmak_izi(&b));
        assert_eq!(parmak_izi(&a), parmak_izi(&plan("a", "b", &[])));
    }

    #[test]
    fn oku_kaydet_gidis_donus() {
        let dir = std::env::temp_dir().join(format!("hiz-test-{}", std::process::id()));
        let root = dir.to_str().unwrap();
        let mut s = Sinirlar::new();
        s.insert(
            "aa:bb:cc:dd:ee:01".into(),
            Sinir { hiz: 5, ad: "Telefon".into(), zaman: "2026-10-10T12:00:00".into(), kim: "yonetici".into() },
        );
        kaydet(root, &s).unwrap();
        assert_eq!(oku(root), s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bozuk_dosya_bos() {
        let dir = std::env::temp_dir().join(format!("hiz-bozuk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_str().unwrap();
        assert!(oku(root).is_empty());
        std::fs::write(dir.join("hiz_sinir.json"), "{bozuk").unwrap();
        assert!(oku(root).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------- uygula
    use crate::ayar::Config;
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    const M1: &str = "aa:bb:cc:dd:ee:01";
    const M2: &str = "aa:bb:cc:dd:ee:02";

    fn kur(ad: &str) -> (Config, PathBuf) {
        let root = std::env::temp_dir().join(format!("hiz-{ad}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let text = format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n", root.display());
        let cfg = Config::parse(&text).unwrap();
        let mut s = Sinirlar::new();
        for (mac, hiz) in [(M1, 5), (M2, 2)] {
            s.insert(mac.into(), Sinir { hiz, ad: "A B".into(), zaman: "2026-10-10T12:00:00".into(), kim: "y".into() });
        }
        kaydet(&cfg.main.state_root, &s).unwrap();
        (cfg, root)
    }

    /// Çağrıları kaydeden runner; `tc_ok` false iken tc komutları (del dışında) başarısız.
    fn runner(tc_ok: Arc<Mutex<bool>>) -> (Arc<Mutex<Vec<String>>>, Box<ortak::Runner>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let c2 = calls.clone();
        let r = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            c[0] != "tc" || c[2] == "del" || *tc_ok.lock().unwrap()
        });
        (calls, r)
    }

    fn ip(mac: &str, ip: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(mac.to_string(), ip.to_string())])
    }

    #[test]
    fn uygula_degismeyince_komut_yok() {
        let (cfg, root) = kur("ayni");
        let (calls, r) = runner(Arc::new(Mutex::new(true)));
        let nftler = RefCell::new(Vec::<String>::new());
        let nft_f = |t: &str| {
            nftler.borrow_mut().push(t.to_string());
            true
        };
        let yol = root.join("uygulanan");
        // M2 sınırlı ama IP'si yok: atlanır
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol), Ok(true));
        let c = calls.lock().unwrap().clone();
        assert_eq!(c[0], "tc qdisc del dev br-hotspot root");
        assert!(c.contains(&"tc qdisc del dev enp3s0 root".to_string()));
        assert!(c.contains(&"tc class replace dev enp3s0 parent 1: classid 1:10 htb rate 5mbit ceil 5mbit".to_string()));
        assert!(!c.iter().any(|x| x.contains("2mbit")));
        assert_eq!(nftler.borrow().len(), 1);
        assert!(nftler.borrow()[0].contains(r#"oifname "br-hotspot" ip daddr 10.50.0.23 meta priority set 1:10"#));
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol), Ok(false));
        assert_eq!(calls.lock().unwrap().len(), c.len());
        assert_eq!(nftler.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uygula_ip_degisince_yeniden() {
        let (cfg, root) = kur("ip");
        let (_calls, r) = runner(Arc::new(Mutex::new(true)));
        let son = RefCell::new(String::new());
        let nft_f = |t: &str| {
            *son.borrow_mut() = t.to_string();
            true
        };
        let yol = root.join("uygulanan");
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol), Ok(true));
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.24"), &*r, &nft_f, &yol), Ok(true));
        assert!(son.borrow().contains("ip daddr 10.50.0.24 ") && !son.borrow().contains("10.50.0.23"));
        // iki sınırlı cihaz: IP sırasıyla sınıf numarası
        let mut i = ip(M1, "10.50.0.90");
        i.insert(M2.into(), "10.50.0.100".into());
        assert_eq!(uygula(&cfg, "enp3s0", &i, &*r, &nft_f, &yol), Ok(true));
        assert!(son.borrow().contains("ip daddr 10.50.0.100 meta priority set 1:10\n"));
        assert!(son.borrow().contains("ip daddr 10.50.0.90 meta priority set 1:11\n"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uygula_hata_parmak_izi_yazilmaz() {
        let (cfg, root) = kur("hata");
        let tc_ok = Arc::new(Mutex::new(false));
        let (_calls, r) = runner(tc_ok.clone());
        let nft_ok = RefCell::new(true);
        let nft_f = |_: &str| *nft_ok.borrow();
        let yol = root.join("uygulanan");
        let e = uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol).unwrap_err();
        assert!(e.starts_with("Yavaşlatma uygulanamadı: "), "{e}");
        assert!(!yol.exists());
        *tc_ok.lock().unwrap() = true;
        *nft_ok.borrow_mut() = false;
        let e = uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol).unwrap_err();
        assert!(e.starts_with("Yavaşlatma uygulanamadı: "), "{e}");
        *nft_ok.borrow_mut() = true;
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol), Ok(true));
        // önceki başarılı plana geri dönüşte hata olursa iz silinir: sonra aynı plan yeniden denenir
        *tc_ok.lock().unwrap() = false;
        assert!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.24"), &*r, &nft_f, &yol).is_err());
        *tc_ok.lock().unwrap() = true;
        assert_eq!(uygula(&cfg, "enp3s0", &ip(M1, "10.50.0.23"), &*r, &nft_f, &yol), Ok(true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn yarim_kalan_uygulama_sinif_zincirini_bosaltir() {
        // A (M1) kaldırıldı, B (M2) eklendi: yeni planda B 1:10'a düşer; WAN tc'si başarısız olursa eski "A → 1:10" kuralı
        // kalmamalı (A, B'nin 2 Mb'lik sınıfına girerdi)
        let (cfg, root) = kur("yarim");
        let tc_ok = Arc::new(Mutex::new(true));
        let (_calls, r) = runner(tc_ok.clone());
        let nftler = RefCell::new(Vec::<String>::new());
        let nft_ok = RefCell::new(true);
        let nft_f = |t: &str| {
            nftler.borrow_mut().push(t.to_string());
            *nft_ok.borrow()
        };
        let yol = root.join("uygulanan");
        let mut s = oku(&cfg.main.state_root);
        s.remove(M2);
        kaydet(&cfg.main.state_root, &s).unwrap();
        let mut ipler = ip(M1, "10.50.0.23");
        ipler.insert(M2.into(), "10.50.0.24".into());
        assert_eq!(uygula(&cfg, "enp3s0", &ipler, &*r, &nft_f, &yol), Ok(true));
        let mut s = oku(&cfg.main.state_root);
        s.remove(M1);
        s.insert(M2.into(), Sinir { hiz: 2, ad: String::new(), zaman: String::new(), kim: "y".into() });
        kaydet(&cfg.main.state_root, &s).unwrap();
        let r2 = |c: &[String]| !(c[0] == "tc" && c[2] != "del" && c.contains(&"enp3s0".to_string()));
        assert!(uygula(&cfg, "enp3s0", &ipler, &r2, &nft_f, &yol).is_err());
        assert_eq!(nftler.borrow().last().unwrap(), SINIF_BOSALT);
        // nft yüklemesi başarısız olsa da boşaltma denenir
        *nft_ok.borrow_mut() = false;
        assert!(uygula(&cfg, "enp3s0", &ipler, &*r, &nft_f, &yol).is_err());
        let n = nftler.borrow();
        assert!(n[n.len() - 2].contains("ip daddr 10.50.0.24 ") && n[n.len() - 1] == SINIF_BOSALT);
        let _ = std::fs::remove_dir_all(&root);
    }
}
