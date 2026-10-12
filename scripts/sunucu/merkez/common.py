"""BOCAFE hotspot ortak yardımcıları (spec §11.4, §14, §17)."""
import contextlib
import csv
import datetime
import functools
import gzip
import ipaddress
import json
import os
import re
import subprocess
import time

TZ = datetime.timezone(datetime.timedelta(hours=3))  # Türkiye: sabit UTC+3, yaz saati yok
MAC_RE = re.compile(r"[0-9a-f]{2}(?::[0-9a-f]{2}){5}")
_NAME_EXTRA = set(" -'.")


def now_iso(ts=None):
    return datetime.datetime.fromtimestamp(time.time() if ts is None else ts, TZ).isoformat(timespec="seconds")


def day_of(iso):
    return iso[:10]


def parse_time(s):
    """'2026-09-24 14:30' veya ISO 8601 → epoch. Saat dilimi yoksa +03:00 kabul edilir."""
    dt = datetime.datetime.fromisoformat(s.strip().replace(" ", "T", 1))
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=TZ)
    return dt.timestamp()


_TARIH_RE = re.compile(r"(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?", re.ASCII)
_GIRDI_RE = re.compile(r"(\d{1,2})\.(\d{1,2})\.(\d{4})(?: +(.*))?", re.ASCII)


def tarih_goster(iso, saniye=False):
    """Ekranda gösterilen tarih (gün.ay.yıl): '2026-10-12' → '12.10.2026'; '2026-10-12T17:38:05+03:00' ya da
    '2026-10-12 17:38' → '12.10.2026 17:38' (saniye=True: '12.10.2026 17:38:05'). Çözülemeyen değer aynen (str; None → '').
    Saklanan veri, CSV, adresler ve JSON ISO kalır; yalnızca gösterimde kullanılır (cihazdaki ortak::tarih_goster ile aynı)."""
    if iso is None:
        return ""
    s = str(iso).strip()
    m = _TARIH_RE.match(s)
    if not m or (m.group(4) is None and len(s) != 10):
        return str(iso)
    y, a, g, sa, dk, sn = m.groups()
    gun = f"{g}.{a}.{y}"
    if sa is None:
        return gun
    return f"{gun} {sa}:{dk}:{sn}" if saniye and sn is not None else f"{gun} {sa}:{dk}"


def tarih_saat_goster(iso):
    return tarih_goster(iso, saniye=True)


def tarih_girdi(s):
    """Kişinin yazdığı tarih: '12.10.2026', '1.9.2026', '12.10.2026 17:38' → ISO '2026-10-12[ 17:38]'.
    Başka her şey (eski ISO yazımı dahil) kırpılıp aynen döner; geçerliliği çağıran denetler."""
    s = str(s or "").strip()
    m = _GIRDI_RE.fullmatch(s)
    if not m:
        return s
    g, a, y, saat = m.groups()
    iso = f"{y}-{int(a):02d}-{int(g):02d}"
    return f"{iso} {saat.strip()}" if saat else iso


def normalize_phone(raw):
    """Türkiye GSM numarasını 10 haneye (5XXXXXXXXX) çevirir; geçersizse None."""
    d = re.sub(r"\D", "", raw or "")
    if len(d) == 12 and d.startswith("90"):
        d = d[2:]
    elif len(d) == 11 and d.startswith("0"):
        d = d[1:]
    return d if re.fullmatch(r"5\d{9}", d) else None


def clean_name(raw):
    s = " ".join((raw or "").split())
    if not 2 <= len(s) <= 40 or not s[0].isalpha():
        return None
    return s if all(c.isalpha() or c in _NAME_EXTRA for c in s) else None


def norm_mac(s):
    s = (s or "").strip().lower().replace("-", ":")
    return s if MAC_RE.fullmatch(s) else None


@functools.lru_cache(maxsize=65536)  # logger her olayda çağırır; IP'ler tekrar eder
def in_subnet(ip, subnet):
    try:
        return ipaddress.IPv4Address(ip) in ipaddress.IPv4Network(subnet)
    except ValueError:
        return False


_CELL_CLEAN = str.maketrans({"\r": " ", "\n": " ", ";": " ", '"': None})


def csv_cell(v):
    """Ayraç/satır kırıcıları temizler, formül enjeksiyonunu (=,+,-,@,TAB) etkisizleştirir. Sıcak yol: boş hücre."""
    if v is None or v == "":
        return ""
    s = (v if isinstance(v, str) else str(v)).translate(_CELL_CLEAN)
    return "'" + s if s[:1] in ("=", "+", "-", "@", "\t") else s


