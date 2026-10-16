//! Panel → Ayarlar → AP kurulumu (hizmet sağlayıcı ve işletme sahibi; spec 2026-10-15-ap-kurulum): cihazın misafir portu bilgisi,
//! ağdaki cihazlar (ayrı sayfa, denetlenir), fabrika adresli AP'ye ulaşmak için 30 dk'lık yan oda (ayrı ağ ad alanı +
//! SOCKS5 aracı; ana ağ ad alanına dokunmaz) ve yabancı DHCP denetimi.
//! Takılan cihaz (spec 2026-10-16-ap-arayuz): etiketteki adresle "Bağlan" / "Otomatik bul" (arka plan işi, src/ap_bul.rs;
//! sayfa kendini yeniler) ve "AP arayüzüne git": hedef yazılır, yeni sekme merkezin geçişine (cihaz.wificorrect.com/_ap-gecis)
//! gider, AP'nin arayüzü ap.wificorrect.com'da açılır (aktarım: panel/ap_aktarim.rs, src/ap_arayuz.rs).

use super::*;
use crate::ap;
use crate::ap_arayuz::{self, Hedef, Yol};
use crate::ap_bul;
use crate::yan_oda::ARACI_PORT;
use std::net::Ipv4Addr;

/// "14:32" (Türkiye saati)
fn saat(ts: f64) -> String {
    ortak::now_iso(ts).get(11..16).unwrap_or("").to_string()
}

fn rol_adi(o: &Oturum) -> &'static str {
    if o.rol == Rol::Hizmet { "hizmet" } else { "sahip" }
}

