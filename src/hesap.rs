//! Panel hesapları, giriş kilidi ve oturumlar (eski panel_auth.py; RUST_YENIDEN_YAZIM.md A10, A14).
//! Hesap dosyası `/etc/wificorrect/hesaplar.json` (600). Parola özeti PBKDF2-HMAC-SHA256, kullanıcı başına tuz.
//! `admin` girişli bir kullanıcı değildir: özeti merkezden gelir (ya da `wificorrect ctl admin-parola`) ve panelde yalnızca
//! admin kilidini açar (Admin ayarları + Panel hareketleri, 30 dk).
//! İşletme sahibi hesabı ilk açılıştaki kurulum ekranında oluşturulur; varsayılan sahip hesabı yok.

use crate::ortak;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;

pub const PATH: &str = "/etc/wificorrect/hesaplar.json";
pub const ADMIN: &str = "admin";
const ITER: u32 = 120_000;
const DUMMY_SALT: &str = "00000000000000000000000000000000";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Rol {
    /// Hizmet sağlayıcı (admin): kafe sahibinin her şeyi + API ayarları (SMS sağlayıcısı, deneme modu)
    Hizmet,
    /// İşletme sahibi: API ayarları hariç her şey (kayıtlar dahil)
    Sahip,
}

impl Rol {
    pub fn ad(self) -> &'static str {
        match self {
            Rol::Hizmet => "Hizmet sağlayıcı",
            Rol::Sahip => "İşletme sahibi",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Hesap {
    pub rol: Rol,
    pub tuz: String,
    pub ozet: String,
    pub yineleme: u32,
}

pub(crate) fn digest(pw: &str, salt: &str, iter: u32) -> String {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(pw.as_bytes(), salt.as_bytes(), iter, &mut out);
    out.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn password_problem(pw: &str) -> Option<&'static str> {
    (pw.chars().count() < 10).then_some("Parola en az 10 karakter olmalı.")
}

pub struct Hesaplar {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Hesaplar {
    pub fn new(path: impl Into<PathBuf>) -> Hesaplar {
        Hesaplar { path: path.into(), lock: Mutex::new(()) }
    }

    /// Dosya yoksa boş. Bozuksa hata: boş sayılsaydı admin parolası yeniden belirlenebilir, cihaz ele geçirilebilirdi.
    pub fn load(&self) -> Result<BTreeMap<String, Hesap>, String> {
        match std::fs::read(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(format!("hesap dosyası okunamadı: {e}")),
            Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("hesap dosyası bozuk: {e}")),
        }
    }

    fn save(&self, m: &BTreeMap<String, Hesap>) -> Result<(), String> {
        let data = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
        ortak::write_atomic(&self.path, &data).map_err(|e| format!("hesap dosyası yazılamadı: {e}"))
    }

    fn new_hesap(rol: Rol, pw: &str) -> Hesap {
        let tuz = ortak::random_hex(16);
        Hesap { rol, ozet: digest(pw, &tuz, ITER), tuz, yineleme: ITER }
    }

    /// Doğruysa rol. Bilinmeyen kullanıcıda da özet hesaplanır (süre farkından kullanıcı adı anlaşılmasın).
    pub fn verify(&self, user: &str, pw: &str) -> Option<Rol> {
        let m = self.load().ok()?;
        match m.get(user) {
            Some(h) => ct_eq(&digest(pw, &h.tuz, h.yineleme), &h.ozet).then_some(h.rol),
            None => {
                let _ = digest(pw, DUMMY_SALT, ITER);
                None
            }
        }
    }

    /// Merkezden gelen admin özeti. Dönen: değişti mi (aynıysa dosyaya dokunulmaz, oturumlar düşmez).
    pub fn set_admin_ozet(&self, tuz: &str, ozet: &str, yineleme: u32) -> Result<bool, String> {
        if tuz.is_empty() || ozet.len() != 64 || !ozet.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) || yineleme < 10_000 {
            return Err("merkezden gelen admin özeti geçersiz".into());
        }
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.load()?;
        if m.get(ADMIN).is_some_and(|h| h.tuz == tuz && h.ozet == ozet && h.yineleme == yineleme) {
            return Ok(false);
        }
        m.insert(ADMIN.into(), Hesap { rol: Rol::Hizmet, tuz: tuz.into(), ozet: ozet.into(), yineleme });
        self.save(&m)?;
        Ok(true)
    }

    pub fn ozet(&self, user: &str) -> Option<String> {
        self.load().ok()?.get(user).map(|h| h.ozet.clone())
    }

    /// `admin` parolasını koyar ya da değiştirir (cihaz konsolundan, hizmet sağlayıcı).
    pub fn set_admin(&self, pw: &str) -> Result<(), String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.load()?;
        m.insert(ADMIN.into(), Self::new_hesap(Rol::Hizmet, pw));
        self.save(&m)
    }

