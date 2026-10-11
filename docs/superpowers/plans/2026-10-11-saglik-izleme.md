# Merkezde cihaz sağlık izleme ve SMS uyarısı — Uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cihazlar 5 dk'da bir sağlık raporu gönderir; merkez 5 dk'da bir kuralları değerlendirir, 10 dk süren sorunda hizmet
sağlayıcıya SMS atar, düzelince haber verir; Yönetim'de açık sorunlar, sağlık raporu, sorun geçmişi ve uyarı ayarları görünür.

**Architecture:** Merkez (Python stdlib + SQLite, `scripts/sunucu/merkez/`): `veri.py`'ye `saglik` ve `sorun` tabloları ve
`uyari` ayarı; `api.py`'ye `POST /api/saglik`; yeni `netgsm.py` (genel SMS ucu) ve `saglik.py` (kurallar + SMS kararı,
`wc-saglik.timer` ile 5 dk'da bir, wcpanel kullanıcısı); `yonetim.py`'ye sayfalar. Cihaz (Rust): yeni `src/saglik.rs` raporu
toplar, `ctl saglik` gönderir, kaydedici 5 dk'da bir `systemd-run` ile başlatır.

**Tech Stack:** Python 3 stdlib (urllib, sqlite3), Rust (`crate::merkez::cagir`/`curl`), systemd timer.

**Spec:** `docs/superpowers/specs/2026-10-11-saglik-izleme-design.md`

## Global Constraints

