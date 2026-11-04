//! Yönetim paneli (Python yonetim.py, spec §5): yonetim.wificorrect.com — müşteriler, cihazlar, parola sıfırlama, serbest
//! bırakma, üyelik, kayıtlar (gerekçeyle), raporlar, hareketler, eski arşivler, yönetici hesabı. Sol menü gruplu.
//!
//! Ayar sayfaları (`/listeler`, `/sms`, `/uyari`, `/uyari/deneme`, `/surumler` ve POST'ları) `ayar_sayfalari` kancasındadır.
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Map, Value};

use crate::api::KuyrukEkle;
use crate::cihaz::CIHAZ_GIRIS;
use crate::detay::{self, e, tr_kucuk};
use crate::guvenlik::{self, GecisBelirtecleri};
use crate::kayitlar::{son_30_gun, Kayitlar};
use crate::listeler::{self, Uretici};
use crate::merkez::py_int;
use crate::musteri::Musteri;
use crate::netgsm;
use crate::ortak::{now_iso, py_dogru, py_str, py_strip, tarih_girdi, tarih_goster, tarih_saat_goster, Tasiyici};
use crate::surumler;
use crate::raporlar::{self, Raporlar};
use crate::saglik::{self, CEVRIMICI_SN, TEKRAR_SN};
use crate::veri::{self, Cihaz, Hareket, Hata, Saat, SaglikKaydi, Sonuc, SmsSon, Sorun, Veri};
use crate::web::{self, acilir, bilgi_tablosu, bolum, kart, sorgu_ilk, yanit_html, yonlendir, Ot, Panel, Taban, Yanit};
use crate::yonetici;

/// Cihaz son eşitlemesinden bu kadar sonra listede sorunlu (26 sa).
pub const ESLESME_GECIKME_SN: f64 = 26.0 * 3600.0;
/// wc-saglik 5 dk'da bir; son turu bundan eskiyse izleme çalışmıyor olabilir.
pub const IZLEME_BAYAT_SN: f64 = 900.0;

/// Günlük uyarı SMS sınırı aralığı (yönetim formu).
pub const SINIR_ARALIK: (i64, i64) = veri::UYARI_SINIR_ARALIK;
pub const DENEME_METNI: &str = "WifiCorrect deneme uyarısı";

/// GitHub sürümlerini alan işlev (Python `github_al`): (eklenen sürümler, hata metinleri) ya da liste alınamadı hatası.
pub type GithubAl = Box<dyn Fn(&Veri, &Path, &dyn Tasiyici) -> Result<(Vec<String>, Vec<String>), String> + Send + Sync>;

/// Sol menü (2026-10-24 kullanıcı kararı): grup başlığı ve bağlantıları. Admin parolası Hesabım'da, Sürümler Ayarlar'da.
pub const MENU: [(&str, &[(&str, &str)]); 3] = [
    ("Müşteriler", &[("/", "Müşteri listesi"), ("/yeni", "Yeni müşteri"), ("/bekleyen", "Bekleyen cihazlar")]),
    ("Kayıtlar", &[("/hareketler", "Hareketler"), ("/eski-arsivler", "Eski arşivler")]),
    (
        "Ayarlar",
        &[("/listeler", "Engel listeleri"), ("/sms", "SMS ayarları"), ("/uyari", "Uyarı ayarları"), ("/surumler", "Sürümler"), ("/hesabim", "Hesabım")],
    ),
];

/// Müşteri sayfasının açılır bölümleri (kullanıcı isteği 2026-10-26): kimlik → başlık, sayfadaki sırayla.
pub const MUSTERI_BOLUMLERI: [(&str, &str); 6] = [
    ("parolalar", "Parolalar"),
    ("lisans", "Lisans ve üyelik"),
    ("cihaz", "Cihaz"),
    ("saglik", "Sağlık"),
    ("guncelleme", "Güncelleme"),
    ("sorunlar", "Sorun geçmişi"),
];

/// Açık sayfanın menüdeki bağlantısı (yoksa None): müşteri sayfaları listede, admin parolası Hesabım'da.
pub fn secili_menu(yol: &str) -> Option<&'static str> {
    if yol == "/admin-parolasi" {
        return Some("/hesabim");
    }
    for (_, baglantilar) in MENU {
        for (href, _) in baglantilar {
            if *href != "/" && (yol == *href || yol.strip_prefix(*href).is_some_and(|k| k.starts_with('/'))) {
                return Some(*href);
            }
        }
    }
    if yol == "/" || yol.starts_with("/m/") {
        Some("/")
    } else {
        None
    }
}

/// Cihazın bildirdiği işletme türünün etiketi; cihaz yoksa ya da tür bildirilmemişse "—".
pub fn isletme_turu(c: Option<&Cihaz>) -> String {
    let Some(c) = c else { return "—".into() };
    veri::ISLETME_TURLERI.iter().find(|(k, _)| *k == c.isletme_turu).map(|(_, v)| v.to_string()).unwrap_or_else(|| "—".into())
}

/// Yönetici parolayı iki kez yazar (yanlış yazılan parola müşteriye/cihaza gitmesin).
pub fn parola_alanlari(etiket: &str) -> String {
    format!(
        "<label for=\"p1\">{etiket} (en az {} karakter)</label>\
         <input id=\"p1\" name=\"parola\" type=\"password\" autocomplete=\"new-password\" required>\
         <label for=\"p2\">Parola (tekrar)</label>\
         <input id=\"p2\" name=\"parola2\" type=\"password\" autocomplete=\"new-password\" required>",
        guvenlik::EN_AZ
    )
}

/// Dönen: parola ya da hata metni. Kısa ya da iki alan farklıysa hata.
pub fn parola_formdan(form: &HashMap<String, String>) -> Result<String, String> {
    let (pw, pw2) = (form.get("parola").map(String::as_str).unwrap_or(""), form.get("parola2").map(String::as_str).unwrap_or(""));
    if pw.chars().count() < guvenlik::EN_AZ {
        return Err(format!("Parola en az {} karakter olmalı.", guvenlik::EN_AZ));
    }
    if pw != pw2 {
        return Err("İki parola aynı değil.".into());
    }
    Ok(pw.to_string())
}

/// İşlemden sonra müşteri sayfasına dönüş: işlemin bölümü açık gelir (betik yok: ?acik=; çapa sayfayı oraya kaydırır).
pub fn musteri_donus(n: i64, kimlik: &str) -> Yanit {
    yonlendir(&format!("/m/{n}?acik={kimlik}#{kimlik}"), None)
}

fn sms_etiket(d: &str) -> Option<&'static str> {
    Some(match d {
        "gonderildi" => "SMS gitti",
        "mock" => "deneme modu",
        "sinir" => "günlük SMS sınırı",
        "yok" => "SMS gönderilmedi (uyarılar kapalı)",
        _ => return None,
    })
}

fn onem_etiketi(onem: &str) -> String {
    match onem {
        "kritik" => "kritik".into(),
        "uyari" => "uyarı".into(),
        "bilgi" => "bilgi".into(),
        d => d.to_string(),
    }
}

/// Sağlık kartındaki "İnternet" satırı (PPPoE, spec 2026-10-23 "Durum ve teşhis"): raporun `pppoe` durum kodu → (metin, sorunlu mu).
fn internet_durumu(kod: &str) -> Option<(&'static str, bool)> {
    Some(match kod {
        "kapali" => ("Modem (DHCP)", false),
        "kablo_yok" => ("kablo yok", true),
        "dhcp_yok" => ("modemden adres alınamadı", true),
        "internet_yok" => ("adres var ama internet yok", true), // PPPoE kapalı, etkin yoklama internete çıkamadı (≥ 0.4.20)
        "sunucu_yok" => ("PPPoE: sunucu yanıt vermiyor", true),
        "modemden" => ("Modemden bağlı (DHCP); PPPoE ONT'ye takılınca bağlanır", false), // PPPoE açık, modemli hat (≥ 0.4.18)
        "kimlik_hatasi" => ("PPPoE: parola reddedildi", true),
        "baglaniyor" => ("PPPoE: bağlanıyor", false),
        "bagli" => ("PPPoE bağlı", false),
        _ => return None,
    })
}

/// Açık sorun satırının SMS durumu (acilis_sms / kapanis_sms değeri). Kaçış çağıranın işi.
pub fn sms_etiketi(deger: &str, onem: &str) -> String {
    if let Some(h) = deger.strip_prefix("hata:") {
        return format!("SMS gönderilemedi ({h})");
    }
    if deger.starts_with("sinir") {
        // sinir:<gün>: o gün sınıra takıldı (açılışsa ertesi gün yeniden denenir)
        return "günlük SMS sınırı".into();
    }
    if let Some(t) = sms_etiket(deger) {
        return t.into();
    }
    if onem == "bilgi" { "09:00'da gidecek" } else { "SMS bekleniyor" }.into()
}

/// Sorun geçmişi satırının (açılış SMS'i, kapanış SMS'i) etiketleri. Kapanmış sorunda boş değer artık gitmeyecek demektir
/// ("—"); tek istisna: açılışı gitmiş, "düzeldi"si TEKRAR_SN dolana dek bekletiliyor.
pub fn gecmis_sms_etiketleri(x: &Sorun, simdi: f64) -> (String, String) {
    if x.kapanis.is_empty() {
        return (sms_etiketi(&x.acilis_sms, &x.onem), "—".into());
    }
    let acilis = if x.acilis_sms.is_empty() { "—".to_string() } else { sms_etiketi(&x.acilis_sms, &x.onem) };
    if !x.kapanis_sms.is_empty() {
        return (acilis, sms_etiketi(&x.kapanis_sms, &x.onem));
    }
    let kapanis = raporlar::epoch(&x.kapanis).filter(|v| *v != 0.0).unwrap_or(0.0);
    if x.onem != "bilgi" && (x.acilis_sms == "gonderildi" || x.acilis_sms == "mock") && simdi - kapanis < TEKRAR_SN {
        return (acilis, "düzeldi bekletiliyor".into());
    }
    (acilis, "—".into())
}

/// Sağlık kartındaki "İnternet" satırı (HTML): yalnızca raporun `pppoe` durum koduna göre. Kod yoksa "—": eski cihaz
/// göndermemiştir ya da saglik_temizle bilinmeyen/bozuk değeri atmıştır; `wan_tur` tek başına gösterilmez. `r` cihazdan gelen
/// veri: yalnızca bilinen değerler gösterilir (bilinmeyen değer sayfaya hiç girmez).
pub fn internet_metni(r: &Map<String, Value>) -> String {
    if let Some((metin, sorunlu)) = r.get("pppoe").and_then(Value::as_str).and_then(internet_durumu) {
        return if sorunlu { format!("<b class=\"kotu\">{}</b>", e(metin)) } else { e(metin) };
    }
    "—".into()
}

/// Python `once(sn)`: "N dk önce" / "N saat önce" / "N gün önce".
pub fn once(sn: f64) -> String {
    let dk = (sn as i64).div_euclid(60);
    if dk < 120 {
        format!("{dk} dk önce")
    } else if dk < 2880 {
        format!("{} saat önce", dk.div_euclid(60))
    } else {
        format!("{} gün önce", dk.div_euclid(1440))
    }
}

/// Müşteri sayfası: "<zaman> · <sağlayıcı> · <sonuç/kod>" (deneme modundaki gönderimler yazılmaz).
pub fn son_sms_metni(s: Option<&SmsSon>) -> String {
    let Some(s) = s else { return "—".into() };
    let saglayici = match s.saglayici.as_str() {
        "netgsm" => "NetGSM",
        "twilio" => "Twilio",
        _ => "—",
    };
    let sonuc = if s.sonuc == "gonderildi" { "gönderildi".to_string() } else { format!("{} ({})", s.sonuc, s.kod) };
    e(format!("{} · {saglayici} · {sonuc}", tarih_goster(&s.zaman, false)))
}

/// `common.tarih_goster(değer)`: None/null → "", metin → gün.ay.yıl, öteki → `str(değer)`.
fn tg_deger(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => tarih_goster(s, false),
        Some(x) => py_str(x),
    }
}

fn veya_tire(s: String) -> String {
    if s.is_empty() { "—".into() } else { s }
}

/// `e(x or "—")`.
fn e_veya(s: &str) -> String {
    e(if s.is_empty() { "—" } else { s })
}

fn veri_hatasi(ne: &str, h: impl std::fmt::Display) -> Hata {
    Hata::Db(format!("{ne}: {h}"))
}

