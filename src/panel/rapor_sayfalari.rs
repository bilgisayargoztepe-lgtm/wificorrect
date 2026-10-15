//! Panel: Ayarlar → Raporlar (spec 2026-10-14 "Ek (2026-10-15)"). Haftalık ve aylık raporlar merkezde üretilir; panel
//! istek anında merkezden (`/api/raporlar`, `/api/rapor`, cihaz anahtarıyla) hazır JSON'u alır ve müşteri panelindeki
//! (merkez raporlar.py) bölümlerle gösterir. Raporda kişisel veri yok: gerekçe istenmez, denetime yazılmaz.
//! Merkezin cevabı güvenilmez girdidir: her alan tür denetlenir, her metin kaçışlanır. JS yok; yazdırma CSS'i şablonda.
//! Merkezin `/api/*` sınırı (IP başına dakikada 20) için cevaplar bellekte birkaç dakika tutulur.

use super::*;
use serde_json::{json, Value};

/// Merkez çağrısının toplam süresi (sn): panel uzun beklemesin.
pub(super) const MERKEZ_SURE: u32 = 8;
const LISTE_SN: f64 = 5.0 * 60.0;
const KESIN_SN: f64 = 60.0 * 60.0;
const HATA_SN: f64 = 60.0;
const ONBELLEK_EN_COK: usize = 64;

pub(super) type Sonuc = Result<(u16, String), String>;

const AYLAR: [&str; 12] = ["Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık"];
const KISA_GUN: [&str; 7] = ["Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz"];
const GUN_ADI: [&str; 7] = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];
const DURUMLAR: [&str; 8] = ["tam", "kayit_yok", "eksik", "dogrulanamadi", "bekleniyor", "bugun", "gelecek", "hizmet_oncesi"];
const NEDEN: [(&str, &str); 5] = [
    ("okunamadi", "kayıt dosyaları okunamadı"),
    ("dosya", "bir dosyanın özeti (hash) MANIFEST'le tutmuyor"),
    ("zincir_yok", "gün mühür zincirinde yok"),
    ("zincir", "mühür zinciri satırı bozuk"),
    ("manifest", "MANIFEST zincirdeki özetle tutmuyor"),
];
const TAM_ACIKLAMA: &str = "Tam: kayıt geldi, mühürlü, merkezde yedekli, bütünlük (MANIFEST ve mühür zinciri) doğrulandı. \
                            Cihazın ilk kayıt gününden önceki günler sayılmaz.";
const ALINAMIYOR: &str = "Raporlar şu an alınamıyor (merkeze ulaşılamadı). Biraz sonra yeniden deneyin.";
const LISANS_KAPALI: &str = "Raporlar şu an alınamıyor: üyelik ya da lisans kapalı. Hizmet sağlayıcınıza başvurun.";
const BAGLI_DEGIL: &str = "Raporlar şu an alınamıyor: cihaz merkeze bağlı değil.";
const LISTE_EN_COK: usize = 100;

// --- tarih ---
fn ayin_gunleri(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// "YYYY-MM-DD" (ASCII rakam, geçerli takvim günü) → (yıl, ay, gün)
fn gun_coz(s: &str) -> Option<(i64, u32, u32)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return None;
    }
    let (y, m, d) = (s[..4].parse::<i64>().ok()?, s[5..7].parse::<u32>().ok()?, s[8..].parse::<u32>().ok()?);
    ((1..=12).contains(&m) && d >= 1 && d <= ayin_gunleri(y, m)).then_some((y, m, d))
}

/// Pazartesi = 0
fn hafta_gunu(y: i64, m: u32, d: u32) -> usize {
    // days_from_civil (1970-01-01 Perşembe)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let gun = era * 146_097 + doe - 719_468;
    (gun + 3).rem_euclid(7) as usize
}

fn gun_adi_of(g: &str, kisa: bool) -> &'static str {
    gun_coz(g).map_or("", |(y, m, d)| if kisa { KISA_GUN[hafta_gunu(y, m, d)] } else { GUN_ADI[hafta_gunu(y, m, d)] })
}

/// URL'deki tür ve dönem: hafta `YYYY-Www` (01–53), ay `YYYY-MM` (01–12); ASCII rakam, başka her şey geçersiz.
pub(super) fn donem_gecerli(tur: &str, donem: &str) -> bool {
    let b = donem.as_bytes();
    let rakam = |r: std::ops::Range<usize>| b.get(r).is_some_and(|x| x.iter().all(u8::is_ascii_digit));
    let (n, en) = match tur {
        "hafta" if b.len() == 8 && b[4] == b'-' && b[5] == b'W' && rakam(0..4) && rakam(6..8) => (&donem[6..8], 53),
        "ay" if b.len() == 7 && b[4] == b'-' && rakam(0..4) && rakam(5..7) => (&donem[5..7], 12),
        _ => return false,
    };
    n.parse::<u32>().is_ok_and(|n| (1..=en).contains(&n))
}

