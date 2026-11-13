#!/usr/bin/env node
// Guard for the locally patched rfd (ADR-023, src-tauri/patches/README.md).
//
// Cargo only *warns* when a `[patch.crates-io]` entry stops matching (for
// example after a tauri-plugin-dialog update that requires another rfd
// version) and then builds the unpatched crate from crates.io, which brings
// back the Windows abort on dialog paths with an unpaired surrogate. This
// script fails instead. It asks Cargo for the resolved dependency graph
// (`cargo metadata --locked`, structured JSON, not warning text) and checks:
//
// - the patch directory and the patched source file exist and still contain
//   the lossless conversion;
// - rfd is in the resolved graph exactly once, at EXPECTED_VERSION, as a path
//   package (no registry source) whose manifest is the one in
//   src-tauri/patches/rfd;
// - Cargo.lock records no unused rfd patch and Cargo printed no
//   "was not used" warning (belt and braces: the graph check alone fails).
//
// Usage: node scripts/check-rfd-patch.mjs [--manifest-path <Cargo.toml>]
// The default manifest is src-tauri/Cargo.toml next to this script, so the
// working directory does not matter. Exit 0: patch applied; 1: not applied
// or not checkable; 2: wrong usage. Needs `cargo` on PATH and the
// dependencies fetched (or fetchable): `cargo metadata` reads every
// package's manifest. Never changes Cargo.lock (`--locked`).

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The upstream version the patch was made against. Change only together with the patch. */
const EXPECTED_VERSION = "0.17.2";
/** The patched copy, relative to the directory of the checked Cargo.toml. */
const PATCH_DIR = join("patches", "rfd");
/** The one changed file, relative to PATCH_DIR. */
const PATCHED_FILE = join("src", "backend", "win_cid", "file_dialog", "com.rs");
/** Text the patched file must contain, and text it must not (the upstream defect). */
const REQUIRED_SNIPPETS = ["unsafe fn read_to_os_string(ptr: *const u16) -> OsString", "OsString::from_wide(slice)"];
const FORBIDDEN_SNIPPETS = ["String::from_utf16(slice).unwrap()"];
const HOW_TO_FIX = "See src-tauri/patches/README.md (\"Updating or removing the patch\").";

const PREFIX = "check-rfd-patch: ";

/**
 * Compares two paths of existing files or directories after resolving
 * symlinks; case-insensitively on Windows, whose file system usually is.
 * @param {string} a
 * @param {string} b
 * @returns {boolean}
 */
function samePath(a, b) {
  const ra = realpathSync(a);
  const rb = realpathSync(b);
  return process.platform === "win32" ? ra.toLowerCase() === rb.toLowerCase() : ra === rb;
}

/**
 * @param {string[]} argv
 * @returns {string} absolute path of the Cargo.toml to check
 */
function parseArgs(argv) {
  if (argv.length === 0) {
    return resolve(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "Cargo.toml");
  }
  if (argv.length === 2 && argv[0] === "--manifest-path" && argv[1]) {
    return resolve(argv[1]);
  }
  console.error(`${PREFIX}usage: node scripts/check-rfd-patch.mjs [--manifest-path <Cargo.toml>]`);
  process.exit(2);
}

/**
 * Checks the patch files on disk. Runs before Cargo so a missing copy gets a
 * clear message rather than Cargo's path error.
 * @param {string} patchDir
 * @returns {string[]} problems found
 */
function checkFiles(patchDir) {
  if (!existsSync(join(patchDir, "Cargo.toml"))) {
    return [`the patched copy is missing: ${join(patchDir, "Cargo.toml")} does not exist.`];
  }
  const file = join(patchDir, PATCHED_FILE);
  if (!existsSync(file)) {
    return [`the patched source file is missing: ${file}.`];
  }
  const text = readFileSync(file, "utf8");
  const problems = [];
  for (const s of REQUIRED_SNIPPETS) {
    if (!text.includes(s)) problems.push(`${file} no longer contains \`${s}\`: the lossless path conversion is gone.`);
  }
  for (const s of FORBIDDEN_SNIPPETS) {
    if (text.includes(s)) problems.push(`${file} contains \`${s}\` again: the upstream abort is back.`);
  }
  return problems;
}

