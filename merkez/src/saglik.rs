//! wc-saglik (Python saglik.py; wcpanel, 5 dk'da bir; spec 2026-10-11): cihaz sağlık kuralları → sorun kaydı →
//! hizmet sağlayıcıya SMS. El sıkışma ve son yedek günü root'un yazdığı durum.json'dan, son rapor veritabanından okunur.
//! SMS kuralı: kritik/uyarı sorun 600 sn açık kalınca tek SMS; kapanıp 600 sn içinde dönen sorun (bildirilmiş olsun
//! olmasın) aynı olaydır; bildirilmişse düzelince 600 sn temiz kalırsa tek "düzeldi" SMS'i; bilgi önemliler 09:00'da
//! toplu; günlük üst sınır dolunca ilk bastırılan uyarıda tek "üst sınır" SMS'i; sınıra takılan açılış (sinir:<gün>)
//! ertesi gün yeniden denenir, "düzeldi"si gitmez; izleme çalışamazsa günde bir "izleme çalışmıyor" SMS'i. Ağ hatasında
//! yeniden deneme yok. Müşteriye bağlı olmayan merkez sorunları (hazır engel listeleri güncellenemiyor) MERKEZ (0)
//! numarasıyla aynı sorun tablosuna ve aynı SMS kurallarına girer. Her tamamlanan turun zamanı `saglik_son_tur`
//! ayarına yazılır (yönetim ana sayfası izlemenin durduğunu oradan görür).
//!
//! Yönetim (Python `yonetim.py`) tarafından da kullanılan eşikler (`CEVRIMICI_SN`, `TEKRAR_SN`, `GECIKME_GUN`,
//! `yedek_gecikti`) burada tanımlıdır: yönetim bunları buradan alır (saglik ← yonetim bağımlılığı olmasın).
//!
//! Python'dan farklar: istisna türü adı (SMS metninde `hata: RuntimeError` gibi) `CalHata::tur`'dur; gönderici
//! `netgsm::gonder` + `Tasiyici` (testte sahte, ağ yok).
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, TimeZone};
use serde_json::Value;

use crate::detay::gun_coz;
use crate::ortak::{self, py_dogru, py_strip, Tasiyici, TZ};
use crate::raporlar::epoch;
use crate::veri::{Cihaz, Hata, Musteri, Saat, Sorun, Veri};

/// Yönetim listesindeki "çevrimiçi" eşiği (sn); `ulasilamiyor` aynı eşiği kullanır.
pub const CEVRIMICI_SN: f64 = 600.0;
pub const ULASILAMIYOR_SN: f64 = CEVRIMICI_SN;
/// Son yedek günü bu kadar günden eskiyse "Yedek gecikti" (TR takvimi).
pub const GECIKME_GUN: i64 = 2;
pub const RAPOR_SN: f64 = 900.0;
pub const SMS_BEKLE_SN: f64 = 600.0;
/// Kapanan sorun bu kadar temiz kalmadan "düzeldi" denmez; içinde dönerse aynı olay (titreme).
pub const TEKRAR_SN: f64 = 600.0;
pub const DISK_ESIK: i64 = 90;
pub const LISANS_GUN: i64 = 15;
/// wc-durum 5 dk'da bir yazar; daha eskiyse el sıkışmalar bilinmez.
pub const DURUM_BAYAT_SN: f64 = 900.0;
pub const BILGI_SAAT: f64 = 9.0;
/// 09:00–09:10: timer kayması bir turu atlatsa da bilgi SMS'i o gün gider.
pub const BILGI_PENCERE_SN: f64 = 600.0;
pub const GONDERILDI: [&str; 2] = ["gonderildi", "mock"];
/// Toplu bilgi SMS'inde en çok bu kadar sorun yazılır, kalanı "+k diğer".
pub const BILGI_EN_COK: usize = 5;
/// Cihaza ulaşılamazken bilinmez (düzeldi sayılmaz).
pub const ULASILAMIYORKEN_BILINMEZ: [&str; 3] = ["rapor_yok", "kayit_durdu", "servis_durdu"];
/// Kaydedici ayrı: `kayit_durdu`.
pub const SERVIS_ADLARI: [&str; 4] = ["portal", "panel", "dnsmasq", "guvenlik"];
/// Merkezdeki son doğrulama SMS'i sonucu bu kadar süre (sn) geçerli.
pub const SMS_SON_SN: f64 = 3600.0;
/// Misafirden kaynaklanan sağlayıcı/merkez kodları (yanlış/yabancı numara, aynı numaraya çok deneme): SMS çalışıyor sayılır.
pub const MISAFIR_KODLARI: [&str; 7] = ["YABANCI", "50", "60200", "21211", "60205", "60203", "80"];
/// Müşteriye bağlı olmayan sorunların "müşteri" numarası (gerçek numaralar 7 hanelidir).
pub const MERKEZ: i64 = 0;
/// Hazır engel listesinin son başarılı üretiminden bu kadar geçtiyse sorun.
pub const LISTE_BAYAT_SN: f64 = 48.0 * 3600.0;

