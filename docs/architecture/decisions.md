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

Status: Accepted. Applies to tray mode; in dashboard-only mode close ends the
process instead ([ADR-012](#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode)).

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

Status: Superseded by [ADR-012](#adr-012-the-tray-is-optional-tray-mode-and-dashboard-only-mode).
The tray is now optional: a Linux environment without a reachable tray host
starts in dashboard-only mode. Every other tray failure is still fatal (exit 1),
as decided here.

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

Status: Superseded by [ADR-013](#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode).

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

## ADR-010: Frontend tests with Vitest and happy-dom

Status: Accepted

- **Context:** The "Hide dashboard" button and its error message had no test
  outside the Windows desktop test, which needs a real desktop.
- **Decision:** Dev-dependencies `vitest` (4.x) and `happy-dom` (21.x). Vitest
  reuses `vite.config.ts` and the Svelte plugin, so there is no second build
  setup. Tauri is mocked with `@tauri-apps/api/mocks`, which ships with the
  existing runtime dependency. Versions were chosen to keep the declared Node
  range (`^20.19 || ^22.12 || >=24`): Vitest 5 drops Node 20, and jsdom 30+ needs
  Node ≥ 24.15 (the Windows 11 machine has 24.14.1). No Testing Library: Svelte's
  own `mount`/`flushSync` and DOM queries are enough for one component.
- **Consequences:** Neither package is shipped (`vite build` output is
  unchanged). Tests check what the frontend asks Tauri to do, not what the OS
  does.

## ADR-011: Linux tray uses tray-icon's ksni backend, not libayatana-appindicator

Status: Accepted

- **Context:** With libayatana-appindicator, about 1 launch in 140 to 420
  registered no tray item. The cause is a race inside that library (details in
  [limitations.md](../status/limitations.md#historical-libayatana-appindicator-registration-race)).
  No released version fixes it. Tauri 2.12.5 enables `tray-icon` 0.25's
  `libappindicator` feature and offers no option to change it. `tray-icon`
  0.25.1 also ships a pure-Rust StatusNotifierItem backend behind its `ksni`
  feature, and its `platform_impl/mod.rs` selects `ksni` whenever that feature
  is on, even with `libappindicator` also enabled.
- **Decision:** `src-tauri/Cargo.toml` declares
  `tray-icon = { version = "0.25", default-features = false, features = ["ksni"] }`
  under `[target.'cfg(target_os = "linux")'.dependencies]`. sotyware's code does
  not use the crate. The declaration only adds the feature. Cargo unifies it
  with Tauri's dependency into one `tray-icon` 0.25.1 that has both features, so
  the Linux tray runs on `ksni` 0.3.7 over `zbus` (already in the tree through
  the single-instance plugin). No Tauri upgrade, patch or second `tray-icon`.
  Rejected: delaying startup, opening the session bus early, or retrying
  registration. These narrow the race but do not remove it.
- **Consequences:**
  - New Linux-only crates: `ksni` 0.3.7, `arc-swap`, `crossbeam-queue`,
    `pastey`, `task-local`. The Windows and macOS graphs are unchanged. The
    release binary grew by about 0.5 MB (4.43 to 4.98 MB).
  - libayatana-appindicator is never loaded: the `libappindicator` crate
    still compiles, but only its unused code references the library, and it
    loaded it at runtime with `dlopen`, never at link time.
  - `ksni` registers with the StatusNotifierWatcher before `tray::create()`
    returns. Under ADR-004, no watcher (or no session bus) now means exit 1 at
    startup. There is no XEmbed fallback any more.
  - The icon is sent over D-Bus as a pixmap and is not written to
    `$XDG_RUNTIME_DIR`.
  - The tray host's left click (`Activate`) reaches sotyware as a left-click
    event and opens the dashboard, as on Windows.
  - If Tauri ever pins a `tray-icon` other than 0.25, this declaration stops
    unifying with it. Revisit it with any Tauri upgrade, and drop it once Tauri
    lets apps choose the backend.

## ADR-012: The tray is optional: tray mode and dashboard-only mode

Status: Accepted. Supersedes ADR-004.

- **Context:** Since ADR-011 the Linux tray registers with the
  StatusNotifierWatcher before `tray::create()` returns. Vanilla GNOME has no
  watcher unless the AppIndicator extension is installed, so under ADR-004
  sotyware exited 1 there, with the message on stderr only. Running without a
  tray is safe only if the dashboard can never be hidden. Otherwise the process
  becomes unreachable.
- **Decision:** `setup` tries to create the tray once and picks an
  `OperatingMode`, stored as Tauri state for the life of the process:
  - **Tray mode** (tray created): unchanged behaviour. Close hides, the Hide
    button hides, tray "Open Dashboard" restores, tray "Quit" calls `exit(0)`.
  - **Dashboard-only mode** (no tray host reachable): the dashboard starts
    visible as always, closing it calls `exit(0)`, the frontend is never granted
    the hide permission ([ADR-013](#adr-013-the-frontend-reads-the-mode-through-one-command-hiding-is-granted-only-in-tray-mode))
    and shows a note instead of the Hide button. No tray object exists, so no
    tray handler can run.
  - **Fatal** (any other tray error): exit 1, as under ADR-004.

  `tray::unavailable_reason()` decides which errors mean "no tray host". It
  matches the typed errors `tray-icon` 0.25.1's `ksni` backend (`ksni` 0.3.7,
  `zbus` 5.19) returns, never message text:

  | Error | Meaning | Mode |
  | --- | --- | --- |
  | `ksni::Error::Dbus(zbus::Error::Connection(io, _))` with `io.kind()` `NotFound` or `ConnectionRefused` | no session bus: socket missing or nobody listening. zbus produces `Connection` only in the connect step | dashboard-only |
  | `ksni::Error::Watcher(ServiceUnknown \| NameHasNoOwner)` from `RegisterStatusNotifierItem` | nobody owns `org.kde.StatusNotifierWatcher` (dbus-daemon / dbus-broker wording), also when the watcher left just before the call | dashboard-only |
  | `ksni::Error::WontShow` | the watcher reports no StatusNotifierHost | dashboard-only |
  | every other `ksni`, `zbus`, `tray-icon` or Tauri error: unparsable bus address, `PermissionDenied`, handshake failure, a watcher replying with another error (`AccessDenied`, `Failed`, `UnknownMethod`), `NoReply` (watcher left during the call, or hung), errors after registration, menu or thread errors | a defect or a broken environment | fatal, exit 1 |

  Off Linux, `unavailable_reason()` always returns `None`, so Windows keeps
  ADR-004's behaviour. `ksni` and `zbus` are declared as Linux-only
  dependencies only to name these types. Both were already in the tree through
  `tray-icon`, and `Cargo.lock` gained no crate. Rejected: retrying or waiting
  for a watcher (no delays, polling or retries, as in ADR-011),
  `ksni`'s `assume_sni_available` (it is not reachable through `tray-icon`, and it
  would hide the missing tray), an XEmbed fallback, and matching error strings.
- **Consequences:**
  - sotyware starts on desktops without a tray host (vanilla GNOME, some window
    managers, XEmbed-only panels) and says so in the dashboard. It does not run
    in the background there: closing the window quits.
  - ADR-003 still holds: the dashboard-only close goes through `exit(0)`.
    "Quit" is no longer the only user-facing exit.
  - A tray host that appears after startup is not used until the next start.
  - A watcher that vanishes *during* the registration call (`NoReply`) is
    treated as fatal: it cannot be told apart from a hung watcher. A watcher
    that accepts the item while no panel shows it is still not detected.
  - Without a session bus the single-instance guard is off (ADR-005), so two
    dashboard-only processes can run side by side there.
  - `tray_failure_exit_code` became `select_mode`; `close_action` takes the mode
    (ADR-009's pattern, new function `frontend_may_hide`).

## ADR-013: The frontend reads the mode through one command; hiding is granted only in tray mode

Status: Accepted. Supersedes ADR-006.

- **Context:** The frontend must know the operating mode (ADR-012) to show the
  right UI, and in dashboard-only mode nothing may hide the window, whether
  through the button or any other `window.hide()` call. The OS and D-Bus
  detection must stay in Rust.
- **Decision:**
  - One custom command, `operating_mode`, returns `"tray"` or
    `"dashboard-only"` from the managed `OperatingMode`. `build.rs` declares it
    in the app manifest (`tauri_build::AppManifest::commands`), so it needs the
    generated permission `allow-operating-mode`. The static capability
    `default` grants window `main` only that permission. The frontend asks once,
    on mount.
  - `core:window:allow-hide` is no longer in the static capability. In tray
    mode only, `setup` adds a runtime capability `tray-mode` (Tauri's
    `dynamic-acl`, on by default) that grants it to window `main`. In
    dashboard-only mode the permission is never granted, so a hide call is
    rejected by Tauri's ACL.
  - The frontend shows the Hide button only for `"tray"`. For
    `"dashboard-only"`, it shows a `role="status"` note. If the command fails or
    returns anything else, it shows the error and no Hide button.
  - Rejected: an event pushed from Rust (needs `core:event` permissions and
    races the page load), injecting a script into the page, and leaving hide in the
    static capability (it would rely only on the UI).
- **Consequences:** The frontend's permissions are still minimal: one app
  command, plus hide in tray mode only. CSP unchanged. The generated file
  `src-tauri/permissions/autogenerated/operating_mode.toml` is rebuilt by every
  build and is ignored by git. A new command must be added to the app manifest
  and given its own permission. The runtime capability is checked by the
  virtual-display test (scenario 12 clicks Hide in the real WebKitGTK webview),
  not yet on Windows.

## ADR-014: Built-in features are Rust modules in a fixed registry

Status: Accepted

- **Context:** sotyware needs a way to add built-in features (native Rust
  modules) without changing the whole app for each one. Each feature has to be
  identifiable, switchable from the frontend, isolated from the others' failures,
  and cleaned up at exit. Persistence (SQLite) and third-party plugins (maybe
  WASM) are planned but undecided, and must not be designed in now.
- **Decision:**
  - A feature implements the trait `Feature` (`metadata`, `start`, `stop`) and
    is listed once in `features::builtin()`. There is no static registry, no
    `inventory`/linker-section registration and no macro. One explicit list
    keeps the order deterministic and visible.
  - `FeatureRegistry` is built in `run()` and managed as Tauri state. It rejects
    duplicate or non-kebab-case IDs and empty metadata, skipping only the bad
    entry. After that its set of features is fixed: registering needs
    `&mut self`.
  - States are `registered`, `disabled`, `initializing`, `running`, `failed`
    and `stopping`. All transitions come from one pure table (`plan`). A failing
    `start` → `failed` (retry or disable). A failing `stop` → `disabled` with
    the error kept. Invalid transitions are rejected without calling the
    feature.
  - Locks are per feature, not one lock for the registry. A status lock is held
    only to check and claim a transition, and a feature lock is held while
    `start`/`stop` runs. Then a slow or failing feature cannot block or stop
    another, concurrent requests for one feature cannot both run, and the
    transient states are visible.
  - Features start in `setup` after the operating mode is chosen, in both modes,
    and not at all when the tray error is fatal. They stop in reverse order on
    `RunEvent::Exit`. Panics are not caught (`panic = "abort"`).
  - The frontend gets two commands, `list_features` and
    `set_feature_enabled { id, enabled }`, each with its own generated
    permission granted to window `main` in the static capability `default`.
    The capability now holds three permissions (this widens ADR-013's one
    command). `set_feature_enabled` runs on Tauri's blocking thread pool
    (`#[tauri::command(async)]`), so a feature's `start`/`stop` never blocks the
    event loop. There are no events: each switch returns the new state.
  - Startup asks `enabled_at_startup(&metadata)`, which today returns the
    feature's `enabled_by_default`. Persistence will answer there and save after
    a successful switch in the command layer. The registry never sees storage.
  - `serde` (with `derive`, already in the tree through Tauri, same version) is
    now a direct dependency, to derive `Serialize` for `FeatureInfo` and
    `FeatureState`.
  - Rejected: one `Mutex<FeatureRegistry>` (a slow `start` would block every
    other feature and the list, and hide `initializing`); separate
    `enable_feature`/`disable_feature` commands (two permissions for one
    switch, with no finer grant that is useful now); pushing state changes as
    events (needs `core:event` permissions; nothing changes state except the
    page itself and startup); a trait for the preference store (it would have
    no implementation yet).
- **Consequences:** a new feature without its own command touches only its file
  and one line of `builtin()`. A feature that needs a frontend call adds a
  command, a manifest entry and a permission, with an ADR. Without persistence,
  every restart returns each feature to its default. A feature cannot report a
  failure after it is `running`, and pages other than the one that switched it
  do not learn of changes. Both wait for a feature that needs them. Details:
  [features.md](features.md).
