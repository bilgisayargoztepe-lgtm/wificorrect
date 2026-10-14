import datetime
import json
import os
import sys
import tempfile
import xml.etree.ElementTree as ET

os.environ["WC_TEST"] = "1"  # netgsm.gonder gerçek isteği reddeder (yanlışlıkla SMS gitmesin)

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import common  # noqa: E402
import netgsm as N  # noqa: E402
import saglik as S  # noqa: E402
import veri as V  # noqa: E402


def yeni(tmp, saat=None):
    return V.Veri(os.path.join(tmp, "m.db"), saat=saat or (lambda: 1_790_000_000.0))


def test_veri_saglik_tek_satir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        t = [1_790_000_000.0]
        v = yeni(tmp, lambda: t[0])
        assert v.saglik(7) is None
        v.saglik_kaydet(7, {"disk": 41, "servisler": ["a"]})
        s1 = v.saglik(7)
        assert s1["rapor"] == {"disk": 41, "servisler": ["a"]} and s1["zaman"].startswith("2026-")
        t[0] += 300
        v.saglik_kaydet(7, {"disk": 42})
        s2 = v.saglik(7)
        assert s2["rapor"] == {"disk": 42} and s2["zaman"] > s1["zaman"]
        assert v._bir("SELECT COUNT(*) n FROM saglik")["n"] == 1


def test_sorun_ac_ayni_turu_tekrar_acmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        a = v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%91)", "uyari")
        assert v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%95)", "uyari") == a
        assert v.sorun_ac(1000001, "rapor_yok", "Rapor gelmiyor", "kritik") != a
        assert v.sorun_ac(1000002, "disk_dolu", "Disk dolu (%91)", "uyari") != a
        assert len(v.acik_sorunlar()) == 3 and len(v.acik_sorunlar(1000001)) == 2
        assert v.acik_sorunlar(1000001)[0]["kapanis"] == ""


def test_sorun_kapat_ve_gecmis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        t = [1_790_000_000.0]
        v = yeni(tmp, lambda: t[0])
        assert v.son_kapanan(5, "disk_dolu") is None
        a = v.sorun_ac(5, "disk_dolu", "Disk dolu (%91)", "uyari")
        v.sorun_sms(a, "acilis_sms", "mock")
        t[0] += 600
        v.sorun_kapat(a)
        v.sorun_sms(a, "kapanis_sms", "hata:30")
        assert v.acik_sorunlar(5) == []
        k = v.son_kapanan(5, "disk_dolu")
        assert k["id"] == a and k["kapanis"] > k["acilis"] and k["acilis_sms"] == "mock" and k["kapanis_sms"] == "hata:30"
        b = v.sorun_ac(5, "disk_dolu", "Disk dolu (%92)", "uyari")  # kapandıktan sonra yenisi açılır
        assert b != a
        g = v.sorun_gecmisi(5)
        assert [x["id"] for x in g] == [b, a] and len(v.sorun_gecmisi(5, limit=1)) == 1
        try:
            v.sorun_sms(a, "baska", "mock")
            assert False
        except ValueError:
            pass


def test_uyari_ayari_dogrulama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.uyari_ayari() == {"telefonlar": [], "acik": False, "gunluk_sinir": 20}
        v.uyari_ayari_koy({"telefonlar": ["0533 455 31 32"], "acik": True, "gunluk_sinir": 5})
        assert v.uyari_ayari() == {"telefonlar": ["905334553132"], "acik": True, "gunluk_sinir": 5}
        for kotu in ({"telefonlar": ["0533 455 31 32", "0533 455 31 33", "0533 455 31 34", "0533 455 31 35"]},
                     {"telefonlar": ["12345"]}):
            try:
                v.uyari_ayari_koy(kotu)
                assert False
            except ValueError:
                pass
        assert v.uyari_ayari()["telefonlar"] == ["905334553132"]  # hatalı kayıt eskisini bozmaz
        for kotu in (0, 1, 101, True, 2.5, "x", None):  # sınır 2–100 tamsayı (1: hiç uyarı gitmezdi)
            try:
                v.uyari_ayari_koy({"telefonlar": [], "gunluk_sinir": kotu})
                assert False, kotu
            except ValueError:
                pass
        for iyi in (2, 100, "7"):
            v.uyari_ayari_koy({"telefonlar": [], "gunluk_sinir": iyi})
            assert v.uyari_ayari()["gunluk_sinir"] == int(iyi)


def test_sms_sayaci_gun_bazli():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.sms_sayaci("2026-10-11") == 0
        assert v.sms_say("2026-10-11") == 1 and v.sms_say("2026-10-11") == 2
        assert v.sms_sayaci("2026-10-11") == 2 and v.sms_sayaci("2026-10-12") == 0


# --- Task 3: netgsm + kurallar + SMS kararı ---
def ts(iso):
    return datetime.datetime.fromisoformat(iso).replace(tzinfo=common.TZ).timestamp()


T0 = ts("2026-10-11T20:15:00")  # TR saati
BUGUN = "2026-10-11"
SERVISLER = ("portal", "panel", "kaydedici", "dnsmasq", "guvenlik")


def rapor(**d):
    r = {"surum": "1.0", "acik_sn": 100, "servisler": {a: True for a in SERVISLER}, "disk_yuzde": 40,
         "son_muhur": "2026-10-10", "kayit_boslugu_1s": False, "wan": True, "sms_sorunu": False, "bagli": 3, "hiz_hata": False}
    for k, v in d.items():
        if k in SERVISLER:
            r["servisler"][k] = v
        else:
            r[k] = v
    return r


M = {"numara": 1537344, "askida": 0, "uyelik": "aktif", "lisans_bitis": "2027-10-11"}
C = {"id": 1, "durum": "bagli", "isletme_adi": "Göztepe"}


