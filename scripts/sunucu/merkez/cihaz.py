"""cihaz.wificorrect.com (spec 2026-10-09): Caddy forward_auth buraya sorar; geçiş belirteci → çerez; çıkış.
/_ap-gecis (spec 2026-10-16): cihaz paneli oturumundan AP arayüzüne (ap.wificorrect.com, ap.py) tek kullanımlık geçiş."""
import time
import urllib.parse

import ap
import guvenlik
import web

PANEL_GIRIS = "https://panel.wificorrect.com/giris"
YONETIM = "https://yonetim.wificorrect.com"
CIHAZ_GIRIS = "https://cihaz.wificorrect.com/_giris?t="
CEREZ = "wcc"
AP_GECIS_SINIRI = 20  # müşteri başına dakikada (API ile aynı); her geçiş bir hareket satırı yazar


def _cerez(token):
    if token is None:
        return f"{CEREZ}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    return f"{CEREZ}={token}; HttpOnly; Secure; SameSite=Lax; Path=/"  # panelden gelen yönlendirmede de gönderilsin


class Cihaz:
    def __init__(self, veri, gecis, musteri_app, saat=time.time):
        self.veri, self.gecis, self.musteri_app, self.saat = veri, gecis, musteri_app, saat
        self.oturumlar = guvenlik.Oturumlar()
        # AP arayüzüne geçiş belirteçleri: ap.Ap (/_ap-giris) tüketir
        self.ap_belirtecleri = guvenlik.TekKullanimlik(guvenlik.GecisBelirtecleri.SURE)
        self.ap_hiz = guvenlik.HizSiniri(AP_GECIS_SINIRI, 60)

    def oturum(self, token):
        """Geçerli cihaz paneli oturumu: {"numara", "rol", "adres" (tünel IP'si), "cihaz" (kayıt no)} ya da None. Parola
        değiştiyse / lisans, üyelik kapandıysa / cihaz yoksa oturum düşer. Her bakış oturumu canlı tutar (boşta 30 dk)."""
        ot = self.oturumlar.get(token, self.saat())
        if not ot:
            return None
        n = int(ot["user"])
        m = self.veri.musteri(n)
        c = self.musteri_app.calisan_cihaz(n)
        if m is None or m["ozet"] != ot["surum"] or c is None:
            self.oturumlar.drop(token)
            return None
        return {"numara": ot["user"], "rol": ot["role"], "adres": c["tunel_ip"], "cihaz": c["id"]}

    def ap_gecis(self, token, basliklar, ip):
        """Cihaz panelindeki "AP arayüzüne git" → bu oturuma, müşteriye ve cihaza bağlı 60 sn'lik tek kullanımlık belirteç →
        ap.wificorrect.com/_ap-giris. Yalnızca cihaz.wificorrect.com sayfasından üst düzey gezinme (ap.ust_duzey_gezinme).
        Hareket satırında yalnızca kim (numara ya da admin) ve rol."""
        if not ap.ust_duzey_gezinme(basliklar, ap.GECIS_KAYNAKLARI):
            if token:  # oturumlu tarayıcı: saldırı ya da beklenmedik tarayıcı davranışı (teşhis için)
                web.log(f"AP_GECIS_RED {ap.getirme_ozeti(basliklar)} ip={ip}")
            return ap.sayfa(403, "AP arayüzü açılamadı", ap.ELLE)
        o = self.oturum(token)
        if o is None:
            return 303, {"Location": PANEL_GIRIS, "Cache-Control": "no-store"}, b""
        simdi = self.saat()
        if not self.ap_hiz.izin(o["numara"], simdi):
            return ap.sayfa(429, "Çok fazla deneme", "AP arayüzü kısa sürede çok kez açıldı. Bir dakika sonra yeniden deneyin.")
        t = self.ap_belirtecleri.uret({"wcc": token, **o}, simdi)
        self.veri.hareket("admin" if o["rol"] == "yonetici" else o["numara"], ip, "AP_GECIS", int(o["numara"]),
                          f"rol={o['rol']}")
        return 303, {"Location": ap.GIRIS + t, "Cache-Control": "no-store"}, b""

    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        token = web.cerez_degeri(basliklar.get("Cookie"), CEREZ)
        if yol == "/cihaz-yetki":
            o = self.oturum(token)
            if o is None:
                return 302, {"Location": PANEL_GIRIS, "Cache-Control": "no-store"}, b""
            return 200, {"X-WFC-Kullanici": o["numara"], "X-WFC-Cihaz": o["adres"], "Cache-Control": "no-store"}, b""
        if yol == "/_ap-gecis":
            return self.ap_gecis(token, basliklar, ip)
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
