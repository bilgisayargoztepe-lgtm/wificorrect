"""Kolay bağlama (spec 2026-10-16): bağlanmamış cihaz bekler, müşteri aynı internet adresinden girince kendiliğinden bağlanır."""
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import api as A  # noqa: E402
import cihaz as C  # noqa: E402
import guvenlik as G  # noqa: E402
import musteri as M  # noqa: E402
import yonetici  # noqa: E402
import yonetim as YN  # noqa: E402

WG, WG2 = "A" * 43 + "=", "B" * 43 + "="
SSH = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI cihaz"
TAMAM = {"durum": "tamam", "tunel_ip": "10.99.0.10", "sunucu_pub": "S" * 43 + "=", "uc_nokta": "vpn.wificorrect.com:51820"}
K1, S1 = "1" * 32, "a" * 64
K2, S2 = "2" * 32, "b" * 64
IP, IP2 = "203.0.113.5", "198.51.100.7"  # IP: yardim.al/gonder varsayılanı


class Kuyruk:
    def __init__(self, sonuc=TAMAM):
        self.isler, self.sonuc = [], sonuc

    def ekle(self, islem, veri):
        self.isler.append((islem, veri))
        return f"{len(self.isler):016x}"

    def bekle(self, kimlik):
        return self.sonuc


def kur(tmp, sonuc=TAMAM):
    v, k, saat = Y.ortam(tmp)
    q = Kuyruk(sonuc)
    api = A.Api(v, saat, q.ekle, q.bekle, liste_dizini=os.path.join(tmp, "listeler"))
    gecis = G.GecisBelirtecleri()
    m = M.Musteri(v, k, saat, gecis=gecis)
    return api, m, v, q, saat


def post(api, yol, govde, ip=IP):
    d, b, g = api.istek("POST", yol, {}, json.dumps(govde) if not isinstance(govde, str) else govde, "", ip)
    return d, json.loads(g)


def bildir(api, k=K1, s=S1, wg=WG, ip=IP, mac="00:0e:c4:ce:a0:9b", surum="0.4.13"):
    return post(api, "/api/bekliyor", {"kimlik": k, "sir": s, "surum": surum, "wan_mac": mac, "wg_pub": wg, "ssh_pub": SSH}, ip)


def bagla(api, belirtec, k=K1, s=S1, wg=WG, ip=IP):
    return post(api, "/api/bagla", {"kimlik": k, "sir": s, "belirtec": belirtec, "wg_pub": wg, "ssh_pub": SSH,
                                    "isletme_adi": "", "unvan": "", "surum": "0.4.13"}, ip)


def musteri_gir(m, n, pw, ip=IP):
    r = Y.giris(m, str(n), pw, ip=ip)
    return r, Y.cerez(r)


def test_bildirim_kaydeder_sir_tutmazsa_reddeder():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, _, v, _, saat = kur(tmp)
        assert bildir(api) == (200, {"durum": "bekle"})
        r = v.bekleyenler()
        assert len(r) == 1 and r[0]["kimlik"] == K1 and r[0]["ip"] == IP and r[0]["sir_ozet"] != S1
        assert [h["olay"] for h in v.hareketler()] == ["CIHAZ_BEKLIYOR"]
        saat.t += 120
        assert bildir(api, ip=IP2)[0] == 200
        assert v.bekleyenler()[0]["ip"] == IP2 and [h["olay"] for h in v.hareketler()] == ["CIHAZ_BEKLIYOR"]  # yalnızca ilk görülme
        d, j = bildir(api, s="c" * 64, ip=IP)  # kimliği bilen ama sırrı bilmeyen
        assert d == 401 and j["kod"] == "taninmadi" and v.bekleyenler()[0]["ip"] == IP2


def test_bildirim_girdi_dogrulama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, _, v, _, _ = kur(tmp)
        for k, s, wg, mac in (("x" * 32, S1, WG, "00:11:22:33:44:55"), (K1, "kisa", WG, "00:11:22:33:44:55"),
                              (K1, S1, "kotu", "00:11:22:33:44:55"), (K1, S1, WG, "<b>"), ("A" * 32, S1, WG, "")):
            assert bildir(api, k, s, wg, mac=mac)[1]["kod"] == "gecersiz", (k, s, wg, mac)
        assert bildir(api, mac="")[0] == 200  # WAN MAC bilinmiyorsa boş gelebilir
        assert v.bekleyenler()[0]["wan_mac"] == ""


