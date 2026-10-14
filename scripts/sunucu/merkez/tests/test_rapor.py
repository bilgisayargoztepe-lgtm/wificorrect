import gzip
import hashlib
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import rapor as R  # noqa: E402

BASLIK = ("zaman;olay;telefon;ad;soyad;mac;ic_ip;protokol;ic_port;hedef_ip;hedef_port;nat_ip;nat_port;"
          "alan_adi;gonderilen_bayt;alinan_bayt;sure_sn;oturum_id;ek").split(";")
SIFIR = "0" * 64
SIR = b"s" * 32
TEL1, TEL2, TEL3 = "905551112233", "905559998877", "905554443322"
TC = "TC:10000000146"
PAS = "PASAPORT:DE:X1234567"
MAC_IZINLI = "aa:bb:cc:dd:ee:01"
MAC_MISAFIR = "aa:bb:cc:dd:ee:02"


def sha(b):
    return hashlib.sha256(b).hexdigest()


def satir(zaman, olay, **alanlar):
    r = dict.fromkeys(BASLIK, "")
    r.update(alanlar, zaman=zaman, olay=olay)
    return ";".join(r[k] for k in BASLIK)


def gz_yaz(yol, satirlar):
    os.makedirs(os.path.dirname(yol), exist_ok=True)
    with gzip.open(yol, "wt", encoding="utf-8", newline="") as f:
        f.write("﻿" + ";".join(BASLIK) + "\r\n")
        for s in satirlar:
            f.write(s + "\r\n")


def manifest_yaz(d, gun):
    """Cihazın biçimi: başlık yorumu + '<sha>  <göreli ad>' (sıralı, kullanicilar/ dahil)."""
    adlar = []
    for kok, _dizinler, dosyalar in os.walk(d):
        for a in dosyalar:
            if a.endswith(".csv.gz"):
                adlar.append(os.path.relpath(os.path.join(kok, a), d).replace("\\", "/"))
    metin = f"# gun={gun} olusturma={gun}T00:15:00+03:00 onceki_zincir=x\n"
    for a in sorted(adlar):
        with open(os.path.join(d, a), "rb") as f:
            metin += f"{sha(f.read())}  {a}\n"
    with open(os.path.join(d, "MANIFEST.sha256"), "w", encoding="utf-8", newline="") as f:
        f.write(metin)


def gun_yaz(kok, gun, trafik=(), oturum=(), denetim=(), dns=(), kisi=(), manifest=True):
    d = os.path.join(kok, "gunluk", gun)
    os.makedirs(d, exist_ok=True)
    for ad, satirlar in (("trafik", trafik), ("oturum", oturum), ("denetim", denetim), ("dns", dns)):
        if satirlar:
            gz_yaz(os.path.join(d, ad + ".csv.gz"), satirlar)
    for tel in kisi:
        gz_yaz(os.path.join(d, "kullanicilar", tel + ".csv.gz"), [])
    if manifest:
        manifest_yaz(d, gun)
    return d


def man_sha(kok, gun):
    with open(os.path.join(kok, "gunluk", gun, "MANIFEST.sha256"), "rb") as f:
        return sha(f.read())


def zincir_satirlari(gunler_sha, prev=SIFIR):
    out = []
    for gun, m in gunler_sha:
        z = sha((prev + m).encode())
        out.append(f"{gun};{m};{z}")
        prev = z
    return out


def muhurle(kok, gunler, dosya="zincir.txt"):
    """Cihazdaki append_chain gibi zincir.txt üretir."""
    yol = os.path.join(kok, dosya)
    os.makedirs(os.path.dirname(yol), exist_ok=True)
    with open(yol, "w", encoding="utf-8", newline="") as f:
        f.write("".join(s + "\n" for s in zincir_satirlari([(g, man_sha(kok, g)) for g in gunler])))
    return yol


def z(gun, saat="10:00:00"):
    return f"{gun}T{saat}+03:00"


def basit_gun(kok, gun, tel=TEL1):
    return gun_yaz(kok, gun, trafik=[satir(z(gun), "BAGLANTI_BASLA", telefon=tel, mac=MAC_MISAFIR, ic_ip="10.50.0.23")],
                   kisi=[tel])


