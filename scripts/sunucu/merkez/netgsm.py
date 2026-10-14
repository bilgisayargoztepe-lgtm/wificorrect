"""NetGSM: genel SMS ucu (XML, 1:n; merkezden hizmet sağlayıcıya uyarı SMS'i) ve OTP ucu (misafir doğrulama kodu).
Yeniden deneme yok. Parola ve istek gövdesi hiçbir yere yazılmaz."""
import os
import re
import urllib.request
from xml.sax.saxutils import escape

import common

URL = "https://api.netgsm.com.tr/sms/send/xml"
BASARI = ("00", "0")
_DENETIM = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")  # XML'de geçersiz denetim karakterleri


def numaralar_10(numaralar):
    """90XXXXXXXXXX / 0XXXXXXXXXX / 5XXXXXXXXX → 5XXXXXXXXX; geçersizler atlanır."""
    out = []
    for n in numaralar:
        d = common.normalize_phone(str(n))
        if d and d not in out:
            out.append(d)
    return out


def xml_govde(usercode, password, msgheader, metin, numaralar):
    msg = _DENETIM.sub("", str(metin)).replace("]]>", "]]]]><![CDATA[>")  # metin CDATA'dan taşamaz
    nolar = "".join(f"<no>{n}</no>" for n in numaralar_10(numaralar))
    return ('<?xml version="1.0" encoding="UTF-8"?><mainbody><header><company dil="TR">Netgsm</company>'
            f"<usercode>{escape(str(usercode))}</usercode><password>{escape(str(password))}</password>"
            f"<type>1:n</type><msgheader>{escape(str(msgheader))}</msgheader></header>"
            f"<body><msg><![CDATA[{msg}]]></msg>{nolar}</body></mainbody>")


def yanit_kodu(metin):
    """"00 123456" → "00"; XML yanıtta <code>; boşsa ""."""
    metin = (metin or "").strip()
    m = re.search(r"<code>\s*([^<\s]*)\s*</code>", metin)
    if m:
        return m.group(1)
    return metin.split()[0] if metin else ""


def gonder(ayar, metin, numaralar, zaman_asimi=10, ac=None):
    """Dönen: gonderildi | mock | hata:<kod> (AYAR, NUMARA, AG, BILINMIYOR ya da NetGSM kodu).
    WC_TEST ortamında (testler) gerçek gönderici hiç çalışmaz: sahte `ac` verilmediyse hata fırlatır."""
    if ac is None:
        if os.environ.get("WC_TEST"):
            raise RuntimeError("WC_TEST: gerçek NetGSM isteği engellendi")
        ac = urllib.request.urlopen
    if not ayar:
        return "hata:AYAR"
    if ayar.get("mock"):
        return "mock"
    if not all(ayar.get(k) for k in ("usercode", "password", "msgheader")):
        return "hata:AYAR"
    nolar = numaralar_10(numaralar)
    if not nolar:
        return "hata:NUMARA"
    govde = xml_govde(ayar["usercode"], ayar["password"], ayar["msgheader"], metin, nolar).encode("utf-8")
    istek = urllib.request.Request(URL, data=govde, method="POST",
                                   headers={"Content-Type": "application/xml; charset=UTF-8"})
    try:
        with ac(istek, timeout=zaman_asimi) as r:
            yanit = r.read(4096).decode("utf-8", "replace")
    except Exception:  # ağ, zaman aşımı, TLS, HTTP hatası: hepsi aynı sonuç
        return "hata:AG"
    kod = yanit_kodu(yanit)
    if kod in BASARI:
        return "gonderildi"
    return f"hata:{kod}" if re.fullmatch(r"[0-9]{1,8}", kod) else "hata:BILINMIYOR"


# --- Doğrulama (OTP) SMS'i: cihazların misafir kodu merkezden gider (2026-10-13). Cihazdaki sms::build_xml /
# parse_response ile aynı biçim. Yeniden deneme yok (çift SMS = çift ücret); kod ve parola hiçbir yere yazılmaz.
OTP_URL = "https://api.netgsm.com.tr/sms/send/otp"
# işlem başına zaman aşımı (sn): bağlan + oku (TLS dahil) en kötü ~10-11 sn, cihaz merkezi 15 sn bekler. Çağrı bitmeden
# cevap dönülmez (SMS gitmiş ama kod atılmış olmasın).
ISLEM_SURE = 5
_YANIT_KODU = re.compile(r"[0-9A-Za-z]{1,16}")
_IS_NO = re.compile(r"[0-9A-Za-z_-]{1,64}")


def _xml_kacis(s):
    return (str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
            .replace('"', "&quot;").replace("'", "&apos;"))


def otp_xml(usercode, password, msgheader, appkey, metin, telefon_10):
    e = _xml_kacis
    ak = f"<appkey>{e(appkey)}</appkey>" if appkey else ""
    return ("<?xml version='1.0' encoding='utf-8'?>\n<mainbody><header>"
            f"<usercode>{e(usercode)}</usercode><password>{e(password)}</password><msgheader>{e(msgheader)}</msgheader>{ak}"
            f"</header><body><msg>{e(metin)}</msg><no>{e(telefon_10)}</no></body></mainbody>")


def _etiket(metin, ad):
    m = re.search(rf"<{ad}>(.*?)</{ad}>", metin, re.S)
    return m.group(1).strip() if m else None


def otp_yaniti(metin):
    """XML (<code>, <jobID>) ya da düz metin ("00 123456") → (kod, iş no); anlaşılmazsa ("PARSE", "")."""
    metin = (metin or "").strip()
    kod = _etiket(metin, "code")
    if kod is not None:
        is_no = _etiket(metin, "jobID") or ""
    else:
        p = metin.split()
        if not p or not p[0].isascii() or not p[0].isdigit():
            return "PARSE", ""
        kod, is_no = p[0], (p[1] if len(p) > 1 else "")
    if not _YANIT_KODU.fullmatch(kod):
        return "PARSE", ""
    return kod, is_no if _IS_NO.fullmatch(is_no) else ""


def otp_gonder(ayar, telefon_10, metin, zaman_asimi=ISLEM_SURE, ac=None):
    """Dönen (sonuc, kod, is): sonuc gonderildi | mock | hata; kod NetGSM kodu ya da AYAR | AG | PARSE.
    Zaman aşımı işlem başına (aşılırsa AG). WC_TEST ortamında sahte `ac` verilmediyse hata fırlatır (gerçek istek hiç gitmez)."""
    if ac is None:
        if os.environ.get("WC_TEST"):
            raise RuntimeError("WC_TEST: gerçek NetGSM isteği engellendi")
        ac = urllib.request.urlopen
    if not ayar:
        return "hata", "AYAR", ""
    if ayar.get("mock"):
        return "mock", "0", ""
    if not all(ayar.get(k) for k in ("usercode", "password", "msgheader")):
        return "hata", "AYAR", ""
    govde = otp_xml(ayar["usercode"], ayar["password"], ayar["msgheader"], ayar.get("appkey") or "", metin,
                    telefon_10).encode("utf-8")
    istek = urllib.request.Request(OTP_URL, data=govde, method="POST",
                                   headers={"Content-Type": "application/xml; charset=UTF-8"})
    try:
        durum, govde = common.saglayici_istegi(ac, istek, zaman_asimi, 4096)
    except Exception:  # ağ, zaman aşımı, TLS
        return "hata", "AG", ""
    if durum >= 400:  # HTTP hatası
        return "hata", "AG", ""
    kod, is_no = otp_yaniti(govde.decode("utf-8", "replace"))
    if kod in BASARI:
        return "gonderildi", kod, is_no
    return "hata", kod, ""
