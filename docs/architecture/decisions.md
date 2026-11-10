# Architectural decisions

A short record of significant decisions and why they were made. Add an entry
whenever you make a decision a future contributor might otherwise undo by
accident: a new dependency, a new permission, a new process or IPC boundary, a
lifecycle rule. Never rewrite an accepted entry; supersede it with a new one and
mark the old one `Superseded by ADR-NNN`.

Format: **Context** (the problem), **Decision**, **Consequences** (including
costs).

Entries ADR-001 to ADR-007 were reconstructed from the README and the code
comments written with the first implementation. ADR-008 was added with this
documentation structure.

---

## ADR-001: Plain Vite + Svelte 5, not SvelteKit

Status: Accepted

- **Context:** The frontend is a single dashboard page loaded from local files by
  the Tauri webview.
- **Decision:** Use plain Vite with `@sveltejs/vite-plugin-svelte` and strict
  TypeScript. No SvelteKit.
- **Consequences:** No router, SSR or adapter configuration. If several views are
  ever needed, routing must be added deliberately (and recorded here).

## ADR-002: The dashboard window is hidden, never destroyed

Status: Accepted

- **Context:** sotyware runs in the background. Recreating the window on every
  open risks duplicate windows and loses frontend state.
- **Decision:** One window, label `main`, declared in `tauri.conf.json`. Close is
  intercepted (`prevent_close`) and turned into hide. All "open" paths call
  `show_dashboard()`, which only looks up the existing window.
- **Consequences:** The webview stays in memory while hidden. Any future code that
  opens the dashboard must go through `show_dashboard()`.

## ADR-003: Only an explicit exit code ends the process

Status: Accepted

- **Context:** Tauri requests an exit (`ExitRequested { code: None }`) when no
  windows remain, which a hidden-window background app must ignore.
- **Decision:** `prevent_exit()` for `code: None`; allow requests with
  `Some(code)`, which only `AppHandle::exit` produces. The tray "Quit" item calls
  `exit(0)`.
- **Consequences:** The tray is the only user-facing way to quit. Anything new
  that must end the process has to call `exit(code)` explicitly.

## ADR-004: Failure to create the tray is fatal

Status: Accepted

- **Context:** Without a tray, a hidden dashboard can never be restored and the
  process could only be killed.
- **Decision:** If `tray::create()` returns an error in `setup`, print it and
  `exit(1)`.
- **Consequences:** Environments without tray support fail loudly at startup. A
  tray that is created but has no host to display it is **not** detected (see
  [limitations](../status/limitations.md#linux-tray)).

## ADR-005: Use the official single-instance plugin

Status: Accepted

- **Context:** A second launch must not create a second process, tray icon or
  window.
- **Decision:** Use `tauri-plugin-single-instance` (v2), registered as the first
  plugin, desktop targets only. Its callback calls `show_dashboard()`. A
  hand-written lock file or socket was rejected.
- **Consequences:** The plugin already handles per-platform IPC (D-Bus on Linux,
  named mutex + `WM_COPYDATA` on Windows), stale locks after a crash (the OS
  releases D-Bus names and mutexes when a process dies), argument forwarding and
  the Windows foreground-permission rule. It needs no frontend permission. Costs:
  it depends on a D-Bus session bus on Linux and silently disables itself without
  one; Snap/Flatpak packaging would need extra D-Bus permissions. Details:
  [single-instance.md](single-instance.md).

## ADR-006: Minimal frontend capability

Status: Accepted

- **Context:** Tauri capabilities decide what the webview may call.
- **Decision:** The `default` capability grants window `main` only
  `core:window:allow-hide`, not even `core:default`. CSP is
  `default-src 'self'` plus inline styles for Svelte.
- **Consequences:** Every new frontend → Rust call needs an explicit permission
  added to `src-tauri/capabilities/default.json`. Grant the narrowest one.

## ADR-007: Library crate named `sotyware_lib`

Status: Accepted

- **Context:** Tauri builds the app as a library plus a thin binary. With equal
  names, output files collide on Windows (rust-lang/cargo#8519).
- **Decision:** `[lib] name = "sotyware_lib"`; `main.rs` calls
  `sotyware_lib::run()`.
- **Consequences:** Rust paths use `sotyware_lib`; the product, binary and package
  are still `sotyware`.

## ADR-008: The repository is the only source of truth for documentation

Status: Accepted

- **Context:** Coding agents and new contributors start without previous
  conversation history and cannot see external tools.
- **Decision:** All project documentation lives in this repository: `AGENTS.md`
  for agent rules, `docs/` for architecture, development and status. No Notion or
  other external documentation dependency. `CLAUDE.md` only imports `AGENTS.md`
  and `docs/CONTEXT.md`.
- **Consequences:** Documentation must be updated in the same change as the
  behaviour it describes. Rules: [AGENTS.md](../../AGENTS.md#documentation-rules).

## ADR-009: Lifecycle decisions are plain functions behind small traits

Status: Accepted

- **Context:** The lifecycle rules (close → hide, implicit exit blocked, tray
  failure → exit 1, second launch → show dashboard, tray dispatch) lived inside
  Tauri closures and could only be checked with a display, a tray host and the
  release binary. Tauri's `test` feature (`MockRuntime`) does not help: its window
  `show`/`hide` are no-ops that record nothing, `exit` is `unimplemented!()`, and
  tray creation waits for an event loop that never runs during `setup`.
- **Decision:** Each decision is a plain function in `lib.rs` (`close_action`,
  `exit_allowed`, `tray_failure_exit_code`, `on_second_launch`, `reveal`) or a
  method on `TrayAction` in `tray.rs`. Where a decision triggers an operation, it
  calls one of two small traits: `AppControl` (`show_dashboard`, `quit`,
  implemented by `AppHandle`) and `DashboardWindow` (`unminimize`, `show`,
  `set_focus`, implemented by `WebviewWindow`). Unit tests pass recording doubles.
  `serde_json` (already in the tree through `tauri`) is a dev-dependency so a test
  can check `tauri.conf.json` and the capability against `DASHBOARD_LABEL`.
- **Consequences:** `cargo test` covers the decisions without a graphical session.
  It does not cover the wiring to the real runtime (that the handlers are
  registered and Tauri delivers the events); the desktop tests still do. New
  lifecycle rules should follow the same pattern; do not add traits for code that
  has no decision to test.
