// Self-test of scripts/release-assemble.sh, the part of release.yml's
// publish and rehearse jobs that signs and checks a release (ADR-039).
//
// Runs the script as the workflow does: bash, the real Tauri CLI from
// node_modules (`tauri signer generate` and `sign`), throwaway keys made
// for each test in a temporary folder, and small stand-in packages laid
// out like the build jobs' artifacts. No network, no real key.
//
// Run: npm run test:release-assemble (node --test). Needs bash, sha256sum
// and `npm ci`; skipped on Windows, where release.yml never runs it.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, describe, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { readPublicKey, verifyManifest } from "./update-manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "release-assemble.sh");
const skip = process.platform === "win32" ? "release.yml runs the script on Ubuntu only" : false;
const work = mkdtempSync(join(tmpdir(), "sotyware-release-assemble-"));
after(() => rmSync(work, { recursive: true, force: true }));

const VERSION = "1.4.0";
const BASE = `https://github.com/casualsoty/sotyware/releases/download/v${VERSION}/`;
const npx = process.platform === "win32" ? "npx.cmd" : "npx";

/** A throwaway Tauri signing key: its private key's text, password and public key file. */
function throwawayKey(name) {
  const dir = mkdtempSync(join(work, `key-${name}-`));
  const file = join(dir, "update.key");
  const password = `pw-${name}-${process.pid}`;
  const r = spawnSync(npx, ["--no-install", "tauri", "signer", "generate", "--ci", "-p", password, "-w", file], { cwd: join(here, ".."), encoding: "utf8" });
  assert.equal(r.status, 0, `tauri signer generate failed: ${r.stderr}`);
  return { privateKey: readFileSync(file, "utf8").trim(), password, pub: `${file}.pub` };
}

/** Build outputs as actions/download-artifact writes them. */
function built({ mac = { aarch64: "unsigned", x86_64: "unsigned" }, leaveOut = null } = {}) {
  const dir = mkdtempSync(join(work, "built-"));
  const files = {
    "windows-installers": [`sotyware_${VERSION}_x64-setup.exe`, `sotyware_${VERSION}_x64_en-US.msi`],
    "linux-packages": [`sotyware_${VERSION}_amd64.AppImage`, `sotyware_${VERSION}_amd64.deb`, `sotyware-${VERSION}-1.x86_64.rpm`],
    "macos-aarch64": [`sotyware_${VERSION}_aarch64.app.tar.gz`, `sotyware_${VERSION}_aarch64.dmg`],
    "macos-x86_64": [`sotyware_${VERSION}_x64.app.tar.gz`, `sotyware_${VERSION}_x64.dmg`],
  };
  for (const [folder, names] of Object.entries(files)) {
    mkdirSync(join(dir, folder));
    for (const n of names) {
      if (n !== leaveOut) {
        writeFileSync(join(dir, folder, n), `stand-in for ${n}\n`);
      }
    }
  }
  const status = { unsigned: "signed=false\nnotarized=false\n", signed: "signed=true\nnotarized=false\n", notarized: "signed=true\nnotarized=true\n", "notarized-only": "signed=false\nnotarized=true\n" };
  for (const arch of ["aarch64", "x86_64"]) {
    writeFileSync(join(dir, `macos-${arch}`, "status.txt"), `arch=${arch}\n${status[mac[arch]]}`);
  }
  return dir;
}

function run(builtDir, key, { pubkey = key.pub, out = mkdtempSync(join(work, "out-")), extraEnv = {} } = {}) {
  const notes = join(work, "notes.md");
  writeFileSync(notes, "Release notes of the test.\n");
  const githubOutput = join(mkdtempSync(join(work, "gh-")), "output");
  writeFileSync(githubOutput, "");
  const env = {
    ...process.env,
    VERSION,
    BASE,
    PUBKEY: pubkey,
    NOTES: notes,
    TAURI_SIGNING_PRIVATE_KEY: key.privateKey,
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: key.password,
    GITHUB_OUTPUT: githubOutput,
    ...extraEnv,
  };
  const r = spawnSync("bash", [script, builtDir, out], { cwd: join(here, ".."), env, encoding: "utf8" });
  return { ...r, out, output: readFileSync(githubOutput, "utf8") };
}

