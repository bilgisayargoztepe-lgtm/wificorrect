#!/usr/bin/env bash
# Yönetim merkezini (Rust ikilisi) sunucuya kurar ya da günceller. Kullanıcının bilgisayarında, Git Bash'te çalışır;
# `gh` (GitHub CLI) bu bilgisayarda oturum açmış olmalıdır. sudo parolasını kullanıcı terminalde girer; ajan girmez.
#
#   scripts/sunucu/merkez-kur.sh [hedef] [--ikili DOSYA] [--yalniz-denetle] [--kuru]
#
#   hedef              ssh hedefi (varsayılan gbserver@192.168.1.109); anahtar ~/.ssh/gbserver (WFC_SSH_KEY ile değişir)
#   --ikili DOSYA      acil durum: CI denetimini ve yapıt indirmeyi atlar, verilen ikiliyi kurar (sha256 ekrana yazılır);
#                      git denetimleri ve kalan adımlar aynıdır
#   --yalniz-denetle   sunucuda yalnız veritabanı yedeği + yeni ikiliyle denetim (kur.sh --yalniz-denetle); canlıya dokunmaz
#   --kuru             sunucuya BAĞLANMAZ: git/CI denetimleri, yapıt indirme ve sha256 doğrulaması, paketi yerel geçici
#                      dizinde kurma ve içeriğini listeleme, sunucuda çalışacak komutları yazma
#
# Sıra: git ağacı temiz ve HEAD itilmiş -> CI (test iş akışı) bu HEAD için başarılı -> yapıt wificorrect-merkez-<sha>
# indirilir, sha256 doğrulanır -> paket (scripts/sunucu'dan *.py, tests, logo, __pycache__ hariç + ikili) sunucuda
# /tmp/wfc-sunucu'ya açılır -> `sudo bash kur.sh` kullanıcının terminalinde çalışır (çıktı /tmp/wfc-sunucu/sonuc.txt).
# Kurulum başarısızsa kur.sh kendiliğinden eski kuruluma döner; elle dönüş için merkez-geri-don.sh.
set -euo pipefail

host=gbserver@192.168.1.109
ikili=
yalniz_denetle=0
kuru=0
konum=0
while [ $# -gt 0 ]; do
  case $1 in
    --ikili) [ $# -ge 2 ] || { echo "HATA: --ikili bir dosya ister" >&2; exit 2; }; ikili=$2; shift ;;
    --yalniz-denetle) yalniz_denetle=1 ;;
    --kuru) kuru=1 ;;
    -*) echo "HATA: bilinmeyen seçenek: $1" >&2; exit 2 ;;
    *) konum=$((konum + 1)); [ $konum -le 1 ] || { echo "HATA: fazla argüman: $1" >&2; exit 2; }; host=$1 ;;
  esac
  shift
done
[[ $host =~ ^[A-Za-z0-9._@-]+$ ]] || { echo "HATA: geçersiz ssh hedefi: $host" >&2; exit 2; }
anahtar=${WFC_SSH_KEY:-$HOME/.ssh/gbserver}
# göreli --ikili yolu cd'den önce mutlak yapılır
if [ -n "$ikili" ] && [ -f "$ikili" ]; then ikili=$(cd "$(dirname "$ikili")" && pwd)/$(basename "$ikili"); fi
cd "$(dirname "$0")/../.."
gecici=$(mktemp -d)
trap 'rm -rf "$gecici"' EXIT

# 1) git: ağaç temiz, HEAD itilmiş
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "HATA: git ağacında işlenmemiş değişiklik var; önce commit edin (kurulan şey HEAD'dir)." >&2
  git status --short --untracked-files=no >&2
  exit 1
fi
git fetch -q origin
sha=$(git rev-parse HEAD)
if ! git branch -r --contains "$sha" | grep -q .; then
  echo "HATA: HEAD ($sha) uzak depoya itilmemiş; önce 'git push' edin." >&2
  exit 1
fi
echo "git: HEAD $sha temiz ve itilmiş"

# 2) ikili: CI yapıtı ya da --ikili
if [ -n "$ikili" ]; then
  [ -f "$ikili" ] || { echo "HATA: --ikili dosyası yok: $ikili" >&2; exit 1; }
  cp "$ikili" "$gecici/wificorrect-merkez"
  echo "UYARI: --ikili verildi, CI denetimi atlandı. Kurulacak ikilinin sha256'sı:"
  sha256sum "$gecici/wificorrect-merkez"
