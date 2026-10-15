//! AP "yan odası" (spec 2026-10-15-ap-kurulum, "Ek: yan oda"): fabrika adresli AP'ye (ör. 192.168.1.1, modemin ağıyla
//! aynı) ulaşmak için ana ağ ad alanına hiçbir adres/rota eklemeden, misafir köprüsüne bağlı ayrı bir ağ ad alanı.
//!
//! - Ad alanı adsızdır (`ip netns add` yok, bağlama/mount yok): onu açan süreç yaşadıkça vardır. Bir iş parçacığı
//!   `unshare(CLONE_NEWNET)` yapar; ana ad alanından `ip link add link <köprü> … netns <tid> type macvlan` ile arayüz
//!   doğrudan o ad alanında doğar. Süreç bitince (aracı durdurulunca, süre dolunca, çökünce, yeniden başlatmada) ad
//!   alanı ve macvlan çekirdekçe silinir.
//! - macvlan köprü portu değildir: yan odanın paketleri köprüden "yerel çıkış" olarak geçer, `kapi.nft`'teki köprü
//!   yalıtımına (`forward` kancası) takılmaz (veth köprü portu takılır; cihazda geçici ad alanlarında denendi).
//! - Yan odada iletim kapalı (`ip_forward=0`), IPv6 kapalı, yalnızca macvlan arayüzü var: misafir trafiği oradan geçmez.
//! - SOCKS5 aracı (`ctl ap-araci`): yalnızca 127.0.0.1'de dinler, bağlantıları yan odadan kurar (her bağlantının iş
//!   parçacığı `setns` ile yan odaya girer), yalnızca yan odanın alt ağına; CONNECT, kimlik doğrulamasız, IPv4.
//! - Yabancı DHCP denetimi (`ctl ap-dhcp-denetle`): kısa ömürlü ayrı bir yan odadan DHCPDISCOVER, gelen teklifler
//!   (AF_PACKET; 10.50.0.1'in kendi dnsmasq'ı bu yayını almaz, köprüden yerel çıkış olarak gider).

use crate::ap::Cidr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpListener, TcpStream};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Yan odadaki macvlan arayüzü (aracı) ve DHCP denetiminin arayüzü
pub const ARAYUZ: &str = "wfcap0";
pub const DHCP_ARAYUZ: &str = "wfcdhcp0";
/// Aracının dinlediği yerel port (127.0.0.1)
pub const ARACI_PORT: u16 = 1085;
/// Aracının geçici systemd birimi (tek yan oda)
pub const ARACI_BIRIM: &str = "wfc-ap-araci";
/// Aynı anda en çok bağlantı, el sıkışma / bağlanma / boşta bekleme süreleri
const EN_COK_BAGLANTI: usize = 32;
const EL_SIKISMA: Duration = Duration::from_secs(10);
const BAGLANMA: Duration = Duration::from_secs(5);
const BOSTA: Duration = Duration::from_secs(300);

