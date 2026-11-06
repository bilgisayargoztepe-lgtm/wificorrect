//! Yönetim merkezi güvenliği (Python guvenlik.py): parola özetleri (cihazla aynı biçim), oturumlar, giriş kilidi, hız sınırı.
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use serde_json::Value;
use sha2::Sha256;

/// Cihazdaki hesap.rs ile aynı: cihaz aynı özeti internetsiz doğrular.
pub const YINELEME: u32 = 120_000;
pub const EN_AZ: usize = 10;
/// 0/O, 1/l/I, 8/B karışmasın.
pub const HARFLER: &str = "abcdefghjkmnpqrstuvwxyzACDEFGHJKLMNPQRSTUVWXYZ2345679";

/// Müşteri numarası: 7 hane, `[1-9][0-9]{6}`, yalnızca ASCII (Unicode rakamlar kilidi atlatırdı).
pub fn numara_gecerli(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && (b'1'..=b'9').contains(&b[0]) && b[1..].iter().all(u8::is_ascii_digit)
}

/// PBKDF2-HMAC-SHA256; tuz metnin baytları, hex.
pub fn ozet(pw: &str, tuz: &str, yineleme: u32) -> String {
    let mut cikti = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(pw.as_bytes(), tuz.as_bytes(), yineleme, &mut cikti);
    cikti.iter().map(|b| format!("{b:02x}")).collect()
}

/// (tuz, özet, yineleme)
pub fn yeni_kayit(pw: &str) -> (String, String, u32) {
    let tuz = rastgele::token_hex(16);
    let oz = ozet(pw, &tuz, YINELEME);
    (tuz, oz, YINELEME)
}

