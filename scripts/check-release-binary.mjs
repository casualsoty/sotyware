#!/usr/bin/env node
// Guard against shipping the request recorder (ADR-024).
//
// The Cargo feature `request-recorder` compiles an opt-in recorder of the
// app's S3 requests into the binary. It is for controlled integration runs
// only and must never be packaged. build.rs already refuses the feature
// unless the target directory is named `recorder`; this script adds three
// checks:
//
//   --manifest            the feature exists and no default (and no other
//                         feature) turns it on. In test:headless and CI.
//   <binary>...           none of these binaries contains the recorder's
//                         marker. CI runs it on the release binary the
//                         desktop workflows test.
//   --before-bundle       Tauri's beforeBundleCommand (tauri.conf.json):
//                         refuses to package when the `tauri build` command
//                         line (found among this process's ancestors) asks
//                         for the feature or for all features, or when the
//                         binary about to be bundled contains the marker.
//                         If the command line cannot be read, every
//                         candidate binary under the target directory that
//                         is at least as new as the normal one is checked
//                         (conservative: it may refuse a normal build next
//                         to a newer recorder build; remove
//                         src-tauri/target/recorder then).
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
const FEATURE = "request-recorder";
/** The recorder's interceptor name (`MARKER` in recorder.rs). */
export const MARKER = "sotyware-request-recorder";
const here = dirname(fileURLToPath(import.meta.url));
const srcTauri = resolve(here, "..", "src-tauri");
const exe = process.platform === "win32" ? "sotyware.exe" : "sotyware";

function fail(msg) {
  console.error(`${PREFIX}REFUSED: ${msg}`);
  process.exit(1);
}

/** @param {string} path @returns {boolean} */
export function containsMarker(path) {
  return readFileSync(path).includes(Buffer.from(MARKER, "latin1"));
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
  if (!(FEATURE in features)) fail(`the feature ${FEATURE} is missing from ${manifest}`);
  for (const [name, enables] of Object.entries(features)) {
    if (name !== FEATURE && enables.some((e) => e === FEATURE || e.endsWith(`/${FEATURE}`))) {
      fail(`feature "${name}" turns on ${FEATURE}; it must stay off by default (ADR-024)`);
    }
  }
  console.log(`${PREFIX}ok: ${FEATURE} exists and is off by default`);
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
      if (line.includes(FEATURE) || /--all-features/.test(line)) {
        fail(`the build command asks for the ${FEATURE} feature; recorder builds are never packaged (ADR-024): ${line}`);
      }
      const t = targetDirOf(line);
      if (t) targets.add(resolve(srcTauri, t));
    }
  } else {
    // Unknown command line: also check recorder builds at least as new as
    // the normal binary.
    const normal = binariesIn(defaultTarget, profile)[0];
    const since = normal ? statSync(normal).mtimeMs : 0;
    for (const b of binariesIn(join(defaultTarget, "recorder"), profile)) {
      if (statSync(b).mtimeMs >= since) targets.add(join(defaultTarget, "recorder"));
    }
  }
  const checked = [];
  for (const t of targets) {
    for (const b of binariesIn(t, profile)) {
      if (containsMarker(b)) fail(`${b} is a request-recorder build and must not be packaged (ADR-024)`);
      checked.push(b);
    }
  }
  if (checked.length === 0) fail(`no ${profile} binary found to check`);
  console.log(`${PREFIX}ok: no recorder in ${checked.join(", ")}`);
}

function main(argv) {
  if (argv[0] === "--manifest") {
    checkManifest(argv[1] ?? join(srcTauri, "Cargo.toml"));
  } else if (argv[0] === "--before-bundle" && argv.length === 1) {
    beforeBundle();
  } else if (argv.length > 0 && !argv[0].startsWith("--")) {
    for (const b of argv) {
      if (!existsSync(b)) fail(`${b} does not exist`);
      if (containsMarker(b)) fail(`${b} is a request-recorder build (ADR-024)`);
    }
    console.log(`${PREFIX}ok: no recorder in ${argv.join(", ")}`);
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
