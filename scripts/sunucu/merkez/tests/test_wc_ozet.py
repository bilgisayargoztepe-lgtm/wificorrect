import gzip
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import ozet as O  # noqa: E402

BASLIK = ("zaman;olay;telefon;ad;soyad;mac;ic_ip;protokol;ic_port;hedef_ip;hedef_port;nat_ip;nat_port;"
          "alan_adi;gonderilen_bayt;alinan_bayt;sure_sn;oturum_id;ek")
KOLON = BASLIK.split(";")
A, B, C = "5330000001", "5330000002", "5330000003"


def gz_yaz(yol, satirlar):
    os.makedirs(os.path.dirname(yol), exist_ok=True)
    with gzip.open(yol, "wt", encoding="utf-8", newline="") as f:
        f.write("﻿" + BASLIK + "\n")
        for s in satirlar:
            f.write(";".join(str(s.get(k, "")) for k in KOLON) + "\n")


def manifest(d, dosyalar):
    with open(os.path.join(d, "MANIFEST.sha256"), "w", encoding="utf-8") as f:
        f.write("# gun\n" + "".join(f"abc  {p}\n" for p in dosyalar))


def z(gun, saat):
    return f"{gun}T{saat}+03:00"


def ornek_gun(kok, gun, bozuk=False):
    """A: o gün oturum açtı (ad oturumda). B: oturumu önceki günden (ad KAYIT'ta). C: yalnızca index.csv'de."""
    d = os.path.join(kok, "gunluk", gun)
    gz_yaz(os.path.join(d, "oturum.csv.gz"), [
        {"zaman": z(gun, "09:00:00"), "olay": "OTURUM_BASLA", "telefon": A, "ad": "Ayşe", "soyad": "Yılmaz",
         "mac": "aa:aa:aa:aa:aa:01", "ic_ip": "10.50.0.21"},
        {"zaman": z(gun, "17:00:00"), "olay": "OTURUM_BITIS", "telefon": A, "ad": "Ayşe", "soyad": "Yılmaz",
         "mac": "aa:aa:aa:aa:aa:01", "ic_ip": "10.50.0.21", "ek": "neden=cikis"}])
    gz_yaz(os.path.join(d, "dns.csv.gz"), [
        {"zaman": z(gun, "09:05:00"), "olay": "DNS", "telefon": A, "mac": "aa:aa:aa:aa:aa:01", "alan_adi": "Instagram.com."},
        {"zaman": z(gun, "09:10:00"), "olay": "DNS", "telefon": A, "alan_adi": "instagram.com"},
        {"zaman": z(gun, "09:07:00"), "olay": "DNS", "telefon": A, "alan_adi": "google.com"},
        {"zaman": z(gun, "10:00:00"), "olay": "DNS", "telefon": B, "mac": "bb:bb:bb:bb:bb:02", "alan_adi": "<script>x</script>.com"},
        {"zaman": z(gun, "11:00:00"), "olay": "DNS", "telefon": C, "alan_adi": "a.com"},
        {"zaman": z(gun, "11:30:00"), "olay": "DNS", "telefon": "", "alan_adi": "izinli.com"}])
    trafik = [
        {"zaman": z(gun, "09:06:00"), "olay": "BAGLANTI_BITIS", "telefon": A, "protokol": "tcp", "hedef_ip": "1.1.1.1",
         "hedef_port": "443", "gonderilen_bayt": "100", "alinan_bayt": "900", "sure_sn": "60"},
        {"zaman": z(gun, "09:08:00"), "olay": "BAGLANTI_BITIS", "telefon": A, "protokol": "udp", "hedef_ip": "8.8.8.8",
         "hedef_port": "53", "gonderilen_bayt": "10", "alinan_bayt": "10", "sure_sn": "1"},
        {"zaman": z(gun, "09:05:30"), "olay": "BAGLANTI_BASLA", "telefon": A, "hedef_ip": "1.1.1.1"},
        {"zaman": z(gun, "10:01:00"), "olay": "BAGLANTI_BITIS", "telefon": B, "gonderilen_bayt": "", "alinan_bayt": "abc"},
        {"zaman": z(gun, "11:01:00"), "olay": "BAGLANTI_BITIS", "telefon": C, "gonderilen_bayt": "5", "alinan_bayt": "5"},
        {"zaman": z(gun, "11:31:00"), "olay": "BAGLANTI_BITIS", "telefon": "", "gonderilen_bayt": "9", "alinan_bayt": "9"}]
    if bozuk:
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "trafik.csv.gz"), "wb") as f:
            f.write(b"\x1f\x8b\x08\x00bozuk")
    else:
        gz_yaz(os.path.join(d, "trafik.csv.gz"), trafik)
    gz_yaz(os.path.join(d, "kullanicilar", B + ".csv.gz"), [
        {"zaman": z(gun, "10:00:00"), "olay": "KAYIT", "telefon": B, "ad": "Bora", "soyad": "Kaya", "mac": "bb:bb:bb:bb:bb:02"}])
    manifest(d, ["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz", f"kullanicilar/{B}.csv.gz"])
    with open(os.path.join(kok, "index.csv"), "w", encoding="utf-8") as f:
        f.write(f"﻿telefon;ad;soyad;ilk_kayit;son_oturum\n{C};Cem;Er;;\n")
    return d


