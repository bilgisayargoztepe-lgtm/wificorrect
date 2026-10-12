//! Panel: Misafir kodları (otel/yurt; spec 2026-10-11 §3). Kod üret formu, aktif kodlar (kimlik maskeli),
//! yazdırılabilir fiş (`@media print`, betik yok). İptal → o kimliğin açık oturumları kapanır (`kod_iptal`);
//! uzatma → oturumların bitişi ve nft süresi yeni bitişe. Adreslerde yalnızca `kod_id` (kod, TC, pasaport yok);
//! denetimde kod/TC/pasaport yazılmaz. Kişisel veri bakışı `PANEL_MISAFIR_KODLARI`; resepsiyon işi olduğu için
//! gerekçe istenmez (Bağlı kullanıcılar gibi), ama her bakış kaydedilir.

use super::*;
use crate::{kimlik, misafir};

const YOL: &str = "/misafir-kodlari";
const OTURUM_HATA: &str = "Oturum kaydı yazılamadı; servisleri yeniden başlatın.";

/// Geçerlilik seçeneğinin adı.
fn sure_adi(saat: u64) -> String {
    match saat {
        168 => "1 hafta".into(),
        336 => "2 hafta".into(),
        720 => "1 ay (30 gün)".into(),
        s => format!("{s} saat"),
    }
}

/// `GG.AA.YYYY SS:DD` (TR saati).
pub(super) fn tarih_saat(ts: f64) -> String {
    ortak::tarih_goster(&ortak::now_iso(ts))
}

fn tur_kisa(anahtar: &str) -> &'static str {
    if anahtar.starts_with("TC:") { "tc" } else { "pasaport" }
}

/// Portal düğmesinin adı (TR, EN): otel ya da yurt.
fn dugme_adi(cfg: &Config) -> (&'static str, &'static str) {
    if crate::ayar::isletme_turu(cfg) == "yurt" { ("Yurt sakiniyim", "Dormitory resident") } else { ("Otel misafiriyim", "Hotel guest") }
}

/// Formdan gelen ve hata olursa geri doldurulan değerler.
#[derive(Default)]
struct FormDeger {
    ad: String,
    soyad: String,
    tur: String,
    tc: String,
    pasaport: String,
    uyruk: String,
    oda: String,
    sure: u64,
}

impl FormDeger {
    fn al(f: &Form) -> FormDeger {
        let g = |k: &str| f.get(k).map_or("", String::as_str).trim().to_string();
        FormDeger {
            ad: g("ad"),
            soyad: g("soyad"),
            tur: g("tur"),
            tc: g("tc"),
            pasaport: g("pasaport"),
            uyruk: g("uyruk"),
            oda: g("oda"),
            sure: g("sure").parse().unwrap_or(0),
        }
    }

    /// Doğrulanmış (kişi anahtarı, ad, soyad, oda, süre) ya da hata listesi.
    fn dogrula(&self) -> Result<(String, String, String, String, u64), Vec<&'static str>> {
        let mut hata = Vec::new();
        let ad = ortak::clean_name(&self.ad);
        let soyad = ortak::clean_name(&self.soyad);
        if ad.is_none() {
            hata.push("Ad 2-40 karakter olmalı; harf, boşluk, - ' . kullanılabilir.");
        }
        if soyad.is_none() {
            hata.push("Soyad 2-40 karakter olmalı; harf, boşluk, - ' . kullanılabilir.");
        }
        let oda_ok = (1..=10).contains(&self.oda.len()) && self.oda.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !oda_ok {
            hata.push("Oda numarası 1-10 karakter olmalı; harf, rakam ve - kullanılabilir.");
        }
        if !misafir::SURELER.contains(&self.sure) {
            hata.push("Geçerlilik süresini seçin.");
        }
        let anahtar = match self.tur.as_str() {
            "tc" => {
                let t: String = self.tc.chars().filter(|c| !c.is_whitespace()).collect();
                if kimlik::tc_gecerli(&t) {
                    Some(kimlik::Kimlik::Tc(t).anahtar())
                } else {
                    hata.push("T.C. kimlik numarası geçersiz.");
                    None
                }
            }
            "pasaport" => {
                let no = kimlik::pasaport_normal(&self.pasaport);
                let uyruk_ok = crate::ulkeler::ULKELER.iter().any(|(i, ..)| *i == self.uyruk);
                if no.is_none() {
                    hata.push("Pasaport numarası 5-20 harf ya da rakam olmalı.");
                }
                if !uyruk_ok {
                    hata.push("Uyruğu seçin.");
                }
                no.filter(|_| uyruk_ok).map(|no| kimlik::Kimlik::Pasaport { uyruk: self.uyruk.clone(), no }.anahtar())
            }
            _ => {
                hata.push("Kimlik türünü seçin (T.C. vatandaşı ya da yabancı).");
                None
            }
        };
        match (anahtar, ad, soyad) {
            (Some(k), Some(a), Some(s)) if hata.is_empty() => Ok((k, a, s, self.oda.clone(), self.sure)),
            _ => Err(hata),
        }
    }
}

