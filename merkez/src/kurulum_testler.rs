//! Kurulum betikleri ve systemd birimleri (yalnızca testler): `scripts/sunucu/merkez/kur.sh`, `wc-*` birimleri,
//! `merkez-kur.sh`, `merkez-geri-don.sh`, `wificorrect-sunucu`. Python'daki dört kur.sh/birim testi (test_rapor,
//! test_surumler x2, test_listeler) yeni ikiliye ve birimlere uyarlanarak buraya taşındı; Python sürümleri `.py` kurulumunu
//! denetliyordu, bunlar aynı niyeti (doğru birim, kullanıcı, kısıt, takvim, kur.sh'nin kurduğu dosyalar) Rust ikilisi için denetler.
use std::path::{Path, PathBuf};
use std::process::Command;

use super::altin::Kayit;

fn sunucu_dizini() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts").join("sunucu")
}

fn merkez_dizini() -> PathBuf {
    sunucu_dizini().join("merkez")
}

fn oku(yol: &Path) -> String {
    std::fs::read_to_string(yol).unwrap_or_else(|e| panic!("{}: {e}", yol.display())).replace("\r\n", "\n")
}

fn kur_sh() -> String {
    oku(&merkez_dizini().join("kur.sh"))
}

fn birim(ad: &str) -> String {
    oku(&merkez_dizini().join(ad))
}

/// `Anahtar=değer` satırlarının değerleri (aynı anahtar birden çok kez geçebilir).
fn degerler(metin: &str, anahtar: &str) -> Vec<String> {
    let on = format!("{anahtar}=");
    metin.lines().filter_map(|l| l.strip_prefix(&on)).map(|v| v.trim().to_string()).collect()
}

fn tek(metin: &str, anahtar: &str) -> String {
    let d = degerler(metin, anahtar);
    assert_eq!(d.len(), 1, "{anahtar}= bir kez olmalı: {d:?}");
    d[0].clone()
}

/// Yorum satırları çıkarılmış betik (başlıktaki açıklamalar yasak sözcükleri anabilir).
fn komutlar(betik: &str) -> String {
    betik.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n")
}

const IKILI: &str = "/usr/local/bin/wificorrect-merkez";

/// (birim, alt komut, kullanıcı). Kullanıcılar Python birimleriyle aynı; `None` = User= satırı yok (root).
const HIZMETLER: [(&str, &str, Option<&str>); 7] = [
    ("wc-merkez.service", "sunucu", Some("wcpanel")),
    ("wc-surumler.service", "surumler al", Some("wcpanel")),
    ("wc-saglik.service", "saglik", Some("wcpanel")),
    ("wc-listeler.service", "listeler", Some("wcpanel")),
    ("wc-durum.service", "durum", None),
    ("wc-istatistik.service", "istatistik", None),
    ("wc-kuyruk.service", "kuyruk", None),
];

#[test]
fn kur_sh_ve_merkez_birimi() {
    // Python test_rapor.test_kur_sh_ve_merkez_birimi (M7, M8): rapor modülleri ve dizinleri kurulur, merkez rapor-ham'a erişemez
    let _k = Kayit::bas("test_rapor", "test_kur_sh_ve_merkez_birimi");
    let kur = kur_sh();
    // Python: "rapor.py" in kur and "raporlar.py" in kur -> rapor modülleri artık ikilinin içinde: ikili kurulur, .py kurulmaz
    assert!(kur.contains("dosya_koy 755 \"$PAKET_IKILI\" \"$IKILI\""), "kur.sh ikiliyi kurmuyor");
    assert!(kur.contains("IKILI=/usr/local/bin/wificorrect-merkez"));
    assert!(!kur.contains("rapor.py") && !kur.contains("raporlar.py"));
    // Python: "install -d -o root -g wcpanel -m 750" in kur and ".../rapor\n" in kur
    let satir = kur.lines().find(|l| l.starts_with("install -d -o root -g wcpanel -m 750")).expect("rapor dizinleri satırı");
    assert!(satir.split_whitespace().any(|a| a == "$VERI/rapor"), "{satir}");
    // Python: "install -d -m 700 /var/lib/wificorrect/rapor-ham" in kur
    assert!(kur.contains("install -d -m 700 $VERI/rapor-ham"));
    assert!(kur.contains("VERI=/var/lib/wificorrect\n"));
    // Python: wc-merkez.service içinde InaccessiblePaths=-/var/lib/wificorrect/rapor-ham
    let srv = birim("wc-merkez.service");
    assert!(degerler(&srv, "InaccessiblePaths").contains(&"-/var/lib/wificorrect/rapor-ham".to_string()));
    // yeni: merkez wcpanel olarak çalışır ve ikilinin sunucu komutunu başlatır
    assert_eq!(tek(&srv, "User"), "wcpanel");
    assert_eq!(tek(&srv, "ExecStart"), format!("{IKILI} sunucu"));
}

