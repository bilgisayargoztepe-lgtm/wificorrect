//! `wificorrect-merkez veritabani-denetle VT [--dosyalar KOK]`: canlıya geçmeden önce Python'un yazdığı veritabanının her
//! satırının ve JSON dosyalarının Rust türleriyle okunabildiğini kanıtlar. Yalnızca okur (veritabanı `?mode=ro`, dosyalara
//! dokunulmaz) ve hiçbir değer basmaz: canlı veri kişisel veri içerir; çıktıda yalnızca tablo/dosya adı, sayı, kimlik
//! (numara, id, göreli yol) ve nedenin sınıfı bulunur.
//!
//! Çıktı: `TABLO <ad> <satır>` / `DOSYA <tür> <sayı>` bilgi satırları, her sorun için `HATA <tablo|dosya> <kimlik> <neden>`
//! ve hiç hata yoksa son satırda `TAMAM`. Çıkış kodu 0 (TAMAM) ya da 1. Okuma sıkıdır: bildirilen türden farklı saklama
//! sınıfı (INTEGER sütunda metin gibi) hatadır; sessiz varsayılan yoktur.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, Row};
use serde_json::Value;

use crate::veri;

/// Tablo başına en çok bu kadar hata satırı basılır; kalanı sayı olarak bildirilir.
const HATA_SINIRI: usize = 20;
/// Tek JSON dosyası için okuma sınırı (bozuk büyük dosya belleği tüketmesin).
const DOSYA_SINIRI: u64 = 64 << 20;
/// Dizin ağacı iç içe en çok bu kadar derin gezilir.
const DERINLIK: usize = 6;

/// Satır okuyucusu: satır türüne okur; sorun varsa nedenini söyler.
type Okuyucu = fn(&Row) -> Result<(), String>;

fn rusqlite_hata(e: rusqlite::Error) -> String {
    // Display yalnızca tür/sütun/dizin adları taşır (değer taşımaz)
    e.to_string().replace(['\n', '\r'], " ")
}

fn tur<T>(s: rusqlite::Result<T>) -> Result<(), String> {
    s.map(|_| ()).map_err(rusqlite_hata)
}

fn musteri(r: &Row) -> Result<(), String> {
    let m = veri::musteri_satir(r).map_err(rusqlite_hata)?;
    // Python `_gocur` bu iki düzeltmeyi açılışta yapardı; Rust yapmaz (envanter 0/0): rastlanırsa geçiş durur
    if m.lisans_bitis.is_empty() {
        return Err("lisans_bitis bos (Python goc duzeltmesi Rust'ta yok)".into());
    }
    if m.ap_parola.is_empty() {
        return Err("ap_parola bos (Python goc duzeltmesi Rust'ta yok)".into());
    }
    Ok(())
}

fn cihaz(r: &Row) -> Result<(), String> {
    tur(veri::cihaz_satir(r))
}

fn hareket(r: &Row) -> Result<(), String> {
    tur(veri::hareket_satir(r))
}

fn eski_arsiv(r: &Row) -> Result<(), String> {
    tur(r.get::<_, String>("ad").and_then(|_| r.get::<_, Option<i64>>("musteri")))
}

/// `ayar.deger`: `admin`, `sms`, `uyari` JSON sözlüğüdür (başka anahtarlar düz metin: kanal hedefi, sayaç, bayrak).
fn ayar(r: &Row) -> Result<(), String> {
    let anahtar: String = r.get("anahtar").map_err(rusqlite_hata)?;
    let deger: String = r.get("deger").map_err(rusqlite_hata)?;
    match anahtar.as_str() {
        "admin" => {
            let j = json_coz(&deger)?;
            let al = |k: &str| j.get(k).and_then(Value::as_str).is_some();
            if al("tuz") && al("ozet") && j.get("yineleme").and_then(Value::as_u64).is_some() {
                Ok(())
            } else {
                Err("admin ayari bozuk (tuz/ozet/yineleme)".into())
            }
        }
        "sms" | "uyari" => json_coz(&deger)?.is_object().then_some(()).ok_or_else(|| "ayar JSON sozluk degil".into()),
        a if a.starts_with("sms_sayac:") && deger.trim().parse::<i64>().is_err() => Err("sayac sayi degil".into()),
        _ => Ok(()),
    }
}

fn saglik(r: &Row) -> Result<(), String> {
    r.get::<_, i64>("cihaz").and_then(|_| r.get::<_, String>("zaman")).map_err(rusqlite_hata)?;
    let rapor: String = r.get("rapor").map_err(rusqlite_hata)?;
    json_coz(&rapor)?.is_object().then_some(()).ok_or_else(|| "rapor JSON sozluk degil".into())
}

