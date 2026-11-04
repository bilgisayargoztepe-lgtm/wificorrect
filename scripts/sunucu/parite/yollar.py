"""Geçici (Task 16'da silinir): yol kapsamı. Python merkezinin cevapladığı bütün yolları koddan (ast) çıkarır ve altın
kayıtlarda (kaydet.py'nin "yollar" listesi: iç içe olanlar dahil her `istek` çağrısı) hiç görülmeyenleri listeler.
Kullanım: py -3 -I scripts/sunucu/parite/yollar.py [--hepsi] [--kayitlar DIZIN]
--hepsi: çıkarılan bütün yolları kayıttaki görülme sayısıyla yazar. Çıkış kodu: kapsanmayan yol varsa 1.

Çıkarma kuralları (merkez kodunun deyimleri):
- Uygulama: `istek` yöntemi olan sınıf; alt sınıfı varsa (web.Taban) yolları alt sınıflara (Musteri, Yonetim) geçer.
  Taranan yöntemler: sınıfın ve atalarının `istek` ve `sayfalar`ı.
- Dağıtıcı (Merkez): `ALANLAR.get(...)` → modüldeki ALANLAR sözlüğünün her alan adı bir yol (`host=<ad>`).
- `if` koşulundaki `yol == "/x"`, `yol in ("/x", ...)`, `yol.startswith("/x/")`, `(yontem, yol) == ("POST", "/x")`;
  `yontem == / in` ile birleşince yöntem de yolun parçası. `m = re.fullmatch(r"/m/(...)(/.*)?", yol)` sonrası `alt` üzerindeki
  koşullar o önekin altındadır. `yontem != "POST" or yol not in uclar` → `uclar` sözlüğünün anahtarları yalnızca POST.
- Yöntemsiz koşulda dalın içi `yontem`e bakıyorsa (iç `if`, ya da `yontem` verilen ve ona bakan işleyici) GET ve POST ayrı
  yollardır; bakmıyorsa her yöntem aynı yoldur (*).
- Her uygulamanın bir de bilinmeyen yolu (404) vardır.
Kapsam: kayıttaki [uygulama, yöntem, yol, host, durum, yer] bir yolla eşleşmeli. Musteri/Yonetim'de oturumsuz isteğin
/giris'e yönlenmesi (303 → /giris) /giris ve /cikis dışındaki yolları kapsamaz; bilinmeyen yol 404 ile kapsanır."""
import ast
import gzip
import json
import os
import re
import sys

AYR = os.path.dirname(os.path.abspath(__file__))
KOK = os.path.normpath(os.path.join(AYR, "..", "..", ".."))
MERKEZ = os.path.join(KOK, "scripts", "sunucu", "merkez")
KAYITLAR = os.path.join(KOK, "merkez", "tests", "altin")


def _ad(d):
    return d.id if isinstance(d, ast.Name) else None


def _metin(d):
    return d.value if isinstance(d, ast.Constant) and isinstance(d.value, str) else None


class Yol:
    def __init__(self, uygulama, yontemler, tur, deger, onek=None, kaynak=""):
        self.uygulama, self.yontemler, self.tur, self.deger, self.onek, self.kaynak = uygulama, yontemler, tur, deger, onek, kaynak
        self.gorulme = 0

    def adi(self):
        if self.tur == "host":
            yol = f"host={self.deger}"
        elif self.tur == "bilinmeyen":
            yol = "<bilinmeyen yol: 404>"
        else:
            on = re.sub(r"\(\[0-9\]\{7\}\)", "<n>", self.onek[0]) if self.onek else ""
            yol = on + (self.deger if self.deger != "/" or not on else "") + ("*" if self.tur == "on" else "")
        return f"{self.uygulama} {'|'.join(sorted(self.yontemler))} {yol}"

    def eslesir(self, yontem, yol, host):
        if "*" not in self.yontemler and yontem not in self.yontemler:
            return False
        if self.tur == "host":
            return host == self.deger
        if self.onek:
            m = re.fullmatch(self.onek[1], yol)
            if not m:
                return False
            yol = m.group(2) or "/"
        return yol == self.deger if self.tur == "tam" else yol.startswith(self.deger)