fn s(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Yerel yönetilen, tek yayınlı rastgele MAC
pub fn rastgele_mac() -> [u8; 6] {
    let mut b = [0u8; 6];
    getrandom::getrandom(&mut b).expect("rastgele sayı üretilemedi");
    b[0] = (b[0] & 0xfc) | 0x02;
    b
}

pub fn mac_yaz(m: &[u8]) -> String {
    m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// Ana ad alanında çalışır: macvlan arayüzü doğrudan `tid` iş parçacığının ad alanında doğar (ana ad alanında hiç
/// görünmez).
pub fn macvlan_komutu(kopru: &str, arayuz: &str, mac: &[u8; 6], tid: i64) -> Vec<String> {
    s(&["ip", "link", "add", "link", kopru, "name", arayuz, "address", &mac_yaz(mac), "netns", &tid.to_string(), "type", "macvlan", "mode", "private"])
}

/// Yan odanın içinde çalışan komutlar (adres yalnızca aracıda; DHCP denetimi adressiz)
pub fn ic_komutlar(arayuz: &str, adres: Option<&Cidr>) -> Vec<Vec<String>> {
    let mut v = vec![];
    if let Some(c) = adres {
        v.push(s(&["ip", "addr", "add", &c.to_string(), "dev", arayuz]));
    }
    v.push(s(&["ip", "link", "set", arayuz, "up"]));
    v
}

/// Bu iş parçacığını verilen ağ ad alanına sokar (yalnızca bu iş parçacığı; süreç ana ad alanında kalır).
pub fn gir(ns: &File) -> io::Result<()> {
    // SAFETY: geçerli dosya tanıtıcısı; setns yalnızca çağıran iş parçacığını etkiler.
    if unsafe { libc::setns(ns.as_raw_fd(), libc::CLONE_NEWNET) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Yan odayı açar ve ad alanının tanıtıcısını döner (tanıtıcı ve süreç yaşadıkça ad alanı yaşar). Root gerekir;
/// testlerde çalıştırılmaz (cihazda geçici ad alanlarıyla denendi).
pub fn ac(kopru: &str, arayuz: &str, adres: Option<Cidr>, mac: [u8; 6]) -> Result<File, String> {
    let (tx, rx) = mpsc::channel::<Result<(i64, File), String>>();
    let (git_tx, git_rx) = mpsc::channel::<bool>();
    let arayuz_ic = arayuz.to_string();
    let is = std::thread::spawn(move || -> Result<(), String> {
        // SAFETY: yalnızca bu iş parçacığının ağ ad alanı değişir.
        if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
            let e = format!("ağ ad alanı açılamadı: {}", io::Error::last_os_error());
            let _ = tx.send(Err(e.clone()));
            return Err(e);
        }
        // SAFETY: argümansız sistem çağrısı
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as i64;
        let ns = match File::open("/proc/thread-self/ns/net") {
            Ok(f) => f,
            Err(e) => {
                let e = format!("ağ ad alanı okunamadı: {e}");
                let _ = tx.send(Err(e.clone()));
                return Err(e);
            }
        };
        // macvlan doğmadan: IPv6 kapalı (yeni arayüz `default`tan alır); iletim kapalı (yeni ad alanı ana ad alanının
        // ip_forward=1 değerini miras alır). /proc/sys/net bu iş parçacığının ad alanını gösterir.
        for f in ["/proc/sys/net/ipv6/conf/all/disable_ipv6", "/proc/sys/net/ipv6/conf/default/disable_ipv6"] {
            let _ = std::fs::write(f, "1"); // IPv6 çekirdekte kapalıysa dosya yok
        }
        if let Err(e) = std::fs::write("/proc/sys/net/ipv4/ip_forward", "0") {
            let e = format!("yan odada iletim kapatılamadı: {e}");
            let _ = tx.send(Err(e.clone()));
            return Err(e);
        }
        let _ = tx.send(Ok((tid, ns)));
        if !git_rx.recv().unwrap_or(false) {
            return Ok(()); // macvlan eklenemedi; hata ana iş parçacığında
        }
        // bu iş parçacığından başlatılan komutlar yan odada çalışır
        for c in ic_komutlar(&arayuz_ic, adres.as_ref()) {
            if !crate::ortak::run(&c) {
                return Err(format!("yan odada komut başarısız: {}", c.join(" ")));
            }
        }
        Ok(())
    });
    let ilk = rx.recv().map_err(|_| "yan oda iş parçacığı çöktü".to_string());
    let (tid, ns) = match ilk {
        Ok(Ok(v)) => v,
        Ok(Err(e)) | Err(e) => {
            let _ = is.join();
            return Err(e);
        }
    };
    let ok = crate::ortak::run(&macvlan_komutu(kopru, arayuz, &mac, tid));
    let _ = git_tx.send(ok);
    let sonuc = is.join().map_err(|_| "yan oda iş parçacığı çöktü".to_string())?;
    if !ok {
        return Err(format!("{kopru} üzerinde macvlan arayüzü eklenemedi (ip link add)"));
    }
    sonuc?;
    Ok(ns)
}

// ---------------------------------------------------------------- SOCKS5 aracı
/// SOCKS5 cevap kodları (RFC 1928)
pub const RET_KURAL: u8 = 0x02;
pub const RET_KONAK: u8 = 0x04;
pub const RET_RED: u8 = 0x05;
pub const RET_SURE: u8 = 0x06;
pub const RET_KOMUT: u8 = 0x07;
pub const RET_ADRES_TURU: u8 = 0x08;

/// Hedef adres: IPv4 (ATYP 1) ya da IPv4 yazılmış alan adı (ATYP 3; Chrome IP adresini de böyle gönderir). Ad çözümü
/// yok (yan odada DNS yok); IPv6 yok. Yalnızca `ag` alt ağında, ağ/yayın adresi değil, port 0 değil.
pub fn hedef_dogrula(atyp: u8, adres: &[u8], port: u16, ag: &Cidr) -> Result<SocketAddrV4, u8> {
    let ip = match atyp {
        1 if adres.len() == 4 => Ipv4Addr::new(adres[0], adres[1], adres[2], adres[3]),
        3 => std::str::from_utf8(adres).ok().and_then(|a| a.parse().ok()).ok_or(RET_KONAK)?,
        _ => return Err(RET_ADRES_TURU),
    };
    let c = Cidr { ip, onek: ag.onek };
    if !c.cakisir(ag) || ip == c.ag() || ip == c.yayin() || port == 0 {
        return Err(RET_KURAL);
    }
    Ok(SocketAddrV4::new(ip, port))
}

fn oku_n(s: &mut TcpStream, n: usize) -> io::Result<Vec<u8>> {
    let mut b = vec![0u8; n];
    s.read_exact(&mut b)?;
    Ok(b)
}

fn cevap(kod: u8) -> [u8; 10] {
    [5, kod, 0, 1, 0, 0, 0, 0, 0, 0]
}

pub type Baglan = dyn Fn(SocketAddrV4) -> io::Result<TcpStream> + Send + Sync;

/// Tek SOCKS5 istemcisi: el sıkışma (yalnızca "kimlik doğrulamasız"), CONNECT, hedef denetimi, bağlan, aktar.
pub fn istemci(mut s: TcpStream, ag: &Cidr, baglan: &Baglan) -> io::Result<()> {
    s.set_read_timeout(Some(EL_SIKISMA))?;
    s.set_write_timeout(Some(EL_SIKISMA))?;
    let b = oku_n(&mut s, 2)?;
    if b[0] != 5 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "SOCKS5 değil"));
    }
    let yontemler = oku_n(&mut s, usize::from(b[1]))?;
    if !yontemler.contains(&0) {
        return s.write_all(&[5, 0xff]);
    }
    s.write_all(&[5, 0])?;
    let b = oku_n(&mut s, 4)?; // VER CMD RSV ATYP
    let adres = match b[3] {
        1 => oku_n(&mut s, 4)?,
        3 => {
            let n = oku_n(&mut s, 1)?[0];
            oku_n(&mut s, usize::from(n))?
        }
        4 => oku_n(&mut s, 16)?,
        _ => return s.write_all(&cevap(RET_ADRES_TURU)),
    };
    let p = oku_n(&mut s, 2)?;
    let port = u16::from_be_bytes([p[0], p[1]]);
    if b[0] != 5 || b[1] != 1 {
        return s.write_all(&cevap(RET_KOMUT));
    }
    let hedef = match hedef_dogrula(b[3], &adres, port, ag) {
        Ok(h) => h,
        Err(k) => return s.write_all(&cevap(k)),
    };
    let u = match baglan(hedef) {
        Ok(u) => u,
        Err(e) => {
            let k = match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => RET_SURE,
                io::ErrorKind::ConnectionRefused => RET_RED,
                _ => RET_KONAK,
            };
            return s.write_all(&cevap(k));
        }
    };
    s.write_all(&cevap(0))?;
    aktar(s, u)
}