describe("release-assemble.sh", { skip }, () => {
  const key = throwawayKey("main");

  test("Windows and Linux only while the Mac builds are not signed and notarized", () => {
    const r = run(built(), key);
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.output, /^kinds=nsis,msi,appimage,deb,rpm$/m);
    assert.match(r.output, /^macos=not included/m);
    const names = readdirSync(r.out).sort();
    assert.ok(!names.some((n) => n.includes("aarch64") || n.endsWith(".dmg") || n.includes("_x64.app")), names.join(" "));
    const m = verifyManifest({
      bytes: readFileSync(join(r.out, "latest.json")),
      signature: readFileSync(join(r.out, "latest.json.sig"), "utf8"),
      publicKey: readPublicKey(key.pub),
      artifactsDir: r.out,
      baseUrl: BASE,
      require: ["nsis", "msi", "appimage", "deb", "rpm"],
    });
    assert.equal(m.version, VERSION);
    assert.equal(m.notes, "Release notes of the test.");
    // SHA256SUMS lists every other file, signatures included.
    const sums = readFileSync(join(r.out, "SHA256SUMS"), "utf8").trim().split("\n").map((l) => l.split(/\s+\*?/)[1]).sort();
    assert.deepEqual(sums, names.filter((n) => n !== "SHA256SUMS"));
    // The key never reaches the output.
    for (const text of [r.stdout, r.stderr, r.output]) {
      assert.ok(!text.includes(key.privateKey) && !text.includes(key.password));
    }
  });

  test("both Macs signed and notarized: the .app.tar.gz files are updates, the .dmg files downloads", () => {
    const r = run(built({ mac: { aarch64: "notarized", x86_64: "notarized" } }), key);
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.output, /^kinds=nsis,msi,appimage,deb,rpm,app-aarch64,app-x64$/m);
    const m = JSON.parse(readFileSync(join(r.out, "latest.json"), "utf8"));
    assert.deepEqual(Object.keys(m.platforms).sort(), ["darwin-aarch64-app", "darwin-x86_64-app", "linux-x86_64-appimage", "linux-x86_64-deb", "linux-x86_64-rpm", "windows-x86_64-msi", "windows-x86_64-nsis"]);
    assert.ok(existsSync(join(r.out, `sotyware_${VERSION}_aarch64.dmg`)) && existsSync(join(r.out, `sotyware_${VERSION}_x64.dmg`)));
    assert.ok(!existsSync(join(r.out, `sotyware_${VERSION}_aarch64.dmg.sig`)), "a .dmg is not an updater package");
  });

  test("macOS is left out unless both architectures are signed and notarized", () => {
    for (const mac of [
      { aarch64: "notarized", x86_64: "signed" },
      { aarch64: "unsigned", x86_64: "notarized" },
      { aarch64: "notarized-only", x86_64: "notarized" },
    ]) {
      const r = run(built({ mac }), key);
      assert.equal(r.status, 0, r.stderr);
      assert.match(r.output, /^kinds=nsis,msi,appimage,deb,rpm$/m, JSON.stringify(mac));
    }
  });

  test("fails without a Windows or Linux package, or with the wrong public key", () => {
    for (const leaveOut of [`sotyware_${VERSION}_x64_en-US.msi`, `sotyware-${VERSION}-1.x86_64.rpm`]) {
      const r = run(built({ leaveOut }), key);
      assert.notEqual(r.status, 0, leaveOut);
    }
    const other = throwawayKey("other");
    const r = run(built(), key, { pubkey: other.pub });
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /signed by another key/);
    assert.ok(!existsSync(join(r.out, "latest.json")), "no manifest is written for packages the key does not verify");
  });

  test("refuses to run without a private key or into a folder that is not empty", () => {
    const missing = run(built(), key, { extraEnv: { TAURI_SIGNING_PRIVATE_KEY: "" } });
    assert.notEqual(missing.status, 0);
    assert.match(missing.stderr, /TAURI_SIGNING_PRIVATE_KEY is not set/);
    const out = mkdtempSync(join(work, "full-"));
    writeFileSync(join(out, "stale.txt"), "left over");
    const full = run(built(), key, { out });
    assert.notEqual(full.status, 0);
    assert.match(full.stderr, /is not empty/);
  });
});
