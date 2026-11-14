// Tests for scripts/check-installer-icons.mjs: `npm run test:installer-icons`.
//
// The fixtures are built here, in memory: minimal PE files with icon
// resources, compound files holding MSI tables, and NSIS setup programs
// whose data is LZMA-compressed by a small literal-only encoder below. The
// real MSI and NSIS installers are checked by release-build.yml.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  cfbStreams,
  encodeMsiStreamName,
  IconCheckError,
  icoEntries,
  lzmaDecompress,
  msiIconProblems,
  msiTables,
  nsisParts,
  nsisUninstallerProblems,
  parseArgs,
  peIconProblems,
  peResources,
  runChecks,
} from "./check-installer-icons.mjs";
import { buildIco, readIco } from "./icons.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "check-installer-icons.mjs");
const icoPath = resolve(here, "..", "src-tauri", "icons", "icon.ico");
const ico = readFileSync(icoPath);

/** icon.ico with one byte of its 256 px image changed: same sizes, different picture. */
function staleIco() {
  const copy = Buffer.from(ico);
  const last = icoEntries(copy).at(-1);
  const at = last.data.byteOffset - copy.byteOffset + Math.floor(last.data.length / 2);
  copy[at] ^= 0xff;
  return copy;
}
/** icon.ico without its 16 px entry. */
const shortIco = () => buildIco(readIco(ico).map((e) => e.png).slice(1));

// ------------------------------------------------------------ PE fixtures

/** The RT_ICON and RT_GROUP_ICON resources an ICO becomes in a PE file. */
function iconResources(source, { firstId = 1, groupId = 1, lang = 1033 } = {}) {
  const entries = icoEntries(source);
  const out = entries.map((e, i) => ({ type: 3, id: firstId + i, lang, data: e.data }));
  const g = Buffer.alloc(6 + 14 * entries.length);
  g.writeUInt16LE(1, 2);
  g.writeUInt16LE(entries.length, 4);
  entries.forEach((e, i) => {
    const o = 6 + 14 * i;
    g[o] = e.width;
    g[o + 1] = e.height;
    g[o + 2] = e.colors;
    g.writeUInt16LE(e.planes, o + 4);
    g.writeUInt16LE(e.bitCount, o + 6);
    g.writeUInt32LE(e.data.length, o + 8);
    g.writeUInt16LE(firstId + i, o + 12);
  });
  out.push({ type: 14, id: groupId, lang, data: g });
  return out;
}

/** A minimal PE32+ file whose only section is .rsrc with these resources. */
function buildPe(resources) {
  const va = 0x1000;
  const tree = new Map();
  for (const r of resources) {
    if (!tree.has(r.type)) tree.set(r.type, new Map());
    const ids = tree.get(r.type);
    if (!ids.has(r.id)) ids.set(r.id, new Map());
    ids.get(r.id).set(r.lang, r.data);
  }
  const sorted = (m) => [...m.entries()].sort(([a], [b]) => a - b);
  let off = 16 + 8 * tree.size;
  const typeDirs = new Map();
  for (const [type, ids] of sorted(tree)) {
    typeDirs.set(type, off);
    off += 16 + 8 * ids.size;
  }
  const idDirs = new Map();
  for (const [type, ids] of sorted(tree)) {
    for (const [id, langs] of sorted(ids)) {
      idDirs.set(`${type}/${id}`, off);
      off += 16 + 8 * langs.size;
    }
  }
  const leaves = [];
  for (const [type, ids] of sorted(tree)) for (const [id, langs] of sorted(ids)) for (const [lang, data] of sorted(langs)) leaves.push({ type, id, lang, data });
  for (const leaf of leaves) {
    leaf.entry = off;
    off += 16;
  }
  for (const leaf of leaves) {
    off = Math.ceil(off / 8) * 8;
    leaf.at = off;
    off += leaf.data.length;
  }
  const rsrc = Buffer.alloc(Math.ceil(off / 512) * 512);
  const dir = (at, entries) => {
    rsrc.writeUInt16LE(entries.length, at + 14);
    entries.forEach(([id, target], i) => {
      rsrc.writeUInt32LE(id, at + 16 + 8 * i);
      rsrc.writeUInt32LE(target, at + 20 + 8 * i);
    });
  };
  dir(0, sorted(tree).map(([type]) => [type, (typeDirs.get(type) | 0x80000000) >>> 0]));
  for (const [type, ids] of sorted(tree)) {
    dir(typeDirs.get(type), sorted(ids).map(([id]) => [id, (idDirs.get(`${type}/${id}`) | 0x80000000) >>> 0]));
    for (const [id, langs] of sorted(ids)) {
      dir(idDirs.get(`${type}/${id}`), sorted(langs).map(([lang]) => [lang, leaves.find((l) => l.type === type && l.id === id && l.lang === lang).entry]));
    }
  }
  for (const leaf of leaves) {
    rsrc.writeUInt32LE(va + leaf.at, leaf.entry);
    rsrc.writeUInt32LE(leaf.data.length, leaf.entry + 4);
    leaf.data.copy(rsrc, leaf.at);
  }
  const head = Buffer.alloc(512);
  head.write("MZ", 0, "latin1");
  head.writeUInt32LE(0x40, 0x3c);
  head.write("PE\0\0", 0x40, "latin1");
  head.writeUInt16LE(0x8664, 0x44);
  head.writeUInt16LE(1, 0x46);
  head.writeUInt16LE(240, 0x54);
  const opt = 0x58;
  head.writeUInt16LE(0x20b, opt);
  head.writeUInt32LE(16, opt + 108);
  head.writeUInt32LE(va, opt + 112 + 16);
  head.writeUInt32LE(off, opt + 112 + 20);
  const sect = opt + 240;
  head.write(".rsrc", sect, "latin1");
  head.writeUInt32LE(off, sect + 8);
  head.writeUInt32LE(va, sect + 12);
  head.writeUInt32LE(rsrc.length, sect + 16);
  head.writeUInt32LE(512, sect + 20);
  return Buffer.concat([head, rsrc]);
}