fn kopya(mut r: TcpStream, mut w: TcpStream) {
    let _ = io::copy(&mut r, &mut w);
    let _ = w.shutdown(Shutdown::Write);
    let _ = r.shutdown(Shutdown::Read);
}

/// İki yönlü aktarım; bir yön `BOSTA` süre sessiz kalırsa o yön kapanır.
fn aktar(a: TcpStream, b: TcpStream) -> io::Result<()> {
    for x in [&a, &b] {
        x.set_read_timeout(Some(BOSTA))?;
        x.set_write_timeout(Some(BOSTA))?;
    }
    let (a2, b2) = (a.try_clone()?, b.try_clone()?);
    let t = std::thread::Builder::new().spawn(move || kopya(a2, b2))?;
    kopya(b, a);
    let _ = t.join();
    Ok(())
}

struct Sayac(Arc<AtomicUsize>);

impl Drop for Sayac {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Dinleyiciden gelen bağlantıları sırayla kabul eder; en çok `EN_COK_BAGLANTI` (fazlası hemen kapatılır).
pub fn sun(dinle: TcpListener, ag: Cidr, baglan: Arc<Baglan>) {
    let acik = Arc::new(AtomicUsize::new(0));
    for s in dinle.incoming() {
        let Ok(s) = s else { continue };
        if acik.fetch_add(1, Ordering::SeqCst) >= EN_COK_BAGLANTI {
            acik.fetch_sub(1, Ordering::SeqCst);
            continue; // s düşer → kapanır
        }
        let sayac = Sayac(acik.clone());
        let baglan = baglan.clone();
        let _ = std::thread::Builder::new().spawn(move || {
            let _sayac = sayac;
            let _ = istemci(s, &ag, &*baglan);
        }); // iş parçacığı açılamazsa kapanış sayacı düşürür
    }
}

/// systemd'ye "hazır" (Type=notify): `systemd-run` kurulum bitene kadar bekler, hata olursa başarısız döner.
fn hazir_bildir() {
    use std::os::unix::net::UnixDatagram;
    let Ok(yol) = std::env::var("NOTIFY_SOCKET") else { return };
    let Ok(s) = UnixDatagram::unbound() else { return };
    if let Some(ad) = yol.strip_prefix('@') {
        use std::os::linux::net::SocketAddrExt;
        if let Ok(a) = std::os::unix::net::SocketAddr::from_abstract_name(ad.as_bytes()) {
            let _ = s.send_to_addr(b"READY=1", &a);
        }
    } else {
        let _ = s.send_to(b"READY=1", &yol);
    }
}

/// `ctl ap-araci ADRES` (geçici systemd birimi): 127.0.0.1:`ARACI_PORT` dinlenir, yan oda `adres` ile açılır,
/// hazır bildirilir; süreç durdurulana kadar hizmet verir (durunca yan oda kendiliğinden kalkar).
pub fn araci_calistir(kopru: &str, adres: Cidr) -> Result<(), String> {
    let dinle = TcpListener::bind((Ipv4Addr::LOCALHOST, ARACI_PORT)).map_err(|e| format!("127.0.0.1:{ARACI_PORT} dinlenemedi: {e}"))?;
    let ns = Arc::new(ac(kopru, ARAYUZ, Some(adres), rastgele_mac())?);
    hazir_bildir();
    eprintln!("yan oda açık: {adres} ({kopru} üzerinde macvlan), aracı 127.0.0.1:{ARACI_PORT}");
    let baglan: Arc<Baglan> = Arc::new(move |h: SocketAddrV4| {
        gir(&ns)?; // bağlantının kendi iş parçacığı
        TcpStream::connect_timeout(&h.into(), BAGLANMA)
    });
    sun(dinle, adres, baglan);
    Ok(())
}

// ---------------------------------------------------------------- yabancı DHCP denetimi
fn toplam(b: &[u8]) -> u16 {
    let mut t: u32 = b.chunks(2).map(|c| u32::from(u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]))).sum();
    while t > 0xffff {
        t = (t & 0xffff) + (t >> 16);
    }
    !(t as u16)
}

