#!/usr/bin/env python3
"""wc-listeler (wcpanel, günde bir; spec 2026-10-14 hazır engel listeleri): açık kaynaklı listeleri indirir, kategori başına
tek, temizlenmiş alan adı listesi üretir. Cihaz sağlık raporu cevabında sürümleri (sha256) görür, değişeni /api/liste'den alır.

Çıktı (DIZIN): <kategori>.txt.gz (sıralı, satır başına bir alan adı, gzip mtime=0 → aynı içerik aynı dosya) ve
<kategori>.json (meta). sha256 gzip'in değil düz metnin özetidir (cihaz açıp doğrular). İsteğe bağlı elle ek:
<kategori>.ek.txt (satır başına alan adı; ör. Türkiye'ye özel bahis siteleri). Kaynak başına son başarılı temiz kopya
kaynak/<kategori>-<url özeti>.txt.gz: bir kaynak inmezse ya da boş/HTML dönerse onun dünkü kopyası kullanılır (liste aniden
küçülmez); hiçbir kaynak bugün inmediyse eski çıktı ve meta korunur, meta'ya `hata` yazılır.

Engelleme dnsmasq'ta alan adı + bütün alt adlarıdır; bu yüzden üst adı listede olan alt adlar atılır (sayı küçülür)."""
import gzip
import hashlib
import io
import json
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import common  # noqa: E402
import surumler  # noqa: E402

DIZIN = "/var/lib/wificorrect/merkez/listeler"
EN_COK = 300_000  # kategori başına; fazlası kesilir (önce elle ek, sonra kaynak sırası öncelikli)
KAYNAK_SINIRI = 64 * 1024 * 1024
KATEGORILER = {"kumar": "Kumar ve bahis", "yetiskin": "Yetişkin içerik", "zararli": "Zararlı ve dolandırıcı siteler"}
EK_AD = "Elle eklenenler"

# Kaynaklar (adresler 2026-10-14'te doğrulandı). Lisanslar:
# - HaGeZi DNS Blocklists (github.com/hagezi/dns-blocklists): GPL-3.0. "wildcard/*-onlydomains" biçimi: satır başına alan
#   adı, alt adları kapsar (bizim engelleme anlamımızla aynı). "doh-vpn-proxy-bypass" listesi BİLEREK kullanılmaz
#   (iş VPN'lerini kırar; kullanıcı kararı 2026-10-14). gambling.medium ~205 bin, nsfw ~91 bin, tif.mini ~236 bin girdi.
# - StevenBlack/hosts "porn" eki (github.com/StevenBlack/hosts): MIT; eki oluşturan kaynakların kendi lisansları var
#   (ör. Clefspeare13/pornhosts MIT, bigdargon/hostsVN). Yetişkin listesinde HaGeZi'ye ek/yedek.
# - USOM zararlı bağlantılar (T.C. Ulusal Siber Olaylara Müdahale Merkezi): kamuya açık liste. 2026-10-14'te bu adres
#   siberguvenlik.gov.tr'ye yönleniyor; HTML dönerse kaynak hatalı sayılır ve son başarılı kopyası kullanılır. Güncel düz
#   metin adresi bulunursa yalnızca burası değişir.
KAYNAKLAR = {
    "kumar": [
        ("HaGeZi Gambling (medium)",
         "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/gambling.medium-onlydomains.txt", "duz"),
    ],
    "yetiskin": [
        ("HaGeZi NSFW", "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/nsfw-onlydomains.txt", "duz"),
        ("StevenBlack porn", "https://raw.githubusercontent.com/StevenBlack/hosts/master/alternates/porn-only/hosts", "hosts"),
    ],
    "zararli": [
        ("USOM zararlı bağlantılar", "https://www.usom.gov.tr/url-list.txt", "usom"),
        ("HaGeZi Threat Intelligence (mini)",
         "https://raw.githubusercontent.com/hagezi/dns-blocklists/main/wildcard/tif.mini-onlydomains.txt", "duz"),
    ],
}

