//! Yönetim merkezi (yonetim.wificorrect.com) yönetici hesabı (Python yonetici.py): tek hesap, dosyada.
//! CLI: `wificorrect-merkez yonetici parola <kullanıcı-adı>` (`wificorrect-sunucu yonetici-parola` çağırır).
//! Root çalıştırılır; parolayı sorduktan sonra, dosyaya dokunmadan önce `wcpanel`'e iner (klasör onun: root olarak
//! yazmak sembolik bağ izleyebilirdi).
use std::path::Path;

use serde_json::{json, Value};

use crate::guvenlik;

pub const YONETICI: &str = "/var/lib/wificorrect/merkez/yonetici.json";
const EN_AZ: usize = 12;
const KULLANIM: &str = "Kullanım: wc-yonetici parola <kullanıcı-adı>";

/// Dosyadaki yönetici hesabı.
#[derive(Clone, Debug, PartialEq)]
pub struct Hesap {
    pub kullanici: String,
    pub tuz: String,
    pub ozet: String,
    pub yineleme: u32,
}

/// Python `KUL_RE.fullmatch` (`[a-z0-9._-]{3,32}`).
pub fn kullanici_gecerli(k: &str) -> bool {
    (3..=32).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

/// Hesabı (yeniden) yazar: 0600. Geçersiz kullanıcı adı ya da kısa parola `ValueError` metniyle `Err`.
pub fn yaz(yol: &Path, kullanici: &str, pw: &str) -> Result<(), String> {
    if !kullanici_gecerli(kullanici) {
        return Err("Kullanıcı adı 3-32 karakter: küçük harf, rakam, . _ -".into());
    }
    if pw.chars().count() < EN_AZ {
        return Err(format!("Parola en az {EN_AZ} karakter olmalı."));
    }
    let (tuz, oz, y) = guvenlik::yeni_kayit(pw);
    crate::ortak::save_json(yol, &json!({"kullanici": kullanici, "tuz": tuz, "ozet": oz, "yineleme": y})).map_err(|e| e.to_string())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(yol, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

/// Dosya yok, JSON değil ya da dört alan (kullanici, tuz, ozet, yineleme) eksik/yanlış türdeyse None.
/// (Python yalnızca yokluğu ve bozuk JSON'u yutardı; okunamayan dosya ve yanlış türdeki alan orada istek hatası verirdi,
/// burada giriş reddedilir.)
pub fn oku(yol: &Path) -> Option<Hesap> {
    let ham = std::fs::read(yol).ok()?;
    let v: Value = serde_json::from_slice(&ham).ok()?;
    Some(Hesap {
        kullanici: v.get("kullanici")?.as_str()?.to_string(),
        tuz: v.get("tuz")?.as_str()?.to_string(),
        ozet: v.get("ozet")?.as_str()?.to_string(),
        yineleme: u32::try_from(v.get("yineleme")?.as_u64()?).ok()?,
    })
}

/// Python `getpass`: terminalden yankısız iki kez sorar; aynı değilse None. Terminal yoksa standart girdiden okur.
pub fn sor() -> Option<String> {
    let s1 = parola_oku("Parola: ")?;
    let s2 = parola_oku("Parola (tekrar): ")?;
    (s1 == s2).then_some(s1)
}

fn parola_oku(istem: &str) -> Option<String> {
    use std::io::{BufRead, Write};
    use std::os::fd::AsRawFd;
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").ok();
    let satir = match tty {
        Some(mut t) => {
            let fd = t.as_raw_fd();
            // SAFETY: fd geçerli; termios yapısı sıfırlanıp tcgetattr ile doldurulur
            let eski = unsafe {
                let mut a: libc::termios = std::mem::zeroed();
                (libc::tcgetattr(fd, &mut a) == 0).then_some(a)
            };
            let _ = t.write_all(istem.as_bytes());
            if let Some(mut a) = eski {
                a.c_lflag &= !libc::ECHO;
                // SAFETY: fd geçerli, a geçerli termios
                unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &a) };
            }
            let mut s = String::new();
            let okuma = std::io::BufReader::new(&t).read_line(&mut s);
            if let Some(a) = eski {
                // SAFETY: fd geçerli, a tcgetattr'den gelen özgün ayar
                unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &a) };
            }
            let _ = t.write_all(b"\n");
            okuma.ok()?;
            s
        }
        None => {
            eprint!("{istem}");
            let mut s = String::new();
            std::io::stdin().lock().read_line(&mut s).ok()?;
            s
        }
    };
    Some(satir.trim_end_matches(['\n', '\r']).to_string())
}

/// root ise `sahip` kullanıcısına iner (setgroups, setgid, setuid); root değilse bir şey yapmaz.
pub fn birak(sahip: &str) -> Result<(), String> {
    // SAFETY: geteuid yan etkisiz
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let ad = std::ffi::CString::new(sahip).map_err(|e| e.to_string())?;
    // SAFETY: ad geçerli NUL sonlu dize; dönen yapı hemen okunur
    let p = unsafe { libc::getpwnam(ad.as_ptr()) };
    if p.is_null() {
        return Err(format!("'{sahip}' kullanıcısı yok"));
    }
    // SAFETY: p boş değil; setgroups boş liste ile; sıra: grup, sonra kullanıcı
    unsafe {
        let (uid, gid) = ((*p).pw_uid, (*p).pw_gid);
        if libc::setgroups(0, std::ptr::null()) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
            return Err(format!("'{sahip}' kullanıcısına inilemedi"));
        }
    }
    Ok(())
}

/// Python `main(argv, yol, sor, sahip, birak)`. Çıkış kodu: 0 tamam, 1 parolalar farklı/geçersiz, 2 kullanım.
/// Parola önce sorulur: `wcpanel`'e indikten sonra terminale (/dev/tty) erişilemez, parola ekranda görünürdü.
pub fn main(argv: &[String], yol: &Path, sor: &dyn Fn() -> Option<String>, sahip: Option<&str>, birak: &dyn Fn(&str) -> Result<(), String>) -> i32 {
    if argv.len() != 2 || argv[0] != "parola" || !kullanici_gecerli(&argv[1]) {
        eprintln!("{KULLANIM}");
        return 2;
    }
    let pw = sor();
    if let Some(s) = sahip {
        if let Err(h) = birak(s) {
            eprintln!("{h}");
            return 1;
        }
    }
    let Some(pw) = pw else {
        eprintln!("Parolalar aynı değil.");
        return 1;
    };
    if let Err(h) = yaz(yol, &argv[1], &pw) {
        eprintln!("{h}");
        return 1;
    }
    println!("tamam");
    0
}

/// `wificorrect-merkez yonetici parola AD`: canlı yol, terminalden sorma, `wcpanel`'e inme.
pub fn main_uretim(argv: &[String]) -> i32 {
    // SAFETY: umask çağrısı yan etkisiz bir sistem çağrısıdır; geçici dosya (parola özeti) başkasınca okunmasın
    unsafe {
        libc::umask(0o077);
    }
    main(argv, Path::new(YONETICI), &sor, Some("wcpanel"), &birak)
}

#[cfg(test)]
#[path = "yonetici_testler.rs"]
mod testler;
