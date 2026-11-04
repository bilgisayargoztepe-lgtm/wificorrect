#!/usr/bin/env bash
# Merkez (Rust) testlerini cihazda çalıştırır: optimizasyonsuz, 2 iş, bellek sınırlı (scripts/test.sh kalıbı).
# Yalnızca /root/rza-merkez altında çalışır. Kullanım: scripts/test-merkez.sh [ssh-hedefi] [test-süzgeci]
set -euo pipefail
host=${1:-wificorrect}
filter=${2:-}
cd "$(dirname "$0")/.."
dosyalar="merkez/Cargo.toml merkez/src .cargo src/sablon/logo scripts/sunucu/merkez/fail2ban-wc-merkez.conf scripts/sunucu/merkez/fail2ban-wc-merkez-filtre.conf"
for f in merkez/Cargo.lock merkez/tests; do [ -e "$f" ] && dosyalar="$dosyalar $f"; done
tar -cf - $dosyalar | ssh "$host" "set -e; mkdir -p /root/rza-merkez; cd /root/rza-merkez; rm -rf merkez/src merkez/tests; tar -xf - -C /root/rza-merkez
cd merkez; set +e
systemd-run --scope -q -p MemoryMax=1200M nice -n 10 timeout 1500 cargo test -j 2 $filter > /root/rza-merkez/son.log 2>&1
k=\$?
grep -aE '^(error|warning)|-->|FAILED|test result|panicked' -A4 /root/rza-merkez/son.log | head -80
exit \$k" && durum=0 || durum=$?
# Cargo.lock cihazda üretilir; repoda tutulur
ssh "$host" "cat /root/rza-merkez/merkez/Cargo.lock" > merkez/Cargo.lock.yeni 2>/dev/null || true
if [ -s merkez/Cargo.lock.yeni ]; then mv merkez/Cargo.lock.yeni merkez/Cargo.lock; else rm -f merkez/Cargo.lock.yeni; fi
exit $durum