def test_bildirim_hiz_ve_yeni_kimlik_siniri():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, _, v, _, saat = kur(tmp)
        assert [bildir(api)[0] for _ in range(4)] == [200, 200, 200, 429]  # kimlik başına dakikada 3
        saat.t += 61
        assert bildir(api)[0] == 200
        for i in range(9):  # aynı IP'den saatte 10 yeni kimlik (K1 dahil)
            saat.t += 4
            assert bildir(api, k=f"{i + 3:032x}", ip=IP)[0] == 200, i
        saat.t += 4
        assert bildir(api, k="f" * 32, ip=IP)[0] == 429
        assert bildir(api, k="f" * 32, ip=IP2)[0] == 200  # başka IP
        assert len(v.bekleyenler()) == 11


def test_eski_kayit_7_gunde_silinir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, _, v, _, saat = kur(tmp)
        bildir(api)
        saat.t += 7 * 86400 + 1
        bildir(api, k=K2, s=S2, ip=IP2)
        assert [r["kimlik"] for r in v.bekleyenler()] == [K2]
        assert bildir(api, s="c" * 64)[0] == 200  # silinen kimlik yeniden kayıt olur (yeni sır)


def test_tek_aday_kendiliginden_baglanir_uctan_uca():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api, ip=IP)
        r, c = musteri_gir(m, n, pw, ip=IP)
        assert r[1]["Location"] == "/baglan"
        s = Y.al(m, "/baglan", c, ip=IP)
        assert s[0] == 200 and "bağlanıyor" in s[2].decode() and 'http-equiv="refresh"' in s[2].decode()
        assert [h["olay"] for h in v.hareketler(n)][0] == "CIHAZ_OTOMATIK_AYRILDI"
        assert v.bekleyenler()[0]["musteri"] == n
        saat.t += 120
        d, j = bildir(api, ip=IP)
        assert d == 200 and j["durum"] == "baglan" and len(j["belirtec"]) > 30
        d, g = bagla(api, j["belirtec"])
        assert d == 200 and g["numara"] == str(n) and g["tunel_ip"] == "10.99.0.10" and g["yedek_hedefi"] == f"wfc-{n}@10.99.0.1:"
        assert len(g["cihaz_anahtari"]) > 30 and (g["tuz"], g["ozet"]) == (v.musteri(n)["tuz"], v.musteri(n)["ozet"])
        assert q.isler == [("cihaz-ekle", {"numara": str(n), "wg_pub": WG, "ssh_pub": SSH})]
        cz = v.bagli_cihaz(n)
        assert cz["tunel_ip"] == "10.99.0.10" and cz["wg_pub"] == WG
        h = [x for x in v.hareketler(n) if x["olay"] == "CIHAZ_BAGLANDI"]
        assert len(h) == 1 and "otomatik" in h[0]["ayrinti"]
        assert bagla(api, j["belirtec"])[1]["kod"] == "taninmadi"  # belirteç tek kullanımlık
        # bağlandıktan hemen sonra panel hazırlanıyor; 30 sn sonra cihaz paneline
        s = Y.al(m, "/baglan", c, ip=IP)
        assert s[0] == 200 and "hazırlanıyor" in s[2].decode()
        saat.t += 31
        assert Y.al(m, "/baglan", c, ip=IP)[1]["Location"] == "/cihaz"
        assert Y.al(m, "/cihaz", c, ip=IP)[1]["Location"].startswith("https://cihaz.wificorrect.com/_giris?t=")


def test_cevabi_kaybeden_cihaz_yeniden_baglan_alir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        saat.t += 120
        j = bildir(api)[1]
        assert bagla(api, j["belirtec"])[0] == 200
        saat.t += 120  # cihaz cevabı alamadı, yine bildiriyor
        d, j = bildir(api)
        assert d == 200 and j["durum"] == "baglan"
        d, g = bagla(api, j["belirtec"])
        assert d == 200 and g["numara"] == str(n)
        # başka wg anahtarıyla gelen (cihaz anahtarını değiştirmiş) aynı kimlik bağa dokunamaz
        saat.t += 120
        assert bildir(api, wg=WG2)[1] == {"durum": "bekle"}
        assert v.bagli_cihaz(n)["wg_pub"] == WG


