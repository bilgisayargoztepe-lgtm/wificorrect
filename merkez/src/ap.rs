//! ap.wificorrect.com (Python ap.py, spec 2026-10-16 "AP arayüzüne panelden tek tıkla erişim", Mimari 2–3): müşterinin taktığı
//! AP'nin ya da modemin kendi web arayüzü cihaz üzerinden (tünel → cihaz → yan oda ya da misafir ağı) bu alan adının kökünde
//! açılır; merkez yalnızca kimliği doğrular. Akış: cihaz.wificorrect.com/_ap-gecis (cihaz.rs) cihaz paneli oturumundan (wcc)
//! 60 sn'lik tek kullanımlık belirteç üretir → `/_ap-giris` belirteci wca çerezine çevirir → Caddy her istekte forward_auth ile
//! `/ap-yetki`'ye sorar → `/_ap-cikis` çerezi siler. wca en çok 30 dk ve yalnızca altındaki wcc geçerli kaldıkça (parola,
//! lisans, üyelik, aynı cihaz ve tünel adresi) geçerlidir; tünel adresi her istekte wcc'nin müşterisinden bulunur: bir
//! müşterinin wca'sı başka müşterinin cihazına gidemez.
//! Bütün müşterilerin AP'leri tek kökte açıldığı için (final review 2026-10-16): Service Worker betiği hiç aktarılmaz (Caddy ap
//! bloğu ve cihaz 403); `/_ap-giris` (yeni AP oturumu) kökün depolamasını (Service Worker kayıtları dahil) siler ve önceki
//! AP'lerin bu kökte kalan çerezlerini sona erdirir, `/_ap-cikis` çerezler dahil her şeyi siler; belirteç yalnızca cihaz
//! alanından yönlenen üst düzey gezinmeyle ve `/_ap-gecis`'i isteyen adresin ağından (IPv4 /24, IPv6 /64) kullanılır; wca
//! SameSite=Strict; AP kipinde kullanıcı X-WFC-AP-Kullanici başlığındadır (0.4.15 öncesi cihaz bunu tanımaz, isteği oturumsuz
//! sayar; X-WFC-Kullanici'yi görse paneli bu kökte açardı).
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use serde_json::Value;

use crate::cihaz::{Cihaz, CihazOturumu};
use crate::guvenlik::Oturumlar;
use crate::merkez::Uygulama;
use crate::ortak::{py_repr, py_strip};
use crate::veri::{Saat, Sonuc};
use crate::web::{self, cerez_degeri, e, guvenli_donus, hata_yaniti, kart, sorgu_ilk, Basliklar, Istek, Yanit};

