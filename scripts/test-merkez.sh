#!/usr/bin/env bash
# Merkez (Rust) testlerini cihazda çalıştırır: optimizasyonsuz, 2 iş, bellek sınırlı (scripts/test.sh kalıbı).
# Yalnızca /root/rza-merkez altında çalışır. Kullanım: scripts/test-merkez.sh [ssh-hedefi] [test-süzgeci]
# Süzgeç "@release": testler yerine `cargo build --release` (uyarısız derleme denetimi; eski ikili cihazdaki target'ta kalır).
# Süzgeç "@release" uyarı bulursa betik sıfırdan farklı çıkar.
# WFC_ORNEK_AGAC=<yerel dizin>: denetle::ornek_agac testinin ürettiği örnek dosya ağacı testten sonra bu yerel dizine kopyalanır
# (scripts/sunucu/parite/geri_oku.py Python ile okur). Dizin yoksa oluşturulur; varsa ve boş değilse betik hiçbir şey silmeden durur.
set -euo pipefail
host=${1:-wificorrect}
filter=${2:-}
cd "$(dirname "$0")/.."
uzak_ortam=""
if [ -n "${WFC_ORNEK_AGAC:-}" ]; then
  if [ -e "$WFC_ORNEK_AGAC" ] && { [ ! -d "$WFC_ORNEK_AGAC" ] || [ -n "$(ls -A "$WFC_ORNEK_AGAC")" ]; }; then
    echo "WFC_ORNEK_AGAC=$WFC_ORNEK_AGAC var ve bos degil; hicbir sey silinmedi. Bos ya da olmayan bir dizin verin." >&2
    exit 2
  fi
  mkdir -p "$WFC_ORNEK_AGAC"
fi
if [ -n "${WFC_ORNEK_AGAC:-}" ]; then uzak_ortam="rm -rf /root/rza-merkez/ornek-agac; export WFC_ORNEK_AGAC=/root/rza-merkez/ornek-agac"; fi
dosyalar="merkez/Cargo.toml merkez/src .cargo src/sablon/logo scripts/sunucu/merkez/Caddyfile scripts/sunucu/merkez/fail2ban-wc-merkez.conf scripts/sunucu/merkez/fail2ban-wc-merkez-filtre.conf scripts/sunucu/merkez/kur.sh scripts/sunucu/merkez/wc-* scripts/sunucu/merkez-kur.sh scripts/sunucu/merkez-geri-don.sh scripts/sunucu/wificorrect-sunucu"
for f in merkez/Cargo.lock merkez/tests; do [ -e "$f" ] && dosyalar="$dosyalar $f"; done
tar -cf - $dosyalar | ssh "$host" "set -e; mkdir -p /root/rza-merkez; cd /root/rza-merkez; rm -rf merkez/src merkez/tests; tar -xf - -C /root/rza-merkez
cd merkez; set +e
$uzak_ortam
if [ '$filter' = '@release' ]; then komut='cargo build --release -j 2'; else komut='cargo test -j 2 $filter'; fi
systemd-run --scope -q -p MemoryMax=1200M nice -n 10 timeout 1500 \$komut > /root/rza-merkez/son.log 2>&1
k=\$?
if [ '$filter' = '@release' ] && grep -aq '^warning' /root/rza-merkez/son.log; then echo 'RELEASE DERLEMESINDE UYARI VAR' >&2; k=1; fi
grep -aE '^(error|warning)|-->|FAILED|test result|panicked' -A4 /root/rza-merkez/son.log | head -80
exit \$k" && durum=0 || durum=$?
if [ -n "${WFC_ORNEK_AGAC:-}" ]; then
  # süzgeç ornek_agac'ı koşturmadıysa cihazda ağaç yoktur: betik düşmesin
  if ssh "$host" "test -d /root/rza-merkez/ornek-agac" 2>/dev/null; then
    ssh "$host" "tar -cf - -C /root/rza-merkez/ornek-agac ." | tar -xf - -C "$WFC_ORNEK_AGAC" || echo "örnek ağaç kopyalanamadı" >&2
  else
    echo "not: cihazda örnek ağaç oluşmadı (süzgeç denetle::ornek_agac'ı koşturmadı); kopyalanmadı" >&2
  fi
fi
# Cargo.lock cihazda üretilir; repoda tutulur
ssh "$host" "cat /root/rza-merkez/merkez/Cargo.lock" > merkez/Cargo.lock.yeni 2>/dev/null || true
if [ -s merkez/Cargo.lock.yeni ]; then mv merkez/Cargo.lock.yeni merkez/Cargo.lock; else rm -f merkez/Cargo.lock.yeni; fi
exit $durum
