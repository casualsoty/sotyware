# The autoclicker and macros (ADR-050) on the REAL Windows desktop: opt-in,
# not part of CI. Drives the Autoclicker page of a test-profile build
# (ADR-027) through UI Automation and sends REAL input (SendInput: keys by
# scan code, so any keyboard layout works; the pointer and its buttons) into
# a topmost window of this script that records what it receives. sotyware's
# own clicks and keys land there too: before every run the script checks
# that its window is under each point sotyware will click (else it stops
# without starting anything). The same checks as the Linux phase
# (tests/desktop/cloud_ui_autoclicker.py), Windows' way: off by default,
# Turn on, a bounded run, F6/F7/F8 and a screen corner from another program,
# nothing clicked on sotyware's own window, recording with F9/F10 (a click
# on sotyware's window left out), playback, the right button and double
# clicks timed, a shortcut changed in its field and the keys it refuses,
# the confirmation without an emergency stop, repetitions at 2x, F7, F8,
# Shift+F11, the editor, a restart that keeps everything, a second profile,
# a macro's own shortcut, export and import through the native dialogs,
# held keys released by F8, by Stop and by quitting, 100 clicks per second,
# turning it off, and the emergency stop's key refused while it is off.
#
# IT MOVES THE POINTER, CLICKS AND PRESSES KEYS (F6 to F12, Shift, Ctrl,
# Alt, Escape, Backspace, A, J, K, M), and quits sotyware three times with
# the tray icon's Quit (a real right click in the "Show Hidden Icons"
# flyout). Run it only on an idle, unlocked desktop, and do not use the
# mouse or keyboard while it runs (about 8 minutes). It refuses to start
# without -ConfirmRealInput pointer-and-keys. Needs node on PATH (the
# autoclicker's database is read with Node's built-in node:sqlite).
#
# usage (PowerShell, repository root, after npm run build:test-profile):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-autoclicker.ps1 -ConfirmRealInput pointer-and-keys
# exit: 0 every check passed; 1 a check failed; 2 nothing run (binary,
# node or the confirmation missing); 3 no interactive desktop.
#
# ASCII only: Windows PowerShell 5.1 reads a file without a BOM as ANSI.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\test-profile\release\sotyware.exe"),
  [string]$ProfileParent = (Join-Path $PSScriptRoot "..\..\src-tauri\target\desktop-test-profiles"),
  [string]$ConfirmRealInput = ""
)
if ($ConfirmRealInput -cne "pointer-and-keys") {
  "ABORT: this test moves the pointer, clicks and presses keys on this desktop; run it on an idle desktop with -ConfirmRealInput pointer-and-keys. Nothing was run."; exit 2
}
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run build:test-profile)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
if (-not (Test-TestProfileBuild $Exe)) { "ABORT: $Exe is not a test-profile build (ADR-027); run: npm run build:test-profile"; exit 2 }
if (-not (Get-Command node -EA SilentlyContinue)) { "ABORT: node is not on PATH (the database is read with node:sqlite)"; exit 2 }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms
$ErrorActionPreference = "Stop"
Add-Type @"
using System; using System.Collections.Generic; using System.Diagnostics;
using System.Runtime.InteropServices; using System.Threading;
public static class AcW {
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, System.Text.StringBuilder l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, string l);
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern IntPtr OpenInputDesktop(uint f, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern IntPtr GetThreadDesktop(uint tid);
  [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformation(IntPtr h, int i, System.Text.StringBuilder b, int n, out int need);
  [DllImport("user32.dll")] static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] static extern bool UnregisterHotKey(IntPtr h, int id);
  [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vk);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint id, System.Text.StringBuilder s, int max, uint flag);
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint i, out RECT r);
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION u; }
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);

  public static string ClassOf(IntPtr h) { var sb = new System.Text.StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public static string DesktopName(IntPtr h) {
    if (h == IntPtr.Zero) return ""; var b = new System.Text.StringBuilder(256); int n;
    return GetUserObjectInformation(h, 2, b, 512, out n) ? b.ToString() : "";
  }
  public static IntPtr[] Find(string cls, uint[] pids) {
    var res = new List<IntPtr>();
    EnumWindows(delegate(IntPtr h, IntPtr l) { uint pid; GetWindowThreadProcessId(h, out pid);
      if (ClassOf(h) == cls && Array.IndexOf(pids, pid) >= 0) res.Add(h); return true; }, IntPtr.Zero);
    return res.ToArray();
  }
  // The top-level window under a screen point.
  public static IntPtr RootAt(int x, int y) { var p = new POINT(); p.X = x; p.Y = y; return GetAncestor(WindowFromPoint(p), 2); }
  public static int[] Rect(IntPtr h) { RECT r; if (!GetWindowRect(h, out r)) return null; return new int[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
  public static int[] Cursor() { POINT p; GetCursorPos(out p); return new int[] { p.X, p.Y }; }
  // Common item dialog controls by id (as in windows-cloud-ui.ps1).
  public static IntPtr ChildById(IntPtr parent, int id, string cls) {
    IntPtr found = IntPtr.Zero;
    EnumChildWindows(parent, delegate(IntPtr h, IntPtr l) {
      if (GetDlgCtrlID(h) == id && (cls == null || ClassOf(h) == cls)) { found = h; return false; }
      return true; }, IntPtr.Zero);
    return found;
  }
  public static string GetText(IntPtr h) { var sb = new System.Text.StringBuilder(4096); SendMessage(h, 0x000D, (IntPtr)4096, sb); return sb.ToString(); }
  public static void SetText(IntPtr h, string t) { SendMessage(h, 0x000C, IntPtr.Zero, t); }
  public static bool PostClick(IntPtr h) { return PostMessage(h, 0x00F5, IntPtr.Zero, IntPtr.Zero); }
  // The tray icon's popup menu (#32768): MN_GETHMENU and its strings.
  public static IntPtr MenuOf(IntPtr h) { return SendMessage(h, 0x01E1, IntPtr.Zero, IntPtr.Zero); }
  public static string[] MenuStrings(IntPtr hm) {
    int n = GetMenuItemCount(hm); var r = new string[Math.Max(n, 0)];
    for (int i = 0; i < n; i++) { var sb = new System.Text.StringBuilder(256); GetMenuString(hm, (uint)i, sb, 256, 0x400); r[i] = sb.ToString(); }
    return r;
  }

  // ---- real input, as from the user's keyboard and mouse ----
  static void Send(INPUT i) { var a = new INPUT[] { i }; SendInput(1, a, Marshal.SizeOf(typeof(INPUT))); }
  // A key by its scan code (KEYEVENTF_SCANCODE): the physical key, whatever the layout.
  public static void Key(int scan, bool up) {
    var i = new INPUT(); i.type = 1; i.u.ki.wScan = (ushort)scan;
    i.u.ki.dwFlags = 0x0008u | (up ? 0x0002u : 0u); Send(i);
  }
  public static void Tap(int scan) { Key(scan, false); Thread.Sleep(30); Key(scan, true); Thread.Sleep(30); }
  public static void Chord(int[] mods, int scan) {
    foreach (int m in mods) { Key(m, false); Thread.Sleep(25); }
    Tap(scan);
    for (int k = mods.Length - 1; k >= 0; k--) { Key(mods[k], true); Thread.Sleep(25); }
  }
  static uint ButtonFlag(int b, bool up) {
    if (b == 2) return up ? 0x0010u : 0x0008u;
    if (b == 3) return up ? 0x0040u : 0x0020u;
    return up ? 0x0004u : 0x0002u;
  }
  public static void Button(int b, bool up) { var i = new INPUT(); i.type = 0; i.u.mi.dwFlags = ButtonFlag(b, up); Send(i); }
  public static void ClickAt(int x, int y, int b) { SetCursorPos(x, y); Thread.Sleep(80); Button(b, false); Thread.Sleep(50); Button(b, true); Thread.Sleep(30); }
  // Keys and buttons down now (virtual-key codes, hex); the toggles' lock state is not "down".
  public static string Held() {
    var l = new List<string>();
    for (int vk = 1; vk < 255; vk++) {
      if (vk == 0x14 || vk == 0x90 || vk == 0x91) continue;
      if ((GetAsyncKeyState(vk) & 0x8000) != 0) l.Add(vk.ToString("X2"));
    }
    return string.Join(",", l.ToArray());
  }
  public static bool IsDown(int vk) { return (GetAsyncKeyState(vk) & 0x8000) != 0; }
  // Lets go of what is still down after a failed step (nothing that is up).
  public static void ReleaseAll() {
    int[] scans = { 0x2A, 0x36, 0x1D, 0x38, 0x1E }; int[] vks = { 0xA0, 0xA1, 0xA2, 0xA4, 0x41 };
    for (int k = 0; k < scans.Length; k++) if (IsDown(vks[k])) Key(scans[k], true);
    if (IsDown(0x01)) Button(1, true); if (IsDown(0x02)) Button(2, true); if (IsDown(0x04)) Button(3, true);
  }
  // Whether this script can register the shortcut itself: not while
  // another program (sotyware) holds it. Probed only while sotyware is
  // not about to register it, so it never takes one away from it.
  public static bool CanRegister(uint mods, int scan) {
    uint vk = MapVirtualKey((uint)scan, 1);
    if (!RegisterHotKey(IntPtr.Zero, 0xBEE0, mods | 0x4000u, vk)) return false;
    UnregisterHotKey(IntPtr.Zero, 0xBEE0); return true;
  }
}

// The program being automated: a topmost window that records every
// button and key it receives, with a microsecond time (Stopwatch).
public static class AcTarget {
  delegate IntPtr WndProcD(IntPtr h, uint m, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  struct WNDCLASS { public uint style; public WndProcD proc; public int cls, wnd; public IntPtr inst, icon, cursor, brush; public string menu, name; }
  [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr h; public uint m; public IntPtr w, l; public uint t; public int x, y; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern ushort RegisterClass(ref WNDCLASS c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowEx(uint ex, string cls, string name, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
  [DllImport("user32.dll")] static extern IntPtr DefWindowProc(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern int GetMessage(out MSG m, IntPtr h, uint a, uint b);
  [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
  [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG m);
  [DllImport("user32.dll")] static extern void PostQuitMessage(int c);
  [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern IntPtr GetModuleHandle(string n);
  [DllImport("user32.dll")] static extern IntPtr LoadCursor(IntPtr inst, IntPtr name);
  [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint c);
  static WndProcD keep; static IntPtr hwnd;
  static readonly object gate = new object();
  static readonly List<long[]> events = new List<long[]>();
  public static IntPtr Handle { get { return hwnd; } }
  public static int Count { get { lock (gate) { return events.Count; } } }
  // [kind, detail, x, y, microseconds]: kind 1 key down, 2 key up (detail:
  // scan code), 3 button down, 4 button up (detail: 1 left, 2 right, 3 middle).
  public static long[][] Since(int mark) {
    lock (gate) {
      var r = new long[Math.Max(0, events.Count - mark)][];
      for (int i = mark; i < events.Count; i++) r[i - mark] = events[i];
      return r;
    }
  }
  static void Note(long kind, long detail, IntPtr l, bool pos) {
    long v = (long)l, x = 0, y = 0;
    if (pos) { x = (short)(v & 0xFFFF); y = (short)((v >> 16) & 0xFFFF); }
    long us = Stopwatch.GetTimestamp() * 1000000L / Stopwatch.Frequency;
    lock (gate) { events.Add(new long[] { kind, detail, x, y, us }); }
  }
  static IntPtr Proc(IntPtr h, uint m, IntPtr w, IntPtr l) {
    switch (m) {
      case 0x0201: Note(3, 1, l, true); break;
      case 0x0202: Note(4, 1, l, true); break;
      case 0x0204: Note(3, 2, l, true); break;
      case 0x0205: Note(4, 2, l, true); break;
      case 0x0207: Note(3, 3, l, true); break;
      case 0x0208: Note(4, 3, l, true); break;
      case 0x0100: case 0x0104: Note(1, ((long)l >> 16) & 0x1FF, l, false); break;
      case 0x0101: case 0x0105: Note(2, ((long)l >> 16) & 0x1FF, l, false); break;
      case 0x0010: DestroyWindow(h); return IntPtr.Zero;
      case 0x0002: PostQuitMessage(0); return IntPtr.Zero;
    }
    return DefWindowProc(h, m, w, l);
  }
  // A topmost popup at (x, y) of w x h physical pixels, on a thread of its
  // own with its message loop. No CS_DBLCLKS: a double click arrives as two
  // button-down messages.
  public static IntPtr Open(int x, int y, int w, int hgt) {
    var ready = new ManualResetEvent(false);
    var t = new Thread(delegate() {
      AcW.SetThreadDpiAwarenessContext(new IntPtr(-4));
      keep = Proc;
      var wc = new WNDCLASS();
      wc.proc = keep; wc.inst = GetModuleHandle(null); wc.name = "sotyware-e2e-target";
      wc.cursor = LoadCursor(IntPtr.Zero, new IntPtr(32512)); wc.brush = CreateSolidBrush(0x00F0D8C0);
      RegisterClass(ref wc);
      hwnd = CreateWindowEx(0x00000008u | 0x00000080u, "sotyware-e2e-target", "sotyware e2e target",
        0x80000000u | 0x10000000u, x, y, w, hgt, IntPtr.Zero, IntPtr.Zero, wc.inst, IntPtr.Zero);
      ready.Set();
      MSG msg;
      while (GetMessage(out msg, IntPtr.Zero, 0, 0) > 0) { TranslateMessage(ref msg); DispatchMessage(ref msg); }
    });
    t.IsBackground = true; t.Start(); ready.WaitOne(5000);
    return hwnd;
  }
  public static void Close() { if (hwnd != IntPtr.Zero) PostMessage(hwnd, 0x0010, IntPtr.Zero, IntPtr.Zero); }
}
"@
$ErrorActionPreference = "Continue"
# Physical pixels everywhere (pointer, windows, UI Automation rectangles),
# as sotyware uses them (ADR-050).
[void][AcW]::SetThreadDpiAwarenessContext([IntPtr](-4))

# Only on the unlocked, interactive desktop with a taskbar (as windows.ps1).
$inputDesk = [AcW]::DesktopName([AcW]::OpenInputDesktop(0, $false, 0x0001))
$ownDesk = [AcW]::DesktopName([AcW]::GetThreadDesktop([AcW]::GetCurrentThreadId()))
if ($inputDesk -eq "" -or $inputDesk -ne $ownDesk) { "ABORT: not on the interactive input desktop (this desktop: '$ownDesk', input desktop: '$inputDesk'); this test needs an unlocked, logged-in desktop session"; exit 3 }
if ([AcW]::FindWindow("Shell_TrayWnd", $null) -eq [IntPtr]::Zero) { "ABORT: no taskbar (Shell_TrayWnd) on this desktop; quitting needs the tray icon"; exit 3 }

$A = [System.Windows.Automation.AutomationElement]; $S = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]
$Invoke = [System.Windows.Automation.InvokePattern]::Pattern
$ValueP = [System.Windows.Automation.ValuePattern]::Pattern
$ToggleP = [System.Windows.Automation.TogglePattern]::Pattern
$SelItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
$ExpColl = [System.Windows.Automation.ExpandCollapsePattern]::Pattern
$script:pass = 0; $script:fail = 0
function Check($name, [bool]$ok, $detail = "") {
  if ($ok) { $script:pass++; "PASS  $name" } else { $script:fail++; $d = "$detail"; if ($d.Length -gt 900) { $d = $d.Substring(0, 900) }; "FAIL  $name  $d" }
}
function WaitFor([scriptblock]$pred, [double]$timeout = 10) {
  $end = (Get-Date).AddSeconds($timeout)
  while ((Get-Date) -lt $end) { if (& $pred) { return $true }; Start-Sleep -Milliseconds 150 }
  return [bool](& $pred)
}
# Non-ASCII text of the page, built from code points (see the top).
$Times = [string][char]0x00D7; $Dot = [string][char]0x00B7; $Lq = [string][char]0x201C; $Rq = [string][char]0x201D
$Ellipsis = [string][char]0x2026

# ---- sotyware, its window and its page ----
function Procs { @(Get-Process sotyware -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }
function DashHwnd { $p = [uint32[]]@((Procs).Id); if ($p.Count -eq 0) { return [IntPtr]::Zero }; $h = @([AcW]::Find("Tauri Window", $p)); if ($h.Count -eq 0) { [IntPtr]::Zero } else { $h[0] } }
function Dash { $h = DashHwnd; if ($h -eq [IntPtr]::Zero) { $null } else { $A::FromHandle($h) } }
function PCT($type) { [System.Windows.Automation.PropertyCondition]::new($A::ControlTypeProperty, $type) }
function PCN($name) { [System.Windows.Automation.PropertyCondition]::new($A::NameProperty, $name) }
function PCA($id) { [System.Windows.Automation.PropertyCondition]::new($A::AutomationIdProperty, $id) }
function Names { $d = Dash; if (-not $d) { return @() }; try { @($d.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name }) } catch { @() } }
function PageText { (Names) -join " | " }
# The page's text with its pieces joined as they read: a sentence made of
# several text nodes ("Autoclicker" ": " "Finished: 4 clicks") reads whole.
function Flat { -join @(Names) }
function Says([string]$s) { (Flat).Contains($s) }
function Btns { $d = Dash; if (-not $d) { return @() }; try { @($d.FindAll($S::Descendants, (PCT $CT::Button)) | ForEach-Object { $_.Current.Name.Trim() }) } catch { @() } }
# The last element of that type and name (dialogs come last in the page).
function Named($name, $type) {
  $d = Dash; if (-not $d) { return $null }
  $all = @($d.FindAll($S::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $type), (PCN $name))))
  if ($all.Count -eq 0) { $null } else { $all[$all.Count - 1] }
}
function Prefixed([string]$prefix, $type) {
  $d = Dash; if (-not $d) { return $null }
  @($d.FindAll($S::Descendants, (PCT $type)) | Where-Object { $_.Current.Name.StartsWith($prefix, [StringComparison]::Ordinal) })[0]
}
# WaitFor runs its block in a child scope: results go through $script: variables.
function Press([string]$name) {
  $script:found = $null
  WaitFor { $script:found = Named $name $CT::Button; ($null -ne $script:found) -and $script:found.Current.IsEnabled } 10 | Out-Null
  if (-not $script:found) { Write-Host "  no button '$name'; buttons: $((Btns | Select-Object -First 50) -join ', ')"; return $false }
  try { $script:found.GetCurrentPattern($Invoke).Invoke(); $true } catch { Write-Host "  press '$name': $($_.Exception.Message)"; $false }
}
function Link([string]$name) { $l = Named $name $CT::Hyperlink; if (-not $l) { return $false }; try { $l.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
function Tab([string]$name) {
  $t = Named $name $CT::TabItem
  if (-not $t) { Write-Host "  no tab '$name'"; return $false }
  try { $t.GetCurrentPattern($Invoke).Invoke(); return $true } catch { }
  try { $t.GetCurrentPattern($SelItem).Select(); $true } catch { Write-Host "  tab '$name': $($_.Exception.Message)"; $false }
}
function Radio([string]$name) {
  $script:rb = $null
  WaitFor { $script:rb = @((Dash).FindAll($S::Descendants, (PCT $CT::RadioButton)) | Where-Object { $_.Current.Name.Trim().StartsWith($name, [StringComparison]::Ordinal) })[0]; $null -ne $script:rb } 5 | Out-Null
  if (-not $script:rb) { Write-Host "  no option '$name'"; return $false }
  try { $script:rb.GetCurrentPattern($SelItem).Select(); Start-Sleep -Milliseconds 250; $true } catch { Write-Host "  option '$name': $($_.Exception.Message)"; $false }
}
# A number or text field by name, set through the Value pattern (WebView2
# fires the input and change events the page listens to).
function SetField([string]$name, [string]$value) {
  $script:fld = $null
  WaitFor {
    $d = Dash; if (-not $d) { return $false }
    $script:fld = @($d.FindAll($S::Descendants, (PCN $name)) | Where-Object { $t = $_.Current.ControlType; ($t -eq $CT::Edit -or $t -eq $CT::Spinner) -and $_.Current.IsEnabled })[0]
    $null -ne $script:fld
  } 5 | Out-Null
  if (-not $script:fld) { Write-Host "  no field '$name'"; return $false }
  try { $script:fld.GetCurrentPattern($ValueP).SetValue($value); Start-Sleep -Milliseconds 300; return $true } catch { }
  # A number input may offer only the RangeValue pattern.
  try { $script:fld.GetCurrentPattern([System.Windows.Automation.RangeValuePattern]::Pattern).SetValue([double]$value); Start-Sleep -Milliseconds 300; $true } catch { Write-Host "  field '$name': $($_.Exception.Message)"; $false }
}
# Chooses `option` in the <select> `label` (its option's SelectionItem).
function Choose([string]$label, [string]$option) {
  $cb = Prefixed $label $CT::ComboBox
  if (-not $cb) { Write-Host "  no select '$label'"; return $false }
  $opts = @($cb.FindAll($S::Descendants, (PCT $CT::ListItem)))
  if ($opts.Count -eq 0) { try { $cb.GetCurrentPattern($ExpColl).Expand(); Start-Sleep -Milliseconds 400 } catch { }; $opts = @($cb.FindAll($S::Descendants, (PCT $CT::ListItem))) }
  $opt = @($opts | Where-Object { $_.Current.Name.Trim() -ceq $option })[0]
  if (-not $opt) { Write-Host "  select '$label' has no option '$option': $(($opts | ForEach-Object { $_.Current.Name } | Select-Object -First 40) -join ', ')"; return $false }
  try { $opt.GetCurrentPattern($SelItem).Select() } catch { Write-Host "  select '$label': $($_.Exception.Message)"; return $false }
  try { $cb.GetCurrentPattern($ExpColl).Collapse() } catch { }
  Start-Sleep -Milliseconds 300; $true
}
function ToggleNamed([string]$name, $type) {
  $e = Named $name $type
  if (-not $e) { Write-Host "  no toggle '$name'"; return $false }
  try { $e.GetCurrentPattern($ToggleP).Toggle(); $true } catch { try { $e.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
}
function Center($el) { $r = $el.Current.BoundingRectangle; @([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2)) }
function ClickEl($el) { $c = Center $el; [AcW]::ClickAt($c[0], $c[1], 1) }

# A shortcut field is a button named "<label>: <keys>". A person clicks it
# and presses the keys: so does this script (a real click, real keys).
function Field([string]$label) { Prefixed "${label}: " $CT::Button }
function Listening([string]$label) { $f = Field $label; ($null -ne $f) -and $f.Current.Name.Contains("press the keys") }
function SetShortcut([string]$label, [scriptblock]$keys) {
  $script:hk = $null
  WaitFor { $script:hk = Field $label; $null -ne $script:hk } 5 | Out-Null
  if (-not $script:hk) { Write-Host "  no shortcut field '$label'"; return $false }
  ClickEl $script:hk
  if (-not (WaitFor { Listening $label } 3)) { Write-Host "  shortcut field '$label' does not listen: '$((Field $label).Current.Name)'"; return $false }
  & $keys
  $true
}
# The state column of the Settings tab's row for `label`: the first known
# state after its field, in reading order.
function RowState([string]$label) {
  $n = @(Names)
  for ($i = 0; $i -lt $n.Count; $i++) {
    if ($n[$i].StartsWith("${label}: ", [StringComparison]::Ordinal)) {
      for ($j = $i + 1; $j -lt [Math]::Min($n.Count, $i + 8); $j++) {
        $t = $n[$j].Trim()
        if ($t -in @("Ready", "Not set", "Off", "Used while the autoclicker is on", $Ellipsis) -or $t.StartsWith("Not registered")) { return $t }
      }
      return $null
    }
  }
  $null
}
function Registered { @(Names | Where-Object { $_ -ceq "registered" }).Count }
# A feature's state in Settings > Features (Running, Disabled, Failed, ...).
function FeatureState([string]$name) {
  $cb = Named $name $CT::CheckBox; if (-not $cb) { return $null }
  $w = [System.Windows.Automation.TreeWalker]::RawViewWalker; $e = $cb
  for ($k = 0; $k -lt 4 -and $e; $k++) {
    $e = $w.GetParent($e); if (-not $e) { break }
    $t = -join @($e.FindAll($S::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name })
    $m = [regex]::Match($t, [regex]::Escape($Dot) + "\s*(\w+)")
    if ($m.Success) { return $m.Groups[1].Value }
  }
  $null
}
function Idle { $b = @(Btns); ($b -contains "Start autoclicker") -and -not ($b -contains "Stop") }

# ---- the autoclicker's database, read with node:sqlite ----
$NodeJs = "const {DatabaseSync}=require('node:sqlite');const d=new DatabaseSync(process.argv[1],{readOnly:true});" +
  "const s=d.prepare('SELECT data FROM settings WHERE id = 1').get();" +
  "console.log(JSON.stringify({settings:s?JSON.parse(s.data):null," +
  "profiles:d.prepare('SELECT data FROM profiles ORDER BY position, id').all().map(r=>JSON.parse(r.data))," +
  "macros:d.prepare('SELECT data FROM macros ORDER BY id').all().map(r=>JSON.parse(r.data))}))"
function DocsRaw { (& node --no-warnings -e $NodeJs $AutoDb 2>$null) -join "" }
function Docs { $raw = DocsRaw; if (-not $raw) { return $null }; $raw | ConvertFrom-Json }
function FirstProfile { $d = Docs; if (-not $d) { return $null }; @($d.profiles)[0] }
function ProfileNamed([string]$n) { $d = Docs; if (-not $d) { return $null }; @($d.profiles | Where-Object { $_.name -ceq $n })[0] }
function MacroNamed([string]$n) { $d = Docs; if (-not $d) { return $null }; @($d.macros | Where-Object { $_.name -ceq $n })[0] }
function Bind($doc, [string]$action) { if (-not $doc -or -not $doc.hotkeys) { return $null }; $b = $doc.hotkeys.$action; if ($b) { $b.chord } else { $null } }
function EmergencyChord { $d = Docs; if (-not $d -or -not $d.settings) { return "unread" }; $d.settings.emergencyStop.chord }
function ClickerSaved([scriptblock]$pred) { $script:cs = $null; WaitFor { $p = FirstProfile; $script:cs = if ($p) { $p.clicker } else { $null }; ($null -ne $script:cs) -and (& $pred $script:cs) } 10 }

# ---- the test window's events ----
function Mark { [AcTarget]::Count }
function Ev([int]$mark) { @([AcTarget]::Since($mark)) }
function Downs([int]$mark) { @(Ev $mark | Where-Object { $_[0] -eq 3 } | ForEach-Object { "$($_[1])@$($_[2]),$($_[3])" }) }
function Ups([int]$mark) { @(Ev $mark | Where-Object { $_[0] -eq 4 } | ForEach-Object { "$($_[1])@$($_[2]),$($_[3])" }) }
function DownTimes([int]$mark) { @(Ev $mark | Where-Object { $_[0] -eq 3 } | ForEach-Object { [double]$_[4] / 1000.0 }) }
# What a macro did: button presses ("b@x,y") and presses of the key A
# (scan code 0x1E, "A") in order.
function Seq([int]$mark) { @(Ev $mark | Where-Object { $_[0] -eq 3 -or ($_[0] -eq 1 -and $_[1] -eq 0x1E) } | ForEach-Object { if ($_[0] -eq 3) { "b@$($_[2]),$($_[3])" } else { "A" } }) }
function NoneHeld { [AcW]::Held() -eq "" }
function ShiftHeld { [AcW]::IsDown(0xA0) }
# Scan codes and RegisterHotKey modifiers.
$F6 = 0x40; $F7 = 0x41; $F8 = 0x42; $F9 = 0x43; $F10 = 0x44; $F11 = 0x57; $F12 = 0x58
$Esc = 0x01; $Bksp = 0x0E; $LShift = 0x2A; $LCtrl = 0x1D; $LAlt = 0x38
$KA = 0x1E; $KJ = 0x24; $KK = 0x25; $KM = 0x32
$MAlt = 1; $MCtrl = 2; $MShift = 4
$Six = @(@(0, $F6, "F6"), @(0, $F7, "F7"), @(0, $F8, "F8"), @(0, $F9, "F9"), @(0, $F10, "F10"), @($MShift, $F11, "Shift+F11"))
function Free { @($Six | Where-Object { [AcW]::CanRegister([uint32]$_[0], $_[1]) } | ForEach-Object { $_[2] }) }

# The tray (as in windows-cloud-ui.ps1): the icon sits in the "Show Hidden
# Icons" flyout; its menu is opened with a real right click and read through
# MN_GETHMENU, then Quit is clicked with the real mouse.
$root = $A::RootElement
$OverflowClass = "TopLevelWindowForOverflowXamlIsland"
function OverflowShown { $o = $root.FindFirst($S::Children, [System.Windows.Automation.PropertyCondition]::new($A::ClassNameProperty, $OverflowClass)); ($null -ne $o) -and -not $o.Current.BoundingRectangle.IsEmpty }
function OverflowOpen { (OverflowShown) -and [AcW]::ClassOf([AcW]::GetForegroundWindow()) -eq $OverflowClass }
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
function TrayQuit {
  $script:icon = $null
  WaitFor { OpenOverflow; $script:icon = @(TrayIcons)[0]; $null -ne $script:icon } 8 | Out-Null
  if (-not $script:icon) { $script:why = "tray icon not found"; return $false }
  $r = $script:icon.Current.BoundingRectangle
  [AcW]::ClickAt([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2), 2)
  $script:menuWnd = [IntPtr]::Zero
  WaitFor { $mw = @([AcW]::Find("#32768", [uint32[]]@((Procs).Id))); if ($mw.Count -ge 1) { $script:menuWnd = $mw[0] }; $script:menuWnd -ne [IntPtr]::Zero } 4 | Out-Null
  if ($script:menuWnd -eq [IntPtr]::Zero) { $script:why = "context menu did not open"; return $false }
  $hm = [AcW]::MenuOf($script:menuWnd)
  $idx = [array]::IndexOf([AcW]::MenuStrings($hm), "Quit")
  if ($idx -lt 0) { $script:why = "no Quit in the menu: $([AcW]::MenuStrings($hm) -join '|')"; [AcW]::Tap($Esc); return $false }
  $rc = New-Object AcW+RECT
  if (-not [AcW]::GetMenuItemRect([IntPtr]::Zero, $hm, [uint32]$idx, [ref]$rc)) { $script:why = "GetMenuItemRect failed"; [AcW]::Tap($Esc); return $false }
  [AcW]::ClickAt([int](($rc.L + $rc.R) / 2), [int](($rc.T + $rc.B) / 2), 1)
  $true
}
# Quits through the tray; the exit code, or $null if it did not end.
function QuitApp {
  $script:why = ""
  if (-not (TrayQuit)) { Write-Host "  tray Quit: $($script:why)"; return $null }
  if (-not $script:app.WaitForExit(30000)) { return $null }
  $script:app.ExitCode
}
# ADR-027: a launch of its own, in this run's profile, acknowledged before
# anything on the page is touched.
function StartApp {
  if ((Procs).Count -gt 0) { Write-Host "  sotyware still runs; not started again"; return $false }
  Clear-TestProfileAck $Prof
  $script:app = Start-Process $Exe -PassThru; $null = $script:app.Handle
  try { $null = Wait-TestProfileAck $Prof $script:app.Id 30 $script:app; Check "launch: the app runs in this run's test profile (acknowledged)" $true | Out-Host }
  catch {
    Check "launch: the app runs in this run's test profile (acknowledged)" $false "$_" | Out-Host
    Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
    throw "ABORT: the app is not provably isolated; nothing else is run"
  }
  WaitFor { (DashHwnd) -ne [IntPtr]::Zero -and $null -ne (Named "Autoclicker" $CT::Hyperlink) } 30
}
# The common item dialogs of export and import: File name (save: Edit
# 1001; open: Edit 1148) and OK (1) by control id, through window messages.
function DialogNamed([string]$title) {
  $p = [uint32[]]@((Procs).Id); if ($p.Count -eq 0) { return [IntPtr]::Zero }
  foreach ($h in [AcW]::Find("#32770", $p)) { if (($A::FromHandle($h)).Current.Name -eq $title) { return $h } }
  [IntPtr]::Zero
}
function DialogPath([string]$title, [int]$box, [string]$path) {
  $script:dlgW = [IntPtr]::Zero
  if (-not (WaitFor { $script:dlgW = DialogNamed $title; ($script:dlgW -ne [IntPtr]::Zero) -and ([AcW]::ChildById($script:dlgW, $box, "Edit") -ne [IntPtr]::Zero) } 20)) { Write-Host "  no dialog '$title' with its File name box"; return $false }
  $b = [AcW]::ChildById($script:dlgW, $box, "Edit")
  if (-not (WaitFor { if ([AcW]::GetText($b) -ne $path) { [AcW]::SetText($b, $path); Start-Sleep -Milliseconds 300 }; [AcW]::GetText($b) -eq $path } 8)) { Write-Host "  dialog '$title': File name says '$([AcW]::GetText($b))'"; return $false }
  [void][AcW]::PostClick([AcW]::ChildById($script:dlgW, 1, "Button"))
  WaitFor { (DialogNamed $title) -eq [IntPtr]::Zero } 10
}

# ---- test profile (ADR-027) ----
$run = Get-Date -Format "yyyyMMddHHmmss"
$NormalBefore = Get-NormalProfileFingerprint
$Prof = New-TestProfile $ProfileParent "autoclicker-$run"
$RealBefore = Get-RealNormalProfileFingerprint $Prof.Parent
if ($null -eq $RealBefore) {
  Check "the user's real app folder can be read from outside this process tree before the run" $false "WMI could not start the reader"
  Remove-TestProfile $Prof
  "ABORT: isolation cannot be proven; sotyware was not started"; exit 1
}
$env:SOTYWARE_TEST_PROFILE = $Prof.Root
$AutoDb = Join-Path $Prof.DataDir "automation.sqlite3"
$Exported = Join-Path $env:TEMP "sotyware-e2e-macro-$run.sotyware.json"
"test profile: $($Prof.Root)"
$PointerBefore = [AcW]::Cursor()

try {
  # ---- sotyware and the test window, side by side ----
  $up = StartApp
  Check "startup: dashboard window" $up
  if (-not $up) { throw "ABORT: no dashboard window; nothing else is run" }
  $wa = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
  $dh = DashHwnd
  # Leave room on the right of the work area for the test window.
  $dw = [Math]::Min(1200, $wa.Width - 480); $dhh = [Math]::Min(820, $wa.Height - 80)
  [void][AcW]::SetWindowPos($dh, [IntPtr]::Zero, $wa.X + 20, $wa.Y + 20, $dw, $dhh, 0x0014)
  $null = WaitFor { $r = [AcW]::Rect($dh); $r[2] -eq $dw } 5
  $D = [AcW]::Rect($dh)
  $tw = 360; $th = 260; $tx = $wa.X + $wa.Width - $tw - 40; $ty = $wa.Y + 80
  $T = [AcTarget]::Open($tx, $ty, $tw, $th)
  $TR = [AcW]::Rect($T)
  $clear = $TR -and ($TR[0] -ge $D[0] + $D[2] + 10)
  Check "the test window is up, clear of the dashboard" ($T -ne [IntPtr]::Zero -and $clear) "window $($TR -join ','); dashboard $($D -join ',')"
  if (-not $clear) { throw "ABORT: the test window would overlap the dashboard" }
  $CLICK = @(($tx + 180), ($ty + 130)); $PA = @(($tx + 60), ($ty + 60)); $PB = @(($tx + 290), ($ty + 200))
  function Rel($p) { "$($p[0] - $tx),$($p[1] - $ty)" }
  # sotyware's clicks may go only where this window is.
  function Covered { foreach ($p in @($CLICK, $PA, $PB)) { if ([AcW]::RootAt($p[0], $p[1]) -ne $T) { return $false } }; $true }
  function MustBeCovered { if (-not (Covered)) { throw "ABORT: something covers the test window; nothing more is sent" } }
  # Brings the test window to the front with a click on its edge (as a
  # person would); events are marked after it.
  function Front { [AcW]::ClickAt($tx + $tw - 8, $ty + $th - 8, 1); WaitFor { [AcW]::GetForegroundWindow() -eq $T } 3 }
  MustBeCovered

  # ---- off by default ----
  $ok = (WaitFor { Link "Autoclicker" } 10) -and (WaitFor { Says "The autoclicker is off." } 15)
  Check "the Autoclicker page says it is off (off by default)" $ok (PageText)
  Check "while it is off nothing is registered: this script can register F6 and F8 itself" ([AcW]::CanRegister(0, $F6) -and [AcW]::CanRegister(0, $F8))
  $null = Front; $m = Mark
  [AcW]::Tap($F6); Start-Sleep -Seconds 1
  Check "F6 does nothing while it is off" ((Downs $m).Count -eq 0 -and -not (Says "Clicking"))

  # ---- turn on ----
  $ok = (Press "Turn on") -and (WaitFor { -not (Says "The autoclicker is off.") -and (Registered) -ge 6 } 15)
  Check "Turn on registers the six suggested shortcuts (F6, F7, F8, F9, F10, Shift+F11)" ($ok -and -not (Says "could not be registered")) (PageText)
  $f = Free
  Check "sotyware holds them: this script can register none of them" ($f.Count -eq 0) "still free: $($f -join ', ')"

  # ---- a bounded run from the Clicker tab, at a fixed position ----
  $ok = (Tab "Clicker") -and (Radio "At a fixed position") -and (SetField "X" "$($CLICK[0])") -and (SetField "Y" "$($CLICK[1])")
  $ok = $ok -and (Radio "Stop after") -and (SetField "Number of clicks" "25") -and (SetField "Speed" "20")
  $saved = ClickerSaved { param($c) $c.target.mode -eq "points" -and @($c.target.points).Count -eq 1 -and @($c.target.points)[0].x -eq $CLICK[0] -and @($c.target.points)[0].y -eq $CLICK[1] -and $c.maxClicks -eq 25 -and [Math]::Abs($c.intervalMs - 50) -lt 0.01 }
  Check "the Clicker tab saves a fixed position, 25 clicks and 20 per second to the profile" ($ok -and $saved) ($script:cs | ConvertTo-Json -Compress -Depth 6)
  MustBeCovered; $m = Mark
  $ok = Press "Start autoclicker"
  $early = WaitFor { (Downs $m).Count -gt 0 } 2
  $null = WaitFor { (Ups $m).Count -ge 25 } 20; Start-Sleep -Milliseconds 500
  $ds = Downs $m; $us = Ups $m; $here = "1@$(Rel $CLICK)"
  Check "after the countdown (no click in the first 2 s) it clicks the test window exactly 25 times, left button, at the position" ($ok -and -not $early -and $ds.Count -eq 25 -and $us.Count -eq 25 -and @($ds | Sort-Object -Unique).Count -eq 1 -and $ds[0] -eq $here) "early $early; $($ds.Count) presses, $($us.Count) releases at $(($ds | Sort-Object -Unique) -join ' ')"
  $ok = (Tab "Overview") -and (WaitFor { Says "Autoclicker: Finished: 25 clicks" } 10)
  Check "the Overview says it finished: 25 clicks; nothing held" ($ok -and (NoneHeld)) "$(PageText) | held $([AcW]::Held())"

  # ---- shortcuts from another program ----
  $ok = (Tab "Clicker") -and (Radio "Until stopped") -and (ClickerSaved { param($c) $null -eq $c.maxClicks })
  Check "Repeat: until stopped, saved" $ok
  $null = Tab "Overview"; MustBeCovered; $null = Front; $m = Mark
  [AcW]::Tap($F6); $started = WaitFor { (Downs $m).Count -ge 5 } 5
  [AcW]::Tap($F6); $stopped = WaitFor { Says "Autoclicker: Stopped with F6" } 10
  $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "F6 in the test window starts the clicker at once, F6 again stops it; no click after, nothing held" ($started -and $stopped -and (Downs $m).Count -eq $n -and (Ups $m).Count -eq $n -and (NoneHeld)) "$n then $((Downs $m).Count) presses, held $([AcW]::Held())"
  $m = Mark
  [AcW]::Tap($F6); $started = WaitFor { (Downs $m).Count -ge 3 } 5
  [AcW]::Tap($F7); $paused = WaitFor { Says "Autoclicker paused" } 5
  Start-Sleep -Milliseconds 200; $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "F7 pauses it: no click while paused, nothing held" ($started -and $paused -and (Downs $m).Count -eq $n -and (NoneHeld)) "$n then $((Downs $m).Count)"
  [AcW]::Tap($F7); $resumed = WaitFor { (Downs $m).Count -ge $n + 3 } 5
  Check "F7 again resumes it" $resumed
  [AcW]::Tap($F8); $stopped = WaitFor { Says "Autoclicker: Emergency stop (F8)" } 10
  $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "F8, the emergency stop, stops it; no click after, nothing held" ($stopped -and (Downs $m).Count -eq $n -and (NoneHeld)) "$n then $((Downs $m).Count)"
  # A corner of the screens that lies on a monitor (the virtual screen's
  # bounding box); pushed into it as a hand does, for half a second.
  $vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $corner = $null
  foreach ($c in @(@($vs.Left, ($vs.Bottom - 1)), @(($vs.Right - 1), ($vs.Bottom - 1)), @($vs.Left, $vs.Top), @(($vs.Right - 1), $vs.Top))) {
    foreach ($scr in [System.Windows.Forms.Screen]::AllScreens) { if ($scr.Bounds.Contains($c[0], $c[1])) { $corner = $c; break } }
    if ($corner) { break }
  }
  MustBeCovered; [AcW]::SetCursorPos($CLICK[0], $CLICK[1]) | Out-Null; $m = Mark
  [AcW]::Tap($F6); $started = WaitFor { (Downs $m).Count -ge 3 } 5
  for ($k = 0; $k -lt 25; $k++) { [void][AcW]::SetCursorPos($corner[0], $corner[1]); Start-Sleep -Milliseconds 20 }
  $stopped = WaitFor { Says "Stopped: the pointer was pushed into a corner of the screen" } 10
  $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "pushing the pointer into a corner of the screens ($($corner -join ',')) stops it" ($started -and $stopped -and (Downs $m).Count -eq $n -and (NoneHeld)) "$n presses"

  # ---- never sotyware's own window ----
  $ok = (Tab "Clicker") -and (Radio "Where the pointer is") -and (ClickerSaved { param($c) $c.target.mode -eq "cursor" })
  $null = Tab "Overview"
  [void][AcW]::SetCursorPos($D[0] + $D[2] - 60, $D[1] + $D[3] - 60); $m = Mark
  [AcW]::Tap($F6); $skipped = WaitFor { [regex]::IsMatch((Flat), "[1-9][0-9,]* clicks skipped over sotyware's window") } 5
  [AcW]::Tap($F6); $stopped = WaitFor { Says "Autoclicker: Stopped with F6" } 10
  Check "with the pointer on sotyware's window it clicks nothing and counts the clicks as skipped" ($ok -and $skipped -and $stopped -and (Downs $m).Count -eq 0) (PageText)

  # ---- record with F9 and F10 ----
  $null = Tab "Recorder"; MustBeCovered
  $front = Front; Start-Sleep -Milliseconds 300
  [AcW]::Tap($F9)
  $recording = WaitFor { (Btns) -contains "Stop recording" -and -not (Says "Recording starts") } 5
  Check "F9 in the test window starts recording at once" ($front -and $recording) (PageText)
  Start-Sleep -Milliseconds 300
  [AcW]::ClickAt($PA[0], $PA[1], 1); Start-Sleep -Milliseconds 250
  # A click on sotyware's own window while it records (on the page's
  # heading, which does nothing): never recorded.
  # The page's heading (text), not the sidebar's link of the same name.
  $heading = @((Dash).FindAll($S::Descendants, (PCN "Autoclicker")) | Where-Object { $_.Current.ControlType -eq $CT::Text -and -not $_.Current.BoundingRectangle.IsEmpty })[0]
  $own = $null
  if ($heading) { $r = $heading.Current.BoundingRectangle; $own = @([int]($r.X + [Math]::Min($r.Width / 2, 40)), [int]($r.Y + $r.Height / 2)); [AcW]::ClickAt($own[0], $own[1], 1); Start-Sleep -Milliseconds 250 }
  [AcW]::ClickAt($PB[0], $PB[1], 1); Start-Sleep -Milliseconds 250
  [AcW]::Tap($KA); Start-Sleep -Milliseconds 300
  [AcW]::Tap($F10)
  $ready = WaitFor { Says "Recording ready" } 10
  Check "F10 stops it: the recording is ready to save" $ready (PageText)
  $ok = (SetField "Name" "E2E clicks") -and (Press "Save macro") -and (WaitFor { $null -ne (MacroNamed "E2E clicks") } 10)
  $mac = MacroNamed "E2E clicks"
  Check "Save macro stores it in automation.sqlite3" $ok (DocsRaw)
  $acts = @($mac.actions)
  $pressed = @($acts | Where-Object { $_.type -eq "button" } | ForEach-Object { "$($_.state)@$($_.x),$($_.y)" })
  $typed = @($acts | Where-Object { $_.type -eq "key" } | ForEach-Object { "$($_.key) $($_.state)" })
  $kinds = @($acts | ForEach-Object { $_.type } | Sort-Object -Unique)
  $wantPressed = @("down@$($PA[0]),$($PA[1])", "up@$($PA[0]),$($PA[1])", "down@$($PB[0]),$($PB[1])", "up@$($PB[0]),$($PB[1])")
  Check "the macro holds both clicks where they were made and the key A, nothing of F9 or F10" ((($pressed -join "|") -eq ($wantPressed -join "|")) -and (($typed -join "|") -eq "KeyA down|KeyA up") -and @($kinds | Where-Object { $_ -notin @("move", "button", "key") }).Count -eq 0) "$($pressed -join ' ') / $($typed -join ' ') / $($kinds -join ',')"
  $inside = @($acts | Where-Object { $_.type -eq "button" -and $_.x -ge $D[0] -and $_.x -lt $D[0] + $D[2] -and $_.y -ge $D[1] -and $_.y -lt $D[1] + $D[3] })
  Check "the click on sotyware's own window while recording is not in the macro" ($null -ne $own -and $inside.Count -eq 0) "clicked at $($own -join ','); buttons inside the window: $($inside.Count)"

  # ---- play it ----
  $null = Tab "Macros"; MustBeCovered
  [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  $ok = Press "Play"
  $null = WaitFor { (Seq $m) -contains "A" } 15; Start-Sleep -Milliseconds 500
  $sq = Seq $m; $sA = "b@$(Rel $PA)"; $sB = "b@$(Rel $PB)"
  Check "Play replays it into the test window: both clicks where they were made, then the key A" ($ok -and ($sq -join "|") -eq "$sA|$sB|A") ($sq -join " ")
  $ok = (Tab "Overview") -and (WaitFor { Says "Playback: Finished" } 10)
  Check "the Overview says the playback finished; nothing is held" ($ok -and (NoneHeld)) ([AcW]::Held())

  # ---- the right button, double clicks and the speed, timed ----
  $ok = (Tab "Clicker") -and (Radio "Right") -and (Radio "Double") -and (Radio "At a fixed position") -and (SetField "X" "$($CLICK[0])") -and (SetField "Y" "$($CLICK[1])")
  $ok = $ok -and (Radio "Stop after") -and (SetField "Number of clicks" "4") -and (SetField "Speed" "5")
  $saved = ClickerSaved { param($c) $c.button -eq "right" -and $c.clickType -eq "double" -and $c.maxClicks -eq 4 -and [Math]::Abs($c.intervalMs - 200) -lt 0.01 -and $c.target.mode -eq "points" }
  Check "the Clicker tab saves the right button, double clicks, 4 clicks at 5 per second" ($ok -and $saved) ($script:cs | ConvertTo-Json -Compress -Depth 6)
  Check "its hint says: one click every 200 ms" (WaitFor { Says "One click every 200 ms." } 5) (PageText)
  MustBeCovered; $m = Mark
  $ok = Press "Start autoclicker"
  $early = WaitFor { (Downs $m).Count -gt 0 } 2
  $null = WaitFor { (Ups $m).Count -ge 8 } 20; Start-Sleep -Milliseconds 600
  $ds = Downs $m; $us = Ups $m; $right = "2@$(Rel $CLICK)"
  Check "after the countdown, 4 double clicks: 8 presses of the right button at the position, no more" ($ok -and -not $early -and $ds.Count -eq 8 -and $us.Count -eq 8 -and @($ds | Sort-Object -Unique).Count -eq 1 -and $ds[0] -eq $right) "early $early; $($ds.Count) presses, $($us.Count) releases at $(($ds | Sort-Object -Unique) -join ' ')"
  $at = DownTimes $m
  $gaps = @(); $pairs = @()
  for ($i = 0; $i + 2 -lt $at.Count; $i += 2) { $gaps += [Math]::Round($at[$i + 2] - $at[$i], 1) }
  for ($i = 0; $i + 1 -lt $at.Count; $i += 2) { $pairs += [Math]::Round($at[$i + 1] - $at[$i], 1) }
  Check "timed in the test window: a double click every 200 ms (150 to 250), its two clicks 50 ms apart (20 to 120)" ($gaps.Count -eq 3 -and @($gaps | Where-Object { $_ -lt 150 -or $_ -gt 250 }).Count -eq 0 -and $pairs.Count -eq 4 -and @($pairs | Where-Object { $_ -lt 20 -or $_ -gt 120 }).Count -eq 0) "between double clicks $($gaps -join ', ') ms; inside each $($pairs -join ', ') ms"
  $ok = (Tab "Overview") -and (WaitFor { Says "Autoclicker: Finished: 4 clicks" } 10)
  Check "a double click counts as one click: Finished: 4 clicks; nothing held" ($ok -and (NoneHeld)) (PageText)

  # ---- a shortcut changed by pressing it in its field; keys it refuses ----
  $Toggle = "Start or stop the autoclicker"; $StopAll = "Emergency stop"
  $ok = (Tab "Clicker") -and (Radio "Left") -and (Radio "Single") -and (Radio "Where the pointer is") -and (Radio "Until stopped") -and (SetField "Speed" "20")
  $saved = ClickerSaved { param($c) $c.button -eq "left" -and $c.clickType -eq "single" -and $null -eq $c.maxClicks -and $c.target.mode -eq "cursor" -and [Math]::Abs($c.intervalMs - 50) -lt 0.01 }
  Check "back to single left clicks at the pointer, until stopped, 20 per second" ($ok -and $saved) ($script:cs | ConvertTo-Json -Compress -Depth 6)
  $ok = (Tab "Settings") -and (SetShortcut $Toggle { [AcW]::Chord(@($LCtrl, $LAlt), $KK) })
  $saved = WaitFor { (Bind (FirstProfile) "toggle-clicker") -eq "Ctrl+Alt+KeyK" } 10
  $ready = $saved -and (WaitFor { (RowState $Toggle) -eq "Ready" } 10)
  Check "Ctrl+Alt+K pressed in the toggle's field becomes its shortcut: saved and registered" ($ok -and $ready) "$(Bind (FirstProfile) 'toggle-clicker'), $(RowState $Toggle)"
  Check "sotyware gave F6 back and holds Ctrl+Alt+K" ($ready -and (WaitFor { [AcW]::CanRegister(0, $F6) } 5) -and -not [AcW]::CanRegister(($MCtrl -bor $MAlt), $KK))
  $ok = SetShortcut $Toggle { [AcW]::Tap($KA) }
  $letter = $ok -and (WaitFor { Says "alone would stop that key from working everywhere" } 5)
  [AcW]::Tap($F12)
  $refusedF12 = WaitFor { Says "F12 alone is reserved by Windows for debuggers" } 5
  Check "the field refuses a letter alone and F12 alone, with the reason, and keeps listening" ($letter -and $refusedF12 -and (Listening $Toggle) -and (Bind (FirstProfile) "toggle-clicker") -eq "Ctrl+Alt+KeyK") (PageText)
  [AcW]::Tap($F8); Start-Sleep -Seconds 1
  $unchanged = (Listening $Toggle) -and (Bind (FirstProfile) "toggle-clicker") -eq "Ctrl+Alt+KeyK"
  [AcW]::Tap($Esc)
  Check "F8 is the emergency stop's while it is on: it never reaches the field, nothing changes; Escape stops listening" ($unchanged -and (WaitFor { -not (Listening $Toggle) } 3)) "$((Field $Toggle).Current.Name)"
  $null = Tab "Overview"; MustBeCovered; $null = Front
  [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  [AcW]::Tap($F6); Start-Sleep -Seconds 1
  $f6free = (Downs $m).Count -eq 0
  [AcW]::Chord(@($LCtrl, $LAlt), $KK); $started = WaitFor { (Downs $m).Count -ge 3 } 5
  [AcW]::Chord(@($LCtrl, $LAlt), $KK); $stopped = WaitFor { [regex]::IsMatch((Flat), "Autoclicker: Stopped with Ctrl\W*Alt\W*K") } 10
  $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "from the test window F6 now does nothing, and Ctrl+Alt+K starts and stops the clicker" ($f6free -and $started -and $stopped -and (Downs $m).Count -eq $n -and (NoneHeld)) "F6 free $f6free; $n then $((Downs $m).Count)"
  $ok = (Tab "Settings") -and (SetShortcut $Toggle { [AcW]::Tap($F6) })
  $back = (WaitFor { (Bind (FirstProfile) "toggle-clicker") -eq "F6" } 10) -and (WaitFor { (RowState $Toggle) -eq "Ready" } 10)
  Check "F6 pressed in the field makes it the toggle again" ($ok -and $back) "$(RowState $Toggle)"

  # ---- without an emergency stop, an endless run from the page asks first ----
  $ok = SetShortcut $StopAll { [AcW]::Tap($Bksp) }
  $cleared = WaitFor { $null -eq (EmergencyChord) } 10
  $free = $cleared -and (WaitFor { (RowState $StopAll) -eq "Not set" } 5) -and (WaitFor { [AcW]::CanRegister(0, $F8) } 5)
  Check "Backspace in the emergency stop's field clears it: saved, and F8 is free" ($ok -and $free) "$(EmergencyChord), $(RowState $StopAll)"
  $ok = (Tab "Clicker") -and (Radio "At a fixed position") -and (SetField "X" "$($CLICK[0])") -and (SetField "Y" "$($CLICK[1])")
  $saved = ClickerSaved { param($c) $c.target.mode -eq "points" -and $null -eq $c.maxClicks }
  MustBeCovered; $m = Mark
  $asked = $ok -and $saved -and (Press "Start autoclicker") -and (WaitFor { Says "Run until you stop it?" } 5)
  Check "with no emergency stop, Start autoclicker (until stopped) asks first: Run until you stop it?" $asked (PageText)
  $ok = (Press "Cancel") -and (WaitFor { -not (Says "Run until you stop it?") } 5)
  Start-Sleep -Seconds 4
  Check "Cancel starts nothing" ($ok -and (Downs $m).Count -eq 0 -and -not (Says "Clicking")) (PageText)
  $ok = (Press "Start autoclicker") -and (WaitFor { (Btns) -contains "Start anyway" } 5) -and (Press "Start anyway")
  $started = WaitFor { (Downs $m).Count -ge 5 } 10
  Check "Start anyway starts it, after the countdown" ($ok -and $started) "$((Downs $m).Count) presses"
  $ok = Press "Stop"; $stopped = WaitFor { -not (Says "Clicking") } 5
  $n = (Downs $m).Count; Start-Sleep -Milliseconds 600
  Check "the page's Stop stops it at once: no click after, nothing held" ($ok -and $stopped -and (Downs $m).Count -eq $n -and (NoneHeld)) "$n then $((Downs $m).Count)"
  $ok = (Tab "Overview") -and (WaitFor { Says "Autoclicker: Stopped from sotyware" } 10)
  Check "the Overview says it was stopped from sotyware" $ok (PageText)
  $ok = (Tab "Settings") -and (Press "Use F8")
  $back = (WaitFor { (EmergencyChord) -eq "F8" } 10) -and (WaitFor { (RowState $StopAll) -eq "Ready" } 10)
  Check "Use F8 sets the emergency stop again: saved and registered" ($ok -and $back) "$(RowState $StopAll)"

  # ---- playback: repetitions at 2x speed, F7, F8, the profile's macro on Shift+F11 ----
  function EditMacro([string]$name) { (Tab "Overview") -and (Tab "Macros") -and (Press "Edit $name") -and (WaitFor { (Btns) -contains "Back to macros" -and -not (Says "Loading$Ellipsis") } 10) }
  function SaveMacro { (Press "Save") -and (WaitFor { Says "Saved." } 10) }
  $ok = (EditMacro "E2E clicks") -and (Choose "Repeat" "Several times") -and (SetField "Number of repetitions" "3") -and (ToggleNamed "2$Times" $CT::Button) -and (SaveMacro)
  $mac = MacroNamed "E2E clicks"
  Check "the editor saves 3 repetitions at 2x speed" ($ok -and $mac.playback.repeat -eq 3 -and $mac.playback.speed -eq 2) ($mac.playback | ConvertTo-Json -Compress)
  $acts = @($mac.actions)
  $dn = @(for ($i = 0; $i -lt $acts.Count; $i++) { if ($acts[$i].type -eq "button" -and $acts[$i].state -eq "down") { $i } })
  $recorded = if ($dn.Count -eq 2) { ($acts[($dn[0] + 1)..$dn[1]] | Measure-Object -Property delayMs -Sum).Sum } else { $null }
  MustBeCovered; [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  $ok = Press "Play"
  $null = WaitFor { @(Seq $m | Where-Object { $_ -eq "A" }).Count -ge 3 } 40; Start-Sleep -Milliseconds 500
  $sq = Seq $m
  Check "Play plays it 3 times: both clicks, then the key A, each time" ($ok -and ($sq -join "|") -eq (@($sA, $sB, "A", $sA, $sB, "A", $sA, $sB, "A") -join "|")) ($sq -join " ")
  $at = DownTimes $m; $played = @(); for ($i = 0; $i + 1 -lt $at.Count; $i += 2) { $played += [Math]::Round($at[$i + 1] - $at[$i], 1) }
  $want = if ($recorded) { $recorded / 2 } else { $null }
  Check "at 2x the time between the two clicks is halved" ($null -ne $want -and $played.Count -eq 3 -and @($played | Where-Object { [Math]::Abs($_ - $want) -gt [Math]::Max(50, $want * 0.3) }).Count -eq 0) "recorded $recorded ms, played $($played -join ', ') ms, expected about $want"
  $ok = (Choose "Repeat" "Until stopped") -and (SaveMacro) -and $null -eq (MacroNamed "E2E clicks").playback.repeat
  MustBeCovered; $m = Mark
  $ok = $ok -and (Press "Play")
  $started = WaitFor { (Seq $m).Count -ge 4 } 15
  [AcW]::Tap($F7); $paused = WaitFor { Says "Paused:" } 5
  Start-Sleep -Milliseconds 200; $n = (Ev $m).Count; Start-Sleep -Milliseconds 800
  Check "an endless playback paused with F7: nothing sent while paused, nothing held" ($ok -and $started -and $paused -and (Ev $m).Count -eq $n -and (NoneHeld)) "$n then $((Ev $m).Count) events, held $([AcW]::Held())"
  [AcW]::Tap($F7); $resumed = WaitFor { (Ev $m).Count -gt $n + 2 } 10
  [AcW]::Tap($F8); $stopped = (Tab "Overview") -and (WaitFor { Says "Playback: Emergency stop (F8)" } 10)
  $n = (Ev $m).Count; Start-Sleep -Milliseconds 600
  Check "F7 again resumes it; F8 stops it: nothing sent after, nothing held" ($resumed -and $stopped -and (Ev $m).Count -eq $n -and (NoneHeld)) "$n then $((Ev $m).Count)"
  $ok = (EditMacro "E2E clicks") -and (Choose "Repeat" "Once") -and (SaveMacro) -and (MacroNamed "E2E clicks").playback.repeat -eq 1
  $mid = (MacroNamed "E2E clicks").id
  $ok = $ok -and (Tab "Profiles") -and (Choose "Macro" "E2E clicks") -and (WaitFor { (FirstProfile).macroId -eq $mid } 10)
  Check "the Profiles tab makes it the profile's macro" $ok "$((FirstProfile).macroId)"
  $null = Tab "Overview"; MustBeCovered; $null = Front
  [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  [AcW]::Chord(@($LShift), $F11)
  $soon = WaitFor { (Seq $m).Count -gt 0 } 2
  $null = WaitFor { (Seq $m) -contains "A" } 15; Start-Sleep -Milliseconds 500
  Check "Shift+F11 in the test window plays the profile's macro at once (no countdown)" ($soon -and ((Seq $m) -join "|") -eq "$sA|$sB|A") ((Seq $m) -join " ")

  # ---- the editor ----
  $ok = EditMacro "E2E clicks"
  $before = @((MacroNamed "E2E clicks").actions); $n0 = $before.Count
  $ok = $ok -and (SetField "Action 1: delay before it in ms" "400") -and (WaitFor { Says "Unsaved changes" } 5)
  $ok = $ok -and (Choose "Kind of action to add" "Wait") -and (Press "Add at the end") -and (WaitFor { (Btns) -contains "Delete action $($n0 + 1)" } 5)
  $ok = $ok -and (Choose "Kind of action to add" "Click") -and (Press "Add after selection") -and (WaitFor { (Btns) -contains "Delete action $($n0 + 2)" } 5)
  # The click (action n0+2, after the wait) moves above the wait, is
  # duplicated, and the copy is deleted again.
  $ok = $ok -and (Press "Move action $($n0 + 2) up") -and (Press "Duplicate action $($n0 + 1)") -and (WaitFor { (Btns) -contains "Delete action $($n0 + 3)" } 5)
  $ok = $ok -and (Press "Delete action $($n0 + 2)") -and (WaitFor { -not ((Btns) -contains "Delete action $($n0 + 3)") } 5)
  Check "the editor sets a delay, adds a wait and a click, moves one up, duplicates and deletes: unsaved" ($ok -and (Says "Unsaved changes")) ((Btns | Select-Object -Last 30) -join ', ')
  $m = Mark
  $ok = (Press "Preview") -and (WaitFor { (Btns) -contains "Stop preview" } 5)
  $shown = $ok -and (WaitFor { Says "Preview (no input is sent)" } 5)
  $ended = $ok -and (WaitFor { -not ((Btns) -contains "Stop preview") } 30)
  Check "Preview runs along the timeline and sends nothing" ($shown -and $ended -and (Ev $m).Count -eq 0) "$((Ev $m).Count) events"
  $ok = (Press "Back to macros") -and (WaitFor { Says "Leave without saving?" } 5)
  $ok = $ok -and (Press "Keep editing") -and (WaitFor { -not (Says "Leave without saving?") } 5)
  Check "leaving with unsaved changes asks first; Keep editing keeps them" ($ok -and (Says "Unsaved changes"))
  $ok = SaveMacro
  $after = @((MacroNamed "E2E clicks").actions)
  $same = ($after.Count -eq $n0 + 2) -and $n0 -gt 0 -and $after[0].delayMs -eq 400 -and
    (($after[1..($n0 - 1)] | ConvertTo-Json -Compress -Depth 6) -eq ($before[1..($n0 - 1)] | ConvertTo-Json -Compress -Depth 6)) -and
    $after[$n0].type -eq "click" -and $after[$n0].x -eq $PB[0] -and $after[$n0].y -eq $PB[1] -and $after[$n0 + 1].type -eq "wait"
  Check "Save stores exactly that: 400 ms before action 1, then a click where the last one was, then a wait" ($ok -and $same) ($after | Select-Object -Last 3 | ConvertTo-Json -Compress -Depth 6)
  $ok = (SetField "Action 1: delay before it in ms" "999") -and (WaitFor { Says "Unsaved changes" } 5)
  $ok = $ok -and (Press "Revert") -and (WaitFor { -not (Says "Unsaved changes") } 5)
  Check "Revert drops a change that was not saved" ($ok -and @((MacroNamed "E2E clicks").actions)[0].delayMs -eq 400)
  MustBeCovered; [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  $ok = Press "Play"
  $null = WaitFor { (Seq $m).Count -ge 4 } 20; Start-Sleep -Seconds 1
  Check "Play plays the saved changes: both clicks, the key A, then the added click" ($ok -and ((Seq $m) -join "|") -eq "$sA|$sB|A|$sB") ((Seq $m) -join " ")
  $ok = (Tab "Overview") -and (WaitFor { Says "Playback: Finished" } 10)
  Check "the playback finished; nothing held" ($ok -and (NoneHeld)) ([AcW]::Held())
  $Edited = DocsRaw

  # ---- a restart keeps everything and starts nothing ----
  $kept = DocsRaw
  $code = QuitApp
  Check "the tray's Quit ends sotyware: exit code 0" ($code -eq 0) "exit $code"
  $ok = (StartApp) -and (Link "Autoclicker") -and (WaitFor { -not (Says "The autoclicker is off.") -and (Registered) -ge 6 } 20)
  Check "started again, the autoclicker is still on, its six shortcuts registered" $ok (PageText)
  $f = Free
  Check "sotyware holds them again: this script can register none of them" ($ok -and $f.Count -eq 0) "free: $($f -join ', ')"
  $m = Mark; Start-Sleep -Seconds 3
  Check "nothing starts by itself" ((Ev $m).Count -eq 0 -and (Idle)) ((Btns) -join ', ')
  Check "its settings, profiles and macros are exactly as they were" ((DocsRaw) -eq $kept)

  # ---- a second profile with its own shortcut; switched; started with ----
  $first = FirstProfile
  $ok = (Tab "Profiles") -and (SetField "New profile" "Second") -and (Press "Create profile") -and (WaitFor { $null -ne (ProfileNamed "Second") } 10)
  $second = ProfileNamed "Second"
  Check "Create profile adds a second profile with the active one's settings and shortcuts" ($ok -and ($second.hotkeys | ConvertTo-Json -Compress) -eq ($first.hotkeys | ConvertTo-Json -Compress) -and ($second.clicker | ConvertTo-Json -Compress -Depth 6) -eq ($first.clicker | ConvertTo-Json -Compress -Depth 6)) (DocsRaw)
  $ok = (Press "Use this profile") -and (WaitFor { Says "$($Lq)Second$($Rq) is the active profile" } 10)
  $ok = $ok -and (Tab "Settings") -and (SetShortcut $Toggle { [AcW]::Chord(@($LCtrl, $LAlt), $KJ) })
  $ready = (WaitFor { (Bind (ProfileNamed "Second") "toggle-clicker") -eq "Ctrl+Alt+KeyJ" } 10) -and (WaitFor { (RowState $Toggle) -eq "Ready" } 10)
  Check "in the second profile the toggle becomes Ctrl+Alt+J; the first keeps F6" ($ok -and $ready -and (Bind (FirstProfile) "toggle-clicker") -eq "F6") "$(RowState $Toggle)"
  Check "with the second profile active, F6 is free and Ctrl+Alt+J is held" ($ready -and (WaitFor { [AcW]::CanRegister(0, $F6) } 5) -and -not [AcW]::CanRegister(($MCtrl -bor $MAlt), $KJ))
  $ok = (Tab "Profiles") -and (Press "Start with it") -and (WaitFor { (Docs).settings.defaultProfile -eq $second.id } 10)
  Check "Start with it makes the second profile the one sotyware starts with" $ok
  $ok = (Press "Use this profile") -and (WaitFor { (Says "is the active profile") -and -not (Says "$($Lq)Second$($Rq) is the active profile") } 10)
  Check "switching back to the first profile changes the shortcuts at once: F6 held, Ctrl+Alt+J free" ($ok -and (WaitFor { [AcW]::CanRegister(($MCtrl -bor $MAlt), $KJ) } 5) -and -not [AcW]::CanRegister(0, $F6))
  $code = QuitApp
  $ok = $code -eq 0 -and (StartApp) -and (Link "Autoclicker") -and (WaitFor { (Registered) -ge 6 } 20)
  Check "after a restart sotyware starts with the second profile: Ctrl+Alt+J held, F6 free" ($ok -and -not [AcW]::CanRegister(($MCtrl -bor $MAlt), $KJ) -and [AcW]::CanRegister(0, $F6)) "exit $code"
  $ok = (Tab "Profiles") -and (Press "Use this profile") -and (WaitFor { Says "is the active profile" } 10) -and (Press "Start with it") -and (WaitFor { (Docs).settings.defaultProfile -eq $first.id } 10)
  Check "back to the first profile, which sotyware starts with again" ($ok -and -not [AcW]::CanRegister(0, $F6))

  # ---- a macro's own shortcut; export, delete, import ----
  $ok = (EditMacro "E2E clicks") -and (SetShortcut "Shortcut that plays E2E clicks" { [AcW]::Chord(@($LCtrl, $LAlt), $KM) })
  $ok = $ok -and (WaitFor { Says "Unsaved changes" } 5) -and (SaveMacro)
  $sc = (MacroNamed "E2E clicks").shortcut
  Check "the macro's own shortcut is set in its editor: Ctrl+Alt+M, saved" ($ok -and $sc.chord -eq "Ctrl+Alt+KeyM" -and $sc.enabled -eq $true) ($sc | ConvertTo-Json -Compress)
  $ok = (Tab "Overview") -and (WaitFor { (Registered) -ge 7 } 10)
  Check "it is registered: this script cannot register Ctrl+Alt+M" ($ok -and -not [AcW]::CanRegister(($MCtrl -bor $MAlt), $KM)) (PageText)
  MustBeCovered; $null = Front; [void][AcW]::SetCursorPos($CLICK[0], $CLICK[1]); $m = Mark
  [AcW]::Chord(@($LCtrl, $LAlt), $KM)
  $null = WaitFor { (Seq $m).Count -ge 4 } 15; Start-Sleep -Seconds 1
  Check "Ctrl+Alt+M in the test window plays the macro" (((Seq $m) -join "|") -eq "$sA|$sB|A|$sB") ((Seq $m) -join " ")
  $ok = (Tab "Macros") -and (Press "Export E2E clicks") -and (DialogPath "Export macros and profiles" 1001 $Exported)
  $done = $ok -and (WaitFor { (Test-Path -LiteralPath $Exported) -and (Says "Exported the macro to") } 20)
  $data = if ($done) { [System.IO.File]::ReadAllText($Exported) | ConvertFrom-Json } else { $null }
  Check "Export writes it through the native save dialog, with its shortcut" ($done -and @($data.macros).Count -eq 1 -and @($data.macros)[0].name -eq "E2E clicks" -and @($data.macros)[0].shortcut.chord -eq "Ctrl+Alt+KeyM") "$Exported"
  $ok = (Press "Delete E2E clicks") -and (WaitFor { (Btns) -contains "Delete macro" } 5) -and (Press "Delete macro")
  $gone = $ok -and (WaitFor { $null -eq (MacroNamed "E2E clicks") } 10)
  Check "Delete asks, removes it and gives Ctrl+Alt+M back" ($gone -and (WaitFor { [AcW]::CanRegister(($MCtrl -bor $MAlt), $KM) } 5))
  $ok = (Press "Import$Ellipsis") -and (DialogPath "Import macros and profiles" 1148 $Exported)
  $report = $ok -and (WaitFor { Says "Imported 1 macro" } 20)
  $back = MacroNamed "E2E clicks"
  $backActs = @($back.actions) | ConvertTo-Json -Compress -Depth 6
  Check "Import brings it back through the native open dialog, its shortcut switched off" ($report -and $back.shortcut.chord -eq "Ctrl+Alt+KeyM" -and $back.shortcut.enabled -eq $false -and $backActs -eq ($after | ConvertTo-Json -Compress -Depth 6) -and (Says "Imported shortcuts are switched off")) (PageText)
  Check "the imported shortcut is not registered: this script can register Ctrl+Alt+M" ([AcW]::CanRegister(($MCtrl -bor $MAlt), $KM))

  # ---- keys a macro holds are released: by F8, by the page's Stop, by quitting ----
  $ok = (Tab "Overview") -and (Tab "Macros") -and (Press "New macro") -and (SetField "Name" "E2E hold") -and (Press "Create and edit") -and (WaitFor { (Btns) -contains "Back to macros" -and (Says "No actions yet") } 10)
  $ok = $ok -and (Choose "Kind of action to add" "Key") -and (Press "Add at the end") -and (WaitFor { $null -ne (Prefixed "Action 1: key" $CT::ComboBox) } 5) -and (Choose "Action 1: key" "Left Shift")
  $ok = $ok -and (Choose "Kind of action to add" "Wait") -and (Press "Add after selection") -and (WaitFor { (Btns) -contains "Delete action 2" } 5) -and (SetField "Action 2: delay before it in ms" "30000") -and (SaveMacro)
  $hold = MacroNamed "E2E hold"
  $holdActs = @($hold.actions | ForEach-Object { "$($_.type) $($_.key) $($_.state) $($_.delayMs)" })
  Check "a macro written in the editor: Left Shift down, then a 30 s wait" ($ok -and ($holdActs -join "|") -eq "key ShiftLeft down 100|wait   30000") ($holdActs -join " / ")
  # Playback sends no keys while sotyware's own window is active: the test
  # window comes to the front first. True once Shift is down.
  function HoldShift { MustBeCovered; $null = Front; (Press "Play") -and (WaitFor { ShiftHeld } 10) }
  $ok = HoldShift
  [AcW]::Tap($F8)
  $released = WaitFor { -not (ShiftHeld) } 3
  $stopped = WaitFor { -not ((Btns) -contains "Stop") } 5
  Check "F8 stops a playback that holds Shift down, and Shift is released" ($ok -and $released -and $stopped) "Shift down at first: $ok; held now $([AcW]::Held())"
  if (ShiftHeld) { $null = Press "Stop"; $null = WaitFor { -not (ShiftHeld) } 5 }
  $ok = HoldShift
  $ok2 = Press "Stop"
  $released = WaitFor { -not (ShiftHeld) } 3
  Check "the page's Stop stops it, and Shift is released" ($ok -and $ok2 -and $released) ([AcW]::Held())
  $ok = HoldShift
  $code = QuitApp
  Check "quitting sotyware (tray Quit) while a macro holds Shift releases it (exit code 0)" ($ok -and $code -eq 0 -and -not (ShiftHeld)) "exit $code, held $([AcW]::Held())"
  $ok = (StartApp) -and (Link "Autoclicker") -and (WaitFor { (Registered) -ge 6 } 20)
  Check "sotyware starts again afterwards" $ok (PageText)

  # ---- at 100 clicks per second the page stays responsive ----
  $ok = (Tab "Clicker") -and (Radio "At a fixed position") -and (SetField "X" "$($CLICK[0])") -and (SetField "Y" "$($CLICK[1])") -and (Radio "Until stopped") -and (SetField "Speed" "100")
  $saved = ClickerSaved { param($c) $c.target.mode -eq "points" -and $null -eq $c.maxClicks -and [Math]::Abs($c.intervalMs - 10) -lt 0.01 }
  Check "100 clicks per second at a fixed position, until stopped, saved" ($ok -and $saved)
  $null = Tab "Overview"; MustBeCovered; $null = Front; $m = Mark
  [AcW]::Tap($F6); $started = WaitFor { (Downs $m).Count -ge 20 } 5
  Start-Sleep -Milliseconds 500
  $n1 = (Downs $m).Count; $cpu1 = (Get-Process -Id $script:app.Id).TotalProcessorTime.TotalSeconds; $w1 = Get-Date
  $slowest = 0.0
  foreach ($tm in @(@("Macros", "Recorded or written sequences"), @("Profiles", "Each profile keeps"), @("Settings", "They work in every program"), @("Overview", "Profile $Lq"))) {
    $t0 = Get-Date
    $shown = (Tab $tm[0]) -and (WaitFor { Says $tm[1] } 10)
    $slowest = [Math]::Max($slowest, $(if ($shown) { ((Get-Date) - $t0).TotalSeconds } else { 99.0 }))
  }
  $left = 2.0 - ((Get-Date) - $w1).TotalSeconds; if ($left -gt 0) { Start-Sleep -Milliseconds ([int]($left * 1000)) }
  $n2 = (Downs $m).Count; $cpu2 = (Get-Process -Id $script:app.Id).TotalProcessorTime.TotalSeconds; $w2 = Get-Date
  $span = ($w2 - $w1).TotalSeconds; $cpu = ($cpu2 - $cpu1) / $span
  # Timed in the test window, as the clicks reached it.
  $stamps = @(DownTimes $m)[$n1..($n2 - 1)]
  $rate = if ($stamps.Count -gt 1 -and $stamps[-1] -gt $stamps[0]) { ($stamps.Count - 1) * 1000.0 / ($stamps[-1] - $stamps[0]) } else { 0 }
  Check "it clicks about 100 times per second (85 to 115, timed in the test window) while the page is used" ($started -and $rate -ge 85 -and $rate -le 115) ("{0:N1} per second over {1} clicks" -f $rate, $stamps.Count)
  # Includes this script's own reading of the page (UI Automation walks
  # WebView2's whole tree), which is far slower than AT-SPI.
  Check "switching tabs meanwhile takes under 5 s each (this script's reading included)" ($slowest -lt 5.0) ("slowest {0:N2} s" -f $slowest)
  Check "sotyware's own process uses under half a CPU core meanwhile" ($cpu -lt 0.5) ("{0:N0} % of one core" -f ($cpu * 100))
  $ok = Press "Stop"; $nStop = (Downs $m).Count
  Start-Sleep -Milliseconds 500; $nAfter = (Downs $m).Count
  $stopped = WaitFor { Idle } 5; Start-Sleep -Milliseconds 300
  Check "the page's Stop answers at once: at most 20 more clicks (0.2 s), none after, nothing held" ($ok -and $stopped -and ($nAfter - $nStop) -le 20 -and (Downs $m).Count -eq $nAfter -and (NoneHeld)) "$($nAfter - $nStop) after Stop; held $([AcW]::Held())"

  # ---- turning it off gives the shortcuts back ----
  $ok = (Link "Settings") -and (WaitFor { $null -ne (Named "Features" $CT::TabItem) } 5) -and (Tab "Features") -and (WaitFor { $null -ne (Named "Autoclicker" $CT::CheckBox) } 5)
  Check "Settings > Features lists the autoclicker as running" ($ok -and (FeatureState "Autoclicker") -eq "Running") "$(FeatureState 'Autoclicker')"
  $ok = $ok -and (ToggleNamed "Autoclicker" $CT::CheckBox) -and (WaitFor { (FeatureState "Autoclicker") -eq "Disabled" } 10)
  Check "Settings > Features turns the autoclicker off" $ok "$(FeatureState 'Autoclicker')"
  $f = Free
  Check "every shortcut is released: this script can register all six again" ($f.Count -eq 6) "free: $($f -join ', ')"
  $null = Front; $m = Mark
  [AcW]::Tap($F6); Start-Sleep -Seconds 1
  Check "F6 does nothing once it is off" ((Downs $m).Count -eq 0)
  $ok = (Link "Autoclicker") -and (WaitFor { Says "The autoclicker is off." } 10)
  Check "the page says it is off again" $ok

  # ---- while it is off, F8 reaches the field, and is refused for another action ----
  $ok = (Tab "Settings") -and (SetShortcut $Toggle { [AcW]::Tap($F8) })
  $refused = $ok -and (WaitFor { Says 'F8 is already the shortcut of "Emergency stop"' } 10)
  Check "while it is off, F8 pressed in the toggle's field is refused: it is the emergency stop's" ($refused -and (Bind (FirstProfile) "toggle-clicker") -eq "F6") (PageText)
  Check "process still running" ((Procs).Count -eq 1)
} catch {
  Check "the test ran to its end" $false "$_"
} finally {
  [AcW]::ReleaseAll()
  [AcTarget]::Close()
  [void][AcW]::SetCursorPos($PointerBefore[0], $PointerBefore[1])
  Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
  $null = WaitFor { (Procs).Count -eq 0 } 10
  if (Test-Path -LiteralPath $Exported) { Remove-Item -LiteralPath $Exported }
  Remove-TestCredentials $Prof
  if (Test-Path -LiteralPath $Prof.Root) { Remove-TestProfile $Prof }
  Check "cleanup: this run's test profile removed" (-not (Test-Path -LiteralPath $Prof.Root))
  Check "the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|"))
  $realAfter = Get-RealNormalProfileFingerprint $Prof.Parent
  Check "the user's real app folder, read from outside this process tree, is unchanged" ($null -ne $realAfter -and (@($realAfter) -join "|") -eq (@($RealBefore) -join "|"))
  Check "the pointer is back where it was" ((([AcW]::Cursor()) -join ",") -eq ($PointerBefore -join ","))
  Check "no key or button is left down" (NoneHeld) ([AcW]::Held())
}
"$($script:pass) passed, $($script:fail) failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
