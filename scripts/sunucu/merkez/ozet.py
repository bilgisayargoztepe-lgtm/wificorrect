"""Kafe paneli kişi özetleri (detay spec §2): gün başına kişi, oturum, site ve bağlantı özeti ve arama dizini.
wc-istatistik (root) çağırır; panel yalnızca üretilen JSON'ları okur."""
import json
import os
import re
import shutil

import common
from istatistik import GUN_RE, satirlar, tamam_mi

DOSYA_ADI_RE = re.compile(r"[A-Za-z0-9_]{6,40}")
ODA_RE = re.compile(r"(?:^|\s)oda=(\S+)")
SITE_SINIRI = 100
BAGLANTI_SINIRI = 50


def _sayi(s):
    try:
        return int(s)
    except (TypeError, ValueError):
        return 0


def _baglanti_sirasi(b):
    return (-(b["gonderilen"] + b["alinan"]), b["zaman"])


def _yaz(yol, veri, grup):
    common.save_json(yol, veri)
    os.chmod(yol, 0o640)
    if grup:
        shutil.chown(yol, group=grup)


def _klasor(yol, grup):
    os.makedirs(yol, exist_ok=True)
    os.chmod(yol, 0o750)
    if grup:
        shutil.chown(yol, group=grup)


def index_oku(kafe_dizini):
    """Kafe arşiv kökündeki index.csv: {telefon: satır}. Yoksa ya da normal dosya değilse boş."""
    try:
        return {r["telefon"]: r for r in satirlar(os.path.join(kafe_dizini, "index.csv")) if r.get("telefon")}
    except ValueError:
        return {}


def dosya_adi(anahtar):
    """Cihazdaki kimlik::dosya_adi: `:` → `_`, yalnızca [A-Za-z0-9_] (TC_…, PASAPORT_DE_…, telefon aynen)."""
    return "".join(c for c in anahtar.replace(":", "_") if c.isascii() and (c.isalnum() or c == "_"))


def _kayit_adi(gun_dizini, tel):
    """Gün klasöründeki kullanicilar/<dosya_adi>.csv.gz'nin KAYIT satırından (ad, soyad) ya da None."""
    k, ad = os.path.join(gun_dizini, "kullanicilar"), dosya_adi(tel)
    if not DOSYA_ADI_RE.fullmatch(ad) or os.path.islink(k):
        return None
    for r in satirlar(os.path.join(k, ad + ".csv.gz")):
        if r.get("olay") == "KAYIT" and r.get("ad"):
            return r["ad"], r.get("soyad", "")
    return None


