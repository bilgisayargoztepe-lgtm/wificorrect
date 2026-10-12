//! Panel: giriş sayfasındaki Aydınlatma, Açık rıza ve İnternet Kullanıcı Sözleşmesi metinleri (iki rol de yazar).
//! Düz metin; boş satır paragraf ayırır. Kaydedince portal yeniden başlar (bağlı müşteriler düşmez).

use super::*;

const EN_FAZLA: usize = 20_000;
const METINLER: &[(&str, &str, &str)] = &[
    ("aydinlatma", "KVKK Aydınlatma Metni", "Giriş sayfasında \"KVKK Aydınlatma Metni\" bağlantısıyla açılır; onay kutusu yoktur. \"* \" ile başlayan satırlar madde listesi olur."),
    ("acik_riza", "Açık Rıza Metni", "Giriş sayfasında onay kutusunun yanında yazar; işaretlemek zorunludur. [vergi levhası unvanı] yerine Ayarlar'daki işletme unvanı yazılır."),
    ("sozlesme", "İnternet Kullanıcı Sözleşmesi", "Müşteri \"okudum, kabul ediyorum\" kutusunu işaretlemeden devam edemez."),
    ("aydinlatma_en", "KVKK Aydınlatma Metni (İngilizce)", EN_NOT),
    ("acik_riza_en", "Açık Rıza Metni (İngilizce)", EN_NOT),
    ("sozlesme_en", "İnternet Kullanıcı Sözleşmesi (İngilizce)", EN_NOT),
];
const EN_NOT: &str = "Giriş sayfasında EN seçildiğinde gösterilir; boş bırakılırsa Türkçesi gösterilir. İŞLETMECİ yalnızca işletme adıyla değişir (ek eklenmez).";

fn field<'a>(cfg: &'a Config, key: &str) -> &'a str {
    match key {
        "aydinlatma" => &cfg.portal.aydinlatma,
        "acik_riza" => &cfg.portal.acik_riza,
        "aydinlatma_en" => &cfg.portal.aydinlatma_en,
        "acik_riza_en" => &cfg.portal.acik_riza_en,
        "sozlesme_en" => &cfg.portal.sozlesme_en,
        _ => &cfg.portal.sozlesme,
    }
}

impl Panel {
    pub(super) fn metinler(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let mut body = format!(
            "<p class=\"not\">Metinlerde geçen <b>İŞLETMECİ</b> kelimesi giriş sayfasında işletme adıyla (şu an: <b>{}</b>) değişir; ekleri ada göre ayarlanır (İŞLETMECİ’nin → {}’nin gibi).</p><form method=\"post\" action=\"/portal-metinleri\">{}",
            h(&cfg.main.site_name),
            h(&cfg.main.site_name),
            csrf_input(o)
        );
        for (key, title, note) in METINLER {
            body.push_str(&format!(
                "<section class=\"kart\"><h2><label for=\"{key}\" style=\"margin:0\">{}</label></h2><p class=\"not\">{}</p>\
                 <textarea id=\"{key}\" name=\"{key}\" rows=\"10\" maxlength=\"{EN_FAZLA}\">{}</textarea></section>",
                h(title),
                h(note),
                h(field(cfg, key))
            ));
        }
        body.push_str(
            "<div class=\"kaydet\" style=\"padding-left:0\"><button>Kaydet</button><p class=\"not\">Kaydedince giriş sayfası yeniden başlatılır; \
             bağlı müşteriler düşmez.</p></div></form>",
        );
        self.page(cfg, req, Some(o), "Portal metinleri", &body)
    }

    pub(super) fn metinler_kaydet(&self, mut cfg: Config, req: &Req, o: &Oturum) -> Resp {
        let mut changed = vec![];
        for (key, title, _) in METINLER {
            let v = req.form.get(*key).map_or("", String::as_str).replace("\r\n", "\n").trim().to_string();
            if v.chars().count() > EN_FAZLA || v.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                return redirect("/portal-metinleri", Some((&format!("{title} geçersiz (en fazla {EN_FAZLA} karakter)."), true)));
            }
            if v != field(&cfg, key) {
                changed.push(format!("{key}({} karakter)", v.chars().count()));
                match *key {
                    "aydinlatma" => cfg.portal.aydinlatma = v,
                    "acik_riza" => cfg.portal.acik_riza = v,
                    "aydinlatma_en" => cfg.portal.aydinlatma_en = v,
                    "acik_riza_en" => cfg.portal.acik_riza_en = v,
                    "sozlesme_en" => cfg.portal.sozlesme_en = v,
                    _ => cfg.portal.sozlesme = v,
                }
            }
        }
        if changed.is_empty() {
            return redirect("/portal-metinleri", Some(("Değişiklik yok.", false)));
        }
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect("/portal-metinleri", Some((&e, true)));
        }
        self.audit(&cfg, req, Some(o), "PANEL_PORTAL_METIN", &changed.join(" "));
        let ok = (self.runner)(&cmd(&["systemctl", "restart", "wificorrect-portal"]));
        redirect("/portal-metinleri", Some(if ok { ("Kaydedildi; giriş sayfasında görünüyor.", false) } else { ("Kaydedildi ama giriş sayfası yeniden başlatılamadı!", true) }))
    }
}
