"""wificorrect_ logosu ve sekme simgesi (2026-10-24): merkezdeki dosyalar cihazdakilerle aynı, sayfaya gömülür, dosya yoksa
çerçeve yazıya düşer."""
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import web  # noqa: E402

KOK = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
DOSYALAR = ("logo-acik.svg", "logo-koyu.svg", "w-koyu.svg", "favicon.svg", "favicon-32.png")


def test_merkez_ve_cihaz_kopyalari_ayni():
    for ad in DOSYALAR:
        with open(os.path.join(web.LOGO_DIZINI, ad), "rb") as m, open(os.path.join(KOK, "src", "sablon", "logo", ad), "rb") as c:
            assert m.read() == c.read(), f"{ad}: scripts/sunucu/merkez/logo ile src/sablon/logo farklı"


def test_kur_sh_her_dosyayi_kurar():
    with open(os.path.join(HERE, "..", "kur.sh"), encoding="utf-8") as f:
        kur = f.read()
    for ad in DOSYALAR:
        assert f"logo/{ad}" in kur, ad


def test_logolar_ve_simge():
    # açık zemin: antrasit "wifi" + mercan "correct_"; koyu zemin: beyaz "wifi" + mercan "correct_"
    assert web.LOGO_ACIK.startswith('<svg class="logo" aria-hidden="true" focusable="false" ') and "#293133" in web.LOGO_ACIK
    assert web.LOGO_KOYU.startswith('<svg class="logo" ') and "#FFFFFF" in web.LOGO_KOYU and "#293133" not in web.LOGO_KOYU
    assert "#FF6247" in web.LOGO_ACIK and "#FF6247" in web.LOGO_KOYU
    for svg in (web.LOGO_ACIK, web.LOGO_KOYU):
        assert svg.endswith("</svg>") and "<script" not in svg and "href=" not in svg
    f = web.FAVICON
    assert f.startswith('<link rel="icon" type="image/png" sizes="32x32" href="data:image/png;base64,iVBORw0KGgo'), f[:120]
    assert '<link rel="icon" type="image/svg+xml" href="data:image/svg+xml;base64,PHN2Zy' in f


def test_cerceve_logo_ve_simge():
    sade = web._cerceve("Deneme", "<p>x</p>")
    bas, govde = sade.split("</head>")
    assert web.FAVICON in bas
    assert govde.startswith(f'<body><header><a class="marka" href="/" aria-label="wificorrect_">{web.LOGO_ACIK}</a>')
    yan = web._cerceve("Deneme", "<p>x</p>", yan="<a href=\"/\">Liste</a>")
    # koyu logo hem sol menüde hem telefondaki üst şeritte; açık logo yok
    assert yan.count(f'<a class="marka" href="/" aria-label="wificorrect_">{web.LOGO_KOYU}</a>') == 2 and web.LOGO_ACIK not in yan


def test_dosya_yoksa_yaziya_duser():
    with tempfile.TemporaryDirectory() as bos:
        assert web.logo_svg("logo-acik.svg", bos) == "" and web.favicon_html(bos) == ""
        with open(os.path.join(bos, "logo-acik.svg"), "w", encoding="utf-8") as f:
            f.write("bozuk")  # SVG değil: gömülmez
        assert web.logo_svg("logo-acik.svg", bos) == ""
    assert web._marka("") == '<a class="marka" href="/" aria-label="wificorrect_">WifiCorrect</a>'


if __name__ == "__main__":
    for _name, _fn in sorted(globals().items()):
        if _name.startswith("test_") and callable(_fn):
            _fn()
            print("ok ", _name)
    print("TUM TESTLER GECTI")
