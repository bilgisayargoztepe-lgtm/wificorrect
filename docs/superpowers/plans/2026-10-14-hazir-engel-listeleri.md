# Hazır engel listeleri — uygulama planı

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kumar/bahis, yetişkin ve zararlı site kategorileri merkezde günlük hazırlanır; cihaz açık kategorileri indirip DNS'te engeller; işletme sahibi panelden aç/kapa yapar.

**Architecture:** Merkez `listeler.py` (kaynak indirme + temizleme + kategori başına gzip + meta) günlük zamanlayıcıyla çalışır; `/api/saglik` cevabı kategori sürümlerini taşır, `/api/liste` gzip'i verir. Cihaz `src/liste.rs` sürüm değişince indirir, sha256 doğrular, açık kategorileri birleştirip (istisnalar ve elle liste ile) dnsmasq `servers-file` olarak yazar ve dnsmasq'a SIGHUP gönderir.

**Tech Stack:** Python stdlib (merkez), Rust (cihaz), dnsmasq `servers-file` (`server=/alan/` satırları, SIGHUP ile yeniden okunur).

**Spec:** `docs/superpowers/specs/2026-10-14-hazir-engel-listeleri-design.md`

## Global Constraints

- Kategoriler (kimlik / ekran adı): `kumar` "Kumar ve bahis", `yetiskin` "Yetişkin içerik", `zararli` "Zararlı ve dolandırıcı siteler". VPN/proxy kategorisi yok; VPN/proxy/DoH alan adları hiçbir listeden gelmez.
- Varsayılan: üç kategori **açık** (ayar dosyasında anahtar yoksa açık sayılır).
- Kategori başına en çok **300.000** alan adı (fazlası kesilir, meta'da `kesildi` işaretlenir).
- "Asla engelleme" listesi merkezde (en az: wificorrect.com, netgsm.com.tr, twilio.com, google.com, gstatic.com, apple.com, icloud.com, microsoft.com, windowsupdate.com, gov.tr, edu.tr, bank alan adları için `*.com.tr` bankaları ayrıca değil — yalnızca bu listedekiler ve alt adları). Aşırı geniş girdiler atılır: tek etiketli adlar, kamu son ekleri (`com`, `net`, `org`, `com.tr`, `gov.tr`, `edu.tr`, `bel.tr`, `k12.tr`, `org.tr`, `net.tr`, `gen.tr`, `av.tr`, `tv.tr`, `info.tr`, `biz.tr`, `web.tr`, `name.tr`, `tel.tr`, `pol.tr`, `tsk.tr`, `dr.tr`, `bbs.tr`, `nc.tr`, `kep.tr`).
- Alan adı geçerliliği: yalnızca `[a-z0-9.-]`, etiket 1–63, toplam ≤ 253, en az iki etiket; IP adresleri atılır.
- Engelleme DNS'te: alan adı ve bütün alt adları NXDOMAIN; sorgular 5651 DNS kaydına bugünkü gibi düşer. İşletmenin istisnaları (`[filtre] istisnalar`, alt dize eşleşmesi — bugünkü kelime istisnasıyla aynı anlam) hazır listeden de çıkarır.
- Liste indirme/uygulama hatası misafirin internetini kesmez: eski liste kalır. Merkeze ulaşılamazsa son liste kullanılır.
- Denetim: cihaz `PANEL_HAZIR_LISTE kategori=<k> durum=acik|kapali`, `HAZIR_LISTE_GUNCEL kategori=<k> surum=<sha8> sayi=<n>`, `HAZIR_LISTE_HATA kategori=<k> neden=<…>`; merkez hareketi `LISTE_GUNCEL` / `LISTE_HATA`.
- Kullanıcıya görünen metin Türkçe.

## Review Focus

- Bozuk/eksik kaynak indirmesi bütün listeyi silmemeli (merkez eski çıktıyı korur) (Task 1).
- `asla engelleme` alan adının alt adı da korunmalı (ör. `api.wificorrect.com`) ve bir kaynak bunu listelese bile çıkmalı (Task 1).
- Cihazda sha256 tutmazsa yazılmamalı, eski dosya kalmalı (Task 2).
- dnsmasq SIGHUP sonrası yeni servers-file okunmalı; dosya yoksa dnsmasq çalışmaya devam etmeli (boş dosya ile başlatılır) (Task 2 + e2e).
- Kategori kapatılınca o kategorinin alan adları hemen açılmalı (Task 2).

---

### Task 1: Merkez — kaynaklar, liste üretimi, API, Yönetim sayfası

**Files:**
- Create: `scripts/sunucu/merkez/listeler.py`, `scripts/sunucu/merkez/wc-listeler.service`, `scripts/sunucu/merkez/wc-listeler.timer`, `scripts/sunucu/merkez/tests/test_listeler.py`
- Modify: `api.py` (`/api/saglik` cevabına `listeler`, yeni `/api/liste`), `yonetim.py` (menüye "Engel listeleri"), `kur.sh` (listeler.py kur, timer etkin), `veri.py` yalnızca gerekirse (hareket)

**Interfaces:**
- `listeler.KAYNAKLAR`: `{kategori: [(ad, url, bicim)]}` — `bicim` ∈ `hosts` (`0.0.0.0 alan`), `duz` (satır başına alan), `adblock` (`||alan^`), `usom` (URL ya da alan; yol/port atılır). Kaynakları uygulayıcı güncel adreslerle doğrular (HaGeZi `gambling`, `nsfw`, `tif` ya da eşdeğeri — **`doh-vpn-proxy-bypass` kullanılmaz**; USOM `https://www.usom.gov.tr/url-list.txt` ya da güncel adresi; StevenBlack yetişkin eki yedek). Lisansları bir yorumda yazar.
- `listeler.temizle(satirlar, bicim) -> set[str]`; `listeler.uret(kategori, getir, dizin, simdi) -> dict meta` — meta: `{kategori, sha256, sayi, zaman, kesildi, kaynaklar: [{ad, sayi, hata?}]}`. Bütün kaynaklar başarısızsa eski `<dizin>/<kategori>.txt.gz` ve meta korunur, meta'ya `hata` yazılır. Çıktı: `<dizin>/<kategori>.txt.gz` (sıralı, satır başına bir alan, gzip, `mtime=0` — aynı içerik aynı sha), `<dizin>/<kategori>.json` (meta); sha256 gzip **içeriğinin değil düz metnin** sha'sıdır (cihaz açıp doğrular). Atomik yazma.
- `listeler.main()`: üç kategoriyi üretir (`DIZIN = /var/lib/wificorrect/merkez/listeler`), hareket yazar. `wc-listeler.timer` günde bir (04:30, `Persistent=true`), servis `wcpanel` olarak, `wc-merkez` sandbox'ı gibi.
- `api`: `/api/saglik` cevabına `"listeler": {kategori: {"sha256","sayi","zaman"}}` (meta okunamazsa kategori yok). `/api/liste` `{cihaz_anahtari, kategori}` → 200 `application/gzip` gövde; bilinmeyen kategori 400; `hizmet_acik` değilse 403; dosya yoksa 404. Gövde parça parça yazılır (mevcut büyük gövde yolu).
- `yonetim`: `/listeler` (yalnız yönetici): kategori, sayı, son güncelleme, kaynak başına sayı/hata, "Şimdi güncelle" (POST, CSRF; `listeler.uret` çalıştırır).

- [ ] **Step 1: Testleri yaz** (sahte `getir`): üç biçimin ayrıştırılması; geçersiz/IP/tek etiket/kamu son eki atılır; asla-engelleme alanı ve alt adı (`api.wificorrect.com`) çıkar; 300.000 sınırı ve `kesildi`; aynı girdiler → aynı sha; bir kaynak hata verince diğerleriyle üretilir ve meta'da hata; hepsi hata → eski dosya korunur; `/api/saglik` cevabında `listeler`; `/api/liste` 200/400/403/404 ve gzip gövde; yönetim sayfası CSRF ve görüntüleme.
- [ ] **Step 2:** hepsi.sh → yeni testler FAIL.
- [ ] **Step 3:** Uygula.
- [ ] **Step 4:** `TUM TESTLER GECTI`.
- [ ] **Step 5: Commit** `merkez: hazir engel listeleri (uretim, /api/liste, yonetim sayfasi)`

### Task 2: Cihaz — indirme, uygulama, panel

**Files:**
- Create: `src/liste.rs` (`main.rs`'e `mod liste;`)
- Modify: `src/ayar.rs` (`[filtre] hazir` = kategori → bool, varsayılan açık), `src/saglik.rs` + `src/ctl.rs` (sağlık cevabındaki `listeler`; değişince `systemd-run --collect --unit wfc-liste … ctl liste-guncelle`), `src/merkez.rs` (gzip indirme — mevcut `indir` ile), `src/filtre.rs` (`check` hazır listeyi de söyler), `src/panel/filtre_sayfasi.rs` (Hazır listeler bölümü + POST `/yasakli-siteler/hazir`), `deploy/debian/etc/dnsmasq.d/wificorrect.conf` (`servers-file=/etc/wificorrect/hazir-liste.conf`), `paket/postinst` (dosya yoksa boş oluştur; çalışan sistemde `ctl liste-uygula`)

**Interfaces:**
- `liste::KATEGORILER = [("kumar","Kumar ve bahis"),("yetiskin","Yetişkin içerik"),("zararli","Zararlı ve dolandırıcı siteler")]`.
- Cihaz dosyaları: `/var/lib/wificorrect/liste/<k>.txt.gz` + `<k>.json` (`{sha256, sayi, zaman}`); birleşik `/etc/wificorrect/hazir-liste.conf` (`server=/alan/` satırları; yalnızca açık kategoriler, istisnalar çıkarılmış, elle listedeki siteler bugünkü `yasak-siteler.conf`'ta kalır).
- `liste::guncelle(y, m, hedef: &BTreeMap<String,Meta>, acik: &BTreeMap<String,bool>, indir, now) -> Vec<String>` (indirilen kategoriler); sha tutmazsa dosya yazılmaz, `HAZIR_LISTE_HATA neden=sha`.
- `liste::uygula(cfg, y, runner) -> Result<usize,String>`: birleşik dosyayı atomik yazar, `systemctl kill -s HUP dnsmasq.service`; döner: satır sayısı. `ctl liste-uygula` ve `ctl liste-guncelle` alt komutları.
- Panel: Yasaklı siteler sayfasının en üstünde "Hazır listeler": kategori adı, açık/kapalı düğmesi, alan adı sayısı, son güncelleme (gün.ay.yıl). POST sonrası `ayar` kaydı + `liste::uygula` (hemen) ve kategori yeni açıldıysa indirme işi başlat. `check()` "Hazır liste: Kumar ve bahis" der.
- [ ] **Step 1: Testleri yaz:** varsayılan açık (anahtar yok); sha doğrulama (tutmazsa eski kalır); birleşik dosya yalnızca açık kategoriler, istisna alt dizesi geçen alan çıkar, `server=/alan/` biçimi; kapatınca dosyadan çıkar; uygula HUP çağırır; panel aç/kapa CSRF + denetim satırı; `check` hazır listeyi bildirir; sağlık cevabındaki sürüm değişince işi başlatır, aynıysa başlatmaz.
- [ ] **Step 2–4:** süzgeçler RED → uygula → GREEN; tam takım PASS, yeni uyarı yok.
- [ ] **Step 5: Commit** `cihaz: hazir engel listeleri (indirme, dnsmasq servers-file, panel)`

### Task 3: Sürüm ve belge

- [ ] Sürüm **0.4.9** (Cargo.toml + yalnızca `wificorrect` girdisi Cargo.lock). `docs/KURULUM_GUNLUGU.md` sonuna "Hazır engel listeleri (2026-10-14)" bölümü: kategoriler, kaynaklar ve lisanslar, merkez zamanlayıcısı, cihaz dosyaları, istisna davranışı, VPN kategorisinin bilerek olmaması. Commit `surum 0.4.9, hazir engel listeleri belgesi`.

### Task 4: Kurulum ve uçtan uca (denetleyici + kullanıcı)

- [ ] Merkez: kullanıcı `merkez-guncelle.sh`; Yönetim → Engel listeleri → "Şimdi güncelle"; sayılar ve hata yok.
- [ ] Cihaz: PR → birleştir → `v0.4.9` → Sürümler → test hedefi + Şimdi güncelle.
- [ ] Cihazda: 5 dk içinde listeler iner; `hazir-liste.conf` satır sayısı; dnsmasq bellek (RSS) önce/sonra; sahte istemciyle bilinen bir bahis alanı NXDOMAIN, google.com ve wificorrect.com açık; kategori kapatılınca açılır, yeniden açılınca kapanır; DNS yanıt süresi ölçümü (100 sorgu).
