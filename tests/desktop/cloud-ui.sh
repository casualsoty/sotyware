#!/bin/bash
# Cloud Storage end-to-end test (Linux, opt-in; not part of CI): drives the
# Cloud Storage page of the real release binary on a private Xvfb display with
# openbox, on a private D-Bus session with a real GNOME Keyring holding a
# throwaway login keyring. Never touches your display, session bus or
# keyrings. Details: docs/development/testing.md#cloud-storage-ui-end-to-end-test
#
# usage: tests/desktop/cloud-ui.sh [phase...]   phases: local hang browse b2 b2-browse (default: local hang browse)
#   SOTYWARE_CLOUD_UI_BIN    binary (default: src-tauri/target/release/sotyware)
#   SOTYWARE_GNOME_KEYRING_ROOT  folder with the gnome-keyring packages unpacked
#                            (dpkg -x), if gnome-keyring-daemon is not installed
#
# local: made-up keys, an endpoint nobody listens on; contacts no provider.
# hang:  a Secret Service that never answers; the window must stay usable.
# browse: the bucket browser against the in-process listing server of the Rust
#        tests (cargo test ... serve_listing_for_ui -- --ignored), which fails
#        if anything but a listing reached it; contacts no provider.
# b2:    the real Backblaze B2 test bucket and key from the opt-in integration
#        test's variables (SOTYWARE_IT_B2_BUCKET, SOTYWARE_IT_B2_REGION,
#        SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID, SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY),
#        read from the environment only; never printed. Before and after, the
#        opt-in Rust test backblaze_b2_fingerprint hashes every object version
#        under sotyware-it/; the phase fails if they differ.
# b2-browse: writes a temporary dataset to the Backblaze test bucket (opt-in
#        Rust test backblaze_b2_browse_dataset, kept, under
#        sotyware-it/browse-validation/<run id>/ only), browses it through the
#        UI with a second, read-only key without a file name prefix
#        (SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID, ..._SECRET_ACCESS_KEY),
#        compares every row with the provider's listing, and always deletes
#        exactly that run's versions afterwards (backblaze_b2_browse_cleanup,
#        also when interrupted). The fingerprint of sotyware-it/ must be the
#        same before and after the UI, and before and after the whole phase.
#
# Exit code: 0 every check passed, 1 a check failed, 2 a dependency or
# variable is missing, 3 Xvfb, openbox or the Secret Service did not start.
set -uo pipefail
H=$(dirname "$(readlink -f "$0")")
REPO=$(readlink -f "$H/../..")
PHASES=("$@")
[ ${#PHASES[@]} -gt 0 ] || PHASES=(local hang browse)

if [ -z "${SOTYWARE_CLOUD_UI_INNER:-}" ]; then
  BIN=$(readlink -f "${SOTYWARE_CLOUD_UI_BIN:-$REPO/src-tauri/target/release/sotyware}")
  [ -x "$BIN" ] || { echo "binary not found: $BIN (run: npm run tauri build -- --no-bundle)" >&2; exit 2; }
  for cmd in Xvfb openbox python3 dbus-run-session dbus-send; do
    command -v "$cmd" >/dev/null || { echo "missing: $cmd" >&2; exit 2; }
  done
  python3 -c 'import dbus, gi; gi.require_version("Atspi", "2.0"); from gi.repository import GLib' 2>/dev/null \
    || { echo "missing: python3-dbus/python3-gi/gir1.2-atspi-2.0" >&2; exit 2; }
  if [ -n "${SOTYWARE_GNOME_KEYRING_ROOT:-}" ]; then
    GK_BIN=$SOTYWARE_GNOME_KEYRING_ROOT/usr/bin/gnome-keyring-daemon
  else
    GK_BIN=$(command -v gnome-keyring-daemon || true)
  fi
  [ -x "${GK_BIN:-}" ] || { echo "gnome-keyring-daemon not found: install gnome-keyring or set SOTYWARE_GNOME_KEYRING_ROOT" >&2; exit 2; }
  for phase in "${PHASES[@]}"; do
    case $phase in
      local|hang|browse) ;;
      b2)
        for v in SOTYWARE_IT_B2_BUCKET SOTYWARE_IT_B2_REGION SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY; do
          [ -n "${!v:-}" ] || { echo "the b2 phase needs $v (see testing.md)" >&2; exit 2; }
        done ;;
      b2-browse)
        for v in SOTYWARE_IT_B2_BUCKET SOTYWARE_IT_B2_REGION SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY \
                 SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID SOTYWARE_CLOUD_IT_B2_READ_SECRET_ACCESS_KEY; do
          [ -n "${!v:-}" ] || { echo "the b2-browse phase needs $v (see testing.md)" >&2; exit 2; }
        done ;;
      *) echo "unknown phase: $phase" >&2; exit 2 ;;
    esac
  done
  export SOTYWARE_CLOUD_UI_BIN=$BIN SOTYWARE_CLOUD_UI_GK=$GK_BIN SOTYWARE_CLOUD_UI_INNER=1
  exec dbus-run-session -- "$0" "${PHASES[@]}"
