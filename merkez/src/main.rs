//! WifiCorrect yönetim merkezi (Rust). Python merkezinin (scripts/sunucu/merkez/) davranışı aynen taşınır.

#[cfg(test)]
mod altin;
mod ap;
mod api;
mod cihaz;
mod denetle;
mod detay;
mod durum;
mod guvenlik;
mod istatistik;
mod kayitlar;
mod kuyruk;
mod listeler;
mod merkez;
mod musteri;
mod netgsm;
mod ortak;
mod ozet;
mod rapor;
mod raporlar;
mod saglik;
mod sunucu;
mod surumler;
#[cfg(test)]
mod test_yardim;
mod twilio;
mod veri;
mod web;
mod yonetici;
mod yonetim;

const SURUM: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("surum") => println!("{SURUM}"),
        Some("sunucu") => sunucu_calistir(),
        Some("kuyruk") => std::process::exit(kuyruk::calistir_uretim()),
        Some("istatistik") => std::process::exit(istatistik::calistir(&istatistik::Yollar::uretim(), None)),
        Some("listeler") => std::process::exit(listeler::main_uretim()),
        Some("surumler") => std::process::exit(surumler::main_uretim(&args[1..])),
        Some("yonetici") => std::process::exit(yonetici::main_uretim(&args[1..])),
        Some("durum") => std::process::exit(durum::main_uretim()),
        Some("saglik") => std::process::exit(saglik::main_uretim()),
        Some("veritabani-denetle") => std::process::exit(denetle::main_uretim(&args[1..])),
        _ => {
            eprintln!("Kullanım: wificorrect-merkez surum|sunucu|kuyruk|istatistik|listeler|surumler|yonetici|durum|saglik|veritabani-denetle");
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
    let saat_uretim = saat.clone();
    let saat_musteri = saat.clone();
    let saat_yonetim = saat.clone();
    let saat_cihaz = saat.clone();
    let saat_ap = saat.clone();
    let v = match veri::Veri::ac(std::path::Path::new(merkez::DB), saat) {
        Ok(v) => Arc::new(v),
        Err(h) => {
            eprintln!("{h}");
            std::process::exit(1);
        }
    };
    // raporlar için müşteri ↔ eski arşiv bağı (root okur); yazılamazsa sunucu yine açılır
    if let Err(h) = v.rapor_gruplari_yaz(std::path::Path::new(veri::RAPOR_GRUPLARI)) {
        web::log(&format!("rapor-gruplari yazilamadi: {h:?}"));
    }
    let mut m = merkez::Merkez::new();
    // cihaz geçidi (Görev 9) ve yönetim aynı geçiş belirteçlerini kullanır
    let gecis = Arc::new(guvenlik::GecisBelirtecleri::new());
    m.ekle("api", Arc::new(api::Api::uretim(v.clone(), saat_uretim)));
    let musteri = Arc::new(musteri::Musteri::uretim(v.clone(), saat_musteri, gecis.clone()));
    m.ekle("musteri", musteri.clone());
    m.ekle("yonetim", Arc::new(yonetim::Yonetim::uretim(v.clone(), saat_yonetim, gecis.clone(), musteri.clone())));
    let cihaz = Arc::new(cihaz::Cihaz::new(v, gecis, musteri, saat_cihaz));
    m.ekle("cihaz", cihaz.clone());
    m.ekle("ap", Arc::new(ap::Ap::new(cihaz, saat_ap)));
    if let Err(h) = sunucu::calistir(Arc::new(m), "127.0.0.1:8081", 256) {
        eprintln!("{h}");
        std::process::exit(1);
    }
}
