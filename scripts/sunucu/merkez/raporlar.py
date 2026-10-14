"""Panel: haftalık ve aylık raporlar (spec 2026-10-14). Root'un (rapor.py, wc-istatistik) yazdığı
/var/lib/wificorrect/rapor/<numara>/{liste.json, hafta-*.json, ay-*.json} dosyalarını okur ve HTML'e çevirir.
Sayfa açılışında arşiv (/srv) ya da rapor-ham okunmaz. Raporda kişisel veri yoktur (yalnızca sayılar): gerekçe istenmez,
hareket yazılmaz. JSON güvenilmez kabul edilir: her alan tür denetlenir, her metin e() ile basılır. JS yok."""
import datetime
import json
import math
import os
import re

import common
import detay
from web import bilgi_tablosu, e, kart, yanit_html

RAPOR = "/var/lib/wificorrect/rapor"
BICIM = 1
DOSYA_SINIRI = 1024 * 1024
NUMARA_RE = re.compile(r"[1-9][0-9]{6}", re.ASCII)
DONEM_RE = {"hafta": re.compile(r"(\d{4})-W(\d{2})", re.ASCII), "ay": re.compile(r"(\d{4})-(\d{2})", re.ASCII)}
YOL_RE = re.compile(r"/rapor/(hafta|ay)/([^/]*)")
SURUM_RE = re.compile(r"[0-9A-Za-z.+~-]{1,32}")
DURUMLAR = ("tam", "kayit_yok", "eksik", "dogrulanamadi", "bekleniyor", "bugun", "gelecek")
NEDEN = {"okunamadi": "kayıt dosyaları okunamadı", "dosya": "bir dosyanın özeti (hash) MANIFEST'le tutmuyor",
         "zincir_yok": "gün mühür zincirinde yok", "zincir": "mühür zinciri satırı bozuk",
         "manifest": "MANIFEST zincirdeki özetle tutmuyor"}
TAM_ACIKLAMA = "Tam: kayıt geldi, mühürlü, merkezde yedekli, bütünlük (MANIFEST ve mühür zinciri) doğrulandı."
ALAN_EN_COK, LISTE_EN_COK = 10, 100


# --- tür denetimi ---
def _sayi(v):
    return v if isinstance(v, int) and not isinstance(v, bool) and v >= 0 else 0


def _ondalik(v):
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v) or v < 0:
        return 0.0
    return float(v)


def _gun(v):
    return v if detay.gecerli_gun(v) else None


def donem_gecerli(tur, donem):
    m = DONEM_RE.get(tur).fullmatch(donem) if tur in DONEM_RE and isinstance(donem, str) else None
    if not m:
        return False
    return 1 <= int(m.group(2)) <= (53 if tur == "hafta" else 12)


def _liste_ogesi(x):
    if not isinstance(x, dict) or not donem_gecerli(x.get("tur"), x.get("donem")):
        return None
    bas, bit = _gun(x.get("baslangic")), _gun(x.get("bitis"))
    if bas is None or bit is None:
        return None
    return {"tur": x["tur"], "donem": x["donem"], "baslangic": bas, "bitis": bit, "kesin": x.get("kesin") is True,
            "farkli": _sayi(x.get("farkli")), "kayit_gun": _sayi(x.get("kayit_gun")), "kayit_tam": _sayi(x.get("kayit_tam"))}


def _sozluk(v):
    return v if isinstance(v, dict) else {}


def _sayi_listesi(v, n):
    v = v if isinstance(v, list) else []
    return [_sayi(x) for x in v[:n]] + [0] * max(0, n - len(v))


