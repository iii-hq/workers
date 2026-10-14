#!/usr/bin/env bash
# Run computer's Linux live tests (the ignored X11 driver tests and
# tests/semantic_live.rs) on a private Xvfb + D-Bus + AT-SPI stack. Used by CI
# and locally. It never touches the caller's desktop, session bus or dconf:
# everything runs inside dbus-run-session with GSETTINGS_BACKEND=memory (the
# IsEnabled write below would otherwise persist toolkit-accessibility=true).
# AT_SPI_BUS_ADDRESS is unset too: accesskit prefers it over the private bus.
#
#   tests/run-linux-live.sh                                   # everything
#   tests/run-linux-live.sh --test semantic_live invoke_dispatches_exactly_once
#
# Needs Xvfb, dbus-run-session, dbus-send, at-spi2-core and the fixture built:
#   cargo build --locked --manifest-path tests/fixture/Cargo.toml
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${COMPUTER_LIVE_INSIDE:-}" ]]; then
  # A Wayland login would make the worker refuse X11 (by design).
  exec env -u WAYLAND_DISPLAY -u XDG_SESSION_TYPE -u DISPLAY \
    -u AT_SPI_BUS_ADDRESS \
    GSETTINGS_BACKEND=memory COMPUTER_LIVE_INSIDE=1 \
    dbus-run-session -- "$0" "$@"
fi

first_exe() {
  for p in "$@"; do [[ -x $p ]] && { echo "$p"; return; }; done
  echo "none of: $*" >&2
  exit 1
}
launcher=$(first_exe /usr/libexec/at-spi-bus-launcher /usr/lib/at-spi-bus-launcher /usr/lib/at-spi2-core/at-spi-bus-launcher)
registryd=$(first_exe /usr/libexec/at-spi2-registryd /usr/lib/at-spi2-registryd /usr/lib/at-spi2-core/at-spi2-registryd)

n=${COMPUTER_LIVE_DISPLAY:-99}
if [[ -e /tmp/.X11-unix/X$n ]]; then
  echo "display :$n is taken; set COMPUTER_LIVE_DISPLAY to a free number" >&2
  exit 1
fi
pids=()
trap 'kill "${pids[@]}" 2>/dev/null || true' EXIT
# -dpi 96 and a scale of 1 keep the fixture's pixels 1:1 with the root.
# -noreset: Xvfb otherwise resets when its last client leaves, and the
# worker's short-lived detect/list_displays connections race that reset.
Xvfb ":$n" -screen 0 1280x1024x24 -dpi 96 -nolisten tcp -noreset &
pids+=($!)
export DISPLAY=":$n" WINIT_X11_SCALE_FACTOR=1
for _ in $(seq 100); do [[ -S /tmp/.X11-unix/X$n ]] && break; sleep 0.1; done

"$launcher" --launch-immediately &
pids+=($!)
for _ in $(seq 100); do
  dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus \
    org.a11y.Bus.GetAddress >/dev/null 2>&1 && break
  sleep 0.1
done
"$registryd" &
pids+=($!)
dbus-send --session --print-reply --dest=org.a11y.Bus /org/a11y/bus \
  org.freedesktop.DBus.Properties.Set string:org.a11y.Status string:IsEnabled \
  variant:boolean:true >/dev/null

export COMPUTER_A11Y_FIXTURE=${COMPUTER_A11Y_FIXTURE:-$PWD/tests/fixture/target/debug/computer-a11y-fixture}
if (($#)); then
  cargo test --locked "$@" -- --ignored --test-threads=1
else
  cargo test --locked --lib x11 -- --ignored --test-threads=1
  cargo test --locked --test semantic_live -- --ignored --test-threads=1
fi