// ------------------------------------------------------------ MSI fixtures

/** A version 3 compound file with these streams under its root. */
function buildCfb(streams) {
  const S = 512;
  const FREE = 0xffffffff;
  const END = 0xfffffffe;
  const entries = [...streams.entries()];
  const big = entries.filter(([, d]) => d.length >= 4096);
  const small = entries.filter(([, d]) => d.length < 4096);
  const miniSectors = new Map();
  let miniCount = 0;
  for (const [name, d] of small) {
    miniSectors.set(name, miniCount);
    miniCount += Math.ceil(d.length / 64) || 0;
  }
  const mini = Buffer.alloc(miniCount * 64);
  for (const [name, d] of small) d.copy(mini, miniSectors.get(name) * 64);
  const dirSectors = Math.ceil(((entries.length + 1) * 128) / S);
  const miniFatSectors = Math.ceil((miniCount * 4) / S);
  const miniStreamSectors = Math.ceil(mini.length / S);
  const bigSectors = big.reduce((a, [, d]) => a + Math.ceil(d.length / S), 0);
  const rest = dirSectors + miniFatSectors + miniStreamSectors + bigSectors;
  let fatSectors = 1;
  while (fatSectors * 128 < fatSectors + rest) fatSectors++;
  const fat = new Array(fatSectors * 128).fill(FREE);
  let next = 0;
  for (let i = 0; i < fatSectors; i++) fat[next++] = 0xfffffffd;
  const run = (count) => {
    if (count === 0) return END;
    const start = next;
    for (let i = 0; i < count; i++, next++) fat[next] = i === count - 1 ? END : next + 1;
    return start;
  };
  const dirStart = run(dirSectors);
  const miniFatStart = run(miniFatSectors);
  const miniStart = run(miniStreamSectors);
  const bigStarts = big.map(([, d]) => run(Math.ceil(d.length / S)));
  const out = Buffer.alloc(S * (1 + next));
  const sector = (n) => S * (n + 1);
  out.writeUInt32LE(0xe011cfd0, 0);
  out.writeUInt32LE(0xe11ab1a1, 4);
  out.writeUInt16LE(0x3e, 0x18);
  out.writeUInt16LE(3, 0x1a);
  out.writeUInt16LE(0xfffe, 0x1c);
  out.writeUInt16LE(9, 0x1e);
  out.writeUInt16LE(6, 0x20);
  out.writeUInt32LE(fatSectors, 0x2c);
  out.writeUInt32LE(dirStart, 0x30);
  out.writeUInt32LE(4096, 0x38);
  out.writeUInt32LE(miniFatSectors ? miniFatStart : END, 0x3c);
  out.writeUInt32LE(miniFatSectors, 0x40);
  out.writeUInt32LE(END, 0x44);
  for (let i = 0; i < 109; i++) out.writeUInt32LE(i < fatSectors ? i : FREE, 0x4c + 4 * i);
  fat.forEach((v, i) => out.writeUInt32LE(v, sector(Math.floor(i / 128)) + 4 * (i % 128)));
  const miniFat = new Array(miniFatSectors * 128).fill(FREE);
  for (const [name, d] of small) {
    const first = miniSectors.get(name);
    const n = Math.ceil(d.length / 64);
    for (let i = 0; i < n; i++) miniFat[first + i] = i === n - 1 ? END : first + i + 1;
  }
  miniFat.forEach((v, i) => out.writeUInt32LE(v, sector(miniFatStart + Math.floor(i / 128)) + 4 * (i % 128)));
  for (let i = 0; i < miniStreamSectors; i++) mini.copy(out, sector(miniStart + i), i * S, Math.min(mini.length, (i + 1) * S));
  big.forEach(([, d], b) => {
    for (let i = 0; i * S < d.length; i++) d.copy(out, sector(bigStarts[b] + i), i * S, Math.min(d.length, (i + 1) * S));
  });
  const dirBuf = Buffer.alloc(dirSectors * S);
  const writeEntry = (i, name, type, right, child, start, size) => {
    const o = i * 128;
    dirBuf.write(name, o, "utf16le");
    dirBuf.writeUInt16LE((name.length + 1) * 2, o + 64);
    dirBuf[o + 66] = type;
    dirBuf[o + 67] = 1;
    dirBuf.writeUInt32LE(FREE, o + 68);
    dirBuf.writeUInt32LE(right, o + 72);
    dirBuf.writeUInt32LE(child, o + 76);
    dirBuf.writeUInt32LE(start, o + 116);
    dirBuf.writeUInt32LE(size, o + 120);
  };
  writeEntry(0, "Root Entry", 5, FREE, entries.length ? 1 : FREE, mini.length ? miniStart : END, mini.length);
  entries.forEach(([name, d], i) => {
    const bigIndex = big.findIndex(([n]) => n === name);
    const start = bigIndex >= 0 ? bigStarts[bigIndex] : d.length ? miniSectors.get(name) : END;
    writeEntry(i + 1, name, 2, i + 1 < entries.length ? i + 2 : FREE, FREE, start, d.length);
  });
  for (let i = 0; i < dirSectors; i++) dirBuf.copy(out, sector(dirStart + i), i * S, (i + 1) * S);
  return out;
}