#[test]
fn kur_sh_ve_saatlik_zamanlayici() {
    // Python test_surumler.test_kur_sh_ve_saatlik_zamanlayici
    let _k = Kayit::bas("test_surumler", "test_kur_sh_ve_saatlik_zamanlayici");
    let kur = kur_sh();
    // Python: kur.sh birimleri /etc/systemd/system/ altına kurar (çok satırlı install, wc-surumler.service wc-surumler.timer ... /etc/systemd/system/)
    let kurulum = kur.find("install -m 644 wc-merkez.service").expect("birim install komutu");
    let kurulum = &kur[kurulum..kur[kurulum..].find("$BIRIMLER/\n").map(|i| kurulum + i).expect("install sonu")];
    assert!(kurulum.contains("wc-surumler.service wc-surumler.timer"), "{kurulum}");
    assert!(kur.contains("BIRIMLER=/etc/systemd/system\n"));
    // Python: systemctl enable -q --now ... wc-surumler.timer
    let zam = kur.lines().find(|l| l.starts_with("ZAMANLAYICILAR=")).expect("ZAMANLAYICILAR");
    assert!(zam.contains("wc-surumler.timer"));
    assert!(kur.contains("systemctl enable -q --now $ZAMANLAYICILAR"));
    // Python: OnCalendar=hourly ve Persistent=true
    let t = birim("wc-surumler.timer");
    assert!(degerler(&t, "OnCalendar").contains(&"hourly".to_string()) && tek(&t, "Persistent") == "true");
    // Python: ExecStart=python3 .../surumler.py al -> yeni ikili: surumler al
    let s = birim("wc-surumler.service");
    assert_eq!(tek(&s, "ExecStart"), format!("{IKILI} surumler al"));
    // Python: User=wcpanel, ProtectSystem=strict, ReadWritePaths=/var/lib/wificorrect/merkez
    assert_eq!(tek(&s, "User"), "wcpanel");
    assert_eq!(tek(&s, "ProtectSystem"), "strict");
    assert_eq!(tek(&s, "ReadWritePaths"), "/var/lib/wificorrect/merkez");
}

