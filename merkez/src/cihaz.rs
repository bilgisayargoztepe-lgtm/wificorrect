//! cihaz.wificorrect.com (Python cihaz.py, spec 2026-10-09): Caddy forward_auth buraya sorar; geçiş belirteci → çerez; çıkış.
//! `/_ap-gecis` (spec 2026-10-16): cihaz paneli oturumundan AP arayüzüne (ap.wificorrect.com, ap.rs) tek kullanımlık geçiş.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ap;
use crate::guvenlik::{GecisBelirtecleri, HizSiniri, Oturumlar, TekKullanimlik};
use crate::merkez::Uygulama;
use crate::musteri::Musteri;
use crate::veri::{Hata, Saat, Sonuc, Veri};
use crate::web::{self, cerez_degeri, hata_yaniti, sorgu_ilk, Basliklar, Istek, Yanit};

pub const PANEL_GIRIS: &str = "https://panel.wificorrect.com/giris";
pub const YONETIM: &str = "https://yonetim.wificorrect.com";
/// Yönetimden doğrudan geçiş (yönetim "Cihaz paneline git"): belirteç bu adrese eklenir.
pub const CIHAZ_GIRIS: &str = "https://cihaz.wificorrect.com/_giris?t=";
pub const CEREZ: &str = "wcc";
/// Müşteri başına dakikada (API ile aynı); her geçiş bir hareket satırı yazar.
pub const AP_GECIS_SINIRI: usize = 20;

fn cerez(token: Option<&str>) -> String {
    match token {
        None => format!("{CEREZ}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"),
        // panelden gelen yönlendirmede de gönderilsin
        Some(t) => format!("{CEREZ}={t}; HttpOnly; Secure; SameSite=Lax; Path=/"),
    }
}

/// Geçerli cihaz paneli oturumu: numara, rol, cihazın tünel IP'si ("adres") ve cihaz kayıt numarası.
#[derive(Clone, Debug, PartialEq)]
pub struct CihazOturumu {
    pub numara: String,
    pub rol: String,
    pub adres: String,
    pub cihaz: i64,
}

/// AP arayüzüne geçiş belirtecinin değeri (Python `{"wcc": token, "ip": ip, **o}`).
#[derive(Clone, Debug)]
pub struct ApBelirteci {
    pub wcc: String,
    pub ip: String,
    pub oturum: CihazOturumu,
}

pub struct Cihaz {
    veri: Arc<Veri>,
    gecis: Arc<GecisBelirtecleri>,
    musteri: Arc<Musteri>,
    saat: Saat,
    pub oturumlar: Oturumlar,
    /// AP arayüzüne geçiş belirteçleri: ap::Ap (`/_ap-giris`) tüketir.
    pub ap_belirtecleri: TekKullanimlik<ApBelirteci>,
    ap_hiz: HizSiniri,
}

impl Cihaz {
    /// `gecis`: panelle (ve yönetimle) paylaşılan geçiş belirteçleri; `musteri`: aynı müşteri paneli uygulaması.
    pub fn new(veri: Arc<Veri>, gecis: Arc<GecisBelirtecleri>, musteri: Arc<Musteri>, saat: Saat) -> Cihaz {
        Cihaz {
            veri,
            gecis,
            musteri,
            saat,
            oturumlar: Oturumlar::default(),
            ap_belirtecleri: TekKullanimlik::new(GecisBelirtecleri::SURE),
            ap_hiz: HizSiniri::new(AP_GECIS_SINIRI, 60.0),
        }
    }

    fn simdi(&self) -> f64 {
        (self.saat)()
    }

    /// Geçerli cihaz paneli oturumu ya da None. Parola değiştiyse / lisans, üyelik kapandıysa / cihaz yoksa oturum düşer.
    /// Her bakış oturumu canlı tutar (boşta 30 dk).
    pub fn oturum(&self, token: Option<&str>) -> Sonuc<Option<CihazOturumu>> {
        let Some(token) = token else {
            return Ok(None);
        };
        let Some(ot) = self.oturumlar.get(token, self.simdi()) else {
            return Ok(None);
        };
        let n: i64 = ot.user.parse().map_err(|_| Hata::Deger(format!("müşteri numarası geçersiz: {}", ot.user)))?;
        let m = self.veri.musteri(n)?;
        let c = self.musteri.calisan_cihaz(n)?;
        match (m, c) {
            (Some(m), Some(c)) if m.ozet == ot.surum => Ok(Some(CihazOturumu { numara: ot.user, rol: ot.role, adres: c.tunel_ip, cihaz: c.id })),
            _ => {
                self.oturumlar.drop(token);
                Ok(None)
            }
        }
    }

