# Doğrulama SMS'i merkezden — uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Misafir doğrulama SMS'i (NetGSM ve Twilio) cihazdan değil, sabit IP'li merkezden gönderilsin/doğrulansın.

**Architecture:** Merkez `api.py`'ye `/api/sms` ve `/api/sms-dogrula` uçları; gönderim `netgsm.py` (OTP ucu) ve yeni
`twilio.py` (Verify) ile, ayarlar `veri` ayar tablosunda (Yönetim → SMS ayarları). Cihazda `sms.rs`/`twilio.rs` sağlayıcı
çağrıları yerine merkez istemcisi; portal akışı ve denetim olayları aynı.

**Tech Stack:** Python stdlib + SQLite (merkez), Rust (cihaz; `crate::merkez::cagir`, `curl_sure`).

**Spec:** `docs/superpowers/specs/2026-10-13-sms-merkezden-design.md`

## Global Constraints

- Mesaj şablonu merkezde sabit: `WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir.` (ASCII). Cihaz serbest metin gönderemez.
- `kod` tam 6 rakam; `telefon` E.164 rakamları (`+` yok, 8–15 hane). Türk numarası = `90` + `5` ile başlayan 12 hane.
- Cihaz başına günlük üst sınır **300** (merkez sayacı, TR günü); aşılınca `{"sonuc":"hata","kod":"SINIR"}`.
- Yalnızca bağlı, üyeliği ve lisansı açık cihaz (`veri.hizmet_acik`); değilse 403.
- Yeniden deneme yok (ne cihaz ne merkez). Doğrulama kodu merkezde hiçbir yere yazılmaz (log, DB, hareket).
- Cihaz → merkez zaman aşımı **10 sn**. Merkeze ulaşılamazsa/bozuk cevap: misafire `Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin.`; cihaz doğrudan sağlayıcıya gitmez.
- Gerçek NetGSM/Twilio isteği `WC_TEST` altında hiç gitmez (mevcut kural, Twilio'ya da uygulanır).
- Cihaz denetim olayları aynı adlarla: `OTP_ISTEK`, `OTP_GONDERILDI`, `OTP_HATA` (ek `netgsm=<kod>` / `twilio=<kod>`, merkeze ulaşılamazsa `<sağlayıcı>=AG`).

## Review Focus

- Çalınmış cihaz anahtarıyla röle kötüye kullanımı: 6 hane dışı kod, Türk olmayan numara (Twilio kapalıyken), sınır aşımı → reddedilmeli (Task 1 testleri).
- Merkezde `mock` açıkken: gerçek istek yok, cevap `mock`; cihaz kodu yerelde doğrular (Task 1 + Task 2).
- Twilio ağ hatasında misafirin deneme hakkı düşmemeli (bugünkü davranış korunmalı) (Task 2).
- Eski sürüm cihaz (yeni uçları çağırmayan) bozulmamalı: merkez eşleşmede SMS bilgilerini göndermeye devam eder (Task 1).
- Kodun sızmaması: merkez hareketinde/stdout'unda kod yok; testte assert (Task 1).

---

### Task 1: Merkez — `/api/sms`, `/api/sms-dogrula`, Twilio ve ayarlar

**Files:**
- Modify: `scripts/sunucu/merkez/netgsm.py` (OTP gönderimi), `api.py`, `veri.py`, `yonetim.py` (SMS ayarları + müşteri sayfası sayaç)
- Create: `scripts/sunucu/merkez/twilio.py`, `scripts/sunucu/merkez/tests/test_sms_merkez.py`
- Modify: `scripts/sunucu/merkez/kur.sh` (twilio.py kurulur; mevcut modül testi bunu zaten denetler)

**Interfaces:**
- `netgsm.otp_gonder(ayar, telefon_10, metin, zaman_asimi=10, ac=None) -> (sonuc, kod, is)` — `sonuc` ∈ `gonderildi|mock|hata`; OTP ucu `https://api.netgsm.com.tr/sms/send/otp`, XML `mainbody/header{usercode,password,msgheader,appkey}/body{msg,no}` (cihazdaki `sms::build_xml` ile aynı biçim; cevap `main/code` + `main/jobID`). `ac=None` + `WC_TEST` → RuntimeError (mevcut kural).
- `twilio.gonder(ayar, telefon_e164, ac=None) -> (sonuc, kod)` ve `twilio.dogrula(ayar, telefon_e164, kod, ac=None) -> (onay: bool|None, kod)` — Verify v2 (`/Services/{verify_sid}/Verifications`, `/VerificationCheck`), Basic auth (auth token ya da API key sid+secret), `Locale` Türk numarasında `tr`, diğerlerinde `en`; ağ hatasında `onay=None, kod="AG"`.
- `veri`: `sms_ayari()` genişler: `tr_saglayici` (`netgsm`|`twilio`, varsayılan netgsm), `twilio: {acik, account_sid, auth_token, api_key_sid, api_key_secret, verify_sid}`; `sms_sayac_artir(musteri, gun) -> int`, `sms_sayac(musteri, gun) -> int` (yeni tablo). Mevcut cihaza giden `_cihaz_ekleri` NetGSM alanları **değişmez** (eski cihazlar).
- `api`: `/api/sms` `{cihaz_anahtari, telefon, kod}` → `{"sonuc", "kod", "is"?, "saglayici"}`; `/api/sms-dogrula` `{cihaz_anahtari, telefon, kod}` → `{"onay": bool}` ya da `{"sonuc":"hata","kod":"AG"}`. Yönlendirme: TR → `tr_saglayici`; yabancı → Twilio açıksa Twilio, değilse `hata/YABANCI`. Twilio'ya giden gönderimde cihaz kendi kodunu kullanmaz (cevapta `saglayici:"twilio"`). Hareket: `SMS_OTP` ayrıntı `tel=**<son 2> saglayici=<…> sonuc=<…>` (kod yok). Gövde sınırı mevcut 4 KB, IP hız sınırı mevcut.
- `yonetim`: SMS ayarları sayfasına Twilio alanları (yazılır-okunmaz sır alanları, mevcut parola alanı kalıbı) ve "Türk numaraları için sağlayıcı" seçimi; müşteri sayfasında "Bugün doğrulama SMS'i: N".

- [ ] **Step 1: Testleri yaz** (`test_sms_merkez.py`, sahte `ac`): NetGSM OTP XML gövdesi ve `code=0`→gonderildi + jobID; mock → gerçek istek yok, `mock`; 5 haneli kod → 400; `+90` / harfli telefon → 400; yabancı numara Twilio kapalı → `YABANCI`; Twilio açık → Verify çağrısı, `saglayici=twilio`; `tr_saglayici=twilio` TR numarasını Twilio'ya yollar; günlük 300 sınır (`sms_sayac` 300'deyken → `SINIR`, istek gitmez); askıda/lisansı bitmiş/serbest cihaz → 403; `/api/sms-dogrula` approved→`onay:true`, pending→false, ağ hatası→`AG`; hareket satırında ve yakalanan stdout'ta kod geçmiyor; yönetim SMS ayarlarında Twilio sırrı sayfada görünmüyor, boş bırakılınca eskisi korunuyor, CSRF'siz 403.
- [ ] **Step 2:** `sh scripts/sunucu/merkez/tests/hepsi.sh <python>` → yeni testler FAIL.
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** Bütün merkez testleri `TUM TESTLER GECTI`.
- [ ] **Step 5: Commit** `merkez: dogrulama SMS'i merkezden (NetGSM OTP + Twilio Verify), /api/sms ve /api/sms-dogrula`

