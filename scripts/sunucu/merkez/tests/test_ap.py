"""AP arayüzü — merkez (spec 2026-10-16 "Mimari (sözleşme)" 2–3): cihaz.wificorrect.com/_ap-gecis → 60 sn'lik tek
kullanımlık belirteç → ap.wificorrect.com/_ap-giris → wca çerezi → Caddy forward_auth /ap-yetki → /_ap-cikis."""
import contextlib
import html
import io
import os
import re
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import ap as A  # noqa: E402
import cihaz as C  # noqa: E402
import guvenlik as G  # noqa: E402
import merkez as MZ  # noqa: E402
import musteri as M  # noqa: E402

WG, WG2, SSH = "A" * 43 + "=", "B" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"
CIHAZ = {"Host": "cihaz.wificorrect.com"}
AP = {"Host": "ap.wificorrect.com"}
IP = "203.0.113.5"
PANEL_GIRIS = "https://panel.wificorrect.com/giris"
AP_GIRIS = "https://ap.wificorrect.com/_ap-giris?t="
CADDYFILE = os.path.join(HERE, "..", "Caddyfile")


def kur(tmp):
    v, k, saat = Y.ortam(tmp)
    gecis = G.GecisBelirtecleri()
    m = M.Musteri(v, k, saat, gecis=gecis)
    c = C.Cihaz(v, gecis, m, saat)
    a = A.Ap(c, saat)
    n = musteri_ac(v, "musteri-parola-1", WG, "10.99.0.11")
    return v, c, a, saat, n


def musteri_ac(v, pw, wg, tunel):
    n, _ = v.musteri_ekle(parola=pw)
    v.cihaz_bagla(n, wg, SSH)
    v.cihaz_guncelle(v.bagli_cihaz(n)["id"], tunel_ip=tunel)
    return n


def wcc_ac(c, v, saat, n, rol="musteri"):
    return c.oturumlar.create(str(n), rol, saat(), v.musteri(n)["ozet"])


def gecis(c, wcc, ip=IP):
    return c.istek("GET", "/_ap-gecis", dict(CIHAZ, Cookie=f"wcc={wcc}") if wcc else dict(CIHAZ), "", "", ip)


def belirtec(r):
    assert r[0] == 303 and r[1]["Location"].startswith(AP_GIRIS), r
    return r[1]["Location"][len(AP_GIRIS):]


def giris(a, t, cerez=""):
    return a.istek("GET", "/_ap-giris", dict(AP, Cookie=cerez), "", "t=" + t, IP)


def wca(r):
    """Yeni AP oturumunun çerezi: /_ap-giris'in ilk Set-Cookie'si (ardından eski AP çerezlerinin silinmesi gelebilir)."""
    assert r[0] == 303 and r[1]["Location"] == "/", r
    return r[1]["Set-Cookie"][0].split(";")[0].split("=", 1)[1]


def yetki(a, wca_):
    return a.istek("GET", "/ap-yetki", dict(AP, Cookie=f"wca={wca_}") if wca_ else dict(AP), "", "", IP)


def ap_ac(c, a, v, saat, n, rol="musteri"):
    """Cihaz paneli oturumu (wcc) + AP oturumu (wca)."""
    w = wcc_ac(c, v, saat, n, rol)
    return w, wca(giris(a, belirtec(gecis(c, w))))


# --- belirteç deposu, oturum temizliği ---
def test_tek_kullanimlik_belirtec():
    b = G.TekKullanimlik(60)
    t = b.uret({"x": 1}, 1000.0)
    assert b.tuket(t, 1059.0) == {"x": 1} and b.tuket(t, 1059.0) is None  # bir kez
    t = b.uret("d", 1000.0)
    assert b.tuket(t, 1060.0) is None  # 60 sn dolunca geçersiz
    assert b.tuket("", 1000.0) is None and b.tuket(None, 1000.0) is None and b.tuket("yok", 1000.0) is None
    b.uret("a", 1000.0)
    b.uret("b", 2000.0)
    assert len(b._b) == 1  # süresi dolanlar her üretimde silinir (bellek şişmesin)


