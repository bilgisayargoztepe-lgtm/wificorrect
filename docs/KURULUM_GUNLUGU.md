# Kurulum günlüğü — Debian 13 (ilk cihaz: Bocafe donanımı)

Donanım: Intel Celeron J1900, 1,85 GB RAM, 32 GB SSD (sda1 EFI, sda2 kök 27 GB ext4, sda3 swap), Wi-Fi Atheros AR9287 (ath9k).
Sistem: Debian 13 "trixie", çekirdek 6.12.111, cihaz adı `wificorrect`. Yönetim: `root` yalnızca SSH anahtarıyla, `wificorrect` kullanıcısı (sudo).

## Arayüz adları
| Debian | Eski (OpenWrt) | Fiziksel | MAC | Görev |
|---|---|---|---|---|
| `enp3s0` | `eth1` | sağ, "Ethernet 1" | 00:0e:c4:ce:a0:9b | internet alır (modem DHCP, modemde **192.168.1.110**'a sabit) |
| `enp1s0` | `eth0` | sol, "Ethernet 2" | 00:0e:c4:ce:a0:9a | müşteri tarafı, `br-hotspot` köprüsünde (AP buraya takılacak) |
| `wlp2s0` | `phy0-ap0` | AR9287 | 90:00:4e:b5:b3:c5 | **kapalı**; ileride panelden açılacak |

## Adım 1 — Temel sistem (2026-10-02)
- Paketler: `dnsmasq hostapd iw wireless-regdb conntrack rsync curl unattended-upgrades sudo bridge-utils`.
- Kurulmayanlar (kullanıcı kararı): `firmware-atheros` (USB Wi-Fi yok), `iptables` (bet/porn filtresi yerine "yasaklı kelimeler ve siteler" modülü yazılacak).
- `hostapd` maskeli (Wi-Fi kapalı), `dnsmasq` Adım 3'e kadar kapalıydı.
- Otomatik güncelleme yalnızca `trixie-security`, kendiliğinden yeniden başlatma yok (`deploy/debian/etc/apt/apt.conf.d/`).
- NTP: `0.tr.pool.ntp.org 1.tr.pool.ntp.org time.google.com`, saat dilimi Europe/Istanbul.
- `/srv/5651`, `/srv/hotspot/state` (700). Ayrı `/srv` bölümü yok; Debian'da sysupgrade riski olmadığı için kökte.
- Açık karar: SSH parola girişi (`wificorrect` kullanıcısı için) hâlâ açık.

## Adım 2 — Köprü (2026-10-02)
- `/etc/network/interfaces.d/hotspot`: `br-hotspot` = `enp1s0`, 10.50.0.1/24, müşteri tarafında IPv6 kapalı. Ölü adam anahtarı + yeni SSH oturumuyla doğrulandı.

## Adım 3 — Güvenlik duvarı + DHCP/DNS (2026-10-02)
- `wificorrect-guvenlik.service` (sysinit, ağdan önce): `guvenlik.nft` (temel duvar, fw4'ün karşılığı) + `kapi.nft` (giriş kapısı: auth / allow_mac / ban_mac setleri, DNS yönlendirme, portala yönlendirme, 0x5651 işareti, köprü yalıtımı). Yönlendirme yalnızca kurallar yüklenince açılır; durdurmak kuralları silmez (kapalıyken güvenli). Debian'ın `nftables.service`'i kapalı.
- dnsmasq: yalnızca `br-hotspot`, havuz 10.50.0.20–249, kira 2 saat, `log-queries=extra`.
- Doğrulama (cihaz içinde sanal müşteri, `ip netns` + veth): DHCP ✔, DNS yönlendirme ✔ (30–60 ms), girişsiz HTTPS anında red ✔, girişsiz HTTP → portala ✔, izinli müşteri HTTPS/HTTP ✔, dükkân ağı / DoT 853 / IPv6 engelli ✔, yeniden başlatmada hepsi kendiliğinden ✔ (~25 sn).
- Tuzak: **example.com** bu hattan neredeyse hiç çözülmüyor (PC'den de 7 sn). DNS testlerinde başka ad kullanın.

## Hedef: kurulum kalıbı (ISO) — bütün adımlarda geçerli kurallar
Son ürün, aynı donanımlı boş bir cihaza yüklenen bir ISO olacak; ilk açılışta **sahiplendirme ekranı** (kurulum sihirbazı) gelecek.
- Cihaza elle yapılan her ayar repoda (`deploy/debian/`) dosya olarak durur; kalıp yalnızca repodan üretilir.
- Kalıba cihaza/kafeye özel hiçbir şey girmez: MAC, IP sabitlemesi, SSH sunucu anahtarları (ilk açılışta yeniden üretilir), root parolası, panel hesapları, SMS bilgileri, yedek anahtarı. Hepsini sahiplendirme ekranı sorar veya üretir.
- Uygulama `.deb` paketi olarak paketlenir; ürün paketi GitHub Actions'ta derlenir. **Kalıpta derleyici yoktur.**

## Adım 4 — Rust geliştirme ortamı (2026-10-02)
- WSL kullanılamadı (varsayılan dağıtımın diski yok; diğerleri başka projelerin). Geliştirme döneminde **cihazın kendisinde** derleniyor: `cargo`, `rustc` 1.85.1, `gcc`, `libc6-dev`, `pkg-config` (~1,7 GB kök alanı). Ürün aşamasında kaldırılacak.
- `scripts/gelistir.sh`: kaynağı `tar | ssh` ile `/root/rza`'ya gönderir, `cargo build --release`, `/usr/local/bin/wificorrect`'e kurar (~30 sn). `Cargo.lock` geri alınıp repoda tutulur.
- İskelet: tek program, alt komutlar `portal`, `kaydedici`, `panel`, `ctl`, `surum`.

## Adım 5 — Giriş portalı (Rust) (2026-10-02)
- `src/portal.rs` (eski portal.py), `src/ortak.rs` (common.py'nin portal kısmı), `src/sms.rs` (NetGSM + deneme modu), `src/ayar.rs` (`/etc/wificorrect/ayarlar.toml`, eski UCI'nin karşılığı; örnek: `deploy/debian/etc/wificorrect/ayarlar.ornek.toml`). Şablonlar eski sistemden aynen (`src/sablon/`, programın içine gömülü).
- Kütüphaneler: tiny_http, ureq (rustls), serde/serde_json/toml, libc (flock), getrandom. Debian Rust 1.85 için `.cargo/config.toml` → MSRV'ye uygun sürüm seçimi.
- `wificorrect-portal.service`: Restart=always, `/srv` bağlı değilse açılmaz, yalnızca /srv'ye yazabilir (ProtectSystem=full).
- 21 birim testi geçti (eski test_portal / test_common / test_netgsm karşılıkları).
- Cihazda uçtan uca (sanal müşteri): girişsiz HTTPS anında red ✔, telefon bağlantı kontrolü → 302 portal ✔, form hataları ✔, kod gönderme (deneme modu, kod günlükte) ✔, yanlış kod sayacı ✔, doğru kod → "Bağlandınız" + 1 sn sonra istenen sayfa ✔, nft yetkisi 30 gün ✔, HTTPS çıkış ✔, KAYIT/OTURUM_BASLA/denetim/index kayıtları ✔.
- Ölçüm: program 2,4 MB, portal bellekte 3,8 MB (Python portalı ~20 MB). İlk derleme 4 dk 40 sn.
- Not: test sırasında `/srv/5651/gunluk/2026-10-02/` altına uydurma numarayla (5550000000, "Deneme Musteri") test kayıtları yazıldı; kayıtlar yalnızca eklenerek tutulduğu için silinmedi.

## Adım 6 — Kaydedici, DHCP kaydı, oturum geri yükleme (2026-10-02)
- `src/kaydedici.rs` (eski logger.py): `conntrack -E … -s 10.50.0.0/24` → `trafik.csv`; dnsmasq sorguları `journalctl -f -n 0 -o short-unix -u dnsmasq` → `dns.csv` (satırın kendi zamanıyla); her ikisi kişinin günlük dosyasına. 30 sn'de bir oturum bakımı (süre dolumu, IP değişimi → taşıma, IP başkasına verildi → beklemeye alma), disk %80 uyarı / %95 kişi kopyası durur. Okuyucu ölürse `LOG_BOSLUK`, ENOBUFS'ta kuyruk ve döngü gecikmesiyle `LOG_BOSLUK`. Wi-Fi bekçisi Wi-Fi açılınca eklenecek.
- Eski sistemden iki iyileştirme: (1) IP oturum listesinde yoksa dosyalar o an yeniden okunur — giriş/taşıma sonrası ilk olay telefonsuz yazılmaz; (2) bağlantı kapanışı (BAGLANTI_BITIS) bağlantıyı **başlatan** oturuma yazılır, oturum bu arada başka IP'ye taşınsa bile.
- `src/ctl.rs`: `dhcp-olay` (dnsmasq `dhcp-script`: `add|old|del mac ip [ad]` → `dhcp.csv` + oturum taşıma), `yukle` (açılışta / güvenlik duvarı yenilenince izinli-yasaklı kümeler + oturumlar kalan süreyle), `oturumlar`.
- Sistem: `/etc/wificorrect/conntrack.sysctl` (acct=1, timestamp=1 — Debian'da ikisi de varsayılan kapalıydı; güvenlik duvarı servisi nft'den sonra uygular), `wificorrect-guvenlik.service` → `ctl yukle`, `/usr/local/lib/wificorrect/dhcp-script`, dnsmasq günlük hız sınırı kapalı (`dnsmasq.service.d/wificorrect.conf`), journald 500 MB / 14 gün (DNS sorguları kişisel veri), `wificorrect-kaydedici.service`.
- 35 birim testi geçti (sahadan alınmış gerçek conntrack/dnsmasq satırlarıyla, `tests/fixtures/`).
- Cihazda uçtan uca: DHCP_ATAMA/YENILEME ✔, giriş → trafik + DNS satırları telefonla ✔, IP değişimi → SMS'siz taşıma + `OTURUM_IP_DEGISTI` + yeni IP'nin ilk isteği bile telefonla ✔, kapanış satırları bayt + süreyle ✔, yeniden başlatmada `1 oturum geri yüklendi` + nft'de kalan süre ✔.
- Bellek: portal 4,5 MB, kaydedici 8,5 MB (Python: 20,3 + 11,5 MB).

## Farklı modem ağları (2026-10-02)
- İnternet alan port DHCP istemcisi: modem hangi havuzu verirse (192.168.0.x, 10.x …) IP, ağ geçidi ve DNS kendiliğinden alınır.
- Yönetim erişimi (SSH 22, panel 8443) artık `192.168.1.0/24`'e değil, internet alan portun arkasındaki **bütün özel ağlara** açık (10/8, 172.16/12, 192.168/16). Cihaz doğrudan genel IP alırsa internetten yönetim kapalı kalır.
- Cihaz ekranındaki giriş istemi yönetim adresini gösterir (`/etc/issue.d/wificorrect.issue`, `\4{enp3s0}`).
- Bilinen sınır: modem ağı 10.50.0.0/24 ise müşteri ağıyla çakışır (çok nadir); müşteri ağı sahiplendirme ekranında seçilebilir yapılacak.

## Adım 7 — Mühürleme, doğrulama, saklama, yedek (2026-10-02)
- `src/muhur.rs` (eski ctl.py §15): `ctl gun-kapat [GÜN] [--zorla]` → bugünden önceki mühürsüz her gün (kaçanlar dahil): `*.csv` → `*.csv.gz` (sıkıştırılmış dosya baştan okunup doğrulanmadan orijinal silinmez), `MANIFEST.sha256` (kişi dosyaları dahil), `zincir.txt` hash zinciri (yarım satıra dayanıklı), dosyalar 400. Son 5 dk'da yazılmış dosya varsa `GUN_KAPAT_ERTELENDI`.
- `ctl dogrula [--son N]`: zincir her zaman baştan, dosya hash'leri; bozulan dosyayı adıyla söyler. `ctl temizle [--kuru]`: `retention_days` (730) aşan **ve yedeklenmiş** günler silinir, yedeksiz gün asla (`YEDEKSIZ_GUN`); kişi listesinden 2 yıldır gelmeyenler çıkar. `ctl yedekle`: rsync, günler `--ignore-existing`, zincir/index'in önceki sürümü sunucuda `eski/` altına (1 saat zaman aşımı).
- Zamanlayıcılar (cron yerine): `wificorrect-gun-kapat.timer` 00:15, `wificorrect-gece.timer` 02:00 (yedekle, sonra temizle); `Persistent=true` — cihaz kapalıysa açılışta çalışır.
- RFC 3161 zaman damgası henüz yok (sağlayıcı seçilince); hash zinciri her durumda çalışıyor.
- 38 birim testi geçti (mühürleme/erteleme/zincir, bir bayt değişince dosya adıyla yakalama, zincir kırılması, yalnızca yedeklenmiş eski günlerin silinmesi, yedek hata/yeniden deneme).
- Yedek: `[backup] enabled = false` — sunucu hesabı ve SSH anahtarı bekleniyor.

## Bekleyen işler (kullanıcı kararıyla ertelendi)
- **Uzak yedek — arşiv sunucusu kurulunca yapılacak** (2026-10-02 kullanıcı kararı): cihazda yeni SSH anahtarı üret (`/root/.ssh/yedek_anahtar`), sunucuda kafe hesabına (`kafe-<ad>`, salt-yazma, rrsync) ekle, `/etc/wificorrect/ayarlar.toml` → `[backup] enabled = true`, `target = "kafe-<ad>@<sunucu>:"`; `ctl yedekle` ile dene. O zamana kadar günler cihazda mühürlenip birikir; `temizle` yedeklenmemiş günü silmez.
- Eski OpenWrt cihazının kayıtları/ayarları yeni sisteme taşınmayacak (kullanıcı kararı).

## Twilio + ülke kodu seçimi (2026-10-02, kullanıcı isteği)
- Portal formunda telefonun yanında ülke seçimi: varsayılan 🇹🇷 +90, 159 ülke (`src/ulkeler.rs`; bayrak ISO kodundan üretilir, JS yok). Numara `+…`/`00…` ile yazılırsa seçim yok sayılır. Türkiye için 5XX XXX XX XX kuralı, diğerlerinde 4–14 haneli ulusal numara.
- **Telefon biçimi değişti:** her yerde uluslararası biçim (E.164, + olmadan): `905334553132`, `4915123456789` — oturum, CSV `telefon` sütunu, kişi dosyası adı, kişi listesi. Ekranda `+90 5XX XXX XX 32`.
- SMS yönlendirme (`sms::route`): Türk numaraları `[sms] provider` (netgsm|twilio); yabancı numaralar Twilio açıksa Twilio'dan, değilse "Yabancı numaralara şu an SMS gönderilemiyor". NetGSM'e 10 haneli ulusal numara gider. Deneme modu tek anahtar: `[sms] mock` (iki sağlayıcıyı da kapsar).
- `src/twilio.rs` (eski twilio.py): Verify API; gerçek modda kodu Twilio üretir/doğrular, ağ hatasında deneme hakkı düşmez; doğrulama sürerken kod yenilenirse sonuç eski koda uygulanmaz. Yerel ayar: Türk numarası `tr`, diğerleri `en`.
- KVKK aydınlatma metnine yurt dışı aktarım (Twilio, ABD) eklendi — **hukukçuya onaylatılmalı (KVKK m.9).**
- 44 birim testi geçti; cihazda: varsayılan +90 seçili, Almanya / `+49` numarası Twilio kapalıyken düzgün reddediliyor (`OTP_ISTEK red=yabanci_numara`), Türk numarası NetGSM (deneme) ile kod alıyor.

## Adım 8a — Yönetim paneli çekirdeği (2026-10-02)
- `wificorrect panel` → `wificorrect-panel.service`, https://<WAN IP>:8443 (yalnızca dükkân ağı; müşteri ağından istek 403). İlk açılışta `/etc/wificorrect/panel.{crt,key}` öz-imzalı (EC P-256, 10 yıl) cihazda üretilir; kalıba girmez.
- **Hesaplar (2026-10-03, kullanıcı kararı):** hizmet sağlayıcı hesabı sabit **`admin`** (root gibi; parolasını yalnızca hizmet sağlayıcı bilir). Parola cihaz konsolundan/SSH ile `wificorrect ctl admin-parola` (ekrana yazılmaz) ile konur; ISO'da kalıba gömülecek. `admin` silinemez, başka hesap bu adı alamaz. **İlk açılış = `/kurulum`:** yalnızca kafe adı (boş gelir) + kafe sahibinin kullanıcı adı ve parolası (≥ 10). Kurulum bitene kadar panelde kafe adı yerine "WifiCorrect" yazar; `admin` kurulumdan önce de `/giris`ten girebilir. Hesaplar `/etc/wificorrect/hesaplar.json` (600, PBKDF2-SHA256 120.000, kullanıcı başına tuz); bozuk dosya kurulum ekranını açmaz.
- Giriş: 5 hatalı deneme / 15 dk (IP ve kullanıcı başına) kilit; çerez `HttpOnly; Secure; SameSite=Strict`, 12 saat; her POST'ta CSRF. Parola değişince kullanıcının diğer oturumları kapanır.
- Sayfalar: Özet (bağlı cihaz, bugün farklı kullanıcı, SMS/tavan, disk, uyarılar) · Bağlı cihazlar (+ bağlantı kes) · Yasaklı / izinli cihazlar · Ayarlar · Sistem (servisler, günü kapat, bütünlüğü doğrula, yeniden başlat) · Hesaplar (yalnızca hizmet sağlayıcı) · Şifremi değiştir.
- Roller (2026-10-03, kullanıcı kararı; eski "sahip kayıt görmez" kuralını değiştirir): kafe sahibi her şeyi admin gibi görür ve yönetir (bağlı cihazlarda numara/ad açık, ileride kayıtlar da). Admin'in tek farkı **API ayarları** sayfası (`/api`: SMS deneme modu, sağlayıcı, NetGSM ve Twilio bilgileri). Kafe sahibi Hesaplar sayfasında kafe sahibi rolünde hesap açar/siler/sıfırlar; admin'e ve hizmet sağlayıcı hesaplarına dokunamaz, API yetkili hesap açamaz. Şifre alanları yalnızca yazılır (boş = aynı kalır), denetimde `***`. Her işlem `denetim.csv`'ye `PANEL_*`.
- Ayar kaydı `ayarlar.toml`'u yeniden yazar (yorumlar gider; örnek dosyada duruyor) ve portalı yeniden başlatır; bağlı müşteriler düşmez.

## Adım 8b — Kayıtlar, kullanıcılar, resmi talep (2026-10-03)
- Panel (iki rol de görür): **Kayıtlar** (gün listesi: açık / mühürlü / yedeklendi; dosyayı tabloda görüntüleme, 500 satırlık sayfalar, satır araması; tek dosya indirme — mühürlü `.gz` açılmış CSV olarak, Excel'de açılır; günü paket olarak indirme), **Kullanıcılar** (index.csv'den liste + arama, kişi sayfası: oturumlar ve kaydı olan günler), **Resmi talep** (iç IP / NAT portu / hedef IP + zaman ± tolerans, telefon, MAC; talep paketi: günlerin mühürlü dosyaları + MANIFEST + zincir.txt + doğrulama çıktısı, `.tar`, en fazla 92 gün).
- Komut satırı: `wificorrect ctl ara (--ic-ip IP | --nat-port P | --hedef-ip IP) --zaman "2026-10-02 21:20" [--tolerans SN]`, `ctl ara --telefon N | --mac M [--zaman …]`, `ctl disa-aktar --baslangic G --bitis G --cikti DOSYA`.
- İç IP araması oturumun IP değişimlerini izler (o anda IP'yi hangi oturum tutuyordu). Dosya yolları yalnızca tarih + izinli dosya adı kalıbıyla kabul edilir (dışarı çıkılamaz). Her görüntüleme / indirme / arama denetime: `PANEL_KAYIT_GORUNTULE`, `PANEL_KAYIT_INDIR`, `PANEL_KULLANICI(_ARA)`, `PANEL_TALEP_ARA`, `PANEL_TALEP_PAKET`.
- Cihazda doğrulandı: 2026-10-02 gecesi ilk otomatik mühür `✔ hash ✔ zincir`; gerçek kayıtta iç IP araması doğru oturumu buldu; talep paketi 2 günü içerdi.
- Not: 2026-10-02'deki deneme satırları eski 10 haneli numarayla (`5550000000`) yazılmış; yeni kayıtlar `90…` biçiminde.

## Adım 8c — Portlar ve Wi-Fi (2026-10-03)
- Tek kaynak `/etc/wificorrect/ag.toml` (yoksa varsayılan: enp3s0 alır, enp1s0 verir, Wi-Fi kapalı). Ondan üretilenler: `/etc/network/interfaces.d/wificorrect` (WAN `allow-hotplug … dhcp` + `hwaddress`, `br-hotspot` köprüsü), `/etc/wificorrect/arayuzler.nft` (`define WAN`, `guvenlik.nft` include eder), `/etc/wificorrect/hostapd.conf` (Wi-Fi açıksa; 600), `/etc/issue.d/wificorrect.issue`. `/etc/network/interfaces` artık yalnızca lo + `source`; dnsmasq'tan `no-dhcp-interface` kaldırıldı (zaten yalnızca köprüde dinliyor).
- Panel → **Portlar** (iki rol): her Ethernet için internet alır / verir / kapalı, canlı kablo durumu ve hız; Wi-Fi verir / kapalı, ağ adı, şifre (boş = şifresiz), kanal 1-13; sağ/sol etiketi değiştirme. Kural: tam bir Ethernet alır, en az bir port (Ethernet ya da Wi-Fi) verir.
- Uygulama: panel yeni ayarı `/var/lib/wificorrect/ag-yeni.toml`'a yazar, `systemd-run` ile 2 sn sonra `wificorrect ctl ag-gecis` çalışır (panel bağlantısı kopsa da). Önce 180 sn'lik geri alma zamanlayıcısı kurulur, sonra: `ifdown -a` (eski dosyalarla) → dosyalar → `ifup -a` + `ifup <wan>` → `nft -f guvenlik.nft` → dnsmasq → `wificorrect-wifi` (hostapd). Panelden **Onayla** denmezse eski ayar döner; **Hemen geri al** düğmesi de var. Bekleyen değişiklik varken cihaz yeniden açılırsa `wificorrect-ag-acilis.service` ağ kalkmadan eski dosyaları yazar. WAN başka porta geçince eski WAN'ın MAC'i yeni porta yazılır (modemdeki .110 rezervasyonu korunur); müşteri tarafına dönen port `permaddr` ile kendi MAC'ine döndürülür. Denetim: `PANEL_PORT`, `PANEL_PORT_ONAY`, `PANEL_PORT_GERI_AL`, `PANEL_PORT_ETIKET`, `AG_UYGULANDI`, `AG_ONAYLANDI`, `AG_GERI_ALINDI neden=sure|elle|acilis`.
- Cihazda doğrulandı (ölü adam anahtarlarıyla): elle yazılmış ağ dosyalarından üretilene geçiş (yeni SSH ile erişim, sahte istemci DHCP aldı, portala yönlendi, HTTPS reddedildi); Wi-Fi test yayını (`WifiCorrect-Test`, WPA2, kanal 6, köprüde) açıldı ve onaylanmayınca 3 dk sonra kendiliğinden kapandı; WAN bilerek kablosuz porta (enp1s0) taşındı → cihaz erişilemez oldu → 3 dk sonra eski ayar geri geldi, aynı IP (192.168.1.110) ile erişim döndü.
- Yapılmadı (gerekince): Wi-Fi'nin internet alması (istemci modu; `wpasupplicant` paketi gerekir), ath9k "bağlı ama internet yok" bekçisi (Wi-Fi gerçekten kullanılınca bakılacak).

## Yasaklı siteler ve kelimeler (2026-10-03)
- Panel → **Yasaklı siteler** (iki rol). Listeler boş başlar (kullanıcı kararı); kafe sahibi yazar. Ayar `ayarlar.toml` `[filtre]` (`siteler`, `kelimeler`, `istisnalar`).
- **Site** → `/etc/wificorrect/yasak-siteler.conf` (`address=/site/`, dnsmasq `conf-file` ile okur): alan adı ve bütün alt adları NXDOMAIN; sorgu DNS kaydına düşer.
- **Kelime** → `iptables` (nf_tables arka ucu) + `xt_string`: `INPUT -i br-hotspot -p udp|tcp --dport 53 -j WFC_DNS`; zincirde önce istisnalar `RETURN`, sonra kelimeler `REJECT` (UDP: port ulaşılamaz → "site bulunamadı"; TCP: sıfırlama), büyük/küçük harf duyarsız. Bütün DNS cihaza yönlendirildiği için başka DNS yazmak atlatmaz; DoH atlatır. Kelime başına eşleşme sayısı panelde.
- Paket: `iptables` kuruldu (Debian 13, 1.8.11, nf_tables). Açılışta `wificorrect-guvenlik` → `ctl filtre-uygula` (kurallar kalıcı değil, her açılışta yeniden).
- Panelde "Bu adres engelli mi?" denemesi; değişiklik hemen geçerli (site listesi değişince dnsmasq yeniden başlar, kelimede yalnızca zincir). Denetim: `PANEL_FILTRE_EKLE`, `PANEL_FILTRE_KALDIR`.
- Cihazda sahte istemciyle doğrulandı (sorgu 8.8.8.8'e gönderilse de): site → NXDOMAIN, `bet` kelimesi → `superbet.com.tr`, `m.bet365.com` reddedildi, `alphabet` istisnası açık, `google.com` açık. Test sonrası listeler boşaltıldı.

## AP (erişim noktası) — 2026-10-03
- Kuralı (kullanıcı): her kafede AP farklı olabilir; hepsi önceden hazırlanmış gelir. Sistem AP'ye özel hiçbir şey bilmez/gerektirmez: AP sol porta (müşteri köprüsü) takılır, müşteriler IP'yi cihazdan alır.
- AP hazırlık listesi: çalışma modu Access Point · **DHCP sunucusu kapalı** (Smart IP / otomatik DHCP gibi "gerekirse kendin dağıt" modları da kapalı; yönetim IP'si sabit, ör. 10.50.0.2/24, ağ geçidi 10.50.0.1 — DHCP havuzu .20–.249 dışında) · ağ adı önceden belirlenmiş · şifresiz · **istemci yalıtımı (AP isolation) açık** · **yönetim parolası fabrika ayarında bırakılmamalı** (ağ şifresiz; müşteri AP'nin kendi sayfasına ulaşabilir).
- İlk denenen: TP-Link TL-WA901ND (70:4f:57:dd:7a:e8), 100 Mb/s; `GoztepeBilgisayar_Misafir`, şifresiz, DHCP kapalı, AP isolation açık, sabit 10.50.0.2. Telefon AP üzerinden 10.50.0.x aldı, portal açıldı. AP yönetimine bilgisayardan: `ssh -L 8091:10.50.0.2:80 wificorrect` → http://127.0.0.1:8091 (TP-Link girişten sonra kendi IP'sine yönlendirir; aynı yolu tünel adresiyle açmak gerekir).
- Portal başlığı işletme adını kurulum ekranından alır (`main.site_name`): "<ad>'ye hoş geldiniz", ek ünlü uyumuyla kesme işaretinden sonra (Bocafe'ye, Starbucks'a, Hilton Otel'e). Bu cihazdaki eski sistemden kalma "Bocafe" adı varsayılana ("İşletme") çevrildi; kurulumda girilen ad geçer.

## Panel hareketleri — kafe sahibinin izlenmesi (2026-10-03, kullanıcı isteği)
- İstek: kafe sahibinin hesabının da kaydı tutulsun; kafe sahibinin müşterileri sürekli izlemesi istenmiyor.
- Kişisel veri gösteren her sayfa açılışı denetime yazılır, kim + rol + IP ile (`kullanici=mudur rol=sahip ip=…`): `PANEL_OTURUMLAR` (bağlı cihazlar), `PANEL_KULLANICILAR` / `PANEL_KULLANICI_ARA` / `PANEL_KULLANICI`, `PANEL_KAYIT_GUN`, `PANEL_KAYIT_GORUNTULE` (her sayfa), `PANEL_KAYIT_INDIR`, `PANEL_TALEP_ARA`, `PANEL_TALEP_PAKET`; ayrıca `PANEL_GIRIS`, `PANEL_CIKIS`. Denetim günlük mühürlenir; panelden silinemez.
- Bu sayfalarda uyarı: "her görüntüleme, arama ve indirme kimin yaptığıyla birlikte kaydedilir ve hizmet sağlayıcı tarafından denetlenir".
- **Panel hareketleri** sayfası (yalnızca admin): hesap başına son 7 gün / dönem kişisel veriye bakma sayısı, son giriş; hesap ve "yalnızca kişisel veri" süzgeci, son 1000 hareket.

## Geliştirme notu: cihaza erişim iki kez koptu (2026-10-03) — sebep benim test yöntemimdi
- Belirti: SSH ve ping'e (ARP) cevap yok; iki kez fişten kapatıp açmak gerekti. İlk teşhis (derlemede bellek tükenmesi / ısınma) **yanlıştı**: ikinci olayda önceki açılışın günlüğü cihazın sağlam çalıştığını (testler bitti, cron çalıştı) ve düzgün kapandığını gösterdi; sıcaklık 46–48 °C.
- Gerçek sebep: sahte istemci testlerinde temizlik için `ip netns exec … pkill dhcpcd` çalıştırmıştım. netns süreçleri ayırmaz → cihazın **kendi WAN dhcpcd'si** de öldü (günlük: 16:32:15 `dhcpcd[749]: received SIGTERM`); WAN IP'si kira süresi dolunca düştü. `dhcpcd -1` kira alınca zaten kendiliğinden çıkar, `pkill` gereksizdi.
- **İkinci, kalıcı hata (aynı gün bulundu):** ağ geçişi (`ag::switch`: port değişikliği, fabrika dönüşü, 8c geçişi) `ifup <wan>`'ı geçici bir systemd işinden çalıştırıyordu → dhcpcd o işin cgroup'unda doğuyor, iş bitince systemd onu da öldürüyordu; IP kira sonunda (modem kirası 1 saat) düşüyordu. İlk kopma 8c geçişinden ~1 saat sonraydı. Düzeltme: WAN artık `systemctl restart ifup@<wan>.service` ile kalkar (dhcpcd her zaman o birimde yaşar). Cihazda doğrulandı: `ctl ag-uygula` geçici işten çalıştı, iş bitti, yeni dhcpcd `ifup@enp3s0.service` içinde yaşıyor.
- **WAN DHCP bekçisi** (kaydedici, 30 sn'de bir): internet portunda kablo takılıyken dhcpcd 1 dk'dan uzun yoksa `ifup@<wan>.service` yeniden başlatılır (en çok 5 dk'da bir), denetime `WAN_DHCP_YENIDEN`. Kablo yoksa ya da dhcpcd var ama IP alamıyorsa dokunmaz. Cihazda denendi: dhcpcd bilerek durduruldu, 85 sn sonra geri geldi, IP ve internet sağlam.
- Önlem: sahte istemci yalnızca `scripts/sahte-istemci.sh` ile (dhcpcd'ye sinyal göndermez, netns'i temizler). Derleme/test bellek sınırı (`scripts/test.sh`, `gelistir.sh`, MemoryMax=1200M) zararsız olduğu için kaldı; testler optimizasyonsuz ~2,5 dk.

## Giriş sayfası: üç metin ve açık tasarım (2026-10-03, kullanıcı isteği; örnek Starbucks portalı)
- Üç metin: **Açık Rıza Metni** (onay kutusu, varsayılan isteğe bağlı — KVKK'da hizmet rızaya bağlanamaz; panelden zorunlu yapılabilir), **İnternet Kullanım Sözleşmesi** (onay kutusu, zorunlu), **Aydınlatma Metni** (yalnızca açılır metin). Eski "KVKK okudum" kutusu ve `/kvkk` sayfası kaldırıldı.
- Metinler `<details>` ile formun içinde açılır (JS yok; ayrı sayfaya gidip dönünce telefonların giriş penceresinde yazılanlar silinmesin).
- Metinler boş başlar; panel → **Portal metinleri** (iki rol) — `ayarlar.toml` `[portal]` (`aydinlatma`, `acik_riza`, `sozlesme`, `acik_riza_zorunlu`). Düz metin, boş satır paragraf; kaçışlanır. Kaydedince portal yeniden başlar; denetim `PANEL_PORTAL_METIN` (değişen metin ve uzunluğu).
- Oturum kaydına onaylar yazılır: `OTURUM_BASLA` ek `… sozlesme=1 riza=0|1`.
- Tasarım: açık zemin, üstte gökyüzü bandı, üstüne binen beyaz kart, yeşil düğme, mavi metin bağlantıları.
- **Sözleşme metni** (2026-10-03, kullanıcı metni): "Son Kullanıcı Lisans Sözleşmesi" ürünle gelir (`src/sablon/sozlesme.txt`, `[portal] sozlesme` boşsa değil, anahtar yoksa varsayılan); panelden değiştirilebilir. Metindeki **İŞLETMECİ** giriş sayfasında kafe adıyla değişir; kesme işaretinden sonraki ek ünlü uyumuna göre yeniden kurulur (Bocafe’nin, Starbucks’un, Kahvecim’den), büyük harfli başlıkta ad da büyük (BOCAFE’NİN). İyelik ekli adlarda (Kahve Dünyası’ya) genel kural uygulanır. Köşeli parantezli bağlantılar düz `www.wificorrect.com` yazıldı (müşteri giriş yapmadan internete çıkamaz). Tamamı büyük harfli satırlar kalın gösterilir.
- **KVKK Aydınlatma Metni** (2026-10-03, kullanıcı metni; veri sorumlusu Göztepe Bilgisayar): ürünle gelir (`src/sablon/aydinlatma.txt`), giriş sayfasındaki başlık "KVKK Aydınlatma Metni". Metinlerde `* ` / `• ` / `- ` ile başlayan satırlar madde listesi olarak gösterilir.
  - **İşletmeden bağımsız (2026-10-15, 0.4.12):** veri sorumlusu ve iletişim bölümü işletmenin bilgileriyle dolar (aşağıda "KVKK veri sorumlusu bilgileri"); Göztepe adresi/telefonu/KEP/e-posta metinden ve koddan kaldırıldı. Ayar dosyasında eski varsayılan (sha256 ile tanınır) kayıtlıysa okunurken yenisiyle değişir; işletmenin düzenlediği metne dokunulmaz.

## Her işletme için (2026-10-03, kullanıcı kararı)
- Ürün yalnızca kafelere değil otel ve internet veren her işletmeye: arayüzde "kafe" → "işletme" (kurulum: **İşletme adı**, rol: **İşletme sahibi**, Ayarlar: İşletme adı).
- Kurulumda ve Ayarlar'da **İşletme unvanı** (`main.unvan`, 2-200 karakter), notu: "Vergi levhasındaki unvan yazılmalıdır".
- **Açık rıza**: metin (kullanıcı metni, `src/sablon/acik_riza.txt`; "okudum e bu" yazım hatası "okudum ve bu" yapıldı) Starbucks örneğindeki gibi onay kutusunun yanında yazar; işaretlemek **her zaman zorunlu** (eski isteğe bağlı/zorunlu seçeneği kaldırıldı — hukuki değerlendirme kullanıcıda). Sözleşmenin adı her yerde açık rıza metnindeki gibi "İnternet Kullanıcı Sözleşmesi" (metnin kendi başlığı "Son Kullanıcı Lisans Sözleşmesi" kullanıcı metni olarak kaldı). Metindeki `[vergi levhası unvanı]` → `main.unvan`; unvan girilmemişse yer tutucu görünür. Panelden düzenlenebilir.

## Admin ayarları ve fabrika ayarları (2026-10-03, kullanıcı isteği)
- "API ayarları" → **Admin ayarları** (`/admin-ayarlari`, yalnızca admin): SMS deneme modu, sağlayıcı, NetGSM, Twilio, **SMS sınırları**, **kayıt saklama süresi**, **uzak yedek** (sunucu adı dahil), sağlayıcı durumu ve fabrika ayarları. İşletme sahibinin Ayarlar sayfasında yalnızca işletme bilgileri (ad, unvan, oturum süresi, telefon başına cihaz).
- İşletme sahibi sağlayıcı ve sunucu adlarını hiçbir yerde görmez: Özet'te yalnızca "Müşterilere doğrulama SMS'i şu an gönderilmiyor; hizmet sağlayıcınıza başvurun" (ayrıntısız), Sistem'de uzak yedek satırı yok.
- **Fabrika ayarları** (admin parolası + onay kutusu): `wificorrect ctl fabrika` ayrı systemd işinde — açık oturumlar kapanır (`neden=fabrika`), admin dışındaki hesaplar silinir (kurulum ekranı gelir), ayarlar ürün varsayılanı (işletme adı/unvanı, SMS bilgileri, yedek, metinler, yasaklı listeler; sistem yolları ve müşteri ağı adresleri korunur), yasaklı listeler uygulanır, portlar varsayılan (Ethernet 1 alır, Ethernet 2 verir, Wi-Fi kapalı; bekleyen port değişikliği silinir), servisler yeniden başlar. Denetim: `PANEL_FABRIKA`, `PANEL_FABRIKA_RED`, `FABRIKA_IPTAL`, `FABRIKA_AYARI`. Cihazda henüz çalıştırılmadı.
- **5651 kayıtları ve fabrika dönüşü** (kullanıcı kararı, seçenek 3): uzak yedek kapalıysa işlem başlamaz. Açıksa: kayıt yazan servisler durur, oturumlar kapanır, bugün dahil bütün günler mühürlenir ve sunucuya gönderilir (zincir.txt dahil — yedek artık zincir gönderilemezse de hata verir); bir gün bile gönderilemezse **iptal** (hiçbir şey silinmez, ayarlara dokunulmaz, servisler geri açılır). Hepsi gönderildiyse yerel kayıtlar (günler, zincir.txt, index.csv) silinir; işletme sahibi kayıtlarını sunucudaki panelden görür. Yeni işletmenin kaydı `FABRIKA_AYARI` satırıyla başlar.
- **Şimdi yedekle ve durum** (2026-10-03): Admin ayarları'nda "Şimdi yedekle" (`systemd-run … ctl yedekle`) ve Durum'da "Son yedek" (denetimdeki son `YEDEK_SONUC`: zaman + sonuç ya da HATA). Yedek artık gönderilecek gün olmasa da zincir.txt'yi gönderir → her çalıştırmada sunucu bağlantısı denenir; `muhur::backup` sonucu `Result`. Fabrika dönüşü sürüyorsa ya da iptal olduysa Admin ayarları'nın en üstünde mesaj (`PANEL_FABRIKA` / `FABRIKA_IPTAL` + sebep).

## Uzak erişim — WireGuard, kendi sunucumuz (2026-10-03, kullanıcı kararı: seçenek A)
- Cihaz: `wireguard-tools` kuruldu (kernel modülü Debian'da var). Gizli anahtar cihazda üretilir (`/etc/wificorrect/wg.key`, 600), cihazdan çıkmaz; açık anahtar Admin ayarları → Durum'da. Ayar `[uzak]` (`enabled`, `sunucu` adres:port, `sunucu_anahtar`, `adres` 10.99.0.x) yalnızca admin. Kaydedince `systemd-run … ctl uzak-uygula` → `/etc/wireguard/wfc.conf` + `wg-quick@wfc` (açılışta kendiliğinden). Tünel yalnızca 10.99.0.0/24'ü taşır (AllowedIPs), PersistentKeepalive 25 (NAT arkası). Denetim `UZAK_ERISIM`.
- Güvenlik duvarı (`guvenlik.nft`): `iifname "wfc" ip saddr 10.99.0.0/24` → yalnızca tcp 22, 8443 ve ping; forward yok (müşteri/modem ağına geçiş yok).
- Bekçi (kaydedici): uzak erişim açıkken arayüz yoksa ya da 10 dk'dır el sıkışma yoksa `wg-quick@wfc` yeniden başlatılır (en çok 10 dk'da bir; sunucu adı açılışta çözülemediyse ya da IP değiştiyse), denetim `UZAK_YENIDEN`.
- Sunucu: `scripts/sunucu/wg-sunucu.sh` (`kur <genel-adres>`, `yonetici-ekle <ad>`, `cihaz-ekle <ad> <anahtar>`, `liste`). Adresler: sunucu .1, yöneticiler .2–.9, cihazlar .10–.254. Sunucu yönlendirmesi: yalnızca yönetici → cihaz (22, 8443, ping), cihaz ↔ cihaz ve tünelden internete yok. UDP 51820 açık olmalı.
- **Sunucu kuruldu (2026-10-03):** `gbservers`, Ubuntu 26.04, 192.168.1.109 (dükkân ağı; modemin dış IP'si <dış-ip>, sabit). Giriş `gbserver` + `~/.ssh/gbserver` anahtarı; `sudo` parolalı → kurulum/ekleme komutlarını kullanıcı çalıştırır. Ubuntu 26.04'te `wg-quick` kısıtlı çalışıyor: PostUp içinden `sysctl`/`nft` "permission denied" → betik yönlendirmeyi `/etc/sysctl.d/99-wificorrect-vpn.conf`'a, kuralı `wificorrect-vpn.service`'e taşıdı (`kur` tekrar çalıştırılabilir). Eşler: HUAWEI-PC = 10.99.0.2 (yönetici), Bocafe-test = 10.99.0.10 (bu cihaz).
- Bu cihaz sunucuyla aynı LAN'da: uç nokta `192.168.1.109:51820` (modem dış IP'ye içeriden dönmeyebilir). Uzaktaki işletmeler `<dış-ip>:51820` kullanır — modemde (Aidata, Güvenlik Duvarı → Port Yönlendirme) `wificorrect-vpn` UDP 51820 → 192.168.1.109:51820, 1_INTERNET eklendi (2026-10-03; Uygula kullanıcı tarafından).
- Doğrulandı: el sıkışma; sunucudan cihaza ping 1 ms, panel 8443 → 200, SSH 22 açık, 8080 ve 53 kapalı.
- Yönetici bağlantı dosyaları (gizli anahtar içerir, repoda değil): Masaüstü `wificorrect-vpn/wificorrect-dukkan.conf` (iç adres) ve `wificorrect-disaridan.conf` (dış IP). Sunucudaki geçici kopya silindi.

## Merkez sunucu programı ve yedek arşivi (2026-10-03, kullanıcı kararı)
- `scripts/sunucu/wificorrect-sunucu` (eski `wg-sunucu.sh` yerine; sunucuda `/usr/local/sbin/wificorrect-sunucu`): `kur <genel-ad>`, `yonetici-ekle`, `cihaz-ekle <ad> <wg> "<ssh>"`, `yedek-ekle <ad> "<ssh>"`, `liste` (son el sıkışma + son yedek), `tasi-paketle <dosya>`, `tasi-kur <dosya>`. Kayıt: `/etc/wireguard/wificorrect/kayit.csv`. Sunucu adla bilinir (`vpn.wificorrect.com`, DNS A → <dış-ip>); sunucu değişince `tasi-paketle` → yeni sunucuda `tasi-kur` → DNS'i yeni IP'ye çevir; cihazlara dokunulmaz (aynı anahtarlar, tünel bekçisi 10 dk'da yeniden bağlanır).
- Yedek arşivi: `/srv/wificorrect-arsiv/<ad>/veri`. Her cihaza ayrı sistem hesabı `wfc-<ad>`; `authorized_keys`: `restrict,from="<tünel-ip>",command="rrsync -wo -no-del …/veri"` (root'a ait, cihaz değiştiremez). Cihaz tünelden gönderir: hedef `wfc-<ad>@10.99.0.1:`. SSH ana makine anahtarı tutulmaz (`StrictHostKeyChecking=no`): sunucunun kimliğini WireGuard doğrular, sunucu taşınınca yedek kırılmaz. Yedek anahtarı cihazda `/etc/wificorrect/yedek_anahtar` (eski `/root/.ssh` yolu panel tarafından okunamıyordu).
- Admin ayarları → Durum'da "Sunucuya tanıtma komutu": `sudo wificorrect-sunucu cihaz-ekle <işletme-adından> <wg> "<ssh>"` (kopyalanır, sunucuda bir kez çalıştırılır).
- Cihazda doğrulandı: 2026-10-02 + zincir.txt tünelden gönderildi (`.yedeklendi`); hesaptan kabuk komutu, okuma ve `--delete` reddedildi; `gbserver` arşivi okuyamıyor (yalnızca root).
- **DNS** (2026-10-03): Güzel Hosting (NS `*.guzelhosting.com`, panel guzel.net.tr → Alan Adı → DNS Yönetimi): `vpn.wificorrect.com` A → <dış-ip> eklendi, ~5 dk'da yayına girdi (8.8.8.8 de çözüyor). `panel.wificorrect.com` zaten aynı IP'de. Yönetici dosyası `wificorrect-disaridan.conf` uç noktası `vpn.wificorrect.com:51820` yapıldı (sunucu taşınsa da değişmez).

## Yönetim merkezi — yonetim / panel / api.wificorrect.com (2026-10-04, kullanıcı kararı)
- Tasarım `docs/superpowers/specs/2026-10-04-yonetim-merkezi-design.md`, plan `docs/superpowers/plans/2026-10-04-yonetim-merkezi-sunucu.md`, kod `scripts/sunucu/merkez/` (Python stdlib). Müşteri hesabı merkezde açılır: **7 haneli rastgele numara + parola**; cihaz ilk girişte kendini bağlar, tünel ve yedek hesabı root kuyruğuyla kendiliğinden açılır.
- **DNS:** Güzel Hosting'e `yonetim` ve `api` A → <dış-ip> eklendi (kök `wificorrect.com` hostingde, 185.106.208.2; dokunulmadı).
- **Kurulum:** `scripts/sunucu/dagit.sh` (sudo parolası kullanıcıda). Birimler: `wc-merkez` (wcpanel, 127.0.0.1:8081, Caddy arkasında üç alan adı), `wc-kuyruk.path/.service` (root işleri: `wificorrect-sunucu cihaz-ekle/cihaz-kapat`), `wc-durum.timer` (5 dk, arşiv/tünel → `durum.json`), `wc-istatistik.timer` (arşivleri klasörlerden alır). fail2ban `wc-merkez` (5 hata, dükkân ağı 192.168.1.0/24 ve VPN hariç). Yeni yedek hesapları `rrsync -munge`.
- Eski kafe paneli kapatıldı (`wc-panel` silindi; `hesaplar.json` → `hesaplar.json.eski`, eski Caddyfile → `Caddyfile.eski`). Eski `bocafe` arşivi "Eski arşivler"de, bağsız (kullanıcı kararı: Bocafe müşteri olarak açılmadı).
- **Yönetici:** `admin` (`sudo wificorrect-sunucu yonetici-parola admin`; parola kullanıcıda). Müşteriler: **Göztepe Bilgisayar** (test cihazı cihaz planında bununla girecek) ve **Deneme** (4511643).
- **Uçtan uca deneme (2026-10-04):** cihazda geçici anahtarla `api/giris` → 200, tünel 10.99.0.11, `/srv/wificorrect-arsiv/4511643` + `wfc-4511643` (from=10.99.0.11, -munge) açıldı; yönetimde "zorla ayır" → `cihaz-kapat tamam`, `authorized_keys` boşaldı, arşiv kaldı; Deneme'nin üyeliği bitirildi. `wc-istatistik` yeni cihaz arşivinin (`bocafe-test`) Rust MANIFEST biçimini sorunsuz özetledi.
- Bekleyen (cihaz planı): kurulum ekranının kalkması, cihaz girişinin merkeze bağlanması, günlük 06:00 eşitlemesi; `bocafe-test` elle eklenmiş tünelinin `cihaz-kapat` ile kapatılıp cihazın Göztepe Bilgisayar numarasıyla yeniden kaydı.

## Cihaz yönetim merkezine bağlandı; lisans ve parola görme (2026-10-05)
- **Sunucu IP'si:** modem 2026-10-05 sabahı yeniden başladı, sunucu DHCP'den .107 aldı → 80/443/2222/51820 yönlendirmeleri (.109) boşa düştü. Modemde **MAC tabanlı atama** eklendi: `1c-1b-0d-09-ee-e4 → 192.168.1.109` (cihaz `00-0e-c4-ce-a0-9b → .110` zaten vardı). Sunucuda `networkctl down/up` ile .109'a geçti; `yonetim`/`panel` dışarıdan 200.
- **Sunucu güncellemesi:** müşteri parolası yönetimde görünür (`parola_acik`, açık saklanır — kullanıcı kararı); yıllık lisans (`lisans_bitis`, `askida`; yeni müşteriye 1 yıl; mevcutlara oluşturmadan 1 yıl); yönetimde "Lisansı 1 yıl uzat / Askıya al / Bitişi ayarla"; API `lisans`, `lisans_bitis` döner. Ödeme ekranı sonradan `veri.lisans_uzat`'ı çağıracak.
- **Cihaz sürümü:** kurulum ekranı yok; giriş müşteri numarasıyla; lisansı kapalı ya da bağlı olmayan cihaz misafire internet vermez (portal kapalı, açık oturumlar kapanır); lisans kapalıyken 30 dk'da bir, açıkken her gün 06:00'da merkezle eşitlenir.
- **Geçiş:** eski elle eklenmiş tünel kaydının adı yeni ad kuralına uymadığı için kuyruk kapatamadı (düzeltildi, sunucuya bir sonraki güncellemede gider). Cihazın WireGuard anahtarı yenilendi (eskisi `/etc/wificorrect/wg.key.bocafe-test.yedek`); yarım kalan bağ yönetimden "zorla ayır" ile silindi; cihaz **Göztepe Bilgisayar = 1537344** ile bağlandı: tünel 10.99.0.11, yedek `wfc-1537344@10.99.0.1:`, lisans 2027-10-04'e kadar. `ctl merkez-eslesme` → `MERKEZ_ESLESME uyelik=aktif`; `ctl yedekle` → zincir gönderildi.
- **Bekleyen (sudo'lu bir sonraki sunucu güncellemesi):** `wificorrect-sunucu cihaz-kapat` eski ad düzeltmesi; sunucuda kalan eski `bocafe-test` WireGuard eşinin kapatılması (`sudo wificorrect-sunucu cihaz-kapat <eski ad>`).
- Cihazın misafir portu (Ethernet 2) kablosuz/AP kapalı görünüyor (2026-10-05 13:50).

## Kurulum ISO'su (2026-10-05/07)
Dal `kurulum-iso`; tasarım `docs/superpowers/specs/2026-10-05-kurulum-iso-design.md`. Boş cihaz USB'den açılır, tek onayla WifiCorrect olur. Aşağıdakiler **uygulandığı hali**; canlı cihaz ve merkez henüz güncellenmedi (bkz. "Açık kalanlar").
- **Admin parolası merkezden:** bütün cihazlarda tek `admin` parolası vardır (2026-10-08 kararı), merkezden gelir. Cihaz bağlanınca merkez 16 karakterlik rastgele parola üretir; yönetimde müşteri sayfasında görünür (müşteri parolasıyla aynı karar, açık saklanır) ve **"Admin parolasını yenile"** düğmesi vardır. Cihaza açık parola gitmez, yalnızca özet (`admin{tuz, ozet, yineleme}`, PBKDF2-SHA256) gider; cihaz yalnızca özeti saklar. Özet değişince `hesaplar.json`'daki `admin` girdisi değişir, **açık admin oturumları düşer**, denetime `ADMIN_PAROLA_MERKEZ` yazılır. Yenileme cihaza sonraki eşitlemede ulaşır. Eski bağlı cihazlar (merkezde parolası boş) için parola migrasyonda **kendiliğinden üretilir** (yönetimde elle yenilemek gerekmez). `wificorrect ctl admin-parola` konsoldan hâlâ çalışır; merkez bir sonraki eşitlemede kendi değerini yazar.
- **NetGSM merkezden:** yönetim → **SMS ayarları** (yalnızca yönetici): deneme modu, kullanıcı kodu, şifre, mesaj başlığı, uygulama anahtarı. **Tek ayar takımı**, bütün bağlı cihazlara gider (işletme başına farklı başlık yok). Cihazda Admin ayarları'ndaki NetGSM alanları ve deneme modu **salt okunur**, "merkezden yönetiliyor" notuyla; Twilio cihazda kalır. Merkezde hiç kaydedilmemiş ya da eksik (deneme modu kapalıyken kullanıcı kodu/şifre/başlık boş) ayar cihaza **uygulanmaz**, cihaz kendi ayarını korur; admin tarafındaki bir hata SMS ayarının uygulanmasını engellemez. Değişiklik cihaza **sonraki eşitlemede** ulaşır (lisans açıkken 06:00); değiştiyse portal yeniden başlar. Denetim: `ADMIN_PAROLA_MERKEZ`, `SMS_AYARI_MERKEZ` (şifre yazılmaz), `MERKEZ_EK_HATA` (merkezden gelen ek uygulanamazsa).
- **`wificorrect ctl ag-ilk`** (postinst çağırır): `ag.toml` varsa dokunmaz. İnternet alan port = kurulumun internete çıktığı arayüz (varsayılan rota); rota yoksa ya da Ethernet değilse bu kutunun varsayılanı (iki varsayılan Ethernet de varsa); o da yoksa ilk Ethernet. Kalan Ethernet'ler misafir tarafı; en az iki Ethernet şart. `ag.toml` **en son** yazılır ("kurulum tamam" işareti; yarıda kalırsa yeniden deneme boşa gitmez). Ağı geçirmez (kurucunun içinde çalışır). Ethernet bulunamazsa (`net.ifnames=0` vb.) hata verir; postinst uyarıyla devam eder, ilk açılışta panel → Portlar.
- **.deb** (`scripts/paket.sh <ikili> <sürüm> <çıktı>` → `wificorrect_<sürüm>_amd64.deb`, yalnızca `dpkg-deb`): `/usr/local/bin/wificorrect` + `deploy/debian/` altındaki her şey; `dhcp-script` 755 (dnsmasq doğrudan çalıştırır; git'te de +x). Depends: `systemd`, `systemd-timesyncd | time-daemon`, `iptables`, dnsmasq, hostapd, iw, wireless-regdb, conntrack, nftables, wireguard-tools, rsync, curl, ca-certificates, openssl, openssh-server/client, bridge-utils, ifupdown, dhcpcd-base, iproute2, unattended-upgrades. **Pakete girmeyen** (panel/`ag-ilk` üretir): `interfaces.d/wificorrect`, `arayuzler.nft`, `issue.d/wificorrect.issue`, `yasak-siteler.conf`. **Ezilmeyen:** `ayarlar.toml` (yoksa `ayarlar.ornek.toml`'dan 600), `ag.toml`, `hesaplar.json`, `merkez.json`, anahtarlar (conffile yok). postinst: `/srv/5651`, `/srv/hotspot`, `/srv/hotspot/state` (700) ve `/var/lib/wificorrect`; boş `yasak-siteler.conf`; `ag-ilk`; `hostapd` mask, `nftables` disable; servis ve zamanlayıcılar `enable`; `update-grub`; çalışan sistemde (güncelleme) `daemon-reload` + portal/kaydedici/panel `try-restart`. **Açılış ayarları** dosyası `deploy/debian/etc/default/grub.d/wificorrect.cfg`: `GRUB_TERMINAL=console`, `GRUB_TIMEOUT=3`, `quiet systemd.show_status=1 intel_idle.max_cstate=1 console=tty0 console=ttyS0,115200n8` (monitörsüz açılış, J1900 donması; seri konsol QEMU testi için, monitörde giriş ekranı aynı).
- **ISO** (`scripts/iso.sh <deb> <sürüm> <çıktı>` → `wificorrect-kurulum-<sürüm>.iso`; Debian 13 netinst, SHA256SUMS imzası `gpgv` + debian-role-keys ile, ISO sha256 doğrulanır, `xorriso`): menüde tek giriş "WifiCorrect kur - DISKTEKI HER SEY SILINIR", **zaman aşımı yok, kendiliğinden başlamaz** (Enter; UEFI `grub.cfg`, BIOS `isolinux.cfg`; kurucuda seri konsol yok). Sorusuz preseed: tr_TR, TR klavye, İstanbul, ana bilgisayar `wificorrect`, kök parolası `*` (kilitli; `!` verilirse user-setup parola sorar). **Disk seçimi** (`iso/disk.sh`): kurulum ortamı ve yolunda `/usb` geçen (USB) diskler atlanır, kalan **ilk disk** seçilir (GRUB da oraya yazılır). Aday yoksa "Kurulacak disk bulunamadı… Hiçbir disk silinmedi." hatası **döngüde** gösterilir, kurulum ilerlemez (preseed komut hatasından sonra devam ettiği için `exit 1` yetmez); yalnız USB belleği olan makinede kurulum bu yüzden takılı kalır, istenen budur. **Bölümleme** (Debian `atomic` kökü ≥8000 MB ister, 8 GB'ta takılır → kendi tarif `wificorrect`): EFI 538 MB (yalnızca GPT/UEFI), kök ext4 ≥4 GB + kalan, takas 1 GB; ayrı `/boot` yok. **Kök kilitli, SSH yalnızca anahtarla:** `WFC_SSH_PUB` (GitHub **değişkeni**, açık anahtar; repoda yok) `/root/.ssh/authorized_keys` olur. `release.yml` değişken boşsa **sürümü derlemez** (anahtarsız cihaz çıkmasın). Kurulum bitince "Kurulum bitti. Cihaz birazdan kapanacak; kapandıktan sonra USB belleği çıkarın." 10 sn görünür, cihaz kapanır. **Teknisyen notu:** kurulumda **Ethernet 1 modeme takılı** (paketler internetten iner; internetsiz kurulum olmaz), monitör + klavye takılı, Enter ile başlat, ~15 dk, kapanınca USB'yi çıkar; ilk açılış monitörsüz olabilir, konsolda `https://<ip>:8443` yazar.
- **CI:** `test.yml` → `paket` işi (.deb temiz Debian 13 kapsayıcısına kurulur: bağımlılıklar, dosyalar, +x, panelin ürettiği dosyaların pakette olmaması, ikinci kurulumda ayarların ezilmemesi; `PAKET TESTI GECTI`). `iso.yml` (yol süzgeçli push + `workflow_dispatch`; sürüm `0.0.0-<sha>`, ISO + günlükler 7 gün ek, release açmaz) → QEMU/KVM duman testi (`scripts/iso-test.sh`; KVM yoksa `ATLANDI` yazıp geçer): makinede hep bir USB bellek; **yalnız USB varken** kurucunun durması ve USB sha256'sının değişmemesi; menünün 20 sn kendiliğinden başlamaması; normal kurulumda `KURULUM DISKI: /dev/vda`, "bitti" ekranı, USB **dokunulmamış**; diskten açılışta `wificorrect-panel.service` başladı + IP'li yönetim adresi; deneme SSH anahtarıyla root girişi (`authorized_keys` dolu). Seri konsol yerine ekran görüntüsü ve kurucu syslog'unun seri aynası kullanılır. `release.yml` yalnızca **`v*` etiketinde**: test + ikili + .deb + paket testi + ISO + duman testi + sunucu tar → `gh release create` (ikili, `.deb`, `.iso`, sunucu `.tar.gz`, `SHA256SUMS`).
- **Açık kalanlar:** BIOS/isolinux yolu CI'da hiç açılmadı (yalnız UEFI); `release.yml` **henüz hiç çalışmadı** (etiket yok; ilk `v*` etiketinde `gh release create` ilk kez çalışacak); `WFC_SSH_PUB` repo değişkeni tanımlanmalı; **gerçek donanım kabulü bekliyor** (boş yedek makinede ISO, monitörsüz ikinci açılış, numarayla bağlanma, yönetimdeki admin parolasıyla giriş, deneme modu kapalıyken kullanıcı onayıyla tek gerçek SMS); `try-restart` dalı ve gerçek `ag-ilk` başarısı yalnızca gerçek kurulumda sınanır; **canlı cihaz ve merkez henüz güncellenmedi** (cihaz: yeni ikili + servis yeniden başlatma; merkez: `scripts/sunucu/dagit.sh`, sonra SMS ayarlarının yönetimde girilmesi; eşitlemeden sonra `ADMIN_PAROLA_MERKEZ` / `SMS_AYARI_MERKEZ` denetimde görülmeli).

### Canlı cihaz karşılaştırması (2026-10-08, yalnızca okuma)
Spec §7 gereği canlı cihaz (1537344) ile `deploy/debian/` karşılaştırıldı (hiçbir şey değiştirilmedi). Bakılanlar: `/etc/{sysctl.d,modprobe.d,modules-load.d,default/grub,default/grub.d,ssh/sshd_config.d,dnsmasq.d,network,apt/apt.conf.d,hosts,hostname,crontab,cron.*}`, `systemd/{system,journald.conf.d,timesyncd.conf.d}` ve etkin/maskeli birimler, `/etc/wificorrect/*`, `dpkg --verify`, `apt-mark showmanual` (paket `Depends`'iyle), kodun çağırdığı dış komutlar.
- **Birebir aynı (sha256):** repodaki 27 dosyanın 25'i (apt, dnsmasq, issue, network, journald, timesyncd, bütün `wificorrect-*` birimleri ve `dnsmasq.service.d`, `*.nft`, `conntrack.sysctl`, `yasak-siteler.conf`, `dhcp-script`). `dpkg --verify` temiz. Etkin birimler (`dnsmasq`, `networking`, `wg-quick@wfc`, `wificorrect-*`, zamanlayıcılar), `hostapd` maskeli, `nftables` devre dışı: postinst ile aynı.
- **Fark, ama zaten repoda:** (1) `/etc/default/grub` elle düzenlenmiş (`quiet intel_idle.max_cstate=1`, `GRUB_TERMINAL=console`, `TIMEOUT=5`) → repoda aynı ayar `default/grub.d/wificorrect.cfg` içinde (üstüne seri konsol, `TIMEOUT=3`); (2) cihazdaki `ayarlar.ornek.toml` eski, repodaki yeni (`retention_days`, `[sms]`, `[twilio]`, `[backup]`).
- **Boş çıkanlar (cihazda özel ayar yok):** `sysctl.d`, `modprobe.d` (yalnızca Debian'ın microcode dosyası), `modules-load.d`, `sshd_config.d`, root crontab, `cron.d` (yalnızca `e2scrub_all`). Hiçbiri alınacak ayar içermiyor.
- **Alınmadı (neden):** `cargo`, `rustc`, `gcc`, `libc6-dev`, `pkg-config` (geliştirme: cihazda derleme); `wificorrect` kullanıcısı (uid 1000, geliştirme; ISO'da kullanıcı yok, kök kilitli); `/etc/default/grub.yedek` (elle yedek); `firmware-realtek`, `firmware-intel-graphics`, `intel-microcode` (bu cihazın donanımı; kurucu `non-free-firmware` deposunu açar ve donanıma göre kendisi seçer); `sudo`, `cron`, `task-ssh-server`, `task-turkish`, `bash-completion`, `vim-tiny`, `nano`, `lsof`, `traceroute`, `netcat-traditional`, `wget`, `dmidecode`, `usbutils`, `pciutils`, `bind9-dnsutils`, `reportbug` vb. (kurucunun standart/tanı paketleri; kod bunları çağırmaz); `wg.key*`, `yedek_anahtar*`, `panel.crt/key`, `hesaplar.json`, `merkez.json`, `ag.toml`, `ayarlar.toml`, `arayuzler.nft`, `/etc/wireguard/wfc.conf`, `resolv.conf`, `hosts` (sır ya da cihaza/işletmeye özel, panel ya da kurucu üretir); `sshd` varsayılanları (`PasswordAuthentication yes`, `X11Forwarding yes`: ISO'da kök kilitli ve başka kullanıcı yok, parola girişi pratikte kapalı; varsayılan bırakıldı).
- **`apt-mark showmanual` ↔ `Depends`:** kodun çağırdığı her komut (`systemctl`, `journalctl`, `ip`, `nft`, `conntrack`, `iptables`, `dnsmasq`, `rsync`, `curl`, `wg`, `ssh`, `ssh-keygen`, `openssl`, `tar`, `ifup`/`ifdown`, `iw`, hostapd) `Depends` ile gelen bir pakette; eksik yok, eklenecek bağımlılık çıkmadı.
- **Sonuç:** `deploy/debian/`'e eklenecek yeni dosya yok; canlı cihazın elle yapılmış ürün ayarlarının hepsi zaten repoda.
- **İnceleme düzeltmeleri (2026-10-08):** kurucu, paket `ag.toml` yazmadıysa (tek Ethernet vb.) "Kurulum bitti" demez, hata ekranında kalıcı durur (`iso/son.sh`); postinst `update-grub` hatasını gösterir ve çalışan sistemde `conntrack.sysctl`'i hemen uygular; `release.yml` `WFC_SSH_PUB`'ın geçerli bir SSH anahtarı olduğunu `ssh-keygen -l` ile doğrular.

## Uzak panel — yalnızca panel.wificorrect.com'dan giriş (2026-10-09)
- **Akış:** müşteri panel.wificorrect.com'da numara+parola ile girer → "Cihaz paneli" bağlantısı kısa ömürlü (60 sn, tek kullanımlık) belirteç üretir → `cihaz.wificorrect.com/_giris?t=…` belirteci `wcc` çerezine çevirir → Caddy her istekte `forward_auth` ile merkeze (`/cihaz-yetki`) sorar; merkez müşterinin cihazının tünel adresini (`X-WFC-Cihaz`) ve numarayı (`X-WFC-Kullanici`) döner → istek `https://<tünel adresi>:8443`'e yönlenir. Çıkış `/_cikis`. Parola değişirse / lisans ya da üyelik kapanırsa / cihaz bağlı değilse çerez geçersiz olur.
- **Güvenlik kuralı:** cihaz `X-WFC-Kullanici` başlığına yalnızca tünelden (`wfc`) ve merkez adresinden geleni güvenir; Caddy istemciden gelen `X-WFC-Kullanici` ve `X-WFC-Cihaz` başlıklarını siler, cihazda nft kuralı wfc dışından gelen 10.99.0.0/24 kaynağını düşürür (sahte kaynak engeli). Merkezin `wcc` çerezi cihaza iletilmez.
- **Admin kilidi:** başlıkla gelen oturum "müşteri"dir; Admin ayarları ve fabrika ayarları için merkezdeki hizmet sağlayıcı (admin) parolası girilir, kilit 30 dk açık kalır (denetim: `PANEL_ADMIN_ACILDI`). Bağlı cihazda `uzak.*` alanları salt okunurdur (merkez bağlanmada yazar).
- **Yerel erişim:** cihaz merkeze bağlıyken modem ağından `:8443` yalnızca "panel.wificorrect.com'dan girin" notu döner; bağlı değilken yalnızca eşleştirme (bağlama) sayfası açılır. Fabrika ayarına dönüş merkezden bırakma (release) ile yapılır.
- **Caddy bloğu** (`scripts/sunucu/merkez/Caddyfile`, `kur.sh` kopyalar): `cihaz.wificorrect.com` — `request_header -X-WFC-Kullanici` / `-X-WFC-Cihaz`; `/_giris /_cikis` merkeze (forward_auth dışı); diğer her şey `forward_auth` + `reverse_proxy {http.request.header.X-WFC-Cihaz}:8443` (`transport http { tls; tls_insecure_skip_verify }` — cihaz sertifikası kendinden imzalı, tünel zaten şifreli).
- **Caddy 2.6.2 uyumu** (Ubuntu paketi; kaynak `v2.6.2` incelendi): üst akış adresinde yer tutucu varsa `https://` şeması yazılamaz ("placeholders are not allowed when an upstream address contains a scheme") → plana yazılan `https://{…}:8443` yerine şemasız adres + `tls` taşıma ayarı kullanıldı. `forward_auth` + `copy_headers` 2.6.2'de var. Yer tutuculu `dial` her istekte çözülür. **DNS:** `cihaz.wificorrect.com` kaydı sunucuya yönlenmeli (sertifika için).

## AP arayüzü — merkez (2026-10-16)
Dal `ap-arayuz-merkez`; tasarım `docs/superpowers/specs/2026-10-16-ap-arayuz-design.md` ("Mimari (sözleşme)" 2–3); cihaz tarafı
ayrı (0.4.15). Kod `scripts/sunucu/merkez/ap.py` (ap.wificorrect.com), `cihaz.py` (`/_ap-gecis`); testler `tests/test_ap.py`.
- **Akış:** cihaz paneli → Ayarlar → AP kurulumu → "AP arayüzüne git" → cihaz hedef durumunu yazar, tarayıcıyı
  `https://cihaz.wificorrect.com/_ap-gecis`'e yönlendirir (Caddy bu yolu forward_auth'suz merkeze verir) → merkez `wcc`
  çerezinden (müşteri ya da yönetici geçişi oturumu) müşteriyi, cihaz kaydını ve tünel adresini bulur; bunlara ve bu `wcc`
  oturumuna ve isteyen adrese bağlı **60 sn'lik tek kullanımlık** belirteç → `303 https://ap.wificorrect.com/_ap-giris?t=…` →
  `wca` çerezi → `303 /` → Caddy her istekte `forward_auth` ile `/ap-yetki`'ye sorar → `200` + `X-WFC-Cihaz: <tünel IP>`,
  `X-WFC-AP-Kullanici: <müşteri numarası>`, `X-WFC-AP: 1` → `reverse_proxy {X-WFC-Cihaz}:8443`. `wcc` geçersizse `/_ap-gecis`
  panel girişine (`https://panel.wificorrect.com/giris`) döner.
- **`wca`:** `HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=1800`, Domain yok (yalnızca ap.wificorrect.com); sunucuda da en
  çok **30 dk** (mutlak, istekle uzamaz). Her istekte altındaki `wcc` cihaz-yetki kuralıyla yeniden doğrulanır (parola özeti,
  üyelik, lisans, bağlı cihaz + tünel adresi) ve **açıldığı cihaz kaydı ve tünel adresi** aynı olmalı; değilse `wca` düşer.
  Adres her istekte `wcc`'nin müşterisinden bulunur: bir müşterinin `wca`'sı başka müşterinin cihazına gidemez; cihaz değişirse
  (aynı tünel adresi verilse bile) eski `wca` geçmez. AP kullanımı `wcc`'yi de canlı tutar (boşta 30 dk). Cihaz panelinden
  çıkış, parola değişimi, lisans/üyelik kapanması, bağın kalkması `wca`'yı da geçersiz kılar.
- **`X-WFC-AP-Kullanici`** yönetici geçişinde de müşteri numarasıdır (cihaz-yetki gibi; cihaz kendi bağındaki numarayla
  karşılaştırır). Açanın kim olduğu merkezde hareket satırında, cihazda kendi `AP_ARAYUZ` denetiminde. AP kipinde
  `X-WFC-Kullanici` hiç gönderilmez: 0.4.15 öncesi cihaz `X-WFC-AP`'yi tanımaz ve `X-WFC-Kullanici`'li isteği oturumlu panel
  isteği sayıp paneli ap.wificorrect.com kökünde verirdi (final review M2); yeni başlığı tanımadığı için isteği oturumsuz
  sayar (giriş yönlendirmesi, sonra 404).
- **Geçersiz durumlar:** belirteç yok/kullanılmış/süresi dolmuş ya da altındaki `wcc` düşmüş → `/_ap-giris` 403 sade Türkçe
  sayfa ("Bağlantı geçersiz … Ayarlar → AP kurulumu → AP arayüzüne git ile yeniden açın", cihaz paneline bağlantı).
  `/ap-yetki` geçersiz → 403 "AP erişimi kapalı" (forward_auth bu cevabı tarayıcıya aynen verir; bozuk/eski `wca` silinir;
  sayfada temizleyerek çıkış bağlantısı `/_ap-cikis`). Bu sade sayfalar çerçeveli AP arayüzünün (TP-Link userRpm frameset)
  çerçevesinde de görünür: `X-Frame-Options: SAMEORIGIN` + CSP `frame-ancestors 'self'` (merkezin diğer cevaplarında
  DENY; cevabın verdiği güvenlik başlığı varsayılanın yerine geçer), bağlantılar `target="_top"` (çerçevedeki gezinmeyi
  `/_ap-cikis` reddeder, cihaz paneli çerçevede açılmaz; re-review m1). `/_ap-cikis[?donus=/yol]` `wca`'yı siler,
  `Clear-Site-Data: "cache", "cookies", "storage"` gönderir (AP'lerin çerezleri, depolama, Service Worker'lar; tarayıcının
  sakladığı HTTP kimlik bilgilerini silip silmemesi tarayıcıya bağlı; "cookies" kayıtlı alan adının — wificorrect.com —
  bütün çerezlerini sildiği için panel, yönetim ve cihaz paneli oturumları da kapanır; sunucudaki `wcc` oturumu düşmez) ve
  `https://cihaz.wificorrect.com/yol`'a döner (yalnızca bu sitede bir yol; varsayılan `/`). Çıkış da yalnızca üst düzey
  gezinmeyle (`same-origin|same-site|none`): başka siteden tetiklenen "her şeyi sil" oturum kapatma CSRF'i olmasın.
- **Yalnızca cihaz panelinden (Fetch Metadata):** AP arayüzü panel/cihaz/yönetimle aynı "site"da çalışır ve `wcc`
  (SameSite=Lax) aynı-site alt isteklerde de gönderilir: kötü niyetli bir AP'nin betiği `<iframe src=…/_ap-gecis>` ile
  tarayıcıdaki BAŞKA müşterinin `wcc`'sinden `wca` aldırıp o AP'yi kendi kökeninde okuyabilirdi. Bu yüzden tarayıcı
  `Sec-Fetch-*` gönderdiyse `/_ap-gecis` yalnızca `Sec-Fetch-Site: same-origin|none` (cihaz.wificorrect.com sayfası ya da elle
  yazılan adres), `/_ap-giris` **yalnızca `same-site`** (cihaz alanından yönlenen gezinme; `same-origin` = AP sayfasının
  kendisi gezindi — bir müşterinin AP'si kendi belirtecini tarayıcıya ekip başka müşterinin açık AP sekmesini kendine
  çevirirdi; `none` = elle yazılan adres); ikisi de `Sec-Fetch-Mode: navigate` + `Sec-Fetch-Dest: document` (çerçeve/fetch/
  resim değil). Değilse 403 "AP arayüzü açılamadı" (belirteç harcanmaz; oturumlu/belirteçli istekte günlüğe `AP_GECIS_RED`
  / `AP_GIRIS_RED` + değerler). **Başlık göndermeyen eski tarayıcı geçer** (Safari < 16.4 vb.; onlarda koruma `wca`
  SameSite=Strict + belirtecin ağ bağı). Sonuç: `/_ap-gecis`'e bağlantı yalnızca cihaz.wificorrect.com sayfasından verilir
  (AP kipi sayfalarından, yani ap.wificorrect.com'dan değil).
- **Final review düzeltmeleri (2026-10-16; tek kök, bütün müşterilerin AP'leri):**
  - **Service Worker yok (C1):** Caddy ap bloğunda `@sw header Service-Worker script` → `handle @sw { respond 403 }`
    (adlı eşleyicili handle'lar yazıldığı sırayla; ilk o), cihaz da reddeder. Bir AP'nin betiği kök kapsamlı SW kaydetseydi
    sonraki müşterilerin AP oturumlarını ve `/_ap-giris` belirtecini dinlerdi.
  - **Yeni AP oturumunda temizlik:** `/_ap-giris` `Clear-Site-Data: "storage"` (kökün localStorage, IndexedDB, Cache API,
    SW kayıtları; "cookies" yok — kayıtlı alan adının bütün çerezlerini, az önce yazılan `wca`'yı da silerdi; "cache" yok —
    re-review m4: bu kökteki her cevap merkezde ve cihazda `no-store`, HTTP önbelleğinde bir şey yok, Chromium'da önbellek
    silme giriş yönlendirmesini gözle görülür geciktirir) ve tarayıcının bu kökte gönderdiği eski (Path=/) çerezleri
    `Max-Age=0` ile siler (`wca` hariç, yalnızca geçerli çerez adları, en çok 30; ör. TP-Link'in betikle yazdığı
    `Authorization` = Basic kullanıcı:parola). Merkez artık bir başlığa liste verebilir (birden çok `Set-Cookie`,
    `merkez.baslik_satirlari`).
  - **Belirteç tarayıcının ağına bağlı:** `/_ap-gecis` belirteci isteyen istemci adresine (Caddy'nin `X-Forwarded-For` son
    öğesi) bağlar; `/_ap-giris` aynı ağdan gelmeli: **IPv4'te aynı /24, IPv6'da aynı /64** (`ap.ayni_ag`; IPv4'e eşlenmiş
    IPv6 yazımı IPv4 sayılır; re-review I-1). cihaz.* ve ap.* ayrı sertifikalı ayrı bağlantılardır: tam adres eşitliği,
    bağlantı başına farklı çıkış adresi kullanan istemcileri (taşıyıcı NAT havuzu, çift hatlı modem) her denemede
    kilitliyordu; /24 sızdırılan belirtecin başka ağdan kullanılmasını ve eski tarayıcıda dışarıda üretilmiş belirteçle
    oturum açtırmayı yine önler. Ağ farklıysa belirteç yine yanar ama genel "geçersiz" değil **ayrı sayfa** (403 "Farklı
    internet adresi": "Bağlantınız farklı bir internet adresinden geldi (ör. iCloud Özel Geçiş, VPN). Özel Geçiş/VPN'i
    kapatıp cihaz panelinden yeniden deneyin. iPhone'da: Ayarlar → adınız → iCloud → Özel Geçiş."); günlükte
    `AP_GIRIS_RED adres_farkli ip=<gelen> gecis_ip=<belirteci isteyen>`. Üst alana (`Domain=wificorrect.com`) çerez
    bilerek kullanılmadı: AP'lerin betiği ap.wificorrect.com'dan okuyup yazabilirdi; ap kökündeki çerez de o kökü ele
    geçirmiş betiğe karşı işe yaramaz. Not: alan adlarına AAAA eklenirse ikisine birden eklenmeli (istemci cihaz ve ap alan
    adlarına farklı IP sürümüyle giderse belirteç tutmaz).
  - **AP yönetim parolası (I1):** müşteri başına 12 karakterlik rastgele parola (`musteri.ap_parola`, açık saklanır; eski
    müşterilere göçte üretilir), yönetimde müşteri sayfasında "AP yönetim parolası" + **Yenile** (`AP_PAROLASI_YENILENDI`,
    parola yazılmaz); `/api/giris`, `/api/bagla` ve her `/api/eslesme` cevabında `ap_parola` (müşteriye özel, ortak sır değil:
    kolay bağlamayla bağlanan cihaza da gider; yanlış bağ "zorla ayır"la kalkınca AP parolasını da yenileyin). AP'ye panel
    parolası yazılmaz (docs/AP_KURULUM.md 3.6).
- **Hız sınırı, hareket:** `/_ap-gecis` müşteri başına dakikada 20 (aşılırsa 429 sayfası); her geçiş hareket `AP_GECIS`
  (kim = müşteri numarası ya da `admin`, IP, müşteri, ayrıntı yalnızca `rol=musteri|yonetici`; hedef adres yok — merkez
  bilmez). `/ap-yetki` ve `/_ap-giris` sınırsız (cihaz-yetki ve `/_giris` gibi; AP sayfası istek başına onlarca kaynak yükler;
  belirteçler 256 bit). fail2ban değişmedi (bu uçlarda parola yok).
- **Caddy `ap.wificorrect.com` bloğu** (cihaz bloğunun kalıbı): `request_header -X-WFC-Kullanici` / `-X-WFC-AP-Kullanici` /
  `-X-WFC-Cihaz` / `-X-WFC-AP` (cihaz bloğunda da); önce `@sw` (Service Worker betiği → 403); `@merkez path /_ap-giris
  /_ap-cikis` → merkez; diğer her şey `forward_auth` (`uri /ap-yetki`,
  `copy_headers X-WFC-AP-Kullanici X-WFC-Cihaz X-WFC-AP`) + `reverse_proxy {http.request.header.X-WFC-Cihaz}:8443` (`tls` +
  `tls_insecure_skip_verify`), `header_up -X-WFC-Cihaz`, `header_up Cookie "\b(?:wca|wcc)=[^;]*" "_"` (merkez çerezleri
  cihaza/AP'ye gitmez, AP'nin kendi çerezleri geçer), Host `ap.wificorrect.com` kalır; 502/503/504 → panel `/cihaz?yok=1`.
- **Güvenlik başlıkları:** HSTS ve `-Server` diğer bloklar gibi; **CSP, X-Frame-Options, X-Content-Type-Options,
  Referrer-Policy bilerek yok**: içerik AP'nin kendi arayüzüdür (satır içi betik/eval, frameset, betiği yanlış MIME türüyle
  sunan gömülü sunucular, Referer denetleyen TP-Link) — herhangi biri arayüzü bozar. Merkezin kendi cevapları (yukarıdaki
  sayfalar) her zamanki başlıklarla çıkar; yalnızca sade sayfalar aynı kökenden çerçevelenebilir (SAMEORIGIN, yukarıda).
- **Cihaz bloğu:** `/_ap-gecis` merkeze (forward_auth dışı); `request_header -X-WFC-AP` (AP kipi başlığı cihaz alanından
  cihaza asla ulaşmaz; cihaz merkezden gelen `X-WFC-AP: 1`'i AP kipi sayar); CSP `form-action`'a `https://ap.wificorrect.com`
  eklendi (form-action yönlendirmeye de uygulanır: "AP arayüzüne git" formu → `/_ap-gecis` → ap.wificorrect.com; eklenmezse
  tarayıcı gezinmeyi sessizce iptal eder).
- **AP arayüzünü her müşteri için yeni bir gizli (incognito) pencerede açın; işiniz bitince bütün gizli pencereleri
  kapatın** (yönetici için önemli): bütün müşterilerin AP'leri tek kökende (ap.wificorrect.com) açılır, tarayıcıda tek
  `wca` çerezi vardır: B'nin AP'si açılınca A'nın AP'si için açık kalmış sekmedeki sonraki istekler (ör. "Kaydet") B'nin
  AP'sine gider. Tarayıcının sakladığı HTTP Basic kimliği bütün müşteriler için ortaktır (aynı model/realm'de A'nın AP
  parolası B'nin AP'sine gönderilebilir); onu sunucu silemez (yalnızca `/_ap-cikis`'in Clear-Site-Data'sı — tarayıcıya
  bağlı — ya da pencereyi kapatmak). Bu yüzden AP'ye panel parolası değil müşteriye özel AP yönetim parolası yazılır;
  yeni AP oturumu eski AP çerezlerini ve depolamasını siler. Gizli pencere ek savunmadır.
- **Bilinen risk:** AP arayüzü wificorrect.com'un alt alanında, panel ve yönetimle aynı "site"da çalışır: kötü niyetli ya da
  açıklı bir AP'nin betiği üst alana çerez atabilir (`Domain=wificorrect.com`; oturum bozma) ya da aynı-site istek yapabilir.
  Merkez çerezleri host-only + HttpOnly (okunamaz), POST'lar CSRF belirteçli, `/_ap-gecis` yukarıdaki gibi korunur. Kalıcı
  çözüm: ayrı kayıtlı alan adı ve müşteri başına alt alan (ör. `<numara>.ap.<ayrı-alan>`, Caddy on-demand TLS + joker DNS);
  cihaz bunun için içerik/Location yeniden yazımında sabit ad yerine gelen `Host`'u kullanmalı.
- **Kurulum sırası:** (1) **DNS: `ap.wificorrect.com` A → merkezin dış IP'si** (Güzel Hosting → DNS Yönetimi; `cihaz` ile
  aynı IP); sertifikayı Caddy alır; DNS gelmeden yalnızca ap.wificorrect.com açılmaz. (2) `merkez-guncelle.sh` (kullanıcı,
  sudo): `kur.sh` `ap.py`'yi kurar, Caddyfile'ı `caddy validate`'ten geçmeden kurmaz. (3) Cihazlar 0.4.15. Merkez önce gelmezse
  eski Caddyfile `/_ap-gecis`'i forward_auth ile cihaza iletir: cihaz bunu "AP erişimi şu an kullanılamıyor" diye göstermeli.
- **Canlıda doğrulanmadı:** yerelde `caddy` yok; blok, canlıda çalışan cihaz bloğunun aynısı, yalnızca çerez ifadesi farklı
  (RE2 `\b(?:wca|wcc)=[^;]*`) ve `@sw header Service-Worker script` (belgelenmiş `header <alan> <değer>` biçimi). Kurulumdan
  sonra: `curl -s -o /dev/null -w '%{http_code}\n' https://ap.wificorrect.com/` → 403; `curl … -H 'Service-Worker: script'
  https://ap.wificorrect.com/sw.js` → 403 (gövdesiz); sonra panelden "AP arayüzüne git" uçtan uca, DevTools → Application →
  Service Workers boş, `/_ap-giris` cevabında `Clear-Site-Data`.

## Uzaktan güncelleme (2026-10-12)
Dal `uzaktan-guncelleme`; tasarım `docs/superpowers/specs/2026-10-12-uzaktan-guncelleme-design.md`. Sürüm **0.4.0**. Cihaz paketini (.deb) merkezden alıp kendi kurar; ISO'dan kurulmuş (paketli) cihazlar için.
- **Akış:** yönetici → Sürümler'de GitHub'dan paketi alır (ya da elle ekler) ve kanal hedefini seçer → cihaz 5 dk'lık `/api/saglik` raporunda hedefi öğrenir (`guncelleme`: sürüm, sha256, boyut, `hemen`) → `ctl guncelle` paketi `/api/paket`ten indirir, sha256 doğrular → `apt-get install` → kurulumdan 10 dk sonra denetim betiği servisleri ve sağlık raporunu kontrol eder; bozuksa önceki paketi geri kurar. Rapor `paket` (kurulu sürüm) ve son sonucu da taşır.
- **Karar (`guncelleme::karar`):** paketsiz cihaz (dpkg'de `wificorrect` yok) güncellenmez; kurulu = hedef ise bir şey yapılmaz; denetim bekleyen varsa beklenir. "Hemen" yoksa: başarısız listesindeki sürüm denenmez, son deneme `hata` ise 12 saat beklenir, yalnızca TR saati 03:00–06:00 penceresinde kurulur. "Hemen" bu üç kuralı atlar (bekleyen denetim hariç).
- **Cihaz dosyaları** (`/var/lib/wificorrect/guncelleme/`): `durum.json` (başarısız sürümler, son sonuç), `hedef.json`, `paketler/` (indirilenler; eskileri uzlaşmada silinir), `onceki.deb` (geri dönüş paketi), `bekleyen` + `sonuc` (denetim durumu: `tamam` | `geri`), `denetle.sh` (kurulumdan ÖNCE eski ikiliyle yazılır, yeni ikiliye güvenmez), `saglik_basari` (son başarılı rapor), `kilit` (flock; uzlaşma ve hedef yazma, güncelleme sürerken dokunmaz). Birimler: `wificorrect-guncelleme-denetle.timer` (açılıştan 5 dk sonra, 5 dk'da bir) → `.service` (`denetle.sh` varsa çalıştırır). Betik geri kurulumu `--allow-downgrades` ve dpkg kilidini 600 sn bekleyerek yapar; başarısızsa sonuç yazmaz, sonraki turda yeniden dener. Yeni sürüm hiç kurulamadıysa (`kurulmadi`) başarısız listesine girilmez, 12 saat sonra yeniden denenir.
- **Merkez:** `surumler.py` (depo `/var/lib/wificorrect/merkez/surumler/<sürüm>.deb`, kayıt veritabanında). GitHub'dan alma: son 10 sürümden depoda olmayanların `wificorrect_<s>_amd64.deb` ve `SHA256SUMS` dosyası indirilir, sha256 doğrulanmadan eklenmez (en çok 50 MB; tek sürümün hatası diğerlerini durdurmaz). Elle: `sudo -u wcpanel python3 /usr/local/lib/wificorrect/surumler.py ekle DOSYA SURUM` (sha256 dosyadan hesaplanır, GitHub doğrulaması yok). Yönetim → **Sürümler** (liste, kurulu cihaz sayısı, "GitHub'dan yeni sürümleri al", Kararlı/Test kanal hedefi; hedef yoksa o kanal güncellenmez); müşteri sayfasında **Güncelleme** kartı (kanal seçimi, "Hemen güncelle", son denemeler). `/api/paket` yalnızca bağlı, üyeliği ve lisansı açık cihaza ve yalnızca hedef sürümü ya da cihazın kurulu (geri dönüş) sürümünü verir. Sağlık izleme: başarısız güncelleme `guncelleme_basarisiz` uyarısı açar.
- **Denetim olayları:** cihaz `GUNCELLEME_BASLADI`, `GUNCELLEME_TAMAM`, `GUNCELLEME_HATA` (neden: indirme, sha, disk, kurulmadi, geri_kurulamadi …), `GUNCELLEME_GERI_ALINDI` (ek: `surum=… onceki=… neden=…`); merkez hareketleri `SURUM_ALINDI`, `SURUM_AL_HATA`, `GUNCELLEME_SONUC`.
- **Paketsiz cihazı bir kez elle pakete geçirme** (canlı cihaz 1537344 elle derlenmiş ikiliyle çalışıyor, dpkg'de kayıtlı değil): önce 0.4.0 .deb'i merkeze ekle (yukarıdaki CLI ya da GitHub); ağ/güvenlik duvarı dosyalarına dokunabileceği için **ölü adam anahtarıyla** yap: yedek + `systemd-run --on-active=300` geri alma → `.deb`'i cihaza kopyala, `apt-get install ./wificorrect_0.4.0_amd64.deb` → YENİ SSH bağlantısıyla doğrula (servisler, panel, portal, tünel) → geri alma zamanlayıcısını iptal et. Bundan sonra cihaz kendi kendine güncellenir.
- **Bekleyen:** uçtan uca deneme (merkezde hedef seçilip bir cihazın pencere dışında "Hemen güncelle" ile kurulması, bilerek bozuk paketle geri alma) **henüz yapılmadı**; merkez güncellemesi (`merkez-guncelle.sh`) kullanıcı tarafından çalıştırılacak.

## SMS merkezden (2026-10-13)
Dal `sms-merkez`; sürüm **0.4.8**. Doğrulama SMS'i artık cihazdan değil yönetim merkezinden gider.
- **Neden:** NetGSM yalnızca izinli (whitelist) IP'lerden istek kabul eder; müşteri cihazlarının dış IP'si dinamik (2026-10-13'te mobil hat olayında IP değişince SMS kesildi). Merkezin IP'si sabit; tek yerde whitelist yeter. Sağlayıcı sırları cihazlardan çıktı.
- **Akış:** portal → `sms::send` → merkez `POST /api/sms` (`cihaz_anahtari`, `telefon`, `kod`; yalnızca bağlı + lisansı açık cihaz) → merkez kendi ayarıyla NetGSM OTP ya da Twilio Verify'a iletir. Twilio'da kodu Twilio üretir; doğrulama `POST /api/sms-dogrula` (merkez üzerinden). Cevap `{sonuc, kod[, is, saglayici]}`; deneme modunda (`mock`) merkez sağlayıcıya gitmez, cihaz kodu yerelde doğrular.
- **Şablon merkezde sabit:** `WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir.` Cihaz serbest metin gönderemez (açık röle olmasın); yalnızca 6 haneli kod kabul edilir.
- **Sınırlar:** TR numara = `905…` (NetGSM'e `tel[2:]` gider, `tr_saglayici` netgsm|twilio); yabancı numara yalnızca Twilio açıksa Twilio'dan, değilse `YABANCI`; cihaz (müşteri) başına günde **300** SMS (`SINIR`, TR günü, yarışa karşı artır-sonra-denetle); ayrıca yabancı numaralara müşteri başına günde **20** (yabancılar 300'e de sayılır); **yeniden deneme yok** (çift SMS = çift ücret). Hareket satırı yazılmaz; müşteri başına son sonuç `sms_son` tablosunda (zaman, sağlayıcı, sonuç, kod; numara ve kod yok), müşteri sayfasında "Son doğrulama SMS'i".
- **Süreler:** merkez sağlayıcı çağrısı bitmeden cevap dönmez (SMS gitmişse kod atılmasın); urllib zaman aşımı işlem başına **5 sn** (`ISLEM_SURE`; bağlan + oku, TLS dahil en kötü ~10-11 sn); cihaz merkezi **15 sn** bekler (`sms::MERKEZ_SURE`): cevap cihaz vazgeçmeden gelir.
- **Deneme modu cihaza:** eşleşme ekinde `sms.etkin_mock` (deneme modu açık ya da seçili Türk sağlayıcısının ayarı eksik) ve her sağlık raporu cevabında `sms_etkin_mock` (5 dk); eski cihazlar için `mock` + `netgsm` aynen. Merkezde SMS ayarı yoksa yalnızca `{"etkin_mock": true}`.
- **Sağlık:** son 60 dk içindeki son `sms_son` misafirden kaynaklanmayan bir hatayla bittiyse (SINIR dahil; `YABANCI`, 50, 80, 60200, 21211, 60205, 60203 hariç) "SMS çalışmıyor"; sonraki başarılı gönderimde kapanır.
- **Eski sırlar:** postinst (`ctl ayar-temizle`) ve açılışta `ctl yukle`, `ayarlar.toml`'da `[netgsm]`, `[twilio]`, `sms.provider`, `sms.merkez` kalmışsa dosyayı bir kez yeniden yazar (600, diğer ayarlar aynen).
- **Misafir metinleri:** merkeze ulaşılamaz/bozuk cevap/sınır (`AG`, `SINIR`, `PARSE`) → "Şu an kod gönderilemiyor, lütfen biraz sonra tekrar deneyin."; `YABANCI` → "Yabancı numaralara şu an SMS gönderilemiyor, lütfen personele başvurun."; ağ hatasında Twilio doğrulamasında misafirin deneme hakkı düşmez. Portaldaki sınırlar (60 sn bekleme, numara/MAC başına, günlük tavan) cihazda aynen kalır.
- **Ayarlar:** Yönetim → **SMS ayarları**: deneme modu, NetGSM, Twilio ve TR sağlayıcı (netgsm|twilio) tek takım, bütün cihazlar için. Cihazda NetGSM/Twilio sırları ve `ureq` bağımlılığı kalktı (HTTP için `merkez.rs` curl).
- **Geçiş sırası:** (1) merkez güncellenir (`scripts/sunucu/merkez-guncelle.sh`) ve SMS ayarları Yönetim'de girilir; NetGSM whitelist'e merkezin IP'si eklenir; (2) cihazlar 0.4.8'e uzaktan güncellemeyle geçer (önce test kanalı/Göztepe). Merkez önce gelmezse yeni cihaz `/api/sms` bulamaz ve misafir kod alamaz.
- **Eski cihazlar:** 0.4.8'den önceki cihazlar kendi SMS'ini göndermeye devam eder; merkez bunlara NetGSM bilgisini eşitlemede göndermeyi sürdürür (hepsi yükselene kadar). Hepsi geçince bu gönderim kaldırılır.

## Hazır engel listeleri (2026-10-14)
Dal `hazir-listeler`; tasarım `docs/superpowers/specs/2026-10-14-hazir-engel-listeleri-design.md`. Sürüm **0.4.9**.
- **Kategoriler:** `kumar` "Kumar ve bahis", `yetiskin` "Yetişkin içerik", `zararli` "Zararlı ve dolandırıcı siteler"; üçü
  varsayılan **açık** (`[filtre.hazir]` anahtarı yoksa açık). Kategori başına en çok 300.000 alan adı (fazlası kesilir,
  meta'da `kesildi`). **VPN/proxy kategorisi bilerek yok** (iş VPN'leri çalışmalı); VPN/proxy/DoH sağlayıcıları ve ilk etiketi
  `vpn|doh|dot|dns|proxy` olan adlar hiçbir listeden gelmez.
- **Kaynaklar ve lisanslar:** HaGeZi DNS Blocklists (GPL-3.0; `gambling.medium`, `nsfw`, `tif.mini`, `wildcard/*-onlydomains`
  biçimi), StevenBlack/hosts `porn-only` (MIT; eki oluşturan kaynakların kendi lisansları), USOM zararlı bağlantılar (kamuya
  açık; yalnızca yolsuz ana bilgisayar adları alınır — bir URL çoğu kez paylaşımlı sitedeki tek dosyayı gösterir). Elle ek:
  merkezde `<kategori>.ek.txt`. Adresler `scripts/sunucu/merkez/listeler.py` başında.
- **Merkez:** `wc-listeler.timer` her gün **04:30** (`Persistent=true`) → `listeler.py` (wcpanel) indirir, temizler
  (geçerlilik, IP'ler, kamu son ekleri, aşırı geniş barındırma son ekleri, "asla" listesi), kategori başına
  `/var/lib/wificorrect/merkez/listeler/<k>.txt.gz` + `<k>.json` (sha256 = düz metnin özeti). Kaynak inmezse ya da HTML/boş
  /yarıdan küçük dönerse o kaynağın son temiz kopyası kullanılır (kopya okunurken bugünkü kurallarla yeniden süzülür);
  hiçbiri inmezse eski çıktı kalır. Sürümler `/api/saglik` cevabında (`listeler`), liste `/api/liste` (gzip). Yönetim →
  Engel listeleri (kaynak durumu, Şimdi güncelle); hareket `LISTE_GUNCEL` / `LISTE_HATA`. Sürümler yalnızca bağlı ve
  hizmeti açık cihaza bildirilir. **Sağlık izleme:** bir kategoride hiçbir kaynak inmediyse (`hata`) ya da son başarılı
  üretim 48 saatten eskiyse "Engel listeleri güncellenemiyor: …" (uyarı) açılır — müşteriye bağlı değil, Açık sorunlar'da
  "Merkez" satırı (sorun tablosunda müşteri 0), SMS kuralları aynı (600 sn sonra tek SMS, düzelince "düzeldi"). Tek
  kaynağın hatası (ör. USOM HTML) sorun açmaz.
- **Asla engellenmez** (alt adlarıyla): sistem ve giriş akışı (wificorrect.com, netgsm, twilio, bağlantı denetimi, NTP),
  google/apple/microsoft, gov.tr/edu.tr ve büyük ortak altyapılar (googleusercontent, azureedge, windows.net, cloudflare,
  trycloudflare, facebook/fbcdn/instagram, youtube/ytimg, akamaihd, live/office/outlook, whatsapp, wa.me), telefon
  üreticileri (hicloud, miui/xiaomi, oppomobile, samsung/samsungcloud), x/twitter, tiktok, amazon, trendyol, hepsiburada,
  papara, iyzico, bankalar (garantibbva, isbank, ziraatbank, akbank, yapikredi, vakifbank, halkbank, qnb, denizbank, enpara)
  ve turkiye.gov.tr. Cihaz da birleşik dosyayı yazarken bağlantı denetimi adlarını ve wificorrect/netgsm/twilio'yu
  (alt adlarıyla) atar (merkez listesi bozuk gelse de giriş akışı kırılmaz). **Paylaşımlı
  platformlar** (github, githubusercontent, dropbox, sharepoint, onedrive, discord, mega.nz, t.me, pastebin, bit.ly …) de
  engellenmez: DNS tek bir dosyayı engelleyemez, bütün platform herkes için kapanırdı; bedeli bu sitelerdeki oltalama
  sayfalarının açık kalması.
- **Cihaz:** `ctl saglik` sürümleri `/var/lib/wificorrect/liste/hedef.json`'a yazar; açık bir kategori eksik/eskiyse
  `systemd-run --unit wfc-liste … ctl liste-guncelle` indirir (`<k>.txt.gz` + `<k>.json`), sha256 tutmazsa yazmaz (eski kalır).
  Aynı sürüm hata verdiyse (`HAZIR_LISTE_HATA`, `hata.json`) bir saat yeniden denenmez. Birleşik dosya
  `/etc/wificorrect/hazir-liste.conf` (0644, `server=/alan/` → alan adı ve alt adları NXDOMAIN, sorgu 5651 DNS kaydına
  düşer); dnsmasq `servers-file` ile okur, değişince **SIGHUP** (`systemctl kill --kill-whom=main`). HUP başarısızsa
  `hup-bekliyor` işareti kalır, sonraki uygulamada yeniden denenir. Dosya hep vardır (boş da olsa; yoksa dnsmasq açılmaz).
  Denetim: `HAZIR_LISTE_GUNCEL`, `HAZIR_LISTE_HATA`, `PANEL_HAZIR_LISTE`.
- **Panel → Yasaklı siteler → Hazır listeler:** Aç/Kapat ayarı yazar ve `wfc-liste-uygula` işini (`ctl liste-uygula`)
  başlatıp hemen döner (birkaç saniyede geçerli; iş ayarı uygulama anında yeniden okur, sürerken gelen değişikliği de
  uygular); açılınca merkezdeki yenisi de indirilir. **İstisnalar** (alt dize, kelime filtresiyle aynı anlam) hazır listeden
  de çıkarır; "Bu adres engelli mi?" listeyi satır satır tarar. Fabrika dönüşü hazır listeyi istisnasız yeniden kurar.
- **Paket:** postinst ve dnsmasq birimi (ExecStartPre) boş `hazir-liste.conf` garanti eder; dnsmasq yalnızca çalışırken okuduğu `dnsmasq.d/wificorrect.conf` diskteki sürümden farklıysa yeniden
  başlar (ExecStartPost `dnsmasq-ozet` her açılışta PID + özeti `/run/wificorrect-dnsmasq-conf.sha256`ye yazar; kancasız
  yeniden başlama — ör. geri dönüş — PID tutmadığı için yakalanır), liste ondan sonra uygulanır.
- **5651 DHCP kaydı:** dnsmasq SIGHUP ve yeniden başlatmada bütün kiralar için dhcp-script'i `old` ile yeniden çağırır.
  `ctl dhcp-olay`, IP ve kira bitişi (`DNSMASQ_LEASE_EXPIRES`, yoksa `DNSMASQ_TIME_REMAINING`) son kaydedilenle aynı olan
  `old` olayını yazmaz, oturuma dokunmaz (`/srv/hotspot/state/dhcp_son.json`; gerçek yenileme bitişi uzattığı için yazılır).
  `add`/`del` her zaman yazılır; bitiş bilinmiyorsa eskisi gibi yazılır.

## Raporlar — haftalık ve aylık (2026-10-14)
Dal `aylik-rapor`; tasarım `docs/superpowers/specs/2026-10-14-aylik-rapor-design.md`, plan `docs/superpowers/plans/2026-10-14-raporlar.md`.
Yalnızca merkez; cihazda değişiklik yok.
- **Kim yazar, kim okur:** `wc-istatistik` (root, 02:30 + 12:30) `rapor.py` ile arşiv günlerinden **ham özet** çıkarır:
  `/var/lib/wificorrect/rapor-ham/<arşiv>/<gün>.json` (root 700/600; misafirler root'a özel sırla HMAC'lenmiş takma kimlik,
  sır `rapor-ham/.sir` — nokta: hiçbir arşiv adı noktayla başlamaz). Gün başına bir kez (imza değişmedikçe yeniden okunmaz/hash'lenmez).
  Sonra müşteri başına **dönem raporları**: `/var/lib/wificorrect/rapor/<numara>/{liste.json, hafta-YYYY-Www.json, ay-YYYY-MM.json}`
  (root:wcpanel 750/640; yalnızca sayılar, kişisel veri yok; en çok 26 hafta + 24 ay). Panel (wcpanel, `raporlar.py`) yalnızca
  bunları ve `sorun` tablosunu okur; `/srv` ve `rapor-ham`'a dokunmaz. Müşteri ↔ eski arşiv bağını panel
  `/var/lib/wificorrect/merkez/rapor-gruplari.json`'a yazar (açılışta ve bağ değişince); root bunu güvenilmez girdi olarak okur.
- **"Tam" gün:** gün klasörü tamam + MANIFEST'teki her dosyanın sha256'sı tutuyor + MANIFEST'in sha'sı zincir satırındakine eşit +
  zincir satırı doğru (cihazdaki `muhur::verify` ile aynı). Emin olunamayan her gün "Doğrulanamadı" (neden yazılır).
- **İçerik:** 5651 kayıt durumu (her gün), misafirler (farklı/yeni/dönen, giriş SMS/misafir kodu/izinli cihaz), saat ve gün
  yoğunluğu (CSS çubuk), kullanım (veri, doğrulama SMS'i; alan adı listesi 2026-10-15'te kaldırıldı, biçim 2),
  cihaz (dönemde açılan kritik/uyarı sorunlar, "en az" çevrimdışı süre, dönem sonu sürümü).
- **Engellenenler bölümü yok:** `dns.csv` yalnızca dnsmasq'ın sorgu satırlarından yazılır, engel cevabı işaretlenmez; kelime
  filtresi sorguyu dnsmasq'a hiç ulaştırmaz. Uydurma sayı gösterilmez; eklemek için cihazda kaydedicinin engel cevabını işaretlemesi gerekir.
- **Yazdırma:** JS yok (CSP); print CSS (menü gizli, A4) + "Ctrl+P ile yazdırın / PDF olarak kaydedin" notu.

### Raporlar cihaz panelinde (2026-10-15, sürüm 0.4.10)
Dal `rapor-cihaz`; spec "Ek (2026-10-15)", plan `docs/superpowers/plans/2026-10-15-rapor-cihaz.md` (API sözleşmesi).
- Cihaz paneli **Ayarlar → Raporlar** kutucuğu (Admin'den önce; işletme sahibi ve admin): `/raporlar` (önce haftalar, sonra
  aylar; "sürüyor"/"kesinleşmedi") ve `/rapor/<hafta|ay>/<dönem>` (merkezdeki bölümler: 5651 kayıt durumu, misafirler, giriş
  yöntemi, saat/gün çubukları, kullanım, cihaz). Kod `src/panel/rapor_sayfalari.rs`; JS yok, yazdırma CSS'i şablonda (menü gizli, A4).
- Veri istek anında merkezden: `POST /api/raporlar`, `/api/rapor` (cihaz anahtarı, curl, en çok 8 sn). Tür/dönem URL'de
  sıkı doğrulanır (geçersizse merkeze sorulmaz, 404). Merkezin cevabı güvenilmez girdi: alanlar tür denetimli, metinler kaçışlı.
- Önbellek (bellekte, müşteri numarasıyla): liste 5 dk, süren/kesinleşmemiş dönem 5 dk, kesinleşmiş geçmiş dönem 1 saat,
  404 5 dk, hata 1 dk. Merkezde raporların kendi sınırı var (IP başına dakikada 60); SMS/sağlık sayacına (dakikada 20) girmez.
  curl cevabı en çok 2 MB okunur.
- Merkeze ulaşılamazsa "Raporlar şu an alınamıyor…"; lisans/üyelik kapalıysa (yerelde ya da merkez 403) uygun not, yerelde
  kapalıyken merkeze sorulmaz; dosya yoksa "Bu dönem için rapor yok." Görüntüleme denetime yazılmaz (kişisel veri yok).

## AP kurulumu (2026-10-15)
Dal `ap-kurulum`; tasarım `docs/superpowers/specs/2026-10-15-ap-kurulum-design.md`, teknisyen rehberi `docs/AP_KURULUM.md`.
Sürüm **0.4.11**. Hizmet sağlayıcı müşterinin taktığı AP'yi cihaz üzerinden uzaktan kurar; merkezde değişiklik yok.
- **Panel → Ayarlar → AP kurulumu** (hizmet sağlayıcı ve işletme sahibi, admin kilidi yok — 2026-10-15 kullanıcı kararı;
  her POST CSRF'li, denetim satırında kim + rol): cihazın misafir
  portu (köprü adresi + MAC; AP'de "yönetime yalnızca şu IP/MAC" için), **Ağdaki cihazlar** (`/ap-kurulum/cihazlar`:
  dnsmasq kiraları + `ip -4 neigh show dev <köprü>`; rastgele MAC = telefon; her açılış `PANEL_AP_CIHAZLAR sayi=…`),
  **Geçici adres**, **AP adresleri**. Kod `src/ap.rs`, `src/panel/ap_sayfasi.rs`.
- **Geçici adres** (fabrika adresli AP'ye ulaşmak için köprüye ikinci adres): önce zamanlayıcı
  `systemd-run --unit wfc-ap-gecici-<kimlik> --on-active 1800s … ctl ap-adres-kaldir sure <kimlik>` (kurulamazsa hiçbir şey
  değişmez), sonra durum `/var/lib/wificorrect/ap-gecici.json` (600), sonra `ip addr add … dev <köprü> label <köprü>:ap
  valid_lft 1810 preferred_lft 1810` (çekirdek ömrü yedek emniyet). Eski zamanlayıcı başka kimlikli yeni adresi silemez.
  Ret: önek 16–30 dışı, RFC 1918 dışı, ağ/yayın adresi, misafir ağı, tünel ağı 10.99.0.0/24 (tünel kapalıyken de),
  cihazın canlı adres/rotalarıyla (WAN dahil) çakışma, zaten açık adres. Açılışta (`ctl yukle`) dosya kalmış ama adres
  canlıda yoksa temizlenir (`neden=acilis`); servis yeniden başlayınca açık adrese dokunulmaz. Fabrika dönüşünde kalkar
  (`neden=fabrika`). Konsol: `wificorrect ctl ap-adres-ekle ADRES [--sure 60-1800]`, `ctl ap-adres-kaldir elle`.
- **AP adresleri** (`ayarlar.toml` `[ap] adresler`, en çok 8; misafir ağında, köprü/ağ/yayın adresi değil, DHCP havuzu
  .20–.249 dışında): yalnızca kayıt — panelde ve ağdaki cihazlar listesinde "kayıtlı AP adresi". Fabrika dönüşünde boşalır.
- **Güvenlik duvarı kuralı yok (karar):** `kapi.nft`'teki köprü yalıtımı (`table bridge hotspot` → `isolate`) port→port
  iletimi zaten düşürüyor; AP'ye kablosuz bağlı misafirin AP'ye giden trafiği cihazdan hiç geçmez — onu yalnızca AP'nin
  kendi ayarı korur (kablosuzdan yönetim kapalı / yönetim yalnızca 10.50.0.1, parola = merkezin ürettiği müşteriye özel
  **AP yönetim parolası**; 2026-10-16'ya kadar işletme sahibinin panel parolasıydı — bu kuralla kurulmuş AP'ler için canlı
  kabul listesine bakın, "AP arayüzü — cihaz"). Yalıtım ileride gevşetilirse AP adresine köprü kuralı yeniden düşünülmeli.
- **Denetim:** `AP_KURULUM_ADRES eklendi adres=… sure=…` | `kaldirildi adres=… neden=elle|sure|acilis|fabrika`,
  `PANEL_AP_ADRES adresler=<a,b|-> eski=<…|->`, `PANEL_AP_CIHAZLAR sayi=…`.
- **Bilinen sınır:** AP'nin fabrika adresi modem ağındaysa geçici adres eklenemez (panel reddeder; müşteriden sıfırlama /
  DHCP istenir). Ayrı ağ ad alanı (netns) çözümü yapılmadı.
- **Windows betiği** `scripts/ap-ac.ps1` (+ `ap-ac.cmd`; PowerShell 5.1): `ssh -N -D 127.0.0.1:<port>` (BatchMode, 1080,
  doluysa 1081–1099) + geçici profilli Chrome (yoksa Edge) `--proxy-server=socks5://…`; pencere kapanınca ssh durur, profil
  silinir. `-Adres` yalnızca adres/ad + isteğe bağlı port/yol kabul eder; `-Kuru` yalnızca komutları yazar. Adres
  doğrulaması büyük/küçük harf duyarlı (`-cnotmatch`): duyarsız eşleşme Türkçe yerel ayarda `I` yüzünden şaşıyor.
  `.ps1`/`.cmd` depoda CRLF (`.gitattributes`), `.ps1` UTF-8 BOM'lu (5.1 BOM'suz dosyayı ANSI okur). ssh penceresi
  simge durumunda (gizli değil: konsol X ile kapanırsa tünel elle kapatılabilsin).
- **İnceleme düzeltmeleri:** geçici adres işlemleri `ap-gecici.kilit` (flock) ile sıralı (çift tıklamada iki zamanlayıcı
  kurulmaz); `ip` çıktısında köprünün kendi adresi yoksa (okunamadı) ekleme reddedilir, elle kaldırmada durum dosyası
  kalır; süre dolumunda `ip addr del` başarısız olsa da dosya silinir (`neden=sure hata=ip_addr_del|ip_okunamadi`;
  çekirdek ömrü adresi düşürür); bozuk durum dosyası panelde "Kaldır" ile temizlenir; `PANEL_AP_CIHAZLAR` panel
  hareketlerinde kişisel veri sayılır. AP rehberinde yönetim IP kısıtı en sona alındı (önce 10.50.0.2'ye taşı, yeni
  adresten gir, sonra kısıtla) — önce kısıtlanırsa geçici adresten bağlı oturum kesilir ve AP'ye ulaşılamaz.

## AP yan odası, SOCKS aracı, yabancı DHCP denetimi (2026-10-15)
Dal `ap-yan-oda`; spec `docs/superpowers/specs/2026-10-15-ap-kurulum-design.md` → "Ek: yan oda". Sürüm **0.4.12**.
Türkiye'deki modem/AP'lerin çoğu fabrikada 192.168.1.1 (ör. ZyXEL P-660HN-T1H) ve müşterinin modemi de çoğunlukla
192.168.1.0/24'te: 0.4.11'in "köprüye ikinci adres" yöntemi WAN'la çakıştığı için reddediyordu.
- **Yan oda** (eski geçici adresin yerine, tek yöntem): misafir köprüsüne **macvlan** (`mode private`) ile bağlı, adsız
  ayrı bir ağ ad alanı; adres (ör. `192.168.1.250/24`) orada. Ana ağ ad alanına adres/rota eklenmez → WAN, tünel ve
  rota çakışması denetimi kalktı; kalan kurallar: önek 16–30, özel aralık, ağ/yayın adresi değil, misafir ağıyla
  çakışmıyor. Yan odada `ip_forward=0` (yeni ad alanı ana ad alanının 1 değerini miras alıyor — cihazda görüldü), IPv6
  kapalı, tek rota kendi alt ağı.
- **Neden macvlan:** veth'i köprü portu yapmak `kapi.nft`'teki köprü yalıtımına (`table bridge hotspot` → `isolate`,
  forward kancası) takılır; macvlan'ın paketleri köprüden "yerel çıkış" olarak geçer, yalıtıma takılmaz. Cihazda yalnızca
  geçici ad alanlarında (kopya köprü + aynı yalıtım kuralı) denendi: veth → AP'ye ulaşamadı, macvlan → ulaştı. Yan oda
  açıkken köprü `promiscuity 1` olur (macvlan'ın MAC'i için), kapanınca 0'a döner.
- **Ömür:** ad alanını `wfc-ap-araci` geçici birimi (`systemd-run --service-type=notify`, `TimeoutStartSec=12`,
  `RuntimeMaxSec=süre+10`) tutar: bir iş parçacığı `unshare(CLONE_NEWNET)` yapar, ana ad alanından
  `ip link add link <köprü> name wfcap0 address <rastgele yerel MAC> netns <tid> type macvlan mode private` ile arayüz
  doğrudan yan odada doğar (ana ad alanında hiç görünmez; `ip netns add`/mount yok — panelin `PrivateTmp`'li mount ad
  alanı sorun olmaz). Süreç bitince (zamanlayıcı, "Şimdi kapat", `RuntimeMaxSec`, çökme, yeniden başlatma, fabrika)
  ad alanı ve macvlan çekirdekçe silinir. Sıra: kapatma zamanlayıcısı → durum dosyası → aracı; aracı açılamazsa üçü
  geri alınır. flock, durum dosyası (`/var/lib/wificorrect/ap-gecici.json`), açılış temizliği (aracı yoksa
  `neden=acilis`), fabrika kapanışı aynen.
- **SOCKS5 aracı** (`ctl ap-araci ADRES`, yalnızca birim çalıştırır; `src/yan_oda.rs`): yalnızca `127.0.0.1:1085`,
  kimliksiz, yalnızca CONNECT, IPv4 ya da IPv4 yazılmış alan adı (Chrome IP'yi de alan adı biçiminde gönderir); hedef
  yalnızca yan odanın alt ağı (ağ/yayın adresi, port 0 hayır), ad çözümü yok (AP'ye IP ile girilir). Her bağlantının
  iş parçacığı `setns` ile yan odaya girip bağlanır; en çok 32 bağlantı, el sıkışma 10 sn, bağlanma 5 sn, boşta 300 sn.
- **Yabancı DHCP denetimi** (panel → AP kurulumu → "Yabancı DHCP sunucusu var mı?" → `ctl ap-dhcp-denetle`, panel
  doğrudan çalıştırır): kısa ömürlü adressiz yan oda (`wfcdhcp0`), AF_PACKET ile iki DHCPDISCOVER (yayın bayrağı),
  ~3 sn teklif toplanır; çıktı `TEKLIF <sunucu> <mac> <adres>` + `TAMAM <n>`. Cihazın kendi dnsmasq'ı bu yayını almıyor
  (köprüden yerel çıkış); yine de köprü adresi + köprü MAC'inden gelen teklif atılır. Yabancı varsa kırmızı uyarı
  ("Bu cihazın DHCP sunucusu kapatılmalı; açık kaldıkça bazı misafirler internete çıkamaz") + sunucu IP, MAC, teklif.
  **Kendiliğinden fark etme (kaydedici + Özet + merkez sağlık raporu) yapılmadı:** 5651 kaydedicisine sürekli açık bir
  ham paket dinleyicisi (BPF süzgeçli) eklemek gerekir, yalnızca yayınla gönderilen teklifleri görür ve merkezdeki
  sağlık kurallarının da değişmesi gerekir; maliyet/risk küçük değil.
- **Denetim:** `AP_YAN_ODA acildi adres=… sure=…` | `kapandi adres=… neden=elle|sure|acilis|fabrika [hata=araci_durmadi]`
  (eski `AP_KURULUM_ADRES` yerine), `AP_DHCP_DENETIM sonuc=yok|var sunucular=<ip/mac,…>|hata`. Panelden yapılanlarda
  kim + rol.
- **Windows betiği:** `-Adres` misafir ağında (`-MisafirAg`, varsayılan 10.50.0.0/24) ya da alan adıysa eskisi gibi
  `ssh -D`; misafir ağı dışında bir IPv4 ise `ssh -L 127.0.0.1:<port>:127.0.0.1:1085` (`-AraciPort`) + Chrome yine
  `socks5://127.0.0.1:<port>` ile AP'nin kendi adresini açar (Referer/Host bozulmaz). Hazırlık denetimi artık SOCKS5 el
  sıkışması (`05 01 00 → 05 00`); yan oda kapalıysa "Cihazda yan oda açık değil" der.
- **Cihazda doğrulama (yalnızca geçici ad alanları, canlı ağa dokunulmadı; önce/sonra `ip addr`, `ip route`, `ip link`,
  `nft -s list ruleset` aynı):** gerçek ikiliyle sahte "AP" (192.168.1.1, dnsmasq DHCP + web) ve kopya köprü
  (10.50.0.1/24 + yalıtım kuralı + kendi dnsmasq'ı): DHCP denetimi yalnızca AP'nin teklifini buldu (3,1 sn); yan oda
  `192.168.1.250/24`, `ip_forward=0`, IPv6 kapalı; SOCKS ile `http://192.168.1.1/` açıldı (IP ve alan adı biçimli IP),
  `10.50.0.1`, `tplinkap.net`, kapalı port reddedildi; aracı durunca macvlan gitti, köprü promiscuity 0; `systemd-run
  --service-type=notify` hazır olunca döndü, port doluyken başarısız döndü, `RuntimeMaxSec` aracıyı durdurdu; DHCP
  denetimi panelin korumalarıyla (`ProtectSystem=full`, `PrivateTmp`, `NoNewPrivileges`, `ProtectHome`) da çalıştı.
- **Port değişikliği:** panel → Portlar → Uygula önce açık yan odayı kapatır (`kapandi … neden=port`; aracı
  durdurulamazsa port değişikliği yapılmaz): köprü yeniden kurulunca arayüzsüz "açık" yan oda kalmaz.
- **İnceleme düzeltmeleri (final review, 2026-10-15):**
  - Aracı birimi kısıtlı: `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`,
    `CapabilityBoundingSet=CAP_NET_ADMIN CAP_SYS_ADMIN`, `RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK`,
    `TasksMax=128`, `MemoryMax=64M`, `ProtectControlGroups`, `LockPersonality` (`ap::ARACI_KISIT`). `PrivateNetwork`,
    `RestrictNamespaces`, `ProtectKernelTunables` bilerek yok (unshare/setns ve yan odanın `/proc/sys/net` yazımı).
    **Cihazda bu kısıtlarla henüz çalıştırılmadı** — canlı kabulde ilk iş.
  - DHCP denetimi `timeout 25` ile çalışır (süre dolunca süreç ve yan odası gider) ve aynı anda tek denetim (ikinci
    istek "Bir denetim zaten sürüyor"): panelin 4 iş parçacığı bağlanmaz.
  - Kabul döngüsü kalıcı hatada (EMFILE vb.) 100 ms bekler; durum dosyası olmadan çalışan aracı panelde "durum kaydı
    yok — Kapat" olarak görünür, elle kapatma (panel/`ctl ap-adres-kaldir elle`/fabrika) onu da durdurur
    (`kapandi adres=-`).
  - `ctl ap-adres-kaldir` 0.4.11'in köprüdeki geçici adresini de siler (`ip addr flush dev <köprü> label <köprü>:ap`;
    o sürümün zamanlayıcısı güncellemeden sonra çalışırsa).
  - `ap-ac.ps1` hazırlık denetimi istisna türüne değil aşamaya bakar (bağlanamadı / bağlandı ama SOCKS cevabı yok);
    Windows PowerShell 5.1'de kapalı port ve sessiz dinleyiciyle denendi. Gerçek ssh ile (-D ve -L) canlı kabulde.
  - Yan oda formunda not: AP'nin DHCP havuzu dışında bir adres seçin (IPv4'te çakışma denetimi yok).
- **Canlı kabul listesi** (gerçek ZyXEL/TP-Link, uzaktan güncellemeyle 0.4.12; henüz yapılmadı): kısıtlı aracı açılıyor
  ve AP'nin sayfası geliyor (-D ve -L); yan oda aç/kapat ve DHCP denetiminden sonra ana ad alanında
  `sysctl net.ipv4.ip_forward` hâlâ 1; köprü promiscuous iken misafir trafiği ve işlemci normal; yan oda açıkken gerçek
  bir misafir girişi çalışıyor; kapanınca köprü promiscuity 0.

## KVKK veri sorumlusu bilgileri (2026-10-15, 0.4.12)
- Ayarlar → İşletme'de unvanın yanında: **Veri sorumlusunun adresi** (`main.adres`; boş ya da 5–200 karakter, köşeli
  parantez yok), isteğe bağlı **İrtibat telefonu** (`main.telefon`), **E-posta** (`main.eposta`), **KEP adresi**
  (`main.kep`). Kodda işletmeye özel varsayılan yok (boş başlar; fabrika dönüşünde boşalır).
- Aydınlatma metninde yer tutucular: `[vergi levhası unvanı]`, `[başvuru adresi]` (5. maddede "Şahsen merkezimize
  ([başvuru adresi]) gelerek" ve iletişim listesinde "Müracaat Adresi"), `[irtibat numarası]`, `[e-posta adresi]`,
  `[KEP adresi]` (5. maddede "… adresiniz kanalıyla [KEP adresi] KEP adresimize" ve listede). Portal unvan gibi
  yerleştirir (TR ve EN metinler, açık rıza dahil): unvan/adres boşsa yer tutucu görünür; isteğe bağlı bilgi boşsa
  geçtiği **satır gösterilmez** (KEP'siz işletmede KEP ile başvuru satırı da çıkar). Hukuki metnin geri kalanı aynı;
  yeni kalıp hukukçu onayı bekliyor.
- Unvan ya da adres boşsa (cihaz bağlıyken) her sayfada uyarı: "KVKK metninde veri sorumlusu bilgileri eksik (unvan,
  adres) …".
- Merkeze: adres günlük eşitlemede (`/api/eslesme`, unvan gibi) gider; merkezde `cihaz.adres` sütunu (eski veritabanına
  ALTER), yönetimde müşteri sayfasında "Adres". Adres göndermeyen eski cihaz kayıtlı adresi silmez. İlk bağlanmada
  (`/api/giris`) gönderilmez (o anda girilmemiş olur).
- Canlı Göztepe cihazı: 0.4.12'ye geçince eski metin yenisiyle değişir ve adres boş olduğu için yer tutucu görünür;
  güncellemeden sonra panelden unvan + adres (+ telefon, KEP, e-posta) girilmeli.


## Kolay bağlama (2026-10-16, sürüm 0.4.13)
Dal `kolay-baglama`; tasarım `docs/superpowers/specs/2026-10-16-kolay-baglama-design.md`, plan `docs/superpowers/plans/2026-10-16-kolay-baglama.md`.
- **Akış:** bağlanmamış cihaz (merkez.json yok) internete çıkınca kaydedici 2 dk'da bir `systemd-run --unit wfc-bekliyor … ctl bekliyor`
  başlatır → `POST /api/bekliyor` (kimlik, sır, sürüm, WAN MAC, WireGuard + yedek açık anahtarı). Müşteri panel.wificorrect.com'a
  (ya da cihaz.wificorrect.com → panel girişi) girer; bağlı cihazı yoksa `/baglan`: aynı internet adresinden son 10 dk'da
  bildiren tek aday → müşteriye ayrılır (`CIHAZ_OTOMATIK_AYRILDI`), çok aday → liste (ilk görülme, WAN MAC son 4, sürüm), hiç →
  "bulunamadı" + yeniden dene. Ayrılan cihaz sonraki bildiriminde `baglan` + tek kullanımlık belirteç (10 dk) alır ve
  `POST /api/bagla` ile parolasız bağlanır (yan etkiler `/api/giris` ile aynı; cevapta `numara`). Sayfa 5 sn'de bir yenilenir;
  bağlandıktan 30 sn sonra cihaz paneline geçer (tünel kalksın). Yerel `https://<ip>:8443` bağlama sayfası yedek yol olarak kalır.
- **Kimlik:** cihazda `/etc/wificorrect/bekleme.json` (600; 32 hex kimlik + 64 hex sır), ilk `ctl bekliyor`da üretilir, fabrika
  dönüşünde kalır. Merkez sırrın yalnızca SHA-256'sını tutar (ilk bildirim kaydeder); sırrı tutmayan bildirim 401 ve kaydı değiştirmez.
- **Merkez kuralları:** tablo `bekleyen`; 30 dk bildirmeyen yönetimde "eski", 7 gün bildirmeyen silinir; ayrılmış ama 15 dk'da
  bağlanmamış cihaz serbest kalır; wg anahtarı bağlı bir cihazda olan kayıt aday olmaz; bu kayıttan bağlanmış cihaz cevabı
  kaybettiyse (bağ duruyor, cihaz yine bildiriyor) yeniden `baglan` alır (aynı müşteri, aynı wg anahtarı). Hız: IP başına genel
  sınır + kimlik başına dakikada 3 + IP başına saatte 10 yeni kimlik; tabloda en çok 2000 kayıt. Üyeliği bitmiş müşteri bağlamaz.
- **Yönetim → Bekleyen cihazlar** (salt okunur): kimlik (ilk 8), ilk/son görülme, internet adresi, sürüm, WAN MAC, durum.
- **Denetim:** cihaz `MERKEZ_BEKLIYOR durum=bekle|ulasilamadi` (yalnızca değişince; `/var/lib/wificorrect/bekleme-durum`),
  `MERKEZ_BAGLANDI … yol=otomatik`, `MERKEZ_BAGLAMA_HATA`, ayrıca `ADMIN_PAROLA_MERKEZ` / `SMS_AYARI_MERKEZ`; merkez `CIHAZ_BEKLIYOR`
  (ilk görülme), `CIHAZ_OTOMATIK_AYRILDI secim=tek|liste`, `CIHAZ_BAGLANDI … otomatik`.
- **Caddy değişikliği yok:** cihaz.wificorrect.com oturumsuzken zaten panel girişine yönlenir; oturum varken panel `/giris` artık
  giriş sonrası sayfaya (cihaz paneli ya da bağlama) gider. İstemci adresi `X-Forwarded-For`'un son öğesi (api ve panel aynı Caddy).
- **Bilinen sınır:** müşterinin tarayıcısı IPv6, cihaz IPv4 ile çıkarsa adresler eşleşmez; ikisi de IPv6 ile çıksa da eşleşmez
  (gizlilik adresleri aynı /64'te bile farklı) — alan adlarında AAAA kaydı yoksa sorun yok, eklenirse cihaz curl'üne `-4` verilmeli;
  CGNAT'ta aynı dış adresi paylaşan başka bir bekleyen cihaz varsa liste gösterilir (kendiliğinden bağlanmaz).
- **Kalan risk (kabul edildi, spec "Anti-hedefler"):** aynı dış adresi paylaşan biri (müşterilere açık modem Wi-Fi'ı, CGNAT)
  hesapsız sahte cihaz bildirip, sahip gerçek cihazdan önce girerse onu müşteriye bağlatabilir. En aza indirme: `/api/bagla` ve bu
  yolla bağlanmış cihazın (`cihaz.otomatik=1`) eşitlemeleri ortak NetGSM bilgisini taşımaz; parola özeti, admin özeti ve cihaz
  anahtarı bağlama için gerekli olduğundan kalır (`/api/giris` değişmedi). Görme ve kurtarma: Yönetim → Bekleyen cihazlar ve
  hareketler (`CIHAZ_BAGLANDI … otomatik kimlik=…`, `CIHAZ_BEKLIYOR_RED` / `CIHAZ_BAGLA_RED` — sırrı ya da belirteci tutmayan
  deneme, kimlik başına saatte bir satır); yanlış bağ "zorla ayır" ile kalkar.
- **Kayıt hatasında takılmama:** kuyruk (tünel + yedek hesabı) başarısız ya da zaman aşımı olursa (`CIHAZ_KAYIT_HATA`, 503) bekleme
  kaydı bağa önceden işaretlendiği için cihaz sonraki bildiriminde yeniden `baglan` alır ve dener; müşteri bu sürede (en çok 15 dk)
  "bağlanıyor" görür, "ulaşılamıyor"a düşmez. Tablo tavanı (2000) doluysa 10 dk'dır bildirmeyen, kimseye ayrılmamış en eski kayıt
  yer açar. Hız sınırlayıcılar pencereden eski anahtarları bellekten siler.
- **Kurulum sırası:** önce merkez (`merkez-guncelle.sh`), sonra cihazlar (0.4.13); ISO yeniden üretilir (yeni cihazlar kutudan çıkınca bildirsin).

## Kurulum ISO'su gerçek donanımda (2026-10-16, sürüm 0.4.14)
Dal `iso-donanim`. İlk gerçek kurulum (v0.4.13 ISO, J1900, 2× Realtek r8169, UEFI, USB Rufus DD kipiyle yazıldı) iki kez durdu:
- **Yanlış kurulum ortamı:** "Failed to retrieve the preconfiguration file file:///cdrom/preseed.cfg". Kurucu kabuğunda
  `/cdrom` bir Ubuntu kurulum ağacıydı (casper, ubuntu, dists, pool…): `cdrom-detect` adayları sırayla dener ve yalnızca
  `.disk/info` dosyasına bakar; takılı başka bir Ubuntu belleği/diski bizimkinden önce bağlandı. Disk çıkarılınca preseed yüklendi.
- **DHCP başarısız** ("Otomatik ağ yapılandırması başarısızlıkla sonuçlandı"), kablo sağlam, Ethernet 1 modemde. Preseed
  `hw-detect/load_firmware false` idi; RTL8168 ailesinin bir kısmı `rtl_nic` aygıt yazılımı ister (canlı cihazda
  `firmware-realtek` kurulu).

Düzeltmeler:
- **preseed initrd'de:** `scripts/iso.sh` Debian'ın `install.amd/initrd.gz`'sinin arkasına ikinci bir cpio ekler:
  `/preseed.cfg`, `/wificorrect-kimlik` ve `iso/initrd/*` → `/wificorrect/` (`/lib` değil: initrd'de `/lib` sembolik bağdır, ikinci arşivdeki aynı adlı dizin onu ezer ve kurucu çekirdek paniğiyle açılmaz) (`ortak.sh`, `basla.sh`, `list-devices`,
  `disk.sh`). Menüde `preseed/file` yok; preseed hangi disk bağlanırsa bağlansın yüklenir (UEFI ve BIOS aynı initrd).
- **Kimlik:** her ISO derlemesinde yeni `WifiCorrect <sürüm> <uuid>`; ISO'da `/.wificorrect`, initrd'de `/wificorrect-kimlik`.
  `preseed/early_command` (`basla.sh`, cdrom-detect'ten önce) `list-devices`'ı sarmalayıcıyla değiştirir: çağıran
  cdrom-detect ise adaylar salt okunur ISO 9660 olarak denenir, yalnızca kimliği tutan verilir (seri: `KURULUM ORTAMI
  ADAYI`); diğer çağıranlar gerçek listeyi görür. Bulunamazsa cdrom-detect'in kendi "ortam yok / yeniden dene" ekranı
  çıkar. `disk.sh` (partman'dan önce) `/cdrom`'u yeniden doğrular; tutmazsa "Kurulum belleği okunamadı" ekranında kalıcı
  durur, hiçbir diske dokunulmaz. Böylece apt (cdrom) ve `late_command` (`/cdrom/wificorrect/son.sh`) hep bizim ortamı kullanır.
- **Ağ:** `hw-detect/load_firmware true` (eksik aygıt yazılımı sorulmadan ISO'nun `/firmware`'inden; kurucu ilgili paketi
  hedefe de kurar), `pkgsel/include` += `firmware-realtek`, `netcfg/link_wait_timeout 10`, `netcfg/dhcp_timeout 60`.
  DHCP yine olmazsa kurucunun "yeniden dene" ekranı kalır; ağsız devam edilirse `disk.sh` varsayılan rota yoksa
  "İnternet bağlantısı yok" ekranında durur (disk silinmeden).
- **USB belleğe kurulum** (kullanıcı isteği: J1900'de iç disk yok, ikinci boş 32 GB bellek): `disk.sh` kural değişti —
  kurulum ortamının kendisi (`/cdrom`'daki aygıt) ve bağlı (mount) diskler hiç seçilmez, 7 GB'tan küçük diskler atlanır;
  **önce iç disk** (USB'ye bağlı olmayan), yoksa **ilk USB disk** — ISO 9660 içeren (başka bir kurulum belleği) USB atlanır.
  Seçim seride `KURULUM DISKI: <aygıt> (ic|usb)`. GRUB o diske, UEFI'de çıkarılabilir yol da (`force-efi-extra-removable`)
  yazılır: bellek başka makinede/UEFI kayıtları silinmişken de açılır. Kök dosya sistemi `noatime` (`son.sh`; flash aşınması).
  USB'ye kurulduysa bitti mesajı "yalnızca kurulum belleğini çıkarın; WifiCorrect'in kurulduğu bellek takılı kalmalı" der.
- **Duman testi** (`scripts/iso-test.sh`): kurulum ortamı artık `-cdrom` değil, salt okunur ham ISO'lu USB bellek (Rufus DD
  gibi); `/cdrom`'un kimlikle seçildiği denetlenir. Aşamalar: yalnız kurulum belleği (+64 MB yabancı bellek) → "disk yok"ta
  durur; iç virtio disk → `/dev/vda (ic)` + açılış + SSH; iç disk yokken ikinci boş 8 GB USB bellek → `(usb)` + UEFI
  kayıtları sıfır makinede o bellekten açılış. Yabancı bellek ve ISO her aşamada değişmemiş olmalı.

**Teknisyen notu:** kurulumda WifiCorrect kurulum belleği ve (iç disk yoksa) kurulacak boş bellek dışında **bütün USB
bellek ve diskleri çıkarın**. İç disk (SSD) önerilir; USB belleğe kurulum çalışır ama flash daha çabuk aşınır (en az
8 GB, tercihen kaliteli/USB 3). Ethernet 1 modeme takılı olmalı.

## AP arayüzü — cihaz (2026-10-16, sürüm 0.4.15)
Dal `ap-arayuz`; tasarım `docs/superpowers/specs/2026-10-16-ap-arayuz-design.md` (sözleşme: "Mimari"). Merkez tarafı
(ap.wificorrect.com, `/_ap-gecis`, `/_ap-giris`, `/ap-yetki`, `wca` çerezi) ayrı dalda (`ap-arayuz-merkez`).
- **Takılan cihaz** (panel → Ayarlar → AP kurulumu, en üstte; sahip ve hizmet sağlayıcı, CSRF + denetim):
  **Etiketteki adres** (IP ya da bilinen ad: tplinkap.net/tplinkrepeater.net/tplinkeap.net → 192.168.0.254,
  tplinkwifi.net/tplinklogin.net/tendawifi.com/dlinkrouter.local → 192.168.0.1, routerlogin.net/.com, myrouter.local,
  mwlogin.net, netis.cc → 192.168.1.1, dlinkap.local → 192.168.0.50, miwifi.com → 192.168.31.1, fritz.box →
  192.168.178.1; `http://`, yol, port atılır) ve isteğe bağlı **Marka / model** `[ap] etiket`, `[ap] marka`'ya yazılır
  (değişince `PANEL_AP_ETIKET`; fabrika dönüşünde boşalır). Özel ağ adresi olmalı, /24'ün .0/.255'i ve cihazın kendi
  adresi değil. Etiketteki kullanıcı adı/parola istenmez.
- **Bağlan / Otomatik bul** = arka plan işi: panel `/run/wificorrect/ap-bul.json`'a "sürüyor" yazar ve
  `systemd-run --collect --unit wfc-ap-bul --property=RuntimeMaxSec=180 … ctl ap-bul KIM ROL IP [HEDEF]` başlatır
  (aynı anda tek arama; birim varken yeni istek bekleme sayfasına gider). Bekleme sayfası `/ap-kurulum/bul` 2 sn'de bir
  kendini yeniler (meta, JS yok), iş bitince AP kurulumuna döner; sonuç orada. İş (`src/ap_bul.rs`):
  - misafir ağındaki adres: yan oda yok, ana ad alanından web arayüzü (HTTPS tercihi, aşağıda);
  - diğerleri: kısa ömürlü adressiz yan odadan (`wfcarp0`, macvlan, ayrı ağ ad alanı) **ARP yoklaması** (gönderen
    0.0.0.0, RFC 5227: kimsenin ARP tablosu değişmez): hedef cevap veriyor mu ve aynı /24'te .250, .249 … .241'den ilk
    boş adres (ARP'ye cevap yok, hedefin kendisi değil). Hedef yoklamaya cevap vermezse bir kez de boş adresten sorulur.
    Cevap yoksa yan oda açılmaz: "Bu adreste cihaz yok — kabloyu ve adresi kontrol edin". Cevap varsa yan oda
    `<boş adres>/24` açılır (`ap::ekle`, 30 dk kuralları; aynı ağda açık yan oda varsa o kullanılır, başka ağdaysa
    `kapandi … neden=yeni_hedef`), sonra aracı (127.0.0.1:1085) üzerinden web arayüzü (HTTPS tercihi, aşağıda) →
    "Cihaz bulundu (HTTPS)" / "(HTTP)" ya da "web arayüzü (80/443) açılmadı" (yan oda açık kalır).
  - Otomatik bul: kodda 23 yaygın fabrika adresi (`ap::OTOMATIK`: 192.168.1.1, 192.168.0.1, 192.168.0.254,
    192.168.1.254, 192.168.10.1, 192.168.2.1, AirTies 192.168.2.254, Huawei 192.168.8.1 / 192.168.100.1 / 192.168.3.1,
    10.0.0.1, 10.0.0.138, D-Link AP 192.168.0.50, Ubiquiti 192.168.1.20, Cisco 192.168.1.245, Ruijie 192.168.110.1 /
    10.44.77.254, Xiaomi, Asus, MikroTik, Fritz!Box, Buffalo, Apple) tek ARP taramasında yoklanır; cevap verenler
    sırayla denenir, web arayüzü olan ilkinde yan oda açık kalır (olmayanda `kapandi … neden=web_yok`). Birkaç saniye.
  - Denetim (panel gibi `kullanici=… rol=… ip=…`): `AP_BAGLAN tur=baglan|otomatik sonuc=bulundu|web_yok|yok|hata
    [hedef=… port=… yol=…]`, `AP_YAN_ODA acildi|kapandi`.
- **AP arayüzüne git** (bulunan cihaz, açık yan odanın ağındaki adres formu, kayıtlı AP adresleri; yeni sekme):
  hedef yalnızca açık yan odanın ağında (aracısı çalışan; ağ/yayın ve yan odanın kendi adresi değil) ya da kayıtlı AP
  adresi (misafir ağında, havuz dışı, cihazın adresi değil). Web arayüzü yeniden seçilir (HTTPS tercihi, aşağıda). Yazılır:
  `/run/wificorrect/ap-hedef.json` (600; adres, port, şema, yol `yan_oda|misafir`, bitiş = şimdi + 30 dk, yan odada en
  çok yan odanın bitişi, yan oda kimliği, kim/rol), denetim `AP_ARAYUZ acildi hedef=IP:port yol=… sure=…` (önceki başka
  hedef `kapandi … neden=yeni_hedef`). Cevap 303 değil, `https://cihaz.wificorrect.com/_ap-gecis`'e **meta yenilemeli**
  sayfa (+ "açılmazsa tıklayın"): Caddy'nin `form-action` CSP'si POST sonrası başka alana yönlendirmeyi kesmesin. Panelde
  "AP arayüzü erişimi açık … Erişimi kapat" (`kapandi … neden=elle`). Yan oda kapanınca (panel, zamanlayıcı `ctl
  ap-adres-kaldir sure`, port değişikliği, fabrika) yan oda hedefi de kapanır (`neden=yan_oda|port|fabrika`).
  Merkez eski sürümdeyse `/_ap-gecis` cihaza panel isteği olarak gelir: "AP erişimi şu an kullanılamıyor" (503).
- **HTTPS tercihi (0.4.16, 2026-10-18; ilk canlı AP denemesinin bulgusu):** modern bir ZyXEL modem (Vue tek sayfa
  uygulaması, 2020 ürün yazılımı) girişi ve cgi-bin isteklerini yalnızca `location.protocol != "https:"` iken AES/RSA ile
  şifreliyor. Arayüz tarayıcıda https://ap.wificorrect.com'da açıldığı için tarayıcı düz JSON gönderiyordu; aktarım
  ZyXEL'e HTTP'den (80) gidince ZyXEL gerçek RSA anahtarı veriyor ve şifreli istek bekliyordu → giriş hep `/login`'e
  dönüyordu. HTTPS'ten (443) aynı ZyXEL `RSAPublicKey` "None" döndürüp düz JSON kabul ediyor (yan oda aracısı üzerinden
  `curl -k https://192.168.1.1/` → 200; `/getRSAPublickKey` HTTPS'te 47 bayt, HTTP'de 506). **Kural**
  (`ap_arayuz::web_sec` / `web_bul`; "Bağlan", "Otomatik bul" ve "AP arayüzüne git" aynı kuralı kullanır): önce 443'e
  aktarımın kendi TLS ayarıyla (`tls_baglantisi`: rustls, TLS 1.2+, sertifika doğrulamasız, SNI `ap.wificorrect.local`)
  el sıkışma denenir (en çok 5 sn, bütün el sıkışmaya; parça parça cevap veren cihaz süreyi uzatamaz); başarılıysa
  HTTPS/443. Değilse 80'e TCP bağlantısı kuruluyorsa HTTP/80. Yalnızca TCP'si açık 443 yetmez: TLS 1.0/1.1'li ya da
  yalnızca CBC / RSA anahtar değişimli eski AP (rustls'te yok) ve 443'te düz HTTP konuşan cihaz 80'e düşer; 80 de yoksa
  "web arayüzü açılmadı". Panelde "Cihaz bulundu (HTTPS)" / "(HTTP)", açık erişim satırında "(HTTPS, yan odadan)". AP'nin
  aynı adreste başka şemaya yönlendirmesi hedefi iki yönde de günceller (http → https, https → http). **HTTP ile aç**
  (HTTPS'ten bulunan cihazın ve açık erişim satırının yanında; HTTPS arayüzü bozuk AP için): aynı form, `sema=http`,
  yalnızca 80 denenir, hedef http/80; doğrulama ve denetim aynı (`AP_ARAYUZ acildi hedef=IP:80 …`). "AP arayüzüne git"
  şemayı her seferinde yeniden seçer. AP adresi kaydında MAC'i çözdürmek için tek TCP bağlantısı yeter (TLS denenmez).
  Testler: gerçek TLS sunucusu (tiny_http + rustls; sertifika testte `openssl` ile üretilir, depoda anahtar yok), TLS 1.0
  ServerHello'su, TLS uyarısı, 443'te düz HTTP, hemen kapatan ve hiç cevap vermeyen sahte 443'ler; HTTPS hedefe gerçek
  TLS üzerinden aktarım.
- **Aktarım** (`src/ap_arayuz.rs`, `src/panel/ap_aktarim.rs`): `X-WFC-AP: 1` yalnızca `X-WFC-Kullanici` ile aynı güven
  kuralıyla (açık tünel `wfc` + kaynak merkezin tünel adresi) geçerli; başka kaynakta yok sayılır (istek panele düşer,
  bağlı cihazda 404). AP kipinde müşteri numarası `X-WFC-AP-Kullanici`'de (`X-WFC-Kullanici` gelmez: istek panele düşse de
  oturum açtırmaz); bağlı müşterinin numarası değilse 403. Hedef her istekte yeniden denetlenir;
  yoksa/süresi dolmuşsa sade "AP erişimi kapalı" sayfası (403; süresi dolan `kapandi … neden=sure`, kim hedeften; sayfada
  merkezin temizleyerek çıkış bağlantısı `/_ap-cikis`).
  Aktarımlar panelin 4 iş parçacığını tutmaz: ayrı 4 iş parçacığı + 64'lük kuyruk (dolarsa 503). AP kipindeki hiçbir
  cevaba panelin CSP / X-Frame-Options: DENY / Referrer-Policy başlıkları eklenmez.
  - **AP'ye gitmeyen tarayıcı istekleri** (final review C1/I2; `ap_arayuz::istek_reddi`, 403 sade sayfa, AP'ye bağlanılmaz):
    Service Worker betiği (`Service-Worker` başlığı ya da `Sec-Fetch-Dest: serviceworker`; merkezin Caddy'si de reddeder);
    `Sec-Fetch-Site: cross-site`; `same-site` (başka wificorrect.com alt alanı) ise yalnızca üst düzey GET/HEAD gezinme
    (`navigate` + `document`: cihaz panelinden gelen yönlendirme zinciri ve AP'nin o zincirdeki kendi yönlendirmeleri)
    geçer. AP'nin kendi sayfaları (`same-origin`), elle yazılan adres (`none`) ve başlıksız eski tarayıcı geçer. AP'nin
    `Service-Worker-Allowed` cevap başlığı atılır.
  - **Kayıtlı AP adresinde MAC denetimi** (final review I1): AP adresleri kaydedilirken her adresin o anki MAC'i
    (`ip -4 neigh show dev <köprü>`; yoksa AP'ye bir kez bağlanılıp yeniden bakılır) `[ap] mac`'e (adres → MAC) sabitlenir;
    okunamazsa önceki sabit MAC kalır ve uyarı çıkar ("MAC adresi okunamadı …"); aynı listeyi yeniden kaydetmek yeniden
    sabitler (denetim `PANEL_AP_ADRES adresler=… eski=… mac=adres/MAC,… eski_mac=…`). MAC'i sabitlenmemiş adrese panelden
    gidilmez. "AP arayüzüne git" bağlantı denemesinden sonra komşu tablosundaki MAC'i sabitlenenle karşılaştırır (farklıysa
    hedef yazılmaz, `AP_ARAYUZ red hedef=… neden=mac mac=… simdi=…`); hedefe MAC yazılır, sabit MAC değişir/silinirse açık
    erişim kapanır (`adres_kaydi`). Aktarımda her bağlantıdan sonra `/proc/net/arp`'taki MAC (çözülmüş kayıt) hedefinkiyle
    aynı olmalı; değilse 403 "AP'nin MAC adresi değişmiş", erişim kapanır (bir kez `AP_ARAYUZ kapandi … neden=mac mac=…
    simdi=…`). Misafir ağında AP'nin adresini elle alan bir cihaz AP yerine geçip yazılan parolayı toplamasın; ARP
    zehirlemesine karşı tam koruma değildir (AP açıkken iki cihaz aynı adresi kullanırsa komşu kaydı değişebilir).
  - **Hedef dosyası yarışı** (final review M1): hedefi yazan işlemler (git, şema/port değişimi, MAC'te kapatma, kapat)
    süreç içi kilitle sıraya girer; okunamayan hedef dosyası `Yok` sayılır ama silinmez.
  - **AP yönetim parolası:** merkezden gelir (`merkez.json` `ap_parola`, bağlanırken ve her eşitlemede; 0.4.17'den itibaren 5 dk'lık
    sağlık cevabında da, bkz. "PPPoE"), panel → AP kurulumu → Takılan cihaz'da gösterilir ("henüz gelmedi" notuyla); yanında gizli pencere notu.
  - İstek: AP'ye HTTP/1.0, `Connection: close`, `Accept-Encoding: identity`; Host / Origin / Referer AP'nin kendi
    adresine (Origin/Referer yalnızca `https://<gelen Host>` ile başlıyorsa; başkası gitmez); X-WFC-*, X-Forwarded-*,
    Forwarded, Via, Proxy-*, koşullu istek başlıkları (If-*), Cookie'de wca/wcc/wfc ve merkezin `_` yaptığı parçalar
    gitmez; HTTP Basic (eski TP-Link) geçer. Yöntem büyük harf, CONNECT/TRACE yok; yol `/` ile başlar. Gövde en çok
    8 MB (bilinen boyda akar; parçalıysa okunur); fazlası 413.
  - Bağlantı: yan oda hedefi aracı (SOCKS5, 127.0.0.1:1085; aracı da yalnızca yan odanın ağına izin verir), misafir
    hedefi ana ad alanından; 5 sn bağlanma, 30 sn boşta. HTTPS AP: rustls (tiny_http'nin 0.20 sürümü,
    `dangerous_configuration`), sertifika doğrulamasız, SNI `ap.wificorrect.local`, yalnızca TLS 1.2+ (eski TLS'li AP'de
    betik). close_notify'sız kapanış gövdenin sonu sayılır. HTTP/1.0'a parçalı cevap veren AP'ler çözülür.
  - Cevap: hop-by-hop başlıklar, Content-Length (yeniden hesaplanır), Cache-Control/Expires/ETag/Last-Modified/Pragma,
    HSTS, Alt-Svc atılır; her cevap `Cache-Control: no-store` (bütün AP'ler aynı kökende). Başlık değerlerinde ve metin
    gövdede (text/*, JS, JSON, XML; türsüzse NUL içermeyen; sıkıştırılmamış; en çok 2 MB, fazlası olduğu gibi akar)
    `http(s)://<hedef>[:port]`, `//<hedef>` ve JSON kaçışlı `http:\/\/<hedef>` → `https://<gelen Host>` / `//<gelen Host>`
    (`<hedef>`: adres ve ona giden bilinen adlar; 192.168.1.10 gibi başka adresler değişmez). Set-Cookie'den Domain
    çıkar (`Domain =x` gibi boşluklu yazım da; final review M4), wca/wcc/wfc adlı çerez yazılmaz. AP aynı adreste başka
    şemaya/porta yönlendirirse (http → https://<hedef>/) hedef dosyası güncellenir (döngü olmaz). Akan (yeniden
    yazılmayan) gövdenin boyu bildirilmez, tiny_http parçalı gönderir (final review M3: AP bildirdiğinden kısa gövdeyle
    kapatırsa Content-Length'li cevap Caddy bağlantısını kaydırıp isteği asılı bırakıyordu); boy yalnızca HEAD'de ve
    yeniden yazılan gövdede.
  - Bilinen sınır: arayüzünü JS ile sabit adresten kuran modeller, WebSocket, 8 MB'tan büyük yükleme → betik.
- **Küçükler:** panel sertifikası boş/bozuksa (PEM ve base64 denetimi; tiny_http boş anahtarda panik ediyordu, Visavis)
  yeniden üretilir: geçici dosyalar (anahtar baştan 600) doğrulanınca yerlerine taşınır; dosyalar PEM görünüp tiny_http
  açamazsa bir kez daha üretilir (port dolu hatasında üretilmez). **Monitörsüz açılış:**
  `/usr/local/lib/wificorrect/efi-baslangic <EFI dizini>` EFI bölümüne CRLF `startup.nsh` + `\EFI\BOOT\startup.nsh` +
  işaret `\EFI\wificorrect\kimlik` yazar (fs0–fs15'te işareti arar; `\EFI\BOOT\BOOTX64.EFI`, yoksa
  `\EFI\debian\shimx64.efi`, `grubx64.efi`); içerik aynıysa dokunmaz; başka bir sistemin `startup.nsh`'si (işareti
  aramayan) ezilmez, uyarı yazılır (final review M5; paket testi denetler). postinst (`mountpoint -q /boot/efi`) ve kurucu
  (`son.sh`, `/target/boot/efi` bağlıysa; seri günlükte `EFI BASLANGIC YAZILDI`, duman testi arar) çağırır. Etkisi gerçek
  cihazda doğrulanmadı. Portal metinleri örneği gerçek ek hesabıyla (`İŞLETMECİ’nin → Göztepe Bilgisayar’ın`).
  `scripts/ap-ac.ps1 -Tunel 10.99.0.x [-Merkez gbserver@192.168.1.109] [-MerkezAnahtar ~/.ssh/gbserver]`: geçici
  ssh_config (`-F`, BOM'suz), merkez üzerinden `ProxyCommand ssh.exe -F … -W %h:%p` (Windows OpenSSH'ta ProxyJump yerine),
  cihaz anahtarı `-Anahtar` ya da `~/.ssh/bocafe`; cihazın ana makine anahtarı `~/.ssh/known_hosts_wificorrect`'te
  `HostKeyAlias wificorrect-tunel-<IP>` ile (`accept-new`; tünel adresi başka cihaza geçip anahtar değişirse bağlanmaz,
  betik `ssh-keygen -R …` satırını yazar; `-E` günlüğünden). `ssh -G` ile Windows OpenSSH 9.5'te ayrıştırması denendi;
  gerçek atlama canlı kabulde.
- **Arama işi kenar durumları** (final review M6): durum dosyası kalmış ama aracısı ölmüş yan oda arama işinde yeni yan
  odadan önce kapanır (`AP_YAN_ODA kapandi … neden=araci_yok`; `ap::kaldir` bunu kilit altında denetler, yaşayan yan odaya
  dokunmaz) — "Bağlan" artık "Zaten bir yan oda açık" hatasında takılı kalmaz. "Bağlan"/"Otomatik bul" başlatma panelde
  kilit altında; `systemd-run` başarısız ama arama çalışıyorsa durumu "başlatılamadı" ile ezilmez.
- **Canlı kabul listesi** (henüz yapılmadı; merkez + DNS `ap.wificorrect.com` sonrası, uzaktan güncellemeyle 0.4.15):
  Göztepe'nin TP-Link'i (10.50.0.2, AP açıkken AP adresleri'ne kaydedilip — MAC sabitlenmeli) → git → giriş sayfası + menüler
  (frameset) çalışıyor mu; AP yönetim parolası yönetimde ve cihaz panelinde aynı mı (ertesi 06:00 eşitlemesinden sonra);
  fabrika adresli bir AP (ZyXEL 192.168.1.1) → Bağlan → yan oda .250 → git; Otomatik bul; HTTPS'e yönlendiren bir model;
  süre dolunca "AP erişimi kapalı" (merkezin sayfası TP-Link frameset'inde çerçevenin içinde görünüyor, bağlantıları bütün
  pencereyi açıyor mu); yan oda kapanınca erişim kapanıyor mu; `ap-ac.ps1 -Tunel` gerçek atlama.
  - **iPhone Safari (≥ 16.4), iCloud Özel Geçiş açık:** "AP arayüzüne git" → AP açılıyor ya da "Farklı internet adresi"
    sayfası (genel "Bağlantı geçersiz" değil); merkez günlüğündeki `AP_GIRIS_RED adres_farkli ip=… gecis_ip=…`
    adreslerini not edin (aynı /24 mü); Özel Geçiş kapalıyken açılıyor mu; zincir `/_ap-giris`'e `same-site` olarak
    geliyor mu (günlükte `AP_GIRIS_RED site='same-origin'` görülmemeli: WebKit yönlendirmede siteyi yeniden hesaplamazsa
    iPhone'da hiç açılmaz).
  - **0.4.16 HTTPS tercihi:** ZyXEL (192.168.1.1) → Bağlan → "Cihaz bulundu (HTTPS)" → git → giriş `/login`'e dönmeden
    açılıyor mu, menüler ve kaydetme çalışıyor mu. Cihazın rustls'i ile ZyXEL'in TLS el sıkışması canlıda denenmedi (bulgu
    curl/OpenSSL ile): olmazsa cihaz 0.4.15 gibi HTTP'ye düşer ("Cihaz bulundu (HTTP)"), o zaman ZyXEL'in TLS sürümü ve
    şifre takımları incelenmeli. Göztepe'nin TP-Link'i (443 yok) yine HTTP'den açılmalı.
  - **Eski kuralla kurulmuş AP'ler** (2026-10-16'ya kadar AP parolası = işletme sahibinin panel parolası): Göztepe'nin
    TP-Link'i ve önceden kurulmuş her AP'nin yönetim parolası yeni **AP yönetim parolası**na çevrilmeli; panel parolası bir
    AP'ye yazıldıysa (betikle ya da ap.wificorrect.com üzerinden) o müşterinin panel parolası da yenilenmeli.

## PPPoE (2026-10-23, 0.4.17)
Dal `pppoe`; tasarım `docs/superpowers/specs/2026-10-23-pppoe-design.md`, plan `docs/superpowers/plans/2026-10-23-pppoe.md`. Sürüm
**0.4.17**. Bazı işletmelerde araya modem girmeyecek: internet alan Ethernet portuna doğrudan ONT'den ya da binanın merkezinden
gelen kablo takılır ve hat adresi DHCP ile değil **PPPoE** ile (operatörün verdiği kullanıcı adı ve parolayla) verir. Kapsam
yalnızca **internet alan portta PPPoE** (işletme ağı, personel Wi-Fi'ı, ONT'nin yerine geçmek ve PPPoE üzerinden IPv6 yok). Kod
hazır, cihazda sahte operatörle denendi; 2026-10-24'te `main`'e birleşti (PR #27) ve `v0.4.17` etiketlendi; canlı cihazlara henüz
kurulmadı ve gerçek bir PPPoE hattında denenmedi (aşağıda "Canlı kabul listesi").
- **Akış:** (1) *Dükkânda hazırlık:* cihaz modemle (DHCP) çalışırken panel → Ayarlar → Portlar → **PPPoE** kutusunda Açık, operatörün
  kullanıcı adı (ör. `5321234567@turk.net`), parola, gerekirse VLAN (Türk Telekom altyapısındaki hatlarda genelde 35) → Uygula → 3 dk
  içinde Onayla (mevcut port değişikliği akışı). Dükkânda PPPoE sunucusu olmadığı için durum "PPPoE sunucusu yanıt vermiyor" görünür,
  cihaz DHCP ile çalışmaya devam eder (kutuda not: "Dükkânda PPPoE bağlanmaz; ONT'ye takılınca bağlanır."). (2) *Müşteride:* internet
  portu ONT'ye / bina kablosuna takılır; PPPoE bağlanır, tünel ve merkez gelir, panelde "PPPoE bağlı (dış IP …)". (3) *Bağlanamazsa:*
  teknisyen müşteride, internet ve SMS olmadan, telefonla misafir Wi-Fi'ına bağlanıp giriş sayfasındaki **Servis** ile düzeltir
  (aşağıda). **DHCP hep açık kalır** (dhcpcd ve WAN DHCP bekçisi bugünkü gibi): hangisi bağlıysa o kullanılır, ikisi de bağlıysa
  PPPoE'nin varsayılan rotası (metrik **100**) DHCP'ninkinin (1000+) önünde kalır; aynı cihaz iki hatta da ayar değiştirmeden çalışır.
- **Ayar** (`/etc/wificorrect/ag.toml`, 600, root; yeni `[pppoe]` bölümü): `acik` (false), `kullanici` (tek alan: modemlerdeki "kullanıcı
  adı @ alan" birlikte), `parola` (yalnızca yazılır), `vlan` (0 = yok, 1-4094), `oncelik` (VLAN 802.1p, 0-7; yalnızca VLAN varken
  anlamlı), `mtu` (1492; 1280-1492, modemlerdeki "MRU"), `kimlik` (`otomatik` | `pap` | `chap`), `ac_adi`, `servis_adi` (boş =
  herhangi biri). Bölümü olmayan eski ag.toml'da PPPoE kapalıdır; 0.4.17'nin yazdığı ag.toml 0.4.16'da da yüklenir (bilinmeyen
  tablo yok sayılır: uzaktan güncellemenin geri dönüşü bozulmaz). Doğrulama (`ag::pppoe_dogrula`; panel, Servis ve `write_files`
  kullanır): açıkken kullanıcı adı ve parola boş olamaz; ikisi en çok 128 bayt, kontrol karakteri, `"` ve `\` içermez (pppd dosyasına
  satır sokulamasın); VLAN, öncelik ve MTU aralıkları; kimlik üç değerden biri; AC ve servis adı boş ya da en çok 64 karakter
  `A-Z a-z 0-9 . _ -`. `write_files` PPPoE açıkken önce doğrular: geçersiz bilgi hiçbir dosya yazılmadan reddedilir (pppd seçenek
  dosyasındaki her satır root'a bir seçenek verir). Dükkândaki modemin "Ethernet WAN" sayfasındaki seçeneklerden bilerek **olmayanlar:**
  bağlantı türü (hep sürekli), varsayılan rota ve NAT (hep açık), IP protokolü (yalnız IPv4), köprü ve port eşleştirme, MAC klon
  (Portlar'daki "internet portu MAC'i" ayarı), 1492'den büyük MRU. `Ag`/`Pppoe` hiçbir yerde `{:?}` ile biçimlenmez (türetilmiş
  `Debug` parolayı içerir).
- **Birim ve dosyalar:** pppd ayrı bir systemd biriminde, `wificorrect-pppoe.service`: `ConditionPathExists=/etc/ppp/peers/wificorrect`,
  `After=network.target`, `ExecStartPre=/usr/local/bin/wificorrect ctl pppoe-hazirla`, `ExecStart=/usr/sbin/pppd call wificorrect
  nodetach`, `ExecStopPost=… ctl pppoe-temizle`, `Restart=always`, `RestartSec=10`, `RestartSteps=5`, `RestartMaxDelaySec=300`,
  `SuccessExitStatus=5` (pppd SIGTERM'de 5 ile çıkar; durdurma birimi "failed" yapmasın). Geçici bir systemd işinde değil, ayrı birimde
  (2026-10-03 dersi: geçici işte başlatılan süreç iş bitince ölür). `ProtectSystem`/`PrivateTmp` bilerek yok: pppd `/etc/resolv.conf` ve
  `/run/ppp`'ye yazar. **Postinst birimi etkinleştirmez**; açık/kapalıyı program yönetir (`pppoe::birim_uygula`: açıkken `systemctl
  enable` + `restart`, kapalıyken `disable --now`) ve `switch` (panel geçişi, geri alma, fabrika dönüşü) ile `pppoe_uygula` (Servis)
  ikisi de çağırır. **Bağlantı dosyası** `/etc/ppp/peers/wificorrect` (600; yalnızca PPPoE açıkken var, kapanınca ve fabrika dönüşünde
  silinir) ag.toml'dan üretilir: `plugin pppoe.so`, `nic-<arayüz>`, `ifname wfcppp`, `user "…"`, `password "…"`, `noauth`, `hide-password`,
  `noipdefault`, `defaultroute`, `defaultroute-metric 100`, `usepeerdns`, `persist`, `maxfail 3`, `holdoff 10`, `lcp-echo-interval 10`,
  `lcp-echo-failure 6`, `mtu N`, `mru N`, `noipv6`; `kimlik=pap` → `refuse-chap/-mschap/-mschap-v2/-eap`, `chap` → `refuse-pap/-eap`;
  AC/servis adı doluysa `pppoe-ac "…"` ve `pppoe-service "…"` (eklenti seçenekleri, `plugin` satırından sonra; `pppoe-ac-name` diye bir
  seçenek **yok**: pppd 2.5.2 eklenti kaynağından, gerçek pppd ikisini de sahte operatörde kabul etti, yanlış değerle bağlanmadı). Sistemin
  ortak `chap-secrets`/`pap-secrets` dosyalarına dokunulmaz. pppd 2.5.2 dosyanın her satırını Debian'ın `/etc/ppp/options`'u (`auth`,
  `lock`, `crtscts`, `modem` …) ile birlikte kabul etti ve her seçenek etkisiyle doğrulandı (ör. `noauth` o `auth`'u ezer). **DNS:**
  operatörün DNS'i `usepeerdns` ile gelir: pppd `/etc/ppp/resolv.conf`'u yazar, Debian'ın kancası (`/etc/ppp/ip-up.d/0000usepeerdns`) onu
  `/etc/resolv.conf`'a geçirir, dnsmasq yukarı akış olarak onu okur. dhcpcd de `/etc/resolv.conf`'u yalnızca kendi kayıtlarından yeniden kurar
  (`20-resolv.conf` kancası): DHCP'siz PPPoE hattında IPv4LL adresi ya da kısa kablo kopması (NOCARRIER) gibi DNS taşımayan bir olay dosyayı
  sunucusuz yazardı; pppd'nin kancası ancak PPPoE yeniden bağlanınca (günler sonra) çalışır, o zamana kadar misafirler ve cihazın kendisi
  (merkez, sağlık raporu) ad çözemezdi. Bu yüzden paket **`/etc/dhcpcd.exit-hook`**'u kurar (dhcpcd-run-hooks bütün kancalardan sonra source
  eder; dhcpcd-base bu dosyayı içermez): `wfcppp` ayaktaysa (IFF_UP) ve `/etc/ppp/resolv.conf`'ta sunucu varsa, `/etc/resolv.conf`'taki
  sunucular farklıysa dosyayı 0000usepeerdns'in yaptığı gibi yeniden yazar (PPP sunucuları `# PPP: ` işaretli bölümde başta, öteki satırlar
  korunur; geçici dosya + `mv`: dnsmasq değişikliği inode'dan da görür). `/etc/resolv.conf` bir bağlantıysa (resolvconf, systemd-resolved)
  dokunmaz; PPPoE kapalı cihazda hiçbir şey yapmaz; exit kullanmaz, hata vermez, değişken bırakmaz. DHCP ve PPPoE ikisi de bağlıyken PPP
  sunucuları geçerli olur (PPPoE rotası önde). Kanca cihazda birim testiyle sınandı (yollar geçici klasörde; gerçek `/etc/resolv.conf`'un
  özeti sınamadan önce ve sonra aynı); dhcpcd'nin gerçek olaylarıyla birlikte yalnızca canlı kabulde: sahte operatör denemesi dhcpcd
  çalıştırmaz (dhcpcd kancaları cihazın ana bilgisayar adına, timesyncd'sine ve resolv.conf'una dokunurdu) ve `usepeerdns`i kaldırır.
- **Yeniden deneme sınırsızdır ama başarısızken yavaşlar:** pppd arka arkaya 3 başarısız denemeden sonra çıkar (`maxfail 3`); birim onu
  10 sn'den başlayıp 5 adımda 5 dk'ya çıkan gecikmeyle yeniden başlatır (`RestartSec`/`RestartSteps`/`RestartMaxDelaySec`; systemd 257'de
  birim sözdizimi `systemd-analyze verify` ile doğrulandı). Düşen bağlantıyı `persist` yeniden kurar (operatör oturumu kapatırsa pppd hemen
  yeniden keşfe başlar; kimlik reddinde `holdoff 10` araya girer). Neden, sahte operatörde ölçüldü: yanlış parolada `holdoff` işler (~13
  sn'de bir deneme), ama yanlış kimlik **yöntemi** seçilince (kimlik=pap ya da chap, operatör öbürünü ister) LCP hiç kalkmaz, `holdoff`
  uygulanmaz ve pppd ~3,3 sn'de bir yeni oturum açar (saatte ~1000 deneme; bazı operatörler hesabı kilitler). `maxfail 3` ile yanlış
  parolada pppd üç redden (3,1 / 16,3 / 29,5 sn) sonra ~31 sn'de, yanlış yöntemde üç redden sonra 9,9 sn'de (kod 16) kendiliğinden çıktı.
  Bedeli: uzun bir operatör kesintisinden sonra yeniden bağlanma 5 dk'ya kadar sürebilir. İlk taslak "sınırsız yeniden deneme" (`maxfail 0`)
  diyordu. **Sayaç:** systemd bu yeniden başlatma sayacını (`NRestarts`) `Restart=always` altında birim kendiliğinden yeniden başladıkça
  sıfırlamaz: araya giren sağlıklı bağlantılar ne kadar uzun sürerse sürsün kendiliğinden yeniden başlatmalar birikir (sayacı yalnızca elle
  başlatma, `systemctl reset-failed` ya da kendiliğinden yeniden başlamayan bir duruş sıfırlar); birikince her kopmada pppd 5 dk bekletilirdi.
  Paket **`/etc/ppp/ip-up.d/wificorrect`**'u kurar (755; `/etc/ppp/ip-up` run-parts ile çalıştırır): `wfcppp` her bağlandığında (`PPP_IFACE`
  ya da ilk argüman) `systemctl reset-failed wificorrect-pppoe.service` sayacı sıfırlar; sonraki kopmada gecikme yeniden 10 sn'den başlar.
  Komut arka planda ve çıktısız çalışır, kanca her durumda 0 ile çıkar: pppd'yi bekletmez, başarısız olmaz (sahte `systemctl` ile birim
  testi: yalnızca `wfcppp`'de, tam olarak bu komutla çağrılır; `ppp0`'da çağrılmaz; yavaş ya da hatalı `systemctl` kancayı bekletmez).
- **VLAN:** `vlan > 0` ise birim başlarken (`pppoe-hazirla`) internet portu açılır, `<wan>.<vlan>` kurulur (`ip link add link <wan> name
  <wan>.<vlan> type vlan id <vlan>`; `oncelik > 0` ise `egress-qos-map 0:<oncelik>`: giden çerçevelere 802.1p önceliği) ve açılır; pppd
  `nic-<wan>.<vlan>` kullanır. Birim durunca (`ExecStopPost`) `pppoe-temizle` internet portunun `<wan>.<rakam>@<wan>` adlı bütün VLAN
  arayüzlerini siler: ayardaki VLAN'a değil adın biçimine bakar (VLAN değişince ya da PPPoE kapanınca eskisi kalmasın), başka portların
  VLAN'larına dokunmaz. Arayüz kurulamazsa birim başlamaz (systemd 10 sn sonra yeniden dener).
- **Güvenlik duvarı, NAT, MSS:** `arayuzler.nft` artık bir küme: `define WAN = { "<internet portu>", "wfcppp" }` (PPPoE kapalıyken de;
  olmayan arayüz adı eşleşmez, zararsız). Böylece `guvenlik.nft`'teki iletim (`oifname $WAN … mark 0x5651`) ve masquerade kuralları ile
  `hiz.nft` sayaçları değişmeden iki arayüzü de kapsar. **Yönetim `wfcppp`'de kapalı:** yönetim kuralları (SSH 22, panel 8443, ping; özel
  kaynak adreslerinden) `iifname $WAN iifname != "wfcppp" ip saddr { 10/8, 172.16/12, 192.168/16 } …` (eski tek adlı WAN tanımıyla da
  geçerli). PPPoE'nin arkasında dükkân ağı yoktur, yalnızca operatör vardır: CGNAT'lı ya da özel adresli PPPoE hatlarında (ve operatörün
  kendi BRAS adresinden) SSH ve panel açılırdı. PPPoE'de internetten panel ve SSH kapalıdır, yönetim tünelden. `$WAN` kullanan bütün
  kurallar testte listelenip sabitlendi (`pppoe::tests::guvenlik_nft_yonetim_pppoe_arayuzunde_kapali`): misafir iletimi, masquerade ve
  `hiz.nft` sayaçları `wfcppp`'yi kapsar, yönetim kapsamaz; `kapi.nft` yalnızca `br-hotspot`'a bakar, programın yazdığı yavaşlatma kuralları
  etkin çıkış arayüzünü adıyla kullanır. Dosyaların iki WAN tanımıyla da `nft -c`'den geçtiğini sahte operatör denemesi (`nft_kurallari`)
  cihaz ad alanında denetler. **MSS düzeltmesi:** `chain forward`'ın **ilk** kuralı `tcp flags syn / syn,rst tcp option maxseg size set rt mtu` (sonraki
  `ct state established,related accept` SYN-ACK'leri kabul edip değerlendirmeyi bitirirdi, MSS'e sıra gelmezdi): SYN ve SYN-ACK'in MSS'i
  çıkış rotasının MTU'suna indirilir, yalnızca küçültür. Cihazda üç ad alanıyla ölçüldü: çıkış MTU 1400 → MSS 1360, 1492 → 1452, 1500'de
  değişmez (yani bugünkü DHCP cihazlarında etkisiz); sahte operatör denemesinde (`mss`) kuralsız 1460, kurallı 1452. 5651'de NAT
  kaydındaki `nat_ip`/`nat_port` artık doğrudan operatörün gördüğü adres (modemin ikinci NAT'ı yok); şema aynı. **Yükseltme nft'yi
  yeniden yüklemez** (postinst dokunmaz): MSS kuralı bir sonraki açılışta, WAN kümesi (`arayuzler.nft` pakette yok, program yazar) ilk
  PPPoE ayarında ya da bir port değişikliğinde devreye girer; PPPoE ayarını uygulayan iki yol da (panel geçişi `switch`, Servis
  `pppoe_uygula`) ortak sonda (`ag::ortak_kuyruk`) `nft -f guvenlik.nft` ve `hiz.nft`'yi yeniden yükler — yoksa yükseltilmiş cihazda
  çekirdekteki eski tek adlı WAN ile `wfcppp` trafiği iletilmez ve NAT'lanmazdı. Yeniden yükleme kapı tablolarına (`inet hotspot`,
  oturum setleri) dokunmaz.
- **Yavaşlatma (HTB):** yükleme yönündeki kök trafiğin gerçekten çıktığı arayüze kurulur: `ag::etkin_cikis` en düşük metrikli varsayılan
  rota `wfcppp` ise `wfcppp`, yalnız DHCP rotası varsa ya da rota yoksa internet alan Ethernet (`ip -o route show default`; arayüzsüz
  `blackhole default` gibi rotalar ve dhcpcd'nin IPv4LL rotası sayılmaz, aşağıda "Giriş sayfası"). Kaydedici (5 sn) ve panelin anında uygulaması (`ctl hiz-uygula`) aynı kuralı kullanır; rota
  okunamazsa (asılı/süresi dolan `ip`) kaydedici o tur uzlaştırmaz. `wfcppp` her yeniden bağlanmada yeniden doğar ve kökü kaybolur: 30 sn'lik
  kök denetimi yeniden kurdurur. **Çıkış değişince** (`enp3s0` ↔ `wfcppp`) önceki planın izindeki (`/run/wificorrect/hiz_uygulanan`) yeni
  plana girmeyen arayüzlerin kökü silinir (`tc qdisc del dev X root`; yoksa eski çıkışta kalan sınıflar sonraki hız değişikliklerinde
  misafiri ikinci kez yavaşlatırdı); kök denetimi yalnızca uygulanmış plandaki arayüzlere bakar (taze doğan `wfcppp`'nin kökü yoktur ama
  henüz planda değildir, izi silmemeli — silseydi eski kök temizlenemezdi). Küçük açık: başka bir olay (nft tablosunun yeniden yüklenmesi,
  port değişikliği, fabrika) izi zaten silmişse eski çıkıştaki kök temizlenmez (yeniden başlatmaya kadar).
- **Tünel MTU'su:** PPPoE açıkken WireGuard yapılandırmasına `MTU = <mtu − 80>` yazılır (varsayılanda 1412; elle bozulmuş ag.toml'daki
  1280-1492 dışı değer aralığa çekilir); kapalıyken satır yoktur ve dosya bugünküyle birebir aynıdır (yükseltme PPPoE'siz cihazlarda
  tüneli yeniden yazdırmaz). `uzak::mtu_esitle`, `switch` ve `pppoe_uygula`'nın ortak sonunda (güvenlik duvarı → hız sayaçları → pppd birimi
  → tünel) dosyadaki MTU istenenden farklıysa dosyayı yazar ve **tüneli yeniden başlatmadan** MTU'yu canlı değiştirir: tünel arayüzü varsa
  `ip link set dev wfc mtu N` (tünel ve uzak yöneticinin bağlantısı kopmaz). `wg-quick@wfc` yeniden başlatılmaz, çünkü Servis'te PPPoE'yi ilk
  kez açan çevrimdışı cihazda DNS yoktur: yeniden başlatma `vpn.wificorrect.com`'u çözemeyip arayüzü kaldırırdı, tünel ancak gözcünün en çok
  10 dk'da bir denemesinde dönerdi. "Otomatik"e dönüşte (PPPoE kapandı, satır yok) yalnızca dosya yazılır: çalışan tünel eski MTU'sunda
  (1412) kalır (zararsız), bir sonraki başlatılışında `wg-quick` kendisi seçer. Arayüz yoksa (tünel kalkmamış) MTU tünelin sonraki
  başlatılışında dosyadan gelir; yapılandırma hiç yoksa (uzak erişim açık ama tünel kurulmamış) `uzak::uygula` tüneli kurar. Sonuç yalnızca
  `UZAK_ERISIM` denetimine yazılır (sonu `(MTU 1412)` / `(MTU otomatik)`); canlı değiştirilemezse `hata=tünel MTU'su değiştirilemedi …`, ağ
  hatası sayılmaz. Uzak erişim kapalıysa dokunulmaz. Açılışta geri alma (ağ henüz kalkmadı) tünel MTU'sunu ayarlamaz; sonraki ayar
  uygulamasında düzelir.
- **Durum** (`pppoe::durum`; panel Portlar, sağlık raporu ve giriş sayfası uyarısı aynı yerden): `kapali` "PPPoE kapalı (modemden DHCP)" ·
  `kablo_yok` "Kablo takılı değil" · `dhcp_yok` "Modemden adres alınamadı (modem kapalı ya da kablo yanlış porta takılı)" · `sunucu_yok`
  "PPPoE sunucusu yanıt vermiyor (kablo doğru porta mı, VLAN gerekli mi?)" · `kimlik_hatasi` "Kullanıcı adı ya da parola reddedildi" ·
  `baglaniyor` "Bağlanıyor…" · `bagli` "PPPoE bağlı (dış IP …)". **Çözüm sırası** (`durum_coz`): internet portunda kablo yoksa her şeyden önce
  `kablo_yok` (pppd kablo çekilince eski adresi ~70 sn daha tutar); PPPoE kapalıysa DHCP adresi varsa `kapali`, yoksa `dhcp_yok`; açıkken
  `wfcppp`'de adres varsa `bagli` (günlükteki eski hatalar geçersiz); yoksa pppd birim günlüğünün son 10 dk'sı (`journalctl -u
  wificorrect-pppoe.service --since -10min -o cat`) sondan taranır ve kararı veren ilk iz belirler: `authentication failed` ya da `refused to
  authenticate` (yanlış kimlik yöntemi) → `kimlik_hatasi`; `timeout waiting for pado` ya da `unable to complete pppoe discovery` →
  `sunucu_yok`; `authentication succeeded` taramayı bitirir (oturum düşmüş, pppd yeniden deniyor); iz yoksa `baglaniyor`. Satırlar küçük
  harfe çevrilip alt dizgiyle eşlenir: systemd altında pppd satırları çift (syslog + stdout) ve birimin başlangıç satırlarıyla gelebilir,
  tekrar sonucu değiştirmez. 169.254/16 (dhcpcd'nin IPv4LL'i), loopback ve 0.0.0.0 adres sayılmaz. Okumalar tembeldir (kapalıyken günlük,
  bağlıyken DHCP adresi okunmaz) ve komut başına 5 sn sınırlıdır. Gerçek pppd 2.5.2 satırları (sahte operatörden, parola ve kullanıcı adı
  içermez): `tests/fixtures/pppoe/{bagli,kimlik_hatasi,kimlik_hatasi_pap,sunucu_yok}.log`. **Gecikmeler:** ayar değişince önceki sonuç
  yenisi gelene kadar gösterilir (`sunucu_yok` ≤ ~35 sn, `kimlik_hatasi` ≤ ~13 sn); birim hiç başlayamazsa (`ExecStartPre` hatası, ör. VLAN
  arayüzü kurulamadı) günlükte pppd satırı olmaz → `baglaniyor` kalır; ölü hat ~70 sn (LCP yankı 10 sn × 6) + 6 sn sonra fark edilir, bu
  sürede `bagli` görünür; bir komut zaman aşımına uğrarsa durum "okunamadı" yerine bir ağ durumu gibi görünebilir (sonraki okumada düzelir).
- **Panel → Ayarlar → Portlar** (sahip ve hizmet sağlayıcı): internet alan portun altında **PPPoE** kutusu: durum satırı (kapalı/bağlı yeşil
  nokta, sorunlar kırmızı, "Bağlanıyor…" düz), dükkân notu ve form (`pppoe::form_html`; giriş sayfasındaki Servis aynı alan adlarını ve
  ayrıştırmayı kullanır): Açık/Kapalı seçimi (`pppoe_acik` = `acik`|`kapali`; `formdan` `1`'i de açık sayar), Kullanıcı adı, Parola, VLAN (boş =
  yok) ve **Gelişmiş** (VLAN önceliği, MTU, Kimlik doğrulama, AC adı, Servis adı; varsayılandan farklı değer varsa bölüm açık gelir). Form
  parola dışında kayıtlı değerlerle dolu gelir: "Kapalı" gönderince kullanıcı adı ve ileri alanlar silinmez. Açık/Kapalı seçimi de kayıtlı değeri
  gösterir: PPPoE hiç ayarlanmamış cihazda **Kapalı** gelir, açmak için Açık seçilmelidir (Servis'in formunda seçim Açık gelir, aşağıda).
  **Parola yalnızca yazılır:** hiçbir
  sayfaya, denetime ya da mesaja geri yazılmaz; boş gönderilirse kayıtlı parola kalır, yani **kayıtlı bir PPPoE parolası panelden silinemez**
  (yalnızca yenisiyle değiştirilir; PPPoE'yi kapatınca dosyada kalır, fabrika dönüşü siler). Kullanıcı adı, AC ve servis adı baştan/sondan
  kırpılır, parola olduğu gibi alınır. PPPoE alanları yalnızca form PPPoE seçimini (`pppoe_acik`) taşıyorsa okunur (Servis'teki gibi):
  PPPoE alanı olmayan bir `/portlar` gönderimi (betik, ileride başka bir form) kayıtlı PPPoE ayarını varsayılana döndürmez, olduğu gibi
  bırakır. Kaydetme **mevcut port değişikliği akışından** geçer (hazırlanan `ag-yeni.toml` → geri alma
  zamanlayıcısı → `ag::gecis` → `switch` → 3 dk içinde "Çalışıyor, onayla"; onaylanmazsa ya da açılışta onay yoksa eski ayarla PPPoE da
  geri gelir); `ag::gecis` boş parolayı birleştirmez, panel kayıtlı parolayı hazırlanan dosyaya kendisi yazar. Geçersiz ayar doğrulama
  mesajıyla `/portlar`'a döner, hiçbir şey hazırlanmaz. Bir PPPoE değişikliği de ağı yeniden kurar (misafirler birkaç sn düşer, açık AP
  yan odası kapanır). Panel `/portlar` ve `/ayarlar` açılışında 1-2 kısa komut çalıştırır (`ip`; PPPoE açık ve bağlı değilse `journalctl`), her biri
  en çok 5 sn.
- **Giriş sayfası: internet yokken uyarı ve Servis** (`src/portal.rs`, `src/sablon/servis.html`): koşul cihazın **kullanılabilir varsayılan
  rotasının olmaması** (`ip -o route show default`'ta ne DHCP ne PPPoE rotası), PPPoE açık da kapalı da olsa (bütün cihazlarda; modem
  arızasında da). **dhcpcd'nin IPv4LL rotası sayılmaz:** DHCP sunucusu olmayan hatta (ONT, bina kablosu; PPPoE bağlanamamış ya da modemi
  arızalı cihaz) dhcpcd internet portuna 169.254/16 adresi verir ve DHCP'den gelmiş varsayılan rota yoksa o adresle bir varsayılan rota da
  kurar: `default dev enp3s0 proto dhcp scope link src 169.254.x.y metric 1001003` (arayüz metriği + 1000000). Bu rota internete çıkmaz;
  sayılsaydı uyarı ve Servis tam da gerektikleri arızada hiç görünmezdi. Kural kaynak adrese (`src` 169.254/16) bakar, ağ geçidi yokluğuna
  değil (PPPoE rotası `default dev wfcppp scope link metric 100` da ağ geçitsizdir ve sayılır). Sonuç 10 sn önbellekte, tek yoklama; `ip`
  çalışmazsa, süresi dolarsa **ya da hata koduyla biterse** (`ortak::capture_timeout_basarili`: boş çıktı + hata kodu "rota yok" sayılmaz)
  internet var sayılır (emin olunamayınca Servis açılmaz); rota
  yokken internet alan portun durumu `pppoe::durum` ile okunur (internet varken `journalctl` çalışmaz). Misafire açık sayfada **internet
  varken hiçbir ayar yüzeyi yoktur:** `/servis`, `/servis?izle=1` ve POST `/servis` giriş sayfasına yönlenir, form işlenmez ve sayılmaz.
  - **Uyarı:** sayfanın altında "Bu cihazın internet bağlantısı şu an yok" + neden (yukarıdaki durum metni) + **Servis** bağlantısı; **her
    sayfa türünde** (giriş formu, hatalı yeniden çizimi dahil; kod sayfası; misafir kodu; oturumu açık telefonun "Bağlandınız" sayfası; lisans
    kapalı ve MAC tanınmadı hata sayfaları), yalnızca Servis'in kendisinde yok. İngilizce sayfada başlık ve bağlantı İngilizce ("This device has
    no internet connection right now" / "Service"), neden Türkçe. "Bağlandınız" sayfasının 1 sn'lik yönlendirmesi internet yokken atlanır (hedef
    yüklenemez, uyarı kaybolmasın). **Teknisyen adresi `10.50.0.1:8080`** ya da doğrudan **`10.50.0.1:8080/servis`:** internet yokken DNS hiçbir
    adı çözemez ve telefon "ağda oturum açın" penceresini açmaz, bu yüzden adres elle ve IP ile yazılır (internet yokken DNS'in her adı yanıtlaması
    reddedildi: her kesintide bütün misafirlerin davranışını ve 5651 DNS kaydını değiştirirdi).
  - **Servis sayfası** (yalnızca Türkçe, JS yok): durum satırı + tek form: **Servis parolası** (= AP yönetim parolası) + PPPoE alanları
    (`pppoe::servis_form_html`) → **Kaydet ve bağlan**. Parola her gönderimde birlikte yazılır (oturum ve çerez yok). **Kayıtlı kullanıcı adı
    hiçbir sayfada gösterilmez** (çoğu kez abonenin telefon numarasıdır, sayfa parola doğrulanmadan misafire açıktır): alan boş gelir, altında
    "Boş bırakılırsa kayıtlı kullanıcı adı aynı kalır."; boş gönderilirse parola doğrulandıktan sonra kayıtlı ad kullanılır (`kullanici_degisti=0`),
    hata sayfalarında forma yalnızca o istekte yazılan ad döner. VLAN ve ileri alanlar kayıtlı değerlerle dolu gelir; PPPoE parolası alanı hep
    boştur (boş = kayıtlı parola kalır, ipucu "Kayıtlı parola aynı kalır"). **PPPoE Açık/Kapalı seçimi, kayıtlı ayar kapalı olsa da Açık gelir**
    (`servis_ilk_alanlar`): Servis'e PPPoE'yi açmak için gelinir, kayıtlı ayar ise çoğu kez kapalıdır (PPPoE hiç ayarlanmamış cihazda varsayılan);
    seçim kayıtlı değerle gelseydi formu olduğu gibi gönderen teknisyen bilgileri PPPoE kapalıyken kaydeder ve internet gelmezdi. Kapatmak için
    Kapalı elle seçilir; panelin Portlar formu bundan etkilenmez (orada kayıtlı değer görünür). Reddedilen
    bir kayıttan sonra (yanlış ya da boş Servis parolası, kilit, doğrulama hatası) form gönderilen seçimi korur (Kapalı seçilmişse Kapalı kalır);
    hiçbir şey gönderilmediyse (eksik form) kayıtlı ayardan, yani ilk çizim gibi Açık kurulur.
  - **Servis parolası** `merkez.json` → `ap_parola`'dır; merkeze sorulmaz, cihazda SHA-256 özetleri üzerinden sabit sürede karşılaştırılır.
    Yoksa ya da 8 karakterden kısaysa (cihaz bağlanmamış ya da parola eski merkez yüzünden hiç gelmemiş): "Servis parolası tanımlı değil (AP
    yönetim parolası bu cihaza henüz gelmedi); Servis kullanılamıyor." ve form yok.
  - **Deneme sınırı** (bellekte; portal yeniden başlayınca ya da cihaz açılınca sıfırlanır): MAC başına (MAC bilinmiyorsa IP başına) 15 dk'da
    5 yanlış → 15 dk kilit; cihaz genelinde 60 dk'da 20 yanlış → 60 dk kilit (MAC değiştirerek sınırsız deneme olmasın; parola 12 rastgele
    karakter). Kilit sürerken parolaya bakılmaz ("Çok fazla yanlış deneme; N dakika sonra yeniden deneyin."). Yalnızca yanlış Servis parolası
    sayılır: boş parola, eksik form ve doğru paroladan sonraki doğrulama hataları sayılmaz; doğru parola o anahtarın yanlışlarını siler (cihaz
    geneli sayaç kalır). Kontrol, karşılaştırma ve sayma tek kilit altındadır (aynı anda gelen denemeler sınırı aşamaz).
  - **Uygulama:** doğrulanan ayar `/var/lib/wificorrect/pppoe-servis.toml` olarak (600; ag.toml'daki adlarla `[pppoe]` tablosu; **boş PPPoE
    parolası dosyaya boş yazılır**, kayıtlı parola kopyalanmaz) hazırlanır ve `systemd-run --collect --unit wfc-pppoe-servis
    /usr/local/bin/wificorrect ctl pppoe-servis <dosya>` ile ayrı işte uygulanır (portal `ProtectSystem=full`, `/etc`'ye yazamaz; iş
    beklenmez). `ctl pppoe-servis` dosyayı okur okumaz **siler** (sonuç ne olursa olsun; ayrıştırma hatasının metni gösterilmez, parola
    taşıyabilir) ve `ag::pppoe_uygula`'yı çağırır: onay bekleyen bir port değişikliği (`ag-bekliyor`) varsa, ag.toml yoksa ya da var ama
    ayrıştırılamıyorsa (bozuk; `ag::load_kesin`: `ag::load` gibi varsayılana düşmez, yoksa varsayılan portlar — enp3s0 alır, enp1s0 verir;
    portları ters kablolu kutuda, ör. Visavis, yanlış — yazılıp güvenlik duvarı yanlış WAN ile yüklenirdi; hata metni ayrıştırıcının
    ayrıntısını taşımaz) reddeder; boş
    parolayı kayıtlıyla birleştirir; doğrular; ag.toml'u kaydeder, dosyaları yazar, `nft -f guvenlik.nft` + `hiz.nft`, pppd birimi, tünel MTU'su.
    Ağı yeniden kurmaz (`switch` değil) ve **geri alma zamanlayıcısı kurmaz** (cihaz zaten çevrimdışı; geri almak teknisyenin düzelttiği bilgiyi
    yanlış eskisine döndürürdü). Kayıttan sonra sayfa `/servis?izle=1` durum sayfasına geçer (5 sn'de bir meta yenileme, "Ayarları değiştir"
    bağlantısı); internet gelince giriş sayfasına döner. **Kayıt mesajı kaydedilen seçimi söyler** (kullanıcı adı ve parola hiçbirinde yok):
    Açık → "Ayar kaydedildi; PPPoE yeniden bağlanıyor. Bu sayfa birkaç saniyede bir yenilenip durumu gösterir (güncel durumun gelmesi bir
    dakikayı bulabilir); internet gelince giriş sayfası açılır."; Kapalı → "Ayar kaydedildi; PPPoE kapalı. Cihaz internet portundan modemin adres
    vermesini bekliyor (PPPoE hattı için Ayarları değiştir bağlantısından PPPoE seçimini Açık yapın). Bu sayfa birkaç saniyede bir yenilenip
    durumu gösterir; internet gelince giriş sayfası açılır." (Kapalıda "bir dakikayı bulabilir" yok: pppd durur, durum DHCP adresinden hemen
    okunur.) Önceki iş sürerken yeniden kaydetmek "iş başlatılamadı; birkaç saniye sonra yeniden deneyin" der; ag.toml yoksa, bozuksa ("Ağ ayar
    dosyası okunamadı (bozuk); PPPoE ayarı uygulanamaz.") ya da port değişikliği onay bekliyorsa sayfa bunu hemen söyler (form geri gelir).
    Formun altındaki not seçimden bağımsızdır: "Kaydedince ayar uygulanır (PPPoE Açık ise yeniden bağlanır, Kapalı ise durur) …".
- **AP yönetim parolası cihaza 5 dk içinde ulaşır (≥ 0.4.17):** Servis'in parolası yönetimdeki "Yenile"yle değişebilir, o yüzden merkez onu
  5 dk'lık sağlık cevabında da gönderir (`/api/saglik` cevabında `ap_parola`; yalnızca bağlı cihaza ve müşterinin parolası doluysa; `/api/eslesme`
  gibi lisans/askı durumuna bakmaz) ve cihaz (`ctl saglik`) değiştiyse `merkez.json`'a yazar. Cihaz tarafı değeri `merkez::ap_parola_oku` kuralıyla
  doğrular (8-64 yazdırılabilir ASCII, boşluksuz); geçersiz değer ya da alanı olmayan eski merkez → dokunulmaz; `merkez.json` yoksa (bağlı
  değil, fabrika dönüşü) dosya oluşturulmaz; aynıysa yazılmaz. Böylece "Yenile"den 5 dk sonra cihaz yeni parolayı bilir ve Servis onu ister.
  ≤ 0.4.16 cihazlar ile çevrimdışı cihazlar eski parolayı, yeniden bağlanana (ya da günlük 06:00 eşitlemesine) kadar kullanır; yönetimde "Yenile"
  yanındaki not buna göre güncellendi.
- **Sağlık raporu ve merkez:** cihaz 12 anahtarlı raporda (`/api/saglik`, 5 dk) iki yeni alan yollar: `wan_tur` (`dhcp` | `pppoe`; ag.toml'da PPPoE
  açık mı, bağlı olması gerekmez) ve `pppoe` (yukarıdaki durum kodu; DHCP'li cihaz da `kapali`/`dhcp_yok`/`kablo_yok` gönderir). `wan` artık
  "internet portunun IPv4 adresi var" demektir: Ethernet'te (DHCP) **ya da** yalnızca PPPoE açıkken `wfcppp`'de (PPPoE'de Ethernet'te adres olmaz;
  PPPoE bağlanamasa da modem hâlâ DHCP veriyorsa `wan=true`, `pppoe` yine hatayı gösterir; kapalıyken `wfcppp` kalıntısı sayılmaz). Merkez
  (`api.py`) iki alanı bağımsız ve isteğe bağlı doğrular: **bilinmeyen ya da bozuk değer raporu reddetmez, yalnızca o alanı atar** (ileride
  eklenecek bir durum kodu eski merkezde "Rapor gelmiyor" uyarısı/SMS'i doğurmasın; öteki alanlar yine sıkı doğrulanır, bilinmeyen anahtarlar
  zaten atılır). Yönetim → müşteri sayfası sağlık kartında "WAN"dan sonra, "SMS"ten önce **"İnternet"** satırı, yalnızca `pppoe` koduna bakar:
  Modem (DHCP) · kablo yok · modemden adres alınamadı · PPPoE: sunucu yanıt vermiyor · PPPoE: parola reddedildi · PPPoE: bağlanıyor · PPPoE bağlı;
  sorunlular (kablo yok, adres yok, sunucu yok, parola reddedildi) kırmızı; kod yoksa "—" (eski cihaz ya da atılmış değer). **Yeni sağlık kuralı
  yok:** PPPoE bağlanamayan cihaz zaten "ulaşılamıyor" olur ve mevcut uyarı SMS'i gider.
- **Denetim olayları:** `PANEL_PORT` satırı `ag::describe` metnine (`… , PPPoE açık (VLAN 35)`) ek olarak ` pppoe=acik|kapali vlan=N
  kullanici_degisti=0|1 parola_degisti=0|1` taşır (kullanıcı adı ve parola yazılmaz); Servis: `SERVIS_GIRIS` ek `sonuc=hatali|kilit` (MAC, IP;
  `kilit` her kilit dönemi için bir kez), `SERVIS_PPPOE` ek `pppoe=acik|kapali vlan=N kullanici_degisti=0|1 parola_degisti=0|1` (MAC, IP; iş
  başlatılamazsa sonuna ` hata=yazilamadi|baslatilamadi`), uygulayan komut `PPPOE_SERVIS_UYGULANDI` ek `pppoe=acik|kapali vlan=N[ hata=…]`;
  tünel MTU'su `UZAK_ERISIM`; AP parolası `AP_PAROLA_MERKEZ` ek `kaynak=saglik` (değer yazılmaz), yazılamazsa `MERKEZ_EK_HATA` (`ap_parola: …`). Mevcut
  `AG_UYGULANDI` / `AG_ONAYLANDI` / `AG_GERI_ALINDI` aynen. Hiçbir olay, sayfa ya da hata metni PPPoE kullanıcı adını, PPPoE parolasını ya da Servis
  parolasını taşımaz (testlerle sabitlendi).
- **Sahte operatör denemesi** (`scripts/pppoe-deneme.sh`, cihazda root; `ppp` ve **yalnızca bu deneme için** `pppoe` Debian paketleri kurulu
  olmalı, `pppoe` Depends'e girmez): `scp scripts/pppoe-deneme.sh wificorrect:/root/ && ssh wificorrect bash /root/pppoe-deneme.sh` (~4,5 dk;
  kurulu sürümde `ctl pppoe-peers` yoksa başa `WFC=/root/rza/target/debug/wificorrect`; güvenlik duvarı kuralları varsayılan olarak kurulu sürümden
  (`/etc/wificorrect`), dalınkiler için `KURALLAR=/root/rza/deploy/debian/etc/wificorrect` (`scripts/test.sh` gönderir); belirli senaryolar
  `bash /root/pppoe-deneme.sh bagli vlan`;
  yarıda kalanın artıkları `--temizle`; çıkış kodu: hepsi PASS 0, en az bir FAIL 1, kurulum hatası 2; günlükler `/root/pppoe-deneme/`). Cihazın kendi
  içinde üç ağ ad alanı (`wfc-isp` operatör, `wfc-cpe` cihaz, `wfc-lan` MSS için misafir) ve ad alanının içinde doğan sanal kablolar: operatör
  tarafı `pppoe-server`, cihaz tarafı **gerçek `pppd call wificorrect nodetach`** (bağlantı dosyasını `ctl pppoe-peers <ag.toml> <arayüz>` üretir). `/etc/ppp`,
  `/etc/resolv.conf` ve `/run` iki tarafta da yalıtılır (`/etc/netns/…`, ad alanına özel tmpfs): sistemin `/etc/ppp`'si ve kancaları kullanılmaz, kök
  `/run`'a hiçbir şey yazılmaz; yalnızca kendi süreçleri durdurulur (`pkill` yok; 2026-10-03 dersi: netns süreçleri ayırmaz, cihazın kendi `dhcpcd`'si de
  ölürdü); başta ve sonda kök ad alanı (ip addr/route/rule/link, nft kuralları, `resolv.conf`, `/etc/ppp`, `ip_forward`) karşılaştırılır. **19 kontrol:**
  `bagli` (IP 15 sn içinde, rota `metric 100`, MTU 1492), `kimlik_hatasi` (CHAP) ve `kimlik_hatasi_pap`, `sunucu_yok`, `vlan_gerekli` (olumsuz: VLAN'sız
  istemci VLAN'lı operatöre bağlanmaz) ve `vlan` (`egress-qos-map`), `ac_yanlis` + `servis_yanlis` (olumsuz) ve `ac_servis`, `kimlik_pap` / `kimlik_chap` ve
  `kimlik_chap_red` / `kimlik_pap_red` (`refuse-*` satırları yanlış yöntemi gerçekten engelliyor: IP yok, günlükte operatörün reddi `peer refused to
  authenticate`, pppd ya hâlâ deniyor ya da `maxfail 3` ile kendiliğinden ve seçenek/ölümcül hata kodu 1-4 dışında çıktı), `yeniden_baglanma` (operatör durup
  gelince), `hat_kopmasi` (iki yönde paket düşürme; cihazın kendi LCP yankı algılaması), `mss` (kural `KURALLAR`'daki guvenlik.nft'ten), `nft_kurallari`
  (`guvenlik.nft` ve `hiz.nft` eski tek adlı ve yeni küme WAN tanımıyla `nft -c`'den geçer, bilerek bozulmuş kopya reddedilir; cihaz ad alanında, kök ad
  alanının kurallarına dokunulmaz), `parola_gizli` (hiçbir çıktıda parola yok), `kok_ad_alani`. **Son ağaçta (final review düzeltmelerinden sonra, `maxfail 3`'lü
  bağlantı dosyası, `WFC=/root/rza/target/debug/wificorrect KURALLAR=/root/rza/deploy/debian/etc/wificorrect`) canlı Göztepe'de 2026-10-24: 19 PASS, 0 FAIL
  (250,1 sn), çıkış kodu 0; kök ad alanı (adres, rota, nft, `resolv.conf`) denemeden önce ve sonra aynı (`kok-fark.txt` boş), sonra ad alanı, `/etc/netns`
  artığı ve pppd süreci yok, servisler ve tünel çalışıyor.** Yanlış yöntemde (`kimlik_chap_red`, `kimlik_pap_red`) pppd 3 redden sonra 9,9 sn'de kod 16 ile
  kendiliğinden çıktı. (Önceki 18/18 turları, 2026-10-23, `maxfail 0`'lı taslak bağlantı dosyasıyla ve `nft_kurallari` olmadan yapılmıştı.) Ölçülen
  süreler: oturum 3,3 sn; yanlış parolada yeni deneme 13 sn sonra (`holdoff` işliyor); yanlış yöntemde ~3,3 sn'de bir (`holdoff` yok); operatör yokken bir
  keşif turu 35 sn; operatör kapanıp gelince yeniden bağlanma ~5 sn; hat kopması ~70 sn (+6 sn) sonra fark edilir, hat gelince ~9 sn. Hat kopması iki
  yönde paket düşürülerek taklit edildi (arayüzler ayakta kalır); gerçek kablo çekme denenmedi (çekirdeğin PPPoE sürücüsü taşıyıcı değişiminde oturumu
  bırakmaz diye değerlendirildi, kaynak okumasına dayanır) → canlı kabulde.
- **Testler (2026-10-24, sürüm 0.4.17):** cihazda `bash scripts/test.sh wificorrect` → final review düzeltmelerinden sonra 453 birim testi geçti
  (uyarısız); merkez `sh scripts/sunucu/merkez/tests/hepsi.sh py` → 22/22 dosya `TUM TESTLER GECTI` (merkez son düzeltmelerde değişmedi). Cihaz
  birim testleri pppd gerektirmez (sahte `/sys`, sahte komut çalıştırıcı, gerçek pppd günlük örnekleri); testler `ag::Yollar`'a geçici dizin verir
  (canlı `/etc/ppp` ve `/etc/wireguard` yolları yerine); iki kancanın testleri `sh` ile geçici yollarda çalışır (`scripts/test.sh` kancaları,
  `guvenlik.nft` ve `hiz.nft`'yi de cihaza gönderir).
- **Paket ve kurulum sırası:** `paket/control.in` Depends'e **`ppp`** (Debian 13: 2.5.2, PPPoE eklentisi içinde); uzaktan güncelleme `apt-get install
  ./paket.deb` ile bağımlılığı Debian deposundan çeker (cihazın internet erişimi gerekir; iki canlı cihazda `ppp` Debian deposundan kurulabildiği
  doğrulandı). Birim pakette ama postinst etkinleştirmez. Paket iki kanca da taşır: `/etc/dhcpcd.exit-hook` (644; dhcpcd source eder) ve
  `/etc/ppp/ip-up.d/wificorrect` (755; `scripts/paket.sh` iki modu da açıkça verir, git'te de +x); ikisi de başka bir paketin dosyası değil
  (`dpkg -S` boş). `scripts/paket-test.sh` denetler: birim pakette, `multi-user.target.wants`'ta bağlantı yok, `/usr/sbin/pppd` kurulu; iki kanca
  pakette ve modları doğru, `run-parts --test` ip-up kancasını görür, kanca `wfcppp`/`ppp0` ile (systemd'siz kapsayıcıda) 0 döner, dhcpcd-run-hooks
  exit-hook'u hâlâ okuyor, exit-hook `sh -eu` altında source edilince durmaz (yalnızca CI'da çalışır, bu makinede çalıştırılamadı; paketin
  içeriği ve modları cihazda geçici bir `scripts/paket.sh` derlemesiyle `dpkg-deb -c` ile görüldü). PPPoE kapalı cihazlarda yükseltmeden sonra
  görünen fark: yeni (devre dışı) birim ve iki kanca (PPPoE kapalıyken hiçbir şey yapmazlar), internet yokken giriş sayfasındaki uyarı ve Servis,
  sağlık raporunda iki yeni alan ve sağlık cevabıyla gelen AP yönetim parolası;
  güvenlik duvarı değişiklikleri yukarıdaki gibi sonra devreye girer. **Merkez ile cihaz güncelleme sırası önemsizdir:** eski merkez yeni cihazın
  raporundaki bilinmeyen anahtarları atar (raporu reddetmez), eski cihaz cevaptaki `ap_parola`'yı yok sayar; yeni merkez + eski cihazda "İnternet"
  satırı "—" ve Servis yoktur. Servis'in AP parolasını 5 dk'da alması için ikisi de 0.4.17 olmalıdır. Merkez: `scripts/sunucu/merkez-guncelle.sh`
  (kullanıcı, sudo). Cihazlar: `v0.4.17` etiketi → release → Yönetim → Sürümler → kanal hedefi (docs/DEVAM.md "Dallar ve durum").
  **Uyarı — eski sürüme dönüş yok:** PPPoE açık bir cihaza (sağlık kartında "İnternet: PPPoE …") hiçbir zaman 0.4.17'den eski bir sürüm
  hedeflenmez (kanal hedefi ya da "Şimdi güncelle"): eski paket `wificorrect-pppoe.service`'i kaldırır (etkinleştirme bağlantısı boşa düşer), pppd
  bir sonraki açılışa kadar çalışır, sonra PPPoE de Servis de yoktur; yalnızca PPPoE'li hatta cihaz internetsiz kalır ve yerinde düzeltilemez.
  Merkez bunu engellemez (karar: yalnızca belge).
- **Bilinen sınırlar ve riskler:**
  - *Operatörün hesabı kilitlemesi:* yanlış kullanıcı adı/parola/kimlik yöntemiyle cihaz yoklamayı sürdürür (yukarıdaki geri çekilmeyle: 3 denemelik
    turlar, tur arası bekleme 10 sn'den 5 dk'ya); bazı operatörler yine de kilitleyebilir, teknisyen yanlış bilgiyi hemen düzeltmelidir. systemd
    `RestartSteps` sayacını `Restart=always` altında kendiliğinden yeniden başlatmalar arasında sıfırlamaz (sağlıklı bağlantılar araya girse de
    birikir); bu yüzden ip-up kancası her başarılı bağlantıdan sonra `systemctl reset-failed` ile sıfırlar ("Yeniden deneme" yukarıda). Kanca
    çalışmazsa (ör. elle silinmişse) en kötü durum: beşinci kendiliğinden yeniden başlatmadan sonra her kesintide 5 dk'ya kadar gecikme.
  - *VLAN önceliği (PCP) yavaşlatılmış misafirlerin yükleme trafiğinde 0 olur:* `egress-qos-map 0:N` yalnızca çekirdek önceliği tam 0 olan
    paketleri eşler; yavaşlatma sınıfları misafir paketine `1:1x` önceliğini verir ve bu öncelik PPPoE'den VLAN'a kadar taşınır, o paketler
    VLAN'dan PCP 0 ile çıkar. Yalnızca veri çerçevelerinde PCP isteyen operatörlerde önemlidir; gerekirse sınıf öncelikleri de `egress-qos-map`'e
    eklenir.
  - *Servis parolası şifresiz ağdan gider:* giriş sayfası HTTP, Wi-Fi şifresiz; yakındaki biri o anki AP yönetim parolasını yakalayabilir (bu parola o
    müşterinin AP'sini de korur). Etki o müşteriyle sınırlı; Servis yalnızca cihaz çevrimdışıyken vardır. Servis kullanıldıktan sonra yönetimde AP
    yönetim parolasını **Yenile** deyip AP'ye de yazın. (Ortak admin parolası bilerek Servis'te kullanılmaz.)
  - *Cihaz geneli kilit* kesinti sırasında bir misafirce tetiklenebilir (20 yanlış deneme → 60 dk); portal yeniden başlayınca/cihaz açılınca kalkar.
  - *Operatörün tek oturum sınırı:* bazı operatörler aynı hesapla tek PPPoE oturumuna izin verir; müşterinin eski modemi de bağlıysa biri düşer
    (kurulumda eski modem sökülür). VLAN bilinmezse cihaz `sunucu_yok`'ta kalır (operatörden sorulup Servis'ten girilir).
  - *Bayat/yanlış görünüm:* durum sayfası ayar sonrası ~35 sn geriden gelebilir; ölü hat ~76 sn boyunca `bagli` görünür; sağlık raporundaki `wan`
    169.254/16 adresini de sayar (ONT köprü kipinde DHCP vermezse dhcpcd Ethernet'e IPv4LL atar: `wan=true` ama `pppoe=sunucu_yok`; böyle bir cihaz
    zaten rapor gönderemez — yalnızca görünüm). Durum (`pppoe::durum`, PPP adresi) ve internet koşulu IPv4LL'i saymaz: dhcpcd'nin o adresle
    kurduğu varsayılan rota (`… scope link src 169.254.x.y metric 1001003`) internet sayılmaz, uyarı ve Servis görünür ("Giriş sayfası" yukarıda).
  - *Servis kayıt yarışı:* kayıttan hemen sonra (iş dosyayı okumadan) ikinci kayıt aynı hazırlık dosyasının üzerine yazabilir ya da silebilir; sonucu durum
    sayfasında görüp gerekirse yeniden kaydedin.
  - *Küçükler:* USB Ethernet gibi uzun adlı (`enx…`) internet portlarında `<port>.<vlan>` adı Linux'un 15 karakter sınırını aşabilir (J1900'ün
    `enp1s0`/`enp3s0`'unda sorun yok); `pppoe-hazirla` tekrar çalıştırılabilir değildir (VLAN arayüzü zaten varsa kurulum başarısız olur; her durdurma
    `pppoe-temizle` çalıştırdığı için normal akışta olmaz); internet portu başka porta geçerken eski portun `<port>.<vlan>` arayüzü yeniden başlatmaya
    kadar artık kalabilir; PPPoE'li müşteride de ekransız açılış için sahte ekran fişi ve BIOS "Restore on AC Power Loss" gerekir (ayrı konu, docs/DEVAM.md).
- **Canlı kabul listesi** (henüz yapılmadı; ilk PPPoE'li müşteride, **yanında yedek modemle**; cihazlar 0.4.17, merkez güncel):
  - *Dükkânda (DHCP):* panelde PPPoE kutusu görünüyor mu; uydurma bilgiyle Açık → Uygula → Onayla: durum "PPPoE sunucusu yanıt vermiyor (…)", DHCP
    internet ve tünel sürüyor (kopmuyor), kapatınca `/etc/ppp/peers/wificorrect` siliniyor, VLAN arayüzü gidiyor, tünel yapılandırmasındaki `MTU` satırı kalkıyor.
    Cihazın AP yönetim parolası geldi mi (panel → AP kurulumu → Takılan cihaz; yönetimdekiyle aynı olmalı: cihaz çevrimdışı kalınca Servis'in tek anahtarı).
  - *ONT'ye takınca:* PPPoE bağlanıyor mu; `ip -o route show default` → `default dev wfcppp scope link metric 100` (yanında dhcpcd'nin IPv4LL satırı
    `… src 169.254.… metric 1001003` de olabilir; çıktıyı not edin); panelde "PPPoE bağlı (dış IP …)"; Yönetim →
    müşteri → sağlık kartı "İnternet: PPPoE bağlı"; tünel kalkıyor (`ip link show wfc` MTU 1412, el sıkışma) ve merkez bağlantısı geliyor.
    `nft list chain inet wfc input`'ta yönetim satırlarında `iifname != "wfcppp"`; `/etc/resolv.conf`'ta operatörün sunucuları (`# PPP: ` bölümü).
  - **Bir kez `journalctl -u wificorrect-pppoe.service -o cat` bakın** (pppd'nin systemd altındaki gerçek çıktısı sahte operatörde gözlenmedi: satırlar çift mi
    geliyor, hangi başlangıç satırları var; durum çözümleyici alt dizgiyle eşler, ama kimlik/PADO/`local  IP address` satırları gerçek operatörde beklenen
    biçimde mi) ve panel durumunun doğru olup olmadığını karşılaştırın.
  - *Misafir:* SMS ile giriş (gerçek SMS yalnızca kullanıcının onayıyla), büyük sayfalar ve HTTPS siteler açılıyor mu (MSS), misafir DNS çözümü ve
    `/etc/resolv.conf` (operatör DNS'i; DHCP ile birlikteyken son yazan), 5651 `trafik.csv`'de `nat_ip` operatörün verdiği dış IP.
  - *Yavaşlatma:* bir cihazı yavaşlat (ör. 2 Mb/sn) → `tc -s class show dev wfcppp` sınıf sayaçları artıyor mu; **PPPoE'yi düşürüp/kabloyu çekip geri gelince
    (çıkış arayüzü değişti)** yine `tc -s class show dev wfcppp` (sınıflar yeniden kuruldu mu, sayaçlar işliyor mu) ve eski çıkış `tc qdisc show dev enp3s0 root`
    (HTB kalmamış olmalı).
  - *Kopma:* kabloyu çekip takın (ya da ONT'yi kısa süre kapatın): kablo çekilince `kablo_yok` hemen görünür; kablo gelince kendiliğinden yeniden
    bağlanma; kablo takılıyken hat ölürse (ONT kapanınca) ~70 sn'de fark edilmesi ve durum satırı.
  - **DNS (dhcpcd kancası):** kabloyu 5 sn çekip takınca `/etc/resolv.conf`'ta operatör DNS'i kalıyor mu (`cat /etc/resolv.conf`: `# PPP: ` bölümü ve
    sunucular; dhcpcd'nin NOCARRIER/CARRIER olayından sonra) ve misafir ile cihaz ad çözebiliyor mu (`getent hosts api.wificorrect.com`); aynısını cihazı
    yeniden başlatınca (dhcpcd'nin IPv4LL olayı PPPoE'den önce ya da sonra gelir) da bakın. `journalctl -u ifup@enp3s0` içinde dhcpcd kanca hatası yok.
  - *Geri çekilme:* yanlış parolayla bırakıp `journalctl` zaman damgalarından yeniden deneme aralıklarının 10 sn'den 5 dk'ya çıktığını görün; doğru
    bilgiyle bağlandıktan hemen sonra `systemctl show -p NRestarts wificorrect-pppoe.service` 0 mı (ip-up kancası sıfırladı) ve sonraki bir kesintide
    aralık yeniden 10 sn'den mi başlıyor.
  - *Servis:* panelden bilerek yanlış parola (ya da bu hattın VLAN'ı yanlış) → durum "Kullanıcı adı ya da parola reddedildi"; telefonla misafir Wi-Fi'ına bağlanın:
    telefon "internet yok" uyarısına rağmen Wi-Fi'da kalıyor mu (iOS ve Android), `10.50.0.1:8080` ve `10.50.0.1:8080/servis` açılıyor mu, uyarı + Servis bağlantısı
    görünüyor mu (bu sırada `ip -o route show default`'ta en çok dhcpcd'nin IPv4LL satırı olmalı; uyarı görünmüyorsa bu çıktıyı not edin); AP yönetim parolasıyla
    doğru bilgiyi yazın → izleme sayfası → bağlanma → giriş sayfasına dönüş (tünel yeniden başlatılmaz: arayüz varsa `ip link show wfc` MTU 1412 olur ve
    internet gelince el sıkışma kendiliğinden döner; cihaz internetsiz açıldıysa tünel hiç kalkmamış olabilir, o zaman tünel gözcüsü en çok 10 dk içinde
    kurar); yanlış Servis parolası 5 kez → kilit mesajı ve
    `SERVIS_GIRIS` satırları; kayıtlı kullanıcı adı hiçbir Servis sayfasında görünmüyor. **PPPoE seçimi:** PPPoE hiç ayarlanmamış (Kapalı) bir cihazda Servis
    formunda Açık/Kapalı seçimi **Açık hazır geliyor mu** (panelin Portlar kutusunda aynı cihazda Kapalı görünür); bilgileri girip kaydedince "Ayar
    kaydedildi; PPPoE yeniden bağlanıyor" ve durum PPPoE'ye geçiyor mu; Kapalı seçip kaydedince "Ayar kaydedildi; PPPoE kapalı…" mesajı ve durum "Modemden adres
    alınamadı…"/"PPPoE kapalı (modemden DHCP)"; reddedilen bir kayıtta (yanlış Servis parolası) seçilen Kapalı/Açık formda korunuyor mu.
  - *Yükseltme:* PPPoE kapalı Göztepe ve Visavis'te güncelleme sonrası misafir girişi, tünel ve `nft` kuralları eskisi gibi; `dpkg -s ppp` kurulu; sağlık kartında
    "İnternet: Modem (DHCP)" ve yönetimde "Yenile"den 5 dk sonra cihazda yeni AP parolası.
