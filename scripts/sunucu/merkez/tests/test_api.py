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
    return A.Api(v, saat, q.ekle, q.bekle, liste_dizini=os.path.join(tmp, "listeler")), v, q, saat


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
        d, j = post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_adi": "Bocafe Göztepe", "unvan": "<b>", "surum": "1.1",
                                          "adres": "Moda Cad. 1 Kadıköy"})
        assert d == 200 and j["durum"] == "bagli" and j["uyelik"] == "aktif"
        c = v.bagli_cihaz(n)
        assert c["isletme_adi"] == "Bocafe Göztepe" and c["surum"] == "1.1" and c["son_eslesme"]
        assert c["adres"] == "Moda Cad. 1 Kadıköy"
        # adres göndermeyen (eski) cihaz kayıtlı adresi silmez; gönderilen bozuk adres boşalır
        post(api, "/api/eslesme", {"cihaz_anahtari": a, "isletme_adi": "Bocafe Göztepe", "unvan": "<b>", "surum": "1.0"})
        assert v.bagli_cihaz(n)["adres"] == "Moda Cad. 1 Kadıköy"
        post(api, "/api/eslesme", {"cihaz_anahtari": a, "adres": "a\x00b"})
        assert v.bagli_cihaz(n)["adres"] == ""
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


def test_giris_ve_eslesme_ap_parolasini_tasir():
    """AP yönetim parolası (final review I1; müşteriye özel, panel parolasından ayrı) cihaza bağlanırken ve her eşitlemede
    gider: cihaz paneli AP kurulumunda işletme sahibine de gösterilir. Yenilenen parola sonraki eşitlemede iner."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        ap = v.musteri(n)["ap_parola"]
        assert d == 200 and j["ap_parola"] == ap and ap != pw
        yeni = v.ap_parolasi_yenile(n)
        assert post(api, "/api/eslesme", {"cihaz_anahtari": j["cihaz_anahtari"]})[1]["ap_parola"] == yeni
        d, h = post(api, "/api/eslesme", {"cihaz_anahtari": "yanlis"})
        assert d == 401 and "ap_parola" not in h


def test_sms_ayari_yoksa_yalnizca_etkin_mock_varsa_gider():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        d, j = giris(api, n, pw)
        assert j["sms"] == {"etkin_mock": True}  # eski cihaz `netgsm` olmadan eki yok sayar
        v.sms_ayari_koy({"mock": False, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""})
        d, j2 = post(api, "/api/eslesme", {"cihaz_anahtari": j["cihaz_anahtari"]})
        assert j2["sms"] == {"mock": False, "provider": "netgsm", "etkin_mock": False,
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
        assert d == 200 and r == {"tamam": True, "sms_etkin_mock": True, "listeler": {}, "ap_parola": v.musteri(n)["ap_parola"]}
        assert v.saglik(c["id"])["rapor"] == RAPOR  # bilinmeyen alanlar atıldı
        assert post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": dict(RAPOR, son_muhur="", bagli=0)})[0] == 200
        assert v.saglik(c["id"])["rapor"]["son_muhur"] == "" and v.saglik(c["id"])["rapor"]["bagli"] == 0
        # serbest bırakılan cihaz: 200 ama yazılmaz
        v.saglik_kaydet(c["id"], {"eski": 1})
        v.serbest_birak(n)
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": RAPOR})
        # bırakılan cihaza liste sürümü bildirilmez (indiremeyeceği listeyi her raporda denemesin)
        assert d == 200 and r == {"tamam": True, "sms_etkin_mock": True} and v.saglik(c["id"])["rapor"] == {"eski": 1}


def test_saglik_hizmet_kapaliyken_liste_surumu_yok():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        _, j = giris(api, n, pw)
        v.askiya_al(n, True)
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": RAPOR})
        assert d == 200 and "listeler" not in r
        v.askiya_al(n, False)
        d, r = post(api, "/api/saglik", {"cihaz_anahtari": j["cihaz_anahtari"], "rapor": RAPOR})
        assert d == 200 and r["listeler"] == {}


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


def test_saglik_cevabinda_ap_parola():
    """PPPoE (spec 2026-10-23, "AP yönetim parolası cihaza 5 dk içinde ulaşır"): müşterinin AP yönetim parolası bağlı cihazın
    5 dk'lık sağlık cevabında da gider (yönetimde "Yenile"den sonra cihaz günlük 06:00 eşitlemesini beklemez); müşteride parola
    boşsa alan hiç yok. Bırakılan cihaza ve tanınmayan anahtara gitmez; parola hareket satırına yazılmaz."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        _, j = giris(api, n, pw)
        k = j["cihaz_anahtari"]
        ap = v.musteri(n)["ap_parola"]
        assert ap and ap != pw
        d, r = rapor_yolla(api, k)
        assert d == 200 and r["ap_parola"] == ap
        yeni = v.ap_parolasi_yenile(n)  # yönetimde "Yenile": sonraki rapor cevabında iner
        assert yeni != ap and rapor_yolla(api, k)[1]["ap_parola"] == yeni
        # hizmet (lisans/üyelik) kapalı olsa da /api/eslesme gibi gider: cihazın bildiği parola merkezle aynı kalsın
        v.askiya_al(n, True)
        assert rapor_yolla(api, k)[1]["ap_parola"] == yeni
        v.askiya_al(n, False)
        # müşteride parola yok: alan hiç gönderilmez (boş dize de değil)
        v.db.execute("UPDATE musteri SET ap_parola = '' WHERE numara = ?", (n,))
        d, r = rapor_yolla(api, k)
        assert d == 200 and "ap_parola" not in r and r["tamam"] is True
        v.db.execute("UPDATE musteri SET ap_parola = ? WHERE numara = ?", (yeni, n))
        # parola hareket satırlarına ve kayıtlı rapora yazılmaz
        assert all(yeni not in (h["ayrinti"] or "") and ap not in (h["ayrinti"] or "") for h in v.hareketler())
        assert yeni not in json.dumps(v.saglik(v.bagli_cihaz(n)["id"])["rapor"])
        # tanınmayan anahtar: 401, parola yok
        d, h = post(api, "/api/saglik", {"cihaz_anahtari": "yanlis", "rapor": RAPOR})
        assert d == 401 and "ap_parola" not in h
        # bırakılan cihaz (bağlı değil): 200 ama parola gitmez
        v.serbest_birak(n)
        d, r = rapor_yolla(api, k)
        assert d == 200 and "ap_parola" not in r


