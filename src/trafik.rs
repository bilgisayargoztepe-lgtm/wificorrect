//! nft sayaç setlerinden (indir/yükle) cihaz başına hız ve günlük toplam.
//! Saf mantık + küçük dosya IO'su; okuyucu/yazıcı döngüsü kaydedici'de.

use crate::ortak;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

pub const DURUM_YOLU: &str = "/run/wificorrect/trafik.json";
/// Hız penceresi: yaşı en az bu kadar olan en yeni örnek referans alınır (~10 sn).
const PENCERE_SN: f64 = 9.0;

/// `nft -j list set` çıktısından IP → bayt. Bozuk girdi → boş harita.
pub fn parse_set(json: &str) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return out };
    let Some(items) = v.get("nftables").and_then(|x| x.as_array()) else { return out };
    for it in items {
        let Some(elems) = it.get("set").and_then(|s| s.get("elem")).and_then(|e| e.as_array()) else { continue };
        for e in elems {
            let e = e.get("elem").unwrap_or(e);
            let ip = e.get("val").and_then(|x| x.as_str());
            let bayt = e.get("counter").and_then(|c| c.get("bytes")).and_then(|b| b.as_u64());
            if let (Some(ip), Some(bayt)) = (ip, bayt) {
                out.insert(ip.to_string(), bayt);
            }
        }
    }
    out
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
pub struct Cihaz {
    pub ip: String,
    pub indir_bps: u64,
    pub yukle_bps: u64,
    pub bugun_bayt: u64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Durum {
    pub zaman: f64,
    /// mac → ölçüm
    pub cihazlar: BTreeMap<String, Cihaz>,
    #[serde(default)]
    pub hiz_hata: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct GunToplam {
    pub gun: String,
    /// mac → bugünkü bayt
    pub bayt: BTreeMap<String, u64>,
}

fn gun_yolu(state_root: &str) -> PathBuf {
    Path::new(state_root).join("trafik_gun.json")
}

pub fn gun_oku(state_root: &str) -> GunToplam {
    std::fs::read_to_string(gun_yolu(state_root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn gun_yaz(state_root: &str, g: &GunToplam) -> std::io::Result<()> {
    let data = serde_json::to_vec(g).map_err(std::io::Error::other)?;
    ortak::write_atomic(&gun_yolu(state_root), &data)
}

pub fn oku(path: &Path) -> Option<Durum> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn yaz(path: &Path, d: &Durum) -> std::io::Result<()> {
    let data = serde_json::to_vec(d).map_err(std::io::Error::other)?;
    ortak::write_atomic(path, &data)
}

/// IP → MAC: oturumlar ((ip, mac); DHCP o IP'yi başka cihaza vermediyse) + kirada IP'si olan izinli cihazlar.
pub fn ip_mac<'a>(
    oturumlar: impl IntoIterator<Item = (&'a String, &'a String)>,
    izinli: &BTreeMap<String, String>,
    kiralar: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (ip, mac) in oturumlar {
        if !ip.is_empty() && kiralar.get(ip).is_none_or(|o| o == mac) {
            out.insert(ip.clone(), mac.clone());
        }
    }
    for (ip, mac) in kiralar {
        if izinli.contains_key(mac) {
            out.entry(ip.clone()).or_insert_with(|| mac.clone());
        }
    }
    out
}

type Gecmis = VecDeque<(f64, u64)>;

#[derive(Default)]
struct IpGecmis {
    mac: String,
    indir: Gecmis,
    yukle: Gecmis,
}

pub struct Olcer {
    gun: GunToplam,
    ipler: BTreeMap<String, IpGecmis>,
}

/// Örneği ekler, pencereyi budar; eklemeden önceki son baytı (varsa) döner.
fn ekle(g: &mut Gecmis, t: f64, bayt: u64) -> Option<u64> {
    let onceki = g.back().map(|x| x.1);
    g.push_back((t, bayt));
    // Yaşı ≥ pencere olan en yeni örnekten eskileri gereksiz.
    while g.len() >= 2 && g[1].0 <= t - PENCERE_SN {
        g.pop_front();
    }
    onceki
}

/// En eski ile en yeni örnek arası ortalama bit/sn; Δt < 1 sn veya aralıkta azalma → 0.
fn hiz(g: &Gecmis) -> u64 {
    let (Some(&(t0, b0)), Some(&(t1, b1))) = (g.front(), g.back()) else { return 0 };
    if t1 - t0 < 1.0 || g.iter().zip(g.iter().skip(1)).any(|(a, b)| b.1 < a.1) {
        return 0;
    }
    ((b1 - b0) as f64 * 8.0 / (t1 - t0)) as u64
}

/// Sayaç azalmışsa (set yeniden doğdu) yeni değerin tamamı fark sayılır; ilk örnekte 0.
fn fark(onceki: Option<u64>, yeni: u64) -> u64 {
    match onceki {
        Some(o) if yeni >= o => yeni - o,
        Some(_) => yeni,
        None => 0,
    }
}

impl Olcer {
    pub fn new(gun: GunToplam) -> Olcer {
        Olcer { gun, ipler: BTreeMap::new() }
    }

    pub fn gun_toplam(&self) -> &GunToplam {
        &self.gun
    }

    /// Bir okuma turu: IP → bayt sayaçlarından cihaz (mac) başına hız ve günlük toplam.
    pub fn ornek(
        &mut self,
        t: f64,
        gun: &str,
        indir: &BTreeMap<String, u64>,
        yukle: &BTreeMap<String, u64>,
        ip_mac: &BTreeMap<String, String>,
    ) -> Durum {
        let gun_degisti = self.gun.gun != gun;
        if gun_degisti {
            self.gun = GunToplam { gun: gun.to_string(), bayt: BTreeMap::new() };
        }
        let mut cihazlar: BTreeMap<String, Cihaz> = BTreeMap::new();
        let ipler: BTreeSet<&String> = indir.keys().chain(yukle.keys()).collect();
        for ip in ipler {
            let Some(mac) = ip_mac.get(ip) else { continue };
            let h = self.ipler.entry(ip.clone()).or_default();
            if h.mac != *mac {
                // IP başka cihaza geçti: eski sahibin baytı yenisine yazılmasın.
                *h = IpGecmis { mac: mac.clone(), ..Default::default() };
            }
            let (mut ind_f, mut yuk_f) = (0, 0);
            if let Some(&b) = indir.get(ip) {
                ind_f = fark(ekle(&mut h.indir, t, b), b);
            }
            if let Some(&b) = yukle.get(ip) {
                yuk_f = fark(ekle(&mut h.yukle, t, b), b);
            }
            let toplam = self.gun.bayt.entry(mac.clone()).or_insert(0);
            if !gun_degisti {
                *toplam += ind_f + yuk_f;
            }
            let c = cihazlar.entry(mac.clone()).or_insert_with(|| Cihaz { ip: ip.clone(), ..Default::default() });
            c.indir_bps += hiz(&h.indir);
            c.yukle_bps += hiz(&h.yukle);
        }
        for (mac, c) in cihazlar.iter_mut() {
            c.bugun_bayt = self.gun.bayt.get(mac).copied().unwrap_or(0);
        }
        // Setlerden düşen (zaman aşımı) IP'nin geçmişi unutulur; iki set birden boşsa okuma hatası sayılır, dokunulmaz.
        if !indir.is_empty() || !yukle.is_empty() {
            self.ipler.retain(|ip, _| indir.contains_key(ip) || yukle.contains_key(ip));
        }
        Durum { zaman: t, cihazlar, hiz_hata: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aa:bb:cc:dd:ee:01";
    const B: &str = "aa:bb:cc:dd:ee:02";
    const IP: &str = "10.50.0.93";

    fn harita(ip: &str, v: u64) -> BTreeMap<String, u64> {
        BTreeMap::from([(ip.to_string(), v)])
    }
    fn ipmac(mac: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(IP.to_string(), mac.to_string())])
    }

    #[test]
    fn parse_set_nft_json() {
        let j = r#"{"nftables": [{"metainfo": {"version": "1.1.3", "json_schema_version": 1}}, {"set": {"family": "inet", "name": "indir", "table": "wfc_hiz", "type": "ipv4_addr", "flags": ["timeout", "dynamic"], "timeout": 86400, "elem": [{"elem": {"val": "10.50.0.93", "expires": 86399, "counter": {"packets": 3, "bytes": 252}}}, {"elem": {"val": "10.50.0.57", "expires": 86399, "counter": {"packets": 1, "bytes": 84}}}]}}]}"#;
        let m = parse_set(j);
        assert_eq!(m, BTreeMap::from([("10.50.0.93".to_string(), 252), ("10.50.0.57".to_string(), 84)]));
        assert!(parse_set("bozuk").is_empty());
        let bos = r#"{"nftables": [{"set": {"name": "indir"}}]}"#;
        assert!(parse_set(bos).is_empty());
    }

    #[test]
    fn hiz_on_saniye_ortalamasi() {
        let mut o = Olcer::new(GunToplam::default());
        let m = ipmac(A);
        let g = "2026-10-10";
        for (t, b) in [(0.0, 0), (5.0, 6_250_000), (10.0, 12_500_000), (15.0, 18_750_000)] {
            let d = o.ornek(t, g, &harita(IP, b), &BTreeMap::new(), &m);
            if t >= 10.0 {
                assert_eq!(d.cihazlar[A].indir_bps, 10_000_000, "t={t}");
            }
        }
    }

    #[test]
    fn bugun_toplam_birikir_gun_degisince_sifir() {
        let mut o = Olcer::new(GunToplam::default());
        let m = ipmac(A);
        o.ornek(0.0, "2026-10-10", &harita(IP, 1000), &harita(IP, 100), &m);
        let d = o.ornek(5.0, "2026-10-10", &harita(IP, 3000), &harita(IP, 600), &m);
        assert_eq!(d.cihazlar[A].bugun_bayt, 2000 + 500);
        let d = o.ornek(10.0, "2026-10-11", &harita(IP, 4000), &harita(IP, 700), &m);
        assert_eq!(d.cihazlar[A].bugun_bayt, 0);
        assert_eq!(o.gun_toplam().gun, "2026-10-11");
        let d = o.ornek(15.0, "2026-10-11", &harita(IP, 4500), &harita(IP, 700), &m);
        assert_eq!(d.cihazlar[A].bugun_bayt, 500);
    }

    #[test]
    fn sayac_azalinca_hiz_sifir_bugun_gerilemez() {
        let mut o = Olcer::new(GunToplam::default());
        let m = ipmac(A);
        let g = "2026-10-10";
        o.ornek(0.0, g, &harita(IP, 0), &BTreeMap::new(), &m);
        let d = o.ornek(5.0, g, &harita(IP, 1_000_000), &BTreeMap::new(), &m);
        assert_eq!(d.cihazlar[A].bugun_bayt, 1_000_000);
        let d = o.ornek(10.0, g, &harita(IP, 400), &BTreeMap::new(), &m);
        assert_eq!(d.cihazlar[A].indir_bps, 0);
        assert_eq!(d.cihazlar[A].bugun_bayt, 1_000_400);
    }

    #[test]
    fn ip_sahibi_degisince_toplam_dogru_maca() {
        let mut o = Olcer::new(GunToplam::default());
        let g = "2026-10-10";
        o.ornek(0.0, g, &harita(IP, 1000), &BTreeMap::new(), &ipmac(A));
        let d = o.ornek(5.0, g, &harita(IP, 3000), &BTreeMap::new(), &ipmac(A));
        assert_eq!(d.cihazlar[A].bugun_bayt, 2000);
        let d = o.ornek(10.0, g, &harita(IP, 3500), &BTreeMap::new(), &ipmac(B));
        assert_eq!(d.cihazlar[B].bugun_bayt, 0);
        assert!(!d.cihazlar.contains_key(A));
        let d = o.ornek(15.0, g, &harita(IP, 3800), &BTreeMap::new(), &ipmac(B));
        assert_eq!(d.cihazlar[B].bugun_bayt, 300);
        assert_eq!(o.gun_toplam().bayt[A], 2000);
    }

    #[test]
    fn ip_mac_te_olmayan_ip_yok_sayilir() {
        let mut o = Olcer::new(GunToplam::default());
        let d = o.ornek(0.0, "2026-10-10", &harita("10.50.0.99", 100), &BTreeMap::new(), &ipmac(A));
        assert!(d.cihazlar.is_empty());
        assert!(o.gun_toplam().bayt.is_empty());
    }

    #[test]
    fn durum_yaz_oku_gidis_donus() {
        let dir = std::env::temp_dir().join(format!("trafik-test-{}", std::process::id()));
        let yol = dir.join("alt").join("trafik.json");
        let mut d = Durum { zaman: 12.5, ..Default::default() };
        d.cihazlar.insert(A.into(), Cihaz { ip: IP.into(), indir_bps: 1, yukle_bps: 2, bugun_bayt: 3 });
        yaz(&yol, &d).unwrap();
        let o = oku(&yol).unwrap();
        assert_eq!(o.zaman, 12.5);
        assert_eq!(o.cihazlar[A], d.cihazlar[A]);
        let root = dir.to_str().unwrap();
        let g = GunToplam { gun: "2026-10-10".into(), bayt: BTreeMap::from([(A.to_string(), 7)]) };
        gun_yaz(root, &g).unwrap();
        assert_eq!(gun_oku(root).bayt[A], 7);
        assert_eq!(gun_oku("/yok/boyle/dizin").gun, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setlerden_dusen_ip_unutulur_okuma_hatasinda_kalir() {
        let mut o = Olcer::new(GunToplam::default());
        let g = "2026-10-10";
        let mut m = ipmac(A);
        m.insert("10.50.0.5".into(), B.into());
        o.ornek(0.0, g, &harita(IP, 1000), &BTreeMap::new(), &m);
        o.ornek(5.0, g, &BTreeMap::new(), &BTreeMap::new(), &m); // iki set de boş: okuma hatası sayılır
        assert!(o.ipler.contains_key(IP));
        o.ornek(10.0, g, &harita("10.50.0.5", 10), &BTreeMap::new(), &m);
        assert!(!o.ipler.contains_key(IP));
        assert!(o.ipler.contains_key("10.50.0.5"));
        assert_eq!(o.gun_toplam().bayt[A], 0); // günlük toplam silinmez
    }

    #[test]
    fn ip_mac_oturum_ve_izinli_kira() {
        let oturum = BTreeMap::from([("10.50.0.23".to_string(), A.to_string()), ("10.50.0.40".to_string(), B.to_string())]);
        let izinli = BTreeMap::from([("aa:bb:cc:dd:ee:99".to_string(), "AP".to_string())]);
        let kira = BTreeMap::from([
            ("10.50.0.40".to_string(), "aa:bb:cc:dd:ee:40".to_string()), // oturumun IP'si başka cihazda
            ("10.50.0.2".to_string(), "aa:bb:cc:dd:ee:99".to_string()),
            ("10.50.0.3".to_string(), "aa:bb:cc:dd:ee:03".to_string()), // izinsiz, oturumsuz
        ]);
        let m = ip_mac(oturum.iter(), &izinli, &kira);
        assert_eq!(
            m,
            BTreeMap::from([("10.50.0.23".to_string(), A.to_string()), ("10.50.0.2".to_string(), "aa:bb:cc:dd:ee:99".to_string())])
        );
    }
}