const COLUMNS = {
  Property: [
    ["Property", 0x2d48],
    ["Value", 0x0f00],
  ],
  Icon: [
    ["Name", 0x2d48],
    ["Data", 0x0900],
  ],
  Shortcut: [
    ["Shortcut", 0x2d48],
    ["Target", 0x0d48],
    ["Icon_", 0x1d48],
    ["IconIndex", 0x1502],
  ],
  File: [
    ["File", 0x2d48],
    ["FileName", 0x0fff],
    ["FileSize", 0x0104],
  ],
};

/**
 * An MSI database (as a compound file) with the tables this check reads.
 * `tables` maps a table name to its rows; Icon rows carry a Buffer as Data.
 */
function buildMsi(tables, { omit = [] } = {}) {
  const strings = [];
  const id = (s) => {
    if (s === null || s === undefined) return 0;
    let i = strings.indexOf(s);
    if (i < 0) i = strings.push(s) - 1;
    return i + 1;
  };
  const streams = new Map();
  const tableStream = (cols, rows) => {
    const parts = cols.map(([, w, encode]) => {
      const b = Buffer.alloc(w * rows.length);
      rows.forEach((r, i) => (w === 4 ? b.writeUInt32LE(encode(r), i * w) : b.writeUInt16LE(encode(r), i * w)));
      return b;
    });
    return Buffer.concat(parts);
  };
  const names = Object.keys(tables);
  const columns = [];
  for (const t of names) COLUMNS[t].forEach(([name, type], i) => columns.push([t, i + 1, name, type]));
  for (const t of names) {
    const cols = COLUMNS[t].map(([name, type]) => {
      if ((type & ~0x1000) === 0x0900) return [name, 2, (r) => (r[name] ? 1 : 0)];
      if (type & 0x0800) return [name, 2, (r) => id(r[name])];
      if ((type & 0xff) === 4) return [name, 4, (r) => (r[name] === null || r[name] === undefined ? 0 : r[name] + 0x80000000)];
      return [name, 2, (r) => (r[name] === null || r[name] === undefined ? 0 : r[name] + 0x8000)];
    });
    streams.set(encodeMsiStreamName(t, true), tableStream(cols, tables[t]));
    if (t === "Icon") for (const r of tables[t]) if (r.Data) streams.set(encodeMsiStreamName(`Icon.${r.Name}`, false), r.Data);
  }
  streams.set(encodeMsiStreamName("_Tables", true), tableStream([[0, 2, (t) => id(t)]], names));
  streams.set(
    encodeMsiStreamName("_Columns", true),
    tableStream(
      [
        [0, 2, (c) => id(c[0])],
        [0, 2, (c) => c[1] + 0x8000],
        [0, 2, (c) => id(c[2])],
        [0, 2, (c) => c[3] + 0x8000],
      ],
      columns,
    ),
  );
  const pool = Buffer.alloc(4 + 4 * strings.length);
  pool.writeUInt16LE(1252, 0);
  strings.forEach((s, i) => {
    pool.writeUInt16LE(Buffer.byteLength(s, "latin1"), 4 + 4 * i);
    pool.writeUInt16LE(1, 6 + 4 * i);
  });
  streams.set(encodeMsiStreamName("_StringPool", true), pool);
  streams.set(encodeMsiStreamName("_StringData", true), Buffer.from(strings.join(""), "latin1"));
  for (const name of omit) streams.delete(name);
  return buildCfb(streams);
}

/** The tables of a Tauri MSI, with `icon` as its ProductIcon. */
function tauriTables(icon = ico) {
  return {
    Property: [
      { Property: "ARPPRODUCTICON", Value: "ProductIcon" },
      { Property: "ProductName", Value: "sotyware" },
    ],
    Icon: [{ Name: "ProductIcon", Data: icon }],
    Shortcut: [
      { Shortcut: "ApplicationStartMenuShortcut", Target: "[!Path]", Icon_: "ProductIcon", IconIndex: null },
      { Shortcut: "ApplicationDesktopShortcut", Target: "[!Path]", Icon_: null, IconIndex: null },
      { Shortcut: "UninstallShortcut", Target: "[System64Folder]msiexec.exe", Icon_: null, IconIndex: null },
    ],
    File: [{ File: "Path", FileName: "sotyware.exe", FileSize: 10381824 }],
  };
}