fn sorun(r: &Row) -> Result<(), String> {
    tur(veri::sorun_satir(r))
}

fn surum(r: &Row) -> Result<(), String> {
    tur(veri::surum_satir(r))
}

fn sms_sayac(r: &Row) -> Result<(), String> {
    tur(r.get::<_, i64>("musteri").and_then(|_| r.get::<_, String>("gun")).and_then(|_| r.get::<_, i64>("sayi")))
}

fn sms_son(r: &Row) -> Result<(), String> {
    tur(veri::sms_son_satir(r))
}

fn bekleyen(r: &Row) -> Result<(), String> {
    tur(veri::bekleyen_satir(r))
}

/// JSON'u ayrıştırır; hata metni değer taşımaz (yalnızca sınıf, satır ve sütun).
fn json_coz(metin: &str) -> Result<Value, String> {
    serde_json::from_str(metin).map_err(|e| json_hata(&e))
}

fn json_hata(e: &serde_json::Error) -> String {
    format!("JSON bozuk ({:?}, satir {}, sutun {})", e.classify(), e.line(), e.column())
}

/// (tablo, kimlik ifadesi, okuyucu). Kimlik ifadesi yalnızca anahtar sütunları basar; `bekleyen.kimlik` 8 karaktere kısalır.
const TABLOLAR: [(&str, &str, Okuyucu); 11] = [
    ("musteri", "numara", musteri),
    ("cihaz", "id", cihaz),
    ("hareket", "id", hareket),
    ("eski_arsiv", "ad", eski_arsiv),
    ("ayar", "anahtar", ayar),
    ("saglik", "cihaz", saglik),
    ("sorun", "id", sorun),
    ("surum", "surum", surum),
    ("sms_sayac", "CAST(musteri AS TEXT) || '/' || CAST(gun AS TEXT)", sms_sayac),
    ("sms_son", "musteri", sms_son),
    ("bekleyen", "SUBSTR(CAST(kimlik AS TEXT), 1, 8)", bekleyen),
];

/// Hata biriktiren yazıcı: tablo/dosya başına sınırlı sayıda `HATA` satırı basar.
struct Rapor<'a> {
    cikti: &'a mut dyn Write,
    hata: usize,
}

impl Rapor<'_> {
    fn bilgi(&mut self, satir: &str) {
        let _ = writeln!(self.cikti, "{satir}");
    }

    fn hata(&mut self, yer: &str, kimlik: &str, neden: &str) {
        self.hata += 1;
        let _ = writeln!(self.cikti, "HATA {yer} {} {neden}", temiz(kimlik));
    }
}

/// Çıktıda boşluk ve denetim karakteri olmasın (satır biçimi bozulmasın).
fn temiz(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).collect();
    if t.is_empty() {
        "-".into()
    } else {
        t
    }
}

fn tablo_denetle(c: &Connection, r: &mut Rapor, tablo: &str, kimlik: &str, oku: Okuyucu) {
    let sql = format!("SELECT *, CAST(COALESCE({kimlik}, '?') AS TEXT) AS __kimlik FROM {tablo}");
    let mut st = match c.prepare(&sql) {
        Ok(s) => s,
        Err(e) => return r.hata(tablo, "-", &rusqlite_hata(e)),
    };
    let mut satirlar = match st.query([]) {
        Ok(s) => s,
        Err(e) => return r.hata(tablo, "-", &rusqlite_hata(e)),
    };
    let (mut sayi, mut sorunlu) = (0usize, 0usize);
    loop {
        let satir = match satirlar.next() {
            Ok(Some(s)) => s,
            Ok(None) => break,
            Err(e) => {
                r.hata(tablo, "-", &format!("satir okunamadi: {}", rusqlite_hata(e)));
                break;
            }
        };
        sayi += 1;
        if let Err(neden) = oku(satir) {
            sorunlu += 1;
            if sorunlu <= HATA_SINIRI {
                let k: String = satir.get::<_, String>("__kimlik").unwrap_or_else(|_| "?".into());
                r.hata(tablo, &k, &neden);
            } else {
                r.hata += 1;
            }
        }
    }
    if sorunlu > HATA_SINIRI {
        let _ = writeln!(r.cikti, "HATA {tablo} - {} hata daha (listelenmedi)", sorunlu - HATA_SINIRI);
    }
    r.bilgi(&format!("TABLO {tablo} {sayi}"));
}

