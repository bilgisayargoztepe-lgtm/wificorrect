//! Müşteri kişi özetleri (Python ozet.py; detay spec §2): gün başına kişi, oturum, site ve bağlantı özeti ve arama dizini.
//! wc-istatistik (root) çağırır; panel yalnızca üretilen JSON'ları okur.
//!
//! Dosya biçimleri Python'la aynı: özetler `0o640`, klasör `0o750`, root çalışırken grup `wcpanel`. JSON anahtar sırası da
//! Python sözlüklerinin ekleme sırasıdır.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::CString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::detay::py_casefold;
use crate::istatistik::{gun_adi_mi, gun_klasorleri, satirlar, sembolik_mi, tamam_mi, Satir, SatirHata};
use crate::merkez::py_int;
use crate::ortak::{self, py_bosluk, py_strip};

pub const SITE_SINIRI: usize = 100;
pub const BAGLANTI_SINIRI: usize = 50;

/// Arama dizinine giren kayıt: (gün, [(telefon, ad, soyad, odalar)]).
pub type DizinKaydi = (String, Vec<(String, String, String, Vec<String>)>);

/// `{telefon: kafe index.csv satırı}`.
pub type Index = HashMap<String, Satir>;

fn sayi(s: Option<&String>) -> i64 {
    s.and_then(|s| py_int(s)).unwrap_or(0)
}

fn ver<'a>(r: &'a Satir, anahtar: &str) -> &'a str {
    r.get(anahtar).map(String::as_str).unwrap_or("")
}

