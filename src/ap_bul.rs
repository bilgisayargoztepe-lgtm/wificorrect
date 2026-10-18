//! AP kurulumu → "Takılan cihaz" (spec 2026-10-16-ap-arayuz): etiketteki adresle **Bağlan** ya da yaygın fabrika
//! adresleriyle **Otomatik bul**. Panel `systemd-run --unit wfc-ap-bul … ctl ap-bul` ile arka planda başlatır (panelin
//! iş parçacıkları beklemez; sayfa kendini yeniler, JS yok); durum `/run/wificorrect/ap-bul.json`.
//! - Web arayüzü (`ap_arayuz::web_bul`): 443'te aktarımın TLS'iyle el sıkışma olursa HTTPS, olmazsa 80'e TCP açıksa
//!   HTTP (2026-10-18, canlı ZyXEL: sayfa HTTPS'te açıldığı için AP'nin betiği istekleri şifrelemez, AP'ye de HTTPS ile
//!   gidilmeli).
//! - Misafir ağındaki adres (ör. 10.50.0.2): yan oda gerekmez, web arayüzü ana ad alanından denenir.
//! - Diğerleri: kısa ömürlü yan odadan ARP yoklaması (gönderen 0.0.0.0): hedef cevap veriyor mu, aynı /24'te yan oda
//!   için boş adres (.250, doluysa .249 … ; "boş" = ARP'ye cevap yok ve hedefin kendisi değil). Hedef yoklamaya cevap
//!   vermezse bir kez de boş adresten sorulur (yalnızca kendi ağından gelen ARP'ye cevap veren cihazlar için).
//!   Cevap veren hedef için yan oda açılır (`ap::ekle`, 30 dk kuralları; aynı ağda açık yan oda varsa o kullanılır,
//!   başka ağdaysa kapanır), sonra web arayüzü aracı üzerinden denenir.
//! - "Otomatik bul" cevap veren ilk adreste yan odayı açık bırakır; web arayüzü olmayan adreste yan odayı kapatıp
//!   sıradakine geçer.

use crate::ap::{self, Gecici, Yollar};
use crate::ayar::Config;
use crate::ortak;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::Ipv4Addr;

/// Arama işinin geçici systemd birimi (aynı anda tek arama)
pub const BIRIM: &str = "wfc-ap-bul";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Durum {
    /// baglan | otomatik
    pub tur: String,
    /// suruyor | bulundu | web_yok | yok | hata
    pub durum: String,
    pub hedefler: Vec<String>,
    pub baslangic: f64,
    pub bitis: f64,
    /// Bulunan cihaz: adres, web portu, şema, yol (yan_oda | misafir), yan odanın adresi (a.b.c.d/24)
    pub adres: String,
    pub port: u16,
    pub sema: String,
    pub yol: String,
    pub yan_oda: String,
    pub mesaj: String,
    pub kim: String,
    pub rol: String,
}

impl Durum {
    pub fn basla(hedef: Option<Ipv4Addr>, kim: &str, rol: &str, now: f64) -> Durum {
        let hedefler = match hedef {
            Some(h) => vec![h.to_string()],
            None => ap::OTOMATIK.iter().map(|o| Ipv4Addr::from(*o).to_string()).collect(),
        };
        Durum {
            tur: if hedef.is_some() { "baglan" } else { "otomatik" }.into(),
            durum: "suruyor".into(),
            hedefler,
            baslangic: now,
            kim: kim.into(),
            rol: rol.into(),
            ..Default::default()
        }
    }
}

pub fn oku(y: &Yollar) -> Option<Durum> {
    serde_json::from_slice(&std::fs::read(&y.bul).ok()?).ok()
}

pub fn kaydet(y: &Yollar, d: &Durum) -> Result<(), String> {
    let v = serde_json::to_vec(d).map_err(|e| e.to_string())?;
    ortak::write_atomic_mode(&y.bul, &v, 0o600).map_err(|e| format!("arama durumu yazılamadı: {e}"))
}