/// Veritabanını salt okunur açıp şema ve her tablonun her satırını denetler.
fn veritabani_denetle(vt: &Path, r: &mut Rapor) {
    let c = match Connection::open_with_flags(vt, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) {
        Ok(c) => c,
        Err(e) => return r.hata("veritabani", "-", &format!("acilamadi: {}", rusqlite_hata(e))),
    };
    if let Err(e) = c.busy_timeout(std::time::Duration::from_secs(5)) {
        return r.hata("veritabani", "-", &rusqlite_hata(e));
    }
    // dosya bozuksa satır okumaları anlamsız olur
    match c.prepare("PRAGMA integrity_check").and_then(|mut s| s.query_map([], |x| x.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()) {
        Ok(l) if l.len() == 1 && l[0] == "ok" => {}
        Ok(l) => return r.hata("veritabani", "-", &format!("butunluk denetimi basarisiz ({} bildirim)", l.len())),
        Err(e) => return r.hata("veritabani", "-", &format!("butunluk denetimi: {}", rusqlite_hata(e))),
    }
    if let Err(e) = veri::sema_denetle(&c) {
        return r.hata("sema", "-", &e.replace(['\n', '\r'], " "));
    }
    for (tablo, kimlik, oku) in TABLOLAR {
        tablo_denetle(&c, r, tablo, kimlik, oku);
    }
}

// --- dosyalar ---

/// KOK altındaki `*.json` dosyaları (sembolik bağ izlenmez), göreli yola göre sıralı.
fn json_dosyalari(kok: &Path, r: &mut Rapor) -> Vec<PathBuf> {
    fn yuru(d: &Path, derinlik: usize, kok: &Path, out: &mut Vec<PathBuf>, r: &mut Rapor) {
        let okuma = match std::fs::read_dir(d) {
            Ok(o) => o,
            Err(e) => {
                let g = d.strip_prefix(kok).unwrap_or(d).to_string_lossy().into_owned();
                return r.hata("dizin", &g, &format!("okunamadi: {:?}", e.kind()));
            }
        };
        for x in okuma.flatten() {
            let yol = x.path();
            let Ok(md) = std::fs::symlink_metadata(&yol) else { continue };
            if md.is_dir() && derinlik < DERINLIK {
                yuru(&yol, derinlik + 1, kok, out, r);
            } else if md.is_file() && yol.extension().is_some_and(|e| e == "json") {
                out.push(yol);
            }
        }
    }
    let mut out = Vec::new();
    yuru(kok, 0, kok, &mut out, r);
    out.sort();
    out
}

/// Göreli yola göre dosyanın türünü ve türe özgü denetimini seçer: `(tür adı, sonuç)`.
fn dosya_denetle(bilesenler: &[String], yol: &Path, v: &Value, kok: &Path) -> (&'static str, Result<(), String>) {
    let nesne = |v: &Value| v.is_object().then_some(()).ok_or_else(|| "JSON sozluk degil".to_string());
    match bilesenler.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["durum.json"] => ("durum", nesne(v)),
        ["merkez", "rapor-gruplari.json"] => (
            "merkez",
            v.as_object()
                .filter(|o| o.values().all(|d| d.as_array().is_some_and(|a| a.iter().all(Value::is_string))))
                .map(|_| ())
                .ok_or_else(|| "{numara: [arsiv adlari]} degil".to_string()),
        ),
        ["merkez", "yonetici.json"] => ("merkez", crate::yonetici::oku(yol).map(|_| ()).ok_or_else(|| "yonetici hesabi okunamadi".to_string())),
        ["merkez", "listeler", f] => {
            let kategori = f.trim_end_matches(".json");
            ("merkez", crate::listeler::meta_oku(&kok.join("merkez/listeler"), kategori).map(|_| ()).ok_or_else(|| "liste ozeti okunamadi".to_string()))
        }
        ["merkez", ..] => ("merkez", Ok(())),
        ["istatistik", _] => (
            "istatistik",
            v.as_object().filter(|o| o.values().all(|n| n.is_u64())).map(|_| ()).ok_or_else(|| "{gun: sayi} degil".to_string()),
        ),
        ["detay", _, "kisiler.json"] => ("detay", nesne(v)),
        ["detay", _, _] => (
            "detay",
            (v.get("gun").is_some_and(Value::is_string) && v.get("kisiler").is_some_and(Value::is_array)).then_some(()).ok_or_else(|| "gun/kisiler yok".to_string()),
        ),
        ["rapor", n, "liste.json"] => (
            "rapor",
            if !crate::rapor::numara_mi(n) {
                Err("musteri numarasi gecersiz".to_string())
            } else {
                match v.as_array() {
                    Some(a) if a.iter().all(|x| crate::raporlar::liste_ogesi(x).is_some()) => Ok(()),
                    Some(_) => Err("liste ogesi gecersiz".to_string()),
                    None => Err("JSON dizi degil".to_string()),
                }
            },
        ),
        ["rapor", n, f] => {
            let govde = f.trim_end_matches(".json");
            let cozum = govde.split_once('-').map(|(t, d)| (t.to_string(), d.to_string()));
            let sonuc = match cozum {
                Some((t, d)) if crate::rapor::numara_mi(n) && crate::raporlar::donem_gecerli(&t, &d) => {
                    crate::raporlar::normal_rapor(v, &t, &d).map(|_| ()).ok_or_else(|| "rapor biçimi okunamadi (bicim/tur/donem/tarih)".to_string())
                }
                _ => Err("rapor dosya adi gecersiz".to_string()),
            };
            ("rapor", sonuc)
        }
        ["rapor-ham", _, _] => ("rapor-ham", crate::rapor::ham_gecerli(v, None).then_some(()).ok_or_else(|| "ham ozet gecersiz (bicim)".to_string())),
        ["kuyruk-sonuc", _] => ("kuyruk-sonuc", nesne(v)),
        ["kuyruk", _] => ("kuyruk", crate::kuyruk::is_oku(yol).map(|_| ()).ok_or_else(|| "is dosyasi okunamadi".to_string())),
        _ => ("diger", Ok(())),
    }
}

