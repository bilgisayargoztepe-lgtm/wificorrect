//! Panel: haftalık ve aylık raporlar (Python raporlar.py; spec 2026-10-14). Root'un (`rapor`, wc-istatistik) yazdığı
//! `/var/lib/wificorrect/rapor/<numara>/{liste.json, hafta-*.json, ay-*.json}` dosyalarını okur ve HTML'e çevirir.
//! Sayfa açılışında arşiv (/srv) ya da rapor-ham okunmaz. Raporda kişisel veri yoktur (yalnızca sayılar): gerekçe istenmez,
//! hareket yazılmaz. JSON güvenilmez kabul edilir: her alan tür denetlenir, her metin `e()` ile basılır. JS yok.
//! `rapor_verisi` panel sayfasının verisidir.
//!
//! Python'dan fark (spec bilinçli fark 6): yalnızca güncel biçim (2) okunur; biçim 1 (alan adı listeli) dosya yok sayılır.
use std::fmt::Display;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, NaiveDate, TimeZone};
use serde_json::Value;

use crate::detay::{self, e, gecerli_gun, gun_coz};
use crate::ortak::{self, tarih_goster, TZ};
use crate::veri::{Cihaz, Sonuc, Sorun, Veri};
use crate::web::{bilgi_tablosu, kart, yanit_html, Ot, Panel, Yanit};

pub use crate::istatistik::RAPOR;

/// Panelin okuduğu biçim (`rapor::BICIM`).
pub const BICIM: u64 = 2;
const DOSYA_SINIRI: usize = 1024 * 1024;
const LISTE_EN_COK: usize = 100;
const DURUMLAR: [&str; 8] = ["tam", "kayit_yok", "eksik", "dogrulanamadi", "bekleniyor", "bugun", "gelecek", "hizmet_oncesi"];
const TAM_ACIKLAMA: &str = "Tam: kayıt geldi, mühürlü, merkezde yedekli, bütünlük (MANIFEST ve mühür zinciri) doğrulandı. \
                            Cihazın ilk kayıt gününden önceki günler sayılmaz.";

fn neden_metni(k: &str) -> Option<&'static str> {
    Some(match k {
        "okunamadi" => "kayıt dosyaları okunamadı",
        "dosya" => "bir dosyanın özeti (hash) MANIFEST'le tutmuyor",
        "zincir_yok" => "gün mühür zincirinde yok",
        "zincir" => "mühür zinciri satırı bozuk",
        "manifest" => "MANIFEST zincirdeki özetle tutmuyor",
        _ => return None,
    })
}

fn neden_anahtari(k: &str) -> Option<&'static str> {
    ["okunamadi", "dosya", "zincir_yok", "zincir", "manifest"].into_iter().find(|n| *n == k)
}

// --- tür denetimi ---

/// Python `_sayi`: negatif olmayan, bool olmayan tamsayı; yoksa 0.
fn sayi(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

fn ondalik(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).unwrap_or(0.0),
        _ => 0.0,
    }
}

fn gun(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|g| gecerli_gun(g)).map(str::to_string)
}

