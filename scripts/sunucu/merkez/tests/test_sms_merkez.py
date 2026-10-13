"""Doğrulama SMS'i merkezden: /api/sms, /api/sms-dogrula, NetGSM OTP, Twilio Verify, SMS ayarları (sahte ağ; gerçek istek yok)."""
import base64
import contextlib
import io
import json
import os
import sys
import tempfile
import urllib.error
import urllib.parse

os.environ["WC_TEST"] = "1"  # gerçek NetGSM/Twilio isteği hata fırlatır

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import api as A  # noqa: E402
import netgsm as N  # noqa: E402
import twilio as T  # noqa: E402
import yonetici  # noqa: E402
import yonetim as YN  # noqa: E402

WG, SSH = "A" * 43 + "=", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI c"
NETGSM = {"mock": False, "usercode": "8500000000", "password": "netgsm-gizli", "msgheader": "BASLIK", "appkey": ""}
TW = {"acik": True, "account_sid": "AC" + "a" * 32, "auth_token": "tw-token-gizli", "api_key_sid": "",
      "api_key_secret": "", "verify_sid": "VA" + "b" * 32}
KOD = "482913"
TEL = "905334553132"


class Yanit:
    def __init__(self, govde, durum=200):
        self.b, self.status = govde, durum

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def read(self, n=-1):
        return self.b


class Ag:
    """Sahte urlopen: her isteği kaydeder, sıradaki cevabı (bytes, (durum, bytes) ya da istisna) döner."""

    def __init__(self, *cevaplar):
        self.cevaplar, self.istekler = list(cevaplar), []

    def __call__(self, req, timeout):
        self.istekler.append((req, timeout))
        c = self.cevaplar.pop(0) if self.cevaplar else b"<xml><main><code>0</code><jobID>111</jobID></main></xml>"
        if isinstance(c, Exception):
            raise c
        if isinstance(c, tuple):
            if c[0] >= 400:
                raise urllib.error.HTTPError(req.full_url, c[0], "hata", {}, io.BytesIO(c[1]))
            return Yanit(c[1], c[0])
        return Yanit(c)


def ag_yok(*a, **k):
    raise AssertionError("ağ isteği gitmemeliydi")


def kur(tmp, ag=None, sms=NETGSM, twilio=None, tr="netgsm"):
    v, _, saat = Y.ortam(tmp)
    if sms is not None:
        v.sms_ayari_koy(dict(sms, tr_saglayici=tr, twilio=twilio or {}))
    n, _ = v.musteri_ekle()
    cid, anahtar = v.cihaz_bagla(n, WG, SSH)
    api = A.Api(v, saat, sms_ac=ag if ag is not None else ag_yok)
    return api, v, n, cid, anahtar


def post(api, yol, govde, ip="198.51.100.7"):
    d, _, g = api.istek("POST", yol, {}, json.dumps(govde), "", ip)
    return d, json.loads(g)


def sms(api, anahtar, telefon=TEL, kod=KOD, ip="198.51.100.7"):
    return post(api, "/api/sms", {"cihaz_anahtari": anahtar, "telefon": telefon, "kod": kod}, ip)


def dogrula(api, anahtar, telefon=TEL, kod=KOD, ip="198.51.100.7"):
    return post(api, "/api/sms-dogrula", {"cihaz_anahtari": anahtar, "telefon": telefon, "kod": kod}, ip)


