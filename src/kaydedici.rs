//! 5651 kaydedici (eski logger.py; docs/MASTER_ENGINEERING.md §14, RUST_YENIDEN_YAZIM.md A7).
//! conntrack olayları → trafik.csv, dnsmasq sorguları (journald) → dns.csv, her ikisi kişinin günlük dosyasına;
//! 30 sn'de bir oturum bakımı (süre dolumu, IP değişimi, IP başkasına verildi), disk doluluğu.
//! Wi-Fi takılma bekçisi (A9) Wi-Fi açılınca eklenecek.

use crate::ayar::Config;
use crate::ortak::{self, Row, Runner, Session};
use crate::{hiz, trafik};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const MAX_OPEN_FLOWS: usize = 200_000; // NEW zaman önbelleği sınırı
const QUEUE: usize = 100_000; // dolarsa okuyucu bekler → çekirdek ENOBUFS → LOG_BOSLUK
/// Sayaç tablosu (`inet wfc_hiz`); yoksa kaydedici yalnızca bunu yükler.
const HIZ_NFT: &str = "/etc/wificorrect/hiz.nft";

/// Yalnızca müşteri ağından başlayan akışlar; süzme çekirdekte (BPF). -b: ani yükte olaylar tamponda beklesin.
pub fn ct_cmd(subnet: &str) -> Vec<String> {
    ["conntrack", "-E", "-e", "NEW,DESTROY", "-o", "timestamp,extended,id", "-b", "33554432", "-s", subnet]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// `-n 0`: yeniden başlarken eski satırlar tekrar okunmaz (aradaki boşluk LOG_BOSLUK ile belgelenir).
pub fn dns_cmd() -> Vec<String> {
    ["journalctl", "-f", "-n", "0", "-o", "short-unix", "-u", "dnsmasq.service", "--no-pager"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------- ayrıştırıcılar
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Tuple {
    pub src: String,
    pub dst: String,
    pub sport: String,
    pub dport: String,
    pub packets: String,
    pub bytes: String,
}

impl Tuple {
    fn slot(&mut self, key: &str) -> Option<&mut String> {
        Some(match key {
            "src" => &mut self.src,
            "dst" => &mut self.dst,
            "sport" => &mut self.sport,
            "dport" => &mut self.dport,
            "packets" => &mut self.packets,
            "bytes" => &mut self.bytes,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub ts: f64,
    pub new: bool,
    pub proto: String,
    pub orig: Tuple,
    pub reply: Tuple,
    pub delta: String,
    pub id: String,
}

/// `[epoch]  [NEW|DESTROY] ipv4 2 tcp 6 ... src= dst= sport= dport= [packets= bytes=] src= ... id=`.
/// İlk src/dst/sport/dport/packets/bytes grubu orig, ikincisi reply. Son `id=` conntrack kimliği. IPv6 atlanır.
pub fn parse_conntrack(line: &str) -> Option<Flow> {
    let mut it = line.split_whitespace();
    let ts = it.next()?.strip_prefix('[')?.strip_suffix(']')?.parse::<f64>().ok()?;
    let new = match it.next()? {
        "[NEW]" => true,
        "[DESTROY]" => false,
        _ => return None,
    };
    if it.next()? != "ipv4" {
        return None;
    }
    it.next()?;
    let proto = it.next()?.to_string();
    let mut f = Flow { ts, new, proto, orig: Tuple::default(), reply: Tuple::default(), delta: String::new(), id: String::new() };
    for tok in it {
        let Some((k, v)) = tok.split_once('=') else { continue };
        if !k.starts_with(|c: char| c.is_ascii_lowercase()) || !k.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            continue;
        }
        match k {
            "delta-time" => f.delta = v.to_string(),
            "id" => f.id = v.to_string(),
            _ => {
                let in_orig = f.orig.slot(k).is_some_and(|s| !s.is_empty());
                let t = if in_orig { &mut f.reply } else { &mut f.orig };
                if let Some(slot) = t.slot(k) {
                    *slot = v.to_string();
                }
            }
        }
    }
    (!f.orig.src.is_empty()).then_some(f)
}

/// dnsmasq `query[TIP] ad from IP` satırı (log-queries=extra ya da düz) → (tip, ad, ip).
pub fn parse_dns(line: &str) -> Option<(String, String, String)> {
    let rest = &line[line.find("dnsmasq[")?..];
    let rest = &rest[rest.find("]: ")? + 3..];
    let t: Vec<&str> = rest.split_whitespace().collect();
    let i = if t.first()?.starts_with("query[") {
        0
    } else if t.len() > 2 && t[0].bytes().all(|b| b.is_ascii_digit()) && t[2].starts_with("query[") {
        2
    } else {
        return None;
    };
    let qtype = t[i].strip_prefix("query[")?.strip_suffix(']')?;
    (t.get(i + 2) == Some(&"from")).then(|| (qtype.to_string(), t[i + 1].to_string(), t.get(i + 3).unwrap_or(&"").to_string()))
}

/// `journalctl -o short-unix` satırının başındaki epoch.
pub fn journal_epoch(line: &str) -> Option<f64> {
    let first = line.split_whitespace().next()?;
    first.contains('.').then(|| first.parse::<f64>().ok()).flatten()
}

fn flow_row(f: &Flow, sess: Option<&Session>, mac: &str) -> Row {
    let olay = if f.new { "BAGLANTI_BASLA" } else { "BAGLANTI_BITIS" };
    let mut r = Row::new(olay, &ortak::now_iso(f.ts))
        .set("mac", mac)
        .set("ic_ip", &f.orig.src)
        .set("protokol", &f.proto)
        .set("ic_port", &f.orig.sport)
        .set("hedef_ip", &f.orig.dst)
        .set("hedef_port", &f.orig.dport)
        .set("nat_ip", &f.reply.dst)
        .set("nat_port", &f.reply.dport)
        .set("ek", format!("ct={}", f.id));
    if !f.new {
        r = r.set("gonderilen_bayt", &f.orig.bytes).set("alinan_bayt", &f.reply.bytes).set("sure_sn", &f.delta);
    }
    if let Some(s) = sess {
        r = r.set("telefon", &s.phone).set("oturum_id", &s.session_id);
    }
    r
}

// ---------------------------------------------------------------- kaydedici
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kaynak {
    Ct,
    Dns,
    Bosluk,
}

/// (günlük dosya adı, satır, oturum sahibi (mac, oturum))
pub type Item = (&'static str, Row, Option<(String, Session)>);

pub struct Kaydedici {
    pub cfg: Config,
    allow: BTreeMap<String, String>,
    pub by_ip: HashMap<String, (String, Session)>,
    pub leases: BTreeMap<String, String>,
    sess_mtime: Option<SystemTime>,
    /// conntrack id → (NEW zamanı, NEW anındaki sahip): kapanış satırı bağlantıyı başlatan kişiye yazılır
    flow_start: HashMap<String, (f64, Option<(String, Session)>, String)>,
    user_copy: bool,
    last_disk_warn: f64,
    pub runner: Box<Runner>,
    /// WAN DHCP bekçisi: port görevleri dosyası ve /sys/class/net (testte geçici)
    pub ag_path: std::path::PathBuf,
    pub sys: std::path::PathBuf,
    wan_bad_since: Option<f64>,
    wan_last_fix: f64,
    uzak_last_fix: f64,
    /// Yönetim merkezi bağı (testte geçici) ve son eşitleme başlatma zamanı
    pub merkez_path: std::path::PathBuf,
    merkez_son: f64,
    /// son bilinen hizmet durumu (lisans); değişince misafir oturumları kapanır / denetime yazılır
    lisans_acik: Option<bool>,
    /// Trafik ölçümü ve yavaşlatma (5 sn); yollar ve nft yükleyicisi testte değiştirilir
    olcer: trafik::Olcer,
    gun_son: f64,
    /// son uzlaştırmanın hatası (trafik.json'a, panelde uyarı) ve zamanı
    hiz_hata: Option<String>,
    hiz_hata_zaman: f64,
    /// sayaç tablosu son turda var mıydı (yalnızca değişince günlüğe) ve son tc kök denetimi
    hiz_tablo_var: Option<bool>,
    kok_son: f64,
    pub trafik_yolu: PathBuf,
    pub uygulanan_yolu: PathBuf,
    pub nft_f: Box<dyn Fn(&str) -> bool + Send + Sync>,
    /// komut çıktısı (nft setleri, tc kökleri), süre sınırlı: asılı nft/tc 5651 döngüsünü durdurmasın (`None` = okunamadı); testte sahte
    pub capture: Box<dyn Fn(&[&str]) -> Option<String> + Send + Sync>,
    /// ayar dosyası (panel izinli listeyi buraya yazar); değişince yalnızca izinli liste yeniden okunur
    pub ayar_yolu: PathBuf,
    ayar_mtime: Option<SystemTime>,
}

impl Kaydedici {
    pub fn new(cfg: Config, runner: Box<Runner>) -> Kaydedici {
        let olcer = trafik::Olcer::new(trafik::gun_oku(&cfg.main.state_root)); // bugünkü MB yeniden başlatmada kaybolmasın
        let ayar_yolu = PathBuf::from(std::env::var("WFC_AYAR").unwrap_or_else(|_| crate::ayar::PATH.to_string()));
        Kaydedici {
            allow: Config::devices(&cfg.allow),
            cfg,
            by_ip: HashMap::new(),
            leases: BTreeMap::new(),
            sess_mtime: None,
            flow_start: HashMap::new(),
            user_copy: true,
            last_disk_warn: 0.0,
            runner,
            ag_path: crate::ag::Yollar::sistem().ag,
            sys: "/sys/class/net".into(),
            wan_bad_since: None,
            wan_last_fix: f64::NEG_INFINITY,
            uzak_last_fix: f64::NEG_INFINITY,
            merkez_path: crate::merkez::PATH.into(),
            merkez_son: f64::NEG_INFINITY,
            lisans_acik: None,
            olcer,
            gun_son: f64::NEG_INFINITY,
            hiz_hata: None,
            hiz_hata_zaman: f64::NEG_INFINITY,
            hiz_tablo_var: None,
            kok_son: f64::NEG_INFINITY,
            trafik_yolu: trafik::DURUM_YOLU.into(),
            uygulanan_yolu: hiz::UYGULANAN_YOLU.into(),
            nft_f: Box::new(hiz::nft_dosya),
            capture: Box::new(|c: &[&str]| ortak::capture_timeout(c, 5)),
            ayar_yolu: ayar_yolu.clone(),
            ayar_mtime: std::fs::metadata(&ayar_yolu).and_then(|m| m.modified()).ok(),
        }
    }

    /// DHCP kiraları her döngüde; oturumlar yalnızca dosya değişince yeniden okunur.
    pub fn refresh(&mut self) {
        self.reload_leases();
        self.reload_sessions_if_changed();
        self.reload_allow_if_changed();
    }

    /// Panel izinli cihaz ekleyip/kaldırınca kaydedici yeniden başlamaz: ayar dosyası değiştiyse yalnızca izinli liste
    /// yenilenir (yoksa yeni izinli cihazın hızı görünmez, yavaşlatması 5 sn içinde geri alınırdı). Okunamazsa eski liste kalır.
    fn reload_allow_if_changed(&mut self) {
        let mtime = std::fs::metadata(&self.ayar_yolu).and_then(|m| m.modified()).ok();
        if mtime == self.ayar_mtime {
            return;
        }
        self.ayar_mtime = mtime;
        match Config::load(&self.ayar_yolu.to_string_lossy()) {
            Ok(c) => {
                self.allow = Config::devices(&c.allow);
                self.cfg.allow = c.allow;
            }
            Err(e) => eprintln!("kaydedici: izinli liste yenilenemedi: {e}"),
        }
    }

    fn reload_leases(&mut self) {
        self.leases = ortak::parse_leases(&std::fs::read_to_string(&self.cfg.main.leases_file).unwrap_or_default());
    }

    fn reload_sessions_if_changed(&mut self) {
        let path = ortak::sessions_path(&self.cfg.main.state_root);
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime != self.sess_mtime {
            self.sess_mtime = mtime;
            self.by_ip = ortak::load_sessions(&self.cfg.main.state_root)
                .into_iter()
                .filter(|(_, s)| !s.ip.is_empty())
                .map(|(mac, s)| (s.ip.clone(), (mac, s)))
                .collect();
        }
    }

    /// IP'nin sahibi: oturum varsa ve DHCP o IP'yi başka cihaza vermemişse oturum; MAC oturumdan ya da kiradan.
    /// Bulunamazsa dosyalar o an yeniden okunur: son tazelemeden sonra açılan/taşınan oturum ilk olayını kaçırmasın.
    fn who(&mut self, ip: &str) -> (Option<(String, Session)>, String) {
        if !self.by_ip.contains_key(ip) {
            self.reload_sessions_if_changed();
        }
        if !self.leases.contains_key(ip) {
            self.reload_leases();
        }
        let owner = self.leases.get(ip);
        let sess = self.by_ip.get(ip).filter(|(mac, _)| owner.is_none_or(|o| o == mac)).cloned();
        let mac = sess.as_ref().map(|(m, _)| m.clone()).or_else(|| owner.cloned()).unwrap_or_default();
        (sess, mac)
    }

    /// NEW'de zamanı ve sahibi hatırlar. DESTROY'da: süre çekirdekten gelmediyse NEW'den hesaplanır; sahip NEW
    /// anındaki sahiptir (oturum bu arada başka IP'ye taşındıysa bile kapanış onu başlatana yazılır).
    fn owner_of(&mut self, f: &mut Flow) -> (Option<(String, Session)>, String) {
        if f.new {
            let who = self.who(&f.orig.src);
            if self.flow_start.len() >= MAX_OPEN_FLOWS {
                self.flow_start.clear(); // ponytail: nadir taşmada toptan temizlik; o akışların kapanışı o anki sahibe yazılır
            }
            self.flow_start.insert(f.id.clone(), (f.ts, who.0.clone(), who.1.clone()));
            return who;
        }
        match self.flow_start.remove(&f.id) {
            Some((start, sess, mac)) => {
                if f.delta.is_empty() {
                    f.delta = ((f.ts - start) as i64).to_string();
                }
                (sess, mac)
            }
            None => self.who(&f.orig.src), // NEW görülmedi (kaydedici sonradan başladı)
        }
    }

    pub fn handle(&mut self, kind: Kaynak, line: &str, t: f64, queue: usize, loop_delay: f64) -> Vec<Item> {
        let m = &self.cfg.main;
        match kind {
            Kaynak::Bosluk => vec![("denetim.csv", Row::new("LOG_BOSLUK", &ortak::now_iso(t)).set("ek", line), None)],
            Kaynak::Ct => {
                let Some(mut f) = parse_conntrack(line) else {
                    if line.contains("ENOBUFS") || line.contains("No buffer space") {
                        let ek = format!("kaynak=conntrack neden=ENOBUFS kuyruk={queue} dongu_gecikme={loop_delay:.0}sn");
                        return vec![("denetim.csv", Row::new("LOG_BOSLUK", &ortak::now_iso(t)).set("ek", ek), None)];
                    }
                    return vec![];
                };
                let ip = f.orig.src.clone();
                // router'a giden ya da router'a yönlendirilen (DNS :53, portal :8080) akış dışarı çıkmadı → trafik değil
                let router = &m.router_ip;
                if !ortak::in_subnet(&ip, &m.subnet) || [&ip, &f.orig.dst, &f.reply.src].contains(&router) {
                    return vec![];
                }
                let (sess, mac) = self.owner_of(&mut f);
                let mut row = flow_row(&f, sess.as_ref().map(|(_, s)| s), &mac);
                if sess.is_none() {
                    if let Some(name) = self.allow.get(&mac) {
                        row = row.clone().set("ek", format!("{} izinli={name}", row.get("ek")));
                    }
                }
                vec![("trafik.csv", row, sess)]
            }
            Kaynak::Dns => {
                let Some((qtype, name, ip)) = parse_dns(line) else { return vec![] };
                if !ortak::in_subnet(&ip, &m.subnet) {
                    return vec![];
                }
                let (sess, mac) = self.who(&ip);
                let zaman = ortak::now_iso(journal_epoch(line).unwrap_or(t));
                let mut row = Row::new("DNS", &zaman).set("mac", mac).set("ic_ip", &ip).set("protokol", qtype).set("alan_adi", name);
                if let Some((_, s)) = &sess {
                    row = row.set("telefon", &s.phone).set("oturum_id", &s.session_id);
                }
                vec![("dns.csv", row, sess)]
            }
        }
    }

    /// Günlük dosya satırın kendi zamanından seçilir (gece yarısı güvenli); dosya başına tek ekleme.
    pub fn write(&self, items: Vec<Item>) {
        let root = &self.cfg.main.log_root;
        let mut batches: BTreeMap<PathBuf, Vec<Row>> = BTreeMap::new();
        let mut per_user: BTreeMap<String, (String, Session, Vec<Row>)> = BTreeMap::new();
        for (name, row, sess) in items {
            if let Some((mac, s)) = sess.filter(|_| self.user_copy) {
                per_user.entry(s.session_id.clone()).or_insert_with(|| (mac, s, vec![])).2.push(row.clone());
            }
            batches.entry(ortak::day_file(root, ortak::day_of(row.get("zaman")), name)).or_default().push(row);
        }
        for (path, rows) in batches {
            if let Err(e) = ortak::append_rows(&path, &rows) {
                eprintln!("kaydedici: {} yazılamadı: {e}", path.display());
            }
        }
        for (mac, s, rows) in per_user.into_values() {
            if let Err(e) = ortak::append_user_rows(root, &s, &mac, &rows) {
                eprintln!("kaydedici: kişi dosyası yazılamadı: {e}");
            }
        }
    }

    /// IP → MAC: oturumlar + kirada IP'si olan izinli cihazlar (hız tablosu ve yavaşlatma için).
    fn ip_mac(&self) -> BTreeMap<String, String> {
        trafik::ip_mac(self.by_ip.iter().map(|(ip, (mac, _))| (ip, mac)), &self.allow, &self.leases)
    }

    /// 5 sn adımı: set sayaçlarından cihaz başına hız ve bugünkü toplam → trafik.json (son yavaşlatma hatasıyla).
    /// Günlük toplam dakikada bir diske (kaydedici/cihaz yeniden başlarsa bugünkü MB kaybolmasın).
    pub fn trafik_adimi(&mut self, now: f64, indir_json: &str, yukle_json: &str) -> trafik::Durum {
        let ip_mac = self.ip_mac();
        let zaman = ortak::now_iso(now);
        let (indir, yukle) = (trafik::parse_set(indir_json), trafik::parse_set(yukle_json));
        let mut d = self.olcer.ornek(now, ortak::day_of(&zaman), &indir, &yukle, &ip_mac);
        d.hiz_hata = self.hiz_hata.clone();
        if let Err(e) = trafik::yaz(&self.trafik_yolu, &d) {
            eprintln!("kaydedici: {} yazılamadı: {e}", self.trafik_yolu.display());
        }
        if now - self.gun_son >= 60.0 {
            self.gun_son = now;
            if let Err(e) = trafik::gun_yaz(&self.cfg.main.state_root, self.olcer.gun_toplam()) {
                eprintln!("kaydedici: günlük trafik toplamı yazılamadı: {e}");
            }
        }
        d
    }

    /// Yavaşlatma uzlaştırması (değişiklik yoksa komut çalışmaz); hata bir sonraki trafik.json'a yazılır.
    pub fn hiz_adimi(&mut self, wan: &str) -> Result<bool, String> {
        let ip_of: BTreeMap<String, String> = self.ip_mac().into_iter().map(|(ip, mac)| (mac, ip)).collect();
        let r = hiz::uygula(&self.cfg, wan, &ip_of, &*self.runner, &*self.nft_f, &self.uygulanan_yolu);
        let hata = r.as_ref().err().cloned();
        if hata.is_some() && hata != self.hiz_hata {
            eprintln!("kaydedici: {}", hata.as_deref().unwrap_or(""));
        }
        self.hiz_hata = hata;
        r
    }

    /// `inet wfc_hiz` yoksa (paket güncellemesi sonrası ilk çalışma) yalnızca o tablo yüklenir; diğer tablolara dokunulmaz.
    /// Yeni tablonun sınıf zinciri boş olduğundan parmak izi silinir: sonraki uzlaştırma kuralları yeniden yazar.
    /// Yüklenemezse `false`: o turda yavaşlatma adımı hiç çalışmaz (tc'ye dokunulmaz); günlüğe yalnızca durum değişince.
    fn hiz_tablosu(&mut self) -> bool {
        let c = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let var = (self.runner)(&c(&["nft", "list", "table", "inet", "wfc_hiz"])) || {
            let ok = (self.runner)(&c(&["nft", "-f", HIZ_NFT]));
            if ok {
                let _ = std::fs::remove_file(&self.uygulanan_yolu);
            }
            ok
        };
        if self.hiz_tablo_var != Some(var) {
            if !var {
                eprintln!("kaydedici: {HIZ_NFT} yüklenemedi; hız ölçümü ve yavaşlatma duruyor");
            } else if self.hiz_tablo_var.is_some() {
                eprintln!("kaydedici: {HIZ_NFT} yüklendi");
            }
            self.hiz_tablo_var = Some(var);
        }
        if !var {
            self.hiz_hata = Some("Yavaşlatma uygulanamadı: hız tablosu (hiz.nft) yüklenemedi".into());
            self.hiz_hata_zaman = f64::NEG_INFINITY; // tablo gelince hemen uzlaştır
        }
        var
    }

    /// tc kökleri dışarıdan gittiyse (arayüz yeniden kuruldu, elle silindi) plan izi silinir: sonraki uzlaştırma yeniden kurar.
    /// Plan her zaman HTB kökü kurduğundan iki arayüzde de `htb 1:` beklenir.
    fn kok_denetle(&mut self, wan: &str) {
        if !self.uygulanan_yolu.exists() {
            return;
        }
        let lan = self.cfg.main.iface.clone();
        // okunamayan (asılı/süresi dolan) tc yok sayılır: sağlam kökler boşuna yeniden kurulmasın
        if [lan.as_str(), wan].iter().any(|d| (self.capture)(&["tc", "qdisc", "show", "dev", d, "root"]).is_some_and(|o| !o.contains("htb 1:"))) {
            let _ = std::fs::remove_file(&self.uygulanan_yolu);
        }
    }

    /// Gerçek 5 sn turu: tablo → sayaçlar → (30 sn'de bir kök denetimi) → uzlaştırma → trafik.json. Uzlaştırma hata verdiyse
    /// en çok dakikada bir yeniden denenir (her denemede tc kökleri silinip kurulur; 5 sn'de bir kuyrukları boşaltmasın).
    fn hiz_turu(&mut self, now: f64) {
        let tablo = self.hiz_tablosu();
        // okunamayan set boş sayılır; ikisi birden boşsa ölçer okuma hatası sayar, geçmişe dokunmaz
        let set = |ad: &str| (self.capture)(&["nft", "-j", "list", "set", "inet", "wfc_hiz", ad]).unwrap_or_default();
        let (indir, yukle) = (set("indir"), set("yukle"));
        if tablo {
            let wan = crate::ag::load(&self.ag_path).wan;
            if now - self.kok_son >= 30.0 {
                self.kok_son = now;
                self.kok_denetle(&wan);
            }
            if self.hiz_hata.is_none() || now - self.hiz_hata_zaman >= 60.0 {
                if self.hiz_adimi(&wan).is_err() {
                    self.hiz_hata_zaman = now;
                }
            }
        }
        self.trafik_adimi(now, &indir, &yukle);
    }

    /// Süresi dolanı kapat; IP'si değişeni taşı (taşınamazsa kapat); IP'si başkasına verileni beklemeye al.
    pub fn maintain(&mut self, now: f64) {
        let (root, state) = (self.cfg.main.log_root.clone(), self.cfg.main.state_root.clone());
        let runner = &*self.runner;
        let mac_ip: BTreeMap<String, String> = self.leases.iter().map(|(ip, mac)| (mac.clone(), ip.clone())).collect();
        let Ok(_g) = ortak::state_lock(&state) else { return };
        let mut ses = ortak::load_sessions(&state);
        let mut changed = false;
        for mac in ses.keys().cloned().collect::<Vec<_>>() {
            let Some(s) = ses.get(&mac).cloned() else { continue }; // bu turda başka oturum için beklemeye alınmış olabilir
            let reason = if s.expires_epoch <= now {
                "sure_doldu"
            } else if mac_ip.get(&mac).is_some_and(|ip| *ip != s.ip) {
                changed = true;
                if ortak::move_session(&root, &mut ses, &mac, &mac_ip[&mac], now, runner) {
                    continue;
                }
                "ip_degisti"
            } else if !s.ip.is_empty() && self.leases.get(&s.ip).is_some_and(|owner| *owner != mac) {
                // telefon ayrıldı, IP başka cihazda: oturum IP'siz beklemeye (dönünce yeni IP'sine taşınır)
                let owner = self.leases[&s.ip].clone();
                ortak::park_ip_holders(&root, &mut ses, &owner, &s.ip, now, runner);
                changed = true;
                continue;
            } else {
                continue;
            };
            ortak::close_session(&root, &mut ses, &mac, reason, now, runner);
            changed = true;
        }
        if changed {
            if let Err(e) = ortak::save_sessions(&state, &ses) {
                eprintln!("kaydedici: oturum dosyası yazılamadı: {e}");
            }
        }
        drop(_g);
        self.check_disk(now);
        self.check_wan(now);
        self.check_uzak(now);
        self.check_merkez(now);
        self.check_lisans(now);
    }

    /// Uzak erişim açıkken tünel yoksa ya da sunucuyla 10 dk'dır el sıkışılmadıysa (açılışta ad çözülemedi, sunucu IP'si
    /// değişti…) tünel yeniden kurulur; en çok 10 dk'da bir.
    fn check_uzak(&mut self, now: f64) {
        if !self.cfg.uzak.enabled || now - self.uzak_last_fix < 600.0 {
            return;
        }
        let up = self.sys.join(crate::uzak::IFACE).exists();
        let fresh = crate::uzak::last_handshake(&ortak::capture(&["wg", "show", crate::uzak::IFACE, "latest-handshakes"])).is_some_and(|t| now - t < 600.0);
        if up && fresh {
            return;
        }
        if self.uzak_last_fix.is_infinite() && up {
            self.uzak_last_fix = now; // yeni açılmış olabilir: ilk el sıkışmaya süre tanı
            return;
        }
        self.uzak_last_fix = now;
        let ok = (self.runner)(&["systemctl".to_string(), "restart".to_string(), crate::uzak::UNIT.to_string()]);
        let row = Row::new("UZAK_YENIDEN", &ortak::now_iso(now)).set("ek", format!("arayuz={} sonuc={}", if up { "var" } else { "yok" }, if ok { "tamam" } else { "hata" }));
        ortak::audit(&self.cfg.main.log_root, row);
    }

    /// Lisans kapalıyken (bitiş günü geçti, askıya alındı ya da cihaz bağlı değil) açık misafir oturumları her turda
    /// kapanır: kapanış anında doğrulanmakta olan bir oturum sonradan açılsa da kalmaz. Portal zaten yenisini kabul etmez.
    /// Denetime yalnızca gerçek geçişte ya da oturum kapandığında yazılır (boş turlarda satır yok).
    fn check_lisans(&mut self, now: f64) {
        let acik = crate::merkez::hizmet_acik(&self.merkez_path, now);
        let onceki = self.lisans_acik;
        let root = self.cfg.main.log_root.clone();
        if acik {
            if onceki == Some(false) {
                ortak::audit(&root, Row::new("LISANS_ACIK", &ortak::now_iso(now)));
            }
            self.lisans_acik = Some(true);
            return;
        }
        let state = self.cfg.main.state_root.clone();
        let Ok(_g) = ortak::state_lock(&state) else { return }; // kilit alınamazsa sonraki turda yeniden
        let mut ses = ortak::load_sessions(&state);
        let macs: Vec<String> = ses.keys().cloned().collect();
        for mac in &macs {
            ortak::close_session(&root, &mut ses, mac, "lisans", now, &*self.runner);
        }
        if !macs.is_empty() {
            if let Err(e) = ortak::save_sessions(&state, &ses) {
                eprintln!("kaydedici: oturum dosyası yazılamadı: {e}");
            }
        }
        if !macs.is_empty() || onceki == Some(true) {
            ortak::audit(&root, Row::new("LISANS_KAPALI", &ortak::now_iso(now)).set("ek", format!("kapanan_oturum={}", macs.len())));
        }
        self.lisans_acik = Some(false);
    }

    /// Yönetim merkezi eşitlemesi: zamanı geldiyse (her gün 06:00 sonrası, hatada 30 dk sonra) arka planda
    /// `ctl merkez-eslesme` başlatılır (ağ çağrısı kaydediciyi bekletmesin). En çok dakikada bir.
    fn check_merkez(&mut self, now: f64) {
        if now - self.merkez_son < 60.0 {
            return;
        }
        let Some(m) = crate::merkez::oku(&self.merkez_path) else { return };
        if !crate::merkez::zamani_geldi(&m, now) {
            return;
        }
        self.merkez_son = now;
        let unit = format!("wfc-merkez-eslesme-{}", now as u64);
        let c: Vec<String> = ["systemd-run", "--collect", "--unit", &unit, "--on-active", "1s", "/usr/local/bin/wificorrect", "ctl", "merkez-eslesme"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        (self.runner)(&c);
    }

    /// İnternet portunda kablo takılıyken DHCP istemcisi (dhcpcd) 1 dk'dan uzun yoksa WAN birimi yeniden başlatılır
    /// (en çok 5 dk'da bir). dhcpcd ölürse IP kira sonunda düşer, cihaz internetini ve uzaktan yönetimini kaybeder.
    /// Kablo yoksa ya da dhcpcd var ama IP alamıyorsa (modem kapalı) dokunulmaz: dhcpcd zaten deniyor.
    fn check_wan(&mut self, now: f64) {
        let wan = crate::ag::load(&self.ag_path).wan;
        let carrier = std::fs::read_to_string(self.sys.join(&wan).join("carrier")).is_ok_and(|c| c.trim() == "1");
        let alive = (self.runner)(&["pgrep".to_string(), "-f".to_string(), format!("^dhcpcd: {wan} ")]);
        if !carrier || alive {
            self.wan_bad_since = None;
            return;
        }
        let since = *self.wan_bad_since.get_or_insert(now);
        if now - since < 60.0 || now - self.wan_last_fix < 300.0 {
            return;
        }
        self.wan_last_fix = now;
        let unit = crate::ag::wan_unit(&wan);
        let ok = (self.runner)(&["systemctl".to_string(), "restart".to_string(), unit.clone()]);
        let row = Row::new("WAN_DHCP_YENIDEN", &ortak::now_iso(now)).set("ek", format!("arayuz={wan} birim={unit} sonuc={}", if ok { "tamam" } else { "hata" }));
        ortak::audit(&self.cfg.main.log_root, row);
        eprintln!("kaydedici: {wan} DHCP istemcisi yoktu, {unit} yeniden başlatıldı");
    }

    /// %80'de saatte bir uyarı; %95'te kişi kopyaları durur, günlük yasal dosyalar yazılmaya devam eder.
    fn check_disk(&mut self, now: f64) {
        let Ok(path) = std::ffi::CString::new(self.cfg.main.log_root.as_str()) else { return };
        // SAFETY: statvfs yalnızca verilen yapıya yazar.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 || st.f_blocks == 0 {
            return;
        }
        let pct = 100 - (st.f_bavail as u64 * 100 / st.f_blocks as u64);
        self.user_copy = pct < 95;
        if pct >= 80 && now - self.last_disk_warn >= 3600.0 {
            self.last_disk_warn = now;
            let row = Row::new("DISK_UYARI", &ortak::now_iso(now)).set("ek", format!("doluluk={pct}"));
            ortak::audit(&self.cfg.main.log_root, row);
            eprintln!("kaydedici: kayıt diski doluluğu %{pct}");
        }
    }
}

// ---------------------------------------------------------------- çalıştırma
type Msg = (Kaynak, String, f64);

fn send(tx: &SyncSender<Msg>, depth: &AtomicUsize, msg: Msg) -> bool {
    depth.fetch_add(1, Ordering::Relaxed);
    tx.send(msg).is_ok()
}

fn pump(src: impl Read, kind: Kaynak, tx: &SyncSender<Msg>, depth: &AtomicUsize) {
    let mut r = BufReader::with_capacity(1 << 16, src);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                if !send(tx, depth, (kind, line, ortak::wall())) {
                    return;
                }
            }
        }
    }
}

/// Alt süreci çalıştırır, satırlarını kuyruğa atar; ölürse 2 sn sonra yeniden başlatır ve boşluğu kaydeder.
fn reader(kind: Kaynak, cmd: Vec<String>, tx: SyncSender<Msg>, depth: Arc<AtomicUsize>) {
    let name = if kind == Kaynak::Ct { "conntrack" } else { "dns" };
    loop {
        match Command::new(&cmd[0]).args(&cmd[1..]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let (out, err) = (child.stdout.take().expect("stdout"), child.stderr.take().expect("stderr"));
                // 64 KB'lık boru ani yükte dolup conntrack'i bekletmesin
                // SAFETY: geçerli boru tanıtıcısı.
                unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20) };
                let (tx2, d2) = (tx.clone(), depth.clone());
                let errs = std::thread::spawn(move || pump(err, kind, &tx2, &d2)); // ENOBUFS uyarısı stderr'den gelir
                pump(out, kind, &tx, &depth);
                let _ = child.kill();
                let _ = child.wait();
                let _ = errs.join();
            }
            Err(e) => eprintln!("kaydedici: {} çalıştırılamadı: {e}", cmd[0]),
        }
        let stopped = ortak::wall();
        std::thread::sleep(Duration::from_secs(2));
        let now = ortak::wall();
        let ek = format!("kaynak={name} baslangic={} bitis={}", ortak::now_iso(stopped), ortak::now_iso(now));
        if !send(&tx, &depth, (Kaynak::Bosluk, ek, now)) {
            return;
        }
    }
}