fn izleme_metni(neden: &str) -> String {
    format!("WifiCorrect: izleme çalışmıyor ({neden})")
}

fn sinir_metni(sinir: i64) -> String {
    format!("WifiCorrect: günlük SMS üst sınırına ({sinir}) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek.")
}

/// Python istisnası: `tur` = `type(h).__name__` (SMS metnine girer), `ileti` stderr için.
#[derive(Debug, Clone, PartialEq)]
pub struct CalHata {
    pub tur: String,
    pub ileti: String,
}

impl CalHata {
    pub fn yeni(tur: &str, ileti: &str) -> CalHata {
        CalHata { tur: tur.into(), ileti: ileti.into() }
    }
}

impl From<Hata> for CalHata {
    fn from(h: Hata) -> CalHata {
        let tur = match &h {
            Hata::Db(_) => "OperationalError",
            Hata::Deger(_) => "ValueError",
            Hata::BaskaCihaz => "BaskaCihaz",
        };
        CalHata { tur: tur.into(), ileti: h.to_string() }
    }
}

type Sonuc<T> = Result<T, CalHata>;

/// Bir kuralın çıktısı: (tür, metin, önem).
pub type SorunSatiri = (String, String, String);

fn bugun(simdi: f64) -> NaiveDate {
    TZ.timestamp_opt(simdi.floor() as i64, 0).single().map(|t| t.date_naive()).unwrap_or_default()
}

/// Python `x or varsayılan` (0.0 da yanlıştır).
fn veya(x: Option<f64>, varsayilan: f64) -> f64 {
    match x {
        Some(v) if v != 0.0 => v,
        _ => varsayilan,
    }
}

fn epoch_deger(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_str).and_then(epoch)
}

/// Yedek gecikti: son gelen gün yok ya da GECIKME_GUN günden eski (TR takvimi). Liste (yönetim) ve uyarı ortak.
pub fn yedek_gecikti(son_gun: Option<&str>, simdi: f64) -> bool {
    let sinir = (bugun(simdi) - chrono::Duration::days(GECIKME_GUN)).format("%Y-%m-%d").to_string();
    match son_gun {
        None | Some("") => true,
        Some(g) => g < sinir.as_str(),
    }
}

