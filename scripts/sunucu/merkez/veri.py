"""Yönetim merkezi veritabanı (spec §3): müşteriler, cihazlar, hareketler, eski arşiv bağları.
SQLite; tek bağlantı + kilit (ThreadingHTTPServer iş parçacıkları için). Yalnızca panel (wcpanel) açar, root açmaz."""
import contextlib
import datetime
import hashlib
import hmac
import json
import os
import secrets
import sqlite3
import threading
import time

import common
import guvenlik
import twilio as twilio_modulu

NUMARA_ARALIGI = (1_000_000, 9_999_999)  # 7 haneli, rastgele (sıralı değil: müşteri sayısı ve sırası anlaşılmasın)
SEMA = """
CREATE TABLE IF NOT EXISTS musteri (
  numara INTEGER PRIMARY KEY, tuz TEXT NOT NULL, ozet TEXT NOT NULL, yineleme INTEGER NOT NULL,
  not_ TEXT NOT NULL DEFAULT '', uyelik TEXT NOT NULL DEFAULT 'aktif', bitis TEXT NOT NULL DEFAULT '',
  olusturma TEXT NOT NULL, son_giris TEXT NOT NULL DEFAULT '',
  parola_acik TEXT NOT NULL DEFAULT '', lisans_bitis TEXT NOT NULL DEFAULT '', askida INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS cihaz (
  id INTEGER PRIMARY KEY AUTOINCREMENT, musteri INTEGER NOT NULL REFERENCES musteri(numara),
  durum TEXT NOT NULL, tunel_ip TEXT NOT NULL DEFAULT '', wg_pub TEXT NOT NULL, ssh_pub TEXT NOT NULL,
  anahtar_ozet TEXT NOT NULL, isletme_adi TEXT NOT NULL DEFAULT '', unvan TEXT NOT NULL DEFAULT '',
  surum TEXT NOT NULL DEFAULT '', son_eslesme TEXT NOT NULL DEFAULT '', baglanma TEXT NOT NULL,
  admin_parola TEXT NOT NULL DEFAULT '', admin_tuz TEXT NOT NULL DEFAULT '',
  admin_ozet TEXT NOT NULL DEFAULT '', admin_yineleme INTEGER NOT NULL DEFAULT 0,
  ayrilma TEXT NOT NULL DEFAULT '', isletme_turu TEXT NOT NULL DEFAULT '', adres TEXT NOT NULL DEFAULT '');
CREATE UNIQUE INDEX IF NOT EXISTS cihaz_tek ON cihaz(musteri) WHERE durum != 'serbest';
CREATE TABLE IF NOT EXISTS hareket (
  id INTEGER PRIMARY KEY AUTOINCREMENT, zaman TEXT NOT NULL, kim TEXT NOT NULL, ip TEXT NOT NULL,
  olay TEXT NOT NULL, musteri INTEGER, ayrinti TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS hareket_musteri ON hareket(musteri, id);
CREATE TABLE IF NOT EXISTS eski_arsiv (ad TEXT PRIMARY KEY, musteri INTEGER REFERENCES musteri(numara));
CREATE TABLE IF NOT EXISTS ayar (anahtar TEXT PRIMARY KEY, deger TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS saglik (cihaz INTEGER PRIMARY KEY, zaman TEXT NOT NULL, rapor TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sorun (
  id INTEGER PRIMARY KEY AUTOINCREMENT, musteri INTEGER NOT NULL, tur TEXT NOT NULL, metin TEXT NOT NULL,
  onem TEXT NOT NULL, acilis TEXT NOT NULL, kapanis TEXT NOT NULL DEFAULT '',
  acilis_sms TEXT NOT NULL DEFAULT '', kapanis_sms TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS sorun_musteri ON sorun(musteri, kapanis);
CREATE TABLE IF NOT EXISTS surum (
  surum TEXT PRIMARY KEY, sha256 TEXT NOT NULL, boyut INTEGER NOT NULL, kaynak TEXT NOT NULL, zaman TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sms_sayac (
  musteri INTEGER NOT NULL, gun TEXT NOT NULL, sayi INTEGER NOT NULL, PRIMARY KEY (musteri, gun));
CREATE TABLE IF NOT EXISTS sms_son (
  musteri INTEGER PRIMARY KEY, zaman TEXT NOT NULL, saglayici TEXT NOT NULL, sonuc TEXT NOT NULL, kod TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS bekleyen (
  kimlik TEXT PRIMARY KEY, sir_ozet TEXT NOT NULL, ilk REAL NOT NULL, son REAL NOT NULL, ip TEXT NOT NULL,
  surum TEXT NOT NULL DEFAULT '', wan_mac TEXT NOT NULL DEFAULT '', wg_pub TEXT NOT NULL, ssh_pub TEXT NOT NULL,
  musteri INTEGER, ayrilma REAL NOT NULL DEFAULT 0, belirtec_ozet TEXT NOT NULL DEFAULT '',
  belirtec_bitis REAL NOT NULL DEFAULT 0, cihaz INTEGER, baglandi REAL NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS bekleyen_ip ON bekleyen(ip, son);
"""
# Kolay bağlama (spec 2026-10-16): bağlanmamış cihazın bekleme kaydı
BEKLEYEN_SIL_SN = 7 * 86400  # bu kadar bildirmeyen kayıt silinir
BEKLEYEN_ESKI_SN = 1800  # yönetimde "eski"
BEKLEYEN_ADAY_SN = 600  # müşteri girişinde aday: son bu kadar süre içinde bildirmiş
AYRILMA_SN = 900  # ayrılmış ama bu sürede bağlanmamış cihaz serbest kalır
BELIRTEC_SN = 600  # tek kullanımlık bağlama belirteci
BEKLEYEN_EN_COK = 2000
CIHAZ_ALANLARI = {"tunel_ip", "isletme_adi", "unvan", "adres", "surum", "son_eslesme", "isletme_turu"}
# Cihazın eşitlemede bildirdiği işletme türü (cihaz ayarı main.isletme_turu); başka değer saklanmaz
ISLETME_TURLERI = {"kafe": "Kafe/Restoran", "otel": "Otel/Pansiyon", "yurt": "Yurt", "diger": "Diğer"}
# Sonradan eklenen müşteri sütunları (eski veritabanına ALTER ile eklenir)
YENI_SUTUNLAR = (("parola_acik", "TEXT NOT NULL DEFAULT ''"), ("lisans_bitis", "TEXT NOT NULL DEFAULT ''"),
                 ("askida", "INTEGER NOT NULL DEFAULT 0"), ("guncelleme_kanali", "TEXT NOT NULL DEFAULT 'kararli'"),
                 ("guncelleme_hemen", "REAL NOT NULL DEFAULT 0"), ("guncelleme_hemen_surum", "TEXT NOT NULL DEFAULT ''"))