fn rakam_mi(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `hafta`: `(\d{4})-W(\d{2})` (hafta 1–53), `ay`: `(\d{4})-(\d{2})` (ay 1–12); ASCII, tam eşleşme.
pub fn donem_gecerli(tur: &str, donem: &str) -> bool {
    let b = donem.as_bytes();
    let sayi_basi = match tur {
        "hafta" => 6usize,
        "ay" => 5,
        _ => return false,
    };
    if b.len() != sayi_basi + 2 || !b.is_ascii() {
        return false;
    }
    let ayrac_dogru = if tur == "hafta" { &donem[4..6] == "-W" } else { &donem[4..5] == "-" };
    if !ayrac_dogru || !rakam_mi(&donem[..4]) || !rakam_mi(&donem[sayi_basi..]) {
        return false;
    }
    let n: u32 = donem[sayi_basi..].parse().unwrap_or(0);
    (1..=if tur == "hafta" { 53 } else { 12 }).contains(&n)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListeOgesi {
    pub tur: String,
    pub donem: String,
    pub baslangic: String,
    pub bitis: String,
    pub kesin: bool,
    pub farkli: u64,
    pub kayit_gun: u64,
    pub kayit_tam: u64,
}

fn liste_ogesi(x: &Value) -> Option<ListeOgesi> {
    let (tur, donem) = (x.get("tur")?.as_str()?, x.get("donem")?.as_str()?);
    if !donem_gecerli(tur, donem) {
        return None;
    }
    let (bas, bit) = (gun(x.get("baslangic"))?, gun(x.get("bitis"))?);
    Some(ListeOgesi {
        tur: tur.into(),
        donem: donem.into(),
        baslangic: bas,
        bitis: bit,
        kesin: x.get("kesin") == Some(&Value::Bool(true)),
        farkli: sayi(x.get("farkli")),
        kayit_gun: sayi(x.get("kayit_gun")),
        kayit_tam: sayi(x.get("kayit_tam")),
    })
}

fn sayi_listesi(v: Option<&Value>, n: usize) -> Vec<u64> {
    let l: &[Value] = v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let mut out: Vec<u64> = l.iter().take(n).map(|x| sayi(Some(x))).collect();
    out.resize(n, 0);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Misafir {
    pub farkli: u64,
    pub yeni: u64,
    pub donen: u64,
    pub giris: u64,
    pub giris_sms: u64,
    pub giris_kod: u64,
    pub izinli_cihaz: u64,
    pub gunluk_ortalama: f64,
    /// (gün, farklı misafir)
    pub en_yogun_gun: Option<(String, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GunSatiri {
    pub gun: String,
    /// `DURUMLAR`'dan biri ya da "bilinmiyor".
    pub durum: &'static str,
    /// Bilinen neden anahtarı ya da "".
    pub neden: &'static str,
    pub farkli: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rapor {
    pub tur: String,
    pub donem: String,
    pub baslangic: String,
    pub bitis: String,
    pub uretim: String,
    pub kesin: bool,
    pub misafir: Misafir,
    pub saat: Vec<u64>,
    pub hafta_gunu: Vec<u64>,
    pub gunler: Vec<GunSatiri>,
    pub kayit_gun: u64,
    pub kayit_tam: u64,
    pub gonderilen: u64,
    pub alinan: u64,
    pub sms: u64,
    pub surum: Option<String>,
    pub surum_sonra_degisti: bool,
}

/// Dosyadan gelen raporun güvenli kopyası; biçim, tür, dönem ya da tarihler tutmuyorsa None.
pub fn normal_rapor(r: &Value, tur: &str, donem: &str) -> Option<Rapor> {
    if !r.is_object() || r.get("bicim").and_then(Value::as_f64) != Some(BICIM as f64) || r.get("tur").and_then(Value::as_str) != Some(tur) || r.get("donem").and_then(Value::as_str) != Some(donem) {
        return None;
    }
    let (bas, bit) = (gun(r.get("baslangic"))?, gun(r.get("bitis"))?);
    let bos = Value::Null;
    let sozluk = |k: &str| -> Value { r.get(k).filter(|v| v.is_object()).cloned().unwrap_or(bos.clone()) };
    let m = sozluk("misafir");
    let en_yogun_gun = m.get("en_yogun_gun").filter(|v| v.is_object()).and_then(|ey| gun(ey.get("gun")).map(|g| (g, sayi(ey.get("farkli")))));
    let mut gunler = Vec::new();
    for g in r.get("gunler").and_then(Value::as_array).into_iter().flatten() {
        if !g.is_object() {
            continue;
        }
        let Some(gn) = gun(g.get("gun")) else { continue };
        let durum = g.get("durum").and_then(Value::as_str).and_then(|d| DURUMLAR.into_iter().find(|x| *x == d)).unwrap_or("bilinmiyor");
        let neden = g.get("neden").and_then(Value::as_str).and_then(neden_anahtari).unwrap_or("");
        gunler.push(GunSatiri { gun: gn, durum, neden, farkli: g.get("farkli").and_then(Value::as_u64) });
    }
    let (k, ku) = (sozluk("kayit"), sozluk("kullanim"));
    let surum = r.get("surum").and_then(Value::as_str).filter(|s| crate::rapor::surum_gecerli(s)).map(str::to_string);
    Some(Rapor {
        tur: tur.into(),
        donem: donem.into(),
        baslangic: bas,
        bitis: bit,
        uretim: r.get("uretim").and_then(Value::as_str).unwrap_or("").to_string(),
        kesin: r.get("kesin") == Some(&Value::Bool(true)),
        misafir: Misafir {
            farkli: sayi(m.get("farkli")),
            yeni: sayi(m.get("yeni")),
            donen: sayi(m.get("donen")),
            giris: sayi(m.get("giris")),
            giris_sms: sayi(m.get("giris_sms")),
            giris_kod: sayi(m.get("giris_kod")),
            izinli_cihaz: sayi(m.get("izinli_cihaz")),
            gunluk_ortalama: ondalik(m.get("gunluk_ortalama")),
            en_yogun_gun,
        },
        saat: sayi_listesi(r.get("saat"), 24),
        hafta_gunu: sayi_listesi(r.get("hafta_gunu"), 7),
        gunler,
        kayit_gun: sayi(k.get("gun")),
        kayit_tam: sayi(k.get("tam")),
        gonderilen: sayi(ku.get("gonderilen")),
        alinan: sayi(ku.get("alinan")),
        sms: sayi(ku.get("sms")),
        surum,
        surum_sonra_degisti: r.get("surum_sonra_degisti") == Some(&Value::Bool(true)),
    })
}

/// Yalnızca `<dizin>/<numara>/` altındaki iki tür dosyayı okur; yol yalnızca doğrulanmış numara ve dönemden kurulur.
#[derive(Clone, Debug)]
pub struct Raporlar {
    pub dizin: PathBuf,
}

impl Default for Raporlar {
    fn default() -> Self {
        Raporlar { dizin: RAPOR.into() }
    }
}

impl Raporlar {
    pub fn new(dizin: impl Into<PathBuf>) -> Raporlar {
        Raporlar { dizin: dizin.into() }
    }

    fn oku(&self, numara: &str, ad: &str) -> Option<Value> {
        if !crate::rapor::numara_mi(numara) {
            return None;
        }
        use std::io::Read;
        let f = std::fs::File::open(self.dizin.join(numara).join(ad)).ok()?;
        let mut b = Vec::new();
        f.take(DOSYA_SINIRI as u64 + 1).read_to_end(&mut b).ok()?;
        if b.len() > DOSYA_SINIRI {
            return None;
        }
        serde_json::from_str(std::str::from_utf8(&b).ok()?).ok()
    }

    pub fn liste(&self, numara: impl Display) -> Vec<ListeOgesi> {
        let ham = self.oku(&numara.to_string(), "liste.json");
        ham.as_ref().and_then(Value::as_array).into_iter().flatten().take(LISTE_EN_COK).filter_map(liste_ogesi).collect()
    }

    pub fn rapor(&self, numara: impl Display, tur: &str, donem: &str) -> Option<Rapor> {
        if !donem_gecerli(tur, donem) {
            return None;
        }
        normal_rapor(&self.oku(&numara.to_string(), &format!("{tur}-{donem}.json"))?, tur, donem)
    }

    pub fn dizin(&self) -> &Path {
        &self.dizin
    }
}

// --- biçimlendirme ---

/// Sayıdan sonra kesme işaretiyle gelen 3. tekil iyelik eki: 6'sı, 31'i, 4'ü, 10'u, 40'ı.
pub fn iyelik(n: u64) -> &'static str {
    if n == 0 {
        return "ı";
    }
    if n % 10 != 0 {
        return ["i", "si", "ü", "ü", "i", "sı", "si", "i", "u"][(n % 10 - 1) as usize];
    }
    if n / 10 % 10 != 0 {
        return ["u", "si", "u", "ı", "si", "ı", "i", "i", "ı"][(n / 10 % 10 - 1) as usize];
    }
    if n / 100 % 10 != 0 {
        "ü"
    } else {
        "i"
    }
}

/// Yalnızca tamsayılardan kurulur (kaçış gerekmez; kesme işareti `e()`'de `&#x27;` olurdu).
pub fn tam_metni(gun: u64, tam: u64) -> String {
    if gun != 0 {
        format!("{gun} günün {tam}'{} tam", iyelik(tam))
    } else {
        "Henüz biten gün yok".into()
    }
}

fn t(g: &str) -> NaiveDate {
    gun_coz(g).unwrap_or(NaiveDate::MIN)
}

/// Hafta: '12–18.10.2026', '29.09–05.10.2026', '28.12.2026–03.01.2027'; ay: 'Ekim 2026'.
pub fn donem_adi(tur: &str, bas: &str, bit: &str) -> String {
    let (b, s) = (t(bas), t(bit));
    if tur == "ay" {
        return format!("{} {}", detay::AYLAR[b.month0() as usize], b.year());
    }
    if b.year() != s.year() {
        return format!("{}–{}", b.format("%d.%m.%Y"), s.format("%d.%m.%Y"));
    }
    if b.month() != s.month() {
        return format!("{}–{}", b.format("%d.%m"), s.format("%d.%m.%Y"));
    }
    format!("{}–{}", b.format("%d"), s.format("%d.%m.%Y"))
}

fn bugun(simdi: f64) -> String {
    TZ.timestamp_opt(simdi.floor() as i64, 0).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

pub fn sure_metni(sn: i64) -> String {
    let dk = sn.max(0) / 60;
    if dk < 60 {
        return format!("{dk} dk");
    }
    if dk < 1440 {
        return format!("{} sa {} dk", dk / 60, dk % 60);
    }
    format!("{} gün {} sa", dk / 1440, dk % 1440 / 60)
}

/// Python `datetime.fromisoformat(str(iso))` (saat dilimi yoksa TR): epoch saniyesi.
pub(crate) fn epoch(iso: &str) -> Option<f64> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(iso) {
        return Some(d.timestamp() as f64 + d.timestamp_subsec_nanos() as f64 / 1e9);
    }
    for bicim in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(n) = chrono::NaiveDateTime::parse_from_str(iso, bicim) {
            return TZ.from_local_datetime(&n).single().map(|d| d.timestamp() as f64 + d.timestamp_subsec_nanos() as f64 / 1e9);
        }
    }
    None
}

/// `[baş 00:00, bitişten sonraki gün 00:00)` TR: ISO metin ve epoch.
pub fn donem_sinirlari(bas: &str, bit: &str) -> (String, String, f64, f64) {
    let gece = |d: NaiveDate| TZ.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap_or_default()).single();
    let (a, b) = (gece(t(bas)), gece(t(bit) + Duration::days(1)));
    let iso = |d: Option<chrono::DateTime<chrono::FixedOffset>>| d.map(|d| d.format("%Y-%m-%dT%H:%M:%S%:z").to_string()).unwrap_or_default();
    let ts = |d: Option<chrono::DateTime<chrono::FixedOffset>>| d.map(|d| d.timestamp() as f64).unwrap_or(0.0);
    (iso(a), iso(b), ts(a), ts(b))
}

/// `ulasilamiyor` sorunlarının dönemle (ve şimdiye kadarla) kesişen toplam süresi (dönem öncesi açılıp süren dahil).
pub fn cevrimdisi_sn(sorunlar: &[Sorun], bas: &str, bit: &str, simdi: f64) -> i64 {
    let (_, _, a, b) = donem_sinirlari(bas, bit);
    let b = b.min(simdi);
    let mut top = 0.0f64;
    for s in sorunlar {
        if s.tur != "ulasilamiyor" {
            continue;
        }
        let ac = epoch(&s.acilis);
        let kp = if s.kapanis.is_empty() { Some(simdi) } else { epoch(&s.kapanis) };
        let (Some(ac), Some(kp)) = (ac, kp) else { continue };
        top += (kp.min(b) - ac.max(a)).max(0.0);
    }
    top as i64
}

/// Dönemde açılan kritik/uyarı sorunlar (bilgi, ör. lisans, raporda yok).
pub fn donemde_acilan<'a>(sorunlar: &'a [Sorun], bas: &str, bit: &str) -> Vec<&'a Sorun> {
    let (a, b, _, _) = donem_sinirlari(bas, bit);
    sorunlar.iter().filter(|s| (s.onem == "kritik" || s.onem == "uyari") && a.as_str() <= s.acilis.as_str() && s.acilis.as_str() < b.as_str()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct CihazSorun {
    pub tur: String,
    pub onem: String,
    pub metin: String,
    pub acilis: String,
    pub kapanis: Option<String>,
    pub sure_sn: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CihazBolumu {
    pub surum: Option<String>,
    pub cevrimdisi_sn: i64,
    pub sorunlar: Vec<CihazSorun>,
}

/// Raporun "Cihaz" bölümü. `sorunlar`: dönemle kesişen bütün sorunlar (`veri.sorunlar_aralik`); listede yalnızca dönemde
/// açılan kritik/uyarılar, çevrimdışı süre hepsinden. Sürüm: raporun dönem sonu sürümü; yoksa ve sonradan değişmediyse
/// cihazın bugünkü sürümü; yoksa None.
pub fn cihaz_bolumu(r: &Rapor, sorunlar: &[Sorun], cihaz_surumu: &str, simdi: f64) -> CihazBolumu {
    let (bas, bit) = (r.baslangic.as_str(), r.bitis.as_str());
    let mut surum = r.surum.clone();
    if surum.is_none() && !r.surum_sonra_degisti && crate::rapor::surum_gecerli(cihaz_surumu) {
        surum = Some(cihaz_surumu.to_string());
    }
    let mut liste = Vec::new();
    for s in donemde_acilan(sorunlar, bas, bit) {
        let ac = epoch(&s.acilis);
        let kapanis = (!s.kapanis.is_empty()).then(|| s.kapanis.clone());
        let kp = kapanis.as_deref().and_then(epoch);
        liste.push(CihazSorun {
            tur: s.tur.clone(),
            onem: s.onem.clone(),
            metin: s.metin.clone(),
            acilis: s.acilis.clone(),
            kapanis,
            sure_sn: ac.map(|ac| ((kp.unwrap_or(simdi) - ac) as i64).max(0)),
        });
    }
    CihazBolumu { surum, cevrimdisi_sn: cevrimdisi_sn(sorunlar, bas, bit, simdi), sorunlar: liste }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Isletme {
    pub numara: String,
    pub ad: String,
    pub unvan: String,
}

/// Rapor başlığı: bağlı cihazın bildirdiği işletme adı (yoksa "Müşteri <numara>") ve unvanı.
pub fn isletme_bilgisi(c: Option<&Cihaz>, numara: impl Display) -> Isletme {
    Isletme {
        numara: numara.to_string(),
        ad: c.map(|c| c.isletme_adi.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| format!("Müşteri {numara}")),
        unvan: c.map(|c| c.unvan.clone()).unwrap_or_default(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RaporVerisi {
    pub rapor: Rapor,
    pub isletme: Isletme,
    pub cihaz: CihazBolumu,
}

/// `{rapor, isletme, cihaz}` ya da None (dönem geçersiz, dosya yok/bozuk). Okunan: tek dönem JSON'u + bir sorun sorgusu.
pub fn rapor_verisi(veri: &Veri, raporlar: &Raporlar, numara: i64, tur: &str, donem: &str, simdi: f64) -> Sonuc<Option<RaporVerisi>> {
    let Some(r) = raporlar.rapor(numara, tur, donem) else { return Ok(None) };
    let c = veri.bagli_cihaz(numara)?;
    let (a, b, _, _) = donem_sinirlari(&r.baslangic, &r.bitis);
    let sorunlar = veri.sorunlar_aralik(numara, &a, &b)?;
    let cihaz = cihaz_bolumu(&r, &sorunlar, c.as_ref().map(|c| c.surum.as_str()).unwrap_or(""), simdi);
    Ok(Some(RaporVerisi { rapor: r, isletme: isletme_bilgisi(c.as_ref(), numara), cihaz }))
}

// --- sayfalar ---

pub fn liste_html(liste: &[ListeOgesi], onek: &str, simdi: f64) -> String {
    if liste.is_empty() {
        return kart("<h1>Raporlar</h1><p>Henüz rapor yok; ilk gün arşive geldiğinde oluşur.</p>");
    }
    let bugun = bugun(simdi);
    let mut bolumler = String::new();
    for (tur, baslik) in [("hafta", "Haftalık raporlar"), ("ay", "Aylık raporlar")] {
        let mut satirlar = Vec::new();
        for x in liste.iter().filter(|x| x.tur == tur) {
            let etiket = if x.bitis >= bugun {
                "<b class=\"uyarisi\">sürüyor</b>"
            } else if x.kesin {
                ""
            } else {
                "<span class=\"alt\">kesinleşmedi</span>"
            };
            satirlar.push(format!(
                "<tr><td><a href=\"{}/rapor/{}/{}\">{}</a> {etiket}</td><td class=\"n\">{}</td><td>{}</td></tr>",
                e(onek),
                e(tur),
                e(&x.donem),
                e(donem_adi(tur, &x.baslangic, &x.bitis)),
                x.farkli,
                tam_metni(x.kayit_gun, x.kayit_tam)
            ));
        }
        let govde = if satirlar.is_empty() {
            "<p class=\"alt\">Bu türde rapor yok.</p>".to_string()
        } else {
            format!("<div class=\"kaydir\"><table class=\"detay\"><tr><th>Dönem</th><th>Farklı misafir</th><th>5651 kaydı</th></tr>{}</table></div>", satirlar.concat())
        };
        bolumler += &kart(&format!("<h2>{baslik}</h2>{govde}"));
    }
    kart(
        "<h1>Raporlar</h1><p class=\"alt\">Misafir sayıları, 5651 kayıt durumu ve cihazın çalışması. Raporlarda \
         kişisel veri (telefon, ad, MAC, IP) yoktur. Her gün 02:30 ve 12:30'da güncellenir.</p>",
    ) + &bolumler
}

fn cubuklar(etiketler: &[String], sayilar: &[u64]) -> String {
    let en = sayilar.iter().copied().max().unwrap_or(0).max(1);
    let satirlar: String = etiketler
        .iter()
        .zip(sayilar)
        .map(|(et, n)| format!("<tr><td>{}</td><td class=\"cubuk\"><span style=\"width:{}%\"></span></td><td class=\"n\">{n}</td></tr>", e(et), (*n as u128 * 100 / en as u128)))
        .collect();
    format!("<table>{satirlar}</table>")
}

fn gun_durumu_html(g: &GunSatiri) -> String {
    match g.durum {
        "tam" => "<span class=\"iyi\">✔ Tam</span>".into(),
        "kayit_yok" => "<b class=\"kotu\">Kayıt gelmedi</b> <span class=\"alt\">(cihaz kapalı, bağlantısız ya da yedek gönderilmedi)</span>".into(),
        "eksik" => "<b class=\"kotu\">Eksik geldi</b> <span class=\"alt\">(günün dosyaları merkeze tam ulaşmadı)</span>".into(),
        "dogrulanamadi" => format!("<b class=\"kotu\">Doğrulanamadı</b> <span class=\"alt\">({})</span>", e(neden_metni(g.neden).unwrap_or("neden bilinmiyor"))),
        "bekleniyor" => "<span class=\"uyarisi\">Bekleniyor</span> <span class=\"alt\">(gece yedeğiyle gelir)</span>".into(),
        "bugun" => "Sürüyor".into(),
        "gelecek" => "<span class=\"bos\">—</span>".into(),
        "hizmet_oncesi" => "<span class=\"bos\">Hizmet öncesi</span> <span class=\"alt\">(cihaz henüz kurulmamıştı; sayılmaz)</span>".into(),
        _ => "<span class=\"bos\">Bilinmiyor</span>".into(),
    }
}

/// Python `f"{x:.1f}".replace(".", ",")`.
fn virgullu(x: f64) -> String {
    crate::rapor::ondalik1(x).replace('.', ",")
}

fn boyut(n: u64) -> String {
    detay::boyut(n.min(i64::MAX as u64) as i64)
}

pub fn rapor_html(v: &RaporVerisi, simdi: f64, onek: &str) -> String {
    let (r, isl, ch) = (&v.rapor, &v.isletme, &v.cihaz);
    let bugun = bugun(simdi);
    let ad = donem_adi(&r.tur, &r.baslangic, &r.bitis);
    let tur_adi = if r.tur == "hafta" { "Haftalık rapor" } else { "Aylık rapor" };
    let suruyor = r.bitis >= bugun;
    let durum = if suruyor {
        "<p class=\"uyarisi\">Dönem sürüyor — her gün 02:30 ve 12:30'da güncellenir.</p>"
    } else if r.kesin {
        ""
    } else {
        "<p class=\"alt\">Geç gelen günlerle birkaç gün içinde değişebilir.</p>"
    };
    let ust = format!(
        "<p class=\"yazdirma alt\"><a href=\"{}/raporlar\">← Raporlar</a> · Yazdırmak ya da PDF olarak kaydetmek \
         için Ctrl+P'ye basın (telefonda tarayıcı menüsünden Yazdır / Paylaş).</p>",
        e(onek)
    );
    let uretim = tarih_goster(&r.uretim, false);
    let bas_kart = kart(&format!(
        "<h1>{} · {}</h1>{}{durum}",
        e(tur_adi),
        e(&ad),
        bilgi_tablosu(&[
            ("İşletme", e(&isl.ad)),
            ("Unvan", e(if isl.unvan.is_empty() { "—" } else { &isl.unvan })),
            ("Müşteri no", e(&isl.numara)),
            ("Dönem", e(format!("{} – {}", tarih_goster(&r.baslangic, false), tarih_goster(&r.bitis, false)))),
            ("Üretim", e(if uretim.is_empty() { "—" } else { &uretim })),
        ])
    ));

    let satirlar: String = r
        .gunler
        .iter()
        .map(|g| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"n\">{}</td></tr>",
                e(tarih_goster(&g.gun, false)),
                e(detay::KISA_GUN[t(&g.gun).weekday().num_days_from_monday() as usize]),
                gun_durumu_html(g),
                g.farkli.map(|f| f.to_string()).unwrap_or_default()
            )
        })
        .collect();
    let kayit = kart(&format!(
        "<h2>5651 kayıt durumu</h2><p><b>{}</b></p><p class=\"alt\">{}</p>\
         <div class=\"kaydir\"><table class=\"detay\"><tr><th>Tarih</th><th>Gün</th><th>Durum</th><th>Misafir</th></tr>{satirlar}</table></div>",
        tam_metni(r.kayit_gun, r.kayit_tam),
        e(TAM_ACIKLAMA)
    ));

    let m = &r.misafir;
    let en_yogun = match &m.en_yogun_gun {
        Some((g, f)) => format!("{} {} · {f} misafir", tarih_goster(g, false), detay::GUN_ADI[t(g).weekday().num_days_from_monday() as usize]),
        None => "—".into(),
    };
    let misafir = kart(&format!(
        "<h2>Misafirler</h2>{}<h3>Giriş yöntemi</h3>{}",
        bilgi_tablosu(&[
            ("Farklı misafir", m.farkli.to_string()),
            ("İlk kez gelen (yeni)", m.yeni.to_string()),
            ("Daha önce de gelmiş", m.donen.to_string()),
            ("Toplam giriş", m.giris.to_string()),
            ("Günlük ortalama", e(virgullu(m.gunluk_ortalama))),
            ("En yoğun gün", e(en_yogun)),
        ]),
        bilgi_tablosu(&[
            ("SMS ile giriş", m.giris_sms.to_string()),
            ("Misafir kodu ile giriş", m.giris_kod.to_string()),
            ("İzinli cihaz (portalsız)", m.izinli_cihaz.to_string()),
        ])
    ));

    let saat_et: Vec<String> = (0..24).map(|h| format!("{h:02}–{:02}", (h + 1) % 24)).collect();
    let gun_et: Vec<String> = detay::GUN_ADI.iter().map(|s| s.to_string()).collect();
    let yogunluk = kart(&format!(
        "<h2>Yoğunluk</h2><p class=\"alt\">Her satır: o saatte ya da günde internete çıkan farklı misafir.</p>\
         <h3>Saate göre</h3>{}<h3>Haftanın gününe göre</h3>{}",
        cubuklar(&saat_et, &r.saat),
        cubuklar(&gun_et, &r.hafta_gunu)
    ));

    let toplam = r.gonderilen.saturating_add(r.alinan);
    let kullanim = kart(&format!(
        "<h2>Kullanım</h2>{}",
        bilgi_tablosu(&[
            ("Toplam veri", e(format!("{} (indirme {}, yükleme {})", boyut(toplam), boyut(r.alinan), boyut(r.gonderilen)))),
            ("Doğrulama SMS'i", r.sms.to_string()),
        ])
    ));

    let surum = ch.surum.clone().unwrap_or_else(|| "—".into());
    let cevrimdisi = ch.cevrimdisi_sn;
    let sorun_satirlari: Vec<String> = ch
        .sorunlar
        .iter()
        .map(|s| {
            let sure = s.sure_sn.map(sure_metni).unwrap_or_else(|| "—".into());
            let kapanis = match &s.kapanis {
                Some(k) => e(ortak::tarih_goster(k, false)),
                None => "<b class=\"kotu\">açık</b>".into(),
            };
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{kapanis}</td><td>{}</td></tr>",
                e(&s.metin),
                if s.onem == "kritik" { "Kritik" } else { "Uyarı" },
                e(tarih_goster(&s.acilis, false)),
                e(sure)
            )
        })
        .collect();
    let sorun_tablo = if sorun_satirlari.is_empty() {
        "<p class=\"alt\">Bu dönemde açılan sorun yok.</p>".to_string()
    } else {
        format!("<div class=\"kaydir\"><table class=\"detay\"><tr><th>Sorun</th><th>Önem</th><th>Açılış</th><th>Kapanış</th><th>Süre</th></tr>{}</table></div>", sorun_satirlari.concat())
    };
    let cihaz = kart(&format!(
        "<h2>Cihaz</h2>{}<h3>Dönemde açılan sorunlar</h3>{sorun_tablo}\
         <p class=\"alt\">Çevrimdışı süre merkezin cihaza ulaşamadığını fark ettiği andan sayılır; gerçek süre biraz daha uzun olabilir.</p>",
        bilgi_tablosu(&[
            ("Toplam çevrimdışı süre", if cevrimdisi != 0 { e(format!("en az {}", sure_metni(cevrimdisi))) } else { "Yok".to_string() }),
            ("Yazılım sürümü (dönem sonu)", e(surum)),
        ])
    ));
    ust + &bas_kart + &kayit + &misafir + &yogunluk + &kullanim + &cihaz
}

/// Müşteri paneli (onek "") ve yönetim (onek "/m/<n>") için ortak: `alt` "/raporlar" ya da "/rapor/<tür>/<dönem>". Gerekçe
/// istenmez, hareket yazılmaz (kişisel veri yok). Okunan: liste.json ya da tek dönem JSON'u + sorun tablosu.
pub fn istek(panel: &dyn Panel, ot: &Ot, numara: i64, alt: &str, onek: &str) -> Sonuc<Option<Yanit>> {
    let t = panel.taban();
    let simdi = t.simdi();
    if alt == "/raporlar" {
        let h = liste_html(&t.raporlar.liste(numara), onek, simdi);
        return Ok(Some(yanit_html(&panel.sayfa("Raporlar", &h, Some(ot), true, None), 200, None)));
    }
    // YOL_RE: /rapor/(hafta|ay)/([^/]*) tam eşleşme
    let yol = alt.strip_prefix("/rapor/").and_then(|r| r.split_once('/')).filter(|(tur, donem)| matches!(*tur, "hafta" | "ay") && !donem.contains('/'));
    let v = match yol {
        Some((tur, donem)) => rapor_verisi(&t.veri, &t.raporlar, numara, tur, donem, simdi)?,
        None => None,
    };
    let Some(v) = v else {
        return Ok(Some(yanit_html(&panel.mesaj(Some(ot), "Bulunamadı", "Bu dönem için rapor yok."), 404, None)));
    };
    let r = &v.rapor;
    let baslik = format!("Rapor {}", donem_adi(&r.tur, &r.baslangic, &r.bitis));
    Ok(Some(yanit_html(&panel.sayfa(&baslik, &rapor_html(&v, simdi, onek), Some(ot), true, None), 200, None)))
}

#[cfg(test)]
#[path = "raporlar_testler.rs"]
pub(crate) mod testler;
