#!/usr/bin/env node
// Checks that the Windows installers carry sotyware's icon
// (src-tauri/icons/icon.ico), by reading their bytes only
// (docs/development/icons.md#installer-icons):
//
//   node scripts/check-installer-icons.mjs [--ico <icon.ico>]
//        [--msi <file.msi>]... [--msi-image <dir>]...
//        [--nsis <setup.exe>]... [--exe <file.exe>]...
//
//   --msi        the MSI's Icon table row named by ARPPRODUCTICON (the
//                Add/Remove Programs icon) holds exactly icon.ico; every
//                shortcut names that icon or targets sotyware.exe, at least
//                one names it, and none names another icon.
//   --msi-image  an administrative image of the MSI (msiexec /a): the .msi
//                copy in it passes --msi, and the sotyware.exe in it passes
//                --exe (the icon of shortcuts without their own icon).
//   --nsis       the setup program's icon group, and the uninstaller's: the
//                uninstaller is not stored as a file; NSIS writes it at
//                install time from the setup program's header, patched with
//                the uninstaller icon data kept in the compressed data. This
//                script decompresses that data (solid LZMA, Tauri's default)
//                and applies the patch in memory, as NSIS would.
//   --exe        a PE file's icon group.
//
// "Holds exactly icon.ico" means: one icon group, whose entries are, in
// order, icon.ico's entries (width, height, colours, planes, bit count,
// size), each pointing at an RT_ICON resource byte-equal to that entry's
// image, and no other RT_ICON resource. For the MSI the Icon stream is
// compared with icon.ico byte for byte.
//
// Nothing is executed, installed, extracted to disk or written: files are
// read with readFileSync and parsed in memory. Only Node built-ins.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readIco } from "./icons.mjs";

const PREFIX = "installer-icons: ";
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_ICO = join(repo, "src-tauri", "icons", "icon.ico");
const RT_ICON = 3;
const RT_GROUP_ICON = 14;

/** A problem with a file being checked. */
export class IconCheckError extends Error {}
const fail = (message) => {
  throw new IconCheckError(message);
};

function need(buf, offset, length, what) {
  if (!Number.isInteger(offset) || offset < 0 || offset + length > buf.length) fail(`${what} lies outside the file (truncated or malformed)`);
}

// ---------------------------------------------------------------- ICO

/**
 * The directory entries and images of an ICO file.
 * @param {Buffer} buf
 * @returns {{ width: number, height: number, colors: number, planes: number, bitCount: number, data: Buffer }[]}
 */
export function icoEntries(buf) {
  readIco(buf); // structure and PNG sizes
  const count = buf.readUInt16LE(4);
  if (count === 0) fail("the ICO has no entries");
  return Array.from({ length: count }, (_, i) => {
    const e = 6 + 16 * i;
    const length = buf.readUInt32LE(e + 8);
    const offset = buf.readUInt32LE(e + 12);
    return {
      width: buf[e],
      height: buf[e + 1],
      colors: buf[e + 2],
      planes: buf.readUInt16LE(e + 4),
      bitCount: buf.readUInt16LE(e + 6),
      data: buf.subarray(offset, offset + length),
    };
  });
}

// ---------------------------------------------------------------- PE resources

/**
 * Every resource of a PE file (EXE or DLL).
 * @param {Buffer} buf
 * @returns {{ type: number | string, id: number | string, lang: number | string, data: Buffer }[]}
 */
