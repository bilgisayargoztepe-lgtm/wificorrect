#!/usr/bin/env python3
"""Haftalık ve aylık raporların root tarafı (spec 2026-10-14-aylik-rapor, plan 2026-10-14-raporlar).
Görev 1: arşiv günü başına **ham özet** (rapor-ham/<arşiv>/<gün>.json, root 700/600) ve günün bütünlük doğrulaması
(MANIFEST hash'leri + cihazdaki muhur::verify ile birebir hash zinciri). Ham özetteki misafir/izinli kimlikleri root'a
özel sırla HMAC'lenmiş takma kimliktir (tekil sayım için); telefon, TC, pasaport, MAC, IP, ad hiç yazılmaz.
Hata metinleri yalnızca gün + istisna türü taşır (dosya adı `kullanicilar/<telefon>.csv.gz` kişisel veridir).
Görev 2: müşteri grupları (numara arşivi + panelin rapor-gruplari.json'daki bağlı eski arşivler), gün durumu ("tam" yalnızca
doğrulanmışsa) ve haftalık/aylık dönem raporları rapor/<numara>/ (root:wcpanel 750/640; yalnızca sayılar, takma kimlik yok)."""
import datetime
import hashlib
import hmac
import json
import os
import re
import shutil
import stat
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import common  # noqa: E402
import detay  # noqa: E402
import istatistik  # noqa: E402

HAM = "/var/lib/wificorrect/rapor-ham"
CIKTI = "/var/lib/wificorrect/rapor"
GRUPLAR = "/var/lib/wificorrect/merkez/rapor-gruplari.json"  # panel (wcpanel) yazar; root için güvenilmez girdi
PANEL_GRUBU = "wcpanel"
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


# ======================================================================================================================
# Görev 2: gruplar, gün durumu, dönem raporları (rapor/<numara>/, root:wcpanel 750/640, KİŞİSEL VERİ YOK)
# ======================================================================================================================
NUMARA_RE = re.compile(r"[1-9][0-9]{6}", re.ASCII)
GRUP_SINIRI = 64 * 1024
HAFTA_EN_COK, AY_EN_COK = 26, 24
KESIN_GUN = 7  # bitişten sonraki gün gelmese de bu kadar gün sonra rapor kesinleşir
ALAN_EN_AZ, ALAN_EN_COK = 3, 10  # alan adı en az 3 farklı misafir (tek kişiyi ele vermesin), ilk 10
DONEM_DOSYA_RE = re.compile(r"(?:hafta-\d{4}-W\d{2}|ay-\d{4}-\d{2})\.json")


def _tarih(gun):
    try:
        return datetime.date.fromisoformat(gun) if isinstance(gun, str) and GUN_RE.fullmatch(gun) else None
    except ValueError:
        return None


def grup_dosyasi_oku(yol):
    """Panelin yazdığı {"<numara>": [eski arşiv adı, …]}; güvenilmez: normal dosya, ≤ 64 KB, JSON sözlük. Bozuksa {}."""
    b = _oku_sinirli(yol, GRUP_SINIRI)
    if b is None:
        return {}
    try:
        d = json.loads(b.decode("utf-8"))
    except ValueError:
        return {}
    if not isinstance(d, dict):
        return {}
    return {k: [a for a in v if isinstance(a, str)] for k, v in d.items()
            if isinstance(k, str) and NUMARA_RE.fullmatch(k) and isinstance(v, list)}


def gruplar(arsivler, grup_dosyasi):
    """{numara: [(arşiv adı, kaynak), …]}: müşterinin numara arşivi önce, bağlı eski arşivler ada göre.
    Eski arşiv 7 haneli numara olamaz (başka müşterinin cihaz arşivi çekilemez), var olmalı ve tek gruba girer
    (numara sırasıyla ilk kazanır). Bağsız eski arşiv rapor almaz."""
    bag = grup_dosyasi_oku(grup_dosyasi)
    verilen, out = set(), {}
    for n in sorted({a for a in arsivler if NUMARA_RE.fullmatch(a)} | set(bag)):
        adlar = [n] if n in arsivler else []
        for ad in sorted(set(bag.get(n, ()))):
            if AD_RE.fullmatch(ad) and not NUMARA_RE.fullmatch(ad) and ad in arsivler and ad not in verilen:
                verilen.add(ad)
                adlar.append(ad)
        if adlar:
            out[n] = [(ad, k) for ad in adlar for k in arsivler[ad]["kaynaklar"]]
    return out


def _tamam(d):
    try:
        return istatistik.tamam_mi(d)
    except (OSError, ValueError):
        return False


