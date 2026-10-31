//! Panel: Ayarlar → Raporlar (0.4.28; kullanıcı 2026-10-31: "raporlar kısmı çok karmaşık ve anlaşılmaz … günlük, haftalık ve aylık
//! rapor; toplam kullanılan veri, en çok süre bağlı kalanlar, en çok veri tüketenler olsun; sadeleştir"). Veri cihazın kendi
//! kayıtlarından (src/rapor.rs), merkeze sorulmaz. Sayfa: Günlük / Haftalık / Aylık sekmeleri, dönem okları, iki sayı (toplam
//! kullanılan veri, kullanıcı) ve iki sıralama (ilk 10). Kişi adı gösterdiği için her bakış denetime yazılır (`PANEL_RAPOR`; panel
//! hareketlerinde kişisel veri). Sıralamalarda yalnızca kişiler (izinli cihazlar toplam veride, sıralamada değil). JS yok.

use super::*;
use crate::rapor::{GunOzeti, Kisi};
use std::collections::BTreeMap;

const AYLAR: [&str; 12] = ["Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık"];
const GUN_ADI: [&str; 7] = ["Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar"];
/// Sıralamalarda gösterilen kişi
const SIRA: usize = 10;
/// Açık günün (bugün, mühürlenmemiş dün) özeti bellekte bu kadar tutulur (sn)
const ACIK_GUN_SN: f64 = 60.0;
const SIMGE_SOL: &str = "M15 18l-6-6 6-6";
const SIMGE_SAG: &str = "M9 18l6-6-6-6";

