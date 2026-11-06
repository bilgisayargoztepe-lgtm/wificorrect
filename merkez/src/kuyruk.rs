//! Root işleri kuyruğu (Python kuyruk.py, spec §2). Panel (wcpanel) iş dosyası yazar ve sonucu bekler; wc-kuyruk (root,
//! systemd .path ile anında) işi doğrulayıp `wificorrect-sunucu` ile yapar, sonucu yalnızca root'un yazabildiği klasöre
//! koyar. Panelden gelen hiçbir metin kabuğa verilmez: alanlar doğrulanır, komutlar liste argümanla çalışır.
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::guvenlik::rastgele;
use crate::ortak::{py_str, py_strip};
use crate::veri::py_json;

pub const KUYRUK: &str = "/var/lib/wificorrect/kuyruk";
pub const SONUC: &str = "/var/lib/wificorrect/kuyruk-sonuc";
pub const SUNUCU: &str = "/usr/local/sbin/wificorrect-sunucu";
pub const KAYIT: &str = "/etc/wireguard/wificorrect/kayit.csv";
pub const WG_DIR: &str = "/etc/wireguard/wificorrect";
/// Sonuç dosyalarının grubu (panel okur).
pub const GRUP: &str = "wcpanel";

/// Python `WG_RE` (`[A-Za-z0-9+/]{43}=`).
pub fn wg_gecerli(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 44 && b[43] == b'=' && b[..43].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'/')
}

/// Python `SSH_RE` (`ssh-ed25519 [A-Za-z0-9+/]+=*( [A-Za-z0-9@._-]+)?`), tam eşleşme.
pub fn ssh_gecerli(s: &str) -> bool {
    let Some(geri) = s.strip_prefix("ssh-ed25519 ") else { return false };
    let b = geri.as_bytes();
    let anahtar = b.iter().take_while(|c| c.is_ascii_alphanumeric() || **c == b'+' || **c == b'/').count();
    if anahtar == 0 {
        return false;
    }
    let esit = b[anahtar..].iter().take_while(|c| **c == b'=').count();
    let kalan = &b[anahtar + esit..];
    if kalan.is_empty() {
        return true;
    }
    kalan[0] == b' ' && kalan.len() > 1 && kalan[1..].iter().all(|c| c.is_ascii_alphanumeric() || b"@._-".contains(c))
}

/// Python `IS_RE` (`[0-9a-f]{16}\.json`).
pub fn is_adi_gecerli(ad: &str) -> bool {
    ad.len() == 21 && ad.ends_with(".json") && ad[..16].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// --- panel tarafı (wcpanel) ---
/// İş dosyası yazar (`<kuyruk>/.<kimlik>.tmp` → `<kimlik>.json`; .path birimi yalnızca *.json görür). Dönen: iş kimliği.
pub fn ekle(islem: &str, veri: &Value, kuyruk: &Path) -> std::io::Result<String> {
    let kimlik = rastgele::token_hex(8);
    let gecici = kuyruk.join(format!(".{kimlik}.tmp"));
    // Python `dict(veri, islem=islem)`: alanlar olduğu gibi, islem sonda (varsa yerinde)
    let mut is_: Map<String, Value> = veri.as_object().cloned().unwrap_or_default();
    is_.insert("islem".into(), Value::String(islem.into()));
    {
        let mut f = std::fs::File::create(&gecici)?;
        f.write_all(py_json(&Value::Object(is_), true).as_bytes())?;
    }
    std::fs::rename(&gecici, kuyruk.join(format!("{kimlik}.json")))?;
    Ok(kimlik)
}

/// Kök işlemin yazdığı sonuç; dosya yok, okunamıyor ya da sözlük değilse None.
pub fn sonuc(kimlik: &str, sonuc_dizini: &Path) -> Option<Value> {
    let ham = std::fs::read(sonuc_dizini.join(format!("{kimlik}.json"))).ok()?;
    let v: Value = serde_json::from_slice(&ham).ok()?;
    v.is_object().then_some(v)
}

/// Sonucu `aralik` sn arayla `sure` sn kadar bekler (Python: `range(int(sure/aralik) + 1)` deneme, her denemeden sonra uyku).
pub fn bekle(kimlik: &str, sure: f64, aralik: f64, sonuc_dizini: &Path, uyku: &mut dyn FnMut(f64)) -> Option<Value> {
    for _ in 0..(sure / aralik) as usize + 1 {
        if let Some(s) = sonuc(kimlik, sonuc_dizini) {
            return Some(s);
        }
        uyku(aralik);
    }
    None
}

/// Üretim beklemesi: 15 sn, 0,5 sn aralık.
pub fn bekle_uretim(kimlik: &str) -> Option<Value> {
    bekle(kimlik, 15.0, 0.5, Path::new(SONUC), &mut |s| std::thread::sleep(Duration::from_secs_f64(s)))
}

// --- root tarafı (wc-kuyruk) ---
/// İş dosyası güvenilmez (wcpanel'in klasöründe): sembolik bağ izlenmez, FIFO'da beklenmez, en çok 4 KB.
pub fn is_oku(yol: &Path) -> Option<Value> {
    let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(yol).ok()?;
    let md = f.metadata().ok()?;
    if !md.is_file() || md.len() > 4096 {
        return None;
    }
    let mut ham = Vec::new();
    f.take(4097).read_to_end(&mut ham).ok()?;
    let v: Value = serde_json::from_slice(&ham).ok()?;
    v.is_object().then_some(v)
}

/// Python `str.splitlines()` (satır sonu karakterleri; sonda boş satır üretmez).
pub(crate) fn satirlar(metin: &str) -> Vec<&str> {
    let mut o = Vec::new();
    let mut bas = 0;
    let mut it = metin.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            o.push(&metin[bas..i]);
            bas = i + c.len_utf8();
            if c == '\r' && it.peek().is_some_and(|(_, d)| *d == '\n') {
                it.next();
                bas += 1;
            }
        }
    }
    if bas < metin.len() {
        o.push(&metin[bas..]);
    }
    o
}