/// `shutil.chown(yol, group=grup)`: grup adı çözülemezse hata (Python `LookupError`).
pub fn chown_grup(yol: &Path, grup: &str) -> Result<(), String> {
    let ad = CString::new(grup).map_err(|_| "LookupError: grup adı".to_string())?;
    let mut tampon = vec![0u8; 4096];
    // SAFETY: getgrnam_r yalnızca bizim tamponlarımıza yazar; dönen işaretçi tamponun içinde kalır.
    let gid = unsafe {
        let mut g: libc::group = std::mem::zeroed();
        let mut sonuc: *mut libc::group = std::ptr::null_mut();
        let k = libc::getgrnam_r(ad.as_ptr(), &mut g, tampon.as_mut_ptr() as *mut libc::c_char, tampon.len(), &mut sonuc);
        if k != 0 || sonuc.is_null() {
            return Err(format!("LookupError: no such group: {grup:?}"));
        }
        g.gr_gid
    };
    let yol_c = CString::new(yol.as_os_str().as_encoded_bytes()).map_err(|_| "OSError: yol".to_string())?;
    // SAFETY: yol_c geçerli, NUL sonlu bir C dizgisidir; uid -1 = değiştirme
    if unsafe { libc::chown(yol_c.as_ptr(), u32::MAX, gid) } != 0 {
        return Err(format!("OSError({})", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn hata_oku(h: SatirHata) -> String {
    h.to_string()
}

fn io(h: std::io::Error) -> String {
    format!("OSError({h})")
}

fn yaz(yol: &Path, veri: &Value, grup: Option<&str>) -> Result<(), String> {
    ortak::save_json(yol, veri).map_err(io)?;
    std::fs::set_permissions(yol, std::fs::Permissions::from_mode(0o640)).map_err(io)?;
    if let Some(g) = grup {
        chown_grup(yol, g)?;
    }
    Ok(())
}

fn klasor(yol: &Path, grup: Option<&str>) -> Result<(), String> {
    std::fs::create_dir_all(yol).map_err(io)?;
    std::fs::set_permissions(yol, std::fs::Permissions::from_mode(0o750)).map_err(io)?;
    if let Some(g) = grup {
        chown_grup(yol, g)?;
    }
    Ok(())
}

/// Kafe arşiv kökündeki index.csv: `{telefon: satır}`. Yoksa, normal dosya değilse ya da UTF-8 değilse boş.
pub fn index_oku(kafe_dizini: &Path) -> Result<Index, String> {
    match satirlar(&kafe_dizini.join("index.csv")) {
        Ok(s) => Ok(s.into_iter().filter(|r| r.get("telefon").is_some_and(|t| !t.is_empty())).map(|r| (r["telefon"].clone(), r)).collect()),
        Err(SatirHata::Deger(_)) => Ok(HashMap::new()),
        Err(h) => Err(hata_oku(h)),
    }
}

/// Cihazdaki kimlik::dosya_adi: `:` → `_`, yalnızca `[A-Za-z0-9_]` (TC_…, PASAPORT_DE_…, telefon aynen).
pub fn dosya_adi(anahtar: &str) -> String {
    anahtar.replace(':', "_").chars().filter(|c| c.is_ascii() && (c.is_ascii_alphanumeric() || *c == '_')).collect()
}

/// Gün klasöründeki kullanicilar/<dosya_adi>.csv.gz'nin KAYIT satırından (ad, soyad) ya da None.
fn kayit_adi(gun_dizini: &Path, tel: &str) -> Result<Option<(String, String)>, SatirHata> {
    let k = gun_dizini.join("kullanicilar");
    let ad = dosya_adi(tel);
    if !(6..=40).contains(&ad.len()) || sembolik_mi(&k) {
        return Ok(None);
    }
    for r in satirlar(&k.join(format!("{ad}.csv.gz")))? {
        if ver(&r, "olay") == "KAYIT" && !ver(&r, "ad").is_empty() {
            return Ok(Some((ver(&r, "ad").to_string(), ver(&r, "soyad").to_string())));
        }
    }
    Ok(None)
}

struct Site {
    alan: String,
    sayi: i64,
    ilk: String,
    son: String,
}

struct Baglanti {
    zaman: String,
    protokol: String,
    hedef_ip: String,
    hedef_port: String,
    gonderilen: i64,
    alinan: i64,
    sure_sn: i64,
}

struct Oturum {
    olay: String,
    zaman: String,
    ic_ip: String,
    ek: String,
}

struct Kisi {
    telefon: String,
    ad: String,
    soyad: String,
    mac: Vec<String>,
    oturumlar: Vec<Oturum>,
    gonderilen: i64,
    alinan: i64,
    baglanti_sayisi: i64,
    ilk: String,
    son: String,
    siteler: Vec<Site>,
    site_dizini: HashMap<String, usize>,
    baglantilar: Vec<Baglanti>,
}

fn baglanti_sirasi(a: &Baglanti, b: &Baglanti) -> std::cmp::Ordering {
    (-(a.gonderilen + a.alinan)).cmp(&-(b.gonderilen + b.alinan)).then_with(|| a.zaman.cmp(&b.zaman))
}

/// Bir günün kişi bazında özeti; kişiler adı olanlar önce, ad soyad sonra telefon sırasıyla.
pub fn gun_ozeti(gun_dizini: &Path, index: &Index) -> Result<Vec<Value>, SatirHata> {
    let mut kisiler: Vec<Kisi> = Vec::new();
    let mut dizin: HashMap<String, usize> = HashMap::new();

    // Python `kisi(r)`: kişiyi açar, zaman ve MAC'i işler
    fn kisi<'a>(kisiler: &'a mut Vec<Kisi>, dizin: &mut HashMap<String, usize>, r: &Satir) -> &'a mut Kisi {
        let tel = ver(r, "telefon").to_string();
        let i = *dizin.entry(tel.clone()).or_insert_with(|| {
            kisiler.push(Kisi {
                telefon: tel,
                ad: String::new(),
                soyad: String::new(),
                mac: Vec::new(),
                oturumlar: Vec::new(),
                gonderilen: 0,
                alinan: 0,
                baglanti_sayisi: 0,
                ilk: String::new(),
                son: String::new(),
                siteler: Vec::new(),
                site_dizini: HashMap::new(),
                baglantilar: Vec::new(),
            });
            kisiler.len() - 1
        });
        let k = &mut kisiler[i];
        let (zaman, mac) = (ver(r, "zaman"), ver(r, "mac"));
        if !zaman.is_empty() && (k.ilk.is_empty() || zaman < k.ilk.as_str()) {
            k.ilk = zaman.to_string();
        }
        if zaman > k.son.as_str() {
            k.son = zaman.to_string();
        }
        if !mac.is_empty() && !k.mac.iter().any(|m| m == mac) {
            k.mac.push(mac.to_string());
        }
        k
    }

    for r in satirlar(&gun_dizini.join("oturum.csv.gz"))? {
        if ver(&r, "telefon").is_empty() {
            continue;
        }
        let k = kisi(&mut kisiler, &mut dizin, &r);
        if !ver(&r, "ad").is_empty() && k.ad.is_empty() {
            k.ad = ver(&r, "ad").to_string();
            k.soyad = ver(&r, "soyad").to_string();
        }
        if ver(&r, "olay").starts_with("OTURUM_") {
            k.oturumlar.push(Oturum { olay: ver(&r, "olay").into(), zaman: ver(&r, "zaman").into(), ic_ip: ver(&r, "ic_ip").into(), ek: ver(&r, "ek").into() });
        }
    }
    for r in satirlar(&gun_dizini.join("dns.csv.gz"))? {
        if ver(&r, "telefon").is_empty() {
            continue;
        }
        let k = kisi(&mut kisiler, &mut dizin, &r);
        let alan = py_strip(ver(&r, "alan_adi")).to_lowercase().trim_end_matches('.').to_string();
        if ver(&r, "olay") == "DNS" && !alan.is_empty() {
            let zaman = ver(&r, "zaman").to_string();
            let i = match k.site_dizini.get(&alan) {
                Some(i) => *i,
                None => {
                    k.siteler.push(Site { alan: alan.clone(), sayi: 0, ilk: zaman.clone(), son: zaman.clone() });
                    k.site_dizini.insert(alan, k.siteler.len() - 1);
                    k.siteler.len() - 1
                }
            };
            let s = &mut k.siteler[i];
            s.sayi += 1;
            if zaman < s.ilk {
                s.ilk = zaman.clone();
            }
            if zaman > s.son {
                s.son = zaman;
            }
        }
    }
    for r in satirlar(&gun_dizini.join("trafik.csv.gz"))? {
        if ver(&r, "telefon").is_empty() {
            continue;
        }
        let k = kisi(&mut kisiler, &mut dizin, &r);
        if ver(&r, "olay") != "BAGLANTI_BITIS" {
            continue;
        }
        let (g, a) = (sayi(r.get("gonderilen_bayt")), sayi(r.get("alinan_bayt")));
        k.gonderilen += g;
        k.alinan += a;
        k.baglanti_sayisi += 1;
        k.baglantilar.push(Baglanti {
            zaman: ver(&r, "zaman").into(),
            protokol: ver(&r, "protokol").into(),
            hedef_ip: ver(&r, "hedef_ip").into(),
            hedef_port: ver(&r, "hedef_port").into(),
            gonderilen: g,
            alinan: a,
            sure_sn: sayi(r.get("sure_sn")),
        });
        if k.baglantilar.len() > 10 * BAGLANTI_SINIRI {
            // bellek sınırı: binlerce bağlantılı kişide yalnızca en büyükler kalır
            k.baglantilar.sort_by(baglanti_sirasi);
            k.baglantilar.truncate(BAGLANTI_SINIRI);
        }
    }
    for k in kisiler.iter_mut() {
        if k.ad.is_empty() {
            let mut ad = kayit_adi(gun_dizini, &k.telefon)?;
            if ad.is_none() {
                if let Some(i) = index.get(&k.telefon) {
                    ad = Some((ver(i, "ad").to_string(), ver(i, "soyad").to_string()));
                }
            }
            if let Some((a, s)) = ad {
                k.ad = a;
                k.soyad = s;
            }
        }
        k.siteler.sort_by(|a, b| b.sayi.cmp(&a.sayi).then_with(|| a.alan.cmp(&b.alan)));
        k.baglantilar.sort_by(baglanti_sirasi);
        k.baglantilar.truncate(BAGLANTI_SINIRI);
        k.oturumlar.sort_by(|a, b| a.zaman.cmp(&b.zaman));
    }
    kisiler.sort_by(|a, b| {
        (a.ad.is_empty() as u8)
            .cmp(&(b.ad.is_empty() as u8))
            .then_with(|| py_casefold(&format!("{} {}", a.ad, a.soyad)).cmp(&py_casefold(&format!("{} {}", b.ad, b.soyad))))
            .then_with(|| a.telefon.cmp(&b.telefon))
    });
    Ok(kisiler
        .into_iter()
        .map(|mut k| {
            let diger = k.siteler.len().saturating_sub(SITE_SINIRI);
            k.siteler.truncate(SITE_SINIRI);
            // anahtar sırası Python sözlüğününki: diger_site_sayisi en sonda
            let mut m = Map::new();
            m.insert("telefon".into(), json!(k.telefon));
            m.insert("ad".into(), json!(k.ad));
            m.insert("soyad".into(), json!(k.soyad));
            m.insert("mac".into(), json!(k.mac));
            m.insert("oturumlar".into(), Value::Array(k.oturumlar.iter().map(|o| json!({"olay": o.olay, "zaman": o.zaman, "ic_ip": o.ic_ip, "ek": o.ek})).collect()));
            m.insert("gonderilen".into(), json!(k.gonderilen));
            m.insert("alinan".into(), json!(k.alinan));
            m.insert("baglanti_sayisi".into(), json!(k.baglanti_sayisi));
            m.insert("ilk".into(), json!(k.ilk));
            m.insert("son".into(), json!(k.son));
            m.insert("siteler".into(), Value::Array(k.siteler.iter().map(|s| json!({"alan": s.alan, "sayi": s.sayi, "ilk": s.ilk, "son": s.son})).collect()));
            m.insert(
                "baglantilar".into(),
                Value::Array(
                    k.baglantilar
                        .iter()
                        .map(|b| {
                            json!({"zaman": b.zaman, "protokol": b.protokol, "hedef_ip": b.hedef_ip, "hedef_port": b.hedef_port,
                                "gonderilen": b.gonderilen, "alinan": b.alinan, "sure_sn": b.sure_sn})
                        })
                        .collect(),
                ),
            );
            m.insert("diger_site_sayisi".into(), json!(diger));
            Value::Object(m)
        })
        .collect())
}

/// Kişinin o gün misafir koduyla açtığı oturumların odaları (oturum `ek`'indeki `oda=<no>`, büyük harf).
pub fn odalar(k: &Value) -> Vec<String> {
    let mut out: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(Value::Array(oturumlar)) = k.get("oturumlar") {
        for o in oturumlar.iter().filter(|o| o.is_object()) {
            if let Some(oda) = oda_ara(&ortak::py_veya_bos(o.get("ek"))) {
                out.insert(oda.to_uppercase());
            }
        }
    }
    out.into_iter().collect()
}

/// Python `(?:^|\s)oda=(\S+)` ile `search`.
fn oda_ara(ek: &str) -> Option<String> {
    let mut onceki: Option<char> = None;
    for (i, c) in ek.char_indices() {
        let basta = onceki.is_none_or(py_bosluk);
        onceki = Some(c);
        if basta && ek[i..].starts_with("oda=") {
            let kalan = &ek[i + 4..];
            let oda: String = kalan.chars().take_while(|c| !py_bosluk(*c)).collect();
            if !oda.is_empty() {
                return Some(oda);
            }
        }
    }
    None
}

/// Arama dizini için özetten yalnızca gereken (tam özetler bellekte tutulmaz). Odalar özetin oturum notlarından çıkarılır.
/// Özet beklenen biçimde değilse (`KeyError`/`TypeError`) None.
pub fn dizin_kaydi(ozet: &Value) -> Option<DizinKaydi> {
    let gun = ozet.get("gun")?.as_str()?.to_string();
    let mut liste = Vec::new();
    for k in ozet.get("kisiler")?.as_array()? {
        let s = |a: &str| k.get(a).and_then(Value::as_str).map(String::from);
        liste.push((s("telefon")?, s("ad")?, s("soyad")?, odalar(k)));
    }
    Some((gun, liste))
}

struct Giris {
    ad: String,
    soyad: String,
    gunler: Vec<String>,
    odalar: BTreeMap<String, Vec<String>>,
}

fn metin_listesi(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

/// kisiler.json: `{telefon: {ad, soyad, gunler (yeni→eski), odalar {oda: [gün]}}}`. `yeni`: `dizin_kaydi` listesi. Dizinde
/// hiç görünmeyen özet günleri (görev özet ile dizin arasında kesildiyse) dosyadan tek tek okunup eklenir; özet dosyası
/// olmayan günler çıkarılır. Dizin yoksa, bozuksa ya da oda bilgisi olmayan eski biçimdeyse tüm özetlerden böyle yeniden
/// kurulur. Ad en yeni günden gelir.
pub fn kisi_dizini_guncelle(detay_dizini: &Path, yeni: Vec<DizinKaydi>, grup: Option<&str>) -> Result<(), String> {
    let yol = detay_dizini.join("kisiler.json");
    let mut mevcut: HashSet<String> = HashSet::new();
    for d in std::fs::read_dir(detay_dizini).map_err(io)? {
        let a = d.map_err(io)?.file_name().to_string_lossy().into_owned();
        if let Some(gun) = a.strip_suffix(".json") {
            if gun_adi_mi(gun) {
                mevcut.insert(gun.to_string());
            }
        }
    }
    let mut dizin: BTreeMap<String, Giris> = BTreeMap::new();
    match std::fs::read(&yol) {
        Ok(ham) => {
            let v = std::str::from_utf8(&ham).ok().and_then(|m| serde_json::from_str::<Value>(m).ok());
            // sözlük değilse ya da bir giriş sözlük/oda sözlüğü değilse (oda alanından önceki biçim): baştan kurulur
            if let Some(Value::Object(o)) = v {
                if o.values().all(|x| x.is_object() && x.get("odalar").is_some_and(Value::is_object)) {
                    for (tel, x) in &o {
                        let odalar = x["odalar"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), metin_listesi(Some(v)))).collect()).unwrap_or_default();
                        dizin.insert(
                            tel.clone(),
                            Giris {
                                ad: x.get("ad").and_then(Value::as_str).unwrap_or("").into(),
                                soyad: x.get("soyad").and_then(Value::as_str).unwrap_or("").into(),
                                gunler: metin_listesi(x.get("gunler")),
                                odalar,
                            },
                        );
                    }
                }
            }
        }
        Err(h) if h.kind() == std::io::ErrorKind::NotFound => {}
        Err(h) => return Err(io(h)),
    }
    let mut bilinen: HashSet<String> = dizin.values().flat_map(|e| e.gunler.iter().cloned()).collect();
    bilinen.extend(yeni.iter().map(|(g, _)| g.clone()));
    let mut kayitlar = yeni;
    let mut eksik: Vec<&String> = mevcut.difference(&bilinen).collect();
    eksik.sort(); // ponytail: kişisiz günler her çalıştırmada yeniden okunur; küçük dosyalar
    for gun in eksik {
        let okunan = std::fs::read(detay_dizini.join(format!("{gun}.json")))
            .ok()
            .and_then(|ham| serde_json::from_slice::<Value>(&ham).ok())
            .and_then(|v| dizin_kaydi(&v));
        if let Some(k) = okunan {
            kayitlar.push(k); // bozuk özet aramayı durdurmasın
        }
    }
    kayitlar.sort_by(|a, b| a.0.cmp(&b.0));
    for (gun, kisiler) in kayitlar {
        for (tel, ad, soyad, odalar) in kisiler {
            let e = dizin.entry(tel).or_insert_with(|| Giris { ad: String::new(), soyad: String::new(), gunler: Vec::new(), odalar: BTreeMap::new() });
            if !ad.is_empty() && (e.gunler.is_empty() || gun >= *e.gunler.iter().max().expect("dolu")) {
                e.ad = ad;
                e.soyad = soyad;
            }
            if !e.gunler.contains(&gun) {
                e.gunler.push(gun.clone());
            }
            for o in odalar {
                let l = e.odalar.entry(o).or_default();
                if !l.contains(&gun) {
                    l.push(gun.clone());
                }
            }
        }
    }
    let mut cikti = Map::new();
    for (tel, e) in dizin {
        let mut gunler: Vec<String> = e.gunler.into_iter().filter(|g| mevcut.contains(g)).collect::<HashSet<_>>().into_iter().collect();
        if gunler.is_empty() {
            continue;
        }
        gunler.sort_by(|a, b| b.cmp(a));
        let mut odalar = Map::new();
        for (oda, l) in e.odalar {
            let mut l: Vec<String> = l.into_iter().filter(|g| mevcut.contains(g)).collect();
            l.sort();
            if !l.is_empty() {
                odalar.insert(oda, json!(l));
            }
        }
        cikti.insert(tel, json!({"ad": e.ad, "soyad": e.soyad, "gunler": gunler, "odalar": Value::Object(odalar)}));
    }
    yaz(&yol, &Value::Object(cikti), grup)
}