class Cikarici:
    def __init__(self):
        self.moduller = {}  # ad → ast
        self.siniflar = {}  # sınıf adı → (modül, ast.ClassDef)
        for a in sorted(os.listdir(MERKEZ)):
            if a.endswith(".py"):
                with open(os.path.join(MERKEZ, a), encoding="utf-8") as f:
                    agac = ast.parse(f.read(), a)
                self.moduller[a[:-3]] = agac
                for d in agac.body:
                    if isinstance(d, ast.ClassDef):
                        self.siniflar[d.name] = (a[:-3], d)

    def _taban_adlari(self, sinif):
        out = []
        for b in sinif.bases:
            if isinstance(b, ast.Attribute):
                out.append(b.attr)
            elif isinstance(b, ast.Name):
                out.append(b.id)
        return [b for b in out if b in self.siniflar]

    def _yontem(self, sinif_adi, ad):
        """Sınıfta ya da atalarında `ad` yöntemi (modül, FunctionDef)."""
        modul, sinif = self.siniflar[sinif_adi]
        for d in sinif.body:
            if isinstance(d, ast.FunctionDef) and d.name == ad:
                return modul, d
        for t in self._taban_adlari(sinif):
            r = self._yontem(t, ad)
            if r:
                return r
        return None

    def uygulamalar(self):
        istekli = [s for s in self.siniflar if self._yontem(s, "istek")]
        alt = {s: [a for a in istekli if s in self._taban_adlari(self.siniflar[a][1])] for s in istekli}
        return sorted(s for s in istekli if not alt[s])

    # --- koşul çözümleme ---
    def _kosul(self, d, onek, sozlukler):
        """[(yöntemler | None, tür, değer, önek)] — koşulu sağlayan yollar (ayrık birleşim)."""
        if isinstance(d, ast.BoolOp) and isinstance(d.op, ast.Or):
            # yontem != "POST" or yol not in uclar → uclar'ın anahtarları, yalnızca POST
            yasak = [x for x in d.values if isinstance(x, ast.Compare) and _ad(x.left) == "yontem"
                     and isinstance(x.ops[0], ast.NotEq) and _metin(x.comparators[0])]
            sozluk = [x for x in d.values if isinstance(x, ast.Compare) and _ad(x.left) == "yol"
                      and isinstance(x.ops[0], ast.NotIn) and _ad(x.comparators[0]) in sozlukler]
            if yasak and sozluk:
                y = {_metin(yasak[0].comparators[0])}
                return [(y, "tam", k, None) for k in sozlukler[_ad(sozluk[0].comparators[0])]]
            return [r for x in d.values for r in self._kosul(x, onek, sozlukler)]
        if isinstance(d, ast.BoolOp) and isinstance(d.op, ast.And):
            yontem, yollar = None, []
            for x in d.values:
                y = self._yontem_kosulu(x)
                if y is not None:
                    yontem = y if yontem is None else yontem & y
                else:
                    yollar += self._kosul(x, onek, sozlukler)
            return [(yontem if r[0] is None else (r[0] if yontem is None else r[0] & yontem), r[1], r[2], r[3]) for r in yollar]
        if isinstance(d, ast.Compare) and len(d.ops) == 1:
            sol, op, sag = d.left, d.ops[0], d.comparators[0]
            # (yontem, yol) == ("POST", "/x")
            if isinstance(sol, ast.Tuple) and isinstance(sag, ast.Tuple) and isinstance(op, ast.Eq) and len(sol.elts) == 2:
                if _ad(sol.elts[0]) == "yontem" and _ad(sol.elts[1]) in ("yol", "alt"):
                    o = onek if _ad(sol.elts[1]) == "alt" else None
                    return [({_metin(sag.elts[0])}, "tam", _metin(sag.elts[1]), o)]
            if _ad(sol) in ("yol", "alt"):
                o = onek if _ad(sol) == "alt" else None
                if isinstance(op, ast.Eq) and _metin(sag) is not None:
                    return [(None, "tam", _metin(sag), o)]
                if isinstance(op, ast.In) and isinstance(sag, (ast.Tuple, ast.List, ast.Set)):
                    return [(None, "tam", _metin(x), o) for x in sag.elts if _metin(x) is not None]
        if isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute) and d.func.attr == "startswith":
            if _ad(d.func.value) in ("yol", "alt") and d.args and _metin(d.args[0]) is not None:
                return [(None, "on", _metin(d.args[0]), onek if _ad(d.func.value) == "alt" else None)]
        return []

    @staticmethod
    def _yontem_kosulu(d):
        if isinstance(d, ast.Compare) and len(d.ops) == 1 and _ad(d.left) == "yontem":
            if isinstance(d.ops[0], ast.Eq) and _metin(d.comparators[0]):
                return {_metin(d.comparators[0])}
            if isinstance(d.ops[0], ast.In) and isinstance(d.comparators[0], (ast.Tuple, ast.List, ast.Set)):
                return {_metin(x) for x in d.comparators[0].elts}
        return None

    @staticmethod
    def _yontem_bakar(dugumler):
        for d in dugumler:
            for x in ast.walk(d):
                if isinstance(x, ast.Compare) and (_ad(x.left) == "yontem" or any(
                        isinstance(e, ast.Tuple) and e.elts and _ad(e.elts[0]) == "yontem" for e in [x.left])):
                    return True
        return False

    def _dal_yontemleri(self, sinif_adi, govde):
        """Yöntemsiz koşulun dalı GET/POST ayırıyor mu: iç if ya da `yontem` verilen ve ona bakan işleyici."""
        if self._yontem_bakar(govde):
            return {"GET", "POST"}
        for d in govde:
            for x in ast.walk(d):
                if isinstance(x, ast.Call) and any(_ad(a) == "yontem" for a in x.args):
                    f = x.func
                    hedef = None
                    if isinstance(f, ast.Attribute) and _ad(f.value) == "self":
                        hedef = self._yontem(sinif_adi, f.attr)
                    elif isinstance(f, ast.Attribute) and isinstance(f.value, ast.Name):
                        hedef = self._modul_islevi(f.value.id, f.attr)
                    if hedef and self._yontem_bakar(hedef[1].body):
                        return {"GET", "POST"}
        return {"*"}

    def _modul_islevi(self, takma, ad):
        for m, agac in self.moduller.items():
            if m == takma or m + "_modulu" == takma:
                for d in agac.body:
                    if isinstance(d, ast.FunctionDef) and d.name == ad:
                        return m, d
        return None

    def islev_yollari(self, uygulama, sinif_adi, modul, fn):
        out = []
        onek = None
        sozlukler = {}
        for d in ast.walk(fn):
            if isinstance(d, ast.Assign) and len(d.targets) == 1 and isinstance(d.value, ast.Call):
                c = d.value
                if (isinstance(c.func, ast.Attribute) and c.func.attr == "fullmatch" and _ad(c.func.value) == "re"
                        and len(c.args) == 2 and _metin(c.args[0]) and _ad(c.args[1]) == "yol"):
                    desen = _metin(c.args[0])
                    taban = desen[:desen.rindex("(")] if desen.endswith(")?") else desen
                    onek = (taban, desen)
            if isinstance(d, ast.Assign) and len(d.targets) == 1 and isinstance(d.value, ast.Dict) and _ad(d.targets[0]):
                anahtarlar = [_metin(k) for k in d.value.keys]
                if anahtarlar and all(k and k.startswith("/") for k in anahtarlar):
                    sozlukler[_ad(d.targets[0])] = anahtarlar
            # dağıtıcı: ALANLAR.get(host)
            if (isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute) and d.func.attr == "get"
                    and isinstance(d.func.value, ast.Name)):
                for x in self.moduller[modul].body:
                    if (isinstance(x, ast.Assign) and _ad(x.targets[0]) == d.func.value.id and isinstance(x.value, ast.Dict)
                            and all(_metin(k) and "." in _metin(k) for k in x.value.keys)):
                        out += [Yol(uygulama, {"*"}, "host", _metin(k), None, f"{modul}.py:{x.lineno}") for k in x.value.keys]
        for d in ast.walk(fn):
            if isinstance(d, ast.If):
                for yontemler, tur, deger, o in self._kosul(d.test, onek, sozlukler):
                    if deger is None:
                        continue
                    y = yontemler if yontemler is not None else self._dal_yontemleri(sinif_adi, d.body)
                    out += [Yol(uygulama, {m}, tur, deger, o, f"{modul}.py:{d.lineno}") for m in sorted(y)]  # her yöntem ayrı yol
        return out

    def yollar(self):
        out = []
        for u in self.uygulamalar():
            ozel = []
            for ad in ("istek", "sayfalar"):
                r = self._yontem(u, ad)
                if r:
                    ozel += self.islev_yollari(u, u, r[0], r[1])
            gorulen = set()
            for y in ozel:  # aynı yol iki koşulda geçebilir (ör. /giris): bir kez
                k = (frozenset(y.yontemler), y.tur, y.deger, y.onek)
                if k not in gorulen:
                    gorulen.add(k)
                    out.append(y)
            out.append(Yol(u, {"*"}, "bilinmeyen", None))
        return out


