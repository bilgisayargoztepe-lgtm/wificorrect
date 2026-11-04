//! Cihaz API'si (Python api.py, spec §4): api.wificorrect.com/api/{giris,bekliyor,bagla,eslesme,parola,saglik,paket,liste}.
//! JSON gövde ≤ 4 KB, IP başına dakikada 20 istek. Cihazın kimliği ilk girişte verilen cihaz anahtarıdır (merkez yalnızca
//! SHA-256'sını tutar). `/api/sms` ve `/api/sms-dogrula`: misafir doğrulama SMS'i (NetGSM OTP, Twilio Verify). `/api/raporlar` ve `/api/rapor` yoktur (spec
//! bilinçli fark 1): 404.
use std::path::PathBuf;
use std::sync::Arc;

use chrono::NaiveDate;
use serde_json::{json, Map, Value};

use crate::guvenlik::{self, GirisKilidi, HizSiniri};
use crate::kuyruk;
use crate::listeler;
use crate::merkez::Uygulama;
use crate::netgsm;
use crate::ortak::{py_dogru, py_str, py_yazdirilabilir, Tasiyici};
use crate::surumler;
use crate::twilio;
use crate::veri::{py_json, Hata, Musteri, Saat, Sonuc, Veri};
use crate::web::{hata_yaniti, Istek, Yanit};

pub const GOVDE_SINIRI: usize = 4096;
pub const YEDEK_SUNUCU: &str = "10.99.0.1";
/// Doğrulama SMS'i: sabit şablon, yalnızca 6 haneli kod; cihaz serbest metin gönderemez (açık röle olmasın).
pub const OTP_SABLON: &str = "WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir.";
/// Müşteri (cihaz) başına günlük (TR günü) doğrulama SMS'i sınırı.
pub const SMS_GUNLUK_SINIR: i64 = 300;
/// Yabancı numaralara (Twilio, pahalı) ayrıca; bunlar 300'e de sayılır.
pub const SMS_YABANCI_SINIR: i64 = 20;
/// `sms_sayac`'ta yabancı numara sayacının gün anahtarı eki.
pub const YABANCI_SAYAC: &str = "/yabanci";

pub const SAGLIK_SERVISLER: [&str; 5] = ["portal", "panel", "kaydedici", "dnsmasq", "guvenlik"];
pub const SAGLIK_BOOL: [&str; 4] = ["kayit_boslugu_1s", "wan", "sms_sorunu", "hiz_hata"];
/// PPPoE (spec 2026-10-23 "Durum ve teşhis"): internet portunun türü ve durumu. Bilinmeyen değer raporu bozmaz,
/// `saglik_temizle` yalnızca o alanı atar.
pub const SAGLIK_WAN_TURLERI: [&str; 2] = ["dhcp", "pppoe"];
pub const SAGLIK_PPPOE_DURUMLARI: [&str; 9] =
    ["kapali", "kablo_yok", "dhcp_yok", "internet_yok", "sunucu_yok", "modemden", "kimlik_hatasi", "baglaniyor", "bagli"];

/// Kuyruğa iş koyar (`islem`, alanlar); dönen iş kimliği. Testte sahte, üretimde `kuyruk::ekle`.
pub type KuyrukEkle = Box<dyn Fn(&str, &Value) -> std::io::Result<String> + Send + Sync>;
/// İşin sonucunu bekler; süre dolarsa None.
pub type KuyrukBekle = Box<dyn Fn(&str) -> Option<Value> + Send + Sync>;

fn json(durum: u16, v: &Value) -> Yanit {
    Yanit::yeni(
        durum,
        &[("Content-Type", "application/json; charset=utf-8"), ("Cache-Control", "no-store")],
        py_json(v, false).into_bytes(),
    )
}

fn hata(durum: u16, kod: &str, metin: &str) -> Yanit {
    json(durum, &json!({"hata": metin, "kod": kod}))
}

fn gecersiz() -> Yanit {
    hata(400, "gecersiz", "Geçersiz istek.")
}

fn taninmadi() -> Yanit {
    hata(401, "taninmadi", "Cihaz tanınmadı.")
}

/// Cihazın bildirdiği metin: ≤ 200 yazdırılabilir karakter, değilse boş.
fn metin(v: Option<&Value>) -> String {
    match v.and_then(Value::as_str) {
        Some(s) if s.chars().count() <= 200 && s.chars().all(py_yazdirilabilir) => s.to_string(),
        _ => String::new(),
    }
}

fn dize<'a>(b: &'a Value, alan: &str) -> Option<&'a str> {
    b.get(alan).and_then(Value::as_str)
}

fn parola_alanlari(m: &Musteri) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("tuz".into(), json!(m.tuz));
    o.insert("ozet".into(), json!(m.ozet));
    o.insert("yineleme".into(), json!(m.yineleme));
    o
}

fn anahtarlar_gecerli(wg: Option<&str>, ssh: Option<&str>) -> bool {
    wg.is_some_and(kuyruk::wg_gecerli) && ssh.is_some_and(kuyruk::ssh_gecerli)
}

/// Kolay bağlama: bağlanmamış cihazın kalıcı kimliği (32 hex) ve gizli bekleme anahtarı (64 hex).
fn kimlik_gecerli(kimlik: Option<&str>, sir: Option<&str>) -> bool {
    let kucuk_hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    kimlik.is_some_and(|k| kucuk_hex(k, 32)) && sir.is_some_and(|s| kucuk_hex(s, 64))
}

