#!/usr/bin/env node
// Static guard for the desktop test drivers (ADR-027): no driver may name,
// move, write or delete the app's own profile or credentials, and every
// driver that launches sotyware must use a test-profile build and check its
// acknowledgement.
//
// Rules, for every driver file in tests/desktop/ (except the two profile
// helpers, TestProfile.psm1 and test_profile.py, which hold the only
// references to the normal profile, for comparison):
//
//   - no %LOCALAPPDATA% / LocalApplicationData / ApplicationData path, and
//     no "com.sotyware.desktop" followed by a path separator (the app's own
//     data folder or credential namespace), outside comments;
//   - no Move-Item, Rename-Item, Remove-Item, Copy-Item, os.remove, rename
//     or shutil call on a line that names a .sqlite3 file;
//   - no "cmdkey /delete" (credentials are deleted only through
//     Remove-TestCredential, which refuses anything outside the profile);
//   - a driver that starts the app (Start-Process $Exe, Popen([BIN]) loads
//     the profile helper and calls its build check and acknowledgement wait.
//
// Exit 0: fine; 1: a rule was broken (each finding is printed). Usage:
//   node scripts/check-desktop-isolation.mjs [dir]   (default: tests/desktop)

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const HELPERS = new Set(["TestProfile.psm1", "test_profile.py", "test_profile_test.py", "test-profile-selftest.ps1"]);

/** The code part of a line: no trailing comment (good enough for the rules). */
function code(line, ext) {
  const t = line.trimStart();
  if (ext === "ps1" || ext === "psm1" || ext === "py" || ext === "sh") {
    if (t.startsWith("#")) return "";
  }
  return line;
}

const RULES = [
  {
    id: "normal-profile-path",
    test: (l) => /LOCALAPPDATA|LocalApplicationData|GetFolderPath\("ApplicationData"\)/i.test(l),
    why: "names the user's AppData folder; use the test profile's paths (TestProfile.psm1)",
  },
  {
    id: "normal-identifier-path",
    test: (l) => /com\.sotyware\.desktop[\\/]/.test(l),
    why: "names the app's own data folder or credential namespace (com.sotyware.desktop/ or \\)",
  },
  {
    id: "database-file-operation",
    test: (l) =>
      /\.sqlite3/.test(l) &&
      /\b(Move-Item|Rename-Item|Remove-Item|Copy-Item|os\.remove|os\.rename|os\.replace|shutil\.|rm -|mv )\b/.test(l),
    why: "moves, copies or deletes a database file; a test never does that (ADR-027)",
  },
  {
    id: "credential-delete",
    test: (l) => /cmdkey\s+["']?\/delete/i.test(l),
    why: "deletes a credential directly; use Remove-TestCredential, which refuses entries outside the profile",
  },
];

/** @returns {{file: string, line: number, id: string, why: string, text: string}[]} */
export function scan(dir) {
  const findings = [];
  for (const name of readdirSync(dir).sort()) {
    const m = name.match(/\.(ps1|psm1|py|sh)$/);
    if (!m || HELPERS.has(name)) continue;
    const ext = m[1];
    const text = readFileSync(join(dir, name), "utf8");
    const lines = text.split(/\r?\n/);
    lines.forEach((raw, i) => {
      const l = code(raw, ext);
      if (!l) return;
      for (const r of RULES) {
        if (r.test(l)) findings.push({ file: name, line: i + 1, id: r.id, why: r.why, text: raw.trim() });
      }
    });
    const launches = /Start-Process \$Exe\b|Popen\(\[BIN\]|subprocess\.run\(\[BIN\]/.test(text);
    if (launches) {
      const ps = ext === "ps1";
      const needs = ps
        ? [/Import-Module .*TestProfile\.psm1/, /Test-TestProfileBuild/, /Wait-TestProfileAck|Verified\b/]
        : [/import test_profile|exec\(/, /require_test_profile_build|exec\(/, /verify_ack|verified\(|exec\(/];
      // cloud_ui_upload.py runs inside cloud_ui.py (exec), which does the checks.
      const execd = name === "cloud_ui_upload.py";
      if (!execd) {
        for (const re of needs) {
          if (!re.test(text)) {
            findings.push({ file: name, line: 0, id: "launch-without-profile-check", why: `starts the app but has no ${re}`, text: "" });
          }
        }
      }
    }
  }
  return findings;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const dir = process.argv[2] ?? resolve(here, "..", "tests", "desktop");
  const findings = scan(dir);
  for (const f of findings) {
    console.error(`check-desktop-isolation: ${f.file}:${f.line}: ${f.id}: ${f.why}${f.text ? `\n    ${f.text}` : ""}`);
  }
  if (findings.length > 0) {
    console.error(`check-desktop-isolation: REFUSED: ${findings.length} finding(s) (ADR-027)`);
    process.exit(1);
  }
  console.log("check-desktop-isolation: ok: no driver names the app's own profile, and every launching driver checks its test profile");
}
