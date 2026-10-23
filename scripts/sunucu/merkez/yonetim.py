"""Yönetim merkezi (spec §5): yonetim.wificorrect.com — müşteriler, cihazlar, parola sıfırlama, serbest bırakma,
üyelik, kayıtlar (gerekçeyle), hareketler, eski arşivler, yönetici hesabı."""
import datetime
import json
import re
import time
import urllib.parse

import cihaz as cihaz_modulu
import common
import detay
import durum as durum_modulu
import guvenlik
import kayitlar as kayitlar_modulu
import kuyruk
import listeler
import netgsm
import raporlar as raporlar_modulu
import surumler
import veri as veri_modulu
import web
import yonetici
from web import bilgi_tablosu, e, kart, yanit_html, yonlendir

NUMARA_RE = guvenlik.NUMARA_RE
CEVRIMICI_SN = 600
ESLESME_GECIKME_SN = 26 * 3600
GECIKME_GUN = 2
TEKRAR_SN = 600  # saglik.py: kapanan sorun bu kadar temiz kalmadan "düzeldi" denmez (SMS bekletilir)
IZLEME_BAYAT_SN = 900  # wc-saglik 5 dk'da bir; son turu bundan eskiyse izleme çalışmıyor olabilir


def isletme_turu(c):
    """Cihazın bildirdiği işletme türünün etiketi; cihaz yoksa ya da tür bildirilmemişse "—"."""
    return veri_modulu.ISLETME_TURLERI.get(c["isletme_turu"], "—") if c is not None else "—"


def _epoch(iso):
    try:
        return datetime.datetime.fromisoformat(iso).timestamp()
    except (TypeError, ValueError):
        return None


def yedek_gecikti(son_gun, simdi):
    """Yedek gecikti: son gelen gün yok ya da GECIKME_GUN günden eski (TR takvimi). Liste ve uyarı (saglik.py) ortak."""
    sinir = (datetime.datetime.fromtimestamp(simdi, common.TZ).date() - datetime.timedelta(days=GECIKME_GUN)).isoformat()
    return not son_gun or son_gun < sinir


def parola_alanlari(etiket="Parola"):
    """Yönetici parolayı iki kez yazar (yanlış yazılan parola müşteriye/cihaza gitmesin)."""
    return (f'<label for="p1">{etiket} (en az {guvenlik.EN_AZ} karakter)</label>'
            '<input id="p1" name="parola" type="password" autocomplete="new-password" required>'
            '<label for="p2">Parola (tekrar)</label>'
            '<input id="p2" name="parola2" type="password" autocomplete="new-password" required>')


def parola_formdan(form):
    """Dönen: (parola, hata). Kısa ya da iki alan farklıysa hata."""
    pw, pw2 = form.get("parola", ""), form.get("parola2", "")
    if len(pw) < guvenlik.EN_AZ:
        return None, f"Parola en az {guvenlik.EN_AZ} karakter olmalı."
    if pw != pw2:
        return None, "İki parola aynı değil."
    return pw, ""



SMS_ETIKET = {"gonderildi": "SMS gitti", "mock": "deneme modu", "sinir": "günlük SMS sınırı",
              "yok": "SMS gönderilmedi (uyarılar kapalı)"}
ONEM_ETIKET = {"kritik": "kritik", "uyari": "uyarı", "bilgi": "bilgi"}
# Sağlık kartındaki "İnternet" satırı (PPPoE, spec 2026-10-23 "Durum ve teşhis"): raporun `pppoe` durum kodu → (metin, sorunlu mu)
INTERNET_DURUMLARI = {
    "kapali": ("Modem (DHCP)", False),
    "kablo_yok": ("kablo yok", True),
    "dhcp_yok": ("modemden adres alınamadı", True),
    "sunucu_yok": ("PPPoE: sunucu yanıt vermiyor", True),
    "kimlik_hatasi": ("PPPoE: parola reddedildi", True),
    "baglaniyor": ("PPPoE: bağlanıyor", False),
    "bagli": ("PPPoE bağlı", False),
}
INTERNET_TURLERI = {"dhcp": "Modem (DHCP)", "pppoe": "PPPoE"}  # durum kodu yokken yalnızca `wan_tur`
SINIR_ARALIK = veri_modulu.UYARI_SINIR_ARALIK  # günlük SMS sınırı aralığı (yönetim formu)
DENEME_METNI = "WifiCorrect deneme uyarısı"


def sms_etiketi(deger, onem):
    """Açık sorun satırının SMS durumu (acilis_sms / kapanis_sms değeri). Kaçış çağıranın işi."""
    if deger.startswith("hata:"):
        return f"SMS gönderilemedi ({deger[5:]})"
    if deger.startswith("sinir"):  # sinir:<gün>: o gün sınıra takıldı (açılışsa ertesi gün yeniden denenir)
        return SMS_ETIKET["sinir"]
    if deger in SMS_ETIKET:
        return SMS_ETIKET[deger]
    return "09:00'da gidecek" if onem == "bilgi" else "SMS bekleniyor"


def gecmis_sms_etiketleri(x, simdi):
    """Sorun geçmişi satırının (açılış SMS'i, kapanış SMS'i) etiketleri. Kapanmış sorunda boş değer artık gitmeyecek
    demektir ("—"); tek istisna: açılışı gitmiş, "düzeldi"si TEKRAR_SN dolana dek bekletiliyor."""
    if not x["kapanis"]:
        return sms_etiketi(x["acilis_sms"], x["onem"]), "—"
    acilis = sms_etiketi(x["acilis_sms"], x["onem"]) if x["acilis_sms"] else "—"
    if x["kapanis_sms"]:
        return acilis, sms_etiketi(x["kapanis_sms"], x["onem"])
    if (x["onem"] != "bilgi" and x["acilis_sms"] in ("gonderildi", "mock")
            and simdi - (_epoch(x["kapanis"]) or 0) < TEKRAR_SN):
        return acilis, "düzeldi bekletiliyor"
    return acilis, "—"


def internet_metni(r):
    """Sağlık kartındaki "İnternet" satırı (HTML): raporun `pppoe` durum koduna göre; kod yoksa ya da bilinmiyorsa yalnızca
    `wan_tur`; ikisi de yoksa (eski cihaz) ya da bozuksa "—". Sorunlu durumlar kötü işaretli. `r` cihazdan gelen veri: yalnızca
    bilinen değerler gösterilir (bilinmeyen değer sayfaya hiç girmez)."""
    durum, tur = r.get("pppoe"), r.get("wan_tur")
    if isinstance(durum, str) and durum in INTERNET_DURUMLARI:
        metin, sorunlu = INTERNET_DURUMLARI[durum]
        return f'<b class="kotu">{e(metin)}</b>' if sorunlu else e(metin)
    if isinstance(tur, str) and tur in INTERNET_TURLERI:
        return e(INTERNET_TURLERI[tur])
    return "—"


def once(sn):
    dk = int(sn) // 60
    return f"{dk} dk önce" if dk < 120 else (f"{dk // 60} saat önce" if dk < 2880 else f"{dk // 1440} gün önce")


def son_sms_metni(s):
    """Müşteri sayfası: "<zaman> · <sağlayıcı> · <sonuç/kod>" (deneme modundaki gönderimler yazılmaz)."""
    if not s:
        return "—"
    saglayici = {"netgsm": "NetGSM", "twilio": "Twilio"}.get(s["saglayici"], "—")
    sonuc = "gönderildi" if s["sonuc"] == "gonderildi" else f'{s["sonuc"]} ({s["kod"]})'
    return e(f'{common.tarih_goster(s["zaman"])} · {saglayici} · {sonuc}')


