//! Kayıt/arşiv sayfaları yardımcıları (Python detay.py). Bu görevde yalnızca `e()`; kalanı Task 5'te.
use std::fmt::Display;

/// Python `html.escape(str(s), quote=True)`: `&` `<` `>` `"` `'`.
pub fn e(s: impl Display) -> String {
    let s = s.to_string();
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#x27;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod testler {
    use super::e;

    #[test]
    fn kacis_python_gibi() {
        assert_eq!(e("<a href=\"x\">'&'</a>"), "&lt;a href=&quot;x&quot;&gt;&#x27;&amp;&#x27;&lt;/a&gt;");
        assert_eq!(e(42), "42");
        assert_eq!(e("ç ö ü"), "ç ö ü");
    }
}
