"""Panel: Raporlar sayfaları (müşteri + yönetim), yazdırma, cihaz bölümü. Rapor JSON'ları elle yazılır (root'un çıktısı);
uçtan uca testte rapor.py'nin gerçek çıktısı kullanılır."""
import builtins
import copy
import datetime
import json
import os
import sys
import tempfile

os.environ["WC_TEST"] = "1"
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import musteri as MU  # noqa: E402
import raporlar as RP  # noqa: E402
import yonetici  # noqa: E402
import yonetim as YN  # noqa: E402

WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"
PW = "yonetici-parola-1"
TZ = datetime.timezone(datetime.timedelta(hours=3))


def ep(iso):
    return datetime.datetime.fromisoformat(iso).timestamp()


def gunler(bas, n, durum="tam"):
    t = datetime.date.fromisoformat(bas)
    return [{"gun": (t + datetime.timedelta(days=i)).isoformat(), "durum": durum, "neden": "", "farkli": 2} for i in range(n)]


def ay_raporu(**degisik):
    """2026-09 (panel saati 21.09.2026 17:13: süren dönem)."""
    g = gunler("2026-09-01", 30)
    for x in g[20:]:
        x.update(durum="gelecek", farkli=None)
    g[20].update(durum="bugun")
    g[19].update(durum="bekleniyor", farkli=None)
    g[3].update(durum="dogrulanamadi", neden="zincir_yok")
    g[4].update(durum="kayit_yok", farkli=None)
    g[5].update(durum="eksik", farkli=None)
    r = {"bicim": 1, "tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30",
         "uretim": "2026-09-21T12:31:07+03:00", "kesin": False, "imza": "x" * 64,
         "misafir": {"farkli": 41, "yeni": 30, "donen": 11, "giris": 77, "giris_sms": 70, "giris_kod": 7, "izinli_cihaz": 2,
                     "gunluk_ortalama": 4.5, "en_yogun_gun": {"gun": "2026-09-12", "farkli": 9}},
         "saat": [0] * 9 + [5, 9] + [1] * 13, "hafta_gunu": [1, 2, 3, 4, 5, 6, 7],
         "gunler": g, "kayit": {"gun": 20, "tam": 16},
         "kullanim": {"gonderilen": 1024 ** 3, "alinan": 3 * 1024 ** 3, "sms": 70,
                      "alanlar": [{"alan": "instagram.com", "misafir": 12}, {"alan": "<script>alert(1)</script>.com", "misafir": 3}]},
         "surum": "0.4.9", "surum_sonra_degisti": False}
    r.update(degisik)
    return r


def hafta_raporu(**degisik):
    r = {"bicim": 1, "tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20",
         "uretim": "2026-09-21T02:31:07+03:00", "kesin": False, "imza": "y" * 64,
         "misafir": {"farkli": 6, "yeni": 1, "donen": 5, "giris": 9, "giris_sms": 9, "giris_kod": 0, "izinli_cihaz": 1,
                     "gunluk_ortalama": 2.0, "en_yogun_gun": None},
         "saat": [0] * 24, "hafta_gunu": [0] * 7, "gunler": gunler("2026-09-14", 7) [:6] + [
             {"gun": "2026-09-20", "durum": "bekleniyor", "neden": "", "farkli": None}],
         "kayit": {"gun": 7, "tam": 6},
         "kullanim": {"gonderilen": 0, "alinan": 0, "sms": 9, "alanlar": []}, "surum": None, "surum_sonra_degisti": False}
    r.update(degisik)
    return r


LISTE = [
    {"tur": "hafta", "donem": "2026-W39", "baslangic": "2026-09-21", "bitis": "2026-09-27", "kesin": False, "farkli": 3,
     "kayit_gun": 0, "kayit_tam": 0},
    {"tur": "hafta", "donem": "2026-W38", "baslangic": "2026-09-14", "bitis": "2026-09-20", "kesin": False, "farkli": 6,
     "kayit_gun": 7, "kayit_tam": 6},
    {"tur": "hafta", "donem": "2026-W36", "baslangic": "2026-08-31", "bitis": "2026-09-06", "kesin": True, "farkli": 8,
     "kayit_gun": 7, "kayit_tam": 7},
    {"tur": "ay", "donem": "2026-09", "baslangic": "2026-09-01", "bitis": "2026-09-30", "kesin": False, "farkli": 41,
     "kayit_gun": 20, "kayit_tam": 16},
    {"tur": "ay", "donem": "2026-08", "baslangic": "2026-08-01", "bitis": "2026-08-31", "kesin": True, "farkli": 50,
     "kayit_gun": 31, "kayit_tam": 31},
]


