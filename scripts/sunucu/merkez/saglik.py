#!/usr/bin/env python3
"""wc-saglik (wcpanel, 5 dk'da bir; spec 2026-10-11): cihaz sağlık kuralları → sorun kaydı → hizmet sağlayıcıya SMS.
El sıkışma ve son yedek günü root'un yazdığı durum.json'dan, son rapor veritabanından okunur.
SMS kuralı: kritik/uyarı sorun 600 sn açık kalınca tek SMS; düzelince 600 sn temiz kalırsa tek "düzeldi" SMS'i (bu sürede
geri dönerse aynı olay sürer); bilgi önemliler 09:00'da toplu; günlük üst sınır dolunca tek "üst sınır" SMS'i;
izleme çalışamazsa günde bir "izleme çalışmıyor" SMS'i. Yeniden deneme yok."""
import datetime
import json
import os
import sys
import time
import traceback

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import common  # noqa: E402
import durum as durum_modulu  # noqa: E402
import merkez  # noqa: E402
import netgsm  # noqa: E402
import veri as veri_modulu  # noqa: E402
import yonetim  # noqa: E402

ULASILAMIYOR_SN = yonetim.CEVRIMICI_SN  # yönetim listesindeki "çevrimiçi" ile aynı eşik
RAPOR_SN = 900
SMS_BEKLE_SN = 600
TEKRAR_SN = 600  # bildirilmiş sorun bu kadar temiz kalmadan "düzeldi" denmez; içinde dönerse aynı olay (titreme)
DISK_ESIK = 90
LISANS_GUN = 15
DURUM_BAYAT_SN = 900  # wc-durum 5 dk'da bir yazar; daha eskiyse el sıkışmalar bilinmez
BILGI_SAAT = 9
BILGI_PENCERE_SN = 600  # 09:00–09:10: timer kayması bir turu atlatsa da bilgi SMS'i o gün gider
GONDERILDI = ("gonderildi", "mock")
ULASILAMIYORKEN_BILINMEZ = ("rapor_yok", "kayit_durdu", "servis_durdu")  # cihaza ulaşılamazken bilinmez
SERVIS_ADLARI = ("portal", "panel", "dnsmasq", "guvenlik")  # kaydedici ayrı: kayit_durdu
IZLEME_METNI = "WifiCorrect: izleme çalışmıyor ({})"
SINIR_METNI ="WifiCorrect: günlük SMS üst sınırına ({}) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."


def _epoch(iso):
    try:
        return datetime.datetime.fromisoformat(iso).timestamp()
    except (TypeError, ValueError):
        return None


def _bugun(simdi):
    return datetime.datetime.fromtimestamp(simdi, common.TZ).date()


def kapsam(m, c, simdi):
    """None: uyarı yok (cihaz yok/serbest bırakılıyor, askıda, üyelik bitti); "lisans": lisansı bitmiş, yalnızca lisans
    bilgisi; "tam": bütün kurallar."""
    if c is None or c["durum"] != "bagli" or m["askida"] or m["uyelik"] != "aktif":
        return None
    if m["lisans_bitis"] and m["lisans_bitis"] < _bugun(simdi).isoformat():
        return "lisans"
    return "tam"


