//! denetle.rs testleri: Python'un yazdığı veritabanı (`tests/veri/python-gocmus.db`) `TAMAM` verir; kasten bozulmuş kopyalar
//! adıyla `HATA` verir ve hiçbir değer basılmaz; `ornek_agac` Rust'ın yazdığı bütün dosya türlerinden bir ağaç üretir
//! (scripts/sunucu/parite/geri_oku.py bunu Python okuyucularıyla açar: geri dönüşün ön koşulu).
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::*;
use crate::test_yardim::{gecici_dizin, SahteSaat};

const GIZLI: &str = "GIZLI-DEGER-9f3a";

fn gocmus_db() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/veri/python-gocmus.db")
}

fn sha(yol: &Path) -> String {
    Sha256::digest(fs::read(yol).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}

/// (başarılı mı, çıktı metni).
fn calistir(vt: &Path, kok: Option<&Path>) -> (bool, String) {
    let mut cikti = Vec::new();
    let tamam = denetle(vt, kok, &mut cikti);
    (tamam, String::from_utf8(cikti).unwrap())
}

/// Python'un yazdığı veritabanının değiştirilebilir kopyası.
fn kopya() -> (PathBuf, Connection) {
    let vt = gecici_dizin().join("kopya.db");
    fs::copy(gocmus_db(), &vt).unwrap();
    let c = Connection::open(&vt).unwrap();
    (vt, c)
}

fn hata_satirlari(cikti: &str) -> Vec<&str> {
    cikti.lines().filter(|l| l.starts_with("HATA ")).collect()
}

#[test]
fn python_gocmus_veritabani_tamam_ve_dokunulmaz() {
    let once = sha(&gocmus_db());
    let (tamam, cikti) = calistir(&gocmus_db(), None);
    assert!(tamam, "{cikti}");
    assert_eq!(cikti.lines().last(), Some("TAMAM"));
    assert!(hata_satirlari(&cikti).is_empty());
    // her TABLO satırındaki sayı gerçek satır sayısı
    let c = Connection::open(gocmus_db()).unwrap();
    let mut gorulen = 0;
    for l in cikti.lines().filter(|l| l.starts_with("TABLO ")) {
        let p: Vec<&str> = l.split(' ').collect();
        let n: i64 = c.query_row(&format!("SELECT COUNT(*) FROM {}", p[1]), [], |r| r.get(0)).unwrap();
        assert_eq!(p[2], n.to_string(), "{l}");
        gorulen += 1;
    }
    assert_eq!(gorulen, 11);
    assert_eq!(sha(&gocmus_db()), once, "denetim veritabanına dokunmamalı");
}

#[test]
fn depolama_sinifi_uyusmazligi_hatadir_ve_deger_basilmaz() {
    let (vt, c) = kopya();
    let numara: i64 = c.query_row("SELECT numara FROM musteri LIMIT 1", [], |r| r.get(0)).unwrap();
    // INTEGER sütunda metin, REAL sütunda metin
    c.execute("UPDATE musteri SET yineleme = ?, guncelleme_hemen = ? WHERE numara = ?", rusqlite::params![GIZLI, GIZLI, numara]).unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(!tamam);
    assert!(!cikti.contains(GIZLI) && !cikti.contains("TAMAM"), "{cikti}");
    let h = hata_satirlari(&cikti);
    assert_eq!(h.len(), 1, "{cikti}");
    assert!(h[0].starts_with(&format!("HATA musteri {numara} ")), "{}", h[0]);
    assert!(h[0].contains("Invalid column type"), "{}", h[0]);
    // satır sayısı yine bildirilir
    assert!(cikti.contains("TABLO musteri "));
}

#[test]
fn python_goc_duzeltmeleri_tasinmadigi_icin_bos_alanlar_hatadir() {
    let (vt, c) = kopya();
    c.execute("UPDATE musteri SET lisans_bitis = '' WHERE numara = (SELECT numara FROM musteri LIMIT 1)", []).unwrap();
    c.execute("UPDATE musteri SET ap_parola = '' WHERE numara = (SELECT numara FROM musteri ORDER BY numara DESC LIMIT 1)", []).unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(!tamam);
    let h = hata_satirlari(&cikti);
    assert_eq!(h.len(), 2, "{cikti}");
    assert!(h.iter().any(|l| l.contains("lisans_bitis bos")) && h.iter().any(|l| l.contains("ap_parola bos")), "{cikti}");
}

#[test]
fn json_sutunlari_ayristirilir_hata_degeri_basmaz() {
    let (vt, c) = kopya();
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('sms', ?)", [format!("[1, {GIZLI}")]).unwrap();
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('uyari', '[1]')", []).unwrap();
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('admin', '{\"tuz\": \"a\"}')", []).unwrap();
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('sms_sayac:2026-09-21', 'x')", []).unwrap();
    // düz metin anahtarları serbest
    c.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('kanal_test', 'en-son')", []).unwrap();
    let cid: i64 = c.query_row("SELECT id FROM cihaz LIMIT 1", [], |r| r.get(0)).unwrap();
    c.execute("INSERT OR REPLACE INTO saglik (cihaz, zaman, rapor) VALUES (?, 'z', ?)", rusqlite::params![cid, format!("{{\"a\": {GIZLI}")]).unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(!tamam);
    assert!(!cikti.contains(GIZLI), "{cikti}");
    let h = hata_satirlari(&cikti);
    for beklenen in ["HATA ayar sms ", "HATA ayar uyari ", "HATA ayar admin ", "HATA ayar sms_sayac:2026-09-21 ", &format!("HATA saglik {cid} ")] {
        assert!(h.iter().any(|l| l.starts_with(beklenen)), "{beklenen}: {cikti}");
    }
    assert_eq!(h.len(), 5, "{cikti}");
}

#[test]
fn bos_olabilen_sutunlar_null_kabul_edilir() {
    let (vt, c) = kopya();
    c.execute("UPDATE hareket SET musteri = NULL", []).unwrap();
    c.execute("INSERT OR REPLACE INTO eski_arsiv (ad, musteri) VALUES ('yetim', NULL)", []).unwrap();
    c.execute(
        "INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, wg_pub, ssh_pub, musteri, cihaz) VALUES ('k1234567890', 'o', 1, 2, '1.2.3.4', 'w', 's', NULL, NULL)",
        [],
    )
    .unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(tamam, "{cikti}");
    assert!(cikti.contains("TABLO bekleyen "));
}

#[test]
fn bekleyen_kimligi_kisaltilir_sorunlu_satir_adla_bildirilir() {
    let (vt, c) = kopya();
    c.execute(
        "INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, wg_pub, ssh_pub) VALUES ('abcdef0123456789xyz', 'o', ?, 2, '1.2.3.4', 'w', 's')",
        [GIZLI],
    )
    .unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(!tamam);
    let h = hata_satirlari(&cikti);
    assert_eq!(h.len(), 1, "{cikti}");
    assert!(h[0].starts_with("HATA bekleyen abcdef01 "), "{}", h[0]);
    assert!(!cikti.contains(GIZLI));
}

#[test]
fn eksik_sutun_ve_tablo_sema_hatasidir() {
    let (vt, c) = kopya();
    c.execute("ALTER TABLE musteri DROP COLUMN ap_parola", []).unwrap();
    let (tamam, cikti) = calistir(&vt, None);
    assert!(!tamam);
    assert!(cikti.contains("HATA sema - ") && cikti.contains("sütun yok: musteri.ap_parola"), "{cikti}");
    assert!(!cikti.contains("TABLO "), "şema hatasında satırlara geçilmez");
    c.execute("DROP TABLE sms_son", []).unwrap();
    let (_, cikti) = calistir(&vt, None);
    assert!(cikti.contains("tablo yok: sms_son"), "{cikti}");
}

#[test]
fn bozuk_ya_da_olmayan_dosya() {
    let (tamam, cikti) = calistir(&gecici_dizin().join("yok.db"), None);
    assert!(!tamam && cikti.starts_with("HATA veritabani - acilamadi"), "{cikti}");
    // SQLite dosyası değil
    let d = gecici_dizin().join("bozuk.db");
    fs::write(&d, format!("bu bir veritabani degil {GIZLI}").repeat(50)).unwrap();
    let (tamam, cikti) = calistir(&d, None);
    assert!(!tamam && cikti.contains("HATA veritabani") && !cikti.contains(GIZLI), "{cikti}");
}

#[test]
fn denetimin_acicisi_salt_okunur() {
    let (vt, _c) = kopya();
    // izin bitleri root'u durdurmaz: denetim bağlantısının kendisi sınanır
    let c = ac(&vt).unwrap();
    assert!(c.is_readonly("main").unwrap());
    let e = c.execute("DELETE FROM hareket", []).unwrap_err();
    assert_eq!(e.sqlite_error_code(), Some(rusqlite::ErrorCode::ReadOnly), "{e}");
    // denetim yeni dosya (günlük, wal) bırakmaz, dosyaya dokunmaz
    let dizin = vt.parent().unwrap().to_path_buf();
    let once: Vec<_> = fs::read_dir(&dizin).unwrap().map(|e| e.unwrap().file_name()).collect();
    let sha_once = sha(&vt);
    let (tamam, _) = calistir(&vt, None);
    assert!(tamam);
    let sonra: Vec<_> = fs::read_dir(&dizin).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(once.len(), sonra.len());
    assert_eq!(sha(&vt), sha_once);
}

/// Yazılamayan çıktı: `TAMAM` okunamadıysa başarı sayılmaz.
struct Bozuk {
    flush_hatasi: bool,
}

impl std::io::Write for Bozuk {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.flush_hatasi {
            Ok(b.len())
        } else {
            Err(std::io::Error::other("yazilamadi"))
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_hatasi {
            Err(std::io::Error::other("flush"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn cikti_yazilamazsa_basarisiz() {
    assert!(!denetle(&gocmus_db(), None, &mut Bozuk { flush_hatasi: false }));
    assert!(!denetle(&gocmus_db(), None, &mut Bozuk { flush_hatasi: true }));
}

#[test]
fn komut_satiri() {
    let a = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(main_uretim(&a(&[])), 2);
    assert_eq!(main_uretim(&a(&["a.db", "b.db"])), 2);
    assert_eq!(main_uretim(&a(&["a.db", "--dosyalar"])), 2);
    assert_eq!(main_uretim(&a(&["a.db", "--bilinmeyen"])), 2);
    let vt = gocmus_db().to_string_lossy().into_owned();
    assert_eq!(main_uretim(&a(&[&vt])), 0);
    assert_eq!(main_uretim(&a(&["/yok/yok.db"])), 1);
}

// --- dosyalar ---

fn yaz(kok: &Path, yol: &str, icerik: &str) {
    let p = kok.join(yol);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, icerik).unwrap();
}

const RAPOR_V2: &str = r#"{"bicim": 2, "tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20"}"#;

#[test]
fn dosya_turleri_iyi_ve_bozuk() {
    let kok = gecici_dizin().join("kok");
    yaz(&kok, "durum.json", r#"{"zaman": "x", "arsivler": {}}"#);
    yaz(&kok, "istatistik/1234567.json", r#"{"2026-09-14": 2}"#);
    yaz(&kok, "detay/1234567/2026-09-14.json", r#"{"gun": "2026-09-14", "kisiler": []}"#);
    yaz(&kok, "detay/1234567/kisiler.json", "{}");
    yaz(&kok, "rapor/1234567/liste.json", r#"[{"tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20"}]"#);
    yaz(&kok, "rapor/1234567/hafta-2026-W38.json", RAPOR_V2);
    yaz(&kok, "merkez/rapor-gruplari.json", r#"{"1234567": ["bocafe"]}"#);
    yaz(&kok, "kuyruk-sonuc/0123456789abcdef.json", r#"{"durum": "tamam"}"#);
    let (tamam, cikti) = calistir(&gocmus_db(), Some(&kok));
    assert!(tamam, "{cikti}");
    for tur in ["durum", "istatistik", "detay", "rapor", "merkez", "kuyruk-sonuc"] {
        assert!(cikti.lines().any(|l| l.starts_with(&format!("DOSYA {tur} "))), "{tur}: {cikti}");
    }
    assert!(cikti.contains("DOSYA detay 2\n") && cikti.contains("DOSYA rapor 2\n"), "{cikti}");

    // şimdi bozuklar
    yaz(&kok, "detay/1234567/2026-09-15.json", &format!("{{\"gun\": \"{GIZLI}"));
    yaz(&kok, "rapor/1234567/ay-2026-09.json", r#"{"bicim": 1, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30"}"#);
    yaz(&kok, "rapor/1234567/hafta-2026-W99.json", RAPOR_V2);
    yaz(&kok, "rapor-ham/1234567/2026-09-14.json", r#"{"bicim": 1, "imza": "x"}"#);
    yaz(&kok, "istatistik/bozuk.json", r#"{"2026-09-14": "iki"}"#);
    yaz(&kok, "merkez/yonetici.json", r#"{"kullanici": "ornek"}"#);
    yaz(&kok, "merkez/listeler/kumar.json", "[]");
    yaz(&kok, "kuyruk/0123456789abcdef.json", "[]");
    yaz(&kok, "rapor/1234567/liste.json", r#"[{"tur": "hafta"}]"#);
    let (tamam, cikti) = calistir(&gocmus_db(), Some(&kok));
    assert!(!tamam && !cikti.contains(GIZLI) && !cikti.contains("TAMAM"), "{cikti}");
    let h = hata_satirlari(&cikti);
    for dosya in [
        "detay/1234567/2026-09-15.json",
        "rapor/1234567/ay-2026-09.json",
        "rapor/1234567/hafta-2026-W99.json",
        "rapor-ham/1234567/2026-09-14.json",
        "istatistik/bozuk.json",
        "merkez/yonetici.json",
        "merkez/listeler/kumar.json",
        "kuyruk/0123456789abcdef.json",
        "rapor/1234567/liste.json",
    ] {
        assert!(h.iter().any(|l| l.starts_with(&format!("HATA dosya {dosya} "))), "{dosya}: {cikti}");
    }
    assert_eq!(h.len(), 9, "{cikti}");
}

#[test]
fn sembolik_bag_izlenmez() {
    let kok = gecici_dizin().join("kok");
    let hedef = gecici_dizin().join("hedef");
    yaz(&hedef, "x.json", "{bozuk");
    fs::create_dir_all(&kok).unwrap();
    std::os::unix::fs::symlink(hedef.join("x.json"), kok.join("bag.json")).unwrap();
    std::os::unix::fs::symlink(&hedef, kok.join("bagdizin")).unwrap();
    let (tamam, cikti) = calistir(&gocmus_db(), Some(&kok));
    assert!(tamam, "{cikti}");
    assert!(!cikti.contains("DOSYA"), "{cikti}");
}

// --- örnek ağaç (geri okuma testinin girdisi) ---

const WG: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const WG2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
const SSH: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test";
const MUSTERI_PAROLA: &str = "ornek-musteri-parolasi-1";
const ADMIN_PAROLA: &str = "ornek-ortak-admin-parola-1";
const YONETICI_PAROLA: &str = "ornek-yonetici-parolasi-1";

/// `dizin` altındaki bütün dosyaların değişiklik zamanını tam saniyeye (SAAT_BASLANGIC) sabitler.
fn sabit_zaman(dizin: &Path) {
    let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(crate::test_yardim::SAAT_BASLANGIC as u64);
    for x in fs::read_dir(dizin).unwrap().flatten() {
        let yol = x.path();
        if yol.is_dir() {
            sabit_zaman(&yol);
        } else {
            let f = fs::OpenOptions::new().write(true).open(&yol).unwrap();
            f.set_times(fs::FileTimes::new().set_accessed(t).set_modified(t)).unwrap();
        }
    }
}

/// Dosyaların göreli yolları (örnek ağaçtaki `.json`lar); `ornek-beklenen.json` hariç.
fn json_adlari(kok: &Path, alt: &str) -> Vec<String> {
    let mut o: Vec<String> = fs::read_dir(kok.join(alt))
        .map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|a| a.ends_with(".json")).collect())
        .unwrap_or_default();
    o.sort();
    o
}

/// Rust'ın gerçek yazıcılarıyla (veri, durum, istatistik, detay, rapor, rapor-ham, kuyruk, kuyruk-sonuc, listeler, yonetici)
/// bir `/var/lib/wificorrect` benzeri ağaç üretir, `denetle`'den geçirir ve Python'un doğrulayacağı sayıları
/// `ornek-beklenen.json`'a yazar. `WFC_ORNEK_AGAC=<dizin>` verilirse ağaç orada kalır (scripts/test-merkez.sh cihazdan
/// yerel dizine kopyalar); yoksa geçici dizinde üretilip yalnızca denetlenir.
#[test]
fn ornek_agac() {
    use crate::rapor::testler::{misafir_gun, muhurle, TEL1, TEL2, TEL3};
    let kok = match std::env::var("WFC_ORNEK_AGAC") {
        Ok(d) if !d.is_empty() => {
            let p = PathBuf::from(d);
            let _ = fs::remove_dir_all(&p);
            p
        }
        _ => gecici_dizin().join("kok"),
    };
    let merkez = kok.join("merkez");
    fs::create_dir_all(&merkez).unwrap();
    // arşiv kaynakları (kaynak/yeni, kaynak/eski, kaynak/kayit.csv; geri_oku.py rapor.py'yi bunlarla çalıştırır) ve sahte sunucu
    let kaynak = kok.join("kaynak");
    fs::create_dir_all(&kaynak).unwrap();
    let saat = SahteSaat::default();
    let simdi = saat.t();

    // --- veritabanı: gerçek yazıcılarla ---
    let vt = merkez.join("merkez.db");
    let v = crate::veri::Veri::ac(&vt, saat.saat()).unwrap();
    let (n, _) = v.musteri_ekle("Örnek müşteri", Some(MUSTERI_PAROLA)).unwrap();
    let (n2, _) = v.musteri_ekle("Bitmiş müşteri", None).unwrap();
    let (cid, anahtar) = v.cihaz_bagla(n, WG, SSH, "Örnek Kafe", "Örnek Kafe Ltd. Şti.", "0.4.29").unwrap();
    v.eslesme_kaydet(cid, "Örnek Kafe", "Örnek Kafe Ltd. Şti.", "0.4.29", Some("otel"), Some("Örnek Mah. 1/2")).unwrap();
    v.guncelleme_kanali_koy(n, "test").unwrap();
    v.guncelleme_hemen_koy(n, simdi + 10.0, "0.4.29").unwrap();
    v.cihaz_bagla(n2, WG2, SSH, "", "", "").unwrap();
    v.serbest_birak(n2, true).unwrap();
    v.uyelik(n2, "bitti").unwrap();
    v.hareket("admin", "203.0.113.5", "MUSTERI_EKLENDI", Some(n), "ornek").unwrap();
    v.hareket("sistem", "", "SURUM_ALINDI", None, "0.4.29").unwrap();
    let s1 = v.sorun_ac(n, "cihaz_yok", "Cihaz raporu gelmiyor", "kritik").unwrap();
    v.sorun_sms(s1, "acilis_sms", "gonderildi").unwrap();
    let s2 = v.sorun_ac(n, "disk", "Disk dolu", "uyari").unwrap();
    v.sorun_kapat(s2).unwrap();
    v.surum_ekle("0.4.29", &"a".repeat(64), 1234, "github").unwrap();
    v.surum_ekle("0.4.28", &"b".repeat(64), 1200, "yerel").unwrap();
    v.kanal_hedef_koy("kararli", Some(crate::veri::EN_SON)).unwrap();
    v.kanal_hedef_koy("test", Some("0.4.28")).unwrap();
    v.admin_parolasi_koy(ADMIN_PAROLA).unwrap();
    v.sms_ayari_koy(&json!({"mock": true, "tr_saglayici": "netgsm"})).unwrap();
    v.uyari_ayari_koy(&json!({"telefonlar": ["5321234567"], "acik": true, "gunluk_sinir": 20})).unwrap();
    v.sms_sayac_artir(n, "2026-09-21").unwrap();
    v.sms_sayac_artir(n, "2026-09-21").unwrap();
    v.sms_son_kaydet(n, "netgsm", "ok", "0").unwrap();
    v.sms_say("2026-09-21").unwrap();
    v.saglik_son_tur_koy("2026-09-21T17:10:00+03:00").unwrap();
    assert!(v.gunluk_bayrak("izleme", "2026-09-21").unwrap());
    v.saglik_kaydet(cid, &json!({"disk": 42, "servisler": {"wg": true}, "surum": "0.4.29"})).unwrap();
    let bekleyen = "a".repeat(32);
    v.bekleyen_bildir(&bekleyen, &"s".repeat(64), "203.0.113.9", "0.4.29", "aa:bb:cc:dd:ee:ff", WG2, SSH).unwrap();
    v.eski_arsiv_bagla("bocafe", Some(n)).unwrap();
    v.eski_arsiv_bagla("bocafe-test", None).unwrap();
    v.rapor_gruplari_yaz(&merkez.join("rapor-gruplari.json")).unwrap();

    // --- arşiv kaynakları, durum, istatistik, detay, rapor-ham, rapor ---
    let (yeni, eski) = (kaynak.join("yeni"), kaynak.join("eski"));
    let veri_dizini = yeni.join(n.to_string()).join("veri");
    let gunler = ["2026-09-15", "2026-09-16", "2026-09-17"];
    misafir_gun(&veri_dizini, gunler[0], &[TEL1, TEL2]);
    misafir_gun(&veri_dizini, gunler[1], &[TEL1]);
    misafir_gun(&veri_dizini, gunler[2], &[TEL3]);
    muhurle(&veri_dizini, &gunler);
    misafir_gun(&eski.join("bocafe"), "2026-09-14", &[TEL2]);
    // ham özetin `imza`sı dosya değişiklik zamanlarını (ns) içerir; ağaç cihazdan yerele tar ile taşınıp Python'da yeniden
    // doğrulanacağından zamanlar tam saniyeye sabitlenir (tar saniye hassasiyetinde taşır)
    sabit_zaman(&kaynak.join("yeni"));
    sabit_zaman(&kaynak.join("eski"));
    let kayit = kaynak.join("kayit.csv");
    fs::write(&kayit, format!("cihaz;{n};10.99.0.11;{WG};wfc-{n}\n")).unwrap();
    let durum_yollari = crate::durum::Yollar { kayit: kayit.clone(), yeni: yeni.clone(), eski: eski.clone(), cikti: kok.join("durum.json"), grup: None };
    assert_eq!(crate::durum::calistir(&durum_yollari, &|| Some(String::new()), simdi), 0);
    for d in ["istatistik", "detay"] {
        fs::create_dir_all(kok.join(d)).unwrap();
    }
    let y = crate::istatistik::Yollar {
        kayit,
        yeni,
        eski,
        cikti: kok.join("istatistik"),
        detay: kok.join("detay"),
        grup: None,
        rapor_ham: kok.join("rapor-ham"),
        rapor_cikti: kok.join("rapor"),
        rapor_gruplari: merkez.join("rapor-gruplari.json"),
    };
    assert_eq!(crate::istatistik::calistir(&y, Some(simdi)), 0);

    // --- kuyruk ve sonuçları (sahte sunucu komutu) ---
    let (kq, sd, wg_dizini) = (kok.join("kuyruk"), kok.join("kuyruk-sonuc"), kaynak.join("wg"));
    for d in [&kq, &sd, &wg_dizini] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(wg_dizini.join("sunucu.pub"), format!("{}=\n", "S".repeat(43))).unwrap();
    fs::write(wg_dizini.join("uc-nokta"), "vpn.wificorrect.com:51820\n").unwrap();
    let sunucu_kayit = kaynak.join("sunucu-kayit.csv");
    fs::write(&sunucu_kayit, "").unwrap();
    let tamam_is = crate::kuyruk::ekle("cihaz-ekle", &json!({"numara": n.to_string(), "wg_pub": WG, "ssh_pub": SSH}), &kq).unwrap();
    let hata_is = crate::kuyruk::ekle("cihaz-ekle", &json!({"numara": n.to_string(), "wg_pub": "gecersiz", "ssh_pub": SSH}), &kq).unwrap();
    let sk = sunucu_kayit.clone();
    let mut komut = move |k: &[String]| -> (i32, String, String) {
        if k[1] == "cihaz-ekle" {
            let mut s = fs::read_to_string(&sk).unwrap();
            s.push_str(&format!("cihaz;{};10.99.0.12;{};wfc-{}\n", k[2], k[3], k[2]));
            fs::write(&sk, s).unwrap();
        }
        (0, String::new(), String::new())
    };
    assert_eq!(crate::kuyruk::calistir(&kq, &sd, &mut komut, &sunucu_kayit, &wg_dizini, None, None), 0);
    assert!(json_adlari(&kok, "kuyruk").is_empty(), "iş dosyaları tüketilmeli");

    // --- hazır liste, yönetici hesabı ---
    let liste_url = "https://ornek.invalid/liste";
    let indirme = crate::test_yardim::SahteIndirme::yeni(std::collections::HashMap::from([(
        liste_url.to_string(),
        Ok("kumar1.com\nbahis2.net\nbet3.org\n".as_bytes().to_vec()),
    )]));
    let uretici = crate::listeler::Uretici {
        kaynaklar: vec![("kumar", vec![crate::listeler::Kaynak { ad: "Örnek", url: liste_url, bicim: crate::listeler::Bicim::Duz }])],
        en_cok: crate::listeler::EN_COK,
    };
    let meta = uretici.uret("kumar", &indirme, &merkez.join("listeler"), simdi).unwrap();
    crate::yonetici::yaz(&merkez.join("yonetici.json"), "ornek-yonetici", YONETICI_PAROLA).unwrap();

    // --- denetim: Rust'ın kendi yazdığı her şey okunabilmeli ---
    let (tamam, cikti) = calistir(&vt, Some(&kok));
    assert!(tamam, "{cikti}");

    // --- Python'un doğrulayacağı sayılar (Rust'ın yazdıkları) ---
    let sayi = |tablo: &str| -> i64 { Connection::open(&vt).unwrap().query_row(&format!("SELECT COUNT(*) FROM {tablo}"), [], |r| r.get(0)).unwrap() };
    let tablolar: BTreeMap<&str, i64> = ["musteri", "cihaz", "hareket", "eski_arsiv", "ayar", "saglik", "sorun", "surum", "sms_sayac", "sms_son", "bekleyen"]
        .into_iter()
        .map(|t| (t, sayi(t)))
        .collect();
    let arsivler = [n.to_string(), "bocafe".to_string()];
    let raporlar = crate::raporlar::Raporlar::new(kok.join("rapor"));
    let liste: Vec<Value> = raporlar.liste(n).iter().map(|o| json!({"tur": o.tur, "donem": o.donem, "kesin": o.kesin})).collect();
    assert!(!liste.is_empty(), "rapor üretilmedi");
    let istatistik: BTreeMap<String, Vec<String>> = arsivler
        .iter()
        .map(|a| {
            let v: Value = serde_json::from_str(&fs::read_to_string(kok.join("istatistik").join(format!("{a}.json"))).unwrap()).unwrap();
            (a.clone(), v.as_object().unwrap().keys().cloned().collect())
        })
        .collect();
    let detay: BTreeMap<String, Vec<String>> = arsivler
        .iter()
        .map(|a| (a.clone(), json_adlari(&kok, &format!("detay/{a}")).into_iter().filter(|x| x != "kisiler.json").map(|x| x.trim_end_matches(".json").to_string()).collect()))
        .collect();
    let sonuc = |id: &str| crate::kuyruk::sonuc(id, &sd).unwrap()["durum"].as_str().unwrap().to_string();
    let kuyruk_sonuclari: BTreeMap<String, String> = [tamam_is.clone(), hata_is.clone()].into_iter().map(|id| (id.clone(), sonuc(&id))).collect();
    let beklenen = json!({
        "numara": n, "numara_bitmis": n2, "parola": MUSTERI_PAROLA, "cihaz_id": cid, "cihaz_anahtari": anahtar,
        "tablolar": tablolar,
        "sorun_acik": 1, "sorun_toplam": 2,
        "surumler": ["0.4.29", "0.4.28"], "kanal_kararli": "en-son", "kanal_test": "0.4.28", "en_son_surum": "0.4.29",
        "admin_parola": ADMIN_PAROLA, "sms_sayac": 2, "sms_son": {"saglayici": "netgsm", "sonuc": "ok"}, "sms_sayaci": 1,
        "bekleyen": bekleyen, "eski_arsivler": ["bocafe", "bocafe-test"], "arsivler": arsivler,
        "istatistik": istatistik, "detay_gunleri": detay, "rapor_liste": liste,
        "kuyruk": kuyruk_sonuclari,
        "liste": {"kumar": meta["sayi"].clone()},
        "yonetici": {"kullanici": "ornek-yonetici", "parola": YONETICI_PAROLA},
        "durum_arsivleri": arsivler,
        "saat": simdi, "gun": "2026-09-21", "sms_mock": true, "guncelleme_kanali": "test", "cihazlar": 1, "bitmis_cihaz_gecmisi": 1,
        "musteri_hareketleri": 1, "rapor_gruplari": {n.to_string(): ["bocafe"]},
        "isletme_turu": "otel", "cihaz_hedefi": "0.4.28", "saglik_disk": 42, "sms_tr_saglayici": "netgsm",
        "uyari_telefonlari": ["905321234567"], "saglik_son_tur": "2026-09-21T17:10:00+03:00",
    });
    fs::write(kok.join("ornek-beklenen.json"), serde_json::to_string_pretty(&beklenen).unwrap()).unwrap();
}