def yaz(tmp, numara, ad, icerik):
    d = os.path.join(tmp, "rapor", str(numara))
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, ad), "w", encoding="utf-8") as f:
        f.write(icerik if isinstance(icerik, str) else json.dumps(icerik))


def kur_musteri(tmp):
    v, k, saat = Y.ortam(tmp)
    app = MU.Musteri(v, k, saat, raporlar=RP.Raporlar(os.path.join(tmp, "rapor")))
    n, pw = v.musteri_ekle()
    return app, v, saat, n, Y.cerez(Y.giris(app, str(n), pw))


def kur_yonetim(tmp):
    v, k, saat = Y.ortam(tmp)
    yol, dyol = os.path.join(tmp, "yonetici.json"), os.path.join(tmp, "durum.json")
    yonetici.yaz(yol, "serkan", PW)
    with open(dyol, "w", encoding="utf-8") as f:
        json.dump({"zaman": "", "arsivler": {}}, f)
    app = YN.Yonetim(v, k, saat, yol, dyol, lambda islem, veri: "0" * 16, raporlar=RP.Raporlar(os.path.join(tmp, "rapor")))
    return app, v, saat, Y.cerez(Y.giris(app, "serkan", PW))


def html(r):
    return r[2].decode()


# --- Raporlar sınıfı ---
def test_liste_ve_rapor_okuma_dogrulama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        rp = RP.Raporlar(os.path.join(tmp, "rapor"))
        assert rp.liste(1234567) == [] and rp.rapor(1234567, "ay", "2026-09") is None  # klasör yok
        yaz(tmp, 1234567, "liste.json", LISTE + [{"tur": "yil", "donem": "2026"}, "x", {"tur": "ay", "donem": "../x"}])
        assert [x["donem"] for x in rp.liste(1234567)] == ["2026-W39", "2026-W38", "2026-W36", "2026-09", "2026-08"]
        yaz(tmp, 1234567, "ay-2026-09.json", ay_raporu())
        assert rp.rapor(1234567, "ay", "2026-09")["misafir"]["farkli"] == 41
        for tur, donem in (("ay", "2026-13"), ("ay", "2026-00"), ("hafta", "2026-W54"), ("hafta", "2026-W00"),
                           ("hafta", "../x"), ("yil", "2026"), ("ay", "2026-9"), ("ay", "２０２６-09")):
            assert rp.rapor(1234567, tur, donem) is None, (tur, donem)
        assert not any(RP.donem_gecerli(t, d) for t, d in (("ay", "2026-13"), ("ay", "2026-00"), ("hafta", "2026-W54"),
                                                            ("hafta", "2026-W00"), ("yil", "2026"), ("ay", None)))
        assert RP.donem_gecerli("hafta", "2026-W53") and RP.donem_gecerli("ay", "2026-12")
        assert rp.rapor(7654321, "ay", "2026-09") is None and rp.rapor("../1234567", "ay", "2026-09") is None
        yaz(tmp, 1234567, "ay-2026-08.json", "{bozuk")
        assert rp.rapor(1234567, "ay", "2026-08") is None
        yaz(tmp, 1234567, "ay-2026-08.json", ay_raporu(bicim=2, donem="2026-08"))
        assert rp.rapor(1234567, "ay", "2026-08") is None
        yaz(tmp, 1234567, "ay-2026-08.json", ay_raporu())  # dosya adı ile içerik farklı dönem
        assert rp.rapor(1234567, "ay", "2026-08") is None
        yaz(tmp, 1234567, "liste.json", "[bozuk")
        assert rp.liste(1234567) == []


def test_rapor_turleri_denetlenir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        rp = RP.Raporlar(os.path.join(tmp, "rapor"))
        r = ay_raporu(misafir={"farkli": "41", "yeni": -3, "donen": True, "giris": 1.5, "gunluk_ortalama": "x",
                               "en_yogun_gun": {"gun": "<b>", "farkli": 3}},
                      saat=["a"] * 30, hafta_gunu="x", gunler=[{"gun": "2026-09-01", "durum": "<i>", "neden": 5, "farkli": -1},
                                                              "bozuk", {"gun": "x"}],
                      kayit=[], kullanim={"gonderilen": "1", "alanlar": [{"alan": 5, "misafir": 3}, "x", {"alan": "a.com", "misafir": "9"}]},
                      surum="<script>", kesin="evet")
        yaz(tmp, 1234567, "ay-2026-09.json", r)
        n = rp.rapor(1234567, "ay", "2026-09")
        assert n["misafir"]["farkli"] == 0 and n["misafir"]["yeni"] == 0 and n["misafir"]["donen"] == 0
        assert n["misafir"]["giris"] == 0 and n["misafir"]["gunluk_ortalama"] == 0.0 and n["misafir"]["en_yogun_gun"] is None
        assert n["saat"] == [0] * 24 and n["hafta_gunu"] == [0] * 7 and n["kayit"] == {"gun": 0, "tam": 0}
        assert n["gunler"] == [{"gun": "2026-09-01", "durum": "bilinmiyor", "neden": "", "farkli": None}]
        assert n["kullanim"]["gonderilen"] == 0 and n["kullanim"]["alanlar"] == [{"alan": "a.com", "misafir": 0}]
        assert n["surum"] is None and n["kesin"] is False
        s = RP.rapor_html(n, "İşletme", "", 1234567, [], 0, "", Y.Saat.t)  # çökmez
        assert "<script" not in s


