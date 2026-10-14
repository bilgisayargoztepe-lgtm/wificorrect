#!/usr/bin/env python3
"""Haftalık ve aylık raporların root tarafı (spec 2026-10-14-aylik-rapor, plan 2026-10-14-raporlar).
Görev 1: arşiv günü başına **ham özet** (rapor-ham/<arşiv>/<gün>.json, root 700/600) ve günün bütünlük doğrulaması
(MANIFEST hash'leri + cihazdaki muhur::verify ile birebir hash zinciri). Ham özetteki misafir/izinli kimlikleri root'a
özel sırla HMAC'lenmiş takma kimliktir (tekil sayım için); telefon, TC, pasaport, MAC, IP, ad hiç yazılmaz.
Hata metinleri yalnızca gün + istisna türü taşır (dosya adı `kullanicilar/<telefon>.csv.gz` kişisel veridir)."""
import hashlib
import hmac
import json
import os
import re
import stat
import sys

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import detay  # noqa: E402
import istatistik  # noqa: E402

HAM = "/var/lib/wificorrect/rapor-ham"
SIR_YOLU = os.path.join(HAM, "sir")
BICIM = 1
ZINCIR_SINIRI = 8 * 1024 * 1024  # zincir.txt ve eski/zincir.txt.* başına; büyüğü atlanır
MANIFEST_SINIRI = 16 * 1024 * 1024
PARCA = 1 << 20
SIFIR = "0" * 64

GUN_RE = re.compile(r"\d{4}-\d{2}-\d{2}")
AD_RE = re.compile(r"[a-z0-9][a-z0-9._-]{0,31}")  # durum.AD_RE ile aynı
HEX64_RE = re.compile(r"[0-9a-f]{64}")
ZAMAN_RE = re.compile(r"\d{4}-\d{2}-\d{2}T([01]\d|2[0-3]):[0-5]\d:[0-5]\d", re.ASCII)
ETIKET_RE = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", re.ASCII)
SURUM_RE = re.compile(r"[0-9A-Za-z.+~-]{1,32}", re.ASCII)
IKINCI_DUZEY = frozenset(
    "com.tr net.tr org.tr gov.tr edu.tr k12.tr bel.tr gen.tr biz.tr info.tr web.tr tv.tr av.tr pol.tr tsk.tr "
    "co.uk org.uk ac.uk com.au co.jp com.br co.kr co.in".split())
YEREL_SONEK = ("arpa", "local", "lan", "home", "internal")


# --- alan adı kökü ---
def kok_alan(ad):
    """`www.instagram.com.` → `instagram.com`; `a.b.example.com.tr` → `example.com.tr`. IP, yerel ad, biçimsiz ad → None."""
    if not isinstance(ad, str):
        return None
    ad = ad.strip().lower()
    if ad.endswith("."):
        ad = ad[:-1]
    if not ad or len(ad) > 253:
        return None
    etiketler = ad.split(".")
    if len(etiketler) < 2 or not all(ETIKET_RE.fullmatch(t) for t in etiketler):
        return None
    if etiketler[-1].isdigit() or etiketler[-1] in YEREL_SONEK:  # IP biçimi (son etiket sayı olamaz), yerel adlar
        return None
    n = 3 if ".".join(etiketler[-2:]) in IKINCI_DUZEY else 2
    return ".".join(etiketler[-n:]) if len(etiketler) >= n else None


# --- güvenli okuma (cihazdan gelen sembolik bağ/FIFO root'u takmasın, başka dosyayı okutmasın) ---
def _ac(yol):
    """Normal dosyayı bağ izlemeden ve beklemeden açar; yoksa ya da normal dosya değilse None."""
    try:
        fd = os.open(yol, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
                     | getattr(os, "O_BINARY", 0))
    except OSError:
        return None
    try:
        if os.path.islink(yol) or not stat.S_ISREG(os.fstat(fd).st_mode):
            os.close(fd)
            return None
    except OSError:
        os.close(fd)
        return None
    return os.fdopen(fd, "rb")


