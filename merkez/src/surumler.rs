//! Cihaz paketi (.deb) sürüm deposu (Python surumler.py): GitHub sürümlerinden doğrulayarak alır ya da elle eklenir.
//! Dosyalar `DIZIN/<sürüm>.deb`; sürüm kaydı (sha256, boyut) veritabanındadır.
//! Alt komutlar: `surumler ekle DOSYA SURUM` | `surumler al` (wc-surumler.timer, saatte bir).
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ortak::{hex, py_bol, py_dogru, py_str, sha_gecerli, DisIstek, Tasiyici};
use crate::veri::Veri;

pub const DIZIN: &str = "/var/lib/wificorrect/merkez/surumler";
pub const EN_COK_BAYT: usize = 50 * 1024 * 1024;
const LISTE_SINIRI: usize = 2 * 1024 * 1024;
const SUMS_SINIRI: usize = 64 * 1024;
pub(crate) const GITHUB_LISTE: &str = "https://api.github.com/repos/bilgisayargoztepe-lgtm/wificorrect/releases?per_page=10";
/// Python `urlopen(timeout=60)`: bağlanma ve her okuma 60 sn; toplam sınır yok. Burada toplam yalnızca güvenlik ağıdır.
const BAGLANMA_SURESI: Duration = Duration::from_secs(60);
const BOSTA_SURESI: Duration = Duration::from_secs(60);
const TOPLAM_SINIR: Duration = Duration::from_secs(1800);
/// Python urllib 10 yönlendirmeye kadar izlerdi; GitHub indirmesi için 5 yeter.
const YONLENDIRME: u32 = 5;

/// Python `SURUM_RE.fullmatch` (`\d+\.\d+\.\d+`); yalnızca ASCII rakam (spec bilinçli fark 5).
pub fn surum_gecerli(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
}

/// Python `paket_yolu`: geçersiz sürüm `ValueError("Geçersiz sürüm.")`.
pub fn paket_yolu(dizin: &Path, surum: &str) -> Result<PathBuf, String> {
    if !surum_gecerli(surum) {
        return Err("Geçersiz sürüm.".into());
    }
    Ok(dizin.join(format!("{surum}.deb")))
}

fn http_hata(kod: u16) -> String {
    let neden = match kod {
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    };
    format!("HTTP Error {kod}: {neden}")
}

/// Gerçek indirici (Python `getir`): yönlendirmeleri izler (http/https, en çok `YONLENDIRME`), okurken sınırı uygular.
/// 2xx dışı durum ve sınırı aşan gövde hatadır. Kimlik ve gövde yoktur (GitHub sürüm dosyaları ve liste kaynakları).
pub fn getir(t: &dyn Tasiyici, url: &str, sinir_bayt: usize) -> Result<Vec<u8>, String> {
    let i = DisIstek {
        yontem: "GET".into(),
        url: url.into(),
        basliklar: vec![("User-Agent".into(), "wificorrect-merkez".into())],
        govde: None,
        kullanici: None,
        zaman_asimi: TOPLAM_SINIR,
        okuma_siniri: sinir_bayt + 1,
        yonlendirme: YONLENDIRME,
        indirme_sureleri: Some((BAGLANMA_SURESI, BOSTA_SURESI)),
    };
    let (kod, veri) = t.gonder(&i)?;
    if !(200..300).contains(&kod) {
        return Err(http_hata(kod));
    }
    if veri.len() > sinir_bayt {
        return Err("Dosya çok büyük.".into());
    }
    Ok(veri)
}

/// SHA256SUMS içinde `ad` dosyasının özeti (küçük harf) ya da None.
fn sha_satiri(sums: &[u8], ad: &str) -> Option<String> {
    let metin = String::from_utf8_lossy(sums);
    for satir in crate::kuyruk::satirlar(&metin) {
        let parca = py_bol(satir);
        if parca.len() == 2 && parca[1].trim_start_matches('*') == ad {
            let s = parca[0].to_lowercase();
            if sha_gecerli(&s) {
                return Some(s);
            }
        }
    }
    None
}