# --- müşteri paneli ---
def test_musteri_liste_sayfasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        s = html(Y.al(app, "/raporlar", c))
        assert "Henüz rapor yok" in s
        yaz(tmp, n, "liste.json", LISTE)
        r = Y.al(app, "/raporlar", c)
        s = html(r)
        assert r[0] == 200 and s.index("Haftalık raporlar") < s.index("Aylık raporlar")
        assert s.index("21–27.09.2026") < s.index("14–20.09.2026") < s.index("31.08–06.09.2026")
        assert s.index("Eylül 2026") < s.index("Ağustos 2026")
        assert s.count("sürüyor") == 2  # W39 ve Eylül
        assert "7 günün 6'sı tam" in s and "31 günün 31'i tam" in s and "20 günün 16'sı tam" in s
        assert 'href="/rapor/hafta/2026-W38"' in s and 'href="/rapor/ay/2026-08"' in s
        assert "<script" not in s


def test_musteri_menu_ve_rapor_sayfasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        v.cihaz_bagla(n, WG, SSH, "Bocafe Göztepe", "Bocafe Gıda Ltd. Şti.", "0.5.0")
        yaz(tmp, n, "ay-2026-09.json", ay_raporu())
        r = Y.al(app, "/rapor/ay/2026-09", c)
        s = html(r)
        assert r[0] == 200
        menu = s[s.index("<nav>"):s.index("</nav>")]
        assert menu.index("Yedek arşiv") < menu.index("Raporlar") < menu.index("Parola")
        for b in ("5651 kayıt durumu", "Misafirler", "Yoğunluk", "Kullanım", "Cihaz"):
            assert f"<h2>{b}</h2>" in s, b
        assert s.index("5651 kayıt durumu") < s.index("<h2>Misafirler")
        assert "Engellen" not in s
        assert "Bocafe Göztepe" in s and "Bocafe Gıda Ltd. Şti." in s and str(n) in s and "Eylül 2026" in s
        assert "21.09.2026 12:31" in s and "sürüyor" in s and "02:30" in s
        assert "20 günün 16'sı tam" in s
        assert "gün mühür zincirinde yok" in s and "Kayıt gelmedi" in s and "Eksik geldi" in s
        assert "&lt;script&gt;alert(1)&lt;/script&gt;.com" in s and "<script" not in s
        assert "instagram.com" in s and "en az 3 farklı misafir" in s
        assert "4,0 GB" in s and "4,5" in s and "12.09.2026" in s
        assert "@media print" in s and 'class="yazdirma' in s and "Ctrl+P" in s and 'href="/raporlar"' in s
        assert "0.4.9" in s
        # gerekçe istenmez, hareket yazılmaz
        assert [h["olay"] for h in v.hareketler(n)] == ["GIRIS"]
        yaz(tmp, n, "hafta-2026-W38.json", hafta_raporu())
        s = html(Y.al(app, "/rapor/hafta/2026-W38", c))
        assert "14–20.09.2026" in s and "7 günün 6'sı tam" in s and "Gösterilecek alan adı yok" in s
        assert "0.5.0" in s  # sürüm null ve sonra değişmedi: cihazın bugünkü sürümü
        assert [h["olay"] for h in v.hareketler(n)] == ["GIRIS"]