#[test]
fn kur_sh_yerel_modulleri_kuruyor() {
    // Python test_surumler.test_kur_sh_yerel_modulleri_kuruyor: bir modülün import ettiği her yerel modül kur.sh ile kurulur.
    // Rust'ta modül yok, tek ikili var: eşdeğer iddia, birimlerin çağırdığı her alt komut kurulan ikilide vardır ve kur.sh
    // Python'dan hiçbir şey kurmaz/yapmaz.
    let _k = Kayit::bas("test_surumler", "test_kur_sh_yerel_modulleri_kuruyor");
    let kur = kur_sh();
    let ana = oku(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("main.rs"));
    for (ad, komut, _) in HIZMETLER {
        let ilk = komut.split_whitespace().next().unwrap();
        assert!(ana.contains(&format!("Some(\"{ilk}\")")), "{ad}: ikilide {ilk} alt komutu yok");
        assert_eq!(tek(&birim(ad), "ExecStart"), format!("{IKILI} {komut}"), "{ad}");
    }
    // kur.sh yalnızca ikiliyi ve wificorrect-sunucu'yu kurar; Python dosyası, wc-* bağı, logo kurmaz (ikiliye gömülü)
    let kmt = komutlar(&kur);
    assert!(!kmt.contains(".py"), "kur.sh .py dosyasına dokunmamalı");
    // python_donemi satırı Python dönemi dizinlerini tanımak için bu yolu desen olarak anar; kurulum komutu değildir
    let kmt_kurulum: String = kmt.lines().filter(|l| !l.starts_with("python_donemi()")).collect::<Vec<_>>().join("\n");
    assert!(!kmt_kurulum.contains("ln -s") && !kmt_kurulum.contains("/usr/local/sbin/wc-") && !kmt_kurulum.contains("/usr/local/lib/wificorrect"));
    assert!(!kmt.contains("logo/") && !kmt.contains("/logo"));
    assert!(!kmt.contains("PYTHONDONTWRITEBYTECODE"));
    // kur.sh'nin kopyaladığı ikili paketteki ikilidir
    assert!(kur.contains("PAKET_IKILI=$PWD/wificorrect-merkez"));
}

#[test]
fn kur_sh_ve_birimler() {
    // Python test_listeler.test_kur_sh_ve_birimler
    let _k = Kayit::bas("test_listeler", "test_kur_sh_ve_birimler");
    let kur = kur_sh();
    // Python: "listeler.py" in kur and "wc-listeler.timer" in kur and "wc-listeler.service" in kur
    assert!(kur.contains("wc-listeler.timer") && kur.contains("wc-listeler.service"));
    assert!(!kur.contains("listeler.py"));
    // Python: 04:30 ve Persistent=true
    let t = birim("wc-listeler.timer");
    assert!(tek(&t, "OnCalendar").contains("04:30"));
    assert_eq!(tek(&t, "Persistent"), "true");
    // Python: User=wcpanel, ProtectSystem=strict, ReadWritePaths=/var/lib/wificorrect/merkez
    let s = birim("wc-listeler.service");
    assert_eq!(tek(&s, "User"), "wcpanel");
    assert_eq!(tek(&s, "ProtectSystem"), "strict");
    assert_eq!(tek(&s, "ReadWritePaths"), "/var/lib/wificorrect/merkez");
    assert_eq!(tek(&s, "ExecStart"), format!("{IKILI} listeler"));
}