# --- NetGSM OTP gönderici ---
def test_netgsm_otp_xml_ve_cevap():
    ag = Ag(b"<?xml version='1.0'?><xml><main><code>0</code><jobID>17283946</jobID></main></xml>")
    metin = "WiFi dogrulama kodunuz: 123456. Kod 3 dakika gecerlidir."
    ayar = dict(NETGSM, password="p<&>", appkey="k1")
    assert N.otp_gonder(ayar, "5334553132", metin, ac=ag) == ("gonderildi", "0", "17283946")
    req, zaman_asimi = ag.istekler[-1]
    assert req.full_url == "https://api.netgsm.com.tr/sms/send/otp" and req.get_method() == "POST" and zaman_asimi == 10
    assert req.get_header("Content-type") == "application/xml; charset=UTF-8"
    g = req.data.decode()
    assert g == ("<?xml version='1.0' encoding='utf-8'?>\n<mainbody><header><usercode>8500000000</usercode>"
                 "<password>p&lt;&amp;&gt;</password><msgheader>BASLIK</msgheader><appkey>k1</appkey></header>"
                 f"<body><msg>{metin}</msg><no>5334553132</no></body></mainbody>")
    assert "<appkey>" not in N.otp_xml("u", "p", "h", "", "m", "5")
    assert N.otp_gonder(NETGSM, "5334553132", metin, ac=Ag(b"00 998877")) == ("gonderildi", "00", "998877")
    assert N.otp_gonder(NETGSM, "5334553132", metin, ac=Ag(b"<xml><main><code>30</code></main></xml>")) == ("hata", "30", "")
    assert N.otp_gonder(NETGSM, "5334553132", metin, ac=Ag(b"<html>bozuk")) == ("hata", "PARSE", "")
    assert N.otp_gonder(NETGSM, "5334553132", metin, ac=Ag(OSError("yok"))) == ("hata", "AG", "")
    assert N.otp_gonder(NETGSM, "5334553132", metin, ac=Ag(TimeoutError())) == ("hata", "AG", "")
    assert N.otp_gonder(dict(NETGSM, mock=True), "5334553132", metin, ac=ag_yok) == ("mock", "0", "")
    assert N.otp_gonder(dict(NETGSM, password=""), "5334553132", metin, ac=ag_yok) == ("hata", "AYAR", "")
    assert N.otp_gonder(None, "5334553132", metin, ac=ag_yok) == ("hata", "AYAR", "")
    try:
        N.otp_gonder(NETGSM, "5334553132", metin)  # ac= yok: WC_TEST gerçek isteği engeller
        raise AssertionError("WC_TEST engellemedi")
    except RuntimeError:
        pass


# --- Twilio Verify ---
def test_twilio_gonder_ve_dogrula():
    ag = Ag((201, b'{"status": "pending", "sid": "VE123"}'))
    assert T.gonder(TW, TEL, ac=ag) == ("gonderildi", "0")
    req, zaman_asimi = ag.istekler[-1]
    assert req.full_url == f"https://verify.twilio.com/v2/Services/{TW['verify_sid']}/Verifications" and zaman_asimi == 10
    assert urllib.parse.parse_qs(req.data.decode()) == {"To": ["+" + TEL], "Channel": ["sms"], "Locale": ["tr"]}
    kimlik = base64.b64encode(f"{TW['account_sid']}:{TW['auth_token']}".encode()).decode()
    assert req.get_header("Authorization") == "Basic " + kimlik
    assert req.get_header("Content-type") == "application/x-www-form-urlencoded"
    ag = Ag((201, b'{"status": "pending"}'))
    anahtarli = dict(TW, api_key_sid="SK" + "c" * 32, api_key_secret="sk-gizli")
    assert T.gonder(anahtarli, "4915123456789", ac=ag) == ("gonderildi", "0")
    req = ag.istekler[-1][0]
    assert urllib.parse.parse_qs(req.data.decode())["Locale"] == ["en"]
    assert req.get_header("Authorization") == "Basic " + base64.b64encode(f"SK{'c' * 32}:sk-gizli".encode()).decode()
    assert T.gonder(TW, TEL, ac=Ag((400, b'{"code": 60200, "message": "Invalid"}'))) == ("hata", "60200")
    assert T.gonder(TW, TEL, ac=Ag((503, b"bozuk"))) == ("hata", "503")
    assert T.gonder(TW, TEL, ac=Ag(OSError("yok"))) == ("hata", "AG")
    assert T.gonder(dict(TW, verify_sid="../x"), TEL, ac=ag_yok) == ("hata", "AYAR")
    assert T.gonder(dict(TW, auth_token=""), TEL, ac=ag_yok) == ("hata", "AYAR")
    # doğrulama
    ag = Ag((200, b'{"status": "approved"}'))
    assert T.dogrula(TW, TEL, KOD, ac=ag) == (True, "0")
    req = ag.istekler[-1][0]
    assert req.full_url.endswith("/VerificationCheck")
    assert urllib.parse.parse_qs(req.data.decode()) == {"To": ["+" + TEL], "Code": [KOD]}
    assert T.dogrula(TW, TEL, KOD, ac=Ag((200, b'{"status": "pending"}'))) == (False, "0")
    assert T.dogrula(TW, TEL, KOD, ac=Ag((404, b'{"code": 20404}'))) == (False, "0")  # süresi dolmuş
    assert T.dogrula(TW, TEL, KOD, ac=Ag(TimeoutError())) == (None, "AG")
    try:
        T.gonder(TW, TEL)
        raise AssertionError("WC_TEST engellemedi")
    except RuntimeError:
        pass