/// Python `MAC_RE` (`[0-9a-f]{2}(:[0-9a-f]{2}){5}`, küçük harf).
fn mac_gecerli(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 17
        && b.iter().enumerate().all(|(i, c)| if i % 3 == 2 { *c == b':' } else { c.is_ascii_digit() || (b'a'..=b'f').contains(c) })
}

/// Python `type(v) is int and v >= 0 and (ust is None or v <= ust)`: bool ve kayan nokta int değildir.
fn tam(v: Option<&Value>, ust: Option<u64>) -> bool {
    v.and_then(Value::as_u64).is_some_and(|n| ust.is_none_or(|u| n <= u))
}

fn bool_mu(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(_)))
}

/// `""` ya da `YYYY-AA-GG` (Python `strptime("%Y-%m-%d")` + `len == 10` + ASCII).
fn tarih_gecerli(v: &Value) -> bool {
    let Some(s) = v.as_str() else { return false };
    if s.is_empty() {
        return true;
    }
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return false;
    }
    let sayi = |a: usize, z: usize| s[a..z].parse::<u32>().unwrap_or(0);
    let yil = sayi(0, 4) as i32;
    yil >= 1 && NaiveDate::from_ymd_opt(yil, sayi(5, 7), sayi(8, 10)).is_some()
}

fn guncelleme_sonucu(g: &Value) -> Option<Value> {
    let o = g.as_object()?;
    if o.len() != 3 || !["surum", "sonuc", "zaman"].iter().all(|k| o.contains_key(*k)) {
        return None;
    }
    let surum = o["surum"].as_str().filter(|s| surumler::surum_gecerli(s))?;
    let sonuc = o["sonuc"].as_str().filter(|s| ["tamam", "geri_alindi", "hata"].contains(s))?;
    if !tam(o.get("zaman"), None) {
        return None;
    }
    Some(json!({"surum": surum, "sonuc": sonuc, "zaman": o["zaman"]}))
}

/// Cihaz sağlık raporunu sıkı doğrular; yalnızca bilinen alanlarla yeni sözlük döner, bozuksa None. İstisna: isteğe bağlı
/// PPPoE alanları (`wan_tur`, `pppoe`) — bilinmeyen/bozuk değer yalnızca o alanı atar, raporu reddetmez.
pub fn saglik_temizle(r: &Value) -> Option<Value> {
    let r = r.as_object()?;
    let sv = r.get("servisler")?.as_object()?;
    if !SAGLIK_SERVISLER.iter().all(|a| bool_mu(sv.get(*a))) {
        return None;
    }
    r.get("surum")?.as_str().filter(|s| s.chars().count() <= 40 && s.chars().all(py_yazdirilabilir))?;
    if !(tam(r.get("acik_sn"), None) && tam(r.get("disk_yuzde"), Some(100)) && tam(r.get("bagli"), None)) {
        return None;
    }
    if !r.get("son_muhur").is_some_and(tarih_gecerli) {
        return None;
    }
    if !SAGLIK_BOOL.iter().all(|a| bool_mu(r.get(*a))) {
        return None;
    }
    let mut t = Map::new();
    for a in ["surum", "acik_sn", "disk_yuzde", "son_muhur", "bagli"] {
        t.insert(a.into(), r[a].clone());
    }
    let mut s = Map::new();
    for a in SAGLIK_SERVISLER {
        s.insert(a.into(), sv[a].clone());
    }
    t.insert("servisler".into(), Value::Object(s));
    for a in SAGLIK_BOOL {
        t.insert(a.into(), r[a].clone());
    }
    if let Some(p) = r.get("paket") {
        // uzaktan güncelleme: cihazda kurulu paket sürümü
        let p = p.as_str().filter(|p| p.chars().count() <= 40 && (p.is_empty() || surumler::surum_gecerli(p)))?;
        t.insert("paket".into(), json!(p));
    }
    if let Some(g) = r.get("guncelleme") {
        // son güncelleme denemesinin sonucu
        let g = if g.is_null() { Value::Null } else { guncelleme_sonucu(g)? };
        t.insert("guncelleme".into(), g);
    }
    // Yukarıdakilerden FARKLI olarak bilinmeyen/bozuk değer raporu reddetmez, yalnızca o alan atılır: yeni bir cihaz sürümü
    // yeni bir durum kodu eklese bile eski merkez raporu kabul eder (reddetseydi çalışan cihaz için "Rapor gelmiyor" uyarısı
    // ve SMS'i çıkardı).
    for (alan, izinli) in [("wan_tur", &SAGLIK_WAN_TURLERI[..]), ("pppoe", &SAGLIK_PPPOE_DURUMLARI[..])] {
        if let Some(d) = r.get(alan).and_then(Value::as_str).filter(|d| izinli.contains(d)) {
            t.insert(alan.into(), json!(d));
        }
    }
    Some(Value::Object(t))
}

fn db_hatasi(ne: &str, h: impl std::fmt::Display) -> Hata {
    Hata::Db(format!("{ne}: {h}"))
}

