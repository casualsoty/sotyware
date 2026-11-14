#!/usr/bin/env node
// The normal release build without the build machine's paths (ADR-028).
//
// Rust embeds source file paths in panic locations (`file!()`, `#[track_caller]`).
// For crates.io dependencies these are absolute, e.g.
// `C:\Users\<name>\.cargo\registry\src\...`, so a plain `tauri build` ships
// the builder's user name. This script runs `tauri build` with
// `--remap-path-prefix` for the home directory, RUSTUP_HOME, CARGO_HOME and
// the checkout (the most specific one wins), passed through
// CARGO_ENCODED_RUSTFLAGS, then checks the release binary with
// `check-release-binary.mjs --local-paths`. The panic locations keep their
// crate-relative part (`cargo/registry/src/index.crates.io-…/tokio-1.54.1/src/…`).
//
//   npm run build:release                      # binary + every bundle for the host
//   npm run build:release -- --bundles msi     # extra arguments go to `tauri build`
//   npm run build:release -- --no-bundle       # binary only
//
// Only for the normal release: `--features`, `--target`, `--target-dir`,
// `--debug` and `--config` are refused (test builds: `npm run
// build:test-profile`). RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS already set are
// kept and the remaps appended; a `[build] rustflags` from a Cargo config
// file is overridden by the environment, so do not rely on one here.
// Only Node built-ins.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PREFIX = "build-release: ";
const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const REFUSED = /^--(features|target|target-dir|debug|config)(=|$)|^-[fdtc]$/;

/**
 * The `--remap-path-prefix` flags, most general first: rustc applies the
 * last matching one, so a CARGO_HOME inside the home directory becomes
 * `cargo/…`, not `~/.cargo/…`.
 * @param {NodeJS.ProcessEnv} env
 * @param {string} home
 * @param {string} checkout
 * @returns {string[]}
 */
export function remapFlags(env = process.env, home = homedir(), checkout = repo) {
  const pairs = [
    [home, "~"],
    [env.RUSTUP_HOME || join(home, ".rustup"), "rustup"],
    [env.CARGO_HOME || join(home, ".cargo"), "cargo"],
    [checkout, "sotyware"],
  ].map(([from, to]) => [resolve(from), to]);
  pairs.sort((a, b) => a[0].length - b[0].length);
  return pairs.map(([from, to]) => `--remap-path-prefix=${from}=${to}`);
}

/**
 * CARGO_ENCODED_RUSTFLAGS with the remaps appended to what the caller set
 * (CARGO_ENCODED_RUSTFLAGS wins over RUSTFLAGS in Cargo, so both are read).
 * @param {NodeJS.ProcessEnv} env
 * @param {string[]} flags
 * @returns {string}
 */
export function encodedRustflags(env, flags) {
  let existing = [];
  if (env.CARGO_ENCODED_RUSTFLAGS) existing = env.CARGO_ENCODED_RUSTFLAGS.split("\x1f");
  else if (env.RUSTFLAGS) existing = env.RUSTFLAGS.split(/\s+/);
  return [...existing.filter(Boolean), ...flags].join("\x1f");
}

/** @param {string[]} args @returns {string | null} the first refused argument */
export function refusedArgument(args) {
  return args.find((a) => REFUSED.test(a)) ?? null;
}

function main(args) {
  const refused = refusedArgument(args);
  if (refused) {
    console.error(`${PREFIX}REFUSED: ${refused} is not for the normal release build (ADR-028)`);
    process.exit(2);
  }
  const flags = remapFlags();
  const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: encodedRustflags(process.env, flags) };
  delete env.RUSTFLAGS;
  console.log(`${PREFIX}${flags.length} path remaps: ${flags.map((f) => f.split("=").pop()).join(", ")}`);
  const tauri = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const r = spawnSync(process.execPath, [tauri, "build", ...args], { cwd: repo, env, stdio: "inherit" });
  if (r.status !== 0) process.exit(r.status ?? 1);
  const exe = process.platform === "win32" ? "sotyware.exe" : "sotyware";
  const targetDir = process.env.CARGO_TARGET_DIR
    ? resolve(repo, "src-tauri", process.env.CARGO_TARGET_DIR)
    : join(repo, "src-tauri", "target");
  const binary = join(targetDir, "release", exe);
  if (!existsSync(binary)) {
    console.error(`${PREFIX}REFUSED: ${binary} was not built`);
    process.exit(1);
  }
  const check = spawnSync(process.execPath, [join(here, "check-release-binary.mjs"), "--local-paths", binary], {
    stdio: "inherit",
  });
  process.exit(check.status ?? 1);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2));
}
