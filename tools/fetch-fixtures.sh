#!/usr/bin/env bash
# Download public STEP test files into tests/fixtures/.
#
# Fixtures are not vendored: the NIST sets are US-Government public domain,
# but the STEP Tools samples carry no stated redistribution terms.
# See tests/fixtures/README.md for sources and tests/fixtures/manifest.toml
# for what each file exercises.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$here/tests/fixtures"
mkdir -p "$dest/steptools" "$dest/nist" "$dest/nist-edm"

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

# NIST MBE PMI validation models (CTC/FTC test cases): single parts in
# AP203 and AP242, with semantic and graphical PMI. ~14 MB.
# The /document/ link redirects to the current upload.
nist_zip="$dest/nist/NIST-PMI-STEP-Files.zip"
fetch "https://www.nist.gov/document/nist-pmi-step-files" "$nist_zip"
if [[ ! -d "$dest/nist/NIST-PMI-STEP-Files" ]]; then
  echo "unzip   $(basename "$nist_zip")"
  unzip -q -o "$nist_zip" -d "$dest/nist"
fi

# NIST Engineering Design Model Repository: 1990s AP203 files, published
# by NIST as public domain. as1_pe is the same AS1 model as the STEP Tools
# set, written by Pro/ENGINEER, so it doubles as an open nested assembly.
edm="https://raw.githubusercontent.com/usnistgov/engineering-design-models/master/models/STEP/STI"
for f in as1_pe moon_buggy_asm vaccase_asm_solid weldment_asm_solid \
         clevis21 valve gear; do
  fetch "$edm/$f/$f.stp" "$dest/nist-edm/$f.stp"
done

echo
echo "fixtures ready in $dest"
find "$dest" -type f \( -iname '*.stp' -o -iname '*.step' \) | wc -l | xargs echo "STEP files:"