- Merkez testleri yerelde: `sh scripts/sunucu/merkez/tests/hepsi.sh /c/Users/hp/AppData/Local/Python/bin/python3.exe` (tek dosya: `… python3.exe -I scripts/sunucu/merkez/tests/test_X.py`). Cihaz testleri: `bash scripts/test.sh wificorrect <süzgeç>`.
- Rapor aralığı 300 sn; "Rapor gelmiyor" eşiği 900 sn; "Ulaşılamıyor" eşiği 600 sn (`yonetim.CEVRIMICI_SN` ile aynı); SMS bekleme 600 sn; günlük SMS üst sınırı varsayılan 20; telefon en çok 3; disk eşiği %90; lisans uyarısı ≤ 15 gün; bilgi önemli SMS'ler günde bir kez 09:00 (TR saati) toplu.
- Sorun türleri ve önemleri (kimlik, metin): `ulasilamiyor` "Cihaz ulaşılamıyor" kritik; `rapor_yok` "Rapor gelmiyor" kritik; `kayit_durdu` "5651 kaydı durdu" kritik; `servis_durdu` "Servis durdu: <adlar>" kritik; `yedek_gecikti` "Yedek gecikti" uyarı; `disk_dolu` "Disk dolu (%N)" uyarı; `sms_calismiyor` "SMS çalışmıyor" uyarı; `lisans_bitiyor` "Lisans bitiyor (<tarih>)" bilgi.
- SMS metni: `WifiCorrect <numara> <işletme adı>: <sorun metni> (<SS:DD>'ten beri)`; düzelince `WifiCorrect <numara> <işletme adı>: <sorun metni> düzeldi (<N> dk sürdü)`; üst sınır: `WifiCorrect: günlük SMS üst sınırına (<N>) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek.`
- Uyarı SMS'i NetGSM'in **genel SMS ucu**yla gider (`https://api.netgsm.com.tr/sms/send/xml`, XML `mainbody/header/body`, `type 1:n`), OTP ucu (`/sms/send/otp`) kullanılmaz. Kimlik bilgisi merkezdeki `sms_ayari` (usercode/password/msgheader). `mock` açıksa gönderilmez, yalnızca kaydedilir. Yeniden deneme yok.
- Raporda kişisel veri yok (ad, telefon, MAC yok). Serbest/askıda müşteriye uyarı yok; lisansı bitmiş müşteride yalnızca lisans bilgisi.
- **Gerçek SMS yalnızca kullanıcı açıkça onaylarsa** gönderilir (deneme SMS'i dahil); testler sahte göndericiyle.
- Cihazda rapor gönderimi kaydedici döngüsünü beklemez (`systemd-run`), istek 10 sn zaman aşımlı; başarısızlık sessiz.

## Review Focus

- **Titreyen bağlantı** (el sıkışma 9–11 dk arası gidip gelir): SMS yağmuru olmamalı; aynı sorun açık kaldıkça tek SMS, kapanıp 600 sn içinde yeniden açılırsa yeni SMS yok → Task 3 testi `titreyen_baglanti_tek_sms`.
- **Merkez kendi yeniden başlarsa / timer kaçarsa**: açık sorunlar veritabanında olduğu için çift SMS gitmemeli → Task 3 testi `yeniden_calistirmada_cift_sms_yok`.
- **Bozuk/eksik rapor gövdesi** (alan eksik, tip yanlış, 4 KB üstü, tanınmayan anahtar): 400/401, veritabanına yazılmaz → Task 2 testi `bozuk_rapor_reddedilir`.
- **NetGSM hatası/erişilemez**: SMS "gönderilemedi" kaydedilir, sorun yine açık görünür, bir sonraki turda tekrar SMS denenmez (yeniden deneme yok) ama Yönetim'de "SMS gönderilemedi" görünür → Task 3 testi `sms_hatasi_kaydedilir`.
- **Cihaz yeni kuruldu, hiç rapor yok** (eski sürüm cihaz): `rapor_yok` yalnızca en az bir rapor gelmiş cihazda açılır; eski sürümü uyarı yağmuruna boğmaz → Task 3 testi `hic_rapor_yoksa_rapor_yok_acilmaz`.

---

### Task 1: Veri katmanı — `saglik`, `sorun` tabloları ve `uyari` ayarı

**Files:** Modify `scripts/sunucu/merkez/veri.py`; Test `scripts/sunucu/merkez/tests/test_saglik.py` (yeni)

**Interfaces — Produces (veri.Veri yöntemleri):**
- `saglik_kaydet(cid: int, rapor: dict) -> None` (cihaz başına tek satır: `cihaz`, `zaman` ISO, `rapor` JSON metni; üzerine yazar)
- `saglik(cid: int) -> dict | None` → `{"zaman": str, "rapor": dict}`
- `sorun_ac(musteri: int, tur: str, metin: str, onem: str) -> int` (aynı müşteri+tür açık sorun varsa onun id'si, yenisi açılmaz)
- `sorun_kapat(sorun_id: int) -> None`; `sorun_sms(sorun_id: int, alan: str, deger: str) -> None` (`alan` ∈ `acilis_sms`, `kapanis_sms`; değer `gonderildi|mock|hata:<kod>|sinir`)
- `acik_sorunlar(musteri: int | None = None) -> list[dict]`; `sorun_gecmisi(musteri: int, limit: int = 50) -> list[dict]`; `son_kapanan(musteri: int, tur: str) -> dict | None`
- `uyari_ayari() -> dict` (varsayılan `{"telefonlar": [], "acik": False, "gunluk_sinir": 20}`), `uyari_ayari_koy(d: dict) -> None` (telefonlar `common.normalize_phone` ile 90… biçimi, en çok 3, geçersiz → `ValueError`)
- `sms_sayaci(gun: str) -> int`, `sms_say(gun: str) -> int` (artırır, yeni değeri döner; `ayar` tablosunda `sms_sayac:<gun>`)
- Şema: `sorun(id, musteri, tur, metin, onem, acilis, kapanis DEFAULT '', acilis_sms DEFAULT '', kapanis_sms DEFAULT '')`, indeks `(musteri, kapanis)`; `saglik(cihaz PRIMARY KEY, zaman, rapor)`. `_gocur` mevcut kalıpla.

- [ ] Step 1: Failing tests in `test_saglik.py` (dosyanın kalıbı `test_durum.py` gibi, `if __name__ == "__main__"` ile testleri çalıştıran): `veri_saglik_tek_satir`, `sorun_ac_ayni_turu_tekrar_acmaz`, `sorun_kapat_ve_gecmis`, `uyari_ayari_dogrulama` (4 telefon → ValueError, `0533 455 31 32` → `905334553132`), `sms_sayaci_gun_bazli`.
- [ ] Step 2: Run `…/python3.exe -I scripts/sunucu/merkez/tests/test_saglik.py` → FAIL.
- [ ] Step 3: Implement.
- [ ] Step 4: Run → PASS; `hepsi.sh` → PASS.
- [ ] Step 5: Commit `merkez: sağlık raporu, sorun kaydı ve uyarı ayarı (veri)`.

### Task 2: `POST /api/saglik`

**Files:** Modify `scripts/sunucu/merkez/api.py`; Test `scripts/sunucu/merkez/tests/test_api.py`

**Interfaces:**
- Consumes: `veri.saglik_kaydet`, `veri.cihaz_anahtarla`.
- Produces: gövde `{"cihaz_anahtari": str, "rapor": {...}}`; rapor alanları (hepsi zorunlu, tipleri sıkı): `surum` str ≤ 40, `acik_sn` int ≥ 0, `servisler` dict {`portal`,`panel`,`kaydedici`,`dnsmasq`,`guvenlik`: bool}, `disk_yuzde` int 0–100, `son_muhur` str (`YYYY-MM-DD` ya da ""), `kayit_boslugu_1s` bool, `wan` bool, `sms_sorunu` bool, `bagli` int ≥ 0, `hiz_hata` bool. Bilinmeyen alanlar atılır. Yanıt `200 {"tamam": true}`; tanınmayan anahtar 401 `taninmadi`; bozuk alan 400 `gecersiz`. Serbest bırakılan cihaz 200 ama yazılmaz.
- [ ] Step 1: Failing tests: `saglik_raporu_kaydedilir`, `bozuk_rapor_reddedilir` (eksik alan, `disk_yuzde: 101`, `servisler` liste, bilinmeyen anahtar → 401), mevcut 4 KB ve hız sınırı testleri yeni uca da uyuyor.
- [ ] Step 2–4: RED, implement (`uclar` sözlüğüne ekle), GREEN, `hepsi.sh`.
- [ ] Step 5: Commit `merkez: cihaz sağlık raporu ucu`.

### Task 3: `netgsm.py` ve `saglik.py` — kurallar, SMS kararı, timer

**Files:** Create `scripts/sunucu/merkez/netgsm.py`, `scripts/sunucu/merkez/saglik.py`, `scripts/sunucu/merkez/wc-saglik.service`, `scripts/sunucu/merkez/wc-saglik.timer`; Modify `scripts/sunucu/merkez/kur.sh` (birimleri kur, `/usr/local/sbin/wc-saglik` bağı mevcut kalıpla, timer'ı etkinleştir); Test `scripts/sunucu/merkez/tests/test_saglik.py`

**Interfaces:**
- `netgsm.xml_govde(usercode, password, msgheader, metin, numaralar: list[str]) -> str` (XML kaçışlı, `<no>` 10 haneli 5XXXXXXXXX), `netgsm.yanit_kodu(metin) -> str` (`"00 123"` → `"00"`, XML `<code>`), `netgsm.gonder(ayar: dict, metin: str, numaralar: list[str], zaman_asimi=10) -> str` (`"gonderildi"` / `"mock"` / `"hata:<kod>"`; `ayar["mock"]` → ağ yok).
- `saglik.sorunlar(m: dict, c: dict | None, rapor: dict | None, rapor_zaman: float | None, el_sikisma: float | None, son_gun: str | None, simdi: float) -> list[tuple[tur, metin, onem]]` — saf fonksiyon, Global Constraints'teki kurallar ve eşikler. `rapor_yok` yalnızca `rapor_zaman` varsa ve el sıkışma taze iken. `ulasilamiyor` varken `rapor_yok`/`kayit_durdu`/`servis_durdu` üretilmez (aynı kök neden).
- `saglik.calistir(veri, durum: dict, gonder=netgsm.gonder, simdi: float | None = None) -> None` — her bağlı müşteri için `sorunlar` → yeni olanları `sorun_ac`, artık olmayanları `sorun_kapat`; SMS: kritik/uyarı sorun **600 sn açık kaldıysa** ve `acilis_sms` boşsa gönder; kapanan sorunun `acilis_sms` gönderilmişse kapanış SMS'i; aynı tür 600 sn içinde yeniden açılırsa (`son_kapanan`) açılış SMS'i gönderilmez ve kapanışınki de gönderilmez; `bilgi` önemliler yalnızca TR saatiyle 09:00–09:04 turunda toplu tek SMS; günlük sınır dolunca tek "üst sınır" SMS'i, sonra `sinir`. `uyari_ayari.acik` false ya da telefon yoksa SMS yok, sorun kaydı yine tutulur. Her SMS `veri.hareket("saglik", "-", "UYARI_SMS", musteri, …)`.
- `saglik.main()` — `durum.json` (`durum.DURUM`) ve DB (`merkez.DB`) ile `calistir`; çıkış kodu 0.
- `wc-saglik.service`: `User=wcpanel`, `Group=wcpanel`, `Type=oneshot`, `ExecStart=/usr/local/sbin/wc-saglik`, `TimeoutStartSec=2min`; timer `OnBootSec=3min`, `OnUnitActiveSec=5min`.
- [ ] Step 1: Failing tests: `netgsm_xml_ve_yanit`, `kural_ulasilamiyor_diger_kritikleri_bastirir`, `kural_esikleri` (disk 89/90, lisans 16/15 gün, rapor 899/901 sn), `hic_rapor_yoksa_rapor_yok_acilmaz`, `sms_10_dk_sonra_ve_duzeldi`, `titreyen_baglanti_tek_sms`, `yeniden_calistirmada_cift_sms_yok`, `gunluk_sinir`, `bilgi_09da_toplu`, `sms_hatasi_kaydedilir`, `serbest_askida_uyarisiz`, `uyari_kapaliyken_sms_yok_kayit_var`.
- [ ] Step 2–4: RED, implement, GREEN, `hepsi.sh`.
- [ ] Step 5: Commit `merkez: sağlık kuralları, NetGSM uyarı SMS'i, wc-saglik zamanlayıcısı`.

### Task 4: Yönetim sayfaları

**Files:** Modify `scripts/sunucu/merkez/yonetim.py`; Test `scripts/sunucu/merkez/tests/test_merkez.py` (ya da yönetim testlerinin bulunduğu dosya)

**Interfaces:**
- Ana sayfa (`liste`) üstünde **Açık sorunlar** kartı: müşteri numarası (bağlantı), işletme adı, sorun metni, önem, "… önce", SMS durumu ("SMS gitti" / "SMS gönderilemedi" / "SMS bekleniyor" / "deneme modu"); sorun yoksa kart gösterilmez. Liste satırında açık kritik sorun varsa `sorunlu` rozetine sorun metni eklenir.
- Müşteri sayfası: "Sağlık" kartı (son rapor zamanı, sürüm, açık kalma, servisler, disk, son mühür, WAN, SMS, bağlı sayısı; rapor yoksa "Henüz rapor yok") ve "Sorun geçmişi" tablosu (son 50).
- `/uyari` sayfası (menüye "Uyarı ayarları"): telefonlar (3 alan), "SMS uyarıları açık" onay kutusu, günlük sınır (1–100); CSRF; kayıt `hareket`e `UYARI_AYAR`. "Deneme SMS'i gönder" düğmesi (`POST /uyari/deneme`) ilk telefona `WifiCorrect deneme uyarısı` gönderir, sonucu mesajla gösterir, `UYARI_DENEME` hareketi; sayaç artar.
- [ ] Step 1: Failing tests: `acik_sorunlar_karti`, `musteri_saglik_ve_gecmis`, `uyari_ayarlari_kaydet_ve_dogrula`, `deneme_sms_sahte_gondericiyle` (Yönetim'e `gonder` enjekte edilebilir), CSRF'siz POST reddi.
- [ ] Step 2–4: RED, implement, GREEN, `hepsi.sh`.
- [ ] Step 5: Commit `merkez: açık sorunlar, sağlık kartı, sorun geçmişi, uyarı ayarları`.

### Task 5: Cihaz — sağlık raporu toplama ve gönderme

**Files:** Create `src/saglik.rs` (+ `mod saglik;`); Modify `src/ctl.rs` (`saglik`), `src/kaydedici.rs` (5 dk'da bir başlat), `src/merkez.rs` (gerekirse `saglik_gonder`)

**Interfaces:**
- `pub struct Girdi { pub surum, pub acik_sn: u64, pub servisler: [(&'static str, bool); 5], pub disk_yuzde: u64, pub son_muhur: String, pub kayit_boslugu_1s: bool, pub wan: bool, pub sms_sorunu: bool, pub bagli: u64, pub hiz_hata: bool }` ve `pub fn rapor(g: &Girdi) -> serde_json::Value` (Task 2'deki alan adlarıyla birebir).
- `pub fn topla(cfg: &Config, now: f64) -> Girdi` — servisler `systemctl is-active` (wificorrect-portal/-panel/-kaydedici, dnsmasq, wificorrect-guvenlik), disk `log_root` statvfs (panel.rs `disk_percent` ortak yere taşınabilir), son mühür (mühür dosyalarından en yeni gün — `muhur.rs`'deki mevcut okuyucuyu kullan), kayıt boşluğu: bugünkü `denetim.csv`'de son 3600 sn'de `LOG_BOSLUK`, WAN `ag.toml` wan arayüzünde IPv4 var mı, SMS `cfg.sms.mock || !cfg.sms_missing().is_empty()`, bağlı: oturumu süren ve köprüde görülen MAC sayısı, `hiz_hata`: trafik.json'da dolu mu.
- `pub fn gonder(m: &Merkez, rapor: &Value, http: &Http) -> Result<(), String>` → `POST /api/saglik` `{"cihaz_anahtari", "rapor"}`.
- `wificorrect ctl saglik` → bağlı değilse sessiz çıkış 0; gönderemezse stderr'e tek satır, çıkış 0.
- Kaydedici: `check_saglik(now)` 300 sn'de bir `systemd-run --collect --unit wfc-saglik-<t> --on-active 1s /usr/local/bin/wificorrect ctl saglik` (check_merkez kalıbı), yalnızca cihaz bağlıysa.
- [ ] Step 1: Failing tests: `rapor_alanlari_birebir` (JSON anahtarları ve tipleri Task 2 ile aynı), `kayit_boslugu_son_saat`, `kaydedici_5_dk_da_bir_baslatir` (runner çağrıları), `bagli_degilken_gonderilmez`.
- [ ] Step 2–4: RED, implement, GREEN (`bash scripts/test.sh wificorrect saglik::`, sonra tamamı).
- [ ] Step 5: Commit `cihaz: 5 dk'da bir merkeze sağlık raporu`.

### Task 6: Kurulum ve uçtan uca doğrulama (denetleyici + kullanıcı)

- [ ] Step 1: Cihaz: `bash scripts/gelistir.sh` + `systemctl restart wificorrect-kaydedici wificorrect-panel` (ağa dokunmaz). `wificorrect ctl saglik` elle bir kez → çıktı.
- [ ] Step 2: Merkez: dal itilir; **kullanıcı** `scripts/sunucu/merkez-guncelle.sh`'yi çalıştırır (sudo). `systemctl list-timers wc-saglik.timer` ve `journalctl -u wc-saglik` (kullanıcıdan çıktı ya da gbserver SSH'ı ile okuma).
- [ ] Step 3: Yönetim'de 1537344 sayfasında Sağlık kartı dolu; Açık sorunlar boş (ya da gerçek sorunlar).
- [ ] Step 4: Uyarı ayarları: kullanıcının numarası girilir; **deneme SMS'i yalnızca kullanıcı açıkça onaylarsa**.
- [ ] Step 5: Canlıda sorun benzetimi yapılmaz (servis durdurmak misafiri keser). Kurallar ve SMS kararı Task 3 testleriyle
  kanıtlanır; canlıda gerçek sorun yoksa Açık sorunlar boş ve SMS gitmediği, SMS yolu da deneme SMS'iyle gösterilir.