/// Özetlenmemiş tamam günleri özetler, arama dizinini uzlaştırır. Bozuk gün atlanır, sonraki çalıştırmada yeniden denenir.
/// Dönen: hata iletileri; `Err`: kaynakları durduran G/Ç hatası.
pub fn detay_guncelle(kafe_dizini: &Path, detay_dizini: &Path, grup: Option<&str>) -> Result<Vec<String>, String> {
    klasor(detay_dizini, grup)?;
    let mut index: Option<Index> = None;
    let mut yeni: Vec<DizinKaydi> = Vec::new();
    let mut hatalar = Vec::new();
    for gun in gun_klasorleri(kafe_dizini)? {
        let d = kafe_dizini.join("gunluk").join(&gun);
        let hedef = detay_dizini.join(format!("{gun}.json"));
        if hedef.exists() || sembolik_mi(&d) || !tamam_mi(&d) {
            continue;
        }
        if index.is_none() {
            index = Some(index_oku(kafe_dizini)?);
        }
        let kisiler = match gun_ozeti(&d, index.as_ref().expect("yukarıda dolduruldu")) {
            Ok(k) => k,
            Err(h) => {
                // bozuk/kesik gz ya da normal olmayan dosya: yalnızca o gün düşer
                hatalar.push(format!("detay {gun}: {h}"));
                continue;
            }
        };
        let ozet = json!({"gun": gun, "kisiler": kisiler});
        yaz(&hedef, &ozet, grup)?;
        if let Some(k) = dizin_kaydi(&ozet) {
            yeni.push(k);
        }
    }
    kisi_dizini_guncelle(detay_dizini, yeni, grup)?;
    Ok(hatalar)
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::altin::Kayit;
    use crate::test_yardim::gecici_dizin;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    const BASLIK: &str = "zaman;olay;telefon;ad;soyad;mac;ic_ip;protokol;ic_port;hedef_ip;hedef_port;nat_ip;nat_port;alan_adi;gonderilen_bayt;alinan_bayt;sure_sn;oturum_id;ek";
    const A: &str = "5330000001";
    const B: &str = "5330000002";
    const C: &str = "5330000003";

    type Alanlar<'a> = Vec<(&'a str, &'a str)>;

    /// Python `gz_yaz(yol, satirlar)`: BOM + başlık + her satır `;` ile (olmayan alan boş).
    fn gz_yaz(yol: &Path, satirlar: &[Alanlar]) {
        fs::create_dir_all(yol.parent().unwrap()).unwrap();
        let kolonlar: Vec<&str> = BASLIK.split(';').collect();
        let mut m = format!("\u{feff}{BASLIK}\n");
        for s in satirlar {
            let alan: Vec<&str> = kolonlar.iter().map(|k| s.iter().find(|(a, _)| a == k).map(|(_, v)| *v).unwrap_or("")).collect();
            m.push_str(&alan.join(";"));
            m.push('\n');
        }
        let mut e = flate2::write::GzEncoder::new(fs::File::create(yol).unwrap(), flate2::Compression::default());
        e.write_all(m.as_bytes()).unwrap();
        e.finish().unwrap();
    }

    fn manifest(d: &Path, dosyalar: &[&str]) {
        let mut m = String::from("# gun\n");
        for p in dosyalar {
            m.push_str(&format!("abc  {p}\n"));
        }
        fs::write(d.join("MANIFEST.sha256"), m).unwrap();
    }

    fn z(gun: &str, saat: &str) -> String {
        format!("{gun}T{saat}+03:00")
    }

    /// A: o gün oturum açtı (ad oturumda). B: oturumu önceki günden (ad KAYIT'ta). C: yalnızca index.csv'de.
    fn ornek_gun(kok: &Path, gun: &str, bozuk: bool) -> PathBuf {
        let d = kok.join("gunluk").join(gun);
        let t = |s: &str| z(gun, s);
        gz_yaz(
            &d.join("oturum.csv.gz"),
            &[
                vec![("zaman", &t("09:00:00")), ("olay", "OTURUM_BASLA"), ("telefon", A), ("ad", "Ayşe"), ("soyad", "Yılmaz"), ("mac", "aa:aa:aa:aa:aa:01"), ("ic_ip", "10.50.0.21")],
                vec![("zaman", &t("17:00:00")), ("olay", "OTURUM_BITIS"), ("telefon", A), ("ad", "Ayşe"), ("soyad", "Yılmaz"), ("mac", "aa:aa:aa:aa:aa:01"), ("ic_ip", "10.50.0.21"), ("ek", "neden=cikis")],
            ],
        );
        gz_yaz(
            &d.join("dns.csv.gz"),
            &[
                vec![("zaman", &t("09:05:00")), ("olay", "DNS"), ("telefon", A), ("mac", "aa:aa:aa:aa:aa:01"), ("alan_adi", "Instagram.com.")],
                vec![("zaman", &t("09:10:00")), ("olay", "DNS"), ("telefon", A), ("alan_adi", "instagram.com")],
                vec![("zaman", &t("09:07:00")), ("olay", "DNS"), ("telefon", A), ("alan_adi", "google.com")],
                vec![("zaman", &t("10:00:00")), ("olay", "DNS"), ("telefon", B), ("mac", "bb:bb:bb:bb:bb:02"), ("alan_adi", "<script>x</script>.com")],
                vec![("zaman", &t("11:00:00")), ("olay", "DNS"), ("telefon", C), ("alan_adi", "a.com")],
                vec![("zaman", &t("11:30:00")), ("olay", "DNS"), ("telefon", ""), ("alan_adi", "izinli.com")],
            ],
        );
        if bozuk {
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("trafik.csv.gz"), b"\x1f\x8b\x08\x00bozuk").unwrap();
        } else {
            gz_yaz(
                &d.join("trafik.csv.gz"),
                &[
                    vec![("zaman", &t("09:06:00")), ("olay", "BAGLANTI_BITIS"), ("telefon", A), ("protokol", "tcp"), ("hedef_ip", "1.1.1.1"), ("hedef_port", "443"), ("gonderilen_bayt", "100"), ("alinan_bayt", "900"), ("sure_sn", "60")],
                    vec![("zaman", &t("09:08:00")), ("olay", "BAGLANTI_BITIS"), ("telefon", A), ("protokol", "udp"), ("hedef_ip", "8.8.8.8"), ("hedef_port", "53"), ("gonderilen_bayt", "10"), ("alinan_bayt", "10"), ("sure_sn", "1")],
                    vec![("zaman", &t("09:05:30")), ("olay", "BAGLANTI_BASLA"), ("telefon", A), ("hedef_ip", "1.1.1.1")],
                    vec![("zaman", &t("10:01:00")), ("olay", "BAGLANTI_BITIS"), ("telefon", B), ("gonderilen_bayt", ""), ("alinan_bayt", "abc")],
                    vec![("zaman", &t("11:01:00")), ("olay", "BAGLANTI_BITIS"), ("telefon", C), ("gonderilen_bayt", "5"), ("alinan_bayt", "5")],
                    vec![("zaman", &t("11:31:00")), ("olay", "BAGLANTI_BITIS"), ("telefon", ""), ("gonderilen_bayt", "9"), ("alinan_bayt", "9")],
                ],
            );
        }
        gz_yaz(
            &d.join("kullanicilar").join(format!("{B}.csv.gz")),
            &[vec![("zaman", &t("10:00:00")), ("olay", "KAYIT"), ("telefon", B), ("ad", "Bora"), ("soyad", "Kaya"), ("mac", "bb:bb:bb:bb:bb:02")]],
        );
        manifest(&d, &["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz", &format!("kullanicilar/{B}.csv.gz")]);
        fs::write(kok.join("index.csv"), format!("\u{feff}telefon;ad;soyad;ilk_kayit;son_oturum\n{C};Cem;Er;;\n")).unwrap();
        d
    }

    fn oku(yol: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(yol).unwrap()).unwrap()
    }

    fn listele(d: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(d).unwrap().map(|x| x.unwrap().file_name().into_string().unwrap()).collect();
        v.sort();
        v
    }

    #[test]
    fn gun_ozeti_kisiler_ve_toplamlar() {
        let _k = Kayit::bas("test_wc_ozet", "test_gun_ozeti_kisiler_ve_toplamlar");
        let tmp = gecici_dizin();
        let gun = "2026-09-27";
        let d = ornek_gun(&tmp, gun, false);
        let kisiler = gun_ozeti(&d, &index_oku(&tmp).unwrap()).unwrap();
        let tel: Vec<&str> = kisiler.iter().map(|k| k["telefon"].as_str().unwrap()).collect();
        assert_eq!(tel, vec![A, B, C]); // Ayşe, Bora, Cem; telefonsuz satır yok
        let (a, b, c) = (&kisiler[0], &kisiler[1], &kisiler[2]);
        let adlar = |k: &Value| (k["ad"].as_str().unwrap().to_string(), k["soyad"].as_str().unwrap().to_string());
        assert_eq!((adlar(a), adlar(b), adlar(c)), (("Ayşe".into(), "Yılmaz".into()), ("Bora".into(), "Kaya".into()), ("Cem".into(), "Er".into())));
        assert_eq!((&a["gonderilen"], &a["alinan"], &a["baglanti_sayisi"]), (&json!(110), &json!(910), &json!(2)));
        assert_eq!((&b["gonderilen"], &b["alinan"], &b["baglanti_sayisi"]), (&json!(0), &json!(0), &json!(1)));
        assert_eq!(a["siteler"][0], json!({"alan": "instagram.com", "sayi": 2, "ilk": z(gun, "09:05:00"), "son": z(gun, "09:10:00")}));
        let alanlar: Vec<&str> = a["siteler"].as_array().unwrap().iter().map(|s| s["alan"].as_str().unwrap()).collect();
        assert!(alanlar == vec!["instagram.com", "google.com"] && a["diger_site_sayisi"] == json!(0));
        let boy: Vec<i64> = a["baglantilar"].as_array().unwrap().iter().map(|x| x["gonderilen"].as_i64().unwrap() + x["alinan"].as_i64().unwrap()).collect();
        assert_eq!(boy, vec![1000, 20]);
        assert_eq!(
            a["baglantilar"][0],
            json!({"zaman": z(gun, "09:06:00"), "protokol": "tcp", "hedef_ip": "1.1.1.1", "hedef_port": "443", "gonderilen": 100, "alinan": 900, "sure_sn": 60})
        );
        let olaylar: Vec<&str> = a["oturumlar"].as_array().unwrap().iter().map(|o| o["olay"].as_str().unwrap()).collect();
        assert_eq!(olaylar, vec!["OTURUM_BASLA", "OTURUM_BITIS"]);
        assert_eq!((&a["ilk"], &a["son"]), (&json!(z(gun, "09:00:00")), &json!(z(gun, "17:00:00"))));
        assert!(a["mac"] == json!(["aa:aa:aa:aa:aa:01"]) && b["mac"] == json!(["bb:bb:bb:bb:bb:02"]));
        assert!(b["siteler"][0]["alan"] == json!("<script>x</script>.com") && b["oturumlar"] == json!([]));
        // anahtar sırası Python sözlüğününki (dosya biçimi)
        let anahtarlar: Vec<&str> = a.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            anahtarlar,
            vec!["telefon", "ad", "soyad", "mac", "oturumlar", "gonderilen", "alinan", "baglanti_sayisi", "ilk", "son", "siteler", "baglantilar", "diger_site_sayisi"]
        );
    }

    #[test]
    fn site_ve_baglanti_sinirlari() {
        let _k = Kayit::bas("test_wc_ozet", "test_site_ve_baglanti_sinirlari");
        let tmp = gecici_dizin();
        let gun = "2026-09-27";
        let d = tmp.join("gunluk").join(gun);
        let t = z(gun, "10:00:00");
        let alanlar: Vec<String> = (0..105).map(|i| format!("s{i:03}.com")).collect();
        let dns: Vec<Alanlar> = alanlar.iter().map(|a| vec![("zaman", t.as_str()), ("olay", "DNS"), ("telefon", A), ("alan_adi", a.as_str())]).collect();
        gz_yaz(&d.join("dns.csv.gz"), &dns);
        let bayt: Vec<String> = (0..600).map(|i| i.to_string()).collect();
        let trafik: Vec<Alanlar> = bayt.iter().map(|g| vec![("zaman", t.as_str()), ("olay", "BAGLANTI_BITIS"), ("telefon", A), ("gonderilen_bayt", g.as_str()), ("alinan_bayt", "0")]).collect();
        gz_yaz(&d.join("trafik.csv.gz"), &trafik);
        let kisiler = gun_ozeti(&d, &Index::new()).unwrap();
        assert_eq!(kisiler.len(), 1);
        let k = &kisiler[0];
        assert!(k["siteler"].as_array().unwrap().len() == 100 && k["diger_site_sayisi"] == json!(5));
        let g: Vec<i64> = k["baglantilar"].as_array().unwrap().iter().map(|x| x["gonderilen"].as_i64().unwrap()).collect();
        assert_eq!(g, (550..=599).rev().collect::<Vec<i64>>());
        assert!(k["baglanti_sayisi"] == json!(600) && k["gonderilen"] == json!((0..600).sum::<i64>()));
    }

    #[test]
    fn detay_guncelle_artimli_ve_dizin() {
        let _k = Kayit::bas("test_wc_ozet", "test_detay_guncelle_artimli_ve_dizin");
        let tmp = gecici_dizin();
        let (kafe, dd) = (tmp.join("arsiv").join("bocafe"), tmp.join("detay").join("bocafe"));
        ornek_gun(&kafe, "2026-09-26", false);
        ornek_gun(&kafe, "2026-09-27", false);
        let yarim = kafe.join("gunluk").join("2026-09-28");
        fs::create_dir_all(&yarim).unwrap();
        manifest(&yarim, &["trafik.csv.gz"]); // rsync sürüyor: trafik henüz yok
        assert_eq!(detay_guncelle(&kafe, &dd, None).unwrap(), Vec::<String>::new());
        assert_eq!(listele(&dd), vec!["2026-09-26.json", "2026-09-27.json", "kisiler.json"]);
        let dizin = oku(&dd.join("kisiler.json"));
        assert_eq!(dizin[A], json!({"ad": "Ayşe", "soyad": "Yılmaz", "gunler": ["2026-09-27", "2026-09-26"], "odalar": {}}));
        fs::write(dd.join("2026-09-26.json"), r#"{"gun": "2026-09-26", "kisiler": []}"#).unwrap(); // özetlenmiş gün yeniden üretilmez
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("2026-09-26.json"))["kisiler"], json!([]));
        fs::remove_file(dd.join("2026-09-27.json")).unwrap(); // saklama sildi → dizinden de düşer (arşivde hâlâ var → yeniden üretilir)
        fs::remove_file(kafe.join("gunluk").join("2026-09-27").join("MANIFEST.sha256")).unwrap();
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("kisiler.json"))[A]["gunler"], json!(["2026-09-26"])); // dosyası olmayan 27 düştü
        fs::write(dd.join("kisiler.json"), "{bozuk").unwrap();
        manifest(&kafe.join("gunluk").join("2026-09-27"), &["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz"]);
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("kisiler.json"))[A]["gunler"], json!(["2026-09-27"]));
    }

    #[test]
    fn detay_bozuk_gun_atlanir_sonra_denenir() {
        let _k = Kayit::bas("test_wc_ozet", "test_detay_bozuk_gun_atlanir_sonra_denenir");
        let tmp = gecici_dizin();
        let (kafe, dd) = (tmp.join("bocafe"), tmp.join("detay"));
        ornek_gun(&kafe, "2026-09-26", true);
        ornek_gun(&kafe, "2026-09-27", false);
        let hatalar = detay_guncelle(&kafe, &dd, None).unwrap();
        assert!(hatalar.len() == 1 && hatalar[0].contains("2026-09-26"), "{hatalar:?}");
        assert_eq!(listele(&dd), vec!["2026-09-27.json", "kisiler.json"]);
    }

    #[test]
    fn telefon_yol_disina_cikamaz() {
        let _k = Kayit::bas("test_wc_ozet", "test_telefon_yol_disina_cikamaz");
        let tmp = gecici_dizin();
        let d = tmp.join("gunluk").join("2026-09-27");
        gz_yaz(&d.join("dns.csv.gz"), &[vec![("zaman", "x"), ("olay", "DNS"), ("telefon", "../../../index"), ("alan_adi", "a.com")]]);
        let kisiler = gun_ozeti(&d, &Index::new()).unwrap();
        assert_eq!(kisiler.len(), 1);
        assert!(kisiler[0]["telefon"] == json!("../../../index") && kisiler[0]["ad"] == json!(""));
    }

    #[test]
    fn dizine_girmemis_ozet_gunu_eklenir() {
        let _k = Kayit::bas("test_wc_ozet", "test_dizine_girmemis_ozet_gunu_eklenir");
        let tmp = gecici_dizin();
        let (kafe, dd) = (tmp.join("arsiv").join("bocafe"), tmp.join("detay").join("bocafe"));
        ornek_gun(&kafe, "2026-09-26", false);
        detay_guncelle(&kafe, &dd, None).unwrap();
        let d27 = ornek_gun(&kafe, "2026-09-27", false);
        // görev özeti yazıp dizinden önce öldü
        let ozet = json!({"gun": "2026-09-27", "kisiler": gun_ozeti(&d27, &Index::new()).unwrap()});
        fs::write(dd.join("2026-09-27.json"), serde_json::to_string(&ozet).unwrap()).unwrap();
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("kisiler.json"))[A]["gunler"], json!(["2026-09-27", "2026-09-26"]));
    }

    #[test]
    fn oda_dizini_ve_eski_bicim_yeniden_kurulur() {
        let _k = Kayit::bas("test_wc_ozet", "test_oda_dizini_ve_eski_bicim_yeniden_kurulur");
        let tmp = gecici_dizin();
        let (kafe, dd) = (tmp.join("arsiv").join("bocafe"), tmp.join("detay").join("bocafe"));
        let d = ornek_gun(&kafe, "2026-09-26", false);
        gz_yaz(
            &d.join("oturum.csv.gz"),
            &[vec![
                ("zaman", &z("2026-09-26", "09:00:00")),
                ("olay", "OTURUM_BASLA"),
                ("telefon", "TC:10000000146"),
                ("ad", "Deniz"),
                ("ek", "sozlesme=1 riza=1 giris=misafir_kodu oda=a-12 kod_bitis=x ureten=mudur"),
            ]],
        );
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("kisiler.json"))["TC:10000000146"]["odalar"], json!({"A-12": ["2026-09-26"]}));
        // oda alanından önceki biçim
        fs::write(dd.join("kisiler.json"), r#"{"TC:10000000146": {"ad": "Deniz", "soyad": "", "gunler": ["2026-09-26"]}}"#).unwrap();
        detay_guncelle(&kafe, &dd, None).unwrap();
        assert_eq!(oku(&dd.join("kisiler.json"))["TC:10000000146"]["odalar"], json!({"A-12": ["2026-09-26"]}));
    }

    #[test]
    fn kimlikli_kisi_dosyasindan_ad() {
        let _k = Kayit::bas("test_wc_ozet", "test_kimlikli_kisi_dosyasindan_ad");
        let tmp = gecici_dizin();
        let d = tmp.join("gunluk").join("2026-09-27");
        gz_yaz(&d.join("dns.csv.gz"), &[vec![("zaman", "x"), ("olay", "DNS"), ("telefon", "PASAPORT:DE:X1234567"), ("alan_adi", "a.com")]]);
        gz_yaz(
            &d.join("kullanicilar").join("PASAPORT_DE_X1234567.csv.gz"),
            &[vec![("zaman", "x"), ("olay", "KAYIT"), ("telefon", "PASAPORT:DE:X1234567"), ("ad", "Hans"), ("soyad", "Muller")]],
        );
        let kisiler = gun_ozeti(&d, &Index::new()).unwrap();
        assert_eq!(kisiler.len(), 1);
        assert_eq!((&kisiler[0]["ad"], &kisiler[0]["soyad"]), (&json!("Hans"), &json!("Muller")));
    }

    #[test]
    fn oda_arama_regex_gibi() {
        assert_eq!(oda_ara("giris=misafir_kodu oda=a-12 kod_bitis=x").as_deref(), Some("a-12"));
        assert_eq!(oda_ara("oda=7").as_deref(), Some("7")); // başta
        assert_eq!(oda_ara("xoda=7"), None); // sözcük ortasında değil
        assert_eq!(oda_ara("oda= x oda=9").as_deref(), Some("9")); // boş değerli aday atlanır
        assert_eq!(oda_ara("a\toda=Z1").as_deref(), Some("Z1"));
        assert_eq!(oda_ara(""), None);
    }

    #[test]
    fn dosya_adi_ve_dizin_kaydi() {
        assert_eq!(dosya_adi("TC:10000000146"), "TC_10000000146");
        assert_eq!(dosya_adi("PASAPORT:DE:X1234567"), "PASAPORT_DE_X1234567");
        assert_eq!(dosya_adi("../../../index"), "index");
        assert_eq!(dizin_kaydi(&json!({"gun": "2026-09-27", "kisiler": [{"telefon": A, "ad": "x", "soyad": "y", "oturumlar": []}]})), Some(("2026-09-27".into(), vec![(A.into(), "x".into(), "y".into(), vec![])])));
        assert_eq!(dizin_kaydi(&json!({"gun": "2026-09-27", "kisiler": [{"telefon": A}]})), None); // KeyError
    }
}