    /// Fabrika ayarı: admin dışındaki bütün hesaplar silinir.
    pub fn keep_only_admin(&self) -> Result<(), String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.load()?;
        m.retain(|u, _| u == ADMIN);
        self.save(&m)
    }

}

/// Giriş kilidi: 15 dk içinde 5 hata → IP de kullanıcı adı da kilitlenir.
#[derive(Default)]
pub struct LoginGuard(Mutex<HashMap<String, VecDeque<f64>>>);

impl LoginGuard {
    const LIMIT: usize = 5;
    const WINDOW: f64 = 900.0;

    fn count(m: &mut HashMap<String, VecDeque<f64>>, key: &str, now: f64) -> usize {
        let q = m.entry(key.to_string()).or_default();
        while q.front().is_some_and(|&t| t <= now - Self::WINDOW) {
            q.pop_front();
        }
        q.len()
    }

    pub fn allowed(&self, ip: &str, user: &str, now: f64) -> bool {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::count(&mut m, &format!("ip:{ip}"), now) < Self::LIMIT && Self::count(&mut m, &format!("u:{user}"), now) < Self::LIMIT
    }

    pub fn failed(&self, ip: &str, user: &str, now: f64) {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for k in [format!("ip:{ip}"), format!("u:{user}")] {
            m.entry(k).or_default().push_back(now);
        }
    }

    pub fn succeeded(&self, ip: &str, user: &str) {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.remove(&format!("ip:{ip}"));
        m.remove(&format!("u:{user}"));
    }
}

#[derive(Clone, Debug)]
pub struct Oturum {
    pub user: String,
    pub rol: Rol,
    pub csrf: String,
    expires: f64,
    /// Kafe sahibinin kişisel veriye bakma gerekçesi ve geçerlilik sonu (panel/gerekce.rs)
    pub gerekce: Option<(String, f64)>,
    /// Sahip oturumunda girişteki merkez parola özeti; merkezde değişince (eşitleme) oturum düşer
    pub surum: String,
    /// Admin kilidi açık kalma sonu ve açılıştaki admin özeti; özet merkezde değişince kilit kapanır
    pub admin_kadar: f64,
    pub admin_surum: String,
    /// Bağlı kullanıcılarda 10 sn yenilemenin açıldığı (ya da pencerenin yenilendiği) an; 0 = hiç açılmadı
    pub yenile_acildi: f64,
}

/// Bellekte oturumlar (panel yeniden başlayınca herkes yeniden girer). 12 saat geçerli.
#[derive(Default)]
pub struct Oturumlar(Mutex<HashMap<String, Oturum>>);

impl Oturumlar {
    const TTL: f64 = 12.0 * 3600.0;