CSV_HEADER = [
    "zaman", "olay", "telefon", "ad", "soyad", "mac", "ic_ip", "protokol", "ic_port",
    "hedef_ip", "hedef_port", "nat_ip", "nat_port", "alan_adi", "gonderilen_bayt",
    "alinan_bayt", "sure_sn", "oturum_id", "ek",
]
INDEX_HEADER = ["telefon", "ad", "soyad", "ilk_kayit", "son_oturum"]
BOM = chr(0xFEFF)  # UTF-8 BOM: Türkçe Excel dosyayı doğru kodlamayla açsın


# --- yapılandırma (UCI) ---
def parse_uci_show(text):
    """`uci show hotspot` çıktısı → {bolum: {anahtar: deger, ".type": tip}}."""
    out = {}
    for line in text.splitlines():
        key, sep, val = line.partition("=")
        if not sep:
            continue
        parts = key.split(".", 2)
        if len(parts) < 2:
            continue
        sec = out.setdefault(parts[1], {})
        if len(parts) == 2:
            sec[".type"] = val
            continue
        if len(val) >= 2 and val[0] == val[-1] == "'":
            val = val[1:-1].replace("'\\''", "'")
        sec[parts[2]] = val
    return out


class Config:
    def __init__(self, sections):
        self.sections = sections

    def get(self, sec, key, default=""):
        return self.sections.get(sec, {}).get(key, default)

    def num(self, sec, key, default):
        try:
            return int(self.get(sec, key, default))
        except (TypeError, ValueError):
            return default

    def of_type(self, kind):
        """[(bolum_adi, bolum)] — ör. anonim bölümler için ('@ban[0]', {...})."""
        return [(name, s) for name, s in self.sections.items() if s.get(".type") == kind]

    def allow(self):
        return {norm_mac(s.get("mac")): s.get("name", "") for _, s in self.of_type("allow") if norm_mac(s.get("mac"))}

    def bans(self):
        return {norm_mac(s.get("mac")): s.get("not", "") for _, s in self.of_type("ban") if norm_mac(s.get("mac"))}

    @property
    def log_root(self):
        return self.get("main", "log_root", "/srv/5651")

    @property
    def state_root(self):
        return self.get("main", "state_root", "/srv/hotspot/state")


def load_config(text=None):
    if text is None:
        text = subprocess.run(["uci", "-q", "show", "hotspot"], capture_output=True, text=True).stdout
    return Config(parse_uci_show(text))


# --- CSV ---
def make_row(olay, zaman=None, **fields):
    unknown = set(fields) - set(CSV_HEADER)
    if unknown:
        raise KeyError(sorted(unknown))
    row = dict.fromkeys(CSV_HEADER, "")
    row.update(fields)
    row["olay"] = olay
    row["zaman"] = zaman or now_iso()
    return row


def lock_file(f):
    try:
        import fcntl
    except ImportError:  # Windows'taki birim testleri: kilitsiz
        return
    fcntl.flock(f.fileno(), fcntl.LOCK_EX)


def append_rows(path, rows):
    """Kilitli append. Dosya yoksa 600 izinle, BOM + başlıkla oluşturur (§14.6)."""
    if not rows:
        return
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    with open(fd, "a", encoding="utf-8", newline="") as f:
        lock_file(f)
        w = csv.writer(f, delimiter=";")
        if os.fstat(fd).st_size == 0:
            f.write(BOM)
            w.writerow(CSV_HEADER)
        for r in rows:
            w.writerow([csv_cell(r.get(k, "")) for k in CSV_HEADER])


def read_rows(path):
    opener = gzip.open if path.endswith(".gz") else open
    try:
        with opener(path, "rt", encoding="utf-8-sig", newline="") as f:
            yield from csv.DictReader(f, delimiter=";")
    except FileNotFoundError:
        return


def day_open(root, day, now):
    """Güne hâlâ satır eklenebilir mi: bugün, ya da klasörü duran ve henüz mühürlenmemiş geçmiş gün."""
    d = os.path.join(root, "gunluk", day)
    if day >= day_of(now_iso(now)):
        return True
    return os.path.isdir(d) and not os.path.exists(os.path.join(d, "MANIFEST.sha256"))


def storage_ready(root):
    """Log kökü /srv altındaysa /srv bağlı olmalı; değilse yazılanlar kök dosya sistemine düşer (spec §13.3)."""
    return not root.startswith("/srv/") or os.path.ismount("/srv")


def day_file(root, day, name):
    return os.path.join(root, "gunluk", day, name)


def user_file(root, day, phone):
    """Kişinin o günkü dosyası; günün tüm trafiği tek klasörde olsun diye günlük klasörün içinde (§7.2)."""
    return os.path.join(root, "gunluk", day, "kullanicilar", phone + ".csv")


