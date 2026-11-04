//! Yönetim merkezi veritabanı (Python veri.py): müşteriler, cihazlar, hareketler, eski arşiv bağları.
//! SQLite; tek bağlantı + kilit (iş parçacıkları için). Şema, sorgular ve işlem sınırları Python'la aynıdır; satırlar sütunlara
//! ADLA okunur (canlı veritabanında sonradan eklenen sütunlar tablonun sonundadır).
//!
//! Python'dan farklar: `parola_sifirla` ve `ALTER TABLE` göçleri/`_gocur` düzeltmeleri taşınmadı (canlı envanter: bütün sütunlar
//! var, boş lisans/AP parolası yok); eksik tablo ya da sütun `Veri::ac` içinde `sema_denetle` ile hata verir.
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{NaiveDate, TimeZone};
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection, OptionalExtension, Params, Row, Transaction, TransactionBehavior};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::guvenlik::{self, rastgele};
use crate::ortak::{self, py_dogru, py_str, py_veya_bos};
use crate::twilio;

pub const NUMARA_ARALIGI: (i64, i64) = (1_000_000, 9_999_999); // 7 haneli, rastgele (sıralı değil)
/// Tam şema: Python'da yalnızca `ALTER` ile gelen sütunlar (`YENI_SUTUNLAR`, `YENI_CIHAZ_SUTUNLARI`) CREATE içinde.
const SEMA: &str = "
CREATE TABLE IF NOT EXISTS musteri (
  numara INTEGER PRIMARY KEY, tuz TEXT NOT NULL, ozet TEXT NOT NULL, yineleme INTEGER NOT NULL,
  not_ TEXT NOT NULL DEFAULT '', uyelik TEXT NOT NULL DEFAULT 'aktif', bitis TEXT NOT NULL DEFAULT '',
  olusturma TEXT NOT NULL, son_giris TEXT NOT NULL DEFAULT '',
  parola_acik TEXT NOT NULL DEFAULT '', lisans_bitis TEXT NOT NULL DEFAULT '', askida INTEGER NOT NULL DEFAULT 0,
  guncelleme_kanali TEXT NOT NULL DEFAULT 'kararli', guncelleme_hemen REAL NOT NULL DEFAULT 0,
  guncelleme_hemen_surum TEXT NOT NULL DEFAULT '', ap_parola TEXT NOT NULL DEFAULT '');
CREATE TABLE IF NOT EXISTS cihaz (
  id INTEGER PRIMARY KEY AUTOINCREMENT, musteri INTEGER NOT NULL REFERENCES musteri(numara),
  durum TEXT NOT NULL, tunel_ip TEXT NOT NULL DEFAULT '', wg_pub TEXT NOT NULL, ssh_pub TEXT NOT NULL,
  anahtar_ozet TEXT NOT NULL, isletme_adi TEXT NOT NULL DEFAULT '', unvan TEXT NOT NULL DEFAULT '',
  surum TEXT NOT NULL DEFAULT '', son_eslesme TEXT NOT NULL DEFAULT '', baglanma TEXT NOT NULL,
  admin_parola TEXT NOT NULL DEFAULT '', admin_tuz TEXT NOT NULL DEFAULT '',
  admin_ozet TEXT NOT NULL DEFAULT '', admin_yineleme INTEGER NOT NULL DEFAULT 0,
  ayrilma TEXT NOT NULL DEFAULT '', isletme_turu TEXT NOT NULL DEFAULT '', adres TEXT NOT NULL DEFAULT '',
  otomatik INTEGER NOT NULL DEFAULT 0);
