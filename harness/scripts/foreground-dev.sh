#!/bin/sh
set -eu

MODE=${1:-up}

: "${REPO_ROOT:?REPO_ROOT is required}"
: "${WORKTREE_ROOT_ABS:?WORKTREE_ROOT_ABS is required}"
: "${ENGINE_CONFIG:?ENGINE_CONFIG is required}"
: "${ENGINE_URL:?ENGINE_URL is required}"
: "${ENGINE_PORT:?ENGINE_PORT is required}"
: "${CONSOLE_PORT:?CONSOLE_PORT is required}"
: "${RUST_LOG:?RUST_LOG is required}"
: "${RUN_PROFILE:?RUN_PROFILE is required}"
: "${STACK:?STACK is required}"
: "${PID_FILE:?PID_FILE is required}"

is_alive() {
	kill -0 "$1" 2>/dev/null
}

stop_running() {
	if [ ! -f "$PID_FILE" ]; then
		echo "no foreground harness stack is running"
		return 0
	fi

	while IFS=' ' read -r name pid; do
		case "$pid" in
			''|*[!0-9]*) continue ;;
		esac
		if is_alive "$pid"; then
			echo "stopping $name ($pid)"
			kill "$pid" 2>/dev/null || true
		fi
	done < "$PID_FILE"

	rm -f "$PID_FILE"
}

if [ "$MODE" = stop ]; then
	stop_running
	exit 0
fi

if [ "$MODE" != up ]; then
	echo "usage: $0 [up|stop]" >&2
	exit 2
fi

command -v iii >/dev/null 2>&1 || {
	echo "iii CLI is required; run make install-iii-next first" >&2
	exit 127
}
command -v cargo >/dev/null 2>&1 || {
	echo "cargo is required; install the Rust toolchain first" >&2
	exit 127
}

if [ -f "$PID_FILE" ]; then
	alive_pid=
	while IFS=' ' read -r _name pid; do
		case "$pid" in
			''|*[!0-9]*) continue ;;
		esac
		if is_alive "$pid"; then
			alive_pid=$pid
			break
		fi
	done < "$PID_FILE"
	if [ -n "$alive_pid" ]; then
		echo "foreground harness stack is already running (pid $alive_pid)" >&2
		echo "run make dev-down before starting it again" >&2
		exit 2
	fi
	rm -f "$PID_FILE"
fi

mkdir -p "$(dirname "$PID_FILE")"
: > "$PID_FILE"
pids=

record_pid() {
	name=$1
	pid=$2
	pids="$pids $pid"
	printf '%s %s\n' "$name" "$pid" >> "$PID_FILE"
	printf 'started %-24s (pid %s)\n' "$name" "$pid"
}

cleanup() {
	exit_code=$?
	trap - 0 1 2 3 15
	for pid in $pids; do
		kill "$pid" 2>/dev/null || true
	done
	for pid in $pids; do
		wait "$pid" 2>/dev/null || true
	done
	rm -f "$PID_FILE"
	exit "$exit_code"
}

trap cleanup 0 1 2 3 15

if command -v lsof >/dev/null 2>&1 &&
	lsof -tiTCP:"$ENGINE_PORT" -sTCP:LISTEN >/dev/null 2>&1; then
	echo "TCP $ENGINE_PORT is already in use" >&2
	lsof -nP -iTCP:"$ENGINE_PORT" -sTCP:LISTEN || true
	exit 2
fi

(
	cd "$REPO_ROOT"
	exec iii -c "$ENGINE_CONFIG"
) &
engine_pid=$!
record_pid engine "$engine_pid"

echo "waiting for engine on $ENGINE_URL"
i=0
while [ "$i" -lt 40 ]; do
	if iii trigger engine::workers::list --port "$ENGINE_PORT" --json '{}' >/dev/null 2>&1; then
		echo "engine ready"
		break
	fi
	if ! is_alive "$engine_pid"; then
		echo "engine exited before becoming ready" >&2
		exit 1
	fi
	i=$((i + 1))
	sleep 0.5
done
if [ "$i" -ge 40 ]; then
	echo "engine did not become ready on $ENGINE_URL" >&2
	exit 1
fi

start_worker() {
	worker=$1
	manifest="$WORKTREE_ROOT_ABS/$worker/Cargo.toml"
	test -f "$manifest" || {
		echo "missing $manifest" >&2
		return 1
	}

	(
		cd "$WORKTREE_ROOT_ABS"
		if [ "$worker" = console ]; then
			if [ "$RUN_PROFILE" = release ]; then
				exec env RUST_LOG="$RUST_LOG" cargo run --release --manifest-path "$manifest" -- --url "$ENGINE_URL" --http-port "$CONSOLE_PORT"
			fi
			exec env RUST_LOG="$RUST_LOG" cargo run --manifest-path "$manifest" -- --url "$ENGINE_URL" --http-port "$CONSOLE_PORT"
		fi
		if [ "$RUN_PROFILE" = release ]; then
			exec env RUST_LOG="$RUST_LOG" cargo run --release --manifest-path "$manifest" -- --url "$ENGINE_URL"
		fi
		exec env RUST_LOG="$RUST_LOG" cargo run --manifest-path "$manifest" -- --url "$ENGINE_URL"
	) &
	record_pid "$worker" "$!"
}

for worker in $STACK; do
	start_worker "$worker"
done

echo "foreground harness stack is running"
echo "console: http://127.0.0.1:$CONSOLE_PORT"
echo "press Ctrl-C to stop the engine and all workers"

while is_alive "$engine_pid"; do
	sleep 1
done

echo "engine exited" >&2
exit 1
