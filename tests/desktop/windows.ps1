# Native Windows desktop test of the sotyware release binary: window lifecycle,
# "Hide dashboard" button, built-in features section (list and switch the
# Scheduled backups feature), tray icon and menu, single instance, Quit and
# relaunch, that feature's saved choice across real restarts, and the theme
# chosen just before a tray Quit (ADR-035), each relaunch waiting until
# WebView2's processes of the quitting instance have ended too.
#
# usage (PowerShell, from the repository root on a Windows path):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1 [-Exe <path>]
#   (default: src-tauri\target\test-profile\release\sotyware.exe, a test-profile
#   build: npm run build:test-profile)
#
# This runs on your real desktop, not in a sandbox: it moves the mouse and clicks
# (tray icon, tray menu, the dashboard's buttons, and its own title bar: Close, a
# double click and a drag that moves the window by 80 x 60 px), and
# opens the notification-area "Show Hidden Icons" flyout. Do not use the mouse or
# keyboard while it runs. It needs no extra modules: Windows PowerShell 5.1, UI
# Automation and user32 only.
#
# The app runs in a test profile of this run (ADR-027, TestProfile.psm1): a
# test-profile build that refuses to start without one, keeps its databases
# and webview data in it (under src-tauri\target\desktop-test-profiles\ by
# default) and acknowledges it; every launch that becomes the primary
# instance is checked against that acknowledgement before the window is
# touched. The user's own %LOCALAPPDATA%\com.sotyware.desktop is never moved,
# written or deleted (before 2026-11-14 this script moved and deleted the
# databases there, which is unsafe: see docs/status/status.md). The run never
# adds a cloud account, so it never writes to the Windows Credential Manager.
#
# Each check prints PASS or FAIL; INFO lines record observations that Windows
# focus rules may legitimately change (which window is in the foreground).
# Exit code: 0 all passed, 1 a check failed, 2 binary missing or already running,
# 3 no interactive desktop (not the input desktop, or no taskbar): nothing was run.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\test-profile\release\sotyware.exe"),
  [string]$ProfileParent = (Join-Path $PSScriptRoot "..\..\src-tauri\target\desktop-test-profiles")
)
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run build:test-profile)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
$TPC = Get-TestProfileConstants
if (-not (Test-TestProfileBuild $Exe)) { "ABORT: $Exe is not a test-profile build (ADR-027); run: npm run build:test-profile"; exit 2 }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  public static void Click(int x, int y, bool right) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(80);
    uint dn = right ? 0x0008u : 0x0002u, up = right ? 0x0010u : 0x0004u;
    mouse_event(dn, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(50); mouse_event(up, 0, 0, 0, UIntPtr.Zero);
  }
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  public static string ClassOf(IntPtr h) { var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr m);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetMenuString(IntPtr m, uint id, System.Text.StringBuilder s, int max, uint flag);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint i, out RECT r);
  // All top-level windows (visible or not) of the given class owned by the given pids.
  public static IntPtr[] Find(string cls, uint[] pids) {
    var res = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, l) => { uint pid; GetWindowThreadProcessId(h, out pid);
      var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256);
      if (sb.ToString() == cls && Array.IndexOf(pids, pid) >= 0) res.Add(h); return true; }, IntPtr.Zero);
    return res.ToArray();
  }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool SystemParametersInfo(uint a, uint b, ref RECT r, uint f);
  // The primary monitor's work area (SPI_GETWORKAREA): x, y, width, height.
  public static int[] WorkArea() { var r = new RECT(); SystemParametersInfo(0x0030, 0, ref r, 0); return new int[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
  // A window's rectangle: x, y, width, height (null if it is gone).
  public static int[] Bounds(IntPtr h) { RECT r; if (!GetWindowRect(h, out r)) return null; return new int[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
  // A real drag with the left button: press at (x1, y1), move there in steps, release.
  public static void Drag(int x1, int y1, int x2, int y2) {
    SetCursorPos(x1, y1); System.Threading.Thread.Sleep(80);
    mouse_event(0x0002u, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(150);
    for (int i = 1; i <= 12; i++) { SetCursorPos(x1 + (x2 - x1) * i / 12, y1 + (y2 - y1) * i / 12); System.Threading.Thread.Sleep(30); }
    System.Threading.Thread.Sleep(150); mouse_event(0x0004u, 0, 0, 0, UIntPtr.Zero);
  }
  public static void DoubleClick(int x, int y) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(80);
    for (int i = 0; i < 2; i++) { mouse_event(0x0002u, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(40); mouse_event(0x0004u, 0, 0, 0, UIntPtr.Zero); System.Threading.Thread.Sleep(60); }
  }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
  // The top-level window (GA_ROOT) that a click at this point would reach.
  public static IntPtr TopLevelAt(int x, int y) { var p = new POINT { X = x, Y = y }; return GetAncestor(WindowFromPoint(p), 2); }
  // The title bar is not in the UIA tree, so find the close button the way the
  // window manager does: the point where WM_NCHITTEST answers HTCLOSE (20).
  public static int[] FindClose(IntPtr h) {
    RECT r; if (!GetWindowRect(h, out r)) return null;
    for (int y = r.T + 2; y < r.T + 60; y += 4)
      for (int x = r.R - 2; x > r.R - 200; x -= 4) {
        long lp = (long)(((uint)(y & 0xFFFF) << 16) | (uint)(x & 0xFFFF));
        if ((long)SendMessage(h, 0x0084, IntPtr.Zero, (IntPtr)lp) == 20) return new int[] { x - 10, y + 6 };
      }
    return null;
  }
  public static string[] MenuStrings(IntPtr hm) {
    int n = GetMenuItemCount(hm); var r = new string[Math.Max(n, 0)];
    for (int i = 0; i < n; i++) { var sb = new System.Text.StringBuilder(256); GetMenuString(hm, (uint)i, sb, 256, 0x400); r[i] = sb.ToString(); }
    return r;
  }
  public static void Esc() { keybd_event(0x1B, 0, 0, UIntPtr.Zero); keybd_event(0x1B, 0, 2, UIntPtr.Zero); }
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern IntPtr GetThreadDesktop(uint tid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetUserObjectInformation(IntPtr h, int i, System.Text.StringBuilder b, int n, out int need);
  public static string DesktopName(IntPtr h) {
    if (h == IntPtr.Zero) return ""; var b = new System.Text.StringBuilder(256); int n;
    return GetUserObjectInformation(h, 2, b, 512, out n) ? b.ToString() : "";
  }
  // A process handle kept across the process's end, so its exit time can
  // still be read (SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION; zero if
  // refused). While it is open the process id cannot be reused.
  [DllImport("kernel32.dll")] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
  [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr h, out long created, out long exited, out long kernel, out long user);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  public static IntPtr Watch(uint pid) { return OpenProcess(0x00101000u, false, pid); }
  public static bool Ended(IntPtr h) { return WaitForSingleObject(h, 0) == 0; }
  // Creation and exit time as UTC file times (100 ns units); the exit time
  // means something only once the process has ended.
  public static long[] Times(IntPtr h) { long c, e, k, u; return GetProcessTimes(h, out c, out e, out k, out u) ? new long[] { c, e } : null; }
}
"@
$ErrorActionPreference = "Continue"
# Real mouse clicks and the notification area exist only on the desktop that receives
# input and has a taskbar. Without one (locked or disconnected session, a service,
# a private desktop) every check would fail for reasons unrelated to sotyware, so
# stop before starting it (exit 3) instead of reporting those failures.
$inputDesk = [W]::DesktopName([W]::OpenInputDesktop(0, $false, 0x0001))
$ownDesk = [W]::DesktopName([W]::GetThreadDesktop([W]::GetCurrentThreadId()))
if ($inputDesk -eq "" -or $inputDesk -ne $ownDesk) { "ABORT: not on the interactive input desktop (this desktop: '$ownDesk', input desktop: '$inputDesk'); this test needs an unlocked, logged-in desktop session"; exit 3 }
if ([W]::FindWindow("Shell_TrayWnd", $null) -eq [IntPtr]::Zero) { "ABORT: no taskbar (Shell_TrayWnd) on this desktop; the tray checks need Explorer's notification area"; exit 3 }
[W]::SetProcessDPIAware() | Out-Null
$A = [System.Windows.Automation.AutomationElement]; $S = [System.Windows.Automation.TreeScope]
$Invoke = [System.Windows.Automation.InvokePattern]::Pattern
function PC($prop, $val) { New-Object System.Windows.Automation.PropertyCondition $prop, $val }
$root = $A::RootElement
$script:pass = 0; $script:fail = 0
function Check($name, [bool]$ok, $detail = "") {
  if ($ok) { $script:pass++; "PASS  $name" } else { $script:fail++; "FAIL  $name  $detail" }
}
function Info($name, $detail) { "INFO  ${name}: $detail" }
function WaitFor([scriptblock]$pred, [double]$timeout = 8) {
  $end = (Get-Date).AddSeconds($timeout)
  while ((Get-Date) -lt $end) { if (& $pred) { return $true }; Start-Sleep -Milliseconds 150 }
  return [bool](& $pred)
}
function Procs { @(Get-Process sotyware -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }
function Pids { [uint32[]]@((Procs).Id) }
function DashHwnds { $p = Pids; if ($p.Count -eq 0) { return @() }; @([W]::Find("Tauri Window", $p)) }
function Hwnd { $d = DashHwnds; if ($d.Count -ge 1) { $d[0] } else { [IntPtr]::Zero } }
function Dash { $h = Hwnd; if ($h -eq [IntPtr]::Zero) { $null } else { $A::FromHandle($h) } }
function Visible { $h = Hwnd; ($h -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($h) -and -not [W]::IsIconic($h) }
function Hidden { $h = Hwnd; ($h -ne [IntPtr]::Zero) -and -not [W]::IsWindowVisible($h) }
function Foreground { (Hwnd) -eq [W]::GetForegroundWindow() }
# The sotyware icon sits in the "hidden icons" overflow flyout (Windows 11 default for
# new icons). The flyout closes whenever focus moves, so open it before every lookup.
# UIA only lists the flyout window while it is shown.
$OverflowClass = "TopLevelWindowForOverflowXamlIsland"
function OverflowShown {
  $o = $root.FindFirst($S::Children, (PC $A::ClassNameProperty $OverflowClass))
  ($null -ne $o) -and -not $o.Current.BoundingRectangle.IsEmpty
}
# When hiding the dashboard moves the foreground to another app, the flyout loses
# focus and starts closing, but UIA still reports it and its icons for a moment; a
# click then lands on nothing. It is only really open while it is the foreground.
function OverflowOpen { (OverflowShown) -and [W]::ClassOf([W]::GetForegroundWindow()) -eq $OverflowClass }
function OpenOverflow {
  if (OverflowOpen) { return }
  if (OverflowShown) { WaitFor { -not (OverflowShown) } 3 | Out-Null }
  if ((TrayIcons).Count -ge 1) { return }  # icon pinned to the taskbar itself
  $chev = $root.FindFirst($S::Descendants, (PC $A::NameProperty "Show Hidden Icons"))
  if ($chev) { $chev.GetCurrentPattern($Invoke).Invoke(); WaitFor { OverflowOpen } 3 | Out-Null; Start-Sleep -Milliseconds 400 }
}
# sotyware notification-area icons that are actually on screen (stale elements of a
# closed flyout have an empty rectangle).
# The test-profile build's tooltip, never an installed sotyware's ("sotyware").
function TrayIcons {
  @($root.FindAll($S::Descendants, (New-Object System.Windows.Automation.AndCondition (PC $A::AutomationIdProperty "NotifyItemIcon"), (PC $A::NameProperty "sotyware test profile"))) |
    Where-Object { $r = $_.Current.BoundingRectangle; -not $r.IsEmpty -and -not [double]::IsNaN($r.X) })
}
function TrayIconCount { OpenOverflow; (TrayIcons).Count }
# Retries because a flyout that is still animating closed reports itself as open.
function TrayIcon {
  WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 5 | Out-Null
  $t = TrayIcons; if ($t.Count -ge 1) { $t[0] } else { $null }
}
# Opens the tray context menu with a real right click and returns its HMENU (or zero).
# UI Automation exposes none of the menu's items, so they are read through the
# Win32 menu handle (MN_GETHMENU) instead.
function OpenTrayMenu {
  $icon = TrayIcon; if (-not $icon) { $script:why = "tray icon not found"; return [IntPtr]::Zero }
  $r = $icon.Current.BoundingRectangle
  [W]::Click([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), $true)
  $script:why = "context menu did not open"
  $script:menuWnd = [IntPtr]::Zero
  WaitFor { $m = @([W]::Find("#32768", (Pids))); if ($m.Count -ge 1) { $script:menuWnd = $m[0] }; $script:menuWnd -ne [IntPtr]::Zero } 4 | Out-Null
  if ($script:menuWnd -eq [IntPtr]::Zero) { return [IntPtr]::Zero }
  return [W]::SendMessage($script:menuWnd, 0x01E1, [IntPtr]::Zero, [IntPtr]::Zero)
}
function MenuItems {
  $hm = OpenTrayMenu; if ($hm -eq [IntPtr]::Zero) { return @() }
  $names = [W]::MenuStrings($hm)
  [W]::Esc(); Start-Sleep -Milliseconds 300
  return $names
}
# Clicks a tray menu item with the real mouse.
function TrayMenu([string]$item) {
  $hm = OpenTrayMenu; if ($hm -eq [IntPtr]::Zero) { return $false }
  $idx = [array]::IndexOf([W]::MenuStrings($hm), $item)
  if ($idx -lt 0) { $script:why = "item '$item' not in menu"; [W]::Esc(); return $false }
  $rc = New-Object W+RECT
  if (-not [W]::GetMenuItemRect([IntPtr]::Zero, $hm, [uint32]$idx, [ref]$rc)) { $script:why = "GetMenuItemRect failed"; [W]::Esc(); return $false }
  [W]::Click([int](($rc.L + $rc.R) / 2), [int](($rc.T + $rc.B) / 2), $false)
  return $true
}
# Built-in features section (src/Features.svelte) as WebView2 exposes it through UIA:
# the checkbox is named after the feature, and its list item's name is the item's
# text (version, state label and description).
# The feature switched: backup-scheduler (src-tauri/src/features/backup.rs), enabled
# by default; a fresh test profile has no backup job, so switching it does nothing visible.
$SwitchedFeature = "Scheduled backups"
$SwitchedVersion = "0.1.0"
$SwitchedDescription = "Runs backup jobs that have an interval when they are due, while sotyware is running."
$Toggle = [System.Windows.Automation.TogglePattern]::Pattern
$Walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
function PCT($type) { [System.Windows.Automation.PropertyCondition]::new($A::ControlTypeProperty, $type) }
function FindFeatureBox {
  $d = Dash; if (-not $d) { return $null }
  $d.FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT ([System.Windows.Automation.ControlType]::CheckBox)), (PC $A::NameProperty $SwitchedFeature)))
}
# Since the redesign Settings has tabs (General, Features, About & updates):
# the Settings link in the sidebar, then the tab (UIA Invoke, or SelectionItem
# where WebView2 offers only that for a tab).
function OpenSettingsTab([string]$name) {
  $d = Dash; if (-not $d) { return $false }
  $link = $d.FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT ([System.Windows.Automation.ControlType]::Hyperlink)), (PC $A::NameProperty "Settings")))
  if (-not $link) { return $false }
  try { $link.GetCurrentPattern($Invoke).Invoke() } catch { return $false }
  $script:settingsTab = $null
  if (-not (WaitFor { $script:settingsTab = (Dash).FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT ([System.Windows.Automation.ControlType]::TabItem)), (PC $A::NameProperty $name))); $script:settingsTab -ne $null } 5)) { return $false }
  try { $script:settingsTab.GetCurrentPattern($Invoke).Invoke(); return $true } catch { }
  try { $script:settingsTab.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select(); return $true } catch { return $false }
}
# The features are under Settings > Features since the redesign.
function OpenFeatures { OpenSettingsTab "Features" }
function FeatureBox {
  $b = FindFeatureBox
  if (-not $b -and (OpenFeatures)) { Start-Sleep -Milliseconds 300; $b = FindFeatureBox }
  $b
}
# The switched feature as shown: checkbox checked and enabled (not busy), and its
# list item's text. $null until the list has loaded.
function FeatureView {
  $b = FeatureBox; if (-not $b) { return $null }
  try {
    $on = $b.GetCurrentPattern($Toggle).Current.ToggleState -eq [System.Windows.Automation.ToggleState]::On
    $li = $b
    for ($i = 0; $i -lt 4 -and $li -and $li.Current.ControlType -ne [System.Windows.Automation.ControlType]::ListItem; $i++) { $li = $Walker.GetParent($li) }
    $item = if ($li -and $li.Current.ControlType -eq [System.Windows.Automation.ControlType]::ListItem) { $li.Current.Name } else { "" }
    [pscustomobject]@{ Box = $b; On = $on; Ready = $b.Current.IsEnabled; Item = $item }
  } catch { $null }
}
# Every name in the page, to look for error text ("Could not ..." from Features.svelte).
function PageNames {
  $d = Dash; if (-not $d) { return "" }
  try { (@($d.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) | ForEach-Object { $_.Current.Name }) -join "`n" } catch { "" }
}
function PageError { (PageNames) -match "Could not" }
function SwitchedTo([bool]$on, [string]$state) {
  $v = FeatureView
  ($null -ne $v) -and $v.On -eq $on -and $v.Ready -and $v.Item.Contains($state) -and -not (PageError)
}
function FeatureDetail { $v = FeatureView; if ($v) { "on=$($v.On) ready=$($v.Ready) item='$($v.Item)' error=$(PageError)" } else { "checkbox not found" } }
# Switches the feature through UIA (TogglePattern, what assistive technology uses).
function ToggleFeature { $v = FeatureView; if (-not $v) { return $false }; try { $v.Box.GetCurrentPattern($Toggle).Toggle(); $true } catch { $false } }
# Switches it with a real mouse click on the checkbox, after checking that the click
# would reach the dashboard and not a window covering it.
function ClickFeature {
  $v = FeatureView; if (-not $v) { $script:why = "checkbox not found"; return $false }
  try { $v.Box.SetFocus() } catch { }   # scrolls it into view
  $r = $v.Box.Current.BoundingRectangle
  if ($r.IsEmpty) { $script:why = "checkbox has no on-screen rectangle"; return $false }
  $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
  $at = [W]::TopLevelAt($x, $y)
  if ($at -ne (Hwnd)) { $script:why = "point $x,$y is covered by $([W]::ClassOf($at))"; return $false }
  [W]::Click($x, $y, $false)
  return $true
}
# The theme (ADR-035, src/theme.ts) in Settings > General: radio buttons named
# System, Light and Dark; WebView2 gives a radio button the SelectionItem
# pattern, whose IsSelected is its checked state.
$RadioType = [System.Windows.Automation.ControlType]::RadioButton
$SelItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
function ThemeRadio([string]$name) {
  $d = Dash; if (-not $d) { return $null }
  $d.FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $RadioType), (PC $A::NameProperty $name)))
}
function IsChosen($radio) { if (-not $radio) { return $false }; try { [bool]$radio.GetCurrentPattern($SelItem).Current.IsSelected } catch { $false } }
# The choices shown as chosen: one, unless something is wrong.
function ThemesShown { @("System", "Light", "Dark" | Where-Object { IsChosen (ThemeRadio $_) }) }
# Chooses a theme as assistive technology would (Settings > General, then the
# radio button's SelectionItem, or Invoke); true once it shows as chosen.
function ChooseTheme([string]$name) {
  if (-not (OpenSettingsTab "General")) { return $false }
  $script:themeRadio = $null
  if (-not (WaitFor { $script:themeRadio = ThemeRadio $name; $null -ne $script:themeRadio } 5)) { return $false }
  $picked = $false
  try { $script:themeRadio.GetCurrentPattern($SelItem).Select(); $picked = $true } catch { }
  if (-not $picked) { try { $script:themeRadio.GetCurrentPattern($Invoke).Invoke(); $picked = $true } catch { } }
  $picked -and (WaitFor { IsChosen (ThemeRadio $name) } 5)
}
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