export function peResources(buf) {
  if (buf.length < 64 || buf.toString("latin1", 0, 2) !== "MZ") fail("not a PE file (no MZ header)");
  const pe = buf.readUInt32LE(0x3c);
  need(buf, pe, 24, "PE header");
  if (buf.toString("latin1", pe, pe + 4) !== "PE\0\0") fail("not a PE file (no PE signature)");
  const sections = buf.readUInt16LE(pe + 6);
  const optSize = buf.readUInt16LE(pe + 20);
  const opt = pe + 24;
  need(buf, opt, optSize, "optional header");
  const magic = buf.readUInt16LE(opt);
  if (magic !== 0x10b && magic !== 0x20b) fail(`unknown optional header magic 0x${magic.toString(16)}`);
  const dirs = opt + (magic === 0x20b ? 112 : 96);
  const dirCount = buf.readUInt32LE(dirs - 4);
  if (dirCount < 3 || dirs + 24 > opt + optSize) return [];
  const rsrcRva = buf.readUInt32LE(dirs + 16);
  if (rsrcRva === 0) return [];
  const table = opt + optSize;
  need(buf, table, 40 * sections, "section table");
  const sects = Array.from({ length: sections }, (_, i) => {
    const s = table + 40 * i;
    return {
      va: buf.readUInt32LE(s + 12),
      vsize: buf.readUInt32LE(s + 8),
      rawSize: buf.readUInt32LE(s + 16),
      raw: buf.readUInt32LE(s + 20),
    };
  });
  const offsetOf = (rva, length, what) => {
    const s = sects.find((x) => rva >= x.va && rva + length <= x.va + Math.max(x.vsize, x.rawSize));
    if (!s || rva + length > s.va + s.rawSize) fail(`${what} (RVA 0x${rva.toString(16)}) is not in the file`);
    const off = s.raw + rva - s.va;
    need(buf, off, length, what);
    return off;
  };
  const root = offsetOf(rsrcRva, 16, "resource directory");
  const out = [];
  const seen = new Set();
  const name = (value) => {
    if (!(value & 0x80000000)) return value;
    const at = root + (value & 0x7fffffff);
    need(buf, at, 2, "resource name");
    const len = buf.readUInt16LE(at);
    need(buf, at + 2, 2 * len, "resource name");
    return buf.toString("utf16le", at + 2, at + 2 + 2 * len);
  };
  const walk = (dirOffset, path) => {
    if (seen.has(dirOffset) || path.length > 2) fail("resource directory is malformed (loop or too deep)");
    seen.add(dirOffset);
    const at = root + dirOffset;
    need(buf, at, 16, "resource directory");
    const count = buf.readUInt16LE(at + 12) + buf.readUInt16LE(at + 14);
    need(buf, at + 16, 8 * count, "resource directory entries");
    for (let i = 0; i < count; i++) {
      const e = at + 16 + 8 * i;
      const key = name(buf.readUInt32LE(e));
      const target = buf.readUInt32LE(e + 4);
      if (target & 0x80000000) {
        walk(target & 0x7fffffff, [...path, key]);
      } else {
        if (path.length !== 2) fail("resource directory is malformed (data entry at the wrong level)");
        const d = root + target;
        need(buf, d, 16, "resource data entry");
        const size = buf.readUInt32LE(d + 4);
        const off = offsetOf(buf.readUInt32LE(d), size, "resource data");
        out.push({ type: path[0], id: path[1], lang: key, data: buf.subarray(off, off + size) });
      }
    }
  };
  walk(0, []);
  return out;
}

/**
 * What is wrong with a PE file's icon compared with an ICO (empty when nothing is).
 * @param {Buffer} pe
 * @param {Buffer} ico
 * @returns {string[]}
 */
export function peIconProblems(pe, ico) {
  const want = icoEntries(ico);
  const resources = peResources(pe);
  const groups = resources.filter((r) => r.type === RT_GROUP_ICON);
  const icons = resources.filter((r) => r.type === RT_ICON);
  if (groups.length === 0) return ["no icon group (RT_GROUP_ICON)"];
  if (groups.length > 1) return [`${groups.length} icon groups, expected one: ${groups.map((g) => `${g.id}/${g.lang}`).join(", ")}`];
  const g = groups[0].data;
  if (g.length < 6 || g.readUInt16LE(0) !== 0 || g.readUInt16LE(2) !== 1) return ["the icon group is malformed"];
  const count = g.readUInt16LE(4);
  if (g.length < 6 + 14 * count) return ["the icon group is truncated"];
  const problems = [];
  if (count !== want.length) problems.push(`the icon group has ${count} entries, icon.ico has ${want.length}`);
  const used = new Set();
  for (let i = 0; i < count; i++) {
    const e = 6 + 14 * i;
    const entry = {
      width: g[e],
      height: g[e + 1],
      colors: g[e + 2],
      planes: g.readUInt16LE(e + 4),
      bitCount: g.readUInt16LE(e + 6),
      bytes: g.readUInt32LE(e + 8),
      id: g.readUInt16LE(e + 12),
    };
    const label = `entry ${i} (${entry.width || 256} px, RT_ICON ${entry.id})`;
    const matches = icons.filter((r) => r.id === entry.id);
    if (matches.length !== 1) {
      problems.push(`${label}: ${matches.length} RT_ICON resources with that id, expected one`);
      continue;
    }
    used.add(entry.id);
    const data = matches[0].data;
    if (data.length !== entry.bytes) problems.push(`${label}: the group says ${entry.bytes} bytes, the resource has ${data.length}`);
    const w = want[i];
    if (!w) continue;
    for (const key of ["width", "height", "colors", "planes", "bitCount"]) {
      if (entry[key] !== w[key]) problems.push(`${label}: ${key} is ${entry[key]}, icon.ico says ${w[key]}`);
    }
    if (!data.equals(w.data)) problems.push(`${label}: image differs from icon.ico entry ${i} (${w.width || 256} px)`);
  }
  const stray = icons.filter((r) => !used.has(r.id));
  if (stray.length) problems.push(`RT_ICON resources outside the icon group: ${stray.map((r) => r.id).join(", ")}`);
  return problems;
}

// ---------------------------------------------------------------- compound file (MSI container)

/**
 * The streams directly under the root storage of a compound file.
 * @param {Buffer} buf
 * @returns {Map<string, Buffer>} keyed by the raw (UTF-16) stream name
 */
