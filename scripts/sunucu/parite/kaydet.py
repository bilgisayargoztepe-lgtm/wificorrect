"""Geçici (Task 16'da silinir): altın kayıt üreticisi. Python merkez testlerini tek tek, belirlenimli rastgelelikle çalıştırır;
her testin istek cevaplarını ve veritabanlarının son hâlini merkez/tests/altin/<dosya>/<test>.json'a yazar. Rust'taki aynı
test (`altin::Kayit::bas`) aynı kaydı üretmelidir. Kullanım:
    py -3 -I scripts/sunucu/parite/kaydet.py [test_dosyasi ...] [--is N] [--cikti DIZIN] [--tohum N]
test_dosyasi: `test_yonetim` ya da yolu; verilmezse hepsi. Her test ayrı süreçte çalışır (testler arası modül durumu
taşınmaz); süreç PYTHONHASHSEED=0 (--tohum), UTF-8 kipi ve Linux gibi "\\n" satır sonuyla başlar.

Kayıt biçimi (Rust altin.rs aynısını üretir):
- cevaplar: testin her üst düzey `istek` çağrısı (iç içe çağrıda yalnız en dıştaki; yalnız ana iş parçacığı), sırayla:
  istek ("YÖNTEM /yol[?sorgu]", belirteçler <T?>), durum, basliklar (SECILI_BASLIKLAR; [ad, değer], sırayla), govde:
  {"metin": ...} | {"json": ...} | {"ikili": {"sha256", "uzunluk"}}. İstisna 500 "Bir sorun oluştu" olarak yazılır (HTTP
  işleyicisi gibi), "istisna" alanında türüyle.
- veritabanlari: testte açılan her veri.Veri dosyası (ilk açılış sırasıyla, yol başına bir kez): tablo adına göre sıralı
  tablolar, satırlar rowid sırasıyla, satır = {sütun: değer}.
- Metin gövdede ortak CSS, logolar ve sekme simgesi {CSS}, {LOGO_ACIK}, {LOGO_KOYU}, {FAVICON} olur.
- Belirteç: testte secrets.token_urlsafe / token_hex'in döndürdüğü, en az 8 karakterlik her değer. Her metinde (başlık
  değeri, metin gövde, JSON metinleri ve anahtarları, tablo metinleri) en soldaki-en uzun eşleşmeyle <Tn> olur; n ilk
  görünüş sırası: cevaplar sırayla (başlıklar, gövde; JSON nesnelerinde anahtar sırasıyla), sonra veritabanları (tablo adı,
  rowid, sütun adı sırasıyla).
- yollar: kapsam için (yollar.py) her `istek` çağrısı, iç içe olanlar da: [uygulama, yöntem, yol, host, durum, yer].
"""
import ast
import base64
import concurrent.futures
import gzip
import hashlib
import importlib.util
import io
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import weakref

AYR = os.path.dirname(os.path.abspath(__file__))
KOK = os.path.normpath(os.path.join(AYR, "..", "..", ".."))
MERKEZ = os.path.join(KOK, "scripts", "sunucu", "merkez")
TESTLER = os.path.join(MERKEZ, "tests")
CIKTI = os.path.join(KOK, "merkez", "tests", "altin")
GZ_SINIRI = 15 * 1024 * 1024  # kayıtlar (açık hâliyle) bundan büyükse her dosya .json.gz
BELIRTEC_EN_AZ = 8
SECILI_BASLIKLAR = ("location", "set-cookie", "content-type", "cache-control", "clear-site-data", "content-disposition",
                    "x-content-type-options", "referrer-policy", "x-frame-options", "content-security-policy",
                    "strict-transport-security")
UYGULAMALAR = ("Merkez", "Yonetim", "Musteri", "Api", "Cihaz", "Ap")
HATA_GOVDE = "Bir sorun oluştu"


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
    return {"ikili": {"sha256": hashlib.sha256(govde).hexdigest(), "uzunluk": len(govde)}}


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
            return f"<T{self.no[t]}>"
        return self._degistir(s, yer)

    def numarasiz(self, s):
        return self._degistir(s, lambda t: "<T?>")

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