pub fn sabit_esit(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn dogru(pw: &str, tuz: &str, oz: &str, yineleme: u32) -> bool {
    if yineleme == 0 {
        return false;
    }
    sabit_esit(ozet(pw, tuz, yineleme).as_bytes(), oz.as_bytes())
}

/// Olmayan hesapta da aynı süre geçsin (numara ya da kullanıcı adı yanıt süresinden anlaşılmasın).
pub fn bos_dogrulama(pw: &str) {
    let _ = ozet(pw, &"0".repeat(32), YINELEME);
}

pub fn parola_uret(n: usize) -> String {
    (0..n).map(|_| rastgele::choice(HARFLER)).collect()
}

/// Bütün rastgelelik buradan geçer (üretimde işletim sistemi; testte `belirlenimli()` ile tekrarlanabilir).
pub mod rastgele {
    #[cfg(test)]
    use sha2::{Digest, Sha256};

    #[cfg(test)]
    use std::cell::RefCell;

    /// Belirlenimli kipin durumu: tür başına sayaçlar ve üretilen belirteçler (altın kayıtta `<Tn>` olurlar).
    #[cfg(test)]
    #[derive(Default)]
    struct Kip {
        sayaclar: std::collections::HashMap<&'static str, u64>,
        uretilen: Vec<String>,
    }

    #[cfg(test)]
    thread_local! {
        /// None: kapalı. İş parçacığına özel: paralel testler birbirini bozmaz.
        static BELIRLENIMLI: RefCell<Option<Kip>> = const { RefCell::new(None) };
    }

    /// Yalnızca testler: bu iş parçacığında rastgelelik belirlenimli olur; sayaçlar 1'den başlar.
    /// Her işlev kendi sayacıyla `sha256("<tür>:<sayaç>:<blok>")` bloklarını birleştirip `n` bayta kırpar.
    /// Dönen koruyucu düşünce kip kapanır.
    #[cfg(test)]
    pub struct Belirlenimli;

    #[cfg(test)]
    pub fn belirlenimli() -> Belirlenimli {
        BELIRLENIMLI.with(|b| *b.borrow_mut() = Some(Kip::default()));
        Belirlenimli
    }

    #[cfg(test)]
    impl Drop for Belirlenimli {
        fn drop(&mut self) {
            BELIRLENIMLI.with(|b| *b.borrow_mut() = None);
        }
    }

    /// Yalnızca testler: belirlenimli kip açıldığından beri `token_urlsafe` ve `token_hex`'in döndürdükleri, sırayla.
    #[cfg(test)]
    pub fn uretilenler() -> Vec<String> {
        BELIRLENIMLI.with(|b| b.borrow().as_ref().map(|k| k.uretilen.clone()).unwrap_or_default())
    }

    #[cfg(test)]
    fn uretildi(t: &str) {
        BELIRLENIMLI.with(|b| {
            if let Some(k) = b.borrow_mut().as_mut() {
                k.uretilen.push(t.to_string());
            }
        });
    }

    #[cfg(test)]
    fn belirlenimli_baytlar(tur: &'static str, n: usize) -> Option<Vec<u8>> {
        BELIRLENIMLI.with(|b| {
            let mut b = b.borrow_mut();
            let sayaclar = &mut b.as_mut()?.sayaclar;
            let sayac = sayaclar.entry(tur).or_insert(0);
            *sayac += 1;
            let mut cikti = Vec::new();
            let mut blok = 0u64;
            while cikti.len() < n {
                cikti.extend_from_slice(&Sha256::digest(format!("{tur}:{sayac}:{blok}").as_bytes()));
                blok += 1;
            }
            cikti.truncate(n);
            Some(cikti)
        })
    }

    fn baytlar(tur: &'static str, n: usize) -> Vec<u8> {
        #[cfg(test)]
        if let Some(v) = belirlenimli_baytlar(tur, n) {
            return v;
        }
        let _ = tur;
        let mut v = vec![0u8; n];
        getrandom::getrandom(&mut v).expect("işletim sistemi rastgeleliği");
        v
    }

    fn base64url(b: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
        for p in b.chunks(3) {
            let v = (p[0] as u32) << 16 | (*p.get(1).unwrap_or(&0) as u32) << 8 | *p.get(2).unwrap_or(&0) as u32;
            o.push(A[(v >> 18) as usize & 63] as char);
            o.push(A[(v >> 12) as usize & 63] as char);
            if p.len() > 1 {
                o.push(A[(v >> 6) as usize & 63] as char);
            }
            if p.len() > 2 {
                o.push(A[v as usize & 63] as char);
            }
        }
        o
    }

    /// `n` rastgele bayt, base64url (dolgusuz).
    pub fn token_urlsafe(n: usize) -> String {
        let t = base64url(&baytlar("urlsafe", n));
        #[cfg(test)]
        uretildi(&t);
        t
    }

    /// `n` rastgele bayt, küçük harfli hex (2n karakter).
    pub fn token_hex(n: usize) -> String {
        let t: String = baytlar("hex", n).iter().map(|b| format!("{b:02x}")).collect();
        #[cfg(test)]
        uretildi(&t);
        t
    }

    pub fn token_bytes(n: usize) -> Vec<u8> {
        baytlar("bytes", n)
    }

    fn u64_oku(b: &[u8]) -> u64 {
        u64::from_be_bytes(b[..8].try_into().expect("8 bayt"))
    }

    fn asagi(tur: &'static str, n: u64) -> u64 {
        assert!(n > 0, "randbelow(0)");
        // belirlenimli kip: düz kalan (altın kayıt üreticisi kaydet.py ile birebir)
        #[cfg(test)]
        if let Some(b) = belirlenimli_baytlar(tur, 8) {
            return u64_oku(&b) % n;
        }
        // sapmasız: n'in tam katlarının dışına düşenler atılır
        let sinir = (u64::MAX / n) * n;
        loop {
            let x = u64_oku(&baytlar(tur, 8));
            if x < sinir {
                return x % n;
            }
        }
    }

    /// [0, n) aralığında düzgün dağılımlı sayı.
    pub fn randbelow(n: u64) -> u64 {
        asagi("below", n)
    }

    pub fn choice(alfabe: &str) -> char {
        let h: Vec<char> = alfabe.chars().collect();
        h[asagi("choice", h.len() as u64) as usize]
    }
}

/// Bir oturumun bilgileri (sorgulanınca kopyası verilir).
#[derive(Clone, Debug, PartialEq)]
pub struct Oturum {
    pub user: String,
    pub role: String,
    pub csrf: String,
    pub created: f64,
    pub last: f64,
    /// (metin, bitiş)
    pub gerekce: Option<(String, f64)>,
    /// Hesabın o anki parola özeti; değişince oturum geçersizleşir.
    pub surum: String,
    /// Oturuma bağlı ek alanlar (ör. AP oturumunun altındaki cihaz paneli oturumu); asıl alanları ezmez.
    pub ek: BTreeMap<String, Value>,
}

/// Bellek içi oturumlar: boşta 30 dk, en çok 12 saat. Servis yeniden başlarsa herkes yeniden girer.
pub struct Oturumlar {
    pub bosta: f64,
    pub en_cok: f64,
    s: Mutex<HashMap<String, Oturum>>,
}

impl Default for Oturumlar {
    fn default() -> Self {
        Oturumlar::new(1800.0, 43200.0)
    }
}

impl Oturumlar {
    pub fn new(bosta: f64, en_cok: f64) -> Oturumlar {
        Oturumlar { bosta, en_cok, s: Mutex::new(HashMap::new()) }
    }

    fn dolmus(&self, o: &Oturum, now: f64) -> bool {
        now - o.last > self.bosta || now - o.created > self.en_cok
    }

    /// Yeni oturum belirteci. Süresi dolmuş oturumlar burada silinir (bir daha bakılmayan oturum bellekte kalmasın).
    pub fn create(&self, user: &str, role: &str, now: f64, surum: &str, ek: BTreeMap<String, Value>) -> String {
        let token = rastgele::token_urlsafe(32);
        let mut s = crate::ortak::kilitle(&self.s);
        s.retain(|_, o| !self.dolmus(o, now));
        s.insert(
            token.clone(),
            Oturum {
                user: user.to_string(),
                role: role.to_string(),
                csrf: rastgele::token_urlsafe(24),
                created: now,
                last: now,
                gerekce: None,
                surum: surum.to_string(),
                ek,
            },
        );
        token
    }

    pub fn get(&self, token: &str, now: f64) -> Option<Oturum> {
        let mut s = crate::ortak::kilitle(&self.s);
        let o = s.get_mut(token)?;
        if now - o.last > self.bosta || now - o.created > self.en_cok {
            s.remove(token);
            return None;
        }
        o.last = now;
        Some(o.clone())
    }

    pub fn drop(&self, token: &str) {
        crate::ortak::kilitle(&self.s).remove(token);
    }

    pub fn drop_user(&self, user: &str) {
        crate::ortak::kilitle(&self.s).retain(|_, o| o.user != user);
    }

    pub fn set_gerekce(&self, token: &str, metin: &str, bitis: f64) {
        if let Some(o) = crate::ortak::kilitle(&self.s).get_mut(token) {
            o.gerekce = Some((metin.to_string(), bitis));
        }
    }
}

/// panel → cihaz.wificorrect.com geçişi: tek kullanımlık, 60 sn (URL'de taşınır, çerez değil).
pub struct GecisBelirtecleri {
    /// belirteç → (numara, bitiş, özet, rol)
    b: Mutex<HashMap<String, (String, f64, String, String)>>,
}

impl Default for GecisBelirtecleri {
    fn default() -> Self {
        GecisBelirtecleri { b: Mutex::new(HashMap::new()) }
    }
}

impl GecisBelirtecleri {
    pub const SURE: f64 = 60.0;

    pub fn new() -> GecisBelirtecleri {
        GecisBelirtecleri::default()
    }

    /// `ozet`: müşterinin o anki parola özeti; belirteç alındıktan sonra parola değişirse cihaz oturumu açılmaz.
    /// `rol`: "musteri" (panel.wificorrect.com) ya da "yonetici" (yönetimden doğrudan geçiş).
    pub fn uret(&self, numara: &str, simdi: f64, ozet: &str, rol: &str) -> String {
        let t = rastgele::token_urlsafe(32);
        let mut b = crate::ortak::kilitle(&self.b);
        b.retain(|_, v| v.1 > simdi);
        b.insert(t.clone(), (numara.to_string(), simdi + Self::SURE, ozet.to_string(), rol.to_string()));
        t
    }

    /// (numara, özet, rol) ya da None.
    pub fn tuket(&self, token: &str, simdi: f64) -> Option<(String, String, String)> {
        let v = crate::ortak::kilitle(&self.b).remove(token)?;
        (v.1 > simdi).then(|| (v.0, v.2, v.3))
    }
}

/// Kısa ömürlü, tek kullanımlık belirteç → değer (URL'de taşınır, çerez değil; ör. cihaz paneli → AP arayüzü geçişi).
/// Süresi dolanlar her üretimde silinir.
pub struct TekKullanimlik<T> {
    pub sure: f64,
    b: Mutex<HashMap<String, (f64, T)>>,
}

impl<T> TekKullanimlik<T> {
    /// Testler: bekleyen belirteç sayısı (Python `len(b._b)`).
    #[cfg(test)]
    pub(crate) fn uzunluk(&self) -> usize {
        crate::ortak::kilitle(&self.b).len()
    }

    pub fn new(sure: f64) -> TekKullanimlik<T> {
        TekKullanimlik { sure, b: Mutex::new(HashMap::new()) }
    }

    pub fn uret(&self, deger: T, simdi: f64) -> String {
        let t = rastgele::token_urlsafe(32);
        let mut b = crate::ortak::kilitle(&self.b);
        b.retain(|_, v| v.0 > simdi);
        b.insert(t.clone(), (simdi + self.sure, deger));
        t
    }

    /// Değer ya da None (yok, kullanılmış ya da süresi dolmuş).
    pub fn tuket(&self, token: &str, simdi: f64) -> Option<T> {
        let v = crate::ortak::kilitle(&self.b).remove(token)?;
        (v.0 > simdi).then_some(v.1)
    }
}

/// IP başına ve hesap başına kayan pencere: `limit` deneme `pencere` sn içinde → kilit. Deneme doğrulamadan ÖNCE
/// sayılır; başarılı giriş yalnızca kendi denemesini ve o hesabın sayacını siler.
pub struct GirisKilidi {
    pub limit: usize,
    pub pencere: f64,
    f: Mutex<HashMap<(&'static str, String), Vec<f64>>>,
}

impl Default for GirisKilidi {
    fn default() -> Self {
        GirisKilidi::new(5, 900.0)
    }
}

impl GirisKilidi {
    pub fn new(limit: usize, pencere: f64) -> GirisKilidi {
        GirisKilidi { limit, pencere, f: Mutex::new(HashMap::new()) }
    }

    fn son(&self, f: &mut HashMap<(&'static str, String), Vec<f64>>, k: &(&'static str, String), now: f64) -> usize {
        let v = f.entry(k.clone()).or_default();
        v.retain(|t| *t > now - self.pencere);
        v.len()
    }

    pub fn attempt(&self, ip: &str, user: &str, now: f64) -> bool {
        let anahtarlar = [("ip", ip.to_string()), ("user", user.to_string())];
        let mut f = crate::ortak::kilitle(&self.f);
        let dolu = anahtarlar.iter().map(|k| self.son(&mut f, k, now) >= self.limit).fold(false, |a, b| a || b);
        if dolu {
            return false;
        }
        for k in anahtarlar {
            f.entry(k).or_default().push(now);
        }
        true
    }

    pub fn succeed(&self, ip: &str, user: &str, now: f64) {
        let mut f = crate::ortak::kilitle(&self.f);
        if let Some(v) = f.get_mut(&("ip", ip.to_string())) {
            if let Some(i) = v.iter().position(|t| *t == now) {
                v.remove(i);
            }
        }
        f.remove(&("user", user.to_string()));
    }
}

/// Anahtar başına kayan pencere (API: IP başına dakikada 20 istek). Pencere başına bir kez, son isteği pencereden
/// eski anahtarlar bellekten silinir (rastgele kimlik/IP ile bellek şişmesin).
pub struct HizSiniri {
    pub limit: usize,
    pub pencere: f64,
    ic: Mutex<HizIc>,
}

#[derive(Default)]
struct HizIc {
    z: HashMap<String, VecDeque<f64>>,
    temizlik: Option<f64>,
}

impl Default for HizSiniri {
    fn default() -> Self {
        HizSiniri::new(20, 60.0)
    }
}

impl HizSiniri {
    pub fn new(limit: usize, pencere: f64) -> HizSiniri {
        HizSiniri { limit, pencere, ic: Mutex::new(HizIc::default()) }
    }

    pub fn izin(&self, anahtar: &str, now: f64) -> bool {
        let mut ic = crate::ortak::kilitle(&self.ic);
        if ic.temizlik.map(|t| now - t >= self.pencere).unwrap_or(true) {
            ic.temizlik = Some(now);
            let p = self.pencere;
            ic.z.retain(|_, q| !(q.is_empty() || q.back().is_some_and(|s| *s <= now - p)));
        }
        let q = ic.z.entry(anahtar.to_string()).or_default();
        while q.front().is_some_and(|s| *s <= now - self.pencere) {
            q.pop_front();
        }
        if q.len() >= self.limit {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ozet_cihazla_ayni() {
        assert_eq!(ozet("parola-12345", "a1b2c3d4", 120000), "ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495");
    }

    #[test]
    fn ozet_pythonun_urettigi_degerlerle_ayni() {
        // py -3 -I: hashlib.pbkdf2_hmac("sha256", pw.encode(), tuz.encode(), n).hex()
        let oz = "4e329d931fa97a1458d9c27602e6e332ecdf4db62fef2f9e1ebcb8f33467b012";
        assert_eq!(ozet("şifre-ğüÇ 12", "tuz-xyz", 120000), oz);
        assert!(dogru("şifre-ğüÇ 12", "tuz-xyz", oz, 120000));
        assert!(!dogru("şifre-ğüÇ 13", "tuz-xyz", oz, 120000));
        assert_eq!(ozet("parola", &"0".repeat(32), 1000), "81e1b4d37538a8da1f34c3e8cddefae1c739e50f5e35a7ec6931596d3e0cec00");
        assert!(!dogru("parola", "x", "81e1", 0));
    }

    #[test]
    fn yeni_kayit_ve_dogrulama() {
        let (tuz, oz, y) = yeni_kayit("parola-12345");
        assert!(tuz.len() == 32 && y == 120000 && dogru("parola-12345", &tuz, &oz, y));
        assert!(!dogru("parola-12346", &tuz, &oz, y) && !dogru("", &tuz, &oz, y));
        bos_dogrulama("x");
    }

    #[test]
    fn parola_uret_karisan_harf_yok() {
        for _ in 0..200 {
            let p = parola_uret(12);
            assert!(p.chars().count() == 12 && !p.chars().any(|c| "0O1lI8B".contains(c)), "{p}");
        }
    }

    #[test]
    fn numara_yalniz_ascii_yedi_hane() {
        assert!(numara_gecerli("1537344"));
        assert!(!numara_gecerli("0537344") && !numara_gecerli("153734") && !numara_gecerli("15373445"));
        assert!(!numara_gecerli("1537344\n") && !numara_gecerli("١٥٣٧٣٤٤") && !numara_gecerli(""));
    }

    #[test]
    fn oturum_gerekce_ve_sure() {
        let s = Oturumlar::default();
        let t = s.create("100001", "musteri", 100.0, "", BTreeMap::new());
        assert!(s.get(&t, 100.0).unwrap().gerekce.is_none());
        s.set_gerekce(&t, "Müşteri şikâyeti", 1900.0);
        assert_eq!(s.get(&t, 101.0).unwrap().gerekce, Some(("Müşteri şikâyeti".to_string(), 1900.0)));
        assert!(s.get(&t, 101.0 + 1801.0).is_none()); // boşta 30 dk
        assert!(s.get(&t, 101.0).is_none()); // süresi dolan silindi
    }

    #[test]
    fn oturum_ek_alan_mutlak_sure_ve_silme() {
        let s = Oturumlar::default();
        let mut ek = BTreeMap::new();
        ek.insert("cihaz".to_string(), Value::from("10.99.0.11"));
        let t = s.create("a", "musteri", 0.0, "ozet1", ek);
        let o = s.get(&t, 10.0).unwrap();
        assert_eq!((o.user.as_str(), o.role.as_str(), o.surum.as_str()), ("a", "musteri", "ozet1"));
        assert_eq!(o.ek["cihaz"], "10.99.0.11");
        assert_eq!(o.csrf.len(), 32);
        // her istek boşta süresini uzatır ama 12 saat mutlak
        let mut t0 = 10.0;
        while t0 < 43200.0 {
            t0 += 1000.0;
            if s.get(&t, t0).is_none() {
                break;
            }
        }
        assert!(s.get(&t, 43201.0).is_none());
        let t1 = s.create("b", "yonetici", 0.0, "", BTreeMap::new());
        let t2 = s.create("b", "yonetici", 0.0, "", BTreeMap::new());
        s.drop(&t1);
        assert!(s.get(&t1, 1.0).is_none() && s.get(&t2, 1.0).is_some());
        s.drop_user("b");
        assert!(s.get(&t2, 2.0).is_none());
    }

    #[test]
    fn create_suresi_dolmuslari_siler() {
        let s = Oturumlar::default();
        s.create("a", "r", 0.0, "", BTreeMap::new());
        s.create("b", "r", 5000.0, "", BTreeMap::new());
        assert_eq!(crate::ortak::kilitle(&s.s).len(), 1);
    }

    #[test]
    fn kilit_ve_hiz() {
        let k = GirisKilidi::default();
        assert!((0..5).all(|_| k.attempt("1.1.1.1", "100001", 0.0)));
        assert!(!k.attempt("1.1.1.1", "100001", 1.0) && !k.attempt("2.2.2.2", "100001", 1.0)); // hesap kilitli
        assert!(k.attempt("2.2.2.2", "100002", 1.0));
        assert!(k.attempt("1.1.1.1", "100001", 901.0)); // 15 dk sonra açılır
        let h = HizSiniri::new(3, 60.0);
        assert_eq!([0.0, 1.0, 2.0, 3.0].map(|t| h.izin("ip", t)), [true, true, true, false]);
        assert!(h.izin("ip", 61.0));
    }

    #[test]
    fn basarili_giris_yalniz_kendi_denemesini_siler() {
        let k = GirisKilidi::default();
        for t in 0..4 {
            assert!(k.attempt("1.1.1.1", "u", t as f64));
        }
        k.succeed("1.1.1.1", "u", 3.0);
        assert!(k.attempt("1.1.1.1", "u", 4.0) && k.attempt("1.1.1.1", "u", 5.0));
        assert!(!k.attempt("1.1.1.1", "u", 6.0)); // ip sayacında 5 deneme var
        // hesap sayacı silindi: başka IP'den hâlâ girilebilir
        assert!(k.attempt("3.3.3.3", "u", 7.0));
    }

    #[test]
    fn hiz_siniri_eski_anahtarlari_unutur() {
        let h = HizSiniri::new(1, 60.0);
        assert!(h.izin("a", 0.0) && h.izin("b", 1.0) && !h.izin("a", 2.0));
        assert!(h.izin("c", 200.0)); // pencereden eski anahtarlar bellekten silinir
        let anahtarlar: Vec<String> = crate::ortak::kilitle(&h.ic).z.keys().cloned().collect();
        assert_eq!(anahtarlar, vec!["c".to_string()]);
    }

    #[test]
    fn gecis_belirteci_tek_kullanimlik_ve_60_sn() {
        let g = GecisBelirtecleri::new();
        let t = g.uret("1537344", 100.0, "oz", "musteri");
        assert_eq!(g.tuket(&t, 120.0), Some(("1537344".into(), "oz".into(), "musteri".into())));
        assert_eq!(g.tuket(&t, 121.0), None); // tek kullanım
        let t = g.uret("1", 100.0, "", "yonetici");
        assert_eq!(g.tuket(&t, 160.5), None); // 60 sn
        assert_eq!(g.tuket("", 0.0), None);
    }

    #[test]
    fn tek_kullanimlik_deger_ve_temizlik() {
        let k: TekKullanimlik<(String, u32)> = TekKullanimlik::new(60.0);
        let t = k.uret(("ap".to_string(), 7), 0.0);
        assert_eq!(k.tuket(&t, 59.0), Some(("ap".to_string(), 7)));
        assert_eq!(k.tuket(&t, 59.0), None);
        k.uret(("x".to_string(), 1), 0.0);
        k.uret(("y".to_string(), 2), 100.0); // üretimde süresi dolanlar silinir
        assert_eq!(crate::ortak::kilitle(&k.b).len(), 1);
    }

    #[test]
    fn belirlenimli_kip_tekrarlanabilir() {
        {
            let _k = rastgele::belirlenimli();
            let a = rastgele::token_urlsafe(32);
            let b = rastgele::token_urlsafe(32);
            assert!(a != b && a.len() == 43 && b.len() == 43);
            assert_eq!(rastgele::token_hex(16).len(), 32);
            assert_eq!(rastgele::token_bytes(40).len(), 40);
            assert!(rastgele::randbelow(10) < 10);
            let c = rastgele::choice("abc");
            assert!("abc".contains(c));
        }
        // yeni kip sayaçları baştan başlatır: aynı sıra, aynı değerler
        let a2 = {
            let _k = rastgele::belirlenimli();
            rastgele::token_urlsafe(32)
        };
        let b2 = {
            let _k = rastgele::belirlenimli();
            rastgele::token_urlsafe(32)
        };
        assert_eq!(a2, b2);
    }

    #[test]
    fn belirlenimli_kip_sabit_degerler() {
        use sha2::{Digest, Sha256};
        let _k = rastgele::belirlenimli();
        // token_hex(16): sha256("hex:1:0")'ın ilk 16 baytı
        let beklenen: String = Sha256::digest(b"hex:1:0")[..16].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(rastgele::token_hex(16), beklenen);
        // token_bytes(40): iki blok
        let mut e = Sha256::digest(b"bytes:1:0").to_vec();
        e.extend_from_slice(&Sha256::digest(b"bytes:1:1"));
        e.truncate(40);
        assert_eq!(rastgele::token_bytes(40), e);
        // randbelow(1000): sha256("below:1:0")'ın ilk 8 baytı büyük sonlu u64 % 1000
        let u = u64::from_be_bytes(Sha256::digest(b"below:1:0")[..8].try_into().unwrap());
        assert_eq!(rastgele::randbelow(1000), u % 1000);
        // choice: kendi sayacı ("choice")
        let u = u64::from_be_bytes(Sha256::digest(b"choice:1:0")[..8].try_into().unwrap());
        assert_eq!(rastgele::choice("abcdef"), "abcdef".chars().nth((u % 6) as usize).unwrap());
        // urlsafe(3): base64url, dolgusuz
        let b = Sha256::digest(b"urlsafe:1:0");
        let ilk = rastgele::token_urlsafe(3);
        assert_eq!(ilk.len(), 4);
        assert_eq!(ilk.as_bytes()[0], b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"[(b[0] >> 2) as usize]);
    }

    #[test]
    fn uretimde_rastgele_os_den_gelir() {
        let a = rastgele::token_urlsafe(32);
        let b = rastgele::token_urlsafe(32);
        assert!(a != b && a.len() == 43);
        assert!(rastgele::token_hex(16).bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(rastgele::token_urlsafe(24).len(), 32);
        assert_eq!(rastgele::token_urlsafe(1).len(), 2);
        assert_eq!(rastgele::token_urlsafe(2).len(), 3);
    }
}