/// DHCPDISCOVER (IPv4 başlığından başlayarak; AF_PACKET SOCK_DGRAM ile gönderilir): 0.0.0.0:68 → 255.255.255.255:67,
/// yayın bayrağı açık (sunucu teklifi yayınla gönderir).
pub fn kesif(mac: &[u8; 6], xid: u32) -> Vec<u8> {
    let mut d = vec![0u8; 236];
    d[0] = 1; // BOOTREQUEST
    d[1] = 1; // Ethernet
    d[2] = 6;
    d[4..8].copy_from_slice(&xid.to_be_bytes());
    d[10] = 0x80; // yayın bayrağı
    d[28..34].copy_from_slice(mac);
    d.extend([99, 130, 83, 99, 53, 1, 1, 55, 3, 1, 3, 6, 255]);
    d.resize(300, 0);
    let udp_boy = 8 + d.len();
    let mut p = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, 17, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255];
    p[2..4].copy_from_slice(&((20 + udp_boy) as u16).to_be_bytes());
    let t = toplam(&p);
    p[10..12].copy_from_slice(&t.to_be_bytes());
    p.extend(68u16.to_be_bytes());
    p.extend(67u16.to_be_bytes());
    p.extend((udp_boy as u16).to_be_bytes());
    p.extend([0, 0]); // UDP sağlama toplamı isteğe bağlı
    p.extend(d);
    p
}

