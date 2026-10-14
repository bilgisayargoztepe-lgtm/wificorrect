import datetime
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
        assert "alan" not in h and "instagram" not in json.dumps(h)  # alan adı verisi tutulmaz (spec Ek 2026-10-15)
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


# ======================================================================================================================
# Görev 2: gruplar, gün durumu, dönem raporları
# ======================================================================================================================
def ts(gun, saat=12):
    return datetime.datetime.fromisoformat(f"{gun}T{saat:02d}:00:00+03:00").timestamp()


def ars(*kaynaklar, tur="cihaz"):
    return {"tur": tur, "pub": "", "kaynaklar": list(kaynaklar)}


def misafir_gun(kok, gun, tels=(), saat="10:00:00", oturum=(), denetim=(), dns=()):
    trafik = [satir(z(gun, saat), "BAGLANTI_BASLA", telefon=t, mac=MAC_MISAFIR, ic_ip="10.50.0.23") for t in tels]
    return gun_yaz(kok, gun, trafik=trafik, oturum=oturum, denetim=denetim, dns=dns,
                   kisi=[t for t in tels if t.isdigit()])


def calis(tmp, arsivler, gun, grup_icerik=None, saat=12):
    gd = os.path.join(tmp, "gruplar.json")
    if grup_icerik is not None:
        with open(gd, "w", encoding="utf-8") as f:
            json.dump(grup_icerik, f)
    return R.calistir(arsivler, gd, os.path.join(tmp, "ham"), os.path.join(tmp, "rapor"), None, ts(gun, saat), SIR)


def rp(tmp, numara, ad):
    return oku(os.path.join(tmp, "rapor", numara, ad + ".json"))


def test_gruplar():
    with tempfile.TemporaryDirectory() as tmp:
        a = {"1234567": ars("k1", "k1b"), "7654321": ars("k2"), "bocafe": ars("k3", tur="eski"),
             "eski-b": ars("k4", tur="eski"), "eski-c": ars("k5", tur="eski"), "eski-d": ars("k6", tur="eski"),
             "0123456": ars("k7", tur="eski")}
        gd = os.path.join(tmp, "g.json")
        tek = {"1234567": [("1234567", "k1"), ("1234567", "k1b")], "7654321": [("7654321", "k2")]}
        assert R.gruplar(a, gd) == tek  # dosya yok: her numara tek başına; bağsız eski arşiv rapor almaz
        with open(gd, "w", encoding="utf-8") as f:
            json.dump({"1234567": ["eski-c", "bocafe", "7654321", "yok", "../x", 5, "bocafe"],
                       "7654321": ["bocafe", "eski-b"], "abc": ["eski-d"], "3333333": ["eski-c"],
                       "4444444": ["eski-d", "0123456"], "5555555": "bocafe"}, f)
        assert R.gruplar(a, gd) == {
            "1234567": [("1234567", "k1"), ("1234567", "k1b"), ("bocafe", "k3"), ("eski-c", "k5")],
            "7654321": [("7654321", "k2"), ("eski-b", "k4")],  # bocafe ilk müşteride; başka numara çekilemez
            "4444444": [("0123456", "k7"), ("eski-d", "k6")]}  # numara arşivi henüz yok ama eski arşivi bağlı
        for bozuk in ("{bozuk", "[1, 2]", json.dumps({"1234567": ["bocafe"], "x": "y" * 70000})):
            with open(gd, "w", encoding="utf-8") as f:
                f.write(bozuk)
            assert R.gruplar(a, gd) == tek, bozuk[:10]
        os.remove(gd)
        baska = os.path.join(tmp, "baska.json")
        with open(baska, "w", encoding="utf-8") as f:
            json.dump({"1234567": ["bocafe"]}, f)
        try:
            os.symlink(baska, gd)
        except OSError:
            return  # Windows: sembolik bağ yetkisi yok, sunucuda koşar
        assert R.gruplar(a, gd) == tek


