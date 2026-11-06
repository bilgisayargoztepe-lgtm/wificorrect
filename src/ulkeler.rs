//! Ülke kodları ve telefon numarası doğrulama. Numaralar uluslararası biçimde (E.164, başında + olmadan) saklanır:
//! `905334553132`, `4915123456789`. Portalda varsayılan Türkiye (+90); yabancı numara için ülke seçilir ya da `+…` yazılır.
//! ponytail: ülke başına numara uzunluğu kuralı yalnızca Türkiye için (5XX XXX XX XX); diğerlerinde 4–14 hane ulusal numara.

/// (ISO kodu, ülke kodu, Türkçe ad) — Türkiye başta, gerisi Türkçe alfabetik. Liste gerekirse genişletilir.
pub const ULKELER: &[(&str, &str, &str)] = &[
    ("TR", "90", "Türkiye"),
    ("AF", "93", "Afganistan"),
    ("DE", "49", "Almanya"),
    ("US", "1", "Amerika Birleşik Devletleri"),
    ("AD", "376", "Andorra"),
    ("AO", "244", "Angola"),
    ("AR", "54", "Arjantin"),
    ("AL", "355", "Arnavutluk"),
    ("AU", "61", "Avustralya"),
    ("AT", "43", "Avusturya"),
    ("AZ", "994", "Azerbaycan"),
    ("BH", "973", "Bahreyn"),
    ("BD", "880", "Bangladeş"),
    ("BY", "375", "Belarus"),
    ("BE", "32", "Belçika"),
    ("BJ", "229", "Benin"),
    ("AE", "971", "Birleşik Arap Emirlikleri"),
    ("GB", "44", "Birleşik Krallık"),
    ("BO", "591", "Bolivya"),
    ("BA", "387", "Bosna-Hersek"),
    ("BW", "267", "Botsvana"),
    ("BR", "55", "Brezilya"),
    ("BN", "673", "Brunei"),
    ("BG", "359", "Bulgaristan"),
    ("BF", "226", "Burkina Faso"),
    ("DZ", "213", "Cezayir"),
    ("DJ", "253", "Cibuti"),
    ("TD", "235", "Çad"),
    ("CZ", "420", "Çekya"),
    ("CN", "86", "Çin"),
    ("DK", "45", "Danimarka"),
    ("EC", "593", "Ekvador"),
    ("SV", "503", "El Salvador"),
    ("ID", "62", "Endonezya"),
    ("ER", "291", "Eritre"),
    ("AM", "374", "Ermenistan"),
    ("EE", "372", "Estonya"),
    ("ET", "251", "Etiyopya"),
    ("MA", "212", "Fas"),
    ("FJ", "679", "Fiji"),
    ("CI", "225", "Fildişi Sahili"),
    ("PH", "63", "Filipinler"),
    ("PS", "970", "Filistin"),
    ("FI", "358", "Finlandiya"),
    ("FR", "33", "Fransa"),
    ("GA", "241", "Gabon"),
    ("GM", "220", "Gambiya"),
    ("GH", "233", "Gana"),
    ("GN", "224", "Gine"),
    ("GT", "502", "Guatemala"),
    ("ZA", "27", "Güney Afrika"),
    ("CY", "357", "Güney Kıbrıs"),
    ("KR", "82", "Güney Kore"),
    ("GE", "995", "Gürcistan"),
    ("HT", "509", "Haiti"),
    ("IN", "91", "Hindistan"),
    ("HR", "385", "Hırvatistan"),
    ("NL", "31", "Hollanda"),
    ("HN", "504", "Honduras"),
    ("HK", "852", "Hong Kong"),
    ("IQ", "964", "Irak"),
    ("IR", "98", "İran"),
    ("IE", "353", "İrlanda"),
    ("ES", "34", "İspanya"),
    ("IL", "972", "İsrail"),
    ("SE", "46", "İsveç"),
    ("CH", "41", "İsviçre"),
    ("IT", "39", "İtalya"),
    ("IS", "354", "İzlanda"),
    ("JP", "81", "Japonya"),
    ("KH", "855", "Kamboçya"),
    ("CM", "237", "Kamerun"),
    ("CA", "1", "Kanada"),
    ("ME", "382", "Karadağ"),
    ("QA", "974", "Katar"),
    ("KZ", "7", "Kazakistan"),
    ("KE", "254", "Kenya"),
    ("KG", "996", "Kırgızistan"),
    ("CO", "57", "Kolombiya"),
    ("CG", "242", "Kongo"),
    ("CD", "243", "Kongo Demokratik Cumhuriyeti"),
    ("XK", "383", "Kosova"),
    ("CR", "506", "Kosta Rika"),
    ("KW", "965", "Kuveyt"),
    ("MK", "389", "Kuzey Makedonya"),
    ("CU", "53", "Küba"),
    ("LA", "856", "Laos"),
    ("LV", "371", "Letonya"),
    ("LR", "231", "Liberya"),
    ("LY", "218", "Libya"),
    ("LI", "423", "Lihtenştayn"),
    ("LT", "370", "Litvanya"),
    ("LB", "961", "Lübnan"),
    ("LU", "352", "Lüksemburg"),
    ("HU", "36", "Macaristan"),
    ("MO", "853", "Makao"),
    ("MG", "261", "Madagaskar"),
    ("MW", "265", "Malavi"),
    ("MV", "960", "Maldivler"),
    ("MY", "60", "Malezya"),
    ("ML", "223", "Mali"),
    ("MT", "356", "Malta"),
    ("MU", "230", "Mauritius"),
    ("MX", "52", "Meksika"),
    ("EG", "20", "Mısır"),
    ("MN", "976", "Moğolistan"),
    ("MD", "373", "Moldova"),
    ("MC", "377", "Monako"),
    ("MR", "222", "Moritanya"),
    ("MZ", "258", "Mozambik"),
    ("MM", "95", "Myanmar"),
    ("NA", "264", "Namibya"),
    ("NP", "977", "Nepal"),
    ("NE", "227", "Nijer"),
    ("NG", "234", "Nijerya"),
    ("NI", "505", "Nikaragua"),
    ("NO", "47", "Norveç"),
    ("UZ", "998", "Özbekistan"),
    ("PK", "92", "Pakistan"),
    ("PA", "507", "Panama"),
    ("PY", "595", "Paraguay"),
    ("PE", "51", "Peru"),
    ("PL", "48", "Polonya"),
    ("PT", "351", "Portekiz"),
    ("RO", "40", "Romanya"),
    ("RW", "250", "Ruanda"),
    ("RU", "7", "Rusya"),
    ("SM", "378", "San Marino"),
    ("SN", "221", "Senegal"),
    ("SL", "232", "Sierra Leone"),
    ("SG", "65", "Singapur"),
    ("SK", "421", "Slovakya"),
    ("SI", "386", "Slovenya"),
    ("SO", "252", "Somali"),
    ("LK", "94", "Sri Lanka"),
    ("SD", "249", "Sudan"),
    ("SY", "963", "Suriye"),
    ("SA", "966", "Suudi Arabistan"),
    ("RS", "381", "Sırbistan"),
    ("CL", "56", "Şili"),
    ("TJ", "992", "Tacikistan"),
    ("TZ", "255", "Tanzanya"),
    ("TH", "66", "Tayland"),
    ("TW", "886", "Tayvan"),
    ("TG", "228", "Togo"),
    ("TN", "216", "Tunus"),
    ("TM", "993", "Türkmenistan"),
    ("UG", "256", "Uganda"),
    ("UA", "380", "Ukrayna"),
    ("OM", "968", "Umman"),
    ("UY", "598", "Uruguay"),
    ("JO", "962", "Ürdün"),
    ("VE", "58", "Venezuela"),
    ("VN", "84", "Vietnam"),
    ("YE", "967", "Yemen"),
    ("NZ", "64", "Yeni Zelanda"),
    ("GR", "30", "Yunanistan"),
    ("ZM", "260", "Zambiya"),
    ("ZW", "263", "Zimbabve"),
];

