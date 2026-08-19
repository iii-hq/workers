#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
crate_dir=$(cd "$script_dir/../vendor/curl_impersonate_sys" && pwd)
manifest="$crate_dir/artifacts.manifest"
cache_dir=${CURL_IMPERSONATE_ARTIFACT_DIR:-"$crate_dir/artifacts"}
mode=fetch
requested_target=${1:-}

if [[ $requested_target == --verify ]]; then
  mode=verify
  requested_target=${2:-}
fi

if [[ -z $requested_target ]]; then
  case "$(uname -m)-$(uname -s)" in
    x86_64-Linux) requested_target=x86_64-unknown-linux-gnu ;;
    aarch64-Linux) requested_target=aarch64-unknown-linux-gnu ;;
    *)
      echo "unsupported host; pass x86_64-unknown-linux-gnu or aarch64-unknown-linux-gnu" >&2
      exit 2
      ;;
  esac
fi

verify_one() {
  local target=$1 archive=$2 bytes=$3 expected=$4
  local target_dir="$cache_dir/$target"
  local archive_path="$target_dir/$archive"
  local root="$target_dir/root"

  [[ -f $archive_path ]] || {
    echo "missing artifact: $archive_path" >&2
    return 1
  }
  local actual_bytes
  actual_bytes=$(wc -c < "$archive_path")
  [[ $actual_bytes == "$bytes" ]] || {
    echo "size mismatch for $archive_path: expected $bytes, got $actual_bytes" >&2
    return 1
  }
  local actual
  actual=$(sha256sum "$archive_path" | awk '{print $1}')
  [[ $actual == "$expected" ]] || {
    echo "SHA-256 mismatch for $archive_path: expected $expected, got $actual" >&2
    return 1
  }
  [[ -f "$root/libcurl-impersonate.a" && -f "$root/include/curl/curl.h" ]] || {
    echo "verified archive is not extracted under $root" >&2
    return 1
  }
  echo "verified $target $expected"
}

fetch_one() {
  local target=$1 archive=$2 bytes=$3 expected=$4 url=$5
  local target_dir="$cache_dir/$target"
  mkdir -p "$target_dir/root"
  local archive_path="$target_dir/$archive"
  local temporary
  temporary=$(mktemp "$target_dir/.download.XXXXXX")
  curl --fail --location --retry 3 --output "$temporary" "$url"

  local actual_bytes
  actual_bytes=$(wc -c < "$temporary")
  [[ $actual_bytes == "$bytes" ]] || {
    echo "size mismatch for downloaded $url: expected $bytes, got $actual_bytes" >&2
    return 1
  }
  local actual
  actual=$(sha256sum "$temporary" | awk '{print $1}')
  [[ $actual == "$expected" ]] || {
    echo "SHA-256 mismatch for downloaded $url: expected $expected, got $actual" >&2
    return 1
  }

  mv "$temporary" "$archive_path"
  tar -xzf "$archive_path" -C "$target_dir/root"
  verify_one "$target" "$archive" "$bytes" "$expected"
}

matched=0
while IFS='|' read -r target archive bytes sha256 url; do
  [[ -z $target || $target == \#* ]] && continue
  if [[ $requested_target != all && $requested_target != "$target" ]]; then
    continue
  fi
  matched=1
  if [[ $mode == verify ]]; then
    verify_one "$target" "$archive" "$bytes" "$sha256"
  else
    fetch_one "$target" "$archive" "$bytes" "$sha256" "$url"
  fi
done < "$manifest"

if [[ $matched == 0 ]]; then
  echo "target not present in $manifest: $requested_target" >&2
  exit 2
fi