pub struct Api {
    pub veri: Arc<Veri>,
    saat: Saat,
    pub surum_dizini: PathBuf,
    pub liste_dizini: PathBuf,
    kuyruk_ekle: KuyrukEkle,
    kuyruk_bekle: KuyrukBekle,
    kilit: GirisKilidi,
    hiz: HizSiniri,
    /// Bekleme bildirimi (2 dk'da bir): kimlik başına dakikada 3; bir IP'den saatte en çok 10 yeni kimlik (tablo şişmesin).
    bekleme_hiz: HizSiniri,
    yeni_kimlik_hiz: HizSiniri,
    /// Reddedilen bildirim/bağlama hareket satırı: kimlik başına saatte bir (kapma girişimi görünsün, tablo şişmesin).
    red_hiz: HizSiniri,
    /// NetGSM/Twilio isteği (testte sahte; üretimde curl, `WC_TEST=1` altında reddedilir).
    pub sms_tasiyici: Arc<dyn Tasiyici>,
}

impl Api {
    /// Sürüm ve liste dizinleri üretim yollarıdır; testler alanları değiştirir.
    pub fn new(veri: Arc<Veri>, saat: Saat, kuyruk_ekle: KuyrukEkle, kuyruk_bekle: KuyrukBekle) -> Api {
        Api {
            veri,
            saat,
            surum_dizini: PathBuf::from(surumler::DIZIN),
            liste_dizini: PathBuf::from(listeler::DIZIN),
            kuyruk_ekle,
            kuyruk_bekle,
            kilit: GirisKilidi::default(),
            hiz: HizSiniri::new(20, 60.0),
            bekleme_hiz: HizSiniri::new(3, 60.0),
            yeni_kimlik_hiz: HizSiniri::new(10, 3600.0),
            red_hiz: HizSiniri::new(1, 3600.0),
            // testte varsayılan: istek gitmemeli (gerçek SMS asla); sahte taşıyıcı açıkça verilir
            #[cfg(test)]
            sms_tasiyici: Arc::new(crate::test_yardim::AgYok),
            #[cfg(not(test))]
            sms_tasiyici: Arc::new(crate::ortak::Curl),
        }
    }

    /// Üretim: gerçek kuyruk (`/var/lib/wificorrect/kuyruk`), sonuç bekleme 15 sn.
    pub fn uretim(veri: Arc<Veri>, saat: Saat) -> Api {
        Api::new(
            veri,
            saat,
            Box::new(|islem, alanlar| kuyruk::ekle(islem, alanlar, std::path::Path::new(kuyruk::KUYRUK))),
            Box::new(|kimlik| kuyruk::bekle_uretim(kimlik)),
        )
    }

    fn simdi(&self) -> f64 {
        (self.saat)()
    }

    fn kuyrukla(&self, islem: &str, alanlar: &Value) -> Sonuc<String> {
        (self.kuyruk_ekle)(islem, alanlar).map_err(|h| db_hatasi("kuyruk", h))
    }

    fn musteri(&self, numara: i64) -> Sonuc<Musteri> {
        self.veri.musteri(numara)?.ok_or_else(|| Hata::Db(format!("müşteri yok: {numara}")))
    }

    fn lisans(&self, m: &Musteri) -> Map<String, Value> {
        let (durum, bitis) = self.veri.lisans_durumu(m);
        let mut o = Map::new();
        o.insert("lisans".into(), json!(durum));
        o.insert("lisans_bitis".into(), json!(bitis));
        o
    }

    /// Belirlenmişse tek admin parolasının özeti (açık parola gitmez) ve SMS eki. `sms` yalnızca `etkin_mock`: deneme modu
    /// açık ya da seçili sağlayıcının ayarı eksik (ayar hiç yoksa da). NetGSM bilgisi (kullanıcı kodu, parola) hiçbir cihaza
    /// gitmez (spec bilinçli fark 2). `ap_parola`: müşterinin AP yönetim parolası — cihaz panelindeki AP kurulumunda
    /// gösterilir; müşteriye özeldir (ortak sır değil).
    fn cihaz_ekleri(&self, m: &Musteri) -> Sonuc<Map<String, Value>> {
        let mut ek = Map::new();
        if !m.ap_parola.is_empty() {
            ek.insert("ap_parola".into(), json!(m.ap_parola));
        }
        if let Some(a) = self.veri.admin_parolasi()? {
            ek.insert("admin".into(), json!({"tuz": a.tuz, "ozet": a.ozet, "yineleme": a.yineleme}));
        }
        ek.insert("sms".into(), json!({"etkin_mock": self.veri.sms_etkin_mock()?}));
        Ok(ek)
    }

    pub fn isle(&self, i: &Istek) -> Sonuc<Yanit> {
        const UCLAR: [&str; 10] = [
            "/api/giris",
            "/api/eslesme",
            "/api/parola",
            "/api/saglik",
            "/api/paket",
            "/api/sms",
            "/api/sms-dogrula",
            "/api/liste",
            "/api/bekliyor",
            "/api/bagla",
        ];
        if i.yontem != "POST" || !UCLAR.contains(&i.yol.as_str()) {
            return Ok(hata(404, "yok", "Bulunamadı."));
        }
        if i.govde.len() > GOVDE_SINIRI {
            return Ok(hata(413, "gecersiz", "İstek çok büyük."));
        }
        if !self.hiz.izin(&i.ip, self.simdi()) {
            return Ok(hata(429, "hiz", "Çok fazla istek, biraz sonra tekrar deneyin."));
        }
        let b: Value = match serde_json::from_str(&i.govde) {
            Ok(v @ Value::Object(_)) => v,
            _ => return Ok(hata(400, "gecersiz", "Geçersiz istek.")),
        };
        let ip = i.ip.as_str();
        match i.yol.as_str() {
            "/api/giris" => self.giris(&b, ip),
            "/api/eslesme" => self.eslesme(&b, ip),
            "/api/parola" => self.parola(&b, ip),
            "/api/saglik" => self.saglik(&b, ip),
            "/api/paket" => self.paket(&b),
            "/api/sms" => self.sms(&b),
            "/api/sms-dogrula" => self.sms_dogrula(&b),
            "/api/liste" => self.liste(&b),
            "/api/bekliyor" => self.bekliyor(&b, ip),
            _ => self.bagla(&b, ip),
        }
    }