/**
 * Asks Cargo for the resolved graph and checks where rfd comes from.
 * @param {string} manifest
 * @param {string} patchDir
 * @returns {string[]} problems found
 */
function checkResolution(manifest, patchDir) {
  const run = spawnSync(
    "cargo",
    ["metadata", "--format-version", "1", "--locked", "--manifest-path", manifest],
    { cwd: dirname(manifest), encoding: "utf8", maxBuffer: 512 * 1024 * 1024 },
  );
  if (run.error) {
    return [`could not run cargo: ${run.error.message}. Is cargo on PATH?`];
  }
  if (run.status !== 0) {
    return [
      `\`cargo metadata --locked\` failed (exit ${run.status}). If Cargo.lock is out of date, ` +
        `update it deliberately and run this check again.\n${run.stderr.trim()}`,
    ];
  }

  const problems = [];
  if (/patch[^\n]*rfd[^\n]*was not used/i.test(run.stderr)) {
    problems.push("Cargo reports that the rfd patch was not used.");
  }

  /** @type {{packages: {id: string, name: string, version: string, source: string | null, manifest_path: string}[], resolve: {nodes: {id: string}[]} | null}} */
  const meta = JSON.parse(run.stdout);
  if (!meta.resolve) {
    return [...problems, "cargo metadata returned no resolved graph."];
  }
  const resolved = new Set(meta.resolve.nodes.map((n) => n.id));
  const rfds = meta.packages.filter((p) => p.name === "rfd" && resolved.has(p.id));

  if (rfds.length === 0) {
    problems.push(
      "rfd is no longer in the dependency graph. If tauri-plugin-dialog dropped it, remove the patch " +
        "and this check together.",
    );
  } else if (rfds.length > 1) {
    problems.push(
      `rfd is resolved ${rfds.length} times (${rfds.map((p) => p.id).join(", ")}); ` +
        "every copy except the patched one is unpatched.",
    );
  }
  for (const p of rfds) {
    if (p.version !== EXPECTED_VERSION) {
      problems.push(`resolved rfd is ${p.version} (${p.id}), expected ${EXPECTED_VERSION}.`);
    }
    if (p.source !== null) {
      problems.push(`resolved rfd comes from ${p.source} (${p.id}), not from the local patch: it is unpatched.`);
    }
    const dir = dirname(p.manifest_path);
    if (!existsSync(dir) || !samePath(dir, patchDir)) {
      problems.push(`resolved rfd is built from ${dir}, not from ${patchDir}.`);
    }
  }

  const lockPath = join(dirname(manifest), "Cargo.lock");
  if (existsSync(lockPath)) {
    const lock = readFileSync(lockPath, "utf8");
    const unused = lock.split(/^\[\[patch\.unused\]\]$/m).slice(1);
    if (unused.some((section) => /^name = "rfd"$/m.test(section.split(/^\[/m)[0] ?? ""))) {
      problems.push("Cargo.lock lists the rfd patch under [[patch.unused]].");
    }
  }
  return problems;
}

function main() {
  const manifest = parseArgs(process.argv.slice(2));
  if (!existsSync(manifest)) {
    console.error(`${PREFIX}FAIL: ${manifest} does not exist.`);
    process.exit(1);
  }
  const patchDir = join(dirname(manifest), PATCH_DIR);
  let problems = checkFiles(patchDir);
  if (problems.length === 0) problems = checkResolution(manifest, patchDir);

  if (problems.length > 0) {
    console.error(`${PREFIX}FAIL: the patched rfd ${EXPECTED_VERSION} is not what Cargo builds.`);
    for (const p of problems) console.error(`  - ${p}`);
    console.error(`  ${HOW_TO_FIX}`);
    process.exit(1);
  }
  console.log(`${PREFIX}ok: rfd ${EXPECTED_VERSION} resolves to the local patch (${patchDir}).`);
}

main();