def turler(m=M, c=C, r="varsayilan", rz=T0 - 60, el=T0 - 60, sg=BUGUN, simdi=T0):
    return [t for t, _, _ in S.sorunlar(m, c, rapor() if r == "varsayilan" else r, rz, el, sg, simdi)]


def test_netgsm_xml_ve_yanit():
    x = N.xml_govde("u&1", "p<2", "BAŞLIK", "a]]>b ç", ["905334553132", "5334553133", "12"])
    k = ET.fromstring(x.encode("utf-8"))
    assert k.find("header/usercode").text == "u&1" and k.find("header/password").text == "p<2"
    assert k.find("header/type").text == "1:n" and k.find("header/msgheader").text == "BAŞLIK"
    assert k.find("body/msg").text == "a]]>b ç"  # CDATA'dan taşmaz
    assert [n.text for n in k.findall("body/no")] == ["5334553132", "5334553133"]
    assert N.yanit_kodu("00 123456") == "00" and N.yanit_kodu("0") == "0" and N.yanit_kodu("30\n") == "30"
    assert N.yanit_kodu("<xml><main><code>40</code></main></xml>") == "40" and N.yanit_kodu("") == ""
    ayar = {"mock": False, "usercode": "u", "password": "gizli", "msgheader": "H"}

    def ag_yok(*a, **k):
        raise AssertionError("ağ isteği")
    assert N.gonder({**ayar, "mock": True}, "m", ["905334553132"], ac=ag_yok) == "mock"
    assert N.gonder(None, "m", ["905334553132"], ac=ag_yok) == "hata:AYAR"
    assert N.gonder({**ayar, "password": ""}, "m", ["905334553132"], ac=ag_yok) == "hata:AYAR"
    assert N.gonder(ayar, "m", ["123"], ac=ag_yok) == "hata:NUMARA"
    istek = []

    class Yanit:
        def __init__(self, b):
            self.b = b

        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def read(self, n=-1):
            return self.b

    def ac(cevap):
        def f(req, timeout):
            istek.append((req, timeout))
            if isinstance(cevap, Exception):
                raise cevap
            return Yanit(cevap)
        return f
    assert N.gonder(ayar, "merhaba", ["905334553132"], ac=ac(b"00 987654")) == "gonderildi"
    req, zaman_asimi = istek[-1]
    assert req.full_url == "https://api.netgsm.com.tr/sms/send/xml" and req.get_method() == "POST" and zaman_asimi == 10
    assert req.get_header("Content-type") == "application/xml; charset=UTF-8"
    assert b"<no>5334553132</no>" in req.data and b"merhaba" in req.data
    assert N.gonder(ayar, "m", ["905334553132"], ac=ac(b"30")) == "hata:30"
    assert N.gonder(ayar, "m", ["905334553132"], ac=ac(b"<html>bozuk")) == "hata:BILINMIYOR"
    assert N.gonder(ayar, "m", ["905334553132"], ac=ac(OSError("yok"))) == "hata:AG"
    assert N.gonder(ayar, "m", ["905334553132"], ac=ac(TimeoutError())) == "hata:AG"


def test_kural_saglam_cihazda_sorun_yok():
    assert turler() == []


def test_kural_ulasilamiyor_diger_kritikleri_bastirir():
    kotu = rapor(kaydedici=False, portal=False, disk_yuzde=95)
    assert turler(r=kotu, rz=T0 - 2000, el=T0 - 601) == ["ulasilamiyor", "disk_dolu"]
    assert turler(r=kotu, rz=T0 - 2000, el=None) == ["ulasilamiyor", "disk_dolu"]  # hiç el sıkışmamış
    assert turler(r=kotu, rz=T0 - 2000, el=T0 - 60) == ["rapor_yok", "kayit_durdu", "servis_durdu", "disk_dolu"]
    s = {t: (metin, onem) for t, metin, onem in S.sorunlar(M, C, kotu, T0 - 60, T0 - 60, BUGUN, T0)}
    assert s["servis_durdu"] == ("Servis durdu: portal", "kritik") and s["kayit_durdu"] == ("5651 kaydı durdu", "kritik")
    assert turler(r=rapor(kayit_boslugu_1s=True)) == ["kayit_durdu"]
    assert turler(r=rapor(panel=False, dnsmasq=False, guvenlik=False)) == ["servis_durdu"]
    s = S.sorunlar(M, C, rapor(panel=False, dnsmasq=False), T0 - 60, T0 - 60, BUGUN, T0)
    assert s == [("servis_durdu", "Servis durdu: panel, dnsmasq", "kritik")]
    assert S.sorunlar(M, C, rapor(), T0 - 60, T0 - 700, BUGUN, T0) == [("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik")]


def test_kural_esikleri():
    assert turler(r=rapor(disk_yuzde=89)) == []
    assert S.sorunlar(M, C, rapor(disk_yuzde=90), T0 - 60, T0 - 60, BUGUN, T0) == [("disk_dolu", "Disk dolu (%90)", "uyari")]
    assert turler(m={**M, "lisans_bitis": "2026-10-27"}) == []
    assert S.sorunlar({**M, "lisans_bitis": "2026-10-26"}, C, rapor(), T0 - 60, T0 - 60, BUGUN, T0) == \
        [("lisans_bitiyor", "Lisans bitiyor (26.10.2026)", "bilgi")]
    assert turler(rz=T0 - 899) == [] and turler(rz=T0 - 901) == ["rapor_yok"]
    assert turler(el=T0 - 600) == [] and turler(el=T0 - 601) == ["ulasilamiyor"]
    assert turler(sg="2026-10-09") == [] and turler(sg="2026-10-08") == ["yedek_gecikti"] and turler(sg=None) == ["yedek_gecikti"]
    assert S.sorunlar(M, C, rapor(sms_sorunu=True), T0 - 60, T0 - 60, BUGUN, T0) == [("sms_calismiyor", "SMS çalışmıyor", "uyari")]
    # lisansı bitmiş: yalnızca lisans bilgisi
    s = S.sorunlar({**M, "lisans_bitis": "2026-10-10"}, C, rapor(disk_yuzde=99), T0 - 5000, T0 - 5000, None, T0)
    assert s == [("lisans_bitti", "Lisans bitti (10.10.2026)", "bilgi")]


