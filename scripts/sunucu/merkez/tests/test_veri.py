import json
import os
import sqlite3
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import guvenlik as G  # noqa: E402
import veri as V  # noqa: E402

WG1, WG2 = "A" * 43 + "=", "B" * 43 + "="
SSH = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI test@cihaz"


def yeni(tmp):
    return V.Veri(os.path.join(tmp, "m.db"), saat=lambda: 1_790_000_000.0)


def test_musteri_numara_sirasi_ve_parola():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n1, p1 = v.musteri_ekle("Bocafe, Göztepe")
        numaralar = [n1] + [v.musteri_ekle()[0] for _ in range(30)]
        assert all(1_000_000 <= n <= 9_999_999 for n in numaralar) and len(set(numaralar)) == 31 and len(p1) == 12
        assert sorted(numaralar) != numaralar or numaralar != list(range(n1, n1 + 31))  # sıralı değil, rastgele
        assert v.parola_dogrula(n1, p1)["not_"] == "Bocafe, Göztepe"
        assert v.parola_dogrula(n1, "yanlis") is None and v.parola_dogrula(1, p1) is None
        assert v.parola_dogrula(str(n1), p1) is None  # numara int olmalı
        try:
            v.parola_koy(n1, "kisa")
            assert False
        except ValueError:
            pass
        v.parola_koy(n1, "yeni-parola-1")
        assert v.parola_dogrula(n1, "yeni-parola-1")
        p = v.parola_sifirla(n1)
        assert v.parola_dogrula(n1, p) and not v.parola_dogrula(n1, "yeni-parola-1")


def test_cihaz_bagla_tek_cihaz_ve_yeniden_deneme():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n, _ = v.musteri_ekle()
        cid, a1 = v.cihaz_bagla(n, WG1, SSH, "Bocafe", "Bocafe Ltd.", "1.0")
        assert v.cihaz_anahtarla(a1)["id"] == cid
        cid2, a2 = v.cihaz_bagla(n, WG1, SSH)  # aynı cihaz yeniden denedi
        assert cid2 == cid and v.cihaz_anahtarla(a1) is None and v.cihaz_anahtarla(a2)["id"] == cid
        try:
            v.cihaz_bagla(n, WG2, SSH)  # başka cihaz
            assert False
        except V.BaskaCihaz:
            pass
        m2, _ = v.musteri_ekle()
        try:
            v.cihaz_bagla(m2, WG1, SSH)  # bu cihaz başka müşteriye bağlı
            assert False
        except V.BaskaCihaz:
            pass


def test_serbest_birakma_akisi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n, _ = v.musteri_ekle()
        cid, a = v.cihaz_bagla(n, WG1, SSH)
        assert v.serbest_birak(n)["id"] == cid
        assert v.bagli_cihaz(n)["durum"] == "serbest_birakiliyor" and v.cihaz_anahtarla(a) is not None
        try:
            v.cihaz_bagla(n, WG2, SSH)  # eski cihaz temizlenmeden yenisi bağlanmaz
            assert False
        except V.BaskaCihaz:
            pass
        v.temizlendi(cid)
        assert v.bagli_cihaz(n) is None and v.cihaz_anahtarla(a) is None
        cid3, _ = v.cihaz_bagla(n, WG2, SSH)
        assert v.serbest_birak(n, zorla=True)["id"] == cid3 and v.bagli_cihaz(n) is None
        assert [c["durum"] for c in v.cihaz_gecmisi(n)] == ["serbest", "serbest"]
        assert v.serbest_birak(n) is None


def test_uyelik_hareket_eski_arsiv():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n, _ = v.musteri_ekle()
        v.uyelik(n, "bitti")
        assert v.musteri(n)["uyelik"] == "bitti" and v.musteri(n)["bitis"]
        v.uyelik(n, "aktif")
        assert v.musteri(n)["bitis"] == ""
        v.hareket("admin", "1.2.3.4", "MUSTERI_ACILDI", n, "x" * 1000)
        v.hareket("admin", "1.2.3.4", "GIRIS")
        assert [h["olay"] for h in v.hareketler()] == ["GIRIS", "MUSTERI_ACILDI"]
        assert len(v.hareketler(n)) == 1 and len(v.hareketler(n)[0]["ayrinti"]) == 500
        v.eski_arsiv_bagla("bocafe", n)
        v.eski_arsiv_bagla("bocafe-test", None)
        assert v.eski_arsivler() == {"bocafe": n, "bocafe-test": None}
        assert v.arsivler(n) == [str(n), "bocafe"]


