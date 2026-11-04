//! Ortak yardımcılar (Python common.py'nin canlı kalan parçaları): zaman, tarih gösterimi, JSON/CSV dosyaları,
//! telefon numarası ve dış istek taşıyıcısı.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{FixedOffset, TimeZone};

/// Türkiye: sabit UTC+3, yaz saati yok.
pub const TZ: FixedOffset = match FixedOffset::east_opt(3 * 3600) {
    Some(z) => z,
    None => panic!("TZ"),
};

pub fn simdi() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// `2026-11-04T03:17:00+03:00` (saniye hassasiyeti, Python `isoformat(timespec="seconds")`).
pub fn now_iso(ts: f64) -> String {
    let mikro = (ts * 1e6).round() as i64;
    let sn = mikro.div_euclid(1_000_000);
    match TZ.timestamp_opt(sn, 0).single() {
        Some(t) => t.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        None => String::new(),
    }
}

fn rakam(b: &[u8]) -> bool {
    !b.is_empty() && b.iter().all(u8::is_ascii_digit)
}

/// Ekranda gösterilen tarih (gün.ay.yıl): "2026-10-12" → "12.10.2026"; "2026-10-12T17:38:05+03:00" ya da
/// "2026-10-12 17:38" → "12.10.2026 17:38" (saniye=true: "12.10.2026 17:38:05"). Çözülemeyen değer aynen.
pub fn tarih_goster(iso: &str, saniye: bool) -> String {
    let s = iso.trim();
    let b = s.as_bytes();
    if b.len() < 10 || !(rakam(&b[0..4]) && b[4] == b'-' && rakam(&b[5..7]) && b[7] == b'-' && rakam(&b[8..10])) {
        return iso.to_string();
    }
    let (y, a, g) = (&s[0..4], &s[5..7], &s[8..10]);
    // saat bölümü: [T ]HH:MM(:SS)?
    let saat = if b.len() >= 16 && (b[10] == b'T' || b[10] == b' ') && rakam(&b[11..13]) && b[13] == b':' && rakam(&b[14..16]) {
        let sn = if b.len() >= 19 && b[16] == b':' && rakam(&b[17..19]) { Some(&s[17..19]) } else { None };
        Some((&s[11..13], &s[14..16], sn))
    } else {
        None
    };
    let gun = format!("{g}.{a}.{y}");
    match saat {
        None if b.len() == 10 => gun,
        None => iso.to_string(),
        Some((sa, dk, Some(sn))) if saniye => format!("{gun} {sa}:{dk}:{sn}"),
        Some((sa, dk, _)) => format!("{gun} {sa}:{dk}"),
    }
}

pub fn tarih_saat_goster(iso: &str) -> String {
    tarih_goster(iso, true)
}

/// Kişinin yazdığı tarih: "12.10.2026", "1.9.2026", "12.10.2026 17:38" → ISO "2026-10-12[ 17:38]".
/// Başka her şey kırpılıp aynen döner; geçerliliği çağıran denetler.
pub fn tarih_girdi(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    // 1-2 rakam . 1-2 rakam . 4 rakam
    let say = |bas: usize| b[bas..].iter().take_while(|c| c.is_ascii_digit()).count();
    let g = say(0).min(2);
    let mut i = g;
    if g == 0 || b.get(i) != Some(&b'.') {
        return s.to_string();
    }
    i += 1;
    let a = say(i).min(2);
    let a_bas = i;
    i += a;
    if a == 0 || b.get(i) != Some(&b'.') {
        return s.to_string();
    }
    i += 1;
    if i + 4 > b.len() || !rakam(&b[i..i + 4]) {
        return s.to_string();
    }
    let y = &s[i..i + 4];
    i += 4;
    let sonra = &s[i..];
    let saat = if sonra.is_empty() {
        ""
    } else if sonra.starts_with(' ') && !sonra.contains('\n') {
        sonra.trim()
    } else {
        return s.to_string();
    };
    let iso = format!(
        "{y}-{:02}-{:02}",
        s[a_bas..a_bas + a].parse::<u32>().unwrap_or(0),
        s[0..g].parse::<u32>().unwrap_or(0)
    );
    if saat.is_empty() {
        iso
    } else {
        format!("{iso} {saat}")
    }
}

