#!/usr/bin/env bash
# Download public STEP test files into tests/fixtures/.
#
# Fixtures are not vendored: the NIST set is US-Government public domain,
# but the STEP Tools samples carry no stated redistribution terms.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$here/tests/fixtures"
mkdir -p "$dest/steptools" "$dest/nist"

fetch() {
  local url="$1" out="$2"
  if [[ -s "$out" ]]; then
    echo "have    $(basename "$out")"
  else
    echo "fetch   $(basename "$out")"
    curl -fsSL --retry 3 "$url" -o "$out"
  fi
}

# STEP Tools: the AS1 assembly as written by five different exporters.
# Same geometry, five dialects — ideal for differential testing.
for f in as1-ac-214 as1-ec-214 as1-md-214 as1-tc-214 as1-ug-214; do
  fetch "https://steptools.com/docs/stpfiles/ap214/$f.stp" "$dest/steptools/$f.stp"
done

# NIST MBE PMI validation models: the same parts in AP203 and AP242
# ed1/ed2/ed3 variants, with semantic and graphical PMI.
nist_zip="$dest/nist/NIST-PMI-STEP-Files.zip"
fetch "https://www.nist.gov/system/files/documents/noindex/2024/06/19/NIST-PMI-STEP-Files.zip" "$nist_zip"
if [[ ! -d "$dest/nist/NIST-PMI-STEP-Files" ]]; then
  echo "unzip   $(basename "$nist_zip")"
  unzip -q -o "$nist_zip" -d "$dest/nist"
fi

echo
echo "fixtures ready in $dest"
find "$dest" -type f \( -iname '*.stp' -o -iname '*.step' \) | wc -l | xargs echo "STEP files:"