def kayitlari_oku(dizin):
    for kok, _, adlar in os.walk(dizin):
        if os.path.samefile(kok, dizin):
            continue  # _krom.json
        for a in sorted(adlar):
            yol = os.path.join(kok, a)
            if a.endswith(".json"):
                with open(yol, encoding="utf-8") as f:
                    yield json.load(f)
            elif a.endswith(".json.gz"):
                with gzip.open(yol, "rt", encoding="utf-8") as f:
                    yield json.load(f)


def ana(arg):
    for akis in (sys.stdout, sys.stderr):
        akis.reconfigure(encoding="utf-8", errors="replace")
    dizin = arg[arg.index("--kayitlar") + 1] if "--kayitlar" in arg else KAYITLAR
    yollar = Cikarici().yollar()
    cagrilar = [(k["dosya"], k["test"], c) for k in kayitlari_oku(dizin) for c in k.get("yollar", [])]
    if not cagrilar:
        sys.exit(f"kayıt yok: {dizin}")
    for _, _, c in cagrilar:
        uygulama, yontem, yol = c[0], c[1], c[2]
        host, durum, yer = (c[3:6] + [None, None, None])[:3]
        adaylar = [y for y in yollar if y.uygulama == uygulama and y.tur != "bilinmeyen" and y.eslesir(yontem, yol, host)]
        oturumsuz = uygulama in ("Musteri", "Yonetim") and durum == 303 and yer == "/giris"
        for y in adaylar:
            if not oturumsuz or y.deger in ("/giris", "/cikis"):
                y.gorulme += 1
        if not adaylar and durum == 404:
            for y in yollar:
                if y.uygulama == uygulama and y.tur == "bilinmeyen":
                    y.gorulme += 1
    if "--hepsi" in arg:
        for y in yollar:
            print(f"{y.gorulme:6d}  {y.adi():50s} {y.kaynak}")
    eksik = [y for y in yollar if not y.gorulme]
    print(f"{len(yollar)} yol, {len(cagrilar)} çağrı; kapsanmayan {len(eksik)}")
    for y in eksik:
        print(f"  KAPSANMAYAN {y.adi()}")
    sys.exit(1 if eksik else 0)


if __name__ == "__main__":
    ana(sys.argv[1:])