export function cfbStreams(buf) {
  if (buf.length < 512 || buf.readUInt32LE(0) !== 0xe011cfd0 || buf.readUInt32LE(4) !== 0xe11ab1a1) {
    fail("not an MSI (no compound file signature)");
  }
  const shift = buf.readUInt16LE(0x1e);
  const miniShift = buf.readUInt16LE(0x20);
  if ((shift !== 9 && shift !== 12) || miniShift !== 6) fail("compound file header is malformed (sector sizes)");
  const size = 1 << shift;
  const perSector = size / 4;
  const cutoff = buf.readUInt32LE(0x38);
  const sectorOffset = (n) => {
    const off = (n + 1) * size;
    need(buf, off, size, `sector ${n}`);
    return off;
  };
  // FAT sector numbers: 109 in the header, the rest in a DIFAT chain.
  const fatSectors = [];
  for (let i = 0; i < 109; i++) fatSectors.push(buf.readUInt32LE(0x4c + 4 * i));
  let difat = buf.readUInt32LE(0x44);
  for (let n = buf.readUInt32LE(0x48); n > 0 && difat < 0xfffffffa; n--) {
    const off = sectorOffset(difat);
    for (let i = 0; i < perSector - 1; i++) fatSectors.push(buf.readUInt32LE(off + 4 * i));
    difat = buf.readUInt32LE(off + size - 4);
  }
  const fatCount = buf.readUInt32LE(0x2c);
  const fat = [];
  for (const s of fatSectors.slice(0, fatCount)) {
    if (s >= 0xfffffffa) fail("compound file FAT is malformed");
    const off = sectorOffset(s);
    for (let i = 0; i < perSector; i++) fat.push(buf.readUInt32LE(off + 4 * i));
  }
  const chain = (start, table, what) => {
    const out = [];
    for (let s = start; s !== 0xfffffffe; s = table[s]) {
      if (s === undefined || s >= table.length || out.length > table.length) fail(`${what}: broken sector chain`);
      out.push(s);
    }
    return out;
  };
  const readChain = (start, what) => Buffer.concat(chain(start, fat, what).map((s) => buf.subarray(sectorOffset(s), sectorOffset(s) + size)));
  const dir = readChain(buf.readUInt32LE(0x30), "directory");
  const entries = [];
  for (let off = 0; off + 128 <= dir.length; off += 128) {
    const nameLen = dir.readUInt16LE(off + 64);
    entries.push({
      name: dir.toString("utf16le", off, off + Math.max(0, Math.min(64, nameLen) - 2)),
      type: dir[off + 66],
      left: dir.readUInt32LE(off + 68),
      right: dir.readUInt32LE(off + 72),
      child: dir.readUInt32LE(off + 76),
      start: dir.readUInt32LE(off + 116),
      size: dir.readUInt32LE(off + 120) + (shift === 12 ? dir.readUInt32LE(off + 124) * 2 ** 32 : 0),
    });
  }
  const root = entries[0];
  if (!root || root.type !== 5) fail("compound file has no root entry");
  const miniStream = root.size > 0 ? readChain(root.start, "mini stream") : Buffer.alloc(0);
  const miniFat = [];
  if (buf.readUInt32LE(0x40) > 0) {
    const raw = readChain(buf.readUInt32LE(0x3c), "mini FAT");
    for (let i = 0; i + 4 <= raw.length; i += 4) miniFat.push(raw.readUInt32LE(i));
  }
  const read = (e) => {
    if (e.size >= cutoff) {
      const data = readChain(e.start, e.name);
      if (data.length < e.size) fail(`stream ${JSON.stringify(e.name)} is truncated`);
      return data.subarray(0, e.size);
    }
    const parts = chain(e.start, miniFat, e.name).map((s) => {
      need(miniStream, s * 64, 64, `mini sector ${s}`);
      return miniStream.subarray(s * 64, s * 64 + 64);
    });
    const data = Buffer.concat(parts);
    if (data.length < e.size) fail(`stream ${JSON.stringify(e.name)} is truncated`);
    return data.subarray(0, e.size);
  };
  const streams = new Map();
  const visited = new Set();
  const visit = (i) => {
    if (i === 0xffffffff) return;
    if (visited.has(i) || !entries[i]) fail("compound file directory is malformed");
    visited.add(i);
    const e = entries[i];
    visit(e.left);
    visit(e.right);
    if (e.type === 2) streams.set(e.name, read(e));
  };
  visit(root.child);
  return streams;
}

// ---------------------------------------------------------------- MSI tables

const CHARSET = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz._";

/**
 * An MSI stream name as Windows Installer stores it (table streams carry a 0x4840 prefix).
 * @param {string} name
 * @param {boolean} table
 * @returns {string}
 */
