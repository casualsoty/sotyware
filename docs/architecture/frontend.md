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
| Sidebar | `src/Sidebar.svelte` | sotyware's name and mark; the sections **Overview**, **Cloud Storage**, **Backups** (links, `aria-current="page"` on the one shown); **Storage accounts**: one link per account, with a status dot, that opens its bucket; **Add storage account**; at the bottom **Account** and **Settings**. Navigation only: every action belongs to a page. |
| Main area | `src/App.svelte` | One update notification when a version is available or installed (never on Settings › About & updates, which shows the same thing), then the page. |

Navigation is local state in `App.svelte` (`section`, `browsing`,
`settingsTab`; types in `src/navigation.ts`). On every change the new page's
`<h1>` receives focus (without a ring), so keyboard and screen-reader users
start where the page starts. The shell loads the account list once for the
sidebar; the Cloud Storage page reports every later change of the list back
(`onaccountschange`), and the sidebar opens a bucket or the add form through
the page's `browsing` (bindable) and `requestAdd` props.

**Narrow windows** (below 760 px): the sidebar becomes an overlay opened by
a button in a top bar, closed by Escape, a click outside or a choice. The
window's minimum size is 640×480 (`tauri.conf.json`).

## Pages

| Page | File | Notes |
| --- | --- | --- |
| Overview | `src/Overview.svelte` | Tray mode: the hint where sotyware keeps running ("system tray", "menu bar" on a Mac) and **Hide dashboard**; dashboard-only mode: the no-tray note. Three summary cards: storage accounts and their last check, backup jobs and how many need attention (`list_backups`), version and update state. Reads only. |
| Cloud Storage | `src/CloudStorage.svelte`, `CloudAccountForm.svelte`, `CloudRecovery.svelte` | Account cards in a grid (status badge, bucket, region, endpoint, **Open bucket**, **Check connection**, Edit, Remove with confirmation), the add/edit form, interrupted work. |
| Bucket browser | `src/CloudBrowser.svelte` and its panels | Full width. Back link, title and bucket/folder line, breadcrumb, toolbar (Up one folder, Refresh, **Upload files…**, Upload folder…, New folder…, paste, Deleted items), search row, favourites and recent items, sort and filter bar, a sticky selection bar, the file table, a context menu (also from the ⋯ button and the keyboard), transfer panels (`CloudUpload`, `CloudOperation`, `CloudBulkDownload`, the download panel), a toast for the last action, dialogs (`Modal`): new folder, rename, destination picker (`CloudFolderPicker`), review of plans, properties and preview as a side sheet (`CloudProperties`). |
| Backups | `src/Backup.svelte` | Job cards with a health badge, the job form, task progress and results, snapshots and restore. |
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
alone (every badge has text).

**Accessibility tree (WebKitGTK).** Found 2026-11-15 by the Linux desktop
driver and confirmed by dumping AT-SPI: WebKitGTK leaves the text of a
plain `<span>` that is a **flex item** (a child of a `display: flex` box)
out of the accessibility tree, so a screen reader never reads it; and a
block box (`display: flex`/`block`) inside a **table cell** becomes a
second accessible object that repeats the cell's text. Therefore: put a
message's text directly in its `<p>` (the toast, the update notification,
"Last checked …", "N selected"), use a `<p>` rather than a `<span>` for text
that must be read inside a flex row, and keep cell content inline (the row
icon is `inline-block`). `src/CloudFileManager.test.ts` pins the shapes;
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
saved.

## Interaction

- **Keyboard:** Ctrl+1/2/3 (⌘ on a Mac) Overview, Cloud Storage, Backups;
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
also Settings), Hide dashboard, Add account, Open bucket, Check connection,
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
