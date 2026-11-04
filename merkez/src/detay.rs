//! Kayıt/arşiv ekranlarının HTML parçaları ve biçimlendirme (Python detay.py; detay spec §3). Sayfa çerçevesi `web.rs`'de.
//! Özetlerdeki cihazdan gelen tüm metin (alan adı, MAC, ek, ad) güvenilmezdir: hepsi `e()` ile kaçışlı basılır.
//! Kişi anahtarı (`telefon` sütunu): telefon (`905…`), `TC:<11 hane>` ya da `PASAPORT:<ISO2>:<NO>`. TC/pasaport ekranda
//! maskeli (cihazdaki src/kimlik.rs ile aynı), adreslerde/çapalarda kişi yalnızca opak kimlikle (HMAC) geçer.
//!
//! Python'dan küçük farklar: rakam/boşluk sınıfları ASCII (Python `\d` Unicode rakamları da sayardı; spec bilinçli fark 5
//! ile aynı yön), `isalpha` için Rust `is_alphabetic`; özet JSON'unda liste olması gereken alan başka türdeyse boş sayılır
//! (Python `TypeError` ile 500 verirdi; özetleri yalnızca root'un işi üretir).
use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Write;
use std::path::Path;

use chrono::{Datelike, NaiveDate};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::guvenlik::rastgele;
use crate::ortak::{self, py_bosluk, py_strip, py_veya_bos};

pub const KISA_GUN: [&str; 7] = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];
pub const GUN_ADI: [&str; 7] = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];
pub const AYLAR: [&str; 12] = ["Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık"];
pub const ARAMA_SINIRI: usize = 50;
pub const GUN_BAGLANTI_SINIRI: usize = 30;
pub const ARAMA_TURLERI: [(&str, &str); 2] = [("kisi", "Telefon, ad soyad, T.C. kimlik no ya da pasaport no"), ("oda", "Oda numarası")];

/// Arama dizini: Python `dict` (ekleme sırası korunur: eşit anahtarlı sonuçların sırası buna bağlı). Değerler JSON nesnesi.
pub type Dizin = Vec<(String, Value)>;

/// Python `html.escape(str(s), quote=True)`: `&` `<` `>` `"` `'`.
pub fn e(s: impl Display) -> String {
    let s = s.to_string();
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#x27;"),
            c => o.push(c),
        }
    }
    o
}

/// Python `_int(n)`: `int(n)`, olmazsa 0 (JSON değeri; metin Python `int()` kurallarıyla, ondalık sayı kırpılır).
pub fn int_of(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u.min(i64::MAX as u64) as i64))
            .unwrap_or_else(|| n.as_f64().map(|f| f as i64).unwrap_or(0)),
        Some(Value::String(s)) => crate::merkez::py_int(s).unwrap_or(0),
        Some(Value::Bool(b)) => *b as i64,
        _ => 0,
    }
}

/// `YYYY-AA-GG`, ASCII rakam, gerçek bir takvim günü (Python `date.fromisoformat`: yıl 1–9999).
pub fn gun_coz(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let a: u32 = s[5..7].parse().ok()?;
    let g: u32 = s[8..10].parse().ok()?;
    if y < 1 {
        return None;
    }
    NaiveDate::from_ymd_opt(y, a, g)
}

pub fn gecerli_gun(s: &str) -> bool {
    gun_coz(s).is_some()
}

pub fn boyut(n: i64) -> String {
    let mut n = n as f64;
    for birim in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || birim == "GB" {
            return if birim == "B" { format!("{n:.0} B") } else { format!("{n:.1} {birim}").replace('.', ",") };
        }
        n /= 1024.0;
    }
    unreachable!()
}

fn saat_deger(s: &str) -> String {
    let t: String = s.chars().skip(11).take(5).collect();
    let b = t.as_bytes();
    if b.len() == 5 && b[2] == b':' && [0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit()) {
        t
    } else {
        String::new()
    }
}

/// ISO zamandan HH:MM; cihazdan gelen alan biçimsizse boş (çıktı hiçbir zaman işaretleme taşımaz).
pub fn saat(z: &str) -> String {
    saat_deger(z)
}

/// `saat` JSON değeri için (metin değilse boş).
pub fn saat_v(z: Option<&Value>) -> String {
    match z {
        Some(Value::String(s)) => saat_deger(s),
        _ => String::new(),
    }
}

pub fn sure(sn: i64) -> String {
    if sn >= 3600 {
        format!("{}:{:02}:{:02}", sn / 3600, sn % 3600 / 60, sn % 60)
    } else {
        format!("{}:{:02}", sn.div_euclid(60), sn.rem_euclid(60))
    }
}

pub fn tarih(g: &str) -> String {
    if gecerli_gun(g) {
        ortak::tarih_goster(g, false)
    } else {
        g.to_string()
    }
}

pub fn tr_kucuk(s: &str) -> String {
    s.replace('İ', "i").replace('I', "ı").to_lowercase()
}

/// Python `str.casefold()` yaklaşığı (ß → ss, ς → σ, sonra küçük harf).
pub fn py_casefold(s: &str) -> String {
    s.replace('ß', "ss").replace('ς', "σ").to_lowercase()
}