// --- tarih: gün numarası (1970-01-01 = 0; H. Hinnant'ın takvim dönüşümü) ---
fn gun_no(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn tarih(n: i64) -> (i64, u32, u32) {
    let z = n + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn gun_metni(n: i64) -> String {
    let (y, m, d) = tarih(n);
    format!("{y:04}-{m:02}-{d:02}")
}

fn ayin_gunleri(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn rakamlar(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

/// "YYYY-MM" (ASCII rakam, geçerli ay) → (yıl, ay)
fn ay_coz(s: &str) -> Option<(i64, u32)> {
    let (y, m) = s.split_once('-')?;
    if y.len() != 4 || m.len() != 2 || !rakamlar(y) || !rakamlar(m) {
        return None;
    }
    let (y, m) = (y.parse().ok()?, m.parse().ok()?);
    (1..=12).contains(&m).then_some((y, m))
}

/// "YYYY-MM-DD" (geçerli takvim günü) → gün numarası
fn gun_coz(s: &str) -> Option<i64> {
    let (ay, g) = s.split_at_checked(7)?;
    let g = g.strip_prefix('-')?;
    let (y, m) = ay_coz(ay)?;
    if g.len() != 2 || !rakamlar(g) {
        return None;
    }
    let d: u32 = g.parse().ok()?;
    (1..=ayin_gunleri(y, m)).contains(&d).then(|| gun_no(y, m, d))
}

/// Pazartesi = 0
fn haftanin_gunu(n: i64) -> usize {
    (n + 3).rem_euclid(7) as usize
}

fn gun_adi(n: i64) -> String {
    let (y, m, d) = tarih(n);
    format!("{d} {} {y}", AYLAR[m as usize - 1])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Donem {
    Gun(i64),
    /// haftanın pazartesisi
    Hafta(i64),
    Ay(i64, u32),
}

impl Donem {
    /// İlk ve son gün (ikisi de dahil)
    fn aralik(self) -> (i64, i64) {
        match self {
            Donem::Gun(n) => (n, n),
            Donem::Hafta(p) => (p, p + 6),
            Donem::Ay(y, m) => (gun_no(y, m, 1), gun_no(y, m, ayin_gunleri(y, m))),
        }
    }

    fn onceki(self) -> Donem {
        match self {
            Donem::Gun(n) => Donem::Gun(n - 1),
            Donem::Hafta(p) => Donem::Hafta(p - 7),
            Donem::Ay(y, 1) => Donem::Ay(y - 1, 12),
            Donem::Ay(y, m) => Donem::Ay(y, m - 1),
        }
    }

    fn sonraki(self) -> Donem {
        match self {
            Donem::Gun(n) => Donem::Gun(n + 1),
            Donem::Hafta(p) => Donem::Hafta(p + 7),
            Donem::Ay(y, 12) => Donem::Ay(y + 1, 1),
            Donem::Ay(y, m) => Donem::Ay(y, m + 1),
        }
    }

    /// Aynı türün `n` gününü içeren dönemi
    fn ayni_tur(self, n: i64) -> Donem {
        let (y, m, _) = tarih(n);
        match self {
            Donem::Gun(_) => Donem::Gun(n),
            Donem::Hafta(_) => Donem::Hafta(n - haftanin_gunu(n) as i64),
            Donem::Ay(..) => Donem::Ay(y, m),
        }
    }

    fn adres(self) -> String {
        match self {
            Donem::Gun(n) => format!("/raporlar?gun={}", gun_metni(n)),
            Donem::Hafta(p) => format!("/raporlar?hafta={}", gun_metni(p)),
            Donem::Ay(y, m) => format!("/raporlar?ay={y:04}-{m:02}"),
        }
    }

    fn baslik(self) -> String {
        match self {
            Donem::Gun(n) => gun_adi(n),
            Donem::Hafta(p) => {
                let ((y1, m1, d1), (y2, m2, d2)) = (tarih(p), tarih(p + 6));
                let ay = |m: u32| AYLAR[m as usize - 1];
                if y1 != y2 {
                    format!("{d1} {} {y1} – {d2} {} {y2}", ay(m1), ay(m2))
                } else if m1 != m2 {
                    format!("{d1} {} – {d2} {} {y2}", ay(m1), ay(m2))
                } else {
                    format!("{d1} – {d2} {} {y2}", ay(m2))
                }
            }
            Donem::Ay(y, m) => format!("{} {y}", AYLAR[m as usize - 1]),
        }
    }

    /// Başlığın altındaki kısa not: gün adı, "bugün", "bu hafta", "geçen ay" …
    fn alt_baslik(self, bugun: i64) -> String {
        let simdi = self.ayni_tur(bugun);
        let goreli = if self == simdi {
            Some(match self {
                Donem::Gun(_) => "bugün",
                Donem::Hafta(_) => "bu hafta",
                Donem::Ay(..) => "bu ay",
            })
        } else if self == simdi.onceki() {
            Some(match self {
                Donem::Gun(_) => "dün",
                Donem::Hafta(_) => "geçen hafta",
                Donem::Ay(..) => "geçen ay",
            })
        } else {
            None
        };
        match (self, goreli) {
            (Donem::Gun(n), Some(g)) => format!("{} · {g}", GUN_ADI[haftanin_gunu(n)]),
            (Donem::Gun(n), None) => GUN_ADI[haftanin_gunu(n)].to_string(),
            (_, Some(g)) => g.to_string(),
            (Donem::Hafta(_), None) => "Pazartesi – Pazar".to_string(),
            (Donem::Ay(..), None) => String::new(),
        }
    }
}

/// Sorgudan dönem: `gun=YYYY-MM-DD`, `hafta=YYYY-MM-DD` (o günün haftası), `ay=YYYY-MM`; yoksa bugün. Gelecek dönem bugünün
/// dönemine çekilir. Bozuk değer: None.
fn donem_coz(q: &Form, bugun: i64) -> Option<Donem> {
    let d = if let Some(v) = q.get("gun") {
        Donem::Gun(gun_coz(v)?)
    } else if let Some(v) = q.get("hafta") {
        let n = gun_coz(v)?;
        Donem::Hafta(n - haftanin_gunu(n) as i64)
    } else if let Some(v) = q.get("ay") {
        let (y, m) = ay_coz(v)?;
        Donem::Ay(y, m)
    } else {
        Donem::Gun(bugun)
    };
    Some(if d.aralik().0 > bugun { d.ayni_tur(bugun) } else { d })
}

/// 3 anlamlı basamak, ondalık virgül: 4,12 GB · 850 MB · 12,3 MB
fn veri_metni(b: u64) -> String {
    if b >= 1_000_000_000 {
        return format!("{} GB", gb(b));
    }
    if b < 1_000_000 {
        return if b == 0 { "0".into() } else { "1 MB'tan az".into() };
    }
    let mb = b as f64 / 1e6;
    let s = if mb < 9.995 {
        format!("{mb:.2}")
    } else if mb < 99.95 {
        format!("{mb:.1}")
    } else {
        format!("{mb:.0}")
    };
    format!("{} MB", s.replace('.', ","))
}

fn sure_metni(sn: u64) -> String {
    let dk = sn / 60;
    if dk == 0 {
        "1 dk'dan az".into()
    } else if dk < 60 {
        format!("{dk} dk")
    } else if dk < 1440 {
        format!("{} sa {} dk", dk / 60, dk % 60)
    } else {
        format!("{} gün {} sa", dk / 1440, dk % 1440 / 60)
    }
}

/// Kişi anahtarının ekrandaki maskeli biçimi: telefon `+90 5XX XXX XX 32`, T.C./pasaport `123******01`
fn maskeli(anahtar: &str) -> String {
    if crate::kimlik::kimlik_mi(anahtar) {
        crate::kimlik::maskele(anahtar)
    } else if rakamlar(anahtar) {
        crate::ulkeler::mask_phone(anahtar)
    } else {
        anahtar.to_string()
    }
}

/// İlk 10 kişi: (ad, maskeli kimlik, değer), büyükten küçüğe; eşitlikte anahtar sırası.
fn sirala<'a>(kisiler: &[(&'a String, &Kisi)], deger: fn(&Kisi) -> u64) -> Vec<(&'a String, u64)> {
    let mut v: Vec<(&String, u64)> = kisiler.iter().map(|(k, d)| (*k, deger(d))).filter(|(_, n)| *n > 0).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    v.truncate(SIRA);
    v
}

fn sira_html(baslik: &str, sira: &[(&String, u64)], adlar: &BTreeMap<String, [String; 5]>, bicim: fn(u64) -> String) -> String {
    if sira.is_empty() {
        return format!("<section class=\"kutu\"><h2>{}</h2><p class=\"not\">Bu dönemde kayıt yok.</p></section>", h(baslik));
    }
    let en = sira[0].1.max(1);
    let satirlar: String = sira
        .iter()
        .enumerate()
        .map(|(i, (k, n))| {
            let ad = adlar.get(k.as_str()).map(|r| format!("{} {}", r[1], r[2]).trim().to_string()).filter(|a| !a.is_empty());
            let kimlik = maskeli(k);
            let (ust, alt) = match ad {
                Some(a) => (a, kimlik),
                None => (kimlik, String::new()),
            };
            let alt = if alt.is_empty() { String::new() } else { format!("<small>{}</small>", h(&alt)) };
            format!(
                "<li><span class=\"sira-no\">{}</span><span class=\"sira-metin\"><span class=\"sira-ust\"><b>{}</b>\
                 <span class=\"sira-deger\">{}</span></span>{alt}<span class=\"cubuk\"><span style=\"width:{}%\"></span></span></span></li>",
                i + 1,
                h(&ust),
                h(&bicim(*n)),
                (n.saturating_mul(100) / en).max(2)
            )
        })
        .collect();
    format!("<section class=\"kutu\"><h2>{}</h2><ol class=\"siralama\">{satirlar}</ol></section>", h(baslik))
}

impl Panel {
    /// Günün özeti: mühürlü gün diskteki önbellekten (src/rapor.rs), açık gün bellekte en çok 1 dk.
    fn rapor_gunu(&self, cfg: &Config, gun: &str, now: f64) -> GunOzeti {
        if let Some((t, o)) = self.rapor_onbellek.lock().unwrap().get(gun) {
            if (0.0..ACIK_GUN_SN).contains(&(now - t)) {
                return o.clone();
            }
        }
        let (o, kesin) = crate::rapor::gun_ozeti(cfg, gun);
        if !kesin {
            let mut m = self.rapor_onbellek.lock().unwrap();
            m.retain(|_, (t, _)| (0.0..ACIK_GUN_SN).contains(&(now - *t)));
            m.insert(gun.to_string(), (now, o.clone()));
        }
        o
    }

    pub(super) fn raporlar(&self, cfg: &Config, req: &Req, o: &Oturum, now: f64) -> Resp {
        let bugun = gun_coz(ortak::day_of(&ortak::now_iso(now))).unwrap_or(0);
        let Some(donem) = donem_coz(&req.query, bugun) else {
            let mut r = self.page(cfg, req, Some(o), "Raporlar", "<p>Bu dönem için rapor yok. <a href=\"/raporlar\">Bugünün raporu</a></p>");
            r.status = 404;
            return r;
        };
        self.audit(cfg, req, Some(o), "PANEL_RAPOR", donem.adres().trim_start_matches("/raporlar?"));
        let (bas, bit) = donem.aralik();
        let ilk = crate::muhur::days(&cfg.main.log_root).iter().find_map(|d| gun_coz(d)).unwrap_or(bugun);
        let mut toplam: BTreeMap<String, Kisi> = BTreeMap::new();
        for n in bas.max(ilk)..=bit.min(bugun) {
            for (k, v) in self.rapor_gunu(cfg, &gun_metni(n), now) {
                let e = toplam.entry(k).or_default();
                e.bayt = e.bayt.saturating_add(v.bayt);
                e.sn = e.sn.saturating_add(v.sn);
            }
        }
        let veri: u64 = toplam.values().fold(0, |a, k| a.saturating_add(k.bayt));
        let kisiler: Vec<(&String, &Kisi)> = toplam.iter().filter(|(k, _)| !k.starts_with("mac:")).collect();
        let adlar = ortak::read_index(&cfg.main.log_root);
        // sekmeler, seçili dönemin bugüne kadarki son gününü içerir (haftalığa geçince o günün haftası)
        let ref_gun = bit.min(bugun).max(bas);
        let sekme = |d: Donem, ad: &str| {
            let secili = std::mem::discriminant(&d) == std::mem::discriminant(&donem);
            format!("<a href=\"{}\"{}>{ad}</a>", d.adres(), if secili { " class=\"secili\" aria-current=\"page\"" } else { "" })
        };
        let ok = |d: Donem, var: bool, sinif: &str, etiket: &str, yol: &str| {
            if var {
                format!("<a class=\"ok {sinif}\" href=\"{}\" title=\"{etiket}\" aria-label=\"{etiket}\">{}</a>", d.adres(), simge(yol))
            } else {
                format!("<span class=\"ok {sinif} bos\" aria-hidden=\"true\">{}</span>", simge(yol))
            }
        };
        let (onceki, sonraki) = match donem {
            Donem::Gun(_) => ("Önceki gün", "Sonraki gün"),
            Donem::Hafta(_) => ("Önceki hafta", "Sonraki hafta"),
            Donem::Ay(..) => ("Önceki ay", "Sonraki ay"),
        };
        let alt = donem.alt_baslik(bugun);
        let govde = format!(
            "<div class=\"rapor\"><section class=\"kutu rapor-ust\"><nav class=\"sekmeler\" aria-label=\"Rapor türü\">{}{}{}</nav>\
             <div class=\"donem\">{}<div class=\"donem-ad\"><h2>{}</h2>{}</div>{}</div>\
             <dl class=\"rapor-sayilar\"><div><dt>Toplam kullanılan veri</dt><dd>{}</dd></div><div><dt>Kullanıcı</dt><dd>{}</dd></div></dl></section>\
             {}{}<p class=\"not rapor-not\">Kullanılan veri indirme ve yüklemenin toplamıdır; bağlı kalma, cihazın Wi-Fi ağında göründüğü \
             süredir. Bu sayfaya her bakış, bakanın adıyla kaydedilir.</p></div>",
            sekme(Donem::Gun(ref_gun), "Günlük"),
            sekme(Donem::Hafta(0).ayni_tur(ref_gun), "Haftalık"),
            sekme(Donem::Ay(0, 1).ayni_tur(ref_gun), "Aylık"),
            ok(donem.onceki(), bas > ilk, "sol", onceki, SIMGE_SOL),
            h(&donem.baslik()),
            if alt.is_empty() { String::new() } else { format!("<small>{}</small>", h(&alt)) },
            ok(donem.sonraki(), bit < bugun, "sag", sonraki, SIMGE_SAG),
            if veri == 0 { "0".to_string() } else { veri_metni(veri) },
            kisiler.iter().filter(|(_, k)| k.bayt > 0 || k.sn > 0).count(),
            sira_html("En çok veri kullananlar", &sirala(&kisiler, |k| k.bayt), &adlar, veri_metni),
            sira_html("En uzun bağlı kalanlar", &sirala(&kisiler, |k| k.sn), &adlar, sure_metni),
        );
        self.cerceve(cfg, req, Some(o), "Raporlar", Duzen::Serbest(&govde), "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takvim_donusumu() {
        assert_eq!(gun_no(1970, 1, 1), 0);
        assert_eq!(gun_metni(gun_no(2026, 10, 31)), "2026-10-31");
        for n in [-1, 0, 59, 60, 11_016, 20_725, 20_757, 36_525] {
            let (y, m, d) = tarih(n);
            assert_eq!(gun_no(y, m, d), n);
        }
        assert_eq!(haftanin_gunu(gun_no(2026, 10, 31)), 5); // cumartesi
        assert_eq!(haftanin_gunu(gun_no(2026, 10, 26)), 0); // pazartesi
        assert_eq!(gun_coz("2028-02-29"), Some(gun_no(2028, 2, 29)));
        for bozuk in ["2026-02-29", "2026-13-01", "2026-1-01", "26-10-31", "2026-10-31x", "２０２６-10-31", "", "2026-10-3a", "2026-10"] {
            assert_eq!(gun_coz(bozuk), None, "{bozuk}");
        }
        assert_eq!(ay_coz("2026-10"), Some((2026, 10)));
        assert_eq!((ay_coz("2026-00"), ay_coz("2026-1"), ay_coz("2026-10-01")), (None, None, None));
    }

    #[test]
    fn donemler_ve_basliklar() {
        let bugun = gun_no(2026, 10, 31);
        let q = |k: &str, v: &str| donem_coz(&Form::from([(k.to_string(), v.to_string())]), bugun);
        assert_eq!(donem_coz(&Form::new(), bugun), Some(Donem::Gun(bugun)));
        assert_eq!(q("hafta", "2026-10-29"), Some(Donem::Hafta(gun_no(2026, 10, 26))));
        assert_eq!(q("gun", "2026-11-05"), Some(Donem::Gun(bugun))); // gelecek → bugün
        assert_eq!(q("ay", "2027-01"), Some(Donem::Ay(2026, 10)));
        assert_eq!((q("gun", "dün"), q("ay", "x")), (None, None));
        assert_eq!(Donem::Gun(bugun).baslik(), "31 Ekim 2026");
        assert_eq!(Donem::Gun(bugun).alt_baslik(bugun), "Cumartesi · bugün");
        assert_eq!(Donem::Gun(bugun - 1).alt_baslik(bugun), "Cuma · dün");
        assert_eq!(Donem::Hafta(gun_no(2026, 10, 26)).baslik(), "26 Ekim – 1 Kasım 2026");
        assert_eq!(Donem::Hafta(gun_no(2026, 10, 19)).baslik(), "19 – 25 Ekim 2026");
        assert_eq!(Donem::Hafta(gun_no(2025, 12, 29)).baslik(), "29 Aralık 2025 – 4 Ocak 2026");
        assert_eq!(Donem::Hafta(gun_no(2026, 10, 19)).alt_baslik(bugun), "geçen hafta");
        assert_eq!((Donem::Ay(2026, 1).onceki(), Donem::Ay(2026, 12).sonraki()), (Donem::Ay(2025, 12), Donem::Ay(2027, 1)));
        assert_eq!(Donem::Ay(2026, 2).aralik(), (gun_no(2026, 2, 1), gun_no(2026, 2, 28)));
        assert_eq!(Donem::Ay(2026, 9).alt_baslik(bugun), "geçen ay");
        assert_eq!(Donem::Hafta(gun_no(2026, 10, 26)).adres(), "/raporlar?hafta=2026-10-26");
    }

    #[test]
    fn bicimler() {
        assert_eq!(veri_metni(4_120_000_000), "4,12 GB");
        assert_eq!(veri_metni(850_400_000), "850 MB");
        assert_eq!(veri_metni(12_340_000), "12,3 MB");
        assert_eq!(veri_metni(1_234_000), "1,23 MB");
        assert_eq!(veri_metni(999_999), "1 MB'tan az");
        assert_eq!(sure_metni(30), "1 dk'dan az");
        assert_eq!(sure_metni(45 * 60), "45 dk");
        assert_eq!(sure_metni(200 * 60), "3 sa 20 dk");
        assert_eq!(sure_metni(3000 * 60), "2 gün 2 sa");
        assert_eq!(maskeli("905334553132"), "+90 5XX XXX XX 32");
        assert_eq!(maskeli("TC:12345678901"), "123******01");
    }
}