pub struct Yonetim {
    taban: Taban,
    pub yonetici_yolu: PathBuf,
    pub durum_yolu: PathBuf,
    kuyruk_ekle: KuyrukEkle,
    /// cihaz paneline doğrudan geçiş: müşteri paneliyle aynı belirteç deposu ve cihaz adresi kuralı
    pub gecis: Option<Arc<GecisBelirtecleri>>,
    pub musteri_app: Option<Arc<Musteri>>,
    /// eski arşiv bağı değişince root'un okuduğu grup dosyası (None: yazılmaz)
    pub rapor_gruplari: Option<PathBuf>,
    /// uzaktan güncelleme sürüm deposu ve hazır engel listeleri dizini
    pub surum_dizini: PathBuf,
    pub liste_dizini: PathBuf,
    /// deneme SMS'i (testte `AgYok`/`SahteAg`; üretimde curl, `WC_TEST=1` altında reddedilir)
    pub sms_tasiyici: Arc<dyn Tasiyici>,
    /// GitHub sürüm listesi ve liste kaynakları indirmesi (testte sahte)
    pub indirme_tasiyici: Arc<dyn Tasiyici>,
    /// Python `L.KAYNAKLAR`/`L.EN_COK`: liste kaynakları (testte değiştirilir)
    pub liste_uretici: Uretici,
    github_al: GithubAl,
}

impl Yonetim {
    /// Python `Yonetim(veri, kayitlar, saat, yonetici_yolu, durum_yolu, kuyruk_ekle)`; öteki parametreler `*_ile` yöntemleriyle.
    pub fn new(veri: Arc<Veri>, kayitlar: Kayitlar, saat: Saat, yonetici_yolu: PathBuf, durum_yolu: PathBuf, kuyruk_ekle: KuyrukEkle) -> Yonetim {
        // testte varsayılan: dış istek gitmemeli (gerçek SMS/GitHub asla); sahte taşıyıcı açıkça verilir
        #[cfg(test)]
        let (sms_tasiyici, indirme_tasiyici): (Arc<dyn Tasiyici>, Arc<dyn Tasiyici>) = (Arc::new(crate::test_yardim::AgYok), Arc::new(crate::test_yardim::AgYok));
        #[cfg(not(test))]
        let (sms_tasiyici, indirme_tasiyici): (Arc<dyn Tasiyici>, Arc<dyn Tasiyici>) = (Arc::new(crate::ortak::Curl), Arc::new(crate::ortak::Curl));
        Yonetim {
            taban: Taban::new(veri, kayitlar, saat),
            yonetici_yolu,
            durum_yolu,
            kuyruk_ekle,
            gecis: None,
            musteri_app: None,
            rapor_gruplari: None,
            surum_dizini: PathBuf::from(surumler::DIZIN),
            liste_dizini: PathBuf::from(listeler::DIZIN),
            sms_tasiyici,
            indirme_tasiyici,
            liste_uretici: Uretici::varsayilan(),
            github_al: Box::new(|v, d, t| surumler::github_al(v, d, t)),
        }
    }

    /// Üretim: varsayılan yollar, gerçek kuyruk, paylaşılan geçiş belirteçleri ve müşteri paneli (cihaz adresi), rapor grupları.
    pub fn uretim(veri: Arc<Veri>, saat: Saat, gecis: Arc<GecisBelirtecleri>, musteri: Arc<Musteri>) -> Yonetim {
        Yonetim::new(
            veri,
            Kayitlar::default(),
            saat,
            PathBuf::from(yonetici::YONETICI),
            PathBuf::from(crate::durum::DURUM),
            Box::new(|islem, alanlar| crate::kuyruk::ekle(islem, alanlar, Path::new(crate::kuyruk::KUYRUK))),
        )
        .gecis_ile(gecis, musteri)
        .rapor_gruplari_ile(PathBuf::from(veri::RAPOR_GRUPLARI))
    }

    /// Python `Yonetim(..., gecis=..., musteri_app=...)`.
    pub fn gecis_ile(mut self, gecis: Arc<GecisBelirtecleri>, musteri_app: Arc<Musteri>) -> Yonetim {
        self.gecis = Some(gecis);
        self.musteri_app = Some(musteri_app);
        self
    }

    /// Python `Yonetim(..., rapor_gruplari=...)`.
    pub fn rapor_gruplari_ile(mut self, yol: PathBuf) -> Yonetim {
        self.rapor_gruplari = Some(yol);
        self
    }

    /// Python `Yonetim(..., surum_dizini=...)`.
    pub fn surum_dizini_ile(mut self, d: PathBuf) -> Yonetim {
        self.surum_dizini = d;
        self
    }

    /// Python `Yonetim(..., liste_dizini=...)`.
    pub fn liste_dizini_ile(mut self, d: PathBuf) -> Yonetim {
        self.liste_dizini = d;
        self
    }

    /// Python `Yonetim(..., gonder=...)`: deneme SMS'inin gideceği taşıyıcı.
    pub fn sms_tasiyici_ile(mut self, t: Arc<dyn Tasiyici>) -> Yonetim {
        self.sms_tasiyici = t;
        self
    }

    /// Python `Yonetim(..., getir=..., liste_getir=...)`: GitHub ve liste kaynakları indirmesi.
    pub fn indirme_tasiyici_ile(mut self, t: Arc<dyn Tasiyici>) -> Yonetim {
        self.indirme_tasiyici = t;
        self
    }

    /// Python `L.KAYNAKLAR` değişikliği (testlerde).
    pub fn liste_uretici_ile(mut self, u: Uretici) -> Yonetim {
        self.liste_uretici = u;
        self
    }

    /// Python `Yonetim(..., github_al=...)`.
    pub fn github_al_ile(mut self, f: GithubAl) -> Yonetim {
        self.github_al = f;
        self
    }

    /// Python `Yonetim(..., raporlar=...)`.
    pub fn raporlar_ile(mut self, r: Raporlar) -> Yonetim {
        self.taban = self.taban.raporlar_ile(r);
        self
    }

    // --- yardımcılar ---
    fn veri(&self) -> &Veri {
        &self.taban.veri
    }

    /// durum.json'daki `arsivler` sözlüğü; dosya yoksa ya da bozuksa boş.
    fn durum(&self) -> Sonuc<Map<String, Value>> {
        let ham = match std::fs::read(&self.durum_yolu) {
            Ok(b) => b,
            Err(h) if h.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(h) => return Err(veri_hatasi("durum.json", h)),
        };
        let d: Value = serde_json::from_slice(&ham).unwrap_or(Value::Null);
        Ok(d.get("arsivler").and_then(Value::as_object).cloned().unwrap_or_default())
    }

    /// Bir müşterinin liste satırı bilgileri: (cihaz metni, son gün metni, sorunlu mu).
    fn ozet(&self, c: Option<&Cihaz>, d: &Value) -> (String, String, bool) {
        let simdi = self.taban.simdi();
        let Some(c) = c else {
            return ("cihaz yok".into(), e(veya_tire(tg_deger(d.get("son_gun")))), false);
        };
        let el = d.get("el_sikisma").and_then(Value::as_f64).filter(|v| *v != 0.0);
        let es = raporlar::epoch(&c.son_eslesme).filter(|v| *v != 0.0).or_else(|| raporlar::epoch(&c.baglanma).filter(|v| *v != 0.0)).unwrap_or(0.0);
        let cihaz = if c.durum == "serbest_birakiliyor" {
            "serbest bırakılıyor".to_string()
        } else if el.is_some_and(|el| simdi - el <= CEVRIMICI_SN) {
            "çevrimiçi".to_string()
        } else if !c.son_eslesme.is_empty() {
            format!("son eşitleme {}", once(simdi - es))
        } else {
            "henüz eşitlenmedi".to_string()
        };
        let sg = d.get("son_gun");
        let gecikti = saglik::yedek_gecikti(sg.and_then(Value::as_str), simdi);
        let son = e(veya_tire(tg_deger(sg))) + if gecikti { " <b class=\"kotu\">GECİKTİ</b>" } else { "" };
        let sorunlu = gecikti || simdi - es > ESLESME_GECIKME_SN || c.durum == "serbest_birakiliyor";
        (cihaz, son, sorunlu)
    }

    fn cihaz_adresi(&self, n: i64) -> Sonuc<Option<String>> {
        match (&self.musteri_app, &self.gecis) {
            (Some(m), Some(_)) => Ok(m.cihaz_adresi(n)?.filter(|a| !a.is_empty())),
            _ => Ok(None),
        }
    }

    /// Cihazın son raporundaki kurulu paket sürümü: Some("1.2.0"), Some("") (paketsiz kurulum) ya da None (rapor yok / eski sürüm).
    fn kurulu_paket(&self, c: Option<&Cihaz>) -> Sonuc<Option<String>> {
        let s = self.saglik_kaydi(c)?;
        let p = s.as_ref().and_then(|s| s.rapor.as_object()).and_then(|r| r.get("paket")).and_then(Value::as_str);
        Ok(p.map(str::to_string))
    }

    fn saglik_kaydi(&self, c: Option<&Cihaz>) -> Sonuc<Option<SaglikKaydi>> {
        match c {
            Some(c) => self.veri().saglik(c.id),
            None => Ok(None),
        }
    }

    /// Liste hücresi: kurulu paket; hedeften farklıysa sarı.
    fn paket(&self, m: &veri::Musteri, c: Option<&Cihaz>) -> Sonuc<String> {
        let Some(p) = self.kurulu_paket(c)? else { return Ok("—".into()) };
        if p.is_empty() {
            return Ok("paketsiz".into());
        }
        let hedef = self.veri().cihaz_hedefi(m)?;
        Ok(match hedef {
            Some(h) if h.surum != p => format!("<span class=\"uyarisi\">{}</span>", e(&p)),
            _ => e(&p),
        })
    }

    /// Ayar sayfaları kancası: `/listeler` (+ `/listeler/guncelle`), `/sms`, `/uyari`, `/uyari/deneme`, `/surumler`
    /// (+ `/surumler/al`, `/surumler/hedef`). Eşleşmeyen yol için `Ok(None)`.
    fn ayar_sayfalari(&self, ot: &Ot, yontem: &str, yol: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Option<Yanit>> {
        Ok(Some(match (yontem, yol) {
            (_, "/sms") => self.sms(ot, yontem, form, ip)?,
            ("GET", "/surumler") => self.surumler_sayfasi(ot)?,
            ("POST", "/surumler/al") => self.surum_al(ot, ip)?,
            ("POST", "/surumler/hedef") => self.kanal_hedef(ot, form, ip)?,
            ("GET", "/listeler") => self.listeler_sayfasi(ot),
            ("POST", "/listeler/guncelle") => {
                // web isteği asılı kalmasın: kilit tutuluyorsa beklemeden Mesgul
                match self.liste_uretici.calistir(self.veri(), self.indirme_tasiyici.as_ref(), &self.liste_dizini, self.taban.simdi(), "admin", ip, false) {
                    Ok(_) => yonlendir("/listeler", None),
                    Err(listeler::Mesgul) => {
                        yanit_html(&self.mesaj(Some(ot), "Engel listeleri", "Güncelleme zaten sürüyor; birkaç dakika sonra sayfayı yenileyin."), 200, None)
                    }
                }
            }
            (_, "/uyari") => self.uyari(ot, yontem, form, ip)?,
            ("POST", "/uyari/deneme") => self.uyari_deneme(ot, ip)?,
            _ => return Ok(None),
        }))
    }

    // --- uzaktan güncelleme: sürüm deposu ve kanal hedefleri ---
    fn surumler_sayfasi(&self, ot: &Ot) -> Sonuc<Yanit> {
        let cs = e(&ot.csrf);
        let mut kurulu: HashMap<String, i64> = HashMap::new();
        for c in self.veri().cihazlar()? {
            if let Some(p) = self.kurulu_paket(Some(&c))?.filter(|p| !p.is_empty()) {
                *kurulu.entry(p).or_insert(0) += 1;
            }
        }
        let liste = self.veri().surumler()?;
        let satirlar: String = liste
            .iter()
            .map(|x| {
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    e(&x.surum),
                    e(&x.kaynak),
                    e(tarih_goster(&x.zaman, false)),
                    e(detay::boyut(x.boyut)),
                    e(x.sha256.chars().take(12).collect::<String>()),
                    kurulu.get(&x.surum).copied().unwrap_or(0)
                )
            })
            .collect();
        let tablo = if liste.is_empty() {
            "<p class=\"alt\">Depoda sürüm yok.</p>".to_string()
        } else {
            format!(
                "<div class=\"kaydir\"><table class=\"detay\"><tr><th>Sürüm</th><th>Kaynak</th><th>Tarih</th><th>Boyut</th>\
                 <th>sha256</th><th>Kurulu cihaz</th></tr>{satirlar}</table></div>"
            )
        };
        let al = format!(
            "<form method=\"post\" action=\"/surumler/al\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
             <button>GitHub'dan yeni sürümleri al</button></form>"
        );
        let mut hedefler = String::new();
        let en_son = self.veri().en_son_surum()?;
        for (kanal, ad) in [("kararli", "Kararlı"), ("test", "Test")] {
            let secili = self.veri().kanal_hedef(kanal)?;
            let mut sec = format!(
                "<option value=\"\">Hedef yok</option>\
                 <option value=\"{}\"{}>Her zaman en son sürüm ({})</option>",
                veri::EN_SON,
                if secili.as_deref() == Some(veri::EN_SON) { " selected" } else { "" },
                match &en_son {
                    Some(s) if !s.is_empty() => format!("şu an {}", e(s)),
                    _ => "GitHub sürümü yok".to_string(),
                }
            );
            for x in &liste {
                sec += &format!("<option value=\"{0}\"{1}>{0}</option>", e(&x.surum), if secili.as_deref() == Some(x.surum.as_str()) { " selected" } else { "" });
            }
            hedefler += &format!(
                "<form method=\"post\" action=\"/surumler/hedef\" class=\"ara\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
                 <input type=\"hidden\" name=\"kanal\" value=\"{kanal}\"><label for=\"k-{kanal}\">{ad} kanal hedefi</label>\
                 <select id=\"k-{kanal}\" name=\"surum\">{sec}</select><button>Kaydet</button></form>"
            );
        }
        let icerik = kart(&format!("<h1>Sürümler</h1>{al}{tablo}"))
            + &kart(&format!(
                "<h2>Kanal hedefleri</h2>{hedefler}\
                 <p class=\"alt\">Cihazlar kendi kanalının hedefini bir sonraki sağlık raporunda (5 dk) \
                 öğrenir ve kurar; hedef yoksa o kanaldaki cihazlar güncellenmez. \
                 <b>Her zaman en son sürüm</b>: GitHub'dan alınmış en yüksek sürüm; merkez GitHub'a \
                 saatte bir bakıp yeni sürümü kendisi alır. Elle eklenen (yerel) paketler buna \
                 girmez, onları kanalda ayrıca seçin.</p>"
            ));
        Ok(yanit_html(&self.sayfa("Sürümler", &icerik, Some(ot), true, None), 200, None))
    }

