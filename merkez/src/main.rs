//! WifiCorrect yönetim merkezi (Rust). Python merkezinin (scripts/sunucu/merkez/) davranışı aynen taşınır.
// İskelet: modüllerin çoğu sonraki görevlerde kullanılır.
#![allow(dead_code)]

mod guvenlik;
mod ortak;

const SURUM: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("surum") => println!("{SURUM}"),
        _ => {
            eprintln!("Kullanım: wificorrect-merkez surum");
            std::process::exit(2);
        }
    }
}
