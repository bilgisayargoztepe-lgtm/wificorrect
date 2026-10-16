//! Ayar dosyası (/etc/wificorrect/ayarlar.toml). Eski UCI `hotspot` yapılandırmasının karşılığı;
//! bölüm ve anahtar adları docs/MASTER_ENGINEERING.md §8 ile aynı. Eksik anahtar varsayılanı alır.

use serde::{Deserialize, Serialize};

pub const PATH: &str = "/etc/wificorrect/ayarlar.toml";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Main {
    /// İşletme adı (giriş sayfası, sözleşmedeki İŞLETMECİ)
    pub site_name: String,
    /// Vergi levhasındaki unvan (açık rıza metnindeki [vergi levhası unvanı]); işletme sahibi Ayarlar'da girer
    pub unvan: String,
    /// Veri sorumlusunun başvuru adresi (KVKK aydınlatma metnindeki [başvuru adresi]); metnin tam olması için gerekli
    pub adres: String,
    /// İsteğe bağlı iletişim bilgileri ([irtibat numarası], [e-posta adresi], [KEP adresi]); boşsa metindeki satırı çıkar
    pub telefon: String,
    pub eposta: String,
    pub kep: String,
    pub iface: String,
    pub router_ip: String,
    pub subnet: String,
    pub portal_port: u16,
    pub session_minutes: u64,
    pub max_devices_per_phone: usize,
    pub log_root: String,
    pub state_root: String,
    pub leases_file: String,
    /// Kayıtlar cihazda kaç gün kalır (730 = 2 yıl); yalnızca yedeklenmiş günler silinir.
    pub retention_days: u64,
    /// İşletme türü: kafe | otel | yurt | diger. Anahtar yoksa "seçilmemiş" (Özet uyarısı); davranış kafe gibi.
    /// Yalnızca sahip türü seçip kaydedince yazılır (None yazılmaz; başka ayarı kaydetmek anahtarı eklemez).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isletme_turu: Option<String>,
}

impl Default for Main {
    fn default() -> Self {
        Main {
            site_name: "İşletme".into(),
            unvan: String::new(),
            adres: String::new(),
            telefon: String::new(),
            eposta: String::new(),
            kep: String::new(),
            iface: "br-hotspot".into(),
            router_ip: "10.50.0.1".into(),
            subnet: "10.50.0.0/24".into(),
            portal_port: 8080,
            session_minutes: 43200,
            max_devices_per_phone: 3,
            log_root: "/srv/5651".into(),
            state_root: "/srv/hotspot/state".into(),
            leases_file: "/var/lib/misc/dnsmasq.leases".into(),
            retention_days: 730,
            isletme_turu: None,
        }
    }
}

/// SMS genel ayarı. `mock`: merkezdeki deneme modu (bağlı cihazda eşitlemede merkezden gelir; Özet uyarısı ve sağlık raporu).
/// Doğrulama SMS'i yönetim merkezinden gider (spec 2026-10-13): NetGSM / Twilio bilgileri ve sağlayıcı seçimi cihazda
/// tutulmaz. Eski dosyalardaki `[netgsm]`, `[twilio]`, `sms.provider`, `sms.merkez` okunurken yok sayılır, ilk kayıtta silinir.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Sms {
    pub mock: bool,
}

impl Default for Sms {
    fn default() -> Self {
        Sms { mock: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Limits {
    pub otp_ttl_sec: u64,
    pub otp_max_attempts: u32,
    pub resend_cooldown_sec: u64,
    pub sms_per_phone_15min: usize,
    pub sms_per_phone_day: usize,
    pub sms_per_mac_hour: usize,
    pub sms_global_day: u64,
    pub verify_fail_lock_min: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            otp_ttl_sec: 180,
            otp_max_attempts: 5,
            resend_cooldown_sec: 60,
            sms_per_phone_15min: 3,
            sms_per_phone_day: 10,
            sms_per_mac_hour: 5,
            sms_global_day: 300,
            verify_fail_lock_min: 15,
        }
    }
}

/// Uzak yedek (rsync, SSH). Hedef: merkez sunucuda cihaza özel, yalnızca-yazma hesap, tünelden: `wfc-<ad>@10.99.0.1:`
/// (sunucuda `wificorrect-sunucu cihaz-ekle`). Sunucunun kimliğini WireGuard doğrular (10.99.0.1 yalnızca tünelden
/// erişilir); bu yüzden SSH ana makine anahtarı tutulmaz — sunucu taşınınca yedek kırılmasın.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Backup {
    pub enabled: bool,
    pub target: String,
    pub ssh: String,
}

