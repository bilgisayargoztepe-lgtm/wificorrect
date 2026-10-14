"""Cihaz API'si (spec §4): api.wificorrect.com/api/{giris,eslesme,parola}. JSON gövde ≤ 4 KB, IP başına dakikada 20 istek.
Cihazın kimliği ilk girişte verilen cihaz anahtarıdır (merkez yalnızca SHA-256'sını tutar)."""
import datetime
import json
import re
import time

import guvenlik
import kuyruk
import listeler
import netgsm
import surumler
import twilio
import veri as veri_modulu

GOVDE_SINIRI = 4096
NUMARA_RE = guvenlik.NUMARA_RE
YEDEK_SUNUCU = "10.99.0.1"
# Doğrulama SMS'i (/api/sms): sabit şablon, yalnızca 6 haneli kod; cihaz serbest metin gönderemez (açık röle olmasın)
OTP_SABLON = "WiFi dogrulama kodunuz: {kod}. Kod 3 dakika gecerlidir."
OTP_KOD_RE = re.compile(r"[0-9]{6}")
TELEFON_RE = re.compile(r"[0-9]{8,15}")  # E.164 rakamları, + olmadan
SMS_GUNLUK_SINIR = 300  # müşteri (cihaz) başına, TR günü
SMS_YABANCI_SINIR = 20  # yabancı numaralara (Twilio, pahalı) ayrıca; bunlar 300'e de sayılır
YABANCI_SAYAC = "/yabanci"  # sms_sayac'ta yabancı numara sayacının gün anahtarı eki


def _json(durum, veri):
    return durum, {"Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store"}, \
        json.dumps(veri, ensure_ascii=False).encode("utf-8")


def _hata(durum, kod, metin):
    return _json(durum, {"hata": metin, "kod": kod})


def _metin(v):
    """Cihazın bildirdiği metin: ≤ 200 yazdırılabilir karakter, değilse boş."""
    return v if isinstance(v, str) and len(v) <= 200 and v.isprintable() else ""


def _parola_alanlari(m):
    return {"tuz": m["tuz"], "ozet": m["ozet"], "yineleme": m["yineleme"]}


def _lisans(veri, m):
    durum, bitis = veri.lisans_durumu(m)
    return {"lisans": durum, "lisans_bitis": bitis}


def _cihaz_ekleri(veri):
    """Belirlenmişse tek admin parolasının özeti (açık parola gitmez) ve SMS eki. `mock` + `netgsm` yalnızca eski (0.4.7)
    cihazlar için, bugünkü gibi (ayar kayıtlıysa); yeni cihazlar `etkin_mock`u okur: deneme modu açık ya da seçili
    sağlayıcının ayarı eksik (ayar hiç yoksa yalnızca bu alan gider; eski cihaz `netgsm` olmadan eki yok sayar)."""
    ek = {}
    a = veri.admin_parolasi()
    if a is not None:
        ek["admin"] = a
    s = veri.sms_ayari()
    if s is None:
        ek["sms"] = {"etkin_mock": True}
    else:
        ek["sms"] = {"mock": s["mock"], "provider": "netgsm", "netgsm": {k: s[k] for k in veri_modulu.SMS_ALANLARI},
                     "etkin_mock": bool(s["mock"]) or not veri_modulu.sms_hazir(s)}
    return ek


SAGLIK_SERVISLER = ("portal", "panel", "kaydedici", "dnsmasq", "guvenlik")
SAGLIK_BOOL = ("kayit_boslugu_1s", "wan", "sms_sorunu", "hiz_hata")


def _tam(v, alt, ust=None):
    """bool int'in alt sınıfıdır: tamsayı alanı bool kabul etmez."""
    return type(v) is int and v >= alt and (ust is None or v <= ust)


def _tarih(v):
    if v == "":
        return True
    try:
        return len(v) == 10 and v.isascii() and datetime.datetime.strptime(v, "%Y-%m-%d") is not None
    except ValueError:
        return False


def _guncelleme_sonucu(g):
    ok = (isinstance(g, dict) and set(g) == {"surum", "sonuc", "zaman"} and isinstance(g["surum"], str)
          and surumler.SURUM_RE.fullmatch(g["surum"]) and g["sonuc"] in ("tamam", "geri_alindi", "hata") and _tam(g["zaman"], 0))
    return {a: g[a] for a in ("surum", "sonuc", "zaman")} if ok else False


