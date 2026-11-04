# Merkezi Rust'a taşıma ve kod sadeleştirme — tasarım (2026-11-04)

## İstek

Kullanıcı (2026-11-04): "projeyi tekrar analiz et pyton betiği kullanan bütün kod satırlarını rust'a çevir ve hepsini rust'a
çevir. kodları sadeleştir. çalışmayan fonksiyonu bulunmayan ve geçmişten kalan bütün kod satırlarını sil. istisnalar gelecekte
yapacaklarımıza dair bir şey varsa yok sayabilirsin. bunu planlı bir şekilde yap kayıp istemiyorum."

Sorulara cevap (2026-11-04): silinecek özellik olarak merkez raporları, eski arşivler sayfası ve Twilio sorulduğunda **hiçbiri
seçilmedi**: üçü de olduğu gibi taşınır. Yürütme: **görev başına ajan + denetçi** (subagent-driven development).

## Bugünkü durum (2026-11-04 analizi)

- **Python yalnızca yönetim merkezinde:** `scripts/sunucu/merkez/` 24 modül, 7.401 satır kod; 24 test dosyası, 7.821 satır,
  365 test. Ayrıca `.github/workflows/test.yml` ve `release.yml` merkez testlerini Python'la çalıştırır, `scripts/iso-test.sh`
  iki satır içi Python yardımcısı kullanır (QEMU izleme soketi, PPM→PNG).
- **Cihaz tamamen Rust:** `src/` 33.457 satır. Derleyici (test dışı derleme, cihazda `cargo check`) yalnızca 4 kullanılmayan öğe
  bildirir: `logo::LOGO_KOYU`, `logo::marka_koyu`, `merkez::curl_args`, `ulkeler::is_tr`.
- **Canlı:** iki cihaz 0.4.29 (Göztepe test, Visavis kararlı). Merkez gbservers'ta (Ubuntu 26.04.1, glibc 2.43, Python 3.14),
  Caddy arkasında 127.0.0.1:8081.

## Amaç

1. Python'u projeden tamamen çıkarmak: merkez Rust'a; CI ve `iso-test.sh` Python'suz.
2. Ölü ve geçmişten kalan kodu silmek (merkezde taşınmaz, cihazda ayrı sürümle silinir).
3. Davranış aynı kalır: aynı adresler ve alan adları, aynı sayfalar (HTML birebir), aynı API cevapları, aynı veritabanı ve
   dosyalar, aynı günlük satırları (fail2ban).

## Hedef dışı

- Yeni özellik ya da arayüz metni değişikliği yok (bilinçli farklar aşağıda, yalnızca silinen eski kod).
- Bash betikleri Python değildir, kalır: `wificorrect-sunucu`, `kur.sh`, deneme betikleri, `hotspot-arsiv-saklama`
  (yalnızca Python çağıran satırları değişir).
- Veritabanı şeması ve dosya biçimleri değişmez; veri taşıma yok.
- VPS taşıması ayrı iş; bu iş bitince Rust merkez her sunucuya kurulabilir olur.
- Cihaz ↔ merkez API sözleşmesi değişmez; yalnızca eski cihazlar için kalmış uç ve alanlar kalkar.

## Kayıp olmaması için güvenceler

1. **Testler:** 365 Python testi Rust'a aynı senaryo ve aynı beklentiyle taşınır. Taşınmayan her test (logonun tek kopyaya
   inmesiyle gereksizleşen 5 logo testi, silinen eski uçların testleri) plan defterinde adıyla ve nedeniyle yazılır.