def gun_ozeti(gun_dizini, index):
    """Bir günün kişi bazında özeti; kişiler adı olanlar önce, ad soyad sonra telefon sırasıyla."""
    kisiler = {}

    def kisi(r):
        k = kisiler.setdefault(r["telefon"], {
            "telefon": r["telefon"], "ad": "", "soyad": "", "mac": [], "oturumlar": [],
            "gonderilen": 0, "alinan": 0, "baglanti_sayisi": 0, "ilk": "", "son": "",
            "siteler": {}, "baglantilar": []})
        zaman, mac = r.get("zaman") or "", r.get("mac") or ""
        if zaman and (not k["ilk"] or zaman < k["ilk"]):
            k["ilk"] = zaman
        if zaman > k["son"]:
            k["son"] = zaman
        if mac and mac not in k["mac"]:
            k["mac"].append(mac)
        return k

    for r in satirlar(os.path.join(gun_dizini, "oturum.csv.gz")):
        if not r.get("telefon"):
            continue
        k = kisi(r)
        if r.get("ad") and not k["ad"]:
            k["ad"], k["soyad"] = r["ad"], r.get("soyad") or ""
        if (r.get("olay") or "").startswith("OTURUM_"):
            k["oturumlar"].append({"olay": r["olay"], "zaman": r.get("zaman") or "",
                                   "ic_ip": r.get("ic_ip") or "", "ek": r.get("ek") or ""})
    for r in satirlar(os.path.join(gun_dizini, "dns.csv.gz")):
        if not r.get("telefon"):
            continue
        k = kisi(r)
        alan = (r.get("alan_adi") or "").strip().lower().rstrip(".")
        if r.get("olay") == "DNS" and alan:
            zaman = r.get("zaman") or ""
            s = k["siteler"].setdefault(alan, {"alan": alan, "sayi": 0, "ilk": zaman, "son": zaman})
            s["sayi"] += 1
            s["ilk"], s["son"] = min(s["ilk"], zaman), max(s["son"], zaman)
    for r in satirlar(os.path.join(gun_dizini, "trafik.csv.gz")):
        if not r.get("telefon"):
            continue
        k = kisi(r)
        if r.get("olay") != "BAGLANTI_BITIS":
            continue
        g, a = _sayi(r.get("gonderilen_bayt")), _sayi(r.get("alinan_bayt"))
        k["gonderilen"] += g
        k["alinan"] += a
        k["baglanti_sayisi"] += 1
        b = k["baglantilar"]
        b.append({"zaman": r.get("zaman") or "", "protokol": r.get("protokol") or "",
                  "hedef_ip": r.get("hedef_ip") or "", "hedef_port": r.get("hedef_port") or "",
                  "gonderilen": g, "alinan": a, "sure_sn": _sayi(r.get("sure_sn"))})
        if len(b) > 10 * BAGLANTI_SINIRI:  # bellek sınırı: binlerce bağlantılı kişide yalnızca en büyükler kalır
            b.sort(key=_baglanti_sirasi)
            del b[BAGLANTI_SINIRI:]
    sonuc = []
    for tel, k in kisiler.items():
        if not k["ad"]:
            ad = _kayit_adi(gun_dizini, tel)
            if ad is None and tel in index:
                ad = (index[tel].get("ad") or "", index[tel].get("soyad") or "")
            if ad:
                k["ad"], k["soyad"] = ad
        siteler = sorted(k["siteler"].values(), key=lambda s: (-s["sayi"], s["alan"]))
        k["siteler"], k["diger_site_sayisi"] = siteler[:SITE_SINIRI], max(0, len(siteler) - SITE_SINIRI)
        k["baglantilar"] = sorted(k["baglantilar"], key=_baglanti_sirasi)[:BAGLANTI_SINIRI]
        k["oturumlar"].sort(key=lambda o: o["zaman"])
        sonuc.append(k)
    sonuc.sort(key=lambda k: (0 if k["ad"] else 1, f'{k["ad"]} {k["soyad"]}'.casefold(), k["telefon"]))
    return sonuc


def odalar(k):
    """Kişinin o gün misafir koduyla açtığı oturumların odaları (oturum `ek`'indeki `oda=<no>`, büyük harf)."""
    out = set()
    for o in k.get("oturumlar") or []:
        m = ODA_RE.search(o.get("ek") or "") if isinstance(o, dict) else None
        if m:
            out.add(m.group(1).upper())
    return sorted(out)


def dizin_kaydi(ozet):
    """Arama dizini için özetten yalnızca gereken: (gün, [(telefon, ad, soyad, odalar)]) — tam özetler bellekte tutulmaz.
    Odalar özetin oturum notlarından çıkarılır (eski özet dosyaları da aynı biçimde okunur)."""
    return ozet["gun"], [(k["telefon"], k["ad"], k["soyad"], odalar(k)) for k in ozet["kisiler"]]


