#!/usr/bin/env node
// Creates and verifies sotyware's signed update manifest `latest.json`
// (ADR-033, ADR-036, docs/architecture/updates.md). Signing is not done
// here: artifacts and the manifest are signed with the official `tauri
// signer sign --app-version` (minisign, the key stays in the release job).
// This script
//
//   create  writes the manifest from signed updater artifacts (Windows
//           installers, macOS .app.tar.gz, Linux AppImage/deb/rpm), after
//           checking every artifact signature and its signed version, with
//           URLs under the GitHub release's download folder;
//   verify  checks a manifest, its signature and (optionally) the local
//           artifacts it names: format, channel, HTTPS URLs under the
//           expected folder, exactly the required platforms, artifact
//           signatures with the right version, SHA-256;
//   check-signature  one file signed for a version by the key (the release
//           job's check that the signing secret matches the committed key);
//   check-key  the committed public key (src-tauri/update-key.pub) is one
//           minisign Ed25519 public key, not a private key pasted by
//           mistake and not one of the test fixtures' throwaway keys
//           (scripts/fixtures/**/test-key.pub); prints it;
//   release-repository  prints the repository whose releases installations
//           read, as written once in src-tauri/src/updates.rs (ADR-039);
//           with --expect, fails unless it is that repository (release.yml
//           passes the repository it publishes to).
//
// It never reads or prints a private key. Only Node's standard library.
//
//   node scripts/update-manifest.mjs create --version 0.2.0 \
//     --base-url https://github.com/OWNER/REPO/releases/download/v0.2.0/ \
//     --pubkey <file|base64> --nsis <setup.exe> [--msi <x.msi>] \
//     [--app-aarch64 <x.app.tar.gz>] [--app-x64 <x.app.tar.gz>] \
//     [--appimage <x.AppImage>] [--deb <x.deb>] [--rpm <x.rpm>] \
//     [--notes-file notes.md] [--pub-date 2026-11-15T12:00:00Z] --out latest.json
//   (each artifact's signature is read from <artifact>.sig)
//
//   node scripts/update-manifest.mjs verify --manifest latest.json \
//     --pubkey <file|base64> [--sig latest.json.sig] [--artifacts <dir>]
//     [--current 0.1.0] [--base-url <folder>/] [--require nsis,msi,…]
//
//   node scripts/update-manifest.mjs check-signature --pubkey <file|base64> \
//     --file <f> --version 0.2.0 [--sig <f>.sig]
//
//   node scripts/update-manifest.mjs check-key [--key-file src-tauri/update-key.pub] \
//     [--fixtures scripts/fixtures] [--allow-missing]
//
//   node scripts/update-manifest.mjs release-repository \
//     [--source src-tauri/src/updates.rs] [--expect <owner/name>]
//
// --pubkey takes a key file (comment lines starting with # are skipped, as
// in src-tauri/update-key.pub) or the base64 line itself.

import { createHash, createPublicKey, verify as edVerify } from "node:crypto";
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const FORMAT = "sotyware-update";
export const FORMAT_VERSION = 1;
export const CHANNEL = "stable";
/** Platform keys, as tauri-plugin-updater names them (`{os}-{arch}-{installer}`),
 * by artifact kind (the command-line option). No generic `{os}-{arch}`
 * entry: an installation whose installer type is unknown (a development
 * build) gets no package. */
export const TARGETS = {
  nsis: "windows-x86_64-nsis",
  msi: "windows-x86_64-msi",
  "app-aarch64": "darwin-aarch64-app",
  "app-x64": "darwin-x86_64-app",
  appimage: "linux-x86_64-appimage",
  deb: "linux-x86_64-deb",
  rpm: "linux-x86_64-rpm",
};

/** What each kind's file is called, so a wrong file is caught before it is
 * signed into a manifest. */
const ARTIFACT_NAMES = {
  nsis: /-setup\.exe$/,
  msi: /\.msi$/,
  "app-aarch64": /_aarch64\.app\.tar\.gz$/,
  "app-x64": /_x64\.app\.tar\.gz$/,
  appimage: /\.AppImage$/,
  deb: /\.deb$/,
  rpm: /\.rpm$/,
};

