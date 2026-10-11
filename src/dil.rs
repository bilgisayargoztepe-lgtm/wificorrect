//! Portal dili (TR | EN). Bütün müşteri metinleri tek tabloda, iki dil yan yana; yeni anahtar eklemek tek satır.
//! İngilizcesi eksik (boş) anahtar Türkçesine düşer. `{0}`, `{1}` → `f` ile doldurulur; `$ad` yer tutucuları şablonda
//! ikinci geçişte dolar (portal::render). Metinler güvenilir sabitlerdir: şablona kaçışsız girer (yalnızca <b> içerebilir).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dil {
    #[default]
    Tr,
    En,
}

impl Dil {
    pub fn kod(self) -> &'static str {
        match self {
            Dil::Tr => "tr",
            Dil::En => "en",
        }
    }

    /// "tr" / "en" (büyük-küçük harf duyarsız); başka her şey None.
    pub fn coz(s: &str) -> Option<Dil> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tr" => Some(Dil::Tr),
            "en" => Some(Dil::En),
            _ => None,
        }
    }
}

/// Öncelik: sorgu (`?dil=` ya da form alanı) > çerez `dil` > Accept-Language ("en" ile başlıyorsa EN) > TR.
/// Geçersiz değer yok sayılır, bir sonraki kaynağa bakılır.
pub fn sec(sorgu: Option<&str>, cerez: Option<&str>, accept_language: Option<&str>) -> Dil {
    if let Some(d) = sorgu.and_then(Dil::coz).or_else(|| cerez.and_then(Dil::coz)) {
        return d;
    }
    match accept_language {
        Some(al) if al.trim_start().to_ascii_lowercase().starts_with("en") => Dil::En,
        _ => Dil::Tr,
    }
}

/// `Cookie:` başlığından `dil` değeri.
pub fn cerezden(cookie: &str) -> Option<&str> {
    cookie.split(';').filter_map(|p| p.trim().split_once('=')).find(|(k, _)| *k == "dil").map(|(_, v)| v.trim())
}

/// Seçim bir yıl hatırlansın (giriş penceresi aynı sitede kalır: Lax yeterli).
pub fn cerez_basligi(d: Dil) -> String {
    format!("dil={}; Path=/; Max-Age=31536000; SameSite=Lax", d.kod())
}

