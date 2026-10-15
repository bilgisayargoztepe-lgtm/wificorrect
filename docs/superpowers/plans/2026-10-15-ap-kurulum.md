# AP kurulumu — uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hizmet sağlayıcı, müşterinin taktığı erişim noktasını (AP) cihaz üzerinden uzaktan kurabilsin: ağdaki cihazları
görür, fabrika adresli AP'ye ulaşmak için köprüye 30 dk'lık geçici adres ekler, AP'nin kalıcı adresini kaydeder (köprüden
AP'ye misafir trafiği düşer) ve kendi Windows bilgisayarından tek komutla SOCKS + ayrı Chrome açar.

**Architecture:** Yeni `src/ap.rs` (saf işlevler + komut listeleri, `Runner` ile), panelde yeni admin bölümü
`src/panel/ap_sayfasi.rs` (`/admin` grubunda "AP kurulumu"), yeni `ctl` alt komutları, yeni kural dosyası
`deploy/debian/etc/wificorrect/ap.nft` (ayrı `table bridge wfc_ap`, `wificorrect-guvenlik.service` yükler, `ctl yukle`
doldurur). Geçici adres `ip addr add … label br-hotspot:ap valid_lft …` + `systemd-run --on-active` zamanlayıcısı.
Windows tarafı `scripts/ap-ac.ps1` (+ `ap-ac.cmd`), belge `docs/AP_KURULUM.md`. Merkezde değişiklik yok.

**Spec:** `docs/superpowers/specs/2026-10-15-ap-kurulum-design.md` · Dal `ap-kurulum` · Sürüm **0.4.11**

> **Sonradan değişenler (2026-10-15, son inceleme ve kullanıcı kararı):** (1) Bölüm **Ayarlar → AP kurulumu**'na taşındı;
> işletme sahibi de admin gibi kullanır (admin kilidi yok; CSRF, kim + rol denetimi ve bütün doğrulamalar aynen). Aşağıdaki
> "yalnızca admin / `if hizmet` / kilit formu / sahip 403" maddeleri bu kararla geçersizdir. (2) AP rehberinde yönetim
> erişimi IP kısıtı en sona alındı: önce AP 10.50.0.2'ye taşınır ve yeni adresten girilir, sonra kısıt (geçici adresten
> bağlıyken `10.50.0.1` kısıtı oturumu keser, AP'ye ulaşılamaz). (3) `ap.nft` kuralı yapılmadı (spec'teki karar).
> (4) **0.4.12 (spec "Ek: yan oda"):** köprüye ikinci adres (Task 1'deki `ip addr add … label br-hotspot:ap`, WAN/tünel/
> rota çakışma retleri) kaldırıldı; yerine misafir köprüsüne macvlan ile bağlı ayrı ağ ad alanı ("yan oda") + cihazda
> 127.0.0.1:1085'te SOCKS5 aracı (`wfc-ap-araci` geçici birimi, `src/yan_oda.rs`), panelde yabancı DHCP denetimi;
> `ap-ac.ps1` misafir ağı dışındaki IPv4 için `ssh -L` ile aracıya bağlanır. Denetim olayı `AP_YAN_ODA`
> (eski `AP_KURULUM_ADRES`), `AP_DHCP_DENETIM`. Ayrıntı: KURULUM_GUNLUGU "AP yan odası…".

## Önemli bulgu (plan yazılırken)

`kapi.nft`'teki `table bridge hotspot` → `chain isolate` (`meta ibrname "br-hotspot" counter drop`, forward kancası)
köprü portları arasındaki **bütün** iletimi zaten düşürüyor (ARP dahil). Yani "başka porttaki misafir → AP adresi"
trafiği bugün de geçmiyor; yönlendirilmiş yol da (`ip daddr` özel ağ → `gate` reject) kapalı. Spec'teki AP kuralı bu
yüzden **ikinci kat + sayaç** olarak uygulanır: ayrı tablo, `isolate`'ten önce (öncelik −10), kendi `counter`'ı ile —
kabul testinde düşen paketin bu kuraldan düştüğü sayaçla kanıtlanır. Yalıtım ileride gevşetilirse (ör. AP'ler arası
dolaşım) koruma kalır. Asıl koruma AP'nin kendi ayarıdır (spec madde 2/4).

## Global Constraints

`.superpowers/sdd/2026-10-15-ap-kurulum/kisitlar.md` ile aynı; özet: yalnızca admin (hizmet rolü, admin kilidi);
komutlar her zaman argüman listesi (kabuk yok); testler gerçek `ip`/`nft`/`systemd-run` çalıştırmaz (sahte `Runner` ve
sahte `ip` okuyucu — testler CANLI cihazda koşar); misafir interneti ve DHCP hiçbir adımda kesilmez; kullanıcıya görünen
metin Türkçe; denetim olay adları aşağıdaki gibi birebir.