def test_kural_merkezdeki_son_dogrulama_sms_sonucu():
    """Merkezin son doğrulama SMS'i (sms_son) son 60 dk içinde misafirden kaynaklanmayan bir hatayla bittiyse SMS çalışmıyor."""
    def son(sonuc="hata", kod="30", once=60):
        return {"zaman": common.now_iso(T0 - once), "saglayici": "netgsm", "sonuc": sonuc, "kod": kod}

    def t(sms_son):
        return [x for x, _, _ in S.sorunlar(M, C, rapor(), T0 - 60, T0 - 60, BUGUN, T0, sms_son=sms_son)]
    assert t(None) == []
    for kod in ("30", "AG", "AYAR", "PARSE", "SINIR", "100", "20003"):
        assert t(son(kod=kod)) == ["sms_calismiyor"], kod
    for kod in ("YABANCI", "50", "60200", "21211", "60205", "60203", "80"):  # misafirden kaynaklanan
        assert t(son(kod=kod)) == [], kod
    assert t(son(sonuc="gonderildi", kod="0")) == []  # son gönderim başarılı: kapanır
    assert t(son(once=3601)) == []  # 60 dk'dan eski sonuç
    assert t(son(once=3599)) == ["sms_calismiyor"]
    # rapordaki sms_sorunu ile birlikte tek sorun
    s = S.sorunlar(M, C, rapor(sms_sorunu=True), T0 - 60, T0 - 60, BUGUN, T0, sms_son=son())
    assert s == [("sms_calismiyor", "SMS çalışmıyor", "uyari")]
    # ulaşılamıyorken de (merkezin kendi bilgisi)
    assert "sms_calismiyor" in [x for x, _, _ in S.sorunlar(M, C, rapor(), T0 - 60, T0 - 700, BUGUN, T0, sms_son=son())]


def test_merkez_sms_hatasi_sorun_acar_ve_kapatir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri()
        o.tur()
        assert o.acik(n) == {}
        o.v.sms_son_kaydet(n, "netgsm", "hata", "30")
        o.tur(60)
        assert list(o.acik(n)) == ["sms_calismiyor"]
        o.v.sms_son_kaydet(n, "netgsm", "gonderildi", "0")
        o.tur(60)
        assert o.acik(n) == {}


def test_hic_rapor_yoksa_rapor_yok_acilmaz():
    assert turler(r=None, rz=None) == []
    assert turler(r=None, rz=None, el=T0 - 700) == ["ulasilamiyor"]


def test_serbest_askida_kuralsiz():
    assert turler(m={**M, "askida": 1}, el=None, sg=None) == []
    assert turler(c={**C, "durum": "serbest_birakiliyor"}, el=None, sg=None) == []
    assert turler(c=None, el=None, sg=None) == []
    assert turler(m={**M, "uyelik": "bitti"}, el=None, sg=None) == []


def test_kural_guncelleme_basarisiz():
    def t(r, hedef="1.2.0"):
        return [x for x, _, _ in S.sorunlar(M, C, r, T0 - 60, T0 - 60, BUGUN, T0, hedef=hedef)]
    g = {"surum": "1.2.0", "sonuc": "geri_alindi", "zaman": 1}
    assert t(rapor(paket="1.1.0", guncelleme=g)) == ["guncelleme_basarisiz"]
    assert t(rapor(paket="1.1.0", guncelleme=dict(g, sonuc="hata"))) == ["guncelleme_basarisiz"]
    s = S.sorunlar(M, C, rapor(paket="1.1.0", guncelleme=g), T0 - 60, T0 - 60, BUGUN, T0, hedef="1.2.0")
    assert s == [("guncelleme_basarisiz", "Güncelleme başarısız (1.2.0)", "uyari")]
    assert t(rapor(paket="1.2.0", guncelleme=g)) == []  # paket hedefe eşit
    assert t(rapor(paket="1.1.0", guncelleme=dict(g, sonuc="tamam"))) == []
    assert t(rapor(paket="1.1.0", guncelleme=dict(g, surum="1.1.5"))) == []  # başka sürümün denemesi
    assert t(rapor(paket="1.1.0", guncelleme=g), hedef=None) == []
    assert t(rapor(paket="1.1.0")) == [] and t(rapor(paket="1.1.0", guncelleme=None)) == []


class Sahte:
    """Sahte NetGSM: hiçbir ağ isteği yok."""
    def __init__(self, sonuc="gonderildi"):
        self.giden, self.sonuc = [], sonuc

    def __call__(self, ayar, metin, numaralar):
        assert ayar["usercode"] == "u" and numaralar == ["905334553132"]
        self.giden.append(metin)
        return self.sonuc


