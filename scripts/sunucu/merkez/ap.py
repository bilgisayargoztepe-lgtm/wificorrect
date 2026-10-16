"""ap.wificorrect.com (spec 2026-10-16 "AP arayüzüne panelden tek tıkla erişim", Mimari 2–3): müşterinin taktığı AP'nin ya
da modemin kendi web arayüzü cihaz üzerinden (tünel → cihaz → yan oda ya da misafir ağı) bu alan adının kökünde açılır;
merkez yalnızca kimliği doğrular. Akış: cihaz.wificorrect.com/_ap-gecis (cihaz.py) cihaz paneli oturumundan (wcc) 60 sn'lik
tek kullanımlık belirteç üretir → /_ap-giris belirteci wca çerezine çevirir → Caddy her istekte forward_auth ile /ap-yetki'ye
sorar → /_ap-cikis çerezi siler. wca en çok 30 dk ve yalnızca altındaki wcc geçerli kaldıkça (parola, lisans, üyelik, aynı
cihaz ve tünel adresi) geçerlidir; tünel adresi her istekte wcc'nin müşterisinden bulunur: bir müşterinin wca'sı başka
müşterinin cihazına gidemez."""
import time
import urllib.parse

import guvenlik
import web
from web import e

CEREZ = "wca"
SURE = 1800  # en çok 30 dk (yan odayla aynı); boşta ve toplam süre aynı: mutlak
GIRIS = "https://ap.wificorrect.com/_ap-giris?t="
CIHAZ_PANELI = "https://cihaz.wificorrect.com"
YENIDEN = "Cihaz panelinde Ayarlar → AP kurulumu → AP arayüzüne git ile yeniden açın."
# Fetch Metadata: /_ap-gecis yalnızca cihaz.wificorrect.com sayfasından (ya da elle yazılan adresle), /_ap-giris ondan
# yönlenen gezinmeyle (aynı site). AP arayüzü panel ve cihazla aynı "site"da çalışır: kötü niyetli bir AP'nin betiği
# çerçeve/fetch/pencereyle tarayıcıdaki başka müşterinin cihaz paneli oturumundan (wcc) AP oturumu aldıramasın.
GECIS_KAYNAKLARI = ("same-origin", "none")
GIRIS_KAYNAKLARI = ("same-origin", "same-site", "none")
ELLE = "AP arayüzü yalnızca cihaz panelinden açılabilir. " + YENIDEN


def ust_duzey_gezinme(basliklar, kaynaklar):
    """Tarayıcı Sec-Fetch-* gönderdiyse istek `kaynaklar`dan gelen üst düzey bir gezinme olmalı (çerçeve, fetch, resim
    değil). Göndermeyen eski tarayıcı geçer (bu başlıkları sayfa değil tarayıcı yazar; betik değiştiremez)."""
    def b(ad):
        v = basliklar.get(ad)
        return v.strip().lower() if v is not None else None
    site, mod, hedef = b("Sec-Fetch-Site"), b("Sec-Fetch-Mode"), b("Sec-Fetch-Dest")
    return site in (None, *kaynaklar) and mod in (None, "navigate") and hedef in (None, "document")


def getirme_ozeti(basliklar):
    """Günlük için Sec-Fetch-* değerleri (kısaltılmış, kaçışlı: istemcinin yazdığı değer satır bozamaz)."""
    return " ".join(f"{ad[10:].lower()}={str(basliklar.get(ad))[:20]!r}"
                    for ad in ("Sec-Fetch-Site", "Sec-Fetch-Mode", "Sec-Fetch-Dest"))


def _cerez(token):
    """Yalnızca ap.wificorrect.com (Domain yok); Lax: cihaz.wificorrect.com'dan gelen yönlendirmede de gönderilsin."""
    if token is None:
        return f"{CEREZ}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
    return f"{CEREZ}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={SURE}"


def sayfa(durum, baslik, metin, cerez=None):
    """Sade Türkçe sayfa ve cihaz paneline bağlantı (JavaScript yok)."""
    govde = web._cerceve(baslik, web.kart(f'<h1>{e(baslik)}</h1><p>{e(metin)}</p>'
                                          f'<p><a href="{CIHAZ_PANELI}/">Cihaz paneline git</a></p>'))
    b = {"Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store"}
    if cerez:
        b["Set-Cookie"] = cerez
    return durum, b, govde.encode("utf-8")


