//! Müşteri giriş portalı (eski portal.py; docs/MASTER_ENGINEERING.md §11, RUST_YENIDEN_YAZIM.md A4).
//! Akış: form → hız sınırları → SMS kodu → doğrulama → nft yetkisi + oturum + 5651 kayıtları.

use crate::ag;
use crate::ayar::Config;
use crate::dil::{self, Dil};
use crate::ortak::{self, Row, Runner, Session};
use crate::pppoe::{self, Durum};
use crate::sms::{self, Saglayici, Sonuc};
use crate::{kimlik, misafir, ulkeler};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

const AFTER_LOGIN_URL: &str = "http://www.google.com/"; // istenen sayfa bilinmiyorsa başarıdan sonra

pub type Form = HashMap<String, String>;
pub type Clock = dyn Fn() -> f64 + Send + Sync;
/// (numara E.164, kod) → sonuç. Kod yönetim merkezine gider (sms::send); Twilio gerçek modda kodu Twilio üretir.
pub type SmsFn = dyn Fn(&str, &str) -> Sonuc + Send + Sync;
/// Twilio Verify doğrulaması merkez üzerinden (sms::check): (onaylandı, ağ hatası).
pub type CheckFn = dyn Fn(&str, &str) -> (bool, Option<String>) + Send + Sync;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner()) // bir istekteki panik diğerlerini kilitlemesin
}

/// Yalnızca http(s) ve ≤ 2048 karakter (açık yönlendirme koruması).
pub fn safe_dst(dst: &str) -> String {
    let d = dst.trim();
    if d.len() <= 2048 && (d.starts_with("http://") || d.starts_with("https://")) {
        d.to_string()
    } else {
        String::new()
    }
}

/// Kayan pencere sayacı. ponytail: anahtarlar bellekten silinmez; kafe ölçeğinde önemsiz, gerekirse periyodik temizlik.
#[derive(Default)]
pub struct RateLimiter(Mutex<HashMap<String, VecDeque<f64>>>);

impl RateLimiter {
    pub fn count(&self, key: &str, window: f64, now: f64) -> usize {
        let mut m = lock(&self.0);
        let q = m.entry(key.to_string()).or_default();
        while q.front().is_some_and(|&t| t <= now - window) {
            q.pop_front();
        }
        q.len()
    }

    pub fn hit(&self, key: &str, now: f64) {
        lock(&self.0).entry(key.to_string()).or_default().push_back(now);
    }
}

#[derive(Debug)]
pub struct Page {
    pub tpl: &'static str,
    pub ctx: Vec<(&'static str, String)>,
    pub dil: Dil,
}

impl Page {
    fn new(tpl: &'static str, ctx: Vec<(&'static str, String)>, dil: Dil) -> Page {
        Page { tpl, ctx, dil }
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> &str {
        self.ctx.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[derive(Clone, Debug)]
struct Pending {
    phone: String,
    ad: String,
    soyad: String,
    code: String,
    dst: String,
    expires: f64,
    attempts: u32,
    sent_at: f64,
    /// Kod Twilio Verify ile gönderildi (merkez deneme modunda değil): doğrulama merkezden; değilse yerelde
    uzak_dogrulama: bool,
    /// Açık rıza kutusu işaretli miydi (oturum kaydına yazılır)
    riza: bool,
    /// Uzaktan doğrulama sürerken kod yenilendiyse sonuç eski koda uygulanmasın
    nonce: u64,
}

/// Oturum açacak giriş: kişi anahtarı (telefon ya da kimlik), ad, soyad; `bitis` verilmişse oturum o anda biter
/// (misafir kodu), yoksa ayardaki oturum süresi. `ek` OTURUM_BASLA'nın `ek` alanına `bitis=…`'ten sonra eklenir.
struct Giris<'a> {
    kisi: &'a str,
    ad: &'a str,
    soyad: &'a str,
    bitis: Option<f64>,
    ek: String,
    /// Misafir kodu: oturum `state_lock` altında açılırken kod yeniden okunur (bu arada iptal edildiyse / süresi
    /// dolduysa oturum açılmaz; uzatıldıysa yeni bitiş kullanılır).
    kod_id: Option<&'a str>,
}

/// Misafir kodu hata sayaçları. Kontrol ve sayma tek kilit altında (8 işçi aynı anda tahmin etse de tavan aşılmaz):
/// deneme önce hata olarak ayrılır, başarılıysa geri alınır. ponytail: anahtarlar bellekten silinmez (otel ölçeği).
#[derive(Default)]
struct MisafirSayac {
    /// `misafir_ip:<ip>` / `misafir_mac:<mac>` → hata zamanları (kayan pencere)
    pencere: HashMap<String, VecDeque<f64>>,
    /// `kod:<kod_id>` (kimliğin etkin kodu yoksa `kimlik:<anahtar>`) → toplam hata; yeni kod yeni sayaç
    kod: HashMap<String, usize>,
    /// Kaba kuvvet kaydı yazılmış kod anahtarları (her kod için bir kez)
    bildirildi: std::collections::HashSet<String>,
}

enum MisafirRed {
    /// IP ya da MAC kilitli
    Kilit,
    /// Bu koda çok fazla hatalı deneme: personel yeni kod üretmeli
    KodTavani,
}

/// `Giris.ek` içinde oturum açılırken kodun güncel bitişiyle değiştirilen yer tutucu.
const KOD_BITIS: &str = "{kod_bitis}";

/// Kayan pencerede (`MISAFIR_KILIT_SN`) kalan hata sayısı; eskiler atılır.
fn pencere_say(p: &mut HashMap<String, VecDeque<f64>>, key: &str, now: f64) -> usize {
    let q = p.entry(key.to_string()).or_default();
    while q.front().is_some_and(|&t| t <= now - MISAFIR_KILIT_SN) {
        q.pop_front();
    }
    q.len()
}

/// Misafir kodu kaba kuvvet koruması: IP ve MAC başına bu kadar hata → kilit (SMS kod sayaçlarından ayrı).
const MISAFIR_HATA_SINIRI: usize = 5;
const MISAFIR_KILIT_SN: f64 = 15.0 * 60.0;

/// Cihazın internet durumu (docs/superpowers/specs/2026-10-23-pppoe-design.md, "Giriş sayfası: internet yokken uyarı ve
/// Servis"): `None` = varsayılan rota var; `Some(durum)` = yok, internet alan portun durumu. Girdi ag.toml'un yolu. Testte sahte.
pub type InternetFn = dyn Fn(&Path) -> Option<Durum> + Send + Sync;

/// İnternet durumu en çok bu kadar önbellekte: giriş sayfası her açılışta komut çalıştırmasın; internet gelince uyarı en geç bu
/// kadar sonra kendiliğinden kalkar.
const INTERNET_ONBELLEK_SN: f64 = 10.0;
/// Servis parolası (AP yönetim parolası) yanlış deneme sınırları: MAC (bilinmiyorsa IP) başına 15 dk'da 5 yanlış → 15 dk kilit;
/// cihaz genelinde 60 dk'da 20 yanlış → 60 dk kilit (MAC değiştirerek sınırsız deneme olmasın). Süre hem pencere hem kilit.
const SERVIS_HATA_SINIRI: usize = 5;
const SERVIS_SURE_SN: f64 = 15.0 * 60.0;
const SERVIS_GENEL_HATA_SINIRI: usize = 20;
const SERVIS_GENEL_SURE_SN: f64 = 60.0 * 60.0;
/// Servis'in hazırladığı PPPoE ayarı (`ag::Yollar::durum`, sistemde /var/lib/wificorrect altında; `ctl pppoe-servis` okur okumaz
/// siler) ve onu uygulayan geçici iş.
const SERVIS_DOSYASI: &str = "pppoe-servis.toml";
const SERVIS_BIRIMI: &str = "wfc-pppoe-servis";
/// Kayıttan sonra durum sayfasının yenilenme aralığı (sn).
const SERVIS_YENILE_SN: u32 = 5;
const SERVIS_TANIMSIZ: &str = "Servis parolası tanımlı değil (AP yönetim parolası bu cihaza henüz gelmedi); Servis kullanılamıyor.";
/// Kayıttan sonraki durum sayfasının mesajı, kaydedilen PPPoE seçimine göre. Açık: pppd yeniden başlar (pppd günlüğünün
/// semantiği yüzünden güncel durum bir dakikayı bulabilir). Kapalı: pppd durur, cihaz modemden DHCP bekler (durum hemen
/// okunur); PPPoE hattında yanlışlıkla kapalı kaydedilmişse çıkış yolu da söylenir. Kullanıcı adı ve parola yazılmaz.
const SERVIS_KAYIT_ACIK: &str = "Ayar kaydedildi; PPPoE yeniden bağlanıyor. Bu sayfa birkaç saniyede bir yenilenip durumu gösterir (güncel durumun \
     gelmesi bir dakikayı bulabilir); internet gelince giriş sayfası açılır.";
const SERVIS_KAYIT_KAPALI: &str = "Ayar kaydedildi; PPPoE kapalı. Cihaz internet portundan modemin adres vermesini bekliyor (PPPoE hattı için \
     Ayarları değiştir bağlantısından PPPoE seçimini Açık yapın). Bu sayfa birkaç saniyede bir yenilenip durumu gösterir; internet gelince giriş \
     sayfası açılır.";

/// Servis formunun kayıtlı ayardan kurulan hâli (ilk çizim; eksik formdan sonra da): **PPPoE seçimi Açık gelir**, kayıtlı ayar kapalı
/// olsa da. Teknisyen Servis'e PPPoE'yi açmak için gelir, kayıtlı ayar ise çoğu kez kapalıdır (PPPoE hiç ayarlanmamış cihaz, ya da
/// kapatılmış); seçim kayıtlı değerle gelseydi formu olduğu gibi gönderen kişi bilgileri PPPoE kapalıyken kaydeder, internet gelmez.
/// Kapatmak için Kapalı elle seçilir. Öteki alanlar kayıtlı değerlerle dolu kalır; kullanıcı adı yine yazılmaz (`servis_formu`'na
/// boş verilir). Panelin Portlar formu bundan etkilenmez: orada seçim kayıtlı değeri gösterir.
fn servis_ilk_alanlar(kayitli: &ag::Pppoe) -> ag::Pppoe {
    ag::Pppoe { acik: true, ..kayitli.clone() }
}

/// Servis parolası yanlış deneme sayaçları (bellekte; portal yeniden başlayınca sıfırlanır, misafir kodu sayaçları gibi).
/// Yalnızca yanlış parola sayılır; sınıra ulaşan yanlış deneme kilidi başlatır, kilit süresi o andan sayılır.
#[derive(Default)]
struct ServisSayac {
    /// `mac:<mac>` / `ip:<ip>` → pencere içindeki yanlış denemeler
    hatalar: HashMap<String, VecDeque<f64>>,
    /// anahtar → kilidin bittiği an
    kilitler: HashMap<String, f64>,
    /// cihaz geneli
    genel: VecDeque<f64>,
    genel_kilit: Option<f64>,
    /// Denetime yazılmış kilitler: anahtar (cihaz geneli kilitte `cihaz`) → o kilidin bitişi. Kilitli istek kilit dönemi başına
    /// bir kez yazılır (kilit sürerken gelen her istek ayrı satır olmasın).
    bildirilen: HashMap<String, f64>,
}

/// Zamanı pencereye ekler, pencereden çıkanları atar; kalan sayı.
fn pencereye_ekle(q: &mut VecDeque<f64>, now: f64, pencere: f64) -> usize {
    q.push_back(now);
    while q.front().is_some_and(|&t| t <= now - pencere) {
        q.pop_front();
    }
    q.len()
}

impl ServisSayac {
    /// Kilitliyse ekranda söylenecek süre (dakika): önce cihaz geneli (uzun olan), sonra anahtarın kendisi.
    fn kilit(&self, anahtar: &str, now: f64) -> Option<u64> {
        if self.genel_kilit.is_some_and(|b| now < b) {
            return Some((SERVIS_GENEL_SURE_SN / 60.0) as u64);
        }
        self.kilitler.get(anahtar).is_some_and(|&b| now < b).then_some((SERVIS_SURE_SN / 60.0) as u64)
    }

    /// Kilitli istek: (ekranda söylenecek dakika, denetime yazılsın mı). Yazma kilit dönemi başına bir kez: cihaz geneli kilitte
    /// cihaz için, anahtar kilidinde o anahtar için. `kilit` gibi önce cihaz geneli; sayma ve kilit davranışı değişmez.
    fn kilitli_istek(&mut self, anahtar: &str, now: f64) -> Option<(u64, bool)> {
        let (kim, bitis, sure) = match self.genel_kilit.filter(|&b| now < b) {
            Some(b) => ("cihaz", b, SERVIS_GENEL_SURE_SN),
            None => (anahtar, self.kilitler.get(anahtar).copied().filter(|&b| now < b)?, SERVIS_SURE_SN),
        };
        let ilk = self.bildirilen.insert(kim.to_string(), bitis) != Some(bitis);
        Some(((sure / 60.0) as u64, ilk))
    }

    /// Yanlış parola: anahtara ve cihaz geneline sayılır; pencerede sınıra ulaşan kilitlenir.
    fn hata(&mut self, anahtar: &str, now: f64) {
        let q = self.hatalar.entry(anahtar.to_string()).or_default();
        if pencereye_ekle(q, now, SERVIS_SURE_SN) >= SERVIS_HATA_SINIRI {
            q.clear();
            self.kilitler.insert(anahtar.to_string(), now + SERVIS_SURE_SN);
        }
        if pencereye_ekle(&mut self.genel, now, SERVIS_GENEL_SURE_SN) >= SERVIS_GENEL_HATA_SINIRI {
            self.genel.clear();
            self.genel_kilit = Some(now + SERVIS_GENEL_SURE_SN);
        }
        // süresi geçenler atılır (cihaz geneli sınır saatte en çok ~20 yanlış sayar: tablolar küçük kalır)
        self.hatalar.retain(|_, q| q.back().is_some_and(|&t| t > now - SERVIS_SURE_SN));
        self.kilitler.retain(|_, b| *b > now);
        self.bildirilen.retain(|_, b| *b > now);
    }

    /// Doğru parola: o anahtarın yanlışları silinir (cihaz geneli sayaç kalır).
    fn dogru(&mut self, anahtar: &str) {
        self.hatalar.remove(anahtar);
    }
}

fn servis_kilit_metni(dakika: u64) -> String {
    format!("Çok fazla yanlış deneme; {dakika} dakika sonra yeniden deneyin.")
}

pub struct Portal {
    pub cfg: Config,
    pending: Mutex<HashMap<String, Pending>>,
    rl: RateLimiter,
    misafir_sayac: Mutex<MisafirSayac>,
    clock: Box<Clock>,
    wall: Box<Clock>,
    runner: Box<Runner>,
    send_sms: Box<SmsFn>,
    check_sms: Box<CheckFn>,
    nonce: AtomicU64,
    pub host: String,
    pub base_url: String,
    /// Yönetim merkezi bağı: bağlı değilse ya da lisans kapalıysa misafire internet verilmez (testte geçici)
    pub merkez_path: std::path::PathBuf,
    /// İnternet durumu sağlayıcısı (giriş sayfasındaki uyarı ve Servis; testte sahte)
    pub internet: Box<InternetFn>,
    /// (yoklama anı, sonuç); en çok `INTERNET_ONBELLEK_SN` geçerli
    internet_onbellek: Mutex<Option<(f64, Option<Durum>)>>,
    /// Aynı anda tek yoklama: o sırada gelen istekler bekleyip sonucu paylaşır
    internet_yoklama: Mutex<()>,
    servis_sayac: Mutex<ServisSayac>,
    /// Hazırlık dosyasının yazılması ve işin başlatılması sıraya girer (iki istek aynı dosyayı karıştırmasın)
    servis_sira: Mutex<()>,
    /// ag.toml (Servis formunun kayıtlı değerleri, bekleyen port değişikliği) ve /var/lib/wificorrect (Servis'in hazırladığı
    /// dosya); testte geçici
    pub ag_yollar: ag::Yollar,
}

fn template(name: &str) -> &'static str {
    match name {
        "_sayfa" => include_str!("sablon/_sayfa.html"),
        "giris" => include_str!("sablon/giris.html"),
        "kod" => include_str!("sablon/kod.html"),
        "basarili" => include_str!("sablon/basarili.html"),
        "hata" => include_str!("sablon/hata.html"),
        "misafir" => include_str!("sablon/misafir.html"),
        "servis" => include_str!("sablon/servis.html"),
        _ => "",
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// `$ad` yer tutucularını doldurur; bilinmeyeni olduğu gibi bırakır (Python string.Template.safe_substitute).
pub fn substitute(tpl: &str, values: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(stripped) = after.strip_prefix('$') {
            out.push('$');
            rest = stripped;
            continue;
        }
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        let name = &after[..n];
        match values.get(name) {
            Some(v) if !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) => out.push_str(v),
            _ => {
                out.push('$');
                out.push_str(name);
            }
        }
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

/// Şablondaki `$m_<anahtar>` → sayfa dilindeki metin (dil.rs; güvenilir sabit, kaçışsız). Metnin içindeki `$ad`
/// yer tutucuları ikinci geçişte (substitute) dolar.
fn yerel(tpl: &str, d: Dil) -> String {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(i) = rest.find("$m_") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 3..];
        let n = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        out.push_str(dil::m(d, &after[..n]));
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub fn metin_html(text: &str) -> String {
    metin_html_dil(text, Dil::Tr)
}

/// Panelden yazılan düz metin → güvenli HTML: boş satır paragraf, tek satır sonu <br>. Boşsa bilgi notu (sayfa dilinde).
pub fn metin_html_dil(text: &str, d: Dil) -> String {
    let text = text.replace("\r\n", "\n");
    let paras: Vec<String> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            // "* " / "• " / "- " ile başlayan satırlar madde listesi; tamamı büyük harf satırlar (başlıklar) kalın
            let mut html = String::new();
            let (mut lines, mut items): (Vec<String>, Vec<String>) = (vec![], vec![]);
            let flush = |html: &mut String, lines: &mut Vec<String>, items: &mut Vec<String>| {
                if !lines.is_empty() {
                    html.push_str(&format!("<p>{}</p>", lines.join("<br>")));
                    lines.clear();
                }
                if !items.is_empty() {
                    html.push_str(&format!("<ul>{}</ul>", items.iter().map(|i| format!("<li>{i}</li>")).collect::<String>()));
                    items.clear();
                }
            };
            for l in p.lines().map(str::trim) {
                if let Some(item) = l.strip_prefix("* ").or_else(|| l.strip_prefix("• ")).or_else(|| l.strip_prefix("- ")) {
                    if !lines.is_empty() {
                        flush(&mut html, &mut lines, &mut vec![]);
                    }
                    items.push(html_escape(item.trim()));
                } else {
                    if !items.is_empty() {
                        flush(&mut html, &mut vec![], &mut items);
                    }
                    let e = html_escape(l);
                    let heading = l.chars().any(char::is_alphabetic) && !l.chars().any(char::is_lowercase);
                    lines.push(if heading { format!("<strong>{e}</strong>") } else { e });
                }
            }
            flush(&mut html, &mut lines, &mut items);
            html
        })
        .collect();
    if paras.is_empty() { format!("<p class=\"not\">{}</p>", dil::m(d, "metin_yok")) } else { paras.concat() }
}

fn tr_lower(s: &str) -> String {
    s.chars().map(|c| match c { 'I' => "ı".into(), 'İ' => "i".into(), c => c.to_lowercase().to_string() }).collect()
}

fn tr_upper(s: &str) -> String {
    s.chars().map(|c| match c { 'i' => "İ".into(), 'ı' => "I".into(), c => c.to_uppercase().to_string() }).collect()
}

/// Ada uygun ek (ünlü uyumu): 'y' yönelme (’ye/’ya), 'n' tamlayan (’nin/’ın/’un/’ün), 'd' ayrılma (’den/’tan).
/// ponytail: yazımdan çıkarılır; okunuşu farklı yabancı adlarda (Starbucks) ek yazıma göre olur.
fn ek(name: &str, kind: char) -> String {
    const UNLU: &str = "aeıioöuü";
    let l: Vec<char> = tr_lower(name).chars().filter(|c| c.is_alphabetic()).collect();
    let last_v = l.iter().rev().find(|c| UNLU.contains(**c)).copied().unwrap_or('e');
    let ends_vowel = l.last().is_some_and(|c| UNLU.contains(*c));
    let a2 = if "aıou".contains(last_v) { 'a' } else { 'e' };
    let i4 = match last_v {
        'a' | 'ı' => 'ı',
        'o' | 'u' => 'u',
        'ö' | 'ü' => 'ü',
        _ => 'i',
    };
    match kind {
        'y' => format!("{}{a2}", if ends_vowel { "y" } else { "" }),
        'n' => format!("{}{i4}n", if ends_vowel { "n" } else { "" }),
        _ => format!("{}{a2}n", if l.last().is_some_and(|c| "çfhkpsşt".contains(*c)) { 't' } else { 'd' }),
    }
}

/// Veri sorumlusu yer tutucuları (KVKK aydınlatma ve açık rıza metinleri; panel Ayarlar → İşletme):
/// zorunlu olanlar (unvan, adres) girilmemişse yer tutucu görünür kalır (Özet uyarır); isteğe bağlı olanlar boşsa
/// yer tutucunun geçtiği satır bütünüyle çıkar (boş etiket görünmesin).
pub const YER_UNVAN: &str = "[vergi levhası unvanı]";
pub const YER_ADRES: &str = "[başvuru adresi]";
const YER_ISTEGE_BAGLI: [&str; 3] = ["[irtibat numarası]", "[e-posta adresi]", "[KEP adresi]"];

pub fn veri_sorumlusu_yerlestir(text: &str, m: &crate::ayar::Main) -> String {
    let istege_bagli = [m.telefon.trim(), m.eposta.trim(), m.kep.trim()];
    let mut t: String = text
        .split('\n')
        .filter(|satir| !YER_ISTEGE_BAGLI.iter().zip(istege_bagli).any(|(y, v)| v.is_empty() && satir.contains(y)))
        .collect::<Vec<_>>()
        .join("\n");
    for (y, v) in [(YER_UNVAN, m.unvan.trim()), (YER_ADRES, m.adres.trim())].into_iter().chain(YER_ISTEGE_BAGLI.into_iter().zip(istege_bagli)) {
        if !v.is_empty() {
            t = t.replace(y, v);
        }
    }
    t
}

/// Metindeki "İŞLETMECİ" → kafe adı; kesme işaretinden sonraki ek ada göre yeniden kurulur, büyük harfli yerde ad da büyük.
pub fn isletme_yerlestir(text: &str, name: &str) -> String {
    const KELIME: &str = "İŞLETMECİ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(KELIME) {
        out.push_str(&rest[..i]);
        rest = &rest[i + KELIME.len()..];
        let apos = rest.chars().next().filter(|c| *c == '’' || *c == '\'');
        let suffix: String = apos.map_or(String::new(), |a| rest[a.len_utf8()..].chars().take_while(|c| c.is_alphabetic()).collect());
        let upper = !suffix.is_empty() && !suffix.chars().any(char::is_lowercase);
        let kind = match tr_lower(&suffix).chars().next() {
            Some('y') => Some('y'),
            Some('n') => Some('n'),
            Some('d') | Some('t') => Some('d'),
            _ => None,
        };
        match (apos, kind) {
            (Some(a), Some(k)) => {
                let e = ek(name, k);
                out.push_str(&if upper { format!("{}{a}{}", tr_upper(name), tr_upper(&e)) } else { format!("{name}{a}{e}") });
                rest = &rest[a.len_utf8() + suffix.len()..];
            }
            _ => out.push_str(name),
        }
    }
    out.push_str(rest);
    out
}