#[test]
fn birimler_kullanici_kisit_ve_takvim_aynen() {
    // Python birimleriyle aynı: kullanıcı, kısıtlar ve takvimler değişmez; yalnız ExecStart ikiliye geçti, PYTHONDONTWRITEBYTECODE gitti
    let wcpanel_kisitlari = [
        ("NoNewPrivileges", "yes"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "yes"),
        ("PrivateTmp", "yes"),
        ("PrivateDevices", "yes"),
    ];
    for (ad, _, kullanici) in HIZMETLER {
        let b = birim(ad);
        assert!(degerler(&b, "Environment").is_empty(), "{ad}: Environment kalmamalı");
        assert!(!b.contains("python"), "{ad}: python geçmemeli");
        match kullanici {
            Some(k) => {
                assert_eq!(tek(&b, "User"), k, "{ad}");
                assert_eq!(tek(&b, "Group"), k, "{ad}");
                for (a, d) in wcpanel_kisitlari {
                    assert_eq!(tek(&b, a), d, "{ad}: {a}");
                }
                assert!(degerler(&b, "InaccessiblePaths").contains(&"/srv".to_string()), "{ad}");
            }
            None => assert!(degerler(&b, "User").is_empty(), "{ad}: root olarak çalışır, User= olmamalı"),
        }
    }
    let m = birim("wc-merkez.service");
    assert_eq!(tek(&m, "ReadWritePaths"), "/var/lib/wificorrect/merkez /var/lib/wificorrect/kuyruk");
    assert_eq!(tek(&m, "Restart"), "on-failure");
    assert_eq!(tek(&m, "RestartSec"), "3");
    assert_eq!(tek(&m, "WantedBy"), "multi-user.target");
    for ad in ["wc-surumler.service", "wc-saglik.service", "wc-listeler.service"] {
        assert_eq!(tek(&birim(ad), "ReadWritePaths"), "/var/lib/wificorrect/merkez", "{ad}");
    }
    // oneshot'lar ve zaman aşımları
    for (ad, sure) in [
        ("wc-durum.service", "10min"),
        ("wc-istatistik.service", "2h"),
        ("wc-kuyruk.service", "10min"),
        ("wc-listeler.service", "20min"),
        ("wc-saglik.service", "4min"),
        ("wc-surumler.service", "20min"),
    ] {
        let b = birim(ad);
        assert_eq!(tek(&b, "TimeoutStartSec"), sure, "{ad}");
        assert_eq!(tek(&b, "Type"), "oneshot", "{ad}");
    }
    // zamanlayıcılar
    let d = birim("wc-durum.timer");
    assert_eq!((tek(&d, "OnBootSec").as_str(), tek(&d, "OnUnitActiveSec").as_str()), ("1min", "5min"));
    let i = birim("wc-istatistik.timer");
    assert_eq!(degerler(&i, "OnCalendar"), ["*-*-* 02:30:00", "*-*-* 12:30:00"]);
    assert_eq!(tek(&i, "Persistent"), "true");
    let s = birim("wc-saglik.timer");
    assert_eq!((tek(&s, "OnBootSec").as_str(), tek(&s, "OnUnitActiveSec").as_str(), tek(&s, "AccuracySec").as_str()), ("3min", "5min", "10s"));
    let l = birim("wc-listeler.timer");
    assert_eq!((tek(&l, "OnCalendar").as_str(), tek(&l, "RandomizedDelaySec").as_str()), ("*-*-* 04:30", "5min"));
    let su = birim("wc-surumler.timer");
    assert_eq!((tek(&su, "OnBootSec").as_str(), tek(&su, "RandomizedDelaySec").as_str()), ("5min", "5min"));
    // kuyruk: dosya gelince tetiklenir
    let p = birim("wc-kuyruk.path");
    assert_eq!(tek(&p, "PathExistsGlob"), "/var/lib/wificorrect/kuyruk/*.json");
    assert_eq!(tek(&p, "Unit"), "wc-kuyruk.service");
    for ad in ["wc-durum.timer", "wc-istatistik.timer", "wc-saglik.timer", "wc-listeler.timer", "wc-surumler.timer"] {
        assert_eq!(tek(&birim(ad), "WantedBy"), "timers.target", "{ad}");
    }
}