export function encodeMsiStreamName(name, table) {
  let out = table ? String.fromCharCode(0x4840) : "";
  for (let i = 0; i < name.length; i++) {
    const a = CHARSET.indexOf(name[i]);
    if (a < 0) {
      out += name[i];
      continue;
    }
    const b = i + 1 < name.length ? CHARSET.indexOf(name[i + 1]) : -1;
    if (b >= 0) {
      out += String.fromCharCode(0x3800 + a + (b << 6));
      i++;
    } else {
      out += String.fromCharCode(0x4800 + a);
    }
  }
  return out;
}

/**
 * The tables of an MSI database that `wanted` names, as arrays of row objects
 * (column name → string, number or null; binary columns → true when set).
 * @param {Map<string, Buffer>} streams from cfbStreams
 * @param {string[]} wanted
 * @returns {Record<string, Record<string, string | number | boolean | null>[]>}
 */
export function msiTables(streams, wanted) {
  const stream = (name) => streams.get(encodeMsiStreamName(name, true));
  const pool = stream("_StringPool");
  const data = stream("_StringData");
  if (!pool || !data) fail("not an MSI database (no string pool)");
  if (pool.length < 4 || pool.length % 4) fail("MSI string pool is malformed");
  const longRefs = (pool.readUInt16LE(2) & 0x8000) !== 0;
  const codepage = pool.readUInt16LE(0) | ((pool.readUInt16LE(2) & 0x7fff) << 16);
  const decoder = codepage === 65001 ? "utf8" : "latin1";
  const strings = [null];
  let offset = 0;
  for (let i = 4; i + 4 <= pool.length; ) {
    let len = pool.readUInt16LE(i);
    const refs = pool.readUInt16LE(i + 2);
    if (len === 0 && refs === 0) {
      strings.push("");
      i += 4;
      continue;
    }
    if (len === 0) {
      if (i + 8 > pool.length) fail("MSI string pool is truncated");
      len = (refs << 16) + pool.readUInt16LE(i + 4);
      i += 8;
    } else {
      i += 4;
    }
    if (offset + len > data.length) fail("MSI string data is truncated");
    strings.push(data.toString(decoder, offset, offset + len));
    offset += len;
  }
  const strRef = longRefs ? 3 : 2;
  const str = (id) => {
    if (id === 0) return null;
    if (id >= strings.length) fail(`MSI string ${id} does not exist`);
    return strings[id];
  };
  const readCols = (buf, widths, what) => {
    const rowSize = widths.reduce((a, b) => a + b, 0);
    if (!buf) return [];
    if (buf.length % rowSize) fail(`MSI table ${what} has a size that is not a whole number of rows`);
    const rows = buf.length / rowSize;
    let base = 0;
    const cols = widths.map((w) => {
      const values = [];
      for (let r = 0; r < rows; r++) {
        const at = base + r * w;
        values.push(w === 4 ? buf.readUInt32LE(at) : w === 3 ? buf.readUIntLE(at, 3) : buf.readUInt16LE(at));
      }
      base += w * rows;
      return values;
    });
    return Array.from({ length: rows }, (_, r) => cols.map((c) => c[r]));
  };
  const int = (raw, w) => (raw === 0 ? null : w === 4 ? raw - 0x80000000 : raw - 0x8000);
  const tables = new Set(readCols(stream("_Tables"), [strRef], "_Tables").map(([n]) => str(n)));
  const columns = readCols(stream("_Columns"), [strRef, 2, strRef, 2], "_Columns").map(([t, n, name, type]) => ({
    table: str(t),
    number: int(n, 2),
    name: str(name),
    type: int(type, 2),
  }));
  const result = {};
  for (const table of wanted) {
    if (!tables.has(table)) {
      result[table] = [];
      continue;
    }
    // Temporary columns (0x4000) are not stored.
    const cols = columns.filter((c) => c.table === table && !(c.type & 0x4000)).sort((a, b) => a.number - b.number);
    const kinds = cols.map((c) => {
      const t = c.type;
      if ((t & ~0x1000) === 0x0900) return { kind: "binary", width: 2 };
      if (t & 0x0800) return { kind: "string", width: strRef };
      const size = t & 0xff;
      if (size !== 2 && size !== 4 && size !== 1) fail(`MSI column ${table}.${c.name} has an unknown type 0x${t.toString(16)}`);
      return { kind: "int", width: size === 4 ? 4 : 2 };
    });
    result[table] = readCols(stream(table), kinds.map((k) => k.width), table).map((row) =>
      Object.fromEntries(
        cols.map((c, i) => {
          const k = kinds[i];
          const v = row[i];
          return [c.name, k.kind === "string" ? str(v) : k.kind === "binary" ? v !== 0 : int(v, k.width)];
        }),
      ),
    );
  }
  return result;
}

/**
 * What is wrong with an MSI's Add/Remove Programs and shortcut icons (empty when nothing is).
 * @param {Buffer} msi
 * @param {Buffer} ico
 * @returns {{ problems: string[], notes: string[], exeFiles: string[] }} exeFiles: File keys of shortcut targets whose icon comes from the target file
 */
