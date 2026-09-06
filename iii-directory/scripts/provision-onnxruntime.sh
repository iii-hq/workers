#!/usr/bin/env bash
# Provision the pinned static ONNX Runtime that iii-directory links on
# x86_64-unknown-linux-gnu (ort-sys with binary download disabled, so cargo
# never fetches it). Idempotent: exits 0 without network when the library is
# already present with the expected SHA-256. Prints the directory to use as
# ORT_LIB_PATH. Same archive ort-sys would fetch, verified at both layers.
set -euo pipefail

VERSION="1.28.0"
TARGET="x86_64-unknown-linux-gnu"
URL="https://cdn.pyke.io/0/pyke:ort-rs/ms@${VERSION}/${TARGET}.tar.lzma2"
ARCHIVE_SHA256="e454f710f8a49f53aa5b4ff51e3454ae1835777e431c6c35c5255ce6f205fd68"
LIB_SHA256="0bb8a9982b44df690195c2c34b75ca791c3b9f20070b8cecbd8f50c6264dd2e2"

dest="${ORT_LIB_PATH:-$HOME/.cache/iii/onnxruntime-static-${VERSION}-${TARGET}}"
lib="$dest/libonnxruntime.a"

if [[ -f "$lib" ]] && echo "$LIB_SHA256  $lib" | sha256sum -c --status -; then
  echo "$dest"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL --retry 3 -o "$tmp/ort.tar.lzma2" "$URL"
echo "$ARCHIVE_SHA256  $tmp/ort.tar.lzma2" | sha256sum -c --status -
mkdir -p "$dest"
# Raw LZMA2 stream with a 64 MiB dictionary (what ort-sys' lzma-rust2 reader
# uses); xz's default 8 MiB dictionary truncates it silently.
xz -dc --format=raw --lzma2=dict=64MiB "$tmp/ort.tar.lzma2" | tar -x -C "$dest"
echo "$LIB_SHA256  $lib" | sha256sum -c --status -
echo "$dest"