    /// Cihaz panelindeki "AP arayüzüne git" → bu oturuma, müşteriye, cihaza ve isteyen adrese (`ip`; `/_ap-giris` aynı ağdan
    /// gelmeli: `ap::ayni_ag`) bağlı 60 sn'lik tek kullanımlık belirteç → ap.wificorrect.com/_ap-giris. Yalnızca
    /// cihaz.wificorrect.com sayfasından üst düzey gezinme (`ap::ust_duzey_gezinme`). Hareket satırında yalnızca kim (numara ya
    /// da admin) ve rol.
    fn ap_gecis(&self, token: Option<&str>, basliklar: &Basliklar, ip: &str) -> Sonuc<Yanit> {
        if !ap::ust_duzey_gezinme(basliklar, &ap::GECIS_KAYNAKLARI) {
            if token.is_some_and(|t| !t.is_empty()) {
                // oturumlu tarayıcı: saldırı ya da beklenmedik tarayıcı davranışı (teşhis için)
                web::log(&format!("AP_GECIS_RED {} ip={ip}", ap::getirme_ozeti(basliklar)));
            }
            return Ok(ap::sayfa(403, "AP arayüzü açılamadı", &ap::elle(), None, false));
        }
        let Some(o) = self.oturum(token)? else {
            return Ok(Yanit::yeni(303, &[("Location", PANEL_GIRIS), ("Cache-Control", "no-store")], Vec::new()));
        };
        let simdi = self.simdi();
        if !self.ap_hiz.izin(&o.numara, simdi) {
            return Ok(ap::sayfa(429, "Çok fazla deneme", "AP arayüzü kısa sürede çok kez açıldı. Bir dakika sonra yeniden deneyin.", None, false));
        }
        let n: i64 = o.numara.parse().map_err(|_| Hata::Deger(o.numara.clone()))?;
        let (rol, numara) = (o.rol.clone(), o.numara.clone());
        let t = self.ap_belirtecleri.uret(ApBelirteci { wcc: token.unwrap_or("").to_string(), ip: ip.to_string(), oturum: o }, simdi);
        self.veri.hareket(if rol == "yonetici" { "admin" } else { &numara }, ip, "AP_GECIS", Some(n), &format!("rol={rol}"))?;
        Ok(Yanit::yeni(303, &[("Location", &format!("{}{t}", ap::GIRIS)), ("Cache-Control", "no-store")], Vec::new()))
    }

    fn isle(&self, i: &Istek) -> Sonuc<Yanit> {
        let token = cerez_degeri(i.basliklar.get("Cookie"), CEREZ);
        let token = token.as_deref();
        match i.yol.as_str() {
            "/cihaz-yetki" => {
                let Some(o) = self.oturum(token)? else {
                    return Ok(Yanit::yeni(302, &[("Location", PANEL_GIRIS), ("Cache-Control", "no-store")], Vec::new()));
                };
                Ok(Yanit::yeni(200, &[("X-WFC-Kullanici", &o.numara), ("X-WFC-Cihaz", &o.adres), ("Cache-Control", "no-store")], Vec::new()))
            }
            "/_ap-gecis" => self.ap_gecis(token, &i.basliklar, &i.ip),
            "/_giris" => {
                let t = sorgu_ilk(&i.sorgu, "t").unwrap_or_default();
                let b = self.gecis.tuket(&t, self.simdi());
                let m = match &b {
                    Some((numara, _, _)) if !numara.is_empty() => {
                        let n: i64 = numara.parse().map_err(|_| Hata::Deger(numara.clone()))?;
                        self.veri.musteri(n)?
                    }
                    _ => None,
                };
                match (b, m) {
                    (Some((numara, ozet, rol)), Some(m)) if m.ozet == ozet => {
                        let yeni = self.oturumlar.create(&numara, &rol, self.simdi(), &m.ozet, BTreeMap::new());
                        Ok(Yanit::yeni(303, &[("Location", "/"), ("Set-Cookie", &cerez(Some(&yeni))), ("Cache-Control", "no-store")], Vec::new()))
                    }
                    _ => Ok(Yanit::yeni(303, &[("Location", PANEL_GIRIS), ("Cache-Control", "no-store")], Vec::new())),
                }
            }
            "/_cikis" => {
                let ot = token.and_then(|t| self.oturumlar.get(t, self.simdi()));
                if let Some(ot) = &ot {
                    if ot.role == "yonetici" {
                        // yönetimden gelen: yalnızca bu oturum kapanır, müşterinin kendi oturumları düşmez
                        self.oturumlar.drop(token.unwrap_or(""));
                        return Ok(Yanit::yeni(
                            303,
                            &[("Location", &format!("{YONETIM}/m/{}", ot.user)), ("Set-Cookie", &cerez(None)), ("Cache-Control", "no-store")],
                            Vec::new(),
                        ));
                    }
                    self.oturumlar.drop_user(&ot.user);
                    self.musteri.oturumlar().drop_user(&ot.user);
                }
                Ok(Yanit::yeni(303, &[("Location", PANEL_GIRIS), ("Set-Cookie", &cerez(None)), ("Cache-Control", "no-store")], Vec::new()))
            }
            _ => Ok(Yanit::metin(404, "Bulunamadı")),
        }
    }
}

impl Uygulama for Cihaz {
    fn istek(&self, i: &Istek) -> Yanit {
        match self.isle(i) {
            Ok(y) => y,
            Err(h) => hata_yaniti(&i.yontem, &i.yol, &format!("{h:?}")),
        }
    }
}

#[cfg(test)]
#[path = "cihaz_testler.rs"]
pub(crate) mod testler;
