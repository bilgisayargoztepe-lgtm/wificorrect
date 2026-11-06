//! Müşteri paneli (Python musteri.py, spec §6): panel.wificorrect.com — müşteri numarası + parola; kayıtlar (gün/kişi/arama),
//! raporlar, parola, cihaz paneline geçiş ve kolay bağlama. Üyeliği bitmiş müşteri de girer (yalnızca okuma).
use std::collections::HashMap;
use std::sync::Arc;

use crate::guvenlik::{self, GecisBelirtecleri};
use crate::kayitlar::Kayitlar;
use crate::ortak::{now_iso, tarih_goster};
use crate::raporlar;
#[cfg(test)]
use crate::raporlar::Raporlar;
use crate::veri::{Cihaz, Hata, Saat, Sonuc, Veri, AYRILMA_SN};
use crate::web::{e, kart, sorgu_ilk, yanit_html, yonlendir, Ot, Panel, Taban, Yanit};

/// Cihaz bağlandıktan sonra tünel kalkana dek "panel hazırlanıyor" (sonra cihaz paneline).
pub const HAZIRLIK_SN: f64 = 30.0;
pub const YENILE_SN: u32 = 5;
pub const BULUNAMADI: &str = "Cihazınız bulunamadı. Cihazın modeme takılı ve açık olduğundan, bu sayfayı cihazla aynı internetten \
(işletmenin Wi-Fi'ı ya da kablosu; mobil veri değil) açtığınızdan emin olun.";
pub const BAGLANIYOR: &str = "Cihazınız bulundu ve hesabınıza bağlanıyor. Bu birkaç dakika sürebilir; bu sayfa kendini yeniler.";

fn parola_html(csrf: &str, hata: &str, tamam: &str) -> String {
    let m = if !hata.is_empty() {
        format!("<p class=\"hata\">{}</p>", e(hata))
    } else if !tamam.is_empty() {
        format!("<p class=\"tamam\">{}</p>", e(tamam))
    } else {
        String::new()
    };
    kart(&format!(
        "<h1>Parola değiştir</h1><p class=\"alt\">Yeni parola en az 10 karakter olmalı.</p>\
         <form method=\"post\" action=\"/parola\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <label for=\"m\">Mevcut parola</label><input id=\"m\" name=\"mevcut\" type=\"password\" autocomplete=\"current-password\" required>\
         <label for=\"y\">Yeni parola</label><input id=\"y\" name=\"yeni\" type=\"password\" autocomplete=\"new-password\" minlength=\"10\" required>\
         <label for=\"t\">Yeni parola (tekrar)</label><input id=\"t\" name=\"tekrar\" type=\"password\" autocomplete=\"new-password\" minlength=\"10\" required>\
         {m}<button>Değiştir</button></form><p><a href=\"/\">← Özete dön</a></p>",
        e(csrf)
    ))
}

/// Oturumdaki kullanıcı (müşteri numarası); oturumu biz açtığımız için hep geçerlidir.
fn numara(ot: &Ot) -> Sonuc<i64> {
    ot.user.parse::<i64>().map_err(|_| Hata::Deger(format!("müşteri numarası geçersiz: {}", ot.user)))
}

pub struct Musteri {
    taban: Taban,
    /// panel → cihaz.wificorrect.com geçiş belirteçleri; cihaz geçidiyle ve yönetimle paylaşılır.
    pub gecis: Arc<GecisBelirtecleri>,
}

impl Musteri {
    pub fn new(veri: Arc<Veri>, kayitlar: impl Into<Arc<Kayitlar>>, saat: Saat) -> Musteri {
        Musteri { taban: Taban::new(veri, kayitlar, saat), gecis: Arc::new(GecisBelirtecleri::new()) }
    }

    /// Üretim: varsayılan klasörler (özetler, raporlar), paylaşılan kayıtlar ve geçiş belirteçleri.
    pub fn uretim(veri: Arc<Veri>, saat: Saat, gecis: Arc<GecisBelirtecleri>, kayitlar: Arc<Kayitlar>) -> Musteri {
        Musteri::new(veri, kayitlar, saat).gecis_ile(gecis)
    }

    /// Python `Musteri(..., gecis=...)`: cihaz geçidiyle ve yönetimle aynı belirteç deposu.
    pub fn gecis_ile(mut self, gecis: Arc<GecisBelirtecleri>) -> Musteri {
        self.gecis = gecis;
        self
    }

    /// Python `Musteri(..., raporlar=...)`.
    #[cfg(test)]
    pub fn raporlar_ile(mut self, r: Raporlar) -> Musteri {
        self.taban = self.taban.raporlar_ile(r);
        self
    }

