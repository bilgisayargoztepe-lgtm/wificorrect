//! WifiCorrect yönetim merkezi (Rust). Python merkezinin (scripts/sunucu/merkez/) davranışı aynen taşınır.
// İskelet: modüllerin çoğu sonraki görevlerde kullanılır.
#![allow(dead_code)]

#[cfg(test)]
mod altin;
mod detay;
mod guvenlik;
mod istatistik;
mod kayitlar;
mod merkez;
mod ortak;
mod ozet;
mod rapor;
mod raporlar;
mod sunucu;
#[cfg(test)]
mod test_yardim;
mod twilio;
mod veri;
mod web;

const SURUM: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("surum") => println!("{SURUM}"),
        Some("sunucu") => sunucu_calistir(),
        Some("istatistik") => std::process::exit(istatistik::calistir(&istatistik::Yollar::uretim(), None)),
        _ => {
            eprintln!("Kullanım: wificorrect-merkez surum|sunucu|istatistik");
            std::process::exit(2);
        }
    }
}

/// Python `merkez.main`: umask 077, veritabanı, rapor grupları, uygulamalar, 127.0.0.1:8081.
/// Uygulamalar (yönetim, müşteri, api, cihaz, ap) taşındıkça buraya eklenir; kaydı olmayan alan adı 404 verir.
fn sunucu_calistir() {
    use std::sync::Arc;
    // SAFETY: umask çağrısı yan etkisiz bir sistem çağrısıdır
    unsafe {
        libc::umask(0o077);
    }
    let saat: veri::Saat = Arc::new(ortak::simdi);
    let v = match veri::Veri::ac(std::path::Path::new(merkez::DB), saat) {
        Ok(v) => v,
        Err(h) => {
            eprintln!("{h}");
            std::process::exit(1);
        }
    };
    // raporlar için müşteri ↔ eski arşiv bağı (root okur); yazılamazsa sunucu yine açılır
    if let Err(h) = v.rapor_gruplari_yaz(std::path::Path::new(veri::RAPOR_GRUPLARI)) {
        web::log(&format!("rapor-gruplari yazilamadi: {h:?}"));
    }
    let m = merkez::Merkez::new();
    if let Err(h) = sunucu::calistir(Arc::new(m), "127.0.0.1:8081", 256) {
        eprintln!("{h}");
        std::process::exit(1);
    }
}
