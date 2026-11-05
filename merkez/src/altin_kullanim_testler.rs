//! Altın kayıt kullanım denetimi: `tests/altin/**` altındaki her boş olmayan kayıt (cevap, veritabanı ya da dosya içeren)
//! `merkez/src` içinde bir `Kayit::bas` çağrısıyla kullanılmalıdır; kullanılmayan kayıt, taşınmamış bir Python testi demektir.
//! Kasıtlı olarak taşınmayanlar aşağıdaki izin listesindedir ve nedeniyle yazılır. İzin listesindeki bir kayıt kullanılmaya
//! başlarsa test düşer (liste eskimesin).
use std::collections::BTreeSet;
use std::path::Path;

use super::kayit_oku_dizinden;

/// (Python test dosyası, test, neden): boş olmayan ama bilerek taşınmayan kayıtlar.
const IZIN_LISTESI: [(&str, &str, &str); 13] = [
    // spec bilinçli fark 1: /api/raporlar ve /api/rapor yok (404; yerine kayıtsız api_rapor_uclari_yok_404)
    ("test_api", "test_api_raporlar_liste_ve_donem", "bilinçli fark 1: rapor uçları kaldırıldı"),
    ("test_api", "test_api_rapor_dogrulama_ve_yetki", "bilinçli fark 1: rapor uçları kaldırıldı"),
    ("test_api", "test_rapor_hiz_siniri_sms_butcesinden_ayri", "bilinçli fark 1: rapor uçları kaldırıldı"),
    ("test_api", "test_api_rapor_liste_ya_da_sozluk_tur_400", "bilinçli fark 1: rapor uçları kaldırıldı"),
    // spec bilinçli fark 2: eşleşme cevabında sms yalnızca etkin_mock taşır (Rust testleri Kayit::altinsiz)
    ("test_api", "test_sms_ayari_yoksa_yalnizca_etkin_mock_varsa_gider", "bilinçli fark 2: Rust testi altınsız"),
    ("test_bekleme", "test_bagla_cevabi_ortak_sms_sirlarini_tasimaz", "bilinçli fark 2: Rust testi altınsız"),
    ("test_sms_merkez", "test_eski_cihaz_eki_degismez", "bilinçli fark 2: Rust testi altınsız"),
    ("test_sms_merkez", "test_etkin_deneme_modu_eki", "bilinçli fark 2: Rust testi altınsız"),
    // spec bilinçli fark 6: biçim 1 rapor/ham özet dosyaları okunmaz ve temizlenmez
    ("test_rapor", "test_listeden_cikan_arsivin_eski_bicim_hamlari_silinir", "bilinçli fark 6: biçim 1 dosyalar yok sayılır"),
    // Python `_gocur` ve ALTER göçleri taşınmadı (Task 2 kararı: eksik sütun `sema_denetle` hatası; `veritabani-denetle` sınar)
    ("test_veri", "test_eski_veritabani_gocu", "ALTER göçü taşınmadı (Task 2 kararı)"),
    ("test_veri", "test_eski_cihaz_tablosuna_admin_sutunlari_eklenir", "ALTER göçü taşınmadı (Task 2 kararı)"),
    ("test_misafir_kodu", "test_eski_veritabanina_tur_sutunu_eklenir", "ALTER göçü taşınmadı (Task 2 kararı)"),
    // logonun tek kopyaya inmesiyle gereksizleşen 5 logo testinden (spec "Kayıp olmaması için güvenceler" 1)
    ("test_logo", "test_dosya_yoksa_yaziya_duser", "logo tek kopya: cihaz/merkez kopya testleri gereksiz"),
];

/// Kayıtları boştur (cevap, veritabanı, dosya yok: kullanım denetimi bunları zaten istemez) ve 13b'de taşınacak kur.sh/birim testleri; taşınınca `Kayit::bas` ile kullanılır
/// ve bu liste (ile test) düşer.
const DEVREDILEN_13B: [(&str, &str); 4] = [
    ("test_rapor", "test_kur_sh_ve_merkez_birimi"),
    ("test_surumler", "test_kur_sh_ve_saatlik_zamanlayici"),
    ("test_surumler", "test_kur_sh_yerel_modulleri_kuruyor"),
    ("test_listeler", "test_kur_sh_ve_birimler"),
];