def gun_haritasi(grup):
    """{gün: (arşiv, kaynak, gün klasörü, tamam)}: aynı gün birden çok kaynakta ise ilk tamam olan, yoksa ilki."""
    out = {}
    for ad, kaynak in grup:
        g = os.path.join(kaynak, "gunluk")
        if not os.path.isdir(g) or os.path.islink(g):
            continue
        try:
            adlar = sorted(os.listdir(g))
        except OSError:
            continue
        for gun in adlar:
            d = os.path.join(g, gun)
            if _tarih(gun) is None or os.path.islink(d) or not os.path.isdir(d):
                continue
            onceki = out.get(gun)
            if onceki is not None and onceki[3]:
                continue
            tamam = _tamam(d)
            if onceki is None or tamam:
                out[gun] = (ad, kaynak, d, tamam)
    return out


def _ham_gecerli(h):
    return (isinstance(h, dict) and h.get("bicim") == BICIM and isinstance(h.get("imza"), str)
            and all(isinstance(h.get(k), list) for k in ("misafir", "izinli"))
            and all(isinstance(h.get(k), dict) for k in ("saat", "alan", "giris")))


def _ham_yukle(ham_dizini, girdi):
    """Seçilen kaynağın ham özeti; yoksa, bozuksa ya da gün klasörü ham yazıldıktan sonra değiştiyse None."""
    ad, _kaynak, d, tamam = girdi
    if not tamam:
        return None
    h = _json_oku(os.path.join(ham_dizini, ad, os.path.basename(d) + ".json"))
    if not _ham_gecerli(h) or h["imza"] != imza(d):
        return None
    return h


def gun_durumu(gun, bugun, girdi, ham, zincir):
    """(durum, neden). `tam`'ın tek yolu: klasör tamam + ham var + dosya hash'leri tutuyor + zincir satırı doğru +
    zincirdeki manifest_sha MANIFEST'in sha'sına eşit. Emin olunamayan her şey `tam` değildir."""
    t = _tarih(gun)
    if t > bugun:
        return "gelecek", ""
    if t == bugun:
        return "bugun", ""
    if girdi is None:
        return ("bekleniyor" if t >= bugun - datetime.timedelta(days=1) else "kayit_yok"), ""
    if not girdi[3]:
        return "eksik", ""
    if ham is None:
        return "dogrulanamadi", "okunamadi"
    if ham.get("dosya_ok") is not True:
        return "dogrulanamadi", "dosya"
    z = zincir.get(gun)
    if z is None:
        return "dogrulanamadi", "zincir_yok"
    if not z[1]:
        return "dogrulanamadi", "zincir"
    if not ham.get("manifest_sha") or z[0] != ham["manifest_sha"]:
        return "dogrulanamadi", "manifest"
    return "tam", ""


# --- dönemler (ISO hafta Pzt–Paz, TR takvim ayı) ---
def hafta(t):
    """(anahtar, başlangıç, bitiş): ISO yılı ile `YYYY-Www`, Pazartesi başlar."""
    y, w, _ = t.isocalendar()
    bas = t - datetime.timedelta(days=t.weekday())
    return f"{y}-W{w:02d}", bas, bas + datetime.timedelta(days=6)


def ay(t):
    bas = t.replace(day=1)
    sonraki = (bas + datetime.timedelta(days=32)).replace(day=1)
    return f"{bas.year}-{bas.month:02d}", bas, sonraki - datetime.timedelta(days=1)


def donemler(en_eski, bugun):
    """[(tur, anahtar, baş, bit)]: haftalar sonra aylar, her biri yeniden eskiye; en çok 26 hafta ve 24 ay."""
    out = []
    for tur, f, en_cok in (("hafta", hafta, HAFTA_EN_COK), ("ay", ay, AY_EN_COK)):
        t = bugun
        for _ in range(en_cok):
            anahtar, bas, bit = f(t)
            out.append((tur, anahtar, bas, bit))
            if bas <= en_eski:
                break
            t = bas - datetime.timedelta(days=1)
    return out


def _gunler(bas, bit):
    t = bas
    while t <= bit:
        yield t
        t += datetime.timedelta(days=1)


def _int(v):
    return v if isinstance(v, int) and not isinstance(v, bool) and v >= 0 else 0


def _kume(v):
    return {x for x in v if isinstance(x, str)} if isinstance(v, list) else set()


