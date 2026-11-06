import hashlib
import io
import json
import os
import re
import stat
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import yardim as Y  # noqa: E402
import surumler as S  # noqa: E402

LISTE = "https://api.github.com/repos/bilgisayargoztepe-lgtm/wificorrect/releases?per_page=10"


def sha(b):
    return hashlib.sha256(b).hexdigest()


def release(tag, ad_deb, ad_sums):
    return {"tag_name": tag, "assets": [
        {"name": ad_deb, "browser_download_url": f"https://x/{tag}/{ad_deb}"},
        {"name": ad_sums, "browser_download_url": f"https://x/{tag}/{ad_sums}"}]}


def sahte(dosyalar, cagrilar):
    def getir(url, sinir):
        cagrilar.append(url)
        b = dosyalar[url]
        if len(b) > sinir:
            raise ValueError("çok büyük")
        return b
    return getir


def kur_dosyalar(deb041=b"deb-0.4.1", sums_sha=None, ek=()):
    d041, d040 = deb041, b"deb-0.4.0"
    rel = [release("v0.4.1", "wificorrect_0.4.1_amd64.deb", "SHA256SUMS"),
           release("v0.4.0", "wificorrect_0.4.0_amd64.deb", "SHA256SUMS"), *ek]
    return {LISTE: json.dumps(rel).encode(),
            "https://x/v0.4.1/wificorrect_0.4.1_amd64.deb": d041,
            "https://x/v0.4.1/SHA256SUMS": f"{sums_sha or sha(d041)}  wificorrect_0.4.1_amd64.deb\n".encode(),
            "https://x/v0.4.0/wificorrect_0.4.0_amd64.deb": d040,
            "https://x/v0.4.0/SHA256SUMS": f"{sha(d040)}  wificorrect_0.4.0_amd64.deb\n".encode()}


def test_github_al_dogru_ekler_yanlisi_atlar_tekrar_indirmez():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        dizin = os.path.join(tmp, "surumler")
        f = kur_dosyalar(sums_sha="0" * 64)  # 0.4.1'in SHA'sı tutmuyor
        c = []
        eklenen, hatalar = S.github_al(v, dizin, sahte(f, c))
        assert eklenen == ["0.4.0"] and len(hatalar) == 1 and "0.4.1" in hatalar[0]
        assert [s["surum"] for s in v.surumler()] == ["0.4.0"]
        yol = S.paket_yolu(dizin, "0.4.0")
        assert open(yol, "rb").read() == b"deb-0.4.0"
        if os.name != "nt":
            assert stat.S_IMODE(os.stat(yol).st_mode) == 0o644
        assert not os.path.exists(S.paket_yolu(dizin, "0.4.1")) and not [a for a in os.listdir(dizin) if a.endswith(".tmp")]
        k = v.surum("0.4.0")
        assert k["sha256"] == sha(b"deb-0.4.0") and k["boyut"] == 9 and k["kaynak"] == "github"
        # ikinci çağrı: 0.4.0 yeniden indirilmez
        c.clear()
        eklenen, _ = S.github_al(v, dizin, sahte(f, c))
        assert eklenen == [] and not any("v0.4.0/wificorrect" in u for u in c)
        # düzelince 0.4.1 eklenir
        eklenen, hatalar = S.github_al(v, dizin, sahte(kur_dosyalar(), []))
        assert eklenen == ["0.4.1"] and hatalar == []
        assert [s["surum"] for s in v.surumler()] == ["0.4.1", "0.4.0"]


def test_gecersiz_etiket_atlanir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        f = kur_dosyalar(ek=[release("v1.2", "wificorrect_1.2_amd64.deb", "SHA256SUMS"),
                             release("vx", "wificorrect_x_amd64.deb", "SHA256SUMS")])
        c = []
        eklenen, hatalar = S.github_al(v, os.path.join(tmp, "s"), sahte(f, c))
        assert sorted(eklenen) == ["0.4.0", "0.4.1"] and hatalar == []
        assert not any("v1.2" in u or "vx" in u for u in c)


