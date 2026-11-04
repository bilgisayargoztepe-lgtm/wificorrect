"""Geçici: Python merkezinin (veri.Veri) oluşturup göç ettirdiği küçük bir veritabanı üretir.
Rust testi (merkez/tests/veri/python-gocmus.db) bunu bir kopyadan açar. Kullanım:
    py -3 -I scripts/sunucu/parite/eski_vt.py merkez/tests/veri/python-gocmus.db
Önce ALTER ile gelen sütunlar OLMADAN eski şema kurulur, uydurma satırlar eklenir, sonra Python `veri.Veri` açılıp
ALTER göçleri uygulatılır (sütunlar tablonun sonuna eklenir), en sonda göçle gelen sütunlara örnek değerler yazılır."""
import json
import os
import sqlite3
import sys

AYR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(AYR, "..", "merkez"))
os.environ.setdefault("WC_TEST", "1")
import veri  # noqa: E402

ESKI_MUSTERI = """CREATE TABLE musteri (
  numara INTEGER PRIMARY KEY, tuz TEXT NOT NULL, ozet TEXT NOT NULL, yineleme INTEGER NOT NULL,
  not_ TEXT NOT NULL DEFAULT '', uyelik TEXT NOT NULL DEFAULT 'aktif', bitis TEXT NOT NULL DEFAULT '',
  olusturma TEXT NOT NULL, son_giris TEXT NOT NULL DEFAULT '')"""
ESKI_CIHAZ = """CREATE TABLE cihaz (
  id INTEGER PRIMARY KEY AUTOINCREMENT, musteri INTEGER NOT NULL REFERENCES musteri(numara),
  durum TEXT NOT NULL, tunel_ip TEXT NOT NULL DEFAULT '', wg_pub TEXT NOT NULL, ssh_pub TEXT NOT NULL,
  anahtar_ozet TEXT NOT NULL, isletme_adi TEXT NOT NULL DEFAULT '', unvan TEXT NOT NULL DEFAULT '',
  surum TEXT NOT NULL DEFAULT '', son_eslesme TEXT NOT NULL DEFAULT '', baglanma TEXT NOT NULL,
  ayrilma TEXT NOT NULL DEFAULT '')"""


