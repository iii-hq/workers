#!/bin/sh
set -eu

fail() { printf '%s\n' "$*" >&2; exit 1; }
dry_run=no
if [ "${1-}" = --dry-run ]; then dry_run=yes; shift; fi
[ "$#" -eq 3 ] || fail 'usage: install.sh [--dry-run] <version> <public_url> <code> | <version> --token <token>'
version=$1
mode=code
if [ "$2" = --token ]; then
    mode=token
    token=$3
    case "$token" in iiimj_?*) ;; *) fail 'invalid join token' ;; esac
    case "${token#iiimj_}" in *[!A-Za-z0-9_-]*) fail 'invalid join token' ;; esac
else
    public_url=$2
    code=$3
    case "$public_url" in http://?*|https://?*) ;; *) fail 'hub URL must use HTTP or HTTPS' ;; esac
    case "$code" in *[!A-Za-z0-9-]*) fail 'invalid device code' ;; esac
    [ "${#code}" -eq 8 ] || [ "${#code}" -eq 9 ] || fail 'invalid device code'
fi
for dependency in curl jq; do
    command -v "$dependency" >/dev/null 2>&1 || fail "install requires $dependency; install it and retry"
done
printf '%s' "$version" | jq -Re 'test("^[0-9]+\\.[0-9]+\\.[0-9]+([+-][A-Za-z0-9.+-]+)?$")' >/dev/null || fail 'invalid mesh version'
if [ "$mode" = token ]; then
    [ "${#token}" -le 8192 ] || fail 'invalid join token'
    printf '%s' "${token#iiimj_}" | jq -Re '
        try (gsub("-"; "+") | gsub("_"; "/") | @base64d | fromjson |
        (keys == ["identity", "public_url", "secret"]) and
        (.public_url | test("^[Hh][Tt][Tt][Pp][Ss]?://[^/?#@[:space:]]+(/[^?#]*)?$")) and
        (.identity | test("^[A-Za-z0-9+/]{43}=$")) and
        (.secret | test("^iiimjs_[A-Za-z0-9_-]{43}$"))) catch false
    ' >/dev/null || fail 'invalid join token'
fi
case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
    *) fail 'unsupported CPU architecture' ;;
esac
case "$(uname -s)" in
    Linux)
        libc=gnu
        for loader in /lib/ld-musl-*.so.1; do
            if [ -e "$loader" ]; then libc=musl; break; fi
        done
        [ "$arch-$libc" != aarch64-musl ] || fail 'Linux arm64 musl is unsupported'
        target=$arch-unknown-linux-$libc
        ;;
    Darwin) target=$arch-apple-darwin ;;
    *) fail 'this installer supports Linux and macOS' ;;
esac
request=$(jq -cn --arg version "=$version" --arg target "$target" '{worker:"mesh",version:$version,target:$target}')
response=$(curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-time 60 \
    --header 'Content-Type: application/json' --data "$request" https://api.workers.iii.dev/resolve)
artifact=$(printf '%s' "$response" | jq -ces --arg target "$target" --arg version "$version" '
    if length != 1 then error("multiple registry responses") else .[0] end |
    [.graph[] | select(.name == "mesh")] |
    if length != 1 then error("registry must resolve mesh exactly once") else .[0] end |
    if .type != "binary" or .version != $version then error("wrong mesh version or artifact type") else . end |
    .binaries[$target] |
    if (.url | type) != "string" or (.sha256 | type) != "string" then error("missing target artifact") else . end |
    if (.url | test("^https://[^/?#[:space:]@]+(/[^[:space:]]*)?$")) and (.sha256 | test("^[A-Fa-f0-9]{64}$"))
    then . else error("artifact requires HTTPS and a SHA256 digest") end')
artifact_url=$(printf '%s' "$artifact" | jq -r .url)
if [ "$dry_run" = yes ]; then printf '%s\n' "$artifact_url"; exit 0; fi
command -v sudo >/dev/null 2>&1 || fail 'install requires sudo and a non-root operator'
if command -v sha256sum >/dev/null 2>&1; then hash_tool=sha256sum
elif command -v shasum >/dev/null 2>&1; then hash_tool=shasum
else fail 'install requires sha256sum or shasum'; fi
work=$(mktemp -d "${TMPDIR:-/tmp}/iii-mesh-install.XXXXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-time 300 \
    --output "$work/mesh" "$artifact_url"
expected=$(printf '%s' "$artifact" | jq -r '.sha256 | ascii_downcase')
if [ "$hash_tool" = sha256sum ]; then actual=$(sha256sum "$work/mesh")
else actual=$(shasum -a 256 "$work/mesh"); fi
actual=${actual%% *}
[ "$actual" = "$expected" ] || fail 'mesh SHA256 mismatch; refusing installation'
chmod 755 "$work/mesh"
if [ "$mode" = token ]; then sudo -- "$work/mesh" join --token "$token"
else sudo -- "$work/mesh" join "$public_url" "$code"; fi
