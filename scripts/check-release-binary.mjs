#!/usr/bin/env node
// Guard against shipping a test build: the request recorder (ADR-024) or
// a test profile (ADR-027).
//
// The Cargo feature `request-recorder` compiles an opt-in recorder of the
// app's S3 requests into the binary; `test-profile` makes the app run only
// inside a test profile (and `request-recorder` turns it on as well). Both
// are for desktop and integration test runs only and must never be
// packaged. build.rs already refuses them unless the target directory is
// named `recorder` (or `test-profile`); this script adds three checks:
//
//   --manifest            both features exist and no default (and no other
//                         feature except these two) turns one on. In
//                         test:headless and CI.
//   <binary>...           none of these binaries contains the recorder's
//                         or the test profile's marker.
//   --before-bundle       Tauri's beforeBundleCommand (tauri.conf.json):
//                         refuses to package when the `tauri build` command
//                         line (found among this process's ancestors) asks
//                         for the feature or for all features, or when the
//                         binary about to be bundled contains a marker.
//                         If the command line cannot be read, every
//                         candidate binary under the target directory that
//                         is at least as new as the normal one is checked
//                         (conservative: it may refuse a normal build next
//                         to a newer test build; remove
//                         src-tauri/target/recorder or target/test-profile
//                         then).
//
// A recorder binary that a bundler patched anyway refuses to record at
// runtime (src-tauri/src/cloud/recorder.rs, `bundle_type()`).
//
// Exit 0: fine; 1: refused or not checkable; 2: wrong usage. Only Node
// built-ins; `cargo` on PATH for --manifest and --before-bundle.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PREFIX = "check-release-binary: ";
/** The test-only features, each with the string its binaries contain. */
export const GUARDED = {
  /** The recorder's interceptor name (`MARKER` in recorder.rs), ADR-024. */
  "request-recorder": "sotyware-request-recorder",
  /** `BUILD_MARKER` in test_profile.rs, ADR-027. */
  "test-profile": "sotyware-test-profile-build",
};
const FEATURES = Object.keys(GUARDED);
/** Kept for the tests and older callers: the recorder's marker. */
export const MARKER = GUARDED["request-recorder"];
/** Where each feature's builds go (build.rs). */
const TEST_TARGETS = ["recorder", "test-profile"];
const here = dirname(fileURLToPath(import.meta.url));
const srcTauri = resolve(here, "..", "src-tauri");
const exe = process.platform === "win32" ? "sotyware.exe" : "sotyware";

function fail(msg) {
  console.error(`${PREFIX}REFUSED: ${msg}`);
  process.exit(1);
}

/**
 * @param {string} path
 * @returns {string | null} the guarded feature whose marker the file holds
 */
export function markerIn(path) {
  const data = readFileSync(path);
  for (const [feature, marker] of Object.entries(GUARDED)) {
    if (data.includes(Buffer.from(marker, "latin1"))) return feature;
  }
  return null;
}

/** @param {string} path @returns {boolean} */
export function containsMarker(path) {
  return markerIn(path) !== null;
}

/** @param {string} manifest */
function checkManifest(manifest) {
  const r = spawnSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1", "--manifest-path", manifest],
    { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
  );
  if (r.status !== 0) fail(`cargo metadata failed:\n${r.stderr}`);
  const pkg = JSON.parse(r.stdout).packages.find((p) => p.name === "sotyware");
  if (!pkg) fail("package sotyware not found");
  const features = pkg.features ?? {};
  for (const feature of FEATURES) {
    if (!(feature in features)) fail(`the feature ${feature} is missing from ${manifest}`);
  }
  for (const [name, enables] of Object.entries(features)) {
    // One test feature may turn on the other; nothing else may.
    if (FEATURES.includes(name)) continue;
    for (const feature of FEATURES) {
      if (enables.some((e) => e === feature || e.endsWith(`/${feature}`))) {
        fail(`feature "${name}" turns on ${feature}; it must stay off by default (ADR-024, ADR-027)`);
      }
    }
  }
  console.log(`${PREFIX}ok: ${FEATURES.join(" and ")} exist and are off by default`);
}

/** Programs whose command line can carry the build's features. */
const BUILDERS = /^(node|nodejs|npm|npx|tauri|cargo|cargo-tauri)(\.exe|\.cmd)?$/i;

/** @param {string} line @returns {string} the program's file name */
function program(line) {
  const first = line.trim().match(/^"([^"]+)"|^(\S+)/);
  const path = first ? (first[1] ?? first[2]) : "";
  return path.split(/[\\/]/).pop() ?? "";
}