def test_buyuk_dosya_reddedilir():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        buyuk = b"x" * (S.EN_COK_BAYT + 1)
        # getir sınırı uygulamasa bile github_al uzunluğa bakar
        f = kur_dosyalar(deb041=buyuk)
        eklenen, hatalar = S.github_al(v, os.path.join(tmp, "s"), lambda u, s: f[u])
        assert eklenen == ["0.4.0"] and len(hatalar) == 1
        assert v.surum("0.4.1") is None
        assert not os.path.exists(S.paket_yolu(os.path.join(tmp, "s"), "0.4.1"))


def test_yerel_ekle_ve_siralama():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        dizin = os.path.join(tmp, "surumler")
        dosya = os.path.join(tmp, "a.deb")
        with open(dosya, "wb") as f:
            f.write(b"yerel")
        S.yerel_ekle(v, dizin, dosya, "0.10.0")
        S.yerel_ekle(v, dizin, dosya, "0.9.2")
        assert [s["surum"] for s in v.surumler()] == ["0.10.0", "0.9.2"]  # sayısal sıra
        assert v.surum("0.9.2")["kaynak"] == "yerel" and v.surum("9.9.9") is None
        assert open(S.paket_yolu(dizin, "0.9.2"), "rb").read() == b"yerel"
        try:
            S.yerel_ekle(v, dizin, dosya, "../x")
            assert False
        except ValueError:
            pass


def test_otomatik_al_ekleneni_harekete_hatayi_gunluge_yazar():
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        v, _, _ = Y.ortam(tmp)
        dizin = os.path.join(tmp, "surumler")
        gunluk = io.StringIO()
        assert S.otomatik_al(v, dizin, sahte(kur_dosyalar(sums_sha="0" * 64), []), gunluk) == 0
        assert [s["surum"] for s in v.surumler()] == ["0.4.0"]
        alinan = [h for h in v.hareketler() if h["olay"] == "SURUM_ALINDI"]
        assert [(h["kim"], h["ayrinti"]) for h in alinan] == [("surumler", "surum=0.4.0 otomatik")]
        assert "0.4.1" in gunluk.getvalue() and "SHA256 tutmuyor" in gunluk.getvalue()
        assert not any("0.4.1" in h["ayrinti"] for h in v.hareketler())  # hata harekete yazılmaz

        def ulasilamaz(url, sinir):
            raise OSError("ag yok")
        gunluk = io.StringIO()
        assert S.otomatik_al(v, dizin, ulasilamaz, gunluk) == 1
        assert "ag yok" in gunluk.getvalue() and len(v.hareketler()) == 1


def test_kur_sh_ve_saatlik_zamanlayici():
    kok = os.path.dirname(HERE)
    kur = open(os.path.join(kok, "kur.sh"), encoding="utf-8").read()
    # Rust ikilisine geçişten sonra (asıl denetim merkez/src/kurulum_testler.rs): birim ikiliyi çağırır
    assert re.search(r"install -m 644 [^\n]*\\\n[^\n]*\\\n[^\n]*wc-surumler\.service wc-surumler\.timer \$BIRIMLER/", kur)
    assert re.search(r"^ZAMANLAYICILAR=.*wc-surumler\.timer", kur, re.M) and "systemctl enable -q --now $ZAMANLAYICILAR" in kur
    zam = open(os.path.join(kok, "wc-surumler.timer"), encoding="utf-8").read()
    assert "OnCalendar=hourly" in zam and "Persistent=true" in zam
    srv = open(os.path.join(kok, "wc-surumler.service"), encoding="utf-8").read()
    assert "ExecStart=/usr/local/bin/wificorrect-merkez surumler al" in srv
    assert "User=wcpanel" in srv and "ProtectSystem=strict" in srv and "ReadWritePaths=/var/lib/wificorrect/merkez" in srv


def test_kur_sh_yerel_modulleri_kuruyor():
    # Rust ikilisine geçişten sonra: kur.sh hiçbir .py kurmaz (modüller ikilinin içinde); Python kurulumu sunucuda kalır
    kok = os.path.dirname(HERE)
    satirlar = open(os.path.join(kok, "kur.sh"), encoding="utf-8").read().splitlines()
    kur = "\n".join(s for s in satirlar if not s.lstrip().startswith("#"))
    assert ".py" not in kur and "ln -s" not in kur and "PYTHONDONTWRITEBYTECODE" not in kur
    assert "wificorrect-merkez" in kur


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
