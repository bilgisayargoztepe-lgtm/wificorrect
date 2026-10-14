"""Hazır engel listeleri (spec 2026-10-14): kaynak ayrıştırma, temizleme, üretim, /api/saglik + /api/liste, yönetim sayfası.
Ağa çıkılmaz: bütün indirmeler sahte `getir` ile."""
import gzip
import hashlib
import json
import os
import sys
import tempfile

os.environ["WC_TEST"] = "1"

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import listeler as L  # noqa: E402
import api as A  # noqa: E402
import yonetici  # noqa: E402
import yonetim as YN  # noqa: E402

SIMDI = 1_790_000_000.0
WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI cihaz"
PW = "yonetici-parola-1"


def sahte(dosyalar, cagrilar=None):
    """url → bytes ya da Exception örneği (fırlatılır)."""
    def getir(url, sinir):
        if cagrilar is not None:
            cagrilar.append(url)
        b = dosyalar[url]
        if isinstance(b, Exception):
            raise b
        if len(b) > sinir:
            raise ValueError("çok büyük")
        return b
    return getir


def kaynaklar_koy(kaynaklar):
    """Testte KAYNAKLAR'ı geçici olarak değiştirir; eskiyi döner."""
    eski = L.KAYNAKLAR
    L.KAYNAKLAR = kaynaklar
    return eski


def gz_oku(yol):
    with gzip.open(yol, "rb") as f:
        return f.read().decode("utf-8")


# --- ayrıştırma ve temizleme ---
def test_hosts_bicimi():
    s = ["# yorum", "127.0.0.1 localhost", "127.0.0.1 localhost.localdomain", "0.0.0.0 0.0.0.0",
         "0.0.0.0 Kotu-Site.COM  # sonda yorum", "0.0.0.0 a.example.net b.example.org", "", "  0.0.0.0   bosluklu.com.tr  "]
    assert L.temizle(s, "hosts") == {"kotu-site.com", "a.example.net", "b.example.org", "bosluklu.com.tr"}


def test_duz_bicimi():
    s = ["# HaGeZi", "! baska yorum", "kumar.com", "BAHIS.net.", "  bet365.com  ", "x.y.z.org # not"]
    assert L.temizle(s, "duz") == {"kumar.com", "bahis.net", "bet365.com", "x.y.z.org"}


def test_adblock_bicimi():
    s = ["[Adblock Plus]", "! Title: x", "||kumar.com^", "||alt.bahis.net^$important", "@@||izinli.com^",
         "kotu.org", "||yol.com/a^", "||*.joker.com^", "/regex/"]
    assert L.temizle(s, "adblock") == {"kumar.com", "alt.bahis.net"}


def test_usom_bicimi():
    """Yol ya da sorgu içeren URL atlanır (paylaşımlı sitedeki tek dosya bütün siteyi engellemesin); çıplak adlar alınır."""
    s = ["http://phish.example.com/login.php?x=1", "https://Kotu.COM:8443/", "zararli.net", "ftp://user@dosya.org",
         "http://1.2.3.4/", "www.banka-giris.xyz/", "# yorum", "https://github.com/kotu/repo/raw/a.exe",
         "https://cdn.discordapp.com/attachments/1/2/x.exe", "http://bit.ly/abc", "https://t.me/kanal", "yol.com/a",
         "sorgu.com?x=1", "sorgu2.com/?x=1", "parca.com#x", "http://kotu2.com"]
    assert L.temizle(s, "usom") == {"kotu.com", "zararli.net", "dosya.org", "www.banka-giris.xyz", "parca.com", "kotu2.com"}


def test_paylasimli_platformlar_ve_genis_son_ekler():
    s = ["github.com", "raw.githubusercontent.com", "cdn.discordapp.com", "discord.com", "dropbox.com", "dl.dropbox.com",
         "bit.ly", "t.me", "ipfs.io", "mediafire.com", "pastebin.com", "drive.google.com", "docs.google.com",
         "github.io", "blogspot.com", "co.uk", "com.br", "workers.dev", "cloudfront.net", "amazonaws.com",
         "kotu.github.io", "sahte.blogspot.com", "d111.cloudfront.net", "kotu.co.uk", "phish.workers.dev"]
    assert L.temizle(s, "duz") == {"kotu.github.io", "sahte.blogspot.com", "d111.cloudfront.net", "kotu.co.uk",
                                   "phish.workers.dev"}