$cv = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
"sotyware native Windows desktop test  $(Get-Date -Format s)"
"os: $((Get-CimInstance Win32_OperatingSystem).Caption) $($cv.DisplayVersion) build $($cv.CurrentBuild).$($cv.UBR)"
"binary: $Exe"
if ((Procs).Count -gt 0) { "ABORT: sotyware from this binary is already running"; exit 2 }

# --- test profile (ADR-027) -------------------------------------------------
$NormalBefore = Get-NormalProfileFingerprint
$NormalCredBefore = Get-NormalCredentialLines
$Prof = New-TestProfile $ProfileParent ("windows-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddHHmmssfff"))
"test profile: $($Prof.Root)"
$RealBefore = Get-RealNormalProfileFingerprint $Prof.Parent
"user's real app folder (outside this process tree): $(if ($null -eq $RealBefore) { 'could not be read' } else { "$(@($RealBefore).Count) line(s)" })"
# Fail closed: inside an MSIX package (the Claude desktop app) this
# process's own view of AppData is virtual, so without the real one nothing
# could show the run left the user's data alone (ADR-027).
if ($null -eq $RealBefore) {
  Check "the user's real app folder can be read from outside this process tree before the run" $false "WMI could not start the reader"
  Remove-TestProfile $Prof
  "ABORT: isolation cannot be proven; nothing was started"; exit 1
}
$PrefsDb = Join-Path $Prof.DataDir "preferences.sqlite3"
$CloudDb = Join-Path $Prof.DataDir "cloud-accounts.sqlite3"
# This run's profile as the driver spells it and as the app does (canonical,
# links resolved), lower case without \\?\: what WebView2's command lines
# carry (see WebViewProcs).
$WebViewRoots = @(ConvertTo-ComparablePath $Prof.Root)
# Stops the whole run unless $proc acknowledges this run's profile: nothing is
# clicked in a window whose data is not provably this run's.
function Verified($proc, [string]$label) {
  try { $ack = Wait-TestProfileAck $Prof $proc.Id 30 $proc }
  catch {
    Check "${label}: runs in this run's test profile (acknowledged)" $false "$_"
    Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
    "ABORT: the app is not provably isolated; nothing else is run"; exit 1
  }
  Check "${label}: runs in this run's test profile (acknowledged)" $true
  $ackRoot = ConvertTo-ComparablePath ([string]$ack.root)
  if ($script:WebViewRoots -notcontains $ackRoot) { $script:WebViewRoots += $ackRoot }
}