fn kayit_ara(kayit: &Path, uygun: &dyn Fn(&[&str]) -> bool) -> Option<Vec<String>> {
    // dosya yok (ya da okunamıyor): kayıt yok
    let ham = std::fs::read(kayit).ok()?;
    let metin = String::from_utf8_lossy(&ham);
    for s in satirlar(&metin) {
        let p: Vec<&str> = s.split(';').collect();
        if p.len() >= 5 && p[0] == "cihaz" && uygun(&p) {
            return Some(p.iter().map(|x| x.to_string()).collect());
        }
    }
    None
}

/// kayit.csv'deki aktif cihaz satırı: `[tur, ad, ip, wg, yedek-kullanıcısı, ...]` ya da None.
pub fn kayit_satiri(ad: &str, kayit: &Path) -> Option<Vec<String>> {
    kayit_ara(kayit, &|p| p[1] == ad)
}

/// Bu WireGuard anahtarıyla kayıtlı aktif cihaz satırı (başka adla olabilir) ya da None.
pub fn anahtar_satiri(wg: &str, kayit: &Path) -> Option<Vec<String>> {
    kayit_ara(kayit, &|p| p[3] == wg)
}

fn oku(yol: &Path) -> Result<String, String> {
    std::fs::read(yol)
        .map(|b| py_strip(&String::from_utf8_lossy(&b)).to_string())
        .map_err(|e| format!("{}: {e}", yol.display()))
}

fn hata_sonucu(metin: &str) -> Value {
    // Python `(metin or "bilinmeyen hata").strip()[:300]`
    let m = if metin.is_empty() { "bilinmeyen hata" } else { py_strip(metin) };
    json!({"durum": "hata", "hata": m.chars().take(300).collect::<String>()})
}

/// Komut çalıştırıcı: (çıkış kodu, stdout, stderr).
pub type Komut<'a> = &'a mut dyn FnMut(&[String]) -> (i32, String, String);

fn sunucu_komutu(args: &[&str]) -> Vec<String> {
    std::iter::once(SUNUCU).chain(args.iter().copied()).map(String::from).collect()
}