else
  command -v gh >/dev/null || { echo "HATA: gh (GitHub CLI) bulunamadı" >&2; exit 1; }
  run=$(gh run list --commit "$sha" --workflow test --event push --limit 20 --json databaseId,conclusion \
    --jq '[.[] | select(.conclusion == "success")] | .[0].databaseId // empty')
  if [ -z "$run" ]; then
    echo "HATA: $sha için 'test' iş akışı başarıyla bitmiş görünmüyor (bitmedi ya da başarısız). Durum:" >&2
    gh run list --commit "$sha" --workflow test --event push --limit 5 >&2 || true
    exit 1
  fi
  echo "CI: test iş akışı başarılı (koşu $run)"
  gh run download "$run" -n "wificorrect-merkez-$sha" -D "$gecici/yapit"
  [ -f "$gecici/yapit/wificorrect-merkez" ] && [ -f "$gecici/yapit/wificorrect-merkez.sha256" ] \
    || { echo "HATA: yapıtta wificorrect-merkez ve wificorrect-merkez.sha256 olmalı" >&2; ls -la "$gecici/yapit" >&2; exit 1; }
  (cd "$gecici/yapit" && sha256sum -c wificorrect-merkez.sha256) || { echo "HATA: ikilinin sha256'sı tutmuyor" >&2; exit 1; }
  cp "$gecici/yapit/wificorrect-merkez" "$gecici/wificorrect-merkez"
  echo "ikili: wificorrect-merkez-$sha ($(sha256sum "$gecici/wificorrect-merkez" | cut -d' ' -f1))"
fi
chmod 755 "$gecici/wificorrect-merkez"

# 3) paket: HEAD'deki scripts/sunucu (Python, test, parite, logo, __pycache__ olmadan) + ikili
mkdir "$gecici/kaynak" "$gecici/paket"
git archive HEAD scripts/sunucu | tar -x -C "$gecici/kaynak"
tar -C "$gecici/kaynak/scripts/sunucu" -cf - --exclude='*.py' --exclude=tests --exclude=parite --exclude=logo --exclude=__pycache__ \
  wificorrect-sunucu merkez | tar -x -C "$gecici/paket"
cp "$gecici/wificorrect-merkez" "$gecici/paket/merkez/wificorrect-merkez"
[ -f "$gecici/paket/merkez/kur.sh" ] || { echo "HATA: pakette merkez/kur.sh yok" >&2; exit 1; }

kur_secenek=
[ "$yalniz_denetle" = 1 ] && kur_secenek=" --yalniz-denetle"
uzak_ssh=(ssh -i "$anahtar" "$host")
uzak_kur="set -o pipefail; sudo bash /tmp/wfc-sunucu/merkez/kur.sh${kur_secenek} 2>&1 | tee /tmp/wfc-sunucu/sonuc.txt; k=\$?; sudo chmod 644 /tmp/wfc-sunucu/sonuc.txt; exit \$k"

if [ "$kuru" = 1 ]; then
  echo "--- --kuru: sunucuya bağlanılmıyor. Paket içeriği:"
  (cd "$gecici/paket" && find . -type f | sort | while read -r f; do printf '  %s (%s bayt)\n' "$f" "$(wc -c <"$f" | tr -d ' ')"; done)
  echo "--- sunucuda çalışacak komutlar ($host):"
  echo "  ssh -i $anahtar $host 'rm -rf /tmp/wfc-sunucu && mkdir /tmp/wfc-sunucu && tar -C /tmp/wfc-sunucu -xf -'   (paket tar ile gönderilir)"
  echo "  ssh -t -i $anahtar $host '$uzak_kur'"
  echo "--kuru tamam: hiçbir şey kurulmadı."
  exit 0
fi

# 4) sunucuya kopyala ve kur (sudo parolasını kullanıcı girer)
echo "paket $host sunucusuna gönderiliyor..."
tar -C "$gecici/paket" -cf - . | "${uzak_ssh[@]}" 'rm -rf /tmp/wfc-sunucu && mkdir /tmp/wfc-sunucu && tar -C /tmp/wfc-sunucu -xf -'
echo "kur.sh çalıştırılıyor (sudo parolası istenecek)..."
ssh -t -i "$anahtar" "$host" "$uzak_kur"