pub fn dial(iso: &str) -> Option<&'static str> {
    ULKELER.iter().find(|(i, _, _)| *i == iso).map(|(_, d, _)| *d)
}

/// ISO kodundan bayrak (bölgesel gösterge harfleri).
pub fn flag(iso: &str) -> String {
    iso.chars().filter(char::is_ascii_uppercase).map(|c| char::from_u32(0x1F1E6 + (c as u32 - 'A' as u32)).unwrap_or(c)).collect()
}

/// En uzun eşleşen ülke kodu (`4915…` → "49").
fn match_dial(digits: &str) -> Option<&'static str> {
    ULKELER.iter().map(|(_, d, _)| *d).filter(|d| digits.starts_with(d)).max_by_key(|d| d.len())
}

fn check(cc: &str, national: &str) -> Option<String> {
    let ok = if cc == "90" {
        national.len() == 10 && national.starts_with('5') // Türkiye GSM
    } else {
        (4..=14).contains(&national.len()) && cc.len() + national.len() <= 15
    };
    ok.then(|| format!("{cc}{national}"))
}

/// Seçilen ülke + girilen numara → uluslararası biçim (E.164, + olmadan). `+…` ya da `00…` ile yazılırsa seçim yok sayılır.
pub fn normalize_phone(ulke: &str, raw: &str) -> Option<String> {
    let raw = raw.trim();
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if raw.starts_with('+') || digits.starts_with("00") {
        let d = if raw.starts_with('+') { digits.as_str() } else { &digits[2..] };
        let cc = match_dial(d)?;
        return check(cc, &d[cc.len()..]);
    }
    let cc = dial(ulke)?;
    if cc == "90" {
        // Türkiye: +90 / 90 / 0 önekiyle ya da öneksiz yazılabilir
        let n = if digits.len() == 12 && digits.starts_with("90") {
            &digits[2..]
        } else if digits.len() == 11 && digits.starts_with('0') {
            &digits[1..]
        } else {
            digits.as_str()
        };
        return check(cc, n);
    }
    check(cc, digits.trim_start_matches('0')) // çoğu ülkede ulusal 0 öneki
}