    pub fn create(&self, user: &str, rol: Rol, now: f64, surum: &str) -> String {
        let token = ortak::random_hex(32);
        let o = Oturum { user: user.into(), rol, csrf: ortak::random_hex(16), expires: now + Self::TTL, gerekce: None, surum: surum.into(),
                       admin_kadar: 0.0, admin_surum: String::new(), yenile_acildi: 0.0 };
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, v| v.expires > now);
        m.insert(token.clone(), o);
        token
    }

    pub fn get(&self, token: &str, now: f64) -> Option<Oturum> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(token).filter(|o| o.expires > now).cloned()
    }

    pub fn set_gerekce(&self, token: &str, text: &str, until: f64) {
        if let Some(o) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get_mut(token) {
            o.gerekce = Some((text.to_string(), until));
        }
    }

    pub fn set_admin(&self, token: &str, until: f64, surum: &str) {
        if let Some(o) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get_mut(token) {
            (o.admin_kadar, o.admin_surum) = (until, surum.to_string());
        }
    }

    pub fn set_yenile(&self, token: &str, t: f64) {
        if let Some(o) = self.0.lock().unwrap_or_else(|e| e.into_inner()).get_mut(token) {
            o.yenile_acildi = t;
        }
    }

    pub fn remove(&self, token: &str) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(token);
    }

    /// Parola değişince / hesap silinince o kullanıcının bütün oturumları kapanır (`keep` hariç).
    pub fn remove_user(&self, user: &str, keep: Option<&str>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).retain(|t, o| o.user != user || Some(t.as_str()) == keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("wfc-hesap-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&d);
        d.join("hesaplar.json")
    }

    #[test]
    fn admin_ozeti_merkezden() {
        let h = Hesaplar::new(tmp());
        let oz = digest("merkez-parola-1", "ab12", 120_000);
        assert!(h.set_admin_ozet("ab12", &oz, 120_000).unwrap());
        assert!(!h.set_admin_ozet("ab12", &oz, 120_000).unwrap()); // aynı: değişmedi
        assert_eq!(h.verify(ADMIN, "merkez-parola-1"), Some(Rol::Hizmet));
        assert_eq!(h.ozet(ADMIN), Some(oz.clone()));
        assert!(h.set_admin_ozet("ab12", &oz.to_uppercase(), 120_000).is_err()); // büyük harf: verify küçük harfle karşılaştırır, admin kilitlenirdi
        assert!(h.set_admin_ozet("", "zz", 120_000).is_err() && h.set_admin_ozet("ab", "ab", 10).is_err()); // bozuk özet yazılmaz
    }

    #[test]
    fn admin_verify_and_password() {
        let h = Hesaplar::new(tmp());
        assert_eq!(h.verify(ADMIN, "hizmet-parola-1"), None);
        h.set_admin("hizmet-parola-1").unwrap();
        assert_eq!(h.verify(ADMIN, "hizmet-parola-1"), Some(Rol::Hizmet));
        assert_eq!(h.verify(ADMIN, "yanlis-parola"), None);
        assert_eq!(h.verify("yok", "hizmet-parola-1"), None);
        h.set_admin("yeni-admin-parola").unwrap();
        assert_eq!(h.verify(ADMIN, "yeni-admin-parola"), Some(Rol::Hizmet));
        h.keep_only_admin().unwrap();
        assert_eq!(h.load().unwrap().len(), 1);
        let raw = std::fs::read_to_string(&h.path).unwrap();
        assert!(!raw.contains("yeni-admin-parola")); // parola düz metin saklanmaz
    }

    #[test]
    fn corrupt_file_does_not_open_setup() {
        let p = tmp();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{bozuk").unwrap();
        let h = Hesaplar::new(&p);
        assert!(h.load().is_err() && h.set_admin("x".repeat(12).as_str()).is_err()); // bozuk dosya ezilmez
        assert_eq!(h.verify(ADMIN, "hizmet-parola-1"), None);
    }

    #[test]
    fn rules_guard_sessions() {
        assert!(password_problem("kisa").is_some() && password_problem("onkarakter").is_none());
        let g = LoginGuard::default();
        for _ in 0..5 {
            assert!(g.allowed("1.1.1.1", "mudur", 10.0));
            g.failed("1.1.1.1", "mudur", 10.0);
        }
        assert!(!g.allowed("1.1.1.1", "baska", 10.0)); // IP kilitli
        assert!(!g.allowed("2.2.2.2", "mudur", 10.0)); // kullanıcı kilitli
        assert!(g.allowed("1.1.1.1", "mudur", 10.0 + 901.0)); // 15 dk sonra açılır
        let s = Oturumlar::default();
        let t1 = s.create("mudur", Rol::Sahip, 0.0, "ozet-1");
        let t2 = s.create("mudur", Rol::Sahip, 0.0, "ozet-1");
        assert_eq!(s.get(&t1, 1.0).unwrap().rol, Rol::Sahip);
        assert_eq!(s.get(&t1, 1.0).unwrap().surum, "ozet-1");
        assert!(s.get(&t1, 13.0 * 3600.0).is_none()); // süresi doldu
        s.remove_user("mudur", Some(&t2));
        assert!(s.get(&t1, 1.0).is_none() && s.get(&t2, 1.0).is_some());
    }
}