/** Names GitHub keeps unchanged as release asset names (it rewrites spaces
 * and other characters), so a URL built from the file name finds it. */
export const ASSET_NAME = /^[A-Za-z0-9._-]+$/;

const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

export class ManifestError extends Error {}

function fail(message) {
  throw new ManifestError(message);
}

/** Minisign text from Tauri's form (base64 of the text) or the text itself. */
function minisignText(value) {
  const t = value.trim();
  if (t.startsWith("untrusted comment:")) {
    return t;
  }
  const decoded = Buffer.from(t, "base64").toString("utf8");
  if (!decoded.startsWith("untrusted comment:")) {
    fail("not a minisign key or signature");
  }
  return decoded.trim();
}

/** The key lines of a key file: neither empty nor a `#` comment (the rule
 * `pubkey_in` in src-tauri/src/updates.rs applies to update-key.pub). */
export function keyLines(text) {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#"));
}

/** A public key from a file or a base64 string (as `tauri signer generate` writes it). */
export function readPublicKey(value) {
  let raw = value;
  if (existsSync(value)) {
    const text = readFileSync(value, "utf8");
    // A key file such as update-key.pub: comments, then the base64 line.
    raw = text.trimStart().startsWith("untrusted comment:") ? text : (keyLines(text)[0] ?? "");
  }
  const text = minisignText(raw);
  if (/secret key/i.test(text.split(/\r?\n/)[0])) {
    fail("this is a minisign SECRET key, not a public key");
  }
  const lines = text.split(/\r?\n/);
  const bytes = Buffer.from(lines[1] ?? "", "base64");
  if (bytes.length !== 42 || bytes.toString("latin1", 0, 2) !== "Ed") {
    fail("not a minisign Ed25519 public key");
  }
  return {
    keyId: bytes.subarray(2, 10),
    key: createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: bytes.subarray(10).toString("base64url") }, format: "jwk" }),
  };
}

function parseSignature(value) {
  const lines = minisignText(value).split(/\r?\n/);
  if (lines.length < 4 || !lines[2].startsWith("trusted comment: ")) {
    fail("malformed signature");
  }
  const sig = Buffer.from(lines[1], "base64");
  const global = Buffer.from(lines[3], "base64");
  const alg = sig.toString("latin1", 0, 2);
  if (sig.length !== 74 || global.length !== 64 || (alg !== "ED" && alg !== "Ed")) {
    fail("malformed signature");
  }
  return { alg, keyId: sig.subarray(2, 10), signature: sig.subarray(10), trusted: lines[2].slice("trusted comment: ".length), global };
}

/** Verifies a minisign signature (prehashed "ED" or legacy "Ed") and its
 * trusted comment; returns the trusted comment. */
export function verifySignature(data, signature, publicKey) {
  const s = parseSignature(signature);
  if (!s.keyId.equals(publicKey.keyId)) {
    fail("signed by another key");
  }
  const message = s.alg === "ED" ? createHash("blake2b512").update(data).digest() : data;
  if (!edVerify(null, message, publicKey.key, s.signature)) {
    fail("signature does not match");
  }
  if (!edVerify(null, Buffer.concat([s.signature, Buffer.from(s.trusted, "utf8")]), publicKey.key, s.global)) {
    fail("trusted comment does not match its signature");
  }
  return s.trusted;
}

/** The `version:` field of a trusted comment written by `tauri signer sign --app-version`. */
export function signedVersion(trusted) {
  for (const field of trusted.split("\t")) {
    if (field.startsWith("version:")) {
      return field.slice("version:".length);
    }
  }
  return null;
}

function sha256(data) {
  return createHash("sha256").update(data).digest("hex");
}

function checkVersion(version) {
  if (!SEMVER.test(version)) {
    fail(`not a version: ${version}`);
  }
}

