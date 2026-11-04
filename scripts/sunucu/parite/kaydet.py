"""Geçici (Task 16'da silinir): altın kayıt üreticisi. Python merkez testlerini tek tek, belirlenimli rastgelelikle çalıştırır;
her testin istek cevaplarını, veritabanlarının son hâlini ve geçici dizinlerindeki dosyaları
merkez/tests/altin/<dosya>/<test>.json'a yazar. Rust'taki aynı test (`altin::Kayit::bas`) aynı kaydı üretmelidir. Kullanım:
    py -3 -I scripts/sunucu/parite/kaydet.py [test_dosyasi ...] [--is N] [--cikti DIZIN] [--tohum N]
test_dosyasi: `test_yonetim` ya da yolu; verilmezse hepsi. Her test ayrı süreçte çalışır (testler arası modül durumu
taşınmaz); süreç PYTHONHASHSEED=0 (--tohum), UTF-8 kipi ve Linux gibi "\\n" satır sonuyla başlar. Ayrıca
merkez/tests/altin/_krom.json: Python'un ortak CSS, logo ve sekme simgesinin sha256'sı (Rust'takilerle aynı olmalı).

Kayıt biçimi 2 (Rust altin.rs aynısını üretir):
- cevaplar: testin her üst düzey `istek` çağrısı (iç içe çağrıda yalnız en dıştaki; yalnız ana iş parçacığı), sırayla:
  istek ("YÖNTEM /yol[?sorgu]", belirteçler <T?:UZUNLUK>), durum, basliklar (SECILI_BASLIKLAR; [ad, değer], sırayla; seçili
  olmayan başlık hatadır), govde: {"metin": ...} | {"json": ...} | {"ikili": {"sha256", "uzunluk"}}
  (metin ya da JSON olmayan gzip gövde açılır: "gz": true, metin ya da ikili). İstisna 500
  "Bir sorun oluştu" olarak yazılır (HTTP işleyicisi gibi), "istisna" alanında türüyle.
- veritabanlari: testte açılan her veri.Veri dosyası (ilk açılış sırasıyla, yol başına bir kez): tablo adına göre sıralı
  tablolar, satırlar rowid sırasıyla, satır = {sütun: değer}.
- dosyalar: testin açtığı her geçici dizin (tempfile.mkdtemp / TemporaryDirectory, açılış sırasıyla), silinmeden hemen önce
  ya da test sonunda: {göreli yol (/ ile): içerik}; dizinler "yol/": {"dizin": true}; dosya: .gz açılmış hâliyle ("gz": true),
  .json(.gz) JSON değeri, UTF-8 metin, değilse sha256 + uzunluk. SQLite veritabanları ve günlükleri, sembolik bağlar ve
  özel dosyalar (FIFO) alınmaz.
- platform_farki: testin Windows'ta atladığı Linux adımları (sembolik bağ kurulamadı, testin baktığı os.X yok). Yalnızca
  PLATFORM_TESTLERI'ndeki testlerde ve oradaki değerle olabilir (başka türlüsü kayıt hatası). Doluysa Rust `dosyalar`'da
  Python'un dizin ve dosyalarının kendi dökümünde aynı içerikle bulunmasını ister (Linux'ta fazlası olabilir).
- Kararsız değerler: her test iki kez koşar; ikincisinde gzip başlık zamanı ve sıkıştırma düzeyi değişir, dosya zamanları
  zaten farklıdır. Yanıtlar, veritabanları, dizinler ve dosya adları iki koşuda aynı olmalı; dosya içeriğinde de yalnızca
  `kararsiz_izinli` yapraklar farklı olabilir ve "<KARARSIZ>" olur (dosya JSON'unun üst düzey `imza`/`manifest_sha`'sı,
  MANIFEST.sha256 ve zincir.txt metni, açılamayan .gz'nin ikili özeti: hepsi gz baytına ya da dosya zamanına bağlı, Rust'ta
  da farklıdır). Başka her fark (tür, anahtar, uzunluk, yaprak) yoluyla birlikte kayıt hatasıdır. Rust "<KARARSIZ>"ı yalnızca
  bir yaprağa karşı kabul eder.
- Metin gövdede ortak CSS, logolar ve sekme simgesi {CSS}, {LOGO_ACIK}, {LOGO_KOYU}, {FAVICON} olur.
- Belirteç: testte secrets.token_urlsafe / token_hex'in döndürdüğü, en az 8 karakterlik her değer. Her metinde (başlık
  değeri, metin gövde, JSON metinleri ve anahtarları, tablo metinleri, dosya yolları ve içerikleri) en soldaki-en uzun
  eşleşmeyle <Tn:UZUNLUK> olur; n ilk görünüş sırası: cevaplar sırayla (başlıklar, gövde; JSON nesnelerinde anahtar
  sırasıyla), sonra veritabanları (tablo adı, rowid, sütun adı sırasıyla), sonra dosyalar (dizin sırası, yol sırası).
- yollar: kapsam için (yollar.py) her `istek` çağrısı, iç içe olanlar da: [uygulama, yöntem, yol, host, durum, yer].
"""
import ast
import base64
import builtins
import concurrent.futures
import gzip
import hashlib
import importlib.util
import io
import json
import os
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import weakref
import zlib