def _oku_sinirli(yol, sinir):
    f = _ac(yol)
    if f is None:
        return None
    with f:
        if os.fstat(f.fileno()).st_size > sinir:
            return None
        b = f.read(sinir + 1)
    return b if len(b) <= sinir else None


def _dosya_sha(f):
    h = hashlib.sha256()
    with f:
        while True:
            p = f.read(PARCA)
            if not p:
                return h.hexdigest()
            h.update(p)


# --- zincir ---
def _zincir_dosyasi(metin):
    """Tek zincir dosyası: cihazdaki chain_lines + verify ile birebir. {gün: (manifest_sha, ok)}, aynı gün → son satır."""
    out, prev = {}, SIFIR
    for satir in metin.split("\n"):
        p = satir.strip().split(";")
        if len(p) != 3 or len(p[1]) != 64 or len(p[2]) != 64:
            continue  # yarım/bozuk satır cihazda da sayılmaz (prev'i değiştirmez)
        gun, man, zincir = p
        ok = hashlib.sha256((prev + man).encode()).hexdigest() == zincir
        prev = zincir  # bir önceki satırın YAZILI değeri (cihazdaki verify gibi)
        if GUN_RE.fullmatch(gun):
            out[gun] = (man, ok)
    return out


def zincir_oku(kaynak):
    """`kaynak/zincir.txt` + `kaynak/eski/zincir.txt*` (yeniden kurulumda eski zincir). Bir gün herhangi bir dosyada
    doğrulanıyorsa ok (o satırın manifest_sha'sı); hiçbirinde değilse ilk bulunan. Normal olmayan ya da büyük dosya okunmaz."""
    yollar = [os.path.join(kaynak, "zincir.txt")]
    eski = os.path.join(kaynak, "eski")
    if os.path.isdir(eski) and not os.path.islink(eski):
        try:
            yollar += [os.path.join(eski, a) for a in sorted(os.listdir(eski)) if a.startswith("zincir.txt")]
        except OSError:
            pass
    out = {}
    for yol in yollar:
        b = _oku_sinirli(yol, ZINCIR_SINIRI)
        if b is None:
            continue
        try:
            metin = b.decode("utf-8")
        except UnicodeDecodeError:
            continue  # cihazdaki read_to_string de bozuk UTF-8'de boş döner
        for gun, (man, ok) in _zincir_dosyasi(metin).items():
            if gun not in out or (ok and not out[gun][1]):
                out[gun] = (man, ok)
    return out


# --- gün bütünlüğü ---
def _ad_gecerli(ad):
    if not ad or "\\" in ad or "\0" in ad or ad.startswith("/") or (len(ad) > 1 and ad[1] == ":"):
        return False
    return all(p not in ("", ".", "..") for p in ad.split("/"))


def gun_dogrula(gun_dizini):
    """(sha256(MANIFEST.sha256), MANIFEST'teki her dosya yerinde ve hash'i tutuyor mu). MANIFEST yoksa (None, False).
    Hangi dosyanın tutmadığı söylenmez (kişi dosyasının adı telefondur)."""
    b = _oku_sinirli(os.path.join(gun_dizini, "MANIFEST.sha256"), MANIFEST_SINIRI)
    if b is None:
        return None, False
    man_sha = hashlib.sha256(b).hexdigest()
    try:
        metin = b.decode("utf-8")
    except UnicodeDecodeError:
        return man_sha, False
    kok = os.path.realpath(gun_dizini)
    for satir in metin.split("\n"):
        satir = satir.rstrip("\r")
        if satir.startswith("#") or not satir.strip():
            continue
        ozet, ayrac, ad = satir.partition("  ")
        if not ayrac or not HEX64_RE.fullmatch(ozet) or not _ad_gecerli(ad):
            return man_sha, False
        yol = os.path.join(gun_dizini, *ad.split("/"))
        if not os.path.realpath(yol).startswith(kok + os.sep):
            return man_sha, False
        f = _ac(yol)
        if f is None or _dosya_sha(f) != ozet:
            return man_sha, False
    return man_sha, True