def test_saglik_temizle_pppoe():
    """PPPoE (spec 2026-10-23 "Durum ve teşhis"): rapora wan_tur (dhcp|pppoe) ve pppoe (durum kodu) girer. İkisi de isteğe
    bağlı ve ileriye dönük uyumlu: eski cihaz göndermez (eskisi gibi kabul); bilinmeyen ya da bozuk değer (yeni bir cihaz
    sürümünün yeni durum kodu dahil) RAPORU REDDETMEZ, yalnızca o alan atılır (reddedilseydi çalışan cihaz için "Rapor
    gelmiyor" uyarısı/SMS'i çıkardı). Öteki alanlar katı kalır."""
    t = A.saglik_temizle(dict(RAPOR, wan_tur="pppoe", pppoe="bagli"))
    assert t == dict(RAPOR, wan_tur="pppoe", pppoe="bagli")
    eski = A.saglik_temizle(RAPOR)  # alanlar yok: eskisi gibi, anahtarlar da eklenmez
    assert eski == RAPOR and "wan_tur" not in eski and "pppoe" not in eski
    for tur in ("dhcp", "pppoe"):
        assert A.saglik_temizle(dict(RAPOR, wan_tur=tur))["wan_tur"] == tur
    for durum in ("kapali", "kablo_yok", "dhcp_yok", "sunucu_yok", "kimlik_hatasi", "baglaniyor", "bagli"):
        assert A.saglik_temizle(dict(RAPOR, pppoe=durum))["pppoe"] == durum
    assert "pppoe" not in A.saglik_temizle(dict(RAPOR, wan_tur="dhcp"))  # tek alan gelebilir
    assert "ek" not in A.saglik_temizle(dict(RAPOR, wan_tur="dhcp", pppoe="kapali", ek=1))  # bilinmeyen alan hâlâ atılır
    # bilinmeyen / bozuk değer: rapor KABUL edilir, yalnızca o alan yoktur (öteki alanlar aynen)
    for kotu in ("x", "", "PPPoE", "pppoe ", "ppp", 5, None, True, ["pppoe"], {"pppoe": 1}):
        assert A.saglik_temizle(dict(RAPOR, wan_tur=kotu)) == RAPOR, kotu
    for kotu in ("?", "", "Bagli", "bagli ", "kablo yok", "yeni_durum", 5, None, False, ["bagli"], {"bagli": 1}):
        assert A.saglik_temizle(dict(RAPOR, pppoe=kotu)) == RAPOR, kotu
    # biri geçerli, öteki bozuk: geçerli olan saklanır, bozuk olan atılır (ikisi birden atılmaz)
    assert A.saglik_temizle(dict(RAPOR, wan_tur="pppoe", pppoe="?")) == dict(RAPOR, wan_tur="pppoe")
    assert A.saglik_temizle(dict(RAPOR, wan_tur="x", pppoe="bagli")) == dict(RAPOR, pppoe="bagli")
    assert A.saglik_temizle(dict(RAPOR, wan_tur="x", pppoe="?")) == RAPOR
    # öteki alanlar katı kalır: yeni alanlar geçerli olsa da bozuk bir alan raporu reddeder; yeni alanlar bozuk olsa da bozuk alan hâlâ red sebebi
    assert A.saglik_temizle(dict(RAPOR, wan_tur="pppoe", pppoe="bagli", disk_yuzde=101)) is None
    assert A.saglik_temizle(dict(RAPOR, wan_tur="x", pppoe="?", disk_yuzde=101)) is None
    assert A.saglik_temizle(dict(RAPOR, wan_tur="x", paket=5)) is None
    assert A.saglik_temizle({a: b for a, b in dict(RAPOR, wan_tur="pppoe", pppoe="bagli").items() if a != "wan"}) is None  # zorunlu alan eksik
    # uçtan uca: geçerli rapor saklanır; yeni/bozuk değerli rapor 200 ile kabul edilir (Rapor gelmiyor çıkmaz) ve o alan olmadan saklanır
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, q, _ = kur(tmp)
        n, pw = v.musteri_ekle()
        k = giris(api, n, pw)[1]["cihaz_anahtari"]
        cid = v.bagli_cihaz(n)["id"]
        assert rapor_yolla(api, k)[0] == 200
        assert "wan_tur" not in v.saglik(cid)["rapor"] and "pppoe" not in v.saglik(cid)["rapor"]
        assert rapor_yolla(api, k, wan_tur="pppoe", pppoe="bagli")[0] == 200
        r = v.saglik(cid)["rapor"]
        assert (r["wan_tur"], r["pppoe"]) == ("pppoe", "bagli")
        for tuhaf, beklenen in ((dict(wan_tur="pppoe", pppoe="yeni_durum"), dict(wan_tur="pppoe")),  # gelecekteki yeni durum kodu
                                (dict(wan_tur="l2tp", pppoe="bagli"), dict(pppoe="bagli")),
                                (dict(wan_tur=5, pppoe=None), {})):
            d, h = rapor_yolla(api, k, **tuhaf)
            assert d == 200 and h["tamam"] is True, tuhaf
            assert v.saglik(cid)["rapor"] == dict(RAPOR, **beklenen), tuhaf  # yeni rapor kaydedildi; öncekinin alanı kalmaz
        # başka bir alan bozuksa rapor hâlâ reddedilir ve öncekinin üzerine yazmaz
        onceki = v.saglik(cid)["rapor"]
        d, h = rapor_yolla(api, k, wan_tur="pppoe", pppoe="bagli", disk_yuzde=101)
        assert d == 400 and h["kod"] == "gecersiz" and v.saglik(cid)["rapor"] == onceki


