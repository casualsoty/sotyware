# Gaming notes overlay

The notes overlay is a small window of notes that stays above your game,
or any other program: boss strategies, routes, loot lists, checklists,
anything you want to glance at while you play, without switching windows.
This guide explains how to use it. (How it works inside:
[notes-overlay.md](../architecture/notes-overlay.md).)

**Where it works:** Windows (the platform sotyware's releases target) and
Linux desktops; its global shortcuts work on Windows and on Linux desktops
running X11 (Xorg), not on Wayland and not yet on macOS, where you open
and hide it from sotyware's window instead. See
[What it cannot do](#what-it-cannot-do).

## Turn it on, open and close it

The overlay is off until you turn it on: **Settings › Notes overlay ›
Turn on** (or Settings › Features › Gaming notes overlay, or **Set up** on
the Overview's Notes overlay card). It appears at once, at the top right
of the screen the pointer is on.

- **Show or hide it** from any program with **Ctrl+Shift+F1**, or with
  **Show overlay** / **Hide overlay** on the Overview's card or in the
  settings. Showing it never takes the keyboard from your game.
- **✕** on its title bar (or Alt+F4) **hides** it. Your notes and settings
  stay; nothing is closed for good.
- It has no taskbar button (and on Windows it is not in Alt+Tab either):
  reach it with its shortcut or from sotyware's window.
- It keeps running when you close or minimize sotyware's own window; it
  belongs to sotyware running in the tray. (Without a tray, on some Linux
  desktops, closing sotyware's window quits sotyware, overlay included.)
- **Turn off** (in the settings) removes the window and frees its
  shortcuts. Your notes stay and come back when you turn it on again.

## The overlay

From left to right, its title bar shows the note's name and how many of
its checklist items are ticked; then **passive** (let clicks through, see
below), **opacity**, **collapse**, **settings** and **hide**.

- **Move** it by dragging its title bar, **resize** it from its edges or
  the bottom-right corner. It opens where you left it next time, also on
  another monitor (Settings › Notes overlay › Open where it was last).
- **Collapse** (or double-click the title bar) shrinks it to its title
  bar, which then shows your note's next unticked item. Expand to get the
  note back.
- **Opacity** makes its background more or less see-through (20 to 100 %);
  the text stays solid, with a shadow when the background is light, so it
  stays readable over bright and dark scenes.

## Notes

- **New note**: **+** next to the tabs (or Ctrl+N in the overlay, or
  **Ctrl+Shift+F3** from any program). Type its title, press Enter, write.
- **Switch notes** with the tabs, **Ctrl+Tab** / **Ctrl+Shift+Tab** (or
  Ctrl+Page Down / Page Up, Ctrl+1 to Ctrl+9) in the overlay, or
  **Ctrl+Shift+F5** / **Ctrl+Shift+F4** from any program.
- **Rename** a note in the title field above it (double-click a tab to get
  there).
- **Edit**: click the note (the cursor goes to the line you clicked), or
  **Edit**. **Escape** or **Done** shows it again. Everything is saved as
  you type, a moment after you stop.
- **⋯** next to the title: **Pin to the front** (pinned notes stay first,
  marked with a pin), **Duplicate**, **Move left** / **Move right** (or
  drag a tab), **Delete…** (asks first; it cannot be undone).
- **Search** (the magnifier, or Ctrl+F): finds every word in titles and
  text, ignoring case and accents. Enter opens the first match.

### Formatting

Notes are written in Markdown, which reads fine as plain text too: what
you type without it shows as you typed it, line breaks included.

| Write | To get |
| --- | --- |
| `# Title`, `## Section`, `### Smaller` | Headings |
| `- item` or `* item` | A bulleted list (indent with Tab for a list inside) |
| `1. item` | A numbered list |
| `- [ ] item` and `- [x] item` | A checklist: tick the boxes in the note |
| `**bold**`, `*italic*`, `~~struck~~`, `` `code` `` | Bold, italic, struck, code |
| `[wiki](https://example.com)` or `https://example.com` | A link (opens in your browser) |
| `> tip` | A quote |
| `---` | A line across |

While editing, **Enter** continues a list or checklist (Enter on an empty
item ends it), **Tab** / **Shift+Tab** indent list items, **Ctrl+B**,
**Ctrl+I**, **Ctrl+Shift+X** and **Ctrl+K** make bold, italic, struck
and a link, **Ctrl+Enter** ticks the checkbox of the line you are on. The
small bar under the editor does the same, plus headings and lists for the
lines you selected.

## Interactive and passive

- **Interactive** (normal): the overlay takes your clicks, and the
  keyboard when you click it.
- **Passive**: clicks go **through** the overlay to the game below, and it
  never takes the keyboard. Its edge turns dashed and its title bar says
  **Click-through** with the shortcut that switches back. Use it while
  playing.

Switch with **Ctrl+Shift+F2** from any program (to interactive it also
takes the keyboard, so you can type at once; back to passive, the game
gets the keyboard back), or with passive on the title bar. In passive mode
nothing on the overlay can be clicked, so switch back with the shortcut,
or with **Make interactive** in Settings › Notes overlay. Every start of
the overlay begins in the mode set there (interactive unless you choose
passive).

## Shortcuts

| Shortcut | Does |
| --- | --- |
| Ctrl+Shift+F1 | Show or hide the overlay |
| Ctrl+Shift+F2 | Switch between interactive and passive |
| Ctrl+Shift+F3 | New note |
| Ctrl+Shift+F4 | Previous note |
| Ctrl+Shift+F5 | Next note |

They work in every program while the overlay is on. Change them in
Settings › Notes overlay › Shortcuts: click a field and press the keys
(Backspace clears it, **Use …** puts the suggestion back), or switch one
off with **On**. The column on the right says whether each is **Ready**,
or why not: another program, or sotyware's autoclicker, may already use
that combination. Letters and digits need Ctrl, Alt or the Windows key, so
your typing keeps working.

## Settings

Settings › Notes overlay also has: the **text size**, the **mode when it
starts**, **Show the overlay when sotyware starts**, the **size** and the
**place** it opens at (changing them moves an open overlay there now): a
corner, the middle of the left or right edge (**Centre left**, **Centre
right**), the centre, or between edge and centre, **Mid centre left** and
**Mid centre right** (centred vertically, its middle a third of the way
across the screen from that side); **Open where it was last**, what a
**new note starts** with (empty or a first checklist item), which note it
**shows when it starts** (the last one, or the first pinned one), and
**Reset overlay settings…**, which puts all of these and the shortcuts
back to their defaults and forgets where the overlay was. Reset keeps your
notes.

## Where your notes are

In `notes.sqlite3` in sotyware's data folder, on this computer only
(Windows: `%LOCALAPPDATA%\com.sotyware.desktop\`; Linux:
`~/.local/share/com.sotyware.desktop/`). They are saved as you type (at
most about two seconds behind while you keep typing), when you switch
notes, hide the overlay or click elsewhere, so a crash loses at most the
last moment of typing. They are plain text: not encrypted, not synced to
your sotyware account and not part of sotyware's backups (copy the file
to keep a copy).

## What it cannot do

- **Exclusive fullscreen**: a game in exclusive (true) fullscreen owns the
  screen; no window, the overlay included, shows above it. Set the game to
  **borderless** or **windowed fullscreen** (most games call it
  "Borderless", "Fullscreen windowed" or "Windowed fullscreen").
- While the overlay is shown above a fullscreen game, Windows composes the
  screen, which can cost the game a little speed; hide it when you do not
  need it.
- A game that reads the keyboard directly, or anti-cheat software, can
  keep the shortcuts from reaching sotyware; the overlay's buttons and
  sotyware's window still work.
- **Linux**: on Wayland there are no global shortcuts, and the desktop
  decides whether the overlay stays on top. On X11 without a compositor
  its see-through background is drawn black.
- **macOS**: no global shortcuts yet, and its background is not
  see-through.
- Links open only web addresses (`http`, `https`) written in the note.

More: [limitations.md](../status/limitations.md#gaming-notes-overlay-adr-052).