def test_hic_aday_yoksa_bulunamadi_baska_ipden_bulunmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api, ip=IP2)
        c = musteri_gir(m, n, pw, ip=IP)[1]
        s = Y.al(m, "/baglan", c, ip=IP)
        assert s[0] == 200 and "bulunamadı" in s[2].decode() and "mobil veri" in s[2].decode()
        assert v.bekleyenler()[0]["musteri"] is None
        # aynı IP ama 10 dk'dır bildirmeyen cihaz aday değil
        bildir(api, k=K2, s=S2, ip=IP)
        saat.t += 601
        assert "bulunamadı" in Y.al(m, "/baglan", c, ip=IP)[2].decode()
        bildir(api, k=K2, s=S2, ip=IP)
        assert "bağlanıyor" in Y.al(m, "/baglan", c, ip=IP)[2].decode()


def test_birden_cok_aday_liste_ve_secim():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api, mac="00:0e:c4:ce:a0:9b")
        bildir(api, k=K2, s=S2, wg=WG2, mac="00:0e:c4:11:22:33")
        c = musteri_gir(m, n, pw)[1]
        s = Y.al(m, "/baglan", c)
        h = s[2].decode()
        assert s[0] == 200 and "A0:9B" in h and "22:33" in h and "00:0e:c4" not in h.lower()  # yalnızca son 4 hane
        assert all(r["musteri"] is None for r in v.bekleyenler())  # kendiliğinden ayrılmaz
        assert Y.gonder(m, "/baglan", c, {"csrf": "yanlis", "kimlik": K2})[0] == 403
        assert Y.gonder(m, "/baglan", c, {"csrf": Y.csrf(s), "kimlik": K2}, ip=IP2)[0] == 400  # başka IP'den seçilemez
        r = Y.gonder(m, "/baglan", c, {"csrf": Y.csrf(s), "kimlik": K2})
        assert r[0] == 303 and r[1]["Location"] == "/baglan"
        assert {r["kimlik"]: r["musteri"] for r in v.bekleyenler()} == {K1: None, K2: n}
        h = [x for x in v.hareketler(n) if x["olay"] == "CIHAZ_OTOMATIK_AYRILDI"]
        assert len(h) == 1 and "secim=liste" in h[0]["ayrinti"]
        assert bildir(api)[1] == {"durum": "bekle"} and bildir(api, k=K2, s=S2, wg=WG2)[1]["durum"] == "baglan"


def test_ayrilan_15_dk_baglanmazsa_serbest_ve_baska_musteri_kapamaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        n2, pw2 = v.musteri_ekle(parola="musteri-parola-2")
        bildir(api)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        c2 = musteri_gir(m, n2, pw2)[1]
        assert "bulunamadı" in Y.al(m, "/baglan", c2)[2].decode()  # n'e ayrılmış: n2 göremez
        saat.t += 901
        bildir(api)  # süre doldu: bekle (ayrılma düşer)
        assert v.bekleyenler()[0]["musteri"] is None
        assert "bağlanıyor" in Y.al(m, "/baglan", c2)[2].decode()
        assert v.bekleyenler()[0]["musteri"] == n2


def test_belirtec_kurallari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        t = bildir(api)[1]["belirtec"]
        assert bagla(api, t, s="c" * 64)[1]["kod"] == "taninmadi"  # sırrı bilmeyen
        assert bagla(api, "x" * 43)[1]["kod"] == "taninmadi"
        assert bagla(api, t, k=K2)[1]["kod"] == "taninmadi"
        assert post(api, "/api/bagla", {"kimlik": K1, "sir": S1, "belirtec": t, "wg_pub": "kotu", "ssh_pub": SSH})[1]["kod"] == "gecersiz"
        saat.t += 601  # belirteç 10 dk
        assert bagla(api, t)[1]["kod"] == "taninmadi"
        assert q.isler == [] and v.bagli_cihaz(n) is None


def test_bagli_cihazi_olan_musteri_ve_bagli_cihaz_kaydi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        n2, pw2 = v.musteri_ekle(parola="musteri-parola-2")
        v.cihaz_bagla(n, WG, SSH)  # yerel sayfadan bağlanmış cihaz (WG), bağ dosyasını kaybetmiş, bildiriyor
        bildir(api)
        assert "bulunamadı" in Y.al(m, "/baglan", musteri_gir(m, n2, pw2)[1])[2].decode()  # bağlı cihaz aday değil
        bildir(api, k=K2, s=S2, wg=WG2)
        r, c = musteri_gir(m, n, pw)
        assert r[1]["Location"] == "/"  # cihazı var (tünelsiz): bağlama yok
        assert Y.al(m, "/baglan", c)[1]["Location"] == "/cihaz"
        assert all(x["musteri"] != n for x in v.bekleyenler())