export function msiIconProblems(msi, ico) {
  icoEntries(ico);
  const streams = cfbStreams(msi);
  const t = msiTables(streams, ["Property", "Icon", "Shortcut", "File"]);
  const problems = [];
  const notes = [];
  const icons = new Map();
  for (const row of t.Icon) {
    const name = row.Name;
    const data = streams.get(encodeMsiStreamName(`Icon.${name}`, false));
    if (!row.Data || !data) problems.push(`Icon table row ${name} has no data stream`);
    else icons.set(name, data);
  }
  const checkIcon = (name, what) => {
    const data = icons.get(name);
    if (!data) return problems.push(`${what} names icon ${name}, which is not in the Icon table`);
    if (!data.equals(ico)) {
      let detail = "";
      try {
        detail = `; it holds ${icoEntries(data).length} entries`;
      } catch (e) {
        detail = `; it is not a valid ICO (${e.message})`;
      }
      return problems.push(`${what}: icon ${name} (${data.length} bytes) differs from icon.ico (${ico.length} bytes)${detail}`);
    }
    notes.push(`${what}: icon ${name} equals icon.ico (${data.length} bytes, SHA-256 identical)`);
  };
  const arp = t.Property.find((r) => r.Property === "ARPPRODUCTICON")?.Value;
  if (!arp) problems.push("Property ARPPRODUCTICON (the Add/Remove Programs icon) is not set");
  else checkIcon(arp, `ARPPRODUCTICON`);
  if (t.Shortcut.length === 0) problems.push("the Shortcut table is empty");
  const files = new Map(t.File.map((f) => [f.File, f]));
  const exeFiles = [];
  let named = 0;
  for (const s of t.Shortcut) {
    const what = `shortcut ${s.Shortcut}`;
    if (s.Icon_) {
      named++;
      checkIcon(s.Icon_, what);
      continue;
    }
    const m = (s.Target ?? "").match(/^\[[#!]([^\]]+)\]$/);
    const file = m ? files.get(m[1]) : undefined;
    if (file && /(^|\|)sotyware\.exe$/i.test(file.FileName ?? "")) {
      exeFiles.push(file.File);
      notes.push(`${what}: no icon of its own; shows the icon of its target, File ${file.File} (sotyware.exe)`);
    } else if (!m) {
      notes.push(`${what}: target ${s.Target}, not a file of this package; not sotyware's icon (not checked)`);
    } else {
      problems.push(`${what}: no icon, and its target ${s.Target} is not sotyware.exe`);
    }
  }
  if (t.Shortcut.length && named === 0) problems.push("no shortcut names an icon from the Icon table");
  return { problems, notes, exeFiles };
}

// ---------------------------------------------------------------- LZMA

const EOF = Symbol("eof");

/**
 * Decompresses a raw LZMA stream (5 property bytes, then the range-coded
 * data, no size field) until its end marker or the end of the input.
 * @param {Buffer} input
 * @param {{ maxOutput?: number }} [options]
 * @returns {{ data: Buffer, endMarker: boolean, inputUsed: number }}
 */
export function lzmaDecompress(input, { maxOutput = 512 * 1024 * 1024 } = {}) {
  if (input.length < 6) fail("LZMA data is truncated");
  let d = input[0];
  if (d >= 9 * 5 * 5) fail("LZMA properties are malformed");
  const lc = d % 9;
  d = (d / 9) | 0;
  const lp = d % 5;
  const pb = (d / 5) | 0;
  const dictSize = Math.max(input.readUInt32LE(1), 4096);
  let pos = 5;
  const next = () => {
    if (pos >= input.length) throw EOF;
    return input[pos++];
  };
  if (next() !== 0) fail("LZMA data is malformed (first range coder byte is not 0)");
  let code = 0;
  for (let i = 0; i < 4; i++) code = code * 256 + next();
  let range = 0xffffffff;
  if (code >= range) fail("LZMA data is malformed");
  const bit = (probs, i) => {
    const p = probs[i];
    const bound = (range >>> 11) * p;
    let b;
    if (code < bound) {
      range = bound;
      probs[i] = p + ((2048 - p) >>> 5);
      b = 0;
    } else {
      range -= bound;
      code -= bound;
      probs[i] = p - (p >>> 5);
      b = 1;
    }
    if (range < 0x1000000) {
      range *= 256;
      code = code * 256 + next();
    }
    return b;
  };
  const direct = (n) => {
    let res = 0;
    for (; n > 0; n--) {
      range = Math.floor(range / 2);
      let b = 0;
      if (code >= range) {
        code -= range;
        b = 1;
      }
      if (range < 0x1000000) {
        range *= 256;
        code = code * 256 + next();
      }
      res = res * 2 + b;
    }
    return res;
  };
  const probs = (n) => new Uint16Array(n).fill(1024);
  const tree = (p, base, bits) => {
    let m = 1;
    for (let i = 0; i < bits; i++) m = (m << 1) + bit(p, base + m);
    return m - (1 << bits);
  };
  const reverse = (p, base, bits) => {
    let m = 1;
    let sym = 0;
    for (let i = 0; i < bits; i++) {
      const b = bit(p, base + m);
      m = (m << 1) + b;
      sym |= b << i;
    }
    return sym;
  };
  const lenCoder = () => ({ choice: probs(2), low: probs(16 << 3), mid: probs(16 << 3), high: probs(256) });
  const decodeLen = (c, posState) => {
    if (bit(c.choice, 0) === 0) return tree(c.low, posState << 3, 3);
    if (bit(c.choice, 1) === 0) return 8 + tree(c.mid, posState << 3, 3);
    return 16 + tree(c.high, 0, 8);
  };
  const isMatch = probs(12 << 4);
  const isRep = probs(12);
  const isRepG0 = probs(12);
  const isRepG1 = probs(12);
  const isRepG2 = probs(12);
  const isRep0Long = probs(12 << 4);
  const literal = probs(0x300 << (lc + lp));
  const posSlot = probs(4 << 6);
  const posCoders = probs(1 + 114);
  const align = probs(16);
  const lenC = lenCoder();
  const repLenC = lenCoder();
  let out = Buffer.alloc(Math.min(maxOutput, 1 << 20));
  let n = 0;
  const grow = (extra) => {
    if (n + extra <= out.length) return;
    if (n + extra > maxOutput) fail(`LZMA output exceeds ${maxOutput} bytes`);
    const bigger = Buffer.alloc(Math.min(maxOutput, Math.max(out.length * 2, n + extra)));
    out.copy(bigger, 0, 0, n);
    out = bigger;
  };
  let state = 0;
  let rep0 = 0;
  let rep1 = 0;
  let rep2 = 0;
  let rep3 = 0;
  let endMarker = false;
  const pbMask = (1 << pb) - 1;
  const lpMask = (1 << lp) - 1;
  try {
    for (;;) {
      const posState = n & pbMask;
      if (bit(isMatch, (state << 4) + posState) === 0) {
        const prev = n > 0 ? out[n - 1] : 0;
        const base = 0x300 * (((n & lpMask) << lc) + (prev >> (8 - lc)));
        let sym = 1;
        if (state >= 7) {
          if (rep0 >= n) fail("LZMA data is malformed (distance before the start)");
          let match = out[n - rep0 - 1];
          do {
            const mb = (match >> 7) & 1;
            match <<= 1;
            const b = bit(literal, base + ((1 + mb) << 8) + sym);
            sym = (sym << 1) | b;
            if (mb !== b) break;
          } while (sym < 0x100);
        }
        while (sym < 0x100) sym = (sym << 1) | bit(literal, base + sym);
        grow(1);
        out[n++] = sym - 0x100;
        state = state < 4 ? 0 : state < 10 ? state - 3 : state - 6;
        continue;
      }
      let len;
      if (bit(isRep, state) !== 0) {
        if (n === 0) fail("LZMA data is malformed (repeat at the start)");
        if (bit(isRepG0, state) === 0) {
          if (bit(isRep0Long, (state << 4) + posState) === 0) {
            state = state < 7 ? 9 : 11;
            grow(1);
            out[n] = out[n - rep0 - 1];
            n++;
            continue;
          }
        } else {
          let dist;
          if (bit(isRepG1, state) === 0) {
            dist = rep1;
          } else {
            if (bit(isRepG2, state) === 0) {
              dist = rep2;
            } else {
              dist = rep3;
              rep3 = rep2;
            }
            rep2 = rep1;
          }
          rep1 = rep0;
          rep0 = dist;
        }
        len = decodeLen(repLenC, posState);
        state = state < 7 ? 8 : 11;
      } else {
        rep3 = rep2;
        rep2 = rep1;
        rep1 = rep0;
        len = decodeLen(lenC, posState);
        state = state < 7 ? 7 : 10;
        const slot = tree(posSlot, Math.min(len, 3) << 6, 6);
        if (slot < 4) {
          rep0 = slot;
        } else {
          const bits = (slot >> 1) - 1;
          let dist = (2 | (slot & 1)) * 2 ** bits;
          if (slot < 14) dist += reverse(posCoders, dist - slot, bits);
          else dist += direct(bits - 4) * 16 + reverse(align, 0, 4);
          rep0 = dist;
        }
        if (rep0 === 0xffffffff) {
          endMarker = true;
          break;
        }
        if (rep0 >= n || rep0 >= dictSize) fail("LZMA data is malformed (distance before the start)");
      }
      len += 2;
      grow(len);
      for (let i = 0; i < len; i++, n++) out[n] = out[n - rep0 - 1];
    }
  } catch (e) {
    if (e !== EOF) throw e;
  }
  return { data: out.subarray(0, n), endMarker, inputUsed: pos };
}

// ---------------------------------------------------------------- NSIS

/**
 * The parts of an NSIS setup program this check needs.
 * @param {Buffer} exe
 * @returns {{ stub: Buffer, header: Buffer, items: Buffer[] }} stub: the bytes before NSIS's data (the PE NSIS copies into the uninstaller); items: the data block entries
 */
export function nsisParts(exe) {
  let fh = -1;
  for (let off = 512; off + 28 <= exe.length; off += 512) {
    if (exe.readUInt32LE(off + 4) === 0xdeadbeef && exe.toString("latin1", off + 8, off + 20) === "NullsoftInst") {
      fh = off;
      break;
    }
  }
  if (fh < 0) fail("not an NSIS installer (no NullsoftInst header)");
  const flags = exe.readUInt32LE(fh);
  if (flags & 1) fail("this NSIS file is an uninstaller, not a setup program");
  const headerLength = exe.readUInt32LE(fh + 20);
  const total = exe.readUInt32LE(fh + 24);
  need(exe, fh, total, "NSIS data");
  // The data ends with a CRC32 of everything before it.
  const start = fh + 28;
  const end = fh + total - 4;
  if (end <= start) fail("NSIS data is truncated");
  const first = exe[start];
  if (first !== 0x5d && !(first <= 1 && exe[start + 1] === 0x5d)) {
    fail("NSIS data is not solid LZMA (Tauri's default compression); this check reads only that");
  }
  if (first === 1) fail("NSIS data uses the LZMA x86 filter, which this check cannot read");
  const input = exe.subarray(first === 0 ? start + 1 : start, end);
  const { data, endMarker, inputUsed } = lzmaDecompress(input);
  // NSIS's LZMA encoder writes an end marker; without it the data was cut short.
  if (!endMarker) fail("NSIS data is truncated (no LZMA end marker)");
  if (inputUsed !== input.length) fail(`NSIS data is malformed (${input.length - inputUsed} bytes after the LZMA end marker)`);
  if (data.length < 4 || data.readUInt32LE(0) !== headerLength) fail("NSIS data is malformed (header length does not match)");
  need(data, 4, headerLength, "NSIS header");
  const items = [];
  let at = 4 + headerLength;
  while (at < data.length) {
    if (at + 4 > data.length) fail("NSIS data block is malformed (partial length)");
    const len = data.readUInt32LE(at);
    if (len & 0x80000000) fail("NSIS data block is malformed (compressed entry in solid data)");
    if (at + 4 + len > data.length) fail("NSIS data block is truncated");
    items.push(data.subarray(at + 4, at + 4 + len));
    at += 4 + len;
  }
  return { stub: exe.subarray(0, fh), header: data.subarray(4, 4 + headerLength), items };
}

/**
 * The patches of NSIS uninstaller icon data ({size, offset, bytes}… then a
 * zero size), or null when `item` is not exactly that.
 * @param {Buffer} item
 * @param {number} stubLength
 * @returns {{ offset: number, data: Buffer }[] | null}
 */
export function uninstallerIconPatches(item, stubLength) {
  const patches = [];
  let at = 0;
  for (;;) {
    if (at + 4 > item.length) return null;
    const size = item.readUInt32LE(at);
    if (size === 0) break;
    if (at + 8 + size > item.length) return null;
    const offset = item.readUInt32LE(at + 4);
    if (offset < 64 || offset + size > stubLength) return null;
    patches.push({ offset, data: item.subarray(at + 8, at + 8 + size) });
    at += 8 + size;
  }
  return patches.length > 0 && at + 4 === item.length ? patches : null;
}

/**
 * The uninstaller NSIS would write: the setup program's header with the
 * uninstaller icon patched in. The rest of the uninstaller is NSIS data, not
 * PE resources.
 * @param {Buffer} exe the setup program
 * @returns {{ header: Buffer, patches: { offset: number, data: Buffer }[] }}
 */
export function nsisUninstallerHeader(exe) {
  const { stub, items } = nsisParts(exe);
  const candidates = items.map((item) => uninstallerIconPatches(item, stub.length)).filter((p) => p !== null);
  if (candidates.length === 0) fail("the NSIS data holds no uninstaller icon data (no WriteUninstaller, or not readable)");
  if (candidates.length > 1) fail(`the NSIS data holds ${candidates.length} candidates for the uninstaller icon data, expected one`);
  const header = Buffer.from(stub);
  for (const p of candidates[0]) p.data.copy(header, p.offset);
  return { header, patches: candidates[0] };
}

/**
 * What is wrong with the uninstaller icon of an NSIS setup program (empty
 * when nothing is). Besides the icon group check, every icon resource must
 * have been written by the uninstaller icon data: otherwise what is checked
 * could be the setup program's own icon left in place.
 * @param {Buffer} exe
 * @param {Buffer} ico
 * @returns {string[]}
 */
export function nsisUninstallerProblems(exe, ico) {
  const { header, patches } = nsisUninstallerHeader(exe);
  const problems = peIconProblems(header, ico);
  const covered = (off, len) => patches.some((p) => off >= p.offset && off + len <= p.offset + p.data.length);
  for (const r of peResources(header)) {
    if (r.type !== RT_ICON && r.type !== RT_GROUP_ICON) continue;
    const off = r.data.byteOffset - header.byteOffset;
    if (!covered(off, r.data.length)) {
      problems.push(`${r.type === RT_ICON ? "RT_ICON" : "RT_GROUP_ICON"} ${r.id} is not written by the uninstaller icon data (it is the setup program's)`);
    }
  }
  return problems;
}

// ---------------------------------------------------------------- CLI

function findFiles(dir, test) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...findFiles(path, test));
    else if (test(name)) out.push(path);
  }
  return out;
}