/// JSON dosyası: geçici dosya + fsync + yer değiştirme; UTF-8 (kaçışsız), girinti 1 (Python `indent=1`).
pub fn save_json(yol: &Path, v: &serde_json::Value) -> std::io::Result<()> {
    use serde::Serialize;
    let mut tampon = Vec::new();
    let bicim = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut tampon, bicim);
    v.serialize(&mut ser).map_err(std::io::Error::other)?;
    let mut tmp = yol.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(&tampon)?;
    f.flush()?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, yol)
}

/// `;` ayraçlı CSV'yi (Python csv: `"` ile alıntı, `""` kaçışı, CRLF/LF/CR satır sonu) alan listelerine ayırır.
/// Boş satırlar atlanır (csv.DictReader gibi).
fn csv_coz(metin: &str) -> Vec<Vec<String>> {
    let mut satirlar = Vec::new();
    let mut satir: Vec<String> = Vec::new();
    let mut alan = String::new();
    let mut alintida = false;
    // satırda henüz hiçbir karakter görülmedi (boş satır atlanır)
    let mut satir_bos = true;
    let mut alan_basi = true;
    let mut ch = metin.chars().peekable();
    while let Some(c) = ch.next() {
        if alintida {
            if c == '"' {
                if ch.peek() == Some(&'"') {
                    ch.next();
                    alan.push('"');
                } else {
                    alintida = false;
                }
            } else {
                alan.push(c);
            }
            continue;
        }
        match c {
            ';' => {
                satir.push(std::mem::take(&mut alan));
                satir_bos = false;
                alan_basi = true;
            }
            '\r' | '\n' => {
                if c == '\r' && ch.peek() == Some(&'\n') {
                    ch.next();
                }
                if !satir_bos {
                    satir.push(std::mem::take(&mut alan));
                    satirlar.push(std::mem::take(&mut satir));
                }
                satir_bos = true;
                alan_basi = true;
            }
            '"' if alan_basi => {
                alintida = true;
                alan_basi = false;
                satir_bos = false;
            }
            _ => {
                alan.push(c);
                alan_basi = false;
                satir_bos = false;
            }
        }
    }
    if !satir_bos {
        satir.push(alan);
        satirlar.push(satir);
    }
    satirlar
}

/// CSV dosyası (`;`, BOM'lu olabilir, `.gz` ise açılır) → başlık adlarıyla satırlar. Dosya yoksa boş.
/// Eksik alanlar satırda bulunmaz, fazlalar atılır (DictReader).
pub fn read_rows(yol: &Path) -> Vec<BTreeMap<String, String>> {
    let Ok(mut f) = std::fs::File::open(yol) else { return Vec::new() };
    let mut ham = Vec::new();
    if yol.extension().map(|e| e == "gz").unwrap_or(false) {
        if flate2::read::MultiGzDecoder::new(f).read_to_end(&mut ham).is_err() && ham.is_empty() {
            return Vec::new();
        }
    } else if f.read_to_end(&mut ham).is_err() {
        return Vec::new();
    }
    let metin = String::from_utf8_lossy(&ham);
    let metin = metin.strip_prefix('\u{feff}').unwrap_or(&metin);
    let mut satirlar = csv_coz(metin).into_iter();
    let Some(baslik) = satirlar.next() else { return Vec::new() };
    satirlar
        .map(|s| baslik.iter().cloned().zip(s).collect::<BTreeMap<String, String>>())
        .collect()
}

/// Türkiye GSM numarasını 10 haneye (5XXXXXXXXX) çevirir; geçersizse None.
/// (Python `\D` Unicode rakamlarını da tutardı; burada yalnızca ASCII rakam sayılır.)
pub fn normalize_phone(raw: &str) -> Option<String> {
    let mut d: String = raw.chars().filter(char::is_ascii_digit).collect();
    if d.len() == 12 && d.starts_with("90") {
        d = d[2..].to_string();
    } else if d.len() == 11 && d.starts_with('0') {
        d = d[1..].to_string();
    }
    (d.len() == 10 && d.starts_with('5')).then_some(d)
}

/// Dış sağlayıcıya giden istek (NetGSM, Twilio, GitHub, liste kaynakları). `Debug` bilerek yok: sır taşır.
pub struct DisIstek {
    pub yontem: String,
    pub url: String,
    pub basliklar: Vec<(String, String)>,
    pub govde: Option<Vec<u8>>,
    /// (kullanıcı, parola): komut satırına değil yapılandırmaya (stdin) yazılır.
    pub kullanici: Option<(String, String)>,
    pub zaman_asimi: Duration,
    pub okuma_siniri: usize,
}