def test_ayrilmis_ama_musteri_baska_cihaz_baglarsa_bekle():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        t = bildir(api)[1]["belirtec"]
        v.cihaz_bagla(n, WG2, SSH)  # bu arada müşteri başka cihazı yerelden bağladı
        d, j = bagla(api, t)
        assert d == 409 and j["kod"] == "baska_cihaz" and q.isler == []
        assert bildir(api)[1] == {"durum": "bekle"}


def test_uyeligi_biten_musteri_baglanmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        v.uyelik(n, "bitti")
        bildir(api)
        r, c = musteri_gir(m, n, pw)
        assert r[1]["Location"] == "/"
        assert "sona ermiş" in Y.al(m, "/baglan", c)[2].decode()
        assert v.bekleyenler()[0]["musteri"] is None


def test_cihaz_sitesinden_gelen_oturumlu_musteri_giris_sonrasina():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, _ = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        c = musteri_gir(m, n, pw)[1]
        # cihaz.wificorrect.com oturumsuz → panel girişi; oturum varsa doğrudan bağlama/cihaz adımına
        assert Y.al(m, "/giris", c)[1]["Location"] == "/baglan"
        assert Y.al(m, "/cihaz", c)[1]["Location"] == "/baglan"
        v.cihaz_bagla(n, WG, SSH)
        v.cihaz_guncelle(v.bagli_cihaz(n)["id"], tunel_ip="10.99.0.11")
        assert Y.al(m, "/giris", c)[1]["Location"] == "/cihaz"


def test_yonetim_bekleyen_listesi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, _, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api, mac="00:0e:c4:ce:a0:9b", surum="<b>")
        bildir(api, k=K2, s=S2, wg=WG2, ip=IP2)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        saat.t += 1801
        bildir(api, k=K2, s=S2, wg=WG2, ip=IP2)
        yonetici.yaz(os.path.join(tmp, "yonetici.json"), "admin", "yonetici-parola-1")
        y = YN.Yonetim(v, m.kayitlar, saat, yonetici_yolu=os.path.join(tmp, "yonetici.json"),
                       durum_yolu=os.path.join(tmp, "durum.json"))
        cz = Y.cerez(Y.giris(y, "admin", "yonetici-parola-1"))
        h = Y.al(y, "/bekleyen", cz)[2].decode()
        assert "Bekleyen cihazlar" in h and IP in h and IP2 in h and "00:0e:c4:ce:a0:9b" in h
        assert "&lt;b&gt;" in h and "<b>" not in h  # cihazın bildirdiği sürüm kaçışlı
        assert f'/m/{n}' in h and "eski" in h
        assert "Bekleyen cihazlar" in Y.al(y, "/", cz)[2].decode()  # menüde