/// IPv4 paketi bu `xid`'e DHCPOFFER ise (sunucu kimliği ya da kaynak IP, teklif edilen adres).
pub fn teklif_coz(p: &[u8], xid: u32) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if p.len() < 20 || p[0] >> 4 != 4 || p[9] != 17 {
        return None;
    }
    let ihl = usize::from(p[0] & 0x0f) * 4;
    let u = p.get(ihl..)?;
    if u.len() < 8 || u16::from_be_bytes([u[0], u[1]]) != 67 || u16::from_be_bytes([u[2], u[3]]) != 68 {
        return None;
    }
    let d = &u[8..];
    if d.len() < 240 || d[0] != 2 || d[4..8] != xid.to_be_bytes() || d[236..240] != [99, 130, 83, 99] {
        return None;
    }
    let (mut tur, mut sunucu) = (None, None);
    let mut i = 240;
    while let Some(&kod) = d.get(i) {
        match kod {
            0 => i += 1,
            255 => break,
            _ => {
                let n = usize::from(*d.get(i + 1)?);
                let v = d.get(i + 2..i + 2 + n)?;
                match (kod, n) {
                    (53, 1) => tur = Some(v[0]),
                    (54, 4) => sunucu = Some(Ipv4Addr::new(v[0], v[1], v[2], v[3])),
                    _ => {}
                }
                i += 2 + n;
            }
        }
    }
    (tur == Some(2)).then(|| (sunucu.unwrap_or(Ipv4Addr::new(p[12], p[13], p[14], p[15])), Ipv4Addr::new(d[16], d[17], d[18], d[19])))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Teklif {
    pub sunucu: Ipv4Addr,
    pub mac: String,
    pub adres: Ipv4Addr,
}

/// Yan odanın içinde (çağıran iş parçacığı `gir` yapmış olmalı): `arayuz`dan iki kez DISCOVER, `sure` boyunca teklifler.
fn yokla(arayuz: &str, mac: &[u8; 6], sure: Duration) -> Result<Vec<Teklif>, String> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let ad = std::ffi::CString::new(arayuz).map_err(|e| e.to_string())?;
    // SAFETY: geçerli C dizgisi
    let ifindex = unsafe { libc::if_nametoindex(ad.as_ptr()) };
    if ifindex == 0 {
        return Err(format!("{arayuz} bulunamadı"));
    }
    let proto = (libc::ETH_P_IP as u16).to_be();
    // SAFETY: sistem çağrısı; dönen tanıtıcı OwnedFd'ye geçer
    let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, i32::from(proto)) };
    if fd < 0 {
        return Err(format!("paket soketi açılamadı: {}", io::Error::last_os_error()));
    }
    // SAFETY: az önce açılan, başka sahibi olmayan tanıtıcı
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let raw = fd.as_raw_fd();
    // SAFETY: sıfırlanmış düz C yapısı
    let mut sa: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
    sa.sll_family = libc::AF_PACKET as u16;
    sa.sll_protocol = proto;
    sa.sll_ifindex = ifindex as i32;
    let boy = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
    // SAFETY: sa geçerli sockaddr_ll, boyu doğru
    if unsafe { libc::bind(raw, &sa as *const _ as *const libc::sockaddr, boy) } != 0 {
        return Err(format!("paket soketi bağlanamadı: {}", io::Error::last_os_error()));
    }
    let tv = libc::timeval { tv_sec: 0, tv_usec: 200_000 };
    // SAFETY: tv geçerli timeval
    unsafe { libc::setsockopt(raw, libc::SOL_SOCKET, libc::SO_RCVTIMEO, &tv as *const _ as *const libc::c_void, std::mem::size_of::<libc::timeval>() as libc::socklen_t) };
    let mut x = [0u8; 4];
    getrandom::getrandom(&mut x).map_err(|e| e.to_string())?;
    let xid = u32::from_be_bytes(x);
    let paket = kesif(mac, xid);
    let mut hedef = sa;
    hedef.sll_halen = 6;
    hedef.sll_addr[..6].copy_from_slice(&[0xff; 6]);
    let gonder = || {
        // SAFETY: paket ve hedef geçerli
        unsafe { libc::sendto(raw, paket.as_ptr() as *const libc::c_void, paket.len(), 0, &hedef as *const _ as *const libc::sockaddr, boy) }
    };
    if gonder() < 0 {
        return Err(format!("DHCP keşfi gönderilemedi: {}", io::Error::last_os_error()));
    }
    let basla = Instant::now();
    let mut ikinci = false;
    let mut v: Vec<Teklif> = vec![];
    let mut buf = [0u8; 2048];
    while basla.elapsed() < sure {
        if !ikinci && basla.elapsed() >= Duration::from_secs(1) {
            ikinci = true;
            gonder();
        }
        // SAFETY: sıfırlanmış düz C yapısı
        let mut kaynak: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut kboy = boy;
        // SAFETY: buf ve kaynak geçerli, boyları doğru
        let n = unsafe { libc::recvfrom(raw, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0, &mut kaynak as *mut _ as *mut libc::sockaddr, &mut kboy) };
        if n <= 0 || kaynak.sll_pkttype == libc::PACKET_OUTGOING as u8 {
            continue;
        }
        if let Some((sunucu, adres)) = teklif_coz(&buf[..n as usize], xid) {
            let t = Teklif { sunucu, mac: mac_yaz(&kaynak.sll_addr[..6]), adres };
            if !v.iter().any(|o| o.sunucu == t.sunucu && o.mac == t.mac) {
                v.push(t);
            }
        }
    }
    Ok(v)
}