    /// Müşteri oturumları (cihaz geçidi çıkışta `drop_user` çağırır; Python `musteri_app.oturumlar`).
    pub fn oturumlar(&self) -> &guvenlik::Oturumlar {
        &self.taban.oturumlar
    }

    /// Bağlı, tünel adresi olan, üyeliği ve lisansı açık cihazın kaydı; değilse None.
    pub fn calisan_cihaz(&self, numara: i64) -> Sonuc<Option<Cihaz>> {
        let v = &self.taban.veri;
        let (c, m) = (v.bagli_cihaz(numara)?, v.musteri(numara)?);
        let (Some(c), Some(m)) = (c, m) else {
            return Ok(None);
        };
        if c.durum != "bagli" || c.tunel_ip.is_empty() {
            return Ok(None);
        }
        if m.uyelik != "aktif" || v.lisans_durumu(&m).0 != "aktif" {
            return Ok(None);
        }
        Ok(Some(c))
    }

    /// Çalışan cihazın tünel IP'si; değilse None.
    pub fn cihaz_adresi(&self, numara: i64) -> Sonuc<Option<String>> {
        Ok(self.calisan_cihaz(numara)?.map(|c| c.tunel_ip))
    }

    // --- kolay bağlama (spec 2026-10-16) ---
    fn bekle_sayfasi(&self, ot: &Ot, baslik: &str, metin: &str) -> Yanit {
        let h = self.sayfa(baslik, &kart(&format!("<h1>{}</h1><p>{}</p>", e(baslik), e(metin))), Some(ot), false, Some(YENILE_SN));
        yanit_html(&h, 200, None)
    }

    /// Müşterinin bağlı cihazı yoksa aynı internet adresinden bekleyen cihazı bulur: tek → kendiliğinden ayırır,
    /// çok → liste (müşteri seçer), hiç → nasıl yapılacağı. Ayrılan cihaz sonraki bildiriminde bağlanır.
    fn baglan(&self, ot: &Ot, n: i64, yontem: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Yanit> {
        let v = &self.taban.veri;
        if let Some(c) = v.bagli_cihaz(n)? {
            if let Some(r) = v.bekleyen_musterinin(n)? {
                if r.atama == "bagli" {
                    let gecen = self.taban.simdi() - r.kayit.baglandi;
                    if c.tunel_ip.is_empty() && gecen < AYRILMA_SN {
                        // tünel kaydı başarısız: cihaz yeniden deniyor
                        return Ok(self.bekle_sayfasi(ot, "Cihazınız bağlanıyor", BAGLANIYOR));
                    }
                    if !c.tunel_ip.is_empty() && gecen < HAZIRLIK_SN {
                        return Ok(self.bekle_sayfasi(ot, "Cihazınız bağlandı", "Cihaz paneliniz hazırlanıyor, bu sayfa kendini yeniler."));
                    }
                }
            }
            return Ok(yonlendir("/cihaz", None));
        }
        let m = v.musteri(n)?;
        if m.as_ref().map(|m| m.uyelik.as_str()) != Some("aktif") {
            return Ok(yanit_html(&self.mesaj(Some(ot), "Cihaz bağlanamıyor", "Üyeliğiniz sona ermiş. Hizmet sağlayıcınıza başvurun."), 200, None));
        }
        let adaylar = if !ip.is_empty() && ip != "yerel" { v.bekleyen_adaylar(ip)? } else { Vec::new() };
        if yontem == "POST" {
            let k = form.get("kimlik").map(String::as_str).unwrap_or("");
            if !adaylar.iter().any(|a| a.kimlik == k) || !v.bekleyen_ayir(k, n, ip)? {
                return Ok(yanit_html(
                    &self.mesaj(Some(ot), "Cihaz seçilemedi", "Bu cihaz artık seçilemiyor. Sayfayı yenileyip tekrar deneyin."),
                    400,
                    None,
                ));
            }
            v.hareket(&n.to_string(), ip, "CIHAZ_OTOMATIK_AYRILDI", Some(n), &format!("kimlik={} secim=liste", ilk8(k)))?;
            return Ok(yonlendir("/baglan", None));
        }
        if v.bekleyen_musterinin(n)?.is_some() {
            return Ok(self.bekle_sayfasi(ot, "Cihazınız bağlanıyor", BAGLANIYOR));
        }
        if adaylar.len() == 1 && v.bekleyen_ayir(&adaylar[0].kimlik, n, ip)? {
            v.hareket(&n.to_string(), ip, "CIHAZ_OTOMATIK_AYRILDI", Some(n), &format!("kimlik={} secim=tek", ilk8(&adaylar[0].kimlik)))?;
            return Ok(self.bekle_sayfasi(ot, "Cihazınız bağlanıyor", BAGLANIYOR));
        }
        if adaylar.is_empty() {
            let h = self.sayfa(
                "Cihaz bağlama",
                &kart(&format!("<h1>Cihazınız bulunamadı</h1><p>{}</p><p><a href=\"/baglan\">Yeniden dene</a></p>", e(BULUNAMADI))),
                Some(ot),
                false,
                None,
            );
            return Ok(yanit_html(&h, 200, None));
        }
        let satirlar: String = adaylar
            .iter()
            .map(|a| {
                let mac: String = {
                    let k: Vec<char> = a.wan_mac.chars().collect();
                    k[k.len().saturating_sub(5)..].iter().collect::<String>().to_uppercase()
                };
                format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td><form method=\"post\" action=\"/baglan\">\
                     <input type=\"hidden\" name=\"csrf\" value=\"{}\"><input type=\"hidden\" name=\"kimlik\" value=\"{}\">\
                     <button class=\"ikincil\">Bu cihaz</button></form></td></tr>",
                    e(&tarih_goster(&now_iso(a.ilk), false)),
                    e(if mac.is_empty() { "—" } else { &mac }),
                    e(if a.surum.is_empty() { "—" } else { &a.surum }),
                    e(&ot.csrf),
                    e(&a.kimlik)
                )
            })
            .collect();
        let h = self.sayfa(
            "Cihaz bağlama",
            &kart(&format!(
                "<h1>Birden çok cihaz bulundu</h1><p>Bu internet bağlantısında bağlanmayı bekleyen birden çok cihaz var. \
                 Sizinkini seçin: cihazın altındaki etiketteki MAC adresinin son dört hanesine bakın.</p>\
                 <div class=\"kaydir\"><table><tr><th>İlk görülme</th><th>MAC (son 4)</th><th>Sürüm</th><th></th></tr>\
                 {satirlar}</table></div><p><a href=\"/baglan\">Listeyi yenile</a></p>"
            )),
            Some(ot),
            false,
            None,
        );
        Ok(yanit_html(&h, 200, None))
    }
}

