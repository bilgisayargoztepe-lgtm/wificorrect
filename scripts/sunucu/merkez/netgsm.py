"""NetGSM genel SMS ucu (XML, 1:n): merkezden hizmet sağlayıcıya uyarı SMS'i. OTP ucu kullanılmaz; yeniden deneme yok.
Parola ve istek gövdesi hiçbir yere yazılmaz."""
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