# --- kok_alan ---
def test_kok_alan():
    assert R.kok_alan("www.instagram.com.") == "instagram.com"
    assert R.kok_alan("WWW.Instagram.COM") == "instagram.com"
    assert R.kok_alan("a.b.example.com.tr") == "example.com.tr"
    assert R.kok_alan("bbc.co.uk") == "bbc.co.uk"
    assert R.kok_alan("news.bbc.co.uk") == "bbc.co.uk"
    assert R.kok_alan("instagram.com") == "instagram.com"
    for kotu in ("10.0.0.1", "localhost", "x.local", "1.0.168.192.in-addr.arpa", "ab_c.com", "", None, "a..b.com",
                 "router.lan", "x.home", "x.internal", "-a.com", "a" * 64 + ".com", ("a." * 130) + "com", "com.tr",
                 "<script>.com", 5):
        assert R.kok_alan(kotu) is None, kotu


# --- zincir ---
def test_zincir_dogru():
    with tempfile.TemporaryDirectory() as kok:
        gunler = ["2026-10-01", "2026-10-02", "2026-10-03"]
        for g in gunler:
            basit_gun(kok, g)
        muhurle(kok, gunler)
        sonuc = R.zincir_oku(kok)
        assert sonuc == {g: (man_sha(kok, g), True) for g in gunler}


def _yaz(yol, satirlar, son="\n"):
    with open(yol, "w", encoding="utf-8", newline="") as f:
        f.write("\n".join(satirlar) + son)


def test_zincir_ortadaki_manifest_sha_degisti():
    with tempfile.TemporaryDirectory() as kok:
        s = zincir_satirlari([("2026-10-01", "a" * 64), ("2026-10-02", "b" * 64), ("2026-10-03", "c" * 64)])
        g, _m, zz = s[1].split(";")
        s[1] = f"{g};{'d' * 64};{zz}"
        _yaz(os.path.join(kok, "zincir.txt"), s)
        sonuc = R.zincir_oku(kok)
        assert sonuc["2026-10-01"] == ("a" * 64, True)
        assert sonuc["2026-10-02"] == ("d" * 64, False)
        assert sonuc["2026-10-03"] == ("c" * 64, True)  # cihazdaki verify gibi: prev = yazılı zincir değeri


def test_zincir_sha_degisti_sonrakiler_bozuk():
    with tempfile.TemporaryDirectory() as kok:
        s = zincir_satirlari([("2026-10-01", "a" * 64), ("2026-10-02", "b" * 64), ("2026-10-03", "c" * 64)])
        g, m, _zz = s[1].split(";")
        s[1] = f"{g};{m};{'e' * 64}"
        _yaz(os.path.join(kok, "zincir.txt"), s)
        sonuc = R.zincir_oku(kok)
        assert sonuc["2026-10-01"][1] is True
        assert sonuc["2026-10-02"][1] is False
        assert sonuc["2026-10-03"][1] is False


def test_zincir_yarim_satir_ve_cift_gun():
    with tempfile.TemporaryDirectory() as kok:
        s = zincir_satirlari([("2026-10-01", "a" * 64), ("2026-10-02", "b" * 64)])
        yarim = s[1][:40]
        _yaz(os.path.join(kok, "zincir.txt"), [s[0], yarim], son="")
        assert R.zincir_oku(kok) == {"2026-10-01": ("a" * 64, True)}
        # aynı gün iki satır: son satır geçerli (bozuk ilk satır ok'u ezmez değil; son satır kazanır)
        s2 = zincir_satirlari([("2026-10-01", "a" * 64), ("2026-10-01", "f" * 64)])
        _yaz(os.path.join(kok, "zincir.txt"), s2)
        assert R.zincir_oku(kok) == {"2026-10-01": ("f" * 64, True)}
        s3 = s2[:1] + [f"2026-10-01;{'f' * 64};{'0' * 64}"]
        _yaz(os.path.join(kok, "zincir.txt"), s3)
        assert R.zincir_oku(kok) == {"2026-10-01": ("f" * 64, False)}