pub fn run(cfg: Config) -> ExitCode {
    if let Err(e) = ortak::mkdirs(std::path::Path::new(&cfg.main.log_root)) {
        eprintln!("kaydedici: {} oluşturulamadı: {e}", cfg.main.log_root);
        return ExitCode::from(1);
    }
    let ct = ct_cmd(&cfg.main.subnet);
    let mut k = Kaydedici::new(cfg, Box::new(|c: &[String]| ortak::run(c)));
    ortak::audit(&k.cfg.main.log_root, Row::new("SERVIS_BASLADI", &ortak::now_iso(ortak::wall())).set("ek", "kaydedici"));
    let (tx, rx) = sync_channel::<Msg>(QUEUE);
    let depth = Arc::new(AtomicUsize::new(0));
    for (kind, cmd) in [(Kaynak::Ct, ct), (Kaynak::Dns, dns_cmd())] {
        let (tx, depth) = (tx.clone(), depth.clone());
        std::thread::spawn(move || reader(kind, cmd, tx, depth));
    }
    drop(tx);
    eprintln!("kaydedici: başladı");
    let (mut next_maint, mut next_hiz, mut last_loop) = (0.0, 0.0, Instant::now());
    loop {
        k.refresh();
        let mut items = vec![];
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok((kind, line, t)) => {
                    let q = depth.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
                    items.extend(k.handle(kind, &line, t, q, last_loop.elapsed().as_secs_f64()));
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return ExitCode::from(1),
            }
        }
        let t0 = Instant::now();
        let n = items.len();
        k.write(items);
        let now = ortak::wall();
        if now >= next_maint {
            k.maintain(now);
            next_maint = now + 30.0;
        }
        if now >= next_hiz {
            k.hiz_turu(now); // ilk turda (açılışta) sayaç tablosu da denetlenir
            next_hiz = now + 5.0;
        }
        let took = t0.elapsed().as_secs_f64();
        if took > 3.0 {
            // teşhis: yavaş adım (disk kilidi, alt komut) sistem günlüğüne
            eprintln!("kaydedici: yavaş döngü {took:.1} sn, satır={n}, kuyruk={}", depth.load(Ordering::Relaxed));
        }
        last_loop = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    const NEW: &str = "[1790253007.123456]\t    [NEW] ipv4     2 tcp      6 120 SYN_SENT src=10.50.0.23 dst=142.250.187.110 sport=51234 dport=443 [UNREPLIED] src=142.250.187.110 dst=192.168.1.10 sport=443 dport=40123 id=3301452";
    const DESTROY: &str = "[1790253190.654321]\t[DESTROY] ipv4     2 tcp      6 src=10.50.0.23 dst=142.250.187.110 sport=51234 dport=443 packets=41 bytes=18344 src=142.250.187.110 dst=192.168.1.10 sport=443 dport=40123 packets=702 bytes=912331 [ASSURED] delta-time=183 id=3301452";
    const ICMP: &str = "[1790253200.000001]\t    [NEW] ipv4     2 icmp     1 30 src=10.50.0.23 dst=8.8.8.8 type=8 code=0 id=77 [UNREPLIED] src=8.8.8.8 dst=192.168.1.10 type=0 code=0 id=77 id=999";
    const V6: &str = "[1790253200.000002]\t    [NEW] ipv6     10 udp      17 30 src=fe80::1 dst=ff02::1 sport=5353 dport=5353 [UNREPLIED] src=ff02::1 dst=fe80::1 sport=5353 dport=5353 id=5";
    const DNS_EXTRA: &str = "Thu Sep 24 14:30:07 2026 daemon.info dnsmasq[2345]: 1734 10.50.0.23/53012 query[A] www.google.com from 10.50.0.23";
    const DNS_PLAIN: &str = "Thu Sep 24 14:30:07 2026 daemon.info dnsmasq[2345]: query[AAAA] örnek.com.tr from 10.50.0.24";
    const DNS_FWD: &str = "Thu Sep 24 14:30:07 2026 daemon.info dnsmasq[2345]: 1734 10.50.0.23/53012 forwarded www.google.com to 1.1.1.1";
    const DNS_JOURNAL: &str = "1790964763.902386 wificorrect dnsmasq[1240]: 33 10.50.0.23/54755 query[A] www.wikipedia.org from 10.50.0.23";
    const M1: &str = "aa:bb:cc:dd:ee:01";
    const M2: &str = "aa:bb:cc:dd:ee:02";

    fn sess(phone: &str, ip: &str, expires: f64, sid: &str) -> Session {
        Session {
            phone: phone.into(),
            ad: "A".into(),
            soyad: "B".into(),
            ip: ip.into(),
            session_id: sid.into(),
            start: ortak::now_iso(1000.0),
            start_epoch: 1000.0,
            expires_epoch: expires,
        }
    }

    fn setup() -> (Kaydedici, PathBuf, Arc<Mutex<bool>>) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("wfc-kayit-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        let text = format!(
            "[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\nleases_file = '{0}/leases'\n[[allow]]\nmac = 'aa:bb:cc:dd:ee:99'\nname = 'AP'\n",
            root.display()
        );
        let ok = Arc::new(Mutex::new(true));
        let ok2 = ok.clone();
        let mut k = Kaydedici::new(Config::parse(&text).unwrap(), Box::new(move |_c: &[String]| *ok2.lock().unwrap()));
        std::fs::create_dir_all(&root).unwrap();
        k.merkez_path = root.join("merkez.json"); // bağlı, lisansı açık cihaz
        crate::merkez::kaydet(&k.merkez_path, &crate::merkez::Merkez { numara: "4511643".into(), son_eslesme: 9e9, ..Default::default() }).unwrap();
        k.by_ip.insert("10.50.0.23".into(), (M1.into(), sess("5334553132", "10.50.0.23", 9e9, "s1")));
        k.trafik_yolu = root.join("trafik.json");
        k.uygulanan_yolu = root.join("hiz_uygulanan");
        k.nft_f = Box::new(|_: &str| true);
        k.capture = Box::new(|_: &[&str]| Some(String::new()));
        k.ayar_yolu = root.join("ayarlar.toml"); // yok: izinli liste yukarıdaki metinden
        k.leases = [("10.50.0.40", "aa:bb:cc:dd:ee:40"), ("10.50.0.2", "aa:bb:cc:dd:ee:99")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        (k, root, ok)
    }

    #[test]
    fn conntrack_new_destroy_icmp_v6_garbage() {
        let f = parse_conntrack(NEW).unwrap();
        assert!(f.new && f.proto == "tcp" && f.ts == 1790253007.123456);
        assert_eq!((f.orig.src.as_str(), f.orig.dst.as_str(), f.orig.sport.as_str(), f.orig.dport.as_str()), ("10.50.0.23", "142.250.187.110", "51234", "443"));
        assert_eq!((f.reply.dst.as_str(), f.reply.dport.as_str(), f.id.as_str()), ("192.168.1.10", "40123", "3301452"));
        let f = parse_conntrack(DESTROY).unwrap();
        assert!(!f.new && f.orig.bytes == "18344" && f.reply.bytes == "912331" && f.delta == "183");
        let f = parse_conntrack(ICMP).unwrap();
        assert!(f.proto == "icmp" && f.orig.dst == "8.8.8.8" && f.id == "999");
        assert!(parse_conntrack(V6).is_none());
        assert!(parse_conntrack("WARNING: We have hit ENOBUFS! We are losing events.").is_none());
        assert!(parse_conntrack("").is_none());
    }

    #[test]
    fn dns_lines() {
        let t = |a: &str, b: &str, c: &str| Some((a.to_string(), b.to_string(), c.to_string()));
        assert_eq!(parse_dns(DNS_EXTRA), t("A", "www.google.com", "10.50.0.23"));
        assert_eq!(parse_dns(DNS_PLAIN), t("AAAA", "örnek.com.tr", "10.50.0.24"));
        assert_eq!(parse_dns(DNS_JOURNAL), t("A", "www.wikipedia.org", "10.50.0.23"));
        assert_eq!(parse_dns(DNS_FWD), None);
        assert_eq!(parse_dns("1790964757.2 wificorrect dnsmasq-dhcp[1240]: DHCPACK(br-hotspot) 10.50.0.200 x"), None);
        assert_eq!(journal_epoch(DNS_JOURNAL), Some(1790964763.902386));
        assert_eq!(journal_epoch(DNS_EXTRA), None);
    }

    #[test]
    fn real_fixtures_parse() {
        let lines: Vec<&str> = include_str!("../tests/fixtures/conntrack.txt")
            .lines()
            .filter(|l| (l.contains("[NEW]") || l.contains("[DESTROY]")) && l.contains(" ipv4 "))
            .collect();
        assert!(lines.len() > 500);
        for l in lines {
            let f = parse_conntrack(l).unwrap_or_else(|| panic!("{l}"));
            assert!(!f.orig.dst.is_empty() && !f.reply.dst.is_empty(), "{l}");
            if !f.new {
                assert!(!f.orig.bytes.is_empty(), "nf_conntrack_acct kapalı mı? {l}");
            }
        }
        for text in [include_str!("../tests/fixtures/dnsmasq-openwrt.txt"), include_str!("../tests/fixtures/dnsmasq-debian.txt")] {
            let q: Vec<&str> = text.lines().filter(|l| l.contains("query[")).collect();
            assert!(!q.is_empty());
            for l in q {
                assert!(parse_dns(l).is_some(), "{l}");
            }
        }
    }

    #[test]
    fn conntrack_row_for_session_and_skips() {
        let (mut k, _r, _) = setup();
        let items = k.handle(Kaynak::Ct, NEW, 0.0, 0, 0.0);
        let (name, row, s) = &items[0];
        assert_eq!(*name, "trafik.csv");
        assert_eq!((row.get("olay"), row.get("telefon"), row.get("mac"), row.get("nat_port")), ("BAGLANTI_BASLA", "5334553132", M1, "40123"));
        assert_eq!(row.get("zaman"), ortak::now_iso(1790253007.123456));
        assert!(s.is_some());
        // router'ın kendi trafiği, müşteri ağı dışı ve router'a yönlendirilmiş akışlar trafik değildir
        assert!(k.handle(Kaynak::Ct, &NEW.replace("src=10.50.0.23 dst=142", "src=192.168.1.5 dst=142"), 0.0, 0, 0.0).is_empty());
        assert!(k.handle(Kaynak::Ct, &NEW.replace("dst=142.250.187.110 sport=51234", "dst=10.50.0.1 sport=51234"), 0.0, 0, 0.0).is_empty());
        let redirected = NEW.replace("[UNREPLIED] src=142.250.187.110", "[UNREPLIED] src=10.50.0.1");
        assert!(k.handle(Kaynak::Ct, &redirected, 0.0, 0, 0.0).is_empty());
    }

    #[test]
    fn unauthenticated_allowlisted_enobufs_gap_dns() {
        let (mut k, _r, _) = setup();
        let row = k.handle(Kaynak::Ct, &NEW.replace("10.50.0.23", "10.50.0.40"), 0.0, 0, 0.0).remove(0).1;
        assert_eq!((row.get("telefon"), row.get("mac")), ("", "aa:bb:cc:dd:ee:40"));
        let row = k.handle(Kaynak::Ct, &NEW.replace("10.50.0.23", "10.50.0.2"), 0.0, 0, 0.0).remove(0).1;
        assert_eq!(row.get("ek"), "ct=3301452 izinli=AP");
        let it = k.handle(Kaynak::Ct, "WARNING: We have hit ENOBUFS! We are losing events.", 5.0, 7, 1.2);
        assert_eq!((it[0].0, it[0].1.get("olay")), ("denetim.csv", "LOG_BOSLUK"));
        assert_eq!(it[0].1.get("ek"), "kaynak=conntrack neden=ENOBUFS kuyruk=7 dongu_gecikme=1sn");
        assert_eq!(k.handle(Kaynak::Bosluk, "kaynak=dns", 5.0, 0, 0.0)[0].1.get("olay"), "LOG_BOSLUK");
        let it = k.handle(Kaynak::Dns, DNS_JOURNAL, 1.0, 0, 0.0);
        assert_eq!((it[0].0, it[0].1.get("alan_adi"), it[0].1.get("telefon")), ("dns.csv", "www.wikipedia.org", "5334553132"));
        assert_eq!(it[0].1.get("zaman"), ortak::now_iso(1790964763.0)); // satırın kendi zamanı
        assert!(k.handle(Kaynak::Dns, DNS_FWD, 1.0, 0, 0.0).is_empty());
    }

    #[test]
    fn session_opened_after_last_refresh_is_still_attributed() {
        let (mut k, _r, _) = setup();
        k.by_ip.clear();
        k.refresh(); // oturum dosyası henüz yok
        let mut ses = ortak::Sessions::new();
        ses.insert(M2.into(), sess("5550000000", "10.50.0.55", 9e9, "s9")); // portal az önce oturum açtı
        ortak::save_sessions(&k.cfg.main.state_root, &ses).unwrap();
        std::fs::write(&k.cfg.main.leases_file, format!("1 {M2} 10.50.0.55 tel 01\n")).unwrap(); // dnsmasq az önce kiraladı
        let row = k.handle(Kaynak::Ct, &NEW.replace("10.50.0.23", "10.50.0.55"), 0.0, 0, 0.0).remove(0).1;
        assert_eq!((row.get("telefon"), row.get("mac")), ("5550000000", M2));
    }

    #[test]
    fn close_row_goes_to_session_that_opened_the_flow() {
        let (mut k, _r, _) = setup();
        k.handle(Kaynak::Ct, NEW, 0.0, 0, 0.0); // 10.50.0.23 → M1'in oturumu
        k.by_ip.clear(); // oturum bu arada başka IP'ye taşındı
        k.by_ip.insert("10.50.0.77".into(), (M1.into(), sess("5334553132", "10.50.0.77", 9e9, "s1")));
        let row = k.handle(Kaynak::Ct, DESTROY, 0.0, 0, 0.0).remove(0).1;
        assert_eq!((row.get("telefon"), row.get("mac"), row.get("oturum_id")), ("5334553132", M1, "s1"));
    }

    #[test]
    fn duration_from_new_when_kernel_has_none() {
        let (mut k, _r, _) = setup();
        k.handle(Kaynak::Ct, NEW, 0.0, 0, 0.0);
        let no_delta = DESTROY.replace(" delta-time=183", "");
        assert_eq!(k.handle(Kaynak::Ct, &no_delta, 0.0, 0, 0.0)[0].1.get("sure_sn"), "183");
        assert_eq!(k.handle(Kaynak::Ct, &no_delta, 0.0, 0, 0.0)[0].1.get("sure_sn"), ""); // NEW görülmedi
    }

    #[test]
    fn write_uses_row_time_and_user_copy() {
        let (mut k, root, _) = setup();
        let mut items = k.handle(Kaynak::Ct, NEW, 0.0, 0, 0.0); // 2026-09-24 gününe ait
        items.extend(k.handle(Kaynak::Dns, DNS_JOURNAL, 0.0, 0, 0.0)); // 2026-10-02
        k.write(items);
        let day1 = root.join("5651/gunluk/2026-09-24");
        assert!(std::fs::read_to_string(day1.join("trafik.csv")).unwrap().contains("BAGLANTI_BASLA"));
        let user = std::fs::read_to_string(day1.join("kullanicilar/5334553132.csv")).unwrap();
        assert!(user.contains(";KAYIT;") && user.contains(";BAGLANTI_BASLA;"));
        assert!(std::fs::read_to_string(root.join("5651/gunluk/2026-10-02/dns.csv")).unwrap().contains("www.wikipedia.org"));
    }

    #[test]
    fn maintain_expires_moves_and_closes_on_failure() {
        let (mut k, root, ok) = setup();
        let state = k.cfg.main.state_root.clone();
        let mut ses = ortak::Sessions::new();
        ses.insert(M1.into(), sess("5334553132", "10.50.0.23", 2000.0, "s1"));
        ses.insert(M2.into(), sess("5550000000", "10.50.0.24", 9e9, "s2"));
        ortak::save_sessions(&state, &ses).unwrap();
        k.leases = [("10.50.0.77".to_string(), M2.to_string())].into();
        k.maintain(2500.0);
        let ses = ortak::load_sessions(&state);
        assert_eq!(ses.keys().collect::<Vec<_>>(), vec![M2]);
        assert_eq!(ses[M2].ip, "10.50.0.77"); // IP değişince oturum taşınır, kapanmaz
        let d = ortak::day_of(&ortak::now_iso(2000.0)).to_string();
        let o = std::fs::read_to_string(root.join(format!("5651/gunluk/{d}/oturum.csv"))).unwrap();
        assert!(o.contains(&format!("{};OTURUM_BITIS;5334553132", ortak::now_iso(2000.0))) && o.contains("neden=sure_doldu"));
        assert!(o.contains("OTURUM_IP_DEGISTI") && o.contains("eski_ip=10.50.0.24"));
        k.leases = [("10.50.0.78".to_string(), M2.to_string())].into();
        *ok.lock().unwrap() = false; // taşınamazsa (nft hatası) kapanır
        k.maintain(2600.0);
        assert!(ortak::load_sessions(&state).is_empty());
    }

    #[test]
    fn ip_reused_by_other_device_is_not_attributed_and_parked() {
        let (mut k, _r, _) = setup();
        let state = k.cfg.main.state_root.clone();
        let mut ses = ortak::Sessions::new();
        ses.insert(M1.into(), sess("5334553132", "10.50.0.23", 9e9, "s1"));
        ortak::save_sessions(&state, &ses).unwrap();
        let other = "1c:1b:0d:09:ee:e4";
        k.leases = [("10.50.0.23".to_string(), other.to_string())].into();
        let (_, row, s) = k.handle(Kaynak::Ct, NEW, 0.0, 0, 0.0).remove(0);
        assert!(s.is_none());
        assert_eq!((row.get("telefon"), row.get("mac")), ("", other)); // oturum sahibine yazılmaz
        k.maintain(2000.0);
        assert_eq!(ortak::load_sessions(&state)[M1].ip, ""); // IP'siz beklemede
    }

    #[test]
    fn conntrack_command_filters_customer_subnet_in_kernel() {
        let c = ct_cmd("10.50.0.0/24");
        assert_eq!(&c[c.len() - 2..], ["-s", "10.50.0.0/24"]);
        assert!(c.contains(&"33554432".to_string()));
    }

    #[test]
    fn wan_dhcp_watchdog_restarts_dead_client() {
        let (mut k, root, _) = setup();
        let sys = root.join("sys");
        std::fs::create_dir_all(sys.join("enp3s0")).unwrap();
        std::fs::write(sys.join("enp3s0/carrier"), "1\n").unwrap();
        k.sys = sys.clone();
        k.ag_path = root.join("yok-ag.toml"); // varsayılan: WAN enp3s0
        let alive = Arc::new(Mutex::new(true));
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let (a2, c2) = (alive.clone(), calls.clone());
        k.runner = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            if c[0] == "pgrep" { *a2.lock().unwrap() } else { true }
        });
        let restarts = |calls: &Arc<Mutex<Vec<String>>>| calls.lock().unwrap().iter().filter(|x| *x == "systemctl restart ifup@enp3s0.service").count();
        k.check_wan(1000.0);
        assert_eq!(restarts(&calls), 0); // sağlam
        assert!(calls.lock().unwrap().iter().any(|x| x == "pgrep -f ^dhcpcd: enp3s0 "));
        *alive.lock().unwrap() = false;
        k.check_wan(1030.0);
        assert_eq!(restarts(&calls), 0); // 1 dk beklenir (anlık durum olabilir)
        k.check_wan(1095.0);
        assert_eq!(restarts(&calls), 1);
        k.check_wan(1125.0);
        k.check_wan(1300.0);
        assert_eq!(restarts(&calls), 1); // 5 dk'da bir en çok
        k.check_wan(1400.0);
        assert_eq!(restarts(&calls), 2);
        // kablo yoksa dokunulmaz
        std::fs::write(sys.join("enp3s0/carrier"), "0\n").unwrap();
        k.check_wan(2000.0);
        k.check_wan(2100.0);
        k.check_wan(2400.0);
        assert_eq!(restarts(&calls), 2);
        let audit = std::fs::read_to_string(root.join(format!("5651/gunluk/{}/denetim.csv", ortak::day_of(&ortak::now_iso(1095.0))))).unwrap();
        assert!(audit.contains("WAN_DHCP_YENIDEN") && audit.contains("arayuz=enp3s0 birim=ifup@enp3s0.service sonuc=tamam"));
    }

    #[test]
    fn merkez_eslesmesi_zamani_gelince_arka_planda() {
        let (mut k, root, _) = setup();
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        k.runner = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            true
        });
        k.merkez_path = root.join("merkez.json");
        crate::merkez::sil(&k.merkez_path);
        let n = |calls: &Arc<Mutex<Vec<String>>>| calls.lock().unwrap().iter().filter(|x| x.contains("ctl merkez-eslesme")).count();
        let gun = 1_791_072_000.0; // 2026-10-04 00:00 UTC
        let tr = |saat: f64| gun + (saat - 3.0) * 3600.0;
        k.check_merkez(tr(7.0));
        assert_eq!(n(&calls), 0); // bağ yok
        let m = crate::merkez::Merkez { numara: "4511643".into(), tuz: "t".into(), ozet: "o".into(), yineleme: 120_000, cihaz_anahtari: "k".into(), son_eslesme: tr(5.0), deneme: 0.0, ..Default::default() };
        std::fs::create_dir_all(&root).unwrap();
        crate::merkez::kaydet(&k.merkez_path, &m).unwrap();
        k.check_merkez(tr(5.5));
        assert_eq!(n(&calls), 0); // 06:00 olmadı
        k.check_merkez(tr(6.1));
        assert_eq!(n(&calls), 1);
        k.check_merkez(tr(6.11));
        assert_eq!(n(&calls), 1); // aynı dakikada ikinci kez başlatılmaz
    }


    #[test]
    fn lisans_kapaninca_misafir_oturumlari_kapanir() {
        let (mut k, root, _) = setup();
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let c2 = calls.clone();
        k.runner = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            true
        });
        k.merkez_path = root.join("merkez.json");
        std::fs::create_dir_all(root.join("state")).unwrap();
        let state = k.cfg.main.state_root.clone();
        let mut ses = ortak::Sessions::new();
        ses.insert(M1.into(), sess("5334553132", "10.50.0.23", 9e9, "s1"));
        ortak::save_sessions(&state, &ses).unwrap();
        let mut m = crate::merkez::Merkez { numara: "4511643".into(), tuz: "t".into(), ozet: "o".into(), yineleme: 120_000, cihaz_anahtari: "k".into(),
                                            son_eslesme: 0.0, deneme: 0.0, ..Default::default() };
        crate::merkez::kaydet(&k.merkez_path, &m).unwrap();
        k.check_lisans(1000.0);
        assert_eq!(ortak::load_sessions(&state).len(), 1); // lisans açık: dokunulmaz
        m.lisans = "askida".into();
        crate::merkez::kaydet(&k.merkez_path, &m).unwrap();
        k.check_lisans(1010.0);
        assert!(ortak::load_sessions(&state).is_empty());
        assert!(calls.lock().unwrap().iter().any(|c| c.starts_with("nft delete element")));
        let n = calls.lock().unwrap().len();
        k.check_lisans(1020.0);
        assert_eq!(calls.lock().unwrap().len(), n); // açık oturum yok: bir şey yapılmaz
        // yarış: lisans kapanırken doğrulanan misafir oturumu sonradan açıldı → bir sonraki turda o da kapanır
        let mut ses = ortak::load_sessions(&state);
        ses.insert(M1.into(), sess("5334553132", "10.50.0.23", 9e9, "s2"));
        ortak::save_sessions(&state, &ses).unwrap();
        k.check_lisans(1025.0);
        assert!(ortak::load_sessions(&state).is_empty());
        m.lisans = "aktif".into();
        crate::merkez::kaydet(&k.merkez_path, &m).unwrap();
        k.check_lisans(1030.0);
        let audit = std::fs::read_to_string(root.join(format!("5651/gunluk/{}/denetim.csv", ortak::day_of(&ortak::now_iso(1010.0))))).unwrap();
        assert!(audit.contains("LISANS_KAPALI") && audit.contains("LISANS_ACIK"));
        assert_eq!(audit.matches("LISANS_KAPALI").count(), 2); // geçişte + yarışta kapanan oturum; boş turlarda satır yok
    }

    /// `nft -j list set` biçiminde set çıktısı.
    fn set_json(ad: &str, ogeler: &[(&str, u64)]) -> String {
        let e: Vec<String> = ogeler.iter().map(|(ip, b)| format!(r#"{{"elem": {{"val": "{ip}", "counter": {{"packets": 1, "bytes": {b}}}}}}}"#)).collect();
        format!(r#"{{"nftables": [{{"metainfo": {{}}}}, {{"set": {{"family": "inet", "name": "{ad}", "table": "wfc_hiz", "elem": [{}]}}}}]}}"#, e.join(", "))
    }

    const T: f64 = 1_791_640_000.0;
    const M3: &str = "aa:bb:cc:dd:ee:03";

    #[test]
    fn trafik_adimi_json_yazar() {
        let (mut k, _root, _) = setup();
        k.allow.insert(M3.into(), "Kasa".into());
        k.leases.insert("10.50.0.30".into(), M3.into());
        // oturumu var ama DHCP IP'yi başka (izinsiz) cihaza vermiş: sayılmaz
        k.by_ip.insert("10.50.0.40".into(), (M2.into(), sess("5550000000", "10.50.0.40", 9e9, "s2")));
        let ipler = |ek: u64| set_json("indir", &[("10.50.0.23", 1000 + ek), ("10.50.0.30", 500 + ek / 2), ("10.50.0.40", 700 + ek), ("10.50.0.77", 9 + ek)]);
        k.trafik_adimi(T, &ipler(0), &set_json("yukle", &[("10.50.0.23", 100)]));
        let d = k.trafik_adimi(T + 10.0, &ipler(12_500_000), &set_json("yukle", &[("10.50.0.23", 100 + 1_250_000)]));
        let yazilan = crate::trafik::oku(&k.trafik_yolu).unwrap();
        assert_eq!(yazilan.cihazlar.keys().collect::<Vec<_>>(), vec![M1, M3]);
        assert_eq!(yazilan.zaman, T + 10.0);
        let c = &yazilan.cihazlar[M1];
        assert_eq!((c.ip.as_str(), c.indir_bps, c.yukle_bps), ("10.50.0.23", 10_000_000, 1_000_000));
        assert_eq!(c.bugun_bayt, 12_500_000 + 1_250_000);
        assert_eq!(yazilan.cihazlar[M3].indir_bps, 5_000_000);
        assert_eq!(d.cihazlar[M1], *c);
        assert_eq!(yazilan.hiz_hata, None);
        // günlük toplam ilk adımda ve sonra dakikada bir diske
        let gun = |k: &Kaydedici| crate::trafik::gun_oku(&k.cfg.main.state_root).bayt.get(M1).copied();
        assert_eq!(gun(&k), Some(0));
        k.trafik_adimi(T + 15.0, &ipler(13_000_000), &set_json("yukle", &[("10.50.0.23", 100 + 1_250_000)]));
        assert_eq!(gun(&k), Some(0));
        k.trafik_adimi(T + 61.0, &ipler(13_000_000), &set_json("yukle", &[("10.50.0.23", 100 + 1_250_000)]));
        assert_eq!(gun(&k), Some(13_000_000 + 1_250_000));
    }

    #[test]
    fn tc_hatasi_trafik_jsona_yazilir() {
        let (mut k, _root, _) = setup();
        k.runner = Box::new(|c: &[String]| c[0] != "tc" || c[2] == "del");
        let mut s = crate::hiz::Sinirlar::new();
        s.insert(M1.into(), crate::hiz::Sinir { hiz: 5, ad: "A B".into(), zaman: String::new(), kim: "y".into() });
        crate::hiz::kaydet(&k.cfg.main.state_root, &s).unwrap();
        let e = k.hiz_adimi("enp3s0").unwrap_err();
        assert!(e.starts_with("Yavaşlatma uygulanamadı: "), "{e}");
        let d = k.trafik_adimi(T, &set_json("indir", &[("10.50.0.23", 1)]), &set_json("yukle", &[]));
        assert_eq!(d.hiz_hata.as_deref(), Some(e.as_str()));
        assert_eq!(crate::trafik::oku(&k.trafik_yolu).unwrap().hiz_hata, Some(e));
        // düzelince uyarı kalkar; nft metni oturumdaki IP'yle
        let son = Arc::new(Mutex::new(String::new()));
        let s2 = son.clone();
        k.runner = Box::new(|_: &[String]| true);
        k.nft_f = Box::new(move |t: &str| {
            *s2.lock().unwrap() = t.to_string();
            true
        });
        assert_eq!(k.hiz_adimi("enp3s0"), Ok(true));
        assert!(son.lock().unwrap().contains("ip daddr 10.50.0.23 "));
        assert_eq!(k.trafik_adimi(T + 5.0, "", "").hiz_hata, None);
    }

    /// hiz_turu için: runner'da tablo var/yok ve tc çağrıları sayılır, tc kökü çıktısı ayarlanır.
    fn hiz_kur(k: &mut Kaydedici, root: &std::path::Path) -> (Arc<Mutex<bool>>, Arc<Mutex<Vec<String>>>, Arc<Mutex<String>>) {
        let mut s = crate::hiz::Sinirlar::new();
        s.insert(M1.into(), crate::hiz::Sinir { hiz: 5, ad: "A B".into(), zaman: String::new(), kim: "y".into() });
        crate::hiz::kaydet(&k.cfg.main.state_root, &s).unwrap();
        k.ag_path = root.join("yok-ag.toml"); // WAN enp3s0
        let (tablo, calls, kok) = (Arc::new(Mutex::new(true)), Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(String::new())));
        let (t2, c2, k2) = (tablo.clone(), calls.clone(), kok.clone());
        k.runner = Box::new(move |c: &[String]| {
            c2.lock().unwrap().push(c.join(" "));
            c[0] != "nft" || *t2.lock().unwrap()
        });
        k.capture = Box::new(move |c: &[&str]| Some(if c[0] == "tc" { k2.lock().unwrap().clone() } else { String::new() }));
        (tablo, calls, kok)
    }

    fn tc_sayisi(calls: &Arc<Mutex<Vec<String>>>) -> usize {
        calls.lock().unwrap().iter().filter(|c| c.starts_with("tc ")).count()
    }

    #[test]
    fn hiz_tablosu_yuklenemezse_tc_ye_dokunulmaz() {
        let (mut k, root, _) = setup();
        let (tablo, calls, _) = hiz_kur(&mut k, &root);
        *tablo.lock().unwrap() = false; // nft list table ve nft -f başarısız
        for i in 0..30 {
            k.hiz_turu(T + 5.0 * i as f64);
        }
        assert_eq!(tc_sayisi(&calls), 0);
        assert!(calls.lock().unwrap().iter().any(|c| c == "nft -f /etc/wificorrect/hiz.nft")); // yüklemeyi denemeye devam
        let d = crate::trafik::oku(&k.trafik_yolu).unwrap();
        assert!(d.hiz_hata.unwrap().contains("hiz.nft"));
        // tablo gelince beklemeden uzlaştırılır
        *tablo.lock().unwrap() = true;
        k.hiz_turu(T + 200.0);
        assert!(tc_sayisi(&calls) > 0);
        assert_eq!(crate::trafik::oku(&k.trafik_yolu).unwrap().hiz_hata, None);
    }

    #[test]
    fn tc_koku_gidince_yeniden_kurulur() {
        let (mut k, root, _) = setup();
        let (_, calls, kok) = hiz_kur(&mut k, &root);
        *kok.lock().unwrap() = "qdisc htb 1: root refcnt 2 r2q 10 default 0x1 direct_packets_stat 0".into();
        k.hiz_turu(T);
        let n = tc_sayisi(&calls);
        assert!(n > 0 && k.uygulanan_yolu.exists());
        k.hiz_turu(T + 5.0);
        k.hiz_turu(T + 30.0);
        assert_eq!(tc_sayisi(&calls), n); // kökler yerinde: komut yok
        *kok.lock().unwrap() = "qdisc fq_codel 0: root refcnt 2".into(); // köprü yeniden kuruldu
        k.hiz_turu(T + 35.0);
        assert_eq!(tc_sayisi(&calls), n); // denetim 30 sn'de bir
        k.hiz_turu(T + 61.0);
        assert!(tc_sayisi(&calls) > n);
    }

    #[test]
    fn yeni_izinli_cihaz_kaydediciye_yansir() {
        // panel izinli cihaz ekledi (ayarlar.toml + kira), kaydedici yeniden başlamadı; cihaz yavaşlatıldı
        let (mut k, root, _) = setup();
        let (_, _, kok) = hiz_kur(&mut k, &root);
        *kok.lock().unwrap() = "qdisc htb 1: root refcnt 2".into();
        k.capture = {
            let k2 = kok.clone();
            Box::new(move |c: &[&str]| {
                Some(match c[0] {
                    "tc" => k2.lock().unwrap().clone(),
                    _ => set_json(c[c.len() - 1], &[("10.50.0.30", 1000)]),
                })
            })
        };
        k.hiz_turu(T); // eski liste: M3 yok
        std::fs::write(&k.cfg.main.leases_file, format!("1 {M3} 10.50.0.30 kasa 01\n")).unwrap();
        let metin = format!(
            "[main]\nlog_root = '{0}/5651'\nstate_root = '{0}/state'\nleases_file = '{0}/leases'\n[[allow]]\nmac = 'aa:bb:cc:dd:ee:99'\nname = 'AP'\n[[allow]]\nmac = '{M3}'\nname = 'Kasa'\n",
            root.display()
        );
        std::fs::write(&k.ayar_yolu, metin).unwrap();
        let mut s = crate::hiz::oku(&k.cfg.main.state_root);
        s.insert(M3.into(), crate::hiz::Sinir { hiz: 2, ad: "İzinli cihaz · Kasa".into(), zaman: String::new(), kim: "y".into() });
        crate::hiz::kaydet(&k.cfg.main.state_root, &s).unwrap();
        for i in 1..4 {
            k.refresh();
            k.hiz_turu(T + 5.0 * i as f64);
        }
        let d = crate::trafik::oku(&k.trafik_yolu).unwrap();
        assert_eq!(d.cihazlar.get(M3).map(|c| c.ip.as_str()), Some("10.50.0.30"), "{:?}", d.cihazlar.keys());
        let iz = std::fs::read_to_string(&k.uygulanan_yolu).unwrap();
        assert!(iz.contains("ip daddr 10.50.0.30 ") && iz.contains("rate 2mbit"), "{iz}"); // sınır geri alınmadı
        // bozuk ayar dosyası: liste korunur
        std::fs::write(&k.ayar_yolu, "[[allow]\nbozuk").unwrap();
        k.refresh();
        k.hiz_turu(T + 30.0);
        assert!(std::fs::read_to_string(&k.uygulanan_yolu).unwrap().contains("ip daddr 10.50.0.30 "));
    }

    #[test]
    fn tc_koku_okunamazsa_plan_izi_kalir() {
        let (mut k, root, _) = setup();
        hiz_kur(&mut k, &root);
        std::fs::write(&k.uygulanan_yolu, "plan").unwrap();
        k.capture = Box::new(|_: &[&str]| None); // tc asıldı, süre doldu
        k.kok_denetle("enp3s0");
        assert!(k.uygulanan_yolu.exists());
    }
}
