// Self-test of scripts/check-release-binary.mjs (ADR-024, ADR-027): the manifest
// check against the real src-tauri and broken temporary copies of its
// Cargo.toml, the binary scan against temporary files, and the
// before-bundle check with a made-up target directory and command line.
// Nothing in src-tauri is changed.
//
// Run: npm run test:release-binary-guard (node --test). Needs cargo on PATH.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, describe, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const guard = join(here, "check-release-binary.mjs");
const realManifest = resolve(here, "..", "src-tauri", "Cargo.toml");
const MARKER = "sotyware-request-recorder";
const PROFILE_MARKER = "sotyware-test-profile-build";
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
  // LF first: Git on Windows (core.autocrlf) checks the manifest out with
  // CRLF, and the edits below look for "\n".
  let text = readFileSync(realManifest, "utf8").replace(/\r\n/g, "\n");
  text = text.replace(/\n\[patch\.crates-io\][^[]*/, "\n");
  const edited = edit(text);
  // An edit that matches nothing would test the unchanged manifest.
  assert.notEqual(edited, text, `the edit for ${name} changed nothing`);
  text = edited;
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
  test("a default that turns the test profile on is refused", () => {
    const m = manifestCopy("default-profile", (t) =>
      t.replace("[features]\n", '[features]\ndefault = ["test-profile"]\n'),
    );
    const r = run(["--manifest", m]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /"default" turns on test-profile/);
  });
  test("another feature that turns the test profile on is refused", () => {
    const m = manifestCopy("other-profile", (t) =>
      t.replace("[features]\n", '[features]\nextras = ["test-profile"]\n'),
    );
    assert.equal(run(["--manifest", m]).status, 1);
  });
  test("a manifest without the test-profile feature is refused", () => {
    const m = manifestCopy("missing-profile", (t) =>
      t.replace(/^test-profile = .*$/m, "").replace('["test-profile", ', "["),
    );
    const r = run(["--manifest", m]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /test-profile is missing/);
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
  test("a test-profile binary is refused", () => {
    const profile = join(tmp, "profile.bin");
    writeFileSync(profile, Buffer.concat([Buffer.alloc(64, 1), Buffer.from(PROFILE_MARKER)]));
    const r = run([clean, profile]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /test-profile build/);
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
    const real = readFileSync(realDefault);
    defaultIsRecorder = real.includes(Buffer.from(MARKER)) || real.includes(Buffer.from(PROFILE_MARKER));
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
  test("a command line asking for the test profile is refused", () => {
    const t = target("t-clean-3", "normal");
    const r = run(["--before-bundle"], {
      CARGO_TARGET_DIR: t,
      SOTYWARE_GUARD_EXTRA_CMDLINE: "node tauri build --features test-profile -- --target-dir target/test-profile",
    });
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /asks for the test-profile feature/);
  });
  test("a test-profile binary in the target directory is refused", () => {
    const t = target("t-profile", `xx${PROFILE_MARKER}xx`);
    const r = run(["--before-bundle"], { CARGO_TARGET_DIR: t });
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /test-profile build and must not be packaged/);
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
  test("a bundled binary that names the cargo registry is refused (ADR-028)", {
    skip: defaultIsRecorder && "the real target/release binary is a recorder build",
  }, () => {
    const t = target("t-paths", `panic at ${join(homedir(), ".cargo", "registry", "src", "x.rs")}`);
    const r = run(["--before-bundle"], { CARGO_TARGET_DIR: t });
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /npm run build:release/);
  });
});

describe("--local-paths (ADR-028)", () => {
  function bin(name, content) {
    const p = join(tmp, name);
    writeFileSync(p, Buffer.concat([Buffer.alloc(256, 0), Buffer.from(content, "latin1"), Buffer.alloc(16)]));
    return p;
  }
  const home = homedir();
  const checkout = resolve(here, "..");
  test("a remapped binary passes", () => {
    const b = bin("remapped.bin", "cargo/registry/src/index.crates.io-1/tokio-1/src/lib.rs sotyware/src-tauri/target");
    const r = run(["--local-paths", b]);
    assert.equal(r.status, 0, r.out);
  });
  test("the home directory is refused", () => {
    const r = run(["--local-paths", bin("home.bin", `${home}/x`)]);
    assert.equal(r.status, 1, r.out);
  });
  test("CARGO_HOME is refused, with the other separator and another case", () => {
    const cargo = resolve(tmp, "Cargo-Home");
    const flipped = cargo.includes("\\") ? cargo.replace(/\\/g, "/") : cargo.replace(/\//g, "\\");
    const r = run(["--local-paths", bin("cargo.bin", `${flipped.toUpperCase()}\\registry`)], { CARGO_HOME: cargo });
    assert.equal(r.status, 1, r.out);
  });
  test("the checkout is refused", () => {
    const r = run(["--local-paths", bin("checkout.bin", join(checkout, "src-tauri", "target", "x"))]);
    assert.equal(r.status, 1, r.out);
  });
  test("a test build is refused here too", () => {
    const r = run(["--local-paths", bin("marker.bin", PROFILE_MARKER)]);
    assert.equal(r.status, 1, r.out);
    assert.match(r.out, /test-profile build/);
  });
  test("without a binary it is a usage error", () => {
    assert.equal(run(["--local-paths"]).status, 2);
  });
});