/// `merkez/src/*.rs` içindeki `Kayit::bas("<dosya>", "<test>")` çağrıları (yorum satırları sayılmaz; bu denetimin kendi
/// dosyası ve `altin.rs` — kendi düzenek testleri — taranmaz).
fn kullanilanlar() -> BTreeSet<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = BTreeSet::new();
    for x in std::fs::read_dir(&src).unwrap().flatten() {
        let ad = x.file_name().to_string_lossy().into_owned();
        if !ad.ends_with(".rs") || ad == "altin.rs" || ad == "altin_kullanim_testler.rs" {
            continue;
        }
        let metin: String = std::fs::read_to_string(x.path())
            .unwrap()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let cagri = ["Kayit", "::", "bas", "("].concat();
        let mut kalan = metin.as_str();
        while let Some(i) = kalan.find(&cagri) {
            kalan = &kalan[i + cagri.len()..];
            if let Some((d, t)) = iki_metin(kalan) {
                out.insert((d, t));
            }
        }
    }
    out
}

/// `"a", "b"` biçiminde iki kaçışsız metin sabiti (aralarında boşluk/satır sonu olabilir).
fn iki_metin(s: &str) -> Option<(String, String)> {
    fn metin(s: &str) -> Option<(&str, &str)> {
        let s = s.trim_start().strip_prefix('"')?;
        let k = s.find('"')?;
        Some((&s[..k], &s[k + 1..]))
    }
    let (a, s) = metin(s)?;
    let s = s.trim_start().strip_prefix(',')?;
    let (b, _) = metin(s)?;
    Some((a.to_string(), b.to_string()))
}

/// (dosya, test, boş mu) — `tests/altin/<dosya>/<test>.json[.gz]`.
fn kayitlar() -> Vec<(String, String, bool)> {
    let altin = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/altin");
    let mut out = Vec::new();
    for d in std::fs::read_dir(&altin).unwrap().flatten() {
        if !d.path().is_dir() {
            continue;
        }
        let dosya = d.file_name().to_string_lossy().into_owned();
        for k in std::fs::read_dir(d.path()).unwrap().flatten() {
            let ad = k.file_name().to_string_lossy().into_owned();
            let Some(test) = ad.strip_suffix(".json.gz").or_else(|| ad.strip_suffix(".json")) else { continue };
            let v = kayit_oku_dizinden(&altin, &dosya, test).unwrap();
            let dolu = ["cevaplar", "veritabanlari", "dosyalar"].iter().any(|a| v[*a].as_array().is_some_and(|l| !l.is_empty()));
            out.push((dosya.clone(), test.to_string(), !dolu));
        }
    }
    out.sort();
    out
}

#[test]
fn bos_olmayan_her_altin_kayit_kullanilir() {
    let kullanilan = kullanilanlar();
    assert!(kullanilan.len() > 300, "tarama bozuk: yalnızca {} Kayit::bas bulundu", kullanilan.len());
    let izinli: BTreeSet<(&str, &str)> = IZIN_LISTESI.iter().map(|(d, t, _)| (*d, *t)).collect();
    let mut kullanilmayan = Vec::new();
    for (d, t, bos) in kayitlar() {
        let kullanildi = kullanilan.contains(&(d.clone(), t.clone()));
        if !bos && !kullanildi && !izinli.contains(&(d.as_str(), t.as_str())) {
            kullanilmayan.push(format!("{d}/{t}"));
        }
    }
    assert!(
        kullanilmayan.is_empty(),
        "Kayit::bas ile kullanılmayan boş olmayan altın kayıtlar (testi taşıyın ya da nedeniyle izin listesine yazın): {kullanilmayan:?}"
    );
}

#[test]
fn izin_listesi_guncel() {
    let kullanilan = kullanilanlar();
    let kayit: BTreeSet<(String, String)> = kayitlar().into_iter().map(|(d, t, _)| (d, t)).collect();
    let girdiler = IZIN_LISTESI.iter().map(|(d, t, _)| (*d, *t)).chain(DEVREDILEN_13B);
    for (d, t) in girdiler {
        let anahtar = (d.to_string(), t.to_string());
        assert!(kayit.contains(&anahtar), "izin listesinde ama altın kaydı yok: {d}/{t}");
        assert!(!kullanilan.contains(&anahtar), "izin listesinde ama Kayit::bas ile kullanılıyor, listeden çıkarın: {d}/{t}");
    }
    for (_, _, neden) in IZIN_LISTESI {
        assert!(!neden.trim().is_empty());
    }
}