def test_baglanti_denetimi_ve_ntp_asla():
    s = ["detectportal.firefox.com", "connectivitycheck.gstatic.com", "captive.apple.com", "www.msftconnecttest.com",
         "pool.ntp.org", "tr.pool.ntp.org", "firefox.com"]
    assert L.temizle(s, "duz") == {"firefox.com"}


def test_html_denetimi_bom_ve_dogtype():
    assert L.html_mi("\ufeff<!DOCTYPE html><html></html>")
    assert L.html_mi("\ufeff\n  <HTML lang=tr>")
    assert L.html_mi("# baslik\n" + "x" * 100 + "\n<!doctype html>")
    assert L.html_mi("kotu.com\n<html>")
    assert not L.html_mi("\ufeffkotu.com\nbaska.com\n")
    assert not L.html_mi("a.com\n" * 2000 + "<html>")  # ilk 4 KB'ın dışında


def test_gecersizler_ip_tek_etiket_kamu_son_eki_atilir():
    s = ["1.2.3.4", "10.0.0.1", "tekad", "com", "com.tr", "gov.tr", "k12.tr", "net", "a..b.com", "alt_cizgi.com",
         "ü.com", "a" * 64 + ".com", ".".join(["a" * 60] * 5), "-", "*.com", "iyi.com", "iyi.bel.tr"]
    assert L.temizle(s, "duz") == {"iyi.com", "iyi.bel.tr"}


def test_asla_engelleme_ve_alt_adlari_cikar():
    s = ["wificorrect.com", "api.wificorrect.com", "netgsm.com.tr", "verify.twilio.com", "www.google.com",
         "connectivitycheck.gstatic.com", "e-devlet.gov.tr", "odtu.edu.tr", "captive.apple.com",
         "notwificorrect.com", "google.com.kotu.net", "msftconnecttest.com"]
    assert L.temizle(s, "duz") == {"notwificorrect.com", "google.com.kotu.net"}


def test_buyuk_ortak_platformlar_asla():
    """Bir alt adı bile engellenirse milyonlarca meşru kullanıcıyı etkileyen ortak altyapılar ve alt adları."""
    s = ["googleusercontent.com", "lh3.googleusercontent.com", "x.azureedge.net", "blob.core.windows.net",
         "cloudflare.com", "abc.trycloudflare.com", "facebook.com", "m.facebook.com", "scontent.fbcdn.net",
         "instagram.com", "www.youtube.com", "i.ytimg.com", "x.akamaihd.net", "login.live.com", "office.com",
         "outlook.com", "web.whatsapp.com", "mmg.whatsapp.net", "wa.me", "kotu-facebook.com", "facebook.com.kotu.net"]
    assert L.temizle(s, "duz") == {"kotu-facebook.com", "facebook.com.kotu.net"}


def test_telefon_sosyal_alisveris_banka_asla():
    """Telefon üreticileri, sosyal medya, alışveriş, ödeme ve bankalar (ve alt adları) hiçbir listeden gelmez."""
    asla = ["hicloud.com", "miui.com", "xiaomi.com", "oppomobile.com", "samsung.com", "samsungcloud.com", "x.com",
            "twitter.com", "tiktok.com", "amazon.com", "trendyol.com", "hepsiburada.com", "papara.com", "iyzico.com",
            "garantibbva.com.tr", "isbank.com.tr", "ziraatbank.com.tr", "akbank.com", "yapikredi.com.tr",
            "vakifbank.com.tr", "halkbank.com.tr", "qnb.com.tr", "denizbank.com", "enpara.com", "turkiye.gov.tr"]
    alt = ["giris." + a for a in asla]
    assert L.temizle(asla + alt + ["akbank-giris.com", "x.com.kotu.net"], "duz") == {"akbank-giris.com", "x.com.kotu.net"}