impl Default for Backup {
    fn default() -> Self {
        Backup { enabled: false, target: String::new(), ssh: "ssh -i /etc/wificorrect/yedek_anahtar -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null".into() }
    }
}

/// Giriş sayfasındaki metinler (panel → Portal metinleri). Boş başlar; kafe sahibi ya da admin yazar.
/// Düz metin: boş satır paragraf ayırır; "İŞLETMECİ" giriş sayfasında kafe adıyla değişir (ekleri uyumlu).
/// Açık rıza metni onay kutusunun yanında yazar ve işaretlemek her zaman zorunludur (2026-10-03, kullanıcı kararı);
/// içindeki [vergi levhası unvanı] kurulumda girilen unvanla değişir. Üç metin de ürünle gelir (kullanıcı metinleri).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct PortalMetin {
    pub aydinlatma: String,
    pub acik_riza: String,
    pub sozlesme: String,
    /// İngilizce giriş sayfası için (boşsa Türkçesi gösterilir); İŞLETMECİ yalnızca adla değişir (ek yok).
    pub aydinlatma_en: String,
    pub acik_riza_en: String,
    pub sozlesme_en: String,
}

/// 0.4.11 ve öncesinin varsayılan aydınlatma metninin (veri sorumlusu sabit "Göztepe Bilgisayar") sha256'sı (kırpılmış).
/// Panel her kayıtta bütün ayarları yazdığı için eski varsayılan çoğu cihazın ayar dosyasında durur.
const ESKI_AYDINLATMA_SHA: &str = "4b2efded1040a72a2f9e39b28e1406bf70ef76356876736abd6f09e2f1dec7c1";

impl PortalMetin {
    /// Kayıtlı aydınlatma metni eski ürün varsayılanının aynısıysa yeni (işletmeden bağımsız) varsayılanla değişir;
    /// işletmenin düzenlediği metne dokunulmaz.
    fn eski_varsayilani_guncelle(&mut self) {
        use sha2::{Digest, Sha256};
        let ozet: String = Sha256::digest(self.aydinlatma.trim().as_bytes()).iter().map(|x| format!("{x:02x}")).collect();
        if ozet == ESKI_AYDINLATMA_SHA {
            self.aydinlatma = PortalMetin::default().aydinlatma;
        }
    }
}

impl Default for PortalMetin {
    fn default() -> Self {
        PortalMetin {
            aydinlatma: include_str!("sablon/aydinlatma.txt").trim().to_string(),
            acik_riza: include_str!("sablon/acik_riza.txt").trim().to_string(),
            sozlesme: include_str!("sablon/sozlesme.txt").trim().to_string(),
            aydinlatma_en: String::new(),
            acik_riza_en: String::new(),
            sozlesme_en: String::new(),
        }
    }
}

/// Uzak erişim (src/uzak.rs): WireGuard tüneli kendi sunucumuza; yalnızca admin görür ve değiştirir.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Uzak {
    pub enabled: bool,
    /// `vpn.ornek.com:51820`
    pub sunucu: String,
    pub sunucu_anahtar: String,
    /// cihazın tünel adresi, ör. 10.99.0.17
    pub adres: String,
}

/// Yasaklı siteler / kelimeler (src/filtre.rs). Boş başlar; kafe sahibi panelden yazar.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Filtre {
    pub siteler: Vec<String>,
    pub kelimeler: Vec<String>,
    pub istisnalar: Vec<String>,
    /// Hazır engel listeleri (src/liste.rs): kategori → açık mı. Anahtar yoksa açık (varsayılan: hepsi açık).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub hazir: std::collections::BTreeMap<String, bool>,
}

/// İşletme türleri (değer, etiket); ilk öğe varsayılan davranış.
pub const ISLETME_TURLERI: &[(&str, &str)] = &[("kafe", "Kafe/Restoran"), ("otel", "Otel/Pansiyon"), ("yurt", "Yurt"), ("diger", "Diğer")];

/// Sahibin seçtiği tür; anahtar yoksa ya da tanınmıyorsa (elle bozulmuş) None = "seçilmemiş".
pub fn isletme_turu_secili(cfg: &Config) -> Option<&str> {
    cfg.main.isletme_turu.as_deref().filter(|t| ISLETME_TURLERI.iter().any(|(k, _)| k == t))
}