def donem_raporu(tur, anahtar, bas, bit, bugun, simdi, grup_adlari, harita, hamlar, durumlar):
    """Bir dönemin raporu (yalnızca sayılar). `hamlar`: {gün: geçerli ham özet} (grubun bütün günleri),
    `durumlar`: {gün: (durum, neden)}. İstatistik yalnızca bugünden önceki, ham özeti olan günlerden."""
    gunler, kayit_gun, kayit_tam, s_gunler = [], 0, 0, []
    for t in _gunler(bas, bit):
        g = t.isoformat()
        durum, neden = durumlar[g]
        h = hamlar.get(g) if t < bugun else None
        if t < bugun:
            kayit_gun += 1
            kayit_tam += durum == "tam"
        if h is not None:
            s_gunler.append((t, h))
        gunler.append({"gun": g, "durum": durum, "neden": neden,
                       "farkli": len(_kume(h["misafir"])) if h is not None else None})

    farkli, izinli, alan = set(), set(), {}
    saat, hafta_gunu = [set() for _ in range(24)], [set() for _ in range(7)]
    giris_sms = giris_kod = gonderilen = alinan = sms = 0
    gunluk = []
    for t, h in s_gunler:
        m = _kume(h["misafir"])
        farkli |= m
        hafta_gunu[t.weekday()] |= m
        izinli |= _kume(h["izinli"])
        for k, v in h["saat"].items():
            if isinstance(k, str) and k.isascii() and k.isdigit() and int(k) < 24:
                saat[int(k)] |= _kume(v)
        for k, v in h["alan"].items():
            if kok_alan(k) == k:
                alan.setdefault(k, set()).update(_kume(v))
        giris_sms += _int(h["giris"].get("sms"))
        giris_kod += _int(h["giris"].get("kod"))
        gonderilen += _int(h.get("gonderilen"))
        alinan += _int(h.get("alinan"))
        sms += _int(h.get("sms"))
        gunluk.append((t.isoformat(), len(m)))

    bas_s, bit_s = bas.isoformat(), bit.isoformat()
    onceki = set()
    for g, h in hamlar.items():
        if g < bas_s:
            onceki |= _kume(h["misafir"])
    yeni = len(farkli - onceki)

    surum, sonra = None, False
    for g in sorted(hamlar):
        v = hamlar[g].get("surum")
        if not (isinstance(v, str) and SURUM_RE.fullmatch(v)):
            continue
        if g <= bit_s:
            surum = v
        else:
            sonra = True

    en_yogun = None
    for g, n in gunluk:  # eşitlikte erken gün
        if en_yogun is None or n > en_yogun["farkli"]:
            en_yogun = {"gun": g, "farkli": n}
    alanlar = sorted(((k, len(v)) for k, v in alan.items() if len(v) >= ALAN_EN_AZ), key=lambda x: (-x[1], x[0]))
    kesin = bit < bugun and (any(g > bit_s for g in harita) or bugun >= bit + datetime.timedelta(days=KESIN_GUN))

    # imza: dönemin günleri + dönemden önceki günler ("yeni" ve sürüm onlara bağlı) + sonraki sürüm bilgisi + kesin
    def h_imza(g):
        return hamlar[g]["imza"] if g in hamlar else ""
    onceki_ozet = hashlib.sha256(json.dumps([(g, h_imza(g)) for g in sorted(hamlar) if g < bas_s]).encode()).hexdigest()
    imz = hashlib.sha256(json.dumps({
        "bicim": BICIM, "grup": grup_adlari, "onceki": onceki_ozet, "sonra_surum": sonra, "kesin": kesin,
        "gunler": [(x["gun"], (harita.get(x["gun"]) or ("",))[0], h_imza(x["gun"]), x["durum"], x["neden"])
                   for x in gunler]}, sort_keys=True).encode()).hexdigest()

    return {"bicim": BICIM, "tur": tur, "donem": anahtar, "baslangic": bas_s, "bitis": bit_s,
            "uretim": common.now_iso(simdi), "kesin": kesin, "imza": imz,
            "misafir": {"farkli": len(farkli), "yeni": yeni, "donen": len(farkli) - yeni,
                        "giris": giris_sms + giris_kod, "giris_sms": giris_sms, "giris_kod": giris_kod,
                        "izinli_cihaz": len(izinli),
                        "gunluk_ortalama": round(sum(n for _, n in gunluk) / len(gunluk), 1) if gunluk else 0.0,
                        "en_yogun_gun": en_yogun},
            "saat": [len(x) for x in saat], "hafta_gunu": [len(x) for x in hafta_gunu],
            "gunler": gunler, "kayit": {"gun": kayit_gun, "tam": kayit_tam},
            "kullanim": {"gonderilen": gonderilen, "alinan": alinan, "sms": sms,
                         "alanlar": [{"alan": k, "misafir": n} for k, n in alanlar[:ALAN_EN_COK]]},
            "surum": surum, "surum_sonra_degisti": sonra}