#[test]
fn kur_sh_birimleri_kurar_zamanlayicilari_acar() {
    let kur = kur_sh();
    let kurulum = kur.find("install -m 644 wc-merkez.service").expect("birim install komutu");
    let kurulum = &kur[kurulum..kur[kurulum..].find("$BIRIMLER/\n").map(|i| kurulum + i).unwrap()];
    // bütün birim dosyaları kurulur (dizindeki her wc-* dosyası)
    let mut dosyalar: Vec<String> = std::fs::read_dir(merkez_dizini())
        .unwrap()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .filter(|a| a.starts_with("wc-"))
        .collect();
    dosyalar.sort();
    assert_eq!(dosyalar.len(), 13, "{dosyalar:?}");
    for a in &dosyalar {
        assert!(kurulum.split_whitespace().any(|k| k == a), "kur.sh {a} dosyasını kurmuyor");
    }
    // zamanlayıcılar ve yol birimi etkinleştirilir; wc-merkez etkin ve yeniden başlatılır
    let zam = kur.lines().find(|l| l.starts_with("ZAMANLAYICILAR=")).unwrap();
    for a in dosyalar.iter().filter(|a| a.ends_with(".timer") || a.ends_with(".path")) {
        assert!(zam.contains(a.as_str()), "ZAMANLAYICILAR {a} içermiyor");
    }
    assert!(kur.contains("systemctl enable -q wc-merkez\nsystemctl restart wc-merkez"));
    // eski kafe paneli ve Python izleri yok
    for yasak in ["panel.py", "panel_auth", "hesap.py", "wc-panel", "hesaplar.json", "wc-hesap", "PYTHONDONTWRITEBYTECODE"] {
        assert!(!komutlar(&kur).contains(yasak), "kur.sh {yasak} içermemeli");
    }
    // boş sunucu: caddy, fail2ban ve curl (Rust merkez dış istekleri curl ile yapar), wcpanel kullanıcısı, dizinler
    for k in [
        "apt-get install -y -q caddy",
        "apt-get install -y -q fail2ban",
        "apt-get install -y -q curl",
        "useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin wcpanel",
        "install -d -o wcpanel -g wcpanel -m 700 $VERI/merkez $VERI/kuyruk",
        "install -d -o root -g wcpanel -m 750 $VERI/detay $VERI/kuyruk-sonuc $VERI/rapor",
        "dosya_koy 755 \"$SUNUCU_BETIGI\" \"$SUNUCU\"",
        "SUNUCU=/usr/local/sbin/wificorrect-sunucu",
        "install -m 755 hotspot-arsiv-saklama /etc/cron.daily/",
    ] {
        assert!(kur.contains(k), "kur.sh {k:?} içermiyor");
    }
}

#[test]
fn kur_sh_adim_sirasi() {
    // yedek -> denetle -> geri dönüş kopyası -> kur -> başlat -> 200 (ruling 13); işlevler üstte, ana akış "# ANA AKIŞ"tan sonra
    let kur = kur_sh();
    let ana = &kur[kur.find("# ANA AKIŞ").expect("ana akış işareti")..];
    let konum = |s: &str| ana.find(s).unwrap_or_else(|| panic!("ana akışta {s:?} yok"));
    let sira = [
        konum("# ADIM 1: veritabanı yedeği"),
        konum("veritabani-yedekle \"$DB\" \"$yedek_gecici\""),
        konum("# ADIM 2:"),
        konum("veritabani-denetle \"$yedek_gecici\" --dosyalar \"$VERI\""),
        konum("if [ \"$YALNIZ_DENETLE\" = 1 ]"),
        konum("# ADIM 3: geri dönüş kopyası"),
        konum("cp -p \"$f\" \"$GD/\""),
        konum("# ADIM 4:"),
        konum("dosya_koy 755 \"$PAKET_IKILI\" \"$IKILI\""),
        konum("# ADIM 5: başlat"),
        konum("systemctl restart wc-merkez"),
        konum("# ADIM 6:"),
        konum("if ! saglik_denetle; then"),
    ];
    assert!(sira.windows(2).all(|p| p[0] < p[1]), "adım sırası bozuk: {sira:?}");
    // denetim hatası canlıya dokunmadan çıkar (adım 3'ten önce exit 1)
    let denetim = &ana[konum("# ADIM 2:")..konum("# ADIM 3:")];
    assert!(denetim.contains("Canlıya dokunulmadı") && denetim.contains("exit 1") && denetim.contains("exit 0"));
    // 200 denetimi
    assert!(kur.contains("curl -s -m 5 --connect-timeout 2 -o /dev/null -w '%{http_code}' -H 'Host: yonetim.wificorrect.com' http://127.0.0.1:8081/giris"));
    assert!(kur.contains("[ \"$k\" = 200 ]"));
    // sağlık geçmezse geri dönüş dizininden kendiliğinden geri dönülür ve çıkış 1
    let son = &ana[konum("# ADIM 6:")..];
    assert!(son.contains("exit 1") && kur.contains("geri_don_uygula \"$GD\"") && kur.contains("GERİ DÖNÜLDÜ"));
    // en yeni 5 geri dönüş dizini tutulur; yalnız /etc/systemd birimleri, wificorrect-sunucu ve ikili kopyalanır
    assert!(ana.contains("python_donemi \"$d\" || diger+=") && ana.contains("${#diger[@]}\" -gt 5") && ana.contains("budandı"));
    assert!(ana.contains("GERI=$VERI/geri-donus") || kur.contains("GERI=$VERI/geri-donus"));
    // --geri-don: dizini yerine koyar, yeniden başlatır, 200 denetler
    let gd = &kur[kur.find("geri_don_uygula() {").unwrap()..];
    let gd = &gd[..gd.find("\n}\n").unwrap()];
    for k in ["dosya_koy 644", "dosya_koy 755 \"$f\" \"$SUNUCU\"", "dosya_koy 755 \"$f\" \"$IKILI\"", "systemctl daemon-reload", "systemctl restart wc-merkez", "saglik_denetle"] {
        assert!(gd.contains(k), "geri_don_uygula {k:?} içermiyor");
    }
}

