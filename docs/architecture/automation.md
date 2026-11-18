# Autoclicker and macros

Status: implemented 2026-11-17
([ADR-050](decisions.md#adr-050-the-autoclicker-sends-and-records-input-in-rust-through-native-apis-one-session-at-a-time-behind-an-opt-in-feature)).
What was tested where: [status.md](../status/status.md#autoclicker-and-macros-2026-11-17);
what it cannot do: [limitations.md](../status/limitations.md#autoclicker-adr-050);
how to use it: [user guide](../user/automation.md).

sotyware can click automatically, record the user's mouse and keyboard
input into **macros**, edit and replay them, keep **profiles** of settings,
and do all of it from **global shortcuts** that work in every program. The
page only shows and asks: every input is sent and recorded in Rust, by
native APIs, and only while the built-in feature `autoclicker` runs.

## Pieces

| Piece | File | Role |
| --- | --- | --- |
| `Automation` | `src-tauri/src/automation/mod.rs` | Managed state: settings, profiles and the macro library (cached summaries) in `automation.sqlite3`; the one session that may run; shortcut registration; the live status and its subscribers; the 29 commands |
| model | `automation/model.rs` | Pure data and rules: clicker configuration, actions, macros, playback settings, profiles, settings, bindings and their conflicts, screen geometry (virtual desktop, monitor mapping, fail-safe corners), the export file |
| keys | `automation/keys.rs` | The key table (W3C `code` ↔ Windows scan code and virtual key ↔ X11 keycode), `Key`, `Chord` and the shortcut rules |
| timing | `automation/timing.rs` | `Control` (stop and pause from any thread), `Clock` (`RealClock`; `FakeClock` in tests), a small seeded random generator |
| input | `automation/input.rs` | The `Backend` trait (send, read the pointer, monitors and active window, shortcuts, capture), `Output` (what a session holds down, released on every exit path, also from a panic hook), `mock::MockBackend` for tests |
| engines | `automation/clicker.rs`, `recorder.rs`, `player.rs` | The autoclicker, the recorder (raw events → actions) and playback, each over a `Backend` and a `Clock` |
| native | `automation/native.rs`, `windows.rs`, `x11.rs` | The platform's backend and the import/export dialogs |
| storage | `src-tauri/src/storage/automation.rs` | `automation.sqlite3` (settings, profiles, macros as JSON documents) |
| feature | `src-tauri/src/features/autoclicker.rs` | The built-in feature `autoclicker` (off by default): starts and stops `Automation` through `automation::LINK` |
| page | `src/Autoclicker.svelte` and `Autoclicker*.svelte`, `HotkeyInput.svelte`, `PlaybackForm.svelte`; `src/automation.ts`, `automationFormat.ts`, `automationModel.svelte.ts` | Tabs Overview, Clicker, Recorder, Macros (with the editor), Profiles, Settings; typed commands with checked responses; pure helpers; the shared live model ([frontend.md](frontend.md#pages)) |

## Lifecycle

- **Off until the user turns it on.** `autoclicker` has
  `enabled_by_default: false`: on a new installation, or after an update,
  sotyware registers no shortcut and sends or records nothing. The page
  explains what turning it on does (with the shortcuts it will register)
  and has a **Turn on** button (`set_feature_enabled`); Settings › Features
  has the same switch. The choice is saved like every feature preference.
- **Start** (`Automation::start`): refused with the reason where the system
  has no backend (the feature is then `failed` with that text); otherwise
  installs the panic hook once, opens the backend, starts the dispatcher
  thread that turns shortcut presses into actions, and registers the
  shortcuts.
- **Stop** (the switch, or quitting: `RunEvent::Exit` → `stop_all` →
  feature stop; `release_resources` calls `Automation::shutdown` again,
  which does nothing the second time): the backend is taken first (so no
  shortcut can start a session any more), the shortcuts are unregistered,
  the running session is stopped and waited for (at most 3 s), which
  releases everything it held, then the backend's threads end. A stopped
  recording stays as a draft until saved or discarded (in memory).
- **Nothing restarts by itself.** Sessions and drafts are never stored;
  after a restart nothing runs until the user starts it. The active profile
  at startup is the **default profile**.

## Native input

### Windows (`windows.rs`, windows-sys)

- **One input thread** (`sotyware-input`) with a message loop owns the
  shortcuts (`RegisterHotKey` with no window and `MOD_NOREPEAT`, so
  `WM_HOTKEY` arrives in its queue once per press) and, only while a
  recording runs, the low-level hooks (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`).
  Requests reach it through a channel and `PostThreadMessageW`. Hooks run
  on that thread inside `GetMessageW`; they copy the event into the
  recorder's channel with an `Instant` and return at once, so Windows never
  removes them for taking too long. A shortcut press goes to the
  dispatcher thread, never blocking the input thread.
- **Sending** (`SendInput`, from the session's thread): every input carries
  the marker `0x534F5459` ("SOTY") in `dwExtraInfo`; the hooks skip it, so
  playback is never recorded (input injected by other software is recorded
  like the user's). Pointer moves are absolute over the virtual desktop
  (`MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`; Windows maps 0 to the
  desktop's first pixel and 65535 to its last, rounding down, so sotyware
  sends the middle of the pixel's range of values), then checked with
  `GetCursorPos` and put on the exact pixel with `SetCursorPos` if it
  missed, before any button of the same batch. Buttons and keys of one
  batch go in one `SendInput` call (no user input in between).
- **Keys** are sent by scan code (`KEYEVENTF_SCANCODE`, with
  `KEYEVENTF_EXTENDEDKEY` for `0xE0` keys), as the keyboard would: the
  layout of the program that receives them gives the character, and games
  that read scan codes see them. Pause and the media keys go by virtual-key
  code, which is how Windows identifies them. Recorded keys are mapped from
  the hook's scan code and extended flag (the physical key), with the few
  shared scan codes told apart by virtual key (NumLock/Pause, Ctrl+Break,
  Print Screen); a key sotyware cannot replay is counted as left out.
  Shortcuts use the virtual key the current layout gives the physical key
  (`MapVirtualKeyW`), and key names come from `GetKeyNameTextW`.
- **Coordinates** are physical pixels: sotyware is per-monitor DPI aware v2
  (set by tao), so the hooks, `GetCursorPos`, `SetCursorPos` and
  `GetMonitorInfoW` agree on every monitor and scale.
- **Windows** (`GetForegroundWindow`, `GetWindowTextW`,
  `QueryFullProcessImageNameW`, the client area by `GetClientRect` +
  `ClientToScreen`) and sotyware's own window (`WindowFromPoint` +
  `GetAncestor(GA_ROOT)`, compared by process id, since WebView2's content
  windows belong to its own process).
- **Not possible** (Windows' rules): input to programs running as
  administrator unless sotyware runs as administrator too (UIPI;
  `SendInput` says nothing), the secure desktop (UAC, the lock screen).

### Linux on X11 (`x11.rs`, x11-dl)

- libX11, libXtst (XTEST and RECORD) and libXrandr are loaded when the
  feature starts; nothing is linked at build time. A Wayland session is
  refused with the reason (Wayland lets no application send or record other
  programs' input; XWayland would reach X11 programs only), and so is a
  session without `DISPLAY`.
- **Sending**: `XTestFakeMotionEvent`, `XTestFakeButtonEvent` (buttons 1–3,
  8, 9; the wheel as buttons 4–7, one press and release per notch),
  `XTestFakeKeyEvent` with the table's keycode (evdev + 8), then `XSync`.
- **Recording**: the RECORD extension on two connections (one blocks in
  `XRecordEnableContext` on the capture thread, the other disables it),
  device events `KeyPress`…`MotionNotify` from every client, parsed from
  the wire event.
- **Shortcuts**: `XGrabKey` on the root window, with the variants for
  CapsLock and NumLock, on a connection owned by one thread that waits in
  `poll` on the connection and a wake pipe (rustix); held keys fire once
  (`XkbSetDetectableAutoRepeat`). A key another client grabbed fails with
  `BadAccess` → "another program uses this shortcut".
- **Errors**: Xlib reports request errors to one process-wide handler
  whose default ends the process. Every call on sotyware's connections that
  can fail runs in `Conn::trapped`: sotyware's handler is put in place
  first (GTK may have replaced it), the call is synced, and the error is
  returned; errors of other connections go to the handler that was there.
- **Windows** through EWMH: `_NET_ACTIVE_WINDOW` (else the input focus),
  `_NET_WM_PID`, `_NET_WM_NAME`; the program's name from `/proc/<pid>/exe`;
  sotyware's own window by `_NET_WM_PID` under the point
  (`XTranslateCoordinates`); monitors from RandR 1.5 (`XRRGetMonitors`),
  else the root window.

### macOS

No backend yet: `support()` says so, the feature cannot start, and the page
shows the reason; macros and profiles can still be edited, imported and
exported.

## Keys and shortcuts

- A key is its **physical position**, named by the W3C UI Events `code`
  (`KeyA`, `Digit1`, `F6`, `ShiftLeft`, `ArrowUp`, `NumpadEnter`,
  `MediaPlayPause`…). The table in `keys.rs` holds 131 keys; Power, Sleep
  and Wake are deliberately absent (never recorded, never replayed). The
  page gets the list with each key's name on the current keyboard
  (`automation_config.keys`), so it keeps no table of its own.
- A **chord** is `Ctrl+Alt+Shift+Meta+<code>` (modifiers in that order;
  Meta is the Windows key, ⌘ or Super). A shortcut must not take a key
  away from typing: a letter, digit, punctuation, Space, Enter, the
  keypad… needs Ctrl, Alt or Meta; F1–F24, Pause, Scroll Lock and the media
  keys may stand alone. Refused as the system's: F12 alone (Windows'
  debugger key), Ctrl+Alt+Delete, Alt+Tab, Alt+F4, Ctrl/Alt+Esc, Meta+L.
  The page checks the same rules while the user presses keys.
- **Bindings** (`{ chord, enabled }`): the **emergency stop** (Settings,
  shared by every profile; F8 suggested), each profile's actions (toggle
  the autoclicker F6, start it, pause/resume F7, start recording F9, stop
  recording F10, play the profile's macro Shift+F11, stop playback), and
  each macro's own shortcut (plays it, or stops it while it plays). A
  binding can be switched off without forgetting its keys.
- **Conflicts**: two enabled bindings may not share a chord. Saving a
  profile, a macro's shortcut or the settings is refused with the other
  owner's name (`conflict`); bindings that clash anyway (an imported
  profile activated later) register first come first served and the others
  show "has the same shortcut". A chord the system or another program
  holds shows that program's refusal per binding (Settings tab, Overview),
  the others still work.
- **Registration** follows the configuration: the emergency stop, the
  active profile's and the macros' enabled bindings are registered while
  the feature runs (and, while a session runs, the emergency stop with
  other modifiers: see [Safety](#safety)); switching profiles, saving or
  deleting changes only what changed. Shortcut ids stay within 1…0xBFFF
  (Windows' range).
- **What a shortcut does while something runs**: the emergency stop stops
  anything (and releases what it held); toggle stops the autoclicker it
  started; a macro's shortcut stops that macro; a start of anything else is
  refused and the reason appears on the page as a notice ("F9: A macro is
  playing; stop it first"). Shortcut presses are not recorded (below).

## Sessions

One session at a time: the autoclicker, a recording or a playback
(`busy` otherwise). Each runs on its own thread with a `Control`; its
progress goes into a shared slot the status reads. A stop takes effect
within milliseconds; the dashboard's Stop (`stop_automation`) returns once
the session has released everything.

**Starting from the dashboard** waits the settings' countdown (3 s by
default, 0–10 s) on top of the configured start delay, so the pointer can
be moved where it should click or the other program brought to the front.
A run that would go on **until stopped** from the dashboard needs the
user's confirmation (`confirm`) when no emergency stop shortcut is
registered. Shortcut starts need neither: the key press is the user's
explicit action and the same shortcut stops it.

### The autoclicker (`clicker.rs`)

- **Rate**: the interval runs from one click's start to the next one's,
  1 ms to 24 h, shown as clicks per second or milliseconds. Random
  intervals: evenly between a minimum and a maximum, or ± a percentage.
  Deadlines are absolute from the start (kept as fractional milliseconds,
  so 3⅓ ms never drifts); a session that falls more than an interval
  behind continues from now instead of catching up in a burst.
- **Clicks**: left, right, middle, back or forward; single or double (with
  a gap); an optional hold time per click; an optional pause after every
  N clicks (batches).
- **Where**: at the pointer, at a fixed position, at a list of positions in
  turn or at random, or at random points of an area; an optional spread
  around each position; every point is put onto a monitor. Positions are
  picked with a 3-second countdown (`capture_cursor_position`).
- **Limits**: a number of clicks (a double-click counts once), a time
  (paused time not counted), or none.
- **Status**: clicks, clicks in the last second, the target rate, time
  clicking, what is left, clicks skipped over sotyware's window.

### Recording (`recorder.rs`, the session in `mod.rs`)

- Captures pointer movements, button presses and releases, the wheel and
  key presses and releases, each with its time (an `Instant` taken in the
  hook), from the moment the countdown ends. The live list shows the last
  40 actions, updated at most 20 times a second.
- **Timing**: each action keeps its time since the start (pauses not
  counted); delays are differences of times rounded to the millisecond, so
  rounding never adds up. The idle time before the first action is
  dropped (playback has a start delay).
- **Options** (Settings, editable on the Recorder tab): which kinds of
  input; pointer movements at most N per second and only when they moved
  at least M px (the last position before a click, key or scroll is always
  kept); keys and buttons never recorded; how positions replay (absolute,
  relative to the pointer at the start, relative to the window of the
  first click or key).
- **Left out on purpose**: sotyware's own window (clicks and scrolls on
  it, keys while it is active), active shortcuts and the modifier presses
  just before them (the stop shortcut never ends up in the macro), a
  release whose press was not recorded (the shortcut or click that started
  the recording), input while paused. A key held while paused is released
  at the pause; anything still held at the end is released by the macro's
  last actions. Auto-repeat of a key is kept (holding Backspace deletes
  as it did), a held modifier's repeats are not.
- **Limit**: 100,000 actions; recording then stops with the reason.
- **Draft**: a stopped recording waits in memory with its count, length,
  last actions and the first window; **Save** makes it a macro (name made
  unique, description, folder, the recording's screens, pointer and window
  as `recordedOn`, positions as chosen, and the window's program as a
  "warn" target for window positions), **Discard** drops it. A new
  recording needs the draft saved or discarded first. Stopping from the
  dashboard also drops the movements after the last click, key or scroll
  (they only lead to the Stop button).

### Playback (`player.rs`)

- **Timing**: each action happens its delay after the previous one,
  divided by the speed (0.1×–20×) and varied by the jitter (± up to 50 %);
  times are absolute from the start. The start delay and the delay between
  repetitions are not scaled. An endless macro without any delay still
  waits 1 ms between repetitions.
- **Repetition**: once, N times (≤ 1,000,000) or until stopped, and an
  optional time limit. With a **section** (actions *first*…*last*), what
  comes before plays once, the section repeats, what comes after plays
  once.
- **Placement**, decided once after the start delay: absolute (mapped to
  the current monitors by role and relative position when they differ from
  the recording's and scaling is on), relative to the pointer (offset from
  the recording's starting pointer), or relative to the active window
  (offset from the recorded window's client area, optionally scaled to the
  window's size). Every point is then put onto a monitor.
- **Target program** (optional, per macro): the program's file name and
  part of a title; when another program is active at the start or before a
  key, playback either says so and goes on (`warn`) or stops (`stop`).
- **Pause points**: a `pause` action pauses playback until the user
  resumes it (shortcut or page).
- **Status**: repetition, action, its description, progress and time left
  when the end is known, time playing, warnings, actions skipped.

### Safety

- **Everything held is released** on stop, pause (and pressed again on
  resume), the end of a macro, an error, the feature's stop and quitting:
  `Output` tracks every key and button a session pressed and releases them
  in reverse order; dropping it releases too. A panic (release builds abort
  on one) runs a hook installed once that sends the pending releases
  before the process ends. A process killed from outside cannot release
  anything; the user's next press and release of that key does.
- **Emergency stop**: a shortcut shared by every profile, handled on its
  own thread, stops whatever runs. The system matches a shortcut's
  modifiers exactly (`RegisterHotKey`, `XGrabKey`), so while a macro holds
  Shift the user's F8 arrives as Shift+F8: while a session runs, the
  emergency stop is also registered with every other combination of Ctrl,
  Alt, Shift and Meta (`Chord::with_other_modifiers`, leaving out the
  system's own), never listed, and given up when the session ends; a
  combination another shortcut or program already has stays theirs. The
  recorder leaves those combinations out too. Found by the Linux desktop
  test (2026-11-18): F8 stopped nothing while a macro held Shift.
- **Fail-safe corners** (on by default): pushing the pointer into a corner
  of the screen (one of the bounding box's corners that lies on a monitor,
  within 2 px) stops the autoclicker or playback before its next action.
  Not the exact spot the run itself last moved the pointer to: a fixed
  position, an area's random point or a recorded position in a corner is
  clicked like any other, and only the user's push counts (found by the
  Linux desktop test: a fixed position left at 0, 0 clicked once, then the
  clicker took its own move for the user's push and stopped). The check
  runs again right before each move (`pushed_into_corner`), so a push
  while the engine looks where its click would land is not undone by that
  move (also found by the desktop test); what is left is the instant
  between that check and the move, which a hand pushing a mouse outlasts.
- **sotyware's own window is never clicked**: a click, press or scroll that
  would land on it is skipped (with its release), and keys are not sent
  while it is active, so the dashboard's Stop always works. A skipped click
  does not count towards the clicker's number of clicks (a time limit
  still ends the run).
- **One session at a time**, nothing restarts after a restart, shortcuts
  only while the feature runs, and the confirmation above.

## Status and the page

`watch_automation` sends the `AutomationStatus` on a per-call channel at
once and whenever it changes (at most ten times a second; at least once a
second while a session runs, so the clock moves): whether the feature runs
and the system's support, the session's view, the last session's outcome,
the draft, every registered (or refused) shortcut with its keys' names, the
active profile, the program in front while a session runs, a notice from a
shortcut that could not act, and a storage error. The configuration
(`automation_config`: settings, profiles, the active profile, the key list,
the suggestions) and the library (`list_macros`, summaries without actions)
are read when the page needs them; every save returns what Rust stored. No
custom event exists: the status uses a channel like folder sync.

The shell keeps one `AutomationModel` (one subscription) for the sidebar's
indicator (a dot after Autoclicker: green running, red recording, amber
paused, with the words for screen readers) and the page, which is its own
JavaScript chunk loaded right after startup.

## Storage and the macro format

`automation.sqlite3` in the app's local data folder (`storage.rs`
conventions: STRICT tables, `PRAGMA user_version`, append-only
migrations; an unusable database is never touched, defaults run in memory
and saves report `storage`):

| Table | Holds |
| --- | --- |
| `settings` | one row: the settings document |
| `profiles` | one row per profile: id, position, document |
| `macros` | one row per macro: id, its summary (for the library), the whole document, time of the last change |

A stored document this version cannot read is skipped (a profile, a macro)
or left as it is (the settings: defaults are used until the user saves).
Macros and profiles are JSON with camelCase fields and kebab-case values;
unknown fields are refused, so a file from a newer sotyware is never
half-understood. **A macro is data only**: an action is one of the eight
kinds below, with bounded values; nothing in a file is ever run as code.

**Macro** (`formatVersion` 1):

| Field | Meaning |
| --- | --- |
| `formatVersion`, `appVersion` | format version (refused if not 1) and the sotyware that last saved it |
| `id`, `name`, `description`, `folder`, `favorite` | identity and library data (name ≤ 120, description ≤ 2,000, folder ≤ 80 characters) |
| `shortcut` | `{ "chord": "Ctrl+Alt+KeyR" \| null, "enabled": bool }` |
| `createdMs`, `modifiedMs` | times (ms since 1970) |
| `recordedOn` | `{ cursor, window, monitors }` of the recording, or `null`: the reference for relative and scaled playback |
| `target` | `{ "process", "title", "onMismatch": "warn" \| "stop" }` or `null` |
| `playback` | `repeat` (null: until stopped), `stopAfterMs`, `speed`, `startDelayMs`, `repeatDelayMs`, `jitterPercent`, `positioning` (`absolute`, `cursor`, `window`), `scaleToScreen`, `section` (`{ first, last }` or null) |
| `actions` | ≤ 100,000 actions, each `{ "delayMs": 0…86,400,000, "type": …, … }` |

Action kinds: `move {x, y}`; `button {button, state, x, y}` (`left`,
`right`, `middle`, `back`, `forward`; `down`, `up`); `click {button,
count: 1|2, x, y}` (null position: where the pointer is); `scroll {dx, dy,
x, y}` (120 per notch, one direction); `key {key, state}`; `keys {chord}`
(press and release a shortcut); `wait`; `pause`.

```json
{
  "formatVersion": 1, "appVersion": "0.1.2", "id": "macro-1a2b3c4d5e6f7a8b",
  "name": "Save the report", "description": "", "folder": "Work", "favorite": false,
  "shortcut": { "chord": "Ctrl+Alt+KeyR", "enabled": true },
  "createdMs": 1794920000000, "modifiedMs": 1794920000000,
  "recordedOn": { "cursor": { "x": 960, "y": 540 }, "window": null,
    "monitors": [{ "bounds": { "x": 0, "y": 0, "width": 1920, "height": 1080 }, "primary": true }] },
  "target": { "process": "EXCEL.EXE", "title": null, "onMismatch": "warn" },
  "playback": { "repeat": 1, "stopAfterMs": null, "speed": 1.0, "startDelayMs": 0, "repeatDelayMs": 0,
    "jitterPercent": 0, "positioning": "absolute", "scaleToScreen": true, "section": null },
  "actions": [
    { "delayMs": 0, "type": "move", "x": 640, "y": 480 },
    { "delayMs": 40, "type": "button", "button": "left", "state": "down", "x": 640, "y": 480 },
    { "delayMs": 85, "type": "button", "button": "left", "state": "up", "x": 640, "y": 480 },
    { "delayMs": 500, "type": "keys", "chord": "Ctrl+KeyS" },
    { "delayMs": 1000, "type": "wait" }
  ]
}
```

**Profile**: `id`, `name`, `clicker` (the autoclicker's configuration:
`button`, `clickType`, `intervalMs`, `rateUnit`, `random`, `maxClicks`,
`maxDurationMs`, `startDelayMs`, `batch`, `target`, `holdMs`,
`doubleGapMs`, `advanced`), `hotkeys` (action → binding; not the
emergency stop), `macroId`, `playback` (null: the macro's own),
`createdMs`, `modifiedMs`.

**Export file** (`*.sotyware.json`, written through a save dialog Rust
opens, via a `.part` file renamed into place): `{ "format":
"sotyware-automation", "formatVersion": 1, "appVersion", "exportedAt",
"macros": [...], "profiles": [...] }`. **Import** (open dialog opened by
Rust; at most 64 MB): the envelope must be sotyware's and version 1; each
macro and profile is checked on its own and reported if refused; imported
ones get new ids and free names (`Name (2)`), imported macro shortcuts are
switched off (they may clash), and a profile's macro follows its imported
copy (or an existing macro with the same id).

## Commands

29 commands, each with one `allow-<command>` permission; arguments are ids
and documents, never paths (the dialogs are Rust's):
`automation_status`, `watch_automation`, `unwatch_automation`,
`automation_config`; `start_clicker`, `play_macro`, `start_recording`,
`set_automation_paused`, `stop_automation`; `save_recording`,
`discard_recording`; `list_macros`, `get_macro`, `create_macro`,
`save_macro`, `update_macro_info`, `duplicate_macro`, `delete_macro`;
`create_profile`, `save_profile`, `delete_profile`, `activate_profile`,
`set_default_profile`; `save_automation_settings`; `export_automation`,
`import_automation`; `capture_cursor_position`, `capture_active_window`,
`screen_layout`. Errors are `{ kind, message }` with the kinds
`invalid-input`, `not-found`, `busy`, `feature-off`, `unavailable`,
`storage`, `confirm`, `input`, `file`, `conflict`.

## Testing

- **Engines, headless** (`clicker.rs`, `recorder.rs`, `player.rs`
  tests): on the mock backend and `FakeClock`, whose time moves only when
  an engine waits and which pauses, resumes or stops a session at given
  times; every timing is exact: interval and count accuracy (10,000 clicks
  at 1 ms, fractional intervals without drift), endless runs and stops,
  pause and resume, random ranges, double clicks, batches, areas, the
  fail-safe, sotyware's window, time limits, the recorder's timing,
  sampling, shortcut stripping and filters, playback timing, speed,
  repetition, sections, jitter, placement (pointer, window, changed
  monitors), target programs, pause points, released keys and buttons.
- **Model and keys**: validation, JSON shapes (strict, no unknown action),
  the key table (unique codes, scan codes and keycodes; round trips),
  shortcut rules, conflicts, monitor mapping and corners.
- **Service** (`automation/tests.rs`): with an in-memory store and the
  mock backend: first start, registration and conflicts, shortcut presses
  through the dispatcher, sessions, confirmations, recording to a saved
  macro, playback, library and profiles, persistence across a reload,
  import/export, a broken or newer database, the feature's stop, the
  status shape the page parses.
- **Storage**: `storage/automation.rs` on real files (reopen, a newer
  schema refused).
- **X11, for real** (opt-in, `#[ignore]`): `automation::x11::tests` starts
  a private Xvfb on its abstract socket and checks sending, the pointer,
  RECORD capture, `XGrabKey` (with NumLock, a key another client holds,
  unregistering), monitors, the active window and sotyware's own window,
  then runs the real clicker (25 clicks into a test window) and a stopped
  playback that held Shift. Input goes only to that private display.
- **The app on X11, for real** (opt-in): the `autoclicker` phase of
  `tests/desktop/cloud-ui.sh` drives the test-profile build's page through
  AT-SPI on the script's private Xvfb with openbox, and sends input with
  XTest into a window of the driver that records what it receives: turning
  the feature on and off (the shortcuts grabbed and given back), a bounded
  run from the Clicker tab, F6, F7, F8 and a screen corner from another
  program, sotyware's own window, recording with F9 and F10, saving and
  playing the macro, and nothing held after any stop
  ([testing.md](../development/testing.md#cloud-storage-ui-end-to-end-test)).
- **Windows, for real** (opt-in, `#[ignore]`):
  `automation::windows::desktop` on the user's desktop, into a topmost
  window it creates (checked under each point; keys only while it has the
  keyboard, counted there by scan code): moves on the exact pixel, clicks,
  the hooks (the user's input recorded, sotyware's own not), the clicker
  skipping sotyware's own window and then 20 clicks at 5 ms, a key, a
  global shortcut and one another registration holds, playback skipping
  keys while sotyware's window is active, and a Shift held by playback
  released when it is stopped. The test window belongs to the test
  process, so it is sotyware's own to the engines; they run through a
  wrapper (`Confined`) that lets them send to that window only, or nothing
  where the skipping is checked. The pointer is put back, also after a
  failed check. Never in CI.
- **Frontend**: `src/automationFormat.test.ts` (helpers),
  `src/Autoclicker.test.ts` (the page with Tauri's IPC mocks).

Commands to run them: [testing.md](../development/testing.md#autoclicker-tests).