function compareVersions(a, b) {
  const pa = a.split(/[-+]/)[0].split(".").map(Number);
  const pb = b.split(/[-+]/)[0].split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) {
      return pa[i] < pb[i] ? -1 : 1;
    }
  }
  return 0;
}

function checkArtifact(path, sigText, publicKey, version) {
  const data = readFileSync(path);
  let trusted;
  try {
    trusted = verifySignature(data, sigText, publicKey);
  } catch (e) {
    fail(`${basename(path)}: ${e.message}`);
  }
  const signed = signedVersion(trusted);
  if (signed === null) {
    fail(`${basename(path)}: the signature names no version (sign with --app-version)`);
  }
  if (signed.replace(/^v/, "") !== version) {
    fail(`${basename(path)}: signed for version ${signed}, not ${version}`);
  }
  return sha256(data);
}

/** Builds the manifest object; checks every artifact first. */
export function createManifest({ version, baseUrl, publicKey, artifacts, notes = null, pubDate = null }) {
  checkVersion(version);
  if (!/^https:\/\/[^/]+\/(.*\/)?$/.test(baseUrl)) {
    fail("the base URL must be an https:// URL ending in /");
  }
  const platforms = {};
  for (const [kind, path] of Object.entries(artifacts)) {
    const target = TARGETS[kind];
    if (!target) {
      fail(`unknown artifact kind ${kind}`);
    }
    const name = basename(path);
    if (!ASSET_NAME.test(name)) {
      fail(`${name}: GitHub would rename this asset; use only letters, digits, dot, dash and underscore`);
    }
    if (!ARTIFACT_NAMES[kind].test(name)) {
      fail(`${name} is not a ${kind} artifact`);
    }
    const sigPath = `${path}.sig`;
    if (!existsSync(sigPath)) {
      fail(`${basename(path)}: no ${basename(sigPath)} (sign it with tauri signer sign --app-version ${version})`);
    }
    const signature = readFileSync(sigPath, "utf8").trim();
    const hash = checkArtifact(path, signature, publicKey, version);
    platforms[target] = { url: baseUrl + encodeURIComponent(basename(path)), signature, sha256: hash };
  }
  if (Object.keys(platforms).length === 0) {
    fail("no artifact given");
  }
  return {
    format: FORMAT,
    formatVersion: FORMAT_VERSION,
    channel: CHANNEL,
    version,
    notes,
    pub_date: pubDate,
    platforms,
  };
}

/** Checks a manifest's bytes (signature first), its content and, when
 * `artifactsDir` is given, the artifacts it names. Returns the manifest. */
export function verifyManifest({ bytes, signature, publicKey, artifactsDir = null, current = null, baseUrl = null, require = null }) {
  if (signature !== null) {
    try {
      verifySignature(bytes, signature, publicKey);
    } catch (e) {
      fail(`manifest: ${e.message}`);
    }
  }
  const m = JSON.parse(bytes.toString("utf8"));
  if (m.format !== FORMAT || m.formatVersion !== FORMAT_VERSION) {
    fail(`manifest: not a ${FORMAT} version ${FORMAT_VERSION} document`);
  }
  if (m.channel !== CHANNEL) {
    fail(`manifest: channel ${m.channel}`);
  }
  checkVersion(m.version);
  if (current !== null && compareVersions(m.version, current) <= 0) {
    fail(`manifest: version ${m.version} is not newer than ${current}`);
  }
  const entries = Object.entries(m.platforms ?? {});
  if (entries.length === 0) {
    fail("manifest: no platform");
  }
  if (require !== null) {
    const want = require.map((kind) => TARGETS[kind] ?? fail(`unknown artifact kind ${kind}`)).sort();
    const got = entries.map(([t]) => t).sort();
    if (want.join() !== got.join()) {
      fail(`manifest: platforms ${got.join(", ")}; expected exactly ${want.join(", ")}`);
    }
  }
  for (const [target, e] of entries) {
    if (!Object.values(TARGETS).includes(target)) {
      fail(`manifest: unexpected platform ${target}`);
    }
    if (typeof e.url !== "string" || !e.url.startsWith("https://")) {
      fail(`${target}: the URL must use https://`);
    }
    if (baseUrl !== null && !e.url.startsWith(baseUrl)) {
      fail(`${target}: ${e.url} is not under ${baseUrl}`);
    }
    if (typeof e.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(e.sha256) || typeof e.signature !== "string" || e.signature === "") {
      fail(`${target}: needs a signature and a SHA-256`);
    }
    if (artifactsDir !== null) {
      const name = decodeURIComponent(new URL(e.url).pathname.split("/").pop() ?? "");
      const path = join(artifactsDir, name);
      if (!existsSync(path)) {
        fail(`${target}: ${name} is not in ${artifactsDir}`);
      }
      const hash = checkArtifact(path, e.signature, publicKey, m.version);
      if (hash !== e.sha256) {
        fail(`${target}: ${name} has SHA-256 ${hash}, the manifest says ${e.sha256}`);
      }
    }
  }
  return m;
}

