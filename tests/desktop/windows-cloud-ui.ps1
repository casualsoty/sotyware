# Cloud Storage end-to-end test on native Windows (opt-in; not part of CI):
# drives the Cloud Storage page of the release binary through UI Automation
# (Invoke, Value and Toggle patterns; no mouse, no keystrokes) and checks the
# entries in the real Windows Credential Manager. Details:
# docs/development/testing.md#cloud-storage-ui-end-to-end-test
#
# usage (PowerShell, from the repository root on a Windows path):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 [-Mode local|b2|browse|b2-browse|download|b2-download|upload|b2-upload|b2-lifecycle] [-Exe <path>] [-ExpectSha256 <hex>]
#   b2-upload also takes -Rehearse (local server) or -ConfirmB2Upload upload-ui-validation (Backblaze)
#   b2-lifecycle takes -Scenario multipart|cancel|retry|leave|quit and -Rehearse or -ConfirmB2Lifecycle lifecycle-ui-validation
#   (or -Scenario settle, only with -Rehearse -RecordRequests)
#   download, upload, b2-upload and b2-lifecycle also take -RecordRequests: -Exe must be a
#   request-recorder build (ADR-024, src-tauri\target\recorder\release\sotyware.exe); the app
#   records its S3 requests into a kept folder under %TEMP%; every recording must be complete and
#   free of keys, secrets and the bucket name; against the local server they must also agree with
#   the requests it saw signed with the app's key (the helper's own requests use another key).
#   It ends the app with the tray Quit (MOVES THE MOUSE once): a killed app writes no end line.
#
# -Mode local (default): made-up keys and an endpoint nobody listens on;
#   contacts no provider. Writes one Credential Manager entry per account it
#   adds, under a new id per run, and removes it through the app.
# -Mode browse: the bucket browser against the in-process listing server of
#   the Rust tests (cargo test ... serve_listing_for_ui -- --ignored, in this
#   checkout's src-tauri; made-up key), which fails if anything but a listing
#   or the connection check reached it. Contacts no provider.
# -Mode download: downloads through the native Windows save dialog (the common
#   item dialog: Cancel, File name + Save, the "Confirm Save As" question for
#   an existing file; its controls are not in the UI Automation tree, so they
#   are found by control id and driven with window messages, not the mouse) from the in-process
#   object server of the Rust tests (serve_objects_for_ui; made-up key), which
#   fails if anything but listing, the connection check and GetObject reached
#   it. Checks progress, Cancel, failures with Try again, and each saved
#   file's SHA-256 against the server's. Files go to a new folder under
#   %TEMP%, removed afterwards. Contacts no provider. The dialog is a real
#   window on the desktop (it is not moved or clicked with the mouse).
# -Mode upload: upload batches through the native Windows multi-file dialog
#   (the common item dialog "Choose files to upload": a folder is opened by
#   typing its path into File name and pressing Open, files are selected in
#   its file list through UI Automation (SelectionItem) or by typing their
#   quoted names into File name, then Open) into the in-process s3s-fs server
#   of the Rust tests (serve_uploads_for_ui; made-up key; a new temporary
#   bucket per run, seeded with inbox/existing.txt, other/readme.txt and
#   downloads/big.bin), behind a link that takes 6 MiB/s. After each batch
#   the server's state is read (every object's content, size and SHA-256
#   metadata, the open multipart files); at the end exactly the known keys
#   are deleted and the bucket must be empty. Local files go to a new folder
#   under %TEMP%, removed afterwards. Contacts no provider. Unlike the other
#   modes this one needs the unlocked, interactive desktop with a taskbar
#   (exit 3 otherwise): for "tray Quit during a multipart upload" it MOVES THE
#   MOUSE and clicks (right click on the tray icon, a click on Quit, as
#   windows.ps1 does), opens the "Show Hidden Icons" flyout and may press Esc
#   once to close it. Everything else goes through UI Automation and window
#   messages. Do not use the mouse or keyboard while it runs.
# -Mode b2: the real Backblaze B2 test bucket. The key comes from
#   $env:USERPROFILE\.sotyware-it\b2.xml (Get-Credential | Export-Clixml: keyID
#   as user name, applicationKey as password, DPAPI-encrypted for this Windows
#   user) and the bucket name from b2-bucket.txt next to it; -Region defaults
#   to eu-central-003. Nothing is printed. Before and after, the opt-in Rust
#   test backblaze_b2_fingerprint (cargo, in this checkout's src-tauri) hashes
#   every object version under sotyware-it/; the run fails if they differ.
# -Mode b2-browse: writes a temporary dataset to the Backblaze test bucket
#   (opt-in Rust test backblaze_b2_browse_dataset with the b2.xml key, kept,
#   under sotyware-it/browse-validation/<run id>/ only), browses it in the
#   app with a second, read-only key without a file name prefix
#   ($env:USERPROFILE\.sotyware-it\b2-read.xml, same form), compares every row
#   (name, size, date) with the provider's listing, and always deletes exactly
#   that run's versions afterwards (backblaze_b2_browse_cleanup). Needs node
#   on PATH (the page's date format is reproduced with the same Intl call).
# -Mode b2-download: one real download from the Backblaze test bucket through
#   the dashboard and the native save dialog. The opt-in Rust test
#   backblaze_b2_download_ui_prepare (b2.xml key) checks that a new run prefix
#   sotyware-it/download-ui-validation/<run id>/ holds no version, generates
#   one 16 MiB + 3 file (SHA-256 computed first) into a new folder under
#   %TEMP% and uploads it as "gr\u00f6\u00dfe test.bin" (a space, two
#   non-ASCII letters); the app then browses to it and
#   downloads it with the read-only key (b2-read.xml) into another new folder
#   under %TEMP%, sampling the progress shown; the saved file is compared with
#   the generated one (size, SHA-256, every byte). Afterwards
#   backblaze_b2_download_ui_cleanup deletes exactly that version and lists
#   the run prefix once (none may be left), and both local folders are
#   removed. No fingerprint of the rest of the bucket is taken.
# -Mode b2-upload: one upload batch through the dashboard and the native
#   Windows multi-file dialog into the Backblaze test bucket. Refused unless
#   -ConfirmB2Upload upload-ui-validation is given. The local files are
#   written first, into a new folder under %TEMP% (seed\existing.txt;
#   batch\ with existing.txt (other content), a-small.txt,
#   "gr\u00f6\u00dfe +test.bin" (200,003 bytes) and progress.bin (24 MiB + 5,
#   one PUT)). The opt-in Rust test backblaze_b2_upload_ui_prepare (b2.xml
#   write key) checks that the run prefix
#   sotyware-it/upload-ui-validation/<run id>/ holds no version, delete marker
#   or incomplete multipart upload and uploads the seed as existing.txt (so
#   the run folder exists). The app, with a third key
#   ($env:USERPROFILE\.sotyware-it\b2-upload.xml: the test bucket only, read
#   and write, no file name prefix, so the browser can open the root),
#   browses to the run folder; the panel must name it as the destination
#   before Open is pressed in the dialog (else the dialog is cancelled and
#   nothing is sent); then the four files go up as one batch: three
#   uploaded, existing.txt skipped. Afterwards backblaze_b2_upload_ui_verify
#   downloads the four keys and compares them with the local files (the seed
#   for existing.txt), then deletes exactly this run's versions by id, aborts
#   any incomplete multipart upload of its keys by id, and lists the prefix
#   again (none may be left); backblaze_b2_upload_ui_cleanup instead if the
#   batch never started. Does not move the mouse.
#   -Rehearse runs the same dashboard steps against the in-process s3s-fs
#   server (serve_uploads_for_ui seeded with the same existing.txt under the
#   same kind of key, made-up key; contacts no provider) and also checks the
#   app's requests there by operation.
# -Mode b2-lifecycle -Scenario <scenario>: one upload lifecycle behaviour
#   through the dashboard (release binary, release settings: multipart above
#   64 MiB in 16 MiB parts) and the native multi-file dialog into the
#   Backblaze test bucket, one scenario per run, each under its own prefix
#   sotyware-it/upload-lifecycle-validation/<scenario>-<run id>/. Refused
#   unless -ConfirmB2Lifecycle lifecycle-ui-validation is given. Keys as
#   for b2-upload (b2.xml for the Rust tests, b2-upload.xml for the app).
#   The opt-in Rust test backblaze_b2_upload_lifecycle_prepare checks the
#   local files (each must be the single PutObject or the multipart upload
#   the scenario needs with the release settings) and that the run prefix
#   is empty, and uploads seed\anchor.txt (so the browser can open the run
#   folder). Scenarios:
#     multipart: multipart.bin (72 MiB + 3, 5 parts) uploads and completes;
#       while it is sent, backblaze_b2_upload_lifecycle_probe must list one
#       open multipart upload for it.
#     cancel: cancel.bin (144 MiB, 9 parts), Cancel upload at 20% or more.
#     retry: anchor.txt (other content: skipped), a-first.txt,
#       b-locked.txt (held open without sharing by this script: fails),
#       c-third.txt; then Try 1 file again once it is closed.
#     leave: a-away.bin (60 MiB, one PUT) and b-away.txt; Back to accounts
#       at 10% or more.
#     quit: quit.bin (144 MiB, 9 parts), tray Quit at 20% or more (MOVES
#       THE MOUSE once, like -Mode upload; needs the interactive desktop).
#     settle (rehearsal only, needs -RecordRequests; never at Backblaze):
#       settle.bin (144 MiB, 9 parts) behind a link that takes every request
#       body at once and forwards it at 6 MiB/s (Pace::Buffered), so each
#       part is handed over completely and then about 2.7 s unanswered.
#       Cancel upload right after the percentage moves to a new part (20%
#       or more); the recording must show the ADR-025 wait: the part in
#       flight at the cancel answered 200 at least 500 ms after it (not
#       dropped), and the abort only after that answer.
#   Afterwards backblaze_b2_upload_lifecycle_verify downloads what the
#   scenario must have stored and compares it, deletes exactly this run's
#   versions by id, aborts any incomplete multipart upload of its keys by id
#   (and fails if there was one), and lists again (none may be left);
#   backblaze_b2_upload_lifecycle_cleanup instead if the batch never
#   started. -Rehearse runs the same steps against the in-process s3s-fs
#   server (seeded with the anchor under the same kind of key) and checks
#   the app's requests there by operation.
#
# The app runs in a test profile of this run (ADR-027, TestProfile.psm1): a
# test-profile build (or a request-recorder build, which is one too) that
# refuses to start without the profile, keeps its databases, webview data and
# Credential Manager entries (com.sotyware.desktop.test/<nonce>/...) inside it
# and acknowledges it. Every launch that becomes the primary instance is
# checked against that acknowledgement before the window is touched; the run
# stops otherwise. The user's own %LOCALAPPDATA%\com.sotyware.desktop and
# com.sotyware.desktop/ credentials are never touched and are compared before
# and after. (Until 2026-11-14 this script moved the user's account database
# aside and deleted "its" copy at the end; inside an MSIX-virtualized
# process that acted on the user's real account and deleted the real
# database: docs/status/status.md.) Secret values are only compared, never
# shown. Needs a logged-in session (the WebView2 window
# must be created); it does not move the mouse (except -Mode upload and
# -Mode b2-lifecycle -Scenario quit, above).
# Exit code: 0 all passed, 1 a check failed, 2 binary missing, already running,
# or b2 credentials missing, 3 (upload, b2-lifecycle quit) no interactive
# desktop: nothing was run.
param(
  [ValidateSet("local", "b2", "browse", "b2-browse", "download", "b2-download", "upload", "b2-upload", "b2-lifecycle", "files")][string]$Mode = "local",
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\test-profile\release\sotyware.exe"),
  [string]$ProfileParent = (Join-Path $PSScriptRoot "..\..\src-tauri\target\desktop-test-profiles"),
  [string]$Region = "eu-central-003",
  # b2-upload, b2-lifecycle: the in-process server instead of Backblaze.
  [switch]$Rehearse,
  # b2-lifecycle: the one behaviour this run checks.
  [ValidateSet("", "multipart", "cancel", "retry", "leave", "quit", "settle")][string]$Scenario = "",
  # b2-lifecycle against Backblaze: must be exactly "lifecycle-ui-validation".
  [string]$ConfirmB2Lifecycle = "",
  # b2-upload against Backblaze: must be exactly "upload-ui-validation".
  [string]$ConfirmB2Upload = "",
  # Stops before anything if the binary's SHA-256 differs.
  [string]$ExpectSha256 = "",
  # The app records its own S3 requests (a request-recorder build only).
  [switch]$RecordRequests
)
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run build:test-profile)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
if (-not (Test-TestProfileBuild $Exe)) { "ABORT: $Exe is not a test-profile build (ADR-027); run: npm run build:test-profile"; exit 2 }
if ($ExpectSha256 -and (Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash.ToLowerInvariant() -ne $ExpectSha256.ToLowerInvariant()) { "ABORT: $Exe is not the expected binary (SHA-256 differs from $ExpectSha256)"; exit 2 }
# b2-lifecycle shares b2-upload's keys, rehearsal server and cleanup rules.
$IsLc = $Mode -eq "b2-lifecycle"
$IsB2Up = $Mode -eq "b2-upload" -or $IsLc
if ($Rehearse -and -not $IsB2Up) { "ABORT: -Rehearse is only for -Mode b2-upload and b2-lifecycle"; exit 2 }
if ($Scenario -and -not $IsLc) { "ABORT: -Scenario is only for -Mode b2-lifecycle"; exit 2 }
if ($IsLc -and -not $Scenario) { "ABORT: -Mode b2-lifecycle needs -Scenario multipart, cancel, retry, leave, quit or settle (one per run)"; exit 2 }
# settle needs a link no provider has and proves its point only from the recording.
if ($Scenario -eq "settle" -and -not ($Rehearse -and $RecordRequests)) { "ABORT: -Scenario settle runs only with -Rehearse -RecordRequests"; exit 2 }
if ($IsLc -and -not $Rehearse -and $ConfirmB2Lifecycle -cne "lifecycle-ui-validation") {
  "ABORT: -Mode b2-lifecycle writes to the Backblaze test bucket; add -ConfirmB2Lifecycle lifecycle-ui-validation (or -Rehearse for the local server). Nothing was run."; exit 2
}
if ($RecordRequests) {
  if ($Mode -notin "download", "upload", "b2-upload", "b2-lifecycle") { "ABORT: -RecordRequests is only for -Mode download, upload, b2-upload and b2-lifecycle"; exit 2 }
  if (-not [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($Exe)).Contains("sotyware-request-recorder")) {
    "ABORT: -RecordRequests needs a request-recorder build (see testing.md); $Exe is a normal build"; exit 2
  }
}
if ($Mode -eq "b2-upload" -and -not $Rehearse -and $ConfirmB2Upload -cne "upload-ui-validation") {
  "ABORT: -Mode b2-upload writes to the Backblaze test bucket; add -ConfirmB2Upload upload-ui-validation (or -Rehearse for the local server). Nothing was run."; exit 2
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class CloudUiW {
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, System.Text.StringBuilder l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static string GetText(IntPtr h) { var sb = new System.Text.StringBuilder(4096); SendMessage(h, 0x000D, (IntPtr)4096, sb); return sb.ToString(); }
  public static void SetText(IntPtr h, string t) { SendMessage(h, 0x000C, IntPtr.Zero, t); }
  // BM_CLICK, posted: the dialog may open a modal question in reply.
  public static bool Click(IntPtr h) { return PostMessage(h, 0x00F5, IntPtr.Zero, IntPtr.Zero); }
  // Presses button `id` (IDYES = 6) of a task dialog (TDM_CLICK_BUTTON), or of
  // a classic message box (WM_COMMAND), whichever `h` is.
  public static void Answer(IntPtr h, int id) {
    var b = ChildById(h, id, "Button");
    if (b != IntPtr.Zero) { Click(b); return; }
    PostMessage(h, 0x0400 + 102, (IntPtr)id, IntPtr.Zero);
  }
  // The first descendant window of `parent` with control id `id` (and class
  // `cls`, if given): the common item dialog's Save (1), Cancel (2) and File
  // name edit (1001) are classic child windows that its UI Automation tree
  // does not always expose.
  public static IntPtr ChildById(IntPtr parent, int id, string cls) {
    IntPtr found = IntPtr.Zero;
    EnumChildWindows(parent, (h, l) => {
      var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
      if (GetDlgCtrlID(h) == id && (cls == null || sb.ToString() == cls)) { found = h; return false; }
      return true; }, IntPtr.Zero);
    return found;
  }
  public static IntPtr[] Find(string cls, uint[] pids) {
    var res = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid);
      var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == cls && Array.IndexOf(pids, pid) >= 0) res.Add(h); return true; }, IntPtr.Zero);
    return res.ToArray();
  }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  struct CREDENTIAL { public uint Flags, Type; public string TargetName, Comment; public long LastWritten;
    public uint CredentialBlobSize; public IntPtr CredentialBlob; public uint Persist, AttributeCount;
    public IntPtr Attributes; public string TargetAlias, UserName; }
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CredRead(string target, uint type, uint flags, out IntPtr cred);
  [DllImport("advapi32.dll")] static extern void CredFree(IntPtr cred);
  // Whether a generic credential exists, and whether its secret equals
  // `expected` (as UTF-8 or UTF-16): the secret itself never leaves here.
  // -1 missing, 0 present but different, 1 present and equal.
  public static int Matches(string target, string expected) {
    IntPtr p; if (!CredRead(target, 1, 0, out p)) return -1;
    try {
      var c = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
      var blob = new byte[c.CredentialBlobSize]; Marshal.Copy(c.CredentialBlob, blob, 0, blob.Length);
      bool eq = System.Text.Encoding.UTF8.GetString(blob) == expected || System.Text.Encoding.Unicode.GetString(blob) == expected;
      Array.Clear(blob, 0, blob.Length);
      return eq ? 1 : 0;
    } finally { CredFree(p); }
  }
  // Size of a file another process may still hold open (the directory entry
  // can lag behind on NTFS); -1 if it does not exist.
  public static long Length(string path) {
    try { using (var f = new System.IO.FileStream(path, System.IO.FileMode.Open, System.IO.FileAccess.Read, System.IO.FileShare.ReadWrite | System.IO.FileShare.Delete)) { return f.Length; } }
    catch (System.IO.FileNotFoundException) { return -1; }
  }
  // ---- upload mode ----
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  // The window manager's close (the title bar's X) without the mouse:
  // WM_SYSCOMMAND SC_CLOSE, which DefWindowProc turns into WM_CLOSE.
  public static bool SysClose(IntPtr h) { return PostMessage(h, 0x0112, (IntPtr)0xF060, IntPtr.Zero); }
  // A file of `size` bytes that does not compress and differs per seed.
  public static void WriteBlob(string path, long size, int seed) {
    using (var sha = System.Security.Cryptography.SHA256.Create())
    using (var f = new System.IO.FileStream(path, System.IO.FileMode.CreateNew)) {
      var block = sha.ComputeHash(new byte[] { (byte)seed }); var buf = new byte[32 * 64];
      for (long left = size; left > 0; ) {
        block = sha.ComputeHash(block);
        for (int i = 0; i < 64; i++) Buffer.BlockCopy(block, 0, buf, i * 32, 32);
        int n = (int)Math.Min(left, buf.Length); f.Write(buf, 0, n); left -= n;
      }
    }
  }
  // Tray menu (as in windows.ps1): the real mouse and the Win32 menu handle.
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  public static void MouseClick(int x, int y, bool right) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(80);
    uint dn = right ? 0x0008u : 0x0002u, up = right ? 0x0010u : 0x0004u;
    mouse_event(dn, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(50); mouse_event(up, 0, 0, 0, UIntPtr.Zero);
  }
  public static void Esc() { keybd_event(0x1B, 0, 0, UIntPtr.Zero); keybd_event(0x1B, 0, 2, UIntPtr.Zero); }
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  // MN_GETHMENU of an open popup menu window (#32768).
  public static IntPtr MenuOf(IntPtr h) { return SendMessage(h, 0x01E1, IntPtr.Zero, IntPtr.Zero); }
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint id, System.Text.StringBuilder s, int max, uint flag);
  public static string[] MenuStrings(IntPtr hm) {
    int n = GetMenuItemCount(hm); var r = new string[Math.Max(n, 0)];
    for (int i = 0; i < n; i++) { var sb = new System.Text.StringBuilder(256); GetMenuString(hm, (uint)i, sb, 256, 0x400); r[i] = sb.ToString(); }
    return r;
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint i, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern IntPtr GetThreadDesktop(uint tid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int i, System.Text.StringBuilder b, int n, out int need);
  public static string DesktopName(IntPtr h) {
    if (h == IntPtr.Zero) return ""; var b = new System.Text.StringBuilder(256); int n;
    return GetUserObjectInformation(h, 2, b, 512, out n) ? b.ToString() : "";
  }
  public static string ClassOf(IntPtr h) { var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public static bool SameBytes(string a, string b) {
    using (var x = System.IO.File.OpenRead(a)) using (var y = System.IO.File.OpenRead(b)) {
      if (x.Length != y.Length) return false;
      var bx = new byte[1 << 16]; var by = new byte[1 << 16];
      while (true) {
        int n = x.Read(bx, 0, bx.Length); if (n == 0) return true;
        int m = 0; while (m < n) { int k = y.Read(by, m, n - m); if (k == 0) return false; m += k; }
        for (int i = 0; i < n; i++) if (bx[i] != by[i]) return false;
      }
    }
  }
}
"@
$ErrorActionPreference = "Continue"

$A = [System.Windows.Automation.AutomationElement]; $S = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]
$Invoke = [System.Windows.Automation.InvokePattern]::Pattern
$ValuePattern = [System.Windows.Automation.ValuePattern]::Pattern
$Toggle = [System.Windows.Automation.TogglePattern]::Pattern
$script:pass = 0; $script:fail = 0
$script:redact = @()
function Redact([string]$s) { foreach ($r in $script:redact) { if ($r[0]) { $s = $s.Replace($r[0], $r[1]) } }; $s }
function Check($name, [bool]$ok, $detail = "") {
  if ($ok) { $script:pass++; "PASS  $name" } else { $script:fail++; $d = Redact "$detail"; if ($d.Length -gt 800) { $d = $d.Substring(0, 800) }; "FAIL  $name  $d" }
}
function WaitFor([scriptblock]$pred, [double]$timeout = 10) {
  $end = (Get-Date).AddSeconds($timeout)
  while ((Get-Date) -lt $end) { if (& $pred) { return $true }; Start-Sleep -Milliseconds 150 }
  return [bool](& $pred)
}
function Sha($path) { (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant() }
function Procs { @(Get-Process sotyware -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }
function Dash {
  $p = [uint32[]]@((Procs).Id); if ($p.Count -eq 0) { return $null }
  $h = @([CloudUiW]::Find("Tauri Window", $p)); if ($h.Count -eq 0) { return $null }
  $A::FromHandle($h[0])
}
function PCT($type) { [System.Windows.Automation.PropertyCondition]::new($A::ControlTypeProperty, $type) }
function PCN($name) { [System.Windows.Automation.PropertyCondition]::new($A::NameProperty, $name) }
# The last control of that type and name (the form's submit button comes after
# the page's own "Add account" button).
function Named($name, $type) {
  $d = Dash; if (-not $d) { return $null }
  $all = @($d.FindAll($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $type), (PCN $name))))
  if ($all.Count -eq 0) { $null } else { $all[$all.Count - 1] }
}
function PageText { $d = Dash; if (-not $d) { return "" }; try { @($d.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name }) -join " | " } catch { "" } }
# WaitFor runs its block in a child scope, so results go through $script: variables.
function Press($name) { $script:found = $null; WaitFor { $script:found = Named $name $CT::Button; $null -ne $script:found } 10 | Out-Null; if (-not $script:found) { return $false }; try { $script:found.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
# A button by its name without surrounding spaces or the folder icon (folder
# rows hold a hidden icon next to the name).
function PressFolder($name) {
  $script:found = $null
  WaitFor {
    $d = Dash; if (-not $d) { return $false }
    $script:found = @($d.FindAll($S::Descendants, (PCT $CT::Button)) | Where-Object { $_.Current.Name.Replace([string][char]::ConvertFromUtf32(0x1F4C1), "").Trim() -eq $name })[0]
    $null -ne $script:found
  } 10 | Out-Null
  if (-not $script:found) { Write-Host "  no button '$name'"; return $false }
  try { $script:found.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false }
}
function Link($name) { $l = Named $name $CT::Hyperlink; if (-not $l) { return $false }; try { $l.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
# Sets a field through the Value pattern (WebView2 fires the input event the
# form listens to); checks the length afterwards (password fields are masked).
# A text input with a datalist (Region) is a ComboBox to UI Automation.
function Fill($name, [string]$value) {
  $script:field = $null; WaitFor { $script:field = Named $name $CT::Edit; if (-not $script:field) { $script:field = Named $name $CT::ComboBox }; $null -ne $script:field } 10 | Out-Null
  $e = $script:field
  if (-not $e) { Write-Host "  no field '$name'"; return $false }
  try { $e.GetCurrentPattern($ValuePattern).SetValue($value) } catch { Write-Host "  fill '$name': $($_.Exception.Message)"; return $false }
  WaitFor { $e.GetCurrentPattern($ValuePattern).Current.Value.Length -eq $value.Length } 5
}
function Fields { $d = Dash; if (-not $d) { return @() }; @($d.FindAll($S::Descendants, [System.Windows.Automation.OrCondition]::new((PCT $CT::Edit), (PCT $CT::ComboBox))) | ForEach-Object { $_.Current.Name }) }
$Labels = @("Connected", "Invalid credentials", "Access denied", "Bucket unavailable", "Network error",
  "Credential store unavailable", "Credential store locked", "Credentials missing", "Configuration problem", "Check failed")
# Presses Check connection on the only account; the status label afterwards.
function RunCheck([double]$timeout = 60) {
  if (-not (Press "Check connection")) { return $null }
  $script:status = $null
  WaitFor {
    $t = PageText
    if ($t -like "*Checking connection*") { return $false }
    # No label's text occurs elsewhere on the page (hints and intro use other words).
    foreach ($l in $Labels) { if ($t.Contains($l)) { $script:status = $l; return $true } }
    $false
  } $timeout | Out-Null
  $script:status
}
# This run's namespace only (ADR-027); never a com.sotyware.desktop/ target.
function Target($id) { Get-TestCredentialTarget $Prof $id }
function Stored($key, $secret) { "sotyware-s3-credentials-v1`n$key`n$secret" }
# The user's own sotyware entries, compared before and after the run.
function CmdkeyLines { Get-NormalCredentialLines }
# Entries cmdkey lists for exactly this target. Only its "Target:" lines count:
# the header "Currently stored credentials for target: <target>" names the
# target even when nothing is stored.
function CmdkeyEntries($target) { @(cmdkey "/list:$target" | Where-Object { $_ -match '^\s*Target:' -and $_.TrimEnd().EndsWith($target) }).Count }

$cv = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
"sotyware Cloud Storage UI test ($Mode)  $(Get-Date -Format s)"
"os: $((Get-CimInstance Win32_OperatingSystem).Caption) $($cv.DisplayVersion) build $($cv.CurrentBuild).$($cv.UBR)"
"binary: $Exe"
if ((Procs).Count -gt 0) { "ABORT: sotyware from this binary is already running"; exit 2 }
if ($Mode -eq "upload" -or ($IsLc -and $Scenario -eq "quit") -or $RecordRequests) {
  # The tray Quit needs real clicks, which reach only the input desktop with
  # a taskbar (as in windows.ps1); stop before starting anything otherwise.
  $inputDesk = [CloudUiW]::DesktopName([CloudUiW]::OpenInputDesktop(0, $false, 0x0001))
  $ownDesk = [CloudUiW]::DesktopName([CloudUiW]::GetThreadDesktop([CloudUiW]::GetCurrentThreadId()))
  if ($inputDesk -eq "" -or $inputDesk -ne $ownDesk) { "ABORT: not on the interactive input desktop (this desktop: '$ownDesk', input desktop: '$inputDesk')"; exit 3 }
  if ([CloudUiW]::FindWindow("Shell_TrayWnd", $null) -eq [IntPtr]::Zero) { "ABORT: no taskbar (Shell_TrayWnd) on this desktop"; exit 3 }
  [void][CloudUiW]::SetProcessDPIAware()
  "binary SHA-256: $(Sha $Exe), built $((Get-Item $Exe).LastWriteTime.ToString('s'))"
  "this test moves the mouse once (tray Quit); do not use the mouse or keyboard while it runs"
}
if ($IsLc -and $Scenario -ne "quit") {
  "binary SHA-256: $((Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash.ToLowerInvariant()), built $((Get-Item $Exe).LastWriteTime.ToString('s'))"
}
if ($Mode -eq "b2-upload") {
  "binary SHA-256: $((Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash.ToLowerInvariant()), built $((Get-Item $Exe).LastWriteTime.ToString('s'))"
  "variant: $(if ($Rehearse) { 'rehearsal against the in-process server (no provider)' } else { 'Backblaze B2 (confirmed with -ConfirmB2Upload)' }); does not move the mouse; do not use the mouse or keyboard while it runs"
}

$run = (Get-Date).ToUniversalTime().ToString("yyyyMMddHHmmssfff")
if ($RecordRequests) {
  $RecDir = Join-Path $env:TEMP "sotyware-recording-$Mode-$run"
  New-Item -ItemType Directory $RecDir | Out-Null
  "request recorder: the app records into $RecDir (kept afterwards)"
}
if ($Mode -eq "b2-upload") {
  # The local files, before anything else (the rehearsal server is seeded
  # with seed\existing.txt; the Backblaze prepare test uploads it). .NET
  # calls only: the helper functions are defined further down.
  $UiPrefix = "sotyware-it/upload-ui-validation/$run/"
  $UiUni = "gr$([char]0xF6)$([char]0xDF)e +test.bin"
  $UiNames = @("existing.txt", "a-small.txt", $UiUni, "progress.bin")
  $UpSrc = Join-Path $env:TEMP "sotyware-b2-upload-$run"
  $UiBatch = Join-Path $UpSrc "batch"; $UiSeedDir = Join-Path $UpSrc "seed"
  New-Item -ItemType Directory $UiBatch, $UiSeedDir | Out-Null
  $UiSeed = Join-Path $UiSeedDir "existing.txt"
  [System.IO.File]::WriteAllBytes($UiSeed, [System.Text.Encoding]::UTF8.GetBytes("seeded before the dashboard upload, run $run`n"))
  [System.IO.File]::WriteAllBytes((Join-Path $UiBatch "existing.txt"), [System.Text.Encoding]::UTF8.GetBytes("the dashboard's own existing.txt: must be skipped, never stored (run $run)`n"))
  [System.IO.File]::WriteAllBytes((Join-Path $UiBatch "a-small.txt"), [System.Text.Encoding]::UTF8.GetBytes("a small file uploaded through the sotyware dashboard (run $run)`n"))
  [CloudUiW]::WriteBlob((Join-Path $UiBatch $UiUni), 200003, 2)
  [CloudUiW]::WriteBlob((Join-Path $UiBatch "progress.bin"), 24 * 1048576 + 5, 3)
  "b2-upload: run id $run, run prefix $UiPrefix, local files in $UpSrc"
}
if ($IsLc) {
  # One scenario per run, under its own prefix; the local files first.
  $LcRun = "$Scenario-$run"
  $UiPrefix = "sotyware-it/upload-lifecycle-validation/$LcRun/"
  $LcMiB = 1048576
  $UpSrc = Join-Path $env:TEMP "sotyware-b2-lifecycle-$LcRun"
  $UiBatch = Join-Path $UpSrc "batch"; $UiSeedDir = Join-Path $UpSrc "seed"
  New-Item -ItemType Directory $UiBatch, $UiSeedDir | Out-Null
  $UiSeed = Join-Path $UiSeedDir "anchor.txt"
  [System.IO.File]::WriteAllBytes($UiSeed, [System.Text.Encoding]::UTF8.GetBytes("anchor of upload lifecycle run $LcRun`n"))
  function LcText([string]$name, [string]$text) { [System.IO.File]::WriteAllBytes((Join-Path $UiBatch $name), [System.Text.Encoding]::UTF8.GetBytes("$text (run $LcRun)`n")) }
  switch ($Scenario) {
    "multipart" { [CloudUiW]::WriteBlob((Join-Path $UiBatch "multipart.bin"), 72 * $LcMiB + 3, 21); $UiNames = @("multipart.bin"); $LcStored = @("multipart.bin") }
    "cancel" { [CloudUiW]::WriteBlob((Join-Path $UiBatch "cancel.bin"), 144 * $LcMiB, 22); $UiNames = @("cancel.bin"); $LcStored = @() }
    "retry" {
      LcText "anchor.txt" "the dashboard's own anchor.txt: must be skipped, never stored"
      LcText "a-first.txt" "uploaded by the first batch, never again"
      LcText "b-locked.txt" "held open without sharing during the first batch"
      LcText "c-third.txt" "uploaded by the first batch after the failure"
      $UiNames = @("anchor.txt", "a-first.txt", "b-locked.txt", "c-third.txt"); $LcStored = @("a-first.txt", "b-locked.txt", "c-third.txt")
    }
    "leave" { [CloudUiW]::WriteBlob((Join-Path $UiBatch "a-away.bin"), 60 * $LcMiB, 23); LcText "b-away.txt" "never started: the browser was left"; $UiNames = @("a-away.bin", "b-away.txt"); $LcStored = @() }
    "quit" { [CloudUiW]::WriteBlob((Join-Path $UiBatch "quit.bin"), 144 * $LcMiB, 24); $UiNames = @("quit.bin"); $LcStored = @() }
    "settle" { [CloudUiW]::WriteBlob((Join-Path $UiBatch "settle.bin"), 144 * $LcMiB, 25); $UiNames = @("settle.bin"); $LcStored = @() }
  }
  "b2-lifecycle: scenario $Scenario, run id $LcRun, run prefix $UiPrefix, local files in $UpSrc"
  "variant: $(if ($Rehearse) { 'rehearsal against the in-process server (no provider)' } else { 'Backblaze B2 (confirmed with -ConfirmB2Lifecycle)' })$(if ($Scenario -ne 'quit') { '; does not move the mouse' }); do not use the mouse or keyboard while it runs"
}
if ($Mode -eq "b2-browse" -or $Mode -eq "b2-download") {
  $dir = Join-Path $env:USERPROFILE ".sotyware-it"
  foreach ($f in "b2.xml", "b2-read.xml", "b2-bucket.txt") { if (-not (Test-Path "$dir\$f")) { "ABORT: $dir\$f missing (see testing.md)"; exit 2 } }
  $w = Import-Clixml "$dir\b2.xml"; $WriteKey = $w.UserName; $WriteSecret = $w.GetNetworkCredential().Password; $w = $null
  $c = Import-Clixml "$dir\b2-read.xml"
  $Key1 = $c.UserName; $Secret1 = $c.GetNetworkCredential().Password
  $Bucket = (Get-Content "$dir\b2-bucket.txt" -Raw).Trim()
  # Replacing the key only stores it (no request); made-up values.
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $Endpoint = ""
} elseif ($Mode -eq "b2") {
  $dir = Join-Path $env:USERPROFILE ".sotyware-it"
  if (-not (Test-Path "$dir\b2.xml") -or -not (Test-Path "$dir\b2-bucket.txt")) { "ABORT: $dir\b2.xml or b2-bucket.txt missing (see testing.md)"; exit 2 }
  $c = Import-Clixml "$dir\b2.xml"
  $Key1 = $c.UserName; $Secret1 = $c.GetNetworkCredential().Password
  $Bucket = (Get-Content "$dir\b2-bucket.txt" -Raw).Trim()
  $Key2 = $Key1; $Secret2 = "sotywareE2eWrongSecret0123456789abcdef"
  $Endpoint = ""
} elseif ($IsB2Up -and -not $Rehearse) {
  $dir = Join-Path $env:USERPROFILE ".sotyware-it"
  foreach ($f in "b2.xml", "b2-upload.xml", "b2-bucket.txt") { if (-not (Test-Path "$dir\$f")) { "ABORT: $dir\$f missing (see testing.md)"; [System.IO.Directory]::Delete($UpSrc, $true); exit 2 } }
  # b2.xml (prefix sotyware-it/): the prepare, verify and cleanup tests only.
  $w = Import-Clixml "$dir\b2.xml"; $WriteKey = $w.UserName; $WriteSecret = $w.GetNetworkCredential().Password; $w = $null
  # b2-upload.xml (no file name prefix): the app's key.
  $c = Import-Clixml "$dir\b2-upload.xml"
  $Key1 = $c.UserName; $Secret1 = $c.GetNetworkCredential().Password
  $Bucket = (Get-Content "$dir\b2-bucket.txt" -Raw).Trim()
  if ($Key1 -eq $WriteKey) { "ABORT: b2-upload.xml holds the write key of b2.xml; the app needs its own key without a file name prefix"; [System.IO.Directory]::Delete($UpSrc, $true); exit 2 }
  # Replacing the key only stores it (no request); made-up values.
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $Endpoint = ""
} elseif ($Mode -eq "browse" -or $Mode -eq "download" -or $Mode -eq "upload" -or $IsB2Up -or $Mode -eq "files") {
  # The made-up keys of serve_listing_for_ui / serve_objects_for_ui /
  # serve_uploads_for_ui (src-tauri/src/cloud/test_server.rs) and
  # serve_files_for_ui (test_server/files_ui.rs).
  if ($Mode -eq "files") {
    $Key1 = "e2eFilesKey00000000001"; $Secret1 = "e2e-Files-Secret-0123456789"; $Bucket = "files-versioned"; $Helper = "files_ui::serve_files_for_ui"
  } elseif ($Mode -eq "browse") {
    $Key1 = "e2eBrowseKey0000000001"; $Secret1 = "e2e-Browse-Secret-0123456789"; $Bucket = "browse-bucket"; $Helper = "serve_listing_for_ui"
  } elseif ($Mode -eq "upload" -or $IsB2Up) {
    $Key1 = "e2eUploadKey0000000001"; $Secret1 = "e2e-Upload-Secret-0123456789"; $Bucket = "upload-bucket"; $Helper = "serve_uploads_for_ui"
  } else {
    $Key1 = "e2eDownloadKey00000001"; $Secret1 = "e2e-Download-Secret-0123456789"; $Bucket = "download-bucket"; $Helper = "serve_objects_for_ui"
  }
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $SrvFile = Join-Path $env:TEMP "sotyware-$Mode-endpoint-$run.txt"
  $SrvOut = Join-Path $env:TEMP "sotyware-$Mode-server-$run.log"
  $env:SOTYWARE_UI_SERVER_FILE = $SrvFile
  # upload: the link takes 2 MiB/s (the helper's default is 6 MiB/s), so that
  # UI Automation, much slower to read than AT-SPI, still sees a 40 MiB PUT
  # in progress and can cancel it.
  if ($Mode -eq "upload" -or $IsB2Up) { $env:SOTYWARE_UI_UPLOAD_RATE = "$(2 * 1048576)" }
  # settle: every body taken at once, forwarded at 6 MiB/s (a 16 MiB part
  # about 2.7 s, inside the app's 5 s bound for the wait).
  if ($Scenario -eq "settle") { $env:SOTYWARE_UI_UPLOAD_RATE = "$(6 * 1048576)"; $env:SOTYWARE_UI_UPLOAD_PACE = "buffered" }
  # The rehearsal's bucket holds only the seed, under the run's key.
  if ($Mode -eq "b2-upload") { $env:SOTYWARE_UI_UPLOAD_SEED_KEY = "${UiPrefix}existing.txt"; $env:SOTYWARE_UI_UPLOAD_SEED_FILE = $UiSeed }
  if ($IsLc) { $env:SOTYWARE_UI_UPLOAD_SEED_KEY = "${UiPrefix}anchor.txt"; $env:SOTYWARE_UI_UPLOAD_SEED_FILE = $UiSeed }
  if ($RecordRequests) { $env:SOTYWARE_UI_RECORDING_DIR = $RecDir }
  $srv = Start-Process cargo -ArgumentList "test", "--quiet", "--lib", "cloud::test_server::$Helper", "--", "--ignored", "--nocapture" `
    -WorkingDirectory (Join-Path $PSScriptRoot "..\..\src-tauri") -NoNewWindow -PassThru -RedirectStandardOutput $SrvOut -RedirectStandardError "$SrvOut.err"
  $null = $srv.Handle  # without a cached handle, ExitCode stays empty after the exit
  Remove-Item Env:SOTYWARE_UI_SERVER_FILE
  Remove-Item Env:SOTYWARE_UI_UPLOAD_RATE, Env:SOTYWARE_UI_UPLOAD_PACE, Env:SOTYWARE_UI_UPLOAD_SEED_KEY, Env:SOTYWARE_UI_UPLOAD_SEED_FILE, Env:SOTYWARE_UI_RECORDING_DIR -EA SilentlyContinue
  if (-not (WaitFor { (Test-Path $SrvFile) -and (Get-Content $SrvFile -Raw) } 900)) { "ABORT: the listing server did not start"; Get-Content "$SrvOut.err" -Tail 20; exit 2 }
  $Endpoint = (Get-Content $SrvFile -Raw).Trim()
  "test server: $Endpoint"
  if ($Mode -eq "upload" -or $IsB2Up) { @(Get-Content $SrvOut -Encoding UTF8 | Where-Object { $_ -match '^serving' }) | ForEach-Object { "  $_" } }
  if ($Mode -eq "files") {
    $Out = Join-Path $env:TEMP "sotyware-files-downloads-$run"
    New-Item -ItemType Directory $Out | Out-Null
  }
  if ($Mode -eq "download" -or $Mode -eq "upload" -or $IsB2Up) {
    # download: the objects to download; upload: the seeded objects.
    $script:objects = @{}; foreach ($o in (Get-Content "$SrvFile.json" -Raw -Encoding UTF8 | ConvertFrom-Json)) { $script:objects[[string]$o.key] = $o }
    $Out = Join-Path $env:TEMP "sotyware-downloads-$run"
    New-Item -ItemType Directory $Out | Out-Null
  }
  if ($Mode -eq "upload") {
    $UpSrc = Join-Path $env:TEMP "sotyware-upload-src-$run"
    New-Item -ItemType Directory $UpSrc | Out-Null
    "upload test: a new temporary bucket '$Bucket' for this run (run id $run), local files in $UpSrc"
  }
} else {
  $Key1 = "e2eKeyIdOne000000000001"; $Secret1 = "e2e-Secret-One-0123456789abcdef"
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $Bucket = "soty-photos"; $Endpoint = "http://127.0.0.1:9"
}
# Set after the helper server started, so only the app (started from here)
# records; a later cargo test child ignores them (no recorder in test builds
# without the feature).
if ($RecordRequests) {
  $env:SOTYWARE_REQUEST_RECORD_DIR = $RecDir
  $env:SOTYWARE_REQUEST_RECORD_RUN = "ui-$Mode-$run"
  $env:SOTYWARE_REQUEST_RECORD_SCOPE = $(if ($UiPrefix) { $UiPrefix } else { "inbox/" })
  $env:SOTYWARE_REQUEST_RECORD_ACCOUNT = "sotyware-e2e-$run"
}
$script:redact = @(@($Secret1, "<secret>"), @($Secret2, "<secret2>"), @($Key1, "<keyID>"), @($Key2, "<keyID2>"), @($Bucket, "<bucket>"))
if ($Mode -eq "b2-browse" -or $Mode -eq "b2-download" -or ($IsB2Up -and -not $Rehearse)) { $script:redact += , @($WriteSecret, "<write-secret>"); $script:redact += , @($WriteKey, "<write-keyID>") }

# Runs the opt-in Rust test that fingerprints the B2 bucket; the variables are
# set for that child only and removed again.
# Runs one opt-in Rust test with the B2 variables set for that child only
# (removed again afterwards); returns its output lines.
function CargoIt($test, $extra = @{}) {
  $vars = @{ SOTYWARE_IT_B2_BUCKET = $Bucket; SOTYWARE_IT_B2_REGION = $Region }
  $exact = @()
  if ($IsB2Up) {
    # Only the write key (b2.xml) reaches these tests; the app's key never does.
    $vars.SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $WriteKey; $vars.SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $WriteSecret
    $vars.SOTYWARE_IT_B2_UPLOAD_CONFIRM = "upload-validation"
    $exact = @("--exact")
  } elseif ($Mode -eq "b2-browse" -or $Mode -eq "b2-download") {
    $vars.SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $WriteKey; $vars.SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $WriteSecret
    $vars.SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID = $Key1; $vars.SOTYWARE_CLOUD_IT_B2_READ_SECRET_ACCESS_KEY = $Secret1
  } else {
    $vars.SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $Key1; $vars.SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $Secret1
  }
  foreach ($k in $extra.Keys) { $vars[$k] = $extra[$k] }
  foreach ($k in $vars.Keys) { Set-Item "Env:$k" $vars[$k] }
  try {
    Push-Location (Join-Path $PSScriptRoot "..\..\src-tauri")
    # b2-upload keeps stderr too (filtered by the caller): a failed
    # assertion's panic message goes there.
    if ($IsB2Up) { $out = & cargo test --quiet --lib $test -- --ignored --nocapture @exact 2>&1 | ForEach-Object { "$_" } }
    else { $out = & cargo test --quiet --lib $test -- --ignored --nocapture @exact 2>$null }
    $script:cargoExit = $LASTEXITCODE
    Pop-Location
  } finally {
    foreach ($k in $vars.Keys) { Remove-Item "Env:$k" -EA SilentlyContinue }
  }
  @($out)
}
function Fingerprint {
  @(CargoIt "cloud::integration::backblaze_b2_fingerprint" | Where-Object { $_ -match '^sotyware-it fingerprint [0-9a-f]{64} versions \d+$' })[0]
}
$script:browseRun = $null
# Deletes exactly the versions of this run's kept dataset and lists again.
function BrowseCleanup {
  if (-not $script:browseRun) { return }
  $out = CargoIt "cloud::integration::browse::backblaze_b2_browse_cleanup" @{ SOTYWARE_IT_B2_BROWSE_RUN = $script:browseRun }
  $out | Where-Object { $_ -match '^(cleaning up|cleanup:|CLEANUP)' } | ForEach-Object { "  $(Redact $_)" }
  Check "b2-browse: cleanup deleted this run's versions by id and found none left (run $script:browseRun)" ($script:cargoExit -eq 0)
  if ($script:cargoExit -eq 0) { $script:browseRun = $null }
}

# b2-download: the run id and version id of the kept object, once uploaded.
$script:dlRun = $null; $script:dlVersion = $null
# Deletes exactly that version and lists the run prefix once; never retried
# (a refusal, such as the daily transaction cap, is reported, not repeated).
function DownloadCleanup {
  if (-not $script:dlRun) { return }
  $out = CargoIt "cloud::integration::download::backblaze_b2_download_ui_cleanup" @{ SOTYWARE_IT_B2_DOWNLOAD_UI_RUN = $script:dlRun; SOTYWARE_IT_B2_DOWNLOAD_UI_VERSION = $script:dlVersion }
  $out | Where-Object { $_ -match '^(cleaning up|cleanup:|write key:|CLEANUP|thread .* panicked)' } | ForEach-Object { "  $(Redact $_)" }
  Check "b2-download: cleanup deleted version $script:dlVersion of run $script:dlRun and listed 0 versions left" ($script:cargoExit -eq 0)
  if ($script:cargoExit -eq 0) { $script:dlRun = $null } else { "  NOT CLEANED UP: run $script:dlRun, version $script:dlVersion, key sotyware-it/download-ui-validation/$script:dlRun/<file>; not retried" }
}

# b2-upload: the run (set before the prepare, so the cleanup always runs),
# the seed's version id, and whether Open was pressed in the dialog (the app
# may have uploaded something).
$script:upRun = $null; $script:seedVersion = $null; $script:uiBatchRan = $false
function TestExe {
  $e = Get-ChildItem (Join-Path $PSScriptRoot "..\..\src-tauri\target\debug\deps\sotyware_lib-*.exe") -EA SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($e) { "$($e.FullName) SHA-256 $((Get-FileHash -Algorithm SHA256 -LiteralPath $e.FullName).Hash.ToLowerInvariant())" } else { "none" }
}
# Verifies what arrived (if the batch may have run) and deletes exactly this
# run's versions by id; never retried.
function UploadUiFinish {
  if (-not $script:upRun) { return }
  $vars = @{ SOTYWARE_IT_B2_UPLOAD_UI_RUN = $script:upRun; SOTYWARE_IT_B2_UPLOAD_UI_DIR = $UpSrc }
  $verify = $script:uiBatchRan -and $script:seedVersion
  if ($verify) {
    $vars.SOTYWARE_IT_B2_UPLOAD_UI_SEED_VERSION = $script:seedVersion
    $res = CargoIt "cloud::integration::upload::ui::backblaze_b2_upload_ui_verify" $vars
  } else {
    "  the batch never started: cleanup only"
    $res = CargoIt "cloud::integration::upload::ui::backblaze_b2_upload_ui_cleanup" $vars
  }
  $res | Where-Object { $_ -match '^(verifying|cleaning up|found:|after cleanup:|left:|NOT OURS|CLEANUP|upload-ui|download |\d+ requests:|  (GET|PUT|HEAD|DELETE|POST) |thread .* panicked|assertion|  left|  right|the download checks|what the provider|requests: |the run scope)' } | ForEach-Object { "  $(Redact $_)" }
  $clean = @($res | Where-Object { $_ -eq "upload-ui cleanup: clean" }).Count -eq 1
  if ($verify) {
    Check "b2-upload: the provider held exactly the seed and the three uploaded files, one version each, existing.txt still the seed's version; each downloaded byte-equal with its SHA-256 verified" (($script:cargoExit -eq 0) -and @($res | Where-Object { $_ -eq "upload-ui verify: passed" }).Count -eq 1)
  }
  Check "b2-upload: cleanup deleted this run's versions by id and listed no version or multipart upload left (run $script:upRun)" $clean
  if ($clean) { $script:upRun = $null } else { "  NOT CLEANED UP: run $script:upRun under $UiPrefix; run backblaze_b2_upload_ui_cleanup with SOTYWARE_IT_B2_UPLOAD_UI_RUN=$script:upRun; not retried" }
}
# b2-lifecycle: the same, for one scenario's run.
function LcVars { @{ SOTYWARE_IT_B2_LIFECYCLE_RUN = $LcRun; SOTYWARE_IT_B2_LIFECYCLE_DIR = $UpSrc; SOTYWARE_IT_B2_LIFECYCLE_SCENARIO = $Scenario } }
$LcKeep = '^(provider|verifying|cleaning up|found:|after cleanup:|left:|local |run prefix|NOT OURS|CLEANUP|upload-lifecycle|download |\d+ requests:|  (GET|PUT|HEAD|DELETE|POST) |thread .* panicked|assertion|  left|  right|the download checks|what the provider|requests: |the run scope|nothing uploaded|anchor )'
function LifecycleFinish {
  if (-not $script:upRun) { return }
  $vars = LcVars
  $verify = $script:uiBatchRan -and $script:seedVersion
  if ($verify) {
    $vars.SOTYWARE_IT_B2_LIFECYCLE_ANCHOR_VERSION = $script:seedVersion
    $res = CargoIt "cloud::integration::upload::lifecycle::backblaze_b2_upload_lifecycle_verify" $vars
  } else {
    "  the batch never started: cleanup only"
    $res = CargoIt "cloud::integration::upload::lifecycle::backblaze_b2_upload_lifecycle_cleanup" $vars
  }
  $res | Where-Object { $_ -match $LcKeep } | ForEach-Object { "  $(Redact $_)" }
  $clean = @($res | Where-Object { $_ -eq "upload-lifecycle cleanup: clean" }).Count -eq 1
  if ($verify) {
    $what = if ($LcStored.Count -gt 0) { "the anchor (its own version) and $($LcStored -join ', '), one version each, each downloaded byte-equal with its SHA-256 verified" } else { "only the anchor (its own version): nothing of the batch" }
    Check "b2-lifecycle: the provider held $what; no incomplete multipart upload" (($script:cargoExit -eq 0) -and @($res | Where-Object { $_ -eq "upload-lifecycle verify: passed" }).Count -eq 1)
  }
  Check "b2-lifecycle: cleanup deleted this run's versions by id and listed no version or multipart upload left (run $LcRun)" $clean
  if ($clean) { $script:upRun = $null } else { "  NOT CLEANED UP: run $LcRun under $UiPrefix; run backblaze_b2_upload_lifecycle_cleanup with SOTYWARE_IT_B2_LIFECYCLE_RUN=$LcRun SOTYWARE_IT_B2_LIFECYCLE_SCENARIO=$Scenario; not retried" }
}
# While the dashboard uploads: the open multipart uploads under the run
# prefix, as "<count>:<names>" ($null if unknown). Backblaze: one
# ListMultipartUploads with the write key; rehearsal: the server's state.
function LcOpenUploads {
  # s3s-fs keeps two files per open upload (".upload-<id>.json" and its
  # ".metadata.json"): count the distinct upload ids.
  if ($Rehearse) { $st = State; $ids = @($st.Multipart | ForEach-Object { [regex]::Match([string]$_, 'upload-([0-9a-f-]{36})').Groups[1].Value } | Where-Object { $_ } | Sort-Object -Unique); return "$($ids.Count):s3s-fs" }
  $res = CargoIt "cloud::integration::upload::lifecycle::backblaze_b2_upload_lifecycle_probe" (LcVars)
  $res | Where-Object { $_ -match '^(upload-lifecycle probe|\d+ requests:|  GET |thread .* panicked)' } | ForEach-Object { "  $(Redact $_)" }
  $m = [regex]::Match(($res -join "`n"), 'upload-lifecycle probe: (\d+) open multipart upload\(s\): \[([^\]]*)\]')
  if ($script:cargoExit -ne 0 -or -not $m.Success) { return $null }
  "$($m.Groups[1].Value):$($m.Groups[2].Value -replace '"', '')"
}

# ---- test profile (ADR-027) ----
$NormalBefore = Get-NormalProfileFingerprint
$Prof = New-TestProfile $ProfileParent "cloud-$Mode-$run"
# The user's real folder, seen from outside this (possibly MSIX-virtualized) process tree.
$RealBefore = Get-RealNormalProfileFingerprint $Prof.Parent
"user's real app folder (outside this process tree): $(if ($null -eq $RealBefore) { 'could not be read' } else { "$(@($RealBefore).Count) line(s)" })"
$env:SOTYWARE_TEST_PROFILE = $Prof.Root
"test profile: $($Prof.Root) (credentials under $($Prof.TargetPrefix))"
$CloudDb = Join-Path $Prof.DataDir "cloud-accounts.sqlite3"
# Stops the run unless $proc acknowledges this run's profile; nothing is
# clicked in a window whose data and credentials are not provably this run's.
function Verified($proc, [string]$label) {
  try { $null = Wait-TestProfileAck $Prof $proc.Id 30 $proc; Check "${label}: the app runs in this run's test profile (acknowledged)" $true }
  catch {
    Check "${label}: the app runs in this run's test profile (acknowledged)" $false "$_"
    Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
    throw "ABORT: the app is not provably isolated; nothing else is run"
  }
}
# Removes this run's profile and credentials; called on every way out.
function ProfileCleanup {
  $null = WaitFor { (Procs).Count -eq 0 } 10
  Remove-TestCredentials $Prof
  Check "cleanup: no Credential Manager entry of this run's profile left" ((Get-TestCredentialLines $Prof).Count -eq 0)
  if (Test-Path -LiteralPath $Prof.Root) { Remove-TestProfile $Prof }
  Check "cleanup: this run's test profile removed" (-not (Test-Path -LiteralPath $Prof.Root))
  Check "the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|"))
  if ($null -ne $RealBefore) {
    $realAfter = Get-RealNormalProfileFingerprint $Prof.Parent
    Check "the user's real app folder, read from outside this process tree, is unchanged" ($null -ne $realAfter -and (@($realAfter) -join "|") -eq (@($RealBefore) -join "|"))
  }
}
# Unique names: a step that reused $before once made this check compare the
# wrong thing (2026-11-14, files mode).
$UserCredBefore = CmdkeyLines
"credential manager: $($UserCredBefore.Count) com.sotyware.desktop line(s) before the run"

if ($Mode -eq "b2" -or $Mode -eq "b2-browse") {
  $fpBefore = Fingerprint
  Check "b2: bucket fingerprinted before the run" ($null -ne $fpBefore)
  if ($fpBefore) { "  before: $($fpBefore -replace '^sotyware-it fingerprint ', '')" }
}

if ($Mode -eq "b2-download") {
  $DlName = "gr$([char]0xF6)$([char]0xDF)e test.bin"
  $SrcDir = Join-Path $env:TEMP "sotyware-b2-download-source-$run"
  $Out = Join-Path $env:TEMP "sotyware-b2-download-dest-$run"
  New-Item -ItemType Directory $SrcDir, $Out | Out-Null
  # Not $out: names ignore case, and $Out is the download folder.
  $prep = CargoIt "cloud::integration::download::backblaze_b2_download_ui_prepare" @{ SOTYWARE_IT_B2_DOWNLOAD_UI_DIR = $SrcDir }
  $prepExit = $script:cargoExit
  $m = [regex]::Match(($prep -join "`n"), 'download-ui object kept; run id ([0-9A-Za-z_-]+) version (\S+)')
  if ($m.Success) { $script:dlRun = $m.Groups[1].Value; $script:dlVersion = $m.Groups[2].Value }
  $prep | Where-Object { $_ -match '^(provider|run prefix|generated|upload|write key|download-ui|manifest|thread .* panicked)' } | ForEach-Object { "  $(Redact $_)" }
  $ManFile = Join-Path $SrcDir "manifest.json"
  $prepOk = ($prepExit -eq 0) -and $script:dlRun -and (Test-Path $ManFile)
  Check "b2-download: account checked, run prefix empty, file generated and uploaded (run $script:dlRun)" $prepOk
  if (-not $script:dlRun -and $prepExit -ne 0) { "  no version id was reported: if the upload reached Backblaze anyway, look under the run prefix printed above and delete by version id" }
  if ($prepOk) {
    $Dl = Get-Content $ManFile -Raw -Encoding UTF8 | ConvertFrom-Json
    $Source = Join-Path $SrcDir "source.bin"
    # The page is driven to the manifest's run; the cleanup deletes the run
    # and version printed by the upload: both must name the same object.
    $prepOk = ($Dl.run -eq $script:dlRun) -and ($Dl.versionId -eq $script:dlVersion) -and ($Dl.fileName -eq $DlName) -and ([long]$Dl.size -eq (Get-Item $Source).Length) -and ((Sha $Source) -eq $Dl.sha256) -and ($Dl.key -eq "sotyware-it/download-ui-validation/$script:dlRun/$DlName")
    Check "b2-download: manifest names this run's object ($DlName, $($Dl.size) bytes, version as uploaded), and the generated file has its SHA-256 (checked here again)" $prepOk
    "  object: sotyware-it/download-ui-validation/$($Dl.run)/<$DlName>, $($Dl.size) bytes, SHA-256 $($Dl.sha256), version $($Dl.versionId)"
  }
  if (-not $prepOk) {
    # Nothing is downloaded; only a version the upload reported is deleted.
    DownloadCleanup
    Remove-Item -Recurse -Force $SrcDir, $Out -EA SilentlyContinue
    ProfileCleanup
    "summary: $script:pass passed, $script:fail failed"; exit 1
  }
}

if ($Mode -eq "b2-upload" -and -not $Rehearse) {
  "test program before the prepare: $(TestExe)"
  $script:upRun = $run
  $prep = CargoIt "cloud::integration::upload::ui::backblaze_b2_upload_ui_prepare" @{ SOTYWARE_IT_B2_UPLOAD_UI_RUN = $run; SOTYWARE_IT_B2_UPLOAD_UI_DIR = $UpSrc }
  $prepExit = $script:cargoExit
  $m = [regex]::Match(($prep -join "`n"), 'upload-ui seed kept; run id ([0-9A-Za-z_-]+) version (\S+)')
  if ($m.Success -and $m.Groups[1].Value -eq $run) { $script:seedVersion = $m.Groups[2].Value }
  $prep | Where-Object { $_ -match '^(provider|run prefix|local |upload-ui|\d+ requests:|  (GET|PUT|HEAD|DELETE|POST) |thread .* panicked|nothing uploaded|seed )' } | ForEach-Object { "  $(Redact $_)" }
  Check "b2-upload: local files checked, run prefix empty (no version, delete marker or multipart upload), seed existing.txt uploaded (version $script:seedVersion)" (($prepExit -eq 0) -and $script:seedVersion)
  if (-not (($prepExit -eq 0) -and $script:seedVersion)) {
    # Nothing goes through the app; only what this run's keys hold is removed.
    UploadUiFinish
    [System.IO.Directory]::Delete($UpSrc, $true)
    ProfileCleanup
    "summary: $script:pass passed, $script:fail failed"; exit 1
  }
}

if ($IsLc -and -not $Rehearse) {
  "test program before the prepare: $(TestExe)"
  $script:upRun = $LcRun
  $prep = CargoIt "cloud::integration::upload::lifecycle::backblaze_b2_upload_lifecycle_prepare" (LcVars)
  $prepExit = $script:cargoExit
  $m = [regex]::Match(($prep -join "`n"), 'upload-lifecycle anchor kept; run id ([0-9A-Za-z_-]+) version (\S+)')
  if ($m.Success -and $m.Groups[1].Value -eq $LcRun) { $script:seedVersion = $m.Groups[2].Value }
  $prep | Where-Object { $_ -match $LcKeep } | ForEach-Object { "  $(Redact $_)" }
  Check "b2-lifecycle: local files checked against the release settings, run prefix empty (no version, delete marker or multipart upload), anchor.txt uploaded (version $script:seedVersion)" (($prepExit -eq 0) -and $script:seedVersion)
  if (-not (($prepExit -eq 0) -and $script:seedVersion)) {
    LifecycleFinish
    [System.IO.Directory]::Delete($UpSrc, $true)
    ProfileCleanup
    "summary: $script:pass passed, $script:fail failed"; exit 1
  }
}

if ($Mode -eq "b2-browse") {
  $ManFile = Join-Path $env:TEMP "sotyware-browse-manifest-$run.json"
  # Not $out: names ignore case, and $Out is the download folder of the
  # download modes (tests/desktop/check-ps-variables.ps1 finds such clashes).
  $dataset = CargoIt "cloud::integration::browse::backblaze_b2_browse_dataset" @{ SOTYWARE_IT_B2_BROWSE_KEEP = "1"; SOTYWARE_IT_B2_BROWSE_MANIFEST = $ManFile }
  $createExit = $script:cargoExit
  $m = [regex]::Match(($dataset -join "`n"), 'dataset kept; run id (\d+-\d+)')
  if ($m.Success) { $script:browseRun = $m.Groups[1].Value }
  $dataset | Where-Object { $_ -match '^(provider|run prefix|permissions|dataset:|stored:|read-only UI key|local server|checks:|manifest:|FAIL)' } | ForEach-Object { "  $(Redact $_)" }
  Check "b2-browse: dataset created and checked through the Rust listing (run $script:browseRun)" (($createExit -eq 0) -and (Test-Path $ManFile))
  if (-not (Test-Path $ManFile)) { BrowseCleanup; "summary: $script:pass passed, $script:fail failed"; exit 1 }
  $Manifest = Get-Content $ManFile -Raw -Encoding UTF8 | ConvertFrom-Json
  $script:mf = @{}; foreach ($f in $Manifest.folders) { $script:mf[[string]$f.path] = $f }
  $fpMid = Fingerprint
}

function FormatSize([long]$n) {
  if ($n -lt 1024) { return "$n B" }
  $v = $n / 1024.0; $u = 0; $units = "KiB", "MiB", "GiB", "TiB", "PiB"
  while ($v -ge 1024 -and $u -lt 4) { $v /= 1024; $u++ }
  $digits = if ($v -lt 10) { 1 } else { 0 }
  "{0} {1}" -f $v.ToString("F$digits", [Globalization.CultureInfo]::InvariantCulture), $units[$u]
}
# The page's formatDate for each RFC 3339 string, by the same Intl call in
# node, in the same time zone. WebView2's default locale is the Windows display
# language, while node's is the regional format (they can differ: en-US vs
# fr-FR seen 2026-11-11), so the display language is passed explicitly.
function FormatDates($dates) {
  $locale = (Get-UICulture).Name
  $js = "const a=JSON.parse(require('fs').readFileSync(0,'utf8'));console.log(JSON.stringify(a.map(s=>new Date(s).toLocaleString(['$locale'],{year:'numeric',month:'short',day:'numeric',hour:'2-digit',minute:'2-digit'}).replace(/[\u202f\u00a0]/g,' '))))"
  $f = Join-Path $env:TEMP "sotyware-dates-$run.js"; Set-Content $f $js -Encoding ASCII
  $out = (ConvertTo-Json @($dates) -Compress) | & node $f
  Remove-Item $f
  $list = $out | ConvertFrom-Json
  $map = @{}; for ($i = 0; $i -lt $dates.Count; $i++) { $map[$dates[$i]] = $list[$i] }
  $map
}
function SameText([string]$t) { (($t -replace '[\u202f\u00a0]', ' ') -replace [string][char]::ConvertFromUtf32(0x1F4C1), ' ' -replace '\s+', ' ').Trim() }
function ExpectedRows($rel, [bool]$firstPage = $false) {
  $f = $script:mf[$rel]; $files = @($f.files); $folders = @($f.folders)
  if ($firstPage) { $folders = @(); $files = @($files | Select-Object -First 1000) }
  $dates = FormatDates @($files | ForEach-Object { $_.lastModified } | Select-Object -Unique)
  $rows = @()
  foreach ($n in $folders) { $rows += , ((SameText $(if ($n -eq "") { "(no name)" } else { $n })) + " | Folder | " + [char]0x2014) }
  foreach ($x in $files) { $rows += , ((SameText $x.name) + " | " + (FormatSize $x.size) + " | " + (SameText $dates[$x.lastModified])) }
  , $rows
}
# The table's body rows as "name | size | date", through the Grid pattern.
function TableRows {
  $d = Dash; if (-not $d) { return , @() }
  $tab = $d.FindFirst($S::Descendants, (PCT $CT::Table)); if (-not $tab) { return , @() }
  $grid = $tab.GetCurrentPattern([System.Windows.Automation.GridPattern]::Pattern)
  $rows = @()
  for ($r = 0; $r -lt $grid.Current.RowCount; $r++) {
    $cells = @()
    for ($c = 0; $c -lt 3; $c++) {
      $cell = $grid.GetItem($r, $c)
      $t = $cell.Current.Name
      if (-not $t) { $t = (@($cell.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ }) | Select-Object -First 1) }
      $cells += , (SameText "$t")
    }
    $row = $cells -join " | "
    if ($row -ne "Name | Size | Last modified") { $rows += , $row }
  }
  , $rows
}
# Waits until folder $rel of the run is shown with every row as listed.
function Shows($rel, [bool]$firstPage = $false) {
  $want = ExpectedRows $rel $firstPage
  $path = $Manifest.prefix + $rel
  $script:lastRows = @()
  $ok = WaitFor {
    $t = PageText
    if (-not $t.Contains("| $path |") -or $t.Contains("Loading")) { return $false }
    if ($want.Count -eq 0) { return $t -like "*This folder is empty.*" }
    $script:lastRows = TableRows
    ($script:lastRows -join "`n") -eq ($want -join "`n")
  } 120
  if (-not $ok) {
    $diff = @(); for ($i = 0; $i -lt [Math]::Max($want.Count, $script:lastRows.Count) -and $diff.Count -lt 4; $i++) { if ($script:lastRows[$i] -ne $want[$i]) { $diff += "#${i}: shown [$($script:lastRows[$i])] expected [$($want[$i])]" } }
    $script:why = "$path : $($script:lastRows.Count) rows shown, $($want.Count) expected; $($diff -join '; ')"
  } else { $script:why = "" }
  $ok
}
function Go($folder, $rel, [bool]$firstPage = $false) { (PressFolder $folder) -and (Shows $rel $firstPage) }

# ---- the native save dialog (download mode) ----
function DialogNamed($title) {
  $p = [uint32[]]@((Procs).Id); if ($p.Count -eq 0) { return $null }
  foreach ($h in [CloudUiW]::Find("#32770", $p)) { $e = $A::FromHandle($h); if ($e.Current.Name -eq $title) { return $e } }
  $null
}
function PCA($id) { [System.Windows.Automation.PropertyCondition]::new($A::AutomationIdProperty, $id) }
# The save dialog's File name box (common item dialog: Edit 1001, found as a
# child window by its control id).
function Hwnd($el) { [IntPtr][int64]$el.Current.NativeWindowHandle }
# The common item dialog's File name box (Edit, control id 1001) and its
# Save (1) and Cancel (2) buttons are classic child windows; its UI
# Automation tree does not expose them (Windows 11 25H2), so they are found
# by control id and driven with WM_GETTEXT / WM_SETTEXT / BM_CLICK (UI
# Automation "no keystrokes, no mouse" still holds).
function NameBox($dlg) { $h = [CloudUiW]::ChildById((Hwnd $dlg), 1001, "Edit"); if ($h -eq [IntPtr]::Zero) { $null } else { $h } }
function DialogButton($dlg, [int]$id) { $h = [CloudUiW]::ChildById((Hwnd $dlg), $id, "Button"); if ($h -eq [IntPtr]::Zero) { $null } else { $h } }
# Presses the file's Download action and waits for the save dialog and its
# File name box (the window exists a moment before its controls do).
function OpenSave($file) {
  if (-not (Press "Download $file")) { return $null }
  $script:dlg = $null
  $ready = WaitFor { $script:dlg = DialogNamed "Save download as"; ($null -ne $script:dlg) -and ($null -ne (NameBox $script:dlg)) } 20
  if ($script:dlg -and -not $ready) { Write-Host "  save dialog without a File name box" }
  $script:dlg
}
function CancelDialog($dlg) { $b = DialogButton $dlg 2; if ($b) { [void][CloudUiW]::Click($b) } }
function Suggested($dlg) { $b = NameBox $dlg; if (-not $b) { return "" }; [CloudUiW]::GetText($b) }
# Types $path into File name and presses Save.
function SaveAs($dlg, $path) {
  $b = NameBox $dlg; if (-not $b) { Write-Host "  no File name box"; return $false }
  [CloudUiW]::SetText($b, $path)
  if (-not (WaitFor { [CloudUiW]::GetText($b) -eq $path } 5)) { return $false }
  $save = DialogButton $dlg 1; if (-not $save) { return $false }
  [CloudUiW]::Click($save)
}
function Saved { @(Get-ChildItem -Force $Out | ForEach-Object { $_.Name } | Sort-Object) }
function Says([string[]]$parts, [double]$timeout = 60) { WaitFor { $t = PageText; foreach ($x in $parts) { if (-not $t.Contains($x)) { return $false } }; $true } $timeout }

# ---- upload mode: local files, the server's state, the panel, the dialog ----
$MiB = 1048576
function UpSha([string]$path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant() }
function Utf8([string]$s) { , [System.Text.Encoding]::UTF8.GetBytes($s) }
function UpFolder([string]$batch) { $f = Join-Path $UpSrc $batch; if (-not (Test-Path -LiteralPath $f)) { New-Item -ItemType Directory $f | Out-Null }; $f }
# Writes a local test file through .NET, so any name NTFS allows works
# (also an unpaired surrogate, which PowerShell's own cmdlets may not pass).
function UpFile([string]$batch, [string]$name, [byte[]]$data) {
  $f = Join-Path (UpFolder $batch) $name; [System.IO.File]::WriteAllBytes($f, $data); $f
}
function UpBlob([string]$batch, [string]$name, [long]$size, [int]$seed) {
  $f = Join-Path (UpFolder $batch) $name; [CloudUiW]::WriteBlob($f, $size, $seed); $f
}
# Ordinal (case-sensitive) string sets: object keys and file names.
function NewSet { New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal) }
$script:created = NewSet  # every key an upload of this mode may have created
$script:mpSeen = NewSet   # every multipart file s3s-fs showed during the run
# The helper's control file (serve_uploads_for_ui): commands, one per line,
# UTF-8 without BOM, written under another name and renamed into place.
function Ctl([string[]]$cmds) {
  $tmp = "$SrvFile.ctl.tmp"
  [System.IO.File]::WriteAllText($tmp, (($cmds -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding $false))
  [System.IO.File]::Move($tmp, "$SrvFile.ctl")
  WaitFor { -not (Test-Path -LiteralPath "$SrvFile.ctl") } 30
}
# The server's state: Objects (key -> object, case-sensitive), Multipart, Ops.
function State {
  $f = "$SrvFile.state.json"
  if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f }
  if (-not (Ctl @("dump")) -or -not (WaitFor { Test-Path -LiteralPath $f } 10)) { throw "the upload server did not answer dump" }
  $j = [System.IO.File]::ReadAllText($f, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
  Remove-Item -LiteralPath $f
  $objs = New-Object 'System.Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
  foreach ($o in @($j.objects)) { if ($o) { $objs[[string]$o.key] = $o } }
  $mp = @($j.multipart | Where-Object { $_ }); foreach ($x in $mp) { [void]$script:mpSeen.Add([string]$x) }
  [pscustomobject]@{ Objects = $objs; Multipart = $mp; Ops = @($j.ops) }
}
function Obj($st, [string]$key) { if ($st.Objects.ContainsKey($key)) { $st.Objects[$key] } else { $null } }
function KeysUnder($st, [string]$prefix) { @($st.Objects.Keys | Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) } | Sort-Object) -join "|" }
function SortedKeys($list) { @($list | Sort-Object) -join "|" }
# The object holds exactly the local file's bytes, HEAD says so, and its
# SHA-256 metadata is the one sotyware writes. Sizes compared as text, so a
# missing HEAD size never equals 0.
function StoredOk($o, [string]$path) {
  if ($null -eq $o) { return $false }
  $len = "$((Get-Item -LiteralPath $path).Length)"; $h = UpSha $path
  ("$($o.size)" -eq $len) -and ($o.sha256 -eq $h) -and ("$($o.headSize)" -eq $len) -and ($o.sha256Metadata -eq $h)
}
# Upload requests the server saw so far (PutObject, multipart).
function UploadOps($st) { @($st.Ops | Where-Object { $_ -match '^(PutObject|CreateMultipartUpload|UploadPart|CompleteMultipartUpload)' }).Count }