def test_oturumlar_ek_alan_ve_suresi_dolanlar_silinir():
    s = G.Oturumlar(bosta=1800, en_cok=1800)
    t = s.create("1234567", "musteri", 100.0, ek={"wcc": "w", "user": "baskasi"})
    o = s.get(t, 100.0)
    assert o["wcc"] == "w" and o["user"] == "1234567"  # ek alan asıl alanları ezemez
    assert s.get(t, 1500.0) is not None and s.get(t, 1901.0) is None  # bakılsa da en çok 30 dk
    s.create("1", "musteri", 100.0)
    s.create("2", "musteri", 5000.0)
    assert len(s._s) == 1  # süresi dolan oturum yeni oturum açılırken silinir


# --- /_ap-gecis (cihaz.wificorrect.com, forward_auth dışı) ---
def test_gecis_belirteci_wca_cerezi_ve_yetki():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        r = gecis(c, w)
        t = belirtec(r)
        assert r[1]["Cache-Control"] == "no-store" and "Set-Cookie" not in r[1]
        h = v.hareketler(n)[0]
        assert (h["olay"], h["kim"], h["ip"], h["musteri"], h["ayrinti"]) == ("AP_GECIS", str(n), IP, n, "rol=musteri")
        r = giris(a, t)
        z = wca(r)
        assert r[1]["Set-Cookie"] == [r[1]["Set-Cookie"][0]]  # eski AP çerezi yoktu: yalnızca yeni wca
        sc = r[1]["Set-Cookie"][0]
        parcalar = [p.strip() for p in sc.split(";")]
        # Strict (final review I2): başka sitenin üst düzey GET gezinmesi de AP'ye oturumlu istek gönderemez
        assert parcalar[0].startswith("wca=") and {"HttpOnly", "Secure", "SameSite=Strict", "Path=/", "Max-Age=1800"} <= set(parcalar)
        assert "domain" not in sc.lower()  # yalnızca ap.wificorrect.com (alt/üst alanlara gitmez)
        assert r[1]["Cache-Control"] == "no-store"
        # yeni AP oturumu (müşteri değişimi): kökün depolaması — Service Worker kayıtları dahil — silinir (final review C1).
        # "cookies" yok: kayıtlı alan adının (wificorrect.com) bütün çerezlerini, yeni wca'yı da siler. "cache" yok
        # (re-review m4): bu kökteki her cevap no-store, önbellekte bir şey yok; Chromium'da önbellek silme yönlendirmeyi
        # gözle görülür geciktirir
        assert r[1]["Clear-Site-Data"] == '"storage"'
        y = yetki(a, z)
        assert y[0] == 200 and y[2] == b""
        # AP kipinde kullanıcı ayrı başlıkta (final review M2): 0.4.15 öncesi cihaz X-WFC-Kullanici'li isteği oturumlu panel
        # isteği sayar ve paneli ap.wificorrect.com kökünde verirdi; yeni başlığı tanımaz, isteği oturumsuz sayar
        assert (y[1]["X-WFC-Cihaz"], y[1]["X-WFC-AP-Kullanici"], y[1]["X-WFC-AP"]) == ("10.99.0.11", str(n), "1")
        assert "X-WFC-Kullanici" not in y[1]
        assert y[1]["Cache-Control"] == "no-store"
        # belirteç bir kez: ikinci kullanımda sade Türkçe sayfa, çerez yok
        r = giris(a, t)
        assert r[0] == 403 and "Set-Cookie" not in r[1] and "text/html" in r[1]["Content-Type"]
        s = r[2].decode()
        assert "geçersiz" in s and "https://cihaz.wificorrect.com/" in s
        assert giris(a, "")[0] == 403 and a.istek("GET", "/_ap-giris", AP, "", "", IP)[0] == 403


def test_gecis_oturumsuz_panel_girisine():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        for w in ("", "uydurma"):
            r = gecis(c, w)
            assert r[0] == 303 and r[1]["Location"] == PANEL_GIRIS, w
        w = wcc_ac(c, v, saat, n)
        v.askiya_al(n, True)  # lisans kapalı: cihaz paneli oturumu da geçersiz
        assert gecis(c, w)[1]["Location"] == PANEL_GIRIS
        assert [h for h in v.hareketler(n) if h["olay"] == "AP_GECIS"] == []


