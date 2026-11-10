#!/bin/bash
# Simulated-desktop test of single-instance and window lifecycle behaviour (Linux/X11).
#
# usage: tests/desktop/run.sh [binary]     (default: src-tauri/target/release/sotyware)
#
# Needs: an X11 display ($DISPLAY, e.g. WSLg or Xvfb + a window manager), python3,
# python3-dbus, python3-gi, libX11 and dbus-run-session.
# Everything runs inside a private D-Bus session bus, so a sotyware already running
# on your real desktop is not touched, and a stub StatusNotifierWatcher stands in
# for the tray host.
set -euo pipefail
H=$(dirname "$(readlink -f "$0")")
BIN=$(readlink -f "${1:-$H/../../src-tauri/target/release/sotyware}")
[ -x "$BIN" ] || { echo "binary not found: $BIN (run: npm run tauri build)" >&2; exit 2; }

if [ -z "${SOTYWARE_TEST_WORK:-}" ]; then
  WORK=$(mktemp -d)
  # Private runtime dir (WSL may have no usable /run/user/$UID); the tray icon image
  # goes here. Set before the bus starts so the bus and its services use it too.
  mkdir -m 700 "$WORK/runtime"
  rc=0
  SOTYWARE_TEST_WORK="$WORK" XDG_RUNTIME_DIR="$WORK/runtime" \
    dbus-run-session -- "$0" "$BIN" || rc=$?
  # xdg-document-portal (activated by WebKitGTK) may leave a FUSE mount here.
  fusermount3 -uz "$WORK/runtime/doc" 2>/dev/null || fusermount -uz "$WORK/runtime/doc" 2>/dev/null || true
  rm -rf "$WORK" 2>/dev/null || echo "note: could not fully remove $WORK" >&2
  exit "$rc"
fi

WORK=$SOTYWARE_TEST_WORK
trap 'kill "${WATCHER:-}" 2>/dev/null || true' EXIT
REG="$WORK/tray-registrations"
touch "$REG"

# The driver waits until the watcher owns its bus name before launching sotyware.
python3 "$H/sni_watcher.py" "$REG" &
WATCHER=$!

GDK_BACKEND=x11 python3 "$H/single_instance.py" "$BIN" "$REG"