// ------------------------------------------------------------ NSIS fixtures

/** Raw LZMA (lc 3, lp 0, pb 2) of `data`, literals only, with an end marker. */
function lzmaLiterals(data, { endMarker = true } = {}) {
  const out = [];
  let low = 0;
  let range = 0xffffffff;
  let cache = 0;
  let cacheSize = 1;
  const shiftLow = () => {
    if (low < 0xff000000 || low >= 2 ** 32) {
      const carry = low >= 2 ** 32 ? 1 : 0;
      let temp = cache;
      do {
        out.push((temp + carry) & 0xff);
        temp = 0xff;
      } while (--cacheSize !== 0);
      cache = Math.floor(low / 2 ** 24) & 0xff;
    }
    cacheSize++;
    low = (low % 2 ** 24) * 256;
  };
  const bit = (probs, i, b) => {
    const p = probs[i];
    const bound = (range >>> 11) * p;
    if (b === 0) {
      range = bound;
      probs[i] = p + ((2048 - p) >>> 5);
    } else {
      low += bound;
      range -= bound;
      probs[i] = p - (p >>> 5);
    }
    while (range < 0x1000000) {
      range *= 256;
      shiftLow();
    }
  };
  const direct = (value, n) => {
    for (let i = n - 1; i >= 0; i--) {
      range = Math.floor(range / 2);
      if (Math.floor(value / 2 ** i) % 2) low += range;
      while (range < 0x1000000) {
        range *= 256;
        shiftLow();
      }
    }
  };
  const probs = (n) => new Uint16Array(n).fill(1024);
  const isMatch = probs(12 << 4);
  const isRep = probs(12);
  const literal = probs(0x300 << 3);
  const choice = probs(2);
  const lenLow = probs(16 << 3);
  const posSlot = probs(4 << 6);
  const align = probs(16);
  let n = 0;
  for (const byte of data) {
    bit(isMatch, n & 3, 0);
    const base = 0x300 * ((n > 0 ? data[n - 1] : 0) >> 5);
    let sym = 1;
    for (let i = 7; i >= 0; i--) {
      const b = (byte >> i) & 1;
      bit(literal, base + sym, b);
      sym = (sym << 1) | b;
    }
    n++;
  }
  if (endMarker) {
    const posState = n & 3;
    bit(isMatch, posState, 1);
    bit(isRep, 0, 0);
    bit(choice, 0, 0);
    for (let m = 1, i = 0; i < 3; i++, m <<= 1) bit(lenLow, (posState << 3) + m, 0);
    for (let m = 1, i = 0; i < 6; i++, m = (m << 1) | 1) bit(posSlot, m, 1);
    direct(2 ** 26 - 1, 26);
    for (let m = 1, i = 0; i < 4; i++, m = (m << 1) | 1) bit(align, m, 1);
  }
  for (let i = 0; i < 5; i++) shiftLow();
  const props = Buffer.from([0x5d, 0, 0, 1, 0]);
  return Buffer.concat([props, Buffer.from(out)]);
}

/** NSIS uninstaller icon data that writes `resources` over the stub's own icon resources. */
function iconPatchItem(stub, resources, { skip = [] } = {}) {
  const parts = [];
  for (const r of peResources(stub)) {
    if (r.type !== 3 && r.type !== 14) continue;
    if (skip.includes(`${r.type}/${r.id}`)) continue;
    const data = resources.find((x) => x.type === r.type && x.id === r.id)?.data ?? r.data;
    const head = Buffer.alloc(8);
    head.writeUInt32LE(data.length, 0);
    head.writeUInt32LE(r.data.byteOffset - stub.byteOffset, 4);
    parts.push(head, data);
  }
  parts.push(Buffer.alloc(4));
  return Buffer.concat(parts);
}

/** An NSIS setup program: `stub`, then a first header and solid LZMA data holding a header and `items`. */
function buildNsis(stub, items, { endMarker = true, flags = 0 } = {}) {
  const padded = Buffer.concat([stub, Buffer.alloc((512 - (stub.length % 512)) % 512)]);
  const header = Buffer.alloc(300, 0x11);
  const u32 = (v) => {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(v);
    return b;
  };
  const payload = Buffer.concat([u32(header.length), header, ...items.flatMap((it) => [u32(it.length), it])]);
  const lz = lzmaLiterals(payload, { endMarker });
  const fh = Buffer.alloc(28);
  fh.writeUInt32LE(flags, 0);
  fh.writeUInt32LE(0xdeadbeef, 4);
  fh.write("NullsoftInst", 8, "latin1");
  fh.writeUInt32LE(header.length, 20);
  fh.writeUInt32LE(28 + lz.length + 4, 24);
  return Buffer.concat([padded, fh, lz, Buffer.alloc(4)]);
}

