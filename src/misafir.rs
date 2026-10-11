//! Misafir kodu deposu (otel/yurt): üret, iptal et, uzat, doğrula, temizle. `misafir_kodlari.json` (600, atomik).
//! `kod_id` panel bağlantıları içindir (kodun kendisi URL'de görünmez). Sonraki görevlerde kullanılır.
#![allow(dead_code)]

use crate::{hesap, ortak};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Geçerlilik seçenekleri (saat): 1 gün (varsayılan), 2, 3 gün, 1, 2 hafta, 1 ay.
pub const SURELER: [u64; 6] = [24, 48, 72, 168, 336, 720];
const SAKLAMA_SN: f64 = 30.0 * 86400.0;
/// Bir misafir kodu oturumunun makul en uzun süresi (1 ay + üst üste uzatmalar): resmi talepte IP → oturum araması
/// bu kadar geriye bakar.
pub const EN_UZUN_OTURUM_SN: f64 = 90.0 * 86400.0;
/// Bir koda (kod yoksa kimliğe) bu kadar hatalı denemeden sonra kod portalda reddedilir; personel yeni kod üretir.
pub const KOD_HATA_TAVANI: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Kod {
    /// Panel bağlantılarındaki kimlik (8 hex); kodun kendisi değil.
    #[serde(default)]
    pub kod_id: String,
    pub kod: String,
    /// Kişi anahtarı (`TC:…` / `PASAPORT:ISO2:NO`).
    pub kimlik: String,
    pub ad: String,
    pub soyad: String,
    pub oda: String,
    pub uretim: f64,
    pub bitis: f64,
    pub ureten: String,
    #[serde(default)]
    pub iptal: bool,
}

impl Kod {
    pub fn aktif(&self, now: f64) -> bool {
        !self.iptal && self.bitis > now
    }
}

fn dosya(state_root: impl AsRef<Path>) -> std::path::PathBuf {
    state_root.as_ref().join("misafir_kodlari.json")
}

