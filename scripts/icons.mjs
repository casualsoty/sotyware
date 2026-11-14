#!/usr/bin/env node
// The application icons in src-tauri/icons/, generated from three SVG
// sources in src-tauri/icons/source/ (docs/development/icons.md):
//
//   sotyware.svg      master, used for 48 px and up
//   sotyware-32.svg   24 and 32 px (the tray and window icon at 100 %)
//   sotyware-16.svg   16 px
//
//   npm run icons          regenerate every file listed in GENERATED
//   npm run check:icons    check them (in test:headless): every icon in
//                          tauri.conf.json exists, the NSIS installer and
//                          uninstaller use icon.ico, every PNG is RGBA at the
//                          size its name says, icon.ico holds exactly the
//                          ICO_SIZES as PNG entries and its 32 px entry is
//                          32x32.png, icon.icns is an ICNS file, no other
//                          file lies in src-tauri/icons/.
//
// Rendering is done by `tauri icon` from @tauri-apps/cli (resvg); this
// script only picks the sizes and writes icon.ico itself, so that its
// small entries come from the small sources. Only Node built-ins.

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PREFIX = "icons: ";
const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const ICONS = join(repo, "src-tauri", "icons");
const SOURCES = ["sotyware.svg", "sotyware-32.svg", "sotyware-16.svg"];

/** The entries of icon.ico, smallest first, and the source each comes from. */
export const ICO_SIZES = [16, 24, 32, 48, 64, 256];
const icoSource = (size) => (size === 16 ? "sotyware-16.svg" : size <= 32 ? "sotyware-32.svg" : "sotyware.svg");

/** Every file this script writes into src-tauri/icons/, with its pixel size (null: not a PNG). */
export const GENERATED = {
  "32x32.png": 32,
  "128x128.png": 128,
  "128x128@2x.png": 256,
  "icon.png": 512,
  "icon.icns": null,
  "icon.ico": null,
};

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/**
 * Width, height, bit depth and colour type of a PNG.
 * @param {Buffer} buf
 * @returns {{ width: number, height: number, bitDepth: number, colorType: number }}
 */
export function pngInfo(buf) {
  if (buf.length < 29 || !buf.subarray(0, 8).equals(PNG_SIGNATURE) || buf.toString("latin1", 12, 16) !== "IHDR") {
    throw new Error("not a PNG");
  }
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20), bitDepth: buf[24], colorType: buf[25] };
}

/**
 * An ICO file with one PNG-compressed entry per image (Windows Vista and later).
 * @param {Buffer[]} pngs square PNGs of at most 256 px
 * @returns {Buffer}
 */