    fn surum_al(&self, ot: &Ot, ip: &str) -> Sonuc<Yanit> {
        let (eklenen, hatalar) = match (self.github_al)(self.veri(), &self.surum_dizini, self.indirme_tasiyici.as_ref()) {
            Ok(r) => r,
            Err(h) => {
                // liste alınamadı: sayfa düşmez, mesaj gösterilir
                self.veri().hareket("admin", ip, "SURUM_AL_HATA", None, &h.chars().take(200).collect::<String>())?;
                return Ok(yanit_html(&self.mesaj(Some(ot), "Sürümler alınamadı", &format!("GitHub sürüm listesi alınamadı: {h}")), 200, None));
            }
        };
        for s in &eklenen {
            self.veri().hareket("admin", ip, "SURUM_ALINDI", None, &format!("surum={s}"))?;
        }
        let mut metin = if eklenen.is_empty() { "Yeni sürüm yok. ".to_string() } else { format!("Eklenen: {}. ", eklenen.join(", ")) };
        if !hatalar.is_empty() {
            metin += &format!("Alınamayanlar: {}", hatalar.join("; "));
        }
        Ok(yanit_html(&self.mesaj(Some(ot), "Sürümler", py_strip(&metin)), 200, None))
    }

    fn kanal_hedef(&self, ot: &Ot, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let (kanal, sv) = (form.get("kanal").map(String::as_str).unwrap_or(""), form.get("surum").map(String::as_str).unwrap_or(""));
        let gecersiz = !["kararli", "test"].contains(&kanal)
            || (!sv.is_empty() && sv != veri::EN_SON && (!surumler::surum_gecerli(sv) || self.veri().surum(sv)?.is_none()));
        if gecersiz {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Hedef kaydedilmedi", "Kanal ya da sürüm geçersiz (sürüm depoda olmalı)."), 400, None));
        }
        self.veri().kanal_hedef_koy(kanal, if sv.is_empty() { None } else { Some(sv) })?;
        self.veri().hareket("admin", ip, "KANAL_HEDEF", None, &format!("kanal={kanal} surum={}", if sv.is_empty() { "-" } else { sv }))?;
        Ok(yonlendir("/surumler", None))
    }

