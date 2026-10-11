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
import netgsm
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


def once(sn):
    dk = int(sn) // 60
    return f"{dk} dk önce" if dk < 120 else (f"{dk // 60} saat önce" if dk < 2880 else f"{dk // 1440} gün önce")


class Yonetim(web.Taban):
    rol = "yonetici"
    cerez = "wc_yonetim"
    giris_basligi = "Yönetim merkezi"
    giris_aciklama = "Hizmet sağlayıcı girişi."

    def __init__(self, veri, kayitlar, saat=time.time, yonetici_yolu=yonetici.YONETICI,
                 durum_yolu=durum_modulu.DURUM, kuyruk_ekle=kuyruk.ekle, gonder=netgsm.gonder, gecis=None, musteri_app=None):
        super().__init__(veri, kayitlar, saat)
        self.yonetici_yolu, self.durum_yolu, self.kuyruk_ekle, self.gonder = yonetici_yolu, durum_yolu, kuyruk_ekle, gonder
        # cihaz paneline doğrudan geçiş: müşteri paneliyle aynı belirteç deposu ve cihaz adresi kuralı
        self.gecis, self.musteri_app = gecis, musteri_app

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
        return ('<a href="/">Müşteriler</a><a href="/yeni">Yeni müşteri</a><a href="/hareketler">Hareketler</a>'
                '<a href="/sms">SMS ayarları</a><a href="/uyari">Uyarı ayarları</a><a href="/admin-parolasi">Admin parolası</a><a href="/eski-arsivler">Eski arşivler</a><a href="/hesabim">Hesabım</a>')

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
            return "cihaz yok", e(d.get("son_gun") or "—"), False
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
        son = e(sg or "—") + (' <b class="kotu">GECİKTİ</b>' if gecikti else "")
        sorunlu = gecikti or simdi - es > ESLESME_GECIKME_SN or c["durum"] == "serbest_birakiliyor"
        return cihaz, son, sorunlu

    # --- sayfalar ---
    def sayfalar(self, ot, token, yontem, yol, sorgu, form, ip):
        if (yontem, yol) == ("GET", "/"):
            return self.liste(ot, sorgu)
        if yol == "/yeni":
            return self.yeni(ot, yontem, form, ip)
        if (yontem, yol) == ("GET", "/hareketler"):
            return yanit_html(self.sayfa("Hareketler", kart("<h1>Hareketler</h1>" + self._hareket_tablosu(self.veri.hareketler())), ot, genis=True))
        if yol == "/eski-arsivler":
            return self.eski_arsivler(ot, yontem, form, ip)
        if yol == "/sms":
            return self.sms(ot, yontem, form, ip)
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
        if (yontem, alt) == ("POST", "/serbest"):
            return self.serbest(ot, n, form, ip)
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
        if (yontem == "GET" and (alt == "/kayitlar" or alt.startswith("/kayitlar/"))) or (yontem, alt) == ("POST", "/kayitlar/ara"):
            c = self.veri.bagli_cihaz(n)
            baslik = (c["isletme_adi"] if c is not None and c["isletme_adi"] else f"Müşteri {n}")
            return self.kayit_sayfasi(ot, n, alt[len("/kayitlar"):] or "/", sorgu, f"/m/{n}/kayitlar", ip, baslik, yontem, form)
        return None

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
            ad = c["isletme_adi"] if c is not None else ""
            tur = isletme_turu(c)
            if q and q not in str(n) and q not in detay.tr_kucuk(ad) and q not in detay.tr_kucuk(m["not_"]):
                continue
            cihaz, son, sorunlu = self._ozet(m, c, durum.get(str(n), {}))
            cihaz = e(cihaz) + "".join(f' <b class="kotu">{e(t)}</b>' for t in kritik.get(n, []))  # kritik sorun rozeti
            sorunlu = sorunlu or n in kritik
            sayilar = self.kayitlar.sayilar(self.veri.arsivler(n))[0]
            yedi = " ".join(str(sayilar.get(g, "—")) for g in reversed(gunler))
            uyelik = "Aktif" if m["uyelik"] == "aktif" else f"Bitti {m['bitis'][:10]}"
            ldurum, lbitis = self.veri.lisans_durumu(m)
            lisans = {"aktif": e(lbitis), "bitti": f'<b class="kotu">süresi doldu {e(lbitis)}</b>',
                      "askida": f'<b class="kotu">askıda</b> {e(lbitis)}'}[ldurum]
            sorunlu = sorunlu or ldurum != "aktif"
            satirlar.append((not sorunlu, n,
                             f'<tr><td><a href="/m/{n}">{n}</a></td><td>{e(ad or "—")}<br><span class="alt">{e(m["not_"])}</span></td>'
                             f'<td>{e(tur)}</td><td>{cihaz}</td><td>{son}</td><td>{e(yedi)}</td><td>{lisans}</td><td>{e(uyelik)}</td></tr>'))
        satirlar.sort()
        tablo = ('<div class="kaydir"><table class="detay"><tr><th>No</th><th>İşletme / not</th><th>Tür</th><th>Cihaz</th>'
                 '<th>Son gelen gün</th><th>Son 7 gün</th><th>Lisans</th><th>Üyelik</th></tr>' + "".join(s[2] for s in satirlar)
                 + "</table></div>") if satirlar else '<p class="alt">Müşteri yok.</p>'
        ara = (f'<form class="ara" method="get" action="/"><input name="q" value="{e(q)}" placeholder="Numara, işletme adı ya da not">'
               '<button>Ara</button></form>')
        return yanit_html(self.sayfa("Müşteriler", self._acik_sorunlar_karti(acik, cihazlar)
                                     + kart(f"<h1>Müşteriler</h1>{self._izleme_satiri()}{ara}") + kart(tablo), ot, genis=True))

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
            satirlar.append(f'<tr><td><a href="/m/{x["musteri"]}">{x["musteri"]}</a></td><td>{e(c["isletme_adi"] if c is not None else "")}</td>'
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
            f'<p>Lisans bitişi: {e(self.veri.musteri(n)["lisans_bitis"])}</p>'
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
        bilgi = bilgi_tablosu([("Numara", str(n)), ("Parola", parola), ("Lisans", f"{lisans_metni} · bitiş {e(lbitis or '—')}"),
                               ("Not", e(m["not_"] or "—")), ("Oluşturma", e(m["olusturma"][:16])),
                               ("Son giriş (panel)", e(m["son_giris"][:16] or "—")),
                               ("Üyelik", "Aktif" if m["uyelik"] == "aktif" else f"Bitti {e(m['bitis'][:10])}")])
        if c is None:
            cihaz = '<p class="alt">Bağlı cihaz yok. Müşteri bir cihazın giriş ekranına numarasıyla girince bağlanır.</p>'
        else:
            metin, son, _ = self._ozet(m, c, d)
            cihaz = bilgi_tablosu([
                ("Durum", e(metin)), ("İşletme adı", e(c["isletme_adi"] or "—")), ("İşletme türü", e(isletme_turu(c))),
                ("Unvan", e(c["unvan"] or "—")),
                ("Tünel adresi", e(c["tunel_ip"] or "—")), ("Sürüm", e(c["surum"] or "—")),
                ("Bağlanma", e(c["baglanma"][:16])), ("Son eşitleme", e(c["son_eslesme"][:16] or "—")),
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
                    f'<p><a href="/m/{n}/kayitlar/">Kayıtlar</a> · <a href="/m/{n}/hareketler">Hareketler</a></p>')
        gecmis = detay._tablo(("No", "Durum", "Bağlanma", "Ayrılma", "Tünel"),
                                  [(x["id"], x["durum"], x["baglanma"][:16], x["ayrilma"][:16], x["tunel_ip"])
                                   for x in self.veri.cihaz_gecmisi(n)], "Henüz cihaz yok.")
        return yanit_html(self.sayfa(f"Müşteri {n}", kart(f"<h1>Müşteri {n}</h1>{bilgi}") + kart(f"<h2>Cihaz</h2>{cihaz}")
                                     + kart(f"<h2>Sağlık</h2>{self._saglik_tablosu(c)}")
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
        zaman = f'{e(s["zaman"][:16])} ({e(once(self.saat() - ep))})' if ep else e(s["zaman"])

        def durum(d, iyi):
            return "—" if d is None else ("tamam" if bool(d) == iyi else '<b class="kotu">sorun</b>')
        return bilgi_tablosu([
            ("Son rapor", zaman), ("Sürüm", e(r.get("surum", "—"))), ("Açık kalma", acik), ("Servisler", servis),
            ("Disk", f'%{e(r["disk_yuzde"])}' if "disk_yuzde" in r else "—"), ("Son mühür", e(r.get("son_muhur") or "—")),
            ("WAN", durum(r.get("wan"), True)), ("SMS", durum(r.get("sms_sorunu"), False)),
            ("Bağlı cihaz", e(r.get("bagli", "—")))])

    def _sorun_gecmisi(self, n):
        simdi = self.saat()
        satirlar = [(x["acilis"][:16], x["kapanis"][:16] or "açık", x["metin"], ONEM_ETIKET.get(x["onem"], x["onem"]),
                     *gecmis_sms_etiketleri(x, simdi)) for x in self.veri.sorun_gecmisi(n, 50)]
        return detay._tablo(("Açılış", "Kapanış", "Sorun", "Önem", "Açılış SMS'i", "Kapanış SMS'i"), satirlar, "Sorun kaydı yok.")

    def sms(self, ot, yontem, form, ip):
        """Bütün cihazların NetGSM ayarı. Şifre yalnızca yazılır; boş bırakılırsa eskisi kalır."""
        eski = self.veri.sms_ayari() or {"mock": True, "usercode": "", "password": "", "msgheader": "", "appkey": ""}
        hata = ""
        if yontem == "POST":
            yeni = {"mock": form.get("mock") == "1", "usercode": form.get("usercode", ""), "msgheader": form.get("msgheader", ""),
                    "appkey": form.get("appkey", ""), "password": form.get("password", "") or eski["password"]}
            try:
                self.veri.sms_ayari_koy(yeni)
            except ValueError as h:
                hata = str(h)
            else:
                degisen = [k for k in ("mock", "usercode", "msgheader", "appkey", "password") if yeni[k] != eski[k]]
                self.veri.hareket("admin", ip, "SMS_AYARI", None, "değişen=" + ",".join(degisen))  # değer yazılmaz
                return yonlendir("/sms")
            eski = dict(yeni, password=eski["password"])
        cs = e(ot["csrf"])
        sifre = "tanımlı" if eski["password"] else "tanımsız"
        h = f'<p class="hata">{e(hata)}</p>' if hata else ""  # iç içe aynı tırnak 3.12 öncesinde sözdizimi hatası
        icerik = (f'<h1>SMS ayarları</h1>{h}'
                  f'<form method="post" action="/sms"><input type="hidden" name="csrf" value="{cs}">'
                  f'<label><input type="checkbox" name="mock" value="1"{" checked" if eski["mock"] else ""}> Deneme modu (SMS gönderilmez)</label>'
                  f'<label for="u">NetGSM kullanıcı kodu</label><input id="u" name="usercode" value="{e(eski["usercode"])}" autocomplete="off">'
                  f'<label for="p">API şifresi ({sifre})</label><input id="p" name="password" type="password" autocomplete="new-password" '
                  'placeholder="değiştirmek için yazın (boş = aynı kalır)">'
                  f'<label for="b">Mesaj başlığı</label><input id="b" name="msgheader" value="{e(eski["msgheader"])}" maxlength="11">'
                  f'<label for="a">Uygulama anahtarı (opsiyonel)</label><input id="a" name="appkey" value="{e(eski["appkey"])}">'
                  '<button>Kaydet</button></form>'
                  '<p class="alt">Bütün bağlı cihazlara bir sonraki eşitlemede (her gün 06:00) gider.</p>')
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
                tarih = self.veri.lisans_ayarla(n, form.get("tarih", ""))
            except ValueError:
                return yanit_html(self.mesaj(ot, "Geçersiz tarih", "Tarih YYYY-AA-GG biçiminde olmalı."), 400)
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
                                [(h["zaman"][:19], h["kim"], h["ip"], h["olay"], h["musteri"] or "", h["ayrinti"]) for h in hareketler])

    def eski_arsivler(self, ot, yontem, form, ip):
        durum, bagli = self._durum(), self.veri.eski_arsivler()
        adaylar = sorted(set(a for a in durum if not NUMARA_RE.fullmatch(a)) | set(bagli))
        if yontem == "POST":
            ad, mn = form.get("ad", ""), form.get("musteri", "").strip()
            if ad not in adaylar or (mn and (not NUMARA_RE.fullmatch(mn) or self.veri.musteri(int(mn)) is None)):
                return yanit_html(self.mesaj(ot, "Geçersiz", "Arşiv ya da müşteri numarası bulunamadı."), 400)
            self.veri.eski_arsiv_bagla(ad, int(mn) if mn else None)
            self.veri.hareket("admin", ip, "ESKI_ARSIV_BAGLANDI", int(mn) if mn else None, f"arsiv={ad}")
            return yonlendir("/eski-arsivler")
        cs = e(ot["csrf"])
        satirlar = "".join(
            f'<tr><td>{e(a)}</td><td>{e(durum.get(a, {}).get("tur", "—"))}</td><td>{e(durum.get(a, {}).get("son_gun") or "—")}</td>'
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