# --- /api/sms ---
def test_api_sms_netgsm_gonderir_ve_sayar():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        ag = Ag(b"<xml><main><code>0</code><jobID>555</jobID></main></xml>")
        api, v, n, _, anahtar = kur(tmp, ag)
        cikti = io.StringIO()
        with contextlib.redirect_stdout(cikti), contextlib.redirect_stderr(cikti):
            d, j = sms(api, anahtar)
        assert d == 200 and j == {"sonuc": "gonderildi", "kod": "0", "is": "555", "saglayici": "netgsm"}
        g = ag.istekler[-1][0].data.decode()
        assert f"<msg>WiFi dogrulama kodunuz: {KOD}. Kod 3 dakika gecerlidir.</msg>" in g and "<no>5334553132</no>" in g
        gun = v.bugun().isoformat()
        assert v.sms_sayac(n, gun) == 1
        h = v.hareketler(n)[0]
        assert h["olay"] == "SMS_OTP" and h["ayrinti"] == "tel=**32 saglayici=netgsm sonuc=gonderildi"
        assert all(KOD not in x["ayrinti"] for x in v.hareketler()) and KOD not in cikti.getvalue()
        # veritabanı dosyasında da kod yok
        with open(os.path.join(tmp, "m.db"), "rb") as f:
            assert KOD.encode() not in f.read()
        ag.cevaplar.append(b"<xml><main><code>50</code></main></xml>")
        d, j = sms(api, anahtar)
        assert d == 200 and j == {"sonuc": "hata", "kod": "50", "saglayici": "netgsm"}
        assert v.hareketler(n)[0]["ayrinti"] == "tel=**32 saglayici=netgsm sonuc=hata:50"


def test_api_sms_mock_istek_gondermez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp, sms=dict(NETGSM, mock=True))
        d, j = sms(api, anahtar)
        assert d == 200 and j["sonuc"] == "mock" and j["saglayici"] == "netgsm"
        assert v.hareketler(n)[0]["ayrinti"] == "tel=**32 saglayici=netgsm sonuc=mock"
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp, sms=dict(NETGSM, mock=True), twilio=TW, tr="twilio")
        d, j = sms(api, anahtar)
        assert d == 200 and j["sonuc"] == "mock" and j["saglayici"] == "twilio"


def test_api_sms_gecersiz_girdi_400():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp)
        for i, (tel, kod) in enumerate(((TEL, "12345"), (TEL, "1234567"), (TEL, "12a456"), (TEL, 123456), (TEL, "١٢٣٤٥٦"),
                         ("+905334553132", KOD), ("90533455313x", KOD), ("1234567", KOD), ("9" * 16, KOD),
                         (905334553132, KOD), (None, KOD))):
            ip = f"198.51.100.{i + 10}"  # IP başına dakikada 20 istek sınırına takılmasın
            assert sms(api, anahtar, tel, kod, ip)[0] == 400, (tel, kod)
            assert dogrula(api, anahtar, tel, kod, ip)[0] == 400, (tel, kod)
        assert post(api, "/api/sms", {"cihaz_anahtari": anahtar, "telefon": TEL, "kod": KOD, "metin": "x"})[0] == 200
        assert v.sms_sayac(n, v.bugun().isoformat()) == 1  # serbest metin alanı yok sayıldı, şablon kullanıldı


