#!/bin/sh
# Bütün sunucu testleri; ilk başarısızda durur. Kullanım: sh scripts/sunucu/merkez/tests/hepsi.sh [python]
py=${1:-python}
export WC_TEST=1  # netgsm.gonder gerçek NetGSM isteği yerine hata fırlatır
for f in "$(dirname "$0")"/test_*.py; do
	printf '%s: ' "$(basename "$f")"
	"$py" "$f" 2>&1 | tail -1
	"$py" "$f" >/dev/null 2>&1 || exit 1
done