fn bekliyor(deger: &str, gun: &str) -> bool {
    deger.is_empty() || (deger.starts_with("sinir") && deger != format!("sinir:{gun}"))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kapsam {
    /// Lisansı bitmiş: yalnızca lisans bilgisi.
    Lisans,
    /// Bütün kurallar.
    Tam,
}

/// None: uyarı yok (cihaz yok/serbest bırakılıyor, askıda, üyelik bitti).
pub fn kapsam(m: &Musteri, c: Option<&Cihaz>, simdi: f64) -> Option<Kapsam> {
    let c = c?;
    if c.durum != "bagli" || m.askida != 0 || m.uyelik != "aktif" {
        return None;
    }
    if !m.lisans_bitis.is_empty() && m.lisans_bitis < bugun(simdi).format("%Y-%m-%d").to_string() {
        return Some(Kapsam::Lisans);
    }
    Some(Kapsam::Tam)
}

/// Merkezin bu müşteri için son doğrulama SMS'i son 60 dk içinde misafirden kaynaklanmayan bir hatayla mı bitti.
fn sms_hatali(sms_son: Option<&crate::veri::SmsSon>, simdi: f64) -> bool {
    let Some(s) = sms_son else { return false };
    if s.sonuc != "hata" || MISAFIR_KODLARI.contains(&s.kod.as_str()) {
        return false;
    }
    epoch(&s.zaman).is_some_and(|z| simdi - z <= SMS_SON_SN)
}

fn satir(tur: &str, metin: impl Into<String>, onem: &str) -> SorunSatiri {
    (tur.into(), metin.into(), onem.into())
}

/// Saf kurallar. Dönen: [(tür, metin, önem)]. `arsiv_var=false`: durum.json'da cihazın kaydı yok (yeni bağlandı, arşivi
/// henüz yok): tünel bilinmez → ulaşılamıyor/rapor yok açılmaz; yedek gecikmesi bağlanma gününden sayılır.
/// `hedef`: cihaza kurdurulması istenen paket sürümü; o sürümün denemesi başarısızsa uyarı.
/// `sms_son`: merkezin bu müşteri için son doğrulama SMS'i sonucu; misafirden kaynaklanmayan bir hatayla bittiyse
/// (SINIR dahil) "SMS çalışmıyor"; son gönderim başarılıysa ya da 60 dk'dan eskiyse kendiliğinden kapanır.
#[allow(clippy::too_many_arguments)]
pub fn sorunlar(
    m: &Musteri, c: Option<&Cihaz>, rapor: Option<&Value>, rapor_zaman: Option<f64>, el_sikisma: Option<f64>,
    son_gun: Option<&str>, simdi: f64, arsiv_var: bool, hedef: Option<&str>, sms_son: Option<&crate::veri::SmsSon>,
) -> Vec<SorunSatiri> {
    let Some(k) = kapsam(m, c, simdi) else { return Vec::new() };
    let mut out = Vec::new();
    if k == Kapsam::Tam {
        let rapor = rapor.filter(|r| py_dogru(r));
        let ulasilamiyor = arsiv_var && el_sikisma.is_none_or(|e| simdi - e > ULASILAMIYOR_SN);
        if ulasilamiyor {
            out.push(satir("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik"));
        } else if arsiv_var && rapor_zaman.is_some_and(|z| simdi - z > RAPOR_SN) {
            // hiç rapor yoksa (eski sürüm) açılmaz
            out.push(satir("rapor_yok", "Rapor gelmiyor", "kritik"));
        }
        if let (Some(r), false) = (rapor, ulasilamiyor) {
            let sv = r.get("servisler").filter(|s| py_dogru(s));
            let acik = |ad: &str| sv.and_then(|s| s.get(ad)).is_some_and(py_dogru);
            if !acik("kaydedici") || r.get("kayit_boslugu_1s").is_some_and(py_dogru) {
                out.push(satir("kayit_durdu", "5651 kaydı durdu", "kritik"));
            }
            let durdu: Vec<&str> = SERVIS_ADLARI.iter().copied().filter(|a| !acik(a)).collect();
            if !durdu.is_empty() {
                out.push(satir("servis_durdu", format!("Servis durdu: {}", durdu.join(", ")), "kritik"));
            }
        }
        let yedek_gunu: String = if arsiv_var {
            son_gun.unwrap_or("").to_string()
        } else {
            c.map(|c| c.baglanma.chars().take(10).collect()).unwrap_or_default()
        };
        if yedek_gecikti(Some(&yedek_gunu), simdi) {
            out.push(satir("yedek_gecikti", "Yedek gecikti", "uyari"));
        }
        if let Some(d) = rapor.and_then(|r| r.get("disk_yuzde")).filter(|d| d.is_i64() || d.is_u64()).and_then(Value::as_i64) {
            if d >= DISK_ESIK {
                out.push(satir("disk_dolu", format!("Disk dolu (%{d})"), "uyari"));
            }
        }
        if rapor.and_then(|r| r.get("sms_sorunu")).is_some_and(py_dogru) || sms_hatali(sms_son, simdi) {
            out.push(satir("sms_calismiyor", "SMS çalışmıyor", "uyari"));
        }
        let g = rapor.and_then(|r| r.get("guncelleme")).and_then(Value::as_object);
        if let (Some(hedef), Some(g), Some(r)) = (hedef.filter(|h| !h.is_empty()), g, rapor) {
            let sonuc = g.get("sonuc").and_then(Value::as_str);
            if matches!(sonuc, Some("geri_alindi") | Some("hata"))
                && g.get("surum").and_then(Value::as_str) == Some(hedef)
                && r.get("paket").and_then(Value::as_str) != Some(hedef)
            {
                out.push(satir("guncelleme_basarisiz", format!("Güncelleme başarısız ({hedef})"), "uyari"));
            }
        }
    }
    if let Some(bitis) = gun_coz(&m.lisans_bitis) {
        let kalan = (bitis - bugun(simdi)).num_days();
        let tarih = ortak::tarih_goster(&format!("{:04}-{:02}-{:02}", bitis.year(), bitis.month(), bitis.day()), false);
        if kalan < 0 {
            out.push(satir("lisans_bitti", format!("Lisans bitti ({tarih})"), "bilgi"));
        } else if kalan <= LISANS_GUN {
            out.push(satir("lisans_bitiyor", format!("Lisans bitiyor ({tarih})"), "bilgi"));
        }
    }
    out
}

/// Hazır engel listesi sorunu metni ya da None. Kategori düzeyinde hata (bugün hiçbir kaynak inmedi) ya da son başarılı
/// üretim LISTE_BAYAT_SN'den eski. Tek kaynağın hatası (diğeri indi) sorun değildir; hiç üretilmemiş ya da okunamayan
/// meta yok sayılır (yeni kurulum; yönetim sayfası gösterir).
pub fn liste_sorunu(dizin: &Path, simdi: f64) -> Option<String> {
    let mut kotu = Vec::new();
    for (k, ad) in crate::listeler::KATEGORILER {
        let Some(m) = crate::listeler::meta_oku(dizin, k) else { continue };
        let z = epoch_deger(m.get("zaman"));
        if m.get("hata").is_some_and(py_dogru) || z.is_none_or(|z| simdi - z > LISTE_BAYAT_SN) {
            kotu.push(ad);
        }
    }
    if kotu.is_empty() {
        None
    } else {
        Some(format!("Engel listeleri güncellenemiyor: {}", kotu.join(", ")))
    }
}

fn kim(m: Option<&Musteri>, c: Option<&Cihaz>) -> String {
    let Some(m) = m else { return "merkez".into() };
    let ad = c.map(|c| py_strip(&c.isletme_adi)).unwrap_or("");
    if ad.is_empty() {
        m.numara.to_string()
    } else {
        format!("{} {ad}", m.numara)
    }
}

/// SMS kararının gönderim kısmı: uyarı kapalıysa hiçbir şey yapmaz; günlük sınırı sayar; her SMS bir hareket.
pub struct Uyari<'a> {
    veri: &'a Veri,
    t: &'a dyn Tasiyici,
    simdi: f64,
    ayar: Value,
    acik: bool,
    gun: String,
    sms: Value,
}

