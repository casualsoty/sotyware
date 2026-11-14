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

// Audit 2026-11-14 (later session): bypasses of the first version.

test("exec() anywhere no longer stands in for the profile checks", () => {
  const d = join(tmp, "exec");
  makeDir(d);
  // How cloud_ui.py looked before ADR-027: it launched the app without any
  // profile check and ran a phase file with exec(); the first guard passed it.
  writeFileSync(join(d, "cloud_ui.py"), [
    'DB = os.path.join(data, "com.sotyware.desktop", "cloud-accounts.sqlite3")',
    "p = subprocess.Popen([BIN], env=env)",
    'exec(open("cloud_ui_upload.py").read())',
  ].join("\n"));
  writeFileSync(join(d, "cloud_ui_upload.py"), "second = subprocess.run([BIN], env=env)\n");
  const ids = scan(d).map((f) => `${f.file}:${f.line}:${f.id}`);
  assert.ok(ids.includes("cloud_ui.py:1:normal-identifier-path"), ids.join(" "));
  assert.equal(ids.filter((f) => f === "cloud_ui.py:0:launch-without-profile-check").length, 3, ids.join(" "));
  // The phase file is only exempt when a checking driver execs it.
  assert.ok(ids.includes("cloud_ui_upload.py:0:launch-without-profile-check"), ids.join(" "));
});

test("an exec'd phase file is accepted when a checking driver runs it", () => {
  const d = join(tmp, "exec-ok");
  makeDir(d);
  writeFileSync(join(d, "cloud_ui.py"), [
    "import test_profile as tp",
    "tp.require_test_profile_build(BIN)",
    "p = subprocess.Popen([BIN], env=env)",
    "tp.verify_ack(PROFILE, p.pid)",
    'exec(open(os.path.join(here, "cloud_ui_upload.py")).read())',
  ].join("\n"));
  writeFileSync(join(d, "cloud_ui_upload.py"), "second = subprocess.run([BIN], env=env)\n");
  assert.deepEqual(scan(d), []);
});

test("checks named only in comments do not count", () => {
  const d = join(tmp, "comments");
  makeDir(d);
  writeFileSync(join(d, "a.ps1"), [
    "# Import-Module TestProfile.psm1; Test-TestProfileBuild $Exe; Wait-TestProfileAck $Prof",
    "$p = Start-Process $Exe -PassThru",
  ].join("\n"));
  writeFileSync(join(d, "b.py"), [
    "# import test_profile; require_test_profile_build(BIN); verify_ack(p)",
    "p = subprocess.Popen([BIN])",
  ].join("\n"));
  const ids = scan(d).map((f) => `${f.file}:${f.id}`);
  assert.equal(ids.filter((f) => f === "a.ps1:launch-without-profile-check").length, 3, ids.join(" "));
  assert.equal(ids.filter((f) => f === "b.py:launch-without-profile-check").length, 3, ids.join(" "));
});

test("other ways to launch the app are recognised", () => {
  const d = join(tmp, "launch2");
  makeDir(d);
  writeFileSync(join(d, "a.ps1"), "$p = Start-Process -FilePath $Exe -PassThru\n");
  writeFileSync(join(d, "b.ps1"), "& $Exe\n");
  writeFileSync(join(d, "c.ps1"), "[System.Diagnostics.Process]::Start($Exe)\n");
  writeFileSync(join(d, "d.py"), "p = subprocess.Popen([ str(BIN) ], env=env)\n");
  writeFileSync(join(d, "e.py"), "subprocess.check_call([BIN, '--x'])\n");
  writeFileSync(join(d, "f.sh"), 'XDG_DATA_HOME=/tmp/x "$BIN" &\nexec "$BIN"\n[ -x "$BIN" ] || exit 2\npython3 d.py "$BIN"\n');
  const ids = scan(d).map((f) => `${f.file}:${f.line}:${f.id}`);
  for (const f of ["a.ps1", "b.ps1", "c.ps1", "d.py", "e.py"]) {
    assert.ok(ids.includes(`${f}:0:launch-without-profile-check`), `${f}: ${ids.join(" ")}`);
  }
  assert.ok(ids.includes("f.sh:1:launch-from-shell"), ids.join(" "));
  assert.ok(ids.includes("f.sh:2:launch-from-shell"), ids.join(" "));
  assert.ok(!ids.some((f) => f.startsWith("f.sh:3:") || f.startsWith("f.sh:4:")), ids.join(" "));
});

test("other spellings of the app's folders and credential deletes are refused", () => {
  const d = join(tmp, "spellings");
  makeDir(d);
  writeFileSync(join(d, "a.ps1"), [
    "$d = Join-Path $env:APPDATA 'x'",
    "$e = [Environment]::GetFolderPath('LocalApplicationData')",
    "$f = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)",
  ].join("\n"));
  writeFileSync(join(d, "b.py"), [
    'DIR = os.path.join(os.environ["HOME"], ".local", "share", "com.sotyware.desktop")',
    "os.unlink(DIR + '/cloud-accounts.sqlite3')",
    "subprocess.run(['secret-tool', 'clear', 'service', 'x'])",
    'keyring.delete_password("svc", "user")',
    'print("the service com.sotyware.desktop.test is fine")',
  ].join("\n"));
  const ids = scan(d).map((f) => `${f.file}:${f.line}:${f.id}`);
  for (const id of [
    "a.ps1:1:normal-profile-path",
    "a.ps1:2:normal-profile-path",
    "a.ps1:3:normal-profile-path",
    "b.py:1:normal-identifier-path",
    "b.py:2:database-file-operation",
    "b.py:4:credential-delete",
  ]) {
    assert.ok(ids.includes(id), `${id}: ${ids.join(" ")}`);
  }
  // `secret-tool clear` inside a Python list is split over quotes; the shell form is caught:
  writeFileSync(join(d, "c.sh"), "secret-tool clear service com.sotyware.desktop\n");
  assert.ok(scan(d).some((f) => f.file === "c.sh" && f.id === "credential-delete"));
  assert.ok(!ids.some((f) => f.startsWith("b.py:5:")), ids.join(" "));
});
