//! Panel: Yasaklı siteler ve kelimeler (iki rol de görür). Kaydedince hemen uygulanır (src/filtre.rs).

use super::*;
use crate::filtre;
use crate::liste;

const LISTELER: &[(&str, &str, &str)] = &[
    ("site", "Yasaklı siteler", "Alan adı ve bütün alt adları engellenir (ör. bet365.com → www.bet365.com, m.bet365.com)."),
    ("kelime", "Yasaklı kelimeler", "Alan adının herhangi bir yerinde geçerse engellenir (ör. bet → superbet.com.tr). Nokta içeremez."),
    ("istisna", "İstisnalar", "Kelime filtresine ya da hazır listeye takılan meşru siteler için: bu kelime geçen alan adı kelime filtresinden ve hazır listelerden geçer (ör. alphabet, better)."),
];

fn list_mut<'a>(cfg: &'a mut Config, liste: &str) -> Option<&'a mut Vec<String>> {
    match liste {
        "site" => Some(&mut cfg.filtre.siteler),
        "kelime" => Some(&mut cfg.filtre.kelimeler),
        "istisna" => Some(&mut cfg.filtre.istisnalar),
        _ => None,
    }
}

impl Panel {
    /// Birleşik hazır listeyi ayrı işte yeniden kurdurur (~1 milyon satır; panel isteği beklemez). Sabit ad: iş zaten
    /// sürüyorsa systemd ikincisini reddeder, süren iş bitmeden ayarı yeniden okuyup son değişikliği de uygular.
    fn hazir_uygula_baslat(&self) {
        (self.runner)(&cmd(&["systemd-run", "--collect", "--unit", "wfc-liste-uygula", "/usr/local/bin/wificorrect", "ctl", "liste-uygula"]));
    }

    /// Hazır listeler bölümü: kategori, açık/kapalı düğmesi, alan adı sayısı, son güncelleme.
    fn hazir_bolum(&self, cfg: &Config, o: &Oturum) -> String {
        let rows: Vec<Vec<String>> = liste::KATEGORILER
            .iter()
            .map(|(k, ad)| {
                let acik = liste::acik(cfg, k);
                let m = liste::meta_oku(&self.liste_yollar, k);
                let (durum, dugme, yeni) = if acik { ("<b>Açık</b>", "Kapat", "kapali") } else { ("Kapalı", "Aç", "acik") };
                vec![
                    h(ad),
                    durum.to_string(),
                    m.as_ref().map_or("—".into(), |m| m.sayi.to_string()),
                    m.as_ref().map_or("Henüz indirilmedi".into(), |m| h(&ortak::tarih_goster(&m.zaman))),
                    post_button(o, "/yasakli-siteler/hazir", dugme, &[("kategori", k), ("durum", yeni)], if acik { "ikincil" } else { "" }),
                ]
            })
            .collect();
        format!(
            "<section class=\"kart\"><h2>Hazır listeler</h2><p class=\"not\">Merkezden güncel tutulan hazır engel listeleri; \
             değişiklik birkaç saniye içinde geçerli. Yanlış engellenen bir site için alan adının bir parçasını aşağıdaki İstisnalar'a yazın.</p>{}</section>",
            table(&["Kategori", "Durum", "Alan adı", "Son güncelleme", ""], &rows, "")
        )
    }