impl<'a> Uyari<'a> {
    pub fn yeni(veri: &'a Veri, t: &'a dyn Tasiyici, simdi: f64) -> Sonuc<Uyari<'a>> {
        let ayar = veri.uyari_ayari()?;
        let acik = ayar.get("acik").is_some_and(py_dogru) && ayar.get("telefonlar").is_some_and(py_dogru);
        let sms = if acik { veri.sms_ayari()?.unwrap_or(Value::Null) } else { Value::Null };
        Ok(Uyari { veri, t, simdi, ayar, acik, gun: bugun(simdi).format("%Y-%m-%d").to_string(), sms })
    }

    fn yolla(&self, metin: &str) -> String {
        let telefonlar: Vec<String> = self
            .ayar
            .get("telefonlar")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(ortak::py_str).collect())
            .unwrap_or_default();
        crate::netgsm::gonder(&self.sms, metin, &telefonlar, self.t)
    }

    /// Dönen: gonderildi | mock | hata:<kod> | sinir:<gün>. Son hak üst sınır bildiriminin; sayaç başka yoldan (deneme
    /// SMS'i) dolduysa bildirim o günün ilk bastırılan uyarısında yine bir kez gider (sayaca girer).
    fn gonder(&self, metin: &str) -> Sonuc<String> {
        let sinir = self.ayar.get("gunluk_sinir").and_then(Value::as_i64).unwrap_or(20);
        if self.veri.sms_sayaci(&self.gun)? < sinir - 1 {
            self.veri.sms_say(&self.gun)?;
            return Ok(self.yolla(metin));
        }
        if self.veri.gunluk_bayrak("sinir_sms", &self.gun)? {
            self.veri.sms_say(&self.gun)?;
            let r = self.yolla(&sinir_metni(sinir));
            self.veri.hareket("saglik", "-", "UYARI_SMS", None, &format!("üst sınır: {r}"))?;
        }
        Ok(format!("sinir:{}", self.gun))
    }

    fn kaydet(&self, s: &Sorun, alan: &str, r: &str, ne: &str) -> Sonuc<()> {
        self.veri.sorun_sms(s.id, alan, r)?;
        if !r.starts_with("sinir") {
            self.veri.hareket("saglik", "-", "UYARI_SMS", Some(s.musteri), &format!("{} {ne}: {r}", s.tur))?;
        }
        Ok(())
    }

    pub fn acilis(&self, m: Option<&Musteri>, c: Option<&Cihaz>, s: &Sorun) -> Sonuc<()> {
        if !self.acik {
            return Ok(());
        }
        let z = epoch(&s.acilis).ok_or_else(|| CalHata::yeni("ValueError", &format!("Invalid isoformat string: {:?}", s.acilis)))?;
        let saat = TZ.timestamp_opt(z.floor() as i64, 0).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default();
        let r = self.gonder(&format!("WifiCorrect {}: {} ({saat}'ten beri)", kim(m, c), s.metin))?;
        self.kaydet(s, "acilis_sms", &r, "açılış")
    }

    /// Bekletilmiş "düzeldi" SMS'i; süre açılıştan son kapanışa. Uyarı kapalıysa "yok" (sonradan gitmez). Sınıra
    /// takılırsa ertesi gün denenmez (geç "düzeldi" anlamsız).
    pub fn kapanis(&self, m: Option<&Musteri>, c: Option<&Cihaz>, s: &Sorun) -> Sonuc<()> {
        if !self.acik {
            self.veri.sorun_sms(s.id, "kapanis_sms", "yok")?;
            return Ok(());
        }
        let fark = veya(epoch(&s.kapanis), self.simdi) - veya(epoch(&s.acilis), self.simdi);
        let dk = ((fark / 60.0).round_ties_even() as i64).max(1);
        let r = self.gonder(&format!("WifiCorrect {}: {} düzeldi ({dk} dk sürdü)", kim(m, c), s.metin))?;
        self.kaydet(s, "kapanis_sms", &r, "kapanış")
    }

    /// İzleme çalışamadı: neden türü (kod) başına günde bir SMS; biri ötekini yutmaz. Aynı gönderici; açık/kapalı ve
    /// günlük sınır geçerli.
    pub fn izleme_bozuk(&self, kod: &str, neden: &str) -> Sonuc<()> {
        eprintln!("wc-saglik: izleme çalışmıyor: {neden}");
        if self.acik && self.veri.gunluk_bayrak(&format!("izleme_sms:{kod}"), &self.gun)? {
            let r = self.gonder(&izleme_metni(neden))?;
            self.veri.hareket("saglik", "-", "UYARI_SMS", None, &format!("izleme çalışmıyor ({neden}): {r}"))?;
        }
        Ok(())
    }

    /// TR 09:00 turunda bildirilmemiş (ya da önceki gün sınıra takılmış) bütün bilgi sorunları tek SMS'te; en çok
    /// BILGI_EN_COK tanesi yazılır, kalanı "+k diğer".
    pub fn bilgi_toplu(&self, musteriler: &HashMap<i64, &Musteri>, cihazlar: &HashMap<i64, &Cihaz>) -> Sonuc<()> {
        // 09:00'dan bu yana geçen saniye (TR sabit +03:00; Python `(t - dokuz).total_seconds()`)
        let gecen = (self.simdi + 3.0 * 3600.0).rem_euclid(86400.0) - BILGI_SAAT * 3600.0;
        if !self.acik || !(0.0..BILGI_PENCERE_SN).contains(&gecen) {
            return Ok(());
        }
        let bekleyen: Vec<Sorun> = self
            .veri
            .acik_sorunlar(None)?
            .into_iter()
            .filter(|s| s.onem == "bilgi" && bekliyor(&s.acilis_sms, &self.gun) && musteriler.contains_key(&s.musteri))
            .collect();
        if bekleyen.is_empty() {
            return Ok(());
        }
        let mut parca: Vec<String> = bekleyen
            .iter()
            .take(BILGI_EN_COK)
            .map(|s| format!("{}: {}", kim(musteriler.get(&s.musteri).copied(), cihazlar.get(&s.musteri).copied()), s.metin))
            .collect();
        if bekleyen.len() > BILGI_EN_COK {
            parca.push(format!("+{} diğer", bekleyen.len() - BILGI_EN_COK));
        }
        let r = self.gonder(&format!("WifiCorrect bilgi: {}", parca.join("; ")))?;
        for s in &bekleyen {
            self.kaydet(s, "acilis_sms", &r, "bilgi")?;
        }
        Ok(())
    }
}

