# Devir notu — kaldığın yerden devam (2026-10-11)

Bu dosya, projeye yeni bir oturumda/hesapta devam eden ajan içindir. Önce bunu, sonra `CLAUDE.md`'deki belgeleri oku.

## Kullanıcıyla çalışma kuralları (zorunlu)
- **Her zaman Türkçe** konuş. Kullanıcı kısa ve doğrudan yanıt ister; "devam edeyim mi" diye sık sorma, uygun varsayılanla ilerle.
- Cihaz komutlarını **sen çalıştırırsın**: `ssh wificorrect` (root, anahtarla; cihaz 192.168.1.110, modem ağı).
  Sunucu: `ssh -i ~/.ssh/gbserver gbserver@192.168.1.109` (sudo parolası **yalnızca kullanıcıda**; sudo gereken işleri
  kullanıcıya terminalde çalıştırtırsın — cmd için: `"C:\Program Files\Git\bin\bash.exe" <betik>`).
- **Asla** parola, API anahtarı, belirteç girme/kullanma — kullanıcı sohbete yazsa bile. `openwrt/serverp` dosyasını okuma.
- Gerçek SMS yalnızca kullanıcı açıkça onaylarsa. `/srv/5651` dosyalarını silme. Ağ/güvenlik duvarı değişikliği yalnızca
  ölü adam anahtarıyla (yedek + `systemd-run --on-active=300` geri alma + YENİ bağlantıyla doğrula + iptal).
- Sırlar repoya girmez (repo **herkese açık**: github.com/bilgisayargoztepe-lgtm/wificorrect).
- Cihaz testleri **cihazda** koşar: `bash scripts/test.sh wificorrect [süzgeç]` (~5 dk; aynı anda iki tane çalıştırma).
  Cihaza kurulum: `bash scripts/gelistir.sh` + `ssh wificorrect 'systemctl restart wificorrect-panel wificorrect-portal wificorrect-kaydedici'`.
  Merkez testleri: `sh scripts/sunucu/merkez/tests/hepsi.sh python`.
- Merkez kurulumu: `scripts/sunucu/merkez-guncelle.sh` (şu anki daldan; önce veritabanı yedeği, sonra `kur.sh` —
  Caddyfile doğrulanmadan kurulmaz). Kullanıcı çalıştırır (sudo).

## Yeni bilgisayar kurulumu (2026-10-10'da başka PC'ye geçildi)
- Gerekenler: Git for Windows (Git Bash), Python 3, GitHub CLI (`gh auth login`, hesap bilgisayargoztepe-lgtm), Claude Code.
  Rust gerekmez (derleme ve testler cihazda).
- Repo: `git clone https://github.com/bilgisayargoztepe-lgtm/wificorrect.git` → `git checkout uzak-panel`.
- SSH: `~/.ssh/config`'e
  ```
  Host wificorrect
      HostName 192.168.1.110
      User root
      IdentityFile ~/.ssh/bocafe
  ```
  Anahtarlar: `~/.ssh/bocafe` (cihaz root) ve `~/.ssh/gbserver` (sunucu). Ya eski PC'den güvenli yolla kopyalanır ya da yeni
  PC'de yeni anahtar üretilip açık anahtar cihazın `/root/.ssh/authorized_keys`'ine ve sunucuda
  `gbserver`'ın `~/.ssh/authorized_keys`'ine eklenir.
- Ağ: cihaz (192.168.1.110) ve sunucu (192.168.1.109) SSH'ı yalnızca dükkânın modem ağından. Dışarıdan çalışılıyorsa
  WireGuard yönetici bağlantısı (`wificorrect-disaridan.conf`) açılır; o zaman cihaz `10.99.0.11`, sunucu `10.99.0.1`.

## Ürün özeti
WifiCorrect: işletmelere yıllık abonelikle satılan 5651 uyumlu, SMS (NetGSM) doğrulamalı misafir internet cihazı.
Cihaz: Debian 13 + Rust ikilisi `wificorrect` (`src/`). Merkez: Python stdlib (`scripts/sunucu/merkez/`), Caddy 2.6.2 arkasında,
`yonetim.wificorrect.com` (hizmet sağlayıcı), `panel.wificorrect.com` (müşteri), `api.wificorrect.com` (cihazlar),
`cihaz.wificorrect.com` (cihaz paneline geçit). Cihaz ↔ merkez WireGuard (10.99.0.0/24; merkez .1, cihaz 1537344 = .11).
Canlı müşteri: **Göztepe Bilgisayar = 1537344**, cihaz 192.168.1.110.

