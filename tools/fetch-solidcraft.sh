#!/bin/sh
# Download the pinned SolidCraft CLI and web app (upstream.toml) into upstream/<tag>/ and verify them.
# Each pinned sha256 must match both the release's SHA256SUMS.txt and the downloaded zip.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
get() { sed -n "s/^$1 *= *\"\(.*\)\"/\1/p" "$root/upstream.toml"; }
repo=$(get repo); tag=$(get tag); asset=$(get asset); pinned=$(get sha256)
web_asset=$(get web_asset); web_pinned=$(get web_sha256)
dir="$root/upstream/$tag"
mkdir -p "$dir"
cd "$dir"
[ -f "$asset" ] || gh release download "$tag" -R "$repo" -p "$asset" -p SHA256SUMS.txt --clobber
check() { # asset, pinned sha256
  listed=$(awk -v a="$1" '$2 == a { print $1 }' SHA256SUMS.txt)
  actual=$(shasum -a 256 "$1" | awk '{ print $1 }')
  if [ "$listed" != "$2" ] || [ "$actual" != "$2" ]; then
    echo "sha256 mismatch for $1: pinned $2, SHA256SUMS $listed, file $actual" >&2
    exit 1
  fi
}
check "$asset" "$pinned"
rm -rf cli && unzip -q "$asset" -d cli
if [ -n "$web_asset" ]; then
  [ -f "$web_asset" ] || gh release download "$tag" -R "$repo" -p "$web_asset" --clobber
  check "$web_asset" "$web_pinned"
  rm -rf web && unzip -q "$web_asset" -d web
fi
"$dir/cli/$(get binary)" --version
