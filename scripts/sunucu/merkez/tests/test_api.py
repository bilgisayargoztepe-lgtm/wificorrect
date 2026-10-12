import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import api as A  # noqa: E402

WG, WG2 = "A" * 43 + "=", "B" * 43 + "="
SSH = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI cihaz"
TAMAM = {"durum": "tamam", "tunel_ip": "10.99.0.10", "sunucu_pub": "S" * 43 + "=", "uc_nokta": "vpn.wificorrect.com:51820"}


class Kuyruk:
    def __init__(self, sonuc=TAMAM):
        self.isler, self.sonuc = [], sonuc

    def ekle(self, islem, veri):
        self.isler.append((islem, veri))
        return f"{len(self.isler):016x}"

    def bekle(self, kimlik):
        return self.sonuc


def kur(tmp, sonuc=TAMAM):
    v, _, saat = Y.ortam(tmp)
    q = Kuyruk(sonuc)
    return A.Api(v, saat, q.ekle, q.bekle), v, q, saat


def baska(n):
    """n'den farklı, geçerli biçimde ama kayıtlı olmayan numara."""
    return 1234567 if n != 1234567 else 7654321


def post(api, yol, govde, ip="198.51.100.7"):
    d, b, g = api.istek("POST", yol, {}, json.dumps(govde) if not isinstance(govde, str) else govde, "", ip)
    return d, json.loads(g)


def giris(api, n, pw, wg=WG, ip="198.51.100.7"):
    return post(api, "/api/giris", {"numara": str(n), "parola": pw, "wg_pub": wg, "ssh_pub": SSH,
                                    "isletme_adi": "Bocafe", "unvan": "Bocafe Ltd.", "surum": "1.0"}, ip)


def test_ilk_giris_baglar_ve_kuyruga_atar():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert d == 200 and j["tunel_ip"] == "10.99.0.10" and j["yedek_hedefi"] == f"wfc-{n}@10.99.0.1:"
        assert j["uc_nokta"] == "vpn.wificorrect.com:51820" and j["yineleme"] == 120000 and len(j["cihaz_anahtari"]) > 30
        m = v.musteri(n)
        assert (j["tuz"], j["ozet"]) == (m["tuz"], m["ozet"])
        assert q.isler == [("cihaz-ekle", {"numara": str(n), "wg_pub": WG, "ssh_pub": SSH})]
        c = v.bagli_cihaz(n)
        assert c["tunel_ip"] == "10.99.0.10" and c["isletme_adi"] == "Bocafe"
        assert [h["olay"] for h in v.hareketler(n)] == ["CIHAZ_BAGLANDI"]


def test_hatali_parola_kilit_ve_gecersiz_girdi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        assert giris(api, n, "yanlis-parola")[1]["kod"] == "hatali"
        assert giris(api, baska(n), "yanlis-parola")[1]["kod"] == "hatali"
        assert giris(api, 100001, "yanlis-parola")[1]["kod"] == "gecersiz"  # 6 hane
        for _ in range(4):
            giris(api, n, "yanlis-parola", ip="198.51.100.8")
        assert giris(api, n, pw)[0] == 429  # numara kilitli
        assert post(api, "/api/giris", "{bozuk")[0] == 400
        assert post(api, "/api/giris", {"numara": "abc", "parola": "x"})[1]["kod"] == "gecersiz"
        assert post(api, "/api/giris", {"numara": str(baska(n)), "parola": "x" * 10, "wg_pub": "kotu", "ssh_pub": SSH})[1]["kod"] == "hatali"
        assert api.istek("GET", "/api/giris", {}, "", "", "1.1.1.1")[0] == 404
        assert api.istek("POST", "/baska", {}, "{}", "", "1.1.1.1")[0] == 404
        assert api.istek("POST", "/api/giris", {}, "x" * 5000, "", "1.1.1.1")[0] == 413
        assert q.isler == []


def test_baska_cihaz_409_ve_ayni_cihaz_yeniden_dener():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp, sonuc=None)  # kuyruk zaman aşımı
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert d == 503 and j["kod"] == "kayit"
        q.sonuc = TAMAM
        d, j1 = giris(api, n, pw)  # aynı cihaz yeniden
        assert d == 200
        d, j2 = giris(api, n, pw)
        assert d == 200 and j2["cihaz_anahtari"] != j1["cihaz_anahtari"]
        assert post(api, "/api/eslesme", {"cihaz_anahtari": j1["cihaz_anahtari"]})[1]["kod"] == "taninmadi"
        d, j = giris(api, n, pw, wg=WG2)
        assert d == 409 and j["kod"] == "baska_cihaz"


def test_uyelik_bitti_giris_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        v.uyelik(n, "bitti")
        assert giris(api, n, pw)[1]["kod"] == "uyelik"
        v.uyelik(n, "aktif")
        assert giris(api, n, pw)[0] == 200