def oku(yol):
    with open(yol, encoding="utf-8") as f:
        return json.load(f)


def test_gun_ozeti_kisiler_ve_toplamlar():
    with tempfile.TemporaryDirectory() as tmp:
        gun = "2026-09-27"
        d = ornek_gun(tmp, gun)
        kisiler = O.gun_ozeti(d, O.index_oku(tmp))
        assert [k["telefon"] for k in kisiler] == [A, B, C]  # Ayşe, Bora, Cem; telefonsuz satır yok
        a, b, c = kisiler
        assert (a["ad"], a["soyad"], b["ad"], b["soyad"], c["ad"], c["soyad"]) == ("Ayşe", "Yılmaz", "Bora", "Kaya", "Cem", "Er")
        assert (a["gonderilen"], a["alinan"], a["baglanti_sayisi"]) == (110, 910, 2)
        assert (b["gonderilen"], b["alinan"], b["baglanti_sayisi"]) == (0, 0, 1)
        assert a["siteler"][0] == {"alan": "instagram.com", "sayi": 2, "ilk": z(gun, "09:05:00"), "son": z(gun, "09:10:00")}
        assert [s["alan"] for s in a["siteler"]] == ["instagram.com", "google.com"] and a["diger_site_sayisi"] == 0
        assert [x["gonderilen"] + x["alinan"] for x in a["baglantilar"]] == [1000, 20]
        assert a["baglantilar"][0] == {"zaman": z(gun, "09:06:00"), "protokol": "tcp", "hedef_ip": "1.1.1.1",
                                       "hedef_port": "443", "gonderilen": 100, "alinan": 900, "sure_sn": 60}
        assert [o["olay"] for o in a["oturumlar"]] == ["OTURUM_BASLA", "OTURUM_BITIS"]
        assert (a["ilk"], a["son"]) == (z(gun, "09:00:00"), z(gun, "17:00:00"))
        assert a["mac"] == ["aa:aa:aa:aa:aa:01"] and b["mac"] == ["bb:bb:bb:bb:bb:02"]
        assert b["siteler"][0]["alan"] == "<script>x</script>.com" and b["oturumlar"] == []


def test_site_ve_baglanti_sinirlari():
    with tempfile.TemporaryDirectory() as tmp:
        gun = "2026-09-27"
        d = os.path.join(tmp, "gunluk", gun)
        gz_yaz(os.path.join(d, "dns.csv.gz"),
               [{"zaman": z(gun, "10:00:00"), "olay": "DNS", "telefon": A, "alan_adi": f"s{i:03d}.com"} for i in range(105)])
        gz_yaz(os.path.join(d, "trafik.csv.gz"),
               [{"zaman": z(gun, "10:00:00"), "olay": "BAGLANTI_BITIS", "telefon": A,
                 "gonderilen_bayt": str(i), "alinan_bayt": "0"} for i in range(600)])
        (k,) = O.gun_ozeti(d, {})
        assert len(k["siteler"]) == 100 and k["diger_site_sayisi"] == 5
        assert [x["gonderilen"] for x in k["baglantilar"]] == list(range(599, 549, -1))
        assert k["baglanti_sayisi"] == 600 and k["gonderilen"] == sum(range(600))


def test_detay_guncelle_artimli_ve_dizin():
    with tempfile.TemporaryDirectory() as tmp:
        kafe, dd = os.path.join(tmp, "arsiv", "bocafe"), os.path.join(tmp, "detay", "bocafe")
        ornek_gun(kafe, "2026-09-26")
        ornek_gun(kafe, "2026-09-27")
        yarim = os.path.join(kafe, "gunluk", "2026-09-28")
        os.makedirs(yarim)
        manifest(yarim, ["trafik.csv.gz"])  # rsync sürüyor: trafik henüz yok
        assert O.detay_guncelle(kafe, dd) == []
        assert sorted(os.listdir(dd)) == ["2026-09-26.json", "2026-09-27.json", "kisiler.json"]
        dizin = oku(os.path.join(dd, "kisiler.json"))
        assert dizin[A] == {"ad": "Ayşe", "soyad": "Yılmaz", "gunler": ["2026-09-27", "2026-09-26"], "odalar": {}}
        with open(os.path.join(dd, "2026-09-26.json"), "w", encoding="utf-8") as f:
            json.dump({"gun": "2026-09-26", "kisiler": []}, f)  # özetlenmiş gün yeniden üretilmez
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "2026-09-26.json"))["kisiler"] == []
        os.remove(os.path.join(dd, "2026-09-27.json"))  # saklama sildi → dizinden de düşer (arşivde hâlâ var → yeniden üretilir)
        os.remove(os.path.join(kafe, "gunluk", "2026-09-27", "MANIFEST.sha256"))
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "kisiler.json"))[A]["gunler"] == ["2026-09-26"]  # dosyası olmayan 27 düştü
        with open(os.path.join(dd, "kisiler.json"), "w", encoding="utf-8") as f:
            f.write("{bozuk")
        manifest(os.path.join(kafe, "gunluk", "2026-09-27"), ["oturum.csv.gz", "dns.csv.gz", "trafik.csv.gz"])
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "kisiler.json"))[A]["gunler"] == ["2026-09-27"]