/// İşi doğrular ve `wificorrect-sunucu` ile yapar; sonuç sözlüğü (`durum`: tamam | hata).
pub fn isle(is_: &Value, calistir: Komut, kayit: &Path, wg_dir: &Path) -> Value {
    let numara = match is_.get("numara").and_then(Value::as_str) {
        Some(n) if crate::guvenlik::numara_gecerli(n) => n,
        _ => return hata_sonucu("gecersiz numara"),
    };
    let islem = is_.get("islem");
    if islem == Some(&Value::String("cihaz-kapat".into())) {
        let wg = is_.get("wg_pub");
        if let Some(w) = wg {
            if !w.as_str().is_some_and(wg_gecerli) {
                return hata_sonucu("gecersiz anahtar");
            }
        }
        let s = kayit_satiri(numara, kayit);
        // eski bir kapatma işi, numaraya sonradan bağlanan cihazı kapatmasın
        if s.is_some_and(|s| wg.is_none() || wg.and_then(Value::as_str) == Some(s[3].as_str())) {
            let (kod, _, hata) = calistir(&sunucu_komutu(&["cihaz-kapat", numara]));
            if kod != 0 {
                return hata_sonucu(&hata);
            }
        }
        return json!({"durum": "tamam"});
    }
    if islem != Some(&Value::String("cihaz-ekle".into())) {
        return hata_sonucu("bilinmeyen islem");
    }
    let (Some(wg), Some(ssh)) = (
        is_.get("wg_pub").and_then(Value::as_str).filter(|w| wg_gecerli(w)),
        is_.get("ssh_pub").and_then(Value::as_str).filter(|s| ssh_gecerli(s)),
    ) else {
        return hata_sonucu("gecersiz anahtar");
    };
    if let Some(eski) = anahtar_satiri(wg, kayit).filter(|e| e[1] != numara) {
        // aynı cihaz elle eklenmiş eski bir kayıtla (ör. bocafe-test) tünelde: önce o kapanır. Başka müşterinin kaydıysa dokunulmaz.
        if crate::guvenlik::numara_gecerli(&eski[1]) {
            return hata_sonucu(&format!("anahtar başka müşterinin cihazında ({})", eski[1]));
        }
        let (kod, _, hata) = calistir(&sunucu_komutu(&["cihaz-kapat", &eski[1]]));
        if kod != 0 {
            return hata_sonucu(&hata);
        }
    }
    let mut s = kayit_satiri(numara, kayit);
    if s.as_ref().is_some_and(|s| s[3] != wg) {
        // müşterinin eski cihazı hâlâ tünelde: önce kapat
        let (kod, _, hata) = calistir(&sunucu_komutu(&["cihaz-kapat", numara]));
        if kod != 0 {
            return hata_sonucu(&hata);
        }
        s = None;
    }
    if s.is_none() {
        let (kod, _, hata) = calistir(&sunucu_komutu(&["cihaz-ekle", numara, wg, ssh]));
        s = kayit_satiri(numara, kayit);
        if kod != 0 || s.is_none() {
            return hata_sonucu(if hata.is_empty() { "cihaz eklenemedi" } else { &hata });
        }
    }
    let s = s.unwrap_or_default();
    let (pub_, uc) = match (oku(&wg_dir.join("sunucu.pub")), oku(&wg_dir.join("uc-nokta"))) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(h), _) | (_, Err(h)) => return hata_sonucu(&h),
    };
    json!({"durum": "tamam", "tunel_ip": s[2], "sunucu_pub": pub_, "uc_nokta": uc})
}

