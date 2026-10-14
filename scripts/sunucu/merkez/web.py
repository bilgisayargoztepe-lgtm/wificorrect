"""Yönetim merkezi web tabanı: CSS, sayfa çerçevesi, çerezler, giriş/çıkış/CSRF/gerekçe (spec §5–§6, §10).
Uygulamalar (yönetim, müşteri) Taban'dan türer ve yalnızca kendi sayfalarını yazar."""
import hmac
import sys
import time
import urllib.parse

import detay
import guvenlik

GEREKCE_SN = 1800
GEREKCE_ONERILERI = ("Emniyet / savcılık talebi", "Müşteri şikâyeti", "Müşterinin kendi talebi", "Teknik sorun incelemesi")
CSS = """*{box-sizing:border-box}body{margin:0;font:16px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;background:#f5f7fa;color:#1d2330}
header{background:#fff;border-bottom:1px solid #e3e7ee;padding:12px 16px;display:flex;align-items:center;justify-content:space-between;gap:12px;flex-wrap:wrap}
.marka{font-weight:700;color:#0a6ebd;font-size:18px;text-decoration:none}nav{display:flex;gap:12px;align-items:center;flex-wrap:wrap}nav form{margin:0}
main{max-width:640px;margin:0 auto;padding:16px}main.genis{max-width:1100px}.kart{background:#fff;border:1px solid #e3e7ee;border-radius:12px;padding:16px;margin-bottom:16px}
h1{font-size:20px;margin:0 0 4px}.alt{color:#5b6475;font-size:14px;margin:0}
label{display:block;font-size:14px;margin:12px 0 4px}input,select{width:100%;padding:10px 12px;border:1px solid #c9d0db;border-radius:8px;font:inherit}
input[type=checkbox]{width:auto}button{background:#0a6ebd;color:#fff;border:0;border-radius:999px;padding:10px 18px;font:inherit;cursor:pointer;margin-top:16px}
button.ikincil{background:#fff;color:#0a6ebd;border:1px solid #0a6ebd;margin:0;padding:4px 14px}button.tehlike{background:#a4262c}
.hata{background:#fdecec;color:#a4262c;padding:10px 12px;border-radius:8px;margin:12px 0 0}.kotu{color:#a4262c}.uyarisi{color:#8a5a00;font-weight:600}
.tamam{background:#e7f6ec;color:#1e6b34;padding:10px 12px;border-radius:8px;margin:12px 0 0}
.sir{font:600 22px ui-monospace,Consolas,monospace;letter-spacing:1px;user-select:all}
table{width:100%;border-collapse:collapse;margin-bottom:8px}td,th{padding:6px 4px;border-bottom:1px solid #eef1f5;font-size:14px;text-align:left;vertical-align:top}
td.cubuk{width:100%}td.cubuk span{display:block;height:10px;background:#0a6ebd;border-radius:5px;min-width:2px}
td.n{text-align:right;font-weight:600;min-width:3ch}.bos{color:#9aa3b2}a{color:#0a6ebd}h2{font-size:17px;margin:0 0 8px}h3{font-size:15px;margin:12px 0 4px}
details{border:1px solid #e3e7ee;border-radius:10px;margin:8px 0;background:#fff}summary{cursor:pointer;padding:10px 12px}details>div{padding:0 12px 12px}
.kaydir{overflow-x:auto}table.detay th{font-size:13px;color:#5b6475}ul.gunler{list-style:none;margin:0;padding:0 12px 12px}ul.gunler li{padding:4px 0}
form.ara{display:flex;gap:8px;flex-wrap:wrap}form.ara input{flex:1;min-width:10em}form.ara button{margin:0}
form.ara select{width:auto}form.ara input.uyruk{flex:0 0 5.5em;min-width:0}.iyi{color:#1e6b34;font-weight:600}
td.cubuk span{-webkit-print-color-adjust:exact;print-color-adjust:exact}
@media print{header,nav,.yazdirma{display:none}body{background:#fff}main,main.genis{max-width:none;padding:0}
.kart{border:0;padding:0 0 8px;break-inside:avoid}.kaydir{overflow:visible}@page{size:A4;margin:12mm}}"""
e = detay.e
SINIF_GENIS = ' class="genis"'


def log(msg):
    print(msg, file=sys.stderr, flush=True)  # journal; fail2ban GIRIS_HATALI satırlarını okur


def cerez_degeri(baslik, ad):
    """Cookie başlığından `ad` çerezini okur (SimpleCookie ilk bozuk çerezde durduğu için elle)."""
    for parca in (baslik or "").split(";"):
        k, _, v = parca.strip().partition("=")
        if k == ad:
            return v
    return None


def _cerez_basligi(cerez):
    ad, token = cerez
    if token is None:
        return f"{ad}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
    return f"{ad}={token}; HttpOnly; Secure; SameSite=Strict; Path=/"


def yanit_html(govde, durum=200, cerez=None):
    b = {"Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store"}
    if cerez:
        b["Set-Cookie"] = _cerez_basligi(cerez)
    return durum, b, govde.encode("utf-8")


