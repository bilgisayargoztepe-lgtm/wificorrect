"""Kafe paneli detay ekranlarının HTML parçaları ve biçimlendirme (detay spec §3). Sayfa çerçevesi panel.py'de.
Özetlerdeki cihazdan gelen tüm metin (alan adı, MAC, ek, ad) güvenilmezdir: hepsi e() ile kaçışlı basılır.
Kişi anahtarı (`telefon` sütunu): telefon (`905…`), `TC:<11 hane>` ya da `PASAPORT:<ISO2>:<NO>`. TC/pasaport ekranda
maskeli (cihazdaki src/kimlik.rs ile aynı), adreslerde/çapalarda kişi yalnızca opak kimlikle (HMAC) geçer."""
import datetime
import hashlib
import hmac
import html
import os
import re
import secrets

GUN_RE = re.compile(r"\d{4}-\d{2}-\d{2}")
SAAT_RE = re.compile(r"\d\d:\d\d")
KISA_GUN = ("Pzt", "Sal", "Çar", "Per", "Cum", "Cmt", "Paz")
GUN_ADI = ("Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar")
AYLAR = ("Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim", "Kasım", "Aralık")
OLAY = {"OTURUM_BASLA": "Giriş", "OTURUM_BITIS": "Çıkış", "OTURUM_IP_DEGISTI": "IP değişti"}
ARAMA_SINIRI, GUN_BAGLANTI_SINIRI = 50, 30


def e(s):
    return html.escape(str(s), quote=True)


def _int(n):
    try:
        return int(n)
    except (TypeError, ValueError):
        return 0


def gecerli_gun(s):
    if not isinstance(s, str) or not GUN_RE.fullmatch(s):
        return False
    try:
        datetime.date.fromisoformat(s)
    except ValueError:
        return False
    return True


def boyut(n):
    n = float(_int(n))
    for birim in ("B", "KB", "MB", "GB"):
        if n < 1024 or birim == "GB":
            return f"{n:.0f} B" if birim == "B" else f"{n:.1f} {birim}".replace(".", ",")
        n /= 1024


def saat(z):
    """ISO zamandan HH:MM; cihazdan gelen alan biçimsizse boş (çıktı hiçbir zaman işaretleme taşımaz)."""
    s = z[11:16] if isinstance(z, str) else ""
    return s if SAAT_RE.fullmatch(s) else ""


def sure(sn):
    sn = _int(sn)
    return f"{sn // 3600}:{sn % 3600 // 60:02d}:{sn % 60:02d}" if sn >= 3600 else f"{sn // 60}:{sn % 60:02d}"


def tarih(g):
    return f"{datetime.date.fromisoformat(g):%d.%m.%Y}" if gecerli_gun(g) else str(g)


def tr_kucuk(s):
    return str(s).replace("İ", "i").replace("I", "ı").lower()


def _ad(k):
    return f'{k.get("ad") or ""} {k.get("soyad") or ""}'.strip() or "(adı yok)"


def _tablo(basliklar, satirlar, bos="Kayıt yok."):
    if not satirlar:
        return f'<p class="alt">{e(bos)}</p>'
    bas = "".join(f"<th>{e(b)}</th>" for b in basliklar)
    govde = "".join("<tr>" + "".join(f"<td>{e(h)}</td>" for h in s) + "</tr>" for s in satirlar)
    return f'<div class="kaydir"><table class="detay"><tr>{bas}</tr>{govde}</table></div>'


# --- kişi anahtarı: maske, opak kimlik, kimlik doğrulama (cihazdaki src/kimlik.rs'in karşılığı) ---
def tc_gecerli(s):
    """T.C. kimlik no: 11 hane, ilk hane 0 değil, 10. ve 11. kontrol haneleri tutar."""
    if not isinstance(s, str) or len(s) != 11 or not all("0" <= c <= "9" for c in s):
        return False
    d = [int(c) for c in s]
    if d[0] == 0:
        return False
    tek, cift = d[0] + d[2] + d[4] + d[6] + d[8], d[1] + d[3] + d[5] + d[7]
    return (tek * 7 - cift) % 10 == d[9] and sum(d[:10]) % 10 == d[10]


def pasaport_normal(s):
    """Boşluk/tire atılır, büyük harf; 5–20 karakter [A-Z0-9]. Değilse None."""
    t = "".join(c for c in str(s) if not c.isspace() and c != "-").upper()
    return t if 5 <= len(t) <= 20 and all(c.isascii() and (c.isupper() or c.isdigit()) for c in t) else None