class Ortam:
    def __init__(self, tmp, acik=True, sinir=20, telefon=True, t=T0):
        self.yol = os.path.join(tmp, "m.db")
        self.t = [t]
        self.v = V.Veri(self.yol, saat=lambda: self.t[0])
        self.v.uyari_ayari_koy({"telefonlar": ["0533 455 31 32"] if telefon else [], "acik": acik, "gunluk_sinir": sinir})
        self.v.sms_ayari_koy({"mock": False, "usercode": "u", "password": "p", "msgheader": "H"})
        self.g = Sahte()
        self.el, self.sg, self.rd = {}, {}, {}
        self.wg_ok = True  # durum.json'daki `wg show` başarısı; None: alan yok (eski wc-durum)
        self.ld = os.path.join(tmp, "listeler")  # hazır engel listeleri meta dizini (boş: denetim sessiz)

    def musteri(self, ad="Göztepe", lisans="2027-10-11", el_yas=60):
        n, _ = self.v.musteri_ekle()
        cid, _ = self.v.cihaz_bagla(n, f"{n:043d}=", "ssh-ed25519 AAAA", ad)
        self.v.lisans_ayarla(n, lisans)
        self.el[n] = el_yas
        return n, cid

    def tur(self, ileri=0, rapor_yaz=True):
        """ileri sn sonra bir wc-saglik turu; cihazlar raporlarını yeni göndermiş olur."""
        self.t[0] += ileri
        simdi = self.t[0]
        ars = {}
        for n, yas in self.el.items():
            c = self.v.bagli_cihaz(n)
            if rapor_yaz and c is not None:
                self.v.saglik_kaydet(c["id"], rapor(**self.rd.get(n, {})))
            gun = datetime.datetime.fromtimestamp(simdi, common.TZ).date().isoformat()
            if yas == "yok":  # durum.json'da kaydı yok (arşiv henüz oluşmadı)
                continue
            ars[str(n)] = {"tur": "cihaz", "son_gun": self.sg.get(n, gun), "boyut": 0,
                           "el_sikisma": None if yas is None else simdi - yas}
        d = {"zaman": common.now_iso(simdi), "arsivler": ars}
        if self.wg_ok is not None:
            d["wg_ok"] = self.wg_ok
        S.calistir(self.v, d, gonder=self.g, simdi=simdi, liste_dizini=self.ld)

    def acik(self, n):
        return {s["tur"]: s for s in self.v.acik_sorunlar(n)}


def test_sms_10_dk_sonra_ve_duzeldi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas=700)
        o.tur()
        assert list(o.acik(n)) == ["ulasilamiyor"] and o.g.giden == []
        o.tur(300)
        assert o.g.giden == []
        o.tur(299)
        assert o.g.giden == []
        o.tur(1)  # 600 sn açık kaldı
        assert o.g.giden == [f"WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (20:15'ten beri)"]
        assert o.acik(n)["ulasilamiyor"]["acilis_sms"] == "gonderildi"
        o.tur(300)
        assert len(o.g.giden) == 1
        o.el[n] = 60
        o.tur(300)  # 1200 sn sonra düzeldi; "düzeldi" 600 sn temiz kalınca
        assert o.acik(n) == {} and len(o.g.giden) == 1
        o.tur(300)
        assert len(o.g.giden) == 1
        o.tur(300)
        assert o.g.giden[1] == f"WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor düzeldi (20 dk sürdü)"
        assert o.v.son_kapanan(n, "ulasilamiyor")["kapanis_sms"] == "gonderildi"
        h = [r for r in o.v.hareketler(n) if r["olay"] == "UYARI_SMS"]
        assert len(h) == 2 and all(r["kim"] == "saglik" and r["ip"] == "-" for r in h)
        o.tur(300)
        assert len(o.g.giden) == 2 and o.v.sms_sayaci(BUGUN) == 2


def test_titreyen_baglanti_tek_sms():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri()
        # bir saat 9–11 dk arası gidip gelen el sıkışma: her kopukluk 600 sn temiz kalmadan döner → bildirilmemiş olsa da
        # aynı olay sürer (aynı satır, ilk açılış), 600 sn'yi bulunca tek SMS
        for yas in (660, 540) * 6:
            o.el[n] = yas
            o.tur(300)
        assert o.g.giden == [f"WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (20:20'ten beri)"]
        assert len(o.v.sorun_gecmisi(n)) == 1
        o.el[n] = 60
        o.tur(300)
        assert len(o.g.giden) == 1
        o.tur(300)  # son kapanıştan (T0+3600) 600 sn temiz: tek "düzeldi", toplam süreyle
        assert o.g.giden[1] == f"WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor düzeldi (55 dk sürdü)"
        o.tur(300), o.tur(300)
        assert len(o.g.giden) == 2


def test_bildirilmemis_kisa_kopma_sms_atmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas=700)
        o.tur()
        o.el[n] = 60
        o.tur(300)
        for _ in range(4):
            o.tur(300)  # tek seferlik 300 sn'lik kopukluk: SMS yok
        o.el[n] = 700
        o.tur(300), o.tur(300)  # 600 sn'den sonra yeniden: yeni olay, 600 sn dolmadı
        assert o.g.giden == [] and len(o.v.sorun_gecmisi(n)) == 2


def test_kisa_kapanip_acilma_tek_olay():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        assert len(o.g.giden) == 1
        ilk = o.acik(n)["ulasilamiyor"]
        o.el[n] = 60
        o.tur(300)  # 300 sn temiz
        o.el[n] = 700
        o.tur(300), o.tur(300), o.tur(300)
        assert len(o.g.giden) == 1  # düzeldi yok, yeni açılış yok
        s = o.acik(n)["ulasilamiyor"]
        assert s["id"] == ilk["id"] and s["acilis"] == ilk["acilis"] and len(o.v.sorun_gecmisi(n)) == 1


def test_duzeldi_sonrasi_uzun_kesinti_sms_atar():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        o.el[n] = 60
        o.tur(300), o.tur(300), o.tur(300)  # 600 sn temiz → düzeldi
        assert len(o.g.giden) == 2 and "düzeldi" in o.g.giden[1]
        o.el[n] = 700
        o.tur(300)  # yeniden düştü (artık yeni olay)
        assert len(o.g.giden) == 2
        o.tur(300), o.tur(300)  # 600 sn sürdü → yeni SMS
        assert len(o.g.giden) == 3 and o.g.giden[2].startswith(f"WifiCorrect {n} Göztepe: Cihaz ulaşılamıyor (")
        for _ in range(12):
            o.tur(300)  # saatlerce sürse de tek SMS
        assert len(o.g.giden) == 3