const setupStub = (source = ico) => buildPe([...iconResources(source), { type: 24, id: 1, lang: 1033, data: Buffer.from("<assembly/>") }]);
const goodNsis = () => {
  const stub = setupStub();
  return buildNsis(stub, [Buffer.from("a file"), iconPatchItem(stub, iconResources(ico)), Buffer.alloc(5000, 0x42)]);
};

function withTemp(fn) {
  const dir = mkdtempSync(join(tmpdir(), "sotyware-installer-icons-test-"));
  try {
    return fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

// ------------------------------------------------------------ PE

test("a PE file with exactly icon.ico passes", () => {
  assert.deepEqual(peIconProblems(buildPe(iconResources(ico)), ico), []);
});

test("a PE file without an icon fails", () => {
  const pe = buildPe([{ type: 24, id: 1, lang: 1033, data: Buffer.from("<assembly/>") }]);
  assert.deepEqual(peIconProblems(pe, ico), ["no icon group (RT_GROUP_ICON)"]);
});

test("a PE file with a stale or incomplete icon fails", () => {
  const stale = peIconProblems(buildPe(iconResources(staleIco())), ico);
  assert.deepEqual(stale, ["entry 5 (256 px, RT_ICON 6): image differs from icon.ico entry 5 (256 px)"]);
  const short = peIconProblems(buildPe(iconResources(shortIco())), ico);
  assert.ok(short.includes("the icon group has 5 entries, icon.ico has 6"), short.join("\n"));
  assert.ok(short.some((p) => p.includes("width is 24, icon.ico says 16")), short.join("\n"));
});

test("a PE file with a second icon group or a stray icon fails", () => {
  const two = peIconProblems(buildPe([...iconResources(ico), ...iconResources(ico, { firstId: 20, groupId: 2 })]), ico);
  assert.match(two[0], /^2 icon groups, expected one/);
  const stray = peIconProblems(buildPe([...iconResources(ico), { type: 3, id: 99, lang: 1033, data: Buffer.alloc(40) }]), ico);
  assert.deepEqual(stray, ["RT_ICON resources outside the icon group: 99"]);
});

test("a malformed PE file fails clearly", () => {
  const pe = buildPe(iconResources(ico));
  assert.throws(() => peIconProblems(Buffer.from("plainly not an executable file, but long enough to read".repeat(2)), ico), /not a PE file \(no MZ header\)/);
  assert.throws(() => peIconProblems(pe.subarray(0, 700), ico), (e) => e instanceof IconCheckError && /outside the file|not in the file/.test(e.message));
  const badRva = Buffer.from(pe);
  badRva.writeUInt32LE(0x7000, 0x58 + 112 + 16);
  assert.throws(() => peIconProblems(badRva, ico), /resource directory \(RVA 0x7000\) is not in the file/);
});

// ------------------------------------------------------------ MSI

test("MSI stream names are encoded as Windows Installer stores them", () => {
  // Seen in a WiX-built MSI: the Icon table's stream and the _Tables table.
  const hex = (s) => [...s].map((c) => c.charCodeAt(0).toString(16)).join(" ");
  assert.equal(hex(encodeMsiStreamName("Icon.ProductIcon", false)), "4192 4472 3e7e 44b5 4627 45e6 4192 4472");
  assert.equal(hex(encodeMsiStreamName("_Tables", true)), "4840 3f7f 4164 422f 4836");
});

test("MSI tables read back as written", () => {
  const streams = cfbStreams(buildMsi(tauriTables()));
  const t = msiTables(streams, ["Property", "Shortcut", "File", "Missing"]);
  assert.deepEqual(t.Property[0], { Property: "ARPPRODUCTICON", Value: "ProductIcon" });
  assert.deepEqual(t.Shortcut[2], { Shortcut: "UninstallShortcut", Target: "[System64Folder]msiexec.exe", Icon_: null, IconIndex: null });
  assert.deepEqual(t.File, [{ File: "Path", FileName: "sotyware.exe", FileSize: 10381824 }]);
  assert.deepEqual(t.Missing, []);
  // icon.ico (15,800 bytes) is above the mini stream cutoff: read from regular sectors.
  assert.ok(streams.get(encodeMsiStreamName("Icon.ProductIcon", false)).equals(ico));
});

test("an MSI whose product and shortcut icon is icon.ico passes", () => {
  const r = msiIconProblems(buildMsi(tauriTables()), ico);
  assert.deepEqual(r.problems, []);
  assert.deepEqual(r.exeFiles, ["Path"]);
  assert.ok(r.notes.includes("ARPPRODUCTICON: icon ProductIcon equals icon.ico (15800 bytes, SHA-256 identical)"), r.notes.join("\n"));
  assert.ok(r.notes.some((n) => n.startsWith("shortcut UninstallShortcut: target [System64Folder]msiexec.exe")), r.notes.join("\n"));
});

test("an MSI with a small icon (in the mini stream) is read too", () => {
  const tiny = buildIco([readIco(ico)[0].png]);
  assert.ok(tiny.length < 4096);
  assert.deepEqual(msiIconProblems(buildMsi(tauriTables(tiny)), tiny).problems, []);
});

test("an MSI without ARPPRODUCTICON, Icon row or icon stream fails", () => {
  const t = tauriTables();
  t.Property = t.Property.filter((p) => p.Property !== "ARPPRODUCTICON");
  assert.deepEqual(msiIconProblems(buildMsi(t), ico).problems, ["Property ARPPRODUCTICON (the Add/Remove Programs icon) is not set"]);
  const noRow = tauriTables();
  noRow.Icon = [];
  assert.deepEqual(msiIconProblems(buildMsi(noRow), ico).problems, [
    "ARPPRODUCTICON names icon ProductIcon, which is not in the Icon table",
    "shortcut ApplicationStartMenuShortcut names icon ProductIcon, which is not in the Icon table",
  ]);
  const noStream = buildMsi(tauriTables(), { omit: [encodeMsiStreamName("Icon.ProductIcon", false)] });
  assert.ok(msiIconProblems(noStream, ico).problems.includes("Icon table row ProductIcon has no data stream"));
});

test("an MSI with a stale, truncated or other icon fails", () => {
  const stale = msiIconProblems(buildMsi(tauriTables(staleIco())), ico).problems;
  assert.equal(stale.length, 2);
  assert.match(stale[0], /^ARPPRODUCTICON: icon ProductIcon \(15800 bytes\) differs from icon\.ico \(15800 bytes\); it holds 6 entries$/);
  const short = msiIconProblems(buildMsi(tauriTables(shortIco())), ico).problems;
  assert.match(short[0], /differs from icon\.ico .*; it holds 5 entries$/);
  const broken = msiIconProblems(buildMsi(tauriTables(ico.subarray(0, 100))), ico).problems;
  assert.match(broken[0], /differs from icon\.ico .*; it is not a valid ICO/);
});

test("an MSI whose shortcuts name no icon or target another file fails", () => {
  const t = tauriTables();
  t.Shortcut[0].Icon_ = null;
  t.Shortcut[1].Target = "[!Other]";
  t.File.push({ File: "Other", FileName: "other.exe", FileSize: 1 });
  assert.deepEqual(msiIconProblems(buildMsi(t), ico).problems, [
    "shortcut ApplicationDesktopShortcut: no icon, and its target [!Other] is not sotyware.exe",
    "no shortcut names an icon from the Icon table",
  ]);
  const second = tauriTables();
  second.Icon.push({ Name: "OldIcon", Data: staleIco() });
  second.Shortcut[1].Icon_ = "OldIcon";
  assert.match(msiIconProblems(buildMsi(second), ico).problems[0], /^shortcut ApplicationDesktopShortcut: icon OldIcon .* differs/);
});

test("a malformed MSI fails clearly", () => {
  assert.throws(() => msiIconProblems(Buffer.alloc(4096, 1), ico), /not an MSI \(no compound file signature\)/);
  const msi = buildMsi(tauriTables());
  assert.throws(() => msiIconProblems(msi.subarray(0, 1024), ico), (e) => e instanceof IconCheckError && /outside the file|broken sector chain/.test(e.message));
  assert.throws(() => msiIconProblems(buildCfb(new Map([["a", Buffer.from("x")]])), ico), /not an MSI database \(no string pool\)/);
  const loop = Buffer.from(msi);
  // Point the first FAT entry after the FAT sectors (the directory) at itself.
  const fatSector = loop.readUInt32LE(0x4c);
  const dir = loop.readUInt32LE(0x30);
  loop.writeUInt32LE(dir, 512 * (fatSector + 1) + 4 * dir);
  assert.throws(() => msiIconProblems(loop, ico), /directory: broken sector chain/);
});

// ------------------------------------------------------------ LZMA

test("LZMA streams with matches, repeats and long distances decode (liblzma vector)", () => {
  // Python: lzma.compress(sample(), format=FORMAT_ALONE, filters=[{id: FILTER_LZMA1,
  // dict_size: 1 << 16, lc: 3, lp: 0, pb: 2}]) without its 8-byte size field.
  const vector = Buffer.from(
    "XQAAAQAAGA9JBoIC3PT/rkODp3cS52Wn48wzcA9IQdhN/jm7+GgsUOQctZkuS465VJES8szxrTeQWlpqgTQVyQr9ZGoWXdfIDPtlLBjfqXt5QiTOPC0tgrYYps5CcViKy5CrAEVX5gNqctgNl+rfy4afqX+5SKLX7S8fg6aJzlxaNHfG836WWXY7+a6ZrfXnXoV2PWIl0ohzjIB78rtJNC/GpfZBIXEy7Dp9Vp+WWgtAg4M4h+1g3duqzih+EETekKrVdqYJUnDz6zU+wf/xQANE",
    "base64",
  );
  const vocab = ["cube", "tile", "blue", "teal", "object", "storage", "bucket", "icon", "sotyware", "panel"];
  const words = [];
  for (let i = 0; i < 1200; i++) {
    words.push(vocab[(i * i * 7 + i * 3) % 10]);
    if (i % 97 === 0) words.push("x".repeat((i % 40) + 1));
  }
  let block = "";
  for (let j = 0; j < 300; j++) block += String.fromCharCode(48 + ((j * j * 13) % 75));
  const expected = Buffer.from(block + words.join(" ") + block, "latin1");
  const r = lzmaDecompress(vector);
  assert.equal(r.endMarker, true);
  assert.equal(r.inputUsed, vector.length);
  assert.ok(r.data.equals(expected));
});

test("the test's literal encoder round-trips and a damaged stream fails", () => {
  const data = Buffer.from(Array.from({ length: 3000 }, (_, i) => (i * 31 + (i >> 3)) & 0xff));
  const r = lzmaDecompress(lzmaLiterals(data));
  assert.equal(r.endMarker, true);
  assert.ok(r.data.equals(data));
  const noEnd = lzmaDecompress(lzmaLiterals(data, { endMarker: false }));
  assert.equal(noEnd.endMarker, false);
  const bad = lzmaLiterals(data);
  bad[5] = 1;
  assert.throws(() => lzmaDecompress(bad), /first range coder byte is not 0/);
  assert.throws(() => lzmaDecompress(Buffer.from([0xff, 0, 0, 1, 0, 0])), /LZMA properties are malformed/);
});

// ------------------------------------------------------------ NSIS

test("an NSIS setup program and uninstaller with icon.ico pass", () => {
  const exe = goodNsis();
  assert.deepEqual(peIconProblems(exe, ico), []);
  assert.deepEqual(nsisUninstallerProblems(exe, ico), []);
  const { items } = nsisParts(exe);
  assert.equal(items.length, 3);
});

test("an NSIS setup program without its own icon fails", () => {
  const stub = buildPe([{ type: 24, id: 1, lang: 1033, data: Buffer.from("<assembly/>") }]);
  assert.deepEqual(peIconProblems(buildNsis(stub, [Buffer.from("a file")]), ico), ["no icon group (RT_GROUP_ICON)"]);
});

test("an NSIS installer without uninstaller icon data fails", () => {
  const exe = buildNsis(setupStub(), [Buffer.from("a file"), Buffer.alloc(5000, 0x42)]);
  assert.throws(() => nsisUninstallerProblems(exe, ico), /holds no uninstaller icon data/);
});

test("a stale uninstaller icon fails even when the setup program's is right", () => {
  const stub = setupStub();
  const exe = buildNsis(stub, [iconPatchItem(stub, iconResources(staleIco()))]);
  assert.deepEqual(peIconProblems(exe, ico), []);
  assert.deepEqual(nsisUninstallerProblems(exe, ico), ["entry 5 (256 px, RT_ICON 6): image differs from icon.ico entry 5 (256 px)"]);
});

test("an uninstaller icon that leaves part of the setup program's icon in place fails", () => {
  const stub = setupStub();
  const exe = buildNsis(stub, [iconPatchItem(stub, iconResources(ico), { skip: ["3/6"] })]);
  assert.deepEqual(nsisUninstallerProblems(exe, ico), ["RT_ICON 6 is not written by the uninstaller icon data (it is the setup program's)"]);
});

test("a setup program with a stale icon fails", () => {
  const stub = setupStub(staleIco());
  const exe = buildNsis(stub, [iconPatchItem(stub, iconResources(ico))]);
  assert.equal(peIconProblems(exe, ico).length, 1);
  assert.deepEqual(nsisUninstallerProblems(exe, ico), []);
});

test("a malformed NSIS installer fails clearly", () => {
  const exe = goodNsis();
  assert.throws(() => nsisUninstallerProblems(buildPe(iconResources(ico)), ico), /not an NSIS installer \(no NullsoftInst header\)/);
  assert.throws(() => nsisUninstallerProblems(exe.subarray(0, exe.length - 100), ico), /NSIS data lies outside the file/);
  const stub = setupStub();
  const noEnd = buildNsis(stub, [iconPatchItem(stub, iconResources(ico))], { endMarker: false });
  assert.throws(() => nsisUninstallerProblems(noEnd, ico), /NSIS data is truncated \(no LZMA end marker\)/);
  const uninstaller = buildNsis(stub, [], { flags: 1 });
  assert.throws(() => nsisUninstallerProblems(uninstaller, ico), /is an uninstaller, not a setup program/);
  const zlib = Buffer.from(exe);
  const fh = exe.indexOf("NullsoftInst") - 8;
  zlib[fh + 28] = 0x78;
  assert.throws(() => nsisUninstallerProblems(zlib, ico), /not solid LZMA/);
  const damaged = Buffer.from(exe);
  for (let i = fh + 600; i < fh + 700; i++) damaged[i] ^= 0x5a;
  assert.throws(() => nsisUninstallerProblems(damaged, ico), (e) => e instanceof IconCheckError);
});

// ------------------------------------------------------------ the command

/** The fixtures a release build would produce, written to `dir`. */
function writeFixtures(dir, { msiIcon = ico, exeIcon = ico, nsis = goodNsis() } = {}) {
  const files = { msi: join(dir, "app.msi"), nsis: join(dir, "app-setup.exe"), exe: join(dir, "sotyware.exe"), image: join(dir, "image") };
  writeFileSync(files.msi, buildMsi(tauriTables(msiIcon)));
  writeFileSync(files.nsis, nsis);
  writeFileSync(files.exe, buildPe(iconResources(exeIcon)));
  mkdirSync(join(files.image, "PFiles", "sotyware"), { recursive: true });
  writeFileSync(join(files.image, "app.msi"), buildMsi(tauriTables(msiIcon)));
  writeFileSync(join(files.image, "PFiles", "sotyware", "sotyware.exe"), buildPe(iconResources(exeIcon)));
  return files;
}

const fingerprint = (paths) =>
  paths.map((p) => {
    const s = statSync(p);
    return [p, createHash("sha256").update(readFileSync(p)).digest("hex"), s.size, s.mtimeMs];
  });

test("the checks pass on good installers and change none of the files", () => {
  withTemp((dir) => {
    const f = writeFixtures(dir);
    const all = [f.msi, f.nsis, f.exe, join(f.image, "app.msi"), join(f.image, "PFiles", "sotyware", "sotyware.exe")];
    const before = fingerprint(all);
    const r = runChecks({ ico: icoPath, msi: [f.msi], msiImage: [f.image], nsis: [f.nsis], exe: [f.exe] });
    assert.equal(r.ok, true, r.lines.join("\n"));
    assert.equal(r.lines.filter((l) => l.startsWith("ok ")).length, 7, r.lines.join("\n"));
    assert.deepEqual(fingerprint(all), before);
  });
});

test("the checks fail on a stale icon in any installer", () => {
  withTemp((dir) => {
    const f = writeFixtures(dir, { msiIcon: staleIco(), exeIcon: staleIco() });
    const r = runChecks({ ico: icoPath, msi: [f.msi], msiImage: [f.image], nsis: [], exe: [f.exe] });
    assert.equal(r.ok, false);
    const fails = r.lines.filter((l) => l.startsWith("FAIL"));
    assert.ok(fails.some((l) => l.includes(`MSI ${f.msi}`)), fails.join("\n"));
    assert.ok(fails.some((l) => l.includes(`executable ${f.exe}`)), fails.join("\n"));
    assert.ok(fails.some((l) => l.includes("administrative image's sotyware.exe")), fails.join("\n"));
  });
});

test("an administrative image without exactly one .msi and sotyware.exe fails", () => {
  withTemp((dir) => {
    const f = writeFixtures(dir);
    writeFileSync(join(f.image, "second.msi"), readFileSync(f.msi));
    rmSync(join(f.image, "PFiles"), { recursive: true });
    const r = runChecks({ ico: icoPath, msi: [], msiImage: [f.image], nsis: [], exe: [] });
    assert.equal(r.ok, false);
    assert.ok(r.lines.some((l) => l.endsWith("expected one .msi, found 2")), r.lines.join("\n"));
    assert.ok(r.lines.some((l) => l.endsWith("expected one sotyware.exe, found 0")), r.lines.join("\n"));
  });
});

test("an unreadable or missing file fails clearly, and nothing to check is a failure", () => {
  const r = runChecks({ ico: icoPath, msi: [join(tmpdir(), "sotyware-no-such.msi")], msiImage: [], nsis: [], exe: [] });
  assert.equal(r.ok, false);
  assert.match(r.lines.find((l) => l.startsWith("FAIL")), /ENOENT/);
  const none = runChecks({ ico: icoPath, msi: [], msiImage: [], nsis: [], exe: [] });
  assert.equal(none.ok, false);
  assert.throws(() => parseArgs(["--msi"]), /--msi needs a path/);
  assert.throws(() => parseArgs(["--zip", "x"]), /unknown argument --zip/);
});

test("the command line exits 0, 1 or 2", () => {
  withTemp((dir) => {
    mkdirSync(join(dir, "good"));
    mkdirSync(join(dir, "bad"));
    const good = writeFixtures(join(dir, "good"));
    const stub = setupStub();
    const bad = writeFixtures(join(dir, "bad"), { nsis: buildNsis(stub, [iconPatchItem(stub, iconResources(staleIco()))]) });
    const run = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
    const ok = run("--msi", good.msi, "--msi-image", good.image, "--nsis", good.nsis, "--exe", good.exe);
    assert.equal(ok.status, 0, ok.stdout + ok.stderr);
    assert.match(ok.stdout, /installer-icons: ok$/m);
    const fail = run("--nsis", bad.nsis);
    assert.equal(fail.status, 1, fail.stdout + fail.stderr);
    assert.match(fail.stderr, /FAIL NSIS uninstaller .*image differs/);
    assert.equal(run("--bogus").status, 2);
  });
});

test("the check reads files only: it imports no process, network or write API", () => {
  const source = readFileSync(script, "utf8");
  const imports = [...source.matchAll(/^import (.+) from "([^"]+)";$/gm)].map((m) => `${m[1]} from ${m[2]}`);
  assert.deepEqual(imports, [
    "{ readFileSync, readdirSync, statSync } from node:fs",
    "{ dirname, join, resolve } from node:path",
    "{ fileURLToPath } from node:url",
    "{ readIco } from ./icons.mjs",
  ]);
  assert.doesNotMatch(source, /\bimport\(|require\(|child_process|spawn|exec[A-Z(]|writeFile|appendFile|unlink|rmSync|renameSync|openSync/);
});