def imza(gun_dizini):
    """Önbellek anahtarı: MANIFEST içeriği + gün klasöründeki her girdinin (göreli ad, boyut, mtime_ns), sıralı.
    MANIFEST ya da herhangi bir dosya değişince değişir; dosyaların kendisi okunmaz."""
    h = hashlib.sha256()
    b = _oku_sinirli(os.path.join(gun_dizini, "MANIFEST.sha256"), MANIFEST_SINIRI)
    h.update(b if b is not None else b"\0yok")
    girdiler = []
    for kok, dizinler, dosyalar in os.walk(gun_dizini, followlinks=False):
        adlar = dosyalar + [d for d in dizinler if os.path.islink(os.path.join(kok, d))]
        for a in adlar:
            yol = os.path.join(kok, a)
            try:
                st = os.lstat(yol)
            except OSError:
                continue
            girdiler.append((os.path.relpath(yol, gun_dizini).replace("\\", "/"), st.st_size, st.st_mtime_ns))
    for ad, boy, mt in sorted(girdiler):
        h.update(f"\n{ad}\0{boy}\0{mt}".encode("utf-8", "surrogateescape"))
    return h.hexdigest()


# --- ham özet ---
def takma(sir, anahtar):
    return hmac.new(sir, anahtar.encode("utf-8", "surrogateescape"), hashlib.sha256).hexdigest()[:16]


def _alan(r, k):
    v = r.get(k)
    return v if isinstance(v, str) else ""


def _saat(r):
    m = ZAMAN_RE.match(_alan(r, "zaman"))
    return int(m.group(1)) if m else None


def _sayi(s):
    return int(s) if s.isascii() and s.isdigit() else 0


def _ek_deger(ek, anahtar):
    for t in ek.split():
        if t.startswith(anahtar + "="):
            v = t[len(anahtar) + 1:]
            return v if SURUM_RE.fullmatch(v) else None
    return None


def _satirlar(gun_dizini, ad):
    return istatistik.satirlar(os.path.join(gun_dizini, ad))


def gun_ham(gun_dizini, sir):
    """Tamam (istatistik.tamam_mi) bir günün ham özeti; bkz. plan Görev 1 §4. Bozuk gz istisna fırlatır."""
    imz = imza(gun_dizini)
    man_sha, dosya_ok = gun_dogrula(gun_dizini)
    misafir, izinli, saat, alan = set(), set(), {}, {}
    giris = {"sms": 0, "kod": 0}
    gonderilen = alinan = sms = 0
    surum, surum_zaman = None, ""

    for r in _satirlar(gun_dizini, "trafik.csv.gz"):
        s = _saat(r)
        if s is None:
            continue
        tel = _alan(r, "telefon")
        if tel:
            k = takma(sir, tel)
            misafir.add(k)
            saat.setdefault(s, set()).add(k)
        elif "izinli=" in _alan(r, "ek") and _alan(r, "mac"):
            izinli.add(takma(sir, "mac:" + _alan(r, "mac")))
        if _alan(r, "olay") == "BAGLANTI_BITIS":
            gonderilen += _sayi(_alan(r, "gonderilen_bayt"))
            alinan += _sayi(_alan(r, "alinan_bayt"))

    for r in _satirlar(gun_dizini, "oturum.csv.gz"):
        if _alan(r, "olay") == "OTURUM_BASLA" and _saat(r) is not None:
            giris["kod" if "giris=misafir_kodu" in _alan(r, "ek").split() else "sms"] += 1

    for r in _satirlar(gun_dizini, "denetim.csv.gz"):
        olay = _alan(r, "olay")
        if _saat(r) is None:
            continue
        if olay == "OTP_GONDERILDI":
            sms += 1
        elif olay in ("GUNCELLEME_TAMAM", "GUNCELLEME_GERI_ALINDI"):
            v = _ek_deger(_alan(r, "ek"), "surum" if olay == "GUNCELLEME_TAMAM" else "onceki")
            z = _alan(r, "zaman")
            if v is not None and z >= surum_zaman:
                surum, surum_zaman = v, z

    for r in _satirlar(gun_dizini, "dns.csv.gz"):
        tel = _alan(r, "telefon")
        if _alan(r, "olay") != "DNS" or not tel or _saat(r) is None:
            continue
        k = kok_alan(_alan(r, "alan_adi"))
        if k:
            alan.setdefault(k, set()).add(takma(sir, tel))

    return {"bicim": BICIM, "imza": imz, "manifest_sha": man_sha, "dosya_ok": dosya_ok,
            "misafir": sorted(misafir), "saat": {str(s): sorted(saat[s]) for s in sorted(saat)},
            "giris": giris, "izinli": sorted(izinli), "gonderilen": gonderilen, "alinan": alinan, "sms": sms,
            "alan": {k: sorted(alan[k]) for k in sorted(alan)}, "surum": surum}


