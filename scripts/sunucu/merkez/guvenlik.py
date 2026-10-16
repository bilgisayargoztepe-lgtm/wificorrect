"""Yönetim merkezi güvenliği (spec §3, §10): parola özetleri (cihazla aynı biçim), oturumlar, giriş kilidi, hız sınırı."""
import collections
import hashlib
import hmac
import re
import secrets
import threading

YINELEME = 120_000  # Rust hesap.rs ITER ile aynı: cihaz aynı özeti internetsiz doğrular
EN_AZ = 10
NUMARA_RE = re.compile(r"[1-9][0-9]{6}")  # 7 hane; yalnızca ASCII: \d Unicode rakamları da kabul eder (kilit atlatılırdı)
HARFLER = "abcdefghjkmnpqrstuvwxyzACDEFGHJKLMNPQRSTUVWXYZ2345679"  # 0/O, 1/l/I, 8/B karışmasın


def ozet(pw, tuz, yineleme):
    """PBKDF2-HMAC-SHA256; tuz metnin baytları (Rust: salt.as_bytes()), hex."""
    return hashlib.pbkdf2_hmac("sha256", pw.encode("utf-8"), tuz.encode("utf-8"), int(yineleme)).hex()


def yeni_kayit(pw):
    tuz = secrets.token_hex(16)
    return tuz, ozet(pw, tuz, YINELEME), YINELEME


def dogru(pw, tuz, oz, yineleme):
    return hmac.compare_digest(ozet(pw or "", tuz, yineleme), oz)


def bos_dogrulama(pw):
    """Olmayan hesapta da aynı süre geçsin (numara ya da kullanıcı adı yanıt süresinden anlaşılmasın)."""
    ozet(pw or "", "0" * 32, YINELEME)


def parola_uret(n=12):
    return "".join(secrets.choice(HARFLER) for _ in range(n))


class Oturumlar:
    """Bellek içi oturumlar: boşta 30 dk, en çok 12 saat. Servis yeniden başlarsa herkes yeniden girer."""

    def __init__(self, bosta=1800, en_cok=43200):
        self.bosta, self.en_cok = bosta, en_cok
        self._s = {}
        self.lock = threading.Lock()

    def create(self, user, role, now, surum="", ek=None):
        """surum: hesabın o anki parola özeti; değişince (sıfırlama, başka yerden değişim) oturum geçersizleşir.
        ek: oturuma bağlı ek alanlar (ör. AP oturumunun altındaki cihaz paneli oturumu); asıl alanları ezemez.
        Süresi dolmuş oturumlar burada silinir (bir daha bakılmayan oturum bellekte kalmasın)."""
        token = secrets.token_urlsafe(32)
        with self.lock:
            for t in [t for t, s in self._s.items() if now - s["last"] > self.bosta or now - s["created"] > self.en_cok]:
                del self._s[t]
            self._s[token] = {**(ek or {}), "user": user, "role": role, "csrf": secrets.token_urlsafe(24),
                              "created": now, "last": now, "gerekce": None, "surum": surum}
        return token

    def get(self, token, now):
        with self.lock:
            s = self._s.get(token or "")
            if s is None:
                return None
            if now - s["last"] > self.bosta or now - s["created"] > self.en_cok:
                del self._s[token]
                return None
            s["last"] = now
            return dict(s)

    def drop(self, token):
        with self.lock:
            self._s.pop(token or "", None)

    def drop_user(self, user):
        with self.lock:
            for t in [t for t, s in self._s.items() if s["user"] == user]:
                del self._s[t]

    def set_gerekce(self, token, metin, bitis):
        with self.lock:
            s = self._s.get(token or "")
            if s is not None:
                s["gerekce"] = (metin, bitis)


class GecisBelirtecleri:
    """panel → cihaz.wificorrect.com geçişi: tek kullanımlık, 60 sn (URL'de taşınır, çerez değil)."""
    SURE = 60

    def __init__(self):
        self._b, self.lock = {}, threading.Lock()

    def uret(self, numara, simdi, ozet="", rol="musteri"):
        """ozet: müşterinin o anki parola özeti; belirteç alındıktan sonra parola değişirse cihaz oturumu açılmaz.
        rol: "musteri" (panel.wificorrect.com) ya da "yonetici" (yönetimden doğrudan geçiş)."""
        t = secrets.token_urlsafe(32)
        with self.lock:
            self._b = {k: v for k, v in self._b.items() if v[1] > simdi}
            self._b[t] = (numara, simdi + self.SURE, ozet, rol)
        return t

    def tuket(self, token, simdi):
        """(numara, ozet, rol) ya da None."""
        with self.lock:
            v = self._b.pop(token or "", None)
        return (v[0], v[2], v[3]) if v and v[1] > simdi else None


class TekKullanimlik:
    """Kısa ömürlü, tek kullanımlık belirteç → değer (URL'de taşınır, çerez değil; ör. cihaz paneli → AP arayüzü geçişi).
    Süresi dolanlar her üretimde silinir."""

    def __init__(self, sure=60):
        self.sure = sure
        self._b, self.lock = {}, threading.Lock()

    def uret(self, deger, simdi):
        t = secrets.token_urlsafe(32)
        with self.lock:
            self._b = {k: v for k, v in self._b.items() if v[0] > simdi}
            self._b[t] = (simdi + self.sure, deger)
        return t

    def tuket(self, token, simdi):
        """Değer ya da None (yok, kullanılmış ya da süresi dolmuş)."""
        with self.lock:
            v = self._b.pop(token or "", None)
        return v[1] if v and v[0] > simdi else None


class GirisKilidi:
    """IP başına ve hesap başına kayan pencere: `limit` deneme `pencere` sn içinde → kilit. Deneme doğrulamadan ÖNCE
    sayılır; başarılı giriş yalnızca kendi denemesini ve o hesabın sayacını siler."""

    def __init__(self, limit=5, pencere=900):
        self.limit, self.pencere = limit, pencere
        self._f = {}
        self.lock = threading.Lock()

    def _son(self, k, now):
        self._f[k] = [t for t in self._f.get(k, []) if t > now - self.pencere]
        return self._f[k]

    def attempt(self, ip, user, now):
        keys = (("ip", ip), ("user", user))
        with self.lock:
            if any(len(self._son(k, now)) >= self.limit for k in keys):
                return False
            for k in keys:
                self._f[k].append(now)
            return True

    def succeed(self, ip, user, now):
        with self.lock:
            if now in self._f.get(("ip", ip), []):
                self._f[("ip", ip)].remove(now)
            self._f.pop(("user", user), None)


class HizSiniri:
    """Anahtar başına kayan pencere (API: IP başına dakikada 20 istek). Pencere başına bir kez, son isteği pencereden
    eski anahtarlar bellekten silinir (rastgele kimlik/IP ile bellek şişmesin)."""

    def __init__(self, limit=20, pencere=60):
        self.limit, self.pencere = limit, pencere
        self._z = collections.defaultdict(collections.deque)
        self.lock = threading.Lock()
        self._temizlik = None

    def izin(self, anahtar, now):
        with self.lock:
            if self._temizlik is None or now - self._temizlik >= self.pencere:
                self._temizlik = now
                for k in [k for k, q in self._z.items() if not q or q[-1] <= now - self.pencere]:
                    del self._z[k]
            q = self._z[anahtar]
            while q and q[0] <= now - self.pencere:
                q.popleft()
            if len(q) >= self.limit:
                return False
            q.append(now)
            return True