def test_yonetici_gecisi():
    """Yönetimden cihaz paneline geçen yönetici (wcc rolü "yonetici"): hareket "admin"; cihaza giden kullanıcı yine müşteri
    numarası (cihaz X-WFC-Kullanici'yi kendi bağındaki numarayla karşılaştırır, cihaz-yetki ile aynı)."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        _, z = ap_ac(c, a, v, saat, n, rol="yonetici")
        h = v.hareketler(n)[0]
        assert (h["olay"], h["kim"], h["ayrinti"]) == ("AP_GECIS", "admin", "rol=yonetici")
        y = yetki(a, z)
        assert y[0] == 200 and y[1]["X-WFC-AP-Kullanici"] == str(n) and y[1]["X-WFC-AP"] == "1"


def _getirme(site, mod="navigate", hedef="document"):
    return {"Sec-Fetch-Site": site, "Sec-Fetch-Mode": mod, "Sec-Fetch-Dest": hedef}


def test_gecis_yalnizca_cihaz_panelinden_ust_duzey_gezinmeyle():
    """AP arayüzü (ap.wificorrect.com) panel ve cihazla aynı "site": kötü niyetli bir AP'nin betiği çerçeve/fetch/yeni
    pencereyle /_ap-gecis'i tetikleyip tarayıcıdaki BAŞKA müşterinin cihaz paneli oturumuyla (wcc, aynı-site isteklerde
    gönderilir) wca aldıramamalı. Fetch Metadata: yalnızca cihaz.wificorrect.com sayfasından (same-origin) ya da elle yazılan
    adresle (none) üst düzey gezinme; başlıkları göndermeyen eski tarayıcı geçer."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        h = dict(CIHAZ, Cookie=f"wcc={w}")
        for kotu in (_getirme("same-site"), _getirme("cross-site"), _getirme("same-origin", hedef="iframe"),
                     _getirme("same-origin", mod="no-cors", hedef="empty"), _getirme("same-origin", mod="cors", hedef="empty"),
                     _getirme("same-origin", hedef="image")):
            r = c.istek("GET", "/_ap-gecis", dict(h, **kotu), "", "", IP)
            assert r[0] == 403 and "Location" not in r[1] and "cihaz panelinden" in r[2].decode(), kotu
        assert [x for x in v.hareketler(n) if x["olay"] == "AP_GECIS"] == [] and c.ap_belirtecleri._b == {}
        for iyi in (_getirme("same-origin"), _getirme("none"), {}):
            belirtec(c.istek("GET", "/_ap-gecis", dict(h, **iyi), "", "", IP))
        # /_ap-giris: yalnızca cihaz alanından yönlenen üst düzey gezinme (same-site; final review C1). Çerçeve/fetch/başka
        # site, AP sayfasının kendisinin gezinmesi (same-origin: A'nın AP'si A'nın belirtecini tarayıcıya ekip B sekmesini
        # kendine çevirirdi — oturum açtırma CSRF'i) ve elle yazılan adres (none) reddedilir; belirteç harcanmaz
        t = belirtec(gecis(c, w))
        for kotu in (_getirme("cross-site"), _getirme("same-site", hedef="iframe"), _getirme("same-site", mod="no-cors", hedef="empty"),
                     _getirme("same-origin"), _getirme("none")):
            r = a.istek("GET", "/_ap-giris", dict(AP, **kotu), "", "t=" + t, IP)
            assert r[0] == 403 and "Set-Cookie" not in r[1], kotu
        wca(a.istek("GET", "/_ap-giris", dict(AP, **_getirme("same-site")), "", "t=" + t, IP))
        wca(giris(a, belirtec(gecis(c, w))))  # Sec-Fetch-* göndermeyen eski tarayıcı geçer (asıl koruma SameSite=Strict + IP)


def test_ayni_ag():
    """Belirtecin adres bağı: IPv4'te /24, IPv6'da /64 (IPv4'e eşlenmiş IPv6 yazımı IPv4 sayılır); adres olmayan değer
    yalnızca kendisiyle aynı (eski tam eşitlik)."""
    for x, y, beklenen in (("203.0.113.5", "203.0.113.5", True), ("203.0.113.5", "203.0.113.254", True),
                           ("203.0.113.5", "203.0.112.5", False), ("10.0.0.1", "10.0.1.1", False),
                           ("2001:db8:1:2::5", "2001:db8:1:2:ffff:ffff:ffff:ffff", True),
                           ("2001:db8:1:2::5", "2001:db8:1:3::5", False), ("::ffff:203.0.113.5", "203.0.113.77", True),
                           ("203.0.113.5", "2001:db8::5", False), ("203.0.113.5", "::cb00:7105", False),
                           ("yerel", "yerel", True), ("yerel", "203.0.113.5", False), (None, "203.0.113.5", False)):
        assert A.ayni_ag(x, y) is beklenen and A.ayni_ag(y, x) is beklenen, (x, y)