AYR = os.path.dirname(os.path.abspath(__file__))
KOK = os.path.normpath(os.path.join(AYR, "..", "..", ".."))
MERKEZ = os.path.join(KOK, "scripts", "sunucu", "merkez")
TESTLER = os.path.join(MERKEZ, "tests")
CIKTI = os.path.join(KOK, "merkez", "tests", "altin")
BICIM = 2
GZ_SINIRI = 15 * 1024 * 1024  # kayıtlar (açık hâliyle) bundan büyükse her dosya .json.gz
BELIRTEC_EN_AZ = 8
SECILI_BASLIKLAR = ("location", "set-cookie", "content-type", "cache-control", "clear-site-data", "content-disposition",
                    "x-content-type-options", "referrer-policy", "x-frame-options", "content-security-policy",
                    "strict-transport-security")
UYGULAMALAR = ("Merkez", "Yonetim", "Musteri", "Api", "Cihaz", "Ap")
HATA_GOVDE = "Bir sorun oluştu"
SQLITE_IMZA = b"SQLite format 3\x00"
GUNLUK_EKLERI = ("-journal", "-wal", "-shm")


def secili(ad):
    a = ad.lower()
    return a in SECILI_BASLIKLAR or a.startswith("x-wfc-")


# ======================================================================================================================
# Normalleştirme (Rust altin.rs ile birebir)
# ======================================================================================================================
def govde_bicimi(basliklar, govde, krom):
    """basliklar: [(ad, değer)] (hepsi). Gövde: json / metin / ikili."""
    ct = next((v for k, v in basliklar if k.lower() == "content-type"), "")
    tur = ct.split(";")[0].strip().lower()
    if tur == "application/json":
        try:
            return {"json": json.loads(govde.decode("utf-8"))}
        except ValueError:
            pass
    if not govde:
        return {"metin": ""}
    if tur.startswith("text/") or tur in ("", "application/json"):
        try:
            return {"metin": krom(govde.decode("utf-8"))}
        except UnicodeDecodeError:
            pass
    if govde[:2] == b"\x1f\x8b":  # gzip (ör. /api/liste): sıkıştırılmış baytlar uygulamaya göre değişir, içerik değişmez
        try:
            acik = gzip.decompress(govde)
        except (OSError, EOFError, zlib.error):
            return ikili(govde)
        try:
            return {"metin": acik.decode("utf-8"), "gz": True}
        except UnicodeDecodeError:
            return dict(ikili(acik), gz=True)
    return ikili(govde)


def ikili(b):
    return {"ikili": {"sha256": hashlib.sha256(b).hexdigest(), "uzunluk": len(b)}}


def dosya_bicimi(ad, bayt):
    """Dosya içeriği: .gz açılır ("gz": true); .json → JSON değeri; UTF-8 → metin; değilse sha256 + uzunluk."""
    gz = False
    if ad.endswith(".gz") and bayt:
        try:
            bayt, gz, ad = gzip.decompress(bayt), True, ad[:-3]
        except (OSError, EOFError, zlib.error):
            return ikili(bayt)
    out = None
    if ad.endswith(".json"):
        try:
            out = {"json": json.loads(bayt.decode("utf-8"))}
        except ValueError:
            pass
    if out is None:
        try:
            out = {"metin": bayt.decode("utf-8")}
        except UnicodeDecodeError:
            out = ikili(bayt)
    if gz:
        out["gz"] = True
    return out


def dizin_dok(kok):
    """{göreli yol: içerik}, yola göre sıralı. Dizinler "yol/". Veritabanları, günlükleri, bağlar ve özel dosyalar alınmaz."""
    out = {}
    for ust, dizinler, dosyalar in os.walk(kok):
        dizinler.sort()
        for ad in sorted(dizinler + dosyalar):
            yol = os.path.join(ust, ad)
            goreli = os.path.relpath(yol, kok).replace(os.sep, "/")
            st = os.lstat(yol)
            if stat.S_ISLNK(st.st_mode):
                continue
            if stat.S_ISDIR(st.st_mode):
                out[goreli + "/"] = {"dizin": True}
                continue
            if not stat.S_ISREG(st.st_mode) or ad.endswith(GUNLUK_EKLERI):
                continue
            with io.open(yol, "rb") as f:
                bayt = f.read()
            if bayt.startswith(SQLITE_IMZA):
                continue
            out[goreli] = dosya_bicimi(ad, bayt)
    return dict(sorted(out.items()))


