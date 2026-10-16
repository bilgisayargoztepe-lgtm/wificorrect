"""ap.wificorrect.com (spec 2026-10-16 "AP arayüzüne panelden tek tıkla erişim", Mimari 2–3): müşterinin taktığı AP'nin ya
da modemin kendi web arayüzü cihaz üzerinden (tünel → cihaz → yan oda ya da misafir ağı) bu alan adının kökünde açılır;
merkez yalnızca kimliği doğrular. Akış: cihaz.wificorrect.com/_ap-gecis (cihaz.py) cihaz paneli oturumundan (wcc) 60 sn'lik
tek kullanımlık belirteç üretir → /_ap-giris belirteci wca çerezine çevirir → Caddy her istekte forward_auth ile /ap-yetki'ye
sorar → /_ap-cikis çerezi siler. wca en çok 30 dk ve yalnızca altındaki wcc geçerli kaldıkça (parola, lisans, üyelik, aynı
cihaz ve tünel adresi) geçerlidir; tünel adresi her istekte wcc'nin müşterisinden bulunur: bir müşterinin wca'sı başka
müşterinin cihazına gidemez.
Bütün müşterilerin AP'leri tek kökte açıldığı için (final review 2026-10-16): Service Worker betiği hiç aktarılmaz (Caddy ap
bloğu ve cihaz 403); /_ap-giris (yeni AP oturumu) kökün depolamasını (Service Worker kayıtları dahil) siler ve önceki
AP'lerin bu kökte kalan çerezlerini sona erdirir, /_ap-cikis çerezler dahil her şeyi siler; belirteç yalnızca cihaz
alanından yönlenen üst düzey gezinmeyle ve /_ap-gecis'i isteyen adresin ağından (IPv4 /24, IPv6 /64; re-review I-1)
kullanılır; wca SameSite=Strict; AP kipinde kullanıcı X-WFC-AP-Kullanici başlığındadır (0.4.15 öncesi cihaz bunu tanımaz,
isteği oturumsuz sayar; X-WFC-Kullanici'yi görse paneli bu kökte açardı)."""
import ipaddress
import re
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
# /_ap-giris'te same-origin = AP sayfasının kendisi gezindi (bir müşterinin AP'si kendi belirtecini tarayıcıya ekip başka
# müşterinin açık AP sekmesini kendine çevirirdi), none = elle yazılan adres (tek kullanımlık belirteç elle yazılmaz)
GIRIS_KAYNAKLARI = ("same-site",)
# /_ap-cikis her şeyi siler: başka sitenin (ya da çerçeve/resmin) tetiklediği çıkış oturum kapatma CSRF'i olmasın
CIKIS_KAYNAKLARI = ("same-origin", "same-site", "none")
ELLE = "AP arayüzü yalnızca cihaz panelinden açılabilir. " + YENIDEN
# /_ap-giris'e gelen adres /_ap-gecis'i isteyen adresle aynı ağda olmalı (re-review I-1): cihaz.* ve ap.* ayrı sertifikalı,
# ayrı bağlantılar; aynı ağdan başka çıkış adresi (taşıyıcı NAT havuzu, çift hatlı modem) geçer, tarayıcıdan sızdırılan
# belirteç başka ağdan kullanılamaz
AG_ONEKI_V4, AG_ONEKI_V6 = 24, 64
# her bağlantısı ayrı adresten çıkan istemci (iCloud Özel Geçiş, VPN): yeniden denemek aynı yerde takılır, sebebi söylenir
FARKLI_AG = ("Bağlantınız farklı bir internet adresinden geldi (ör. iCloud Özel Geçiş, VPN). Özel Geçiş/VPN'i kapatıp "
             "cihaz panelinden yeniden deneyin. iPhone'da: Ayarlar → adınız → iCloud → Özel Geçiş.")
