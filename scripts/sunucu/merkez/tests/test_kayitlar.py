import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import kayitlar as K  # noqa: E402
import detay as D  # noqa: E402


def test_birden_cok_arsiv_birlesir_ilki_gecerli():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        _, k, saat = Y.ortam(tmp)
        dun = K.son_30_gun(saat())[0].isoformat()
        Y.sayi_yaz(tmp, "100001", {dun: 5})
        Y.sayi_yaz(tmp, "bocafe", {dun: 99, "2026-09-01": 7})
        assert k.sayilar(["100001", "bocafe"])[0] == {dun: 5, "2026-09-01": 7}
        Y.gun_yaz(tmp, "100001", "2026-09-20", dizin={"5330000001": {"ad": "Ayşe", "gunler": ["2026-09-20"]}})
        Y.gun_yaz(tmp, "bocafe", "2026-09-20", dizin={"5330000001": {"ad": "Ayşe", "gunler": ["2026-09-01"]}})
        Y.gun_yaz(tmp, "bocafe", "2026-09-01")
        assert k.gunler(["100001", "bocafe"]) == {"2026-09-20": "100001", "2026-09-01": "bocafe"}
        s = k.ara_html(["100001", "bocafe"], {"q": "ayşe"}, "/m/100001/kayitlar")
        capa = "k" + D.opak(k.sir(), "5330000001")
        assert f'href="/m/100001/kayitlar/gun/2026-09-20#{capa}"' in s and f"/gun/2026-09-01#{capa}" in s
        assert 'method="post"' in k.ara_html(["100001"], None, "") and "Ayşe" not in k.ara_html(["100001"], None, "")


def test_ozet_gun_ve_bilinmeyen():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        _, k, saat = Y.ortam(tmp)
        dun = K.son_30_gun(saat())[0].isoformat()
        Y.sayi_yaz(tmp, "100001", {dun: 14})
        Y.gun_yaz(tmp, "100001", "2026-09-20", [{"telefon": "5330000001", "ad": "<b>x</b>"}])
        s = k.ozet_html("Bocafe <Göztepe>", ["100001"], saat(), "")
        assert "Bocafe &lt;Göztepe&gt;" in s and ">14<" in s and 'href="/gun/2026-09-20"' in s
        assert "&lt;b&gt;x&lt;/b&gt;" in k.gun_html(["100001"], "2026-09-20", "")
        assert k.gun_html(["100001"], "2026-09-21", "") is None
        assert k.gun_html(["100001"], "../../etc", "") is None
        assert "Henüz veri yok" in k.ozet_html("X", ["yok"], saat(), "")


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
