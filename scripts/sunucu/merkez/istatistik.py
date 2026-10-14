#!/usr/bin/env python3
"""wc-istatistik: yönetim merkezi sayıları ve gün özetleri.
Her arşivin (wc-durum'un listesi: cihaz yedekleri + eski arşivler) eksiksiz gelmiş her günü için o gün internete çıkan
farklı telefon sayısını /var/lib/wificorrect/istatistik/<arşiv>.json'a yazar. Root çalışır; panel yalnızca bu dosyaları okur."""
import json
import os
import re
import stat
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import common  # noqa: E402

CIKTI = "/var/lib/wificorrect/istatistik"
DETAY = "/var/lib/wificorrect/detay"
RAPOR = "/var/lib/wificorrect/rapor"
RAPOR_HAM = "/var/lib/wificorrect/rapor-ham"
RAPOR_GRUPLARI = "/var/lib/wificorrect/merkez/rapor-gruplari.json"
PANEL_GRUBU = "wcpanel"  # özetler root:wcpanel 640; testlerde None (sahiplik değişmez)
GUN_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


def tamam_mi(gun_dizini):
    """MANIFEST var ve listelediği her dosya yerinde mi (rsync yarıdayken hayır). MANIFEST normal dosya değilse
    (sembolik bağ, FIFO, klasör), okunamıyorsa ya da UTF-8 değilse hayır: root beklemesin, bozuk bir gün diğerlerini
    durdurmasın."""
    yol = os.path.join(gun_dizini, "MANIFEST.sha256")
    try:
        fd = os.open(yol, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
                     | getattr(os, "O_BINARY", 0))
    except OSError:
        return False
    try:
        with os.fdopen(fd, "rb") as f:
            if os.path.islink(yol) or not stat.S_ISREG(os.fstat(f.fileno()).st_mode):
                return False
            metin = f.read().decode("utf-8")
    except (OSError, UnicodeDecodeError):
        return False
    metin = metin.replace("\r\n", "\n").replace("\r", "\n")  # metin kipindeki satır sonu çevirisiyle aynı
    satirlar = [s.split(None, 1) for s in metin.split("\n") if s.strip() and not s.startswith("#")]
    return all(len(p) == 2 and os.path.exists(os.path.join(gun_dizini, p[1].strip())) for p in satirlar)


def normal_dosya(yol):
    """Yol yoksa None; varsa ve normal dosya değilse (cihazdan gelmiş sembolik bağ/FIFO) ValueError:
    başka arşivi okutmasın, root görevini takmasın."""
    if not os.path.lexists(yol):
        return None
    if os.path.islink(yol) or not os.path.isfile(yol):
        raise ValueError(f"{os.path.basename(yol)} normal dosya değil")
    return yol


def satirlar(yol):
    yol = normal_dosya(yol)
    return common.read_rows(yol) if yol else iter(())


def farkli_kisi(gun_dizini):
    """Günün trafik.csv.gz'sindeki farklı, boş olmayan telefon sayısı (trafik dosyası yoksa 0)."""
    return len({r["telefon"] for r in satirlar(os.path.join(gun_dizini, "trafik.csv.gz")) if r.get("telefon")})


def guncelle(kafe_dizini, json_yolu):
    """Henüz hesaplanmamış tamam günleri ekler. Bozuk gün atlanır ve sonraki çalıştırmada yeniden denenir."""
    try:
        with open(json_yolu, encoding="utf-8") as f:
            sayilar = json.load(f)
        if not isinstance(sayilar, dict):
            sayilar = {}
    except (FileNotFoundError, ValueError):
        sayilar = {}
    g = os.path.join(kafe_dizini, "gunluk")
    gunler = sorted(d for d in os.listdir(g) if GUN_RE.fullmatch(d)) if os.path.isdir(g) and not os.path.islink(g) else []
    hatalar = []
    for gun in gunler:
        d = os.path.join(g, gun)
        if gun in sayilar or os.path.islink(d) or not tamam_mi(d):
            continue
        try:
            sayilar[gun] = farkli_kisi(d)
        except Exception as e:  # bozuk/kesik gz (zlib.error, EOFError, OSError): yalnızca o gün düşer
            hatalar.append(f"{gun}: {e!r}")
    os.makedirs(os.path.dirname(json_yolu) or ".", exist_ok=True)
    common.save_json(json_yolu, dict(sorted(sayilar.items())))
    os.chmod(json_yolu, 0o644)
    return hatalar


def main(kayit=None, yeni=None, eski=None, cikti=CIKTI, detay=DETAY, grup=PANEL_GRUBU,
         rapor_ham=RAPOR_HAM, rapor_cikti=RAPOR, rapor_gruplari=RAPOR_GRUPLARI, simdi=None):
    import durum  # durum → common; döngü yok ama istatistik'i hafif tutmak için burada
    import ozet  # istatistik'i içe aktarır; döngüsel içe aktarma olmasın diye burada
    import rapor  # aynı sebep
    kod = 0
    arsivler = durum.arsivler(kayit or durum.KAYIT, yeni or durum.YENI, eski or durum.ESKI)
    for ad, a in arsivler.items():
        hatalar = []
        for kaynak in a["kaynaklar"]:  # yeni arşiv önce: aynı gün eskide de varsa atlanır
            try:
                hatalar += guncelle(kaynak, os.path.join(cikti, ad + ".json"))
                hatalar += ozet.detay_guncelle(kaynak, os.path.join(detay, ad), grup)
            except Exception as e:  # bir arşivin bozuk klasörü diğerlerini durdurmasın
                hatalar.append(repr(e))
        for hata in hatalar:
            print(f"{ad}: {hata}", file=sys.stderr)
            kod = 1
    # haftalık/aylık raporlar (ham özet önbelleği + müşteri başına dönem raporları); hatası yukarıdakileri etkilemez
    try:
        hatalar = rapor.calistir(arsivler, rapor_gruplari, rapor_ham, rapor_cikti, grup, simdi)
    except Exception as e:
        hatalar = [type(e).__name__]
    for hata in hatalar:
        print(f"rapor: {hata}", file=sys.stderr)
        kod = 1
    return kod


if __name__ == "__main__":
    sys.exit(main())
