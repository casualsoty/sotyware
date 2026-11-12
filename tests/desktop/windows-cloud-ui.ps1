# Cloud Storage end-to-end test on native Windows (opt-in; not part of CI):
# drives the Cloud Storage page of the release binary through UI Automation
# (Invoke, Value and Toggle patterns; no mouse, no keystrokes) and checks the
# entries in the real Windows Credential Manager. Details:
# docs/development/testing.md#cloud-storage-ui-end-to-end-test
#
# usage (PowerShell, from the repository root on a Windows path):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-cloud-ui.ps1 [-Mode local|b2|browse|b2-browse|download|b2-download] [-Exe <path>]
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
#
# The account database (%LOCALAPPDATA%\com.sotyware.desktop\cloud-accounts.sqlite3)
# is moved aside first and put back at the end, as in windows.ps1; secret values
# are only compared, never shown. Needs a logged-in session (the WebView2 window
# must be created); it does not move the mouse.
# Exit code: 0 all passed, 1 a check failed, 2 binary missing, already running,
# or b2 credentials missing.
param(
  [ValidateSet("local", "b2", "browse", "b2-browse", "download", "b2-download")][string]$Mode = "local",
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\release\sotyware.exe"),
  [string]$Region = "eu-central-003"
)
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run tauri build -- --no-bundle)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
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
function Target($id) { "com.sotyware.desktop/cloud-account/$id" }
function Stored($key, $secret) { "sotyware-s3-credentials-v1`n$key`n$secret" }
function CmdkeyLines { @(cmdkey /list | Select-String -SimpleMatch "com.sotyware.desktop" | ForEach-Object { $_.Line.Trim() }) }
# Entries cmdkey lists for exactly this target. Only its "Target:" lines count:
# the header "Currently stored credentials for target: <target>" names the
# target even when nothing is stored.
function CmdkeyEntries($target) { @(cmdkey "/list:$target" | Where-Object { $_ -match '^\s*Target:' -and $_.TrimEnd().EndsWith($target) }).Count }

$cv = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
"sotyware Cloud Storage UI test ($Mode)  $(Get-Date -Format s)"
"os: $((Get-CimInstance Win32_OperatingSystem).Caption) $($cv.DisplayVersion) build $($cv.CurrentBuild).$($cv.UBR)"
"binary: $Exe"
if ((Procs).Count -gt 0) { "ABORT: sotyware from this binary is already running"; exit 2 }