def test_rapor_gruplari_yaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n1, _ = v.musteri_ekle()
        n2, _ = v.musteri_ekle()
        yol = os.path.join(tmp, "rapor-gruplari.json")
        v.rapor_gruplari_yaz(yol)
        with open(yol, encoding="utf-8") as f:
            assert json.load(f) == {}
        v.eski_arsiv_bagla("zkafe", n1)
        v.eski_arsiv_bagla("akafe", n1)
        v.eski_arsiv_bagla("bocafe", n2)
        v.eski_arsiv_bagla("bagsiz", None)
        v.rapor_gruplari_yaz(yol)
        with open(yol, encoding="utf-8") as f:
            assert json.load(f) == {str(n1): ["akafe", "zkafe"], str(n2): ["bocafe"]}
        if os.name == "posix":
            assert os.stat(yol).st_mode & 0o777 == 0o600


def test_kalici():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n, p = v.musteri_ekle()
        v.db.close()
        assert yeni(tmp).parola_dogrula(n, p)


def test_parola_acik_ve_lisans():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)  # saat: 2026-09-21
        n, p = v.musteri_ekle()
        m = v.musteri(n)
        assert m["parola_acik"] == p and m["lisans_bitis"] == "2027-09-21" and m["askida"] == 0
        v.parola_koy(n, "yeni-parola-1")
        assert v.musteri(n)["parola_acik"] == "yeni-parola-1"
        p2 = v.parola_sifirla(n)
        assert v.musteri(n)["parola_acik"] == p2
        assert v.lisans_durumu(v.musteri(n)) == ("aktif", "2027-09-21")
        assert v.lisans_uzat(n) == "2028-09-20"  # bitişin üstüne 1 yıl
        v.lisans_ayarla(n, "2026-09-01")
        assert v.lisans_durumu(v.musteri(n)) == ("bitti", "2026-09-01")
        assert v.lisans_uzat(n) == "2027-09-21"  # süresi geçmişse bugünden 1 yıl
        v.askiya_al(n, True)
        assert v.lisans_durumu(v.musteri(n))[0] == "askida"
        v.askiya_al(n, False)
        assert v.lisans_durumu(v.musteri(n))[0] == "aktif"
        try:
            v.lisans_ayarla(n, "bozuk")
            assert False
        except ValueError:
            pass


def test_eski_veritabani_gocu():
    import sqlite3
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        yol = os.path.join(tmp, "m.db")
        db = sqlite3.connect(yol)
        db.execute("CREATE TABLE musteri (numara INTEGER PRIMARY KEY, tuz TEXT NOT NULL, ozet TEXT NOT NULL, yineleme INTEGER NOT NULL, "
                   "not_ TEXT NOT NULL DEFAULT '', uyelik TEXT NOT NULL DEFAULT 'aktif', bitis TEXT NOT NULL DEFAULT '', "
                   "olusturma TEXT NOT NULL, son_giris TEXT NOT NULL DEFAULT '')")
        db.execute("INSERT INTO musteri (numara, tuz, ozet, yineleme, olusturma) VALUES (4511643, 't', 'o', 120000, '2026-10-04T17:00:00+03:00')")
        db.commit()
        db.close()
        v = V.Veri(yol, saat=lambda: 1_790_000_000.0)
        m = v.musteri(4511643)
        assert (m["lisans_bitis"], m["parola_acik"], m["askida"]) == ("2027-10-04", "", 0)
        ap = m["ap_parola"]  # eski müşteriye göçte AP yönetim parolası üretilir
        assert len(ap) == V.AP_PAROLA_UZUNLUK and set(ap) <= set(G.HARFLER)
        v.db.close()
        assert V.Veri(yol, saat=lambda: 1_790_000_000.0).musteri(4511643)["ap_parola"] == ap  # her açılışta yeniden üretilmez


