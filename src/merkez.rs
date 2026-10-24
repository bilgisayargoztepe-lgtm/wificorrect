//! Yönetim merkezi bağı (spec 2026-10-04 §7): müşteri numarası + parola merkezde açılır; cihaz ilk girişte
//! `api.wificorrect.com`'a bağlanır, bağ `merkez.json`'da tutulur (internetsiz giriş yerel özetle). Her gün 06:00'dan
//! sonra eşitlenir; serbest bırakılınca fabrika akışı kayıtları teslim edip bağı siler. Cihaz anahtarı loglanmaz.

use crate::ayar::Config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const PATH: &str = "/etc/wificorrect/merkez.json";
pub const API: &str = "https://api.wificorrect.com";
const TR: f64 = 3.0 * 3600.0; // Türkiye UTC+3, yaz saati yok
const YENIDEN_SN: f64 = 30.0 * 60.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Merkez {
    pub numara: String,
    pub tuz: String,
    pub ozet: String,
    pub yineleme: u32,
    pub cihaz_anahtari: String,
    /// son başarılı eşitleme (epoch); ilk girişte giriş anı
    #[serde(default)]
    pub son_eslesme: f64,
    /// son eşitleme denemesi (epoch) — başarısızsa 30 dk sonra yeniden
    #[serde(default)]
    pub deneme: f64,
    /// yıllık lisans: "aktif" | "bitti" | "askida" (boş = eski bağ, açık sayılır)
    #[serde(default)]
    pub lisans: String,
    /// lisans bitiş günü (YYYY-AA-GG, TR): cihaz bu günden sonra misafirlere internet vermez (eşitleme beklemeden)
    #[serde(default)]
    pub lisans_bitis: String,
    /// Müşterinin AP yönetim parolası (merkez üretir; bağlanırken ve her eşitlemede gelir, panel → AP kurulumu gösterir).
    /// AP'ye panel parolası değil bu yazılır (final review 2026-10-16 I1). Eski merkez göndermezse boş.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ap_parola: String,
}

