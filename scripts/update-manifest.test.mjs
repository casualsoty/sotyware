// Self-test of scripts/update-manifest.mjs (ADR-033).
//
// scripts/fixtures/update/ was produced once by the real Tauri CLI with a
// throwaway test key (`tauri signer generate`, then `tauri signer sign
// --app-version 0.2.0` for both installers and the manifest); only its
// public key is kept, the private key was deleted. The Rust side verifies
// the same fixture (src-tauri/src/updates/tests.rs), so both readers agree
// with the official signer. Further cases sign with keys generated here.
//
// Run: npm run test:update-manifest (node --test). No network, no key file.

import { createHash, generateKeyPairSync, randomBytes, sign as edSign } from "node:crypto";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, describe, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createManifest, main, readPublicKey, signedVersion, verifyManifest, verifySignature } from "./update-manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = join(here, "fixtures", "update");
const work = mkdtempSync(join(tmpdir(), "sotyware-update-manifest-"));
after(() => rmSync(work, { recursive: true, force: true }));

/** A minisign key made here, in Tauri's base64 form; signs like `tauri signer sign`. */
function testKey() {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const keyId = randomBytes(8);
  const raw = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
  const text = `untrusted comment: minisign public key\n${Buffer.concat([Buffer.from("Ed"), keyId, raw]).toString("base64")}\n`;
  return {
    pubkey: Buffer.from(text).toString("base64"),
    sign(data, trusted) {
      const sig = edSign(null, createHash("blake2b512").update(data).digest(), privateKey);
      const global = edSign(null, Buffer.concat([sig, Buffer.from(trusted)]), privateKey);
      const body = `untrusted comment: signature from tauri secret key\n${Buffer.concat([Buffer.from("ED"), keyId, sig]).toString("base64")}\ntrusted comment: ${trusted}\n${global.toString("base64")}\n`;
      return Buffer.from(body).toString("base64");
    },
  };
}

function artifactsSignedWith(key, version, dir = mkdtempSync(join(work, "a-"))) {
  const nsis = join(dir, `sotyware_${version}_x64-setup.exe`);
  const msi = join(dir, `sotyware_${version}_x64_en-US.msi`);
  for (const p of [nsis, msi]) {
    writeFileSync(p, `contents of ${p}`);
    writeFileSync(`${p}.sig`, key.sign(readFileSync(p), `timestamp:1\tfile:${p}\tversion:${version}`));
  }
  return { dir, nsis, msi };
}

describe("the Tauri CLI fixture", () => {
  const pub = readPublicKey(join(fixture, "test-key.pub"));
  const bytes = readFileSync(join(fixture, "sotyware-update.json"));
  const sig = readFileSync(join(fixture, "sotyware-update.json.sig"), "utf8");

  test("verifies with its artifacts", () => {
    const m = verifyManifest({ bytes, signature: sig, publicKey: pub, artifactsDir: fixture, current: "0.1.0" });
    assert.equal(m.version, "0.2.0");
    assert.deepEqual(Object.keys(m.platforms).sort(), ["windows-x86_64-msi", "windows-x86_64-nsis"]);
    const trusted = verifySignature(readFileSync(join(fixture, "sotyware_0.2.0_x64-setup.exe")), m.platforms["windows-x86_64-nsis"].signature, pub);
    assert.equal(signedVersion(trusted), "0.2.0");
  });

  test("refuses a changed manifest, a changed artifact and an older target", () => {
    const changed = Buffer.from(bytes.toString("utf8").replace("not a real release", "a real release"));
    assert.throws(() => verifyManifest({ bytes: changed, signature: sig, publicKey: pub }), /manifest: signature does not match/);
    const copy = mkdtempSync(join(work, "f-"));
    cpSync(fixture, copy, { recursive: true });
    writeFileSync(join(copy, "sotyware_0.2.0_x64_en-US.msi"), "MSI something else\n");
    assert.throws(() => verifyManifest({ bytes, signature: sig, publicKey: pub, artifactsDir: copy }), /en-US\.msi: signature does not match/);
    assert.throws(() => verifyManifest({ bytes, signature: sig, publicKey: pub, current: "0.2.0" }), /not newer/);
    assert.throws(() => verifyManifest({ bytes, signature: sig, publicKey: readPublicKey(testKey().pubkey) }), /another key/);
  });

  test("works from the command line", () => {
    const log = console.log;
    const lines = [];
    console.log = (l) => lines.push(l);
    try {
      assert.equal(main(["verify", "--manifest", join(fixture, "sotyware-update.json"), "--pubkey", join(fixture, "test-key.pub"), "--artifacts", fixture]), 0);
    } finally {
      console.log = log;
    }
    assert.match(lines.join("\n"), /manifest, signature and artifacts verified for 0\.2\.0/);
  });
});