/// Dış istek taşıyıcısı. HTTP 4xx/5xx hata değil `(durum, gövde)` döner; ağ hatası `Err`.
/// İstek BİTMEDEN dönülmez: sağlayıcı SMS'i gönderdiyse cevap (iş no) kaybolmasın.
pub trait Tasiyici: Send + Sync {
    fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String>;
}

/// Gerçek taşıyıcı: curl. URL, başlıklar, kimlik ve gövde standart girdideki yapılandırmadan gelir (`-K -`):
/// sırlar komut satırında ve süreç listesinde görünmez.
pub struct Curl;

fn curl_alinti(d: &str) -> Result<String, String> {
    let mut o = String::from("\"");
    for c in d.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if c.is_control() => return Err("curl: denetim karakteri".into()),
            c => o.push(c),
        }
    }
    o.push('"');
    Ok(o)
}

/// Başlık/URL/kimlik değerleri tek satır olmalı (satır sokma = yapılandırma enjeksiyonu).
fn tek_satir(d: &str) -> Result<String, String> {
    if d.chars().any(char::is_control) {
        return Err("curl: denetim karakteri".into());
    }
    curl_alinti(d)
}

impl Curl {
    /// stdin'e verilecek curl yapılandırması (testte doğrulanır).
    pub(crate) fn yapilandirma(i: &DisIstek) -> Result<String, String> {
        if i.yontem.is_empty() || !i.yontem.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err("curl: yöntem".into());
        }
        let sn = i.zaman_asimi.as_secs().max(1);
        let mut c = String::new();
        c.push_str(&format!("url = {}\n", tek_satir(&i.url)?));
        c.push_str("proto = \"=http,https\"\n");
        c.push_str(&format!("request = \"{}\"\n", i.yontem));
        c.push_str(&format!("connect-timeout = {sn}\n"));
        for (ad, deger) in &i.basliklar {
            c.push_str(&format!("header = {}\n", tek_satir(&format!("{ad}: {deger}"))?));
        }
        if let Some((k, p)) = &i.kullanici {
            c.push_str(&format!("user = {}\n", tek_satir(&format!("{k}:{p}"))?));
        }
        if let Some(g) = &i.govde {
            // data-raw: '@' ile başlayan gövde dosya okumaz
            let g = std::str::from_utf8(g).map_err(|_| "curl: gövde UTF-8 değil".to_string())?;
            c.push_str(&format!("data-raw = {}\n", curl_alinti(g)?));
        }
        Ok(c)
    }

    pub(crate) fn calistir(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String> {
        let yap = Self::yapilandirma(i)?;
        let mut cocuk = Command::new("curl")
            .args(["-sS", "--max-time", &i.zaman_asimi.as_secs().max(1).to_string(), "-w", "%{http_code}", "-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("curl: {e}"))?;
        let mut stdin = cocuk.stdin.take().ok_or("curl stdin")?;
        let yazici = std::thread::spawn(move || {
            let _ = stdin.write_all(yap.as_bytes());
        });
        let mut hata = cocuk.stderr.take().ok_or("curl stderr")?;
        let hata_okuyucu = std::thread::spawn(move || {
            let mut t = Vec::new();
            let _ = (&mut hata).take(4096).read_to_end(&mut t);
            // kalanı at (boru dolup curl'ü bekletmesin)
            let _ = std::io::copy(&mut hata, &mut std::io::sink());
            t
        });
        // Gövdenin ilk `okuma_siniri` baytı tutulur, fazlası okunup atılır; durum kodu (3 hane) akışın sonundadır.
        let mut cikti = cocuk.stdout.take().ok_or("curl stdout")?;
        let mut bas: Vec<u8> = Vec::new();
        let mut son: Vec<u8> = Vec::new();
        let mut toplam: usize = 0;
        let mut tmp = [0u8; 8192];
        loop {
            let n = cikti.read(&mut tmp).map_err(|e| format!("curl: {e}"))?;
            if n == 0 {
                break;
            }
            if bas.len() < i.okuma_siniri {
                let al = n.min(i.okuma_siniri - bas.len());
                bas.extend_from_slice(&tmp[..al]);
            }
            son.extend_from_slice(&tmp[..n]);
            if son.len() > 3 {
                son.drain(..son.len() - 3);
            }
            toplam += n;
        }
        let durum = cocuk.wait().map_err(|e| format!("curl: {e}"))?;
        let _ = yazici.join();
        let hata = hata_okuyucu.join().unwrap_or_default();
        if !durum.success() {
            let m = String::from_utf8_lossy(&hata);
            let m: String = m.trim().chars().take(200).collect();
            return Err(format!("curl: {m}"));
        }
        if toplam < 3 {
            return Err("curl: durum kodu yok".into());
        }
        let kod: u16 = std::str::from_utf8(&son).ok().and_then(|s| s.parse().ok()).ok_or("curl: durum kodu")?;
        bas.truncate((toplam - 3).min(i.okuma_siniri));
        Ok((kod, bas))
    }
}

impl Tasiyici for Curl {
    fn gonder(&self, i: &DisIstek) -> Result<(u16, Vec<u8>), String> {
        if std::env::var("WC_TEST").as_deref() == Ok("1") {
            return Err("WC_TEST".into());
        }
        self.calistir(i)
    }
}

/// Python doğruluk değeri (`bool(x)`): null/false/0/""/[]/{} yanlıştır.
pub fn py_dogru(v: &serde_json::Value) -> bool {
    use serde_json::Value;
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python `str(x)` (düz değerler için yeterince yakın): metin olduğu gibi, null "None", doğru/yanlış "True"/"False".
pub fn py_str(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::String(s) => s.clone(),
        d => d.to_string(),
    }
}

/// Python `str(d.get(k) or "")`.
pub fn py_veya_bos(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(x) if py_dogru(x) => py_str(x),
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SAYAC: AtomicU32 = AtomicU32::new(0);

    /// Her çağrıda yeni, boş geçici dizin.
    pub(crate) fn gecici_dizin() -> PathBuf {
        let d = std::env::temp_dir().join(format!("wfc-merkez-test-{}-{}", std::process::id(), SAYAC.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn now_iso_ve_gun_sinirlari() {
        assert_eq!(now_iso(1_790_000_000.0), "2026-09-21T17:13:20+03:00");
        assert_eq!(now_iso(1_793_825_999.0), "2026-11-04T23:59:59+03:00");
        assert_eq!(now_iso(1_793_826_000.0), "2026-11-05T00:00:00+03:00");
        assert_eq!(now_iso(1_793_825_999.4), "2026-11-04T23:59:59+03:00");
        assert_eq!(now_iso(0.0), "1970-01-01T03:00:00+03:00");
    }

    #[test]
    fn tarih_goster_ornekleri() {
        assert_eq!(tarih_goster("2026-10-12", false), "12.10.2026");
        assert_eq!(tarih_goster("2026-10-12T17:38:05+03:00", false), "12.10.2026 17:38");
        assert_eq!(tarih_goster("2026-10-12T17:38:05+03:00", true), "12.10.2026 17:38:05");
        assert_eq!(tarih_goster("2026-10-12 17:38", false), "12.10.2026 17:38");
        assert_eq!(tarih_goster("2026-10-12 17:38", true), "12.10.2026 17:38");
        assert_eq!(tarih_saat_goster("2026-10-12T17:38:05+03:00"), "12.10.2026 17:38:05");
        assert_eq!(tarih_goster(" 2026-10-12 ", false), "12.10.2026");
        // çözülemeyenler aynen
        assert_eq!(tarih_goster("", false), "");
        assert_eq!(tarih_goster("abc", false), "abc");
        assert_eq!(tarih_goster("2026-10-12 junk", false), "2026-10-12 junk");
        assert_eq!(tarih_goster("2026-10-12T17", false), "2026-10-12T17");
        assert_eq!(tarih_goster("12.10.2026", false), "12.10.2026");
    }

    #[test]
    fn tarih_girdi_ornekleri() {
        assert_eq!(tarih_girdi("12.10.2026"), "2026-10-12");
        assert_eq!(tarih_girdi("1.9.2026"), "2026-09-01");
        assert_eq!(tarih_girdi("12.10.2026 17:38"), "2026-10-12 17:38");
        assert_eq!(tarih_girdi("12.10.2026   17:38"), "2026-10-12 17:38");
        assert_eq!(tarih_girdi("2026-10-12"), "2026-10-12");
        assert_eq!(tarih_girdi("  x  "), "x");
        assert_eq!(tarih_girdi("123.10.2026"), "123.10.2026");
        assert_eq!(tarih_girdi("12.10.20261"), "12.10.20261");
        assert_eq!(tarih_girdi("12.10.2026x"), "12.10.2026x");
        assert_eq!(tarih_girdi("12.10.2026 a\nb"), "12.10.2026 a\nb");
        assert_eq!(tarih_girdi(""), "");
    }

    #[test]
    fn telefon_normallesir() {
        assert_eq!(normalize_phone("+90 532 123 45 67").as_deref(), Some("5321234567"));
        assert_eq!(normalize_phone("0532 123 45 67").as_deref(), Some("5321234567"));
        assert_eq!(normalize_phone("5321234567").as_deref(), Some("5321234567"));
        assert_eq!(normalize_phone("905321234567").as_deref(), Some("5321234567"));
        assert_eq!(normalize_phone("(0532) 123-45-67").as_deref(), Some("5321234567"));
        assert_eq!(normalize_phone("4321234567"), None);
        assert_eq!(normalize_phone("532123456"), None);
        assert_eq!(normalize_phone("53212345678"), None);
        assert_eq!(normalize_phone(""), None);
        assert_eq!(normalize_phone("abc"), None);
    }

    #[test]
    fn read_rows_bom_gz_ve_eksik_dosya() {
        let d = gecici_dizin();
        assert!(read_rows(&d.join("yok.csv")).is_empty());
        let icerik = "\u{feff}zaman;olay;ad\r\n2026-11-04;GIRIS;Ayşe \"A\"\r\n\r\n2026-11-05;\"CIKIS;x\";\"a\"\"b\"\r\nkisa\r\n";
        std::fs::write(d.join("a.csv"), icerik).unwrap();
        let r = read_rows(&d.join("a.csv"));
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["zaman"], "2026-11-04");
        assert_eq!(r[0]["ad"], "Ayşe \"A\"");
        assert_eq!(r[1]["olay"], "CIKIS;x");
        assert_eq!(r[1]["ad"], "a\"b");
        assert_eq!(r[2]["zaman"], "kisa");
        assert!(!r[2].contains_key("olay"));
        // gzip
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(icerik.as_bytes()).unwrap();
        std::fs::write(d.join("a.csv.gz"), gz.finish().unwrap()).unwrap();
        assert_eq!(read_rows(&d.join("a.csv.gz")), r);
        // yalnızca başlık
        std::fs::write(d.join("b.csv"), "a;b\n").unwrap();
        assert!(read_rows(&d.join("b.csv")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn save_json_girinti_bir_ve_kacissiz() {
        let d = gecici_dizin();
        let v = serde_json::json!({"ad": "Müşteri", "l": [1, 2], "e": {}, "i": {"a": null}});
        let yol = d.join("x.json");
        save_json(&yol, &v).unwrap();
        let s = std::fs::read_to_string(&yol).unwrap();
        // Python: json.dumps(v, ensure_ascii=False, indent=1) ile aynı
        assert_eq!(s, "{\n \"ad\": \"Müşteri\",\n \"l\": [\n  1,\n  2\n ],\n \"e\": {},\n \"i\": {\n  \"a\": null\n }\n}");
        assert!(!d.join("x.json.tmp").exists());
        save_json(&yol, &serde_json::json!([])).unwrap();
        assert_eq!(std::fs::read_to_string(&yol).unwrap(), "[]");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn istek() -> DisIstek {
        DisIstek {
            yontem: "POST".into(),
            url: "https://api.example.com/x".into(),
            basliklar: vec![("Content-Type".into(), "application/xml".into())],
            govde: Some(b"<a>\"s\\\"\n</a>".to_vec()),
            kullanici: Some(("AC123".into(), "gizli-parola".into())),
            zaman_asimi: Duration::from_secs(5),
            okuma_siniri: 16384,
        }
    }

    #[test]
    fn curl_yapilandirmasi_sirlari_icerir_ama_komut_satirinda_degil() {
        let y = Curl::yapilandirma(&istek()).unwrap();
        assert!(y.contains("url = \"https://api.example.com/x\"\n"));
        assert!(y.contains("request = \"POST\"\n"));
        assert!(y.contains("header = \"Content-Type: application/xml\"\n"));
        assert!(y.contains("user = \"AC123:gizli-parola\"\n"));
        assert!(y.contains("data-raw = \"<a>\\\"s\\\\\\\"\\n</a>\"\n"));
        assert!(y.contains("proto = \"=http,https\"\n"));
    }

    #[test]
    fn curl_yapilandirmasi_enjeksiyonu_reddeder() {
        let mut i = istek();
        i.basliklar.push(("X".into(), "a\nurl = \"file:///etc/passwd\"".into()));
        assert!(Curl::yapilandirma(&i).is_err());
        let mut i = istek();
        i.url = "https://a/\nrequest = \"X\"".into();
        assert!(Curl::yapilandirma(&i).is_err());
        let mut i = istek();
        i.yontem = "POST\n".into();
        assert!(Curl::yapilandirma(&i).is_err());
        let mut i = istek();
        i.govde = Some(vec![0, 1]);
        assert!(Curl::yapilandirma(&i).is_err());
        let mut i = istek();
        i.govde = Some(b"@/etc/passwd".to_vec()); // data-raw: dosya okumaz, düz metin
        assert!(Curl::yapilandirma(&i).unwrap().contains("data-raw = \"@/etc/passwd\""));
    }

    #[test]
    fn wc_test_curl_calistirmaz() {
        // gonder(), WC_TEST=1 iken curl'e hiç uğramadan Err("WC_TEST") döner
        let onceki = std::env::var("WC_TEST").ok();
        std::env::set_var("WC_TEST", "1");
        let s = Curl.gonder(&istek());
        match onceki {
            Some(v) => std::env::set_var("WC_TEST", v),
            None => std::env::remove_var("WC_TEST"),
        }
        assert_eq!(s.err().as_deref(), Some("WC_TEST"));
    }

    /// Bir HTTP isteğini (başlıklar + Content-Length kadar gövde) okur, `cevap`ı yazar.
    fn istek_oku_cevapla(l: &std::net::TcpListener, cevap: &[u8]) -> String {
        let (mut s, _) = l.accept().unwrap();
        let mut ham = Vec::new();
        let mut tmp = [0u8; 4096];
        let (bas_son, uzunluk) = loop {
            let n = s.read(&mut tmp).unwrap();
            ham.extend_from_slice(&tmp[..n]);
            if let Some(p) = ham.windows(4).position(|w| w == b"\r\n\r\n") {
                let basliklar = String::from_utf8_lossy(&ham[..p]).to_lowercase();
                let u = basliklar
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (p + 4, u);
            }
            assert!(n > 0, "istek eksik");
        };
        while ham.len() < bas_son + uzunluk {
            let n = s.read(&mut tmp).unwrap();
            assert!(n > 0, "gövde eksik");
            ham.extend_from_slice(&tmp[..n]);
        }
        s.write_all(cevap).unwrap();
        String::from_utf8_lossy(&ham).to_string()
    }

    /// Gerçek curl, yalnızca 127.0.0.1 üzerindeki yerel dinleyiciye: gövde ve sırlar stdin'den gider, durum alınır, sınır işler.
    #[test]
    fn curl_yerel_dinleyiciyle_calisir() {
        use std::net::TcpListener;
        if Command::new("curl").arg("--version").output().is_err() {
            return;
        }
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let sunucu = std::thread::spawn(move || {
            let a = istek_oku_cevapla(&l, b"HTTP/1.0 403 Forbidden\r\nContent-Length: 5\r\n\r\nyasak");
            let b = istek_oku_cevapla(&l, b"HTTP/1.0 200 OK\r\n\r\n0123456789");
            vec![a, b]
        });
        let mut i = istek();
        i.url = format!("http://127.0.0.1:{port}/yol");
        let govde = "{\"ad\":\"Ayşe\",\n\"x\":\"\\\"\"}";
        i.govde = Some(govde.as_bytes().to_vec());
        let (kod, g) = Curl.calistir(&i).unwrap();
        assert_eq!((kod, g.as_slice()), (403, &b"yasak"[..]));
        i.okuma_siniri = 4;
        let (kod, g) = Curl.calistir(&i).unwrap();
        assert_eq!((kod, g.as_slice()), (200, &b"0123"[..]));
        let alinan = sunucu.join().unwrap();
        assert!(alinan[0].starts_with("POST /yol HTTP/1.1"));
        assert!(alinan[0].contains("Content-Type: application/xml"));
        assert!(alinan[0].to_lowercase().contains("authorization: basic "));
        assert!(alinan[0].ends_with(govde), "{}", alinan[0]);
        // bağlanılamayan yerel adres: Err
        let mut i = istek();
        i.url = "http://127.0.0.1:1/".into();
        assert!(Curl.calistir(&i).is_err());
    }
}
