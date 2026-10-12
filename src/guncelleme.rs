//! Merkezden uzaktan güncelleme (spec 2026-10-12): sağlık cevabındaki hedef sürüm, cihazdaki karar mantığı ve
//! güncelleme sonucunun uzlaştırılması. İndirme, kurma ve geri alma `ctl guncelle` ile denetim betiğindedir.

use crate::ayar::Config;
use crate::merkez::{Indir, Merkez};
use crate::ortak::{self, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Yerel saat dilimi: Türkiye UTC+3, yaz saati yok (güncelleme penceresi TR saatiyle).
const TR: f64 = 3.0 * 3600.0;
/// Başarısız (hata) bir sürüm için `hemen` olmayan yeni deneme bekleme süresi.
const HATA_BEKLE_SN: f64 = 43200.0;

pub struct Yollar {
    pub kok: PathBuf,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar { kok: "/var/lib/wificorrect/guncelleme".into() }
    }
    pub fn durum(&self) -> PathBuf {
        self.kok.join("durum.json")
    }
    pub fn hedef(&self) -> PathBuf {
        self.kok.join("hedef.json")
    }
    pub fn paketler(&self) -> PathBuf {
        self.kok.join("paketler")
    }
    pub fn bekleyen(&self) -> PathBuf {
        self.kok.join("bekleyen") // denetim betiği bu adla bakar
    }
    pub fn sonuc(&self) -> PathBuf {
        self.kok.join("sonuc")
    }
    pub fn onceki_deb(&self) -> PathBuf {
        self.kok.join("onceki.deb")
    }
    pub fn betik(&self) -> PathBuf {
        self.kok.join("denetle.sh")
    }
    pub fn saglik_basari(&self) -> PathBuf {
        self.kok.join("saglik_basari")
    }
    pub fn kilit(&self) -> PathBuf {
        self.kok.join("kilit")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hedef {
    pub surum: String,
    pub sha256: String,
    pub boyut: u64,
    #[serde(default)]
    pub hemen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sonuc {
    pub surum: String,
    /// tamam | geri_alindi | hata
    pub sonuc: String,
    pub zaman: u64,
    #[serde(default)]
    pub neden: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Durum {
    #[serde(default)]
    pub basarisiz: Vec<String>,
    #[serde(default)]
    pub son: Option<Sonuc>,
    /// son güncellemeden önce kurulu sürüm (paketi saklanır)
    #[serde(default)]
    pub onceki: String,
}

/// `X.Y.Z` (yalnızca rakamlar).
pub fn surum_gecerli(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit()))
}

/// Merkez cevabındaki `guncelleme` nesnesi; geçersiz/eksik → None.
pub fn hedef_coz(v: &Value) -> Option<Hedef> {
    let surum = v.get("surum")?.as_str()?;
    let sha = v.get("sha256")?.as_str()?;
    let boyut = v.get("boyut")?.as_u64()?;
    if !surum_gecerli(surum) || sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) || boyut == 0 {
        return None;
    }
    let hemen = v.get("hemen").and_then(Value::as_bool).unwrap_or(false);
    Some(Hedef { surum: surum.into(), sha256: sha.into(), boyut, hemen })
}

/// Yok ya da bozuk dosya → varsayılan.
pub fn durum_oku(y: &Yollar) -> Durum {
    std::fs::read(y.durum()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn durum_yaz(y: &Yollar, d: &Durum) -> std::io::Result<()> {
    ortak::write_atomic(&y.durum(), &serde_json::to_vec_pretty(d).unwrap_or_default())
}

/// Kurulu paket sürümü (`dpkg-query`); hata ya da boş çıktı → "".
pub fn kurulu(komut: &dyn Fn(&[&str]) -> Option<String>) -> String {
    komut(&["dpkg-query", "-W", "-f=${Version}", "wificorrect"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

#[derive(Debug, PartialEq)]
pub enum Karar {
    Kur,
    Bekle(&'static str),
}

/// TR saati 03:00 ≤ t < 06:00.
pub fn pencere(now: f64) -> bool {
    let sn = (now + TR).rem_euclid(86400.0);
    (3.0 * 3600.0..6.0 * 3600.0).contains(&sn)
}

pub fn karar(kurulu: &str, h: &Hedef, d: &Durum, bekleyen_var: bool, now: f64) -> Karar {
    if kurulu.is_empty() {
        return Karar::Bekle("paketsiz");
    }
    if kurulu == h.surum {
        return Karar::Bekle("guncel");
    }
    if bekleyen_var {
        return Karar::Bekle("denetim");
    }
    if !h.hemen {
        if d.basarisiz.contains(&h.surum) {
            return Karar::Bekle("basarisiz");
        }
        if d.son.as_ref().is_some_and(|s| s.sonuc == "hata" && s.surum == h.surum && now - (s.zaman as f64) < HATA_BEKLE_SN) {
            return Karar::Bekle("hata_bekle");
        }
        if !pencere(now) {
            return Karar::Bekle("pencere");
        }
    }
    Karar::Kur
}

/// Güncelleme kilidi (`kilit` dosyası, flock): `ctl guncelle` bütün işi boyunca tutar; uzlaşma ve hedef yazma
/// kilit tutuluyorsa bir şey yapmaz. Dosya kapanınca (süreç bitince de) bırakılır.
pub struct Kilit {
    _dosya: std::fs::File,
}

/// Kilit beklemeden alınır; başkası tutuyorsa (ya da klasör açılamazsa) None.
pub fn kilit_al(y: &Yollar) -> Option<Kilit> {
    use std::os::unix::io::AsRawFd;
    ortak::mkdirs(&y.kok).ok()?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(y.kilit()).ok()?;
    // SAFETY: geçerli dosya tanıtıcısı; kilit dosya kapanınca bırakılır.
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(Kilit { _dosya: f })
}

/// Denetim satırının `ek` alanı: `surum=… onceki=… neden=…` (boş olanlar yazılmaz).
fn ek(surum: &str, onceki: &str, neden: &str) -> String {
    [("surum", surum), ("onceki", onceki), ("neden", neden)].iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
}

fn denetim_yaz(log_root: &str, olay: &str, ek: String, now: f64) {
    ortak::audit(log_root, Row::new(olay, &ortak::now_iso(now)).set("ek", ek));
}

/// Denetim betiğinin bıraktığı sonucu işler (`bekleyen` + `sonuc`), sonra eski paket dosyalarını temizler.
/// Güncelleme sürüyorsa (kilit tutuluyor) bir şey yapılmaz.
pub fn uzlas(y: &Yollar, log_root: &str, kurulu: &str, now: f64) {
    if let Some(_k) = kilit_al(y) {
        uzlas_kilitli(y, log_root, kurulu, now);
    }
}

/// `uzlas`, kilit çağıranda.
fn uzlas_kilitli(y: &Yollar, log_root: &str, kurulu: &str, now: f64) {
    if y.bekleyen().exists() && y.sonuc().exists() {
        let b: Value = std::fs::read(y.bekleyen()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
        let yeni = b.get("yeni").and_then(Value::as_str).unwrap_or("").to_string();
        let onceki = b.get("onceki").and_then(Value::as_str).unwrap_or("").to_string();
        let sonuc = std::fs::read_to_string(y.sonuc()).unwrap_or_default();
        let sonuc = sonuc.trim();
        let tamam = sonuc == "tamam";
        if !yeni.is_empty() && tamam && !kurulu.is_empty() && kurulu != yeni {
            // yeni sürüm hiç kurulamadı (apt ve hemen geri kurulum başarısız), eski sürüm sağlıklı: denenmiş sayılmaz,
            // başarısız listesine girmez; 12 saat sonra (ya da "hemen" ile) yeniden denenir
            denetim_yaz(log_root, "GUNCELLEME_HATA", ek(&yeni, &onceki, "kurulmadi"), now);
            let mut d = durum_oku(y);
            d.son = Some(Sonuc { surum: yeni.clone(), sonuc: "hata".into(), zaman: now.max(0.0) as u64, neden: "kurulmadi".into() });
            let _ = durum_yaz(y, &d);
        } else if !yeni.is_empty() && (tamam || sonuc == "geri") {
            let mut d = durum_oku(y);
            let zaman = now.max(0.0) as u64;
            if tamam {
                denetim_yaz(log_root, "GUNCELLEME_TAMAM", ek(&yeni, &onceki, ""), now);
                d.basarisiz.retain(|s| *s != yeni);
            } else {
                denetim_yaz(log_root, "GUNCELLEME_GERI_ALINDI", ek(&yeni, &onceki, ""), now);
                if !d.basarisiz.contains(&yeni) {
                    d.basarisiz.push(yeni.clone());
                }
            }
            d.son = Some(Sonuc { surum: yeni, sonuc: if tamam { "tamam" } else { "geri_alindi" }.into(), zaman, neden: String::new() });
            let _ = durum_yaz(y, &d);
        }
        let _ = std::fs::remove_file(y.bekleyen());
        let _ = std::fs::remove_file(y.sonuc());
    }
    paket_temizle(y, &[kurulu, &durum_oku(y).onceki]);
}

/// `paketler/`: korunan sürümlerin (kurulu ve önceki) paketleri her zaman kalır, diğerlerinden yalnızca en yenisi;
/// yarım indirmeler (`.deb.tmp`) silinir (kilit altında çağrılır, süren indirme yoktur).
fn paket_temizle(y: &Yollar, korunan: &[&str]) {
    let Ok(rd) = std::fs::read_dir(y.paketler()) else { return };
    let korunan: Vec<String> = korunan.iter().filter(|s| !s.is_empty()).map(|s| format!("{s}.deb")).collect();
    let mut digerleri: Vec<(std::time::SystemTime, PathBuf)> = vec![];
    for e in rd.filter_map(|e| e.ok()).filter(|e| e.path().is_file()) {
        let ad = e.file_name().to_string_lossy().into_owned();
        if korunan.contains(&ad) {
            continue;
        }
        if !ad.ends_with(".deb") {
            let _ = std::fs::remove_file(e.path());
            continue;
        }
        digerleri.push((e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()));
    }
    digerleri.sort_by(|a, b| b.cmp(a));
    for (_, p) in digerleri.into_iter().skip(1) {
        let _ = std::fs::remove_file(p);
    }
}

/// Rapora eklenecek alanlar: kurulu paket sürümü (merkezin biçimine uymuyorsa "") ve son sonuç (yoksa null).
pub fn rapor_alanlari(y: &Yollar, kurulu: &str) -> (String, Value) {
    let paket = if surum_gecerli(kurulu) { kurulu.to_string() } else { String::new() };
    let g = durum_oku(y).son.map_or(Value::Null, |s| json!({"surum": s.surum, "sonuc": s.sonuc, "zaman": s.zaman}));
    (paket, g)
}

/// Merkezin hedefini işler: karar `Kur` ise `hedef.json` yazılır ve `baslat` çağrılır. Güncelleme sürüyorsa
/// (kilit tutuluyor) hedef ezilmez: `Bekle("suruyor")`. Kilit `baslat`tan önce bırakılır (başlatılan iş alabilsin).
pub fn hedefi_isle(y: &Yollar, h: &Hedef, kurulu: &str, now: f64, baslat: &dyn Fn()) -> Karar {
    let yazildi = {
        let Some(_k) = kilit_al(y) else { return Karar::Bekle("suruyor") };
        let k = karar(kurulu, h, &durum_oku(y), y.bekleyen().exists(), now);
        if k != Karar::Kur {
            return k;
        }
        ortak::write_atomic(&y.hedef(), &serde_json::to_vec_pretty(h).unwrap_or_default()).is_ok()
    };
    if yazildi {
        baslat();
    }
    Karar::Kur
}

/// Kurulumdan önce (eski ikiliyle) yazılan denetim betiği; zamanlayıcı 5 dk'da bir çalıştırır. Yeni ikiliye güvenmez:
/// kurulumdan 10 dk sonra servisler açık değilse ya da kurulumdan sonra merkeze sağlık raporu gitmemişse önceki paketi kurar.
/// Kurulum başarılıysa `bekleyen` kurulum bitince yeniden yazılır (süre ve sağlık raporu karşılaştırması oradan başlar).
pub const BETIK: &str = r#"#!/bin/sh
# Güncelleme denetimi: kurulumdan ÖNCE eski ikili yazdı; yeni ikiliye güvenmez. Zamanlayıcı 5 dk'da bir çalıştırır.
D=${WFC_GUNCELLEME_DIZIN:-/var/lib/wificorrect/guncelleme}
[ -f "$D/bekleyen" ] || exit 0
[ -f "$D/sonuc" ] && exit 0
[ $(( $(date +%s) - $(stat -c %Y "$D/bekleyen") )) -ge 600 ] || exit 0
ok=1
for s in wificorrect-portal wificorrect-panel wificorrect-kaydedici dnsmasq wificorrect-guvenlik; do
  systemctl is-active -q "$s" || ok=0
done
[ -f "$D/saglik_basari" ] && [ "$D/saglik_basari" -nt "$D/bekleyen" ] || ok=0
if [ "$ok" = 1 ]; then echo tamam > "$D/sonuc"; exit 0; fi
# sonuç yalnızca geri kurulum başarılıysa yazılır: başarısızsa (dpkg kilidi, yarım kurulum) sonraki turda yeniden denenir
dpkg --configure -a >/dev/null 2>&1
DEBIAN_FRONTEND=noninteractive apt-get install -y --allow-downgrades -o DPkg::Lock::Timeout=600 "$D/onceki.deb" && echo geri > "$D/sonuc"
"#;

/// Paketi `/api/paket`ten `hedef`e indirir (önce `.deb.tmp`); HTTP 200 değilse `indirme`, `kabul` tutmazsa `sha`
/// hatası; hata olursa yarım dosya silinir.
fn paket_indir(indir: &Indir<'_>, m: &Merkez, surum: &str, hedef: &Path, kabul: &dyn Fn(&Path) -> bool) -> Result<(), &'static str> {
    let tmp = hedef.with_extension("deb.tmp");
    let govde = json!({"cihaz_anahtari": m.cihaz_anahtari, "surum": surum}).to_string();
    let r = if !matches!(indir("/api/paket", &govde, &tmp), Ok(200)) {
        Err("indirme")
    } else if !kabul(&tmp) {
        Err("sha")
    } else {
        std::fs::rename(&tmp, hedef).map_err(|_| "disk")
    };
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// `ctl guncelle` (ayrı systemd işi `wfc-guncelle`): hedef sürümü indirir, doğrular ve kurar. Kurulumdan önce önceki
/// sürümün paketi `onceki.deb` olarak saklanır ve denetim betiği yazılır; apt başarısız olursa hemen önceki paket kurulur.
pub fn calistir_guncelle(cfg: &Config, y: &Yollar, m: &Merkez, indir: &Indir<'_>, komut: &dyn Fn(&[&str]) -> bool, kurulu: &str, now: f64) -> Result<String, String> {
    let Some(_k) = kilit_al(y) else { return Err("zaten sürüyor".into()) };
    let lr = cfg.main.log_root.as_str();
    uzlas_kilitli(y, lr, kurulu, now);
    let h: Hedef = std::fs::read(y.hedef()).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or("hedef sürüm yok")?;
    let mut d = durum_oku(y);
    if karar(kurulu, &h, &d, y.bekleyen().exists(), now) != Karar::Kur {
        return Ok("gerek yok".into());
    }
    denetim_yaz(lr, "GUNCELLEME_BASLADI", ek(&h.surum, kurulu, ""), now);
    d.onceki = kurulu.to_string();
    let _ = durum_yaz(y, &d);
    let hata = |neden: &str| -> Result<String, String> {
        denetim_yaz(lr, "GUNCELLEME_HATA", ek(&h.surum, kurulu, neden), now);
        let mut d = durum_oku(y);
        d.son = Some(Sonuc { surum: h.surum.clone(), sonuc: "hata".into(), zaman: now.max(0.0) as u64, neden: neden.into() });
        let _ = durum_yaz(y, &d);
        Err(format!("güncelleme yapılamadı: {neden}"))
    };
    if ortak::mkdirs(&y.paketler()).is_err() {
        return hata("disk");
    }
    // önceki sürümün paketi (geri dönüş için): merkezde sha yok, boş olmaması yeter
    let eski = y.paketler().join(format!("{kurulu}.deb"));
    if !eski.is_file() && paket_indir(indir, m, kurulu, &eski, &|p| std::fs::metadata(p).is_ok_and(|md| md.len() > 0)).is_err() {
        return hata("onceki_paket_yok");
    }
    let yeni = y.paketler().join(format!("{}.deb", h.surum));
    let dogru = |p: &Path| std::fs::metadata(p).is_ok_and(|md| md.len() == h.boyut) && crate::muhur::sha256_file(p).is_ok_and(|s| s == h.sha256);
    if let Err(neden) = paket_indir(indir, m, &h.surum, &yeni, &dogru) {
        return hata(neden);
    }
    if std::fs::copy(&eski, y.onceki_deb()).is_err() {
        return hata("disk");
    }
    let betik_yazildi = std::fs::write(y.betik(), BETIK).and_then(|_| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(y.betik(), std::fs::Permissions::from_mode(0o755))
    });
    let _ = std::fs::remove_file(y.sonuc());
    let bekleyen = json!({"yeni": h.surum, "onceki": kurulu, "zaman": now.max(0.0) as u64}).to_string();
    if betik_yazildi.is_err() || ortak::write_atomic(&y.bekleyen(), bekleyen.as_bytes()).is_err() {
        let _ = std::fs::remove_file(y.bekleyen());
        return hata("disk");
    }
    let apt = |deb: &Path| {
        let deb = deb.display().to_string();
        // dpkg kilidi (unattended-upgrades, apt-daily) 10 dk'ya kadar beklenir
        komut(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "--allow-downgrades", "-o", "DPkg::Lock::Timeout=600", &deb])
    };
    if apt(&yeni) {
        // apt sürerken eski ikilinin sağlık raporu `saglik_basari`ya dokunmuş olabilir: denetim kurulum bitişinden başlar
        let bitis = ortak::wall().max(0.0) as u64;
        let bekleyen = json!({"yeni": h.surum, "onceki": kurulu, "zaman": bitis}).to_string();
        let _ = ortak::write_atomic(&y.bekleyen(), bekleyen.as_bytes());
        return Ok("kuruldu, 10 dk sonra denetlenecek".into());
    }
    if apt(&y.onceki_deb()) {
        // sonuç sonraki raporda uzlaştırılır (GUNCELLEME_GERI_ALINDI, sürüm başarısız listesine)
        let _ = std::fs::write(y.sonuc(), "geri\n");
        return Err("kurulum başarısız; önceki sürüm geri kuruldu".into());
    }
    // ikisi de olmadı (ör. dpkg kilidi): sonuç yazılmaz, `bekleyen` kalır; denetim betiği geri kurulumu yeniden dener
    denetim_yaz(lr, "GUNCELLEME_HATA", ek(&h.surum, kurulu, "geri_kurulamadi"), now);
    Err("kurulum başarısız; önceki sürüm de kurulamadı, denetim yeniden deneyecek".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::path::Path;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn kur() -> Yollar {
        static N: AtomicUsize = AtomicUsize::new(0);
        let kok = std::env::temp_dir().join(format!("wfc-gunc-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&kok);
        std::fs::create_dir_all(&kok).unwrap();
        Yollar { kok }
    }

    fn hedef(hemen: bool) -> Hedef {
        Hedef { surum: "0.4.1".into(), sha256: "a".repeat(64), boyut: 100, hemen }
    }

    // 2026-10-12 00:00 UTC = TR 03:00
    const T0300: f64 = 1_791_763_200.0 + 0.0;

    fn tr_saat(s: f64) -> f64 {
        // TR gün başı (00:00) + s saat; 1_791_763_200 UTC = TR 03:00 → TR 00:00 = -3 saat
        T0300 - 3.0 * 3600.0 + s * 3600.0
    }

    #[test]
    fn pencere_sinirlari() {
        assert!(!pencere(tr_saat(3.0) - 60.0)); // 02:59
        assert!(pencere(tr_saat(3.0)));
        assert!(pencere(tr_saat(6.0) - 60.0)); // 05:59
        assert!(!pencere(tr_saat(6.0)));
    }

    #[test]
    fn karar_tablosu() {
        let now = tr_saat(4.0);
        let gece = now;
        let gunduz = tr_saat(14.0);
        let bos = Durum::default();
        assert_eq!(karar("", &hedef(false), &bos, false, gece), Karar::Bekle("paketsiz"));
        assert_eq!(karar("0.4.1", &hedef(false), &bos, false, gece), Karar::Bekle("guncel"));
        assert_eq!(karar("0.4.0", &hedef(false), &bos, true, gece), Karar::Bekle("denetim"));
        let basarisiz = Durum { basarisiz: vec!["0.4.1".into()], son: None, ..Default::default() };
        assert_eq!(karar("0.4.0", &hedef(false), &basarisiz, false, gece), Karar::Bekle("basarisiz"));
        assert_eq!(karar("0.4.0", &hedef(true), &basarisiz, false, gunduz), Karar::Kur); // hemen: başarısızı da dener
        let hata = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: gece as u64 - 3600, neden: String::new() }), ..Default::default() };
        assert_eq!(karar("0.4.0", &hedef(false), &hata, false, gece), Karar::Bekle("hata_bekle"));
        assert_eq!(karar("0.4.0", &hedef(true), &hata, false, gece), Karar::Kur);
        let eski_hata = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: gece as u64 - 50000, neden: String::new() }), ..Default::default() };
        assert_eq!(karar("0.4.0", &hedef(false), &eski_hata, false, gece), Karar::Kur);
        let baska = Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.0".into(), sonuc: "hata".into(), zaman: gece as u64 - 10, neden: String::new() }), ..Default::default() };
        assert_eq!(karar("0.3.0", &hedef(false), &baska, false, gece), Karar::Kur);
        assert_eq!(karar("0.4.0", &hedef(false), &bos, false, gunduz), Karar::Bekle("pencere"));
        assert_eq!(karar("0.4.0", &hedef(true), &bos, false, gunduz), Karar::Kur);
        assert_eq!(karar("0.4.0", &hedef(false), &bos, false, gece), Karar::Kur);
    }

    #[test]
    fn hedef_coz_dogrular() {
        let iyi = json!({"surum": "0.4.1", "sha256": "a".repeat(64), "boyut": 12345, "hemen": true});
        assert_eq!(hedef_coz(&iyi), Some(Hedef { surum: "0.4.1".into(), sha256: "a".repeat(64), boyut: 12345, hemen: true }));
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "0".repeat(64), "boyut": 1})).map(|h| h.hemen), Some(false));
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "A".repeat(64), "boyut": 1})), None); // büyük harf
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "a".repeat(63), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "0.4.1", "sha256": "g".repeat(64), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2", "sha256": "a".repeat(64), "boyut": 1})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2.3", "sha256": "a".repeat(64), "boyut": 0})), None);
        assert_eq!(hedef_coz(&json!({"surum": "1.2.3", "sha256": "a".repeat(64)})), None);
        assert_eq!(hedef_coz(&Value::Null), None);
    }

    #[test]
    fn kurulu_bos_cikti() {
        assert_eq!(kurulu(&|_: &[&str]| Some("0.4.0\n".into())), "0.4.0");
        assert_eq!(kurulu(&|_: &[&str]| Some(String::new())), "");
        assert_eq!(kurulu(&|_: &[&str]| None), "");
        let gorulen = std::cell::RefCell::new(vec![]);
        kurulu(&|c: &[&str]| {
            gorulen.borrow_mut().extend(c.iter().map(|s| s.to_string()));
            None
        });
        assert_eq!(gorulen.borrow()[..], ["dpkg-query", "-W", "-f=${Version}", "wificorrect"]);
    }

    fn bekleyen_yaz(y: &Yollar, sonuc: &str) {
        std::fs::write(y.bekleyen(), r#"{"yeni":"0.4.1","onceki":"0.4.0","zaman":1}"#).unwrap();
        std::fs::write(y.sonuc(), format!("{sonuc}\n")).unwrap();
    }

    fn denetim(log_root: &str, now: f64) -> String {
        std::fs::read_to_string(ortak::day_file(log_root, ortak::day_of(&ortak::now_iso(now)), "denetim.csv")).unwrap_or_default()
    }

    #[test]
    fn uzlas_tamam_ve_geri() {
        let y = kur();
        let lr = y.kok.join("5651").display().to_string();
        let now = tr_saat(4.0);
        // tamam: durum + denetim, dosyalar silinir, başarısız listesinden çıkar
        durum_yaz(&y, &Durum { basarisiz: vec!["0.4.1".into(), "0.3.9".into()], son: None, ..Default::default() }).unwrap();
        bekleyen_yaz(&y, "tamam");
        uzlas(&y, &lr, "0.4.1", now);
        let d = durum_oku(&y);
        assert_eq!(d.basarisiz, ["0.3.9"]);
        let s = d.son.unwrap();
        assert_eq!((s.surum.as_str(), s.sonuc.as_str(), s.zaman), ("0.4.1", "tamam", now as u64));
        assert!(!y.bekleyen().exists() && !y.sonuc().exists());
        let a = denetim(&lr, now);
        assert!(a.contains("GUNCELLEME_TAMAM") && a.contains("surum=0.4.1 onceki=0.4.0"), "{a}");
        // geri iki kez: başarısız'da tek
        for _ in 0..2 {
            bekleyen_yaz(&y, "geri");
            uzlas(&y, &lr, "0.4.0", now + 10.0);
        }
        let d = durum_oku(&y);
        assert_eq!(d.basarisiz.iter().filter(|s| *s == "0.4.1").count(), 1);
        assert_eq!(d.son.unwrap().sonuc, "geri_alindi");
        assert!(denetim(&lr, now).contains("GUNCELLEME_GERI_ALINDI"));
        assert!(!y.bekleyen().exists() && !y.sonuc().exists());
        // yalnız bekleyen (sonuç yok): dokunulmaz
        std::fs::write(y.bekleyen(), r#"{"yeni":"0.4.2","onceki":"0.4.1","zaman":1}"#).unwrap();
        uzlas(&y, &lr, "0.4.0", now);
        assert!(y.bekleyen().exists());
    }

    #[test]
    fn uzlas_paketleri_temizler() {
        let y = kur();
        std::fs::create_dir_all(y.paketler()).unwrap();
        // kurulu (0.3.8) ve önceki (0.3.9) en eskiler olsa da kalır; diğerlerinden yalnızca en yenisi; yarım indirme silinir
        for (i, ad) in ["0.3.8.deb", "0.3.9.deb", "0.4.0.deb", "0.4.1.deb", "0.4.2.deb.tmp"].iter().enumerate() {
            let p = y.paketler().join(ad);
            std::fs::write(&p, "x").unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64 * 100);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        durum_yaz(&y, &Durum { onceki: "0.3.9".into(), ..Default::default() }).unwrap();
        uzlas(&y, &y.kok.join("5651").display().to_string(), "0.3.8", 1000.0);
        let mut kalan: Vec<String> = std::fs::read_dir(y.paketler()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        kalan.sort();
        assert_eq!(kalan, ["0.3.8.deb", "0.3.9.deb", "0.4.1.deb"]);
    }

    #[test]
    fn rapor_alanlari_paket_ve_son() {
        let y = kur();
        let (p, g) = rapor_alanlari(&y, "0.4.0");
        assert_eq!((p.as_str(), g), ("0.4.0", Value::Null));
        durum_yaz(&y, &Durum { basarisiz: vec![], son: Some(Sonuc { surum: "0.4.1".into(), sonuc: "hata".into(), zaman: 77, neden: "x".into() }), ..Default::default() }).unwrap();
        let (_, g) = rapor_alanlari(&y, "");
        assert_eq!(g, json!({"surum": "0.4.1", "sonuc": "hata", "zaman": 77})); // neden gitmez
        assert_eq!(rapor_alanlari(&y, "0.4.1-1").0, ""); // merkezin biçimine uymayan sürüm gönderilmez
    }

    #[test]
    fn hedefi_isle_yazar_ve_baslatir() {
        let y = kur();
        let sayac = std::cell::Cell::new(0);
        let b = || sayac.set(sayac.get() + 1);
        let gece = tr_saat(4.0);
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.1", gece, &b), Karar::Bekle("guncel"));
        assert_eq!((sayac.get(), y.hedef().exists()), (0, false));
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.0", tr_saat(14.0), &b), Karar::Bekle("pencere"));
        assert_eq!(sayac.get(), 0);
        assert_eq!(hedefi_isle(&y, &hedef(false), "0.4.0", gece, &b), Karar::Kur);
        assert_eq!(sayac.get(), 1);
        let yazilan: Hedef = serde_json::from_slice(&std::fs::read(y.hedef()).unwrap()).unwrap();
        assert_eq!(yazilan, hedef(false));
    }

    // ---------------------------------------------------------------- ctl guncelle
    const YENI: &[u8] = b"yeni paket icerigi";
    const ESKI: &[u8] = b"eski paket";

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
    }

    fn ortam() -> (Yollar, Config, Merkez) {
        let y = kur();
        let mut cfg = Config::default();
        cfg.main.log_root = y.kok.join("5651").display().to_string();
        let m = Merkez { cihaz_anahtari: "anahtar".into(), ..Default::default() };
        (y, cfg, m)
    }

    fn hedef_koy(y: &Yollar) -> Hedef {
        let h = Hedef { surum: "0.4.1".into(), sha256: sha(YENI), boyut: YENI.len() as u64, hemen: false };
        ortak::write_atomic(&y.hedef(), &serde_json::to_vec(&h).unwrap()).unwrap();
        h
    }

    type Indirilen = RefCell<Vec<(String, Value, PathBuf)>>;

    /// Sahte indirme: 0.4.0 → `onceki`, diğerleri → `yeni` (kod, gövde); gövde çıktı dosyasına yazılır.
    fn indir_sahte<'a>(onceki: (u16, &'a [u8]), yeni: (u16, &'a [u8]), kayit: &'a Indirilen) -> impl Fn(&str, &str, &Path) -> Result<u16, String> + 'a {
        move |yol, govde, cikti| {
            let v: Value = serde_json::from_str(govde).unwrap();
            kayit.borrow_mut().push((yol.to_string(), v.clone(), cikti.to_path_buf()));
            let (kod, b) = if v["surum"] == "0.4.0" { onceki } else { yeni };
            std::fs::write(cikti, b).unwrap();
            Ok(kod)
        }
    }

    /// Kayıt tutan komut; i. çağrının sonucu `sonuclar[i]` (yoksa başarı).
    fn komut_sahte<'a>(sonuclar: &'a [bool], kayit: &'a RefCell<Vec<Vec<String>>>) -> impl Fn(&[&str]) -> bool + 'a {
        move |c| {
            let mut k = kayit.borrow_mut();
            let i = k.len();
            k.push(c.iter().map(|s| s.to_string()).collect());
            sonuclar.get(i).copied().unwrap_or(true)
        }
    }

    fn apt(deb: &Path) -> Vec<String> {
        ["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "--allow-downgrades", "-o", "DPkg::Lock::Timeout=600"].iter().map(|s| s.to_string()).chain([deb.display().to_string()]).collect()
    }

    #[test]
    fn guncelle_basari() {
        use std::os::unix::fs::PermissionsExt;
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        hedef_koy(&y);
        std::fs::write(y.sonuc(), "tamam\n").unwrap(); // bekleyensiz eski sonuç silinmeli
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        // apt sürerken eski ikilinin sağlık raporu saglik_basari'ya dokunur
        let apt_sirasinda_rapor = |c: &[&str]| {
            std::fs::write(y.saglik_basari(), "1\n").unwrap();
            komut_sahte(&[], &kk)(c)
        };
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, YENI), &ik), &apt_sirasinda_rapor, "0.4.0", now);
        assert_eq!(r, Ok("kuruldu, 10 dk sonra denetlenecek".into()));
        let mtime = |p: PathBuf| std::fs::metadata(p).unwrap().modified().unwrap();
        assert!(mtime(y.saglik_basari()) <= mtime(y.bekleyen()), "bekleyen kurulumdan sonra yeniden yazılmalı");
        {
            let ik = ik.borrow();
            assert_eq!(ik.len(), 2);
            assert_eq!((ik[0].0.as_str(), ik[0].1["surum"].as_str(), ik[0].1["cihaz_anahtari"].as_str()), ("/api/paket", Some("0.4.0"), Some("anahtar")));
            assert_eq!((ik[1].1["surum"].as_str(), &ik[1].2), (Some("0.4.1"), &y.paketler().join("0.4.1.deb.tmp")));
        }
        assert_eq!(std::fs::read(y.paketler().join("0.4.0.deb")).unwrap(), ESKI);
        assert_eq!(std::fs::read(y.onceki_deb()).unwrap(), ESKI);
        assert_eq!(std::fs::read(y.paketler().join("0.4.1.deb")).unwrap(), YENI);
        assert!(!y.paketler().join("0.4.1.deb.tmp").exists());
        assert_eq!(std::fs::read_to_string(y.betik()).unwrap(), BETIK);
        assert_eq!(std::fs::metadata(y.betik()).unwrap().permissions().mode() & 0o777, 0o755);
        let b: Value = serde_json::from_slice(&std::fs::read(y.bekleyen()).unwrap()).unwrap();
        assert_eq!((b["yeni"].as_str(), b["onceki"].as_str()), (Some("0.4.1"), Some("0.4.0")));
        assert!(!y.sonuc().exists());
        assert_eq!(*kk.borrow(), vec![apt(&y.paketler().join("0.4.1.deb"))]);
        let a = denetim(&cfg.main.log_root, now);
        assert!(a.contains("GUNCELLEME_BASLADI") && a.contains("surum=0.4.1 onceki=0.4.0"), "{a}");
        assert_eq!(durum_oku(&y).onceki, "0.4.0");
        // denetim sürerken yeniden: kurulmaz
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, YENI), &ik), &komut_sahte(&[], &kk), "0.4.0", now);
        assert_eq!((r, kk.borrow().len()), (Ok("gerek yok".into()), 1));
    }

    #[test]
    fn guncelle_onceki_paket_varsa_indirilmez() {
        let (y, cfg, m) = ortam();
        hedef_koy(&y);
        std::fs::create_dir_all(y.paketler()).unwrap();
        std::fs::write(y.paketler().join("0.4.0.deb"), ESKI).unwrap();
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((404, b""), (200, YENI), &ik), &komut_sahte(&[], &kk), "0.4.0", tr_saat(4.0));
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(ik.borrow().len(), 1);
        assert_eq!(std::fs::read(y.onceki_deb()).unwrap(), ESKI);
    }

    #[test]
    fn guncelle_sha_tutmaz() {
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        hedef_koy(&y);
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let bozuk = vec![b'z'; YENI.len()]; // aynı boyutta bozuk içerik
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, &bozuk), &ik), &komut_sahte(&[], &kk), "0.4.0", now);
        assert!(r.is_err());
        assert!(kk.borrow().is_empty());
        assert!(!y.paketler().join("0.4.1.deb.tmp").exists() && !y.paketler().join("0.4.1.deb").exists() && !y.bekleyen().exists());
        let s = durum_oku(&y).son.unwrap();
        assert_eq!((s.surum.as_str(), s.sonuc.as_str(), s.neden.as_str()), ("0.4.1", "hata", "sha"));
        let a = denetim(&cfg.main.log_root, now);
        assert!(a.contains("GUNCELLEME_HATA") && a.contains("surum=0.4.1 onceki=0.4.0 neden=sha"), "{a}");
        // merkez paketi vermedi (404): neden=indirme, yarım dosya kalmaz
        std::fs::remove_file(y.durum()).unwrap();
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (404, b"{}"), &ik), &komut_sahte(&[], &kk), "0.4.0", now);
        assert!(r.is_err() && kk.borrow().is_empty());
        assert_eq!(durum_oku(&y).son.unwrap().neden, "indirme");
        assert!(!y.paketler().join("0.4.1.deb.tmp").exists());
    }

    #[test]
    fn guncelle_onceki_paket_yok() {
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        for onceki in [(404u16, &b"{\"hata\":\"yok\"}"[..]), (200, &b""[..])] {
            let _ = std::fs::remove_file(y.durum());
            hedef_koy(&y);
            let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
            let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte(onceki, (200, YENI), &ik), &komut_sahte(&[], &kk), "0.4.0", now);
            assert!(r.is_err());
            assert!(kk.borrow().is_empty());
            assert_eq!(ik.borrow().len(), 1); // yeni paket indirilmedi
            assert!(!y.paketler().join("0.4.0.deb").exists() && !y.paketler().join("0.4.0.deb.tmp").exists());
            assert_eq!(durum_oku(&y).son.unwrap().neden, "onceki_paket_yok");
        }
        assert!(denetim(&cfg.main.log_root, now).contains("neden=onceki_paket_yok"));
    }

    #[test]
    fn guncelle_apt_basarisiz_geri_doner() {
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        hedef_koy(&y);
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, YENI), &ik), &komut_sahte(&[false, true], &kk), "0.4.0", now);
        assert!(r.is_err());
        assert_eq!(*kk.borrow(), vec![apt(&y.paketler().join("0.4.1.deb")), apt(&y.onceki_deb())]);
        assert_eq!(std::fs::read_to_string(y.sonuc()).unwrap().trim(), "geri");
        // sonraki rapordaki uzlaşma: başarısız listesine girer
        uzlas(&y, &cfg.main.log_root, "0.4.0", now + 60.0);
        let d = durum_oku(&y);
        assert_eq!((d.basarisiz, d.son.unwrap().sonuc), (vec!["0.4.1".to_string()], "geri_alindi".to_string()));
    }

    #[test]
    fn guncelle_iki_apt_basarisiz_betik_yeniden_dener() {
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        hedef_koy(&y);
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, YENI), &ik), &komut_sahte(&[false, false], &kk), "0.4.0", now);
        assert!(r.is_err());
        assert_eq!(kk.borrow().len(), 2);
        assert!(kk.borrow().iter().all(|c| c.iter().any(|a| a == "DPkg::Lock::Timeout=600")));
        // sonuç yok, bekleyen kalır (denetim betiği geri kurulumu dener); başarısız sayılmaz
        assert!(y.bekleyen().exists() && !y.sonuc().exists());
        assert!(denetim(&cfg.main.log_root, now).contains("neden=geri_kurulamadi"));
        uzlas(&y, &cfg.main.log_root, "0.4.0", now + 60.0);
        assert!(y.bekleyen().exists() && durum_oku(&y).basarisiz.is_empty());
        // betik eski sürümü sağlıklı bulursa (yeni sürüm hiç kurulamamış): tamam ama kurulu ≠ yeni → hata, başarısız değil
        std::fs::write(y.sonuc(), "tamam\n").unwrap();
        uzlas(&y, &cfg.main.log_root, "0.4.0", now + 700.0);
        let d = durum_oku(&y);
        assert!(d.basarisiz.is_empty() && !y.bekleyen().exists());
        let s = d.son.unwrap();
        assert_eq!((s.sonuc.as_str(), s.neden.as_str()), ("hata", "kurulmadi"));
        assert!(!denetim(&cfg.main.log_root, now).contains("GUNCELLEME_TAMAM"));
    }

    #[test]
    fn kilit_tutulurken_hicbiri_calismaz() {
        let (y, cfg, m) = ortam();
        let now = tr_saat(4.0);
        let h = hedef_koy(&y);
        let k = kilit_al(&y).unwrap();
        let (ik, kk) = (RefCell::new(vec![]), RefCell::new(vec![]));
        let r = calistir_guncelle(&cfg, &y, &m, &indir_sahte((200, ESKI), (200, YENI), &ik), &komut_sahte(&[], &kk), "0.4.0", now);
        assert_eq!(r, Err("zaten sürüyor".into()));
        assert!(ik.borrow().is_empty() && kk.borrow().is_empty());
        // hedefi_isle hedef.json'u ezmez, başlatmaz
        std::fs::remove_file(y.hedef()).unwrap();
        let sayac = std::cell::Cell::new(0);
        assert_eq!(hedefi_isle(&y, &h, "0.4.0", now, &|| sayac.set(sayac.get() + 1)), Karar::Bekle("suruyor"));
        assert!(!y.hedef().exists() && sayac.get() == 0);
        // uzlaşma dokunmaz
        bekleyen_yaz(&y, "tamam");
        uzlas(&y, &cfg.main.log_root, "0.4.1", now);
        assert!(y.bekleyen().exists() && y.sonuc().exists());
        drop(k);
        uzlas(&y, &cfg.main.log_root, "0.4.1", now);
        assert!(!y.bekleyen().exists());
        // kilit bırakılınca hedefi_isle başlatır; başlatırken kilidi tutmaz (başlatılan iş kilidi alabilmeli)
        let kilitli = std::cell::Cell::new(true);
        assert_eq!(hedefi_isle(&y, &h, "0.4.0", now, &|| kilitli.set(kilit_al(&y).is_none())), Karar::Kur);
        assert!(!kilitli.get());
    }

    #[test]
    fn betik_sozdizimi() {
        let mut c = Command::new("sh").arg("-n").stdin(std::process::Stdio::piped()).spawn().unwrap();
        c.stdin.take().unwrap().write_all(BETIK.as_bytes()).unwrap();
        assert!(c.wait().unwrap().success());
        assert!(BETIK.starts_with("#!/bin/sh\n"));
    }

    fn dokun(p: &Path, ne_zaman: &str) {
        assert!(Command::new("touch").arg("-d").arg(ne_zaman).arg(p).status().unwrap().success());
    }

    #[test]
    fn betik_davranisi() {
        use std::os::unix::fs::PermissionsExt;
        let y = kur();
        let d = &y.kok;
        let bin = d.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let apt_kayit = d.join("apt.txt");
        let sahte = [
            ("systemctl", format!("#!/bin/sh\n[ -f '{}' ] && exit 3\nexit 0\n", d.join("servis_durdu").display())),
            ("apt-get", format!("#!/bin/sh\necho \"$DEBIAN_FRONTEND $*\" >> '{}'\n[ -f '{}' ] && exit 1\nexit 0\n", apt_kayit.display(), d.join("apt_hata").display())),
            ("dpkg", format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 1\n", d.join("dpkg.txt").display())),
        ];
        for (ad, icerik) in sahte {
            std::fs::write(bin.join(ad), icerik).unwrap();
            std::fs::set_permissions(bin.join(ad), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let betik = d.join("denetle.sh");
        std::fs::write(&betik, BETIK).unwrap();
        let calistir = || {
            let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
            let basari = Command::new("sh").arg(&betik).env("WFC_GUNCELLEME_DIZIN", d).env("PATH", path).status().unwrap().success();
            // geri kurulum başarısızsa betik de başarısız biter (birim günlükte hata görünür)
            assert_eq!(basari, !d.join("apt_hata").exists());
        };
        let sonuc = || std::fs::read_to_string(d.join("sonuc")).ok().map(|s| s.trim().to_string());
        let sifirla = || {
            let _ = std::fs::remove_file(d.join("sonuc"));
            let _ = std::fs::remove_file(&apt_kayit);
        };
        // bekleyen yok: hiçbir şey
        calistir();
        assert_eq!(sonuc(), None);
        // 11 dk önce kuruldu, sağlık raporu hiç gitmedi → geri
        std::fs::write(d.join("bekleyen"), "{}").unwrap();
        dokun(&d.join("bekleyen"), "11 minutes ago");
        // geri kurulum başarısız (dpkg kilidi): sonuç yazılmaz, sonraki turda yeniden denenir
        std::fs::write(d.join("apt_hata"), "").unwrap();
        calistir();
        calistir();
        assert_eq!(sonuc(), None);
        let beklenen = format!("noninteractive install -y --allow-downgrades -o DPkg::Lock::Timeout=600 {}/onceki.deb", d.display());
        assert_eq!(std::fs::read_to_string(&apt_kayit).unwrap().lines().collect::<Vec<_>>(), [beklenen.as_str(), beklenen.as_str()]);
        assert_eq!(std::fs::read_to_string(d.join("dpkg.txt")).unwrap().lines().next(), Some("--configure -a")); // çıkış kodu yok sayılır
        std::fs::remove_file(d.join("apt_hata")).unwrap();
        std::fs::remove_file(&apt_kayit).unwrap();
        calistir();
        assert_eq!(sonuc().as_deref(), Some("geri"));
        assert_eq!(std::fs::read_to_string(&apt_kayit).unwrap().trim(), beklenen);
        // sonuç varken yeniden çalışırsa dokunmaz
        std::fs::remove_file(&apt_kayit).unwrap();
        calistir();
        assert!(!apt_kayit.exists());
        // sağlık raporu kurulumdan sonra gitti, servisler açık → tamam, apt çağrılmaz
        sifirla();
        dokun(&d.join("saglik_basari"), "1 minute ago");
        calistir();
        assert_eq!(sonuc().as_deref(), Some("tamam"));
        assert!(!apt_kayit.exists());
        // bir servis durmuş → geri
        sifirla();
        std::fs::write(d.join("servis_durdu"), "").unwrap();
        calistir();
        assert_eq!(sonuc().as_deref(), Some("geri"));
        assert!(apt_kayit.exists());
        // sağlık raporu kurulumdan önce → geri
        sifirla();
        std::fs::remove_file(d.join("servis_durdu")).unwrap();
        dokun(&d.join("saglik_basari"), "20 minutes ago");
        calistir();
        assert_eq!(sonuc().as_deref(), Some("geri"));
        // 10 dk dolmadı: henüz karar yok
        sifirla();
        dokun(&d.join("bekleyen"), "1 minute ago");
        calistir();
        assert_eq!(sonuc(), None);
        assert!(!apt_kayit.exists());
    }
}