/// Python `k[:8]`.
fn ilk8(k: &str) -> String {
    k.chars().take(8).collect()
}

impl Panel for Musteri {
    fn taban(&self) -> &Taban {
        &self.taban
    }

    fn rol(&self) -> &'static str {
        "musteri"
    }

    fn cerez(&self) -> &'static str {
        "wc_musteri"
    }

    fn kullanici_etiketi(&self) -> &'static str {
        "Müşteri numarası"
    }

    fn giris_basligi(&self) -> &'static str {
        "Müşteri girişi"
    }

    fn giris_aciklama(&self) -> &'static str {
        "Size verilen müşteri numarası ve parolayla girin."
    }

    fn dogrula(&self, kul: &str, pw: &str) -> Sonuc<Option<String>> {
        if !guvenlik::numara_gecerli(kul) {
            guvenlik::bos_dogrulama(pw);
            return Ok(None);
        }
        let n: i64 = kul.parse().map_err(|_| Hata::Deger(kul.to_string()))?;
        Ok(self.taban.veri.parola_dogrula(n, pw)?.map(|m| m.numara.to_string()))
    }

    fn hesap_surumu(&self, kimlik: &str) -> Sonuc<Option<String>> {
        let n: i64 = kimlik.parse().map_err(|_| Hata::Deger(kimlik.to_string()))?;
        Ok(self.taban.veri.musteri(n)?.map(|m| m.ozet))
    }

    fn girdi(&self, kimlik: &str, ip: &str) -> Sonuc<()> {
        let n: i64 = kimlik.parse().map_err(|_| Hata::Deger(kimlik.to_string()))?;
        self.taban.veri.giris_kaydet(n)?;
        self.taban.veri.hareket(kimlik, ip, "GIRIS", Some(n), "müşteri paneli")
    }

    fn kim(&self, ot: &Ot) -> String {
        ot.user.clone()
    }

    fn hareket_musterisi(&self, ot: &Ot, _yol: &str) -> Option<i64> {
        ot.user.parse().ok()
    }

    fn menu(&self, _ot: &Ot) -> String {
        "<a href=\"/cihaz\">Cihaz paneli</a><a href=\"/\">Yedek arşiv</a><a href=\"/raporlar\">Raporlar</a><a href=\"/parola\">Parola</a>".into()
    }

    /// Cihazı çalışıyorsa cihaz paneli; bağlı cihazı yoksa (üyelik aktif) kolay bağlama; değilse yedek arşiv.
    fn giris_sonrasi(&self, kimlik: &str) -> Sonuc<String> {
        let n: i64 = kimlik.parse().map_err(|_| Hata::Deger(kimlik.to_string()))?;
        if self.cihaz_adresi(n)?.is_some() {
            return Ok("/cihaz".into());
        }
        let m = self.taban.veri.musteri(n)?;
        if self.taban.veri.bagli_cihaz(n)?.is_none() && m.is_some_and(|m| m.uyelik == "aktif") {
            return Ok("/baglan".into());
        }
        Ok("/".into())
    }

    fn sayfalar(&self, ot: &Ot, _token: &str, yontem: &str, yol: &str, sorgu: &str, form: &HashMap<String, String>, ip: &str) -> Sonuc<Option<Yanit>> {
        let n = numara(ot)?;
        let v = &self.taban.veri;
        if (yontem == "GET" && (yol == "/" || yol.starts_with("/gun/") || yol == "/ara")) || (yontem == "POST" && yol == "/ara") {
            let c = v.bagli_cihaz(n)?;
            let baslik = match &c {
                Some(c) if !c.isletme_adi.is_empty() => c.isletme_adi.clone(),
                _ => format!("Müşteri {n}"),
            };
            return self.kayit_sayfasi(ot, n, yol, sorgu, "", ip, &baslik, yontem, Some(form));
        }
        if yontem == "GET" && (yol == "/raporlar" || yol.starts_with("/rapor/")) {
            return raporlar::istek(self, ot, n, yol, "");
        }
        if yol == "/baglan" && (yontem == "GET" || yontem == "POST") {
            return self.baglan(ot, n, yontem, form, ip).map(Some);
        }
        if yol == "/cihaz" && yontem == "GET" {
            let m = v.musteri(n)?;
            if v.bagli_cihaz(n)?.is_none() && m.is_some_and(|m| m.uyelik == "aktif") {
                return Ok(Some(yonlendir("/baglan", None)));
            }
            // yok=1: Caddy cihaza ulaşamadı → döngü olmasın
            if self.cihaz_adresi(n)?.is_some() && sorgu_ilk(sorgu, "yok").is_none() {
                let ozet = v.musteri(n)?.map(|m| m.ozet).unwrap_or_default();
                let t = self.gecis.uret(&ot.user, self.taban.simdi(), &ozet, "musteri");
                return Ok(Some(yonlendir(&format!("https://cihaz.wificorrect.com/_giris?t={t}"), None)));
            }
            let h = self.sayfa(
                "Cihaz paneli",
                &kart(
                    "<h1>Cihazınıza şu an ulaşılamıyor</h1><p>Cihaz kapalı, internetsiz ya da lisansı kapalı olabilir. \
                     Yedek arşiviniz aşağıdadır.</p><p><a href=\"/\">Yedek arşiv</a></p>",
                ),
                Some(ot),
                false,
                None,
            );
            return Ok(Some(yanit_html(&h, 200, None)));
        }
        if yol == "/parola" && yontem == "GET" {
            return Ok(Some(yanit_html(&self.sayfa("Parola", &parola_html(&ot.csrf, "", ""), Some(ot), false, None), 200, None)));
        }
        if yol == "/parola" && yontem == "POST" {
            let hata = |m: &str| -> Sonuc<Option<Yanit>> { Ok(Some(yanit_html(&self.sayfa("Parola", &parola_html(&ot.csrf, m, ""), Some(ot), false, None), 400, None))) };
            let al = |k: &str| form.get(k).map(String::as_str).unwrap_or("");
            if v.parola_dogrula(n, al("mevcut"))?.is_none() {
                return hata("Mevcut parola yanlış.");
            }
            if al("yeni") != al("tekrar") {
                return hata("Yeni parolalar aynı değil.");
            }
            match v.parola_koy(n, al("yeni")) {
                Ok(_) => {}
                Err(Hata::Deger(m)) => return hata(&m),
                Err(h) => return Err(h),
            }
            let t = &self.taban;
            t.oturumlar.drop_user(&ot.user);
            let yeni = self.oturum_ac(&ot.user, t.simdi())?;
            v.hareket(&ot.user, ip, "PAROLA_DEGISTI", Some(n), "müşteri paneli")?;
            let tamam = "Parolanız değiştirildi. Cihazınızda ertesi sabah 06:00'dan sonra geçerli olur.";
            // gövdedeki CSRF yeni oturumundur; üst menüdeki çıkış formu eski oturumun (Python gibi)
            let csrf = t.oturumlar.get(&yeni, t.simdi()).map(|o| o.csrf).unwrap_or_default();
            let h = self.sayfa("Parola", &parola_html(&csrf, "", tamam), Some(ot), false, None);
            return Ok(Some(yanit_html(&h, 200, Some((self.cerez(), Some(&yeni))))));
        }
        Ok(None)
    }
}

#[cfg(test)]
#[path = "musteri_testler.rs"]
pub(crate) mod testler;