def saglik_temizle(r):
    """Cihaz sağlık raporunu sıkı doğrular; yalnızca bilinen alanlarla yeni sözlük döner, bozuksa None."""
    if not isinstance(r, dict):
        return None
    sv = r.get("servisler")
    if not (isinstance(sv, dict) and all(type(sv.get(a)) is bool for a in SAGLIK_SERVISLER)):
        return None
    if not (isinstance(r.get("surum"), str) and len(r["surum"]) <= 40 and r["surum"].isprintable()):
        return None
    if not (_tam(r.get("acik_sn"), 0) and _tam(r.get("disk_yuzde"), 0, 100) and _tam(r.get("bagli"), 0)):
        return None
    if not (isinstance(r.get("son_muhur"), str) and _tarih(r["son_muhur"])):
        return None
    if not all(type(r.get(a)) is bool for a in SAGLIK_BOOL):
        return None
    t = {a: r[a] for a in ("surum", "acik_sn", "disk_yuzde", "son_muhur", "bagli")}
    t["servisler"] = {a: sv[a] for a in SAGLIK_SERVISLER}
    t.update({a: r[a] for a in SAGLIK_BOOL})
    if "paket" in r:  # uzaktan güncelleme: cihazda kurulu paket sürümü
        p = r["paket"]
        if not (isinstance(p, str) and len(p) <= 40 and (p == "" or surumler.SURUM_RE.fullmatch(p))):
            return None
        t["paket"] = p
    if "guncelleme" in r:  # son güncelleme denemesinin sonucu
        g = r["guncelleme"]
        if g is not None:
            g = _guncelleme_sonucu(g)
            if g is False:
                return None
        t["guncelleme"] = g
    return t