def ayracsiz(s):
    return "".join(c for c in str(s) if not c.isspace() and c not in "-.,/;:")


def maskele(anahtar):
    """TC `TC:12345678901` → `123******01`; pasaport `PASAPORT:DE:X1234567` → `DE X12***67` (ilk 3 + yıldız + son 2;
    6 karakterden kısa numarada yalnızca ilk karakter açık). Telefon anahtarı aynen. Biçime bakar, sağlamaya bakmaz."""
    anahtar = str(anahtar)
    if anahtar.startswith("TC:"):
        t = anahtar[3:]
        if len(t) == 11 and t.isascii() and t.isdigit():
            return t[:3] + "*" * 6 + t[9:]
        return anahtar
    if anahtar.startswith("PASAPORT:"):
        uyruk, ayrac, no = anahtar[len("PASAPORT:"):].partition(":")
        if ayrac and len(uyruk.encode()) == 2 and no and no.isascii():
            bas, son = (3, 2) if len(no) >= 6 else (1, 0)
            return f"{uyruk} {no[:bas]}{'*' * (len(no) - bas - son)}{no[len(no) - son:]}"
    return anahtar


def opak(sir, anahtar):
    """Adres/çapa için kişi kimliği: HMAC-SHA256(sır, anahtar) ilk 16 hex (cihazdaki kimlik::opak ile aynı)."""
    return hmac.new(sir, str(anahtar).encode("utf-8"), hashlib.sha256).hexdigest()[:16]