/// `yeni`: bu turda geçerli sorunlar → aç/kapat/yeniden aç, SMS kararları. `m` None: merkez.
#[allow(clippy::too_many_arguments)]
fn sorunlari_isle(
    veri: &Veri, u: &Uyari, n: i64, m: Option<&Musteri>, c: Option<&Cihaz>, yeni: &[SorunSatiri], tam: bool, simdi: f64,
) -> Sonuc<()> {
    let yeni_var = |tur: &str| yeni.iter().any(|y| y.0 == tur);
    let mut acik: Vec<Sorun> = veri.acik_sorunlar(Some(n))?;
    for s in &acik {
        if yeni_var(&s.tur) || (yeni_var("ulasilamiyor") && ULASILAMIYORKEN_BILINMEZ.contains(&s.tur.as_str())) {
            continue; // bilinmiyor: düzeldi sayılmaz
        }
        veri.sorun_kapat(s.id)?; // bildirilmişse "düzeldi" TEKRAR_SN temiz kalınca gider (aşağıda)
        if !tam && s.onem != "bilgi" && GONDERILDI.contains(&s.acilis_sms.as_str()) {
            veri.sorun_sms(s.id, "kapanis_sms", "yok")?; // askıya alındı/serbest: sessiz kapanış
        }
    }
    for (tur, metin, onem) in yeni {
        if let Some(a) = acik.iter_mut().find(|s| s.tur == *tur) {
            if a.metin != *metin {
                // ör. disk yüzdesi değişti: metin güncellenir, SMS yok
                veri.sorun_metin(a.id, metin)?;
                a.metin = metin.clone();
            }
            continue;
        }
        let once = veri.son_kapanan(n, tur)?;
        match once {
            Some(o) if onem != "bilgi" && o.kapanis_sms.is_empty() && simdi - epoch(&o.kapanis).unwrap_or(0.0) < TEKRAR_SN => {
                // TEKRAR_SN temiz kalmadan döndü: bildirilmiş olsun olmasın aynı olay (ilk açılış korunur); titreyen
                // bağlantı böylece 600 sn açık yaşa ulaşıp tek SMS atar, bildirilmişse yeni SMS yok
                veri.sorun_yeniden_ac(o.id)?;
                if o.metin != *metin {
                    veri.sorun_metin(o.id, metin)?;
                }
                acik.push(Sorun { kapanis: String::new(), metin: metin.clone(), ..o });
            }
            _ => {
                veri.sorun_ac(n, tur, metin, onem)?;
            }
        }
    }
    for s in veri.kapanis_bekleyenler(n)? {
        if !tam {
            veri.sorun_sms(s.id, "kapanis_sms", "yok")?;
        } else if simdi - veya(epoch(&s.kapanis), simdi) >= TEKRAR_SN {
            u.kapanis(m, c, &s)?;
        }
    }
    for s in &acik {
        if yeni_var(&s.tur)
            && s.onem != "bilgi"
            && bekliyor(&s.acilis_sms, &u.gun)
            && simdi - veya(epoch(&s.acilis), simdi) >= SMS_BEKLE_SN
        {
            u.acilis(m, c, s)?;
        }
    }
    Ok(())
}