def _ap_giris(a, t, ip):
    return a.istek("GET", "/_ap-giris", dict(AP, **_getirme("same-site")), "", "t=" + t, ip)


def test_belirtec_veren_tarayicinin_agina_bagli():
    """Belirteç /_ap-gecis'teki istemci adresinin ağına bağlı (final review C1 "bind the token", re-review I-1): kötü
    niyetli bir AP betiğinin tarayıcıdan sızdırdığı belirteç başka ağdan kullanılamaz, denenen belirteç yanar. cihaz.* ve
    ap.* ayrı bağlantılar: aynı ağdan (IPv4 /24, IPv6 /64) başka çıkış adresi (taşıyıcı NAT havuzu, çift hat) geçer. Ağ
    farklıysa (iCloud Özel Geçiş, VPN) genel "geçersiz" değil, sebebi ve çözümü söyleyen ayrı sayfa (yeniden denemek
    aynı yerde takılırdı)."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        for gecis_ip, giris_ip in (("203.0.113.5", "203.0.113.200"), ("2001:db8:1:2::5", "2001:db8:1:2:aaaa:bbbb:cccc:dddd"),
                                   ("203.0.113.5", "::ffff:203.0.113.9")):
            wca(_ap_giris(a, belirtec(gecis(c, w, ip=gecis_ip)), giris_ip))
        for gecis_ip, giris_ip in (("203.0.113.5", "198.51.100.66"), ("203.0.113.5", "203.0.114.5"),
                                   ("2001:db8:1:2::5", "2001:db8:1:3::5"), ("203.0.113.5", "2001:db8:1:2::5"),
                                   ("203.0.113.5", "yerel")):
            t = belirtec(gecis(c, w, ip=gecis_ip))
            gunluk = io.StringIO()
            with contextlib.redirect_stderr(gunluk):
                r = _ap_giris(a, t, giris_ip)
            s = html.unescape(r[2].decode())
            assert r[0] == 403 and "Set-Cookie" not in r[1] and "text/html" in r[1]["Content-Type"], giris_ip
            assert r[1]["Cache-Control"] == "no-store", giris_ip
            assert ("Bağlantınız farklı bir internet adresinden geldi (ör. iCloud Özel Geçiş, VPN). Özel Geçiş/VPN'i "
                    "kapatıp cihaz panelinden yeniden deneyin.") in s and "geçersiz" not in s, giris_ip
            assert 'href="https://cihaz.wificorrect.com/"' in s, giris_ip
            # teşhis için iki adres de günlükte (merkez journal)
            assert f"AP_GIRIS_RED adres_farkli ip={giris_ip} gecis_ip={gecis_ip}" in gunluk.getvalue(), giris_ip
            # belirteç yandı: aynı ağdan da artık genel "geçersiz" sayfası
            r = _ap_giris(a, t, gecis_ip)
            assert r[0] == 403 and "Set-Cookie" not in r[1] and "geçersiz" in r[2].decode(), giris_ip
        wca(giris(a, belirtec(gecis(c, w))))  # aynı tarayıcıdan (adresten) yenisi açılır


def test_yeni_ap_oturumu_eski_ap_cerezlerini_siler():
    """Bütün müşterilerin AP'leri tek kökte: önceki AP'nin çerezleri (ör. TP-Link'in betikle yazdığı Authorization = Basic
    kullanıcı:parola) sonraki müşterinin AP'sine gitmesin (final review I1). /_ap-giris tarayıcının bu kökte gönderdiği
    (Path=/) çerezleri siler; yalnızca geçerli çerez adları (başlığa enjeksiyon yok), en çok ESKI_CEREZ_EN_COK."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        r = giris(a, belirtec(gecis(c, w)), cerez="wca=eski; SID=abc; Authorization=Basic%20YWRtaW46eA%3D%3D; kötü ad=1; SID=2")
        sc = r[1]["Set-Cookie"]
        assert sc[0].startswith("wca=") and not sc[0].startswith("wca=eski;")
        assert sc[1:] == ["SID=; Path=/; Max-Age=0; Secure", "Authorization=; Path=/; Max-Age=0; Secure"]
        cok = "; ".join(f"c{i}=1" for i in range(A.ESKI_CEREZ_EN_COK + 10))
        assert len(giris(a, belirtec(gecis(c, w)), cerez=cok)[1]["Set-Cookie"]) == 1 + A.ESKI_CEREZ_EN_COK