class Belirtecler:
    def __init__(self, uretilen):
        self.liste = sorted({t for t in uretilen if len(t) >= BELIRTEC_EN_AZ})
        self.no = {}

    def _degistir(self, s, yer):
        adaylar = []
        for t in self.liste:
            i = s.find(t)
            while i >= 0:
                adaylar.append((i, -len(t), t))
                i = s.find(t, i + 1)
        if not adaylar:
            return s
        adaylar.sort()
        out, pos = [], 0
        for i, _, t in adaylar:
            if i < pos:
                continue
            out.append(s[pos:i])
            out.append(yer(t))
            pos = i + len(t)
        out.append(s[pos:])
        return "".join(out)

    def numarali(self, s):
        def yer(t):
            if t not in self.no:
                self.no[t] = len(self.no) + 1
            return f"<T{self.no[t]}:{len(t)}>"
        return self._degistir(s, yer)

    def numarasiz(self, s):
        return self._degistir(s, lambda t: f"<T?:{len(t)}>")

    def json(self, v):
        if isinstance(v, str):
            return self.numarali(v)
        if isinstance(v, list):
            return [self.json(x) for x in v]
        if isinstance(v, dict):
            yeni = {}  # numaralama anahtar sırasıyla; nesnenin kendi sırası korunur
            for k in sorted(v):
                yeni[k] = (self.numarali(k), self.json(v[k]))
            return dict(yeni[k] for k in v)
        return v

    def icerik(self, d):
        """Dosya ya da gövde içeriği ({"metin"} / {"json"} / diğerleri olduğu gibi)."""
        if "metin" in d:
            d["metin"] = self.numarali(d["metin"])
        elif "json" in d:
            d["json"] = self.json(d["json"])


def normallestir(cevaplar, veritabanlari, dizinler, uretilen):
    b = Belirtecler(uretilen)
    for c in cevaplar:
        c["istek"] = b.numarasiz(c["istek"])
    for c in cevaplar:
        c["basliklar"] = [[k, b.numarali(v)] for k, v in c["basliklar"]]
        b.icerik(c["govde"])
    for vt in veritabanlari:
        for t in sorted(vt["tablolar"]):
            for satir in vt["tablolar"][t]:
                for s in sorted(satir):
                    if isinstance(satir[s], str):
                        satir[s] = b.numarali(satir[s])
    for i, dz in enumerate(dizinler):
        yeni = {}
        for yol in sorted(dz):
            icerik = dz[yol]
            yeni[b.numarali(yol)] = icerik
            b.icerik(icerik)
        dizinler[i] = yeni


KARARSIZ = "<KARARSIZ>"
# İki koşuda farklı çıkabilecek tek yerler (dosya içerikleri): gz baytının ya da dosya zamanının özeti
KARARSIZ_JSON_ALANLARI = ("imza", "manifest_sha")  # dosya JSON'unun üst düzeyinde
KARARSIZ_METINLER = ("MANIFEST.sha256", "zincir.txt")
# Windows'ta (sembolik bağ yetkisi, os.mkfifo, O_NOFOLLOW yok) Linux adımını atlayan testler ve beklenen platform_farki
PLATFORM_TESTLERI = {
    ("test_kuyruk", "test_sembolik_bag_ve_fifo_izlenmez"): ["os.O_NOFOLLOW yok"],
    ("test_rapor", "test_gruplar"): ["os.symlink kurulamadı"],
    ("test_rapor", "test_gun_dogrula_sembolik_bag"): ["os.symlink kurulamadı"],
    ("test_rapor", "test_tamam_mi_fifo_beklemez"): ["os.mkfifo yok"],
    ("test_rapor", "test_zincir_sembolik_bag_okunmaz"): ["os.symlink kurulamadı"],
    ("test_wc_istatistik", "test_gunluk_sembolik_bag_ise_arsiv_okunmaz"): ["os.symlink kurulamadı"],
    ("test_wc_istatistik", "test_sembolik_bag_gun_ve_dosya_atlanir"): ["os.symlink kurulamadı"],
}


class KararsizKayit(Exception):
    pass


def kararsiz_izinli(dosya, alan):
    """dosya: geçici dizindeki göreli yol; alan: içerikteki yol (ör. ("json", "imza")). Rust altin.rs ile aynı kural."""
    ad = dosya.rsplit("/", 1)[-1]
    return ((alan == ("metin",) and ad in KARARSIZ_METINLER)
            or (len(alan) == 2 and alan[0] == "json" and alan[1] in KARARSIZ_JSON_ALANLARI)
            or (dosya.endswith(".gz") and alan in (("ikili", "sha256"), ("ikili", "uzunluk"))))


