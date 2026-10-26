//! Panel: giriş sayfasındaki Aydınlatma, Açık rıza ve İnternet Kullanıcı Sözleşmesi metinleri (iki rol de yazar).
//! Düz metin; boş satır paragraf ayırır. Kaydedince portal yeniden başlar (bağlı müşteriler düşmez).
//! Her metin giriş sayfasında göründüğü gibi, Ayarlar → İşletme'deki bilgilerle doldurulmuş gösterilir (`portal::metin_doldur`;
//! işletme sahibi adını ve unvanını yazınca metinlerde görmeli). Düzenleme kutusunda şablon durur (İŞLETMECİ ve köşeli parantezli
//! yer tutucular): bilgiler sonradan değişince metinler de kendiliğinden değişir.

use super::*;

const EN_FAZLA: usize = 20_000;
/// (anahtar, başlık, giriş sayfasında nerede görünür, düzenleme notu)
const METINLER: &[(&str, &str, &str, &str)] = &[
    ("aydinlatma", "KVKK Aydınlatma Metni", "Giriş sayfasında \"KVKK Aydınlatma Metni\" bağlantısıyla açılır; onay kutusu yoktur.", "\"* \" ile başlayan satırlar madde listesi olur. Veri sorumlusu bilgileri Ayarlar → İşletme'den gelir: [vergi levhası unvanı], [başvuru adresi], [irtibat numarası], [e-posta adresi], [KEP adresi]; isteğe bağlı bilgi boşsa geçtiği satır gösterilmez."),
    ("acik_riza", "Açık Rıza Metni", "Giriş sayfasında onay kutusunun yanında yazar; işaretlemek zorunludur.", "[vergi levhası unvanı] yerine Ayarlar → İşletme'deki unvan yazılır."),
    ("sozlesme", "İnternet Kullanıcı Sözleşmesi", "Müşteri \"okudum, kabul ediyorum\" kutusunu işaretlemeden devam edemez.", ""),
    ("aydinlatma_en", "KVKK Aydınlatma Metni (İngilizce)", EN_NOT, EN_DUZENLEME),
    ("acik_riza_en", "Açık Rıza Metni (İngilizce)", EN_NOT, EN_DUZENLEME),
    ("sozlesme_en", "İnternet Kullanıcı Sözleşmesi (İngilizce)", EN_NOT, EN_DUZENLEME),
];
const EN_NOT: &str = "Giriş sayfasında EN seçildiğinde gösterilir; boş bırakılırsa Türkçesi gösterilir.";
const EN_DUZENLEME: &str = "İŞLETMECİ yalnızca işletme adıyla değişir (ek eklenmez); köşeli parantezli yerler Türkçedeki gibi dolar.";

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

/// Metnin giriş sayfasındaki hali (HTML): işletme bilgileriyle doldurulmuş, giriş sayfasının biçimiyle (açık rıza onay kutusunun
/// yanında satır satır, öbürleri paragraf ve madde listesi). Boş İngilizce metin yerine giriş sayfası Türkçesini gösterir.
fn onizleme(cfg: &Config, key: &str) -> String {
    let ham = field(cfg, key);
    let ingilizce = key.ends_with("_en");
    if ingilizce && ham.trim().is_empty() {
        return "<p class=\"not\">Boş: giriş sayfasında Türkçesi gösterilir.</p>".into();
    }
    let dolu = crate::portal::metin_doldur(ham, &cfg.main, ingilizce);
    if key.starts_with("acik_riza") {
        format!("<p>{}</p>", h(&dolu).replace('\n', "<br>"))
    } else {
        crate::portal::metin_html_dil(&dolu, crate::dil::Dil::Tr)
    }
}

impl Panel {
    pub(super) fn metinler(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        // örnek, giriş sayfasındaki gerçek ek hesabıyla (Göztepe Bilgisayar’ın, Bocafe’nin, Starbucks’un)
        let ornek = crate::portal::isletme_yerlestir("İŞLETMECİ’nin", &cfg.main.site_name);
        let mut body = format!(
            "<p class=\"not\">Metinler giriş sayfasında göründüğü gibi, Ayarlar → İşletme'deki bilgilerle doldurulmuş gösterilir. \
             Değiştirmek için \"Metni düzenle\"yi açın: düzenleme kutusunda <b>İŞLETMECİ</b> işletme adıyla (şu an: <b>{}</b>; ekleri ada \
             göre ayarlanır, İŞLETMECİ’nin → {} gibi), köşeli parantezli yerler işletme bilgileriyle değişir. Bu yerleri silmeyin; \
             işletme bilgileri değişince metinler de kendiliğinden değişir.</p><form method=\"post\" action=\"/portal-metinleri\">{}",
            h(&cfg.main.site_name),
            h(&ornek),
            csrf_input(o)
        );
        for (key, title, nerede, duzenleme) in METINLER {
            let not = if duzenleme.is_empty() { String::new() } else { format!("<p class=\"not\">{}</p>", h(duzenleme)) };
            body.push_str(&format!(
                "<section class=\"kart\"><h2>{}</h2><p class=\"not\">{}</p><div class=\"metin-onizleme\">{}</div>\
                 <details class=\"metin-duzenle\"><summary>Metni düzenle</summary>{not}\
                 <textarea id=\"{key}\" name=\"{key}\" rows=\"10\" maxlength=\"{EN_FAZLA}\" aria-label=\"{} (düzenleme)\">{}</textarea>\
                 </details></section>",
                h(title),
                h(nerede),
                onizleme(cfg, key),
                h(title),
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
        for (key, title, ..) in METINLER {
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
