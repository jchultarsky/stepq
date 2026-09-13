#!/usr/bin/env bash
# Rewrite every fetched fixture through stepq, renumbering every instance,
# and check through Open CASCADE that no solid, volume, name or colour
# changed. Fetch fixtures first with tools/fetch-fixtures.sh.
#
# Fixtures over 16 MB are skipped unless STEPQ_LARGE_FIXTURES=1: Open
# CASCADE needs minutes for each of the large Open Compute assemblies.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

cargo build --release --quiet --example rewrite --manifest-path "$here/Cargo.toml"
rewrite="$here/target/release/examples/rewrite"

size=(-size -16M)
if [[ -n "${STEPQ_LARGE_FIXTURES:-}" && "${STEPQ_LARGE_FIXTURES}" != "0" ]]; then
  size=()
fi
files=()
while IFS= read -r -d '' file; do
  files+=("$file")
done < <(find "$here/tests/fixtures" -type f \( -iname '*.stp' -o -iname '*.step' \) ${size[@]+"${size[@]}"} -print0 | sort -z)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "no fixtures under tests/fixtures (run tools/fetch-fixtures.sh)" >&2
  exit 2
fi

status=0
for i in "${!files[@]}"; do
  file="${files[$i]}"
  target="$out/$i-$(basename "$file")"
  "$rewrite" --dense "$file" "$target"
  rc=0
  "$here/tools/verify-occt.py" compare "$file" "$target" || rc=$?
  case $rc in
    0) ;;
    1) status=1 ;;
    *) echo "CRASH $file: verify-occt.py exited with status $rc" >&2; status=1 ;;
  esac
done

echo
if [[ $status -eq 0 ]]; then
  echo "all ${#files[@]} rewritten fixtures match through Open CASCADE"
else
  echo "some rewritten fixtures do not match; see FAIL lines above" >&2
fi
exit $status