def test_api_sms_yetkisiz_cihaz_reddi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp)
        assert sms(api, "yanlis-anahtar")[0] == 401
        v.askiya_al(n, True)
        assert sms(api, anahtar)[0] == 403 and dogrula(api, anahtar)[0] == 403
        v.askiya_al(n, False)
        v.lisans_ayarla(n, "2026-01-01")
        assert sms(api, anahtar)[0] == 403
        v.lisans_uzat(n)
        v.uyelik(n, "bitti")
        assert sms(api, anahtar)[0] == 403
        v.uyelik(n, "aktif")
        v.serbest_birak(n)
        assert sms(api, anahtar)[0] == 403 and dogrula(api, anahtar)[0] == 403
        assert v.sms_sayac(n, v.bugun().isoformat()) == 0


def test_api_sms_yabanci_numara_twilio_kapali():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp)
        d, j = sms(api, anahtar, "4915123456789")
        assert d == 200 and j["sonuc"] == "hata" and j["kod"] == "YABANCI"
        d, j = sms(api, anahtar, "902161234567")  # 90 ama cep değil: Türk cep numarası sayılmaz
        assert j["kod"] == "YABANCI"
        assert v.sms_sayac(n, v.bugun().isoformat()) == 0


def test_api_sms_yabanci_numara_twilio_acik():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        ag = Ag((201, b'{"status": "pending", "sid": "VE1"}'))
        api, v, n, _, anahtar = kur(tmp, ag, twilio=TW)
        d, j = sms(api, anahtar, "4915123456789")
        assert d == 200 and j == {"sonuc": "gonderildi", "kod": "0", "saglayici": "twilio"}
        req = ag.istekler[-1][0]
        assert "verify.twilio.com" in req.full_url and KOD not in req.data.decode()  # kodu Twilio üretir
        assert v.hareketler(n)[0]["ayrinti"] == "tel=**89 saglayici=twilio sonuc=gonderildi"
        ag.cevaplar.append(b"<xml><main><code>0</code><jobID>9</jobID></main></xml>")
        d, j = sms(api, anahtar)  # Türk numarası hâlâ NetGSM'den
        assert j["saglayici"] == "netgsm" and "netgsm" in ag.istekler[-1][0].full_url


def test_api_sms_tr_saglayici_twilio():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        ag = Ag((201, b'{"status": "pending"}'))
        api, v, n, _, anahtar = kur(tmp, ag, twilio=TW, tr="twilio")
        d, j = sms(api, anahtar)
        assert j == {"sonuc": "gonderildi", "kod": "0", "saglayici": "twilio"}
        assert "verify.twilio.com" in ag.istekler[-1][0].full_url and len(ag.istekler) == 1
        ag.cevaplar.append((429, b'{"code": 60203}'))
        assert sms(api, anahtar)[1] == {"sonuc": "hata", "kod": "60203", "saglayici": "twilio"}


def test_api_sms_ayar_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp, sms=None)
        d, j = sms(api, anahtar)
        assert d == 200 and j["sonuc"] == "hata" and j["kod"] == "AYAR"


