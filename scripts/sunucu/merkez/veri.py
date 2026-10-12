"""Yönetim merkezi veritabanı (spec §3): müşteriler, cihazlar, hareketler, eski arşiv bağları.
SQLite; tek bağlantı + kilit (ThreadingHTTPServer iş parçacıkları için). Yalnızca panel (wcpanel) açar, root açmaz."""
import contextlib
import datetime
import hashlib
import json
import secrets
import sqlite3
import threading
import time

import common
import guvenlik

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
  ayrilma TEXT NOT NULL DEFAULT '', isletme_turu TEXT NOT NULL DEFAULT '');
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
"""
CIHAZ_ALANLARI = {"tunel_ip", "isletme_adi", "unvan", "surum", "son_eslesme", "isletme_turu"}
# Cihazın eşitlemede bildirdiği işletme türü (cihaz ayarı main.isletme_turu); başka değer saklanmaz
ISLETME_TURLERI = {"kafe": "Kafe/Restoran", "otel": "Otel/Pansiyon", "yurt": "Yurt", "diger": "Diğer"}
# Sonradan eklenen müşteri sütunları (eski veritabanına ALTER ile eklenir)
YENI_SUTUNLAR = (("parola_acik", "TEXT NOT NULL DEFAULT ''"), ("lisans_bitis", "TEXT NOT NULL DEFAULT ''"),
                 ("askida", "INTEGER NOT NULL DEFAULT 0"), ("guncelleme_kanali", "TEXT NOT NULL DEFAULT 'kararli'"),
                 ("guncelleme_hemen", "REAL NOT NULL DEFAULT 0"), ("guncelleme_hemen_surum", "TEXT NOT NULL DEFAULT ''"))
GUNCELLEME_KANALLARI = ("kararli", "test", "kapali")
YENI_CIHAZ_SUTUNLARI = (("admin_parola", "TEXT NOT NULL DEFAULT ''"), ("admin_tuz", "TEXT NOT NULL DEFAULT ''"),
                        ("admin_ozet", "TEXT NOT NULL DEFAULT ''"), ("admin_yineleme", "INTEGER NOT NULL DEFAULT 0"),
                        ("isletme_turu", "TEXT NOT NULL DEFAULT ''"))
UYARI_VARSAYILAN = {"telefonlar": [], "acik": False, "gunluk_sinir": 20}
UYARI_TELEFON_EN_COK = 3
UYARI_SINIR_ARALIK = (2, 100)  # günlük uyarı SMS sınırı (yönetim formu da bunu kullanır)
SORUN_SMS_ALANLARI = ("acilis_sms", "kapanis_sms")
SMS_ALANLARI = ("usercode", "password", "msgheader", "appkey")
LISANS_GUN = 365


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

    def eslesme_kaydet(self, cid, isletme_adi, unvan, surum, isletme_turu=None):
        """isletme_turu yalnızca izinli değerse yazılır; yoksa ya da bozuksa eskisi kalır."""
        ek = {"isletme_turu": isletme_turu} if isinstance(isletme_turu, str) and isletme_turu in ISLETME_TURLERI else {}
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

    # --- bütün cihazlara giden SMS (NetGSM) ayarı ---
    def sms_ayari(self):
        r = self._bir("SELECT deger FROM ayar WHERE anahtar = 'sms'")
        return json.loads(r["deger"]) if r else None

    def sms_ayari_koy(self, d):
        a = {"mock": bool(d.get("mock"))}
        for k in SMS_ALANLARI:
            a[k] = str(d.get(k) or "").strip()
        eksik = [k for k in ("usercode", "password", "msgheader") if not a[k]]
        if not a["mock"] and eksik:
            raise ValueError(f"Deneme modu kapalıyken şu alanlar gerekli: {', '.join(eksik)}")
        with self._islem() as db:
            # ponytail: NetGSM şifresi açık saklanır (cihaza gönderilmesi gerekir); veritabanı yalnızca wcpanel'in (600)
            db.execute("INSERT OR REPLACE INTO ayar (anahtar, deger) VALUES ('sms', ?)", (json.dumps(a, ensure_ascii=False),))

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

    def hareketler(self, musteri=None, limit=300):
        if musteri is None:
            return self._hepsi("SELECT * FROM hareket ORDER BY id DESC LIMIT ?", limit)
        return self._hepsi("SELECT * FROM hareket WHERE musteri = ? ORDER BY id DESC LIMIT ?", musteri, limit)

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
