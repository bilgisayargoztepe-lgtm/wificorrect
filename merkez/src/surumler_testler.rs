//! surumler.rs testleri: Python tests/test_surumler.py (7 testten 5'i; `test_kur_sh_ve_saatlik_zamanlayici` ve
//! `test_kur_sh_yerel_modulleri_kuruyor` kur.sh/birim denetimidir: Task 13). Ağa çıkılmaz: `SahteIndirme`.
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{self as Y, SahteIndirme};

type Sonuc = Result<Vec<u8>, String>;

fn sha(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

fn release(tag: &str, ad_deb: &str, ad_sums: &str) -> Value {
    json!({"tag_name": tag, "assets": [
        {"name": ad_deb, "browser_download_url": format!("https://x/{tag}/{ad_deb}")},
        {"name": ad_sums, "browser_download_url": format!("https://x/{tag}/{ad_sums}")}]})
}

/// Python `kur_dosyalar(deb041, sums_sha, ek)`.
fn kur_dosyalar(deb041: &[u8], sums_sha: Option<&str>, ek: Vec<Value>) -> SahteIndirme {
    let (d041, d040) = (deb041.to_vec(), b"deb-0.4.0".to_vec());
    let mut rel = vec![release("v0.4.1", "wificorrect_0.4.1_amd64.deb", "SHA256SUMS"), release("v0.4.0", "wificorrect_0.4.0_amd64.deb", "SHA256SUMS")];
    rel.extend(ek);
    let sums041 = format!("{}  wificorrect_0.4.1_amd64.deb\n", sums_sha.map(String::from).unwrap_or_else(|| sha(&d041)));
    let sums040 = format!("{}  wificorrect_0.4.0_amd64.deb\n", sha(&d040));
    let m: HashMap<String, Sonuc> = HashMap::from([
        (GITHUB_LISTE.to_string(), Ok(serde_json::to_vec(&rel).unwrap())),
        ("https://x/v0.4.1/wificorrect_0.4.1_amd64.deb".to_string(), Ok(d041)),
        ("https://x/v0.4.1/SHA256SUMS".to_string(), Ok(sums041.into_bytes())),
        ("https://x/v0.4.0/wificorrect_0.4.0_amd64.deb".to_string(), Ok(d040)),
        ("https://x/v0.4.0/SHA256SUMS".to_string(), Ok(sums040.into_bytes())),
    ]);
    SahteIndirme::yeni(m)
}

fn surumler(v: &Veri) -> Vec<String> {
    v.surumler().unwrap().into_iter().map(|s| s.surum).collect()
}

fn tmp_var(dizin: &Path) -> bool {
    std::fs::read_dir(dizin).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
}

#[test]
fn github_al_dogru_ekler_yanlisi_atlar_tekrar_indirmez() {
    let _k = Kayit::bas("test_surumler", "test_github_al_dogru_ekler_yanlisi_atlar_tekrar_indirmez");
    let o = Y::ortam();
    let dizin = o.dizin.join("surumler");
    let f = kur_dosyalar(b"deb-0.4.1", Some(&"0".repeat(64)), vec![]); // 0.4.1'in SHA'sı tutmuyor
    let (eklenen, hatalar) = github_al(&o.veri, &dizin, &f).unwrap();
    assert!(eklenen == ["0.4.0"] && hatalar.len() == 1 && hatalar[0].contains("0.4.1"));
    assert_eq!(surumler(&o.veri), ["0.4.0"]);
    let yol = paket_yolu(&dizin, "0.4.0").unwrap();
    assert_eq!(std::fs::read(&yol).unwrap(), b"deb-0.4.0");
    assert_eq!(std::fs::metadata(&yol).unwrap().permissions().mode() & 0o7777, 0o644);
    assert!(!paket_yolu(&dizin, "0.4.1").unwrap().exists() && !tmp_var(&dizin));
    let k = o.veri.surum("0.4.0").unwrap().unwrap();
    assert!(k.sha256 == sha(b"deb-0.4.0") && k.boyut == 9 && k.kaynak == "github");
    // ikinci çağrı: 0.4.0 yeniden indirilmez
    f.cagrilari_sil();
    let (eklenen, _) = github_al(&o.veri, &dizin, &f).unwrap();
    assert!(eklenen.is_empty() && !f.cagrilar().iter().any(|u| u.contains("v0.4.0/wificorrect")));
    // düzelince 0.4.1 eklenir
    let (eklenen, hatalar) = github_al(&o.veri, &dizin, &kur_dosyalar(b"deb-0.4.1", None, vec![])).unwrap();
    assert!(eklenen == ["0.4.1"] && hatalar.is_empty());
    assert_eq!(surumler(&o.veri), ["0.4.1", "0.4.0"]);
}

#[test]
fn gecersiz_etiket_atlanir() {
    let _k = Kayit::bas("test_surumler", "test_gecersiz_etiket_atlanir");
    let o = Y::ortam();
    let f = kur_dosyalar(
        b"deb-0.4.1",
        None,
        vec![release("v1.2", "wificorrect_1.2_amd64.deb", "SHA256SUMS"), release("vx", "wificorrect_x_amd64.deb", "SHA256SUMS")],
    );
    let (mut eklenen, hatalar) = github_al(&o.veri, &o.dizin.join("s"), &f).unwrap();
    eklenen.sort();
    assert!(eklenen == ["0.4.0", "0.4.1"] && hatalar.is_empty());
    assert!(!f.cagrilar().iter().any(|u| u.contains("v1.2") || u.contains("vx")));
}

#[test]
fn buyuk_dosya_reddedilir() {
    let _k = Kayit::bas("test_surumler", "test_buyuk_dosya_reddedilir");
    let o = Y::ortam();
    let buyuk = vec![b'x'; EN_COK_BAYT + 1];
    // taşıyıcı sınırı uygulamasa bile (sahte tümünü döndürür) indirici gövde uzunluğuna bakar
    let f = kur_dosyalar(&buyuk, None, vec![]);
    let (eklenen, hatalar) = github_al(&o.veri, &o.dizin.join("s"), &f).unwrap();
    assert!(eklenen == ["0.4.0"] && hatalar.len() == 1);
    assert!(o.veri.surum("0.4.1").unwrap().is_none());
    assert!(!paket_yolu(&o.dizin.join("s"), "0.4.1").unwrap().exists());
}

#[test]
fn yerel_ekle_ve_siralama() {
    let _k = Kayit::bas("test_surumler", "test_yerel_ekle_ve_siralama");
    let o = Y::ortam();
    let dizin = o.dizin.join("surumler");
    let dosya = o.dizin.join("a.deb");
    std::fs::write(&dosya, b"yerel").unwrap();
    yerel_ekle(&o.veri, &dizin, &dosya, "0.10.0").unwrap();
    yerel_ekle(&o.veri, &dizin, &dosya, "0.9.2").unwrap();
    assert_eq!(surumler(&o.veri), ["0.10.0", "0.9.2"]); // sayısal sıra
    assert!(o.veri.surum("0.9.2").unwrap().unwrap().kaynak == "yerel" && o.veri.surum("9.9.9").unwrap().is_none());
    assert_eq!(std::fs::read(paket_yolu(&dizin, "0.9.2").unwrap()).unwrap(), b"yerel");
    assert!(yerel_ekle(&o.veri, &dizin, &dosya, "../x").is_err());
}

#[test]
fn otomatik_al_ekleneni_harekete_hatayi_gunluge_yazar() {
    let _k = Kayit::bas("test_surumler", "test_otomatik_al_ekleneni_harekete_hatayi_gunluge_yazar");
    let o = Y::ortam();
    let dizin = o.dizin.join("surumler");
    let mut gunluk: Vec<u8> = Vec::new();
    assert_eq!(otomatik_al(&o.veri, &dizin, &kur_dosyalar(b"deb-0.4.1", Some(&"0".repeat(64)), vec![]), &mut gunluk), 0);
    assert_eq!(surumler(&o.veri), ["0.4.0"]);
    let alinan: Vec<(String, String)> =
        o.veri.hareketler(None, 300, None).unwrap().into_iter().filter(|h| h.olay == "SURUM_ALINDI").map(|h| (h.kim, h.ayrinti)).collect();
    assert_eq!(alinan, [("surumler".to_string(), "surum=0.4.0 otomatik".to_string())]);
    let g = String::from_utf8(gunluk).unwrap();
    assert!(g.contains("0.4.1") && g.contains("SHA256 tutmuyor"));
    assert!(!o.veri.hareketler(None, 300, None).unwrap().iter().any(|h| h.ayrinti.contains("0.4.1"))); // hata harekete yazılmaz

    let ulasilamaz = SahteIndirme::yeni(HashMap::from([(GITHUB_LISTE.to_string(), Err("ag yok".to_string()))]));
    let mut gunluk: Vec<u8> = Vec::new();
    assert_eq!(otomatik_al(&o.veri, &dizin, &ulasilamaz, &mut gunluk), 1);
    assert!(String::from_utf8(gunluk).unwrap().contains("ag yok") && o.veri.hareketler(None, 300, None).unwrap().len() == 1);
}

#[test]
fn getir_http_hatasi_ve_sinir() {
    // sarmalayıcı: 2xx dışı durum hata (Python HTTPError), sınırı aşan gövde hata; istek kimliksiz GET + yönlendirmeli
    struct Sabit(u16, usize);
    impl Tasiyici for Sabit {
        fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String> {
            assert_eq!((i.yontem.as_str(), i.yonlendirme > 0, i.kullanici.is_none(), i.govde.is_none()), ("GET", true, true, true));
            assert_eq!(i.okuma_siniri, 11);
            assert!(i.basliklar.iter().any(|(k, v)| k == "User-Agent" && v == "wificorrect-merkez"));
            Ok((self.0, vec![b'a'; self.1]))
        }
    }
    assert_eq!(getir(&Sabit(200, 10), "https://x/", 10).unwrap().len(), 10);
    assert_eq!(getir(&Sabit(200, 11), "https://x/", 10), Err("Dosya çok büyük.".to_string()));
    assert_eq!(getir(&Sabit(404, 0), "https://x/", 10), Err("HTTP Error 404: Not Found".to_string()));
    assert_eq!(getir(&Sabit(302, 0), "https://x/", 10), Err("HTTP Error 302: Found".to_string()));
}

#[test]
fn kullanim_ve_surum_dogrulamasi() {
    assert!(surum_gecerli("0.4.28") && !surum_gecerli("0.4") && !surum_gecerli("0.4.x") && !surum_gecerli("٠.٤.١")); // yalnızca ASCII rakam
    assert_eq!(main_uretim(&[]), 2);
    assert_eq!(main_uretim(&["ekle".to_string(), "dosya".to_string()]), 2);
    assert_eq!(main_uretim(&["al".to_string(), "x".to_string()]), 2);
}
