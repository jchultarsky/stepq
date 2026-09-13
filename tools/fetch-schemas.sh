#!/usr/bin/env bash
# Download long-form EXPRESS schemas into tests/schemas/.
#
# The schemas are ISO copyright. ISO permits using them "in their original
# format without any modifications" for the purposes of the standard, but
# does not clearly permit redistribution. stepq therefore neither commits
# nor ships them, nor any table derived from them: they are read at run
# time by stepq::express::Schema::parse.
#
# AP242 comes from ISO directly. ISO does not host AP203 or AP214, so those
# come from the stepcode project, pinned to a commit. Every file is checked
# against its SHA-256 on each run.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$here/tests/schemas"
mkdir -p "$dest"

stepcode="https://raw.githubusercontent.com/stepcode/stepcode/ed686ee1d9cb8bf763ab8d61ef6d417c3b45c146/data"

sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# fetch URL OUT SHA256 — an empty SHA256 prints the checksum instead of checking it.
fetch() {
  local url="$1" out="$2" expected="$3"
  if [[ -s "$out" ]]; then
    echo "have    $(basename "$out")"
  else
    echo "fetch   $(basename "$out")"
    curl -fsSL --retry 3 "$url" -o "$out.part"
    mv "$out.part" "$out"
  fi
  local actual
  actual="$(sha256 "$out")"
  if [[ -z "$expected" ]]; then
    echo "        sha256 $actual (not pinned yet)"
  elif [[ "$actual" != "$expected" ]]; then
    echo "error: $(basename "$out") has sha256 $actual, expected $expected" >&2
    exit 1
  fi
}

# AP242 edition 4 (ISO/TS 10303-442 ed. 7 long-form MIM).
fetch "https://standards.iso.org/iso/ts/10303/-442/ed-7/tech/express/mim_lf.exp" \
  "$dest/ap242e4_mim_lf.exp" e7e93cf97880fd87d634e4b9ee58400da0a1be6c06a1de76ec13807ecdc15ccb
# AP214 edition 3.
fetch "$stepcode/ap214e3/AP214E3_2010.exp" "$dest/ap214e3.exp" \
  71ab140fe7f774321beee6a31e6fee2afc3973fd60350ae2018c74c211fb4295
# AP203 edition 2 long-form MIM.
fetch "$stepcode/ap203e2/ap203e2_mim_lf.exp" "$dest/ap203e2_mim_lf.exp" \
  c68dd02200e2f4213e311553a4f6295effd168209de1fdc4e8003ce4c9ddd088
# AP203 edition 1 (CONFIG_CONTROL_DESIGN).
fetch "$stepcode/ap203/ap203.exp" "$dest/ap203.exp" \
  020b4d25dbd0b6ee7d15099b978e3448f6699a72cb862d381e416e32187562f1

echo
echo "schemas ready in $dest"