def sorunlar(m, c, rapor, rapor_zaman, el_sikisma, son_gun, simdi, arsiv_var=True):
    """Saf kurallar. Dönen: [(tur, metin, onem)]. arsiv_var=False: durum.json'da cihazın kaydı yok (yeni bağlandı, arşivi
    henüz yok): tünel bilinmez → ulaşılamıyor/rapor yok açılmaz; yedek gecikmesi bağlanma gününden sayılır."""
    k = kapsam(m, c, simdi)
    if k is None:
        return []
    out = []
    if k == "tam":
        ulasilamiyor = arsiv_var and (el_sikisma is None or simdi - el_sikisma > ULASILAMIYOR_SN)
        if ulasilamiyor:
            out.append(("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik"))
        elif arsiv_var and rapor_zaman is not None and simdi - rapor_zaman > RAPOR_SN:  # hiç rapor yoksa (eski sürüm) açılmaz
            out.append(("rapor_yok", "Rapor gelmiyor", "kritik"))
        if rapor and not ulasilamiyor:
            sv = rapor.get("servisler") or {}
            if not sv.get("kaydedici") or rapor.get("kayit_boslugu_1s"):
                out.append(("kayit_durdu", "5651 kaydı durdu", "kritik"))
            durdu = [a for a in SERVIS_ADLARI if not sv.get(a)]
            if durdu:
                out.append(("servis_durdu", "Servis durdu: " + ", ".join(durdu), "kritik"))
        if yonetim.yedek_gecikti(son_gun if arsiv_var else (c["baglanma"] or "")[:10], simdi):
            out.append(("yedek_gecikti", "Yedek gecikti", "uyari"))
        if rapor and isinstance(rapor.get("disk_yuzde"), int) and rapor["disk_yuzde"] >= DISK_ESIK:
            out.append(("disk_dolu", f"Disk dolu (%{rapor['disk_yuzde']})", "uyari"))
        if rapor and rapor.get("sms_sorunu"):
            out.append(("sms_calismiyor", "SMS çalışmıyor", "uyari"))
    try:
        bitis = datetime.date.fromisoformat(m["lisans_bitis"])
    except (TypeError, ValueError):
        bitis = None
    if bitis is not None:
        kalan, tarih = (bitis - _bugun(simdi)).days, bitis.strftime("%d.%m.%Y")
        if kalan < 0:
            out.append(("lisans_bitti", f"Lisans bitti ({tarih})", "bilgi"))
        elif kalan <= LISANS_GUN:
            out.append(("lisans_bitiyor", f"Lisans bitiyor ({tarih})", "bilgi"))
    return out


def _kim(m, c):
    ad = (c["isletme_adi"] if c is not None else "").strip()
    return f"{m['numara']} {ad}" if ad else str(m["numara"])


