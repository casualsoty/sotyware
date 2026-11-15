#!/usr/bin/env node
// Creates and verifies sotyware's signed update manifest (ADR-033,
// docs/architecture/updates.md). Signing is not done here: artifacts and the
// manifest are signed with the official `tauri signer sign --app-version`
// (minisign, the key stays in the release environment). This script
//
//   create  writes sotyware-update.json from signed Windows installers,
//           after checking every artifact signature and its signed version;
//   verify  checks a manifest, its signature and (optionally) the local
//           artifacts it names: format, channel, HTTPS URLs, artifact
//           signatures with the right version, SHA-256.
//
// It never reads or prints a private key. Only Node's standard library.
//
//   node scripts/update-manifest.mjs create --version 0.2.0 \
//     --base-url https://updates.example.org/0.2.0/ --pubkey <file|base64> \
//     --nsis <setup.exe> [--msi <x.msi>] [--notes-file notes.md] \
//     [--pub-date 2026-11-15T12:00:00Z] --out sotyware-update.json
//   (each artifact's signature is read from <artifact>.sig)
//
//   node scripts/update-manifest.mjs verify --manifest sotyware-update.json \
//     --pubkey <file|base64> [--sig sotyware-update.json.sig] [--artifacts <dir>]
//     [--current 0.1.0]

import { createHash, createPublicKey, verify as edVerify } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

export const FORMAT = "sotyware-update";
export const FORMAT_VERSION = 1;
export const CHANNEL = "stable";
/** Platform keys, as tauri-plugin-updater names them (`{os}-{arch}-{installer}`).
 * No generic `windows-x86_64` entry: an installation whose installer type
 * is unknown gets no package. */
export const TARGETS = { nsis: "windows-x86_64-nsis", msi: "windows-x86_64-msi" };

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

/** A public key from a file or a base64 string (as `tauri signer generate` writes it). */
export function readPublicKey(value) {
  const raw = existsSync(value) ? readFileSync(value, "utf8") : value;
  const lines = minisignText(raw).split(/\r?\n/);
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
export function verifyManifest({ bytes, signature, publicKey, artifactsDir = null, current = null }) {
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
  for (const [target, e] of entries) {
    if (!Object.values(TARGETS).includes(target)) {
      fail(`manifest: unexpected platform ${target}`);
    }
    if (typeof e.url !== "string" || !e.url.startsWith("https://")) {
      fail(`${target}: the URL must use https://`);
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

function args(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) {
      fail(`unexpected argument ${a}`);
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
  const publicKey = readPublicKey(need(o, "pubkey"));
  if (command === "create") {
    const artifacts = {};
    if (o.nsis) artifacts.nsis = o.nsis;
    if (o.msi) artifacts.msi = o.msi;
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
    });
    const what = o.artifacts ? "manifest, signature and artifacts" : "manifest and signature";
    console.log(`update-manifest: ${what} verified for ${m.version}: ${Object.keys(m.platforms).join(", ")}`);
    return 0;
  }
  fail("usage: update-manifest.mjs create|verify --pubkey <key> …");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (e) {
    console.error(`update-manifest: ${e instanceof ManifestError ? e.message : e}`);
    process.exitCode = 1;
  }
}