GUNCELLEME_KANALLARI = ("kararli", "test", "kapali")
YENI_CIHAZ_SUTUNLARI = (("admin_parola", "TEXT NOT NULL DEFAULT ''"), ("admin_tuz", "TEXT NOT NULL DEFAULT ''"),
                        ("admin_ozet", "TEXT NOT NULL DEFAULT ''"), ("admin_yineleme", "INTEGER NOT NULL DEFAULT 0"),
                        ("isletme_turu", "TEXT NOT NULL DEFAULT ''"), ("adres", "TEXT NOT NULL DEFAULT ''"))
UYARI_VARSAYILAN = {"telefonlar": [], "acik": False, "gunluk_sinir": 20}
UYARI_TELEFON_EN_COK = 3
UYARI_SINIR_ARALIK = (2, 100)  # günlük uyarı SMS sınırı (yönetim formu da bunu kullanır)
SORUN_SMS_ALANLARI = ("acilis_sms", "kapanis_sms")
SMS_ALANLARI = ("usercode", "password", "msgheader", "appkey")
TR_SAGLAYICILAR = ("netgsm", "twilio")
TWILIO_ALANLARI = ("account_sid", "auth_token", "api_key_sid", "api_key_secret", "verify_sid")
TWILIO_SIRLARI = ("auth_token", "api_key_secret")
TWILIO_VARSAYILAN = {"acik": False, **{k: "" for k in TWILIO_ALANLARI}}
LISANS_GUN = 365
# Raporlar: müşteri → bağlı eski arşivler (root yalnızca bunu görür, veritabanını açmaz; rapor.gruplar güvenilmez girdi sayar)
RAPOR_GRUPLARI = "/var/lib/wificorrect/merkez/rapor-gruplari.json"


def sms_hazir(s):
    """Seçili Türk numarası sağlayıcısının ayarı tam mı (deneme modu kapalıyken gerçekten gönderilebilir mi)."""
    if s.get("tr_saglayici", "netgsm") == "twilio":
        t = s.get("twilio") or {}
        return bool(t.get("acik") and twilio_modulu.SID_RE["verify_sid"].fullmatch(str(t.get("verify_sid") or ""))
                    and twilio_modulu.kimlik(t))
    return all(s.get(k) for k in ("usercode", "password", "msgheader"))


def _yil_sonra(gun):
    return (gun + datetime.timedelta(days=LISANS_GUN)).isoformat()


class BaskaCihaz(Exception):
    """Numaranın başka (serbest bırakılmamış) cihazı var ya da bu cihaz başka müşteriye bağlı."""


def anahtar_ozeti(anahtar):
    return hashlib.sha256(anahtar.encode("utf-8")).hexdigest()


