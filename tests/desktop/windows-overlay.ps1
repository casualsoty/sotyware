# The gaming notes overlay (ADR-052) on Windows: opt-in, not part of CI.
# Drives a test-profile build (ADR-027) through UI Automation, as the Linux
# phase does (tests/desktop/cloud_ui_overlay.py), Windows' way. Two ways to
# run it:
#
# - Unattended (the default): the script creates a private desktop
#   (CreateDesktop, as windows-isolated.ps1 does), runs itself on it and
#   prints what that copy reported. Nothing appears on your screen and the
#   mouse and keyboard are not used, so you can keep working. Checked
#   there: off by default; Settings > Notes overlay > Turn on shows the
#   overlay at the top right of the work area, 24 px in, 360 x 440 at the
#   monitor's scaling, topmost, without a frame, a tool window (no taskbar
#   button), and without being activated (the active window of sotyware's
#   window thread, read with GetGUIThreadInfo, stays the dashboard; a
#   private desktop has no foreground window, but activation still
#   happens there, so a show that activates is seen); the five
#   suggested shortcuts registered (this script cannot register them); a
#   note created in the overlay, its title and checklist set, autosaved to
#   notes.sqlite3, a checkbox ticked; passive (WS_EX_TRANSPARENT and
#   WS_EX_NOACTIVATE) from the overlay's button and back with Make
#   interactive; collapse and expand; a new corner in the settings moving
#   the open overlay; its close button and WM_CLOSE (Alt+F4) hiding it, the
#   window kept; a move onto another monitor (when there is one) at that
#   monitor's scaling; a restart that keeps the notes and the place; and
#   turning it off, which destroys the window and releases the shortcuts.
#   What needs the input desktop is not checked there: focus, real keys
#   and clicks, the shortcuts' effect, pixels on the screen, the tray.
#
# - -ConfirmRealInput pointer-and-keys: on the REAL desktop, with real
#   input (SendInput: keys by virtual key, text as Unicode characters, so
#   any keyboard layout works; the pointer and its left button). IT MOVES
#   THE POINTER, CLICKS AND PRESSES KEYS (Ctrl, Shift, F1 to F5, Enter,
#   Escape, Home, Down, A, and typed text), and quits sotyware with the
#   tray icon's Quit (a real right click in the "Show Hidden Icons"
#   flyout). Run it only on an idle, unlocked desktop, and do not use the
#   mouse or keyboard while it runs (about half a minute). A plain window of
#   this script plays the game, partly under the overlay, and records the
#   clicks and keys it gets; it is made topmost only for the clicks that
#   bring it to the front (a window of a program in the background opens
#   behind the one in front). On top of the unattended checks: Turn on does
#   not take the keyboard from the dashboard; a click in the title gives
#   the overlay the keyboard, and Enter continues a checklist typed as real
#   keys; from the game, Ctrl+Shift+F1 hides and shows the overlay where it
#   was while the game keeps the keyboard; interactive, a click on the
#   overlay stays there; Ctrl+Shift+F2 makes it passive, the same click
#   reaches the game, the overlay is still drawn on the screen, and
#   Ctrl+Shift+F2 again makes it interactive with the keyboard; the game
#   never gets F1 to F5 or an Alt; Ctrl+Shift+F3 creates a note with the
#   cursor in its title; Ctrl+Shift+F5 and Ctrl+Shift+F4 switch notes and
#   leave the keyboard to the game; and sotyware's own process stays idle
#   while the overlay is shown.
#
# usage (PowerShell, repository root, after npm run build:test-profile):
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-overlay.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\windows-overlay.ps1 -ConfirmRealInput pointer-and-keys
# Needs node on PATH (notes.sqlite3 is read with Node's built-in node:sqlite).
# Not in an elevated session when unattended (WebView2 cannot attach to a
# window on a private desktop then; see windows-isolated.ps1).
# exit: 0 every check passed; 1 a check failed; 2 nothing run (binary, node
# or a wrong confirmation); 3 no private desktop, or (real input) not the
# interactive desktop; 4 not run: elevated session (unattended); 124 timeout.
#
# ASCII only: Windows PowerShell 5.1 reads a file without a BOM as ANSI.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\..\src-tauri\target\test-profile\release\sotyware.exe"),
  [string]$ProfileParent = (Join-Path $PSScriptRoot "..\..\src-tauri\target\desktop-test-profiles"),
  [string]$ConfirmRealInput = "",
  [string]$OnDesktop = "",
  [string]$Log = "",
  [int]$Timeout = 480
)
$Real = $ConfirmRealInput -ceq "pointer-and-keys"
if ($ConfirmRealInput -ne "" -and -not $Real) { "ABORT: -ConfirmRealInput takes exactly pointer-and-keys; nothing was run"; exit 2 }
if (-not (Test-Path $Exe)) { "binary not found: $Exe (run: npm run build:test-profile)"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
if (-not (Test-TestProfileBuild $Exe)) { "ABORT: $Exe is not a test-profile build (ADR-027); run: npm run build:test-profile"; exit 2 }
if (-not (Get-Command node -EA SilentlyContinue)) { "ABORT: node is not on PATH (notes.sqlite3 is read with node:sqlite)"; exit 2 }

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Collections.Generic; using System.Diagnostics;
using System.Runtime.InteropServices; using System.Text; using System.Threading;
public static class OvW {
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cb; public RECT mon, work; public uint flags; }

  // ---- a private desktop and a process on it (as in windows-isolated.ps1) ----
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
  public static IntPtr Start(string app, string cmd, string desktop) {
    var si = new SI(); si.cb = Marshal.SizeOf(typeof(SI)); si.desktop = desktop; PI pi;
    if (!CreateProcessW(app, new StringBuilder(cmd, 8192), IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref si, out pi))
      throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    return pi.process;
  }
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

  // ---- windows ----
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  public static string ClassOf(IntPtr h) { var sb = new StringBuilder(256); GetClassName(h, sb, 256); return sb.ToString(); }
  public static string TitleOf(IntPtr h) { var sb = new StringBuilder(512); GetWindowText(h, sb, 512); return sb.ToString(); }
  // All top-level windows (shown or not) of a class owned by the pids, in
  // z-order from the top. EnumWindows lists only this thread's desktop.
  public static IntPtr[] Find(string cls, uint[] pids) {
    var res = new List<IntPtr>();
    EnumWindows(delegate(IntPtr h, IntPtr l) { uint pid; GetWindowThreadProcessId(h, out pid);
      if (ClassOf(h) == cls && Array.IndexOf(pids, pid) >= 0) res.Add(h); return true; }, IntPtr.Zero);
    return res.ToArray();
  }
  // Whether a is above b in the z-order of top-level windows.
  public static bool Above(IntPtr a, IntPtr b) {
    int ia = -1, ib = -1, i = 0;
    EnumWindows(delegate(IntPtr h, IntPtr l) { if (h == a) ia = i; if (h == b) ib = i; i++; return true; }, IntPtr.Zero);
    return ia >= 0 && ib >= 0 && ia < ib;
  }
  public static IntPtr Child(IntPtr parent, string cls) {
    IntPtr found = IntPtr.Zero;
    EnumChildWindows(parent, delegate(IntPtr h, IntPtr l) { if (ClassOf(h) == cls) { found = h; return false; } return true; }, IntPtr.Zero);
    return found;
  }
  public static int[] Rect(IntPtr h) { RECT r; if (!GetWindowRect(h, out r)) return null; return new int[] { r.L, r.T, r.R - r.L, r.B - r.T }; }
  [StructLayout(LayoutKind.Sequential)] public struct GUITHREADINFO { public int cbSize; public uint flags; public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret; public RECT rcCaret; }
  [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint tid, ref GUITHREADINFO info);
  // The active window of the thread that owns window h: which of its
  // windows that thread activated last. Activation happens there even on a
  // desktop without input (no foreground window), so it tells whether
  // showing a window activated it.
  public static IntPtr ActiveInThreadOf(IntPtr h) {
    uint pid; uint tid = GetWindowThreadProcessId(h, out pid);
    var gi = new GUITHREADINFO(); gi.cbSize = Marshal.SizeOf(typeof(GUITHREADINFO));
    return GetGUIThreadInfo(tid, ref gi) ? gi.hwndActive : IntPtr.Zero;
  }
  public static int[] ClientSize(IntPtr h) { RECT r; if (!GetClientRect(h, out r)) return null; return new int[] { r.R - r.L, r.B - r.T }; }
  // The top-level window under a screen point (WindowFromPoint skips
  // windows that let clicks through, as a click does).
  public static IntPtr RootAt(int x, int y) { var p = new POINT(); p.X = x; p.Y = y; return GetAncestor(WindowFromPoint(p), 2); }

  // ---- monitors: [work x, y, width, height, dpi, primary] ----
  [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
  [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(POINT p, uint f);
  [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO i);
  delegate bool MonProc(IntPtr m, IntPtr dc, ref RECT r, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, MonProc f, IntPtr l);
  [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr m, int type, out uint x, out uint y);
  static int[] Describe(IntPtr m) {
    var mi = new MONITORINFO(); mi.cb = Marshal.SizeOf(typeof(MONITORINFO));
    if (!GetMonitorInfo(m, ref mi)) return null;
    uint dx, dy; if (GetDpiForMonitor(m, 0, out dx, out dy) != 0) dx = 96;
    return new int[] { mi.work.L, mi.work.T, mi.work.R - mi.work.L, mi.work.B - mi.work.T, (int)dx, (int)(mi.flags & 1) };
  }
  public static int[][] Monitors() {
    var l = new List<int[]>();
    EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, delegate(IntPtr m, IntPtr dc, ref RECT r, IntPtr p) {
      var d = Describe(m); if (d != null) l.Add(d); return true; }, IntPtr.Zero);
    return l.ToArray();
  }
  public static int[] MonitorOf(IntPtr h) { return Describe(MonitorFromWindow(h, 2)); }
  public static int[] MonitorAt(int x, int y) { var p = new POINT(); p.X = x; p.Y = y; return Describe(MonitorFromPoint(p, 2)); }
  [DllImport("user32.dll")] static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  // The pointer, or null where it cannot be read (not the input desktop).
  public static int[] Cursor() { POINT p; if (!GetCursorPos(out p)) return null; return new int[] { p.X, p.Y }; }

  // ---- shortcuts ----
  [DllImport("user32.dll")] static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
  [DllImport("user32.dll")] static extern bool UnregisterHotKey(IntPtr h, int id);
  // Whether this script can register the shortcut itself: not while
  // another program (sotyware) holds it. Probed only while sotyware is not
  // about to register it, so it never takes one away from it.
  public static bool CanRegister(uint mods, uint vk) {
    if (!RegisterHotKey(IntPtr.Zero, 0xBEE1, mods | 0x4000u, vk)) return false;
    UnregisterHotKey(IntPtr.Zero, 0xBEE1); return true;
  }

  // ---- real input, as from the user's keyboard and mouse ----
  [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
  [StructLayout(LayoutKind.Explicit)] struct INPUTUNION { [FieldOffset(0)] public MOUSEINPUT mi; [FieldOffset(0)] public KEYBDINPUT ki; }
  [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public INPUTUNION u; }
  [DllImport("user32.dll", SetLastError=true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vk);
  static void Send(INPUT i) { var a = new INPUT[] { i }; SendInput(1, a, Marshal.SizeOf(typeof(INPUT))); }
  // A key by its virtual key, which the layout maps to its key: Ctrl+A is
  // select-all on AZERTY too. The arrows and Home are extended keys.
  public static void Vk(int vk, bool up) {
    var i = new INPUT(); i.type = 1; i.u.ki.wVk = (ushort)vk; i.u.ki.wScan = (ushort)MapVirtualKey((uint)vk, 0);
    bool ext = vk == 0x24 || vk == 0x23 || (vk >= 0x25 && vk <= 0x28);
    i.u.ki.dwFlags = (up ? 0x0002u : 0u) | (ext ? 0x0001u : 0u); Send(i);
  }
  public static void Tap(int vk) { Vk(vk, false); Thread.Sleep(30); Vk(vk, true); Thread.Sleep(30); }
  public static void Chord(int[] mods, int vk) {
    foreach (int m in mods) { Vk(m, false); Thread.Sleep(25); }
    Tap(vk);
    for (int k = mods.Length - 1; k >= 0; k--) { Vk(mods[k], true); Thread.Sleep(25); }
  }
  // Text as characters (KEYEVENTF_UNICODE), whatever the layout.
  public static void Type(string text) {
    foreach (char c in text) {
      var d = new INPUT(); d.type = 1; d.u.ki.wScan = c; d.u.ki.dwFlags = 0x0004u; Send(d);
      var u = new INPUT(); u.type = 1; u.u.ki.wScan = c; u.u.ki.dwFlags = 0x0004u | 0x0002u; Send(u);
      Thread.Sleep(15);
    }
  }
  public static void Button(bool up) { var i = new INPUT(); i.type = 0; i.u.mi.dwFlags = up ? 0x0004u : 0x0002u; Send(i); }
  public static void RightButton(bool up) { var i = new INPUT(); i.type = 0; i.u.mi.dwFlags = up ? 0x0010u : 0x0008u; Send(i); }
  public static void ClickAt(int x, int y) { SetCursorPos(x, y); Thread.Sleep(80); Button(false); Thread.Sleep(50); Button(true); Thread.Sleep(30); }
  public static void RightClickAt(int x, int y) { SetCursorPos(x, y); Thread.Sleep(80); RightButton(false); Thread.Sleep(50); RightButton(true); Thread.Sleep(30); }
  public static bool IsDown(int vk) { return (GetAsyncKeyState(vk) & 0x8000) != 0; }
  public static string Held() {
    var l = new List<string>();
    for (int vk = 1; vk < 255; vk++) {
      if (vk == 0x14 || vk == 0x90 || vk == 0x91) continue;
      if ((GetAsyncKeyState(vk) & 0x8000) != 0) l.Add(vk.ToString("X2"));
    }
    return string.Join(",", l.ToArray());
  }
  public static void ReleaseAll() {
    int[] vks = { 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5 };
    foreach (int vk in vks) if (IsDown(vk)) Vk(vk, true);
    if (IsDown(0x01)) Button(true); if (IsDown(0x02)) RightButton(true);
  }
  // A key posted to a window (no input desktop needed): WebView2's render
  // widget passes it to the page as a key event.
  public static void PostKey(IntPtr h, int vk) {
    uint scan = MapVirtualKey((uint)vk, 0);
    bool ext = vk == 0x24 || vk == 0x23 || (vk >= 0x25 && vk <= 0x28);
    long l = 1 | ((long)scan << 16) | (ext ? (1L << 24) : 0);
    PostMessage(h, 0x0100, (IntPtr)vk, (IntPtr)l);
    Thread.Sleep(40);
    PostMessage(h, 0x0101, (IntPtr)vk, (IntPtr)(l | (1L << 30) | (1L << 31)));
    Thread.Sleep(40);
  }

  // ---- the tray icon's popup menu (#32768): MN_GETHMENU and its strings ----
  [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetMenuString(IntPtr m, uint id, StringBuilder s, int max, uint flag);
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint i, out RECT r);
  public static IntPtr MenuOf(IntPtr h) { return SendMessage(h, 0x01E1, IntPtr.Zero, IntPtr.Zero); }
  public static string[] MenuStrings(IntPtr hm) {
    int n = GetMenuItemCount(hm); var r = new string[Math.Max(n, 0)];
    for (int i = 0; i < n; i++) { var sb = new StringBuilder(256); GetMenuString(hm, (uint)i, sb, 256, 0x400); r[i] = sb.ToString(); }
    return r;
  }

  // ---- the screen: the fraction of pixels of a rectangle that differ
  // between two captures (any channel by more than 24) ----
  public static int[] Capture(int x, int y, int w, int h) {
    using (var bmp = new System.Drawing.Bitmap(w, h)) {
      using (var g = System.Drawing.Graphics.FromImage(bmp)) { g.CopyFromScreen(x, y, 0, 0, new System.Drawing.Size(w, h)); }
      var px = new int[w * h];
      for (int j = 0; j < h; j++) for (int i = 0; i < w; i++) px[j * w + i] = bmp.GetPixel(i, j).ToArgb();
      return px;
    }
  }
  public static double Differ(int[] a, int[] b) {
    if (a == null || b == null || a.Length != b.Length || a.Length == 0) return -1;
    int n = 0;
    for (int i = 0; i < a.Length; i++) {
      int x = a[i], y = b[i];
      if (Math.Abs(((x >> 16) & 255) - ((y >> 16) & 255)) > 24 || Math.Abs(((x >> 8) & 255) - ((y >> 8) & 255)) > 24 || Math.Abs((x & 255) - (y & 255)) > 24) n++;
    }
    return (double)n / a.Length;
  }
}

// The game: a plain window (not topmost) of this script that records the
// left clicks (client coordinates) and the keys (virtual keys) it gets.
public static class OvGame {
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
  // [kind, detail, x, y]: kind 1 key down (detail: virtual key), 3 left button down.
  public static long[][] Since(int mark) {
    lock (gate) {
      var r = new long[Math.Max(0, events.Count - mark)][];
      for (int i = mark; i < events.Count; i++) r[i - mark] = events[i];
      return r;
    }
  }
  static void Note(long kind, long detail, IntPtr l) {
    long v = (long)l; long x = (short)(v & 0xFFFF), y = (short)((v >> 16) & 0xFFFF);
    lock (gate) { events.Add(new long[] { kind, detail, x, y }); }
  }
  static IntPtr Proc(IntPtr h, uint m, IntPtr w, IntPtr l) {
    switch (m) {
      case 0x0201: Note(3, 1, l); break;
      case 0x0100: case 0x0104: Note(1, (long)w & 0xFFFF, IntPtr.Zero); break;
      case 0x0010: DestroyWindow(h); return IntPtr.Zero;
      case 0x0002: PostQuitMessage(0); return IntPtr.Zero;
    }
    return DefWindowProc(h, m, w, l);
  }
  public static IntPtr Open(int x, int y, int w, int hgt) {
    var ready = new ManualResetEvent(false);
    var t = new Thread(delegate() {
      OvW.SetThreadDpiAwarenessContext(new IntPtr(-4));
      keep = Proc;
      var wc = new WNDCLASS();
      wc.proc = keep; wc.inst = GetModuleHandle(null); wc.name = "sotyware-e2e-game";
      wc.cursor = LoadCursor(IntPtr.Zero, new IntPtr(32512)); wc.brush = CreateSolidBrush(0x00505050);
      RegisterClass(ref wc);
      // WS_POPUP | WS_VISIBLE, no extended style: an ordinary window.
      hwnd = CreateWindowEx(0, "sotyware-e2e-game", "sotyware e2e game", 0x80000000u | 0x10000000u, x, y, w, hgt, IntPtr.Zero, IntPtr.Zero, wc.inst, IntPtr.Zero);
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
# Physical pixels everywhere (windows, the pointer, UI Automation
# rectangles), as sotyware places the overlay.
[void][OvW]::SetThreadDpiAwarenessContext([IntPtr](-4))
function Procs { @(Get-Process sotyware -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }

# --- unattended: create the private desktop and run this script on it ------------
if (-not $Real -and $OnDesktop -eq "") {
  $cv = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion"
  "sotyware notes overlay test on a private desktop  $(Get-Date -Format s)"
  "os: $((Get-CimInstance Win32_OperatingSystem).Caption) $($cv.DisplayVersion) build $($cv.CurrentBuild).$($cv.UBR)"
  "binary: $Exe"
  if ((Procs).Count -gt 0) { "ABORT: sotyware from this binary is already running"; exit 2 }
  $elevated = (New-Object Security.Principal.WindowsPrincipal ([Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  if ($elevated) { "NOT RUN: this session is elevated; WebView2 cannot attach to a window on a private desktop then (windows-isolated.ps1). Run it from a non-elevated PowerShell."; exit 4 }
  $NormalBefore = Get-NormalProfileFingerprint
  $NormalCredBefore = Get-NormalCredentialLines
  $Prof = New-TestProfile $ProfileParent ("overlay-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddHHmmssfff"))
  "test profile: $($Prof.Root)"
  $RealBefore = Get-RealNormalProfileFingerprint $Prof.Parent
  if ($null -eq $RealBefore) {
    "FAIL  the user's real app folder can be read from outside this process tree before the run  WMI could not start the reader"
    Remove-TestProfile $Prof
    "ABORT: isolation cannot be proven; nothing was started"; exit 1
  }
  $env:SOTYWARE_TEST_PROFILE = $Prof.Root
  $deskName = "sotyware-overlay-$PID"
  $desk = [OvW]::CreateDesktop($deskName, [IntPtr]::Zero, [IntPtr]::Zero, 0, 0x10000000, [IntPtr]::Zero)
  if ($desk -eq [IntPtr]::Zero) { "ABORT: CreateDesktop failed (error $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))"; Remove-TestProfile $Prof; exit 3 }
  $logFile = Join-Path ([IO.Path]::GetTempPath()) "$deskName.log"
  $psExe = Join-Path $env:WINDIR "System32\WindowsPowerShell\v1.0\powershell.exe"
  $cmdLine = "`"$psExe`" -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Exe `"$Exe`" -OnDesktop `"$deskName`" -Log `"$logFile`""
  try { $driver = [OvW]::Start($psExe, $cmdLine, "WinSta0\$deskName") }
  catch { "ABORT: could not start the test on desktop ${deskName}: $($_.Exception.Message)"; [OvW]::CloseDesktop($desk) | Out-Null; Remove-TestProfile $Prof; exit 3 }
  $code = [OvW]::WaitExit($driver, [uint32]($Timeout * 1000))
  if ($code -lt 0) { "TIMEOUT after $Timeout s"; [OvW]::Kill($driver); $code = 124 }
  Procs | ForEach-Object { Stop-Process -Id $_.Id -Force -EA SilentlyContinue }
  if (Test-Path $logFile) {
    $lines = Get-Content $logFile -Encoding UTF8; $lines; Remove-Item $logFile -EA SilentlyContinue
    if ($code -eq 0 -and @($lines | Where-Object { $_ -like "FAIL *" }).Count -gt 0) { "ERROR: FAIL lines but exit code 0; reporting 1"; $code = 1 }
  }
  else { "ABORT: the test on desktop $deskName wrote no output"; if ($code -ne 124) { $code = 3 } }
  [OvW]::CloseDesktop($desk) | Out-Null
  for ($i = 0; $i -lt 50 -and (Procs).Count -gt 0; $i++) { Start-Sleep -Milliseconds 100 }
  try { Remove-TestProfile $Prof; "PASS  cleanup: this run's test profile removed" }
  catch { "FAIL  cleanup: this run's test profile removed  $_"; $code = 1 }
  if (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|")) { "PASS  the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" }
  else { "FAIL  the user's own sotyware folders are unchanged"; $code = 1 }
  if (((Get-NormalCredentialLines) -join "|") -eq ($NormalCredBefore -join "|")) { "PASS  the user's own sotyware credentials are unchanged" }
  else { "FAIL  the user's own sotyware credentials are unchanged"; $code = 1 }
  $realAfter = Get-RealNormalProfileFingerprint $Prof.Parent
  if ($null -ne $realAfter -and (@($realAfter) -join "|") -eq (@($RealBefore) -join "|")) { "PASS  the user's real app folder, read from outside this process tree, is unchanged" }
  else { "FAIL  the user's real app folder, read from outside this process tree, is unchanged"; $code = 1 }
  exit $code
}

# --- the test itself: on the real desktop, or the copy on the private desktop -----
function Say([string]$line) {
  if ($Log -ne "") { Add-Content -Path $Log -Value $line -Encoding UTF8 } else { [Console]::Out.WriteLine($line) }
}
$script:passes = 0; $script:fails = 0
function Check([string]$what, [bool]$good, $detail = "") {
  if ($good) { $script:passes++; Say "PASS  $what" }
  else { $script:fails++; $d = "$detail"; if ($d.Length -gt 900) { $d = $d.Substring(0, 900) }; Say "FAIL  $what  $d" }
}
function Info([string]$what, $detail) { Say "INFO  ${what}: $detail" }
function WaitFor([scriptblock]$cond, [double]$seconds = 10) {
  $end = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $end) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 150 }
  return [bool](& $cond)
}

$ownDesk = [OvW]::DesktopName([OvW]::GetThreadDesktop([OvW]::GetCurrentThreadId()))
$inputDesk = [OvW]::DesktopName([OvW]::OpenInputDesktop(0, $false, 0x0001))
if ($Real) {
  if ($inputDesk -eq "" -or $inputDesk -ne $ownDesk) { Say "ABORT: not on the interactive input desktop (this desktop: '$ownDesk', input desktop: '$inputDesk'); real input needs an unlocked, logged-in desktop session"; exit 3 }
  if ([OvW]::FindWindow("Shell_TrayWnd", $null) -eq [IntPtr]::Zero) { Say "ABORT: no taskbar (Shell_TrayWnd) on this desktop; quitting needs the tray icon"; exit 3 }
} elseif ($ownDesk -ne $OnDesktop) { Say "ABORT: running on desktop '$ownDesk', expected '$OnDesktop'"; exit 3 }
Info "desktop" $(if ($Real) { "the real desktop, with real input" } else { "private desktop '$ownDesk', no input" })

$A = [System.Windows.Automation.AutomationElement]; $TS = [System.Windows.Automation.TreeScope]
$CT = [System.Windows.Automation.ControlType]
$Invoke = [System.Windows.Automation.InvokePattern]::Pattern
$ValueP = [System.Windows.Automation.ValuePattern]::Pattern
$ToggleP = [System.Windows.Automation.TogglePattern]::Pattern
$SelItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
$OvTitle = "sotyware notes (test profile)"
# Virtual keys, and RegisterHotKey's modifiers.
$VkCtrl = 0xA2; $VkShift = 0xA0; $VkAlt = 0x12; $VkEnter = 0x0D; $VkEsc = 0x1B; $VkHome = 0x24; $VkDown = 0x28; $VkA = 0x41
$VkF = @{ 1 = 0x70; 2 = 0x71; 3 = 0x72; 4 = 0x73; 5 = 0x74 }
$ModCtrlShift = 6

# ---- sotyware's two windows and their pages ----
function Pids { [uint32[]]@((Procs).Id) }
function TauriWindows { $p = Pids; if ($p.Count -eq 0) { return @() }; @([OvW]::Find("Tauri Window", $p)) }
function OvHwnd { foreach ($h in TauriWindows) { if ([OvW]::TitleOf($h) -ceq $OvTitle) { return $h } }; [IntPtr]::Zero }
function DashHwnd { foreach ($h in TauriWindows) { if ([OvW]::TitleOf($h) -cne $OvTitle) { return $h } }; [IntPtr]::Zero }
function OvShown { $h = OvHwnd; ($h -ne [IntPtr]::Zero) -and [OvW]::IsWindowVisible($h) }
function Dash { $h = DashHwnd; if ($h -eq [IntPtr]::Zero) { $null } else { $A::FromHandle($h) } }
function Ov { $h = OvHwnd; if ($h -eq [IntPtr]::Zero -or -not [OvW]::IsWindowVisible($h)) { $null } else { $A::FromHandle($h) } }
function PCT($type) { [System.Windows.Automation.PropertyCondition]::new($A::ControlTypeProperty, $type) }
function PCN($label) { [System.Windows.Automation.PropertyCondition]::new($A::NameProperty, $label) }
function NamesOf($el) { if (-not $el) { return @() }; try { @($el.FindAll($TS::Descendants, [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name }) } catch { @() } }
# A page's text with its pieces joined as they read.
function DashText { -join @(NamesOf (Dash)) }
function OvText { -join @(NamesOf (Ov)) }
function DashSays([string]$phrase) { (DashText).Contains($phrase) }
function OvSays([string]$phrase) { (OvText).Contains($phrase) }
# The last element of that type and name in a page.
function NamedIn($el, [string]$label, $type) {
  if (-not $el) { return $null }
  try { $all = @($el.FindAll($TS::Descendants, [System.Windows.Automation.AndCondition]::new((PCT $type), (PCN $label)))) } catch { return $null }
  if ($all.Count -eq 0) { $null } else { $all[$all.Count - 1] }
}
function ButtonsOf($el) { if (-not $el) { return @() }; try { @($el.FindAll($TS::Descendants, (PCT $CT::Button)) | ForEach-Object { $_.Current.Name.Trim() }) } catch { @() } }
# WaitFor runs its block in a child scope: results go through $script: variables.
function PressIn([string]$where, [string]$label) {
  $script:btn = $null
  WaitFor { $root = if ($where -eq "overlay") { Ov } else { Dash }; $script:btn = NamedIn $root $label $CT::Button; ($null -ne $script:btn) -and $script:btn.Current.IsEnabled } 10 | Out-Null
  if (-not $script:btn) { Info "no button '$label' in the $where" ((ButtonsOf $(if ($where -eq "overlay") { Ov } else { Dash })) -join ", "); return $false }
  try { $script:btn.GetCurrentPattern($Invoke).Invoke(); $true } catch { Info "press '$label'" $_.Exception.Message; $false }
}
function Press([string]$label) { PressIn "dashboard" $label }
function OvPress([string]$label) { PressIn "overlay" $label }
function Link([string]$label) { $l = NamedIn (Dash) $label $CT::Hyperlink; if (-not $l) { return $false }; try { $l.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false } }
function Tab([string]$label) {
  $t = NamedIn (Dash) $label $CT::TabItem
  if (-not $t) { Info "no tab" $label; return $false }
  try { $t.GetCurrentPattern($SelItem).Select(); return $true } catch { }
  try { $t.GetCurrentPattern($Invoke).Invoke(); $true } catch { Info "tab '$label'" $_.Exception.Message; $false }
}
# The overlay's note tabs, "name" or "name*" for the selected one.
function NoteTabs {
  $o = Ov; if (-not $o) { return @() }
  try { @($o.FindAll($TS::Descendants, (PCT $CT::TabItem)) | ForEach-Object { $sel = $false; try { $sel = $_.GetCurrentPattern($SelItem).Current.IsSelected } catch { }; $_.Current.Name.Trim() + $(if ($sel) { "*" } else { "" }) }) } catch { @() }
}
function SelectedNote { $s1 = @(NoteTabs | Where-Object { $_.EndsWith("*") }); if ($s1.Count -eq 1) { $s1[0].TrimEnd("*") } else { $null } }
function SelectNote([string]$label) {
  $t = NamedIn (Ov) $label $CT::TabItem; if (-not $t) { return $false }
  try { $t.GetCurrentPattern($SelItem).Select(); return $true } catch { }
  try { $t.GetCurrentPattern($Invoke).Invoke(); $true } catch { $false }
}
function FieldIn($el, [string]$label) {
  if (-not $el) { return $null }
  try { @($el.FindAll($TS::Descendants, (PCN $label)) | Where-Object { $_.Current.ControlType -eq $CT::Edit })[0] } catch { $null }
}
function SetValueOf($el, [string]$text) { if (-not $el) { return $false }; try { $el.GetCurrentPattern($ValueP).SetValue($text); Start-Sleep -Milliseconds 300; $true } catch { Info "set value" $_.Exception.Message; $false } }
function FocusedName { try { $A::FocusedElement.Current.Name } catch { "" } }
# Settings > Notes overlay. An Invoke of the sidebar's link was once lost
# right after a launch on the real desktop (the page stayed on the
# Overview): the link is invoked again, at most three times, and said so.
function Settings {
  $opened = $false
  for ($try = 1; $try -le 3 -and -not $opened; $try++) {
    if ($try -gt 1) { Info "Settings" "the page did not open; invoking its link again ($try)" }
    $opened = (Link "Settings") -and (WaitFor { $null -ne (NamedIn (Dash) "Notes overlay" $CT::TabItem) } 4)
  }
  $opened -and (Tab "Notes overlay") -and (WaitFor { (DashSays "Off.") -or (DashSays "Shown, ") -or (DashSays "Hidden, ") } 10)
}
function StatusLine { $t = DashText; foreach ($k in @("Off.", "Shown, ", "Hidden, ")) { $i = $t.IndexOf($k); if ($i -ge 0) { return $t.Substring($i, [Math]::Min(60, $t.Length - $i)) } }; "" }
# The names of the taskbar's sotyware buttons (Windows groups an app's
# windows into one button whose name counts them, in the system's language).
function TaskButtons {
  $tray = [OvW]::FindWindow("Shell_TrayWnd", $null); if ($tray -eq [IntPtr]::Zero) { return "" }
  (@(NamesOf ($A::FromHandle($tray)) | Where-Object { $_ -like "sotyware*" }) | Sort-Object) -join " | "
}
# The dashboard in front, as after a person's click on it: on the page's
# heading, which does nothing.
function FrontDash([string]$heading) {
  $dh = DashHwnd
  # Behind another program's window, it is brought forward the way UI
  # Automation does (windows-autoclicker.ps1's selects rely on it), then
  # clicked like a person would.
  if ([OvW]::GetForegroundWindow() -ne $dh) {
    try { (Dash).SetFocus() } catch { }
    $null = WaitFor { [OvW]::GetForegroundWindow() -eq $dh } 3
  }
  $hd = NamedIn (Dash) $heading $CT::Text
  if (-not $hd) { $hd = NamedIn (Dash) $heading $CT::Header }
  if (-not $hd) { Info "no heading" $heading; return $false }
  $hr = $hd.Current.BoundingRectangle
  $at = @(([int]($hr.X + [Math]::Min(10, $hr.Width / 2))), ([int]($hr.Y + $hr.Height / 2)))
  if ([OvW]::RootAt($at[0], $at[1]) -ne $dh) { Info "front the dashboard" "something covers its heading"; return $false }
  [OvW]::ClickAt($at[0], $at[1])
  WaitFor { [OvW]::GetForegroundWindow() -eq $dh } 3
}
# A feature's checkbox in Settings > Features.
function FeatureBox([string]$label) { NamedIn (Dash) $label $CT::CheckBox }

# Chooses `option` in the <select> `label` with the keyboard (WebView2
# ignores the Value pattern's SetValue on a select and shows its options
# only while open; windows-autoclicker.ps1): focus, Home, then Down until
# its Value reads the option. Real keys on the real desktop; on the private
# desktop the keys are posted to the page's render widget.
function Choose([string]$label, [string]$option) {
  $script:cb = $null
  WaitFor { $d = Dash; $script:cb = if ($d) { @($d.FindAll($TS::Descendants, (PCT $CT::ComboBox)) | Where-Object { $_.Current.Name.StartsWith($label, [StringComparison]::Ordinal) })[0] } else { $null }; $null -ne $script:cb } 5 | Out-Null
  if (-not $script:cb) { Info "no select" $label; return $false }
  try { $vp = $script:cb.GetCurrentPattern($ValueP) } catch { Info "select '$label'" "no Value pattern"; return $false }
  if ("$($vp.Current.Value)".Trim() -ceq $option) { return $true }
  try { $script:cb.SetFocus() } catch { }
  if (-not (WaitFor { $script:cb.Current.HasKeyboardFocus -and (-not $Real -or [OvW]::GetForegroundWindow() -eq (DashHwnd)) } 3)) { Info "select '$label'" "could not be focused"; return $false }
  $widget = [OvW]::Child((DashHwnd), "Chrome_RenderWidgetHostHWND")
  $keyPress = { param($vk) if ($Real) { [OvW]::Tap($vk) } else { [OvW]::PostKey($widget, $vk) } }
  & $keyPress $VkHome; Start-Sleep -Milliseconds 700
  $seen = @("$($vp.Current.Value)".Trim())
  for ($k = 0; $k -lt 20 -and $seen[-1] -cne $option; $k++) {
    & $keyPress $VkDown
    $script:sv = $seen[-1]
    if (-not (WaitFor { $script:sv = "$($vp.Current.Value)".Trim(); $script:sv -cne $seen[-1] } 1.5)) { break }
    $seen += $script:sv
  }
  if ($seen[-1] -ceq $option) { Start-Sleep -Milliseconds 300; return $true }
  Info "select '$label'" "no option '$option' among $($seen -join ', ')"
  $false
}

# ---- where the overlay should be ----
function Scale([int[]]$mon) { [double]$mon[4] / 96.0 }
function Px([double]$logical, [double]$sc) { [int][Math]::Round($logical * $sc) }
# The default placement: a corner of the monitor's work area, 24 px in,
# 360 x 440 at that monitor's scaling (overlay::placement).
function Expected([int[]]$mon, [string]$corner) {
  $sc = Scale $mon; $w = Px 360 $sc; $h = Px 440 $sc; $m = Px 24 $sc
  $x = if ($corner -like "*right") { $mon[0] + $mon[2] - $m - $w } else { $mon[0] + $m }
  $y = if ($corner -like "top*") { $mon[1] + $m } else { $mon[1] + $mon[3] - $m - $h }
  @($x, $y, $w, $h)
}
function Near([int[]]$have, [int[]]$want, [int]$slack = 2) {
  if (-not $have -or -not $want -or $have.Count -ne $want.Count) { return $false }
  for ($i = 0; $i -lt $want.Count; $i++) { if ([Math]::Abs($have[$i] - $want[$i]) -gt $slack) { return $false } }
  $true
}
function OvRect { $h = OvHwnd; if ($h -eq [IntPtr]::Zero) { $null } else { [OvW]::Rect($h) } }
function ExStyle { $h = OvHwnd; if ($h -eq [IntPtr]::Zero) { 0 } else { [OvW]::GetWindowLong($h, -20) } }
function HasEx([int]$bit) { ((ExStyle) -band $bit) -ne 0 }
$WsExTopmost = 0x8; $WsExTransparent = 0x20; $WsExToolWindow = 0x80; $WsExAppWindow = 0x40000; $WsExLayered = 0x80000; $WsExNoActivate = 0x8000000
# Whether showing activated the overlay: the active window of sotyware's
# window thread (the dashboard's and the overlay's) is not the overlay.
function NotActivated { $h = OvHwnd; ($h -ne [IntPtr]::Zero) -and [OvW]::ActiveInThreadOf($h) -ne $h }
function ActiveName { $h = OvHwnd; if ($h -eq [IntPtr]::Zero) { return "no overlay" }; $act = [OvW]::ActiveInThreadOf($h); if ($act -eq [IntPtr]::Zero) { "none" } else { [OvW]::TitleOf($act) } }

# ---- notes.sqlite3, read with node:sqlite ----
$NotesJs = "const {DatabaseSync}=require('node:sqlite');let d;try{d=new DatabaseSync(process.argv[1],{readOnly:true})}catch(e){console.log('[]');process.exit(0)}" +
  "try{console.log(JSON.stringify(d.prepare('SELECT title, body, pinned FROM notes ORDER BY pinned DESC, position, id').all()))}catch(e){console.log('[]')}"
# "title|body|pinned" per note, the body's line breaks as \n.
function NotesRows {
  $raw = (& node --no-warnings -e $NotesJs $NotesDb 2>$null) -join ""
  if (-not $raw) { return @() }
  # Windows PowerShell 5.1 writes a parsed JSON array as one object.
  $parsed = $raw | ConvertFrom-Json
  $rows = @()
  foreach ($row in @($parsed)) {
    if ($null -ne $row) { $rows += "$($row.title)|$(([string]$row.body).Replace("`r", '').Replace("`n", '\n'))|$($row.pinned)" }
  }
  $rows
}
function Titles { @(NotesRows | ForEach-Object { $_.Split("|")[0] }) }

# ---- the shortcuts ----
function Free { @(1..5 | Where-Object { [OvW]::CanRegister($ModCtrlShift, $VkF[$_]) }) }
function Shortcut([int]$n) { [OvW]::Chord(@($VkCtrl, $VkShift), $VkF[$n]) }

# ---- the game's events ----
function Mark { [OvGame]::Count }
function Clicks([int]$mark) { @([OvGame]::Since($mark) | Where-Object { $_[0] -eq 3 } | ForEach-Object { "$($_[2]),$($_[3])" }) }
function KeysSince([int]$mark) { @([OvGame]::Since($mark) | Where-Object { $_[0] -eq 1 } | ForEach-Object { [int]$_[1] }) }

# ---- the tray (as in windows-autoclicker.ps1): real right click, then Quit ----
$UiRoot = $A::RootElement
$OverflowClass = "TopLevelWindowForOverflowXamlIsland"
function OverflowShown { $o = $UiRoot.FindFirst($TS::Children, [System.Windows.Automation.PropertyCondition]::new($A::ClassNameProperty, $OverflowClass)); ($null -ne $o) -and -not $o.Current.BoundingRectangle.IsEmpty }
function OverflowOpen { (OverflowShown) -and [OvW]::ClassOf([OvW]::GetForegroundWindow()) -eq $OverflowClass }
function TrayIcons {
  @($UiRoot.FindAll($TS::Descendants, [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.PropertyCondition]::new($A::AutomationIdProperty, "NotifyItemIcon"), (PCN "sotyware test profile"))) |
    Where-Object { $r = $_.Current.BoundingRectangle; -not $r.IsEmpty -and -not [double]::IsNaN($r.X) })
}
function OpenOverflow {
  if (OverflowOpen) { return }
  if (OverflowShown) { WaitFor { -not (OverflowShown) } 3 | Out-Null }
  if ((TrayIcons).Count -ge 1) { return }
  $chev = $UiRoot.FindFirst($TS::Descendants, (PCN "Show Hidden Icons"))
  if ($chev) { $chev.GetCurrentPattern($Invoke).Invoke(); WaitFor { OverflowOpen } 3 | Out-Null; Start-Sleep -Milliseconds 400 }
}
function TrayQuit {
  $script:icon = $null
  WaitFor { OpenOverflow; $script:icon = @(TrayIcons)[0]; $null -ne $script:icon } 8 | Out-Null
  if (-not $script:icon) { $script:why = "tray icon not found"; return $false }
  $r = $script:icon.Current.BoundingRectangle
  [OvW]::RightClickAt([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2))
  $script:menuWnd = [IntPtr]::Zero
  WaitFor { $mw = @([OvW]::Find("#32768", (Pids))); if ($mw.Count -ge 1) { $script:menuWnd = $mw[0] }; $script:menuWnd -ne [IntPtr]::Zero } 4 | Out-Null
  if ($script:menuWnd -eq [IntPtr]::Zero) { $script:why = "context menu did not open"; return $false }
  $hm = [OvW]::MenuOf($script:menuWnd)
  $idx = [array]::IndexOf([OvW]::MenuStrings($hm), "Quit")
  if ($idx -lt 0) { $script:why = "no Quit in the menu: $([OvW]::MenuStrings($hm) -join '|')"; [OvW]::Tap($VkEsc); return $false }
  $rc = New-Object OvW+RECT
  if (-not [OvW]::GetMenuItemRect([IntPtr]::Zero, $hm, [uint32]$idx, [ref]$rc)) { $script:why = "GetMenuItemRect failed"; [OvW]::Tap($VkEsc); return $false }
  [OvW]::ClickAt([int](($rc.L + $rc.R) / 2), [int](($rc.T + $rc.B) / 2))
  $true
}
# Ends sotyware: the tray's Quit on the real desktop; on the private desktop
# (no taskbar, so no tray) Stop-Process, after the overlay saved its state
# when it was hidden. The exit code, or $null if it did not end.
function QuitApp {
  if ($Real) {
    $script:why = ""
    if (-not (TrayQuit)) { Info "tray Quit" $script:why; return $null }
    if (-not $script:app.WaitForExit(30000)) { return $null }
    return $script:app.ExitCode
  }
  Stop-Process -Id $script:app.Id -Force -EA SilentlyContinue
  if (-not $script:app.WaitForExit(15000)) { return $null }
  0
}
# ADR-027: a launch of its own, in this run's profile, acknowledged before
# anything on a page is touched.
function StartApp {
  if ((Procs).Count -gt 0) { Info "start" "sotyware still runs; not started again"; return $false }
  Clear-TestProfileAck $Prof
  $script:app = Start-Process $Exe -PassThru; $null = $script:app.Handle
  try { $null = Wait-TestProfileAck $Prof $script:app.Id 30 $script:app; Check "launch: the app runs in this run's test profile (acknowledged)" $true }
  catch {
    Check "launch: the app runs in this run's test profile (acknowledged)" $false "$_"
    Procs | ForEach-Object { Stop-Process -Id $_.Id -EA SilentlyContinue }
    throw "ABORT: the app is not provably isolated; nothing else is run"
  }
  WaitFor { (DashHwnd) -ne [IntPtr]::Zero -and $null -ne (NamedIn (Dash) "Settings" $CT::Hyperlink) } 30
}

# ---- test profile (ADR-027) ----
if ($Real) {
  $NormalBefore = Get-NormalProfileFingerprint
  $Prof = New-TestProfile $ProfileParent ("overlay-" + (Get-Date -Format "yyyyMMddHHmmss"))
  $RealBefore = Get-RealNormalProfileFingerprint $Prof.Parent
  if ($null -eq $RealBefore) {
    Check "the user's real app folder can be read from outside this process tree before the run" $false "WMI could not start the reader"
    Remove-TestProfile $Prof
    Say "ABORT: isolation cannot be proven; sotyware was not started"; exit 1
  }
  $env:SOTYWARE_TEST_PROFILE = $Prof.Root
  Say "test profile: $($Prof.Root)"
} else {
  $Prof = Open-TestProfile $env:SOTYWARE_TEST_PROFILE
}
$NotesDb = Join-Path $Prof.DataDir "notes.sqlite3"
$PointerBefore = [OvW]::Cursor()
$mons = @([OvW]::Monitors())
Info "monitors (work area x, y, width, height, dpi, primary)" (($mons | ForEach-Object { "[" + ($_ -join ", ") + "]" }) -join " ")

try {
  $up = StartApp
  Check "startup: the dashboard and its page" $up
  if (-not $up) { throw "ABORT: no dashboard; nothing else is run" }

  # ---- off by default ----
  Check "off by default: no overlay window" ((OvHwnd) -eq [IntPtr]::Zero)
  $free = Free
  Check "off by default: this script can register the five suggested shortcuts (Ctrl+Shift+F1 to F5) itself" ($free.Count -eq 5) "free: F$($free -join ', F')"
  $ok = Settings
  Check "Settings > Notes overlay says it is off" ($ok -and (StatusLine).StartsWith("Off.")) (StatusLine)

  # ---- turned on ----
  $dash = DashHwnd
  if ($Real) {
    $buttonsBefore = TaskButtons
    Check "the dashboard has the keyboard" (FrontDash "Gaming notes overlay")
  }
  $pointer = [OvW]::Cursor()
  $mon = if ($pointer) { [OvW]::MonitorAt($pointer[0], $pointer[1]) } else { @($mons | Where-Object { $_[5] -eq 1 })[0] }
  $ok = (Press "Turn on") -and (WaitFor { OvShown } 15)
  Check "Turn on shows the overlay at once" $ok
  $want = Expected $mon "top-right"
  $og = OvRect
  Check "at the top right of the work area of the pointer's monitor$(if (-not $pointer) { ' (the primary: no pointer on this desktop)' }), 24 px in, 360 x 440 at its scaling ($($mon[4]) dpi)" (Near $og $want) "rect $($og -join ','), want $($want -join ',')"
  $ov = OvHwnd
  Check "its scaling is the monitor's (GetDpiForWindow)" ([int][OvW]::GetDpiForWindow($ov) -eq $mon[4]) "window $([OvW]::GetDpiForWindow($ov)) dpi, monitor $($mon[4])"
  Check "topmost (WS_EX_TOPMOST)" (HasEx $WsExTopmost) ("ex style 0x{0:X}" -f (ExStyle))
  Check "a tool window: no taskbar button, not in Alt+Tab (WS_EX_TOOLWINDOW, no WS_EX_APPWINDOW)" ((HasEx $WsExToolWindow) -and -not (HasEx $WsExAppWindow)) ("ex style 0x{0:X}" -f (ExStyle))
  Start-Sleep -Milliseconds 300
  Check "shown without being activated (sotyware's window thread keeps the dashboard active)" (NotActivated) "active: '$(ActiveName)'"
  $cs = [OvW]::ClientSize($ov)
  Check "without a frame: the whole window is page (client area = window)" ($cs -and $cs[0] -eq $og[2] -and $cs[1] -eq $og[3]) "client $($cs -join 'x'), window $($og[2])x$($og[3])"
  Check "interactive: it takes clicks (no WS_EX_TRANSPARENT)" (-not (HasEx $WsExTransparent)) ("ex style 0x{0:X}" -f (ExStyle))
  if ($Real) {
    Start-Sleep -Milliseconds 500
    Check "shown without taking the keyboard: the dashboard keeps it" ([OvW]::GetForegroundWindow() -eq $dash) "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))'"
    $buttonsAfter = TaskButtons
    Check "no taskbar entry: sotyware's taskbar buttons read as before" ($buttonsBefore -ne "" -and $buttonsAfter -ceq $buttonsBefore) "before '$buttonsBefore', after '$buttonsAfter'"
  }
  $free = Free
  Check "its five shortcuts are registered (this script can register none of them)" (WaitFor { (Free).Count -eq 0 } 10) "free: F$($free -join ', F')"
  Check "the settings say it is shown and interactive" (WaitFor { (StatusLine).StartsWith("Shown, interactive") } 10) (StatusLine)
  Check "the overlay has no note yet" (WaitFor { OvSays "No notes yet." } 10) (OvText)

  # ---- a note ----
  $ok = (OvPress "New note") -and (WaitFor { ((NoteTabs) -join "|") -ceq "New note*" } 10)
  Check "New note in the overlay: one tab, selected" $ok ((NoteTabs) -join " | ")
  $want = "Boss strategy|- [ ] Prepare healing items\n- [ ] Equip gear|0"
  if ($Real) {
    $title = FieldIn (Ov) "Note title"
    $tr = if ($title) { $title.Current.BoundingRectangle } else { $null }
    if ($tr -and [OvW]::RootAt([int]($tr.X + $tr.Width / 2), [int]($tr.Y + $tr.Height / 2)) -eq $ov) { [OvW]::ClickAt([int]($tr.X + $tr.Width / 2), [int]($tr.Y + $tr.Height / 2)) }
    $ok = WaitFor { [OvW]::GetForegroundWindow() -eq $ov -and (FocusedName) -ceq "Note title" } 5
    Check "a click in the title gives the overlay the keyboard" $ok "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))', focus '$(FocusedName)'"
    [OvW]::Chord(@($VkCtrl), $VkA); [OvW]::Type("Boss strategy"); [OvW]::Tap($VkEnter)
    $ok = WaitFor { (FocusedName) -ceq "Note text (Markdown)" } 5
    Check "Enter in the title opens the editor" $ok "focus '$(FocusedName)'"
    [OvW]::Type("- [ ] Prepare healing items"); [OvW]::Tap($VkEnter); [OvW]::Type("Equip gear"); [OvW]::Tap($VkEsc)
    $ok = WaitFor { ((NotesRows) -join "~") -ceq $want } 10
    Check "typed as real keys and autosaved to notes.sqlite3, the checklist continued by Enter" $ok ((NotesRows) -join " ~ ")
  } else {
    $ok = (SetValueOf (FieldIn (Ov) "Note title") "Boss strategy") -and (OvPress "Edit the note") -and (WaitFor { $null -ne (FieldIn (Ov) "Note text (Markdown)") } 5)
    $ok = $ok -and (SetValueOf (FieldIn (Ov) "Note text (Markdown)") "- [ ] Prepare healing items`n- [ ] Equip gear") -and (OvPress "Done")
    $ok = $ok -and (WaitFor { ((NotesRows) -join "~") -ceq $want } 10)
    Check "its title and a checklist set in the overlay, autosaved to notes.sqlite3" $ok ((NotesRows) -join " ~ ")
  }
  $box = $null
  $ok = WaitFor { $script:box = NamedIn (Ov) "Prepare healing items" $CT::CheckBox; $null -ne $script:box } 5
  Check "the note shows again, its checkboxes named by their text" $ok ((NamesOf (Ov)) -join " | ")
  $ok = $ok -and $(try { $script:box.GetCurrentPattern($ToggleP).Toggle(); $true } catch { $false })
  $ok = $ok -and (WaitFor { $r = @(NotesRows); $r.Count -eq 1 -and $r[0].Contains("- [x] Prepare healing items") } 5)
  Check "ticking a checkbox saves at once" $ok ((NotesRows) -join " ~ ")

  if ($Real) {
    # ---- the game, partly under the overlay, and the shortcuts from it ----
    $og = OvRect; $sc = Scale $mon
    $gw = Px 520 $sc; $gh = Px 520 $sc; $gx = $og[0] + $og[2] + 4 - $gw; $gy = $og[1] + (Px 80 $sc)
    $game = [OvGame]::Open($gx, $gy, $gw, $gh)
    # Its corner below and left of the overlay, where a person would click it.
    $frontAt = @(($gx + (Px 20 $sc)), ($gy + $gh - (Px 20 $sc)))
    $point = @(([int]($og[0] + $og[2] / 2)), ($og[1] + $og[3] - (Px 40 $sc)))
    # The game to the front with a click on that corner, as a person would.
    # A window of a program in the background opens behind the one in
    # front (seen on the first run), so it is made topmost for the click
    # only, and an ordinary window again right after: in front of the
    # other programs, below the overlay and every other topmost window.
    # The click is sent only where the game is.
    function FrontGame {
      [void][OvW]::SetWindowPos($game, [IntPtr](-1), 0, 0, 0, 0, 0x0013)
      $reachable = WaitFor { [OvW]::RootAt($frontAt[0], $frontAt[1]) -eq $game } 3
      if ($reachable) { [OvW]::ClickAt($frontAt[0], $frontAt[1]) }
      [void][OvW]::SetWindowPos($game, [IntPtr](-2), 0, 0, 0, 0, 0x0013)
      $reachable -and (WaitFor { [OvW]::GetForegroundWindow() -eq $game } 3)
    }
    $fronted = ($game -ne [IntPtr]::Zero) -and (FrontGame)
    Check "the game window is up, partly under the overlay" ($fronted -and [OvW]::RootAt($point[0], $point[1]) -eq $ov -and [OvW]::RootAt($frontAt[0], $frontAt[1]) -eq $game) "game $([OvW]::Rect($game) -join ','), overlay $($og -join ','), under the corner '$([OvW]::TitleOf([OvW]::RootAt($frontAt[0], $frontAt[1])))'"
    if (-not $fronted) { throw "ABORT: the game window cannot be brought to the front; nothing more is sent" }
    Check "the game has the keyboard" ([OvW]::GetForegroundWindow() -eq $game)
    Check "the overlay stays above the game, which came after it" ([OvW]::Above($ov, $game))
    $mKeys = Mark
    Shortcut 1
    $ok = WaitFor { -not (OvShown) } 5
    Check "Ctrl+Shift+F1 from the game hides the overlay (its window kept)" ($ok -and [OvW]::IsWindow($ov) -and (OvHwnd) -eq $ov)
    Start-Sleep -Milliseconds 400
    $hidden = [OvW]::Capture($og[0], $og[1], $og[2], $og[3])
    Shortcut 1
    $ok = (WaitFor { OvShown } 5) -and (WaitFor { Near (OvRect) $og } 5)
    Start-Sleep -Milliseconds 600
    Check "Ctrl+Shift+F1 shows it again where it was, and the game keeps the keyboard" ($ok -and [OvW]::GetForegroundWindow() -eq $game) "rect $((OvRect) -join ','), want $($og -join ','); foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))'"
    $shownPx = [OvW]::Capture($og[0], $og[1], $og[2], $og[3])
    $diff = [OvW]::Differ($hidden, $shownPx)
    Check "shown, the overlay is drawn on the screen (its pixels differ from what is below)" ($diff -gt 0.2) ("{0:P0} of its pixels differ" -f $diff)
    $m = Mark
    [OvW]::ClickAt($point[0], $point[1]); Start-Sleep -Milliseconds 600
    Check "interactive: a click on the overlay stays there (the game gets nothing)" ((Clicks $m).Count -eq 0) ((Clicks $m) -join " ")
    [OvW]::Tap($VkEsc)
    $null = FrontGame
    Shortcut 2
    # The page writes a shortcut with spaces: "Ctrl + Shift + F2".
    $ok = WaitFor { (OvSays "Click-through") -and (OvSays "Ctrl + Shift + F2") } 5
    Check "Ctrl+Shift+F2: passive, the overlay says so with the shortcut back" $ok (OvText)
    Check "passive: clicks go through it and it never takes the keyboard (WS_EX_TRANSPARENT, WS_EX_NOACTIVATE)" ((HasEx $WsExTransparent) -and (HasEx $WsExNoActivate)) ("ex style 0x{0:X}" -f (ExStyle))
    Check "switched to passive, the game keeps the keyboard" ([OvW]::GetForegroundWindow() -eq $game) "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))'"
    Start-Sleep -Milliseconds 400
    $passivePx = [OvW]::Capture($og[0], $og[1], $og[2], $og[3])
    $diff = [OvW]::Differ($hidden, $passivePx)
    Check "passive, the overlay is still drawn on the screen" ($diff -gt 0.2) ("{0:P0} of its pixels differ from the hidden overlay's" -f $diff)
    Check "passive: the window under the overlay's point is the game's" ([OvW]::RootAt($point[0], $point[1]) -eq $game)
    $m = Mark
    [OvW]::ClickAt($point[0], $point[1])
    $ok = WaitFor { (Clicks $m).Count -ge 1 } 3
    $gr = [OvW]::Rect($game)
    $wantAt = "$($point[0] - $gr[0]),$($point[1] - $gr[1])"
    # @(): a function's one-element list comes back as that element, and
    # [0] of a string is its first character.
    Check "passive: the same click goes through to the game, at that point" ($ok -and @(Clicks $m)[0] -eq $wantAt) "clicks $((Clicks $m) -join ' '), want $wantAt"
    Check "passive: nothing of the overlay to click (no note tab, no title field)" ((NoteTabs).Count -eq 0 -and $null -eq (FieldIn (Ov) "Note title")) ((NoteTabs) -join " | ")
    Shortcut 2
    $ok = WaitFor { -not (OvSays "Click-through") -and (NoteTabs).Count -gt 0 } 5
    Check "Ctrl+Shift+F2 again: interactive" $ok
    Check "switched by shortcut, the overlay takes the keyboard" (WaitFor { [OvW]::GetForegroundWindow() -eq $ov } 5) "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))'"
    Check "interactive again: it takes clicks (no WS_EX_TRANSPARENT)" (-not (HasEx $WsExTransparent)) ("ex style 0x{0:X}" -f (ExStyle))
    $m = Mark
    [OvW]::ClickAt($point[0], $point[1]); Start-Sleep -Milliseconds 600
    Check "interactive again: the click stays on the overlay" ((Clicks $m).Count -eq 0)
    [OvW]::Tap($VkEsc)

    # ---- notes by shortcut ----
    $null = FrontGame
    Shortcut 3
    $ok = WaitFor { (Titles).Count -eq 2 -and (SelectedNote) -ceq "New note" } 10
    Check "Ctrl+Shift+F3 creates a note and shows it" $ok "$((NotesRows) -join ' ~ ') / $((NoteTabs) -join ' | ')"
    $ok = WaitFor { [OvW]::GetForegroundWindow() -eq $ov -and (FocusedName) -ceq "Note title" } 5
    Check "...with the keyboard and the cursor in its title" $ok "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))', focus '$(FocusedName)'"
    [OvW]::Type("Route")
    $ok = WaitFor { ((Titles) -join "|") -ceq "Boss strategy|Route" } 10
    Check "its title, typed at once, is saved" $ok ((NotesRows) -join " ~ ")
    $null = FrontGame
    Shortcut 5
    Check "Ctrl+Shift+F5: the next note (round to the first)" (WaitFor { (SelectedNote) -ceq "Boss strategy" } 5) ((NoteTabs) -join " | ")
    Shortcut 4
    Check "Ctrl+Shift+F4: the previous note" (WaitFor { (SelectedNote) -ceq "Route" } 5) ((NoteTabs) -join " | ")
    Check "switching notes by shortcut leaves the keyboard to the game" ([OvW]::GetForegroundWindow() -eq $game) "foreground '$([OvW]::TitleOf([OvW]::GetForegroundWindow()))'"
    $sent = @(KeysSince $mKeys)
    $fkeys = @($sent | Where-Object { $_ -ge 0x70 -and $_ -le 0x74 })
    Check "the game never got F1 to F5 (sotyware took them)" ($fkeys.Count -eq 0) ("keys: " + (($sent | ForEach-Object { "{0:X2}" -f $_ }) -join ","))
    Check "the game never got an Alt (no key was made up to take the keyboard)" (@($sent | Where-Object { $_ -eq $VkAlt }).Count -eq 0) ("keys: " + (($sent | ForEach-Object { "{0:X2}" -f $_ }) -join ","))

    # ---- idle while shown ----
    $proc = Get-Process -Id $script:app.Id
    $cpu1 = $proc.TotalProcessorTime.TotalSeconds; $t1 = Get-Date
    Start-Sleep -Seconds 5
    $proc.Refresh(); $cpu = ($proc.TotalProcessorTime.TotalSeconds - $cpu1) / ((Get-Date) - $t1).TotalSeconds
    Check "sotyware's own process stays idle while the overlay is shown (under 3 % of a core over 5 s)" ($cpu -lt 0.03) ("{0:N1} % of one core" -f ($cpu * 100))
  } else {
    # ---- passive and back, without input ----
    $ok = (OvPress "Passive: let clicks through to the game") -and (WaitFor { OvSays "Click-through" } 5)
    Check "the overlay's passive button: it says Click-through" $ok (OvText)
    Check "passive: clicks go through it and it never takes the keyboard (WS_EX_TRANSPARENT, WS_EX_NOACTIVATE)" ((HasEx $WsExTransparent) -and (HasEx $WsExNoActivate)) ("ex style 0x{0:X}" -f (ExStyle))
    Check "passive: nothing of the overlay to click (no note tab, no title field)" ((NoteTabs).Count -eq 0 -and $null -eq (FieldIn (Ov) "Note title")) ((NoteTabs) -join " | ")
    Check "the settings say it is passive" (WaitFor { (StatusLine).StartsWith("Shown, passive") } 5) (StatusLine)
    $ok = (Press "Make interactive") -and (WaitFor { -not (OvSays "Click-through") -and (NoteTabs).Count -gt 0 } 5)
    Check "Make interactive in the settings brings it back" ($ok -and -not (HasEx $WsExTransparent)) ("ex style 0x{0:X}" -f (ExStyle))
    $ok = (OvPress "New note (Ctrl+N)") -and (WaitFor { (Titles).Count -eq 2 -and (SelectedNote) -ceq "New note" } 10)
    $ok = $ok -and (SetValueOf (FieldIn (Ov) "Note title") "Route") -and (WaitFor { ((Titles) -join "|") -ceq "Boss strategy|Route" } 10)
    Check "a second note, created and named in the overlay, is saved after the first" $ok ((NotesRows) -join " ~ ")
    $ok = (SelectNote "Boss strategy") -and (WaitFor { (SelectedNote) -ceq "Boss strategy" } 5)
    $ok = $ok -and (SelectNote "Route") -and (WaitFor { (SelectedNote) -ceq "Route" } 5)
    Check "its tabs switch notes" $ok ((NoteTabs) -join " | ")
  }

  # ---- collapse, corner, hiding ----
  $og = OvRect; $sc = [double][OvW]::GetDpiForWindow($ov) / 96.0
  $ok = (OvPress "Collapse to the title bar") -and (WaitFor { $r = OvRect; $r -and $r[3] -le (Px 38 $sc) + 2 } 5)
  Check "Collapse leaves only the title bar (38 px at its scaling)" $ok "rect $((OvRect) -join ',')"
  $ok = (OvPress "Expand") -and (WaitFor { Near (OvRect) $og } 5)
  Check "Expand brings it back at its size" $ok "rect $((OvRect) -join ','), want $($og -join ',')"
  $here = [OvW]::MonitorOf($ov)
  $ok = Settings
  if ($Real) { $ok = $ok -and (FrontDash "Gaming notes overlay") }
  $ok = $ok -and (Choose "Opens at" "Top left")
  $want = Expected $here "top-left"
  $ok = $ok -and (WaitFor { Near (OvRect) $want } 10)
  Check "a new corner in the settings moves the open overlay there" $ok "rect $((OvRect) -join ','), want $($want -join ',')"
  $og = OvRect
  $ok = (OvPress "Hide the overlay (notes are kept)") -and (WaitFor { -not (OvShown) } 5)
  Check "its close button hides it, the window kept" ($ok -and (OvHwnd) -eq $ov)
  Check "the settings say it is hidden" (WaitFor { (StatusLine).StartsWith("Hidden, ") } 5) (StatusLine)
  $ok = (Press "Show overlay") -and (WaitFor { OvShown } 5) -and (WaitFor { Near (OvRect) $og } 5)
  Check "Show overlay brings it back where it was, the same window" ($ok -and (OvHwnd) -eq $ov) "rect $((OvRect) -join ','), want $($og -join ',')"
  Start-Sleep -Milliseconds 300
  Check "shown again without being activated" (NotActivated) "active: '$(ActiveName)'"
  [void][OvW]::PostMessage($ov, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
  $ok = WaitFor { -not (OvShown) } 5
  Check "a close request (WM_CLOSE, as Alt+F4) hides it too; the window and sotyware stay" ($ok -and [OvW]::IsWindow($ov) -and -not $script:app.HasExited)

  # ---- another place, another monitor ----
  $ok = (Press "Show overlay") -and (WaitFor { OvShown } 5)
  $other = @($mons | Where-Object { -not ($_[0] -le $og[0] -and $og[0] -lt $_[0] + $_[2] -and $_[1] -le $og[1] -and $og[1] -lt $_[1] + $_[3]) })[0]
  $target = if ($other) { $other } else { [OvW]::MonitorOf($ov) }
  $tsc = Scale $target
  $to = @(($target[0] + (Px 100 $tsc)), ($target[1] + (Px 80 $tsc)))
  # As a drag ends: the window moved by the system, sotyware told after.
  [void][OvW]::SetWindowPos($ov, [IntPtr]::Zero, $to[0], $to[1], 0, 0, 0x0015)
  $moved = @($to[0], $to[1], (Px 360 $tsc), (Px 440 $tsc))
  $ok = $ok -and (WaitFor { Near (OvRect) $moved 3 } 5)
  Start-Sleep -Milliseconds 800
  $ok = $ok -and (Near (OvRect) $moved 3)
  $movedHow = if ($other) { "onto another monitor ($($target[4]) dpi), it takes that monitor's scaling and stays" } else { "within its monitor (only one monitor), it stays" }
  Check "moved $movedHow" $ok "rect $((OvRect) -join ','), want $($moved -join ',')"
  if ($other -and $target[4] -eq $here[4]) { Info "scaling" "both monitors use $($here[4]) dpi: no change of scaling was exercised" }
  $moved = OvRect
  $ok = (OvPress "Hide the overlay (notes are kept)") -and (WaitFor { -not (OvShown) } 5)
  Check "hidden again (its place saved)" $ok

  # ---- a restart ----
  $code = QuitApp
  Check $(if ($Real) { "the tray's Quit ends sotyware: exit code 0" } else { "sotyware ended (Stop-Process: no tray on a private desktop)" }) ($code -eq 0) "exit code $code"
  $up = StartApp
  Check "sotyware starts again" $up
  if (-not $up) { throw "ABORT: no dashboard after the restart" }
  Start-Sleep -Seconds 2
  Check "after a restart the overlay stays hidden (not set to show at startup)" (-not (OvShown))
  Check "...but its shortcuts are registered again" (WaitFor { (Free).Count -eq 0 } 10) "free: F$((Free) -join ', F')"
  if ($Real) {
    $null = FrontGame
    Shortcut 1
  } else {
    $null = (Settings) -and (Press "Show overlay")
  }
  $ok = (WaitFor { OvShown } 15) -and (WaitFor { Near (OvRect) $moved 3 } 5)
  Check "$(if ($Real) { 'Ctrl+Shift+F1 shows it' } else { 'Show overlay shows it' }) where it was, at that monitor's scaling" $ok "rect $((OvRect) -join ','), want $($moved -join ',')"
  Check "...a tool window again, not activated" ((HasEx $WsExToolWindow) -and -not (HasEx $WsExAppWindow) -and (NotActivated)) ("ex style 0x{0:X}, active '{1}'" -f (ExStyle), (ActiveName))
  $ok = WaitFor { ((NoteTabs | ForEach-Object { $_.TrimEnd("*") }) -join "|") -ceq "Boss strategy|Route" } 10
  Check "with its notes" $ok ((NoteTabs) -join " | ")
  $ok = (SelectNote "Boss strategy") -and (WaitFor { OvSays "Prepare healing items" } 5)
  Check "and their text" $ok

  # ---- turned off ----
  $ok = (Link "Settings") -and (WaitFor { $null -ne (NamedIn (Dash) "Features" $CT::TabItem) } 5) -and (Tab "Features") -and (WaitFor { $null -ne (FeatureBox "Gaming notes overlay") } 10)
  $ok = $ok -and $(try { (FeatureBox "Gaming notes overlay").GetCurrentPattern($ToggleP).Toggle(); $true } catch { $false })
  $ok = $ok -and (WaitFor { (OvHwnd) -eq [IntPtr]::Zero } 10)
  Check "turned off in Settings > Features: the overlay window is gone" $ok
  Check "...and every shortcut is released (this script can register all five)" (WaitFor { (Free).Count -eq 5 } 10) "free: F$((Free) -join ', F')"
  Check "the notes stay in notes.sqlite3" (((Titles) -join "|") -ceq "Boss strategy|Route") ((NotesRows) -join " ~ ")
  Check "sotyware is still running" (-not $script:app.HasExited)
} catch {
  Check "the test ran to its end" $false "$_"
} finally {
  if ($Real) { [OvW]::ReleaseAll() }
  [OvGame]::Close()
  Procs | ForEach-Object { Stop-Process -Id $_.Id -Force -EA SilentlyContinue }
  $null = WaitFor { (Procs).Count -eq 0 } 10
  if ($Real) {
    if ($PointerBefore) { [void][OvW]::SetCursorPos($PointerBefore[0], $PointerBefore[1]) }
    if (Test-Path -LiteralPath $Prof.Root) { Remove-TestProfile $Prof }
    Check "cleanup: this run's test profile removed" (-not (Test-Path -LiteralPath $Prof.Root))
    Check "the user's own sotyware folders are unchanged (names, sizes, times, SHA-256)" (((Get-NormalProfileFingerprint) -join "|") -eq ($NormalBefore -join "|"))
    $realAfter = Get-RealNormalProfileFingerprint $Prof.Parent
    Check "the user's real app folder, read from outside this process tree, is unchanged" ($null -ne $realAfter -and (@($realAfter) -join "|") -eq (@($RealBefore) -join "|"))
    if ($PointerBefore) { Check "the pointer is back where it was" ((([OvW]::Cursor()) -join ",") -eq ($PointerBefore -join ",")) }
    Check "no key or button is left down" ([OvW]::Held() -eq "") ([OvW]::Held())
  }
}
Say ""
Say "summary: $($script:passes) passed, $($script:fails) failed$(if (-not $Real) { ' (unattended: focus, real keys and clicks, the shortcuts themselves, pixels and the tray are checked with -ConfirmRealInput only)' })"
if ($script:fails -gt 0) { exit 1 } else { exit 0 }
