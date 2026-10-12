"""Merkez: işletme türü, misafir kimliklerinin maskelenmesi, opak kişi kimliği ve TC/pasaport/oda araması (POST).
TC/pasaport değerleri uydurmadır (algoritmaya uyan 10000000146 ve 11111111110)."""
import os
import re
import sys
import tempfile

os.environ["WC_TEST"] = "1"

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import test_wc_ozet as TO  # noqa: E402
import test_api as TA  # noqa: E402
import test_yonetim as TY  # noqa: E402
import detay as D  # noqa: E402
import musteri as M  # noqa: E402
import ozet as O  # noqa: E402

TC, TC2 = "TC:10000000146", "TC:11111111110"
PAS = "PASAPORT:DE:X1234567"
TEL = "905330000001"
GUN = "2026-09-20"
TAM = ("10000000146", "11111111110", "X1234567")


def test_maske_cihazla_ayni():
    assert D.maskele("TC:12345678901") == "123******01"
    assert D.maskele(TC) == "100******46"
    assert D.maskele(PAS) == "DE X12***67"
    assert D.maskele("PASAPORT:GB:AB123") == "GB A****"
    assert D.maskele(TEL) == TEL and D.maskele("") == ""


def test_opak_ve_sir_dosyasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        yol = os.path.join(tmp, "kisi_sir")
        s = D.sir_oku_ya_uret(yol)
        assert len(s) == 32 and D.sir_oku_ya_uret(yol) == s  # ikinci çağrı aynı sırrı okur
        with open(yol, "wb") as f:
            f.write(b"kisa")  # bozuk dosya yenilenir
        s2 = D.sir_oku_ya_uret(yol)
        assert len(s2) == 32 and s2 != b"kisa"
        a = D.opak(b"sir", TC)
        assert re.fullmatch(r"[0-9a-f]{16}", a) and a == D.opak(b"sir", TC)
        assert a != D.opak(b"sir", TC2) and a != D.opak(b"baska", TC)
        # RFC 4231 test vektörü 1 (cihazdaki hmac ile aynı algoritma)
        assert D.opak(b"\x0b" * 20, "Hi There") == "b0344c61d8db3853"


def arsiv_yaz(kok, gun=GUN):
    """Bir telefonlu, bir TC'li (oda 101) ve bir pasaportlu (oda 202) kişi."""
    d = os.path.join(kok, "gunluk", gun)
    z = lambda s: f"{gun}T{s}+03:00"  # noqa: E731
    misafir = "sozlesme=1 riza=1 giris=misafir_kodu oda={} kod_bitis=2026-09-21T12:00:00+03:00 ureten=mudur"
    TO.gz_yaz(os.path.join(d, "oturum.csv.gz"), [
        {"zaman": z("09:00:00"), "olay": "OTURUM_BASLA", "telefon": TEL, "ad": "Ayşe", "soyad": "Yılmaz",
         "mac": "aa:aa:aa:aa:aa:01", "ic_ip": "10.50.0.21", "ek": "sozlesme=1 riza=1"},
        {"zaman": z("10:00:00"), "olay": "OTURUM_BASLA", "telefon": TC, "ad": "Deniz", "soyad": "Kara",
         "mac": "aa:aa:aa:aa:aa:02", "ic_ip": "10.50.0.22", "ek": misafir.format("101")},
        {"zaman": z("11:00:00"), "olay": "OTURUM_BASLA", "telefon": PAS, "ad": "Hans", "soyad": "Muller",
         "mac": "aa:aa:aa:aa:aa:03", "ic_ip": "10.50.0.23", "ek": misafir.format("202")}])
    TO.gz_yaz(os.path.join(d, "dns.csv.gz"), [
        {"zaman": z("10:05:00"), "olay": "DNS", "telefon": TC, "alan_adi": "ornek.com"}])
    TO.gz_yaz(os.path.join(d, "trafik.csv.gz"), [
        {"zaman": z("11:06:00"), "olay": "BAGLANTI_BITIS", "telefon": PAS, "protokol": "tcp", "hedef_ip": "1.1.1.1",
         "hedef_port": "443", "gonderilen_bayt": "10", "alinan_bayt": "90", "sure_sn": "6"}])
    TO.gz_yaz(os.path.join(d, "kullanicilar", "TC_10000000146.csv.gz"), [
        {"zaman": z("10:00:00"), "olay": "KAYIT", "telefon": TC, "ad": "Deniz", "soyad": "Kara"}])
    TO.manifest(d, ["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz", "kullanicilar/TC_10000000146.csv.gz"])


