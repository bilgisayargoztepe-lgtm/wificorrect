import os
import sys
import tempfile

os.environ["WC_TEST"] = "1"  # netgsm.gonder gerçek isteği reddeder (yanlışlıkla SMS gitmesin)

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import merkez as MZ  # noqa: E402


class Sahte:
    def __init__(self, ad):
        self.ad = ad

    def istek(self, yontem, yol, basliklar, govde, sorgu, ip):
        return 200, {}, f"{self.ad} {yol} {ip}".encode()


def test_host_yonlendirme():
    m = MZ.Merkez({"yonetim": Sahte("Y"), "musteri": Sahte("M"), "api": Sahte("A")})
    assert m.istek("GET", "/", {"Host": "yonetim.wificorrect.com"}, "", "", "1.1.1.1")[2] == b"Y / 1.1.1.1"
    assert m.istek("GET", "/", {"Host": "PANEL.wificorrect.com:443"}, "", "", "1.1.1.1")[2].startswith(b"M")
    assert m.istek("POST", "/api/giris", {"Host": "api.wificorrect.com"}, "", "", "1.1.1.1")[2].startswith(b"A")
    for h in ("wificorrect.com", "evil.com", "", "yonetim.wificorrect.com.evil.com"):
        assert m.istek("GET", "/", {"Host": h}, "", "", "1.1.1.1")[0] == 404, h


def test_musteri_cerezi_yonetimde_gecmez():
    import musteri, yonetim, yonetici
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, k, saat = Y.ortam(tmp)
        yonetici.yaz(os.path.join(tmp, "y.json"), "serkan", "yonetici-parola-1")
        m = MZ.Merkez({"yonetim": yonetim.Yonetim(v, k, saat, os.path.join(tmp, "y.json"), os.path.join(tmp, "d.json")),
                       "musteri": musteri.Musteri(v, k, saat)})
        n, pw = v.musteri_ekle()
        import urllib.parse
        r = m.istek("POST", "/giris", {"Host": "panel.wificorrect.com"},
                    urllib.parse.urlencode({"kullanici": str(n), "sifre": pw}), "", "1.1.1.1")
        c = Y.cerez(r)
        assert m.istek("GET", "/", {"Host": "panel.wificorrect.com", "Cookie": c}, "", "", "1.1.1.1")[0] == 200
        assert m.istek("GET", "/", {"Host": "yonetim.wificorrect.com", "Cookie": c}, "", "", "1.1.1.1")[0] == 303


def test_istemci_ip_son_xff():
    assert MZ.istemci_ip({"X-Forwarded-For": "6.6.6.6, 203.0.113.5"}) == "203.0.113.5"
    assert MZ.istemci_ip({}) == "yerel"


def test_fail2ban_hatali_ve_kilitli_satirlari_yakalar():
    import re
    s = open(os.path.join(HERE, "..", "fail2ban-wc-merkez-filtre.conf"), encoding="utf-8").read()
    satirlar = [x.strip() for x in s.split("failregex =", 1)[1].split("journalmatch")[0].splitlines() if x.strip()]
    desenler = [re.compile(x.replace("<HOST>", r"(?P<host>\S+)")) for x in satirlar]
    for log in ("GIRIS_HATALI alan=musteri kullanici='100001' ip=203.0.113.5",
                "GIRIS_KILITLI alan=yonetici kullanici='serkan' ip=203.0.113.5"):
        assert any(d.search(log) and d.search(log).group("host") == "203.0.113.5" for d in desenler), log
    jail = open(os.path.join(HERE, "..", "fail2ban-wc-merkez.conf"), encoding="utf-8").read()
    assert "maxretry = 5" in jail and "192.168.1.0/24" in jail and "10.99.0.0/24" in jail


def test_soket_zaman_asimi():
    assert MZ.Isleyici.timeout == 20


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