def main(yol):
    if os.path.exists(yol):
        os.remove(yol)
    db = sqlite3.connect(yol)
    db.execute(ESKI_MUSTERI)
    db.execute(ESKI_CIHAZ)
    # geri kalan tablolar güncel şemadan (musteri/cihaz CREATE'leri IF NOT EXISTS: yukarıdakiler korunur)
    db.executescript(veri.SEMA)
    db.executemany("INSERT INTO musteri (numara, tuz, ozet, yineleme, not_, uyelik, bitis, olusturma, son_giris) VALUES (?,?,?,?,?,?,?,?,?)", [
        (1000001, "tuz1", "ozet1", 120000, "Deneme Kafe, Ornek", "aktif", "", "2026-01-15T10:00:00+03:00", "2026-11-01T09:00:00+03:00"),
        (1000002, "tuz2", "ozet2", 120000, "", "bitti", "2026-10-04T17:00:00+03:00", "2026-02-01T12:30:00+03:00", ""),
        (1000003, "tuz3", "ozet3", 120000, "Bos alanli", "aktif", "", "2026-03-10T08:00:00+03:00", "")])
    db.executemany("INSERT INTO cihaz (musteri, durum, tunel_ip, wg_pub, ssh_pub, anahtar_ozet, isletme_adi, unvan, surum, son_eslesme, baglanma, ayrilma) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", [
        (1000001, "bagli", "10.99.0.50", "A" * 43 + "=", "ssh-ed25519 AAAA deneme@cihaz", "ozetanahtar1", "Deneme Kafe", "Deneme Ltd.", "0.4.28", "2026-11-03T06:00:00+03:00", "2026-01-15T10:05:00+03:00", ""),
        (1000002, "serbest", "", "B" * 43 + "=", "ssh-ed25519 BBBB deneme@cihaz2", "ozetanahtar2", "", "", "", "", "2026-02-01T12:40:00+03:00", "2026-10-04T17:00:00+03:00"),
        (1000003, "bagli", "10.99.0.51", "C" * 43 + "=", "ssh-ed25519 CCCC deneme@cihaz3", "ozetanahtar3", "", "", "0.4.20", "", "2026-03-10T08:10:00+03:00", "")])
    db.executemany("INSERT INTO hareket (zaman, kim, ip, olay, musteri, ayrinti) VALUES (?,?,?,?,?,?)", [
        ("2026-01-15T10:00:00+03:00", "admin", "192.168.1.5", "MUSTERI_ACILDI", 1000001, ""),
        ("2026-10-04T17:00:00+03:00", "admin", "192.168.1.5", "UYELIK_BITTI", 1000002, "uyelik bitirildi"),
        ("2026-11-01T09:00:00+03:00", "1000001", "10.0.0.7", "GIRIS", 1000001, "Türkçe ayrıntı ğüşıöç"),
        ("2026-11-02T09:00:00+03:00", "admin", "192.168.1.5", "GIRIS", None, "")])
    db.executemany("INSERT INTO eski_arsiv (ad, musteri) VALUES (?,?)", [("bocafe-test", None), ("deneme-eski", 1000001)])
    ayarlar = [
        ("admin", json.dumps({"tuz": "atuz", "ozet": "aozet", "yineleme": 120000})),
        ("sms", json.dumps({"mock": True, "usercode": "", "password": "", "msgheader": "", "appkey": ""}, ensure_ascii=False)),
        ("uyari", json.dumps({"telefonlar": ["905550000001"], "acik": True, "gunluk_sinir": 20})),
        ("kanal_kararli", "0.4.28"),  # JSON değil, düz metin
        ("kanal_test", "en-son"),
        ("saglik_son_tur", "2026-11-03T20:00:00+03:00"),
        ("sms_sayac:2026-11-03", "7"),
        ("gunluk_rapor:2026-11-03", "1")]
    db.executemany("INSERT INTO ayar (anahtar, deger) VALUES (?,?)", ayarlar)
    db.execute("INSERT INTO saglik (cihaz, zaman, rapor) VALUES (1, '2026-11-03T20:00:00+03:00', ?)",
               (json.dumps({"servisler": {"panel": True}, "disk": 41, "not": "ğ"}, ensure_ascii=False),))
    db.executemany("INSERT INTO sorun (musteri, tur, metin, onem, acilis, kapanis, acilis_sms, kapanis_sms) VALUES (?,?,?,?,?,?,?,?)", [
        (1000001, "disk", "Disk %91 dolu", "uyari", "2026-10-30T10:00:00+03:00", "2026-10-30T12:00:00+03:00", "gonderildi", ""),
        (1000003, "rapor", "Rapor gelmiyor", "kritik", "2026-11-03T08:00:00+03:00", "", "mock", "")])
    db.executemany("INSERT INTO surum (surum, sha256, boyut, kaynak, zaman) VALUES (?,?,?,?,?)", [
        ("0.4.9", "a" * 64, 1000, "github", "2026-10-14T10:00:00+03:00"),
        ("0.4.28", "b" * 64, 2000000, "github", "2026-10-31T10:00:00+03:00"),
        ("0.10.0", "c" * 64, 3000, "yerel", "2026-11-01T10:00:00+03:00")])
    db.execute("INSERT INTO sms_sayac (musteri, gun, sayi) VALUES (1000001, '2026-11-03', 4)")
    db.execute("INSERT INTO sms_sayac (musteri, gun, sayi) VALUES (1000001, '2026-11-03/yabanci', 1)")
    db.execute("INSERT INTO sms_son (musteri, zaman, saglayici, sonuc, kod) VALUES (1000001, '2026-11-03T12:00:00+03:00', 'netgsm', 'gonderildi', '0')")
    db.executemany("INSERT INTO bekleyen (kimlik, sir_ozet, ilk, son, ip, surum, wan_mac, wg_pub, ssh_pub, musteri, ayrilma, belirtec_ozet, belirtec_bitis, cihaz, baglandi) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", [
        ("k" * 32, "s" * 64, 1790000000.0, 1790000600.0, "203.0.113.9", "0.4.28", "aa:bb:cc:dd:ee:01", "D" * 43 + "=", "ssh-ed25519 DDDD d@d", None, 0, "", 0, None, 0),
        ("m" * 32, "t" * 64, 1790000100.0, 1790000700.0, "203.0.113.9", "0.4.28", "aa:bb:cc:dd:ee:02", "E" * 43 + "=", "ssh-ed25519 EEEE e@e", 1000003, 1790000650.0, "u" * 64, 1790001250.0, 3, 1790000660.5)])
    db.commit()
    db.close()
    # Python göçü: ALTER sütunları sona eklenir, lisans_bitis/ap_parola doldurulur
    v = veri.Veri(yol, saat=lambda: 1_790_000_000.0)
    v.db.execute("UPDATE musteri SET parola_acik = 'Deneme-parola-1', guncelleme_kanali = 'test', askida = 0 WHERE numara = 1000001")
    v.db.execute("UPDATE musteri SET askida = 1, guncelleme_kanali = 'kapali' WHERE numara = 1000002")
    v.db.execute("UPDATE musteri SET guncelleme_hemen = 1790000000.0, guncelleme_hemen_surum = '0.4.28' WHERE numara = 1000003")
    v.db.execute("UPDATE cihaz SET isletme_turu = 'otel', adres = 'Deneme Mah. 1', otomatik = 1, admin_yineleme = 0 WHERE musteri = 1000001")
    v.db.close()


if __name__ == "__main__":
    main(sys.argv[1])