#[test]
fn betikler_bash_soz_dizimi_ve_surumsuz_dosyalar() {
    // dagit.sh ve merkez-guncelle.sh yerine merkez-kur.sh
    assert!(!sunucu_dizini().join("dagit.sh").exists() && !sunucu_dizini().join("merkez-guncelle.sh").exists());
    let kur = oku(&sunucu_dizini().join("merkez-kur.sh"));
    for k in ["--ikili", "--kuru", "--yalniz-denetle", "gh run list --commit", "--workflow test", "gh run download", "wificorrect-merkez-$sha", "sha256sum -c", "git archive HEAD scripts/sunucu", "tee /tmp/wfc-sunucu/sonuc.txt", "ssh -t", "sudo bash /tmp/wfc-sunucu/merkez/kur.sh"] {
        assert!(kur.contains(k), "merkez-kur.sh {k:?} içermiyor");
    }
    // CI denetimi --ikili ile atlanır, --kuru sunucuya bağlanmaz (ssh çağrıları --kuru çıkışından sonra)
    assert!(kur.find("exit 0").unwrap() < kur.find("# 4) sunucuya kopyala").unwrap());
    let geri = oku(&sunucu_dizini().join("merkez-geri-don.sh"));
    assert!(geri.contains("kur.sh --geri-don") && geri.contains("ssh -t") && geri.contains("sudo bash"));
    // wificorrect-sunucu yonetici-parola Rust ikilisini çağırır (wc-yonetici bağı gider)
    let sunucu = oku(&sunucu_dizini().join("wificorrect-sunucu"));
    assert!(sunucu.contains(&format!("exec {IKILI} yonetici parola \"$2\"")));
    assert!(!sunucu.contains("wc-yonetici"));
    // bash -n (bash yoksa atla)
    if Command::new("bash").arg("--version").output().is_err() {
        return;
    }
    for yol in [merkez_dizini().join("kur.sh"), sunucu_dizini().join("merkez-kur.sh"), sunucu_dizini().join("merkez-geri-don.sh"), sunucu_dizini().join("wificorrect-sunucu")] {
        let o = Command::new("bash").arg("-n").arg(&yol).output().unwrap();
        assert!(o.status.success(), "{}: {}", yol.display(), String::from_utf8_lossy(&o.stderr));
    }
}

