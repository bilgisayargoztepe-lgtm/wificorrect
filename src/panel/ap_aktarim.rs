//! AP arayüzü aktarımı (spec 2026-10-16-ap-arayuz): merkezden (açık tünel + merkezin tünel adresi; `X-WFC-Kullanici`
//! ile aynı güven kuralı) `X-WFC-AP: 1` başlığıyla gelen istek panel sayfası değil, açık AP hedefine **aktarım**dır
//! (src/ap_arayuz.rs). Başlık başka kaynaktan gelirse yok sayılır: istek panele düşer (bağlı cihazda yerel istek 404).
//! Aktarımlar panelin 4 iş parçacığını tutmaz: ayrı `AP_ISCI` iş parçacığı + `AP_KUYRUK` sıralık kuyruk (dolarsa 503).
//! AP kipindeki cevaplara panelin sıkı başlıkları (CSP, X-Frame-Options: DENY …) eklenmez: AP arayüzlerini bozar.

use super::*;
use crate::ap_arayuz::{self, Hedef};

/// Aynı anda en çok bu kadar AP aktarımı (AP'lerin çoğu az bağlantı kaldırır) ve bekleyen istek
pub(super) const AP_ISCI: usize = 4;
pub(super) const AP_KUYRUK: usize = 64;

pub(super) enum ApKarar {
    /// Başlık güvenilmez (merkezden değil) ya da cihaz bağlı değil: normal panel isteği
    Panel,
    /// Aktarım yok, bu cevap (AP kipinde: panel başlıkları olmadan)
    Cevap(Resp),
    Aktar(Hedef),
}

/// AP kipinde ve geçişte gösterilen sade sayfa (menüsüz, betiksiz)
pub(super) fn sade_sayfa(durum: u16, baslik: &str, mesaj: &str, baglanti: bool) -> Resp {
    let alt = if baglanti {
        "<p><a href=\"https://cihaz.wificorrect.com/ayarlar#ap-kurulum\">Cihaz paneli → Ayarlar → AP kurulumu</a></p>"
    } else {
        ""
    };
    let body = format!(
        "<!doctype html><html lang=\"tr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{b}</title></head><body style=\"font-family:system-ui,sans-serif;max-width:40em;margin:3em auto;padding:0 16px;line-height:1.5;\
         background:#fff;color:#222\"><h1 style=\"font-size:1.4em\">{b}</h1><p>{m}</p>{alt}</body></html>",
        b = h(baslik),
        m = h(mesaj)
    );
    Resp { status: durum, body, headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())], file: None }
}

/// Merkez eski sürümdeyse `/_ap-gecis` merkeze değil cihaza gelir (normal panel isteği olarak)
pub(super) fn gecis_yok() -> Resp {
    sade_sayfa(
        503,
        "AP erişimi şu an kullanılamıyor",
        "Yönetim merkezi AP arayüzü geçişini henüz desteklemiyor. Hizmet sağlayıcınıza başvurun; AP'ye bilgisayardan betikle (ap-ac) girilebilir.",
        false,
    )
}

impl Panel {
    /// `X-WFC-AP: 1` başlıklı istek: aktarım mı, sade cevap mı, yoksa (başlık güvenilmez) normal panel isteği mi.
    /// Merkez AP kipinde de müşteri numarasını gönderir (yönetici geçişinde de): bağlı müşteri değilse 403.
    pub(super) fn ap_karar(&self, ip: &str, kullanici: Option<&str>) -> ApKarar {
        let Ok(cfg) = Config::load(&self.cfg_path) else { return ApKarar::Panel };
        if !self.merkezden(&cfg, ip) {
            return ApKarar::Panel;
        }
        let Some(bag) = crate::merkez::oku(&self.merkez_path) else { return ApKarar::Panel };
        if kullanici.map(str::trim) != Some(bag.numara.as_str()) {
            return ApKarar::Cevap(sade_sayfa(403, "Erişim yok", "Bu cihaz bu müşteriye ait değil.", false));
        }
        let now = (self.clock)();
        let kapali = || {
            ApKarar::Cevap(sade_sayfa(403, "AP erişimi kapalı", "AP arayüzü erişimi açık değil ya da süresi doldu. Cihaz panelinden yeniden açın.", true))
        };
        match ap_arayuz::denetle(&self.ap_yollar, &cfg, now) {
            ap_arayuz::Durum::Gecerli(h) => ApKarar::Aktar(h),
            ap_arayuz::Durum::Kapandi(h, neden) => {
                ap_arayuz::denetim(&cfg.main.log_root, now, &h, &ap_arayuz::kapandi_ek(&h, neden));
                kapali()
            }
            ap_arayuz::Durum::Yok => kapali(),
        }
    }
}

/// AP kipindeki sade cevap: yalnızca tür ve önbelleksiz (panelin CSP / X-Frame-Options'ı yok)
pub(super) fn ap_cevapla(req: tiny_http::Request, r: Resp) {
    let mut resp = tiny_http::Response::from_string(r.body).with_status_code(r.status);
    for (k, v) in r.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).chain([("Cache-Control", "no-store"), ("X-Content-Type-Options", "nosniff")]) {
        if let Ok(hd) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            resp.add_header(hd);
        }
    }
    let _ = req.respond(resp);
}

/// AP iş parçacığında: isteği hedefe aktarır, cevabı (gerekirse yeniden yazılmış) akıtır.
pub(super) fn ap_isle(p: &Panel, mut req: tiny_http::Request, h: Hedef) {
    let yontem = req.method().as_str().to_uppercase();
    let yol = req.url().to_string();
    let basliklar: Vec<(String, String)> = req.headers().iter().map(|hd| (hd.field.as_str().as_str().to_string(), hd.value.as_str().to_string())).collect();
    let kamu = ap_arayuz::kamu_adi(basliklar.iter().find(|(a, _)| a.eq_ignore_ascii_case("Host")).map(|(_, d)| d.as_str()));
    let boy = req.body_length().map(|n| n as u64);
    let yollar = &p.ap_yollar;
    // AP aynı adreste başka şemaya/porta yönlendirdi: hedef (hâlâ aynıysa) güncellenir
    let degisti = |sema: &str, port: u16| {
        if let Some(mut g) = ap_arayuz::oku(yollar).filter(|g| g.adres == h.adres && g.yol == h.yol && g.bitis == h.bitis) {
            (g.sema, g.port) = (sema.to_string(), port);
            let _ = ap_arayuz::yaz(yollar, &g);
        }
    };
    let sonuc = ap_arayuz::aktar(&h, &kamu, &yontem, &yol, &basliklar, boy, req.as_reader(), &*p.ap_baglan, &degisti);
    match sonuc {
        Ok(c) => {
            let basliklar: Vec<tiny_http::Header> =
                c.basliklar.iter().filter_map(|(a, d)| tiny_http::Header::from_bytes(a.as_bytes(), d.as_bytes()).ok()).collect();
            let boy = c.boy.and_then(|b| usize::try_from(b).ok());
            let _ = req.respond(tiny_http::Response::new(tiny_http::StatusCode(c.durum), basliklar, c.govde, boy, None));
        }
        Err(e) => ap_cevapla(req, sade_sayfa(e.durum(), "AP arayüzü", &e.mesaj(), true)),
    }
}