/// Bir müşterinin durum.json kaydı (el sıkışması durum.json zamanına göre kaydırılmış).
#[derive(Clone, Debug, Default)]
struct DurumKaydi {
    el_sikisma: Option<f64>,
    son_gun: Option<String>,
}

fn musteri_turu(veri: &Veri, u: &Uyari, m: &Musteri, c: Option<&Cihaz>, d: Option<&DurumKaydi>, simdi: f64) -> Sonuc<()> {
    let n = m.numara;
    let (mut rapor, mut rz) = (None, None);
    if let Some(c) = c {
        if let Some(s) = veri.saglik(c.id)? {
            rz = epoch(&s.zaman);
            rapor = Some(s.rapor);
        }
    }
    let hedef = veri.cihaz_hedefi(m)?;
    let sms_son = veri.sms_son(n)?;
    let yeni = sorunlar(
        m,
        c,
        rapor.as_ref(),
        rz,
        d.and_then(|d| d.el_sikisma),
        d.and_then(|d| d.son_gun.as_deref()),
        simdi,
        d.is_some(),
        hedef.as_ref().map(|h| h.surum.as_str()),
        sms_son.as_ref(),
    );
    sorunlari_isle(veri, u, n, Some(m), c, &yeni, kapsam(m, c, simdi) == Some(Kapsam::Tam), simdi)
}

