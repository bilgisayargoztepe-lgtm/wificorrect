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
TABAN_ESIK, TABAN_ORAN = 1000, 0.5  # küçük liste tabanı (bkz. _kaynak)

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
    "connectivitycheck.gstatic.com", "captive.apple.com", "detectportal.firefox.com", "pool.ntp.org", "ntp.org",
    "gov.tr", "edu.tr",
    # büyük ortak altyapılar: bir alt adı engellemek bile milyonlarca meşru kullanıcıyı ve sayfayı bozar
    "googleusercontent.com", "azureedge.net", "windows.net", "cloudflare.com", "trycloudflare.com", "facebook.com",
    "fbcdn.net", "instagram.com", "youtube.com", "ytimg.com", "akamaihd.net", "live.com", "office.com", "outlook.com",
    "whatsapp.com", "whatsapp.net", "wa.me",
    # telefon üreticilerinin hizmetleri, sosyal medya, alışveriş, ödeme ve bankalar: yanlış pozitif misafiri doğrudan
    # etkiler (giriş, ödeme, telefon güncellemesi); bu sitelerin taklitleri ayrı alan adlarıdır ve engellenebilir.
    # turkiye.gov.tr (e-Devlet) gov.tr altında zaten korunur, açıkça yazıldı.
    "hicloud.com", "miui.com", "xiaomi.com", "oppomobile.com", "samsung.com", "samsungcloud.com", "x.com", "twitter.com",
    "tiktok.com", "amazon.com", "trendyol.com", "hepsiburada.com", "papara.com", "iyzico.com", "garantibbva.com.tr",
    "isbank.com.tr", "ziraatbank.com.tr", "akbank.com", "yapikredi.com.tr", "vakifbank.com.tr", "halkbank.com.tr",
    "qnb.com.tr", "denizbank.com", "enpara.com", "turkiye.gov.tr",
})
# Paylaşımlı platformlar: zararlı listelerde (özellikle URL listelerinde) bir kullanıcının dosyası/sayfası yüzünden bütün
# platform listelenebilir. Bunlar ve alt adları (platformun kendi altyapısı, kullanıcıya ayrılmış değil) engellenmez.
PAYLASIMLI = frozenset({
    "github.com", "githubusercontent.com", "discordapp.com", "discord.com", "discord.gg", "dropbox.com",
    "dropboxusercontent.com", "bit.ly", "t.me", "telegram.org", "ipfs.io", "mediafire.com", "pastebin.com",
    "onedrive.live.com", "1drv.ms", "sharepoint.com", "wetransfer.com", "mega.nz", "tinyurl.com",
})
# Aşırı geniş girdiler — yalnızca KENDİSİ atılır, alt adları listelenebilir: kamu son ekleri (tek başına engellenirse
# bütün bir alan kapanır) ve kullanıcıya alt ad veren barındırma platformları (kotu.github.io listelenebilir, github.io değil).
KAMU_SON_EK = frozenset({
    "com", "net", "org", "com.tr", "gov.tr", "edu.tr", "bel.tr", "k12.tr", "org.tr", "net.tr", "gen.tr", "av.tr",
    "tv.tr", "info.tr", "biz.tr", "web.tr", "name.tr", "tel.tr", "pol.tr", "tsk.tr", "dr.tr", "bbs.tr", "nc.tr", "kep.tr",
    "co.uk", "org.uk", "ac.uk", "gov.uk", "com.br", "com.au", "net.au", "org.au", "co.jp", "ne.jp", "co.in", "co.za",
    "com.cn", "com.mx", "co.kr", "com.ar", "co.nz", "com.sg", "com.ua", "com.pl", "co.id", "com.hk", "com.tw", "eu.org",
    "blogspot.com", "github.io", "gitlab.io", "wordpress.com", "weebly.com", "wixsite.com", "firebaseapp.com", "web.app",
    "pages.dev", "workers.dev", "vercel.app", "netlify.app", "herokuapp.com", "azurewebsites.net", "cloudfront.net",
    "amazonaws.com", "s3.amazonaws.com", "appspot.com", "glitch.me", "repl.co", "ngrok.io", "ngrok-free.app",
    "duckdns.org", "no-ip.com", "ddns.net", "000webhostapp.com", "r2.dev", "onrender.com", "fly.dev", "surge.sh",
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
    if ad in KAMU_SON_EK or _ust_mu(ad, ASLA) or _ust_mu(ad, PAYLASIMLI) or _ust_mu(ad, VPN_DOH) or p[0] in VPN_ETIKET:
        return None
    return ad


def _usom_ad(s):
    """USOM satırından ana bilgisayar adı; yol ya da sorgu içeren URL'ler için None. Bir URL çoğu kez paylaşımlı bir
    sitedeki tek dosyayı gösterir (github.com/x/y, cdn.discordapp.com/attachments/…); bütün siteyi engellemek yanlış olur.
    Yalnızca çıplak ana bilgisayar girdileri (kotu.com, http://kotu.com/) alınır."""
    if "://" in s:
        s = s.split("://", 1)[1]
    i = min([s.find(c) for c in "/?#" if c in s] or [len(s)])
    if s[i:] not in ("", "/"):
        return None
    s = s[:i].rsplit("@", 1)[-1]
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
        ad = _usom_ad(s.split()[0])
        return [ad] if ad else []
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
            ham = f.read().decode("utf-8").split()
    except (OSError, EOFError, ValueError):
        return None
    # eski sürümün yazdığı kopya: bugünkü kurallarla (asla listesi, geçerlilik) yeniden süzülür
    return {a for a in map(gecerli, ham) if a}


def html_mi(ham):
    """Liste yerine bir web sayfası (yönlendirme, hata ya da giriş sayfası) mı döndü."""
    bas = ham.lstrip("\ufeff").lstrip()[:4096].lower()
    return bas[:1] == "<" or "<html" in bas or "<!doctype" in bas


def _kaynak(kategori, ad, url, bicim, getir, dizin):
    """(alan adları, durum satırı, bugün indi mi). Hatada son başarılı kopya kullanılır ve kopyanın üzerine yazılmaz.
    Taban: önceki kopya TABAN_ESIK'ten büyükken yeni liste onun TABAN_ORAN'ından küçükse (kaynak yarım ya da bozuk
    yayımlamış olabilir) hata sayılır. Kaynak gerçekten kalıcı küçüldüyse yönetici kaynak/ altındaki kopyayı silerek yeni
    listeyi kabul ettirir."""
    eski = _onbellek_oku(_onbellek_yolu(dizin, kategori, url))
    try:
        ham = getir(url, KAYNAK_SINIRI).decode("utf-8", "replace").lstrip("\ufeff")
        if html_mi(ham):
            raise ValueError("liste yerine HTML sayfası döndü")
        alanlar = temizle(ham.splitlines(), bicim)
        if not alanlar:
            raise ValueError("liste boş")
        if eski and len(eski) > TABAN_ESIK and len(alanlar) < len(eski) * TABAN_ORAN:
            raise ValueError(f"liste beklenenden çok küçük: {len(alanlar)} / eski {len(eski)}")
    except Exception as h:
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
            if len(secilen) >= EN_COK:
                break
        sade = secilen
    metin = "".join(a + "\n" for a in sorted(sade))
    # Önce liste, sonra meta: arada (ya da meta yazılamazsa) cihaz eski sha ile yeni dosyayı alabilir; cihaz sha'yı
    # doğruladığı için yazmaz, eskisini tutar ve sonraki raporda yeniden dener. Zararsız pencere.
    _atomik(liste_yolu(dizin, kategori), _gz(metin))
    meta = {"kategori": kategori, "sha256": hashlib.sha256(metin.encode("utf-8")).hexdigest(), "sayi": len(sade),
            "zaman": common.now_iso(simdi), "kesildi": kesildi, "kaynaklar": durumlar}
    _atomik(_meta_yolu(dizin, kategori), json.dumps(meta, ensure_ascii=False).encode("utf-8"))
    return meta


class Mesgul(Exception):
    """Başka bir güncelleme sürüyor (bekle=False)."""


class _Kilit:
    """Zamanlayıcı ve yönetimdeki "Şimdi güncelle" aynı anda yazmasın (Linux'ta flock; testte Windows'ta yok sayılır).
    bekle=False: kilit tutuluyorsa beklemeden Mesgul (web isteği asılı kalmasın)."""
    def __init__(self, dizin, bekle=True):
        self.yol, self.bekle, self.f = os.path.join(dizin, ".kilit"), bekle, None

    def __enter__(self):
        os.makedirs(os.path.dirname(self.yol), exist_ok=True)
        self.f = open(self.yol, "a")
        try:
            import fcntl
        except ImportError:
            return self
        try:
            fcntl.flock(self.f, fcntl.LOCK_EX | (0 if self.bekle else fcntl.LOCK_NB))
        except BlockingIOError:
            self.f.close()
            raise Mesgul()
        return self

    def __exit__(self, *a):
        self.f.close()


def calistir(veri, getir, dizin, simdi, kim="listeler", ip="", bekle=True):
    """Üç kategoriyi üretir, hareket yazar. Dönen: 0 hepsi temiz, 1 bir kaynak ya da kategori hatalı.
    bekle=False iken başka güncelleme sürüyorsa Mesgul fırlatır."""
    sonuc = 0
    with _Kilit(dizin, bekle):
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