class Uyari:
    """SMS kararının gönderim kısmı: uyarı kapalıysa hiçbir şey yapmaz; günlük sınırı sayar; her SMS bir hareket."""

    def __init__(self, veri, gonder, simdi):
        self.veri, self.gonder, self.simdi = veri, gonder, simdi
        self.ayar = veri.uyari_ayari()
        self.acik = bool(self.ayar["acik"] and self.ayar["telefonlar"])
        self.gun = _bugun(simdi).isoformat()
        self.sms = veri.sms_ayari() if self.acik else None

    def _yolla(self, metin):
        try:
            return self.gonder(self.sms, metin, self.ayar["telefonlar"])
        except Exception:  # gönderici ne olursa olsun tur sürer
            return "hata:AG"

    def _gonder(self, metin):
        """Dönen: gonderildi | mock | hata:<kod> | sinir. Üst sınır bildirimi de sayaca girer, günde bir kez gider."""
        sinir = int(self.ayar["gunluk_sinir"])
        if self.veri.sms_sayaci(self.gun) >= sinir:
            return "sinir"
        if self.veri.sms_say(self.gun) >= sinir:  # son hak üst sınır bildiriminin
            r = self._yolla(SINIR_METNI.format(sinir))
            self.veri.hareket("saglik", "-", "UYARI_SMS", None, f"üst sınır: {r}")
            return "sinir"
        return self._yolla(metin)

    def _kaydet(self, s, alan, r, ne):
        self.veri.sorun_sms(s["id"], alan, r)
        if r != "sinir":
            self.veri.hareket("saglik", "-", "UYARI_SMS", s["musteri"], f"{s['tur']} {ne}: {r}")

    def acilis(self, m, c, s):
        if self.acik:
            saat = datetime.datetime.fromisoformat(s["acilis"]).astimezone(common.TZ).strftime("%H:%M")
            self._kaydet(s, "acilis_sms", self._gonder(f"WifiCorrect {_kim(m, c)}: {s['metin']} ({saat}'ten beri)"), "açılış")

    def kapanis(self, m, c, s):
        """Bekletilmiş "düzeldi" SMS'i; süre açılıştan son kapanışa. Uyarı kapalıysa "yok" (sonradan gitmez)."""
        if not self.acik:
            self.veri.sorun_sms(s["id"], "kapanis_sms", "yok")
        else:
            dk = max(1, round(((_epoch(s["kapanis"]) or self.simdi) - (_epoch(s["acilis"]) or self.simdi)) / 60))
            self._kaydet(s, "kapanis_sms", self._gonder(f"WifiCorrect {_kim(m, c)}: {s['metin']} düzeldi ({dk} dk sürdü)"), "kapanış")

    def izleme_bozuk(self, kod, neden):
        """İzleme çalışamadı: neden türü (kod) başına günde bir SMS; biri ötekini yutmaz. Aynı gönderici; açık/kapalı
        ve günlük sınır geçerli."""
        print(f"wc-saglik: izleme çalışmıyor: {neden}", file=sys.stderr)
        if self.acik and self.veri.gunluk_bayrak(f"izleme_sms:{kod}", self.gun):
            r = self._gonder(IZLEME_METNI.format(neden))
            self.veri.hareket("saglik", "-", "UYARI_SMS", None, f"izleme çalışmıyor ({neden}): {r}")

    def bilgi_toplu(self, musteriler, cihazlar):
        """TR 09:00 turunda bildirilmemiş bütün bilgi sorunları tek SMS'te."""
        t = datetime.datetime.fromtimestamp(self.simdi, common.TZ)
        dokuz = t.replace(hour=BILGI_SAAT, minute=0, second=0, microsecond=0)
        if not self.acik or not 0 <= (t - dokuz).total_seconds() < BILGI_PENCERE_SN:
            return
        bekleyen = [s for s in self.veri.acik_sorunlar() if s["onem"] == "bilgi" and not s["acilis_sms"]
                    and s["musteri"] in musteriler]
        if not bekleyen:
            return
        parca = [f"{_kim(musteriler[s['musteri']], cihazlar.get(s['musteri']))}: {s['metin']}"
                 for s in bekleyen]
        r = self._gonder("WifiCorrect bilgi: " + "; ".join(parca))
        for s in bekleyen:
            self._kaydet(s, "acilis_sms", r, "bilgi")


def _musteri_turu(veri, u, m, c, d, simdi):
    """d: müşterinin durum.json kaydı (yoksa None), el sıkışması durum.json zamanına göre kaydırılmış."""
    n = m["numara"]
    rapor = rz = None
    if c is not None:
        s = veri.saglik(c["id"])
        if s:
            rapor, rz = s["rapor"], _epoch(s["zaman"])
    d_ = d or {}
    yeni = {t: (metin, onem) for t, metin, onem in
            sorunlar(m, c, rapor, rz, d_.get("el_sikisma"), d_.get("son_gun"), simdi, arsiv_var=d is not None)}
    acik = {s["tur"]: s for s in veri.acik_sorunlar(n)}
    tam = kapsam(m, c, simdi) == "tam"
    for tur, s in acik.items():
        if tur in yeni or ("ulasilamiyor" in yeni and tur in ULASILAMIYORKEN_BILINMEZ):  # bilinmiyor: düzeldi sayılmaz
            continue
        veri.sorun_kapat(s["id"])  # bildirilmişse "düzeldi" TEKRAR_SN temiz kalınca gider (aşağıda)
        if not tam and s["onem"] != "bilgi" and s["acilis_sms"] in GONDERILDI:
            veri.sorun_sms(s["id"], "kapanis_sms", "yok")  # askıya alındı/serbest: sessiz kapanış
    for tur, (metin, onem) in yeni.items():
        if tur in acik:
            continue
        once = veri.son_kapanan(n, tur)
        if (once and onem != "bilgi" and once["acilis_sms"] in GONDERILDI and not once["kapanis_sms"]
                and simdi - (_epoch(once["kapanis"]) or 0) < TEKRAR_SN):
            veri.sorun_yeniden_ac(once["id"])  # "düzeldi" denmeden döndü: aynı olay sürüyor, yeni SMS yok
        else:
            veri.sorun_ac(n, tur, metin, onem)
    for s in veri.kapanis_bekleyenler(n):
        if not tam:
            veri.sorun_sms(s["id"], "kapanis_sms", "yok")
        elif simdi - (_epoch(s["kapanis"]) or simdi) >= TEKRAR_SN:
            u.kapanis(m, c, s)
    for tur, s in acik.items():
        if (tur in yeni and s["onem"] != "bilgi" and not s["acilis_sms"]
                and simdi - (_epoch(s["acilis"]) or simdi) >= SMS_BEKLE_SN):
            u.acilis(m, c, s)


