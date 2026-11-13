#!/bin/sh
# Deterministic child fixture: emits JSON only, never opens sockets.
# POSIX sh on purpose: the worker gives every child a fresh HOME, and macOS
# /usr/bin/python3 keeps its bytecode cache there, so a Python fixture needed
# 170-300 ms to start and lost the races the tests budget 100-500 ms for.
[ "$1" = tunnel ] || exit 1
base=${0%/*}
read -r mode < "$base/mode"
while [ $# -gt 0 ]; do
    case $1 in
        --output) output=$2 ;;
        --metrics) metrics=$2 ;;
        --url) url=$2 ;;
        --config) config=$2 ;;
        --no-autoupdate) no_autoupdate=1 ;;
    esac
    shift
done
[ "$output" = json ] || exit 1
[ "$metrics" = 127.0.0.1:0 ] || exit 1
[ "$url" = http://127.0.0.1:3112 ] || exit 1
[ "$no_autoupdate" = 1 ] || exit 1
if env | grep -Eq '^(TUNNEL_|III_)'; then exit 1; fi
printf '{}\n' | cmp -s - "$config" || exit 1
echo $$ >> "$base/pids"

emit() {
    printf '%s\n' "$1" >&2
}
connected() {
    emit '{"level":"info","message":"Registered tunnel connection","connIndex":0,"connection":"72ff9dc2-84c6-4bb5-a536-349cf6dfe71d","protocol":"quic"}'
}

case $mode in
    crash) exit 23 ;;
    oversized) printf '%20000s\n' x >&2 ;;
    silent) ;;
    *) emit "{\"level\":\"info\",\"message\":\"|  https://fake-$$.trycloudflare.com  |\"}" ;;
esac
case $mode in
    ready | disconnect | reconnect | ignore-term) connected ;;
esac
# An ignored signal stays ignored across the exec below.
[ "$mode" = ignore-term ] && trap '' TERM
case $mode in
    disconnect | reconnect)
        sleep 0.2
        emit '{"level":"error","message":"Serve tunnel error","connIndex":0,"error":"fixture secret must never escape"}'
        if [ "$mode" = reconnect ]; then
            sleep 0.05
            connected
        fi
        ;;
esac
# exec keeps the recorded PID, so killing it leaves no orphan behind.
exec sleep 3600