def test_yeniden_calistirmada_cift_sms_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        assert len(o.g.giden) == 1
        o.tur(0)  # aynı anda ikinci tur (timer kaçtı/çift tetiklendi)
        o.v = V.Veri(o.yol, saat=lambda: o.t[0])  # merkez yeniden başladı
        o.tur(0), o.tur(300)
        assert len(o.g.giden) == 1 and len(o.v.acik_sorunlar(n)) == 1


def test_gunluk_sinir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp, sinir=3)
        ns = sorted(o.musteri(ad=f"K{i}", el_yas=700)[0] for i in range(5))  # müşteri sırası numaraya göre
        o.musteri(ad="Sağlam")  # hepsi birden düşmüş olmasın (o, izleme arızası sayılır)
        o.tur(), o.tur(600)
        assert len(o.g.giden) == 3 and o.v.sms_sayaci(BUGUN) == 3
        assert o.g.giden[2] == "WifiCorrect: günlük SMS üst sınırına (3) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."
        durumlar = [o.acik(n)["ulasilamiyor"]["acilis_sms"] for n in ns]
        assert durumlar == ["gonderildi", "gonderildi", "sinir:2026-10-11", "sinir:2026-10-11", "sinir:2026-10-11"]
        for n in ns:
            o.el[n] = 60
        o.tur(300), o.tur(300), o.tur(300)  # düzeldi: ilk ikisinin kapanışı sınıra takılır, üst sınır SMS'i ikinci kez gitmez
        assert len(o.g.giden) == 3
        # sınıra takılmış açılışın "düzeldi"si hiç gitmez (operatör açılışı almadı)
        assert [o.v.son_kapanan(n, "ulasilamiyor")["kapanis_sms"] for n in ns] == ["sinir:2026-10-11"] * 2 + [""] * 3
        assert len([r for r in o.v.hareketler() if r["olay"] == "UYARI_SMS"]) == 3
        # ertesi gün sayaç sıfırdan
        o.el[ns[0]] = 700
        o.tur(86400), o.tur(600)
        assert len(o.g.giden) == 4 and o.v.sms_sayaci("2026-10-12") == 1


def test_bilgi_09da_toplu():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        x, _ = o.musteri(ad="A Kafe", lisans="2026-10-20")
        y, _ = o.musteri(ad="B Kafe", lisans="2026-10-21")
        (a, ad_a, t_a), (b, ad_b, t_b) = sorted([(x, "A Kafe", "20.10.2026"), (y, "B Kafe", "21.10.2026")])
        o.tur()
        assert {s["tur"] for s in o.v.acik_sorunlar()} == {"lisans_bitiyor"} and len(o.v.acik_sorunlar()) == 2
        o.tur(3600)
        assert o.g.giden == []  # bilgi 600 sn sonra gitmez
        o.t[0] = ts("2026-10-12T08:55:00")
        o.tur()
        assert o.g.giden == []
        o.tur(300)  # 09:00 turu
        assert len(o.g.giden) == 1
        assert o.g.giden[0] == (f"WifiCorrect bilgi: {a} {ad_a}: Lisans bitiyor ({t_a}); "
                                f"{b} {ad_b}: Lisans bitiyor ({t_b})")
        assert all(s["acilis_sms"] == "gonderildi" for s in o.v.acik_sorunlar())
        assert len([r for r in o.v.hareketler() if r["olay"] == "UYARI_SMS"]) == 2  # müşteri başına kayıt
        o.tur(300), o.tur(86400)
        assert len(o.g.giden) == 1  # aynı sorun için tekrar yok
        o.v.lisans_ayarla(a, "2027-10-20")
        o.tur(300)
        assert len(o.g.giden) == 1 and [s["musteri"] for s in o.v.acik_sorunlar()] == [b]  # bilgi kapanışı SMS'siz
        # b'nin lisansı bitti: "bitiyor" sessizce kapanır, "bitti" ayrı sorun olarak ertesi 09:00 özetine girer
        ertesi = datetime.datetime.strptime(t_b, "%d.%m.%Y").date() + datetime.timedelta(days=1)
        o.t[0] = ts(f"{ertesi.isoformat()}T09:00:00")
        o.tur()
        assert [(s["tur"], s["metin"]) for s in o.v.acik_sorunlar(b)] == [("lisans_bitti", f"Lisans bitti ({t_b})")]
        assert len(o.g.giden) == 2 and o.g.giden[1] == f"WifiCorrect bilgi: {b} {ad_b}: Lisans bitti ({t_b})"


def test_sms_hatasi_kaydedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        o.g.sonuc = "hata:30"
        n, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        assert len(o.g.giden) == 1 and o.acik(n)["ulasilamiyor"]["acilis_sms"] == "hata:30"
        o.tur(300)
        assert len(o.g.giden) == 1  # yeniden deneme yok
        h = [r for r in o.v.hareketler(n) if r["olay"] == "UYARI_SMS"]
        assert len(h) == 1 and "hata:30" in h[0]["ayrinti"]
        o.el[n] = 60
        o.tur(300)
        assert len(o.g.giden) == 1 and o.v.son_kapanan(n, "ulasilamiyor")["kapanis_sms"] == ""


def test_serbest_askida_uyarisiz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        a, _ = o.musteri(el_yas=700)
        b, _ = o.musteri(el_yas=700)
        c, _ = o.musteri(el_yas=700)
        o.v.askiya_al(b, True)
        o.v.serbest_birak(c)
        o.tur(), o.tur(600)
        assert len(o.g.giden) == 1 and o.v.acik_sorunlar(b) == [] and o.v.acik_sorunlar(c) == []
        o.v.askiya_al(a, True)  # bildirilmiş sorun askıya alınınca sessizce kapanır
        o.tur(300)
        assert o.v.acik_sorunlar(a) == [] and len(o.g.giden) == 1