def test_onbellek_okurken_yeniden_dogrulanir():
    """Eski sürümün yazdığı önbellekte artık asla listesinde olan ya da geçersiz ad varsa listeye girmez."""
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            yol = L._onbellek_yolu(tmp, K, "https://a/liste")
            L._atomik(yol, L._gz("a.com\nm.facebook.com\n1.2.3.4\nwa.me\nb.com\n"))
            assert L._onbellek_oku(yol) == {"a.com", "b.com"}
            L.uret(K, sahte({"https://a/liste": OSError("ag yok"), "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            assert gz_oku(os.path.join(tmp, "kumar.txt.gz")) == "a.com\nb.com\nc.com\n"
    finally:
        L.KAYNAKLAR = eski


def test_vpn_doh_alan_adlari_cikar():
    s = ["nordvpn.com", "api.nordvpn.com", "mullvad.net", "dns.google", "cloudflare-dns.com", "doh.example.org",
         "vpn.sirket.com.tr", "dns.quad9.net", "kotu-site.com", "phishing-vpnlogin.com"]
    assert L.temizle(s, "duz") == {"kotu-site.com", "phishing-vpnlogin.com"}


# --- üretim ---
K = "kumar"


def iki_kaynak():
    return {K: [("A", "https://a/liste", "duz"), ("B", "https://b/liste", "adblock")]}


def test_uret_yazar_meta_ve_sha():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            f = {"https://a/liste": b"b.com\na.com\nalt.a.com\n", "https://b/liste": b"||c.com^\n||a.com^\n"}
            meta = L.uret(K, sahte(f), tmp, SIMDI)
            metin = gz_oku(os.path.join(tmp, "kumar.txt.gz"))
            assert metin == "a.com\nb.com\nc.com\n"  # sıralı; alt.a.com üst ad engellendiği için gereksiz
            assert meta["sha256"] == hashlib.sha256(metin.encode()).hexdigest()
            assert meta["sayi"] == 3 and meta["kategori"] == K and meta["kesildi"] is False
            assert meta["zaman"].startswith("2026-09-2") and "hata" not in meta
            assert meta["kaynaklar"] == [{"ad": "A", "sayi": 3}, {"ad": "B", "sayi": 2}]
            with open(os.path.join(tmp, "kumar.json"), encoding="utf-8") as fj:
                assert json.load(fj) == meta
            assert not [x for x in os.listdir(tmp) if x.endswith(".tmp")]
    finally:
        L.KAYNAKLAR = eski


def test_ayni_girdi_ayni_sha_ve_ayni_gzip():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        f = {"https://a/liste": b"b.com\na.com\n", "https://b/liste": b"||c.com^\n"}
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as t1, \
                tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as t2:
            m1 = L.uret(K, sahte(f), t1, SIMDI)
            m2 = L.uret(K, sahte(dict(f, **{"https://a/liste": b"a.com\nb.com\n"})), t2, SIMDI + 500)
            assert m1["sha256"] == m2["sha256"]
            assert open(os.path.join(t1, "kumar.txt.gz"), "rb").read() == open(os.path.join(t2, "kumar.txt.gz"), "rb").read()
    finally:
        L.KAYNAKLAR = eski


def test_sinir_ve_kesildi():
    eski = kaynaklar_koy(iki_kaynak())
    eski_sinir = L.EN_COK
    L.EN_COK = 5
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            a = "".join(f"a{i}.com\n" for i in range(4)).encode()
            b = "".join(f"||b{i}.com^\n" for i in range(4)).encode()
            meta = L.uret(K, sahte({"https://a/liste": a, "https://b/liste": b}), tmp, SIMDI)
            assert meta["sayi"] == 5 and meta["kesildi"] is True
            alanlar = gz_oku(os.path.join(tmp, "kumar.txt.gz")).split()
            assert {f"a{i}.com" for i in range(4)} <= set(alanlar)  # önce gelen kaynak öncelikli
    finally:
        L.KAYNAKLAR, L.EN_COK = eski, eski_sinir


def test_varsayilan_sinir_300000():
    assert L.EN_COK == 300_000


def test_bir_kaynak_hata_digerleriyle_uretilir():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            meta = L.uret(K, sahte({"https://a/liste": OSError("ag yok"), "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            assert meta["sayi"] == 1 and "hata" not in meta
            assert meta["kaynaklar"][0]["ad"] == "A" and "ag yok" in meta["kaynaklar"][0]["hata"]
            assert meta["kaynaklar"][1] == {"ad": "B", "sayi": 1}
    finally:
        L.KAYNAKLAR = eski


def test_hatali_kaynak_onceki_basarili_kopyasiyla_devam():
    """Bir kaynak bugün inmezse (ya da HTML/boş dönerse) dünkü temiz kopyası kullanılır: liste aniden küçülmez."""
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            L.uret(K, sahte({"https://a/liste": b"a.com\nb.com\n", "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            for bozuk in (OSError("ag yok"), b"<!DOCTYPE html><html>http://x.com/</html>", b"# bos\n"):
                meta = L.uret(K, sahte({"https://a/liste": bozuk, "https://b/liste": b"||d.com^\n"}), tmp, SIMDI + 10)
                assert gz_oku(os.path.join(tmp, "kumar.txt.gz")) == "a.com\nb.com\nd.com\n", bozuk
                assert meta["kaynaklar"][0]["hata"] and meta["kaynaklar"][0]["sayi"] == 2 and meta["kaynaklar"][0]["eski"] is True
    finally:
        L.KAYNAKLAR = eski


def test_hepsi_hata_eski_dosya_korunur():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            iyi = L.uret(K, sahte({"https://a/liste": b"a.com\n", "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            gz = open(os.path.join(tmp, "kumar.txt.gz"), "rb").read()
            meta = L.uret(K, sahte({"https://a/liste": OSError("x"), "https://b/liste": OSError("y")}), tmp, SIMDI + 100)
            assert open(os.path.join(tmp, "kumar.txt.gz"), "rb").read() == gz
            assert meta["hata"] and meta["sha256"] == iyi["sha256"] and meta["sayi"] == iyi["sayi"] and meta["zaman"] == iyi["zaman"]
            with open(os.path.join(tmp, "kumar.json"), encoding="utf-8") as fj:
                assert json.load(fj)["hata"]
            assert L.ozet(tmp)[K] == {"sha256": iyi["sha256"], "sayi": 2, "zaman": iyi["zaman"]}
            # sonra düzelince hata kalkar
            assert "hata" not in L.uret(K, sahte({"https://a/liste": b"a.com\n", "https://b/liste": b"||c.com^\n"}), tmp, SIMDI + 200)
    finally:
        L.KAYNAKLAR = eski


def test_hic_liste_yokken_hepsi_hata():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            meta = L.uret(K, sahte({"https://a/liste": OSError("x"), "https://b/liste": OSError("y")}), tmp, SIMDI)
            assert meta["hata"] and "sha256" not in meta
            assert not os.path.exists(os.path.join(tmp, "kumar.txt.gz")) and K not in L.ozet(tmp)
    finally:
        L.KAYNAKLAR = eski


def test_elle_ek_dosyasi():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            with open(os.path.join(tmp, "kumar.ek.txt"), "w", encoding="utf-8") as f:
                f.write("# Türkiye eki\nyerelbahis.com\ngoogle.com\n")
            meta = L.uret(K, sahte({"https://a/liste": b"a.com\n", "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            assert "yerelbahis.com" in gz_oku(os.path.join(tmp, "kumar.txt.gz")).split()
            assert "google.com" not in gz_oku(os.path.join(tmp, "kumar.txt.gz"))
            assert {"ad": "Elle eklenenler", "sayi": 1} in meta["kaynaklar"]
    finally:
        L.KAYNAKLAR = eski


def test_kaynaklar_kategoriler_ve_vpn_listesi_yok():
    assert set(L.KAYNAKLAR) == {"kumar", "yetiskin", "zararli"} == set(L.KATEGORILER)
    assert L.KATEGORILER["kumar"] == "Kumar ve bahis" and L.KATEGORILER["yetiskin"] == "Yetişkin içerik"
    assert L.KATEGORILER["zararli"] == "Zararlı ve dolandırıcı siteler"
    for k, ks in L.KAYNAKLAR.items():
        for ad, url, bicim in ks:
            assert url.startswith("https://") and bicim in ("hosts", "duz", "adblock", "usom")
            assert "doh-vpn-proxy-bypass" not in url and "vpn" not in url.lower()


def test_main_uretir_ve_hareket():
    eski = kaynaklar_koy({"kumar": [("A", "https://a", "duz")], "yetiskin": [("B", "https://b", "duz")],
                          "zararli": [("C", "https://c", "duz")]})
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            v, _, _ = Y.ortam(tmp)
            d = os.path.join(tmp, "l")
            f = {"https://a": b"a.com\n", "https://b": b"b.com\n", "https://c": OSError("ag yok")}
            assert L.calistir(v, sahte(f), d, SIMDI) == 1  # biri hata
            ol = {(h["olay"], h["ayrinti"].split()[0]) for h in v.hareketler()}
            assert ("LISTE_GUNCEL", "kategori=kumar") in ol and ("LISTE_GUNCEL", "kategori=yetiskin") in ol
            assert ("LISTE_HATA", "kategori=zararli") in ol
            n = len(v.hareketler())
            assert L.calistir(v, sahte(dict(f, **{"https://c": b"c.com\n"})), d, SIMDI + 10) == 0
            # değişmeyen kategori için yeniden LISTE_GUNCEL yazılmaz; yalnızca zararli
            yeni = v.hareketler()[: len(v.hareketler()) - n]
            assert [(h["olay"], h["ayrinti"].split()[0]) for h in yeni] == [("LISTE_GUNCEL", "kategori=zararli")]
    finally:
        L.KAYNAKLAR = eski


def test_bom_ile_baslayan_html_kaynak_hata_bom_li_liste_iyi():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            meta = L.uret(K, sahte({"https://a/liste": "\ufeff<!DOCTYPE html><p>kotu.com</p>".encode(),
                                    "https://b/liste": "\ufeff||c.com^\n".encode()}), tmp, SIMDI)
            assert "HTML" in meta["kaynaklar"][0]["hata"] and meta["kaynaklar"][1] == {"ad": "B", "sayi": 1}
            meta = L.uret(K, sahte({"https://a/liste": "\ufeffilk.com\nikinci.com\n".encode(),
                                    "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            assert meta["kaynaklar"][0] == {"ad": "A", "sayi": 2}  # BOM ilk satırı bozmaz
    finally:
        L.KAYNAKLAR = eski


def test_kucuk_liste_tabani():
    """Önceki kopya > 1000 iken yeni liste yarısından küçükse hata: kopya kullanılır ve üzerine yazılmaz."""
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            buyuk = "".join(f"a{i}.com\n" for i in range(2000)).encode()
            L.uret(K, sahte({"https://a/liste": buyuk, "https://b/liste": b"||c.com^\n"}), tmp, SIMDI)
            kucuk = "".join(f"a{i}.com\n" for i in range(999)).encode()
            for _ in range(2):  # ikinci turda da kopya hâlâ 2000'lik (üzerine yazılmadı)
                meta = L.uret(K, sahte({"https://a/liste": kucuk, "https://b/liste": b"||c.com^\n"}), tmp, SIMDI + 10)
                d = meta["kaynaklar"][0]
                assert "beklenenden çok küçük: 999 / eski 2000" in d["hata"] and d["sayi"] == 2000 and d["eski"] is True
                assert meta["sayi"] == 2001
            # yarısı ya da fazlası kabul edilir
            meta = L.uret(K, sahte({"https://a/liste": "".join(f"a{i}.com\n" for i in range(1000)).encode(),
                                    "https://b/liste": b"||c.com^\n"}), tmp, SIMDI + 20)
            assert meta["kaynaklar"][0] == {"ad": "A", "sayi": 1000} and meta["sayi"] == 1001
            # küçük önceki kopyada (≤ 1000) taban uygulanmaz
            meta = L.uret(K, sahte({"https://a/liste": b"z.com\n", "https://b/liste": b"||c.com^\n"}), tmp, SIMDI + 30)
            assert meta["kaynaklar"][0] == {"ad": "A", "sayi": 1}
    finally:
        L.KAYNAKLAR = eski


def test_kaynak_hatasi_hareketi_ve_guncel():
    eski = kaynaklar_koy({"kumar": [("A", "https://a", "duz"), ("B", "https://b", "duz")], "yetiskin": [], "zararli": []})
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            v, _, _ = Y.ortam(tmp)
            d = os.path.join(tmp, "l")
            assert L.calistir(v, sahte({"https://a": OSError("ag yok"), "https://b": b"b.com\n"}), d, SIMDI) == 1
            h = [(x["olay"], x["ayrinti"]) for x in v.hareketler() if "kategori=kumar" in x["ayrinti"]]
            assert ("LISTE_HATA", "kategori=kumar kaynak=A neden=ag yok") in h
            assert any(o == "LISTE_GUNCEL" and a.startswith("kategori=kumar surum=") and "sayi=1" in a for o, a in h)
    finally:
        L.KAYNAKLAR = eski


def test_simdi_guncelle_mesgulse_beklemez():
    eski_kilit = L._Kilit

    class Dolu:
        def __init__(self, dizin, bekle=True):
            assert bekle is False  # web isteği beklememeli
        def __enter__(self):
            raise L.Mesgul()
        def __exit__(self, *a):
            pass
    L._Kilit = Dolu
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            cagri = []
            app, v, c, d = yonetim_kur(tmp, sahte({}, cagri))
            t = Y.csrf(Y.al(app, "/listeler", c))
            r = Y.gonder(app, "/listeler/guncelle", c, {"csrf": t})
            assert r[0] == 200 and "zaten sürüyor" in r[2].decode() and cagri == []
    finally:
        L._Kilit = eski_kilit


# --- API ---
class Kuyruk:
    def ekle(self, islem, veri):
        return "0" * 16

    def bekle(self, kimlik):
        return {"durum": "tamam", "tunel_ip": "10.99.0.10", "sunucu_pub": "S" * 43 + "=", "uc_nokta": "vpn.wificorrect.com:51820"}


RAPOR = {"surum": "1.2.3", "acik_sn": 5000, "servisler": {"portal": True, "panel": True, "kaydedici": True, "dnsmasq": True, "guvenlik": True},
         "disk_yuzde": 42, "son_muhur": "2026-10-10", "kayit_boslugu_1s": False, "wan": True, "sms_sorunu": False, "bagli": 7, "hiz_hata": False}
_ip = [0]


def post(api, yol, govde):
    _ip[0] += 1
    return api.istek("POST", yol, {}, json.dumps(govde), "", "203.0.113.%d" % (_ip[0] % 250))


def api_kur(tmp):
    v, _, saat = Y.ortam(tmp)
    q = Kuyruk()
    d = os.path.join(tmp, "listeler")
    api = A.Api(v, saat, q.ekle, q.bekle, liste_dizini=d)
    n, pw = v.musteri_ekle()
    _, _, g = post(api, "/api/giris", {"numara": str(n), "parola": pw, "wg_pub": WG, "ssh_pub": SSH})
    return api, v, n, json.loads(g)["cihaz_anahtari"], d


def test_saglik_cevabinda_listeler():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            api, v, n, k, d = api_kur(tmp)
            durum, _, g = post(api, "/api/saglik", {"cihaz_anahtari": k, "rapor": RAPOR})
            assert durum == 200 and json.loads(g)["listeler"] == {}
            meta = L.uret(K, sahte({"https://a/liste": b"a.com\n", "https://b/liste": b"||c.com^\n"}), d, SIMDI)
            with open(os.path.join(d, "yetiskin.json"), "w", encoding="utf-8") as f:
                f.write("{bozuk")
            j = json.loads(post(api, "/api/saglik", {"cihaz_anahtari": k, "rapor": RAPOR})[2])
            assert j["listeler"] == {K: {"sha256": meta["sha256"], "sayi": 2, "zaman": meta["zaman"]}}
    finally:
        L.KAYNAKLAR = eski


def test_api_liste_200_400_403_404():
    eski = kaynaklar_koy(iki_kaynak())
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            api, v, n, k, d = api_kur(tmp)
            assert post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": K})[0] == 404  # henüz üretilmedi
            meta = L.uret(K, sahte({"https://a/liste": b"a.com\n", "https://b/liste": b"||c.com^\n"}), d, SIMDI)
            durum, h, g = post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": K})
            assert durum == 200 and h["Content-Type"] == "application/gzip" and h["Cache-Control"] == "no-store"
            assert hashlib.sha256(gzip.decompress(g)).hexdigest() == meta["sha256"]
            for kotu in ("vpn", "../kumar", "", 5, None):
                assert post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": kotu})[0] == 400, kotu
            assert post(api, "/api/liste", {"cihaz_anahtari": "yanlis", "kategori": K})[0] == 401
            v.uyelik(n, "bitti")
            assert post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": K})[0] == 403
            v.uyelik(n, "aktif")
            assert post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": K})[0] == 200
            os.remove(os.path.join(d, "kumar.txt.gz"))
            assert post(api, "/api/liste", {"cihaz_anahtari": k, "kategori": K})[0] == 404
    finally:
        L.KAYNAKLAR = eski


# --- yönetim ---
def yonetim_kur(tmp, getir):
    v, k, saat = Y.ortam(tmp)
    yol, dyol = os.path.join(tmp, "yonetici.json"), os.path.join(tmp, "durum.json")
    yonetici.yaz(yol, "serkan", PW)
    with open(dyol, "w", encoding="utf-8") as f:
        json.dump({"zaman": "", "arsivler": {}}, f)
    d = os.path.join(tmp, "listeler")
    app = YN.Yonetim(v, k, saat, yol, dyol, lambda *a: "0" * 16, liste_getir=getir, liste_dizini=d)
    return app, v, Y.cerez(Y.giris(app, "serkan", PW)), d


def test_yonetim_listeler_sayfasi_ve_simdi_guncelle():
    eski = kaynaklar_koy({"kumar": [("HaGeZi kumar", "https://a", "duz")], "yetiskin": [("B", "https://b", "duz")],
                          "zararli": [("USOM", "https://c", "usom")]})
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            cagri = []
            f = {"https://a": b"a.com\nb.com\n", "https://b": b"x.com\n", "https://c": OSError("ag yok")}
            app, v, c, d = yonetim_kur(tmp, sahte(f, cagri))
            assert Y.al(app, "/listeler", "")[0] in (302, 303, 401, 403)
            s = Y.al(app, "/listeler", c)[2].decode()
            assert 'href="/listeler"' in s and "Kumar ve bahis" in s and "Zararlı ve dolandırıcı siteler" in s
            t = Y.csrf(Y.al(app, "/listeler", c))
            assert Y.gonder(app, "/listeler/guncelle", c, {"csrf": "yanlis"})[0] == 403 and cagri == []
            r = Y.gonder(app, "/listeler/guncelle", c, {"csrf": t})
            assert r[0] in (200, 303) and set(cagri) == {"https://a", "https://b", "https://c"}
            assert os.path.exists(os.path.join(d, "kumar.txt.gz"))
            s = Y.al(app, "/listeler", c)[2].decode()
            assert "HaGeZi kumar" in s and "ag yok" in s and "USOM" in s
            ol = {h["olay"] for h in v.hareketler()}
            assert "LISTE_GUNCEL" in ol and "LISTE_HATA" in ol
    finally:
        L.KAYNAKLAR = eski


def test_kur_sh_ve_birimler():
    kok = os.path.dirname(HERE)
    kur = open(os.path.join(kok, "kur.sh"), encoding="utf-8").read()
    assert "listeler.py" in kur and "wc-listeler.timer" in kur and "wc-listeler.service" in kur
    zam = open(os.path.join(kok, "wc-listeler.timer"), encoding="utf-8").read()
    assert "04:30" in zam and "Persistent=true" in zam
    srv = open(os.path.join(kok, "wc-listeler.service"), encoding="utf-8").read()
    assert "User=wcpanel" in srv and "ProtectSystem=strict" in srv and "ReadWritePaths=/var/lib/wificorrect/merkez" in srv


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
