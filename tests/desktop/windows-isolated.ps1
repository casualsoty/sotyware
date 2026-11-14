# Unattended Windows test of the sotyware release binary on a private desktop:
# dashboard window, "Hide dashboard" button, close -> hide, single instance.
#
# usage (PowerShell, from the repository root on a Windows path):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-isolated.ps1 [-Exe <path>]
#   (default: src-tauri\target\test-profile\release\sotyware.exe, a test-profile
#   build: npm run build:test-profile)
#
# The app runs in a test profile of this run (ADR-027, TestProfile.psm1),
# created by the launcher under src-tauri\target\desktop-test-profiles\ and
# removed at the end; the copy on the private desktop checks the app's
# acknowledgement before it touches the window. The user's own
# %LOCALAPPDATA%\com.sotyware.desktop is never read or written (before
# 2026-11-14 this test left an empty account database there).
#
# The script creates a new desktop (CreateDesktop) in the current window station,
# runs itself on it with -OnDesktop, and prints what that copy reported. sotyware
# and every window it opens live on that desktop, which is never shown: nothing
# appears on your screen and the mouse and keyboard are not used, so you can keep
# working. It is NOT headless: it needs a logged-in Windows session (the
# interactive window station WinSta0), but not a visible, unlocked screen.
#
# What it cannot cover: a private desktop has no taskbar, so the tray icon is never
# shown (sotyware still starts in tray mode; the tray backend waits for a taskbar
# that never comes), and the tray menu, left click, Quit and the tray icon's
# removal are not tested here. tests\desktop\windows.ps1 covers them on the real
# desktop. The process is ended with Stop-Process at the end, which is cleanup,
# not a check of Quit.
#
# It does not run in an elevated session (administrator token, as on GitHub-hosted
# runners): an elevated WebView2 host gets its browser process started
# de-elevated through explorer.exe, so that process lands on explorer's desktop
# (WinSta0\Default) instead of the private one and cannot attach to the dashboard
# window; sotyware then panics with "failed to create webview ... 0x80070578
# Invalid window handle". That is this test's environment, not sotyware: elevated
# on the Default desktop it works. The script reports NOT RUN and exits 4 before
# starting anything (docs/development/testing.md#windows-isolated-desktop-test).
#
# Each check prints PASS or FAIL, INFO lines are observations. Exit code: 0 all
# passed, 1 a check failed, 2 binary missing or already running, 3 the private
# desktop could not be created or the copy did not run on it, 4 not run: elevated
# session, 124 timeout.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\test-profile\release\sotyware.exe"),
  [string]$ProfileParent = (Join-Path $PSScriptRoot "..\..\src-tauri\target\desktop-test-profiles"),
  [string]$OnDesktop = "",
  [string]$Log = "",
  [int]$Timeout = 180
)
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run build:test-profile)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
if (-not (Test-TestProfileBuild $Exe)) { "ABORT: $Exe is not a test-profile build (ADR-027); run: npm run build:test-profile"; exit 2 }
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class I {
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct SI {
    public int cb; public string reserved, desktop, title; public int x, y, w, h, cx, cy, fill, flags;
    public short show, r2; public IntPtr r3, stdIn, stdOut, stdErr; }
  [StructLayout(LayoutKind.Sequential)] public struct PI { public IntPtr process, thread; public int pid, tid; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr CreateDesktop(string n, IntPtr d, IntPtr m, int f, uint a, IntPtr s);
  [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateProcessW(string a, StringBuilder c, IntPtr pa, IntPtr ta, bool inh, uint f, IntPtr env, string cwd, ref SI si, out PI pi);
  [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32.dll")] static extern bool GetExitCodeProcess(IntPtr h, out uint code);
  [DllImport("kernel32.dll")] static extern bool TerminateProcess(IntPtr h, uint code);
  // Starts a process on the given desktop and returns its process handle, which is
  // kept open so its exit code can still be read after it ends.
  public static IntPtr Start(string app, string cmd, string desktop) {
    var si = new SI(); si.cb = Marshal.SizeOf(typeof(SI)); si.desktop = desktop; PI pi;
    if (!CreateProcessW(app, new StringBuilder(cmd, 8192), IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref si, out pi))
      throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    return pi.process;
  }
  // The exit code of the process, or -1 if it is still running after ms.
  public static long WaitExit(IntPtr h, uint ms) {
    if (WaitForSingleObject(h, ms) != 0) return -1; uint c; GetExitCodeProcess(h, out c); return c;
  }
  public static void Kill(IntPtr h) { TerminateProcess(h, 1); }
  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern IntPtr GetThreadDesktop(uint tid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int i, StringBuilder b, int n, out int need);
  public static string DesktopName(IntPtr h) {
    if (h == IntPtr.Zero) return ""; var b = new StringBuilder(256); int n;
    return GetUserObjectInformation(h, 2, b, 512, out n) ? b.ToString() : "";
  }
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  // All top-level windows (visible or not) of the given class owned by the given
  // pids. EnumWindows lists only the calling thread's desktop.
  public static IntPtr[] Find(string cls, uint[] pids) {
    var res = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid);
      var sb = new StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == cls && Array.IndexOf(pids, pid) >= 0) res.Add(h); return true; }, IntPtr.Zero);
    return res.ToArray();
  }
}
"@
$ErrorActionPreference = "Continue"
function Procs { @(Get-Process sotyware -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }

# --- launcher: create the private desktop and run this script on it -------------
if ($OnDesktop -eq "") {
  $cv = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
  "sotyware Windows isolated-desktop test  $(Get-Date -Format s)"
  "os: $((Get-CimInstance Win32_OperatingSystem).Caption) $($cv.DisplayVersion) build $($cv.CurrentBuild).$($cv.UBR)"
  "binary: $Exe"
  if ((Procs).Count -gt 0) { "ABORT: sotyware from this binary is already running"; exit 2 }
  $elevated = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  "elevated: $elevated"
  if ($elevated) {
    "NOT RUN: this session is elevated. WebView2 then starts its browser process de-elevated on the Default desktop, which cannot attach to a window on the private desktop, so no check could tell anything about sotyware. Run the test from a non-elevated PowerShell."
    exit 4
  }
  $NormalBefore = Get-NormalProfileFingerprint
  $NormalCredBefore = Get-NormalCredentialLines
  $Prof = New-TestProfile $ProfileParent ("isolated-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddHHmmssfff"))
  "test profile: $($Prof.Root)"
  # Inherited by the copy on the private desktop and by every app it starts.
  $env:SOTYWARE_TEST_PROFILE = $Prof.Root
  $name = "sotyware-test-$PID"
  $desk = [I]::CreateDesktop($name, [IntPtr]::Zero, [IntPtr]::Zero, 0, 0x10000000, [IntPtr]::Zero)
  if ($desk -eq [IntPtr]::Zero) { "ABORT: CreateDesktop failed (error $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"; exit 3 }
  $logFile = Join-Path ([IO.Path]::GetTempPath()) "$name.log"
  $ps = Join-Path $env:WINDIR "System32\WindowsPowerShell\v1.0\powershell.exe"
  $cmd = "`"$ps`" -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Exe `"$Exe`" -OnDesktop `"$name`" -Log `"$logFile`""
  try { $driver = [I]::Start($ps, $cmd, "WinSta0\$name") }
  catch { "ABORT: could not start the test on desktop ${name}: $($_.Exception.Message)"; [I]::CloseDesktop($desk) | Out-Null; exit 3 }
  $code = [I]::WaitExit($driver, [uint32]($Timeout * 1000))
  if ($code -lt 0) { "TIMEOUT after $Timeout s"; [I]::Kill($driver); $code = 124 }
  # Whatever the copy left running (only after a timeout or a crash) is stopped here.
  Procs | ForEach-Object { Stop-Process -Id $_.Id -Force -EA SilentlyContinue }
  if (Test-Path $logFile) {
    $lines = Get-Content $logFile; $lines; Remove-Item $logFile -EA SilentlyContinue
    # A FAIL line must never end in exit 0, whatever the copy returned.
    if ($code -eq 0 -and @($lines | Where-Object { $_ -like "FAIL *" }).Count -gt 0) { "ERROR: FAIL lines but exit code 0; reporting 1"; $code = 1 }
  }
  else { "ABORT: the test on desktop $name wrote no output"; if ($code -ne 124) { $code = 3 } }
  [I]::CloseDesktop($desk) | Out-Null
  for ($i = 0; $i -lt 50 -and (Procs).Count -gt 0; $i++) { Start-Sleep -Milliseconds 100 }
  try { Remove-TestProfile $Prof; "PASS  cleanup: this run's test profile removed" }
  catch { "FAIL  cleanup: this run's test profile removed  $_"; $code = 1 }
  if (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|")) { "PASS  the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" }
  else { "FAIL  the user's own sotyware folders are unchanged"; $code = 1 }
  if (((Get-NormalCredentialLines) -join "|") -eq ($NormalCredBefore -join "|")) { "PASS  the user's own sotyware credentials are unchanged" }
  else { "FAIL  the user's own sotyware credentials are unchanged"; $code = 1 }
  exit $code
}

# --- the test itself, running on the private desktop ------------------------------
function Say($s) { Add-Content -Path $Log -Value $s -Encoding UTF8 }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]; $S = [System.Windows.Automation.TreeScope]
$Invoke = [System.Windows.Automation.InvokePattern]::Pattern
function PC($prop, $val) { New-Object System.Windows.Automation.PropertyCondition $prop, $val }
$script:pass = 0; $script:fail = 0
function Check($name, [bool]$ok, $detail = "") {
  if ($ok) { $script:pass++; Say "PASS  $name" } else { $script:fail++; Say "FAIL  $name  $detail" }
}
function Info($name, $detail) { Say "INFO  ${name}: $detail" }
function WaitFor([scriptblock]$pred, [double]$timeout = 8) {
  $end = (Get-Date).AddSeconds($timeout)
  while ((Get-Date) -lt $end) { if (& $pred) { return $true }; Start-Sleep -Milliseconds 150 }
  return [bool](& $pred)
}
function Pids { [uint32[]]@((Procs).Id) }
function DashHwnds { $p = Pids; if ($p.Count -eq 0) { return @() }; @([I]::Find("Tauri Window", $p)) }
function Hwnd { $d = DashHwnds; if ($d.Count -ge 1) { $d[0] } else { [IntPtr]::Zero } }
function Dash { $h = Hwnd; if ($h -eq [IntPtr]::Zero) { $null } else { $A::FromHandle($h) } }
function Visible { $h = Hwnd; ($h -ne [IntPtr]::Zero) -and [I]::IsWindowVisible($h) -and -not [I]::IsIconic($h) }
function Hidden { $h = Hwnd; ($h -ne [IntPtr]::Zero) -and -not [I]::IsWindowVisible($h) }
function HideButton { $d = Dash; if ($d) { $d.FindFirst($S::Descendants, (PC $A::NameProperty "Hide dashboard")) } else { $null } }
function SecondLaunch($label) {
  $before = (Procs).Id
  $p = Start-Process $Exe -PassThru
  $exited = $p.WaitForExit(10000)
  Check "${label}: second launch exits" $exited
  if ($exited) { Check "${label}: second launch exit code 0" ($p.ExitCode -eq 0) "code=$($p.ExitCode)" }
  Start-Sleep -Milliseconds 500
  $now = Procs
  Check "${label}: still exactly one sotyware process (same pid)" (($now.Count -eq 1) -and ($now[0].Id -eq $before[0])) "pids=$($now.Id -join ',')"
  Check "${label}: still exactly one dashboard window" ((DashHwnds).Count -eq 1) "count=$((DashHwnds).Count)"
}

$own = [I]::DesktopName([I]::GetThreadDesktop([I]::GetCurrentThreadId()))
$inputDesk = [I]::DesktopName([I]::OpenInputDesktop(0, $false, 0x0001))
# If this copy were on the visible desktop, it would not be the test it claims to be.
if ($own -ne $OnDesktop) { Say "ABORT: running on desktop '$own', expected '$OnDesktop'"; exit 3 }
Info "desktop" "running on '$own' (input desktop: '$(if ($inputDesk) { $inputDesk } else { 'none: locked or disconnected' })')"
Info "taskbar on this desktop" ([I]::FindWindow("Shell_TrayWnd", $null) -ne [IntPtr]::Zero)

$stderr = Join-Path ([IO.Path]::GetTempPath()) "$OnDesktop.stderr"
$Prof = Open-TestProfile $env:SOTYWARE_TEST_PROFILE
$primary = Start-Process $Exe -PassThru -RedirectStandardError $stderr
# Nothing is touched in a window whose data is not provably this run's (ADR-027).
try { $null = Wait-TestProfileAck $Prof $primary.Id 30 $primary; Check "startup: runs in this run's test profile (acknowledged)" $true }
catch {
  Check "startup: runs in this run's test profile (acknowledged)" $false "$_"
  Procs | ForEach-Object { Stop-Process -Id $_.Id -Force -EA SilentlyContinue }
  Say "ABORT: the app is not provably isolated; nothing else is run"; exit 1
}

# --- startup ----------------------------------------------------------------------
Check "startup: dashboard window visible" (WaitFor { Visible } 15)
Check "startup: one process" ((Procs).Count -eq 1)
Check "startup: one dashboard window" ((DashHwnds).Count -eq 1)
Check "startup: dashboard title is sotyware" ((Dash).Current.Name -eq "sotyware")

# --- the page: tray mode shows the Hide button (WebView2 content via UIA) ----------
$btn = $null
WaitFor { $script:btn = HideButton; $null -ne $script:btn } 10 | Out-Null
Check "page: Hide dashboard button present (tray mode)" ($null -ne $btn)
if ($btn) {
  $btn.GetCurrentPattern($Invoke).Invoke()
  Check "hide button (UIA invoke): dashboard hidden" (WaitFor { Hidden } 5)
  Check "hide button (UIA invoke): process alive" ((Procs).Count -eq 1 -and -not $primary.HasExited)
  Check "hide button (UIA invoke): window not destroyed" ((DashHwnds).Count -eq 1)
}

# --- single instance --------------------------------------------------------------
SecondLaunch "hidden"
Check "hidden: second launch shows dashboard" (WaitFor { Visible } 5)
SecondLaunch "visible"
Check "visible: dashboard still visible" (Visible)
[I]::ShowWindow((Hwnd), 6) | Out-Null   # SW_MINIMIZE
Check "minimized: dashboard is iconic" (WaitFor { [I]::IsIconic((Hwnd)) } 3)
SecondLaunch "minimized"
Check "minimized: second launch restores (not iconic, visible)" (WaitFor { Visible } 5)

# --- close requests hide ---------------------------------------------------------------
# WM_CLOSE is what the title-bar X and Alt+F4 send; WindowPattern.Close is the UIA
# request. Neither needs input on this desktop.
[I]::PostMessage((Hwnd), 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Check "close (WM_CLOSE): dashboard hidden" (WaitFor { Hidden } 5)
Check "close (WM_CLOSE): process alive" ((Procs).Count -eq 1 -and -not $primary.HasExited)
Check "close (WM_CLOSE): window not destroyed" ((DashHwnds).Count -eq 1)
SecondLaunch "after close"
Check "after close: second launch shows dashboard" (WaitFor { Visible } 5)
$d = Dash; if ($d) { ($d.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)).Close() }
Check "close request (UIA WindowPattern.Close): dashboard hidden" (WaitFor { Hidden } 5)
Check "close request (UIA WindowPattern.Close): process alive" ((Procs).Count -eq 1)
Check "close request (UIA WindowPattern.Close): window not destroyed" ((DashHwnds).Count -eq 1)

# --- repeated and concurrent launches --------------------------------------------------
for ($i = 1; $i -le 3; $i++) { SecondLaunch "sequential $i" }
Check "sequential: dashboard shown" (WaitFor { Visible } 5)
$conc = 1..4 | ForEach-Object { Start-Process $Exe -PassThru }
$allExit = $true; foreach ($c in $conc) { if (-not $c.WaitForExit(15000)) { $allExit = $false } }
Check "concurrent x4: all exited" $allExit
Check "concurrent x4: all exit code 0" (@($conc | Where-Object { $_.ExitCode -ne 0 }).Count -eq 0) "codes=$(($conc | % ExitCode) -join ',')"
Start-Sleep -Milliseconds 500
Check "concurrent x4: one process (the primary)" ((Procs).Count -eq 1 -and (Procs)[0].Id -eq $primary.Id)
Check "concurrent x4: one window" ((DashHwnds).Count -eq 1)
$btn = HideButton
Check "after the launches: Hide dashboard button present" ($null -ne $btn)
if ($btn) { $btn.GetCurrentPattern($Invoke).Invoke(); Check "after the launches: hide button hides" (WaitFor { Hidden } 5) }

# --- cleanup: no tray, so no Quit; stop the process ----------------------------------
Check "end: primary still running before cleanup" (-not $primary.HasExited)
Procs | ForEach-Object { Stop-Process -Id $_.Id -Force -EA SilentlyContinue }
Check "cleanup: no sotyware process left" (WaitFor { (Procs).Count -eq 0 } 5)
$err = if (Test-Path $stderr) { (Get-Content $stderr -Raw) } else { "" }
Info "stderr of the primary" $(if ($err) { $err.Trim() } else { "(empty)" })
Remove-Item $stderr -EA SilentlyContinue

Say ""
Say "summary: $script:pass passed, $script:fail failed (tray, tray menu, left click and Quit not tested: no taskbar on a private desktop)"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