/// (anahtar, Türkçe, İngilizce)
const METINLER: &[(&str, &str, &str)] = &[
    // çerçeve
    ("misafir_wifi", "Misafir Wi-Fi", "Guest Wi-Fi"),
    ("alt_bilgi", "WifiCorrect bir Göztepe Bilgisayar ürünüdür. Tüm hakları saklıdır.", "WifiCorrect is a Göztepe Bilgisayar product. All rights reserved."),
    ("dil_secimi", "Dil", "Language"),
    // giriş formu
    ("giris_baslik", "$site_ye hoş geldiniz", "Welcome to $site"),
    ("giris_aciklama", "İnternete bağlanmak için bilgilerinizi girin ve telefonunuza gelen kodu onaylayın.", "To connect to the internet, enter your details and confirm the code sent to your phone."),
    ("ad", "Ad", "First name"),
    ("soyad", "Soyad", "Last name"),
    ("telefon", "Cep telefonu", "Mobile phone"),
    ("ulke_kodu", "Ülke kodu", "Country code"),
    ("sozlesme_onay", "İnternet Kullanıcı Sözleşmesi'ni okudum, kabul ediyorum.", "I have read and accept the Internet User Agreement."),
    ("sozlesme_baslik", "İnternet Kullanıcı Sözleşmesi", "Internet User Agreement"),
    ("aydinlatma_baslik", "KVKK Aydınlatma Metni", "Privacy Notice (KVKK)"),
    ("aydinlatma_not", "Kişisel verilerinizin nasıl işlendiğini öğrenmek için KVKK Aydınlatma Metni'ne dokunun.", "Tap the Privacy Notice to learn how your personal data is processed."),
    ("kod_gonder", "Kod Gönder", "Send code"),
    ("metin_yok", "Metin henüz eklenmedi.", "This text has not been added yet."),
    // kod sayfası
    ("kod_baslik", "Doğrulama kodu", "Verification code"),
    ("kod_aciklama", "$telefon_maskeli numarasına 6 haneli bir kod gönderdik. Kod $dakika dakika geçerlidir.", "We sent a 6-digit code to $telefon_maskeli. The code is valid for $dakika minutes."),
    ("kod", "Kod", "Code"),
    ("dogrula", "Doğrula", "Verify"),
    ("tekrar_gonder", "Kodu tekrar gönder", "Resend code"),
    ("numara_degistir", "Numarayı değiştir", "Change number"),
    // başarı ve hata sayfaları
    ("basarili_baslik", "Bağlandınız", "You are connected"),
    ("basarili_metin", "$site Wi-Fi'ye bağlandınız, internet kullanıma hazır.", "You are connected to $site Wi-Fi; the internet is ready to use."),
    ("basarili_not", "Bu pencere kendiliğinden kapanmazsa üstteki <b>Bitti</b> ya da <b>Tamam</b> düğmesine basarak kapatabilirsiniz.", "If this window does not close by itself, tap <b>Done</b> or <b>OK</b> at the top to close it."),
    ("hata_baslik", "Bir sorun oluştu", "Something went wrong"),
    ("basa_don", "Başa dön", "Start over"),
    // hata ve bilgi iletileri
    ("mac_yok", "Cihazınız tanınamadı. Wi-Fi bağlantısını kapatıp yeniden açın; sorun sürerse personele başvurun.", "Your device could not be identified. Turn Wi-Fi off and on again; if the problem persists, please ask the staff."),
    ("hizmet_kapali", "Bu işletmenin misafir internet hizmeti şu an kapalı. Lütfen işletme görevlisine başvurun.", "Guest internet at this venue is currently unavailable. Please ask the staff."),
    ("hata_ad", "Adınızı harflerle yazın (2-40 karakter).", "Enter your first name using letters (2-40 characters)."),
    ("hata_soyad", "Soyadınızı harflerle yazın (2-40 karakter).", "Enter your last name using letters (2-40 characters)."),
    ("hata_telefon_tr", "Geçerli bir cep telefonu numarası girin (5XX XXX XX XX).", "Enter a valid mobile number (5XX XXX XX XX)."),
    ("hata_telefon", "Geçerli bir telefon numarası girin (ülke kodunu kontrol edin).", "Enter a valid phone number (check the country code)."),
    ("hata_sozlesme", "Devam etmek için İnternet Kullanıcı Sözleşmesi'ni kabul edin.", "Accept the Internet User Agreement to continue."),
    ("hata_riza", "Devam etmek için açık rıza onayını işaretleyin.", "Tick the explicit consent box to continue."),
    ("bekleme", "Yeni kod için {0} saniye bekleyin.", "Please wait {0} seconds before requesting a new code."),
    ("kod_az_once", "Kod az önce gönderildi. Yeni kod için {0} saniye bekleyin.", "A code was just sent. Please wait {0} seconds before requesting a new code."),
    ("tel_15dk", "Bu numaraya çok fazla kod gönderildi. 15 dakika sonra tekrar deneyin.", "Too many codes were sent to this number. Try again in 15 minutes."),
    ("tel_gun", "Bu numara için bugünkü kod sınırı doldu.", "The daily code limit for this number has been reached."),
    ("mac_saat", "Bu cihazdan çok fazla kod istendi. Bir saat sonra tekrar deneyin.", "Too many codes were requested from this device. Try again in an hour."),
    ("tavan", "Şu an SMS gönderilemiyor, lütfen personele başvurun.", "SMS cannot be sent right now, please ask the staff."),
    ("kod_yok_bilgi", "Kod bulunamadı, lütfen bilgilerinizi yeniden girin.", "No code found, please enter your details again."),
    ("kod_yok", "Kod bulunamadı, lütfen yeni kod isteyin.", "No code found, please request a new code."),
    ("kilit", "Çok fazla hatalı deneme yapıldı. Lütfen {0} dakika sonra tekrar deneyin.", "Too many incorrect attempts. Please try again in {0} minutes."),
    ("kod_suresi", "Kodun süresi doldu. Yeni kod isteyin.", "The code has expired. Request a new code."),
    ("dogrulanamadi", "Kod şu an doğrulanamadı, lütfen birkaç saniye sonra tekrar deneyin.", "The code could not be verified right now, please try again in a few seconds."),
    ("cok_hatali", "Çok fazla hatalı deneme. Yeni kod isteyin.", "Too many incorrect attempts. Request a new code."),
    ("kod_hatali", "Kod hatalı. Kalan deneme hakkı: {0}", "Incorrect code. Attempts left: {0}"),
    ("baglanti_acilamadi", "Bağlantı açılamadı, lütfen tekrar deneyin.", "The connection could not be opened, please try again."),
    // SMS sağlayıcı iletileri (sms.rs / twilio.rs Türkçe döndürür; `cevir` ile çevrilir)
    ("sms_yabanci", "Yabancı numaralara şu an SMS gönderilemiyor, lütfen personele başvurun.", "SMS cannot be sent to foreign numbers right now, please ask the staff."),
    ("sms_numara", "Bu numaraya SMS gönderilemiyor, lütfen numarayı kontrol edin.", "SMS cannot be sent to this number, please check the number."),
    ("sms_cok", "Çok fazla deneme yapıldı, lütfen biraz sonra tekrar deneyin.", "Too many attempts, please try again a little later."),
    ("sms_yanitsiz", "SMS servisi geçici olarak yanıt vermiyor, lütfen biraz sonra tekrar deneyin.", "The SMS service is temporarily not responding, please try again a little later."),
    ("sms_ag", "SMS servisine ulaşılamadı, lütfen personele başvurun.", "The SMS service could not be reached, please ask the staff."),
    ("sms_genel", "SMS gönderilemedi, lütfen personele başvurun.", "The SMS could not be sent, please ask the staff."),
    // düz metin HTTP cevapları
    ("cok_istek", "Çok fazla istek. Biraz bekleyip tekrar deneyin.", "Too many requests. Wait a moment and try again."),
    ("istek_buyuk", "İstek çok büyük.", "Request too large."),
    ("istek_gecersiz", "Geçersiz istek.", "Invalid request."),
    ("istek_desteklenmez", "Desteklenmeyen istek.", "Unsupported request."),
];

