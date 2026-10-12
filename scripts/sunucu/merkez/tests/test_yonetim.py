import json
import os
import re
import sys
import tempfile

os.environ["WC_TEST"] = "1"  # netgsm.gonder gerçek isteği reddeder (yanlışlıkla SMS gitmesin)

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import kayitlar  # noqa: E402
import yonetici  # noqa: E402
import yonetim as YN  # noqa: E402
import cihaz as CZ  # noqa: E402
import guvenlik as G  # noqa: E402
import musteri as MU  # noqa: E402

WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"
PW = "yonetici-parola-1"


def kur(tmp, durum=None, gonderilen=None, **yeni_ek):
    v, k, saat = Y.ortam(tmp)
    yol, dyol = os.path.join(tmp, "yonetici.json"), os.path.join(tmp, "durum.json")
    yonetici.yaz(yol, "serkan", PW)
    with open(dyol, "w", encoding="utf-8") as f:
        json.dump({"zaman": "", "arsivler": durum or {}}, f)
    isler = []
    def sahte(ayar, metin, numaralar, zaman_asimi=10):
        gonderilen.append((ayar, metin, list(numaralar)))
        return "gonderildi"
    ek = {"gonder": sahte} if gonderilen is not None else {}
    ek.update(yeni_ek)
    app = YN.Yonetim(v, k, saat, yol, dyol, lambda islem, veri: isler.append((islem, veri)) or "0" * 16, **ek)
    c = Y.cerez(Y.giris(app, "serkan", PW))
    return app, v, saat, c, isler


def test_yonetici_dosyasi_ve_giris():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        yol = os.path.join(tmp, "y.json")
        try:
            yonetici.yaz(yol, "serkan", "kisa-parola")
            assert False
        except ValueError:
            pass
        assert yonetici.main(["parola", "serkan"], yol, sor=lambda: PW, sahip=None) == 0
        assert yonetici.oku(yol)["kullanici"] == "serkan"
        assert yonetici.main(["parola", "Kötü Ad"], yol, sor=lambda: PW, sahip=None) == 2
        app, v, _, c, _ = kur(tmp)
        assert c.startswith("wc_yonetim=")
        assert Y.giris(app, "serkan", "yanlis-parola-1")[0] == 401 and Y.giris(app, "baska", PW)[0] == 401
        n, pw = v.musteri_ekle()
        assert Y.giris(app, str(n), pw)[0] == 401  # müşteri yönetime giremez


def test_yeni_musteri_parola_bir_kez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        f = Y.al(app, "/yeni", c)
        assert 'name="parola"' in f[2].decode()
        r = Y.gonder(app, "/yeni", c, {"csrf": Y.csrf(f), "not": "x", "parola": "kisa", "parola2": "kisa"})
        assert r[0] == 400 and v.musteriler() == []  # parola en az 10 karakter
        r = Y.gonder(app, "/yeni", c, {"csrf": Y.csrf(f), "not": "x", "parola": "musteri-parola-1", "parola2": "baska-parola-1"})
        assert r[0] == 400 and v.musteriler() == []  # iki parola aynı olmalı
        r = Y.gonder(app, "/yeni", c, {"csrf": Y.csrf(f), "not": "Bocafe <Göztepe>", "parola": "musteri-parola-1", "parola2": "musteri-parola-1"})
        numara, pw = re.findall(r'class="sir">([^<]+)<', r[2].decode())
        assert r[0] == 200 and re.fullmatch(r"[1-9][0-9]{6}", numara) and pw == "musteri-parola-1" and v.parola_dogrula(int(numara), pw)
        assert pw in Y.al(app, f"/m/{numara}", c)[2].decode()  # 2026-10-04: parola yönetimde görünür (kullanıcı isteği)
        assert "Bocafe &lt;Göztepe&gt;" in Y.al(app, "/", c)[2].decode()
        assert v.hareketler(int(numara))[0]["olay"] == "MUSTERI_ACILDI"


def test_liste_durumlar_sorunlu_ustte_ve_arama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n1, _ = v.musteri_ekle("Sağlam kafe")
        n2, _ = v.musteri_ekle("Gecikmiş kafe")
        with open(app.durum_yolu, "w", encoding="utf-8") as f:
            json.dump({"zaman": "", "arsivler": {str(n2): {"tur": "cihaz", "son_gun": "2026-09-01", "boyut": 2048,
                                                          "el_sikisma": 1_790_000_000 - 60}}}, f)
        cid, _ = v.cihaz_bagla(n2, WG, SSH, "Kafe <B>")
        s = Y.al(app, "/", c)[2].decode()
        assert s.index("Gecikmiş kafe") < s.index("Sağlam kafe")
        assert "GECİKTİ" in s and "çevrimiçi" in s and "Kafe &lt;B&gt;" in s and "cihaz yok" in s
        s = Y.al(app, "/?q=sa%C4%9Flam", c)[2].decode()
        assert "Sağlam kafe" in s and "Gecikmiş kafe" not in s