## Dallar ve durum (2026-10-11)
Repo `bilgisayargoztepe-lgtm/wificorrect` (avsharbeyi'den devredildi). Her şey **`main`**'de (PR #3 uzak-panel, #4 ozet-tasarim,
#5 hiz-olcer birleşti; PR #2 kapatıldı). **v0.3.0** yayında (ISO + .deb; `WFC_SSH_PUB` = `~/.ssh/bocafe.pub`).
Canlı cihaz `hiz-olcer` (ec74ddf ≈ main) çalıştırıyor; merkez değişmedi.

Bitenler (canlıda):
- ISO (Debian netinst + preseed + .deb), GitHub'da QEMU duman testi (`iso.yml`), `release.yml` yalnızca `v*` etiketinde.
- Parolalar: müşteri parolasını yönetici yazar; tek admin parolası (merkezden bütün cihazlara).
- Uzak panel (spec 2026-10-08): giriş yalnızca panel.wificorrect.com'dan → `cihaz.wificorrect.com`; telefondan denendi, çalışıyor.
- **Panel görünümü** (örnek görsel: koyu üst şerit + koyu sol menü, kırmızı vurgu): menü Özet, Kullanıcılar, Kayıtlar,
  Ayarlar, Admin; grup sayfalarında simgeli kutucuklar + kırmızı başlıklı bölüm kutuları. **Özet**: kısayollar (Bağlı
  kullanıcılar, Site/IP ara, Yasaklı siteler), Bağlı Kullanıcılar, sayılar, Bugün Yeni Üyeler, SMS ve Disk halkaları
  (telefon/MAC yok; her bakış `PANEL_OZET` denetimi).
- **Hız ve yavaşlatma** (spec `docs/superpowers/specs/2026-10-10-hiz-ve-kullanim-design.md`): nft `inet wfc_hiz` IP başına
  sayaç (`/etc/wificorrect/hiz.nft`), kaydedici 5 sn'de `/run/wificorrect/trafik.json`; Bağlı kullanıcılar tablosunda
  ↓/↑ Mb/sn, Bugün MB, sıralama, 10 sn yenileme; Yavaşlat 1/2/5/10/20 Mb/sn (HTB + meta priority, MAC'e bağlı, kaldırılana
  kadar), Yavaşlatılmış cihazlar bölümü; izinli cihazlar da görünür. Kabul: `scripts/hiz-deneme.sh` (sahte istemci; WAN'ı
  ~100 sn doldurur, sakin saatte) 2026-10-10'da 5/5 PASS. Canlıya kurulum ölü adam anahtarıyla yapıldı.

## Sıradaki işler
1. Önerilen sonraki özellikler (kullanıcı seçecek): donanım bekçisi (watchdog — J1900 donuyor), merkezden uzaktan güncelleme
   (apt deposu), merkezde sağlık izleme + uyarı, online ödeme (`veri.lisans_uzat`), personel kodu ile giriş, aylık rapor,
   hazır engel listeleri, imzalı resmi talep raporu. Zaman damgası: kontör alınınca (merkezde). İki aşamalı giriş istenmedi.
2. Hız özelliğinin ertelenmiş küçükleri (ledger'da): bozuk `hiz_sinir.json` sonraki yazımda bütün sınırları siler; 10 sn
   yenileme Kullanıcılar sayfasındaki formlara yazılanı siler; her /cihazlar bakışı iki kişisel veri satırı yazar
   (OTURUMLAR + YAVASLATILMIS); `hiz_sinir.json`'daki adlar saklama süresine tabi değil (KVKK kararı kullanıcıda).
3. Geniş ekranda içerik 1320 px'te duruyor (tam genişlik önerildi, kullanıcı karar vermedi).

## Açık küçük notlar
- AP (TP-Link TL-WA901ND, 10.50.0.2) menzili zayıf; kullanıcı sonra ilgilenecek (iletim gücü High, kanal 1/6/11, 20 MHz).
- Monitörsüz açılış ve J1900 donması için GRUB ayarı `deploy/debian/etc/default/grub.d/wificorrect.cfg` (cihazda uygulandı).
- Ertelenmiş küçük bulgular ve verilen bütün kararlar: `.superpowers/sdd/*/progress.md` (git'e girmez, yalnızca bu makinede).