$run = (Get-Date).ToUniversalTime().ToString("yyyyMMddHHmmssfff")
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
} elseif ($Mode -eq "browse" -or $Mode -eq "download") {
  # The made-up keys of serve_listing_for_ui / serve_objects_for_ui (src-tauri/src/cloud/test_server.rs).
  if ($Mode -eq "browse") {
    $Key1 = "e2eBrowseKey0000000001"; $Secret1 = "e2e-Browse-Secret-0123456789"; $Bucket = "browse-bucket"; $Helper = "serve_listing_for_ui"
  } else {
    $Key1 = "e2eDownloadKey00000001"; $Secret1 = "e2e-Download-Secret-0123456789"; $Bucket = "download-bucket"; $Helper = "serve_objects_for_ui"
  }
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $SrvFile = Join-Path $env:TEMP "sotyware-$Mode-endpoint-$run.txt"
  $SrvOut = Join-Path $env:TEMP "sotyware-$Mode-server-$run.log"
  $env:SOTYWARE_UI_SERVER_FILE = $SrvFile
  $srv = Start-Process cargo -ArgumentList "test", "--quiet", "--lib", "cloud::test_server::$Helper", "--", "--ignored", "--nocapture" `
    -WorkingDirectory (Join-Path $PSScriptRoot "..\..\src-tauri") -NoNewWindow -PassThru -RedirectStandardOutput $SrvOut -RedirectStandardError "$SrvOut.err"
  $null = $srv.Handle  # without a cached handle, ExitCode stays empty after the exit
  Remove-Item Env:SOTYWARE_UI_SERVER_FILE
  if (-not (WaitFor { (Test-Path $SrvFile) -and (Get-Content $SrvFile -Raw) } 900)) { "ABORT: the listing server did not start"; Get-Content "$SrvOut.err" -Tail 20; exit 2 }
  $Endpoint = (Get-Content $SrvFile -Raw).Trim()
  "test server: $Endpoint"
  if ($Mode -eq "download") {
    $script:objects = @{}; foreach ($o in (Get-Content "$SrvFile.json" -Raw -Encoding UTF8 | ConvertFrom-Json)) { $script:objects[[string]$o.key] = $o }
    $Out = Join-Path $env:TEMP "sotyware-downloads-$run"
    New-Item -ItemType Directory $Out | Out-Null
  }
} else {
  $Key1 = "e2eKeyIdOne000000000001"; $Secret1 = "e2e-Secret-One-0123456789abcdef"
  $Key2 = "e2eKeyIdTwo000000000002"; $Secret2 = "e2e-Secret-Two-fedcba9876543210"
  $Bucket = "soty-photos"; $Endpoint = "http://127.0.0.1:9"
}
$script:redact = @(@($Secret1, "<secret>"), @($Secret2, "<secret2>"), @($Key1, "<keyID>"), @($Key2, "<keyID2>"), @($Bucket, "<bucket>"))
if ($Mode -eq "b2-browse" -or $Mode -eq "b2-download") { $script:redact += , @($WriteSecret, "<write-secret>"); $script:redact += , @($WriteKey, "<write-keyID>") }

# Runs the opt-in Rust test that fingerprints the B2 bucket; the variables are
# set for that child only and removed again.
# Runs one opt-in Rust test with the B2 variables set for that child only
# (removed again afterwards); returns its output lines.
function CargoIt($test, $extra = @{}) {
  $vars = @{ SOTYWARE_IT_B2_BUCKET = $Bucket; SOTYWARE_IT_B2_REGION = $Region }
  if ($Mode -eq "b2-browse" -or $Mode -eq "b2-download") {
    $vars.SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $WriteKey; $vars.SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $WriteSecret
    $vars.SOTYWARE_CLOUD_IT_B2_READ_ACCESS_KEY_ID = $Key1; $vars.SOTYWARE_CLOUD_IT_B2_READ_SECRET_ACCESS_KEY = $Secret1
  } else {
    $vars.SOTYWARE_CLOUD_IT_B2_ACCESS_KEY_ID = $Key1; $vars.SOTYWARE_CLOUD_IT_B2_SECRET_ACCESS_KEY = $Secret1
  }
  foreach ($k in $extra.Keys) { $vars[$k] = $extra[$k] }
  foreach ($k in $vars.Keys) { Set-Item "Env:$k" $vars[$k] }
  try {
    Push-Location (Join-Path $PSScriptRoot "..\..\src-tauri")
    $out = & cargo test --quiet --lib $test -- --ignored --nocapture 2>$null
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

$CloudDb = Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) "com.sotyware.desktop\cloud-accounts.sqlite3"
$CloudBackup = "$CloudDb.cloud-ui-test-backup"
if (Test-Path $CloudBackup) { "ABORT: $CloudBackup exists (left by an interrupted run?); restore or remove it first"; exit 2 }
foreach ($f in @($CloudDb, "$CloudDb-journal")) { if (Test-Path $f) { Move-Item $f ($f -replace [regex]::Escape($CloudDb), $CloudBackup) } }
if (Test-Path $CloudBackup) { "cloud accounts: existing database moved to $CloudBackup for this run" }
$before = CmdkeyLines
"credential manager: $($before.Count) com.sotyware.desktop line(s) before the run"

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
    foreach ($f in @($CloudBackup, "$CloudBackup-journal")) { if (Test-Path $f) { Move-Item $f ($f -replace [regex]::Escape($CloudBackup), $CloudDb) } }
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

$label = "sotyware e2e $run"; $id = "sotyware-e2e-$run"; $renamed = "$label renamed"
try {
  $p = Start-Process $Exe -PassThru
  Check "startup: dashboard window" (WaitFor { $null -ne (Dash) } 20)
  Check "Cloud Storage link opens the empty page" ((WaitFor { Link "Cloud Storage" } 15) -and (WaitFor { (PageText) -like "*No storage accounts yet*" } 10)) (PageText)
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
    Check "the saved files are exactly the completed downloads" (((Saved) -join "|") -eq "Bericht 2024 $([char]0xFC) & co.pdf|existing.txt|foreign.txt|readme copy.txt") "$(Saved)"
   } catch {
    # Still go on to remove the account, so no test key stays behind.
    Check "download steps ran without a script error" $false "$($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    $d = DialogNamed "Save download as"; if ($d) { CancelDialog $d }
    $null = Press "Dismiss"
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
} finally {
  Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
  $null = WaitFor { (Procs).Count -eq 0 } 5
  foreach ($f in @($CloudDb, "$CloudDb-journal")) { if (Test-Path $f) { Remove-Item $f } }
  foreach ($f in @($CloudBackup, "$CloudBackup-journal")) { if (Test-Path $f) { Move-Item $f ($f -replace [regex]::Escape($CloudBackup), $CloudDb) } }
  if (Test-Path $CloudDb) { "cloud accounts: your database was put back" }
  if ($Mode -eq "b2-browse") {
    $fpMid2 = Fingerprint
    Check "b2-browse: browsing changed no object version" (($null -ne $fpMid) -and $fpMid -eq $fpMid2)
    BrowseCleanup
    Remove-Item $ManFile -EA SilentlyContinue
  }
  if ($Mode -eq "browse" -or $Mode -eq "download") {
    Remove-Item $SrvFile, "$SrvFile.json" -EA SilentlyContinue
    if (-not $srv.WaitForExit(30000)) { Stop-Process -Id $srv.Id -EA SilentlyContinue }
  }
  if ($Mode -eq "download" -and (Test-Path $Out)) { Remove-Item -Recurse -Force $Out }
  if ($Mode -eq "b2-download") {
    DownloadCleanup
    $partials = @(Get-ChildItem -Force -Recurse -Path @($Out, $SrcDir) -EA SilentlyContinue | Where-Object { $_.Name -like "*.sotyware-partial" })
    Remove-Item -Recurse -Force -Path @($Out, $SrcDir) -EA SilentlyContinue
    Check "b2-download: local files removed (downloaded file, generated original, both folders; no .sotyware-partial)" ($partials.Count -eq 0 -and -not (Test-Path $Out) -and -not (Test-Path $SrcDir))
  }
}
if ($Mode -eq "browse" -or $Mode -eq "download") {
  $srvLog = (Get-Content $SrvOut -Raw -EA SilentlyContinue)
  $ops = [regex]::Match("$srvLog", 'operations: [^\r\n]*').Value
  Check "${Mode}: the test server saw only reads and the check ($ops)" (($srv.ExitCode -eq 0) -and ("$srvLog" -match 'test result: ok\. 1 passed')) "$srvLog $(Get-Content "$SrvOut.err" -Tail 15 -EA SilentlyContinue)"
  Remove-Item $SrvOut, "$SrvOut.err" -EA SilentlyContinue
}
$after = CmdkeyLines
Check "Credential Manager: same com.sotyware.desktop lines as before the run" (($after -join "|") -eq ($before -join "|")) "before=$($before.Count) after=$($after.Count)"
if ($Mode -eq "b2" -or $Mode -eq "b2-browse") {
  $fpAfter = Fingerprint
  if ($fpAfter) { "  after:  $($fpAfter -replace '^sotyware-it fingerprint ', '')" }
  Check "${Mode}: every object version under sotyware-it/ is as before the run" (($null -ne $fpAfter) -and $fpAfter -eq $fpBefore)
}
$Secret1 = $null; $Secret2 = $null; $c = $null; $WriteSecret = $null
""
"summary: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