def _kisa(v):
    m = json.dumps(v, ensure_ascii=False)
    return m if len(m) <= 120 else m[:120] + "…"


def _birlestir(a, b, yol, izin, alan=()):
    """İki koşunun aynı içeriği. Tür, anahtar ya da uzunluk farkı hatadır; farklı bir yaprak yalnızca izin(alan) ise
    KARARSIZ olur, değilse yoluyla hata."""
    if type(a) is not type(b):
        raise KararsizKayit(f"{yol}: iki koşuda tür farklı ({_kisa(a)} / {_kisa(b)})")
    if isinstance(a, dict):
        if a.keys() != b.keys():
            raise KararsizKayit(f"{yol}: iki koşuda anahtarlar farklı ({sorted(a.keys() ^ b.keys())})")
        return {k: _birlestir(a[k], b[k], f"{yol}/{k}", izin, alan + (k,)) for k in a}
    if isinstance(a, list):
        if len(a) != len(b):
            raise KararsizKayit(f"{yol}: iki koşuda uzunluk farklı ({len(a)} / {len(b)})")
        return [_birlestir(x, y, f"{yol}[{i}]", izin, alan + (i,)) for i, (x, y) in enumerate(zip(a, b))]
    if a == b:
        return a
    if izin(alan):
        return KARARSIZ
    raise KararsizKayit(f"{yol}: iki koşuda farklı ({_kisa(a)} / {_kisa(b)})")


def kayit_birlestir(a, b):
    """İki koşunun kaydı → tek kayıt. Dosya içerikleri dışında her şey aynı olmalı (farkta ilk farklı yol yazılır)."""
    for alan in ("bicim", "dosya", "test", "platform_farki", "yollar", "cevaplar", "veritabanlari"):
        _birlestir(a[alan], b[alan], alan, lambda _: False)
    if len(a["dosyalar"]) != len(b["dosyalar"]):
        raise KararsizKayit(f"dosyalar: iki koşuda geçici dizin sayısı farklı ({len(a['dosyalar'])} / {len(b['dosyalar'])})")
    out = dict(a)
    out["dosyalar"] = []
    for i, (x, y) in enumerate(zip(a["dosyalar"], b["dosyalar"])):
        if x.keys() != y.keys():
            raise KararsizKayit(f"dosyalar[{i}]: iki koşuda dosya adları farklı ({sorted(x.keys() ^ y.keys())})")
        out["dosyalar"].append({k: _birlestir(x[k], y[k], f"dosyalar[{i}]/{k}",
                                              lambda alan, k=k: kararsiz_izinli(k, alan)) for k in x})
    return out


def tablolar(db):
    """{tablo: [satır]} (tablo adı, rowid, sütun adı sırasıyla)."""
    sonuc = {}
    adlar = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").fetchall()]
    for ad in sorted(adlar):
        cur = db.execute(f'SELECT * FROM "{ad}" ORDER BY rowid')
        sutunlar = [d[0] for d in cur.description]
        satirlar = []
        for r in cur.fetchall():
            d = {}
            for s, x in sorted(zip(sutunlar, tuple(r))):
                d[s] = {"blob": x.hex()} if isinstance(x, (bytes, bytearray, memoryview)) else x
            satirlar.append(d)
        sonuc[ad] = satirlar
    return sonuc


# ======================================================================================================================
# Tek test (alt süreç)
# ======================================================================================================================
class Durum:
    def __init__(self):
        self.sayac = {}
        self.uretilen = []
        self.cevaplar = []
        self.yollar = []
        self.derinlik = 0
        self.veriler = {}  # gerçek yol → [Veri örneklerine zayıf başvuru] (çöpe gidişleri değişmesin)
        self.dokum = {}  # gerçek yol → tablolar (geçici dizin silinmeden önce alınan)
        self.sira = []  # veritabanlarının ilk açılış sırası
        self.dizinler = []  # [gerçek yol, döküm ya da None], açılış sırasıyla
        self.platform = []  # Windows'ta atlanan Linux adımları
        self.hatalar = []  # kaydı geçersiz kılan durumlar
        self.ana = threading.get_ident()


D = Durum()


def _baytlar(tur, n):
    n = 32 if n is None else n
    D.sayac[tur] = D.sayac.get(tur, 0) + 1
    c, out, blok = D.sayac[tur], b"", 0
    while len(out) < n:
        out += hashlib.sha256(f"{tur}:{c}:{blok}".encode()).digest()
        blok += 1
    return out[:n]


def token_bytes(nbytes=None):
    return _baytlar("bytes", nbytes)