/// Anahtarın metni; İngilizcesi boşsa Türkçesi. Bilinmeyen anahtar: testte panik, üründe boş metin.
pub fn m(d: Dil, anahtar: &str) -> &'static str {
    match METINLER.iter().find(|(k, _, _)| *k == anahtar) {
        Some((_, tr, en)) => {
            if d == Dil::En && !en.is_empty() {
                en
            } else {
                tr
            }
        }
        None => {
            debug_assert!(false, "dil: bilinmeyen anahtar {anahtar}");
            ""
        }
    }
}

/// `{0}`, `{1}` … yer tutucularını doldurur.
pub fn f(d: Dil, anahtar: &str, args: &[&str]) -> String {
    args.iter().enumerate().fold(m(d, anahtar).to_string(), |s, (i, a)| s.replace(&format!("{{{i}}}"), a))
}

/// Başka modülün döndürdüğü Türkçe metni (sms.rs) çevirir; tabloda yoksa olduğu gibi.
pub fn cevir(d: Dil, tr_metin: &str) -> String {
    match METINLER.iter().find(|(_, tr, _)| *tr == tr_metin) {
        Some((k, _, _)) => m(d, k).to_string(),
        None => tr_metin.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dil_secimi_oncelik() {
        // sorgu > çerez > Accept-Language > tr
        assert_eq!(sec(Some("en"), Some("tr"), Some("tr-TR")), Dil::En);
        assert_eq!(sec(Some("tr"), Some("en"), Some("en-US")), Dil::Tr);
        assert_eq!(sec(None, Some("en"), Some("tr-TR,tr;q=0.9")), Dil::En);
        assert_eq!(sec(None, Some("tr"), Some("en-GB")), Dil::Tr);
        assert_eq!(sec(None, None, Some("en-US,en;q=0.9")), Dil::En);
        assert_eq!(sec(None, None, Some("EN")), Dil::En);
        assert_eq!(sec(None, None, Some("de-DE,en;q=0.5")), Dil::Tr);
        assert_eq!(sec(None, None, None), Dil::Tr);
        // geçersiz değer yok sayılır, sonraki kaynağa geçilir
        assert_eq!(sec(Some("fr"), Some("xx"), Some("en")), Dil::En);
        assert_eq!(sec(Some(""), Some("en"), None), Dil::En);
        // çerez başlığı
        assert_eq!(cerezden("a=1; dil=en; b=2"), Some("en"));
        assert_eq!(cerezden("xdil=en"), None);
        assert_eq!(cerezden(""), None);
        assert_eq!(cerez_basligi(Dil::En), "dil=en; Path=/; Max-Age=31536000; SameSite=Lax");
    }

    #[test]
    fn her_anahtar_iki_dilde() {
        let mut gorulen = std::collections::HashSet::new();
        for (k, tr, en) in METINLER {
            assert!(gorulen.insert(*k), "çift anahtar {k}");
            assert!(!tr.trim().is_empty(), "{k}: Türkçe boş");
            assert!(!en.trim().is_empty(), "{k}: İngilizce boş");
            assert!(!en.contains("kupon") && !tr.contains("kupon"), "{k}");
        }
        assert_eq!(m(Dil::En, "kod_gonder"), "Send code");
        assert_eq!(m(Dil::Tr, "kod_gonder"), "Kod Gönder");
        assert_eq!(f(Dil::En, "kod_hatali", &["4"]), "Incorrect code. Attempts left: 4");
        assert_eq!(cevir(Dil::En, crate::sms::YABANCI_YOK), "SMS cannot be sent to foreign numbers right now, please ask the staff.");
        assert_eq!(cevir(Dil::Tr, crate::sms::YABANCI_YOK), crate::sms::YABANCI_YOK);
        assert_eq!(cevir(Dil::En, "tabloda yok"), "tabloda yok");
        // sms.rs / twilio.rs'nin bütün kullanıcı iletileri tabloda
        for k in ["50", "80", "100", "AG", "?"] {
            assert_ne!(cevir(Dil::En, crate::sms::user_message(k)), crate::sms::user_message(k), "{k}");
        }
        for k in ["60200", "60203", "AG", "?"] {
            assert_ne!(cevir(Dil::En, crate::twilio::user_message(k)), crate::twilio::user_message(k), "{k}");
        }
    }
}
