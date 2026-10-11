import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import veri as V  # noqa: E402


def yeni(tmp, saat=None):
    return V.Veri(os.path.join(tmp, "m.db"), saat=saat or (lambda: 1_790_000_000.0))


def test_veri_saglik_tek_satir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        t = [1_790_000_000.0]
        v = yeni(tmp, lambda: t[0])
        assert v.saglik(7) is None
        v.saglik_kaydet(7, {"disk": 41, "servisler": ["a"]})
        s1 = v.saglik(7)
        assert s1["rapor"] == {"disk": 41, "servisler": ["a"]} and s1["zaman"].startswith("2026-")
        t[0] += 300
        v.saglik_kaydet(7, {"disk": 42})
        s2 = v.saglik(7)
        assert s2["rapor"] == {"disk": 42} and s2["zaman"] > s1["zaman"]
        assert v._bir("SELECT COUNT(*) n FROM saglik")["n"] == 1


def test_sorun_ac_ayni_turu_tekrar_acmaz():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        a = v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%91)", "uyari")
        assert v.sorun_ac(1000001, "disk_dolu", "Disk dolu (%95)", "uyari") == a
        assert v.sorun_ac(1000001, "rapor_yok", "Rapor gelmiyor", "kritik") != a
        assert v.sorun_ac(1000002, "disk_dolu", "Disk dolu (%91)", "uyari") != a
        assert len(v.acik_sorunlar()) == 3 and len(v.acik_sorunlar(1000001)) == 2
        assert v.acik_sorunlar(1000001)[0]["kapanis"] == ""


def test_sorun_kapat_ve_gecmis():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        t = [1_790_000_000.0]
        v = yeni(tmp, lambda: t[0])
        assert v.son_kapanan(5, "disk_dolu") is None
        a = v.sorun_ac(5, "disk_dolu", "Disk dolu (%91)", "uyari")
        v.sorun_sms(a, "acilis_sms", "mock")
        t[0] += 600
        v.sorun_kapat(a)
        v.sorun_sms(a, "kapanis_sms", "hata:30")
        assert v.acik_sorunlar(5) == []
        k = v.son_kapanan(5, "disk_dolu")
        assert k["id"] == a and k["kapanis"] > k["acilis"] and k["acilis_sms"] == "mock" and k["kapanis_sms"] == "hata:30"
        b = v.sorun_ac(5, "disk_dolu", "Disk dolu (%92)", "uyari")  # kapandıktan sonra yenisi açılır
        assert b != a
        g = v.sorun_gecmisi(5)
        assert [x["id"] for x in g] == [b, a] and len(v.sorun_gecmisi(5, limit=1)) == 1
        try:
            v.sorun_sms(a, "baska", "mock")
            assert False
        except ValueError:
            pass


def test_uyari_ayari_dogrulama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.uyari_ayari() == {"telefonlar": [], "acik": False, "gunluk_sinir": 20}
        v.uyari_ayari_koy({"telefonlar": ["0533 455 31 32"], "acik": True, "gunluk_sinir": 5})
        assert v.uyari_ayari() == {"telefonlar": ["905334553132"], "acik": True, "gunluk_sinir": 5}
        for kotu in ({"telefonlar": ["0533 455 31 32", "0533 455 31 33", "0533 455 31 34", "0533 455 31 35"]},
                     {"telefonlar": ["12345"]}):
            try:
                v.uyari_ayari_koy(kotu)
                assert False
            except ValueError:
                pass
        assert v.uyari_ayari()["telefonlar"] == ["905334553132"]  # hatalı kayıt eskisini bozmaz


def test_sms_sayaci_gun_bazli():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v = yeni(tmp)
        assert v.sms_sayaci("2026-10-11") == 0
        assert v.sms_say("2026-10-11") == 1 and v.sms_say("2026-10-11") == 2
        assert v.sms_sayaci("2026-10-11") == 2 and v.sms_sayaci("2026-10-12") == 0


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