def test_zincir_eski_dosya_yeniden_kurulum():
    with tempfile.TemporaryDirectory() as kok:
        os.makedirs(os.path.join(kok, "eski"))
        _yaz(os.path.join(kok, "eski", "zincir.txt.2026-10-01"),
             zincir_satirlari([("2026-09-29", "a" * 64), ("2026-09-30", "b" * 64)]))
        _yaz(os.path.join(kok, "zincir.txt"), zincir_satirlari([("2026-10-02", "c" * 64)]))  # sıfırdan başlıyor
        sonuc = R.zincir_oku(kok)
        assert sonuc == {"2026-09-29": ("a" * 64, True), "2026-09-30": ("b" * 64, True), "2026-10-02": ("c" * 64, True)}


def test_zincir_birlesim_ok_kazanir():
    with tempfile.TemporaryDirectory() as kok:
        os.makedirs(os.path.join(kok, "eski"))
        _yaz(os.path.join(kok, "eski", "zincir.txt.1"), [f"2026-10-01;{'a' * 64};{'1' * 64}"])  # bozuk
        _yaz(os.path.join(kok, "zincir.txt"), zincir_satirlari([("2026-10-01", "a" * 64)]))
        assert R.zincir_oku(kok) == {"2026-10-01": ("a" * 64, True)}
        _yaz(os.path.join(kok, "zincir.txt"), [f"2026-10-01;{'b' * 64};{'1' * 64}"])
        _yaz(os.path.join(kok, "eski", "zincir.txt.1"), zincir_satirlari([("2026-10-01", "a" * 64)]))
        assert R.zincir_oku(kok) == {"2026-10-01": ("a" * 64, True)}


def test_zincir_yok_ve_buyuk_dosya():
    with tempfile.TemporaryDirectory() as kok:
        assert R.zincir_oku(kok) == {}
        _yaz(os.path.join(kok, "zincir.txt"), zincir_satirlari([("2026-10-01", "a" * 64)]))
        eski = R.ZINCIR_SINIRI
        try:
            R.ZINCIR_SINIRI = 10
            assert R.zincir_oku(kok) == {}
        finally:
            R.ZINCIR_SINIRI = eski


def test_zincir_sembolik_bag_okunmaz():
    with tempfile.TemporaryDirectory() as tmp:
        kok, baska = os.path.join(tmp, "a"), os.path.join(tmp, "b")
        os.makedirs(kok)
        os.makedirs(baska)
        _yaz(os.path.join(baska, "zincir.txt"), zincir_satirlari([("2026-10-01", "a" * 64)]))
        try:
            os.symlink(os.path.join(baska, "zincir.txt"), os.path.join(kok, "zincir.txt"))
        except OSError:
            return  # Windows: sembolik bağ yetkisi yok, sunucuda koşar
        assert R.zincir_oku(kok) == {}


# --- gun_dogrula ---
def test_gun_dogrula_dogru_ve_bozuk_bayt():
    with tempfile.TemporaryDirectory() as kok:
        d = basit_gun(kok, "2026-10-01")
        assert R.gun_dogrula(d) == (man_sha(kok, "2026-10-01"), True)
        yol = os.path.join(d, "kullanicilar", TEL1 + ".csv.gz")
        with open(yol, "rb") as f:
            b = bytearray(f.read())
        b[-1] ^= 1
        with open(yol, "wb") as f:
            f.write(b)
        m, ok = R.gun_dogrula(d)
        assert m == man_sha(kok, "2026-10-01") and ok is False


def test_gun_dogrula_yol_hilesi():
    with tempfile.TemporaryDirectory() as kok:
        d = basit_gun(kok, "2026-10-01")
        with open(os.path.join(kok, "gunluk", "x"), "wb") as f:
            f.write(b"x")
        for ad in ("../x", "/etc/passwd", "..\\x", "kullanicilar/../../x"):
            with open(os.path.join(d, "MANIFEST.sha256"), "w", encoding="utf-8") as f:
                f.write(f"{sha(b'x')}  {ad}\n")
            assert R.gun_dogrula(d)[1] is False, ad
        with open(os.path.join(d, "MANIFEST.sha256"), "w", encoding="utf-8") as f:
            f.write("bozuk satir\n")
        assert R.gun_dogrula(d)[1] is False
        os.remove(os.path.join(d, "MANIFEST.sha256"))
        assert R.gun_dogrula(d) == (None, False)


