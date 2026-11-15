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
import { checkReleaseKey, createManifest, main, readPublicKey, releaseRepository, signedVersion, verifyManifest, verifySignature } from "./update-manifest.mjs";

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

describe("every platform from a GitHub release (ADR-036)", () => {
  const BASE = "https://github.com/owner/repo/releases/download/v1.4.0/";
  const NAMES = {
    nsis: "sotyware_1.4.0_x64-setup.exe",
    msi: "sotyware_1.4.0_x64_en-US.msi",
    "app-aarch64": "sotyware_1.4.0_aarch64.app.tar.gz",
    "app-x64": "sotyware_1.4.0_x64.app.tar.gz",
    appimage: "sotyware_1.4.0_amd64.AppImage",
    deb: "sotyware_1.4.0_amd64.deb",
    rpm: "sotyware-1.4.0-1.x86_64.rpm",
  };

  function allSigned(key) {
    const dir = mkdtempSync(join(work, "gh-"));
    const artifacts = {};
    for (const [kind, name] of Object.entries(NAMES)) {
      const p = join(dir, name);
      writeFileSync(p, `contents of ${name}`);
      writeFileSync(`${p}.sig`, key.sign(readFileSync(p), `timestamp:1\tfile:${name}\tversion:1.4.0`));
      artifacts[kind] = p;
    }
    return { dir, artifacts };
  }

  test("names each platform as the updater plugin does, under the tag's download folder", () => {
    const key = testKey();
    const pub = readPublicKey(key.pubkey);
    const { dir, artifacts } = allSigned(key);
    const m = createManifest({ version: "1.4.0", baseUrl: BASE, publicKey: pub, artifacts });
    assert.deepEqual(Object.keys(m.platforms).sort(), [
      "darwin-aarch64-app",
      "darwin-x86_64-app",
      "linux-x86_64-appimage",
      "linux-x86_64-deb",
      "linux-x86_64-rpm",
      "windows-x86_64-msi",
      "windows-x86_64-nsis",
    ]);
    assert.equal(m.platforms["darwin-aarch64-app"].url, `${BASE}sotyware_1.4.0_aarch64.app.tar.gz`);
    const bytes = Buffer.from(JSON.stringify(m));
    const signature = key.sign(bytes, "timestamp:1\tfile:latest.json\tversion:1.4.0");
    const all = Object.keys(NAMES);
    assert.equal(verifyManifest({ bytes, signature, publicKey: pub, artifactsDir: dir, baseUrl: BASE, require: all }).version, "1.4.0");
    // Exactly the required platforms: one missing or one extra fails.
    assert.throws(() => verifyManifest({ bytes, signature, publicKey: pub, require: all.filter((k) => k !== "rpm") }), /expected exactly/);
    const fewer = createManifest({ version: "1.4.0", baseUrl: BASE, publicKey: pub, artifacts: { nsis: artifacts.nsis } });
    assert.throws(() => verifyManifest({ bytes: Buffer.from(JSON.stringify(fewer)), signature: null, publicKey: pub, require: ["nsis", "msi"] }), /expected exactly/);
    // A URL outside the release's own folder (another tag, another host).
    assert.throws(() => verifyManifest({ bytes, signature, publicKey: pub, baseUrl: "https://github.com/owner/repo/releases/download/v1.3.0/" }), /is not under/);
  });

  test("refuses a file of another kind and a name GitHub would change", () => {
    const key = testKey();
    const pub = readPublicKey(key.pubkey);
    const { artifacts } = allSigned(key);
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: BASE, publicKey: pub, artifacts: { "app-x64": artifacts["app-aarch64"] } }), /is not a app-x64 artifact/);
    const dir = mkdtempSync(join(work, "sp-"));
    const spaced = join(dir, "sotyware 1.4.0_x64-setup.exe");
    writeFileSync(spaced, "x");
    writeFileSync(`${spaced}.sig`, key.sign(readFileSync(spaced), "version:1.4.0"));
    assert.throws(() => createManifest({ version: "1.4.0", baseUrl: BASE, publicKey: pub, artifacts: { nsis: spaced } }), /GitHub would rename/);
    assert.throws(() => verifyManifest({ bytes: Buffer.from("{}"), signature: null, publicKey: pub, require: ["zip"] }), /not a sotyware-update|unknown artifact kind/);
  });

  test("takes every kind from the command line", () => {
    const key = testKey();
    const { dir, artifacts } = allSigned(key);
    const pubFile = join(dir, "key.pub");
    writeFileSync(pubFile, key.pubkey);
    const out = join(dir, "latest.json");
    const log = console.log;
    console.log = () => {};
    try {
      const args = ["create", "--version", "1.4.0", "--base-url", BASE, "--pubkey", pubFile, "--out", out, "--pub-date", "2026-12-01T00:00:00Z"];
      for (const [kind, p] of Object.entries(artifacts)) args.push(`--${kind}`, p);
      assert.equal(main(args), 0);
      writeFileSync(`${out}.sig`, key.sign(readFileSync(out), "timestamp:1\tfile:latest.json\tversion:1.4.0"));
      assert.equal(main(["verify", "--manifest", out, "--pubkey", pubFile, "--artifacts", dir, "--base-url", BASE, "--require", Object.keys(NAMES).join(",")]), 0);
    } finally {
      console.log = log;
    }
    assert.equal(Object.keys(JSON.parse(readFileSync(out, "utf8")).platforms).length, 7);
  });
});