def kur(tmp):
    v, k, saat = Y.ortam(tmp)
    app = M.Musteri(v, k, saat)
    n, pw = v.musteri_ekle()
    arsiv_yaz(os.path.join(tmp, "arsiv"))
    assert O.detay_guncelle(os.path.join(tmp, "arsiv"), os.path.join(tmp, "detay", str(n))) == []
    c = Y.cerez(Y.giris(app, str(n), pw))
    Y.gerekce_ver(app, c)
    return app, v, n, c


def ara(app, c, q, tur="kisi", uyruk=""):
    f = Y.al(app, "/ara", c)
    r = Y.gonder(app, "/ara", c, {"csrf": Y.csrf(f), "q": q, "tur": tur, "uyruk": uyruk})
    assert r[0] == 200, r
    return r[2].decode()


def test_eslesme_turu_kaydeder():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = TA.kur(tmp)
        n, pw = v.musteri_ekle()
        a = TA.giris(api, n, pw)[1]["cihaz_anahtari"]
        assert v.bagli_cihaz(n)["isletme_turu"] == ""
        assert TA.post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_turu": "otel"})[0] == 200
        assert v.bagli_cihaz(n)["isletme_turu"] == "otel"
        for bozuk in ("hastane", "OTEL", "", 5, None, ["yurt"]):  # izinli olmayan değer saklanmaz
            assert TA.post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_turu": bozuk})[0] == 200
            assert v.bagli_cihaz(n)["isletme_turu"] == "otel", bozuk
        TA.post(api, "/api/eslesme", {"cihaz_anahtari": a})  # gönderilmezse eski değer kalır
        assert v.bagli_cihaz(n)["isletme_turu"] == "otel"
        for tur in ("yurt", "diger", "kafe"):
            TA.post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_turu": tur})
            assert v.bagli_cihaz(n)["isletme_turu"] == tur


def test_eski_veritabanina_tur_sutunu_eklenir():
    import sqlite3
    import veri as V
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        yol = os.path.join(tmp, "m.db")
        db = sqlite3.connect(yol)
        eski = V.SEMA.replace(", isletme_turu TEXT NOT NULL DEFAULT ''", "")
        assert eski != V.SEMA
        db.executescript(eski)
        db.execute("INSERT INTO musteri (numara, tuz, ozet, yineleme, olusturma) VALUES (1111111, 't', 'o', 1, '2026-01-01T00:00')")
        db.execute("INSERT INTO cihaz (musteri, durum, wg_pub, ssh_pub, anahtar_ozet, baglanma) "
                   "VALUES (1111111, 'bagli', 'p', 's', 'h', '2026-01-01')")
        db.commit()
        db.close()
        v = V.Veri(yol)
        assert v.bagli_cihaz(1111111)["isletme_turu"] == ""


def test_yonetim_listesinde_tur():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = TY.kur(tmp)
        n1, _ = v.musteri_ekle("Otel müşterisi")
        n2, _ = v.musteri_ekle("Türsüz")
        cid, _ = v.cihaz_bagla(n1, TY.WG, TY.SSH, "Deniz Otel")
        v.eslesme_kaydet(cid, "Deniz Otel", "", "1.0", "otel")
        v.cihaz_bagla(n2, "B" * 43 + "=", TY.SSH, "Kafe X")
        s = Y.al(app, "/", c)[2].decode()
        assert "<th>Tür</th>" in s and "Otel/Pansiyon" in s
        assert "Otel/Pansiyon" in Y.al(app, f"/m/{n1}", c)[2].decode()
        s2 = Y.al(app, f"/m/{n2}", c)[2].decode()
        assert "İşletme türü" in s2 and "Otel/Pansiyon" not in s2
        for tur, etiket in (("kafe", "Kafe/Restoran"), ("yurt", "Yurt"), ("diger", "Diğer")):
            v.eslesme_kaydet(cid, "Deniz Otel", "", "1.0", tur)
            assert etiket in Y.al(app, f"/m/{n1}", c)[2].decode()


def _sizmaz(h):
    for t in TAM:
        assert t not in h, t
    for nitelik in re.findall(r'(?:href|action|id)="([^"]*)"', h):  # adres, çapa, form hedefi: kimlik ve telefon yok
        temiz = re.sub(r"k[0-9a-f]{16}", "", nitelik).replace("2026-09-20", "")  # opak çapada rastgele rakam olabilir
        assert not re.search(r"\d{5}", temiz), nitelik
        assert "X123" not in nitelik and TEL not in nitelik


