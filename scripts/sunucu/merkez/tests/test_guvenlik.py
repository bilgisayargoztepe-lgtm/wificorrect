import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import guvenlik as G  # noqa: E402


def test_ozet_cihazla_ayni():
    assert G.ozet("parola-12345", "a1b2c3d4", 120000) == "ee38d08b3d6573ecc281263ad6259d2e7ee39f37d6b8958b3180906580a44495"


def test_yeni_kayit_ve_dogrulama():
    tuz, oz, y = G.yeni_kayit("parola-12345")
    assert len(tuz) == 32 and y == 120000 and G.dogru("parola-12345", tuz, oz, y)
    assert not G.dogru("parola-12346", tuz, oz, y) and not G.dogru(None, tuz, oz, y)


def test_parola_uret_karisan_harf_yok():
    for _ in range(200):
        p = G.parola_uret()
        assert len(p) == 12 and not set(p) & set("0O1lI8B")


def test_oturum_gerekce_ve_sure():
    s = G.Oturumlar()
    t = s.create("100001", "musteri", 100.0)
    assert s.get(t, 100.0)["gerekce"] is None
    s.set_gerekce(t, "Müşteri şikâyeti", 1900.0)
    assert s.get(t, 101.0)["gerekce"] == ("Müşteri şikâyeti", 1900.0)
    assert s.get(t, 101.0 + 1801) is None  # boşta 30 dk


def test_kilit_ve_hiz():
    k = G.GirisKilidi()
    assert all(k.attempt("1.1.1.1", "100001", 0) for _ in range(5))
    assert not k.attempt("1.1.1.1", "100001", 1) and not k.attempt("2.2.2.2", "100001", 1)  # hesap kilitli
    assert k.attempt("2.2.2.2", "100002", 1)
    assert k.attempt("1.1.1.1", "100001", 901)  # 15 dk sonra açılır
    h = G.HizSiniri(3, 60)
    assert [h.izin("ip", t) for t in (0, 1, 2, 3)] == [True, True, True, False]
    assert h.izin("ip", 61)


def test_hiz_siniri_eski_anahtarlari_unutur():
    h = G.HizSiniri(1, 60)
    assert h.izin("a", 0) and h.izin("b", 1) and not h.izin("a", 2)
    assert h.izin("c", 200)  # pencereden eski anahtarlar bellekten silinir
    assert set(h._z) == {"c"}


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
