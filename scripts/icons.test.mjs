// Tests for scripts/icons.mjs: `npm run test:icons`.

import assert from "node:assert/strict";
import { copyFileSync, cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { buildIco, ICO_SIZES, iconProblems, pngInfo, readIco } from "./icons.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const icons = join(repo, "src-tauri", "icons");
const bundle = JSON.parse(readFileSync(join(repo, "src-tauri", "tauri.conf.json"), "utf8")).bundle;

/** A copy of src-tauri/icons in a temporary src-tauri, removed afterwards. */
function withCopy(fn) {
  const tmp = mkdtempSync(join(tmpdir(), "sotyware-icons-test-"));
  try {
    const dir = join(tmp, "src-tauri", "icons");
    cpSync(icons, dir, { recursive: true });
    fn(dir);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

test("the committed icons pass", () => {
  assert.deepEqual(iconProblems(icons, bundle), []);
});

test("an ICO written by buildIco reads back with the same PNGs", () => {
  const ico = readFileSync(join(icons, "icon.ico"));
  const pngs = readIco(ico).map((e) => e.png);
  const rebuilt = buildIco(pngs);
  assert.ok(rebuilt.equals(ico));
  assert.deepEqual(readIco(rebuilt).map((e) => e.size), ICO_SIZES);
  assert.equal(rebuilt[6 + 16 * 5], 0, "256 px is stored as 0");
});

test("buildIco refuses a non-square or too large image", () => {
  const png = Buffer.from(readFileSync(join(icons, "32x32.png")));
  png.writeUInt32BE(31, 20);
  assert.throws(() => buildIco([png]), /32x31/);
  assert.throws(() => buildIco([readFileSync(join(icons, "icon.png"))]), /512x512/);
});

test("pngInfo and readIco refuse other files", () => {
  assert.throws(() => pngInfo(Buffer.from("not a png at all, really not")), /not a PNG/);
  assert.throws(() => readIco(readFileSync(join(icons, "32x32.png"))), /not an ICO/);
});

test("a missing configured icon is reported", () => {
  withCopy((dir) => {
    rmSync(join(dir, "icon.ico"));
    const problems = iconProblems(dir, bundle);
    assert.ok(problems.includes("icons/icon.ico is in tauri.conf.json but does not exist"), problems.join("\n"));
  });
});

test("a PNG of the wrong size is reported", () => {
  withCopy((dir) => {
    copyFileSync(join(dir, "128x128.png"), join(dir, "32x32.png"));
    const problems = iconProblems(dir, bundle);
    assert.ok(problems.includes("32x32.png is 128x128, not 32x32"), problems.join("\n"));
    assert.ok(problems.includes("icon.ico's 32 px entry differs from 32x32.png"), problems.join("\n"));
  });
});

test("an ICO without every size is reported", () => {
  withCopy((dir) => {
    const pngs = readIco(readFileSync(join(dir, "icon.ico"))).map((e) => e.png).slice(1);
    writeFileSync(join(dir, "icon.ico"), buildIco(pngs));
    const problems = iconProblems(dir, bundle);
    assert.ok(problems.some((p) => p.startsWith("icon.ico holds 24, 32")), problems.join("\n"));
  });
});

test("a leftover file, a missing source and a config without 32x32.png first are reported", () => {
  withCopy((dir) => {
    writeFileSync(join(dir, "StoreLogo.png"), readFileSync(join(dir, "32x32.png")));
    rmSync(join(dir, "source", "sotyware-16.svg"));
    const problems = iconProblems(dir, { ...bundle, icon: ["icons/128x128.png", "icons/32x32.png", "icons/icon.ico"] });
    assert.ok(problems.includes("unexpected file icons/StoreLogo.png"), problems.join("\n"));
    assert.ok(problems.includes("source/sotyware-16.svg is missing"), problems.join("\n"));
    assert.ok(problems.some((p) => p.startsWith("the first PNG in tauri.conf.json is icons/128x128.png")), problems.join("\n"));
  });
});

test("NSIS installer and uninstaller icons must be icon.ico", () => {
  const { nsis } = bundle.windows;
  const problems = iconProblems(icons, { ...bundle, windows: { nsis: { ...nsis, uninstallerIcon: undefined } } });
  assert.deepEqual(problems, ["bundle.windows.nsis.uninstallerIcon is not set, not icons/icon.ico"]);
});
