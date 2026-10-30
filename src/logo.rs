//! wificorrect_ logosu ve sekme simgesi (2026-10-24, kullanıcının logo kiti: Desktop/logo; harfler yola çevrilmiş, yazı tipi
//! gerekmez). Dosyalar `src/sablon/logo/` altında; merkezde aynıları `scripts/sunucu/merkez/logo/` (merkez testi ikisinin aynı
//! olduğunu denetler). Sayfaya gömülür: panelin CSP'si (`img-src data:`) ve merkezin cihaz bloğundaki CSP dosya sunmaya izin
//! vermez, satır içi SVG ve `data:` simgesi CSP'ye takılmaz (cihaz ile merkez hangi sırayla güncellenirse güncellensin).
//! Yerleşim yönetimdeki gibi: sol menünün üstünde koyu zemin logosu, oturumsuz sayfalarda beyaz şeritte açık zemin logosu
//! (`w-koyu.svg` merkezle aynı kalsın diye klasörde duruyor, cihaz kullanmıyor).

use std::sync::OnceLock;

/// Açık zemin: antrasit "wifi" + mercan "correct_" (oturumsuz sayfaların beyaz üst şeridi)
const LOGO_ACIK: &str = include_str!("sablon/logo/logo-acik.svg");
/// Koyu zemin: beyaz "wifi" + mercan "correct_" (sol menü ve telefondaki üst şerit)
const LOGO_KOYU: &str = include_str!("sablon/logo/logo-koyu.svg");
/// Sekme simgesi: W_ uygulama ikonu (SVG destekleyen tarayıcılar) ve 32 px PNG (ötekiler)
const FAVICON_SVG: &str = include_str!("sablon/logo/favicon.svg");
const FAVICON_PNG: &[u8] = include_bytes!("sablon/logo/favicon-32.png");

/// Standart base64 (RFC 4648, dolgulu).
fn base64(veri: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(veri.len().div_ceil(3) * 4);
    for p in veri.chunks(3) {
        let n = (u32::from(p[0]) << 16) | (u32::from(*p.get(1).unwrap_or(&0)) << 8) | u32::from(*p.get(2).unwrap_or(&0));
        for (i, kaydir) in [18, 12, 6, 0].into_iter().enumerate() {
            s.push(if i <= p.len() { ABC[(n >> kaydir) as usize & 63] as char } else { '=' });
        }
    }
    s
}

/// SVG'ye sınıf verir ve ekran okuyucudan gizler (adı çevreleyen öğe verir).
fn sinifli(svg: &str, sinif: &str) -> String {
    svg.trim().replacen("<svg ", &format!("<svg class=\"{sinif}\" aria-hidden=\"true\" focusable=\"false\" "), 1)
}

/// `<head>` içine: sekme simgesi bağlantıları (PNG önce, SVG sonra: ikisini de tanıyan tarayıcı sonuncuyu seçer).
pub fn favicon_html() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| {
        format!(
            "<link rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"data:image/png;base64,{}\">\
             <link rel=\"icon\" type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,{}\">",
            base64(FAVICON_PNG),
            base64(FAVICON_SVG.trim().as_bytes())
        )
    })
}

/// Ana sayfaya giden logo (merkezdeki `web._marka` ile aynı biçim).
fn marka(svg: &str) -> String {
    format!("<a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\">{}</a>", sinifli(svg, "logo"))
}

/// Koyu zemin için logo bağlantısı: sol menünün üstü ve telefondaki üst şerit (panel.html `.marka`).
pub fn marka_koyu() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| marka(LOGO_KOYU))
}

/// Açık zemin için logo bağlantısı: oturumsuz sayfaların beyaz üst şeridi (yönetimin giriş sayfası gibi).
pub fn marka_acik() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| marka(LOGO_ACIK))
}

/// Sol şeridin üstündeki W_ uygulama işareti (sekme simgesiyle aynı çizim; 2026-10-30 WhatsApp Web benzeri panel), ana sayfaya gider.
pub fn isaret() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| format!("<a class=\"marka isaret\" href=\"/\" aria-label=\"wificorrect_\">{}</a>", sinifli(FAVICON_SVG, "logo")))
}

/// Giriş sayfasının alt bilgisi (açık zemin, `_sayfa.html` `footer .wc-logo`): bağlantısız, süs; ürünün adını alt bilgi yazısı
/// söyler. Misafir giriş yapmadan internete çıkamaz, bağlantı verilmez.
pub fn alt_bilgi_logo() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| sinifli(LOGO_ACIK, "wc-logo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648_ornekleri() {
        for (girdi, beklenen) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(girdi.as_bytes()), beklenen, "{girdi}");
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn dosyalar_gecerli_ve_renkler_kitteki_gibi() {
        for svg in [LOGO_ACIK, LOGO_KOYU, FAVICON_SVG] {
            assert!(svg.trim_start().starts_with("<svg ") && svg.trim_end().ends_with("</svg>") && svg.contains("viewBox="), "{svg}");
            assert!(!svg.contains("<script") && !svg.contains("href="), "SVG dışarıya bağlanmaz ve betik taşımaz");
        }
        // açık zemin: antrasit "wifi"; koyu zemin: beyaz "wifi"; ikisinde de mercan "correct_"; ikon antrasit zeminli
        assert!(LOGO_ACIK.contains("#293133") && LOGO_ACIK.contains("#FF6247"));
        assert!(LOGO_KOYU.contains("#FFFFFF") && LOGO_KOYU.contains("#FF6247") && !LOGO_KOYU.contains("#293133"));
        assert!(FAVICON_SVG.contains("fill=\"#293133\"") && FAVICON_SVG.contains("skewX"), "W_ ikonu (alt çizgili)");
        assert!(FAVICON_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn simge_ve_logo_baglantilari() {
        let f = favicon_html();
        assert!(f.contains("rel=\"icon\" type=\"image/png\" sizes=\"32x32\" href=\"data:image/png;base64,iVBORw0KGgo"), "{f}");
        assert!(f.contains("type=\"image/svg+xml\" href=\"data:image/svg+xml;base64,PHN2Zy"), "{f}");
        assert!(f.find("image/png").unwrap() < f.find("image/svg+xml").unwrap());
        for (m, renk) in [(marka_koyu(), "#FFFFFF"), (marka_acik(), "#293133")] {
            assert!(m.starts_with("<a class=\"marka\" href=\"/\" aria-label=\"wificorrect_\"><svg class=\"logo\" aria-hidden=\"true\" focusable=\"false\" "), "{m}");
            assert!(m.ends_with("</svg></a>") && m.matches("<svg ").count() == 1 && m.contains(renk), "{m}");
        }
    }
}
