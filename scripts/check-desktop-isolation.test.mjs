// Self-test of scripts/check-desktop-isolation.mjs (ADR-027): the real
// drivers pass; drivers written the way they were before 2026-11-14 (moving
// and deleting the app's own databases, deleting credentials directly,
// launching without a profile check) are refused.
//
// Run: npm run test:desktop-isolation-guard (node --test).

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

import { scan } from "./check-desktop-isolation.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const tmp = mkdtempSync(join(tmpdir(), "sotyware-isolation-guard-"));
after(() => rmSync(tmp, { recursive: true, force: true }));

function makeDir(d) {
  rmSync(d, { recursive: true, force: true });
  mkdirSync(d, { recursive: true });
}

const GOOD_PS = [
  'Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force',
  "if (-not (Test-TestProfileBuild $Exe)) { exit 2 }",
  "$p = Start-Process $Exe -PassThru",
  "$null = Wait-TestProfileAck $Prof $p.Id 30 $p",
].join("\n");

test("the real drivers pass", () => {
  const findings = scan(resolve(here, "..", "tests", "desktop"));
  assert.deepEqual(findings, []);
});

test("each forbidden pattern is found, comments are not", () => {
  const d = join(tmp, "bad");
  makeDir(d);
  writeFileSync(join(d, "old.ps1"), [
    GOOD_PS,
    '# %LOCALAPPDATA%\\com.sotyware.desktop is only named in this comment',
    '$CloudDb = Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "com.sotyware.desktop\\cloud-accounts.sqlite3"',
    'foreach ($f in @($CloudDb)) { Move-Item $f "x.sqlite3" }',
    'Remove-Item "$dir\\preferences.sqlite3"',
    'function Target($id) { "com.sotyware.desktop/cloud-account/$id" }',
    'cmdkey "/delete:$t"',
  ].join("\n"));
  const ids = scan(d).map((f) => `${f.line}:${f.id}`);
  assert.deepEqual(ids, [
    "6:normal-profile-path",
    "6:normal-identifier-path",
    "7:database-file-operation",
    "8:database-file-operation",
    "9:normal-identifier-path",
    "10:credential-delete",
  ]);
});

test("a driver that launches without the profile checks is refused", () => {
  const d = join(tmp, "launch");
  makeDir(d);
  writeFileSync(join(d, "a.ps1"), "$p = Start-Process $Exe -PassThru\n");
  writeFileSync(join(d, "b.py"), "p = subprocess.Popen([BIN])\n");
  writeFileSync(join(d, "c.ps1"), GOOD_PS + "\n");
  const files = scan(d).map((f) => `${f.file}:${f.id}`);
  assert.equal(files.filter((f) => f === "a.ps1:launch-without-profile-check").length, 3);
  assert.equal(files.filter((f) => f === "b.py:launch-without-profile-check").length, 3);
  assert.ok(!files.some((f) => f.startsWith("c.ps1")));
});

test("the profile helpers themselves are exempt", () => {
  const d = join(tmp, "helpers");
  makeDir(d);
  writeFileSync(join(d, "TestProfile.psm1"), '(Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "x")\n');
  writeFileSync(join(d, "test_profile.py"), 'NORMAL = "com.sotyware.desktop/"\n');
  assert.deepEqual(scan(d), []);
});
