import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import detay as D  # noqa: E402

A = "5330000001"


def kisi(tel, ad, soyad, **ek):
    k = {"telefon": tel, "ad": ad, "soyad": soyad, "mac": ["aa:aa:aa:aa:aa:01"], "oturumlar": [],
         "gonderilen": 1536, "alinan": 1048576, "baglanti_sayisi": 1, "ilk": "2026-09-27T09:00:00+03:00",
         "son": "2026-09-27T17:05:00+03:00", "siteler": [], "diger_site_sayisi": 0, "baglantilar": []}
    k.update(ek)
    return k


def test_bicimler():
    assert D.boyut(0) == "0 B" and D.boyut(1536) == "1,5 KB" and D.boyut(1048576) == "1,0 MB" and D.boyut("x") == "0 B"
    assert D.saat("2026-09-27T09:05:00+03:00") == "09:05" and D.saat("") == "" and D.saat(None) == ""
    assert D.sure(59) == "0:59" and D.sure(3725) == "1:02:05"
    assert D.tarih("2026-09-27") == "27.09.2026" and D.tarih("bozuk") == "bozuk"
    assert D.gecerli_gun("2026-09-27") and not D.gecerli_gun("2026-13-45") and not D.gecerli_gun("../x")
    assert D.tr_kucuk("İLKER ŞAHIN") == "ilker şahın"


def test_cihaz_metni_her_yerde_kacisli():
    kotu = '"><script>x</script><img src=x '
    k = kisi('"><img>' + A, kotu, kotu, mac=[kotu], ilk="2026-09-27T<!-- x", son="2026-09-27T<img ",
             oturumlar=[{"olay": kotu, "zaman": "2026-09-27T<u>9:0", "ic_ip": kotu, "ek": kotu}],
             siteler=[{"alan": kotu, "sayi": 1, "ilk": "2026-09-27T<i>x", "son": ""}],
             baglantilar=[{"zaman": "", "protokol": kotu, "hedef_ip": kotu, "hedef_port": kotu}])
    g = D.kisi_html(k, b"s") + D.ara_icerik("x", [('"><img>' + A, {"ad": kotu, "soyad": "", "gunler": ["2026-09-27"]})], b"s")
    for parca in ("<script", "<img", "<!--", "<u>", "<i>"):
        assert parca not in g, parca


def test_telefon_aramasi_bicimleri():
    dizin = {A: {"ad": "Ayşe", "soyad": "Yılmaz", "gunler": ["2026-09-27"]}}
    for sorgu in (A, "0533 000 0001", "+90 533 000 00 01", "905330000001", "05330000001", "533", "0533"):
        assert [t for t, _ in D.ara_sonuclari(dizin, sorgu)] == [A], sorgu
    assert D.ara_sonuclari(dizin, "ayşe") and not D.ara_sonuclari(dizin, "0544")


def test_tarih_gosterimi_ve_girdisi():
    import common as C
    assert C.tarih_goster("2026-10-12") == "12.10.2026"
    assert C.tarih_goster("2026-10-12T17:38:05+03:00") == "12.10.2026 17:38"
    assert C.tarih_goster("2026-10-12 17:38") == "12.10.2026 17:38"
    assert C.tarih_saat_goster("2026-10-12T17:38:05+03:00") == "12.10.2026 17:38:05"
    assert C.tarih_saat_goster("2026-10-12 17:38") == "12.10.2026 17:38" and C.tarih_saat_goster("2026-10-12") == "12.10.2026"
    assert C.tarih_goster(None) == "" and C.tarih_goster("") == ""
    for bozuk in ("dün", "2026-1-12", "12.10.2026", "2026-10-12X", "2026-10-12T17", "abcd-ef-gh", "٢٠٢٦-١٠-١٢"):
        assert C.tarih_goster(bozuk) == bozuk and C.tarih_saat_goster(bozuk) == bozuk, bozuk
    assert C.tarih_girdi("12.10.2026") == "2026-10-12" and C.tarih_girdi(" 1.9.2026 ") == "2026-09-01"
    assert C.tarih_girdi("12.10.2026 17:38") == "2026-10-12 17:38"
    assert C.tarih_girdi("2026-10-12") == "2026-10-12"  # eski yazım aynen
    for bozuk in ("dün", "12.10.26", "123.10.2026", "12.10", ""):
        assert C.tarih_girdi(bozuk) == bozuk, bozuk
    assert C.tarih_girdi(None) == ""


def test_gunler_ve_baslik_gun_ay_yil():
    s = D.gunler_html(["2026-09-30"], {}, "")
    assert ">30.09.2026 Çar<" in s and 'href="/gun/2026-09-30"' in s
    assert "<h1>30.09.2026 Çarşamba</h1>" in D.gun_icerik("2026-09-30", {"kisiler": []}, b"s")


def test_onek_butun_baglantilarda():
    on = "/m/100001/kayitlar"
    assert f'action="{on}/ara"' in D.arama_formu("x", on)
    assert f'href="{on}/gun/2026-09-30"' in D.gunler_html(["2026-09-30"], {}, on)
    assert f'href="{on}/"' in D.gun_icerik("2026-09-30", {"kisiler": []}, b"s", on)
    s = D.ara_icerik("ay", [("5330000001", {"ad": "Ayşe", "gunler": ["2026-09-30"]})], b"s", on)
    capa = "k" + D.opak(b"s", "5330000001")
    assert f'href="{on}/gun/2026-09-30#{capa}"' in s and f'href="{on}/"' in s and f'action="{on}/ara"' in s
    assert f'id="{capa}"' in D.kisi_html({"telefon": "5330000001"}, b"s") and "#k5330000001" not in s
    assert 'href="/gun/' not in s and 'href="/"' not in s


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