pub const CEREZ: &str = "wca";
/// En çok 30 dk (yan odayla aynı); boşta ve toplam süre aynı: mutlak.
pub const SURE: f64 = 1800.0;
pub const GIRIS: &str = "https://ap.wificorrect.com/_ap-giris?t=";
pub const CIHAZ_PANELI: &str = "https://cihaz.wificorrect.com";
pub const YENIDEN: &str = "Cihaz panelinde Ayarlar → AP kurulumu → AP arayüzüne git ile yeniden açın.";
/// Fetch Metadata: `/_ap-gecis` yalnızca cihaz.wificorrect.com sayfasından (ya da elle yazılan adresle), `/_ap-giris` ondan
/// yönlenen gezinmeyle (aynı site). AP arayüzü panel ve cihazla aynı "site"da çalışır: kötü niyetli bir AP'nin betiği
/// çerçeve/fetch/pencereyle tarayıcıdaki başka müşterinin cihaz paneli oturumundan (wcc) AP oturumu aldıramasın.
pub const GECIS_KAYNAKLARI: [&str; 2] = ["same-origin", "none"];
/// `/_ap-giris`'te same-origin = AP sayfasının kendisi gezindi (bir müşterinin AP'si kendi belirtecini tarayıcıya ekip başka
/// müşterinin açık AP sekmesini kendine çevirirdi), none = elle yazılan adres (tek kullanımlık belirteç elle yazılmaz).
pub const GIRIS_KAYNAKLARI: [&str; 1] = ["same-site"];
/// `/_ap-cikis` her şeyi siler: başka sitenin (ya da çerçeve/resmin) tetiklediği çıkış oturum kapatma CSRF'i olmasın.
pub const CIKIS_KAYNAKLARI: [&str; 3] = ["same-origin", "same-site", "none"];
/// `/_ap-gecis` reddinin metni.
pub fn elle() -> String {
    format!("AP arayüzü yalnızca cihaz panelinden açılabilir. {YENIDEN}")
}
/// `/_ap-giris`'e gelen adres `/_ap-gecis`'i isteyen adresle aynı ağda olmalı (re-review I-1): cihaz.* ve ap.* ayrı
/// sertifikalı, ayrı bağlantılar; aynı ağdan başka çıkış adresi (taşıyıcı NAT havuzu, çift hatlı modem) geçer, tarayıcıdan
/// sızdırılan belirteç başka ağdan kullanılamaz.
pub const AG_ONEKI_V4: u32 = 24;
pub const AG_ONEKI_V6: u32 = 64;
/// Her bağlantısı ayrı adresten çıkan istemci (iCloud Özel Geçiş, VPN): yeniden denemek aynı yerde takılır, sebebi söylenir.
pub const FARKLI_AG: &str = "Bağlantınız farklı bir internet adresinden geldi (ör. iCloud Özel Geçiş, VPN). Özel Geçiş/VPN'i kapatıp \
cihaz panelinden yeniden deneyin. iPhone'da: Ayarlar → adınız → iCloud → Özel Geçiş.";
/// Yeni AP oturumu: kökün depolaması (localStorage, IndexedDB, Cache API, Service Worker kayıtları) silinir. "cookies" yok:
/// kayıtlı alan adının (wificorrect.com) bütün çerezlerini — az önce yazılan wca'yı da — silerdi. "cache" yok (re-review m4):
/// bu kökteki her cevap no-store (merkez ve cihaz), HTTP önbelleğinde bir şey yok; Chromium'da önbellek silme yönlendirmeyi
/// gözle görülür geciktirir.
pub const GIRIS_TEMIZLIK: &str = "\"storage\"";
/// Çıkış: çerezler de (AP'lerin çerezleri, tarayıcının sakladığı HTTP kimlik bilgileri). Kayıtlı alan adının bütün çerezleri
/// silindiği için panel, yönetim ve cihaz paneli oturumları da kapanır.
pub const CIKIS_TEMIZLIK: &str = "\"cache\", \"cookies\", \"storage\"";
pub const ESKI_CEREZ_EN_COK: usize = 30;
/// Çerçeveli AP arayüzlerinde (ör. TP-Link userRpm frameset) sade sayfalar çerçevenin içinde görünsün ve bağlantıları üst
/// pencereyi açsın (re-review m1). Yalnızca aynı kökenden; merkezin diğer cevaplarında DENY kalır (cevabın verdiği güvenlik
/// başlığı varsayılanın yerine geçer). CSP = `merkez::GUVENLIK_BASLIKLARI` + frame-ancestors.
const CERCEVE: [(&str, &str); 2] = [
    ("X-Frame-Options", "SAMEORIGIN"),
    (
        "Content-Security-Policy",
        "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'self'",
    ),
];

/// Tarayıcı Sec-Fetch-* gönderdiyse istek `kaynaklar`dan gelen üst düzey bir gezinme olmalı (çerçeve, fetch, resim değil).
/// Göndermeyen eski tarayıcı geçer (bu başlıkları sayfa değil tarayıcı yazar; betik değiştiremez).
pub fn ust_duzey_gezinme(basliklar: &Basliklar, kaynaklar: &[&str]) -> bool {
    let b = |ad: &str| basliklar.get(ad).map(|v| py_strip(v).to_lowercase());
    let (site, mod_, hedef) = (b("Sec-Fetch-Site"), b("Sec-Fetch-Mode"), b("Sec-Fetch-Dest"));
    site.as_deref().is_none_or(|s| kaynaklar.contains(&s)) && mod_.as_deref().is_none_or(|m| m == "navigate") && hedef.as_deref().is_none_or(|h| h == "document")
}