    /// Hazır listeyi aç / kapat: ayar kaydı, denetim, birleşik dosya ayrı işte; açılan kategorinin yenisi merkezden indirilir.
    pub(super) fn hazir_degistir(&self, mut cfg: Config, req: &Req, o: &Oturum) -> Resp {
        let k = req.form.get("kategori").map_or("", String::as_str);
        let acik = match req.form.get("durum").map(String::as_str) {
            Some("acik") => true,
            Some("kapali") => false,
            _ => return redirect("/yasakli-siteler", Some(("Geçersiz istek.", true))),
        };
        if liste::ad(k).is_none() {
            return redirect("/yasakli-siteler", Some(("Bilinmeyen liste.", true)));
        }
        cfg.filtre.hazir.insert(k.to_string(), acik);
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect("/yasakli-siteler", Some((&e, true)));
        }
        self.audit(&cfg, req, Some(o), "PANEL_HAZIR_LISTE", &format!("kategori={k} durum={}", if acik { "acik" } else { "kapali" }));
        self.hazir_uygula_baslat(); // eldeki son liste
        if acik {
            // merkezde daha yenisi varsa ayrı işte indirilir (iş sürüyorsa systemd ikincisini reddeder)
            (self.runner)(&cmd(&["systemd-run", "--collect", "--unit", "wfc-liste", "/usr/local/bin/wificorrect", "ctl", "liste-guncelle"]));
        }
        redirect("/yasakli-siteler", Some((if acik { "Liste açıldı, birkaç saniye içinde geçerli." } else { "Liste kapatıldı, birkaç saniye içinde geçerli." }, false)))
    }

    pub(super) fn filtre_sayfa(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let hits = filtre::counters(&ortak::capture(&["iptables", "-L", "WFC_DNS", "-v", "-n", "-x"]));
        let mut cards = String::new();
        for (key, title, note) in LISTELER {
            let list = match *key {
                "site" => &cfg.filtre.siteler,
                "kelime" => &cfg.filtre.kelimeler,
                _ => &cfg.filtre.istisnalar,
            };
            let rows: Vec<Vec<String>> = list
                .iter()
                .map(|v| {
                    let mut r = vec![format!("<code>{}</code>", h(v))];
                    if *key != "site" {
                        r.push(hits.get(v).map_or("0".into(), |n| n.to_string()));
                    }
                    r.push(post_button(o, "/yasakli-siteler/kaldir", "Kaldır", &[("liste", key), ("deger", v)], "ikincil"));
                    r
                })
                .collect();
            let headers: &[&str] = if *key == "site" { &["Site", ""] } else { &["Kelime", "Eşleşen sorgu", ""] };
            cards.push_str(&format!(
                "<section class=\"kart\"><h2>{}</h2><p class=\"not\">{}</p>\
                 <form class=\"satir\" method=\"post\" action=\"/yasakli-siteler/ekle\">{}<input type=\"hidden\" name=\"liste\" value=\"{key}\">\
                 <div><label for=\"d-{key}\">Ekle</label><input type=\"text\" id=\"d-{key}\" name=\"deger\" required autocomplete=\"off\"></div><button>Ekle</button></form>{}</section>",
                h(title),
                h(note),
                csrf_input(o),
                table(headers, &rows, "Liste boş")
            ));
        }
        let ad = req.query.get("ad").map_or("", |s| s.trim());
        let sonuc = if ad.is_empty() {
            String::new()
        } else {
            match filtre::check(cfg, &self.liste_yollar, ad) {
                Some(why) => format!("<p class=\"hata\"><b>{}</b> engelli ({}).</p>", h(ad), h(&why)),
                None => format!("<p><b>{}</b> engelli değil.</p>", h(ad)),
            }
        };
        let body = format!(
            "<p class=\"not\">Müşteri ağında engellenen adresler \"site bulunamadı\" olarak görünür. Bütün DNS cihaza yönlendirildiği için \
             başka DNS yazmak filtreyi atlatmaz; ancak telefonda \"Özel DNS\" ya da tarayıcıda \"Güvenli DNS\" (şifreli DNS) açıksa atlatılır.</p>\
             {hazir}<section class=\"kart\"><h2>Bu adres engelli mi?</h2><form class=\"satir\" method=\"get\" action=\"/yasakli-siteler\">\
             <div><label for=\"ad\">Alan adı</label><input type=\"text\" id=\"ad\" name=\"ad\" value=\"{}\" placeholder=\"ör. www.ornek.com\"></div>\
             <button class=\"ikincil\">Dene</button></form>{sonuc}</section>{cards}",
            h(ad),
            hazir = self.hazir_bolum(cfg, o)
        );
        self.page(cfg, req, Some(o), "Yasaklı siteler", &body)
    }

    pub(super) fn filtre_degistir(&self, mut cfg: Config, req: &Req, o: &Oturum, ekle: bool) -> Resp {
        let liste = req.form.get("liste").map_or("", String::as_str);
        let raw = req.form.get("deger").map_or("", String::as_str);
        let value = if liste == "site" { filtre::normalize_site(raw) } else { filtre::normalize_word(raw) };
        let Some(value) = value else {
            let why = if liste == "site" { "Geçersiz alan adı (ör. ornek.com)." } else { "Geçersiz kelime: 2-63 karakter, yalnızca harf (Türkçe karakter olmadan), rakam, tire." };
            return redirect("/yasakli-siteler", Some((why, true)));
        };
        let Some(list) = list_mut(&mut cfg, liste) else { return redirect("/yasakli-siteler", Some(("Geçersiz liste.", true))) };
        if ekle {
            if list.contains(&value) {
                return redirect("/yasakli-siteler", Some(("Zaten listede.", true)));
            }
            list.push(value.clone());
            list.sort();
        } else if let Some(i) = list.iter().position(|v| *v == value) {
            list.remove(i);
        } else {
            return redirect("/yasakli-siteler", Some(("Listede yok.", true)));
        }
        if let Err(e) = cfg.save(&self.cfg_path) {
            return redirect("/yasakli-siteler", Some((&e, true)));
        }
        self.audit(&cfg, req, Some(o), if ekle { "PANEL_FILTRE_EKLE" } else { "PANEL_FILTRE_KALDIR" }, &format!("liste={liste} deger={value}"));
        // istisnalar hazır listeden de çıkarır (ayrı işte)
        if liste == "istisna" {
            self.hazir_uygula_baslat();
        }
        match filtre::uygula(&cfg, &self.filtre_conf, &*self.runner) {
            Ok(()) => redirect("/yasakli-siteler", Some((if ekle { "Eklendi, hemen geçerli." } else { "Kaldırıldı." }, false))),
            Err(e) => redirect("/yasakli-siteler", Some((&format!("Kaydedildi ama uygulanamadı: {e}"), true))),
        }
    }
}
