#!/usr/bin/env bash
# Assembles a sotyware release from the build jobs' artifacts (ADR-036,
# ADR-039, docs/development/release.md#publishing-a-release): picks the
# files, signs every updater package and latest.json with `tauri signer sign
# --app-version`, then, with the private key removed from the environment,
# verifies all of it with the public key alone and writes SHA256SUMS.
#
# release.yml runs it twice over: its publish job with the release key
# (environment `release`), its rehearse job (a dry run) with a throwaway key
# made for that run. It never prints a key.
#
#   scripts/release-assemble.sh <built> <out>
#
# <built> holds the artifacts as actions/download-artifact writes them:
#   windows-installers/  sotyware_<v>_x64-setup.exe, sotyware_<v>_x64_en-US.msi
#   linux-packages/      sotyware_<v>_amd64.AppImage, sotyware_<v>_amd64.deb,
#                        sotyware-<v>-1.x86_64.rpm
#   macos-aarch64/, macos-x86_64/
#                        sotyware_<v>_<aarch64|x64>.app.tar.gz and .dmg, and
#                        status.txt (signed=, notarized=)
# Windows and Linux files are required. The Mac files are included only when
# both architectures were signed AND notarized; otherwise macOS is left out
# of the release and of latest.json, so an unsigned Mac build is never
# offered.
#
# Environment: VERSION; BASE, the release's download folder
# (https://github.com/<owner>/<repo>/releases/download/<tag>/); PUBKEY, the
# public key (file or base64 line) the signatures must verify with; NOTES,
# a release notes file; TAURI_SIGNING_PRIVATE_KEY and
# TAURI_SIGNING_PRIVATE_KEY_PASSWORD. Writes `kinds=` and `macos=` to
# $GITHUB_OUTPUT when that is set, and always to stdout.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <built> <out>" >&2
  exit 2
fi
in="$1"
out="$2"
: "${VERSION:?VERSION is required}" "${BASE:?BASE is required}" "${PUBKEY:?PUBKEY is required}" "${NOTES:?NOTES is required}"
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  echo "release-assemble: TAURI_SIGNING_PRIVATE_KEY is not set" >&2
  exit 1
fi
here="$(cd "$(dirname "$0")" && pwd)"
manifest_tool="$here/update-manifest.mjs"
v="$VERSION"

mkdir -p "$out"
if [ -n "$(ls -A "$out")" ]; then
  echo "release-assemble: $out is not empty" >&2
  exit 1
fi

# Every release: the Windows installers and the Linux packages.
cp "$in/windows-installers/sotyware_${v}_x64-setup.exe" "$in/windows-installers/sotyware_${v}_x64_en-US.msi" "$out/"
cp "$in/linux-packages/sotyware_${v}_amd64.AppImage" "$in/linux-packages/sotyware_${v}_amd64.deb" "$in/linux-packages/sotyware-${v}-1.x86_64.rpm" "$out/"
kinds="nsis,msi,appimage,deb,rpm"

# macOS only when both architectures are signed and notarized.
ready() {
  grep -qsx 'signed=true' "$1/status.txt" && grep -qsx 'notarized=true' "$1/status.txt"
}
if ready "$in/macos-aarch64" && ready "$in/macos-x86_64"; then
  cp "$in/macos-aarch64/sotyware_${v}_aarch64.app.tar.gz" "$in/macos-aarch64/sotyware_${v}_aarch64.dmg" "$out/"
  cp "$in/macos-x86_64/sotyware_${v}_x64.app.tar.gz" "$in/macos-x86_64/sotyware_${v}_x64.dmg" "$out/"
  kinds="$kinds,app-aarch64,app-x64"
  macos="included: Apple Silicon and Intel, signed and notarized"
else
  macos="not included: the builds were not signed and notarized"
fi

# Sign each updater package for exactly this version, then the manifest
# (create checks every package signature with PUBKEY first).
tauri() { npx --no-install tauri "$@"; }
args=()
for kind in ${kinds//,/ }; do
  case "$kind" in
    nsis) f="sotyware_${v}_x64-setup.exe" ;;
    msi) f="sotyware_${v}_x64_en-US.msi" ;;
    appimage) f="sotyware_${v}_amd64.AppImage" ;;
    deb) f="sotyware_${v}_amd64.deb" ;;
    rpm) f="sotyware-${v}-1.x86_64.rpm" ;;
    app-aarch64) f="sotyware_${v}_aarch64.app.tar.gz" ;;
    app-x64) f="sotyware_${v}_x64.app.tar.gz" ;;
    *) echo "release-assemble: unknown kind $kind" >&2; exit 1 ;;
  esac
  tauri signer sign --app-version "$v" "$out/$f" > /dev/null
  args+=("--$kind" "$out/$f")
done
node "$manifest_tool" create --version "$v" --base-url "$BASE" --pubkey "$PUBKEY" --notes-file "$NOTES" --out "$out/latest.json" "${args[@]}"
tauri signer sign --app-version "$v" "$out/latest.json" > /dev/null

# From here on, the public key alone.
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD TAURI_SIGNING_PRIVATE_KEY_PATH
node "$manifest_tool" verify --manifest "$out/latest.json" --pubkey "$PUBKEY" --artifacts "$out" --base-url "$BASE" --require "$kinds"
(cd "$out" && sha256sum -- * > SHA256SUMS && sha256sum -c --quiet SHA256SUMS)

echo "kinds=$kinds"
echo "macos=$macos"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "kinds=$kinds"
    echo "macos=$macos"
  } >> "$GITHUB_OUTPUT"
fi
ls -l "$out"
cat "$out/SHA256SUMS"