describe("check-signature", () => {
  test("accepts a file signed for the version by this key, and nothing else", () => {
    const key = testKey();
    const dir = mkdtempSync(join(work, "cs-"));
    const pubFile = join(dir, "key.pub");
    writeFileSync(pubFile, key.pubkey);
    const probe = join(dir, "probe.txt");
    writeFileSync(probe, "probe");
    writeFileSync(`${probe}.sig`, key.sign(readFileSync(probe), "timestamp:1\tfile:probe.txt\tversion:1.4.0"));
    const log = console.log;
    console.log = () => {};
    try {
      assert.equal(main(["check-signature", "--pubkey", pubFile, "--file", probe, "--version", "1.4.0"]), 0);
      assert.throws(() => main(["check-signature", "--pubkey", pubFile, "--file", probe, "--version", "1.5.0"]), /signed for version 1\.4\.0/);
      writeFileSync(pubFile, testKey().pubkey);
      assert.throws(() => main(["check-signature", "--pubkey", pubFile, "--file", probe, "--version", "1.4.0"]), /another key/);
    } finally {
      console.log = log;
    }
  });
});

describe("the GitHub release fixture (every platform, real Tauri CLI)", () => {
  // scripts/fixtures/update-github/ was produced once by the release
  // workflow's own commands (tauri signer sign --app-version, then
  // update-manifest create and sign) with a throwaway key whose private half
  // was deleted. src-tauri/src/updates/tests.rs reads the same files.
  const dir = join(here, "fixtures", "update-github");
  const pub = readPublicKey(join(dir, "test-key.pub"));
  const BASE = "https://github.com/owner/repo/releases/download/v0.2.0/";
  const ALL = ["nsis", "msi", "app-aarch64", "app-x64", "appimage", "deb", "rpm"];

  test("verifies with every artifact, under the tag's folder, exactly the seven platforms", () => {
    const m = verifyManifest({
      bytes: readFileSync(join(dir, "latest.json")),
      signature: readFileSync(join(dir, "latest.json.sig"), "utf8"),
      publicKey: pub,
      artifactsDir: dir,
      current: "0.1.0",
      baseUrl: BASE,
      require: ALL,
    });
    assert.equal(Object.keys(m.platforms).length, 7);
    assert.equal(m.pub_date, "2026-11-15T12:00:00Z");
    for (const e of Object.values(m.platforms)) {
      assert.equal(signedVersion(verifySignature(readFileSync(join(dir, e.url.slice(BASE.length))), e.signature, pub)), "0.2.0");
    }
  });
});