### Task 2: Cihaz — sağlayıcı çağrıları merkeze

**Files:**
- Modify: `src/sms.rs`, `src/twilio.rs`, `src/merkez.rs` (istemci işlevleri), `src/portal.rs` (yalnızca gerekirse), `src/ayar.rs` (`[twilio]`, `[sms] provider`, `[netgsm]` sırlarının kullanımı kalkar; eski dosyalar okunmaya devam eder), `src/panel.rs` (Admin ayarları NetGSM/Twilio alanları kalkar, not), `src/saglik.rs` (`sms_sorunu`: merkez bağlı değilse ya da merkezden gelen mock bilgisi)

**Interfaces:**
- Consumes: Task 1 uçları ve cevap biçimleri.
- `merkez::sms_gonder(m: &Merkez, telefon: &str, kod: &str, http: &Http) -> Result<SmsCevap, String>`; `merkez::sms_dogrula(m, telefon, kod, http) -> Result<Option<bool>, String>` (None = AG). `SmsCevap { sonuc: String, kod: String, is: Option<String>, saglayici: String }`.
- `sms::send` imzası portalın beklediğiyle uyumlu kalır (portal `Box::new(sms::send)` / `Box::new(crate::twilio::check)` ile çağırıyor); içleri merkez istemcisine döner. Cihaz merkeze bağlı değilse (`merkez.json` yok) → `Sonuc { ok:false, kod:"AG" }`.
- `sms::route` sağlayıcıyı artık cihaz ayarından seçmez: TR → gönderim merkeze; merkez `saglayici:"twilio"` dönerse portal o oturumda Twilio doğrulama yolunu (merkezde `/api/sms-dogrula`) kullanır; `YABANCI` → mevcut `YABANCI_YOK` metni.
- `sms::user_message`: `AG`, `SINIR`, bozuk cevap → `Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin.`; diğer NetGSM kodları bugünkü metinler.