def test_guncelleme_hedefi_kanal_lisans_ve_hemen():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, saat = _surumlu(tmp)
        d, r = rapor_yolla(api, k)
        assert r == {"tamam": True, "sms_etkin_mock": True, "listeler": {}, "ap_parola": v.musteri(n)["ap_parola"],
                     "guncelleme": {"surum": "0.4.1", "sha256": "ab" * 32, "boyut": 9, "hemen": False}}
        v.guncelleme_hemen_koy(n, 100, "0.4.1")
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


def test_hemen_bayragi_hedef_surume_bagli():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, _ = _surumlu(tmp)
        # bayrak başka bir hedef için konmuştu: gönderilmez, sağlık raporunda temizlenir
        v.guncelleme_hemen_koy(n, 100, "0.4.0")
        assert rapor_yolla(api, k)[1]["guncelleme"]["hemen"] is False
        assert v.musteri(n)["guncelleme_hemen"] == 0 and v.musteri(n)["guncelleme_hemen_surum"] == ""
        # cihaz zaten hedefte: sonuç gelmeyecek, bayrak temizlenir
        v.guncelleme_hemen_koy(n, 100, "0.4.1")
        assert rapor_yolla(api, k, paket="0.4.0")[1]["guncelleme"]["hemen"] is True
        assert v.musteri(n)["guncelleme_hemen"] == 100
        assert rapor_yolla(api, k, paket="0.4.1")[1]["guncelleme"]["hemen"] is False
        assert v.musteri(n)["guncelleme_hemen"] == 0
        # hedef kalktı (kanal kapalı): bayrak temizlenir, kanal yeniden açılınca eski bayrak geri gelmez
        v.guncelleme_hemen_koy(n, 100, "0.4.1")
        v.guncelleme_kanali_koy(n, "kapali")
        assert "guncelleme" not in rapor_yolla(api, k)[1]
        assert v.musteri(n)["guncelleme_hemen"] == 0
        v.guncelleme_kanali_koy(n, "test")
        assert rapor_yolla(api, k)[1]["guncelleme"]["hemen"] is False


