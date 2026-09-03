#!/usr/bin/env bash
set -euo pipefail

readonly STACK_BINARIES=(
  queue
  iii-directory
  session-manager
  context-manager
  cron
  state
  database
  harness
  harness-integration
  console
)

usage() {
  cat <<'EOF'
Usage:
  harness_stack_bundle.sh pack --output FILE --source-sha SHA --engine-version VERSION --engine-bin FILE --bin-dir DIR
  harness_stack_bundle.sh unpack --archive FILE --destination DIR --expected-source-sha SHA
EOF
}

fail() {
  printf '%s\n' "$*" >&2
  exit 3
}

require_sha() {
  [[ "$1" =~ ^[0-9a-f]{40}$ ]] || fail "invalid source SHA: $1"
}

pack() {
  local output='' source_sha='' engine_version='' engine_bin='' bin_dir=''
  while (($#)); do
    case "$1" in
      --output) output=${2:-}; shift 2 ;;
      --source-sha) source_sha=${2:-}; shift 2 ;;
      --engine-version) engine_version=${2:-}; shift 2 ;;
      --engine-bin) engine_bin=${2:-}; shift 2 ;;
      --bin-dir) bin_dir=${2:-}; shift 2 ;;
      *) fail "unknown pack argument: $1" ;;
    esac
  done

  [[ -n "$output" && -n "$source_sha" && -n "$engine_version" && -n "$engine_bin" && -n "$bin_dir" ]] || {
    usage >&2
    exit 2
  }
  require_sha "$source_sha"
  [[ -x "$engine_bin" ]] || fail "engine binary is missing or not executable: $engine_bin"
  for binary in "${STACK_BINARIES[@]}"; do
    [[ -x "$bin_dir/$binary" ]] || fail "stack binary is missing or not executable: $bin_dir/$binary"
  done

  output=$(realpath -m "$output")
  mkdir -p "$(dirname "$output")"
  local temp_root
  temp_root=$(mktemp -d "${RUNNER_TEMP:-/tmp}/harness-stack-bundle.XXXXXX")
  mkdir -p "$temp_root/bin"
  install -m 0755 "$engine_bin" "$temp_root/bin/iii"
  for binary in "${STACK_BINARIES[@]}"; do
    install -m 0755 "$bin_dir/$binary" "$temp_root/bin/$binary"
  done

  (
    cd "$temp_root"
    sha256sum bin/* > SHA256SUMS
  )
  jq -n \
    --arg source_sha "$source_sha" \
    --arg engine_version "$engine_version" \
    --arg runner_os "${RUNNER_OS:-unknown}" \
    --arg runner_arch "${RUNNER_ARCH:-unknown}" \
    --argjson binaries "$(printf '%s\n' iii "${STACK_BINARIES[@]}" | jq -R . | jq -s .)" \
    '{
      schema: "harness-stack-bundle/v1",
      source_sha: $source_sha,
      engine_version: $engine_version,
      runner: {os: $runner_os, arch: $runner_arch},
      checksums: "SHA256SUMS",
      binaries: $binaries
    }' > "$temp_root/manifest.json"

  tar --zstd -cf "$output" -C "$temp_root" .
  rm -r -- "$temp_root"
  printf 'packed %s\n' "$output"
}

unpack() {
  local archive='' destination='' expected_source_sha=''
  while (($#)); do
    case "$1" in
      --archive) archive=${2:-}; shift 2 ;;
      --destination) destination=${2:-}; shift 2 ;;
      --expected-source-sha) expected_source_sha=${2:-}; shift 2 ;;
      *) fail "unknown unpack argument: $1" ;;
    esac
  done

  [[ -n "$archive" && -n "$destination" && -n "$expected_source_sha" ]] || {
    usage >&2
    exit 2
  }
  require_sha "$expected_source_sha"
  [[ -f "$archive" ]] || fail "bundle archive does not exist: $archive"
  if tar --zstd -tf "$archive" | awk '$0 ~ /^\// || $0 ~ /(^|\/)\.\.($|\/)/ {exit 1}'; then
    :
  else
    fail "bundle archive contains an unsafe path"
  fi
  if [[ -e "$destination" ]]; then
    [[ -d "$destination" ]] || fail "bundle destination is not a directory: $destination"
    [[ -z "$(find "$destination" -mindepth 1 -print -quit)" ]] || fail "bundle destination is not empty: $destination"
  fi
  mkdir -p "$destination"
  tar --zstd -xf "$archive" -C "$destination"

  [[ $(jq -er '.schema' "$destination/manifest.json") == 'harness-stack-bundle/v1' ]] || fail "unexpected bundle schema"
  [[ $(jq -er '.source_sha' "$destination/manifest.json") == "$expected_source_sha" ]] || fail "bundle source SHA does not match $expected_source_sha"
  [[ $(jq -er '.checksums' "$destination/manifest.json") == 'SHA256SUMS' ]] || fail "unexpected checksum manifest"
  (
    cd "$destination"
    sha256sum -c SHA256SUMS
  )
  for binary in iii "${STACK_BINARIES[@]}"; do
    [[ -f "$destination/bin/$binary" ]] || fail "bundle binary is missing: $binary"
    chmod 0755 "$destination/bin/$binary"
  done
  printf 'unpacked source %s into %s\n' "$expected_source_sha" "$destination"
}

command=${1:-}
[[ -n "$command" ]] || { usage >&2; exit 2; }
shift
case "$command" in
  pack) pack "$@" ;;
  unpack) unpack "$@" ;;
  *) usage >&2; exit 2 ;;
esac