fn ad_soyad(k: &Value) -> String {
    let ad = py_strip(&format!("{} {}", py_veya_bos(k.get("ad")), py_veya_bos(k.get("soyad")))).to_string();
    if ad.is_empty() {
        "(adı yok)".into()
    } else {
        ad
    }
}

fn tablo(basliklar: &[&str], satirlar: &[Vec<String>], bos: &str) -> String {
    if satirlar.is_empty() {
        return format!("<p class=\"alt\">{}</p>", e(bos));
    }
    let bas: String = basliklar.iter().map(|b| format!("<th>{}</th>", e(b))).collect();
    let govde: String = satirlar
        .iter()
        .map(|s| format!("<tr>{}</tr>", s.iter().map(|h| format!("<td>{}</td>", e(h))).collect::<String>()))
        .collect();
    format!("<div class=\"kaydir\"><table class=\"detay\"><tr>{bas}</tr>{govde}</table></div>")
}

// --- kişi anahtarı: maske, opak kimlik, kimlik doğrulama (cihazdaki src/kimlik.rs'in karşılığı) ---

/// T.C. kimlik no: 11 hane, ilk hane 0 değil, 10. ve 11. kontrol haneleri tutar.
pub fn tc_gecerli(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 11 || !b.iter().all(u8::is_ascii_digit) {
        return false;
    }
    let d: Vec<i64> = b.iter().map(|c| (c - b'0') as i64).collect();
    if d[0] == 0 {
        return false;
    }
    let tek = d[0] + d[2] + d[4] + d[6] + d[8];
    let cift = d[1] + d[3] + d[5] + d[7];
    (tek * 7 - cift).rem_euclid(10) == d[9] && d[..10].iter().sum::<i64>() % 10 == d[10]
}

/// Boşluk/tire atılır, büyük harf; 5–20 karakter `[A-Z0-9]`. Değilse None.
pub fn pasaport_normal(s: &str) -> Option<String> {
    let t: String = s.chars().filter(|c| !py_bosluk(*c) && *c != '-').collect::<String>().to_uppercase();
    let n = t.chars().count();
    ((5..=20).contains(&n) && t.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then_some(t)
}

pub fn ayracsiz(s: &str) -> String {
    s.chars().filter(|c| !py_bosluk(*c) && !"-.,/;:".contains(*c)).collect()
}

/// TC `TC:12345678901` → `123******01`; pasaport `PASAPORT:DE:X1234567` → `DE X12***67` (ilk 3 + yıldız + son 2;
/// 6 karakterden kısa numarada yalnızca ilk karakter açık). Telefon anahtarı aynen. Biçime bakar, sağlamaya bakmaz.
pub fn maskele(anahtar: &str) -> String {
    if let Some(t) = anahtar.strip_prefix("TC:") {
        if t.len() == 11 && t.bytes().all(|b| b.is_ascii_digit()) {
            return format!("{}{}{}", &t[..3], "*".repeat(6), &t[9..]);
        }
        return anahtar.to_string();
    }
    if let Some(r) = anahtar.strip_prefix("PASAPORT:") {
        if let Some((uyruk, no)) = r.split_once(':') {
            if uyruk.len() == 2 && !no.is_empty() && no.is_ascii() {
                let (bas, son) = if no.len() >= 6 { (3, 2) } else { (1, 0) };
                return format!("{uyruk} {}{}{}", &no[..bas], "*".repeat(no.len() - bas - son), &no[no.len() - son..]);
            }
        }
    }
    anahtar.to_string()
}

/// Adres/çapa için kişi kimliği: HMAC-SHA256(sır, anahtar) ilk 16 hex (cihazdaki kimlik::opak ile aynı).
pub fn opak(sir: &[u8], anahtar: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(sir).expect("HMAC her uzunlukta anahtar alır");
    m.update(anahtar.as_bytes());
    let h: String = m.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    h[..16].to_string()
}

/// 32 baytlık sır: dosya yoksa O_EXCL ile 600 oluşturulur; varsa okunur; bozuksa (yarım yazım) yenisi atomik yazılır.
/// Python gibi her çağrıda önce 32 bayt üretilir (belirlenimli test kipinde çağrı sırası altın kayıtla aynı kalsın).
pub fn sir_oku_ya_uret(yol: &Path) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let b = rastgele::token_bytes(32);
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(yol) {
        Ok(mut f) => {
            f.write_all(&b)?;
            f.flush()?;
            f.sync_all()?;
            Ok(b)
        }
        Err(h) if h.kind() == std::io::ErrorKind::AlreadyExists => {
            let mevcut = std::fs::read(yol)?;
            if mevcut.len() == 32 {
                return Ok(mevcut);
            }
            let mut gecici = yol.as_os_str().to_owned();
            gecici.push(format!(".{}.yeni", std::process::id()));
            let gecici = std::path::PathBuf::from(gecici);
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&gecici)?;
            f.write_all(&b)?;
            f.flush()?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&gecici, yol)?;
            Ok(b)
        }
        Err(h) => Err(h),
    }
}