def yonlendir(yer, cerez=None):
    b = {"Location": yer, "Cache-Control": "no-store"}
    if cerez:
        b["Set-Cookie"] = _cerez_basligi(cerez)
    return 303, b, b""


def guvenli_donus(p):
    """Yalnızca bu sitedeki bir yol. Tarayıcılar sekme/satır sonunu silip "/\t/x"i "//x"e çevirir: boşluk ve denetim
    karakteri, ters bölü ve "//" reddedilir."""
    if not isinstance(p, str) or not p.startswith("/") or p.startswith("//"):
        return "/"
    return "/" if any(c.isspace() or ord(c) < 0x20 or ord(c) == 0x7f or c == "\\" for c in p) else p


def kart(icerik):
    return f'<div class="kart">{icerik}</div>'


def bilgi_tablosu(satirlar):
    return "<table>" + "".join(f"<tr><th>{e(k)}</th><td>{v}</td></tr>" for k, v in satirlar) + "</table>"


def _cerceve(baslik, icerik, nav="", genis=False):
    return ('<!doctype html><html lang="tr"><head><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">'
            f'<title>{e(baslik)} · WifiCorrect</title><style>{CSS}</style></head><body>'
            f'<header><a class="marka" href="/">WifiCorrect</a>{nav}</header>'
            f'<main{SINIF_GENIS if genis else ""}>{icerik}</main></body></html>')