def token_urlsafe(nbytes=None):
    t = base64.urlsafe_b64encode(_baytlar("urlsafe", nbytes)).rstrip(b"=").decode("ascii")
    D.uretilen.append(t)
    return t


def token_hex(nbytes=None):
    t = _baytlar("hex", nbytes).hex()
    D.uretilen.append(t)
    return t


def randbelow(n):
    if n <= 0:
        raise ValueError("Upper bound must be positive.")
    return int.from_bytes(_baytlar("below", 8), "big") % n


def choice(seq):
    if not len(seq):
        raise IndexError("Cannot choose from an empty sequence")
    return seq[int.from_bytes(_baytlar("choice", 8), "big") % len(seq)]


def _yol_dok(yol):
    """yol'daki veritabanının tabloları: açık bir Veri bağlantısıyla, yoksa salt okunur yeni bağlantıyla."""
    for z in D.veriler.get(yol, []):
        v = z()
        if v is None:
            continue
        try:
            return tablolar(v.db)
        except sqlite3.ProgrammingError:  # kapatılmış bağlantı
            continue
    if not os.path.exists(yol):
        return None
    db = sqlite3.connect(f"file:{yol}?mode=ro", uri=True)
    try:
        return tablolar(db)
    finally:
        db.close()


def _altinda(kok, yol):
    try:
        return not yol.startswith(":memory:") and os.path.commonpath([kok, yol]) == kok
    except ValueError:  # başka sürücü
        return False


def _dizin_kapat(ad):
    """Geçici dizin silinmeden hemen önce: dosyaları ve içindeki veritabanlarının tabloları."""
    kok = os.path.realpath(ad)
    for d in D.dizinler:
        if d[0] == kok and d[1] is None:
            for g in list(D.veriler):
                if _altinda(kok, g):
                    D.dokum[g] = _yol_dok(g)
            d[1] = dizin_dok(kok)


def kancalari_kur():
    import secrets
    secrets.token_bytes, secrets.token_urlsafe, secrets.token_hex = token_bytes, token_urlsafe, token_hex
    secrets.randbelow, secrets.choice = randbelow, choice

    # Windows'ta da Linux gibi: metin yazımında "\n" (çeviri yok)
    asil_open = io.open

    def _open(file, mode="r", buffering=-1, encoding=None, errors=None, newline=None, closefd=True, opener=None):
        if newline is None and "b" not in mode and any(c in mode for c in "wax+"):
            newline = "\n"
        return asil_open(file, mode, buffering, encoding, errors, newline, closefd, opener)
    io.open = _open
    builtins.open = _open
    asil_gzip = gzip.open

    def _gzip_open(filename, mode="rb", compresslevel=9, encoding=None, errors=None, newline=None):
        if newline is None and "t" in mode and any(c in mode for c in "wax"):
            newline = "\n"
        return asil_gzip(filename, mode, compresslevel, encoding, errors, newline)
    gzip.open = _gzip_open

    if os.environ.get("WC_KAYIT_KOSU") == "2":
        _gzip_oynat()

    # platform farkı: davranış aynı, yalnızca not düşülür
    asil_symlink = os.symlink

    def _symlink(*a, **k):
        try:
            return asil_symlink(*a, **k)
        except OSError:
            if "os.symlink kurulamadı" not in D.platform:
                D.platform.append("os.symlink kurulamadı")
            raise
    os.symlink = _symlink
    asil_hasattr = builtins.hasattr

    def _hasattr(nesne, ad):
        sonuc = asil_hasattr(nesne, ad)
        if not sonuc and nesne is os and sys._getframe(1).f_code.co_filename.startswith(TESTLER):
            if f"os.{ad} yok" not in D.platform:
                D.platform.append(f"os.{ad} yok")
        return sonuc
    builtins.hasattr = _hasattr

    # geçici dizinler: açılış sırasıyla; TemporaryDirectory silmeden hemen önce dökülür
    asil_mkdtemp = tempfile.mkdtemp

    def _mkdtemp(*a, **k):
        ad = asil_mkdtemp(*a, **k)
        if threading.get_ident() == D.ana:
            D.dizinler.append([os.path.realpath(ad), None])
        return ad
    tempfile.mkdtemp = _mkdtemp
    asil_rmtree = tempfile.TemporaryDirectory._rmtree.__func__

    def _rmtree(cls, name, ignore_errors=False, repeated=False):
        if not repeated:
            _dizin_kapat(name)
        return asil_rmtree(cls, name, ignore_errors=ignore_errors, repeated=repeated)
    tempfile.TemporaryDirectory._rmtree = classmethod(_rmtree)

    import veri
    asil_init = veri.Veri.__init__

    def init(self, yol, *a, **k):
        asil_init(self, yol, *a, **k)
        g = os.path.realpath(yol) if yol != ":memory:" else f":memory:{id(self)}"
        if g not in D.veriler:
            D.veriler[g] = []
            D.sira.append((g, os.path.basename(yol)))
        D.veriler[g].append(weakref.ref(self))
    veri.Veri.__init__ = init

    import ap
    import api
    import cihaz
    import merkez
    import web
    for sinif in (web.Taban, api.Api, cihaz.Cihaz, ap.Ap, merkez.Merkez):
        sinif.istek = _sar(sinif.istek)