/// Günlük için Sec-Fetch-* değerleri (kısaltılmış, kaçışlı: istemcinin yazdığı değer satır bozamaz).
pub fn getirme_ozeti(basliklar: &Basliklar) -> String {
    ["Sec-Fetch-Site", "Sec-Fetch-Mode", "Sec-Fetch-Dest"]
        .iter()
        .map(|ad| {
            let deger: String = basliklar.get(ad).unwrap_or("None").chars().take(20).collect();
            format!("{}={}", ad[10..].to_lowercase(), py_repr(&deger))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Yalnızca ap.wificorrect.com (Domain yok). Strict: giriş zinciri (cihaz.wificorrect.com → /_ap-gecis → /_ap-giris → /)
/// baştan sona aynı site; başka siteden üst düzey GET gezinmesi de oturumlu gitmez (GET ile ayar değiştiren AP'ler).
fn cerez(token: Option<&str>) -> String {
    match token {
        None => format!("{CEREZ}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"),
        Some(t) => format!("{CEREZ}={t}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={}", SURE as i64),
    }
}

/// RFC 6265 belirteci (ASCII): Set-Cookie'ye eklenen ad başlık bozamaz.
fn cerez_adi_gecerli(ad: &str) -> bool {
    let n = ad.chars().count();
    (1..=64).contains(&n) && ad.chars().all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c))
}

/// `/_ap-giris`'e gelen, bu kökte önceki AP oturumlarından kalan (Path=/) çerezlerin adları (wca hariç, tekrarsız, en çok
/// `ESKI_CEREZ_EN_COK`): yeni oturumda silinir — bir müşterinin AP'sinin çerezleri (ör. TP-Link'in betikle yazdığı
/// Authorization = Basic kullanıcı:parola) sonraki müşterinin AP'sine gitmesin. Geçersiz ad atlanır.
pub fn eski_cerezler(baslik: Option<&str>) -> Vec<String> {
    let mut adlar: Vec<String> = Vec::new();
    for parca in baslik.unwrap_or("").split(';') {
        let ad = py_strip(parca.split_once('=').map(|(a, _)| a).unwrap_or(parca));
        if ad != CEREZ && cerez_adi_gecerli(ad) && !adlar.iter().any(|x| x == ad) {
            adlar.push(ad.to_string());
        }
    }
    adlar.truncate(ESKI_CEREZ_EN_COK);
    adlar
}

/// Sade Türkçe sayfa ve cihaz paneline bağlantı (JavaScript yok). `cikis`: AP kökünde, temizleyerek çıkış bağlantısı.
/// Bağlantılar üst pencerede açılır, sayfa aynı kökenden çerçevelenebilir (CERCEVE).
pub fn sayfa(durum: u16, baslik: &str, metin: &str, cerez: Option<&str>, cikis: bool) -> Yanit {
    let cik = if cikis {
        "<p><a href=\"/_ap-cikis\" target=\"_top\">AP oturumunu kapat ve bu tarayıcıdaki AP verisini sil</a> (WifiCorrect \
         panel ve yönetim oturumlarınız da kapanır)</p>"
    } else {
        ""
    };
    let govde = web::cerceve(
        baslik,
        &kart(&format!(
            "<h1>{}</h1><p>{}</p><p><a href=\"{CIHAZ_PANELI}/\" target=\"_top\">Cihaz paneline git</a></p>{cik}",
            e(baslik),
            e(metin)
        )),
        "",
        false,
        None,
        "",
    );
    let mut b: Vec<(String, String)> = vec![
        ("Content-Type".into(), "text/html; charset=utf-8".into()),
        ("Cache-Control".into(), "no-store".into()),
    ];
    b.extend(CERCEVE.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    if let Some(c) = cerez {
        b.push(("Set-Cookie".into(), c.to_string()));
    }
    Yanit { durum, basliklar: b, govde: govde.into_bytes() }
}

/// Cihaz paneli oturumu (`Cihaz::oturum`) belirteçteki / AP oturumundaki müşteri, cihaz ve tünel adresiyle aynı mı.
fn ayni(o: Option<&CihazOturumu>, b: &CihazOturumu) -> bool {
    o.is_some_and(|o| (&o.numara, o.cihaz, &o.adres) == (&b.numara, b.cihaz, &b.adres))
}

#[derive(PartialEq, Debug)]
enum Ag {
    V4(u32),
    V6(u128),
    /// Adres olmayan değer (ör. X-Forwarded-For'suz "yerel"): yalnızca kendisiyle aynı.
    Ham(Option<String>),
}

/// Python `ipaddress.ip_address`: IPv6 kapsam kimliği (`%eth0`) kabul edilir.
fn ip_coz(s: &str) -> Option<IpAddr> {
    if s.contains(':') {
        let (adres, ayrac, kapsam) = match s.split_once('%') {
            Some((a, k)) => (a, true, k),
            None => (s, false, ""),
        };
        if ayrac && (kapsam.is_empty() || kapsam.contains('%')) {
            return None;
        }
        adres.parse::<std::net::Ipv6Addr>().ok().map(IpAddr::V6)
    } else {
        s.parse::<std::net::Ipv4Addr>().ok().map(IpAddr::V4)
    }
}

/// Adresin ağı (IPv4 /24, IPv6 /64; IPv4'e eşlenmiş IPv6 yazımı IPv4 sayılır); adres değilse değerin kendisi.
fn ag(ip: Option<&str>) -> Ag {
    let Some(s) = ip else {
        return Ag::Ham(None);
    };
    match ip_coz(s) {
        None => Ag::Ham(Some(s.to_string())),
        Some(IpAddr::V4(a)) => Ag::V4(u32::from(a) & (u32::MAX << (32 - AG_ONEKI_V4))),
        Some(IpAddr::V6(a)) => match a.to_ipv4_mapped() {
            Some(v4) => Ag::V4(u32::from(v4) & (u32::MAX << (32 - AG_ONEKI_V4))),
            None => Ag::V6(u128::from(a) & (u128::MAX << (128 - AG_ONEKI_V6))),
        },
    }
}

/// `/_ap-gecis`'i isteyen adres ile `/_ap-giris`'e gelen adres aynı ağda mı. IPv4 ile IPv6 hiçbir zaman; adres olmayan değer
/// (ör. X-Forwarded-For'suz "yerel") yalnızca kendisiyle aynı.
pub fn ayni_ag(x: Option<&str>, y: Option<&str>) -> bool {
    ag(x) == ag(y)
}

/// Python `urllib.parse.quote(s, safe="/?=&-._~%")`: harf, rakam, `_.-~` ve `safe` dışındaki her bayt `%XX`.
fn quote_donus(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/?=&%".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// Belirteç yok / kullanılmış / süresi dolmuş ya da altındaki cihaz paneli oturumu düşmüş.
fn gecersiz() -> Yanit {
    sayfa(
        403,
        "Bağlantı geçersiz",
        &format!("AP arayüzü bağlantısı geçersiz ya da süresi dolmuş (bağlantı bir dakika geçerlidir ve bir kez kullanılır). {YENIDEN}"),
        None,
        false,
    )
}

pub struct Ap {
    cihaz: Arc<Cihaz>,
    saat: Saat,
    oturumlar: Oturumlar,
}

impl Ap {
    /// `cihaz`: cihaz paneli oturumları ve AP geçiş belirteçleri onda.
    pub fn new(cihaz: Arc<Cihaz>, saat: Saat) -> Ap {
        Ap { cihaz, saat, oturumlar: Oturumlar::new(SURE, SURE) }
    }

    fn simdi(&self) -> f64 {
        (self.saat)()
    }

    /// Altındaki cihaz paneli oturumu; o düşmüşse ya da başka cihaza/adrese dönmüşse AP oturumu da düşer.
    fn gecerli(&self, token: Option<&str>) -> Sonuc<Option<CihazOturumu>> {
        let Some(token) = token else {
            return Ok(None);
        };
        let Some(ot) = self.oturumlar.get(token, self.simdi()) else {
            return Ok(None);
        };
        let al = |k: &str| ot.ek.get(k).cloned().unwrap_or(Value::Null);
        let (wcc, cihaz, adres) = (al("wcc"), al("cihaz"), al("adres"));
        let o = self.cihaz.oturum(wcc.as_str())?;
        let b = CihazOturumu {
            numara: ot.user.clone(),
            rol: String::new(),
            adres: adres.as_str().unwrap_or("").to_string(),
            cihaz: cihaz.as_i64().unwrap_or(-1),
        };
        if !ayni(o.as_ref(), &b) {
            self.oturumlar.drop(token);
            return Ok(None);
        }
        Ok(o)
    }

    fn isle(&self, i: &Istek) -> Sonuc<Yanit> {
        let token = cerez_degeri(i.basliklar.get("Cookie"), CEREZ);
        let token = token.as_deref();
        match i.yol.as_str() {
            "/ap-yetki" => {
                let Some(o) = self.gecerli(token)? else {
                    return Ok(sayfa(
                        403,
                        "AP erişimi kapalı",
                        &format!("AP arayüzü oturumu yok ya da süresi doldu (en çok 30 dakika). {YENIDEN}"),
                        token.filter(|t| !t.is_empty()).map(|_| cerez(None)).as_deref(),
                        true,
                    ));
                };
                // kullanıcı her zaman müşteri numarası (yönetici geçişinde de): cihaz kendi bağındaki numarayla karşılaştırır.
                // Ayrı başlık: X-WFC-Kullanici olsaydı eski (0.4.15 öncesi) cihaz isteği oturumlu panel isteği sayardı
                Ok(Yanit::yeni(
                    200,
                    &[("X-WFC-Cihaz", &o.adres), ("X-WFC-AP-Kullanici", &o.numara), ("X-WFC-AP", "1"), ("Cache-Control", "no-store")],
                    Vec::new(),
                ))
            }
            "/_ap-giris" => {
                let t = sorgu_ilk(&i.sorgu, "t").unwrap_or_default();
                if !ust_duzey_gezinme(&i.basliklar, &GIRIS_KAYNAKLARI) {
                    // belirteç harcanmadan
                    if !t.is_empty() {
                        // belirteçli istek: saldırı ya da beklenmedik tarayıcı davranışı (teşhis için)
                        web::log(&format!("AP_GIRIS_RED {} ip={}", getirme_ozeti(&i.basliklar), i.ip));
                    }
                    return Ok(sayfa(403, "AP arayüzü açılamadı", &elle(), None, false));
                }
                let simdi = self.simdi();
                let b = self.cihaz.ap_belirtecleri.tuket(&t, simdi);
                if let Some(b) = &b {
                    if !ayni_ag(Some(&b.ip), Some(&i.ip)) {
                        // belirteç /_ap-gecis'i isteyen adresin ağına bağlı: dışarı sızdırılan belirteç başka ağdan kullanılamaz
                        // (yanar). Meşru kullanıcıysa bağlantıları farklı adreslerden çıkıyor: genel "geçersiz" değil, sebep ve çözüm
                        web::log(&format!("AP_GIRIS_RED adres_farkli ip={} gecis_ip={}", i.ip, b.ip));
                        return Ok(sayfa(403, "Farklı internet adresi", FARKLI_AG, None, false));
                    }
                }
                let o = match &b {
                    Some(b) => self.cihaz.oturum(Some(&b.wcc))?,
                    None => None,
                };
                let (Some(b), Some(o)) = (&b, &o) else {
                    return Ok(gecersiz());
                };
                if !ayni(Some(o), &b.oturum) {
                    return Ok(gecersiz());
                }
                self.oturumlar.drop(token.unwrap_or("")); // bu tarayıcının eski AP oturumu
                let mut ek = BTreeMap::new();
                ek.insert("wcc".to_string(), Value::from(b.wcc.clone()));
                ek.insert("cihaz".to_string(), Value::from(o.cihaz));
                ek.insert("adres".to_string(), Value::from(o.adres.clone()));
                let yeni = self.oturumlar.create(&o.numara, &o.rol, simdi, "", ek);
                let mut y = Yanit::yeni(303, &[("Location", "/"), ("Set-Cookie", &cerez(Some(&yeni)))], Vec::new());
                for ad in eski_cerezler(i.basliklar.get("Cookie")) {
                    y.basliklar.push(("Set-Cookie".into(), format!("{ad}=; Path=/; Max-Age=0; Secure")));
                }
                y.basliklar.push(("Clear-Site-Data".into(), GIRIS_TEMIZLIK.into()));
                y.basliklar.push(("Cache-Control".into(), "no-store".into()));
                Ok(y)
            }
            "/_ap-cikis" => {
                if !ust_duzey_gezinme(&i.basliklar, &CIKIS_KAYNAKLARI) {
                    return Ok(sayfa(403, "AP oturumu kapatılamadı", "Çıkış yalnızca AP arayüzünden ya da cihaz panelinden açılır.", None, false));
                }
                self.oturumlar.drop(token.unwrap_or(""));
                // yalnızca cihaz.wificorrect.com'da bir yol; Location ASCII kalsın (http.server başlığı latin-1 yazar)
                let donus = quote_donus(&guvenli_donus(&sorgu_ilk(&i.sorgu, "donus").unwrap_or_else(|| "/".into())));
                Ok(Yanit::yeni(
                    303,
                    &[
                        ("Location", &format!("{CIHAZ_PANELI}{donus}")),
                        ("Set-Cookie", &cerez(None)),
                        ("Clear-Site-Data", CIKIS_TEMIZLIK),
                        ("Cache-Control", "no-store"),
                    ],
                    Vec::new(),
                ))
            }
            _ => Ok(Yanit::metin(404, "Bulunamadı")),
        }
    }
}

impl Uygulama for Ap {
    fn istek(&self, i: &Istek) -> Yanit {
        match self.isle(i) {
            Ok(y) => y,
            Err(h) => hata_yaniti(&i.yontem, &i.yol, &format!("{h:?}")),
        }
    }
}

#[cfg(test)]
#[path = "ap_testler.rs"]
mod testler;
