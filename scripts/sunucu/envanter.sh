#!/bin/bash
# Merkez envanteri (Rust'a taşıma, plan Task 0). YALNIZCA OKUR; yalnızca yapı ve sayı basar: hiçbir değer, parola,
# telefon ya da ad basmaz. Kullanım: sudo bash envanter.sh
set -u
VT=/var/lib/wificorrect/merkez/merkez.db
echo "== surum: $(date -Iseconds) $(hostname)"
python3 -I - "$VT" <<'PY'
import json, sqlite3, sys
db = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True)
tablolar = [r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")]
for t in tablolar:
    sutunlar = [r[1] for r in db.execute(f"PRAGMA table_info({t})")]
    n = db.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0]
    print(f"TABLO {t} satir={n} sutunlar={','.join(sutunlar)}")
def say(sorgu):
    try:
        return db.execute(sorgu).fetchone()[0]
    except sqlite3.Error as h:
        return f"hata:{h}"
print("musteri bos_lisans_bitis", say("SELECT COUNT(*) FROM musteri WHERE lisans_bitis = ''"))
print("musteri bos_ap_parola", say("SELECT COUNT(*) FROM musteri WHERE ap_parola = ''"))
print("musteri bos_parola_acik", say("SELECT COUNT(*) FROM musteri WHERE parola_acik = ''"))
print("musteri uyelik", db.execute("SELECT uyelik, COUNT(*) FROM musteri GROUP BY uyelik").fetchall())
print("cihaz durum", db.execute("SELECT durum, COUNT(*) FROM cihaz GROUP BY durum").fetchall())
print("cihaz otomatik", say("SELECT COUNT(*) FROM cihaz WHERE otomatik = 1"))
print("cihaz surum", db.execute("SELECT surum, COUNT(*) FROM cihaz WHERE durum != 'serbest' GROUP BY surum").fetchall())
print("ayar anahtarlari", [r[0] for r in db.execute("SELECT anahtar FROM ayar ORDER BY anahtar")])
bozuk = 0
for (rapor,) in db.execute("SELECT rapor FROM saglik"):
    try:
        json.loads(rapor)
    except ValueError:
        bozuk += 1
print("saglik json_bozuk", bozuk)
bozuk = 0
for (d,) in db.execute("SELECT deger FROM ayar"):
    try:
        json.loads(d)
    except ValueError:
        bozuk += 1
print("ayar json_olmayan", bozuk)
print("sema_surumu", db.execute("PRAGMA user_version").fetchone()[0], "journal", db.execute("PRAGMA journal_mode").fetchone()[0])
PY
python3 -I - <<'PY'
import collections, glob, json, os
def bicimler(desen):
    c, bozuk = collections.Counter(), 0
    for y in glob.glob(desen):
        try:
            with open(y, encoding="utf-8") as f:
                v = json.load(f)
            c[v.get("bicim") if isinstance(v, dict) else "liste"] += 1
        except (OSError, ValueError):
            bozuk += 1
    return dict(c), bozuk
for ad, desen in (("rapor", "/var/lib/wificorrect/rapor/*/*.json"), ("rapor-ham", "/var/lib/wificorrect/rapor-ham/*/*.json")):
    c, b = bicimler(desen)
    print(f"BICIM {ad} {c} bozuk={b}")
for ad, desen in (("durum", "/var/lib/wificorrect/durum.json"), ("istatistik", "/var/lib/wificorrect/istatistik/*.json"),
                  ("detay", "/var/lib/wificorrect/detay/*/*.json"), ("kuyruk", "/var/lib/wificorrect/kuyruk/*"),
                  ("kuyruk-sonuc", "/var/lib/wificorrect/kuyruk-sonuc/*"), ("merkez-json", "/var/lib/wificorrect/merkez/*.json"),
                  ("listeler", "/var/lib/wificorrect/merkez/listeler/*"), ("surumler", "/var/lib/wificorrect/merkez/surumler/*")):
    ys = glob.glob(desen)
    bozuk = 0
    for y in ys:
        if y.endswith(".json"):
            try:
                with open(y, encoding="utf-8") as f:
                    json.load(f)
            except (OSError, ValueError):
                bozuk += 1
    print(f"DOSYA {ad} sayi={len(ys)} json_bozuk={bozuk}")
print("merkez-json adlari", sorted(os.path.basename(y) for y in glob.glob("/var/lib/wificorrect/merkez/*.json")))
for kok in ("/srv/hotspot-arsiv", "/srv/wificorrect-arsiv"):
    for d in sorted(glob.glob(kok + "/*")):
        gunler = sorted(os.path.basename(g) for g in glob.glob(d + "/gunluk/*") + glob.glob(d + "/veri/gunluk/*"))
        print(f"ARSIV {d} gun={len(gunler)} ilk={gunler[0] if gunler else '-'} son={gunler[-1] if gunler else '-'}")
PY
echo "== birimler"
systemctl is-active wc-merkez wc-kuyruk.path wc-durum.timer wc-istatistik.timer wc-saglik.timer wc-listeler.timer wc-surumler.timer
ls -la /usr/local/sbin/wc-* 2>/dev/null | awk '{print $9, $10, $11}'
echo "== bitti"
