import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import durum as D  # noqa: E402

PUB = "A" * 43 + "="


def ortam(tmp):
    kayit = os.path.join(tmp, "kayit.csv")
    with open(kayit, "w", encoding="utf-8") as f:
        f.write(f"yonetici;pc;10.99.0.2;{PUB};\ncihaz;100001;10.99.0.10;{PUB};wfc-100001\n"
                f"bitti;bocafe-test;10.99.0.11;{'B' * 43}=;wfc-bocafe-test;2026-10-04\n")
    yeni, eski = os.path.join(tmp, "yeni"), os.path.join(tmp, "eski")
    os.makedirs(os.path.join(yeni, "100001", "veri", "gunluk", "2026-10-03"))
    os.makedirs(os.path.join(yeni, "bocafe-test", "veri", "gunluk", "2026-10-01"))
    os.makedirs(os.path.join(yeni, "..kotu", "veri"))
    os.makedirs(os.path.join(eski, "bocafe", "gunluk", "2026-09-20"))
    with open(os.path.join(yeni, "100001", "veri", "zincir.txt"), "w") as f:
        f.write("x" * 100)
    return kayit, yeni, eski


def test_arsivler():
    with tempfile.TemporaryDirectory() as tmp:
        kayit, yeni, eski = ortam(tmp)
        a = D.arsivler(kayit, yeni, eski)
        assert sorted(a) == ["100001", "bocafe", "bocafe-test"]
        assert a["100001"] == {"tur": "cihaz", "pub": PUB, "kaynaklar": [os.path.join(yeni, "100001", "veri")]}
        assert a["bocafe-test"]["tur"] == "bitti" and a["bocafe"]["tur"] == "eski"


def test_yardimcilar():
    assert D.el_sikismalari(f"{PUB}\t1790000000\nbozuk\n{'B' * 43}=\t0\n") == {PUB: 1790000000}
    with tempfile.TemporaryDirectory() as tmp:
        kayit, yeni, eski = ortam(tmp)
        k = [os.path.join(yeni, "100001", "veri")]
        assert D.son_gun(k) == "2026-10-03" and D.son_gun([os.path.join(tmp, "yok")]) is None
        assert D.boyut(k) == 100


def test_main():
    with tempfile.TemporaryDirectory() as tmp:
        kayit, yeni, eski = ortam(tmp)
        cikti = os.path.join(tmp, "durum.json")
        D.main(kayit, yeni, eski, cikti, wg=f"{PUB}\t1790000000\n", simdi=1_790_000_100.0, grup=None)
        with open(cikti, encoding="utf-8") as f:
            d = json.load(f)
        assert d["arsivler"]["100001"] == {"tur": "cihaz", "son_gun": "2026-10-03", "boyut": 100, "el_sikisma": 1790000000}
        assert d["arsivler"]["bocafe"]["el_sikisma"] is None and d["zaman"].startswith("2026-")
        assert d["wg_ok"] is True
        onceki = D._wg
        D._wg = lambda: None  # `wg show` başarısız
        try:
            D.main(kayit, yeni, eski, cikti, simdi=1_790_000_100.0, grup=None)
        finally:
            D._wg = onceki
        with open(cikti, encoding="utf-8") as f:
            d = json.load(f)
        assert d["wg_ok"] is False and d["arsivler"]["100001"]["el_sikisma"] is None


def test_wg_basarisi():
    class Sonuc:
        def __init__(self, kod, cikti):
            self.returncode, self.stdout = kod, cikti

    assert D._wg(lambda *a, **k: Sonuc(0, f"{PUB}\t1790000000\n")) == f"{PUB}\t1790000000\n"
    assert D._wg(lambda *a, **k: Sonuc(0, "")) == ""  # çalıştı, hiç eş yok
    assert D._wg(lambda *a, **k: Sonuc(1, "")) is None  # arayüz yok / yetki yok

    def patlar(*a, **k):
        raise OSError("wg yok")
    assert D._wg(patlar) is None


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