def test_parola_sifirla_serbest_birak_uyelik():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, isler = kur(tmp)
        n, pw = v.musteri_ekle()
        v.cihaz_bagla(n, WG, SSH)
        f = Y.al(app, f"/m/{n}", c)
        t = Y.csrf(f)
        r = Y.gonder(app, f"/m/{n}/parola-sifirla", c, {"csrf": t, "parola": "yeni-musteri-1", "parola2": "yeni-musteri-1"})
        assert r[0] == 303 and v.parola_dogrula(n, "yeni-musteri-1") and not v.parola_dogrula(n, pw)
        assert Y.gonder(app, f"/m/{n}/serbest", c, {"csrf": t, "onay": "123"})[0] == 400  # numara yazılmadı
        assert Y.gonder(app, f"/m/{n}/serbest", c, {"csrf": t, "onay": str(n)})[0] == 303
        assert v.bagli_cihaz(n)["durum"] == "serbest_birakiliyor" and isler == []
        assert Y.gonder(app, f"/m/{n}/serbest", c, {"csrf": t, "onay": str(n), "zorla": "1"})[0] == 303
        assert v.bagli_cihaz(n) is None and isler == [("cihaz-kapat", {"numara": str(n), "wg_pub": WG})]
        Y.gonder(app, f"/m/{n}/uyelik", c, {"csrf": t, "islem": "bitir", "onay": str(n)})
        assert v.musteri(n)["uyelik"] == "bitti"
        Y.gonder(app, f"/m/{n}/uyelik", c, {"csrf": t, "islem": "ac"})
        assert v.musteri(n)["uyelik"] == "aktif"
        olaylar = [h["olay"] for h in v.hareketler(n)]
        for o in ("PAROLA_SIFIRLANDI", "SERBEST_BIRAKMA_ISTENDI", "CIHAZ_ZORLA_AYRILDI", "UYELIK_BITTI", "UYELIK_ACILDI"):
            assert o in olaylar, o
        assert Y.al(app, "/m/1234567" if n != 1234567 else "/m/7654321", c)[0] == 404
        assert Y.al(app, "/m/abc", c)[0] == 404 and Y.al(app, f"/m/{str(n)[:6]}", c)[0] == 404


def test_musteri_kayitlari_gerekce_ister():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        dun = kayitlar.son_30_gun(saat())[0].isoformat()
        Y.sayi_yaz(tmp, str(n), {dun: 9})
        Y.gun_yaz(tmp, str(n), "2026-09-20", [{"telefon": "5330000001", "ad": "Ayşe"}])
        assert ">9<" in Y.al(app, f"/m/{n}/kayitlar/", c)[2].decode()
        r = Y.al(app, f"/m/{n}/kayitlar/gun/2026-09-20", c)
        assert r[0] == 303 and f"donus=%2Fm%2F{n}%2Fkayitlar%2Fgun" in r[1]["Location"]
        Y.gerekce_ver(app, c)
        assert "Ayşe" in Y.al(app, f"/m/{n}/kayitlar/gun/2026-09-20", c)[2].decode()
        assert v.hareketler(n)[0]["olay"] == "GORUNTULEME" and v.hareketler(n)[0]["kim"] == "admin"


def test_eski_arsiv_baglama_ve_hesabim():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp, {"bocafe": {"tur": "eski", "son_gun": "2026-09-20", "boyut": 10, "el_sikisma": None},
                                    "1234567": {"tur": "cihaz", "son_gun": None, "boyut": 0, "el_sikisma": None}})
        n, _ = v.musteri_ekle()
        f = Y.al(app, "/eski-arsivler", c)
        s = f[2].decode()
        assert "<td>bocafe</td>" in s and "<td>1234567</td>" not in s  # müşteri arşivi eski arşiv adayı değil
        assert Y.gonder(app, "/eski-arsivler", c, {"csrf": Y.csrf(f), "ad": "yok-boyle", "musteri": str(n)})[0] == 400
        assert Y.gonder(app, "/eski-arsivler", c, {"csrf": Y.csrf(f), "ad": "bocafe", "musteri": "1234567" if n != 1234567 else "7654321"})[0] == 400
        assert Y.gonder(app, "/eski-arsivler", c, {"csrf": Y.csrf(f), "ad": "bocafe", "musteri": str(n)})[0] == 303
        assert v.arsivler(n) == [str(n), "bocafe"]
        h = Y.al(app, "/hesabim", c)
        assert Y.gonder(app, "/hesabim", c, {"csrf": Y.csrf(h), "mevcut": PW, "yeni": "kisa", "tekrar": "kisa"})[0] == 400
        assert Y.gonder(app, "/hesabim", c, {"csrf": Y.csrf(h), "mevcut": PW, "yeni": "yeni-yonetici-12", "tekrar": "yeni-yonetici-12"})[0] == 200
        assert Y.giris(app, "serkan", "yeni-yonetici-12")[0] == 303


def test_yonetici_parolasi_degisince_eski_oturum_duser():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        c2 = Y.cerez(Y.giris(app, "serkan", PW))
        h = Y.al(app, "/hesabim", c)
        r = Y.gonder(app, "/hesabim", c, {"csrf": Y.csrf(h), "mevcut": PW, "yeni": "yeni-yonetici-12", "tekrar": "yeni-yonetici-12"})
        assert r[0] == 200 and Y.al(app, "/", Y.cerez(r))[0] == 200  # değiştiren yeni oturumla devam eder
        assert Y.al(app, "/", c2)[0] == 303 and Y.al(app, "/", c)[0] == 303