describe("create", () => {
  test("builds a manifest that verifies, with HTTPS URLs and hashes", () => {
    const key = testKey();
    const { dir, nsis, msi } = artifactsSignedWith(key, "1.4.0");
    const pub = readPublicKey(key.pubkey);
    const m = createManifest({ version: "1.4.0", baseUrl: "https://updates.example.org/1.4.0/", publicKey: pub, artifacts: { nsis, msi }, notes: "n", pubDate: "2026-11-15T00:00:00Z" });
    assert.equal(m.platforms["windows-x86_64-nsis"].url, "https://updates.example.org/1.4.0/sotyware_1.4.0_x64-setup.exe");
    assert.equal(m.platforms["windows-x86_64-msi"].sha256, createHash("sha256").update(readFileSync(msi)).digest("hex"));
    assert.equal(m.windows, undefined);
    const bytes = Buffer.from(`${JSON.stringify(m, null, 2)}\n`);
    const signature = key.sign(bytes, "timestamp:1\tfile:sotyware-update.json\tversion:1.4.0");
    assert.equal(verifyManifest({ bytes, signature, publicKey: pub, artifactsDir: dir, current: "1.3.9" }).version, "1.4.0");
  });

  test("refuses artifacts signed for another version, without a version or by another key", () => {
    const key = testKey();
    const pub = readPublicKey(key.pubkey);
    const old = artifactsSignedWith(key, "1.3.0");
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: "https://u.example.org/", publicKey: pub, artifacts: { nsis: old.nsis } }), /signed for version 1\.3\.0, not 1\.4\.0/);
    const { nsis } = artifactsSignedWith(key, "1.4.0");
    writeFileSync(`${nsis}.sig`, key.sign(readFileSync(nsis), "timestamp:1\tfile:x"));
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: "https://u.example.org/", publicKey: pub, artifacts: { nsis } }), /names no version/);
    const other = artifactsSignedWith(testKey(), "1.4.0");
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: "https://u.example.org/", publicKey: pub, artifacts: { nsis: other.nsis } }), /another key/);
  });

  test("refuses plain HTTP, a bad version and missing signatures", () => {
    const key = testKey();
    const pub = readPublicKey(key.pubkey);
    const { nsis } = artifactsSignedWith(key, "1.4.0");
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: "http://u.example.org/", publicKey: pub, artifacts: { nsis } }), /https/);
    assert.throws(() => createManifest({ version: "1.4", baseUrl: "https://u.example.org/", publicKey: pub, artifacts: { nsis } }), /not a version/);
    rmSync(`${nsis}.sig`);
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: "https://u.example.org/", publicKey: pub, artifacts: { nsis } }), /no .*\.sig/);
  });
});

describe("verify", () => {
  const key = testKey();
  const pub = readPublicKey(key.pubkey);
  const doc = (over) => ({
    format: "sotyware-update",
    formatVersion: 1,
    channel: "stable",
    version: "1.4.0",
    notes: null,
    pub_date: null,
    platforms: { "windows-x86_64-nsis": { url: "https://u.example.org/s.exe", signature: "x", sha256: "a".repeat(64) } },
    ...over,
  });
  const check = (d) => verifyManifest({ bytes: Buffer.from(JSON.stringify(d)), signature: null, publicKey: pub });

  test("checks content: format, channel, platforms, URLs and hashes", () => {
    assert.equal(check(doc({})).version, "1.4.0");
    assert.throws(() => check(doc({ format: "x" })), /not a sotyware-update/);
    assert.throws(() => check(doc({ channel: "beta" })), /channel beta/);
    assert.throws(() => check(doc({ platforms: {} })), /no platform/);
    assert.throws(() => check(doc({ platforms: { "windows-x86_64": { url: "https://u/s", signature: "x", sha256: "a".repeat(64) } } })), /unexpected platform/);
    assert.throws(() => check(doc({ platforms: { "windows-x86_64-nsis": { url: "http://u/s", signature: "x", sha256: "a".repeat(64) } } })), /https/);
    assert.throws(() => check(doc({ platforms: { "windows-x86_64-nsis": { url: "https://u/s", signature: "x", sha256: "abc" } } })), /SHA-256/);
  });

  test("checks the manifest signature and its key", () => {
    const bytes = Buffer.from(JSON.stringify(doc({})));
    assert.doesNotThrow(() => verifyManifest({ bytes, signature: key.sign(bytes, "t"), publicKey: pub }));
    assert.throws(() => verifyManifest({ bytes, signature: testKey().sign(bytes, "t"), publicKey: pub }), /another key/);
    assert.throws(() => verifyManifest({ bytes, signature: "bm90IGEgc2lnbmF0dXJl", publicKey: pub }), /not a minisign/);
    // A trusted comment changed after signing.
    const text = Buffer.from(key.sign(bytes, "version:1.4.0"), "base64").toString("utf8").replace("version:1.4.0", "version:9.9.9");
    assert.throws(() => verifyManifest({ bytes, signature: text, publicKey: pub }), /trusted comment/);
  });
});