/// Arama işi çalışıyor mu (`systemctl is-active`)
pub fn calisiyor(ip_oku: &ap::IpOku) -> bool {
    matches!(ip_oku(&["systemctl", "is-active", &format!("{BIRIM}.service")]).trim(), "active" | "activating" | "deactivating" | "reloading")
}

/// Panelin başlattığı komut: `ctl ap-bul KIM ROL IP [HEDEF]` (RuntimeMaxSec: takılırsa öldürülür)
pub fn komut(kim: &str, rol: &str, ip: &str, hedef: Option<Ipv4Addr>) -> Vec<String> {
    let mut v: Vec<String> = ["systemd-run", "--collect", "--unit", BIRIM, "--property=RuntimeMaxSec=180", "/usr/local/bin/wificorrect", "ctl", "ap-bul", kim, rol, ip]
        .into_iter()
        .map(String::from)
        .collect();
    v.extend(hedef.map(|h| h.to_string()));
    v
}

/// Arama işinin dış dünyası (testte sahte)
pub struct Araclar<'a> {
    /// ARP taraması: (hedef, gönderen) istekleri → cevap veren hedefler
    pub arp: &'a dyn Fn(&[(Ipv4Addr, Ipv4Addr)]) -> Result<BTreeMap<Ipv4Addr, String>, String>,
    /// Açık (aracısı çalışan) yan oda
    pub yan_oda: &'a dyn Fn() -> Option<Gecici>,
    /// Yan oda aç (a.b.c.d/24)
    pub ac: &'a dyn Fn(&str) -> Result<Gecici, String>,
    /// Açık yan odayı kapat (neden denetim için); kapanan adres
    pub kapat: &'a dyn Fn(&str) -> Result<Option<String>, String>,
    /// Hedefin web arayüzü (yan odadan: aracı üzerinden) → port (443 = https, 80 = http); gerçeği `ap_arayuz::web_portu`
    pub web: &'a dyn Fn(Ipv4Addr, bool) -> Option<u16>,
    /// Denetim satırı (olay, ek); kim / rol / IP'yi çağıran ekler
    pub denetim: &'a dyn Fn(&str, String),
    /// Ara durum (sayfa "aranıyor" gösterir)
    pub kaydet: &'a dyn Fn(&Durum),
}

fn bitir(mut d: Durum, durum: &str, mesaj: String, now: f64) -> Durum {
    d.durum = durum.into();
    d.mesaj = mesaj;
    d.bitis = now;
    d
}

fn sema(port: u16) -> &'static str {
    if port == 443 { "https" } else { "http" }
}

/// Hedef(ler) için yan oda boş adresi seçimi: (cevap veren hedefler sırayla, her biri için boş adres ya da None)
fn arp_sec(hedefler: &[Ipv4Addr], a: &Araclar) -> Result<Vec<(Ipv4Addr, Option<Ipv4Addr>)>, String> {
    let sifir = Ipv4Addr::UNSPECIFIED;
    let tek = hedefler.len() == 1;
    // tek hedefte (Bağlan) adaylar aynı taramada; çok hedefte (Otomatik) önce yalnızca hedefler
    let mut istek: Vec<(Ipv4Addr, Ipv4Addr)> = hedefler.iter().map(|h| (*h, sifir)).collect();
    if tek {
        istek.extend(ap::bos_adaylari(hedefler[0]).into_iter().map(|x| (x, sifir)));
    }
    let ilk = (a.arp)(&istek)?;
    let mut sonuc = vec![];
    if tek {
        let h = hedefler[0];
        let bos = ap::bos_adaylari(h).into_iter().find(|x| !ilk.contains_key(x));
        if ilk.contains_key(&h) {
            sonuc.push((h, bos));
        } else if let Some(b) = bos {
            // yoklamaya (0.0.0.0) cevap vermeyen cihaz: kendi ağından bir adresle bir kez daha
            if (a.arp)(&[(h, b)])?.contains_key(&h) {
                sonuc.push((h, Some(b)));
            }
        }
        return Ok(sonuc);
    }
    for h in hedefler.iter().filter(|h| ilk.contains_key(h)) {
        let adaylar = ap::bos_adaylari(*h);
        let dolu = (a.arp)(&adaylar.iter().map(|x| (*x, sifir)).collect::<Vec<_>>())?;
        sonuc.push((*h, adaylar.into_iter().find(|x| !dolu.contains_key(x) && !hedefler.contains(x))));
    }
    Ok(sonuc)
}

