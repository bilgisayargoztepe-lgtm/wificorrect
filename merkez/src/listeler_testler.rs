//! listeler.rs testleri: Python tests/test_listeler.py (32 testten 29'u; `test_simdi_guncelle_mesgulse_beklemez` ve
//! `test_yonetim_listeler_sayfasi_ve_simdi_guncelle` Yonetim ister: Task 11; `test_kur_sh_ve_birimler`: Task 13).
//! Ağa çıkılmaz: bütün indirmeler `SahteIndirme` ile.
use std::collections::HashMap;

use serde_json::{json, Value};

use super::*;
use crate::altin::Kayit;
use crate::test_yardim::{self as Y, SahteIndirme};

const SIMDI: f64 = 1_790_000_000.0;
const K: &str = "kumar";

type Sonuc = Result<Vec<u8>, String>;

fn ok(s: &str) -> Sonuc {
    Ok(s.as_bytes().to_vec())
}

fn hata(s: &str) -> Sonuc {
    Err(s.to_string())
}

/// Python `sahte(dosyalar)`.
fn sahte(d: &[(&str, Sonuc)]) -> SahteIndirme {
    SahteIndirme::yeni(d.iter().map(|(u, s)| (u.to_string(), s.clone())).collect::<HashMap<_, _>>())
}

fn gz_oku(yol: &Path) -> String {
    let mut s = String::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(yol).unwrap()).read_to_string(&mut s).unwrap();
    s
}

fn kume(a: &[&str]) -> BTreeSet<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn tem(satirlar: &[&str], bicim: Bicim) -> BTreeSet<String> {
    temizle(satirlar.iter().copied(), bicim)
}

fn k1(ad: &'static str, url: &'static str, bicim: Bicim) -> Kaynak {
    Kaynak { ad, url, bicim }
}

/// Python `iki_kaynak()` (KAYNAKLAR değiştirmesi): yalnızca kumar.
fn iki_kaynak() -> Uretici {
    Uretici {
        kaynaklar: vec![(K, vec![k1("A", "https://a/liste", Bicim::Duz), k1("B", "https://b/liste", Bicim::Adblock)])],
        en_cok: EN_COK,
    }
}

// --- ayrıştırma ve temizleme ---
#[test]
fn hosts_bicimi() {
    let _k = Kayit::bas("test_listeler", "test_hosts_bicimi");
    let s = [
        "# yorum", "127.0.0.1 localhost", "127.0.0.1 localhost.localdomain", "0.0.0.0 0.0.0.0", "0.0.0.0 Kotu-Site.COM  # sonda yorum",
        "0.0.0.0 a.example.net b.example.org", "", "  0.0.0.0   bosluklu.com.tr  ",
    ];
    assert_eq!(tem(&s, Bicim::Hosts), kume(&["kotu-site.com", "a.example.net", "b.example.org", "bosluklu.com.tr"]));
}

#[test]
fn duz_bicimi() {
    let _k = Kayit::bas("test_listeler", "test_duz_bicimi");
    let s = ["# HaGeZi", "! baska yorum", "kumar.com", "BAHIS.net.", "  bet365.com  ", "x.y.z.org # not"];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["kumar.com", "bahis.net", "bet365.com", "x.y.z.org"]));
}

#[test]
fn adblock_bicimi() {
    let _k = Kayit::bas("test_listeler", "test_adblock_bicimi");
    let s = [
        "[Adblock Plus]", "! Title: x", "||kumar.com^", "||alt.bahis.net^$important", "@@||izinli.com^", "kotu.org", "||yol.com/a^",
        "||*.joker.com^", "/regex/",
    ];
    assert_eq!(tem(&s, Bicim::Adblock), kume(&["kumar.com", "alt.bahis.net"]));
}