/// Geçerli işletme türü: seçilmemişse `kafe`.
pub fn isletme_turu(cfg: &Config) -> &str {
    isletme_turu_secili(cfg).unwrap_or("kafe")
}

/// Giriş yöntemleri ayarı (`[giris]`). SMS her zaman açıktır, anahtarı yok.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Giris {
    /// Misafir kodu ile giriş (yalnızca otel/yurt türünde açılabilir; tür başka bir şeye çekilince kapanır)
    pub misafir_kodu: bool,
}

/// Misafirin internete çıkma yolları. Yeni yöntem (ör. kupon) yeni öğe + görünürlük/açıklık kuralıyla eklenir.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GirisYontemi {
    Sms,
    MisafirKodu,
}

impl GirisYontemi {
    pub const HEPSI: &'static [GirisYontemi] = &[GirisYontemi::Sms, GirisYontemi::MisafirKodu];

    /// Ayar anahtarı (`giris.<kimlik>`)
    pub fn kimlik(&self) -> &'static str {
        match self {
            GirisYontemi::Sms => "sms",
            GirisYontemi::MisafirKodu => "misafir_kodu",
        }
    }

    pub fn ad(&self) -> &'static str {
        match self {
            GirisYontemi::Sms => "SMS",
            GirisYontemi::MisafirKodu => "Misafir kodu",
        }
    }

    /// Panelden açılıp kapatılabilir mi (SMS her zaman açık)
    pub fn degistirilebilir(&self) -> bool {
        !matches!(self, GirisYontemi::Sms)
    }

    /// Bu işletme türünde ayarlarda görünür ve açılabilir mi
    pub fn gorunur(&self, tur: &str) -> bool {
        match self {
            GirisYontemi::Sms => true,
            GirisYontemi::MisafirKodu => matches!(tur, "otel" | "yurt"),
        }
    }

    /// Şu an kullanılabilir mi: türde görünür ve ayarda açık
    pub fn acik(&self, cfg: &Config) -> bool {
        self.gorunur(isletme_turu(cfg))
            && match self {
                GirisYontemi::Sms => true,
                GirisYontemi::MisafirKodu => cfg.giris.misafir_kodu,
            }
    }
}

/// Misafir kodu ile giriş kullanılabilir mi (açık ve tür otel/yurt).
pub fn misafir_kodu_acik(cfg: &Config) -> bool {
    GirisYontemi::MisafirKodu.acik(cfg)
}

/// AP kurulumu (`[ap]`, src/ap.rs): AP'lerin kalıcı yönetim adresleri (misafir alt ağında, DHCP havuzu dışında; en çok
/// 8). Yalnızca kayıt: panelde gösterilir, ağdaki cihazlar listesinde işaretlenir; panelden AP arayüzüne gidilebilir
/// (src/ap_arayuz.rs). Güvenlik duvarı kuralı yoktur: köprü portları arası iletim `kapi.nft`'te zaten kapalı.
/// `etiket` / `marka`: takılan cihazın etiketindeki adres (IP ya da bilinen ad, "Bağlan") ve marka/model (yalnızca
/// bilgi; spec 2026-10-16-ap-arayuz). `mac`: kayıtlı adresin kaydedildiği andaki MAC'i (adres → MAC; final review I1):
/// panelden aktarım yalnızca komşu tablosundaki MAC bununla aynıysa — misafir ağında AP'nin adresini elle alan bir cihaz
/// AP yerine geçip yazılan AP parolasını toplamasın. Fabrika dönüşünde hepsi boşalır.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Ap {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub adresler: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub etiket: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub marka: String,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mac: std::collections::BTreeMap<String, String>,
}