def kisi_dizini_guncelle(detay_dizini, yeni, grup):
    """kisiler.json: {telefon: {ad, soyad, gunler (yeni→eski), odalar {oda: [gün]}}}. `yeni`: dizin_kaydi listesi. Dizinde hiç görünmeyen
    özet günleri (görev özet ile dizin arasında kesildiyse) dosyadan tek tek okunup eklenir; özet dosyası olmayan
    günler çıkarılır. Dizin yoksa, bozuksa ya da oda bilgisi olmayan eski biçimdeyse tüm özetlerden böyle yeniden kurulur.
    Ad en yeni günden gelir."""
    yol = os.path.join(detay_dizini, "kisiler.json")
    mevcut = {a[:-5] for a in os.listdir(detay_dizini) if a.endswith(".json") and GUN_RE.fullmatch(a[:-5])}
    try:
        with open(yol, encoding="utf-8") as f:
            dizin = json.load(f)
        if not isinstance(dizin, dict):
            raise ValueError("kisiler.json sözlük değil")
        if any(not isinstance(x, dict) or not isinstance(x.get("odalar"), dict) for x in dizin.values()):
            dizin = {}  # oda alanından önceki biçim: bir kez baştan kurulur
    except (FileNotFoundError, ValueError):
        dizin = {}
    bilinen = {g for e in dizin.values() for g in e.get("gunler", [])} | {g for g, _ in yeni}
    kayitlar = list(yeni)
    for gun in sorted(mevcut - bilinen):  # ponytail: kişisiz günler her çalıştırmada yeniden okunur; küçük dosyalar
        try:
            with open(os.path.join(detay_dizini, gun + ".json"), encoding="utf-8") as f:
                kayitlar.append(dizin_kaydi(json.load(f)))
        except (OSError, ValueError, KeyError, TypeError):
            continue  # bozuk özet aramayı durdurmasın
    for gun, kisiler in sorted(kayitlar):
        for tel, ad, soyad, *oda in kisiler:
            e = dizin.setdefault(tel, {"ad": "", "soyad": "", "gunler": [], "odalar": {}})
            if ad and (not e["gunler"] or gun >= max(e["gunler"])):
                e["ad"], e["soyad"] = ad, soyad
            if gun not in e["gunler"]:
                e["gunler"].append(gun)
            for o in (oda[0] if oda else ()):
                if gun not in e["odalar"].setdefault(o, []):
                    e["odalar"][o].append(gun)
    for tel in list(dizin):
        gunler = sorted({g for g in dizin[tel].get("gunler", []) if g in mevcut}, reverse=True)
        if gunler:
            dizin[tel]["gunler"] = gunler
            o = {k: sorted(g for g in v if g in mevcut) for k, v in dizin[tel]["odalar"].items()}
            dizin[tel]["odalar"] = {k: v for k, v in sorted(o.items()) if v}
        else:
            del dizin[tel]
    _yaz(yol, dict(sorted(dizin.items())), grup)


def detay_guncelle(kafe_dizini, detay_dizini, grup=None):
    """Özetlenmemiş tamam günleri özetler, arama dizinini uzlaştırır. Bozuk gün atlanır, sonraki çalıştırmada
    yeniden denenir. Dönen: hata mesajları."""
    _klasor(detay_dizini, grup)
    g = os.path.join(kafe_dizini, "gunluk")
    gunler = sorted(d for d in os.listdir(g) if GUN_RE.fullmatch(d)) if os.path.isdir(g) and not os.path.islink(g) else []
    index, yeni, hatalar = None, [], []
    for gun in gunler:
        d, hedef = os.path.join(g, gun), os.path.join(detay_dizini, gun + ".json")
        if os.path.exists(hedef) or os.path.islink(d) or not tamam_mi(d):
            continue
        if index is None:
            index = index_oku(kafe_dizini)
        try:
            ozet = {"gun": gun, "kisiler": gun_ozeti(d, index)}
        except Exception as e:  # bozuk/kesik gz ya da normal olmayan dosya: yalnızca o gün düşer
            hatalar.append(f"detay {gun}: {e!r}")
            continue
        _yaz(hedef, ozet, grup)
        yeni.append(dizin_kaydi(ozet))
    kisi_dizini_guncelle(detay_dizini, yeni, grup)
    return hatalar