def _gzip_oynat():
    """İkinci koşu: yazılan gzip'lerin başlık zamanı (verilmemişse) ve sıkıştırma düzeyi değişir; içerik aynı kalır.
    Böylece sıkıştırılmış bayta ya da boyuta bağlı değerler iki koşuda farklı çıkar ve kararsız işaretlenir."""
    asil_init = gzip.GzipFile.__init__

    def init(self, filename=None, mode=None, compresslevel=9, fileobj=None, mtime=None):
        yazma = any(c in (mode or getattr(fileobj, "mode", "") or "") for c in "wax")
        if yazma:
            compresslevel = 1 if compresslevel != 1 else 9
            mtime = 12345 if mtime is None else mtime
        asil_init(self, filename, mode, compresslevel, fileobj, mtime)
    gzip.GzipFile.__init__ = init
    asil_compress = gzip.compress

    def compress(data, compresslevel=9, *, mtime=None):
        return asil_compress(data, 1 if compresslevel != 1 else 9, mtime=12345 if mtime is None else mtime)
    gzip.compress = compress


def _uygulama(nesne):
    for s in type(nesne).__mro__:
        if s.__name__ in UYGULAMALAR:
            return s.__name__
    return type(nesne).__name__


def _sar(asil):
    def istek(self, yontem, yol, basliklar, govde, sorgu="", ip=""):
        ana = threading.get_ident() == D.ana
        if not ana:
            return asil(self, yontem, yol, basliklar, govde, sorgu, ip)
        uygulama = _uygulama(self)
        host = None
        if uygulama == "Merkez" and hasattr(basliklar, "get"):
            host = (basliklar.get("Host") or "").split(":")[0].strip().lower()
        D.derinlik += 1
        try:
            sonuc = asil(self, yontem, yol, basliklar, govde, sorgu, ip)
        except Exception as ex:
            D.derinlik -= 1
            D.yollar.append([uygulama, yontem, yol, host, 500, None])
            if D.derinlik == 0:
                _cevap_ekle(yontem, yol, sorgu, (500, {"Content-Type": "text/plain; charset=utf-8"},
                                                 HATA_GOVDE.encode("utf-8")), type(ex).__name__)
            raise
        D.derinlik -= 1
        yer = sonuc[1].get("Location") if isinstance(sonuc[1], dict) else None
        D.yollar.append([uygulama, yontem, yol, host, sonuc[0], yer])
        if D.derinlik == 0:
            _cevap_ekle(yontem, yol, sorgu, sonuc, None)
        return sonuc
    return istek


class SeciliOlmayanBaslik(Exception):
    pass


def _cevap_ekle(yontem, yol, sorgu, sonuc, istisna):
    durum, basliklar, govde = sonuc
    satirlar = []
    for k, v in basliklar.items():
        for d in (v if isinstance(v, (list, tuple)) else (v,)):
            satirlar.append((str(k), str(d)))
    if isinstance(govde, str):
        govde = govde.encode("utf-8")
    disarda = [k for k, _ in satirlar if not secili(k)]
    if disarda:  # Rust da reddeder: başlık SECILI_BASLIKLAR'a (iki tarafta) eklenmeli
        D.hatalar.append(f"{yontem} {yol}: seçili olmayan başlık {disarda}")  # test istisnayı yutsa da kayıt düşer
        raise SeciliOlmayanBaslik(D.hatalar[-1])
    c = {"istek": f"{yontem} {yol}" + (f"?{sorgu}" if sorgu else ""), "durum": int(durum),
         "basliklar": satirlar, "govde": govde_bicimi(satirlar, bytes(govde), _krom)}
    if istisna:
        c["istisna"] = istisna
    D.cevaplar.append(c)


def _kromlar():
    import web
    return (("CSS", web.CSS), ("LOGO_ACIK", web.LOGO_ACIK), ("LOGO_KOYU", web.LOGO_KOYU), ("FAVICON", web.FAVICON))


def _krom(s):
    for ad, deger in _kromlar():
        if deger:
            s = s.replace(deger, "{" + ad + "}")
    return s