def sir_oku_ya_uret(yol):
    """32 baytlık sır: dosya yoksa O_EXCL ile 600 oluşturulur; varsa okunur; bozuksa (yarım yazım) yenisi atomik yazılır."""
    b = secrets.token_bytes(32)
    ikili = getattr(os, "O_BINARY", 0)
    try:
        fd = os.open(yol, os.O_WRONLY | os.O_CREAT | os.O_EXCL | ikili, 0o600)
    except FileExistsError:
        with open(yol, "rb") as f:
            mevcut = f.read()
        if len(mevcut) == 32:
            return mevcut
        gecici = f"{yol}.{os.getpid()}.yeni"
        fd = os.open(gecici, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | ikili, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(b)
            f.flush()
            os.fsync(f.fileno())
        os.replace(gecici, yol)
        return b
    with os.fdopen(fd, "wb") as f:
        f.write(b)
        f.flush()
        os.fsync(f.fileno())
    return b


ARAMA_TURLERI = (("kisi", "Telefon, ad soyad, T.C. kimlik no ya da pasaport no"), ("oda", "Oda numarası"))


def arama_formu(q="", onek="", csrf="", tur="kisi", uyruk=""):
    """POST: aranan değer (telefon/kimlik) adrese ve tarayıcı geçmişine girmez."""
    secenekler = "".join(f'<option value="{d}"{" selected" if d == tur else ""}>{e(ad)}</option>' for d, ad in ARAMA_TURLERI)
    return (f'<form class="ara" method="post" action="{onek}/ara"><input type="hidden" name="csrf" value="{e(csrf)}">'
            f'<input name="q" value="{e(q)}" maxlength="50" placeholder="Telefon, ad soyad, T.C. no, pasaport ya da oda" '
            'aria-label="Kişi ara" autocomplete="off">'
            f'<select name="tur" aria-label="Arama türü">{secenekler}</select>'
            f'<input class="uyruk" name="uyruk" value="{e(uyruk)}" maxlength="2" placeholder="Uyruk" '
            'aria-label="Pasaport uyruğu (isteğe bağlı, ör. DE)" autocomplete="off"><button>Ara</button></form>')


def gunler_html(gunler, sayilar, onek=""):
    """Özeti olan günler aya göre açılır listelerde; en yeni ay açık. Geçersiz gün adları gösterilmez."""
    aylar = {}
    for g in sorted((g for g in gunler if gecerli_gun(g)), reverse=True):
        aylar.setdefault(g[:7], []).append(g)
    if not aylar:
        return ""
    parcalar = []
    for i, liste in enumerate(aylar.values()):
        ilk = datetime.date.fromisoformat(liste[0])
        maddeler = []
        for g in liste:
            d = datetime.date.fromisoformat(g)
            n = f' <span class="alt">· {_int(sayilar[g])} kişi</span>' if g in sayilar else ""
            maddeler.append(f'<li><a href="{onek}/gun/{g}">{d:%d.%m} {KISA_GUN[d.weekday()]}</a>{n}</li>')
        parcalar.append(f'<details{" open" if i == 0 else ""}><summary>{AYLAR[ilk.month - 1]} {ilk.year} '
                        f'<span class="alt">({len(liste)} gün)</span></summary><ul class="gunler">{"".join(maddeler)}</ul></details>')
    return '<div class="kart"><h2>Günler</h2>' + "".join(parcalar) + "</div>"


def kisi_html(k, sir):
    tel = str(k.get("telefon") or "")
    siteler = [s for s in k.get("siteler") or [] if isinstance(s, dict)]
    baglantilar = [b for b in k.get("baglantilar") or [] if isinstance(b, dict)]
    diger, toplam_bag = _int(k.get("diger_site_sayisi")), _int(k.get("baglanti_sayisi"))
    oturum_t = _tablo(("Saat", "Olay", "İç IP", "Not"),
                      [(saat(o.get("zaman")), OLAY.get(o.get("olay"), o.get("olay") or ""), o.get("ic_ip") or "", o.get("ek") or "")
                       for o in k.get("oturumlar") or [] if isinstance(o, dict)],
                      "Bu gün oturum olayı yok (oturum daha önce başlamış).")
    site_t = _tablo(("Site", "Kaç kez", "İlk", "Son"),
                    [(s.get("alan") or "", _int(s.get("sayi")), saat(s.get("ilk")), saat(s.get("son"))) for s in siteler])
    if diger:
        site_t += f'<p class="alt">ve {diger} site daha</p>'
    bag_t = _tablo(("Saat", "Protokol", "Hedef", "Gönderilen", "İndirilen", "Süre"),
                   [(saat(b.get("zaman")), b.get("protokol") or "",
                     f'{b.get("hedef_ip") or ""}:{b.get("hedef_port") or ""}'.rstrip(":"),
                     boyut(b.get("gonderilen")), boyut(b.get("alinan")), sure(b.get("sure_sn"))) for b in baglantilar])
    toplam = _int(k.get("gonderilen")) + _int(k.get("alinan"))
    return (f'<details class="kisi" id="k{opak(sir, tel)}"><summary><b>{e(_ad(k))}</b> · {e(maskele(tel))} · '
            f'{saat(k.get("ilk"))}–{saat(k.get("son"))} · {boyut(toplam)}</summary><div>'
            f'<p class="alt">Cihaz: {e(", ".join(map(str, k.get("mac") or [])) or "—")} · {toplam_bag} bağlantı · '
            f'gönderilen {boyut(k.get("gonderilen"))}, indirilen {boyut(k.get("alinan"))}</p>'
            f'<h3>Oturum</h3>{oturum_t}'
            f'<details><summary>Girdiği siteler ({len(siteler) + diger})</summary><div>{site_t}</div></details>'
            f'<details><summary>Bağlantılar (en çok veri kullanan {len(baglantilar)} / toplam {toplam_bag})</summary>'
            f'<div>{bag_t}</div></details></div></details>')


def gun_icerik(gun, ozet, sir, onek=""):
    d = datetime.date.fromisoformat(gun)
    kisiler = [k for k in ozet.get("kisiler") or [] if isinstance(k, dict)]
    liste = "".join(kisi_html(k, sir) for k in kisiler) or '<p class="alt">Bu gün kayıt yok.</p>'
    return (f'<p><a href="{onek}/">← Özet</a></p><div class="kart"><h1>{d:%d.%m.%Y} {GUN_ADI[d.weekday()]}</h1>'
            f'<p class="alt">{len(kisiler)} kişi · ayrıntı için kişiye dokunun</p></div>{liste}')


def telefon_sorgusu(q):
    """Harf yoksa rakamlar alınır; baştaki 90 (ardından 5 gelirse) ya da 0 atılır: 0533…, +90 533… → 533…"""
    if any(ch.isalpha() for ch in q):
        return None
    d = re.sub(r"\D", "", q)
    if d.startswith("90") and d[2:3] == "5":
        d = d[2:]
    elif d.startswith("0"):
        d = d[1:]
    return d or None


ODA_RE = re.compile(r"[A-Z0-9-]{1,10}")


def oda_normal(q):
    o = str(q).strip().upper()
    return o if ODA_RE.fullmatch(o) else None


def _uyruk(u):
    u = str(u or "").strip().upper()
    return u if len(u) == 2 and u.isascii() and u.isalpha() else None


def ara_sonuclari(dizin, q, tur="kisi", uyruk=""):
    """tur=kisi: telefon biçimindeyse telefonlarda parça araması (bugünkü gibi), harfliyse ad soyadda (Türkçe
    büyük/küçük harf duyarsız); ayrıca geçerli T.C. no (ayraçlar atılır) ya da pasaport no (+ isteğe bağlı uyruk) ile
    tam eşleşme — kimlik numarasında parça araması yok. tur=oda: o odada açılmış misafir oturumları (yalnızca o günler).
    En son geleni önce, 50 kişi."""
    if tur == "oda":
        oda = oda_normal(q)
        bulunan = []
        for tel, k in dizin.items():
            odalar = k.get("odalar") if isinstance(k, dict) else None
            gunler = odalar.get(oda) if oda and isinstance(odalar, dict) else None
            if gunler:
                bulunan.append((tel, dict(k, gunler=sorted(gunler, reverse=True))))
    else:
        rakam, tc, pas = telefon_sorgusu(q), ayracsiz(q), pasaport_normal(q)
        tc = tc if tc_gecerli(tc) else None
        u = _uyruk(uyruk)
        uyruk_gecersiz = bool(str(uyruk or "").strip()) and u is None

        def ad_eslesir(k):
            return not rakam and tr_kucuk(q) in tr_kucuk(f'{k.get("ad") or ""} {k.get("soyad") or ""}')

        def eslesir(tel, k):
            if tel.startswith("TC:"):
                return (tc is not None and tel == "TC:" + tc) or ad_eslesir(k)
            if tel.startswith("PASAPORT:"):
                ku, _, no = tel[len("PASAPORT:"):].partition(":")
                return (pas is not None and no == pas and not uyruk_gecersiz and (u is None or ku == u)) or ad_eslesir(k)
            return (rakam in tel) if rakam else ad_eslesir(k)

        bulunan = [(tel, k) for tel, k in dizin.items() if isinstance(k, dict) and eslesir(str(tel), k)]
    bulunan.sort(key=lambda tk: max(tk[1].get("gunler") or [""]), reverse=True)
    return bulunan[:ARAMA_SINIRI]


def kimlik_sorgusu_mu(q, dizin):
    """Aranan değer bir T.C./pasaport numarası olabilir mi: öyleyse sonuç sayfasında forma geri yazılmaz."""
    if tc_gecerli(ayracsiz(q)):
        return True
    pas = pasaport_normal(q)
    if pas is None:
        return False
    return any(ch.isalpha() for ch in pas) or any(str(t).startswith("PASAPORT:") and str(t).endswith(":" + pas) for t in dizin)


def ara_icerik(q, sonuclar, sir, onek="", csrf="", tur="kisi", uyruk="", mesaj=""):
    """q: forma geri yazılacak değer (kimlik aramasında boş verilir). sonuclar None: arama yapılmadı (mesaj varsa o)."""
    if sonuclar is None:
        govde = f'<p class="alt">{e(mesaj)}</p>' if mesaj else ""
    elif not sonuclar:
        govde = '<p class="alt">Sonuç yok.</p>'
    else:
        parcalar = []
        for tel, k in sonuclar:
            gunler = [g for g in k.get("gunler") or [] if gecerli_gun(g)]
            capa = opak(sir, tel)
            linkler = ", ".join(f'<a href="{onek}/gun/{g}#k{capa}">{tarih(g)}</a>' for g in gunler[:GUN_BAGLANTI_SINIRI])
            fazla = len(gunler) - GUN_BAGLANTI_SINIRI
            if fazla > 0:
                linkler += f' <span class="alt">ve {fazla} gün daha</span>'
            parcalar.append(f'<div class="kart"><b>{e(_ad(k))}</b> · {e(maskele(tel))}<p>{linkler}</p></div>')
        govde = "".join(parcalar)
    return (f'<p><a href="{onek}/">← Özet</a></p><div class="kart"><h1>Kişi ara</h1>{arama_formu(q, onek, csrf, tur, uyruk)}'
            '<p class="alt">T.C. kimlik ve pasaport numarasıyla yalnızca tam numara aranır; sonuçlarda kimlikler maskelidir.</p>'
            f'</div>{govde}')