fn dosyalari_denetle(kok: &Path, r: &mut Rapor) {
    let mut sayilar: std::collections::BTreeMap<&'static str, usize> = std::collections::BTreeMap::new();
    for yol in json_dosyalari(kok, r) {
        let goreli = yol.strip_prefix(kok).unwrap_or(&yol);
        let bilesenler: Vec<String> = goreli.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let kimlik = bilesenler.join("/");
        let mut ham = Vec::new();
        let okundu = std::fs::File::open(&yol).and_then(|f| f.take(DOSYA_SINIRI + 1).read_to_end(&mut ham));
        if let Err(e) = okundu {
            r.hata("dosya", &kimlik, &format!("okunamadi: {:?}", e.kind()));
            continue;
        }
        if ham.len() as u64 > DOSYA_SINIRI {
            r.hata("dosya", &kimlik, "cok buyuk");
            continue;
        }
        let v: Value = match serde_json::from_slice(&ham) {
            Ok(v) => v,
            Err(e) => {
                r.hata("dosya", &kimlik, &json_hata(&e));
                continue;
            }
        };
        let (tur, sonuc) = dosya_denetle(&bilesenler, &yol, &v, kok);
        *sayilar.entry(tur).or_insert(0) += 1;
        if let Err(neden) = sonuc {
            r.hata("dosya", &kimlik, &neden);
        }
    }
    for (tur, n) in sayilar {
        r.bilgi(&format!("DOSYA {tur} {n}"));
    }
}

/// Denetimi çalıştırır; `true`: hata yok (çıktının son satırı `TAMAM`).
pub fn denetle(vt: &Path, dosyalar: Option<&Path>, cikti: &mut dyn Write) -> bool {
    let mut r = Rapor { cikti, hata: 0 };
    veritabani_denetle(vt, &mut r);
    if let Some(kok) = dosyalar {
        dosyalari_denetle(kok, &mut r);
    }
    if r.hata == 0 {
        r.bilgi("TAMAM");
    }
    r.hata == 0
}

const KULLANIM: &str = "Kullanım: wificorrect-merkez veritabani-denetle <veritabani> [--dosyalar /var/lib/wificorrect]";

/// `veritabani-denetle` alt komutu: çıkış kodu 0 (TAMAM), 1 (hata), 2 (kullanım).
pub fn main_uretim(args: &[String]) -> i32 {
    let (mut vt, mut kok): (Option<&String>, Option<&String>) = (None, None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dosyalar" if kok.is_none() && i + 1 < args.len() => {
                kok = Some(&args[i + 1]);
                i += 2;
            }
            a if !a.starts_with("--") && vt.is_none() => {
                vt = Some(&args[i]);
                i += 1;
            }
            _ => {
                eprintln!("{KULLANIM}");
                return 2;
            }
        }
    }
    let Some(vt) = vt else {
        eprintln!("{KULLANIM}");
        return 2;
    };
    let mut cikti = std::io::stdout().lock();
    if denetle(Path::new(vt), kok.map(Path::new), &mut cikti) {
        0
    } else {
        1
    }
}

#[cfg(test)]
#[path = "denetle_testler.rs"]
mod testler;
