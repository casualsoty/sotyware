#!/usr/bin/env node
// Static guard for the desktop test drivers (ADR-027): no driver may name,
// move, write or delete the app's own profile or credentials, and every
// driver that launches sotyware must use a test-profile build and check its
// acknowledgement.
//
// Rules, for every driver file in tests/desktop/ (except the profile
// helpers, TestProfile.psm1 and test_profile.py and their self-tests, which
// hold the only references to the normal profile, for comparison):
//
//   - no %LOCALAPPDATA% / %APPDATA% / LocalApplicationData / ApplicationData
//     path, no "com.sotyware.desktop" followed by a path separator, and no
//     bare "com.sotyware.desktop" string literal (the app's own data folder
//     or credential namespace, e.g. os.path.join(data, "com.sotyware.desktop")),
//     outside comments;
//   - no Move-Item, Rename-Item, Remove-Item, Copy-Item, os.remove, rename
//     or shutil call on a line that names a .sqlite3 file;
//   - no "cmdkey /delete", "secret-tool clear" or keyring.delete_password
//     (credentials are deleted only through Remove-TestCredential, which
//     refuses anything outside the profile, or not at all);
//   - a driver that starts the app (Start-Process [-FilePath] $Exe, & $Exe,
//     [Diagnostics.Process]::Start($Exe); subprocess.Popen/run/call([BIN ...]))
//     calls, in code (not only in a comment), the profile helper's module
//     import, its build check and its acknowledgement wait. The only
//     exemption is a phase file that another driver runs with exec() in its
//     own namespace (EXECD below), and only if that driver passes the checks;
//   - a shell script never starts the app itself ("$BIN" in command
//     position); it hands the binary to a Python driver.
//
// What it cannot see: a path built across several lines or variables (a
// .sqlite3 name in one line and Remove-Item $f in another). The runtime
// test profile (exit 3 without one, its own identifier, the
// acknowledgement) is the protection; this guard keeps drivers from
// reintroducing the patterns of the 2026-11-14 incident.
//
// Exit 0: fine; 1: a rule was broken (each finding is printed). Usage:
//   node scripts/check-desktop-isolation.mjs [dir]   (default: tests/desktop)

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const HELPERS = new Set(["TestProfile.psm1", "test_profile.py", "test_profile_test.py", "test-profile-selftest.ps1"]);
/** Phase files run with exec() inside cloud_ui.py, which does the checks. */
const EXECD = new Set(["cloud_ui_upload.py", "cloud_ui_files.py"]);

/** The code part of a line: no comment-only line (good enough for the rules). */
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
    test: (l) => /LOCALAPPDATA|\bAPPDATA\b|ApplicationData/i.test(l),
    why: "names the user's AppData folder; use the test profile's paths (TestProfile.psm1)",
  },
  {
    id: "normal-identifier-path",
    test: (l) => /com\.sotyware\.desktop[\\/]|["']com\.sotyware\.desktop["']/.test(l),
    why: "names the app's own data folder or credential namespace (com.sotyware.desktop/ or \\, or the bare identifier as a string)",
  },
  {
    id: "database-file-operation",
    test: (l) =>
      /\.sqlite3/.test(l) &&
      /\b(Move-Item|Rename-Item|Remove-Item|Copy-Item|os\.remove|os\.unlink|os\.rename|os\.replace|shutil\.|rm -|mv )\b/.test(l),
    why: "moves, copies or deletes a database file; a test never does that (ADR-027)",
  },
  {
    id: "credential-delete",
    test: (l) => /cmdkey\s+["']?\/delete|secret-tool\s+(clear|delete)\b|delete_password\s*\(/i.test(l),
    why: "deletes a credential directly; use Remove-TestCredential, which refuses entries outside the profile",
  },
];

const LAUNCH = {
  ps: /Start-Process\s+(-FilePath\s+)?\$Exe\b|&\s*\$Exe\b|Process\]::Start\(\s*\$Exe\b/i,
  py: /\b(Popen|run|call|check_call|check_output)\(\s*\[\s*(str\(\s*)?BIN\b/,
};
const NEEDS = {
  ps: [/Import-Module\s.*TestProfile\.psm1/, /Test-TestProfileBuild\s+\$/, /Wait-TestProfileAck\s+\$/],
  py: [/^\s*import test_profile\b|^\s*import test_profile as/m, /require_test_profile_build\(/, /verify_ack\(/],
};
// "$BIN" (or $BIN, ${BIN}) as the command of a shell line.
const SHELL_LAUNCH = /(^\s*|[;&|(]\s*|\b(exec|nohup|setsid)\s+)(\w+=\S*\s+)*"?\$\{?BIN\}?"?(\s|$|&)/;

/** @returns {{file: string, line: number, id: string, why: string, text: string}[]} */
export function scan(dir) {
  const findings = [];
  /** name -> code text, of the files that pass the launch checks */
  const checked = new Map();
  const execd = [];
  for (const name of readdirSync(dir).sort()) {
    const m = name.match(/\.(ps1|psm1|py|sh)$/);
    if (!m || HELPERS.has(name)) continue;
    const ext = m[1];
    const text = readFileSync(join(dir, name), "utf8");
    const lines = text.split(/\r?\n/);
    const codeLines = lines.map((raw) => code(raw, ext));
    codeLines.forEach((l, i) => {
      if (!l) return;
      for (const r of RULES) {
        if (r.test(l)) findings.push({ file: name, line: i + 1, id: r.id, why: r.why, text: lines[i].trim() });
      }
      if (ext === "sh" && SHELL_LAUNCH.test(l)) {
        findings.push({
          file: name, line: i + 1, id: "launch-from-shell",
          why: "a shell script starts the app; hand the binary to a Python driver that checks the profile", text: lines[i].trim(),
        });
      }
    });
    const body = codeLines.join("\n");
    const kind = ext === "py" ? "py" : ext === "ps1" ? "ps" : null;
    if (!kind || !LAUNCH[kind].test(body)) continue;
    if (kind === "py" && EXECD.has(name)) {
      execd.push(name);
      continue;
    }
    const missing = NEEDS[kind].filter((re) => !re.test(body));
    for (const re of missing) {
      findings.push({ file: name, line: 0, id: "launch-without-profile-check", why: `starts the app but its code has no ${re}`, text: "" });
    }
    if (missing.length === 0) checked.set(name, body);
  }
  for (const name of execd) {
    const runner = [...checked].find(([, body]) => new RegExp(`exec\\(.*["']${name.replace(".", "\\.")}["']`).test(body));
    if (!runner) {
      findings.push({
        file: name, line: 0, id: "launch-without-profile-check",
        why: "starts the app, and no driver that checks the profile runs it with exec()", text: "",
      });
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
