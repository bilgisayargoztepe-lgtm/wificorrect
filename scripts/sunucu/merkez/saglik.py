#!/usr/bin/env python3
"""wc-saglik (wcpanel, 5 dk'da bir; spec 2026-10-11): cihaz sağlık kuralları → sorun kaydı → hizmet sağlayıcıya SMS.
El sıkışma ve son yedek günü root'un yazdığı durum.json'dan, son rapor veritabanından okunur.
SMS kuralı: kritik/uyarı sorun 600 sn açık kalınca tek SMS, düzelince tek SMS; bilgi önemliler 09:00'da toplu;
günlük üst sınır dolunca tek "üst sınır" SMS'i. Yeniden deneme yok."""
import datetime
import json
import os
import sys
import time

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
TEKRAR_SN = 600  # bildirilmiş sorun bu süre içinde yeniden açılırsa sessiz (titreyen bağlantı)
DISK_ESIK = 90
LISANS_GUN = 15
DURUM_BAYAT_SN = 900  # wc-durum 5 dk'da bir yazar; daha eskiyse el sıkışmalar bilinmez
BILGI_SAAT = 9
BILGI_PENCERE_SN = 600  # 09:00–09:10: timer kayması bir turu atlatsa da bilgi SMS'i o gün gider
GONDERILDI = ("gonderildi", "mock")
ULASILAMIYORKEN_BILINMEZ = ("rapor_yok", "kayit_durdu", "servis_durdu")  # cihaza ulaşılamazken bilinmez
SERVIS_ADLARI = ("portal", "panel", "dnsmasq", "guvenlik")  # kaydedici ayrı: kayit_durdu
SINIR_METNI = "WifiCorrect: günlük SMS üst sınırına ({}) ulaşıldı, bugün başka uyarı SMS'i gitmeyecek."


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


def sorunlar(m, c, rapor, rapor_zaman, el_sikisma, son_gun, simdi):
    """Saf kurallar. Dönen: [(tur, metin, onem)]."""
    k = kapsam(m, c, simdi)
    if k is None:
        return []
    out = []
    if k == "tam":
        ulasilamiyor = el_sikisma is None or simdi - el_sikisma > ULASILAMIYOR_SN
        if ulasilamiyor:
            out.append(("ulasilamiyor", "Cihaz ulaşılamıyor", "kritik"))
        elif rapor_zaman is not None and simdi - rapor_zaman > RAPOR_SN:  # hiç rapor yoksa (eski sürüm) açılmaz
            out.append(("rapor_yok", "Rapor gelmiyor", "kritik"))
        if rapor and not ulasilamiyor:
            sv = rapor.get("servisler") or {}
            if not sv.get("kaydedici") or rapor.get("kayit_boslugu_1s"):
                out.append(("kayit_durdu", "5651 kaydı durdu", "kritik"))
            durdu = [a for a in SERVIS_ADLARI if not sv.get(a)]
            if durdu:
                out.append(("servis_durdu", "Servis durdu: " + ", ".join(durdu), "kritik"))
        if yonetim.yedek_gecikti(son_gun, simdi):
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
            out.append(("lisans_bitiyor", f"Lisans bitti ({tarih})", "bilgi"))
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
        if self.acik:
            dk = max(1, round((self.simdi - (_epoch(s["acilis"]) or self.simdi)) / 60))
            self._kaydet(s, "kapanis_sms", self._gonder(f"WifiCorrect {_kim(m, c)}: {s['metin']} düzeldi ({dk} dk sürdü)"), "kapanış")

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
    n = m["numara"]
    rapor = rz = None
    if c is not None:
        s = veri.saglik(c["id"])
        if s:
            rapor, rz = s["rapor"], _epoch(s["zaman"])
    yeni = {t: (metin, onem) for t, metin, onem in sorunlar(m, c, rapor, rz, d.get("el_sikisma"), d.get("son_gun"), simdi)}
    acik = {s["tur"]: s for s in veri.acik_sorunlar(n)}
    tam = kapsam(m, c, simdi) == "tam"
    for tur, s in acik.items():
        if tur in yeni or ("ulasilamiyor" in yeni and tur in ULASILAMIYORKEN_BILINMEZ):  # bilinmiyor: düzeldi sayılmaz
            continue
        veri.sorun_kapat(s["id"])
        # askıya alınan/serbest bırakılan müşteride ve bilgi önemlide sessiz kapanış
        if tam and s["onem"] != "bilgi" and s["acilis_sms"] in GONDERILDI:
            u.kapanis(m, c, s)
    for tur, (metin, onem) in yeni.items():
        s = acik.get(tur)
        if s is None:
            once = veri.son_kapanan(n, tur)
            sid = veri.sorun_ac(n, tur, metin, onem)
            # bildirilmiş sorun 600 sn içinde yeniden açıldı: ne açılış ne kapanış SMS'i (titreyen bağlantı)
            if (once and onem != "bilgi" and once["acilis_sms"] in GONDERILDI + ("tekrar",)
                    and simdi - (_epoch(once["kapanis"]) or 0) < TEKRAR_SN):
                veri.sorun_sms(sid, "acilis_sms", "tekrar")
        elif onem != "bilgi" and not s["acilis_sms"] and simdi - (_epoch(s["acilis"]) or simdi) >= SMS_BEKLE_SN:
            u.acilis(m, c, s)


def calistir(veri, durum, gonder=netgsm.gonder, simdi=None):
    simdi = time.time() if simdi is None else simdi
    zaman = _epoch(durum.get("zaman")) if isinstance(durum, dict) else None
    if zaman is None or simdi - zaman > DURUM_BAYAT_SN:
        # wc-durum çalışmıyorsa her cihaz "ulaşılamıyor" görünürdü: hiçbir sorun açılıp kapanmaz
        print("wc-saglik: durum.json yok ya da bayat; değerlendirme atlandı", file=sys.stderr)
        return
    arsivler = durum.get("arsivler") if isinstance(durum.get("arsivler"), dict) else {}
    u = Uyari(veri, gonder, simdi)
    cihazlar = veri.cihazlar()
    musteriler = {m["numara"]: m for m in veri.musteriler()}
    for n, m in musteriler.items():
        d = arsivler.get(str(n))
        try:
            _musteri_turu(veri, u, m, cihazlar.get(n), d if isinstance(d, dict) else {}, simdi)
        except Exception as h:  # bir müşterinin bozuk verisi ötekileri durdurmasın
            print(f"wc-saglik: {n}: {type(h).__name__}: {h}", file=sys.stderr)
    u.bilgi_toplu(musteriler, cihazlar)


def main(db=merkez.DB, durum_yolu=durum_modulu.DURUM):
    try:
        with open(durum_yolu, encoding="utf-8") as f:
            d = json.load(f)
    except (OSError, ValueError):
        d = {}
    calistir(veri_modulu.Veri(db), d)
    return 0


if __name__ == "__main__":
    sys.exit(main())