def normallestir(cevaplar, veritabanlari, uretilen):
    b = Belirtecler(uretilen)
    for c in cevaplar:
        c["istek"] = b.numarasiz(c["istek"])
    for c in cevaplar:
        c["basliklar"] = [[k, b.numarali(v)] for k, v in c["basliklar"]]
        g = c["govde"]
        if "metin" in g:
            g["metin"] = b.numarali(g["metin"])
        elif "json" in g:
            g["json"] = b.json(g["json"])
    for vt in veritabanlari:
        for t in sorted(vt["tablolar"]):
            for satir in vt["tablolar"][t]:
                for s in sorted(satir):
                    if isinstance(satir[s], str):
                        satir[s] = b.numarali(satir[s])


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
        self.sira = []  # ilk açılış sırası
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
    import builtins
    builtins.open = _open
    asil_gzip = gzip.open

    def _gzip_open(filename, mode="rb", compresslevel=9, encoding=None, errors=None, newline=None):
        if newline is None and "t" in mode and any(c in mode for c in "wax"):
            newline = "\n"
        return asil_gzip(filename, mode, compresslevel, encoding, errors, newline)
    gzip.open = _gzip_open

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

    asil_temizle = tempfile.TemporaryDirectory.cleanup

    def temizle(self):
        kok = os.path.realpath(self.name)
        for g in list(D.veriler):
            try:
                altinda = not g.startswith(":memory:") and os.path.commonpath([kok, g]) == kok
            except ValueError:  # başka sürücü
                altinda = False
            if altinda:
                D.dokum[g] = _yol_dok(g)
        return asil_temizle(self)
    tempfile.TemporaryDirectory.cleanup = temizle

    import ap
    import api
    import cihaz
    import merkez
    import web
    for sinif in (web.Taban, api.Api, cihaz.Cihaz, ap.Ap, merkez.Merkez):
        sinif.istek = _sar(sinif.istek)


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


def _cevap_ekle(yontem, yol, sorgu, sonuc, istisna):
    durum, basliklar, govde = sonuc
    satirlar = []
    for k, v in basliklar.items():
        for d in (v if isinstance(v, (list, tuple)) else (v,)):
            satirlar.append((str(k), str(d)))
    if isinstance(govde, str):
        govde = govde.encode("utf-8")
    for k, _ in satirlar:
        if not secili(k):
            print(f"UYARI: seçili olmayan başlık kaydedilmedi: {k}", file=sys.stderr)
    c = {"istek": f"{yontem} {yol}" + (f"?{sorgu}" if sorgu else ""), "durum": int(durum),
         "basliklar": [(k, v) for k, v in satirlar if secili(k)], "govde": govde_bicimi(satirlar, bytes(govde), _krom)}
    if istisna:
        c["istisna"] = istisna
    D.cevaplar.append(c)


def _krom(s):
    import web
    for ad, deger in (("CSS", web.CSS), ("LOGO_ACIK", web.LOGO_ACIK), ("LOGO_KOYU", web.LOGO_KOYU), ("FAVICON", web.FAVICON)):
        if deger:
            s = s.replace(deger, "{" + ad + "}")
    return s


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
    getattr(mod, test)()
    vts = []
    for g, temel in D.sira:
        t = D.dokum[g] if g in D.dokum else _yol_dok(g)
        vts.append({"ad": temel, "tablolar": t if t is not None else {}})
    normallestir(D.cevaplar, vts, D.uretilen)
    kayit = {"bicim": 1, "dosya": ad, "test": test, "cevaplar": D.cevaplar, "veritabanlari": vts, "yollar": D.yollar}
    os.makedirs(os.path.join(cikti, ad), exist_ok=True)
    with io.open(os.path.join(cikti, ad, test + ".json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(kayit, ensure_ascii=False, indent=1) + "\n")


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


def calistir(dosya, test, cikti, tohum):
    komut = [sys.executable, "-X", "utf8", "-s", "-P", "-B", os.path.abspath(__file__), "--tek", dosya, test, "--cikti", cikti]
    t0 = time.time()
    r = subprocess.run(komut, cwd=KOK, env=alt_ortam(tohum), capture_output=True, text=True, encoding="utf-8", errors="replace")
    return dosya, test, r.returncode, r.stdout + r.stderr, time.time() - t0


def boyut_duzenle(cikti):
    """Kayıtlar (açık hâliyle) GZ_SINIRI'ndan büyükse hepsi .json.gz, değilse hepsi .json."""
    dosyalar, toplam = [], 0
    for kok, _, adlar in os.walk(cikti):
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
    t0, hatalar, n = time.time(), [], 0
    with concurrent.futures.ThreadPoolExecutor(is_sayisi) as ex:
        for dosya, test, kod, cikis, sure in ex.map(lambda x: calistir(*x, cikti, tohum), isler):
            n += 1
            ad = os.path.splitext(os.path.basename(dosya))[0]
            if kod != 0:
                hatalar.append(f"{ad}.{test}")
                print(f"HATA {ad}.{test} ({sure:.1f} sn)\n{cikis}", file=sys.stderr)
            elif "UYARI" in cikis:
                print(f"{ad}.{test}: {cikis.strip()}", file=sys.stderr)
    toplam, gz, adet = boyut_duzenle(cikti)
    print(f"{n} test, {len(hatalar)} hata, {time.time() - t0:.0f} sn; kayıtlar {adet} dosya, {toplam / 1e6:.1f} MB"
          f"{' (gzip)' if gz else ''}")
    if hatalar:
        print("Başarısız: " + " ".join(hatalar), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--tek":
        try:
            tek(sys.argv[2], sys.argv[3], sys.argv[5] if len(sys.argv) > 5 else CIKTI)
        except SystemExit:
            raise
        except BaseException:
            traceback.print_exc()
            sys.exit(1)
    else:
        ana(sys.argv[1:])
