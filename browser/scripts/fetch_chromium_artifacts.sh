#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
worker_dir=$(cd "$script_dir/.." && pwd)
manifest="$worker_dir/oracle/manifest.json"
cache_dir=${SCRAPLING_CHROMIUM_ARTIFACT_DIR:-"$worker_dir/target/scrapling-chromium"}
mode=${1:-fetch}
target=${2:-}

if [[ -z $target ]]; then
  case "$(uname -m)-$(uname -s)" in
    x86_64-Linux) target=x86_64-unknown-linux-gnu ;;
    aarch64-Linux) target=aarch64-unknown-linux-gnu ;;
    *) echo "unsupported host; pass a Tier-1 Linux target" >&2; exit 2 ;;
  esac
fi
case "$mode:$target" in
  fetch:x86_64-unknown-linux-gnu|verify:x86_64-unknown-linux-gnu) suffix=linux-x64 ;;
  fetch:aarch64-unknown-linux-gnu|verify:aarch64-unknown-linux-gnu) suffix=linux-arm64 ;;
  *) echo "usage: $0 [fetch|verify] [x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu]" >&2; exit 2 ;;
esac

target_dir="$cache_dir/$target"
mkdir -p "$target_dir"

while IFS='|' read -r name archive bytes sha256 url; do
  archive_path="$target_dir/$archive"
  if [[ $mode == fetch ]]; then
    temporary=$(mktemp "$target_dir/.download.XXXXXX")
    curl --fail --location --retry 3 --output "$temporary" "$url"
    mv "$temporary" "$archive_path"
  fi
  [[ -f $archive_path ]] || { echo "missing artifact: $archive_path" >&2; exit 1; }
  actual_bytes=$(wc -c < "$archive_path")
  [[ $actual_bytes == "$bytes" ]] || {
    echo "size mismatch for $archive_path: expected $bytes, got $actual_bytes" >&2
    exit 1
  }
  actual_sha=$(sha256sum "$archive_path" | awk '{print $1}')
  [[ $actual_sha == "$sha256" ]] || {
    echo "SHA-256 mismatch for $archive_path: expected $sha256, got $actual_sha" >&2
    exit 1
  }
  if [[ $mode == fetch ]]; then
    unzip -oq "$archive_path" -d "$target_dir"
  fi
  echo "verified $name $sha256"
done < <(
  python3 - "$manifest" "$suffix" <<'PY'
import json, pathlib, sys
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
suffix = sys.argv[2]
for item in manifest["browser"]["archives"]:
    if item["name"] in {f"chromium-{suffix}", f"chromium-headless-shell-{suffix}"}:
        print("|".join(map(str, (item["name"], item["path"], item["size"], item["sha256"], item["url"]))))
PY
)

if [[ $mode == fetch ]]; then
  chrome_dir=$(find "$target_dir" -mindepth 1 -maxdepth 1 -type d -name 'chrome-linux*' ! -name 'chrome-headless*' | head -1)
  headless_dir=$(find "$target_dir" -mindepth 1 -maxdepth 1 -type d -name 'chrome-headless-shell-linux*' | head -1)
  mkdir -p "$target_dir/pw/chromium-1223" "$target_dir/pw/chromium_headless_shell-1223"
  ln -sfn "$chrome_dir" "$target_dir/pw/chromium-1223/$(basename "$chrome_dir")"
  ln -sfn "$headless_dir" "$target_dir/pw/chromium_headless_shell-1223/$(basename "$headless_dir")"
fi

if [[ $target == x86_64-unknown-linux-gnu ]]; then
  executable="$target_dir/chrome-linux64/chrome"
else
  executable="$target_dir/chrome-linux/chrome"
fi
[[ -x $executable ]] || {
  echo "verified archive is not extracted; run '$0 fetch $target'" >&2
  exit 1
}
"$executable" --version
printf 'SCRAPLING_CHROMIUM_EXECUTABLE=%s\n' "$executable"
printf 'PLAYWRIGHT_BROWSERS_PATH=%s\n' "$target_dir/pw"