---

### Task 1: Cihaz — AP kurulum modu (ağdaki cihazlar, geçici adres)

**Files:**
- Create: `src/ap.rs` (`main.rs`'e `mod ap;`), `src/panel/ap_sayfasi.rs` (`panel.rs`'e `mod ap_sayfasi;`)
- Modify: `src/panel.rs` (alan + yönlendirme + `BOLUMLER` + `bolum_simgesi` + `admin_donus` + `grup_sayfasi` eşlemesi),
  `src/ctl.rs` (`ap-adres-ekle`, `ap-adres-kaldir`; `yukle` içinde açılış temizliği; yardım metni), `src/fabrika.rs`
  (geçici adresi kaldır, `neden=fabrika`)

**Durum dosyası:** `/var/lib/wificorrect/ap-gecici.json` (600, `ortak::write_atomic`):
`{"adres":"192.168.0.250/24","kimlik":"<8 hex>","bitis":<epoch f64>}`. Spec "`/run` altında" diyor; **bilerek
`/var/lib`**: açılışta dosya kalmışsa adresin yeniden başlatmayla gittiği anlaşılır ve `neden=acilis` denetimi yazılabilir
(`/run`'da bu iz kaybolur). Adresin kendisi hiçbir ağ dosyasına yazılmaz → açılışta zaten yoktur. Test için
`ap::Yollar { gecici: PathBuf }` (`Yollar::sistem()`).

**`src/ap.rs` arayüzü (saf, test edilebilir):**
- `pub const ETIKET: &str = "br-hotspot:ap";` (`ip` etiketi ≤15 karakter, arayüz adıyla başlar), `pub const SURE_SN: u64 = 1800;`,
  `pub const HAVUZ: (u8, u8) = (20, 249);` (dnsmasq `dhcp-range` ile aynı; test `include_str!` ile
  `deploy/debian/etc/dnsmasq.d/wificorrect.conf` içinde `dhcp-range=10.50.0.20,10.50.0.249` arar), `pub const EN_COK: usize = 8;`
- `pub struct Cidr { pub ip: Ipv4Addr, pub onek: u8 }` + `parse("a.b.c.d/p")`, `ag()`, `yayin()`, `cakisir(&Cidr)`
  (biri diğerinin ağını kapsıyorsa).
- `pub fn adresler(ip_addr: &str) -> Vec<(String /*arayüz*/, Cidr)>` — `ip -o -4 addr show` çıktısı (`lo` atlanır).
- `pub fn rotalar(ip_route: &str) -> Vec<(String /*arayüz*/, Cidr)>` — `ip -4 route show` (`default` atlanır; önek
  yoksa /32).
- `pub fn gecici_dogrula(girdi: &str, cfg: &Config, ip_addr: &str, ip_route: &str) -> Result<Cidr, String>` — sırayla
  reddeder (metinler birebir değil ama anlamı bu):
  1. biçim değil → "Adres a.b.c.d/önek biçiminde olmalı (ör. 192.168.0.250/24)."
  2. önek 16–30 dışı → "Önek 16 ile 30 arasında olmalı (/16'dan geniş alt ağ kabul edilmez)."
  3. RFC 1918 dışı (10/8, 172.16/12, 192.168/16) → "Yalnızca özel ağ adresi kullanılabilir."
  4. ağ ya da yayın adresi → "Bu, alt ağın ağ ya da yayın adresi; başka bir adres seçin."
  5. `cfg.main.subnet` ile çakışma → "Bu adres misafir ağının içinde; AP zaten bu ağdaysa geçici adres gerekmez."
  6. `crate::uzak::AG` (10.99.0.0/24) ile çakışma (tünel kapalı olsa da) → "Uzak erişim tünelinin ağıyla çakışıyor."
  7. `adresler()` ya da `rotalar()`'daki herhangi bir ağla çakışma (WAN dahil) → "Bu adres cihazın `<arayüz>` ağıyla
     (`<ağ>`) çakışıyor; eklenirse cihazın bağlantısı bozulur. AP'nin fabrika adresi modemin ağındaysa müşteriden AP'yi
     sıfırlayıp DHCP'den adres almasını isteyin (docs/AP_KURULUM.md)." (spec "bilinen sınır"; netns çözümü **yapılmaz**).
- `pub fn ekle_komutlari(c: &Cidr, iface: &str, kimlik: &str, sure: u64) -> (Vec<String> /*zamanlayıcı*/, Vec<String> /*ip*/)`:
  - zamanlayıcı: `systemd-run --collect --unit wfc-ap-gecici-<kimlik> --on-active <sure>s --timer-property=AccuracySec=1s /usr/local/bin/wificorrect ctl ap-adres-kaldir sure <kimlik>`
  - adres: `ip addr add <a/p> dev <iface> label br-hotspot:ap valid_lft <sure+10> preferred_lft <sure+10>`
    (çekirdek ömrü yedek emniyettir: zamanlayıcı çalışmasa da adres düşer; zamanlayıcı 10 sn önce çalışıp denetimi yazar).
- `pub fn ekle(cfg, y: &Yollar, girdi, sure, ip_oku: &dyn Fn(&[&str]) -> String, runner: &Runner, now) -> Result<Gecici, String>`:
  canlı adres/rota okunur (`ip -o -4 addr show`, `ip -4 route show`) → doğrula → zaten açık geçici adres varsa (dosya ya
  da `ip -o -4 addr show dev <iface> label br-hotspot:ap` çıktısı dolu) **ret** ("Zaten bir geçici adres açık; önce
  kaldırın.") → **önce zamanlayıcı** (kurulamazsa hiçbir şey değişmez, hata) → durum dosyası → `ip addr add`
  (başarısızsa zamanlayıcı `systemctl stop wfc-ap-gecici-<k>.timer`, dosya silinir, hata). `iface = cfg.main.iface`.
- `pub fn kaldir(cfg, y, neden: &str, kimlik: Option<&str>, ip_oku, runner) -> Result<Option<String /*adres*/>, String>`:
  dosya yoksa `Ok(None)`. `kimlik` verildiyse (zamanlayıcı yolu) ve dosyadakiyle aynı değilse **hiçbir şey yapmaz**
  (eski zamanlayıcı yeni adresi erken silemez). Adres canlıda varsa `ip addr del <a/p> dev <iface>`; `neden != "sure"`
  ise `systemctl stop wfc-ap-gecici-<k>.timer`; dosya silinir; dönen adres denetim için.
- `pub fn durum(y, cfg, ip_oku) -> Option<(Gecici, bool /*canlıda var mı*/)>` — panel gösterimi (ör. port değişikliği
  `ifdown -a` ile adresi silmişse "artık yok" görünür; dosyayı zamanlayıcı temizler).
- `pub fn acilis_temizle(cfg, y, ip_oku) -> Option<String>` — dosya var ve adres canlıda **yoksa** dosyayı siler, adresi
  döner (`ctl yukle` `neden=acilis` yazar); canlıda varsa (servis yeniden başlatıldı) dokunmaz.
- Ağdaki cihazlar: `pub struct Komsu { mac, ipler: BTreeSet<Ipv4Addr>, ad: String, dhcp: bool, arp: Option<String /*REACHABLE…*/>, rastgele: bool }`;
  `pub fn komsular(kiralar: &str, neigh: &str) -> Vec<Komsu>` — dnsmasq kira dosyası (`bitis mac ip ad istemci-kimliği`;
  ad `*` ise boş) + `ip -4 neigh show dev br-hotspot` (`lladdr`'sız satırlar — FAILED/INCOMPLETE — atlanır); MAC'e göre
  birleşir, IP'ye göre sıralanır. `rastgele` = ilk baytın 0x02 biti (telefonların rastgele MAC'i; AP'ler genelde değil).
  OUI/üretici tablosu **yok** (yanlış eşleşme riski; müşteri MAC'i zaten verir).

**Panel (`ap_sayfasi.rs`):**
- `Panel`'e alanlar: `ap_yollar: crate::ap::Yollar` (testte geçici), `ip_oku: Box<dyn Fn(&[&str]) -> String + Send + Sync>`
  (varsayılan `ortak::capture`; testte sahte çıktı).
- `BOLUMLER`'e `("/admin", "/ap-kurulum", "ap-kurulum", "AP kurulumu", true)` (Admin ayarları'ndan sonra), simge `SIMGE_WIFI`,
  `admin_donus` `/ap-kurulum`'u da kabul eder.
- Yönlendirme (Admin ayarları ile aynı kilit kuralı): `GET /ap-kurulum`, `GET /ap-kurulum/cihazlar` `if hizmet`, değilse
  `admin_kilidi_formu`; `POST /ap-kurulum/gecici-ekle`, `POST /ap-kurulum/gecici-kaldir` `if hizmet`, değilse
  `text(403, "Admin kilidi kapalı.")`. Her POST'ta CSRF (mevcut düzen). POST sonrası yönlendirme Admin ayarları'nınki gibi
  (`/admin?m=…#ap-kurulum`).
- Bölüm içeriği:
  1. "Cihazın misafir portu": `10.50.0.1` (`cfg.main.router_ip`) ve köprünün MAC'i (`<sys>/br-hotspot/address`) — AP'de
     "yönetime yalnızca şu IP/MAC izin ver" için.
  2. "Ağdaki cihazlar" → bağlantı `GET /ap-kurulum/cihazlar` (ayrı sayfa; cihaz adları kişisel veri olabilir → her açılış
     denetime `PANEL_AP_CIHAZLAR sayi=<n>`). Tablo: MAC · IP · Ad · Kaynak (DHCP / ARP durumu) · Not ("rastgele MAC —
     büyük olasılıkla telefon", izinli cihazsa adı, Task 2'den sonra "kayıtlı AP adresi"). Boşsa "Cihaz görülmedi".
     Not: "Fabrika adresli AP, cihaz ona hiç paket göndermediyse listede görünmez; önce geçici adres ekleyin."
  3. "Geçici adres": açıksa "192.168.0.250/24 açık — HH:MM'de kendiliğinden kalkar (N dk)" + "Şimdi kaldır"; dosya var ama
     canlıda yoksa "Geçici adres artık yok (ağ yeniden kuruldu)" + "Kaldır" (dosyayı temizler); kapalıysa form
     (`adres`, örnek `192.168.0.250/24`) + "Ekle (30 dk)" ve not: "AP'nin fabrika adresiyle aynı alt ağdan, AP'nin
     kullanmadığı bir adres yazın. Misafirler etkilenmez; misafirden bu alt ağa erişim kapalıdır."
- Denetim (`self.audit`, kullanıcı/IP önekiyle): olay `AP_KURULUM_ADRES`, ek `eklendi adres=<a/p> sure=1800` ya da
  `kaldirildi adres=<a/p> neden=elle`. Reddedilen giriş denetime yazılmaz; yalnızca hata mesajı gösterilir.

**ctl:**
- `ctl ap-adres-ekle ADRES [--sure SN]` (SN 60–1800; konsol eşdeğeri ve kabul testinde kısa süre için) → `ap::ekle`;
  denetim `AP_KURULUM_ADRES eklendi adres=… sure=… kullanici=konsol`.
- `ctl ap-adres-kaldir NEDEN [KIMLIK]` (NEDEN ∈ `elle|sure`) → `ap::kaldir`; adres döndüyse denetim
  `AP_KURULUM_ADRES kaldirildi adres=… neden=<NEDEN>`.
- `yukle`: `ap::acilis_temizle` → `AP_KURULUM_ADRES kaldirildi adres=… neden=acilis`.
- `fabrika`: `ap::kaldir(…, "fabrika", None, …)` → `neden=fabrika` (spec listesine ek; port varsayılanı zaten siler,
  dosya bayat kalmasın).
- Yardım metnine iki komut.

- [ ] **Step 1: Testleri yaz (RED)** — `ap::tests`: `Cidr` ayrıştırma/ağ/yayın/çakışma; `gecici_dogrula` yedi ret
  (biçim, `/15`, `/31`, `8.8.8.0/24` genel, `192.168.0.0/24` ağ adresi, misafir `10.50.0.9/24`, tünel `10.99.0.50/16`,
  WAN `192.168.1.250/24` gerçekçi `ip -o -4 addr` çıktısıyla, rota `172.20.0.0/16 dev docker0` ile) ve kabul
  `192.168.0.250/24`; `ekle_komutlari` birebir argüman listesi (etiket, `valid_lft 1810`); `ekle` sırası: zamanlayıcı ip'den
  önce, zamanlayıcı başarısızsa ip çağrılmaz ve dosya yok, ip başarısızsa zamanlayıcı durdurulur ve dosya yok, zaten açıksa
  ret; `kaldir`: yanlış kimlikle hiçbir komut yok, `sure`'de `systemctl stop` yok, `elle`'de var, canlıda yoksa `ip addr del`
  yok ama dosya silinir; `acilis_temizle` iki durum; `komsular`: kira + neigh birleşmesi, FAILED atlanır, `*` adı boş,
  rastgele bit; `HAVUZ` dnsmasq dosyasıyla aynı. `panel::tests`: işletme sahibi `GET /ap-kurulum` → kilit formu, POST 403;
  admin sayfada köprü MAC'i ve form; CSRF'siz POST reddi; ekleme → runner çağrıları + denetim satırı + yönlendirme;
  çakışan adres → mesaj, runner'da `ip addr add` yok; `GET /ap-kurulum/cihazlar` tablo + `PANEL_AP_CIHAZLAR`; kaldırma.
  `ctl::tests`: `yukle` açılış temizliği denetimi.
- [ ] **Step 2:** `bash scripts/test.sh wificorrect ap` → yeni testler FAIL (derleme hatası değil, iddia hatası olacak
  şekilde iskelet).
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** `bash scripts/test.sh wificorrect ap` GREEN, sonra tam takım (`bash scripts/test.sh wificorrect`) PASS,
  yeni uyarı yok.
- [ ] **Step 5: Commit** `cihaz: AP kurulum modu (agdaki cihazlar, 30 dk gecici adres)`

**Kabul — `test.sh` ile cihazda (yalnızca birim testleri, canlı ağa dokunmaz):** yukarıdaki testlerin hepsi.
**Kabul — sonra elle, canlıda, ölü adam anahtarıyla (denetleyici + kullanıcı; Task sonrası "Canlı kabul" bölümü):**
geçici adresin gerçekten eklenip çekirdek ömrü ve zamanlayıcıyla kalkması, misafirlerin etkilenmemesi, çakışma reddi.

### Task 2: Cihaz — AP adresi kaydı ve köprü kuralı

> **Değişiklik (2026-10-15, uygulama sırasında karar):** köprü kuralı **yapılmadı** — `ap.nft` yok, nft/postinst/
> `wificorrect-guvenlik.service`/`paket-test.sh` değişmedi, `ctl ap-uygula` ve `kume_dosyasi`/`kume_uygula` yok.
> Gerekçe: `kapi.nft`'teki köprü yalıtımı port→port trafiği zaten düşürüyor; AP'ye kablosuz bağlı misafirin trafiği
> cihazdan hiç geçmez. Task 2 = yalnızca AP adresi kaydı: `[ap] adresler` (`ayar::Ap`), `ap::kayit_dogrula` (aşağıdaki
> kurallar, en çok 8), panel "AP adresleri" kartı + `POST /ap-kurulum/adresler` (yalnızca admin, CSRF; denetim
> `PANEL_AP_ADRES adresler=… eski=…`; başarı "Kaydedildi."), ağdaki cihazlar tablosunda "kayıtlı AP adresi" notu, fabrika
> dönüşünde liste boşalır (varsayılan ayar). Canlı kabul 1–3'teki nft adımları düşer; köprü yalıtımının AP adresini
> kapattığı (sahte istemci → 10.50.0.2 kapalı, cihazdan açık) yine denenebilir.

**Files:**
- Create: `deploy/debian/etc/wificorrect/ap.nft`
- Modify: `src/ayar.rs` (`[ap]`), `src/ap.rs` (kayıt doğrulama + küme uygulama), `src/panel/ap_sayfasi.rs` (AP adresleri
  kartı + `POST /ap-kurulum/adresler`), `src/panel.rs` (yönlendirme), `src/ctl.rs` (`ap-uygula`; `yukle` kümeyi doldurur),
  `src/fabrika.rs` (boş küme uygulanır), `deploy/debian/etc/systemd/system/wificorrect-guvenlik.service`, `paket/postinst`,
  `scripts/paket-test.sh`

**Ayar:** `Config.ap: Ap` — `#[derive(Serialize, Deserialize, Clone, Debug, Default)] #[serde(default)] pub struct Ap {
#[serde(skip_serializing_if = "Vec::is_empty")] pub adresler: Vec<String> }` (`ayarlar.toml` `[ap] adresler = ["10.50.0.2"]`).
Fabrika dönüşünde varsayılan (boş) — yeni işletme yeni AP.

**`ap.nft` (birebir; tablo kendi dosyasında, `kapi.nft`/`guvenlik.nft` yeniden yüklense de silinmez):**

```
#!/usr/sbin/nft -f
# AP yönetim adresleri (panel → Admin → AP kurulumu; küme `wificorrect ctl ap-uygula` / `ctl yukle` ile dolar).
# Köprüden geçen (başka porttaki misafirden AP'ye giden) bütün IPv4 trafiği düşer; ARP serbest (AP ağda kaybolmasın).
# kapi.nft'teki köprü yalıtımı (isolate, öncelik 0) bunu zaten keser: bu tablo ondan önce çalışan ikinci kat ve sayaçtır.
# AP'ye kablosuz bağlı misafirin paketi cihazdan hiç geçmez: asıl koruma AP'nin kendi ayarıdır (docs/AP_KURULUM.md).
# Cihazın kendisi (10.50.0.1, çıkış yolu) AP'ye erişir; tünelden SOCKS ile biz de.

table bridge wfc_ap
delete table bridge wfc_ap

table bridge wfc_ap {
	set adres {
		type ipv4_addr
	}

	chain koru {
		type filter hook forward priority -10; policy accept;
		meta ibrname "br-hotspot" ip daddr @adres counter drop
	}
}
```

**Yükleme:** `wificorrect-guvenlik.service`'te `kapi.nft`'ten hemen sonra
`ExecStart=-/usr/sbin/nft -f /etc/wificorrect/ap.nft` (yüklenemezse açılış ve misafir interneti durmaz — yalıtım zaten
korur; kapalıyken güvenli ilkesi `kapi.nft`/`guvenlik.nft`'te aynen kalır). `ctl yukle` (zaten `ExecStartPost`) sonunda
`ap::kume_uygula(&cfg, runner)` çağırır (hata yalnızca `eprintln`).
**postinst** (çalışan sistem dalında, `try-restart`'tan sonra; `kapi.nft` **yeniden yüklenmez** — oturum kümesi silinir):
`nft -f /etc/wificorrect/ap.nft && /usr/local/bin/wificorrect ctl ap-uygula >/dev/null || echo "wificorrect: AP kuralı yüklenemedi" >&2`.
`paket-test.sh`: `dpkg -L wificorrect | grep -q /etc/wificorrect/ap.nft` ve unit dosyasında `ap.nft` satırı.

**`ap.rs` ekleri:**
- `pub fn kayit_dogrula(girdi: &str, cfg: &Config) -> Result<Vec<String>, String>` — satır/virgül/boşlukla ayrılmış
  liste; her biri IPv4, `cfg.main.subnet` içinde, `router_ip` değil, ağ/yayın adresi değil, **DHCP havuzu dışında**
  (son sekizli `HAVUZ.0..=HAVUZ.1` değil; /24 dışı misafir ağı için aynı kural ağ adresine göre); tekrar atılır; en çok
  `EN_COK` (8). Boş girdi = liste boşaltılır (geçerli). Hata metni hatalı adresi söyler ("10.50.0.30 DHCP havuzunda
  (10.50.0.20–249); AP'ye havuz dışında bir adres verin.").
- `pub fn kume_dosyasi(adresler: &[String]) -> String` — `flush set bridge wfc_ap adres\n` + (boş değilse)
  `add element bridge wfc_ap adres { a, b }\n`.
- `pub fn kume_uygula(cfg: &Config, runner: &Runner) -> Result<(), String>` — geçici dosyaya (`std::env::temp_dir()`,
  `filtre::uygula`'daki gibi) yazıp `nft -f <dosya>` (tek işlem = atomik), dosyayı siler. Adresler yeniden
  `kayit_dogrula`'dan geçmiş olmalı (elle bozulmuş ayar dosyasından gelen geçersiz satır nft'ye gitmez; geçersizse hata).

**Panel:** "AP adresleri" kartı: mevcut liste, `textarea name="adresler"` (satır başına bir adres, en çok 8) + "Kaydet";
not: "AP ayarlandıktan sonra kalıcı yönetim adresini yazın (ör. 10.50.0.2). Kaydedince başka porttaki misafirlerden bu
adrese giden trafik cihazda düşer. AP'ye kablosuz bağlı misafirleri yalnızca AP'nin kendi ayarı korur."
`POST /ap-kurulum/adresler` (`if hizmet`, CSRF): doğrula → `Config` yeniden yükle, `ap.adresler` yaz, `save` →
`kume_uygula`; başarı "Kaydedildi ve uygulandı.", uygulama hatası "Kaydedildi ama güvenlik duvarına uygulanamadı: …"
(ayar kalır, açılışta yeniden denenir). Denetim `PANEL_AP_ADRES adresler=<a,b|-> eski=<…|->`. Ağdaki cihazlar
tablosunda kayıtlı adres "kayıtlı AP adresi" notu alır.

**ctl:** `ctl ap-uygula` → `kume_uygula`; çıkış kodu 0/1, hata stderr.

- [ ] **Step 1: Testleri yaz (RED)** — `ayar`: `[ap]` yok → boş, yazılınca okunur, boşken `[ap]` altında satır yok;
  `ap`: `kayit_dogrula` (10.50.0.2 kabul; 10.50.0.1, 10.50.0.0, 10.50.0.255, 10.50.0.30 havuz, 192.168.0.2 alt ağ dışı,
  `abc`, 9 adres → ret; tekrar atılır; boş → boş liste); `kume_dosyasi` iki durum birebir; `kume_uygula` runner'a
  `nft -f <yol>` gider ve dosya içeriği doğru (runner içinde okunur), geçersiz ayar satırında nft çağrılmaz;
  `ap.nft` dosyası (`include_str!`) `table bridge wfc_ap`, `priority -10`, `ip daddr @adres counter drop` içerir ve
  `kapi.nft`'teki köprü tablosunun adı farklıdır; guvenlik unit dosyası `ExecStart=-/usr/sbin/nft -f /etc/wificorrect/ap.nft`
  satırını `kapi.nft` satırından sonra içerir. `panel`: kayıt (CSRF, sahip 403, geçersiz → mesaj ve ayar değişmez, geçerli
  → ayar dosyası + runner'da `nft -f` + `PANEL_AP_ADRES` satırı); `ctl`: `yukle` kümeyi doldurur.
- [ ] **Step 2:** `bash scripts/test.sh wificorrect ap` → FAIL.
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** süzgeçli + tam takım PASS, yeni uyarı yok. (`paket-test.sh` CI'da koşar; yerelde `bash -n`.)
- [ ] **Step 5: Commit** `cihaz: AP adresi kaydi, kopru katmaninda AP'ye misafir erisimi kapali`

**Kabul — `test.sh`:** yukarıdaki birim testleri. **Elle, canlıda (ölü adam anahtarıyla):** aşağıda "Canlı kabul" 1–4.

### Task 3: Windows SOCKS kısayolu, belge, sürüm

**Files:**
- Create: `scripts/ap-ac.ps1`, `scripts/ap-ac.cmd`, `docs/AP_KURULUM.md`
- Modify: `Cargo.toml` + `Cargo.lock` (yalnızca `wificorrect` girdisi) → **0.4.11**, `docs/KURULUM_GUNLUGU.md` (sona
  "AP kurulumu (2026-10-15)" bölümü)

**`scripts/ap-ac.ps1`** (Windows PowerShell 5.1 uyumlu; `&&`, `?:`, `??` yok; sır yok):
- Parametreler: `-Adres` (zorunlu; `10.50.0.2`, `192.168.0.254` ya da `http(s)://…`; yalnızca adres/ad + isteğe bağlı
  port/yol kabul edilir, başka karakter → hata), `-Cihaz` (varsayılan `wificorrect` = `~/.ssh/config` girdisi; dışarıdan
  `root@10.99.0.11`), `-Port` (varsayılan 1080; doluysa 1081–1099 arasında boş olanı seçer), `-Anahtar` (isteğe bağlı
  `-i` yolu), `-Kuru` (komutları yazar, çalıştırmaz — sınama için).
- Akış: `ssh.exe` (`Get-Command ssh`) → `Start-Process -PassThru -WindowStyle Hidden ssh -ArgumentList @('-N',
  '-D', "127.0.0.1:$Port", '-o', 'ExitOnForwardFailure=yes', '-o', 'ServerAliveInterval=15', '-o', 'BatchMode=yes', $Cihaz)`;
  en çok 15 sn `System.Net.Sockets.TcpClient` ile 127.0.0.1:$Port dinleniyor mu bekler (ssh erken çıkarsa çıkış koduyla
  hata: "SSH tüneli açılamadı: cihaz adı/anahtar/ağ (WireGuard açık mı?)"). Chrome yolu: `Program Files`, `Program Files
  (x86)`, `%LOCALAPPDATA%\Google\Chrome\Application\chrome.exe` (bulunamazsa Edge `msedge.exe`, yoksa hata). Geçici profil
  `$env:TEMP\wfc-ap-<rastgele>`; Chrome argümanları `--user-data-dir=<profil>`, `--proxy-server=socks5://127.0.0.1:$Port`,
  `--no-first-run`, `--no-default-browser-check`, `--new-window`, `http://<Adres>/` (şema verilmediyse). Chrome
  `-PassThru` + `WaitForExit()`; `finally`: ssh süreci durdurulur, profil klasörü silinir (silinemezse uyarı). Ad çözümü
  SOCKS5'te uzakta yapılır (ayrıca `--host-resolver-rules` gerekmez; belgeye not).
- `scripts/ap-ac.cmd`: `@powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0ap-ac.ps1" %*`.

**`docs/AP_KURULUM.md`** (Türkçe, teknisyen için; spec madde 4 tam liste): müşteriden istenecekler (marka, model,
etiketteki adres/alan adı, fabrika kullanıcı adı/parolası, MAC) · panel → Ayarlar → AP kurulumu → Ağdaki cihazlar ile MAC
eşleştirme · DHCP'den 10.50.0.x aldıysa doğrudan, fabrika adresliyse geçici adres (aynı alt ağdan, AP'nin adresi değil;
modem ağıyla çakışırsa panel reddeder → müşteriden sıfırlama/DHCP) · `scripts\ap-ac.cmd -Adres … [-Cihaz root@10.99.0.11]`
· ayar listesi ve sırası (AP modu; DHCP sunucusu ve "Smart IP" kapalı; SSID işletme adı, şifresiz; AP isolation açık;
kanal 1/6/11, 20 MHz, iletim gücü yüksek; yönetim parolası = işletme sahibinin panel parolası, ≤15 karakter sınırlı
modelde kısaltılır ve kullanılan parola merkezde müşteri sayfasına not yazılır; yönetim adresi 10.50.0.2/24, ağ geçidi
ve DNS 10.50.0.1, bağlantı kopar → yeni adresten girilir; **en son** (yeni adresten girdikten sonra) "kablosuzdan yönetim"
kapalı ya da yönetim yalnızca 10.50.0.1 / panelde gösterilen köprü MAC'i; yoksa yalnızca parolaya kalır — not edilir) ·
geçici adresi kaldır · AP
adresini kaydet · kontrol (telefonla portal açılıyor mu; panelde "Ağdaki cihazlar"da AP 10.50.0.2) · bilinen tuzaklar
(TP-Link `ssh -L` ile 127.0.0.1'den girişte Referer reddi — SOCKS'ta yok; fabrika modunda DHCP sunucusu açık AP
misafirlere yanlış adres dağıtabilir → takar takmaz kurulmalı; port değişikliği geçici adresi siler).

**KURULUM_GUNLUGU:** bölüm: dosyalar ve yollar (`ap.rs`, `ap.nft`, `/var/lib/wificorrect/ap-gecici.json`, etiket,
zamanlayıcı birimi `wfc-ap-gecici-*`, çekirdek ömrü), denetim olayları, yalıtım bulgusu (kural ikinci kat), bilinen sınır
(modem ağı çakışması; netns yapılmadı), Windows betiği.

- [ ] **Step 1:** Betik sözdizimi: `powershell -NoProfile -Command "[void][System.Management.Automation.Language.Parser]::ParseFile('scripts/ap-ac.ps1',[ref]$null,[ref]$e); if($e){$e;exit 1}"`
  ve `-Kuru` ile çalıştırma (gerçek ssh açmaz) → beklenen ssh ve Chrome argümanları yazılır; hatalı `-Adres "x;y"` → hata.
- [ ] **Step 2:** Belgeler + sürüm. `bash scripts/test.sh wificorrect` tam takım PASS (sürüm testi varsa).
- [ ] **Step 3: Commit** `betik: AP icin SOCKS kisayolu; belge: AP kurulumu; surum 0.4.11`

---

## Canlı kabul (uygulayıcı değil — denetleyici + kullanıcı, PR birleşip `v0.4.11` yayınlandıktan sonra)

Kurulum: Yönetim → Sürümler → GitHub'dan al → test kanalı → Göztepe "Şimdi güncelle" (uzaktan güncellemenin kendi geri
dönüşü var). Her deneme **ölü adam anahtarıyla**: `systemd-run --unit wfc-olu-adam --on-active=300 /bin/sh -c 'nft delete table bridge wfc_ap; ip addr flush dev br-hotspot label br-hotspot:ap'`
→ dene → **yeni** SSH bağlantısıyla erişimi doğrula → `systemctl stop wfc-olu-adam.timer`. Sakin saatte.

1. `nft list table bridge wfc_ap` var, küme boş; `systemctl status wificorrect-guvenlik` sorunsuz; misafir girişi
   (sahte istemci: DHCP + portal 302) çalışıyor.
2. Panel → Admin → AP kurulumu → 10.50.0.2 kaydet → `nft list set bridge wfc_ap adres` = 10.50.0.2; denetimde
   `PANEL_AP_ADRES`.
3. `scripts/sahte-istemci.sh 'ping -c3 -W1 10.50.0.2; curl -s -m3 -o /dev/null -w "%{http_code}\n" http://10.50.0.2/ || echo kapali'`
   → ping kayıp, `kapali`; `nft list chain bridge wfc_ap koru` sayacı arttı. Cihazdan `curl -s -m3 -o /dev/null -w '%{http_code}' http://10.50.0.2/`
   → 200/401 (açık). Sahte istemci aynı anda `curl http://10.50.0.1:8080/` → portal (misafir akışı bozulmadı).
4. Geçici adres, sahte AP ile (fabrika adresli AP taklidi; ölü adam anahtarı netns'i de siler: `ip netns del sahteap`):
   `ip netns add sahteap; ip link add vsap type veth peer name isap; ip link set isap netns sahteap; ip link set vsap master br-hotspot up; ip netns exec sahteap ip addr add 192.168.0.254/24 dev isap; ip netns exec sahteap ip link set isap up`
   → `wificorrect ctl ap-adres-ekle 192.168.0.250/24 --sure 120` → `ip -4 addr show dev br-hotspot` etiketi ve
   `valid_lft` görünür; `ping -c2 192.168.0.254` (cihazdan) başarılı; sahte istemci (DHCP'li) 192.168.0.254'e
   ulaşamaz, interneti/portalı çalışır; ~120 sn sonra adres yok, denetimde `AP_KURULUM_ADRES kaldirildi … neden=sure`.
   Panelden ekle → `systemctl list-timers 'wfc-ap-gecici-*'` ~30 dk → "Şimdi kaldır" → adres ve zamanlayıcı yok,
   `neden=elle`. Panelde `192.168.1.250/24` ve `10.99.0.50/24` reddedilir. Sonra `ip netns del sahteap; ip link del vsap`.
5. Windows: `scripts\ap-ac.cmd -Adres 10.50.0.2` (dükkân ağı) ve WireGuard açıkken `-Cihaz root@10.99.0.11` → TP-Link
   giriş sayfası açılır, giriş yapılabilir; Chrome kapanınca `ssh.exe` kalmaz, profil klasörü silinir.
6. Göztepe AP (TL-WA901ND, 10.50.0.2): adres kayıtlı kalır; AP'de kablosuz yönetim seçeneği var mı elle bakılır
   (yoksa not edilir), yönetim parolası listeye göre.
7. (İsteğe bağlı, kullanıcı onayıyla — canlı cihazı yeniden başlatır) yeniden açılışta küme yeniden dolu, geçici adres
   yok; açıkken yeniden açıldıysa `neden=acilis`.