    fn cihaz(&self, b: &Value) -> Sonuc<Option<crate::veri::Cihaz>> {
        self.veri.cihaz_anahtarla(dize(b, "cihaz_anahtari").unwrap_or(""))
    }

    fn giris(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let simdi = self.simdi();
        let (numara, pw) = (dize(b, "numara"), dize(b, "parola"));
        let (Some(numara), Some(pw)) = (numara.filter(|n| guvenlik::numara_gecerli(n)), pw) else {
            return Ok(hata(400, "gecersiz", "Müşteri numarası 7 haneli olmalı."));
        };
        if !self.kilit.attempt(ip, numara, simdi) {
            return Ok(hata(429, "kilit", "Çok fazla deneme. 15 dakika sonra tekrar deneyin."));
        }
        let n: i64 = numara.parse().unwrap_or(0);
        let Some(m) = self.veri.parola_dogrula(n, pw)? else {
            if self.veri.musteri(n)?.is_some() {
                self.veri.hareket(numara, ip, "CIHAZ_GIRIS_HATALI", Some(n), "")?;
            }
            return Ok(hata(401, "hatali", "Müşteri numarası veya parola hatalı."));
        };
        self.kilit.succeed(ip, numara, simdi);
        if m.uyelik != "aktif" {
            return Ok(hata(403, "uyelik", "Üyeliğiniz sona ermiş. Hizmet sağlayıcınıza başvurun."));
        }
        let (wg, ssh) = (dize(b, "wg_pub"), dize(b, "ssh_pub"));
        if !anahtarlar_gecerli(wg, ssh) {
            return Ok(hata(400, "gecersiz", "Cihaz anahtarları geçersiz."));
        }
        self.bagla_ortak(&m, wg.unwrap_or(""), ssh.unwrap_or(""), b, ip, None)
    }

    /// `/api/giris` ve `/api/bagla` ortak: cihazı bağlar, tünel + yedek hesabını kuyrukla açar. `otomatik`: bekleme kimliği
    /// (kolay bağlama; cevaba müşteri numarası da girer, cihaz numarayı bilmez).
    fn bagla_ortak(&self, m: &Musteri, wg: &str, ssh: &str, b: &Value, ip: &str, otomatik: Option<&str>) -> Sonuc<Yanit> {
        let numara = m.numara.to_string();
        let bagli = self.veri.cihaz_bagla(
            m.numara,
            wg,
            ssh,
            &metin(b.get("isletme_adi")),
            &metin(b.get("unvan")),
            &metin(b.get("surum")),
        );
        let (cid, anahtar) = match bagli {
            Ok(x) => x,
            Err(Hata::BaskaCihaz) => {
                self.veri.hareket(&numara, ip, "CIHAZ_REDDEDILDI", Some(m.numara), "numara başka cihazda")?;
                return Ok(hata(409, "baska_cihaz", "Bu numara başka bir cihazda kullanılıyor."));
            }
            Err(h) => return Err(h),
        };
        if let Some(k) = otomatik {
            // kuyruktan ÖNCE: kayıt başarısız olsa da bekleme kaydı bu bağa işaret eder, cihaz sonraki bildiriminde (aynı wg
            // anahtarıyla) yeni belirteç alıp yeniden dener; işaret eşitlemede ortak SMS sırrını keser
            self.veri.cihaz_otomatik_isaretle(cid)?;
            self.veri.bekleyen_baglandi(k, cid)?;
        }
        let is_no = self.kuyrukla("cihaz-ekle", &json!({"numara": numara, "wg_pub": wg, "ssh_pub": ssh}))?;
        let s = (self.kuyruk_bekle)(&is_no);
        let tamam = s.as_ref().filter(|s| s.as_object().is_some_and(|o| !o.is_empty()) && dize(s, "durum") == Some("tamam"));
        let Some(s) = tamam else {
            let neden = s.as_ref().and_then(|s| s.get("hata")).map(py_str).unwrap_or_else(|| "zaman aşımı".into());
            self.veri.hareket(&numara, ip, "CIHAZ_KAYIT_HATA", Some(m.numara), &neden)?;
            return Ok(hata(503, "kayit", "Cihaz kaydı tamamlanamadı, biraz sonra tekrar deneyin."));
        };
        let (Some(tunel), Some(sunucu_pub), Some(uc_nokta)) = (s.get("tunel_ip"), s.get("sunucu_pub"), s.get("uc_nokta")) else {
            return Err(Hata::Db("kuyruk sonucu eksik".into()));
        };
        let tunel_metin = py_str(tunel);
        self.veri.cihaz_guncelle(cid, &[("tunel_ip", tunel_metin.as_str())])?;
        let mut ek = String::new();
        if let Some(k) = otomatik {
            self.veri.bekleyen_baglandi(k, cid)?; // "hazırlanıyor" süresi başarılı bağdan sayılır
            ek = format!(" otomatik kimlik={}", k.chars().take(8).collect::<String>());
        }
        self.veri.hareket(&numara, ip, "CIHAZ_BAGLANDI", Some(m.numara), &format!("cihaz={cid} tunel={tunel_metin}{ek}"))?;
        let mut y = parola_alanlari(m);
        y.extend(self.lisans(m));
        y.extend(self.cihaz_ekleri(m)?);
        y.insert("cihaz_anahtari".into(), json!(anahtar));
        y.insert("tunel_ip".into(), tunel.clone());
        y.insert("sunucu_pub".into(), sunucu_pub.clone());
        y.insert("uc_nokta".into(), uc_nokta.clone());
        y.insert("yedek_hedefi".into(), json!(format!("wfc-{numara}@{YEDEK_SUNUCU}:")));
        if otomatik.is_some() {
            y.insert("numara".into(), json!(numara));
        }
        Ok(json(200, &Value::Object(y)))
    }