/// POST: aranan değer (telefon/kimlik) adrese ve tarayıcı geçmişine girmez.
pub fn arama_formu(q: &str, onek: &str, csrf: &str, tur: &str, uyruk: &str) -> String {
    let secenekler: String = ARAMA_TURLERI
        .iter()
        .map(|(d, ad)| format!("<option value=\"{d}\"{}>{}</option>", if *d == tur { " selected" } else { "" }, e(ad)))
        .collect();
    format!(
        "<form class=\"ara\" method=\"post\" action=\"{onek}/ara\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <input name=\"q\" value=\"{}\" maxlength=\"50\" placeholder=\"Telefon, ad soyad, T.C. no, pasaport ya da oda\" \
         aria-label=\"Kişi ara\" autocomplete=\"off\">\
         <select name=\"tur\" aria-label=\"Arama türü\">{secenekler}</select>\
         <input class=\"uyruk\" name=\"uyruk\" value=\"{}\" maxlength=\"2\" placeholder=\"Uyruk\" \
         aria-label=\"Pasaport uyruğu (isteğe bağlı, ör. DE)\" autocomplete=\"off\"><button>Ara</button></form>",
        e(csrf),
        e(q),
        e(uyruk)
    )
}

/// Özeti olan günler aya göre açılır listelerde; en yeni ay açık. Geçersiz gün adları gösterilmez.
pub fn gunler_html(gunler: &[String], sayilar: &BTreeMap<String, Value>, onek: &str) -> String {
    let mut sirali: Vec<&String> = gunler.iter().filter(|g| gecerli_gun(g)).collect();
    sirali.sort_by(|a, b| b.cmp(a));
    let mut aylar: Vec<(String, Vec<&String>)> = Vec::new();
    for g in sirali {
        let ay = g[..7].to_string();
        match aylar.iter_mut().find(|(a, _)| *a == ay) {
            Some((_, l)) => l.push(g),
            None => aylar.push((ay, vec![g])),
        }
    }
    if aylar.is_empty() {
        return String::new();
    }
    let mut parcalar = String::new();
    for (i, (_, liste)) in aylar.iter().enumerate() {
        let ilk = gun_coz(liste[0]).expect("geçerli gün");
        let mut maddeler = String::new();
        for g in liste {
            let d = gun_coz(g).expect("geçerli gün");
            let n = match sayilar.get(g.as_str()) {
                Some(v) => format!(" <span class=\"alt\">· {} kişi</span>", int_of(Some(v))),
                None => String::new(),
            };
            maddeler.push_str(&format!(
                "<li><a href=\"{onek}/gun/{g}\">{} {}</a>{n}</li>",
                tarih(g),
                KISA_GUN[d.weekday().num_days_from_monday() as usize]
            ));
        }
        parcalar.push_str(&format!(
            "<details{}><summary>{} {} <span class=\"alt\">({} gün)</span></summary><ul class=\"gunler\">{maddeler}</ul></details>",
            if i == 0 { " open" } else { "" },
            AYLAR[ilk.month0() as usize],
            ilk.year(),
            liste.len()
        ));
    }
    format!("<div class=\"kart\"><h2>Günler</h2>{parcalar}</div>")
}

/// `x.get(ad) or []` içindeki nesneler (`isinstance(s, dict)`); liste olmayan alan boş sayılır.
fn nesneler(v: Option<&Value>) -> Vec<&Value> {
    match v {
        Some(Value::Array(a)) => a.iter().filter(|x| x.is_object()).collect(),
        _ => Vec::new(),
    }
}