def test_uyari_kapaliyken_sms_yok_kayit_var():
    for kw in ({"acik": False}, {"telefon": False}):
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            o = Ortam(tmp, **kw)
            n, _ = o.musteri(el_yas=700)
            o.tur(), o.tur(600), o.tur(300)
            assert o.g.giden == [] and o.acik(n)["ulasilamiyor"]["acilis_sms"] == ""
            assert o.v.sms_sayaci(BUGUN) == 0


def test_ulasilamiyorken_rapor_kritikleri_duzeldi_sayilmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, cid = o.musteri()
        o.v.saglik_kaydet(cid, rapor(kaydedici=False))
        o.tur(rapor_yaz=False), o.tur(600, rapor_yaz=False)
        assert list(o.acik(n)) == ["kayit_durdu"] and len(o.g.giden) == 1
        o.el[n] = 700
        o.tur(300, rapor_yaz=False)
        assert sorted(o.acik(n)) == ["kayit_durdu", "ulasilamiyor"] and len(o.g.giden) == 1


def test_bayat_durumda_degerlendirme_yok_gunde_bir_sms():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        o.musteri(el_yas=None)
        for d in ({}, {"zaman": "bozuk", "arsivler": {}}, {"zaman": common.now_iso(T0 - 1000), "arsivler": {}},
                  {"zaman": common.now_iso(T0 - 2000), "arsivler": {}}):
            S.calistir(o.v, d, gonder=o.g, simdi=T0)
        # neden türü başına günde bir: aynı neden ikinci kez gitmez, farklı neden ötekini yutmaz
        assert o.v.acik_sorunlar() == [] and o.g.giden == ["WifiCorrect: izleme çalışmıyor (durum.json yok)",
                                                         "WifiCorrect: izleme çalışmıyor (durum.json bayat)"]
        S.calistir(o.v, {"zaman": common.now_iso(T0), "arsivler": {}}, gonder=o.g, simdi=T0 + 86400 + 1000)
        assert o.g.giden[2:] == ["WifiCorrect: izleme çalışmıyor (durum.json bayat)"]  # ertesi gün yine bir kez
        assert len([r for r in o.v.hareketler() if r["olay"] == "UYARI_SMS"]) == 3 and o.v.sms_sayaci(BUGUN) == 2
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp, acik=False)
        S.calistir(o.v, {}, gonder=o.g, simdi=T0)
        assert o.g.giden == []


def test_el_sikisma_yasi_durum_zamanina_gore():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, cid = o.musteri()
        o.v.saglik_kaydet(cid, rapor())
        dz = T0 - 500  # durum.json 500 sn önce yazılmış, el sıkışma ondan 120 sn önce
        d = {"zaman": common.now_iso(dz), "arsivler": {str(n): {"son_gun": BUGUN, "el_sikisma": dz - 120}}}
        S.calistir(o.v, d, gonder=o.g, simdi=T0)
        assert o.v.acik_sorunlar(n) == []
        d["arsivler"][str(n)]["el_sikisma"] = dz - 601
        S.calistir(o.v, d, gonder=o.g, simdi=T0)
        assert list(o.acik(n)) == ["ulasilamiyor"]


def test_tunel_bilgisi_okunamadi_toplu_ulasilamiyor_acilmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        o.wg_ok = None  # eski wc-durum: wg_ok yok, izlenen hepsinin el sıkışması yok → okunamadı sayılır
        a, _ = o.musteri(el_yas=None)
        b, _ = o.musteri(el_yas=None)
        o.tur(), o.tur(600), o.tur(300)
        assert o.v.acik_sorunlar() == []
        assert o.g.giden == ["WifiCorrect: izleme çalışmıyor (tünel bilgisi okunamadı)"]
        o.el[b] = 60  # biri ayakta: ötekinin düşmesi gerçek
        o.tur(300)
        assert list(o.acik(a)) == ["ulasilamiyor"] and o.v.acik_sorunlar(b) == []
        o.el[a] = o.el[b] = None  # aynı gün durum.json bayatlarsa o ayrı bir neden: yine bildirilir
        o.tur(300), o.tur(300)
        S.calistir(o.v, {"zaman": common.now_iso(o.t[0] - 1000), "arsivler": {}}, gonder=o.g, simdi=o.t[0])
        assert o.g.giden[1:] == ["WifiCorrect: izleme çalışmıyor (durum.json bayat)"]


def test_tunel_kaydi_olmayan_cihaz_toplu_kopmaya_sayilmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        # (a) iki yeni cihaz, ikisinin de kaydı yok: kurallar normal çalışır, tünel SMS'i yok
        o = Ortam(tmp)
        a, cid = o.musteri(el_yas="yok")
        o.musteri(el_yas="yok")
        o.v.saglik_kaydet(cid, rapor(kaydedici=False))
        o.tur(rapor_yaz=False), o.tur(600, rapor_yaz=False)
        assert list(o.acik(a)) == ["kayit_durdu"] and len(o.g.giden) == 1 and "5651 kaydı durdu" in o.g.giden[0]
        assert not any("izleme" in g for g in o.g.giden)
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        # (b) biri yeni (kayıt yok), öteki gerçekten düşmüş: düşen için ulaşılamıyor açılır
        o = Ortam(tmp)
        o.musteri(el_yas="yok")
        b, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        assert list(o.acik(b)) == ["ulasilamiyor"] and o.g.giden == [f"WifiCorrect {b} Göztepe: Cihaz ulaşılamıyor (20:15'ten beri)"]


