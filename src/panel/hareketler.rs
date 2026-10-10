//! Panel hareketleri (yalnızca admin): panel hesaplarının ne zaman neye baktığı, denetim kaydından (denetim.csv).
//! Amaç: kafe sahibinin müşterileri sürekli izleyip izlemediği hizmet sağlayıcı tarafından denetlenebilsin.
//! Kayıtlar her gün mühürlenir; panelden silinemez, değiştirilemez.

use super::*;
use crate::kayit;

/// Kişisel veri gösteren ya da veren işlemler (izleme sayılır).
const KISISEL: &[&str] = &[
    "PANEL_OZET", "PANEL_OTURUMLAR", "PANEL_OTURUMLAR_YENILE", "PANEL_KULLANICILAR", "PANEL_KULLANICI_ARA", "PANEL_KULLANICI", "PANEL_KAYIT_GUN",
    "PANEL_KAYIT_GORUNTULE", "PANEL_KAYIT_INDIR", "PANEL_TALEP_ARA", "PANEL_TALEP_PAKET", "PANEL_GEREKCE", "PANEL_YAVASLATILMIS",
];

pub(super) fn olay_adi(olay: &str) -> &str {
    match olay {
        "PANEL_GIRIS" => "Giriş",
        "PANEL_GEREKCE" => "Gerekçe yazdı",
        "PANEL_CIKIS" => "Çıkış",
        "PANEL_GIRIS_HATA" => "Hatalı giriş",
        "PANEL_GIRIS_KILIT" => "Giriş kilitlendi",
        "PANEL_OZET" => "Özete baktı (bağlı kullanıcılar, yeni üyeler)",
        "PANEL_OTURUMLAR" => "Bağlı kullanıcılara baktı",
        "PANEL_OTURUMLAR_YENILE" => "Otomatik yenilemeyi açtı",
        "PANEL_KULLANICILAR" => "Kullanıcı listesine baktı",
        "PANEL_KULLANICI_ARA" => "Kullanıcı aradı",
        "PANEL_KULLANICI" => "Kişi sayfasına baktı",
        "PANEL_KAYIT_GUN" => "Günün dosyalarına baktı",
        "PANEL_KAYIT_GORUNTULE" => "Kayıt dosyasını görüntüledi",
        "PANEL_KAYIT_INDIR" => "Kayıt indirdi",
        "PANEL_TALEP_ARA" => "Resmi talep araması",
        "PANEL_TALEP_PAKET" => "Talep paketi indirdi",
        "PANEL_AT" => "Bağlantı kesti",
        "PANEL_YAVASLAT" => "Yavaşlattı",
        "PANEL_YAVASLATMA_KALDIR" => "Yavaşlatmayı kaldırdı",
        "PANEL_YAVASLATILMIS" => "Yavaşlatılmış cihazlara baktı",
        "PANEL_AYAR" => "Ayar değiştirdi",
        "PANEL_PORT" => "Port değiştirdi",
        "PANEL_PORTAL_METIN" => "Portal metnini değiştirdi",
        "PANEL_YEDEKLE" => "Yedeklemeyi başlattı",
        "PANEL_FABRIKA" => "Fabrika ayarlarını başlattı",
        "PANEL_FABRIKA_RED" => "Fabrika ayarları reddedildi",
        "PANEL_ADMIN_ACILDI" => "Admin kilidini açtı",
        "PANEL_ADMIN_HATALI" => "Hatalı admin parolası",
        "PANEL_ADMIN_KAPANDI" => "Admin kilidini kapattı",
        o => o.strip_prefix("PANEL_").unwrap_or(o),
    }
}

/// `kullanici=X rol=Y ip=Z ayrıntı…` → (kullanıcı, rol, ip, ayrıntı). Eski kayıtlarda rol yok.
pub(super) fn parse_ek(ek: &str) -> (String, String, String, String) {
    let mut rest = ek;
    let mut take = |key: &str| -> String {
        let Some(r) = rest.strip_prefix(key) else { return String::new() };
        let (v, tail) = r.split_once(' ').unwrap_or((r, ""));
        rest = tail;
        v.to_string()
    };
    let user = take("kullanici=");
    let rol = take("rol=");
    let ip = take("ip=");
    (user, rol, ip, rest.to_string())
}

