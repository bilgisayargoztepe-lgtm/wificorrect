"""Twilio Verify (cihazdaki twilio.rs'in merkez karşılığı): kodu Twilio üretir ve doğrular; merkez yalnızca başlatır ve sorar.
API: POST /v2/Services/{verify_sid}/Verifications (To, Channel=sms, Locale) ve /VerificationCheck (To, Code).
Kimlik: API anahtarı (sid + secret) ikisi de girilmişse o, değilse account SID + auth token (Basic). Yeniden deneme yok;
kimlik bilgisi, kod ve istek gövdesi hiçbir yere yazılmaz."""
import base64
import json
import os
import re
import urllib.parse
import urllib.request

import common

BASE = "https://verify.twilio.com/v2/Services"
SURE = 6  # toplam süre bütçesi (sn); cihaz merkezi 15 sn bekler
SID_RE = {"account_sid": re.compile(r"AC[0-9a-fA-F]{32}"), "api_key_sid": re.compile(r"SK[0-9a-fA-F]{32}"),
          "verify_sid": re.compile(r"VA[0-9a-fA-F]{32}")}


def tr_mi(telefon):
    """Türk cep numarası: 90 + 5 ile başlayan 12 hane."""
    return len(telefon) == 12 and telefon.startswith("905")


def kimlik(ayar):
    """(kullanıcı, sır) ya da None (eksik ayar)."""
    if ayar.get("api_key_sid") and ayar.get("api_key_secret"):
        return ayar["api_key_sid"], ayar["api_key_secret"]
    if ayar.get("account_sid") and ayar.get("auth_token"):
        return ayar["account_sid"], ayar["auth_token"]
    return None


def _istek(ayar, yol, alanlar, sure, ac):
    """(HTTP durumu, JSON gövde) ya da AYAR (eksik/bozuk ayar) ya da None (ağ hatası)."""
    if ac is None:
        if os.environ.get("WC_TEST"):
            raise RuntimeError("WC_TEST: gerçek Twilio isteği engellendi")
        ac = urllib.request.urlopen
    k = kimlik(ayar or {})
    if k is None or not SID_RE["verify_sid"].fullmatch(str(ayar.get("verify_sid") or "")):
        return "AYAR"
    yetki = base64.b64encode(f"{k[0]}:{k[1]}".encode("utf-8")).decode("ascii")
    istek = urllib.request.Request(f"{BASE}/{ayar['verify_sid']}/{yol}", data=urllib.parse.urlencode(alanlar).encode("ascii"),
                                   method="POST", headers={"Authorization": "Basic " + yetki,
                                                           "Content-Type": "application/x-www-form-urlencoded"})
    try:
        durum, govde = common.sinirli_istek(ac, istek, sure)
    except Exception:  # ağ, zaman aşımı (toplam bütçe dahil), TLS
        return None
    try:
        j = json.loads(govde)
    except ValueError:
        j = None
    return durum, j if isinstance(j, dict) else {}


def gonder(ayar, telefon_e164, sure=SURE, ac=None):
    """Doğrulamayı başlatır. telefon_e164: + olmadan. Dönen (sonuc, kod): gonderildi/0, mock/0, hata/<Twilio kodu|HTTP|AYAR|AG>."""
    if ayar and ayar.get("mock"):
        return "mock", "0"
    yerel = "tr" if tr_mi(telefon_e164) else "en"
    r = _istek(ayar, "Verifications", {"To": "+" + telefon_e164, "Channel": "sms", "Locale": yerel}, sure, ac)
    if r == "AYAR":
        return "hata", "AYAR"
    if r is None:
        return "hata", "AG"
    durum, j = r
    if durum < 300 and j.get("status") == "pending":
        return "gonderildi", "0"
    kod = j.get("code")
    return "hata", str(kod) if type(kod) is int else str(durum)


def dogrula(ayar, telefon_e164, kod, sure=SURE, ac=None):
    """(onay, kod): onay True/False; Twilio'ya ulaşılamazsa ya da Twilio geçici hata veriyorsa (5xx, 429) (None, "AG"),
    ayar eksik ya da kimlik reddedildiyse (401/403) (None, "AYAR") — ikisinde de misafirin deneme hakkı düşmez.
    Yanlış, süresi dolmuş ya da iptal edilmiş kod (pending/canceled, 404 ve diğer 4xx) → (False, "0")."""
    r = _istek(ayar, "VerificationCheck", {"To": "+" + telefon_e164, "Code": kod}, sure, ac)
    if r == "AYAR":
        return None, "AYAR"
    if r is None:
        return None, "AG"
    durum, j = r
    if durum >= 500 or durum == 429:
        return None, "AG"
    if durum in (401, 403):
        return None, "AYAR"
    return durum < 300 and j.get("status") == "approved", "0"