/// "/rapor/<tür>/<dönem>" → (tür, dönem); yalnızca doğrulanmış değerler.
pub(super) fn rapor_yolu(path: &str) -> Option<(&'static str, String)> {
    let (tur, donem) = path.strip_prefix("/rapor/")?.split_once('/')?;
    let tur = match tur {
        "hafta" => "hafta",
        "ay" => "ay",
        _ => return None,
    };
    donem_gecerli(tur, donem).then(|| (tur, donem.to_string()))
}

// --- biçimlendirme (merkez raporlar.py ile aynı) ---
/// Sayıdan sonra kesme işaretiyle gelen 3. tekil iyelik eki: 6'sı, 31'i, 4'ü, 10'u, 40'ı.
pub(super) fn iyelik(n: u64) -> &'static str {
    if n == 0 {
        return "ı";
    }
    if n % 10 != 0 {
        return ["i", "si", "ü", "ü", "i", "sı", "si", "i", "u"][(n % 10 - 1) as usize];
    }
    if n / 10 % 10 != 0 {
        return ["u", "si", "u", "ı", "si", "ı", "i", "i", "ı"][(n / 10 % 10 - 1) as usize];
    }
    if n / 100 % 10 != 0 { "ü" } else { "i" }
}

/// Yalnızca tamsayılardan kurulur (kaçış gerekmez).
pub(super) fn tam_metni(gun: u64, tam: u64) -> String {
    if gun == 0 { "Henüz biten gün yok".into() } else { format!("{gun} günün {tam}'{} tam", iyelik(tam)) }
}

/// Hafta: "12–18.10.2026", "29.09–05.10.2026", "28.12.2026–03.01.2027"; ay: "Ekim 2026".
pub(super) fn donem_adi(tur: &str, bas: &str, bit: &str) -> String {
    let (Some((by, bm, bd)), Some((sy, sm, sd))) = (gun_coz(bas), gun_coz(bit)) else {
        return format!("{bas}–{bit}");
    };
    if tur == "ay" {
        return format!("{} {by}", AYLAR[(bm - 1) as usize]);
    }
    if by != sy {
        format!("{bd:02}.{bm:02}.{by}–{sd:02}.{sm:02}.{sy}")
    } else if bm != sm {
        format!("{bd:02}.{bm:02}–{sd:02}.{sm:02}.{sy}")
    } else {
        format!("{bd:02}–{sd:02}.{sm:02}.{sy}")
    }
}

pub(super) fn sure_metni(sn: u64) -> String {
    let dk = sn / 60;
    if dk < 60 {
        format!("{dk} dk")
    } else if dk < 1440 {
        format!("{} sa {} dk", dk / 60, dk % 60)
    } else {
        format!("{} gün {} sa", dk / 1440, dk % 1440 / 60)
    }
}

pub(super) fn boyut(n: u64) -> String {
    let mut x = n as f64;
    for birim in ["B", "KB", "MB", "GB"] {
        if x < 1024.0 || birim == "GB" {
            return if birim == "B" { format!("{x:.0} B") } else { format!("{x:.1} {birim}").replace('.', ",") };
        }
        x /= 1024.0;
    }
    unreachable!()
}

// --- tür denetimi ---
fn sayi(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

fn metin(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").to_string()
}

fn gun(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|s| gun_coz(s).is_some()).map(String::from)
}

fn surum_gecerli(s: &str) -> bool {
    (1..=32).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || b".+~-".contains(&c))
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ListeOge {
    pub tur: &'static str,
    pub donem: String,
    pub bas: String,
    pub bit: String,
    pub kesin: bool,
    pub farkli: u64,
    pub kayit_gun: u64,
    pub kayit_tam: u64,
}

