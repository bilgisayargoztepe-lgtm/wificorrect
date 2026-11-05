"""Geri okuma testi (Task 13a; canlıya geçişin ön koşulu): Rust'ın yazdığı dosya ağacını Python merkezinin okuyucularıyla açar.
Geri dönüş (merkez-geri-don.sh) Python merkezine dönmek demektir; o gün Python, Rust'ın yazdığı her şeyi okuyabilmelidir.

Ağaç Rust testiyle üretilir (cihazda; scripts/test-merkez.sh örnek ağacı yerel dizine kopyalar):
    WFC_ORNEK_AGAC=.superpowers/ornek-agac bash scripts/test-merkez.sh wificorrect denetle::ornek_agac
Sonra yerelde:
    py -3 -I scripts/sunucu/parite/geri_oku.py .superpowers/ornek-agac

Ağaç önce geçici bir dizine kopyalanır (Python `Veri` açılışta şemayı ve göçü çalıştırır; asıl ağaca dokunulmaz).
Python şunları çalıştırır: `veri.Veri`'nin bütün okuma yöntemleri, `kayitlar.Kayitlar`'ın sayfaları, `raporlar.Raporlar`,
`kuyruk.sonuc`, `listeler.meta_oku` ve `yonetici.oku`. Geçme ölçütü: hiçbir istisna yok ve sayılar/değerler
`ornek-beklenen.json`'daki (Rust'ın yazdıklarının) aynısı. Çıktı: her denetim için `TAMAM <ad>` ya da `HATA <ad>: <neden>`,
sonunda `GERI OKUMA TAMAM` (çıkış 0) ya da `GERI OKUMA BASARISIZ` (çıkış 1). Değer basmaz yalnızca sayı/ad.
"""
import json
import os
import shutil
import sys
import tempfile
import traceback

AYR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(AYR, "..", "merkez"))
os.environ.setdefault("WC_TEST", "1")
import guvenlik  # noqa: E402
import kayitlar  # noqa: E402
import kuyruk  # noqa: E402
import listeler  # noqa: E402
import raporlar  # noqa: E402
import veri  # noqa: E402
import yonetici  # noqa: E402

SONUCLAR = []


def denetim(ad):
    """Dekoratör: işlev istisna fırlatırsa ya da AssertionError verirse HATA yazar."""

    def sar(f):
        try:
            f()
        except Exception:  # noqa: BLE001 - hepsi raporlanır
            son = traceback.format_exc().strip().splitlines()[-1]
            print(f"HATA {ad}: {son}")
            SONUCLAR.append(False)
        else:
            print(f"TAMAM {ad}")
            SONUCLAR.append(True)
        return f

    return sar


def esit(ad, bulunan, beklenen):
    assert bulunan == beklenen, f"{ad}: {bulunan!r} != {beklenen!r}"