#[test]
fn usom_bicimi() {
    let _k = Kayit::bas("test_listeler", "test_usom_bicimi");
    // Yol ya da sorgu içeren URL atlanır (paylaşımlı sitedeki tek dosya bütün siteyi engellemesin); çıplak adlar alınır.
    let s = [
        "http://phish.example.com/login.php?x=1", "https://Kotu.COM:8443/", "zararli.net", "ftp://user@dosya.org", "http://1.2.3.4/",
        "www.banka-giris.xyz/", "# yorum", "https://github.com/kotu/repo/raw/a.exe", "https://cdn.discordapp.com/attachments/1/2/x.exe",
        "http://bit.ly/abc", "https://t.me/kanal", "yol.com/a", "sorgu.com?x=1", "sorgu2.com/?x=1", "parca.com#x", "http://kotu2.com",
    ];
    assert_eq!(tem(&s, Bicim::Usom), kume(&["kotu.com", "zararli.net", "dosya.org", "www.banka-giris.xyz", "parca.com", "kotu2.com"]));
}

#[test]
fn paylasimli_platformlar_ve_genis_son_ekler() {
    let _k = Kayit::bas("test_listeler", "test_paylasimli_platformlar_ve_genis_son_ekler");
    let s = [
        "github.com", "raw.githubusercontent.com", "cdn.discordapp.com", "discord.com", "dropbox.com", "dl.dropbox.com", "bit.ly", "t.me",
        "ipfs.io", "mediafire.com", "pastebin.com", "drive.google.com", "docs.google.com", "github.io", "blogspot.com", "co.uk", "com.br",
        "workers.dev", "cloudfront.net", "amazonaws.com", "kotu.github.io", "sahte.blogspot.com", "d111.cloudfront.net", "kotu.co.uk",
        "phish.workers.dev",
    ];
    assert_eq!(
        tem(&s, Bicim::Duz),
        kume(&["kotu.github.io", "sahte.blogspot.com", "d111.cloudfront.net", "kotu.co.uk", "phish.workers.dev"])
    );
}

#[test]
fn baglanti_denetimi_ve_ntp_asla() {
    let _k = Kayit::bas("test_listeler", "test_baglanti_denetimi_ve_ntp_asla");
    let s = [
        "detectportal.firefox.com", "connectivitycheck.gstatic.com", "captive.apple.com", "www.msftconnecttest.com", "pool.ntp.org",
        "tr.pool.ntp.org", "firefox.com",
    ];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["firefox.com"]));
}

#[test]
fn html_denetimi_bom_ve_doctype() {
    let _k = Kayit::bas("test_listeler", "test_html_denetimi_bom_ve_dogtype");
    assert!(html_mi("\u{feff}<!DOCTYPE html><html></html>"));
    assert!(html_mi("\u{feff}\n  <HTML lang=tr>"));
    assert!(html_mi(&format!("# baslik\n{}\n<!doctype html>", "x".repeat(100))));
    assert!(html_mi("kotu.com\n<html>"));
    assert!(!html_mi("\u{feff}kotu.com\nbaska.com\n"));
    assert!(!html_mi(&format!("{}<html>", "a.com\n".repeat(2000)))); // ilk 4 KB'ın dışında
}

#[test]
fn gecersizler_ip_tek_etiket_kamu_son_eki_atilir() {
    let _k = Kayit::bas("test_listeler", "test_gecersizler_ip_tek_etiket_kamu_son_eki_atilir");
    let uzun_etiket = format!("{}.com", "a".repeat(64));
    let uzun_ad = vec!["a".repeat(60); 5].join(".");
    let s = [
        "1.2.3.4", "10.0.0.1", "tekad", "com", "com.tr", "gov.tr", "k12.tr", "net", "a..b.com", "alt_cizgi.com", "ü.com", &uzun_etiket,
        &uzun_ad, "-", "*.com", "iyi.com", "iyi.bel.tr",
    ];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["iyi.com", "iyi.bel.tr"]));
}

#[test]
fn asla_engelleme_ve_alt_adlari_cikar() {
    let _k = Kayit::bas("test_listeler", "test_asla_engelleme_ve_alt_adlari_cikar");
    let s = [
        "wificorrect.com", "api.wificorrect.com", "netgsm.com.tr", "verify.twilio.com", "www.google.com", "connectivitycheck.gstatic.com",
        "e-devlet.gov.tr", "odtu.edu.tr", "captive.apple.com", "notwificorrect.com", "google.com.kotu.net", "msftconnecttest.com",
    ];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["notwificorrect.com", "google.com.kotu.net"]));
}

