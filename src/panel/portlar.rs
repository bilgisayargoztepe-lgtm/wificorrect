//! Panel 8c: Portlar ve Wi-Fi (iki rol de görür). Uygulama ayrı bir systemd işinde, 3 dk onay süresiyle (src/ag.rs).
//! İnternet alan portun PPPoE kutusu da burada (alanlar ve form `pppoe::form_html` / `pppoe::formdan`: giriş sayfasındaki Servis ile ortak).

use super::*;
use crate::ag::{self, Ag};
use crate::pppoe::{self, Durum};

fn rol_secim(name: &str, cur: &str, opts: &[(&str, &str)]) -> String {
    let o: String = opts.iter().map(|(k, l)| format!("<option value=\"{k}\"{}>{l}</option>", if *k == cur { " selected" } else { "" })).collect();
    format!("<select name=\"{name}\" aria-label=\"Görev\">{o}</select>")
}

/// İnternet alan portun durum satırı: bağlı ve kapalı (modemden DHCP) yeşil nokta, bağlanma sürerken düz metin, sorunlar kırmızı.
fn pppoe_durum_html(d: &Durum) -> String {
    let metin = h(&d.metin());
    match d {
        Durum::Kapali | Durum::Bagli(_) => format!("<span class=\"durum\">{metin}</span>"),
        Durum::Baglaniyor => format!("<span>{metin}</span>"),
        _ => format!("<span class=\"durum kotu\">{metin}</span>"),
    }
}

impl Panel {
    pub(super) fn portlar(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let y = &self.ag_yollar;
        let cur = ag::load(&y.ag);
        let eths = ag::ethernets(&self.sys);
        let mut banner = String::new();
        if let Some((deadline, _)) = ag::pending(y) {
            let left = (deadline - now).max(0.0) as u64;
            banner = if left > 0 {
                format!(
                    "<div class=\"kart\"><div class=\"mesaj hata\">Değişiklik uygulandı. <b>{left} saniye</b> içinde onaylamazsanız eski ayar kendiliğinden geri gelir.</div>\
                     <div class=\"eylemler\">{}{}</div></div>",
                    post_button(o, "/portlar/onayla", "Çalışıyor, onayla", &[], ""),
                    post_button(o, "/portlar/geri-al", "Hemen geri al", &[], "tehlike")
                )
            } else {
                "<div class=\"mesaj hata\">Onay süresi doldu, eski ayar geri yükleniyor. Birkaç saniye sonra sayfayı yenileyin.</div>".into()
            };
        }
        let rows: Vec<Vec<String>> = eths
            .iter()
            .map(|e| {
                let (up, speed, mac) = ag::link(&self.sys, e);
                let role = if *e == cur.wan { "alir" } else if cur.lan.contains(e) { "verir" } else { "kapali" };
                vec![
                    format!("{} <span class=\"not\">{}</span>", h(&ag::label(&cur, &eths, e)), h(e)),
                    if up { format!("<span class=\"durum\">bağlı{}</span>", speed.map_or(String::new(), |s| format!(", {s} Mb/s"))) } else { "<span class=\"not\">kablo yok</span>".into() },
                    format!("<code>{}</code>", h(&mac)),
                    rol_secim(&format!("rol_{e}"), role, &[("alir", "İnternet alır"), ("verir", "İnternet verir"), ("kapali", "Kapalı")]),
                ]
            })
            .collect();
        let w = &cur.wifi;
        let has_card = self.sys.join(&w.iface).exists();
        let kanal: String = (1..=13u8).map(|k| format!("<option value=\"{k}\"{}>{k}</option>", if k == w.kanal { " selected" } else { "" })).collect();
        // internet alan portun PPPoE'si: durum (kayıtlı ayara göre canlı), dükkân notu, form (parola hiç yazılmaz)
        let pppoe_kutu = format!(
            "<section class=\"kart grup\"><h2>PPPoE</h2><div><p>Durum: {}</p>\
             <p class=\"not\">Dükkânda PPPoE bağlanmaz; ONT'ye takılınca bağlanır.</p>{}</div></section>",
            pppoe_durum_html(&pppoe::durum(&cur, &self.sys, &*self.komut)),
            pppoe::form_html(&cur.pppoe)
        );
        let body = format!(
            "{banner}<form method=\"post\" action=\"/portlar\">{}\
             <section class=\"kart\"><h2>Ethernet portları</h2>{}</section>{pppoe_kutu}\
             <section class=\"kart grup\"><h2>Wi-Fi</h2><div>{}\
             <label for=\"wifi\">Görev</label>{}\
             <label for=\"ssid\">Ağ adı</label><input type=\"text\" id=\"ssid\" name=\"ssid\" value=\"{}\" maxlength=\"32\" autocomplete=\"off\">\
             <label for=\"sifre\">Şifre (boş = şifresiz)</label><input type=\"text\" id=\"sifre\" name=\"sifre\" value=\"{}\" maxlength=\"63\" autocomplete=\"off\">\
             <label for=\"kanal\">Kanal</label><select id=\"kanal\" name=\"kanal\">{kanal}</select></div></section>\
             <div class=\"kaydet\" style=\"padding-left:0\"><button>Uygula</button><p class=\"not\">Uygulayınca ağ yeniden başlar; müşteriler birkaç saniye kopar, \
             oturumları sürer. İnternet alan portu değiştirdiyseniz modem kablosunu yeni porta takın. <b>3 dakika içinde</b> bu sayfaya dönüp \
             onaylayın; onaylanmazsa (ya da cihaz yeniden açılırsa) eski ayar kendiliğinden geri gelir. Yönetim adresi değişmez \
             (internet alan porta hep aynı MAC yazılır).</p></div></form>\
             <div class=\"kart\" style=\"margin-top:36px\"><p class=\"not\">Sağ/sol etiketi cihazla ters mi?</p>{}</div>",
            csrf_input(o),
            table(&["Port", "Kablo", "MAC", "Görev"], &rows, "Ethernet bulunamadı"),
            if has_card { String::new() } else { format!("<p class=\"hata\">Wi-Fi kartı ({}) bulunamadı.</p>", h(&w.iface)) },
            rol_secim("wifi", if w.acik { "verir" } else { "kapali" }, &[("verir", "İnternet verir (yayın)"), ("kapali", "Kapalı")]),
            h(&w.ssid),
            h(&w.sifre),
            post_button(o, "/portlar/etiket", "Sağ/sol etiketini değiştir", &[], "ikincil"),
        );
        self.page(cfg, req, Some(o), "Portlar", &body)
    }