static GECICI_SAYAC: AtomicU32 = AtomicU32::new(0);

/// Geçici ad her yazana özel: zamanlayıcı ile "GitHub'dan al" aynı sürümü aynı anda yazarsa biri öbürünün yarım dosyasını
/// yerine taşımasın. Dosya 0644 olur.
fn yaz(dizin: &Path, surum: &str, icerik: &[u8]) -> Result<(), String> {
    let hata = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(dizin).map_err(hata)?;
    let yol = paket_yolu(dizin, surum)?;
    let gecici = dizin.join(format!("{surum}.{}-{}.tmp", std::process::id(), GECICI_SAYAC.fetch_add(1, Ordering::SeqCst)));
    let sonuc = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&gecici)?;
        f.write_all(icerik)?;
        drop(f);
        std::fs::set_permissions(&gecici, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&gecici, &yol)
    })();
    if let Err(e) = sonuc {
        let _ = std::fs::remove_file(&gecici);
        return Err(e.to_string());
    }
    Ok(())
}

/// Bir GitHub sürümünü doğrulayıp ekler (hatası yalnızca o sürümü durdurur).
fn surum_al(veri: &Veri, dizin: &Path, t: &dyn Tasiyici, r: &serde_json::Map<String, Value>, s: &str) -> Result<(), String> {
    let ad = format!("wificorrect_{s}_amd64.deb");
    let mut ek: std::collections::HashMap<&str, &Value> = Default::default();
    match r.get("assets") {
        Some(v) if py_dogru(v) => match v {
            Value::Array(a) => {
                for x in a {
                    if let Some(o) = x.as_object() {
                        if let Some(n) = o.get("name").and_then(Value::as_str) {
                            ek.insert(n, o.get("browser_download_url").unwrap_or(&Value::Null));
                        }
                    }
                }
            }
            Value::Object(_) | Value::String(_) => {} // anahtarlar/karakterler sözlük değil: atlanır
            _ => return Err(format!("'{}' object is not iterable", if v.is_number() { "int" } else { "bool" })),
        },
        _ => {}
    }
    let url = |ad: &str| -> Result<Option<&str>, String> {
        match ek.get(ad) {
            Some(v) if py_dogru(v) => v.as_str().map(Some).ok_or_else(|| "indirme adresi metin değil".to_string()),
            _ => Ok(None),
        }
    };
    let (Some(deb_url), Some(sums_url)) = (url(&ad)?, url("SHA256SUMS")?) else {
        return Err("paket ya da SHA256SUMS eksik".into());
    };
    let beklenen = sha_satiri(&getir(t, sums_url, SUMS_SINIRI)?, &ad).ok_or("SHA256SUMS'ta paket satırı yok")?;
    let icerik = getir(t, deb_url, EN_COK_BAYT)?;
    if icerik.len() > EN_COK_BAYT {
        return Err("paket 50 MB'tan büyük".into());
    }
    if hex(&Sha256::digest(&icerik)) != beklenen {
        return Err("SHA256 tutmuyor".into());
    }
    yaz(dizin, s, &icerik)?;
    veri.surum_ekle(s, &beklenen, icerik.len() as i64, "github").map_err(|e| e.to_string())
}

/// GitHub'daki son sürümlerden depoda olmayanları doğrulayıp ekler. Dönen: (eklenen sürümler, hata metinleri).
pub fn github_al(veri: &Veri, dizin: &Path, t: &dyn Tasiyici) -> Result<(Vec<String>, Vec<String>), String> {
    let (mut eklenen, mut hatalar) = (Vec::new(), Vec::new());
    let ham = getir(t, GITHUB_LISTE, LISTE_SINIRI)?;
    let releaseler: Value = serde_json::from_slice(&ham).map_err(|e| e.to_string())?;
    let liste: &[Value] = match &releaseler {
        Value::Array(a) => a,
        Value::Object(_) | Value::String(_) => &[], // sözlüğün anahtarları / metnin karakterleri sözlük değil: atlanır
        _ => return Err("liste yinelenemez".into()),
    };
    for r in liste {
        let Some(o) = r.as_object() else { continue };
        let etiket = o.get("tag_name").map(py_str).unwrap_or_default();
        let Some(s) = etiket.strip_prefix('v').filter(|s| surum_gecerli(s)) else { continue };
        if veri.surum(s).map_err(|e| e.to_string())?.is_some() {
            continue;
        }
        // tek sürümün hatası diğerlerini durdurmaz
        match surum_al(veri, dizin, t, o, s) {
            Ok(()) => eklenen.push(s.to_string()),
            Err(ex) => hatalar.push(format!("{s}: {ex}")),
        }
    }
    Ok((eklenen, hatalar))
}

