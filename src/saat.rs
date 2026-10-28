//! Saat doğrulaması ve 5651 satırlarının bekletilmesi (2026-10-26; docs/KURULUM_GUNLUGU.md "Saat doğrulanmadan kayıt").
//! Visavis'in kurulumdan sonraki ilk açılışında RTC 2014'teydi (kurucu hwclock bulamamıştı), systemd saati kendi paket
//! tarihine (13 Nisan 2026 22:38) getirdi ve servisler 3 satırı 2026-04-13'e yazdı; NTP saati 185 gün ileri atınca
//! gün-kapat o günü mühürledi.
//!
//! Her açılışın ilk 5651 yazımında saatin kaynağına karar verilir (`/run/wificorrect/saat.json`):
//! - `ntp`: timesyncd bu açılışta eşitledi (`/run/systemd/timesync/synchronized`) ya da çekirdek eşitlenmiş diyor;
//! - `rtc`: önceki açılışta saat doğrulanmıştı (`saat-dogru` işareti; kurucu da RTC'yi yazınca bırakır), sistem saati
//!   RTC'den geliyor (systemd ileri almamış) ve RTC son kayıtlı zamandan (timesyncd'nin dakikada bir güncellediği dosya,
//!   programın kendi dosyası) geride değil;
//! - `bekliyor`: satırlar gün dosyalarına değil `<log_root>/bekleyen/<açılış>.jsonl`'a, yazıldıkları andaki gerçek zaman ile
//!   açılıştan beri geçen süre (CLOCK_BOOTTIME) farkıyla birikir. Saat doğrulanınca (`ilerle`: kaydedici her döngüde,
//!   gün-kapat/temizlik/fabrika önce) her satırın zamanı aradaki sıçrama kadar düzeltilip doğru güne yazılır (`ek`'te
//!   `ham_zaman=`); o açılışta açılan oturumların başlangıcı/bitişi ve kişi listesi de kaydırılır, denetime
//!   SAAT_DOGRULANDI. Önceki açılışlardan kalanlar (saat hiç doğrulanmadan cihaz yeniden başladı) düzeltilemez: ham
//!   zamanıyla ve `saat=dogrulanamadi` işaretiyle yazılır.
//!
//! Portal ve Servis saati beklemez (internetsiz cihazda giriş sayfası açılmalı); yalnızca satırların yeri bekler.
//! Mühürleme, temizlik ve fabrika dönüşü saat doğrulanmadan ve bekleyenler yazılmadan çalışmaz (`hazir`).

use crate::ayar::Config;
use crate::ortak::{self, Row};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const NTP: &str = "ntp";
pub const RTC: &str = "rtc";
pub const BEKLIYOR: &str = "bekliyor";
/// Satır, bekleme dosyasına yazılmadan en çok bu kadar önce zamanlanmış olabilir (kapanan oturumun bitişi, kuyruk).
const GECIKME_SN: f64 = 600.0;
/// RTC'ye güvenilen açılışta NTP bundan büyük fark bulursa SAAT_SICRAMA yazılır.
const SICRAMA_SN: f64 = 120.0;

#[derive(Clone, Debug)]
pub struct Yollar {
    /// bu açılışın kararı (tmpfs: açılışta silinir)
    pub durum: PathBuf,
    pub kilit: PathBuf,
    /// "önceki açılışta saat doğrulandı" (kurucu da yazar)
    pub isaret: PathBuf,
    /// timesyncd bu açılışta eşitlediyse var
    pub senkron: PathBuf,
    /// RTC'nin zamanı (sn, UTC)
    pub rtc: PathBuf,
    /// değişiklik zamanları saatin alt sınırı: timesyncd'nin zaman dosyası, programın kendisi
    pub taban: Vec<PathBuf>,
    pub acilis: PathBuf,
    /// gün-kapat saat yüzünden ertelendi: doğrulanınca kaydedici çalıştırır
    pub gun_kapat: PathBuf,
    /// çekirdeğin NTP durumu (adjtimex) da sorulsun mu; testte hayır
    pub cekirdek: bool,
}

impl Yollar {
    pub fn sistem() -> Yollar {
        Yollar {
            durum: "/run/wificorrect/saat.json".into(),
            kilit: "/run/wificorrect/saat.kilit".into(),
            isaret: "/var/lib/wificorrect/saat-dogru".into(),
            senkron: "/run/systemd/timesync/synchronized".into(),
            rtc: "/sys/class/rtc/rtc0/since_epoch".into(),
            taban: vec!["/var/lib/systemd/timesync/clock".into(), "/proc/self/exe".into()],
            acilis: "/proc/sys/kernel/random/boot_id".into(),
            gun_kapat: "/run/wificorrect/gun-kapat-bekliyor".into(),
            cekirdek: true,
        }
    }
}

/// Programın kullandığı yollar; testte yalnızca `DENEME` verilmişse (yoksa saat hep doğrulanmış sayılır).
#[cfg(not(test))]
pub fn ortam() -> Option<Yollar> {
    Some(Yollar::sistem())
}

#[cfg(test)]
thread_local! {
    pub static DENEME: std::cell::RefCell<Option<Yollar>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn ortam() -> Option<Yollar> {
    DENEME.with(|d| d.borrow().clone())
}

/// Bu süreçte saatin doğrulandığı görüldü (açılış içinde geri dönmez): sıcak yolda dosya okunmaz.
#[cfg(not(test))]
static DOGRU: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(not(test))]
fn dogru_biliniyor() -> bool {
    DOGRU.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(test))]