impl Panel {
    /// `/misafir-kodlari…` istekleri; misafir kodu kapalıyken hepsi 404.
    pub(super) fn misafir_kodlari(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        if !crate::ayar::misafir_kodu_acik(cfg) {
            return text(404, "Sayfa bulunamadı");
        }
        match (req.method.as_str(), req.path.strip_prefix(YOL).unwrap_or("-")) {
            ("GET", "") => self.misafir_sayfa(cfg, req, o, now, &FormDeger { tur: "tc".into(), sure: 24, ..Default::default() }, &[]),
            ("POST", "/uret") => self.misafir_uret(cfg, req, o, now),
            ("POST", "/iptal") => self.misafir_iptal(cfg, req, o, now),
            ("POST", "/uzat") => self.misafir_uzat(cfg, req, o, now),
            ("GET", "/fis") => self.misafir_fis(cfg, req, o, now),
            _ => text(404, "Sayfa bulunamadı"),
        }
    }

    fn misafir_sayfa(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64, f: &FormDeger, hatalar: &[&str]) -> Resp {
        let (kodlar, oku_hata) = match misafir::oku_kesin(&cfg.main.state_root) {
            Ok(k) => (k, String::new()),
            Err(e) => (Vec::new(), format!("<div class=\"mesaj hata\">{} Hizmet sağlayıcınıza başvurun.</div>", h(&e))),
        };
        let mut aktif: Vec<&misafir::Kod> = kodlar.iter().filter(|k| k.aktif(now)).collect();
        aktif.sort_by(|a, b| b.uretim.total_cmp(&a.uretim));
        self.audit(cfg, req, Some(o), "PANEL_MISAFIR_KODLARI", &format!("aktif={}", aktif.len()));
        let rows: Vec<Vec<String>> = aktif
            .iter()
            .map(|k| {
                let uzat: String = misafir::SURELER
                    .iter()
                    .map(|s| post_button(o, "/misafir-kodlari/uzat", &format!("+{}", sure_adi(*s)), &[("id", &k.kod_id), ("sure", &s.to_string())], "ikincil"))
                    .collect();
                vec![
                    h(&format!("{} {}", k.ad, k.soyad)),
                    h(&k.oda),
                    if tur_kisa(&k.kimlik) == "tc" { "T.C.".into() } else { "Yabancı".into() },
                    format!("<code>{}</code>", h(&kimlik::maskele(&k.kimlik))),
                    h(&tarih_saat(k.bitis)),
                    format!(
                        "<div class=\"eylemler\"><a class=\"dugme ikincil\" href=\"/misafir-kodlari/fis?id={}\">Fişi göster</a>\
                         <details class=\"yavas\"><summary class=\"dugme ikincil\">Süre uzat</summary><div class=\"yavas-menu\">\
                         <span class=\"not\">Bitişin üzerine eklenir</span>{uzat}</div></details>{}</div>",
                        pct(&k.kod_id),
                        post_button(o, "/misafir-kodlari/iptal", "İptal et", &[("id", &k.kod_id)], "ikincil")
                    ),
                ]
            })
            .collect();
        let hata = if hatalar.is_empty() {
            String::new()
        } else {
            format!("<div class=\"mesaj hata\">{}</div>", hatalar.iter().map(|x| h(x)).collect::<Vec<_>>().join("<br>"))
        };
        let secili = |a: &str, b: &str| if a == b { " checked" } else { "" };
        let uyruklar: String = std::iter::once(format!("<option value=\"\"{}>— Seçin —</option>", if f.uyruk.is_empty() { " selected" } else { "" }))
            .chain(crate::ulkeler::ULKELER.iter().map(|(iso, _, ad)| {
                format!("<option value=\"{iso}\"{}>{}</option>", if *iso == f.uyruk { " selected" } else { "" }, h(ad))
            }))
            .collect();
        let sureler: String = misafir::SURELER
            .iter()
            .map(|s| format!("<option value=\"{s}\"{}>{}</option>", if *s == f.sure { " selected" } else { "" }, h(&sure_adi(*s))))
            .collect();
        let form = format!(
            "{hata}<form class=\"kart\" method=\"post\" action=\"/misafir-kodlari/uret\" autocomplete=\"off\">{}\
             <div class=\"satir\"><div><label for=\"mk-ad\">Ad</label><input type=\"text\" id=\"mk-ad\" name=\"ad\" maxlength=\"40\" value=\"{}\" required></div>\
             <div><label for=\"mk-soyad\">Soyad</label><input type=\"text\" id=\"mk-soyad\" name=\"soyad\" maxlength=\"40\" value=\"{}\" required></div>\
             <div><label for=\"mk-oda\">Oda numarası</label><input type=\"text\" id=\"mk-oda\" name=\"oda\" maxlength=\"10\" pattern=\"[A-Za-z0-9\\-]{{1,10}}\" value=\"{}\" required></div>\
             <div><label for=\"mk-sure\">Geçerlilik</label><select id=\"mk-sure\" name=\"sure\">{sureler}</select></div></div>\
             <fieldset class=\"kart\"><legend>Kimlik</legend>\
             <label class=\"secim\"><input type=\"radio\" name=\"tur\" value=\"tc\"{}> T.C. vatandaşı</label>\
             <label class=\"secim\"><input type=\"radio\" name=\"tur\" value=\"pasaport\"{}> Yabancı</label>\
             <div class=\"satir\"><div><label for=\"mk-tc\">T.C. kimlik no (T.C. vatandaşı)</label><input type=\"text\" id=\"mk-tc\" name=\"tc\" inputmode=\"numeric\" maxlength=\"11\" value=\"{}\"></div>\
             <div><label for=\"mk-pasaport\">Pasaport no (yabancı)</label><input type=\"text\" id=\"mk-pasaport\" name=\"pasaport\" maxlength=\"20\" value=\"{}\"></div>\
             <div><label for=\"mk-uyruk\">Uyruk (yabancı)</label><select id=\"mk-uyruk\" name=\"uyruk\">{uyruklar}</select></div></div></fieldset>\
             <div class=\"eylemler\" style=\"margin-top:20px\"><button>Kod üret</button></div></form>",
            csrf_input(o),
            h(&f.ad),
            h(&f.soyad),
            h(&f.oda),
            secili(&f.tur, "tc"),
            secili(&f.tur, "pasaport"),
            h(&f.tc),
            h(&f.pasaport),
        );
        let body = format!(
            "{oku_hata}<section class=\"bolum\" id=\"kod-uret\" aria-labelledby=\"b-kod-uret\"><h2 class=\"bolum-baslik\" id=\"b-kod-uret\">Kod üret</h2>\
             <p class=\"not\">Misafir, kimlik bilgisi ve bu kodla giriş sayfasından internete bağlanır; bağlantısı kodun bitişinde kapanır. \
             Aynı kişiye yeni kod üretilirse eski kodu iptal edilir. Her üretim ve bakış kaydedilir.</p>{form}</section>\
             <section class=\"bolum\" id=\"aktif-kodlar\" aria-labelledby=\"b-aktif-kodlar\"><h2 class=\"bolum-baslik\" id=\"b-aktif-kodlar\">Aktif kodlar</h2>{}</section>",
            table(&["Ad soyad", "Oda", "Kimlik türü", "Kimlik", "Bitiş", ""], &rows, "Aktif kod yok")
        );
        self.page(cfg, req, Some(o), "Misafir kodları", &body)
    }