/// Merkezin gönderdiği AP yönetim parolası: 8–64 yazdırılabilir ASCII karakter (boşluk yok); değilse yok sayılır.
/// Bağlanma/eşitleme cevapları ve 5 dk'lık sağlık cevabı (`saglik::gonder`) aynı kuralı kullanır.
pub(crate) fn ap_parola_oku(v: &Value) -> Option<String> {
    metin(v, "ap_parola").filter(|s| (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
}

/// (yol, JSON gövde) → (HTTP durumu, gövde). Err: merkeze ulaşılamadı.
pub type Http = dyn Fn(&str, &str) -> Result<(u16, String), String> + Send + Sync;

/// Gerçek istemci: curl, sertifika doğrulanır, gövde standart girdiden (parola komut satırında görünmez).
/// curl argümanları: gövde (parola, cihaz anahtarı) standart girdiden gelir, komut satırında görünmez.
/// Bağlanma 10 sn, toplam 25 sn (merkez ilk girişte kuyruğu en çok 15 sn bekler); panelin işçileri uzun bloklanmasın.
pub fn curl_args(url: &str) -> Vec<String> {
    curl_args_sure(url, 25)
}

/// `curl_args`, toplam süre `max_sn` (bağlanma en çok 10 sn ya da `max_sn`). Cevap en çok 2 MB (JSON; rapor ≤ 1 MB).
pub fn curl_args_sure(url: &str, max_sn: u32) -> Vec<String> {
    let (bag, max) = (max_sn.min(10).to_string(), max_sn.to_string());
    ["-s", "--connect-timeout", &bag, "--max-time", &max, "--max-filesize", "2M", "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-", "-w", "\n%{http_code}", url]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn curl(yol: &str, govde: &str) -> Result<(u16, String), String> {
    curl_sure(yol, govde, 25)
}

/// `curl`, toplam süre `max_sn` (sağlık raporu 10 sn).
pub fn curl_sure(yol: &str, govde: &str, max_sn: u32) -> Result<(u16, String), String> {
    let mut c = Command::new("curl")
        .args(curl_args_sure(&format!("{API}{yol}"), max_sn))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    c.stdin.take().ok_or("curl stdin")?.write_all(govde.as_bytes()).map_err(|e| e.to_string())?;
    let out = c.wait_with_output().map_err(|e| e.to_string())?;
    let s = String::from_utf8_lossy(&out.stdout);
    let (g, kod) = s.rsplit_once('\n').ok_or("merkezden yanıt yok")?;
    match kod.trim().parse::<u16>() {
        Ok(k) if k > 0 => Ok((k, g.to_string())),
        _ => Err("merkeze ulaşılamadı".into()),
    }
}

/// (yol, JSON gövde, çıktı dosyası) → HTTP durumu; cevap gövdesi dosyaya yazılır (paket indirme).
pub type Indir<'a> = dyn Fn(&str, &str, &Path) -> Result<u16, String> + 'a;

/// `indir` için curl argümanları: gövde standart girdiden, cevap `cikti`ya, standart çıktıda yalnızca HTTP kodu.
pub fn indir_args(url: &str, cikti: &Path, max_sn: u32) -> Vec<String> {
    let (bag, max) = (max_sn.min(10).to_string(), max_sn.to_string());
    let cikti = cikti.display().to_string();
    ["-s", "--connect-timeout", &bag, "--max-time", &max, "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-", "-o", &cikti, "-w", "%{http_code}", url]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Merkezden dosya indirir (`/api/paket`); toplam süre `max_sn`. Err: merkeze ulaşılamadı.
pub fn indir(yol: &str, govde: &str, cikti: &Path, max_sn: u32) -> Result<u16, String> {
    let mut c = Command::new("curl")
        .args(indir_args(&format!("{API}{yol}"), cikti, max_sn))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    c.stdin.take().ok_or("curl stdin")?.write_all(govde.as_bytes()).map_err(|e| e.to_string())?;
    let out = c.wait_with_output().map_err(|e| e.to_string())?;
    indir_sonuc(out.status.success(), &String::from_utf8_lossy(&out.stdout))
}

/// curl'ün çıkış durumu ve yazdığı HTTP kodu → sonuç. curl hata verdiyse (18 kısmi aktarım, 28 süre) kod 200 olsa da
/// dosya yarımdır: Err.
pub fn indir_sonuc(basari: bool, cikti: &str) -> Result<u16, String> {
    match cikti.trim().parse::<u16>() {
        Ok(k) if k > 0 && basari => Ok(k),
        Ok(k) if k > 0 => Err("indirme yarım kaldı".into()),
        _ => Err("merkeze ulaşılamadı".into()),
    }
}

pub fn oku(path: &Path) -> Option<Merkez> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn kaydet(path: &Path, m: &Merkez) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
    crate::ortak::write_atomic(path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn sil(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn numara_gecerli(s: &str) -> bool {
    s.len() == 7 && s.bytes().all(|b| b.is_ascii_digit()) && !s.starts_with('0')
}

pub fn dogrula(m: &Merkez, pw: &str) -> bool {
    crate::hesap::ct_eq(&crate::hesap::digest(pw, &m.tuz, m.yineleme), &m.ozet)
}

#[derive(Debug)]
pub enum GirisHata {
    /// merkezin Türkçe mesajı; bool: hatalı parola gibi kilit sayacına yazılmalı mı
    Mesaj(String, bool),
    Baglanti,
}

pub struct Giris {
    pub merkez: Merkez,
    pub tunel_ip: String,
    pub sunucu_pub: String,
    pub uc_nokta: String,
    pub yedek_hedefi: String,
    pub ek: Ek,
}

fn metin(v: &Value, k: &str) -> Option<String> {
    v.get(k)?.as_str().map(String::from)
}

/// Yanıttan özet alanları (tuz, ozet, yineleme).
fn ozet_alanlari(v: &Value) -> Option<(String, String, u32)> {
    Some((metin(v, "tuz")?, metin(v, "ozet")?, u32::try_from(v.get("yineleme")?.as_u64()?).ok()?))
}

/// Merkezin bağlı cihaza gönderdiği ekler (spec 2026-10-05 §5-6). Eski merkez göndermezse boş.
/// `sms_mock`: merkezin etkin deneme modu (Özet uyarısı ve sağlık raporu için; bkz. `sms_mock_oku`). Merkez eski cihazlar
/// için NetGSM bilgilerini de gönderir; bu cihaz onları okumaz (SMS merkezden gider, spec 2026-10-13).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ek {
    pub admin: Option<(String, String, u32)>,
    pub sms_mock: Option<bool>,
}

fn ek(v: &Value) -> Ek {
    Ek { admin: v.get("admin").and_then(ozet_alanlari), sms_mock: v.get("sms").and_then(sms_mock_oku) }
}

/// SMS ekinden etkin deneme modu: `etkin_mock` (deneme modu açık ya da merkezde seçili sağlayıcının ayarı eksik) varsa o,
/// yoksa (bu alandan önceki merkez) `mock`.
pub fn sms_mock_oku(s: &Value) -> Option<bool> {
    s.get("etkin_mock").and_then(Value::as_bool).or_else(|| s.get("mock")?.as_bool())
}

/// Bağlanırken ve her eşitlemede: (admin sonucu, SMS sonucu); ikisi birbirinden bağımsız, her biri "değişti mi" döner.
/// Admin özeti hesaplara, deneme modu bilgisi `cfg`'ye uygulanır (kaydetmek çağıranın işi).
pub fn ek_uygula(cfg: &mut Config, hesaplar: &crate::hesap::Hesaplar, ek: &Ek) -> (Result<bool, String>, Result<bool, String>) {
    let admin = match &ek.admin {
        Some((tuz, ozet, y)) => hesaplar.set_admin_ozet(tuz, ozet, *y),
        None => Ok(false),
    };
    let sms = match ek.sms_mock {
        Some(mock) => {
            let degisti = cfg.sms.mock != mock;
            cfg.sms.mock = mock;
            Ok(degisti)
        }
        None => Ok(false),
    };
    (admin, sms)
}

/// `POST /api/sms` cevabı (spec 2026-10-13). `saglayici` cevapta yoksa boş (AYAR, YABANCI).
#[derive(Debug, Clone, PartialEq)]
pub struct SmsCevap {
    pub sonuc: String,
    pub kod: String,
    pub is: Option<String>,
    pub saglayici: String,
}

/// Doğrulama kodunu merkeze gönderir; mesajı merkez kendi şablonundan kurar. Err: merkeze ulaşılamadı, HTTP 200 değil ya da
/// cevap bozuk. Kod ve cihaz anahtarı hata metnine girmez.
pub fn sms_gonder(m: &Merkez, telefon: &str, kod: &str, http: &Http) -> Result<SmsCevap, String> {
    let (d, v) = cagir(http, "/api/sms", &json!({"cihaz_anahtari": m.cihaz_anahtari, "telefon": telefon, "kod": kod}))?;
    if d != 200 {
        return Err(hata(&v).map_or(format!("merkez SMS: HTTP {d}"), |(h, _)| format!("merkez SMS: {h}")));
    }
    let sonuc = metin(&v, "sonuc").filter(|s| ["gonderildi", "mock", "hata"].contains(&s.as_str())).ok_or("merkez SMS: geçersiz yanıt")?;
    Ok(SmsCevap { sonuc, kod: metin(&v, "kod").unwrap_or_default(), is: metin(&v, "is").filter(|s| !s.is_empty()), saglayici: metin(&v, "saglayici").unwrap_or_default() })
}

/// Twilio Verify kodunu merkez üzerinden doğrular: Some(onay) ya da None (merkez Twilio'ya ulaşamadı, `hata/AG`).
/// Err: merkeze ulaşılamadı ya da başka bir hata (AYAR …) / bozuk cevap.
pub fn sms_dogrula(m: &Merkez, telefon: &str, kod: &str, http: &Http) -> Result<Option<bool>, String> {
    let (d, v) = cagir(http, "/api/sms-dogrula", &json!({"cihaz_anahtari": m.cihaz_anahtari, "telefon": telefon, "kod": kod}))?;
    if d != 200 {
        return Err(format!("merkez SMS doğrulama: HTTP {d}"));
    }
    if let Some(onay) = v.get("onay").and_then(Value::as_bool) {
        return Ok(Some(onay));
    }
    match (metin(&v, "sonuc").as_deref(), metin(&v, "kod").as_deref()) {
        (Some("hata"), Some("AG")) => Ok(None),
        (Some("hata"), Some(k)) => Err(format!("merkez SMS doğrulama: {k}")),
        _ => Err("merkez SMS doğrulama: geçersiz yanıt".into()),
    }
}

/// Hata yanıtı ise (mesaj, kod).
pub fn hata(v: &Value) -> Option<(String, String)> {
    Some((metin(v, "hata")?, metin(v, "kod").unwrap_or_default()))
}

pub fn cagir(http: &Http, yol: &str, govde: &Value) -> Result<(u16, Value), String> {
    let (d, g) = http(yol, &govde.to_string())?;
    let v: Value = serde_json::from_str(&g).map_err(|_| format!("merkezden geçersiz yanıt (HTTP {d})"))?;
    Ok((d, v))
}

pub fn giris(numara: &str, pw: &str, wg_pub: &str, ssh_pub: &str, isletme_adi: &str, unvan: &str, http: &Http, now: f64) -> Result<Giris, GirisHata> {
    let govde = json!({"numara": numara, "parola": pw, "wg_pub": wg_pub, "ssh_pub": ssh_pub,
                       "isletme_adi": isletme_adi, "unvan": unvan, "surum": env!("CARGO_PKG_VERSION")});
    let (d, v) = cagir(http, "/api/giris", &govde).map_err(|_| GirisHata::Baglanti)?;
    giris_cevabi(d, &v, numara, now)
}

/// `/api/giris` ve `/api/bagla` (kolay bağlama) cevabı → bağ bilgileri. Hata cevabı merkezin mesajıyla; bozuk/eksik cevap
/// bağlantı hatası sayılır.
pub fn giris_cevabi(d: u16, v: &Value, numara: &str, now: f64) -> Result<Giris, GirisHata> {
    if d != 200 {
        return match hata(v) {
            Some((m, kod)) => Err(GirisHata::Mesaj(m, kod == "hatali")),
            None => Err(GirisHata::Baglanti),
        };
    }
    let alanlar = (|| {
        let (tuz, ozet, yineleme) = ozet_alanlari(v)?;
        Some(Giris {
            merkez: Merkez { numara: numara.into(), tuz, ozet, yineleme, cihaz_anahtari: metin(v, "cihaz_anahtari")?, son_eslesme: now, deneme: 0.0,
                             lisans: metin(v, "lisans").unwrap_or_default(), lisans_bitis: metin(v, "lisans_bitis").unwrap_or_default(),
                             ap_parola: ap_parola_oku(v).unwrap_or_default() },
            tunel_ip: metin(v, "tunel_ip")?,
            sunucu_pub: metin(v, "sunucu_pub")?,
            uc_nokta: metin(v, "uc_nokta")?,
            yedek_hedefi: metin(v, "yedek_hedefi")?,
            ek: ek(v),
        })
    })();
    alanlar.ok_or(GirisHata::Baglanti)
}

/// Bağlanan cihazın uzak erişimi ve uzak yedeği merkezden gelen değerlerle açılır.
pub fn uygula(cfg: &mut Config, g: &Giris) {
    cfg.uzak.enabled = true;
    cfg.uzak.sunucu.clone_from(&g.uc_nokta);
    cfg.uzak.sunucu_anahtar.clone_from(&g.sunucu_pub);
    cfg.uzak.adres.clone_from(&g.tunel_ip);
    cfg.backup.enabled = true;
    cfg.backup.target.clone_from(&g.yedek_hedefi);
}

/// Bağlama dosya kilidi (`merkez.json` yanında `merkez.kilit`): yerel panelden bağlama ile kolay bağlama (`ctl bekliyor`,
/// ayrı süreç) aynı anda iki bağ yazmasın. Düşünce bırakılır.
pub fn kilit(merkez_path: &Path) -> std::io::Result<crate::ortak::StateLock> {
    crate::ortak::dosya_kilidi(&merkez_path.with_extension("kilit"))
}

/// Merkez bağlamayı kabul ettikten sonra cihaz tarafı (yerel panel ve kolay bağlama ortak): önce ayarlar (tünel + yedek +
/// merkezden gelen ekler), sonra bağ dosyası — ayar yazılamazsa cihaz "bağlı ama tünelsiz" kalmasın; eski yerel sahip
/// hesapları silinir, tünel ayrı işte kurulur, SMS deneme modu değiştiyse portal yeniden başlar.
/// Dönen: (yeni ayar, admin eki sonucu, SMS eki sonucu) — denetim satırları çağıranın.
pub fn kur(cfg: &Config, cfg_path: &str, merkez_path: &Path, hesaplar: &crate::hesap::Hesaplar, g: &Giris, runner: &crate::ortak::Runner) -> Result<(Config, Result<bool, String>, Result<bool, String>), String> {
    let mut yeni = cfg.clone();
    uygula(&mut yeni, g);
    let (admin, sms) = ek_uygula(&mut yeni, hesaplar, &g.ek);
    yeni.save(cfg_path)?;
    kaydet(merkez_path, &g.merkez)?;
    if let Err(e) = hesaplar.keep_only_admin() {
        eprintln!("merkez: {e}");
    }
    let unit = format!("wfc-uzak-{}", crate::ortak::random_hex(4));
    runner(&["systemd-run", "--collect", "--unit", unit.as_str(), "--on-active", "1s", "/usr/local/bin/wificorrect", "ctl", "uzak-uygula"].map(String::from).to_vec());
    if sms == Ok(true) {
        runner(&["systemctl", "restart", "wificorrect-portal"].map(String::from).to_vec());
    }
    Ok((yeni, admin, sms))
}

#[derive(Debug)]
pub enum Eslesme {
    Bagli { uyelik: String, ek: Ek },
    Serbest,
    Taninmadi,
}

/// `adres`: veri sorumlusunun adresi (Ayarlar → İşletme); merkezde unvanın yanında görünür.
#[allow(clippy::too_many_arguments)]
pub fn eslesme(m: &mut Merkez, isletme_adi: &str, unvan: &str, adres: &str, isletme_turu: &str, temizlendi: bool, http: &Http, now: f64) -> Result<Eslesme, String> {
    let mut govde = json!({"cihaz_anahtari": m.cihaz_anahtari, "isletme_adi": isletme_adi, "unvan": unvan, "adres": adres, "surum": env!("CARGO_PKG_VERSION")});
    if !isletme_turu.is_empty() {
        govde["isletme_turu"] = Value::String(isletme_turu.into()); // yönetimde müşteri listesinde görünür
    }
    if temizlendi {
        govde["temizlendi"] = Value::Bool(true);
    }
    let (d, v) = cagir(http, "/api/eslesme", &govde)?;
    if d == 401 && hata(&v).is_some_and(|(_, k)| k == "taninmadi") {
        return Ok(Eslesme::Taninmadi);
    }
    if d != 200 {
        return Err(hata(&v).map_or(format!("HTTP {d}"), |(m, _)| m));
    }
    match metin(&v, "durum").as_deref() {
        Some("serbest") => Ok(Eslesme::Serbest),
        Some("bagli") => {
            let (tuz, ozet, yineleme) = ozet_alanlari(&v).ok_or("merkez yanıtında parola özeti yok")?;
            (m.tuz, m.ozet, m.yineleme, m.son_eslesme) = (tuz, ozet, yineleme, now);
            if let Some(l) = metin(&v, "lisans") {
                m.lisans = l;
            }
            if let Some(b) = metin(&v, "lisans_bitis") {
                m.lisans_bitis = b;
            }
            if let Some(a) = ap_parola_oku(&v) {
                m.ap_parola = a; // merkezde yenilendiyse burada iner; göndermeyen (eski) merkezde eskisi kalır
            }
            Ok(Eslesme::Bagli { uyelik: metin(&v, "uyelik").unwrap_or_default(), ek: ek(&v) })
        }
        _ => Err("merkezden geçersiz yanıt".into()),
    }
}

pub fn parola(m: &mut Merkez, eski: &str, yeni: &str, http: &Http) -> Result<(), String> {
    let govde = json!({"cihaz_anahtari": m.cihaz_anahtari, "eski": eski, "yeni": yeni});
    let (d, v) = cagir(http, "/api/parola", &govde).map_err(|_| "Parola değişimi için internet gerekli.".to_string())?;
    if d != 200 {
        return Err(hata(&v).map_or(format!("Merkez parola değişimini kabul etmedi (HTTP {d})."), |(m, _)| m));
    }
    let (tuz, ozet, yineleme) = ozet_alanlari(&v).ok_or("Merkezden geçersiz yanıt.")?;
    (m.tuz, m.ozet, m.yineleme) = (tuz, ozet, yineleme);
    Ok(())
}

/// Eşitleme zamanı: son başarılı eşitlemeden sonraki ilk 06:00 (TR) geçtiyse ve son denemenin üzerinden 30 dk geçtiyse.
/// Lisans kapalıyken 06:00 beklenmez (ödeme/askıdan çıkarma 30 dk içinde cihaza insin). Saat geri gittiyse (deneme
/// gelecekte) beklenmez.
pub fn zamani_geldi(m: &Merkez, now: f64) -> bool {
    if m.deneme > now + 60.0 {
        return true;
    }
    if now - m.deneme < YENIDEN_SN {
        return false;
    }
    if !lisans_aktif(m, now) {
        return true;
    }
    let yerel = m.son_eslesme.min(now) + TR;
    let mut alti = (yerel / 86_400.0).floor() * 86_400.0 + 6.0 * 3600.0;
    if yerel >= alti {
        alti += 86_400.0;
    }
    now >= alti - TR
}

/// Lisans açık mı: durum "aktif" (ya da eski bağda boş) ve bitiş günü (TR) geçmemiş. Bilinmeyen durum kapalı sayılır.
pub fn lisans_aktif(m: &Merkez, now: f64) -> bool {
    let bugun = crate::ortak::now_iso(now);
    let bugun = crate::ortak::day_of(&bugun);
    (m.lisans.is_empty() || m.lisans == "aktif") && (m.lisans_bitis.is_empty() || bugun <= m.lisans_bitis.as_str())
}

/// Misafirlere internet verilir mi: cihaz bir müşteriye bağlı ve lisansı açık.
pub fn hizmet_acik(path: &Path, now: f64) -> bool {
    oku(path).is_some_and(|m| lisans_aktif(&m, now))
}

/// Fabrika dönüşünde merkeze "temizlendi" bildirimi. Yalnızca merkez cihazı serbest bırakmışsa (ya da zaten tanımıyorsa —
/// önceki bildirim ulaşmış / zorla ayrılmış) tamam; "bağlı" derse hata (bağ silinmesin). Ağ hatasında `deneme` kez dener.
pub fn temizlendi_bildir(m: &Merkez, http: &Http, deneme: u32, bekle: &dyn Fn()) -> Result<(), String> {
    let mut son = String::new();
    for i in 0..deneme {
        if i > 0 {
            bekle();
        }
        match eslesme(&mut m.clone(), "", "", "", "", true, http, 0.0) {
            Ok(Eslesme::Serbest) | Ok(Eslesme::Taninmadi) => return Ok(()),
            Ok(Eslesme::Bagli { .. }) => return Err("merkez cihazı serbest bırakmadı".into()),
            Err(e) => son = e,
        }
    }
    Err(son)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const N: &str = "4511643";

    fn tmp(ad: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wfc-merkez-{}-{ad}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Sahte merkez: çağrıları kaydeder, sırayla verilen yanıtları döner.
    fn sahte(yanitlar: Vec<Result<(u16, &'static str), &'static str>>) -> (Box<Http>, Arc<Mutex<Vec<(String, serde_json::Value)>>>) {
        let cagrilar = Arc::new(Mutex::new(vec![]));
        let (c2, y) = (cagrilar.clone(), Arc::new(Mutex::new(yanitlar.into_iter())));
        let f = move |yol: &str, govde: &str| {
            c2.lock().unwrap().push((yol.to_string(), serde_json::from_str(govde).unwrap()));
            match y.lock().unwrap().next().expect("beklenmeyen çağrı") {
                Ok((d, g)) => Ok((d, g.to_string())),
                Err(e) => Err(e.to_string()),
            }
        };
        (Box::new(f), cagrilar)
    }

    fn ornek() -> Merkez {
        let tuz = "a1b2c3d4".to_string();
        Merkez { numara: N.into(), ozet: crate::hesap::digest("parola-12345", &tuz, 120_000), tuz, yineleme: 120_000,
                 cihaz_anahtari: "gizli-anahtar".into(), son_eslesme: 0.0, deneme: 0.0, ..Default::default() }
    }

    const GIRIS_OK: &str = r#"{"tuz":"a1b2c3d4","ozet":"ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495","yineleme":120000,
        "cihaz_anahtari":"k-123","tunel_ip":"10.99.0.12","sunucu_pub":"S","uc_nokta":"vpn.wificorrect.com:51820","yedek_hedefi":"wfc-4511643@10.99.0.1:","lisans":"aktif","lisans_bitis":"2027-10-04"}"#;

    const EK: &str = r#""admin":{"tuz":"ab12","ozet":"OZET","yineleme":120000},
        "sms":{"mock":false,"provider":"netgsm","netgsm":{"usercode":"8503027084","password":"gizli-1","msgheader":"gztp.blgsyr","appkey":""}}"#;

    fn ekli(govde: &str, admin_ozet: &str) -> &'static str {
        let g = govde.trim_end_matches('}').to_string() + "," + &EK.replace("OZET", admin_ozet) + "}";
        Box::leak(g.into_boxed_str())
    }

    #[test]
    fn giris_ve_eslesme_ekleri() {
        let oz = crate::hesap::digest("merkez-parola-1", "ab12", 120_000);
        let (h, _) = sahte(vec![Ok((200, ekli(GIRIS_OK, &oz)))]);
        let g = giris(N, "parola-12345", "W", "S", "", "", &*h, 0.0).unwrap();
        assert_eq!(g.ek.admin, Some(("ab12".into(), oz.clone(), 120_000)));
        assert_eq!(g.ek.sms_mock, Some(false));
        let (h, _) = sahte(vec![Ok((200, GIRIS_OK))]); // eski merkez: ek yok
        assert_eq!(giris(N, "parola-12345", "W", "S", "", "", &*h, 0.0).unwrap().ek, Ek::default());
        let mut m = ornek();
        let b = ekli(r#"{"durum":"bagli","tuz":"a1b2c3d4","ozet":"ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495","yineleme":120000,"uyelik":"aktif"}"#, &oz);
        let (h, _) = sahte(vec![Ok((200, b))]);
        assert!(matches!(eslesme(&mut m, "", "", "", "", false, &*h, 1.0), Ok(Eslesme::Bagli { ek, .. }) if ek.admin.is_some() && ek.sms_mock == Some(false)));
    }

    #[test]
    fn ap_yonetim_parolasi_giris_ve_eslesmede() {
        // final review I1: merkez müşteriye özel AP parolasını bağlanırken ve her eşitlemede gönderir; geçersiz değer yok sayılır
        let g: &'static str = Box::leak(GIRIS_OK.replace(r#""lisans":"aktif""#, r#""ap_parola":"Kp7mQx2Rtz4W","lisans":"aktif""#).into_boxed_str());
        let (h, _) = sahte(vec![Ok((200, g)), Ok((200, GIRIS_OK))]);
        assert_eq!(giris(N, "parola-12345", "W", "S", "", "", &*h, 0.0).unwrap().merkez.ap_parola, "Kp7mQx2Rtz4W");
        assert_eq!(giris(N, "parola-12345", "W", "S", "", "", &*h, 0.0).unwrap().merkez.ap_parola, ""); // eski merkez
        let govde = |ek: &str| -> &'static str {
            Box::leak(format!(r#"{{"durum":"bagli","tuz":"a1b2c3d4","ozet":"o","yineleme":120000,"uyelik":"aktif"{ek}}}"#).into_boxed_str())
        };
        let mut m = ornek();
        let (h, _) = sahte(vec![Ok((200, govde(r#","ap_parola":"Yeni2345Abcd""#))), Ok((200, govde(""))), Ok((200, govde(r#","ap_parola":"a b c d e f""#))),
                               Ok((200, govde(r#","ap_parola":"kisa""#))), Ok((200, govde(r#","ap_parola":12345678901"#)))]);
        eslesme(&mut m, "", "", "", "", false, &*h, 1.0).unwrap();
        assert_eq!(m.ap_parola, "Yeni2345Abcd");
        for _ in 0..4 {
            eslesme(&mut m, "", "", "", "", false, &*h, 1.0).unwrap(); // yok, boşluklu, kısa, sayı: eskisi kalır
            assert_eq!(m.ap_parola, "Yeni2345Abcd");
        }
        // merkez.json'da saklanır (600); boşsa dosyaya yazılmaz
        let p = tmp("ap-parola").join("merkez.json");
        kaydet(&p, &m).unwrap();
        assert_eq!(oku(&p).unwrap().ap_parola, "Yeni2345Abcd");
        kaydet(&p, &ornek()).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("ap_parola") && oku(&p).unwrap().ap_parola.is_empty());
    }

    #[test]
    fn ek_etkin_deneme_modu_once() {
        // merkez deneme modunda değil ama seçili sağlayıcının ayarı eksik: etkin deneme modu açık
        assert_eq!(ek(&json!({"sms": {"mock": false, "etkin_mock": true, "netgsm": {}}})).sms_mock, Some(true));
        assert_eq!(ek(&json!({"sms": {"mock": true, "etkin_mock": false}})).sms_mock, Some(false));
        assert_eq!(ek(&json!({"sms": {"etkin_mock": true}})).sms_mock, Some(true)); // merkezde SMS ayarı hiç yok
        assert_eq!(ek(&json!({"sms": {"mock": false}})).sms_mock, Some(false)); // eski merkez
        assert_eq!(ek(&json!({"sms": {"etkin_mock": "evet", "mock": true}})).sms_mock, Some(true));
        assert_eq!(ek(&json!({"sms": {}})).sms_mock, None);
    }

    #[test]
    fn ek_uygula_sms_ve_admin() {
        // SMS ekinden yalnızca deneme modu alınır; NetGSM bilgileri cihaza yazılmaz (SMS merkezden gider)
        let d = tmp("ek");
        let hs = crate::hesap::Hesaplar::new(d.join("hesaplar.json"));
        let oz = crate::hesap::digest("merkez-parola-1", "ab12", 120_000);
        let ek = Ek { admin: Some(("ab12".into(), oz, 120_000)), sms_mock: Some(false) };
        let mut cfg = Config::default();
        assert_eq!(ek_uygula(&mut cfg, &hs, &ek), (Ok(true), Ok(true)));
        assert!(!cfg.sms.mock);
        assert_eq!(ek_uygula(&mut cfg, &hs, &ek), (Ok(false), Ok(false))); // aynı değerler: dokunulmaz
        assert_eq!(ek_uygula(&mut cfg, &hs, &Ek { admin: None, sms_mock: Some(true) }), (Ok(false), Ok(true)));
        assert!(cfg.sms.mock);
    }

    #[test]
    fn ek_admin_ve_sms_birbirinden_bagimsiz() {
        let d = tmp("ek-bagimsiz");
        let hs = crate::hesap::Hesaplar::new(d.join("hesaplar.json"));
        // bozuk admin özeti + SMS: deneme modu yine uygulanır
        let ek = Ek { admin: Some(("ab12".into(), "zz".into(), 120_000)), sms_mock: Some(false) };
        let mut cfg = Config::default();
        let (admin, sms) = ek_uygula(&mut cfg, &hs, &ek);
        assert!(admin.is_err());
        assert_eq!(sms, Ok(true));
        assert!(!cfg.sms.mock);
    }

    fn bag() -> Merkez {
        Merkez { numara: N.into(), cihaz_anahtari: "k-1".into(), ..Default::default() }
    }

    #[test]
    fn sms_gonder_cevaplari() {
        let (h, c) = sahte(vec![Ok((200, r#"{"sonuc":"gonderildi","kod":"0","is":"J9","saglayici":"netgsm"}"#))]);
        assert_eq!(sms_gonder(&bag(), "905334553132", "123456", &*h),
                   Ok(SmsCevap { sonuc: "gonderildi".into(), kod: "0".into(), is: Some("J9".into()), saglayici: "netgsm".into() }));
        assert_eq!(c.lock().unwrap()[0], ("/api/sms".to_string(), json!({"cihaz_anahtari": "k-1", "telefon": "905334553132", "kod": "123456"})));
        let (h, _) = sahte(vec![Ok((200, r#"{"sonuc":"hata","kod":"YABANCI"}"#))]);
        assert_eq!(sms_gonder(&bag(), "4915123456789", "123456", &*h),
                   Ok(SmsCevap { sonuc: "hata".into(), kod: "YABANCI".into(), is: None, saglayici: String::new() }));
        for y in [Err("ağ"), Ok((200, "bozuk")), Ok((401, r#"{"hata":"tanınmayan cihaz"}"#)), Ok((200, r#"{"sonuc":"?"}"#))] {
            let (h, _) = sahte(vec![y]);
            let e = sms_gonder(&bag(), "905334553132", "123456", &*h).unwrap_err();
            assert!(!e.contains("123456") && !e.contains("k-1"), "{e}");
        }
    }

    #[test]
    fn sms_dogrula_cevaplari() {
        for (y, beklenen) in [
            (Ok((200, r#"{"onay":true}"#)), Ok(Some(true))),
            (Ok((200, r#"{"onay":false}"#)), Ok(Some(false))),
            (Ok((200, r#"{"sonuc":"hata","kod":"AG"}"#)), Ok(None)),
        ] {
            let (h, c) = sahte(vec![y]);
            assert_eq!(sms_dogrula(&bag(), "4915123456789", "424242", &*h), beklenen);
            assert_eq!(c.lock().unwrap()[0].0, "/api/sms-dogrula");
        }
        for y in [Err("ağ"), Ok((200, r#"{"sonuc":"hata","kod":"AYAR"}"#)), Ok((403, r#"{"hata":"x"}"#)), Ok((200, "{}"))] {
            let (h, _) = sahte(vec![y]);
            assert!(sms_dogrula(&bag(), "4915123456789", "424242", &*h).is_err());
        }
    }

    #[test]
    fn numara_ozet_ve_dosya() {
        assert!(numara_gecerli(N) && !numara_gecerli("451164") && !numara_gecerli("0451164") && !numara_gecerli("45116434") && !numara_gecerli("45116٤3"));
        assert_eq!(crate::hesap::digest("parola-12345", "a1b2c3d4", 120_000), "ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495");
        let m = ornek();
        assert!(dogrula(&m, "parola-12345") && !dogrula(&m, "parola-12346"));
        let p = tmp("dosya").join("merkez.json");
        assert!(oku(&p).is_none());
        kaydet(&p, &m).unwrap();
        assert_eq!(oku(&p), Some(m));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&p, "{bozuk").unwrap();
        assert!(oku(&p).is_none());
        sil(&p);
        assert!(!p.exists());
    }

    #[test]
    fn giris_basarili_ve_hatalar() {
        let (h, c) = sahte(vec![Ok((200, GIRIS_OK))]);
        let g = giris(N, "parola-12345", "WG=", "ssh-ed25519 AAAA x", "Bocafe", "Boca Ltd.", &*h, 1000.0).unwrap();
        assert_eq!((g.tunel_ip.as_str(), g.yedek_hedefi.as_str(), g.merkez.cihaz_anahtari.as_str()), ("10.99.0.12", "wfc-4511643@10.99.0.1:", "k-123"));
        assert!(dogrula(&g.merkez, "parola-12345") && g.merkez.son_eslesme == 1000.0);
        let (yol, govde) = c.lock().unwrap()[0].clone();
        assert_eq!(yol, "/api/giris");
        assert_eq!((govde["numara"].as_str(), govde["parola"].as_str(), govde["wg_pub"].as_str()), (Some(N), Some("parola-12345"), Some("WG=")));
        let (h, _) = sahte(vec![Ok((401, r#"{"hata":"Müşteri numarası veya parola hatalı.","kod":"hatali"}"#))]);
        assert!(matches!(giris(N, "x", "W", "S", "", "", &*h, 0.0), Err(GirisHata::Mesaj(m, true)) if m.contains("hatalı")));
        let (h, _) = sahte(vec![Ok((409, r#"{"hata":"Bu numara başka bir cihazda kullanılıyor.","kod":"baska_cihaz"}"#))]);
        assert!(matches!(giris(N, "x", "W", "S", "", "", &*h, 0.0), Err(GirisHata::Mesaj(m, false)) if m.contains("başka")));
        let (h, _) = sahte(vec![Err("bağlanılamadı")]);
        assert!(matches!(giris(N, "x", "W", "S", "", "", &*h, 0.0), Err(GirisHata::Baglanti)));
        let (h, _) = sahte(vec![Ok((502, "<html>"))]);
        assert!(matches!(giris(N, "x", "W", "S", "", "", &*h, 0.0), Err(GirisHata::Baglanti)));
        let (h, _) = sahte(vec![Ok((200, r#"{"tuz":"a"}"#))]); // eksik alan
        assert!(matches!(giris(N, "x", "W", "S", "", "", &*h, 0.0), Err(GirisHata::Baglanti)));
    }

    #[test]
    fn uygula_uzak_ve_yedek() {
        let (h, _) = sahte(vec![Ok((200, GIRIS_OK))]);
        let g = giris(N, "parola-12345", "W", "S", "", "", &*h, 0.0).unwrap();
        let mut cfg = Config::default();
        uygula(&mut cfg, &g);
        assert!(cfg.uzak.enabled && cfg.backup.enabled);
        assert_eq!((cfg.uzak.sunucu.as_str(), cfg.uzak.sunucu_anahtar.as_str(), cfg.uzak.adres.as_str()), ("vpn.wificorrect.com:51820", "S", "10.99.0.12"));
        assert_eq!(cfg.backup.target, "wfc-4511643@10.99.0.1:");
    }

    #[test]
    fn eslesme_bagli_serbest_taninmadi() {
        let mut m = ornek();
        let yeni = crate::hesap::digest("yeni-parola-1", "ff", 120_000);
        let govde: &'static str = Box::leak(format!(r#"{{"durum":"bagli","tuz":"ff","ozet":"{yeni}","yineleme":120000,"uyelik":"aktif"}}"#).into_boxed_str());
        let (h, c) = sahte(vec![Ok((200, govde)), Ok((200, r#"{"durum":"serbest"}"#)), Ok((401, r#"{"hata":"Cihaz tanınmadı.","kod":"taninmadi"}"#)), Err("yok")]);
        assert!(matches!(eslesme(&mut m, "Bocafe", "Ltd", "Moda Cad. 1", "otel", false, &*h, 5000.0), Ok(Eslesme::Bagli { uyelik, .. }) if uyelik == "aktif"));
        assert!(dogrula(&m, "yeni-parola-1") && m.son_eslesme == 5000.0);
        let g = c.lock().unwrap()[0].1.clone(); // kopya: kilit tutulursa sonraki sahte çağrı kilitlenir
        assert_eq!((g["cihaz_anahtari"].as_str(), g["isletme_adi"].as_str(), g.get("temizlendi")), (Some("gizli-anahtar"), Some("Bocafe"), None));
        assert_eq!((g["unvan"].as_str(), g["adres"].as_str()), (Some("Ltd"), Some("Moda Cad. 1"))); // veri sorumlusu merkeze
        assert!(matches!(eslesme(&mut m, "", "", "", "", true, &*h, 0.0), Ok(Eslesme::Serbest)));
        assert_eq!(c.lock().unwrap()[1].1["temizlendi"], serde_json::Value::Bool(true));
        assert!(matches!(eslesme(&mut m, "", "", "", "", false, &*h, 0.0), Ok(Eslesme::Taninmadi)));
        assert!(eslesme(&mut m, "", "", "", "", false, &*h, 0.0).is_err());
        assert!(dogrula(&m, "yeni-parola-1")); // hata yanıtları özeti değiştirmez
    }

    #[test]
    fn eslesme_govdesinde_tur() {
        let mut m = ornek();
        let serbest = Ok((200, r#"{"durum":"serbest"}"#));
        let (h, c) = sahte(vec![serbest.clone(), serbest.clone(), serbest]);
        // ctl merkez-eslesme yalnızca seçilmiş türü gönderir: seçilmemiş/tanınmayan → alan yok (yönetimde "—")
        for metin in ["", "[main]\nisletme_turu = 'x'\n", "[main]\nisletme_turu = 'otel'\n"] {
            let cfg = crate::ayar::Config::parse(metin).unwrap();
            eslesme(&mut m, "Bocafe", "Ltd", "", crate::ayar::isletme_turu_secili(&cfg).unwrap_or(""), false, &*h, 0.0).unwrap();
        }
        let c = c.lock().unwrap();
        assert_eq!((c[0].1.get("isletme_turu"), c[1].1.get("isletme_turu")), (None, None));
        assert_eq!(c[2].1["isletme_turu"].as_str(), Some("otel"));
    }

    #[test]
    fn parola_degisimi() {
        let mut m = ornek();
        let yeni = crate::hesap::digest("yeni-parola-1", "ee", 120_000);
        let govde: &'static str = Box::leak(format!(r#"{{"tuz":"ee","ozet":"{yeni}","yineleme":120000}}"#).into_boxed_str());
        let (h, c) = sahte(vec![Ok((401, r#"{"hata":"Mevcut parola yanlış.","kod":"hatali"}"#)), Err("yok"), Ok((200, govde))]);
        assert_eq!(parola(&mut m, "x", "yeni-parola-1", &*h).unwrap_err(), "Mevcut parola yanlış.");
        assert_eq!(parola(&mut m, "parola-12345", "yeni-parola-1", &*h).unwrap_err(), "Parola değişimi için internet gerekli.");
        parola(&mut m, "parola-12345", "yeni-parola-1", &*h).unwrap();
        assert!(dogrula(&m, "yeni-parola-1") && !dogrula(&m, "parola-12345"));
        assert_eq!(c.lock().unwrap()[2].0, "/api/parola");
    }

    #[test]
    fn eslesme_zamani_0600_ve_30_dk() {
        // 2026-10-04 05:00 TR = 02:00 UTC
        let gun = 1_791_072_000.0; // 2026-10-04 00:00 UTC
        let tr = |saat: f64| gun + (saat - 3.0) * 3600.0;
        let mut m = ornek();
        m.son_eslesme = tr(5.0); // 05:00'te bağlandı
        assert!(!zamani_geldi(&m, tr(5.9)) && zamani_geldi(&m, tr(6.0)));
        m.son_eslesme = tr(6.5); // 06:30'da eşitlendi
        assert!(!zamani_geldi(&m, tr(23.0)) && !zamani_geldi(&m, tr(29.9)) && zamani_geldi(&m, tr(30.0))); // ertesi gün 06:00
        m.deneme = tr(30.0); // 06:00 denemesi başarısız
        assert!(!zamani_geldi(&m, tr(30.4)) && zamani_geldi(&m, tr(30.5)));
    }

    #[test]
    fn lisans_ve_hizmet() {
        let gun = 1_791_072_000.0; // 2026-10-04 00:00 UTC (TR 03:00)
        let (h, _) = sahte(vec![Ok((200, GIRIS_OK))]);
        let g = giris(N, "parola-12345", "W", "S", "", "", &*h, gun).unwrap();
        assert_eq!((g.merkez.lisans.as_str(), g.merkez.lisans_bitis.as_str()), ("aktif", "2027-10-04"));
        let mut m = ornek();
        assert!(lisans_aktif(&m, gun)); // eski bağ: alan yok → açık
        m.lisans_bitis = "2026-10-04".into();
        assert!(lisans_aktif(&m, gun + 20.0 * 3600.0)); // bitiş günü (TR 23:00) hâlâ açık
        assert!(!lisans_aktif(&m, gun + 21.5 * 3600.0)); // TR ertesi gün
        m.lisans_bitis = "2027-01-01".into();
        m.lisans = "askida".into();
        assert!(!lisans_aktif(&m, gun));
        m.lisans = "bitti".into();
        assert!(!lisans_aktif(&m, gun));
        m.lisans = "beklemede".into(); // bilinmeyen durum (merkezde yeni bir değer) açık sayılmaz
        assert!(!lisans_aktif(&m, gun));
        let p = tmp("hizmet").join("merkez.json");
        assert!(!hizmet_acik(&p, gun)); // bağlı değil: kapalı
        m.lisans = "aktif".into();
        kaydet(&p, &m).unwrap();
        assert!(hizmet_acik(&p, gun));
        // eşitleme lisansı günceller
        let (h, _) = sahte(vec![Ok((200, r#"{"durum":"bagli","tuz":"a1b2c3d4","ozet":"x","yineleme":120000,"uyelik":"aktif","lisans":"askida","lisans_bitis":"2027-01-01"}"#))]);
        eslesme(&mut m, "", "", "", "", false, &*h, gun).unwrap();
        assert_eq!((m.lisans.as_str(), m.lisans_bitis.as_str()), ("askida", "2027-01-01"));
    }

    #[test]
    fn kapaliyken_30_dk_da_bir_ve_saat_geri_giderse() {
        let gun = 1_791_072_000.0;
        let tr = |saat: f64| gun + (saat - 3.0) * 3600.0;
        let mut m = ornek();
        m.son_eslesme = tr(6.5);
        m.lisans = "askida".into();
        assert!(zamani_geldi(&m, tr(12.0))); // askıda: 06:00 beklenmez (ödeme gelince çabuk açılsın)
        m.deneme = tr(12.0);
        assert!(!zamani_geldi(&m, tr(12.4)) && zamani_geldi(&m, tr(12.5)));
        m.lisans = "aktif".into();
        m.deneme = tr(40.0); // saat ileri sıçramıştı, sonra geri geldi
        assert!(zamani_geldi(&m, tr(30.5)));
    }

    #[test]
    fn curl_parolayi_komut_satirina_koymaz() {
        let a = curl_args("https://api.wificorrect.com/api/giris");
        assert!(a.iter().any(|x| x == "@-") && a.windows(2).any(|w| w[0] == "--connect-timeout" && w[1] == "10"));
        assert!(a.windows(2).any(|w| w[0] == "--max-time" && w[1] == "25") && !a.iter().any(|x| x.contains("parola")));
        // JSON cevapları küçük (rapor dosyası en çok 1 MB); bellekte tutulan gövde sınırsız büyümesin
        assert!(a.windows(2).any(|w| w[0] == "--max-filesize" && w[1] == "2M"));
    }

    #[test]
    fn indir_args_govde_stdin_cevap_dosyaya() {
        let a = indir_args("https://api.wificorrect.com/api/paket", Path::new("/tmp/p.deb.tmp"), 300);
        assert!(a.iter().any(|x| x == "@-") && a.windows(2).any(|w| w[0] == "-o" && w[1] == "/tmp/p.deb.tmp"));
        assert!(a.windows(2).any(|w| w[0] == "--max-time" && w[1] == "300") && a.windows(2).any(|w| w[0] == "--connect-timeout" && w[1] == "10"));
        assert!(a.windows(2).any(|w| w[0] == "-w" && w[1] == "%{http_code}") && a.last().unwrap().ends_with("/api/paket"));
    }

    #[test]
    fn indir_curl_hatasinda_err() {
        assert_eq!(indir_sonuc(true, "200"), Ok(200));
        assert_eq!(indir_sonuc(true, "404\n"), Ok(404));
        // curl 18 (kısmi aktarım) ya da 28 (süre) de kodu 200 yazar: dosya yarımdır
        assert!(indir_sonuc(false, "200").is_err());
        assert!(indir_sonuc(true, "000").is_err() && indir_sonuc(true, "").is_err());
    }

    #[test]
    fn temizlendi_bildirimi_yalnizca_serbest_ya_da_taninmadi_ise_tamam() {
        let m = ornek();
        let say = std::sync::atomic::AtomicUsize::new(0);
        let bekle = || {
            say.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        };
        let (h, _) = sahte(vec![Ok((200, r#"{"durum":"bagli","tuz":"t","ozet":"o","yineleme":120000,"uyelik":"aktif"}"#))]);
        assert!(temizlendi_bildir(&m, &*h, 3, &bekle).is_err()); // merkez serbest bırakmamış: bağ silinmesin
        let (h, _) = sahte(vec![Err("ağ"), Ok((200, r#"{"durum":"serbest"}"#))]);
        assert!(temizlendi_bildir(&m, &*h, 3, &bekle).is_ok());
        assert_eq!(say.load(std::sync::atomic::Ordering::SeqCst), 1); // bir kez beklendi, yeniden denendi
        let (h, _) = sahte(vec![Ok((401, r#"{"hata":"Cihaz tanınmadı.","kod":"taninmadi"}"#))]);
        assert!(temizlendi_bildir(&m, &*h, 3, &bekle).is_ok()); // önceki bildirim ulaşmış ya da zorla ayrılmış
        let (h, _) = sahte(vec![Err("ağ"), Err("ağ"), Err("ağ")]);
        assert!(temizlendi_bildir(&m, &*h, 3, &bekle).is_err());
    }

}
