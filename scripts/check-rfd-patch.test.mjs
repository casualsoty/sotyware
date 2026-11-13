// Self-test of scripts/check-rfd-patch.mjs: runs the guard against the real
// src-tauri and against broken temporary copies of it. The real manifest,
// lockfile and patch are only read, never changed.
//
// Run: npm run test:rfd-patch-guard (node --test). Needs cargo on PATH; the
// broken copies re-resolve rfd from crates.io, so the registry index and the
// rfd crates must be cached or reachable (no other network access).

import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, before, describe, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const guard = join(here, "check-rfd-patch.mjs");
const srcTauri = resolve(here, "..", "src-tauri");

/** Runs the guard, optionally against another manifest. */
function runGuard(manifest) {
  const args = manifest ? [guard, "--manifest-path", manifest] : [guard];
  const r = spawnSync(process.execPath, args, { encoding: "utf8", cwd: tmpdir() });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}

/** Lets Cargo rewrite the copy's Cargo.lock for its changed manifest (the guard itself never does). */
function relock(dir) {
  const r = spawnSync("cargo", ["metadata", "--format-version", "1", "--manifest-path", join(dir, "Cargo.toml")], {
    encoding: "utf8",
    maxBuffer: 512 * 1024 * 1024,
  });
  assert.equal(r.status, 0, `cargo could not re-resolve the copy:\n${r.stderr}`);
  return r.stderr;
}

function edit(file, from, to) {
  const text = readFileSync(file, "utf8");
  assert.ok(text.includes(from), `${file} does not contain ${from}`);
  writeFileSync(file, text.replace(from, to));
}

const RFD_VERSION_LINE = 'version = "0.17.2"';

describe("check-rfd-patch", () => {
  let root;
  let n = 0;

  /** A fresh copy of what cargo metadata needs from src-tauri (no target/, no gen/). */
  function copy() {
    const dir = join(root, `case-${++n}`);
    for (const p of ["Cargo.toml", "Cargo.lock", "build.rs", "src", "patches"]) {
      cpSync(join(srcTauri, p), join(dir, p), {
        recursive: true,
        filter: (src) => !/[\\/]patches[\\/]rfd[\\/]target([\\/]|$)/.test(src),
      });
    }
    return dir;
  }

  before(() => {
    root = mkdtempSync(join(tmpdir(), "sotyware-rfd-guard-"));
  });
  after(() => {
    rmSync(root, { recursive: true, force: true });
  });

  test("passes for the real src-tauri, from any working directory", () => {
    const r = runGuard();
    assert.equal(r.status, 0, r.out);
    assert.match(r.out, /ok: rfd 0\.17\.2 resolves to the local patch/);
  });

  test("passes for an unchanged copy", () => {
    const r = runGuard(join(copy(), "Cargo.toml"));
    assert.equal(r.status, 0, r.out);
  });

  test("fails when the patch is unused because its version no longer matches", () => {
    const dir = copy();
    // tauri-plugin-dialog requires rfd ^0.17: a 0.18.0 copy cannot be used,
    // Cargo warns and resolves the unpatched 0.17.2 from crates.io.
    edit(join(dir, "patches", "rfd", "Cargo.toml"), RFD_VERSION_LINE, 'version = "0.18.0"');
    assert.match(relock(dir), /was not used/, "cargo should warn about the unused patch");
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /comes from registry\+https:\/\/github\.com\/rust-lang\/crates\.io-index/);
    assert.match(r.out, /\[\[patch\.unused\]\]/);
  });

  test("fails when the [patch.crates-io] entry is removed", () => {
    const dir = copy();
    edit(join(dir, "Cargo.toml"), 'rfd = { path = "patches/rfd" }', "");
    relock(dir);
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /not from the local patch: it is unpatched/);
  });

  test("fails when the patch resolves to another version", () => {
    const dir = copy();
    // 0.17.3 still satisfies ^0.17, so Cargo uses the patch, at the wrong version.
    edit(join(dir, "patches", "rfd", "Cargo.toml"), RFD_VERSION_LINE, 'version = "0.17.3"');
    relock(dir);
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /resolved rfd is 0\.17\.3 .*expected 0\.17\.2/);
  });

  test("fails when Cargo.lock does not match the manifest", () => {
    const dir = copy();
    edit(join(dir, "patches", "rfd", "Cargo.toml"), RFD_VERSION_LINE, 'version = "0.17.3"');
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /cargo metadata --locked` failed/);
  });

  test("fails when the patch directory is missing", () => {
    const dir = copy();
    rmSync(join(dir, "patches", "rfd"), { recursive: true });
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /the patched copy is missing/);
  });

  test("fails when the patched source file is missing", () => {
    const dir = copy();
    rmSync(join(dir, "patches", "rfd", "src", "backend", "win_cid", "file_dialog", "com.rs"));
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /the patched source file is missing/);
  });

  test("fails when the patched conversion is reverted", () => {
    const dir = copy();
    edit(
      join(dir, "patches", "rfd", "src", "backend", "win_cid", "file_dialog", "com.rs"),
      "OsString::from_wide(slice)",
      "String::from_utf16(slice).unwrap()",
    );
    const r = runGuard(join(dir, "Cargo.toml"));
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /lossless path conversion is gone/);
    assert.match(r.out, /upstream abort is back/);
  });

  test("rejects wrong usage", () => {
    const r = spawnSync(process.execPath, [guard, "--bogus"], { encoding: "utf8" });
    assert.equal(r.status, 2);
  });
});