/// Yalnızca OKUMA için (portal doğrulaması: bozuk dosya → hiçbir kod eşleşmez, kapalı kalır).
/// Yazan her yol (panel üret/iptal/uzat, temizle) `oku_kesin` kullanmalı, Err'de kaydetmeden çıkmalı.
pub fn oku(state_root: impl AsRef<Path>) -> Vec<Kod> {
    std::fs::read(dosya(state_root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Yazmadan önce okuma: dosya yoksa boş, var ama okunamıyor/bozuksa hata (üzerine yazılıp etkin kodlar silinmesin).
pub fn oku_kesin(state_root: impl AsRef<Path>) -> Result<Vec<Kod>, String> {
    match std::fs::read(dosya(state_root)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Misafir kodları okunamadı ({e}).")),
        Ok(b) => serde_json::from_slice(&b).map_err(|_| "Misafir kodları okunamadı (dosya bozuk).".to_string()),
    }
}

pub fn kaydet(state_root: impl AsRef<Path>, kodlar: &[Kod]) -> std::io::Result<()> {
    let b = serde_json::to_vec_pretty(kodlar).map_err(std::io::Error::other)?;
    ortak::write_atomic(&dosya(state_root), &b)
}

/// 000000–999999, sapmasız (ret örneklemesi).
fn alti_hane() -> String {
    const SINIR: u32 = u32::MAX / 1_000_000 * 1_000_000;
    loop {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
        let n = u32::from_le_bytes(b);
        if n < SINIR {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

/// Yeni kod üretir; aynı kimliğin etkin eski kodu iptal edilir ve (oturumları kapatılsın diye) döndürülür.
#[allow(clippy::too_many_arguments)]
pub fn uret(
    kodlar: &mut Vec<Kod>,
    kimlik: &str,
    ad: &str,
    soyad: &str,
    oda: &str,
    sure_saat: u64,
    ureten: &str,
    now: f64,
) -> (Kod, Option<Kod>) {
    let mut eski = None;
    for k in kodlar.iter_mut() {
        if k.aktif(now) && k.kimlik == kimlik {
            k.iptal = true;
            eski.get_or_insert_with(|| k.clone());
        }
    }
    let kod = loop {
        let c = alti_hane();
        if !kodlar.iter().any(|k| k.aktif(now) && k.kod == c) {
            break c;
        }
    };
    let yeni = Kod {
        kod_id: ortak::random_hex(4),
        kod,
        kimlik: kimlik.into(),
        ad: ad.into(),
        soyad: soyad.into(),
        oda: oda.into(),
        uretim: now,
        bitis: now + sure_saat as f64 * 3600.0,
        ureten: ureten.into(),
        iptal: false,
    };
    kodlar.push(yeni.clone());
    (yeni, eski)
}

/// Kodu iptal eder (zaten iptalse `None`); iptal edilen kayıt döner.
pub fn iptal(kodlar: &mut [Kod], kod_id: &str) -> Option<Kod> {
    let k = kodlar.iter_mut().find(|k| k.kod_id == kod_id && !k.iptal)?;
    k.iptal = true;
    Some(k.clone())
}

/// Mevcut bitişin (süresi dolmuşsa şimdinin) üzerine ekler; iptal edilmiş kod uzatılamaz. Yeni bitişi döner.
pub fn uzat(kodlar: &mut [Kod], kod_id: &str, sure_saat: u64, now: f64) -> Option<f64> {
    let k = kodlar.iter_mut().find(|k| k.kod_id == kod_id && !k.iptal)?;
    k.bitis = k.bitis.max(now) + sure_saat as f64 * 3600.0;
    Some(k.bitis)
}

/// Kimlik + kod eşleşen etkin kayıt. Bütün kayıtlar gezilir, her ikisi de sabit süreli karşılaştırılır:
/// kimliğin var olup olmadığı süreden anlaşılmaz.
pub fn dogrula<'a>(kodlar: &'a [Kod], kimlik: &str, kod: &str, now: f64) -> Option<&'a Kod> {
    let mut bulunan = None;
    for k in kodlar {
        let a = hesap::ct_eq(&k.kimlik, kimlik);
        let b = hesap::ct_eq(&k.kod, kod);
        if a & b & k.aktif(now) && bulunan.is_none() {
            bulunan = Some(k);
        }
    }
    bulunan
}

/// Bitişinden 30 gün geçenleri siler (iptal edilmiş olsun olmasın).
pub fn temizle(kodlar: &mut Vec<Kod>, now: f64) {
    kodlar.retain(|k| k.bitis + SAKLAMA_SN > now);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TC: &str = "TC:10000000146";
    const NOW: f64 = 1_800_000_000.0;

    fn bir(k: &mut Vec<Kod>, kimlik: &str, saat: u64) -> Kod {
        uret(k, kimlik, "Ali", "Veli", "101", saat, "mudur", NOW).0
    }

    #[test]
    fn uret_alti_hane_tekil() {
        let mut k = Vec::new();
        for i in 0..300 {
            bir(&mut k, &format!("TC:{i}"), 24);
        }
        let mut kodlar: Vec<_> = k.iter().map(|x| x.kod.clone()).collect();
        assert!(kodlar.iter().all(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_digit())));
        kodlar.sort();
        kodlar.dedup();
        assert_eq!(kodlar.len(), 300);
        let mut idler: Vec<_> = k.iter().map(|x| x.kod_id.clone()).collect();
        assert!(idler.iter().all(|i| i.len() == 8));
        idler.sort();
        idler.dedup();
        assert_eq!(idler.len(), 300);
    }

    #[test]
    fn ayni_kimlik_eskisini_iptal() {
        let mut k = Vec::new();
        let ilk = bir(&mut k, TC, 24);
        let (yeni, eski) = uret(&mut k, TC, "Ali", "Veli", "102", 24, "mudur", NOW + 10.0);
        assert_eq!(eski.as_ref().map(|e| e.kod_id.clone()), Some(ilk.kod_id.clone()));
        assert!(k.iter().find(|x| x.kod_id == ilk.kod_id).unwrap().iptal);
        assert!(!k.iter().find(|x| x.kod_id == yeni.kod_id).unwrap().iptal);
        let (_, e) = uret(&mut k, "TC:11111111110", "A", "B", "1", 24, "m", NOW);
        assert!(e.is_none());
    }

    #[test]
    fn uzat_bitis_uzerine_ekler() {
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 24);
        let yeni = uzat(&mut k, &c.kod_id, 48, NOW + 5000.0).unwrap();
        assert_eq!(yeni, NOW + 72.0 * 3600.0);
        assert_eq!(k[0].bitis, yeni);
        assert!(uzat(&mut k, "yok", 24, NOW).is_none());
        iptal(&mut k, &c.kod_id);
        assert!(uzat(&mut k, &c.kod_id, 24, NOW).is_none());
    }

    #[test]
    fn bir_hafta_bitis_dogru() {
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 168);
        assert_eq!(c.bitis, NOW + 7.0 * 86400.0);
        assert_eq!(c.uretim, NOW);
        assert!(SURELER.contains(&168));
    }

    #[test]
    fn dogrula_iptal_ve_suresi_dolmus_red() {
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 24);
        assert_eq!(dogrula(&k, TC, &c.kod, NOW + 1.0).map(|x| x.oda.as_str()), Some("101"));
        assert!(dogrula(&k, TC, if c.kod == "000000" { "000001" } else { "000000" }, NOW).is_none());
        assert!(dogrula(&k, "TC:11111111110", &c.kod, NOW).is_none());
        assert!(dogrula(&k, TC, &c.kod, c.bitis).is_none());
        assert!(dogrula(&k, TC, &c.kod, c.bitis + 1.0).is_none());
        assert_eq!(iptal(&mut k, &c.kod_id).map(|x| x.kod_id), Some(c.kod_id.clone()));
        assert!(iptal(&mut k, &c.kod_id).is_none());
        assert!(dogrula(&k, TC, &c.kod, NOW + 1.0).is_none());
    }

    #[test]
    fn temizle_30_gun() {
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 24);
        let b = c.bitis;
        temizle(&mut k, b + 29.0 * 86400.0);
        assert_eq!(k.len(), 1);
        temizle(&mut k, b + 30.0 * 86400.0 + 1.0);
        assert!(k.is_empty());
    }

    #[test]
    fn gidis_donus() {
        let kok = std::env::temp_dir().join(format!("wfc-misafir-{}", ortak::random_hex(4)));
        assert!(oku(&kok).is_empty());
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 72);
        kaydet(&kok, &k).unwrap();
        assert_eq!(oku(&kok), k);
        assert_eq!(oku(&kok)[0].kod_id, c.kod_id);
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(kok.join("misafir_kodlari.json")).unwrap().permissions().mode();
        assert_eq!(m & 0o777, 0o600);
        std::fs::write(kok.join("misafir_kodlari.json"), b"bozuk").unwrap();
        assert!(oku(&kok).is_empty());
        let _ = std::fs::remove_dir_all(&kok);
    }

    #[test]
    fn oku_kesin_bozuk_dosyayi_korur() {
        let kok = std::env::temp_dir().join(format!("wfc-misafir-{}", ortak::random_hex(4)));
        assert_eq!(oku_kesin(&kok), Ok(Vec::new()));
        std::fs::create_dir_all(&kok).unwrap();
        let y = kok.join("misafir_kodlari.json");
        std::fs::write(&y, b"bozuk").unwrap();
        assert!(oku_kesin(&kok).is_err());
        assert_eq!(std::fs::read(&y).unwrap(), b"bozuk");
        let mut k = Vec::new();
        bir(&mut k, TC, 24);
        kaydet(&kok, &k).unwrap();
        assert_eq!(oku_kesin(&kok).unwrap(), k);
        let _ = std::fs::remove_dir_all(&kok);
    }

    #[test]
    fn uzat_suresi_dolmusu_simdiden_uzatir() {
        let mut k = Vec::new();
        let c = bir(&mut k, TC, 24);
        let simdi = c.bitis + 10_000.0;
        let yeni = uzat(&mut k, &c.kod_id, 24, simdi).unwrap();
        assert_eq!(yeni, simdi + 86400.0);
        assert!(dogrula(&k, TC, &c.kod, simdi + 1.0).is_some());
    }
}