fn dogru_gordum() {
    DOGRU.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
fn dogru_biliniyor() -> bool {
    false
}

#[cfg(test)]
fn dogru_gordum() {}

/// Aynı anda okunan gerçek zaman ve açılıştan beri geçen süre (CLOCK_BOOTTIME; saat ayarından etkilenmez).
#[derive(Clone, Copy, Debug)]
pub struct Saatler {
    pub gercek: f64,
    pub acilistan: f64,
}

impl Saatler {
    /// Saat sıçrayınca yalnızca bu değişir.
    pub fn fark(&self) -> f64 {
        self.gercek - self.acilistan
    }
}

pub fn simdi() -> Saatler {
    let oku = |id: libc::clockid_t| {
        let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: geçerli timespec göstergesi
        unsafe { libc::clock_gettime(id, &mut t) };
        t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
    };
    Saatler { acilistan: oku(libc::CLOCK_BOOTTIME), gercek: oku(libc::CLOCK_REALTIME) }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Durum {
    pub acilis: String,
    pub kaynak: String,
    /// karar anındaki `Saatler::fark`: sonraki sıçrama bunun değişimidir
    pub fark: f64,
    /// `rtc` açılışında NTP eşitlemesine bakıldı mı (SAAT_SICRAMA bir kez)
    #[serde(default)]
    pub ntp_bakildi: bool,
}

fn acilis_kimligi(y: &Yollar) -> String {
    let k = fs::read_to_string(&y.acilis).map(|s| s.trim().to_string()).unwrap_or_default();
    if k.is_empty() || !k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return "bilinmeyen".into();
    }
    k
}

/// Bu açılışın kararı (yoksa ya da başka açılışınsa `None`).
pub fn durum_oku(y: &Yollar) -> Option<Durum> {
    let d: Durum = serde_json::from_slice(&fs::read(&y.durum).ok()?).ok()?;
    (d.acilis == acilis_kimligi(y)).then_some(d)
}

fn durum_yaz(y: &Yollar, d: &Durum) {
    if let Err(e) = ortak::write_atomic_mode(&y.durum, &serde_json::to_vec(d).unwrap_or_default(), 0o644) {
        eprintln!("saat: {} yazılamadı: {e}", y.durum.display());
    }
}

/// Doğrulanan açılış işareti bırakır, doğrulanmayan siler: bir sonraki açılış RTC'ye ancak bu açılışta saat doğrulandıysa güvenir.
fn isaret_guncelle(y: &Yollar, d: &Durum, s: Saatler) {
    let r = if d.kaynak == BEKLIYOR {
        fs::remove_file(&y.isaret).or_else(|e| if e.kind() == ErrorKind::NotFound { Ok(()) } else { Err(e) })
    } else {
        ortak::write_atomic_mode(&y.isaret, format!("dogru {} {} {}\n", d.acilis, s.gercek as i64, d.kaynak).as_bytes(), 0o644)
    };
    if let Err(e) = r {
        eprintln!("saat: {} güncellenemedi: {e}", y.isaret.display());
    }
}

pub fn senkron_mu(y: &Yollar) -> bool {
    y.senkron.exists() || (y.cekirdek && cekirdek_esit())
}

/// Çekirdek saati NTP ile eşitlenmiş sayıyor mu (timedatectl'in NTPSynchronized'ı; chrony de bunu ayarlar).
fn cekirdek_esit() -> bool {
    // SAFETY: modes = 0 → adjtimex yalnızca okur
    let mut t: libc::timex = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::adjtimex(&mut t) };
    r >= 0 && r != libc::TIME_ERROR && t.status & libc::STA_UNSYNC == 0
}

