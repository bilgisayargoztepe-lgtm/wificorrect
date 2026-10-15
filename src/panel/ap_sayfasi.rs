//! Panel → Admin → AP kurulumu (yalnızca hizmet sağlayıcı; spec 2026-10-15-ap-kurulum): cihazın misafir portu bilgisi,
//! ağdaki cihazlar (ayrı sayfa, denetlenir) ve fabrika adresli AP'ye ulaşmak için 30 dk'lık geçici adres.

use super::*;
use crate::ap;

impl Panel {
    pub(super) fn ap_kurulum(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let m = &cfg.main;
        let mac = std::fs::read_to_string(self.sys.join(&m.iface).join("address")).unwrap_or_default().trim().to_string();
        let port = facts(&[
            ("Adres", format!("<code>{}</code>", h(&m.router_ip))),
            ("MAC", if mac.is_empty() { "bilinmiyor".into() } else { format!("<code>{}</code>", h(&mac)) }),
        ]);
        let gecici = match ap::durum(&self.ap_yollar, cfg, &*self.ip_oku) {
            Some((g, true)) => {
                let kalan = ((g.bitis - now).max(0.0) / 60.0).ceil() as u64;
                let saat = ortak::now_iso(g.bitis).get(11..16).unwrap_or("").to_string();
                format!(
                    "<p><code>{}</code> açık — {} kendiliğinden kalkar ({kalan} dk).</p>{}",
                    h(&g.adres),
                    h(&saat),
                    post_button(o, "/ap-kurulum/gecici-kaldir", "Şimdi kaldır", &[], "ikincil")
                )
            }
            Some((g, false)) => format!(
                "<p>Geçici adres <code>{}</code> artık yok (ağ yeniden kuruldu).</p>{}",
                h(&g.adres),
                post_button(o, "/ap-kurulum/gecici-kaldir", "Kaldır", &[], "ikincil")
            ),
            None => format!(
                "<form class=\"satir\" method=\"post\" action=\"/ap-kurulum/gecici-ekle\">{}\
                 <div><label for=\"apg\">Geçici adres</label><input type=\"text\" id=\"apg\" name=\"adres\" placeholder=\"192.168.0.250/24\" required></div>\
                 <button>Ekle ({} dk)</button></form>\
                 <p class=\"not\">AP'nin fabrika adresiyle aynı alt ağdan, AP'nin kullanmadığı bir adres yazın. Misafirler etkilenmez; \
                 misafirden bu alt ağa erişim kapalıdır. Adres {} dk sonra (ve cihaz yeniden başlayınca) kendiliğinden kalkar.</p>",
                csrf_input(o),
                ap::SURE_SN / 60,
                ap::SURE_SN / 60
            ),
        };
        let body = format!(
            "<section class=\"kart grup\"><h2>Cihazın misafir portu</h2><div>{port}<p class=\"not\">AP'de \"yönetime yalnızca şu IP/MAC \
             izin ver\" seçeneği varsa bu adres ve MAC yazılır.</p></div></section>\
             <section class=\"kart grup\"><h2>Ağdaki cihazlar</h2><div><p>Misafir portunda son görülen cihazlar (DHCP kiraları ve ARP \
             tablosu); müşterinin verdiği MAC'le eşleştirin.</p><p><a class=\"dugme ikincil\" href=\"/ap-kurulum/cihazlar\">Ağdaki cihazları göster</a></p>\
             <p class=\"not\">Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; önce geçici adres ekleyin.</p></div></section>\
             <section class=\"kart grup\"><h2>Geçici adres</h2><div>{gecici}</div></section>\
             <section class=\"kart grup\"><h2>AP adresleri</h2><div>{kayit}</div></section>",
            kayit = self.ap_adres_karti(cfg, o)
        );
        self.page(cfg, req, Some(o), "AP kurulumu", &body)
    }

    fn ap_adres_karti(&self, cfg: &Config, o: &Oturum) -> String {
        let liste = &cfg.ap.adresler;
        let mevcut = if liste.is_empty() {
            "<p>Kayıtlı AP adresi yok.</p>".to_string()
        } else {
            format!("<p>Kayıtlı: {}</p>", liste.iter().map(|a| format!("<code>{}</code>", h(a))).collect::<Vec<_>>().join(", "))
        };
        format!(
            "{mevcut}<form method=\"post\" action=\"/ap-kurulum/adresler\">{}\
             <label for=\"apa\">AP yönetim adresleri (satır başına bir adres, en çok {})</label>\
             <textarea id=\"apa\" name=\"adresler\" rows=\"3\" maxlength=\"200\">{}</textarea>\
             <p><button>Kaydet</button></p></form>\
             <p class=\"not\">AP ayarlandıktan sonra kalıcı yönetim adresini yazın (ör. 10.50.0.2; misafir ağında, DHCP havuzu dışında). \
             Kayıt bilgi içindir; ağdaki cihazlar listesinde işaretlenir. Başka porttaki misafirlerden AP'ye giden trafik cihazın \
             köprü yalıtımıyla zaten düşer; AP'ye kablosuz bağlı misafirleri yalnızca AP'nin kendi ayarı korur (kablosuzdan \
             yönetim kapalı ya da yönetim yalnızca {}).</p>",
            csrf_input(o),
            ap::EN_COK,
            h(&liste.join("\n")),
            h(&cfg.main.router_ip)
        )
    }