def append_user_rows(root, sess, mac, rows):
    """Satırları, zamanlarının gününe göre kişinin günlük dosyasına yazar.
    Dosya o gün ilk kez açılıyorsa başa KAYIT satırı (ad, soyad, MAC) koyar; her günlük dosya kendi başına okunabilir."""
    by_day = {}
    for r in rows:
        by_day.setdefault(day_of(r["zaman"]), []).append(r)
    for day, day_rows in by_day.items():
        path = user_file(root, day, sess["phone"])
        if not os.path.exists(path):  # ponytail: iki süreç aynı anda açarsa KAYIT iki kez yazılabilir; zararsız
            day_rows = [make_row("KAYIT", zaman=day_rows[0]["zaman"], telefon=sess["phone"], ad=sess.get("ad", ""),
                                 soyad=sess.get("soyad", ""), mac=mac, ic_ip=sess.get("ip", ""),
                                 oturum_id=sess.get("session_id", ""))] + day_rows
        append_rows(path, day_rows)


# --- kullanıcı index'i (çağıran state_lock tutar) ---
def _index_path(root):
    return os.path.join(root, "kullanicilar", "index.csv")


def read_index(root):
    try:
        with open(_index_path(root), encoding="utf-8-sig", newline="") as f:
            return {r["telefon"]: r for r in csv.DictReader(f, delimiter=";")}
    except FileNotFoundError:
        return {}


def _write_index(root, rows):
    path = _index_path(root)
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="") as f:
        f.write(BOM)
        w = csv.writer(f, delimiter=";")
        w.writerow(INDEX_HEADER)
        for phone in sorted(rows):
            w.writerow([csv_cell(rows[phone].get(k, "")) for k in INDEX_HEADER])
    os.chmod(tmp, 0o600)
    os.replace(tmp, path)


def upsert_index(root, phone, ad, soyad, zaman):
    rows = read_index(root)
    row = rows.get(phone) or {"telefon": phone, "ilk_kayit": zaman}
    row.update(ad=ad, soyad=soyad, son_oturum=zaman)
    rows[phone] = row
    _write_index(root, rows)


def remove_from_index(root, phones):
    rows = read_index(root)
    for phone in phones:
        rows.pop(phone, None)
    _write_index(root, rows)


# --- durum dosyaları ---
@contextlib.contextmanager
def state_lock(state_root):
    os.makedirs(state_root, mode=0o700, exist_ok=True)
    with open(os.path.join(state_root, ".lock"), "a") as f:
        lock_file(f)
        yield


def sessions_path(state_root):
    return os.path.join(state_root, "sessions.json")


def load_sessions(state_root):
    """Eksik, boş veya bozuk dosya → {} (elektrik kesintisinde servisler çökmesin)."""
    try:
        with open(sessions_path(state_root), encoding="utf-8") as f:
            data = json.load(f)
    except (FileNotFoundError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def save_json(path, data):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=1)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def _sms_path(state_root):
    return os.path.join(state_root, "sms_sayac.json")


def sms_count(state_root, day):
    try:
        with open(_sms_path(state_root), encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return 0
    return int(data.get("adet", 0)) if isinstance(data, dict) and data.get("tarih") == day else 0


def sms_count_inc(state_root, day):
    n = sms_count(state_root, day) + 1
    os.makedirs(state_root, mode=0o700, exist_ok=True)
    save_json(_sms_path(state_root), {"tarih": day, "adet": n})
    return n


# --- nft / sistem komutları ---
def _check_mac_ip(mac, ip):
    if norm_mac(mac) != mac:
        raise ValueError(f"geçersiz MAC: {mac!r}")
    ipaddress.IPv4Address(ip)  # geçersizse ValueError


def nft_add_cmd(mac, ip, seconds):
    _check_mac_ip(mac, ip)
    return ["nft", "add", "element", "inet", "hotspot", "auth",
            "{ %s . %s timeout %ds }" % (mac, ip, max(1, int(seconds)))]


def nft_del_cmd(mac, ip):
    _check_mac_ip(mac, ip)
    return ["nft", "delete", "element", "inet", "hotspot", "auth", "{ %s . %s }" % (mac, ip)]


def _mac_set_cmd(verb, set_name, mac):
    if norm_mac(mac) != mac:
        raise ValueError(f"geçersiz MAC: {mac!r}")
    return ["nft", verb, "element", "inet", "hotspot", set_name, "{ %s }" % mac]


def nft_allow_cmd(mac):
    return _mac_set_cmd("add", "allow_mac", mac)


def nft_disallow_cmd(mac):
    return _mac_set_cmd("delete", "allow_mac", mac)


def nft_ban_cmd(mac):
    return _mac_set_cmd("add", "ban_mac", mac)


def nft_unban_cmd(mac):
    return _mac_set_cmd("delete", "ban_mac", mac)


def run(cmd, timeout=15):
    try:
        return subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=timeout).returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


