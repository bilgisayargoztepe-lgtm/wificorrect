//! Panel: cihaz yavaşlatma (1/2/5/10/20 Mb/sn, kaldırılana kadar) ve "Yavaşlatılmış cihazlar" bölümü.
//! Panel yalnızca kaydı (hiz_sinir.json) yazar ve `ctl hiz-uygula` ile hemen uzlaştırma ister; tc/nft'yi kaydedici de 5 sn'de bir uygular.

use super::*;
use crate::hiz;

/// Bağlı kullanıcılar tablosundaki "Yavaşlat" hücresi: sınırsızsa hız menüsü, sınırlıysa rozet + kaldır.
pub(super) fn yavas_hucresi(o: &Oturum, mac: &str, sinir: Option<&hiz::Sinir>) -> String {
    match sinir {
        Some(s) => format!(
            "<span class=\"rozet sinirli\">≤ {} Mb/sn</span> {}",
            s.hiz,
            post_button(o, "/oturumlar/yavaslatma-kaldir", "Yavaşlatmayı kaldır", &[("mac", mac), ("donus", "oturumlar")], "ikincil")
        ),
        None => {
            let secenekler: String = hiz::HIZLAR
                .iter()
                .map(|n| post_button(o, "/oturumlar/yavaslat", &format!("{n} Mb/sn"), &[("mac", mac), ("hiz", &n.to_string())], "ikincil"))
                .collect();
            format!(
                "<details class=\"yavas\"><summary class=\"dugme ikincil\">Yavaşlat</summary><div class=\"yavas-menu\">\
                 <span class=\"not\">Hız sınırı seçin</span>{secenekler}</div></details>"
            )
        }
    }
}

impl Panel {
    pub(super) fn yavaslatilmis(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        if !self.oto_yenileme(req, o, now) {
            self.audit(cfg, req, Some(o), "PANEL_YAVASLATILMIS", "");
        }
        let hata = crate::trafik::oku(&self.trafik_path)
            .and_then(|d| d.hiz_hata)
            .map(|e| format!("<div class=\"mesaj hata\">{}</div>", h(&e)))
            .unwrap_or_default();
        let rows: Vec<Vec<String>> = hiz::oku(&cfg.main.state_root)
            .iter()
            .map(|(mac, s)| {
                vec![
                    format!("<code>{}</code>", h(mac)),
                    h(&s.ad),
                    h(&format!("{} Mb/sn", s.hiz)),
                    h(&s.zaman.get(..16).unwrap_or("").replace('T', " ")),
                    h(&s.kim),
                    post_button(o, "/oturumlar/yavaslatma-kaldir", "Yavaşlatmayı kaldır", &[("mac", mac), ("donus", "yavaslatilmis")], "ikincil"),
                ]
            })
            .collect();
        let body = format!(
            "{hata}<p class=\"not\">Yavaşlatılan cihazın indirme ve yükleme hızı seçilen değerle sınırlanır; siz kaldırana kadar sürer \
             (cihaz ayrılıp yeniden bağlansa da). Ad, yavaşlatıldığı andaki addır.</p>{}",
            table(&["MAC", "Son bilinen ad", "Hız sınırı", "Zaman", "Kim", ""], &rows, "Yavaşlatılmış cihaz yok")
        );
        self.page(cfg, req, Some(o), "Yavaşlatılmış cihazlar", &body)
    }

    /// Kaydı yazdıktan sonra hemen uzlaştırma; çalışmazsa kaydedici 5 sn içinde uygular (kullanıcıya hata değil).
    fn hiz_uygula(&self) {
        let _ = (self.runner)(&cmd(&["/usr/local/bin/wificorrect", "ctl", "hiz-uygula"]));
    }

    pub(super) fn yavaslat(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let hata = |m: &str| redirect("/oturumlar", Some((m, true)));
        let Some(hiz) = req.form.get("hiz").and_then(|v| v.parse::<u32>().ok()).filter(|v| hiz::HIZLAR.contains(v)) else {
            return hata("Geçersiz hız.");
        };
        let Some(mac) = req.form.get("mac").and_then(|m| ortak::norm_mac(m)) else { return hata("Geçersiz MAC.") };
        let m = &cfg.main;
        // bağlı kullanıcı (oturum) ya da ağda (kirada) izinli cihaz olmalı
        let ad = match ortak::load_sessions(&m.state_root).get(&mac) {
            Some(s) => format!("{} {}", s.ad, s.soyad),
            None => {
                let izinli = Config::devices(&cfg.allow);
                let kirada = ortak::parse_leases(&std::fs::read_to_string(&m.leases_file).unwrap_or_default()).values().any(|k| *k == mac);
                match izinli.get(&mac).filter(|_| kirada) {
                    Some(not) if not.trim().is_empty() => "İzinli cihaz".to_string(),
                    Some(not) => format!("İzinli cihaz · {}", not.trim()),
                    None => return hata("Cihaz bağlı değil."),
                }
            }
        };
        {
            let Ok(_g) = ortak::state_lock(&m.state_root) else { return hata("Kilit alınamadı.") };
            let mut k = hiz::oku(&m.state_root);
            k.insert(mac.clone(), hiz::Sinir { hiz, ad, zaman: ortak::now_iso(now), kim: o.user.clone() });
            if hiz::kaydet(&m.state_root, &k).is_err() {
                return hata("Kaydedilemedi.");
            }
        }
        self.audit(cfg, req, Some(o), "PANEL_YAVASLAT", &format!("mac={mac} hiz={hiz}"));
        self.hiz_uygula();
        redirect("/oturumlar", Some((&format!("Yavaşlatıldı: en fazla {hiz} Mb/sn."), false)))
    }

    pub(super) fn yavaslatma_kaldir(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let donus = if req.form.get("donus").is_some_and(|d| d == "yavaslatilmis") { "/yavaslatilmis" } else { "/oturumlar" };
        let Some(mac) = req.form.get("mac").and_then(|m| ortak::norm_mac(m)) else { return redirect(donus, Some(("Geçersiz MAC.", true))) };
        let m = &cfg.main;
        {
            let Ok(_g) = ortak::state_lock(&m.state_root) else { return redirect(donus, Some(("Kilit alınamadı.", true))) };
            let mut k = hiz::oku(&m.state_root);
            if k.remove(&mac).is_none() {
                return redirect(donus, Some(("Bu cihaz yavaşlatılmamış.", true)));
            }
            if hiz::kaydet(&m.state_root, &k).is_err() {
                return redirect(donus, Some(("Kaydedilemedi.", true)));
            }
        }
        self.audit(cfg, req, Some(o), "PANEL_YAVASLATMA_KALDIR", &format!("mac={mac}"));
        self.hiz_uygula();
        redirect(donus, Some(("Yavaşlatma kaldırıldı.", false)))
    }
}