impl Panel {
    pub(super) fn hareketler(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let gun: usize = req.query.get("gun").and_then(|g| g.parse().ok()).unwrap_or(30).clamp(1, 365);
        let kim = req.query.get("kim").map_or("", |s| s.as_str());
        let sadece_kisisel = req.query.get("kisisel").is_some_and(|v| v == "1");
        let since = ortak::day_of(&ortak::now_iso(now - (gun as f64 - 1.0) * 86400.0)).to_string();
        let root = &cfg.main.log_root;
        let mut rows: Vec<(String, String, String, String, String, String)> = vec![];
        for day in kayit::days_desc(root).into_iter().filter(|d| *d >= since) {
            for r in kayit::day_rows(root, &day, "denetim.csv") {
                let olay = kayit::col(&r, "olay");
                if !olay.starts_with("PANEL_") {
                    continue;
                }
                let (user, rol, ip, detail) = parse_ek(kayit::col(&r, "ek"));
                rows.push((kayit::col(&r, "zaman").to_string(), user, rol, olay.to_string(), ip, detail));
            }
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0));
        // kişi başına özet: kişisel veriye bakma sayısı (son 7 gün / seçilen dönem)
        let week = ortak::day_of(&ortak::now_iso(now - 6.0 * 86400.0)).to_string();
        let mut ozet: std::collections::BTreeMap<String, (String, usize, usize, String)> = Default::default();
        for (z, user, rol, olay, _, _) in &rows {
            if user.is_empty() || user == "-" {
                continue;
            }
            let e = ozet.entry(user.clone()).or_insert((rol.clone(), 0, 0, String::new()));
            if e.0.is_empty() {
                e.0 = rol.clone();
            }
            if KISISEL.contains(&olay.as_str()) && olay != "PANEL_GEREKCE" {
                e.2 += 1;
                if z.as_str() >= week.as_str() {
                    e.1 += 1;
                }
            }
            if olay == "PANEL_GIRIS" && e.3.is_empty() {
                e.3 = z.get(..16).unwrap_or("").replace('T', " ");
            }
        }
        let ozet_rows: Vec<Vec<String>> = ozet
            .iter()
            .map(|(u, (rol, w, all, last))| {
                vec![
                    format!("<a href=\"/panel-hareketleri?kim={}&amp;gun={gun}&amp;kisisel=1\">{}</a>", pct(u), h(u)),
                    h(match rol.as_str() {
                        "sahip" => "İşletme sahibi",
                        "hizmet" => "Hizmet sağlayıcı",
                        _ => "?",
                    }),
                    w.to_string(),
                    all.to_string(),
                    h(last),
                ]
            })
            .collect();
        let shown: Vec<Vec<String>> = rows
            .iter()
            .filter(|r| kim.is_empty() || r.1 == kim)
            .filter(|r| !sadece_kisisel || KISISEL.contains(&r.3.as_str()))
            .take(1000)
            .map(|(z, user, _, olay, ip, detail)| {
                let cls = if KISISEL.contains(&olay.as_str()) { " class=\"durum kotu\"" } else { "" };
                vec![h(&z.get(..19).unwrap_or("").replace('T', " ")), h(user), format!("<span{cls}>{}</span>", h(olay_adi(olay))), h(detail), h(ip)]
            })
            .collect();
        let body = format!(
            "<p class=\"not\">Panel hesaplarının yaptığı her şey denetim kaydına yazılır; kayıt her gece mühürlenir, panelden silinemez. \
             Kırmızı satırlar müşterilerin kişisel verisine bakma işlemleridir.</p>\
             <section class=\"kart\"><h2>Hesap başına (son {gun} gün)</h2>{}</section>\
             <form class=\"satir kart\" method=\"get\" action=\"/panel-hareketleri\">\
             <div><label for=\"kim\">Hesap</label><input type=\"text\" id=\"kim\" name=\"kim\" value=\"{}\" placeholder=\"hepsi\"></div>\
             <div><label for=\"gun\">Gün</label><input type=\"number\" id=\"gun\" name=\"gun\" value=\"{gun}\"></div>\
             <label class=\"secim\"><input type=\"checkbox\" name=\"kisisel\" value=\"1\"{}> Yalnızca kişisel veri</label><button>Göster</button></form>{}",
            table(&["Hesap", "Rol", "Kişisel veriye bakma (7 gün)", "Dönem toplamı", "Son giriş"], &ozet_rows, "Kayıt yok"),
            h(kim),
            if sadece_kisisel { " checked" } else { "" },
            table(&["Zaman", "Hesap", "İşlem", "Ayrıntı", "IP"], &shown, "Kayıt yok")
        );
        self.page(cfg, req, Some(o), "Panel hareketleri", &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_detail_parsing() {
        assert_eq!(parse_ek("kullanici=mudur rol=sahip ip=192.168.1.5 gun=2026-09-27 dosya=dns.csv.gz"), ("mudur".into(), "sahip".into(), "192.168.1.5".into(), "gun=2026-09-27 dosya=dns.csv.gz".into()));
        assert_eq!(parse_ek("kullanici=admin ip=192.168.1.5"), ("admin".into(), String::new(), "192.168.1.5".into(), String::new())); // eski biçim
        assert_eq!(olay_adi("PANEL_KAYIT_INDIR"), "Kayıt indirdi");
    }
}