# Yeni AP oturumu: kökün depolaması (localStorage, IndexedDB, Cache API, Service Worker kayıtları) silinir. "cookies" yok:
# kayıtlı alan adının (wificorrect.com) bütün çerezlerini — az önce yazılan wca'yı da — silerdi. "cache" yok (re-review m4):
# bu kökteki her cevap no-store (merkez ve cihaz), HTTP önbelleğinde bir şey yok; Chromium'da önbellek silme yönlendirmeyi
# gözle görülür geciktirir.
GIRIS_TEMIZLIK = '"storage"'
# Çıkış: çerezler de (AP'lerin çerezleri, tarayıcının sakladığı HTTP kimlik bilgileri). Kayıtlı alan adının bütün çerezleri
# silindiği için panel, yönetim ve cihaz paneli oturumları da kapanır.
CIKIS_TEMIZLIK = '"cache", "cookies", "storage"'
CEREZ_ADI = re.compile(r"[!#$%&'*+.^_`|~0-9A-Za-z-]{1,64}")  # RFC 6265 belirteci: Set-Cookie'ye eklenen ad başlık bozamaz
ESKI_CEREZ_EN_COK = 30
# Çerçeveli AP arayüzlerinde (ör. TP-Link userRpm frameset) sade sayfalar çerçevenin içinde görünsün ve bağlantıları üst
# pencereyi açsın (re-review m1: çerçevedeki gezinmeyi /_ap-cikis reddeder, cihaz paneli çerçevede açılmaz). Yalnızca aynı
# kökenden; merkezin diğer cevaplarında DENY kalır (merkez: cevabın verdiği güvenlik başlığı varsayılanın yerine geçer).
# Sayfalarda sır yok, aynı kökendeki AP betiği bu adreslere zaten gidebilir. CSP = merkez.GUVENLIK_BASLIKLARI + frame-ancestors.
CERCEVE = {"X-Frame-Options": "SAMEORIGIN",
           "Content-Security-Policy": "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; "
                                      "frame-ancestors 'self'"}


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
    """Yalnızca ap.wificorrect.com (Domain yok). Strict: giriş zinciri (cihaz.wificorrect.com → /_ap-gecis → /_ap-giris →
    /) baştan sona aynı site; başka siteden üst düzey GET gezinmesi de oturumlu gitmez (GET ile ayar değiştiren AP'ler)."""
    if token is None:
        return f"{CEREZ}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
    return f"{CEREZ}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SURE}"


def eski_cerezler(baslik):
    """/_ap-giris'e gelen, bu kökte önceki AP oturumlarından kalan (Path=/) çerezlerin adları (wca hariç, tekrarsız, en çok
    ESKI_CEREZ_EN_COK): yeni oturumda silinir — bir müşterinin AP'sinin çerezleri (ör. TP-Link'in betikle yazdığı
    Authorization = Basic kullanıcı:parola) sonraki müşterinin AP'sine gitmesin. Geçersiz ad atlanır."""
    adlar = []
    for parca in (baslik or "").split(";"):
        ad = parca.partition("=")[0].strip()
        if ad != CEREZ and CEREZ_ADI.fullmatch(ad) and ad not in adlar:
            adlar.append(ad)
    return adlar[:ESKI_CEREZ_EN_COK]


def sayfa(durum, baslik, metin, cerez=None, cikis=False):
    """Sade Türkçe sayfa ve cihaz paneline bağlantı (JavaScript yok). cikis: AP kökünde, temizleyerek çıkış bağlantısı.
    Bağlantılar üst pencerede açılır, sayfa aynı kökenden çerçevelenebilir (CERCEVE)."""
    cik = ('<p><a href="/_ap-cikis" target="_top">AP oturumunu kapat ve bu tarayıcıdaki AP verisini sil</a> (WifiCorrect '
           'panel ve yönetim oturumlarınız da kapanır)</p>') if cikis else ""
    govde = web._cerceve(baslik, web.kart(f'<h1>{e(baslik)}</h1><p>{e(metin)}</p>'
                                          f'<p><a href="{CIHAZ_PANELI}/" target="_top">Cihaz paneline git</a></p>{cik}'))
    b = {"Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store", **CERCEVE}
    if cerez:
        b["Set-Cookie"] = cerez
    return durum, b, govde.encode("utf-8")


def _ayni(o, b):
    """Cihaz paneli oturumu (cihaz.Cihaz.oturum) belirteçteki / AP oturumundaki müşteri, cihaz ve tünel adresiyle aynı mı."""
    return o is not None and (o["numara"], o["cihaz"], o["adres"]) == (b["numara"], b["cihaz"], b["adres"])


def _ag(ip):
    """Adresin ağı (IPv4 /24, IPv6 /64; IPv4'e eşlenmiş IPv6 yazımı IPv4 sayılır); adres değilse değerin kendisi."""
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return ip
    if a.version == 6 and a.ipv4_mapped is not None:
        a = a.ipv4_mapped
    if a.version == 4:
        return ipaddress.IPv4Network((int(a), AG_ONEKI_V4), strict=False)
    return ipaddress.IPv6Network((int(a), AG_ONEKI_V6), strict=False)  # tamsayıdan: kapsam (%eth0) sorun olmaz