/// `/api/raporlar` cevabı → (liste, merkezin bugünü). Liste dizi değilse None.
pub(super) fn liste_coz(v: &Value) -> Option<(Vec<ListeOge>, Option<String>)> {
    let liste = v.get("raporlar")?.as_array()?;
    let ogeler = liste
        .iter()
        .take(LISTE_EN_COK)
        .filter_map(|x| {
            let tur = match x.get("tur")?.as_str()? {
                "hafta" => "hafta",
                "ay" => "ay",
                _ => return None,
            };
            let donem = x.get("donem")?.as_str()?;
            if !donem_gecerli(tur, donem) {
                return None;
            }
            Some(ListeOge {
                tur,
                donem: donem.into(),
                bas: gun(x.get("baslangic"))?,
                bit: gun(x.get("bitis"))?,
                kesin: x.get("kesin") == Some(&Value::Bool(true)),
                farkli: sayi(x.get("farkli")),
                kayit_gun: sayi(x.get("kayit_gun")),
                kayit_tam: sayi(x.get("kayit_tam")),
            })
        })
        .collect();
    Some((ogeler, gun(v.get("bugun"))))
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GunDurumu {
    pub gun: String,
    pub durum: &'static str,
    pub neden: &'static str,
    pub farkli: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Sorun {
    pub onem_kritik: bool,
    pub metin: String,
    pub acilis: String,
    pub kapanis: Option<String>,
    pub sure_sn: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Rapor {
    pub tur: &'static str,
    pub bas: String,
    pub bit: String,
    pub uretim: String,
    pub kesin: bool,
    /// farkli, yeni, donen, giris, giris_sms, giris_kod, izinli_cihaz
    pub misafir: [u64; 7],
    pub gunluk_ortalama: f64,
    pub en_yogun: Option<(String, u64)>,
    pub saat: Vec<u64>,
    pub hafta_gunu: Vec<u64>,
    pub gunler: Vec<GunDurumu>,
    pub kayit_gun: u64,
    pub kayit_tam: u64,
    pub gonderilen: u64,
    pub alinan: u64,
    pub sms: u64,
    pub isletme_ad: String,
    pub isletme_unvan: String,
    pub numara: String,
    pub cihaz_surum: Option<String>,
    pub cevrimdisi_sn: u64,
    pub sorunlar: Vec<Sorun>,
    pub bugun: Option<String>,
}

fn sayi_listesi(v: Option<&Value>, n: usize) -> Vec<u64> {
    let mut out: Vec<u64> = v.and_then(Value::as_array).map_or(vec![], |a| a.iter().take(n).map(|x| sayi(Some(x))).collect());
    out.resize(n, 0);
    out
}

/// `/api/rapor` cevabı → güvenli rapor; tür/dönem istekle tutmuyorsa ya da tarihler bozuksa None.
pub(super) fn rapor_coz(v: &Value, tur: &'static str, donem: &str) -> Option<Rapor> {
    let r = v.get("rapor")?;
    if r.get("tur")?.as_str()? != tur || r.get("donem")?.as_str()? != donem {
        return None;
    }
    let m = r.get("misafir").unwrap_or(&Value::Null);
    let ey = m.get("en_yogun_gun");
    let en_yogun = ey.and_then(|ey| Some((gun(ey.get("gun"))?, sayi(ey.get("farkli")))));
    let ort = m.get("gunluk_ortalama").and_then(Value::as_f64).filter(|x| x.is_finite() && *x >= 0.0).unwrap_or(0.0);
    let gunler = r.get("gunler").and_then(Value::as_array).map_or(vec![], |a| {
        a.iter()
            .filter_map(|g| {
                let d = g.get("durum").and_then(Value::as_str).unwrap_or("");
                let n = g.get("neden").and_then(Value::as_str).unwrap_or("");
                Some(GunDurumu {
                    gun: gun(g.get("gun"))?,
                    durum: DURUMLAR.iter().find(|x| **x == d).copied().unwrap_or("bilinmiyor"),
                    neden: NEDEN.iter().find(|(k, _)| *k == n).map_or("", |(k, _)| k),
                    farkli: g.get("farkli").and_then(Value::as_u64),
                })
            })
            .collect()
    });
    let (k, ku) = (r.get("kayit").unwrap_or(&Value::Null), r.get("kullanim").unwrap_or(&Value::Null));
    let isl = v.get("isletme").unwrap_or(&Value::Null);
    let ch = v.get("cihaz").unwrap_or(&Value::Null);
    let numara = metin(isl.get("numara"));
    let sorunlar = ch.get("sorunlar").and_then(Value::as_array).map_or(vec![], |a| {
        a.iter()
            .filter_map(|s| {
                let onem = s.get("onem")?.as_str()?;
                if onem != "kritik" && onem != "uyari" {
                    return None;
                }
                Some(Sorun {
                    onem_kritik: onem == "kritik",
                    metin: metin(s.get("metin")),
                    acilis: metin(s.get("acilis")),
                    kapanis: s.get("kapanis").and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from),
                    sure_sn: s.get("sure_sn").and_then(Value::as_u64),
                })
            })
            .collect()
    });
    Some(Rapor {
        tur,
        bas: gun(r.get("baslangic"))?,
        bit: gun(r.get("bitis"))?,
        uretim: metin(r.get("uretim")),
        kesin: r.get("kesin") == Some(&Value::Bool(true)),
        misafir: ["farkli", "yeni", "donen", "giris", "giris_sms", "giris_kod", "izinli_cihaz"].map(|a| sayi(m.get(a))),
        gunluk_ortalama: ort,
        en_yogun,
        saat: sayi_listesi(r.get("saat"), 24),
        hafta_gunu: sayi_listesi(r.get("hafta_gunu"), 7),
        gunler,
        kayit_gun: sayi(k.get("gun")),
        kayit_tam: sayi(k.get("tam")),
        gonderilen: sayi(ku.get("gonderilen")),
        alinan: sayi(ku.get("alinan")),
        sms: sayi(ku.get("sms")),
        isletme_ad: metin(isl.get("ad")),
        isletme_unvan: metin(isl.get("unvan")),
        numara: if crate::merkez::numara_gecerli(&numara) { numara } else { String::new() },
        cihaz_surum: ch.get("surum").and_then(Value::as_str).filter(|s| surum_gecerli(s)).map(String::from),
        cevrimdisi_sn: sayi(ch.get("cevrimdisi_sn")),
        sorunlar,
        bugun: gun(v.get("bugun")),
    })
}

// --- HTML ---
fn bolum(baslik: &str, govde: &str) -> String {
    format!("<section class=\"bolum\"><h2 class=\"bolum-baslik\">{}</h2>{govde}</section>", h(baslik))
}

pub(super) fn liste_html(liste: &[ListeOge], bugun: &str) -> String {
    if liste.is_empty() {
        return "<div class=\"rapor\"><p>Henüz rapor yok; ilk gün arşive geldiğinde oluşur.</p></div>".into();
    }
    let mut out = String::from(
        "<div class=\"rapor\"><p class=\"not\">Misafir sayıları, 5651 kayıt durumu ve cihazın çalışması. Raporlarda kişisel veri \
         (telefon, ad, MAC, IP) yoktur. Merkezde her gün 02:30 ve 12:30'da güncellenir.</p>",
    );
    for (tur, baslik) in [("hafta", "Haftalık raporlar"), ("ay", "Aylık raporlar")] {
        let rows: Vec<Vec<String>> = liste
            .iter()
            .filter(|x| x.tur == tur)
            .map(|x| {
                let etiket = if x.bit.as_str() >= bugun {
                    " <b class=\"uyarisi\">sürüyor</b>"
                } else if !x.kesin {
                    " <span class=\"bos\">kesinleşmedi</span>"
                } else {
                    ""
                };
                vec![
                    format!("<a href=\"/rapor/{tur}/{}\">{}</a>{etiket}", h(&x.donem), h(&donem_adi(tur, &x.bas, &x.bit))),
                    x.farkli.to_string(),
                    tam_metni(x.kayit_gun, x.kayit_tam),
                ]
            })
            .collect();
        out.push_str(&bolum(baslik, &table(&["Dönem", "Farklı misafir", "5651 kaydı"], &rows, "Bu türde rapor yok.")));
    }
    out.push_str("</div>");
    out
}

fn cubuklar(etiketler: &[String], sayilar: &[u64]) -> String {
    let en = sayilar.iter().copied().max().unwrap_or(0).max(1);
    let satirlar: String = etiketler
        .iter()
        .zip(sayilar)
        .map(|(et, n)| format!("<tr><td>{}</td><td class=\"cubuk\"><span style=\"width:{}%\"></span></td><td class=\"n\">{n}</td></tr>", h(et), n.saturating_mul(100) / en))
        .collect();
    format!("<div class=\"tablo\"><table class=\"cubuklar\"><tbody>{satirlar}</tbody></table></div>")
}

fn gun_durumu_html(g: &GunDurumu) -> String {
    match g.durum {
        "tam" => "<span class=\"iyi\">✔ Tam</span>".into(),
        "kayit_yok" => "<b class=\"kotu\">Kayıt gelmedi</b> <span class=\"bos\">(cihaz kapalı, bağlantısız ya da yedek gönderilmedi)</span>".into(),
        "eksik" => "<b class=\"kotu\">Eksik geldi</b> <span class=\"bos\">(günün dosyaları merkeze tam ulaşmadı)</span>".into(),
        "dogrulanamadi" => format!(
            "<b class=\"kotu\">Doğrulanamadı</b> <span class=\"bos\">({})</span>",
            h(NEDEN.iter().find(|(k, _)| *k == g.neden).map_or("neden bilinmiyor", |(_, v)| v))
        ),
        "bekleniyor" => "<span class=\"uyarisi\">Bekleniyor</span> <span class=\"bos\">(gece yedeğiyle gelir)</span>".into(),
        "bugun" => "Sürüyor".into(),
        "gelecek" => "<span class=\"bos\">—</span>".into(),
        "hizmet_oncesi" => "<span class=\"bos\">Hizmet öncesi (cihaz henüz kurulmamıştı; sayılmaz)</span>".into(),
        _ => "<span class=\"bos\">Bilinmiyor</span>".into(),
    }
}

/// (sayfa başlığı, gövde)
pub(super) fn rapor_html(r: &Rapor, bugun: &str) -> (String, String) {
    let ad = donem_adi(r.tur, &r.bas, &r.bit);
    let baslik = format!("{} · {ad}", if r.tur == "hafta" { "Haftalık rapor" } else { "Aylık rapor" });
    let durum = if r.bit.as_str() >= bugun {
        "<p class=\"uyarisi\">Dönem sürüyor — merkezde her gün 02:30 ve 12:30'da güncellenir.</p>"
    } else if !r.kesin {
        "<p class=\"not\">Geç gelen günlerle birkaç gün içinde değişebilir.</p>"
    } else {
        ""
    };
    let mut b = String::from(
        "<div class=\"rapor\"><p class=\"not yazdirma\"><a href=\"/raporlar\">← Raporlar</a> · Yazdırmak ya da PDF olarak kaydetmek için \
         Ctrl+P'ye basın (telefonda tarayıcı menüsünden Yazdır / Paylaş).</p>",
    );
    let bos_tire = |s: &str| if s.is_empty() { "—".to_string() } else { h(s) };
    b.push_str(&facts(&[
        ("İşletme", bos_tire(&r.isletme_ad)),
        ("Unvan", bos_tire(&r.isletme_unvan)),
        ("Müşteri no", bos_tire(&r.numara)),
        ("Dönem", h(&format!("{} – {}", ortak::tarih_goster(&r.bas), ortak::tarih_goster(&r.bit)))),
        ("Üretim", bos_tire(&ortak::tarih_goster(&r.uretim))),
    ]));
    b.push_str(durum);

    let gunler: Vec<Vec<String>> = r
        .gunler
        .iter()
        .map(|g| vec![h(&ortak::tarih_goster(&g.gun)), gun_adi_of(&g.gun, true).into(), gun_durumu_html(g), g.farkli.map_or(String::new(), |n| n.to_string())])
        .collect();
    b.push_str(&bolum(
        "5651 kayıt durumu",
        &format!(
            "<p><b>{}</b></p><p class=\"not\">{}</p>{}",
            tam_metni(r.kayit_gun, r.kayit_tam),
            h(TAM_ACIKLAMA),
            table(&["Tarih", "Gün", "Durum", "Misafir"], &gunler, "Bu dönemde gün yok.")
        ),
    ));

    let m = &r.misafir;
    let en_yogun = r.en_yogun.as_ref().map_or("—".to_string(), |(g, n)| format!("{} {} · {n} misafir", ortak::tarih_goster(g), gun_adi_of(g, false)));
    b.push_str(&bolum(
        "Misafirler",
        &(facts(&[
            ("Farklı misafir", m[0].to_string()),
            ("İlk kez gelen (yeni)", m[1].to_string()),
            ("Daha önce de gelmiş", m[2].to_string()),
            ("Toplam giriş", m[3].to_string()),
            ("Günlük ortalama", format!("{:.1}", r.gunluk_ortalama).replace('.', ",")),
            ("En yoğun gün", h(&en_yogun)),
        ]) + "<h3>Giriş yöntemi</h3>"
            + &facts(&[("SMS ile giriş", m[4].to_string()), ("Misafir kodu ile giriş", m[5].to_string()), ("İzinli cihaz (portalsız)", m[6].to_string())])),
    ));

    let saatler: Vec<String> = (0..24).map(|s| format!("{s:02}–{:02}", (s + 1) % 24)).collect();
    let gun_adlari: Vec<String> = GUN_ADI.iter().map(|s| s.to_string()).collect();
    b.push_str(&bolum(
        "Yoğunluk",
        &format!(
            "<p class=\"not\">Her satır: o saatte ya da günde internete çıkan farklı misafir.</p><h3>Saate göre</h3>{}<h3>Haftanın gününe göre</h3>{}",
            cubuklar(&saatler, &r.saat),
            cubuklar(&gun_adlari, &r.hafta_gunu)
        ),
    ));

    b.push_str(&bolum(
        "Kullanım",
        &facts(&[
            (
                "Toplam veri",
                h(&format!("{} (indirme {}, yükleme {})", boyut(r.gonderilen.saturating_add(r.alinan)), boyut(r.alinan), boyut(r.gonderilen))),
            ),
            ("Doğrulama SMS'i", r.sms.to_string()),
        ]),
    ));

    let sorunlar: Vec<Vec<String>> = r
        .sorunlar
        .iter()
        .map(|s| {
            vec![
                h(&s.metin),
                (if s.onem_kritik { "Kritik" } else { "Uyarı" }).into(),
                h(&ortak::tarih_goster(&s.acilis)),
                s.kapanis.as_ref().map_or("<b class=\"kotu\">açık</b>".into(), |k| h(&ortak::tarih_goster(k))),
                s.sure_sn.map_or("—".into(), sure_metni),
            ]
        })
        .collect();
    b.push_str(&bolum(
        "Cihaz",
        &format!(
            "{}<h3>Dönemde açılan sorunlar</h3>{}<p class=\"not\">Çevrimdışı süre merkezin cihaza ulaşamadığını fark ettiği andan sayılır; \
             gerçek süre biraz daha uzun olabilir.</p>",
            facts(&[
                ("Toplam çevrimdışı süre", if r.cevrimdisi_sn > 0 { format!("en az {}", sure_metni(r.cevrimdisi_sn)) } else { "Yok".into() }),
                ("Yazılım sürümü (dönem sonu)", r.cihaz_surum.as_deref().map_or("—".into(), h)),
            ]),
            table(&["Sorun", "Önem", "Açılış", "Kapanış", "Süre"], &sorunlar, "Bu dönemde açılan sorun yok.")
        ),
    ));
    b.push_str("</div>");
    (baslik, b)
}

// --- merkez ---
/// 200 cevabın önbellek süresi: liste 5 dk; kesinleşmiş geçmiş dönem 1 saat; süren dönem 5 dk.
fn sure_of(s: &Sonuc, ttl_200: f64) -> f64 {
    match s {
        Ok((200, _)) => ttl_200,
        Ok((404, _)) => LISTE_SN,
        _ => HATA_SN,
    }
}

impl Panel {
    /// Önbellekli merkez çağrısı. Anahtar müşteri numarasını içerir (bağ değişirse eski cevap kullanılmaz).
    fn rapor_cagir(&self, anahtar: &str, yol: &str, govde: &Value, now: f64, ttl: impl Fn(&Sonuc) -> f64) -> Sonuc {
        if let Some((bitis, s)) = self.rapor_onbellek.lock().unwrap_or_else(|e| e.into_inner()).get(anahtar) {
            if *bitis > now {
                return s.clone();
            }
        }
        let s = (self.rapor_http)(yol, &govde.to_string());
        let mut ob = self.rapor_onbellek.lock().unwrap_or_else(|e| e.into_inner());
        if ob.len() >= ONBELLEK_EN_COK {
            ob.retain(|_, (bitis, _)| *bitis > now);
            if ob.len() >= ONBELLEK_EN_COK {
                ob.clear();
            }
        }
        ob.insert(anahtar.to_string(), (now + ttl(&s), s.clone()));
        s
    }

    /// Bağ ve lisans: çağrıdan önce (lisans kapalıysa merkez zaten 403 verir; boşuna istek atılmaz).
    fn rapor_bag(&self, now: f64) -> Result<crate::merkez::Merkez, &'static str> {
        let m = crate::merkez::oku(&self.merkez_path).ok_or(BAGLI_DEGIL)?;
        if !crate::merkez::lisans_aktif(&m, now) {
            return Err(LISANS_KAPALI);
        }
        Ok(m)
    }

    fn rapor_mesaj(&self, cfg: &Config, req: &Req, o: &Oturum, baslik: &str, msg: &str, status: u16) -> Resp {
        let geri = if req.path == "/raporlar" { String::new() } else { "<p><a href=\"/raporlar\">← Raporlar</a></p>".into() };
        let mut r = self.page(cfg, req, Some(o), baslik, &format!("<div class=\"rapor\"><p>{}</p>{geri}</div>", h(msg)));
        r.status = status;
        r
    }

    pub(super) fn raporlar(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let m = match self.rapor_bag(now) {
            Ok(m) => m,
            Err(msg) => return self.rapor_mesaj(cfg, req, o, "Raporlar", msg, 200),
        };
        let s = self.rapor_cagir(&format!("{}:liste", m.numara), "/api/raporlar", &json!({"cihaz_anahtari": m.cihaz_anahtari}), now, |s| sure_of(s, LISTE_SN));
        let cozulen = match &s {
            Ok((200, g)) => serde_json::from_str::<Value>(g).ok().and_then(|v| liste_coz(&v)),
            Ok((403, _)) => return self.rapor_mesaj(cfg, req, o, "Raporlar", LISANS_KAPALI, 200),
            _ => None,
        };
        let Some((liste, bugun)) = cozulen else {
            return self.rapor_mesaj(cfg, req, o, "Raporlar", ALINAMIYOR, 200);
        };
        let bugun = bugun.unwrap_or_else(|| ortak::day_of(&ortak::now_iso(now)).to_string());
        self.page(cfg, req, Some(o), "Raporlar", &liste_html(&liste, &bugun))
    }

    pub(super) fn rapor(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let Some((tur, donem)) = rapor_yolu(&req.path) else {
            return self.rapor_mesaj(cfg, req, o, "Rapor", "Bu dönem için rapor yok.", 404);
        };
        let m = match self.rapor_bag(now) {
            Ok(m) => m,
            Err(msg) => return self.rapor_mesaj(cfg, req, o, "Rapor", msg, 200),
        };
        let yerel_bugun = ortak::day_of(&ortak::now_iso(now)).to_string();
        let govde = json!({"cihaz_anahtari": m.cihaz_anahtari, "tur": tur, "donem": donem});
        let ttl = |s: &Sonuc| {
            // kesinleşmiş geçmiş dönem uzun tutulur
            let kesin = matches!(s, Ok((200, g)) if serde_json::from_str::<Value>(g).ok().and_then(|v| rapor_coz(&v, tur, &donem))
                .is_some_and(|r| r.kesin && r.bit < r.bugun.clone().unwrap_or_else(|| yerel_bugun.clone())));
            sure_of(s, if kesin { KESIN_SN } else { LISTE_SN })
        };
        let s = self.rapor_cagir(&format!("{}:{tur}:{donem}", m.numara), "/api/rapor", &govde, now, ttl);
        let r = match &s {
            Ok((200, g)) => serde_json::from_str::<Value>(g).ok().and_then(|v| rapor_coz(&v, tur, &donem)),
            Ok((404 | 400, _)) => return self.rapor_mesaj(cfg, req, o, "Rapor", "Bu dönem için rapor yok.", 404),
            Ok((403, _)) => return self.rapor_mesaj(cfg, req, o, "Rapor", LISANS_KAPALI, 200),
            _ => None,
        };
        let Some(r) = r else {
            return self.rapor_mesaj(cfg, req, o, "Rapor", ALINAMIYOR, 200);
        };
        let bugun = r.bugun.clone().unwrap_or(yerel_bugun);
        let (baslik, govde) = rapor_html(&r, &bugun);
        self.page(cfg, req, Some(o), &baslik, &govde)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn donem_dogrulama() {
        for (t, d) in [("hafta", "2026-W42"), ("hafta", "2026-W01"), ("hafta", "2026-W53"), ("ay", "2026-10"), ("ay", "2026-01"), ("ay", "2026-12")] {
            assert!(donem_gecerli(t, d), "{t} {d}");
        }
        for (t, d) in [
            ("hafta", "2026-W00"), ("hafta", "2026-W54"), ("hafta", "2026-w42"), ("hafta", "2026-42"), ("hafta", "2026-W4"),
            ("ay", "2026-00"), ("ay", "2026-13"), ("ay", "2026-9"), ("ay", "../x"), ("ay", "2026-1٠"), ("gun", "2026-10"),
            ("ay", "2026-10/"), ("ay", ""), ("hafta", "2026-W42x"),
        ] {
            assert!(!donem_gecerli(t, d), "{t} {d}");
        }
        assert_eq!(rapor_yolu("/rapor/ay/2026-10"), Some(("ay", "2026-10".into())));
        assert_eq!(rapor_yolu("/rapor/hafta/2026-W42"), Some(("hafta", "2026-W42".into())));
        for p in ["/rapor/ay/2026-10/x", "/rapor/ay/../2026-10", "/rapor/yil/2026", "/rapor/ay", "/rapor/ay/%32026-10", "/rapor/ay/2026-10%00"] {
            assert_eq!(rapor_yolu(p), None, "{p}");
        }
    }

    #[test]
    fn bicimler_merkezle_ayni() {
        let ek: Vec<String> = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 31, 1000].iter().map(|n| format!("{n}'{}", iyelik(*n))).collect();
        assert_eq!(ek, ["0'ı", "1'i", "2'si", "3'ü", "4'ü", "5'i", "6'sı", "7'si", "8'i", "9'u", "10'u", "20'si", "30'u", "40'ı", "50'si", "60'ı", "70'i", "80'i", "90'ı", "100'ü", "31'i", "1000'i"]);
        assert_eq!(tam_metni(31, 31), "31 günün 31'i tam");
        assert_eq!(tam_metni(0, 0), "Henüz biten gün yok");
        assert_eq!(donem_adi("hafta", "2026-10-12", "2026-10-18"), "12–18.10.2026");
        assert_eq!(donem_adi("hafta", "2026-09-28", "2026-10-04"), "28.09–04.10.2026");
        assert_eq!(donem_adi("hafta", "2026-12-28", "2027-01-03"), "28.12.2026–03.01.2027");
        assert_eq!(donem_adi("ay", "2026-10-01", "2026-10-31"), "Ekim 2026");
        assert_eq!(sure_metni(59), "0 dk");
        assert_eq!(sure_metni(5400), "1 sa 30 dk");
        assert_eq!(sure_metni(90_000), "1 gün 1 sa");
        assert_eq!(boyut(500), "500 B");
        assert_eq!(boyut(1536), "1,5 KB");
        assert_eq!(boyut(4_294_967_296), "4,0 GB");
        assert_eq!(gun_adi_of("2026-10-14", false), "Çarşamba");
        assert_eq!(gun_adi_of("2026-10-12", true), "Pzt");
        assert_eq!(gun_adi_of("2000-02-29", false), "Salı");
        assert!(gun_coz("2026-02-29").is_none() && gun_coz("2024-02-29").is_some() && gun_coz("2026-1-01").is_none());
    }

    fn ornek_liste() -> Value {
        json!({"raporlar": [
            {"tur": "hafta", "donem": "2026-W42", "baslangic": "2026-10-12", "bitis": "2026-10-18", "kesin": false, "farkli": 14, "kayit_gun": 2, "kayit_tam": 2},
            {"tur": "hafta", "donem": "2026-W41", "baslangic": "2026-10-05", "bitis": "2026-10-11", "kesin": false, "farkli": 9, "kayit_gun": 7, "kayit_tam": 6},
            {"tur": "hafta", "donem": "2026-W40", "baslangic": "2026-09-28", "bitis": "2026-10-04", "kesin": true, "farkli": 3, "kayit_gun": 7, "kayit_tam": 7},
            {"tur": "ay", "donem": "2026-10", "baslangic": "2026-10-01", "bitis": "2026-10-31", "kesin": false, "farkli": 41, "kayit_gun": 13, "kayit_tam": 12},
            {"tur": "ay", "donem": "../../x", "baslangic": "2026-10-01", "bitis": "2026-10-31"},
            {"tur": "yil", "donem": "2026", "baslangic": "2026-01-01", "bitis": "2026-12-31"},
            {"tur": "ay", "donem": "2026-09", "baslangic": "2026-9-1", "bitis": "2026-09-30"},
            "bozuk"
        ], "bugun": "2026-10-14"})
    }

    #[test]
    fn liste_cozumu_ve_html() {
        let (liste, bugun) = liste_coz(&ornek_liste()).unwrap();
        assert_eq!(liste.len(), 4, "geçersiz öğeler atlanır");
        assert_eq!(bugun.as_deref(), Some("2026-10-14"));
        let html = liste_html(&liste, "2026-10-14");
        let haftalar = html.find("Haftalık raporlar").unwrap();
        assert!(haftalar < html.find("Aylık raporlar").unwrap(), "önce haftalar");
        assert!(html.contains("<a href=\"/rapor/hafta/2026-W42\">12–18.10.2026</a> <b class=\"uyarisi\">sürüyor</b>"));
        assert!(html.contains("<a href=\"/rapor/hafta/2026-W41\">05–11.10.2026</a> <span class=\"bos\">kesinleşmedi</span>"));
        assert!(html.contains("<a href=\"/rapor/hafta/2026-W40\">28.09–04.10.2026</a></td>"));
        assert!(html.contains("7 günün 6'sı tam") && html.contains("Ekim 2026") && !html.contains("x</a>"));
        assert!(liste_html(&[], "2026-10-14").contains("Henüz rapor yok"));
        assert!(liste_coz(&json!({"raporlar": "x"})).is_none());
    }

    fn ornek_rapor() -> Value {
        json!({
            "rapor": {
                "bicim": 2, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30",
                "uretim": "2026-10-01T02:31:07+03:00", "kesin": true,
                "misafir": {"farkli": 41, "yeni": 30, "donen": 11, "giris": 77, "giris_sms": 70, "giris_kod": 7,
                            "izinli_cihaz": 2, "gunluk_ortalama": 4.46, "en_yogun_gun": {"gun": "2026-09-12", "farkli": 9}},
                "saat": [0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 10, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, "x"],
                "hafta_gunu": [1, 2, 3],
                "gunler": [
                    {"gun": "2026-09-01", "durum": "tam", "neden": "", "farkli": 2},
                    {"gun": "2026-09-04", "durum": "dogrulanamadi", "neden": "zincir_yok", "farkli": 2},
                    {"gun": "2026-09-05", "durum": "kayit_yok", "neden": "", "farkli": null},
                    {"gun": "2026-09-06", "durum": "<script>", "neden": "<x>", "farkli": -1},
                    {"gun": "2026-09-07", "durum": "hizmet_oncesi", "neden": "", "farkli": null},
                    {"gun": "bozuk", "durum": "tam"}
                ],
                "kayit": {"gun": 20, "tam": 16},
                "kullanim": {"gonderilen": 1073741824u64, "alinan": 3221225472u64, "sms": 70, "alanlar": [["instagram.com", 5]]},
                "surum": "0.4.9", "surum_sonra_degisti": false
            },
            "isletme": {"numara": "1537344", "ad": "Göztepe <b>Bilgisayar</b>", "unvan": ""},
            "cihaz": {"surum": "0.4.9<script>", "cevrimdisi_sn": 14400, "sorunlar": [
                {"tur": "servis", "onem": "kritik", "metin": "Servis durdu: <img src=x>", "acilis": "2026-09-10T08:00:00+03:00",
                 "kapanis": "2026-09-10T09:30:00+03:00", "sure_sn": 5400},
                {"tur": "ulasilamiyor", "onem": "uyari", "metin": "Cihaz ulaşılamıyor", "acilis": "2026-09-21T15:13:20+03:00",
                 "kapanis": null, "sure_sn": null},
                {"tur": "lisans", "onem": "bilgi", "metin": "gizli", "acilis": "2026-09-22T10:00:00+03:00"}
            ]},
            "bugun": "2026-10-14"
        })
    }

    #[test]
    fn rapor_cozumu_ve_html_kacislar() {
        let v = ornek_rapor();
        assert!(rapor_coz(&v, "ay", "2026-10").is_none(), "dönem tutmuyor");
        assert!(rapor_coz(&v, "hafta", "2026-09").is_none(), "tür tutmuyor");
        let r = rapor_coz(&v, "ay", "2026-09").unwrap();
        assert_eq!(r.saat.len(), 24);
        assert_eq!(r.saat[23], 0);
        assert_eq!(r.hafta_gunu, vec![1, 2, 3, 0, 0, 0, 0]);
        assert_eq!(r.gunler.len(), 5);
        assert_eq!(r.gunler[3].durum, "bilinmiyor");
        assert_eq!(r.gunler[3].farkli, None);
        assert_eq!(r.sorunlar.len(), 2, "bilgi düzeyi sorun gösterilmez");
        assert_eq!(r.cihaz_surum, None, "geçersiz sürüm metni");
        let (baslik, html) = rapor_html(&r, "2026-10-14");
        assert_eq!(baslik, "Aylık rapor · Eylül 2026");
        for p in [
            "20 günün 16'sı tam", "<span class=\"iyi\">✔ Tam</span>", "(gün mühür zincirinde yok)", "Kayıt gelmedi", "Hizmet öncesi",
            "Göztepe &lt;b&gt;Bilgisayar&lt;/b&gt;", "Servis durdu: &lt;img src=x&gt;", "4,5", "12.09.2026 Cumartesi · 9 misafir",
            "4,0 GB (indirme 3,0 GB, yükleme 1,0 GB)", "en az 4 sa 0 dk", "1 sa 30 dk", "<b class=\"kotu\">açık</b>", "Ctrl+P",
            "<span style=\"width:50%\"></span>", "<span style=\"width:100%\"></span>", "01.10.2026 02:31", "1537344", "Pzt",
        ] {
            assert!(html.contains(p), "{p} yok");
        }
        for p in ["<script>", "<img", "instagram", "gizli", "<x>", "Dönem sürüyor", "kesinleşmedi"] {
            assert!(!html.contains(p), "{p} var");
        }
        // süren dönem
        let (_, html) = rapor_html(&r, "2026-09-20");
        assert!(html.contains("Dönem sürüyor"));
    }
}