def main(kaynak):
    with open(os.path.join(kaynak, "ornek-beklenen.json"), encoding="utf-8") as f:
        b = json.load(f)
    kok = os.path.join(tempfile.mkdtemp(prefix="geri-oku-"), "kok")
    shutil.copytree(kaynak, kok)
    merkez = os.path.join(kok, "merkez")
    n = b["numara"]

    v = None

    @denetim("veri.Veri acilir (Rust'in yazdigi veritabani, sema + goc)")
    def _():
        nonlocal v
        v = veri.Veri(os.path.join(merkez, "merkez.db"), saat=lambda: b["saat"])  # Rust testi sahte saatle yazdi

    if v is None:
        return False

    @denetim("tablo satir sayilari")
    def _():
        for tablo, n_ in b["tablolar"].items():
            esit(tablo, v.db.execute(f"SELECT COUNT(*) FROM {tablo}").fetchone()[0], n_)

    @denetim("musteriler, parola, lisans, uyelik")
    def _():
        esit("musteriler", len(v.musteriler()), b["tablolar"]["musteri"])
        m = v.parola_dogrula(n, b["parola"])
        assert m is not None, "Rust'in yazdigi musteri parolasi Python'da dogrulanmadi"
        assert v.parola_dogrula(n, b["parola"] + "x") is None
        assert v.musteri(b["numara_bitmis"])["uyelik"] == "bitti"
        v.lisans_durumu(m)
        assert m["guncelleme_kanali"] == "test" and m["ap_parola"] and m["lisans_bitis"]

    @denetim("cihaz, anahtar, hedef")
    def _():
        c = v.bagli_cihaz(n)
        esit("cihaz id", c["id"], b["cihaz_id"])
        esit("anahtar ozeti", v.cihaz_anahtarla(b["cihaz_anahtari"])["id"], b["cihaz_id"])
        esit("cihazlar", len(v.cihazlar()), 1)
        esit("cihaz gecmisi", len(v.cihaz_gecmisi(b["numara_bitmis"])), 1)
        esit("isletme turu", c["isletme_turu"], "otel")
        esit("cihaz hedefi", v.cihaz_hedefi(v.musteri(n))["surum"], "0.4.28")

    @denetim("hareketler, sorunlar")
    def _():
        esit("hareketler", len(v.hareketler(None, 1000)), b["tablolar"]["hareket"])
        esit("musteri hareketleri", len(v.hareketler(n, 1000)), 1)
        esit("acik sorun", len(v.acik_sorunlar(n)), b["sorun_acik"])
        esit("sorun gecmisi", len(v.sorun_gecmisi(n)), b["sorun_toplam"])
        v.sorunlar_aralik(n, "2026-09-01T00:00:00+03:00", "2026-10-01T00:00:00+03:00")
        v.son_kapanan(n, "disk")
        v.kapanis_bekleyenler(n)

    @denetim("surumler ve kanal hedefleri")
    def _():
        esit("surumler", [s["surum"] for s in v.surumler()], b["surumler"])
        esit("kanal kararli", v.kanal_hedef("kararli"), b["kanal_kararli"])
        esit("kanal test", v.kanal_hedef("test"), b["kanal_test"])
        esit("en son", v.en_son_surum(), b["en_son_surum"])
        esit("kanal surumu", v.kanal_surumu("kararli"), b["en_son_surum"])

    @denetim("ayarlar: admin, sms, uyari, sayaclar")
    def _():
        a = v.admin_parolasi()
        assert guvenlik.dogru(b["admin_parola"], a["tuz"], a["ozet"], a["yineleme"]), "admin parolasi Python'da dogrulanmadi"
        s = v.sms_ayari()
        assert s["mock"] is True and s["tr_saglayici"] == "netgsm" and "twilio" in s
        assert v.sms_etkin_mock() is True
        u = v.uyari_ayari()
        esit("uyari telefonlari", u["telefonlar"], ["905321234567"])
        esit("sms sayac", v.sms_sayac(n, "2026-09-21"), b["sms_sayac"])
        esit("sms_son", {k: v.sms_son(n)[k] for k in ("saglayici", "sonuc")}, b["sms_son"])
        esit("uyari sayaci", v.sms_sayaci("2026-09-21"), b["sms_sayaci"])
        assert v.saglik_son_tur() == "2026-09-21T17:10:00+03:00"
        assert v.gunluk_bayrak("izleme", "2026-09-21") is False, "bayrak Python'da bulunamadi"

    @denetim("saglik raporu, bekleyen, eski arsivler")
    def _():
        s = v.saglik(b["cihaz_id"])
        esit("saglik", s["rapor"]["disk"], 42)
        esit("bekleyenler", [r["kimlik"] for r in v.bekleyenler()], [b["bekleyen"]])
        assert v.bekleyen_var(b["bekleyen"])
        esit("eski arsivler", sorted(v.eski_arsivler()), b["eski_arsivler"])
        esit("arsivler", v.arsivler(n), b["arsivler"])
        esit("rapor gruplari", v.rapor_gruplari(), {str(n): ["bocafe"]})

    @denetim("rapor-gruplari.json dosyasi")
    def _():
        with open(os.path.join(merkez, "rapor-gruplari.json"), encoding="utf-8") as f:
            esit("dosya", json.load(f), v.rapor_gruplari())

    k = kayitlar.Kayitlar(os.path.join(kok, "istatistik"), os.path.join(kok, "detay"), os.path.join(merkez, "kisi_sir"))

    @denetim("kayitlar: gun sayilari ve sayfalar")
    def _():
        arsivler = b["arsivler"]
        sayilar, guncel = k.sayilar(arsivler)
        beklenen_gunler = sorted({g for a in arsivler for g in b["istatistik"][a]})
        esit("istatistik gunleri", sorted(sayilar), beklenen_gunler)
        assert guncel
        gunler = k.gunler(arsivler)
        esit("detay gunleri", sorted(gunler), sorted({g for a in arsivler for g in b["detay_gunleri"][a]}))
        for g in gunler:
            assert k.gun_html(arsivler, g, "/k") is not None, f"{g} sayfasi uretilemedi"
        simdi = b["saat"]
        assert "<table>" in k.ozet_html("Ornek", arsivler, simdi, "/k")
        assert k.ara_html(arsivler, None, "/k")
        assert k.ara_html(arsivler, {"q": "5551112233", "tur": "kisi"}, "/k")

    r = raporlar.Raporlar(os.path.join(kok, "rapor"))

    @denetim("raporlar: liste ve donem raporlari")
    def _():
        liste = r.liste(n)
        esit("liste", [{"tur": x["tur"], "donem": x["donem"], "kesin": x["kesin"]} for x in liste], b["rapor_liste"])
        for x in liste:
            assert r.rapor(n, x["tur"], x["donem"]) is not None, f"{x['tur']}-{x['donem']} okunamadi"

    @denetim("kuyruk sonuclari")
    def _():
        for kimlik, durum in b["kuyruk"].items():
            s = kuyruk.sonuc(kimlik, os.path.join(kok, "kuyruk-sonuc"))
            assert s is not None and s["durum"] == durum, f"{kimlik}"
        esit("kuyruk bos", [a for a in os.listdir(os.path.join(kok, "kuyruk")) if a.endswith(".json")], [])

    @denetim("listeler: ozet ve gz")
    def _():
        d = os.path.join(merkez, "listeler")
        m = listeler.meta_oku(d, "kumar")
        esit("liste sayisi", m["sayi"], b["liste"]["kumar"])
        ozet = listeler.ozet(d)
        esit("ozet sayi", ozet["kumar"]["sayi"], b["liste"]["kumar"])
        import gzip

        with gzip.open(listeler.liste_yolu(d, "kumar"), "rt", encoding="utf-8") as f:
            esit("gz satirlari", len([s for s in f.read().splitlines() if s]), b["liste"]["kumar"])

    @denetim("yonetici.json")
    def _():
        h = yonetici.oku(os.path.join(merkez, "yonetici.json"))
        assert h is not None
        esit("kullanici", h["kullanici"], b["yonetici"]["kullanici"])
        assert guvenlik.dogru(b["yonetici"]["parola"], h["tuz"], h["ozet"], h["yineleme"]), "yonetici parolasi dogrulanmadi"

    @denetim("durum.json")
    def _():
        with open(os.path.join(kok, "durum.json"), encoding="utf-8") as f:
            d = json.load(f)
        esit("durum arsivleri", sorted(d["arsivler"]), sorted(b["durum_arsivleri"]))
        assert d["wg_ok"] in (True, False) and d["zaman"]

    return all(SONUCLAR)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Kullanım: py -3 -I scripts/sunucu/parite/geri_oku.py <ornek-agac-dizini>")
        sys.exit(2)
    ok = main(sys.argv[1])
    print("GERI OKUMA TAMAM" if ok else "GERI OKUMA BASARISIZ")
    sys.exit(0 if ok else 1)