def test_gun_dogrula_sembolik_bag():
    with tempfile.TemporaryDirectory() as kok:
        d = basit_gun(kok, "2026-10-01")
        hedef = os.path.join(kok, "disari.csv.gz")
        with open(hedef, "wb") as f:
            f.write(b"x")
        try:
            os.symlink(hedef, os.path.join(d, "bag.csv.gz"))
        except OSError:
            return  # Windows: sembolik bağ yetkisi yok, sunucuda koşar
        with open(os.path.join(d, "MANIFEST.sha256"), "a", encoding="utf-8") as f:
            f.write(f"{sha(b'x')}  bag.csv.gz\n")
        assert R.gun_dogrula(d)[1] is False


# --- gun_ham ---
def zengin_gun(kok, gun="2026-10-05"):
    trafik = [
        satir(z(gun, "09:10:00"), "BAGLANTI_BASLA", telefon=TEL1, ad="Ali", soyad="Veli", mac=MAC_MISAFIR,
              ic_ip="10.50.0.23", hedef_ip="142.250.187.110", oturum_id="oturum-abc"),
        satir(z(gun, "09:40:00"), "BAGLANTI_BITIS", telefon=TEL1, mac=MAC_MISAFIR, ic_ip="10.50.0.23",
              gonderilen_bayt="100", alinan_bayt="1000"),
        satir(z(gun, "21:05:00"), "BAGLANTI_BASLA", telefon=TC, ad="Ayşe", mac="aa:bb:cc:dd:ee:03", ic_ip="10.50.0.24"),
        satir(z(gun, "21:06:00"), "BAGLANTI_BITIS", telefon=PAS, mac="aa:bb:cc:dd:ee:04", ic_ip="10.50.0.25",
              gonderilen_bayt="5", alinan_bayt="x"),
        satir(z(gun, "11:00:00"), "BAGLANTI_BITIS", telefon="", mac=MAC_IZINLI, ic_ip="10.50.0.2", ek="ct=1 izinli=AP",
              gonderilen_bayt="7", alinan_bayt="3"),
        satir("bozuk-zaman", "BAGLANTI_BITIS", telefon=TEL3, mac=MAC_MISAFIR, gonderilen_bayt="999999"),
        satir(z(gun, "25:00:00"), "BAGLANTI_BASLA", telefon=TEL3),
    ]
    oturum = [
        satir(z(gun, "09:09:00"), "OTURUM_BASLA", telefon=TEL1, ek="bitis=x sozlesme=1 riza=1"),
        satir(z(gun, "21:04:00"), "OTURUM_BASLA", telefon=TC, ek="bitis=x sozlesme=1 riza=1 giris=misafir_kodu oda=101"),
        satir(z(gun, "21:05:00"), "OTURUM_BASLA", telefon=PAS, ek="giris=misafir_kodu"),
        satir(z(gun, "22:00:00"), "OTURUM_UZATILDI", telefon=TC, ek="giris=misafir_kodu"),
        satir(z(gun, "23:00:00"), "OTURUM_BITIS", telefon=TEL1),
    ]
    denetim = [
        satir(z(gun, "09:08:00"), "OTP_GONDERILDI", telefon=TEL1),
        satir(z(gun, "09:08:30"), "OTP_GONDERILDI", telefon=TEL2),
        satir(z(gun, "09:08:40"), "OTP_ISTEK", telefon=TEL2),
        satir(z(gun, "03:10:00"), "GUNCELLEME_TAMAM", ek="surum=0.4.8 onceki=0.4.7"),
        satir(z(gun, "03:20:00"), "GUNCELLEME_TAMAM", ek="surum=0.4.9 onceki=0.4.8"),
    ]
    dns = [
        satir(z(gun, "09:11:00"), "DNS", telefon=TEL1, alan_adi="www.instagram.com"),
        satir(z(gun, "09:12:00"), "DNS", telefon=TEL1, alan_adi="i.instagram.com"),
        satir(z(gun, "21:07:00"), "DNS", telefon=TC, alan_adi="instagram.com."),
        satir(z(gun, "21:08:00"), "DNS", telefon="", alan_adi="facebook.com"),  # izinli/telefonsuz: sayılmaz
        satir(z(gun, "21:09:00"), "DNS", telefon=TEL1, alan_adi="10.0.0.1"),
        satir(z(gun, "21:10:00"), "DNSX", telefon=TEL1, alan_adi="twitter.com"),
    ]
    return gun_yaz(kok, gun, trafik=trafik, oturum=oturum, denetim=denetim, dns=dns, kisi=[TEL1, TC])