# Asla engellenmez (ve bütün alt adları): sistemin, giriş akışının, telefonların bağlantı denetiminin ve kamu/eğitim
# sitelerinin ihtiyacı. Bir kaynak listelese de çıkar.
ASLA = frozenset({
    "wificorrect.com", "netgsm.com.tr", "twilio.com", "google.com", "googleapis.com", "gstatic.com", "android.com",
    "apple.com", "icloud.com", "microsoft.com", "windowsupdate.com", "msftconnecttest.com", "msftncsi.com",
    "gov.tr", "edu.tr",
})
# Kamu son ekleri: tek başına engellenirse bütün bir alan kapanır.
KAMU_SON_EK = frozenset({
    "com", "net", "org", "com.tr", "gov.tr", "edu.tr", "bel.tr", "k12.tr", "org.tr", "net.tr", "gen.tr", "av.tr",
    "tv.tr", "info.tr", "biz.tr", "web.tr", "name.tr", "tel.tr", "pol.tr", "tsk.tr", "dr.tr", "bbs.tr", "nc.tr", "kep.tr",
})
# VPN/proxy/DoH: VPN kategorisi yok ve iş VPN'leri çalışmalı; tehdit listelerinde geçse de hiçbir listeden gelmez.
# Basit yöntem: bilinen sağlayıcıların alan adları (ve alt adları) + ilk etiketi tam olarak vpn/doh/dot/dns/proxy
# (ör. vpn.sirket.com.tr, doh.ornek.org). "phishing-vpnlogin.com" gibi adlar kalır.
VPN_DOH = frozenset({
    "nordvpn.com", "nordcdn.com", "expressvpn.com", "protonvpn.com", "proton.me", "surfshark.com", "cyberghostvpn.com",
    "privateinternetaccess.com", "mullvad.net", "windscribe.com", "tunnelbear.com", "hotspotshield.com", "ipvanish.com",
    "vyprvpn.com", "purevpn.com", "hide.me", "psiphon.ca", "openvpn.net", "wireguard.com", "tailscale.com", "zerotier.com",
    "cloudflare-dns.com", "cloudflareclient.com", "one.one.one.one", "dns.google", "quad9.net", "nextdns.io",
    "adguard-dns.com", "adguard-dns.io", "opendns.com", "dns.sb", "controld.com", "mullvad-dns.net",
})
VPN_ETIKET = frozenset({"vpn", "doh", "dot", "dns", "proxy"})
ATLA = frozenset({"localhost.localdomain", "ip6-localhost", "ip6-loopback", "local", "broadcasthost"})

_ETIKET = re.compile(r"[a-z0-9-]{1,63}")
_ADBLOCK = re.compile(r"\|\|([^\^/$|]+)\^(\$.*)?")
_SHA = re.compile(r"[0-9a-f]{64}")


def _ust_mu(ad, kume):
    """ad kümedeki bir adın kendisi ya da alt adı mı."""
    p = ad.split(".")
    return any(".".join(p[i:]) in kume for i in range(len(p)))


def gecerli(ad):
    """Normalize edilmiş, engellenebilir alan adı ya da None."""
    ad = ad.strip().lower().rstrip(".")
    if not ad or len(ad) > 253 or ad in ATLA:
        return None
    p = ad.split(".")
    if len(p) < 2 or not all(_ETIKET.fullmatch(x) for x in p):
        return None
    if all(x.isdigit() for x in p):  # IP adresi
        return None
    if ad in KAMU_SON_EK or _ust_mu(ad, ASLA) or _ust_mu(ad, VPN_DOH) or p[0] in VPN_ETIKET:
        return None
    return ad


def _usom_ad(s):
    if "://" in s:
        s = s.split("://", 1)[1]
    for ayrac in "/?#":
        s = s.split(ayrac, 1)[0]
    s = s.rsplit("@", 1)[-1]
    if ":" in s:
        s = s.split(":", 1)[0]
    return s


def _adaylar(satir, bicim):
    s = satir.split("#", 1)[0].strip() if bicim != "adblock" else satir.strip()
    if not s or s.startswith("!"):
        return []
    if bicim == "hosts":
        return s.split()[1:]
    if bicim == "duz":
        return [s.split()[0]]
    if bicim == "adblock":
        m = _ADBLOCK.fullmatch(s)
        return [m.group(1)] if m else []
    if bicim == "usom":
        return [_usom_ad(s.split()[0])]
    raise ValueError(f"bilinmeyen biçim: {bicim}")