#[test]
fn buyuk_ortak_platformlar_asla() {
    let _k = Kayit::bas("test_listeler", "test_buyuk_ortak_platformlar_asla");
    // Bir alt adı bile engellenirse milyonlarca meşru kullanıcıyı etkileyen ortak altyapılar ve alt adları.
    let s = [
        "googleusercontent.com", "lh3.googleusercontent.com", "x.azureedge.net", "blob.core.windows.net", "cloudflare.com",
        "abc.trycloudflare.com", "facebook.com", "m.facebook.com", "scontent.fbcdn.net", "instagram.com", "www.youtube.com", "i.ytimg.com",
        "x.akamaihd.net", "login.live.com", "office.com", "outlook.com", "web.whatsapp.com", "mmg.whatsapp.net", "wa.me", "kotu-facebook.com",
        "facebook.com.kotu.net",
    ];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["kotu-facebook.com", "facebook.com.kotu.net"]));
}

#[test]
fn telefon_sosyal_alisveris_banka_asla() {
    let _k = Kayit::bas("test_listeler", "test_telefon_sosyal_alisveris_banka_asla");
    // Telefon üreticileri, sosyal medya, alışveriş, ödeme ve bankalar (ve alt adları) hiçbir listeden gelmez.
    let asla = [
        "hicloud.com", "miui.com", "xiaomi.com", "oppomobile.com", "samsung.com", "samsungcloud.com", "x.com", "twitter.com", "tiktok.com",
        "amazon.com", "trendyol.com", "hepsiburada.com", "papara.com", "iyzico.com", "garantibbva.com.tr", "isbank.com.tr", "ziraatbank.com.tr",
        "akbank.com", "yapikredi.com.tr", "vakifbank.com.tr", "halkbank.com.tr", "qnb.com.tr", "denizbank.com", "enpara.com", "turkiye.gov.tr",
    ];
    let mut s: Vec<String> = asla.iter().map(|a| a.to_string()).collect();
    s.extend(asla.iter().map(|a| format!("giris.{a}")));
    s.extend(["akbank-giris.com".to_string(), "x.com.kotu.net".to_string()]);
    let s: Vec<&str> = s.iter().map(String::as_str).collect();
    assert_eq!(tem(&s, Bicim::Duz), kume(&["akbank-giris.com", "x.com.kotu.net"]));
}

#[test]
fn onbellek_okurken_yeniden_dogrulanir() {
    let _k = Kayit::bas("test_listeler", "test_onbellek_okurken_yeniden_dogrulanir");
    // Eski sürümün yazdığı önbellekte artık asla listesinde olan ya da geçersiz ad varsa listeye girmez.
    let tmp = Y::gecici_dizin();
    let yol = onbellek_yolu(&tmp, K, "https://a/liste");
    atomik(&yol, &gz("a.com\nm.facebook.com\n1.2.3.4\nwa.me\nb.com\n")).unwrap();
    assert_eq!(onbellek_oku(&yol), Some(kume(&["a.com", "b.com"])));
    let t = sahte(&[("https://a/liste", hata("ag yok")), ("https://b/liste", ok("||c.com^\n"))]);
    iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    assert_eq!(gz_oku(&tmp.join("kumar.txt.gz")), "a.com\nb.com\nc.com\n");
}

#[test]
fn vpn_doh_alan_adlari_cikar() {
    let _k = Kayit::bas("test_listeler", "test_vpn_doh_alan_adlari_cikar");
    let s = [
        "nordvpn.com", "api.nordvpn.com", "mullvad.net", "dns.google", "cloudflare-dns.com", "doh.example.org", "vpn.sirket.com.tr",
        "dns.quad9.net", "kotu-site.com", "phishing-vpnlogin.com",
    ];
    assert_eq!(tem(&s, Bicim::Duz), kume(&["kotu-site.com", "phishing-vpnlogin.com"]));
}