/// `ctl ap-dhcp-denetle`: kısa ömürlü yan oda (adressiz macvlan) açar, ~3 sn teklif toplar; süreç bitince yan oda gider.
pub fn dhcp_denetle(kopru: &str) -> Result<Vec<Teklif>, String> {
    let mac = rastgele_mac();
    let ns = ac(kopru, DHCP_ARAYUZ, None, mac)?;
    std::thread::spawn(move || {
        gir(&ns).map_err(|e| format!("yan odaya girilemedi: {e}"))?;
        yokla(DHCP_ARAYUZ, &mac, Duration::from_secs(3))
    })
    .join()
    .map_err(|_| "DHCP denetimi çöktü".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ag() -> Cidr {
        Cidr::parse("192.168.1.250/24").unwrap()
    }

    #[test]
    fn komutlar_birebir() {
        let mac = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
        assert_eq!(macvlan_komutu("br-hotspot", ARAYUZ, &mac, 4242),
            s(&["ip", "link", "add", "link", "br-hotspot", "name", "wfcap0", "address", "02:aa:bb:cc:dd:ee", "netns", "4242", "type", "macvlan", "mode", "private"]));
        assert_eq!(ic_komutlar(ARAYUZ, Some(&ag())), vec![s(&["ip", "addr", "add", "192.168.1.250/24", "dev", "wfcap0"]), s(&["ip", "link", "set", "wfcap0", "up"])]);
        assert_eq!(ic_komutlar(DHCP_ARAYUZ, None), vec![s(&["ip", "link", "set", "wfcdhcp0", "up"])]);
        assert!(ARAYUZ.len() <= 15 && DHCP_ARAYUZ.len() <= 15);
        for _ in 0..20 {
            let m = rastgele_mac();
            assert_eq!(m[0] & 0x03, 0x02, "yerel yönetilen, tek yayınlı");
        }
    }

    #[test]
    fn hedef_kurallari() {
        let a = ag();
        assert_eq!(hedef_dogrula(1, &[192, 168, 1, 1], 80, &a), Ok("192.168.1.1:80".parse().unwrap()));
        assert_eq!(hedef_dogrula(3, b"192.168.1.1", 443, &a), Ok("192.168.1.1:443".parse().unwrap()));
        assert_eq!(hedef_dogrula(1, &[10, 50, 0, 1], 80, &a), Err(RET_KURAL)); // alt ağ dışı
        assert_eq!(hedef_dogrula(1, &[8, 8, 8, 8], 53, &a), Err(RET_KURAL));
        assert_eq!(hedef_dogrula(1, &[127, 0, 0, 1], 22, &a), Err(RET_KURAL));
        assert_eq!(hedef_dogrula(1, &[192, 168, 1, 0], 80, &a), Err(RET_KURAL));
        assert_eq!(hedef_dogrula(1, &[192, 168, 1, 255], 80, &a), Err(RET_KURAL));
        assert_eq!(hedef_dogrula(1, &[192, 168, 1, 1], 0, &a), Err(RET_KURAL));
        assert_eq!(hedef_dogrula(3, b"tplinkap.net", 80, &a), Err(RET_KONAK)); // ad çözümü yok
        assert_eq!(hedef_dogrula(4, &[0; 16], 80, &a), Err(RET_ADRES_TURU));
        assert_eq!(hedef_dogrula(1, &[192, 168], 80, &a), Err(RET_ADRES_TURU));
    }

    /// Yerel geri döngüde aracı + sahte bağlantı kurucu (gerçek ağ ad alanı yok); hedef sunucu da yerel.
    fn araci_dene(istek: &[u8], baglan: Arc<Baglan>) -> Vec<u8> {
        let dinle = TcpListener::bind("127.0.0.1:0").unwrap();
        let adres = dinle.local_addr().unwrap();
        std::thread::spawn(move || sun(dinle, ag(), baglan));
        let mut c = TcpStream::connect(adres).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(istek).unwrap();
        let mut v = vec![];
        let _ = c.read_to_end(&mut v);
        v
    }

    #[test]
    fn socks_akisi() {
        // hedef "AP": yerel sunucu; sahte kurucu her izinli hedefi ona bağlar ve istenen hedefi kaydeder
        let ap = TcpListener::bind("127.0.0.1:0").unwrap();
        let ap_adres = ap.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in ap.incoming().flatten() {
                let mut s = s;
                let mut b = [0u8; 4];
                let _ = s.read_exact(&mut b);
                let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\nmerhaba");
            }
        });
        let istenen = Arc::new(std::sync::Mutex::new(vec![]));
        let i2 = istenen.clone();
        let baglan: Arc<Baglan> = Arc::new(move |h: SocketAddrV4| {
            i2.lock().unwrap().push(h);
            TcpStream::connect(ap_adres)
        });
        // kimliksiz el sıkışma + CONNECT alan adı biçiminde IP (Chrome) → 0 + aktarım
        let mut istek = vec![5, 1, 0, 5, 1, 0, 3, 11];
        istek.extend(b"192.168.1.1");
        istek.extend(80u16.to_be_bytes());
        istek.extend(b"GET ");
        let v = araci_dene(&istek, baglan.clone());
        assert_eq!(&v[..2], &[5, 0]);
        assert_eq!(&v[2..4], &[5, 0], "{v:?}");
        assert!(v.ends_with(b"merhaba"));
        assert_eq!(*istenen.lock().unwrap(), vec!["192.168.1.1:80".parse::<SocketAddrV4>().unwrap()]);
        // alt ağ dışı: kural reddi, bağlanılmaz
        let v = araci_dene(&[5, 1, 0, 5, 1, 0, 1, 10, 50, 0, 1, 0, 80], baglan.clone());
        assert_eq!(v, [5, 0, 5, RET_KURAL, 0, 1, 0, 0, 0, 0, 0, 0]);
        // yalnızca parola yöntemi önerilirse: yöntem yok
        assert_eq!(araci_dene(&[5, 1, 2], baglan.clone()), [5, 0xff]);
        // BIND komutu: desteklenmez
        assert_eq!(araci_dene(&[5, 1, 0, 5, 2, 0, 1, 192, 168, 1, 1, 0, 80], baglan.clone())[2..4], [5, RET_KOMUT]);
        // SOCKS4: bağlantı kapanır, cevap yok
        assert!(araci_dene(&[4, 1, 0, 80, 192, 168, 1, 1, 0], baglan.clone()).is_empty());
        assert_eq!(istenen.lock().unwrap().len(), 1);
        // bağlanamadı (reddedildi): RET_RED
        let red: Arc<Baglan> = Arc::new(|_| Err(io::Error::from(io::ErrorKind::ConnectionRefused)));
        assert_eq!(araci_dene(&[5, 1, 0, 5, 1, 0, 1, 192, 168, 1, 1, 0, 80], red)[2..4], [5, RET_RED]);
        let sure: Arc<Baglan> = Arc::new(|_| Err(io::Error::from(io::ErrorKind::TimedOut)));
        assert_eq!(araci_dene(&[5, 1, 0, 5, 1, 0, 1, 192, 168, 1, 1, 0, 80], sure)[2..4], [5, RET_SURE]);
    }

    #[test]
    fn baglanti_siniri() {
        // sınır dolunca yeni bağlantı hemen kapanır
        let dinle = TcpListener::bind("127.0.0.1:0").unwrap();
        let adres = dinle.local_addr().unwrap();
        let baglan: Arc<Baglan> = Arc::new(|_| Err(io::Error::from(io::ErrorKind::ConnectionRefused)));
        std::thread::spawn(move || sun(dinle, ag(), baglan));
        let acik: Vec<TcpStream> = (0..EN_COK_BAGLANTI).map(|_| TcpStream::connect(adres).unwrap()).collect();
        std::thread::sleep(Duration::from_millis(300));
        let mut fazla = TcpStream::connect(adres).unwrap();
        fazla.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let _ = fazla.write_all(&[5, 1, 0]);
        let mut b = [0u8; 2];
        assert!(matches!(fazla.read(&mut b), Ok(0) | Err(_)), "fazla bağlantı kapanmalı");
        drop(acik);
    }

    fn teklif_paketi(xid: u32, tur: u8, sunucu: Option<[u8; 4]>) -> Vec<u8> {
        let mut d = vec![0u8; 236];
        d[0] = 2;
        d[4..8].copy_from_slice(&xid.to_be_bytes());
        d[16..20].copy_from_slice(&[192, 168, 1, 100]);
        d.extend([99, 130, 83, 99, 0, 53, 1, tur]);
        if let Some(s) = sunucu {
            d.extend([54, 4]);
            d.extend(s);
        }
        d.extend([1, 4, 255, 255, 255, 0, 255]);
        let mut p = vec![0x45, 0, 0, 0, 0, 0, 0, 0, 64, 17, 0, 0, 192, 168, 1, 1, 255, 255, 255, 255];
        p.extend(67u16.to_be_bytes());
        p.extend(68u16.to_be_bytes());
        p.extend(((8 + d.len()) as u16).to_be_bytes());
        p.extend([0, 0]);
        p.extend(d);
        p
    }

    #[test]
    fn dhcp_paketleri() {
        let mac = [0x02, 1, 2, 3, 4, 5];
        let k = kesif(&mac, 0xdead_beef);
        assert_eq!(k.len(), 20 + 8 + 300);
        assert_eq!(toplam(&k[..20]), 0, "IP başlık toplamı");
        assert_eq!((&k[12..16], &k[16..20]), (&[0u8, 0, 0, 0][..], &[255u8, 255, 255, 255][..]));
        assert_eq!((u16::from_be_bytes([k[20], k[21]]), u16::from_be_bytes([k[22], k[23]])), (68, 67));
        let d = &k[28..];
        assert_eq!((d[0], &d[4..8], d[10], &d[28..34]), (1, &0xdead_beefu32.to_be_bytes()[..], 0x80, &mac[..]));
        assert_eq!(&d[236..243], &[99, 130, 83, 99, 53, 1, 1]);
        // kendi isteğimiz teklif sayılmaz
        assert_eq!(teklif_coz(&k, 0xdead_beef), None);
        // teklif: sunucu kimliği seçeneği, yoksa kaynak IP
        assert_eq!(teklif_coz(&teklif_paketi(7, 2, Some([192, 168, 1, 254])), 7), Some(("192.168.1.254".parse().unwrap(), "192.168.1.100".parse().unwrap())));
        assert_eq!(teklif_coz(&teklif_paketi(7, 2, None), 7), Some(("192.168.1.1".parse().unwrap(), "192.168.1.100".parse().unwrap())));
        assert_eq!(teklif_coz(&teklif_paketi(7, 5, None), 7), None); // ACK değil teklif aranıyor
        assert_eq!(teklif_coz(&teklif_paketi(8, 2, None), 7), None); // başka xid
        let mut kisa = teklif_paketi(7, 2, None);
        kisa.truncate(200);
        assert_eq!(teklif_coz(&kisa, 7), None);
        let mut bozuk = teklif_paketi(7, 2, None);
        let n = bozuk.len();
        bozuk[n - 6] = 200; // son seçeneğin (1: alt ağ maskesi) boyu paketi taşar
        assert_eq!(teklif_coz(&bozuk, 7), None);
    }
}