def krom_yaz(cikti):
    """_krom.json: Python'un ortak parçalarının sha256'sı (Rust testi kendi web::CSS/LOGO_*/FAVICON'uyla karşılaştırır)."""
    sys.path[:0] = [MERKEZ]
    d = {ad: hashlib.sha256(deger.encode("utf-8")).hexdigest() for ad, deger in _kromlar()}
    with io.open(os.path.join(cikti, "_krom.json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(d, indent=1) + "\n")


def tek(dosya, test, cikti):
    if sys.flags.ignore_environment or not os.environ.get("PYTHONHASHSEED", "").isdigit():
        sys.exit("PYTHONHASHSEED verilmemiş (alt süreç -I/-E olmadan, ana süreçten başlatılmalı)")
    os.environ["WC_TEST"] = "1"
    sys.path[:0] = [TESTLER, MERKEZ]
    kancalari_kur()
    ad = os.path.splitext(os.path.basename(dosya))[0]
    spec = importlib.util.spec_from_file_location(ad, dosya)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[ad] = mod
    spec.loader.exec_module(mod)
    D.sayac.clear()
    D.uretilen.clear()
    D.cevaplar.clear()
    D.yollar.clear()
    D.platform.clear()
    D.dizinler.clear()
    getattr(mod, test)()
    if D.hatalar:
        raise RuntimeError("; ".join(D.hatalar))
    beklenen = PLATFORM_TESTLERI.get((ad, test), []) if os.name == "nt" else []
    if D.platform != beklenen:
        raise RuntimeError(f"platform_farki {D.platform}, beklenen {beklenen}: PLATFORM_TESTLERI'ni denetleyin")
    for d in D.dizinler:  # silinmemiş geçici dizinler (mkdtemp, kapanmamış TemporaryDirectory)
        if d[1] is None:
            if not os.path.isdir(d[0]):
                raise RuntimeError(f"geçici dizin dökülmeden silindi: {d[0]}")
            _dizin_kapat(d[0])
    vts = []
    for g, temel in D.sira:
        t = D.dokum[g] if g in D.dokum else _yol_dok(g)
        vts.append({"ad": temel, "tablolar": t if t is not None else {}})
    dizinler = [d[1] for d in D.dizinler]
    normallestir(D.cevaplar, vts, dizinler, D.uretilen)
    kayit = {"bicim": BICIM, "dosya": ad, "test": test, "platform_farki": D.platform, "cevaplar": D.cevaplar,
             "veritabanlari": vts, "dosyalar": dizinler, "yollar": D.yollar}
    metin = json.dumps(kayit, ensure_ascii=False, indent=1) + "\n"
    for d in D.dizinler:  # kayda geçici dizin yolu girmemeli (Windows ve Linux'ta farklıdır)
        if os.path.basename(d[0]) in metin:
            raise RuntimeError(f"kayıtta geçici dizin adı var: {os.path.basename(d[0])}")
    os.makedirs(os.path.join(cikti, ad), exist_ok=True)
    with io.open(os.path.join(cikti, ad, test + ".json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(metin)


# ======================================================================================================================
# Bütün testler (ana süreç)
# ======================================================================================================================
def test_adlari(yol):
    with io.open(yol, encoding="utf-8") as f:
        agac = ast.parse(f.read(), yol)
    return sorted(n.name for n in agac.body if isinstance(n, ast.FunctionDef) and n.name.startswith("test_"))


def dosyalari_bul(adlar):
    if not adlar:
        return sorted(os.path.join(TESTLER, a) for a in os.listdir(TESTLER) if a.startswith("test_") and a.endswith(".py"))
    out = []
    for a in adlar:
        yol = a if a.endswith(".py") else os.path.join(TESTLER, a + ".py")
        if not os.path.isfile(yol):
            sys.exit(f"test dosyası yok: {a}")
        out.append(os.path.abspath(yol))
    return out


def alt_ortam(tohum):
    o = {k: v for k, v in os.environ.items() if not k.upper().startswith("PYTHON")}
    o.update(PYTHONHASHSEED=str(tohum), WC_TEST="1", PYTHONIOENCODING="utf-8")
    return o


def _alt_komut(*arg):
    return [sys.executable, "-X", "utf8", "-s", "-P", "-B", os.path.abspath(__file__), *arg]


def calistir(dosya, test, cikti, tohum):
    """Test iki kez (ikincisi gzip oynatılarak) koşar; kayıtlar birleştirilip cikti'ye yazılır."""
    t0 = time.time()
    ad = os.path.splitext(os.path.basename(dosya))[0]
    with tempfile.TemporaryDirectory() as gecici:
        kayitlar, cikis = [], ""
        for kosu in ("1", "2"):
            hedef = os.path.join(gecici, kosu)
            r = subprocess.run(_alt_komut("--tek", dosya, test, "--cikti", hedef), cwd=KOK,
                               env=dict(alt_ortam(tohum), WC_KAYIT_KOSU=kosu), capture_output=True, text=True,
                               encoding="utf-8", errors="replace")
            cikis += r.stdout + r.stderr
            if r.returncode:
                return dosya, test, r.returncode, cikis, time.time() - t0
            with io.open(os.path.join(hedef, ad, test + ".json"), encoding="utf-8") as f:
                kayitlar.append(json.load(f))
        try:
            kayit = kayit_birlestir(*kayitlar)
        except KararsizKayit as h:
            return dosya, test, 1, f"{cikis}kararsız kayıt: {h}", time.time() - t0
    os.makedirs(os.path.join(cikti, ad), exist_ok=True)
    with io.open(os.path.join(cikti, ad, test + ".json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(kayit, ensure_ascii=False, indent=1) + "\n")
    return dosya, test, 0, cikis, time.time() - t0


def boyut_duzenle(cikti):
    """Kayıtlar (alt dizinlerdeki; açık hâliyle) GZ_SINIRI'ndan büyükse hepsi .json.gz, değilse hepsi .json."""
    dosyalar, toplam = [], 0
    for kok, _, adlar in os.walk(cikti):
        if os.path.samefile(kok, cikti):
            continue  # _krom.json
        for a in adlar:
            yol = os.path.join(kok, a)
            if a.endswith(".json"):
                with io.open(yol, "rb") as f:
                    icerik = f.read()
            elif a.endswith(".json.gz"):
                with io.open(yol, "rb") as f:
                    icerik = gzip.decompress(f.read())
            else:
                continue
            dosyalar.append((yol, icerik))
            toplam += len(icerik)
    gz = toplam > GZ_SINIRI
    for yol, icerik in dosyalar:
        temel = yol[:-3] if yol.endswith(".gz") else yol
        hedef = temel + ".gz" if gz else temel
        if hedef != yol:
            with io.open(hedef, "wb") as f:
                f.write(gzip.compress(icerik, 9, mtime=0) if gz else icerik)
            os.remove(yol)
    return toplam, gz, len(dosyalar)


def ana(arg):
    for akis in (sys.stdout, sys.stderr):
        akis.reconfigure(encoding="utf-8", errors="replace")
    is_sayisi, cikti, tohum, adlar = os.cpu_count() or 2, CIKTI, 0, []
    i = 0
    while i < len(arg):
        if arg[i] == "--is":
            is_sayisi, i = int(arg[i + 1]), i + 2
        elif arg[i] == "--cikti":
            cikti, i = os.path.abspath(arg[i + 1]), i + 2
        elif arg[i] == "--tohum":
            tohum, i = int(arg[i + 1]), i + 2
        else:
            adlar.append(arg[i])
            i += 1
    dosyalar = dosyalari_bul(adlar)
    isler = [(d, t) for d in dosyalar for t in test_adlari(d)]
    for d in dosyalar:
        shutil.rmtree(os.path.join(cikti, os.path.splitext(os.path.basename(d))[0]), ignore_errors=True)
    os.makedirs(cikti, exist_ok=True)
    r = subprocess.run(_alt_komut("--krom", cikti), cwd=KOK, env=alt_ortam(tohum), capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    if r.returncode:
        sys.exit(f"_krom.json yazılamadı:\n{r.stdout}{r.stderr}")
    t0, hatalar, n = time.time(), [], 0
    with concurrent.futures.ThreadPoolExecutor(is_sayisi) as ex:
        for dosya, test, kod, cikis, sure in ex.map(lambda x: calistir(*x, cikti, tohum), isler):
            n += 1
            ad = os.path.splitext(os.path.basename(dosya))[0]
            if kod != 0:
                hatalar.append(f"{ad}.{test}")
                print(f"HATA {ad}.{test} ({sure:.1f} sn)\n{cikis}", file=sys.stderr)
    toplam, gz, adet = boyut_duzenle(cikti)
    print(f"{n} test, {len(hatalar)} hata, {time.time() - t0:.0f} sn; kayıtlar {adet} dosya, {toplam / 1e6:.1f} MB"
          f"{' (gzip)' if gz else ''}")
    if hatalar:
        print("Başarısız: " + " ".join(hatalar), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] in ("--tek", "--krom"):
        try:
            if sys.argv[1] == "--krom":
                krom_yaz(sys.argv[2])
            else:
                tek(sys.argv[2], sys.argv[3], sys.argv[5] if len(sys.argv) > 5 else CIKTI)
        except SystemExit:
            raise
        except BaseException:
            traceback.print_exc()
            sys.exit(1)
    else:
        ana(sys.argv[1:])