def test_ayni_gun_iki_arsivde_bir_kez_sayilir():
    with tempfile.TemporaryDirectory() as tmp:
        k1, k1b, k2 = (os.path.join(tmp, x) for x in ("yeni", "yeni-eski-klasor", "bocafe"))
        g = "2026-10-05"
        bas = [satir(z(g), "OTURUM_BASLA", telefon=TEL1, ek="sozlesme=1 riza=1")]
        misafir_gun(k1, g, [TEL1, TEL2], oturum=bas)
        misafir_gun(k2, g, [TEL1, TEL2, TEL3], oturum=bas * 3)
        misafir_gun(k2, "2026-10-04", [TC])
        misafir_gun(k1b, "2026-10-06", [PAS])  # aynı arşivin ikinci kaynağı (eski klasör): tek ham_guncelle çağrısı
        muhurle(k1, [g])
        muhurle(k2, ["2026-10-04", g])
        muhurle(k1b, ["2026-10-06"])
        a = {"1234567": ars(k1, k1b), "bocafe": ars(k2, tur="eski")}
        assert calis(tmp, a, "2026-10-08") == []
        r = rp(tmp, "1234567", "hafta-2026-W41")
        assert r["misafir"]["farkli"] == 3 and r["misafir"]["giris"] == 1  # bağsız: 10-05 (2) + 10-06 (PAS)
        assert [x["farkli"] for x in r["gunler"][:3]] == [2, 1, None]
        assert [x["durum"] for x in r["gunler"][:2]] == ["tam", "tam"]
        assert not os.path.exists(os.path.join(tmp, "rapor", "bocafe"))
        eski_imza = r["imza"]

        assert calis(tmp, a, "2026-10-08", {"1234567": ["bocafe"]}) == []
        r = rp(tmp, "1234567", "hafta-2026-W41")
        assert r["imza"] != eski_imza  # grup değişti → yeniden üretildi
        assert r["misafir"]["farkli"] == 3 and r["misafir"]["giris"] == 1  # 10-05 bir kez: yeni arşivinki (iki kat yok)
        assert r["gunler"][0]["farkli"] == 2
        w40 = rp(tmp, "1234567", "hafta-2026-W40")
        assert w40["gunler"][-1] == {"gun": "2026-10-04", "durum": "tam", "neden": "", "farkli": 1}

        os.remove(os.path.join(k1, "gunluk", g, "kullanicilar", TEL1 + ".csv.gz"))  # yeni arşivde 10-05 eksik
        assert calis(tmp, a, "2026-10-08", {"1234567": ["bocafe"]}) == []
        r = rp(tmp, "1234567", "hafta-2026-W41")
        assert r["gunler"][0]["farkli"] == 3 and r["gunler"][0]["durum"] == "tam"  # ilk TAMAM kaynak kazanır
        assert r["misafir"]["farkli"] == 4 and r["misafir"]["giris"] == 3


def test_gun_durumlari():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        for g in ("2026-09-28", "2026-09-29", "2026-09-30", "2026-10-02", "2026-10-03", "2026-10-04", "2026-10-05",
                  "2026-10-07", "2026-10-08", "2026-10-10"):
            misafir_gun(k, g, [TEL1])
        d1 = gun_yaz(k, "2026-10-01")  # trafik.csv.gz bozuk ama MANIFEST'te doğru hash'le → okunamadı
        with open(os.path.join(d1, "trafik.csv.gz"), "wb") as f:
            f.write(b"bozuk-gz")
        manifest_yaz(d1, "2026-10-01")
        os.remove(os.path.join(k, "gunluk", "2026-10-07", "kullanicilar", TEL1 + ".csv.gz"))  # eksik
        with open(os.path.join(k, "gunluk", "2026-10-08", "kullanicilar", TEL1 + ".csv.gz"), "ab") as f:
            f.write(b"x")  # hash tutmuyor → dosya
        gunler = [(g, man_sha(k, g)) for g in ("2026-09-29", "2026-09-30", "2026-10-01", "2026-10-02", "2026-10-03",
                                               "2026-10-04", "2026-10-05", "2026-10-08", "2026-10-10")]
        gunler[1] = ("2026-09-30", "e" * 64)  # zincir satırı doğru ama MANIFEST başka → manifest
        satirlar = zincir_satirlari(gunler)
        g0, _m, z0 = satirlar[0].split(";")
        satirlar[0] = f"{g0};{'f' * 64};{z0}"  # 09-29 satırı bozuk → zincir (sonrakiler etkilenmez)
        _yaz(os.path.join(k, "zincir.txt"), satirlar)  # 09-28 zincirde yok → zincir_yok (sahte "tam" yok)

        hatalar = calis(tmp, {"1234567": ars(k)}, "2026-10-10")
        assert len(hatalar) == 1 and hatalar[0].startswith("1234567: 2026-10-01: "), hatalar
        w40 = {x["gun"]: (x["durum"], x["neden"]) for x in rp(tmp, "1234567", "hafta-2026-W40")["gunler"]}
        assert w40 == {"2026-09-28": ("dogrulanamadi", "zincir_yok"), "2026-09-29": ("dogrulanamadi", "zincir"),
                       "2026-09-30": ("dogrulanamadi", "manifest"), "2026-10-01": ("dogrulanamadi", "okunamadi"),
                       "2026-10-02": ("tam", ""), "2026-10-03": ("tam", ""), "2026-10-04": ("tam", "")}
        w41 = rp(tmp, "1234567", "hafta-2026-W41")
        assert {x["gun"]: (x["durum"], x["neden"], x["farkli"]) for x in w41["gunler"]} == {
            "2026-10-05": ("tam", "", 1), "2026-10-06": ("kayit_yok", "", None), "2026-10-07": ("eksik", "", None),
            "2026-10-08": ("dogrulanamadi", "dosya", 1), "2026-10-09": ("bekleniyor", "", None),
            "2026-10-10": ("bugun", "", None), "2026-10-11": ("gelecek", "", None)}
        assert w41["kayit"] == {"gun": 5, "tam": 1} and w41["misafir"]["farkli"] == 1  # bugün sayılmaz
        assert rp(tmp, "1234567", "hafta-2026-W40")["kayit"] == {"gun": 7, "tam": 3}
        ay = rp(tmp, "1234567", "ay-2026-10")
        assert len(ay["gunler"]) == 31 and ay["kayit"] == {"gun": 9, "tam": 4}
        assert ay["gunler"][-1]["durum"] == "gelecek"