def test_hizmet_oncesi_gunler():
    """Cihaz kurulmadan önceki günler 'Kayıt gelmedi' değil, nötr 'Hizmet öncesi' görünür."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        v.cihaz_bagla(n, WG, SSH, "Bocafe Göztepe", "Bocafe Gıda Ltd. Şti.", "0.5.0")
        r = ay_raporu()
        r["gunler"][4].update(durum="tam", farkli=2)
        for x in r["gunler"][:3]:
            x.update(durum="hizmet_oncesi", neden="", farkli=None)
        yaz(tmp, n, "ay-2026-09.json", r)
        okunan = RP.Raporlar(os.path.join(tmp, "rapor")).rapor(n, "ay", "2026-09")
        assert [x["durum"] for x in okunan["gunler"][:3]] == ["hizmet_oncesi"] * 3
        s = html(Y.al(app, "/rapor/ay/2026-09", c))
        assert s.count("Hizmet öncesi") == 3 and "Kayıt gelmedi" not in s


def test_musteri_yollar_404_ve_baska_musteri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        n2, _ = v.musteri_ekle()
        yaz(tmp, n2, "ay-2026-09.json", ay_raporu())
        yaz(tmp, n2, "liste.json", LISTE)
        assert Y.al(app, "/rapor/ay/2026-09", c)[0] == 404  # yalnızca kendi klasörü
        assert "2026-W38" not in html(Y.al(app, "/raporlar", c))
        for yol in ("/rapor/ay/2026-13", "/rapor/hafta/../x", "/rapor/yil/2026", "/rapor/hafta/2026-W54", "/rapor/ay/",
                    f"/rapor/ay/2026-09/../../{n2}", "/rapor/hafta/2026-w38"):
            r = Y.al(app, yol, c)
            assert r[0] == 404 and "Bu dönem için rapor yok" in html(r), yol
        yaz(tmp, n, "ay-2026-09.json", "{bozuk")
        assert Y.al(app, "/rapor/ay/2026-09", c)[0] == 404
        assert Y.al(app, "/raporlar", "")[0] == 303  # giriş ister


def test_sayfa_arsiv_ve_ham_okumaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        yaz(tmp, n, "liste.json", LISTE)
        yaz(tmp, n, "ay-2026-09.json", ay_raporu())
        acilan, gercek = [], builtins.open

        def izle(yol, *a, **k):
            acilan.append(os.path.abspath(str(yol)))
            return gercek(yol, *a, **k)
        builtins.open = izle
        try:
            assert Y.al(app, "/raporlar", c)[0] == 200 and Y.al(app, "/rapor/ay/2026-09", c)[0] == 200
        finally:
            builtins.open = gercek
        kok = os.path.abspath(os.path.join(tmp, "rapor", str(n)))
        assert acilan and all(a.startswith(kok) for a in acilan), acilan
        assert not os.path.exists(os.path.join(tmp, "rapor-ham"))


# --- cihaz bölümü ---
def test_cihaz_bolumu_sorunlar_ve_cevrimdisi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        n2, _ = v.musteri_ekle()

        def sorun(tur, metin, onem, acilis, kapanis=None, musteri=n):
            saat.t = ep(acilis)
            i = v.sorun_ac(musteri, tur, metin, onem)
            if kapanis:
                saat.t = ep(kapanis)
                v.sorun_kapat(i)
        sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-08-25T10:00:00+03:00", "2026-08-26T10:00:00+03:00")
        sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-08-31T22:00:00+03:00", "2026-09-01T02:00:00+03:00")
        sorun("servis", "Servis durdu: <portal>", "kritik", "2026-09-10T08:00:00+03:00", "2026-09-10T09:30:00+03:00")
        sorun("lisans", "Lisans bitiyor", "bilgi", "2026-09-12T08:00:00+03:00")
        sorun("disk", "Disk dolu", "uyari", "2026-09-15T08:00:00+03:00", "2026-09-15T08:20:00+03:00", musteri=n2)
        sorun("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik", "2026-09-21T15:13:20+03:00")
        saat.t = Y.Saat.t  # 21.09.2026 17:13:20
        ar = v.sorunlar_aralik(n, "2026-09-01T00:00:00+03:00", "2026-10-01T00:00:00+03:00")
        assert [x["tur"] for x in ar] == ["ulasilamiyor", "servis", "lisans", "ulasilamiyor"]
        yaz(tmp, n, "ay-2026-09.json", ay_raporu())
        s = html(Y.al(app, "/rapor/ay/2026-09", c))
        cihaz = s[s.index("<h2>Cihaz</h2>"):]
        assert "Servis durdu: &lt;portal&gt;" in cihaz and "1 sa 30 dk" in cihaz
        assert "Lisans bitiyor" not in cihaz and "Disk dolu" not in cihaz
        assert "31.08.2026" not in cihaz  # dönemden önce açılan listelenmez ama süresi sayılır
        assert "açık" in cihaz and "en az 4 sa 0 dk" in cihaz
        assert RP.cevrimdisi_sn(ar, "2026-09-01", "2026-09-30", Y.Saat.t) == 4 * 3600
        assert RP.cevrimdisi_sn(ar, "2026-09-14", "2026-09-20", Y.Saat.t) == 0


def test_surum_kurali():
    r = RP.normal_rapor(ay_raporu(surum=None, surum_sonra_degisti=True), "ay", "2026-09")
    assert "0.5.0" not in RP.rapor_html(r, "X", "", 1234567, [], 0, "0.5.0", Y.Saat.t)
    r = RP.normal_rapor(ay_raporu(surum=None), "ay", "2026-09")
    assert "0.5.0" in RP.rapor_html(r, "X", "", 1234567, [], 0, "0.5.0", Y.Saat.t)
    r = RP.normal_rapor(ay_raporu(surum="0.4.7", surum_sonra_degisti=True), "ay", "2026-09")
    s = RP.rapor_html(r, "X", "", 1234567, [], 0, "0.5.0", Y.Saat.t)
    assert "0.4.7" in s and "0.5.0" not in s


def test_turkce_ek():
    assert [RP.iyelik(x) for x in (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 31, 40, 50, 60, 100)] == [
        "ı", "i", "si", "ü", "ü", "i", "sı", "si", "i", "u", "u", "si", "u", "i", "ı", "si", "ı", "ü"]


# --- yönetim ---
def test_yonetim_raporlar_gerekcesiz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c = kur_yonetim(tmp)
        n, _ = v.musteri_ekle()
        assert f'href="/m/{n}/raporlar"' in html(Y.al(app, f"/m/{n}", c))
        yaz(tmp, n, "liste.json", LISTE)
        yaz(tmp, n, "ay-2026-09.json", ay_raporu())
        r = Y.al(app, f"/m/{n}/raporlar", c)
        assert r[0] == 200 and f'href="/m/{n}/rapor/ay/2026-09"' in html(r)
        r = Y.al(app, f"/m/{n}/rapor/ay/2026-09", c)
        assert r[0] == 200 and f"Müşteri {n}" in html(r) and f'href="/m/{n}/raporlar"' in html(r)
        assert Y.al(app, f"/m/{n}/rapor/ay/2026-13", c)[0] == 404
        if v.musteri(1000000) is None:
            assert Y.al(app, "/m/1000000/raporlar", c)[0] == 404
        assert not [h for h in v.hareketler(n) if h["olay"] in ("GORUNTULEME", "GEREKCE")]


# --- uçtan uca: rapor.py çıktısı → sayfa; kişisel veri yok ---
def test_uctan_uca_kisisel_veri_yok():
    import test_rapor as T
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, n, c = kur_musteri(tmp)
        k = os.path.join(tmp, "a")
        for g in ("2026-09-05", "2026-09-06", "2026-09-07"):
            T.zengin_gun(k, g)
        T.misafir_gun(k, "2026-09-08", tels=[T.TEL1, T.TEL2, T.TEL3],
                      dns=[T.satir(T.z("2026-09-08"), "DNS", telefon=t, alan_adi="www.ornek.com.tr")
                           for t in (T.TEL1, T.TEL2, T.TEL3)])
        T.muhurle(k, ["2026-09-05", "2026-09-06", "2026-09-07", "2026-09-08"])
        hatalar = T.R.calistir({str(n): T.ars(k)}, os.path.join(tmp, "yok.json"), os.path.join(tmp, "ham"),
                               os.path.join(tmp, "rapor"), None, T.ts("2026-09-21"), T.SIR)
        assert hatalar == [], hatalar
        s = html(Y.al(app, "/raporlar", c)) + html(Y.al(app, "/rapor/ay/2026-09", c))
        s += html(Y.al(app, "/rapor/hafta/2026-W36", c))
        # ilk arşiv günü 05.09: 1–4 Eylül hizmet öncesi, sayılmaz (16 gün = 05–20 Eylül)
        assert "ornek.com.tr" in s and "16 günün 4'ü tam" in s and "Hizmet öncesi" in s
        assert "Kayıt gelmedi" in s  # 09–19 Eylül: hizmet içinde, kayıt yok
        gizli = [T.TEL1, T.TEL2, T.TEL3, "5551112233", "TC:", "10000000146", "PASAPORT", "X1234567", T.MAC_MISAFIR,
                 T.MAC_IZINLI, "aa:bb", "10.50.0", "142.250", "Ali", "Veli", "Ayşe", "oturum-abc", "csv", "kullanicilar"]
        gizli += [T.R.takma(T.SIR, x) for x in (T.TEL1, T.TC, T.PAS, "mac:" + T.MAC_IZINLI)]
        for x in gizli:
            assert x not in s, x


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