    // --- kolay bağlama (spec 2026-10-16) ---
    /// Bağlanmamış cihaz 2 dk'da bir: `{durum: bekle}` ya da `{durum: baglan, belirtec}` (müşteri aynı IP'den girip bu cihazı
    /// aldıysa). Sır tutmazsa kayıt değişmez.
    fn bekliyor(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let (kimlik, sir, wg, ssh) = (dize(b, "kimlik"), dize(b, "sir"), dize(b, "wg_pub"), dize(b, "ssh_pub"));
        let mac = match b.get("wan_mac") {
            None => Some(""),
            Some(v) => v.as_str(),
        };
        if !(kimlik_gecerli(kimlik, sir) && anahtarlar_gecerli(wg, ssh) && mac.is_some_and(|m| m.is_empty() || mac_gecerli(m))) {
            return Ok(gecersiz());
        }
        let (kimlik, sir, wg, ssh, mac) = (kimlik.unwrap_or(""), sir.unwrap_or(""), wg.unwrap_or(""), ssh.unwrap_or(""), mac.unwrap_or(""));
        let simdi = self.simdi();
        let cok_istek = || hata(429, "hiz", "Çok fazla istek, biraz sonra tekrar deneyin.");
        if !self.bekleme_hiz.izin(kimlik, simdi) {
            return Ok(cok_istek());
        }
        if !self.veri.bekleyen_var(kimlik)? && !self.yeni_kimlik_hiz.izin(ip, simdi) {
            return Ok(cok_istek());
        }
        let surum: String = metin(b.get("surum")).chars().take(40).collect();
        let (durum, yeni, belirtec) = self.veri.bekleyen_bildir(kimlik, sir, ip, &surum, mac, wg, ssh)?;
        let k8: String = kimlik.chars().take(8).collect();
        match durum {
            "taninmadi" => {
                self.red(kimlik, ip, "CIHAZ_BEKLIYOR_RED")?;
                return Ok(taninmadi());
            }
            "dolu" => return Ok(hata(503, "dolu", "Şu an kayıt alınamıyor, biraz sonra tekrar deneyin.")),
            _ => {}
        }
        if yeni {
            let mac_g = if mac.is_empty() { "-" } else { mac };
            self.veri.hareket(&format!("bekleyen:{k8}"), ip, "CIHAZ_BEKLIYOR", None, &format!("kimlik={k8} surum={surum} mac={mac_g}"))?;
        }
        if durum == "baglan" {
            return Ok(json(200, &json!({"durum": "baglan", "belirtec": belirtec})));
        }
        Ok(json(200, &json!({"durum": "bekle"})))
    }

    /// Sırrı ya da belirteci tutmayan deneme (sır/belirteç yazılmaz); kimlik + olay başına saatte bir satır.
    fn red(&self, kimlik: &str, ip: &str, olay: &str) -> Sonuc<()> {
        if self.red_hiz.izin(&format!("{olay}:{kimlik}"), self.simdi()) {
            let k8: String = kimlik.chars().take(8).collect();
            self.veri.hareket(&format!("bekleyen:{k8}"), ip, olay, None, &format!("kimlik={k8}"))?;
        }
        Ok(())
    }

    /// Ayrılmış bekleyen cihaz, bekliyor cevabındaki tek kullanımlık belirteçle parolasız bağlanır (`/api/giris` gibi).
    fn bagla(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let (kimlik, sir, belirtec, wg, ssh) = (dize(b, "kimlik"), dize(b, "sir"), dize(b, "belirtec"), dize(b, "wg_pub"), dize(b, "ssh_pub"));
        let (true, Some(belirtec), Some(kimlik), Some(sir)) = (anahtarlar_gecerli(wg, ssh), belirtec, kimlik, sir) else {
            return Ok(gecersiz());
        };
        if !kimlik_gecerli(Some(kimlik), Some(sir)) {
            return Ok(gecersiz());
        }
        let Some(r) = self.veri.bekleyen_belirtec_tuket(kimlik, sir, belirtec)? else {
            self.red(kimlik, ip, "CIHAZ_BAGLA_RED")?;
            return Ok(hata(401, "taninmadi", "Bağlama belirteci geçersiz."));
        };
        let m = match r.musteri {
            Some(n) => self.veri.musteri(n)?,
            None => None,
        };
        let Some(m) = m.filter(|m| m.uyelik == "aktif") else {
            return Ok(hata(403, "uyelik", "Üyelik sona ermiş."));
        };
        self.bagla_ortak(&m, wg.unwrap_or(""), ssh.unwrap_or(""), b, ip, Some(kimlik))
    }