def test_eslesme_parola_ve_serbest_birakma():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        a = giris(api, n, pw)[1]["cihaz_anahtari"]
        d, j = post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_adi": "Bocafe Göztepe", "unvan": "<b>", "surum": "1.1"})
        assert d == 200 and j["durum"] == "bagli" and j["uyelik"] == "aktif"
        c = v.bagli_cihaz(n)
        assert c["isletme_adi"] == "Bocafe Göztepe" and c["surum"] == "1.1" and c["son_eslesme"]
        assert post(api, "/api/parola", {"cihaz_anahtari": a, "eski": "yanlis-1234", "yeni": "yeni-parola-1"})[1]["kod"] == "hatali"
        assert post(api, "/api/parola", {"cihaz_anahtari": a, "eski": pw, "yeni": "kisa"})[0] == 400
        d, j = post(api, "/api/parola", {"cihaz_anahtari": a, "eski": pw, "yeni": "yeni-parola-1"})
        assert d == 200 and v.parola_dogrula(n, "yeni-parola-1") and j["ozet"] == v.musteri(n)["ozet"]
        v.serbest_birak(n)
        assert post(api, "/api/eslesme", {"cihaz_anahtari": a})[1]["durum"] == "serbest"
        assert post(api, "/api/eslesme", {"cihaz_anahtari": a, "temizlendi": True})[1]["durum"] == "serbest"
        assert q.isler[-1] == ("cihaz-kapat", {"numara": str(n), "wg_pub": WG})
        assert post(api, "/api/eslesme", {"cihaz_anahtari": a})[1]["kod"] == "taninmadi"
        assert v.bagli_cihaz(n) is None


def test_metinler_kirpilir_ve_hiz_siniri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        a = giris(api, n, pw)[1]["cihaz_anahtari"]
        post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_adi": "x" * 500, "unvan": "a\x00b", "surum": 5})
        c = v.bagli_cihaz(n)
        assert c["isletme_adi"] == "" and c["unvan"] == "" and c["surum"] == ""
        kodlar = [post(api, "/api/eslesme", {"cihaz_anahtari": a}, ip="203.0.113.99")[0] for _ in range(21)]
        assert kodlar[-1] == 429 and 429 not in kodlar[:20]


def test_unicode_rakam_kilidi_atlatamaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        for _ in range(5):
            giris(api, n, "yanlis-parola")
        assert giris(api, n, pw)[0] == 429
        d, j = post(api, "/api/giris", {"numara": "1\uff100001", "parola": pw, "wg_pub": WG, "ssh_pub": SSH})
        assert d == 400 and j["kod"] == "gecersiz" and q.isler == []


def test_lisans_bilgisi_ve_parola_tipi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert d == 200 and (j["lisans"], j["lisans_bitis"]) == ("aktif", "2027-09-21")
        a = j["cihaz_anahtari"]
        v.askiya_al(n, True)
        j = post(api, "/api/eslesme", {"cihaz_anahtari": a})[1]
        assert (j["durum"], j["lisans"]) == ("bagli", "askida")
        assert post(api, "/api/parola", {"cihaz_anahtari": a, "eski": 5, "yeni": "yeni-parola-1"})[0] == 400
        assert v.musteri(n)["parola_acik"] == pw
        post(api, "/api/parola", {"cihaz_anahtari": a, "eski": pw, "yeni": "yeni-parola-1"})
        assert v.musteri(n)["parola_acik"] == "yeni-parola-1"  # cihazdan değişen parola da yönetimde görünür


def test_giris_ve_eslesme_tek_admin_ozetini_tasir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert d == 200 and "admin" not in j  # yönetici belirlemediyse cihazdaki admin aynen kalır
        v.admin_parolasi_koy("benim-admin-parolam")
        d, j2 = post(api, "/api/eslesme", {"cihaz_anahtari": j["cihaz_anahtari"]})
        assert d == 200 and j2["admin"] == v.admin_parolasi() and "benim-admin-parolam" not in json.dumps(j2)
        d, j3 = post(api, "/api/eslesme", {"cihaz_anahtari": j["cihaz_anahtari"]})
        assert j3["admin"] == j2["admin"]  # her eşitlemede aynı özet: admin oturumları boşuna düşmez