# ---- files mode (ADR-026): the file manager against serve_files_for_ui ----
# The server's state (`dump`): per bucket, key -> { current, versions }.
function FState([string]$bucket = "files-versioned") {
  $f = "$SrvFile.state.json"
  if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f }
  if (-not (Ctl @("dump")) -or -not (WaitFor { Test-Path -LiteralPath $f } 10)) { throw "the files server did not answer dump" }
  $j = [System.IO.File]::ReadAllText($f, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
  Remove-Item -LiteralPath $f
  $keys = New-Object 'System.Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
  foreach ($p in $j.buckets.$bucket.keys.PSObject.Properties) { $keys[$p.Name] = $p.Value }
  $keys
}
function FCurrent($keys, [string]$prefix = "") { @($keys.Keys | Where-Object { $_.StartsWith($prefix, [StringComparison]::Ordinal) -and $null -ne $keys[$_].current } | Sort-Object) -join "|" }
function FText($keys, [string]$k) { if ($keys.ContainsKey($k) -and $keys[$k].current) { [string]$keys[$k].current.text } else { $null } }
function FVersions($keys, [string]$k) { if ($keys.ContainsKey($k)) { @($keys[$k].versions) } else { @() } }
# The checkbox "Select <name>", toggled through the Toggle pattern.
function Sel([string]$name) {
  $script:box = $null
  WaitFor { $script:box = Named "Select $name" $CT::CheckBox; $null -ne $script:box } 10 | Out-Null
  if (-not $script:box) { Write-Host "  no checkbox 'Select $name'"; return $false }
  try { $script:box.GetCurrentPattern($Toggle).Toggle(); $true } catch { $false }
}
function SelectedCount { $m = [regex]::Match((PageText), '(\d+) selected'); if ($m.Success) { [int]$m.Groups[1].Value } else { 0 } }
# The open modal whose title satisfies $pred (the element, not its heading).
function ModalWhere([scriptblock]$pred) {
  $d = Dash; if (-not $d) { return $null }
  foreach ($e in @($d.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition))) {
    $n = $e.Current.Name
    if ($n -and $e.Current.ControlType -ne $CT::Text -and $e.Current.ControlType -ne $CT::Button -and (& $pred $n)) {
      if ($e.FindFirst($S::Descendants, (PCT $CT::Button))) { return $e }
    }
  }
  $null
}
# A button in $el by its name without surrounding spaces or the folder icon
# (UI Automation includes the aria-hidden icon in the name).
function PressFolderIn($el, [string]$name) {
  if (-not $el) { return $false }
  $icon = [string][char]::ConvertFromUtf32(0x1F4C1)
  $b = @($el.FindAll($S::Descendants, (PCT $CT::Button)) | Where-Object { $_.Current.Name.Replace($icon, "").Trim() -eq $name })[0]
  if (-not $b) { return $false }
  try { $b.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false }
}
# Removes the account card labelled $label (its Remove, then the confirmation).
function RemoveCard([string]$label) {
  $d = Dash; if (-not $d) { return $false }
  $item = @($d.FindAll($S::Descendants, (PCT $CT::ListItem)) | Where-Object { $_.Current.Name.StartsWith($label) -and $null -ne (InEl $_ "Remove" $CT::Button) })[0]
  if (-not $item) { Write-Host "  no account card '$label'"; return $false }
  if (-not (PressIn $item "Remove")) { return $false }
  $script:conf = $null
  WaitFor { $script:conf = @((Dash).FindAll($S::Descendants, (PCN "Confirm removal of $label")))[0]; $null -ne $script:conf } 10 | Out-Null
  (PressIn $script:conf "Remove account") -and (Says @("Removed $label") 30)
}
function InEl($el, [string]$name, $type) { if (-not $el) { return $null }; $el.FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $type), (PCN $name))) }
function PressIn($el, [string]$name) { $b = InEl $el $name $CT::Button; if (-not $b -or -not $b.Current.IsEnabled) { return $false }; try { $b.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
function OpRegion { $d = Dash; if (-not $d) { return $null }; @($d.FindAll($S::Descendants, (PCN "File operation")) | Where-Object { $_.Current.ControlType -ne $CT::Text })[0] }
function OpText { $r = OpRegion; if ($r) { AllNames $r } else { "" } }
function OpFinished { $r = OpRegion; ($null -ne $r) -and ($null -ne (InEl $r "Dismiss" $CT::Button)) }
# Waits for the review ("<Verb> ...?"), runs $before on it, presses $verb in
# it and waits for the operation's final state. Returns review + panel text.
function Reviewed([string]$verb, [scriptblock]$before = $null, [double]$timeout = 60) {
  $script:rv = $null
  WaitFor { $script:rv = ModalWhere { param($n) $n.StartsWith($verb) -and $n.TrimEnd().EndsWith("?") }; $null -ne $script:rv } 30 | Out-Null
  if (-not $script:rv) { Write-Host "  no review for ${verb}: $((OpText).Substring(0, [Math]::Min(200, (OpText).Length)))"; return $null }
  $review = AllNames $script:rv
  if ($before) { & $before $script:rv }
  if (-not (PressIn $script:rv $verb)) { Write-Host "  $verb could not be pressed"; return $null }
  if (-not (WaitFor { (OpFinished) -and $null -eq (ModalWhere { param($n) $n.StartsWith($verb) -and $n.TrimEnd().EndsWith("?") }) } $timeout)) { Write-Host "  $verb did not finish: $(OpText)"; return $null }
  "$review || $(OpText)"
}
function RetryOp { $before = OpText; if (-not (PressIn (OpRegion) "Try again")) { return $null }; WaitFor { (OpText) -ne $before -or -not (OpFinished) } 10 | Out-Null; if (WaitFor { OpFinished } 60) { OpText } else { $null } }
function DismissOp { $r = OpRegion; if ($r) { $null = PressIn $r "Dismiss"; WaitFor { $null -eq (OpRegion) } 5 | Out-Null } }
function AnyModal { ModalWhere { param($n) $true } }
# Closes what a failed step left open, so the next step starts clean.
function Recover { for ($i = 0; $i -lt 3; $i++) { $m = ModalWhere { param($n) $n.StartsWith("Rename") -or $n.StartsWith("Copy") -or $n.StartsWith("Move") -or $n.StartsWith("Delete") -or $n.StartsWith("Restore") -or $n.StartsWith("Properties") -or $n.StartsWith("New folder") }; if (-not $m) { break }; if (-not ((PressIn $m "Cancel") -or (PressIn $m "Close"))) { break }; Start-Sleep -Milliseconds 400 }; DismissOp }
function FRoot { if (-not ((PageText) -like "*Root of the bucket*")) { $null = PressFolder $Bucket }; WaitFor { $t = PageText; $t.Contains("Root of the bucket") -and -not $t.Contains("Loading") } 20 }
function FGo([string]$folder) {
  if (-not (FRoot)) { return $false }
  $path = ""
  foreach ($seg in $folder.Trim("/").Split("/")) { $path += "$seg/"; $p = $path; if (-not ((PressFolder $seg) -and (WaitFor { $t = PageText; $t.Contains($p) -and -not $t.Contains("Loading") } 20))) { return $false } }
  $true
}
# In the destination picker: from its root, open each folder of $folder, then press $confirm.
function Pick2([string]$folder, [string]$confirm) {
  $script:pk = $null
  WaitFor { $script:pk = ModalWhere { param($n) $n -match ' to\u2026$' -or $n -match ' to\.\.\.$' }; $null -ne $script:pk } 10 | Out-Null
  if (-not $script:pk) { Write-Host "  no destination picker"; return $false }
  $null = PressIn $script:pk $Bucket; Start-Sleep -Milliseconds 600
  $last = ""
  if ($folder) {
    foreach ($seg in $folder.Trim("/").Split("/")) {
      $s2 = $seg
      if (-not (WaitFor { PressFolderIn $script:pk $s2 } 10)) { Write-Host "  picker: no folder '$seg'"; return $false }
      $last = $seg; Start-Sleep -Milliseconds 800
    }
  }
  # The confirm button names the folder shown: "Move to <quote>made<quote>".
  $want = if ($last) { "$confirm $([char]0x201C)$last$([char]0x201D)" } else { $confirm }
  $script:cb = $null
  WaitFor { $script:cb = @($script:pk.FindAll($S::Descendants, (PCT $CT::Button)) | Where-Object { $_.Current.Name.StartsWith($want) -and $_.Current.IsEnabled })[0]; $null -ne $script:cb } 10 | Out-Null
  if (-not $script:cb) { Write-Host "  picker: no button '$want'"; return $false }
  try { $script:cb.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false }
}
# A button with aria-pressed: Invoke if offered, else the Toggle pattern.
function PressToggle([string]$name) {
  $script:tb = $null; WaitFor { $script:tb = Named $name $CT::Button; $null -ne $script:tb } 10 | Out-Null
  if (-not $script:tb) { return $false }
  try { $script:tb.GetCurrentPattern($Invoke).Invoke(); return $true } catch { }
  try { $script:tb.GetCurrentPattern($Toggle).Toggle(); $true } catch { $false }
}
# Runs the steps in order; on the first that fails says which.
function Chain([string]$label, [scriptblock[]]$steps) {
  $i = 0
  foreach ($st in $steps) { $i++; if (-not (& $st)) { Write-Host "  ${label}: step $i of $($steps.Count) failed"; return $false } }
  $true
}
function KeepBoth($dlg) { $r = @($dlg.FindAll($S::Descendants, (PCT $CT::RadioButton)) | Where-Object { $_.Current.Name.StartsWith("Keep both") })[0]; if ($r) { try { $r.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select() } catch { } }; Start-Sleep -Milliseconds 300 }
function FFill([string]$label, [string]$value) {
  $d = Dash; if (-not $d) { return $false }
  $e = @($d.FindAll($S::Descendants, (PCT $CT::Edit)) | Where-Object { $n = $_.Current.Name; $n -eq $label -or $n.StartsWith("$label ") })[0]
  if (-not $e) { Write-Host "  no field '$label'"; return $false }
  try { $e.GetCurrentPattern($ValuePattern).SetValue($value) } catch { return $false }
  WaitFor { $e.GetCurrentPattern($ValuePattern).Current.Value -eq $value } 5
}

# The panel (CloudUpload.svelte, a region named "Upload") and its file rows.
function AllNames($el) { @($el.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ }) -join " | " }
# Searching WebView2's whole tree takes long (a 40 MiB PUT at 6 MiB/s ended
# before one FindAll-based poll), so the region is found with one FindFirst
# and everything else is read inside it.
$UpRegionCond = [System.Windows.Automation.AndCondition]::new((PCN "Upload"),
  [System.Windows.Automation.NotCondition]::new((PCT $CT::Text)), [System.Windows.Automation.NotCondition]::new((PCT $CT::Button)))
$UpListCond = [System.Windows.Automation.AndCondition]::new((PCN "Files in this upload"), [System.Windows.Automation.NotCondition]::new((PCT $CT::ListItem)))
function UpRegion { $d = Dash; if (-not $d) { return $null }; try { $d.FindFirst($S::Descendants, $UpRegionCond) } catch { $null } }
function UpPanel { $r = UpRegion; if (-not $r) { return "" }; try { AllNames $r } catch { "" } }
# One text per file row: WebView2 names a list item after all its text
# ("a.txt12 BUploaded and verified...").
function UpRows {
  $r = UpRegion; if (-not $r) { return , @() }
  try {
    $list = $r.FindFirst($S::Descendants, $UpListCond)
    if (-not $list) { return , @() }
    $res = @(); foreach ($it in @($list.FindAll($S::Children, (PCT $CT::ListItem)))) { $res += , [string]$it.Current.Name }
    , $res
  } catch { , @() }
}
function UpRow([string]$name) { foreach ($x in (UpRows)) { if ($x.StartsWith($name, [StringComparison]::Ordinal)) { return $x } }; "" }
$script:seen = New-Object System.Collections.Generic.List[string]
# Waits for $cond (a block taking the panel text), keeping every panel text
# seen meanwhile; the text that matched, or $null. (Not named $pred: WaitFor's
# parameter of that name would be seen inside the block instead.)
$script:polls = 0; $script:pollTime = [Diagnostics.Stopwatch]::new()
function Until([scriptblock]$cond, [double]$timeout = 60) {
  $script:untilCond = $cond; $script:untilText = $null
  $ok = WaitFor {
    $script:pollTime.Start(); $t = UpPanel; $script:pollTime.Stop(); $script:polls++
    if ($script:seen.Count -eq 0 -or $script:seen[$script:seen.Count - 1] -ne $t) { $script:seen.Add($t) }
    $script:untilText = $t
    [bool](& $script:untilCond $t)
  } $timeout
  if (-not $ok) {
    # Diagnosis: the last panel text, non-ASCII characters as <hex>.
    $e = -join ("$script:untilText".ToCharArray() | ForEach-Object { if ([int]$_ -ge 32 -and [int]$_ -lt 127) { $_ } else { "<{0:X}>" -f [int]$_ } })
    Write-Host "  (wait timed out after $timeout s; last panel text: $($e.Substring([Math]::Max(0, $e.Length - 400))))"
    $sx = @($script:seen | Where-Object { $_.Contains("Sending") }); if ($sx.Count -gt 0) {
      $e = -join ($sx[$sx.Count - 1].ToCharArray() | ForEach-Object { if ([int]$_ -ge 32 -and [int]$_ -lt 127) { $_ } else { "<{0:X}>" -f [int]$_ } })
      Write-Host "  (last text with Sending: $e)"
    }
  }
  if ($ok) { $script:untilText } else { $null }
}
function Finished([string]$t) { $t.Contains("Upload finished.") -or $t.Contains("Upload cancelled.") -or $t.Contains("Upload stopped after an error.") }
# "Retrying upload" is two texts to UI Automation ("Retrying upload", " to ").
function Retried([string]$t) { $t.Contains("Retrying upload") -and (Finished $t) }
# Whether panel text $t shows file $name's row as "Sending <U+00B7> n%" with
# n >= $least (the row is one text, the panel's texts are joined by " | ").
function SendingIn([string]$t, [string]$name, [int]$least = 0) {
  $m = [regex]::Match($t, '(?:^| \| )' + [regex]::Escape($name) + '[^|]*?Sending \u00b7 (\d+)%')
  $m.Success -and [int]$m.Groups[1].Value -ge $least
}
function Btn([string]$name) { $null -ne (Named $name $CT::Button) }
function UpDismiss { $null = Press "Dismiss"; WaitFor { (UpPanel) -eq "" } 5 | Out-Null }
$UploadButton = "Upload files$([char]0x2026)"
function UploadEnabled { $b = Named $UploadButton $CT::Button; ($null -ne $b) -and $b.Current.IsEnabled }
# Opens folder $folder of the bucket's root through its folder button.
function UpGo([string]$folder, [string]$marker) {
  if (-not (PageText).Contains("Root of the bucket")) { $null = PressFolder $Bucket }
  (WaitFor { (PageText).Contains("Root of the bucket") } 20) -and (PressFolder $folder) -and (WaitFor { (PageText).Contains($marker) } 20)
}

# The native Windows Open dialog (common item dialog, as rfd shows it): its
# File name box (Edit, control id 1148), Open (1) and Cancel (2) are classic
# child windows, the address bar's text (ToolbarWindow32 1001) says which
# folder it shows, and its file list is in the UI Automation tree
# ("Items View", one UIItem per file, SelectionItem pattern).
$OpenTitle = "Choose files to upload"
function OpenNameBox($dlg) { $h = [CloudUiW]::ChildById((Hwnd $dlg), 1148, "Edit"); if ($h -eq [IntPtr]::Zero) { $null } else { $h } }
function DialogFolder($dlg) { $h = [CloudUiW]::ChildById((Hwnd $dlg), 1001, "ToolbarWindow32"); if ($h -eq [IntPtr]::Zero) { "" } else { [CloudUiW]::GetText($h) } }
function DialogItems($dlg) { @($dlg.FindAll($S::Descendants, [System.Windows.Automation.PropertyCondition]::new($A::ClassNameProperty, "UIItem"))) }
$SelItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
# Presses "Upload files...", opens $folder in the dialog, chooses files and
# presses Open. $how: "list" selects $names in the file list (UI Automation:
# Select, then AddToSelection), "all" selects every item of the folder (there
# must be $names.Count), "names" types the quoted names into File name.
# $before runs while the dialog is open, just before Open.
function Pick([string]$folder, [string[]]$names, [string]$how = "names", [scriptblock]$before = $null) {
  if (-not (Press $UploadButton)) { Write-Host "  no '$UploadButton' button"; return $false }
  $script:odlg = $null
  if (-not (WaitFor { $script:odlg = DialogNamed $OpenTitle; ($null -ne $script:odlg) -and ($null -ne (OpenNameBox $script:odlg)) } 20)) { Write-Host "  no file dialog"; return $false }
  $dlg = $script:odlg; $box = OpenNameBox $dlg; $open = DialogButton $dlg 1
  [CloudUiW]::SetText($box, $folder); [void][CloudUiW]::Click($open)
  if (-not (WaitFor { (DialogFolder $script:odlg) -eq "Address: $folder" } 10)) { Write-Host "  the dialog shows '$(DialogFolder $dlg)'"; CancelDialog $dlg; return $false }
  $want = NewSet; foreach ($n in $names) { [void]$want.Add($n) }
  $script:listed = @()
  $ok = WaitFor {
    $script:listed = @(DialogItems $script:odlg)
    if ($how -eq "all") { return $script:listed.Count -eq $names.Count }
    $have = NewSet; foreach ($i in $script:listed) { [void]$have.Add($i.Current.Name) }
    $want.IsSubsetOf($have)
  } 10
  if (-not $ok) { Write-Host "  the dialog lists: $(@($script:listed | ForEach-Object { $_.Current.Name }) -join ' | ')"; CancelDialog $dlg; return $false }
  if ($how -eq "names") {
    [CloudUiW]::SetText($box, (@($names | ForEach-Object { '"' + $_ + '"' }) -join " "))
  } else {
    $first = $true
    foreach ($i in $script:listed) {
      if ($how -eq "all" -or $want.Contains($i.Current.Name)) {
        $sp = $i.GetCurrentPattern($SelItem)
        if ($first) { $sp.Select(); $first = $false } else { $sp.AddToSelection() }
      }
    }
    # The dialog writes the selection into File name, each name quoted.
    $quotes = 2 * $names.Count
    if ($names.Count -gt 1 -and -not (WaitFor { ([CloudUiW]::GetText($box).ToCharArray() | Where-Object { $_ -eq '"' }).Count -eq $quotes } 5)) {
      Write-Host "  File name after the selection: '$([CloudUiW]::GetText($box))'"; CancelDialog $dlg; return $false
    }
  }
  # $before may veto Open by returning $false: the dialog is cancelled.
  if ($before) { $r = @(& $before); if ($r.Count -eq 1 -and $r[0] -is [bool] -and -not $r[0]) { Write-Host "  not opened: cancelled before Open"; CancelDialog $dlg; return $false } }
  [void][CloudUiW]::Click($open)
  WaitFor { $null -eq (DialogNamed $OpenTitle) } 15
}

# The tray (as in windows.ps1): the icon sits in the "Show Hidden Icons"
# flyout on Windows 11; its context menu is not in the UI Automation tree,
# so it is opened with a real right click and read through MN_GETHMENU.
$root = $A::RootElement
$OverflowClass = "TopLevelWindowForOverflowXamlIsland"
function OverflowShown { $o = $root.FindFirst($S::Children, [System.Windows.Automation.PropertyCondition]::new($A::ClassNameProperty, $OverflowClass)); ($null -ne $o) -and -not $o.Current.BoundingRectangle.IsEmpty }
function OverflowOpen { (OverflowShown) -and [CloudUiW]::ClassOf([CloudUiW]::GetForegroundWindow()) -eq $OverflowClass }
# The test-profile build's tooltip, never an installed sotyware's ("sotyware").
function TrayIcons {
  @($root.FindAll($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCA "NotifyItemIcon"), (PCN "sotyware test profile"))) |
    Where-Object { $r = $_.Current.BoundingRectangle; -not $r.IsEmpty -and -not [double]::IsNaN($r.X) })
}
function OpenOverflow {
  if (OverflowOpen) { return }
  if (OverflowShown) { WaitFor { -not (OverflowShown) } 3 | Out-Null }
  if ((TrayIcons).Count -ge 1) { return }
  $chev = $root.FindFirst($S::Descendants, (PCN "Show Hidden Icons"))
  if ($chev) { $chev.GetCurrentPattern($Invoke).Invoke(); WaitFor { OverflowOpen } 3 | Out-Null; Start-Sleep -Milliseconds 400 }
}
# Right-clicks the tray icon and clicks "Quit" in its menu, both with the
# real mouse. $true once the click on Quit was sent.
function TrayQuit {
  $script:icon = $null
  WaitFor { OpenOverflow; $script:icon = @(TrayIcons)[0]; $null -ne $script:icon } 8 | Out-Null
  if (-not $script:icon) { $script:why = "tray icon not found"; return $false }
  $r = $script:icon.Current.BoundingRectangle
  [CloudUiW]::MouseClick([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), $true)
  $script:menuWnd = [IntPtr]::Zero
  WaitFor { $mw = @([CloudUiW]::Find("#32768", [uint32[]]@((Procs).Id))); if ($mw.Count -ge 1) { $script:menuWnd = $mw[0] }; $script:menuWnd -ne [IntPtr]::Zero } 4 | Out-Null
  if ($script:menuWnd -eq [IntPtr]::Zero) { $script:why = "context menu did not open"; return $false }
  $hm = [CloudUiW]::MenuOf($script:menuWnd)
  $idx = [array]::IndexOf([CloudUiW]::MenuStrings($hm), "Quit")
  if ($idx -lt 0) { $script:why = "no Quit in the menu: $([CloudUiW]::MenuStrings($hm) -join '|')"; [CloudUiW]::Esc(); return $false }
  $rc = New-Object CloudUiW+RECT
  if (-not [CloudUiW]::GetMenuItemRect([IntPtr]::Zero, $hm, [uint32]$idx, [ref]$rc)) { $script:why = "GetMenuItemRect failed"; [CloudUiW]::Esc(); return $false }
  [CloudUiW]::MouseClick([int](($rc.L + $rc.R) / 2), [int](($rc.T + $rc.B) / 2), $false)
  $true
}
function DashVisible { $d = Dash; ($null -ne $d) -and [CloudUiW]::IsWindowVisible((Hwnd $d)) }
# Starts the app and opens the bucket's root in the browser.
# Refuses while one still runs: that start would only hand over and exit.
function LaunchToBucket {
  $script:app = $null
  if ((Procs).Count -gt 0) { Write-Host "  sotyware still runs; not started again"; return $false }
  $script:app = Start-Process $Exe -PassThru; $null = $script:app.Handle
  Verified $script:app "launch" | Out-Host
  (WaitFor { $null -ne (Dash) } 20) -and (WaitFor { Link "Cloud Storage" } 15) -and (WaitFor { Btn "Open bucket" } 15) -and
    (Press "Open bucket") -and (WaitFor { $t = PageText; $t.Contains("Root of the bucket") -and $t.Contains("inbox") } 30)
}
# Deletes exactly the keys this mode may have created and the seed (never an
# unknown key: those are reported), then checks the bucket is empty.
function UpCleanup {
  $st = State
  $known = NewSet; foreach ($k in $script:created) { [void]$known.Add($k) }; foreach ($k in $script:objects.Keys) { [void]$known.Add($k) }
  $unknown = @($st.Objects.Keys | Where-Object { -not $known.Contains($_) })
  Check "upload: the server holds no object this run does not know about" ($unknown.Count -eq 0) ($unknown -join "|")
  "  keys created by uploads and still stored: $(@($st.Objects.Keys | Where-Object { $script:created.Contains($_) } | Sort-Object) -join ' | ')"
  "  multipart uploads seen during the run (s3s-fs files): $(@($script:mpSeen) -join ' | ')"
  $del = @($st.Objects.Keys | Where-Object { $known.Contains($_) } | Sort-Object | ForEach-Object { "delete $_" })
  if ($del.Count -gt 0) { $null = Ctl $del }
  $st = State
  Check "upload cleanup by exact key: the bucket is empty, no multipart upload left" ($st.Objects.Count -eq 0 -and $st.Multipart.Count -eq 0) "$(SortedKeys $st.Objects.Keys) $($st.Multipart -join ',')"
}