def test_uyelik_bitir_onay_ister_yeniden_acma_serbest_birakmayi_geri_alir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        v.cihaz_bagla(n, WG, SSH)
        t = Y.csrf(Y.al(app, f"/m/{n}", c))
        assert Y.gonder(app, f"/m/{n}/uyelik", c, {"csrf": t, "islem": "bitir"})[0] == 400
        assert v.musteri(n)["uyelik"] == "aktif" and v.bagli_cihaz(n)["durum"] == "bagli"
        Y.gonder(app, f"/m/{n}/uyelik", c, {"csrf": t, "islem": "bitir", "onay": str(n)})
        assert v.musteri(n)["uyelik"] == "bitti" and v.bagli_cihaz(n)["durum"] == "serbest_birakiliyor"
        Y.gonder(app, f"/m/{n}/uyelik", c, {"csrf": t, "islem": "ac"})
        assert v.musteri(n)["uyelik"] == "aktif" and v.bagli_cihaz(n)["durum"] == "bagli"


def test_parola_yetki_birakilmadan_once_sorulur():
    with tempfile.TemporaryDirectory() as tmp:
        sira = []
        kod = yonetici.main(["parola", "serkan"], os.path.join(tmp, "y.json"), sor=lambda: sira.append("sor") or PW,
                            sahip="wcpanel", birak=lambda s: sira.append("birak"))
        assert kod == 0 and sira == ["sor", "birak"]  # wcpanel terminale erişemez: parola root iken sorulur


def test_parola_gorunur_ve_lisans_yonetimi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n, pw = v.musteri_ekle("Göztepe Bilgisayar")
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert pw in s and "2027-09-21" in s and "Lisans" in s
        t = Y.csrf(Y.al(app, f"/m/{n}", c))
        assert Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "uzat"})[0] == 303
        assert v.musteri(n)["lisans_bitis"] == "2028-09-20"
        Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "askiya"})
        assert v.musteri(n)["askida"] == 1
        Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "ac"})
        assert v.musteri(n)["askida"] == 0
        Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "tarih", "tarih": "2026-09-01"})
        assert v.musteri(n)["lisans_bitis"] == "2026-09-01"
        assert Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "tarih", "tarih": "bozuk"})[0] == 400
        liste = Y.al(app, "/", c)[2].decode()
        assert "Lisans" in liste and "süresi doldu" in liste
        olaylar = [h["olay"] for h in v.hareketler(n)]
        for o in ("LISANS_UZATILDI", "LISANS_ASKIYA_ALINDI", "LISANS_ACILDI", "LISANS_TARIHI"):
            assert o in olaylar, o
        v2, _ = v.musteri_ekle()
        v.db.execute("UPDATE musteri SET parola_acik = '' WHERE numara = ?", (v2,))
        assert "bilinmiyor" in Y.al(app, f"/m/{v2}", c)[2].decode()


def test_sifirlama_sayfasi_ve_lisans_tarihi_kaydi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        t = Y.csrf(Y.al(app, f"/m/{n}", c))
        r = Y.gonder(app, f"/m/{n}/parola-sifirla", c, {"csrf": t, "parola": "kisa", "parola2": "kisa"})
        assert r[0] == 400 and "10" in r[2].decode()  # kısa parola reddedilir, rastgele parola üretilmez
        Y.gonder(app, f"/m/{n}/lisans", c, {"csrf": t, "islem": "tarih", "tarih": "20270101"})
        assert v.hareketler(n)[0]["ayrinti"] == "bitis=2027-01-01"  # girilen değil, kaydedilen tarih
        assert "gününden sonra" in Y.al(app, f"/m/{n}", c)[2].decode()


def test_tek_admin_parolasi_sayfasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        v.cihaz_bagla(n, WG, SSH)
        assert f"/m/{n}/admin-parola" not in Y.al(app, f"/m/{n}", c)[2].decode()  # cihaz başına admin parolası yok
        s = Y.al(app, "/admin-parolasi", c)[2].decode()
        assert 'href="/admin-parolasi"' in Y.al(app, "/", c)[2].decode() and "tanımsız" in s
        t = Y.csrf(Y.al(app, "/admin-parolasi", c))
        r = Y.gonder(app, "/admin-parolasi", c, {"csrf": t, "parola": "benim-admin-parolam", "parola2": "baska"})
        assert r[0] == 400 and v.admin_parolasi() is None
        Y.gonder(app, "/admin-parolasi", c, {"csrf": t, "parola": "benim-admin-parolam", "parola2": "benim-admin-parolam"})
        a = v.admin_parolasi()
        assert a is not None and "tanımlı" in Y.al(app, "/admin-parolasi", c)[2].decode()
        assert all("benim-admin-parolam" not in h["ayrinti"] for h in v.hareketler())
        assert any(h["olay"] == "ADMIN_PAROLASI" for h in v.hareketler())


