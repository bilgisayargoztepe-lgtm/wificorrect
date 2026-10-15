//! Panel → Ayarlar → AP kurulumu (hizmet sağlayıcı ve işletme sahibi; spec 2026-10-15-ap-kurulum): cihazın misafir portu bilgisi,
//! ağdaki cihazlar (ayrı sayfa, denetlenir), fabrika adresli AP'ye ulaşmak için 30 dk'lık yan oda (ayrı ağ ad alanı +
//! SOCKS5 aracı; ana ağ ad alanına dokunmaz) ve yabancı DHCP denetimi.

use super::*;
use crate::ap;
use crate::yan_oda::ARACI_PORT;

impl Panel {
    fn kopru_mac(&self, cfg: &Config) -> String {
        std::fs::read_to_string(self.sys.join(&cfg.main.iface).join("address")).unwrap_or_default().trim().to_string()
    }

    pub(super) fn ap_kurulum(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let m = &cfg.main;
        let mac = self.kopru_mac(cfg);
        let port = facts(&[
            ("Adres", format!("<code>{}</code>", h(&m.router_ip))),
            ("MAC", if mac.is_empty() { "bilinmiyor".into() } else { format!("<code>{}</code>", h(&mac)) }),
        ]);
        let gecici = match ap::durum(&self.ap_yollar, &*self.ip_oku) {
            Some((g, true)) => {
                let kalan = ((g.bitis - now).max(0.0) / 60.0).ceil() as u64;
                let saat = ortak::now_iso(g.bitis).get(11..16).unwrap_or("").to_string();
                let ag = ap::Cidr::parse(&g.adres).map(|c| format!("{}/{}", c.ag(), c.onek)).unwrap_or_default();
                format!(
                    "<p>Yan oda <code>{}</code> açık — {} kendiliğinden kapanır ({kalan} dk).</p>\
                     <p class=\"not\">AP'ye bilgisayardan: <code>scripts\\ap-ac.cmd -Adres &lt;AP'nin adresi&gt;</code> (ör. {} ağındaki \
                     fabrika adresi). Betik bu ağdaki adres için cihazdaki aracıya (127.0.0.1:{ARACI_PORT}) bağlanır; yalnızca \
                     <code>{}</code> ağındaki adreslere izin verilir, alan adı çözülmez (AP'ye IP adresiyle girin).</p>{}",
                    h(&g.adres),
                    h(&saat),
                    h(&ag),
                    h(&ag),
                    post_button(o, "/ap-kurulum/gecici-kaldir", "Şimdi kapat", &[], "ikincil")
                )
            }
            Some((g, false)) => format!(
                "<p>Yan oda <code>{}</code> artık yok (aracı durdu ya da cihaz yeniden başladı).</p>{}",
                h(&g.adres),
                post_button(o, "/ap-kurulum/gecici-kaldir", "Kapat", &[], "ikincil")
            ),
            // durum dosyası var ama okunamıyor (bozuk): açma reddedilir, kapatma dosyayı temizler
            None if self.ap_yollar.gecici.exists() => format!(
                "<p>Yan oda durumu okunamadı.</p>{}",
                post_button(o, "/ap-kurulum/gecici-kaldir", "Kapat", &[], "ikincil")
            ),
            None => format!(
                "<form class=\"satir\" method=\"post\" action=\"/ap-kurulum/gecici-ekle\">{}\
                 <div><label for=\"apg\">Yan oda adresi</label><input type=\"text\" id=\"apg\" name=\"adres\" placeholder=\"192.168.1.250/24\" required></div>\
                 <button>Aç ({} dk)</button></form>\
                 <p class=\"not\">AP'nin fabrika adresiyle aynı alt ağdan, AP'nin kullanmadığı bir adres yazın (ör. AP 192.168.1.1 ise \
                 192.168.1.250/24). Adres cihazın kendi ağına değil, misafir portuna bağlı ayrı bir \"yan oda\"ya verilir: modemin \
                 ağıyla aynı olsa da cihazın interneti ve misafirler etkilenmez. Yan oda {} dk sonra (ve cihaz yeniden başlayınca) \
                 kendiliğinden kapanır.</p>",
                csrf_input(o),
                ap::SURE_SN / 60,
                ap::SURE_SN / 60
            ),
        };
        let dhcp = format!(
            "<p>Kutudan yeni çıkmış AP'nin (ör. ZyXEL, 192.168.1.1) DHCP sunucusu çoğu zaman açık gelir: misafirlere kendi adresini dağıtır, \
             o misafirler giriş sayfasını göremez ve internete çıkamaz. Denetim misafir portunda ~3 sn DHCP teklifi dinler; \
             cihazın kendi sunucusu (<code>{}</code>) sayılmaz.</p>{}",
            h(&m.router_ip),
            post_button(o, "/ap-kurulum/dhcp-denetle", "Yabancı DHCP sunucusu var mı?", &[], "ikincil")
        );
        let body = format!(
            "<section class=\"kart grup\"><h2>Cihazın misafir portu</h2><div>{port}<p class=\"not\">AP'de \"yönetime yalnızca şu IP/MAC \
             izin ver\" seçeneği varsa bu adres ve MAC yazılır — en son, AP kalıcı adresine (ör. 10.50.0.2) taşınıp o adresten \
             yeniden girildikten sonra (yan odadan bağlıyken IP kısıtı oturumu keser, AP'ye ulaşılamaz).</p></div></section>\
             <section class=\"kart grup\"><h2>Ağdaki cihazlar</h2><div><p>Misafir portunda son görülen cihazlar (DHCP kiraları ve ARP \
             tablosu); müşterinin verdiği MAC'le eşleştirin.</p><p><a class=\"dugme ikincil\" href=\"/ap-kurulum/cihazlar\">Ağdaki cihazları göster</a></p>\
             <p class=\"not\">Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; yan oda açıp AP'ye bağlanın.</p></div></section>\
             <section class=\"kart grup\"><h2>Yabancı DHCP sunucusu</h2><div>{dhcp}</div></section>\
             <section class=\"kart grup\"><h2>Yan oda (fabrika adresli AP)</h2><div>{gecici}</div></section>\
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
            "<p><a href=\"/ayarlar#ap-kurulum\">← AP kurulumu</a></p>{}\
             <p class=\"not\">Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; yan oda açıp AP'ye bağlanın.</p>",
            table(&["MAC", "IP", "Ad", "Kaynak", "Not"], &rows, "Cihaz görülmedi")
        );
        self.page(cfg, req, Some(o), "Ağdaki cihazlar", &body)
    }