def test_gecis_hiz_siniri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        for _ in range(C.AP_GECIS_SINIRI):
            belirtec(gecis(c, w))
        r = gecis(c, w)
        assert r[0] == 429 and "Location" not in r[1] and "dakika" in r[2].decode()
        assert len([h for h in v.hareketler(n) if h["olay"] == "AP_GECIS"]) == C.AP_GECIS_SINIRI  # hareket şişmez
        saat.t += 61
        belirtec(gecis(c, w))


# --- /_ap-giris: belirteç süresi, altındaki oturum ---
def test_belirtec_suresi_ve_cihaz_oturumu():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w = wcc_ac(c, v, saat, n)
        t = belirtec(gecis(c, w))
        saat.t += 61
        assert giris(a, t)[0] == 403  # 60 sn geçti
        t = belirtec(gecis(c, w))
        c.istek("GET", "/_cikis", dict(CIHAZ, Cookie=f"wcc={w}"), "", "", IP)  # cihaz panelinden çıkış
        r = giris(a, t)
        assert r[0] == 403 and "Set-Cookie" not in r[1]
        w = wcc_ac(c, v, saat, n)
        t = belirtec(gecis(c, w))
        v.parola_koy(n, "yeni-parola-12")  # belirteç alındıktan sonra parola değişti
        assert giris(a, t)[0] == 403


def test_yeni_giris_eski_ap_oturumunu_kapatir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w, z = ap_ac(c, a, v, saat, n)
        z2 = wca(giris(a, belirtec(gecis(c, w)), cerez=f"wca={z}"))
        assert z2 != z and yetki(a, z)[0] == 403 and yetki(a, z2)[0] == 200


# --- /ap-yetki: wca + altındaki wcc ---
def test_wca_en_cok_30_dakika_ve_cihaz_oturumunu_canli_tutar():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w, z = ap_ac(c, a, v, saat, n)
        bas = saat.t
        saat.t = bas + 1500
        assert yetki(a, z)[0] == 200  # AP kullanımı cihaz paneli oturumunu da canlı tutar (boşta 30 dk)
        saat.t = bas + 1799
        assert yetki(a, z)[0] == 200
        saat.t = bas + 1801
        assert yetki(a, z)[0] == 403  # 30 dk mutlak: wcc hâlâ geçerli olsa da
        assert c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={w}"), "", "", IP)[0] == 200


def test_wca_cihaz_oturumu_dusunce_gecersiz():
    durumlar = {
        "cikis": lambda v, c, n, w: c.istek("GET", "/_cikis", dict(CIHAZ, Cookie=f"wcc={w}"), "", "", IP),
        "parola": lambda v, c, n, w: v.parola_koy(n, "yeni-parola-12"),
        "lisans": lambda v, c, n, w: v.askiya_al(n, True),
        "uyelik": lambda v, c, n, w: v.uyelik(n, "bitti"),
        "bag": lambda v, c, n, w: v.serbest_birak(n, zorla=True),
        "tunel": lambda v, c, n, w: v.cihaz_guncelle(v.bagli_cihaz(n)["id"], tunel_ip="10.99.0.77"),
    }
    for ad, bozan in durumlar.items():
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            v, c, a, saat, n = kur(tmp)
            w, z = ap_ac(c, a, v, saat, n)
            assert yetki(a, z)[0] == 200, ad
            bozan(v, c, n, w)
            y = yetki(a, z)
            assert y[0] == 403 and not any(k.startswith("X-WFC") for k in y[1]), ad