/**
 * Command lines of this process's ancestors that are a build tool (node,
 * npm, tauri, cargo), or null if the ancestors cannot be read.
 */
function ancestorCommandLines() {
  const lines = [];
  if (process.env.SOTYWARE_GUARD_EXTRA_CMDLINE) {
    // Test hook: only adds a command line to check, never removes one.
    lines.push(process.env.SOTYWARE_GUARD_EXTRA_CMDLINE);
  }
  try {
    if (process.platform === "linux") {
      let pid = process.ppid;
      for (let i = 0; i < 16 && pid > 1; i++) {
        const argv = readFileSync(`/proc/${pid}/cmdline`, "utf8").split("\0").filter(Boolean);
        // One token per argument: quote the program so `program()` keeps spaces.
        if (argv.length > 0) lines.push([`"${argv[0]}"`, ...argv.slice(1)].join(" "));
        const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
        pid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      }
    } else if (process.platform === "win32") {
      const script =
        "$p=" +
        process.ppid +
        ";for($i=0;$i -lt 16 -and $p;$i++){$x=Get-CimInstance Win32_Process -Filter \"ProcessId=$p\";if(-not $x){break};$x.CommandLine;$p=$x.ParentProcessId}";
      const r = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], {
        encoding: "utf8",
      });
      if (r.status !== 0) return null;
      lines.push(...r.stdout.split(/\r?\n/).filter(Boolean));
    } else {
      return null;
    }
  } catch {
    return null;
  }
  return lines.filter((l) => BUILDERS.test(program(l)));
}

/** @param {string} line @returns {string | null} the --target-dir value */
function targetDirOf(line) {
  const m = line.match(/--target-dir(?:=|\s+)("[^"]+"|\S+)/);
  return m ? m[1].replace(/^"|"$/g, "") : null;
}

function binariesIn(targetDir, profile) {
  const out = [join(targetDir, profile, exe)];
  if (process.env.TAURI_ENV_TARGET_TRIPLE) {
    out.push(join(targetDir, process.env.TAURI_ENV_TARGET_TRIPLE, profile, exe));
  }
  return out.filter((p) => existsSync(p));
}

function beforeBundle() {
  const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
  const defaultTarget = join(srcTauri, "target");
  const lines = ancestorCommandLines();
  const targets = new Set([defaultTarget]);
  if (process.env.CARGO_TARGET_DIR) targets.add(resolve(srcTauri, process.env.CARGO_TARGET_DIR));
  if (lines) {
    for (const line of lines) {
      for (const feature of FEATURES) {
        if (line.includes(feature)) {
          fail(`the build command asks for the ${feature} feature; test builds are never packaged (ADR-024, ADR-027): ${line}`);
        }
      }
      if (/--all-features/.test(line)) {
        fail(`the build command asks for all features; test builds are never packaged (ADR-024, ADR-027): ${line}`);
      }
      const t = targetDirOf(line);
      if (t) targets.add(resolve(srcTauri, t));
    }
  } else {
    // Unknown command line: also check test builds at least as new as the
    // normal binary.
    const normal = binariesIn(defaultTarget, profile)[0];
    const since = normal ? statSync(normal).mtimeMs : 0;
    for (const dir of TEST_TARGETS) {
      for (const b of binariesIn(join(defaultTarget, dir), profile)) {
        if (statSync(b).mtimeMs >= since) targets.add(join(defaultTarget, dir));
      }
    }
  }
  const checked = [];
  for (const t of targets) {
    for (const b of binariesIn(t, profile)) {
      const found = markerIn(b);
      if (found) fail(`${b} is a ${found} build and must not be packaged (ADR-024, ADR-027)`);
      checked.push(b);
    }
  }
  if (checked.length === 0) fail(`no ${profile} binary found to check`);
  console.log(`${PREFIX}ok: no test build in ${checked.join(", ")}`);
}

function main(argv) {
  if (argv[0] === "--manifest") {
    checkManifest(argv[1] ?? join(srcTauri, "Cargo.toml"));
  } else if (argv[0] === "--before-bundle" && argv.length === 1) {
    beforeBundle();
  } else if (argv.length > 0 && !argv[0].startsWith("--")) {
    for (const b of argv) {
      if (!existsSync(b)) fail(`${b} does not exist`);
      const found = markerIn(b);
      if (found) fail(`${b} is a ${found} build (ADR-024, ADR-027)`);
    }
    console.log(`${PREFIX}ok: no test build in ${argv.join(", ")}`);
  } else {
    console.error(
      "usage: check-release-binary.mjs --manifest [Cargo.toml] | --before-bundle | <binary>...",
    );
    process.exit(2);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