// --- üretim ---
fn tmp_dosyalari(tmp: &Path, bitis: &str) -> Vec<String> {
    std::fs::read_dir(tmp).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|a| a.ends_with(bitis)).collect()
}

#[test]
fn uret_yazar_meta_ve_sha() {
    let _k = Kayit::bas("test_listeler", "test_uret_yazar_meta_ve_sha");
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", ok("b.com\na.com\nalt.a.com\n")), ("https://b/liste", ok("||c.com^\n||a.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    let metin = gz_oku(&tmp.join("kumar.txt.gz"));
    assert_eq!(metin, "a.com\nb.com\nc.com\n"); // sıralı; alt.a.com üst ad engellendiği için gereksiz
    assert_eq!(meta["sha256"], hex(&Sha256::digest(metin.as_bytes())).as_str());
    assert!(meta["sayi"] == 3 && meta["kategori"] == K && meta["kesildi"] == false);
    assert!(meta["zaman"].as_str().unwrap().starts_with("2026-09-2") && !meta.contains_key("hata"));
    assert_eq!(meta["kaynaklar"], json!([{"ad": "A", "sayi": 3}, {"ad": "B", "sayi": 2}]));
    let dosya: Value = serde_json::from_slice(&std::fs::read(tmp.join("kumar.json")).unwrap()).unwrap();
    assert_eq!(dosya, Value::Object(meta));
    assert!(tmp_dosyalari(&tmp, ".tmp").is_empty());
}