def test_sms_ayari_yoksa_alan_yok_varsa_gider():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert "sms" not in j
        v.sms_ayari_koy({"mock": False, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""})
        d, j2 = post(api, "/api/eslesme", {"cihaz_anahtari": j["cihaz_anahtari"]})
        assert j2["sms"] == {"mock": False, "provider": "netgsm",
                             "netgsm": {"usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""}}
        d, j3 = post(api, "/api/eslesme", {"cihaz_anahtari": "yanlis"})
        assert d == 401 and "sms" not in j3


RAPOR = {"surum": "1.2.3", "acik_sn": 5000, "servisler": {"portal": True, "panel": True, "kaydedici": True, "dnsmasq": True, "guvenlik": False},
         "disk_yuzde": 42, "son_muhur": "2026-10-10", "kayit_boslugu_1s": False, "wan": True, "sms_sorunu": False, "bagli": 7, "hiz_hata": False}


def test_saglik_raporu_kaydedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        _, j = giris(api, n, pw)
        c = v.bagli_cihaz(n)
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": dict(RAPOR, ad="Ali", telefon="5551112233")})
        assert d == 200 and r == {"tamam": True}
        assert v.saglik(c["id"])["rapor"] == RAPOR  # bilinmeyen alanlar atıldı
        assert post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": dict(RAPOR, son_muhur="", bagli=0)})[0] == 200
        assert v.saglik(c["id"])["rapor"]["son_muhur"] == "" and v.saglik(c["id"])["rapor"]["bagli"] == 0
        # serbest bırakılan cihaz: 200 ama yazılmaz
        v.saglik_kaydet(c["id"], {"eski": 1})
        v.serbest_birak(n)
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": RAPOR})
        assert d == 200 and r == {"tamam": True} and v.saglik(c["id"])["rapor"] == {"eski": 1}


def test_bozuk_rapor_reddedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        _, j = giris(api, n, pw)
        k, cid = j["cihaz_anahtari"], v.bagli_cihaz(n)["id"]

        def yolla(rapor, ip="198.51.100.9"):
            return post(api, "/api/saglik", {"cihaz_anahtari": k, "rapor": rapor}, ip)
        eksik = dict(RAPOR)
        del eksik["wan"]
        kotu = [eksik, dict(RAPOR, disk_yuzde=101), dict(RAPOR, disk_yuzde=-1), dict(RAPOR, servisler=["portal"]),
                dict(RAPOR, servisler=dict(RAPOR["servisler"], portal=1)), dict(RAPOR, servisler={"portal": True}),
                dict(RAPOR, acik_sn=True), dict(RAPOR, bagli=True), dict(RAPOR, disk_yuzde=True), dict(RAPOR, acik_sn=-1),
                dict(RAPOR, wan=1), dict(RAPOR, hiz_hata="evet"), dict(RAPOR, surum="x" * 41), dict(RAPOR, surum=5),
                dict(RAPOR, son_muhur="10.10.2026"), dict(RAPOR, son_muhur="2026-13-40"), dict(RAPOR, acik_sn=1.5), "metin", None]
        for rp in kotu:
            d, r = yolla(rp)
            assert d == 400 and r["kod"] == "gecersiz", rp
        assert post(api, "/api/saglik", {"cihaz_anahtari": k})[0] == 400  # rapor yok
        assert v.saglik(cid) is None
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": "bilinmeyen", "rapor": RAPOR})
        assert d == 401 and r["kod"] == "taninmadi"


def test_saglik_govde_siniri_ve_hiz_siniri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        assert api.istek("POST", "/api/saglik", {}, "x" * 5000, "", "1.1.1.1")[0] == 413
        n, pw = v.musteri_ekle()
        _, j = giris(api, n, pw, ip="198.51.100.20")
        sonuc = [post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": RAPOR}, "203.0.113.5")[0] for _ in range(21)]
        assert sonuc[:20] == [200] * 20 and sonuc[20] == 429


def _surumlu(tmp, kanal="test", hedef="0.4.1"):
    api, v, q, saat = kur(tmp)
    dizin = os.path.join(tmp, "sur")
    os.makedirs(dizin)
    api.surum_dizini = dizin
    n, pw = v.musteri_ekle()
    _, j = giris(api, n, pw)
    for s in ("0.4.0", "0.4.1", "0.3.9"):
        b = f"deb-{s}".encode()
        with open(os.path.join(dizin, s + ".deb"), "wb") as f:
            f.write(b)
        v.surum_ekle(s, "ab" * 32, len(b), "yerel")
    v.kanal_hedef_koy("test", hedef)
    v.guncelleme_kanali_koy(n, kanal)
    return api, v, n, j["cihaz_anahtari"], saat


_ip = [0]


def rapor_yolla(api, k, **ek):
    _ip[0] += 1  # istek hız sınırına takılmasın
    return post(api, "/api/saglik", {"cihaz_anahtari": k, "rapor": dict(RAPOR, **ek)}, "203.0.113.%d" % _ip[0])


def test_rapor_paket_ve_guncelleme_alanlari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, _ = _surumlu(tmp)
        cid = v.bagli_cihaz(n)["id"]
        assert rapor_yolla(api, k)[0] == 200  # eski biçim
        assert "paket" not in v.saglik(cid)["rapor"] and "guncelleme" not in v.saglik(cid)["rapor"]
        g = {"surum": "0.4.1", "sonuc": "geri_alindi", "zaman": 5}
        assert rapor_yolla(api, k, paket="0.4.0", guncelleme=g)[0] == 200
        r = v.saglik(cid)["rapor"]
        assert r["paket"] == "0.4.0" and r["guncelleme"] == g
        olaylar = [h for h in v.hareketler(n) if h["olay"] == "GUNCELLEME_SONUC"]
        assert len(olaylar) == 1 and "surum=0.4.1" in olaylar[0]["ayrinti"] and "sonuc=geri_alindi" in olaylar[0]["ayrinti"]
        assert rapor_yolla(api, k, paket="0.4.0", guncelleme=g)[0] == 200  # değişmedi: yeni hareket yok
        assert len([h for h in v.hareketler(n) if h["olay"] == "GUNCELLEME_SONUC"]) == 1
        for kotu in (dict(g, sonuc="x"), dict(g, surum="1.2"), dict(g, zaman=-1), dict(g, zaman=True), "x", dict(g, ek=1)):
            assert rapor_yolla(api, k, guncelleme=kotu)[0] == 400, kotu
        for kotu in ("v" * 41, 5, "1.2", "../x"):
            assert rapor_yolla(api, k, paket=kotu)[0] == 400, kotu
        assert rapor_yolla(api, k, paket="", guncelleme=None)[0] == 200


def test_guncelleme_hedefi_kanal_lisans_ve_hemen():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, saat = _surumlu(tmp)
        d, r = rapor_yolla(api, k)
        assert r == {"tamam": True, "guncelleme": {"surum": "0.4.1", "sha256": "ab" * 32, "boyut": 9, "hemen": False}}
        v.guncelleme_hemen_koy(n, 100)
        assert rapor_yolla(api, k)[1]["guncelleme"]["hemen"] is True
        assert rapor_yolla(api, k, guncelleme={"surum": "0.4.1", "sonuc": "hata", "zaman": 50})[1]["guncelleme"]["hemen"] is True
        assert rapor_yolla(api, k, guncelleme={"surum": "0.4.1", "sonuc": "tamam", "zaman": 200})[1]["guncelleme"]["hemen"] is False
        assert v.musteri(n)["guncelleme_hemen"] == 0
        v.guncelleme_kanali_koy(n, "kapali")
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        v.guncelleme_kanali_koy(n, "kararli")  # kararlı kanalın hedefi yok
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        v.kanal_hedef_koy("kararli", "0.4.0")
        assert rapor_yolla(api, k)[1]["guncelleme"]["surum"] == "0.4.0"
        v.kanal_hedef_koy("kararli", "9.9.9")  # sürüm kaydı yok
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        v.kanal_hedef_koy("kararli", "0.4.0")
        v.askiya_al(n, True)
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        v.askiya_al(n, False)
        v.lisans_ayarla(n, "2020-01-01")
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        v.kanal_hedef_koy("kararli", None)
        assert v.kanal_hedef("kararli") is None


def test_api_paket_yetkisi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, _ = _surumlu(tmp)

        def paket(surum, anahtar=k):
            return api.istek("POST", "/api/paket", {}, json.dumps({"cihaz_anahtari": anahtar, "surum": surum}), "", "198.51.100.77")
        d, h, g = paket("0.4.1")
        assert d == 200 and g == b"deb-0.4.1" and h["Content-Type"] == "application/octet-stream" and h["Cache-Control"] == "no-store"
        d, _, g = paket("0.4.0")
        assert d == 403 and json.loads(g)["kod"] == "yetki"
        rapor_yolla(api, k, paket="0.4.0")  # geri dönüş paketi: raporlanan sürüm
        d, _, g = paket("0.4.0")
        assert d == 200 and g == b"deb-0.4.0"
        assert paket("0.3.9")[0] == 403
        assert paket("../x")[0] == 403
        assert paket("0.4.1", "bilinmeyen")[0] == 401
        v.askiya_al(n, True)
        assert paket("0.4.1")[0] == 403 and paket("0.4.0")[0] == 403
        v.askiya_al(n, False)
        assert paket("0.4.0")[0] == 200
        v.lisans_ayarla(n, "2020-01-01")
        assert paket("0.4.1")[0] == 403 and paket("0.4.0")[0] == 403
        v.lisans_uzat(n)
        v.uyelik(n, "bitti")
        assert paket("0.4.0")[0] == 403
        v.uyelik(n, "aktif")
        v.serbest_birak(n)
        assert paket("0.4.1")[0] == 403 and paket("0.4.0")[0] == 403
        v.serbest_iptal(n)
        os.remove(os.path.join(api.surum_dizini, "0.4.1.deb"))
        assert paket("0.4.1")[0] == 404


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