/// Portalsız geçen (`[[allow]]`, ör. AP, personel; trafiği yine kaydedilir) ya da yasaklı (`[[ban]]`) cihaz.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Device {
    pub mac: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub main: Main,
    pub sms: Sms,
    pub limits: Limits,
    pub backup: Backup,
    pub filtre: Filtre,
    pub portal: PortalMetin,
    pub uzak: Uzak,
    pub giris: Giris,
    pub ap: Ap,
    pub allow: Vec<Device>,
    pub ban: Vec<Device>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c: Config = toml::from_str(text).map_err(|e| format!("ayar dosyası okunamadı: {e}"))?;
        c.portal.eski_varsayilani_guncelle();
        Ok(c)
    }

    /// Panelden kayıt: atomik, izin 600. ponytail: dosyadaki yorumlar korunmaz (örnek dosyada duruyor).
    pub fn save(&self, path: &str) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| format!("ayar yazılamadı: {e}"))?;
        crate::ortak::write_atomic(std::path::Path::new(path), text.as_bytes()).map_err(|e| format!("{path} yazılamadı: {e}"))
    }

    pub fn load(path: &str) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path} açılamadı: {e}"))?;
        Self::parse(&text)
    }

    /// 0.4.8 öncesinden kalan SMS sırları ve ayarları (`[netgsm]`, `[twilio]`, `[sms]` altında `provider` / `merkez`;
    /// SMS artık merkezden gider): dosyada varsa ayarlar bir kez yeniden yazılır (atomik, 600; diğer ayarlar aynen).
    /// Paket kurulumunda (postinst) ve açılışta (`ctl yukle`) çağrılır. Dönen: dosya yeniden yazıldı mı.
    pub fn eski_sirlari_temizle(path: &str) -> Result<bool, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path} açılamadı: {e}"))?;
        let ham: toml::Table = text.parse().map_err(|e| format!("ayar dosyası okunamadı: {e}"))?;
        let sms_eski = ham.get("sms").and_then(toml::Value::as_table).is_some_and(|s| s.contains_key("provider") || s.contains_key("merkez"));
        if !(ham.contains_key("netgsm") || ham.contains_key("twilio") || sms_eski) {
            return Ok(false);
        }
        Self::parse(&text)?.save(path)?;
        Ok(true)
    }

    /// Geçerli MAC'li girdiler: {mac: ad}.
    pub fn devices(list: &[Device]) -> std::collections::BTreeMap<String, String> {
        list.iter().filter_map(|d| crate::ortak::norm_mac(&d.mac).map(|m| (m, d.name.clone()))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_file() {
        let c = Config::parse("[main]\nsite_name = 'Bocafe'\n[sms]\nmock = false\n[netgsm]\nusercode = '850'\n").unwrap();
        assert_eq!(c.main.site_name, "Bocafe");
        assert_eq!(c.main.session_minutes, 43200);
        assert_eq!(c.limits.otp_ttl_sec, 180);
        assert!(!c.sms.mock);
        assert!(Config::parse("").unwrap().sms.mock); // varsayılan: deneme modu
        // eski dosya (cihazda NetGSM/Twilio sırları vardı): okunur, ilk kayıtta sırlar dosyadan silinir
        let eski = "[sms]\nmock = false\nprovider = 'twilio'\nmerkez = true\n[netgsm]\nusercode = '850'\npassword = 'gizli-1'\n\
                    [twilio]\nenabled = true\naccount_sid = 'AC1'\nauth_token = 'gizli-2'\n";
        let c = Config::parse(eski).unwrap();
        assert!(!c.sms.mock);
        let yeni = toml::to_string_pretty(&c).unwrap();
        for yok in ["gizli-1", "gizli-2", "[netgsm]", "[twilio]", "provider =", "merkez ="] {
            assert!(!yeni.contains(yok), "{yok}");
        }
        let c = Config::parse("[[allow]]\nmac = 'AA-BB-CC-DD-EE-99'\nname = 'AP'\n[[allow]]\nmac = 'bozuk'\n").unwrap();
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap(); // yaz-oku aynı
        assert_eq!(back.allow.len(), 2);
        assert_eq!(Config::devices(&c.allow).into_iter().collect::<Vec<_>>(), vec![("aa:bb:cc:dd:ee:99".into(), "AP".into())]);
        assert!(Config::parse("[main\n").is_err());
    }

    #[test]
    fn eski_sirlar_dosyadan_silinir() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("wfc-ayar-sir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("ayarlar.toml");
        let ps = p.to_str().unwrap();
        let eski = "# yorum\n[main]\nsite_name = 'Bocafe'\nsession_minutes = 1440\n[sms]\nmock = false\nprovider = 'twilio'\nmerkez = true\n\
                    [netgsm]\nusercode = '850'\npassword = 'gizli-1'\n[twilio]\nenabled = true\nauth_token = 'gizli-2'\n\
                    [[allow]]\nmac = 'aa:bb:cc:dd:ee:99'\nname = 'AP'\n";
        std::fs::write(&p, eski).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(Config::eski_sirlari_temizle(ps), Ok(true));
        let yeni = std::fs::read_to_string(&p).unwrap();
        for yok in ["gizli-1", "gizli-2", "[netgsm]", "[twilio]", "provider =", "merkez ="] {
            assert!(!yeni.contains(yok), "{yok}");
        }
        let c = Config::load(ps).unwrap();
        assert_eq!((c.main.site_name.as_str(), c.main.session_minutes, c.sms.mock, c.allow.len()), ("Bocafe", 1440, false, 1));
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        // temiz dosyaya dokunulmaz (yorumlar kalır)
        let temiz = "# yorum\n[main]\nsite_name = 'X'\n[sms]\nmock = true\n";
        std::fs::write(&p, temiz).unwrap();
        assert_eq!(Config::eski_sirlari_temizle(ps), Ok(false));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), temiz);
        // yalnızca [sms] altındaki eski anahtar da yeter; bozuk dosya hata, dokunulmaz
        std::fs::write(&p, "[sms]\nmock = false\nmerkez = true\n").unwrap();
        assert_eq!(Config::eski_sirlari_temizle(ps), Ok(true));
        assert!(!std::fs::read_to_string(&p).unwrap().contains("merkez ="));
        std::fs::write(&p, "[main\n[netgsm]\n").unwrap();
        assert!(Config::eski_sirlari_temizle(ps).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[main\n[netgsm]\n");
        assert!(Config::eski_sirlari_temizle(d.join("yok.toml").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn aydinlatma_isletmeden_bagimsiz() {
        // ürünle gelen metinlerde belirli bir işletme yok; veri sorumlusu unvan yer tutucusu
        let d = PortalMetin::default();
        for m in [&d.aydinlatma, &d.acik_riza, &d.sozlesme] {
            assert!(!m.contains("Göztepe") && !m.contains("goztepe"), "{m}");
        }
        assert!(d.aydinlatma.contains("[vergi levhası unvanı] (“İşletme”), veri sorumlusu"));
        // iletişim bilgileri işletmeden gelir (yer tutucular); eski Göztepe bilgileri yok
        assert!(d.aydinlatma.ends_with(
            "[vergi levhası unvanı]\n\n* Müracaat Adresi: [başvuru adresi]\n* İrtibat Numarası: [irtibat numarası]\n* KEP Adresi: [KEP adresi]\n* E-Posta: [e-posta adresi]"
        ));
        assert!(d.aydinlatma.contains("* Şahsen merkezimize ([başvuru adresi]) gelerek ıslak imzalı dilekçeyle"));
        assert!(d.aydinlatma.contains("adresiniz kanalıyla [KEP adresi] KEP adresimize\n"));
        assert!(!d.aydinlatma.contains("Sahrayıcedit") && !d.aydinlatma.contains("0216") && !d.aydinlatma.contains("kep.tr"));
        // iletişim alanları boş başlar (kodda işletmeye özel varsayılan yok)
        let m = Main::default();
        assert!(m.adres.is_empty() && m.telefon.is_empty() && m.eposta.is_empty() && m.kep.is_empty());
        // eski varsayılan kayıtlıysa yenisiyle değişir (kaydedilen dosyada da)
        let eski = include_str!("../tests/fixtures/aydinlatma-0.4.11.txt").trim();
        let mut c = Config::default();
        c.portal.aydinlatma = eski.into();
        let yazi = toml::to_string_pretty(&c).unwrap();
        assert!(yazi.contains("info@goztepebilgisayar.com"));
        let c = Config::parse(&yazi).unwrap();
        assert_eq!(c.portal.aydinlatma, d.aydinlatma);
        // anahtar yoksa varsayılan; işletmenin düzenlediği metin (eskinin değiştirilmişi dahil) aynen kalır
        assert_eq!(Config::parse("[main]\nsite_name = 'X'\n").unwrap().portal.aydinlatma, d.aydinlatma);
        let mut c = Config::default();
        c.portal.aydinlatma = format!("{eski}\n* Ek satır");
        let c = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert!(c.portal.aydinlatma.contains("info@goztepebilgisayar.com") && c.portal.aydinlatma.ends_with("* Ek satır"));
    }

    #[test]
    fn ap_adresleri() {
        let c = Config::parse("[main]\nsite_name = 'X'\n").unwrap();
        assert!(c.ap.adresler.is_empty());
        let yazi = toml::to_string_pretty(&c).unwrap();
        assert!(!yazi.contains("adresler"), "{yazi}");
        let c = Config::parse("[ap]\nadresler = ['10.50.0.2', '10.50.0.3']\n[[allow]]\nmac = 'aa:bb:cc:dd:ee:99'\n").unwrap();
        assert_eq!(c.ap.adresler, ["10.50.0.2", "10.50.0.3"]);
        assert!(c.ap.etiket.is_empty() && c.ap.marka.is_empty());
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!((back.ap.adresler.len(), back.allow.len()), (2, 1));
        assert!(!toml::to_string_pretty(&c).unwrap().contains("etiket"), "boş etiket yazılmaz");
        // etiketteki adres ve marka/model (bilgi için) saklanır
        let c = Config::parse("[ap]\netiket = 'tplinkap.net'\nmarka = 'TP-Link TL-WA901ND'\n").unwrap();
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!((back.ap.etiket.as_str(), back.ap.marka.as_str(), back.ap.adresler.len()), ("tplinkap.net", "TP-Link TL-WA901ND", 0));
        assert!(back.ap.mac.is_empty() && !toml::to_string_pretty(&back).unwrap().contains("ap.mac"), "boş MAC tablosu yazılmaz");
        // kayıtlı adreslerin sabitlenen MAC'leri (adres anahtarında nokta: tırnaklı anahtar) gidip gelir
        let mut c = Config::parse("[ap]\nadresler = ['10.50.0.2', '10.50.0.3']\n").unwrap();
        c.ap.mac.insert("10.50.0.2".into(), "70:4f:57:dd:7a:e8".into());
        let yazi = toml::to_string_pretty(&c).unwrap();
        let back = Config::parse(&yazi).unwrap();
        assert_eq!(back.ap.mac.get("10.50.0.2").map(String::as_str), Some("70:4f:57:dd:7a:e8"), "{yazi}");
        assert_eq!((back.ap.mac.len(), back.ap.adresler.len()), (1, 2));
    }

    #[test]
    fn isletme_turu_ve_giris_yontemleri() {
        // eski ayar dosyası (anahtarsız) yüklenir: seçilmemiş, davranış kafe, misafir kodu kapalı
        let c = Config::parse("[main]\nsite_name = 'Bocafe'\n").unwrap();
        assert_eq!((c.main.isletme_turu.as_deref(), isletme_turu(&c), misafir_kodu_acik(&c)), (None, "kafe", false));
        // seçilmemiş tür yazılmaz (başka ayarın kaydı anahtarı eklemez)
        assert!(!toml::to_string_pretty(&c).unwrap().contains("isletme_turu"));
        let mut c = Config::parse("[main]\nisletme_turu = 'otel'\n[giris]\nmisafir_kodu = true\n").unwrap();
        assert_eq!((isletme_turu(&c), misafir_kodu_acik(&c)), ("otel", true));
        let back = Config::parse(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!((back.main.isletme_turu.as_deref(), back.giris.misafir_kodu), (Some("otel"), true));
        c.main.isletme_turu = Some("kafe".into()); // ayar açık kalsa da türde görünmez → kullanılamaz
        assert!(!misafir_kodu_acik(&c));
        c.main.isletme_turu = Some("bilinmeyen".into()); // elle bozulmuş: seçilmemiş sayılır, davranış kafe
        assert_eq!((isletme_turu(&c), isletme_turu_secili(&c)), ("kafe", None));
        c.main.isletme_turu = Some("yurt".into());
        assert_eq!(isletme_turu_secili(&c), Some("yurt"));
        for (tur, gorunur) in [("kafe", false), ("otel", true), ("yurt", true), ("diger", false)] {
            assert_eq!(GirisYontemi::MisafirKodu.gorunur(tur), gorunur, "{tur}");
            assert!(GirisYontemi::Sms.gorunur(tur));
        }
        assert!(GirisYontemi::Sms.acik(&Config::default()) && !GirisYontemi::Sms.degistirilebilir());
        assert_eq!(GirisYontemi::HEPSI.iter().map(|y| y.kimlik()).collect::<Vec<_>>(), ["sms", "misafir_kodu"]);
    }
}