    /// Yabancı DHCP denetimi (`ctl ap-dhcp-denetle`, kısa ömürlü yan odadan). Denetim `AP_DHCP_DENETIM sonuc=yok|var|hata`.
    pub(super) fn ap_dhcp_denetle(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let cikti = (self.ip_oku)(&["/usr/local/bin/wificorrect", "ctl", "ap-dhcp-denetle"]);
        let geri = "<p><a href=\"/ayarlar#ap-kurulum\">← AP kurulumu</a></p>";
        let govde = match ap::dhcp_cikti_coz(&cikti, &cfg.main.router_ip, &self.kopru_mac(cfg)) {
            None => {
                self.audit(cfg, req, Some(o), "AP_DHCP_DENETIM", "sonuc=hata");
                "<div class=\"mesaj hata\">Denetim yapılamadı (cihazda yan oda açılamadı). Biraz sonra yeniden deneyin.</div>".to_string()
            }
            Some(v) if v.is_empty() => {
                self.audit(cfg, req, Some(o), "AP_DHCP_DENETIM", "sonuc=yok");
                "<div class=\"mesaj\">Yabancı DHCP sunucusu bulunmadı.</div><p class=\"not\">Yalnızca denetim anında açık ve misafir \
                 portuna bağlı cihazlar görünür; AP yeni takıldıysa açılmasını (1–2 dk) bekleyip yeniden deneyin.</p>"
                    .to_string()
            }
            Some(v) => {
                let ozet: Vec<String> = v.iter().map(|t| format!("{}/{}", t.sunucu, t.mac)).collect();
                self.audit(cfg, req, Some(o), "AP_DHCP_DENETIM", &format!("sonuc=var sunucular={}", ozet.join(",")));
                let rows: Vec<Vec<String>> = v
                    .iter()
                    .map(|t| vec![format!("<code>{}</code>", h(&t.sunucu.to_string())), format!("<code>{}</code>", h(&t.mac)), h(&t.adres.to_string())])
                    .collect();
                format!(
                    "<div class=\"mesaj hata\">Misafir ağında başka bir DHCP sunucusu var. Bu cihazın DHCP sunucusu kapatılmalı; \
                     açık kaldıkça bazı misafirler internete çıkamaz.</div>{}\
                     <p class=\"not\">MAC'i ağdaki cihazlar listesiyle ve müşterinin verdiği MAC'le karşılaştırın. AP fabrika \
                     adresindeyse yan oda açıp AP'nin arayüzünden DHCP'yi kapatın (docs/AP_KURULUM.md).</p>",
                    table(&["Sunucu", "MAC", "Teklif ettiği adres"], &rows, "")
                )
            }
        };
        self.page(cfg, req, Some(o), "Yabancı DHCP denetimi", &format!("{geri}{govde}"))
    }

    pub(super) fn ap_gecici_ekle(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let girdi = req.form.get("adres").map_or("", String::as_str);
        match ap::ekle(cfg, &self.ap_yollar, girdi, ap::SURE_SN, &*self.ip_oku, &*self.runner, now) {
            Ok(g) => {
                self.audit(cfg, req, Some(o), "AP_YAN_ODA", &format!("acildi adres={} sure={}", g.adres, ap::SURE_SN));
                let msg = format!("Yan oda {} açıldı; {} dk sonra kendiliğinden kapanır.", g.adres, ap::SURE_SN / 60);
                redirect("/ap-kurulum", Some((&msg, false)))
            }
            Err(e) => redirect("/ap-kurulum", Some((&e, true))),
        }
    }

    pub(super) fn ap_gecici_kaldir(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        match ap::kaldir(&self.ap_yollar, "elle", None, &*self.ip_oku, &*self.runner) {
            Ok(Some(k)) => {
                self.audit(cfg, req, Some(o), "AP_YAN_ODA", &format!("kapandi adres={} neden=elle", k.adres));
                redirect("/ap-kurulum", Some((&format!("Yan oda {} kapatıldı.", k.adres), false)))
            }
            Ok(None) => redirect("/ap-kurulum", Some(("Açık yan oda yok.", false))),
            Err(e) => redirect("/ap-kurulum", Some((&e, true))),
        }
    }
}