def temizle(satirlar, bicim):
    """Kaynak satırlarından geçerli, engellenebilir alan adı kümesi."""
    sonuc = set()
    for satir in satirlar:
        for a in _adaylar(satir, bicim):
            ad = gecerli(a)
            if ad:
                sonuc.add(ad)
    return sonuc


def liste_yolu(dizin, kategori):
    if kategori not in KATEGORILER:
        raise ValueError("Bilinmeyen kategori.")
    return os.path.join(dizin, kategori + ".txt.gz")


def _meta_yolu(dizin, kategori):
    return os.path.join(dizin, kategori + ".json")


def _gz(metin):
    b = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=b, mtime=0) as g:
        g.write(metin.encode("utf-8"))
    return b.getvalue()


def _atomik(yol, icerik):
    os.makedirs(os.path.dirname(yol), exist_ok=True)
    gecici = yol + ".tmp"
    with open(gecici, "wb") as f:
        f.write(icerik)
    os.replace(gecici, yol)


def meta_oku(dizin, kategori):
    try:
        with open(_meta_yolu(dizin, kategori), encoding="utf-8") as f:
            m = json.load(f)
        return m if isinstance(m, dict) else None
    except (OSError, ValueError):
        return None


def ozet(dizin):
    """Sağlık cevabı için {kategori: {sha256, sayi, zaman}}; meta okunamayan ya da hiç liste üretilmemiş kategori yok."""
    o = {}
    for k in KATEGORILER:
        m = meta_oku(dizin, k)
        if (m and isinstance(m.get("sha256"), str) and _SHA.fullmatch(m["sha256"]) and type(m.get("sayi")) is int
                and isinstance(m.get("zaman"), str)):
            o[k] = {"sha256": m["sha256"], "sayi": m["sayi"], "zaman": m["zaman"]}
    return o


def _onbellek_yolu(dizin, kategori, url):
    return os.path.join(dizin, "kaynak", f"{kategori}-{hashlib.sha256(url.encode()).hexdigest()[:12]}.txt.gz")


def _onbellek_oku(yol):
    try:
        with gzip.open(yol, "rb") as f:
            return set(f.read().decode("utf-8").split())
    except (OSError, EOFError, ValueError):
        return None


def _kaynak(kategori, ad, url, bicim, getir, dizin):
    """(alan adları, durum satırı, bugün indi mi)."""
    try:
        ham = getir(url, KAYNAK_SINIRI).decode("utf-8", "replace")
        if ham.lstrip()[:1] == "<":
            raise ValueError("liste yerine HTML sayfası döndü")
        alanlar = temizle(ham.splitlines(), bicim)
        if not alanlar:
            raise ValueError("liste boş")
    except Exception as h:
        eski = _onbellek_oku(_onbellek_yolu(dizin, kategori, url))
        durum = {"ad": ad, "sayi": len(eski or ()), "hata": (str(h) or type(h).__name__)[:200]}
        if eski:
            durum["eski"] = True
        return eski or set(), durum, False
    _atomik(_onbellek_yolu(dizin, kategori, url), _gz("\n".join(sorted(alanlar)) + "\n"))
    return alanlar, {"ad": ad, "sayi": len(alanlar)}, True


def _ek(dizin, kategori):
    try:
        with open(os.path.join(dizin, kategori + ".ek.txt"), encoding="utf-8") as f:
            return temizle(f.read().splitlines(), "duz")
    except OSError:
        return None