fn mtime(p: &Path) -> Option<f64> {
    fs::metadata(p).and_then(|m| m.modified()).ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

/// RTC'ye güvenilir mi: önceki açılış doğrulanmıştı, sistem saati RTC'den alınmış (systemd/timesyncd ileri almamış: ileri
/// alırlarsa RTC sıfırlanmış demektir) ve RTC kayıtlı alt sınırdan geride değil.
pub fn rtc_guvenilir(y: &Yollar, s: Saatler) -> bool {
    let isaret = fs::read_to_string(&y.isaret).is_ok_and(|t| t.starts_with("dogru"));
    let rtc = fs::read_to_string(&y.rtc).ok().and_then(|t| t.trim().parse::<f64>().ok());
    let taban = y.taban.iter().filter_map(|p| mtime(p)).fold(0.0, f64::max);
    isaret && rtc.is_some_and(|r| (s.gercek - r).abs() <= 10.0 && r + 5.0 >= taban)
}

fn karar(y: &Yollar, s: Saatler) -> Durum {
    let kaynak = if senkron_mu(y) {
        NTP
    } else if rtc_guvenilir(y, s) {
        RTC
    } else {
        BEKLIYOR
    };
    Durum { acilis: acilis_kimligi(y), kaynak: kaynak.into(), fark: s.fark(), ntp_bakildi: kaynak == NTP }
}

/// Bu açılışın saat durumu; ilk çağrıda karar verilip yazılır (işaret de güncellenir).
pub fn durum(y: &Yollar, s: Saatler) -> Durum {
    if let Some(d) = durum_oku(y) {
        return d;
    }
    let _k = ortak::dosya_kilidi(&y.kilit).ok();
    if let Some(d) = durum_oku(y) {
        return d;
    }
    let d = karar(y, s);
    durum_yaz(y, &d);
    isaret_guncelle(y, &d, s);
    d
}

/// Bekleme dosyasının bir satırı: hedef (gün klasörüne göre yol), yazım anındaki açılıştan beri süre ve gerçek zaman farkı.
#[derive(Serialize, Deserialize)]
struct Biriken {
    g: String,
    b: f64,
    f: f64,
    s: Vec<String>,
}

fn bekleyen_dizini(kok: &Path) -> PathBuf {
    kok.join("bekleyen")
}

fn bekleme_dosyalari(kok: &Path) -> std::io::Result<Vec<PathBuf>> {
    match fs::read_dir(bekleyen_dizini(kok)) {
        Ok(rd) => Ok(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn bekleyen_var(kok: &Path) -> bool {
    bekleme_dosyalari(kok).is_ok_and(|v| !v.is_empty())
}

/// Bekleyen satır ve dosya sayısı (`ctl saat`).
pub fn bekleyen_sayisi(kok: &Path) -> (usize, usize) {
    let dosyalar = bekleme_dosyalari(kok).unwrap_or_default();
    let satir = dosyalar.iter().map(|p| fs::read_to_string(p).map(|t| t.lines().count()).unwrap_or(0)).sum();
    (satir, dosyalar.len())
}

fn biriktir(kok: &Path, acilis: &str, goreli: &str, rows: &[Row], s: Saatler) -> std::io::Result<()> {
    let dir = bekleyen_dizini(kok);
    ortak::mkdirs(&dir)?;
    let mut out = String::new();
    for r in rows {
        out.push_str(&serde_json::to_string(&Biriken { g: goreli.into(), b: s.acilistan, f: s.fark(), s: r.0.to_vec() })?);
        out.push('\n');
    }
    OpenOptions::new().append(true).create(true).mode(0o600).open(dir.join(format!("{acilis}.jsonl")))?.write_all(out.as_bytes())
}

/// `ortak::append_rows`'tan: hedef bir 5651 gün dosyasıysa ve saat bu açılışta doğrulanmadıysa satırlar bekleme dosyasına
/// yazılır (`true`); doğrulandıysa `false` (çağıran gün dosyasına yazar).
pub fn biriktir_gerekirse(path: &Path, rows: &[Row]) -> std::io::Result<bool> {
    if dogru_biliniyor() {
        return Ok(false);
    }
    let Some(y) = ortam() else { return Ok(false) };
    let Some(h) = ortak::gun_hedefi(path) else { return Ok(false) };
    if durum(&y, simdi()).kaynak != BEKLIYOR {
        dogru_gordum();
        return Ok(false);
    }
    let _k = ortak::dosya_kilidi(&y.kilit)?;
    // kilidi beklerken saat doğrulanıp bekleyenler yazılmış olabilir
    let Some(d) = durum_oku(&y).filter(|d| d.kaynak == BEKLIYOR) else { return Ok(false) };
    biriktir(&h.kok, &d.acilis, &h.goreli, rows, simdi())?;
    Ok(true)
}

/// Kişi dosyasının başlığı (KAYIT): saat doğrulanmadıysa satırların hangi güne düşeceği bilinmez, var olan dosyaya
/// bakılamaz → `Some(bu süreçte o kişi için ilk bekletme mi)` (her toplu yazımda değil: uzun bekleyişte dosya şişmesin;
/// aktarım her yeni gün dosyasının başına koyar, `yaz`). Doğrulandıysa `None`: dosya yoksa eklenir.
pub fn kayit_bekletmesi(path: &Path) -> Option<bool> {
    static GORULEN: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());
    if dogru_biliniyor() {
        return None;
    }
    let y = ortam()?;
    let h = ortak::gun_hedefi(path)?;
    if durum(&y, simdi()).kaynak != BEKLIYOR {
        return None;
    }
    Some(GORULEN.lock().map(|mut g| g.insert(format!("{}|{}", h.kok.display(), h.goreli))).unwrap_or(true))
}

/// Aktarılan satırlardan çıkan düzeltmeler.
#[derive(Default, Debug)]
pub struct Rapor {
    pub duzeltilen: usize,
    pub onceki: usize,
    pub bozuk: usize,
    /// oturum kimliği → (başlangıç kayması, bitiş kayması; misafir kodunda bitiş koda bağlı, kaymaz)
    kaydir: BTreeMap<String, (f64, f64)>,
    /// kişi listesi: (telefon, ham zaman, düzeltilmiş zaman)
    kisi: Vec<(String, String, String)>,
}

/// Bekleme dosyasının satırı: (bekletilen satır, satır). Bozuksa `None`.
fn ayristir(l: &str) -> Option<(Biriken, Row)> {
    let b: Biriken = serde_json::from_str(l).ok()?;
    let dizi = <[String; 19]>::try_from(b.s.clone()).ok()?;
    Some((b, Row(dizi)))
}

/// Bir bekleme dosyasındaki farklı yazım farkları (1 sn'den yakınlar bir sayılır): saat sıçradıkça bir tane eklenir.
fn farklar<'a>(satirlar: impl Iterator<Item = &'a Biriken>) -> Vec<f64> {
    let mut v: Vec<f64> = vec![];
    for b in satirlar {
        if !v.iter().any(|f| (f - b.f).abs() < 1.0) {
            v.push(b.f);
        }
    }
    v
}

/// Bu açılışın bir satırı: yazım anındaki farkla zamanlanmış sayılır; satırın zamanı o farkla tutmuyorsa (eşitleme anında
/// kuyrukta bekleyen olay: eski farkla zamanlandı, yeni farkla yazıldı) dosyadaki diğer farklar denenir. Hiçbiri tutmazsa
/// (bitişi eski bir oturum) zaman olduğu gibi kalır, `saat=duzeltilmedi`.
fn duzelt(b: &Biriken, row: Row, farklar: &[f64], s: Saatler, rapor: &mut Rapor) -> Row {
    let ham = row.0[0].clone();
    let taban = crate::kayit::parse_time(&ham).and_then(|r| {
        let tutar = |f: f64| (-2.0..=GECIKME_SN).contains(&(b.b + f - r));
        std::iter::once(b.f).chain(farklar.iter().copied()).find(|&f| tutar(f)).map(|f| (r, s.fark() - f))
    });
    match taban {
        None => row.ek_ekle("saat=duzeltilmedi"),
        Some((_, delta)) if delta.abs() < 1.0 => row,
        Some((r, delta)) => {
            let yeni = ortak::now_iso((r + delta).round());
            rapor.duzeltilen += 1;
            if row.get("olay") == "OTURUM_BASLA" && !b.g.starts_with("kullanicilar/") {
                let bitis = if row.get("ek").contains("giris=misafir_kodu") { 0.0 } else { delta };
                rapor.kaydir.insert(row.get("oturum_id").to_string(), (delta, bitis));
                rapor.kisi.push((row.get("telefon").to_string(), ham.clone(), yeni.clone()));
            }
            let mut row = row.ek_ekle(&format!("ham_zaman={ham}"));
            row.0[0] = yeni;
            row
        }
    }
}

/// Aktarım boyunca kişi dosyalarının başlığı: satır yazılan dosyalar ve kişi başına son bekletilen KAYIT.
#[derive(Default)]
struct Basliklar {
    dolu: HashSet<PathBuf>,
    kayit: BTreeMap<String, Row>,
}

/// Satırları gün dosyalarına yazar. Bugünden sonraki ya da okunamayan gün bugüne (`asil_zaman=`); mühürlü günü
/// `ortak::gun_dosyasina_ekle` bugüne çevirir. Kişi dosyası yeni açılıyorsa başına o kişinin KAYIT'ı (satırın zamanıyla)
/// gelir; dosya doluysa KAYIT yazılmaz.
fn yaz(kok: &Path, list: Vec<(String, Row)>, now: f64, b: &mut Basliklar) -> std::io::Result<()> {
    let simdi_iso = ortak::now_iso(now);
    let bugun = ortak::day_of(&simdi_iso).to_string();
    let mut gruplar: BTreeMap<PathBuf, Vec<Row>> = BTreeMap::new();
    for (goreli, mut row) in list {
        let zaman = row.0[0].clone();
        let gun = zaman.get(..10).filter(|g| ortak::gun_adi(g) && *g <= bugun.as_str()).map(str::to_string);
        let gun = gun.unwrap_or_else(|| {
            row = row.clone().ek_ekle(&format!("asil_zaman={zaman}"));
            row.0[0] = simdi_iso.clone();
            bugun.clone()
        });
        let yol = kok.join("gunluk").join(&gun).join(&goreli);
        if goreli.starts_with("kullanicilar/") {
            let dolu = b.dolu.contains(&yol) || fs::metadata(&yol).is_ok_and(|m| m.len() > 0);
            if row.get("olay") == "KAYIT" {
                b.kayit.insert(goreli.clone(), row.clone());
                if dolu {
                    continue;
                }
            } else if !dolu {
                if let Some(k) = b.kayit.get(&goreli) {
                    let mut k = k.clone();
                    k.0[0] = row.0[0].clone();
                    gruplar.entry(yol.clone()).or_default().push(k);
                }
            }
            b.dolu.insert(yol.clone());
        }
        gruplar.entry(yol).or_default().push(row);
    }
    for (yol, rows) in gruplar {
        ortak::gun_dosyasina_ekle(&yol, &rows)?;
    }
    Ok(())
}

/// Bekleme dosyalarını gün dosyalarına yazar: bu açılışınki düzeltilir, öncekiler ham. Saat doğrulanmış olmalı (kilit tutulur).
/// Satırlar 5000'lik parçalarla yazılır (uzun süre internetsiz kalan cihazın dosyası büyük olabilir).
/// ponytail: bir dosya yarıda hata verirse dosya kalır, sonraki denemede baştan yazılır (yazılmış satırlar iki kez).
fn aktar(kok: &Path, acilis: &str, s: Saatler) -> std::io::Result<Rapor> {
    let mut rapor = Rapor::default();
    let mut dosyalar = bekleme_dosyalari(kok)?;
    // önceki açılışlar önce (eskiden yeniye), bu açılış en son
    dosyalar.sort_by_key(|p| (p.file_stem().is_some_and(|x| x == acilis), fs::metadata(p).and_then(|m| m.modified()).ok()));
    for yol in dosyalar {
        let stem = yol.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
        let bu = stem == acilis;
        let ham = fs::read(&yol)?;
        let metin = String::from_utf8_lossy(&ham);
        let satirlar = || metin.lines().filter(|l| !l.trim().is_empty());
        let tabanlar = if bu { farklar(satirlar().filter_map(ayristir).map(|(b, _)| b).collect::<Vec<_>>().iter()) } else { vec![] };
        let etiket = format!("saat=dogrulanamadi acilis={}", stem.get(..8).unwrap_or(&stem));
        let (mut parca, mut basliklar, mut bozuk) = (vec![], Basliklar::default(), String::new());
        for l in satirlar() {
            let Some((b, row)) = ayristir(l) else {
                bozuk.push_str(l);
                bozuk.push('\n');
                continue;
            };
            let row = if bu {
                duzelt(&b, row, &tabanlar, s, &mut rapor)
            } else {
                rapor.onceki += 1;
                row.ek_ekle(&etiket)
            };
            parca.push((b.g, row));
            if parca.len() >= 5000 {
                yaz(kok, std::mem::take(&mut parca), s.gercek, &mut basliklar)?;
            }
        }
        yaz(kok, parca, s.gercek, &mut basliklar)?;
        if !bozuk.is_empty() {
            // yarım satır (elektrik kesintisi): silinmez, incelemeye kalır
            rapor.bozuk += bozuk.lines().count();
            OpenOptions::new().append(true).create(true).mode(0o600).open(yol.with_extension("bozuk"))?.write_all(bozuk.as_bytes())?;
        }
        fs::remove_file(&yol)?;
    }
    Ok(rapor)
}

/// O açılışta (saat doğrulanmadan) açılan oturumların süreleri ve kişi listesindeki tarihler aynı farkla kayar.
fn kayitlari_kaydir(cfg: &Config, r: &Rapor) {
    if r.kaydir.is_empty() && r.kisi.is_empty() {
        return;
    }
    let st = &cfg.main.state_root;
    let Ok(_g) = ortak::state_lock(st) else { return };
    if !r.kaydir.is_empty() {
        let mut ses = ortak::load_sessions(st);
        let mut degisti = false;
        for s in ses.values_mut() {
            if let Some(&(bas, bitis)) = r.kaydir.get(&s.session_id) {
                s.start_epoch += bas;
                s.start = ortak::now_iso(s.start_epoch);
                s.expires_epoch += bitis;
                degisti = true;
            }
        }
        if degisti {
            if let Err(e) = ortak::save_sessions(st, &ses) {
                eprintln!("saat: oturum dosyası yazılamadı: {e}");
            }
        }
    }
    if let Err(e) = ortak::index_zaman_duzelt(&cfg.main.log_root, &r.kisi) {
        eprintln!("saat: kişi listesi yazılamadı: {e}");
    }
}

#[derive(Default, Debug)]
pub struct Ilerleme {
    /// saat bu açılışta doğrulandı (RTC ya da NTP) ve bekleyen kalmadı
    pub dogru: bool,
    /// yapılacak iş kalmadı (RTC açılışında NTP'ye de bakıldı)
    pub bitti: bool,
    pub ileti: Option<String>,
}

fn kisa(acilis: &str) -> &str {
    acilis.get(..8).unwrap_or(acilis)
}

/// Kaydedici her döngüde (ve `hazir`) çağırır: saat şimdi doğrulandıysa bekleyenler düzeltilip yazılır, durum `ntp` olur;
/// RTC'ye güvenilen açılışta NTP gelince fark denetlenir; önceki açılışlardan kalan bekleme dosyaları yazılır.
pub fn ilerle(cfg: &Config, y: &Yollar, s: Saatler) -> Ilerleme {
    let kok = Path::new(&cfg.main.log_root);
    let d = durum(y, s);
    let senkron = senkron_mu(y);
    if d.kaynak == BEKLIYOR && !senkron {
        return Ilerleme::default();
    }
    let rtc_bakilacak = d.kaynak == RTC && !d.ntp_bakildi && senkron;
    let mut sonuc = Ilerleme { dogru: true, bitti: d.kaynak != RTC || d.ntp_bakildi || senkron, ileti: None };
    if d.kaynak != BEKLIYOR && !rtc_bakilacak && !bekleyen_var(kok) {
        return sonuc;
    }
    let Ok(kilit) = ortak::dosya_kilidi(&y.kilit) else { return Ilerleme::default() };
    let d = durum_oku(y).unwrap_or(d);
    let rapor = match aktar(kok, &d.acilis, s) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("saat: bekleyen kayıtlar yazılamadı: {e}");
            return Ilerleme { dogru: false, bitti: false, ileti: None };
        }
    };
    let mut yeni = d.clone();
    let mut olay = None;
    if d.kaynak == BEKLIYOR {
        yeni = Durum { kaynak: NTP.into(), fark: s.fark(), ntp_bakildi: true, ..d.clone() };
        let ek = format!(
            "kaynak=ntp fark={:+.0}sn satir={} onceki={} acilis={}",
            s.fark() - d.fark,
            rapor.duzeltilen,
            rapor.onceki,
            kisa(&d.acilis)
        );
        olay = Some(("SAAT_DOGRULANDI", ek));
    } else if d.kaynak == RTC && senkron && !d.ntp_bakildi {
        yeni.ntp_bakildi = true;
        let fark = s.fark() - d.fark;
        if fark.abs() > SICRAMA_SN {
            olay = Some(("SAAT_SICRAMA", format!("kaynak=rtc fark={fark:+.0}sn acilis={}", kisa(&d.acilis))));
        }
    }
    if yeni != d {
        durum_yaz(y, &yeni);
        if yeni.kaynak != d.kaynak {
            isaret_guncelle(y, &yeni, s);
        }
    }
    drop(kilit);
    dogru_gordum();
    kayitlari_kaydir(cfg, &rapor);
    let mut ileti = vec![];
    if let Some((o, ek)) = olay {
        ileti.push(format!("{o} {ek}"));
        ortak::audit(&cfg.main.log_root, Row::new(o, &ortak::now_iso(s.gercek)).set("ek", ek));
    }
    if rapor.duzeltilen + rapor.onceki + rapor.bozuk > 0 {
        ileti.push(format!("bekleyen kayıtlar yazıldı: {} düzeltildi, {} önceki açılıştan, {} bozuk", rapor.duzeltilen, rapor.onceki, rapor.bozuk));
    }
    sonuc.ileti = (!ileti.is_empty()).then(|| ileti.join("; "));
    sonuc.bitti = yeni.kaynak != RTC || yeni.ntp_bakildi;
    sonuc
}