def test_arsiv_kaydi_yoksa_ulasilamiyor_ve_yedek_acilmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri(el_yas="yok")
        o.tur(), o.tur(600), o.tur(86400)
        assert o.v.acik_sorunlar(n) == [] and o.g.giden == []
        o.tur(2 * 86400)  # bağlanalı 3 gün: yedek gecikti
        assert list(o.acik(n)) == ["yedek_gecikti"]
    c = {**C, "baglanma": "2026-10-09T10:00:00+03:00"}
    assert S.sorunlar(M, c, rapor(), T0 - 60, None, None, T0, arsiv_var=False) == []
    c = {**C, "baglanma": "2026-10-08T23:00:00+03:00"}
    assert S.sorunlar(M, c, rapor(), T0 - 60, None, None, T0, arsiv_var=False) == [("yedek_gecikti", "Yedek gecikti", "uyari")]
    assert [t for t, _, _ in S.sorunlar(M, c, rapor(kaydedici=False), T0 - 5000, None, None, T0, arsiv_var=False)] == \
        ["kayit_durdu", "yedek_gecikti"]  # rapor kuralları yine çalışır; tünel bilinmediğinden rapor_yok yok


def test_main_beklenmeyen_hatada_sms():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        eski = S.calistir

        def bozuk(*a, **k):
            raise RuntimeError("deneme")
        S.calistir = bozuk
        try:
            assert S.main(db=o.yol, durum_yolu=os.path.join(tmp, "yok.json"), gonder=o.g) == 1
        finally:
            S.calistir = eski
        assert o.g.giden == ["WifiCorrect: izleme çalışmıyor (hata: RuntimeError)"]

        def patlayan(*a):
            raise OSError("ağ")
        S.calistir = bozuk
        try:  # SMS denemesi de patlasa main düşmez
            assert S.main(db=os.path.join(tmp, "b.db"), durum_yolu=os.path.join(tmp, "yok.json"), gonder=patlayan) == 1
        finally:
            S.calistir = eski



def test_sinira_takilan_acilis_ertesi_gun_bir_kez_gider():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp, sinir=2)
        ad = {o.musteri(ad=a, el_yas=700)[0]: a for a in ("A", "B")}
        o.musteri(ad="Sağlam")
        x, y = sorted(ad)
        o.tur(), o.tur(600)
        assert o.g.giden == [f"WifiCorrect {x} {ad[x]}: Cihaz ulaşılamıyor (20:15'ten beri)",
                             "WifiCorrect: günlük SMS üst sınırına (2) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."]
        assert o.acik(y)["ulasilamiyor"]["acilis_sms"] == "sinir:2026-10-11"
        o.tur(300)
        assert len(o.g.giden) == 2  # aynı gün yeniden denenmez
        o.t[0] = ts("2026-10-12T00:05:00")
        o.tur()  # ertesi gün y hâlâ açık: tam bir SMS
        assert o.g.giden[2:] == [f"WifiCorrect {y} {ad[y]}: Cihaz ulaşılamıyor (20:15'ten beri)"]
        o.tur(300), o.tur(300)
        assert len(o.g.giden) == 3 and o.acik(y)["ulasilamiyor"]["acilis_sms"] == "gonderildi"


def test_sinira_takilan_acilisin_duzeldisi_gitmez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp, sinir=2)
        ad = {o.musteri(ad=a, el_yas=700)[0]: a for a in ("A", "B")}
        o.musteri(ad="Sağlam")
        x, y = sorted(ad)
        o.tur(), o.tur(600)
        o.el[y] = 60
        o.tur(300), o.tur(300), o.tur(300)
        o.t[0] = ts("2026-10-12T00:05:00")
        o.tur(), o.tur(300), o.tur(300)
        assert len(o.g.giden) == 2 and not any(f"{y} " in g for g in o.g.giden)
        assert o.v.son_kapanan(y, "ulasilamiyor")["kapanis_sms"] == ""


def test_ust_sinir_bildirimi_deneme_sayaci_doldurdugunda_da_gider():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp, sinir=2)
        o.v.sms_say(BUGUN), o.v.sms_say(BUGUN)  # deneme SMS'leri sayacı doldurdu
        n, _ = o.musteri(el_yas=700)
        o.musteri(ad="Sağlam")
        o.tur(), o.tur(600)  # ilk bastırılan uyarıda üst sınır bildirimi bir kez
        assert o.g.giden == ["WifiCorrect: günlük SMS üst sınırına (2) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."]
        assert o.acik(n)["ulasilamiyor"]["acilis_sms"] == "sinir:2026-10-11"
        o.el[n] = 60
        o.tur(300), o.tur(300), o.tur(300)
        assert len(o.g.giden) == 1


def test_son_tur_zamani_yazilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        assert o.v.saglik_son_tur() is None
        o.musteri()
        o.tur()
        assert o.v.saglik_son_tur() == common.now_iso(T0)
        S.calistir(o.v, {}, gonder=o.g, simdi=T0 + 300)  # değerlendirme yapılamadı: tamamlanmış tur sayılmaz
        assert o.v.saglik_son_tur() == common.now_iso(T0)
        o.tur(600)
        assert o.v.saglik_son_tur() == common.now_iso(T0 + 600)


def test_wg_ok_kucuk_filoda_gercek_kopma():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        # `wg show` çalıştı: biri çoktan kapalı (hiç el sıkışmamış), öteki şimdi düştü → ikisi de gerçek
        o = Ortam(tmp)
        a, _ = o.musteri(el_yas=None)
        b, _ = o.musteri(el_yas=700)
        o.tur(), o.tur(600)
        assert list(o.acik(a)) == ["ulasilamiyor"] and list(o.acik(b)) == ["ulasilamiyor"]
        assert len(o.g.giden) == 2 and not any("izleme" in g for g in o.g.giden)
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        # `wg show` okunamadı: tek cihaz olsa da değerlendirme yok, izleme SMS'i
        o = Ortam(tmp)
        o.wg_ok = False
        o.musteri(el_yas=None)
        o.tur(), o.tur(600)
        assert o.v.acik_sorunlar() == [] and o.g.giden == ["WifiCorrect: izleme çalışmıyor (tünel bilgisi okunamadı)"]