def _json_oku(yol):
    try:
        with open(yol, encoding="utf-8") as f:
            v = json.load(f)
    except (OSError, ValueError):
        return None
    return v if isinstance(v, dict) else None


def _yaz_600(yol, veri):
    gecici = f"{yol}.{os.getpid()}.tmp"
    fd = os.open(gecici, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(veri, f, ensure_ascii=False, separators=(",", ":"))
        f.flush()
        os.fsync(f.fileno())
    os.chmod(gecici, 0o600)
    os.replace(gecici, yol)


def _sil(yol):
    try:
        os.remove(yol)
    except FileNotFoundError:
        pass


def ham_guncelle(arsiv_adi, kaynak, ham_dizini, sir):
    """Kaynak(lar)daki her tamam gün için `ham_dizini/<arşiv>/<gün>.json`'u yoksa ya da imza/biçim değiştiyse yeniden
    hesaplar; değişmeyen gün okunmaz. `kaynak` tek yol ya da yol listesi (aynı gün iki kaynakta ise ilki geçerli).
    Bozuk gün atlanır (eski ham'ı silinir, sonraki çalıştırmada yeniden denenir); artık tamam olmayan günün ham'ı silinir.
    Dönüş: ["<gün>: <istisna türü>", …]."""
    if not isinstance(arsiv_adi, str) or not AD_RE.fullmatch(arsiv_adi):
        raise ValueError("geçersiz arşiv adı")
    kaynaklar = [kaynak] if isinstance(kaynak, str) else list(kaynak)
    hedef = os.path.join(ham_dizini, arsiv_adi)
    os.makedirs(hedef, mode=0o700, exist_ok=True)
    os.chmod(ham_dizini, 0o700)
    os.chmod(hedef, 0o700)

    gunler = {}
    for k in kaynaklar:
        g = os.path.join(k, "gunluk")
        if not os.path.isdir(g) or os.path.islink(g):
            continue
        for gun in sorted(os.listdir(g)):
            d = os.path.join(g, gun)
            if gun in gunler or not GUN_RE.fullmatch(gun) or os.path.islink(d) or not os.path.isdir(d):
                continue
            if istatistik.tamam_mi(d):
                gunler[gun] = d

    hatalar = []
    for gun, d in sorted(gunler.items()):
        yol = os.path.join(hedef, gun + ".json")
        try:
            mevcut = _json_oku(yol)
            if mevcut and mevcut.get("bicim") == BICIM and mevcut.get("imza") == imza(d):
                continue
            _yaz_600(yol, gun_ham(d, sir))
        except Exception as e:  # bozuk/kesik gz, normal olmayan dosya: yalnızca o gün düşer
            _sil(yol)
            hatalar.append(f"{gun}: {type(e).__name__}")

    for a in os.listdir(hedef):
        gun = a[:-5] if a.endswith(".json") else None
        if gun is not None and GUN_RE.fullmatch(gun) and gun not in gunler:
            _sil(os.path.join(hedef, a))
    return hatalar


def sir_al(yol=SIR_YOLU):
    """Root'a özel takma kimlik sırrı (panelin kisi_sir'i değil)."""
    os.makedirs(os.path.dirname(yol), mode=0o700, exist_ok=True)
    os.chmod(os.path.dirname(yol), 0o700)
    return detay.sir_oku_ya_uret(yol)