/**
 * Runs the checks; returns the report lines and whether everything passed.
 * @param {{ ico: string, msi: string[], msiImage: string[], nsis: string[], exe: string[] }} opts
 * @returns {{ ok: boolean, lines: string[] }}
 */
export function runChecks(opts) {
  const lines = [];
  let ok = true;
  const ico = readFileSync(opts.ico);
  const entries = icoEntries(ico);
  lines.push(`expected ${opts.ico}: ${ico.length} bytes, ${entries.map((e) => e.width || 256).join("/")} px`);
  const check = (label, fn) => {
    try {
      const problems = fn();
      if (problems.length === 0) {
        lines.push(`ok   ${label}`);
      } else {
        ok = false;
        for (const p of problems) lines.push(`FAIL ${label}: ${p}`);
      }
    } catch (e) {
      ok = false;
      lines.push(`FAIL ${label}: ${e instanceof IconCheckError || e.code ? e.message : e.stack}`);
    }
  };
  const msiCheck = (path, image) => {
    let exeFiles = [];
    check(`MSI ${path}: Add/Remove Programs and shortcut icons`, () => {
      const r = msiIconProblems(readFileSync(path), ico);
      for (const n of r.notes) lines.push(`     ${n}`);
      exeFiles = r.exeFiles;
      return r.problems;
    });
    return exeFiles;
  };
  const exeCheck = (label, path) => check(`${label} ${path}: icon group`, () => peIconProblems(readFileSync(path), ico));
  for (const path of opts.msi) msiCheck(path);
  for (const dir of opts.msiImage) {
    check(`MSI administrative image ${dir}`, () => {
      const msis = findFiles(dir, (n) => n.toLowerCase().endsWith(".msi"));
      const exes = findFiles(dir, (n) => n.toLowerCase() === "sotyware.exe");
      const problems = [];
      if (msis.length !== 1) problems.push(`expected one .msi, found ${msis.length}`);
      if (exes.length !== 1) problems.push(`expected one sotyware.exe, found ${exes.length}`);
      if (problems.length) return problems;
      const exeFiles = msiCheck(msis[0]);
      if (exeFiles.length === 0) lines.push("     no shortcut takes its icon from sotyware.exe");
      exeCheck("MSI administrative image's sotyware.exe", exes[0]);
      return [];
    });
  }
  for (const path of opts.nsis) {
    check(`NSIS setup program ${path}: icon group`, () => peIconProblems(readFileSync(path), ico));
    check(`NSIS uninstaller (reconstructed from ${path}): icon group`, () => nsisUninstallerProblems(readFileSync(path), ico));
  }
  for (const path of opts.exe) exeCheck("executable", path);
  if (opts.msi.length + opts.msiImage.length + opts.nsis.length + opts.exe.length === 0) {
    ok = false;
    lines.push("FAIL nothing to check: pass --msi, --msi-image, --nsis or --exe");
  }
  return { ok, lines };
}