def normal_rapor(r, tur, donem):
    """Dosyadan gelen raporun güvenli kopyası; biçim, tür, dönem ya da tarihler tutmuyorsa None."""
    if not isinstance(r, dict) or r.get("bicim") != BICIM or r.get("tur") != tur or r.get("donem") != donem:
        return None
    bas, bit = _gun(r.get("baslangic")), _gun(r.get("bitis"))
    if bas is None or bit is None:
        return None
    m = _sozluk(r.get("misafir"))
    ey = m.get("en_yogun_gun")
    en_yogun = ({"gun": ey["gun"], "farkli": _sayi(ey.get("farkli"))}
                if isinstance(ey, dict) and _gun(ey.get("gun")) else None)
    gunler = []
    for g in r.get("gunler") if isinstance(r.get("gunler"), list) else []:
        if not isinstance(g, dict) or _gun(g.get("gun")) is None:
            continue
        f = g.get("farkli")
        gunler.append({"gun": g["gun"], "durum": g.get("durum") if g.get("durum") in DURUMLAR else "bilinmiyor",
                       "neden": g.get("neden") if g.get("neden") in NEDEN else "",
                       "farkli": f if isinstance(f, int) and not isinstance(f, bool) and f >= 0 else None})
    k, ku = _sozluk(r.get("kayit")), _sozluk(r.get("kullanim"))
    alanlar = []
    for a in ku.get("alanlar") if isinstance(ku.get("alanlar"), list) else []:
        if isinstance(a, dict) and isinstance(a.get("alan"), str) and 0 < len(a["alan"]) <= 253:
            alanlar.append({"alan": a["alan"], "misafir": _sayi(a.get("misafir"))})
    surum = r.get("surum")
    return {"bicim": BICIM, "tur": tur, "donem": donem, "baslangic": bas, "bitis": bit,
            "uretim": r.get("uretim") if isinstance(r.get("uretim"), str) else "", "kesin": r.get("kesin") is True,
            "misafir": {a: _sayi(m.get(a)) for a in ("farkli", "yeni", "donen", "giris", "giris_sms", "giris_kod", "izinli_cihaz")}
            | {"gunluk_ortalama": _ondalik(m.get("gunluk_ortalama")), "en_yogun_gun": en_yogun},
            "saat": _sayi_listesi(r.get("saat"), 24), "hafta_gunu": _sayi_listesi(r.get("hafta_gunu"), 7),
            "gunler": gunler, "kayit": {"gun": _sayi(k.get("gun")), "tam": _sayi(k.get("tam"))},
            "kullanim": {"gonderilen": _sayi(ku.get("gonderilen")), "alinan": _sayi(ku.get("alinan")), "sms": _sayi(ku.get("sms")),
                         "alanlar": alanlar[:ALAN_EN_COK]},
            "surum": surum if isinstance(surum, str) and SURUM_RE.fullmatch(surum) else None,
            "surum_sonra_degisti": r.get("surum_sonra_degisti") is True}


class Raporlar:
    """Yalnızca `<dizin>/<numara>/` altındaki iki tür dosyayı okur; yol yalnızca doğrulanmış numara ve dönemden kurulur."""

    def __init__(self, dizin=RAPOR):
        self.dizin = dizin

    def _oku(self, numara, ad):
        if not NUMARA_RE.fullmatch(str(numara)):
            return None
        try:
            with open(os.path.join(self.dizin, str(numara), ad), "rb") as f:
                b = f.read(DOSYA_SINIRI + 1)
            return json.loads(b.decode("utf-8")) if len(b) <= DOSYA_SINIRI else None
        except (OSError, ValueError):
            return None

    def liste(self, numara):
        ham = self._oku(numara, "liste.json")
        out = [x for x in (_liste_ogesi(x) for x in (ham if isinstance(ham, list) else [])[:LISTE_EN_COK]) if x]
        return out

    def rapor(self, numara, tur, donem):
        if not donem_gecerli(tur, donem):
            return None
        return normal_rapor(self._oku(numara, f"{tur}-{donem}.json"), tur, donem)