fi

# ---- inside the private session bus ----
BIN=$SOTYWARE_CLOUD_UI_BIN GK_BIN=$SOTYWARE_CLOUD_UI_GK
ORIG_HOME=$HOME
# Short: Unix socket paths must stay under 108 bytes.
WORK=$(mktemp -d /tmp/sotyware-cloud-ui.XXXXXX) || exit 3
XVFB_PID="" WM_PID="" SS_PID="" SRV_PID="" BROWSE_RUN="" rc=0
# Deletes exactly the versions of the kept browse dataset of this run, and
# lists again; prints the result lines of the Rust test.
b2_browse_cleanup() {
  [ -n "$BROWSE_RUN" ] || return 0
  (cd "$REPO/src-tauri" && HOME=$ORIG_HOME SOTYWARE_IT_B2_BROWSE_RUN=$BROWSE_RUN cargo test --quiet --lib \
    cloud::integration::browse::backblaze_b2_browse_cleanup -- --ignored --nocapture >"$WORK/browse-cleanup.log" 2>&1)
  local crc=$?
  grep -E '^(cleaning up|cleanup:|CLEANUP)' "$WORK/browse-cleanup.log" | sed 's/^/  /'
  if [ $crc -eq 0 ]; then
    echo "[PASS] b2-browse: cleanup deleted this run's versions by id and found none left"
    BROWSE_RUN=""
  else
    echo "[FAIL] b2-browse: CLEANUP INCOMPLETE for run $BROWSE_RUN (see the lines above)"; rc=1
  fi
}
cleanup() {
  b2_browse_cleanup
  rm -f -- "$WORK/browse-endpoint"
  for p in $SS_PID $SRV_PID $WM_PID $XVFB_PID; do kill "$p" 2>/dev/null; done
  pkill -f -- "^$GK_BIN " 2>/dev/null
  # Only the folder this run created (throwaway HOME and keyring).
  case $WORK in /tmp/sotyware-cloud-ui.??????) rm -rf -- "$WORK" ;; esac
}
trap cleanup EXIT

has_owner() {
  dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets 2>/dev/null | grep -q true
}
wait_owner() { # <want: 0 owned, 1 not owned>
  for _ in $(seq 100); do
    if has_owner; then [ "$1" = 0 ] && return 0; else [ "$1" = 1 ] && return 0; fi
    sleep 0.1
  done
  return 1
}

fingerprint() { # prints the fingerprint line of the opt-in Rust test, or nothing
  (cd "$REPO/src-tauri" && HOME=$ORIG_HOME cargo test --quiet --lib cloud::integration::backblaze_b2_fingerprint \
    -- --ignored --nocapture 2>/dev/null) | grep -E '^sotyware-it fingerprint [0-9a-f]{64} versions [0-9]+$'
}

for n in $(seq 88 -1 60); do
  grep -q " @/tmp/.X11-unix/X$n\$" /proc/net/unix 2>/dev/null || [ -e "/tmp/.X11-unix/X$n" ] || { DISP=$n; break; }