2. **Altın senaryolar:** Python merkez silinmeden önce geçici bir Python çalıştırıcısı (`scripts/sunucu/parite/`) senaryo
   dosyalarındaki adımları (kurulum, istek, arka plan işi) sabit saat ve belirlenimli rastgelelikle Python modülleri üzerinde
   çalıştırır, sonuçları `merkez/tests/altin/`a yazar. Rust aynı senaryolarda aynı sonucu vermelidir:
   - HTML bayt bayt aynı (rastgele belirteçler ilk görünüş sırasıyla yer tutucuya çevrilerek);
   - JSON (API cevapları, üretilen dosyalar) değer olarak aynı;
   - veritabanı tablo tablo, satır satır aynı.
   Her sayfa, her API ucu ve her arka plan işi en az bir senaryoda bulunur.
3. **Canlı veri denetimi:** geçişten önce `wificorrect-merkez veritabani-denetle` canlı `merkez.db`'yi yalnızca okunur açar:
   - her tablo ve sütun var mı;
   - her satır Rust'ın satır okuyucusundan geçiyor mu;
   - JSON sütunları ve `/var/lib/wificorrect` altındaki dosyalar ayrıştırılıyor mu.
   Çıktı yalnızca sayılar ve hata yeri (tablo + satır kimliği + neden); hiçbir değer yazılmaz. Tek hata varsa geçiş yapılmaz.
4. **Geçiş ve geri dönüş:**
   - Aynı port (127.0.0.1:8081), aynı birim adları ve kullanıcıları; Caddyfile değişmez.
   - Kurulum veritabanını yedekler ve Python dosyalarına dokunmaz.
   - Geri dönüş betiği eski birim dosyalarını geri koyar (tek komut).
   - Geri dönüş izleme haftası boyunca geçerlidir. Rust aynı şema ve biçimlerle yazar. Python'un, Rust'ın yazdığı veritabanını ve
     dosyaları okuyabildiğini bir geri okuma testi kanıtlar (plan Task 13). Bu test geçmeden canlıya geçilmez.
   - Python, Rust bir hafta sorunsuz çalıştıktan sonra kullanıcının onayıyla silinir.
5. **Dış yan etki yok:** testlerde gerçek SMS, NetGSM, Twilio ya da GitHub isteği gitmez (sahte taşıyıcı; `WC_TEST=1` iken gerçek
   istek hata verir).
6. **Cihaz ayrı sürümle:** cihaz değişiklikleri 0.4.30 olarak önce Göztepe'ye (test kanalı), sorunsuzsa Visavis'e gider.

## Mimari kararlar (benim kararlarım, kullanıcı değiştirebilir)

- **Ayrı crate:** `merkez/` dizini, paket ve ikili `wificorrect-merkez`, kendi `Cargo.lock`'u. Kök `.cargo/config.toml`
  geçerli: Rust 1.85 ile uyumlu en yeni kütüphane sürümleri seçilir. Cihaz crate'i, betikleri ve CI işleri dokunulmadan çalışır.
- **Kütüphaneler:**
  - Cihazdakilerin aynısı: tiny_http, serde/serde_json, sha2, pbkdf2 (hmac), flate2, getrandom, libc.
  - Ek olarak: rusqlite (`bundled` SQLite), hmac (rapor takma kimliği), chrono (sabit +03:00 takvim hesabı: gün ekleme, ISO
    hafta, ay).
- **Dış istekler** curl alt süreciyle yapılır (cihazdaki gibi). Sırlar (NetGSM parolası, Twilio anahtarı) ve istek gövdesi
  komut satırına yazılmaz, stdin'den verilir.