def test_merkezde_tam_kimlik_sizmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, n, c = kur(tmp)
        sayfalar = [Y.al(app, "/", c)[2].decode(), Y.al(app, f"/gun/{GUN}", c)[2].decode(), Y.al(app, "/ara", c)[2].decode(),
                    ara(app, c, "10000000146"), ara(app, c, "100 000 001 46"), ara(app, c, "X1234567", uyruk="DE"),
                    ara(app, c, "x1234567"), ara(app, c, "101", "oda"), ara(app, c, "202", "oda"), ara(app, c, "Deniz"),
                    ara(app, c, "0533 000 00 01")]
        gun = sayfalar[1]
        assert "100******46" in gun and "DE X12***67" in gun and "Deniz Kara" in gun and "Hans Muller" in gun
        assert "oda=101" in gun  # oturum notu açık (kimlik içermez)
        for h in sayfalar:
            _sizmaz(h)
        for h in sayfalar[3:5]:
            assert "100******46" in h and "Deniz Kara" in h
        assert "DE X12***67" in sayfalar[5] and "DE X12***67" in sayfalar[6]
        # çapalar opak ve gün sayfasındaki kişiye götürür
        capa = re.search(r'href="/gun/2026-09-20#(k[0-9a-f]{16})"', sayfalar[3]).group(1)
        assert f'id="{capa}"' in gun
        # hareket kaydına kimlik girmez
        for h in v.hareketler(n):
            for t in TAM:
                assert t not in h["ayrinti"], h["ayrinti"]
        # GET ile sorgu dizgisindeki değer aranmaz, sayfaya yansımaz
        h = Y.al(app, "/ara?q=10000000146", c)[2].decode()
        assert "10000000146" not in h and "Deniz Kara" not in h


def test_merkez_tc_pasaport_oda_aramasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, _, _, c = kur(tmp)
        assert "Deniz Kara" in ara(app, c, "10000000146") and "Deniz Kara" in ara(app, c, "100-000-001.46")
        assert "Hans Muller" not in ara(app, c, "10000000146")
        assert "Deniz Kara" not in ara(app, c, "10000000147")  # kontrol hanesi bozuk: kimlik araması yok
        assert "Deniz Kara" not in ara(app, c, "000000")  # kimlik numarasında parça araması yok
        assert "Hans Muller" in ara(app, c, "x1234567") and "Hans Muller" in ara(app, c, "X1234567", uyruk="de")
        assert "Hans Muller" not in ara(app, c, "X1234567", uyruk="FR")
        assert "Hans Muller" not in ara(app, c, "X12345")  # pasaportta da parça araması yok
        h = ara(app, c, "101", "oda")
        assert "Deniz Kara" in h and "Hans Muller" not in h and "Ayşe" not in h
        assert "Hans Muller" in ara(app, c, "202", "oda")
        assert "Sonuç yok" in ara(app, c, "1010", "oda") and "Sonuç yok" in ara(app, c, "10", "oda")


def test_yonetimde_de_post_arama_maskeli():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = TY.kur(tmp)
        n, _ = v.musteri_ekle()
        arsiv_yaz(os.path.join(tmp, "arsiv"))
        O.detay_guncelle(os.path.join(tmp, "arsiv"), os.path.join(tmp, "detay", str(n)))
        Y.gerekce_ver(app, c)
        on = f"/m/{n}/kayitlar"
        f = Y.al(app, on + "/ara", c)
        h = f[2].decode()
        assert f'action="{on}/ara"' in h and 'method="post"' in h
        r = Y.gonder(app, on + "/ara", c, {"csrf": Y.csrf(f), "q": "10000000146"})
        h = r[2].decode()
        assert r[0] == 200 and "100******46" in h and "Deniz Kara" in h
        _sizmaz(h.replace(str(n), ""))
        _sizmaz(Y.al(app, f"{on}/gun/{GUN}", c)[2].decode().replace(str(n), ""))


def test_telefonlu_eski_arama_aynen():
    dizin = {TEL: {"ad": "Ayşe", "soyad": "Yılmaz", "gunler": [GUN]},
             TC: {"ad": "Deniz", "soyad": "Kara", "gunler": [GUN]}}
    for sorgu in (TEL, "0533 000 0001", "+90 533 000 00 01", "05330000001", "533", "0533", "0001"):
        assert [t for t, _ in D.ara_sonuclari(dizin, sorgu)] == [TEL], sorgu
    assert [t for t, _ in D.ara_sonuclari(dizin, "ayşe")] == [TEL] and not D.ara_sonuclari(dizin, "0544")
    assert [t for t, _ in D.ara_sonuclari(dizin, "kara")] == [TC]
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, _, _, c = kur(tmp)
        h = ara(app, c, "0533 000 00 01")
        assert "Ayşe Yılmaz" in h and TEL in h and "Deniz" not in h
        assert 'value="0533 000 00 01"' in h  # telefon/ad araması formda kalır
        assert "Ayşe" in ara(app, c, "ayşe")


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
