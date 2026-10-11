//! Kimlik doğrulama ve kişi anahtarı: T.C. kimlik no, pasaport, maske, dosya adı, opak kimlik.
//! Kişi anahtarı: `TC:<11 hane>` / `PASAPORT:<ISO2>:<NO>`; telefonlu anahtarlar (`905…`) aynen kalır.
//! Sonraki görevlerde kullanılır; o zamana kadar ölü kod uyarısı kapalı.
#![allow(dead_code)]

use crate::ortak;
use sha2::{Digest, Sha256};
use std::path::Path;

/// T.C. kimlik no: 11 hane, ilk hane 0 değil, 10. ve 11. kontrol haneleri tutar.
pub fn tc_gecerli(s: &str) -> bool {
    if s.len() != 11 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let d: Vec<i32> = s.bytes().map(|b| (b - b'0') as i32).collect();
    if d[0] == 0 {
        return false;
    }
    let tek = d[0] + d[2] + d[4] + d[6] + d[8];
    let cift = d[1] + d[3] + d[5] + d[7];
    if (tek * 7 - cift).rem_euclid(10) != d[9] {
        return false;
    }
    d[..10].iter().sum::<i32>().rem_euclid(10) == d[10]
}

/// Pasaport no: boşluk/tire atılır, büyük harfe çevrilir; 5–20 karakter `[A-Z0-9]`.
pub fn pasaport_normal(s: &str) -> Option<String> {
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect();
    let ok = (5..=20).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    ok.then_some(t)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kimlik {
    Tc(String),
    Pasaport { uyruk: String, no: String },
}

impl Kimlik {
    pub fn anahtar(&self) -> String {
        match self {
            Kimlik::Tc(t) => format!("TC:{t}"),
            Kimlik::Pasaport { uyruk, no } => format!("PASAPORT:{uyruk}:{no}"),
        }
    }
}

/// Anahtardan kimliği geri kurar; telefon anahtarı ya da bozuk değer için `None`.
pub fn coz(anahtar: &str) -> Option<Kimlik> {
    if let Some(t) = anahtar.strip_prefix("TC:") {
        return tc_gecerli(t).then(|| Kimlik::Tc(t.to_string()));
    }
    let r = anahtar.strip_prefix("PASAPORT:")?;
    let (uyruk, no) = r.split_once(':')?;
    let uyruk_ok = uyruk.len() == 2 && uyruk.bytes().all(|b| b.is_ascii_uppercase());
    if !uyruk_ok || pasaport_normal(no).as_deref() != Some(no) {
        return None;
    }
    Some(Kimlik::Pasaport { uyruk: uyruk.to_string(), no: no.to_string() })
}

/// Ekranda gösterim: gizlenen orta kısım birer birer `*` olur. Biçime bakar, sağlamaya bakmaz
/// (kayıtta bozuk de olsa kimlik sızmasın).
/// TC `TC:12345678901` → `123******01`; pasaport `PASAPORT:DE:X1234567` → `DE X12***67`
/// (ilk 3 + yıldız + son 2; 6 haneden kısa numarada yalnızca ilk hane açık). Telefon anahtarı aynen.
pub fn maskele(anahtar: &str) -> String {
    if let Some(t) = anahtar.strip_prefix("TC:") {
        if t.len() == 11 && t.bytes().all(|b| b.is_ascii_digit()) {
            return format!("{}{}{}", &t[..3], "*".repeat(6), &t[9..]);
        }
        return anahtar.to_string();
    }
    if let Some(r) = anahtar.strip_prefix("PASAPORT:") {
        if let Some((uyruk, no)) = r.split_once(':') {
            let n = no.len();
            if uyruk.len() == 2 && n >= 1 && no.is_ascii() {
                let (bas, son) = if n >= 6 { (3, 2) } else { (1, 0) };
                return format!("{uyruk} {}{}{}", &no[..bas], "*".repeat(n - bas - son), &no[n - son..]);
            }
        }
    }
    anahtar.to_string()
}

/// Dosya adı için güvenli biçim: `:` → `_`, sonra yalnızca `[A-Za-z0-9_]` kalır (yol ayracı, nokta, NUL atılır;
/// telefon anahtarı aynen).
pub fn dosya_adi(anahtar: &str) -> String {
    anahtar.replace(':', "_").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
}

/// Kişi anahtarı mı: geçerli TC/pasaport anahtarı ya da 6–15 haneli telefon.
pub fn gecerli_anahtar(anahtar: &str) -> bool {
    coz(anahtar).is_some() || ((6..=15).contains(&anahtar.len()) && anahtar.bytes().all(|b| b.is_ascii_digit()))
}

/// `dosya_adi`'nın tersi: `TC_…` → `TC:…`, `PASAPORT_DE_X…` → `PASAPORT:DE:X…`, telefon aynen; geçersizse `None`.
pub fn dosya_adindan(ad: &str) -> Option<String> {
    let k = if let Some(t) = ad.strip_prefix("TC_") {
        format!("TC:{t}")
    } else if let Some(r) = ad.strip_prefix("PASAPORT_") {
        let (uyruk, no) = r.split_once('_')?;
        format!("PASAPORT:{uyruk}:{no}")
    } else {
        ad.to_string()
    };
    gecerli_anahtar(&k).then_some(k)
}

/// TC ya da pasaport anahtarı mı (biçime bakar; listelerde maskelenmesi gereken kimlik).
pub fn kimlik_mi(anahtar: &str) -> bool {
    anahtar.starts_with("TC:") || anahtar.starts_with("PASAPORT:")
}

/// Panel listelerinde kişi: kimlik maskeli, telefon `+905…` (bugünkü gibi), boşsa boş.
pub fn goster(anahtar: &str) -> String {
    match anahtar {
        "" => String::new(),
        k if kimlik_mi(k) => maskele(k),
        k => format!("+{k}"),
    }
}

/// Denetim satırına yazılacak serbest metin: kimlik anahtarı, 11 haneli geçerli TC ya da harf+rakam karışık sözcük
/// (pasaport olabilir) maskelenir. Ad, IP ve telefon aynen kalır.
pub fn denetim_metni(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let n = w.chars().count();
            if kimlik_mi(w) {
                maskele(w)
            } else if tc_gecerli(w) {
                maskele(&format!("TC:{w}"))
            } else if n >= 5 && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(char::is_alphabetic) {
                let c: Vec<char> = w.chars().collect();
                format!("{}{}{}", c[..3].iter().collect::<String>(), "*".repeat(n - 5), c[n - 2..].iter().collect::<String>())
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(anahtar: &[u8], veri: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if anahtar.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(anahtar));
    } else {
        k[..anahtar.len()].copy_from_slice(anahtar);
    }
    let ic: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let dis: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let mut h = Sha256::new();
    h.update(&ic);
    h.update(veri);
    let ic_ozet = h.finalize();
    let mut h = Sha256::new();
    h.update(&dis);
    h.update(ic_ozet);
    h.finalize().into()
}

/// URL'de kullanılacak opak kimlik: HMAC-SHA256(sır, anahtar) ilk 16 hex.
pub fn opak(sir: &[u8], anahtar: &str) -> String {
    hmac_sha256(sir, anahtar.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `<state_root>/kisi_sir` (32 bayt, 600): varsa okur, yoksa (ya da bozuksa) üretip yazar.
pub fn sir_oku_ya_uret(state_root: &str) -> Vec<u8> {
    let yol = Path::new(state_root).join("kisi_sir");
    if let Ok(b) = std::fs::read(&yol) {
        if b.len() == 32 {
            return b;
        }
    }
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
    if let Err(e) = ortak::write_atomic(&yol, &b) {
        eprintln!("kisi_sir yazılamadı: {e}");
    }
    b.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc_kontrol_basamagi() {
        assert!(tc_gecerli("10000000146"));
        assert!(tc_gecerli("11111111110"));
        assert!(!tc_gecerli("00000000000")); // ilk hane 0
        assert!(!tc_gecerli("1000000014")); // 10 hane
        assert!(!tc_gecerli("100000001466")); // 12 hane
        assert!(!tc_gecerli("1000000014a")); // harf
        assert!(!tc_gecerli("10000000147")); // 11. hane bozuk
        assert!(!tc_gecerli("10000000156")); // 10. hane bozuk
    }

    #[test]
    fn pasaport_normal_buyuk_harf_ve_sinirlar() {
        assert_eq!(pasaport_normal("x1234567").as_deref(), Some("X1234567"));
        assert_eq!(pasaport_normal(" ab 123-45 ").as_deref(), Some("AB12345"));
        assert_eq!(pasaport_normal("A123"), None);
        assert_eq!(pasaport_normal("A1234"), Some("A1234".into()));
        assert!(pasaport_normal(&"A".repeat(20)).is_some());
        assert_eq!(pasaport_normal(&"A".repeat(21)), None);
        assert_eq!(pasaport_normal("AB12_345"), None);
        assert_eq!(pasaport_normal("ÇA12345"), None);
    }

    #[test]
    fn anahtar_gidis_donus() {
        let t = Kimlik::Tc("10000000146".into());
        assert_eq!(t.anahtar(), "TC:10000000146");
        assert_eq!(coz("TC:10000000146"), Some(t));
        let p = Kimlik::Pasaport { uyruk: "DE".into(), no: "X1234567".into() };
        assert_eq!(p.anahtar(), "PASAPORT:DE:X1234567");
        assert_eq!(coz("PASAPORT:DE:X1234567"), Some(p));
        assert_eq!(coz("905334553132"), None);
        assert_eq!(coz("TC:10000000147"), None);
        assert_eq!(coz("PASAPORT:DEU:X1234567"), None);
        assert_eq!(coz("PASAPORT:de:X1234567"), None);
        assert_eq!(coz("PASAPORT:DE:x1234567"), None);
    }

    #[test]
    fn maske_bicimi() {
        assert_eq!(maskele("TC:12345678901"), "123******01");
        assert_eq!(maskele("TC:10000000146"), "100******46");
        assert_eq!(maskele("PASAPORT:DE:X1234567"), "DE X12***67");
        assert_eq!(maskele("PASAPORT:GB:AB123"), "GB A****");
        assert_eq!(maskele("905334553132"), "905334553132");
    }

    #[test]
    fn dosya_adi_iki_nokta_yok() {
        assert_eq!(dosya_adi("TC:10000000146"), "TC_10000000146");
        assert_eq!(dosya_adi("PASAPORT:DE:X1234567"), "PASAPORT_DE_X1234567");
        assert_eq!(dosya_adi("905334553132"), "905334553132");
        assert!(!dosya_adi("PASAPORT:DE:X1").contains(':'));
    }

    #[test]
    fn dosya_adi_yol_ayraci_ve_nul_atilir() {
        for (girdi, beklenen) in [("../x", "x"), ("a/b", "ab"), ("\0", ""), ("a\\b", "ab"), ("TC:1.2", "TC_12")] {
            assert_eq!(dosya_adi(girdi), beklenen, "{girdi:?}");
        }
        assert!(dosya_adi("../../etc/passwd").bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    }

    #[test]
    fn dosya_adindan_geri_kurar() {
        assert_eq!(dosya_adindan("TC_10000000146").as_deref(), Some("TC:10000000146"));
        assert_eq!(dosya_adindan("PASAPORT_DE_X1234567").as_deref(), Some("PASAPORT:DE:X1234567"));
        assert_eq!(dosya_adindan("905334553132").as_deref(), Some("905334553132"));
        for bad in ["TC_10000000147", "PASAPORT_DEU_X1234567", "abc", "12345", "", "index"] {
            assert_eq!(dosya_adindan(bad), None, "{bad}");
        }
        for k in ["TC:10000000146", "PASAPORT:DE:X1234567", "905334553132"] {
            assert_eq!(dosya_adindan(&dosya_adi(k)).as_deref(), Some(k));
        }
    }

    #[test]
    fn goster_ve_denetim_metni_maskeli() {
        assert_eq!(goster("TC:10000000146"), "100******46");
        assert_eq!(goster("PASAPORT:DE:X1234567"), "DE X12***67");
        assert_eq!(goster("905334553132"), "+905334553132");
        assert_eq!(goster(""), "");
        assert_eq!(denetim_metni("10000000146"), "100******46");
        assert_eq!(denetim_metni("X1234567"), "X12***67");
        assert_eq!(denetim_metni("ayşe 905334553132 10.50.0.23"), "ayşe 905334553132 10.50.0.23");
        assert_eq!(denetim_metni("TC:10000000146"), "100******46");
    }

    #[test]
    fn hmac_rfc4231_vektor_1() {
        let o = hmac_sha256(&[0x0b; 20], b"Hi There");
        let hex: String = o.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn opak_sabit_ve_16_hex() {
        let a = opak(b"sir", "TC:10000000146");
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(a, opak(b"sir", "TC:10000000146"));
        assert_ne!(a, opak(b"sir", "TC:11111111110"));
        assert_ne!(a, opak(b"baska", "TC:10000000146"));
    }

    #[test]
    fn sir_uretilir_ve_kalici() {
        let d = std::env::temp_dir().join(format!("wfc-kimlik-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let r = d.to_str().unwrap();
        let a = sir_oku_ya_uret(r);
        assert_eq!(a.len(), 32);
        assert_eq!(a, sir_oku_ya_uret(r));
        let _ = std::fs::remove_dir_all(&d);
    }
}
