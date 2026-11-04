//! Sürüm deposu (Python surumler.py). Bu görevde yalnızca okuma tarafı (cihaz API'si: `paket_yolu`, sürüm biçimi);
//! GitHub'dan alma ve elle ekleme Task 12'de eklenir.
use std::path::{Path, PathBuf};

pub const DIZIN: &str = "/var/lib/wificorrect/merkez/surumler";

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
