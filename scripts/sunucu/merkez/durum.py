#!/usr/bin/env python3
"""wc-durum (root, 5 dk'da bir): arşivlerin ve tünelin durumu → /var/lib/wificorrect/durum.json (root:wcpanel 640).
Panel /srv'yi ve WireGuard'ı göremez; müşteri listesindeki son gelen gün, arşiv boyutu ve tünel bilgisi buradan gelir.
Root, panelin veritabanına dokunmaz: arşivler klasörlerden ve kayit.csv'den bulunur."""
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import common  # noqa: E402

KAYIT = "/etc/wireguard/wificorrect/kayit.csv"
YENI = "/srv/wificorrect-arsiv"
ESKI = "/srv/hotspot-arsiv"
DURUM = "/var/lib/wificorrect/durum.json"
AD_RE = re.compile(r"[a-z0-9][a-z0-9._-]{0,31}")
GUN_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


def _kayit(kayit):
    """kayit.csv → {arşiv adı: (tur, wg açık anahtarı)} (yalnızca cihaz/bitti satırları)."""
    out = {}
    try:
        with open(kayit, encoding="utf-8") as f:
            for s in f.read().splitlines():
                p = s.split(";")
                if len(p) >= 5 and p[0] in ("cihaz", "bitti") and p[4].startswith("wfc-"):
                    out[p[4][4:]] = (p[0], p[3])
    except FileNotFoundError:
        pass
    return out


def arsivler(kayit=KAYIT, yeni=YENI, eski=ESKI):
    """{ad: {tur, pub, kaynaklar}}: yeni arşiv (cihaz yedekleri) önce; aynı ad eski arşivde de varsa ikinci kaynak olur."""
    k, out = _kayit(kayit), {}
    if os.path.isdir(yeni):
        for ad in sorted(os.listdir(yeni)):
            d = os.path.join(yeni, ad, "veri")
            if AD_RE.fullmatch(ad) and not os.path.islink(os.path.join(yeni, ad)) and os.path.isdir(d):
                tur, pub = k.get(ad, ("bitti", ""))
                out[ad] = {"tur": tur, "pub": pub, "kaynaklar": [d]}
    if os.path.isdir(eski):
        for ad in sorted(os.listdir(eski)):
            d = os.path.join(eski, ad)
            if AD_RE.fullmatch(ad) and not os.path.islink(d) and os.path.isdir(d):
                out.setdefault(ad, {"tur": "eski", "pub": "", "kaynaklar": []})["kaynaklar"].append(d)
    return out


def el_sikismalari(metin):
    out = {}
    for s in metin.splitlines():
        p = s.split("\t")
        if len(p) == 2 and p[1].isdigit() and int(p[1]) > 0:
            out[p[0]] = int(p[1])
    return out


def son_gun(kaynaklar):
    gunler = []
    for k in kaynaklar:
        g = os.path.join(k, "gunluk")
        if os.path.isdir(g) and not os.path.islink(g):
            gunler += [d for d in os.listdir(g) if GUN_RE.fullmatch(d)]
    return max(gunler) if gunler else None


def boyut(kaynaklar):
    # ponytail: her çalıştırmada tüm arşivi tarar; yüzlerce müşteride günde bire indirilir
    toplam = 0
    for k in kaynaklar:
        for kok, _, dosyalar in os.walk(k):
            for d in dosyalar:
                try:
                    toplam += os.lstat(os.path.join(kok, d)).st_size
                except OSError:
                    pass
    return toplam


def _wg(calistir=subprocess.run):
    """`wg show` çıktısı; çalışmadıysa (komut yok, arayüz yok, zaman aşımı) None."""
    try:
        r = calistir(["wg", "show", "wg0", "latest-handshakes"], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return r.stdout if r.returncode == 0 else None


def main(kayit=KAYIT, yeni=YENI, eski=ESKI, cikti=DURUM, wg=None, simdi=None, grup="wcpanel"):
    simdi = time.time() if simdi is None else simdi
    metin = _wg() if wg is None else wg
    el = el_sikismalari(metin or "")
    out = {ad: {"tur": a["tur"], "son_gun": son_gun(a["kaynaklar"]), "boyut": boyut(a["kaynaklar"]),
                "el_sikisma": el.get(a["pub"]) if a["pub"] else None}
           for ad, a in arsivler(kayit, yeni, eski).items()}
    # wg_ok: tünel bilgisi okundu mu (wc-saglik "hepsi ulaşılamıyor"u "okunamadı"dan böyle ayırır)
    common.save_json(cikti, {"zaman": common.now_iso(simdi), "wg_ok": metin is not None, "arsivler": out})
    os.chmod(cikti, 0o640)
    if grup:
        shutil.chown(cikti, group=grup)
    return 0


if __name__ == "__main__":
    sys.exit(main())