fn merkez_turu(veri: &Veri, u: &Uyari, simdi: f64, liste_dizini: &Path) -> Sonuc<()> {
    let yeni: Vec<SorunSatiri> = liste_sorunu(liste_dizini, simdi).map(|metin| satir("liste_guncellenemiyor", metin, "uyari")).into_iter().collect();
    sorunlari_isle(veri, u, MERKEZ, None, None, &yeni, true, simdi)
}

fn sayi(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(*b as i64 as f64), // Python: bool, int'in alt sınıfı
        _ => None,
    }
}

/// Bir tur: durum.json içeriği `durum` (okunamadıysa boş nesne), `simdi` epoch.
pub fn calistir(veri: &Veri, durum: &Value, t: &dyn Tasiyici, simdi: f64, liste_dizini: &Path) -> Sonuc<()> {
    let u = Uyari::yeni(veri, t, simdi)?;
    // cihazlardan (durum.json'dan) bağımsız: izleme bozuk olsa da değerlendirilir
    if let Err(h) = merkez_turu(veri, &u, simdi, liste_dizini) {
        u.izleme_bozuk("merkez_hatasi", &format!("merkez denetimi: {}", h.tur))?;
    }
    let zaman = if durum.is_object() { epoch_deger(durum.get("zaman")) } else { None };
    // wc-durum çalışmıyorsa her cihaz "ulaşılamıyor" görünürdü: hiçbir sorun açılıp kapanmaz, günde bir SMS
    let Some(zaman) = zaman else {
        return u.izleme_bozuk("durum_yok", "durum.json yok");
    };
    if simdi - zaman > DURUM_BAYAT_SN {
        return u.izleme_bozuk("durum_bayat", "durum.json bayat");
    }
    let arsivler = durum.get("arsivler").and_then(Value::as_object);
    let cihaz_listesi = veri.cihazlar()?;
    let cihazlar: HashMap<i64, &Cihaz> = cihaz_listesi.iter().map(|c| (c.musteri, c)).collect();
    let musteri_listesi = veri.musteriler()?;
    let musteriler: HashMap<i64, &Musteri> = musteri_listesi.iter().map(|m| (m.numara, m)).collect();
    // müşteri → durum.json kaydı (sözlük değilse yok)
    let kayit: HashMap<i64, Option<&Value>> = musteri_listesi
        .iter()
        .map(|m| (m.numara, arsivler.and_then(|a| a.get(&m.numara.to_string())).filter(|v| v.is_object())))
        .collect();
    // tünel bilgisi okunamadıysa (`wg show` başarısız) her cihaz "ulaşılamıyor" görünürdü: değerlendirme yok, SMS bütçesi
    // yakılmaz. wg_ok yoksa (eski wc-durum): kaydı olan izlenen cihazların hiçbirinde el sıkışma yoksa okunamadı sayılır.
    // Kaydı olmayan (yeni bağlanmış, arşivi henüz yok) cihazın tüneli bilinmez: hesaba girmez
    let izlenen: Vec<i64> = musteri_listesi
        .iter()
        .filter(|m| kapsam(m, cihazlar.get(&m.numara).copied(), simdi) == Some(Kapsam::Tam) && kayit[&m.numara].is_some())
        .map(|m| m.numara)
        .collect();
    let wg_ok = durum.get("wg_ok");
    let wg_yok = matches!(wg_ok, Some(Value::Bool(false)));
    let wg_bilinmiyor = wg_ok.is_none_or(Value::is_null);
    if wg_yok
        || (wg_bilinmiyor
            && !izlenen.is_empty()
            && izlenen.iter().all(|n| kayit[n].and_then(|d| d.get("el_sikisma")).is_none_or(Value::is_null)))
    {
        return u.izleme_bozuk("tunel_okunamadi", "tünel bilgisi okunamadı");
    }
    let mut hatalar = 0;
    for m in &musteri_listesi {
        let n = m.numara;
        // el sıkışma yaşı, durum.json'un yazıldığı ana göre
        let d = kayit[&n].map(|d| DurumKaydi {
            el_sikisma: sayi(d.get("el_sikisma")).map(|e| e + (simdi - zaman)),
            son_gun: d.get("son_gun").and_then(Value::as_str).map(String::from),
        });
        if let Err(h) = musteri_turu(veri, &u, m, cihazlar.get(&n).copied(), d.as_ref(), simdi) {
            // bir müşterinin bozuk verisi ötekileri durdurmasın
            hatalar += 1;
            eprintln!("wc-saglik: {n}: {}: {}", h.tur, h.ileti);
        }
    }
    u.bilgi_toplu(&musteriler, &cihazlar)?;
    if hatalar > 0 {
        u.izleme_bozuk("musteri_hatasi", &format!("{hatalar} müşteride hata"))?;
    }
    veri.saglik_son_tur_koy(&ortak::now_iso(simdi))?;
    Ok(())
}

