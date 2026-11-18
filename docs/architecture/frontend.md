# Frontend: shell, pages and design system

Status: implemented 2026-11-15
([ADR-035](decisions.md#adr-035-the-dashboard-is-a-desktop-shell-with-a-sidebar-on-one-token-based-design-system)).
The dashboard is one Svelte 5 page (plain Vite, no router, ADR-001) in the
window `main`. It reaches Rust only through the commands listed in
[overview.md](overview.md#components-and-boundaries); this page describes
what it shows and how it is built.

## Shell

`src/App.svelte` lays out two columns:

| Part | File | What it holds |
| --- | --- | --- |
| Title bar (Windows only) | `src/TitleBar.svelte` | The page's own title bar across the top ([below](#title-bar-windows)). |
| Sidebar | `src/Sidebar.svelte` | sotyware's name and mark (`src/Logo.svelte`; in the title bar instead on Windows); the sections **Overview**, **Cloud Storage**, **Backups**, **Autoclicker** (links, `aria-current="page"` on the one shown; Autoclicker carries a dot while it runs something: green running, red recording, amber paused, with the words for screen readers, from the shell's one `AutomationModel` subscription, ADR-050); **Storage accounts**: one link per account, with a status dot, that opens its bucket, and a sync glyph coloured by its folder sync's state when it has one; below them one line summing every folder sync up (ADR-043; the shell's `FolderSyncModel` keeps one `watch_folder_syncs` subscription for the sidebar and the Sync tab); **Add storage account**; at the bottom **Account** (with a dot and tooltip for the sotyware account's sync state when there is an account, ADR-042) and **Settings**. Navigation only: every action belongs to a page. |
| Main area | `src/App.svelte` | One update notification when a version is available or installed (never on Settings › About & updates, which shows the same thing), then the page. |

Navigation is local state in `App.svelte` (`section`, `browsing`,
`settingsTab`; types in `src/navigation.ts`). On every change the new page's
`<h1>` receives focus (without a ring), so keyboard and screen-reader users
start where the page starts. The shell loads the account list once for the
sidebar; the Cloud Storage page reports every later change of the list back
(`onaccountschange`), and the sidebar opens a bucket or the add form through
the page's `browsing` (bindable) and `requestAdd` props.

**Narrow windows** (below 760 px): the sidebar becomes an overlay opened by
a button in a top bar (in the title bar on Windows), closed by Escape, a
click outside or a choice. The window's minimum size is 640×480
(`tauri.conf.json`); where it opens and how big is decided in Rust
([lifecycle.md](lifecycle.md#placement-and-chrome)).

### Title bar (Windows)

Since 2026-11-15 ([ADR-040](decisions.md#adr-040-on-windows-the-dashboard-draws-its-own-title-bar-the-window-opens-at-half-the-primary-work-area-and-remembers-its-place)).
On Windows the window has no native decorations, and `App.svelte` draws
`TitleBar.svelte` as the shell's first row (40 px, `--titlebar-h`) when
`getCurrentWindow().isDecorated()` answers `false` (a guess from the user
agent covers the first paint); anything else keeps the native title bar,
as on macOS and Linux. The bar continues the sidebar's colour and edge over
the sidebar column and the page's colour over the main column, so there is
no line between title bar and page. It holds the mark and the name (moved
from the sidebar), the page's name and the navigation toggle in narrow
windows, a refused window action as an inline alert, and the window
controls in a group named "Window": Minimize, Maximize/Restore (the icon
follows `isMaximized()`, read again after each resize) and Close, 46 px
wide caption buttons with Windows' own glyphs (Segoe Fluent Icons / MDL2
Assets), a light hover fill and the system's red for Close
(`--titlebar-*` tokens, light and dark). They are out of the tab order like
native caption buttons; the keyboard has Alt+Space, Win+Arrow and Alt+F4.
Everything that is not a control is Tauri's drag region
(`data-tauri-drag-region="deep"`): dragging uses the system's move loop
(Aero Snap included) and a double click maximizes or restores. While
another window is active the bar's text is dimmed. It stays above dialog
backdrops, as a native title bar does, so the window can be moved,
minimized or closed while a dialog is open. Close sends the same close
request as Alt+F4; in tray mode its tooltip says sotyware keeps running in
the tray.

## Pages

| Page | File | Notes |
| --- | --- | --- |
| Overview | `src/Overview.svelte` | Tray mode: the hint where sotyware keeps running ("system tray", "menu bar" on a Mac) and **Hide dashboard**; dashboard-only mode: the no-tray note. Three summary cards: storage accounts and their last check, backup jobs and how many need attention (`list_backups`), version and update state. Reads only. |
| Cloud Storage | `src/CloudStorage.svelte`, `CloudAccountForm.svelte`, `CloudRecovery.svelte` | Account cards in a grid (status badge, bucket, region, endpoint, **Open bucket**, **Check connection**, Edit, Remove with confirmation), the add/edit form, interrupted work. Every account's connection is checked once at startup by the shell (`startupChecks.svelte.ts`, ADR-046). |
| Bucket browser | `src/CloudBrowser.svelte` and its panels | Full width. Back link, title and bucket/folder line, then four tabs (ADR-041, ADR-043): **Files** (everything below; kept mounted while another tab shows, so transfers go on), **Usage & costs** (`CloudUsage.svelte`; with a sync, also what it keeps on this computer), **Snapshots & backups** (`CloudSnapshots.svelte`) and **Sync** (`CloudSync.svelte`: the bucket kept in step with a folder on this computer, [folder-sync.md](folder-sync.md#status-and-the-sync-tab)), mounted when first opened ([cloud-storage.md](cloud-storage.md#storage-usage-and-cost-estimates)); the file manager's shortcuts act only on the Files tab. Files: breadcrumb, toolbar (Up one folder, Refresh, **Upload files…**, Upload folder…, New folder…, paste, Deleted items), search row, favourites and recent items, sort and filter bar, a sticky selection bar, the file table, a context menu (also from the ⋯ button and the keyboard), transfer panels (`CloudUpload`, `CloudOperation`, `CloudBulkDownload`, the download panel), a toast for the last action, dialogs (`Modal`): new folder, rename, destination picker (`CloudFolderPicker`), review of plans, properties and preview as a side sheet (`CloudProperties`). |
| Backups | `src/Backup.svelte` | Job cards with a health badge, the job form, task progress and results, snapshots and restore. |
| Autoclicker | `src/Autoclicker.svelte` and its tabs `AutoclickerOverview.svelte`, `AutoclickerClicker.svelte`, `AutoclickerRecorder.svelte`, `AutoclickerMacros.svelte` (with `AutoclickerEditor.svelte`), `AutoclickerProfiles.svelte`, `AutoclickerSettings.svelte`; `HotkeyInput.svelte`, `PlaybackForm.svelte` | ADR-050, [automation.md](automation.md). A wider column (72rem). Header with the profile switcher; banners when the feature is off (with **Turn on**) or the system has no input automation; a sticky bar with the running session, Pause/Resume and Stop on every tab; tabs **Overview** (status with metrics and controls, the profile, the shortcuts and whether each is registered, the program in front), **Clicker** (simple or advanced, saved to the profile as it changes), **Recorder** (record, live list, save or discard, options), **Macros** (library and the editor: details, shortcut, program, playback, timeline with a preview that sends nothing, paged action table with inline editing and bulk changes), **Profiles**, **Settings** (shortcuts by key capture, safety). Its own JavaScript chunk, loaded right after startup. |
| Account | `src/Account.svelte` | The optional sotyware account (ADR-032). |
| Settings | `src/Settings.svelte` | Tabs down the left (across the top when narrow): **General** (Appearance: System, Light, Dark; how closing the window behaves; keyboard shortcuts), **Features** (`Features.svelte`), **About & updates** (`Updates.svelte`). |

## Design system

Two global stylesheets, imported once by `src/main.ts`:

- `src/styles/tokens.css`: every value as a custom property on `:root`.
  Type scale (`--text-xs` 12 px … `--text-2xl` 24 px, system font stack),
  a 4 px spacing scale (`--space-1` … `--space-12`), radii (6/8/12/16 px),
  sizes (`--control-h` 32 px, `--sidebar-w` 256 px, `--content-w` 56rem),
  layers, durations (zero with `prefers-reduced-motion`) and the colour
  roles: `--bg`, `--bg-sidebar`, `--surface`, `--surface-raised`,
  `--surface-2`/`-3`, `--border`/`-strong`, `--text`/`-muted`/`-subtle`,
  `--accent` (the blue of the icon tile) with `-hover`, `-text`, `-soft`,
  `--focus`, and `--danger`, `--success`, `--warning`, `--info`, each with
  `-soft` and `-border`. A dark palette redefines the colour roles only.
  Plain custom properties and `rgb()`: no `color-mix()` or CSS nesting, so
  older WebKitGTK and macOS WebKit render it.
- `src/styles/base.css`: element defaults (body, headings, buttons,
  inputs, selects with one chevron on every engine, checkboxes,
  `progress`, tables with sticky headers, scrollbars, focus rings) and the
  shared vocabulary:

| Class | Meaning |
| --- | --- |
| `button.primary`, `.danger`, `.primary.danger`, `.ghost`, `.small`, `.icon`, `.link` | the main action; a destructive one; its confirmation; a toolbar button; compact; icon-only (needs `aria-label`); a text action |
| `.callout` (+ `.neutral`, `.success`, `.warning`, `.danger`); `.notice`, `div/p.problem`, `div/p.error` | a message in the page's flow; the older names map to the same shapes |
| `.empty`, `.empty-state` | an empty list or folder |
| `.badge` (+ meanings), `.dot` (+ meanings) | a short status ("Connected", "Healthy") |
| `.card`, `.section-title`, `.meta-list`, `.actions`, `.toolbar`, `.spacer` | layout pieces |
| `.page`, `.page.wide`, `.page-header` (`.titles`, `.lede`) | the page column and its header |
| `.field`, `.check`, `.hint`, `.muted`, `.subtle`, `.text-danger`, `.spinner`, `kbd`, `.visually-hidden` | forms and small text |

Rules: a component's `<style>` holds layout that only it has and uses
tokens for every value; no colour literal outside `tokens.css` (the theme
swatches and the brand mark are pictures of fixed colours); meanings, not
colour names (`.danger`, never `.red`); status is never told by colour
alone (every badge has text). A `th` in `base.css` is a file table's
column header: sticky, on the page background, on one line. A component
whose `th` are row headers (`scope="row"`, as in the Estimated cost table
of `CloudUsage.svelte`) resets `position`, `background` and `white-space`,
or a long header keeps the table wider than its card.

**Accessibility tree (WebKitGTK).** Found 2026-11-15 by the Linux desktop
driver and confirmed by dumping AT-SPI: WebKitGTK leaves the text of a
plain `<span>` that is a **flex item** (a child of a `display: flex` box)
out of the accessibility tree, so a screen reader never reads it; and a
block box (`display: flex`/`block`) inside a **table cell** becomes a
second accessible object that repeats the cell's text. Therefore: put a
message's text directly in its `<p>` (the toast, the update notification,
"Last checked …", "N selected"), use a `<p>` rather than a `<span>` for text
that must be read inside a flex row (the Usage & costs tiles, whose figures
`cloud-ui.sh usage` found missing on 2026-11-16), and keep cell content
inline (the row icon is `inline-block`). The Autoclicker page's hints,
units, the Overview's shortcut labels and the editor's counts are
paragraphs too since 2026-11-18 (the Linux desktop test found the Clicker
tab's speed hint missing; "(optional)" inside a label and a table header's
hint stay `<span>`s). `src/CloudFileManager.test.ts`, `src/CloudUsage.test.ts`
and `src/Autoclicker.test.ts` pin the shapes;
after changing a layout, compare a page's `innerText` with an AT-SPI dump
(the method is in [status.md](../status/status.md#desktop-redesign-macos-and-github-releases-updates-2026-11-15)).

**Icons** are `src/Icon.svelte`: 24×24 line drawings in the current text
colour, drawn for sotyware, always `aria-hidden` next to a control that has
its own name. The brand mark in the sidebar and About is the application
icon's 32 px drawing (`src-tauri/icons/source/sotyware-32.svg`).

**Theme** (`src/theme.ts`): `system` (default), `light` or `dark`, saved
under `sotyware.theme.v1` in the webview's `localStorage`, applied before
the first paint as `data-theme` on `<html>`, and passed to the window's
`setTheme` so the title bar matches. Storage that is unavailable or throws
only means the system theme; Settings says when a choice could not be
saved. The webview, not sotyware, writes it to disk: on Linux WebKitGTK
commits it 0.5 s later, or just after sotyware quits, so a sotyware
started again within that moment can miss it
([limitations](../status/limitations.md#dashboard-adr-035)).

## Interaction

- **Keyboard:** Ctrl+1/2/3/4 (⌘ on a Mac) Overview, Cloud Storage, Backups, Autoclicker;
  Ctrl/⌘+, Settings. In a bucket: arrows and Shift+arrows, Enter, Ctrl/⌘+A,
  Ctrl/⌘+C/X/V (sotyware's own clipboard of items), Delete or ⌘⌫, F2,
  Ctrl/⌘+R or F5, Ctrl/⌘+F (search), Escape. The context menu takes focus
  and is walked with the arrows, Home, End; Escape or Tab closes it. Settings
  tabs follow the arrow keys. Shortcuts never act while a dialog is open or
  while typing in a field.
- **Dialogs** (`src/Modal.svelte`): focus moves to the first control (not
  the header's Close), Tab stays inside, Escape and the backdrop close
  unless the dialog is locked while something runs, focus returns where it
  was. `side` makes a details sheet along the right edge.
- **Motion:** short fades and slides (≤ 180 ms), none with reduced motion.
- **Drag and drop, native dialogs:** unchanged: files dropped on the window
  and every file or folder dialog are handled in Rust (ADR-020, ADR-022,
  ADR-026); the page only shows names and counts.

## Names the tests rely on

The headless tests (`src/*.test.ts`) and the desktop drivers
(`tests/desktop/`) find controls by role and accessible name. Kept as they
were: the section links (Overview, Cloud Storage, Backups, Account, now
also Settings, and Autoclicker since ADR-050: its link reads
"Autoclicker, running" (or recording, paused) while something runs), Hide dashboard, Add account, Open bucket, Check connection,
Edit, Remove, Remove account, Up one folder, Refresh, Upload files…,
Upload folder…, New folder…, Deleted items, Download, Copy to…, Move to…,
the context menu's items, `#cloud-storage-heading`, `section.browser`,
`li[data-account]`, `.bucket`, `.path`, `.count`, `.loading`, the table's
first three columns (Name, Size, Last modified). Changed on purpose:

| Before | Now |
| --- | --- |
| "← Back to accounts" | "Back to accounts" (an icon instead of the arrow) |
| folder names "📁 name" | "name" (an SVG icon, not part of the name) |
| features on the Overview | Settings › Features |
| page titles `h2` (`h3` in the browser) | `h1` |
| the sidebar adds "Add storage account" and one link per account | |

## Adding a page

Add the section to `Section` and `SECTION_TITLES` (`src/navigation.ts`), a
link in `Sidebar.svelte`, a branch in `App.svelte`, and give the page a
`.page` column with a `.page-header` whose `<h1>` names it. Use the
vocabulary above before writing styles, and add tests that mount the page
with the IPC mocks.
