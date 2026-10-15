#!/usr/bin/env python3
"""WifiCorrect yönetim merkezi (spec 2026-10-04): tek süreç, Caddy arkasında 127.0.0.1:8081. Alan adına göre:
yonetim.wificorrect.com (hizmet sağlayıcı), panel.wificorrect.com (müşteri), api.wificorrect.com (cihazlar)."""
import os
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.realpath(__file__)))
import api  # noqa: E402
import cihaz  # noqa: E402
import guvenlik  # noqa: E402
import kayitlar  # noqa: E402
import musteri  # noqa: E402
import raporlar  # noqa: E402
import veri  # noqa: E402
import web  # noqa: E402
import yonetim  # noqa: E402

DB = "/var/lib/wificorrect/merkez/merkez.db"
ALANLAR = {"yonetim.wificorrect.com": "yonetim", "panel.wificorrect.com": "musteri", "api.wificorrect.com": "api",
           "cihaz.wificorrect.com": "cihaz"}
GOVDE_SINIRI = 10_000
PARCA = 65536
GUVENLIK_BASLIKLARI = (("X-Content-Type-Options", "nosniff"), ("Referrer-Policy", "no-referrer"), ("X-Frame-Options", "DENY"),
                       ("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:"))


def istemci_ip(basliklar):
    """Yalnızca 127.0.0.1 dinlenir; Caddy gelen X-Forwarded-For'a gerçek istemciyi SONA ekler."""
    return (basliklar.get("X-Forwarded-For") or "").split(",")[-1].strip() or "yerel"


def parcali_yaz(w, icerik, parca=PARCA):
    """Soket zaman aşımı her yazmaya ayrı uygulanır (sendall'da bütün gönderime): paket yavaş hatta kesilmesin."""
    for i in range(0, len(icerik), parca):
        w.write(icerik[i:i + parca])


class Merkez:
    def __init__(self, uygulamalar):
        self.uygulamalar = uygulamalar

    def istek(self, yontem, yol, basliklar, govde, sorgu, ip):
        host = (basliklar.get("Host") or "").split(":")[0].strip().lower()
        app = self.uygulamalar.get(ALANLAR.get(host, ""))
        if app is None:
            return 404, {"Content-Type": "text/plain; charset=utf-8"}, "Bulunamadı".encode("utf-8")
        return app.istek(yontem, yol, basliklar, govde, sorgu, ip)


class Isleyici(BaseHTTPRequestHandler):
    app = None
    server_version = "wificorrect"
    sys_version = ""
    timeout = 20  # yavaş istemci iş parçacığını süresiz tutmasın

    def _cevapla(self, yontem):
        parca = urllib.parse.urlsplit(self.path)
        try:
            uzunluk = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            uzunluk = -1
        if not 0 <= uzunluk <= GOVDE_SINIRI:
            durum, basliklar, icerik = 413, {"Content-Type": "text/plain; charset=utf-8"}, "İstek çok büyük".encode()
        else:
            govde = self.rfile.read(uzunluk).decode("utf-8", "replace")
            try:
                durum, basliklar, icerik = self.app.istek(yontem, parca.path, self.headers, govde, parca.query,
                                                          istemci_ip(self.headers))
            except Exception as ex:  # ayrıntı journal'a, kullanıcıya genel mesaj
                web.log(f"HATA {yontem} {parca.path}: {ex!r}")
                durum, basliklar, icerik = 500, {"Content-Type": "text/plain; charset=utf-8"}, "Bir sorun oluştu".encode()
        self.send_response(durum)
        for k, v in list(basliklar.items()) + list(GUVENLIK_BASLIKLARI):
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(icerik)))
        self.end_headers()
        parcali_yaz(self.wfile, icerik)

    def do_GET(self):
        self._cevapla("GET")

    def do_POST(self):
        self._cevapla("POST")

    def log_message(self, *args):
        pass


def main():
    os.umask(0o077)
    v, k = veri.Veri(DB), kayitlar.Kayitlar()
    try:
        v.rapor_gruplari_yaz(veri.RAPOR_GRUPLARI)  # raporlar için müşteri ↔ eski arşiv bağı (root okur)
    except OSError as ex:
        web.log(f"rapor-gruplari yazilamadi: {ex!r}")
    gecis, rp = guvenlik.GecisBelirtecleri(), raporlar.Raporlar()
    m = musteri.Musteri(v, k, gecis=gecis, raporlar=rp)
    Isleyici.app = Merkez({"yonetim": yonetim.Yonetim(v, k, gecis=gecis, musteri_app=m, rapor_gruplari=veri.RAPOR_GRUPLARI,
                                                      raporlar=rp), "musteri": m, "api": api.Api(v, raporlar=rp),
                           "cihaz": cihaz.Cihaz(v, gecis, m)})
    srv = ThreadingHTTPServer(("127.0.0.1", 8081), Isleyici)
    srv.daemon_threads = True
    srv.serve_forever()


if __name__ == "__main__":
    main()