def test_iso_hafta_ay_siniri_ve_saat():
    d = datetime.date
    assert R.hafta(d(2026, 12, 28)) == ("2026-W53", d(2026, 12, 28), d(2027, 1, 3))
    assert R.hafta(d(2027, 1, 3))[0] == "2026-W53" and R.hafta(d(2027, 1, 4))[0] == "2027-W01"
    assert R.ay(d(2026, 2, 10)) == ("2026-02", d(2026, 2, 1), d(2026, 2, 28))
    assert R.ay(d(2026, 12, 31)) == ("2026-12", d(2026, 12, 1), d(2026, 12, 31))
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-09-30", [TEL1], saat="23:30:00")
        misafir_gun(k, "2026-10-01", [TEL2], saat="00:10:00")
        muhurle(k, ["2026-09-30", "2026-10-01"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
        eylul, ekim = rp(tmp, "1234567", "ay-2026-09"), rp(tmp, "1234567", "ay-2026-10")
        assert eylul["misafir"]["farkli"] == 1 and len(eylul["gunler"]) == 30 and eylul["gunler"][-1]["farkli"] == 1
        assert eylul["saat"][23] == 1 and sum(eylul["saat"]) == 1
        assert ekim["misafir"]["farkli"] == 1 and ekim["saat"][0] == 1 and ekim["gunler"][0]["farkli"] == 1
        w40 = rp(tmp, "1234567", "hafta-2026-W40")
        assert (w40["baslangic"], w40["bitis"]) == ("2026-09-28", "2026-10-04")
        assert w40["misafir"]["farkli"] == 2 and w40["hafta_gunu"] == [0, 0, 1, 1, 0, 0, 0]  # Pzt…Paz
        assert w40["misafir"]["en_yogun_gun"] == {"gun": "2026-09-30", "farkli": 1}  # eşitlikte erken gün
        assert w40["misafir"]["gunluk_ortalama"] == 1.0


def test_yeni_donen():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-09-10", [TEL1])
        misafir_gun(k, "2026-10-05", [TEL1, TEL2])
        misafir_gun(k, "2026-10-13", [TEL2])
        muhurle(k, ["2026-09-10", "2026-10-05", "2026-10-13"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []

        def m(ad):
            return rp(tmp, "1234567", ad)["misafir"]
        assert (m("ay-2026-09")["yeni"], m("ay-2026-09")["donen"]) == (1, 0)
        assert (m("ay-2026-10")["farkli"], m("ay-2026-10")["yeni"], m("ay-2026-10")["donen"]) == (2, 1, 1)
        assert (m("hafta-2026-W41")["yeni"], m("hafta-2026-W41")["donen"]) == (1, 1)
        assert (m("hafta-2026-W42")["farkli"], m("hafta-2026-W42")["yeni"], m("hafta-2026-W42")["donen"]) == (1, 0, 1)
        assert m("ay-2026-10")["gunluk_ortalama"] == 1.5  # (2 + 1) / 2 gün
        misafir_gun(k, "2026-09-15", [TEL2])  # dönemden ÖNCEKİ gün geç geldi: "yeni" değişir, rapor yeniden yazılır
        muhurle(k, ["2026-09-10", "2026-09-15", "2026-10-05", "2026-10-13"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
        assert (m("ay-2026-10")["yeni"], m("ay-2026-10")["donen"]) == (0, 2)


def test_kesin_ve_imza():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-10-05", [TEL1])
        muhurle(k, ["2026-10-05"])
        a = {"1234567": ars(k)}
        assert calis(tmp, a, "2026-10-08") == []
        assert rp(tmp, "1234567", "hafta-2026-W41")["kesin"] is False  # süren dönem
        assert calis(tmp, a, "2026-10-14") == []
        yol = os.path.join(tmp, "rapor", "1234567", "hafta-2026-W41.json")
        assert oku(yol)["kesin"] is False  # bitti ama sonraki gün yok ve 7 gün geçmedi
        os.utime(yol, (1_000_000_000, 1_000_000_000))
        assert calis(tmp, a, "2026-10-14", saat=13) == []
        assert os.path.getmtime(yol) == 1_000_000_000  # değişiklik yok → yeniden yazılmaz
        misafir_gun(k, "2026-10-07", [TEL2])  # geç gelen gün
        muhurle(k, ["2026-10-05", "2026-10-07"])
        assert calis(tmp, a, "2026-10-14") == []
        r = oku(yol)
        assert os.path.getmtime(yol) != 1_000_000_000 and r["misafir"]["farkli"] == 2 and r["kesin"] is False
        os.makedirs(os.path.join(k, "gunluk", "2026-10-13"))  # bitişten sonraki gün geldi (henüz tamam değil)
        assert calis(tmp, a, "2026-10-14") == []
        assert oku(yol)["kesin"] is True
        assert rp(tmp, "1234567", "hafta-2026-W42")["kesin"] is False
        os.rmdir(os.path.join(k, "gunluk", "2026-10-13"))
        assert calis(tmp, a, "2026-10-18") == [] and oku(yol)["kesin"] is True  # 7 gün geçti
        assert oku(yol)["surum"] is None and oku(yol)["surum_sonra_degisti"] is False


def test_surum():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-09-20", [TEL1], denetim=[satir(z("2026-09-20"), "GUNCELLEME_TAMAM", ek="surum=0.4.8")])
        misafir_gun(k, "2026-10-05", [TEL1])
        misafir_gun(k, "2026-10-14", [TEL1], denetim=[satir(z("2026-10-14"), "GUNCELLEME_TAMAM", ek="surum=0.4.9")])
        muhurle(k, ["2026-09-20", "2026-10-05", "2026-10-14"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
        w41 = rp(tmp, "1234567", "hafta-2026-W41")
        assert w41["surum"] == "0.4.8" and w41["surum_sonra_degisti"] is True  # dönem öncesi günler dahil
        assert rp(tmp, "1234567", "hafta-2026-W42")["surum"] == "0.4.9"
        ekim = rp(tmp, "1234567", "ay-2026-10")
        assert ekim["surum"] == "0.4.9" and ekim["surum_sonra_degisti"] is False


def test_pencere_ve_liste():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2025-09-01", [TEL1])
        misafir_gun(k, "2026-10-05", [TEL1, TEL2])
        muhurle(k, ["2025-09-01", "2026-10-05"])
        hedef = os.path.join(tmp, "rapor", "1234567")
        os.makedirs(hedef)
        for ad in ("hafta-2020-W01.json", "ay-2020-01.json", "baska.txt"):
            with open(os.path.join(hedef, ad), "w") as f:
                f.write("{}")
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
        adlar = set(os.listdir(hedef))
        assert "hafta-2020-W01.json" not in adlar and "ay-2020-01.json" not in adlar and "baska.txt" in adlar
        assert len([a for a in adlar if a.startswith("hafta-")]) == 26
        assert len([a for a in adlar if a.startswith("ay-")]) == 14  # 2025-09 … 2026-10
        liste = oku(os.path.join(hedef, "liste.json"))
        haftalar = [x for x in liste if x["tur"] == "hafta"]
        aylar = [x for x in liste if x["tur"] == "ay"]
        assert [x["tur"] for x in liste] == ["hafta"] * 26 + ["ay"] * 14  # önce haftalar, sonra aylar
        assert haftalar[0]["donem"] == "2026-W43" and haftalar[0]["kesin"] is False
        assert [x["donem"] for x in haftalar] == sorted([x["donem"] for x in haftalar], reverse=True)
        assert aylar[0]["donem"] == "2026-10" and aylar[-1]["donem"] == "2025-09"
        ekim = next(x for x in aylar if x["donem"] == "2026-10")
        assert ekim == {"tur": "ay", "donem": "2026-10", "baslangic": "2026-10-01", "bitis": "2026-10-31",
                        "kesin": False, "farkli": 2, "kayit_gun": 19, "kayit_tam": 1}


def test_gunu_olmayan_donem_ve_bos_grup():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-09-01", [TEL1])
        muhurle(k, ["2026-09-01"])
        os.makedirs(os.path.join(tmp, "bos", "gunluk"))
        assert calis(tmp, {"1234567": ars(k), "2345678": ars(os.path.join(tmp, "bos"))}, "2026-10-20") == []
        r = rp(tmp, "1234567", "hafta-2026-W42")
        assert r["misafir"]["farkli"] == 0 and r["misafir"]["en_yogun_gun"] is None
        assert r["saat"] == [0] * 24 and "alanlar" not in r["kullanim"] and r["misafir"]["gunluk_ortalama"] == 0.0
        assert oku(os.path.join(tmp, "rapor", "2345678", "liste.json")) == []


def test_raporlarda_kisisel_veri_yok():
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        for g in ("2026-10-05", "2026-10-06", "2026-10-07"):
            zengin_gun(k, g)
        d = gun_yaz(k, "2026-10-08")
        with open(os.path.join(d, "trafik.csv.gz"), "wb") as f:
            f.write(b"bozuk")
        manifest_yaz(d, "2026-10-08")
        muhurle(k, ["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08"])
        hatalar = calis(tmp, {"1234567": ars(k)}, "2026-10-20")
        assert len(hatalar) == 1
        metin = "\n".join(hatalar)
        for kok, _d, dosyalar in os.walk(os.path.join(tmp, "rapor")):
            for a in dosyalar:
                with open(os.path.join(kok, a), encoding="utf-8") as f:
                    metin += f.read()
        assert "instagram.com" not in metin  # yalnızca 2 misafir
        gizli = [TEL1, TEL2, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", MAC_MISAFIR, MAC_IZINLI,
                 "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "csv", "kullanicilar"]
        gizli += [R.takma(SIR, x) for x in (TEL1, TC, PAS, "mac:" + MAC_IZINLI)]
        for x in gizli:
            assert x not in metin, x
        nedenler = {x["neden"] for x in rp(tmp, "1234567", "ay-2026-10")["gunler"]}
        assert nedenler <= {"", "okunamadi", "dosya", "zincir_yok", "zincir", "manifest"}


def test_alan_adi_verisi_yok_ve_eski_bicim_yenilenir():
    """Spec Ek (2026-10-15): ham özette ve raporlarda alan adı verisi yok; eski biçimdeki (alan adı içeren) ham ve
    raporlar bir kez yeniden üretilir."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        for g in ("2026-10-05", "2026-10-06", "2026-10-07"):
            zengin_gun(k, g)
        muhurle(k, ["2026-10-05", "2026-10-06", "2026-10-07"])
        a = {"1234567": ars(k)}
        eski, R.BICIM = R.BICIM, 1  # önceki sürümün yazdığı dosyalar
        try:
            assert calis(tmp, a, "2026-10-20") == []
        finally:
            R.BICIM = eski
        assert R.BICIM >= 2
        ham_d, rap_d = os.path.join(tmp, "ham", "1234567"), os.path.join(tmp, "rapor", "1234567")
        for a_ in os.listdir(ham_d):  # eski hamlar alan adı listesi taşıyordu
            h = oku(os.path.join(ham_d, a_))
            h["alan"] = {"instagram.com": h["misafir"]}
            with open(os.path.join(ham_d, a_), "w", encoding="utf-8") as f:
                json.dump(h, f)
        for a_ in os.listdir(rap_d):
            if a_ != "liste.json":
                r = oku(os.path.join(rap_d, a_))
                r["kullanim"]["alanlar"] = [{"alan": "instagram.com", "misafir": 3}]
                with open(os.path.join(rap_d, a_), "w", encoding="utf-8") as f:
                    json.dump(r, f)
        assert calis(tmp, a, "2026-10-20") == []
        metin = ""
        for d in (ham_d, rap_d):
            for a_ in os.listdir(d):
                v = oku(os.path.join(d, a_))
                if isinstance(v, dict):
                    assert v["bicim"] == R.BICIM, a_
                    assert "alan" not in v and "alanlar" not in v.get("kullanim", {}), a_
                metin += json.dumps(v)
        assert "instagram" not in metin and "alan" not in metin.replace("alan_", "")
        assert rp(tmp, "1234567", "ay-2026-10")["kullanim"] == {"gonderilen": 3 * 112, "alinan": 3 * 1003, "sms": 6}
        assert not hasattr(R, "kok_alan")


def test_sir_arsiv_adiyla_cakismaz():
    """Root sırrı `.sir`: `sir` adlı bir arşivin ham klasörüyle çakışmaz (arşiv adı noktayla başlayamaz)."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        basit_gun(k, "2026-10-05")
        muhurle(k, ["2026-10-05"])
        ham = os.path.join(tmp, "ham")
        hatalar = R.calistir({"sir": ars(k)}, os.path.join(tmp, "yok.json"), ham, os.path.join(tmp, "rapor"), None,
                             ts("2026-10-07"))
        assert hatalar == [], hatalar
        assert os.path.isfile(os.path.join(ham, ".sir")) and os.path.isfile(os.path.join(ham, "sir", "2026-10-05.json"))
        import durum
        assert os.path.basename(R.SIR_YOLU) == ".sir" and not durum.AD_RE.fullmatch(".sir")


# ======================================================================================================================
# Son inceleme düzeltmeleri (I1–I4, küçükler)
# ======================================================================================================================
def test_bozuk_manifest_sonraki_gunleri_durdurmaz():
    """I1: bir günün MANIFEST'i UTF-8 değil ya da klasör: o gün düşer, sonraki günler ham alır ve 'tam' olur."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-10-05", [TEL1])
        d6 = misafir_gun(k, "2026-10-06", [TEL1])
        misafir_gun(k, "2026-10-07", [TEL2])
        misafir_gun(k, "2026-10-08", [TEL2])
        muhurle(k, ["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08"])
        with open(os.path.join(d6, "MANIFEST.sha256"), "wb") as f:
            f.write(b"\xff\xfe\x00bozuk")
        d8 = os.path.join(k, "gunluk", "2026-10-08")
        os.remove(os.path.join(d8, "MANIFEST.sha256"))
        os.makedirs(os.path.join(d8, "MANIFEST.sha256"))
        assert R.ham_guncelle("1234567", k, os.path.join(tmp, "ham"), SIR) == []
        assert sorted(os.listdir(os.path.join(tmp, "ham", "1234567"))) == ["2026-10-05.json", "2026-10-07.json"]
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-10") == []
        w41 = {x["gun"]: x["durum"] for x in rp(tmp, "1234567", "hafta-2026-W41")["gunler"]}
        assert w41["2026-10-05"] == "tam" and w41["2026-10-07"] == "tam"
        assert w41["2026-10-06"] == "eksik" and w41["2026-10-08"] == "eksik"


def test_hizmet_oncesi_gunler_sayilmaz():
    """I2: grubun ilk gününden önceki günler 'kayıt gelmedi' değil, hizmet öncesi; 'N günün M'i tam'a girmez."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        for g in ("2026-10-05", "2026-10-06", "2026-10-07"):
            misafir_gun(k, g, [TEL1])
        muhurle(k, ["2026-10-05", "2026-10-06", "2026-10-07"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-09") == []
        ay = rp(tmp, "1234567", "ay-2026-10")
        durum = {x["gun"]: (x["durum"], x["farkli"]) for x in ay["gunler"]}
        for g in ("2026-10-01", "2026-10-02", "2026-10-03", "2026-10-04"):
            assert durum[g] == ("hizmet_oncesi", None), g
        assert durum["2026-10-08"][0] == "bekleniyor"
        assert ay["kayit"] == {"gun": 4, "tam": 3}  # 10-05..10-08; 1-4 Ekim sayılmaz
        liste = oku(os.path.join(tmp, "rapor", "1234567", "liste.json"))
        ekim = next(x for x in liste if x["donem"] == "2026-10")
        assert (ekim["kayit_gun"], ekim["kayit_tam"]) == (4, 3)
        assert not os.path.exists(os.path.join(tmp, "rapor", "1234567", "hafta-2026-W40.json"))
        # geç gelen daha eski gün ilk günü geri çeker: 10-03 artık sayılır, 10-04 kayıt gelmedi olur
        misafir_gun(k, "2026-10-03", [TEL2])
        muhurle(k, ["2026-10-03", "2026-10-05", "2026-10-06", "2026-10-07"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-09") == []
        ay = rp(tmp, "1234567", "ay-2026-10")
        durum = {x["gun"]: x["durum"] for x in ay["gunler"]}
        assert durum["2026-10-02"] == "hizmet_oncesi" and durum["2026-10-03"] == "tam"
        assert durum["2026-10-04"] == "kayit_yok"
        assert ay["kayit"] == {"gun": 6, "tam": 4}


def test_gun_durumu_hizmet_oncesi_ve_gelecek():
    b = datetime.date(2026, 10, 9)
    ilk = datetime.date(2026, 10, 5)
    assert R.gun_durumu("2026-10-04", b, None, None, {}, ilk) == ("hizmet_oncesi", "")
    assert R.gun_durumu("2026-10-06", b, None, None, {}, ilk) == ("kayit_yok", "")
    assert R.gun_durumu("2026-10-10", b, None, None, {}, ilk) == ("gelecek", "")
    assert R.gun_durumu("2026-10-04", b, None, None, {}) == ("kayit_yok", "")  # ilk gün verilmezse eski davranış


def test_grup_kalkinca_raporlari_silinir():
    """I3: eski arşiv yanlış müşteriye bağlanıp geri alınınca o müşterinin raporları kalmaz."""
    with tempfile.TemporaryDirectory() as tmp:
        k1, kb = os.path.join(tmp, "a"), os.path.join(tmp, "bocafe")
        misafir_gun(k1, "2026-10-05", [TEL1])
        misafir_gun(kb, "2026-10-06", [TEL2])
        muhurle(k1, ["2026-10-05"])
        muhurle(kb, ["2026-10-06"])
        a = {"1234567": ars(k1), "bocafe": ars(kb, tur="eski")}
        assert calis(tmp, a, "2026-10-08", {"4511643": ["bocafe"]}) == []
        yanlis = os.path.join(tmp, "rapor", "4511643")
        assert oku(os.path.join(yanlis, "liste.json")) and rp(tmp, "4511643", "hafta-2026-W41")["misafir"]["farkli"] == 1
        with open(os.path.join(yanlis, "baska.txt"), "w") as f:
            f.write("x")  # bizim olmayan dosyaya dokunulmaz
        assert calis(tmp, a, "2026-10-08", {"1234567": ["bocafe"]}) == []
        assert sorted(os.listdir(yanlis)) == ["baska.txt"]
        assert rp(tmp, "1234567", "hafta-2026-W41")["misafir"]["farkli"] == 2
        os.remove(os.path.join(yanlis, "baska.txt"))
        assert calis(tmp, {"bocafe": ars(kb, tur="eski")}, "2026-10-08", {}) == []  # numara arşivi de kalktı
        assert not os.path.exists(yanlis)
        assert not os.path.exists(os.path.join(tmp, "rapor", "1234567"))


def test_donem_yalnizca_kendi_gunlerinin_hamini_yukler():
    """I4: dönem raporu yalnızca o dönemin ham özetlerini belleğe alır; imzası değişmeyen dönem için hiç yüklemez."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        gunler = ["2026-08-20", "2026-09-10", "2026-10-05", "2026-10-13"]
        for g in gunler:
            misafir_gun(k, g, [TEL1, TEL2] if g == "2026-10-05" else [TEL1])
        muhurle(k, gunler)
        cagri = []
        asil = R._donem_hamlari

        def izle(ham_dizini, harita, ozet, bas, bit, bugun):
            h = asil(ham_dizini, harita, ozet, bas, bit, bugun)
            cagri.append((bas, bit, sorted(h)))
            return h
        R._donem_hamlari = izle
        try:
            assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
            assert cagri and all(all(bas.isoformat() <= g <= bit.isoformat() for g in gs) for bas, bit, gs in cagri)
            assert max(len(gs) for _b, _bt, gs in cagri) <= 2
            cagri.clear()
            assert calis(tmp, {"1234567": ars(k)}, "2026-10-20", saat=13) == []
            assert cagri == []  # hiçbir şey değişmedi: dönem hamları yüklenmez
        finally:
            R._donem_hamlari = asil
        m = rp(tmp, "1234567", "ay-2026-10")["misafir"]
        assert (m["farkli"], m["yeni"], m["donen"]) == (2, 1, 1)
        assert rp(tmp, "1234567", "ay-2026-08")["misafir"]["yeni"] == 1
        assert rp(tmp, "1234567", "ay-2026-09")["misafir"]["donen"] == 1


def test_sir_degisince_hamlar_yeniden_hesaplanir():
    """M1: sır yeniden üretilirse eski takma kimlikli hamlar yeniden hesaplanır; herkes 'yeni' görünmez."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-09-10", [TEL1])
        misafir_gun(k, "2026-10-05", [TEL1, TEL2])
        muhurle(k, ["2026-09-10", "2026-10-05"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-20") == []
        yeni_sir = b"t" * 32
        hatalar = R.calistir({"1234567": ars(k)}, os.path.join(tmp, "yok.json"), os.path.join(tmp, "ham"),
                             os.path.join(tmp, "rapor"), None, ts("2026-10-20"), yeni_sir)
        assert hatalar == []
        h = oku(os.path.join(tmp, "ham", "1234567", "2026-09-10.json"))
        assert h["misafir"] == [R.takma(yeni_sir, TEL1)]
        m = rp(tmp, "1234567", "ay-2026-10")["misafir"]
        assert (m["yeni"], m["donen"]) == (1, 1)


def test_sir_izi_tutmayan_ham_kullanilmaz():
    """M1: ham başka sırla yazılmışsa (izi tutmuyor) rapor onu kullanmaz."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-10-05", [TEL1])
        muhurle(k, ["2026-10-05"])
        R.ham_guncelle("1234567", k, os.path.join(tmp, "ham"), b"t" * 32)  # başka sırla
        R.grup_raporla("1234567", [("1234567", k)], os.path.join(tmp, "ham"), os.path.join(tmp, "rapor"), None,
                       datetime.date(2026, 10, 8), ts("2026-10-08"), SIR)
        r = rp(tmp, "1234567", "hafta-2026-W41")
        assert r["gunler"][0]["durum"] == "dogrulanamadi" and r["misafir"]["farkli"] == 0


def test_gelecek_tarihli_gun_kesinlestirmez():
    """M2: cihaz saati yanlışken gelen ileri tarihli gün biten dönemi hemen kesinleştirmez, sürüm bilgisi vermez."""
    with tempfile.TemporaryDirectory() as tmp:
        k = os.path.join(tmp, "a")
        misafir_gun(k, "2026-10-05", [TEL1])
        misafir_gun(k, "2026-10-16", [TEL1], denetim=[satir(z("2026-10-16"), "GUNCELLEME_TAMAM", ek="surum=0.9.9")])
        muhurle(k, ["2026-10-05", "2026-10-16"])
        assert calis(tmp, {"1234567": ars(k)}, "2026-10-14") == []
        r = rp(tmp, "1234567", "hafta-2026-W41")
        assert r["kesin"] is False and r["surum_sonra_degisti"] is False
        assert rp(tmp, "1234567", "hafta-2026-W42")["surum"] is None


def test_tamam_mi_fifo_beklemez():
    """M3: MANIFEST.sha256 FIFO ise root takılmaz."""
    if not hasattr(os, "mkfifo"):
        return  # Windows: FIFO yok, sunucuda koşar
    import istatistik
    with tempfile.TemporaryDirectory() as tmp:
        d = os.path.join(tmp, "gun")
        os.makedirs(d)
        os.mkfifo(os.path.join(d, "MANIFEST.sha256"))
        assert istatistik.tamam_mi(d) is False


def test_tamam_mi_bozuk_manifest():
    import istatistik
    with tempfile.TemporaryDirectory() as tmp:
        d = basit_gun(tmp, "2026-10-05")
        assert istatistik.tamam_mi(d) is True
        with open(os.path.join(d, "MANIFEST.sha256"), "wb") as f:
            f.write(b"\xff\xfe bozuk")
        assert istatistik.tamam_mi(d) is False
        os.remove(os.path.join(d, "MANIFEST.sha256"))
        os.makedirs(os.path.join(d, "MANIFEST.sha256"))
        assert istatistik.tamam_mi(d) is False


def test_zincir_eski_dosyalar_gerekmedikce_okunmaz():
    """M4: istenen günler güncel zincir.txt'de doğrulanıyorsa eski/zincir.txt.* okunmaz; değilse en yeniden eskiye."""
    with tempfile.TemporaryDirectory() as kok:
        os.makedirs(os.path.join(kok, "eski"))
        for i in range(5):
            _yaz(os.path.join(kok, "eski", f"zincir.txt.2026-09-2{i}"), zincir_satirlari([("2026-09-01", "a" * 64)]))
        _yaz(os.path.join(kok, "zincir.txt"), zincir_satirlari([("2026-10-02", "c" * 64)]))
        okunan = []
        asil = R._oku_sinirli
        R._oku_sinirli = lambda yol, sinir: okunan.append(os.path.basename(yol)) or asil(yol, sinir)
        try:
            assert R.zincir_oku(kok, {"2026-10-02"}) == {"2026-10-02": ("c" * 64, True)}
            assert okunan == ["zincir.txt"]
            okunan.clear()
            assert R.zincir_oku(kok, {"2026-10-02", "2026-09-01"})["2026-09-01"] == ("a" * 64, True)
            assert okunan == ["zincir.txt", "zincir.txt.2026-09-24"]  # en yeni eski kopya yetti
        finally:
            R._oku_sinirli = asil
        assert set(R.zincir_oku(kok)) == {"2026-10-02", "2026-09-01"}  # gerekli verilmezse hepsi


def test_gecici_dosya_hata_sonrasi_kalmaz():
    """M5: yazma hatasında <yol>.<pid>.tmp kalmaz."""
    with tempfile.TemporaryDirectory() as tmp:
        for f in (lambda y: R._yaz_600(y, {"x": {1, 2}}), lambda y: R._yaz_panel(y, {"x": {1, 2}}, None)):
            try:
                f(os.path.join(tmp, "a.json"))
            except TypeError:
                pass
            else:
                raise AssertionError("TypeError bekleniyordu")
            assert os.listdir(tmp) == []


def test_kur_sh_ve_merkez_birimi():
    """M7, M8: kur.sh rapor modüllerini ve dizinlerini kurar; merkez (wcpanel) rapor-ham'a erişemez."""
    kok = os.path.dirname(HERE)
    kur = open(os.path.join(kok, "kur.sh"), encoding="utf-8").read()
    assert "rapor.py" in kur and "raporlar.py" in kur
    assert "install -d -o root -g wcpanel -m 750" in kur and "/var/lib/wificorrect/rapor\n" in kur
    assert "install -d -m 700 /var/lib/wificorrect/rapor-ham" in kur
    srv = open(os.path.join(kok, "wc-merkez.service"), encoding="utf-8").read()
    assert "InaccessiblePaths=-/var/lib/wificorrect/rapor-ham" in srv


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