class Veri:
    def __init__(self, yol, saat=time.time):
        self.saat = saat
        self.lock = threading.Lock()
        self.db = sqlite3.connect(yol, check_same_thread=False, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.execute("PRAGMA foreign_keys = ON")
        self.db.execute("PRAGMA secure_delete = ON")  # değişen açık parolalar dosyanın boş sayfalarında kalmasın
        self.db.executescript(SEMA)
        self._gocur()

    def _gocur(self):
        """Eski veritabanı: eksik sütunlar eklenir; lisansı olmayan müşteriye oluşturmadan 1 yıl verilir."""
        var = {r[1] for r in self.db.execute("PRAGMA table_info(musteri)")}
        for ad, tip in YENI_SUTUNLAR:
            if ad not in var:
                self.db.execute(f"ALTER TABLE musteri ADD COLUMN {ad} {tip}")
        for r in self.db.execute("SELECT numara, olusturma FROM musteri WHERE lisans_bitis = ''").fetchall():
            self.db.execute("UPDATE musteri SET lisans_bitis = ? WHERE numara = ?",
                            (_yil_sonra(datetime.date.fromisoformat(r["olusturma"][:10])), r["numara"]))
        var_c = {r[1] for r in self.db.execute("PRAGMA table_info(cihaz)")}
        for ad, tip in YENI_CIHAZ_SUTUNLARI:
            if ad not in var_c:
                self.db.execute(f"ALTER TABLE cihaz ADD COLUMN {ad} {tip}")

    def bugun(self):
        return datetime.datetime.fromtimestamp(self.saat(), common.TZ).date()

    def _simdi(self):
        return common.now_iso(self.saat())

    @contextlib.contextmanager
    def _islem(self):
        """Kilitli tek işlem; içinde self._bir/_hepsi çağrılmaz (kilit yeniden girilmez), db doğrudan kullanılır."""
        with self.lock:
            self.db.execute("BEGIN IMMEDIATE")
            try:
                yield self.db
            except BaseException:
                self.db.execute("ROLLBACK")
                raise
            self.db.execute("COMMIT")

    def _bir(self, sql, *a):
        with self.lock:
            return self.db.execute(sql, a).fetchone()

    def _hepsi(self, sql, *a):
        with self.lock:
            return self.db.execute(sql, a).fetchall()

    # --- müşteriler ---
    def musteri_ekle(self, not_="", parola=None):
        """Parolayı yönetici yazar (2026-10-08, kullanıcı kararı); verilmezse (testler) rastgele."""
        if parola is not None and (not isinstance(parola, str) or len(parola) < guvenlik.EN_AZ):
            raise ValueError(f"Parola en az {guvenlik.EN_AZ} karakter olmalı.")
        pw = parola if parola is not None else guvenlik.parola_uret()
        tuz, oz, y = guvenlik.yeni_kayit(pw)
        with self._islem() as db:
            while True:
                numara = NUMARA_ARALIGI[0] + secrets.randbelow(NUMARA_ARALIGI[1] - NUMARA_ARALIGI[0] + 1)
                if db.execute("SELECT 1 FROM musteri WHERE numara = ?", (numara,)).fetchone() is None:
                    break
            db.execute("INSERT INTO musteri (numara, tuz, ozet, yineleme, not_, olusturma, parola_acik, lisans_bitis) "
                       "VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                       (numara, tuz, oz, y, (not_ or "").strip()[:200], self._simdi(), pw, _yil_sonra(self.bugun())))
        return numara, pw

    def musteri(self, numara):
        return self._bir("SELECT * FROM musteri WHERE numara = ?", numara)

    def musteriler(self):
        return self._hepsi("SELECT * FROM musteri ORDER BY numara")

    def parola_dogrula(self, numara, pw):
        m = self.musteri(numara) if type(numara) is int else None
        if m is None:
            guvenlik.bos_dogrulama(pw)
            return None
        return m if guvenlik.dogru(pw, m["tuz"], m["ozet"], m["yineleme"]) else None

    def parola_koy(self, numara, pw):
        if not isinstance(pw, str) or len(pw) < guvenlik.EN_AZ:
            raise ValueError(f"Parola en az {guvenlik.EN_AZ} karakter olmalı.")
        tuz, oz, y = guvenlik.yeni_kayit(pw)
        with self._islem() as db:
            # ponytail: parola yönetimde gösterilsin diye (kullanıcı kararı 2026-10-04) açık saklanır; veritabanı yalnızca
            # wcpanel'in (600). Sızarsa müşteri parolaları da sızar — şifreli saklama ya da yalnızca sıfırlama ile değiştirilebilir.
            db.execute("UPDATE musteri SET tuz = ?, ozet = ?, yineleme = ?, parola_acik = ? WHERE numara = ?", (tuz, oz, y, pw, numara))
        return tuz, oz, y

    def parola_sifirla(self, numara):
        pw = guvenlik.parola_uret()
        self.parola_koy(numara, pw)
        return pw

    def giris_kaydet(self, numara):
        with self._islem() as db:
            db.execute("UPDATE musteri SET son_giris = ? WHERE numara = ?", (self._simdi(), numara))

    def uyelik(self, numara, durum):
        if durum not in ("aktif", "bitti"):
            raise ValueError(durum)
        with self._islem() as db:
            db.execute("UPDATE musteri SET uyelik = ?, bitis = ? WHERE numara = ?",
                       (durum, self._simdi() if durum == "bitti" else "", numara))

    # --- lisans (yıllık; ödeme ekranı sonradan lisans_uzat çağırır) ---
    def lisans_durumu(self, m):
        """(durum, bitiş): askida | bitti | aktif."""
        bitis = m["lisans_bitis"]
        if m["askida"]:
            return "askida", bitis
        if bitis and bitis < self.bugun().isoformat():
            return "bitti", bitis
        return "aktif", bitis

    def lisans_uzat(self, numara, gun=LISANS_GUN):
        """Bitişin üstüne (süresi geçmişse bugünden) `gun` gün ekler. Dönen: yeni bitiş."""
        with self._islem() as db:
            m = db.execute("SELECT lisans_bitis FROM musteri WHERE numara = ?", (numara,)).fetchone()
            taban = self.bugun()
            if m and m["lisans_bitis"]:
                taban = max(taban, datetime.date.fromisoformat(m["lisans_bitis"]))
            yeni = (taban + datetime.timedelta(days=gun)).isoformat()
            db.execute("UPDATE musteri SET lisans_bitis = ? WHERE numara = ?", (yeni, numara))
        return yeni

    def lisans_ayarla(self, numara, tarih):
        """Dönen: kaydedilen tarih (YYYY-AA-GG). Geçersizse ValueError."""
        tarih = datetime.date.fromisoformat(tarih).isoformat()
        with self._islem() as db:
            db.execute("UPDATE musteri SET lisans_bitis = ? WHERE numara = ?", (tarih, numara))
        return tarih

    def askiya_al(self, numara, askida):
        with self._islem() as db:
            db.execute("UPDATE musteri SET askida = ? WHERE numara = ?", (1 if askida else 0, numara))

    # --- cihazlar ---
    def bagli_cihaz(self, numara):
        return self._bir("SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", numara)

    def cihaz_bagla(self, numara, wg_pub, ssh_pub, isletme_adi="", unvan="", surum=""):
        """Müşterinin serbest bırakılmamış cihazı yoksa bu cihazı bağlar; aynı cihaz (aynı wg_pub) yeniden denerse yeni
        anahtar verir (eskisi geçersizleşir). Dönen: (cihaz_id, cihaz_anahtari) — anahtar yalnızca burada açık görünür."""
        anahtar = secrets.token_urlsafe(32)
        with self._islem() as db:
            baska = db.execute("SELECT 1 FROM cihaz WHERE wg_pub = ? AND durum != 'serbest' AND musteri != ?",
                               (wg_pub, numara)).fetchone()
            c = db.execute("SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", (numara,)).fetchone()
            if baska or (c is not None and (c["wg_pub"] != wg_pub or c["durum"] != "bagli")):
                raise BaskaCihaz()
            if c is not None:
                db.execute("UPDATE cihaz SET anahtar_ozet = ?, ssh_pub = ?, isletme_adi = ?, unvan = ?, surum = ? WHERE id = ?",
                           (anahtar_ozeti(anahtar), ssh_pub, isletme_adi, unvan, surum, c["id"]))
                return c["id"], anahtar
            cur = db.execute(
                "INSERT INTO cihaz (musteri, durum, wg_pub, ssh_pub, anahtar_ozet, isletme_adi, unvan, surum, baglanma) "
                "VALUES (?, 'bagli', ?, ?, ?, ?, ?, ?, ?)",
                (numara, wg_pub, ssh_pub, anahtar_ozeti(anahtar), isletme_adi, unvan, surum, self._simdi()))
            return cur.lastrowid, anahtar

    def cihaz_anahtarla(self, anahtar):
        if not isinstance(anahtar, str) or not anahtar:
            return None
        return self._bir("SELECT * FROM cihaz WHERE anahtar_ozet = ? AND durum != 'serbest'", anahtar_ozeti(anahtar))

    def cihaz_guncelle(self, cid, **alanlar):
        if not alanlar or not set(alanlar) <= CIHAZ_ALANLARI:
            raise ValueError(sorted(alanlar))
        with self._islem() as db:
            db.execute(f"UPDATE cihaz SET {', '.join(k + ' = ?' for k in alanlar)} WHERE id = ?", (*alanlar.values(), cid))

    def eslesme_kaydet(self, cid, isletme_adi, unvan, surum, isletme_turu=None, adres=None):
        """isletme_turu yalnızca izinli değerse yazılır; yoksa ya da bozuksa eskisi kalır. adres (veri sorumlusunun
        adresi) yalnızca cihaz gönderdiyse yazılır (eski sürümler göndermez)."""
        ek = {"isletme_turu": isletme_turu} if isinstance(isletme_turu, str) and isletme_turu in ISLETME_TURLERI else {}
        if adres is not None:
            ek["adres"] = adres
        self.cihaz_guncelle(cid, isletme_adi=isletme_adi, unvan=unvan, surum=surum, son_eslesme=self._simdi(), **ek)

    def serbest_birak(self, numara, zorla=False):
        """Dönen: etkilenen cihaz satırı (yoksa None). Zorla: cihaz beklenmeden serbest (arızalı/kayıp cihaz)."""
        with self._islem() as db:
            c = db.execute("SELECT * FROM cihaz WHERE musteri = ? AND durum != 'serbest'", (numara,)).fetchone()
            if c is None:
                return None
            if zorla:
                db.execute("UPDATE cihaz SET durum = 'serbest', ayrilma = ? WHERE id = ?", (self._simdi(), c["id"]))
            else:
                db.execute("UPDATE cihaz SET durum = 'serbest_birakiliyor' WHERE id = ?", (c["id"],))
            return c

    def serbest_iptal(self, numara):
        """Henüz tamamlanmamış serbest bırakmayı geri alır (cihaz temizlenmeden önce)."""
        with self._islem() as db:
            db.execute("UPDATE cihaz SET durum = 'bagli' WHERE musteri = ? AND durum = 'serbest_birakiliyor'", (numara,))

    def temizlendi(self, cid):
        with self._islem() as db:
            db.execute("UPDATE cihaz SET durum = 'serbest', ayrilma = ? WHERE id = ?", (self._simdi(), cid))

    # --- kolay bağlama (spec 2026-10-16): bağlanmamış cihazların bekleme kayıtları ---
    @staticmethod
    def _atama(db, r, simdi, wg_pub=None):
        """Kaydın müşteriye ayrılması hâlâ geçerli mi: "bagli" (bu kayıttan bağlanmış, bağ aynı wg anahtarıyla duruyor —
        cihaz cevabı kaybettiyse yeniden bağlanır), "ayrildi" (AYRILMA_SN içinde, müşterinin bağlı cihazı yok, üyeliği
        aktif) ya da None."""
        if r["musteri"] is None:
            return None
        if r["cihaz"] is not None:
            c = db.execute("SELECT * FROM cihaz WHERE id = ?", (r["cihaz"],)).fetchone()
            ok = (c is not None and c["durum"] == "bagli" and c["musteri"] == r["musteri"]
                  and c["wg_pub"] == (wg_pub or r["wg_pub"]))
            return "bagli" if ok else None
        if simdi - r["ayrilma"] > AYRILMA_SN:
            return None
        m = db.execute("SELECT uyelik FROM musteri WHERE numara = ?", (r["musteri"],)).fetchone()
        if m is None or m["uyelik"] != "aktif":
            return None
        if db.execute("SELECT 1 FROM cihaz WHERE musteri = ? AND durum != 'serbest'", (r["musteri"],)).fetchone():
            return None
        return "ayrildi"

    def _aday_mi(self, db, r, simdi):
        """Kimseye (geçerli olarak) ayrılmamış ve wg anahtarı bağlı bir cihazda olmayan kayıt."""
        if self._atama(db, r, simdi) is not None:
            return False
        return db.execute("SELECT 1 FROM cihaz WHERE wg_pub = ? AND durum != 'serbest'", (r["wg_pub"],)).fetchone() is None

    def bekleyen_bildir(self, kimlik, sir, ip, surum, wan_mac, wg_pub, ssh_pub):
        """Cihazın "bekliyorum" bildirimi. Dönen: (durum, yeni_mi, belirteç): durum "bekle" | "baglan" (belirteç açık,
        yalnızca burada) | "taninmadi" (sır tutmadı, kayıt değişmez) | "dolu" (tablo sınırı). İlk bildirim sırrın özetini
        kaydeder; sonrakiler aynı sırrı kanıtlamalıdır."""
        simdi = self.saat()
        oz = anahtar_ozeti(sir)
        with self._islem() as db:
            db.execute("DELETE FROM bekleyen WHERE son < ?", (simdi - BEKLEYEN_SIL_SN,))
            r = db.execute("SELECT * FROM bekleyen WHERE kimlik = ?", (kimlik,)).fetchone()
            if r is None:
                if db.execute("SELECT COUNT(*) FROM bekleyen").fetchone()[0] >= BEKLEYEN_EN_COK:
                    return "dolu", False, None
                db.execute("INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, surum, wan_mac, wg_pub, ssh_pub) "
                           "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)", (kimlik, oz, simdi, simdi, ip, surum, wan_mac, wg_pub, ssh_pub))
                return "bekle", True, None
            if not hmac.compare_digest(r["sir_ozet"], oz):
                return "taninmadi", False, None
            db.execute("UPDATE bekleyen SET son = ?, ip = ?, surum = ?, wan_mac = ?, wg_pub = ?, ssh_pub = ? WHERE kimlik = ?",
                       (simdi, ip, surum, wan_mac, wg_pub, ssh_pub, kimlik))
            if self._atama(db, r, simdi, wg_pub) is None:
                if r["musteri"] is not None:  # süresi dolmuş / bozulmuş ayrılma düşer
                    db.execute("UPDATE bekleyen SET musteri = NULL, ayrilma = 0, cihaz = NULL, baglandi = 0, "
                               "belirtec_ozet = '', belirtec_bitis = 0 WHERE kimlik = ?", (kimlik,))
                return "bekle", False, None
            t = secrets.token_urlsafe(32)
            db.execute("UPDATE bekleyen SET belirtec_ozet = ?, belirtec_bitis = ? WHERE kimlik = ?",
                       (anahtar_ozeti(t), simdi + BELIRTEC_SN, kimlik))
            return "baglan", False, t

    def bekleyen_var(self, kimlik):
        return self._bir("SELECT 1 FROM bekleyen WHERE kimlik = ?", kimlik) is not None

    def bekleyen_adaylar(self, ip):
        """Müşterinin tarayıcısıyla aynı internet adresinden son BEKLEYEN_ADAY_SN içinde bildirmiş, ayrılabilir kayıtlar."""
        simdi = self.saat()
        with self.lock:
            rows = self.db.execute("SELECT * FROM bekleyen WHERE ip = ? AND son >= ? ORDER BY ilk, kimlik",
                                   (ip, simdi - BEKLEYEN_ADAY_SN)).fetchall()
            return [dict(r) for r in rows if self._aday_mi(self.db, r, simdi)]

    def bekleyen_ayir(self, kimlik, numara, ip):
        """Kaydı müşteriye ayırır (hâlâ aynı IP'den aday ve müşterinin bağlı cihazı yoksa). Müşterinin bağlanmamış
        başka ayrılmaları düşer. Dönen: başarılı mı."""
        simdi = self.saat()
        with self._islem() as db:
            r = db.execute("SELECT * FROM bekleyen WHERE kimlik = ? AND ip = ? AND son >= ?",
                           (kimlik, ip, simdi - BEKLEYEN_ADAY_SN)).fetchone()
            if r is None or not self._aday_mi(db, r, simdi):
                return False
            m = db.execute("SELECT uyelik FROM musteri WHERE numara = ?", (numara,)).fetchone()
            if m is None or m["uyelik"] != "aktif":
                return False
            if db.execute("SELECT 1 FROM cihaz WHERE musteri = ? AND durum != 'serbest'", (numara,)).fetchone():
                return False
            db.execute("UPDATE bekleyen SET musteri = NULL, ayrilma = 0, belirtec_ozet = '', belirtec_bitis = 0 "
                       "WHERE musteri = ? AND cihaz IS NULL", (numara,))
            db.execute("UPDATE bekleyen SET musteri = ?, ayrilma = ?, cihaz = NULL, baglandi = 0, belirtec_ozet = '', "
                       "belirtec_bitis = 0 WHERE kimlik = ?", (numara, simdi, kimlik))
            return True

    def bekleyen_musterinin(self, numara):
        """Müşteriye geçerli olarak ayrılmış (ya da ondan bağlanmış) kayıt; {"atama": "ayrildi"|"bagli", …} ya da None."""
        simdi = self.saat()
        with self.lock:
            for r in self.db.execute("SELECT * FROM bekleyen WHERE musteri = ? ORDER BY ayrilma DESC", (numara,)).fetchall():
                a = self._atama(self.db, r, simdi)
                if a:
                    return dict(r, atama=a)
        return None

    def bekleyen_belirtec_tuket(self, kimlik, sir, belirtec):
        """Bağlama belirteci: sır ve belirteç tutuyor, süresi geçmemiş, kayıt bir müşteriye ayrılmışsa kaydı döner;
        belirteç her durumda tüketilir (tek kullanımlık). Değilse None."""
        simdi = self.saat()
        with self._islem() as db:
            r = db.execute("SELECT * FROM bekleyen WHERE kimlik = ?", (kimlik,)).fetchone()
            if r is None or not hmac.compare_digest(r["sir_ozet"], anahtar_ozeti(sir)) or not r["belirtec_ozet"]:
                return None
            db.execute("UPDATE bekleyen SET belirtec_ozet = '', belirtec_bitis = 0 WHERE kimlik = ?", (kimlik,))
            if not hmac.compare_digest(r["belirtec_ozet"], anahtar_ozeti(belirtec)) or simdi > r["belirtec_bitis"]:
                return None
            return dict(r) if r["musteri"] is not None else None

    def bekleyen_baglandi(self, kimlik, cid):
        with self._islem() as db:
            db.execute("UPDATE bekleyen SET cihaz = ?, baglandi = ? WHERE kimlik = ?", (cid, self.saat(), kimlik))

    def bekleyenler(self):
        """Yönetim listesi (yeniden eskiye); BEKLEYEN_SIL_SN'den eski kayıtlar önce silinir."""
        with self._islem() as db:
            db.execute("DELETE FROM bekleyen WHERE son < ?", (self.saat() - BEKLEYEN_SIL_SN,))
            return [dict(r) for r in db.execute("SELECT * FROM bekleyen ORDER BY son DESC, kimlik")]

    # --- sürüm deposu ve güncelleme kanalları (uzaktan güncelleme) ---
    @staticmethod
    def _surum_sirasi(s):
        return tuple(int(x) for x in s.split("."))

    def surum_ekle(self, surum, sha256, boyut, kaynak):
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO surum (surum, sha256, boyut, kaynak, zaman) VALUES (?, ?, ?, ?, ?)",
                       (surum, sha256, int(boyut), kaynak, self._simdi()))

    def surumler(self):
        """Yeniden eskiye, sürüm numarasına göre (0.10.0 > 0.9.2)."""
        satirlar = [dict(r) for r in self._hepsi("SELECT * FROM surum")]
        return sorted(satirlar, key=lambda r: self._surum_sirasi(r["surum"]), reverse=True)

    def surum(self, surum):
        r = self._bir("SELECT * FROM surum WHERE surum = ?", surum)
        return dict(r) if r else None

    def kanal_hedef(self, kanal):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = ?", "kanal_" + kanal)
        return r["deger"] if r else None

    def kanal_hedef_koy(self, kanal, surum):
        if kanal not in GUNCELLEME_KANALLARI or kanal == "kapali":
            raise ValueError(kanal)
        with self._islem() as db:
            if surum is None:
                db.execute("DELETE FROM ayar WHERE anahtar = ?", ("kanal_" + kanal,))
            else:
                db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES (?, ?)", ("kanal_" + kanal, surum))

    def guncelleme_kanali_koy(self, numara, kanal):
        if kanal not in GUNCELLEME_KANALLARI:
            raise ValueError(kanal)
        with self._islem() as db:
            db.execute("UPDATE musteri SET guncelleme_kanali = ? WHERE numara = ?", (kanal, numara))

    def guncelleme_hemen_koy(self, numara, epoch, surum=""):
        """"Şimdi güncelle" bayrağı, basıldığı andaki hedef sürümle; epoch 0 = bayrak yok."""
        with self._islem() as db:
            db.execute("UPDATE musteri SET guncelleme_hemen = ?, guncelleme_hemen_surum = ? WHERE numara = ?",
                       (float(epoch), surum if epoch else "", numara))

    def hizmet_acik(self, m):
        """Üyelik aktif, askıda değil, lisans süresi dolmamış."""
        return m["uyelik"] == "aktif" and self.lisans_durumu(m)[0] == "aktif"

    def cihaz_hedefi(self, m):
        """Cihazın kurması istenen paket {surum, sha256, boyut, hemen} ya da None (kanal kapalı/hedefsiz, lisans ya da üyelik kapalı)."""
        kanal = m["guncelleme_kanali"]
        if kanal == "kapali" or not self.hizmet_acik(m):
            return None
        hedef = self.kanal_hedef(kanal)
        k = self.surum(hedef) if hedef else None
        if k is None:
            return None
        hemen = m["guncelleme_hemen"] > 0 and m["guncelleme_hemen_surum"] == k["surum"]  # bayrak başka hedef içinse geçmez
        return {"surum": k["surum"], "sha256": k["sha256"], "boyut": k["boyut"], "hemen": hemen}

    # --- bütün cihazlarda tek admin parolası (2026-10-08, kullanıcı kararı): yalnızca özet saklanır ---
    # ponytail: cihaz başına admin_* sütunları (2026-10-05) artık kullanılmıyor; eski veritabanında boş durur.
    def admin_parolasi(self):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = 'admin'")
        return json.loads(r["deger"]) if r else None

    def admin_parolasi_koy(self, pw):
        if not isinstance(pw, str) or len(pw) < guvenlik.EN_AZ:
            raise ValueError(f"Parola en az {guvenlik.EN_AZ} karakter olmalı.")
        tuz, oz, y = guvenlik.yeni_kayit(pw)
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('admin', ?)",
                       (json.dumps({"tuz": tuz, "ozet": oz, "yineleme": y}),))

    # --- SMS ayarı: NetGSM (eski cihazlara da gider) + doğrulama SMS'i sağlayıcı seçimi ve Twilio (yalnızca merkezde) ---
    def sms_ayari(self):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = 'sms'")
        if not r:
            return None
        a = json.loads(r["deger"])
        a.setdefault("tr_saglayici", "netgsm")
        a["twilio"] = {**TWILIO_VARSAYILAN, **(a.get("twilio") or {})}
        return a

    def sms_ayari_koy(self, d):
        a = {"mock": bool(d.get("mock"))}
        for k in SMS_ALANLARI:
            a[k] = str(d.get(k) or "").strip()
        a["tr_saglayici"] = d.get("tr_saglayici") or "netgsm"
        if a["tr_saglayici"] not in TR_SAGLAYICILAR:
            raise ValueError("Türk numaraları için sağlayıcı netgsm ya da twilio olmalı.")
        tw = d.get("twilio") or {}
        t = {"acik": bool(tw.get("acik"))}
        for k in TWILIO_ALANLARI:
            t[k] = str(tw.get(k) or "").strip()
        for k, desen in twilio_modulu.SID_RE.items():
            if t[k] and not desen.fullmatch(t[k]):
                raise ValueError(f"Twilio {k} biçimi geçersiz.")
        if a["tr_saglayici"] == "twilio" and not t["acik"]:
            raise ValueError("Türk numaraları için Twilio seçildiyse Twilio açık olmalı.")
        if t["acik"] and not (t["account_sid"] and t["verify_sid"] and twilio_modulu.kimlik(t)):
            raise ValueError("Twilio açıkken account_sid, verify_sid ve auth_token (ya da API anahtarı sid + secret) gerekli.")
        eksik = [k for k in ("usercode", "password", "msgheader") if not a[k]]
        if not a["mock"] and a["tr_saglayici"] == "netgsm" and eksik:
            raise ValueError(f"Deneme modu kapalıyken şu alanlar gerekli: {', '.join(eksik)}")
        a["twilio"] = t
        with self._islem() as db:
            # ponytail: NetGSM şifresi ve Twilio sırları açık saklanır (sağlayıcıya gönderilmesi gerekir); veritabanı
            # yalnızca wcpanel'in (600). NetGSM bilgisi eski sürüm cihazlara da gider; Twilio hiçbir cihaza gitmez.
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('sms', ?)", (json.dumps(a, ensure_ascii=False),))

    def sms_etkin_mock(self):
        """Cihaza bildirilen etkin deneme modu: ayar yok, deneme modu açık ya da seçili sağlayıcının ayarı eksik."""
        s = self.sms_ayari()
        return s is None or bool(s["mock"]) or not sms_hazir(s)

    # --- müşteri (cihaz) başına günlük doğrulama SMS'i sayacı (/api/sms; kod saklanmaz). Yabancı numaralar ayrıca
    # `<gün>/yabanci` anahtarında sayılır (daha düşük sınır) ---
    def sms_sayac(self, musteri, gun):
        r = self._bir("SELECT sayi FROM sms_sayac WHERE musteri = ? AND gun = ?", musteri, gun)
        return r["sayi"] if r else 0

    def sms_sayac_artir(self, musteri, gun):
        """Bir artırır, yeni değeri döner."""
        with self._islem() as db:
            db.execute("INSERT INTO sms_sayac (musteri, gun, sayi) VALUES (?, ?, 1) "
                       "ON CONFLICT(musteri, gun) DO UPDATE SET sayi = sayi + 1", (musteri, gun))
            return db.execute("SELECT sayi FROM sms_sayac WHERE musteri = ? AND gun = ?", (musteri, gun)).fetchone()["sayi"]

    # --- müşterinin son doğrulama SMS'i sonucu (hareket satırı yerine tek satır; kod ve numara yok) ---
    def sms_son_kaydet(self, musteri, saglayici, sonuc, kod):
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO sms_son (musteri, zaman, saglayici, sonuc, kod) VALUES (?, ?, ?, ?, ?)",
                       (musteri, self._simdi(), saglayici or "", sonuc, str(kod)[:16]))

    def sms_son(self, musteri):
        r = self._bir("SELECT * FROM sms_son WHERE musteri = ?", musteri)
        return dict(r) if r else None

    # --- sağlık raporu, sorun kaydı, uyarı ayarı ---
    def saglik_kaydet(self, cid, rapor):
        """Cihaz başına tek satır; her rapor öncekinin üzerine yazar."""
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO saglik (cihaz, zaman, rapor) VALUES (?, ?, ?)",
                       (cid, self._simdi(), json.dumps(rapor, ensure_ascii=False)))

    def saglik(self, cid):
        r = self._bir("SELECT zaman, rapor FROM saglik WHERE cihaz = ?", cid)
        return {"zaman": r["zaman"], "rapor": json.loads(r["rapor"])} if r else None

    def sorun_ac(self, musteri, tur, metin, onem):
        """Aynı müşteri+tür için açık sorun varsa onun id'si döner, yenisi açılmaz."""
        with self._islem() as db:
            r = db.execute("SELECT id FROM sorun WHERE musteri = ? AND tur = ? AND kapanis = ''", (musteri, tur)).fetchone()
            if r:
                return r["id"]
            return db.execute("INSERT INTO sorun (musteri, tur, metin, onem, acilis) VALUES (?, ?, ?, ?, ?)",
                              (musteri, tur, metin, onem, self._simdi())).lastrowid

    def sorun_kapat(self, sorun_id):
        with self._islem() as db:
            db.execute("UPDATE sorun SET kapanis = ? WHERE id = ? AND kapanis = ''", (self._simdi(), sorun_id))

    def sorun_sms(self, sorun_id, alan, deger):
        """alan: acilis_sms | kapanis_sms; değer: gonderildi|mock|hata:<kod>|sinir:<gün>|yok (kapanış SMS'i gerekmedi)."""
        if alan not in SORUN_SMS_ALANLARI:
            raise ValueError("Geçersiz SMS alanı.")
        with self._islem() as db:
            db.execute(f"UPDATE sorun SET {alan} = ? WHERE id = ?", (str(deger)[:64], sorun_id))

    def sorun_metin(self, sorun_id, metin):
        """Açık sorunun metni değişti (ör. disk yüzdesi): güncellenir, açılış zamanı ve SMS durumu aynı kalır."""
        with self._islem() as db:
            db.execute("UPDATE sorun SET metin = ? WHERE id = ?", (str(metin)[:200], sorun_id))

    def sorun_yeniden_ac(self, sorun_id):
        """Kapanıp TEKRAR_SN temiz kalmadan dönen sorun aynı olayın devamıdır: kapanış silinir."""
        with self._islem() as db:
            db.execute("UPDATE sorun SET kapanis = '' WHERE id = ?", (sorun_id,))

    def kapanis_bekleyenler(self, musteri):
        """Açılış SMS'i gitmiş, kapanmış, kapanış SMS'i henüz karara bağlanmamış (kritik/uyarı) sorunlar."""
        return [dict(r) for r in self._hepsi(
            "SELECT * FROM sorun WHERE musteri = ? AND kapanis != '' AND kapanis_sms = '' AND onem != 'bilgi' "
            "AND acilis_sms IN ('gonderildi', 'mock') ORDER BY id", musteri)]

    def gunluk_bayrak(self, ad, gun):
        """Günde bir kez yapılacak iş: o gün ilk çağrıda True döner (bayrak konur), sonra False."""
        with self._islem() as db:
            if db.execute("SELECT 1 FROM ayar WHERE anahtar = ?", (f"{ad}:{gun}",)).fetchone():
                return False
            db.execute("INSERT INTO ayar (anahtar, deger) VALUES (?, '1')", (f"{ad}:{gun}",))
            return True

    def acik_sorunlar(self, musteri=None):
        if musteri is None:
            rows = self._hepsi("SELECT * FROM sorun WHERE kapanis = '' ORDER BY id")
        else:
            rows = self._hepsi("SELECT * FROM sorun WHERE kapanis = '' AND musteri = ? ORDER BY id", musteri)
        return [dict(r) for r in rows]

    def sorun_gecmisi(self, musteri, limit=50):
        return [dict(r) for r in self._hepsi("SELECT * FROM sorun WHERE musteri = ? ORDER BY id DESC LIMIT ?", musteri, limit)]

    def sorunlar_aralik(self, musteri, bas_iso, bit_iso):
        """[bas, bit) aralığıyla kesişen sorunlar (raporlar): açılışı bitişten önce, kapanışı yok ya da başlangıçtan sonra.
        Zamanlar hep +03:00 ISO: metin karşılaştırması sıralamayla aynı."""
        return [dict(r) for r in self._hepsi(
            "SELECT * FROM sorun WHERE musteri = ? AND acilis < ? AND (kapanis = '' OR kapanis >= ?) ORDER BY acilis, id",
            musteri, bit_iso, bas_iso)]

    def son_kapanan(self, musteri, tur):
        r = self._bir("SELECT * FROM sorun WHERE musteri = ? AND tur = ? AND kapanis != '' ORDER BY id DESC LIMIT 1",
                      musteri, tur)
        return dict(r) if r else None

    def uyari_ayari(self):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = 'uyari'")
        return {**UYARI_VARSAYILAN, **json.loads(r["deger"])} if r else dict(UYARI_VARSAYILAN)

    def uyari_ayari_koy(self, d):
        tel = []
        for ham in d.get("telefonlar") or []:
            n = common.normalize_phone(str(ham))
            if n is None:
                raise ValueError(f"Geçersiz telefon: {ham}")
            if "90" + n not in tel:
                tel.append("90" + n)
        if len(tel) > UYARI_TELEFON_EN_COK:
            raise ValueError(f"En çok {UYARI_TELEFON_EN_COK} telefon girilebilir.")
        ham = d.get("gunluk_sinir", UYARI_VARSAYILAN["gunluk_sinir"])
        try:
            if isinstance(ham, (bool, float)):
                raise ValueError
            sinir = int(ham)
        except (TypeError, ValueError):
            raise ValueError("Günlük SMS sınırı tamsayı olmalı.")
        if not UYARI_SINIR_ARALIK[0] <= sinir <= UYARI_SINIR_ARALIK[1]:  # son hak üst sınır bildiriminin: 1 olamaz
            raise ValueError(f"Günlük SMS sınırı {UYARI_SINIR_ARALIK[0]} ile {UYARI_SINIR_ARALIK[1]} arasında olmalı.")
        a = {"telefonlar": tel, "acik": bool(d.get("acik")), "gunluk_sinir": sinir}
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('uyari', ?)", (json.dumps(a),))

    def saglik_son_tur(self):
        """wc-saglik'in son tamamlanan turunun zamanı (ISO) ya da None (hiç çalışmadı)."""
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = 'saglik_son_tur'")
        return r["deger"] if r else None

    def saglik_son_tur_koy(self, zaman):
        with self._islem() as db:
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('saglik_son_tur', ?)", (str(zaman),))

    def sms_sayaci(self, gun):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = ?", "sms_sayac:" + gun)
        return int(r["deger"]) if r else 0

    def sms_say(self, gun):
        """Günlük uyarı SMS sayacını bir artırır, yeni değeri döner."""
        with self._islem() as db:
            r = db.execute("SELECT deger FROM ayar WHERE anahtar = ?", ("sms_sayac:" + gun,)).fetchone()
            n = (int(r["deger"]) if r else 0) + 1
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES (?, ?)", ("sms_sayac:" + gun, str(n)))
            return n

    def cihazlar(self):
        return {c["musteri"]: c for c in self._hepsi("SELECT * FROM cihaz WHERE durum != 'serbest'")}

    def cihaz_gecmisi(self, numara):
        return self._hepsi("SELECT * FROM cihaz WHERE musteri = ? ORDER BY id DESC", numara)

    # --- hareketler ---
    def hareket(self, kim, ip, olay, musteri=None, ayrinti=""):
        with self._islem() as db:
            db.execute("INSERT INTO hareket (zaman, kim, ip, olay, musteri, ayrinti) VALUES (?, ?, ?, ?, ?, ?)",
                       (self._simdi(), str(kim)[:64], str(ip)[:64], olay, musteri, str(ayrinti)[:500]))

    def hareketler(self, musteri=None, limit=300, olay=None):
        kosul, a = [], []
        if musteri is not None:
            kosul.append("musteri = ?")
            a.append(musteri)
        if olay is not None:
            kosul.append("olay = ?")
            a.append(olay)
        nerede = " WHERE " + " AND ".join(kosul) if kosul else ""
        return self._hepsi(f"SELECT * FROM hareket{nerede} ORDER BY id DESC LIMIT ?", *a, limit)

    # --- eski arşivler ---
    def eski_arsiv_bagla(self, ad, numara):
        with self._islem() as db:
            db.execute("INSERT INTO eski_arsiv (ad, musteri) VALUES (?, ?) "
                       "ON CONFLICT(ad) DO UPDATE SET musteri = excluded.musteri", (ad, numara))

    def eski_arsivler(self):
        return {r["ad"]: r["musteri"] for r in self._hepsi("SELECT * FROM eski_arsiv")}

    def arsivler(self, numara):
        """Müşterinin arşiv adları: kendi numarası önce (aynı gün iki yerdeyse o geçerli), sonra bağlı eski arşivler."""
        return [str(numara)] + [r["ad"] for r in self._hepsi("SELECT ad FROM eski_arsiv WHERE musteri = ? ORDER BY ad", numara)]

    def rapor_gruplari(self):
        """{"<numara>": [eski arşiv adları (sıralı)]}: yalnızca bağlı eski arşivi olan müşteriler."""
        out = {}
        for r in self._hepsi("SELECT ad, musteri FROM eski_arsiv WHERE musteri IS NOT NULL ORDER BY musteri, ad"):
            out.setdefault(str(r["musteri"]), []).append(r["ad"])
        return out

    def rapor_gruplari_yaz(self, yol=RAPOR_GRUPLARI):
        """Atomik, 600 (wcpanel; root okur). Merkez açılışında ve eski arşiv bağı değişince."""
        common.save_json(yol, self.rapor_gruplari())
        os.chmod(yol, 0o600)