    fn eslesme(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let Some(c) = self.cihaz(b)? else { return Ok(taninmadi()) };
        if c.durum == "serbest_birakiliyor" {
            if b.get("temizlendi") == Some(&Value::Bool(true)) {
                self.veri.temizlendi(c.id)?;
                // sonucu beklenmez
                self.kuyrukla("cihaz-kapat", &json!({"numara": c.musteri.to_string(), "wg_pub": c.wg_pub}))?;
                self.veri.hareket(&format!("cihaz:{}", c.id), ip, "CIHAZ_SERBEST", Some(c.musteri), "")?;
            }
            return Ok(json(200, &json!({"durum": "serbest"})));
        }
        let adres = b.get("adres").map(|a| metin(Some(a)));
        self.veri.eslesme_kaydet(
            c.id,
            &metin(b.get("isletme_adi")),
            &metin(b.get("unvan")),
            &metin(b.get("surum")),
            dize(b, "isletme_turu"),
            adres.as_deref(),
        )?;
        let m = self.musteri(c.musteri)?;
        let mut y = parola_alanlari(&m);
        y.extend(self.lisans(&m));
        y.extend(self.cihaz_ekleri(&m)?);
        y.insert("durum".into(), json!("bagli"));
        y.insert("uyelik".into(), json!(m.uyelik));
        Ok(json(200, &Value::Object(y)))
    }

    fn parola(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let Some(c) = self.cihaz(b)? else { return Ok(taninmadi()) };
        if !self.kilit.attempt(ip, &format!("cihaz:{}", c.id), self.simdi()) {
            return Ok(hata(429, "kilit", "Çok fazla deneme. 15 dakika sonra tekrar deneyin."));
        }
        let (Some(eski), Some(yeni)) = (dize(b, "eski"), dize(b, "yeni")) else {
            return Ok(gecersiz());
        };
        if self.veri.parola_dogrula(c.musteri, eski)?.is_none() {
            return Ok(hata(401, "hatali", "Mevcut parola yanlış."));
        }
        let (tuz, oz, y) = match self.veri.parola_koy(c.musteri, yeni) {
            Ok(x) => x,
            Err(Hata::Deger(m)) => return Ok(hata(400, "gecersiz", &m)),
            Err(h) => return Err(h),
        };
        self.veri.hareket(&c.musteri.to_string(), ip, "PAROLA_DEGISTI", Some(c.musteri), "cihazdan")?;
        Ok(json(200, &json!({"tuz": tuz, "ozet": oz, "yineleme": y})))
    }

    /// Cihazın son sağlık raporundaki alan (kayıt ya da rapor yoksa None).
    fn rapor_alani(&self, cid: i64, alan: &str) -> Sonuc<Option<Value>> {
        Ok(self.veri.saglik(cid)?.and_then(|k| k.rapor.get(alan).cloned()))
    }

    fn saglik(&self, b: &Value, ip: &str) -> Sonuc<Yanit> {
        let Some(c) = self.cihaz(b)? else { return Ok(taninmadi()) };
        let Some(r) = b.get("rapor").and_then(saglik_temizle) else {
            return Ok(hata(400, "gecersiz", "Geçersiz rapor."));
        };
        if c.durum != "serbest_birakiliyor" {
            // serbest bırakılan cihazın raporu yazılmaz
            let onceki = self.rapor_alani(c.id, "guncelleme")?;
            self.veri.saglik_kaydet(c.id, &r)?;
            if let Some(g) = r.get("guncelleme").filter(|g| !g.is_null()) {
                let zaman = g["zaman"].as_u64().unwrap_or(0) as f64;
                if zaman > self.musteri(c.musteri)?.guncelleme_hemen {
                    self.veri.guncelleme_hemen_koy(c.musteri, 0.0, "")?; // "hemen" isteği bu denemeyle tüketildi
                }
                if Some(g) != onceki.as_ref() {
                    let ayrinti = format!("surum={} sonuc={}", py_str(&g["surum"]), py_str(&g["sonuc"]));
                    self.veri.hareket(&format!("cihaz:{}", c.id), ip, "GUNCELLEME_SONUC", Some(c.musteri), &ayrinti)?;
                }
            }
        }
        // 06:00 eşitlemesini beklemesin
        let mut yanit = Map::new();
        yanit.insert("tamam".into(), json!(true));
        yanit.insert("sms_etkin_mock".into(), json!(self.veri.sms_etkin_mock()?));
        let m = self.musteri(c.musteri)?;
        if c.durum == "bagli" && self.veri.hizmet_acik(&m) {
            // /api/liste ile aynı koşul: indiremeyeceği sürüm bildirilmez
            yanit.insert("listeler".into(), listeler::ozet(&self.liste_dizini));
        }
        if c.durum == "bagli" && !m.ap_parola.is_empty() {
            // AP yönetim parolası (spec 2026-10-23): yönetimde "Yenile"den 5 dk sonra cihaz yeni parolayı bilsin (giriş
            // sayfasındaki Servis bu parolayı ister); /api/eslesme gibi lisans durumuna bakmaz. Bir sır: hareket satırına ya
            // da günlüğe yazılmaz.
            yanit.insert("ap_parola".into(), json!(m.ap_parola));
        }
        let mut hedef = self.veri.cihaz_hedefi(&m)?;
        let paket = r.get("paket").and_then(Value::as_str);
        if m.guncelleme_hemen > 0.0
            && hedef.as_ref().is_none_or(|h| m.guncelleme_hemen_surum != h.surum || paket == Some(h.surum.as_str()))
        {
            // sonuç gelmeyecek (hedef yok/değişti, cihaz zaten hedefte): bayrak kalıp sonra pencereyi atlamasın
            self.veri.guncelleme_hemen_koy(c.musteri, 0.0, "")?;
            if let Some(h) = hedef.as_mut() {
                h.hemen = false;
            }
        }
        if let Some(h) = hedef {
            let mut g = Map::new();
            g.insert("surum".into(), json!(h.surum));
            g.insert("sha256".into(), json!(h.sha256));
            g.insert("boyut".into(), json!(h.boyut));
            g.insert("hemen".into(), json!(h.hemen));
            if let Some(p) = paket.filter(|p| !p.is_empty()) {
                // geri dönüş paketi (cihazda kurulu sürüm) depoda: cihaz indirdiğini bununla doğrular
                if let Some(k) = self.veri.surum(p)? {
                    g.insert("onceki_sha256".into(), json!(k.sha256));
                }
            }
            yanit.insert("guncelleme".into(), Value::Object(g));
        }
        Ok(json(200, &Value::Object(yanit)))
    }

