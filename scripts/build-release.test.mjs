// Self-test of scripts/build-release.mjs (ADR-028): the remap flags, how they
// join flags the caller set, and the refused arguments. Builds nothing.
//
// Run: npm run test:release-binary-guard (node --test).

import { describe, test } from "node:test";
import assert from "node:assert/strict";
import { join, resolve } from "node:path";
import { encodedRustflags, refusedArgument, remapFlags } from "./build-release.mjs";

describe("remapFlags", () => {
  const home = resolve("/home/builder");
  const checkout = resolve("/home/builder/src/sotyware");

  test("defaults: home, rustup, cargo, checkout; most specific last", () => {
    const flags = remapFlags({}, home, checkout);
    assert.deepEqual(flags, [
      `--remap-path-prefix=${home}=~`,
      `--remap-path-prefix=${join(home, ".cargo")}=cargo`,
      `--remap-path-prefix=${join(home, ".rustup")}=rustup`,
      `--remap-path-prefix=${checkout}=sotyware`,
    ]);
  });

  test("CARGO_HOME and RUSTUP_HOME from the environment", () => {
    const cargo = resolve("/opt/cargo-home-for-ci");
    const rustup = resolve("/opt/rustup");
    const flags = remapFlags({ CARGO_HOME: cargo, RUSTUP_HOME: rustup }, home, checkout);
    assert.ok(flags.includes(`--remap-path-prefix=${cargo}=cargo`));
    assert.ok(flags.includes(`--remap-path-prefix=${rustup}=rustup`));
    // rustc applies the last match: each prefix comes after every shorter one.
    const lengths = flags.map((f) => f.split("=")[1].length);
    assert.deepEqual(lengths, [...lengths].sort((a, b) => a - b));
  });
});

describe("encodedRustflags", () => {
  const ours = ["--remap-path-prefix=/a=b"];
  test("alone", () => {
    assert.equal(encodedRustflags({}, ours), "--remap-path-prefix=/a=b");
  });
  test("keeps RUSTFLAGS", () => {
    assert.equal(
      encodedRustflags({ RUSTFLAGS: " -C  target-cpu=native " }, ours),
      ["-C", "target-cpu=native", ...ours].join("\x1f"),
    );
  });
  test("CARGO_ENCODED_RUSTFLAGS wins over RUSTFLAGS, as in Cargo", () => {
    assert.equal(
      encodedRustflags({ RUSTFLAGS: "-Cx", CARGO_ENCODED_RUSTFLAGS: "-C\x1fa b" }, ours),
      ["-C", "a b", ...ours].join("\x1f"),
    );
  });
});

describe("refusedArgument", () => {
  test("bundle choices pass", () => {
    assert.equal(refusedArgument(["--bundles", "msi", "--no-bundle", "-v", "--ci"]), null);
  });
  for (const args of [
    ["--features", "test-profile"],
    ["--features=request-recorder"],
    ["-f", "x"],
    ["--debug"],
    ["--", "--target-dir", "target/x"],
    ["--target", "x86_64-pc-windows-msvc"],
    ["--config", "{}"],
  ]) {
    test(`refuses ${args.join(" ")}`, () => {
      assert.notEqual(refusedArgument(args), null);
    });
  }
});