/**
 * The options of a command line.
 * @param {string[]} args
 */
export function parseArgs(args) {
  const opts = { ico: DEFAULT_ICO, msi: [], msiImage: [], nsis: [], exe: [] };
  const keys = { "--msi": "msi", "--msi-image": "msiImage", "--nsis": "nsis", "--exe": "exe" };
  for (let i = 0; i < args.length; i++) {
    const value = args[i + 1];
    if (!(args[i] in keys) && args[i] !== "--ico") throw new Error(`unknown argument ${args[i]}`);
    if (value === undefined || value.startsWith("--")) throw new Error(`${args[i]} needs a path`);
    if (args[i] === "--ico") opts.ico = value;
    else opts[keys[args[i]]].push(value);
    i++;
  }
  return opts;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  let opts;
  try {
    opts = parseArgs(process.argv.slice(2));
  } catch (e) {
    console.error(`${PREFIX}${e.message}`);
    console.error(`${PREFIX}usage: node scripts/check-installer-icons.mjs [--ico <icon.ico>] [--msi <file.msi>] [--msi-image <dir>] [--nsis <setup.exe>] [--exe <file.exe>]`);
    process.exit(2);
  }
  const { ok, lines } = runChecks(opts);
  for (const l of lines) (l.startsWith("FAIL") ? console.error : console.log)(`${PREFIX}${l}`);
  if (!ok) process.exit(1);
  console.log(`${PREFIX}ok`);
}