    /// Kişinin (kişi anahtarı = `Session.phone`) açık oturumlarını kapatır; çağıran state_lock tutar.
    /// (kapanan sayısı, sessions.json yazıldı mı): yazılamazsa `ctl yukle` kapanan oturumu geri getirebilir → hata gösterilir.
    fn kimlik_oturumlari_kapat(&self, cfg: &Config, anahtar: &str, now: f64) -> (usize, bool) {
        let m = &cfg.main;
        let mut ses = ortak::load_sessions(&m.state_root);
        let macler: Vec<String> = ses.iter().filter(|(_, s)| s.phone == anahtar).map(|(mac, _)| mac.clone()).collect();
        for mac in &macler {
            ortak::close_session(&m.log_root, &mut ses, mac, "kod_iptal", now, &*self.runner);
        }
        let yazildi = macler.is_empty() || ortak::save_sessions(&m.state_root, &ses).is_ok();
        (macler.len(), yazildi)
    }

    fn misafir_uret(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let f = FormDeger::al(&req.form);
        let (anahtar, ad, soyad, oda, sure) = match f.dogrula() {
            Ok(v) => v,
            Err(hatalar) => {
                let mut r = self.misafir_sayfa(cfg, req, o, now, &f, &hatalar);
                r.status = 400;
                return r;
            }
        };
        let m = &cfg.main;
        let (yeni, eski, (kapanan, yazildi)) = {
            let Ok(_g) = ortak::state_lock(&m.state_root) else { return redirect(YOL, Some(("Kilit alınamadı.", true))) };
            let mut kodlar = match misafir::oku_kesin(&m.state_root) {
                Ok(k) => k,
                Err(e) => return redirect(YOL, Some((&format!("{e} Kod üretilmedi; hizmet sağlayıcınıza başvurun."), true))),
            };
            misafir::temizle(&mut kodlar, now);
            let (yeni, eski) = misafir::uret(&mut kodlar, &anahtar, &ad, &soyad, &oda, sure, &o.user, now);
            if misafir::kaydet(&m.state_root, &kodlar).is_err() {
                return redirect(YOL, Some(("Kaydedilemedi; kod üretilmedi.", true)));
            }
            let kapanan = if eski.is_some() { self.kimlik_oturumlari_kapat(cfg, &anahtar, now) } else { (0, true) };
            (yeni, eski, kapanan)
        };
        self.audit(
            cfg,
            req,
            Some(o),
            "PANEL_MISAFIR_KODU",
            &format!(
                "islem=uret kod_id={} oda={} bitis={} tur={} sure={sure}{}{}",
                yeni.kod_id,
                yeni.oda,
                ortak::now_iso(yeni.bitis),
                tur_kisa(&anahtar),
                eski.as_ref().map_or(String::new(), |e| format!(" onceki_iptal={} kapanan={kapanan}", e.kod_id)),
                if yazildi { "" } else { " hatali=1" }
            ),
        );
        let msg = match (eski.is_some(), yazildi) {
            (_, false) => format!("Kod üretildi, önceki kodu iptal edildi; ama {OTURUM_HATA}"),
            (true, true) => "Kod üretildi. Bu kişinin önceki kodu iptal edildi.".into(),
            (false, true) => "Kod üretildi.".into(),
        };
        Resp {
            status: 303,
            body: String::new(),
            headers: vec![("Location".into(), format!("{YOL}/fis?id={}&m={}{}", pct(&yeni.kod_id), pct(&msg), if yazildi { "" } else { "&e=1" }))],
            file: None,
        }
    }