def uret(kategori, getir, dizin, simdi):
    """Kategorinin listesini üretir; meta döner (ayrıca <kategori>.json)."""
    os.makedirs(dizin, exist_ok=True)
    kumeler, durumlar, inen = [], [], 0
    ek = _ek(dizin, kategori)
    for ad, url, bicim in KAYNAKLAR.get(kategori, []):
        alanlar, durum, indi = _kaynak(kategori, ad, url, bicim, getir, dizin)
        kumeler.append(alanlar)
        durumlar.append(durum)
        inen += indi
    if ek is not None:
        kumeler.insert(0, ek)
        durumlar.append({"ad": EK_AD, "sayi": len(ek)})
    hepsi = set().union(*kumeler) if kumeler else set()
    if inen == 0 or not hepsi:  # hiçbir kaynak bugün inmedi: eski liste ve sürümü kalır
        meta = meta_oku(dizin, kategori) or {"kategori": kategori}
        meta.update(hata="Hiçbir kaynak indirilemedi; önceki liste kullanılıyor.", hata_zaman=common.now_iso(simdi),
                    kaynaklar=durumlar)
        _atomik(_meta_yolu(dizin, kategori), json.dumps(meta, ensure_ascii=False).encode("utf-8"))
        return meta
    # üst adı listede olan alt ad gereksiz (dnsmasq alt adları da engeller)
    sade = {a for a in hepsi if not any(".".join(a.split(".")[i:]) in hepsi for i in range(1, a.count(".")))}
    kesildi = len(sade) > EN_COK
    if kesildi:
        secilen = set()
        for k in kumeler:
            for a in sorted(k & sade):
                if len(secilen) >= EN_COK:
                    break
                secilen.add(a)
        sade = secilen
    metin = "".join(a + "\n" for a in sorted(sade))
    _atomik(liste_yolu(dizin, kategori), _gz(metin))
    meta = {"kategori": kategori, "sha256": hashlib.sha256(metin.encode("utf-8")).hexdigest(), "sayi": len(sade),
            "zaman": common.now_iso(simdi), "kesildi": kesildi, "kaynaklar": durumlar}
    _atomik(_meta_yolu(dizin, kategori), json.dumps(meta, ensure_ascii=False).encode("utf-8"))
    return meta


class _Kilit:
    """Zamanlayıcı ve yönetimdeki "Şimdi güncelle" aynı anda yazmasın (Linux'ta flock; testte Windows'ta yok sayılır)."""
    def __init__(self, dizin):
        self.yol, self.f = os.path.join(dizin, ".kilit"), None

    def __enter__(self):
        os.makedirs(os.path.dirname(self.yol), exist_ok=True)
        self.f = open(self.yol, "a")
        try:
            import fcntl
            fcntl.flock(self.f, fcntl.LOCK_EX)
        except ImportError:
            pass
        return self

    def __exit__(self, *a):
        self.f.close()


def calistir(veri, getir, dizin, simdi, kim="listeler", ip=""):
    """Üç kategoriyi üretir, hareket yazar. Dönen: 0 hepsi temiz, 1 bir kaynak ya da kategori hatalı."""
    sonuc = 0
    with _Kilit(dizin):
        for k in KATEGORILER:
            onceki = (meta_oku(dizin, k) or {}).get("sha256")
            try:
                meta = uret(k, getir, dizin, simdi)
            except Exception as h:  # bir kategorinin hatası diğerlerini durdurmaz
                veri.hareket(kim, ip, "LISTE_HATA", None, f"kategori={k} neden={type(h).__name__}: {h}")
                sonuc = 1
                continue
            if meta.get("hata"):
                neden = "; ".join(f"{d['ad']}: {d['hata']}" for d in meta["kaynaklar"] if d.get("hata"))
                veri.hareket(kim, ip, "LISTE_HATA", None, f"kategori={k} neden={neden or meta['hata']}")
                sonuc = 1
                continue
            for d in meta["kaynaklar"]:
                if d.get("hata"):
                    veri.hareket(kim, ip, "LISTE_HATA", None, f"kategori={k} kaynak={d['ad']} neden={d['hata']}")
                    sonuc = 1
            if meta["sha256"] != onceki:
                veri.hareket(kim, ip, "LISTE_GUNCEL", None, f"kategori={k} surum={meta['sha256'][:8]} sayi={meta['sayi']}"
                             + (" kesildi" if meta["kesildi"] else ""))
    return sonuc


def main():
    os.umask(0o077)
    import veri as veri_modulu
    from merkez import DB
    return calistir(veri_modulu.Veri(DB), surumler.getir, DIZIN, time.time())


if __name__ == "__main__":
    sys.exit(main())