def test_api_sms_gunluk_sinir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        ag = Ag()
        api, v, n, _, anahtar = kur(tmp, ag)
        gun = v.bugun().isoformat()
        for _ in range(299):
            v.sms_sayac_artir(n, gun)
        assert sms(api, anahtar)[1]["sonuc"] == "gonderildi" and len(ag.istekler) == 1
        assert v.sms_sayac(n, gun) == 300
        d, j = sms(api, anahtar)
        assert d == 200 and j["sonuc"] == "hata" and j["kod"] == "SINIR" and len(ag.istekler) == 1
        assert v.sms_sayac(n, gun) == 300
        assert v.hareketler(n)[0]["ayrinti"] == "tel=**32 saglayici=netgsm sonuc=hata:SINIR"
        n2, _ = v.musteri_ekle()  # sınır müşteri başına
        _, a2 = v.cihaz_bagla(n2, "B" * 43 + "=", SSH)
        assert sms(api, a2)[1]["sonuc"] == "gonderildi"
        api.saat.t += 86400  # ertesi gün sayaç sıfırdan
        assert sms(api, anahtar)[1]["sonuc"] == "gonderildi" and v.sms_sayac(n, gun) == 300


# --- /api/sms-dogrula ---
def test_api_sms_dogrula():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        ag = Ag((200, b'{"status": "approved"}'), (200, b'{"status": "pending"}'), (404, b'{"code": 20404}'), OSError("yok"))
        api, v, n, _, anahtar = kur(tmp, ag, twilio=TW, tr="twilio")
        cikti = io.StringIO()
        with contextlib.redirect_stdout(cikti), contextlib.redirect_stderr(cikti):
            assert dogrula(api, anahtar) == (200, {"onay": True})
            assert dogrula(api, anahtar) == (200, {"onay": False})
            assert dogrula(api, anahtar) == (200, {"onay": False})
            assert dogrula(api, anahtar) == (200, {"sonuc": "hata", "kod": "AG"})
        assert urllib.parse.parse_qs(ag.istekler[0][0].data.decode())["Code"] == [KOD]
        assert KOD not in cikti.getvalue() and all(KOD not in x["ayrinti"] for x in v.hareketler())
        assert dogrula(api, "yanlis")[0] == 401


def test_api_sms_dogrula_twilio_kapali():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp)
        assert dogrula(api, anahtar) == (200, {"sonuc": "hata", "kod": "AYAR"})


# --- eski cihazlar: eşleşmede giden NetGSM eki değişmez, Twilio sırları cihaza gitmez ---
def test_eski_cihaz_eki_degismez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, _, anahtar = kur(tmp, twilio=TW, tr="twilio")
        d, j = post(api, "/api/eslesme", {"cihaz_anahtari": anahtar})
        assert d == 200 and j["sms"] == {"mock": False, "provider": "netgsm",
                                         "netgsm": {k: NETGSM[k] for k in ("usercode", "password", "msgheader", "appkey")}}
        assert "tw-token-gizli" not in json.dumps(j)
        s = v.sms_ayari()
        assert s["tr_saglayici"] == "twilio" and s["twilio"]["auth_token"] == "tw-token-gizli"


def test_sms_ayari_dogrulama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        v.sms_ayari_koy(dict(NETGSM))
        s = v.sms_ayari()
        assert s["tr_saglayici"] == "netgsm" and s["twilio"]["acik"] is False
        for kotu in ({"tr_saglayici": "baska"}, {"tr_saglayici": "twilio"},  # Twilio kapalıyken TR'yi Twilio'ya veremez
                     {"twilio": dict(TW, verify_sid="VA../x")}, {"twilio": dict(TW, account_sid="XX1")},
                     {"twilio": dict(TW, auth_token="")}):
            try:
                v.sms_ayari_koy(dict(NETGSM, **kotu))
                raise AssertionError(kotu)
            except ValueError:
                pass
        v.sms_ayari_koy(dict(NETGSM, password="", usercode="", tr_saglayici="twilio", twilio=TW))  # NetGSM gerekmez
        v.sms_ayari_koy(dict(NETGSM, twilio=dict(TW, auth_token="", api_key_sid="SK" + "1" * 32, api_key_secret="s")))


