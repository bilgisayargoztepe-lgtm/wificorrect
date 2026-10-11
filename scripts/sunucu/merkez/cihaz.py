"""cihaz.wificorrect.com (spec 2026-10-09): Caddy forward_auth buraya sorar; geçiş belirteci → çerez; çıkış."""
import time
import urllib.parse

import guvenlik
import web

PANEL_GIRIS = "https://panel.wificorrect.com/giris"
YONETIM = "https://yonetim.wificorrect.com"
CIHAZ_GIRIS = "https://cihaz.wificorrect.com/_giris?t="
CEREZ = "wcc"


def _cerez(token):
    if token is None:
        return f"{CEREZ}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    return f"{CEREZ}={token}; HttpOnly; Secure; SameSite=Lax; Path=/"  # panelden gelen yönlendirmede de gönderilsin


class Cihaz:
    def __init__(self, veri, gecis, musteri_app, saat=time.time):
        self.veri, self.gecis, self.musteri_app, self.saat = veri, gecis, musteri_app, saat
        self.oturumlar = guvenlik.Oturumlar()

    def _gecerli(self, token):
        """Dönen: (numara, tünel adresi) ya da None. Parola değiştiyse / lisans, üyelik kapandıysa / cihaz yoksa düşer."""
        ot = self.oturumlar.get(token, self.saat())
        if not ot:
            return None
        n = int(ot["user"])
        m = self.veri.musteri(n)
        adres = self.musteri_app.cihaz_adresi(n)
        if m is None or m["ozet"] != ot["surum"] or adres is None:
            self.oturumlar.drop(token)
            return None
        return ot["user"], adres

    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        token = web.cerez_degeri(basliklar.get("Cookie"), CEREZ)
        if yol == "/cihaz-yetki":
            g = self._gecerli(token)
            if g is None:
                return 302, {"Location": PANEL_GIRIS, "Cache-Control": "no-store"}, b""
            return 200, {"X-WFC-Kullanici": g[0], "X-WFC-Cihaz": g[1], "Cache-Control": "no-store"}, b""
        if yol == "/_giris":
            b = self.gecis.tuket((urllib.parse.parse_qs(sorgu).get("t") or [""])[0], self.saat())
            numara = b[0] if b else None
            m = self.veri.musteri(int(numara)) if numara else None
            if m is None or m["ozet"] != b[1]:
                return 303, {"Location": PANEL_GIRIS, "Cache-Control": "no-store"}, b""
            yeni = self.oturumlar.create(numara, b[2], self.saat(), m["ozet"])
            return 303, {"Location": "/", "Set-Cookie": _cerez(yeni), "Cache-Control": "no-store"}, b""
        if yol == "/_cikis":
            ot = self.oturumlar.get(token, self.saat())
            if ot and ot["role"] == "yonetici":
                # yönetimden gelen: yalnızca bu oturum kapanır, müşterinin kendi oturumları düşmez
                self.oturumlar.drop(token)
                return 303, {"Location": f"{YONETIM}/m/{ot['user']}", "Set-Cookie": _cerez(None), "Cache-Control": "no-store"}, b""
            if ot:
                self.oturumlar.drop_user(ot["user"])
                self.musteri_app.oturumlar.drop_user(ot["user"])
            return 303, {"Location": PANEL_GIRIS, "Set-Cookie": _cerez(None), "Cache-Control": "no-store"}, b""
        return 404, {"Content-Type": "text/plain; charset=utf-8"}, "Bulunamadı".encode("utf-8")