/// Mühürleme, temizlik ve fabrika dönüşünden önce: saat bu açılışta doğrulanmış ve bekleyen satır kalmamış olmalı.
pub fn hazir(cfg: &Config) -> Result<(), String> {
    let Some(y) = ortam() else { return Ok(()) };
    if !ilerle(cfg, &y, simdi()).dogru {
        return Err("saat bu açılışta henüz doğrulanmadı (NTP eşitlemesi bekleniyor)".into());
    }
    if bekleyen_var(Path::new(&cfg.main.log_root)) {
        return Err("bekleyen kayıtlar gün dosyalarına yazılamadı".into());
    }
    Ok(())
}

/// Gün-kapat saat yüzünden ertelendi: saat doğrulanınca kaydedici gün-kapat'ı çalıştırır.
pub fn gun_kapat_ertele() {
    if let Some(y) = ortam() {
        let _ = ortak::write_atomic_mode(&y.gun_kapat, b"", 0o644);
    }
}

/// Ertelenmiş gün-kapat var mı (varsa işaret silinir).
pub fn gun_kapat_istendi(y: &Yollar) -> bool {
    fs::remove_file(&y.gun_kapat).is_ok()
}

/// `ctl saat`: bu açılışın saat durumu ve bekleyen satırlar.
pub fn ozet(cfg: &Config, y: &Yollar) -> String {
    let d = durum(y, simdi());
    let (satir, dosya) = bekleyen_sayisi(Path::new(&cfg.main.log_root));
    let kaynak = match d.kaynak.as_str() {
        NTP => "doğrulandı (NTP)",
        RTC => "doğrulandı (RTC)",
        _ => "doğrulanmadı, NTP eşitlemesi bekleniyor",
    };
    format!("Saat: {kaynak}\nAçılış: {}\nBekleyen satır: {satir} ({dosya} dosya)", d.acilis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Ortam {
        cfg: Config,
        y: Yollar,
        kok: PathBuf,
        temp: PathBuf,
    }

    impl Drop for Ortam {
        fn drop(&mut self) {
            DENEME.with(|d| *d.borrow_mut() = None);
            let _ = fs::remove_dir_all(&self.temp);
        }
    }

    /// Geçici kök; saat bekliyor (eşitleme yok, RTC yok), açılış "acilis-1".
    fn ortam_kur() -> Ortam {
        static N: AtomicUsize = AtomicUsize::new(0);
        let temp = std::env::temp_dir().join(format!("wfc-saat-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).unwrap();
        fs::write(temp.join("boot_id"), "acilis-1\n").unwrap();
        let y = Yollar {
            durum: temp.join("run/saat.json"),
            kilit: temp.join("run/saat.kilit"),
            isaret: temp.join("lib/saat-dogru"),
            senkron: temp.join("run/synchronized"),
            rtc: temp.join("rtc"),
            taban: vec![temp.join("timesync-clock")],
            acilis: temp.join("boot_id"),
            gun_kapat: temp.join("run/gun-kapat-bekliyor"),
            cekirdek: false,
        };
        let text = format!("[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\n", temp.display());
        let cfg = Config::parse(&text).unwrap();
        DENEME.with(|d| *d.borrow_mut() = Some(y.clone()));
        Ortam { kok: temp.join("5651"), cfg, y, temp }
    }

    fn oku(p: &Path) -> String {
        fs::read_to_string(p).unwrap_or_default()
    }

    fn gun_dosyasi(o: &Ortam, gun: &str, ad: &str) -> PathBuf {
        o.kok.join("gunluk").join(gun).join(ad)
    }

    #[test]
    fn karar_kaynaklari() {
        let o = ortam_kur();
        let s = simdi();
        // işaret yok → bekliyor
        assert_eq!(karar(&o.y, s).kaynak, BEKLIYOR);
        fs::create_dir_all(o.y.isaret.parent().unwrap()).unwrap();
        fs::write(&o.y.isaret, "dogru kurulum 1\n").unwrap();
        // RTC okunamıyor → bekliyor
        assert_eq!(karar(&o.y, s).kaynak, BEKLIYOR);
        // RTC sistem saatiyle aynı → rtc
        fs::write(&o.y.rtc, format!("{}\n", s.gercek as i64)).unwrap();
        assert_eq!(karar(&o.y, s).kaynak, RTC);
        // systemd saati ileri almış (RTC 2014'te) → bekliyor
        fs::write(&o.y.rtc, "1388625600\n").unwrap();
        assert_eq!(karar(&o.y, s).kaynak, BEKLIYOR);
        // RTC kayıtlı alt sınırdan (timesyncd'nin dosyası) geride → bekliyor
        let ileri = Saatler { gercek: s.gercek - 3600.0, acilistan: s.acilistan };
        fs::write(&o.y.rtc, format!("{}\n", ileri.gercek as i64)).unwrap();
        fs::write(&o.y.taban[0], "").unwrap(); // değişiklik zamanı = şimdi > RTC
        assert_eq!(karar(&o.y, ileri).kaynak, BEKLIYOR);
        // eşitleme her şeyin önünde
        fs::create_dir_all(o.y.senkron.parent().unwrap()).unwrap();
        fs::write(&o.y.senkron, "").unwrap();
        assert_eq!(karar(&o.y, ileri).kaynak, NTP);
    }

    #[test]
    fn karar_acilis_basina_bir_kez_ve_isaret() {
        let o = ortam_kur();
        fs::create_dir_all(o.y.isaret.parent().unwrap()).unwrap();
        fs::write(&o.y.isaret, "dogru eski\n").unwrap();
        let d = durum(&o.y, simdi());
        assert_eq!(d.kaynak, BEKLIYOR);
        assert!(!o.y.isaret.exists(), "doğrulanmayan açılış işareti siler");
        // eşitleme sonradan gelse de karar aynı açılışta değişmez (geçişi `ilerle` yapar)
        fs::write(&o.y.senkron, "").unwrap();
        assert_eq!(durum(&o.y, simdi()).kaynak, BEKLIYOR);
        // yeni açılış → yeni karar
        fs::write(&o.y.acilis, "acilis-2\n").unwrap();
        let d2 = durum(&o.y, simdi());
        assert_eq!((d2.kaynak.as_str(), d2.acilis.as_str()), (NTP, "acilis-2"));
        assert!(oku(&o.y.isaret).starts_with("dogru acilis-2 "), "{}", oku(&o.y.isaret));
    }

    #[test]
    fn bekliyorken_satirlar_biriktirilir_ntp_gelince_duzeltilip_dogru_gune_yazilir() {
        let o = ortam_kur();
        let t = ortak::wall();
        let zaman = ortak::now_iso(t);
        ortak::audit(&o.cfg.main.log_root, Row::new("SERVIS_BASLADI", &zaman).set("ek", "portal"));
        let dhcp = Row::new("DHCP_ATAMA", &zaman).set("mac", "aa:bb:cc:dd:ee:01").set("ic_ip", "10.50.0.20");
        ortak::append_rows(&ortak::day_file(&o.cfg.main.log_root, ortak::day_of(&zaman), "dhcp.csv"), &[dhcp]).unwrap();
        // gün dosyası yok, bekleme dosyası var
        assert!(!o.kok.join("gunluk").exists());
        let bekleyen = oku(&o.kok.join("bekleyen/acilis-1.jsonl"));
        assert_eq!(bekleyen.lines().count(), 2, "{bekleyen}");
        assert!(!hazir(&o.cfg).is_ok(), "doğrulanmadan mühürleme yok");
        // NTP eşitledi: saat 30 dk ileri atladı
        fs::create_dir_all(o.y.senkron.parent().unwrap()).unwrap();
        fs::write(&o.y.senkron, "").unwrap();
        let s0 = simdi();
        let s = Saatler { gercek: s0.gercek + 1800.0, acilistan: s0.acilistan };
        let r = ilerle(&o.cfg, &o.y, s);
        assert!(r.dogru && r.bitti, "{r:?}");
        let dogru = ortak::now_iso((crate::kayit::parse_time(&zaman).unwrap() + 1800.0).round());
        let gun = ortak::day_of(&dogru).to_string();
        let dhcp = oku(&gun_dosyasi(&o, &gun, "dhcp.csv"));
        assert!(dhcp.contains(&format!("{dogru};DHCP_ATAMA;")) && dhcp.contains(&format!("ham_zaman={zaman}")), "{dhcp}");
        let denetim = oku(&gun_dosyasi(&o, ortak::day_of(&ortak::now_iso(s.gercek)), "denetim.csv"));
        assert!(denetim.contains(&format!("{dogru};SERVIS_BASLADI;")), "{denetim}");
        assert!(denetim.contains(";SAAT_DOGRULANDI;") && denetim.contains("kaynak=ntp fark=+1800sn satir=2 onceki=0 acilis=acilis-1"), "{denetim}");
        assert!(!o.kok.join("bekleyen/acilis-1.jsonl").exists());
        assert_eq!(durum_oku(&o.y).unwrap().kaynak, NTP);
        assert!(oku(&o.y.isaret).starts_with("dogru acilis-1 "));
        assert!(hazir(&o.cfg).is_ok());
        // doğrulandıktan sonra doğrudan yazılır
        let sonra = Row::new("DHCP_YENILEME", &ortak::now_iso(ortak::wall())).set("mac", "aa:bb:cc:dd:ee:01");
        let yol = ortak::day_file(&o.cfg.main.log_root, ortak::day_of(&ortak::now_iso(ortak::wall())), "dhcp.csv");
        ortak::append_rows(&yol, &[sonra]).unwrap();
        assert!(oku(&yol).contains(";DHCP_YENILEME;"));
    }

    #[test]
    fn sicrama_aninda_kuyruktaki_olay_eski_farkla_duzeltilir() {
        let f0 = 1_790_000_000.0;
        let satir = |b: f64, f: f64, zaman: f64, olay: &str| {
            let r = Row::new(olay, &ortak::now_iso(zaman));
            serde_json::to_string(&Biriken { g: "trafik.csv".into(), b, f, s: r.0.to_vec() }).unwrap() + "\n"
        };
        // 100. sn eski farkla; 101. sn'de 1800 sn sıçrama oldu, 100.5'te zamanlanan olay yeni farkla yazıldı; 102. sn yeni
        let metin = satir(100.0, f0, f0 + 100.0, "A") + &satir(101.0, f0 + 1800.0, f0 + 100.0, "B") + &satir(102.0, f0 + 1800.0, f0 + 1902.0, "C");
        let mut rapor = Rapor::default();
        let s = Saatler { gercek: f0 + 1800.0 + 200.0, acilistan: 200.0 };
        let satirlar: Vec<(Biriken, Row)> = metin.lines().map(|l| ayristir(l).unwrap()).collect();
        let tabanlar = farklar(satirlar.iter().map(|(b, _)| b));
        assert_eq!(tabanlar, vec![f0, f0 + 1800.0]);
        let out: Vec<Row> = satirlar.into_iter().map(|(b, row)| duzelt(&b, row, &tabanlar, s, &mut rapor)).collect();
        let zaman: Vec<String> = out.iter().map(|r| r.0[0].clone()).collect();
        assert_eq!(zaman, vec![ortak::now_iso(f0 + 1900.0), ortak::now_iso(f0 + 1900.0), ortak::now_iso(f0 + 1902.0)]);
        assert!(out[1].get("ek").contains("ham_zaman=") && !out[2].get("ek").contains("ham_zaman="));
        assert_eq!(rapor.duzeltilen, 2);
        // hiçbir farkla tutmayan (çok eski) zaman olduğu gibi kalır
        let row = Row::new("OTURUM_BITIS", &ortak::now_iso(f0 - 86_400.0));
        let eski = Biriken { g: "oturum.csv".into(), b: 300.0, f: f0 + 1800.0, s: row.0.to_vec() };
        let out = duzelt(&eski, row, &farklar(std::iter::once(&eski)), s, &mut rapor);
        assert_eq!(out.0[0], ortak::now_iso(f0 - 86_400.0));
        assert!(out.get("ek").contains("saat=duzeltilmedi"));
        // bozuk satır
        assert!(ayristir("{\"g\":\"dhcp.csv\",\"b\":1").is_none() && ayristir("{\"g\":\"x\",\"b\":1,\"f\":0,\"s\":[\"a\"]}").is_none());
    }

    #[test]
    fn onceki_acilisin_bekleyenleri_ham_yazilir_gelecek_ve_muhurlu_gun_bugune() {
        let o = ortam_kur();
        fs::create_dir_all(o.y.senkron.parent().unwrap()).unwrap();
        fs::write(&o.y.senkron, "").unwrap();
        let satir = |zaman: &str, olay: &str| {
            serde_json::to_string(&Biriken { g: "denetim.csv".into(), b: 1.0, f: 0.0, s: Row::new(olay, zaman).0.to_vec() }).unwrap() + "\n"
        };
        let metin = satir("2026-04-13T22:38:20+03:00", "SERVIS_BASLADI")
            + &satir("2099-01-01T10:00:00+03:00", "GELECEK")
            + &satir("2026-04-12T10:00:00+03:00", "MUHURLU")
            + "{\"g\":\"denetim.csv\",\"b\":1";
        fs::create_dir_all(o.kok.join("bekleyen")).unwrap();
        fs::write(o.kok.join("bekleyen/c21e272be69f4944.jsonl"), metin).unwrap();
        let muhurlu = o.kok.join("gunluk/2026-04-12");
        fs::create_dir_all(&muhurlu).unwrap();
        fs::write(muhurlu.join("MANIFEST.sha256"), "# gun=2026-04-12\n").unwrap();
        let r = ilerle(&o.cfg, &o.y, simdi());
        assert!(r.dogru, "{r:?}");
        let nisan = oku(&gun_dosyasi(&o, "2026-04-13", "denetim.csv"));
        assert!(nisan.contains("2026-04-13T22:38:20+03:00;SERVIS_BASLADI;") && nisan.contains("saat=dogrulanamadi acilis=c21e272b"), "{nisan}");
        let bugun = oku(&gun_dosyasi(&o, ortak::day_of(&ortak::now_iso(ortak::wall())), "denetim.csv"));
        assert!(bugun.contains(";GELECEK;") && bugun.contains("asil_zaman=2099-01-01T10:00:00+03:00"), "{bugun}");
        assert!(bugun.contains(";MUHURLU;") && bugun.contains("asil_zaman=2026-04-12T10:00:00+03:00"), "{bugun}");
        assert!(!muhurlu.join("denetim.csv").exists(), "mühürlü güne dosya açılmaz");
        assert!(oku(&o.kok.join("bekleyen/c21e272be69f4944.bozuk")).starts_with("{\"g\""), "yarım satır incelemeye kalır");
        assert!(!o.kok.join("bekleyen/c21e272be69f4944.jsonl").exists());
    }

    #[test]
    fn kisi_dosyasinda_tek_kayit_basligi_ve_oturum_kaymasi() {
        let o = ortam_kur();
        let t = ortak::wall();
        let zaman = ortak::now_iso(t);
        let s = ortak::Session {
            phone: "905334553132".into(),
            ad: "Ayşe".into(),
            soyad: "Yılmaz".into(),
            ip: "10.50.0.23".into(),
            session_id: "s1".into(),
            start: zaman.clone(),
            start_epoch: t,
            expires_epoch: t + 3600.0,
        };
        let mut ses = ortak::Sessions::new();
        ses.insert("aa:bb:cc:dd:ee:01".into(), s.clone());
        ortak::save_sessions(&o.cfg.main.state_root, &ses).unwrap();
        ortak::upsert_index(&o.cfg.main.log_root, &s.phone, &s.ad, &s.soyad, &zaman).unwrap();
        let basla = Row::new("OTURUM_BASLA", &zaman).set("telefon", &s.phone).set("oturum_id", "s1").set("ek", "bitis=x sozlesme=1");
        let gun = ortak::day_of(&zaman).to_string();
        ortak::append_user_rows(&o.cfg.main.log_root, &s, "aa:bb:cc:dd:ee:01", &[basla.clone()]).unwrap();
        ortak::append_rows(&ortak::day_file(&o.cfg.main.log_root, &gun, "oturum.csv"), &[basla]).unwrap();
        let dns = Row::new("DNS", &zaman).set("telefon", &s.phone).set("oturum_id", "s1").set("alan_adi", "ornek.com.tr");
        ortak::append_user_rows(&o.cfg.main.log_root, &s, "aa:bb:cc:dd:ee:01", &[dns]).unwrap();
        assert_eq!(oku(&o.kok.join("bekleyen/acilis-1.jsonl")).matches("\"KAYIT\"").count(), 1, "KAYIT kişi başına bir kez bekletilir");
        fs::create_dir_all(o.y.senkron.parent().unwrap()).unwrap();
        fs::write(&o.y.senkron, "").unwrap();
        let s0 = simdi();
        assert!(ilerle(&o.cfg, &o.y, Saatler { gercek: s0.gercek + 600.0, acilistan: s0.acilistan }).dogru);
        let dogru = ortak::now_iso((crate::kayit::parse_time(&zaman).unwrap() + 600.0).round());
        let kisi = oku(&o.kok.join("gunluk").join(ortak::day_of(&dogru)).join("kullanicilar/905334553132.csv"));
        assert_eq!(kisi.matches(";KAYIT;").count(), 1, "{kisi}");
        let satirlar: Vec<&str> = kisi.lines().skip(1).collect();
        assert!(satirlar[0].contains(";KAYIT;") && satirlar[1].contains(";OTURUM_BASLA;") && satirlar[2].contains(";DNS;"), "{kisi}");
        let ses = ortak::load_sessions(&o.cfg.main.state_root);
        let s2 = &ses["aa:bb:cc:dd:ee:01"];
        assert!((s2.start_epoch - (t + 600.0)).abs() < 0.01 && (s2.expires_epoch - (t + 4200.0)).abs() < 0.01, "{s2:?}");
        assert!((crate::kayit::parse_time(&s2.start).unwrap() - (t + 600.0)).abs() <= 1.0, "{}", s2.start);
        let idx = ortak::read_index(&o.cfg.main.log_root);
        assert_eq!((idx["905334553132"][3].as_str(), idx["905334553132"][4].as_str()), (dogru.as_str(), dogru.as_str()));
    }

    #[test]
    fn kisi_dosyasi_gun_degisince_kayit_basligi_yeni_dosyaya_da_konur() {
        let o = ortam_kur();
        let g = "kullanicilar/905334553132.csv".to_string();
        let kayit = Row::new("KAYIT", "2026-09-27T23:59:59+03:00").set("telefon", "905334553132").set("ad", "Ayşe");
        let dns = |z: &str| Row::new("DNS", z).set("telefon", "905334553132");
        let list = vec![
            (g.clone(), kayit),
            (g.clone(), dns("2026-09-27T23:59:59+03:00")),
            (g.clone(), dns("2026-09-28T00:00:01+03:00")),
        ];
        yaz(&o.kok, list, ortak::wall(), &mut Basliklar::default()).unwrap();
        let d1 = oku(&gun_dosyasi(&o, "2026-09-27", &g));
        let d2 = oku(&gun_dosyasi(&o, "2026-09-28", &g));
        assert!(d1.contains("2026-09-27T23:59:59+03:00;KAYIT;905334553132;Ayşe") && d1.matches(";KAYIT;").count() == 1, "{d1}");
        let s2: Vec<&str> = d2.lines().skip(1).collect();
        assert!(s2.len() == 2 && s2[0].starts_with("2026-09-28T00:00:01+03:00;KAYIT;905334553132;Ayşe") && s2[1].contains(";DNS;"), "{d2}");
        // dosya zaten doluysa KAYIT yazılmaz
        let list = vec![(g.clone(), Row::new("KAYIT", "2026-09-28T10:00:00+03:00")), (g.clone(), dns("2026-09-28T10:00:00+03:00"))];
        yaz(&o.kok, list, ortak::wall(), &mut Basliklar::default()).unwrap();
        assert_eq!(oku(&gun_dosyasi(&o, "2026-09-28", &g)).matches(";KAYIT;").count(), 1);
    }

    #[test]
    fn rtc_acilisinda_ntp_buyuk_fark_bulursa_sicrama_yazilir() {
        let o = ortam_kur();
        fs::create_dir_all(o.y.isaret.parent().unwrap()).unwrap();
        fs::write(&o.y.isaret, "dogru eski\n").unwrap();
        let s = simdi();
        fs::write(&o.y.rtc, format!("{}\n", s.gercek as i64)).unwrap();
        assert_eq!(durum(&o.y, s).kaynak, RTC);
        // RTC açılışı doğrudan yazar
        let yol = ortak::day_file(&o.cfg.main.log_root, ortak::day_of(&ortak::now_iso(s.gercek)), "denetim.csv");
        ortak::audit(&o.cfg.main.log_root, Row::new("SERVIS_BASLADI", &ortak::now_iso(s.gercek)));
        assert!(oku(&yol).contains(";SERVIS_BASLADI;"));
        // NTP gelmeden iş bitmez; gelince 1 saat fark → SAAT_SICRAMA, bir kez
        let r = ilerle(&o.cfg, &o.y, s);
        assert!(r.dogru && !r.bitti);
        fs::write(&o.y.senkron, "").unwrap();
        let sonra = Saatler { gercek: s.gercek + 3600.0, acilistan: s.acilistan };
        assert!(ilerle(&o.cfg, &o.y, sonra).bitti);
        assert!(ilerle(&o.cfg, &o.y, sonra).bitti);
        let gun = ortak::day_of(&ortak::now_iso(sonra.gercek)).to_string();
        let ikinci = gun_dosyasi(&o, &gun, "denetim.csv");
        let denetim = oku(&yol) + &if ikinci == yol { String::new() } else { oku(&ikinci) };
        assert_eq!(denetim.matches(";SAAT_SICRAMA;").count(), 1, "{denetim}");
        assert!(denetim.contains("kaynak=rtc fark=+3600sn acilis=acilis-1"));
    }

    #[test]
    fn gun_kapat_ve_temizlik_saat_dogrulanmadan_calismaz() {
        let o = ortam_kur();
        let gun = "2026-09-27";
        let d = o.kok.join("gunluk").join(gun);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("dhcp.csv"), "x\r\n").unwrap();
        let out = crate::muhur::gun_kapat(&o.cfg, None, 1_790_705_134.0, true);
        assert!(out.len() == 1 && out[0].starts_with("saat doğrulanmadı"), "{out:?}");
        assert!(!d.join("MANIFEST.sha256").exists() && o.y.gun_kapat.exists());
        let out = crate::muhur::prune(&o.cfg, 1_790_705_134.0, false);
        assert!(out.len() == 1 && out[0].starts_with("saat doğrulanmadı"), "{out:?}");
        // doğrulanınca kaydedici ertelenen gün-kapat'ı çalıştırır
        fs::create_dir_all(o.y.senkron.parent().unwrap()).unwrap();
        fs::write(&o.y.senkron, "").unwrap();
        assert!(ilerle(&o.cfg, &o.y, simdi()).dogru);
        assert!(gun_kapat_istendi(&o.y) && !gun_kapat_istendi(&o.y));
        let out = crate::muhur::gun_kapat(&o.cfg, None, 1_790_705_134.0, true);
        assert_eq!(out, vec![format!("{gun}: kapatıldı")]);
    }
}
