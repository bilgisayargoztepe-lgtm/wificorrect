//! Hazır engel listeleri (Python listeler.py). Bu görevde yalnızca okuma tarafı (cihaz API'si: `liste_yolu`, `ozet`);
//! üretim (kaynak indirme, temizleme) Task 12'de eklenir.
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub const DIZIN: &str = "/var/lib/wificorrect/merkez/listeler";
/// Python `KATEGORILER` (sıra korunur: sağlık cevabındaki `listeler` bu sırayla yazılır).
pub const KATEGORILER: [(&str, &str); 3] =
    [("kumar", "Kumar ve bahis"), ("yetiskin", "Yetişkin içerik"), ("zararli", "Zararlı ve dolandırıcı siteler")];

pub fn kategori_gecerli(k: &str) -> bool {
    KATEGORILER.iter().any(|(a, _)| *a == k)
}

/// Python `liste_yolu`: bilinmeyen kategori `ValueError("Bilinmeyen kategori.")`.
pub fn liste_yolu(dizin: &Path, kategori: &str) -> Result<PathBuf, String> {
    if !kategori_gecerli(kategori) {
        return Err("Bilinmeyen kategori.".into());
    }
    Ok(dizin.join(format!("{kategori}.txt.gz")))
}

fn meta_yolu(dizin: &Path, kategori: &str) -> PathBuf {
    dizin.join(format!("{kategori}.json"))
}

/// Kategorinin üretim özeti (`<k>.json`); okunamıyor ya da sözlük değilse None.
pub fn meta_oku(dizin: &Path, kategori: &str) -> Option<Map<String, Value>> {
    let ham = std::fs::read(meta_yolu(dizin, kategori)).ok()?;
    match serde_json::from_slice::<Value>(&ham).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

fn sha_gecerli(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Sağlık cevabı için `{kategori: {sha256, sayi, zaman}}`; meta okunamayan ya da hiç liste üretilmemiş kategori yok.
pub fn ozet(dizin: &Path) -> Value {
    let mut o = Map::new();
    for (k, _) in KATEGORILER {
        let Some(m) = meta_oku(dizin, k) else { continue };
        let sha = m.get("sha256").and_then(Value::as_str).filter(|s| sha_gecerli(s));
        // Python `type(x) is int`: bool ve kayan nokta değil
        let sayi = m.get("sayi").filter(|v| v.is_i64() || v.is_u64());
        let zaman = m.get("zaman").filter(|v| v.is_string());
        if let (Some(sha), Some(sayi), Some(zaman)) = (sha, sayi, zaman) {
            let mut e = Map::new();
            e.insert("sha256".into(), Value::String(sha.to_string()));
            e.insert("sayi".into(), sayi.clone());
            e.insert("zaman".into(), zaman.clone());
            o.insert(k.to_string(), Value::Object(e));
        }
    }
    Value::Object(o)
}
