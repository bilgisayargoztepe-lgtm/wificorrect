"""Müşteri paneli (spec §6): panel.wificorrect.com — müşteri numarası + parola; kayıtlar (gün/kişi/arama) ve parola.
Üyeliği bitmiş müşteri de girer (yalnızca okuma)."""

import time
import urllib.parse

import common
import guvenlik
import raporlar as raporlar_modulu
import web
from web import e, kart, yanit_html, yonlendir

HAZIRLIK_SN = 30  # cihaz bağlandıktan sonra tünel kalkana dek "panel hazırlanıyor" (sonra cihaz paneline)
YENILE_SN = 5
BULUNAMADI = ("Cihazınız bulunamadı. Cihazın modeme takılı ve açık olduğundan, bu sayfayı cihazla aynı internetten "
              "(işletmenin Wi-Fi'ı ya da kablosu; mobil veri değil) açtığınızdan emin olun.")


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

    def __init__(self, veri, kayitlar, saat=time.time, gecis=None, raporlar=None):
        super().__init__(veri, kayitlar, saat, raporlar or raporlar_modulu.Raporlar())
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
        """Cihazı çalışıyorsa cihaz paneli; bağlı cihazı yoksa (üyelik aktif) kolay bağlama; değilse yedek arşiv."""
        n = int(kimlik)
        if self.cihaz_adresi(n):
            return "/cihaz"
        m = self.veri.musteri(n)
        if self.veri.bagli_cihaz(n) is None and m is not None and m["uyelik"] == "aktif":
            return "/baglan"
        return "/"

    # --- kolay bağlama (spec 2026-10-16) ---
    def _bekle_sayfasi(self, ot, baslik, metin):
        return yanit_html(self.sayfa(baslik, kart(f'<h1>{e(baslik)}</h1><p>{e(metin)}</p>'), ot, yenile=YENILE_SN))

    def baglan(self, ot, n, yontem, form, ip):
        """Müşterinin bağlı cihazı yoksa aynı internet adresinden bekleyen cihazı bulur: tek → kendiliğinden ayırır,
        çok → liste (müşteri seçer), hiç → nasıl yapılacağı. Ayrılan cihaz sonraki bildiriminde bağlanır."""
        if self.veri.bagli_cihaz(n) is not None:
            r = self.veri.bekleyen_musterinin(n)
            if r and r["atama"] == "bagli" and self.saat() - r["baglandi"] < HAZIRLIK_SN:
                return self._bekle_sayfasi(ot, "Cihazınız bağlandı", "Cihaz paneliniz hazırlanıyor, bu sayfa kendini yeniler.")
            return yonlendir("/cihaz")
        m = self.veri.musteri(n)
        if m["uyelik"] != "aktif":
            return yanit_html(self.mesaj(ot, "Cihaz bağlanamıyor", "Üyeliğiniz sona ermiş. Hizmet sağlayıcınıza başvurun."))
        adaylar = self.veri.bekleyen_adaylar(ip) if ip and ip != "yerel" else []
        if yontem == "POST":
            k = form.get("kimlik", "")
            if not any(a["kimlik"] == k for a in adaylar) or not self.veri.bekleyen_ayir(k, n, ip):
                return yanit_html(self.mesaj(ot, "Cihaz seçilemedi", "Bu cihaz artık seçilemiyor. Sayfayı yenileyip tekrar deneyin."), 400)
            self.veri.hareket(str(n), ip, "CIHAZ_OTOMATIK_AYRILDI", n, f"kimlik={k[:8]} secim=liste")
            return yonlendir("/baglan")
        if self.veri.bekleyen_musterinin(n):
            return self._bekle_sayfasi(ot, "Cihazınız bağlanıyor",
                                       "Cihazınız bulundu ve hesabınıza bağlanıyor. Bu birkaç dakika sürebilir; bu sayfa kendini yeniler.")
        if len(adaylar) == 1 and self.veri.bekleyen_ayir(adaylar[0]["kimlik"], n, ip):
            self.veri.hareket(str(n), ip, "CIHAZ_OTOMATIK_AYRILDI", n, f"kimlik={adaylar[0]['kimlik'][:8]} secim=tek")
            return self._bekle_sayfasi(ot, "Cihazınız bağlanıyor",
                                       "Cihazınız bulundu ve hesabınıza bağlanıyor. Bu birkaç dakika sürebilir; bu sayfa kendini yeniler.")
        if not adaylar:
            return yanit_html(self.sayfa("Cihaz bağlama", kart(
                f'<h1>Cihazınız bulunamadı</h1><p>{e(BULUNAMADI)}</p><p><a href="/baglan">Yeniden dene</a></p>'), ot))
        satirlar = "".join(
            f'<tr><td>{e(common.tarih_goster(common.now_iso(a["ilk"])))}</td><td>{e(a["wan_mac"][-5:].upper() or "—")}</td>'
            f'<td>{e(a["surum"] or "—")}</td><td><form method="post" action="/baglan">'
            f'<input type="hidden" name="csrf" value="{e(ot["csrf"])}"><input type="hidden" name="kimlik" value="{e(a["kimlik"])}">'
            '<button class="ikincil">Bu cihaz</button></form></td></tr>' for a in adaylar)
        return yanit_html(self.sayfa("Cihaz bağlama", kart(
            '<h1>Birden çok cihaz bulundu</h1><p>Bu internet bağlantısında bağlanmayı bekleyen birden çok cihaz var. '
            'Sizinkini seçin: cihazın altındaki etiketteki MAC adresinin son dört hanesine bakın.</p>'
            '<div class="kaydir"><table><tr><th>İlk görülme</th><th>MAC (son 4)</th><th>Sürüm</th><th></th></tr>'
            f'{satirlar}</table></div><p><a href="/baglan">Listeyi yenile</a></p>'), ot))

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
        return '<a href="/cihaz">Cihaz paneli</a><a href="/">Yedek arşiv</a><a href="/raporlar">Raporlar</a><a href="/parola">Parola</a>'

    def sayfalar(self, ot, token, yontem, yol, sorgu, form, ip):
        n = int(ot["user"])
        if (yontem == "GET" and (yol == "/" or yol.startswith("/gun/") or yol == "/ara")) or (yontem, yol) == ("POST", "/ara"):
            c = self.veri.bagli_cihaz(n)
            baslik = c["isletme_adi"] if c is not None and c["isletme_adi"] else f"Müşteri {n}"
            return self.kayit_sayfasi(ot, n, yol, sorgu, "", ip, baslik, yontem, form)
        if yontem == "GET" and (yol == "/raporlar" or yol.startswith("/rapor/")):
            return raporlar_modulu.istek(self, ot, n, yol, "")
        if yol == "/baglan" and yontem in ("GET", "POST"):
            return self.baglan(ot, n, yontem, form, ip)
        if yol == "/cihaz" and yontem == "GET":
            if self.veri.bagli_cihaz(n) is None and self.veri.musteri(n)["uyelik"] == "aktif":
                return yonlendir("/baglan")
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