/// "AP arayüzüne git": yeni sekmede (form hedefi), oturumun CSRF'iyle
fn git_formu(o: &Oturum, adres: &str, etiket: &str) -> String {
    format!(
        "<form class=\"ic\" method=\"post\" action=\"/ap-kurulum/arayuz\" target=\"_blank\">{}<input type=\"hidden\" name=\"hedef\" value=\"{}\"><button>{}</button></form>",
        csrf_input(o),
        h(adres),
        h(etiket)
    )
}

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
        let durum = ap::durum(&self.ap_yollar, &*self.ip_oku);
        let acik_yan = durum.as_ref().filter(|(_, canli)| *canli).map(|(g, _)| g.clone());
        let gecici = match durum {
            Some((g, true)) => {
                let kalan = ((g.bitis - now).max(0.0) / 60.0).ceil() as u64;
                let ag = ap::Cidr::parse(&g.adres).map(|c| format!("{}/{}", c.ag(), c.onek)).unwrap_or_default();
                // önerilen AP adresi: son aramanın bu yan odadaki sonucu, yoksa etiketteki adres (bu ağdaysa)
                let oneri = ap_bul::oku(&self.ap_yollar)
                    .filter(|d| d.yan_oda == g.adres && !d.adres.is_empty())
                    .map(|d| d.adres)
                    .or_else(|| ap::etiket_coz(&cfg.ap.etiket, cfg).ok().map(|(ip, _)| ip).filter(|ip| ap::yan_odada(*ip, &g)).map(|ip| ip.to_string()))
                    .unwrap_or_default();
                format!(
                    "<p>Yan oda <code>{}</code> açık — {} kendiliğinden kapanır ({kalan} dk).</p>\
                     <form class=\"satir\" method=\"post\" action=\"/ap-kurulum/arayuz\" target=\"_blank\">{}\
                     <div><label for=\"aph\">AP'nin adresi ({} ağında)</label><input type=\"text\" id=\"aph\" name=\"hedef\" value=\"{}\" required></div>\
                     <button>AP arayüzüne git</button></form>\
                     <p class=\"not\">AP'nin arayüzü yeni sekmede (ap.wificorrect.com) açılır. Yedek yol, bilgisayardan betik: \
                     <code>scripts\\ap-ac.cmd -Adres &lt;AP'nin adresi&gt; -Tunel &lt;tünel adresi&gt;</code>; betik bu ağdaki adres için \
                     cihazdaki aracıya (127.0.0.1:{ARACI_PORT}) bağlanır. Yalnızca <code>{}</code> ağındaki adreslere izin verilir, alan adı \
                     çözülmez (AP'ye IP adresiyle girin).</p>{}",
                    h(&g.adres),
                    h(&saat(g.bitis)),
                    csrf_input(o),
                    h(&ag),
                    h(&oneri),
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
            // durum dosyası yok ama aracı çalışıyor (yarıda kalmış işlem): 127.0.0.1:1085 açık kalmasın
            None if ap::araci_canli(&*self.ip_oku) => format!(
                "<p>Yan oda aracısı çalışıyor ama durum kaydı yok.</p>{}",
                post_button(o, "/ap-kurulum/gecici-kaldir", "Kapat", &[], "ikincil")
            ),
            None => format!(
                "<form class=\"satir\" method=\"post\" action=\"/ap-kurulum/gecici-ekle\">{}\
                 <div><label for=\"apg\">Yan oda adresi</label><input type=\"text\" id=\"apg\" name=\"adres\" placeholder=\"192.168.1.250/24\" required></div>\
                 <button>Aç ({} dk)</button></form>\
                 <p class=\"not\">Çoğu zaman gerekmez: yukarıdaki \"Bağlan\" yan odayı boş bir adresle kendisi açar. Elle: AP'nin fabrika \
                 adresiyle aynı alt ağdan, AP'nin kullanmadığı bir adres yazın (ör. AP 192.168.1.1 ise \
                 192.168.1.250/24; AP'nin DHCP sunucusu açıksa dağıttığı havuzun dışında bir adres seçin). Adres cihazın kendi ağına değil, misafir portuna bağlı ayrı bir \"yan oda\"ya verilir: modemin \
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
            "<section class=\"kart grup\"><h2>Takılan cihaz (AP ya da modem)</h2><div>{takili}</div></section>\
             <section class=\"kart grup\"><h2>Cihazın misafir portu</h2><div>{port}<p class=\"not\">AP'de \"yönetime yalnızca şu IP/MAC \
             izin ver\" seçeneği varsa bu adres ve MAC yazılır — en son, AP kalıcı adresine (ör. 10.50.0.2) taşınıp o adresten \
             yeniden girildikten sonra (yan odadan bağlıyken IP kısıtı oturumu keser, AP'ye ulaşılamaz).</p></div></section>\
             <section class=\"kart grup\"><h2>Ağdaki cihazlar</h2><div><p>Misafir portunda son görülen cihazlar (DHCP kiraları ve ARP \
             tablosu); müşterinin verdiği MAC'le eşleştirin.</p><p><a class=\"dugme ikincil\" href=\"/ap-kurulum/cihazlar\">Ağdaki cihazları göster</a></p>\
             <p class=\"not\">Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; yan oda açıp AP'ye bağlanın.</p></div></section>\
             <section class=\"kart grup\"><h2>Yabancı DHCP sunucusu</h2><div>{dhcp}</div></section>\
             <section class=\"kart grup\"><h2>Yan oda (fabrika adresli AP)</h2><div>{gecici}</div></section>\
             <section class=\"kart grup\"><h2>AP adresleri</h2><div>{kayit}</div></section>",
            takili = self.takili_cihaz(cfg, o, now, acik_yan.as_ref()),
            kayit = self.ap_adres_karti(cfg, o)
        );
        self.page(cfg, req, Some(o), "AP kurulumu", &body)
    }

    /// Takılan cihaz: açık AP arayüzü erişimi, son arama, etiketteki adres + marka formu, "Otomatik bul".
    fn takili_cihaz(&self, cfg: &Config, o: &Oturum, now: f64, yan: Option<&ap::Gecici>) -> String {
        let mut s = String::new();
        match ap_arayuz::denetle(&self.ap_yollar, cfg, now) {
            ap_arayuz::Durum::Gecerli(hd) => s.push_str(&format!(
                "<p><b>AP arayüzü erişimi açık:</b> <code>{}</code> ({}), {} kendiliğinden kapanır.</p><div class=\"eylemler\">{}{}</div>",
                h(&hd.konak()),
                if hd.yol == "yan_oda" { "yan odadan" } else { "misafir ağından" },
                h(&saat(hd.bitis)),
                git_formu(o, &hd.adres, "AP arayüzüne git"),
                post_button(o, "/ap-kurulum/arayuz-kapat", "Erişimi kapat", &[], "ikincil")
            )),
            ap_arayuz::Durum::Kapandi(hd, neden) => ap_arayuz::denetim(&cfg.main.log_root, now, &hd, &ap_arayuz::kapandi_ek(&hd, neden)),
            ap_arayuz::Durum::Yok => {}
        }
        if let Some(d) = ap_bul::oku(&self.ap_yollar) {
            s.push_str(&self.bul_sonucu(cfg, o, &d, yan));
        }
        let adlar: Vec<&str> = ap::BILINEN_ADLAR.iter().map(|(a, _)| *a).filter(|a| !a.ends_with("repeater.net") && !a.ends_with(".com")).take(5).collect();
        s.push_str(&format!(
            "<form class=\"satir\" method=\"post\" action=\"/ap-kurulum/baglan\">{}\
             <div><label for=\"ape\">Etiketteki adres</label><input type=\"text\" id=\"ape\" name=\"etiket\" value=\"{}\" placeholder=\"192.168.1.1\" maxlength=\"100\" autocomplete=\"off\" required></div>\
             <div><label for=\"apm\">Marka / model (isteğe bağlı)</label><input type=\"text\" id=\"apm\" name=\"marka\" value=\"{}\" placeholder=\"ör. TP-Link TL-WA901ND\" maxlength=\"60\" autocomplete=\"off\"></div>\
             <button>Bağlan</button></form>\
             <p class=\"not\">Modemin ya da access point'in altındaki etikette yazan IP adresini yazın. Etiketteki kullanıcı adı ve \
             parolayı buraya yazmayın. Bilinen adlar da olur ({} …). Adres misafir ağında değilse cihaz aynı ağdan boş bir adresle \
             {} dk'lık yan oda açar ve cihazın web arayüzüne (80, yoksa 443) bağlanmayı dener.</p><p>{}</p>\
             <p class=\"not\">Etiket okunmuyorsa: yaygın fabrika adreslerini (192.168.1.1, 192.168.0.1, 192.168.0.254 …) sırayla dener; \
             cevap veren ilk cihazda yan oda açık kalır.</p>",
            csrf_input(o),
            h(&cfg.ap.etiket),
            h(&cfg.ap.marka),
            h(&adlar.join(", ")),
            ap::SURE_SN / 60,
            post_button(o, "/ap-kurulum/otomatik", "Otomatik bul", &[], "ikincil")
        ));
        s
    }

    /// Son aramanın sonucu (sürüyorsa durum bağlantısı); bulunan cihaza hâlâ gidilebiliyorsa "AP arayüzüne git".
    fn bul_sonucu(&self, cfg: &Config, o: &Oturum, d: &ap_bul::Durum, yan: Option<&ap::Gecici>) -> String {
        let ne = match d.tur.as_str() {
            "baglan" => format!("<code>{}</code>", h(d.hedefler.first().map_or("", String::as_str))),
            _ => "yaygın fabrika adresleri".to_string(),
        };
        match d.durum.as_str() {
            "suruyor" if ap_bul::calisiyor(&*self.ip_oku) => format!("<p><b>Aranıyor:</b> {ne} … <a href=\"/ap-kurulum/bul\">durumu göster</a></p>"),
            "suruyor" => "<p class=\"hata\">Son arama yarıda kaldı; yeniden deneyin.</p>".into(),
            "bulundu" => {
                let ip: Option<Ipv4Addr> = d.adres.parse().ok();
                let gidilir = ip.is_some_and(|ip| match d.yol.as_str() {
                    "misafir" => ap::kayitli_ap_mi(ip, cfg),
                    _ => yan.is_some_and(|g| g.adres == d.yan_oda && ap::yan_odada(ip, g)),
                });
                let yer = if d.yol == "misafir" { "misafir ağında".to_string() } else { format!("yan oda <code>{}</code>", h(&d.yan_oda)) };
                let alt = if gidilir {
                    format!("<div class=\"eylemler\">{}</div>", git_formu(o, &d.adres, "AP arayüzüne git"))
                } else if d.yol == "misafir" {
                    "<p class=\"not\">Bu adres kayıtlı AP adresi değil: AP arayüzüne gitmek için aşağıdaki AP adresleri'ne kaydedin \
                     (DHCP havuzu dışında olmalı; AP havuzdan adres aldıysa önce sabit adrese alın ya da betiği kullanın).</p>"
                        .into()
                } else {
                    "<p class=\"not\">Yan oda kapandı; yeniden \"Bağlan\" deyin.</p>".into()
                };
                format!(
                    "<p><b>Cihaz bulundu ({}):</b> <code>{}</code> — {yer} ({}).</p>{alt}",
                    if d.sema == "https" { "HTTPS" } else { "HTTP" },
                    h(&d.adres),
                    h(&saat(d.bitis))
                )
            }
            _ => {
                let ek = if d.durum == "web_yok" && !d.yan_oda.is_empty() {
                    format!(" Yan oda <code>{}</code> açık; başka bir port için betiği kullanın.", h(&d.yan_oda))
                } else {
                    String::new()
                };
                format!(
                    "<p class=\"hata\">{} ({ne}, {})</p>{}",
                    h(if d.mesaj.is_empty() { "Arama sonuç vermedi." } else { &d.mesaj }),
                    h(&saat(d.bitis)),
                    if ek.is_empty() { String::new() } else { format!("<p class=\"not\">{}</p>", ek.trim_start()) }
                )
            }
        }
    }

    fn ap_adres_karti(&self, cfg: &Config, o: &Oturum) -> String {
        let liste = &cfg.ap.adresler;
        let mevcut = if liste.is_empty() {
            "<p>Kayıtlı AP adresi yok.</p>".to_string()
        } else {
            let gitler: String = liste
                .iter()
                .filter(|a| a.parse::<Ipv4Addr>().is_ok_and(|ip| ap::kayitli_ap_mi(ip, cfg)))
                .map(|a| git_formu(o, a, &format!("{a} arayüzüne git")))
                .collect();
            format!(
                "<p>Kayıtlı: {}</p>{}",
                liste.iter().map(|a| format!("<code>{}</code>", h(a))).collect::<Vec<_>>().join(", "),
                if gitler.is_empty() { String::new() } else { format!("<div class=\"eylemler\">{gitler}</div>") }
            )
        };
        format!(
            "{mevcut}<form method=\"post\" action=\"/ap-kurulum/adresler\">{}\
             <label for=\"apa\">AP yönetim adresleri (satır başına bir adres, en çok {})</label>\
             <textarea id=\"apa\" name=\"adresler\" rows=\"3\" maxlength=\"200\">{}</textarea>\
             <p><button>Kaydet</button></p></form>\
             <p class=\"not\">AP ayarlandıktan sonra kalıcı yönetim adresini yazın (ör. 10.50.0.2; misafir ağında, DHCP havuzu dışında). \
             Kayıt ağdaki cihazlar listesinde işaretlenir ve AP'nin arayüzüne panelden yan odasız gidilir. Başka porttaki misafirlerden AP'ye giden trafik cihazın \
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
    /// Aynı anda tek denetim; komut 25 sn'de bitmezse öldürülür (yan oda süreçle gider): panelin iş parçacıkları bağlanmaz.
    pub(super) fn ap_dhcp_denetle(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        use std::sync::atomic::{AtomicBool, Ordering};
        let geri = "<p><a href=\"/ayarlar#ap-kurulum\">← AP kurulumu</a></p>";
        if self.dhcp_denetim.swap(true, Ordering::SeqCst) {
            let govde = "<div class=\"mesaj hata\">Bir denetim zaten sürüyor; birkaç saniye sonra yeniden deneyin.</div>";
            return self.page(cfg, req, Some(o), "Yabancı DHCP denetimi", &format!("{geri}{govde}"));
        }
        struct Birak<'a>(&'a AtomicBool);
        impl Drop for Birak<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let cikti = {
            let _birak = Birak(&self.dhcp_denetim);
            (self.ip_oku)(&["timeout", "25", "/usr/local/bin/wificorrect", "ctl", "ap-dhcp-denetle"])
        };
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
                // yan odanın AP arayüzü erişimi de biter
                if let Some(hd) = ap_arayuz::kapat(&self.ap_yollar, true) {
                    self.audit(cfg, req, Some(o), "AP_ARAYUZ", &ap_arayuz::kapandi_ek(&hd, "yan_oda"));
                }
                let msg = if k.adres == "-" { "Yan oda aracısı durduruldu.".to_string() } else { format!("Yan oda {} kapatıldı.", k.adres) };
                redirect("/ap-kurulum", Some((&msg, false)))
            }
            Ok(None) => redirect("/ap-kurulum", Some(("Açık yan oda yok.", false))),
            Err(e) => redirect("/ap-kurulum", Some((&e, true))),
        }
    }

    /// "Bağlan": etiketteki adres (IP ya da bilinen ad) ve marka/model `[ap]`'ye yazılır (değiştiyse, denetim
    /// `PANEL_AP_ETIKET`), arama işi başlar.
    pub(super) fn ap_baglan(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let (ip, etiket) = match ap::etiket_coz(req.form.get("etiket").map_or("", String::as_str), cfg) {
            Ok(x) => x,
            Err(e) => return redirect("/ap-kurulum", Some((&e, true))),
        };
        let marka = match ap::marka_dogrula(req.form.get("marka").map_or("", String::as_str)) {
            Ok(m) => m,
            Err(e) => return redirect("/ap-kurulum", Some((&e, true))),
        };
        if etiket != cfg.ap.etiket || marka != cfg.ap.marka {
            let mut c = cfg.clone();
            (c.ap.etiket, c.ap.marka) = (etiket.clone(), marka.clone());
            if let Err(e) = c.save(&self.cfg_path) {
                return redirect("/ap-kurulum", Some((&e, true)));
            }
            self.audit(&c, req, Some(o), "PANEL_AP_ETIKET", &format!("etiket={etiket} marka={}", if marka.is_empty() { "-" } else { &marka }));
        }
        self.ap_bul_baslat(cfg, req, o, Some(ip), now)
    }

    /// Arama işini (`ctl ap-bul`, geçici systemd birimi) başlatır; sonuç sayfası kendini yeniler. Aynı anda tek arama.
    pub(super) fn ap_bul_baslat(&self, _cfg: &Config, req: &Req, o: &Oturum, hedef: Option<Ipv4Addr>, now: f64) -> Resp {
        if ap_bul::calisiyor(&*self.ip_oku) {
            return redirect("/ap-kurulum/bul", None); // sürüyor: onun sonucu beklenir
        }
        let rol = rol_adi(o);
        let mut d = ap_bul::Durum::basla(hedef, &o.user, rol, now);
        if let Err(e) = ap_bul::kaydet(&self.ap_yollar, &d) {
            return redirect("/ap-kurulum", Some((&e, true)));
        }
        let ip = if req.ip.parse::<std::net::IpAddr>().is_ok() { req.ip.as_str() } else { "-" };
        if !(self.runner)(&ap_bul::komut(&o.user, rol, ip, hedef)) {
            (d.durum, d.mesaj, d.bitis) = ("hata".into(), "Arama başlatılamadı.".into(), now);
            let _ = ap_bul::kaydet(&self.ap_yollar, &d);
            return redirect("/ap-kurulum", Some(("Arama başlatılamadı (systemd-run).", true)));
        }
        redirect("/ap-kurulum/bul", None)
    }

    /// Arama sürerken bekleme sayfası (2 sn'de bir kendini yeniler; betik yok); bitince AP kurulumuna (sonuç orada).
    pub(super) fn ap_bul_sayfa(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        if !ap_bul::calisiyor(&*self.ip_oku) {
            return redirect("/ap-kurulum", None);
        }
        let ne = match ap_bul::oku(&self.ap_yollar) {
            Some(d) if d.tur == "baglan" => format!("{} aranıyor", d.hedefler.first().map_or("", String::as_str)),
            Some(d) => format!("Yaygın fabrika adresleri ({} adres) deneniyor", d.hedefler.len()),
            None => "Aranıyor".into(),
        };
        let body = format!(
            "<p><a href=\"/ayarlar#ap-kurulum\">← AP kurulumu</a></p><p><b>{}…</b></p>\
             <p class=\"not\">Sayfa birkaç saniyede bir kendini yeniler; arama bitince sonuç AP kurulumu bölümünde görünür.</p>",
            h(&ne)
        );
        self.page(cfg, req, Some(o), "Takılan cihaz aranıyor", &body)
    }

    /// "AP arayüzüne git" (yeni sekme): hedef yalnızca açık yan odanın ağında ya da kayıtlı AP adresi; web arayüzü (80,
    /// yoksa 443) cevap veriyorsa hedef yazılır (`AP_ARAYUZ acildi`) ve sayfa merkezin geçişine gider (meta yenileme:
    /// CSP form-action yönlendirmeyi kesmesin). Erişim 30 dk, yan odada en çok yan odanın kalan süresi.
    pub(super) fn ap_arayuz_ac(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let hata = |m: &str| {
            let body = format!("<p><a href=\"/ayarlar#ap-kurulum\">← AP kurulumu</a></p><p class=\"hata\">{}</p>", h(m));
            self.page(cfg, req, Some(o), "AP arayüzü", &body)
        };
        let yan = ap::durum(&self.ap_yollar, &*self.ip_oku).filter(|(_, canli)| *canli).map(|(g, _)| g);
        let (ip, yol) = match ap_arayuz::hedef_dogrula(req.form.get("hedef").map_or("", String::as_str), cfg, yan.as_ref()) {
            Ok(x) => x,
            Err(e) => return hata(&e),
        };
        let yan_odadan = matches!(yol, Yol::YanOda(..));
        let Some(port) = (self.ap_dene)(ip, yan_odadan) else {
            return hata(&format!("{ip} adresinde web arayüzü (80/443) cevap vermedi. AP açık mı, kablo misafir portunda mı?"));
        };
        let (yol_adi, yan_oda, bitis) = match yol {
            Yol::YanOda(k, b) => ("yan_oda", Some(k), (now + ap_arayuz::SURE_SN).min(b)),
            Yol::Misafir => ("misafir", None, now + ap_arayuz::SURE_SN),
        };
        let yeni = Hedef {
            adres: ip.to_string(),
            port,
            sema: if port == 443 { "https" } else { "http" }.into(),
            yol: yol_adi.into(),
            bitis,
            kim: o.user.clone(),
            rol: rol_adi(o).into(),
            yan_oda,
        };
        if let Some(eski) = ap_arayuz::oku(&self.ap_yollar).filter(|e| e.adres != yeni.adres || e.yol != yeni.yol) {
            self.audit(cfg, req, Some(o), "AP_ARAYUZ", &ap_arayuz::kapandi_ek(&eski, "yeni_hedef"));
        }
        if let Err(e) = ap_arayuz::yaz(&self.ap_yollar, &yeni) {
            return hata(&e);
        }
        let sure = (bitis - now).max(0.0).round() as u64;
        self.audit(cfg, req, Some(o), "AP_ARAYUZ", &format!("acildi hedef={}:{} yol={} sure={sure}", yeni.adres, yeni.port, yeni.yol));
        let body = format!(
            "<p><b>AP arayüzü açılıyor:</b> <code>{}</code> …</p><p>Açılmazsa <a href=\"{}\">buraya tıklayın</a>. Erişim {} kendiliğinden \
             kapanır (ya da AP kurulumu → Erişimi kapat).</p>",
            h(&yeni.konak()),
            h(ap_arayuz::GECIS),
            h(&saat(bitis))
        );
        let meta = format!("<meta http-equiv=\"refresh\" content=\"0;url={}\">", h(ap_arayuz::GECIS));
        self.page_ek(cfg, req, Some(o), "AP arayüzü", &body, &meta)
    }

    pub(super) fn ap_arayuz_kapat(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        match ap_arayuz::kapat(&self.ap_yollar, false) {
            Some(hd) => {
                self.audit(cfg, req, Some(o), "AP_ARAYUZ", &ap_arayuz::kapandi_ek(&hd, "elle"));
                redirect("/ap-kurulum", Some(("AP arayüzü erişimi kapatıldı.", false)))
            }
            None => redirect("/ap-kurulum", Some(("Açık AP arayüzü erişimi yok.", false))),
        }
    }
}
