// Self-test of scripts/check-release-binary.mjs (ADR-024): the manifest
// check against the real src-tauri and broken temporary copies of its
// Cargo.toml, the binary scan against temporary files, and the
// before-bundle check with a made-up target directory and command line.
// Nothing in src-tauri is changed.
//
// Run: npm run test:release-binary-guard (node --test). Needs cargo on PATH.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, describe, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const guard = join(here, "check-release-binary.mjs");
const realManifest = resolve(here, "..", "src-tauri", "Cargo.toml");
const MARKER = "sotyware-request-recorder";
const exe = process.platform === "win32" ? "sotyware.exe" : "sotyware";
const tmp = mkdtempSync(join(tmpdir(), "sotyware-release-guard-"));
after(() => rmSync(tmp, { recursive: true, force: true }));

function run(args, env = {}) {
  const r = spawnSync(process.execPath, [guard, ...args], {
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
  return { status: r.status, out: `${r.stdout}${r.stderr}` };
}

/** A copy of the real manifest (without the [patch] section, which needs the vendored crate) with `edit` applied. */
function manifestCopy(name, edit) {
  const dir = join(tmp, name);
  mkdirSync(join(dir, "src"), { recursive: true });
  let text = readFileSync(realManifest, "utf8");
  text = text.replace(/\n\[patch\.crates-io\][^[]*/, "\n");
  text = edit(text);
  writeFileSync(join(dir, "Cargo.toml"), text);
  writeFileSync(join(dir, "src", "lib.rs"), "");
  writeFileSync(join(dir, "src", "main.rs"), "fn main() {}\n");
  writeFileSync(join(dir, "build.rs"), "fn main() {}\n");
  return join(dir, "Cargo.toml");
}

describe("--manifest", () => {
  test("the real manifest passes", () => {
    const r = run(["--manifest", realManifest]);
    assert.equal(r.status, 0, r.out);
  });
  test("a default that turns the recorder on is refused", () => {
    const m = manifestCopy("default-on", (t) =>
      t.replace("[features]\n", '[features]\ndefault = ["request-recorder"]\n'),
    );
    const r = run(["--manifest", m]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /"default" turns on request-recorder/);
  });
  test("another feature that turns it on is refused", () => {
    const m = manifestCopy("other-on", (t) =>
      t.replace("[features]\n", '[features]\nextras = ["request-recorder"]\n'),
    );
    assert.equal(run(["--manifest", m]).status, 1);
  });
  test("a manifest without the feature is refused", () => {
    const m = manifestCopy("missing", (t) => t.replace(/^request-recorder = .*$/m, ""));
    const r = run(["--manifest", m]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /is missing/);
  });
});

describe("binaries", () => {
  const clean = join(tmp, "clean.bin");
  const recorder = join(tmp, "recorder.bin");
  writeFileSync(clean, Buffer.concat([Buffer.alloc(4096, 1), Buffer.from("sotyware-request-record")]));
  writeFileSync(recorder, Buffer.concat([Buffer.alloc(4096, 1), Buffer.from(MARKER), Buffer.alloc(16)]));
  test("a binary without the marker passes", () => {
    assert.equal(run([clean]).status, 0);
  });
  test("a binary with the marker is refused", () => {
    const r = run([clean, recorder]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /request-recorder build/);
  });
  test("a missing binary is refused", () => {
    assert.equal(run([join(tmp, "nope")]).status, 1);
  });
  test("no argument is a usage error", () => {
    assert.equal(run([]).status, 2);
  });
});

describe("--before-bundle", () => {
  function target(name, content) {
    const dir = join(tmp, name);
    mkdirSync(join(dir, "release"), { recursive: true });
    writeFileSync(join(dir, "release", exe), content);
    return dir;
  }
  const realDefault = join(resolve(here, "..", "src-tauri", "target", "release"), exe);
  let defaultIsRecorder = false;
  try {
    defaultIsRecorder = readFileSync(realDefault).includes(Buffer.from(MARKER));
  } catch {
    // no normal release binary here
  }
  test("a command line asking for the feature is refused", () => {
    const t = target("t-clean-1", "normal");
    const r = run(["--before-bundle"], {
      CARGO_TARGET_DIR: t,
      SOTYWARE_GUARD_EXTRA_CMDLINE: "node tauri build --features request-recorder -- --target-dir target/recorder",
    });
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /asks for the request-recorder feature/);
  });
  test("a command line asking for all features is refused", () => {
    const t = target("t-clean-2", "normal");
    const r = run(["--before-bundle"], {
      CARGO_TARGET_DIR: t,
      SOTYWARE_GUARD_EXTRA_CMDLINE: "cargo build --release --all-features",
    });
    assert.equal(r.status, 1, r.out);
  });
  test("a recorder binary in the target directory is refused", () => {
    const t = target("t-recorder", `xx${MARKER}xx`);
    const r = run(["--before-bundle"], { CARGO_TARGET_DIR: t });
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /must not be packaged/);
  });
  test("a recorder binary named by --target-dir on the command line is refused", () => {
    const t = target("t-recorder-arg", `xx${MARKER}xx`);
    const r = run(["--before-bundle"], {
      SOTYWARE_GUARD_EXTRA_CMDLINE: `node tauri build -- --target-dir ${t}`,
    });
    assert.equal(r.status, 1, r.out);
  });
  test("a normal binary passes", { skip: defaultIsRecorder && "the real target/release binary is a recorder build" }, () => {
    const t = target("t-normal", "a normal binary");
    const r = run(["--before-bundle"], { CARGO_TARGET_DIR: t });
    assert.equal(r.status, 0, r.out);
  });
});