#[test]
fn ayni_girdi_ayni_sha_ve_ayni_gzip() {
    let _k = Kayit::bas("test_listeler", "test_ayni_girdi_ayni_sha_ve_ayni_gzip");
    let (t1, t2) = (Y::gecici_dizin(), Y::gecici_dizin());
    let f = sahte(&[("https://a/liste", ok("b.com\na.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let m1 = iki_kaynak().uret(K, &f, &t1, SIMDI).unwrap();
    let f2 = sahte(&[("https://a/liste", ok("a.com\nb.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let m2 = iki_kaynak().uret(K, &f2, &t2, SIMDI + 500.0).unwrap();
    assert_eq!(m1["sha256"], m2["sha256"]);
    assert_eq!(std::fs::read(t1.join("kumar.txt.gz")).unwrap(), std::fs::read(t2.join("kumar.txt.gz")).unwrap());
}

#[test]
fn sinir_ve_kesildi() {
    let _k = Kayit::bas("test_listeler", "test_sinir_ve_kesildi");
    let tmp = Y::gecici_dizin();
    let a: String = (0..4).map(|i| format!("a{i}.com\n")).collect();
    let b: String = (0..4).map(|i| format!("||b{i}.com^\n")).collect();
    let t = sahte(&[("https://a/liste", ok(&a)), ("https://b/liste", ok(&b))]);
    let mut u = iki_kaynak();
    u.en_cok = 5;
    let meta = u.uret(K, &t, &tmp, SIMDI).unwrap();
    assert!(meta["sayi"] == 5 && meta["kesildi"] == true);
    let alanlar: BTreeSet<String> = gz_oku(&tmp.join("kumar.txt.gz")).split_whitespace().map(String::from).collect();
    assert!((0..4).all(|i| alanlar.contains(&format!("a{i}.com")))); // önce gelen kaynak öncelikli
}

#[test]
fn varsayilan_sinir_300000() {
    let _k = Kayit::bas("test_listeler", "test_varsayilan_sinir_300000");
    assert_eq!(EN_COK, 300_000);
}

#[test]
fn bir_kaynak_hata_digerleriyle_uretilir() {
    let _k = Kayit::bas("test_listeler", "test_bir_kaynak_hata_digerleriyle_uretilir");
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", hata("ag yok")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    assert!(meta["sayi"] == 1 && !meta.contains_key("hata"));
    assert!(meta["kaynaklar"][0]["ad"] == "A" && meta["kaynaklar"][0]["hata"].as_str().unwrap().contains("ag yok"));
    assert_eq!(meta["kaynaklar"][1], json!({"ad": "B", "sayi": 1}));
}

#[test]
fn hatali_kaynak_onceki_basarili_kopyasiyla_devam() {
    let _k = Kayit::bas("test_listeler", "test_hatali_kaynak_onceki_basarili_kopyasiyla_devam");
    // Bir kaynak bugün inmezse (ya da HTML/boş dönerse) dünkü temiz kopyası kullanılır: liste aniden küçülmez.
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", ok("a.com\nb.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    for bozuk in [hata("ag yok"), ok("<!DOCTYPE html><html>http://x.com/</html>"), ok("# bos\n")] {
        let t = sahte(&[("https://a/liste", bozuk.clone()), ("https://b/liste", ok("||d.com^\n"))]);
        let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI + 10.0).unwrap();
        assert_eq!(gz_oku(&tmp.join("kumar.txt.gz")), "a.com\nb.com\nd.com\n", "{bozuk:?}");
        let d = &meta["kaynaklar"][0];
        assert!(d["hata"].as_str().is_some_and(|h| !h.is_empty()) && d["sayi"] == 2 && d["eski"] == true, "{bozuk:?}");
    }
}

#[test]
fn hepsi_hata_eski_dosya_korunur() {
    let _k = Kayit::bas("test_listeler", "test_hepsi_hata_eski_dosya_korunur");
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", ok("a.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let iyi = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    let gz1 = std::fs::read(tmp.join("kumar.txt.gz")).unwrap();
    let t = sahte(&[("https://a/liste", hata("x")), ("https://b/liste", hata("y"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI + 100.0).unwrap();
    assert_eq!(std::fs::read(tmp.join("kumar.txt.gz")).unwrap(), gz1);
    assert!(meta.contains_key("hata") && meta["sha256"] == iyi["sha256"] && meta["sayi"] == iyi["sayi"] && meta["zaman"] == iyi["zaman"]);
    let dosya: Value = serde_json::from_slice(&std::fs::read(tmp.join("kumar.json")).unwrap()).unwrap();
    assert!(dosya.get("hata").is_some());
    assert_eq!(ozet(&tmp)[K], json!({"sha256": iyi["sha256"], "sayi": 2, "zaman": iyi["zaman"]}));
    // sonra düzelince hata kalkar
    let t = sahte(&[("https://a/liste", ok("a.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    assert!(!iki_kaynak().uret(K, &t, &tmp, SIMDI + 200.0).unwrap().contains_key("hata"));
}

#[test]
fn hic_liste_yokken_hepsi_hata() {
    let _k = Kayit::bas("test_listeler", "test_hic_liste_yokken_hepsi_hata");
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", hata("x")), ("https://b/liste", hata("y"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    assert!(meta.contains_key("hata") && !meta.contains_key("sha256"));
    assert!(!tmp.join("kumar.txt.gz").exists() && ozet(&tmp).get(K).is_none());
}

#[test]
fn elle_ek_dosyasi() {
    let _k = Kayit::bas("test_listeler", "test_elle_ek_dosyasi");
    let tmp = Y::gecici_dizin();
    std::fs::write(tmp.join("kumar.ek.txt"), "# Türkiye eki\nyerelbahis.com\ngoogle.com\n").unwrap();
    let t = sahte(&[("https://a/liste", ok("a.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    let metin = gz_oku(&tmp.join("kumar.txt.gz"));
    assert!(metin.split_whitespace().any(|a| a == "yerelbahis.com"));
    assert!(!metin.contains("google.com"));
    assert!(meta["kaynaklar"].as_array().unwrap().contains(&json!({"ad": "Elle eklenenler", "sayi": 1})));
}

#[test]
fn kaynaklar_kategoriler_ve_vpn_listesi_yok() {
    let _k = Kayit::bas("test_listeler", "test_kaynaklar_kategoriler_ve_vpn_listesi_yok");
    let kaynaklar = varsayilan_kaynaklar();
    let a: BTreeSet<&str> = kaynaklar.iter().map(|(k, _)| *k).collect();
    let b: BTreeSet<&str> = KATEGORILER.iter().map(|(k, _)| *k).collect();
    assert_eq!(a, BTreeSet::from(["kumar", "yetiskin", "zararli"]));
    assert_eq!(a, b);
    let ad = |k: &str| KATEGORILER.iter().find(|(x, _)| *x == k).unwrap().1;
    assert!(ad("kumar") == "Kumar ve bahis" && ad("yetiskin") == "Yetişkin içerik" && ad("zararli") == "Zararlı ve dolandırıcı siteler");
    for (_, ks) in &kaynaklar {
        for k in ks {
            assert!(k.url.starts_with("https://"));
            assert!(!k.url.contains("doh-vpn-proxy-bypass") && !k.url.to_lowercase().contains("vpn"));
        }
    }
}

#[test]
fn main_uretir_ve_hareket() {
    let _k = Kayit::bas("test_listeler", "test_main_uretir_ve_hareket");
    let u = Uretici {
        kaynaklar: vec![
            ("kumar", vec![k1("A", "https://a", Bicim::Duz)]),
            ("yetiskin", vec![k1("B", "https://b", Bicim::Duz)]),
            ("zararli", vec![k1("C", "https://c", Bicim::Duz)]),
        ],
        en_cok: EN_COK,
    };
    let o = Y::ortam();
    let d = o.dizin.join("l");
    let t = sahte(&[("https://a", ok("a.com\n")), ("https://b", ok("b.com\n")), ("https://c", hata("ag yok"))]);
    assert_eq!(u.calistir(&o.veri, &t, &d, SIMDI, "listeler", "", true), Ok(1)); // biri hata
    let ol: BTreeSet<(String, String)> = o
        .veri
        .hareketler(None, 300, None)
        .unwrap()
        .into_iter()
        .map(|h| (h.olay, h.ayrinti.split(' ').next().unwrap().to_string()))
        .collect();
    let var = |olay: &str, k: &str| ol.contains(&(olay.to_string(), k.to_string()));
    assert!(var("LISTE_GUNCEL", "kategori=kumar") && var("LISTE_GUNCEL", "kategori=yetiskin"));
    assert!(var("LISTE_HATA", "kategori=zararli"));
    let n = o.veri.hareketler(None, 300, None).unwrap().len();
    let t = sahte(&[("https://a", ok("a.com\n")), ("https://b", ok("b.com\n")), ("https://c", ok("c.com\n"))]);
    assert_eq!(u.calistir(&o.veri, &t, &d, SIMDI + 10.0, "listeler", "", true), Ok(0));
    // değişmeyen kategori için yeniden LISTE_GUNCEL yazılmaz; yalnızca zararli
    let tum = o.veri.hareketler(None, 300, None).unwrap();
    let yeni: Vec<(String, String)> = tum[..tum.len() - n].iter().map(|h| (h.olay.clone(), h.ayrinti.split(' ').next().unwrap().to_string())).collect();
    assert_eq!(yeni, vec![("LISTE_GUNCEL".to_string(), "kategori=zararli".to_string())]);
}

#[test]
fn bom_ile_baslayan_html_kaynak_hata_bom_li_liste_iyi() {
    let _k = Kayit::bas("test_listeler", "test_bom_ile_baslayan_html_kaynak_hata_bom_li_liste_iyi");
    let tmp = Y::gecici_dizin();
    let t = sahte(&[("https://a/liste", ok("\u{feff}<!DOCTYPE html><p>kotu.com</p>")), ("https://b/liste", ok("\u{feff}||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    assert!(meta["kaynaklar"][0]["hata"].as_str().unwrap().contains("HTML") && meta["kaynaklar"][1] == json!({"ad": "B", "sayi": 1}));
    let t = sahte(&[("https://a/liste", ok("\u{feff}ilk.com\nikinci.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    assert_eq!(meta["kaynaklar"][0], json!({"ad": "A", "sayi": 2})); // BOM ilk satırı bozmaz
}

#[test]
fn kucuk_liste_tabani() {
    let _k = Kayit::bas("test_listeler", "test_kucuk_liste_tabani");
    // Önceki kopya > 1000 iken yeni liste yarısından küçükse hata: kopya kullanılır ve üzerine yazılmaz.
    let tmp = Y::gecici_dizin();
    let liste = |n: usize| -> String { (0..n).map(|i| format!("a{i}.com\n")).collect() };
    let t = sahte(&[("https://a/liste", ok(&liste(2000))), ("https://b/liste", ok("||c.com^\n"))]);
    iki_kaynak().uret(K, &t, &tmp, SIMDI).unwrap();
    let t = sahte(&[("https://a/liste", ok(&liste(999))), ("https://b/liste", ok("||c.com^\n"))]);
    for _ in 0..2 {
        // ikinci turda da kopya hâlâ 2000'lik (üzerine yazılmadı)
        let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI + 10.0).unwrap();
        let d = &meta["kaynaklar"][0];
        assert!(d["hata"].as_str().unwrap().contains("beklenenden çok küçük: 999 / eski 2000") && d["sayi"] == 2000 && d["eski"] == true);
        assert_eq!(meta["sayi"], 2001);
    }
    // yarısı ya da fazlası kabul edilir
    let t = sahte(&[("https://a/liste", ok(&liste(1000))), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI + 20.0).unwrap();
    assert!(meta["kaynaklar"][0] == json!({"ad": "A", "sayi": 1000}) && meta["sayi"] == 1001);
    // küçük önceki kopyada (≤ 1000) taban uygulanmaz
    let t = sahte(&[("https://a/liste", ok("z.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &tmp, SIMDI + 30.0).unwrap();
    assert_eq!(meta["kaynaklar"][0], json!({"ad": "A", "sayi": 1}));
}

#[test]
fn kaynak_hatasi_hareketi_ve_guncel() {
    let _k = Kayit::bas("test_listeler", "test_kaynak_hatasi_hareketi_ve_guncel");
    let u = Uretici {
        kaynaklar: vec![
            ("kumar", vec![k1("A", "https://a", Bicim::Duz), k1("B", "https://b", Bicim::Duz)]),
            ("yetiskin", vec![]),
            ("zararli", vec![]),
        ],
        en_cok: EN_COK,
    };
    let o = Y::ortam();
    let d = o.dizin.join("l");
    let t = sahte(&[("https://a", hata("ag yok")), ("https://b", ok("b.com\n"))]);
    assert_eq!(u.calistir(&o.veri, &t, &d, SIMDI, "listeler", "", true), Ok(1));
    let h: Vec<(String, String)> =
        o.veri.hareketler(None, 300, None).unwrap().into_iter().filter(|x| x.ayrinti.contains("kategori=kumar")).map(|x| (x.olay, x.ayrinti)).collect();
    assert!(h.contains(&("LISTE_HATA".to_string(), "kategori=kumar kaynak=A neden=ag yok".to_string())));
    assert!(h.iter().any(|(o, a)| o == "LISTE_GUNCEL" && a.starts_with("kategori=kumar surum=") && a.contains("sayi=1")));
}

// --- API ---
fn rapor() -> Value {
    json!({"surum": "1.2.3", "acik_sn": 5000, "servisler": {"portal": true, "panel": true, "kaydedici": true, "dnsmasq": true, "guvenlik": true},
           "disk_yuzde": 42, "son_muhur": "2026-10-10", "kayit_boslugu_1s": false, "wan": true, "sms_sorunu": false, "bagli": 7, "hiz_hata": false})
}

/// Python `api_kur`: (api düzeni, müşteri numarası, cihaz anahtarı). Her istek ayrı IP'den (hız sınırına takılmasın).
fn api_kur() -> (crate::api::testler::D, i64, String) {
    let d = crate::api::testler::kur(Some(crate::api::testler::tamam()));
    let (n, pw) = d.v.musteri_ekle("", None).unwrap();
    let (_, g) = crate::api::testler::post_ip(
        &d,
        "/api/giris",
        &json!({"numara": n.to_string(), "parola": pw, "wg_pub": crate::api::testler::WG, "ssh_pub": crate::api::testler::SSH}),
        "203.0.113.1",
    );
    let k = g["cihaz_anahtari"].as_str().unwrap().to_string();
    (d, n, k)
}

fn api_ip(sayac: &std::cell::Cell<u32>) -> String {
    sayac.set(sayac.get() + 1);
    format!("203.0.113.{}", 1 + sayac.get() % 250)
}

#[test]
fn saglik_cevabinda_listeler() {
    let _k = Kayit::bas("test_listeler", "test_saglik_cevabinda_listeler");
    let (d, _n, k) = api_kur();
    let sayac = std::cell::Cell::new(1);
    let saglik = |k: &str| crate::api::testler::post_ip(&d, "/api/saglik", &json!({"cihaz_anahtari": k, "rapor": rapor()}), &api_ip(&sayac));
    let (durum, g) = saglik(&k);
    assert!(durum == 200 && g["listeler"] == json!({}));
    let dizin = d.tmp.join("listeler");
    let t = sahte(&[("https://a/liste", ok("a.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &dizin, SIMDI).unwrap();
    std::fs::write(dizin.join("yetiskin.json"), "{bozuk").unwrap();
    let (_, j) = saglik(&k);
    assert_eq!(j["listeler"], json!({K: {"sha256": meta["sha256"], "sayi": 2, "zaman": meta["zaman"]}}));
}

#[test]
fn api_liste_200_400_403_404() {
    let _k = Kayit::bas("test_listeler", "test_api_liste_200_400_403_404");
    let (d, n, k) = api_kur();
    let sayac = std::cell::Cell::new(1);
    let liste = |kategori: Value, anahtar: &str| {
        Y::istek(&d.api, "POST", "/api/liste", &[], &Y::json_govde(&json!({"cihaz_anahtari": anahtar, "kategori": kategori})), "", &api_ip(&sayac))
    };
    assert_eq!(liste(json!(K), &k).durum, 404); // henüz üretilmedi
    let dizin = d.tmp.join("listeler");
    let t = sahte(&[("https://a/liste", ok("a.com\n")), ("https://b/liste", ok("||c.com^\n"))]);
    let meta = iki_kaynak().uret(K, &t, &dizin, SIMDI).unwrap();
    let y = liste(json!(K), &k);
    assert!(y.durum == 200 && y.baslik("Content-Type") == Some("application/gzip") && y.baslik("Cache-Control") == Some("no-store"));
    let mut metin = Vec::new();
    flate2::read::GzDecoder::new(&y.govde[..]).read_to_end(&mut metin).unwrap();
    assert_eq!(meta["sha256"], hex(&Sha256::digest(&metin)).as_str());
    for kotu in [json!("vpn"), json!("../kumar"), json!(""), json!(5), Value::Null] {
        assert_eq!(liste(kotu.clone(), &k).durum, 400, "{kotu}");
    }
    assert_eq!(liste(json!(K), "yanlis").durum, 401);
    d.v.uyelik(n, "bitti").unwrap();
    assert_eq!(liste(json!(K), &k).durum, 403);
    d.v.uyelik(n, "aktif").unwrap();
    assert_eq!(liste(json!(K), &k).durum, 200);
    std::fs::remove_file(dizin.join("kumar.txt.gz")).unwrap();
    assert_eq!(liste(json!(K), &k).durum, 404);
}

#[test]
fn mesgul_kilidi_beklemeden_doner() {
    // Python `test_simdi_guncelle_mesgulse_beklemez`'in kilit tarafı (Yonetim sayfası Task 11'de altınla): bekle=false iken
    // kilit tutuluyorsa Mesgul, bekle=true iken sıra gelince alınır.
    let tmp = Y::gecici_dizin();
    let birinci = Kilit::al(&tmp, false).unwrap();
    assert_eq!(Kilit::al(&tmp, false).err(), Some(Mesgul));
    let o = Y::ortam_dizinde(&Y::gecici_dizin());
    assert_eq!(Uretici::varsayilan().calistir(&o.veri, &Y::AgYok, &tmp, SIMDI, "listeler", "", false), Err(Mesgul));
    drop(birinci);
    assert!(Kilit::al(&tmp, false).is_ok());
}