    // --- hazır engel listeleri ---
    fn listeler_sayfasi(&self, ot: &Ot) -> Yanit {
        let mut satirlar = String::new();
        for (k, ad) in listeler::KATEGORILER {
            let m = listeler::meta_oku(&self.liste_dizini, k).unwrap_or_default();
            let kaynak = m
                .get("kaynaklar")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_object)
                        .map(|d| {
                            let sayi = d.get("sayi").map(py_str).unwrap_or_else(|| "0".into());
                            let durum = if d.get("hata").is_some_and(py_dogru) {
                                format!("<b class=\"kotu\">hata</b> ({})", e(py_str(&d["hata"])))
                                    + &if d.get("eski").is_some_and(py_dogru) { format!(" · son başarılı kopya {}", e(&sayi)) } else { String::new() }
                            } else {
                                e(&sayi)
                            };
                            format!("{}: {durum}", e(d.get("ad").map(py_str).unwrap_or_else(|| "?".into())))
                        })
                        .collect::<Vec<_>>()
                        .join("<br>")
                })
                .unwrap_or_default();
            let mut durum: Vec<String> = Vec::new();
            if m.get("hata").is_some_and(py_dogru) {
                durum.push(format!("<b class=\"kotu\">{}</b> ({})", e(py_str(&m["hata"])), e(tg_deger(m.get("hata_zaman")))));
            }
            if m.get("kesildi").is_some_and(py_dogru) {
                durum.push(format!("<span class=\"uyarisi\">{} sınırında kesildi</span>", self.liste_uretici.en_cok));
            }
            let sha: String = m.get("sha256").map(py_str).unwrap_or_default().chars().take(8).collect();
            let zaman = tg_deger(m.get("zaman"));
            satirlar += &format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                e(ad),
                e(m.get("sayi").map(py_str).unwrap_or_else(|| "—".into())),
                e(if zaman.is_empty() { "henüz üretilmedi" } else { &zaman }),
                if sha.is_empty() { "—".to_string() } else { e(&sha) },
                if kaynak.is_empty() { "—" } else { &kaynak },
                if durum.is_empty() { "tamam".to_string() } else { durum.join("<br>") }
            );
        }
        let tablo = format!(
            "<div class=\"kaydir\"><table class=\"detay\"><tr><th>Kategori</th><th>Alan adı</th><th>Son güncelleme</th>\
             <th>Sürüm</th><th>Kaynaklar</th><th>Durum</th></tr>{satirlar}</table></div>"
        );
        let dugme = format!(
            "<form method=\"post\" action=\"/listeler/guncelle\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <button>Şimdi güncelle</button></form>",
            e(&ot.csrf)
        );
        let icerik = kart(&format!(
            "<h1>Engel listeleri</h1>{dugme}{tablo}\
             <p class=\"alt\">Listeler her gün 04:30'da kaynaklardan yeniden üretilir; cihazlar değişen listeyi bir sonraki \
             sağlık raporunda (5 dk) alır. Bir kaynak indirilemezse onun son başarılı kopyası, hiçbiri indirilemezse önceki \
             liste kullanılır.</p>"
        ));
        yanit_html(&self.sayfa("Engel listeleri", &icerik, Some(ot), true, None), 200, None)
    }

    // --- SMS ve uyarı ayarları ---
    /// NetGSM (eski cihazlara da gider), doğrulama SMS'i için Türk numarası sağlayıcısı ve Twilio (yalnızca merkezde).
    /// Sır alanları (NetGSM şifresi, Twilio auth token / API secret) yalnızca yazılır; boş bırakılırsa eskisi kalır.
    fn sms(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let al = |k: &str| form.get(k).map(String::as_str).unwrap_or("").to_string();
        let mut eski = self.veri().sms_ayari()?.unwrap_or_else(|| {
            json!({"mock": true, "usercode": "", "password": "", "msgheader": "", "appkey": "", "tr_saglayici": "netgsm", "twilio": veri::twilio_varsayilan()})
        });
        let mut hata = String::new();
        if yontem == "POST" {
            let et = eski["twilio"].clone();
            let mut tw = Map::new();
            tw.insert("acik".into(), Value::Bool(al("twilio_acik") == "1"));
            for k in veri::TWILIO_ALANLARI.iter().filter(|k| !veri::TWILIO_SIRLARI.contains(k)) {
                tw.insert((*k).into(), Value::String(al(k)));
            }
            for k in veri::TWILIO_SIRLARI {
                let girilen = al(k);
                tw.insert(k.into(), if girilen.is_empty() { et[k].clone() } else { Value::String(girilen) });
            }
            let parola = if al("password").is_empty() { eski["password"].clone() } else { Value::String(al("password")) };
            let tr = form.get("tr_saglayici").cloned().unwrap_or_else(|| "netgsm".into());
            let yeni = json!({"mock": al("mock") == "1", "usercode": al("usercode"), "msgheader": al("msgheader"), "appkey": al("appkey"),
                              "password": parola, "tr_saglayici": tr, "twilio": Value::Object(tw.clone())});
            match self.veri().sms_ayari_koy(&yeni) {
                Ok(()) => {
                    let k = self.veri().sms_ayari()?.ok_or_else(|| Hata::Db("sms ayarı yazıldı ama okunamadı".into()))?;
                    let mut degisen: Vec<String> =
                        ["mock", "usercode", "msgheader", "appkey", "password", "tr_saglayici"].iter().filter(|x| k[**x] != eski[**x]).map(|x| x.to_string()).collect();
                    if let Some(kt) = k["twilio"].as_object() {
                        degisen.extend(kt.iter().filter(|(x, v)| et.get(x.as_str()) != Some(*v)).map(|(x, _)| format!("twilio_{x}")));
                    }
                    self.veri().hareket("admin", ip, "SMS_AYARI", None, &format!("değişen={}", degisen.join(",")))?; // değer yazılmaz
                    return Ok(yonlendir("/sms", None));
                }
                Err(Hata::Deger(m)) => hata = m,
                Err(h) => return Err(h),
            }
            // hatalı formda yazılanlar geri gösterilir; sırlar asla
            let mut gosterim = yeni.clone();
            gosterim["password"] = eski["password"].clone();
            for x in veri::TWILIO_SIRLARI {
                tw.insert(x.into(), et[x].clone());
            }
            gosterim["twilio"] = Value::Object(tw);
            eski = gosterim;
        }
        let cs = e(&ot.csrf);
        let t = &eski["twilio"];
        let alan = |v: &Value, k: &str| v.get(k).map(py_str).unwrap_or_default();
        let tanim = |v: &Value| if py_dogru(v) { "tanımlı" } else { "tanımsız" };
        let secili = |d: &str| if alan(&eski, "tr_saglayici") == d { " selected" } else { "" };
        let h = if hata.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", e(&hata)) };
        let icerik = format!(
            "<h1>SMS ayarları</h1>{h}\
             <form method=\"post\" action=\"/sms\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
             <label><input type=\"checkbox\" name=\"mock\" value=\"1\"{}> Deneme modu (SMS gönderilmez)</label>\
             <label for=\"tr\">Türk numaraları için sağlayıcı</label><select id=\"tr\" name=\"tr_saglayici\">\
             <option value=\"netgsm\"{}>NetGSM</option><option value=\"twilio\"{}>Twilio</option></select>\
             <h2>NetGSM</h2>\
             <label for=\"u\">NetGSM kullanıcı kodu</label><input id=\"u\" name=\"usercode\" value=\"{}\" autocomplete=\"off\">\
             <label for=\"p\">API şifresi ({})</label><input id=\"p\" name=\"password\" type=\"password\" autocomplete=\"new-password\" \
             placeholder=\"değiştirmek için yazın (boş = aynı kalır)\">\
             <label for=\"b\">Mesaj başlığı</label><input id=\"b\" name=\"msgheader\" value=\"{}\" maxlength=\"11\">\
             <label for=\"a\">Uygulama anahtarı (opsiyonel)</label><input id=\"a\" name=\"appkey\" value=\"{}\">\
             <h2>Twilio Verify</h2>\
             <label><input type=\"checkbox\" name=\"twilio_acik\" value=\"1\"{}> Twilio açık \
             (yabancı numaralar Twilio'dan gider)</label>\
             <label for=\"ta\">Account SID</label><input id=\"ta\" name=\"account_sid\" value=\"{}\" autocomplete=\"off\">\
             <label for=\"tt\">Auth token ({})</label><input id=\"tt\" name=\"auth_token\" type=\"password\" \
             autocomplete=\"new-password\" placeholder=\"değiştirmek için yazın (boş = aynı kalır)\">\
             <label for=\"tk\">API key SID (opsiyonel)</label><input id=\"tk\" name=\"api_key_sid\" value=\"{}\" autocomplete=\"off\">\
             <label for=\"ts\">API key secret ({})</label><input id=\"ts\" name=\"api_key_secret\" type=\"password\" \
             autocomplete=\"new-password\" placeholder=\"değiştirmek için yazın (boş = aynı kalır)\">\
             <label for=\"tv\">Verify service SID</label><input id=\"tv\" name=\"verify_sid\" value=\"{}\" autocomplete=\"off\">\
             <button>Kaydet</button></form>\
             <p class=\"alt\">Doğrulama SMS'leri cihazların isteğiyle bu sunucudan gönderilir (değişiklik hemen geçerli). \
             NetGSM bilgileri eski sürümlü cihazlara bir sonraki eşitlemede (her gün 06:00) de gider; Twilio bilgileri hiçbir cihaza gitmez.</p>",
            if py_dogru(&eski["mock"]) { " checked" } else { "" },
            secili("netgsm"),
            secili("twilio"),
            e(alan(&eski, "usercode")),
            tanim(&eski["password"]),
            e(alan(&eski, "msgheader")),
            e(alan(&eski, "appkey")),
            if py_dogru(&t["acik"]) { " checked" } else { "" },
            e(alan(t, "account_sid")),
            tanim(&t["auth_token"]),
            e(alan(t, "api_key_sid")),
            tanim(&t["api_key_secret"]),
            e(alan(t, "verify_sid"))
        );
        Ok(yanit_html(&self.sayfa("SMS ayarları", &kart(&icerik), Some(ot), false, None), if hata.is_empty() { 200 } else { 400 }, None))
    }

    /// Uyarı SMS'i alıcıları ve günlük sınır. Telefonlar 0533… ya da 533… yazılabilir; veri katmanı doğrular.
    fn uyari(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let mut a = self.veri().uyari_ayari()?;
        let mut hata = String::new();
        if yontem == "POST" {
            let tel: Vec<String> = (1..=3).map(|i| py_strip(form.get(&format!("tel{i}")).map(String::as_str).unwrap_or("")).to_string()).collect();
            let acik = form.get("acik").map(String::as_str) == Some("1");
            let ham = py_strip(form.get("gunluk_sinir").map(String::as_str).unwrap_or("")).to_string();
            match py_int(&ham).filter(|n| (SINIR_ARALIK.0..=SINIR_ARALIK.1).contains(n)) {
                None => hata = format!("Günlük SMS sınırı {} ile {} arasında bir sayı olmalı.", SINIR_ARALIK.0, SINIR_ARALIK.1),
                Some(sinir) => {
                    let dolu: Vec<&String> = tel.iter().filter(|t| !t.is_empty()).collect();
                    match self.veri().uyari_ayari_koy(&json!({"telefonlar": dolu, "acik": acik, "gunluk_sinir": sinir})) {
                        Ok(()) => {
                            // telefon yazılmaz
                            self.veri().hareket("admin", ip, "UYARI_AYAR", None, &format!("telefon={} acik={} sinir={sinir}", dolu.len(), acik as i32))?;
                            return Ok(yonlendir("/uyari", None));
                        }
                        Err(Hata::Deger(m)) => hata = m,
                        Err(h) => return Err(h),
                    }
                }
            }
            a = json!({"telefonlar": tel, "acik": acik, "gunluk_sinir": ham}); // yazılanı geri göster
        }
        let mut tel: Vec<String> = a["telefonlar"].as_array().map(|l| l.iter().map(py_str).collect()).unwrap_or_default();
        tel.extend(std::iter::repeat_n(String::new(), 3));
        let alanlar: String = tel
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, t)| {
                let i = i + 1;
                let goster = if t.starts_with("90") && t.chars().count() == 12 { format!("0{}", t.chars().skip(2).collect::<String>()) } else { t.clone() };
                format!(
                    "<label for=\"t{i}\">Telefon {i}</label><input id=\"t{i}\" name=\"tel{i}\" value=\"{}\" inputmode=\"tel\" autocomplete=\"off\" \
                     placeholder=\"05XX XXX XX XX\">",
                    e(goster)
                )
            })
            .collect();
        let h = if hata.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", e(&hata)) };
        let cs = e(&ot.csrf);
        let icerik = format!(
            "<h1>Uyarı ayarları</h1>{h}<form method=\"post\" action=\"/uyari\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
             {alanlar}<label><input type=\"checkbox\" name=\"acik\" value=\"1\"{}> SMS uyarıları açık</label>\
             <label for=\"s\">Günlük SMS sınırı ({}–{})</label>\
             <input id=\"s\" name=\"gunluk_sinir\" value=\"{}\" inputmode=\"numeric\" autocomplete=\"off\">\
             <button>Kaydet</button></form>\
             <form method=\"post\" action=\"/uyari/deneme\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
             <button class=\"ikincil\">Deneme SMS'i gönder</button></form>\
             <p class=\"alt\">Cihaz sağlık sorunları (ulaşılamıyor, kayıt durdu, servis durdu, disk, lisans…) bu numaralara SMS olarak \
             gider. Deneme SMS'i ilk telefona gider ve günlük sayaca girer. Gönderim için SMS ayarları sayfasında NetGSM bilgileri \
             girilmiş olmalı; deneme modu açıksa SMS gitmez.</p>",
            if py_dogru(&a["acik"]) { " checked" } else { "" },
            SINIR_ARALIK.0,
            SINIR_ARALIK.1,
            e(py_str(&a["gunluk_sinir"]))
        );
        Ok(yanit_html(&self.sayfa("Uyarı ayarları", &kart(&icerik), Some(ot), false, None), if hata.is_empty() { 200 } else { 400 }, None))
    }

    fn uyari_deneme(&self, ot: &Ot, ip: &str) -> Sonuc<Yanit> {
        let (ayar, a) = (self.veri().sms_ayari()?, self.veri().uyari_ayari()?);
        let Some(ayar) = ayar else {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Deneme SMS'i gitmedi", "SMS ayarları girilmemiş. Önce SMS ayarları sayfasını doldurun."), 200, None));
        };
        let telefonlar: Vec<String> = a["telefonlar"].as_array().map(|l| l.iter().map(py_str).collect()).unwrap_or_default();
        let Some(ilk) = telefonlar.first() else {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Deneme SMS'i gitmedi", "Uyarı ayarlarında kayıtlı telefon yok."), 200, None));
        };
        let r = netgsm::gonder(&ayar, DENEME_METNI, std::slice::from_ref(ilk), self.sms_tasiyici.as_ref());
        self.veri().sms_say(&now_iso(self.taban.simdi())[..10])?;
        self.veri().hareket("admin", ip, "UYARI_DENEME", None, &format!("sonuç={r}"))?;
        let ne = match r.as_str() {
            "gonderildi" => "Deneme SMS'i gönderildi.".to_string(),
            "mock" => "Deneme modu açık: SMS gönderilmedi, yalnızca kaydedildi.".to_string(),
            d => format!("Deneme SMS'i gönderilemedi ({d})."),
        };
        Ok(yanit_html(&self.mesaj(Some(ot), "Deneme SMS'i", &ne), 200, None))
    }

    // --- sayfalar ---
    fn bekleyen_sayfasi(&self, ot: &Ot) -> Sonuc<Yanit> {
        let simdi = self.taban.simdi();
        let mut satirlar = String::new();
        for r in self.veri().bekleyenler()? {
            let mut son = e(tarih_goster(&now_iso(r.son), true));
            if simdi - r.son > veri::BEKLEYEN_ESKI_SN {
                son += " <span class=\"uyarisi\">eski</span>";
            }
            let durum = if r.cihaz.is_some() {
                format!("bağlandı: <a href=\"/m/{0}\">{0}</a>", r.musteri.map(|m| m.to_string()).unwrap_or_else(|| "None".into()))
            } else if let Some(m) = r.musteri {
                let gecti = simdi - r.ayrilma > veri::AYRILMA_SN;
                format!("ayrıldı: <a href=\"/m/{m}\">{m}</a>") + if gecti { " <span class=\"alt\">(süresi doldu)</span>" } else { "" }
            } else {
                "bekliyor".into()
            };
            let kimlik: String = r.kimlik.chars().take(8).collect();
            satirlar += &format!(
                "<tr><td>{}</td><td>{}</td><td>{son}</td><td>{}</td><td>{}</td><td>{}</td><td>{durum}</td></tr>",
                e(&kimlik),
                e(tarih_goster(&now_iso(r.ilk), false)),
                e(&r.ip),
                e_veya(&r.surum),
                e_veya(&r.wan_mac)
            );
        }
        let tablo = if satirlar.is_empty() {
            "<p class=\"alt\">Bağlanmayı bekleyen cihaz yok.</p>".to_string()
        } else {
            format!(
                "<div class=\"kaydir\"><table class=\"detay\"><tr><th>Kimlik</th><th>İlk görülme</th><th>Son görülme</th>\
                 <th>İnternet adresi</th><th>Sürüm</th><th>WAN MAC</th><th>Durum</th></tr>{satirlar}</table></div>"
            )
        };
        let aciklama = "<p class=\"alt\">Bağlanmamış cihazlar internete çıkınca 2 dakikada bir bildirir. Müşteri cihazla aynı internet \
                        adresinden panele girince cihaz kendiliğinden bağlanır. 30 dakikadır bildirmeyen \"eski\", 7 gün bildirmeyen silinir.</p>";
        Ok(yanit_html(&self.sayfa("Bekleyen cihazlar", &(kart(&format!("<h1>Bekleyen cihazlar</h1>{aciklama}")) + &kart(&tablo)), Some(ot), true, None), 200, None))
    }

    fn liste(&self, ot: &Ot, sorgu: &str) -> Sonuc<Yanit> {
        let q = tr_kucuk(py_strip(&sorgu_ilk(sorgu, "q").unwrap_or_default()));
        let bitenler = sorgu_ilk(sorgu, "bitenler").as_deref() == Some("1"); // üyeliği bitenler varsayılan gizli (silinmez); aramada hep çıkar
        let durum = self.durum()?;
        let cihazlar: HashMap<i64, Cihaz> = self.veri().cihazlar()?.into_iter().map(|c| (c.musteri, c)).collect();
        let gunler: Vec<String> = son_30_gun(self.taban.simdi()).iter().take(7).map(|g| g.format("%Y-%m-%d").to_string()).collect();
        let acik = self.veri().acik_sorunlar(None)?;
        let mut kritik: HashMap<i64, Vec<String>> = HashMap::new();
        for x in &acik {
            if x.onem == "kritik" {
                kritik.entry(x.musteri).or_default().push(x.metin.clone());
            }
        }
        let musteriler = self.veri().musteriler()?;
        let biten = musteriler.iter().filter(|m| m.uyelik != "aktif").count();
        let mut satirlar: Vec<(bool, bool, i64, String)> = Vec::new();
        for m in &musteriler {
            if m.uyelik != "aktif" && !(bitenler || !q.is_empty()) {
                continue;
            }
            let (n, c) = (m.numara, cihazlar.get(&m.numara));
            let paket = self.paket(m, c)?;
            let ad = c.map(|c| c.isletme_adi.as_str()).unwrap_or("");
            let tur = isletme_turu(c);
            if !n.to_string().contains(&q) && !tr_kucuk(ad).contains(&q) && !tr_kucuk(&m.not_).contains(&q) {
                continue;
            }
            let d = durum.get(&n.to_string()).cloned().unwrap_or(Value::Null);
            let (cihaz, son, sorunlu) = self.ozet(c, &d);
            let cihaz = e(&cihaz) + &kritik.get(&n).map(|l| l.iter().map(|t| format!(" <b class=\"kotu\">{}</b>", e(t))).collect::<String>()).unwrap_or_default(); // kritik sorun rozeti
            let sorunlu = sorunlu || kritik.contains_key(&n);
            let sayilar = self.taban.kayitlar.sayilar(&self.veri().arsivler(n)?).0;
            let yedi = gunler.iter().rev().map(|g| sayilar.get(g).map(py_str).unwrap_or_else(|| "—".into())).collect::<Vec<_>>().join(" ");
            let uyelik = if m.uyelik == "aktif" { "Aktif".to_string() } else { format!("Bitti {}", tarih_goster(&m.bitis.chars().take(10).collect::<String>(), false)) };
            let (ldurum, lbitis) = self.veri().lisans_durumu(m);
            let lbitis = tarih_goster(&lbitis, false);
            let lisans = match ldurum {
                "aktif" => e(&lbitis),
                "bitti" => format!("<b class=\"kotu\">süresi doldu {}</b>", e(&lbitis)),
                _ => format!("<b class=\"kotu\">askıda</b> {}", e(&lbitis)),
            };
            let sorunlu = sorunlu || ldurum != "aktif";
            satirlar.push((
                m.uyelik != "aktif",
                !sorunlu,
                n, // üyeliği bitenler en altta
                format!(
                    "<tr><td><a href=\"/m/{n}\">{n}</a></td><td>{}<br><span class=\"alt\">{}</span></td>\
                     <td>{}</td><td>{cihaz}</td><td>{paket}</td><td>{son}</td><td>{}</td><td>{lisans}</td><td>{}</td></tr>",
                    e_veya(ad),
                    e(&m.not_),
                    e(&tur),
                    e(&yedi),
                    e(&uyelik)
                ),
            ));
        }
        satirlar.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
        let tablo = if satirlar.is_empty() {
            "<p class=\"alt\">Müşteri yok.</p>".to_string()
        } else {
            format!(
                "<div class=\"kaydir\"><table class=\"detay\"><tr><th>No</th><th>İşletme / not</th><th>Tür</th><th>Cihaz</th>\
                 <th>Sürüm</th><th>Son gelen gün</th><th>Son 7 gün</th><th>Lisans</th><th>Üyelik</th></tr>{}</table></div>",
                satirlar.iter().map(|s| s.3.as_str()).collect::<String>()
            )
        };
        let ara = format!(
            "<form class=\"ara\" method=\"get\" action=\"/\"><input name=\"q\" value=\"{}\" placeholder=\"Numara, işletme adı ya da not\">\
             <button>Ara</button></form>",
            e(&q)
        );
        let mut gizli = String::new();
        if biten > 0 && q.is_empty() {
            gizli = if bitenler {
                format!("<p class=\"alt\">Üyeliği biten {biten} müşteri de listede (en altta). <a href=\"/\">Gizle</a></p>")
            } else {
                format!(
                    "<p class=\"alt\">Üyeliği biten {biten} müşteri listede gösterilmiyor; aramada görünür. \
                     <a href=\"/?bitenler=1\">Göster</a></p>"
                )
            };
        }
        let icerik = self.acik_sorunlar_karti(&acik, &cihazlar)? + &kart(&format!("<h1>Müşteriler</h1>{}{ara}{gizli}", self.izleme_satiri()?)) + &kart(&tablo);
        Ok(yanit_html(&self.sayfa("Müşteriler", &icerik, Some(ot), true, None), 200, None))
    }

    /// wc-saglik'in son turu: hep görünür; 15 dk'dan eskiyse ya da hiç yoksa kırmızı (zamanlayıcı durmuş olabilir).
    fn izleme_satiri(&self) -> Sonuc<String> {
        let Some(t) = self.veri().saglik_son_tur()?.and_then(|z| raporlar::epoch(&z)) else {
            return Ok("<p><b class=\"kotu\">Sağlık izleme: hiç tur yok — izleme çalışmıyor olabilir</b></p>".into());
        };
        let sn = (self.taban.simdi() - t).max(0.0);
        let metin = format!("Sağlık izleme: son tur {}", once(sn));
        if sn > IZLEME_BAYAT_SN {
            return Ok(format!("<p><b class=\"kotu\">{} — izleme çalışmıyor olabilir</b></p>", e(&metin)));
        }
        Ok(format!("<p class=\"alt\">{}</p>", e(&metin)))
    }

    fn acik_sorunlar_karti(&self, acik: &[Sorun], cihazlar: &HashMap<i64, Cihaz>) -> Sonuc<String> {
        if acik.is_empty() {
            return Ok(String::new());
        }
        let simdi = self.taban.simdi();
        let mut satirlar = String::new();
        for x in acik {
            let sn = simdi - raporlar::epoch(&x.acilis).filter(|v| *v != 0.0).unwrap_or(simdi);
            let bas = if x.musteri == saglik::MERKEZ {
                // müşteriye bağlı olmayan sorun (ör. engel listeleri)
                "<td>—</td><td><a href=\"/listeler\">Merkez</a></td>".to_string()
            } else {
                format!(
                    "<td><a href=\"/m/{0}\">{0}</a></td><td>{1}</td>",
                    x.musteri,
                    e(cihazlar.get(&x.musteri).map(|c| c.isletme_adi.as_str()).unwrap_or(""))
                )
            };
            satirlar += &format!(
                "<tr>{bas}<td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                e(&x.metin),
                e(onem_etiketi(&x.onem)),
                e(once(sn)),
                e(sms_etiketi(&x.acilis_sms, &x.onem))
            );
        }
        Ok(kart(&format!(
            "<h2>Açık sorunlar</h2><div class=\"kaydir\"><table class=\"detay\"><tr><th>No</th><th>İşletme</th><th>Sorun</th>\
             <th>Önem</th><th>Ne zamandır</th><th>SMS</th></tr>{satirlar}</table></div>"
        )))
    }

    fn yeni(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        if yontem == "GET" {
            let h = kart(&format!(
                "<h1>Yeni müşteri</h1><form method=\"post\" action=\"/yeni\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"{}\">\
                 <label for=\"n\">Not (isteğe bağlı, yalnızca siz görürsünüz)</label>\
                 <input id=\"n\" name=\"not\" maxlength=\"200\" placeholder=\"ör. Ahmet Bey, Kadıköy\">{}<button>Oluştur</button></form>",
                e(&ot.csrf),
                parola_alanlari("Müşteri parolası")
            ));
            return Ok(yanit_html(&self.sayfa("Yeni müşteri", &h, Some(ot), false, None), 200, None));
        }
        let pw = match parola_formdan(form) {
            Ok(p) => p,
            Err(hata) => return Ok(yanit_html(&self.mesaj(Some(ot), "Müşteri açılmadı", &hata), 400, None)),
        };
        let (n, pw) = self.veri().musteri_ekle(form.get("not").map(String::as_str).unwrap_or(""), Some(&pw))?;
        self.veri().hareket("admin", ip, "MUSTERI_ACILDI", Some(n), "")?;
        let lisans = self.veri().musteri(n)?.map(|m| m.lisans_bitis).unwrap_or_default();
        let h = kart(&format!(
            "<h1>Müşteri açıldı</h1><p>Müşteri numarası</p><p class=\"sir\">{n}</p><p>Parola</p><p class=\"sir\">{}</p>\
             <p>Lisans bitişi: {}</p>\
             <p class=\"alt\">Parolayı müşteriye iletin (müşteri sayfasında da görünür). Müşteri cihazın giriş ekranına bu numara ve \
             parolayla girer.</p><p><a href=\"/m/{n}\">Müşteri sayfası →</a></p>",
            e(&pw),
            e(tarih_goster(&lisans, false))
        ));
        Ok(yanit_html(&self.sayfa("Müşteri açıldı", &h, Some(ot), false, None), 200, None))
    }

    /// Müşteri parolası girmeden cihaz.wificorrect.com'a: tek kullanımlık 60 sn'lik geçiş belirteci (yönetici rolü).
    fn cihaz_paneli(&self, ot: &Ot, n: i64, ip: &str) -> Sonuc<Yanit> {
        let gecis = match (self.cihaz_adresi(n)?, &self.gecis) {
            (Some(_), Some(g)) => g,
            _ => {
                return Ok(yanit_html(&self.mesaj(Some(ot), "Cihaza ulaşılamıyor", "Cihaz kapalı, tünel yok ya da lisans/üyelik kapalı."), 409, None));
            }
        };
        let ozet = self.veri().musteri(n)?.map(|m| m.ozet).unwrap_or_default();
        let t = gecis.uret(&n.to_string(), self.taban.simdi(), &ozet, "yonetici");
        self.veri().hareket("admin", ip, "YONETICI_CIHAZ_PANELI", Some(n), "")?;
        Ok(yonlendir(&format!("{CIHAZ_GIRIS}{t}"), None))
    }

    fn guncelleme_kanali(&self, ot: &Ot, n: i64, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let kanal = form.get("kanal").map(String::as_str).unwrap_or("");
        if !veri::GUNCELLEME_KANALLARI.contains(&kanal) {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Kanal kaydedilmedi", "Geçersiz kanal."), 400, None));
        }
        self.veri().guncelleme_kanali_koy(n, kanal)?;
        self.veri().hareket("admin", ip, "GUNCELLEME_KANALI", Some(n), &format!("kanal={kanal}"))?;
        Ok(musteri_donus(n, "guncelleme"))
    }

    fn guncelleme_hemen(&self, ot: &Ot, n: i64, ip: &str) -> Sonuc<Yanit> {
        let Some(m) = self.veri().musteri(n)? else {
            return Err(Hata::Db(format!("müşteri yok: {n}")));
        };
        let Some(hedef) = self.veri().cihaz_hedefi(&m)? else {
            // kanal kapalı, kanalda hedef yok ya da hizmet kapalı: bayrak konmaz (sonuç gelmez, kalırdı)
            return Ok(yanit_html(
                &self.mesaj(Some(ot), "Güncelleme başlatılmadı", "Bu müşterinin kanalında hedef sürüm yok, kanal kapalı ya da hizmeti kapalı."),
                400,
                None,
            ));
        };
        self.veri().guncelleme_hemen_koy(n, self.taban.simdi(), &hedef.surum)?;
        self.veri().hareket("admin", ip, "GUNCELLEME_HEMEN", Some(n), "")?;
        Ok(musteri_donus(n, "guncelleme"))
    }

    /// Dönen: (bölüm başlığının yanındaki kısa durum, bölümün içeriği).
    fn guncelleme_bolumu(&self, m: &veri::Musteri, c: Option<&Cihaz>, cs: &str) -> Sonuc<(String, String)> {
        let n = m.numara;
        let p = self.kurulu_paket(c)?;
        let kurulu = match &p {
            None => "—".to_string(),
            Some(p) if p.is_empty() => "Paketsiz kurulum — uzaktan güncellenmez".to_string(),
            Some(p) => e(p),
        };
        let hedef = self.veri().cihaz_hedefi(m)?;
        let kanal = match m.guncelleme_kanali.as_str() {
            "kararli" => "kararlı",
            "test" => "test",
            "kapali" => "kapalı",
            d => d,
        };
        let hedef_sv = hedef.as_ref().map(|h| e(&h.surum)).unwrap_or_else(|| "—".into());
        let hemen = hedef.as_ref().is_some_and(|h| h.hemen);
        let ozet = format!(
            "kanal {} · kurulu {} · hedef {hedef_sv}{}",
            e(kanal),
            match &p {
                None => "—".to_string(),
                Some(p) if p.is_empty() => "paketsiz".to_string(),
                Some(p) => e(p),
            },
            if hemen { " · <b class=\"uyarisi\">şimdi güncelle bekleniyor</b>" } else { "" }
        );
        let s = self.saglik_kaydi(c)?;
        let g = s.as_ref().and_then(|s| s.rapor.as_object()).and_then(|r| r.get("guncelleme")).and_then(Value::as_object);
        let son = match g {
            Some(g) => {
                let zaman = match g.get("zaman") {
                    Some(Value::Number(z)) if z.as_f64().is_some_and(|f| f > 0.0) => tarih_goster(&now_iso(z.as_f64().unwrap_or(0.0)), false),
                    _ => String::new(),
                };
                let etiket = match g.get("sonuc") {
                    Some(Value::String(t)) => match t.as_str() {
                        "tamam" => "Tamam".to_string(),
                        "geri_alindi" => "Geri alındı".to_string(),
                        "hata" => "Hata".to_string(),
                        d => d.to_string(),
                    },
                    Some(v) => py_str(v),
                    None => "?".into(),
                };
                format!("{} · <b>{}</b> · {}", e(g.get("surum").map(py_str).unwrap_or_else(|| "?".into())), e(&etiket), e(&zaman))
            }
            None => "—".into(),
        };
        let sec: String = [("kararli", "Kararlı"), ("test", "Test"), ("kapali", "Kapalı (güncellenmez)")]
            .iter()
            .map(|(k, ad)| format!("<option value=\"{k}\"{}>{ad}</option>", if *k == m.guncelleme_kanali { " selected" } else { "" }))
            .collect();
        let bekliyor = if hemen { "<b class=\"uyarisi\">bekleniyor</b>" } else { "—" };
        let satirlar: Vec<Vec<String>> = self
            .veri()
            .hareketler(Some(n), 10, Some("GUNCELLEME_SONUC"))?
            .iter()
            .map(|h| vec![tarih_goster(&h.zaman, false), h.ayrinti.clone()])
            .collect();
        let gecmis = detay::tablo(&["Zaman", "Ayrıntı"], &satirlar, "Güncelleme denemesi yok.");
        let icerik = bilgi_tablosu(&[
            ("Kurulu paket", kurulu),
            ("Hedef sürüm", hedef_sv.clone()),
            ("Son sonuç", son),
            ("Şimdi güncelle", bekliyor.to_string()),
        ]) + "<div class=\"dugmeler\">"
            + &format!(
                "<form method=\"post\" action=\"/m/{n}/guncelleme\" class=\"ara\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
                 <select name=\"kanal\" aria-label=\"Güncelleme kanalı\">{sec}</select><button>Kanalı kaydet</button></form>"
            )
            + &format!(
                "<form method=\"post\" action=\"/m/{n}/guncelleme-hemen\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
                 <button>Şimdi güncelle</button></form></div>\
                 <p class=\"alt\">Cihaz hedefi sağlık raporunda (5 dk) görür; \"şimdi güncelle\" bekleme penceresini atlar \
                 (önce Sürümler'de kanalın hedefini seçin).</p>"
            )
            + &acilir("Geçmiş", &gecmis, "");
        Ok((ozet, icerik))
    }

    /// Üstte hep görünen özet (cihaz, lisans, sürüm ve ortak menü: cihaz paneli, kayıtlar, raporlar, hareketler); ayrıntılar
    /// açılır bölümlerde (kullanıcı isteği 2026-10-26: sayfa çok karmaşıktı). Uzun açıklamalar, geçmiş tabloları ve tehlikeli
    /// işlemler bölümün içinde ayrıca tıklayınca açılır. İşlemden dönüşte (?acik=) o bölüm açık gelir.
    fn musteri_sayfasi(&self, ot: &Ot, n: i64, sorgu: &str) -> Sonuc<Yanit> {
        let v = self.veri();
        let m = v.musteri(n)?.ok_or_else(|| Hata::Db(format!("müşteri yok: {n}")))?;
        let c = v.bagli_cihaz(n)?;
        let c = c.as_ref();
        let d = self.durum()?.get(&n.to_string()).cloned().unwrap_or(Value::Null);
        let cs = e(&ot.csrf);
        let csrf = format!("<input type=\"hidden\" name=\"csrf\" value=\"{cs}\">");
        let acik = sorgu_ilk(sorgu, "acik").unwrap_or_default();
        let (ldurum, lbitis) = v.lisans_durumu(&m);
        let bitis = e(veya_tire(tarih_goster(&lbitis, false)));
        let lisans_metni = match ldurum {
            "aktif" => "<b>Aktif</b>",
            "bitti" => "<b class=\"kotu\">Süresi doldu</b>",
            _ => "<b class=\"kotu\">Askıda</b>",
        };
        let uyelik_aktif = m.uyelik == "aktif";
        let (durum, son, sorunlu) = self.ozet(c, &d);
        let durum = if sorunlu { format!("<b class=\"kotu\">{}</b>", e(&durum)) } else { format!("<b>{}</b>", e(&durum)) };
        let surum = match c {
            Some(c) => e_veya(&c.surum),
            None => "—".into(),
        };

        // üst kısım: her zaman görünür; ortak menü
        let panel = if self.cihaz_adresi(n)?.is_some() {
            format!("<form method=\"post\" action=\"/m/{n}/cihaz-paneli\">{csrf}<button>Cihaz paneline git</button></form>")
        } else {
            "<span class=\"bos\" title=\"Cihaz kapalı, tünel yok ya da lisans/üyelik kapalı\">Cihaz paneline şu an ulaşılamıyor</span>".to_string()
        };
        let ad = c.map(|c| c.isletme_adi.as_str()).unwrap_or("");
        let ust = kart(
            &(format!("<h1>Müşteri {n}")
                + &(if ad.is_empty() { String::new() } else { format!(" <span class=\"alt-baslik\">· {}</span>", e(ad)) })
                + "</h1>"
                + &(if m.not_.is_empty() { String::new() } else { format!("<p class=\"alt\">Not: {}</p>", e(&m.not_)) })
                + &format!(
                    "<p class=\"durumlar\"><span>Cihaz: {durum}</span><span>Lisans: {lisans_metni} · bitiş {bitis}</span><span>Sürüm: <b>{surum}</b></span>"
                )
                + if uyelik_aktif { "" } else { "<span><b class=\"kotu\">Üyelik bitti</b></span>" }
                + "</p>"
                + &format!(
                    "<div class=\"ortak\">{panel}<a class=\"dugme\" href=\"/m/{n}/kayitlar/\">Kayıtlar</a>\
                     <a class=\"dugme\" href=\"/m/{n}/raporlar\">Raporlar</a><a class=\"dugme\" href=\"/m/{n}/hareketler\">Hareketler</a></div>"
                )),
        );

        let mut parola = if m.parola_acik.is_empty() {
            "<span class=\"bos\">bilinmiyor (bu özellikten önce belirlendi) — parolayı değiştirin</span>".to_string()
        } else {
            format!("<span class=\"sir\">{}</span>", e(&m.parola_acik))
        };
        parola += &acilir(
            "Parolayı değiştir",
            &format!(
                "<form method=\"post\" action=\"/m/{n}/parola-sifirla\">{csrf}{}<button>Parolayı değiştir</button><p class=\"alt\">Merkezi \
                 panelde hemen, cihazda bir sonraki eşitlemeden (her gün 06:00) sonra geçerlidir.</p></form>",
                parola_alanlari("Yeni müşteri parolası")
            ),
            "",
        );
        // AP'ye (müşterinin erişim noktası/modemi) yönetim parolası olarak bu yazılır, panel parolası değil (final review I1)
        let ap_parola = format!(
            "<span class=\"sir\">{}</span><form method=\"post\" action=\"/m/{n}/ap-parola\" class=\"satir-ici\">{csrf}\
             <button class=\"ikincil\">Yenile</button></form>",
            e(&m.ap_parola)
        ) + &acilir(
            "Açıklama",
            "<p class=\"alt\">AP'nin (modemin) yönetim parolası; panel parolasını AP'ye yazmayın. \
             Yenilenirse AP'de elle değiştirilir; cihaza 5 dk içinde ulaşır (0.4.17'den eski cihazlarda her gün \
             06:00'da) ve cihaz panelinde görünür. Cihaz çevrimdışıyken yenilerseniz cihaz yeniden bağlanana kadar \
             eski parolayı kullanır.</p>",
            "",
        );

        let lisans_formu = |islem: &str, etiket: &str, sinif: &str| -> String {
            let sinif = if sinif.is_empty() { String::new() } else { format!(" class=\"{sinif}\"") };
            format!(
                "<form method=\"post\" action=\"/m/{n}/lisans\">{csrf}<input type=\"hidden\" name=\"islem\" value=\"{islem}\">\
                 <button{sinif}>{etiket}</button></form>"
            )
        };
        let askida = ldurum == "askida";
        let uyelik_hucre = if uyelik_aktif {
            "Aktif".to_string()
        } else {
            format!("<b class=\"kotu\">Bitti</b> {}", e(tarih_goster(&m.bitis.chars().take(10).collect::<String>(), false)))
        };
        let mut lisans = bilgi_tablosu(&[
            ("Lisans", format!("{lisans_metni} · bitiş {bitis}")),
            ("Üyelik", uyelik_hucre),
            ("Oluşturma", e(tarih_goster(&m.olusturma, false))),
            ("Son giriş (panel)", e(veya_tire(tarih_goster(&m.son_giris, false)))),
        ]) + "<div class=\"dugmeler\">"
            + &lisans_formu("uzat", "Lisansı 1 yıl uzat", "")
            + &(if askida { lisans_formu("ac", "Askıdan çıkar (cihazı aç)", "") } else { String::new() })
            + &(if uyelik_aktif {
                String::new()
            } else {
                format!(
                    "<form method=\"post\" action=\"/m/{n}/uyelik\">{csrf}<input type=\"hidden\" name=\"islem\" \
                     value=\"ac\"><button>Üyeliği yeniden aç</button></form>"
                )
            })
            + &format!(
                "</div><form method=\"post\" action=\"/m/{n}/lisans\" class=\"ara\">{csrf}<input type=\"hidden\" name=\"islem\" \
                 value=\"tarih\"><input name=\"tarih\" type=\"date\" required aria-label=\"Lisans bitiş tarihi\"><button>Bitişi ayarla</button></form>"
            )
            + &acilir(
                "Açıklama",
                "<p class=\"alt\">Lisansı biten ya da askıya alınan cihaz misafirlere internet vermez; cihaz bunu \
                 bitiş gününden sonra kendisi uygular, askıya alma/uzatma en geç 30 dakikada (askıdayken) ya da ertesi \
                 06:00'da cihaza iner.</p>",
                "",
            );
        let mut tehlikeli: Vec<(&str, String)> = Vec::new();
        if !askida {
            tehlikeli.push(("Askıya al", lisans_formu("askiya", "Askıya al (cihazı kapat)", "tehlike")));
        }
        if uyelik_aktif {
            tehlikeli.push((
                "Üyeliği bitir",
                format!(
                    "<form method=\"post\" action=\"/m/{n}/uyelik\">{csrf}<input type=\"hidden\" name=\"islem\" value=\"bitir\">\
                     <p class=\"alt\">Cihaz serbest bırakılır, müşteri listede gizlenir; arşivi ve geçmişi silinmez. \
                     \"Üyeliği yeniden aç\" ile geri gelir.</p>\
                     <label for=\"u\">Üyeliği bitirmek için müşteri numarasını yazın ({n})</label>\
                     <input id=\"u\" name=\"onay\" inputmode=\"numeric\" autocomplete=\"off\" required>\
                     <button class=\"tehlike\">Üyeliği bitir</button></form>"
                ),
            ));
        }
        if !tehlikeli.is_empty() {
            lisans += &acilir(&tehlikeli.iter().map(|(a, _)| *a).collect::<Vec<_>>().join(" / "), &tehlikeli.iter().map(|(_, f)| f.as_str()).collect::<String>(), "tehlikeli");
        }

        let (cihaz_ozet, mut cihaz) = match c {
            None => (
                "bağlı cihaz yok".to_string(),
                "<p class=\"alt\">Bağlı cihaz yok. Müşteri bir cihazın giriş ekranına numarasıyla girince bağlanır.</p>".to_string(),
            ),
            Some(c) => (
                format!("{durum} · {surum}"),
                bilgi_tablosu(&[
                    ("Durum", durum.clone()),
                    ("İşletme adı", e_veya(&c.isletme_adi)),
                    ("İşletme türü", e(isletme_turu(Some(c)))),
                    ("Unvan", e_veya(&c.unvan)),
                    ("Adres", e_veya(&c.adres)),
                    ("Tünel adresi", e_veya(&c.tunel_ip)),
                    ("Sürüm", surum.clone()),
                    ("Bağlanma", e(tarih_goster(&c.baglanma, false))),
                    ("Son eşitleme", e(veya_tire(tarih_goster(&c.son_eslesme, false)))),
                    ("Son gelen gün", son.clone()),
                    ("Arşiv", e(detay::boyut(d.get("boyut").and_then(Value::as_f64).unwrap_or(0.0) as i64))),
                ]) + &acilir(
                    "Cihazı serbest bırak",
                    &format!(
                        "<form method=\"post\" action=\"/m/{n}/serbest\">{csrf}\
                         <label for=\"o\">Cihazı serbest bırakmak için müşteri numarasını yazın ({n})</label>\
                         <input id=\"o\" name=\"onay\" inputmode=\"numeric\" autocomplete=\"off\" required>\
                         <label><input type=\"checkbox\" name=\"zorla\" value=\"1\"> Cihaz ulaşılamaz (arızalı/kayıp): beklemeden ayır. \
                         Cihazda kalmış gönderilmemiş günler kaybolabilir.</label>\
                         <button class=\"tehlike\">Cihazı serbest bırak</button></form>"
                    ),
                    "tehlikeli",
                ),
            ),
        };
        let gecmis: Vec<Vec<String>> = v
            .cihaz_gecmisi(n)?
            .iter()
            .map(|x| vec![x.id.to_string(), x.durum.clone(), tarih_goster(&x.baglanma, false), tarih_goster(&x.ayrilma, false), x.tunel_ip.clone()])
            .collect();
        cihaz += &acilir("Cihaz geçmişi", &detay::tablo(&["No", "Durum", "Bağlanma", "Ayrılma", "Tünel"], &gecmis, "Henüz cihaz yok."), "");

        let (guncelleme_ozet, guncelleme) = self.guncelleme_bolumu(&m, c, &cs)?;
        let sorunlar = v.sorun_gecmisi(n, 50)?;
        let acik_sorun = sorunlar.iter().filter(|x| x.kapanis.is_empty()).count();
        let sorun_ozet = if sorunlar.is_empty() {
            "kayıt yok".to_string()
        } else {
            format!("{}{} kayıt", if sorunlar.len() == 50 { "son " } else { "" }, sorunlar.len())
                + &(if acik_sorun > 0 { format!(" · <b class=\"kotu\">{acik_sorun} açık</b>") } else { String::new() })
        };
        let saglik_ek = vec![
            ("Bugün doğrulama SMS'i", v.sms_sayac(n, &v.bugun().to_string())?.to_string()),
            ("Son doğrulama SMS'i", son_sms_metni(v.sms_son(n)?.as_ref())),
        ];
        // MUSTERI_BOLUMLERI sırasıyla (kimlik, başlık) ile eşleşir: (başlık yanındaki durum, içerik)
        let bolumler: [(String, String); 6] = [
            ("müşteri ve AP yönetim parolası".to_string(), bilgi_tablosu(&[("Müşteri parolası", parola), ("AP yönetim parolası", ap_parola)])),
            (format!("{lisans_metni} · bitiş {bitis}") + if uyelik_aktif { "" } else { " · <b class=\"kotu\">üyelik bitti</b>" }, lisans),
            (cihaz_ozet, cihaz),
            (self.saglik_ozeti(c)?, self.saglik_tablosu(c, &saglik_ek)?),
            (guncelleme_ozet, guncelleme),
            (sorun_ozet, self.sorun_gecmisi(&sorunlar)),
        ];
        let mut sayfa = ust;
        for ((k, b), (ozet, icerik)) in MUSTERI_BOLUMLERI.iter().zip(bolumler.iter()) {
            sayfa += &bolum(k, b, ozet, icerik, acik == *k);
        }
        Ok(yanit_html(&self.sayfa(&format!("Müşteri {n}"), &sayfa, Some(ot), true, None), 200, None))
    }

    /// Sağlık bölümünün başlık satırı: son raporun yaşı, disk ve duran servis.
    fn saglik_ozeti(&self, c: Option<&Cihaz>) -> Sonuc<String> {
        let Some(s) = self.saglik_kaydi(c)? else { return Ok("henüz rapor yok".into()) };
        let r = s.rapor.as_object().cloned().unwrap_or_default(); // cihazdan gelen veri: kaçırılır
        let ep = raporlar::epoch(&s.zaman).filter(|v| *v != 0.0);
        let mut parca = vec![match ep {
            Some(ep) => format!("son rapor {}", e(once(self.taban.simdi() - ep))),
            None => "son rapor —".to_string(),
        }];
        if let Some(d) = r.get("disk_yuzde") {
            parca.push(format!("disk %{}", e(py_str(d))));
        }
        let tum = r.get("servisler").and_then(Value::as_object).map(|sv| sv.values().all(py_dogru)).unwrap_or(true);
        if !tum {
            parca.push("<b class=\"kotu\">servis durdu</b>".into());
        }
        Ok(parca.join(" · "))
    }

    /// `ek`: tablonun sonuna eklenen satırlar (merkezin doğrulama SMS'i sayaçları; rapor yokken de gösterilir).
    fn saglik_tablosu(&self, c: Option<&Cihaz>, ek: &[(&str, String)]) -> Sonuc<String> {
        let Some(s) = self.saglik_kaydi(c)? else {
            return Ok("<p class=\"alt\">Henüz rapor yok.</p>".to_string() + &if ek.is_empty() { String::new() } else { bilgi_tablosu(ek) });
        };
        let r = s.rapor.as_object().cloned().unwrap_or_default(); // cihazdan gelen veri: her değer kaçırılır
        let servis = {
            let l: Vec<String> = r
                .get("servisler")
                .and_then(Value::as_object)
                .map(|sv| sv.iter().map(|(ad, ok)| e(ad) + if py_dogru(ok) { " çalışıyor" } else { " <b class=\"kotu\">DURDU</b>" }).collect())
                .unwrap_or_default();
            if l.is_empty() { "—".to_string() } else { l.join(" · ") }
        };
        let acik = match r.get("acik_sn") {
            Some(Value::Number(ac)) => e(once(ac.as_f64().unwrap_or(0.0)).replace(" önce", "")),
            _ => "—".into(),
        };
        let ep = raporlar::epoch(&s.zaman).filter(|v| *v != 0.0);
        let zaman = match ep {
            Some(ep) => format!("{} ({})", e(tarih_goster(&s.zaman, false)), e(once(self.taban.simdi() - ep))),
            None => e(&s.zaman),
        };
        let durum = |d: Option<&Value>, iyi: bool| -> String {
            match d {
                None | Some(Value::Null) => "—".into(),
                Some(d) if py_dogru(d) == iyi => "tamam".into(),
                _ => "<b class=\"kotu\">sorun</b>".into(),
            }
        };
        let veya = |k: &str| e(r.get(k).map(py_str).unwrap_or_else(|| "—".into()));
        let mut satirlar: Vec<(&str, String)> = vec![
            ("Son rapor", zaman),
            ("Sürüm", veya("surum")),
            ("Açık kalma", acik),
            ("Servisler", servis),
            ("Disk", if let Some(d) = r.get("disk_yuzde") { format!("%{}", e(py_str(d))) } else { "—".into() }),
            ("Son mühür", e(veya_tire(tg_deger(r.get("son_muhur"))))),
            ("WAN", durum(r.get("wan"), true)),
            ("İnternet", internet_metni(&r)),
            ("SMS", durum(r.get("sms_sorunu"), false)),
            ("Bağlı cihaz", veya("bagli")),
        ];
        satirlar.extend(ek.iter().cloned());
        Ok(bilgi_tablosu(&satirlar))
    }

    fn sorun_gecmisi(&self, sorunlar: &[Sorun]) -> String {
        let simdi = self.taban.simdi();
        let satirlar: Vec<Vec<String>> = sorunlar
            .iter()
            .map(|x| {
                let (a, k) = gecmis_sms_etiketleri(x, simdi);
                let kapanis = tarih_goster(&x.kapanis, false);
                vec![tarih_goster(&x.acilis, false), if kapanis.is_empty() { "açık".into() } else { kapanis }, x.metin.clone(), onem_etiketi(&x.onem), a, k]
            })
            .collect();
        detay::tablo(&["Açılış", "Kapanış", "Sorun", "Önem", "Açılış SMS'i", "Kapanış SMS'i"], &satirlar, "Sorun kaydı yok.")
    }

    fn admin_parolasi(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        // Bütün cihazlarda tek admin parolası (2026-10-08): yönetici belirler, kendiliğinden değişmez; yalnızca özet saklanır.
        // Formu Hesabım'dadır (2026-10-24); bu adres yalnızca kaydeder, eski bağlantılar Hesabım'a gider.
        if yontem == "POST" {
            let pw = match parola_formdan(form) {
                Ok(p) => p,
                Err(hata) => return Ok(yanit_html(&self.mesaj(Some(ot), "Admin parolası kaydedilmedi", &hata), 400, None)),
            };
            self.veri().admin_parolasi_koy(&pw)?;
            self.veri().hareket("admin", ip, "ADMIN_PAROLASI", None, "")?; // parola yazılmaz
        }
        Ok(yonlendir("/hesabim", None))
    }

    fn admin_parolasi_karti(&self, ot: &Ot) -> Sonuc<String> {
        let durum = if self.veri().admin_parolasi()?.is_some() { "tanımlı" } else { "tanımsız (cihazlar kendi admin parolasını korur)" };
        Ok(kart(&format!(
            "<h2>Admin parolası (bütün cihazlar)</h2><p>Durum: <b>{durum}</b></p>\
             <form method=\"post\" action=\"/admin-parolasi\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">{}<button>Kaydet</button></form>\
             <p class=\"alt\">Bütün cihazlarda <b>admin</b> kullanıcısı bu parolayla girer. Cihazlar bir sonraki eşitlemede \
             (her gün 06:00) alır; siz değiştirmedikçe değişmez. Parola burada saklanmaz, yalnızca özeti tutulur — \
             unutursanız yenisini belirleyin.</p>",
            e(&ot.csrf),
            parola_alanlari("Yeni admin parolası")
        )))
    }

    fn lisans(&self, ot: &Ot, n: i64, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let v = self.veri();
        match form.get("islem").map(String::as_str) {
            Some("uzat") => {
                let bitis = v.lisans_uzat(n, veri::LISANS_GUN)?;
                v.hareket("admin", ip, "LISANS_UZATILDI", Some(n), &format!("bitis={bitis}"))?;
            }
            Some("askiya") => {
                v.askiya_al(n, true)?;
                v.hareket("admin", ip, "LISANS_ASKIYA_ALINDI", Some(n), "")?;
            }
            Some("ac") => {
                v.askiya_al(n, false)?;
                v.hareket("admin", ip, "LISANS_ACILDI", Some(n), "")?;
            }
            Some("tarih") => {
                // 12.10.2026 ya da ISO
                match v.lisans_ayarla(n, &tarih_girdi(form.get("tarih").map(String::as_str).unwrap_or(""))) {
                    Ok(tarih) => v.hareket("admin", ip, "LISANS_TARIHI", Some(n), &format!("bitis={tarih}"))?,
                    Err(Hata::Deger(_)) => {
                        return Ok(yanit_html(&self.mesaj(Some(ot), "Geçersiz tarih", "Tarih GG.AA.YYYY biçiminde olmalı."), 400, None));
                    }
                    Err(h) => return Err(h),
                }
            }
            _ => {}
        }
        Ok(musteri_donus(n, "lisans"))
    }

    fn serbest(&self, ot: &Ot, n: i64, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        if py_strip(form.get("onay").map(String::as_str).unwrap_or("")) != n.to_string() {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Onaylanmadı", "Serbest bırakmak için müşteri numarasını doğru yazın."), 400, None));
        }
        let zorla = form.get("zorla").map(String::as_str) == Some("1");
        let c = self.veri().serbest_birak(n, zorla)?;
        if c.is_none() && !zorla {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Cihaz yok", "Bu müşterinin bağlı cihazı yok."), 400, None));
        }
        if zorla {
            let mut is = json!({"numara": n.to_string()});
            if let Some(c) = &c {
                is["wg_pub"] = json!(c.wg_pub);
            }
            (self.kuyruk_ekle)("cihaz-kapat", &is).map_err(|h| veri_hatasi("kuyruk", h))?;
            let cid = c.as_ref().map(|c| c.id.to_string()).unwrap_or_else(|| "-".into());
            self.veri().hareket("admin", ip, "CIHAZ_ZORLA_AYRILDI", Some(n), &format!("cihaz={cid}"))?;
        } else if let Some(c) = &c {
            self.veri().hareket("admin", ip, "SERBEST_BIRAKMA_ISTENDI", Some(n), &format!("cihaz={}", c.id))?;
        }
        Ok(musteri_donus(n, "cihaz"))
    }

    fn hareket_tablosu(&self, hareketler: &[Hareket]) -> String {
        let satirlar: Vec<Vec<String>> = hareketler
            .iter()
            .map(|h| {
                vec![
                    tarih_saat_goster(&h.zaman),
                    h.kim.clone(),
                    h.ip.clone(),
                    h.olay.clone(),
                    h.musteri.filter(|m| *m != 0).map(|m| m.to_string()).unwrap_or_default(),
                    h.ayrinti.clone(),
                ]
            })
            .collect();
        detay::tablo(&["Zaman", "Kim", "IP", "Olay", "Müşteri", "Ayrıntı"], &satirlar, "Kayıt yok.")
    }

    fn eski_arsivler(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let durum = self.durum()?;
        let bagli: HashMap<String, Option<i64>> = self.veri().eski_arsivler()?.into_iter().collect();
        // sorted(set(durum.keys() - geçerli numaralar) | set(bağlı)): sıralı birleşim
        let adaylar: BTreeSet<String> = durum.keys().filter(|a| !guvenlik::numara_gecerli(a)).cloned().chain(bagli.keys().cloned()).collect();
        if yontem == "POST" {
            let ad = form.get("ad").map(String::as_str).unwrap_or("");
            let mn = py_strip(form.get("musteri").map(String::as_str).unwrap_or("")).to_string();
            let gecersiz = !adaylar.iter().any(|a| a == ad)
                || (!mn.is_empty() && (!guvenlik::numara_gecerli(&mn) || self.veri().musteri(mn.parse().unwrap_or(0))?.is_none()));
            if gecersiz {
                return Ok(yanit_html(&self.mesaj(Some(ot), "Geçersiz", "Arşiv ya da müşteri numarası bulunamadı."), 400, None));
            }
            let numara: Option<i64> = if mn.is_empty() { None } else { mn.parse().ok() };
            self.veri().eski_arsiv_bagla(ad, numara)?;
            self.veri().hareket("admin", ip, "ESKI_ARSIV_BAGLANDI", numara, &format!("arsiv={ad}"))?;
            if let Some(yol) = &self.rapor_gruplari {
                if let Err(h) = self.veri().rapor_gruplari_yaz(yol) {
                    // bağ kaydedildi; dosya merkez açılışında yeniden yazılır
                    web::log(&format!("rapor-gruplari yazilamadi: {h:?}"));
                }
            }
            return Ok(yonlendir("/eski-arsivler", None));
        }
        let cs = e(&ot.csrf);
        let satirlar: String = adaylar
            .iter()
            .map(|a| {
                let d = durum.get(a);
                let tur = d.and_then(|d| d.get("tur")).map(py_str).unwrap_or_else(|| "—".into());
                let son = veya_tire(tg_deger(d.and_then(|d| d.get("son_gun"))));
                let bag = bagli.get(a).copied().flatten().map(|n| n.to_string()).unwrap_or_default();
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td>\
                     <td><form method=\"post\" action=\"/eski-arsivler\" class=\"ara\"><input type=\"hidden\" name=\"csrf\" value=\"{cs}\">\
                     <input type=\"hidden\" name=\"ad\" value=\"{}\"><input name=\"musteri\" value=\"{}\" \
                     placeholder=\"müşteri no (boş = bağsız)\" inputmode=\"numeric\"><button>Kaydet</button></form></td></tr>",
                    e(a),
                    e(&tur),
                    e(&son),
                    e(a),
                    e(&bag)
                )
            })
            .collect();
        let tablo = if adaylar.is_empty() {
            "<p class=\"alt\">Eski arşiv yok.</p>".to_string()
        } else {
            format!(
                "<div class=\"kaydir\"><table class=\"detay\"><tr><th>Arşiv</th><th>Tür</th><th>Son gün</th><th>Bağlı müşteri</th></tr>{satirlar}</table></div>"
            )
        };
        let h = kart(&format!(
            "<h1>Eski arşivler</h1><p class=\"alt\">OpenWrt dönemi ve elle eklenmiş arşivler. Bir müşteriye bağlanırsa müşteri \
             kayıtlarında görünür.</p>{tablo}"
        ));
        Ok(yanit_html(&self.sayfa("Eski arşivler", &h, Some(ot), true, None), 200, None))
    }

    fn hesabim_html(&self, ot: &Ot, hata: &str, tamam: &str) -> Sonuc<String> {
        let m = if !hata.is_empty() {
            format!("<p class=\"hata\">{}</p>", e(hata))
        } else if !tamam.is_empty() {
            format!("<p class=\"tamam\">{}</p>", e(tamam))
        } else {
            String::new()
        };
        let govde = kart(&format!(
            "<h1>Hesabım</h1><h2>Yönetici parolası</h2><p class=\"alt\">Bu yönetim sayfasına girişin parolası.</p>\
             <form method=\"post\" action=\"/hesabim\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
             <label for=\"m\">Mevcut parola</label><input id=\"m\" name=\"mevcut\" type=\"password\" required>\
             <label for=\"y\">Yeni parola (en az 12)</label><input id=\"y\" name=\"yeni\" type=\"password\" minlength=\"12\" required>\
             <label for=\"t\">Yeni parola (tekrar)</label><input id=\"t\" name=\"tekrar\" type=\"password\" minlength=\"12\" required>\
             {m}<button>Değiştir</button></form>",
            e(&ot.csrf)
        )) + &self.admin_parolasi_karti(ot)?;
        Ok(self.sayfa("Hesabım", &govde, Some(ot), false, None))
    }

    fn hesabim(&self, ot: &Ot, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        if yontem == "GET" {
            return Ok(yanit_html(&self.hesabim_html(ot, "", "")?, 200, None));
        }
        let al = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        if self.dogrula(&ot.user, al("mevcut"))?.is_none() {
            return Ok(yanit_html(&self.hesabim_html(ot, "Mevcut parola yanlış.", "")?, 400, None));
        }
        if al("yeni") != al("tekrar") {
            return Ok(yanit_html(&self.hesabim_html(ot, "Yeni parolalar aynı değil.", "")?, 400, None));
        }
        if let Err(h) = yonetici::yaz(&self.yonetici_yolu, &ot.user, al("yeni")) {
            return Ok(yanit_html(&self.hesabim_html(ot, &h, "")?, 400, None));
        }
        self.veri().hareket("admin", ip, "YONETICI_PAROLA_DEGISTI", None, "")?;
        let t = &self.taban;
        t.oturumlar.drop_user(&ot.user); // diğer oturumlar da düşer; değiştiren yeni oturumla devam eder
        let yeni = self.oturum_ac(&ot.user, t.simdi())?;
        let oturum = t.oturumlar.get(&yeni, t.simdi()).ok_or_else(|| Hata::Db("yeni oturum açılamadı".into()))?;
        let ot2 = Ot { oturum, yol: String::new() }; // Python'daki `ot`: oturum kaydı, açık sayfa yolu olmadan
        Ok(yanit_html(&self.hesabim_html(&ot2, "", "Parola değiştirildi.")?, 200, Some((self.cerez(), Some(&yeni)))))
    }
}