/// Elle: dosyanın sha256'sı hesaplanıp eklenir (GitHub doğrulaması yok).
pub fn yerel_ekle(veri: &Veri, dizin: &Path, dosya: &Path, surum: &str) -> Result<(), String> {
    if !surum_gecerli(surum) {
        return Err("Geçersiz sürüm.".into());
    }
    let mut icerik = Vec::new();
    std::fs::File::open(dosya).map_err(|e| format!("{}: {e}", dosya.display()))?.take(EN_COK_BAYT as u64 + 1).read_to_end(&mut icerik).map_err(|e| e.to_string())?;
    if icerik.len() > EN_COK_BAYT {
        return Err("paket 50 MB'tan büyük".into());
    }
    yaz(dizin, surum, &icerik)?;
    veri.surum_ekle(surum, &hex(&Sha256::digest(&icerik)), icerik.len() as i64, "yerel").map_err(|e| e.to_string())
}

/// Zamanlayıcının işi: GitHub'daki yeni sürümleri alır ("her zaman en son sürüm" kanalları bunlara geçer). Eklenenler
/// hareket satırına, hatalar yalnızca günlüğe: GitHub'a ulaşılamazken hareketler saatte bir satırla dolmasın. Dönen: çıkış kodu.
pub fn otomatik_al(veri: &Veri, dizin: &Path, t: &dyn Tasiyici, gunluk: &mut dyn Write) -> i32 {
    let (eklenen, hatalar) = match github_al(veri, dizin, t) {
        Ok(s) => s,
        Err(h) => {
            let _ = writeln!(gunluk, "GitHub sürüm listesi alınamadı: {h}");
            return 1;
        }
    };
    for s in &eklenen {
        if let Err(h) = veri.hareket("surumler", "-", "SURUM_ALINDI", None, &format!("surum={s} otomatik")) {
            let _ = writeln!(gunluk, "hareket yazılamadı: {h}");
            return 1;
        }
    }
    for h in &hatalar {
        let _ = writeln!(gunluk, "alınamadı: {h}");
    }
    0
}

/// `wificorrect-merkez surumler al | ekle DOSYA SURUM` (Python `main`; `args` alt komuttan sonrasıdır).
pub fn main_uretim(args: &[String]) -> i32 {
    let kullanim = || {
        crate::ortak::stderr_yaz("Kullanım: wificorrect-merkez surumler ekle DOSYA SURUM | wificorrect-merkez surumler al");
        2
    };
    let al = match args {
        [a] if a == "al" => true,
        [e, _, _] if e == "ekle" => false,
        _ => return kullanim(),
    };
    let v = match Veri::ac(Path::new(crate::merkez::DB), std::sync::Arc::new(crate::ortak::simdi)) {
        Ok(v) => v,
        Err(h) => {
            crate::ortak::stderr_yaz(&format!("{h}"));
            return 1;
        }
    };
    if al {
        return otomatik_al(&v, Path::new(DIZIN), &crate::ortak::Curl, &mut std::io::stderr());
    }
    match yerel_ekle(&v, Path::new(DIZIN), Path::new(&args[1]), &args[2]) {
        Ok(()) => {
            println!("eklendi: {}", args[2]);
            0
        }
        Err(h) => {
            crate::ortak::stderr_yaz(&format!("{h}"));
            1
        }
    }
}

#[cfg(test)]
#[path = "surumler_testler.rs"]
mod testler;