def _yaz_panel(yol, veri, grup):
    """root:wcpanel 640, atomik."""
    gecici = f"{yol}.{os.getpid()}.tmp"
    fd = os.open(gecici, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o640)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(veri, f, ensure_ascii=False, separators=(",", ":"))
        f.flush()
        os.fsync(f.fileno())
    os.chmod(gecici, 0o640)
    if grup:
        shutil.chown(gecici, group=grup)
    os.replace(gecici, yol)


def _dizin_panel(yol, grup):
    os.makedirs(yol, exist_ok=True)
    os.chmod(yol, 0o750)
    if grup:
        shutil.chown(yol, group=grup)


def grup_raporla(numara, grup, ham_dizini, cikti, panel_grubu, bugun, simdi):
    """Bir müşterinin dönem raporları + liste.json. İmzası aynı rapor yeniden yazılmaz; pencere dışı dosyalar silinir."""
    harita = gun_haritasi(grup)
    adlar = list(dict.fromkeys(ad for ad, _ in grup))
    hamlar = {}
    for g, girdi in harita.items():
        h = _ham_yukle(ham_dizini, girdi)
        if h is not None:
            hamlar[g] = h
    gecmis = [_tarih(g) for g in harita if _tarih(g) <= bugun]
    hedef = os.path.join(cikti, numara)
    _dizin_panel(hedef, panel_grubu)

    zincirler, durumlar, liste, isteniyor = {}, {}, [], set()
    for tur, anahtar, bas, bit in (donemler(min(gecmis), bugun) if gecmis else []):
        for t in _gunler(bas, bit):
            g = t.isoformat()
            if g in durumlar:
                continue
            girdi = harita.get(g)
            zincir = {}
            if girdi is not None:
                if girdi[1] not in zincirler:
                    zincirler[girdi[1]] = zincir_oku(girdi[1])  # her çalıştırmada (ucuz); dosya hash'i ham önbellekten
                zincir = zincirler[girdi[1]]
            durumlar[g] = gun_durumu(g, bugun, girdi, hamlar.get(g), zincir)
        r = donem_raporu(tur, anahtar, bas, bit, bugun, simdi, adlar, harita, hamlar, durumlar)
        ad = f"{tur}-{anahtar}.json"
        isteniyor.add(ad)
        yol = os.path.join(hedef, ad)
        mevcut = _json_oku(yol)
        if not mevcut or mevcut.get("imza") != r["imza"]:
            _yaz_panel(yol, r, panel_grubu)
        liste.append({"tur": tur, "donem": anahtar, "baslangic": r["baslangic"], "bitis": r["bitis"],
                      "kesin": r["kesin"], "farkli": r["misafir"]["farkli"], "kayit_gun": r["kayit"]["gun"],
                      "kayit_tam": r["kayit"]["tam"]})

    liste_yolu = os.path.join(hedef, "liste.json")
    try:
        with open(liste_yolu, encoding="utf-8") as f:
            eski_liste = json.load(f)
    except (OSError, ValueError):
        eski_liste = None
    if eski_liste != liste:
        _yaz_panel(liste_yolu, liste, panel_grubu)
    for a in os.listdir(hedef):
        if DONEM_DOSYA_RE.fullmatch(a) and a not in isteniyor:
            _sil(os.path.join(hedef, a))


def calistir(arsivler, grup_dosyasi=GRUPLAR, ham_dizini=HAM, cikti=CIKTI, panel_grubu=PANEL_GRUBU, simdi=None, sir=None):
    """wc-istatistik'ten: önce her arşivin ham özetleri (bütün kaynakları TEK çağrıda: ayrı çağrılar birbirinin günlerini
    silerdi), sonra her müşterinin raporları. Bir arşivin/grubun hatası diğerlerini durdurmaz.
    Dönüş: hata metinleri (arşiv/numara + gün + istisna türü; dosya adı yok)."""
    simdi = time.time() if simdi is None else simdi
    bugun = datetime.datetime.fromtimestamp(simdi, common.TZ).date()
    try:
        sir = sir if sir is not None else sir_al(os.path.join(ham_dizini, "sir"))
    except Exception as e:  # sır yoksa takma kimlik yok: hiçbir şey yazılmaz
        return [f"sir: {type(e).__name__}"]
    hatalar = []
    for ad, a in sorted(arsivler.items()):
        try:
            hatalar += [f"{ad}: {h}" for h in ham_guncelle(ad, a["kaynaklar"], ham_dizini, sir)]
        except Exception as e:
            hatalar.append(f"{ad}: {type(e).__name__}")
    _dizin_panel(cikti, panel_grubu)
    for numara, grup in sorted(gruplar(arsivler, grup_dosyasi).items()):
        try:
            grup_raporla(numara, grup, ham_dizini, cikti, panel_grubu, bugun, simdi)
        except Exception as e:
            hatalar.append(f"rapor {numara}: {type(e).__name__}")
    return hatalar