    fn misafir_iptal(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let id = req.form.get("id").map_or("", String::as_str);
        let m = &cfg.main;
        let (k, (kapanan, yazildi)) = {
            let Ok(_g) = ortak::state_lock(&m.state_root) else { return redirect(YOL, Some(("Kilit alınamadı.", true))) };
            let mut kodlar = match misafir::oku_kesin(&m.state_root) {
                Ok(k) => k,
                Err(e) => return redirect(YOL, Some((&format!("{e} Hiçbir kod değiştirilmedi; hizmet sağlayıcınıza başvurun."), true))),
            };
            if !kodlar.iter().any(|k| k.kod_id == id && k.aktif(now)) {
                return redirect(YOL, Some(("Kod bulunamadı ya da süresi dolmuş.", true)));
            }
            let Some(k) = misafir::iptal(&mut kodlar, id) else { return redirect(YOL, Some(("Kod bulunamadı.", true))) };
            if misafir::kaydet(&m.state_root, &kodlar).is_err() {
                return redirect(YOL, Some(("Kaydedilemedi; kod iptal edilmedi.", true)));
            }
            // aynı kişinin başka etkin kodu varsa (olmamalı: üretim eskisini iptal eder) oturumlar ona aittir
            let baska = kodlar.iter().any(|x| x.aktif(now) && x.kimlik == k.kimlik);
            let kapanan = if baska { (0, true) } else { self.kimlik_oturumlari_kapat(cfg, &k.kimlik, now) };
            (k, kapanan)
        };
        self.audit(
            cfg,
            req,
            Some(o),
            "PANEL_MISAFIR_KODU",
            &format!(
                "islem=iptal kod_id={} oda={} bitis={} tur={} kapanan={kapanan}{}",
                k.kod_id,
                k.oda,
                ortak::now_iso(k.bitis),
                tur_kisa(&k.kimlik),
                if yazildi { "" } else { " hatali=1" }
            ),
        );
        if !yazildi {
            return redirect(YOL, Some((&format!("Kod iptal edildi; ama {OTURUM_HATA}"), true)));
        }
        let msg = if kapanan > 0 { format!("Kod iptal edildi; {kapanan} cihazın bağlantısı kesildi.") } else { "Kod iptal edildi.".into() };
        redirect(YOL, Some((&msg, false)))
    }