def test_onceki_paketin_sha256_si():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, _ = _surumlu(tmp)
        v.surum_ekle("0.4.0", "cd" * 32, 9, "yerel")
        assert "onceki_sha256" not in rapor_yolla(api, k)[1]["guncelleme"]  # paket bildirilmedi
        assert rapor_yolla(api, k, paket="0.4.0")[1]["guncelleme"]["onceki_sha256"] == "cd" * 32
        assert "onceki_sha256" not in rapor_yolla(api, k, paket="0.2.0")[1]["guncelleme"]  # depoda yok
        assert "onceki_sha256" not in rapor_yolla(api, k, paket="")[1]["guncelleme"]


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


# --- raporlar cihaz panelinde (spec Ek 2026-10-15): /api/raporlar, /api/rapor ---
_IP = [0]


def rpost(api, yol, govde):
    """Her istek ayrı IP'den: sabit saatte IP başına rapor sınırına (dakikada 60) takılmasın."""
    _IP[0] += 1
    return post(api, yol, govde, ip=f"198.51.{100 + _IP[0] // 250}.{_IP[0] % 250 + 1}")


def _raporlu(tmp):
    import raporlar as RP
    import test_raporlar as TR
    v, _, saat = Y.ortam(tmp)
    q = Kuyruk()
    api = A.Api(v, saat, q.ekle, q.bekle, liste_dizini=os.path.join(tmp, "listeler"),
                raporlar=RP.Raporlar(os.path.join(tmp, "rapor")))
    n, pw = v.musteri_ekle()
    k = giris(api, n, pw)[1]["cihaz_anahtari"]
    return api, v, n, k, RP, TR


def test_api_raporlar_liste_ve_donem():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, RP, TR = _raporlu(tmp)
        d, j = rpost(api, "/api/raporlar", {"cihaz_anahtari": k})
        assert d == 200 and j == {"raporlar": [], "bugun": "2026-09-21"}  # henüz rapor yok
        TR.yaz(tmp, n, "liste.json", TR.LISTE + [{"tur": "yil", "donem": "2026"}])
        TR.yaz(tmp, n, "ay-2026-09.json", TR.ay_raporu())
        d, j = rpost(api, "/api/raporlar", {"cihaz_anahtari": k})
        assert d == 200 and j["raporlar"] == TR.LISTE and j["bugun"] == "2026-09-21"
        d, j = rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"})
        assert d == 200 and set(j) == {"rapor", "isletme", "cihaz", "bugun"}
        beklenen = RP.rapor_verisi(v, api.raporlar, n, "ay", "2026-09", Y.Saat.t)  # panel sayfasıyla aynı veri
        assert {a: j[a] for a in ("rapor", "isletme", "cihaz")} == json.loads(json.dumps(beklenen))
        assert j["isletme"] == {"numara": str(n), "ad": "Bocafe", "unvan": "Bocafe Ltd."}
        assert j["cihaz"] == {"surum": "0.4.9", "cevrimdisi_sn": 0, "sorunlar": []}
        assert j["rapor"]["misafir"]["farkli"] == 41 and j["rapor"]["kayit"] == {"gun": 20, "tam": 16}
        assert j["rapor"]["kullanim"] == {"gonderilen": 1024 ** 3, "alinan": 3 * 1024 ** 3, "sms": 70}
        metin = json.dumps(j)
        assert "instagram" not in metin and "alan" not in metin and "imza" not in metin
        d, j = rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "hafta", "donem": "2026-W38"})
        assert d == 404 and j["kod"] == "yok"
        # görüntüleme hareket yazmaz
        assert [h["olay"] for h in v.hareketler(n)] == ["CIHAZ_BAGLANDI"]