def test_cihaz_degisince_gecersiz_ayni_tunel_adresi_olsa_da():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w, z = ap_ac(c, a, v, saat, n)
        v.serbest_birak(n, zorla=True)
        yeni_cihaz = v.cihaz_bagla(n, WG2, SSH)[0]
        v.cihaz_guncelle(yeni_cihaz, tunel_ip="10.99.0.11")  # yeni cihaz, aynı tünel adresi
        assert c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={w}"), "", "", IP)[0] == 200
        assert yetki(a, z)[0] == 403  # AP oturumu açıldığı cihaza bağlı


def test_baska_musterinin_cihazina_gidemez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        n2 = musteri_ac(v, "musteri-parola-2", WG2, "10.99.0.12")
        _, z = ap_ac(c, a, v, saat, n)
        _, z2 = ap_ac(c, a, v, saat, n2)
        y, y2 = yetki(a, z), yetki(a, z2)
        assert (y[1]["X-WFC-Cihaz"], y[1]["X-WFC-AP-Kullanici"]) == ("10.99.0.11", str(n))
        assert (y2[1]["X-WFC-Cihaz"], y2[1]["X-WFC-AP-Kullanici"]) == ("10.99.0.12", str(n2))
        # müşteri 1 cihazsız kalınca onun AP oturumu hiçbir cihaza gitmez (başkasınınkine de)
        v.serbest_birak(n, zorla=True)
        assert yetki(a, z)[0] == 403 and yetki(a, z2)[1]["X-WFC-Cihaz"] == "10.99.0.12"


def test_yetkisiz_sayfa():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        y = yetki(a, "")
        assert y[0] == 403 and "text/html" in y[1]["Content-Type"] and "Set-Cookie" not in y[1]
        assert not any(k.startswith("X-WFC") for k in y[1])
        s = y[2].decode()
        assert "AP erişimi kapalı" in s and "AP kurulumu" in s and "https://cihaz.wificorrect.com/" in s
        # temizleyerek çıkış bağlantısı (aynı kökte; tarayıcıdaki AP verisi silinir)
        assert 'href="/_ap-cikis"' in s
        # çerçeveli AP arayüzlerinde (TP-Link userRpm frameset) sayfa çerçevenin içinde görünür ve bağlantılar üst pencereyi
        # açar (re-review m1): çerçevedeki gezinme Sec-Fetch-Dest: frame olur, /_ap-cikis onu reddeder; cihaz paneli
        # (frame-ancestors 'none') çerçevede boş görünür
        assert '<a href="/_ap-cikis" target="_top">' in s and '<a href="https://cihaz.wificorrect.com/" target="_top">' in s
        # yalnızca aynı kökenden çerçevelenebilir (aynı kökendeki AP betiği bu adrese zaten gidebilir; sayfada sır yok);
        # merkezin diğer cevaplarında DENY kalır (test_merkez)
        assert y[1]["X-Frame-Options"] == "SAMEORIGIN"
        assert y[1]["Content-Security-Policy"] == dict(MZ.GUVENLIK_BASLIKLARI)["Content-Security-Policy"] + "; frame-ancestors 'self'"
        y = yetki(a, "uydurma")
        assert y[0] == 403 and "Max-Age=0" in y[1]["Set-Cookie"] and "SameSite=Strict" in y[1]["Set-Cookie"]  # bozuk/eskimiş çerez silinir
        # cihaz paneli çerezi (wcc) AP alanında geçmez
        w = wcc_ac(c, v, saat, n)
        assert a.istek("GET", "/ap-yetki", dict(AP, Cookie=f"wcc={w}"), "", "", IP)[0] == 403