def test_gun_ham_sayilar():
    with tempfile.TemporaryDirectory() as kok:
        d = zengin_gun(kok)
        h = R.gun_ham(d, SIR)
        assert h["bicim"] == R.BICIM
        assert h["manifest_sha"] == man_sha(kok, "2026-10-05") and h["dosya_ok"] is True
        kim = {k: R.takma(SIR, k) for k in (TEL1, TC, PAS)}
        assert h["misafir"] == sorted(kim.values())
        assert h["saat"] == {"9": [kim[TEL1]], "21": sorted([kim[TC], kim[PAS]])}
        assert h["giris"] == {"sms": 1, "kod": 2}
        assert h["izinli"] == [R.takma(SIR, "mac:" + MAC_IZINLI)]
        assert h["gonderilen"] == 100 + 5 + 7 and h["alinan"] == 1000 + 3
        assert h["sms"] == 2
        assert h["alan"] == {"instagram.com": sorted([kim[TEL1], kim[TC]])}
        assert h["surum"] == "0.4.9"
        assert len(h["imza"]) == 64


def test_gun_ham_surum_geri_alindi_ve_yok():
    with tempfile.TemporaryDirectory() as kok:
        g = "2026-10-06"
        d = gun_yaz(kok, g, denetim=[satir(z(g, "03:10:00"), "GUNCELLEME_TAMAM", ek="surum=0.4.9 onceki=0.4.8"),
                                     satir(z(g, "03:30:00"), "GUNCELLEME_GERI_ALINDI", ek="surum=0.4.9 onceki=0.4.8 neden=x")])
        assert R.gun_ham(d, SIR)["surum"] == "0.4.8"
        g2 = "2026-10-07"
        d2 = gun_yaz(kok, g2, denetim=[satir(z(g2), "GUNCELLEME_TAMAM", ek="surum=<b>")])
        assert R.gun_ham(d2, SIR)["surum"] is None
        g3 = "2026-10-08"
        d3 = gun_yaz(kok, g3)  # hiç dosya yok, yalnızca MANIFEST
        h = R.gun_ham(d3, SIR)
        assert h["surum"] is None and h["misafir"] == [] and h["saat"] == {} and h["dosya_ok"] is True