    fn misafir_uzat(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let id = req.form.get("id").map_or("", String::as_str);
        let Some(sure) = req.form.get("sure").and_then(|s| s.parse::<u64>().ok()).filter(|s| misafir::SURELER.contains(s)) else {
            return redirect(YOL, Some(("Geçersiz süre.", true)));
        };
        let m = &cfg.main;
        let (k, uzayan, hatali) = {
            let Ok(_g) = ortak::state_lock(&m.state_root) else { return redirect(YOL, Some(("Kilit alınamadı.", true))) };
            let mut kodlar = match misafir::oku_kesin(&m.state_root) {
                Ok(k) => k,
                Err(e) => return redirect(YOL, Some((&format!("{e} Hiçbir kod değiştirilmedi; hizmet sağlayıcınıza başvurun."), true))),
            };
            if !kodlar.iter().any(|k| k.kod_id == id && k.aktif(now)) {
                return redirect(YOL, Some(("Kod bulunamadı ya da süresi dolmuş.", true)));
            }
            let Some(bitis) = misafir::uzat(&mut kodlar, id, sure, now) else { return redirect(YOL, Some(("Kod bulunamadı.", true))) };
            if misafir::kaydet(&m.state_root, &kodlar).is_err() {
                return redirect(YOL, Some(("Kaydedilemedi; süre uzatılmadı.", true)));
            }
            let k = kodlar.iter().find(|k| k.kod_id == id).expect("uzatılan kod").clone();
            let mut ses = ortak::load_sessions(&m.state_root);
            let macler: Vec<String> = ses.iter().filter(|(_, s)| s.phone == k.kimlik).map(|(mac, _)| mac.clone()).collect();
            let (mut uzayanlar, mut hatali) = (Vec::new(), 0);
            for mac in &macler {
                if ortak::oturum_uzat(&mut ses, mac, bitis, now, &*self.runner) {
                    uzayanlar.push(mac.clone());
                } else {
                    hatali += 1;
                }
            }
            let mut uzayan = uzayanlar.len();
            // kayıt yazılamazsa nft uzadı ama kaydedici eski bitişte kapatır: hata olarak bildirilir
            if uzayan > 0 && ortak::save_sessions(&m.state_root, &ses).is_err() {
                hatali += uzayan;
                uzayan = 0;
            } else {
                for mac in &uzayanlar {
                    ortak::oturum_uzatildi_yaz(&m.log_root, &ses[mac], mac, now, &o.user);
                }
            }
            (k, uzayan, hatali)
        };
        self.audit(
            cfg,
            req,
            Some(o),
            "PANEL_MISAFIR_KODU",
            &format!(
                "islem=uzat kod_id={} oda={} bitis={} tur={} sure={sure} uzayan={uzayan}{}",
                k.kod_id,
                k.oda,
                ortak::now_iso(k.bitis),
                tur_kisa(&k.kimlik),
                if hatali > 0 { format!(" hatali={hatali}") } else { String::new() }
            ),
        );
        if hatali > 0 {
            let msg = format!("Kod {} tarihine uzatıldı, ama {hatali} bağlı cihazın süresi uzatılamadı ({OTURUM_HATA})", tarih_saat(k.bitis));
            return redirect(YOL, Some((&msg, true)));
        }
        redirect(YOL, Some((&format!("Kod {} tarihine uzatıldı.", tarih_saat(k.bitis)), false)))
    }

