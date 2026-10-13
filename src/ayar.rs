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
    pub allow: Vec<Device>,
    pub ban: Vec<Device>,
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| format!("ayar dosyası okunamadı: {e}"))
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