def test_detay_bozuk_gun_atlanir_sonra_denenir():
    with tempfile.TemporaryDirectory() as tmp:
        kafe, dd = os.path.join(tmp, "bocafe"), os.path.join(tmp, "detay")
        ornek_gun(kafe, "2026-09-26", bozuk=True)
        ornek_gun(kafe, "2026-09-27")
        hatalar = O.detay_guncelle(kafe, dd)
        assert len(hatalar) == 1 and "2026-09-26" in hatalar[0]
        assert sorted(os.listdir(dd)) == ["2026-09-27.json", "kisiler.json"]


def test_telefon_yol_disina_cikamaz():
    with tempfile.TemporaryDirectory() as tmp:
        d = os.path.join(tmp, "gunluk", "2026-09-27")
        gz_yaz(os.path.join(d, "dns.csv.gz"), [{"zaman": "x", "olay": "DNS", "telefon": "../../../index", "alan_adi": "a.com"}])
        (k,) = O.gun_ozeti(d, {})
        assert k["telefon"] == "../../../index" and k["ad"] == ""


def test_dizine_girmemis_ozet_gunu_eklenir():
    with tempfile.TemporaryDirectory() as tmp:
        kafe, dd = os.path.join(tmp, "arsiv", "bocafe"), os.path.join(tmp, "detay", "bocafe")
        ornek_gun(kafe, "2026-09-26")
        O.detay_guncelle(kafe, dd)
        d27 = ornek_gun(kafe, "2026-09-27")
        with open(os.path.join(dd, "2026-09-27.json"), "w", encoding="utf-8") as f:  # görev özeti yazıp dizinden önce öldü
            json.dump({"gun": "2026-09-27", "kisiler": O.gun_ozeti(d27, {})}, f)
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "kisiler.json"))[A]["gunler"] == ["2026-09-27", "2026-09-26"]

def test_oda_dizini_ve_eski_bicim_yeniden_kurulur():
    with tempfile.TemporaryDirectory() as tmp:
        kafe, dd = os.path.join(tmp, "arsiv", "bocafe"), os.path.join(tmp, "detay", "bocafe")
        d = ornek_gun(kafe, "2026-09-26")
        gz_yaz(os.path.join(d, "oturum.csv.gz"), [
            {"zaman": z("2026-09-26", "09:00:00"), "olay": "OTURUM_BASLA", "telefon": "TC:10000000146", "ad": "Deniz",
             "ek": "sozlesme=1 riza=1 giris=misafir_kodu oda=a-12 kod_bitis=x ureten=mudur"}])
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "kisiler.json"))["TC:10000000146"]["odalar"] == {"A-12": ["2026-09-26"]}
        with open(os.path.join(dd, "kisiler.json"), "w", encoding="utf-8") as f:  # oda alanından önceki biçim
            json.dump({"TC:10000000146": {"ad": "Deniz", "soyad": "", "gunler": ["2026-09-26"]}}, f)
        O.detay_guncelle(kafe, dd)
        assert oku(os.path.join(dd, "kisiler.json"))["TC:10000000146"]["odalar"] == {"A-12": ["2026-09-26"]}


def test_kimlikli_kisi_dosyasindan_ad():
    with tempfile.TemporaryDirectory() as tmp:
        d = os.path.join(tmp, "gunluk", "2026-09-27")
        gz_yaz(os.path.join(d, "dns.csv.gz"), [{"zaman": "x", "olay": "DNS", "telefon": "PASAPORT:DE:X1234567", "alan_adi": "a.com"}])
        gz_yaz(os.path.join(d, "kullanicilar", "PASAPORT_DE_X1234567.csv.gz"), [
            {"zaman": "x", "olay": "KAYIT", "telefon": "PASAPORT:DE:X1234567", "ad": "Hans", "soyad": "Muller"}])
        (k,) = O.gun_ozeti(d, {})
        assert (k["ad"], k["soyad"]) == ("Hans", "Muller")


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