    /// AP adresleri kaydı (`[ap] adresler`); güvenlik duvarına dokunmaz. Denetim `PANEL_AP_ADRES adresler=… eski=…`.
    pub(super) fn ap_adres_kaydet(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let girdi = req.form.get("adresler").map_or("", String::as_str);
        let yeni = match ap::kayit_dogrula(girdi, cfg) {
            Ok(v) => v,
            Err(e) => return redirect("/ap-kurulum", Some((&e, true))),
        };
        if yeni == cfg.ap.adresler {
            return redirect("/ap-kurulum", Some(("Değişiklik yok.", false)));
        }
        let mut c = cfg.clone();
        let eski = std::mem::replace(&mut c.ap.adresler, yeni);
        if let Err(e) = c.save(&self.cfg_path) {
            return redirect("/ap-kurulum", Some((&e, true)));
        }
        let yaz = |v: &[String]| if v.is_empty() { "-".to_string() } else { v.join(",") };
        self.audit(&c, req, Some(o), "PANEL_AP_ADRES", &format!("adresler={} eski={}", yaz(&c.ap.adresler), yaz(&eski)));
        redirect("/ap-kurulum", Some(("Kaydedildi.", false)))
    }

    /// Ağdaki cihazlar: cihaz adları kişisel veri olabilir → her açılış denetime (`PANEL_AP_CIHAZLAR`).
    pub(super) fn ap_cihazlar(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let m = &cfg.main;
        let kiralar = std::fs::read_to_string(&m.leases_file).unwrap_or_default();
        let neigh = (self.ip_oku)(&["ip", "-4", "neigh", "show", "dev", &m.iface]);
        let liste = ap::komsular(&kiralar, &neigh);
        self.audit(cfg, req, Some(o), "PANEL_AP_CIHAZLAR", &format!("sayi={}", liste.len()));
        let izinli = Config::devices(&cfg.allow);
        let rows: Vec<Vec<String>> = liste
            .iter()
            .map(|k| {
                let ipler: Vec<String> = k.ipler.iter().map(ToString::to_string).collect();
                let mut kaynak = vec![];
                if k.dhcp {
                    kaynak.push("DHCP".to_string());
                }
                if let Some(a) = &k.arp {
                    kaynak.push(if a.is_empty() { "ARP".into() } else { format!("ARP {a}") });
                }
                let mut not = vec![];
                if ipler.iter().any(|ip| cfg.ap.adresler.contains(ip)) {
                    not.push("kayıtlı AP adresi".to_string());
                }
                if let Some(ad) = izinli.get(&k.mac) {
                    not.push(if ad.is_empty() { "izinli cihaz".to_string() } else { format!("izinli cihaz: {ad}") });
                }
                if k.rastgele {
                    not.push("rastgele MAC — büyük olasılıkla telefon".into());
                }
                vec![format!("<code>{}</code>", h(&k.mac)), h(&ipler.join(", ")), h(&k.ad), h(&kaynak.join(" · ")), h(&not.join("; "))]
            })
            .collect();
        let body = format!(
            "<p><a href=\"/admin#ap-kurulum\">← AP kurulumu</a></p>{}\
             <p class=\"not\">Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; önce geçici adres ekleyin.</p>",
            table(&["MAC", "IP", "Ad", "Kaynak", "Not"], &rows, "Cihaz görülmedi")
        );
        self.page(cfg, req, Some(o), "Ağdaki cihazlar", &body)
    }

    pub(super) fn ap_gecici_ekle(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let girdi = req.form.get("adres").map_or("", String::as_str);
        match ap::ekle(cfg, &self.ap_yollar, girdi, ap::SURE_SN, &*self.ip_oku, &*self.runner, now) {
            Ok(g) => {
                self.audit(cfg, req, Some(o), "AP_KURULUM_ADRES", &format!("eklendi adres={} sure={}", g.adres, ap::SURE_SN));
                let msg = format!("Geçici adres {} eklendi; {} dk sonra kendiliğinden kalkar.", g.adres, ap::SURE_SN / 60);
                redirect("/ap-kurulum", Some((&msg, false)))
            }
            Err(e) => redirect("/ap-kurulum", Some((&e, true))),
        }
    }

    pub(super) fn ap_gecici_kaldir(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        match ap::kaldir(cfg, &self.ap_yollar, "elle", None, &*self.ip_oku, &*self.runner) {
            Ok(Some(a)) => {
                self.audit(cfg, req, Some(o), "AP_KURULUM_ADRES", &format!("kaldirildi adres={a} neden=elle"));
                redirect("/ap-kurulum", Some((&format!("Geçici adres {a} kaldırıldı."), false)))
            }
            Ok(None) => redirect("/ap-kurulum", Some(("Açık geçici adres yok.", false))),
            Err(e) => redirect("/ap-kurulum", Some((&e, true))),
        }
    }
}