def test_sayac():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        assert v.sms_sayac(1234567, "2026-09-21") == 0
        assert v.sms_sayac_artir(1234567, "2026-09-21") == 1 and v.sms_sayac_artir(1234567, "2026-09-21") == 2
        assert v.sms_sayac(1234567, "2026-09-22") == 0 and v.sms_sayac(7654321, "2026-09-21") == 0


# --- yönetim: SMS ayarları ve müşteri sayfası ---
PW = "yonetici-parola-1"


def yonetim_kur(tmp):
    v, k, saat = Y.ortam(tmp)
    yol, dyol = os.path.join(tmp, "yonetici.json"), os.path.join(tmp, "durum.json")
    yonetici.yaz(yol, "serkan", PW)
    with open(dyol, "w", encoding="utf-8") as f:
        json.dump({"zaman": "", "arsivler": {}}, f)
    app = YN.Yonetim(v, k, saat, yol, dyol, lambda islem, veri: "0" * 16)
    return app, v, Y.cerez(Y.giris(app, "serkan", PW))


def test_yonetim_twilio_ayarlari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, c = yonetim_kur(tmp)
        s = Y.al(app, "/sms", c)[2].decode()
        for alan in ("tr_saglayici", "twilio_acik", "account_sid", "auth_token", "api_key_sid", "api_key_secret", "verify_sid"):
            assert f'name="{alan}"' in s, alan
        t = Y.csrf(Y.al(app, "/sms", c))
        form = {"csrf": t, "usercode": "850", "password": "netgsm-gizli", "msgheader": "B", "tr_saglayici": "twilio",
                "twilio_acik": "1", "account_sid": TW["account_sid"], "auth_token": "tw-token-gizli",
                "api_key_sid": "", "api_key_secret": "", "verify_sid": TW["verify_sid"]}
        assert Y.gonder(app, "/sms", c, form)[0] == 303
        s = v.sms_ayari()
        assert s["tr_saglayici"] == "twilio" and s["twilio"]["acik"] is True and s["twilio"]["auth_token"] == "tw-token-gizli"
        sayfa = Y.al(app, "/sms", c)[2].decode()
        assert "tw-token-gizli" not in sayfa and "netgsm-gizli" not in sayfa and TW["account_sid"] in sayfa
        r = Y.gonder(app, "/sms", c, dict(form, auth_token="", password=""))  # boş sır = aynı kalır
        assert r[0] == 303 and v.sms_ayari()["twilio"]["auth_token"] == "tw-token-gizli"
        assert v.sms_ayari()["password"] == "netgsm-gizli"
        r = Y.gonder(app, "/sms", c, dict(form, api_key_secret="sk-yeni-gizli", api_key_sid="SK" + "d" * 32))
        assert r[0] == 303 and v.sms_ayari()["twilio"]["api_key_secret"] == "sk-yeni-gizli"
        assert "sk-yeni-gizli" not in Y.al(app, "/sms", c)[2].decode()
        assert all(z not in h["ayrinti"] for h in v.hareketler() for z in ("tw-token-gizli", "sk-yeni-gizli", "netgsm-gizli"))
        onceki = v.sms_ayari()
        csrfsiz = {k: x for k, x in form.items() if k != "csrf"}
        for f in (dict(csrfsiz, auth_token="baska"), dict(csrfsiz, csrf="yanlis", auth_token="baska")):
            assert Y.gonder(app, "/sms", c, f)[0] == 403
        assert v.sms_ayari() == onceki
        r = Y.gonder(app, "/sms", c, dict(form, verify_sid="kotu"))
        assert r[0] == 400 and v.sms_ayari() == onceki


def test_yonetim_musteri_sayfasi_sms_sayaci():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        app, v, c = yonetim_kur(tmp)
        n, _ = v.musteri_ekle()
        for _ in range(7):
            v.sms_sayac_artir(n, v.bugun().isoformat())
        s = Y.al(app, f"/m/{n}", c)[2].decode()
        assert "Bugün doğrulama SMS" in s and "<td>7</td>" in s


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
