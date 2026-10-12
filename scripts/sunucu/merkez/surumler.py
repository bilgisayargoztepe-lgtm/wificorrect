"""Cihaz paketi (.deb) sürüm deposu: GitHub sürümlerinden doğrulayarak alır ya da elle eklenir.
Dosyalar DIZIN/<sürüm>.deb; sürüm kaydı (sha256, boyut) veritabanındadır. CLI: surumler.py ekle DOSYA SURUM"""
import hashlib
import json
import os
import re
import sys
import urllib.request

DIZIN = "/var/lib/wificorrect/merkez/surumler"
SURUM_RE = re.compile(r"\d+\.\d+\.\d+")
ETIKET_RE = re.compile(r"v(\d+\.\d+\.\d+)")
SHA_RE = re.compile(r"[0-9a-f]{64}")
EN_COK_BAYT = 50 * 1024 * 1024
LISTE_SINIRI = 2 * 1024 * 1024
SUMS_SINIRI = 64 * 1024
GITHUB_LISTE = "https://api.github.com/repos/bilgisayargoztepe-lgtm/wificorrect/releases?per_page=10"


def paket_yolu(dizin, surum):
    if not SURUM_RE.fullmatch(surum):
        raise ValueError("Geçersiz sürüm.")
    return os.path.join(dizin, surum + ".deb")


def getir(url, sinir_bayt):
    """Gerçek indirici: yönlendirmeleri izler, okurken sınırı uygular."""
    istek = urllib.request.Request(url, headers={"User-Agent": "wificorrect-merkez"})
    with urllib.request.urlopen(istek, timeout=60) as y:
        veri = y.read(sinir_bayt + 1)
    if len(veri) > sinir_bayt:
        raise ValueError("Dosya çok büyük.")
    return veri


def _sha_satiri(sums, ad):
    for satir in sums.decode("utf-8", "replace").splitlines():
        parca = satir.split()
        if len(parca) == 2 and parca[1].lstrip("*") == ad and SHA_RE.fullmatch(parca[0].lower()):
            return parca[0].lower()
    return None


def _yaz(dizin, surum, icerik):
    os.makedirs(dizin, exist_ok=True)
    yol = paket_yolu(dizin, surum)
    gecici = yol + ".tmp"
    with open(gecici, "wb") as f:
        f.write(icerik)
    os.chmod(gecici, 0o644)
    os.replace(gecici, yol)


def github_al(veri, dizin, getir=getir):
    """GitHub'daki son sürümlerden depoda olmayanları doğrulayıp ekler. Dönen: (eklenen sürümler, hata metinleri)."""
    eklenen, hatalar = [], []
    releaseler = json.loads(getir(GITHUB_LISTE, LISTE_SINIRI))
    for r in releaseler:
        e = ETIKET_RE.fullmatch(str(r.get("tag_name", ""))) if isinstance(r, dict) else None
        if e is None or veri.surum(e.group(1)) is not None:
            continue
        s = e.group(1)
        ad = f"wificorrect_{s}_amd64.deb"
        try:
            ek = {a.get("name"): a.get("browser_download_url") for a in r.get("assets") or [] if isinstance(a, dict)}
            if not ek.get(ad) or not ek.get("SHA256SUMS"):
                raise ValueError("paket ya da SHA256SUMS eksik")
            beklenen = _sha_satiri(getir(ek["SHA256SUMS"], SUMS_SINIRI), ad)
            if beklenen is None:
                raise ValueError("SHA256SUMS'ta paket satırı yok")
            icerik = getir(ek[ad], EN_COK_BAYT)
            if len(icerik) > EN_COK_BAYT:
                raise ValueError("paket 50 MB'tan büyük")
            if hashlib.sha256(icerik).hexdigest() != beklenen:
                raise ValueError("SHA256 tutmuyor")
            _yaz(dizin, s, icerik)
            veri.surum_ekle(s, beklenen, len(icerik), "github")
            eklenen.append(s)
        except Exception as ex:  # tek sürümün hatası diğerlerini durdurmaz
            hatalar.append(f"{s}: {ex}")
    return eklenen, hatalar


def yerel_ekle(veri, dizin, dosya, surum):
    """Elle: dosyanın sha256'sı hesaplanıp eklenir (GitHub doğrulaması yok)."""
    if not SURUM_RE.fullmatch(surum):
        raise ValueError("Geçersiz sürüm.")
    with open(dosya, "rb") as f:
        icerik = f.read(EN_COK_BAYT + 1)
    if len(icerik) > EN_COK_BAYT:
        raise ValueError("paket 50 MB'tan büyük")
    _yaz(dizin, surum, icerik)
    veri.surum_ekle(surum, hashlib.sha256(icerik).hexdigest(), len(icerik), "yerel")


def main(argv):
    if len(argv) != 4 or argv[1] != "ekle":
        print("Kullanım: surumler.py ekle DOSYA SURUM", file=sys.stderr)
        return 2
    import veri as veri_modulu
    from merkez import DB
    yerel_ekle(veri_modulu.Veri(DB), DIZIN, argv[2], argv[3])
    print("eklendi:", argv[3])
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