def test_ap_yonetim_parolasi():
    """Müşteri başına rastgele AP yönetim parolası (final review I1): panel parolasından ayrı (bütün AP'ler tek kökte açılır,
    tarayıcının sakladığı AP kimliği başka müşterinin AP'sine gidebilir), 12 karakter, karışmayan harfler; açık saklanır
    (yönetimde ve cihaz panelinde gösterilir) ve yenilenebilir."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        n, pw = v.musteri_ekle(parola="musteri-parola-1")
        ap = v.musteri(n)["ap_parola"]
        assert len(ap) == 12 and set(ap) <= set(G.HARFLER) and ap != pw
        n2, _ = v.musteri_ekle()
        assert v.musteri(n2)["ap_parola"] not in ("", ap)
        yeni_ap = v.ap_parolasi_yenile(n)
        assert yeni_ap != ap and len(yeni_ap) == 12 and v.musteri(n)["ap_parola"] == yeni_ap
        v.parola_koy(n, "baska-parola-12")
        assert v.musteri(n)["ap_parola"] == yeni_ap  # müşteri parolası değişince AP parolası değişmez


def test_silinen_veri_sayfalarda_kalmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.db.execute("PRAGMA secure_delete").fetchone()[0] == 1  # eski açık parolalar dosyada kalmasın


def test_tek_admin_parolasi_ve_belirlenen_musteri_parolasi():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.admin_parolasi() is None
        for kotu in ("kisa", "", None):
            try:
                v.admin_parolasi_koy(kotu)
                assert False, kotu
            except ValueError:
                pass
        v.admin_parolasi_koy("benim-admin-parolam")
        a = v.admin_parolasi()
        assert a["yineleme"] == 120000 and G.dogru("benim-admin-parolam", a["tuz"], a["ozet"], a["yineleme"])
        assert "benim-admin-parolam" not in json.dumps(a)  # yalnızca özet saklanır
        n, _ = v.musteri_ekle()
        v.cihaz_bagla(n, WG1, SSH)
        assert v.admin_parolasi() == a and v.bagli_cihaz(n)["admin_ozet"] == ""  # bağlanınca rastgele parola üretilmez
        n2, pw = v.musteri_ekle("not", parola="musteri-parola-1")
        assert pw == "musteri-parola-1" and v.parola_dogrula(n2, "musteri-parola-1") and v.musteri(n2)["parola_acik"] == pw
        try:
            v.musteri_ekle(parola="kisa")
            assert False
        except ValueError:
            pass


def test_sms_ayari():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.sms_ayari() is None
        try:
            v.sms_ayari_koy({"mock": False, "usercode": "850", "password": "", "msgheader": "BASLIK", "appkey": ""})
            assert False
        except ValueError as h:
            assert "password" in str(h)
        v.sms_ayari_koy({"mock": True, "usercode": "", "password": "", "msgheader": "", "appkey": ""})
        assert v.sms_ayari()["mock"] is True
        v.sms_ayari_koy({"mock": False, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": ""})
        assert v.sms_ayari() == {"mock": False, "usercode": "8503027084", "password": "gizli-1", "msgheader": "gztp.blgsyr", "appkey": "",
                                 "tr_saglayici": "netgsm", "twilio": V.TWILIO_VARSAYILAN}
        # Twilio'dan önce kaydedilmiş ayar satırı varsayılanlarla okunur
        v.db.execute("UPDATE ayar SET deger = ? WHERE anahtar = 'sms'",
                     ('{"mock": true, "usercode": "", "password": "", "msgheader": "", "appkey": ""}',))
        assert v.sms_ayari()["tr_saglayici"] == "netgsm" and v.sms_ayari()["twilio"] == V.TWILIO_VARSAYILAN


def test_eski_cihaz_tablosuna_admin_sutunlari_eklenir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        yol = os.path.join(tmp, "m.db")
        db = sqlite3.connect(yol)
        # Eski SEMA: admin sütunları YOK
        db.executescript(V.SEMA.replace("  admin_parola TEXT NOT NULL DEFAULT '', admin_tuz TEXT NOT NULL DEFAULT '',\n"
                                        "  admin_ozet TEXT NOT NULL DEFAULT '', admin_yineleme INTEGER NOT NULL DEFAULT 0,\n", ""))
        # Eski şemada müşteri ve bağlı cihaz oluştur
        db.execute("INSERT INTO musteri (numara, tuz, ozet, yineleme, olusturma) VALUES (1111111, 't', 'o', 120000, '2026-01-01T00:00:00+03:00')")
        db.execute("INSERT INTO cihaz (musteri, durum, wg_pub, ssh_pub, anahtar_ozet, baglanma) VALUES (1111111, 'bagli', 'pub1', 'ssh1', 'hash1', '2026-01-01T00:00:00+03:00')")
        # Serbest bırakılmış cihaz (admin parolası almamalı)
        db.execute("INSERT INTO cihaz (musteri, durum, wg_pub, ssh_pub, anahtar_ozet, baglanma) VALUES (1111111, 'serbest', 'pub2', 'ssh2', 'hash2', '2026-01-01T00:00:00+03:00')")
        db.commit()

        # Admin sütunları YOKSA doğrula (replace maçı kanıtla)
        var_oncesi = {r[1] for r in db.execute("PRAGMA table_info(cihaz)")}
        assert "admin_parola" not in var_oncesi
        db.close()

        # Veri açarken migration tetiklenir
        v = V.Veri(yol, saat=lambda: 1_790_000_000.0)

        # Admin sütunları artık var
        var_sonrasi = {r[1] for r in v.db.execute("PRAGMA table_info(cihaz)")}
        assert {"admin_parola", "admin_ozet"} <= var_sonrasi

        # 2026-10-08: cihaz başına rastgele admin parolası yok (tek admin parolası, yönetici belirler)
        for r in v.db.execute("SELECT * FROM cihaz WHERE musteri = 1111111").fetchall():
            assert r["admin_parola"] == "" and r["admin_ozet"] == ""


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