$label = "sotyware e2e $run"; $id = "sotyware-e2e-$run"; $renamed = "$label renamed"
try {
  $p = Start-Process $Exe -PassThru; $null = $p.Handle  # without a cached handle, ExitCode stays empty
  Verified $p "startup" | Out-Host
  $up = WaitFor { $null -ne (Dash) } 20
  Check "startup: dashboard window" $up
  # Every later step presses the first matching button on the page; without
  # the window nothing below may run (2026-11-14: a run that went on acted
  # on another account).
  if (-not $up) { throw "ABORT: no dashboard window; nothing else is run" }
  if ($Mode -eq "upload" -or $IsB2Up) {
    $wpid = [uint32]0; [void][CloudUiW]::GetWindowThreadProcessId((Hwnd (Dash)), [ref]$wpid)
    Check "the dashboard driven is the window of the started release binary (pid $($p.Id), $Exe)" ($wpid -eq $p.Id -and (Procs).Count -eq 1)
    $listen = @(Get-NetTCPConnection -State Listen -LocalPort 1420 -EA SilentlyContinue)
    Check "the page is the bundled frontend: nothing listens on the dev server port 1420" ($listen.Count -eq 0) "$($listen.Count) listener(s)"
  }
  $empty = (WaitFor { Link "Cloud Storage" } 15) -and (WaitFor { (PageText) -like "*No storage accounts yet*" } 10)
  Check "Cloud Storage link opens the empty page" $empty (PageText)
  # Edit, Remove and Open bucket are found by name: with any other account on
  # the page they would act on it (2026-11-14 the user's own account was
  # opened at Backblaze, edited and removed this way). Stop instead.
  if (-not $empty) { throw "ABORT: the Cloud Storage page is not empty (another account is visible); nothing else is run" }
  Check "no entry for this run's id in Credential Manager yet" ([CloudUiW]::Matches((Target $id), "") -eq -1)

  $ok = (Press "Add account") -and (WaitFor { (Fields) -contains "Secret access key" } 10)
  $ok = $ok -and (Fill "Name" $label) -and (Fill "Bucket" $Bucket) -and (Fill "Region" $Region)
  if ($Endpoint) { $ok = $ok -and (Fill "Custom endpoint (optional)" $Endpoint) }
  $ok = $ok -and (Fill "Access key ID" $Key1) -and (Fill "Secret access key" $Secret1) -and (Press "Add account")
  Check "add a Backblaze B2 account through the form" ($ok -and (WaitFor { (PageText) -like "*Added $label*" } 30)) (PageText)
  Check "Credential Manager has $(Target $id), holding exactly the typed key" ([CloudUiW]::Matches((Target $id), (Stored $Key1 $Secret1)) -eq 1)
  Check "cmdkey lists exactly one entry for it (cmdkey never shows the secret)" ((CmdkeyEntries (Target $id)) -eq 1)
  $t = PageText
  Check "page shows the account and bucket, not the key; no input field left" (($t -like "*$Bucket*") -and ($t -notlike "*$Secret1*") -and ($t -notlike "*$Key1*") -and (Fields).Count -eq 0) $t
  # sotyware keeps the database open: read it with sharing allowed.
  $fs = [System.IO.File]::Open($CloudDb, "Open", "Read", "ReadWrite")
  try { $raw = New-Object byte[] $fs.Length; [void]$fs.Read($raw, 0, $raw.Length) } finally { $fs.Close() }
  $rawText = [System.Text.Encoding]::UTF8.GetString($raw)
  Check "account database has no key id or secret" (($rawText -notlike "*$Secret1*") -and ($rawText -notlike "*$Key1*"))

  $expect = if ($Mode -eq "local") { "Network error" } else { "Connected" }
  $st = RunCheck
  Check "Check connection: $expect" ($st -eq $expect) "$st | $(PageText)"

  # Open the bucket (read-only). local: nobody listens, so the browser shows
  # the network error; b2: the test key may list only under sotyware-it/, so
  # Backblaze may refuse the root.
  $ok = (Press "Open bucket") -and (WaitFor { $t = PageText; ($t -like "*Could not load this folder.*") -or (($t -like "*Root of the bucket*") -and ($t -notlike "*Loading folder*")) } 60)
  $t = PageText
  if ($Mode -eq "local") {
    Check "Open bucket: the network error with Try again" ($ok -and ($t -like "*The provider could not be reached*") -and $null -ne (Named "Try again" $CT::Button)) $t
  } elseif ($Mode -eq "b2-browse") {
    $R = $Manifest.run
    Check "Open bucket lists the bucket's root, with the sotyware-it folder" ($ok -and ((TableRows) -contains "sotyware-it | Folder | $([char]0x2014)")) (PageText)
    $ok = (PressFolder "sotyware-it") -and (WaitFor { (PageText) -like "*browse-validation*" } 60)
    $ok = $ok -and (PressFolder "browse-validation") -and (WaitFor { (PageText) -like "*$R*" } 60)
    Check "navigate sotyware-it, browse-validation, this run's folder" ($ok -and (PressFolder $R))
    Check "run root: exactly its immediate folders and files, names, sizes and dates as listed" (Shows "") $script:why
    $rel = ""
    foreach ($f in "docs", "2024", "q1", "deep", "er") { $rel += "$f/"; Check "open ${rel}: only its own entries" (Go $f $rel) $script:why }
    Check "breadcrumb 2024 opens docs/2024/" (Go "2024" "docs/2024/") $script:why
    Check "Up one folder opens docs/" ((Press "Up one folder") -and (Shows "docs/")) $script:why
    Check "docs/empty-nested/ (marker only) opens as empty" (Go "empty-nested" "docs/empty-nested/") $script:why
    foreach ($pair in @(@("marker-only", "marker-only/: empty"), @("photos", "photos/: the marker is not a file"), @("special", "special/: + & ? # % exactly"))) {
      $ok = Go $R ""
      Check "open $($pair[1])" ($ok -and (Go $pair[0] "$($pair[0])/")) $script:why
    }
    Check "open special/f+o&l?d#e%r/" (Go "f+o&l?d#e%r" "special/f+o&l?d#e%r/") $script:why
    $uni = "$([char]0xFC)n$([char]0xEF)c$([char]0xF8)d$([char]0xE9)"
    $priv = -join ([char[]](0x43F, 0x440, 0x438, 0x432, 0x435, 0x442))
    $ok = (Go $R "") -and (Go $uni "$uni/") -and (Go $priv "$uni/$priv/")
    Check "Unicode folders open and show their files exactly" $ok $script:why
    $ok = (Go $R "") -and (Go "dots" "dots/") -and (Go ".." "dots/../")
    Check "dots/ shows '..' and '.' as names; '..' opens dots/../ (escape.txt), not the run root" ($ok -and ((PageText) -like "*escape.txt*")) $script:why
    Check "Up one folder from dots/../ goes to dots/" ((Press "Up one folder") -and (Shows "dots/")) $script:why
    $ok = (Go $R "") -and (Go "double" "double/") -and (Go "(no name)" "double//") -and (Go "(no name)" "double///")
    Check "double/, (no name), (no name): repeated separators kept" $ok $script:why
    $ok = (Go $R "") -and (Go "many" "many/" $true)
    Check "many/: the first 1,000 entries, with Load more" ($ok -and ((PageText) -like "*1000 items shown; more are available*") -and $null -ne (Named "Load more" $CT::Button)) $script:why
    $ok = (Press "Load more") -and (Shows "many/")
    Check "Load more: all 1,010 entries, every row as listed, no Load more after" ($ok -and ((PageText) -like "*1010 items shown.*") -and $null -eq (Named "Load more" $CT::Button)) $script:why
    $ok = (Press "Refresh") -and (Shows "many/" $true)
    Check "Refresh starts again at the first page" ($ok -and ((PageText) -like "*1000 items shown; more*")) $script:why
    $ok = (Press "$([char]0x2190) Back to accounts") -and (WaitFor { $null -ne (Named "Open bucket" $CT::Button) } 15)
    $ok = $ok -and (Press "Open bucket") -and (WaitFor { $t = PageText; ($t -like "*Root of the bucket*") -and ($t -notlike "*Loading*") } 60)
    Check "Back to accounts, then Open bucket again starts at the bucket's root" ($ok -and ((TableRows) -contains "sotyware-it | Folder | $([char]0x2014)")) (PageText)
  } elseif ($Mode -eq "files") {
   try {
    $ok = $ok -and (WaitFor { (PageText) -like "*photos-backup*" } 30)
    Check "files: Open bucket lists the root" $ok
    $seeded = FCurrent (FState)
    # Folders: create, open, nested, rename with the prefix boundary.
    $ok = (Press "New folder$([char]0x2026)") -and (FFill "Name" "made") -and (Press "Create folder") -and (Says @("Folder", "made", "created") 20)
    $k = FState
    Check "folders: New folder creates the marker made/ (and nothing else)" ($ok -and (FCurrent $k) -eq ((@($seeded.Split("|")) + "made/" | Sort-Object) -join "|"))
    $ok = (PressFolder "made") -and (WaitFor { (PageText) -like "*This folder is empty.*" } 20)
    $ok = $ok -and (Press "New folder$([char]0x2026)") -and (FFill "Name" "inner") -and (Press "Create folder") -and (Says @("inner", "created") 20)
    Check "folders: open the new folder, create a nested one in it" ($ok -and (FState).ContainsKey("made/inner/"))
    Recover
    $ok = (FRoot) -and (Sel "photos") -and (Press "Rename") -and (FFill "New name" "pictures") -and (Press "Rename")
    $rv = if ($ok) { Reviewed "Rename" } else { $null }
    $k = FState
    Check "folders: Rename photos/ to pictures/ moves exactly its 3 objects, content kept" ($rv -and (FCurrent $k "pictures/") -eq "pictures/a.txt|pictures/b.txt|pictures/sub/c.txt" -and (FText $k "pictures/sub/c.txt") -eq "content of photos/sub/c.txt`n") "$rv"
    Check "folders: prefix boundary: photos-backup/ and photosx untouched; originals behind delete markers" ((FCurrent $k "photos-backup") -eq "photos-backup/|photos-backup/keep.txt" -and $null -ne $k["photosx"].current -and (FCurrent $k "photos/") -eq "" -and @(FVersions $k "photos/a.txt").Count -eq 2)
    Recover
    $ok = (FRoot) -and (Sel "pictures") -and (Press "Copy to$([char]0x2026)") -and (Pick2 "made" "Copy to")
    $rv = if ($ok) { Reviewed "Copy" } else { $null }
    $k = FState
    Check "folders: Copy pictures/ into made/ through the picker; originals kept" ($rv -and (FCurrent $k "made/pictures/") -eq "made/pictures/a.txt|made/pictures/b.txt|made/pictures/sub/c.txt" -and (FCurrent $k "pictures/").Split("|").Count -eq 3) "$rv"
    Recover
    $ok = (FGo "made/") -and (Sel "pictures") -and (Press "Move to$([char]0x2026)") -and (Pick2 "" "Move to")
    $rv = if ($ok) { Reviewed "Move" { param($d) KeepBoth $d } } else { $null }
    $k = FState
    Check "folders: Move where the name exists shows the conflict and keeps both as pictures (1)" ($rv -and $rv.Contains("already exist") -and (FCurrent $k "pictures (1)/") -eq "pictures (1)/a.txt|pictures (1)/b.txt|pictures (1)/sub/c.txt" -and (FCurrent $k "made/pictures/") -eq "") "$rv"
    Recover
    $ok = (FRoot) -and (Sel "pictures") -and (Press "Delete")
    $rv = if ($ok) { Reviewed "Delete" } else { $null }
    $k = FState
    Check "folders: Delete pictures/ (bucket keeps versions) hides exactly its objects" ($rv -and $rv.Contains("keeps versions") -and (FCurrent $k "pictures/") -eq "" -and (FCurrent $k "pictures (1)/").Split("|").Count -eq 3 -and (FCurrent $k "photos-backup").Split("|").Count -eq 2) "$rv"
    Recover
    # Files: conflict skipped then kept both; refused delete of a move and retry.
    $ok = (FGo "conflict/src/") -and (Sel "x.txt") -and (Press "Copy to$([char]0x2026)") -and (Pick2 "conflict/dest" "Copy to")
    $script:rv = $null
    $ok = $ok -and (WaitFor { $script:rv = ModalWhere { param($n) $n.StartsWith("Copy") -and $n.TrimEnd().EndsWith("?") }; $null -ne $script:rv } 30)
    $rvText = if ($script:rv) { AllNames $script:rv } else { "" }
    $startBtn = InEl $script:rv "Copy" $CT::Button
    $disabled = $null -ne $startBtn -and -not $startBtn.Current.IsEnabled
    $cancelled = $ok -and (PressIn $script:rv "Cancel")
    Start-Sleep 1
    $k = FState
    Check "files: an existing destination is shown; with Skip (the default) nothing can start; Cancel; nothing replaced" ($ok -and $rvText.Contains("already exist") -and $disabled -and $cancelled -and (FText $k "conflict/dest/x.txt") -eq "already there`n" -and -not $k.ContainsKey("conflict/dest/x (1).txt")) "$rvText"
    Recover
    $ok = (FGo "conflict/src/") -and (Sel "x.txt") -and (Press "Copy to$([char]0x2026)") -and (Pick2 "conflict/dest" "Copy to")
    $rv = if ($ok) { Reviewed "Copy" { param($d) KeepBoth $d } } else { $null }
    $k = FState
    Check "files: Keep both copies to x (1).txt; the existing file unchanged" ($rv -and (FText $k "conflict/dest/x (1).txt") -eq "content of conflict/src/x.txt`n" -and (FText $k "conflict/dest/x.txt") -eq "already there`n") "$rv"
    Recover
    $ok = (FGo "files/") -and (Sel "two.txt") -and (Press "Move to$([char]0x2026)") -and (Pick2 "made" "Move to") -and (Ctl @("fail DeleteObject 1"))
    $rv = if ($ok) { Reviewed "Move" } else { $null }
    $k = FState
    Check "files: a move whose delete is refused: copied and verified, original kept; both exist" ($rv -and $rv.Contains("original could not be deleted") -and $null -ne (FText $k "made/two.txt") -and $null -ne (FText $k "files/two.txt")) "$rv"
    $r2 = RetryOp
    $k = FState
    Check "files: Try again deletes only the original left" ($r2 -and $null -eq (FText $k "files/two.txt") -and $null -ne (FText $k "made/two.txt")) "$r2"
    Recover
    # Cancel in the review changes nothing; delete and restore.
    $fBefore = (FCurrent (FState))
    $ok = (FGo "files/") -and (Sel "one.txt") -and (Press "Delete") -and (WaitFor { $null -ne (ModalWhere { param($n) $n.StartsWith("Delete") -and $n.TrimEnd().EndsWith("?") }) } 20)
    $ok = $ok -and (PressIn (ModalWhere { param($n) $n.StartsWith("Delete") -and $n.TrimEnd().EndsWith("?") }) "Cancel")
    Start-Sleep 1
    Check "delete: Cancel in the review changes nothing" ($ok -and (FCurrent (FState)) -eq $fBefore)
    $null = Sel "one.txt"
    $ok = (Sel "one.txt") -and (Press "Delete")
    $rv = if ($ok) { Reviewed "Delete" } else { $null }
    $k = FState
    Check "delete: deleting one.txt adds a delete marker and keeps the version" ($rv -and $null -eq $k["files/one.txt"].current -and @(FVersions $k "files/one.txt").Count -eq 2)
    $ok = (PressToggle "Deleted items") -and (WaitFor { $null -ne (Named "Restore one.txt" $CT::Button) } 20) -and (Press "Restore one.txt")
    if (-not $ok) { Write-Host "  restore: no Restore one.txt: $((PageText).Substring([Math]::Max(0, (PageText).Length - 400)))" }
    $rv = if ($ok) { Reviewed "Restore" } else { $null }
    $k = FState
    Check "delete: Restore from Deleted items removes exactly the delete marker; one.txt is back" ($rv -and (FText $k "files/one.txt") -eq "content of files/one.txt`n" -and @(FVersions $k "files/one.txt").Count -eq 1) "$rv"
    $null = Press "$([char]0x2190) Back to the folder"
    Recover
    # Previews: text, HTML and SVG as text (never rendered), PNG as an image.
    foreach ($pv in @(@("docs/", "readme.txt", "Hello from the readme."), @("docs/", "cars.txt", "BMW, Audi"), @("docs/", "page.html", "<script>"), @("docs/", "logo.svg", "<svg"))) {
      $ok = (FGo $pv[0]) -and (Sel $pv[1]) -and (Press "Properties")
      $want = $pv[2]
      $got = $ok -and (WaitFor { (PageText).Contains($want) } 20)
      $pt = PageText; $at = $pt.IndexOf("Preview |")
      # The file's own text may say PWNED; executed, it would set the title.
      Check "preview: $($pv[1]) is shown as text, nothing in it ran (window title unchanged)" ($got -and (Dash).Current.Name -eq "sotyware" -and $null -eq (Named "Preview of $($pv[1])" $CT::Image)) "$(if ($at -ge 0) { $pt.Substring($at, [Math]::Min(300, $pt.Length - $at)) })"
      $null = Press "Close"; Start-Sleep -Milliseconds 400
    }
    $ok = (FGo "media/") -and (Sel "pixel.png") -and (Press "Properties")
    Check "preview: a PNG is shown as an image" ($ok -and (WaitFor { $null -ne (Named "Preview of pixel.png" $CT::Image) } 20))
    $null = Press "Close"
    Recover
    # The unversioned bucket: a delete warns that it is final.
    $ok = Chain "plain account" @(
      { Press "$([char]0x2190) Back to accounts" },
      { WaitFor { $null -ne (Named "Open bucket" $CT::Button) } 20 },
      { Press "Add account" },
      { WaitFor { (Fields) -contains "Secret access key" } 10 },
      { Fill "Name" "Plain" }, { Fill "Bucket" "files-plain" }, { Fill "Region" $Region },
      { Fill "Custom endpoint (optional)" $Endpoint }, { Fill "Access key ID" $Key1 },
      { (Fill "Secret access key" $Secret1) -or ((Start-Sleep -Milliseconds 500) -or (Fill "Secret access key" $Secret1)) -or ((Write-Host "  fields: $((Fields) -join ', ')") -and $false) },
      { Press "Add account" },
      { Says @("Added Plain") 30 },
      { $script:opens = @((Dash).FindAll($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $CT::Button), (PCN "Open bucket")))); $script:opens.Count -eq 2 }
    )
    # The cards are in account-id order: "plain" before "sotyware-e2e-...".
    if ($ok) { $script:opens[0].GetCurrentPattern($Invoke).Invoke(); $ok = WaitFor { (PageText).Contains("Root of the bucket") -and (PageText).Contains("files-plain") } 30 }
    if (-not $ok) { Write-Host "  plain: the second account did not open" }
    $ok = $ok -and (PressFolder "files") -and (WaitFor { (PageText) -like "*one.txt*" } 20) -and (Sel "one.txt") -and (Press "Delete")
    $rv = if ($ok) { Reviewed "Delete" } else { $null }
    $kp = FState "files-plain"
    Check "plain: a delete on a bucket without versions warns it is final; the object is gone" ($rv -and $rv.Contains("gone for good") -and -not $kp.ContainsKey("files/one.txt")) "$rv"
    Recover
    $null = Press "$([char]0x2190) Back to accounts"
    # Remove the second account again (its entry is in this profile's
    # namespace), so the common steps below see only the first one.
    $null = WaitFor { $null -ne (Named "Open bucket" $CT::Button) } 20
    Check "plain: the second account is removed again" (RemoveCard "Plain")
    # The common steps below start inside the first account's bucket.
    $null = (Press "Open bucket") -and (WaitFor { (PageText).Contains("Root of the bucket") } 30)
    "  NOT TESTED  several-item downloads and folder uploads through the native Windows folder dialog (not driven by this mode yet; covered on Linux by cloud-ui.sh files and by the Rust tests)"
    "  NOT TESTED  drag and drop from Explorer; the system clipboard"
   } catch {
    Check "files steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    Recover
   }
  } elseif ($Mode -eq "download") {
   try {
    $ok = $ok -and (WaitFor { (PageText) -like "*readme.txt*" } 30)
    Check "Open bucket lists the files with a Download action each" ($ok -and $null -ne (Named "Download readme.txt" $CT::Button) -and $null -ne (Named "Download foreign.txt" $CT::Button)) (PageText)

    $dlg = OpenSave "readme.txt"
    Check "Download opens the native save dialog with the file name suggested" ($null -ne $dlg -and (Suggested $dlg) -eq "readme.txt") "$(if ($dlg) { Suggested $dlg }) | $(PageText)"
    Check "while the dialog is open, the page says so" ((PageText) -like "*Choose where to save the file*")
    if ($dlg) { CancelDialog $dlg }
    $gone = WaitFor { ($null -eq (DialogNamed "Save download as")) -and ((PageText) -notlike "*Choose where to save*") } 10
    Check "Cancel in the dialog starts nothing and leaves no file" ($gone -and (Saved).Count -eq 0) "$(Saved)"

    $target = Join-Path $Out "readme copy.txt"
    $dlg = OpenSave "readme.txt"
    $ok = $dlg -and (SaveAs $dlg $target) -and (Says @("Downloaded", "SHA-256 checksum stored with the file matched"))
    Check "save: the file is downloaded and its SHA-256 verified" ($ok -and ((Saved) -join "|") -eq "readme copy.txt" -and (Sha $target) -eq $script:objects["readme.txt"].sha256) "$(Saved) | $(PageText)"
    $null = Press "Dismiss"

    $mine = Join-Path $Out "existing.txt"
    Set-Content $mine "an older file of mine" -NoNewline
    $dlg = OpenSave "readme.txt"
    $ok = $dlg -and (SaveAs $dlg $mine)
    $script:conf = $null
    $asked = $ok -and (WaitFor { $script:conf = DialogNamed "Confirm Save As"; $null -ne $script:conf } 10)
    Check "an existing file: Windows asks before replacing it, and nothing is written meanwhile" ($asked -and (Get-Content $mine -Raw) -eq "an older file of mine") "$(PageText)"
    # "Confirm Save As" is a task dialog; its Yes (IDYES) is pressed through
    # UI Automation if it is exposed there, else with TDM_CLICK_BUTTON.
    if ($script:conf) {
      $yes = $script:conf.FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $CT::Button), (PCN "Yes")))
      if ($yes) { $yes.GetCurrentPattern($Invoke).Invoke() } else { [CloudUiW]::Answer((Hwnd $script:conf), 6) }
    }
    $ok = $asked -and (Says @("Downloaded", "matched"))
    Check "after Yes the file is replaced in place, no temporary file left" ($ok -and (Sha $mine) -eq $script:objects["readme.txt"].sha256 -and ((Saved) -join "|") -eq "existing.txt|readme copy.txt") "$(Saved)"
    $null = Press "Dismiss"

    $key = "docs/Bericht 2024 $([char]0xFC) & co.pdf"; $name = "Bericht 2024 $([char]0xFC) & co.pdf"
    $ok = (PressFolder "docs") -and (WaitFor { (PageText).Contains($name) } 20)
    $dlg = if ($ok) { OpenSave $name } else { $null }
    Check "a Unicode name with & and spaces is suggested unchanged" ($null -ne $dlg -and (Suggested $dlg) -eq $name) "$(if ($dlg) { Suggested $dlg })"
    $dest = Join-Path $Out $name
    $ok = $dlg -and (SaveAs $dlg $dest) -and (Says @("Downloaded 3.0 MiB", "matched"))
    Check "the 3 MiB file under its exact key and a Unicode file name, SHA-256 verified" ($ok -and (Sha $dest) -eq $script:objects[$key].sha256) (PageText)
    $null = Press "Dismiss"
    $null = (PressFolder $Bucket) -and (WaitFor { (PageText) -like "*readme.txt*" } 20)

    $dlg = OpenSave "foreign.txt"
    $ok = $dlg -and (SaveAs $dlg (Join-Path $Out "foreign.txt")) -and (Says @("Downloaded", "no SHA-256 checksum stored by sotyware"))
    Check "a file without a stored SHA-256: completed, and only the size is claimed" ($ok -and ((PageText) -notlike "*checksum stored with the file matched*") -and (Sha (Join-Path $Out "foreign.txt")) -eq $script:objects["foreign.txt"].sha256) (PageText)
    $null = Press "Dismiss"

    # A destination name with an unpaired surrogate (NTFS allows it; it is not
    # valid Unicode): the save dialog's path must reach sotyware unchanged
    # (ADR-023; with upstream rfd 0.17.2 the conversion panicked here) and the
    # file is saved under exactly that name. .NET, not cmdlets, for the name.
    $LoneSave = "lone-$([char]0xD800)-copy.txt"
    $loneDest = Join-Path $Out $LoneSave
    $dlg = OpenSave "readme.txt"
    $ok = $dlg -and (SaveAs $dlg $loneDest)
    $died = WaitFor { $p.HasExited } 5
    $ok = $ok -and -not $died -and (Says @("Downloaded", "matched"))
    $loneSha = if ([System.IO.File]::Exists($loneDest)) { ([System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::Create().ComputeHash([System.IO.File]::ReadAllBytes($loneDest))) -replace "-", "").ToLowerInvariant() } else { "" }
    Check "a destination name with an unpaired surrogate in the save dialog: sotyware keeps running and saves under exactly that name, SHA-256 verified" ($ok -and $loneSha -eq $script:objects["readme.txt"].sha256 -and @(Saved | Where-Object { $_ -like "*sotyware-partial" }).Count -eq 0) "exited=$died$(if ($died) { ' 0x{0:X8}' -f $p.ExitCode }) | $(Saved) | $(PageText)"
    $null = Press "Dismiss"

    $ok = (PressFolder "slow") -and (WaitFor { (PageText) -like "*large.bin*" } 20)
    $dlg = if ($ok) { OpenSave "large.bin" } else { $null }
    $ok = $dlg -and (SaveAs $dlg (Join-Path $Out "large.bin")) -and (Says @("% ", "of 16 MiB") 20)
    $partial = @(Saved | Where-Object { $_ -like ".large.bin.*.sotyware-partial" })
    Check "progress shows percent and bytes; the data goes to a temporary file" ($ok -and $partial.Count -eq 1 -and (Saved) -notcontains "large.bin") "$(Saved) | $(PageText)"
    $ok = (Press "Cancel download") -and (Says @("Download cancelled. Nothing was saved.") 15)
    Check "Cancel stops it: no file and no temporary file" ($ok -and @(Saved | Where-Object { $_ -like "*large.bin*" }).Count -eq 0) "$(Saved)"
    $null = Press "Dismiss"
    $null = (PressFolder $Bucket) -and (WaitFor { (PageText) -like "*readme.txt*" } 20)

    $ok = (PressFolder "broken") -and (WaitFor { (PageText) -like "*cut.bin*" } 20)
    $dlg = if ($ok) { OpenSave "cut.bin" } else { $null }
    $ok = $dlg -and (SaveAs $dlg (Join-Path $Out "cut.bin")) -and (Says @("Download failed. Nothing was saved.", "could not be reached") 30)
    Check "a broken connection: failed, with Try again, nothing saved" ($ok -and $null -ne (Named "Try download again" $CT::Button) -and @(Saved | Where-Object { $_ -like "*cut.bin*" }).Count -eq 0) "$(Saved) | $(PageText)"
    $ok = (Press "Try download again") -and (WaitFor { ((PageText) -like "*Download failed*") -and $null -ne (Named "Try download again" $CT::Button) } 30)
    Check "Try again repeats it without a dialog (and fails the same way here)" ($ok -and $null -eq (DialogNamed "Save download as") -and @(Saved | Where-Object { $_ -like "*cut.bin*" }).Count -eq 0) (PageText)
    $null = Press "Dismiss"
    $dlg = OpenSave "tampered.bin"
    $ok = $dlg -and (SaveAs $dlg (Join-Path $Out "tampered.bin")) -and (Says @("Download failed", "did not pass the integrity check") 30)
    Check "content that does not match its SHA-256 is never saved" ($ok -and @(Saved | Where-Object { $_ -like "*tampered*" }).Count -eq 0) "$(Saved) | $(PageText)"
    $null = Press "Dismiss"
    $expected = @("Bericht 2024 $([char]0xFC) & co.pdf", "existing.txt", "foreign.txt", $LoneSave, "readme copy.txt") | Sort-Object
    Check "the saved files are exactly the completed downloads" (((Saved) -join "|") -eq ($expected -join "|")) "$(Saved)"
   } catch {
    # Still go on to remove the account, so no test key stays behind.
    Check "download steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    $d = DialogNamed "Save download as"; if ($d) { CancelDialog $d }
    $null = Press "Dismiss"
   }
  } elseif ($Mode -eq "upload") {
   $locker = $null
   try {
    $ok = $ok -and (WaitFor { $t = PageText; $t.Contains("inbox") -and $t.Contains("downloads") } 30)
    Check "Open bucket lists the seeded folders" $ok (PageText)
    $st = State
    $seedOk = (SortedKeys $st.Objects.Keys) -eq (SortedKeys $script:objects.Keys)
    foreach ($k in $script:objects.Keys) { $seedOk = $seedOk -and (Obj $st $k) -and (Obj $st $k).sha256Metadata -eq $script:objects[$k].sha256 }
    Check "before any upload the server holds exactly the three seeded objects, sotyware's SHA-256 with each" ($seedOk -and $st.Multipart.Count -eq 0) "$(SortedKeys $st.Objects.Keys) $($st.Multipart -join ',')"
    $opsBefore = UploadOps $st

    # 0. The dialog cancelled: nothing starts.
    Check "open inbox/" (UpGo "inbox" "existing.txt") (PageText)
    $script:odlg = $null
    $shown = (Press $UploadButton) -and (WaitFor { $script:odlg = DialogNamed $OpenTitle; ($null -ne $script:odlg) -and ($null -ne (OpenNameBox $script:odlg)) } 20)
    $isOpen = $shown -and $script:odlg.Current.ClassName -eq "#32770" -and $null -ne (DialogButton $script:odlg 1)
    Check "Upload files$([char]0x2026) opens the native Windows dialog '$OpenTitle' (common item dialog #32770 of sotyware's process, File name box and Open)" $isOpen (PageText)
    Check "while it is open, the panel says to choose the files" ((UpPanel).Contains("Choose the files to upload")) (UpPanel)
    if ($script:odlg) { CancelDialog $script:odlg }
    $gone = WaitFor { ($null -eq (DialogNamed $OpenTitle)) -and ((UpPanel) -eq "") } 10
    $st = State
    Check "Cancel in the dialog: no panel, Upload files$([char]0x2026) enabled, nothing sent (server unchanged)" ($gone -and (UploadEnabled) -and (SortedKeys $st.Objects.Keys) -eq (SortedKeys $script:objects.Keys) -and (UploadOps $st) -eq $opsBefore) "$(UpPanel) | $(SortedKeys $st.Objects.Keys)"

    # 1. Odd names, the empty file, an existing object, a file another
    #    process holds open without sharing; sequential uploads, progress, and
    #    the destination fixed when the dialog opened.
    $UniName = "Gr$([char]0xFC)$([char]0xDF)e $([char]0xDC)n$([char]0xEF)c$([char]0xF6)d$([char]0xE9) $([char]0x65E5)$([char]0x672C).txt"
    $SpecialName = "report 2024 (final) + notes & more #1.txt"
    $b1 = @{}
    $b1[$UniName] = UpFile "batch1" $UniName (Utf8 "Unicode name")
    $b1["empty.txt"] = UpFile "batch1" "empty.txt" ([byte[]]@())
    $b1[$SpecialName] = UpFile "batch1" $SpecialName (Utf8 "spaces, parentheses, plus, ampersand and hash")
    $b1["existing.txt"] = UpFile "batch1" "existing.txt" (Utf8 "mine, not the bucket's")
    $b1["medium.bin"] = UpBlob "batch1" "medium.bin" (12 * $MiB) 1
    $b1["locked.txt"] = UpFile "batch1" "locked.txt" (Utf8 "held open without sharing")
    $B1Names = @($UniName, "empty.txt", $SpecialName, "existing.txt", "medium.bin", "locked.txt")
    # Another process (this script) holds locked.txt open with no sharing.
    $locker = [System.IO.File]::Open($b1["locked.txt"], "Open", "Read", "None")
    $script:nav = $null; $script:seen.Clear()
    $ok = Pick (UpFolder "batch1") $B1Names "list" { $script:nav = (PressFolder $Bucket) -and (WaitFor { (PageText).Contains("Root of the bucket") } 10) }
    Check "native dialog: folder opened through File name + Open, 6 files selected in its file list (UI Automation), Open" $ok
    Check "while the dialog was open the browser moved to the bucket root" ($script:nav -eq $true) "$script:nav"
    # "Upload", " to " and the destination are separate texts to UI Automation.
    $t = Until { param($x) $x.Contains("$Bucket / ") } 15
    Check "the panel names the folder captured when the dialog opened, not the one shown now" ($t -and $t.Contains("$Bucket / inbox/") -and (PageText).Contains("Root of the bucket")) "$t"
    $mid = Until { param($x) SendingIn $x "medium.bin" } 60
    $st = State
    Check "while medium.bin is being sent, the server has no object inbox/medium.bin yet" ($mid -and -not $st.Objects.ContainsKey("inbox/medium.bin")) "$mid $(KeysUnder $st 'inbox/')"
    $done = Until { param($x) Finished $x } 120
    $ur = UpRows
    "  file rows: $($ur -join ' || ')"
    Check "final summary: Upload finished. 6 files: 4 uploaded, 1 skipped, 1 failed." ($done -and $done.Contains("Upload finished. 6 files: 4 uploaded, 1 skipped, 1 failed.")) "$done"
    $oneRowEach = $ur.Count -eq 6; foreach ($n in $B1Names) { $oneRowEach = $oneRowEach -and @($ur | Where-Object { $_.StartsWith($n, [StringComparison]::Ordinal) }).Count -eq 1 }
    Check "6 rows, one per chosen file, names exactly as on disk (Unicode, spaces, + & # ( ))" $oneRowEach ($ur -join " || ")
    Check "the empty file, the Unicode name, the special characters and medium.bin: Uploaded and verified" (@("empty.txt", $UniName, $SpecialName, "medium.bin" | Where-Object { -not (UpRow $_).Contains("Uploaded and verified") }).Count -eq 0) ($ur -join " || ")
    Check "existing.txt: Skipped: already in this folder" ((UpRow "existing.txt").Contains("Skipped: already in this folder")) (UpRow "existing.txt")
    $lr = UpRow "locked.txt"
    Check "locked.txt (open in another process without sharing): Failed, could not be read on this computer, nothing stored" ($lr.Contains("Failed") -and ($lr.Contains("could not be read on this computer") -or $lr.Contains("may not read this file")) -and $lr.Contains("Nothing was stored for this file.")) $lr
    $stages = @("Computing the SHA-256 checksum", "Sending $([char]0xB7) ", "Verifying the stored object", "Starting$([char]0x2026)" | Where-Object { $s1 = $_; @($script:seen | Where-Object { $_.Contains($s1) }).Count -gt 0 })
    "  stages seen in the panel ($($script:seen.Count) distinct samples; $script:polls polls so far, $([Math]::Round($script:pollTime.Elapsed.TotalMilliseconds / [Math]::Max(1, $script:polls))) ms each): $($stages -join ', ')"
    Check "progress: medium.bin was seen Sending with a percentage" ($stages -contains "Sending $([char]0xB7) ") ($stages -join ", ")
    $st = State
    foreach ($n in "empty.txt", $UniName, $SpecialName, "medium.bin", "locked.txt") { [void]$script:created.Add("inbox/$n") }
    Check "server: inbox/ holds exactly existing.txt and the four uploaded files under their exact keys" ((KeysUnder $st "inbox/") -eq (SortedKeys @("inbox/existing.txt", "inbox/empty.txt", "inbox/$UniName", "inbox/$SpecialName", "inbox/medium.bin"))) (KeysUnder $st "inbox/")
    $contentOk = (StoredOk (Obj $st "inbox/empty.txt") $b1["empty.txt"]) -and (Obj $st "inbox/empty.txt").sha256 -eq "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    foreach ($n in $UniName, $SpecialName, "medium.bin") { $contentOk = $contentOk -and (StoredOk (Obj $st "inbox/$n") $b1[$n]) }
    Check "server: contents, sizes and SHA-256 metadata equal the local files (empty file: 0 bytes, e3b0...)" $contentOk
    Check "server: the existing object was left as it was; nothing went to the root; no multipart upload open" ((Obj $st "inbox/existing.txt").sha256 -eq $script:objects["inbox/existing.txt"].sha256 -and @($st.Objects.Keys | Where-Object { -not $_.Contains("/") }).Count -eq 0 -and $st.Multipart.Count -eq 0)
    Check "Try again offers exactly 1 file (locked.txt)" (Btn "Try 1 file again") (PageText)
    $locker.Close(); $locker = $null
    $null = Press "Try 1 file again"
    $done = Until { param($x) Retried $x } 60
    $st = State
    Check "once the other process closed it, Try again uploads locked.txt: Upload finished. 1 file: 1 uploaded." ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "inbox/locked.txt") $b1["locked.txt"]) -and -not $done.Contains("medium.bin")) "$done"
    UpDismiss

    # 2. Cancel during a single PUT; the file before it stays, the one after
    #    it is not started; Try again takes exactly the two eligible files.
    $a2 = UpFile "batch2" "a-first.txt" (Utf8 "first")
    $b2 = UpBlob "batch2" "b-big.bin" (40 * $MiB) 2
    $c2 = UpFile "batch2" "c-third.txt" (Utf8 "third")
    $ok = (UpGo "inbox" "existing.txt") -and (Pick (UpFolder "batch2") @("a-first.txt", "b-big.bin", "c-third.txt") "names")
    $running = $ok -and (Until { param($x) SendingIn $x "b-big.bin" 10 } 60)
    Check "batch 2 (names typed into File name): a-first.txt done, b-big.bin being sent (single PUT)" ($running -and (UpRow "a-first.txt").Contains("Uploaded and verified")) (UpPanel)
    $null = Press "Cancel upload"
    $done = Until { param($x) Finished $x } 60
    Check "Cancel during b-big.bin: Upload cancelled. 3 files: 1 uploaded, 1 cancelled, 1 not started." ($done -and $done.Contains("Upload cancelled. 3 files: 1 uploaded, 1 cancelled, 1 not started.")) "$done"
    Check "rows: cancelled during its upload with nothing stored; the next not started" ((UpRow "b-big.bin").Contains("Cancelled during its upload") -and (UpRow "b-big.bin").Contains("Nothing was stored for this file.") -and (UpRow "c-third.txt").Contains("Not started: the upload was cancelled")) ((UpRows) -join " || ")
    $st = State
    foreach ($n in "a-first.txt", "b-big.bin", "c-third.txt") { [void]$script:created.Add("inbox/$n") }
    Check "server after the cancel: a-first.txt stored, no b-big.bin, no c-third.txt, no multipart" ((StoredOk (Obj $st "inbox/a-first.txt") $a2) -and -not $st.Objects.ContainsKey("inbox/b-big.bin") -and -not $st.Objects.ContainsKey("inbox/c-third.txt") -and $st.Multipart.Count -eq 0) "$(KeysUnder $st 'inbox/') $($st.Multipart -join ',')"
    Check "Try again offers exactly the 2 eligible files" (Btn "Try 2 files again") (PageText)
    $null = Press "Try 2 files again"
    $done = Until { param($x) Retried $x } 90
    $st = State
    Check "retry: Upload finished. 2 files: 2 uploaded; a-first.txt not sent again" ($done -and $done.Contains("Upload finished. 2 files: 2 uploaded.") -and (StoredOk (Obj $st "inbox/b-big.bin") $b2) -and (StoredOk (Obj $st "inbox/c-third.txt") $c2) -and $st.Multipart.Count -eq 0) "$done"
    UpDismiss

    # 3. Cancel during a multipart upload: aborted, no parts left.
    $null = UpBlob "batch3" "a-multi.bin" (72 * $MiB) 3
    $ok = Pick (UpFolder "batch3") @("a-multi.bin") "names"
    $running = $ok -and (Until { param($x) SendingIn $x "a-multi.bin" 25 } 60)
    $st = State
    Check "batch 3: a 72 MiB multipart upload in progress; the server holds its parts" ($running -and $st.Multipart.Count -gt 0) "$(UpPanel) $($st.Multipart -join ',')"
    $null = Press "Cancel upload"
    $done = Until { param($x) Finished $x } 60
    $st = State
    [void]$script:created.Add("inbox/a-multi.bin")
    Check "multipart cancelled: Upload cancelled. 1 file: 0 uploaded, 1 cancelled.; nothing stored, aborted (no parts left)" ($done -and $done.Contains("Upload cancelled. 1 file: 0 uploaded, 1 cancelled.") -and (UpRow "a-multi.bin").Contains("Nothing was stored for this file.") -and -not $st.Objects.ContainsKey("inbox/a-multi.bin") -and $st.Multipart.Count -eq 0) "$done $($st.Multipart -join ',')"
    UpDismiss

    # 4. The abort is refused: parts may remain; not retryable itself; the
    #    retry of the other file aborts the remembered upload again.
    $null = UpBlob "batch4" "a-parts.bin" (72 * $MiB) 4
    $b4 = UpFile "batch4" "b-next.txt" (Utf8 "next")
    $ok = (Ctl @("abort-denied")) -and (Pick (UpFolder "batch4") @("a-parts.bin", "b-next.txt") "names")
    $running = $ok -and (Until { param($x) SendingIn $x "a-parts.bin" 25 } 60)
    $null = Press "Cancel upload"
    $done = Until { param($x) Finished $x } 60
    $st = State
    foreach ($n in "a-parts.bin", "b-next.txt") { [void]$script:created.Add("inbox/$n") }
    Check "abort refused: the row says parts may remain; nothing completed; the parts are still on the server" ($running -and $done -and (UpRow "a-parts.bin").Contains("may remain at the provider") -and -not $st.Objects.ContainsKey("inbox/a-parts.bin") -and $st.Multipart.Count -gt 0) "$done $(UpRow 'a-parts.bin') $($st.Multipart -join ',')"
    Check "Try again offers only b-next.txt (1 file), never the file with incomplete parts" ((Btn "Try 1 file again") -and -not (Btn "Try 2 files again")) (PageText)
    $null = Press "Try 1 file again"
    $done = Until { param($x) Retried $x } 60
    $st = State
    Check "retry: the remembered upload was aborted (cleanup shown) and b-next.txt uploaded; no parts left" ($done -and $done.Contains("were removed") -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "inbox/b-next.txt") $b4) -and -not $st.Objects.ContainsKey("inbox/a-parts.bin") -and $st.Multipart.Count -eq 0) "$done $($st.Multipart -join ',')"
    UpDismiss

    # 5. Verification refused after the commit point: needs inspection, no
    #    retry for that file, and the batch stops.
    $a5 = UpFile "batch5" "a-inspect.txt" (Utf8 "inspect me")
    $b5 = UpFile "batch5" "b-after.txt" (Utf8 "after")
    $ok = (Ctl @("verify-denied")) -and (Pick (UpFolder "batch5") @("a-inspect.txt", "b-after.txt") "names")
    $done = if ($ok) { Until { param($x) Finished $x } 60 } else { $null }
    Check "verification refused: Upload stopped after an error. 2 files: 0 uploaded, 1 failed, 1 not started." ($ok -and $done -and $done.Contains("Upload stopped after an error. 2 files: 0 uploaded, 1 failed, 1 not started.")) "$done"
    Check "the row says the provider may have stored it and sotyware does not retry or delete it" ((UpRow "a-inspect.txt").Contains("Failed") -and (UpRow "a-inspect.txt").Contains("may have stored this file") -and (UpRow "b-after.txt").Contains("Not started: the upload stopped after an error")) ((UpRows) -join " || ")
    $st = State
    foreach ($n in "a-inspect.txt", "b-after.txt") { [void]$script:created.Add("inbox/$n") }
    Check "server: the ambiguous object really was stored, and was not deleted" ((StoredOk (Obj $st "inbox/a-inspect.txt") $a5) -and -not $st.Objects.ContainsKey("inbox/b-after.txt")) (KeysUnder $st "inbox/")
    Check "no unsafe retry: Try again offers 1 file (b-after.txt), not the one that needs inspection" ((Btn "Try 1 file again") -and -not (Btn "Try 2 files again")) (PageText)
    $null = Press "Try 1 file again"
    $done = Until { param($x) Retried $x } 60
    $st = State
    Check "retry uploads only b-after.txt" ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "inbox/b-after.txt") $b5) -and -not $done.Contains("a-inspect")) "$done"
    UpDismiss

    # 6. A file that changes while it is sent fails with nothing stored; its
    #    retry uploads the new content.
    $a6 = UpBlob "batch6" "a-changing.bin" (30 * $MiB) 6
    $ok = Pick (UpFolder "batch6") @("a-changing.bin") "names"
    $running = $ok -and (Until { param($x) SendingIn $x "a-changing.bin" 20 } 60)
    # sotyware opens the file with read, write and delete sharing, so another
    # program can still append to it.
    if ($running) { [System.IO.File]::AppendAllText($a6, "appended while sending") }
    $done = Until { param($x) Finished $x } 60
    $st = State
    [void]$script:created.Add("inbox/a-changing.bin")
    Check "changed while sent: Failed, the file changed, nothing stored, Try 1 file again" ($running -and $done -and (UpRow "a-changing.bin").Contains("Failed") -and (UpRow "a-changing.bin").Contains("changed while it was being uploaded") -and (UpRow "a-changing.bin").Contains("Nothing was stored for this file.") -and (Btn "Try 1 file again") -and -not $st.Objects.ContainsKey("inbox/a-changing.bin")) "$done | $(UpRow 'a-changing.bin')"
    $null = Press "Try 1 file again"
    $done = Until { param($x) Retried $x } 90
    $st = State
    Check "retry uploads the file as it is now (new size and SHA-256)" ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "inbox/a-changing.bin") $a6)) "$done"
    UpDismiss

    # 7. A download through the native save dialog beside an upload.
    $a7 = UpBlob "batch7" "a-alongside.bin" (40 * $MiB) 7
    $ok = (UpGo "downloads" "big.bin") -and (Pick (UpFolder "batch7") @("a-alongside.bin") "names")
    $running = $ok -and (Until { param($x) SendingIn $x "a-alongside.bin" 5 } 60)
    $dlg = if ($running) { OpenSave "big.bin" } else { $null }
    $target = Join-Path $Out "big copy.bin"
    $downloaded = $dlg -and (SaveAs $dlg $target) -and (Says @("Downloaded", "matched") 90)
    $during = UpPanel
    Check "while the upload is being sent, a download through the native save dialog completes and is verified" ($downloaded -and $during.Contains("Sending $([char]0xB7) ") -and (Btn "Cancel upload") -and (Test-Path -LiteralPath $target) -and (UpSha $target) -eq $script:objects["downloads/big.bin"].sha256) "$during | $(PageText)"
    $done = Until { param($x) Finished $x } 90
    $st = State
    [void]$script:created.Add("downloads/a-alongside.bin")
    Check "the upload beside it finishes too" ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "downloads/a-alongside.bin") $a7)) "$done"
    $null = Press "Dismiss"
    UpDismiss

    # 8. Leaving the browser during a batch cancels it.
    $null = UpBlob "batch8" "a-away.bin" (40 * $MiB) 8
    $null = UpFile "batch8" "b-away.txt" (Utf8 "away")
    $ok = (UpGo "inbox" "existing.txt") -and (Pick (UpFolder "batch8") @("a-away.bin", "b-away.txt") "names")
    $running = $ok -and (Until { param($x) SendingIn $x "a-away.bin" 10 } 60)
    $left = $running -and (Press "$([char]0x2190) Back to accounts") -and (WaitFor { Btn "Open bucket" } 10)
    Start-Sleep 10  # longer than the rest of a-away.bin would take
    $st = State
    foreach ($n in "a-away.bin", "b-away.txt") { [void]$script:created.Add("inbox/$n") }
    Check "Back to accounts during the batch: neither file stored afterwards (the batch was cancelled), no parts" ($left -and -not $st.Objects.ContainsKey("inbox/a-away.bin") -and -not $st.Objects.ContainsKey("inbox/b-away.txt") -and $st.Multipart.Count -eq 0) (KeysUnder $st "inbox/")
    $ok = (Press "Open bucket") -and (WaitFor { (PageText).Contains("inbox") } 30)
    Check "back in the browser: no panel, and Upload files$([char]0x2026) is enabled (the slot is free)" ($ok -and (UpPanel) -eq "" -and (UploadEnabled)) (UpPanel)

    # 9. Close (the title bar's X; tray mode: hide) during a batch; a second
    #    launch shows the dashboard again with the same batch.
    $a9 = UpBlob "batch9" "a-hidden.bin" (40 * $MiB) 9
    $ok = (UpGo "inbox" "existing.txt") -and (Pick (UpFolder "batch9") @("a-hidden.bin") "names")
    $running = $ok -and (Until { param($x) SendingIn $x "a-hidden.bin" 5 } 60)
    $hidden = $running -and [CloudUiW]::SysClose((Hwnd (Dash))) -and (WaitFor { -not (DashVisible) } 10)
    Check "close (SC_CLOSE, as the title bar's X) during the upload hides the window; the process keeps running" ($hidden -and -not $p.HasExited -and (Procs).Count -eq 1)
    Start-Sleep 3
    $second = Start-Process $Exe -PassThru; $null = $second.Handle
    $secondDone = $second.WaitForExit(30000)
    $shown = WaitFor { DashVisible } 10
    $t = UpPanel
    Check "a second launch shows the dashboard again (exit 0); the panel still shows the same batch" ($secondDone -and $second.ExitCode -eq 0 -and $shown -and $t.Contains("a-hidden.bin") -and (Procs).Count -eq 1) "exit $($second.ExitCode) $t"
    $done = Until { param($x) Finished $x } 90
    $st = State
    [void]$script:created.Add("inbox/a-hidden.bin")
    Check "the batch finished while hidden and shown: uploaded and verified" ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and (StoredOk (Obj $st "inbox/a-hidden.bin") $a9)) "$done"
    UpDismiss

    # 10. Quit from the tray (real mouse) during a multipart upload.
    $null = UpBlob "batch10" "a-quit.bin" (72 * $MiB) 10
    $ok = Pick (UpFolder "batch10") @("a-quit.bin") "names"
    $running = $ok -and (Until { param($x) SendingIn $x "a-quit.bin" 25 } 60)
    $stMid = State
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $quitSent = $running -and (TrayQuit)
    $exited = $quitSent -and $p.WaitForExit(45000)
    $secs = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
    if (OverflowOpen) { [CloudUiW]::Esc() }
    # If Quit did not end it, stop it here (the check below fails), so the
    # relaunch starts a new process instead of handing over to this one.
    if (-not $exited) { Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }; $null = WaitFor { (Procs).Count -eq 0 } 10 }
    $st = State
    [void]$script:created.Add("inbox/a-quit.bin")
    "  tray Quit to exit: $secs s"
    Check "tray Quit (real mouse) during a multipart upload: exit 0; no object, its parts aborted" ($running -and $stMid.Multipart.Count -gt 0 -and $exited -and $p.ExitCode -eq 0 -and -not $st.Objects.ContainsKey("inbox/a-quit.bin") -and $st.Multipart.Count -eq 0) "sent=$quitSent $script:why exit=$(if ($exited) { $p.ExitCode }) $($stMid.Multipart -join ',') -> $($st.Multipart -join ',')"

    # Relaunch: the account (database and Credential Manager) is still there.
    $ok = LaunchToBucket
    if ($script:app) { $p = $script:app }
    Check "relaunch: the account is still listed and its bucket opens (key read from Credential Manager)" $ok (PageText)

    # 11. A file name with an unpaired surrogate (NTFS allows it; it is not
    #     valid Unicode, so it cannot be an object key) chosen in the dialog
    #     with a valid one: sotyware must keep running.
    $LoneName = "lone-$([char]0xD800)-surrogate.txt"
    $null = UpFile "batch11" $LoneName (Utf8 "lone surrogate")
    $b11 = UpFile "batch11" "z-valid.txt" (Utf8 "valid")
    [void]$script:created.Add("inbox/z-valid.txt")
    $ok = (Procs).Count -eq 1 -and (Procs)[0].Id -eq $p.Id -and (UpGo "inbox" "existing.txt") -and (Pick (UpFolder "batch11") @($LoneName, "z-valid.txt") "all")
    $died = WaitFor { $p.HasExited } 8
    $code = if ($died) { $p.ExitCode } else { $null }
    "  unpaired surrogate: dialog closed=$ok, process exited=$died$(if ($died) { ", exit code $code (0x{0:X8})" -f $code })"
    $t = if ($died) { "" } else { Until { param($x) Finished $x } 30 }
    Check "an unpaired-surrogate file name chosen in the dialog does not end sotyware" ($ok -and -not $died) "exit code $code"
    if (-not $died) {
      Check "it is skipped as 'name cannot be an object key' and the valid file is uploaded" ("$t".Contains("Upload finished. 2 files: 1 uploaded, 1 skipped.") -and (UpRow "lone-").Contains("Skipped: name cannot be an object key") -and (StoredOk (Obj (State) "inbox/z-valid.txt") $b11)) "$t"
      UpDismiss
    } else {
      $st = State
      Check "after the abort: nothing of batch 11 stored, no multipart upload" (-not $st.Objects.ContainsKey("inbox/z-valid.txt") -and $st.Multipart.Count -eq 0) (KeysUnder $st "inbox/")
      $ok = LaunchToBucket
      if ($script:app) { $p = $script:app }
      Check "relaunch after the abort: the account is still there and its bucket opens" $ok (PageText)
    }
   } catch {
    Check "upload steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    foreach ($title in $OpenTitle, "Save download as") { $d = DialogNamed $title; if ($d) { CancelDialog $d } }
    $null = Press "Cancel upload"; $null = Press "Dismiss"
   } finally {
    if ($locker) { $locker.Close() }
    try { UpCleanup } catch { Check "upload cleanup ran" $false "$($_.Exception.Message)" }
   }
  } elseif ($Mode -eq "b2-upload") {
   try {
    # The same steps against Backblaze and in the rehearsal; only the bucket
    # name and the key differ.
    Check "Open bucket lists the bucket's root, with the sotyware-it folder" ($ok -and ((TableRows) -contains "sotyware-it | Folder | $([char]0x2014)")) (PageText)
    $ok = (PressFolder "sotyware-it") -and (WaitFor { (PageText) -like "*upload-ui-validation*" } 60)
    $ok = $ok -and (PressFolder "upload-ui-validation") -and (WaitFor { (PageText) -like "*$run*" } 60)
    $ok = $ok -and (PressFolder $run) -and (WaitFor { ((PageText) -like "*| $UiPrefix |*") -and ((PageText) -notlike "*Loading*") } 60)
    Check "navigate sotyware-it, upload-ui-validation, this run's folder" $ok (PageText)
    $UiSeedLen = (Get-Item -LiteralPath $UiSeed).Length
    $rows = TableRows
    Check "the run folder shows exactly one row: the seeded existing.txt ($UiSeedLen B)" ($rows.Count -eq 1 -and $rows[0] -like "existing.txt | $(FormatSize $UiSeedLen) | *") "$($rows -join ' || ')"
    # Nothing is sent unless the panel names exactly the run folder while
    # the dialog is open (the destination is fixed when the dialog opens).
    $script:destSeen = ""; $script:destOk = $false; $script:seen.Clear()
    $picked = $ok -and (Pick $UiBatch $UiNames "list" {
      $script:destSeen = UpPanel
      $script:destOk = $script:destSeen.Contains("$Bucket / $UiPrefix") -and -not $script:destSeen.Contains("Retrying")
      $script:destOk
    })
    # From here on the app may have uploaded something.
    if ($script:destOk) { $script:uiBatchRan = $true }
    Check "while the native dialog was open the panel named the run folder as the destination (checked before Open)" $script:destOk (Redact $script:destSeen)
    Check "native dialog: batch folder opened through File name + Open, the 4 files selected in its file list (UI Automation), Open" $picked
    $done = if ($picked) { Until { param($x) Finished $x } 900 } else { $null }
    $ur = UpRows
    "  file rows: $(Redact ($ur -join ' || '))"
    Check "final summary: Upload finished. 4 files: 3 uploaded, 1 skipped." ($done -and $done.Contains("Upload finished. 4 files: 3 uploaded, 1 skipped.")) (Redact "$done")
    $oneRowEach = $ur.Count -eq 4; foreach ($n in $UiNames) { $oneRowEach = $oneRowEach -and @($ur | Where-Object { $_.StartsWith($n, [StringComparison]::Ordinal) }).Count -eq 1 }
    Check "4 rows, one per chosen file, names exactly as on disk" $oneRowEach (Redact ($ur -join " || "))
    Check "a-small.txt, $UiUni and progress.bin: Uploaded and verified" (@("a-small.txt", $UiUni, "progress.bin" | Where-Object { -not (UpRow $_).Contains("Uploaded and verified") }).Count -eq 0) (Redact ($ur -join " || "))
    Check "existing.txt: Skipped: already in this folder (left as it is)" ((UpRow "existing.txt").Contains("Skipped: already in this folder") -and (UpRow "existing.txt").Contains("left as it is")) (UpRow "existing.txt")
    Check "nothing to retry: no Try again button" (-not ((PageText) -match 'Try \d+ files? again')) (PageText)
    $stages = @("Computing the SHA-256 checksum", "Sending $([char]0xB7) ", "Verifying the stored object" | Where-Object { $s1 = $_; @($script:seen | Where-Object { $_.Contains($s1) }).Count -gt 0 })
    $sendSamples = @($script:seen | ForEach-Object { [regex]::Match($_, 'progress\.bin[^|]*?Sending \u00b7 (\d+)%') } | Where-Object { $_.Success } | ForEach-Object { [int]$_.Groups[1].Value })
    "  stages seen in the panel ($($script:seen.Count) distinct samples, $script:polls polls): $($stages -join ', '); progress.bin Sending percentages seen: $($sendSamples -join ' ')"
    Check "progress: progress.bin was seen Sending with a percentage" ($sendSamples.Count -ge 1) ($stages -join ", ")
    $mono = $true; for ($i = 1; $i -lt $sendSamples.Count; $i++) { if ($sendSamples[$i] -lt $sendSamples[$i - 1]) { $mono = $false } }
    Check "progress.bin's percentage never went down" $mono ($sendSamples -join " ")
    # No local path reaches the page: not the folder, not the profile.
    $leaks = @(@($script:seen) + @(PageText) | Where-Object { $_.Contains($UpSrc) -or $_.Contains($env:USERPROFILE) -or $_.Contains($env:TEMP) -or $_ -match '[A-Za-z]:\\' })
    Check "no local path on the page or in any panel text seen (folder, profile, a drive letter)" ($leaks.Count -eq 0) "$($leaks.Count) text(s)"
    $ok = (Press "Show destination folder") -and (WaitFor { ((PageText) -like "*| $UiPrefix |*") -and ((PageText) -notlike "*Loading*") -and (TableRows).Count -eq 4 } 60)
    $rows = TableRows
    "  destination folder rows: $(Redact ($rows -join ' || '))"
    $want = @{ "existing.txt" = $UiSeedLen }
    foreach ($n in "a-small.txt", $UiUni, "progress.bin") { $want[$n] = (Get-Item -LiteralPath (Join-Path $UiBatch $n)).Length }
    $rowsOk = $ok -and $rows.Count -eq 4
    foreach ($n in $want.Keys) { $rowsOk = $rowsOk -and @($rows | Where-Object { $_ -like "$(SameText $n) | $(FormatSize $want[$n]) | *" }).Count -eq 1 }
    Check "Show destination folder lists exactly the four objects; existing.txt keeps the seed's size" $rowsOk (Redact ($rows -join " || "))
    UpDismiss
   } catch {
    Check "b2-upload steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    $d = DialogNamed $OpenTitle; if ($d) { CancelDialog $d }
    $null = Press "Cancel upload"; $null = Press "Dismiss"
   }
  } elseif ($IsLc) {
   $locker = $null
   try {
    # The same steps against Backblaze and in the rehearsal; only the bucket
    # name and the key differ.
    Check "Open bucket lists the bucket's root, with the sotyware-it folder" ($ok -and ((TableRows) -contains "sotyware-it | Folder | $([char]0x2014)")) (PageText)
    $ok = (PressFolder "sotyware-it") -and (WaitFor { (PageText) -like "*upload-lifecycle-validation*" } 60)
    $ok = $ok -and (PressFolder "upload-lifecycle-validation") -and (WaitFor { (PageText) -like "*$LcRun*" } 60)
    $ok = $ok -and (PressFolder $LcRun) -and (WaitFor { ((PageText) -like "*| $UiPrefix |*") -and ((PageText) -notlike "*Loading*") } 60)
    Check "navigate sotyware-it, upload-lifecycle-validation, this run's folder ($LcRun)" $ok (PageText)
    $UiSeedLen = (Get-Item -LiteralPath $UiSeed).Length
    $rows = TableRows
    Check "the run folder shows exactly one row: anchor.txt ($UiSeedLen B)" ($rows.Count -eq 1 -and $rows[0] -like "anchor.txt | $(FormatSize $UiSeedLen) | *") "$($rows -join ' || ')"
    if ($Scenario -eq "retry") { $locker = [System.IO.File]::Open((Join-Path $UiBatch "b-locked.txt"), "Open", "Read", "None") }
    # Nothing is sent unless the panel names exactly the run folder while
    # the dialog is open (the destination is fixed when the dialog opens).
    $script:destSeen = ""; $script:destOk = $false; $script:seen.Clear()
    $picked = $ok -and (Pick $UiBatch $UiNames "list" {
      $script:destSeen = UpPanel
      $script:destOk = $script:destSeen.Contains("$Bucket / $UiPrefix") -and -not $script:destSeen.Contains("Retrying")
      $script:destOk
    })
    if ($script:destOk) { $script:uiBatchRan = $true }
    Check "while the native dialog was open the panel named the run folder as the destination (checked before Open)" $script:destOk (Redact $script:destSeen)
    Check "native dialog: the batch folder opened, its $($UiNames.Count) file(s) selected in the file list (UI Automation), Open" $picked
    $big = $UiNames[0]
    switch ($Scenario) {
      "multipart" {
        # While it is sent, the provider must hold one open multipart upload
        # for it (the release settings chose multipart, not one PUT).
        $mid = if ($picked) { Until { param($x) (SendingIn $x $big 10) -or (Finished $x) } 1800 } else { $null }
        $open = if ($mid -and -not (Finished $mid)) { LcOpenUploads } else { $null }
        "  open multipart uploads while sending: $open"
        Check "while $big was being sent (10% or more), exactly one multipart upload was open, for it" ($open -eq "1:$big" -or ($Rehearse -and $open -like "1:*")) "$open | $(Redact "$mid")"
        $done = if ($picked) { Until { param($x) Finished $x } 1800 } else { $null }
        $ur = UpRows; "  file rows: $(Redact ($ur -join ' || '))"
        Check "final summary: Upload finished. 1 file: 1 uploaded." ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.")) (Redact "$done")
        Check "${big}: Uploaded and verified" ((UpRow $big).Contains("Uploaded and verified")) (UpRow $big)
        $stages = @("Computing the SHA-256 checksum", "Sending $([char]0xB7) ", "Verifying the stored object" | Where-Object { $s1 = $_; @($script:seen | Where-Object { $_.Contains($s1) }).Count -gt 0 })
        $sendSamples = @($script:seen | ForEach-Object { [regex]::Match($_, [regex]::Escape($big) + '[^|]*?Sending \u00b7 (\d+)%') } | Where-Object { $_.Success } | ForEach-Object { [int]$_.Groups[1].Value })
        "  stages seen ($($script:seen.Count) distinct samples, $script:polls polls): $($stages -join ', '); Sending percentages: $($sendSamples -join ' ')"
        Check "progress: $big was seen Sending with at least two different percentages" (@($sendSamples | Sort-Object -Unique).Count -ge 2) ($sendSamples -join " ")
        $mono = $true; for ($i = 1; $i -lt $sendSamples.Count; $i++) { if ($sendSamples[$i] -lt $sendSamples[$i - 1]) { $mono = $false } }
        Check "${big}'s percentage never went down" $mono ($sendSamples -join " ")
        $ok = (Press "Show destination folder") -and (WaitFor { ((PageText) -like "*| $UiPrefix |*") -and ((PageText) -notlike "*Loading*") -and (TableRows).Count -eq 2 } 60)
        $rows = TableRows
        $len = (Get-Item -LiteralPath (Join-Path $UiBatch $big)).Length
        Check "Show destination folder lists anchor.txt and $big ($(FormatSize $len))" ($ok -and @($rows | Where-Object { $_ -like "$big | $(FormatSize $len) | *" }).Count -eq 1 -and @($rows | Where-Object { $_ -like "anchor.txt | *" }).Count -eq 1) (Redact ($rows -join " || "))
        UpDismiss
      }
      "cancel" {
        $running = $picked -and (Until { param($x) (SendingIn $x $big 20) -or (Finished $x) } 1800) -and (SendingIn $script:untilText $big 20)
        $at = [regex]::Match("$script:untilText", 'Sending \u00b7 (\d+)%').Groups[1].Value
        $pressed = $running -and (Press "Cancel upload")
        $done = if ($picked) { Until { param($x) Finished $x } 300 } else { $null }
        "  Cancel upload pressed at $at%; file row: $(Redact (UpRow $big))"
        Check "Cancel upload during the multipart upload (at 20% or more): Upload cancelled. 1 file: 0 uploaded, 1 cancelled." ($pressed -and $done -and $done.Contains("Upload cancelled. 1 file: 0 uploaded, 1 cancelled.")) (Redact "$done")
        Check "the row says cancelled during its upload, nothing stored; no 'may remain'" ((UpRow $big).Contains("Cancelled during its upload") -and (UpRow $big).Contains("Nothing was stored for this file.") -and -not (UpRow $big).Contains("may remain")) (UpRow $big)
        Check "Try 1 file again is offered (cancelled, nothing stored); not pressed" (Btn "Try 1 file again") (PageText)
        UpDismiss
      }
      "retry" {
        $done = if ($picked) { Until { param($x) Finished $x } 600 } else { $null }
        $ur = UpRows; "  file rows: $(Redact ($ur -join ' || '))"
        Check "first batch: Upload finished. 4 files: 2 uploaded, 1 skipped, 1 failed." ($done -and $done.Contains("Upload finished. 4 files: 2 uploaded, 1 skipped, 1 failed.")) (Redact "$done")
        $lr = UpRow "b-locked.txt"
        Check "b-locked.txt (held open without sharing): Failed, could not be read on this computer, nothing stored" ($lr.Contains("Failed") -and ($lr.Contains("could not be read on this computer") -or $lr.Contains("may not read this file")) -and $lr.Contains("Nothing was stored for this file.")) $lr
        Check "a-first.txt and c-third.txt: Uploaded and verified (the batch went on after the failure)" ((UpRow "a-first.txt").Contains("Uploaded and verified") -and (UpRow "c-third.txt").Contains("Uploaded and verified")) (Redact ($ur -join " || "))
        Check "anchor.txt: Skipped: already in this folder (left as it is)" ((UpRow "anchor.txt").Contains("Skipped: already in this folder")) (UpRow "anchor.txt")
        Check "Try again offers exactly 1 file: not the uploaded ones, not the skipped one" ((Btn "Try 1 file again") -and -not (Btn "Try 2 files again") -and -not (Btn "Try 3 files again")) (PageText)
        $locker.Close(); $locker = $null
        $script:seen.Clear()
        $null = Press "Try 1 file again"
        $done = Until { param($x) Retried $x } 600
        $ur = UpRows; "  file rows after Try again: $(Redact ($ur -join ' || '))"
        Check "Try again: Upload finished. 1 file: 1 uploaded.; only b-locked.txt in the new batch" ($done -and $done.Contains("Upload finished. 1 file: 1 uploaded.") -and $ur.Count -eq 1 -and (UpRow "b-locked.txt").Contains("Uploaded and verified") -and -not $done.Contains("a-first.txt") -and -not $done.Contains("c-third.txt") -and -not $done.Contains("anchor.txt")) (Redact "$done")
        UpDismiss
      }
      "leave" {
        $running = $picked -and (Until { param($x) (SendingIn $x $big 10) -or (Finished $x) } 1800) -and (SendingIn $script:untilText $big 10)
        $at = [regex]::Match("$script:untilText", 'Sending \u00b7 (\d+)%').Groups[1].Value
        $left = $running -and (Press "$([char]0x2190) Back to accounts") -and (WaitFor { Btn "Open bucket" } 15)
        "  Back to accounts pressed at $at%"
        Check "Back to accounts during the one-PUT upload of $big (at 10% or more): the account list is shown" $left (PageText)
        # Longer than the rest of the PUT would take at the rehearsal's rate;
        # at Backblaze the verify test then lists what is left.
        Start-Sleep 35
        $ok = (Press "Open bucket") -and (WaitFor { $t = PageText; $t.Contains("Root of the bucket") -and $t.Contains("sotyware-it") } 60)
        Check "back in the browser: no upload panel, and Upload files$([char]0x2026) is enabled (the slot is free)" ($ok -and (UpPanel) -eq "" -and (UploadEnabled)) (UpPanel)
      }
      "settle" {
        # The link takes each part at once, so the percentage moves once
        # per part, when that part has been handed over completely. Cancel
        # right after it moves: that part is then still being forwarded.
        $running = $picked -and (Until { param($x) (SendingIn $x $big 20) -or (Finished $x) } 1800) -and (SendingIn $script:untilText $big 20)
        $settleFrom = [regex]::Match("$script:untilText", 'Sending \u00b7 (\d+)%').Groups[1].Value
        $moved = $running -and (Until { param($x) (Finished $x) -or (SendingIn $x $big ([int]$settleFrom + 1)) } 60) -and -not (Finished $script:untilText)
        $settleTo = [regex]::Match("$script:untilText", 'Sending \u00b7 (\d+)%').Groups[1].Value
        $script:settlePressMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $pressed = $moved -and (Press "Cancel upload")
        $script:settlePressedMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $done = if ($picked) { Until { param($x) Finished $x } 300 } else { $null }
        "  Cancel upload pressed as the percentage moved from $settleFrom% to $settleTo% (Invoke took $($script:settlePressedMs - $script:settlePressMs) ms); file row: $(Redact (UpRow $big))"
        Check "Cancel upload right after a new part was handed over: Upload cancelled. 1 file: 0 uploaded, 1 cancelled." ($pressed -and $done -and $done.Contains("Upload cancelled. 1 file: 0 uploaded, 1 cancelled.")) (Redact "$done")
        Check "the row says cancelled during its upload, nothing stored; no 'may remain'" ((UpRow $big).Contains("Cancelled during its upload") -and (UpRow $big).Contains("Nothing was stored for this file.") -and -not (UpRow $big).Contains("may remain")) (UpRow $big)
        UpDismiss
      }
      "quit" {
        $running = $picked -and (Until { param($x) (SendingIn $x $big 20) -or (Finished $x) } 1800) -and (SendingIn $script:untilText $big 20)
        $at = [regex]::Match("$script:untilText", 'Sending \u00b7 (\d+)%').Groups[1].Value
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $quitSent = $running -and (TrayQuit)
        $exited = $quitSent -and $p.WaitForExit(45000)
        $secs = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
        if (OverflowOpen) { [CloudUiW]::Esc() }
        if (-not $exited) { Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }; $null = WaitFor { (Procs).Count -eq 0 } 10 }
        "  tray Quit at $at%; to exit: $secs s"
        Check "tray Quit (real mouse) during the multipart upload (at 20% or more): exit 0 within 35 s (the documented bound for aborts on quit)" ($running -and $exited -and $p.ExitCode -eq 0 -and $secs -le 35) "sent=$quitSent $script:why exit=$(if ($exited) { $p.ExitCode }) $secs s"
        $claimed = @($script:seen | Where-Object { $_.Contains("Upload finished.") -or $_.Contains("Uploaded and verified") })
        Check "before it ended, the panel never claimed the upload finished or the file uploaded" ($claimed.Count -eq 0) (Redact ($claimed -join " || "))
        # Relaunch, so the account can be removed through the page again.
        $p = Start-Process $Exe -PassThru; $null = $p.Handle
        Verified $p "relaunch" | Out-Host
        $ok = (WaitFor { $null -ne (Dash) } 20) -and (WaitFor { Link "Cloud Storage" } 15) -and (WaitFor { Btn "Open bucket" } 15) -and
          (Press "Open bucket") -and (WaitFor { $t = PageText; $t.Contains("Root of the bucket") -and $t.Contains("sotyware-it") } 60)
        Check "relaunch: the account is still listed and its bucket opens; no upload panel" ($ok -and (UpPanel) -eq "") (PageText)
      }
    }
   } catch {
    Check "b2-lifecycle steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    $d = DialogNamed $OpenTitle; if ($d) { CancelDialog $d }
    $null = Press "Cancel upload"; $null = Press "Dismiss"
   } finally {
    if ($locker) { $locker.Close() }
   }
  } elseif ($Mode -eq "b2-download") {
   try {
    $R = $Dl.run
    Check "Open bucket lists the bucket's root, with the sotyware-it folder" ($ok -and ((TableRows) -contains "sotyware-it | Folder | $([char]0x2014)")) (PageText)
    $ok = (PressFolder "sotyware-it") -and (WaitFor { (PageText) -like "*download-ui-validation*" } 60)
    $ok = $ok -and (PressFolder "download-ui-validation") -and (WaitFor { (PageText) -like "*$R*" } 60)
    $ok = $ok -and (PressFolder $R) -and (WaitFor { ((PageText) -like "*| sotyware-it/download-ui-validation/$R/ |*") -and ((PageText) -notlike "*Loading*") } 60)
    Check "navigate sotyware-it, download-ui-validation, this run's folder" $ok (PageText)
    $rows = TableRows
    Check "the run folder shows exactly one row: '$DlName', 16 MiB" ($rows.Count -eq 1 -and $rows[0] -like "$DlName | 16 MiB | *") "$($rows -join ' || ')"
    $dlg = OpenSave $DlName
    Check "Download opens the native Windows save dialog, '$DlName' suggested unchanged" ($null -ne $dlg -and (Suggested $dlg) -eq $DlName) "$(if ($dlg) { Suggested $dlg } else { 'no dialog' }) | $(PageText)"
    $dest = Join-Path $Out $DlName
    $saved = $dlg -and (SaveAs $dlg $dest)
    Check "save into a new folder outside the repository ($Out)" ([bool]$saved)
    # Samples the progress line until the final message; the page shows
    # percent and rounded sizes (the exact byte counts stay in the channel).
    $samples = New-Object System.Collections.Generic.List[string]
    $final = $null; $atFinal = $null; $partialSeen = $false
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($saved -and $sw.Elapsed.TotalSeconds -lt 600) {
      $t = PageText
      $m = [regex]::Match($t, '(\d+)% \u00b7 ([\d.]+ (?:B|KiB|MiB)) of ([\d.]+ (?:B|KiB|MiB))')
      if ($m.Success -and ($samples.Count -eq 0 -or $samples[$samples.Count - 1] -ne $m.Value)) { $samples.Add($m.Value) }
      if (-not $partialSeen) { $partialSeen = @(Saved | Where-Object { $_ -like ".*.sotyware-partial" }).Count -gt 0 }
      if ($t -cmatch 'Downloaded \d' -or $t -like "*Download failed*" -or $t -like "*Download cancelled*") {
        # Taken at once: the file must already be in place at full size.
        $atFinal = @{ len = [CloudUiW]::Length($dest); names = (Saved) -join "|" }
        $final = $t; break
      }
      Start-Sleep -Milliseconds 40
    }
    $secs = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
    "  progress lines seen ($($samples.Count), $secs s until the final message): $($samples -join ' ; ')"
    "  temporary file seen while downloading: $partialSeen"
    if ($final) { "  final: $([regex]::Match($final, '(Downloaded \d|Download failed|Download cancelled)[^|]*(\| [^|]*)?').Value.Trim())" }
    Check "progress was shown with the total size (Content-Length): '... of 16 MiB'" ($samples.Count -ge 1 -and @($samples | Where-Object { $_ -notlike "* of 16 MiB" }).Count -eq 0) "$($samples -join ' ; ')"
    function ToBytes([string]$v) { $n, $u = $v.Split(" "); [double]::Parse($n, [Globalization.CultureInfo]::InvariantCulture) * @{ "B" = 1; "KiB" = 1024; "MiB" = 1048576 }[$u] }
    $mono = $true
    for ($i = 1; $i -lt $samples.Count; $i++) {
      # Not $a: names ignore case, and $A is the AutomationElement type.
      $prev = [regex]::Match($samples[$i - 1], '^(\d+)% \u00b7 (.+?) of'); $next = [regex]::Match($samples[$i], '^(\d+)% \u00b7 (.+?) of')
      if ([int]$next.Groups[1].Value -lt [int]$prev.Groups[1].Value -or (ToBytes $next.Groups[2].Value) -lt (ToBytes $prev.Groups[2].Value)) { $mono = $false }
    }
    Check "progress never went down (percent and bytes shown)" $mono "$($samples -join ' ; ')"
    Check "final message: Downloaded 16 MiB, the SHA-256 stored with the file matched (sha256-verified)" ($final -and $final.Contains("Downloaded 16 MiB.") -and $final.Contains("The SHA-256 checksum stored with the file matched.")) "$final"
    Check "when completion was shown, the file was already in place at $($Dl.size) bytes and no temporary file was left" ($atFinal -and $atFinal.len -eq [long]$Dl.size -and $atFinal.names -eq $DlName) "$(if ($atFinal) { "len=$($atFinal.len) files=$($atFinal.names)" })"
    $null = Press "Dismiss"
    $len = [CloudUiW]::Length($dest)
    $h = if ($len -ge 0) { Sha $dest } else { "" }
    "  saved file: $len bytes, SHA-256 $h"
    Check "saved file: exactly $($Dl.size) bytes" ($len -eq [long]$Dl.size)
    Check "saved file: SHA-256 equals the one computed before the upload" ($h -eq $Dl.sha256) $h
    Check "saved file: byte-for-byte equal to the generated original" ($len -ge 0 -and [CloudUiW]::SameBytes($dest, $Source))
    Check "the destination folder holds only the file (no .sotyware-partial)" (((Saved) -join "|") -eq $DlName) "$(Saved)"
   } catch {
    Check "b2-download steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    $d = DialogNamed "Save download as"; if ($d) { CancelDialog $d }
    $null = Press "Cancel download"; $null = Press "Dismiss"
   }
  } elseif ($Mode -eq "b2") {
    $outcome = if ($t -like "*the provider refused this request*") { "access denied" } elseif ($t -like "*Could not load this folder.*") { "error" } else { "listed" }
    "  Open bucket at the root: $outcome"
    Check "Open bucket at the root: a listing, or Access denied for the prefix-restricted key" ($ok -and ($outcome -eq "listed" -or $outcome -eq "access denied") -and ($t -notlike "*$Secret1*")) $t
  } else {
    Check "Open bucket lists the root: folders, files, size, bucket, account" ($ok -and ($t -like "*Root of the bucket*") -and ($t -like "*readme.txt*") -and ($t -like "*12 B*") -and ($t -like "*Ferien 2024 *") -and ($t -like "*$label*") -and ($t -notlike "*cat.jpg*") -and ($t -notlike "*item-0000*") -and $null -eq (Named "Add account" $CT::Button)) $t
    $ok = (PressFolder "Ferien 2024 $([char]0xFC)") -and (WaitFor { (PageText) -like "*Strand Bild (1).jpg*" } 20)
    $t = PageText
    Check "open a folder with spaces and Unicode: its files, exact names" ($ok -and ($t -like "*a+b=c&d.txt*") -and ($t -notlike "*readme.txt*")) $t
    $ok = (PressFolder $Bucket) -and (WaitFor { ((PageText) -like "*readme.txt*") -and ((PageText) -like "*Root of the bucket*") } 20)
    Check "the bucket breadcrumb goes back to the root" $ok (PageText)
    $ok = (PressFolder "photos") -and (WaitFor { (PageText) -like "*cat.jpg*" } 20)
    $t = PageText
    Check "open photos/: subfolder and file, not the nested file, not the marker" ($ok -and ($t -like "*2024*") -and ($t -notlike "*beach.jpg*") -and ($t -like "*2.0 KiB*")) $t
    $ok = (PressFolder "2024") -and (WaitFor { (PageText) -like "*beach.jpg*" } 20)
    Check "open photos/2024/: the nested file with its size" ($ok -and ((PageText) -like "*3.0 MiB*")) (PageText)
    $ok = (Press "Up one folder") -and (WaitFor { ((PageText) -like "*cat.jpg*") -and ((PageText) -notlike "*beach.jpg*") } 20)
    Check "Up one folder goes to photos/" $ok (PageText)
    $ok = (PressFolder $Bucket) -and (WaitFor { (PageText) -like "*readme.txt*" } 20)
    $ok = $ok -and (PressFolder "empty") -and (WaitFor { (PageText) -like "*This folder is empty.*" } 20)
    Check "a folder that is only a marker opens as empty" $ok (PageText)
    $ok = (PressFolder $Bucket) -and (WaitFor { (PageText) -like "*readme.txt*" } 20)
    $ok = $ok -and (PressFolder "many") -and (WaitFor { (PageText) -like "*1000 items shown; more are available*" } 120)
    $t = PageText
    Check "a folder of 1,005 entries loads one page of 1,000 with Load more" ($ok -and ($t -like "*item-0999.txt*") -and ($t -notlike "*item-1000.txt*") -and $null -ne (Named "Load more" $CT::Button)) ""
    $ok = (Press "Load more") -and (WaitFor { (PageText) -like "*1005 items shown.*" } 120)
    Check "Load more appends the rest, no Load more after the last page" ($ok -and ((PageText) -like "*item-1004.txt*") -and $null -eq (Named "Load more" $CT::Button)) ""
    $ok = (Press "Refresh") -and (WaitFor { (PageText) -like "*1000 items shown; more are available*" } 120)
    Check "Refresh starts again from the first page" $ok ""
  }
  $ok = (Press "$([char]0x2190) Back to accounts") -and (WaitFor { $null -ne (Named "Check connection" $CT::Button) } 15)
  Check "Back to accounts shows the account list again" $ok (PageText)

  $ok = (Press "Edit") -and (WaitFor { $null -ne (Named "Save changes" $CT::Button) } 10) -and (Fill "Name" $renamed) -and (Press "Save changes")
  Check "rename without re-entering the key" ($ok -and (WaitFor { (PageText) -like "*Saved $renamed*" } 30) -and [CloudUiW]::Matches((Target $id), (Stored $Key1 $Secret1)) -eq 1) (PageText)
  if ($Mode -eq "b2") { $st = RunCheck; Check "after the rename: still Connected" ($st -eq "Connected") "$st | $(PageText)" }

  # Not $s: PowerShell names ignore case, and $S is the TreeScope type.
  function ReplaceKey($k, $sec) {
    $ok = (Press "Edit") -and (WaitFor { $null -ne (Named "Save changes" $CT::Button) } 10)
    $box = Named "Replace access key" $CT::CheckBox
    $ok = $ok -and ($null -ne $box)
    if ($ok) { $box.GetCurrentPattern($Toggle).Toggle() }
    $ok = $ok -and (WaitFor { (Fields) -contains "Secret access key" } 10) -and (Fill "Access key ID" $k) -and (Fill "Secret access key" $sec) -and (Press "Save changes")
    $ok -and (WaitFor { ((PageText) -like "*Saved $renamed*") -and $null -eq (Named "Save changes" $CT::Button) } 30)
  }
  $ok = ReplaceKey $Key2 $Secret2
  Check "replace the key: the entry now holds the new key" ($ok -and [CloudUiW]::Matches((Target $id), (Stored $Key2 $Secret2)) -eq 1) (PageText)
  Check "still one entry for the account" ((CmdkeyEntries (Target $id)) -eq 1)
  if ($Mode -eq "b2") {
    $st = RunCheck; Check "the replaced (wrong) key is the one used: Invalid credentials" ($st -eq "Invalid credentials") "$st | $(PageText)"
    $ok = ReplaceKey $Key1 $Secret1
    Check "replace again with the real key" ($ok -and [CloudUiW]::Matches((Target $id), (Stored $Key1 $Secret1)) -eq 1)
    $st = RunCheck; Check "after the replacement: Connected again" ($st -eq "Connected") "$st | $(PageText)"
  }

  $ok = (Press "Remove") -and (WaitFor { $null -ne (Named "Remove account" $CT::Button) } 10)
  $confirm = PageText
  Check "removal asks first and says the bucket and its files stay" ($ok -and ($confirm -like "*not*deleted or changed*") -and ($confirm -like "*$Bucket*")) $confirm
  $ok = (Press "Remove account") -and (WaitFor { (PageText) -like "*Removed $renamed*" } 30)
  Check "remove: gone from the page and from Credential Manager" ($ok -and [CloudUiW]::Matches((Target $id), "") -eq -1 -and (WaitFor { (PageText) -like "*No storage accounts yet*" } 10)) (PageText)
  Check "cmdkey no longer lists the entry" ((CmdkeyEntries (Target $id)) -eq 0)
  $p.Refresh()
  Check "process still running and responding" (-not $p.HasExited -and $p.Responding)
  if ($RecordRequests) {
    # A recording is complete only after an orderly exit (RunEvent::Exit
    # writes its end line); the finally below would kill the app. In tray
    # mode the orderly exit is the tray Quit, with the real mouse.
    $quitSent = TrayQuit
    $exited = $quitSent -and $p.WaitForExit(45000)
    Check "request recorder: tray Quit (real mouse) ends the app with exit 0, so it can write its end line" ($exited -and $p.ExitCode -eq 0) "sent=$quitSent $script:why"
  }
} finally {
  Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
  $null = WaitFor { (Procs).Count -eq 0 } 5
  ProfileCleanup
  if ($Mode -eq "b2-upload" -and -not $Rehearse) {
    UploadUiFinish
    "test program after the verification: $(TestExe)"
  }
  if ($IsLc -and -not $Rehearse) {
    LifecycleFinish
    "test program after the verification: $(TestExe)"
  }
  if ($IsLc -and $Rehearse) {
    # The provider-side checks of the Backblaze run, on the server's state.
    try {
      $st = State
      foreach ($n in $UiNames) { [void]$script:created.Add("$UiPrefix$n") }
      $want = @("${UiPrefix}anchor.txt") + @($LcStored | ForEach-Object { "$UiPrefix$_" })
      Check "rehearsal: the server holds exactly the anchor$(if ($LcStored.Count) { ' and ' + ($LcStored -join ', ') }) under the run prefix, nothing elsewhere" ((KeysUnder $st $UiPrefix) -eq (SortedKeys $want) -and $st.Objects.Count -eq $want.Count) (Redact (SortedKeys $st.Objects.Keys))
      $contentOk = (Obj $st "${UiPrefix}anchor.txt") -and (Obj $st "${UiPrefix}anchor.txt").sha256 -eq (UpSha $UiSeed)
      foreach ($n in $LcStored) { $contentOk = $contentOk -and (StoredOk (Obj $st "$UiPrefix$n") (Join-Path $UiBatch $n)) }
      Check "rehearsal: stored contents, sizes and SHA-256 metadata equal the local files; anchor.txt is still the seed; no multipart upload left" ($contentOk -and $st.Multipart.Count -eq 0) "$($st.Multipart -join ',')"
      UpCleanup
    } catch { Check "rehearsal: server state read and cleaned up" $false "$($_.Exception.Message)" }
  }
  if ($Mode -eq "b2-upload" -and $Rehearse) {
    # The provider-side checks of the Backblaze run, on the server's state.
    try {
      $st = State
      $under = KeysUnder $st $UiPrefix
      Check "rehearsal: the server holds exactly the four keys under the run prefix, nothing elsewhere" ($under -eq (SortedKeys @($UiNames | ForEach-Object { "$UiPrefix$_" })) -and $st.Objects.Count -eq 4) (Redact (SortedKeys $st.Objects.Keys))
      $contentOk = (Obj $st "${UiPrefix}existing.txt") -and (Obj $st "${UiPrefix}existing.txt").sha256 -eq (UpSha $UiSeed)
      foreach ($n in "a-small.txt", $UiUni, "progress.bin") { $contentOk = $contentOk -and (StoredOk (Obj $st "$UiPrefix$n") (Join-Path $UiBatch $n)); [void]$script:created.Add("$UiPrefix$n") }
      Check "rehearsal: uploaded contents, sizes and SHA-256 metadata equal the local files; existing.txt is still the seed; no multipart upload" ($contentOk -and $st.Multipart.Count -eq 0)
      UpCleanup
    } catch { Check "rehearsal: server state read and cleaned up" $false "$($_.Exception.Message)" }
  }
  if ($Mode -eq "b2-browse") {
    $fpMid2 = Fingerprint
    Check "b2-browse: browsing changed no object version" (($null -ne $fpMid) -and $fpMid -eq $fpMid2)
    BrowseCleanup
    Remove-Item $ManFile -EA SilentlyContinue
  }
  if ($Mode -eq "browse" -or $Mode -eq "download" -or $Mode -eq "upload" -or ($IsB2Up -and $Rehearse) -or $Mode -eq "files") {
    # Removing the endpoint file stops the helper (and with it its temporary bucket).
    Remove-Item $SrvFile, "$SrvFile.json", "$SrvFile.state.json", "$SrvFile.ctl" -EA SilentlyContinue
    if (-not $srv.WaitForExit(30000)) { Stop-Process -Id $srv.Id -EA SilentlyContinue }
  }
  # .NET, not Remove-Item: download mode saves a name with an unpaired surrogate.
  if (($Mode -eq "download" -or $Mode -eq "upload" -or $IsB2Up -or $Mode -eq "files") -and $Out -and (Test-Path $Out)) { [System.IO.Directory]::Delete($Out, $true) }
  if ($Mode -eq "b2-upload" -and (Test-Path -LiteralPath $UpSrc)) {
    [System.IO.Directory]::Delete($UpSrc, $true)
    Check "b2-upload: local test files removed ($UpSrc)" (-not (Test-Path -LiteralPath $UpSrc))
  }
  if ($IsLc -and (Test-Path -LiteralPath $UpSrc)) {
    [System.IO.Directory]::Delete($UpSrc, $true)
    Check "b2-lifecycle: local test files removed ($UpSrc)" (-not (Test-Path -LiteralPath $UpSrc))
  }
  if ($Mode -eq "upload" -and (Test-Path -LiteralPath $UpSrc)) {
    # .NET, not Remove-Item: the folder holds a name with an unpaired surrogate.
    [System.IO.Directory]::Delete($UpSrc, $true)
    Check "upload: local test files removed ($UpSrc, $Out)" (-not (Test-Path -LiteralPath $UpSrc) -and -not (Test-Path -LiteralPath $Out))
  }
  if ($Mode -eq "b2-download") {
    DownloadCleanup
    $partials = @(Get-ChildItem -Force -Recurse -Path @($Out, $SrcDir) -EA SilentlyContinue | Where-Object { $_.Name -like "*.sotyware-partial" })
    Remove-Item -Recurse -Force -Path @($Out, $SrcDir) -EA SilentlyContinue
    Check "b2-download: local files removed (downloaded file, generated original, both folders; no .sotyware-partial)" ($partials.Count -eq 0 -and -not (Test-Path $Out) -and -not (Test-Path $SrcDir))
  }
}
if ($Mode -eq "browse" -or $Mode -eq "download" -or $Mode -eq "upload" -or ($IsB2Up -and $Rehearse) -or $Mode -eq "files") {
  $srvLog = (Get-Content $SrvOut -Raw -Encoding UTF8 -EA SilentlyContinue)
  $ops = [regex]::Match("$srvLog", 'operations: [^\r\n]*').Value
  if ($Mode -eq "b2-upload") {
    # The requests the Backblaze run expects from the app: the check, five
    # listings (root, three folders, Show destination folder), per uploaded
    # file two HeadObject before and one after its PutObject, one HeadObject
    # for the skipped file.
    $wantOps = 'operations: {"HeadBucket": 1, "HeadObject": 10, "ListObjectsV2": 5, "PutObject": 3}'
    Check "rehearsal: the app sent exactly HeadBucket 1, ListObjectsV2 5, HeadObject 10, PutObject 3 (19 requests)" ($ops.StartsWith($wantOps)) $ops
  }
  if ($IsLc) {
    # The app's requests by operation, as the Backblaze run expects them:
    # the check; listings of the root, sotyware-it/, the lifecycle root and
    # the run folder (plus Show destination folder, or the root again after
    # leaving or the relaunch); per file one HeadObject in the batch and
    # one in the adapter before sending, one after it (a multipart upload
    # one more, just before CompleteMultipartUpload).
    $n = @{}; foreach ($x in [regex]::Matches($ops, '"(\w+)": (\d+)')) { $n[$x.Groups[1].Value] = [int]$x.Groups[2].Value }
    function Ops([string]$op) { if ($n.ContainsKey($op)) { $n[$op] } else { 0 } }
    $parts = Ops "UploadPart"
    switch ($Scenario) {
      # Multipart: the adapter checks the key again before completing.
      "multipart" { $want = "HeadBucket 1, ListObjectsV2 5, HeadObject 4, CreateMultipartUpload 1, UploadPart 5, CompleteMultipartUpload 1, nothing else (17)"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 5 -and (Ops "HeadObject") -eq 4 -and (Ops "CreateMultipartUpload") -eq 1 -and $parts -eq 5 -and (Ops "CompleteMultipartUpload") -eq 1 -and $n.Count -eq 6 }
      "cancel" { $want = "HeadBucket 1, ListObjectsV2 4, HeadObject 2, CreateMultipartUpload 1, UploadPart 1 to 8, AbortMultipartUpload 1, nothing else"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 4 -and (Ops "HeadObject") -eq 2 -and (Ops "CreateMultipartUpload") -eq 1 -and $parts -ge 1 -and $parts -le 8 -and (Ops "AbortMultipartUpload") -eq 1 -and $n.Count -eq 6 }
      "settle" { $want = "HeadBucket 1, ListObjectsV2 4, HeadObject 2, CreateMultipartUpload 1, UploadPart 1 to 8, AbortMultipartUpload 1, nothing else"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 4 -and (Ops "HeadObject") -eq 2 -and (Ops "CreateMultipartUpload") -eq 1 -and $parts -ge 1 -and $parts -le 8 -and (Ops "AbortMultipartUpload") -eq 1 -and $n.Count -eq 6 }
      "quit" { $want = "HeadBucket 1, ListObjectsV2 5, HeadObject 2, CreateMultipartUpload 1, UploadPart 1 to 8, AbortMultipartUpload 1, nothing else"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 5 -and (Ops "HeadObject") -eq 2 -and (Ops "CreateMultipartUpload") -eq 1 -and $parts -ge 1 -and $parts -le 8 -and (Ops "AbortMultipartUpload") -eq 1 -and $n.Count -eq 6 }
      # anchor.txt 1 (exists: skipped); a-first, c-third and the retried
      # b-locked 3 each; b-locked in the first batch 1 (the batch's HEAD,
      # then the file cannot be opened).
      "retry" { $want = "HeadBucket 1, ListObjectsV2 4, HeadObject 11, PutObject 3, nothing else (19)"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 4 -and (Ops "HeadObject") -eq 11 -and (Ops "PutObject") -eq 3 -and $n.Count -eq 4 }
      "leave" { $want = "HeadBucket 1, ListObjectsV2 5, HeadObject 2, PutObject at most 1 (its body cut off), nothing for b-away.txt, nothing else"
        $okOps = (Ops "HeadBucket") -eq 1 -and (Ops "ListObjectsV2") -eq 5 -and (Ops "HeadObject") -eq 2 -and (Ops "PutObject") -le 1 -and $n.Count -le 4 -and ($n.Keys | Where-Object { $_ -notin "HeadBucket", "ListObjectsV2", "HeadObject", "PutObject" }).Count -eq 0 }
    }
    Check "rehearsal: the app sent $want" $okOps $ops
  }
  if ($Mode -eq "upload") { [regex]::Matches("$srvLog", '(?m)^(cleanup:|left at the end)[^\r\n]*') | ForEach-Object { "  $($_.Value)" } }
  $what = if ($Mode -eq "files") { "only file-manager requests, and no real version was deleted for good" } elseif ($Mode -eq "upload" -or $IsB2Up) { "only listing, reads and upload requests" } else { "only reads and the check" }
  Check "${Mode}: the test server saw $what ($ops)" (($srv.ExitCode -eq 0) -and ("$srvLog" -match 'test result: ok\. 1 passed')) "$srvLog $(Get-Content "$SrvOut.err" -Tail 15 -EA SilentlyContinue)"
  Remove-Item $SrvOut, "$SrvOut.err" -EA SilentlyContinue
}
if ($RecordRequests) {
  Remove-Item Env:SOTYWARE_REQUEST_RECORD_DIR, Env:SOTYWARE_REQUEST_RECORD_RUN, Env:SOTYWARE_REQUEST_RECORD_SCOPE, Env:SOTYWARE_REQUEST_RECORD_ACCOUNT -EA SilentlyContinue
  $recFiles = @(Get-ChildItem -LiteralPath $RecDir -Filter "*.requests.jsonl" -EA SilentlyContinue)
  $recTexts = @($recFiles | ForEach-Object { [System.IO.File]::ReadAllText($_.FullName) })
  $complete = @($recTexts | Where-Object { $l = @($_.TrimEnd("`n") -split "`n")[-1]; $_.EndsWith("`n") -and $l.Contains('"kind":"end"') -and $l.Contains('"accounting":"complete"') })
  Check "request recorder: $($recFiles.Count) recording file(s), each complete (end line, accounting complete; otherwise the count is unknown)" ($recFiles.Count -gt 0 -and $complete.Count -eq $recFiles.Count) "$RecDir"
  $leaks = @(@($Key1, $Secret1, $Key2, $Secret2, $Bucket, $WriteKey, $WriteSecret) | Where-Object { $_ } | Where-Object { $needle = $_; @($recTexts | Where-Object { $_.Contains($needle) }).Count -gt 0 })
  Check "request recorder: no key id, secret or bucket name in the recordings" ($leaks.Count -eq 0) "$($leaks.Count) found"
  $recSent = @{}
  foreach ($t in $recTexts) { foreach ($l in ($t -split "`n")) { if ($l.Contains('"kind":"attempt-start"') -and $l -match '"name":"(\w+)"') { $recSent[$Matches[1]] = 1 + [int]$recSent[$Matches[1]] } } }
  "  recorder: attempts sent by operation: $((@($recSent.Keys | Sort-Object) | ForEach-Object { "$_ $($recSent[$_])" }) -join ', ')"
  if ($Mode -eq "download" -or $Mode -eq "upload" -or ($IsB2Up -and $Rehearse)) {
    [regex]::Matches("$srvLog", '(?m)^recorder: [^\r\n]*') | ForEach-Object { "  $($_.Value)" }
    Check "request recorder: the recordings agree with the app's requests on the test server (the helper's own excluded)" ("$srvLog" -match '(?m)^recorder: every answered attempt reached the server') ([regex]::Match("$srvLog", 'MISMATCH[^\r\n]*').Value)
  }
  if ($Scenario -eq "settle") {
    # The ADR-025 wait, from the recording: absolute times are the start
    # line's started_unix_ms plus t_us (same clock as the press stamps).
    $sev = @(); $sBase = 0
    foreach ($t in $recTexts) {
      $lines = @($t -split "`n" | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
      if (@($lines | Where-Object { $_.name -eq "UploadPart" }).Count -gt 0) { $sBase = [long]$lines[0].started_unix_ms; $sev = $lines }
    }
    function EvMs($e) { $sBase + [long][Math]::Floor([double]$e.t_us / 1000) }
    $sParts = @(); $sAbort = @()
    foreach ($e in $sev) {
      if ($e.kind -eq "op-start" -and $e.name -eq "UploadPart") { $sParts += , @{ op = $e.op; part = $e.part; start = (EvMs $e); end = $null; endKind = ""; status = $null } }
      if ($e.kind -eq "op-start" -and $e.name -eq "AbortMultipartUpload") { $sAbort += , @{ op = $e.op; start = (EvMs $e); status = $null } }
      if ($e.kind -eq "op-end" -or $e.kind -eq "op-dropped") {
        foreach ($x in $sParts) { if ($x.op -eq $e.op) { $x.end = (EvMs $e); $x.endKind = $e.kind; $x.status = $e.status } }
        foreach ($x in $sAbort) { if ($x.op -eq $e.op) { $x.status = $e.status } }
      }
    }
    foreach ($x in $sParts) { "  recorder: UploadPart $($x.part): $($x.start - $script:settlePressMs) ms to $(if ($x.end) { $x.end - $script:settlePressMs } else { '?' }) ms after the press, $($x.endKind) $($x.status)" }
    foreach ($x in $sAbort) { "  recorder: AbortMultipartUpload at $($x.start - $script:settlePressMs) ms after the press, status $($x.status)" }
    $inFlight = @($sParts | Where-Object { $_.start -le $script:settlePressMs -and $_.end -and $_.end -ge $script:settlePressedMs })
    $cut = if ($inFlight.Count -eq 1) { $inFlight[0] } else { $null }
    Check "settle: exactly one part was in flight when Cancel upload was pressed (by the recording's clock)" ($null -ne $cut) "$($inFlight.Count) in flight"
    Check "settle: that part was answered, not dropped: op-end with 200, at least 500 ms after the press (the ADR-025 wait ran)" ($cut -and $cut.endKind -eq "op-end" -and $cut.status -eq 200 -and ($cut.end - $script:settlePressedMs) -ge 500) "$(if ($cut) { "part $($cut.part) $($cut.endKind) $($cut.status), $($cut.end - $script:settlePressedMs) ms after the press" })"
    Check "settle: no part dropped, none started after the press" (@($sParts | Where-Object { $_.endKind -ne "op-end" }).Count -eq 0 -and @($sParts | Where-Object { $_.start -gt $script:settlePressMs }).Count -eq 0)
    Check "settle: one AbortMultipartUpload, 204, started after that part's answer and within 1 s of it" ($cut -and $sAbort.Count -eq 1 -and $sAbort[0].status -eq 204 -and $sAbort[0].start -ge $cut.end -and ($sAbort[0].start - $cut.end) -le 1000) "$(if ($cut -and $sAbort.Count) { "$($sAbort[0].start - $cut.end) ms after, status $($sAbort[0].status)" })"
  }
  "  recordings kept in $RecDir"
}
$UserCredAfter = CmdkeyLines
Check "Credential Manager: the user's own sotyware entries are as before the run" (($UserCredAfter -join "|") -eq ($UserCredBefore -join "|")) "before=$($UserCredBefore.Count) after=$($UserCredAfter.Count)"
if ($Mode -eq "b2" -or $Mode -eq "b2-browse") {
  $fpAfter = Fingerprint
  if ($fpAfter) { "  after:  $($fpAfter -replace '^sotyware-it fingerprint ', '')" }
  Check "${Mode}: every object version under sotyware-it/ is as before the run" (($null -ne $fpAfter) -and $fpAfter -eq $fpBefore)
}
$Secret1 = $null; $Secret2 = $null; $c = $null; $WriteSecret = $null
""
"summary: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