/// `+90 5XX XXX XX 32` / `+49 XXXXXXXX89` (kod ekranı ve maskeli listeler).
pub fn mask_phone(e164: &str) -> String {
    let cc = match_dial(e164).unwrap_or("");
    let national = &e164[cc.len()..];
    if cc == "90" && national.len() == 10 {
        return format!("+90 {}XX XXX XX {}", &national[..1], &national[8..]);
    }
    let keep = national.len().saturating_sub(2);
    format!("+{cc} {}{}", "X".repeat(keep), &national[keep..])
}

/// Portal formundaki ülke listesi; değerler ISO kodu, gösterim "🇹🇷 +90 Türkiye".
pub fn options_html(selected: &str) -> String {
    ULKELER
        .iter()
        .map(|(iso, d, ad)| {
            let sel = if *iso == selected { " selected" } else { "" };
            format!("<option value=\"{iso}\"{sel}>{} +{d} {ad}</option>", flag(iso))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turkish_numbers() {
        for ok in ["+90 (533) 455 31 32", "0533 455 3132", "5334553132", "905334553132", "05334553132"] {
            assert_eq!(normalize_phone("TR", ok).as_deref(), Some("905334553132"), "{ok}");
        }
        for bad in ["2124553132", "533455313", "53345531321", "abc", "", "0212 455 31 32"] {
            assert_eq!(normalize_phone("TR", bad), None, "{bad}");
        }
    }

    #[test]
    fn foreign_numbers() {
        assert_eq!(normalize_phone("DE", "0151 2345 6789").as_deref(), Some("4915123456789"));
        assert_eq!(normalize_phone("TR", "+49 151 23456789").as_deref(), Some("4915123456789")); // + ile yazılınca seçim yok sayılır
        assert_eq!(normalize_phone("GB", "07700 900123").as_deref(), Some("447700900123"));
        assert_eq!(normalize_phone("US", "(212) 555-0123").as_deref(), Some("12125550123"));
        assert_eq!(normalize_phone("TR", "0049 151 23456789").as_deref(), Some("4915123456789"));
        assert_eq!(normalize_phone("TR", "+90 212 455 31 32"), None); // Türk sabit hat
        assert_eq!(normalize_phone("DE", "12"), None);
        assert_eq!(normalize_phone("XX", "5334553132"), None);
        assert_eq!(normalize_phone("TR", "+999 1234567"), None); // bilinmeyen ülke kodu
        assert_eq!(normalize_phone("AZ", "050 123 45 67").as_deref(), Some("994501234567")); // 994, 99 değil (en uzun eşleşme)
    }

    #[test]
    fn masks_flags_options() {
        assert_eq!(mask_phone("905334553132"), "+90 5XX XXX XX 32");
        assert_eq!(mask_phone("4915123456789"), "+49 XXXXXXXXX89");
        assert_eq!(flag("TR"), "🇹🇷");
        let html = options_html("TR");
        assert!(html.starts_with("<option value=\"TR\" selected>🇹🇷 +90 Türkiye</option>"));
        assert_eq!(html.matches(" selected").count(), 1);
        let mut isos: Vec<&str> = ULKELER.iter().map(|u| u.0).collect();
        isos.sort();
        isos.dedup();
        assert_eq!(isos.len(), ULKELER.len(), "ISO kodu tekrar etmesin");
    }
}