#[test]
fn kur_sh_geri_donus_guvenceleri() {
    let kur = kur_sh();
    let kmt = komutlar(&kur);
    // sağlık curl'ünde zaman aşımı, toplam ~20 sn
    assert!(kmt.contains("curl -s -m 5 --connect-timeout 2 -o /dev/null -w '%{http_code}'") && kmt.contains("bitis=$((SECONDS + 20))"));
    // hata olursa EXIT tuzağı geri döner: yalnız geri dönüş dizini oluşunca açılır, 200'den sonra kapanır
    assert!(kmt.contains("trap kurtar EXIT") && kmt.contains("GERI_TUZAK=1; fi") && kmt.contains("GERI_TUZAK=0\n"));
    assert!(kur.find("GERI_TUZAK=1").unwrap() < kur.find("# ADIM 4:").unwrap());
    assert!(kur.find("if ! saglik_denetle; then").unwrap() < kur.rfind("GERI_TUZAK=0").unwrap());
    assert!(kmt.contains("set +e") && kmt.contains("GERİ DÖNÜLDÜ"));
    // dosya/komut başına hata sayımı
    assert!(kmt.matches("hata=$((hata + 1))").count() >= 7 && kmt.contains("geri dönüşte $hata hata oldu"));
    // ZAMAN doğrulaması merkez-geri-don.sh ile aynı regex
    let re = "^[0-9]{8}-[0-9]{6}$";
    assert!(kmt.contains(re));
    assert!(oku(&sunucu_dizini().join("merkez-geri-don.sh")).contains(re));
    // tutarlı yedek: pakettaki ikiliyle, sahip/izin kaynağınkiyle aynı
    assert!(kmt.contains("\"$PAKET_IKILI\" veritabani-yedekle \"$DB\" \"$yedek_gecici\""));
    assert!(kmt.contains("chown --reference=\"$DB\" \"$yedek_gecici\"") && kmt.contains("chmod --reference=\"$DB\" \"$yedek_gecici\"") && kmt.contains("mktemp -d \"$VERI/.yedek.XXXXXX\"") && kmt.contains("mv -f -T \"$yedek_gecici\" \"$yedek\""));
    assert!(!kmt.contains("cp -p \"$DB\""));
    // kur.sh kalıcı saklanır, acil dönüş komutu yazılır; Python işareti ve öğüt
    assert!(kmt.contains("install -m 700 \"$BU_BETIK\" \"$GERI_KUR\"") && kmt.contains("GERI_KUR=$GERI/kur.sh"));
    assert!(kmt.contains("sudo bash $GERI_KUR --geri-don"));
    assert!(kur.contains("(Python)") && kur.contains("'(Python)' işaretli") && !kur.contains("en ESKİ dizinin"));
    assert!(!kmt.contains("ls -1 \"$GERI\""));
}

#[test]
fn kur_sh_python_donemi_dizinleri_ayirt_edilir() {
    if Command::new("bash").arg("--version").output().is_err() {
        return;
    }
    let kur = kur_sh();
    let islev = |ad: &str| kur.lines().find(|l| l.starts_with(ad)).unwrap_or_else(|| panic!("{ad}")).to_string();
    let d = crate::ortak::tests::gecici_dizin();
    for (dizin, birim) in [
        ("20261001-000000", "ExecStart=/usr/bin/python3 /usr/local/lib/wificorrect/merkez.py"),
        ("20261002-000000", "ExecStart=/usr/local/sbin/wc-saglik"),
        ("20261003-000000", "ExecStart=/usr/local/bin/wificorrect-merkez sunucu"),
    ] {
        std::fs::create_dir_all(d.join(dizin)).unwrap();
        std::fs::write(d.join(dizin).join("wc-merkez.service"), format!("[Service]\n{birim}\n")).unwrap();
    }
    std::fs::write(d.join("kur.sh"), "x").unwrap();
    let betik = format!("GERI='{}'\n{}\n{}\nfor x in $(geri_dizinleri); do if python_donemi \"$x\"; then echo \"$x P\"; else echo \"$x R\"; fi; done\n", d.display(), islev("geri_dizinleri()"), islev("python_donemi()"));
    let o = Command::new("bash").arg("-c").arg(&betik).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout), "20261001-000000 P\n20261002-000000 P\n20261003-000000 R\n", "{}", String::from_utf8_lossy(&o.stderr));
}