class Taban:
    rol = ""
    cerez = ""
    kullanici_etiketi = "Kullanıcı adı"
    giris_basligi = "Giriş"
    giris_aciklama = ""

    def __init__(self, veri, kayitlar, saat=time.time, raporlar=None):
        self.veri, self.kayitlar, self.saat = veri, kayitlar, saat
        self.raporlar = raporlar  # raporlar.Raporlar (haftalık/aylık rapor JSON'ları); None: rapor sayfaları yok
        self.oturumlar, self.kilit = guvenlik.Oturumlar(), guvenlik.GirisKilidi()

    # --- alt sınıfın yazdıkları ---
    def dogrula(self, kul, pw):
        raise NotImplementedError

    def hesap_surumu(self, kimlik):
        """Hesabın parola özeti; hesap yoksa None. Oturumdakiyle aynı değilse oturum düşer."""
        raise NotImplementedError

    def oturum_ac(self, kimlik, simdi):
        return self.oturumlar.create(kimlik, self.rol, simdi, self.hesap_surumu(kimlik))

    def girdi(self, kimlik, ip):
        raise NotImplementedError

    def kim(self, ot):
        raise NotImplementedError

    def hareket_musterisi(self, ot, yol):
        raise NotImplementedError

    def menu(self, ot):
        return ""

    def giris_sonrasi(self, kimlik):
        return "/"

    def sayfalar(self, ot, token, yontem, yol, sorgu, form, ip):
        return None

    # --- ortak ---
    def sayfa(self, baslik, icerik, ot, genis=False):
        nav = ""
        if ot:
            nav = (f'<nav>{self.menu(ot)}<form method="post" action="/cikis">'
                   f'<input type="hidden" name="csrf" value="{e(ot["csrf"])}"><button class="ikincil">Çıkış</button></form></nav>')
        return _cerceve(baslik, icerik, nav, genis)

    def mesaj(self, ot, baslik, metin):
        return self.sayfa(baslik, kart(f'<h1>{e(baslik)}</h1><p>{e(metin)}</p><p><a href="/">Ana sayfa</a></p>'), ot)

    def _giris_html(self, hata=""):
        h = f'<p class="hata">{e(hata)}</p>' if hata else ""
        return self.sayfa("Giriş", kart(
            f'<h1>{e(self.giris_basligi)}</h1><p class="alt">{e(self.giris_aciklama)}</p>'
            '<form method="post" action="/giris">'
            f'<label for="k">{e(self.kullanici_etiketi)}</label>'
            '<input id="k" name="kullanici" autocomplete="username" autocapitalize="none" spellcheck="false" required>'
            '<label for="s">Parola</label><input id="s" name="sifre" type="password" autocomplete="current-password" required>'
            f'{h}<button>Giriş yap</button></form>'), None)

    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        simdi = self.saat()
        token = cerez_degeri(basliklar.get("Cookie"), self.cerez)
        ot = self.oturumlar.get(token, simdi)
        if ot and self.hesap_surumu(ot["user"]) != ot.get("surum"):  # hesap silinmiş ya da parolası değişmiş
            self.oturumlar.drop(token)
            ot = None
        form = dict(urllib.parse.parse_qsl(govde)) if yontem == "POST" else {}
        if yol == "/giris":
            if yontem == "POST":
                return self._giris(form, ip, simdi)
            return yonlendir("/") if ot else yanit_html(self._giris_html())
        if not ot:
            return yonlendir("/giris")
        if yontem == "POST" and not hmac.compare_digest(form.get("csrf", "").encode(), ot["csrf"].encode()):
            return yanit_html(self.mesaj(ot, "Geçersiz istek", "Sayfayı yenileyip tekrar deneyin."), 403)
        if (yontem, yol) == ("POST", "/cikis"):
            self.oturumlar.drop(token)
            return yonlendir("/giris", (self.cerez, None))
        if yol == "/gerekce":
            return self._gerekce(ot, token, yontem, sorgu, form, ip, simdi)
        r = self.sayfalar(ot, token, yontem, yol, sorgu, form, ip)
        return r or yanit_html(self.mesaj(ot, "Bulunamadı", "Böyle bir sayfa yok."), 404)

    def _giris(self, form, ip, simdi):
        kul = form.get("kullanici", "").strip().lower()
        if not self.kilit.attempt(ip, kul, simdi):
            log(f"GIRIS_KILITLI alan={self.rol} kullanici={kul!r} ip={ip}")
            return yanit_html(self._giris_html("Çok fazla deneme. 15 dakika sonra tekrar deneyin."), 429)
        kimlik = self.dogrula(kul, form.get("sifre", ""))
        if kimlik is None:
            log(f"GIRIS_HATALI alan={self.rol} kullanici={kul!r} ip={ip}")
            return yanit_html(self._giris_html(f"{self.kullanici_etiketi} veya parola hatalı."), 401)
        self.kilit.succeed(ip, kul, simdi)
        self.girdi(kimlik, ip)
        return yonlendir(self.giris_sonrasi(kimlik), (self.cerez, self.oturum_ac(kimlik, simdi)))

    def _gerekce(self, ot, token, yontem, sorgu, form, ip, simdi):
        if yontem == "GET":
            donus = guvenli_donus((urllib.parse.parse_qs(sorgu).get("donus") or ["/"])[0])
            return yanit_html(self._gerekce_html(ot, donus))
        metin, donus = " ".join(form.get("gerekce", "").split()), guvenli_donus(form.get("donus", "/"))
        if not 5 <= len(metin) <= 200:
            return yanit_html(self._gerekce_html(ot, donus, "Gerekçe 5–200 karakter olmalı."), 400)
        self.oturumlar.set_gerekce(token, metin, simdi + GEREKCE_SN)
        self.veri.hareket(self.kim(ot), ip, "GEREKCE", self.hareket_musterisi(ot, donus), f"{donus} · {metin}")
        return yonlendir(donus)

    def _gerekce_html(self, ot, donus, hata=""):
        secenekler = "".join(f'<option value="{e(x)}">' for x in GEREKCE_ONERILERI)
        h = f'<p class="hata">{e(hata)}</p>' if hata else ""
        return self.sayfa("Gerekçe", kart(
            '<h1>Gerekçe</h1><p>Müşterilerin kişisel verisine bakmak için gerekçe yazın.</p>'
            f'<p class="alt">Gerekçe {GEREKCE_SN // 60} dakika geçerlidir; bu süredeki her görüntüleme gerekçeyle kaydedilir.</p>'
            f'<form method="post" action="/gerekce"><input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
            f'<input type="hidden" name="donus" value="{e(donus)}">'
            '<label for="g">Gerekçe</label><input id="g" name="gerekce" list="oneriler" minlength="5" maxlength="200" required>'
            f'<datalist id="oneriler">{secenekler}</datalist>{h}<button>Devam</button></form>'), ot)

    def gerekce_iste(self, ot, tam_yol, musteri, ip):
        """Geçerli gerekçe yoksa gerekçe sayfasına yönlendirir; varsa görüntülemeyi kaydeder ve None döner."""
        g = ot.get("gerekce")
        if not g or g[1] <= self.saat():
            return yonlendir("/gerekce?donus=" + urllib.parse.quote(tam_yol, safe=""))
        self.veri.hareket(self.kim(ot), ip, "GORUNTULEME", musteri, f"{tam_yol} · {g[0]}")
        return None

    def kayit_sayfasi(self, ot, numara, alt, sorgu, onek, ip, baslik, yontem="GET", form=None):
        """alt: '/', '/gun/<gün>', '/ara'. Kişi verisi açan sayfalar gerekçe ister (sayılar ve boş arama formu istemez).
        Arama yalnızca POST'la yapılır (telefon/kimlik adrese, tarayıcı geçmişine ve hareket kaydına girmez)."""
        arsivler = self.veri.arsivler(numara)
        if yontem == "POST" and alt != "/ara":
            return None
        if alt == "/":
            return yanit_html(self.sayfa(baslik, self.kayitlar.ozet_html(baslik, arsivler, self.saat(), onek, ot["csrf"]), ot))
        if alt.startswith("/gun/") or yontem == "POST":
            r = self.gerekce_iste(ot, onek + alt + ("?" + sorgu if sorgu and yontem == "GET" else ""), numara, ip)
            if r:
                return r
        if alt.startswith("/gun/"):
            gun = alt[len("/gun/"):]
            h = self.kayitlar.gun_html(arsivler, gun, onek)
            if h is None:
                return yanit_html(self.mesaj(ot, "Bulunamadı", "Bu gün için kayıt yok."), 404)
            return yanit_html(self.sayfa(detay.tarih(gun), h, ot))
        if alt == "/ara":
            h = self.kayitlar.ara_html(arsivler, form if yontem == "POST" else None, onek, ot["csrf"])
            return yanit_html(self.sayfa("Arama", h, ot))
        return None