    /// Yazdırılabilir fiş: yazdırırken yalnızca fiş görünür (menü, başlık, notlar gizli).
    fn misafir_fis(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let id = req.query.get("id").map_or("", String::as_str);
        let kodlar = match misafir::oku_kesin(&cfg.main.state_root) {
            Ok(k) => k,
            Err(e) => return redirect(YOL, Some((&e, true))),
        };
        let Some(k) = kodlar.iter().find(|k| k.kod_id == id && !k.iptal) else {
            return redirect(YOL, Some(("Kod bulunamadı ya da iptal edilmiş.", true)));
        };
        self.audit(cfg, req, Some(o), "PANEL_MISAFIR_KODLARI", &format!("sayfa=fis kod_id={}", k.kod_id));
        let (tr, en) = dugme_adi(cfg);
        // süresi dolmuş kodun kendisi gösterilmez
        let (dolmus, kod) = if k.aktif(now) {
            (String::new(), h(&k.kod))
        } else {
            ("<p class=\"mesaj hata\">Bu kodun süresi doldu.</p>".to_string(), "Süresi doldu / Expired".to_string())
        };
        let body = format!(
            "<style>.fis{{max-width:26rem;border:1px dashed var(--cizgi2);padding:20px 24px;margin:8px 0 20px}}\
             .fis .isletme{{font-weight:700;font-size:1.0625rem;margin-bottom:12px}}.fis .etiket{{font-size:.8125rem;color:var(--soluk);margin-top:14px}}\
             .fis .kod{{font:700 2.75rem/1.15 var(--es);letter-spacing:.18em}}.fis p{{margin:14px 0 0;font-size:.875rem}}\
             @media print{{.ust,.yan,h1,.mesaj,.menu-dugme,.yazdirma-disi{{display:none !important}}.kabuk,main{{display:block !important;margin:0 !important;padding:0 !important;max-width:none !important}}\
             .fis{{border:1px solid #000;margin:0}}}}</style>\
             {dolmus}<p class=\"not yazdirma-disi\">Yazdırmak için Ctrl+P. <a href=\"{YOL}\">Misafir kodlarına dön</a></p>\
             <div class=\"fis\"><div class=\"isletme\">{}</div>\
             <div>Misafir / Guest: <b>{}</b></div><div>Oda / Room: <b>{}</b></div>\
             <div class=\"etiket\">Wi-Fi giriş kodu / Wi-Fi access code</div><div class=\"kod\">{kod}</div>\
             <div>Son geçerlilik / Valid until: <b>{}</b></div>\
             <p>Wi-Fi'ye bağlanın, açılan sayfada '{tr}'e dokunun; kimlik bilginizi ve bu kodu girin.</p>\
             <p lang=\"en\">Connect to the Wi-Fi, tap '{en}' on the page that opens, then enter your ID details and this code.</p></div>",
            h(&cfg.main.site_name),
            h(&format!("{} {}", k.ad, k.soyad)),
            h(&k.oda),
            h(&tarih_saat(k.bitis)),
        );
        self.page(cfg, req, Some(o), "Misafir kodu fişi", &body)
    }
}