done
Xvfb ":$DISP" -screen 0 1280x900x24 -nolisten tcp >"$WORK/xvfb.log" 2>&1 & XVFB_PID=$!
for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.1; done
DISPLAY=":$DISP" openbox --sm-disable >"$WORK/openbox.log" 2>&1 & WM_PID=$!
sleep 1
kill -0 "$XVFB_PID" 2>/dev/null && kill -0 "$WM_PID" 2>/dev/null || { cat "$WORK"/*.log; exit 3; }
export DISPLAY=":$DISP" GDK_BACKEND=x11 GSETTINGS_BACKEND=memory
unset WAYLAND_DISPLAY
export HOME=$WORK/home XDG_DATA_HOME=$WORK/home/data XDG_RUNTIME_DIR=$WORK/run
mkdir -p "$XDG_DATA_HOME" "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
GK_LIBS=""
[ -n "${SOTYWARE_GNOME_KEYRING_ROOT:-}" ] && GK_LIBS=$SOTYWARE_GNOME_KEYRING_ROOT/usr/lib/x86_64-linux-gnu

start_keyring() {
  # --unlock creates and unlocks a "login" collection in the throwaway HOME.
  ( printf 'test-only-keyring-password' | LD_LIBRARY_PATH=$GK_LIBS "$GK_BIN" --foreground --unlock \
      --components=secrets >>"$WORK/gnome-keyring.log" 2>&1 ) &
  wait_owner 0 || { echo "gnome-keyring did not start:"; cat "$WORK/gnome-keyring.log"; exit 3; }
}
stop_keyring() { pkill -f -- "^$GK_BIN " 2>/dev/null; wait_owner 1; }

for phase in "${PHASES[@]}"; do
  echo "== phase $phase"
  case $phase in
    local)
      start_keyring
      python3 "$H/cloud_ui.py" local "$BIN" || rc=1
      stop_keyring ;;
    hang)
      python3 "$H/fake_secret_service.py" & SS_PID=$!
      wait_owner 0 || { echo "fake Secret Service did not start"; exit 3; }
      python3 "$H/cloud_ui.py" hang "$BIN" || rc=1
      kill "$SS_PID"; SS_PID=""; wait_owner 1 ;;
    browse)
      SRV_FILE=$WORK/browse-endpoint
      (cd "$REPO/src-tauri" && HOME=$ORIG_HOME SOTYWARE_UI_SERVER_FILE=$SRV_FILE cargo test --quiet --lib \
        cloud::test_server::serve_listing_for_ui -- --ignored --nocapture >"$WORK/browse-server.log" 2>&1) &
      SRV_PID=$!
      for _ in $(seq 1800); do [ -s "$SRV_FILE" ] && break; kill -0 "$SRV_PID" 2>/dev/null || break; sleep 0.2; done
      [ -s "$SRV_FILE" ] || { echo "listing server did not start:"; cat "$WORK/browse-server.log"; exit 3; }
      start_keyring
      SOTYWARE_UI_ENDPOINT=$(cat "$SRV_FILE") python3 "$H/cloud_ui.py" browse "$BIN" || rc=1
      stop_keyring
      rm -f -- "$SRV_FILE"
      if wait "$SRV_PID"; then
        echo "[PASS] browse: the server saw only listings ($(grep -o 'operations: .*' "$WORK/browse-server.log"))"
      else
        echo "[FAIL] browse: the listing server failed:"; tail -20 "$WORK/browse-server.log"; rc=1
      fi
      SRV_PID="" ;;
    b2)
      before=$(fingerprint)
      [ -n "$before" ] || { echo "[FAIL] b2: could not fingerprint the bucket before the run"; rc=1; continue; }
      echo "  before: ${before#sotyware-it fingerprint }"
      start_keyring
      python3 "$H/cloud_ui.py" b2 "$BIN" || rc=1
      stop_keyring
      after=$(fingerprint)
      echo "  after:  ${after#sotyware-it fingerprint }"
      if [ -n "$after" ] && [ "$before" = "$after" ]; then
        echo "[PASS] b2: every object version under sotyware-it/ is unchanged"
      else
        echo "[FAIL] b2: the bucket changed or could not be read after the run"; rc=1
      fi ;;
    b2-browse)
      before=$(fingerprint)
      [ -n "$before" ] || { echo "[FAIL] b2-browse: could not fingerprint the bucket before the run"; rc=1; continue; }
      echo "  before the dataset: ${before#sotyware-it fingerprint }"
      MANIFEST=$WORK/browse-manifest.json
      (cd "$REPO/src-tauri" && HOME=$ORIG_HOME SOTYWARE_IT_B2_BROWSE_KEEP=1 SOTYWARE_IT_B2_BROWSE_MANIFEST=$MANIFEST \
        cargo test --quiet --lib cloud::integration::browse::backblaze_b2_browse_dataset -- --ignored --nocapture \
        >"$WORK/browse-create.log" 2>&1)
      crc=$?
      BROWSE_RUN=$(grep -oE 'dataset kept; run id [0-9]+-[0-9]+' "$WORK/browse-create.log" | awk '{print $NF}')
      grep -E '^(provider|run prefix|permissions|dataset:|stored:|read-only UI key|local server|checks:|manifest:|FAIL)' \
        "$WORK/browse-create.log" | sed 's/^/  /'
      if [ $crc -ne 0 ] || [ ! -s "$MANIFEST" ]; then
        echo "[FAIL] b2-browse: creating or checking the dataset failed:"; tail -20 "$WORK/browse-create.log"; rc=1
        b2_browse_cleanup; continue
      fi
      echo "[PASS] b2-browse: dataset created and checked through the Rust listing (run $BROWSE_RUN)"
      mid=$(fingerprint)
      start_keyring
      SOTYWARE_UI_MANIFEST=$MANIFEST python3 "$H/cloud_ui.py" b2-browse "$BIN" || rc=1
      stop_keyring
      mid2=$(fingerprint)
      if [ -n "$mid" ] && [ "$mid" = "$mid2" ]; then
        echo "[PASS] b2-browse: browsing changed no object version (${mid#sotyware-it fingerprint })"
      else
        echo "[FAIL] b2-browse: the bucket changed while browsing, or could not be read"; rc=1
      fi
      b2_browse_cleanup
      after=$(fingerprint)
      echo "  after cleanup: ${after#sotyware-it fingerprint }"
      if [ -n "$after" ] && [ "$before" = "$after" ]; then
        echo "[PASS] b2-browse: sotyware-it/ is exactly as before the dataset"
      else
        echo "[FAIL] b2-browse: sotyware-it/ differs from before the dataset"; rc=1
      fi ;;
  esac
done
echo "keyring files left in the throwaway HOME (removed now): $(ls "$XDG_DATA_HOME/keyrings" 2>/dev/null | tr '\n' ' ')"
exit $rc