# --- oturum / denetim ---
_KEEP_CONNS = {"sure_doldu", "yeniden_giris", "yukleme_hatasi"}


def close_session(cfg, sessions, mac, reason, now=None, runner=run):
    """Oturumu kapatır ve OTURUM_BITIS yazar. Çağıran state_lock tutar ve sessions'ı kaydeder."""
    s = sessions.pop(mac, None)
    if s is None:
        return None
    now = time.time() if now is None else now
    end = min(now, s["expires_epoch"]) if reason == "sure_doldu" else now
    if s["ip"]:  # IP'siz (beklemedeki) oturumun nft kaydı yok
        runner(nft_del_cmd(mac, s["ip"]))
        if reason not in _KEEP_CONNS:
            runner(["conntrack", "-D", "-s", s["ip"]])
    zaman, ek = now_iso(end), "neden=" + reason
    if not day_open(cfg.log_root, day_of(zaman), now):  # o gün mühürlendi/silindi: bugüne yaz, bitişi ek'te ver
        zaman, ek = now_iso(now), f"{ek} bitis={zaman}"
    row = make_row("OTURUM_BITIS", zaman=zaman, telefon=s["phone"], ad=s["ad"], soyad=s["soyad"],
                   mac=mac, ic_ip=s["ip"], sure_sn=int(end - s["start_epoch"]),
                   oturum_id=s["session_id"], ek=ek)
    append_user_rows(cfg.log_root, s, mac, [row])
    append_rows(day_file(cfg.log_root, day_of(zaman), "oturum.csv"), [row])
    return s


def _ip_row(cfg, s, mac, old_ip, now):
    zaman = now_iso(now)
    row = make_row("OTURUM_IP_DEGISTI", zaman=zaman, telefon=s["phone"], mac=mac, ic_ip=s["ip"],
                   oturum_id=s["session_id"], ek="eski_ip=" + old_ip)
    append_user_rows(cfg.log_root, s, mac, [row])
    append_rows(day_file(cfg.log_root, day_of(zaman), "oturum.csv"), [row])


def park_ip_holders(cfg, sessions, mac, ip, now=None, runner=run):
    """`ip`'yi tutan başka oturum varsa o cihaz artık bu IP'de değildir (DHCP IP'yi başkasına verdi):
    nft kaydı silinir, oturum IP'siz beklemeye alınır — trafik yanlış kişiye yazılmasın. Çağıran state_lock tutar."""
    now = time.time() if now is None else now
    for other, s in sessions.items():
        if other != mac and s["ip"] == ip:
            runner(nft_del_cmd(other, ip))
            s["ip"] = ""
            _ip_row(cfg, s, other, ip, now)


def move_session(cfg, sessions, mac, new_ip, now=None, runner=run):
    """Süren oturumu cihazın yeni IP'sine taşır (SMS istemeden). nft başarısızsa hiçbir şey değişmez → False.
    Çağıran state_lock tutar ve sessions'ı kaydeder."""
    s = sessions[mac]
    now = time.time() if now is None else now
    if not runner(nft_add_cmd(mac, new_ip, s["expires_epoch"] - now)):
        return False
    park_ip_holders(cfg, sessions, mac, new_ip, now, runner)
    old_ip, s["ip"] = s["ip"], new_ip
    if old_ip:
        runner(nft_del_cmd(mac, old_ip))
    _ip_row(cfg, s, mac, old_ip, now)
    return True


def audit(cfg, olay, **fields):
    row = make_row(olay, **fields)
    append_rows(day_file(cfg.log_root, day_of(row["zaman"]), "denetim.csv"), [row])


# --- IP → MAC ---
def read_text(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def parse_leases(text):
    """/tmp/dhcp.leases: 'bitis_epoch mac ip hostname clientid' → {ip: mac}."""
    out = {}
    for line in text.splitlines():
        p = line.split()
        if len(p) >= 3 and norm_mac(p[1]):
            out[p[2]] = norm_mac(p[1])
    return out


def parse_arp(text):
    """/proc/net/arp → {ip: mac}, yalnızca tamamlanmış (0x2) girdiler."""
    out = {}
    for line in text.splitlines()[1:]:
        p = line.split()
        if len(p) >= 4 and p[2] == "0x2" and norm_mac(p[3]):
            out[p[0]] = norm_mac(p[3])
    return out


def lookup_mac(ip, leases_text=None, arp_text=None):
    leases_text = read_text("/tmp/dhcp.leases") if leases_text is None else leases_text
    arp_text = read_text("/proc/net/arp") if arp_text is None else arp_text
    return parse_leases(leases_text).get(ip) or parse_arp(arp_text).get(ip)