- **Modüller 1:1:** Python modülleri aynı adla Rust modülüne geçer. Ek olarak:
  - `ortak` (common.py'nin merkezin gerçekten kullandığı 9 yardımcısı);
  - `sunucu` (HTTP).
  Uygulamalar Python'daki gibi saf bir arayüzle yazılır (`istek(Istek) -> Yanit`); testler doğrudan bunu çağırır.
- **Tek ikili, alt komutlar:**
  - `sunucu`, `durum`, `istatistik`, `kuyruk`, `saglik`, `listeler`;
  - `surumler al`, `surumler ekle DOSYA SURUM`;
  - `yonetici parola AD`, `veritabani-denetle VT`, `surum`.
  Birimler (wc-merkez, wc-durum, …) aynı kullanıcı ve kısıtlarla bu komutları çağırır. `/usr/local/sbin/wc-*` sembolik bağları
  kalkar; `wificorrect-sunucu yonetici-parola` Rust komutunu çağırır.
- **Logo tek kopya:** merkez `src/sablon/logo/`'yu `include_str!`/`include_bytes!` ile gömer. Merkezdeki ikinci kopya ve
  `test_logo.py` silinir.
- **Testler:** cihazda (`scripts/test-merkez.sh`, cihaz testleri gibi bellek sınırlı) ve CI'da (`merkez-rust` işi) çalışır.
  Python işi, Python silinene kadar yanında kalır.
- **Kurulum:**
  - Tek betik, `scripts/sunucu/merkez-kur.sh` (bugünkü `dagit.sh` + `merkez-guncelle.sh` yerine); boş sunucuda da çalışır, VPS
    için de kullanılır.
  - Merkez ikilisini CI derler (ubuntu-24.04, glibc 2.39: Ubuntu 24.04+ ve Debian 13'te çalışır). Her push'ta test edilip
    commit'e özel yapıt olarak yüklenir; etiketlerde sürüm paketine girer.
  - `merkez-kur.sh` şu anki commit'in başarılı CI yapıtını `gh run download` ile indirir, sha256'sını doğrular ve kurar. Bugünkü
    "şu anki daldan kur" alışkanlığı sürer; sunucuda ve cihazda derleme gerekmez.

## Silinecek eski kod — kanıtıyla

### Merkez (taşınmaz)

- **common.py'nin OpenWrt'den kalan kısmı:** oturum, nft, kira/ARP, 5651 CSV yazma, UCI, SMS sayacı vb. Merkezde çağıran yok
  (AST taraması, 2026-11-04). Yalnızca şunlar taşınır: `TZ`, `now_iso`, `tarih_goster`, `tarih_saat_goster`, `tarih_girdi`,
  `save_json`, `read_rows`, `normalize_phone`, `saglayici_istegi`.
- **`/api/raporlar` ve `/api/rapor`:** cihaz 0.4.28'den beri çağırmıyor (`src/` taraması); iki cihaz 0.4.29.
- **Eşleşmede `sms.mock`, `sms.provider`, `sms.netgsm`:** 0.4.8 öncesi cihazlar içindi. Bugün NetGSM kullanıcı kodu ve
  **parolası** her eşleşmede kolay bağlamayla bağlanmamış her cihaza gidiyor; cihaz yalnızca `etkin_mock`'u okur
  (`src/merkez.rs` `sms_mock_oku`). `etkin_mock` kalır.
- **`cihaz.otomatik`'e bağlı "NetGSM gitmez" istisnası:** yukarıdakiyle birlikte gereksizleşir (sütun şemada kalır).
- **`ALTER TABLE` geçişleri ve `_gocur` veri düzeltmeleri:** tam şema `CREATE TABLE`'da; canlı veritabanı denetimle
  doğrulanır. Boş `lisans_bitis` ya da `ap_parola` bulunursa denetim hata verir ve düzeltme o zaman eklenir.
- **`kur.sh`'deki eski kafe paneli satırları** (`panel.py`, `wc-panel`, `hesaplar.json.eski`, `wc-hesap`) ve
  `PYTHONDONTWRITEBYTECODE`.
- **Kullanılmayan kod:** `api.BELIRTEC_RE`; yalnızca testte kullanılan `Veri.parola_sifirla`.
- **Rapor biçim 1 okuyucuları** (`raporlar.OKUNAN_BICIMLER`'deki 1, `rapor.py` eski biçim temizliği): yalnızca canlı envanterde
  biçim 1 dosya yoksa silinir.

### Cihaz (0.4.30)

- **Derleyicinin bildirdiği 4 öğe:** `logo::LOGO_KOYU`, `logo::marka_koyu`, `merkez::curl_args`, `ulkeler::is_tr`.
- **`ctl ayar-temizle`, `Config::eski_sirlari_temizle` ve postinst satırı:** iki cihazda `[netgsm]`/`[twilio]` bölümü ve
  `sms.provider`/`sms.merkez` anahtarı yok (2026-11-04 sayımı: 0). 0.4.8 sonrası ISO'larda hiç olmadı.
- **`PortalMetin::eski_varsayilani_guncelle`, `ESKI_AYDINLATMA_SHA`, `tests/fixtures/aydinlatma-0.4.11.txt`:** iki cihazda da
  kayıtlı metin eski varsayılan değil (sha karşılaştırması).
- **`ctl ap-adres-kaldir`'daki 0.4.11 geçici adres temizliği:** iki cihazda `br-hotspot:ap` adresi yok.
- **Merkez eski sürümdeyken `/_ap-gecis`'i karşılayan cihaz sayfası:** Caddy bu yolu hep merkeze verir.
- **Modül başlıklarındaki eski sistemden kalan yorumlar:** "eski ctl.py … Bu adımda … sonraki adımlarda" gibi, artık doğru
  olmayanlar.

## Kalacaklar (eski görünür ama gerekli)

- **Eski denetim satırı biçimi:** panel hareketlerinde bu biçimi okuyan kod kalır. 2 yıllık saklama süresindeki kayıtlarda o
  biçim var.
- **0.4.28 öncesi bağlı kalma tahmini:** Ekim günlerinin raporları buna dayanıyor.
- **İsteğe bağlı alanlara tolerans:** cihazın merkez cevabındaki isteğe bağlı alanları hoş görmesi savunmadır, eski sürüm desteği
  değil.
- **Eski ISO'yla kurulan cihazın yolu:** bağlanıp kendini güncelleyebilmesi için bağlanma, sağlık ve güncelleme uçları kalır.
- **Gelecek işlere ait olanlar (kullanıcının istisnası):** müşteri başına AP adresi notları, personel kodu, zaman damgası,
  ödeme. Dokunulmaz.

## Bilinçli farklar (tek liste; altın senaryolar bunları dışarıda bırakır)

1. `/api/raporlar` ve `/api/rapor` yok (404).
2. Eşleşme cevabında `sms` yalnızca `etkin_mock` taşır.
3. Birimler `wificorrect-merkez`'i çağırır; `/usr/local/sbin/wc-*` bağları yok.
4. HTTP sunucusunun kendi başlıkları (`Server`, `Date`) Python'unkinden farklı olabilir: Caddy `Server`'ı zaten siler.
5. Telefon numarası düzeltmesi (`normalize_phone`) yalnızca ASCII rakamları kabul eder. Python başka yazıların rakamlarını da
   kabul ediyor ve NetGSM'in kullanamayacağı bir numara üretiyordu.

Başka her çıktı aynıdır.

## Geçiş sırası

1. **Kod ve testler:** canlıya dokunmaz.
2. **Canlı envanter ve veritabanı denetimi:** kullanıcı sudo ile çalıştırır; çıktı yalnızca yapı ve sayı.
3. **Kurulum** (kullanıcı sudo): yedek → denetim → Rust birimleri → doğrulama. Doğrulananlar:
   - yönetime giriş;
   - iki cihazın sağlık raporu;
   - kuyruk;
   - eşleşme;
   - SMS gönderimi yok.
4. **Bir hafta izleme:** sağlık kartları, hareketler, fail2ban, zamanlayıcılar. Sorun yoksa Python silinir (kullanıcı onayı).
5. **Cihaz sadeleştirme sürümü 0.4.30.**

## Uygulayıcıya kalanlar

- Rust iç yapısı (modül içi bölümleme, tip adları).
- Test yardımcılarının biçimi.
- curl çağrı ayrıntıları.
- Altın senaryo dosyalarının iç biçimi (plan arayüzü verir).