def test_sms_ayarlari_sayfasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        s = Y.al(app, "/sms", c)[2].decode()
        assert 'name="usercode"' in s and 'name="password"' in s
        t = Y.csrf(Y.al(app, "/sms", c))
        r = Y.gonder(app, "/sms", c, {"csrf": t, "usercode": "850", "msgheader": "BASLIK"})  # deneme modu kapalı, şifre yok
        assert r[0] == 400 and "password" in r[2].decode() and v.sms_ayari() is None
        Y.gonder(app, "/sms", c, {"csrf": t, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr"})
        assert v.sms_ayari()["password"] == "gizli-1" and v.sms_ayari()["mock"] is False
        s = Y.al(app, "/sms", c)[2].decode()
        assert "gizli-1" not in s and "tanımlı" in s  # şifre geri gösterilmez
        Y.gonder(app, "/sms", c, {"csrf": t, "usercode": "8503027084", "password": "", "msgheader": "gztp.blgsyr", "mock": "1"})
        assert v.sms_ayari()["password"] == "gizli-1" and v.sms_ayari()["mock"] is True  # boş şifre = aynı kalır
        assert any(h["olay"] == "SMS_AYARI" and "gizli-1" not in h["ayrinti"] for h in v.hareketler())


def _rapor(**d):
    r = {"surum": "1.0.<x>", "acik_sn": 90000, "servisler": {"portal": True, "panel": True, "kaydedici": False, "dnsmasq": True,
         "guvenlik": True}, "disk_yuzde": 91, "son_muhur": "2026-10-10", "kayit_boslugu_1s": False, "wan": True,
         "sms_sorunu": False, "bagli": 7, "hiz_hata": False}
    r.update(d)
    return r


def test_acik_sorunlar_karti():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n1, _ = v.musteri_ekle("a")
        n2, _ = v.musteri_ekle("b")
        assert "Açık sorunlar" not in Y.al(app, "/", c)[2].decode()  # sorun yoksa kart yok
        v.cihaz_bagla(n1, WG, SSH, "Kafe <B>")
        t0 = saat.t
        saat.t -= 7200  # sorunlar iki saat önce açıldı (oturum bozulmasın diye saati geri alıp kaydet)
        i1 = v.sorun_ac(n1, "kayit_durdu", "5651 kaydı durdu <i>", "kritik")
        v.sorun_ac(n2, "lisans_bitti", "Lisans bitti (01.10.2026)", "bilgi")
        v.sorun_sms(i1, "acilis_sms", "gonderildi")
        saat.t = t0
        s = Y.al(app, "/", c)[2].decode()
        assert "Açık sorunlar" in s and f'href="/m/{n1}"' in s and f'href="/m/{n2}"' in s
        assert "Kafe &lt;B&gt;" in s and "5651 kaydı durdu &lt;i&gt;" in s and "durdu <i>" not in s
        assert "2 saat önce" in s and "09:00&#x27;da gidecek" in s and "SMS gitti" in s and "kritik" in s
        satir = s[s.index(f'<a href="/m/{n1}">{n1}</a></td><td>Kafe'):]
        assert "5651 kaydı durdu" in satir[:satir.index("</tr>")]  # liste satırındaki sorunlu rozeti
        v.sorun_kapat(i1)
        s = Y.al(app, "/", c)[2].decode()
        assert "5651 kaydı durdu" not in s


def test_sms_durum_etiketleri():
    f = YN.sms_etiketi
    assert f("", "kritik") == "SMS bekleniyor" and f("", "uyari") == "SMS bekleniyor" and f("", "bilgi") == "09:00'da gidecek"
    assert f("gonderildi", "kritik") == "SMS gitti" and f("mock", "uyari") == "deneme modu"
    assert f("hata:30", "kritik") == "SMS gönderilemedi (30)" and f("sinir", "uyari") == "günlük SMS sınırı"
    assert f("yok", "kritik") == "SMS gönderilmedi (uyarılar kapalı)"
    assert f("sinir:2026-10-11", "kritik") == "günlük SMS sınırı" and f("sinir:2026-10-11", "bilgi") == "günlük SMS sınırı"


def test_gecmis_sms_etiketleri():
    t = 1_790_000_000.0
    iso = YN.common.now_iso
    f = lambda **d: YN.gecmis_sms_etiketleri({"onem": "kritik", "acilis_sms": "", "kapanis_sms": "", "kapanis": "", **d}, t)
    assert f() == ("SMS bekleniyor", "—") and f(onem="bilgi") == ("09:00'da gidecek", "—")  # açık
    # kapanmış, hiç SMS'siz: sonsuza dek "bekleniyor" demez
    assert f(kapanis=iso(t - 5000)) == ("—", "—") and f(kapanis=iso(t - 5000), onem="bilgi") == ("—", "—")
    assert f(kapanis=iso(t - 100), acilis_sms="gonderildi") == ("SMS gitti", "düzeldi bekletiliyor")
    assert f(kapanis=iso(t - 100), acilis_sms="mock") == ("deneme modu", "düzeldi bekletiliyor")
    assert f(kapanis=iso(t - 700), acilis_sms="gonderildi") == ("SMS gitti", "—")
    assert f(kapanis=iso(t - 100), acilis_sms="sinir:2026-09-21") == ("günlük SMS sınırı", "—")  # açılış gitmedi: düzeldi de yok
    assert f(kapanis=iso(t - 5000), acilis_sms="gonderildi", kapanis_sms="gonderildi") == ("SMS gitti", "SMS gitti")
    assert f(kapanis=iso(t - 5000), acilis_sms="gonderildi", kapanis_sms="sinir:2026-09-21") == ("SMS gitti", "günlük SMS sınırı")


def test_izleme_satiri_ana_sayfada():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        s = Y.al(app, "/", c)[2].decode()
        assert '<b class="kotu">Sağlık izleme: hiç tur yok — izleme çalışmıyor olabilir</b>' in s  # hiç çalışmadı
        v.saglik_son_tur_koy(YN.common.now_iso(saat.t - 180))
        s = Y.al(app, "/", c)[2].decode()
        assert "Sağlık izleme: son tur 3 dk önce" in s and "izleme çalışmıyor olabilir" not in s
        v.saglik_son_tur_koy(YN.common.now_iso(saat.t - 16 * 60))
        s = Y.al(app, "/", c)[2].decode()
        assert '<b class="kotu">Sağlık izleme: son tur 16 dk önce — izleme çalışmıyor olabilir</b>' in s


def test_musteri_saglik_ve_gecmis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "Henüz rapor yok" in s and "Sorun geçmişi" in s
        cid, _ = v.cihaz_bagla(n, WG, SSH, "Kafe")
        assert "Henüz rapor yok" in Y.al(app, f"/m/{n}", c)[2].decode()
        t0 = saat.t
        saat.t -= 600
        v.saglik_kaydet(cid, _rapor())
        saat.t = t0
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "Henüz rapor yok" not in s and "10 dk önce" in s
        assert "1.0.&lt;x&gt;" in s and "1.0.<x>" not in s  # cihaz verisi kaçırılır
        assert "kaydedici" in s and "%91" in s and "2026-10-10" in s and ">7<" in s
        saat.t -= 3600
        i = v.sorun_ac(n, "disk_dolu", "Disk dolu (%91)", "uyari")
        saat.t += 1800
        v.sorun_kapat(i)
        saat.t = t0
        assert "Disk dolu (%91)" in Y.al(app, f"/m/{n}", c)[2].decode()
        for k in range(60):
            v.sorun_ac(n, f"t{k}", f"Sorun {k}", "uyari")
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "Sorun 59" in s and "Sorun 5<" not in s and "Disk dolu" not in s  # son 50


def test_sorun_gecmisi_kapanmis_satir_etiketleri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        t0 = saat.t
        saat.t -= 3600
        i = v.sorun_ac(n, "rapor_yok", "Rapor gelmiyor", "kritik")
        j = v.sorun_ac(n, "lisans_bitiyor", "Lisans bitiyor (01.10.2026)", "bilgi")
        k = v.sorun_ac(n, "disk_dolu", "Disk dolu (%93)", "uyari")
        v.sorun_sms(k, "acilis_sms", "gonderildi")
        saat.t -= 1800
        v.sorun_kapat(i), v.sorun_kapat(j)  # SMS'siz kapandı
        saat.t = t0 - 100
        v.sorun_kapat(k)  # açılışı gitmiş, 100 sn önce kapandı
        saat.t = t0
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "SMS bekleniyor" not in s and "09:00&#x27;da gidecek" not in s and "düzeldi bekletiliyor" in s


def test_uyari_ayarlari_kaydet_ve_dogrula():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        s = Y.al(app, "/uyari", c)[2].decode()
        assert 'name="tel1"' in s and 'name="tel3"' in s and 'name="acik"' in s and 'name="gunluk_sinir"' in s
        assert 'href="/uyari"' in s and "Uyarı ayarları" in s
        t = Y.csrf(Y.al(app, "/uyari", c))
        r = Y.gonder(app, "/uyari", c, {"csrf": t, "tel1": "0533 111 22 33", "tel2": "", "tel3": "05441112244", "acik": "1", "gunluk_sinir": "15"})
        assert r[0] == 303
        assert v.uyari_ayari() == {"telefonlar": ["905331112233", "905441112244"], "acik": True, "gunluk_sinir": 15}
        olay = [h for h in v.hareketler() if h["olay"] == "UYARI_AYAR"]
        assert olay and "5331112233" not in olay[0]["ayrinti"]
        s = Y.al(app, "/uyari", c)[2].decode()
        assert "5331112233" in s and "checked" in s and 'value="15"' in s
        r = Y.gonder(app, "/uyari", c, {"csrf": t, "tel1": "123", "gunluk_sinir": "15"})  # geçersiz telefon
        assert r[0] == 400 and "Geçersiz telefon" in r[2].decode() and v.uyari_ayari()["gunluk_sinir"] == 15
        for kotu in ("0", "1", "101", "abc", "", "2.5"):
            r = Y.gonder(app, "/uyari", c, {"csrf": t, "tel1": "05331112233", "gunluk_sinir": kotu})
            assert r[0] == 400, kotu
        assert v.uyari_ayari()["gunluk_sinir"] == 15
        r = Y.gonder(app, "/uyari", c, {"csrf": t, "tel1": "05331112233", "gunluk_sinir": "20"})  # onay kutusu işaretsiz = kapalı
        assert r[0] == 303 and v.uyari_ayari()["acik"] is False
        r = Y.gonder(app, "/uyari", c, {"csrf": t, "tel1": "05331112233", "gunluk_sinir": "2"})  # en az 2
        assert r[0] == 303 and v.uyari_ayari()["gunluk_sinir"] == 2


def test_deneme_sms_sahte_gondericiyle():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        giden = []
        app, v, saat, c, _ = kur(tmp, gonderilen=giden)
        t = Y.csrf(Y.al(app, "/uyari", c))
        v.uyari_ayari_koy({"telefonlar": ["05331112233", "05441112244"], "acik": True, "gunluk_sinir": 20})
        r = Y.gonder(app, "/uyari/deneme", c, {"csrf": t})  # SMS ayarı yok
        assert r[0] == 200 and "SMS ayarları girilmemiş" in r[2].decode() and giden == []
        v.sms_ayari_koy({"mock": False, "usercode": "u", "password": "p", "msgheader": "h"})
        r = Y.gonder(app, "/uyari/deneme", c, {"csrf": t})
        assert r[0] == 200 and "gönderildi" in r[2].decode()
        assert len(giden) == 1 and giden[0][1] == "WifiCorrect deneme uyarısı" and giden[0][2] == ["905331112233"]
        assert giden[0][0]["usercode"] == "u"
        gun = YN.datetime.datetime.fromtimestamp(saat(), YN.common.TZ).date().isoformat()
        assert v.sms_sayaci(gun) == 1
        assert v.hareketler()[0]["olay"] == "UYARI_DENEME"
        v.uyari_ayari_koy({"telefonlar": [], "acik": True, "gunluk_sinir": 20})
        r = Y.gonder(app, "/uyari/deneme", c, {"csrf": t})
        assert "telefon" in r[2].decode() and len(giden) == 1


def test_uyari_csrf_siz_post_reddi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        giden = []
        app, v, _, c, _ = kur(tmp, gonderilen=giden)
        v.uyari_ayari_koy({"telefonlar": ["05331112233"], "acik": True, "gunluk_sinir": 20})
        v.sms_ayari_koy({"mock": False, "usercode": "u", "password": "p", "msgheader": "h"})
        for yol, alan in (("/uyari", {"tel1": "05441112244", "gunluk_sinir": "5"}), ("/uyari/deneme", {})):
            for form in (alan, dict(alan, csrf="yanlis")):
                assert Y.gonder(app, yol, c, form)[0] == 403, yol
        assert giden == [] and v.uyari_ayari()["gunluk_sinir"] == 20 and v.sms_sayaci("2026-09-21") == 0



def test_yonetimden_cihaz_paneline_dogrudan_gecis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, k, saat = Y.ortam(tmp)
        yol, dyol = os.path.join(tmp, "yonetici.json"), os.path.join(tmp, "durum.json")
        yonetici.yaz(yol, "serkan", PW)
        with open(dyol, "w", encoding="utf-8") as f:
            json.dump({"zaman": "", "arsivler": {}}, f)
        gecis = G.GecisBelirtecleri()
        mu = MU.Musteri(v, k, saat, gecis=gecis)
        cz = CZ.Cihaz(v, gecis, mu, saat)
        app = YN.Yonetim(v, k, saat, yol, dyol, lambda *a: "0" * 16, gecis=gecis, musteri_app=mu)
        c = Y.cerez(Y.giris(app, "serkan", PW))
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        v.cihaz_bagla(n, WG, SSH)
        # tünel adresi yok → düğme yok, geçiş reddedilir
        f = Y.al(app, f"/m/{n}", c)
        s = f[2].decode()
        assert "Cihaz paneline git" not in s and "şu an ulaşılamıyor" in s
        t = Y.csrf(f)
        assert Y.gonder(app, f"/m/{n}/cihaz-paneli", c, {"csrf": t})[0] == 409
        v.cihaz_guncelle(v.bagli_cihaz(n)["id"], tunel_ip="10.99.0.11")
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert f'action="/m/{n}/cihaz-paneli"' in s and "Cihaz paneline git" in s
        assert Y.gonder(app, f"/m/{n}/cihaz-paneli", c, {"csrf": "yanlis"})[0] == 403
        r = Y.gonder(app, f"/m/{n}/cihaz-paneli", c, {"csrf": t})
        assert r[0] == 303 and r[1]["Location"].startswith("https://cihaz.wificorrect.com/_giris?t=")
        assert v.hareketler(n)[0]["olay"] == "YONETICI_CIHAZ_PANELI"
        h = {"Host": "cihaz.wificorrect.com"}
        r = cz.istek("GET", "/_giris", h, "", "t=" + r[1]["Location"].split("t=", 1)[1], "1.2.3.4")
        wcc = r[1]["Set-Cookie"].split(";")[0].split("=", 1)[1]
        y = cz.istek("GET", "/cihaz-yetki", dict(h, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")
        assert y[0] == 200 and y[1]["X-WFC-Cihaz"] == "10.99.0.11" and y[1]["X-WFC-Kullanici"] == str(n)
        # müşterinin kendi cihaz oturumu yöneticinin çıkışıyla düşmez; yönetici müşteri sayfasına döner
        mw = cz.oturumlar.create(str(n), "musteri", saat(), v.musteri(n)["ozet"])
        r = cz.istek("GET", "/_cikis", dict(h, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")
        assert r[0] == 303 and r[1]["Location"] == f"https://yonetim.wificorrect.com/m/{n}"
        assert cz.istek("GET", "/cihaz-yetki", dict(h, Cookie=f"wcc={wcc}"), "", "", "1.2.3.4")[0] == 302
        assert cz.istek("GET", "/cihaz-yetki", dict(h, Cookie=f"wcc={mw}"), "", "", "1.2.3.4")[0] == 200


# --- uzaktan güncelleme: Sürümler sayfası, müşteri kartı, liste sütunu ---
def _surum_ekle(v, s="1.2.0"):
    v.surum_ekle(s, "ab" * 32, 2048, "github")


def test_surumler_yonetici_disina_kapali_ve_tablo():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        assert Y.al(app, "/surumler", "")[0] in (302, 303, 401, 403)
        n, _ = v.musteri_ekle()
        cid, _ = v.cihaz_bagla(n, WG, SSH)
        _surum_ekle(v)
        v.saglik_kaydet(cid, {"paket": "1.2.0"})
        s = Y.al(app, "/surumler", c)[2].decode()
        assert "1.2.0" in s and "github" in s and "ab" * 6 in s and "ab" * 7 not in s
        assert 'href="/surumler"' in s
        satir = re.search(r"<tr><td>1\.2\.0</td>.*?</tr>", s, re.S).group(0)
        assert "<td>1</td>" in satir  # kurulu cihaz sayısı


def test_surumler_github_al_mesaji_ve_hareket():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        cagri = []

        def sahte(v_, dizin, getir):
            cagri.append(dizin)
            v_.surum_ekle("1.3.0", "cd" * 32, 10, "github")
            return ["1.3.0"], ["1.2.9: SHA256 tutmuyor"]
        app, v, _, c, _ = kur(tmp, surum_dizini=os.path.join(tmp, "sd"), github_al=sahte)
        t = Y.csrf(Y.al(app, "/surumler", c))
        assert Y.gonder(app, "/surumler/al", c, {"csrf": "yanlis"})[0] == 403 and cagri == []
        r = Y.gonder(app, "/surumler/al", c, {"csrf": t})
        s = r[2].decode()
        assert r[0] == 200 and "1.3.0" in s and "SHA256 tutmuyor" in s and cagri == [os.path.join(tmp, "sd")]
        assert v.surum("1.3.0") is not None
        assert any(h["olay"] == "SURUM_ALINDI" and "surum=1.3.0" in h["ayrinti"] for h in v.hareketler())


def test_surumler_github_listesi_alinamazsa_hata_mesaji():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        def bozuk(v_, dizin, getir):
            raise OSError("ag yok")
        app, v, _, c, _ = kur(tmp, github_al=bozuk)
        t = Y.csrf(Y.al(app, "/surumler", c))
        r = Y.gonder(app, "/surumler/al", c, {"csrf": t})
        assert r[0] == 200 and "alınamadı" in r[2].decode().lower()


def test_surumler_hedef_secimi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        _surum_ekle(v)
        t = Y.csrf(Y.al(app, "/surumler", c))
        assert Y.gonder(app, "/surumler/hedef", c, {"csrf": "yanlis", "kanal": "test", "surum": "1.2.0"})[0] == 403
        r = Y.gonder(app, "/surumler/hedef", c, {"csrf": t, "kanal": "test", "surum": "1.2.0"})
        assert r[0] == 303 and v.kanal_hedef("test") == "1.2.0"
        assert any(h["olay"] == "KANAL_HEDEF" and "kanal=test" in h["ayrinti"] and "surum=1.2.0" in h["ayrinti"] for h in v.hareketler())
        assert 'value="1.2.0" selected' in Y.al(app, "/surumler", c)[2].decode()
        # geçersiz sürüm / kanal reddedilir, değişmez
        assert Y.gonder(app, "/surumler/hedef", c, {"csrf": t, "kanal": "test", "surum": "9.9.9"})[0] == 400
        assert Y.gonder(app, "/surumler/hedef", c, {"csrf": t, "kanal": "kapali", "surum": "1.2.0"})[0] == 400
        assert Y.gonder(app, "/surumler/hedef", c, {"csrf": t, "kanal": "kararli", "surum": "../x"})[0] == 400
        assert v.kanal_hedef("test") == "1.2.0" and v.kanal_hedef("kararli") is None
        # boş = hedef yok
        assert Y.gonder(app, "/surumler/hedef", c, {"csrf": t, "kanal": "test", "surum": ""})[0] == 303
        assert v.kanal_hedef("test") is None


def test_musteri_guncelleme_karti_kanal_ve_hemen():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        cid, _ = v.cihaz_bagla(n, WG, SSH)
        _surum_ekle(v)
        v.kanal_hedef_koy("kararli", "1.2.0")
        f = Y.al(app, f"/m/{n}", c)
        s, t = f[2].decode(), Y.csrf(f)
        assert "Güncelleme" in s and f'action="/m/{n}/guncelleme"' in s and f'action="/m/{n}/guncelleme-hemen"' in s
        assert "Hedef sürüm" in s and "1.2.0" in s
        # CSRF'siz POST reddedilir
        assert Y.gonder(app, f"/m/{n}/guncelleme", c, {"kanal": "test"})[0] == 403
        assert Y.gonder(app, f"/m/{n}/guncelleme-hemen", c, {"csrf": "yanlis"})[0] == 403
        assert v.musteri(n)["guncelleme_kanali"] == "kararli" and v.musteri(n)["guncelleme_hemen"] == 0
        assert Y.gonder(app, f"/m/{n}/guncelleme", c, {"csrf": t, "kanal": "bilinmez"})[0] == 400
        assert Y.gonder(app, f"/m/{n}/guncelleme", c, {"csrf": t, "kanal": "test"})[0] == 303
        assert v.musteri(n)["guncelleme_kanali"] == "test"
        assert any(h["olay"] == "GUNCELLEME_KANALI" and h["musteri"] == n and "kanal=test" in h["ayrinti"] for h in v.hareketler(n))
        v.guncelleme_kanali_koy(n, "kararli")
        assert "bekleniyor" not in Y.al(app, f"/m/{n}", c)[2].decode()
        assert Y.gonder(app, f"/m/{n}/guncelleme-hemen", c, {"csrf": t})[0] == 303
        assert v.musteri(n)["guncelleme_hemen"] == saat() and v.musteri(n)["guncelleme_hemen_surum"] == "1.2.0"
        assert any(h["olay"] == "GUNCELLEME_HEMEN" for h in v.hareketler(n))
        assert "bekleniyor" in Y.al(app, f"/m/{n}", c)[2].decode()


def test_simdi_guncelle_hedef_yoksa_reddedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, saat, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        v.cihaz_bagla(n, WG, SSH)
        _surum_ekle(v)
        t = Y.csrf(Y.al(app, f"/m/{n}", c))
        # kararlı kanalda hedef yok
        d, _, g = Y.gonder(app, f"/m/{n}/guncelleme-hemen", c, {"csrf": t})
        assert d == 400 and "hedef" in g.decode()
        assert v.musteri(n)["guncelleme_hemen"] == 0
        # kanal kapalı
        v.kanal_hedef_koy("kararli", "1.2.0")
        v.guncelleme_kanali_koy(n, "kapali")
        assert Y.gonder(app, f"/m/{n}/guncelleme-hemen", c, {"csrf": t})[0] == 400
        assert v.musteri(n)["guncelleme_hemen"] == 0
        assert not any(h["olay"] == "GUNCELLEME_HEMEN" for h in v.hareketler(n))


def test_musteri_guncelleme_karti_paket_sonuc_gecmis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n, _ = v.musteri_ekle()
        cid, _ = v.cihaz_bagla(n, WG, SSH)
        assert "Paketsiz kurulum" not in Y.al(app, f"/m/{n}", c)[2].decode()
        v.saglik_kaydet(cid, {"paket": ""})
        assert "Paketsiz kurulum — uzaktan güncellenmez" in Y.al(app, f"/m/{n}", c)[2].decode()
        v.saglik_kaydet(cid, {"paket": "1.1.0", "guncelleme": {"surum": "1.2.0", "sonuc": "geri_alindi", "zaman": 1790000000}})
        for i in range(12):
            v.hareket(f"cihaz:{cid}", "10.99.0.11", "GUNCELLEME_SONUC", n, f"surum=1.0.{i} sonuc=hata")
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "Kurulu paket" in s and "1.1.0" in s and "Geri alındı" in s
        assert s.count("surum=1.0.") == 10 and "surum=1.0.11 " in s and "surum=1.0.1 " not in s


def test_liste_surum_sutunu_hedefle_farkli_sari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, _, c, _ = kur(tmp)
        n1, _ = v.musteri_ekle()
        c1, _ = v.cihaz_bagla(n1, WG, SSH)
        n2, _ = v.musteri_ekle()
        c2, _ = v.cihaz_bagla(n2, "B" * 43 + "=", SSH)
        n3, _ = v.musteri_ekle()
        _surum_ekle(v)
        v.kanal_hedef_koy("kararli", "1.2.0")
        v.saglik_kaydet(c1, {"paket": "1.1.0"})
        v.saglik_kaydet(c2, {"paket": "1.2.0"})
        s = Y.al(app, "/", c)[2].decode()
        assert "<th>Sürüm</th>" in s
        sat = {n: re.search(rf'<tr><td><a href="/m/{n}">.*?</tr>', s, re.S).group(0) for n in (n1, n2, n3)}
        assert 'class="uyarisi">1.1.0' in sat[n1] and 'class="uyarisi"' not in sat[n2] and "1.2.0" in sat[n2]
        assert "—" in sat[n3] and 'class="uyarisi"' not in sat[n3]


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