def test_api_rapor_dogrulama_ve_yetki():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, RP, TR = _raporlu(tmp)
        n2, _ = v.musteri_ekle()
        TR.yaz(tmp, n2, "ay-2026-09.json", TR.ay_raporu())
        TR.yaz(tmp, n2, "liste.json", TR.LISTE)
        assert rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"})[0] == 404  # başka müşteri
        assert rpost(api, "/api/raporlar", {"cihaz_anahtari": k})[1]["raporlar"] == []
        TR.yaz(tmp, n, "ay-2026-09.json", TR.ay_raporu())
        for tur, donem in (("ay", "2026-13"), ("hafta", "2026-W54"), ("hafta", "../x"), ("yil", "2026"), ("ay", 202609),
                           (None, "2026-09"), ("ay", f"2026-09/../../{n2}"), ("ay", "2026-9"), ("hafta", "2026-w38")):
            d, j = rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": tur, "donem": donem})
            assert d == 400 and j["kod"] == "gecersiz", (tur, donem)
        assert rpost(api, "/api/rapor", {"tur": "ay", "donem": "2026-09"})[0] == 401
        assert rpost(api, "/api/raporlar", {"cihaz_anahtari": "bilinmeyen"})[0] == 401

        def iki():
            return (rpost(api, "/api/raporlar", {"cihaz_anahtari": k})[0],
                    rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"})[0])
        assert iki() == (200, 200)
        v.askiya_al(n, True)
        assert iki() == (403, 403)
        v.askiya_al(n, False)
        v.lisans_ayarla(n, "2020-01-01")
        assert iki() == (403, 403)
        v.lisans_uzat(n)
        v.uyelik(n, "bitti")
        assert iki() == (403, 403)
        v.uyelik(n, "aktif")
        assert iki() == (200, 200)
        v.serbest_birak(n)
        assert iki() == (403, 403)
        d, j = rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"})
        assert j["kod"] == "yetki"


def test_rapor_hiz_siniri_sms_butcesinden_ayri():
    """Rapor gezinmek (/api/raporlar, /api/rapor) misafir SMS'inin IP başına bütçesini tüketmez; kendi sınırı var."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, RP, TR = _raporlu(tmp)
        TR.yaz(tmp, n, "liste.json", TR.LISTE)
        TR.yaz(tmp, n, "ay-2026-09.json", TR.ay_raporu())
        ip = "203.0.113.9"
        kodlar = [post(api, "/api/rapor" if i % 2 else "/api/raporlar",
                       {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"}, ip)[0] for i in range(61)]
        assert kodlar[:60] == [200] * 60 and kodlar[60] == 429  # raporlara IP başına dakikada 60
        # aynı IP'den SMS/sağlık/eşleşme bütçesi (dakikada 20) hâlâ dolu değil
        assert [post(api, "/api/eslesme", {"cihaz_anahtari": k}, ip)[0] for _ in range(20)] == [200] * 20
        assert post(api, "/api/eslesme", {"cihaz_anahtari": k}, ip)[0] == 429
        # ortak bütçe dolunca da raporlar kendi sınırına bakar (başka IP, temiz rapor bütçesi)
        ip2 = "203.0.113.10"
        assert [post(api, "/api/eslesme", {"cihaz_anahtari": k}, ip2)[0] for _ in range(21)][-1] == 429
        assert post(api, "/api/rapor", {"cihaz_anahtari": k, "tur": "ay", "donem": "2026-09"}, ip2)[0] == 200


def test_api_rapor_liste_ya_da_sozluk_tur_400():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, v, n, k, RP, TR = _raporlu(tmp)
        for tur, donem in (([], "2026-09"), ({}, "2026-09"), ("ay", []), ("ay", {})):
            d, j = rpost(api, "/api/rapor", {"cihaz_anahtari": k, "tur": tur, "donem": donem})
            assert d == 400 and j["kod"] == "gecersiz", (tur, donem)


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