describe("the release repository: the source repository itself (ADR-039)", () => {
  const RELEASES = "https://github.com/casualsoty/sotyware/releases/download/v1.4.0/";

  test("is read from src-tauri/src/updates.rs, the one place it is written", () => {
    const rust = readFileSync(join(here, "..", "src-tauri", "src", "updates.rs"), "utf8");
    assert.equal(releaseRepository(rust), "casualsoty/sotyware");
    // MANIFEST_URL is built from the same literal, not written twice.
    assert.doesNotMatch(rust, /"https:\/\/github\.com\/casualsoty\//);
    assert.doesNotMatch(rust, /sotyware-releases/);
  });

  test("refuses a missing or malformed name, and any repository but the expected one", () => {
    assert.throws(() => releaseRepository("pub const X: &str = \"a/b\";"), /no release_repository! macro/);
    for (const bad of ["", "casualsoty", "a/b/c", "-x/y", "x/..", "x/y z"]) {
      assert.throws(() => releaseRepository(`macro_rules! release_repository { () => { "${bad}" }; }`), /not a GitHub repository/, bad);
    }
    const out = [];
    const log = console.log;
    console.log = (l) => out.push(l);
    try {
      assert.equal(main(["release-repository"]), 0);
      // GitHub names are case-insensitive: GITHUB_REPOSITORY may differ in case.
      assert.equal(main(["release-repository", "--expect", "CasualSoty/Sotyware"]), 0);
      assert.deepEqual(out, ["casualsoty/sotyware", "casualsoty/sotyware"]);
      // A fork, or the old separate repository of ADR-038: installations
      // would never see what it publishes.
      for (const other of ["someone/sotyware", "casualsoty/sotyware-releases"]) {
        assert.throws(() => main(["release-repository", "--expect", other]), /would never reach them/, other);
      }
    } finally {
      console.log = log;
    }
  });

  test("manifests point into the repository's own tag folder", () => {
    const key = testKey();
    const pub = readPublicKey(key.pubkey);
    const { nsis, msi } = artifactsSignedWith(key, "1.4.0");
    const m = createManifest({ version: "1.4.0", baseUrl: RELEASES, publicKey: pub, artifacts: { nsis, msi } });
    assert.equal(m.platforms["windows-x86_64-nsis"].url, `${RELEASES}sotyware_1.4.0_x64-setup.exe`);
    const bytes = Buffer.from(JSON.stringify(m));
    assert.equal(verifyManifest({ bytes, signature: null, publicKey: pub, baseUrl: RELEASES, require: ["nsis", "msi"] }).version, "1.4.0");
    // A manifest naming another repository's (or another tag's) files is refused.
    for (const base of ["https://github.com/casualsoty/sotyware-releases/releases/download/v1.4.0/", "https://github.com/casualsoty/sotyware/releases/download/v1.3.0/"]) {
      const other = createManifest({ version: "1.4.0", baseUrl: base, publicKey: pub, artifacts: { nsis } });
      assert.throws(() => verifyManifest({ bytes: Buffer.from(JSON.stringify(other)), signature: null, publicKey: pub, baseUrl: RELEASES }), /is not under/, base);
    }
  });
});

describe("check-key: the release key is never a test fixture", () => {
  const fixtureKeys = [join(fixture, "test-key.pub"), join(here, "fixtures", "update-github", "test-key.pub")];
  const comments = "# sotyware's update signing key: the PUBLIC half only\n#\n";

  test("the committed update-key.pub passes (no key yet, or a real one)", () => {
    const text = readFileSync(join(here, "..", "src-tauri", "update-key.pub"), "utf8");
    const key = checkReleaseKey(text);
    assert.ok(key === null || typeof key === "string");
  });

  test("accepts one real public key and returns its line", () => {
    const key = testKey();
    assert.equal(checkReleaseKey(`${comments}${key.pubkey}\n`), key.pubkey);
    assert.equal(checkReleaseKey(comments), null);
  });

  test("refuses every fixture key, a secret key, two keys and garbage", () => {
    for (const f of fixtureKeys) {
      const line = readFileSync(f, "utf8").trim();
      assert.throws(() => checkReleaseKey(`${comments}${line}\n`), /test fixture key/, f);
    }
    const secret = Buffer.from("untrusted comment: rsign encrypted secret key\nRWRTY0IyAAAA\n").toString("base64");
    assert.throws(() => checkReleaseKey(`${comments}${secret}\n`), /SECRET key/);
    assert.throws(() => checkReleaseKey(`${testKey().pubkey}\n${testKey().pubkey}\n`), /one key line, not 2/);
    assert.throws(() => checkReleaseKey("bm90IGEga2V5\n"), /update-key\.pub: not a minisign/);
    assert.throws(() => checkReleaseKey(`${testKey().pubkey}\n`, mkdtempSync(join(work, "nofix-"))), /no test-key\.pub/);
  });

  test("from the command line: a missing key fails unless allowed; the key is printed", () => {
    const dir = mkdtempSync(join(work, "ck-"));
    const file = join(dir, "update-key.pub");
    writeFileSync(file, comments);
    const out = [];
    const log = console.log;
    const err = console.error;
    console.log = (l) => out.push(l);
    console.error = () => {};
    try {
      assert.throws(() => main(["check-key", "--key-file", file]), /holds no public key/);
      assert.equal(main(["check-key", "--key-file", file, "--allow-missing"]), 0);
      assert.deepEqual(out, []);
      const key = testKey();
      writeFileSync(file, `${comments}${key.pubkey}\n`);
      assert.equal(main(["check-key", "--key-file", file]), 0);
      assert.deepEqual(out, [key.pubkey]);
      // --pubkey takes the same commented file.
      assert.deepEqual(readPublicKey(file).keyId, readPublicKey(key.pubkey).keyId);
    } finally {
      console.log = log;
      console.error = err;
    }
  });
});