def test_cikis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        w, z = ap_ac(c, a, v, saat, n)
        # başka siteden (ya da çerçeve/resim olarak) çıkış yok: her şeyi silen çıkış oturum kapatma CSRF'i olmasın
        for kotu in (_getirme("cross-site"), _getirme("same-origin", hedef="image", mod="no-cors"), _getirme("same-site", hedef="iframe")):
            r = a.istek("GET", "/_ap-cikis", dict(AP, Cookie=f"wca={z}", **kotu), "", "", IP)
            assert r[0] == 403 and "Set-Cookie" not in r[1] and "Clear-Site-Data" not in r[1], kotu
        assert yetki(a, z)[0] == 200
        r = a.istek("GET", "/_ap-cikis", dict(AP, Cookie=f"wca={z}", **_getirme("same-origin")), "", "", IP)
        assert r[0] == 303 and r[1]["Location"] == "https://cihaz.wificorrect.com/"
        assert r[1]["Set-Cookie"].startswith("wca=;") and "Max-Age=0" in r[1]["Set-Cookie"]
        # çıkış tarayıcıdaki AP verisini de siler (final review C1/I1): AP'lerin çerezleri, HTTP kimlik bilgileri, depolama ve
        # Service Worker'lar. "cookies" kayıtlı alan adının bütün çerezlerini siler: panel/yönetim/cihaz oturumları da kapanır
        assert r[1]["Clear-Site-Data"] == '"cache", "cookies", "storage"'
        assert yetki(a, z)[0] == 403
        # sunucudaki cihaz paneli oturumu düşmez (tarayıcı Clear-Site-Data'yı uygulamazsa panel açık kalır)
        assert c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={w}"), "", "", IP)[0] == 200
        # dönüş yalnızca cihaz.wificorrect.com'da bir yol
        for donus, yer in (("/ap-kurulum", "https://cihaz.wificorrect.com/ap-kurulum"),
                           ("//kotu.example", "https://cihaz.wificorrect.com/"),
                           ("https://kotu.example/", "https://cihaz.wificorrect.com/"),
                           ("/\\kotu.example", "https://cihaz.wificorrect.com/"),
                           ("/%0d%0aSet-Cookie:%20x=1", "https://cihaz.wificorrect.com/"),  # başlık enjeksiyonu yok
                           ("/%C5%9Fube", "https://cihaz.wificorrect.com/%C5%9Fube")):  # Location latin-1'e sığmalı
            r = a.istek("GET", "/_ap-cikis", AP, "", "donus=" + donus, IP)
            assert r[1]["Location"] == yer, donus


def test_alan_adi_ayrimi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, c, a, saat, n = kur(tmp)
        mz = MZ.Merkez({"cihaz": c, "ap": a})
        w, z = ap_ac(c, a, v, saat, n)
        y = mz.istek("GET", "/ap-yetki", {"Host": "ap.wificorrect.com", "Cookie": f"wca={z}"}, "", "", IP)
        assert y[0] == 200 and y[1]["X-WFC-AP"] == "1"
        # AP uçları cihaz alanında, cihaz uçları AP alanında yok
        assert mz.istek("GET", "/ap-yetki", {"Host": "cihaz.wificorrect.com", "Cookie": f"wca={z}"}, "", "", IP)[0] == 404
        assert mz.istek("GET", "/_ap-gecis", {"Host": "ap.wificorrect.com", "Cookie": f"wcc={w}"}, "", "", IP)[0] == 404
        assert mz.istek("GET", "/cihaz-yetki", {"Host": "ap.wificorrect.com", "Cookie": f"wcc={w}"}, "", "", IP)[0] == 404
        assert mz.istek("GET", "/_giris", {"Host": "ap.wificorrect.com"}, "", "t=x", IP)[0] == 404
        assert MZ.ALANLAR["ap.wificorrect.com"] == "ap"


# --- Caddyfile ---
def _yorumsuz(metin):
    return "\n".join(s for s in metin.splitlines() if not s.strip().startswith("#"))


def caddy_blogu(metin, ad):
    """Üst düzey `ad {` bloğunun içi (yorumlar atılmış; {yer.tutucu}lar aynı satırda dengeli)."""
    metin = _yorumsuz(metin)
    m = re.search(r"^" + re.escape(ad) + r" \{$", metin, re.M)
    assert m, ad
    derinlik = 0
    for i in range(m.end() - 1, len(metin)):
        if metin[i] == "{":
            derinlik += 1
        elif metin[i] == "}":
            derinlik -= 1
            if derinlik == 0:
                return metin[m.end():i]
    raise AssertionError(f"{ad}: kapanmayan blok")


def _cerez_regex(blok):
    m = re.search(r'header_up Cookie "([^"]+)" "([^"]*)"', blok)
    assert m and m.group(2) == "_"  # boş değiştirme Caddy 2.6.2'de Cookie'yi ezer (KURULUM_GUNLUGU "Uzak panel")
    return re.compile(m.group(1))  # Go RE2 ile aynı anlam (ASCII çerez başlığında \b, (?:…), [^;])