    fn paket(&self, b: &Value) -> Sonuc<Yanit> {
        let Some(c) = self.cihaz(b)? else { return Ok(taninmadi()) };
        let yetki = || hata(403, "yetki", "Bu paket için yetkiniz yok.");
        let m = self.musteri(c.musteri)?;
        // bırakılan / lisansı ya da üyeliği kapalı cihaza paket verilmez
        if c.durum != "bagli" || !self.veri.hizmet_acik(&m) {
            return Ok(yetki());
        }
        let hedef = self.veri.cihaz_hedefi(&m)?;
        let kurulu = self.rapor_alani(c.id, "paket")?;
        let Some(s) = dize(b, "surum").filter(|s| surumler::surum_gecerli(s)) else { return Ok(yetki()) };
        // ikincisi geri dönüş paketi için
        if !(hedef.as_ref().is_some_and(|h| h.surum == s) || kurulu.as_ref().and_then(Value::as_str) == Some(s)) {
            return Ok(yetki());
        }
        let Ok(yol) = surumler::paket_yolu(&self.surum_dizini, s) else { return Ok(yetki()) };
        match std::fs::read(yol) {
            Ok(icerik) => Ok(Yanit::yeni(200, &[("Content-Type", "application/octet-stream"), ("Cache-Control", "no-store")], icerik)),
            Err(_) => Ok(hata(404, "yok", "Paket bulunamadı.")),
        }
    }

