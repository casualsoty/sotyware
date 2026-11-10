#!/bin/bash
# Virtual-display test (Linux): the simulated desktop test (run.sh) on a private
# Xvfb display with the openbox window manager, so it needs no monitor and no
# desktop session, and never touches the display you are using.
#
# usage: tests/desktop/virtual.sh [binary]   (default: src-tauri/target/release/sotyware)
#
# Needs: Xvfb, openbox, and everything run.sh needs (python3, python3-dbus,
# python3-gi, the AT-SPI typelib and registry, libX11, dbus-run-session).
# Debian/Ubuntu:
#   sudo apt install xvfb openbox python3-dbus python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-x11
#
# Exit code: that of run.sh (0 no check failed, 1 a check failed, 2 binary
# missing); 2 if a dependency is missing; 3 if Xvfb or openbox did not start;
# 124 if the test exceeded SOTYWARE_VIRTUAL_TIMEOUT seconds (default 300).
# Logs of Xvfb and openbox are printed when the run does not pass.
set -euo pipefail
H=$(dirname "$(readlink -f "$0")")
BIN=$(readlink -f "${1:-$H/../../src-tauri/target/release/sotyware}")
TIMEOUT=${SOTYWARE_VIRTUAL_TIMEOUT:-300}

missing=()
for cmd in Xvfb openbox python3 dbus-run-session timeout; do
  command -v "$cmd" >/dev/null || missing+=("$cmd")
done
if command -v python3 >/dev/null && ! python3 -c 'import dbus, gi; gi.require_version("Atspi", "2.0")' 2>/dev/null; then
  missing+=("python3-dbus/python3-gi/gir1.2-atspi-2.0")
fi
if [ ${#missing[@]} -gt 0 ]; then
  echo "missing: ${missing[*]}" >&2
  echo "install (Debian/Ubuntu): sudo apt install xvfb openbox python3-dbus python3-gi gir1.2-atspi-2.0 at-spi2-core dbus-x11" >&2
  exit 2
fi
[ -x "$BIN" ] || { echo "binary not found: $BIN (run: npm run tauri build -- --no-bundle)" >&2; exit 2; }

LOGS=$(mktemp -d)
XVFB_PID="" WM_PID="" DISP="" rc=0

wait_until() { # <seconds> <command...>: poll a readiness condition, no fixed sleep
  local end=$((SECONDS + $1)); shift
  until "$@"; do
    [ "$SECONDS" -lt "$end" ] || return 1
    sleep 0.1
  done
}

cleanup() {
  # sotyware processes started on this display (e.g. left behind by a timeout).
  if [ -n "$DISP" ]; then
    for p in /proc/[0-9]*; do
      [ "$(readlink "$p/exe" 2>/dev/null)" = "$BIN" ] || continue
      tr '\0' '\n' <"$p/environ" 2>/dev/null | grep -qx "DISPLAY=:$DISP" && kill "${p#/proc/}" 2>/dev/null || true
    done
  fi
  [ -n "$WM_PID" ] && kill "$WM_PID" 2>/dev/null || true
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  if [ "$rc" -ne 0 ]; then
    for f in "$LOGS"/*.log; do
      [ -s "$f" ] && { echo "--- $(basename "$f") ---" >&2; tail -n 40 "$f" >&2; }
    done
  fi
  rm -rf "$LOGS"
}
trap cleanup EXIT

# Pick a display number nobody uses. Start high to stay clear of real desktops,
# and skip numbers with a lock file or socket (an Xvfb started by someone else).
for n in $(seq 99 -1 20); do
  [ -e "/tmp/.X$n-lock" ] || [ -e "/tmp/.X11-unix/X$n" ] || { DISP=$n; break; }
done
[ -n "$DISP" ] || { echo "no free X display number in :20-:99" >&2; rc=3; exit 3; }

# Listen on the abstract socket only: no TCP, and no file in /tmp/.X11-unix
# (read-only under WSLg). -displayfd reports when the server accepts clients.
Xvfb ":$DISP" -screen 0 1280x800x24 -nolisten tcp -nolisten unix -displayfd 3 \
  3>"$LOGS/display" >"$LOGS/xvfb.log" 2>&1 &
XVFB_PID=$!
if ! wait_until 15 test -s "$LOGS/display"; then
  echo "Xvfb did not start on :$DISP" >&2; rc=3; exit 3
fi

# openbox runs --startup only once it manages the screen.
DISPLAY=":$DISP" openbox --sm-disable --startup "touch $LOGS/wm-ready" >"$LOGS/openbox.log" 2>&1 &
WM_PID=$!
if ! wait_until 15 test -e "$LOGS/wm-ready"; then
  echo "openbox did not start on :$DISP" >&2; rc=3; exit 3
fi
echo "virtual display :$DISP (Xvfb, $(openbox --version | head -n 1))"

# WAYLAND_DISPLAY unset so nothing reaches a real Wayland session.
set +e
env -u WAYLAND_DISPLAY DISPLAY=":$DISP" timeout --kill-after=10 "$TIMEOUT" "$H/run.sh" "$BIN" \
  2>&1 | tee "$LOGS/run.out"
rc=${PIPESTATUS[0]}
set -e
[ "$rc" -ne 124 ] || echo "timed out after ${TIMEOUT}s" >&2
# This display and window manager are fully controlled, so every check must run:
# a SKIP here means something regressed, not that the environment cannot do it.
if [ "$rc" -eq 0 ] && ! grep -qE '^[0-9]+ passed, 0 failed, 0 skipped$' "$LOGS/run.out"; then
  echo "FAIL: checks were skipped (or no summary line); every check must run on the virtual display" >&2
  rc=1
fi
exit "$rc"
