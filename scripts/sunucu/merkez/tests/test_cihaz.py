import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import cihaz as C  # noqa: E402
import guvenlik as G  # noqa: E402
import musteri as M  # noqa: E402

WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"
CIHAZ = {"Host": "cihaz.wificorrect.com"}


def kur(tmp):
    v, k, saat = Y.ortam(tmp)
    gecis = G.GecisBelirtecleri()
    m = M.Musteri(v, k, saat, gecis=gecis)
    c = C.Cihaz(v, gecis, m, saat)
    n, pw = v.musteri_ekle(parola="musteri-parola-1")
    v.cihaz_bagla(n, WG, SSH)
    v.cihaz_guncelle(v.bagli_cihaz(n)["id"], tunel_ip="10.99.0.11")
    return v, m, c, saat, n


def gir(m, n, pw="musteri-parola-1"):
    return Y.giris(m, str(n), pw)


def cerez(yanit, ad):
    return yanit[1]["Set-Cookie"].split(";")[0].split("=", 1)[1]


def test_belirtec_tek_kullanim():
    g = G.GecisBelirtecleri()
    t = g.uret("1234567", 1000.0, "oz")
    assert g.tuket(t, 1030.0) == ("1234567", "oz", "musteri") and g.tuket(t, 1031.0) is None
    t2 = g.uret("1234567", 1000.0)
    assert g.tuket(t2, 1061.0) is None and g.tuket("yok", 1000.0) is None


def test_giris_cihaza_gecirir_ve_yetki_baslik_verir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, m, c, saat, n = kur(tmp)
        r = gir(m, n)
        assert r[0] == 303 and r[1]["Location"] == "/cihaz"
        cz = Y.cerez(r)
        r = m.istek("GET", "/cihaz", {"Cookie": cz}, "", "", "203.0.113.5")
        assert r[0] == 303 and r[1]["Location"].startswith("https://cihaz.wificorrect.com/_giris?t=")
        t = r[1]["Location"].split("t=", 1)[1]
        r = c.istek("GET", "/_giris", CIHAZ, "", "t=" + t, "203.0.113.5")
        assert r[0] == 303 and r[1]["Location"] == "/" and "SameSite=Lax" in r[1]["Set-Cookie"]
        wcc = cerez(r, "wcc")
        r = c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={wcc}"), "", "", "203.0.113.5")
        assert r[0] == 200 and r[1]["X-WFC-Cihaz"] == "10.99.0.11" and r[1]["X-WFC-Kullanici"] == str(n)
        assert c.istek("GET", "/_giris", CIHAZ, "", "t=" + t, "203.0.113.5")[0] == 303  # ikinci kez: giriş sayfasına
        r = c.istek("GET", "/cihaz-yetki", CIHAZ, "", "", "203.0.113.5")
        assert r[0] == 302 and r[1]["Location"] == "https://panel.wificorrect.com/giris"


def test_belirtec_parola_degisirse_giris_sayfasina():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, m, c, saat, n = kur(tmp)
        r = m.istek("GET", "/cihaz", {"Cookie": Y.cerez(gir(m, n))}, "", "", "203.0.113.5")
        t = r[1]["Location"].split("t=", 1)[1]
        v.parola_koy(n, "yeni-parola-12")  # belirteç alındıktan sonra
        r = c.istek("GET", "/_giris", CIHAZ, "", "t=" + t, "203.0.113.5")
        assert r[0] == 303 and r[1]["Location"] == "https://panel.wificorrect.com/giris" and "Set-Cookie" not in r[1]


def test_cihaz_yok_sorgusu_yonlendirmez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, m, c, saat, n = kur(tmp)
        r = m.istek("GET", "/cihaz", {"Cookie": Y.cerez(gir(m, n))}, "", "yok=1", "203.0.113.5")
        assert r[0] == 200 and "Location" not in r[1] and "ulaşılamıyor" in r[2].decode()


def test_cihaz_oturumu_parola_lisans():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, m, c, saat, n = kur(tmp)
        wcc = c.oturumlar.create(str(n), "musteri", saat(), v.musteri(n)["ozet"])
        yetki = lambda: c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")[0]
        assert yetki() == 200
        v.askiya_al(n, True)
        assert yetki() == 302  # lisans kapalı
        v.askiya_al(n, False)
        v.parola_koy(n, "yeni-parola-12")
        assert yetki() == 302  # parola değişti: oturum düştü
        wcc = c.oturumlar.create(str(n), "musteri", saat(), v.musteri(n)["ozet"])
        v.serbest_birak(n, zorla=True)
        assert yetki() == 302  # cihaz bağlı değil


def test_cikis_iki_oturumu_kapatir_ve_cihazsiz_musteri_merkezde_kalir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, m, c, saat, n = kur(tmp)
        cz = Y.cerez(gir(m, n))
        wcc = c.oturumlar.create(str(n), "musteri", saat(), v.musteri(n)["ozet"])
        r = c.istek("GET", "/_cikis", dict(CIHAZ, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")
        assert r[0] == 303 and r[1]["Location"] == "https://panel.wificorrect.com/giris" and "Max-Age=0" in r[1]["Set-Cookie"]
        assert c.istek("GET", "/cihaz-yetki", dict(CIHAZ, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")[0] == 302
        assert m.istek("GET", "/", {"Cookie": cz}, "", "", "1.2.3.4")[0] == 303  # merkez oturumu da kapandı → giriş
        n2, _ = v.musteri_ekle(parola="musteri-parola-2")  # cihazı yok
        r = gir(m, n2, "musteri-parola-2")
        assert r[1]["Location"] == "/baglan"  # kolay bağlama (spec 2026-10-16)
        assert m.istek("GET", "/cihaz", {"Cookie": Y.cerez(r)}, "", "", "1.2.3.4")[1]["Location"] == "/baglan"
        v.uyelik(n2, "bitti")  # üyeliği biten cihazsız müşteri merkezde (yedek arşiv) kalır
        r = gir(m, n2, "musteri-parola-2")
        assert r[1]["Location"] == "/"
        s = m.istek("GET", "/cihaz", {"Cookie": Y.cerez(r)}, "", "", "1.2.3.4")[2].decode()
        assert "ulaşılamıyor" in s


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