def test_acik_sorun_metni_guncellenir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri()
        o.rd[n] = {"disk_yuzde": 91}
        o.tur()
        ilk = o.acik(n)["disk_dolu"]
        assert ilk["metin"] == "Disk dolu (%91)"
        o.rd[n] = {"disk_yuzde": 95}
        o.tur(300)
        s = o.acik(n)["disk_dolu"]
        assert s["id"] == ilk["id"] and s["metin"] == "Disk dolu (%95)" and o.g.giden == []  # SMS yok
        o.rd[n] = {"disk_yuzde": 97}
        o.tur(300)  # 600 sn: SMS güncel metinle
        assert o.g.giden == [f"WifiCorrect {n} Göztepe: Disk dolu (%97) (20:15'ten beri)"]


def test_bilgi_toplu_en_cok_bes_musteri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        for i in range(7):
            o.musteri(ad=f"K{i}", lisans="2026-10-20")
        o.tur()
        o.t[0] = ts("2026-10-12T09:00:00")
        o.tur()
        assert len(o.g.giden) == 1
        g = o.g.giden[0]
        assert g.startswith("WifiCorrect bilgi: ") and g.count("Lisans bitiyor") == 5 and g.endswith("; +2 diğer")
        assert all(s["acilis_sms"] == "gonderildi" for s in o.v.acik_sorunlar()) and len(o.v.acik_sorunlar()) == 7


def test_netgsm_test_ortaminda_gercek_istek_yok():
    assert os.environ.get("WC_TEST")
    ayar = {"mock": False, "usercode": "u", "password": "p", "msgheader": "H"}
    for a in (ayar, {**ayar, "mock": True}):
        try:
            N.gonder(a, "m", ["905334553132"])  # ac= yok: gerçek gönderici
            assert False
        except RuntimeError:
            pass


def _liste_meta(dizin, kategori, **m):
    os.makedirs(dizin, exist_ok=True)
    with open(os.path.join(dizin, kategori + ".json"), "w", encoding="utf-8") as f:
        json.dump({"kategori": kategori, **m}, f, ensure_ascii=False)


def test_liste_sorunu_saf():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        d = os.path.join(tmp, "l")
        assert S.liste_sorunu(d, T0) is None  # dizin yok (merkez yeni kuruldu, liste hiç üretilmedi)
        taze = common.now_iso(T0 - 3600)
        _liste_meta(d, "kumar", sha256="a" * 64, sayi=5, zaman=taze, kaynaklar=[{"ad": "HaGeZi", "sayi": 5}])
        # tek kaynağın hatası (USOM HTML döndü) kategori başarılıysa sorun değildir
        _liste_meta(d, "zararli", sha256="b" * 64, sayi=5, zaman=taze,
                    kaynaklar=[{"ad": "USOM", "sayi": 0, "hata": "liste yerine HTML sayfası döndü"}, {"ad": "HaGeZi", "sayi": 5}])
        assert S.liste_sorunu(d, T0) is None
        _liste_meta(d, "yetiskin", sha256="c" * 64, sayi=5, zaman=taze, hata="Hiçbir kaynak indirilemedi; önceki liste kullanılıyor.")
        assert S.liste_sorunu(d, T0) == "Engel listeleri güncellenemiyor: Yetişkin içerik"
        _liste_meta(d, "kumar", sha256="a" * 64, sayi=5, zaman=common.now_iso(T0 - 48 * 3600 - 1))  # 48 saatten eski
        assert S.liste_sorunu(d, T0) == "Engel listeleri güncellenemiyor: Kumar ve bahis, Yetişkin içerik"
        _liste_meta(d, "kumar", sha256="a" * 64, sayi=5, zaman=common.now_iso(T0 - 48 * 3600 + 60))
        _liste_meta(d, "yetiskin", hata="Hiçbir kaynak indirilemedi; önceki liste kullanılıyor.")  # hiç başarılı olmadı
        assert S.liste_sorunu(d, T0) == "Engel listeleri güncellenemiyor: Yetişkin içerik"
        with open(os.path.join(d, "yetiskin.json"), "w") as f:
            f.write("{bozuk")
        assert S.liste_sorunu(d, T0) is None  # okunamayan meta: liste yok sayılır (yönetim sayfası gösterir)


def test_liste_sorunu_merkez_satiri_ve_sms():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        n, _ = o.musteri()
        _liste_meta(o.ld, "kumar", sha256="a" * 64, sayi=5, zaman=common.now_iso(T0 - 3600), hata="Hiçbir kaynak indirilemedi.")
        o.tur()
        a = {s["tur"]: s for s in o.v.acik_sorunlar(S.MERKEZ)}
        assert list(a) == ["liste_guncellenemiyor"] and a["liste_guncellenemiyor"]["onem"] == "uyari"
        assert o.acik(n) == {} and o.g.giden == []
        o.tur(600)
        assert o.g.giden == ["WifiCorrect merkez: Engel listeleri güncellenemiyor: Kumar ve bahis (20:15'ten beri)"]
        for _ in range(5):
            o.tur(3600)
        assert len(o.g.giden) == 1  # açık kaldıkça yeni SMS yok
        _liste_meta(o.ld, "kumar", sha256="a" * 64, sayi=5, zaman=common.now_iso(o.t[0]))
        o.tur(300)
        assert o.v.acik_sorunlar(S.MERKEZ) == [] and len(o.g.giden) == 1
        o.tur(600)
        assert len(o.g.giden) == 2 and o.g.giden[1].startswith("WifiCorrect merkez: Engel listeleri güncellenemiyor: Kumar ve bahis düzeldi")


def test_liste_denetimi_durum_bozukken_de_calisir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        o = Ortam(tmp)
        _liste_meta(o.ld, "kumar", hata="Hiçbir kaynak indirilemedi.")
        S.calistir(o.v, {}, gonder=o.g, simdi=o.t[0], liste_dizini=o.ld)  # durum.json yok
        assert [s["tur"] for s in o.v.acik_sorunlar(S.MERKEZ)] == ["liste_guncellenemiyor"]


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