const here = dirname(fileURLToPath(import.meta.url));
export const KEY_FILE = join(here, "..", "src-tauri", "update-key.pub");
export const FIXTURES = join(here, "fixtures");
export const UPDATES_RS = join(here, "..", "src-tauri", "src", "updates.rs");

/** Every `test-key.pub` below `dir`: the throwaway keys the signing
 * fixtures were made with. */
function fixtureKeys(dir) {
  const found = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) {
      found.push(...fixtureKeys(p));
    } else if (e.name === "test-key.pub") {
      found.push(p);
    }
  }
  return found;
}

/** Checks the release public key in `keyFileText` (update-key.pub's
 * content); returns its base64 line, or null when it holds none. Refuses
 * a private key, more than one key line, anything that is not a minisign
 * Ed25519 public key, and the key of any signing fixture under
 * `fixturesDir`, which must hold at least one. */
export function checkReleaseKey(keyFileText, fixturesDir = FIXTURES) {
  const lines = keyLines(keyFileText);
  if (lines.length === 0) {
    return null;
  }
  for (const l of lines) {
    let decoded = "";
    try {
      decoded = Buffer.from(l, "base64").toString("utf8");
    } catch {
      // Not base64: reported below.
    }
    if (/secret key/i.test(decoded) || /secret key/i.test(l)) {
      fail("update-key.pub holds a minisign SECRET key: remove it, never commit it, and treat that key as compromised (generate a new pair)");
    }
  }
  if (lines.length > 1) {
    fail(`update-key.pub must hold one key line, not ${lines.length}`);
  }
  let key;
  try {
    key = readPublicKey(lines[0]);
  } catch (e) {
    fail(`update-key.pub: ${e.message}`);
  }
  const fixtures = fixtureKeys(fixturesDir);
  if (fixtures.length === 0) {
    fail(`no test-key.pub under ${fixturesDir}: cannot check the key against the test fixtures`);
  }
  for (const f of fixtures) {
    if (readPublicKey(f).keyId.equals(key.keyId)) {
      fail(`update-key.pub holds the test fixture key ${f}: fixtures are for tests only; generate the release key (docs/development/release.md#update-signing-key)`);
    }
  }
  return lines[0];
}

/** The repository whose releases installations read, written in
 * src-tauri/src/updates.rs (the `release_repository!` macro, ADR-039). */
export function releaseRepository(rustSource) {
  const m = rustSource.match(/macro_rules!\s+release_repository\s*\{\s*\(\)\s*=>\s*\{\s*"([^"]*)"\s*\};?\s*\}/);
  if (!m) {
    fail("src-tauri/src/updates.rs has no release_repository! macro naming the release repository");
  }
  const repo = m[1];
  if (!/^[A-Za-z0-9][A-Za-z0-9-]{0,38}\/[A-Za-z0-9._-]{1,100}$/.test(repo) || repo.split("/")[1].startsWith(".")) {
    fail(`not a GitHub repository: ${repo}`);
  }
  return repo;
}

/** Options that take no value. */
const FLAGS = new Set(["allow-missing"]);

function args(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) {
      fail(`unexpected argument ${a}`);
    }
    if (FLAGS.has(a.slice(2))) {
      out[a.slice(2)] = "true";
      continue;
    }
    const v = argv[i + 1];
    if (v === undefined || v.startsWith("--")) {
      fail(`${a} needs a value`);
    }
    out[a.slice(2)] = v;
    i++;
  }
  return out;
}

