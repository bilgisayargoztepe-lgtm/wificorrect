//! WifiCorrect — tek program, alt komutlarla çalışır (docs/RUST_YENIDEN_YAZIM.md, B/C bölümleri).

mod ag;
mod ayar;
mod ctl;
mod fabrika;
mod filtre;
mod hiz;
mod hesap;
mod kayit;
mod kaydedici;
mod merkez;
mod muhur;
mod ortak;
mod panel;
mod portal;
mod sms;
mod trafik;
mod twilio;
mod ulkeler;
mod uzak;

use std::process::ExitCode;

const USAGE: &str = "Kullanım: wificorrect <komut>

Komutlar:
  portal      Müşteri giriş portalı (SMS doğrulama)
  kaydedici   5651 bağlantı / DNS / oturum kaydı
  panel       Yönetim paneli (HTTPS)
  ctl         Yönetim komutları (durum, oturumlar, gun-kapat, yedekle ...)
  surum       Sürümü yazar

Ayar dosyası: /etc/wificorrect/ayarlar.toml (WFC_AYAR ortam değişkeniyle değiştirilebilir)";

fn config() -> Result<ayar::Config, ExitCode> {
    let path = std::env::var("WFC_AYAR").unwrap_or_else(|_| ayar::PATH.to_string());
    ayar::Config::load(&path).map_err(|e| {
        eprintln!("{e}");
        ExitCode::from(1)
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("surum") => {
            println!("wificorrect {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("portal") => match config() {
            Ok(cfg) => portal::run(cfg),
            Err(code) => code,
        },
        Some("kaydedici") => match config() {
            Ok(cfg) => kaydedici::run(cfg),
            Err(code) => code,
        },
        Some("ctl") => match config() {
            Ok(cfg) => ctl::run(cfg, &args[1..]),
            Err(code) => code,
        },
        Some("panel") => {
            let path = std::env::var("WFC_AYAR").unwrap_or_else(|_| ayar::PATH.to_string());
            panel::run(&path)
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