class Api:
    def __init__(self, veri, saat=time.time, kuyruk_ekle=kuyruk.ekle, kuyruk_bekle=kuyruk.bekle,
                 surum_dizini=surumler.DIZIN, sms_ac=None, liste_dizini=listeler.DIZIN):
        self.veri, self.saat, self.surum_dizini, self.liste_dizini = veri, saat, surum_dizini, liste_dizini
        self.sms_ac = sms_ac  # NetGSM/Twilio isteği (testte sahte; None = urlopen, WC_TEST altında reddedilir)
        self.kuyruk_ekle, self.kuyruk_bekle = kuyruk_ekle, kuyruk_bekle
        self.kilit, self.hiz = guvenlik.GirisKilidi(), guvenlik.HizSiniri(20, 60)

    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        uclar = {"/api/giris": self.giris, "/api/eslesme": self.eslesme, "/api/parola": self.parola,
                "/api/saglik": self.saglik, "/api/paket": self.paket, "/api/sms": self.sms,
                "/api/sms-dogrula": self.sms_dogrula, "/api/liste": self.liste}
        if yontem != "POST" or yol not in uclar:
            return _hata(404, "yok", "Bulunamadı.")
        if len(govde) > GOVDE_SINIRI:
            return _hata(413, "gecersiz", "İstek çok büyük.")
        if not self.hiz.izin(ip, self.saat()):
            return _hata(429, "hiz", "Çok fazla istek, biraz sonra tekrar deneyin.")
        try:
            b = json.loads(govde)
        except ValueError:
            b = None
        if not isinstance(b, dict):
            return _hata(400, "gecersiz", "Geçersiz istek.")
        return uclar[yol](b, ip)

    def giris(self, b, ip):
        simdi = self.saat()
        numara, pw = b.get("numara"), b.get("parola")
        if not isinstance(numara, str) or not NUMARA_RE.fullmatch(numara) or not isinstance(pw, str):
            return _hata(400, "gecersiz", "Müşteri numarası 7 haneli olmalı.")
        if not self.kilit.attempt(ip, numara, simdi):
            return _hata(429, "kilit", "Çok fazla deneme. 15 dakika sonra tekrar deneyin.")
        m = self.veri.parola_dogrula(int(numara), pw)
        if m is None:
            if self.veri.musteri(int(numara)) is not None:
                self.veri.hareket(numara, ip, "CIHAZ_GIRIS_HATALI", int(numara))
            return _hata(401, "hatali", "Müşteri numarası veya parola hatalı.")
        self.kilit.succeed(ip, numara, simdi)
        if m["uyelik"] != "aktif":
            return _hata(403, "uyelik", "Üyeliğiniz sona ermiş. Hizmet sağlayıcınıza başvurun.")
        wg, ssh = b.get("wg_pub"), b.get("ssh_pub")
        if not (isinstance(wg, str) and kuyruk.WG_RE.fullmatch(wg) and isinstance(ssh, str) and kuyruk.SSH_RE.fullmatch(ssh)):
            return _hata(400, "gecersiz", "Cihaz anahtarları geçersiz.")
        try:
            cid, anahtar = self.veri.cihaz_bagla(m["numara"], wg, ssh, _metin(b.get("isletme_adi")),
                                                 _metin(b.get("unvan")), _metin(b.get("surum")))
        except veri_modulu.BaskaCihaz:
            self.veri.hareket(numara, ip, "CIHAZ_REDDEDILDI", m["numara"], "numara başka cihazda")
            return _hata(409, "baska_cihaz", "Bu numara başka bir cihazda kullanılıyor.")
        s = self.kuyruk_bekle(self.kuyruk_ekle("cihaz-ekle", {"numara": numara, "wg_pub": wg, "ssh_pub": ssh}))
        if not s or s.get("durum") != "tamam":
            self.veri.hareket(numara, ip, "CIHAZ_KAYIT_HATA", m["numara"], (s or {}).get("hata", "zaman aşımı"))
            return _hata(503, "kayit", "Cihaz kaydı tamamlanamadı, biraz sonra tekrar deneyin.")
        self.veri.cihaz_guncelle(cid, tunel_ip=s["tunel_ip"])
        self.veri.hareket(numara, ip, "CIHAZ_BAGLANDI", m["numara"], f"cihaz={cid} tunel={s['tunel_ip']}")
        return _json(200, dict(_parola_alanlari(m), **_lisans(self.veri, m), **_cihaz_ekleri(self.veri),
                               cihaz_anahtari=anahtar, tunel_ip=s["tunel_ip"],
                               sunucu_pub=s["sunucu_pub"], uc_nokta=s["uc_nokta"],
                               yedek_hedefi=f"wfc-{numara}@{YEDEK_SUNUCU}:"))

    def eslesme(self, b, ip):
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return _hata(401, "taninmadi", "Cihaz tanınmadı.")
        if c["durum"] == "serbest_birakiliyor":
            if b.get("temizlendi") is True:
                self.veri.temizlendi(c["id"])
                self.kuyruk_ekle("cihaz-kapat", {"numara": str(c["musteri"]), "wg_pub": c["wg_pub"]})  # sonucu beklenmez
                self.veri.hareket(f"cihaz:{c['id']}", ip, "CIHAZ_SERBEST", c["musteri"])
            return _json(200, {"durum": "serbest"})
        self.veri.eslesme_kaydet(c["id"], _metin(b.get("isletme_adi")), _metin(b.get("unvan")), _metin(b.get("surum")),
                                b.get("isletme_turu"))
        m = self.veri.musteri(c["musteri"])
        return _json(200, dict(_parola_alanlari(m), **_lisans(self.veri, m), **_cihaz_ekleri(self.veri),
                               durum="bagli", uyelik=m["uyelik"]))

    def parola(self, b, ip):
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return _hata(401, "taninmadi", "Cihaz tanınmadı.")
        if not self.kilit.attempt(ip, f"cihaz:{c['id']}", self.saat()):
            return _hata(429, "kilit", "Çok fazla deneme. 15 dakika sonra tekrar deneyin.")
        if not isinstance(b.get("eski"), str) or not isinstance(b.get("yeni"), str):
            return _hata(400, "gecersiz", "Geçersiz istek.")
        if self.veri.parola_dogrula(c["musteri"], b.get("eski")) is None:
            return _hata(401, "hatali", "Mevcut parola yanlış.")
        try:
            tuz, oz, y = self.veri.parola_koy(c["musteri"], b.get("yeni"))
        except ValueError as h:
            return _hata(400, "gecersiz", str(h))
        self.veri.hareket(str(c["musteri"]), ip, "PAROLA_DEGISTI", c["musteri"], "cihazdan")
        return _json(200, {"tuz": tuz, "ozet": oz, "yineleme": y})

    def saglik(self, b, ip):
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return _hata(401, "taninmadi", "Cihaz tanınmadı.")
        r = saglik_temizle(b.get("rapor"))
        if r is None:
            return _hata(400, "gecersiz", "Geçersiz rapor.")
        if c["durum"] != "serbest_birakiliyor":  # serbest bırakılan cihazın raporu yazılmaz
            onceki = (self.veri.saglik(c["id"]) or {}).get("rapor", {}).get("guncelleme")
            self.veri.saglik_kaydet(c["id"], r)
            g = r.get("guncelleme")
            if g is not None:
                if g["zaman"] > self.veri.musteri(c["musteri"])["guncelleme_hemen"]:
                    self.veri.guncelleme_hemen_koy(c["musteri"], 0)  # "hemen" isteği bu denemeyle tüketildi
                if g != onceki:
                    self.veri.hareket(f"cihaz:{c['id']}", ip, "GUNCELLEME_SONUC", c["musteri"],
                                      f"surum={g['surum']} sonuc={g['sonuc']}")
        yanit = {"tamam": True, "sms_etkin_mock": self.veri.sms_etkin_mock()}  # 06:00 eşitlemesini beklemesin
        m = self.veri.musteri(c["musteri"])
        if c["durum"] == "bagli" and self.veri.hizmet_acik(m):  # /api/liste ile aynı koşul: indiremeyeceği sürüm bildirilmez
            yanit["listeler"] = listeler.ozet(self.liste_dizini)  # hazır engel listelerinin sürümleri; cihaz değişeni indirir
        hedef = self.veri.cihaz_hedefi(m)
        if m["guncelleme_hemen"] > 0 and (hedef is None or m["guncelleme_hemen_surum"] != hedef["surum"]
                                         or r.get("paket") == hedef["surum"]):
            # sonuç gelmeyecek (hedef yok/değişti, cihaz zaten hedefte): bayrak kalıp sonra pencereyi atlamasın
            self.veri.guncelleme_hemen_koy(c["musteri"], 0)
            hedef = dict(hedef, hemen=False) if hedef else None
        if hedef is not None:
            p = r.get("paket")
            k = self.veri.surum(p) if p else None
            if k is not None:  # geri dönüş paketi (cihazda kurulu sürüm) depoda: cihaz indirdiğini bununla doğrular
                hedef["onceki_sha256"] = k["sha256"]
            yanit["guncelleme"] = hedef
        return _json(200, yanit)

    def paket(self, b, ip):
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return _hata(401, "taninmadi", "Cihaz tanınmadı.")
        s = b.get("surum")
        m = self.veri.musteri(c["musteri"])
        if c["durum"] != "bagli" or not self.veri.hizmet_acik(m):  # bırakılan / lisansı ya da üyeliği kapalı cihaza paket verilmez
            return _hata(403, "yetki", "Bu paket için yetkiniz yok.")
        hedef = self.veri.cihaz_hedefi(m)
        rapor = (self.veri.saglik(c["id"]) or {}).get("rapor", {})
        if not (isinstance(s, str) and surumler.SURUM_RE.fullmatch(s)
                and (s == (hedef or {}).get("surum") or s == rapor.get("paket"))):  # ikincisi geri dönüş paketi için
            return _hata(403, "yetki", "Bu paket için yetkiniz yok.")
        try:
            with open(surumler.paket_yolu(self.surum_dizini, s), "rb") as f:
                icerik = f.read()
        except OSError:
            return _hata(404, "yok", "Paket bulunamadı.")
        return 200, {"Content-Type": "application/octet-stream", "Cache-Control": "no-store"}, icerik

    def liste(self, b, ip):
        """Hazır engel listesi (gzip, düz metnin sha256'sı sağlık cevabında). Yalnızca bağlı ve hizmeti açık cihaza."""
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return _hata(401, "taninmadi", "Cihaz tanınmadı.")
        k = b.get("kategori")
        if not (isinstance(k, str) and k in listeler.KATEGORILER):
            return _hata(400, "gecersiz", "Bilinmeyen kategori.")
        if c["durum"] != "bagli" or not self.veri.hizmet_acik(self.veri.musteri(c["musteri"])):
            return _hata(403, "yetki", "Bu liste için yetkiniz yok.")
        try:
            with open(listeler.liste_yolu(self.liste_dizini, k), "rb") as f:
                icerik = f.read()
        except OSError:
            return _hata(404, "yok", "Liste bulunamadı.")
        return 200, {"Content-Type": "application/gzip", "Cache-Control": "no-store"}, icerik

    # --- doğrulama SMS'i merkezden (2026-10-13): NetGSM OTP ya da Twilio Verify ---
    def _sms_cihazi(self, b):
        """(cihaz, müşteri, telefon, kod, None) ya da (.., hata yanıtı). Kod hiçbir yere yazılmaz."""
        c = self.veri.cihaz_anahtarla(b.get("cihaz_anahtari"))
        if c is None:
            return None, _hata(401, "taninmadi", "Cihaz tanınmadı.")
        m = self.veri.musteri(c["musteri"])
        if c["durum"] != "bagli" or not self.veri.hizmet_acik(m):
            return None, _hata(403, "yetki", "Bu işlem için yetkiniz yok.")
        tel, kod = b.get("telefon"), b.get("kod")
        if not (isinstance(tel, str) and tel.isascii() and TELEFON_RE.fullmatch(tel)
                and isinstance(kod, str) and kod.isascii() and OTP_KOD_RE.fullmatch(kod)):
            return None, _hata(400, "gecersiz", "Geçersiz istek.")
        return (c, tel, kod), None

    @staticmethod
    def _saglayici(s, tel):
        """Türk cep numarası → tr_saglayici; yabancı → Twilio açıksa twilio, değilse None."""
        if twilio.tr_mi(tel):
            return s["tr_saglayici"]
        return "twilio" if s["twilio"]["acik"] else None

    def sms(self, b, ip):
        t, h = self._sms_cihazi(b)
        if h:
            return h
        c, tel, kod = t
        s = self.veri.sms_ayari()

        def sonuc(saglayici, sonuc_, kod_, is_no=""):
            if sonuc_ != "mock":  # müşteri başına tek satır (her SMS'e hareket satırı geçmişi boğardı); sağlık izleme okur
                self.veri.sms_son_kaydet(c["musteri"], saglayici, sonuc_, kod_)
            y = {"sonuc": sonuc_, "kod": kod_}
            if is_no:
                y["is"] = is_no
            if saglayici:
                y["saglayici"] = saglayici
            return _json(200, y)
        if s is None:
            return sonuc(None, "hata", "AYAR")
        saglayici = self._saglayici(s, tel)
        if saglayici is None:
            return sonuc(None, "hata", "YABANCI")
        if s["mock"]:  # deneme modu: hiçbir sağlayıcıya istek gitmez, cihaz kodu yerelde doğrular
            return sonuc(saglayici, "mock", "0")
        if self._sinir_doldu(c["musteri"], not twilio.tr_mi(tel)):
            return sonuc(saglayici, "hata", "SINIR")
        if saglayici == "twilio":
            r, k = twilio.gonder(s["twilio"], tel, ac=self.sms_ac)
            return sonuc(saglayici, r, k)
        r, k, is_no = netgsm.otp_gonder(s, tel[2:], OTP_SABLON.format(kod=kod), ac=self.sms_ac)
        return sonuc(saglayici, r, k, is_no)

    def _sinir_doldu(self, musteri, yabanci):
        """Günlük sayaçlar (toplam 300; yabancı numaraysa ayrıca 20). Dolmadıysa hepsi bir artar.
        Yarış: iki istek aynı anda sınırın bir altını görürse artırma sonrası sınırı aşan reddedilir (ikinci denetim)."""
        gun = self.veri.bugun().isoformat()
        sayaclar = [(gun, SMS_GUNLUK_SINIR)]
        if yabanci:
            sayaclar.append((gun + YABANCI_SAYAC, SMS_YABANCI_SINIR))
        if any(self.veri.sms_sayac(musteri, g) >= s for g, s in sayaclar):
            return True
        return any([self.veri.sms_sayac_artir(musteri, g) > s for g, s in sayaclar])  # liste: hepsi artsın

    def sms_dogrula(self, b, ip):
        """Twilio Verify kodu: {onay} ya da ağ hatasında hata/AG (misafirin deneme hakkı düşmesin diye ayrı)."""
        t, h = self._sms_cihazi(b)
        if h:
            return h
        c, tel, kod = t
        s = self.veri.sms_ayari()
        if s is None or s["mock"] or self._saglayici(s, tel) != "twilio":
            return _json(200, {"sonuc": "hata", "kod": "AYAR"})
        onay, k = twilio.dogrula(s["twilio"], tel, kod, ac=self.sms_ac)
        if onay is None:
            return _json(200, {"sonuc": "hata", "kod": k})
        return _json(200, {"onay": onay})