    pub(super) fn portlar_uygula(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let y = &self.ag_yollar;
        if ag::pending(y).is_some() {
            return redirect("/portlar", Some(("Önce bekleyen değişikliği onaylayın ya da geri alın.", true)));
        }
        if std::fs::metadata(y.yeni()).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|d| d.as_secs() < 30)) {
            return redirect("/portlar", Some(("Önceki değişiklik uygulanıyor, birkaç saniye bekleyin.", true)));
        }
        let cur = ag::load(&y.ag);
        let eths = ag::ethernets(&self.sys);
        let f = |k: &str| req.form.get(k).map_or("", String::as_str);
        let mut new = cur.clone();
        let wans: Vec<&String> = eths.iter().filter(|e| f(&format!("rol_{e}")) == "alir").collect();
        if wans.len() != 1 {
            return redirect("/portlar", Some(("Tam olarak bir Ethernet portu internet almalı.", true)));
        }
        new.wan = wans[0].clone();
        new.lan = eths.iter().filter(|e| f(&format!("rol_{e}")) == "verir").cloned().collect();
        new.wifi.acik = f("wifi") == "verir";
        new.wifi.ssid = f("ssid").trim().to_string();
        new.wifi.sifre = f("sifre").to_string();
        new.wifi.kanal = f("kanal").parse().unwrap_or(0);
        // PPPoE: boş parola = eskisi (ag::gecis birleştirmez; hazırlanan dosyaya eski parola burada yazılır), boş VLAN = yok. Yalnızca
        // form PPPoE seçimini taşıyorsa (Servis'teki gibi): PPPoE alanı olmayan bir gönderim (betik, başka bir form) kayıtlı PPPoE
        // ayarını varsayılana (kapalı, kullanıcı adı ve ileri alanlar boş) döndürmesin; kayıtlı ayar aynen kalır
        if req.form.contains_key("pppoe_acik") {
            new.pppoe = pppoe::formdan(&cur.pppoe, &|k: &str| f(k).to_string());
        }
        if new.wan != cur.wan && new.wan_mac.is_empty() {
            // modemdeki IP rezervasyonu korunsun: eski internet portunun MAC'i yeni porta yazılır
            new.wan_mac = ortak::norm_mac(&ag::link(&self.sys, &cur.wan).2).unwrap_or_default();
        }
        if let Err(e) = ag::validate(&new, &eths) {
            return redirect("/portlar", Some((&e, true)));
        }
        if new == cur {
            return redirect("/portlar", Some(("Değişiklik yok.", false)));
        }
        // AP yan odası köprüdeki macvlan'a bağlı: köprü yeniden kurulunca arayüzsüz "açık" kalmasın, önce kapanır
        match crate::ap::kaldir(&self.ap_yollar, "port", None, &*self.ip_oku, &*self.runner) {
            Ok(Some(k)) => self.audit(cfg, req, Some(o), "AP_YAN_ODA", &format!("kapandi adres={} neden=port", k.adres)),
            Ok(None) => {}
            Err(e) => return redirect("/portlar", Some((&format!("{e} Port değişikliği yapılmadı."), true))),
        }
        if let Some(h) = crate::ap_arayuz::kapat(&self.ap_yollar, true) {
            self.audit(cfg, req, Some(o), "AP_ARAYUZ", &crate::ap_arayuz::kapandi_ek(&h, "port"));
        }
        let staged = y.yeni();
        if let Err(e) = ortak::mkdirs(&y.durum).map_err(|e| e.to_string()).and_then(|_| ag::save(&staged, &new)) {
            return redirect("/portlar", Some((&e, true)));
        }
        // Yanıt tarayıcıya ulaşsın diye 2 sn sonra, panelden bağımsız bir işte (ağ yeniden başlarken panel bağlantısı kopar)
        let unit = format!("wfc-ag-gecis-{}", ortak::random_hex(4));
        let ok = (self.runner)(&cmd(&["systemd-run", "--unit", &unit, "--on-active", "2s", "--timer-property=AccuracySec=1s", "/usr/local/bin/wificorrect", "ctl", "ag-gecis", &staged.to_string_lossy()]));
        if !ok {
            let _ = std::fs::remove_file(&staged);
            return redirect("/portlar", Some(("Değişiklik başlatılamadı.", true)));
        }
        // kullanıcı adı ve parola denetime yazılmaz: yalnızca açık/kapalı, VLAN ve değişip değişmedikleri
        let pppoe_ozet = format!(
            "pppoe={} vlan={} kullanici_degisti={} parola_degisti={}",
            if new.pppoe.acik { "acik" } else { "kapali" },
            new.pppoe.vlan,
            u8::from(new.pppoe.kullanici != cur.pppoe.kullanici),
            u8::from(new.pppoe.parola != cur.pppoe.parola)
        );
        self.audit(cfg, req, Some(o), "PANEL_PORT", &format!("{} {pppoe_ozet}", ag::describe(&new, &eths)));
        redirect("/portlar", Some(("Uygulanıyor. Birkaç saniye sonra bu sayfayı yenileyip 3 dakika içinde onaylayın.", false)))
    }

    pub(super) fn portlar_onayla(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        match ag::onayla(cfg, &self.ag_yollar, &*self.runner) {
            Ok(()) => {
                self.audit(cfg, req, Some(o), "PANEL_PORT_ONAY", "");
                redirect("/portlar", Some(("Onaylandı; yeni ayar kalıcı.", false)))
            }
            Err(e) => redirect("/portlar", Some((&e, true))),
        }
    }

    pub(super) fn portlar_geri_al(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        if ag::pending(&self.ag_yollar).is_none() {
            return redirect("/portlar", Some(("Onay bekleyen değişiklik yok.", true)));
        }
        let unit = format!("wfc-ag-geri-al-elle-{}", ortak::random_hex(4));
        let ok = (self.runner)(&cmd(&["systemd-run", "--unit", &unit, "--on-active", "2s", "/usr/local/bin/wificorrect", "ctl", "ag-geri-al", "elle"]));
        self.audit(cfg, req, Some(o), "PANEL_PORT_GERI_AL", "");
        redirect("/portlar", Some(if ok { ("Eski ayar geri yükleniyor.", false) } else { ("Geri alma başlatılamadı!", true) }))
    }

    pub(super) fn portlar_etiket(&self, cfg: &Config, req: &Req, o: &Oturum) -> Resp {
        let y = &self.ag_yollar;
        if ag::pending(y).is_some() {
            return redirect("/portlar", Some(("Önce bekleyen değişikliği onaylayın ya da geri alın.", true)));
        }
        let mut cur: Ag = ag::load(&y.ag);
        cur.etiket_ters = !cur.etiket_ters;
        if let Err(e) = ag::save(&y.ag, &cur) {
            return redirect("/portlar", Some((&e, true)));
        }
        self.audit(cfg, req, Some(o), "PANEL_PORT_ETIKET", &format!("ters={}", cur.etiket_ters));
        redirect("/portlar", Some(("Sağ/sol etiketi değiştirildi.", false)))
    }
}