/// Tur işlevi (testte bozuk olanı takılabilsin diye ayrı tür).
pub type TurIslevi<'a> = &'a dyn Fn(&Veri, &Value, &dyn Tasiyici, f64) -> Sonuc<()>;

/// Python `main`: durum.json okunur (okunamıyorsa `{}`), bir tur çalışır; beklenmeyen hatada en iyi çabayla SMS
/// ("izleme çalışmıyor (hata: <tür>)") ve 1. `saat`: hem veritabanının hem turun saati.
pub fn main_ile(db: &Path, durum_yolu: &Path, t: &dyn Tasiyici, saat: Saat, tur: TurIslevi) -> i32 {
    let d: Value = std::fs::read(durum_yolu)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| Value::Object(Default::default()));
    let veri = match Veri::ac(db, saat.clone()) {
        Ok(v) => v,
        Err(h) => {
            eprintln!("{h}");
            return 1;
        }
    };
    match tur(&veri, &d, t, saat()) {
        Ok(()) => 0,
        Err(h) => {
            eprintln!("{}: {}", h.tur, h.ileti);
            // en iyi çaba: haber ver, burada da düşme
            let _ = Uyari::yeni(&veri, t, saat()).and_then(|u| u.izleme_bozuk("hata", &format!("hata: {}", h.tur))).map_err(|h2| {
                eprintln!("{}: {}", h2.tur, h2.ileti);
            });
            1
        }
    }
}

pub fn main_uretim() -> i32 {
    let saat: Saat = Arc::new(ortak::simdi);
    let tur = |v: &Veri, d: &Value, t: &dyn Tasiyici, simdi: f64| calistir(v, d, t, simdi, Path::new(crate::listeler::DIZIN));
    main_ile(Path::new(crate::merkez::DB), Path::new(crate::durum::DURUM), &ortak::Curl, saat, &tur)
}

#[cfg(test)]
#[path = "saglik_testler.rs"]
mod testler;