- [ ] **Step 1: Testleri yaz:** sahte `Http` ile: başarı → `OTP_GONDERILDI is=<jobID>`; merkez `mock` → kod günlükte, yerel doğrulama çalışır; ağ hatası → misafir metni "Şu an kod gönderilemiyor…", `OTP_HATA netgsm=AG`, sağlayıcıya doğrudan istek yok (ureq çağrısı kalmadığını derleme düzeyinde/grep testiyle); `SINIR` → aynı metin; Twilio yolu: `saglayici=twilio` → doğrulama merkezden, `AG`'de deneme hakkı düşmez; yabancı numara `YABANCI` → mevcut metin; Admin ayarları sayfasında `netgsm.password`/`twilio.auth_token` alanı yok, not var.
- [ ] **Step 2:** `bash scripts/test.sh wificorrect sms` / `portal` / `twilio` → FAIL.
- [ ] **Step 3:** Uygula; `send_netgsm` ve Twilio HTTP kodu silinir (ölü kod kalmasın).
- [ ] **Step 4:** Süzgeçler, sonra tam takım PASS (yeni uyarı yok).
- [ ] **Step 5: Commit** `cihaz: dogrulama SMS'i merkezden; NetGSM ve Twilio cihazdan kalkti`

### Task 3: Sürüm, belgeler

- [ ] Cargo sürümü **0.4.8** (Cargo.toml + yalnızca `wificorrect` girdisi Cargo.lock). `docs/KURULUM_GUNLUGU.md` sonuna kısa bölüm "SMS merkezden (2026-10-13)": akış, uçlar, sınırlar, geçiş sırası, eski cihazlara NetGSM bilgisinin geçici gönderimi. Commit `surum 0.4.8, SMS merkezden belgesi`.

### Task 4: Kurulum ve uçtan uca (denetleyici + kullanıcı)

- [ ] Merkez: kullanıcı `merkez-guncelle.sh`; Yönetim → SMS ayarları'nda Twilio alanlarını kullanıcı doldurur (isterse).
- [ ] Cihaz: PR → birleştir → `v0.4.8` etiketi → Sürümler → GitHub'dan al → test hedefi + Şimdi güncelle.
- [ ] Merkez **deneme modu açıkken**: sahte istemciyle kod iste → merkez `mock`, cihaz günlüğünde kod, giriş başarılı.
- [ ] Merkezi geçici olarak engelle (cihazda `nft` ile api adresine giden 443'ü 1 dk düşür, ölü adam anahtarıyla) → misafir metni "Şu an kod gönderilemiyor…".
- [ ] Gerçek SMS: yalnızca kullanıcının açık onayı ve verdiği numarayla, deneme modu kapatılarak tek SMS; sonra durum eski hâline.
