#!/usr/bin/env bash
# Testleri cihazda çalıştırır: optimizasyonsuz, 2 iş, bellek sınırlı (2026-10-03'te release+LTO test derlemesi cihazı kilitledi).
# Kullanım: scripts/test.sh [ssh-hedefi] [test-süzgeci]
set -euo pipefail
host=${1:-wificorrect}
filter=${2:-}
cd "$(dirname "$0")/.."
tar -cf - Cargo.toml Cargo.lock .cargo src tests deploy/debian/etc/dnsmasq.d | ssh "$host" "set -e; mkdir -p /root/rza; cd /root/rza; rm -rf src tests; tar -xf - -C /root/rza
systemd-run --scope -q -p MemoryMax=1200M nice -n 10 timeout 1500 cargo test -j 2 $filter 2>&1 | grep -E '^(error|warning)|-->|FAILED|test result|panicked' -A4 | head -60"