def test_gun_ham_kisisel_veri_yok():
    with tempfile.TemporaryDirectory() as kok:
        d = zengin_gun(kok)
        metin = json.dumps(R.gun_ham(d, SIR), ensure_ascii=False)
        for gizli in (TEL1, TEL2, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", MAC_MISAFIR, MAC_IZINLI,
                      "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "AP"):
            assert gizli not in metin, gizli


# --- ham_guncelle ---
def oku(yol):
    with open(yol, encoding="utf-8") as f:
        return json.load(f)


def test_ham_guncelle_artimli_ve_manifest_degisimi():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        zengin_gun(kok, "2026-10-05")
        basit_gun(kok, "2026-10-06", tel=TEL2)
        gun_yaz(kok, "2026-10-07", trafik=[satir(z("2026-10-07"), "BAGLANTI_BASLA", telefon=TEL1)], manifest=False)
        assert R.ham_guncelle("1537344", kok, ham, SIR) == []
        assert sorted(os.listdir(os.path.join(ham, "1537344"))) == ["2026-10-05.json", "2026-10-06.json"]
        ilk = oku(os.path.join(ham, "1537344", "2026-10-05.json"))
        assert ilk["misafir"] and ilk["dosya_ok"] is True

        # imza aynıyken dosyalar yeniden okunmaz/hash'lenmez: içerik aynı boyutta bozulur, mtime geri konur
        yol = os.path.join(kok, "gunluk", "2026-10-05", "trafik.csv.gz")
        st = os.stat(yol)
        with open(yol, "r+b") as f:
            f.write(b"\x00" * st.st_size)
        os.utime(yol, ns=(st.st_atime_ns, st.st_mtime_ns))
        cagri = []
        asil = R.gun_ham
        R.gun_ham = lambda *a, **k: cagri.append(a) or asil(*a, **k)
        try:
            assert R.ham_guncelle("1537344", kok, ham, SIR) == []
        finally:
            R.gun_ham = asil
        assert cagri == []
        assert oku(os.path.join(ham, "1537344", "2026-10-05.json")) == ilk

        # MANIFEST değişince (yeniden gönderim) yeniden hesaplanır
        d6 = os.path.join(kok, "gunluk", "2026-10-06")
        gz_yaz(os.path.join(d6, "trafik.csv.gz"), [satir(z("2026-10-06"), "BAGLANTI_BASLA", telefon=TEL1),
                                                    satir(z("2026-10-06"), "BAGLANTI_BASLA", telefon=TEL3)])
        manifest_yaz(d6, "2026-10-06")
        assert R.ham_guncelle("1537344", kok, ham, SIR) == []
        assert len(oku(os.path.join(ham, "1537344", "2026-10-06.json"))["misafir"]) == 2


def test_ham_guncelle_dosya_degisince_imza_degisir():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        d = basit_gun(kok, "2026-10-05")
        R.ham_guncelle("a", kok, ham, SIR)
        assert oku(os.path.join(ham, "a", "2026-10-05.json"))["dosya_ok"] is True
        yol = os.path.join(d, "kullanicilar", TEL1 + ".csv.gz")
        with open(yol, "ab") as f:
            f.write(b"x")  # MANIFEST aynı, dosya değişti → önbellek "tam"ı eskitmez
        R.ham_guncelle("a", kok, ham, SIR)
        assert oku(os.path.join(ham, "a", "2026-10-05.json"))["dosya_ok"] is False


def test_ham_guncelle_bozuk_gz_ve_silinen_gun():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        basit_gun(kok, "2026-10-05")
        d = basit_gun(kok, "2026-10-06")
        with open(os.path.join(d, "trafik.csv.gz"), "wb") as f:
            f.write(b"\x1f\x8b\x08\x00bozuk-veri")
        manifest_yaz(d, "2026-10-06")
        basit_gun(kok, "2026-10-07")
        hatalar = R.ham_guncelle("a", kok, ham, SIR)
        assert len(hatalar) == 1 and hatalar[0].startswith("2026-10-06: ")
        assert TEL1 not in hatalar[0] and "kullanicilar" not in hatalar[0] and "trafik" not in hatalar[0]
        assert sorted(os.listdir(os.path.join(ham, "a"))) == ["2026-10-05.json", "2026-10-07.json"]
        assert len(R.ham_guncelle("a", kok, ham, SIR)) == 1  # sonraki çalıştırmada yeniden denenir
        import shutil
        shutil.rmtree(os.path.join(kok, "gunluk", "2026-10-05"))
        R.ham_guncelle("a", kok, ham, SIR)
        assert sorted(os.listdir(os.path.join(ham, "a"))) == ["2026-10-07.json"]


def test_ham_guncelle_bozuk_onbellek_yeniden_yazilir():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        basit_gun(kok, "2026-10-05")
        R.ham_guncelle("a", kok, ham, SIR)
        yol = os.path.join(ham, "a", "2026-10-05.json")
        with open(yol, "w", encoding="utf-8") as f:
            f.write("{bozuk")
        assert R.ham_guncelle("a", kok, ham, SIR) == []
        assert oku(yol)["dosya_ok"] is True


def test_ham_guncelle_tamam_olmayan_gun_yazilmaz():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        d = basit_gun(kok, "2026-10-05")
        os.remove(os.path.join(d, "kullanicilar", TEL1 + ".csv.gz"))  # MANIFEST'teki dosya henüz gelmemiş
        assert R.ham_guncelle("a", kok, ham, SIR) == []
        assert os.listdir(os.path.join(ham, "a")) == []


def test_ham_guncelle_gecersiz_arsiv_adi():
    with tempfile.TemporaryDirectory() as tmp:
        kok, ham = os.path.join(tmp, "arsiv"), os.path.join(tmp, "ham")
        basit_gun(kok, "2026-10-05")
        for ad in ("../x", "", ".", "a/b"):
            try:
                R.ham_guncelle(ad, kok, ham, SIR)
            except ValueError:
                continue
            raise AssertionError(ad)


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
