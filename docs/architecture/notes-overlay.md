# Gaming notes overlay

Status: implemented 2026-11-23
([ADR-052](decisions.md#adr-052-the-gaming-notes-overlay-is-a-second-window-created-at-runtime-with-its-notes-in-sqlite-and-its-own-native-shortcuts-behind-an-opt-in-feature)).
What was tested where: [status.md](../status/status.md#gaming-notes-overlay-2026-11-23);
what it cannot do: [limitations.md](../status/limitations.md#gaming-notes-overlay-adr-052);
how to use it: [user guide](../user/notes-overlay.md).

A small window of notes that stays above a game or any other program. It
holds several notes (Markdown with checklists, tabs, search, pins), can be
moved, resized, made see-through, collapsed to its title bar and hidden,
switches between **interactive** (it takes clicks and, when asked, the
keyboard) and **passive** (clicks go through it to the game), and has its
own global shortcuts. Notes and settings live in Rust; the overlay's page
shows and edits them through commands.

## Pieces

| Piece | File | Role |
| --- | --- | --- |
| `NotesOverlay` | `src-tauri/src/overlay/mod.rs` | Managed state: the notes' order and titles (never their text) and the settings in memory, the window's visibility, mode, collapse and tracked placement, the shortcuts and their dispatcher thread, the status and its subscribers, the 19 commands |
| model | `overlay/model.rs` | Pure rules: `Note`, the display order (pinned first), moving, pinning, copies, next/previous, unique titles, title and text limits, which links may open, `OverlaySettings` and their validation, `OverlayAction` and the suggested chords, `OverlayState` |
| placement | `overlay/placement.rs` | Pure geometry: a new overlay at its place on the pointer's monitor (a corner, the middle of an edge, a third of the way across, the centre), a remembered one restored (through `window_state::restore_placement`), the collapsed height |
| host | `overlay/host.rs` | `TauriHost`, the `OverlayHost` of the app: creates, places, shows, hides and destroys the window, click-through and focusability, the keyboard taken and given back, collapse |
| focus | `overlay/focus.rs` (Windows) | The foreground window: given back, raised to the top of the topmost windows without activation, the window below |
| claims | `src-tauri/src/automation/claims.rs` | Which sotyware feature holds which global shortcut (the autoclicker, the overlay) |
| storage | `src-tauri/src/storage/notes.rs` | `notes.sqlite3`: the notes, the settings and state documents |
| feature | `src-tauri/src/features/notes_overlay.rs` | The built-in feature `notes-overlay` (off by default), through `overlay::LINK` |
| capability | `src-tauri/capabilities/overlay.json` | `notes-overlay`: what the overlay's window may call |
| page | `overlay.html`, `src/overlayMain.ts`, `src/NotesOverlay.svelte`, `src/NoteView.svelte` | The overlay's own page: title bar, tabs, search, the note rendered and edited, the formatting bar, menus |
| page logic | `src/noteMarkdown.ts`, `src/noteAutosave.ts`, `src/notesOverlay.ts` | The Markdown subset and the editor's keys; the autosave; the typed commands with checked responses |
| settings | `src/NotesOverlaySettings.svelte`, `src/overlayModel.svelte.ts` | Settings › Notes overlay, the Overview's card, the dashboard's live status |

## Lifecycle

- **Off until the user turns it on.** `notes-overlay` has
  `enabled_by_default: false`. Settings › Notes overlay (and Settings ›
  Features, and the Overview's card through Set up) turns it on; the choice
  is saved like every feature preference.
- **Start** (`NotesOverlay::start`): unless the system offers no global
  shortcut (`native::shortcuts_unavailable`: Wayland, macOS), opens a
  backend for the shortcuts and its dispatcher thread
  (`sotyware-overlay-hotkeys`); a failure there is shown, the overlay runs
  without shortcuts. Registers the shortcuts, sets the default mode. When
  the user turned it on (after `setup`), shows the window at once; at
  startup, `setup` calls `startup_finished`, which shows it on a worker
  thread only if `showAtStartup`.
- **Stop** (the switch): unregisters the shortcuts, closes the backend,
  destroys the window, saves the state. **Quitting**
  (`release_resources`): `exiting` first, then the feature's stop, which
  then leaves the window to the process (the exit runs on the event loop's
  thread, see [Threads](#threads)); the shortcuts are unregistered and the
  state saved either way.
- **Close is hide.** The overlay's ✕, Alt+F4 and the window manager's
  close hide it (`CloseAction::HideOverlay` in `lib.rs`, both operating
  modes); nothing is destroyed. In dashboard-only mode closing the
  dashboard still quits sotyware, overlay included.

## The window

Created at runtime by `TauriHost::create` the first time the overlay is
shown (`tauri.conf.json` declares only the dashboard), label `overlay`,
page `overlay.html` (a second Vite entry; about 37 KB of its own script
plus the runtime it shares with the dashboard). Hidden, it stays (showing
is instant, the page keeps its state); it is destroyed only when the
feature stops, so with the feature off there is no overlay webview.

- **Look**: no decorations; transparent window and webview background
  (`transparent(true)`, colour `(0, 0, 0, 0)`, no shadow), so the page's
  rounded panel at the chosen opacity is all there is; always dark.
- **Stacking**: `always_on_top` (Windows `WS_EX_TOPMOST`, X11
  `_NET_WM_STATE_ABOVE`); every show on Windows also puts it at the top of
  the topmost windows (`HWND_TOPMOST` in the show's `SetWindowPos`), above
  a game that made itself topmost after it.
- **No taskbar button**: on Windows a tool window (`WS_EX_TOOLWINDOW`,
  without the `WS_EX_APPWINDOW` tao gives every window without an owner,
  so also not in Alt+Tab; `skip_taskbar` alone removed the button only at
  creation, while the window was still hidden, found by the Windows
  desktop test); on X11 `skip_taskbar`. Not maximizable or minimizable,
  no file drop handler (HTML drag and drop reorders the tabs; Windows'
  native drop handling would swallow it).
- **On Windows sotyware drives the window itself** after creating it
  (`overlay/focus.rs`): shown and hidden with `SetWindowPos`
  (`SWP_SHOWWINDOW | SWP_NOACTIVATE`, `SWP_HIDEWINDOW`), its extended
  style set for each mode, and none of tao's window flags changed again.
  tao shows a window with `ShowWindow(SW_SHOW)`, which activates it even
  with `WS_EX_NOACTIVATE` (a probe on a private desktop, 2026-11-23; tao's
  "don't focus" marker lasts only until the window exists), and rewrites
  the extended style from its flags whenever one changes. tao's own idea
  of the window's visibility then stays "hidden", which only its
  `set_focus` reads; `is_visible` asks Windows.
- **Moving and resizing**: the page's title bar calls `startDragging` on a
  mouse press (a double click collapses instead); its bottom-right grip
  calls `startResizeDragging("SouthEast")`; on Windows the native resize
  border of an undecorated window works too. Limits 240 × 160 to 1600 ×
  1600 logical pixels.
- **Collapsed**: only the title bar (38 logical pixels) with the note's
  name and its next unchecked item. The height is pinned by the size
  limits (minimum and maximum 38), not by making the window non-resizable:
  GTK sizes a non-resizable window to its content's request (the webview
  asks for 200 px, found by the Linux desktop test). The expanded height
  is remembered.

### Interactive and passive

| | Interactive | Passive |
| --- | --- | --- |
| Mouse | Clicks, scrolling, dragging, resizing reach the overlay | Windows `WS_EX_TRANSPARENT | WS_EX_LAYERED` (set by sotyware, as tao's `set_ignore_cursor_events` would), X11 `set_ignore_cursor_events(true)`, an input shape without the window; clicks reach the window below |
| Keyboard | Taken by a click, or when switched to by shortcut | Windows `WS_EX_NOACTIVATE`, X11 `set_focusable(false)` (no input focus); if the overlay had the keyboard it gives it back |
| Page | Tabs, search, title, Edit, menus, formatting bar | The note only; checkboxes disabled; no element keeps the focus |
| Look | Solid edge | Dashed accent edge and a "Click-through · <keys>" badge |

- **Showing never takes the keyboard.** On Windows the show's
  `SetWindowPos` carries `SWP_NOACTIVATE` (above); elsewhere the window is
  made unfocusable for the show and focusable again when interactive.
  Should the system activate it anyway, the keyboard goes straight back
  to the window that had it (logged). Verified with real input on X11 (the
  Linux desktop test) and on Windows 11 (`windows-overlay.ps1` on the
  maintainer's desktop: a window of the driver kept the keyboard through
  every show), and without input on a Windows private desktop (sotyware's
  active window stays the dashboard).
- **The keyboard is taken** only when the mode is switched to interactive
  by shortcut (or a note created by shortcut): the window that had it is
  remembered (Windows), the overlay takes the foreground (Windows
  `SetForegroundWindow`, allowed right after the shortcut reached
  sotyware; never tao's `set_focus`, which presses Alt when Windows
  refuses; wry moves the focus into the webview on `WM_SETFOCUS`;
  elsewhere `set_focus`), and the page focuses the note's Edit button (or
  the new note's title).
- **Given back** when going passive or hiding while the overlay has it:
  on Windows to the remembered window if it still exists, else the first
  visible, not minimized, not tool or click-through window below the
  overlay that is not sotyware's (when the overlay was clicked, the game is
  usually that one). On X11 the window manager decides; the page holds no
  focus in passive mode, so keys reach nothing in it.
- **Never stuck**: the mode shortcut (from any program), Settings › Notes
  overlay › Make interactive, show/hide from the Overview or the settings,
  a restart (the default mode) and turning the feature off.

### Placement and scaling

- A **new** overlay (nothing remembered, remembering off, after a reset)
  opens at the settings' size (logical pixels, so a 150 % monitor shows it
  as large as a 100 % one) at the chosen place on the work area of the
  monitor the pointer is on (else the primary), 24 logical pixels in,
  never larger than that work area: a corner, the middle of the left or
  right edge, the centre, or **mid centre left or right**: centred
  vertically, its centre on the line a third of the way across from that
  side, the two mirror images of each other (an overlay too wide to be
  centred there stays 24 pixels in from its side).
- A **remembered** one opens where it was, through the dashboard's rules
  (`window_state::restore_placement`, ADR-040): only where it overlaps a
  connected monitor's work area, moved fully inside it, resized for a
  monitor whose scaling changed, at least the minimum size.
- **Shown again** after hiding, it is put back where it was before it is
  shown and again after (a window manager may place a window it maps
  again anew: openbox put it at 0, 0, found by the Linux desktop test),
  or at its default place when that monitor is gone.
- **Right after each show on X11** (1.5 s, from before the show, which
  the event loop may report while `show` runs, and again after it) a
  position elsewhere than asked is never remembered, and the first one is
  answered with a move back: besides the window manager's placement, the
  first position tao reports after a window is mapped can be one it never
  had (0, 0, read from the frame before openbox placed it; the window was
  at its corner and no later event said so; found by the Linux desktop
  test, where remembering it made the next show put the overlay at 0, 0).
  Each report comes twice (moved, resized). A drag begun within that time
  may be moved back once and is not remembered.
- **Tracked** from the window's own `Moved`/`Resized` events (outer
  position, inner size, scale; only while visible), in memory, by the
  event loop's handler that takes only the tracking lock; saved in the
  state document when the overlay hides or loses the focus and at exit.
  Changing the default size or place in the settings moves an open
  overlay there at once. Monitors added, removed or rescaled while it is
  shown: the system moves the window (Windows rescales it for the new
  monitor's DPI, tao handles `WM_DPICHANGED`); the next show checks again.

## Notes

- **Data**: a note is an id, a title (at most 120 characters, control
  characters become spaces), Markdown text (at most 50,000 characters,
  Windows line ends become `\n`, no NUL), pinned or not, a position, its
  creation and change times and a **revision** bumped by every save. At
  most 500 notes.
- **Order**: pinned notes first, each group by position; moving keeps a
  note in its group, pinning puts it at the end of the pinned ones,
  unpinning at the front of the others, a copy right behind its original.
  Every change renumbers positions to the display order and stores only
  what changed, in one transaction.
- **Saving**: `save_note(id, title, body, revision)` refuses another
  revision (`conflict`) and saves nothing. The page is the notes' only
  editor: after a conflict it reads Rust's revision and saves the user's
  text again. The page's autosave (`noteAutosave.ts`) saves half a second
  after typing stops and at least every two seconds while typing goes on,
  one save at a time per note (typing during a save is saved after it), at
  once after a checkbox tick, a note switch, a copy, hiding, a mode
  change, collapsing, opening the settings, the window losing the focus or
  the page being hidden or unloaded. A failed save keeps the text pending
  ("Not saved" and the reason), tried again after 5 s, or at the next
  change when the text itself was refused.
- **What Rust does on its own** (a shortcut) reaches the page through the
  status: `notesSeq` (read the notes again, keeping text not saved yet),
  `activeSeq` (show the note Rust chose), `focusSeq`/`focusTarget` (focus
  the title or the note). The page's own choices (`select_note`) are
  remembered for the next start and for next/previous without bumping
  them.
- **Opening note**: the one shown last, or the first pinned one
  (`openNote`).

### Markdown

`noteMarkdown.ts` parses a subset into a tree that `NoteView.svelte`
renders with Svelte's own elements (never `{@html}`): headings `#`–`######`
in three sizes (`#hashtag` stays text), paragraphs whose line breaks are
kept, bulleted (`-`, `*`, `+`) and numbered (`1.`, `1)`) lists nested by
indentation, checklists (`- [ ]`, `- [x]`), quotes, fenced code, rules,
and inline bold, italic (`*`, `_` not inside words), struck (`~~`), code,
`[links](https://…)` (balanced parentheses allowed) and bare `https://`
addresses (trailing punctuation left out). Only `http`/`https` become
links. Text that only looks like Markdown (`5 * 3`, `snake_case`) stays
as written.

**Emojis** (`noteEmoji.ts`): a shortcode `:id:` not inside a word, whose
picture is bundled, becomes an `<img>` one line of the note high (`1lh`,
`1.45em` where the webview has no `lh`), with the profession's name as
its text for screen readers, tab names and search; `\:id:` and unknown
shortcodes stay text. The set is Guild Wars 2's nine professions and
their 36 elite specializations in the GW2 wiki's tango style (GFDL,
ADR-053; source and authors in `src/assets/emoji/gw2/README.md`), one
48 px PNG per icon at `src/assets/emoji/gw2/<id>.png`, the id being the
name in lowercase, collected by `import.meta.glob`: an icon without its
file is neither offered nor read, and a checkout without any shows no
emoji button. Vite bundles the files with the overlay's page (those under
4 KiB inlined as `data:`), so the CSP (`img-src 'self' data:`) is
unchanged and nothing is loaded from elsewhere. The editor's toolbar
opens them as rows above it, one per profession, core first
(the toolbar scrolls sideways and would clip a popover); a click types
the shortcode at the caret through the browser's own typing, so Undo
works.

- **Editing**: the note shows rendered; a click opens a plain-text editor
  with the cursor at the end of the line clicked (Edit, or Enter in the
  title, at the end of the note); Escape or Done closes it. Enter continues
  a list or checklist (an empty item ends it), Tab and Shift+Tab indent
  list lines (elsewhere Tab moves the focus), Ctrl+B, Ctrl+I,
  Ctrl+Shift+X and Ctrl+K make bold, italic, struck and a link,
  Ctrl+Enter ticks the line's checkbox; the formatting bar does the same
  and headings, lists and checklists for the selected lines. Changes go
  through the browser's own `insertText` where it can, so Undo works.
- **Checkboxes** tick in the rendered note (the line's `[ ]` becomes
  `[x]`), saved at once.
- **Links**: `open_note_link(id, url)` opens the address in the browser
  only if note `id`'s stored text holds it and it is `http`/`https` without
  spaces (ADR-049's rule that the page never chooses an address);
  test-profile builds record it in the profile instead of starting a
  browser.
- **Search** (Ctrl+F): every word in the title or the text as read
  (without marks or checkboxes), ignoring case and accents, titles first,
  with the text around the match.

## Shortcuts

| Action | Suggested | What it does |
| --- | --- | --- |
| Show or hide the overlay | Ctrl+Shift+F1 | Hidden → shown (no keyboard taken); shown → hidden (keyboard given back) |
| Interactive / passive | Ctrl+Shift+F2 | Switches; to interactive it takes the keyboard; hidden → shown, interactive, with the keyboard |
| New note | Ctrl+Shift+F3 | Creates one, shows the overlay, interactive, the cursor in its title |
| Previous note | Ctrl+Shift+F4 | The note before, going round; shows the overlay if hidden |
| Next note | Ctrl+Shift+F5 | The note after, going round |

- **Native**, through a second instance of the autoclicker's platform
  `Backend` (ADR-050) of which only `register_hotkey`,
  `unregister_hotkey` and `key_name` are used: Windows `RegisterHotKey`
  on its input thread (`MOD_NOREPEAT`), X11 `XGrabKey` on its own
  connection (NumLock and CapsLock ignored). None on Wayland or macOS: the
  overlay works, the settings say why there is no shortcut.
- **Rules**: the autoclicker's (`Chord::problem`): a letter, digit or
  other typing key needs Ctrl, Alt or the Windows key; F12 alone, Alt+F4,
  Alt+Tab, Ctrl+Alt+Delete, Ctrl/Alt+Esc and Win+L are refused. Two
  overlay actions may not share a chord. Each binding can be switched off
  without forgetting its keys.
- **Registration** follows the settings while the feature runs; a change
  registers only what changed; stopping unregisters everything. A chord
  another program holds is shown as "Not registered: another program uses
  this shortcut"; the others work.
- **With the autoclicker**: both features publish the chords they hold in
  `automation::claims`. A chord the other feature holds is refused with
  its name ("Not registered: the autoclicker uses this shortcut"), and
  when a feature gives a chord up, the other retries its refused ones on a
  thread of its own.
- **Presses** go from the backend's thread to the dispatcher thread, which
  runs the action; a failure becomes the status's notice ("Ctrl + Shift +
  F3: …"), shown by the overlay and logged.

## Threads

The window is driven only from worker threads (the commands are `async`,
the dispatcher has its own thread, `startup_finished` spawns one), one
operation at a time (`window_ops`), and never while the service's state
(`inner`) is locked: Tauri's window getters and `build()` wait for the
event loop. The event loop's handlers (`lib.rs`) take only the tracking
lock (`Moved`, `Resized`) or `inner` briefly (`Destroyed`); a close request
and a lost focus hand their work to a worker. At exit the window is not
touched at all. Lock order: `window_ops`, then `inner`, then the store;
`track` alone.

## Settings and state

`OverlaySettings` (JSON, `formatVersion` 1, unknown fields refused):
`opacity` (20–100 %, default 88; the panel's background, the text stays
opaque, with a text shadow below 75 %), `textSize` (small 13 px, medium
14 px, large 16 px), `width` × `height` (default 360 × 440 logical
pixels), `anchor` (`top-right` by default; `top-left`, `bottom-left`,
`bottom-right`, `center`, `center-left`, `center-right`,
`mid-center-left`, `mid-center-right`; the page lists them as they lie on
the screen), `rememberPlacement` (on), `defaultMode` (interactive),
`showAtStartup` (off), `newNote` (empty, or a first checklist item),
`openNote` (the last one shown, or the first pinned one), `shortcuts`
(one `{ chord, enabled }` per action). The overlay's opacity control
saves `opacity` too. Settings a newer sotyware stored are not read and
left alone until the user saves. A reset deletes the document (the
defaults again), forgets the place, expands an open overlay, puts it at
its default place and mode; the notes stay.

`OverlayState` (JSON, `formatVersion` 1): `collapsed`, `activeNote`, and
`placement` (`window_state::Saved`: outer position, inner size of the
expanded overlay, scale). Written only when it changed.

## Storage

`notes.sqlite3` in sotyware's data folder (`storage/notes.rs`, schema in
`PRAGMA user_version`, append-only `NOTES_MIGRATIONS`): `notes` (`id`,
`title`, `body`, `pinned`, `position`, `created_ms`, `updated_ms`,
`revision`; STRICT) and `overlay_documents` (`name` `settings` or
`state`, `data`). A note's save is one `UPDATE`; inserts with moved notes
and changes of order are one transaction. A corrupt, newer or unopenable
database is refused and left untouched: the overlay then shows that
notes cannot be saved, and every command that needs it fails with that
reason. Notes are plain text on this computer, not encrypted, not synced,
not backed up by sotyware.

## Commands

19 commands, each with one `allow-<command>` permission. The overlay's
window (capability `notes-overlay`, `capabilities/overlay.json`):
`watch_overlay`, `unwatch_overlay`, `set_overlay_visible`,
`set_overlay_mode`, `set_overlay_collapsed`, `set_overlay_opacity`,
`open_overlay_settings`, `open_note_link`, `list_notes`, `create_note`,
`save_note`, `delete_note`, `duplicate_note`, `set_note_pinned`,
`move_note`, `select_note`, plus `core:window:allow-start-dragging` and
`core:window:allow-start-resize-dragging`. The dashboard (`default`):
`watch_overlay`, `unwatch_overlay`, `overlay_config`,
`save_overlay_settings`, `reset_overlay`, `set_overlay_visible`,
`set_overlay_mode`. Errors are `{ kind, message }` with the kinds
`invalid-input`, `not-found`, `conflict`, `limit`, `feature-off`,
`storage`, `window`.

The status (`watch_overlay`, a per-call channel, sent at once and then at
most every 40 ms when something changed): `running`, `visible`, `mode`,
`collapsed`, `opacity`, `textSize`, `activeNote` with `activeSeq`,
`notesSeq`, `focusSeq` and `focusTarget`, `settingsSeq` (the overlay
asked for its settings: the dashboard opens Settings › Notes overlay),
`noteCount`, the shortcuts with their keys and registration,
`shortcutsNote`, `notice`, `storageError`.

## Testing

- **Model and placement** (`overlay/model.rs`, `placement.rs`): order,
  moves, pins, copies, stepping, titles, limits, links, settings
  validation and shapes, state parsing, every place to open at (the
  thirds the same from either side, a wide overlay kept inside the
  margins), logical sizes and scaling, negative coordinates, restore and
  the pointer's monitor.
- **Service** (`overlay/tests.rs`): an in-memory store, a fake window
  that records every call, the autoclicker's mock backend (whose sink the
  tests press): notes and their persistence, conflicts, limits, an
  unusable or newer database, the window's lifecycle (shown when turned
  on, at startup only if asked, hidden and shown again, destroyed when
  stopped, left alone at exit, recreated after the system destroyed it,
  moved back onto a monitor), interactive and passive and the keyboard,
  collapse, placement, every shortcut, registration without leaks or
  duplicates, a chord another program or the autoclicker holds, no
  shortcut system, the status shape. `the_autoclicker_and_the_overlay_name_each_other`
  runs the real autoclicker service against the overlay's claims.
- **Storage** (`storage/notes.rs`): reopen, exact text, the two
  documents, a newer or corrupt database refused and left alone, the
  schema.
- **Frontend**: `src/noteMarkdown.test.ts` (parsing, emojis, editing
  keys, search), `src/noteEmoji.test.ts` (the set, what the menu types), `src/noteAutosave.test.ts` (timing, ordering, failures, on
  fake timers), `src/NotesOverlay.test.ts` (the overlay's page),
  `src/NotesOverlaySettings.test.ts` (the settings tab), `src/App.test.ts`
  (the Overview's card, the settings request).
- **On X11, for real** (opt-in): the `overlay` phase of
  `tests/desktop/cloud-ui.sh` on its private Xvfb with openbox, through
  AT-SPI and XTest, with a window of the driver playing the game
  ([testing.md](../development/testing.md#notes-overlay-tests)).
- **On Windows** (opt-in): `tests/desktop/windows-overlay.ps1` through UI
  Automation, unattended on a private desktop (styles, places, modes,
  notes, a restart; no input), or with `-ConfirmRealInput
  pointer-and-keys` on the real desktop with real keys and clicks and a
  window of the script playing the game (the keyboard, click-through,
  the shortcuts, screen captures).

Commands to run them: [testing.md](../development/testing.md#notes-overlay-tests).