def test_caddyfile_ap_blogu():
    metin = open(CADDYFILE, encoding="utf-8").read()
    yorumsuz = _yorumsuz(metin)
    assert yorumsuz.count("{") == yorumsuz.count("}")
    ap = caddy_blogu(metin, "ap.wificorrect.com")
    for b in ("X-WFC-Kullanici", "X-WFC-AP-Kullanici", "X-WFC-Cihaz", "X-WFC-AP"):
        assert f"request_header -{b}\n" in ap, b  # istemcinin gönderdiği kimlik başlıkları geçmez
    # Service Worker betiği (kayıt ve güncelleme isteği "Service-Worker: script" taşır) hiçbir yere gitmez (final review C1);
    # adlı eşleyicili handle'lar yazıldığı sırayla denenir: ilk o
    sw = re.search(r"@sw header Service-Worker script\n\s*handle @sw \{\n\s*respond 403\n\s*\}", ap)
    assert sw
    assert sw.start() < ap.index("handle @merkez {") < ap.index("handle {")
    assert re.search(r"@merkez path /_ap-giris /_ap-cikis\n\s*handle @merkez \{\n\s*reverse_proxy 127\.0\.0\.1:8081\n", ap)
    assert re.search(r"forward_auth 127\.0\.0\.1:8081 \{\n\s*uri /ap-yetki\n\s*copy_headers X-WFC-AP-Kullanici X-WFC-Cihaz X-WFC-AP\n", ap)
    assert "reverse_proxy {http.request.header.X-WFC-Cihaz}:8443 {" in ap
    assert re.search(r"transport http \{\n\s*tls\n\s*tls_insecure_skip_verify\n\s*\}", ap)
    assert "header_up -X-WFC-Cihaz\n" in ap
    assert "header_up Host" not in ap  # Host ap.wificorrect.com kalır
    # AP'nin kendi arayüzünü bozacak başlıklar bilerek yok (satır içi betik, çerçeve, yanlış MIME türü, Referer denetimi)
    for b in ("Content-Security-Policy", "X-Frame-Options", "X-Content-Type-Options", "Referrer-Policy"):
        assert b not in ap, b
    assert 'Strict-Transport-Security "max-age=31536000"' in ap and "-Server" in ap
    r = _cerez_regex(ap)
    for once, sonra in (("wca=abc; SessionID=xyz", "_; SessionID=xyz"),
                        ("SessionID=xyz; wca=abc", "SessionID=xyz; _"),
                        ("wcc=1; wca=2; Authorization=Basic%20YWRtaW46YWRtaW4%3D", "_; _; Authorization=Basic%20YWRtaW46YWRtaW4%3D"),
                        ("xwca=1; wcax=2; _wca=3", "xwca=1; wcax=2; _wca=3")):
        assert r.sub("_", once) == sonra, once


def test_caddyfile_cihaz_blogu_ap_gecisi():
    metin = open(CADDYFILE, encoding="utf-8").read()
    cz = caddy_blogu(metin, "cihaz.wificorrect.com")
    assert re.search(r"@merkez path /_giris /_cikis /_ap-gecis\n", cz)  # forward_auth'suz merkeze
    for b in ("X-WFC-Kullanici", "X-WFC-AP-Kullanici", "X-WFC-Cihaz", "X-WFC-AP"):
        assert f"request_header -{b}\n" in cz, b  # AP kipi başlıkları cihaz alanından cihaza asla ulaşmaz
    assert "copy_headers X-WFC-Kullanici X-WFC-Cihaz\n" in cz  # cihaz-yetki AP başlığı vermez
    csp = re.search(r'header_down Content-Security-Policy "([^"]+)"', cz).group(1)  # cihaz panelinin sıkı CSP'si yerinde
    assert "default-src 'none'" in csp and "frame-ancestors 'none'" in csp
    # form-action yönlendirmelere de uygulanır: "AP arayüzüne git" formu → /_ap-gecis → ap.wificorrect.com
    form_action = re.search(r"form-action ([^;]+)", csp).group(1).split()
    assert {"'self'", "https://panel.wificorrect.com", "https://yonetim.wificorrect.com", "https://ap.wificorrect.com"} == set(form_action)
    r = _cerez_regex(cz)
    assert r.sub("_", "wcc=abc; wfc=def") == "_; wfc=def"


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