impl Panel for Yonetim {
    fn taban(&self) -> &Taban {
        &self.taban
    }

    fn rol(&self) -> &'static str {
        "yonetici"
    }

    fn cerez(&self) -> &'static str {
        "wc_yonetim"
    }

    fn yan_menu(&self) -> bool {
        true
    }

    fn giris_basligi(&self) -> &'static str {
        "Yönetim merkezi"
    }

    fn giris_aciklama(&self) -> &'static str {
        "Hizmet sağlayıcı girişi."
    }

    fn dogrula(&self, kul: &str, pw: &str) -> Sonuc<Option<String>> {
        let y = yonetici::oku(&self.yonetici_yolu);
        let Some(y) = y.filter(|y| y.kullanici == kul) else {
            guvenlik::bos_dogrulama(pw);
            return Ok(None);
        };
        Ok(guvenlik::dogru(pw, &y.tuz, &y.ozet, y.yineleme).then(|| kul.to_string()))
    }

    fn hesap_surumu(&self, kimlik: &str) -> Sonuc<Option<String>> {
        Ok(yonetici::oku(&self.yonetici_yolu).filter(|y| y.kullanici == kimlik).map(|y| y.ozet))
    }

    fn girdi(&self, _kimlik: &str, ip: &str) -> Sonuc<()> {
        self.veri().hareket("admin", ip, "GIRIS", None, "yönetim")
    }

    fn kim(&self, _ot: &Ot) -> String {
        "admin".into()
    }

    /// `re.match(r"/m/([0-9]{7})/", yol)`.
    fn hareket_musterisi(&self, _ot: &Ot, yol: &str) -> Option<i64> {
        let r = yol.strip_prefix("/m/")?;
        let b = r.as_bytes();
        (b.len() > 7 && b[..7].iter().all(u8::is_ascii_digit) && b[7] == b'/').then(|| r[..7].parse().ok()).flatten()
    }

    /// Gruplar ok işaretiyle aşağı açılır (<details>, JS yok): açık sayfanın grubu açık, diğerleri kapalı gelir.
    fn menu(&self, ot: &Ot) -> String {
        let secili = secili_menu(&ot.yol);
        let mut gruplar = String::new();
        for (baslik, baglantilar) in MENU {
            let ic: String = baglantilar
                .iter()
                .map(|(h, ad)| format!("<a href=\"{h}\"{}>{}</a>", if Some(*h) == secili { " class=\"secili\" aria-current=\"page\"" } else { "" }, e(ad)))
                .collect();
            let acik = if baglantilar.iter().any(|(h, _)| Some(*h) == secili) { " open" } else { "" };
            gruplar += &format!("<details class=\"grup\"{acik}><summary>{}</summary>{ic}</details>", e(baslik));
        }
        gruplar
    }

    fn sayfalar(&self, ot: &Ot, _token: &str, yontem: &str, yol: &str, sorgu: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Option<Yanit>> {
        if let Some(r) = self.ayar_sayfalari(ot, yontem, yol, form, ip)? {
            return Ok(Some(r));
        }
        let v = self.veri();
        match (yontem, yol) {
            ("GET", "/") => return self.liste(ot, sorgu).map(Some),
            ("GET", "/bekleyen") => return self.bekleyen_sayfasi(ot).map(Some),
            ("GET", "/hareketler") => {
                let h = kart(&format!("<h1>Hareketler</h1>{}", self.hareket_tablosu(&v.hareketler(None, 300, None)?)));
                return Ok(Some(yanit_html(&self.sayfa("Hareketler", &h, Some(ot), true, None), 200, None)));
            }
            _ => {}
        }
        match yol {
            "/yeni" => return self.yeni(ot, yontem, form, ip).map(Some),
            "/eski-arsivler" => return self.eski_arsivler(ot, yontem, form, ip).map(Some),
            "/admin-parolasi" => return self.admin_parolasi(ot, yontem, form, ip).map(Some),
            "/hesabim" => return self.hesabim(ot, yontem, form, ip).map(Some),
            _ => {}
        }
        // re.fullmatch(r"/m/([0-9]{7})(/.*)?", yol)
        let Some(r) = yol.strip_prefix("/m/") else { return Ok(None) };
        let b = r.as_bytes();
        if b.len() < 7 || !b[..7].iter().all(u8::is_ascii_digit) || (b.len() > 7 && b[7] != b'/') || r.contains('\n') {
            return Ok(None);
        }
        let n: i64 = r[..7].parse().map_err(|_| Hata::Deger(r.to_string()))?;
        if v.musteri(n)?.is_none() {
            return Ok(None);
        }
        let alt = if r.len() == 7 { "/" } else { &r[7..] };
        match (yontem, alt) {
            ("GET", "/") => return self.musteri_sayfasi(ot, n, sorgu).map(Some),
            ("POST", "/parola-sifirla") => {
                let pw = match parola_formdan(form) {
                    Ok(p) => p,
                    Err(hata) => return Ok(Some(yanit_html(&self.mesaj(Some(ot), "Parola kaydedilmedi", &hata), 400, None))),
                };
                v.parola_koy(n, &pw)?;
                v.hareket("admin", ip, "PAROLA_SIFIRLANDI", Some(n), "")?;
                return Ok(Some(musteri_donus(n, "parolalar")));
            }
            ("POST", "/ap-parola") => {
                v.ap_parolasi_yenile(n)?;
                v.hareket("admin", ip, "AP_PAROLASI_YENILENDI", Some(n), "")?; // parola hareket satırına yazılmaz
                return Ok(Some(musteri_donus(n, "parolalar")));
            }
            ("POST", "/serbest") => return self.serbest(ot, n, form, ip).map(Some),
            ("POST", "/guncelleme") => return self.guncelleme_kanali(ot, n, form, ip).map(Some),
            ("POST", "/guncelleme-hemen") => return self.guncelleme_hemen(ot, n, ip).map(Some),
            ("POST", "/cihaz-paneli") => return self.cihaz_paneli(ot, n, ip).map(Some),
            ("POST", "/lisans") => return self.lisans(ot, n, form, ip).map(Some),
            ("POST", "/uyelik") => {
                match form.get("islem").map(String::as_str) {
                    Some("bitir") => {
                        if py_strip(form.get("onay").map(String::as_str).unwrap_or("")) != n.to_string() {
                            return Ok(Some(yanit_html(&self.mesaj(Some(ot), "Onaylanmadı", "Üyeliği bitirmek için müşteri numarasını doğru yazın."), 400, None)));
                        }
                        v.uyelik(n, "bitti")?;
                        v.serbest_birak(n, false)?;
                        v.hareket("admin", ip, "UYELIK_BITTI", Some(n), "")?;
                    }
                    Some("ac") => {
                        v.uyelik(n, "aktif")?;
                        v.serbest_iptal(n)?; // bitirirken başlayan serbest bırakma, cihaz temizlenmediyse geri alınır
                        v.hareket("admin", ip, "UYELIK_ACILDI", Some(n), "")?;
                    }
                    _ => {}
                }
                return Ok(Some(musteri_donus(n, "lisans")));
            }
            ("GET", "/hareketler") => {
                let h = kart(&format!("<h1>Müşteri {n} · hareketler</h1>{}", self.hareket_tablosu(&v.hareketler(Some(n), 300, None)?)));
                return Ok(Some(yanit_html(&self.sayfa("Hareketler", &h, Some(ot), true, None), 200, None)));
            }
            _ => {}
        }
        if yontem == "GET" && (alt == "/raporlar" || alt.starts_with("/rapor/")) {
            // kişisel veri yok: gerekçesiz
            return raporlar::istek(self, ot, n, alt, &format!("/m/{n}"));
        }
        if (yontem == "GET" && (alt == "/kayitlar" || alt.starts_with("/kayitlar/"))) || (yontem, alt) == ("POST", "/kayitlar/ara") {
            let c = v.bagli_cihaz(n)?;
            let baslik = match &c {
                Some(c) if !c.isletme_adi.is_empty() => c.isletme_adi.clone(),
                _ => format!("Müşteri {n}"),
            };
            let kalan = &alt["/kayitlar".len()..];
            return self.kayit_sayfasi(ot, n, if kalan.is_empty() { "/" } else { kalan }, sorgu, &format!("/m/{n}/kayitlar"), ip, &baslik, yontem, Some(form));
        }
        Ok(None)
    }
}

#[cfg(test)]
#[path = "yonetim_testler.rs"]
mod testler;