# --- WebView2's processes of this run -----------------------------------------
# WebView2 runs the page in processes of its own (msedgewebview2.exe: the
# browser process, which also keeps the page's localStorage, and its renderer,
# GPU, utility and crash-handler processes) and gives each, on its command
# line, the user data folder sotyware chose: this run's test profile's webview
# folder (ADR-027). They can outlive sotyware. On Linux WebKit commits a choice
# made just before Quit only as sotyware ends, so a relaunch that read the
# storage before that commit showed the previous theme
# (docs/status/limitations.md#dashboard-adr-035): every relaunch here waits for
# them (at most $WebViewWait s) and records how long they outlived sotyware.
$WebViewWait = 30
function NamesThisRun([string]$cmd) {
  $c = $cmd.ToLowerInvariant()
  foreach ($want in $script:WebViewRoots) { if ($c.Contains($want + '\')) { return $true } }
  $false
}
# Running ones only: a process that has ended but is still held open has no
# thread left.
function WebViewProcs {
  @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -EA SilentlyContinue |
    Where-Object { $_.ThreadCount -gt 0 -and (NamesThisRun "$($_.CommandLine)") })
}
# The kind of WebView2 process a command line starts: browser (no --type),
# renderer, gpu-process, crashpad-handler, or utility with its service.
function WebViewKind([string]$cmd) {
  if ($cmd -notmatch '--type=([^\s"]+)') { return "browser" }
  $kind = $Matches[1]
  if ($kind -eq "utility" -and $cmd -match '--utility-sub-type=([^\s"]+)') { $kind = "utility " + $Matches[1] }
  $kind
}
# This run's WebView2 processes now, each with a handle kept until it has been
# measured (and one to sotyware, $proc), so that exit times can be read after
# the processes have ended. A process id reused between the listing and the
# handle (another creation time) is left out.
function WatchWebView($proc) {
  try { $null = $proc.Handle } catch { }
  @(WebViewProcs | ForEach-Object {
      $h = [W]::Watch([uint32]$_.ProcessId)
      $t = if ($h -ne [IntPtr]::Zero) { [W]::Times($h) } else { $null }
      if ($null -ne $t -and $null -ne $_.CreationDate -and [Math]::Abs($t[0] - $_.CreationDate.ToFileTime()) -lt 10000000) {
        [pscustomobject]@{ Id = [int]$_.ProcessId; Parent = [int]$_.ParentProcessId; Kind = (WebViewKind "$($_.CommandLine)"); Created = $t[0]; Handle = $h }
      } elseif ($h -ne [IntPtr]::Zero) { [void][W]::CloseHandle($h) }
    })
}
# After a tray Quit of $proc: waits until its WebView2 processes have ended
# too, at most $WebViewWait s. Passes when they were found (one browser
# process among them, started after this sotyware) and all ended; records how
# long each outlived sotyware, from the exit times of the kept handles. One
# that started after WatchWebView is seen only by listing, and is this
# sotyware's only if its parent is sotyware or one of its WebView2 processes
# (another sotyware in the profile would start a browser of its own). Sets
# $script:SotywareEnded (UTC file time).
function WebViewEnded([string]$label, $proc, $watched) {
  $watched = @($watched | Where-Object { $null -ne $_ })
  $what = "its WebView2 processes (found by the test profile on their command lines, the browser among them) ended too, within $WebViewWait s"
  $script:SotywareEnded = $null
  if (-not $proc.WaitForExit(10000)) { Check "${label}: $what" $false "sotyware did not end"; return }
  $own = [W]::Times($proc.Handle)
  $startedFt = if ($null -ne $own) { $own[0] } else { 0 }
  $endedFt = if ($null -ne $own) { $own[1] } else { $null }
  $script:SotywareEnded = $endedFt
  $deadline = (Get-Date).AddSeconds($WebViewWait)
  $known = @($watched | ForEach-Object { $_.Id })
  $later = @{}
  while ($true) {
    $running = @($watched | Where-Object { -not [W]::Ended($_.Handle) }).Count
    $extra = @()
    if ($running -eq 0) {
      $family = @($known) + @([int]$proc.Id) + @($later.Keys)
      $extra = @(WebViewProcs | Where-Object { $known -notcontains [int]$_.ProcessId -and $family -contains [int]$_.ParentProcessId })
      foreach ($e in $extra) { $later[[int]$e.ProcessId] = WebViewKind "$($e.CommandLine)" }
    }
    if ($running + $extra.Count -eq 0 -or (Get-Date) -gt $deadline) { break }
    Start-Sleep -Milliseconds 50
  }
  $doneFt = [DateTime]::UtcNow.ToFileTimeUtc()
  $family = @($known) + @([int]$proc.Id) + @($later.Keys)
  $unended = @($watched | Where-Object { -not [W]::Ended($_.Handle) }) + @(WebViewProcs | Where-Object { $known -notcontains [int]$_.ProcessId -and $family -contains [int]$_.ParentProcessId })
  $parts = @($watched | ForEach-Object {
      $over = $null; $done = [W]::Ended($_.Handle)
      $tt = if ($done) { [W]::Times($_.Handle) } else { $null }
      if ($null -ne $tt -and $null -ne $endedFt) { $over = [int](($tt[1] - $endedFt) / 10000) }
      [pscustomobject]@{ Kind = $_.Kind; Done = $done; Ms = $over }
    } | Sort-Object { if ($null -eq $_.Ms) { [int]::MaxValue } else { $_.Ms } })
  $browser = @($watched | Where-Object { $_.Kind -eq "browser" -and $_.Created -ge $startedFt })
  $found = "found=$($watched.Count): " + (($watched | ForEach-Object { "$($_.Kind) $($_.Id) (parent $($_.Parent))" }) -join ", ") + "; sotyware $($proc.Id)"
  Check "${label}: $what" ($browser.Count -eq 1 -and $unended.Count -eq 0) "$found; still running: $($unended.Count)"
  $times = ($parts | ForEach-Object { if (-not $_.Done) { "$($_.Kind) still running" } elseif ($null -eq $_.Ms) { "$($_.Kind) ended" } else { "$($_.Kind) {0:+0;-0} ms" -f $_.Ms } }) -join ", "
  if ($parts.Count -eq 0) { $times = "none found" }
  $seen = if ($later.Count -gt 0) { "; started after the snapshot, seen only by listing: $(($later.Values | Sort-Object) -join ', ')" } else { "" }
  $waited = if ($null -ne $endedFt) { [int](($doneFt - $endedFt) / 10000) } else { "?" }
  Info "${label}: WebView2's processes after sotyware ended (exit times)" "$times$seen; the wait ended at +$waited ms"
  foreach ($entry in $watched) { [void][W]::CloseHandle($entry.Handle) }
}
# For the record, not a check: whether WebView2's localStorage files (a
# LevelDB, "Local Storage\leveldb" under the profile's webview folder) hold
# the theme's key with the value dark yet. Only what WebView2 has committed
# is in them; read without locking them.
$ThemeKey = "sotyware.theme.v1"
# The default profile's folder where WebView2 usually keeps it; with $search,
# else the first "Local Storage\leveldb" under the webview folder (slower).
function LocalStorageDir([bool]$search) {
  $wv = Join-Path $Prof.Root "webview"
  $usual = Join-Path $wv "EBWebView\Default\Local Storage\leveldb"
  if (Test-Path -LiteralPath $usual) { return $usual }
  if (-not $search) { return $null }
  $hit = @(Get-ChildItem -LiteralPath $wv -Recurse -Directory -Filter "leveldb" -EA SilentlyContinue | Where-Object { $_.Parent.Name -eq "Local Storage" })
  if ($hit.Count -ge 1) { $hit[0].FullName } else { $null }
}
function ThemeOnDisk([bool]$search = $false) {
  $dir = LocalStorageDir $search
  if (-not $dir) { return "no Local Storage folder" }
  $latin1 = [System.Text.Encoding]::GetEncoding(28591)
  # Chromium stores an ASCII key and value as Latin-1 (else UTF-16) bytes.
  $narrow = [regex]::Escape($ThemeKey) + "[\s\S]{0,16}dark"
  $wide = (-join ($ThemeKey.ToCharArray() | ForEach-Object { [regex]::Escape([string]$_) + "\x00" })) + "[\s\S]{0,32}d\x00a\x00r\x00k\x00"
  try {
    foreach ($f in @(Get-ChildItem -LiteralPath $dir -File -EA Stop | Where-Object { $_.Extension -eq ".log" -or $_.Extension -eq ".ldb" })) {
      $fs = [System.IO.File]::Open($f.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
      try { $buf = New-Object System.IO.MemoryStream; $fs.CopyTo($buf); $text = $latin1.GetString($buf.ToArray()) } finally { $fs.Dispose() }
      if ($text -match $narrow -or $text -match $wide) { return "yes ($($f.Name))" }
    }
    "no"
  } catch { "unreadable: $($_.Exception.Message)" }
}
# Fail closed: no profile, or an invalid one, ends the app with exit 3 before
# any window exists.
function Refused([string]$label, $value) {
  $saved = $env:SOTYWARE_TEST_PROFILE
  if ($null -eq $value) { Remove-Item Env:SOTYWARE_TEST_PROFILE -EA SilentlyContinue } else { $env:SOTYWARE_TEST_PROFILE = $value }
  try { $r = Start-Process $Exe -PassThru } finally { if ($null -eq $saved) { Remove-Item Env:SOTYWARE_TEST_PROFILE -EA SilentlyContinue } else { $env:SOTYWARE_TEST_PROFILE = $saved } }
  $null = $r.Handle
  $done = $r.WaitForExit(15000)
  if (-not $done) { Stop-Process -Id $r.Id -EA SilentlyContinue }
  Check "fail closed: ${label}: exits $($TPC.ExitRefused) at once, no window" ($done -and $r.ExitCode -eq $TPC.ExitRefused -and (DashHwnds).Count -eq 0) "exit=$($r.ExitCode)"
}
Refused "no SOTYWARE_TEST_PROFILE" $null
$BadProf = Join-Path $Prof.Parent "$($Prof.Run)-bad"
New-Item -ItemType Directory $BadProf | Out-Null
[System.IO.File]::WriteAllText((Join-Path $BadProf $TPC.MarkerFile), ('{"format":1,"purpose":"' + $TPC.Purpose + '","run":"bad","nonce":"not-hex"}'))
Refused "a profile with an invalid marker" $BadProf
Check "fail closed: the refused launches created nothing in either profile" ((@(Get-ChildItem -Force $BadProf).Count -eq 1) -and (@(Get-ChildItem -Force $Prof.Root).Count -eq 1))
[System.IO.Directory]::Delete($BadProf, $true)
$env:SOTYWARE_TEST_PROFILE = $Prof.Root

# --- startup -------------------------------------------------------------
$primary = Start-Process $Exe -PassThru
Verified $primary "startup"
Check "startup: dashboard window visible" (WaitFor { Visible } 15)
Check "startup: one process" ((Procs).Count -eq 1)
Check "startup: one dashboard window" ((DashHwnds).Count -eq 1)
Check "startup: dashboard title is sotyware" ((Dash).Current.Name -eq "sotyware")
Check "startup: exactly one sotyware tray icon" ((WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 8) -and (TrayIcons).Count -eq 1) "count=$((TrayIcons).Count)"
$items = MenuItems
Check "tray: menu items are Open Dashboard, Quit" (($items -join "|") -eq "Open Dashboard|Quit") "got=$($items -join '|')"
Check "tray: menu dismissed, process alive, still one process" ((Procs).Count -eq 1)

# --- first placement (ADR-040) --------------------------------------------------
# A fresh profile has no saved placement: half the primary monitor's work area,
# at least 900 x 540 logical pixels (window_state::DEFAULT_MIN) and at most the
# work area, centred in the work area.
$GeoWork = [W]::WorkArea()
$GeoScale = [double]([W]::GetDpiForWindow((Hwnd))) / 96.0
if ($GeoScale -le 0) { $GeoScale = 1.0 }
$GeoStart = [W]::Bounds((Hwnd))
$GeoWantW = [Math]::Min($GeoWork[2], [Math]::Max([int][Math]::Floor($GeoWork[2] / 2), [int][Math]::Round(900 * $GeoScale)))
$GeoWantH = [Math]::Min($GeoWork[3], [Math]::Max([int][Math]::Floor($GeoWork[3] / 2), [int][Math]::Round(540 * $GeoScale)))
$GeoWantX = $GeoWork[0] + [int][Math]::Floor(($GeoWork[2] - $GeoWantW) / 2)
$GeoWantY = $GeoWork[1] + [int][Math]::Floor(($GeoWork[3] - $GeoWantH) / 2)
function Near($a, $b) { [Math]::Abs([int]$a - [int]$b) -le 2 }
$geoDetail = "work=$($GeoWork -join ',') scale=$GeoScale window=$($GeoStart -join ',') want=$GeoWantX,$GeoWantY,$GeoWantW,$GeoWantH"
Check "first placement: half the work area (at least 900 x 540)" ($null -ne $GeoStart -and (Near $GeoStart[2] $GeoWantW) -and (Near $GeoStart[3] $GeoWantH)) $geoDetail
Check "first placement: centred in the work area" ($null -ne $GeoStart -and (Near $GeoStart[0] $GeoWantX) -and (Near $GeoStart[1] $GeoWantY)) $geoDetail

# --- the page's own title bar (ADR-040) -------------------------------------------
# No native caption: the window manager finds no close button; the page draws
# Minimize, Maximize and Close in a group named "Window".
Check "title bar: no native caption button (WM_NCHITTEST finds no HTCLOSE)" ($null -eq [W]::FindClose((Hwnd)))
function TitleButton([string]$name) {
  $d = Dash; if (-not $d) { return $null }
  $g = $d.FindFirst($S::Descendants, (New-Object System.Windows.Automation.AndCondition (PC $A::ControlTypeProperty ([System.Windows.Automation.ControlType]::Group)), (PC $A::NameProperty "Window")))
  if (-not $g) { return $null }
  $g.FindFirst($S::Descendants, (New-Object System.Windows.Automation.AndCondition (PC $A::ControlTypeProperty ([System.Windows.Automation.ControlType]::Button)), (PC $A::NameProperty $name)))
}
function InvokeTitle([string]$name) { $b = TitleButton $name; if (-not $b) { return $false }; try { $b.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
$tbFound = WaitFor { ($null -ne (TitleButton "Minimize")) -and ($null -ne (TitleButton "Maximize")) -and ($null -ne (TitleButton "Close")) } 10
Check "title bar: the page draws Minimize, Maximize and Close" $tbFound
$ok = (InvokeTitle "Maximize") -and (WaitFor { [W]::IsZoomed((Hwnd)) } 5)
Check "title bar: Maximize (UIA) maximizes the window" $ok
$ok = (WaitFor { $null -ne (TitleButton "Restore") } 5) -and (InvokeTitle "Restore") -and (WaitFor { -not [W]::IsZoomed((Hwnd)) } 5)
$back = [W]::Bounds((Hwnd))
Check "title bar: Restore (UIA) restores it to where it was" ($ok -and $null -ne $back -and (Near $back[0] $GeoStart[0]) -and (Near $back[1] $GeoStart[1]) -and (Near $back[2] $GeoStart[2]) -and (Near $back[3] $GeoStart[3])) "before=$($GeoStart -join ',') after=$($back -join ',')"
# A point in the bar's empty middle: the drag region (40 logical px high).
function TitleDragPoint { $b = [W]::Bounds((Hwnd)); if ($null -eq $b) { return $null }; @(([int]($b[0] + $b[2] / 2)), ([int]($b[1] + 20 * $GeoScale))) }
$pt = TitleDragPoint
$free = ($null -ne $pt) -and ([W]::TopLevelAt($pt[0], $pt[1]) -eq (Hwnd))
Check "title bar: its middle is not covered by another window" $free "point=$($pt -join ',')"
if ($free) {
  [W]::DoubleClick($pt[0], $pt[1])
  Check "title bar: double click (real mouse) maximizes" (WaitFor { [W]::IsZoomed((Hwnd)) } 5)
  $pt = TitleDragPoint
  [W]::DoubleClick($pt[0], $pt[1])
  Check "title bar: double click again restores" (WaitFor { -not [W]::IsZoomed((Hwnd)) } 5)
  $pt = TitleDragPoint; $from = [W]::Bounds((Hwnd))
  [W]::Drag($pt[0], $pt[1], $pt[0] + 80, $pt[1] + 60)
  $GeoMoved = $null
  $moved = WaitFor { $script:GeoMoved = [W]::Bounds((Hwnd)); ($null -ne $script:GeoMoved) -and (Near $script:GeoMoved[0] ($from[0] + 80)) -and (Near $script:GeoMoved[1] ($from[1] + 60)) } 5
  Check "title bar: dragging (real mouse) moves the window, size unchanged" ($moved -and (Near $GeoMoved[2] $from[2]) -and (Near $GeoMoved[3] $from[3])) "from=$($from -join ',') to=$($GeoMoved -join ',')"
  # The frame without native decorations: tao keeps the styles that Snap,
  # Win+Arrow and the system menu use (it only hides the frame), the left,
  # right and bottom resize borders are the system's own (invisible, outside
  # the page) and the top edge is Tauri's resize strip above the drag region
  # (SM_CYFRAME high), so both edges must resize the window, not move it.
  $FrameStyle = [W]::GetWindowLong((Hwnd), -16)
  $FrameWant = 0x00C00000 -bor 0x00040000 -bor 0x00020000 -bor 0x00010000
  Check "frame: WS_CAPTION, WS_THICKFRAME, WS_MINIMIZEBOX and WS_MAXIMIZEBOX kept (Snap, Win+Arrow)" (($FrameStyle -band $FrameWant) -eq $FrameWant) ("style=0x{0:X8}" -f $FrameStyle)
  $from = [W]::Bounds((Hwnd))
  $cornerX = $from[0] + $from[2] - 3; $cornerY = $from[1] + $from[3] - 3
  [W]::Drag($cornerX, $cornerY, $cornerX + 60, $cornerY + 40)
  $GeoGrown = $null
  $grown = WaitFor { $script:GeoGrown = [W]::Bounds((Hwnd)); ($null -ne $script:GeoGrown) -and (Near $script:GeoGrown[2] ($from[2] + 60)) -and (Near $script:GeoGrown[3] ($from[3] + 40)) } 5
  Check "frame: dragging the bottom-right corner (real mouse) resizes the window, its position kept" ($grown -and (Near $GeoGrown[0] $from[0]) -and (Near $GeoGrown[1] $from[1])) "from=$($from -join ',') to=$($GeoGrown -join ',')"
  $from = [W]::Bounds((Hwnd))
  $edgeX = [int]($from[0] + $from[2] / 2); $edgeY = $from[1] + 1
  [W]::Drag($edgeX, $edgeY, $edgeX, $edgeY - 30)
  $GeoTaller = $null
  $taller = WaitFor { $script:GeoTaller = [W]::Bounds((Hwnd)); ($null -ne $script:GeoTaller) -and (Near $script:GeoTaller[1] ($from[1] - 30)) -and (Near $script:GeoTaller[3] ($from[3] + 30)) } 5
  Check "frame: dragging the top edge (real mouse) resizes the window, its bottom kept" ($taller -and (Near $GeoTaller[0] $from[0]) -and (Near $GeoTaller[2] $from[2])) "from=$($from -join ',') to=$($GeoTaller -join ',')"
}
$ok = (InvokeTitle "Minimize") -and (WaitFor { [W]::IsIconic((Hwnd)) } 5)
Check "title bar: Minimize (UIA) minimizes the window, process alive" ($ok -and (Procs).Count -eq 1)
[W]::ShowWindow((Hwnd), 9) | Out-Null
Check "title bar: restored from the taskbar state" (WaitFor { Visible } 5)

# --- built-in features (WebView2 content, real capability and IPC) ------------
# list_features and set_feature_enabled through the static capability: the page
# lists Scheduled backups (enabled by default) and switching it round-trips to
# Rust, whose answer updates the checkbox and the state label.
$heading = $null
$null = OpenFeatures
WaitFor { $script:heading = (Dash).FindFirst($S::Descendants, (PC $A::AutomationIdProperty "features-heading")); $script:heading -ne $null } 10 | Out-Null
Check "features: Settings > Features present in WebView2 content" (($null -ne $heading) -and $heading.Current.Name -eq "Features")
$listed = WaitFor { $v = FeatureView; ($null -ne $v) -and $v.Item -ne "" } 10
$v = FeatureView
Check "features: Scheduled backups listed with name, version and description" ($listed -and $v.Item.Contains($SwitchedVersion) -and $v.Item.Contains($SwitchedDescription)) (FeatureDetail)
Check "features: Scheduled backups checkbox checked and Running at startup" ($listed -and (SwitchedTo $true "Running")) (FeatureDetail)
$ok = (ToggleFeature) -and (WaitFor { SwitchedTo $false "Disabled" } 5)
Check "features: unchecking (UIA toggle) disables it" $ok (FeatureDetail)
$ok = (ToggleFeature) -and (WaitFor { SwitchedTo $true "Running" } 5)
Check "features: checking (UIA toggle) enables it" $ok (FeatureDetail)
$script:why = ""
$ok = (ClickFeature) -and (WaitFor { SwitchedTo $false "Disabled" } 5)
Check "features: real mouse click on the checkbox disables it" $ok "$script:why $(FeatureDetail)"
$script:why = ""
$ok = (ClickFeature) -and (WaitFor { SwitchedTo $true "Running" } 5)
Check "features: real mouse click re-enables it" $ok "$script:why $(FeatureDetail)"
$primary.Refresh()
Check "features: dashboard still responsive (window answers, visible, one process)" ($primary.Responding -and (Visible) -and (Procs).Count -eq 1 -and -not $primary.HasExited)

# --- Cloud Storage section (WebView2 content, real capability and IPC) -------
# The navigation links switch sections; list_cloud_providers and
# list_cloud_accounts are allowed by the static capability; with the account
# database moved aside the page shows its empty state. Listing never reads the
# Credential Manager. Then back to the overview.
function Named($name, $type) {
  (Dash).FindFirst($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $type), (PC $A::NameProperty $name)))
}
function PageNames { @((Dash).FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name }) -join " | " }
$HyperlinkType = [System.Windows.Automation.ControlType]::Hyperlink
$ButtonType = [System.Windows.Automation.ControlType]::Button
$cloudLink = $null
WaitFor { $script:cloudLink = Named "Cloud Storage" $HyperlinkType; $script:cloudLink -ne $null } 10 | Out-Null
Check "cloud storage: Overview and Cloud Storage navigation links present" (($null -ne $cloudLink) -and ($null -ne (Named "Overview" $HyperlinkType)))
if ($cloudLink) { $cloudLink.GetCurrentPattern($Invoke).Invoke() }
$shown = WaitFor { ($null -ne (Dash).FindFirst($S::Descendants, (PC $A::AutomationIdProperty "cloud-storage-heading"))) -and ((PageNames) -like "*No storage accounts yet*") } 10
$names = PageNames
Check "cloud storage: heading, empty state and Add account shown, no error" ($shown -and ($null -ne (Named "Add account" $ButtonType)) -and ($null -eq (Named "Hide dashboard" $ButtonType)) -and ($names -notlike "*Could not*") -and ($names -notlike "*could not be opened*")) $names
Check "cloud storage: account database created in the test profile" (Test-Path $CloudDb) $CloudDb
$overview = Named "Overview" $HyperlinkType
if ($overview) { $overview.GetCurrentPattern($Invoke).Invoke() }
Check "cloud storage: Overview link brings back the Hide button and the summaries" ((WaitFor { ($null -ne (Named "Hide dashboard" $ButtonType)) -and ($null -ne (Dash).FindFirst($S::Descendants, (PC $A::AutomationIdProperty "overview-storage"))) } 10)) (PageNames)

# --- Hide dashboard button (WebView2 content, UIA invoke = click) ---------
$btn = $null
WaitFor { $script:btn = (Dash).FindFirst($S::Descendants, (PC $A::NameProperty "Hide dashboard")); $script:btn -ne $null } 10 | Out-Null
Check "hide button: present in WebView2 content" ($null -ne $btn)
if ($btn) {
  $btn.GetCurrentPattern($Invoke).Invoke()
  Check "hide button (UIA invoke): dashboard hidden" (WaitFor { Hidden } 5)
  Check "hide button (UIA invoke): process alive" ((Procs).Count -eq 1 -and -not $primary.HasExited)
  Check "hide button (UIA invoke): window not destroyed" ((DashHwnds).Count -eq 1)
}

# --- tray left click opens dashboard ---------------------------------------
$icon = TrayIcon
if ($icon) {
  $r = $icon.Current.BoundingRectangle
  [W]::Click([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), $false)
  Check "tray left click (real mouse): dashboard shown" (WaitFor { Visible } 5)
  Start-Sleep -Milliseconds 500
  Check "tray left click: no tray context menu opened" (@([W]::Find("#32768", (Pids))).Count -eq 0)
  Info "tray left click: dashboard is foreground" (Foreground)
} else { Check "tray left click: icon found" $false }

# --- Hide button with a real mouse click -------------------------------------
$btn = (Dash).FindFirst($S::Descendants, (PC $A::NameProperty "Hide dashboard"))
if ($btn) {
  $r = $btn.Current.BoundingRectangle
  [W]::Click([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), $false)
  Check "hide button (real mouse click): dashboard hidden" (WaitFor { Hidden } 5)
  Check "hide button (real mouse click): process alive" ((Procs).Count -eq 1)
}

# --- tray menu Open Dashboard -------------------------------------------------
$ok = TrayMenu "Open Dashboard"; Check "tray menu Open Dashboard: invoked" $ok $script:why
Check "tray menu Open Dashboard: dashboard shown" (WaitFor { Visible } 5)
Check "tray menu Open Dashboard: one window" ((DashHwnds).Count -eq 1)

# --- close button hides ------------------------------------------------------
# The title bar is the page's own (ADR-040): its Close button, found through UIA
# and clicked with the real mouse once the dashboard is the window under it.
$closeBtn = TitleButton "Close"
$close = $null
if ($closeBtn) { $r = $closeBtn.Current.BoundingRectangle; if (-not $r.IsEmpty) { $close = @(([int]($r.X + $r.Width / 2)), ([int]($r.Y + $r.Height / 2))) } }
Check "close: the title bar's Close button located (UIA) and not covered" (($null -ne $close) -and ([W]::TopLevelAt($close[0], $close[1]) -eq (Hwnd)))
if ($close) {
  [W]::Click($close[0], $close[1], $false)
  Check "close (real click on X): dashboard hidden" (WaitFor { Hidden } 5)
  Check "close (real click on X): process alive" ((Procs).Count -eq 1 -and -not $primary.HasExited)
  Check "close (real click on X): window not destroyed" ((DashHwnds).Count -eq 1)
}
# A programmatic close request (what Alt+F4 or the taskbar's "Close window" also ask
# for). Alt+F4 itself is not sent: if focus moved, it would close another app.
$ok = TrayMenu "Open Dashboard"; Check "tray menu Open Dashboard again: invoked" $ok $script:why
Check "restored after close" (WaitFor { Visible } 5)
$d = Dash; if ($d) { ($d.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern)).Close() }
Check "close request (UIA WindowPattern.Close): dashboard hidden" (WaitFor { Hidden } 5)
Check "close request (UIA WindowPattern.Close): process alive" ((Procs).Count -eq 1)
Check "close request (UIA WindowPattern.Close): window not destroyed" ((DashHwnds).Count -eq 1)

# --- single instance -----------------------------------------------------------
SecondLaunch "hidden"
Check "hidden: second launch shows dashboard" (WaitFor { Visible } 5)
Info "hidden: dashboard is foreground after second launch" (Foreground)

SecondLaunch "visible"
Check "visible: dashboard still visible" (Visible)
Info "visible: dashboard is foreground after second launch" (Foreground)

[W]::ShowWindow((Hwnd), 6) | Out-Null   # SW_MINIMIZE
Check "minimized: dashboard is iconic" (WaitFor { [W]::IsIconic((Hwnd)) } 3)
SecondLaunch "minimized"
Check "minimized: second launch restores (not iconic, visible)" (WaitFor { Visible } 5)
Info "minimized: dashboard is foreground after second launch" (Foreground)

for ($i = 1; $i -le 5; $i++) { SecondLaunch "sequential $i" }
$conc = 1..4 | ForEach-Object { Start-Process $Exe -PassThru }
$allExit = $true; foreach ($c in $conc) { if (-not $c.WaitForExit(15000)) { $allExit = $false } }
Check "concurrent x4: all exited" $allExit
Check "concurrent x4: all exit code 0" (@($conc | Where-Object { $_.ExitCode -ne 0 }).Count -eq 0) "codes=$(($conc | % ExitCode) -join ',')"
Start-Sleep -Milliseconds 500
Check "concurrent x4: one process (the primary)" ((Procs).Count -eq 1 -and (Procs)[0].Id -eq $primary.Id)
Check "concurrent x4: one window" ((DashHwnds).Count -eq 1)
$n = TrayIconCount; Check "after all launches: exactly one tray icon" ($n -eq 1) "count=$n"

# --- quit and relaunch ------------------------------------------------------------
# Quit with Scheduled backups disabled; the relaunch (a new process) must keep it
# disabled, since the choice is saved. Then enable it, choose Dark, quit at once,
# relaunch: running, and Dark. Each relaunch waits until WebView2's processes
# of the quitting instance have ended (WebViewEnded).
$ok = (ToggleFeature) -and (WaitFor { SwitchedTo $false "Disabled" } 5)
Check "quit: Scheduled backups disabled before quitting" $ok (FeatureDetail)
# Where the dashboard is now (moved by the drag above): the relaunch must open it here.
$GeoQuit = [W]::Bounds((Hwnd))
# Since the redesign (ADR-035) the Hide button is on the Overview page only,
# and the switch above left Settings open: go back to Overview first.
$overview = Named "Overview" $HyperlinkType
if ($overview) { $overview.GetCurrentPattern($Invoke).Invoke() }
$btn = $null
WaitFor { $script:btn = Named "Hide dashboard" $ButtonType; $script:btn -ne $null } 10 | Out-Null
if ($btn) { $btn.GetCurrentPattern($Invoke).Invoke() }
Check "quit: dashboard hidden first (Hide button on Overview)" (WaitFor { Hidden } 5)
$WebViews = @(WatchWebView $primary)
$ok = TrayMenu "Quit"; Check "quit: tray Quit invoked" $ok $script:why
Check "quit: process exited" ($primary.WaitForExit(10000))
Check "quit: exit code 0" ($primary.ExitCode -eq 0) "code=$($primary.ExitCode)"
Check "quit: no sotyware process left" ((Procs).Count -eq 0)
WebViewEnded "quit" $primary $WebViews
Start-Sleep 2
$n = TrayIconCount; Check "quit: tray icon removed (overflow flyout open and inspected)" ((OverflowOpen) -and $n -eq 0) "count=$n overflowOpen=$(OverflowOpen)"
[W]::Esc()

# --- relaunch after quit -------------------------------------------------------------
$fresh = Start-Process $Exe -PassThru
Verified $fresh "relaunch"
Check "relaunch: fresh dashboard visible" (WaitFor { Visible } 15)
Check "relaunch: fresh process is the running primary" (-not $fresh.HasExited -and (Procs).Count -eq 1 -and (Procs)[0].Id -eq $fresh.Id)
Check "relaunch: one tray icon" ((WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 8) -and (TrayIcons).Count -eq 1)
Check "relaunch: preferences database exists" (Test-Path $PrefsDb) $PrefsDb
$GeoRelaunch = [W]::Bounds((Hwnd))
Check "relaunch: the dashboard opens where it was when sotyware quit (ADR-040)" ($null -ne $GeoQuit -and $null -ne $GeoRelaunch -and (Near $GeoRelaunch[0] $GeoQuit[0]) -and (Near $GeoRelaunch[1] $GeoQuit[1]) -and (Near $GeoRelaunch[2] $GeoQuit[2]) -and (Near $GeoRelaunch[3] $GeoQuit[3])) "quit=$($GeoQuit -join ',') relaunch=$($GeoRelaunch -join ',')"
Check "relaunch: window-state.sqlite3 in the test profile" (Test-Path (Join-Path $Prof.DataDir "window-state.sqlite3"))
Check "relaunch: Scheduled backups still Disabled, checkbox unchecked (persisted)" (WaitFor { SwitchedTo $false "Disabled" } 10) (FeatureDetail)
$ok = (ToggleFeature) -and (WaitFor { SwitchedTo $true "Running" } 5)
Check "relaunch: checking (UIA toggle) enables it, no error shown" $ok (FeatureDetail)

# --- the theme chosen just before Quit (ADR-035) ----------------------------------------
# As scenario 17 of single_instance.py on Linux. The theme lives in the
# webview's localStorage, which WebView2 writes to disk by itself: choose Dark
# in Settings > General, quit through the tray at once, wait until sotyware
# and this run's WebView2 processes have ended, and relaunch 2 must still show
# Dark. Here and not at the first quit, which hides the dashboard first, and
# with WebView2's processes listed before the choice: only the tray menu lies
# between the choice and Quit.
$WebViews = @(WatchWebView $fresh)
$darkChosen = ChooseTheme "Dark"
$DarkAt = [DateTime]::UtcNow
$darkDetail = if ($darkChosen) { "" } else { "shown=$((ThemesShown) -join ',')" }
$OnDiskAtChoice = ThemeOnDisk
$ok = TrayMenu "Quit"
$QuitAt = [DateTime]::UtcNow
Check "theme: Dark chosen in Settings > General (UIA radio button) just before the Quit" $darkChosen $darkDetail
Check "relaunch: tray Quit invoked" $ok $script:why
Check "relaunch: quit exits 0" ($fresh.WaitForExit(10000) -and $fresh.ExitCode -eq 0)
$OnDiskAtExit = ThemeOnDisk
Check "relaunch: no sotyware process left" ((Procs).Count -eq 0)
WebViewEnded "relaunch" $fresh $WebViews
$OnDiskAfter = ThemeOnDisk $true
$ThemeDir = LocalStorageDir $true
$ThemeDir = if ($ThemeDir) { $ThemeDir.Substring($Prof.Root.Length).TrimStart('\') } else { "not found" }
$sinceChoice = if ($null -ne $script:SotywareEnded) { "$([int]([DateTime]::FromFileTimeUtc($script:SotywareEnded) - $DarkAt).TotalMilliseconds) ms" } else { "?" }
Info "theme: timing" "Quit clicked $([int]($QuitAt - $DarkAt).TotalMilliseconds) ms after Dark showed as chosen; sotyware ended $sinceChoice after it"
Info "theme: the choice in WebView2's localStorage files (profile\$ThemeDir)" "just after it was made: $OnDiskAtChoice; once sotyware had ended: $OnDiskAtExit; once WebView2 had ended: $OnDiskAfter"
[W]::Esc()

# --- second relaunch: the enabled choice was saved too ---------------------------------
$again = Start-Process $Exe -PassThru
Verified $again "relaunch 2"
Check "relaunch 2: fresh dashboard visible" (WaitFor { Visible } 15)
Check "relaunch 2: a new process is the running primary" (-not $again.HasExited -and $again.Id -ne $fresh.Id -and (Procs).Count -eq 1 -and (Procs)[0].Id -eq $again.Id)
Check "relaunch 2: Scheduled backups Running, checkbox checked (persisted)" (WaitFor { SwitchedTo $true "Running" } 10) (FeatureDetail)
$kept = (OpenSettingsTab "General") -and (WaitFor { ((ThemesShown) -join ",") -eq "Dark" } 5)
Check "relaunch 2: the theme chosen just before quitting (Dark) is still chosen" ($darkChosen -and $kept) "chosen=$darkChosen shown=$((ThemesShown) -join ',')"
Check "relaunch 2: one tray icon" ((WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 8) -and (TrayIcons).Count -eq 1)
$WebViews = @(WatchWebView $again)
$ok = TrayMenu "Quit"; Check "relaunch 2: tray Quit invoked" $ok $script:why
Check "relaunch 2: quit exits 0" ($again.WaitForExit(10000) -and $again.ExitCode -eq 0)
WebViewEnded "relaunch 2" $again $WebViews
[W]::Esc()

# Anything still running means a quit path failed; report it and clean up.
$left = Procs
Check "cleanup: no sotyware process left" ($left.Count -eq 0) "pids=$($left.Id -join ',')"
$left | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
# A WebView2 process of this run still running (a wait above failed) is
# stopped by its id, found by this run's profile only, so that the profile
# can be removed.
$wvLeft = @(WebViewProcs)
Check "cleanup: no WebView2 process of this run left" ($wvLeft.Count -eq 0) "pids=$(($wvLeft | ForEach-Object { $_.ProcessId }) -join ',')"
$wvLeft | ForEach-Object { Stop-Process -Id $_.ProcessId -EA SilentlyContinue }

# Only this run's profile is removed; the user's own folders and credentials
# must be exactly as before.
$null = WaitFor { (Procs).Count -eq 0 -and (WebViewProcs).Count -eq 0 } 5
Remove-TestProfile $Prof
Check "cleanup: this run's test profile removed" (-not (Test-Path -LiteralPath $Prof.Root))
Check "the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|"))
Check "the user's own sotyware credentials are unchanged" (((Get-NormalCredentialLines) -join "|") -eq ($NormalCredBefore -join "|"))
$realAfter = Get-RealNormalProfileFingerprint $Prof.Parent
Check "the user's real app folder, read from outside this process tree, is unchanged" ($null -ne $realAfter -and (@($realAfter) -join "|") -eq (@($RealBefore) -join "|"))

""
"summary: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