def test_kuyruk_hatasinda_cihaz_takilmaz():
    """Tünel/yedek kaydı (kuyruk) bir kez başarısız olsa da cihaz sonraki bildirimde yeniden bağlan alır; müşteri
    "ulaşılamıyor"a düşmez, "bağlanıyor" görür."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp, sonuc={"durum": "hata", "hata": "wg"})
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api)
        c = musteri_gir(m, n, pw)[1]
        Y.al(m, "/baglan", c)
        saat.t += 120
        d, j = bagla(api, bildir(api)[1]["belirtec"])
        assert d == 503 and j["kod"] == "kayit"
        assert "CIHAZ_KAYIT_HATA" in [h["olay"] for h in v.hareketler(n)]
        saat.t += 31
        s = Y.al(m, "/baglan", c)
        assert s[0] == 200 and "bağlanıyor" in s[2].decode()  # tünel yok: cihaz paneline gönderilmez
        q.sonuc = None  # kuyruk zaman aşımı: yine kurtulur
        saat.t += 120
        d, j = bildir(api)
        assert d == 200 and j["durum"] == "baglan"
        assert bagla(api, j["belirtec"])[0] == 503
        q.sonuc = TAMAM
        saat.t += 120
        d, j = bildir(api)
        assert d == 200 and j["durum"] == "baglan"
        d, g = bagla(api, j["belirtec"])
        assert d == 200 and g["numara"] == str(n) and v.bagli_cihaz(n)["tunel_ip"] == "10.99.0.10"
        saat.t += 31
        assert Y.al(m, "/baglan", c)[1]["Location"] == "/cihaz"


def test_bagla_cevabi_ortak_sms_sirlarini_tasimaz():
    """Kolay bağlamada (hesapsız, aynı IP) cevap yalnızca 0.4.13+ cihazın ihtiyacı olanı taşır: ortak NetGSM bilgisi
    ne bağlamada ne sonraki eşitlemede gider; parolalı /api/giris (eski cihazlar) aynen kalır."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        v.sms_ayari_koy({"mock": False, "usercode": "8500000000", "password": "netgsm-gizli", "msgheader": "BASLIK"})
        v.admin_parolasi_koy("admin-parola-123")
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        bildir(api)
        Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
        d, g = bagla(api, bildir(api)[1]["belirtec"])
        assert d == 200 and "netgsm-gizli" not in json.dumps(g) and "8500000000" not in json.dumps(g)
        assert g["sms"] == {"etkin_mock": False} and set(g["admin"]) == {"tuz", "ozet", "yineleme"}
        d, e = post(api, "/api/eslesme", {"cihaz_anahtari": g["cihaz_anahtari"], "surum": "0.4.7"})
        assert d == 200 and "netgsm-gizli" not in json.dumps(e) and e["sms"] == {"etkin_mock": False}
        # müşteriye özel AP yönetim parolası (ortak sır değil) bu cihaza da gider: cihaz panelinde gösterilir
        assert g["ap_parola"] == e["ap_parola"] == v.musteri(n)["ap_parola"]
        # parolalı giriş (eski cihazlar) değişmedi
        n2, pw2 = v.musteri_ekle(parola="musteri-parola-2")
        d, g2 = post(api, "/api/giris", {"numara": str(n2), "parola": pw2, "wg_pub": WG2, "ssh_pub": SSH})
        assert d == 200 and g2["sms"]["netgsm"]["password"] == "netgsm-gizli"
        d, e2 = post(api, "/api/eslesme", {"cihaz_anahtari": g2["cihaz_anahtari"]})
        assert e2["sms"]["netgsm"]["password"] == "netgsm-gizli"


def test_reddedilen_bildirim_ve_baglama_iz_birakir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        bildir(api)
        assert bildir(api, s="c" * 64)[0] == 401
        assert bildir(api, s="d" * 64)[0] == 401  # aynı kimlikte saatte bir satır
        assert bagla(api, "x" * 43)[0] == 401
        assert bagla(api, "y" * 43)[0] == 401
        h = sorted((x["olay"], x["ayrinti"]) for x in v.hareketler() if x["olay"].endswith("_RED"))
        assert h == [("CIHAZ_BAGLA_RED", f"kimlik={K1[:8]}"), ("CIHAZ_BEKLIYOR_RED", f"kimlik={K1[:8]}")], h
        assert all(S1 not in x["ayrinti"] and "c" * 64 not in x["ayrinti"] for x in v.hareketler())
        saat.t += 3601
        bildir(api, s="c" * 64)
        assert len([x for x in v.hareketler() if x["olay"] == "CIHAZ_BEKLIYOR_RED"]) == 2


def test_tavan_doluysa_bayat_kayit_yer_acar():
    import veri as V
    eski = V.BEKLEYEN_EN_COK
    V.BEKLEYEN_EN_COK = 3
    try:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
            api, m, v, q, saat = kur(tmp)
            n, pw = v.musteri_ekle(parola="musteri-parola-1")
            bildir(api, k="3" * 32)  # en eski; müşteriye ayrılacak, silinmemeli
            Y.al(m, "/baglan", musteri_gir(m, n, pw)[1])
            saat.t += 1
            bildir(api, k="4" * 32, ip=IP2)
            saat.t += 1
            bildir(api, k="5" * 32, ip=IP2)
            assert bildir(api, k="6" * 32, ip=IP2)[1]["kod"] == "dolu"  # hepsi taze: yer yok
            saat.t += 601  # 10 dk'dır bildirmeyen, kimseye ayrılmamış en eski kayıt yer açar
            assert bildir(api, k="6" * 32, ip=IP2) == (200, {"durum": "bekle"})
            assert sorted(r["kimlik"] for r in v.bekleyenler()) == ["3" * 32, "5" * 32, "6" * 32]
    finally:
        V.BEKLEYEN_EN_COK = eski


def test_silinmis_musteri_baglan_ve_cihaz_hata_vermez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        api, m, v, q, saat = kur(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        ot = {"user": str(n), "csrf": "x"}
        v.db.execute("DELETE FROM musteri WHERE numara = ?", (n,))
        for yol in ("/baglan", "/cihaz"):
            assert m.sayfalar(ot, "t", "GET", yol, "", {}, IP)[0] in (200, 303), yol


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
