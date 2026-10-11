//! Gerekçe zorunluluğu (2026-10-03, kullanıcı kararı): kafe sahibi müşterilerin kişisel verisine (kayıtlar, kullanıcılar,
//! resmi talep) bakmadan önce gerekçe yazar. Gerekçe 30 dk geçerli; o süredeki her bakışın denetim satırına eklenir.
//! Admin muaf. Bağlı cihazlar gerekçesiz açılır (bağlantı kesmek için gerekli) ama yine kaydedilir.

use super::*;

pub(super) const SURE_SN: f64 = 30.0 * 60.0;
const ONERILER: &[&str] = &["Emniyet / savcılık talebi", "Müşteri şikâyeti", "Müşterinin kendi talebi", "Teknik sorun incelemesi"];

/// Gerekçe isteyen sayfa mı (yalnızca kafe sahibi için).
/// Kimlikle aramalar (kullanıcılar, resmi talep, satır araması) POST'tur: gerekçe onlarda da sorulur; dönüş adresine
/// aranan değer girmez (yalnızca yol).
pub(super) fn needs_reason(req: &Req) -> bool {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/kayitlar/gun" | "/kayitlar/dosya" | "/kayitlar/indir" | "/kayitlar/gun-indir" | "/kullanicilar" | "/kullanici" | "/talep/paket") => true,
        ("GET", "/ara") => req.query.get("q").is_some_and(|t| !t.trim().is_empty()),
        ("POST", "/kullanicilar" | "/talep" | "/kayitlar/dosya") => true, // boş talep formu (GET) gerekçesiz açılır, arama gerekçe ister
        _ => false,
    }
}

/// Dönülecek adres (yalnızca bu paneldeki bir yol; başka siteye yönlendirme yok).
fn return_path(req: &Req) -> String {
    let mut q: Vec<(&String, &String)> = req.query.iter().collect();
    q.sort();
    let qs: Vec<String> = q.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect();
    if qs.is_empty() { req.path.clone() } else { format!("{}?{}", req.path, qs.join("&")) }
}

fn safe_return(p: &str) -> &str {
    if p.starts_with('/') && !p.starts_with("//") && !p.contains(['\\', '\r', '\n']) { p } else { "/" }
}

impl Panel {
    pub(super) fn gerekce_form(&self, cfg: &Config, req: &Req, o: &Oturum, donus: &str, err: &str) -> Resp {
        let opts: String = ONERILER.iter().map(|x| format!("<option value=\"{}\">", h(x))).collect();
        let err = if err.is_empty() { String::new() } else { format!("<p class=\"hata\">{}</p>", h(err)) };
        let body = format!(
            "<div class=\"kart dar\"><p>Müşterilerin kişisel verisine (kayıtlar, kullanıcılar, resmi talep) bakmak için gerekçe yazın.</p>\
             <p class=\"not\">Gerekçe {} dakika geçerlidir. Bu süredeki her görüntüleme, arama ve indirme gerekçeyle birlikte kaydedilir \
             ve hizmet sağlayıcı tarafından denetlenir.</p>{err}\
             <form method=\"post\" action=\"/gerekce\">{}<input type=\"hidden\" name=\"donus\" value=\"{}\">\
             <label for=\"g\">Gerekçe</label><input type=\"text\" id=\"g\" name=\"gerekce\" list=\"oneriler\" minlength=\"10\" maxlength=\"200\" required autofocus>\
             <datalist id=\"oneriler\">{opts}</datalist>\
             <div class=\"eylemler\" style=\"margin-top:20px\"><button>Devam</button><a class=\"dugme ikincil\" href=\"/\">Vazgeç</a></div></form></div>",
            (SURE_SN / 60.0) as u32,
            csrf_input(o),
            h(donus),
        );
        self.page(cfg, req, Some(o), "Gerekçe gerekli", &body)
    }

    /// Kafe sahibi geçerli gerekçe olmadan kişisel veri sayfası isterse gerekçe ekranı.
    pub(super) fn reason_gate(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Option<Resp> {
        let valid = o.gerekce.as_ref().is_some_and(|(_, until)| *until > now);
        (o.rol == Rol::Sahip && needs_reason(req) && !valid).then(|| self.gerekce_form(cfg, req, o, &return_path(req), ""))
    }

    pub(super) fn gerekce_post(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let donus = safe_return(req.form.get("donus").map_or("/", String::as_str)).to_string();
        let g: String = req.form.get("gerekce").map_or("", String::as_str).split_whitespace().collect::<Vec<_>>().join(" ");
        let n = g.chars().count();
        if !(10..=200).contains(&n) {
            return self.gerekce_form(cfg, req, o, &donus, "Gerekçe 10-200 karakter olmalı.");
        }
        if let Some(t) = &req.token {
            self.oturumlar.set_gerekce(t, &g, now + SURE_SN);
        }
        self.audit(cfg, req, Some(o), "PANEL_GEREKCE", &format!("gerekce={g}"));
        redirect(&donus, None)
    }
}