# --- biçimlendirme ---
def iyelik(n):
    """Sayıdan sonra kesme işaretiyle gelen 3. tekil iyelik eki: 6'sı, 31'i, 4'ü, 10'u, 40'ı."""
    n = abs(int(n))
    if n == 0:
        return "ı"
    if n % 10:
        return ("i", "si", "ü", "ü", "i", "sı", "si", "i", "u")[n % 10 - 1]
    if n // 10 % 10:
        return ("u", "si", "u", "ı", "si", "ı", "i", "i", "ı")[n // 10 % 10 - 1]
    return "ü" if n // 100 % 10 else "i"


def tam_metni(gun, tam):
    """Yalnızca tamsayılardan kurulur (kaçış gerekmez; kesme işareti e()'de &#x27; olurdu)."""
    return f"{gun} günün {tam}'{iyelik(tam)} tam" if gun else "Henüz biten gün yok"


def _t(g):
    return datetime.date.fromisoformat(g)


def donem_adi(tur, bas, bit):
    """Hafta: '12–18.10.2026', '29.09–05.10.2026', '28.12.2026–03.01.2027'; ay: 'Ekim 2026'."""
    b, s = _t(bas), _t(bit)
    if tur == "ay":
        return f"{detay.AYLAR[b.month - 1]} {b.year}"
    if b.year != s.year:
        return f"{b:%d.%m.%Y}–{s:%d.%m.%Y}"
    if b.month != s.month:
        return f"{b:%d.%m}–{s:%d.%m.%Y}"
    return f"{b:%d}–{s:%d.%m.%Y}"


def _bugun(simdi):
    return datetime.datetime.fromtimestamp(simdi, common.TZ).date()


def sure_metni(sn):
    sn = max(0, int(sn))
    dk = sn // 60
    if dk < 60:
        return f"{dk} dk"
    if dk < 1440:
        return f"{dk // 60} sa {dk % 60} dk"
    return f"{dk // 1440} gün {dk % 1440 // 60} sa"


def _epoch(iso):
    try:
        z = datetime.datetime.fromisoformat(str(iso))
    except ValueError:
        return None
    return (z if z.tzinfo else z.replace(tzinfo=common.TZ)).timestamp()


def donem_sinirlari(bas, bit):
    """[baş 00:00, bitişten sonraki gün 00:00) TR, ISO metin ve epoch."""
    a = datetime.datetime.combine(_t(bas), datetime.time(), common.TZ)
    b = datetime.datetime.combine(_t(bit) + datetime.timedelta(days=1), datetime.time(), common.TZ)
    return a.isoformat(), b.isoformat(), a.timestamp(), b.timestamp()


def cevrimdisi_sn(sorunlar, bas, bit, simdi):
    """`ulasilamiyor` sorunlarının dönemle (ve şimdiye kadarla) kesişen toplam süresi (dönem öncesi açılıp süren dahil)."""
    _, _, a, b = donem_sinirlari(bas, bit)
    b = min(b, simdi)
    top = 0
    for s in sorunlar:
        if s.get("tur") != "ulasilamiyor":
            continue
        ac = _epoch(s.get("acilis"))
        kp = _epoch(s.get("kapanis")) if s.get("kapanis") else simdi
        if ac is None or kp is None:
            continue
        top += max(0.0, min(kp, b) - max(ac, a))
    return int(top)


def donemde_acilan(sorunlar, bas, bit):
    """Dönemde açılan kritik/uyarı sorunlar (bilgi, ör. lisans, raporda yok)."""
    a, b, _, _ = donem_sinirlari(bas, bit)
    return [s for s in sorunlar if s.get("onem") in ("kritik", "uyari") and a <= str(s.get("acilis", "")) < b]


# --- sayfalar ---
def liste_html(liste, onek, simdi):
    if not liste:
        return kart("<h1>Raporlar</h1><p>Henüz rapor yok; ilk gün arşive geldiğinde oluşur.</p>")
    bugun = _bugun(simdi).isoformat()
    bolumler = ""
    for tur, baslik in (("hafta", "Haftalık raporlar"), ("ay", "Aylık raporlar")):
        satirlar = []
        for x in liste:
            if x["tur"] != tur:
                continue
            etiket = ('<b class="uyarisi">sürüyor</b>' if x["bitis"] >= bugun
                      else ("" if x["kesin"] else '<span class="alt">kesinleşmedi</span>'))
            satirlar.append(
                f'<tr><td><a href="{e(onek)}/rapor/{e(tur)}/{e(x["donem"])}">{e(donem_adi(tur, x["baslangic"], x["bitis"]))}</a>'
                f' {etiket}</td><td class="n">{x["farkli"]}</td><td>{tam_metni(x["kayit_gun"], x["kayit_tam"])}</td></tr>')
        govde = ('<div class="kaydir"><table class="detay"><tr><th>Dönem</th><th>Farklı misafir</th><th>5651 kaydı</th></tr>'
                 + "".join(satirlar) + "</table></div>") if satirlar else '<p class="alt">Bu türde rapor yok.</p>'
        bolumler += kart(f"<h2>{baslik}</h2>{govde}")
    return (kart("<h1>Raporlar</h1><p class=\"alt\">Misafir sayıları, 5651 kayıt durumu ve cihazın çalışması. Raporlarda "
                 "kişisel veri (telefon, ad, MAC, IP) yoktur. Her gün 02:30 ve 12:30'da güncellenir.</p>") + bolumler)


def _cubuklar(etiketler, sayilar):
    en = max(sayilar) or 1
    return "<table>" + "".join(
        f'<tr><td>{e(et)}</td><td class="cubuk"><span style="width:{n * 100 // en}%"></span></td><td class="n">{n}</td></tr>'
        for et, n in zip(etiketler, sayilar)) + "</table>"


def _gun_durumu(g):
    d = g["durum"]
    if d == "tam":
        return '<span class="iyi">✔ Tam</span>'
    if d == "kayit_yok":
        return '<b class="kotu">Kayıt gelmedi</b> <span class="alt">(cihaz kapalı, bağlantısız ya da yedek gönderilmedi)</span>'
    if d == "eksik":
        return '<b class="kotu">Eksik geldi</b> <span class="alt">(günün dosyaları merkeze tam ulaşmadı)</span>'
    if d == "dogrulanamadi":
        return f'<b class="kotu">Doğrulanamadı</b> <span class="alt">({e(NEDEN.get(g["neden"], "neden bilinmiyor"))})</span>'
    if d == "bekleniyor":
        return '<span class="uyarisi">Bekleniyor</span> <span class="alt">(gece yedeğiyle gelir)</span>'
    if d == "bugun":
        return "Sürüyor"
    if d == "gelecek":
        return '<span class="bos">—</span>'
    return '<span class="bos">Bilinmiyor</span>'


def rapor_html(r, baslik, unvan, numara, sorunlar, cevrimdisi, cihaz_surumu, simdi, onek=""):
    """r: normal_rapor çıktısı; sorunlar: dönemde açılan kritik/uyarı sorunlar; cevrimdisi: saniye."""
    bugun = _bugun(simdi).isoformat()
    ad = donem_adi(r["tur"], r["baslangic"], r["bitis"])
    tur_adi = "Haftalık rapor" if r["tur"] == "hafta" else "Aylık rapor"
    suruyor = r["bitis"] >= bugun
    durum = ('<p class="uyarisi">Dönem sürüyor — her gün 02:30 ve 12:30\'da güncellenir.</p>' if suruyor else
             ("" if r["kesin"] else '<p class="alt">Geç gelen günlerle birkaç gün içinde değişebilir.</p>'))
    ust = (f'<p class="yazdirma alt"><a href="{e(onek)}/raporlar">← Raporlar</a> · Yazdırmak ya da PDF olarak kaydetmek '
           "için Ctrl+P'ye basın (telefonda tarayıcı menüsünden Yazdır / Paylaş).</p>")
    bas_kart = kart(f"<h1>{e(tur_adi)} · {e(ad)}</h1>" + bilgi_tablosu([
        ("İşletme", e(baslik)), ("Unvan", e(unvan or "—")), ("Müşteri no", e(numara)),
        ("Dönem", e(f'{common.tarih_goster(r["baslangic"])} – {common.tarih_goster(r["bitis"])}')),
        ("Üretim", e(common.tarih_goster(r["uretim"]) or "—"))]) + durum)

    satirlar = "".join(
        f'<tr><td>{e(common.tarih_goster(g["gun"]))}</td><td>{e(detay.KISA_GUN[_t(g["gun"]).weekday()])}</td>'
        f'<td>{_gun_durumu(g)}</td><td class="n">{"" if g["farkli"] is None else g["farkli"]}</td></tr>' for g in r["gunler"])
    kayit = kart(f'<h2>5651 kayıt durumu</h2><p><b>{tam_metni(r["kayit"]["gun"], r["kayit"]["tam"])}</b></p>'
                 f'<p class="alt">{e(TAM_ACIKLAMA)}</p>'
                 '<div class="kaydir"><table class="detay"><tr><th>Tarih</th><th>Gün</th><th>Durum</th><th>Misafir</th></tr>'
                 f"{satirlar}</table></div>")

    m = r["misafir"]
    ey = m["en_yogun_gun"]
    en_yogun = (f'{common.tarih_goster(ey["gun"])} {detay.GUN_ADI[_t(ey["gun"]).weekday()]} · {ey["farkli"]} misafir'
                if ey else "—")
    misafir = kart("<h2>Misafirler</h2>" + bilgi_tablosu([
        ("Farklı misafir", str(m["farkli"])), ("İlk kez gelen (yeni)", str(m["yeni"])), ("Daha önce de gelmiş", str(m["donen"])),
        ("Toplam giriş", str(m["giris"])), ("Günlük ortalama", e(f'{m["gunluk_ortalama"]:.1f}'.replace(".", ","))),
        ("En yoğun gün", e(en_yogun))]) + "<h3>Giriş yöntemi</h3>" + bilgi_tablosu([
            ("SMS ile giriş", str(m["giris_sms"])), ("Misafir kodu ile giriş", str(m["giris_kod"])),
            ("İzinli cihaz (portalsız)", str(m["izinli_cihaz"]))]))

    yogunluk = kart("<h2>Yoğunluk</h2><p class=\"alt\">Her satır: o saatte ya da günde internete çıkan farklı misafir.</p>"
                    "<h3>Saate göre</h3>" + _cubuklar([f"{h:02d}–{(h + 1) % 24:02d}" for h in range(24)], r["saat"])
                    + "<h3>Haftanın gününe göre</h3>" + _cubuklar(detay.GUN_ADI, r["hafta_gunu"]))

    ku = r["kullanim"]
    if ku["alanlar"]:
        alanlar = ('<div class="kaydir"><table class="detay"><tr><th>Alan adı</th><th>Farklı misafir</th></tr>'
                   + "".join(f'<tr><td>{e(a["alan"])}</td><td class="n">{a["misafir"]}</td></tr>' for a in ku["alanlar"])
                   + "</table></div>")
    else:
        alanlar = '<p class="alt">Gösterilecek alan adı yok.</p>'
    kullanim = kart("<h2>Kullanım</h2>" + bilgi_tablosu([
        ("Toplam veri", e(f'{detay.boyut(ku["gonderilen"] + ku["alinan"])} (indirme {detay.boyut(ku["alinan"])}, '
                          f'yükleme {detay.boyut(ku["gonderilen"])})')),
        ("Doğrulama SMS'i", str(ku["sms"]))])
        + "<h3>En çok ziyaret edilen alan adları</h3>" + alanlar
        + '<p class="alt">Yalnızca en az 3 farklı misafirin ziyaret ettiği alan adları gösterilir (tek kişiyi ele vermesin).</p>')

    if r["surum"]:
        surum = r["surum"]
    elif not r["surum_sonra_degisti"] and cihaz_surumu:
        surum = cihaz_surumu
    else:
        surum = "—"
    sorun_satirlari = []
    for s in sorunlar:
        ac = _epoch(s.get("acilis"))
        kp = _epoch(s.get("kapanis")) if s.get("kapanis") else None
        sure = sure_metni((kp if kp is not None else simdi) - ac) if ac is not None else "—"
        kapanis = e(common.tarih_goster(s.get("kapanis"))) if s.get("kapanis") else '<b class="kotu">açık</b>'
        sorun_satirlari.append(
            f'<tr><td>{e(s.get("metin", ""))}</td><td>{"Kritik" if s.get("onem") == "kritik" else "Uyarı"}</td>'
            f'<td>{e(common.tarih_goster(s.get("acilis")))}</td><td>{kapanis}</td><td>{e(sure)}</td></tr>')
    sorun_tablo = ('<div class="kaydir"><table class="detay"><tr><th>Sorun</th><th>Önem</th><th>Açılış</th><th>Kapanış</th>'
                   "<th>Süre</th></tr>" + "".join(sorun_satirlari) + "</table></div>" if sorun_satirlari
                   else '<p class="alt">Bu dönemde açılan sorun yok.</p>')
    cihaz = kart("<h2>Cihaz</h2>" + bilgi_tablosu([
        ("Toplam çevrimdışı süre", e(f"en az {sure_metni(cevrimdisi)}") if cevrimdisi else "Yok"),
        ("Yazılım sürümü (dönem sonu)", e(surum))]) + "<h3>Dönemde açılan sorunlar</h3>" + sorun_tablo
        + '<p class="alt">Çevrimdışı süre merkezin cihaza ulaşamadığını fark ettiği andan sayılır; gerçek süre biraz daha uzun olabilir.</p>')
    return ust + bas_kart + kayit + misafir + yogunluk + kullanim + cihaz


def istek(app, ot, numara, alt, onek):
    """Müşteri paneli (onek '') ve yönetim (onek '/m/<n>') için ortak: alt '/raporlar' ya da '/rapor/<tür>/<dönem>'.
    Gerekçe istenmez, hareket yazılmaz (kişisel veri yok). Okunan: liste.json ya da tek dönem JSON'u + sorun tablosu."""
    simdi = app.saat()
    if alt == "/raporlar":
        return yanit_html(app.sayfa("Raporlar", liste_html(app.raporlar.liste(numara), onek, simdi), ot, genis=True))
    m = YOL_RE.fullmatch(alt)
    r = app.raporlar.rapor(numara, m.group(1), m.group(2)) if m else None
    if r is None:
        return yanit_html(app.mesaj(ot, "Bulunamadı", "Bu dönem için rapor yok."), 404)
    c = app.veri.bagli_cihaz(numara)
    baslik = (c["isletme_adi"] if c is not None and c["isletme_adi"] else f"Müşteri {numara}")
    unvan, surum = (c["unvan"], c["surum"]) if c is not None else ("", "")
    a, b, _, _ = donem_sinirlari(r["baslangic"], r["bitis"])
    sorunlar = app.veri.sorunlar_aralik(numara, a, b)
    h = rapor_html(r, baslik, unvan, numara, donemde_acilan(sorunlar, r["baslangic"], r["bitis"]),
                   cevrimdisi_sn(sorunlar, r["baslangic"], r["bitis"], simdi), surum, simdi, onek)
    return yanit_html(app.sayfa(f'Rapor {donem_adi(r["tur"], r["baslangic"], r["bitis"])}', h, ot, genis=True))