/// Sabit sürede karşılaştırma (zamanlama ile kod tahminini önler).
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Servis parolası sabit sürede karşılaştırılır: iki taraf da SHA-256 özetine çevrilir (uzunluk farkı ve ilk farklı karakterin
/// yeri süreye yansımaz).
fn servis_parolasi_dogru(girilen: &str, beklenen: &str) -> bool {
    use sha2::{Digest, Sha256};
    let (a, b) = (Sha256::digest(girilen.as_bytes()), Sha256::digest(beklenen.as_bytes()));
    a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `ip -o route show default` çıktısından karar (spec: internet yok = kullanılabilir varsayılan rota yok; dhcpcd'nin IPv4LL rotası
/// sayılmaz, `ag::internet_var`). Çıktı yoksa (komut çalışmadı, zaman aşımı, hata koduyla bitti) internet var sayılır: emin
/// olunamayınca misafire açık sayfada Servis açılmaz. Rota yoksa internet alan portun durumu (`durum` yalnızca o zaman okunur).
fn internet_coz(rota: Option<&str>, durum: impl FnOnce() -> Durum) -> Option<Durum> {
    match rota {
        Some(r) if !ag::internet_var(r) => Some(durum()),
        _ => None,
    }
}

/// Sistemdeki sağlayıcı: varsayılan rota `ip`'ten, durum `pppoe::durum` ile (kablo, PPP adresi, pppd günlüğü); komutlar süre sınırlı.
fn internet_sistem(ag_toml: &Path) -> Option<Durum> {
    internet_yokla(ag_toml, Path::new("/sys/class/net"), &rota_oku_sistem, &|c: &[&str]| ortak::capture_timeout(c, 5))
}

/// Sistemdeki varsayılan rota okuyucusu: süre sınırlı ve çıkış koduna bakar (hata kodu = sonuç yok, "rota yok" değil).
fn rota_oku_sistem(c: &[&str]) -> Option<String> {
    ortak::capture_timeout_basarili(c, 5)
}

/// `internet_sistem`'in gövdesi (testte yollar ve komutlar verilir). Rota `rota_oku` ile okunur (sistemde `capture_timeout_basarili`):
/// `ip` çalışıp hata koduyla biterse de sonuç yoktur, internet var sayılır (boş çıktı "rota yok" demek değildir). Durum
/// komutları (`komut`) çıkış koduna bakmaz (`pppoe::durum` boş çıktıyı zaten "adres yok / iz yok" okur).
fn internet_yokla(ag_toml: &Path, sys: &Path, rota_oku: &dyn Fn(&[&str]) -> Option<String>, komut: &dyn Fn(&[&str]) -> Option<String>) -> Option<Durum> {
    let rota = rota_oku(&["ip", "-o", "route", "show", "default"]);
    internet_coz(rota.as_deref(), || pppoe::durum(&ag::load(ag_toml), sys, komut))
}

/// Sayfanın altındaki uyarı (herkese), yalnızca internet yokken: neden (spec tablosundaki durum metni) + Servis bağlantısı.
fn internet_uyarisi(durum: &Durum, d: Dil) -> String {
    format!(
        "<p class=\"uyari\"><b>{}</b><br>{}<br><a href=\"/servis\">{}</a></p>",
        dil::m(d, "internet_yok"),
        html_escape(&durum.metin()),
        dil::m(d, "servis")
    )
}

fn otp_code() -> String {
    loop {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
        let x = u32::from_le_bytes(b);
        if x < 4_294_000_000 {
            // eşit dağılım için 10^6'nın katının üstü atılır
            return format!("{:06}", x % 1_000_000);
        }
    }
}

impl Portal {
    pub fn new(cfg: Config, clock: Box<Clock>, wall: Box<Clock>, runner: Box<Runner>, send_sms: Box<SmsFn>, check_sms: Box<CheckFn>) -> Portal {
        let host = format!("{}:{}", cfg.main.router_ip, cfg.main.portal_port);
        Portal {
            base_url: format!("http://{host}"),
            host,
            cfg,
            pending: Mutex::default(),
            rl: RateLimiter::default(),
            misafir_sayac: Mutex::default(),
            clock,
            wall,
            runner,
            send_sms,
            check_sms,
            nonce: AtomicU64::new(1),
            merkez_path: crate::merkez::PATH.into(),
            internet: Box::new(internet_sistem),
            internet_onbellek: Mutex::default(),
            internet_yoklama: Mutex::default(),
            servis_sayac: Mutex::default(),
            servis_sira: Mutex::default(),
            ag_yollar: ag::Yollar::sistem(),
        }
    }

    /// Bekleyen koddan formu yeniden doldurmak için (numara `+` ile, ülke seçimi yok sayılır).
    fn prefill(p: &Pending, d: Dil) -> Form {
        let riza = if p.riza { "1" } else { "" };
        [("ad", p.ad.clone()), ("soyad", p.soyad.clone()), ("telefon", format!("+{}", p.phone)), ("sozlesme", "1".into()), ("riza", riza.into()), ("dst", p.dst.clone()), ("dil", d.kod().into())]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    // --- sayfalar
    pub fn render(&self, page: &Page) -> String {
        let d = page.dil;
        // internet yokken telefonun düşebileceği her sayfanın altında uyarı + Servis bağlantısı (giriş formu, kod, başarı, hata,
        // misafir kodu); Servis sayfasının kendi durum satırı var. Durum önbellekten.
        let internet_yok = if page.tpl == "servis" { None } else { self.internet_yok() };
        let mut values: HashMap<&str, String> = HashMap::new();
        values.insert("dil_kod", d.kod().into());
        values.insert("site", self.cfg.main.site_name.clone());
        // "Bocafe'ye", "Starbucks'a": ek ünlü uyumuyla, kesme işaretiyle ayrılır
        values.insert("site_ye", format!("{}'{}", self.cfg.main.site_name, ek(&self.cfg.main.site_name, 'y')));
        values.insert("portal", self.base_url.clone());
        for (k, v) in &page.ctx {
            values.insert(k, v.clone());
        }
        let safe: HashMap<&str, String> =
            values.into_iter().map(|(k, v)| (k, if k.ends_with("_html") { v } else { html_escape(&v) })).collect();
        let body = substitute(&yerel(template(page.tpl), d), &safe);
        let mut outer = HashMap::new();
        outer.insert("icerik_html", body);
        outer.insert("site", safe["site"].clone());
        // giriş yapmış telefonun başarı sayfası internet yokken yönlenmez: hedef açılmaz, uyarı ve Servis bağlantısı okunabilsin
        let yonlenmez = page.tpl == "basarili" && internet_yok.is_some();
        outer.insert("bas_html", if yonlenmez { String::new() } else { safe.get("bas_html").cloned().unwrap_or_default() });
        outer.insert("internet_uyari_html", internet_yok.map_or_else(String::new, |durum| internet_uyarisi(&durum, d)));
        outer.insert("dil_kod", d.kod().into());
        outer.insert("dil_secim_html", self.dil_secimi(page));
        substitute(&yerel(template("_sayfa"), d), &outer)
    }

    /// Sayfanın üstündeki "TR | EN": istenen adres (`dst`) korunur; kod sayfasındaysa `sayfa=kod` ile kod sayfası
    /// yeni dilde yeniden açılır (bekleyen kod bellekte).
    fn dil_secimi(&self, page: &Page) -> String {
        if page.tpl == "servis" {
            return String::new(); // Servis yalnızca Türkçe (teknisyen sayfası)
        }
        let yol = if page.tpl == "misafir" { "/misafir" } else { "/" };
        let mut ek = String::new();
        if page.tpl == "kod" {
            ek.push_str("&amp;sayfa=kod");
        }
        let dst = page.ctx.iter().find(|(k, _)| *k == "dst").map_or("", |(_, v)| v.as_str());
        if !dst.is_empty() {
            ek.push_str(&format!("&amp;dst={}", pct_encode(dst)));
        }
        let a = |x: Dil, ad: &str| {
            let cur = if x == page.dil { " aria-current=\"true\"" } else { "" };
            format!("<a href=\"{}{yol}?dil={}{ek}\" lang=\"{}\"{cur}>{ad}</a>", html_escape(&self.base_url), x.kod(), x.kod())
        };
        format!("<nav class=\"dil\" aria-label=\"{}\">{} | {}</nav>", dil::m(page.dil, "dil_secimi"), a(Dil::Tr, "TR"), a(Dil::En, "EN"))
    }

    #[cfg(test)]
    fn form_page(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str) -> Page {
        self.form_page_dil(form, errors, dst, Dil::Tr)
    }

    fn form_page_dil(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str, d: Dil) -> Page {
        let f = |k: &str| form.get(k).cloned().unwrap_or_default();
        let pm = &self.cfg.portal;
        let mut ctx = vec![
            ("ad", f("ad")),
            ("soyad", f("soyad")),
            ("telefon", f("telefon")),
            ("sozlesme_checked", if f("sozlesme") == "1" { "checked".into() } else { String::new() }),
            ("riza_checked", if f("riza") == "1" { "checked".into() } else { String::new() }),
            // açık rıza metni onay kutusunun yanında (kısa metin; satır sonları korunur)
            ("riza_metni_html", html_escape(&self.hukuki(d, &pm.acik_riza, &pm.acik_riza_en)).replace('\n', "<br>")),
            ("sozlesme_html", metin_html_dil(&self.hukuki(d, &pm.sozlesme, &pm.sozlesme_en), d)),
            ("aydinlatma_html", metin_html_dil(&self.hukuki(d, &pm.aydinlatma, &pm.aydinlatma_en), d)),
            ("dst", dst.to_string()),
            ("ulke_secenekleri_html", ulkeler::options_html(form.get("ulke").map_or("TR", String::as_str))),
            ("misafir_html", self.misafir_dugmesi(dst, d)),
        ];
        for k in ["hata_genel", "hata_ad", "hata_soyad", "hata_telefon", "hata_riza", "hata_sozlesme"] {
            let v = errors.iter().find(|(e, _)| *e == k).map(|(_, v)| v.clone()).unwrap_or_default();
            ctx.push((k, v));
        }
        Page::new("giris", ctx, d)
    }

    /// Metinlerdeki İŞLETMECİ → işletme adı, veri sorumlusu yer tutucuları → Ayarlar'daki bilgiler
    /// (`veri_sorumlusu_yerlestir`).
    fn yerlestir(&self, text: &str) -> String {
        self.unvan_yerlestir(isletme_yerlestir(text, &self.cfg.main.site_name))
    }

    fn unvan_yerlestir(&self, t: String) -> String {
        veri_sorumlusu_yerlestir(&t, &self.cfg.main)
    }

    /// Hukuki metin sayfa dilinde: İngilizcesi yazılmamışsa Türkçesi. İngilizcede İŞLETMECİ yalnızca adla değişir (ek yok).
    fn hukuki(&self, d: Dil, tr: &str, en: &str) -> String {
        if d == Dil::En && !en.trim().is_empty() {
            self.unvan_yerlestir(en.replace("İŞLETMECİ", &self.cfg.main.site_name))
        } else {
            self.yerlestir(tr)
        }
    }

    fn code_page(&self, p: &Pending, note: &str, error: &str, d: Dil) -> Page {
        let dakika = (self.cfg.limits.otp_ttl_sec / 60).max(1);
        Page::new(
            "kod",
            vec![
                ("telefon_maskeli", ulkeler::mask_phone(&p.phone)),
                ("dakika", dakika.to_string()),
                ("not", note.into()),
                ("hata", error.into()),
            ],
            d,
        )
    }

    fn error_page(&self, msg: &str, d: Dil) -> Page {
        Page::new("hata", vec![("mesaj", msg.into())], d)
    }

    /// JS yok (CSP): 1 sn sonra yönlen. Telefonların giriş penceresi interneti görünce kendini kapatır.
    fn success_page(&self, dst: &str, d: Dil) -> Page {
        let target = html_escape(if dst.is_empty() { AFTER_LOGIN_URL } else { dst });
        Page::new("basarili", vec![("bas_html", format!("<meta http-equiv=\"refresh\" content=\"1;url={target}\">"))], d)
    }

    fn audit(&self, olay: &str, ip: &str, mac: &str, phone: &str, ek: &str) {
        let row = Row::new(olay, &ortak::now_iso((self.wall)()))
            .set("telefon", phone)
            .set("mac", mac)
            .set("ic_ip", ip)
            .set("ek", ek);
        ortak::audit(&self.cfg.main.log_root, row);
    }

    fn session_for(&self, ip: &str, mac: &str) -> bool {
        ortak::load_sessions(&self.cfg.main.state_root)
            .get(mac)
            .is_some_and(|s| s.ip == ip && s.expires_epoch > (self.wall)())
    }

    // --- akış
    /// Lisans kapalı / cihaz bağlı değil: misafir giremez (yeni kod gönderilmez).
    fn hizmet_kapali(&self, d: Dil) -> Option<Page> {
        (!crate::merkez::hizmet_acik(&self.merkez_path, (self.wall)())).then(|| self.error_page(dil::m(d, "hizmet_kapali"), d))
    }

    #[cfg(test)]
    pub fn home(&self, ip: &str, mac: Option<&str>, dst: &str) -> Page {
        self.home_dil(ip, mac, dst, Dil::Tr, false)
    }

    /// `kod_sayfasi`: kod sayfasındaki dil bağlantısından gelindi; süresi dolmamış bekleyen kod varsa kod sayfası açılır.
    pub fn home_dil(&self, ip: &str, mac: Option<&str>, dst: &str, d: Dil, kod_sayfasi: bool) -> Page {
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        if self.session_for(ip, mac) || self.rehome(ip, mac) {
            return self.success_page(dst, d);
        }
        if kod_sayfasi {
            let now = (self.clock)();
            let p = lock(&self.pending).get(mac).filter(|p| now < p.expires).cloned();
            if let Some(p) = p {
                return self.code_page(&p, "", "", d);
            }
        }
        self.form_page_dil(&Form::new(), vec![], dst, d)
    }

    /// Oturumu süren cihaz DHCP'den yeni IP ile döndü: SMS istemeden oturumu yeni IP'ye taşır.
    fn rehome(&self, ip: &str, mac: &str) -> bool {
        let root = &self.cfg.main;
        let Ok(_g) = ortak::state_lock(&root.state_root) else { return false };
        let mut ses = ortak::load_sessions(&root.state_root);
        let now = (self.wall)();
        let movable = ses.get(mac).is_some_and(|s| s.expires_epoch > now && s.ip != ip);
        if !movable || !ortak::move_session(&root.log_root, &mut ses, mac, ip, now, &*self.runner) {
            return false;
        }
        ortak::save_sessions(&root.state_root, &ses).is_ok()
    }

    fn refusal(&self, pending: &HashMap<String, Pending>, mac: &str, phone: &str, now: f64, day: &str, d: Dil) -> Option<(&'static str, String)> {
        let l = &self.cfg.limits;
        let cooldown = l.resend_cooldown_sec as f64;
        if let Some(p) = pending.get(mac) {
            if now - p.sent_at < cooldown {
                let wait = (cooldown - (now - p.sent_at)) as i64 + 1;
                return Some(("bekleme", dil::f(d, "bekleme", &[&wait.to_string()])));
            }
        }
        let tel = format!("tel:{phone}");
        if self.rl.count(&tel, 900.0, now) >= l.sms_per_phone_15min {
            return Some(("tel_15dk", dil::m(d, "tel_15dk").into()));
        }
        if self.rl.count(&tel, 86400.0, now) >= l.sms_per_phone_day {
            return Some(("tel_gun", dil::m(d, "tel_gun").into()));
        }
        if self.rl.count(&format!("mac:{mac}"), 3600.0, now) >= l.sms_per_mac_hour {
            return Some(("mac_saat", dil::m(d, "mac_saat").into()));
        }
        if ortak::sms_count(&self.cfg.main.state_root, day) >= l.sms_global_day {
            return Some(("tavan", dil::m(d, "tavan").into()));
        }
        None
    }

    /// Formdaki `dil` alanı (HTTP katmanı seçilen dili yazar); yoksa Türkçe.
    fn form_dil(form: &Form) -> Dil {
        form.get("dil").and_then(|v| Dil::coz(v)).unwrap_or_default()
    }

    pub fn send_code(&self, ip: &str, mac: Option<&str>, form: &Form) -> Page {
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let dst = safe_dst(get("dst"));
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        let ulke = if get("ulke").is_empty() { "TR" } else { get("ulke") };
        let (ad, soyad, phone) = (ortak::clean_name(get("ad")), ortak::clean_name(get("soyad")), ulkeler::normalize_phone(ulke, get("telefon")));
        let mut errors = vec![];
        if ad.is_none() {
            errors.push(("hata_ad", dil::m(d, "hata_ad").to_string()));
        }
        if soyad.is_none() {
            errors.push(("hata_soyad", dil::m(d, "hata_soyad").to_string()));
        }
        if phone.is_none() {
            let k = if ulke == "TR" && !get("telefon").trim_start().starts_with('+') { "hata_telefon_tr" } else { "hata_telefon" };
            errors.push(("hata_telefon", dil::m(d, k).to_string()));
        }
        if get("sozlesme") != "1" {
            errors.push(("hata_sozlesme", dil::m(d, "hata_sozlesme").to_string()));
        }
        if get("riza") != "1" {
            errors.push(("hata_riza", dil::m(d, "hata_riza").to_string()));
        }
        let (Some(ad), Some(soyad), Some(phone), true) = (ad, soyad, phone, errors.is_empty()) else {
            return self.form_page_dil(form, errors, &dst, d);
        };
        let now = (self.clock)();
        let day = ortak::day_of(&ortak::now_iso((self.wall)())).to_string();
        let cooldown = self.cfg.limits.resend_cooldown_sec as f64;
        let code = {
            let pending = lock(&self.pending);
            if let Some(p) = pending.get(mac) {
                if p.phone == phone && now - p.sent_at < cooldown && now < p.expires {
                    let wait = (cooldown - (now - p.sent_at)) as i64 + 1;
                    return self.code_page(p, &dil::f(d, "kod_az_once", &[&wait.to_string()]), "", d);
                }
            }
            if let Some((kind, msg)) = self.refusal(&pending, mac, &phone, now, &day, d) {
                drop(pending);
                let olay = if kind == "tavan" { "SMS_TAVAN" } else { "OTP_ISTEK" };
                self.audit(olay, ip, mac, &phone, &format!("red={kind}"));
                return self.form_page_dil(form, vec![("hata_genel", msg)], &dst, d);
            }
            self.rl.hit(&format!("tel:{phone}"), now);
            self.rl.hit(&format!("mac:{mac}"), now);
            ortak::sms_count_inc(&self.cfg.main.state_root, &day);
            otp_code()
        };
        // ağ çağrısı kilit dışında; numara cihazda süzülmez, yabancı numarayı merkez reddeder (YABANCI)
        let r = (self.send_sms)(&phone, &code);
        if !r.ok {
            if r.kod == "YABANCI" {
                self.audit("OTP_ISTEK", ip, mac, &phone, "red=yabanci_numara");
            } else {
                self.audit("OTP_HATA", ip, mac, &phone, &format!("{}={}", r.saglayici.ad(), r.kod));
            }
            return self.form_page_dil(form, vec![("hata_genel", dil::cevir(d, sms::message_for(r.saglayici, &r.kod)))], &dst, d);
        }
        let p = Pending {
            phone: phone.clone(),
            ad,
            soyad,
            code,
            dst,
            expires: now + self.cfg.limits.otp_ttl_sec as f64,
            attempts: 0,
            sent_at: now,
            uzak_dogrulama: r.saglayici == Saglayici::Twilio && !r.mock,
            riza: get("riza") == "1",
            nonce: self.nonce.fetch_add(1, Ordering::Relaxed),
        };
        let page = self.code_page(&p, "", "", d);
        lock(&self.pending).insert(mac.to_string(), p);
        let job = r.is.as_deref().unwrap_or("None");
        self.audit("OTP_GONDERILDI", ip, mac, &phone, &format!("{}={job}", r.saglayici.ad()));
        page
    }

    #[cfg(test)]
    pub fn resend(&self, ip: &str, mac: Option<&str>) -> Page {
        self.resend_dil(ip, mac, Dil::Tr)
    }

    pub fn resend_dil(&self, ip: &str, mac: Option<&str>, d: Dil) -> Page {
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let p = mac.and_then(|m| lock(&self.pending).get(m).cloned());
        let Some(p) = p else {
            return self.form_page_dil(&Form::new(), vec![("hata_genel", dil::m(d, "kod_yok_bilgi").into())], "", d);
        };
        self.send_code(ip, mac, &Self::prefill(&p, d))
    }

    pub fn verify(&self, ip: &str, mac: Option<&str>, form: &Form) -> Page {
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return pg;
        }
        let Some(mac) = mac else { return self.error_page(dil::m(d, "mac_yok"), d) };
        let now = (self.clock)();
        let fail_lock = self.cfg.limits.verify_fail_lock_min * 60;
        let fail_key = format!("hata:{mac}");
        if self.rl.count(&fail_key, fail_lock as f64, now) >= 10 {
            return self.error_page(&dil::f(d, "kilit", &[&(fail_lock / 60).to_string()]), d);
        }
        let not_found = || self.form_page_dil(&Form::new(), vec![("hata_genel", dil::m(d, "kod_yok").into())], "", d);
        let code: String = form.get("kod").map(|k| k.chars().filter(char::is_ascii_digit).collect()).unwrap_or_default();
        // 1) bekleyen kod
        let p = {
            let mut pending = lock(&self.pending);
            let Some(p) = pending.get(mac).cloned() else { return not_found() };
            if now >= p.expires {
                pending.remove(mac);
                return self.form_page_dil(&Self::prefill(&p, d), vec![("hata_genel", dil::m(d, "kod_suresi").into())], &p.dst, d);
            }
            p
        };
        // 2) karşılaştır: Twilio gerçek modda kodu Twilio doğrular (merkez üzerinden, ağ çağrısı kilit dışında); diğerlerinde yerelde
        let matched = if p.uzak_dogrulama {
            // Twilio kodu 6 rakam: başka biçim merkeze gitmeden yanlış kod sayılır (merkez 400 verirdi, cihaz bunu ağ hatası
            // sayıp deneme hakkını düşürmezdi: sınırsız deneme)
            if code.len() != 6 {
                false
            } else {
                let (ok, err) = (self.check_sms)(&p.phone, &code);
                if err.is_some() {
                    return self.code_page(&p, "", dil::m(d, "dogrulanamadi"), d);
                }
                ok
            }
        } else {
            ct_eq(&code, &p.code)
        };
        // 3) sonucu uygula — bu arada kod yenilendiyse eski sonuç uygulanmaz
        let wrong_page = {
            let mut pending = lock(&self.pending);
            let Some(cur) = pending.get_mut(mac).filter(|c| c.nonce == p.nonce) else { return not_found() };
            if matched {
                pending.remove(mac);
                None
            } else {
                cur.attempts += 1;
                self.rl.hit(&fail_key, now);
                let left = self.cfg.limits.otp_max_attempts.saturating_sub(cur.attempts);
                Some(if left == 0 {
                    pending.remove(mac);
                    self.form_page_dil(&Self::prefill(&p, d), vec![("hata_genel", dil::m(d, "cok_hatali").into())], &p.dst, d)
                } else {
                    self.code_page(&p, "", &dil::f(d, "kod_hatali", &[&left.to_string()]), d)
                })
            }
        };
        if let Some(page) = wrong_page {
            self.audit("OTP_HATALI_KOD", ip, mac, &p.phone, "");
            return page;
        }
        let g = Giris { kisi: &p.phone, ad: &p.ad, soyad: &p.soyad, bitis: None, ek: format!("sozlesme=1 riza={}", u8::from(p.riza)), kod_id: None };
        let Some(s) = self.open_session(ip, mac, &g) else {
            return self.error_page(dil::m(d, "baglanti_acilamadi"), d);
        };
        self.audit("OTP_BASARILI", ip, mac, &p.phone, &format!("oturum={}", s.session_id));
        self.success_page(&p.dst, d)
    }

    // --- misafir kodu (otel / yurt)
    /// Giriş sayfasındaki ikinci yol; misafir kodu kapalıysa (ya da tür otel/yurt değilse) boş.
    fn misafir_dugmesi(&self, dst: &str, d: Dil) -> String {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return String::new();
        }
        let dst = if dst.is_empty() { String::new() } else { format!("&amp;dst={}", pct_encode(dst)) };
        format!("<a class=\"dugme\" href=\"{}/misafir?dil={}{dst}\">{}</a>", html_escape(&self.base_url), d.kod(), html_escape(self.misafir_adi(d)))
    }

    /// Düğme / sayfa başlığı: Otel "Otel misafiriyim", Yurt "Yurt sakiniyim".
    fn misafir_adi(&self, d: Dil) -> &'static str {
        dil::m(d, if crate::ayar::isletme_turu(&self.cfg) == "yurt" { "misafir_dugme_yurt" } else { "misafir_dugme_otel" })
    }

    fn misafir_form(&self, form: &Form, errors: Vec<(&'static str, String)>, dst: &str, d: Dil) -> Page {
        let f = |k: &str| form.get(k).cloned().unwrap_or_default();
        let pm = &self.cfg.portal;
        let tur = f("tur");
        let uyruk = f("uyruk");
        let secili = |a: &str| if tur == a || (a == "tc" && tur.is_empty()) { "checked".to_string() } else { String::new() };
        let uyruklar: String = std::iter::once(format!("<option value=\"\">{}</option>", dil::m(d, "uyruk_secin")))
            .chain(ulkeler::ULKELER.iter().map(|(iso, _, ad)| {
                let sel = if *iso == uyruk { " selected" } else { "" };
                format!("<option value=\"{iso}\"{sel}>{} {iso} · {}</option>", ulkeler::flag(iso), html_escape(ad))
            }))
            .collect();
        let mut ctx = vec![
            ("baslik", self.misafir_adi(d).to_string()),
            ("tur_tc", secili("tc")),
            ("tur_pasaport", secili("pasaport")),
            ("tc", f("tc")),
            ("pasaport", f("pasaport")),
            ("uyruk_secenekleri_html", uyruklar),
            ("sozlesme_checked", if f("sozlesme") == "1" { "checked".into() } else { String::new() }),
            ("riza_checked", if f("riza") == "1" { "checked".into() } else { String::new() }),
            ("riza_metni_html", html_escape(&self.hukuki(d, &pm.acik_riza, &pm.acik_riza_en)).replace('\n', "<br>")),
            ("sozlesme_html", metin_html_dil(&self.hukuki(d, &pm.sozlesme, &pm.sozlesme_en), d)),
            ("aydinlatma_html", metin_html_dil(&self.hukuki(d, &pm.aydinlatma, &pm.aydinlatma_en), d)),
            ("dst", dst.to_string()),
        ];
        for k in ["hata_genel", "hata_tur", "hata_tc", "hata_pasaport", "hata_uyruk", "hata_kod", "hata_riza", "hata_sozlesme"] {
            let v = errors.iter().find(|(e, _)| *e == k).map(|(_, v)| v.clone()).unwrap_or_default();
            ctx.push((k, v));
        }
        Page::new("misafir", ctx, d)
    }

    /// GET /misafir. `None`: misafir kodu bu işletmede yok (HTTP katmanı ana sayfaya yönlendirir).
    pub fn misafir_sayfasi(&self, ip: &str, mac: Option<&str>, dst: &str, d: Dil) -> Option<Page> {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return None;
        }
        if let Some(pg) = self.hizmet_kapali(d) {
            return Some(pg);
        }
        let Some(mac) = mac else { return Some(self.error_page(dil::m(d, "mac_yok"), d)) };
        if self.session_for(ip, mac) || self.rehome(ip, mac) {
            return Some(self.success_page(dst, d));
        }
        Some(self.misafir_form(&Form::new(), vec![], dst, d))
    }

    /// IP ya da MAC kilitli mi (yalnızca bakar; sayma `misafir_ayir`'da).
    fn misafir_kilitli(&self, ip_key: &str, mac_key: &str, now: f64) -> bool {
        let mut sy = lock(&self.misafir_sayac);
        [ip_key, mac_key].iter().any(|k| pencere_say(&mut sy.pencere, k, now) >= MISAFIR_HATA_SINIRI)
    }

    /// Kontrol + sayma tek kilitte: tavanlar doluysa red, değilse deneme hata olarak ayrılır (başarıda geri alınır).
    /// Kod tavanında reddedilen deneme de IP/MAC hatası sayılır.
    fn misafir_ayir(&self, ip_key: &str, mac_key: &str, kod_key: &str, now: f64) -> Result<(), MisafirRed> {
        let mut sy = lock(&self.misafir_sayac);
        if [ip_key, mac_key].iter().any(|k| pencere_say(&mut sy.pencere, k, now) >= MISAFIR_HATA_SINIRI) {
            return Err(MisafirRed::Kilit);
        }
        for k in [ip_key, mac_key] {
            sy.pencere.entry(k.to_string()).or_default().push_back(now);
        }
        let n = sy.kod.entry(kod_key.to_string()).or_default();
        if *n >= misafir::KOD_HATA_TAVANI {
            return Err(MisafirRed::KodTavani);
        }
        *n += 1;
        Ok(())
    }

    /// Başarılı (ya da hata sayılmayan) denemenin ayrılan hatasını geri alır.
    fn misafir_geri_al(&self, ip_key: &str, mac_key: &str, kod_key: &str, now: f64) {
        let mut sy = lock(&self.misafir_sayac);
        for k in [ip_key, mac_key] {
            if let Some(q) = sy.pencere.get_mut(k) {
                if let Some(i) = q.iter().rposition(|&t| t == now) {
                    q.remove(i);
                }
            }
        }
        if let Some(n) = sy.kod.get_mut(kod_key) {
            *n = n.saturating_sub(1);
        }
    }

    /// Kod tavanına ulaşıldı ve kaba kuvvet kaydı henüz yazılmadı mı (her kod için bir kez true).
    fn misafir_tavan_ilk(&self, kod_key: &str) -> bool {
        let mut sy = lock(&self.misafir_sayac);
        sy.kod.get(kod_key).copied().unwrap_or(0) >= misafir::KOD_HATA_TAVANI && sy.bildirildi.insert(kod_key.to_string())
    }

    /// POST /misafir/dogrula: kimlik + kod eşleşirse kodun bitişine kadar internet. Kimlik yok / kod yanlış / süresi
    /// dolmuş / iptal: tek ileti, IP ve MAC başına hata sayılır (5 → 15 dk; kilitliyken doğru kod da reddedilir).
    /// Denetime kimlik ve kod yazılmaz (başarıda yalnızca maskeli kimlik).
    pub fn misafir_dogrula(&self, ip: &str, mac: Option<&str>, form: &Form) -> Option<Page> {
        if !crate::ayar::misafir_kodu_acik(&self.cfg) {
            return None;
        }
        let d = Self::form_dil(form);
        if let Some(pg) = self.hizmet_kapali(d) {
            return Some(pg);
        }
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let dst = safe_dst(get("dst"));
        let Some(mac) = mac else { return Some(self.error_page(dil::m(d, "mac_yok"), d)) };
        // girilen kod forma geri yazılmaz
        let mut geri = form.clone();
        geri.remove("kod");
        let now = (self.clock)();
        let (ip_key, mac_key) = (format!("misafir_ip:{ip}"), format!("misafir_mac:{mac}"));
        let kilit_sayfasi = |geri: &Form| {
            self.audit("MISAFIR_HATALI", ip, mac, "", "red=kilit");
            let msg = dil::f(d, "kilit", &[&((MISAFIR_KILIT_SN / 60.0) as u64).to_string()]);
            Some(self.misafir_form(geri, vec![("hata_genel", msg)], &dst, d))
        };
        if self.misafir_kilitli(&ip_key, &mac_key, now) {
            return kilit_sayfasi(&geri);
        }
        // biçim hataları (kodun var olup olmadığını söylemez, hata sayılmaz)
        let mut errors = vec![];
        let anahtar = match get("tur") {
            "tc" => {
                let t: String = get("tc").chars().filter(|c| !c.is_whitespace()).collect();
                if kimlik::tc_gecerli(&t) {
                    Some(kimlik::Kimlik::Tc(t).anahtar())
                } else {
                    errors.push(("hata_tc", dil::m(d, "hata_tc").to_string()));
                    None
                }
            }
            "pasaport" => {
                let no = kimlik::pasaport_normal(get("pasaport"));
                let uyruk = get("uyruk");
                let uyruk_ok = ulkeler::ULKELER.iter().any(|(i, ..)| *i == uyruk);
                if no.is_none() {
                    errors.push(("hata_pasaport", dil::m(d, "hata_pasaport").to_string()));
                }
                if !uyruk_ok {
                    errors.push(("hata_uyruk", dil::m(d, "hata_uyruk").to_string()));
                }
                no.filter(|_| uyruk_ok).map(|no| kimlik::Kimlik::Pasaport { uyruk: uyruk.to_string(), no }.anahtar())
            }
            _ => {
                errors.push(("hata_tur", dil::m(d, "hata_kimlik_turu").to_string()));
                None
            }
        };
        let kod: String = get("kod").chars().filter(|c| !c.is_whitespace()).collect();
        if kod.len() != 6 || !kod.bytes().all(|b| b.is_ascii_digit()) {
            errors.push(("hata_kod", dil::m(d, "hata_misafir_kod").to_string()));
        }
        if get("riza") != "1" {
            errors.push(("hata_riza", dil::m(d, "hata_riza").to_string()));
        }
        if get("sozlesme") != "1" {
            errors.push(("hata_sozlesme", dil::m(d, "hata_sozlesme").to_string()));
        }
        let (Some(anahtar), true) = (anahtar, errors.is_empty()) else {
            return Some(self.misafir_form(&geri, errors, &dst, d));
        };
        let wall = (self.wall)();
        let kodlar = misafir::oku(&self.cfg.main.state_root);
        // kimliğin etkin kodu (varsa): kod başına hata tavanı bu koda sayılır; cevaba hiçbir şey yansımaz
        let etkin = kodlar.iter().find(|k| k.aktif(wall) && k.kimlik == anahtar);
        let kod_key = etkin.map_or_else(|| format!("kimlik:{anahtar}"), |k| format!("kod:{}", k.kod_id));
        let eslesmedi = || Some(self.misafir_form(&geri, vec![("hata_genel", dil::m(d, "misafir_eslesmedi").into())], &dst, d));
        match self.misafir_ayir(&ip_key, &mac_key, &kod_key, now) {
            Ok(()) => {}
            Err(MisafirRed::Kilit) => return kilit_sayfasi(&geri),
            Err(MisafirRed::KodTavani) => {
                self.audit("MISAFIR_HATALI", ip, mac, "", "red=kod_tavani");
                return eslesmedi();
            }
        }
        let hata = |ek: &str| {
            // ayrılan hata kalır; tavana bu denemeyle ulaşıldıysa bir kez kaba kuvvet kaydı
            self.audit("MISAFIR_HATALI", ip, mac, "", ek);
            if self.misafir_tavan_ilk(&kod_key) {
                let oda = etkin.map_or(String::new(), |k| format!("oda={}", k.oda));
                self.audit("MISAFIR_KABA_KUVVET", ip, mac, &kimlik::maskele(&anahtar), &oda);
            }
            eslesmedi()
        };
        let Some(k) = misafir::dogrula(&kodlar, &anahtar, &kod, wall).cloned() else {
            return hata("");
        };
        let ek = format!(
            "sozlesme=1 riza=1 giris=misafir_kodu oda={} kod_bitis={KOD_BITIS} ureten={}",
            k.oda,
            k.ureten.replace(char::is_whitespace, "_")
        );
        let g = Giris { kisi: &k.kimlik, ad: &k.ad, soyad: &k.soyad, bitis: Some(k.bitis), ek, kod_id: Some(&k.kod_id) };
        let Some(s) = self.open_session(ip, mac, &g) else {
            // kod bu arada iptal edildi / süresi doldu: aynı tek ileti; değilse (nft) bağlantı hatası
            let hala = misafir::oku(&self.cfg.main.state_root).iter().any(|x| x.kod_id == k.kod_id && x.aktif((self.wall)()));
            if hala {
                self.misafir_geri_al(&ip_key, &mac_key, &kod_key, now);
                return Some(self.error_page(dil::m(d, "baglanti_acilamadi"), d));
            }
            return hata("red=kod_gecersiz");
        };
        self.misafir_geri_al(&ip_key, &mac_key, &kod_key, now);
        self.audit("MISAFIR_BASARILI", ip, mac, &kimlik::maskele(&k.kimlik), &format!("oturum={}", s.session_id));
        Some(self.success_page(&dst, d))
    }

    /// §11.7 sırası. nft başarısızsa yeni oturum yazılmaz. SMS ve misafir kodu aynı adımları kullanır; cihaz sınırı
    /// kişi anahtarı (telefon ya da `TC:…`/`PASAPORT:…`) başına.
    fn open_session(&self, ip: &str, mac: &str, g: &Giris) -> Option<Session> {
        let m = &self.cfg.main;
        let runner = &*self.runner;
        let _g = ortak::state_lock(&m.state_root).ok()?;
        let mut ses = ortak::load_sessions(&m.state_root);
        let now = (self.wall)();
        let mut bitis = g.bitis;
        if let Some(id) = g.kod_id {
            // panelin iptal/uzatması da state_lock altında: kod şimdi hâlâ etkin mi
            bitis = Some(misafir::oku(&m.state_root).iter().find(|k| k.kod_id == id && k.aktif(now))?.bitis);
        }
        let secs = bitis.map_or((m.session_minutes * 60) as f64, |b| b - now);
        if secs < 1.0 {
            return None; // kodun süresi bu arada doldu
        }
        if ses.contains_key(mac) {
            ortak::close_session(&m.log_root, &mut ses, mac, "yeniden_giris", now, runner);
        }
        let mut same: Vec<(f64, String)> =
            ses.iter().filter(|(_, s)| s.phone == g.kisi).map(|(k, s)| (s.start_epoch, k.clone())).collect();
        same.sort_by(|a, b| a.0.total_cmp(&b.0));
        while !same.is_empty() && same.len() >= m.max_devices_per_phone {
            let (_, old) = same.remove(0);
            ortak::close_session(&m.log_root, &mut ses, &old, "cihaz_limiti", now, runner);
        }
        if !ortak::nft_add_cmd(mac, ip, secs).is_some_and(|c| runner(&c)) {
            let _ = ortak::save_sessions(&m.state_root, &ses);
            return None;
        }
        ortak::park_ip_holders(&m.log_root, &mut ses, mac, ip, now, runner);
        let zaman = ortak::now_iso(now);
        let s = Session {
            phone: g.kisi.into(),
            ad: g.ad.into(),
            soyad: g.soyad.into(),
            ip: ip.into(),
            session_id: ortak::random_hex(6),
            start: zaman.clone(),
            start_epoch: now,
            expires_epoch: now + secs,
        };
        ses.insert(mac.into(), s.clone());
        if let Err(e) = ortak::save_sessions(&m.state_root, &ses) {
            eprintln!("oturum dosyası yazılamadı: {e}");
        }
        let basla = Row::new("OTURUM_BASLA", &zaman)
            .set("telefon", &s.phone)
            .set("ad", &s.ad)
            .set("soyad", &s.soyad)
            .set("mac", mac)
            .set("ic_ip", ip)
            .set("oturum_id", &s.session_id)
            // kod_bitis: state_lock altında yeniden okunan kodun bitişi (bu arada uzatılmış olabilir)
            .set("ek", format!("bitis={} {}", ortak::now_iso(now + secs), g.ek.replace(KOD_BITIS, &ortak::now_iso(now + secs))));
        let day = ortak::day_of(&zaman).to_string();
        let written = ortak::append_user_rows(&m.log_root, &s, mac, &[basla.clone()])
            .and_then(|_| ortak::append_rows(&ortak::day_file(&m.log_root, &day, "oturum.csv"), &[basla]))
            .and_then(|_| ortak::upsert_index(&m.log_root, &s.phone, &s.ad, &s.soyad, &zaman));
        if let Err(e) = written {
            eprintln!("oturum kaydı yazılamadı: {e}");
        }
        Some(s)
    }

    // --- internet yokken uyarı ve Servis (docs/superpowers/specs/2026-10-23-pppoe-design.md): teknisyen yerinde, internet ve SMS
    // olmadan, müşteriye özel AP yönetim parolasıyla PPPoE bilgilerini girer. Yalnızca varsayılan rota yokken; varken /servis
    // giriş sayfasına yönlenir (misafire açık sayfada internet varken hiçbir ayar yüzeyi yok).

    /// Cihazın interneti yoksa internet alan portun durumu (`None`: internet var). En çok `INTERNET_ONBELLEK_SN` önbellekte.
    fn internet_yok(&self) -> Option<Durum> {
        let taze = |now: f64| lock(&self.internet_onbellek).as_ref().filter(|(t, _)| now - t < INTERNET_ONBELLEK_SN).map(|(_, d)| d.clone());
        if let Some(d) = taze((self.clock)()) {
            return d;
        }
        // tek yoklama: aynı anda gelen istekler bekleyip sonucu paylaşır (komutlar her istekte yeniden çalışmasın)
        let _yoklama = lock(&self.internet_yoklama);
        let now = (self.clock)();
        if let Some(d) = taze(now) {
            return d;
        }
        let d = (self.internet)(&self.ag_yollar.ag);
        *lock(&self.internet_onbellek) = Some((now, d.clone()));
        d
    }

    /// Servis parolası = müşteriye özel AP yönetim parolası (`merkez.json` → `ap_parola`; merkeze sorulmaz). Yoksa (cihaz
    /// bağlanmamış ya da eski merkez) Servis kapalı; merkezin kuralından (8-64 karakter) kısa bir değer de parola sayılmaz.
    fn servis_parolasi(&self) -> Option<String> {
        crate::merkez::oku(&self.merkez_path).map(|m| m.ap_parola).filter(|p| p.len() >= 8)
    }

    /// Servis sayfası (yalnızca Türkçe, JS yok). `govde_html`: form ya da bağlantı; `yenile`: birkaç saniyede bir izleme sayfasına.
    fn servis_page(&self, durum: Option<&Durum>, hata: &str, bilgi: &str, govde_html: String, yenile: bool) -> Page {
        let mut ctx = vec![
            ("durum", durum.map_or_else(String::new, |d| format!("Durum: {}", d.metin()))),
            ("hata", hata.to_string()),
            ("bilgi", bilgi.to_string()),
            ("govde_html", govde_html),
        ];
        if yenile {
            let hedef = format!("{}/servis?izle=1", html_escape(&self.base_url));
            ctx.push(("bas_html", format!("<meta http-equiv=\"refresh\" content=\"{SERVIS_YENILE_SN};url={hedef}\">")));
        }
        Page::new("servis", ctx, Dil::Tr)
    }

    /// Tek form: Servis parolası + panelle aynı PPPoE alanları (`pppoe::servis_form_html`: değerler kaçışlı, parola hiç yazılmaz,
    /// kayıtlı kullanıcı adı hiç yazılmaz). `alanlar`: VLAN ve ileri alanların değerleri; `kullanici`: yalnızca bu istekte yazılan ad.
    fn servis_formu(&self, alanlar: &ag::Pppoe, kullanici: &str) -> String {
        format!(
            "<form class=\"servis\" method=\"post\" action=\"{}/servis\">\
             <label for=\"servis_parola\">Servis parolası (AP yönetim parolası)</label>\
             <input type=\"password\" id=\"servis_parola\" name=\"servis_parola\" value=\"\" maxlength=\"64\" autocomplete=\"off\" required>\
             {}<button type=\"submit\">Kaydet ve bağlan</button></form>\
             <p class=\"not\">Servis parolası her kayıtta yeniden yazılır. Kaydedince ayar uygulanır (PPPoE Açık ise yeniden bağlanır, \
             Kapalı ise durur) ve sayfa birkaç saniyede bir yenilenip durumu gösterir; internet gelince giriş sayfası açılır.</p>",
            html_escape(&self.base_url),
            pppoe::servis_form_html(alanlar, kullanici)
        )
    }

    /// Hatayla form yeniden: girilen değerler geri gelir (parolalar hariç; kullanıcı adı yalnızca bu istekte yazılan, kayıtlı ad
    /// hiçbir zaman: parola doğrulandıktan sonraki hatalarda da). PPPoE seçimi de gönderilen seçimdir: "Kapalı" seçilmişse Kapalı
    /// kalır. Hiçbir şey gönderilmediyse (eksik form) form kayıtlı ayardan kurulur, yani ilk çizim gibi (`servis_ilk_alanlar`).
    /// Parola ipucu kayıtlı parolaya göre (boş = aynı kalır).
    fn servis_hata(&self, durum: &Durum, hata: &str, girilen: Option<&ag::Pppoe>, kayitli: &ag::Pppoe) -> Page {
        let alanlar = match girilen {
            Some(g) => ag::Pppoe { parola: kayitli.parola.clone(), ..g.clone() },
            None => servis_ilk_alanlar(kayitli),
        };
        let kullanici = girilen.map_or("", |g| g.kullanici.as_str());
        self.servis_page(Some(durum), hata, "", self.servis_formu(&alanlar, kullanici), false)
    }

    /// İzleme sayfası: durum + ayarlara dönüş bağlantısı; birkaç saniyede bir kendini yeniler (internet gelince giriş sayfasına).
    fn servis_izle(&self, durum: Option<&Durum>, bilgi: &str) -> Page {
        let govde = format!("<p><a href=\"{}/servis\">Ayarları değiştir</a></p>", html_escape(&self.base_url));
        self.servis_page(durum, "", bilgi, govde, true)
    }

    /// GET /servis (`izle`: yalnızca durum, birkaç saniyede bir yenilenir). `None`: internet var (HTTP katmanı giriş sayfasına
    /// yönlendirir).
    pub fn servis_sayfasi(&self, izle: bool) -> Option<Page> {
        let durum = self.internet_yok()?;
        if izle {
            return Some(self.servis_izle(Some(&durum), "Bu sayfa birkaç saniyede bir yenilenir; internet gelince giriş sayfası açılır."));
        }
        Some(match self.servis_parolasi() {
            // VLAN ve ileri alanlar kayıtlı değerlerle dolu ("kapalı" gönderilince silinmesin) ama PPPoE seçimi Açık gelir
            // (`servis_ilk_alanlar`); kayıtlı kullanıcı adı (çoğu kez abone telefon numarası) parola doğrulanmadan gösterilmez:
            // alan boş, boş gönderilirse kayıtlı ad kalır
            Some(_) => self.servis_page(Some(&durum), "", "", self.servis_formu(&servis_ilk_alanlar(&ag::load(&self.ag_yollar.ag).pppoe), ""), false),
            None => self.servis_page(Some(&durum), SERVIS_TANIMSIZ, "", String::new(), false),
        })
    }

    /// POST /servis. `None`: internet var (HTTP katmanı giriş sayfasına yönlendirir; form işlenmez, sayılmaz). Sıra: Servis parolası
    /// tanımlı mı → kilit (kilitliyken parolaya bakılmaz; denetime kilit dönemi başına bir satır) → form tam mı (eksikse ayar formdan
    /// okunmaz, sayılmaz) → parola (yanlışsa sayılır) → boş kullanıcı adına kayıtlı ad → ayarın doğrulaması (doğru paroladan sonraki
    /// hata sayılmaz) → hazırlık dosyası + `ctl pppoe-servis` işi. Kayıtlı kullanıcı adı hiçbir cevapta gösterilmez. Denetime yalnızca
    /// özet yazılır (MAC, IP; parolalar ve kullanıcı adı yazılmaz).
    pub fn servis_kaydet(&self, ip: &str, mac: Option<&str>, form: &Form) -> Option<Page> {
        let durum = self.internet_yok()?;
        let Some(ap_parola) = self.servis_parolasi() else {
            return Some(self.servis_page(Some(&durum), SERVIS_TANIMSIZ, "", String::new(), false));
        };
        let get = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
        let kayitli = ag::load(&self.ag_yollar.ag).pppoe;
        // panelle ortak ayrıştırma; yalnızca form tamsa (eksik bir form kayıtlı ayarı varsayılanlara döndürmesin)
        let girilen = form.contains_key("pppoe_acik").then(|| pppoe::formdan(&kayitli, &|k: &str| get(k).to_string()));
        let mac_s = mac.unwrap_or("");
        // MAC başına; MAC bilinmiyorsa IP başına
        let anahtar = mac.map_or_else(|| format!("ip:{ip}"), |m| format!("mac:{m}"));
        let now = (self.clock)();
        let kilitli = |dakika: u64, ilk: bool, g: Option<&ag::Pppoe>| {
            // kilit dönemi başına bir satır (kilit sürerken gelen her istek ayrı satır olmasın)
            if ilk {
                self.audit("SERVIS_GIRIS", ip, mac_s, "", "sonuc=kilit");
            }
            Some(self.servis_hata(&durum, &servis_kilit_metni(dakika), g, &kayitli))
        };
        // kilitliyken parolaya bakılmaz (eksik form da kilit olarak söylenir)
        let kilit = lock(&self.servis_sayac).kilitli_istek(&anahtar, now);
        if let Some((dakika, ilk)) = kilit {
            return kilitli(dakika, ilk, girilen.as_ref());
        }
        // `girilen`: yalnızca bu istekte yazılanlar (hata cevaplarında forma bunlar döner, kayıtlı kullanıcı adı hiçbir zaman)
        let Some(girilen) = girilen else {
            return Some(self.servis_hata(&durum, "Form eksik gönderildi; sayfayı yenileyip yeniden deneyin.", None, &kayitli));
        };
        let parola = get("servis_parola");
        if parola.is_empty() {
            return Some(self.servis_hata(&durum, "Servis parolasını girin.", Some(&girilen), &kayitli));
        }
        // kilit yeniden denetlenir; karşılaştırma ve sayma aynı kilit altında: aynı anda gelen denemeler sınırı aşamaz
        enum Sonuc {
            Dogru,
            Yanlis(Option<u64>),
            Kilitli(u64, bool),
        }
        let sonuc = {
            let mut s = lock(&self.servis_sayac);
            match s.kilitli_istek(&anahtar, now) {
                Some((dakika, ilk)) => Sonuc::Kilitli(dakika, ilk),
                None if servis_parolasi_dogru(parola, &ap_parola) => {
                    s.dogru(&anahtar);
                    Sonuc::Dogru
                }
                None => {
                    s.hata(&anahtar, now);
                    Sonuc::Yanlis(s.kilit(&anahtar, now))
                }
            }
        };
        match sonuc {
            Sonuc::Dogru => {}
            Sonuc::Kilitli(dakika, ilk) => return kilitli(dakika, ilk, Some(&girilen)),
            Sonuc::Yanlis(kilit) => {
                self.audit("SERVIS_GIRIS", ip, mac_s, "", "sonuc=hatali");
                let hata = kilit.map_or_else(|| "Servis parolası yanlış.".to_string(), servis_kilit_metni);
                return Some(self.servis_hata(&durum, &hata, Some(&girilen), &kayitli));
            }
        }
        // boş kullanıcı adı = kayıtlı ad: sayfada hiç gösterilmediği için parola doğrulandıktan sonra burada birleştirilir (ctl
        // birleştirmez; açık/kapalı geçişi adı silmesin). Boş PPPoE parolası = kayıtlı parola (`formdan` birleştirdi). Doğrulama
        // `ctl pppoe-servis`'teki gibi birleşik ayarla.
        let mut yeni = girilen.clone();
        if yeni.kullanici.is_empty() {
            yeni.kullanici = kayitli.kullanici.clone();
        }
        if let Err(e) = ag::pppoe_dogrula(&yeni) {
            return Some(self.servis_hata(&durum, &e, Some(&girilen), &kayitli));
        }
        // `ctl pppoe-servis`'in reddedeceği durumlar burada söylenir (işin sonucu sayfaya dönmez): ag.toml yok ya da bozuk
        // (`ag::load_kesin`; hangi portun internet aldığı bilinmeden PPPoE konmaz), port değişikliği onay bekliyor
        let dosya_hatasi = match ag::load_kesin(&self.ag_yollar.ag) {
            Ok(_) => None,
            Err(ag::AgDosyasi::Yok) => Some("Ağ ayar dosyası yok; PPPoE ayarı uygulanamaz."),
            Err(ag::AgDosyasi::Bozuk) => Some("Ağ ayar dosyası okunamadı (bozuk); PPPoE ayarı uygulanamaz."),
        };
        if let Some(hata) = dosya_hatasi {
            return Some(self.servis_hata(&durum, hata, Some(&girilen), &kayitli));
        }
        if ag::pending(&self.ag_yollar).is_some() {
            let hata = "Panelden yapılan bir port değişikliği onay bekliyor (en çok 3 dakika); süre dolunca yeniden deneyin.";
            return Some(self.servis_hata(&durum, hata, Some(&girilen), &kayitli));
        }
        let ozet = format!(
            "pppoe={} vlan={} kullanici_degisti={} parola_degisti={}",
            if yeni.acik { "acik" } else { "kapali" },
            yeni.vlan,
            u8::from(yeni.kullanici != kayitli.kullanici),
            u8::from(yeni.parola != kayitli.parola)
        );
        // kayıtlı parola hazırlık dosyasına kopyalanmaz: alan boşsa boş gider, `ctl pppoe-servis` (ag::pppoe_uygula) birleştirir;
        // kullanıcı adı yukarıda birleştirildi (dosyada kayıtlı ad)
        let mut hazir = yeni.clone();
        if get("pppoe_parola").is_empty() {
            hazir.parola.clear();
        }
        if let Err(kod) = self.servis_baslat(&hazir) {
            self.audit("SERVIS_PPPOE", ip, mac_s, "", &format!("{ozet} hata={kod}"));
            let hata = if kod == "baslatilamadi" {
                "Ayar uygulanamadı: iş başlatılamadı; birkaç saniye sonra yeniden deneyin."
            } else {
                "Ayar kaydedilemedi; yeniden deneyin."
            };
            return Some(self.servis_hata(&durum, hata, Some(&girilen), &kayitli));
        }
        self.audit("SERVIS_PPPOE", ip, mac_s, "", &ozet);
        // yeni durum hemen yoklansın: internet geldiyse izleme sayfası giriş sayfasına döner
        *lock(&self.internet_onbellek) = None;
        // mesaj kaydedilen ayarı söyler (kullanıcı adı ve parola yazılmaz): PPPoE kapalı kaydedildiyse "yeniden bağlanıyor" demez
        let mesaj = if yeni.acik { SERVIS_KAYIT_ACIK } else { SERVIS_KAYIT_KAPALI };
        Some(self.servis_izle(None, mesaj))
    }

    /// Hazırlık dosyası (600; ag.toml'daki adlarla `[pppoe]` tablosu) + `ctl pppoe-servis` ayrı bir işte: portal ProtectSystem=full
    /// ile çalışır, /etc'ye yazamaz. İş beklenmez (sonuç durum sayfasında görünür); geri alma zamanlayıcısı yok (spec). İş
    /// başlatılamazsa dosya (parola içerebilir) silinir. Hata: denetim kodu (`yazilamadi` | `baslatilamadi`); ayrıntı (dosya
    /// içeriğini taşıyabilir) yazılmaz.
    fn servis_baslat(&self, p: &ag::Pppoe) -> Result<(), &'static str> {
        #[derive(serde::Serialize)]
        struct Hazir<'a> {
            pppoe: &'a ag::Pppoe,
        }
        let _sira = lock(&self.servis_sira);
        let dosya = self.ag_yollar.durum.join(SERVIS_DOSYASI);
        let metin = toml::to_string(&Hazir { pppoe: p }).map_err(|_| "yazilamadi")?;
        ortak::write_atomic(&dosya, metin.as_bytes()).map_err(|_| "yazilamadi")?;
        let komut: Vec<String> = ["systemd-run", "--collect", "--unit", SERVIS_BIRIMI, "/usr/local/bin/wificorrect", "ctl", "pppoe-servis"]
            .iter()
            .map(|s| s.to_string())
            .chain([dosya.to_string_lossy().into_owned()])
            .collect();
        if !(self.runner)(&komut) {
            let _ = std::fs::remove_file(&dosya);
            return Err("baslatilamadi");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- HTTP
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` → ilk değer kazanır; boş değerler atlanır (Python parse_qs gibi).
pub fn parse_query(s: &str) -> Form {
    let mut out = Form::new();
    for pair in s.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (pct_decode(k), pct_decode(v));
        if !k.is_empty() && !v.is_empty() {
            out.entry(k).or_insert(v);
        }
    }
    out
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn respond(req: tiny_http::Request, code: u16, body: String, ctype: &str, extra: Vec<(&'static str, String)>, base_url: &str) {
    let csp = format!("default-src 'none'; style-src 'unsafe-inline'; img-src data:; form-action {base_url}");
    let mut headers = vec![
        ("Content-Type", ctype.to_string()),
        ("Cache-Control", "no-store".into()),
        ("X-Frame-Options", "DENY".into()),
        ("Content-Security-Policy", csp),
        ("Server", "hotspot".into()),
    ];
    headers.extend(extra);
    let mut resp = tiny_http::Response::from_string(body).with_status_code(code);
    for (k, v) in headers {
        if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(h);
        }
    }
    let _ = req.respond(resp);
}

fn baslik(req: &tiny_http::Request, ad: &'static str) -> String {
    req.headers().iter().find(|h| h.field.equiv(ad)).map(|h| h.value.as_str().to_string()).unwrap_or_default()
}

fn handle(p: &Portal, per_ip: &RateLimiter, mut req: tiny_http::Request) {
    let base = p.base_url.clone();
    let text = |req, code, msg: &str| respond(req, code, msg.to_string(), "text/plain; charset=utf-8", vec![], &base);
    let git = |req, to: String| respond(req, 302, String::new(), "text/html; charset=utf-8", vec![("Location", to)], &base);
    let ip = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    if !ortak::in_subnet(&ip, &p.cfg.main.subnet) {
        return text(req, 403, "Yasak");
    }
    let url = req.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let q = parse_query(query);
    // dil: sorgu (GET) ya da form alanı (POST) > çerez > Accept-Language > Türkçe
    let (cerez, al) = (baslik(&req, "Cookie"), baslik(&req, "Accept-Language"));
    let dil_sec = |acik: Option<&str>| dil::sec(acik, dil::cerezden(&cerez), Some(al.as_str()).filter(|a| !a.is_empty()));
    let d = dil_sec(q.get("dil").map(String::as_str));
    let now = ortak::monotonic();
    if per_ip.count(&ip, 60.0, now) >= 30 {
        return text(req, 429, dil::m(d, "cok_istek"));
    }
    per_ip.hit(&ip, now);
    let host = baslik(&req, "Host");
    if host != p.host {
        // nft'nin 80 → 8080 yönlendirdiği yabancı istek: portala gönder, gitmek istediği adresi hatırla
        let dst = if host.is_empty() { String::new() } else { safe_dst(&format!("http://{host}{url}")) };
        let q = if dst.is_empty() { String::new() } else { format!("?dst={}", pct_encode(&dst)) };
        return git(req, format!("{base}/{q}"));
    }
    let mac = ortak::lookup_mac(&ip, &p.cfg.main.leases_file);
    let mac = mac.as_deref();
    let method = req.method().clone();
    let mut extra = vec![];
    let page = match method {
        tiny_http::Method::Get | tiny_http::Method::Head => match path {
            "/" => {
                if q.get("dil").and_then(|v| dil::Dil::coz(v)).is_some() {
                    extra.push(("Set-Cookie", dil::cerez_basligi(d))); // seçim hatırlansın
                }
                let dst = safe_dst(q.get("dst").map(String::as_str).unwrap_or(""));
                p.home_dil(&ip, mac, &dst, d, q.get("sayfa").is_some_and(|s| s == "kod"))
            }
            "/misafir" => {
                if q.get("dil").and_then(|v| dil::Dil::coz(v)).is_some() {
                    extra.push(("Set-Cookie", dil::cerez_basligi(d)));
                }
                let dst = safe_dst(q.get("dst").map(String::as_str).unwrap_or(""));
                match p.misafir_sayfasi(&ip, mac, &dst, d) {
                    Some(pg) => pg,
                    None => return git(req, format!("{base}/")),
                }
            }
            // yalnızca internet yokken; varken giriş sayfasına
            "/servis" => match p.servis_sayfasi(q.get("izle").is_some_and(|v| v == "1")) {
                Some(pg) => pg,
                None => return git(req, format!("{base}/")),
            },
            _ => return git(req, format!("{base}/")),
        },
        tiny_http::Method::Post => {
            if req.body_length().unwrap_or(0) > 4096 {
                return text(req, 413, dil::m(d, "istek_buyuk"));
            }
            let mut raw = String::new();
            if req.as_reader().take(4096).read_to_string(&mut raw).is_err() {
                return text(req, 400, dil::m(d, "istek_gecersiz"));
            }
            let mut form = parse_query(&raw);
            let d = dil_sec(form.get("dil").map(String::as_str));
            form.insert("dil".into(), d.kod().into());
            match path {
                "/kod-gonder" => p.send_code(&ip, mac, &form),
                "/dogrula" => p.verify(&ip, mac, &form),
                "/tekrar-gonder" => p.resend_dil(&ip, mac, d),
                "/misafir/dogrula" => match p.misafir_dogrula(&ip, mac, &form) {
                    Some(pg) => pg,
                    None => return git(req, format!("{base}/")),
                },
                // yalnızca internet yokken; varken form işlenmeden giriş sayfasına
                "/servis" => match p.servis_kaydet(&ip, mac, &form) {
                    Some(pg) => pg,
                    None => return git(req, format!("{base}/")),
                },
                _ => return git(req, format!("{base}/")),
            }
        }
        _ => return text(req, 405, dil::m(d, "istek_desteklenmez")),
    };
    let html = p.render(&page);
    respond(req, 200, html, "text/html; charset=utf-8", extra, &base);
}

pub fn run(cfg: Config) -> ExitCode {
    let addr = format!("{}:{}", cfg.main.router_ip, cfg.main.portal_port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("portal: {addr} dinlenemedi: {e}");
            return ExitCode::from(1);
        }
    };
    // SMS çağrıları portalın kendi merkez bağı yolunu kullanır (`Portal::merkez_path` ile aynı)
    let (mp1, mp2) = (std::path::PathBuf::from(crate::merkez::PATH), std::path::PathBuf::from(crate::merkez::PATH));
    let portal = Arc::new(Portal::new(
        cfg,
        Box::new(ortak::monotonic),
        Box::new(ortak::wall),
        Box::new(|c: &[String]| ortak::run(c)),
        Box::new(move |phone: &str, code: &str| sms::send(&mp1, phone, code)),
        Box::new(move |phone: &str, code: &str| sms::check(&mp2, phone, code)),
    ));
    portal.audit("SERVIS_BASLADI", "", "", "", "portal");
    eprintln!("portal: {addr} dinleniyor (SMS deneme modu: {})", if portal.cfg.sms.mock { "açık" } else { "kapalı" });
    let per_ip = Arc::new(RateLimiter::default());
    // ponytail: 8 işçi + gövde ≤ 4 KB; yavaş istemci bir işçiyi en çok gövde okuma süresince tutar. Kafe ölçeğinde yeterli.
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let (server, portal, per_ip) = (server.clone(), portal.clone(), per_ip.clone());
            std::thread::spawn(move || loop {
                match server.recv() {
                    Ok(req) => {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&portal, &per_ip, req)));
                        if r.is_err() {
                            eprintln!("portal: istek işlenirken beklenmeyen hata");
                        }
                    }
                    Err(e) => {
                        eprintln!("portal: bağlantı hatası: {e}");
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct T {
        p: Portal,
        mono: Arc<Mutex<f64>>,
        wall: Arc<Mutex<f64>>,
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        sms: Arc<Mutex<Vec<(String, String, &'static str)>>>,
        /// sahte merkezin cevabı (boş: başarı, deneme modu)
        cevap: Arc<Mutex<Option<Sonuc>>>,
        /// sahte merkezin Twilio doğrulaması: çağrılan kodlar, onaylanan kod, ağ hatası
        kontrol: Arc<Mutex<Vec<String>>>,
        onayli: Arc<Mutex<String>>,
        ag: Arc<Mutex<bool>>,
        root: std::path::PathBuf,
        /// sahte durum sağlayıcı: `None` internet var (varsayılan), `Some(durum)` yok
        internet: Arc<Mutex<Option<Durum>>>,
        /// sağlayıcı kaç kez çağrıldı (önbellek)
        yoklama: Arc<AtomicUsize>,
    }

    const MAC: &str = "aa:bb:cc:dd:ee:01";
    const IP: &str = "10.50.0.23";
    const WALL0: f64 = 1_790_705_134.0; // 2026-09-29 21:05:34 +03

    fn setup(tweak: impl FnOnce(&mut Config)) -> T {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-portal-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        let mut cfg = Config::default();
        cfg.main.site_name = "Bocafe".into();
        cfg.main.log_root = root.join("5651").to_string_lossy().into();
        cfg.main.state_root = root.join("state").to_string_lossy().into();
        tweak(&mut cfg);
        let (mono, wall) = (Arc::new(Mutex::new(100.0)), Arc::new(Mutex::new(WALL0)));
        let calls = Arc::new(Mutex::new(vec![]));
        let sms_log = Arc::new(Mutex::new(vec![]));
        let cevap: Arc<Mutex<Option<Sonuc>>> = Arc::new(Mutex::new(None));
        let (kontrol, onayli, ag) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new("424242".to_string())), Arc::new(Mutex::new(false)));
        let (kc, on2, ag2) = (kontrol.clone(), onayli.clone(), ag.clone());
        let (m2, w2, c2, s2, k2) = (mono.clone(), wall.clone(), calls.clone(), sms_log.clone(), cevap.clone());
        let p = Portal::new(
            cfg,
            Box::new(move || *m2.lock().unwrap()),
            Box::new(move || *w2.lock().unwrap()),
            Box::new(move |c: &[String]| {
                c2.lock().unwrap().push(c.to_vec());
                !c.iter().any(|a| a.contains("10.50.0.99")) // .99'a nft ekleme başarısız sayılır
            }),
            // sahte merkez: `cevap` boşsa başarı (deneme modu; sağlayıcı Türk numarasına NetGSM, diğerlerine Twilio)
            Box::new(move |phone: &str, code: &str| {
                let r = k2.lock().unwrap().clone().unwrap_or_else(|| Sonuc {
                    ok: true,
                    kod: "0".into(),
                    is: Some("J1".into()),
                    saglayici: if phone.starts_with("90") { Saglayici::Netgsm } else { Saglayici::Twilio },
                    mock: true,
                });
                s2.lock().unwrap().push((phone.into(), code.into(), r.saglayici.ad()));
                r
            }),
            // Twilio doğrulaması (merkezden): `ag` açıksa ağ hatası, değilse yalnızca `onayli` kod onaylanır
            Box::new(move |_phone: &str, code: &str| {
                kc.lock().unwrap().push(code.to_string());
                if *ag2.lock().unwrap() {
                    (false, Some("AG".into()))
                } else {
                    (code == *on2.lock().unwrap(), None)
                }
            }),
        );
        let mut p = p;
        p.merkez_path = root.join("merkez.json");
        std::fs::create_dir_all(&root).unwrap();
        crate::merkez::kaydet(&p.merkez_path, &crate::merkez::Merkez { numara: "4511643".into(), ..Default::default() }).unwrap();
        // internet durumu: testler gerçek `ip` / `journalctl` çalıştırmaz; ag.toml ve hazırlık dosyası geçici klasörde
        let (internet, yoklama): (Arc<Mutex<Option<Durum>>>, Arc<AtomicUsize>) = Default::default();
        let (i2, y2) = (internet.clone(), yoklama.clone());
        p.internet = Box::new(move |_: &std::path::Path| {
            y2.fetch_add(1, Ordering::SeqCst);
            i2.lock().unwrap().clone()
        });
        p.ag_yollar = crate::ag::Yollar {
            ag: root.join("etc/ag.toml"),
            interfaces: root.join("etc/interfaces"),
            nft: root.join("etc/arayuzler.nft"),
            hostapd: root.join("etc/hostapd.conf"),
            issue: root.join("etc/issue"),
            peers: root.join("etc/peers"),
            durum: root.join("var"),
            hiz_uygulanan: root.join("var/hiz-uygulanan"),
            uzak_key: root.join("etc/wg.key"),
            uzak_conf: root.join("etc/wfc.conf"),
        };
        T { p, mono, wall, calls, sms: sms_log, cevap, kontrol, onayli, ag, root, internet, yoklama }
    }

    impl T {
        fn advance(&self, secs: f64) {
            *self.mono.lock().unwrap() += secs;
            *self.wall.lock().unwrap() += secs;
        }
        fn last_code(&self) -> String {
            self.sms.lock().unwrap().last().unwrap().1.clone()
        }
        fn sms_count(&self) -> usize {
            self.sms.lock().unwrap().len()
        }
        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.root.join(rel)).unwrap_or_default()
        }
        fn audit_text(&self) -> String {
            self.read("5651/gunluk/2026-09-29/denetim.csv")
        }
    }

    fn form(pairs: &[(&str, &str)]) -> Form {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn good(phone: &str) -> Form {
        form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", phone), ("sozlesme", "1"), ("riza", "1"), ("dst", "http://neverssl.com/")])
    }

    #[test]
    fn form_errors_send_nothing() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "A"), ("soyad", "Ali1"), ("telefon", "212 455 31 32")]));
        assert_eq!(pg.tpl, "giris");
        for k in ["hata_ad", "hata_soyad", "hata_telefon", "hata_sozlesme"] {
            assert!(!pg.get(k).is_empty(), "{k}");
        }
        assert_eq!(pg.get("telefon"), "212 455 31 32"); // girilen değer korunur
        assert_eq!(t.sms_count(), 0);
        assert_eq!(t.p.send_code(IP, None, &good("5334553132")).tpl, "hata"); // MAC bilinmiyor
    }

    #[test]
    fn full_flow_opens_session_and_writes_records() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &good("0533 455 31 32"));
        assert_eq!(pg.tpl, "kod");
        assert_eq!(pg.get("telefon_maskeli"), "+90 5XX XXX XX 32");
        assert_eq!(t.sms.lock().unwrap()[0].0, "905334553132");
        assert_eq!(t.sms.lock().unwrap()[0].2, "netgsm");
        let code = t.last_code();
        assert_eq!(code.len(), 6);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)]));
        assert_eq!((pg.tpl, pg.get("hata")), ("kod", "Kod hatalı. Kalan deneme hakkı: 4"));
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", &format!(" {code} "))]));
        assert_eq!(pg.tpl, "basarili");
        assert!(pg.get("bas_html").contains("url=http://neverssl.com/"));
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout 2592000s }}")));
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses[MAC].ip, IP);
        assert_eq!(ses[MAC].phone, "905334553132");
        let user = t.read("5651/gunluk/2026-09-29/kullanicilar/905334553132.csv");
        assert!(user.starts_with('\u{feff}'));
        assert!(user.contains(";KAYIT;905334553132;Ayşe;Yılmaz;aa:bb:cc:dd:ee:01;10.50.0.23;"));
        assert!(user.contains(";OTURUM_BASLA;"));
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("OTURUM_BASLA"));
        assert!(t.read("5651/kullanicilar/index.csv").contains("905334553132;Ayşe;Yılmaz;2026-09-29T21:05:34+03:00"));
        let a = t.audit_text();
        for olay in ["OTP_GONDERILDI", "OTP_HATALI_KOD", "OTP_BASARILI"] {
            assert!(a.contains(olay), "{olay}");
        }
        assert!(!a.contains(&code), "kod kayda yazılmaz");
        assert_eq!(t.p.home(IP, Some(MAC), "").tpl, "basarili"); // zaten bağlı
    }

    #[test]
    fn double_submit_does_not_resend() {
        let t = setup(|_| {});
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        t.advance(5.0);
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert_eq!(pg.tpl, "kod");
        assert!(pg.get("not").starts_with("Kod az önce gönderildi"));
        assert_eq!(t.sms_count(), 1);
        assert!(t.p.resend(IP, Some(MAC)).get("not").starts_with("Kod az önce gönderildi"));
        t.advance(61.0);
        assert_eq!(t.p.resend(IP, Some(MAC)).tpl, "kod");
        assert_eq!(t.sms_count(), 2);
    }

    #[test]
    fn five_wrong_codes_cancel_and_expiry() {
        let t = setup(|_| {});
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        let wrong = if t.last_code() == "000000" { "111111" } else { "000000" };
        for _ in 0..4 {
            assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)])).tpl, "kod");
        }
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)]));
        assert_eq!((pg.tpl, pg.get("hata_genel")), ("giris", "Çok fazla hatalı deneme. Yeni kod isteyin."));
        assert_eq!(pg.get("telefon"), "+905334553132");
        assert!(t.p.verify(IP, Some(MAC), &form(&[("kod", wrong)])).get("hata_genel").contains("Kod bulunamadı"));

        t.advance(61.0);
        t.p.send_code(IP, Some(MAC), &good("5334553132"));
        let code = t.last_code();
        t.advance(181.0);
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", &code)]));
        assert_eq!(pg.get("hata_genel"), "Kodun süresi doldu. Yeni kod isteyin.");
    }

    #[test]
    fn rate_limits_and_global_cap() {
        let t = setup(|_| {});
        for i in 0..3 {
            assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod", "gönderim {i}");
            t.advance(61.0);
        }
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert!(pg.get("hata_genel").contains("15 dakika"));
        assert!(t.audit_text().contains("red=tel_15dk"));
        assert_eq!(t.sms_count(), 3);

        let t = setup(|c| c.limits.sms_global_day = 1);
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        let pg = t.p.send_code("10.50.0.24", Some("aa:bb:cc:dd:ee:02"), &good("5321112233"));
        assert_eq!(pg.get("hata_genel"), "Şu an SMS gönderilemiyor, lütfen personele başvurun.");
        assert!(t.audit_text().contains("SMS_TAVAN"));
    }

    fn login(t: &T, mac: &str, ip: &str, phone: &str) -> Page {
        t.p.send_code(ip, Some(mac), &good(phone));
        let code = t.last_code();
        let pg = t.p.verify(ip, Some(mac), &form(&[("kod", &code)]));
        t.advance(61.0);
        pg
    }

    #[test]
    fn ip_change_moves_session_without_sms() {
        let t = setup(|_| {});
        login(&t, MAC, IP, "5334553132");
        let before = t.sms_count();
        assert_eq!(t.p.home("10.50.0.50", Some(MAC), "").tpl, "basarili");
        assert_eq!(t.sms_count(), before);
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c[1] == "add" && c.last().unwrap().contains("10.50.0.50")));
        assert!(calls.iter().any(|c| c[1] == "delete" && c.last().unwrap().contains(IP)));
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].ip, "10.50.0.50");
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("OTURUM_IP_DEGISTI"));
    }

    #[test]
    fn device_limit_closes_oldest() {
        let t = setup(|c| c.main.max_devices_per_phone = 2);
        login(&t, "aa:bb:cc:dd:ee:01", "10.50.0.21", "5334553132");
        login(&t, "aa:bb:cc:dd:ee:02", "10.50.0.22", "5334553132");
        login(&t, "aa:bb:cc:dd:ee:03", "10.50.0.23", "5334553132");
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses.keys().cloned().collect::<Vec<_>>(), vec!["aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:03"]);
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("neden=cihaz_limiti"));
    }

    #[test]
    fn same_ip_holder_is_parked() {
        let t = setup(|_| {});
        login(&t, "aa:bb:cc:dd:ee:01", IP, "5334553132");
        login(&t, "aa:bb:cc:dd:ee:02", IP, "5321112233"); // DHCP aynı IP'yi yeni cihaza verdi
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses["aa:bb:cc:dd:ee:01"].ip, "");
        assert_eq!(ses["aa:bb:cc:dd:ee:02"].ip, IP);
    }

    #[test]
    fn nft_failure_writes_no_session() {
        let t = setup(|_| {});
        let pg = login(&t, MAC, "10.50.0.99", "5334553132");
        assert_eq!((pg.tpl, pg.get("mesaj")), ("hata", "Bağlantı açılamadı, lütfen tekrar deneyin."));
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
    }

    #[test]
    fn rendering_escapes_and_safe_redirects() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "<script>x</script>"), ("dst", "javascript:alert(1)")]));
        let html = t.p.render(&pg);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>x"));
        assert!(html.contains("<h1>Bocafe&#x27;ye hoş geldiniz</h1>")); // ek ada göre, kesme işaretiyle
        assert!(html.contains("name=\"dst\" value=\"\""));
        assert_eq!(safe_dst("https://a.com/x"), "https://a.com/x");
        assert_eq!(safe_dst(&format!("http://{}", "a".repeat(2050))), "");
        assert_eq!(parse_query("a=1&a=2&b=&c=%C5%9F+x&d=%zz"), form(&[("a", "1"), ("c", "ş x"), ("d", "%zz")]));
        assert_eq!(pct_encode("http://a.com/?q=1"), "http%3A%2F%2Fa.com%2F%3Fq%3D1");
        let mut v = HashMap::new();
        v.insert("site", "X".to_string());
        assert_eq!(substitute("$site'ye $bilinmeyen $$5", &v), "X'ye $bilinmeyen $5");
    }

    #[test]
    fn foreign_numbers_and_twilio() {
        // merkezde Twilio kapalı: merkez YABANCI döner, cihaz bugünkü metni gösterir (numara cihazda süzülmez)
        let t = setup(|_| {});
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: false, kod: "YABANCI".into(), is: None, saglayici: Saglayici::Netgsm, mock: false });
        let f = form(&[("ad", "John"), ("soyad", "Smith"), ("ulke", "DE"), ("telefon", "0151 2345 6789"), ("sozlesme", "1"), ("riza", "1")]);
        let pg = t.p.send_code(IP, Some(MAC), &f);
        assert_eq!(pg.get("hata_genel"), sms::YABANCI_YOK);
        assert!(t.p.render(&pg).contains("<option value=\"DE\" selected>")); // seçim korunur
        assert_eq!(t.sms.lock().unwrap()[0].0, "4915123456789");
        assert!(t.audit_text().contains("red=yabanci_numara"));
        // merkez deneme modunda Twilio seçse de kod yerelde doğrulanır (merkeze doğrulama gitmez)
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &f);
        assert_eq!((pg.tpl, pg.get("telefon_maskeli")), ("kod", "+49 XXXXXXXXX89"));
        let code = t.last_code();
        assert_eq!(t.sms.lock().unwrap()[0], ("4915123456789".to_string(), code.clone(), "twilio"));
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", &code)])).tpl, "basarili");
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].phone, "4915123456789");
    }

    #[test]
    fn twilio_real_mode_checks_remotely() {
        let t = setup(|_| {});
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: true, kod: "0".into(), is: None, saglayici: Saglayici::Twilio, mock: false });
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        assert!(t.audit_text().contains("twilio=None"));
        // cihazın kodu geçmez: kodu Twilio üretti (Twilio'nun onayladığı kod cihazınkinden farklı)
        let code = t.last_code();
        let twilio_kodu = if code == "424242" { "535353" } else { "424242" };
        *t.onayli.lock().unwrap() = twilio_kodu.into();
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", &code)])).get("hata"), "Kod hatalı. Kalan deneme hakkı: 4");
        assert_eq!(*t.kontrol.lock().unwrap(), [code.clone()]); // karar merkezde
        // ağ hatası: deneme hakkı düşmez
        *t.ag.lock().unwrap() = true;
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", twilio_kodu)]));
        assert_eq!(pg.get("hata"), "Kod şu an doğrulanamadı, lütfen birkaç saniye sonra tekrar deneyin.");
        *t.ag.lock().unwrap() = false;
        let pg = t.p.verify(IP, Some(MAC), &form(&[("kod", "111111")]));
        assert_eq!(pg.get("hata"), "Kod hatalı. Kalan deneme hakkı: 3");
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", twilio_kodu)])).tpl, "basarili");
    }

    #[test]
    fn twilio_alti_haneli_olmayan_kod_merkeze_gitmeden_yanlis() {
        // merkez 6 hane dışını 400 ile reddeder; cihaz bunu ağ hatası sayıp hakkı düşürmeseydi sınırsız deneme olurdu
        let t = setup(|_| {});
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: true, kod: "0".into(), is: None, saglayici: Saglayici::Twilio, mock: false });
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        for (kod, kalan) in [("12345", 4), ("12a456", 3), ("1234567", 2), ("", 1)] {
            assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", kod)])).get("hata"), format!("Kod hatalı. Kalan deneme hakkı: {kalan}"), "{kod}");
        }
        assert!(t.kontrol.lock().unwrap().is_empty());
        *t.onayli.lock().unwrap() = "123456".into();
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", "12 34 56")])).tpl, "basarili"); // boşluklu 6 rakam merkeze gider
        assert_eq!(*t.kontrol.lock().unwrap(), ["123456"]);
    }

    #[test]
    fn sms_merkezden_basari_ve_hatalar() {
        // başarı: iş numarası denetimde
        let t = setup(|_| {});
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: true, kod: "0".into(), is: Some("J77".into()), saglayici: Saglayici::Netgsm, mock: false });
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "kod");
        assert!(t.audit_text().contains("OTP_GONDERILDI") && t.audit_text().contains("netgsm=J77"));
        assert_eq!(t.p.verify(IP, Some(MAC), &form(&[("kod", &t.last_code())])).tpl, "basarili"); // NetGSM: yerelde
        // merkeze ulaşılamadı / sınır: misafire tek metin, denetimde sağlayıcı=kod
        for (kod, saglayici, ek) in [("AG", Saglayici::Netgsm, "netgsm=AG"), ("SINIR", Saglayici::Netgsm, "netgsm=SINIR"), ("AG", Saglayici::Twilio, "twilio=AG")] {
            let t = setup(|_| {});
            *t.cevap.lock().unwrap() = Some(Sonuc { ok: false, kod: kod.into(), is: None, saglayici, mock: false });
            let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
            assert_eq!(pg.get("hata_genel"), "Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin.", "{kod}");
            assert!(t.audit_text().contains("OTP_HATA") && t.audit_text().contains(ek), "{ek}");
            let mut en = good("5334553132");
            en.insert("dil".into(), "en".into());
            let pg = t.p.send_code(IP, Some(MAC), &en);
            assert_eq!(pg.get("hata_genel"), "The code cannot be sent right now, please try again a little later.");
        }
        // NetGSM'in kendi hata kodu bugünkü metniyle
        let t = setup(|_| {});
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: false, kod: "50".into(), is: None, saglayici: Saglayici::Netgsm, mock: false });
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).get("hata_genel"), "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.");
    }

    #[test]
    fn consent_texts_and_recording() {
        assert_eq!(metin_html(""), "<p class=\"not\">Metin henüz eklenmedi.</p>");
        assert_eq!(metin_html("Birinci <b>\r\nsatır\r\n\r\nİkinci"), "<p>Birinci &lt;b&gt;<br>satır</p><p>İkinci</p>");
        assert_eq!(metin_html("BAŞLIK\ngiriş:\n* bir\n* <iki>\nson"), "<p><strong>BAŞLIK</strong><br>giriş:</p><ul><li>bir</li><li>&lt;iki&gt;</li></ul><p>son</p>");
        let t = setup(|c| c.main.unvan = "Boca Gıda Tic. Ltd. Şti.".into());
        let page = t.p.render(&t.p.form_page(&Form::new(), vec![], ""));
        // açık rıza metni onay kutusunun yanında, unvan yerleşmiş, kutu zorunlu
        assert!(page.contains("name=\"riza\" value=\"1\"  required><span>Şirketinizin Wi-Fi internet hizmetini"));
        assert!(page.contains("ile paylaştığım kişisel verilerimin (ad, soyad, cep telefonu numarası) Boca Gıda Tic. Ltd. Şti. ile paylaşılmasına onay veriyorum."));
        assert!(!page.contains("İsteğe bağlı") && !page.contains("Metin henüz eklenmedi") && !page.contains("[vergi levhası unvanı]"));
        assert!(page.contains("<summary>KVKK Aydınlatma Metni</summary>") && page.contains("<li>Haberleşme ve İletişim Bilgileri: Telefon numarası</li>"));
        assert!(page.contains("<strong>SON KULLANICI LİSANS SÖZLEŞMESİ</strong>") && page.contains("Bocafe’nin uhdesindeki") && !page.contains("İŞLETMECİ"));
        assert!(page.contains("<strong>9. BOCAFE’NİN YETKİ VE İMKÂNLARI</strong>"));
        // aydınlatma metninde veri sorumlusu ve başvuru yeri işletmenin unvanı; başka işletme adı yok
        assert!(page.contains("Boca Gıda Tic. Ltd. Şti. (“İşletme”), veri sorumlusu sıfatıyla"));
        // adres girilmemiş: yer tutucu görünür; isteğe bağlı (telefon, KEP, e-posta) boş: satırları hiç yok
        assert!(page.contains("ADRESLERİ</strong><br>Boca Gıda Tic. Ltd. Şti.</p><ul><li>Müracaat Adresi: [başvuru adresi]</li></ul>"));
        assert!(page.contains("<li>Şahsen merkezimize ([başvuru adresi]) gelerek"));
        assert!(!page.contains("İrtibat Numarası") && !page.contains("E-Posta:") && !page.contains("KEP Adresi") && !page.contains("KEP adresimize"));
        assert!(!page.contains("kayıtlı bulunan KEP (Kayıtlı Elektronik Posta)")); // KEP'siz işletmede KEP ile başvuru satırı yok
        assert!(!page.contains("goztepebilgisayar") && !page.contains("Göztepe Bilgisayar ("));
        // bütün iletişim bilgileri girilmiş: hepsi yerinde
        let tt = setup(|c| {
            c.main.unvan = "Boca Gıda Tic. Ltd. Şti.".into();
            c.main.adres = "Moda Cad. No 1 Kadıköy/İstanbul".into();
            c.main.telefon = "0216 000 00 00".into();
            c.main.eposta = "info@boca.example".into();
            c.main.kep = "boca@hs01.kep.tr".into();
        });
        let pt = tt.p.render(&tt.p.form_page(&Form::new(), vec![], ""));
        assert!(pt.contains(
            "ADRESLERİ</strong><br>Boca Gıda Tic. Ltd. Şti.</p><ul><li>Müracaat Adresi: Moda Cad. No 1 Kadıköy/İstanbul</li>\
             <li>İrtibat Numarası: 0216 000 00 00</li><li>KEP Adresi: boca@hs01.kep.tr</li><li>E-Posta: info@boca.example</li></ul>"
        ));
        assert!(pt.contains("<li>Şahsen merkezimize (Moda Cad. No 1 Kadıköy/İstanbul) gelerek"));
        assert!(pt.contains("adresiniz kanalıyla boca@hs01.kep.tr KEP adresimize</li></ul>"));
        assert!(!pt.contains("[başvuru adresi]") && !pt.contains("[KEP adresi]") && !pt.contains("[irtibat numarası]") && !pt.contains("[e-posta adresi]"));
        // unvan girilmemişse yer tutucu görünür (kurulum yapılmamış)
        let t0 = setup(|_| {});
        let p0 = t0.p.render(&t0.p.form_page(&Form::new(), vec![], ""));
        assert!(p0.contains("[vergi levhası unvanı] (“İşletme”), veri sorumlusu") && p0.contains("ADRESLERİ</strong><br>[vergi levhası unvanı]</p>"));
        // rıza ya da sözleşme işaretlenmeden kod gönderilmez
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", "05334553132"), ("sozlesme", "1")]));
        assert!(!pg.get("hata_riza").is_empty() && t.sms_count() == 0);
        let pg = t.p.send_code(IP, Some(MAC), &form(&[("ad", "Ayşe"), ("soyad", "Yılmaz"), ("telefon", "05334553132"), ("riza", "1")]));
        assert!(!pg.get("hata_sozlesme").is_empty() && t.sms_count() == 0);
        // ikisi de işaretli: giriş, kayda yazılır
        let pg = t.p.send_code(IP, Some(MAC), &good("05334553132"));
        assert_eq!(pg.tpl, "kod");
        let code = t.last_code();
        t.p.verify(IP, Some(MAC), &form(&[("kod", &code)]));
        assert!(t.read("5651/gunluk/2026-09-29/oturum.csv").contains("sozlesme=1 riza=1"));
    }

    #[test]
    fn business_name_with_turkish_suffixes() {
        let t = "İŞLETMECİ’ye, İŞLETMECİ’nin, İŞLETMECİ’den; İŞLETMECİ, 9. İŞLETMECİ’NİN";
        assert_eq!(isletme_yerlestir(t, "Bocafe"), "Bocafe’ye, Bocafe’nin, Bocafe’den; Bocafe, 9. BOCAFE’NİN");
        assert_eq!(isletme_yerlestir(t, "Starbucks"), "Starbucks’a, Starbucks’un, Starbucks’tan; Starbucks, 9. STARBUCKS’UN");
        assert_eq!(isletme_yerlestir(t, "Kahvecim"), "Kahvecim’e, Kahvecim’in, Kahvecim’den; Kahvecim, 9. KAHVECİM’İN");
        // ponytail: iyelik ekli adlarda doğrusu ’na/’ndan; yazımdan ayırt edilemez, genel kural uygulanır
        assert_eq!(isletme_yerlestir(t, "Kahve Dünyası"), "Kahve Dünyası’ya, Kahve Dünyası’nın, Kahve Dünyası’dan; Kahve Dünyası, 9. KAHVE DÜNYASI’NIN");
        assert_eq!(isletme_yerlestir("Göztepe Kafe’ye", "X"), "Göztepe Kafe’ye"); // başka kelimeye dokunmaz
        assert_eq!((ek("Hilton Otel", 'y'), ek("Starbucks", 'y'), ek("Kahvecim", 'y'), ek("Boca", 'y')), ("e".into(), "a".into(), "e".into(), "ya".into()));
    }

    #[test]
    fn lisans_kapaliyken_portal_internet_vermez() {
        let t = setup(|_| {});
        let mut m = crate::merkez::oku(&t.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&t.p.merkez_path, &m).unwrap();
        let pg = t.p.home(IP, Some(MAC), "");
        assert_eq!(pg.tpl, "hata");
        assert!(t.p.render(&pg).contains("internet hizmeti şu an kapalı"));
        assert_eq!(t.p.send_code(IP, Some(MAC), &good("5334553132")).tpl, "hata");
        assert_eq!(t.sms_count(), 0);
        crate::merkez::sil(&t.p.merkez_path); // bağlı değil: kapalı
        assert_eq!(t.p.home(IP, Some(MAC), "").tpl, "hata");
    }

    fn en(pairs: &[(&str, &str)]) -> Form {
        let mut f = form(pairs);
        f.insert("dil".into(), "en".into());
        f
    }

    #[test]
    fn en_formu_ve_hatalar_ingilizce() {
        let t = setup(|_| {});
        let pg = t.p.send_code(IP, Some(MAC), &en(&[("ad", "A"), ("soyad", "Ali1"), ("telefon", "212 455 31 32")]));
        assert_eq!((pg.tpl, pg.dil), ("giris", Dil::En));
        assert_eq!(pg.get("hata_ad"), "Enter your first name using letters (2-40 characters).");
        assert_eq!(pg.get("hata_telefon"), "Enter a valid mobile number (5XX XXX XX XX).");
        assert_eq!(pg.get("hata_sozlesme"), "Accept the Internet User Agreement to continue.");
        let html = t.p.render(&pg);
        assert!(html.contains("<html lang=\"en\">") && html.contains("<h1>Welcome to Bocafe</h1>"));
        assert!(html.contains(">First name</label>") && html.contains(">Send code</button>") && html.contains("Bocafe · Guest Wi-Fi"));
        assert!(html.contains("I have read and accept the Internet User Agreement.") && html.contains("All rights reserved."));
        assert!(html.contains("<input type=\"hidden\" name=\"dil\" value=\"en\">"));
        assert!(!html.contains("hoş geldiniz") && !html.contains("Kod Gönder") && !html.contains("$m_"));
        // sağlayıcı iletisi de çevrilir (merkez yabancı numarayı reddetti)
        *t.cevap.lock().unwrap() = Some(Sonuc { ok: false, kod: "YABANCI".into(), is: None, saglayici: Saglayici::Netgsm, mock: false });
        let f = en(&[("ad", "John"), ("soyad", "Smith"), ("ulke", "DE"), ("telefon", "0151 2345 6789"), ("sozlesme", "1"), ("riza", "1")]);
        assert_eq!(t.p.send_code(IP, Some(MAC), &f).get("hata_genel"), "SMS cannot be sent to foreign numbers right now, please ask the staff.");
        *t.cevap.lock().unwrap() = None;
        // kod sayfası, tekrar gönderme, yanlış ve doğru kod
        let mut g = good("5334553132");
        g.insert("dil".into(), "en".into());
        let html = t.p.render(&t.p.send_code(IP, Some(MAC), &g));
        assert!(html.contains("We sent a 6-digit code to +90 5XX XXX XX 32. The code is valid for 3 minutes."));
        assert!(html.contains(">Resend code</button>") && html.contains(">Change number</a>") && !html.contains("Doğrula"));
        assert!(t.p.resend_dil(IP, Some(MAC), Dil::En).get("not").starts_with("A code was just sent. Please wait"));
        let code = t.last_code();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        assert_eq!(t.p.verify(IP, Some(MAC), &en(&[("kod", wrong)])).get("hata"), "Incorrect code. Attempts left: 4");
        let html = t.p.render(&t.p.verify(IP, Some(MAC), &en(&[("kod", &code)])));
        assert!(html.contains("<h1>You are connected</h1>") && html.contains("You are connected to Bocafe Wi-Fi") && html.contains("<b>Done</b>"));
        // hata sayfası
        let html = t.p.render(&t.p.home_dil(IP, None, "", Dil::En, false));
        assert!(html.contains("Your device could not be identified") && html.contains(">Start over</a>") && html.contains("Something went wrong"));
    }

    #[test]
    fn dil_baglantilari_dst_ve_kod_sayfasini_korur() {
        let t = setup(|_| {});
        let base = t.p.base_url.clone();
        let html = t.p.render(&t.p.home(IP, Some(MAC), "http://neverssl.com/a?b=1"));
        assert!(html.contains("<html lang=\"tr\">"));
        assert!(html.contains(&format!("href=\"{base}/?dil=en&amp;dst=http%3A%2F%2Fneverssl.com%2Fa%3Fb%3D1\"")));
        assert!(html.contains(&format!("href=\"{base}/?dil=tr&amp;dst=http%3A%2F%2Fneverssl.com%2Fa%3Fb%3D1\" lang=\"tr\" aria-current=\"true\">TR</a>")));
        assert!(html.contains("<input type=\"hidden\" name=\"dil\" value=\"tr\">"));
        // kod sayfasında dil değişince kod sayfası yeni dilde gelir; "Numarayı değiştir" (sayfa yok) formu açar
        let pg = t.p.send_code(IP, Some(MAC), &good("5334553132"));
        assert!(t.p.render(&pg).contains(&format!("href=\"{base}/?dil=en&amp;sayfa=kod\"")));
        let pg = t.p.home_dil(IP, Some(MAC), "", Dil::En, true);
        assert_eq!((pg.tpl, pg.dil), ("kod", Dil::En));
        assert_eq!(t.p.home_dil(IP, Some(MAC), "", Dil::En, false).tpl, "giris");
        t.advance(181.0); // kod süresi dolduysa form
        assert_eq!(t.p.home_dil(IP, Some(MAC), "", Dil::En, true).tpl, "giris");
    }

    // --- misafir kodu
    const TC: &str = "10000000146"; // algoritmaya uyan uydurma numara
    const TC_KEY: &str = "TC:10000000146";
    const PAS_KEY: &str = "PASAPORT:DE:X1234567";

    fn otel(tur: &'static str) -> impl FnOnce(&mut Config) {
        move |c: &mut Config| {
            c.main.isletme_turu = Some(tur.into());
            c.giris.misafir_kodu = true;
        }
    }

    /// Personelin panelden ürettiği kod (depoya yazılır).
    fn kod_uret(t: &T, kimlik: &str, saat: u64, uretim: f64) -> misafir::Kod {
        let mut k = misafir::oku(&t.p.cfg.main.state_root);
        let (yeni, _) = misafir::uret(&mut k, kimlik, "Ali", "Veli", "101", saat, "mudur", uretim);
        misafir::kaydet(&t.p.cfg.main.state_root, &k).unwrap();
        yeni
    }

    fn mform(tur: &str, kimlik: &str, kod: &str) -> Form {
        let mut f = form(&[("tur", tur), ("kod", kod), ("sozlesme", "1"), ("riza", "1"), ("dst", "http://neverssl.com/")]);
        match tur {
            "tc" => {
                f.insert("tc".into(), kimlik.into());
            }
            _ => {
                f.insert("pasaport".into(), kimlik.into());
                f.insert("uyruk".into(), "DE".into());
            }
        }
        f
    }

    fn baska(kod: &str) -> String {
        if kod == "000000" { "111111".into() } else { "000000".into() }
    }

    #[test]
    fn misafir_tc_ile_giris() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let pg = t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap();
        assert_eq!((pg.tpl, pg.get("baslik")), ("misafir", "Otel misafiriyim"));
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", "100 0000 0146", &k.kod)).unwrap();
        assert_eq!(pg.tpl, "basarili");
        assert!(pg.get("bas_html").contains("url=http://neverssl.com/"));
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!((ses[MAC].phone.as_str(), ses[MAC].ad.as_str(), ses[MAC].soyad.as_str()), (TC_KEY, "Ali", "Veli"));
        assert_eq!(ses[MAC].expires_epoch, k.bitis);
        let calls = t.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout 86400s }}")));
        // 5651: oturum satırı tam kimlikle, ek alanı; kişi dosyası güvenli adla; kişi listesi
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        let bitis = ortak::now_iso(k.bitis);
        let satir = oturum.lines().find(|l| l.contains("OTURUM_BASLA")).unwrap();
        assert!(satir.contains(TC_KEY), "{satir}");
        assert!(satir.ends_with(&format!("bitis={bitis} sozlesme=1 riza=1 giris=misafir_kodu oda=101 kod_bitis={bitis} ureten=mudur")), "{satir}");
        let user = t.read("5651/gunluk/2026-09-29/kullanicilar/TC_10000000146.csv");
        assert!(user.contains(";KAYIT;TC:10000000146;Ali;Veli;") && user.contains(";OTURUM_BASLA;"));
        assert!(t.read("5651/kullanicilar/index.csv").contains("TC:10000000146;Ali;Veli;"));
        // denetim: kimlik ve kod yok, maskeli kimlik var
        let a = t.audit_text();
        assert!(a.contains("MISAFIR_BASARILI") && a.contains("100******46"), "{a}");
        assert!(!a.contains(TC) && !a.contains(&k.kod), "{a}");
        assert_eq!(t.sms_count(), 0);
        // zaten bağlı: sayfa başarı
        assert_eq!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap().tpl, "basarili");
    }

    #[test]
    fn misafir_pasaport_ile_giris() {
        let t = setup(otel("yurt"));
        let k = kod_uret(&t, PAS_KEY, 48, WALL0);
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("pasaport", "x123-4567", &k.kod)).unwrap();
        assert_eq!(pg.tpl, "basarili");
        assert_eq!(ortak::load_sessions(&t.p.cfg.main.state_root)[MAC].phone, PAS_KEY);
        assert!(t.read("5651/gunluk/2026-09-29/kullanicilar/PASAPORT_DE_X1234567.csv").contains(";KAYIT;PASAPORT:DE:X1234567;"));
        let a = t.audit_text();
        assert!(a.contains("DE X12***67") && !a.contains("X1234567"), "{a}");
        // uyruk farklıysa eşleşmez
        let mut f = mform("pasaport", "X1234567", &k.kod);
        f.insert("uyruk".into(), "FR".into());
        assert_eq!(t.p.misafir_dogrula("10.50.0.24", Some("aa:bb:cc:dd:ee:02"), &f).unwrap().get("hata_genel"), "Bilgiler eşleşmedi ya da kodun süresi doldu.");
    }

    #[test]
    fn misafir_yanlis_kod_tek_mesaj() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let iptal = kod_uret(&t, PAS_KEY, 24, WALL0);
        let mut kodlar = misafir::oku(&t.p.cfg.main.state_root);
        misafir::iptal(&mut kodlar, &iptal.kod_id);
        misafir::kaydet(&t.p.cfg.main.state_root, &kodlar).unwrap();
        let eski = kod_uret(&t, "TC:20000000046", 24, WALL0 - 25.0 * 3600.0); // süresi dolmuş
        const TEK: &str = "Bilgiler eşleşmedi ya da kodun süresi doldu.";
        let denemeler = [
            mform("tc", TC, &baska(&k.kod)),          // kod yanlış
            mform("tc", "11111111110", &k.kod),       // kimlik yok
            mform("tc", "20000000046", &eski.kod),     // süresi dolmuş
            mform("pasaport", "X1234567", &iptal.kod), // iptal edilmiş
        ];
        for (i, f) in denemeler.iter().enumerate() {
            let pg = t.p.misafir_dogrula(IP, Some(MAC), f).unwrap();
            assert_eq!((pg.tpl, pg.get("hata_genel")), ("misafir", TEK), "{i}");
            assert!(pg.ctx.iter().all(|(k2, _)| *k2 != "kod"), "kod forma geri yazılmaz");
        }
        // İngilizce
        let mut f = mform("tc", "20000000046", &eski.kod);
        f.insert("dil".into(), "en".into());
        let pg = t.p.misafir_dogrula("10.50.0.25", Some("aa:bb:cc:dd:ee:03"), &f).unwrap();
        assert_eq!(pg.get("hata_genel"), "The details did not match or the code has expired.");
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        // biçim hataları alan iletisi; hata sayılmaz
        let pg = t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", "10000000147", "12")).unwrap();
        assert!(pg.get("hata_genel").is_empty() && !pg.get("hata_tc").is_empty() && !pg.get("hata_kod").is_empty());
        let a = t.audit_text();
        assert_eq!(a.matches("MISAFIR_HATALI").count(), 5, "{a}");
        assert!(!a.contains(TC) && !a.contains("11111111110") && !a.contains("20000000046") && !a.contains("X1234567"), "{a}");
    }

    #[test]
    fn misafir_kilit_ip_ve_mac() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        // kodu olmayan kimlikle hatalar (bu koda kod başına tavan sayılmasın)
        let yanlis = mform("tc", "11111111110", "000000");
        let dogru = mform("tc", TC, &k.kod);
        const KILIT: &str = "Çok fazla hatalı deneme yapıldı. Lütfen 15 dakika sonra tekrar deneyin.";
        // aynı IP, farklı MAC'ler: 5 hata → IP kilitli, doğru kod da reddedilir
        for i in 0..5 {
            t.p.misafir_dogrula(IP, Some(&format!("aa:bb:cc:dd:ee:1{i}")), &yanlis);
        }
        assert_eq!(t.p.misafir_dogrula(IP, Some("aa:bb:cc:dd:ee:20"), &dogru).unwrap().get("hata_genel"), KILIT);
        // aynı MAC, farklı IP'ler: 5 hata → MAC kilitli
        for i in 0..5 {
            t.p.misafir_dogrula(&format!("10.50.0.{}", 30 + i), Some("aa:bb:cc:dd:ee:30"), &yanlis);
        }
        assert_eq!(t.p.misafir_dogrula("10.50.0.40", Some("aa:bb:cc:dd:ee:30"), &dogru).unwrap().get("hata_genel"), KILIT);
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        // SMS doğrulama sayacından ayrı: SMS akışı kilitlenmez
        assert_eq!(t.p.send_code(IP, Some("aa:bb:cc:dd:ee:20"), &good("5334553132")).tpl, "kod");
        // başka IP ve MAC etkilenmez
        assert_eq!(t.p.misafir_dogrula("10.50.0.41", Some("aa:bb:cc:dd:ee:41"), &dogru).unwrap().tpl, "basarili");
        // 15 dk sonra kilit kalkar
        t.advance(901.0);
        assert_eq!(t.p.misafir_dogrula(IP, Some("aa:bb:cc:dd:ee:20"), &dogru).unwrap().tpl, "basarili");
        assert!(t.audit_text().contains("red=kilit"));
    }

    #[test]
    fn misafir_kod_basina_hata_tavani() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        const TEK: &str = "Bilgiler eşleşmedi ya da kodun süresi doldu.";
        // MAC/IP değiştirerek 10 yanlış deneme (IP/MAC kilidine takılmadan)
        for i in 0..10 {
            let pg = t.p.misafir_dogrula(&format!("10.50.0.{}", 100 + i), Some(&format!("aa:bb:cc:dd:ef:{i:02}")), &mform("tc", TC, &baska(&k.kod)));
            assert_eq!(pg.unwrap().get("hata_genel"), TEK, "{i}");
        }
        // artık doğru kod da aynı iletiyle reddedilir
        let pg = t.p.misafir_dogrula("10.50.0.120", Some("aa:bb:cc:dd:ef:20"), &mform("tc", TC, &k.kod)).unwrap();
        assert_eq!((pg.tpl, pg.get("hata_genel")), ("misafir", TEK));
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        let a = t.audit_text();
        assert_eq!(a.matches("MISAFIR_KABA_KUVVET").count(), 1, "{a}");
        assert!(a.contains("100******46") && a.contains("oda=101") && !a.contains(TC) && !a.contains(&k.kod), "{a}");
        // personel yeni kod üretir: yeni sayaç, giriş açılır
        let yeni = kod_uret(&t, TC_KEY, 24, WALL0);
        assert_eq!(t.p.misafir_dogrula("10.50.0.121", Some("aa:bb:cc:dd:ef:21"), &mform("tc", TC, &yeni.kod)).unwrap().tpl, "basarili");
        assert_eq!(t.audit_text().matches("MISAFIR_KABA_KUVVET").count(), 1);
        // kodu olmayan kimlik için de kimlik başına tavan
        for i in 0..10 {
            t.p.misafir_dogrula(&format!("10.50.0.{}", 140 + i), Some(&format!("aa:bb:cc:dd:f0:{i:02}")), &mform("tc", "11111111110", "000000"));
        }
        assert_eq!(t.audit_text().matches("MISAFIR_KABA_KUVVET").count(), 2);
    }

    #[test]
    fn iptal_edilen_kodla_oturum_acilmaz() {
        // doğrulama ile oturum açma arasında iptal: state_lock altında yeniden kontrol
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let mut kodlar = misafir::oku(&t.p.cfg.main.state_root);
        misafir::iptal(&mut kodlar, &k.kod_id);
        misafir::kaydet(&t.p.cfg.main.state_root, &kodlar).unwrap();
        let g = Giris { kisi: TC_KEY, ad: "Ali", soyad: "Veli", bitis: Some(k.bitis), ek: String::new(), kod_id: Some(&k.kod_id) };
        assert!(t.p.open_session(IP, MAC, &g).is_none());
        assert!(ortak::load_sessions(&t.p.cfg.main.state_root).is_empty());
        assert!(t.calls.lock().unwrap().is_empty(), "nft'ye dokunulmaz");
        // uzatılmışsa yeni bitiş kullanılır
        let k2 = kod_uret(&t, TC_KEY, 24, WALL0);
        let mut kodlar = misafir::oku(&t.p.cfg.main.state_root);
        let yeni = misafir::uzat(&mut kodlar, &k2.kod_id, 24, WALL0).unwrap();
        misafir::kaydet(&t.p.cfg.main.state_root, &kodlar).unwrap();
        let ek = format!("giris=misafir_kodu kod_bitis={KOD_BITIS}");
        let g = Giris { kisi: TC_KEY, ad: "Ali", soyad: "Veli", bitis: Some(k2.bitis), ek, kod_id: Some(&k2.kod_id) };
        assert_eq!(t.p.open_session(IP, MAC, &g).unwrap().expires_epoch, yeni);
        // kayıttaki kod_bitis de yeniden okunan (uzatılmış) bitiş
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        assert!(oturum.contains(&format!("giris=misafir_kodu kod_bitis={}", ortak::now_iso(yeni))), "{oturum}");
        assert!(!oturum.contains(&ortak::now_iso(k2.bitis)));
    }

    #[test]
    fn misafir_oturum_kod_bitisinde_biter() {
        let t = setup(otel("otel"));
        // kod 4 saat önce 1 haftalık üretildi: kalan süre 164 saat
        let k = kod_uret(&t, TC_KEY, 168, WALL0 - 4.0 * 3600.0);
        assert_eq!(t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", TC, &k.kod)).unwrap().tpl, "basarili");
        let s = &ortak::load_sessions(&t.p.cfg.main.state_root)[MAC];
        assert_eq!(s.expires_epoch, WALL0 + 164.0 * 3600.0);
        let kalan = (164 * 3600).to_string();
        assert!(t.calls.lock().unwrap().iter().any(|c| c.last().unwrap() == &format!("{{ {MAC} . {IP} timeout {kalan}s }}")));
        // ayardaki 30 günlük oturum süresi uygulanmaz
        assert!(!t.calls.lock().unwrap().iter().any(|c| c.last().unwrap().contains("2592000s")));
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        assert!(oturum.contains(&format!("bitis={} ", ortak::now_iso(k.bitis))));
    }

    #[test]
    fn kod_basina_cihaz_siniri() {
        let t = setup(|c| {
            otel("otel")(c);
            c.main.max_devices_per_phone = 2;
        });
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        let diger = kod_uret(&t, PAS_KEY, 24, WALL0);
        assert_eq!(t.p.misafir_dogrula("10.50.0.29", Some("aa:bb:cc:dd:ee:09"), &mform("pasaport", "X1234567", &diger.kod)).unwrap().tpl, "basarili");
        for i in 1..=3 {
            t.advance(1.0);
            let pg = t.p.misafir_dogrula(&format!("10.50.0.2{i}"), Some(&format!("aa:bb:cc:dd:ee:0{i}")), &mform("tc", TC, &k.kod));
            assert_eq!(pg.unwrap().tpl, "basarili", "{i}");
        }
        let ses = ortak::load_sessions(&t.p.cfg.main.state_root);
        assert_eq!(ses.keys().cloned().collect::<Vec<_>>(), vec!["aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:03", "aa:bb:cc:dd:ee:09"]);
        let oturum = t.read("5651/gunluk/2026-09-29/oturum.csv");
        assert!(oturum.lines().any(|l| l.contains("OTURUM_BITIS") && l.contains("aa:bb:cc:dd:ee:01") && l.contains("neden=cihaz_limiti")));
    }

    #[test]
    fn kafede_misafir_yolu_yok() {
        // kafe (misafir_kodu ayarı açık kalmış olsa da) ve otel ama kapalı: yol yok, düğme yok
        for tweak in [otel("kafe"), otel("diger")] {
            let t = setup(tweak);
            kod_uret(&t, TC_KEY, 24, WALL0);
            assert!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).is_none());
            assert!(t.p.misafir_dogrula(IP, Some(MAC), &mform("tc", TC, "123456")).is_none());
            assert!(!t.p.render(&t.p.home(IP, Some(MAC), "")).contains("/misafir"));
        }
        let t = setup(|c| c.main.isletme_turu = Some("otel".into()));
        assert!(t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).is_none());
        assert!(!t.p.render(&t.p.home(IP, Some(MAC), "")).contains("/misafir"));
        // otel / yurt açık: türe göre düğme, dst ve dil korunur
        let t = setup(otel("otel"));
        let base = t.p.base_url.clone();
        let html = t.p.render(&t.p.home(IP, Some(MAC), "http://a.com/"));
        assert!(html.contains(&format!("<a class=\"dugme\" href=\"{base}/misafir?dil=tr&amp;dst=http%3A%2F%2Fa.com%2F\">Otel misafiriyim</a>")), "{html}");
        assert!(t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false)).contains(">Hotel guest</a>"));
        let t = setup(otel("yurt"));
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains(">Yurt sakiniyim</a>"));
        let html = t.p.render(&t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::En).unwrap());
        assert!(html.contains("<h1>Dormitory resident</h1>") && html.contains("Enter the code given or sent to you by our staff"));
        assert!(html.contains(&format!("href=\"{base}/misafir?dil=tr\"")) && html.contains(&format!("action=\"{base}/misafir/dogrula\"")));
    }

    #[test]
    fn kupon_kelimesi_yok() {
        let t = setup(otel("otel"));
        let k = kod_uret(&t, TC_KEY, 24, WALL0);
        for d in [Dil::Tr, Dil::En] {
            let mut sayfalar = vec![
                t.p.home_dil(IP, Some(MAC), "", d, false),
                t.p.misafir_sayfasi(IP, Some(MAC), "", d).unwrap(),
            ];
            let mut f = mform("tc", TC, &baska(&k.kod));
            f.insert("dil".into(), d.kod().into());
            sayfalar.push(t.p.misafir_dogrula(IP, Some(MAC), &f).unwrap());
            for pg in &sayfalar {
                let html = t.p.render(pg).to_lowercase();
                assert!(!html.contains("kupon") && !html.contains("coupon"), "{}", pg.tpl);
            }
        }
        let html = t.p.render(&t.p.misafir_sayfasi(IP, Some(MAC), "", Dil::Tr).unwrap());
        assert!(html.contains("Personelin size verdiği veya ilettiği kodu girin") && !html.contains("$m_"));
        for tpl in ["giris", "misafir", "_sayfa", "basarili", "hata", "kod"] {
            assert!(!template(tpl).to_lowercase().contains("kupon"), "{tpl}");
        }
    }

    #[test]
    fn hukuki_metin_en_bossa_turkce() {
        // İngilizce alanlar boş: Türkçe metinler (eklerle) görünür, başlıklar İngilizce
        let t = setup(|c| c.main.unvan = "Boca Ltd.".into());
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("Bocafe’nin uhdesindeki") && html.contains("Şirketinizin Wi-Fi internet hizmetini"));
        assert!(html.contains("<summary>Privacy Notice (KVKK)</summary>") && html.contains("<summary>Internet User Agreement</summary>"));
        // İngilizce yazılmışsa: İŞLETMECİ yalnızca adla, unvan yer tutucusu unvanla değişir
        let t = setup(|c| {
            c.main.unvan = "Boca Ltd.".into();
            c.portal.aydinlatma_en = "Data controller: İŞLETMECİ".into();
            c.portal.acik_riza_en = "I consent to sharing with [vergi levhası unvanı].".into();
            c.portal.sozlesme_en = "Rules of İŞLETMECİ’s network".into();
        });
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("Data controller: Bocafe") && html.contains("I consent to sharing with Boca Ltd.") && html.contains("Rules of Bocafe’s network"));
        assert!(!html.contains("uhdesindeki") && !html.contains("Şirketinizin"));
        // Türkçe sayfa İngilizce metinlerden etkilenmez
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(html.contains("Bocafe’nin uhdesindeki") && !html.contains("Data controller"));
        // ikisi de boşsa bilgi notu sayfanın dilinde
        let t = setup(|c| c.portal.aydinlatma = String::new());
        assert!(t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false)).contains("This text has not been added yet."));
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains("Metin henüz eklenmedi."));
    }

    // --- internet yokken uyarı ve Servis (docs/superpowers/specs/2026-10-23-pppoe-design.md)
    const AP_PAROLA: &str = "Kp7mQx2Rtz4W";
    const UYARI: &str = "Bu cihazın internet bağlantısı şu an yok";
    const SERVIS_LINKI: &str = "href=\"/servis\"";
    const KAYDEDILDI: &str = "Ayar kaydedildi";
    /// Geçerli yeni PPPoE bilgileri (Servis formunun alanları panelle aynı)
    const YENI: [(&str, &str); 4] = [("pppoe_acik", "acik"), ("pppoe_kullanici", "yeni@isp"), ("pppoe_parola", "yeni-parola-35"), ("pppoe_vlan", "35")];

    impl T {
        /// `None`: internet var; `Some(durum)`: yok. Durum önbellekte en çok 10 sn: yeni değer en geç 10 sn sonra görünür.
        fn internet_yok(&self, d: Option<Durum>) {
            *self.internet.lock().unwrap() = d;
        }
        fn ap_parola(&self, parola: &str) {
            let mut m = crate::merkez::oku(&self.p.merkez_path).unwrap();
            m.ap_parola = parola.into();
            crate::merkez::kaydet(&self.p.merkez_path, &m).unwrap();
        }
        fn ag_kaydet(&self, p: crate::ag::Pppoe) {
            crate::ag::save(&self.p.ag_yollar.ag, &crate::ag::Ag { pppoe: p, ..crate::ag::Ag::default() }).unwrap();
        }
        fn servis_dosyasi(&self) -> std::path::PathBuf {
            self.p.ag_yollar.durum.join("pppoe-servis.toml")
        }
        /// Hazırlık dosyasındaki `[pppoe]` (`ctl pppoe-servis`'in okuduğu biçim)
        fn hazir(&self) -> crate::ag::Pppoe {
            #[derive(serde::Deserialize)]
            struct Hazir {
                pppoe: crate::ag::Pppoe,
            }
            toml::from_str::<Hazir>(&std::fs::read_to_string(self.servis_dosyasi()).unwrap()).unwrap().pppoe
        }
        fn systemd_run(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().iter().filter(|c| c[0] == "systemd-run").cloned().collect()
        }
        /// POST /servis'in sayfası (internet yokken)
        fn kaydet(&self, ip: &str, mac: Option<&str>, f: &Form) -> String {
            self.p.render(&self.p.servis_kaydet(ip, mac, f).expect("internet yok: Servis sayfası"))
        }
        /// GET /servis (internet yokken)
        fn servis_html(&self, izle: bool) -> String {
            self.p.render(&self.p.servis_sayfasi(izle).expect("internet yok: Servis sayfası"))
        }
    }

    fn servis_form(parola: &str, alanlar: &[(&str, &str)]) -> Form {
        let mut f = form(alanlar);
        f.insert("servis_parola".into(), parola.into());
        f
    }

    /// Kayıtlı (eski) PPPoE bilgileri
    fn eski_pppoe() -> crate::ag::Pppoe {
        crate::ag::Pppoe { acik: true, kullanici: "eski@isp".into(), parola: "eski-parola-77".into(), ..Default::default() }
    }

    #[test]
    fn uyari_yalnizca_internet_yokken() {
        // internet var: giriş sayfası bugünkü gibi (uyarı ve Servis bağlantısı yok)
        let t = setup(|_| {});
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(!html.contains(UYARI) && !html.contains(SERVIS_LINKI), "{html}");
        // internet yok: sayfanın altında (formdan sonra) uyarı, neden (spec tablosundaki durum metni) ve Servis bağlantısı
        let t = setup(|_| {});
        t.internet_yok(Some(Durum::SunucuYok));
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        let uyari = html.find(UYARI).unwrap_or_else(|| panic!("{html}"));
        assert!(uyari > html.find("</form>").unwrap(), "uyarı sayfanın altında olmalı");
        let alt = &html[uyari..];
        assert!(alt.contains("PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)") && alt.contains(SERVIS_LINKI), "{html}");
        // İngilizce sayfada da
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("This device has no internet connection right now") && html.contains(SERVIS_LINKI) && !html.contains(UYARI), "{html}");
        // giriş formu hatayla yeniden çizilince de (misafir "kod gönderilemiyor" ile birlikte nedeni görür)
        assert!(t.p.render(&t.p.send_code(IP, Some(MAC), &form(&[("ad", "A")]))).contains(UYARI));
    }

    #[test]
    fn uyari_her_sayfada() {
        // telefonun internet yokken düşebileceği her sayfada (yalnızca giriş formunda değil) uyarı + Servis bağlantısı
        let t = setup(otel("otel"));
        assert_eq!(login(&t, MAC, IP, "5334553132").tpl, "basarili");
        let kod = t.p.send_code("10.50.0.32", Some("aa:bb:cc:dd:ee:32"), &good("5321112233"));
        assert_eq!(kod.tpl, "kod");
        // internet varken giriş yapmış telefonun başarı sayfası bugünkü gibi: uyarı yok, istenen sayfaya yönlenir
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(html.contains("http-equiv=\"refresh\"") && !html.contains(UYARI) && !html.contains(SERVIS_LINKI), "{html}");
        t.internet_yok(Some(Durum::DhcpYok));
        t.advance(10.0);
        // başarı sayfası: uyarı, neden, Servis; yönlendirme yok (hedef internetsiz açılmaz, bağlantıya dokunulabilsin)
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(html.contains("Bağlandınız") && html.contains(UYARI) && html.contains("Modemden adres alınamadı") && html.contains(SERVIS_LINKI), "{html}");
        assert!(!html.contains("http-equiv=\"refresh\""), "{html}");
        let html = t.p.render(&t.p.home_dil(IP, Some(MAC), "", Dil::En, false));
        assert!(html.contains("This device has no internet connection right now") && html.contains(SERVIS_LINKI), "{html}");
        // kod sayfası, misafir kodu sayfası, MAC tanınmadı (hata)
        let misafir = t.p.misafir_sayfasi("10.50.0.33", Some("aa:bb:cc:dd:ee:33"), "", Dil::Tr).unwrap();
        for pg in [kod, misafir, t.p.home(IP, None, "")] {
            let html = t.p.render(&pg);
            assert!(html.contains(UYARI) && html.contains(SERVIS_LINKI), "{}: {html}", pg.tpl);
        }
        // lisans kapalı (hata sayfası): Servis yine açık (internet gelmeden lisans da eşitlenemez)
        let mut m = crate::merkez::oku(&t.p.merkez_path).unwrap();
        m.lisans = "askida".into();
        crate::merkez::kaydet(&t.p.merkez_path, &m).unwrap();
        let pg = t.p.home(IP, Some(MAC), "");
        assert_eq!(pg.tpl, "hata");
        let html = t.p.render(&pg);
        assert!(html.contains("internet hizmeti şu an kapalı") && html.contains(UYARI) && html.contains(SERVIS_LINKI), "{html}");
        // Servis sayfasının kendisinde genel uyarı yok (kendi durum satırı var)
        let html = t.servis_html(false);
        assert!(!html.contains(UYARI) && html.contains("Durum: Modemden adres alınamadı"), "{html}");
    }

    #[test]
    fn uyari_kendiliginden_kalkar() {
        let t = setup(|_| {});
        t.internet_yok(Some(Durum::KabloYok));
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains("Kablo takılı değil"));
        let n = t.yoklama.load(Ordering::SeqCst);
        // internet geldi: durum en çok 10 sn önbellekte; o sürede sağlayıcı yeniden çağrılmaz
        t.internet_yok(None);
        t.advance(9.0);
        assert!(t.p.render(&t.p.home(IP, Some(MAC), "")).contains(UYARI));
        assert!(t.p.servis_sayfasi(false).is_some());
        assert_eq!(t.yoklama.load(Ordering::SeqCst), n, "önbellekteyken yeniden yoklandı");
        t.advance(1.0);
        let html = t.p.render(&t.p.home(IP, Some(MAC), ""));
        assert!(!html.contains(UYARI) && !html.contains(SERVIS_LINKI), "{html}");
        assert_eq!(t.yoklama.load(Ordering::SeqCst), n + 1);
        // Servis de kapandı
        assert!(t.p.servis_sayfasi(false).is_none() && t.p.servis_sayfasi(true).is_none());
    }

    #[test]
    fn servis_internet_varken_yonlendirir() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(eski_pppoe());
        // internet var: GET (form ve izleme) ve POST giriş sayfasına yönlenir (HTTP katmanı 302 → /); form işlenmez
        assert!(t.p.servis_sayfasi(false).is_none() && t.p.servis_sayfasi(true).is_none());
        assert!(t.p.servis_kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).is_none());
        for _ in 0..25 {
            assert!(t.p.servis_kaydet(IP, Some(MAC), &servis_form("yanlis-parola", &YENI)).is_none());
        }
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        assert!(!t.audit_text().contains("SERVIS_"));
        // internet gidince: varken gelen yanlış denemeler sayılmamış (ne MAC ne cihaz kilidi), doğru parola uygular
        t.internet_yok(Some(Durum::SunucuYok));
        t.advance(10.0);
        assert!(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
        assert_eq!(t.systemd_run().len(), 1);
    }

    /// Gerçek HTTP katmanı (`handle`) üzerinden tek istek: portal 127.0.0.1'de dinler, istemci aynı makineden bağlanır.
    fn http_istek(t: &T, server: &tiny_http::Server, istek: &str) -> String {
        use std::io::Write;
        let port = server.server_addr().to_ip().unwrap().port();
        let istek = istek.replace("{host}", &format!("127.0.0.1:{port}"));
        let istemci = std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(istek.as_bytes()).unwrap();
            let mut cevap = String::new();
            let _ = s.read_to_string(&mut cevap);
            cevap
        });
        handle(&t.p, &RateLimiter::default(), server.recv().unwrap());
        istemci.join().unwrap()
    }

    #[test]
    fn servis_http_yonlendirme() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let mut t = setup(|c| {
            c.main.router_ip = "127.0.0.1".into();
            c.main.portal_port = port;
            c.main.subnet = "127.0.0.0/8".into();
        });
        // istemcinin MAC'i kira dosyasından (127.0.0.1 → MAC)
        let kiralar = t.root.join("dnsmasq.leases");
        std::fs::write(&kiralar, format!("4102444800 {MAC} 127.0.0.1 telefon *\n")).unwrap();
        t.p.cfg.main.leases_file = kiralar.to_string_lossy().into();
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(eski_pppoe());
        let get = |yol: &str| format!("GET {yol} HTTP/1.1\r\nHost: {{host}}\r\nConnection: close\r\n\r\n");
        let govde = format!("servis_parola={AP_PAROLA}&pppoe_acik=acik&pppoe_kullanici=yeni%40isp&pppoe_parola=yeni-parola-35&pppoe_vlan=35");
        let post = format!(
            "POST /servis HTTP/1.1\r\nHost: {{host}}\r\nConnection: close\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{govde}",
            govde.len()
        );
        // internet var: /servis (GET, izleme, POST) giriş sayfasına yönlenir; form işlenmez, giriş sayfası bugünkü gibi
        let ana = format!("Location: http://127.0.0.1:{port}/\r\n");
        for istek in [get("/servis"), get("/servis?izle=1"), post.clone()] {
            let c = http_istek(&t, &server, &istek);
            assert!(c.starts_with("HTTP/1.1 302") && c.contains(&ana), "{c}");
        }
        let c = http_istek(&t, &server, &get("/"));
        assert!(c.starts_with("HTTP/1.1 200") && !c.contains(UYARI) && !c.contains(SERVIS_LINKI), "{c}");
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists() && !t.audit_text().contains("SERVIS_"));
        // internet yok: giriş sayfasında uyarı + Servis bağlantısı; /servis formu verir; POST uygular
        t.internet_yok(Some(Durum::KimlikHatasi));
        t.advance(10.0);
        let c = http_istek(&t, &server, &get("/"));
        assert!(c.starts_with("HTTP/1.1 200") && c.contains(UYARI) && c.contains(SERVIS_LINKI), "{c}");
        let c = http_istek(&t, &server, &get("/servis"));
        assert!(c.starts_with("HTTP/1.1 200") && c.contains("Kullanıcı adı ya da parola reddedildi") && c.contains("name=\"servis_parola\""), "{c}");
        assert!(!c.contains("eski@isp"), "kayıtlı kullanıcı adı parola doğrulanmadan gösterilmez: {c}");
        let c = http_istek(&t, &server, &post);
        assert!(c.starts_with("HTTP/1.1 200") && c.contains(KAYDEDILDI) && !c.contains("yeni-parola-35") && !c.contains(AP_PAROLA), "{c}");
        assert_eq!(t.systemd_run().len(), 1);
        let h = t.hazir();
        assert!(h.acik && h.vlan == 35 && h.kullanici == "yeni@isp" && h.parola == "yeni-parola-35");
        assert!(t.audit_text().lines().any(|l| l.contains(";SERVIS_PPPOE;") && l.contains(MAC) && l.contains("127.0.0.1")));
    }

    #[test]
    fn servis_parolasi_yoksa_form_yok() {
        let t = setup(|_| {}); // merkez.json'da ap_parola yok (eski merkez ya da parola henüz gelmedi)
        t.ag_kaydet(eski_pppoe());
        t.internet_yok(Some(Durum::SunucuYok));
        let html = t.servis_html(false);
        assert!(html.contains("Servis parolası tanımlı değil") && !html.contains("<form"), "{html}");
        assert!(html.contains("PPPoE sunucusu yanıt vermiyor"), "durum yine görünür: {html}");
        // gönderilse de hiçbir şey olmaz (boş parola eşleşmez), sayılmaz, yazılmaz
        for parola in ["", "x", "yanlis-parola"] {
            let html = t.kaydet(IP, Some(MAC), &servis_form(parola, &YENI));
            assert!(html.contains("Servis parolası tanımlı değil") && !html.contains("<form"), "{html}");
        }
        // cihaz hiç bağlanmamış (merkez.json yok) ya da merkezin kuralından (8-64 karakter) kısa parola: aynı
        crate::merkez::sil(&t.p.merkez_path);
        assert!(t.servis_html(false).contains("Servis parolası tanımlı değil"));
        let kisa = crate::merkez::Merkez { numara: "4511643".into(), ap_parola: "kisa".into(), ..Default::default() };
        crate::merkez::kaydet(&t.p.merkez_path, &kisa).unwrap();
        assert!(!t.servis_html(false).contains("<form"));
        assert!(t.kaydet(IP, Some(MAC), &servis_form("kisa", &YENI)).contains("Servis parolası tanımlı değil"));
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        assert!(!t.audit_text().contains("SERVIS_"));
    }

    #[test]
    fn servis_yanlis_parola_kilit() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(eski_pppoe());
        t.internet_yok(Some(Durum::KimlikHatasi));
        const KILIT: &str = "Çok fazla yanlış deneme; 15 dakika sonra yeniden deneyin.";
        // aynı MAC'ten 5 yanlış: 5.'si kilidi başlatır; form parolalar boş olarak yeniden gelir
        for i in 1..=5 {
            let html = t.kaydet(IP, Some(MAC), &servis_form("yanlis-parola", &YENI));
            assert!(html.contains(if i < 5 { "Servis parolası yanlış." } else { KILIT }), "{i}: {html}");
            assert!(html.contains("<form") && !html.contains("yeni-parola-35") && !html.contains("yanlis-parola"), "{i}: {html}");
        }
        // 6. deneme doğru parolayla da kilitli (parolaya bakılmaz); aynı MAC başka IP'den gelse de
        assert!(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        assert!(t.kaydet("10.50.0.77", Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        let a = t.audit_text();
        let giris: Vec<&str> = a.lines().filter(|l| l.contains(";SERVIS_GIRIS;")).collect();
        assert_eq!(giris.iter().filter(|l| l.ends_with(";sonuc=hatali")).count(), 5, "{a}");
        // kilit dönemi başına tek satır (aynı MAC'in ikinci kilitli isteği yazılmaz)
        assert_eq!(giris.iter().filter(|l| l.ends_with(";sonuc=kilit")).count(), 1, "{a}");
        assert!(giris.iter().all(|l| l.contains(MAC)) && giris.iter().filter(|l| l.contains(IP)).count() == 6, "{a}");
        for gizli in ["yanlis-parola", AP_PAROLA, "yeni-parola-35", "yeni@isp", "eski-parola-77"] {
            assert!(!a.contains(gizli), "{gizli}: {a}");
        }
        // başka cihaz kilitli değil (MAC başına); MAC'i bilinmeyen istemci IP'siyle sayılır
        assert!(t.kaydet("10.50.0.24", Some("aa:bb:cc:dd:ee:02"), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
        for _ in 0..5 {
            t.kaydet("10.50.0.30", None, &servis_form("yanlis-parola", &YENI));
        }
        assert!(t.kaydet("10.50.0.30", None, &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        assert!(t.kaydet("10.50.0.30", Some("aa:bb:cc:dd:ee:03"), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
        // 15 dk sonra açılır
        t.advance(899.0);
        assert!(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        t.advance(1.0);
        assert!(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
        assert_eq!(t.systemd_run().len(), 3);
    }

    #[test]
    fn servis_cihaz_geneli_kilit() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(eski_pppoe());
        t.internet_yok(Some(Durum::SunucuYok));
        const KILIT: &str = "Çok fazla yanlış deneme; 60 dakika sonra yeniden deneyin.";
        let cihaz = |i: usize| (format!("10.50.0.{}", 100 + i), format!("aa:bb:cc:dd:f1:{i:02}"));
        // MAC değiştirerek 20 yanlış (her MAC'ten bir: MAC başına sınıra takılmaz); 20.'si cihaz kilidini başlatır
        for i in 0..20 {
            let (ip, mac) = cihaz(i);
            let html = t.kaydet(&ip, Some(&mac), &servis_form("yanlis-parola", &YENI));
            assert!(html.contains(if i < 19 { "Servis parolası yanlış." } else { KILIT }), "{i}: {html}");
        }
        // 21. deneme her MAC'te (eskisi ya da yenisi; MAC'i bilinmeyen de) kilitli, doğru parolayla da
        for i in [0, 19, 20, 21] {
            let (ip, mac) = cihaz(i);
            assert!(t.kaydet(&ip, Some(&mac), &servis_form(AP_PAROLA, &YENI)).contains(KILIT), "{i}");
        }
        assert!(t.kaydet("10.50.0.250", None, &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        let a = t.audit_text();
        // cihaz geneli kilit: farklı MAC'lerden 5 kilitli istek, kilit dönemi başına tek satır
        assert_eq!((a.matches("sonuc=hatali").count(), a.matches("sonuc=kilit").count()), (20, 1), "{a}");
        // 60 dk sonra açılır
        let (ip, mac) = cihaz(22);
        t.advance(3599.0);
        assert!(t.kaydet(&ip, Some(&mac), &servis_form(AP_PAROLA, &YENI)).contains(KILIT));
        t.advance(1.0);
        assert!(t.kaydet(&ip, Some(&mac), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
    }

    #[test]
    fn servis_dogru_parola_uygular() {
        use std::os::unix::fs::PermissionsExt;
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(crate::ag::Pppoe { acik: false, ..eski_pppoe() });
        t.internet_yok(Some(Durum::DhcpYok));
        let f = servis_form(AP_PAROLA, &[("pppoe_acik", "1"), ("pppoe_kullanici", " 5321234567@turk.net "), ("pppoe_parola", "yeni-parola-35"), ("pppoe_vlan", "35")]);
        let pg = t.p.servis_kaydet(IP, Some(MAC), &f).expect("internet yok: sayfa");
        // hazırlık dosyası 600, `[pppoe]` tablosu: ctl pppoe-servis'in okuduğu biçim (kullanıcı adı kırpılmış)
        let dosya = t.servis_dosyasi();
        assert_eq!(std::fs::metadata(&dosya).unwrap().permissions().mode() & 0o777, 0o600);
        let h = t.hazir();
        assert!(h.acik && h.kullanici == "5321234567@turk.net" && h.parola == "yeni-parola-35" && h.vlan == 35);
        assert!(h.oncelik == 0 && h.mtu == 1492 && h.kimlik == "otomatik" && h.ac_adi.is_empty() && h.servis_adi.is_empty());
        // ctl pppoe-servis ayrı bir işte (portal /etc'ye yazamaz); geri alma zamanlayıcısı yok
        let beklenen: Vec<String> = ["systemd-run", "--collect", "--unit", "wfc-pppoe-servis", "/usr/local/bin/wificorrect", "ctl", "pppoe-servis"]
            .iter()
            .map(|s| s.to_string())
            .chain([dosya.to_string_lossy().to_string()])
            .collect();
        assert_eq!(t.systemd_run(), vec![beklenen]);
        // denetimde yalnızca özet (MAC, IP); kullanıcı adı ve parola yok
        let a = t.audit_text();
        let satir = a.lines().find(|l| l.contains(";SERVIS_PPPOE;")).unwrap_or_else(|| panic!("{a}"));
        assert!(satir.contains(MAC) && satir.contains(IP) && satir.ends_with(";pppoe=acik vlan=35 kullanici_degisti=1 parola_degisti=1"), "{satir}");
        // cevap: 5 sn'de bir yenilenen durum sayfası; parolalar ve kullanıcı adı cevapta ve denetimde yok
        let html = t.p.render(&pg);
        assert!(html.contains(KAYDEDILDI) && !html.contains("<form"), "{html}");
        assert!(html.contains(&format!("<meta http-equiv=\"refresh\" content=\"5;url={}/servis?izle=1\">", t.p.base_url)), "{html}");
        for gizli in ["yeni-parola-35", "eski-parola-77", AP_PAROLA, "5321234567"] {
            assert!(!html.contains(gizli) && !a.contains(gizli), "{gizli}");
        }
        // boş PPPoE parolası = kayıtlı parola kalır: kayıtlı parola dosyaya kopyalanmaz (ctl birleştirir), parola_degisti=0
        t.ag_kaydet(eski_pppoe());
        let f = servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "eski@isp"), ("pppoe_parola", ""), ("pppoe_vlan", "7")]);
        assert!(t.kaydet(IP, Some(MAC), &f).contains(KAYDEDILDI));
        let h = t.hazir();
        assert!(h.parola.is_empty() && h.kullanici == "eski@isp" && h.vlan == 7);
        let a = t.audit_text();
        let son = a.lines().filter(|l| l.contains(";SERVIS_PPPOE;")).last().unwrap();
        assert!(son.ends_with(";pppoe=acik vlan=7 kullanici_degisti=0 parola_degisti=0"), "{son}");
        assert!(!a.contains("eski-parola-77"));
        // PPPoE'yi kapatma da Servis'ten (kullanıcı adı korunur)
        let f = servis_form(AP_PAROLA, &[("pppoe_acik", "kapali"), ("pppoe_kullanici", "eski@isp")]);
        assert!(t.kaydet(IP, Some(MAC), &f).contains(KAYDEDILDI));
        assert!(!t.hazir().acik && t.hazir().kullanici == "eski@isp");
        assert!(t.audit_text().lines().filter(|l| l.contains(";SERVIS_PPPOE;")).last().unwrap().ends_with(";pppoe=kapali vlan=0 kullanici_degisti=0 parola_degisti=0"));
    }

    #[test]
    fn servis_izleme_sayfasi() {
        let t = setup(|_| {});
        t.internet_yok(Some(Durum::Baglaniyor));
        // izleme: durum, 5 sn'de bir kendini yeniler, form yok (yalnızca durum: parola tanımlı olmasa da açılır)
        let html = t.servis_html(true);
        assert!(html.contains("Durum: Bağlanıyor\u{2026}") && !html.contains("<form"), "{html}");
        assert!(html.contains(&format!("<meta http-equiv=\"refresh\" content=\"5;url={}/servis?izle=1\">", t.p.base_url)), "{html}");
        assert!(html.contains(&format!("href=\"{}/servis\"", t.p.base_url)), "ayarları değiştir bağlantısı: {html}");
    }

    #[test]
    fn servis_formu_kayitli_ayarla_dolu() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        let kayitli = crate::ag::Pppoe { vlan: 35, mtu: 1400, ..eski_pppoe() };
        t.ag_kaydet(kayitli.clone());
        t.internet_yok(Some(Durum::KimlikHatasi));
        let pg = t.p.servis_sayfasi(false).unwrap();
        assert_eq!(pg.dil, Dil::Tr);
        let html = t.p.render(&pg);
        // tek form: servis parolası + panelle aynı PPPoE alanları (kayıtlı değerlerle; kayıtlı kullanıcı adı hariç) + Kaydet ve bağlan
        assert_eq!(html.matches("<form").count(), 1, "{html}");
        assert!(html.contains(&format!("<form class=\"servis\" method=\"post\" action=\"{}/servis\">", t.p.base_url)), "{html}");
        assert!(html.contains(&crate::pppoe::servis_form_html(&kayitli, "")) && !html.contains("eski@isp"), "{html}");
        for ad in ["servis_parola", "pppoe_acik", "pppoe_kullanici", "pppoe_parola", "pppoe_vlan", "pppoe_oncelik", "pppoe_mtu", "pppoe_kimlik", "pppoe_ac_adi", "pppoe_servis_adi"] {
            assert_eq!(html.matches(&format!("name=\"{ad}\"")).count(), 1, "{ad}");
        }
        assert!(html.contains("<input type=\"password\" id=\"servis_parola\" name=\"servis_parola\" value=\"\"") && html.contains(">Kaydet ve bağlan</button>"), "{html}");
        // parolalar (PPPoE ve servis) sayfada yok
        assert!(!html.contains("eski-parola-77") && !html.contains(AP_PAROLA), "{html}");
        // durum satırı; yalnızca Türkçe (dil seçimi yok), JS yok, form sayfası kendini yenilemez
        assert!(html.contains("Durum: Kullanıcı adı ya da parola reddedildi"), "{html}");
        assert!(!html.contains("<nav class=\"dil\"") && !html.contains("<script") && !html.contains("http-equiv=\"refresh\""), "{html}");
    }

    /// PPPoE Açık/Kapalı kutusunda seçili seçenek: `acik` | `kapali` (tam biri seçili değilse test düşer)
    fn secili_pppoe(html: &str) -> &'static str {
        match (html.contains("<option value=\"acik\" selected>"), html.contains("<option value=\"kapali\" selected>")) {
            (true, false) => "acik",
            (false, true) => "kapali",
            _ => panic!("PPPoE kutusunda tek seçenek seçili olmalı: {html}"),
        }
    }

    #[test]
    fn servis_pppoe_secimi_acik_gelir() {
        // Servis'e PPPoE'yi açmak için gelinir: ilk çizimde seçim, kayıtlı ayar kapalı (ya da hiç yok) olsa da Açık gelir; yoksa formu
        // olduğu gibi gönderen teknisyen bilgileri PPPoE kapalıyken kaydeder ve internet gelmez
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.internet_yok(Some(Durum::DhcpYok));
        assert_eq!(secili_pppoe(&t.servis_html(false)), "acik", "ag.toml yok (varsayılan: kapalı)");
        t.ag_kaydet(crate::ag::Pppoe::default());
        assert_eq!(secili_pppoe(&t.servis_html(false)), "acik", "hiç ayarlanmamış cihaz");
        // kapalı kaydedilmiş, bilgileri duran cihaz: seçim Açık; VLAN ve ileri alanlar kayıtlı değerlerle, kullanıcı adı ve parola yok
        t.ag_kaydet(crate::ag::Pppoe { acik: false, vlan: 35, mtu: 1400, ..eski_pppoe() });
        let html = t.servis_html(false);
        assert_eq!(secili_pppoe(&html), "acik", "{html}");
        assert!(html.contains("name=\"pppoe_vlan\" value=\"35\"") && html.contains("name=\"pppoe_mtu\" value=\"1400\""), "{html}");
        assert!(!html.contains("eski@isp") && !html.contains("eski-parola-77") && html.contains("Kayıtlı parola aynı kalır"), "{html}");
        t.ag_kaydet(eski_pppoe());
        assert_eq!(secili_pppoe(&t.servis_html(false)), "acik", "kayıtlı ayar açık");
        // ortak çizim değişmedi: panelin Portlar formu (ve `servis_form_html`'in kendisi) kayıtlı değeri gösterir; Açık'ı Servis seçer
        let varsayilan = crate::ag::Pppoe::default();
        assert_eq!(secili_pppoe(&crate::pppoe::form_html(&varsayilan)), "kapali");
        assert_eq!(secili_pppoe(&crate::pppoe::servis_form_html(&varsayilan, "")), "kapali");
        assert_eq!(secili_pppoe(&crate::pppoe::form_html(&eski_pppoe())), "acik");

        // reddedilen kayıtta gönderilen seçim korunur: Kapalı seçilmişse Kapalı, Açık seçilmişse Açık kalır (kayıtlı ayar kapalı)
        t.ag_kaydet(crate::ag::Pppoe { acik: false, ..eski_pppoe() });
        for (i, secim) in ["kapali", "acik"].into_iter().enumerate() {
            let mac = format!("aa:bb:cc:dd:ee:1{i}");
            let ad = [("pppoe_acik", secim), ("pppoe_kullanici", "k@isp"), ("pppoe_parola", "p")];
            // yanlış Servis parolası
            let html = t.kaydet(IP, Some(&mac), &servis_form("yanlis-parola", &ad));
            assert!(html.contains("Servis parolası yanlış."), "{html}");
            assert_eq!(secili_pppoe(&html), secim, "yanlış parola: {html}");
            // formun altındaki not seçimden bağımsız doğru: Kapalı seçiliyken "PPPoE yeniden bağlanır" demez
            assert!(html.contains("Kaydedince ayar uygulanır (PPPoE Açık ise yeniden bağlanır, Kapalı ise durur)"), "{html}");
            // boş Servis parolası
            let html = t.kaydet(IP, Some(&mac), &servis_form("", &ad));
            assert!(html.contains("Servis parolasını girin."), "{html}");
            assert_eq!(secili_pppoe(&html), secim, "boş parola: {html}");
            // doğru paroladan sonra doğrulama hatası
            let vlan = [("pppoe_acik", secim), ("pppoe_kullanici", "k@isp"), ("pppoe_parola", "p"), ("pppoe_vlan", "4095")];
            let html = t.kaydet(IP, Some(&mac), &servis_form(AP_PAROLA, &vlan));
            assert!(html.contains("VLAN 1-4094 arasında olmalı"), "{html}");
            assert_eq!(secili_pppoe(&html), secim, "doğrulama hatası: {html}");
            assert!(html.contains("name=\"pppoe_kullanici\" value=\"k@isp\"") && !html.contains("eski@isp"), "yazılan ad döner, kayıtlı ad hiç: {html}");
        }
        // kilit: 5 yanlış denemeden sonra (doğru parolayla da) kilit sayfası da gönderilen seçimi korur
        let mac = "aa:bb:cc:dd:ee:20";
        for _ in 0..5 {
            t.kaydet(IP, Some(mac), &servis_form("yanlis-parola", &[("pppoe_acik", "kapali")]));
        }
        let html = t.kaydet(IP, Some(mac), &servis_form(AP_PAROLA, &[("pppoe_acik", "kapali")]));
        assert!(html.contains("15 dakika sonra"), "{html}");
        assert_eq!(secili_pppoe(&html), "kapali", "kilit: {html}");
        // hiçbir şey gönderilmedi (eksik form: pppoe_acik yok): form kayıtlı ayardan kurulur, ilk çizim gibi Açık gelir
        let html = t.kaydet(IP, Some("aa:bb:cc:dd:ee:21"), &servis_form("yanlis-parola", &[("pppoe_kullanici", "k@isp")]));
        assert!(html.contains("Form eksik"), "{html}");
        assert_eq!(secili_pppoe(&html), "acik", "eksik form: {html}");
        assert!(!html.contains("eski@isp") && !html.contains("k@isp"), "eksik formda hiçbir ad gösterilmez: {html}");
        // hiçbiri uygulanmadı
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
    }

    #[test]
    fn servis_kayit_mesaji_acik_kapali() {
        // kayıt mesajı kaydedilen ayarı söyler: Açık → "yeniden bağlanıyor"; Kapalı → "PPPoE kapalı" (bağlanmaz, modemden adres bekler)
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(crate::ag::Pppoe::default());
        t.internet_yok(Some(Durum::SunucuYok));
        const GIZLI: [&str; 6] = ["yeni@isp", "yeni-parola-35", "yeni-parola-99", "eski@isp", "eski-parola-77", AP_PAROLA];
        let html = t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI));
        assert!(html.contains("Ayar kaydedildi; PPPoE yeniden bağlanıyor."), "{html}");
        assert!(html.contains("güncel durumun gelmesi bir dakikayı bulabilir") && !html.contains("PPPoE kapalı") && !html.contains("modemin adres"), "{html}");
        assert!(html.contains("birkaç saniyede bir yenilenip durumu gösterir") && html.contains("http-equiv=\"refresh\"") && !html.contains("<form"), "{html}");
        assert!(t.hazir().acik);
        assert!(GIZLI.iter().all(|g| !html.contains(g)), "{html}");
        // Kapalı kaydedildi (kullanıcı adı ve parola yine yazılmış olabilir): mesaj bunu söyler, "yeniden bağlanıyor" demez
        let f = servis_form(AP_PAROLA, &[("pppoe_acik", "kapali"), ("pppoe_kullanici", "yeni@isp"), ("pppoe_parola", "yeni-parola-99")]);
        let html = t.kaydet(IP, Some(MAC), &f);
        assert!(html.contains("Ayar kaydedildi; PPPoE kapalı.") && html.contains("modemin adres vermesini bekliyor"), "{html}");
        assert!(!html.contains("yeniden bağlanıyor") && !html.contains("bir dakikayı bulabilir"), "{html}");
        // Açık yapmanın yolu söylenir; durum sayfası yine yenilenir, internet gelince giriş sayfası açılır
        assert!(html.contains("PPPoE seçimini Açık yapın") && html.contains("Ayarları değiştir"), "{html}");
        assert!(html.contains("birkaç saniyede bir yenilenip durumu gösterir") && html.contains("internet gelince giriş sayfası açılır"), "{html}");
        assert!(html.contains("http-equiv=\"refresh\"") && !html.contains("<form"), "{html}");
        assert!(GIZLI.iter().all(|g| !html.contains(g)), "{html}");
        // kaydedilen gerçekten kapalı (hazırlık dosyası ve denetim özeti)
        assert!(!t.hazir().acik);
        let a = t.audit_text();
        assert!(a.lines().filter(|l| l.contains(";SERVIS_PPPOE;")).last().unwrap().ends_with(";pppoe=kapali vlan=0 kullanici_degisti=1 parola_degisti=1"), "{a}");
        assert!(GIZLI.iter().all(|g| !a.contains(g)), "{a}");
        // yeniden Açık: bugünkü mesaj geri gelir (test ağında ag.toml'u ctl yazmaz: kayıtlı parola hâlâ boş, bu yüzden parola yazılır)
        let f = servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "yeni@isp"), ("pppoe_parola", "yeni-parola-35")]);
        let html = t.kaydet(IP, Some(MAC), &f);
        assert!(html.contains("Ayar kaydedildi; PPPoE yeniden bağlanıyor.") && !html.contains("PPPoE kapalı"), "{html}");
        assert!(t.hazir().acik);
        assert_eq!(t.systemd_run().len(), 3);
    }

    #[test]
    fn servis_kayitli_kullanici_adini_gostermez() {
        // kayıtlı kullanıcı adı (çoğu kez abonenin telefon numarası) Servis sayfasında hiç görünmez: formda, hata ve kilit
        // cevaplarında, parola doğrulandıktan sonraki hatalarda da yok; boş gönderilirse kayıtlı ad kalır (parola gibi)
        const KAYITLI: &str = "eski@isp";
        const NOT: &str = "Boş bırakılırsa kayıtlı kullanıcı adı aynı kalır.";
        let mut t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(crate::ag::Pppoe { vlan: 35, mtu: 1400, ..eski_pppoe() });
        t.internet_yok(Some(Durum::KimlikHatasi));
        // form: kullanıcı adı boş + not; VLAN ve ileri alanlar kayıtlı değerlerle; durum satırı
        let html = t.servis_html(false);
        assert!(!html.contains(KAYITLI), "{html}");
        assert!(html.contains("name=\"pppoe_kullanici\" value=\"\"") && html.contains(NOT), "{html}");
        assert!(html.contains("name=\"pppoe_vlan\" value=\"35\"") && html.contains("name=\"pppoe_mtu\" value=\"1400\""), "{html}");
        assert!(html.contains("Durum: Kullanıcı adı ya da parola reddedildi"), "{html}");
        // hata cevapları: boş servis parolası, eksik form, doğru paroladan sonra doğrulama hatası, yanlış parola, kilit
        let bos_ad = [("pppoe_acik", "acik"), ("pppoe_kullanici", ""), ("pppoe_vlan", "4095")];
        let mut cevaplar = vec![
            t.kaydet(IP, Some(MAC), &servis_form("", &bos_ad)),
            t.kaydet(IP, Some(MAC), &servis_form("yanlis-parola", &[("pppoe_kullanici", "")])),
            t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &bos_ad)),
        ];
        assert!(cevaplar[2].contains("VLAN 1-4094 arasında olmalı"), "{}", cevaplar[2]);
        for _ in 0..5 {
            cevaplar.push(t.kaydet(IP, Some(MAC), &servis_form("yanlis-parola", &bos_ad)));
        }
        cevaplar.push(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &bos_ad)));
        assert!(cevaplar.last().unwrap().contains("15 dakika sonra"));
        for (i, c) in cevaplar.iter().enumerate() {
            assert!(!c.contains(KAYITLI) && c.contains("<form") && c.contains(NOT), "{i}: {c}");
        }
        // başka cihazdan, doğru parolayla: bekleyen port değişikliği (doğrulamadan sonraki hata) de adı göstermez
        let (ip2, mac2) = ("10.50.0.24", Some("aa:bb:cc:dd:ee:02"));
        std::fs::create_dir_all(&t.p.ag_yollar.durum).unwrap();
        let bekliyor = t.p.ag_yollar.durum.join("ag-bekliyor");
        std::fs::write(&bekliyor, "1790705314 wfc-ag-geri-al-ab12\n").unwrap();
        let c = t.kaydet(ip2, mac2, &servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "")]));
        assert!(c.contains("onay bekliyor") && !c.contains(KAYITLI), "{c}");
        std::fs::remove_file(&bekliyor).unwrap();
        // boş (ya da boşluk) kullanıcı adı + doğru parola: kayıtlı ad hazırlık dosyasına girer; kullanici_degisti=0
        let c = t.kaydet(ip2, mac2, &servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", " "), ("pppoe_vlan", "7")]));
        assert!(c.contains(KAYDEDILDI) && !c.contains(KAYITLI), "{c}");
        let h = t.hazir();
        assert!(h.acik && h.kullanici == KAYITLI && h.parola.is_empty() && h.vlan == 7);
        let son = |t: &T| t.audit_text().lines().filter(|l| l.contains(";SERVIS_PPPOE;")).last().unwrap().to_string();
        assert!(son(&t).ends_with(";pppoe=acik vlan=7 kullanici_degisti=0 parola_degisti=0"), "{}", son(&t));
        // kapatırken de kayıtlı ad kalır (açık/kapalı geçişi kullanıcı adını silmez)
        assert!(t.kaydet(ip2, mac2, &servis_form(AP_PAROLA, &[("pppoe_acik", "kapali")])).contains(KAYDEDILDI));
        assert!(!t.hazir().acik && t.hazir().kullanici == KAYITLI);
        // yazılan ad kayıtlıyı değiştirir
        assert!(t.kaydet(ip2, mac2, &servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "yeni@isp")])).contains(KAYDEDILDI));
        assert!(t.hazir().kullanici == "yeni@isp" && son(&t).contains("kullanici_degisti=1"));
        // iş başlatılamadı (doğrulamadan sonraki hata): ad yine gösterilmez; denetimde hiçbir yerde yok
        t.p.runner = Box::new(|c: &[String]| c[0] != "systemd-run");
        let c = t.kaydet(ip2, mac2, &servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "")]));
        assert!(c.contains("başlatılamadı") && !c.contains(KAYITLI), "{c}");
        assert!(!t.audit_text().contains(KAYITLI));
    }

    #[test]
    fn servis_kilit_denetime_donem_basina_bir_kez() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(eski_pppoe());
        t.internet_yok(Some(Durum::SunucuYok));
        let satir = |t: &T, sonuc: &str| t.audit_text().lines().filter(|l| l.contains(";SERVIS_GIRIS;") && l.ends_with(sonuc)).count();
        let yanlis = servis_form("yanlis-parola", &YENI);
        for _ in 0..5 {
            t.kaydet(IP, Some(MAC), &yanlis);
        }
        // kilit sürerken gelen istekler (doğru, yanlış, eksik form; başka IP'den aynı MAC): tek satır; sayma ve kilit davranışı aynı
        for (ip, f) in [(IP, servis_form(AP_PAROLA, &YENI)), (IP, yanlis.clone()), (IP, servis_form("", &[])), ("10.50.0.77", servis_form(AP_PAROLA, &YENI))] {
            assert!(t.kaydet(ip, Some(MAC), &f).contains("15 dakika sonra"));
        }
        assert_eq!((satir(&t, ";sonuc=hatali"), satir(&t, ";sonuc=kilit")), (5, 1));
        // yeni kilit dönemi: yeniden bir satır
        t.advance(900.0);
        for _ in 0..5 {
            t.kaydet(IP, Some(MAC), &yanlis);
        }
        for _ in 0..3 {
            assert!(t.kaydet(IP, Some(MAC), &yanlis).contains("15 dakika sonra"));
        }
        assert_eq!((satir(&t, ";sonuc=hatali"), satir(&t, ";sonuc=kilit")), (10, 2));
        // cihaz geneli kilit (60 dk'da 20. yanlış): farklı MAC'lerden gelen kilitli istekler de dönem başına tek satır
        for i in 0..10 {
            t.kaydet(&format!("10.50.0.{}", 100 + i), Some(&format!("aa:bb:cc:dd:f2:{i:02}")), &yanlis);
        }
        for i in 10..14 {
            assert!(t.kaydet(&format!("10.50.0.{}", 100 + i), Some(&format!("aa:bb:cc:dd:f2:{i:02}")), &servis_form(AP_PAROLA, &YENI)).contains("60 dakika sonra"));
        }
        assert_eq!((satir(&t, ";sonuc=hatali"), satir(&t, ";sonuc=kilit")), (20, 3));
        assert!(t.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn servis_gecersiz_ayar() {
        let t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.ag_kaydet(crate::ag::Pppoe::default()); // kayıtlı bilgi yok
        t.internet_yok(Some(Durum::KabloYok));
        // doğru parola + açık + boş kullanıcı: doğrulama hatası; dosya yok, iş yok, deneme sayılmaz (MAC ve cihaz sınırını aşacak kadar)
        let bos = servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", " "), ("pppoe_parola", "p"), ("pppoe_vlan", "35")]);
        for _ in 0..21 {
            let html = t.kaydet(IP, Some(MAC), &bos);
            assert!(html.contains("PPPoE kullanıcı adı ve parolası boş olamaz.") && html.contains("<form"), "{html}");
            assert!(html.contains("name=\"pppoe_vlan\" value=\"35\""), "girilen değerler forma geri gelir: {html}");
        }
        // biçimsiz değerler de: VLAN aralık dışı, pppd dosyasına satır sokan kullanıcı adı
        let vlan = servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "k@isp"), ("pppoe_parola", "p"), ("pppoe_vlan", "4095")]);
        assert!(t.kaydet(IP, Some(MAC), &vlan).contains("VLAN 1-4094 arasında olmalı"));
        let satir = servis_form(AP_PAROLA, &[("pppoe_acik", "acik"), ("pppoe_kullanici", "k\nconnect /bin/sh"), ("pppoe_parola", "p")]);
        assert!(t.kaydet(IP, Some(MAC), &satir).contains("tırnak, ters eğik çizgi ve kontrol karakteri içeremez"));
        // eksik form (pppoe_acik yok): ayar formdan okunmaz, parola denenmez, sayılmaz
        for _ in 0..21 {
            let html = t.kaydet(IP, Some(MAC), &servis_form("yanlis-parola", &[("pppoe_kullanici", "k@isp")]));
            assert!(html.contains("Form eksik") && !html.contains("Servis parolası yanlış."), "{html}");
        }
        // boş servis parolası: sorulur, sayılmaz
        for _ in 0..21 {
            assert!(t.kaydet(IP, Some(MAC), &servis_form("", &YENI)).contains("Servis parolasını girin."));
        }
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        assert!(!t.audit_text().contains("SERVIS_"));
        // hâlâ kilit yok: geçerli ayar uygulanır
        assert!(t.kaydet(IP, Some(MAC), &servis_form(AP_PAROLA, &YENI)).contains(KAYDEDILDI));
        assert_eq!(t.systemd_run().len(), 1);
    }

    #[test]
    fn servis_uygulanamayan_durumlar() {
        let mut t = setup(|_| {});
        t.ap_parola(AP_PAROLA);
        t.internet_yok(Some(Durum::SunucuYok));
        let f = servis_form(AP_PAROLA, &YENI);
        // ag.toml yok: hangi portun internet aldığı bilinmeden PPPoE konmaz
        assert!(t.kaydet(IP, Some(MAC), &f).contains("Ağ ayar dosyası yok"));
        // ag.toml var ama bozuk (ayrıştırılamıyor): ctl de reddederdi (varsayılan portları yazmaz), sayfa hemen söyler; form geri gelir
        std::fs::create_dir_all(t.p.ag_yollar.ag.parent().unwrap()).unwrap();
        std::fs::write(&t.p.ag_yollar.ag, "wan = \"enp1s0\"\nlan = [\"enp3s0\"\n").unwrap();
        let html = t.kaydet(IP, Some(MAC), &f);
        assert!(html.contains("Ağ ayar dosyası okunamadı (bozuk); PPPoE ayarı uygulanamaz.") && html.contains("<form"), "{html}");
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        // panelden yapılmış, onay bekleyen port değişikliği (en çok 3 dk): ctl de reddederdi, sayfa hemen söyler
        t.ag_kaydet(eski_pppoe());
        std::fs::create_dir_all(&t.p.ag_yollar.durum).unwrap();
        let bekliyor = t.p.ag_yollar.durum.join("ag-bekliyor");
        std::fs::write(&bekliyor, "1790705314 wfc-ag-geri-al-ab12\n").unwrap();
        assert!(t.kaydet(IP, Some(MAC), &f).contains("onay bekliyor"));
        std::fs::remove_file(&bekliyor).unwrap();
        assert!(t.calls.lock().unwrap().is_empty() && !t.servis_dosyasi().exists());
        // iş başlatılamadı: hazırlık dosyası (parola içerir) silinir, denetimde hata
        t.p.runner = Box::new(|c: &[String]| c[0] != "systemd-run");
        let html = t.kaydet(IP, Some(MAC), &f);
        assert!(html.contains("başlatılamadı") && html.contains("<form"), "{html}");
        assert!(!t.servis_dosyasi().exists());
        let a = t.audit_text();
        assert!(a.lines().any(|l| l.contains(";SERVIS_PPPOE;") && l.ends_with(" hata=baslatilamadi")), "{a}");
        assert!(!a.contains("yeni-parola-35"));
    }

    #[test]
    fn servis_parolasi_sabit_surede() {
        assert!(servis_parolasi_dogru(AP_PAROLA, AP_PAROLA));
        for yanlis in ["", "Kp7mQx2Rtz4", "Kp7mQx2Rtz4WW", "kp7mQx2Rtz4W", " Kp7mQx2Rtz4W"] {
            assert!(!servis_parolasi_dogru(yanlis, AP_PAROLA), "{yanlis:?}");
        }
    }

    #[test]
    fn internet_cozumu() {
        // varsayılan rota yoksa internet yok; neden internet alan portun durumu
        assert_eq!(internet_coz(Some(""), || Durum::KabloYok), Some(Durum::KabloYok));
        assert_eq!(internet_coz(Some("blackhole default metric 1\n"), || Durum::DhcpYok), Some(Durum::DhcpYok));
        // rota varsa (DHCP ya da PPPoE) internet var; durum hiç okunmaz
        for rota in ["default via 192.168.1.1 dev enp3s0 proto dhcp src 192.168.1.110 metric 1002 \n", "default dev wfcppp scope link metric 100 \n"] {
            assert_eq!(internet_coz(Some(rota), || panic!("durum okunmamalı")), None, "{rota}");
        }
        // `ip` çalışmadı / zaman aşımı / hata koduyla bitti: internet var sayılır (emin olunamayınca misafire açık sayfada Servis açılmaz)
        assert_eq!(internet_coz(None, || panic!("durum okunmamalı")), None);
        // dhcpcd'nin IPv4LL rotası (PPPoE bağlanamamış ONT hattı) internet değildir: uyarı ve Servis görünür
        for rota in [
            "default dev enp3s0 proto dhcp scope link src 169.254.37.120 metric 1001003 \n",
            "default dev enp3s0 scope link src 169.254.37.120 metric 1001003\n",
        ] {
            assert_eq!(internet_coz(Some(rota), || Durum::SunucuYok), Some(Durum::SunucuYok), "{rota}");
        }
        // IPv4LL + PPPoE: internet var
        let ikisi = "default dev wfcppp scope link metric 100 \ndefault dev enp3s0 proto dhcp scope link src 169.254.37.120 metric 1001003 \n";
        assert_eq!(internet_coz(Some(ikisi), || panic!("durum okunmamalı")), None);
    }

    #[test]
    fn internet_yoklama_ip_hata_kodunda_internet_var_sayilir() {
        // gerçek süreçlerle, sistemdeki okuyucuyla (`rota_oku_sistem`; `ip` yerine betik): `ip` çalıştı ama hata koduyla bitti (ör.
        // netlink hatası; boş çıktı) → sonuç yok, internet var sayılır: uyarı yok, `/servis` giriş sayfasına yönlenir
        let ag_toml = Path::new("/yok/wfc-test/ag.toml"); // yok: varsayılan ayar (PPPoE kapalı, internet portu enp3s0)
        let sys = std::env::temp_dir().join(format!("wfc-portal-sys-{}", std::process::id())); // boş: kablo yok
        let _ = std::fs::remove_dir_all(&sys);
        std::fs::create_dir_all(&sys).unwrap();
        let durum_komutu = |c: &[&str]| -> Option<String> { panic!("kablo yokken komut çalışmamalı: {c:?}") };
        let yokla = |betik: &'static str| {
            let rota_oku = move |c: &[&str]| {
                assert_eq!(c, ["ip", "-o", "route", "show", "default"]);
                rota_oku_sistem(&["sh", "-c", betik])
            };
            internet_yokla(ag_toml, &sys, &rota_oku, &durum_komutu)
        };
        assert_eq!(yokla("exit 2"), None);
        assert_eq!(yokla("echo; exit 1"), None);
        assert_eq!(yokla("echo 'default dev wfcppp scope link metric 100'; exit 1"), None);
        // kod 0: boş çıktı gerçekten "rota yok" (neden: internet portunun durumu); yalnız IPv4LL rotası da
        assert_eq!(yokla("true"), Some(Durum::KabloYok));
        assert_eq!(yokla("echo 'default dev enp3s0 proto dhcp scope link src 169.254.37.120 metric 1001003 '"), Some(Durum::KabloYok));
        assert_eq!(yokla("echo 'default dev wfcppp scope link metric 100 '"), None);
        // karşılaştırma: çıkış koduna bakmayan okuma (`capture_timeout`) hata kodunu "rota yok" sayardı (internet varken Servis açılırdı)
        let eski = |_: &[&str]| ortak::capture_timeout(&["sh", "-c", "exit 2"], 5);
        assert_eq!(internet_yokla(ag_toml, &sys, &eski, &durum_komutu), Some(Durum::KabloYok));
        let _ = std::fs::remove_dir_all(&sys);
    }
}