/// Python `", ".join(map(str, k.get("mac") or []))` (metin verilmişse karakterleri tek tek).
fn mac_listesi(v: Option<&Value>) -> String {
    match v {
        Some(Value::Array(a)) => a.iter().map(ortak::py_str).collect::<Vec<_>>().join(", "),
        Some(Value::String(s)) => s.chars().map(String::from).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

pub fn kisi_html(k: &Value, sir: &[u8]) -> String {
    let tel = py_veya_bos(k.get("telefon"));
    let siteler = nesneler(k.get("siteler"));
    let baglantilar = nesneler(k.get("baglantilar"));
    let diger = int_of(k.get("diger_site_sayisi"));
    let toplam_bag = int_of(k.get("baglanti_sayisi"));
    let oturum_satirlari: Vec<Vec<String>> = nesneler(k.get("oturumlar"))
        .iter()
        .map(|o| {
            let olay = py_veya_bos(o.get("olay"));
            let olay = match olay.as_str() {
                "OTURUM_BASLA" => "Giriş".to_string(),
                "OTURUM_BITIS" => "Çıkış".to_string(),
                "OTURUM_IP_DEGISTI" => "IP değişti".to_string(),
                _ => olay,
            };
            vec![saat_v(o.get("zaman")), olay, py_veya_bos(o.get("ic_ip")), py_veya_bos(o.get("ek"))]
        })
        .collect();
    let oturum_t = tablo(&["Saat", "Olay", "İç IP", "Not"], &oturum_satirlari, "Bu gün oturum olayı yok (oturum daha önce başlamış).");
    let site_satirlari: Vec<Vec<String>> = siteler
        .iter()
        .map(|s| vec![py_veya_bos(s.get("alan")), int_of(s.get("sayi")).to_string(), saat_v(s.get("ilk")), saat_v(s.get("son"))])
        .collect();
    let mut site_t = tablo(&["Site", "Kaç kez", "İlk", "Son"], &site_satirlari, "Kayıt yok.");
    if diger != 0 {
        site_t.push_str(&format!("<p class=\"alt\">ve {diger} site daha</p>"));
    }
    let bag_satirlari: Vec<Vec<String>> = baglantilar
        .iter()
        .map(|b| {
            let hedef = format!("{}:{}", py_veya_bos(b.get("hedef_ip")), py_veya_bos(b.get("hedef_port")));
            vec![
                saat_v(b.get("zaman")),
                py_veya_bos(b.get("protokol")),
                hedef.trim_end_matches(':').to_string(),
                boyut(int_of(b.get("gonderilen"))),
                boyut(int_of(b.get("alinan"))),
                sure(int_of(b.get("sure_sn"))),
            ]
        })
        .collect();
    let bag_t = tablo(&["Saat", "Protokol", "Hedef", "Gönderilen", "İndirilen", "Süre"], &bag_satirlari, "Kayıt yok.");
    let toplam = int_of(k.get("gonderilen")).saturating_add(int_of(k.get("alinan")));
    let mac = mac_listesi(k.get("mac"));
    format!(
        "<details class=\"kisi\" id=\"k{}\"><summary><b>{}</b> · {} · {}–{} · {}</summary><div>\
         <p class=\"alt\">Cihaz: {} · {toplam_bag} bağlantı · gönderilen {}, indirilen {}</p>\
         <h3>Oturum</h3>{oturum_t}\
         <details><summary>Girdiği siteler ({})</summary><div>{site_t}</div></details>\
         <details><summary>Bağlantılar (en çok veri kullanan {} / toplam {toplam_bag})</summary>\
         <div>{bag_t}</div></details></div></details>",
        opak(sir, &tel),
        e(ad_soyad(k)),
        e(maskele(&tel)),
        saat_v(k.get("ilk")),
        saat_v(k.get("son")),
        boyut(toplam),
        e(if mac.is_empty() { "—" } else { &mac }),
        boyut(int_of(k.get("gonderilen"))),
        boyut(int_of(k.get("alinan"))),
        (siteler.len() as i64).saturating_add(diger),
        baglantilar.len()
    )
}

pub fn gun_icerik(gun: &str, ozet: &Value, sir: &[u8], onek: &str) -> String {
    let d = gun_coz(gun).expect("gun_icerik: geçerli gün");
    let kisiler = nesneler(ozet.get("kisiler"));
    let liste: String = kisiler.iter().map(|k| kisi_html(k, sir)).collect();
    let liste = if liste.is_empty() { "<p class=\"alt\">Bu gün kayıt yok.</p>".to_string() } else { liste };
    format!(
        "<p><a href=\"{onek}/\">← Özet</a></p><div class=\"kart\"><h1>{} {}</h1>\
         <p class=\"alt\">{} kişi · ayrıntı için kişiye dokunun</p></div>{liste}",
        tarih(gun),
        GUN_ADI[d.weekday().num_days_from_monday() as usize],
        kisiler.len()
    )
}

/// Harf yoksa rakamlar alınır; baştaki 90 (ardından 5 gelirse) ya da 0 atılır: 0533…, +90 533… → 533…
pub fn telefon_sorgusu(q: &str) -> Option<String> {
    if q.chars().any(char::is_alphabetic) {
        return None;
    }
    let mut d: String = q.chars().filter(char::is_ascii_digit).collect();
    if d.starts_with("90") && d[2..].starts_with('5') {
        d = d[2..].to_string();
    } else if d.starts_with('0') {
        d = d[1..].to_string();
    }
    (!d.is_empty()).then_some(d)
}

/// `[A-Z0-9-]{1,10}` (büyük harfe çevrilmiş, kırpılmış).
pub fn oda_normal(q: &str) -> Option<String> {
    let o = py_strip(q).to_uppercase();
    let n = o.chars().count();
    ((1..=10).contains(&n) && o.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')).then_some(o)
}

fn uyruk_normal(u: &str) -> Option<String> {
    let u = py_strip(u).to_uppercase();
    (u.chars().count() == 2 && u.is_ascii() && u.chars().all(|c| c.is_ascii_alphabetic())).then_some(u)
}

fn gunler_listesi(k: &Value) -> Vec<String> {
    match k.get("gunler") {
        Some(Value::Array(a)) => a.iter().filter_map(|g| g.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

/// tur=kisi: telefon biçimindeyse telefonlarda parça araması (bugünkü gibi), harfliyse ad soyadda (Türkçe
/// büyük/küçük harf duyarsız); ayrıca geçerli T.C. no (ayraçlar atılır) ya da pasaport no (+ isteğe bağlı uyruk) ile
/// tam eşleşme — kimlik numarasında parça araması yok. tur=oda: o odada açılmış misafir oturumları (yalnızca o günler).
/// En son geleni önce, 50 kişi.
pub fn ara_sonuclari(dizin: &Dizin, q: &str, tur: &str, uyruk: &str) -> Vec<(String, Value)> {
    let mut bulunan: Vec<(String, Value)> = Vec::new();
    if tur == "oda" {
        let oda = oda_normal(q);
        for (tel, k) in dizin {
            let gunler = match (&oda, k.get("odalar")) {
                (Some(o), Some(Value::Object(odalar))) => odalar.get(o),
                _ => None,
            };
            if let Some(Value::Array(g)) = gunler {
                if !g.is_empty() {
                    let mut gunler: Vec<String> = g.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                    gunler.sort_by(|a, b| b.cmp(a));
                    let mut kk = k.as_object().cloned().unwrap_or_default();
                    kk.insert("gunler".into(), Value::Array(gunler.into_iter().map(Value::String).collect()));
                    bulunan.push((tel.clone(), Value::Object(kk)));
                }
            }
        }
    } else {
        let rakam = telefon_sorgusu(q);
        let tc = ayracsiz(q);
        let pas = pasaport_normal(q);
        let tc = tc_gecerli(&tc).then_some(tc);
        let u = uyruk_normal(uyruk);
        let uyruk_gecersiz = !py_strip(uyruk).is_empty() && u.is_none();
        let q_kucuk = tr_kucuk(q);
        let ad_eslesir = |k: &Value| -> bool {
            rakam.is_none() && tr_kucuk(&format!("{} {}", py_veya_bos(k.get("ad")), py_veya_bos(k.get("soyad")))).contains(&q_kucuk)
        };
        let eslesir = |tel: &str, k: &Value| -> bool {
            if tel.starts_with("TC:") {
                return tc.as_ref().is_some_and(|t| tel == format!("TC:{t}")) || ad_eslesir(k);
            }
            if let Some(r) = tel.strip_prefix("PASAPORT:") {
                let (ku, no) = r.split_once(':').unwrap_or((r, ""));
                return (pas.as_ref().is_some_and(|p| no == p) && !uyruk_gecersiz && u.as_ref().is_none_or(|u| ku == u)) || ad_eslesir(k);
            }
            match &rakam {
                Some(r) => tel.contains(r.as_str()),
                None => ad_eslesir(k),
            }
        };
        for (tel, k) in dizin {
            if k.is_object() && eslesir(tel, k) {
                bulunan.push((tel.clone(), k.clone()));
            }
        }
    }
    // en yeni gün önce; eşitlerde ekleme sırası (Python sort(reverse=True) kararlı)
    let anahtar = |k: &Value| gunler_listesi(k).into_iter().max().unwrap_or_default();
    bulunan.sort_by(|a, b| anahtar(&b.1).cmp(&anahtar(&a.1)));
    bulunan.truncate(ARAMA_SINIRI);
    bulunan
}

/// Aranan değer bir T.C./pasaport numarası olabilir mi: öyleyse sonuç sayfasında forma geri yazılmaz.
pub fn kimlik_sorgusu_mu(q: &str, dizin: &Dizin) -> bool {
    if tc_gecerli(&ayracsiz(q)) {
        return true;
    }
    let Some(pas) = pasaport_normal(q) else { return false };
    pas.chars().any(char::is_alphabetic) || dizin.iter().any(|(t, _)| t.starts_with("PASAPORT:") && t.ends_with(&format!(":{pas}")))
}

/// q: forma geri yazılacak değer (kimlik aramasında boş verilir). sonuclar None: arama yapılmadı (mesaj varsa o).
#[allow(clippy::too_many_arguments)]
pub fn ara_icerik(q: &str, sonuclar: Option<&[(String, Value)]>, sir: &[u8], onek: &str, csrf: &str, tur: &str, uyruk: &str, mesaj: &str) -> String {
    let govde = match sonuclar {
        None => {
            if mesaj.is_empty() {
                String::new()
            } else {
                format!("<p class=\"alt\">{}</p>", e(mesaj))
            }
        }
        Some([]) => "<p class=\"alt\">Sonuç yok.</p>".to_string(),
        Some(s) => {
            let mut parcalar = String::new();
            for (tel, k) in s {
                let gunler: Vec<String> = gunler_listesi(k).into_iter().filter(|g| gecerli_gun(g)).collect();
                let capa = opak(sir, tel);
                let mut linkler = gunler
                    .iter()
                    .take(GUN_BAGLANTI_SINIRI)
                    .map(|g| format!("<a href=\"{onek}/gun/{g}#k{capa}\">{}</a>", tarih(g)))
                    .collect::<Vec<_>>()
                    .join(", ");
                if gunler.len() > GUN_BAGLANTI_SINIRI {
                    linkler.push_str(&format!(" <span class=\"alt\">ve {} gün daha</span>", gunler.len() - GUN_BAGLANTI_SINIRI));
                }
                parcalar.push_str(&format!("<div class=\"kart\"><b>{}</b> · {}<p>{linkler}</p></div>", e(ad_soyad(k)), e(maskele(tel))));
            }
            parcalar
        }
    };
    format!(
        "<p><a href=\"{onek}/\">← Özet</a></p><div class=\"kart\"><h1>Kişi ara</h1>{}\
         <p class=\"alt\">T.C. kimlik ve pasaport numarasıyla yalnızca tam numara aranır; sonuçlarda kimlikler maskelidir.</p>\
         </div>{govde}",
        arama_formu(q, onek, csrf, tur, uyruk)
    )
}

#[cfg(test)]
mod testler {
    use super::*;
    use crate::altin::Kayit;
    use crate::test_yardim::gecici_dizin;
    use serde_json::json;

    const A: &str = "5330000001";
    const TC: &str = "TC:10000000146";
    const TC2: &str = "TC:11111111110";
    const PAS: &str = "PASAPORT:DE:X1234567";

    /// Python `kisi(tel, ad, soyad, **ek)`.
    fn kisi(tel: &str, ad: &str, soyad: &str, ek: Value) -> Value {
        let mut k = json!({"telefon": tel, "ad": ad, "soyad": soyad, "mac": ["aa:aa:aa:aa:aa:01"], "oturumlar": [],
            "gonderilen": 1536, "alinan": 1048576, "baglanti_sayisi": 1, "ilk": "2026-09-27T09:00:00+03:00",
            "son": "2026-09-27T17:05:00+03:00", "siteler": [], "diger_site_sayisi": 0, "baglantilar": []});
        for (a, v) in ek.as_object().unwrap() {
            k[a] = v.clone();
        }
        k
    }

    fn dizin_of(v: &[(&str, Value)]) -> Dizin {
        v.iter().map(|(t, k)| (t.to_string(), k.clone())).collect()
    }

    fn teller(r: &[(String, Value)]) -> Vec<&str> {
        r.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn kacis_python_gibi() {
        assert_eq!(e("<a href=\"x\">'&'</a>"), "&lt;a href=&quot;x&quot;&gt;&#x27;&amp;&#x27;&lt;/a&gt;");
        assert_eq!(e(42), "42");
        assert_eq!(e("ç ö ü"), "ç ö ü");
    }

    #[test]
    fn bicimler() {
        let _k = Kayit::bas("test_wc_detay", "test_bicimler");
        let xs = Value::String("x".into());
        assert!(boyut(0) == "0 B" && boyut(1536) == "1,5 KB" && boyut(1048576) == "1,0 MB" && boyut(int_of(Some(&xs))) == "0 B");
        assert_eq!(boyut(1280), "1,2 KB"); // Python f"{1.25:.1f}" = 1.2 (yarım çifte yuvarlanır)
        assert!(saat("2026-09-27T09:05:00+03:00") == "09:05" && saat("").is_empty() && saat_v(None).is_empty());
        assert!(sure(59) == "0:59" && sure(3725) == "1:02:05");
        assert!(tarih("2026-09-27") == "27.09.2026" && tarih("bozuk") == "bozuk");
        assert!(gecerli_gun("2026-09-27") && !gecerli_gun("2026-13-45") && !gecerli_gun("../x") && !gecerli_gun("0000-01-01"));
        assert_eq!(tr_kucuk("İLKER ŞAHIN"), "ilker şahın");
    }

    #[test]
    fn cihaz_metni_her_yerde_kacisli() {
        let _k = Kayit::bas("test_wc_detay", "test_cihaz_metni_her_yerde_kacisli");
        let kotu = "\"><script>x</script><img src=x ";
        let k = kisi(
            &format!("\"><img>{A}"),
            kotu,
            kotu,
            json!({"mac": [kotu], "ilk": "2026-09-27T<!-- x", "son": "2026-09-27T<img ",
                "oturumlar": [{"olay": kotu, "zaman": "2026-09-27T<u>9:0", "ic_ip": kotu, "ek": kotu}],
                "siteler": [{"alan": kotu, "sayi": 1, "ilk": "2026-09-27T<i>x", "son": ""}],
                "baglantilar": [{"zaman": "", "protokol": kotu, "hedef_ip": kotu, "hedef_port": kotu}]}),
        );
        let sonuc = [(format!("\"><img>{A}"), json!({"ad": kotu, "soyad": "", "gunler": ["2026-09-27"]}))];
        let g = kisi_html(&k, b"s") + &ara_icerik("x", Some(&sonuc), b"s", "", "", "kisi", "", "");
        for parca in ["<script", "<img", "<!--", "<u>", "<i>"] {
            assert!(!g.contains(parca), "{parca}");
        }
    }

    #[test]
    fn telefon_aramasi_bicimleri() {
        let _k = Kayit::bas("test_wc_detay", "test_telefon_aramasi_bicimleri");
        let dizin = dizin_of(&[(A, json!({"ad": "Ayşe", "soyad": "Yılmaz", "gunler": ["2026-09-27"]}))]);
        for sorgu in [A, "0533 000 0001", "+90 533 000 00 01", "905330000001", "05330000001", "533", "0533"] {
            assert_eq!(teller(&ara_sonuclari(&dizin, sorgu, "kisi", "")), vec![A], "{sorgu}");
        }
        assert!(!ara_sonuclari(&dizin, "ayşe", "kisi", "").is_empty() && ara_sonuclari(&dizin, "0544", "kisi", "").is_empty());
    }

    #[test]
    fn tarih_gosterimi_ve_girdisi() {
        let _k = Kayit::bas("test_wc_detay", "test_tarih_gosterimi_ve_girdisi");
        use crate::ortak::{tarih_girdi, tarih_goster, tarih_saat_goster};
        assert_eq!(tarih_goster("2026-10-12", false), "12.10.2026");
        assert_eq!(tarih_goster("2026-10-12T17:38:05+03:00", false), "12.10.2026 17:38");
        assert_eq!(tarih_goster("2026-10-12 17:38", false), "12.10.2026 17:38");
        assert_eq!(tarih_saat_goster("2026-10-12T17:38:05+03:00"), "12.10.2026 17:38:05");
        assert!(tarih_saat_goster("2026-10-12 17:38") == "12.10.2026 17:38" && tarih_saat_goster("2026-10-12") == "12.10.2026");
        assert_eq!(tarih_goster("", false), ""); // Python'daki None → "" Rust'ta boş metin
        for bozuk in ["dün", "2026-1-12", "12.10.2026", "2026-10-12X", "2026-10-12T17", "abcd-ef-gh", "٢٠٢٦-١٠-١٢"] {
            assert!(tarih_goster(bozuk, false) == bozuk && tarih_saat_goster(bozuk) == bozuk, "{bozuk}");
        }
        assert!(tarih_girdi("12.10.2026") == "2026-10-12" && tarih_girdi(" 1.9.2026 ") == "2026-09-01");
        assert_eq!(tarih_girdi("12.10.2026 17:38"), "2026-10-12 17:38");
        assert_eq!(tarih_girdi("2026-10-12"), "2026-10-12"); // eski yazım aynen
        for bozuk in ["dün", "12.10.26", "123.10.2026", "12.10", ""] {
            assert_eq!(tarih_girdi(bozuk), bozuk, "{bozuk}");
        }
    }

    #[test]
    fn gunler_ve_baslik_gun_ay_yil() {
        let _k = Kayit::bas("test_wc_detay", "test_gunler_ve_baslik_gun_ay_yil");
        let s = gunler_html(&["2026-09-30".to_string()], &BTreeMap::new(), "");
        assert!(s.contains(">30.09.2026 Çar<") && s.contains("href=\"/gun/2026-09-30\""));
        assert!(gun_icerik("2026-09-30", &json!({"kisiler": []}), b"s", "").contains("<h1>30.09.2026 Çarşamba</h1>"));
    }

    #[test]
    fn onek_butun_baglantilarda() {
        let _k = Kayit::bas("test_wc_detay", "test_onek_butun_baglantilarda");
        let on = "/m/100001/kayitlar";
        assert!(arama_formu("x", on, "", "kisi", "").contains(&format!("action=\"{on}/ara\"")));
        assert!(gunler_html(&["2026-09-30".to_string()], &BTreeMap::new(), on).contains(&format!("href=\"{on}/gun/2026-09-30\"")));
        assert!(gun_icerik("2026-09-30", &json!({"kisiler": []}), b"s", on).contains(&format!("href=\"{on}/\"")));
        let sonuc = [("5330000001".to_string(), json!({"ad": "Ayşe", "gunler": ["2026-09-30"]}))];
        let s = ara_icerik("ay", Some(&sonuc), b"s", on, "", "kisi", "", "");
        let capa = format!("k{}", opak(b"s", "5330000001"));
        assert!(s.contains(&format!("href=\"{on}/gun/2026-09-30#{capa}\"")) && s.contains(&format!("href=\"{on}/\"")) && s.contains(&format!("action=\"{on}/ara\"")));
        assert!(kisi_html(&json!({"telefon": "5330000001"}), b"s").contains(&format!("id=\"{capa}\"")) && !s.contains("#k5330000001"));
        assert!(!s.contains("href=\"/gun/") && !s.contains("href=\"/\""));
    }

    // --- test_misafir_kodu.py'den ---

    #[test]
    fn maske_cihazla_ayni() {
        let _k = Kayit::bas("test_misafir_kodu", "test_maske_cihazla_ayni");
        assert_eq!(maskele("TC:12345678901"), "123******01");
        assert_eq!(maskele(TC), "100******46");
        assert_eq!(maskele(PAS), "DE X12***67");
        assert_eq!(maskele("PASAPORT:GB:AB123"), "GB A****");
        assert!(maskele("905330000001") == "905330000001" && maskele("").is_empty());
    }

    #[test]
    fn opak_ve_sir_dosyasi() {
        let _k = Kayit::bas("test_misafir_kodu", "test_opak_ve_sir_dosyasi");
        let tmp = gecici_dizin();
        let yol = tmp.join("kisi_sir");
        let s = sir_oku_ya_uret(&yol).unwrap();
        assert!(s.len() == 32 && sir_oku_ya_uret(&yol).unwrap() == s); // ikinci çağrı aynı sırrı okur
        std::fs::write(&yol, b"kisa").unwrap(); // bozuk dosya yenilenir
        let s2 = sir_oku_ya_uret(&yol).unwrap();
        assert!(s2.len() == 32 && s2 != b"kisa");
        let a = opak(b"sir", TC);
        assert!(a.len() == 16 && a.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) && a == opak(b"sir", TC));
        assert!(a != opak(b"sir", TC2) && a != opak(b"baska", TC));
        // RFC 4231 test vektörü 1 (cihazdaki hmac ile aynı algoritma)
        assert_eq!(opak(&[0x0b; 20], "Hi There"), "b0344c61d8db3853");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&yol).unwrap().permissions().mode() & 0o777, 0o600);
    }

    /// `test_telefonlu_eski_arama_aynen`'in uygulamasız yarısı: `kur(tmp)` Musteri uygulamasını ister (Task 10'da, altın
    /// kayıtlı tam test olarak eklenir); burada yalnızca `ara_sonuclari` çağrıları.
    #[test]
    fn telefonlu_eski_arama_aynen_saf_islevler() {
        let gun = "2026-09-20";
        let tel = "905330000001";
        let dizin = dizin_of(&[
            (tel, json!({"ad": "Ayşe", "soyad": "Yılmaz", "gunler": [gun]})),
            (TC, json!({"ad": "Deniz", "soyad": "Kara", "gunler": [gun]})),
        ]);
        for sorgu in [tel, "0533 000 0001", "+90 533 000 00 01", "05330000001", "533", "0533", "0001"] {
            assert_eq!(teller(&ara_sonuclari(&dizin, sorgu, "kisi", "")), vec![tel], "{sorgu}");
        }
        assert_eq!(teller(&ara_sonuclari(&dizin, "ayşe", "kisi", "")), vec![tel]);
        assert!(ara_sonuclari(&dizin, "0544", "kisi", "").is_empty());
        assert_eq!(teller(&ara_sonuclari(&dizin, "kara", "kisi", "")), vec![TC]);
    }

    #[test]
    fn kimlik_ve_oda_aramasi() {
        let gun = "2026-09-20";
        let dizin = dizin_of(&[
            ("905330000001", json!({"ad": "Ayşe", "soyad": "Yılmaz", "gunler": [gun], "odalar": {}})),
            (TC, json!({"ad": "Deniz", "soyad": "Kara", "gunler": [gun], "odalar": {"101": [gun, "2026-09-18"]}})),
            (PAS, json!({"ad": "Hans", "soyad": "Muller", "gunler": [gun], "odalar": {"202": [gun]}})),
        ]);
        assert_eq!(teller(&ara_sonuclari(&dizin, "10000000146", "kisi", "")), vec![TC]);
        assert_eq!(teller(&ara_sonuclari(&dizin, "100-000-001.46", "kisi", "")), vec![TC]);
        assert!(ara_sonuclari(&dizin, "10000000147", "kisi", "").is_empty()); // kontrol hanesi bozuk
        assert!(!teller(&ara_sonuclari(&dizin, "000000", "kisi", "")).contains(&TC)); // kimlikte parça araması yok
        assert_eq!(teller(&ara_sonuclari(&dizin, "x1234567", "kisi", "")), vec![PAS]);
        assert_eq!(teller(&ara_sonuclari(&dizin, "X1234567", "kisi", "de")), vec![PAS]);
        assert!(ara_sonuclari(&dizin, "X1234567", "kisi", "FR").is_empty());
        assert!(ara_sonuclari(&dizin, "X1234567", "kisi", "D3").is_empty()); // geçersiz uyruk: kimlik eşleşmez
        assert!(ara_sonuclari(&dizin, "X12345", "kisi", "").is_empty());
        let r = ara_sonuclari(&dizin, "101", "oda", "");
        assert_eq!(teller(&r), vec![TC]);
        assert_eq!(r[0].1["gunler"], json!(["2026-09-20", "2026-09-18"]));
        assert_eq!(teller(&ara_sonuclari(&dizin, "202", "oda", "")), vec![PAS]);
        assert!(ara_sonuclari(&dizin, "1010", "oda", "").is_empty() && ara_sonuclari(&dizin, "10", "oda", "").is_empty());
        assert!(kimlik_sorgusu_mu("10000000146", &dizin) && kimlik_sorgusu_mu("X1234567", &dizin) && !kimlik_sorgusu_mu("ayşe", &dizin));
        assert!(!kimlik_sorgusu_mu("12345", &dizin));
    }

    #[test]
    fn arama_siniri_ve_esit_gun_sirasi() {
        let mut d: Dizin = Vec::new();
        for i in 0..60 {
            d.push((format!("90533000{i:04}"), json!({"ad": "x", "gunler": ["2026-09-20"]})));
        }
        d.push(("905330009999".into(), json!({"ad": "x", "gunler": ["2026-09-21"]})));
        let r = ara_sonuclari(&d, "533", "kisi", "");
        assert_eq!(r.len(), 50);
        assert_eq!(r[0].0, "905330009999"); // en yeni gün önce
        assert_eq!(r[1].0, "905330000000"); // eşit günlerde ekleme sırası
    }
}
