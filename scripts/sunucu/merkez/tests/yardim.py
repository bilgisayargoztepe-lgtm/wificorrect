"""Uygulama testlerinin ortak kurulumu: geçici veritabanı, özet klasörleri, sahte saat, istek yardımcıları."""
import json
import os
import re
import sys
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import kayitlar  # noqa: E402
import veri  # noqa: E402


class Saat:
    t = 1_790_000_000.0  # 2026-09-21 ~ 17:00 (+03)

    def __call__(self):
        return self.t


def ortam(tmp):
    saat = Saat()
    v = veri.Veri(os.path.join(tmp, "m.db"), saat=saat)
    ist, det = os.path.join(tmp, "ist"), os.path.join(tmp, "detay")
    os.makedirs(ist)
    os.makedirs(det)
    return v, kayitlar.Kayitlar(ist, det, os.path.join(tmp, "kisi_sir")), saat


def sayi_yaz(tmp, ad, sayilar):
    with open(os.path.join(tmp, "ist", ad + ".json"), "w", encoding="utf-8") as f:
        json.dump(sayilar, f)


def gun_yaz(tmp, ad, gun, kisiler=(), dizin=None):
    d = os.path.join(tmp, "detay", ad)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, gun + ".json"), "w", encoding="utf-8") as f:
        json.dump({"gun": gun, "kisiler": list(kisiler)}, f)
    if dizin is not None:
        with open(os.path.join(d, "kisiler.json"), "w", encoding="utf-8") as f:
            json.dump(dizin, f)


def cerez(yanit):
    return yanit[1]["Set-Cookie"].split(";")[0]


def csrf(yanit):
    return re.search(r'name="csrf" value="([^"]+)"', yanit[2].decode()).group(1)


def al(app, yol, c="", ip="203.0.113.5"):
    yol, _, sorgu = yol.partition("?")
    return app.istek("GET", yol, {"Cookie": c}, "", sorgu, ip)


def gonder(app, yol, c, alanlar, ip="203.0.113.5"):
    return app.istek("POST", yol, {"Cookie": c}, urllib.parse.urlencode(alanlar), "", ip)


def giris(app, kul, pw, ip="203.0.113.5"):
    return app.istek("POST", "/giris", {}, urllib.parse.urlencode({"kullanici": kul, "sifre": pw}), "", ip)


def gerekce_ver(app, c, metin="Müşteri şikâyeti"):
    f = al(app, "/gerekce?donus=%2F", c)
    return gonder(app, "/gerekce", c, {"csrf": csrf(f), "gerekce": metin, "donus": "/"})
