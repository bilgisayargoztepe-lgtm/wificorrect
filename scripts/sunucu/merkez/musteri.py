"""Müşteri paneli (spec §6): panel.wificorrect.com — müşteri numarası + parola; kayıtlar (gün/kişi/arama) ve parola.
Üyeliği bitmiş müşteri de girer (yalnızca okuma)."""

import time
import urllib.parse

import guvenlik
import web
from web import e, kart, yanit_html, yonlendir


def parola_html(ot, hata="", tamam=""):
    m = f'<p class="hata">{e(hata)}</p>' if hata else (f'<p class="tamam">{e(tamam)}</p>' if tamam else "")
    return kart(
        '<h1>Parola değiştir</h1><p class="alt">Yeni parola en az 10 karakter olmalı.</p>'
        f'<form method="post" action="/parola"><input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
        '<label for="m">Mevcut parola</label><input id="m" name="mevcut" type="password" autocomplete="current-password" required>'
        '<label for="y">Yeni parola</label><input id="y" name="yeni" type="password" autocomplete="new-password" minlength="10" required>'
        '<label for="t">Yeni parola (tekrar)</label><input id="t" name="tekrar" type="password" autocomplete="new-password" minlength="10" required>'
        f'{m}<button>Değiştir</button></form><p><a href="/">← Özete dön</a></p>')


class Musteri(web.Taban):
    rol = "musteri"
    cerez = "wc_musteri"
    kullanici_etiketi = "Müşteri numarası"
    giris_basligi = "Müşteri girişi"
    giris_aciklama = "Size verilen müşteri numarası ve parolayla girin."

    def __init__(self, veri, kayitlar, saat=time.time, gecis=None):
        super().__init__(veri, kayitlar, saat)
        self.gecis = gecis or guvenlik.GecisBelirtecleri()

    def cihaz_adresi(self, numara):
        """Bağlı, tünel adresi olan, üyeliği ve lisansı açık cihazın tünel IP'si; değilse None."""
        c, m = self.veri.bagli_cihaz(numara), self.veri.musteri(numara)
        if c is None or m is None or c["durum"] != "bagli" or not c["tunel_ip"]:
            return None
        if m["uyelik"] != "aktif" or self.veri.lisans_durumu(m)[0] != "aktif":
            return None
        return c["tunel_ip"]

    def giris_sonrasi(self, kimlik):
        return "/cihaz" if self.cihaz_adresi(int(kimlik)) else "/"

    def dogrula(self, kul, pw):
        if not guvenlik.NUMARA_RE.fullmatch(kul):
            guvenlik.bos_dogrulama(pw)
            return None
        m = self.veri.parola_dogrula(int(kul), pw)
        return str(m["numara"]) if m else None

    def hesap_surumu(self, kimlik):
        m = self.veri.musteri(int(kimlik))
        return m["ozet"] if m is not None else None

    def girdi(self, kimlik, ip):
        self.veri.giris_kaydet(int(kimlik))
        self.veri.hareket(kimlik, ip, "GIRIS", int(kimlik), "müşteri paneli")

    def kim(self, ot):
        return ot["user"]

    def hareket_musterisi(self, ot, yol):
        return int(ot["user"])

    def menu(self, ot):
        return '<a href="/cihaz">Cihaz paneli</a><a href="/">Yedek arşiv</a><a href="/parola">Parola</a>'

    def sayfalar(self, ot, token, yontem, yol, sorgu, form, ip):
        n = int(ot["user"])
        if (yontem == "GET" and (yol == "/" or yol.startswith("/gun/") or yol == "/ara")) or (yontem, yol) == ("POST", "/ara"):
            c = self.veri.bagli_cihaz(n)
            baslik = c["isletme_adi"] if c is not None and c["isletme_adi"] else f"Müşteri {n}"
            return self.kayit_sayfasi(ot, n, yol, sorgu, "", ip, baslik, yontem, form)
        if yol == "/cihaz" and yontem == "GET":
            if self.cihaz_adresi(n) and "yok" not in urllib.parse.parse_qs(sorgu):  # yok=1: Caddy cihaza ulaşamadı → döngü olmasın
                ozet = self.veri.musteri(n)["ozet"]
                return yonlendir("https://cihaz.wificorrect.com/_giris?t=" + self.gecis.uret(ot["user"], self.saat(), ozet))
            return yanit_html(self.sayfa("Cihaz paneli", kart(
                '<h1>Cihazınıza şu an ulaşılamıyor</h1><p>Cihaz kapalı, internetsiz ya da lisansı kapalı olabilir. '
                'Yedek arşiviniz aşağıdadır.</p><p><a href="/">Yedek arşiv</a></p>'), ot))
        if yol == "/parola" and yontem == "GET":
            return yanit_html(self.sayfa("Parola", parola_html(ot), ot))
        if yol == "/parola" and yontem == "POST":
            if self.veri.parola_dogrula(n, form.get("mevcut", "")) is None:
                return yanit_html(self.sayfa("Parola", parola_html(ot, hata="Mevcut parola yanlış."), ot), 400)
            if form.get("yeni", "") != form.get("tekrar", ""):
                return yanit_html(self.sayfa("Parola", parola_html(ot, hata="Yeni parolalar aynı değil."), ot), 400)
            try:
                self.veri.parola_koy(n, form.get("yeni", ""))
            except ValueError as h:
                return yanit_html(self.sayfa("Parola", parola_html(ot, hata=str(h)), ot), 400)
            self.oturumlar.drop_user(ot["user"])
            yeni = self.oturum_ac(ot["user"], self.saat())
            self.veri.hareket(ot["user"], ip, "PAROLA_DEGISTI", n, "müşteri paneli")
            tamam = "Parolanız değiştirildi. Cihazınızda ertesi sabah 06:00'dan sonra geçerli olur."
            return yanit_html(self.sayfa("Parola", parola_html(self.oturumlar.get(yeni, self.saat()), tamam=tamam), ot),
                              cerez=(self.cerez, yeni))
        return None