def _ayni(o, b):
    """Cihaz paneli oturumu (cihaz.Cihaz.oturum) belirteçteki / AP oturumundaki müşteri, cihaz ve tünel adresiyle aynı mı."""
    return o is not None and (o["numara"], o["cihaz"], o["adres"]) == (b["numara"], b["cihaz"], b["adres"])


class Ap:
    def __init__(self, cihaz_app, saat=time.time):
        """cihaz_app: cihaz.Cihaz (cihaz paneli oturumları ve AP geçiş belirteçleri onda)."""
        self.cihaz_app, self.saat = cihaz_app, saat
        self.oturumlar = guvenlik.Oturumlar(bosta=SURE, en_cok=SURE)

    def _gecerli(self, token):
        """Altındaki cihaz paneli oturumu; o düşmüşse ya da başka cihaza/adrese dönmüşse AP oturumu da düşer."""
        ot = self.oturumlar.get(token, self.saat())
        if not ot:
            return None
        o = self.cihaz_app.oturum(ot["wcc"])
        if not _ayni(o, {"numara": ot["user"], "cihaz": ot["cihaz"], "adres": ot["adres"]}):
            self.oturumlar.drop(token)
            return None
        return o

    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        token = web.cerez_degeri(basliklar.get("Cookie"), CEREZ)
        if yol == "/ap-yetki":
            o = self._gecerli(token)
            if o is None:
                return sayfa(403, "AP erişimi kapalı",
                             "AP arayüzü oturumu yok ya da süresi doldu (en çok 30 dakika). " + YENIDEN,
                             _cerez(None) if token else None)
            # X-WFC-Kullanici her zaman müşteri numarası (yönetici geçişinde de): cihaz kendi bağındaki numarayla karşılaştırır
            return 200, {"X-WFC-Cihaz": o["adres"], "X-WFC-Kullanici": o["numara"], "X-WFC-AP": "1",
                         "Cache-Control": "no-store"}, b""
        if yol == "/_ap-giris":
            t = (urllib.parse.parse_qs(sorgu).get("t") or [""])[0]
            if not ust_duzey_gezinme(basliklar, GIRIS_KAYNAKLARI):  # belirteç harcanmadan
                if t:  # belirteçli istek: saldırı ya da beklenmedik tarayıcı davranışı (teşhis için)
                    web.log(f"AP_GIRIS_RED {getirme_ozeti(basliklar)} ip={ip}")
                return sayfa(403, "AP arayüzü açılamadı", ELLE)
            simdi = self.saat()
            b = self.cihaz_app.ap_belirtecleri.tuket(t, simdi)
            o = self.cihaz_app.oturum(b["wcc"]) if b else None
            if b is None or not _ayni(o, b):
                return sayfa(403, "Bağlantı geçersiz", "AP arayüzü bağlantısı geçersiz ya da süresi dolmuş (bağlantı bir "
                                                       "dakika geçerlidir ve bir kez kullanılır). " + YENIDEN)
            self.oturumlar.drop(token)  # bu tarayıcının eski AP oturumu
            yeni = self.oturumlar.create(o["numara"], o["rol"], simdi,
                                         ek={"wcc": b["wcc"], "cihaz": o["cihaz"], "adres": o["adres"]})
            return 303, {"Location": "/", "Set-Cookie": _cerez(yeni), "Cache-Control": "no-store"}, b""
        if yol == "/_ap-cikis":
            self.oturumlar.drop(token)
            # yalnızca cihaz.wificorrect.com'da bir yol; Location ASCII kalsın (http.server başlığı latin-1 yazar)
            donus = urllib.parse.quote(web.guvenli_donus((urllib.parse.parse_qs(sorgu).get("donus") or ["/"])[0]),
                                       safe="/?=&-._~%")
            return 303, {"Location": CIHAZ_PANELI + donus, "Set-Cookie": _cerez(None), "Cache-Control": "no-store"}, b""
        return 404, {"Content-Type": "text/plain; charset=utf-8"}, "Bulunamadı".encode("utf-8")