    // --- doğrulama SMS'i merkezden: NetGSM OTP ya da Twilio Verify ---
    /// (cihaz, telefon, kod) ya da hata yanıtı. Kod hiçbir yere yazılmaz.
    fn sms_cihazi(&self, b: &Value) -> Sonuc<Result<(crate::veri::Cihaz, String, String), Yanit>> {
        let Some(c) = self.cihaz(b)? else { return Ok(Err(taninmadi())) };
        let m = self.musteri(c.musteri)?;
        if c.durum != "bagli" || !self.veri.hizmet_acik(&m) {
            return Ok(Err(hata(403, "yetki", "Bu işlem için yetkiniz yok.")));
        }
        let hane = |v: Option<&str>, uzunluk: std::ops::RangeInclusive<usize>| {
            v.filter(|s| uzunluk.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())).map(str::to_string)
        };
        match (hane(dize(b, "telefon"), 8..=15), hane(dize(b, "kod"), 6..=6)) {
            (Some(tel), Some(kod)) => Ok(Ok((c, tel, kod))),
            _ => Ok(Err(gecersiz())),
        }
    }

    /// Türk cep numarası → `tr_saglayici`; yabancı → Twilio açıksa twilio, değilse None.
    fn saglayici(s: &Value, tel: &str) -> Option<String> {
        if twilio::tr_mi(tel) {
            return Some(py_str(&s["tr_saglayici"]));
        }
        s["twilio"].get("acik").is_some_and(py_dogru).then(|| "twilio".to_string())
    }

    fn sms(&self, b: &Value) -> Sonuc<Yanit> {
        let (c, tel, kod) = match self.sms_cihazi(b)? {
            Ok(t) => t,
            Err(h) => return Ok(h),
        };
        let s = self.veri.sms_ayari()?;
        let sonuc = |saglayici: Option<&str>, sonuc_: &str, kod_: &str, is_no: &str| -> Sonuc<Yanit> {
            if sonuc_ != "mock" {
                // müşteri başına tek satır (her SMS'e hareket satırı geçmişi boğardı); sağlık izleme okur
                self.veri.sms_son_kaydet(c.musteri, saglayici.unwrap_or(""), sonuc_, kod_)?;
            }
            let mut y = Map::new();
            y.insert("sonuc".into(), json!(sonuc_));
            y.insert("kod".into(), json!(kod_));
            if !is_no.is_empty() {
                y.insert("is".into(), json!(is_no));
            }
            if let Some(sg) = saglayici {
                y.insert("saglayici".into(), json!(sg));
            }
            Ok(json(200, &Value::Object(y)))
        };
        let Some(s) = s else { return sonuc(None, "hata", "AYAR", "") };
        let Some(saglayici) = Self::saglayici(&s, &tel) else { return sonuc(None, "hata", "YABANCI", "") };
        if s.get("mock").is_some_and(py_dogru) {
            // deneme modu: hiçbir sağlayıcıya istek gitmez, cihaz kodu yerelde doğrular
            return sonuc(Some(&saglayici), "mock", "0", "");
        }
        if self.sinir_doldu(c.musteri, !twilio::tr_mi(&tel))? {
            return sonuc(Some(&saglayici), "hata", "SINIR", "");
        }
        if saglayici == "twilio" {
            let (r, k) = twilio::gonder(&s["twilio"], &tel, self.sms_tasiyici.as_ref());
            return sonuc(Some(&saglayici), &r, &k, "");
        }
        let metin = OTP_SABLON.replace("{kod}", &kod);
        let (r, k, is_no) = netgsm::otp_gonder(&s, &tel[2..], &metin, self.sms_tasiyici.as_ref());
        sonuc(Some(&saglayici), &r, &k, &is_no)
    }

    /// Günlük sayaçlar (toplam 300; yabancı numaraysa ayrıca 20). Dolmadıysa hepsi bir artar. Yarış: iki istek aynı anda
    /// sınırın bir altını görürse artırma sonrası sınırı aşan reddedilir (ikinci denetim).
    fn sinir_doldu(&self, musteri: i64, yabanci: bool) -> Sonuc<bool> {
        let gun = self.veri.bugun().to_string();
        let mut sayaclar = vec![(gun.clone(), SMS_GUNLUK_SINIR)];
        if yabanci {
            sayaclar.push((format!("{gun}{YABANCI_SAYAC}"), SMS_YABANCI_SINIR));
        }
        for (g, sinir) in &sayaclar {
            if self.veri.sms_sayac(musteri, g)? >= *sinir {
                return Ok(true);
            }
        }
        let mut doldu = false;
        for (g, sinir) in &sayaclar {
            doldu |= self.veri.sms_sayac_artir(musteri, g)? > *sinir; // hepsi artsın
        }
        Ok(doldu)
    }

    /// Twilio Verify kodu: {onay} ya da ağ hatasında hata/AG (misafirin deneme hakkı düşmesin diye ayrı).
    fn sms_dogrula(&self, b: &Value) -> Sonuc<Yanit> {
        let (_, tel, kod) = match self.sms_cihazi(b)? {
            Ok(t) => t,
            Err(h) => return Ok(h),
        };
        let ayar = || json(200, &json!({"sonuc": "hata", "kod": "AYAR"}));
        let Some(s) = self.veri.sms_ayari()? else { return Ok(ayar()) };
        if s.get("mock").is_some_and(py_dogru) || Self::saglayici(&s, &tel).as_deref() != Some("twilio") {
            return Ok(ayar());
        }
        let (onay, k) = twilio::dogrula(&s["twilio"], &tel, &kod, self.sms_tasiyici.as_ref());
        Ok(match onay {
            None => json(200, &json!({"sonuc": "hata", "kod": k})),
            Some(o) => json(200, &json!({"onay": o})),
        })
    }

    /// Hazır engel listesi (gzip, düz metnin sha256'sı sağlık cevabında). Yalnızca bağlı ve hizmeti açık cihaza.
    fn liste(&self, b: &Value) -> Sonuc<Yanit> {
        let Some(c) = self.cihaz(b)? else { return Ok(taninmadi()) };
        let Some(k) = dize(b, "kategori").filter(|k| listeler::kategori_gecerli(k)) else {
            return Ok(hata(400, "gecersiz", "Bilinmeyen kategori."));
        };
        if c.durum != "bagli" || !self.veri.hizmet_acik(&self.musteri(c.musteri)?) {
            return Ok(hata(403, "yetki", "Bu liste için yetkiniz yok."));
        }
        let Ok(yol) = listeler::liste_yolu(&self.liste_dizini, k) else { return Ok(hata(404, "yok", "Liste bulunamadı.")) };
        match std::fs::read(yol) {
            Ok(icerik) => Ok(Yanit::yeni(200, &[("Content-Type", "application/gzip"), ("Cache-Control", "no-store")], icerik)),
            Err(_) => Ok(hata(404, "yok", "Liste bulunamadı.")),
        }
    }
}

impl Uygulama for Api {
    fn istek(&self, i: &Istek) -> Yanit {
        match self.isle(i) {
            Ok(y) => y,
            Err(h) => hata_yaniti(&i.yontem, &i.yol, &format!("{h:?}")),
        }
    }
}

#[cfg(test)]
#[path = "api_testler.rs"]
mod testler;

#[cfg(test)]
#[path = "api_sms_testler.rs"]
mod sms_testler;
