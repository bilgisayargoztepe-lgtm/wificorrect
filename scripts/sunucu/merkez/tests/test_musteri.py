import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import kayitlar  # noqa: E402
import musteri as M  # noqa: E402

WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"


def kur(tmp):
    v, k, saat = Y.ortam(tmp)
    return M.Musteri(v, k, saat), v, saat


def test_giris_ve_kendi_kayitlari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat = kur(tmp)
        n, pw = v.musteri_ekle()
        n2, _ = v.musteri_ekle()
        dun = kayitlar.son_30_gun(saat())[0].isoformat()
        Y.sayi_yaz(tmp, str(n), {dun: 14})
        Y.sayi_yaz(tmp, str(n2), {dun: 777})
        assert Y.giris(app, str(n), "yanlis")[0] == 401
        assert Y.giris(app, "abc", pw)[0] == 401
        c = Y.cerez(Y.giris(app, str(n), pw))
        assert c.startswith("wc_musteri=")
        s = Y.al(app, "/", c)[2].decode()
        assert ">14<" in s and "777" not in s and f"Müşteri {n}" in s
        assert "<header>" in s and 'id="menu-ac"' not in s  # müşteri paneli üst şeritte (sol menü yalnızca yönetimde)
        cid, _ = v.cihaz_bagla(n, WG, SSH, "Bocafe Göztepe")
        assert "Bocafe Göztepe" in Y.al(app, "/", c)[2].decode()
        assert v.musteri(n)["son_giris"] and [h["olay"] for h in v.hareketler(n)] == ["GIRIS"]


def test_gerekce_olmadan_kisi_verisi_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat = kur(tmp)
        n, pw = v.musteri_ekle()
        Y.gun_yaz(tmp, str(n), "2026-09-20", [{"telefon": "5330000001", "ad": "Ayşe"}])
        c = Y.cerez(Y.giris(app, str(n), pw))
        r = Y.al(app, "/gun/2026-09-20", c)
        assert r[0] == 303 and r[1]["Location"] == "/gerekce?donus=%2Fgun%2F2026-09-20"
        assert Y.al(app, "/ara", c)[0] == 200 and "Ayşe" not in Y.al(app, "/ara?q=ay", c)[2].decode()  # GET aramaz
        r = Y.gonder(app, "/ara", c, {"csrf": Y.csrf(Y.al(app, "/ara", c)), "q": "ay"})
        assert r[0] == 303 and r[1]["Location"] == "/gerekce?donus=%2Fara"  # POST arama gerekçe ister
        assert Y.gerekce_ver(app, c, "x")[0] == 400
        assert Y.gerekce_ver(app, c)[0] == 303
        assert "Ayşe" in Y.al(app, "/gun/2026-09-20", c)[2].decode()
        olaylar = [h["olay"] for h in v.hareketler(n)]
        assert olaylar[:2] == ["GORUNTULEME", "GEREKCE"]
        saat.t += 1801
        c = Y.cerez(Y.giris(app, str(n), pw))
        assert Y.al(app, "/gun/2026-09-20", c)[0] == 303  # yeni oturum, gerekçe yok


def test_parola_degistirme_ve_uyelik_bitti_girer():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        v.uyelik(n, "bitti")
        c = Y.cerez(Y.giris(app, str(n), pw))  # üyelik bitse de kayıtlarına bakar
        f = Y.al(app, "/parola", c)
        assert Y.gonder(app, "/parola", c, {"csrf": Y.csrf(f), "mevcut": "yanlis", "yeni": "yeni-parola-1", "tekrar": "yeni-parola-1"})[0] == 400
        assert Y.gonder(app, "/parola", c, {"csrf": Y.csrf(f), "mevcut": pw, "yeni": "yeni-parola-1", "tekrar": "baska"})[0] == 400
        r = Y.gonder(app, "/parola", c, {"csrf": Y.csrf(f), "mevcut": pw, "yeni": "yeni-parola-1", "tekrar": "yeni-parola-1"})
        assert r[0] == 200 and "06:00" in r[2].decode() and v.parola_dogrula(n, "yeni-parola-1")
        assert Y.al(app, "/", c)[0] == 303  # eski oturum kapandı
        assert Y.al(app, "/", Y.cerez(r))[0] == 200


def test_baska_musterinin_yollari_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        c = Y.cerez(Y.giris(app, str(n), pw))
        for yol in ("/m/100002/", "/m/100001/kayitlar/", "/hareketler", "/yeni"):
            assert Y.al(app, yol, c)[0] == 404, yol


def test_unicode_rakamli_numara_reddedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        assert Y.giris(app, "1\uff100001", pw)[0] == 401
        assert Y.giris(app, "\u00b2" * 7, pw)[0] == 401
        assert Y.giris(app, str(n)[:6], pw)[0] == 401  # 6 hane


def test_parola_sifirlaninca_eski_oturum_duser():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        c = Y.cerez(Y.giris(app, str(n), pw))
        assert Y.al(app, "/", c)[0] == 200
        v.parola_sifirla(n)
        assert Y.al(app, "/", c)[0] == 303


def test_gerekce_donusu_baska_siteye_gitmez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        c = Y.cerez(Y.giris(app, str(n), pw))
        f = Y.al(app, "/gerekce", c)
        for kotu in ("/\t/evil.example", "//evil.example", "/\\evil.example", "/\n/x", "https://evil.example", "/ /evil.example"):
            r = Y.gonder(app, "/gerekce", c, {"csrf": Y.csrf(f), "gerekce": "Müşteri şikâyeti", "donus": kotu})
            assert r[1]["Location"] == "/", kotu


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