export function buildIco(pngs) {
  const header = Buffer.alloc(6 + 16 * pngs.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(pngs.length, 4);
  let offset = header.length;
  pngs.forEach((png, i) => {
    const { width, height } = pngInfo(png);
    if (width !== height || width > 256) throw new Error(`ico entry ${i} is ${width}x${height}`);
    const e = 6 + 16 * i;
    header[e] = width % 256;
    header[e + 1] = height % 256;
    header.writeUInt16LE(1, e + 4);
    header.writeUInt16LE(32, e + 6);
    header.writeUInt32LE(png.length, e + 8);
    header.writeUInt32LE(offset, e + 12);
    offset += png.length;
  });
  return Buffer.concat([header, ...pngs]);
}

/**
 * The entries of an ICO file.
 * @param {Buffer} buf
 * @returns {{ size: number, png: Buffer | null }[]} png is null for a BMP entry
 */
export function readIco(buf) {
  if (buf.length < 6 || buf.readUInt16LE(0) !== 0 || buf.readUInt16LE(2) !== 1) throw new Error("not an ICO");
  const count = buf.readUInt16LE(4);
  if (buf.length < 6 + 16 * count) throw new Error("truncated ICO directory");
  return Array.from({ length: count }, (_, i) => {
    const e = 6 + 16 * i;
    const length = buf.readUInt32LE(e + 8);
    const offset = buf.readUInt32LE(e + 12);
    if (offset + length > buf.length) throw new Error(`ICO entry ${i} lies outside the file`);
    const data = buf.subarray(offset, offset + length);
    const png = data.subarray(0, 8).equals(PNG_SIGNATURE) ? data : null;
    const size = buf[e] || 256;
    if (png) {
      const { width, height } = pngInfo(png);
      if (width !== size || height !== size) throw new Error(`ICO entry ${i} says ${size} px, holds ${width}x${height}`);
    }
    return { size, png };
  });
}

/**
 * What is wrong with the icon files in `dir` (empty when nothing is).
 * @param {string} dir src-tauri/icons
 * @param {any} bundle `bundle` from tauri.conf.json; paths are relative to src-tauri
 * @returns {string[]}
 */
export function iconProblems(dir, bundle) {
  const problems = [];
  const configured = bundle?.icon ?? [];
  // Without these, Tauri's NSIS setup and uninstaller show NSIS's stock icon.
  for (const key of ["installerIcon", "uninstallerIcon"]) {
    const value = bundle?.windows?.nsis?.[key];
    if (value !== "icons/icon.ico") problems.push(`bundle.windows.nsis.${key} is ${value ?? "not set"}, not icons/icon.ico`);
  }
  for (const path of configured) {
    if (!existsSync(join(dir, "..", path))) problems.push(`${path} is in tauri.conf.json but does not exist`);
  }
  if (!configured.some((p) => p.endsWith(".ico"))) problems.push("tauri.conf.json lists no .ico");
  const firstPng = configured.find((p) => p.endsWith(".png"));
  if (firstPng !== "icons/32x32.png") {
    problems.push(`the first PNG in tauri.conf.json is ${firstPng}, not icons/32x32.png (the Linux window and tray icon)`);
  }
  const allowed = new Set([...Object.keys(GENERATED), "source"]);
  for (const name of readdirSync(dir)) if (!allowed.has(name)) problems.push(`unexpected file icons/${name}`);
  for (const name of SOURCES) if (!existsSync(join(dir, "source", name))) problems.push(`source/${name} is missing`);
  for (const [name, size] of Object.entries(GENERATED)) {
    const path = join(dir, name);
    if (!existsSync(path)) {
      problems.push(`${name} is missing`);
      continue;
    }
    const buf = readFileSync(path);
    try {
      if (size !== null) {
        const info = pngInfo(buf);
        if (info.width !== size || info.height !== size) problems.push(`${name} is ${info.width}x${info.height}, not ${size}x${size}`);
        if (info.bitDepth !== 8 || info.colorType !== 6) problems.push(`${name} is not 8-bit RGBA`);
      } else if (name.endsWith(".icns")) {
        if (buf.toString("latin1", 0, 4) !== "icns" || buf.readUInt32BE(4) !== buf.length) problems.push(`${name} is not an ICNS file`);
      } else {
        const entries = readIco(buf);
        const sizes = entries.map((e) => e.size);
        if (sizes.join() !== ICO_SIZES.join()) problems.push(`${name} holds ${sizes.join(", ")} px, not ${ICO_SIZES.join(", ")}`);
        if (entries.some((e) => e.png === null)) problems.push(`${name} has a non-PNG entry`);
        for (const e of entries) {
          if (e.png && (pngInfo(e.png).colorType !== 6 || pngInfo(e.png).bitDepth !== 8)) problems.push(`${name} ${e.size} px entry is not 8-bit RGBA`);
        }
        const e32 = entries.find((e) => e.size === 32)?.png;
        const png32 = join(dir, "32x32.png");
        if (e32 && existsSync(png32) && !e32.equals(readFileSync(png32))) problems.push(`${name}'s 32 px entry differs from 32x32.png`);
      }
    } catch (error) {
      problems.push(`${name}: ${error.message}`);
    }
  }
  return problems;
}

function tauriIcon(source, out, sizes) {
  const tauri = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const args = [tauri, "icon", join(ICONS, "source", source), "--output", out];
  if (sizes) args.push("--png", sizes.join(","));
  // `tauri icon` lists every file it writes on stderr: shown only on failure.
  const r = spawnSync(process.execPath, args, { cwd: repo, encoding: "utf8" });
  if (r.status !== 0) {
    process.stderr.write(`${r.stdout ?? ""}${r.stderr ?? ""}`);
    throw new Error(`tauri icon ${source} failed (exit ${r.status})`);
  }
}

function generate() {
  const tmp = mkdtempSync(join(tmpdir(), "sotyware-icons-"));
  try {
    const full = join(tmp, "full");
    tauriIcon("sotyware.svg", full);
    for (const name of ["128x128.png", "128x128@2x.png", "icon.png", "icon.icns"]) {
      copyFileSync(join(full, name), join(ICONS, name));
    }
    const pngs = [];
    for (const source of new Set(ICO_SIZES.map(icoSource))) {
      const sizes = ICO_SIZES.filter((s) => icoSource(s) === source);
      tauriIcon(source, join(tmp, source), sizes);
    }
    for (const size of ICO_SIZES) pngs.push(readFileSync(join(tmp, icoSource(size), `${size}x${size}.png`)));
    copyFileSync(join(tmp, icoSource(32), "32x32.png"), join(ICONS, "32x32.png"));
    writeFileSync(join(ICONS, "icon.ico"), buildIco(pngs));
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
  console.log(`${PREFIX}wrote ${Object.keys(GENERATED).join(", ")}`);
}

function check() {
  const config = JSON.parse(readFileSync(join(repo, "src-tauri", "tauri.conf.json"), "utf8"));
  const problems = iconProblems(ICONS, config.bundle);
  for (const p of problems) console.error(`${PREFIX}${p}`);
  if (problems.length) process.exit(1);
  console.log(`${PREFIX}ok: ${Object.keys(GENERATED).length} icons, icon.ico ${ICO_SIZES.join("/")} px`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode] = process.argv.slice(2);
  if (mode === "--check") check();
  else if (mode === undefined) generate();
  else {
    console.error(`${PREFIX}usage: node scripts/icons.mjs [--check]`);
    process.exit(2);
  }
}