def ayni_ag(x, y):
    """/_ap-gecis'i isteyen adres ile /_ap-giris'e gelen adres aynı ağda mı (AG_ONEKI_V4, AG_ONEKI_V6). IPv4 ile IPv6 hiçbir
    zaman; adres olmayan değer (ör. X-Forwarded-For'suz "yerel") yalnızca kendisiyle aynı."""
    return _ag(x) == _ag(y)


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
                             _cerez(None) if token else None, cikis=True)
            # kullanıcı her zaman müşteri numarası (yönetici geçişinde de): cihaz kendi bağındaki numarayla karşılaştırır.
            # Ayrı başlık: X-WFC-Kullanici olsaydı eski (0.4.15 öncesi) cihaz isteği oturumlu panel isteği sayardı
            return 200, {"X-WFC-Cihaz": o["adres"], "X-WFC-AP-Kullanici": o["numara"], "X-WFC-AP": "1",
                         "Cache-Control": "no-store"}, b""
        if yol == "/_ap-giris":
            t = (urllib.parse.parse_qs(sorgu).get("t") or [""])[0]
            if not ust_duzey_gezinme(basliklar, GIRIS_KAYNAKLARI):  # belirteç harcanmadan
                if t:  # belirteçli istek: saldırı ya da beklenmedik tarayıcı davranışı (teşhis için)
                    web.log(f"AP_GIRIS_RED {getirme_ozeti(basliklar)} ip={ip}")
                return sayfa(403, "AP arayüzü açılamadı", ELLE)
            simdi = self.saat()
            b = self.cihaz_app.ap_belirtecleri.tuket(t, simdi)
            if b is not None and not ayni_ag(b.get("ip"), ip):
                # belirteç /_ap-gecis'i isteyen adresin ağına bağlı: dışarı sızdırılan belirteç başka ağdan kullanılamaz
                # (yanar). Meşru kullanıcıysa bağlantıları farklı adreslerden çıkıyor: genel "geçersiz" değil, sebep ve çözüm
                web.log(f"AP_GIRIS_RED adres_farkli ip={ip} gecis_ip={b.get('ip')}")
                return sayfa(403, "Farklı internet adresi", FARKLI_AG)
            o = self.cihaz_app.oturum(b["wcc"]) if b else None
            if b is None or not _ayni(o, b):
                return sayfa(403, "Bağlantı geçersiz", "AP arayüzü bağlantısı geçersiz ya da süresi dolmuş (bağlantı bir "
                                                       "dakika geçerlidir ve bir kez kullanılır). " + YENIDEN)
            self.oturumlar.drop(token)  # bu tarayıcının eski AP oturumu
            yeni = self.oturumlar.create(o["numara"], o["rol"], simdi,
                                         ek={"wcc": b["wcc"], "cihaz": o["cihaz"], "adres": o["adres"]})
            sil = [f"{ad}=; Path=/; Max-Age=0; Secure" for ad in eski_cerezler(basliklar.get("Cookie"))]
            return 303, {"Location": "/", "Set-Cookie": [_cerez(yeni)] + sil, "Clear-Site-Data": GIRIS_TEMIZLIK,
                         "Cache-Control": "no-store"}, b""
        if yol == "/_ap-cikis":
            if not ust_duzey_gezinme(basliklar, CIKIS_KAYNAKLARI):
                return sayfa(403, "AP oturumu kapatılamadı", "Çıkış yalnızca AP arayüzünden ya da cihaz panelinden açılır.")
            self.oturumlar.drop(token)
            # yalnızca cihaz.wificorrect.com'da bir yol; Location ASCII kalsın (http.server başlığı latin-1 yazar)
            donus = urllib.parse.quote(web.guvenli_donus((urllib.parse.parse_qs(sorgu).get("donus") or ["/"])[0]),
                                       safe="/?=&-._~%")
            return 303, {"Location": CIHAZ_PANELI + donus, "Set-Cookie": _cerez(None), "Clear-Site-Data": CIKIS_TEMIZLIK,
                         "Cache-Control": "no-store"}, b""
        return 404, {"Content-Type": "text/plain; charset=utf-8"}, "Bulunamadı".encode("utf-8")