/// "Bağlan" (`hedef`) ya da "Otomatik bul" (None). Ara ve son durum `a.kaydet` ile yazılır; sonuç döner.
pub fn calistir(cfg: &Config, hedef: Option<Ipv4Addr>, kim: &str, rol: &str, a: &Araclar, now: &dyn Fn() -> f64) -> Durum {
    let mut d = Durum::basla(hedef, kim, rol, now());
    (a.kaydet)(&d);
    let sonuc = |d: Durum, durum: &str, mesaj: String| {
        let d = bitir(d, durum, mesaj, now());
        let ek = match durum {
            "bulundu" | "web_yok" => format!(" hedef={} port={} yol={}", d.adres, d.port, d.yol),
            _ => d.hedefler.first().filter(|_| d.tur == "baglan").map(|h| format!(" hedef={h}")).unwrap_or_default(),
        };
        (a.denetim)("AP_BAGLAN", format!("tur={} sonuc={durum}{ek}", d.tur));
        (a.kaydet)(&d);
        d
    };
    // misafir ağı: yan oda yok, doğrudan
    if let Some(h) = hedef.filter(|h| ap::misafir_aginda(*h, cfg)) {
        d.adres = h.to_string();
        d.yol = "misafir".into();
        return match (a.web)(h, false) {
            Some(p) => {
                (d.port, d.sema) = (p, sema(p).into());
                sonuc(d, "bulundu", String::new())
            }
            None => sonuc(d, "yok", "Bu adreste cihaz yok — kabloyu ve adresi kontrol edin.".into()),
        };
    }
    let hedefler: Vec<Ipv4Addr> = d.hedefler.iter().filter_map(|h| h.parse().ok()).collect();
    // aynı ağda açık yan oda: yeniden açılmaz
    if let (Some(h), Some(g)) = (hedef, (a.yan_oda)()) {
        if ap::yan_odada(h, &g) {
            (d.adres, d.yol, d.yan_oda) = (h.to_string(), "yan_oda".into(), g.adres.clone());
            return match (a.web)(h, true) {
                Some(p) => {
                    (d.port, d.sema) = (p, sema(p).into());
                    sonuc(d, "bulundu", String::new())
                }
                None => sonuc(d, "yok", "Bu adreste cihaz yok — kabloyu ve adresi kontrol edin.".into()),
            };
        }
    }
    let secim = match arp_sec(&hedefler, a) {
        Ok(v) => v,
        Err(e) => return sonuc(d, "hata", format!("Arama yapılamadı (yan odadan ARP: {e}).")),
    };
    for (h, bos) in secim {
        let Some(bos) = bos else {
            if hedef.is_some() {
                return sonuc(d, "hata", format!("{h} cevap veriyor ama yan oda için boş adres bulunamadı (.241–.250 dolu)."));
            }
            continue;
        };
        // başka ağda açık yan oda kapanır (aynı anda tek yan oda)
        let acik = (a.yan_oda)();
        let yan = match acik {
            Some(g) if ap::yan_odada(h, &g) => g,
            acik => {
                if acik.is_some() {
                    match (a.kapat)("yeni_hedef") {
                        Ok(Some(eski)) => (a.denetim)("AP_YAN_ODA", format!("kapandi adres={eski} neden=yeni_hedef")),
                        Ok(None) => {}
                        Err(e) => return sonuc(d, "hata", e),
                    }
                }
                let cidr = ap::ag24(bos).to_string();
                match (a.ac)(&cidr) {
                    Ok(g) => {
                        (a.denetim)("AP_YAN_ODA", format!("acildi adres={} sure={}", g.adres, ap::SURE_SN));
                        g
                    }
                    Err(e) => return sonuc(d, "hata", e),
                }
            }
        };
        (d.adres, d.yol, d.yan_oda) = (h.to_string(), "yan_oda".into(), yan.adres.clone());
        (a.kaydet)(&d);
        match (a.web)(h, true) {
            Some(p) => {
                (d.port, d.sema) = (p, sema(p).into());
                return sonuc(d, "bulundu", String::new());
            }
            None if hedef.is_some() => {
                // cihaz var (ARP) ama web arayüzü yok: yan oda açık kalır (betikle başka port denenebilir)
                return sonuc(d, "web_yok", "Cihaz cevap veriyor ama web arayüzü (80/443) açılmadı.".into());
            }
            None => {
                if let Ok(Some(eski)) = (a.kapat)("web_yok") {
                    (a.denetim)("AP_YAN_ODA", format!("kapandi adres={eski} neden=web_yok"));
                }
                (d.adres, d.yol, d.yan_oda) = Default::default();
            }
        }
    }
    let mesaj = if hedef.is_some() {
        "Bu adreste cihaz yok — kabloyu ve adresi kontrol edin.".to_string()
    } else {
        "Yaygın fabrika adreslerinde cihaz bulunamadı. Etiketteki adresi yazıp \"Bağlan\" deyin; AP DHCP'den adres aldıysa \"Ağdaki cihazlar\"a bakın.".to_string()
    };
    sonuc(d, "yok", mesaj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::net::SocketAddrV4;

    fn cfg() -> Config {
        Config::parse("[main]\nsite_name = 'X'\n").unwrap()
    }

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    /// Sahte ağ: ARP'ye cevap verenler (gönderen 0.0.0.0'a cevap vermeyenler ayrı), TCP'si açık web portları, 443'te
    /// aktarımın TLS'iyle el sıkışan cihazlar (yoksa 443 yalnızca TCP açık: eski TLS), yan oda.
    struct Ag {
        var: Vec<Ipv4Addr>,
        yoklamaya_sessiz: Vec<Ipv4Addr>,
        web: Vec<SocketAddrV4>,
        tls: Vec<Ipv4Addr>,
        yan_oda: RefCell<Option<Gecici>>,
        olaylar: RefCell<Vec<String>>,
        aramalar: RefCell<usize>,
    }

    impl Ag {
        fn yeni(var: &[&str], web: &[&str]) -> Ag {
            Ag {
                var: var.iter().map(|s| ip(s)).collect(),
                yoklamaya_sessiz: vec![],
                web: web.iter().map(|s| s.parse().unwrap()).collect(),
                tls: vec![],
                yan_oda: RefCell::new(None),
                olaylar: RefCell::default(),
                aramalar: RefCell::new(0),
            }
        }

        fn calistir(&self, hedef: Option<&str>) -> Durum {
            let arp = |istek: &[(Ipv4Addr, Ipv4Addr)]| -> Result<BTreeMap<Ipv4Addr, String>, String> {
                *self.aramalar.borrow_mut() += 1;
                Ok(istek
                    .iter()
                    .filter(|(h, g)| self.var.contains(h) && (!g.is_unspecified() || !self.yoklamaya_sessiz.contains(h)))
                    .map(|(h, _)| (*h, "70:4f:57:dd:7a:e8".to_string()))
                    .collect())
            };
            let yan_oda = || self.yan_oda.borrow().clone();
            let ac = |c: &str| -> Result<Gecici, String> {
                let g = Gecici { adres: c.into(), kimlik: "0a1b2c3d".into(), bitis: 2800.0 };
                *self.yan_oda.borrow_mut() = Some(g.clone());
                self.olaylar.borrow_mut().push(format!("ac {c}"));
                Ok(g)
            };
            let kapat = |n: &str| -> Result<Option<String>, String> {
                let g = self.yan_oda.borrow_mut().take();
                self.olaylar.borrow_mut().push(format!("kapat {n}"));
                Ok(g.map(|g| g.adres))
            };
            // gerçek seçim kuralı (ap_arayuz::web_sec), sahte bağlantılarla
            let web = |h: Ipv4Addr, yan: bool| {
                self.olaylar.borrow_mut().push(format!("web {h} {}", if yan { "yan" } else { "dogrudan" }));
                // yan oda açık değilse ya da hedef onun ağında değilse aracı bağlanamaz
                let ulasir = !yan || self.yan_oda.borrow().as_ref().is_some_and(|g| ap::yan_odada(h, g));
                let acik = |p: u16| ulasir && self.web.contains(&SocketAddrV4::new(h, p));
                crate::ap_arayuz::web_sec(|| acik(443) && self.tls.contains(&h), || acik(80))
            };
            let denetim = |o: &str, e: String| self.olaylar.borrow_mut().push(format!("{o} {e}"));
            let kaydet = |_: &Durum| {};
            let a = Araclar { arp: &arp, yan_oda: &yan_oda, ac: &ac, kapat: &kapat, web: &web, denetim: &denetim, kaydet: &kaydet };
            calistir(&cfg(), hedef.map(ip), "4511643", "sahip", &a, &|| 1000.0)
        }

        fn olay(&self) -> Vec<String> {
            self.olaylar.borrow().clone()
        }
    }

    #[test]
    fn baglan_yan_oda_ve_web() {
        // AP 192.168.1.1, .250 dolu (telefon AP'nin DHCP'sinden almış) → yan oda .249; 80 açık
        let ag = Ag::yeni(&["192.168.1.1", "192.168.1.250"], &["192.168.1.1:80"]);
        let d = ag.calistir(Some("192.168.1.1"));
        assert_eq!((d.durum.as_str(), d.adres.as_str(), d.port, d.sema.as_str(), d.yol.as_str(), d.yan_oda.as_str()),
                   ("bulundu", "192.168.1.1", 80, "http", "yan_oda", "192.168.1.249/24"));
        let o = ag.olay();
        assert_eq!(o[0], "ac 192.168.1.249/24");
        assert!(o.contains(&"AP_YAN_ODA acildi adres=192.168.1.249/24 sure=1800".to_string()));
        assert!(o.contains(&"web 192.168.1.1 yan".to_string()));
        assert_eq!(o.last().unwrap(), "AP_BAGLAN tur=baglan sonuc=bulundu hedef=192.168.1.1 port=80 yol=yan_oda");
        assert_eq!(*ag.aramalar.borrow(), 1, "tek ARP taraması (hedef + adaylar)");
        // yalnızca 443 (TLS çalışıyor): https
        let mut ag = Ag::yeni(&["192.168.0.254"], &["192.168.0.254:443"]);
        ag.tls = vec![ip("192.168.0.254")];
        let d = ag.calistir(Some("192.168.0.254"));
        assert_eq!((d.durum.as_str(), d.port, d.sema.as_str(), d.yan_oda.as_str()), ("bulundu", 443, "https", "192.168.0.250/24"));
        // cihaz var, web yok: yan oda açık kalır
        let ag = Ag::yeni(&["192.168.1.1"], &[]);
        let d = ag.calistir(Some("192.168.1.1"));
        assert_eq!(d.durum, "web_yok");
        assert!(ag.yan_oda.borrow().is_some() && !ag.olay().iter().any(|x| x.starts_with("kapat")));
    }

    #[test]
    fn baglan_cihaz_yok_ve_sessiz_cihaz() {
        // cevap yok: yan oda açılmaz, TCP denenmez
        let ag = Ag::yeni(&[], &[]);
        let d = ag.calistir(Some("192.168.1.1"));
        assert_eq!(d.durum, "yok");
        assert!(d.mesaj.contains("Bu adreste cihaz yok"));
        assert!(!ag.olay().iter().any(|x| x.starts_with("ac ") || x.starts_with("web")));
        assert_eq!(ag.olay().last().unwrap(), "AP_BAGLAN tur=baglan sonuc=yok hedef=192.168.1.1");
        assert_eq!(*ag.aramalar.borrow(), 2, "yoklama + boş adresten ikinci soru");
        // yoklamaya (0.0.0.0) cevap vermeyen, kendi ağından soruya cevap veren cihaz
        let mut ag = Ag::yeni(&["192.168.1.1"], &["192.168.1.1:80"]);
        ag.yoklamaya_sessiz = vec![ip("192.168.1.1")];
        assert_eq!(ag.calistir(Some("192.168.1.1")).durum, "bulundu");
        // adayların hepsi dolu: hata, yan oda açılmaz
        let dolu: Vec<String> = (241..=250).map(|s| format!("192.168.1.{s}")).collect();
        let mut v: Vec<&str> = dolu.iter().map(String::as_str).collect();
        v.push("192.168.1.1");
        let ag = Ag::yeni(&v, &["192.168.1.1:80"]);
        let d = ag.calistir(Some("192.168.1.1"));
        assert!(d.durum == "hata" && d.mesaj.contains("boş adres"), "{d:?}");
        assert!(ag.yan_oda.borrow().is_none());
    }

    #[test]
    fn misafir_ve_acik_yan_oda() {
        // misafir ağı: ARP / yan oda yok, doğrudan
        let ag = Ag::yeni(&[], &["10.50.0.2:80"]);
        let d = ag.calistir(Some("10.50.0.2"));
        assert_eq!((d.durum.as_str(), d.yol.as_str(), d.port), ("bulundu", "misafir", 80));
        assert_eq!(ag.olay()[0], "web 10.50.0.2 dogrudan");
        assert_eq!(*ag.aramalar.borrow(), 0);
        // aynı ağda açık yan oda: yeniden açılmaz
        let ag = Ag::yeni(&["192.168.1.1"], &["192.168.1.1:80"]);
        *ag.yan_oda.borrow_mut() = Some(Gecici { adres: "192.168.1.250/24".into(), kimlik: "11111111".into(), bitis: 2000.0 });
        let d = ag.calistir(Some("192.168.1.1"));
        assert_eq!((d.durum.as_str(), d.yan_oda.as_str()), ("bulundu", "192.168.1.250/24"));
        assert!(!ag.olay().iter().any(|x| x.starts_with("ac ") || x.starts_with("kapat")) && *ag.aramalar.borrow() == 0);
        // başka ağda açık yan oda: kapanır (denetimde), yenisi açılır
        let ag = Ag::yeni(&["192.168.0.254"], &["192.168.0.254:80"]);
        *ag.yan_oda.borrow_mut() = Some(Gecici { adres: "192.168.1.250/24".into(), kimlik: "11111111".into(), bitis: 2000.0 });
        let d = ag.calistir(Some("192.168.0.254"));
        assert_eq!((d.durum.as_str(), d.yan_oda.as_str()), ("bulundu", "192.168.0.250/24"));
        let o = ag.olay();
        let k = o.iter().position(|x| x == "kapat yeni_hedef").unwrap();
        assert!(o[k + 1] == "AP_YAN_ODA kapandi adres=192.168.1.250/24 neden=yeni_hedef" && o[k + 2] == "ac 192.168.0.250/24");
    }

    #[test]
    fn otomatik_bul() {
        // listede 192.168.1.1 (web yok) ve 192.168.0.254 (web var): ilkinde yan oda kapanır, ikincide açık kalır
        let ag = Ag::yeni(&["192.168.1.1", "192.168.0.254", "192.168.0.250"], &["192.168.0.254:80"]);
        let d = ag.calistir(None);
        assert_eq!((d.tur.as_str(), d.durum.as_str(), d.adres.as_str(), d.yan_oda.as_str()), ("otomatik", "bulundu", "192.168.0.254", "192.168.0.249/24"));
        let o = ag.olay();
        assert!(o.contains(&"ac 192.168.1.250/24".to_string()) && o.contains(&"kapat web_yok".to_string()), "{o:?}");
        assert!(o.contains(&"AP_YAN_ODA kapandi adres=192.168.1.250/24 neden=web_yok".to_string()));
        assert_eq!(o.last().unwrap(), "AP_BAGLAN tur=otomatik sonuc=bulundu hedef=192.168.0.254 port=80 yol=yan_oda");
        assert!(d.hedefler.len() == ap::OTOMATIK.len() && d.hedefler[0] == "192.168.1.1");
        // hiçbiri yok
        let ag = Ag::yeni(&[], &[]);
        let d = ag.calistir(None);
        assert!(d.durum == "yok" && d.mesaj.contains("Yaygın fabrika adreslerinde"));
        assert_eq!(*ag.aramalar.borrow(), 1);
        assert_eq!(ag.olay().last().unwrap(), "AP_BAGLAN tur=otomatik sonuc=yok");
    }

    #[test]
    fn web_arayuzu_https_ancak_tls_calisirsa() {
        // Canlı ZyXEL (2026-10-18): 443'te aktarımın TLS'iyle el sıkışan cihaz HTTPS'ten açılır (80 açık olsa da); TLS'i
        // eski (TCP'si açık ama el sıkışma yok) cihaz 80'den; yalnızca TCP'si açık 443 web arayüzü sayılmaz
        let bul = |hedef: &str, web: &[&str], tls: &[&str]| {
            let mut ag = Ag::yeni(&[hedef], web);
            ag.tls = tls.iter().map(|s| ip(s)).collect();
            let d = ag.calistir(Some(hedef));
            (d.durum, d.port, d.sema, d.yol)
        };
        let s = |durum: &str, port: u16, sema: &str, yol: &str| (durum.to_string(), port, sema.to_string(), yol.to_string());
        for (yol, h) in [("yan_oda", "192.168.1.1"), ("misafir", "10.50.0.2")] {
            let (p80, p443) = (format!("{h}:80"), format!("{h}:443"));
            let (p80, p443) = (p80.as_str(), p443.as_str());
            assert_eq!(bul(h, &[p80, p443], &[h]), s("bulundu", 443, "https", yol), "{yol}: TLS el sıkışması başarılı");
            assert_eq!(bul(h, &[p80, p443], &[]), s("bulundu", 80, "http", yol), "{yol}: 443 açık ama TLS eski");
            assert_eq!(bul(h, &[p80], &[]), s("bulundu", 80, "http", yol), "{yol}: yalnızca 80");
            assert_eq!(bul(h, &[p443], &[h]), s("bulundu", 443, "https", yol), "{yol}: yalnızca 443, TLS çalışıyor");
        }
        assert_eq!(bul("192.168.1.1", &["192.168.1.1:443"], &[]).0, "web_yok", "aktarımın konuşamadığı 443");
        assert_eq!(bul("10.50.0.2", &["10.50.0.2:443"], &[]).0, "yok");
        // otomatik bul: denetimde port, sonuçta şema
        let mut ag = Ag::yeni(&["192.168.1.1"], &["192.168.1.1:80", "192.168.1.1:443"]);
        ag.tls = vec![ip("192.168.1.1")];
        let d = ag.calistir(None);
        assert_eq!((d.durum.as_str(), d.port, d.sema.as_str()), ("bulundu", 443, "https"));
        assert_eq!(ag.olay().last().unwrap(), "AP_BAGLAN tur=otomatik sonuc=bulundu hedef=192.168.1.1 port=443 yol=yan_oda");
    }

    #[test]
    fn durum_ve_komut() {
        let d = Durum::basla(Some(ip("192.168.1.1")), "4511643", "hizmet", 5.0);
        assert_eq!((d.tur.as_str(), d.durum.as_str(), d.hedefler.clone(), d.rol.as_str()), ("baglan", "suruyor", vec!["192.168.1.1".to_string()], "hizmet"));
        let root = std::env::temp_dir().join(format!("wfc-apbul-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let y = Yollar::kok(&root);
        assert_eq!(oku(&y), None);
        kaydet(&y, &d).unwrap();
        assert_eq!(oku(&y), Some(d));
        assert_eq!(komut("4511643", "sahip", "192.168.1.50", Some(ip("192.168.1.1"))),
                   ["systemd-run", "--collect", "--unit", "wfc-ap-bul", "--property=RuntimeMaxSec=180", "/usr/local/bin/wificorrect", "ctl", "ap-bul", "4511643", "sahip", "192.168.1.50", "192.168.1.1"]);
        assert_eq!(komut("4511643", "sahip", "192.168.1.50", None).len(), 11);
        let calisan = |a: &[&str]| if a == ["systemctl", "is-active", "wfc-ap-bul.service"] { "active\n".to_string() } else { String::new() };
        assert!(calisiyor(&calisan) && !calisiyor(&|_: &[&str]| "inactive\n".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