/// kur.sh'deki `kurtar` işlevi (kopya değil, dosyadan çıkarılır) gerçek bash ile: sahte `geri_don_uygula`, geçici dizin.
/// `senaryo` kur.sh'nin tuzak kurulumundan sonra çalışan betik gövdesidir. Döner: (geri_don_uygula kaç kez çağrıldı, son-geri-donus.txt var mı).
fn kurtar_dene(senaryo: &str, boruyu_oldur: bool) -> (usize, Option<String>) {
    let kur = kur_sh();
    let basla = kur.find("kurtar() {").expect("kurtar işlevi");
    let bitis = basla + kur[basla..].find("\n}\n").expect("kurtar sonu") + 3;
    let islev = &kur[basla..bitis];
    let d = crate::ortak::tests::gecici_dizin();
    let betik = d.join("dene.sh");
    let cagri = d.join("cagri");
    let govde = format!(
        "set -euo pipefail\nGERI='{d}'\nSON_GERI=\"$GERI/son-geri-donus.txt\"\nGD='{d}/gd'\nYEDEK_TMP=\nyedek=\nGERI_TUZAK=0\n\
         geri_don_uygula() {{ echo cagrildi >>'{c}'; echo 'geri dönüş çıktısı'; return 0; }}\n{islev}\ntrap kurtar EXIT\n{senaryo}\n",
        d = d.display(),
        c = cagri.display(),
    );
    std::fs::write(&betik, govde).unwrap();
    let komut = if boruyu_oldur { format!("bash '{}' | true", betik.display()) } else { format!("bash '{}'", betik.display()) };
    let _ = Command::new("bash").arg("-c").arg(&komut).output().unwrap();
    let say = std::fs::read_to_string(&cagri).map(|m| m.lines().count()).unwrap_or(0);
    (say, std::fs::read_to_string(d.join("son-geri-donus.txt")).ok())
}

#[test]
fn kurtar_sinyal_ve_olu_boru_davranisi() {
    if Command::new("bash").arg("--version").output().is_err() {
        return;
    }
    // (a) tuzak etkinken SIGTERM: geri dönüş bir kez; sonuç dosyaya da yazılır
    let (say, dosya) = kurtar_dene("GERI_TUZAK=1\nkill -TERM $$\nsleep 5\n", false);
    assert_eq!(say, 1, "SIGTERM'de geri dönüş bir kez çağrılmalı");
    assert!(dosya.unwrap().contains("GERİ DÖNÜLDÜ"));
    // (b) çıktı ölü boruya giderken (tee ölmüş) exit 1: geri dönüş yine tamamlanır
    let (say, dosya) = kurtar_dene("GERI_TUZAK=1\nsleep 1\nexit 1\n", true);
    assert_eq!(say, 1, "ölü boruda geri dönüş çağrılmalı");
    assert!(dosya.unwrap().contains("GERİ DÖNÜLDÜ"));
    // (b2) SIGHUP (SSH kopması) ve çıkış 0 görünen set -e dışı durum: tuzak etkinken her çıkış geri döner
    let (say, _) = kurtar_dene("GERI_TUZAK=1\nkill -HUP $$\nsleep 5\n", false);
    assert_eq!(say, 1);
    // (c) başarı yolu (tuzak önce kapatılır): çağrılmaz
    let (say, dosya) = kurtar_dene("GERI_TUZAK=1\nGERI_TUZAK=0\nexit 0\n", false);
    assert_eq!((say, dosya), (0, None));
    // (d) tuzak kapalıyken sinyal: çağrılmaz
    let (say, dosya) = kurtar_dene("GERI_TUZAK=0\nkill -TERM $$\nsleep 5\n", false);
    assert_eq!((say, dosya), (0, None));
}
