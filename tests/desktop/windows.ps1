# Native Windows desktop test of the sotyware release binary: window lifecycle,
# "Hide dashboard" button, tray icon and menu, single instance, Quit and relaunch.
#
# usage (PowerShell, from the repository root on a Windows path):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows.ps1 [-Exe <path>]
#   (default: src-tauri\target\release\sotyware.exe)
#
# This runs on your real desktop, not in a sandbox: it moves the mouse and clicks
# (tray icon, tray menu, the dashboard's button and title-bar close button), and
# opens the notification-area "Show Hidden Icons" flyout. Do not use the mouse or
# keyboard while it runs. It needs no extra modules: Windows PowerShell 5.1, UI
# Automation and user32 only.
#
# Each check prints PASS or FAIL; INFO lines record observations that Windows
# focus rules may legitimately change (which window is in the foreground).
# Exit code: 0 all passed, 1 a check failed, 2 binary missing or already running,
# 3 no interactive desktop (not the input desktop, or no taskbar): nothing was run.
param([string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\release\sotyware.exe"))
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run tauri build)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
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
function TrayIcons {
  @($root.FindAll($S::Descendants, (New-Object System.Windows.Automation.AndCondition (PC $A::AutomationIdProperty "NotifyItemIcon"), (PC $A::NameProperty "sotyware"))) |
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

# --- startup -------------------------------------------------------------
$primary = Start-Process $Exe -PassThru
Check "startup: dashboard window visible" (WaitFor { Visible } 15)
Check "startup: one process" ((Procs).Count -eq 1)
Check "startup: one dashboard window" ((DashHwnds).Count -eq 1)
Check "startup: dashboard title is sotyware" ((Dash).Current.Name -eq "sotyware")
Check "startup: exactly one sotyware tray icon" ((WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 8) -and (TrayIcons).Count -eq 1) "count=$((TrayIcons).Count)"
$items = MenuItems
Check "tray: menu items are Open Dashboard, Quit" (($items -join "|") -eq "Open Dashboard|Quit") "got=$($items -join '|')"
Check "tray: menu dismissed, process alive, still one process" ((Procs).Count -eq 1)

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
$close = [W]::FindClose((Hwnd))
Check "close: title-bar close button located (WM_NCHITTEST = HTCLOSE)" ($null -ne $close)
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
$btn = (Dash).FindFirst($S::Descendants, (PC $A::NameProperty "Hide dashboard"))
if ($btn) { $btn.GetCurrentPattern($Invoke).Invoke() }
Check "quit: dashboard hidden first" (WaitFor { Hidden } 5)
$ok = TrayMenu "Quit"; Check "quit: tray Quit invoked" $ok $script:why
Check "quit: process exited" ($primary.WaitForExit(10000))
Check "quit: exit code 0" ($primary.ExitCode -eq 0) "code=$($primary.ExitCode)"
Check "quit: no sotyware process left" ((Procs).Count -eq 0)
Start-Sleep 2
$n = TrayIconCount; Check "quit: tray icon removed (overflow flyout open and inspected)" ((OverflowOpen) -and $n -eq 0) "count=$n overflowOpen=$(OverflowOpen)"
[W]::Esc()

# --- relaunch after quit -------------------------------------------------------------
$fresh = Start-Process $Exe -PassThru
Check "relaunch: fresh dashboard visible" (WaitFor { Visible } 15)
Check "relaunch: fresh process is the running primary" (-not $fresh.HasExited -and (Procs).Count -eq 1 -and (Procs)[0].Id -eq $fresh.Id)
Check "relaunch: one tray icon" ((WaitFor { OpenOverflow; (TrayIcons).Count -ge 1 } 8) -and (TrayIcons).Count -eq 1)
$ok = TrayMenu "Quit"; Check "relaunch: tray Quit invoked" $ok $script:why
Check "relaunch: quit exits 0" ($fresh.WaitForExit(10000) -and $fresh.ExitCode -eq 0)
[W]::Esc()

# Anything still running means a quit path failed; report it and clean up.
$left = Procs
Check "cleanup: no sotyware process left" ($left.Count -eq 0) "pids=$($left.Id -join ',')"
$left | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }

""
"summary: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