/// Python `_calistir`: komutu liste argümanla çalıştırır (kabuk yok), 120 sn sınır; başlatılamazsa `(1, "", hata)`.
pub fn komut_calistir(komut: &[String]) -> (i32, String, String) {
    use std::process::{Command, Stdio};
    let Some((ilk, geri)) = komut.split_first() else { return (1, String::new(), "boş komut".into()) };
    let mut cocuk = match Command::new(ilk).args(geri).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return (1, String::new(), format!("{e:?}")),
    };
    // boru dolup çocuğu kilitlemesin: çıktıları ayrı iş parçacıklarında oku
    let oku_boru = |b: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut b) = b {
                let _ = b.read_to_end(&mut v);
            }
            String::from_utf8_lossy(&v).into_owned()
        })
    };
    let (o, e) = (
        oku_boru(cocuk.stdout.take().map(|x| Box::new(x) as Box<dyn Read + Send>)),
        oku_boru(cocuk.stderr.take().map(|x| Box::new(x) as Box<dyn Read + Send>)),
    );
    let bas = Instant::now();
    let kod = loop {
        match cocuk.try_wait() {
            Ok(Some(d)) => break d.code().unwrap_or(-1),
            Ok(None) if bas.elapsed() > Duration::from_secs(120) => {
                let _ = cocuk.kill();
                let _ = cocuk.wait();
                return (1, String::new(), format!("TimeoutExpired({komut:?}, 120)"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(h) => return (1, String::new(), format!("{h:?}")),
        }
    };
    (kod, o.join().unwrap_or_default(), e.join().unwrap_or_default())
}

fn yazilis(kuyruk: &Path, ad: &str) -> (f64, String) {
    let m = std::fs::symlink_metadata(kuyruk.join(ad)).map(|m| m.mtime() as f64 + m.mtime_nsec() as f64 * 1e-9).unwrap_or(0.0);
    (m, ad.to_string())
}

fn dosya_zamani(yol: &Path) -> Option<f64> {
    std::fs::metadata(yol).ok().map(|m| m.mtime() as f64 + m.mtime_nsec() as f64 * 1e-9)
}

/// Kuyruğu boşaltır (Python `main`); çıkış kodu. `grup`: sonuç dosyalarının grubu (testte None), `simdi`: eski sonuç temizliği.
pub fn calistir(
    kuyruk: &Path, sonuc_dizini: &Path, komut: Komut, kayit: &Path, wg_dir: &Path, grup: Option<&str>, simdi: Option<f64>,
) -> i32 {
    let simdi = simdi.unwrap_or_else(crate::ortak::simdi);
    let adlar: Vec<String> = match std::fs::read_dir(kuyruk) {
        Ok(d) => d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect(),
        Err(h) => {
            crate::ortak::stderr_yaz(&format!("kuyruk okunamadı: {h}"));
            return 1;
        }
    };
    let mut sirali: Vec<(f64, String)> = adlar.iter().map(|a| yazilis(kuyruk, a)).collect();
    // yazılış sırası: kapat → ekle sırası bozulmasın
    sirali.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.1.cmp(&b.1)));
    for (_, ad) in sirali {
        let yol = kuyruk.join(&ad);
        if !ad.ends_with(".json") {
            continue;
        }
        if !is_adi_gecerli(&ad) {
            // .path birimi her *.json'da tetiklenir: tanınmayanı sil, döngü olmasın
            let _ = std::fs::remove_file(&yol);
            continue;
        }
        let is_ = is_oku(&yol);
        let s = match &is_ {
            Some(i) => isle(i, &mut *komut, kayit, wg_dir),
            None => hata_sonucu("is okunamadi"),
        };
        let hedef = sonuc_dizini.join(&ad);
        let gecici = sonuc_dizini.join(format!("{ad}.tmp"));
        let yazildi = (|| -> Result<(), String> {
            std::fs::write(&gecici, py_json(&s, true)).map_err(|h| h.to_string())?;
            std::fs::set_permissions(&gecici, std::fs::Permissions::from_mode(0o640)).map_err(|h| h.to_string())?;
            if let Some(g) = grup {
                crate::ozet::chown_grup(&gecici, g)?;
            }
            std::fs::rename(&gecici, &hedef).map_err(|h| h.to_string())
        })();
        if let Err(h) = yazildi {
            // iş dosyası silinmez: sonraki çalıştırmada yeniden denenir
            crate::ortak::stderr_yaz(&format!("{ad}: sonuç yazılamadı: {h}"));
            return 1;
        }
        let _ = std::fs::remove_file(&yol);
        let islem = is_.as_ref().and_then(|i| i.get("islem")).map(py_str).unwrap_or_else(|| "?".into());
        let hata = s.get("hata").map(py_str).unwrap_or_default();
        crate::ortak::stderr_yaz(&format!("{ad}: {islem} {} {hata}", s.get("durum").map(py_str).unwrap_or_default()));
    }
    // okunmuş/okunmamış ayrılmaz; 1 günden eski sonuçlar silinir
    if let Ok(d) = std::fs::read_dir(sonuc_dizini) {
        for e in d.filter_map(|e| e.ok()) {
            if dosya_zamani(&e.path()).is_some_and(|t| t < simdi - 86400.0) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    0
}

/// `wificorrect-merkez kuyruk` (wc-kuyruk.service): üretim yolları, gerçek komut çalıştırıcı.
pub fn calistir_uretim() -> i32 {
    let mut komut = |k: &[String]| komut_calistir(k);
    calistir(Path::new(KUYRUK), Path::new(SONUC), &mut komut, Path::new(KAYIT), Path::new(WG_DIR), Some(GRUP), None)
}

#[cfg(test)]
#[path = "kuyruk_testler.rs"]
mod testler;