def _ulasilamaz(d, zaman):
    el = d.get("el_sikisma")
    return not isinstance(el, (int, float)) or zaman - el > ULASILAMIYOR_SN


def calistir(veri, durum, gonder=netgsm.gonder, simdi=None):
    simdi = time.time() if simdi is None else simdi
    u = Uyari(veri, gonder, simdi)
    zaman = _epoch(durum.get("zaman")) if isinstance(durum, dict) else None
    # wc-durum çalışmıyorsa her cihaz "ulaşılamıyor" görünürdü: hiçbir sorun açılıp kapanmaz, günde bir SMS
    if zaman is None:
        return u.izleme_bozuk("durum_yok", "durum.json yok")
    if simdi - zaman > DURUM_BAYAT_SN:
        return u.izleme_bozuk("durum_bayat", "durum.json bayat")
    arsivler = durum.get("arsivler") if isinstance(durum.get("arsivler"), dict) else {}
    cihazlar = veri.cihazlar()
    musteriler = {m["numara"]: m for m in veri.musteriler()}
    kayit = {n: (arsivler.get(str(n)) if isinstance(arsivler.get(str(n)), dict) else None) for n in musteriler}
    # kaydı olmayan (yeni bağlanmış, arşivi henüz yok) cihazın tüneli bilinmez: toplu kopma hesabına girmez
    izlenen = [n for n, m in musteriler.items() if kapsam(m, cihazlar.get(n), simdi) == "tam" and kayit[n] is not None]
    if len(izlenen) >= 2 and all(_ulasilamaz(kayit[n], zaman) for n in izlenen):
        # kaydı olan hepsi birden düştü: büyük olasılıkla `wg show` okunamadı (ya da merkezin ağı); SMS bütçesi yakılmaz
        return u.izleme_bozuk("tunel_okunamadi", "tünel bilgisi okunamadı")
    hatalar = 0
    for n, m in musteriler.items():
        d = kayit[n]
        if d is not None and isinstance(d.get("el_sikisma"), (int, float)):
            d = {**d, "el_sikisma": d["el_sikisma"] + (simdi - zaman)}  # yaş, durum.json'un yazıldığı ana göre
        try:
            _musteri_turu(veri, u, m, cihazlar.get(n), d, simdi)
        except Exception as h:  # bir müşterinin bozuk verisi ötekileri durdurmasın
            hatalar += 1
            print(f"wc-saglik: {n}: {type(h).__name__}: {h}", file=sys.stderr)
    u.bilgi_toplu(musteriler, cihazlar)
    if hatalar:
        u.izleme_bozuk("musteri_hatasi", f"{hatalar} müşteride hata")


def main(db=merkez.DB, durum_yolu=durum_modulu.DURUM, gonder=netgsm.gonder):
    try:
        with open(durum_yolu, encoding="utf-8") as f:
            d = json.load(f)
    except (OSError, ValueError):
        d = {}
    veri = None
    try:
        veri = veri_modulu.Veri(db)
        calistir(veri, d, gonder)
        return 0
    except Exception as h:
        traceback.print_exc()
        try:  # en iyi çaba: haber ver, burada da düşme
            if veri is not None:
                Uyari(veri, gonder, time.time()).izleme_bozuk("hata", f"hata: {type(h).__name__}")
        except Exception:
            traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