function need(o, k) {
  if (o[k] === undefined) {
    fail(`--${k} is required`);
  }
  return o[k];
}

export function main(argv) {
  const [command, ...rest] = argv;
  const o = args(rest);
  if (command === "check-key") {
    const file = o["key-file"] ?? KEY_FILE;
    const key = checkReleaseKey(readFileSync(file, "utf8"), o.fixtures ?? FIXTURES);
    if (key === null) {
      if (o["allow-missing"] !== "true") {
        fail(`${file} holds no public key (docs/development/release.md#update-signing-key)`);
      }
      console.error(`update-manifest: ${basename(file)} holds no key: release builds have updates switched off`);
      return 0;
    }
    console.error(`update-manifest: ${basename(file)} holds one minisign public key, not a test fixture's`);
    console.log(key);
    return 0;
  }
  if (command === "release-repository") {
    const repo = releaseRepository(readFileSync(o.source ?? UPDATES_RS, "utf8"));
    // GitHub compares owner and repository names without regard to case.
    if (o.expect !== undefined && o.expect.toLowerCase() !== repo.toLowerCase()) {
      fail(`installations read the releases of ${repo} (src-tauri/src/updates.rs), not of ${o.expect}: a release published to ${o.expect} would never reach them (ADR-039)`);
    }
    console.log(repo);
    return 0;
  }
  const publicKey = readPublicKey(need(o, "pubkey"));
  if (command === "create") {
    const artifacts = {};
    for (const kind of Object.keys(TARGETS)) {
      if (o[kind]) artifacts[kind] = o[kind];
    }
    const m = createManifest({
      version: need(o, "version"),
      baseUrl: need(o, "base-url"),
      publicKey,
      artifacts,
      notes: o["notes-file"] ? readFileSync(o["notes-file"], "utf8").trim() : null,
      pubDate: o["pub-date"] ?? new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
    });
    writeFileSync(need(o, "out"), `${JSON.stringify(m, null, 2)}\n`);
    console.log(`update-manifest: wrote ${o.out} for ${m.version}: ${Object.keys(m.platforms).join(", ")}`);
    console.log(`update-manifest: now sign it: tauri signer sign --app-version ${m.version} ${o.out}`);
    return 0;
  }
  if (command === "verify") {
    const path = need(o, "manifest");
    const sigPath = o.sig ?? `${path}.sig`;
    const m = verifyManifest({
      bytes: readFileSync(path),
      signature: readFileSync(sigPath, "utf8"),
      publicKey,
      artifactsDir: o.artifacts ?? null,
      current: o.current ?? null,
      baseUrl: o["base-url"] ?? null,
      require: o.require ? o.require.split(",").map((k) => k.trim()).filter(Boolean) : null,
    });
    const what = o.artifacts ? "manifest, signature and artifacts" : "manifest and signature";
    console.log(`update-manifest: ${what} verified for ${m.version}: ${Object.keys(m.platforms).join(", ")}`);
    return 0;
  }
  if (command === "check-signature") {
    // One file and its .sig, signed for --version: the release job's check
    // that the signing secret belongs to the committed public key.
    const file = need(o, "file");
    const version = need(o, "version");
    checkArtifact(file, readFileSync(o.sig ?? `${file}.sig`, "utf8").trim(), publicKey, version);
    console.log(`update-manifest: ${basename(file)} is signed by this key for ${version}`);
    return 0;
  }
  fail("usage: update-manifest.mjs create|verify|check-signature --pubkey <key> … | check-key | release-repository");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (e) {
    console.error(`update-manifest: ${e instanceof ManifestError ? e.message : e}`);
    process.exitCode = 1;
  }
}