CREATE UNIQUE INDEX IF NOT EXISTS cihaz_tek ON cihaz(musteri) WHERE durum != 'serbest';
CREATE TABLE IF NOT EXISTS hareket (
  id INTEGER PRIMARY KEY AUTOINCREMENT, zaman TEXT NOT NULL, kim TEXT NOT NULL, ip TEXT NOT NULL,
  olay TEXT NOT NULL, musteri INTEGER, ayrinti TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS hareket_musteri ON hareket(musteri, id);
CREATE TABLE IF NOT EXISTS eski_arsiv (ad TEXT PRIMARY KEY, musteri INTEGER REFERENCES musteri(numara));
CREATE TABLE IF NOT EXISTS ayar (anahtar TEXT PRIMARY KEY, deger TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS saglik (cihaz INTEGER PRIMARY KEY, zaman TEXT NOT NULL, rapor TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sorun (
  id INTEGER PRIMARY KEY AUTOINCREMENT, musteri INTEGER NOT NULL, tur TEXT NOT NULL, metin TEXT NOT NULL,
  onem TEXT NOT NULL, acilis TEXT NOT NULL, kapanis TEXT NOT NULL DEFAULT '',
  acilis_sms TEXT NOT NULL DEFAULT '', kapanis_sms TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS sorun_musteri ON sorun(musteri, kapanis);
CREATE TABLE IF NOT EXISTS surum (
  surum TEXT PRIMARY KEY, sha256 TEXT NOT NULL, boyut INTEGER NOT NULL, kaynak TEXT NOT NULL, zaman TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sms_sayac (
  musteri INTEGER NOT NULL, gun TEXT NOT NULL, sayi INTEGER NOT NULL, PRIMARY KEY (musteri, gun));
CREATE TABLE IF NOT EXISTS sms_son (
  musteri INTEGER PRIMARY KEY, zaman TEXT NOT NULL, saglayici TEXT NOT NULL, sonuc TEXT NOT NULL, kod TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bekleyen (
  kimlik TEXT PRIMARY KEY, sir_ozet TEXT NOT NULL, ilk REAL NOT NULL, son REAL NOT NULL, ip TEXT NOT NULL,
  surum TEXT NOT NULL DEFAULT '', wan_mac TEXT NOT NULL DEFAULT '', wg_pub TEXT NOT NULL, ssh_pub TEXT NOT NULL,
  musteri INTEGER, ayrilma REAL NOT NULL DEFAULT 0, belirtec_ozet TEXT NOT NULL DEFAULT '',
  belirtec_bitis REAL NOT NULL DEFAULT 0, cihaz INTEGER, baglandi REAL NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS bekleyen_ip ON bekleyen(ip, son);
";
/// Gerekli tablolar ve sütunlar (`sema_denetle`).
pub const SUTUNLAR: [(&str, &[&str]); 11] = [
    ("musteri", &["numara", "tuz", "ozet", "yineleme", "not_", "uyelik", "bitis", "olusturma", "son_giris", "parola_acik",
        "lisans_bitis", "askida", "guncelleme_kanali", "guncelleme_hemen", "guncelleme_hemen_surum", "ap_parola"]),
    ("cihaz", &["id", "musteri", "durum", "tunel_ip", "wg_pub", "ssh_pub", "anahtar_ozet", "isletme_adi", "unvan", "surum",
        "son_eslesme", "baglanma", "admin_parola", "admin_tuz", "admin_ozet", "admin_yineleme", "ayrilma", "isletme_turu",
        "adres", "otomatik"]),
    ("hareket", &["id", "zaman", "kim", "ip", "olay", "musteri", "ayrinti"]),
    ("eski_arsiv", &["ad", "musteri"]),
    ("ayar", &["anahtar", "deger"]),
    ("saglik", &["cihaz", "zaman", "rapor"]),
    ("sorun", &["id", "musteri", "tur", "metin", "onem", "acilis", "kapanis", "acilis_sms", "kapanis_sms"]),
    ("surum", &["surum", "sha256", "boyut", "kaynak", "zaman"]),
    ("sms_sayac", &["musteri", "gun", "sayi"]),
    ("sms_son", &["musteri", "zaman", "saglayici", "sonuc", "kod"]),
    ("bekleyen", &["kimlik", "sir_ozet", "ilk", "son", "ip", "surum", "wan_mac", "wg_pub", "ssh_pub", "musteri", "ayrilma",
        "belirtec_ozet", "belirtec_bitis", "cihaz", "baglandi"]),
];

// Kolay bağlama (spec 2026-10-16): bağlanmamış cihazın bekleme kaydı
pub const BEKLEYEN_SIL_SN: f64 = 7.0 * 86400.0; // bu kadar bildirmeyen kayıt silinir
pub const BEKLEYEN_ESKI_SN: f64 = 1800.0; // yönetimde "eski"
pub const BEKLEYEN_ADAY_SN: f64 = 600.0; // müşteri girişinde aday: son bu kadar süre içinde bildirmiş
pub const AYRILMA_SN: f64 = 900.0; // ayrılmış ama bu sürede bağlanmamış cihaz serbest kalır
pub const BELIRTEC_SN: f64 = 600.0; // tek kullanımlık bağlama belirteci
pub const BEKLEYEN_EN_COK: i64 = 2000;
pub const CIHAZ_ALANLARI: [&str; 7] = ["tunel_ip", "isletme_adi", "unvan", "adres", "surum", "son_eslesme", "isletme_turu"];
/// Cihazın eşitlemede bildirdiği işletme türü (cihaz ayarı main.isletme_turu); başka değer saklanmaz.
pub const ISLETME_TURLERI: [(&str, &str); 4] = [("kafe", "Kafe/Restoran"), ("otel", "Otel/Pansiyon"), ("yurt", "Yurt"), ("diger", "Diğer")];
/// AP yönetim parolası: müşteri başına rastgele, panel parolasından ayrı; harfler karışmaz, simge yok, 12 karakter.
pub const AP_PAROLA_UZUNLUK: usize = 12;
pub const GUNCELLEME_KANALLARI: [&str; 3] = ["kararli", "test", "kapali"];
/// Kanal hedefi "her zaman en son sürüm": GitHub'dan alınmış en yüksek sürüm (elle eklenen yerel paketler girmez).
pub const EN_SON: &str = "en-son";
pub const UYARI_TELEFON_EN_COK: usize = 3;
pub const UYARI_SINIR_ARALIK: (i64, i64) = (2, 100); // günlük uyarı SMS sınırı (yönetim formu da bunu kullanır)
pub const SORUN_SMS_ALANLARI: [&str; 2] = ["acilis_sms", "kapanis_sms"];
pub const SMS_ALANLARI: [&str; 4] = ["usercode", "password", "msgheader", "appkey"];
pub const TR_SAGLAYICILAR: [&str; 2] = ["netgsm", "twilio"];
pub const TWILIO_ALANLARI: [&str; 5] = ["account_sid", "auth_token", "api_key_sid", "api_key_secret", "verify_sid"];
pub const TWILIO_SIRLARI: [&str; 2] = ["auth_token", "api_key_secret"];
pub const LISANS_GUN: i64 = 365;
/// Raporlar: müşteri → bağlı eski arşivler (root yalnızca bunu görür, veritabanını açmaz).
pub const RAPOR_GRUPLARI: &str = "/var/lib/wificorrect/merkez/rapor-gruplari.json";

/// Python `UYARI_VARSAYILAN`.
pub fn uyari_varsayilan() -> Value {
    serde_json::json!({"telefonlar": [], "acik": false, "gunluk_sinir": 20})
}

/// Python `TWILIO_VARSAYILAN`.
pub fn twilio_varsayilan() -> Value {
    let mut m = Map::new();
    m.insert("acik".into(), Value::Bool(false));
    for k in TWILIO_ALANLARI {
        m.insert(k.into(), Value::String(String::new()));
    }
    Value::Object(m)
}

/// Seçili Türk numarası sağlayıcısının ayarı tam mı (deneme modu kapalıyken gerçekten gönderilebilir mi).
pub fn sms_hazir(s: &Value) -> bool {
    let sag = s.get("tr_saglayici").filter(|v| py_dogru(v)).map(py_str).unwrap_or_else(|| "netgsm".into());
    if sag == "twilio" {
        let t = s.get("twilio").filter(|v| py_dogru(v)).cloned().unwrap_or_else(|| Value::Object(Map::new()));
        return t.get("acik").is_some_and(py_dogru)
            && twilio::sid_gecerli("verify_sid", &py_veya_bos(t.get("verify_sid")))
            && twilio::kimlik(&t).is_some();
    }
    ["usercode", "password", "msgheader"].iter().all(|k| s.get(k).is_some_and(py_dogru))
}

pub fn anahtar_ozeti(anahtar: &str) -> String {
    Sha256::digest(anahtar.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Python'daki `ValueError` / `BaskaCihaz` / sqlite hataları.
#[derive(Debug)]
pub enum Hata {
    /// Geçersiz girdi (Python `ValueError`); mesaj kullanıcıya gösterilebilir.
    Deger(String),
    /// Numaranın başka (serbest bırakılmamış) cihazı var ya da bu cihaz başka müşteriye bağlı.
    BaskaCihaz,
    Db(String),
}

impl fmt::Display for Hata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hata::Deger(m) => f.write_str(m),
            Hata::BaskaCihaz => f.write_str("Numaranın başka (serbest bırakılmamış) cihazı var ya da bu cihaz başka müşteriye bağlı."),
            Hata::Db(m) => write!(f, "Veritabanı hatası: {m}"),
        }
    }
}

impl std::error::Error for Hata {}

impl From<rusqlite::Error> for Hata {
    fn from(e: rusqlite::Error) -> Hata {
        Hata::Db(e.to_string())
    }
}

pub type Sonuc<T> = Result<T, Hata>;

fn deger_hata<T>(m: impl Into<String>) -> Sonuc<T> {
    Err(Hata::Deger(m.into()))
}

// --- satır türleri (alan adları = sütun adları) ---
#[derive(Clone)]
pub struct Musteri {
    pub numara: i64,
    pub tuz: String,
    pub ozet: String,
    pub yineleme: i64,
    pub not_: String,
    pub uyelik: String,
    pub bitis: String,
    pub olusturma: String,
    pub son_giris: String,
    pub parola_acik: String,
    pub lisans_bitis: String,
    pub askida: i64,
    pub guncelleme_kanali: String,
    pub guncelleme_hemen: f64,
    pub guncelleme_hemen_surum: String,
    pub ap_parola: String,
}

/// Parolalar hata iletilerine/panik çıktısına sızmasın: yalnızca numara.
impl fmt::Debug for Musteri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Musteri({})", self.numara)
    }
}

#[derive(Clone)]
pub struct Cihaz {
    pub id: i64,
    pub musteri: i64,
    pub durum: String,
    pub tunel_ip: String,
    pub wg_pub: String,
    pub ssh_pub: String,
    pub anahtar_ozet: String,
    pub isletme_adi: String,
    pub unvan: String,
    pub surum: String,
    pub son_eslesme: String,
    pub baglanma: String,
    pub admin_parola: String,
    pub admin_tuz: String,
    pub admin_ozet: String,
    pub admin_yineleme: i64,
    pub ayrilma: String,
    pub isletme_turu: String,
    pub adres: String,
    pub otomatik: i64,
}

impl fmt::Debug for Cihaz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cihaz({}, musteri {}, {})", self.id, self.musteri, self.durum)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hareket {
    pub id: i64,
    pub zaman: String,
    pub kim: String,
    pub ip: String,
    pub olay: String,
    pub musteri: Option<i64>,
    pub ayrinti: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sorun {
    pub id: i64,
    pub musteri: i64,
    pub tur: String,
    pub metin: String,
    pub onem: String,
    pub acilis: String,
    pub kapanis: String,
    pub acilis_sms: String,
    pub kapanis_sms: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurumKaydi {
    pub surum: String,
    pub sha256: String,
    pub boyut: i64,
    pub kaynak: String,
    pub zaman: String,
}

#[derive(Clone, PartialEq)]
pub struct Bekleyen {
    pub kimlik: String,
    pub sir_ozet: String,
    pub ilk: f64,
    pub son: f64,
    pub ip: String,
    pub surum: String,
    pub wan_mac: String,
    pub wg_pub: String,
    pub ssh_pub: String,
    pub musteri: Option<i64>,
    pub ayrilma: f64,
    pub belirtec_ozet: String,
    pub belirtec_bitis: f64,
    pub cihaz: Option<i64>,
    pub baglandi: f64,
}

impl fmt::Debug for Bekleyen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bekleyen({})", self.kimlik)
    }
}

/// Python `dict(r, atama=a)`: `atama` "ayrildi" | "bagli".
#[derive(Clone, Debug)]
pub struct BekleyenAtama {
    pub kayit: Bekleyen,
    pub atama: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SmsSon {
    pub musteri: i64,
    pub zaman: String,
    pub saglayici: String,
    pub sonuc: String,
    pub kod: String,
}

/// `cihaz_hedefi` sonucu: cihazın kurması istenen paket.
#[derive(Clone, Debug, PartialEq)]
pub struct CihazHedefi {
    pub surum: String,
    pub sha256: String,
    pub boyut: i64,
    pub hemen: bool,
}

/// Bütün cihazlar için tek admin parolasının özeti (açık parola saklanmaz).
#[derive(Clone, Debug, PartialEq)]
pub struct AdminParola {
    pub tuz: String,
    pub ozet: String,
    pub yineleme: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaglikKaydi {
    pub zaman: String,
    pub rapor: Value,
}

fn musteri_satir(r: &Row) -> rusqlite::Result<Musteri> {
    Ok(Musteri {
        numara: r.get("numara")?,
        tuz: r.get("tuz")?,
        ozet: r.get("ozet")?,
        yineleme: r.get("yineleme")?,
        not_: r.get("not_")?,
        uyelik: r.get("uyelik")?,
        bitis: r.get("bitis")?,
        olusturma: r.get("olusturma")?,
        son_giris: r.get("son_giris")?,
        parola_acik: r.get("parola_acik")?,
        lisans_bitis: r.get("lisans_bitis")?,
        askida: r.get("askida")?,
        guncelleme_kanali: r.get("guncelleme_kanali")?,
        guncelleme_hemen: r.get("guncelleme_hemen")?,
        guncelleme_hemen_surum: r.get("guncelleme_hemen_surum")?,
        ap_parola: r.get("ap_parola")?,
    })
}

fn cihaz_satir(r: &Row) -> rusqlite::Result<Cihaz> {
    Ok(Cihaz {
        id: r.get("id")?,
        musteri: r.get("musteri")?,
        durum: r.get("durum")?,
        tunel_ip: r.get("tunel_ip")?,
        wg_pub: r.get("wg_pub")?,
        ssh_pub: r.get("ssh_pub")?,
        anahtar_ozet: r.get("anahtar_ozet")?,
        isletme_adi: r.get("isletme_adi")?,
        unvan: r.get("unvan")?,
        surum: r.get("surum")?,
        son_eslesme: r.get("son_eslesme")?,
        baglanma: r.get("baglanma")?,
        admin_parola: r.get("admin_parola")?,
        admin_tuz: r.get("admin_tuz")?,
        admin_ozet: r.get("admin_ozet")?,
        admin_yineleme: r.get("admin_yineleme")?,
        ayrilma: r.get("ayrilma")?,
        isletme_turu: r.get("isletme_turu")?,
        adres: r.get("adres")?,
        otomatik: r.get("otomatik")?,
    })
}

fn hareket_satir(r: &Row) -> rusqlite::Result<Hareket> {
    Ok(Hareket {
        id: r.get("id")?,
        zaman: r.get("zaman")?,
        kim: r.get("kim")?,
        ip: r.get("ip")?,
        olay: r.get("olay")?,
        musteri: r.get("musteri")?,
        ayrinti: r.get("ayrinti")?,
    })
}

fn sorun_satir(r: &Row) -> rusqlite::Result<Sorun> {
    Ok(Sorun {
        id: r.get("id")?,
        musteri: r.get("musteri")?,
        tur: r.get("tur")?,
        metin: r.get("metin")?,
        onem: r.get("onem")?,
        acilis: r.get("acilis")?,
        kapanis: r.get("kapanis")?,
        acilis_sms: r.get("acilis_sms")?,
        kapanis_sms: r.get("kapanis_sms")?,
    })
}

fn surum_satir(r: &Row) -> rusqlite::Result<SurumKaydi> {
    Ok(SurumKaydi { surum: r.get("surum")?, sha256: r.get("sha256")?, boyut: r.get("boyut")?, kaynak: r.get("kaynak")?, zaman: r.get("zaman")? })
}

fn bekleyen_satir(r: &Row) -> rusqlite::Result<Bekleyen> {
    Ok(Bekleyen {
        kimlik: r.get("kimlik")?,
        sir_ozet: r.get("sir_ozet")?,
        ilk: r.get("ilk")?,
        son: r.get("son")?,
        ip: r.get("ip")?,
        surum: r.get("surum")?,
        wan_mac: r.get("wan_mac")?,
        wg_pub: r.get("wg_pub")?,
        ssh_pub: r.get("ssh_pub")?,
        musteri: r.get("musteri")?,
        ayrilma: r.get("ayrilma")?,
        belirtec_ozet: r.get("belirtec_ozet")?,
        belirtec_bitis: r.get("belirtec_bitis")?,
        cihaz: r.get("cihaz")?,
        baglandi: r.get("baglandi")?,
    })
}

fn sms_son_satir(r: &Row) -> rusqlite::Result<SmsSon> {
    Ok(SmsSon { musteri: r.get("musteri")?, zaman: r.get("zaman")?, saglayici: r.get("saglayici")?, sonuc: r.get("sonuc")?, kod: r.get("kod")? })
}

fn bir<T>(c: &Connection, sql: &str, p: impl Params, f: fn(&Row) -> rusqlite::Result<T>) -> Sonuc<Option<T>> {
    Ok(c.query_row(sql, p, f).optional()?)
}

fn hepsi<T>(c: &Connection, sql: &str, p: impl Params, f: fn(&Row) -> rusqlite::Result<T>) -> Sonuc<Vec<T>> {
    let mut st = c.prepare(sql)?;
    let satirlar = st.query_map(p, f)?.collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(satirlar)
}

/// Ayar tablosundaki ham metin (JSON olmayan değerler de olabilir).
fn ayar_ham(c: &Connection, anahtar: &str) -> Sonuc<Option<String>> {
    bir(c, "SELECT deger FROM ayar WHERE anahtar = ?", [anahtar], |r| r.get(0))
}

fn ayar_json(c: &Connection, anahtar: &str) -> Sonuc<Option<Value>> {
    match ayar_ham(c, anahtar)? {
        None => Ok(None),
        Some(m) => serde_json::from_str(&m).map(Some).map_err(|_| Hata::Db(format!("ayar JSON'u okunamadı: {anahtar}"))),
    }
}

fn ayar_yaz(c: &Connection, anahtar: &str, deger: &str) -> Sonuc<()> {
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES (?, ?)", [anahtar, deger])?;
    Ok(())
}

/// Python `s[:n]` (karakter sayısıyla).
fn kes(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Python `json.dumps` biçimi (", " ve ": " ayraçları; `ascii` ise ASCII dışı `\uXXXX`).
struct PyBicim {
    ascii: bool,
}

impl serde_json::ser::Formatter for PyBicim {
    fn begin_array_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W, ilk: bool) -> std::io::Result<()> {
        if ilk {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_key<W: ?Sized + std::io::Write>(&mut self, w: &mut W, ilk: bool) -> std::io::Result<()> {
        if ilk {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b": ")
    }
    fn write_string_fragment<W: ?Sized + std::io::Write>(&mut self, w: &mut W, parca: &str) -> std::io::Result<()> {
        if !self.ascii || parca.is_ascii() {
            return w.write_all(parca.as_bytes());
        }
        for c in parca.chars() {
            if c.is_ascii() {
                w.write_all(&[c as u8])?;
            } else {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b).iter() {
                    w.write_all(format!("\\u{u:04x}").as_bytes())?;
                }
            }
        }
        Ok(())
    }
}

fn py_json(v: &Value, ascii: bool) -> String {
    let mut tampon = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut tampon, PyBicim { ascii });
    if v.serialize(&mut ser).is_err() {
        return "null".into();
    }
    String::from_utf8(tampon).unwrap_or_default()
}

/// Python `date.fromisoformat` (YYYY-AA-GG).
fn tarih_oku(s: &str) -> Sonuc<NaiveDate> {
    let b = s.as_bytes();
    let bicim = b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if bicim {
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(d);
        }
    }
    deger_hata(format!("Geçersiz tarih: {s}"))
}

fn yil_sonra(gun: NaiveDate) -> String {
    (gun + chrono::Duration::days(LISANS_GUN)).to_string()
}

/// Sürüm numarası sıralaması: (0, 10, 0) > (0, 9, 2).
fn surum_sirasi(s: &str) -> Sonuc<Vec<u64>> {
    s.split('.').map(|x| x.trim().parse::<u64>().or_else(|_| deger_hata(format!("Geçersiz sürüm: {s}")))).collect()
}

fn sema_hazirla(c: &Connection) -> Result<(), String> {
    c.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    c.pragma_update(None, "foreign_keys", "ON").map_err(|e| e.to_string())?;
    // değişen açık parolalar dosyanın boş sayfalarında kalmasın
    c.pragma_update(None, "secure_delete", "ON").map_err(|e| e.to_string())?;
    c.execute_batch(SEMA).map_err(|e| format!("Şema kurulamadı: {e}"))?;
    sema_denetle(c)
}

/// Gerekli her tablo ve sütun var mı; eksik olan adıyla bildirilir.
pub fn sema_denetle(c: &Connection) -> Result<(), String> {
    let mut eksik = Vec::new();
    for (tablo, sutunlar) in SUTUNLAR {
        let mut st = c.prepare(&format!("PRAGMA table_info({tablo})")).map_err(|e| e.to_string())?;
        let var = st
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<String>>>()
            .map_err(|e| e.to_string())?;
        if var.is_empty() {
            eksik.push(format!("tablo yok: {tablo}"));
            continue;
        }
        for s in sutunlar {
            if !var.iter().any(|v| v == s) {
                eksik.push(format!("sütun yok: {tablo}.{s}"));
            }
        }
    }
    if eksik.is_empty() {
        Ok(())
    } else {
        Err(format!("Veritabanı şeması eksik ({})", eksik.join("; ")))
    }
}

/// Şimdiki zaman (epoch saniye); testlerde sahte saat verilir.
pub type Saat = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct Veri {
    saat: Saat,
    db: Mutex<Connection>,
}

impl Veri {
    pub fn ac(yol: &Path, saat: Saat) -> Result<Veri, String> {
        let c = Connection::open(yol).map_err(|e| format!("Veritabanı açılamadı: {e}"))?;
        sema_hazirla(&c)?;
        Ok(Veri { saat, db: Mutex::new(c) })
    }

    /// Zehirlenmeye dayanıklı kilit: bir istekteki panik sonraki istekleri düşürmesin.
    pub(crate) fn kilit(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn bugun(&self) -> NaiveDate {
        let sn = (self.saat)().floor() as i64;
        ortak::TZ.timestamp_opt(sn, 0).single().map(|t| t.date_naive()).unwrap_or_default()
    }

    fn simdi(&self) -> String {
        ortak::now_iso((self.saat)())
    }

    /// Kilitli tek işlem (`BEGIN IMMEDIATE` … `COMMIT`); hata ya da panikte geri alınır. İçeride `self`'in kilitleyen
    /// yöntemleri çağrılmaz (kilit yeniden girilmez); `tx` doğrudan kullanılır.
    fn islem<T>(&self, f: impl FnOnce(&Transaction) -> Sonuc<T>) -> Sonuc<T> {
        let mut c = self.kilit();
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let s = f(&tx)?;
        tx.commit()?;
        Ok(s)
    }

    // --- müşteriler ---
    /// Parolayı yönetici yazar (2026-10-08, kullanıcı kararı); verilmezse (testler) rastgele.
    pub fn musteri_ekle(&self, not_: &str, parola: Option<&str>) -> Sonuc<(i64, String)> {
        if parola.is_some_and(|p| p.chars().count() < guvenlik::EN_AZ) {
            return deger_hata(format!("Parola en az {} karakter olmalı.", guvenlik::EN_AZ));
        }
        let pw = parola.map(String::from).unwrap_or_else(|| guvenlik::parola_uret(12));
        let (tuz, oz, y) = guvenlik::yeni_kayit(&pw);
        let not = kes(not_.trim(), 200);
        let olusturma = self.simdi();
        let lisans = yil_sonra(self.bugun());
        let numara = self.islem(|tx| {
            let numara = loop {
                let n = NUMARA_ARALIGI.0 + rastgele::randbelow((NUMARA_ARALIGI.1 - NUMARA_ARALIGI.0 + 1) as u64) as i64;
                if bir(tx, "SELECT 1 FROM musteri WHERE numara = ?", [n], |r| r.get::<_, i64>(0))?.is_none() {
                    break n;
                }
            };
            tx.execute(
                "INSERT INTO musteri (numara, tuz, ozet, yineleme, not_, olusturma, parola_acik, lisans_bitis, ap_parola) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![numara, tuz, oz, y, not, olusturma, pw, lisans, guvenlik::parola_uret(AP_PAROLA_UZUNLUK)],
            )?;
            Ok(numara)
        })?;
        Ok((numara, pw))
    }

    pub fn musteri(&self, numara: i64) -> Sonuc<Option<Musteri>> {
        bir(&self.kilit(), "SELECT * FROM musteri WHERE numara = ?", [numara], musteri_satir)
    }

    pub fn musteriler(&self) -> Sonuc<Vec<Musteri>> {
        hepsi(&self.kilit(), "SELECT * FROM musteri ORDER BY numara", [], musteri_satir)
    }

    pub fn parola_dogrula(&self, numara: i64, pw: &str) -> Sonuc<Option<Musteri>> {
        let Some(m) = self.musteri(numara)? else {
            guvenlik::bos_dogrulama(pw);
            return Ok(None);
        };
        let y = u32::try_from(m.yineleme).unwrap_or(0);
        Ok(guvenlik::dogru(pw, &m.tuz, &m.ozet, y).then_some(m))
    }

    pub fn parola_koy(&self, numara: i64, pw: &str) -> Sonuc<(String, String, u32)> {
        if pw.chars().count() < guvenlik::EN_AZ {
            return deger_hata(format!("Parola en az {} karakter olmalı.", guvenlik::EN_AZ));
        }
        let (tuz, oz, y) = guvenlik::yeni_kayit(pw);
        self.islem(|tx| {
            // parola yönetimde gösterilsin diye (kullanıcı kararı 2026-10-04) açık saklanır; veritabanı yalnızca wcpanel'in (600).
            tx.execute(
                "UPDATE musteri SET tuz = ?, ozet = ?, yineleme = ?, parola_acik = ? WHERE numara = ?",
                rusqlite::params![tuz, oz, y, pw, numara],
            )?;
            Ok(())
        })?;
        Ok((tuz, oz, y))
    }

    /// Yeni AP yönetim parolası (yönetimde gösterilir; cihaza sağlık cevabıyla ya da sonraki eşitlemede gider).
    pub fn ap_parolasi_yenile(&self, numara: i64) -> Sonuc<String> {
        let pw = guvenlik::parola_uret(AP_PAROLA_UZUNLUK);
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET ap_parola = ? WHERE numara = ?", rusqlite::params![pw, numara])?;
            Ok(())
        })?;
        Ok(pw)
    }

    pub fn giris_kaydet(&self, numara: i64) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET son_giris = ? WHERE numara = ?", rusqlite::params![simdi, numara])?;
            Ok(())
        })
    }

    pub fn uyelik(&self, numara: i64, durum: &str) -> Sonuc<()> {
        if durum != "aktif" && durum != "bitti" {
            return deger_hata(durum);
        }
        let bitis = if durum == "bitti" { self.simdi() } else { String::new() };
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET uyelik = ?, bitis = ? WHERE numara = ?", rusqlite::params![durum, bitis, numara])?;
            Ok(())
        })
    }

    // --- lisans (yıllık; ödeme ekranı sonradan lisans_uzat çağırır) ---
    /// (durum, bitiş): askida | bitti | aktif.
    pub fn lisans_durumu(&self, m: &Musteri) -> (&'static str, String) {
        let bitis = m.lisans_bitis.clone();
        if m.askida != 0 {
            return ("askida", bitis);
        }
        if !bitis.is_empty() && bitis < self.bugun().to_string() {
            return ("bitti", bitis);
        }
        ("aktif", bitis)
    }

    /// Bitişin üstüne (süresi geçmişse bugünden) `gun` gün ekler. Dönen: yeni bitiş.
    pub fn lisans_uzat(&self, numara: i64, gun: i64) -> Sonuc<String> {
        self.islem(|tx| {
            let mut taban = self.bugun();
            if let Some(b) = bir(tx, "SELECT lisans_bitis FROM musteri WHERE numara = ?", [numara], |r| r.get::<_, String>(0))? {
                if !b.is_empty() {
                    taban = taban.max(tarih_oku(&b)?);
                }
            }
            let yeni = (taban + chrono::Duration::days(gun)).to_string();
            tx.execute("UPDATE musteri SET lisans_bitis = ? WHERE numara = ?", rusqlite::params![yeni, numara])?;
            Ok(yeni)
        })
    }

    /// Dönen: kaydedilen tarih (YYYY-AA-GG). Geçersizse `Hata::Deger`.
    pub fn lisans_ayarla(&self, numara: i64, tarih: &str) -> Sonuc<String> {
        let tarih = tarih_oku(tarih)?.to_string();
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET lisans_bitis = ? WHERE numara = ?", rusqlite::params![tarih, numara])?;
            Ok(())
        })?;
        Ok(tarih)
    }

    pub fn askiya_al(&self, numara: i64, askida: bool) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET askida = ? WHERE numara = ?", rusqlite::params![askida as i64, numara])?;
            Ok(())
        })
    }

    // --- cihazlar ---
    pub fn bagli_cihaz(&self, numara: i64) -> Sonuc<Option<Cihaz>> {
        bir(&self.kilit(), "SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", [numara], cihaz_satir)
    }

    /// Müşterinin serbest bırakılmamış cihazı yoksa bu cihazı bağlar; aynı cihaz (aynı wg_pub) yeniden denerse yeni anahtar
    /// verir (eskisi geçersizleşir). Dönen: (cihaz_id, cihaz_anahtarı) — anahtar yalnızca burada açık görünür.
    pub fn cihaz_bagla(&self, numara: i64, wg_pub: &str, ssh_pub: &str, isletme_adi: &str, unvan: &str, surum: &str) -> Sonuc<(i64, String)> {
        let anahtar = rastgele::token_urlsafe(32);
        let oz = anahtar_ozeti(&anahtar);
        let simdi = self.simdi();
        let cid = self.islem(|tx| {
            let baska = bir(
                tx,
                "SELECT 1 FROM cihaz WHERE wg_pub = ? AND durum != 'serbest' AND musteri != ?",
                rusqlite::params![wg_pub, numara],
                |r| r.get::<_, i64>(0),
            )?;
            let c = bir(tx, "SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", [numara], cihaz_satir)?;
            if baska.is_some() || c.as_ref().is_some_and(|c| c.wg_pub != wg_pub || c.durum != "bagli") {
                return Err(Hata::BaskaCihaz);
            }
            if let Some(c) = c {
                tx.execute(
                    "UPDATE cihaz SET anahtar_ozet = ?, ssh_pub = ?, isletme_adi = ?, unvan = ?, surum = ? WHERE id = ?",
                    rusqlite::params![oz, ssh_pub, isletme_adi, unvan, surum, c.id],
                )?;
                return Ok(c.id);
            }
            tx.execute(
                "INSERT INTO cihaz (musteri, durum, wg_pub, ssh_pub, anahtar_ozet, isletme_adi, unvan, surum, baglanma) \
                 VALUES (?, 'bagli', ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![numara, wg_pub, ssh_pub, oz, isletme_adi, unvan, surum, simdi],
            )?;
            Ok(tx.last_insert_rowid())
        })?;
        Ok((cid, anahtar))
    }

    pub fn cihaz_otomatik_isaretle(&self, cid: i64) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute("UPDATE cihaz SET otomatik = 1 WHERE id = ?", [cid])?;
            Ok(())
        })
    }

    pub fn cihaz_anahtarla(&self, anahtar: &str) -> Sonuc<Option<Cihaz>> {
        if anahtar.is_empty() {
            return Ok(None);
        }
        bir(&self.kilit(), "SELECT * FROM cihaz WHERE anahtar_ozet = ? AND durum != 'serbest'", [anahtar_ozeti(anahtar)], cihaz_satir)
    }

    /// Yalnızca `CIHAZ_ALANLARI` güncellenir (alan adları sorguya yazıldığı için sıkı denetlenir).
    pub fn cihaz_guncelle(&self, cid: i64, alanlar: &[(&str, &str)]) -> Sonuc<()> {
        if alanlar.is_empty() || !alanlar.iter().all(|(k, _)| CIHAZ_ALANLARI.contains(k)) {
            let mut adlar: Vec<&str> = alanlar.iter().map(|(k, _)| *k).collect();
            adlar.sort_unstable();
            return deger_hata(format!("{adlar:?}"));
        }
        let kume = alanlar.iter().map(|(k, _)| format!("{k} = ?")).collect::<Vec<_>>().join(", ");
        let mut p: Vec<Sql> = alanlar.iter().map(|(_, v)| Sql::Text((*v).to_string())).collect();
        p.push(Sql::Integer(cid));
        self.islem(|tx| {
            tx.execute(&format!("UPDATE cihaz SET {kume} WHERE id = ?"), params_from_iter(p))?;
            Ok(())
        })
    }

    /// `isletme_turu` yalnızca izinli değerse yazılır; yoksa ya da bozuksa eskisi kalır. `adres` (veri sorumlusunun adresi)
    /// yalnızca cihaz gönderdiyse yazılır (eski sürümler göndermez).
    pub fn eslesme_kaydet(
        &self, cid: i64, isletme_adi: &str, unvan: &str, surum: &str, isletme_turu: Option<&str>, adres: Option<&str>,
    ) -> Sonuc<()> {
        let simdi = self.simdi();
        let mut alanlar = vec![("isletme_adi", isletme_adi), ("unvan", unvan), ("surum", surum), ("son_eslesme", simdi.as_str())];
        if let Some(t) = isletme_turu.filter(|t| ISLETME_TURLERI.iter().any(|(k, _)| k == t)) {
            alanlar.push(("isletme_turu", t));
        }
        if let Some(a) = adres {
            alanlar.push(("adres", a));
        }
        self.cihaz_guncelle(cid, &alanlar)
    }

    /// Dönen: etkilenen cihaz satırı (yoksa None). `zorla`: cihaz beklenmeden serbest (arızalı/kayıp cihaz).
    pub fn serbest_birak(&self, numara: i64, zorla: bool) -> Sonuc<Option<Cihaz>> {
        let simdi = self.simdi();
        self.islem(|tx| {
            let Some(c) = bir(tx, "SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", [numara], cihaz_satir)? else {
                return Ok(None);
            };
            if zorla {
                tx.execute("UPDATE cihaz SET durum = 'serbest', ayrilma = ? WHERE id = ?", rusqlite::params![simdi, c.id])?;
            } else {
                tx.execute("UPDATE cihaz SET durum = 'serbest_birakiliyor' WHERE id = ?", [c.id])?;
            }
            Ok(Some(c))
        })
    }

    /// Henüz tamamlanmamış serbest bırakmayı geri alır (cihaz temizlenmeden önce).
    pub fn serbest_iptal(&self, numara: i64) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute("UPDATE cihaz SET durum = 'bagli' WHERE musteri = ? AND durum = 'serbest_birakiliyor'", [numara])?;
            Ok(())
        })
    }

    pub fn temizlendi(&self, cid: i64) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute("UPDATE cihaz SET durum = 'serbest', ayrilma = ? WHERE id = ?", rusqlite::params![simdi, cid])?;
            Ok(())
        })
    }

    // --- kolay bağlama (spec 2026-10-16): bağlanmamış cihazların bekleme kayıtları ---
    /// Kaydın müşteriye ayrılması hâlâ geçerli mi: "bagli" (bu kayıttan bağlanmış, bağ aynı wg anahtarıyla duruyor — cihaz
    /// cevabı kaybettiyse yeniden bağlanır), "ayrildi" (AYRILMA_SN içinde, müşterinin bağlı cihazı yok, üyeliği aktif) ya da None.
    fn atama(c: &Connection, r: &Bekleyen, simdi: f64, wg_pub: Option<&str>) -> Sonuc<Option<&'static str>> {
        let Some(musteri) = r.musteri else {
            return Ok(None);
        };
        if let Some(cid) = r.cihaz {
            let ch = bir(c, "SELECT * FROM cihaz WHERE id = ?", [cid], cihaz_satir)?;
            let wg = wg_pub.filter(|s| !s.is_empty()).unwrap_or(&r.wg_pub);
            let ok = ch.is_some_and(|ch| ch.durum == "bagli" && ch.musteri == musteri && ch.wg_pub == wg);
            return Ok(ok.then_some("bagli"));
        }
        if simdi - r.ayrilma > AYRILMA_SN {
            return Ok(None);
        }
        let uyelik = bir(c, "SELECT uyelik FROM musteri WHERE numara = ?", [musteri], |r| r.get::<_, String>(0))?;
        if uyelik.as_deref() != Some("aktif") {
            return Ok(None);
        }
        if bir(c, "SELECT 1 FROM cihaz WHERE musteri = ? AND durum != 'serbest'", [musteri], |r| r.get::<_, i64>(0))?.is_some() {
            return Ok(None);
        }
        Ok(Some("ayrildi"))
    }

    /// Kimseye (geçerli olarak) ayrılmamış ve wg anahtarı bağlı bir cihazda olmayan kayıt.
    fn aday_mi(c: &Connection, r: &Bekleyen, simdi: f64) -> Sonuc<bool> {
        if Self::atama(c, r, simdi, None)?.is_some() {
            return Ok(false);
        }
        Ok(bir(c, "SELECT 1 FROM cihaz WHERE wg_pub = ? AND durum != 'serbest'", [&r.wg_pub], |r| r.get::<_, i64>(0))?.is_none())
    }

    /// Cihazın "bekliyorum" bildirimi. Dönen: (durum, yeni_mi, belirteç): durum "bekle" | "baglan" (belirteç açık, yalnızca
    /// burada) | "taninmadi" (sır tutmadı, kayıt değişmez) | "dolu" (tablo sınırı). İlk bildirim sırrın özetini kaydeder;
    /// sonrakiler aynı sırrı kanıtlamalıdır.
    pub fn bekleyen_bildir(
        &self, kimlik: &str, sir: &str, ip: &str, surum: &str, wan_mac: &str, wg_pub: &str, ssh_pub: &str,
    ) -> Sonuc<(&'static str, bool, Option<String>)> {
        let simdi = (self.saat)();
        let oz = anahtar_ozeti(sir);
        self.islem(|tx| {
            tx.execute("DELETE FROM bekleyen WHERE son < ?", [simdi - BEKLEYEN_SIL_SN])?;
            let Some(r) = bir(tx, "SELECT * FROM bekleyen WHERE kimlik = ?", [kimlik], bekleyen_satir)? else {
                let sayi: i64 = tx.query_row("SELECT COUNT(*) FROM bekleyen", [], |r| r.get(0))?;
                if sayi >= BEKLEYEN_EN_COK {
                    // tavan doluysa aday olmayan (BEKLEYEN_ADAY_SN'dir bildirmeyen), kimseye ayrılmamış en eski kayıt yer açar:
                    // bir kez doldurulan tablo yeni cihazları 7 gün kapatmasın
                    let sil = bir(
                        tx,
                        "SELECT kimlik FROM bekleyen WHERE musteri IS NULL AND cihaz IS NULL AND son < ? ORDER BY son, kimlik LIMIT 1",
                        [simdi - BEKLEYEN_ADAY_SN],
                        |r| r.get::<_, String>(0),
                    )?;
                    let Some(sil) = sil else {
                        return Ok(("dolu", false, None));
                    };
                    tx.execute("DELETE FROM bekleyen WHERE kimlik = ?", [sil])?;
                }
                tx.execute(
                    "INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, surum, wan_mac, wg_pub, ssh_pub) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    rusqlite::params![kimlik, oz, simdi, simdi, ip, surum, wan_mac, wg_pub, ssh_pub],
                )?;
                return Ok(("bekle", true, None));
            };
            if !guvenlik::sabit_esit(r.sir_ozet.as_bytes(), oz.as_bytes()) {
                return Ok(("taninmadi", false, None));
            }
            tx.execute(
                "UPDATE bekleyen SET son = ?, ip = ?, surum = ?, wan_mac = ?, wg_pub = ?, ssh_pub = ? WHERE kimlik = ?",
                rusqlite::params![simdi, ip, surum, wan_mac, wg_pub, ssh_pub, kimlik],
            )?;
            if Self::atama(tx, &r, simdi, Some(wg_pub))?.is_none() {
                if r.musteri.is_some() {
                    // süresi dolmuş / bozulmuş ayrılma düşer
                    tx.execute(
                        "UPDATE bekleyen SET musteri = NULL, ayrilma = 0, cihaz = NULL, baglandi = 0, belirtec_ozet = '', \
                         belirtec_bitis = 0 WHERE kimlik = ?",
                        [kimlik],
                    )?;
                }
                return Ok(("bekle", false, None));
            }
            let t = rastgele::token_urlsafe(32);
            tx.execute(
                "UPDATE bekleyen SET belirtec_ozet = ?, belirtec_bitis = ? WHERE kimlik = ?",
                rusqlite::params![anahtar_ozeti(&t), simdi + BELIRTEC_SN, kimlik],
            )?;
            Ok(("baglan", false, Some(t)))
        })
    }

    pub fn bekleyen_var(&self, kimlik: &str) -> Sonuc<bool> {
        Ok(bir(&self.kilit(), "SELECT 1 FROM bekleyen WHERE kimlik = ?", [kimlik], |r| r.get::<_, i64>(0))?.is_some())
    }

    /// Müşterinin tarayıcısıyla aynı internet adresinden son BEKLEYEN_ADAY_SN içinde bildirmiş, ayrılabilir kayıtlar.
    pub fn bekleyen_adaylar(&self, ip: &str) -> Sonuc<Vec<Bekleyen>> {
        let simdi = (self.saat)();
        let c = self.kilit();
        let rows = hepsi(
            &c,
            "SELECT * FROM bekleyen WHERE ip = ? AND son >= ? ORDER BY ilk, kimlik",
            rusqlite::params![ip, simdi - BEKLEYEN_ADAY_SN],
            bekleyen_satir,
        )?;
        let mut out = Vec::new();
        for r in rows {
            if Self::aday_mi(&c, &r, simdi)? {
                out.push(r);
            }
        }
        Ok(out)
    }

    /// Kaydı müşteriye ayırır (hâlâ aynı IP'den aday ve müşterinin bağlı cihazı yoksa). Müşterinin bağlanmamış başka
    /// ayrılmaları düşer. Dönen: başarılı mı.
    pub fn bekleyen_ayir(&self, kimlik: &str, numara: i64, ip: &str) -> Sonuc<bool> {
        let simdi = (self.saat)();
        self.islem(|tx| {
            let r = bir(
                tx,
                "SELECT * FROM bekleyen WHERE kimlik = ? AND ip = ? AND son >= ?",
                rusqlite::params![kimlik, ip, simdi - BEKLEYEN_ADAY_SN],
                bekleyen_satir,
            )?;
            let Some(r) = r else {
                return Ok(false);
            };
            if !Self::aday_mi(tx, &r, simdi)? {
                return Ok(false);
            }
            let uyelik = bir(tx, "SELECT uyelik FROM musteri WHERE numara = ?", [numara], |r| r.get::<_, String>(0))?;
            if uyelik.as_deref() != Some("aktif") {
                return Ok(false);
            }
            if bir(tx, "SELECT 1 FROM cihaz WHERE musteri = ? AND durum != 'serbest'", [numara], |r| r.get::<_, i64>(0))?.is_some() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE bekleyen SET musteri = NULL, ayrilma = 0, belirtec_ozet = '', belirtec_bitis = 0 WHERE musteri = ? AND cihaz IS NULL",
                [numara],
            )?;
            tx.execute(
                "UPDATE bekleyen SET musteri = ?, ayrilma = ?, cihaz = NULL, baglandi = 0, belirtec_ozet = '', belirtec_bitis = 0 WHERE kimlik = ?",
                rusqlite::params![numara, simdi, kimlik],
            )?;
            Ok(true)
        })
    }

    /// Müşteriye geçerli olarak ayrılmış (ya da ondan bağlanmış) kayıt ya da None.
    pub fn bekleyen_musterinin(&self, numara: i64) -> Sonuc<Option<BekleyenAtama>> {
        let simdi = (self.saat)();
        let c = self.kilit();
        for r in hepsi(&c, "SELECT * FROM bekleyen WHERE musteri = ? ORDER BY ayrilma DESC", [numara], bekleyen_satir)? {
            if let Some(atama) = Self::atama(&c, &r, simdi, None)? {
                return Ok(Some(BekleyenAtama { kayit: r, atama }));
            }
        }
        Ok(None)
    }

    /// Bağlama belirteci: sır ve belirteç tutuyor, süresi geçmemiş, kayıt bir müşteriye ayrılmışsa kaydı döner; belirteç her
    /// durumda tüketilir (tek kullanımlık). Değilse None.
    pub fn bekleyen_belirtec_tuket(&self, kimlik: &str, sir: &str, belirtec: &str) -> Sonuc<Option<Bekleyen>> {
        let simdi = (self.saat)();
        self.islem(|tx| {
            let Some(r) = bir(tx, "SELECT * FROM bekleyen WHERE kimlik = ?", [kimlik], bekleyen_satir)? else {
                return Ok(None);
            };
            if !guvenlik::sabit_esit(r.sir_ozet.as_bytes(), anahtar_ozeti(sir).as_bytes()) || r.belirtec_ozet.is_empty() {
                return Ok(None);
            }
            tx.execute("UPDATE bekleyen SET belirtec_ozet = '', belirtec_bitis = 0 WHERE kimlik = ?", [kimlik])?;
            if !guvenlik::sabit_esit(r.belirtec_ozet.as_bytes(), anahtar_ozeti(belirtec).as_bytes()) || simdi > r.belirtec_bitis {
                return Ok(None);
            }
            Ok(r.musteri.is_some().then_some(r))
        })
    }

    pub fn bekleyen_baglandi(&self, kimlik: &str, cid: i64) -> Sonuc<()> {
        let simdi = (self.saat)();
        self.islem(|tx| {
            tx.execute("UPDATE bekleyen SET cihaz = ?, baglandi = ? WHERE kimlik = ?", rusqlite::params![cid, simdi, kimlik])?;
            Ok(())
        })
    }

    /// Yönetim listesi (yeniden eskiye); BEKLEYEN_SIL_SN'den eski kayıtlar önce silinir.
    pub fn bekleyenler(&self) -> Sonuc<Vec<Bekleyen>> {
        let simdi = (self.saat)();
        self.islem(|tx| {
            tx.execute("DELETE FROM bekleyen WHERE son < ?", [simdi - BEKLEYEN_SIL_SN])?;
            hepsi(tx, "SELECT * FROM bekleyen ORDER BY son DESC, kimlik", [], bekleyen_satir)
        })
    }

    // --- sürüm deposu ve güncelleme kanalları (uzaktan güncelleme) ---
    pub fn surum_ekle(&self, surum: &str, sha256: &str, boyut: i64, kaynak: &str) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO surum (surum, sha256, boyut, kaynak, zaman) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![surum, sha256, boyut, kaynak, simdi],
            )?;
            Ok(())
        })
    }

    /// Yeniden eskiye, sürüm numarasına göre (0.10.0 > 0.9.2).
    pub fn surumler(&self) -> Sonuc<Vec<SurumKaydi>> {
        let satirlar = hepsi(&self.kilit(), "SELECT * FROM surum", [], surum_satir)?;
        let mut anahtarli = satirlar.into_iter().map(|r| surum_sirasi(&r.surum).map(|k| (k, r))).collect::<Sonuc<Vec<_>>>()?;
        anahtarli.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(anahtarli.into_iter().map(|(_, r)| r).collect())
    }

    pub fn surum(&self, surum: &str) -> Sonuc<Option<SurumKaydi>> {
        bir(&self.kilit(), "SELECT * FROM surum WHERE surum = ?", [surum], surum_satir)
    }

    /// Kayıtlı seçim: sürüm, `EN_SON` ya da None (hedef yok). Ham metin: JSON olmayabilir.
    pub fn kanal_hedef(&self, kanal: &str) -> Sonuc<Option<String>> {
        ayar_ham(&self.kilit(), &format!("kanal_{kanal}"))
    }

    /// GitHub'dan alınmış en yüksek sürüm ya da None.
    pub fn en_son_surum(&self) -> Sonuc<Option<String>> {
        Ok(self.surumler()?.into_iter().find(|r| r.kaynak == "github").map(|r| r.surum))
    }

    /// Kanalın şu anki hedef sürümü (`EN_SON` çözülmüş) ya da None.
    pub fn kanal_surumu(&self, kanal: &str) -> Sonuc<Option<String>> {
        let hedef = self.kanal_hedef(kanal)?;
        if hedef.as_deref() == Some(EN_SON) {
            return self.en_son_surum();
        }
        Ok(hedef)
    }

    pub fn kanal_hedef_koy(&self, kanal: &str, surum: Option<&str>) -> Sonuc<()> {
        if !GUNCELLEME_KANALLARI.contains(&kanal) || kanal == "kapali" {
            return deger_hata(kanal);
        }
        let anahtar = format!("kanal_{kanal}");
        self.islem(|tx| {
            match surum {
                None => tx.execute("DELETE FROM ayar WHERE anahtar = ?", [anahtar.as_str()])?,
                Some(s) => tx.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES (?, ?)", [anahtar.as_str(), s])?,
            };
            Ok(())
        })
    }

    pub fn guncelleme_kanali_koy(&self, numara: i64, kanal: &str) -> Sonuc<()> {
        if !GUNCELLEME_KANALLARI.contains(&kanal) {
            return deger_hata(kanal);
        }
        self.islem(|tx| {
            tx.execute("UPDATE musteri SET guncelleme_kanali = ? WHERE numara = ?", rusqlite::params![kanal, numara])?;
            Ok(())
        })
    }

    /// "Şimdi güncelle" bayrağı, basıldığı andaki hedef sürümle; epoch 0 = bayrak yok.
    pub fn guncelleme_hemen_koy(&self, numara: i64, epoch: f64, surum: &str) -> Sonuc<()> {
        let s = if epoch != 0.0 { surum } else { "" };
        self.islem(|tx| {
            tx.execute(
                "UPDATE musteri SET guncelleme_hemen = ?, guncelleme_hemen_surum = ? WHERE numara = ?",
                rusqlite::params![epoch, s, numara],
            )?;
            Ok(())
        })
    }

    /// Üyelik aktif, askıda değil, lisans süresi dolmamış.
    pub fn hizmet_acik(&self, m: &Musteri) -> bool {
        m.uyelik == "aktif" && self.lisans_durumu(m).0 == "aktif"
    }

    /// Cihazın kurması istenen paket ya da None (kanal kapalı/hedefsiz, lisans ya da üyelik kapalı).
    pub fn cihaz_hedefi(&self, m: &Musteri) -> Sonuc<Option<CihazHedefi>> {
        if m.guncelleme_kanali == "kapali" || !self.hizmet_acik(m) {
            return Ok(None);
        }
        let Some(hedef) = self.kanal_surumu(&m.guncelleme_kanali)? else {
            return Ok(None);
        };
        let Some(k) = self.surum(&hedef)? else {
            return Ok(None);
        };
        let hemen = m.guncelleme_hemen > 0.0 && m.guncelleme_hemen_surum == k.surum; // bayrak başka hedef içinse geçmez
        Ok(Some(CihazHedefi { surum: k.surum, sha256: k.sha256, boyut: k.boyut, hemen }))
    }

    // --- bütün cihazlarda tek admin parolası (2026-10-08, kullanıcı kararı): yalnızca özet saklanır ---
    pub fn admin_parolasi(&self) -> Sonuc<Option<AdminParola>> {
        let Some(j) = ayar_json(&self.kilit(), "admin")? else {
            return Ok(None);
        };
        let al = |k: &str| j.get(k).and_then(Value::as_str).map(String::from);
        match (al("tuz"), al("ozet"), j.get("yineleme").and_then(Value::as_u64)) {
            (Some(tuz), Some(ozet), Some(y)) => Ok(Some(AdminParola { tuz, ozet, yineleme: y as u32 })),
            _ => Err(Hata::Db("admin ayarı bozuk".into())),
        }
    }

    pub fn admin_parolasi_koy(&self, pw: &str) -> Sonuc<()> {
        if pw.chars().count() < guvenlik::EN_AZ {
            return deger_hata(format!("Parola en az {} karakter olmalı.", guvenlik::EN_AZ));
        }
        let (tuz, oz, y) = guvenlik::yeni_kayit(pw);
        let d = py_json(&serde_json::json!({"tuz": tuz, "ozet": oz, "yineleme": y}), true);
        self.islem(|tx| ayar_yaz(tx, "admin", &d))
    }

    // --- SMS ayarı: NetGSM (eski cihazlara da gider) + doğrulama SMS'i sağlayıcı seçimi ve Twilio (yalnızca merkezde) ---
    pub fn sms_ayari(&self) -> Sonuc<Option<Value>> {
        let Some(mut a) = ayar_json(&self.kilit(), "sms")? else {
            return Ok(None);
        };
        let Some(m) = a.as_object_mut() else {
            return Err(Hata::Db("sms ayarı bozuk".into()));
        };
        if !m.contains_key("tr_saglayici") {
            m.insert("tr_saglayici".into(), Value::String("netgsm".into()));
        }
        let mut tw = match twilio_varsayilan() {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        if let Some(Value::Object(kayitli)) = m.get("twilio").filter(|v| py_dogru(v)) {
            for (k, v) in kayitli {
                tw.insert(k.clone(), v.clone());
            }
        }
        m.insert("twilio".into(), Value::Object(tw));
        Ok(Some(a))
    }

    pub fn sms_ayari_koy(&self, d: &Value) -> Sonuc<()> {
        let mut a = Map::new();
        a.insert("mock".into(), Value::Bool(d.get("mock").is_some_and(py_dogru)));
        for k in SMS_ALANLARI {
            a.insert(k.into(), Value::String(py_veya_bos(d.get(k)).trim().to_string()));
        }
        let sag = d.get("tr_saglayici").filter(|v| py_dogru(v)).map(py_str).unwrap_or_else(|| "netgsm".into());
        if !TR_SAGLAYICILAR.contains(&sag.as_str()) {
            return deger_hata("Türk numaraları için sağlayıcı netgsm ya da twilio olmalı.");
        }
        a.insert("tr_saglayici".into(), Value::String(sag.clone()));
        let tw = d.get("twilio").cloned().unwrap_or(Value::Null);
        let mut t = Map::new();
        t.insert("acik".into(), Value::Bool(tw.get("acik").is_some_and(py_dogru)));
        for k in TWILIO_ALANLARI {
            t.insert(k.into(), Value::String(py_veya_bos(tw.get(k)).trim().to_string()));
        }
        for (k, _) in twilio::SID_ALANLARI {
            let v = t[k].as_str().unwrap_or("");
            if !v.is_empty() && !twilio::sid_gecerli(k, v) {
                return deger_hata(format!("Twilio {k} biçimi geçersiz."));
            }
        }
        let acik = t["acik"] == Value::Bool(true);
        if sag == "twilio" && !acik {
            return deger_hata("Türk numaraları için Twilio seçildiyse Twilio açık olmalı.");
        }
        let tv = Value::Object(t.clone());
        let dolu = |k: &str| !t[k].as_str().unwrap_or("").is_empty();
        if acik && !(dolu("account_sid") && dolu("verify_sid") && twilio::kimlik(&tv).is_some()) {
            return deger_hata("Twilio açıkken account_sid, verify_sid ve auth_token (ya da API anahtarı sid + secret) gerekli.");
        }
        let eksik: Vec<&str> = ["usercode", "password", "msgheader"].into_iter().filter(|k| a[*k].as_str().unwrap_or("").is_empty()).collect();
        if a["mock"] != Value::Bool(true) && sag == "netgsm" && !eksik.is_empty() {
            return deger_hata(format!("Deneme modu kapalıyken şu alanlar gerekli: {}", eksik.join(", ")));
        }
        a.insert("twilio".into(), tv);
        // NetGSM şifresi ve Twilio sırları açık saklanır (sağlayıcıya gönderilmesi gerekir); veritabanı yalnızca wcpanel'in (600).
        let metin = py_json(&Value::Object(a), false);
        self.islem(|tx| ayar_yaz(tx, "sms", &metin))
    }

    /// Cihaza bildirilen etkin deneme modu: ayar yok, deneme modu açık ya da seçili sağlayıcının ayarı eksik.
    pub fn sms_etkin_mock(&self) -> Sonuc<bool> {
        Ok(match self.sms_ayari()? {
            None => true,
            Some(s) => s.get("mock").is_some_and(py_dogru) || !sms_hazir(&s),
        })
    }

    // --- müşteri (cihaz) başına günlük doğrulama SMS'i sayacı (kod saklanmaz). Yabancı numaralar ayrıca `<gün>/yabanci`
    // anahtarında sayılır (daha düşük sınır) ---
    pub fn sms_sayac(&self, musteri: i64, gun: &str) -> Sonuc<i64> {
        let n = bir(&self.kilit(), "SELECT sayi FROM sms_sayac WHERE musteri = ? AND gun = ?", rusqlite::params![musteri, gun], |r| r.get::<_, i64>(0))?;
        Ok(n.unwrap_or(0))
    }

    /// Bir artırır, yeni değeri döner.
    pub fn sms_sayac_artir(&self, musteri: i64, gun: &str) -> Sonuc<i64> {
        self.islem(|tx| {
            tx.execute(
                "INSERT INTO sms_sayac (musteri, gun, sayi) VALUES (?, ?, 1) ON CONFLICT(musteri, gun) DO UPDATE SET sayi = sayi + 1",
                rusqlite::params![musteri, gun],
            )?;
            Ok(tx.query_row("SELECT sayi FROM sms_sayac WHERE musteri = ? AND gun = ?", rusqlite::params![musteri, gun], |r| r.get(0))?)
        })
    }

    // --- müşterinin son doğrulama SMS'i sonucu (hareket satırı yerine tek satır; kod ve numara yok) ---
    pub fn sms_son_kaydet(&self, musteri: i64, saglayici: &str, sonuc: &str, kod: &str) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute(
                "INSERT OR REPLACE INTO sms_son (musteri, zaman, saglayici, sonuc, kod) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![musteri, simdi, saglayici, sonuc, kes(kod, 16)],
            )?;
            Ok(())
        })
    }

    pub fn sms_son(&self, musteri: i64) -> Sonuc<Option<SmsSon>> {
        bir(&self.kilit(), "SELECT * FROM sms_son WHERE musteri = ?", [musteri], sms_son_satir)
    }

    // --- sağlık raporu, sorun kaydı, uyarı ayarı ---
    /// Cihaz başına tek satır; her rapor öncekinin üzerine yazar.
    pub fn saglik_kaydet(&self, cid: i64, rapor: &Value) -> Sonuc<()> {
        let simdi = self.simdi();
        let r = py_json(rapor, false);
        self.islem(|tx| {
            tx.execute("INSERT OR REPLACE INTO saglik (cihaz, zaman, rapor) VALUES (?, ?, ?)", rusqlite::params![cid, simdi, r])?;
            Ok(())
        })
    }

    pub fn saglik(&self, cid: i64) -> Sonuc<Option<SaglikKaydi>> {
        let Some((zaman, rapor)) = bir(&self.kilit(), "SELECT zaman, rapor FROM saglik WHERE cihaz = ?", [cid], |r| {
            Ok((r.get::<_, String>("zaman")?, r.get::<_, String>("rapor")?))
        })?
        else {
            return Ok(None);
        };
        let rapor = serde_json::from_str(&rapor).map_err(|_| Hata::Db(format!("sağlık raporu okunamadı: cihaz {cid}")))?;
        Ok(Some(SaglikKaydi { zaman, rapor }))
    }

    /// Aynı müşteri+tür için açık sorun varsa onun id'si döner, yenisi açılmaz.
    pub fn sorun_ac(&self, musteri: i64, tur: &str, metin: &str, onem: &str) -> Sonuc<i64> {
        let simdi = self.simdi();
        self.islem(|tx| {
            let var = bir(tx, "SELECT id FROM sorun WHERE musteri = ? AND tur = ? AND kapanis = ''", rusqlite::params![musteri, tur], |r| r.get::<_, i64>(0))?;
            if let Some(id) = var {
                return Ok(id);
            }
            tx.execute(
                "INSERT INTO sorun (musteri, tur, metin, onem, acilis) VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![musteri, tur, metin, onem, simdi],
            )?;
            Ok(tx.last_insert_rowid())
        })
    }

    pub fn sorun_kapat(&self, sorun_id: i64) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute("UPDATE sorun SET kapanis = ? WHERE id = ? AND kapanis = ''", rusqlite::params![simdi, sorun_id])?;
            Ok(())
        })
    }

    /// `alan`: acilis_sms | kapanis_sms; değer: gonderildi|mock|hata:<kod>|sinir:<gün>|yok (kapanış SMS'i gerekmedi).
    pub fn sorun_sms(&self, sorun_id: i64, alan: &str, deger: &str) -> Sonuc<()> {
        if !SORUN_SMS_ALANLARI.contains(&alan) {
            return deger_hata("Geçersiz SMS alanı.");
        }
        self.islem(|tx| {
            tx.execute(&format!("UPDATE sorun SET {alan} = ? WHERE id = ?"), rusqlite::params![kes(deger, 64), sorun_id])?;
            Ok(())
        })
    }

    /// Açık sorunun metni değişti (ör. disk yüzdesi): güncellenir, açılış zamanı ve SMS durumu aynı kalır.
    pub fn sorun_metin(&self, sorun_id: i64, metin: &str) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute("UPDATE sorun SET metin = ? WHERE id = ?", rusqlite::params![kes(metin, 200), sorun_id])?;
            Ok(())
        })
    }

    /// Kapanıp TEKRAR_SN temiz kalmadan dönen sorun aynı olayın devamıdır: kapanış silinir.
    pub fn sorun_yeniden_ac(&self, sorun_id: i64) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute("UPDATE sorun SET kapanis = '' WHERE id = ?", [sorun_id])?;
            Ok(())
        })
    }

    /// Açılış SMS'i gitmiş, kapanmış, kapanış SMS'i henüz karara bağlanmamış (kritik/uyarı) sorunlar.
    pub fn kapanis_bekleyenler(&self, musteri: i64) -> Sonuc<Vec<Sorun>> {
        hepsi(
            &self.kilit(),
            "SELECT * FROM sorun WHERE musteri = ? AND kapanis != '' AND kapanis_sms = '' AND onem != 'bilgi' \
             AND acilis_sms IN ('gonderildi', 'mock') ORDER BY id",
            [musteri],
            sorun_satir,
        )
    }

    /// Günde bir kez yapılacak iş: o gün ilk çağrıda true döner (bayrak konur), sonra false.
    pub fn gunluk_bayrak(&self, ad: &str, gun: &str) -> Sonuc<bool> {
        let anahtar = format!("{ad}:{gun}");
        self.islem(|tx| {
            if bir(tx, "SELECT 1 FROM ayar WHERE anahtar = ?", [anahtar.as_str()], |r| r.get::<_, i64>(0))?.is_some() {
                return Ok(false);
            }
            tx.execute("INSERT INTO ayar (anahtar, deger) VALUES (?, '1')", [anahtar.as_str()])?;
            Ok(true)
        })
    }

    pub fn acik_sorunlar(&self, musteri: Option<i64>) -> Sonuc<Vec<Sorun>> {
        let c = self.kilit();
        match musteri {
            None => hepsi(&c, "SELECT * FROM sorun WHERE kapanis = '' ORDER BY id", [], sorun_satir),
            Some(m) => hepsi(&c, "SELECT * FROM sorun WHERE kapanis = '' AND musteri = ? ORDER BY id", [m], sorun_satir),
        }
    }

    pub fn sorun_gecmisi(&self, musteri: i64, limit: i64) -> Sonuc<Vec<Sorun>> {
        hepsi(&self.kilit(), "SELECT * FROM sorun WHERE musteri = ? ORDER BY id DESC LIMIT ?", rusqlite::params![musteri, limit], sorun_satir)
    }

    /// [bas, bit) aralığıyla kesişen sorunlar (raporlar): açılışı bitişten önce, kapanışı yok ya da başlangıçtan sonra.
    /// Zamanlar hep +03:00 ISO: metin karşılaştırması sıralamayla aynı.
    pub fn sorunlar_aralik(&self, musteri: i64, bas_iso: &str, bit_iso: &str) -> Sonuc<Vec<Sorun>> {
        hepsi(
            &self.kilit(),
            "SELECT * FROM sorun WHERE musteri = ? AND acilis < ? AND (kapanis = '' OR kapanis >= ?) ORDER BY acilis, id",
            rusqlite::params![musteri, bit_iso, bas_iso],
            sorun_satir,
        )
    }

    pub fn son_kapanan(&self, musteri: i64, tur: &str) -> Sonuc<Option<Sorun>> {
        bir(
            &self.kilit(),
            "SELECT * FROM sorun WHERE musteri = ? AND tur = ? AND kapanis != '' ORDER BY id DESC LIMIT 1",
            rusqlite::params![musteri, tur],
            sorun_satir,
        )
    }

    /// `{**UYARI_VARSAYILAN, **kayıtlı}`.
    pub fn uyari_ayari(&self) -> Sonuc<Value> {
        let mut m = match uyari_varsayilan() {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        if let Some(Value::Object(k)) = ayar_json(&self.kilit(), "uyari")? {
            for (a, d) in k {
                m.insert(a, d);
            }
        }
        Ok(Value::Object(m))
    }

    pub fn uyari_ayari_koy(&self, d: &Value) -> Sonuc<()> {
        let mut tel: Vec<String> = Vec::new();
        if let Some(Value::Array(liste)) = d.get("telefonlar") {
            for ham in liste {
                let ham_s = py_str(ham);
                let Some(n) = ortak::normalize_phone(&ham_s) else {
                    return deger_hata(format!("Geçersiz telefon: {ham_s}"));
                };
                let n = format!("90{n}");
                if !tel.contains(&n) {
                    tel.push(n);
                }
            }
        }
        if tel.len() > UYARI_TELEFON_EN_COK {
            return deger_hata(format!("En çok {UYARI_TELEFON_EN_COK} telefon girilebilir."));
        }
        let tamsayi_hatasi = || Hata::Deger("Günlük SMS sınırı tamsayı olmalı.".into());
        let sinir = match d.get("gunluk_sinir") {
            None => 20,
            Some(Value::Number(n)) => n.as_i64().ok_or_else(tamsayi_hatasi)?,
            Some(Value::String(s)) => s.trim().parse::<i64>().map_err(|_| tamsayi_hatasi())?,
            Some(_) => return Err(tamsayi_hatasi()),
        };
        if !(UYARI_SINIR_ARALIK.0..=UYARI_SINIR_ARALIK.1).contains(&sinir) {
            // son hak üst sınır bildiriminin: 1 olamaz
            return deger_hata(format!("Günlük SMS sınırı {} ile {} arasında olmalı.", UYARI_SINIR_ARALIK.0, UYARI_SINIR_ARALIK.1));
        }
        let a = serde_json::json!({"telefonlar": tel, "acik": d.get("acik").is_some_and(py_dogru), "gunluk_sinir": sinir});
        let metin = py_json(&a, true);
        self.islem(|tx| ayar_yaz(tx, "uyari", &metin))
    }

    /// wc-saglik'in son tamamlanan turunun zamanı (ISO) ya da None (hiç çalışmadı).
    pub fn saglik_son_tur(&self) -> Sonuc<Option<String>> {
        ayar_ham(&self.kilit(), "saglik_son_tur")
    }

    pub fn saglik_son_tur_koy(&self, zaman: &str) -> Sonuc<()> {
        self.islem(|tx| ayar_yaz(tx, "saglik_son_tur", zaman))
    }

    pub fn sms_sayaci(&self, gun: &str) -> Sonuc<i64> {
        match ayar_ham(&self.kilit(), &format!("sms_sayac:{gun}"))? {
            None => Ok(0),
            Some(m) => m.trim().parse::<i64>().map_err(|_| Hata::Db(format!("sms_sayac:{gun} sayı değil"))),
        }
    }

    /// Günlük uyarı SMS sayacını bir artırır, yeni değeri döner.
    pub fn sms_say(&self, gun: &str) -> Sonuc<i64> {
        let anahtar = format!("sms_sayac:{gun}");
        self.islem(|tx| {
            let n = match ayar_ham(tx, &anahtar)? {
                None => 0,
                Some(m) => m.trim().parse::<i64>().map_err(|_| Hata::Db(format!("{anahtar} sayı değil")))?,
            } + 1;
            ayar_yaz(tx, &anahtar, &n.to_string())?;
            Ok(n)
        })
    }

    /// Serbest bırakılmamış cihazlar (Python'da müşteri numarasıyla sözlük; burada cihaz sırasıyla liste).
    pub fn cihazlar(&self) -> Sonuc<Vec<Cihaz>> {
        hepsi(&self.kilit(), "SELECT * FROM cihaz WHERE durum != 'serbest'", [], cihaz_satir)
    }

    pub fn cihaz_gecmisi(&self, numara: i64) -> Sonuc<Vec<Cihaz>> {
        hepsi(&self.kilit(), "SELECT * FROM cihaz WHERE musteri = ? ORDER BY id DESC", [numara], cihaz_satir)
    }

    // --- hareketler ---
    pub fn hareket(&self, kim: &str, ip: &str, olay: &str, musteri: Option<i64>, ayrinti: &str) -> Sonuc<()> {
        let simdi = self.simdi();
        self.islem(|tx| {
            tx.execute(
                "INSERT INTO hareket (zaman, kim, ip, olay, musteri, ayrinti) VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![simdi, kes(kim, 64), kes(ip, 64), olay, musteri, kes(ayrinti, 500)],
            )?;
            Ok(())
        })
    }

    pub fn hareketler(&self, musteri: Option<i64>, limit: i64, olay: Option<&str>) -> Sonuc<Vec<Hareket>> {
        let (mut kosul, mut p) = (Vec::new(), Vec::new());
        if let Some(m) = musteri {
            kosul.push("musteri = ?");
            p.push(Sql::Integer(m));
        }
        if let Some(o) = olay {
            kosul.push("olay = ?");
            p.push(Sql::Text(o.to_string()));
        }
        p.push(Sql::Integer(limit));
        let nerede = if kosul.is_empty() { String::new() } else { format!(" WHERE {}", kosul.join(" AND ")) };
        hepsi(&self.kilit(), &format!("SELECT * FROM hareket{nerede} ORDER BY id DESC LIMIT ?"), params_from_iter(p), hareket_satir)
    }

    // --- eski arşivler ---
    pub fn eski_arsiv_bagla(&self, ad: &str, numara: Option<i64>) -> Sonuc<()> {
        self.islem(|tx| {
            tx.execute(
                "INSERT INTO eski_arsiv (ad, musteri) VALUES (?, ?) ON CONFLICT(ad) DO UPDATE SET musteri = excluded.musteri",
                rusqlite::params![ad, numara],
            )?;
            Ok(())
        })
    }

    /// (ad, bağlı müşteri) çiftleri, tablo sırasıyla.
    pub fn eski_arsivler(&self) -> Sonuc<Vec<(String, Option<i64>)>> {
        hepsi(&self.kilit(), "SELECT * FROM eski_arsiv", [], |r| Ok((r.get::<_, String>("ad")?, r.get::<_, Option<i64>>("musteri")?)))
    }

    /// Müşterinin arşiv adları: kendi numarası önce (aynı gün iki yerdeyse o geçerli), sonra bağlı eski arşivler.
    pub fn arsivler(&self, numara: i64) -> Sonuc<Vec<String>> {
        let mut out = vec![numara.to_string()];
        out.extend(hepsi(&self.kilit(), "SELECT ad FROM eski_arsiv WHERE musteri = ? ORDER BY ad", [numara], |r| r.get::<_, String>("ad"))?);
        Ok(out)
    }

    /// {"<numara>": [eski arşiv adları (sıralı)]}: yalnızca bağlı eski arşivi olan müşteriler.
    pub fn rapor_gruplari(&self) -> Sonuc<Value> {
        let satirlar = hepsi(&self.kilit(), "SELECT ad, musteri FROM eski_arsiv WHERE musteri IS NOT NULL ORDER BY musteri, ad", [], |r| {
            Ok((r.get::<_, String>("ad")?, r.get::<_, i64>("musteri")?))
        })?;
        let mut out = Map::new();
        for (ad, musteri) in satirlar {
            if let Value::Array(a) = out.entry(musteri.to_string()).or_insert_with(|| Value::Array(Vec::new())) {
                a.push(Value::String(ad));
            }
        }
        Ok(Value::Object(out))
    }

    /// Atomik, 600 (wcpanel; root okur). Merkez açılışında ve eski arşiv bağı değişince.
    pub fn rapor_gruplari_yaz(&self, yol: &Path) -> Sonuc<()> {
        let g = self.rapor_gruplari()?;
        ortak::save_json(yol, &g).map_err(|e| Hata::Db(format!("rapor grupları yazılamadı: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(yol, std::fs::Permissions::from_mode(0o600)).map_err(|e| Hata::Db(format!("izin: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "veri_testler.rs"]
mod testler;