class Yonetim(web.Taban):
    rol = "yonetici"
    cerez = "wc_yonetim"
    giris_basligi = "Yönetim merkezi"
    giris_aciklama = "Hizmet sağlayıcı girişi."

    def __init__(self, veri, kayitlar, saat=time.time, yonetici_yolu=yonetici.YONETICI,
                 durum_yolu=durum_modulu.DURUM, kuyruk_ekle=kuyruk.ekle, gonder=netgsm.gonder, gecis=None, musteri_app=None,
                 github_al=surumler.github_al, getir=surumler.getir, surum_dizini=surumler.DIZIN,
                 liste_getir=surumler.getir, liste_dizini=listeler.DIZIN, rapor_gruplari=None, raporlar=None):
        super().__init__(veri, kayitlar, saat, raporlar or raporlar_modulu.Raporlar())
        self.yonetici_yolu, self.durum_yolu, self.kuyruk_ekle, self.gonder = yonetici_yolu, durum_yolu, kuyruk_ekle, gonder
        # cihaz paneline doğrudan geçiş: müşteri paneliyle aynı belirteç deposu ve cihaz adresi kuralı
        self.gecis, self.musteri_app = gecis, musteri_app
        self.github_al, self.getir, self.surum_dizini = github_al, getir, surum_dizini  # uzaktan güncelleme sürüm deposu
        self.liste_getir, self.liste_dizini = liste_getir, liste_dizini  # hazır engel listeleri
        self.rapor_gruplari = rapor_gruplari  # raporlar: eski arşiv bağı değişince root'un okuduğu grup dosyası (None: yazılmaz)

    # --- Taban ---
    def dogrula(self, kul, pw):
        y = yonetici.oku(self.yonetici_yolu)
        if not y or kul != y["kullanici"]:
            guvenlik.bos_dogrulama(pw)
            return None
        return kul if guvenlik.dogru(pw, y["tuz"], y["ozet"], y["yineleme"]) else None

    def hesap_surumu(self, kimlik):
        y = yonetici.oku(self.yonetici_yolu)
        return y["ozet"] if y and y["kullanici"] == kimlik else None

    def girdi(self, kimlik, ip):
        self.veri.hareket("admin", ip, "GIRIS", None, "yönetim")

    def kim(self, ot):
        return "admin"

    def hareket_musterisi(self, ot, yol):
        m = re.match(r"/m/([0-9]{7})/", yol)
        return int(m.group(1)) if m else None

    def menu(self, ot):
        return ('<a href="/">Müşteriler</a><a href="/yeni">Yeni müşteri</a><a href="/bekleyen">Bekleyen cihazlar</a><a href="/hareketler">Hareketler</a>'
                '<a href="/surumler">Sürümler</a><a href="/listeler">Engel listeleri</a><a href="/sms">SMS ayarları</a><a href="/uyari">Uyarı ayarları</a><a href="/admin-parolasi">Admin parolası</a><a href="/eski-arsivler">Eski arşivler</a><a href="/hesabim">Hesabım</a>')

    # --- yardımcılar ---
    def _durum(self):
        try:
            with open(self.durum_yolu, encoding="utf-8") as f:
                d = json.load(f)
        except (FileNotFoundError, ValueError):
            return {}
        a = d.get("arsivler") if isinstance(d, dict) else None
        return a if isinstance(a, dict) else {}

    def _ozet(self, m, c, d):
        """Bir müşterinin liste satırı bilgileri: (cihaz metni, son gün metni, sorunlu mu)."""
        simdi = self.saat()
        if c is None:
            return "cihaz yok", e(common.tarih_goster(d.get("son_gun")) or "—"), False
        el = d.get("el_sikisma")
        es = _epoch(c["son_eslesme"]) or _epoch(c["baglanma"]) or 0
        if c["durum"] == "serbest_birakiliyor":
            cihaz = "serbest bırakılıyor"
        elif el and simdi - el <= CEVRIMICI_SN:
            cihaz = "çevrimiçi"
        else:
            cihaz = f"son eşitleme {once(simdi - es)}" if c["son_eslesme"] else "henüz eşitlenmedi"
        sg = d.get("son_gun")
        gecikti = yedek_gecikti(sg, simdi)
        son = e(common.tarih_goster(sg) or "—") + (' <b class="kotu">GECİKTİ</b>' if gecikti else "")
        sorunlu = gecikti or simdi - es > ESLESME_GECIKME_SN or c["durum"] == "serbest_birakiliyor"
        return cihaz, son, sorunlu

    # --- sayfalar ---
    def sayfalar(self, ot, token, yontem, yol, sorgu, form, ip):
        if (yontem, yol) == ("GET", "/"):
            return self.liste(ot, sorgu)
        if yol == "/yeni":
            return self.yeni(ot, yontem, form, ip)
        if (yontem, yol) == ("GET", "/bekleyen"):
            return self.bekleyen_sayfasi(ot)
        if (yontem, yol) == ("GET", "/hareketler"):
            return yanit_html(self.sayfa("Hareketler", kart("<h1>Hareketler</h1>" + self._hareket_tablosu(self.veri.hareketler())), ot, genis=True))
        if yol == "/eski-arsivler":
            return self.eski_arsivler(ot, yontem, form, ip)
        if yol == "/sms":
            return self.sms(ot, yontem, form, ip)
        if (yontem, yol) == ("GET", "/surumler"):
            return self.surumler_sayfasi(ot)
        if (yontem, yol) == ("POST", "/surumler/al"):
            return self.surum_al(ot, ip)
        if (yontem, yol) == ("POST", "/surumler/hedef"):
            return self.kanal_hedef(ot, form, ip)
        if (yontem, yol) == ("GET", "/listeler"):
            return self.listeler_sayfasi(ot)
        if (yontem, yol) == ("POST", "/listeler/guncelle"):
            try:
                listeler.calistir(self.veri, self.liste_getir, self.liste_dizini, self.saat(), kim="admin", ip=ip, bekle=False)
            except listeler.Mesgul:
                return yanit_html(self.mesaj(ot, "Engel listeleri", "Güncelleme zaten sürüyor; birkaç dakika sonra sayfayı yenileyin."))
            return yonlendir("/listeler")
        if yol == "/uyari":
            return self.uyari(ot, yontem, form, ip)
        if (yontem, yol) == ("POST", "/uyari/deneme"):
            return self.uyari_deneme(ot, ip)
        if yol == "/admin-parolasi":
            return self.admin_parolasi(ot, yontem, form, ip)
        if yol == "/hesabim":
            return self.hesabim(ot, yontem, form, ip)
        m = re.fullmatch(r"/m/([0-9]{7})(/.*)?", yol)
        if not m or self.veri.musteri(int(m.group(1))) is None:
            return None
        n, alt = int(m.group(1)), m.group(2) or "/"
        if (yontem, alt) == ("GET", "/"):
            return self.musteri_sayfasi(ot, n)
        if (yontem, alt) == ("POST", "/parola-sifirla"):
            pw, hata = parola_formdan(form)
            if hata:
                return yanit_html(self.mesaj(ot, "Parola kaydedilmedi", hata), 400)
            self.veri.parola_koy(n, pw)
            self.veri.hareket("admin", ip, "PAROLA_SIFIRLANDI", n)
            return yonlendir(f"/m/{n}")
        if (yontem, alt) == ("POST", "/ap-parola"):
            self.veri.ap_parolasi_yenile(n)
            self.veri.hareket("admin", ip, "AP_PAROLASI_YENILENDI", n)  # parola hareket satırına yazılmaz
            return yonlendir(f"/m/{n}")
        if (yontem, alt) == ("POST", "/serbest"):
            return self.serbest(ot, n, form, ip)
        if (yontem, alt) == ("POST", "/guncelleme"):
            return self.guncelleme_kanali(ot, n, form, ip)
        if (yontem, alt) == ("POST", "/guncelleme-hemen"):
            return self.guncelleme_hemen(ot, n, ip)
        if (yontem, alt) == ("POST", "/cihaz-paneli"):
            return self.cihaz_paneli(ot, n, ip)
        if (yontem, alt) == ("POST", "/lisans"):
            return self.lisans(ot, n, form, ip)
        if (yontem, alt) == ("POST", "/uyelik"):
            if form.get("islem") == "bitir":
                if form.get("onay", "").strip() != str(n):
                    return yanit_html(self.mesaj(ot, "Onaylanmadı", "Üyeliği bitirmek için müşteri numarasını doğru yazın."), 400)
                self.veri.uyelik(n, "bitti")
                self.veri.serbest_birak(n)
                self.veri.hareket("admin", ip, "UYELIK_BITTI", n)
            elif form.get("islem") == "ac":
                self.veri.uyelik(n, "aktif")
                self.veri.serbest_iptal(n)  # bitirirken başlayan serbest bırakma, cihaz temizlenmediyse geri alınır
                self.veri.hareket("admin", ip, "UYELIK_ACILDI", n)
            return yonlendir(f"/m/{n}")
        if (yontem, alt) == ("GET", "/hareketler"):
            return yanit_html(self.sayfa("Hareketler", kart(f"<h1>Müşteri {n} · hareketler</h1>"
                                                            + self._hareket_tablosu(self.veri.hareketler(n))), ot, genis=True))
        if yontem == "GET" and (alt == "/raporlar" or alt.startswith("/rapor/")):  # kişisel veri yok: gerekçesiz
            return raporlar_modulu.istek(self, ot, n, alt, f"/m/{n}")
        if (yontem == "GET" and (alt == "/kayitlar" or alt.startswith("/kayitlar/"))) or (yontem, alt) == ("POST", "/kayitlar/ara"):
            c = self.veri.bagli_cihaz(n)
            baslik = (c["isletme_adi"] if c is not None and c["isletme_adi"] else f"Müşteri {n}")
            return self.kayit_sayfasi(ot, n, alt[len("/kayitlar"):] or "/", sorgu, f"/m/{n}/kayitlar", ip, baslik, yontem, form)
        return None

    def bekleyen_sayfasi(self, ot):
        """Kolay bağlama (spec 2026-10-16): bağlanmayı bekleyen cihazlar, salt okunur (kurulumda neyin olduğunu görmek için)."""
        simdi, satirlar = self.saat(), []
        for r in self.veri.bekleyenler():
            son = e(common.tarih_goster(common.now_iso(r["son"]), saniye=True))
            if simdi - r["son"] > veri_modulu.BEKLEYEN_ESKI_SN:
                son += ' <span class="uyarisi">eski</span>'
            if r["cihaz"] is not None:
                durum = f'bağlandı: <a href="/m/{r["musteri"]}">{r["musteri"]}</a>'
            elif r["musteri"] is not None:
                gecti = simdi - r["ayrilma"] > veri_modulu.AYRILMA_SN
                durum = (f'ayrıldı: <a href="/m/{r["musteri"]}">{r["musteri"]}</a>'
                         + (' <span class="alt">(süresi doldu)</span>' if gecti else ""))
            else:
                durum = "bekliyor"
            satirlar.append(f'<tr><td>{e(r["kimlik"][:8])}</td><td>{e(common.tarih_goster(common.now_iso(r["ilk"])))}</td>'
                            f'<td>{son}</td><td>{e(r["ip"])}</td><td>{e(r["surum"] or "—")}</td>'
                            f'<td>{e(r["wan_mac"] or "—")}</td><td>{durum}</td></tr>')
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>Kimlik</th><th>İlk görülme</th><th>Son görülme</th>'
                 '<th>İnternet adresi</th><th>Sürüm</th><th>WAN MAC</th><th>Durum</th></tr>' + "".join(satirlar)
                 + "</table></div>") if satirlar else '<p class="alt">Bağlanmayı bekleyen cihaz yok.</p>'
        aciklama = ('<p class="alt">Bağlanmamış cihazlar internete çıkınca 2 dakikada bir bildirir. Müşteri cihazla aynı internet '
                    'adresinden panele girince cihaz kendiliğinden bağlanır. 30 dakikadır bildirmeyen "eski", 7 gün bildirmeyen silinir.</p>')
        return yanit_html(self.sayfa("Bekleyen cihazlar", kart(f"<h1>Bekleyen cihazlar</h1>{aciklama}") + kart(tablo), ot, genis=True))

    def liste(self, ot, sorgu):
        q = detay.tr_kucuk((urllib.parse.parse_qs(sorgu).get("q") or [""])[0].strip())
        durum, cihazlar, satirlar = self._durum(), self.veri.cihazlar(), []
        gunler = [g.isoformat() for g in kayitlar_modulu.son_30_gun(self.saat())[:7]]
        acik, kritik = self.veri.acik_sorunlar(), {}
        for x in acik:
            if x["onem"] == "kritik":
                kritik.setdefault(x["musteri"], []).append(x["metin"])
        for m in self.veri.musteriler():
            n, c = m["numara"], cihazlar.get(m["numara"])
            paket = self._paket(m, c)
            ad = c["isletme_adi"] if c is not None else ""
            tur = isletme_turu(c)
            if q and q not in str(n) and q not in detay.tr_kucuk(ad) and q not in detay.tr_kucuk(m["not_"]):
                continue
            cihaz, son, sorunlu = self._ozet(m, c, durum.get(str(n), {}))
            cihaz = e(cihaz) + "".join(f' <b class="kotu">{e(t)}</b>' for t in kritik.get(n, []))  # kritik sorun rozeti
            sorunlu = sorunlu or n in kritik
            sayilar = self.kayitlar.sayilar(self.veri.arsivler(n))[0]
            yedi = " ".join(str(sayilar.get(g, "—")) for g in reversed(gunler))
            uyelik = "Aktif" if m["uyelik"] == "aktif" else f"Bitti {common.tarih_goster(m['bitis'][:10])}"
            ldurum, lbitis = self.veri.lisans_durumu(m)
            lbitis = common.tarih_goster(lbitis)
            lisans = {"aktif": e(lbitis), "bitti": f'<b class="kotu">süresi doldu {e(lbitis)}</b>',
                      "askida": f'<b class="kotu">askıda</b> {e(lbitis)}'}[ldurum]
            sorunlu = sorunlu or ldurum != "aktif"
            satirlar.append((not sorunlu, n,
                             f'<tr><td><a href="/m/{n}">{n}</a></td><td>{e(ad or "—")}<br><span class="alt">{e(m["not_"])}</span></td>'
                             f'<td>{e(tur)}</td><td>{cihaz}</td><td>{paket}</td><td>{son}</td><td>{e(yedi)}</td><td>{lisans}</td><td>{e(uyelik)}</td></tr>'))
        satirlar.sort()
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>No</th><th>İşletme / not</th><th>Tür</th><th>Cihaz</th>'
                 '<th>Sürüm</th><th>Son gelen gün</th><th>Son 7 gün</th><th>Lisans</th><th>Üyelik</th></tr>' + "".join(s[2] for s in satirlar)
                 + "</table></div>") if satirlar else '<p class="alt">Müşteri yok.</p>'
        ara = (f'<form class="ara" method="get" action="/"><input name="q" value="{e(q)}" placeholder="Numara, işletme adı ya da not">'
               '<button>Ara</button></form>')
        return yanit_html(self.sayfa("Müşteriler", self._acik_sorunlar_karti(acik, cihazlar)
                                     + kart(f"<h1>Müşteriler</h1>{self._izleme_satiri()}{ara}") + kart(tablo), ot, genis=True))

    def _kurulu_paket(self, c):
        """Cihazın son raporundaki kurulu paket sürümü: "1.2.0", "" (paketsiz kurulum) ya da None (rapor yok / eski sürüm)."""
        s = self.veri.saglik(c["id"]) if c is not None else None
        r = s["rapor"] if s and isinstance(s["rapor"], dict) else {}
        p = r.get("paket")
        return p if isinstance(p, str) else None

    def _paket(self, m, c):
        """Liste hücresi: kurulu paket; hedeften farklıysa sarı."""
        p = self._kurulu_paket(c)
        if p is None:
            return "—"
        if p == "":
            return "paketsiz"
        hedef = self.veri.cihaz_hedefi(m)
        return f'<span class="uyarisi">{e(p)}</span>' if hedef and hedef["surum"] != p else e(p)

    # --- sürümler (uzaktan güncelleme) ---
    def surumler_sayfasi(self, ot):
        cs = e(ot["csrf"])
        kurulu = {}
        for c in self.veri.cihazlar().values():
            p = self._kurulu_paket(c)
            if p:
                kurulu[p] = kurulu.get(p, 0) + 1
        liste = self.veri.surumler()
        satirlar = "".join(
            f'<tr><td>{e(x["surum"])}</td><td>{e(x["kaynak"])}</td><td>{e(common.tarih_goster(x["zaman"]))}</td><td>{e(detay.boyut(x["boyut"]))}</td>'
            f'<td>{e(x["sha256"][:12])}</td><td>{kurulu.get(x["surum"], 0)}</td></tr>' for x in liste)
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>Sürüm</th><th>Kaynak</th><th>Tarih</th><th>Boyut</th>'
                 f'<th>sha256</th><th>Kurulu cihaz</th></tr>{satirlar}</table></div>') if liste else '<p class="alt">Depoda sürüm yok.</p>'
        al = (f'<form method="post" action="/surumler/al"><input type="hidden" name="csrf" value="{cs}">'
              "<button>GitHub'dan yeni sürümleri al</button></form>")
        hedefler = ""
        for kanal, ad in (("kararli", "Kararlı"), ("test", "Test")):
            secili = self.veri.kanal_hedef(kanal)
            sec = '<option value="">Hedef yok</option>' + "".join(
                f'<option value="{e(x["surum"])}"{" selected" if x["surum"] == secili else ""}>{e(x["surum"])}</option>' for x in liste)
            hedefler += (f'<form method="post" action="/surumler/hedef" class="ara"><input type="hidden" name="csrf" value="{cs}">'
                         f'<input type="hidden" name="kanal" value="{kanal}"><label for="k-{kanal}">{ad} kanal hedefi</label>'
                         f'<select id="k-{kanal}" name="surum">{sec}</select><button>Kaydet</button></form>')
        return yanit_html(self.sayfa("Sürümler", kart("<h1>Sürümler</h1>" + al + tablo)
                                     + kart("<h2>Kanal hedefleri</h2>" + hedefler
                                            + '<p class="alt">Cihazlar kendi kanalının hedefini bir sonraki sağlık raporunda (5 dk) '
                                              "öğrenir ve kurar; hedef yoksa o kanaldaki cihazlar güncellenmez.</p>"), ot, genis=True))

    def surum_al(self, ot, ip):
        try:
            eklenen, hatalar = self.github_al(self.veri, self.surum_dizini, self.getir)
        except Exception as h:  # liste alınamadı: sayfa düşmez, mesaj gösterilir
            self.veri.hareket("admin", ip, "SURUM_AL_HATA", None, str(h)[:200])
            return yanit_html(self.mesaj(ot, "Sürümler alınamadı", f"GitHub sürüm listesi alınamadı: {h}"))
        for s in eklenen:
            self.veri.hareket("admin", ip, "SURUM_ALINDI", None, f"surum={s}")
        metin = ("Eklenen: " + ", ".join(eklenen) + ". ") if eklenen else "Yeni sürüm yok. "
        if hatalar:
            metin += "Alınamayanlar: " + "; ".join(hatalar)
        return yanit_html(self.mesaj(ot, "Sürümler", metin.strip()))

    # --- hazır engel listeleri ---
    def listeler_sayfasi(self, ot):
        satirlar = ""
        for k, ad in listeler.KATEGORILER.items():
            m = listeler.meta_oku(self.liste_dizini, k) or {}
            kaynak = "<br>".join(
                e(d.get("ad", "?")) + ": " + (f'<b class="kotu">hata</b> ({e(d["hata"])})'
                                               + (f' · son başarılı kopya {e(d.get("sayi", 0))}' if d.get("eski") else "")
                                               if d.get("hata") else f'{e(d.get("sayi", 0))}')
                for d in m.get("kaynaklar") or [] if isinstance(d, dict))
            durum = []
            if m.get("hata"):
                durum.append(f'<b class="kotu">{e(m["hata"])}</b> ({e(common.tarih_goster(m.get("hata_zaman")))})')
            if m.get("kesildi"):
                durum.append(f'<span class="uyarisi">{listeler.EN_COK} sınırında kesildi</span>')
            satirlar += (f'<tr><td>{e(ad)}</td><td>{e(m.get("sayi", "—"))}</td>'
                         f'<td>{e(common.tarih_goster(m.get("zaman")) or "henüz üretilmedi")}</td>'
                         f'<td>{e(str(m.get("sha256", ""))[:8]) or "—"}</td><td>{kaynak or "—"}</td><td>{"<br>".join(durum) or "tamam"}</td></tr>')
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>Kategori</th><th>Alan adı</th><th>Son güncelleme</th>'
                 f'<th>Sürüm</th><th>Kaynaklar</th><th>Durum</th></tr>{satirlar}</table></div>')
        dugme = (f'<form method="post" action="/listeler/guncelle"><input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
                 '<button>Şimdi güncelle</button></form>')
        return yanit_html(self.sayfa("Engel listeleri", kart(
            "<h1>Engel listeleri</h1>" + dugme + tablo
            + "<p class=\"alt\">Listeler her gün 04:30'da kaynaklardan yeniden üretilir; cihazlar değişen listeyi bir sonraki "
              "sağlık raporunda (5 dk) alır. Bir kaynak indirilemezse onun son başarılı kopyası, hiçbiri indirilemezse önceki "
              "liste kullanılır.</p>"), ot, genis=True))

    def kanal_hedef(self, ot, form, ip):
        kanal, sv = form.get("kanal", ""), form.get("surum", "")
        if kanal not in ("kararli", "test") or (sv and (not surumler.SURUM_RE.fullmatch(sv) or self.veri.surum(sv) is None)):
            return yanit_html(self.mesaj(ot, "Hedef kaydedilmedi", "Kanal ya da sürüm geçersiz (sürüm depoda olmalı)."), 400)
        self.veri.kanal_hedef_koy(kanal, sv or None)
        self.veri.hareket("admin", ip, "KANAL_HEDEF", None, f"kanal={kanal} surum={sv or '-'}")
        return yonlendir("/surumler")

    def guncelleme_kanali(self, ot, n, form, ip):
        kanal = form.get("kanal", "")
        if kanal not in veri_modulu.GUNCELLEME_KANALLARI:
            return yanit_html(self.mesaj(ot, "Kanal kaydedilmedi", "Geçersiz kanal."), 400)
        self.veri.guncelleme_kanali_koy(n, kanal)
        self.veri.hareket("admin", ip, "GUNCELLEME_KANALI", n, f"kanal={kanal}")
        return yonlendir(f"/m/{n}")

    def guncelleme_hemen(self, ot, n, ip):
        hedef = self.veri.cihaz_hedefi(self.veri.musteri(n))
        if hedef is None:  # kanal kapalı, kanalda hedef yok ya da hizmet kapalı: bayrak konmaz (sonuç gelmez, kalırdı)
            return yanit_html(self.mesaj(ot, "Güncelleme başlatılmadı",
                                         "Bu müşterinin kanalında hedef sürüm yok, kanal kapalı ya da hizmeti kapalı."), 400)
        self.veri.guncelleme_hemen_koy(n, self.saat(), hedef["surum"])
        self.veri.hareket("admin", ip, "GUNCELLEME_HEMEN", n)
        return yonlendir(f"/m/{n}")

    def _guncelleme_karti(self, m, c, cs):
        n = m["numara"]
        p = self._kurulu_paket(c)
        kurulu = "—" if p is None else ("Paketsiz kurulum — uzaktan güncellenmez" if p == "" else e(p))
        hedef = self.veri.cihaz_hedefi(m)
        s = self.veri.saglik(c["id"]) if c is not None else None
        g = (s["rapor"] if s and isinstance(s["rapor"], dict) else {}).get("guncelleme")
        if isinstance(g, dict):
            z = g.get("zaman")
            zaman = (common.tarih_goster(common.now_iso(z))
                     if isinstance(z, (int, float)) and not isinstance(z, bool) and z > 0 else "")
            etiket = {"tamam": "Tamam", "geri_alindi": "Geri alındı", "hata": "Hata"}.get(g.get("sonuc"), g.get("sonuc", "?"))
            son = f'{e(g.get("surum", "?"))} · <b>{e(etiket)}</b> · {e(zaman)}'
        else:
            son = "—"
        kanallar = {"kararli": "Kararlı", "test": "Test", "kapali": "Kapalı (güncellenmez)"}
        sec = "".join(f'<option value="{k}"{" selected" if k == m["guncelleme_kanali"] else ""}>{ad}</option>' for k, ad in kanallar.items())
        bekliyor = '<b class="uyarisi">bekleniyor</b>' if hedef and hedef["hemen"] else "—"
        gecmis = detay._tablo(("Zaman", "Ayrıntı"),
                              [(common.tarih_goster(h["zaman"]), h["ayrinti"])
                               for h in self.veri.hareketler(n, 10, olay="GUNCELLEME_SONUC")], "Güncelleme denemesi yok.")
        return (bilgi_tablosu([("Kurulu paket", kurulu), ("Hedef sürüm", e(hedef["surum"]) if hedef else "—"),
                               ("Son sonuç", son), ("Şimdi güncelle", bekliyor)])
                + f'<form method="post" action="/m/{n}/guncelleme" class="ara"><input type="hidden" name="csrf" value="{cs}">'
                  f'<select name="kanal" aria-label="Güncelleme kanalı">{sec}</select><button>Kanalı kaydet</button></form>'
                + f'<form method="post" action="/m/{n}/guncelleme-hemen"><input type="hidden" name="csrf" value="{cs}">'
                  "<button>Şimdi güncelle</button></form>"
                  '<p class="alt">Cihaz hedefi sağlık raporunda (5 dk) görür; "şimdi güncelle" bekleme penceresini atlar.</p>'
                + "<h3>Geçmiş</h3>" + gecmis)

    def _izleme_satiri(self):
        """wc-saglik'in son turu: hep görünür; 15 dk'dan eskiyse ya da hiç yoksa kırmızı (zamanlayıcı durmuş olabilir)."""
        t = _epoch(self.veri.saglik_son_tur())
        if t is None:
            return '<p><b class="kotu">Sağlık izleme: hiç tur yok — izleme çalışmıyor olabilir</b></p>'
        sn = max(0, self.saat() - t)
        metin = f"Sağlık izleme: son tur {once(sn)}"
        if sn > IZLEME_BAYAT_SN:
            return f'<p><b class="kotu">{e(metin)} — izleme çalışmıyor olabilir</b></p>'
        return f'<p class="alt">{e(metin)}</p>'

    def _acik_sorunlar_karti(self, acik, cihazlar):
        if not acik:
            return ""
        simdi, satirlar = self.saat(), []
        for x in acik:
            c = cihazlar.get(x["musteri"])
            sn = simdi - (_epoch(x["acilis"]) or simdi)
            if x["musteri"] == 0:  # saglik.MERKEZ: müşteriye bağlı olmayan sorun (ör. engel listeleri)
                bas = '<td>—</td><td><a href="/listeler">Merkez</a></td>'
            else:
                bas = (f'<td><a href="/m/{x["musteri"]}">{x["musteri"]}</a></td>'
                       f'<td>{e(c["isletme_adi"] if c is not None else "")}</td>')
            satirlar.append(f'<tr>{bas}'
                            f'<td>{e(x["metin"])}</td><td>{e(ONEM_ETIKET.get(x["onem"], x["onem"]))}</td><td>{e(once(sn))}</td>'
                            f'<td>{e(sms_etiketi(x["acilis_sms"], x["onem"]))}</td></tr>')
        return kart('<h2>Açık sorunlar</h2><div class="kaydir"><table class="detay"><tr><th>No</th><th>İşletme</th><th>Sorun</th>'
                    '<th>Önem</th><th>Ne zamandır</th><th>SMS</th></tr>' + "".join(satirlar) + "</table></div>")

    def yeni(self, ot, yontem, form, ip):
        if yontem == "GET":
            return yanit_html(self.sayfa("Yeni müşteri", kart(
                '<h1>Yeni müşteri</h1><form method="post" action="/yeni">'
                f'<input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
                '<label for="n">Not (isteğe bağlı, yalnızca siz görürsünüz)</label>'
                '<input id="n" name="not" maxlength="200" placeholder="ör. Ahmet Bey, Kadıköy">'
                + parola_alanlari("Müşteri parolası") + '<button>Oluştur</button></form>'), ot))
        pw, hata = parola_formdan(form)
        if hata:
            return yanit_html(self.mesaj(ot, "Müşteri açılmadı", hata), 400)
        n, pw = self.veri.musteri_ekle(form.get("not", ""), parola=pw)
        self.veri.hareket("admin", ip, "MUSTERI_ACILDI", n)
        return yanit_html(self.sayfa("Müşteri açıldı", kart(
            f'<h1>Müşteri açıldı</h1><p>Müşteri numarası</p><p class="sir">{n}</p><p>Parola</p><p class="sir">{e(pw)}</p>'
            f'<p>Lisans bitişi: {e(common.tarih_goster(self.veri.musteri(n)["lisans_bitis"]))}</p>'
            '<p class="alt">Parolayı müşteriye iletin (müşteri sayfasında da görünür). Müşteri cihazın giriş ekranına bu numara ve '
            f'parolayla girer.</p><p><a href="/m/{n}">Müşteri sayfası →</a></p>'), ot))

    def _cihaz_adresi(self, n):
        return self.musteri_app.cihaz_adresi(n) if self.musteri_app is not None and self.gecis is not None else None

    def _cihaz_paneli_dugmesi(self, n, cs):
        if not self._cihaz_adresi(n):
            return '<span class="bos">şu an ulaşılamıyor (cihaz kapalı, tünel yok ya da lisans/üyelik kapalı)</span>'
        return (f'<form method="post" action="/m/{n}/cihaz-paneli" class="satir-ici"><input type="hidden" name="csrf" value="{cs}">'
                '<button>Cihaz paneline git</button></form>')

    def cihaz_paneli(self, ot, n, ip):
        """Müşteri parolası girmeden cihaz.wificorrect.com'a: tek kullanımlık 60 sn'lik geçiş belirteci (yönetici rolü)."""
        if not self._cihaz_adresi(n):
            return yanit_html(self.mesaj(ot, "Cihaza ulaşılamıyor",
                                         "Cihaz kapalı, tünel yok ya da lisans/üyelik kapalı."), 409)
        t = self.gecis.uret(str(n), self.saat(), self.veri.musteri(n)["ozet"], rol="yonetici")
        self.veri.hareket("admin", ip, "YONETICI_CIHAZ_PANELI", n)
        return yonlendir(cihaz_modulu.CIHAZ_GIRIS + t)

    def musteri_sayfasi(self, ot, n):
        m, c, d = self.veri.musteri(n), self.veri.bagli_cihaz(n), self._durum().get(str(n), {})
        cs = e(ot["csrf"])
        ldurum, lbitis = self.veri.lisans_durumu(m)
        parola = (f'<span class="sir">{e(m["parola_acik"])}</span>' if m["parola_acik"]
                  else '<span class="bos">bilinmiyor (bu özellikten önce belirlendi) — parolayı sıfırlayın</span>')
        lisans_metni = {"aktif": "Aktif", "bitti": '<b class="kotu">Süresi doldu</b>', "askida": '<b class="kotu">Askıda</b>'}[ldurum]
        # AP'ye (müşterinin erişim noktası/modemi) yönetim parolası olarak bu yazılır, panel parolası değil (final review I1)
        ap_parola = (f'<span class="sir">{e(m["ap_parola"])}</span>'
                     f'<form method="post" action="/m/{n}/ap-parola" class="satir-ici"><input type="hidden" name="csrf" value="{cs}">'
                     '<button class="ikincil">Yenile</button></form><br><span class="alt">AP\'nin (modemin) yönetim parolası; panel '
                     'parolasını AP\'ye yazmayın. Yenilenirse AP\'de elle değiştirilir; cihaza 5 dk içinde ulaşır (cihaz panelinde '
                     'görünür). Cihaz çevrimdışıyken yenilerseniz cihaz yeniden bağlanana kadar eski parolayı kullanır.</span>')
        bilgi = bilgi_tablosu([("Numara", str(n)), ("Parola", parola), ("AP yönetim parolası", ap_parola),
                               ("Lisans", f"{lisans_metni} · bitiş {e(common.tarih_goster(lbitis) or '—')}"),
                               ("Not", e(m["not_"] or "—")), ("Oluşturma", e(common.tarih_goster(m["olusturma"]))),
                               ("Son giriş (panel)", e(common.tarih_goster(m["son_giris"]) or "—")),
                               ("Bugün doğrulama SMS'i", str(self.veri.sms_sayac(n, self.veri.bugun().isoformat()))),
                               ("Son doğrulama SMS'i", son_sms_metni(self.veri.sms_son(n))),
                               ("Üyelik", "Aktif" if m["uyelik"] == "aktif" else f"Bitti {e(common.tarih_goster(m['bitis'][:10]))}")])
        if c is None:
            cihaz = '<p class="alt">Bağlı cihaz yok. Müşteri bir cihazın giriş ekranına numarasıyla girince bağlanır.</p>'
        else:
            metin, son, _ = self._ozet(m, c, d)
            cihaz = bilgi_tablosu([
                ("Durum", e(metin)), ("İşletme adı", e(c["isletme_adi"] or "—")), ("İşletme türü", e(isletme_turu(c))),
                ("Unvan", e(c["unvan"] or "—")), ("Adres", e(c["adres"] or "—")),
                ("Tünel adresi", e(c["tunel_ip"] or "—")), ("Sürüm", e(c["surum"] or "—")),
                ("Bağlanma", e(common.tarih_goster(c["baglanma"]))), ("Son eşitleme", e(common.tarih_goster(c["son_eslesme"]) or "—")),
                ("Son gelen gün", son), ("Arşiv", e(detay.boyut(d.get("boyut", 0)))),
                ("Cihaz paneli", self._cihaz_paneli_dugmesi(n, cs))])
            cihaz += (f'<form method="post" action="/m/{n}/serbest"><input type="hidden" name="csrf" value="{cs}">'
                      f'<label for="o">Cihazı serbest bırakmak için müşteri numarasını yazın ({n})</label>'
                      '<input id="o" name="onay" inputmode="numeric" autocomplete="off" required>'
                      '<label><input type="checkbox" name="zorla" value="1"> Cihaz ulaşılamaz (arızalı/kayıp): beklemeden ayır. '
                      'Cihazda kalmış gönderilmemiş günler kaybolabilir.</label>'
                      '<button class="tehlike">Cihazı serbest bırak</button></form>')
        uyelik_dugme = ("bitir", "Üyeliği bitir", "tehlike") if m["uyelik"] == "aktif" else ("ac", "Üyeliği yeniden aç", "")
        askida = ldurum == "askida"
        islemler = (f'<form method="post" action="/m/{n}/lisans"><input type="hidden" name="csrf" value="{cs}">'
                    '<input type="hidden" name="islem" value="uzat"><button>Lisansı 1 yıl uzat</button></form>'
                    f'<form method="post" action="/m/{n}/lisans"><input type="hidden" name="csrf" value="{cs}">'
                    f'<input type="hidden" name="islem" value="{"ac" if askida else "askiya"}">'
                    f'<button class="{"" if askida else "tehlike"}">{"Askıdan çıkar (cihazı aç)" if askida else "Askıya al (cihazı kapat)"}</button></form>'
                    f'<form method="post" action="/m/{n}/lisans" class="ara"><input type="hidden" name="csrf" value="{cs}">'
                    '<input type="hidden" name="islem" value="tarih"><input name="tarih" type="date" required aria-label="Lisans bitiş tarihi">'
                    '<button>Bitişi ayarla</button></form>'
                    '<p class="alt">Lisansı biten ya da askıya alınan cihaz misafirlere internet vermez; cihaz bunu bitiş gününden sonra '
                    'kendisi uygular, askıya alma/uzatma en geç 30 dakikada (askıdayken) ya da ertesi 06:00\'da cihaza iner.</p>'
                    f'<form method="post" action="/m/{n}/parola-sifirla"><input type="hidden" name="csrf" value="{cs}">'
                    + parola_alanlari("Yeni müşteri parolası") +
                    '<button>Parolayı değiştir</button><p class="alt">Merkezi panelde hemen, cihazda bir sonraki '
                    'eşitlemeden (her gün 06:00) sonra geçerlidir.</p></form>'
                    f'<form method="post" action="/m/{n}/uyelik"><input type="hidden" name="csrf" value="{cs}">'
                    f'<input type="hidden" name="islem" value="{uyelik_dugme[0]}">'
                    + (f'<label for="u">Üyeliği bitirmek için müşteri numarasını yazın ({n})</label>'
                       '<input id="u" name="onay" inputmode="numeric" autocomplete="off" required>' if uyelik_dugme[0] == "bitir" else "")
                    + f'<button class="{uyelik_dugme[2]}">{uyelik_dugme[1]}</button></form>'
                    f'<p><a href="/m/{n}/kayitlar/">Kayıtlar</a> · <a href="/m/{n}/raporlar">Raporlar</a> · <a href="/m/{n}/hareketler">Hareketler</a></p>')
        gecmis = detay._tablo(("No", "Durum", "Bağlanma", "Ayrılma", "Tünel"),
                                  [(x["id"], x["durum"], common.tarih_goster(x["baglanma"]), common.tarih_goster(x["ayrilma"]), x["tunel_ip"])
                                   for x in self.veri.cihaz_gecmisi(n)], "Henüz cihaz yok.")
        return yanit_html(self.sayfa(f"Müşteri {n}", kart(f"<h1>Müşteri {n}</h1>{bilgi}") + kart(f"<h2>Cihaz</h2>{cihaz}")
                                     + kart(f"<h2>Sağlık</h2>{self._saglik_tablosu(c)}")
                                     + kart(f"<h2>Güncelleme</h2>{self._guncelleme_karti(m, c, cs)}")
                                     + kart(f"<h2>Sorun geçmişi</h2>{self._sorun_gecmisi(n)}")
                                     + kart(f"<h2>İşlemler</h2>{islemler}") + kart(f"<h2>Cihaz geçmişi</h2>{gecmis}"), ot, genis=True))

    def _saglik_tablosu(self, c):
        s = self.veri.saglik(c["id"]) if c is not None else None
        if not s:
            return '<p class="alt">Henüz rapor yok.</p>'
        r = s["rapor"] if isinstance(s["rapor"], dict) else {}  # cihazdan gelen veri: her değer kaçırılır
        sv = r.get("servisler") if isinstance(r.get("servisler"), dict) else {}
        servis = " · ".join(e(ad) + (" çalışıyor" if ok else ' <b class="kotu">DURDU</b>') for ad, ok in sv.items()) or "—"
        ac = r.get("acik_sn")
        acik = e(once(ac).replace(" önce", "")) if isinstance(ac, (int, float)) and not isinstance(ac, bool) else "—"
        ep = _epoch(s["zaman"])
        zaman = f'{e(common.tarih_goster(s["zaman"]))} ({e(once(self.saat() - ep))})' if ep else e(s["zaman"])

        def durum(d, iyi):
            return "—" if d is None else ("tamam" if bool(d) == iyi else '<b class="kotu">sorun</b>')
        return bilgi_tablosu([
            ("Son rapor", zaman), ("Sürüm", e(r.get("surum", "—"))), ("Açık kalma", acik), ("Servisler", servis),
            ("Disk", f'%{e(r["disk_yuzde"])}' if "disk_yuzde" in r else "—"), ("Son mühür", e(common.tarih_goster(r.get("son_muhur")) or "—")),
            ("WAN", durum(r.get("wan"), True)), ("İnternet", internet_metni(r)), ("SMS", durum(r.get("sms_sorunu"), False)),
            ("Bağlı cihaz", e(r.get("bagli", "—")))])

    def _sorun_gecmisi(self, n):
        simdi = self.saat()
        satirlar = [(common.tarih_goster(x["acilis"]), common.tarih_goster(x["kapanis"]) or "açık", x["metin"], ONEM_ETIKET.get(x["onem"], x["onem"]),
                     *gecmis_sms_etiketleri(x, simdi)) for x in self.veri.sorun_gecmisi(n, 50)]
        return detay._tablo(("Açılış", "Kapanış", "Sorun", "Önem", "Açılış SMS'i", "Kapanış SMS'i"), satirlar, "Sorun kaydı yok.")

    def sms(self, ot, yontem, form, ip):
        """NetGSM (eski cihazlara da gider), doğrulama SMS'i için Türk numarası sağlayıcısı ve Twilio (yalnızca merkezde).
        Sır alanları (NetGSM şifresi, Twilio auth token / API secret) yalnızca yazılır; boş bırakılırsa eskisi kalır."""
        eski = self.veri.sms_ayari() or {"mock": True, "usercode": "", "password": "", "msgheader": "", "appkey": "",
                                         "tr_saglayici": "netgsm", "twilio": dict(veri_modulu.TWILIO_VARSAYILAN)}
        hata = ""
        if yontem == "POST":
            et = eski["twilio"]
            tw = {"acik": form.get("twilio_acik") == "1",
                  **{k: form.get(k, "") for k in veri_modulu.TWILIO_ALANLARI if k not in veri_modulu.TWILIO_SIRLARI},
                  **{k: form.get(k, "") or et[k] for k in veri_modulu.TWILIO_SIRLARI}}
            yeni = {"mock": form.get("mock") == "1", "usercode": form.get("usercode", ""), "msgheader": form.get("msgheader", ""),
                    "appkey": form.get("appkey", ""), "password": form.get("password", "") or eski["password"],
                    "tr_saglayici": form.get("tr_saglayici", "netgsm"), "twilio": tw}
            try:
                self.veri.sms_ayari_koy(yeni)
            except ValueError as h:
                hata = str(h)
            else:
                k = self.veri.sms_ayari()
                degisen = [x for x in ("mock", "usercode", "msgheader", "appkey", "password", "tr_saglayici") if k[x] != eski[x]]
                degisen += ["twilio_" + x for x in k["twilio"] if k["twilio"][x] != et[x]]
                self.veri.hareket("admin", ip, "SMS_AYARI", None, "değişen=" + ",".join(degisen))  # değer yazılmaz
                return yonlendir("/sms")
            # hatalı formda yazılanlar geri gösterilir; sırlar asla
            eski = dict(yeni, password=eski["password"], twilio=dict(tw, **{x: et[x] for x in veri_modulu.TWILIO_SIRLARI}))
        cs = e(ot["csrf"])
        t = eski["twilio"]

        def tanim(v):
            return "tanımlı" if v else "tanımsız"

        def secili(d):
            return " selected" if eski["tr_saglayici"] == d else ""
        h = f'<p class="hata">{e(hata)}</p>' if hata else ""  # iç içe aynı tırnak 3.12 öncesinde sözdizimi hatası
        icerik = (f'<h1>SMS ayarları</h1>{h}'
                  f'<form method="post" action="/sms"><input type="hidden" name="csrf" value="{cs}">'
                  f'<label><input type="checkbox" name="mock" value="1"{" checked" if eski["mock"] else ""}> Deneme modu (SMS gönderilmez)</label>'
                  '<label for="tr">Türk numaraları için sağlayıcı</label><select id="tr" name="tr_saglayici">'
                  f'<option value="netgsm"{secili("netgsm")}>NetGSM</option><option value="twilio"{secili("twilio")}>Twilio</option></select>'
                  '<h2>NetGSM</h2>'
                  f'<label for="u">NetGSM kullanıcı kodu</label><input id="u" name="usercode" value="{e(eski["usercode"])}" autocomplete="off">'
                  f'<label for="p">API şifresi ({tanim(eski["password"])})</label><input id="p" name="password" type="password" autocomplete="new-password" '
                  'placeholder="değiştirmek için yazın (boş = aynı kalır)">'
                  f'<label for="b">Mesaj başlığı</label><input id="b" name="msgheader" value="{e(eski["msgheader"])}" maxlength="11">'
                  f'<label for="a">Uygulama anahtarı (opsiyonel)</label><input id="a" name="appkey" value="{e(eski["appkey"])}">'
                  '<h2>Twilio Verify</h2>'
                  f'<label><input type="checkbox" name="twilio_acik" value="1"{" checked" if t["acik"] else ""}> Twilio açık '
                  "(yabancı numaralar Twilio'dan gider)</label>"
                  f'<label for="ta">Account SID</label><input id="ta" name="account_sid" value="{e(t["account_sid"])}" autocomplete="off">'
                  f'<label for="tt">Auth token ({tanim(t["auth_token"])})</label><input id="tt" name="auth_token" type="password" '
                  'autocomplete="new-password" placeholder="değiştirmek için yazın (boş = aynı kalır)">'
                  f'<label for="tk">API key SID (opsiyonel)</label><input id="tk" name="api_key_sid" value="{e(t["api_key_sid"])}" autocomplete="off">'
                  f'<label for="ts">API key secret ({tanim(t["api_key_secret"])})</label><input id="ts" name="api_key_secret" type="password" '
                  'autocomplete="new-password" placeholder="değiştirmek için yazın (boş = aynı kalır)">'
                  f'<label for="tv">Verify service SID</label><input id="tv" name="verify_sid" value="{e(t["verify_sid"])}" autocomplete="off">'
                  '<button>Kaydet</button></form>'
                  "<p class=\"alt\">Doğrulama SMS'leri cihazların isteğiyle bu sunucudan gönderilir (değişiklik hemen geçerli). "
                  'NetGSM bilgileri eski sürümlü cihazlara bir sonraki eşitlemede (her gün 06:00) de gider; Twilio bilgileri hiçbir cihaza gitmez.</p>')
        return yanit_html(self.sayfa("SMS ayarları", kart(icerik), ot), 400 if hata else 200)

    def uyari(self, ot, yontem, form, ip):
        """Uyarı SMS'i alıcıları ve günlük sınır. Telefonlar 0533… ya da 533… yazılabilir; veri katmanı doğrular."""
        a, hata = self.veri.uyari_ayari(), ""
        if yontem == "POST":
            tel = [form.get(f"tel{i}", "").strip() for i in (1, 2, 3)]
            acik, ham = form.get("acik") == "1", form.get("gunluk_sinir", "").strip()
            try:
                sinir = int(ham)
                if not SINIR_ARALIK[0] <= sinir <= SINIR_ARALIK[1]:
                    raise ValueError
            except ValueError:
                hata = f"Günlük SMS sınırı {SINIR_ARALIK[0]} ile {SINIR_ARALIK[1]} arasında bir sayı olmalı."
            else:
                try:
                    self.veri.uyari_ayari_koy({"telefonlar": [t for t in tel if t], "acik": acik, "gunluk_sinir": sinir})
                except ValueError as h:
                    hata = str(h)
                else:
                    self.veri.hareket("admin", ip, "UYARI_AYAR", None,  # telefon yazılmaz
                                      f"telefon={sum(1 for t in tel if t)} acik={int(acik)} sinir={sinir}")
                    return yonlendir("/uyari")
            a = {"telefonlar": tel, "acik": acik, "gunluk_sinir": ham}  # yazılanı geri göster
        tel = list(a["telefonlar"]) + [""] * 3
        alanlar = "".join(
            f'<label for="t{i}">Telefon {i}</label><input id="t{i}" name="tel{i}" '
            f'value="{e(("0" + t[2:]) if t.startswith("90") and len(t) == 12 else t)}" inputmode="tel" autocomplete="off" '
            'placeholder="05XX XXX XX XX">' for i, t in enumerate(tel[:3], 1))
        h = f'<p class="hata">{e(hata)}</p>' if hata else ""
        cs = e(ot["csrf"])
        icerik = (f'<h1>Uyarı ayarları</h1>{h}<form method="post" action="/uyari"><input type="hidden" name="csrf" value="{cs}">'
                  f'{alanlar}<label><input type="checkbox" name="acik" value="1"{" checked" if a["acik"] else ""}> SMS uyarıları açık</label>'
                  f'<label for="s">Günlük SMS sınırı ({SINIR_ARALIK[0]}–{SINIR_ARALIK[1]})</label>'
                  f'<input id="s" name="gunluk_sinir" value="{e(a["gunluk_sinir"])}" inputmode="numeric" autocomplete="off">'
                  '<button>Kaydet</button></form>'
                  f'<form method="post" action="/uyari/deneme"><input type="hidden" name="csrf" value="{cs}">'
                  "<button class=\"ikincil\">Deneme SMS'i gönder</button></form>"
                  "<p class=\"alt\">Cihaz sağlık sorunları (ulaşılamıyor, kayıt durdu, servis durdu, disk, lisans…) bu numaralara SMS olarak "
                  "gider. Deneme SMS'i ilk telefona gider ve günlük sayaca girer. Gönderim için SMS ayarları sayfasında NetGSM bilgileri "
                  "girilmiş olmalı; deneme modu açıksa SMS gitmez.</p>")
        return yanit_html(self.sayfa("Uyarı ayarları", kart(icerik), ot), 400 if hata else 200)

    def uyari_deneme(self, ot, ip):
        ayar, a = self.veri.sms_ayari(), self.veri.uyari_ayari()
        if ayar is None:
            return yanit_html(self.mesaj(ot, "Deneme SMS'i gitmedi", "SMS ayarları girilmemiş. Önce SMS ayarları sayfasını doldurun."))
        if not a["telefonlar"]:
            return yanit_html(self.mesaj(ot, "Deneme SMS'i gitmedi", "Uyarı ayarlarında kayıtlı telefon yok."))
        try:
            r = self.gonder(ayar, DENEME_METNI, a["telefonlar"][:1])
        except Exception:  # gönderici hatası yönetim sayfasını düşürmez
            r = "hata:AG"
        self.veri.sms_say(datetime.datetime.fromtimestamp(self.saat(), common.TZ).date().isoformat())
        self.veri.hareket("admin", ip, "UYARI_DENEME", None, f"sonuç={r}")
        ne = {"gonderildi": "Deneme SMS'i gönderildi.", "mock": "Deneme modu açık: SMS gönderilmedi, yalnızca kaydedildi."}
        return yanit_html(self.mesaj(ot, "Deneme SMS'i", ne.get(r, f"Deneme SMS'i gönderilemedi ({r}).")))

    def admin_parolasi(self, ot, yontem, form, ip):
        """Bütün cihazlarda tek admin parolası (2026-10-08): yönetici belirler, kendiliğinden değişmez; yalnızca özet saklanır."""
        if yontem == "POST":
            pw, hata = parola_formdan(form)
            if hata:
                return yanit_html(self.mesaj(ot, "Admin parolası kaydedilmedi", hata), 400)
            self.veri.admin_parolasi_koy(pw)
            self.veri.hareket("admin", ip, "ADMIN_PAROLASI")  # parola yazılmaz
            return yonlendir("/admin-parolasi")
        durum = "tanımlı" if self.veri.admin_parolasi() else "tanımsız (cihazlar kendi admin parolasını korur)"
        icerik = (f'<h1>Admin parolası</h1><p>Durum: <b>{durum}</b></p>'
                  f'<form method="post" action="/admin-parolasi"><input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
                  + parola_alanlari("Yeni admin parolası") + '<button>Kaydet</button></form>'
                  '<p class="alt">Bütün cihazlarda <b>admin</b> kullanıcısı bu parolayla girer. Cihazlar bir sonraki eşitlemede '
                  '(her gün 06:00) alır; siz değiştirmedikçe değişmez. Parola burada saklanmaz, yalnızca özeti tutulur — '
                  'unutursanız yenisini belirleyin.</p>')
        return yanit_html(self.sayfa("Admin parolası", kart(icerik), ot))

    def lisans(self, ot, n, form, ip):
        islem = form.get("islem")
        if islem == "uzat":
            self.veri.hareket("admin", ip, "LISANS_UZATILDI", n, f"bitis={self.veri.lisans_uzat(n)}")
        elif islem == "askiya":
            self.veri.askiya_al(n, True)
            self.veri.hareket("admin", ip, "LISANS_ASKIYA_ALINDI", n)
        elif islem == "ac":
            self.veri.askiya_al(n, False)
            self.veri.hareket("admin", ip, "LISANS_ACILDI", n)
        elif islem == "tarih":
            try:
                tarih = self.veri.lisans_ayarla(n, common.tarih_girdi(form.get("tarih", "")))  # 12.10.2026 ya da ISO
            except ValueError:
                return yanit_html(self.mesaj(ot, "Geçersiz tarih", "Tarih GG.AA.YYYY biçiminde olmalı."), 400)
            self.veri.hareket("admin", ip, "LISANS_TARIHI", n, f"bitis={tarih}")
        return yonlendir(f"/m/{n}")

    def serbest(self, ot, n, form, ip):
        if form.get("onay", "").strip() != str(n):
            return yanit_html(self.mesaj(ot, "Onaylanmadı", "Serbest bırakmak için müşteri numarasını doğru yazın."), 400)
        zorla = form.get("zorla") == "1"
        c = self.veri.serbest_birak(n, zorla)
        if c is None and not zorla:
            return yanit_html(self.mesaj(ot, "Cihaz yok", "Bu müşterinin bağlı cihazı yok."), 400)
        if zorla:
            self.kuyruk_ekle("cihaz-kapat", dict({"numara": str(n)}, **({"wg_pub": c["wg_pub"]} if c else {})))
            self.veri.hareket("admin", ip, "CIHAZ_ZORLA_AYRILDI", n, f"cihaz={c['id'] if c else '-'}")
        else:
            self.veri.hareket("admin", ip, "SERBEST_BIRAKMA_ISTENDI", n, f"cihaz={c['id']}")
        return yonlendir(f"/m/{n}")

    def _hareket_tablosu(self, hareketler):
        return detay._tablo(("Zaman", "Kim", "IP", "Olay", "Müşteri", "Ayrıntı"),
                                [(common.tarih_saat_goster(h["zaman"]), h["kim"], h["ip"], h["olay"], h["musteri"] or "", h["ayrinti"]) for h in hareketler])

    def eski_arsivler(self, ot, yontem, form, ip):
        durum, bagli = self._durum(), self.veri.eski_arsivler()
        adaylar = sorted(set(a for a in durum if not NUMARA_RE.fullmatch(a)) | set(bagli))
        if yontem == "POST":
            ad, mn = form.get("ad", ""), form.get("musteri", "").strip()
            if ad not in adaylar or (mn and (not NUMARA_RE.fullmatch(mn) or self.veri.musteri(int(mn)) is None)):
                return yanit_html(self.mesaj(ot, "Geçersiz", "Arşiv ya da müşteri numarası bulunamadı."), 400)
            self.veri.eski_arsiv_bagla(ad, int(mn) if mn else None)
            self.veri.hareket("admin", ip, "ESKI_ARSIV_BAGLANDI", int(mn) if mn else None, f"arsiv={ad}")
            if self.rapor_gruplari:
                try:
                    self.veri.rapor_gruplari_yaz(self.rapor_gruplari)
                except OSError as ex:  # bağ kaydedildi; dosya merkez açılışında yeniden yazılır
                    web.log(f"rapor-gruplari yazilamadi: {ex!r}")
            return yonlendir("/eski-arsivler")
        cs = e(ot["csrf"])
        satirlar = "".join(
            f'<tr><td>{e(a)}</td><td>{e(durum.get(a, {}).get("tur", "—"))}</td><td>{e(common.tarih_goster(durum.get(a, {}).get("son_gun")) or "—")}</td>'
            f'<td><form method="post" action="/eski-arsivler" class="ara"><input type="hidden" name="csrf" value="{cs}">'
            f'<input type="hidden" name="ad" value="{e(a)}"><input name="musteri" value="{e(bagli.get(a) or "")}" '
            'placeholder="müşteri no (boş = bağsız)" inputmode="numeric"><button>Kaydet</button></form></td></tr>'
            for a in adaylar)
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>Arşiv</th><th>Tür</th><th>Son gün</th><th>Bağlı müşteri</th></tr>'
                 f"{satirlar}</table></div>") if adaylar else '<p class="alt">Eski arşiv yok.</p>'
        return yanit_html(self.sayfa("Eski arşivler", kart(
            '<h1>Eski arşivler</h1><p class="alt">OpenWrt dönemi ve elle eklenmiş arşivler. Bir müşteriye bağlanırsa müşteri '
            f'kayıtlarında görünür.</p>{tablo}'), ot, genis=True))

    def hesabim(self, ot, yontem, form, ip):
        def html(hata="", tamam=""):
            m = f'<p class="hata">{e(hata)}</p>' if hata else (f'<p class="tamam">{e(tamam)}</p>' if tamam else "")
            return self.sayfa("Hesabım", kart(
                '<h1>Yönetici parolası</h1><form method="post" action="/hesabim">'
                f'<input type="hidden" name="csrf" value="{e(ot["csrf"])}">'
                '<label for="m">Mevcut parola</label><input id="m" name="mevcut" type="password" required>'
                '<label for="y">Yeni parola (en az 12)</label><input id="y" name="yeni" type="password" minlength="12" required>'
                '<label for="t">Yeni parola (tekrar)</label><input id="t" name="tekrar" type="password" minlength="12" required>'
                f'{m}<button>Değiştir</button></form>'), ot)
        if yontem == "GET":
            return yanit_html(html())
        if self.dogrula(ot["user"], form.get("mevcut", "")) is None:
            return yanit_html(html("Mevcut parola yanlış."), 400)
        if form.get("yeni", "") != form.get("tekrar", ""):
            return yanit_html(html("Yeni parolalar aynı değil."), 400)
        try:
            yonetici.yaz(self.yonetici_yolu, ot["user"], form.get("yeni", ""))
        except ValueError as h:
            return yanit_html(html(str(h)), 400)
        self.veri.hareket("admin", ip, "YONETICI_PAROLA_DEGISTI")
        self.oturumlar.drop_user(ot["user"])  # diğer oturumlar da düşer; değiştiren yeni oturumla devam eder
        yeni = self.oturum_ac(ot["user"], self.saat())
        ot = self.oturumlar.get(yeni, self.saat())
        return yanit_html(html(tamam="Parola değiştirildi."), cerez=(self.cerez, yeni))
