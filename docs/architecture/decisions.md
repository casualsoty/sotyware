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

Status: Accepted. Its persistence parts (no stored choices, `enabled_at_startup`
returning the default, no store trait) are superseded by
[ADR-015](#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait).

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

## ADR-015: Feature preferences are stored in SQLite behind a narrow store trait

Status: Accepted

- **Context:** a feature the user disabled ran again after every restart,
  because each start used `enabled_by_default` (ADR-014). The choice has to
  survive restarts on Windows and Linux, offline, without making the registry
  depend on storage, without a background service, and without blocking the
  event loop. SQLite was the intended store.
- **Decision:**
  - **Engine:** SQLite through `rusqlite` 0.41 with the `bundled` feature and no
    default features. The bundled SQLite (3.54.1 in `libsqlite3-sys` 0.39.1) is
    compiled into the binary, so every platform gets the same version and no
    system library is needed (Windows has none to rely on). Cost: a C compiler at
    build time (already needed on both platforms by Tauri's toolchain), about
    1.1 MB more in the Linux release binary (6.34 → 7.45 MB), and four new
    crates in the lockfile (`rusqlite`, `libsqlite3-sys`, `fallible-iterator`,
    `fallible-streaming-iterator`; `vcpkg` is listed as an unused build
    dependency).
  - **Layering:** `features::preferences` holds the policy and the two-method
    trait `PreferenceStore { load, save }`; `storage.rs` implements it with
    SQLite. The registry knows neither: startup passes it a closure
    (`FeaturePreferences::enabled_at_startup`), and the only addition to it is a
    read-only `info(id)`. Lifecycle code calls `FeaturePreferences::load`
    (between `build()` and `run()`) and `close` (on exit). This supersedes
    ADR-014's rejection of a store trait, which was rejected only because it had
    no implementation then.
  - **Location:** `preferences.sqlite3` in Tauri's `app_local_data_dir()`:
    `$XDG_DATA_HOME/com.sotyware.desktop` (normally
    `~/.local/share/com.sotyware.desktop`) on Linux,
    `%LOCALAPPDATA%\com.sotyware.desktop` on Windows. Local, not roaming
    (`app_data_dir()` is `%APPDATA%`, which a domain profile may copy between
    machines; sotyware does no sync, and SQLite files should not roam). Tauri
    already creates and needs this folder for the webview's data (`EBWebView` on
    Windows), so storage adds no new folder and no new failure mode for it.
  - **Schema:** one table,
    `feature_preferences(feature_id TEXT PRIMARY KEY NOT NULL, enabled INTEGER
    NOT NULL CHECK (enabled IN (0, 1))) STRICT`. Only explicit user changes are
    stored: no row means `enabled_by_default`, so a later change of a default
    still reaches users who never touched that feature.
  - **Versioning and migration:** the schema version is `PRAGMA user_version`.
    `storage::MIGRATIONS` lists the steps; step `n` goes from version `n` to
    `n + 1` and released steps are never edited. On open, if the version is
    behind, all pending steps and the new version run in one `BEGIN IMMEDIATE`
    transaction (all or nothing, and two processes cannot both migrate). A
    version newer than the binary knows is refused: no downgrade, no write.
  - **Startup:** the database is opened, checked (`PRAGMA quick_check`),
    migrated and read once, after `build()` and before `run()`. That runs on the
    main thread before the event loop, and only in the primary instance (a second
    launch exits inside `build()`). `setup` then starts features from the
    in-memory result.
  - **Saving:** `set_feature_enabled` (already `async`, on Tauri's blocking pool)
    saves after a switch the registry accepted; a rejected switch saves nothing.
    The save is one `INSERT … ON CONFLICT DO UPDATE` (its own transaction). It
    runs under one lock that also reads the feature's current intent from the
    registry, so whichever save runs last stores the state after the last
    change; concurrent switches of one feature cannot leave a stale value. This
    is by construction; a test cannot force the one interleaving that would
    break a naive version. `busy_timeout` is 2 s, for the case of two processes
    when single instance is off.
  - **Failure policy:**
    - No database: created at first launch, empty, so every feature uses its
      default.
    - Unknown feature ID in the database: ignored (logged) and kept, so a feature
      that returns (for example after a downgrade) keeps its choice.
    - A feature that fails to start: its `enabled` is true (ADR-014), so
      "enabled" is saved and the next start retries. The user disables it to
      stop that.
    - The database cannot be opened, is not a database, is damaged, has a newer
      schema, or a migration fails: sotyware never repairs, replaces or deletes
      it. Features use their defaults, `list_features` returns `storageError`,
      the dashboard says so, and every save fails with that reason until a
      restart. Recovery is manual (move the file away); see
      [limitations](../status/limitations.md#feature-preferences).
    - A save fails (disk full, read-only file, quitting): the switch has already
      happened and stays for this session. The stored value stays the old one
      (the statement did not commit), so the next start uses the last saved
      choice or the default. `set_feature_enabled` resolves with `saveError` and
      the dashboard shows it. Undoing the switch was rejected: it calls the
      feature again and can fail too, leaving a third state.
    - Exit: `stop_all` stops features without saving (stopping at exit is not a
      user choice), then `close()` waits for a save in progress and refuses
      later ones. Every finished save is already durable; a process killed
      mid-save leaves a journal that SQLite rolls back on the next open.
  - **Frontend contract:** no new command or permission. `list_features` now
    returns `{ features, storageError }` and `set_feature_enabled` returns
    `{ feature, saveError }`.
  - Rejected: `tauri-plugin-sql` (sqlx and an async runtime for two queries, and
    it is designed to expose SQL to the frontend); `tauri-plugin-store` or a JSON
    file (no transactions or schema versioning, and SQLite was the stated
    direction); linking the system SQLite (not available on Windows, different
    versions across Linux distributions); WAL mode (two extra files, no benefit
    for one writer and a few writes); renaming a corrupt database aside and
    starting fresh (silently drops the user's choices and hides recurring
    corruption; manual recovery keeps the evidence); writing every feature's
    current state at startup or exit (would freeze defaults and turn exit into a
    user choice).
- **Consequences:** choices survive restarts, also in dashboard-only mode. A
  storage problem is visible in the dashboard and in the log, never a silent
  reset. A corrupt database needs the user to move it away. An unusable data
  folder as a whole was already fatal before this change, because Tauri cannot
  create the webview data folder. The `bundled` build needs a C compiler. Adding
  a column or table means appending a migration and a test for it. Details:
  [features.md](features.md#persistence).

## ADR-016: Cloud storage goes through one generic S3 adapter

Status: Accepted (proof of concept; not wired into the app)

- **Context:** sotyware is to store personal files and backups at cloud
  providers, starting with Backblaze B2 or Hetzner Object Storage
  ([evaluation](cloud-providers.md)), with several accounts at once, possibly
  several at the same provider, each with its own credentials, buckets and
  transfers. Nothing may depend on one provider, no secret may be stored in
  plain configuration, and ordinary tests and CI must never contact a provider.
- **Decision:**
  - **One adapter, providers as data.** Both providers speak S3 with SigV4, so
    `cloud::s3::S3Store` (on the official `aws-sdk-s3`) serves every provider;
    `cloud::provider::ProviderProfile` holds the differences as data (endpoint
    rule, addressing style, SDK checksum policy, limits, capabilities). No
    `ObjectStore` trait until a provider without an S3 API is needed: it would
    have one implementation.
  - **Dependencies:** `aws-sdk-s3` 1.102.0 without default features
    (`rt-tokio`, `http-1x`) and `aws-smithy-http-client` 1.1.9 with
    `rustls-ring`. Not the default `aws-lc-rs` backend, which needs CMake/NASM
    on Windows; ring builds with the C compiler already needed. No
    `aws-config` (credentials come from the account, never from `~/.aws`).
    1.102.0 is the newest `aws-sdk-s3` that supports the crate's
    `rust-version = "1.90"` (newer ones need 1.94.1); Cargo's MSRV-aware
    resolver picked it. `tokio` (fs, io-util, sync) and `sha2` 0.10 are now
    direct dependencies; both were already in the tree through Tauri. Cost: 65
    more crates in the normal dependency graph (300 → 365); the release
    binaries grew by 14.8 KB (Linux) and 13 KB (Windows) because nothing calls
    the module yet. Wiring it in will add the SDK's real size; measure then.
  - **Integrity end to end, not by provider features:** SHA-256 of the file in
    `x-amz-meta-sotyware-sha256`, verified on every download before the file is
    moved into place. The SDK's own CRC32 headers are `WhenRequired`, because
    neither provider documents support for them.
  - **Accounts:** `AccountConfig` without secrets, validated by `resolve()`;
    `Accounts` caches one `S3Store` per account, each with its own connection
    pool and transfer semaphore. Credentials are looked up by account id
    through `CredentialSource`; the proof of concept implements only
    environment variables. Account configurations are stored behind
    `AccountStore`, implemented in `storage.rs` with SQLite in a **separate
    file**, `cloud-accounts.sqlite3`, so the unwired module cannot change the
    released preferences schema. It uses the same open/check/migrate code.
  - **Tests:** an S3 server inside the test process (`s3s` 0.12 + `s3s-fs`,
    dev-dependencies only, on 127.0.0.1) runs real HTTP, SigV4 and multipart
    uploads under `cargo test` on both platforms, with no network or account.
    Real providers only in `#[ignore]` tests that need explicit variables.
    `s3s` 0.12 is the newest that supports Rust 1.90; it needs a dev-only pin
    `digest = "=0.11.0-rc.4"` to build.
  - Rejected: `rust-s3`, `object_store`, OpenDAL (not the official SDK, and
    each brings its own abstraction over what is one protocol here);
    provider-specific modules (they would duplicate the same S3 calls); storing
    secrets in SQLite, even obfuscated; adding the accounts table to
    `preferences.sqlite3` now (it would bump the schema for every user with
    nothing using it); a mock HTTP client instead of a real S3 server (it would
    not exercise signing, streaming or multipart).
- **Consequences:** a new S3-compatible provider is a new profile, without
  frontend or core changes. Provider behaviour beyond the documentation (error
  codes, checksum headers, path style on Hetzner) stays unverified until the
  opt-in integration tests run with real accounts. Wiring the module into the
  app needs its own decision (a feature, commands and permissions, keyring
  credentials). Details: [cloud-storage.md](cloud-storage.md). Credentials:
  moved from environment variables to the OS credential store by
  [ADR-017](#adr-017-cloud-account-credentials-live-only-in-the-os-credential-store).

## ADR-017: Cloud account credentials live only in the OS credential store

Status: Accepted (part of the cloud storage proof of concept; not wired into
the app)

- **Context:** ADR-016 left credentials to environment variables, which other
  processes of the same user can read and which a desktop app cannot ask a user
  to set. Accounts need their secrets kept by the operating system, per
  account (also several at one provider), changeable and removable with the
  account, with no plaintext fallback and no secret in SQLite, logs, errors or
  `Debug` output. Linux desktops may have no Secret Service at all (vanilla
  window managers, WSL2, headless sessions).
- **Decision:**
  - **One `CredentialStore` trait** (`cloud/credentials.rs`): `store`
    (insert or replace), `load`, `delete` (missing is not an error), `exists`.
    `Accounts` holds exactly one, so there is no second source to switch to.
    One real implementation, `OsCredentialStore` (`cloud/keyring.rs`); tests
    inject `MemoryCredentialStore`, which can fail each operation.
  - **Dependencies:** `keyring-core` 1.0.1 (the platform-independent API of
    the `keyring` project, rust-version 1.85) with one store crate per OS:
    `windows-native-keyring-store` 1.1.0 (Windows Credential Manager, generic
    credentials; no default features, so no `regex`) and
    `zbus-secret-service-keyring-store` 1.0.1 (Secret Service over D-Bus,
    pure Rust, feature `rt-async-io-crypto-rust`). The Linux store runs on the
    `zbus` 5.19.0 already in the tree for the tray and single instance; its
    `async-io` runtime is the one already enabled, so the zbus feature set is
    unchanged (the `rt-tokio` variant would have switched zbus's runtime for
    the tray too). `secret-service` 5.1.0 is a Linux-only direct dependency
    without features, only to name its error type. Cost: Linux graph 365 → 379
    crates (`keyring-core`, the store, `secret-service`, and the RustCrypto
    `aes`/`cbc`/`hkdf`/`num` crates for the DH session); Windows 288 → 290.
    Release binaries: Linux +1,536 bytes, Windows unchanged (nothing calls the
    module yet).
  - **Names come from the account id only:** service `com.sotyware.desktop`,
    user `cloud-account/<id>`; Windows target
    `com.sotyware.desktop/cloud-account/<id>`, persisted `Local` (this
    machine; the crate default `Enterprise` roams with domain profiles);
    Secret Service attributes `service`/`username`, label
    `sotyware cloud account <id>`, default collection. The account id is
    checked to be kebab-case before any backend call.
  - **One entry per account** holding the key id and the secret
    (`sotyware-s3-credentials-v1\n<key id>\n<secret>`), so rotating a key is
    one replace, never delete-then-write. Values with whitespace or control
    characters, or over 512 bytes per part, are refused before the backend.
  - **Errors by type, not text:** `NotFound`, `Unavailable`, `AccessDenied`,
    `Backend`, `Invalid`. On Linux the `secret-service`/`zbus` error types are
    inspected (`Unavailable`, a missing bus socket, D-Bus `ServiceUnknown`/
    `NameHasNoOwner` → unavailable; `Locked`, `Prompt`, D-Bus `AccessDenied`
    → access denied). On Windows the store crate's error type is private:
    "no logon session" is unavailable, every other Win32 code is a backend
    failure with the code shown. The `keyring-core` variants that carry stored
    bytes (`BadEncoding`, `BadDataFormat`) are never formatted.
  - **Lifecycle order** (`Accounts`, under one lock): add = credentials, then
    configuration, rolling the credentials back if the configuration save
    fails; remove = credentials, then configuration, stopping if the
    credentials cannot be deleted. Remote buckets and objects are never
    touched. Failures that leave something behind have their own error
    variants.
  - **Environment variables are test-only:** `EnvCredentials` is compiled only
    under `cfg(test)`, for the opt-in provider tests. Nothing copies them into
    the OS store.
  - Rejected: the `keyring` 4 facade crate (it picks stores by default
    features and adds macOS/Android crates); `dbus-secret-service-keyring-store`
    (links libdbus, a C library and a build dependency); the
    `linux-keyutils` store (kernel keyring, lost on reboot); an encrypted file
    as a fallback (it would be the plaintext fallback under another name, with
    a key stored next to it); two entries per account (an update could leave a
    new key id with an old secret).
- **Consequences:** without a Secret Service, Linux users cannot save
  accounts: every credential call fails with `Unavailable` and a message that
  says nothing is stored elsewhere. That is the intended behaviour, not a
  bug; a UI must show it. Secret Service calls can block on an unlock prompt,
  so callers must stay off the UI thread. Access denied on Windows is not
  distinguishable from other backend failures. Details:
  [cloud-storage.md](cloud-storage.md#credentials).

## ADR-018: The dashboard manages cloud accounts through six narrow commands

Status: Accepted (2026-11-11). Wires the account part of ADR-016 and
ADR-017 into the app; file browsing, transfers and backups stay unreachable.

- **Context:** the cloud storage backend (accounts, OS credential store, S3
  adapter) existed only as code. Users need to add, edit, check and remove
  storage accounts from the dashboard without the page ever holding a stored
  secret, without logic duplicated in Svelte, and without blocking the UI
  while the Linux Secret Service waits for an unlock prompt. A connection
  check must not call `ListBuckets`: a Backblaze key restricted to one bucket
  is refused it ("not entitled", seen against B2 on 2026-11-11) although it is
  valid.
- **Decision:**
  - **Not a built-in feature.** Account management has no lifecycle and runs
    nothing in the background, so it is not a `Feature` (ADR-014). It is
    `cloud::dashboard`: a `CloudAccounts` value managed as Tauri state
    (`Arc`), loaded in `run()` between `build()` and `run()` like the feature
    preferences, from `cloud-accounts.sqlite3` (kept separate from
    `preferences.sqlite3`, schema unchanged). Loading reads only SQLite; the
    credential store is first touched by a user action. Something that later
    runs in the background (backups, sync) becomes a feature then.
  - **Six commands**, each with its own permission in
    `capabilities/default.json` and the app manifest: `list_cloud_providers`
    (profiles and presets, pure data), `list_cloud_accounts`,
    `add_cloud_account`, `update_cloud_account`, `remove_cloud_account`,
    `check_cloud_account`. No command returns a key id or a secret, and there
    is no command to read one back.
  - **Safe shapes.** `AccountInfo` carries label, provider, region, endpoint,
    bucket, a configuration error and the last check. `CredentialInput` is
    the only type holding a secret: its `Debug` is redacted and it is turned
    into `Credentials` at once. Both inputs refuse unknown fields.
  - **Structured errors.** `CommandError { kind, message }` with kinds
    `invalid-input`, `unknown-account`, `storage`,
    `credential-store-unavailable`, `credential-store-locked`,
    `credential-store`, `partial`, `other`; check results have their own
    `status` (`connected`, `invalid-credentials`, `access-denied`,
    `bucket-unavailable`, `network`, `credential-store-unavailable`,
    `credential-store-locked`, `credentials-missing`,
    `invalid-configuration`, `error`). The page chooses its text by kind or
    status; Rust's message (whose types never format a secret) is the detail.
  - **Rust owns the rules.** Account ids are generated from the label
    (kebab-case, `-2`, `-3`, … for duplicates), so two accounts with the same
    name and provider stay separate; validation, write order and rollback stay
    in `Accounts`. An edit keeps the id, the provider (a provider change is
    refused: the keys belong to it) and settings the form does not show
    (addressing, transfer concurrency); without new credentials it never
    touches the credential store; with them it validates everything first,
    stores the new key in one replace (the old one stays if that fails), then
    saves the configuration, and reports `partial` if only the key changed.
    The dashboard gives each account exactly one bucket.
  - **Threads.** Every command that can reach SQLite or the credential store
    is `async` and runs that work in `tokio::task::spawn_blocking`; Tauri runs
    `async` commands on its tokio workers, which must not block either. The
    connection check loads the credentials on the blocking pool and sends its
    requests on the async runtime.
  - **Connection check:** `S3Store::check_bucket` sends `HeadBucket` on the
    account's bucket. A HEAD response has no body, so a 403 cannot tell a
    wrong secret from a missing permission; only then one `ListObjectsV2`
    with `max-keys=1` on the same bucket is sent, whose error code settles it.
    Never `ListBuckets`, nothing that writes. Results live in memory, per
    account, and are dropped when the account changes (a check that was
    running meanwhile does not record its result).
  - Rejected: one generic `cloud_command` (one permission for everything);
    returning the key id for display (it would need a credential-store read,
    on Linux possibly an unlock prompt, just to list accounts); checking with
    `ListBuckets` or a test upload; ids typed by the user; a modal dialog
    library; keeping check results in SQLite.
- **Consequences:** the frontend's permissions grow from three commands to
  nine (`lib.rs` test). The page can be reloaded or left without losing
  anything stored. While one call waits for the credential store (an unlock
  prompt), the other account calls queue behind the `Accounts` lock: the page
  stays responsive but shows "Loading…" or "Saving…" until the prompt is
  answered. Release binaries now link the S3 adapter (size not measured).
  Details: [cloud-storage.md](cloud-storage.md#dashboard).

## ADR-019: Read-only bucket browsing is one command with cursors kept in Rust

Status: Accepted (2026-11-11). Extends ADR-018 with browsing; uploads,
downloads and every other change to objects stay unreachable.

- **Context:** users want to see what is in each account's bucket. S3 has no
  folders, only keys; a "folder" is a prefix ending in `/`, and keys may
  contain anything (`a//b`, a trailing `/`, spaces, `..`, any Unicode). A
  bucket can hold millions of keys, so listing everything is not an option,
  and keys may be restricted to one bucket (and, at Backblaze, to a file name
  prefix), so `ListBuckets` is not either. Listings are paged with an opaque
  continuation token that is only valid for the same bucket, prefix and
  credentials.
- **Decision:**
  - **One command**, `list_cloud_folder(id, prefix, page)`, with its own
    permission (`allow-list-cloud-folder`). It lists the account's
    *configured* bucket (the page cannot name a bucket), one level
    (`ListObjectsV2`, delimiter `/`) and one page (`LIST_PAGE_SIZE` = 1000,
    S3's maximum, to keep billed requests down) per call. Never
    recursive, never `ListBuckets`, nothing that writes. Implemented in
    `cloud/browser.rs` on `S3Store::list_folder`; credentials and SQLite are
    read on the blocking pool as for the other account commands.
  - **Prefixes are taken, never built.** The root is `""`; every other
    folder prefix is one the provider returned, and the page sends it back
    unchanged; it never joins names. Breadcrumbs come from Rust, one per
    `/`. Keys are never interpreted as paths (`..` is a name). A folder's own
    marker object is hidden; anything outside the prefix is dropped; deeper
    keys from a provider that ignores the delimiter are folded into their
    immediate folder.
  - **Cursors instead of tokens.** The provider's continuation token stays
    in `CloudAccounts`; the page gets an opaque cursor id, valid for the
    same account at the same revision, the same bucket and the same prefix,
    and only until it has been used successfully (at most 256 are kept). A
    cursor that does not fit, was used, or belongs to an account that
    changed or went, and a token the provider refuses (HTTP 400), are
    `outdated-page`: the page refreshes. A provider that returns the token
    it was sent, or claims more without a token, ends the listing.
  - **Structured errors** extend `ErrorKind` with `credentials-missing`,
    `invalid-credentials`, `access-denied`, `bucket-unavailable` (Backblaze
    answers 404 also for a bucket the key may not see, so the text does not
    claim it does not exist), `network`, `timeout`, `invalid-configuration`,
    `provider` (status and code only) and `outdated-page`. The message is
    sotyware's own sentence; the provider's error text is logged, not shown.
  - **No cache.** Every navigation, refresh and "Load more" is one request;
    the page drops answers for anything but the latest navigation (request
    counter) and is recreated per account (`{#key}`), so one account's
    results never show under another. No total count is shown: S3 does not
    give one.
  - Rejected: a recursive or "list everything" call (unbounded time, memory
    and cost); returning the provider's token to the page (it would have to
    be trusted to send it back with the right account and prefix); building
    prefixes in TypeScript (`a//b` and trailing spaces break); a cache of
    pages (stale results, invalidation on every change, little gain at one
    request per folder); a start-folder setting for prefix-restricted keys
    (not needed yet; see limitations).
- **Consequences:** the frontend's permissions grow to ten commands
  (`lib.rs` test). Opening, refreshing and paging cost one `ListObjectsV2`
  each (a billed class C transaction at Backblaze). A key restricted to a file
  name prefix cannot list the bucket's root: Backblaze refuses it
  (`AccessDenied`, seen 2026-11-11), so the browser shows Access denied
  there. Details: [cloud-storage.md](cloud-storage.md#browsing-a-bucket).

## ADR-020: Single-file downloads go through a save dialog opened by Rust, one at a time, with progress on a channel

Status: Accepted (2026-11-12). Extends ADR-019 with downloading one file from
the browser; uploads, deletes, renames and every other change stay
unreachable.

- **Context:** users want to fetch a file they see in the browser. A download
  can take minutes, must not block the window, must be cancellable, must not
  leave a half-written file where the user expects a complete one, and must
  never let a remote object key decide where on the local disk something is
  written (keys may be `../../x`, `a\b`, `CON`). `S3Store` already streamed a
  download into a temporary file with end-to-end SHA-256 verification, but
  without progress or cancellation. sotyware had no custom events and no file
  dialog.
- **Decision:**
  - **Three commands**, each with its own permission:
    `download_cloud_file(id, bucket, key, onEvent)`,
    `cancel_cloud_download(transfer)` and
    `retry_cloud_download(transfer, onEvent)` (`cloud/download.rs`). The
    object is always read from the account's *configured* bucket; `bucket` is
    only compared with it, so a stale view is refused (`outdated-page`). The
    key is used exactly as the listing returned it.
  - **Rust opens the native save dialog** (`tauri-plugin-dialog`, a new
    dependency: rfd with GTK 3 on Linux, the common item dialog on Windows),
    parented to the dashboard, suggesting a file name made from the key's
    last segment with characters Windows or Linux refuse replaced. The
    frontend gets **no** `dialog:` or file system permission and never sends
    or receives a full local path (only the chosen file's name, for display).
    Both dialogs ask before returning an existing file; that file, and only
    that one, may be replaced (`Existing::Replace`, one rename); any other
    existing file, also one that appears during the download, is left alone
    (`Existing::Refuse`: a hard link that fails if the name is taken, else
    check-then-rename on file systems without hard links). A cancelled
    dialog starts nothing.
  - **Progress on a Tauri `Channel`** passed with the command, not a global
    event: messages reach only the page and the download they belong to,
    need no event permission, and arrive in order. Each message is
    `{ transfer, state, bytes, total, integrity, error }`; "active" at most
    every 250 ms (plus one at the start), then exactly one final message
    (completed, cancelled or failed), sent after the slot is free and, for
    "completed", after the verified file is in place. The page also checks
    the transfer id and ignores anything that is not its current download.
  - **One download at a time** (`CloudDownloads`, managed state): a second
    download or a second open dialog is refused with `busy`; the page
    disables every Download action meanwhile. No queue, no history: only the
    last failed download is kept, for **retry** to the same destination
    without asking again.
  - **Cancellation is cooperative and settled in one place**
    (`TransferControl`): cancelling drops the pending request or body stream
    (closing the connection) and removes the temporary file. Once the
    verified file is being moved into place, cancelling is too late
    (`finishing`) and the download completes. Cancelling is idempotent and
    never affects another transfer id. Leaving the browser cancels; quitting
    cancels and waits up to 5 s for the temporary file to be removed
    (`RunEvent::Exit`).
  - **Integrity is reported as what was checked:** `sha256-verified` only
    when the object carries the SHA-256 sotyware stores at upload and it
    matched; otherwise `size-only` (the size the provider announced), and the
    page says the content could not be verified (since 2026-11-15 a third
    level, `transfer-only`, when the provider announced no size either:
    nothing was compared, and the page says so). A mismatch fails the
    download and nothing is saved. Provider-side checksums (the SDK's
    response validation) are not claimed.
  - **Errors** extend `ErrorKind` with `busy`, `object-not-found`,
    `integrity`, `disk-full`, `local-permission`, `destination-unavailable`,
    `destination-exists` and `local-file`; messages are sotyware's own (the
    user's own destination path may appear; provider text never does).
  - Rejected: a frontend save dialog (`dialog:allow-save` would hand the
    page any path to write to through a later command); global events (any
    window could listen, and events from an old download would need the same
    filtering anyway); several downloads at once and a transfer manager (not
    asked for; one slot keeps cancellation and shutdown simple); writing
    straight to the destination (a crash or cancel would leave a partial
    file); trusting the key as a file name without the dialog.
- **Consequences:** the frontend's permissions grow to thirteen commands
  (`lib.rs` test). `tauri-plugin-dialog` and `tauri-plugin-fs` (required by
  it, unused) join the dependency tree, and the dialog plugin injects its
  small init script into the webview. A download costs one `GetObject` (a
  billed class B transaction at Backblaze) plus egress. A process killed
  during a download (crash, power loss, `kill -9`) leaves its hidden
  temporary file next to the destination. Details:
  [cloud-storage.md](cloud-storage.md#downloading-a-file).

## ADR-021: Uploads have one commit point, re-hash what they send, verify by HEAD and time out on stalls, not on duration

Status: Accepted (2026-11-12); its cancellation of a part that was
handed over completely is amended by
[ADR-025](#adr-025-a-cancelled-part-that-was-handed-over-completely-is-awaited-before-the-abort).
Backend only: `S3Store::upload` in
`src-tauri/src/cloud/s3/upload.rs`. No command, dialog or page reaches it
yet; the planned multi-file uploads will call it once per file, one after
another.
Amended by [ADR-022](#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust)
(2026-11-12): the upload batch commands call it once per file, one after
another; `TransferControl` also holds the open multipart upload id from
`CreateMultipartUpload` until it is completed or aborted
(`open_multipart()`), and `S3Store::abort_incomplete_upload` aborts an id
an earlier upload could not abort (`AbortOutcome`), bounded by
`abort_timeout`. The lifecycle below is unchanged.

- **Context:** the proof-of-concept `upload_file` (ADR-016) hashed the file,
  sent it, and afterwards compared the file's size and modification time,
  deleting the new version if they had changed. It could not be cancelled,
  reported no progress, silently overwrote an existing object, never
  checked what the provider stored, and could not tell what it left behind.
  Reading the Smithy HTTP client (`aws-smithy-http-client` 1.1.9,
  `client/timeout.rs`) showed that the 60-second read timeout wraps the
  whole hyper request future, which includes sending the body: any upload
  request taking longer than 60 s failed, however steadily it progressed.
  The connector is chosen per request from the request's timeout settings
  (`aws-smithy-runtime` 1.10.0, `orchestrator.rs`, cached per settings), so a
  per-operation override takes effect.
- **Decision:**
  - **API:** `S3Store::upload(bucket, key, path, control, progress)` →
    `Result<UploadReport, UploadError>`. `control` is the existing
    `TransferControl` (as `Arc`, because the request body itself takes the
    commit step); `progress` gets `UploadProgress { stage, bytes, total }`
    with the stages `hashing`, `sending`, `verifying`, monotonic, at most one
    update per 250 ms except a stage's first and last (an update held back
    by the throttle is sent once due, so the end of a burst is not hidden).
    `UploadError` is the `StorageError` plus `RemoteOutcome`:
    `NothingStored`, `IncompleteParts { upload_id }` or `NeedsInspection`.
    `upload_file` remains as a wrapper without cancellation or progress that
    returns only the `StorageError`. There is no overwrite parameter: its only
    allowed value would be "refuse" (see Conflicts).
  - **Lifecycle:** bucket and key checks → transfer slot (cancellable) →
    first hashing pass → `HeadObject` on the destination → send → `HeadObject`
    verification → `Ok`. `TransferControl` keeps its two transitions
    (`Running → Cancelling`, `Running → Committed`); `commit()` now also
    answers `true` when already committed, because an SDK retry after the
    commit point commits again.
  - **Commit point:** a single PUT holds back its last body chunk until the
    bytes sent plus that chunk match the first pass's SHA-256, the file's
    size and modification time are unchanged, and `commit()` succeeds; only
    then does the provider receive a complete request. A cancelled or
    changed upload therefore never becomes an object (an empty file, with
    nothing to hold back, commits before its request). A multipart upload
    commits after all parts were sent and checked, the destination was
    checked again and the file found unchanged, right before
    `CompleteMultipartUpload`. Before the commit point `cancel()` returns
    `true` and the upload stops; after it `cancel()` returns `false`
    ("finishing") and the upload runs to its end.
  - **Cancellation:** drops the request in flight. `CreateMultipartUpload` is
    never dropped in flight (an upload id could be created that nothing
    knows). A multipart upload that fails or is cancelled before its commit
    point is aborted in a spawned task (it finishes even if the caller drops
    the upload future), bounded by `ClientOptions::abort_timeout` (30 s). A
    failed or timed-out abort is reported as `IncompleteParts`: parts may
    remain, billed, at the provider. A failed `CompleteMultipartUpload` is
    followed by an abort too; if the provider no longer knows the upload, it
    may have completed, so that is `NeedsInspection`.
  - **Integrity:** the first pass computes the file's SHA-256 (it goes into
    `x-amz-meta-sotyware-sha256`, so it must be known before sending) and,
    for multipart, the SHA-256 of every part at its exact boundaries. Each
    request body hashes the bytes as the connection takes them, with a fresh
    hash for every attempt (the SDK rebuilds the body per attempt, reopening
    the file), and compares before handing over its last chunk. Any
    difference, a file that ends early, or a changed size or modification
    time before the commit point is `FileChanged`, and a multipart upload is
    aborted instead of completed. No third read of the file. The old
    after-the-fact size/mtime check and its delete are gone.
  - **Verification:** `HeadObject` on the version the provider reported (if
    it reported one) must show the size sent and the SHA-256 metadata; only
    then is the upload complete. A failure after the commit point is
    `NeedsInspection`; nothing is deleted (on a versioned bucket a delete
    only hides, and the object may be the user's only good copy).
  - **Timeouts:** requests that carry file data (`PutObject`, `UploadPart`)
    run with a per-operation config override: connect timeout kept, read
    timeout disabled, the SDK's own stalled-stream protection disabled.
    Instead the upload fails with `Timeout` once its body has handed no bytes
    to the connection, and no response has arrived, for
    `ClientOptions::stall_timeout` (60 s). Every other request, downloads
    included, keeps the client's 60 s read timeout unchanged. Proven, not
    assumed: through a throttled TCP relay a plain `PutObject` with a 1 s read
    timeout fails, the same relay with the override passes a 12 MiB PUT of
    about 7.5 s, a stalled relay times out after the stall timeout, and an
    opt-in test passes one 7 MiB PUT of 72 s with the default 60 s settings.
  - **Conflicts:** `HeadObject` before any file data is sent and, for
    multipart, again before completion; an existing object is
    `StorageError::AlreadyExists`. Uploads therefore need a key that may also
    read (HEAD). No conditional write (`If-None-Match: *`): Backblaze does
    not document it, and a provider capability flag is not needed yet.
  - Rejected: keeping the read timeout and raising it (a large file on a
    slow line can need hours; a stall would then go unnoticed for that long);
    the SDK's stalled-stream protection as the only stall check (its 20 s
    grace and 1 B/s floor are not configurable per stage here, and its error
    surfaces as a network error, not a timeout); buffering each part in
    memory to hash it (up to 64 MiB for a single PUT, and no progress while
    sending); deleting a stored object that fails verification; a
    `Replace` option (needs its own confirmation design and versioning
    semantics).
- **Consequences:** `ClientOptions` gains `stall_timeout` and
  `abort_timeout`; `StorageError` gains `AlreadyExists`; tokio's `time`
  feature and a direct `http-body` 1 dependency are added (both already in
  the tree). Every upload sends two `HeadObject` more than before (a
  Backblaze class B transaction each), and needs read permission. Remaining
  risks, all documented in
  [limitations.md](../status/limitations.md#cloud-storage): another client
  can create the object between the check and the write, which the upload
  then replaces or hides; a process killed during a multipart upload, or an
  abort that fails, leaves its parts until a bucket lifecycle rule removes
  them; progress and the stall timeout count bytes taken by the operating
  system's socket buffer, which can run ahead of the network by a few MiB;
  touching a file (new modification time, same content) during an upload
  fails it. Details: [cloud-storage.md](cloud-storage.md#uploads-and-integrity).

## ADR-022: Upload batches run one file after another in their own slot, with paths kept in Rust

Status: Accepted (2026-11-12). Backend and commands
(`src-tauri/src/cloud/upload.rs`); since the same day the bucket browser's
upload panel (`src/CloudUpload.svelte`) calls them
([cloud-storage.md](cloud-storage.md#the-upload-panel)). The page follows
the backend's states and `retryable` flags; it decides nothing itself.

- **Context:** ADR-021 made one file's upload safe (commit point,
  re-hashing, multipart abort, verification, stall timeout), but nothing
  could start one. Users pick several files at once; a batch must not turn
  one bad file into a failed batch, must not overwrite anything, must be
  cancellable as a whole, and must not hand the page local paths or the
  ability to read arbitrary files. Downloads (ADR-020) already showed the
  pattern: Rust opens the native dialog, one slot, progress on a per-call
  channel.
- **Decision:**
  - **Three commands**, one permission each:
    `upload_cloud_files(id, bucket, prefix, onEvent)` →
    `UploadSelection | null`, `cancel_cloud_upload(batch)` →
    `cancelling | finishing | not-active`, and
    `retry_cloud_upload(batch, onEvent)` → `UploadSelection`. The page sends
    the folder it shows (`prefix`, as the listing returned it; `""` or
    ending in `/`) when it asks for the dialog: that is the batch's
    destination, whatever the page does afterwards. `bucket` is only
    compared with the account's configured bucket (`outdated-page`).
  - **Rust opens the native multi-file dialog** (`tauri-plugin-dialog`'s
    `pick_files`, parented to the dashboard). The page gets no `dialog:` or
    `fs:` permission and never sends or receives a local path: it gets file
    names, object keys, sizes and states. Messages of local errors (which
    name the path) are only logged; the page gets sotyware's own sentence.
    A cancelled or empty dialog returns `null` and starts nothing.
  - **Selection:** at most 100 files; more are refused as a whole
    (`invalid-input`), nothing starts. Every file is checked before anything
    is sent, without reading its content: a regular file (links followed),
    a UTF-8 name without control characters, `prefix + name` at most 1,024
    bytes, a size `plan_upload` accepts for the provider, and no other
    selected file with the same key (all of them are skipped then). A file
    that fails is `skipped` with a `SkipReason`; the rest go ahead. With
    nothing left, no batch starts (`batch: null`) and the slot is free.
  - **One file after another, one batch at a time,** in a slot of its own
    (`CloudUploads`, managed state): a download can run meanwhile; both
    still wait for the account's transfer queue. A second batch or dialog
    is `busy`. Batch ids are `upload-<n>`.
  - **Never overwrite:** right before each file a `HeadObject` on its key;
    an existing object skips it (`destination-exists`) before it is read.
    `S3Store::upload` checks again before sending (and before completing a
    multipart upload); a conflict found there skips the file too.
  - **Errors by effect** (`disposition`): `FileChanged`, local read errors,
    `Integrity`, `TooLarge` and non-transient provider refusals fail that
    file and the batch goes on; invalid credentials, access denied, a
    missing bucket, configuration refusals, network errors, timeouts and
    provider 5xx/429 fail the file and stop the batch (`failed`, with the
    error). Credentials are loaded once per batch, on the blocking pool; a
    failure there fails the batch before any file.
  - **States:** batch `active` → `completed` (every file final),
    `cancelled` or `failed`. File `pending` → `hashing` → `sending` →
    `verifying` → `completed`, or `skipped`, `failed`, `cancelled` (also
    for files the batch never reached, `started: false`). A file's
    `remote` says what a failed or cancelled upload may have left:
    `nothing-stored`, `incomplete-parts` or `needs-inspection`.
  - **Events on a Tauri `Channel`** passed with the command (no global
    event): `{ batch, seq, kind, … }` with `kind` `file` (state changes, and
    progress as `S3Store::upload` reports it: at most one per 250 ms within
    a stage, first and last of each stage always, never backwards),
    `cleanup` (retry only) and exactly one `finished` with the summary
    (counts and every file in its final state), sent after the slot is
    free. `seq` counts from 1 per batch, so a page can drop anything stale
    or out of order; ids are never reused.
  - **Cancel** acts only on its batch id and is idempotent: it cancels the
    current file (up to its commit point; after it the answer is
    `finishing` and that file completes) and no further file starts. Leaving
    the browser cancels (the upload panel does it when it is unmounted, as
    for downloads); hiding or minimising the dashboard does not.
  - **Retry** starts a new batch with the last finished batch's files that
    failed with `nothing-stored`, were cancelled with `nothing-stored`, or
    never started, checked again like a new selection, into the same folder.
    Completed and skipped files and anything `needs-inspection` or
    `incomplete-parts` are never retried. Starting any new batch drops the
    old retry.
  - **Multipart cleanup in memory only:** an upload id whose abort failed is
    kept (account, bucket, key, id, and the store with the credentials that
    created it) while sotyware runs; a retry of that account's batch aborts
    it again first (a `cleanup` event: `removed` or `failed`), and quitting
    aborts every one; it is forgotten only when the provider confirms it is
    gone. No SQLite table; no lifecycle rule is configured.
  - **Shutdown** (`RunEvent::Exit`, after the downloads): refuses new
    batches (also one whose dialog was open), cancels the running batch,
    aborts every known incomplete upload concurrently, and waits at most
    35 s for all of it (the 30 s abort timeout plus 5 s); what is not done
    by then is logged as "parts may remain".
  - Rejected: a frontend file dialog (`dialog:allow-open` would let the page
    choose any file to send through a later command); parallel files (more
    to cancel and clean up at once, no need yet); sharing the download slot
    (an upload of hours would block every download); failing the whole
    selection for one bad file; overwrite or rename on conflict (needs its
    own confirmation design); a persistent registry of incomplete uploads
    (needs a migration and recovery UI; a bucket lifecycle rule is the
    documented fallback); retrying `needs-inspection` or `incomplete-parts`
    automatically (could duplicate an object or hide parts that still cost
    money).
- **Consequences:** the frontend's permissions grow to sixteen commands
  (`lib.rs` test); `ErrorKind` gains `file-changed`. Every file costs at
  least two `HeadObject` before sending and one after (billed class B
  transactions at Backblaze), plus the upload requests. Incomplete
  multipart uploads are forgotten when sotyware ends (also after a crash);
  only a bucket lifecycle rule removes them then. The 100-file limit and
  the one-at-a-time order make large selections slow but simple. A quit
  with an unreachable provider can take up to 35 s. Details:
  [cloud-storage.md](cloud-storage.md#uploading-files).

## ADR-023: rfd is patched locally so Windows dialog paths are read as `OsString`

Status: Accepted (2026-11-13). `[patch.crates-io]` in
`src-tauri/Cargo.toml`, the copy in `src-tauri/patches/rfd/`, the diff and
how to check it in [`src-tauri/patches/`](../../src-tauri/patches/README.md).

- **Context:** on Windows, rfd 0.17.2 (reached through
  `tauri-plugin-dialog` 2.8.4, which requires `rfd = "0.17"`) reads every
  path the common item dialog returns with
  `String::from_utf16(..).unwrap()` (`read_to_string` in
  `backend/win_cid/file_dialog/com.rs`, called by `IShellItem::get_path`).
  Windows file names are UTF-16 code units that need not be valid UTF-16:
  NTFS allows an unpaired surrogate. Choosing such a file in the upload
  dialog, or saving under such a name or into such a folder in the save
  dialog, panicked there; with the release profile's `panic = "abort"`
  sotyware ended with `0xC0000409` before it saw any path. Every Windows
  dialog result goes through that one function (`get_result`: open-file,
  folder and save dialogs; `get_results`: multi-selection). sotyware's own
  code was already lossless: `check_file` skips a name that is not valid
  Unicode as `invalid-name` (`OsStr::to_str`, never a lossy key), and
  downloads keep the destination as a `PathBuf`. On 2026-11-13 rfd 0.18.0
  and `master` still had the `unwrap`, and no upstream issue or pull
  request mentioned it.
- **Decision:** build rfd from an unchanged copy of the published 0.17.2
  crate with one function changed: `read_to_os_string` returns
  `OsString::from_wide(slice)`, so the `PathBuf` holds exactly the code
  units Windows returned (no replacement, no loss). Windows-only regression
  tests are added to the same file, one of them through a real
  `IShellItem` (`SHCreateItemFromParsingName`) of files with unpaired
  surrogates. The copy is wired in with `[patch.crates-io]`; the lockfile
  entry loses its registry source and checksum, nothing else in the graph
  changes. sotyware's validation, upload protocol and dialog use are not
  changed.
  - Rejected: `panic = "unwind"` (rfd's dialog thread would still die, the
    dialog would never answer, and the path would still be lost); waiting
    for upstream (no fix exists, and 0.18.0 needs Rust 1.92 and is not
    accepted by `tauri-plugin-dialog` 2.8.4); a git fork (an external
    repository to maintain and trust; the copy is reviewable in this one);
    replacing `tauri-plugin-dialog` with sotyware's own dialog code (much
    more platform code for one line).
- **Consequences:** sotyware maintains a 500 KB vendored crate until a fixed
  rfd is released and accepted by `tauri-plugin-dialog`; then the copy, the
  patch file and the `[patch]` entry are removed (`patches/README.md`). A
  `tauri-plugin-dialog` update that needs another rfd version makes Cargo
  ignore the patch (it warns that the patch was not used) and brings the
  defect back. Since 2026-11-13 `scripts/check-rfd-patch.mjs`
  (`npm run check:rfd-patch`, in `test:headless`, CI and before the manual
  workflows' release builds) turns that into a failure: it checks the graph
  `cargo metadata --locked` resolved (rfd once, 0.17.2, path source in
  `patches/rfd`) and the patched function, not Cargo's warning text alone.
  A small Node script without dependencies, because Node is already
  required on both platforms and the check must run unchanged on Windows
  and Linux; a Rust test was rejected (it would only run after compiling
  the very crate it checks). Cargo does
  not cap lints for path dependencies, so builds now print rfd's own nine
  upstream dead-code warnings (`oneshot.rs`, Linux); they do not fail
  `cargo clippy -- -D warnings`, which applies to sotyware only. The patch
  tests run with `cargo test --manifest-path patches/rfd/Cargo.toml --lib
  --locked`, on Windows only (the module is Windows-only); they are not
  part of `npm run test:headless`. Linux keeps its separate rfd limitation:
  the GTK 3 backend drops a non-UTF-8 path from the selection (no crash).

## ADR-024: An opt-in request recorder observes the app's own S3 requests through an SDK interceptor, compiled only into a test build

Status: Accepted (2026-11-13; proposed and approved the same day),
implemented with the deviations listed under **Implementation** at the
end. Code: `src-tauri/src/cloud/recorder.rs`, `S3Store::new`,
`scripts/check-release-binary.mjs`; behaviour in
[cloud-storage.md](cloud-storage.md#request-recorder). **Never run against
a real provider.**

- **Context:** the five upload lifecycle scenarios at Backblaze through the
  native Windows 11 dashboard passed (2026-11-13), and the harness counted
  its own requests (13, 10, 16, 10, 10). The release app's requests were
  not observed at the provider: their expected counts come only from local
  rehearsals, where the in-process s3s-fs server sees them. At a real
  provider nothing on our side records what the app sent, so "the app sent
  exactly these requests, retried this one, aborted that multipart upload"
  is unverified there. Facts checked in the code and the locked crates:
  - Every app request goes through one `aws_sdk_s3::Client`, built in one
    place, `S3Store::new` (`cloud/s3.rs`), reached from
    `Accounts::connect` (`cloud/account.rs`); one store per account, its
    own connection pool. Uploads use a per-operation config override
    (stalled-stream protection off, `cloud/s3/upload.rs`); it does not
    replace client-level interceptors. Aborts run `send()` in their own
    task under `tokio::time::timeout`; the app's stall detection and
    cancellation (`TransferControl::unless_cancelled`) drop the send
    future.
  - The locked runtime (`aws-smithy-runtime` 1.10.0,
    `aws-smithy-runtime-api` 1.11.3) has the `Intercept` trait with
    read-only hooks per operation (`read_before_execution`,
    `read_after_execution`, the latter run in `finally_op` on success and
    error) and per attempt (`read_before_attempt`, `read_before_transmit`,
    `read_after_transmit`, `read_after_attempt`). The orchestrator stores
    `RequestAttempts` (attempt number) and the operation `Metadata`
    (operation name) in the `ConfigBag`; hooks get `&mut ConfigBag`, so an
    interceptor can store its own per-operation value there. The
    operation input is a type-erased box with `downcast_ref` (available
    before serialisation). An `Err` from any hook **halts the request**.
  - Interceptors already work against this SDK: the opt-in tests add them
    with `S3Store::with_interceptor` (`#[cfg(test)]`, `integration.rs`
    `Requests`, `integration/upload.rs` `RunScope`). They only exist in
    test builds of the library, not in the release binary the Windows
    dashboard tests drive.
  - There is no logging crate (`eprintln!` only), the release profile has
    `panic = "abort"`, and `serde_json` is only a dev-dependency (it is in
    the runtime tree through Tauri).
- **Considered approaches:**
  1. **SDK interceptor added in `S3Store::new`** (chosen). Sees every
     request of every app code path (check, browse, HEAD, PUT, multipart,
     abort, download), each retry as a separate attempt of the same
     operation, the typed input before signing, the status and selected
     response headers. Read-only hooks cannot change retries, timeouts,
     order or bodies as long as they never return `Err`.
  2. Wrapping the HTTP client (`HttpClient`/`HttpConnector`): sees raw
     attempts, including connection failures, but loses the operation
     name, the attempt number and the typed input, must pass bodies
     through and sits inside the TLS stack. More code where a mistake
     changes transfers. Rejected.
  3. A `tracing` subscriber on the SDK's spans: a new logging dependency
     (an architectural decision of its own), span fields are not a stable
     API and may carry URIs or headers. Rejected.
  4. A TLS-intercepting proxy: needs a trusted test CA on Windows, changes
     connection behaviour and timing, and sees authorisation headers and
     bodies, the opposite of the privacy goal. A non-intercepting proxy
     or packet capture sees only TLS connections, not requests. Rejected.
  5. Provider-side logs: not checked during this decision (no provider
     contact). From what is known, Backblaze offers no per-request S3
     access log for a bucket; event notifications cover object changes,
     not HEAD, listings or aborts; transaction reports are daily
     aggregates. At most a coarse cross-check later, never the record.
  6. Logging in each `S3Store` method: sees operations, not SDK retries or
     actual attempts, and needs edits in every request path including the
     upload code. Rejected.
  7. A runtime switch (environment variable alone) in the shipped binary:
     ships recording code and an attack surface to every user. Rejected.
- **Decision (proposed):**
  - **Build gate.** A Cargo feature `request-recorder`, not in `default`,
    enables a new module `src-tauri/src/cloud/recorder.rs` and its
    optional dependency `serde_json` (`dep:serde_json`, already in the
    tree). Without the feature the module, its environment reads and the
    interceptor do not exist: zero code and zero cost in normal builds.
    The recorder build is made with the release profile into its own
    target directory (`cargo build --release --features request-recorder
    --target-dir target/recorder`, through the Tauri CLI's `--features`),
    so it never replaces `target/release/sotyware`. A guard (like
    ADR-023's) fails `test:headless` and CI if `default` gains the
    feature or the normal release build resolves it.
  - **Runtime gate.** In a recorder build, recording starts only if
    `SOTYWARE_REQUEST_RECORD_DIR` (absolute path to an existing directory)
    and `SOTYWARE_REQUEST_RECORD_RUN` (run id, `[a-z0-9-]{8,64}`) are both
    set, plus `SOTYWARE_REQUEST_RECORD_SCOPE` (the run's key prefix,
    required). It is read once in `run()` before `CloudAccounts::load`
    into a `OnceLock`; `S3Store::new` adds the interceptor only if it is
    set (one `OnceLock` read per store, none per request). Unset → no
    file, no thread, no interceptor. Invalid → not recording, one
    `sotyware: request recorder disabled: …` line, app unchanged.
  - **Integration point.** One interceptor per store, added last in
    `S3Store::new`; nothing in upload, download, browse or check code
    changes. Hooks used: `read_before_execution` (operation start, typed
    input, stores an operation id and a drop guard in the `ConfigBag`),
    `read_before_attempt`/`read_after_attempt` (attempt number from
    `RequestAttempts`, status, duration), `read_after_execution`
    (outcome). Every hook returns `Ok(())` on every path, including when
    the recorder itself fails; the module has no `unwrap`/`expect`/
    indexing panics (`#![deny(clippy::unwrap_used, clippy::expect_used,
    clippy::indexing_slicing)]`), since a panic aborts the app.
  - **Harness separation.** The harness runs in its own `cargo test`
    process with its own clients; only the app process installs the
    recorder, so a recorded request is app-originated by construction.
  - **Record format.** JSON Lines, one file per run,
    `<dir>/<run>.requests.jsonl`, opened with `create_new` (an existing
    file is refused, so runs never mix). First line: `start` (format
    version, run id, app version, pid, wall-clock start, scope hash).
    Then events, each with a global `seq` (atomic counter assigned at the
    hook), monotonic `t_ms` since start, account label, `op` id, and per
    kind:
    - `op-start`: operation name (`Metadata::name`), method, bucket class,
      object identity, allowlisted parameters;
    - `attempt-end`: attempt number, HTTP status or failure kind, attempt
      duration, request `content-length`, hashed `x-amz-version-id`;
    - `op-end`: outcome `ok` / `service-error` (status, error code when
      the typed error can be downcast, never the message) / `timeout` /
      `connector` / `response` / `interceptor` / `other`, attempts,
      duration;
    - `op-dropped`: the operation's future was dropped before
      `read_after_execution` (cancellation, stall detection, abort
      timeout, quit): emitted by the drop guard's `Drop`;
    - `gap`: events lost (queue full or write error), with counts;
    - `end`: written on orderly exit: events emitted, written, dropped,
      write errors, operations still open.
  - **Redaction (default and only mode).** Never read: authorisation and
    any other header except request `content-length` and response
    `x-amz-version-id`; bodies; credentials (the interceptor never sees
    the credentials provider); error messages. Bucket: `configured` or
    `other`, never the name. Account: the account id only if it equals
    `SOTYWARE_REQUEST_RECORD_ACCOUNT`, else `other`. Object key or list
    prefix inside the scope: `scope` plus the first 12 bytes (hex) of
    SHA-256(run id ‖ relative key); the harness computes the same hashes
    from the names it created. Outside the scope: `outside-scope`, nothing
    else (and a finding for the harness). Upload ids and version ids:
    truncated SHA-256(run id ‖ id), enough to tie create/parts/complete/
    abort together and match the harness's version listing, not
    reversible (high-entropy values). Query: allowlist (`partNumber`,
    `max-keys`; presence only for `uploadId`, `continuation-token`,
    `delimiter`, `versionId`); unknown operations or inputs that cannot be
    downcast are recorded with name, status and timings only.
  - **Writer.** Events are built fully in the hook and handed to one
    writer thread through a bounded channel with `try_send`; a full queue
    drops the event and counts it (a `seq` gap). The thread writes each
    line with one `write_all`, without a userspace buffer, so a crash
    loses at most what is still queued. On a write error it stops
    writing, counts and logs once; the app continues. In `RunEvent::Exit`
    the recorder is finished **last**, after the download and upload
    shutdown waits (so a quit's aborts are recorded), with a bounded wait
    of 2 s for the queue to drain, then `end`.
- **Failure behaviour:** initialisation failure → no recording, app
  unchanged, the harness sees no file and fails the run as "not recorded"
  (never as "no requests"). Write failure or full queue → `gap`/counts,
  app unchanged. Crash or kill → no `end` line; the analyser reports the
  log as partial, ignores an unterminated last line and lists operations
  without `op-end`/`op-dropped`. SDK retries → several `attempt-end`
  lines with the same `op` and rising attempt numbers, never mistaken for
  separate operations. Concurrent transfers → one writer, whole lines,
  ordered by `seq`; per-account labels and `op` ids keep a download
  beside an upload apart. Shutdown during multipart → the cancelled
  `UploadPart` appears as `op-dropped`, the abort as its own operation
  (`ok`, `service-error` or `op-dropped` at the abort timeout); operations
  still in flight when the process exits are listed as open in `end`.
  **A missing event never proves that a request was not sent:** the
  analyser treats any `gap`, missing `end`, or open operation as "count
  unknown", and a request can reach the provider after its future was
  dropped.
- **Testing requirements (before the recorder is used at a provider):**
  local only, against the in-process s3s-fs server; the recorder build
  must also pass the existing suites unchanged.
  - Disabled: without the feature, `cloud::recorder` does not compile in
    (`cfg` check) and a store has no extra interceptor; with the feature
    but no environment, no file is created during a full upload/download
    round trip.
  - Opt-in: a scripted check, browse, single PUT, multipart upload,
    download and abort produce exactly the expected `op-start`/
    `attempt-end`/`op-end` sequence, compared with the server's own
    request log.
  - Secrets: a test with sentinel access key, secret, session token,
    bucket name, out-of-scope key and body bytes greps the whole file and
    finds none of them; a test over generated keys (no new dependency) finds no key
    substring of length ≥ 4 in the output.
  - Redaction: in-scope keys hash as the harness expects; outside-scope,
    bucket `other` and account `other` are recorded as such.
  - Retries: a server that answers 500 then 200 gives one `op` with
    attempts 1 and 2; a refused request with one attempt is not retried.
  - Cancellation and multipart: cancel during `UploadPart`, stall timeout,
    abort that hangs past `abort_timeout`, quit during multipart:
    `op-dropped` where expected and the abort recorded.
  - Recorder failure: directory removed, file read-only, disk-full
    simulation (a writer that fails after n bytes), queue of size 1 under
    load: uploads, cancels and aborts complete exactly as without the
    recorder (same outcomes, same server state), `gap` counts match.
  - Concurrency: many parallel requests over two accounts: every line
    parses, `seq` is unique and complete, no interleaving.
  - Behaviour unchanged: the existing upload, download and browse tests
    run with the recorder enabled and give identical outcomes; a test
    asserts every hook returns `Ok` even when the writer is gone.
  - End to end: `windows-cloud-ui.ps1 -Rehearse` and `cloud-ui.sh upload`
    with the recorder build: the recorded app requests match the s3s-fs
    server's counts in every scenario before any provider run.
- **Consequences:** the binary tested at Backblaze with recording on is
  not byte-identical to the shipped release binary (one feature more); the
  recording run proves the app code's requests, and the earlier runs of
  the plain release binary remain the evidence that the shipped build
  behaves the same. The interceptor adds a little work per request (a few
  allocations and a channel send) only in recorder builds. The recorder
  sees only what the SDK hands to interceptors: TCP/TLS connection
  attempts, connection reuse and bytes already on the wire when a future
  is dropped are not observable; a status of `op-dropped` does not say
  whether the provider received the request. Timings are local
  (request handed to the connector to response headers), not provider
  timings. The SDK hook semantics are pinned to the locked
  `aws-smithy-runtime` version; an SDK update must rerun the recorder
  tests. Adds the module, one feature, one guard script, harness support
  (`-RecordRequests`) and an analyser in the test code; the
  documentation (`cloud-storage.md`, `testing.md`, the stack table) is
  updated with the implementation, not before.
- **Implementation (2026-11-13), deviations from the proposal above:**
  - The SDK assumptions were tested first (`cloud/s3/hook_assumptions.rs`,
    in every build): a value stored in `interceptor_state()` at
    `read_before_execution` is dropped with a dropped future (and its
    `Drop` then runs without `read_after_execution`); the client
    interceptor runs for uploads with their per-operation override; hook
    order, retries and status/attempt reads behave as assumed. One
    assumption was wrong: the operation `Metadata` is **not** in the
    `ConfigBag` at `read_before_execution` (only from the first attempt),
    so `op-start` names the operation by the typed input's type, and
    `attempt-start` carries the SDK's own name (`sdk_op`).
  - An `attempt-start` event (at `read_before_transmit`, with the request
    `content-length`) was added, so "sent" and "answered" attempts can be
    told apart when comparing with a server's log.
  - One file per run **and process**, `<run>.<pid>.requests.jsonl`: a run
    that relaunches the app (after a quit) would otherwise leave the
    relaunched process unrecorded (the file exists). Still exclusive
    creation; the harness reads all files of a run.
  - `scope` in the `start` line is hashed with the domain `scope`; account
    labels for other accounts are `other-<n>`; a list prefix that is a
    parent of the scope is `scope-ancestor`, the empty prefix `root`.
    Query parameters are never parsed: only typed input fields are read.
  - Optional dependencies besides `serde_json`: `aws-smithy-runtime-api`
    and `aws-smithy-types` (already in the tree; aws-sdk-s3 does not
    re-export `Metadata`, `RequestAttempts`, `Storable`); both are also
    dev-dependencies for the assumption tests.
  - Build gate in `build.rs`: the feature fails the build unless the
    target directory is named `recorder`. Packaging guard
    `scripts/check-release-binary.mjs`: `--manifest` (in `test:headless`
    and CI), a binary scan (the manual desktop workflows check the binary
    they test), and Tauri's `beforeBundleCommand`, which refuses when the
    build command line of a node/npm/tauri/cargo ancestor asks for the
    feature or all features, or the binary to be bundled contains the
    marker. Shown on real builds: a `--bundles deb --features
    request-recorder` build was refused before Tauri patched or bundled
    anything; a normal `--bundles deb` build passed. Backstop at runtime:
    a recorder binary that a bundler patched
    (`tauri::utils::platform::bundle_type()`) never records.
  - The in-process test server accepts a second key, which the desktop
    test helper uses for its own requests, so the comparison counts only
    requests signed with the app's key (`check_recordings` in
    `test_server.rs`).
  - `SOTYWARE_RECORDER_TEST_ALL` (test builds only) records every store of
    a test process, to run the existing suites with the interceptor
    installed.
  - Error codes come from the typed errors of the operations the app uses;
    a refusal the server sends before reading the body may reach the SDK
    as a connector error without a status, and is recorded as such.
  - An orderly exit is needed for the `end` line. On Windows in tray mode
    that is the tray Quit (real mouse); the Windows driver ends a
    recording run that way (`-RecordRequests`). A run that kills the app
    leaves an incomplete recording, which the drivers report as "count
    unknown" (seen on native Windows 11 before this was added).

## ADR-025: A cancelled part that was handed over completely is awaited before the abort

Status: Accepted (2026-11-13). Amends the cancellation rule of
[ADR-021](#adr-021-uploads-have-one-commit-point-re-hash-what-they-send-verify-by-head-and-time-out-on-stalls-not-on-duration);
code: `drive` in `src-tauri/src/cloud/s3/upload.rs`,
`ClientOptions::part_settle_timeout`.

- **Context:** ADR-021 cancels a multipart upload by dropping the
  `UploadPart` in flight, then sending `AbortMultipartUpload` on a new
  connection. Dropping closes the socket gracefully: bytes already in the
  operating system's send buffer (or in a proxy that buffers request
  bodies) still reach the provider, and the abort can arrive first. A
  provider that then stores the part keeps a part of an aborted upload,
  billed and invisible to sotyware, which reports `NothingStored`. Seen on
  2026-11-13 in a native Windows 11 quit rehearsal (the request recorder,
  ADR-024, showed the part dropped in flight and the abort answered 3 ms
  later) and reproduced on WSL2 (1 of 3 probe trials) against s3s-fs, which
  checks the upload id only when a part request starts. AWS documents the
  same: an in-progress part "might or might not succeed" after an abort.
  Backblaze and Hetzner document nothing about it; neither was asked.
- **Decision:** when a cancel arrives while a part's every byte has been
  handed to the connection (the request body finished, after its hash
  matched), the upload keeps that request running and waits for its
  answer, at most `part_settle_timeout` (default 5 s), then returns
  `Cancelled` and aborts as before. The abort then comes after the part
  and removes it with the others. A part not handed over completely is
  dropped at once, as before: its body never reaches the provider whole,
  so it cannot be stored. A single PUT is unchanged (its commit point
  holds the last chunk back, so a cancelled PUT is never complete).
  - Rejected: aborting a second time and checking with `ListParts` (the
    remedy AWS documents; extra requests on every such cancel, and s3s-fs
    answers a second abort with AccessDenied); resetting the connection
    instead of closing it (not reachable through the SDK and hyper);
    reporting `IncompleteParts` instead (honest, but leaves the part).
- **Consequences:** a cancel during the tail of a part can take up to
  5 s longer (in the tests about 1.3 s: the time the part's last bytes
  needed); 5 s plus the 30 s abort timeout stays within the 35 s quit
  bound. If the part is not answered within the bound (a very slow link,
  a provider that does not answer), the abort goes out anyway and the
  part can still land afterwards: logged
  (`sotyware: part N got no answer within …`), still reported as
  `NothingStored`. A bucket lifecycle rule that aborts incomplete
  multipart uploads stays the safety net. Tested against the in-process
  server with a link that buffers the body and forwards it slowly
  (`Pace::Buffered`): with the wait no part remains, without it
  (`part_settle_timeout` zero) the part remains every time, and the wait
  ends at its bound.


## ADR-026: File management runs as reviewed plans in Rust, with S3 semantics made explicit

Status: Accepted (2026-11-14). Extends
[ADR-018](#adr-018-the-dashboard-manages-cloud-accounts-through-six-narrow-commands)
to
[ADR-022](#adr-022-upload-batches-run-one-file-after-another-in-their-own-slot-with-paths-kept-in-rust);
code: `src-tauri/src/cloud/operations.rs` (+ `operations/names.rs`),
`cloud/manage.rs`, `cloud/bulk_download.rs`, `cloud/places.rs`,
`cloud/drop.rs`, `cloud/s3/manage.rs`, the folder upload in
`cloud/upload.rs`; frontend `src/cloudFiles.ts`, `src/fileList.ts`,
`src/CloudOperation.svelte`, `src/CloudFolderPicker.svelte`,
`src/CloudProperties.svelte`, `src/CloudBulkDownload.svelte`,
`src/CloudDeleted.svelte`, `src/Modal.svelte` and `src/CloudBrowser.svelte`.

- **Context:** the browser could list, download one file and upload
  batches. A file manager needs rename, move, copy, delete, folders,
  search, recovery and several-item transfers. S3 has none of a file
  system's operations: no folders (prefixes, sometimes a zero-byte marker),
  no rename (copy then delete), deletes that may only hide (versioned
  buckets add a delete marker), copies whose metadata handling depends on a
  header, and no search beyond key prefixes.
- **Decision:**
  - **One engine for remote changes** (`CloudOperations`, its own slot):
    the page asks for a plan (`plan_cloud_operation`: reads only), shows
    it (items, object counts, sizes, conflicts with a proposed free name,
    and what a delete means on this bucket from `GetBucketVersioning`), and
    starts it only on confirmation (`start_cloud_operation` with a
    conflict policy, skip or numbered name; never overwrite or merge). The
    plan, ETags and version ids stay in Rust; a plan expires after 15
    minutes or when replaced. Events with `seq`, one `finished` after the
    slot is free, cancel between requests, bounded shutdown, retry of what
    is left (as the upload batch).
  - **Folders are prefixes ending in `/`**, enumerated once when planning
    (at most 50,000 objects per plan); `photos/` never matches
    `photos-backup/`. Only the planned keys are touched.
  - **Copy** is `CopyObject` with `MetadataDirective::COPY` (content type,
    cache and encoding headers, user metadata including
    `sotyware-sha256`), conditional on the ETag seen when planning; a
    `HeadObject` on the destination first (an existing object is skipped,
    never replaced) and after (same size; same ETag when the source ETag is
    a plain MD5). Objects above 5 GiB are refused (no multipart copy).
  - **Move and rename** copy and verify every object of an item first and
    delete the sources only then, each after a `HeadObject` that its ETag
    is unchanged. A failed copy keeps every original; a failed delete makes
    the item `partial` ("copied, original kept"), retry deletes only the
    originals left after re-checking their copies. A move with a
    non-retryable copy failure is not retried at all (it would split).
  - **Delete** is `DeleteObject` without a version id; the provider's
    answer says per object whether a delete marker was added (restorable)
    or the object is gone, and both are reported. No version is ever
    deleted permanently, except delete markers removed by a restore.
  - **Restore** removes exactly the delete markers newer than a key's
    newest version (ids from `ListObjectVersions`), then checks the object
    is back. The deleted-items view is the provider's version listing;
    there is no trash of sotyware's own.
  - **Single requests** (`cloud/manage.rs`): create a folder (marker, only
    if nothing is at or under the prefix), properties (`HeadObject`),
    preview (ranged `GetObject`: text ≤ 256 KiB shown as text, raster
    images ≤ 10 MiB recognised by their bytes, never SVG or HTML), search
    (recursive listing, names matched in Rust or a provider-side key
    prefix; ≤ 10 pages per call, cursor in Rust), deleted items, share
    links (presigned `GetObject`, 1 h / 1 day / 7 days, signed locally).
  - **Several-item downloads** (`BulkDownloads`): Rust opens a folder
    dialog, downloads into a hidden staging folder in it through
    `S3Store::download` (size and SHA-256 as before), and moves each item
    into place once all its files are verified, under its name or the
    first free `name (n)`; key segments become safe local names on both
    systems; a failed item is removed, never left half-written.
  - **Folder uploads and drops** reuse the upload batch (same checks,
    `HeadObject` before each file, never overwrite) with keys
    `prefix + folder/…`, at most 1,000 files; symbolic links are not
    followed. Dropped paths stay in Rust (`CloudDrops`); the page hears
    counts and names through a channel and confirms.
  - **Favorites and recent items** are local, per account, in
    `cloud-places.sqlite3` (own schema, append-only migrations); never
    objects in the bucket. Removing an account forgets them.
  - **Sorting and filtering** happen in the page on what is loaded (the
    provider cannot sort or filter by size, date or type); the default
    keeps the provider's key order.
  - **CSP** gains `img-src 'self' data:` for the image previews; nothing
    else (still no remote content, no scripts but the bundle).
  - **Share links carry the access key id** (never the secret) and a
    signature; the page says that anyone with a link can download until it
    expires and that it cannot be withdrawn except by removing the key.
- **Rejected:** a trash folder of sotyware's own (moving objects to a
  hidden prefix would double storage and fight lifecycle rules; the
  provider's versions already are the recovery mechanism); merging folders
  on conflict (could overwrite); deleting versions permanently from the
  app (irreversible; left to the provider's console); multipart copy for
  objects above 5 GiB (not needed yet); a ZIP download (no archive
  dependency; folder download keeps the structure instead); `DeleteObjects`
  batches (per-object results and the ETag check are simpler with one
  request per object; four in flight per item).
- **Consequences:** every change in the bucket is reviewed before it
  starts, and partial results are named as such. A concurrent writer can
  still slip between a `HeadObject` and the following `CopyObject` or
  `DeleteObject` (S3 has no conditional delete at Backblaze); the window
  is one request. Large folders cost three requests per object to copy and
  two per object to delete. None of this ran against a real provider: the
  tests use an in-memory versioned S3 backend (`test_server::Versioned`)
  and s3s-fs, which model S3's documented behaviour, not Backblaze's or
  Hetzner's. 22 more commands (38 in all), each with its own permission.

## ADR-027: Desktop tests run a test-profile build that cannot reach the user's data

Status: Accepted (2026-11-14). Replaces the "move the user's database aside
and put it back" setup of the desktop drivers; code:
`src-tauri/src/test_profile.rs`, `run()` in `src-tauri/src/lib.rs`,
`CredentialNamespace` in `src-tauri/src/cloud/keyring.rs`, the `TOOLTIP` in
`tray.rs`, `test_profile_guard` in `src-tauri/build.rs`,
`scripts/check-release-binary.mjs`, `scripts/check-desktop-isolation.mjs`,
`tests/desktop/TestProfile.psm1`, `tests/desktop/test_profile.py`.

- **Context:** on 2026-11-14 a local rehearsal of `windows-cloud-ui.ps1`
  acted on the user's real Backblaze account and its cleanup deleted the
  user's real account database
  ([status](../status/status.md#test-isolation-incident-root-cause-and-fix-2026-11-14)).
  The drivers computed the app's data folder themselves
  (`%LOCALAPPDATA%\com.sotyware.desktop`, or `$XDG_DATA_HOME` on Linux),
  moved its databases aside, and deleted "the test's" copy at the end. The
  agent's processes ran inside the Claude desktop app's MSIX package, where
  `%LOCALAPPDATA%` is virtualized: the driver moved its virtual copy, the
  app opened the user's real file through the merged view, and the driver
  then deleted that file. Credential Manager is shared by every process, so
  test credentials and the user's lived side by side under one prefix. The
  driver also pressed the first matching button on the page. Any scheme
  where the driver guesses the app's paths, or the app silently uses its
  normal profile, can fail this way again.
- **Decision:**
  - A Cargo feature **`test-profile`**, off by default and never packaged
    (build.rs requires `--target-dir target/test-profile`, or `recorder`;
    `request-recorder` turns it on as well; `check-release-binary.mjs`
    refuses its marker `sotyware-test-profile-build`, its feature on a
    `tauri build` command line, and any default that enables it).
  - Such a build **refuses to start** (exit 3, before any plugin, window,
    store or credential exists) unless `SOTYWARE_TEST_PROFILE` names an
    absolute, existing directory holding `sotyware-test-profile.json`
    written by the driver (format 1, purpose, run id, 32-digit hex nonce).
    There is no fallback to the normal profile; the variable means nothing
    to a normal build (the code is not compiled in).
  - After `build()` it **refuses a profile** that is, contains or lies
    inside the app's own folders (data, local data, config and cache
    folders of the real identifier, resolved through symbolic links and
    `..`, case-insensitive on Windows).
  - It runs under **its own identifier** `com.sotyware.desktop.test` (own
    single-instance lock, own default folders, never a hand-over to an
    installed sotyware), keeps its **three databases in `<profile>/data`**
    and its **webview data in `<profile>/webview`** (the dashboard is
    created in `setup` with that folder), and its tray icon's tooltip is
    "sotyware test profile".
  - Credentials live in a **separate namespace**: service
    `com.sotyware.desktop.test`, user names prefixed with the profile's
    nonce (Windows target `com.sotyware.desktop.test/<nonce>/cloud-account/<id>`).
    `CredentialNamespace::normal()` is not compiled into a test-profile
    build, so no code path of such a build can name the user's entries.
  - Once its stores are open it writes **`sotyware-test-profile-ack.json`**
    (atomically): pid, run, nonce, identifier, the profile, data and webview
    folders, the three database files, the credential service and prefix.
    Without it the app exits 3.
  - Every driver **creates a fresh profile** per run (never reused), sets
    the variable, and after each launch that becomes the primary instance
    **waits for the acknowledgement and compares it** with what it set up
    and launched (same run, nonce and pid, the test identifier, every path
    inside the profile, none in the app's own folders). Anything else stops
    the run before a window is touched. Drivers check the binary is a
    test-profile build first, launch it once without a profile and once
    with an invalid one (both must exit 3 with no window), fingerprint the
    user's own folders (names, sizes, times, SHA-256) and, on Windows, the
    user's `com.sotyware.desktop/` Credential Manager lines before and
    after, and remove only their own profile (refused unless it is directly
    under the parent it was created in and its marker has the run's nonce)
    and only credentials under the profile's prefix.
  - **`scripts/check-desktop-isolation.mjs`** (in `test:headless`) fails
    if a driver names `%LOCALAPPDATA%`, a `com.sotyware.desktop/` path or
    target, moves, copies or deletes a `.sqlite3` file, calls
    `cmdkey /delete`, or launches the app without the profile checks.
    Tightened on 2026-11-14 after an audit found it passed the
    incident-era `cloud_ui.py` (any `exec(` stood in for all three launch
    checks): checks count only in code, not comments; more launch forms
    (`Start-Process -FilePath`, `& $Exe`, `subprocess.run/call/...([BIN`)
    and a shell script starting `$BIN` itself are caught; `%APPDATA%`,
    `ApplicationData` and the bare identifier as a string literal (as in
    `os.path.join(data, "com.sotyware.desktop", …)`) count as the user's
    folders; `secret-tool clear` and `delete_password` as credential
    deletes. It cannot follow a path across variables; the runtime
    profile is the protection, the guard keeps the incident's patterns
    out. Since `ci.yml` skips `tests/desktop/**`, `isolation-guard.yml`
    runs the guard for changes there.
  - On Windows a driver **stops before launching** if the user's real
    app folders cannot be read from outside its process tree (WMI); until
    2026-11-14 it only printed "could not be read" and went on, which is
    exactly the MSIX case the check exists for.
- **Rejected:** moving or copying the user's databases (exactly what
  failed); an environment variable that redirects a normal build's data
  folder (one forgotten variable would put the user's data back in reach,
  and production would carry a test switch); checking that the page "looks
  empty" (an empty normal profile passes, and the user's data can still be
  touched by a later step); identifying the app by its window title or a
  first-match button.
- **Consequences:** every desktop test needs a separate build
  (`npm run build:test-profile`, about the time of a release build; the
  recorder build is one too). Packaged builds are unchanged. The tests no
  longer exercise the exact identifier and folders of a shipped build;
  `windows.ps1` and the Linux drivers still test single instance, tray,
  close and persistence, under the test identifier. Webview2 or WebKitGTK
  may still write outside the profile where they ignore the data folder
  they are given (not seen: the fingerprints of the user's folders were
  unchanged in every run so far).

## ADR-028: Release binaries are built with the build machine's paths remapped

Status: Accepted (2026-11-14). Code: `scripts/build-release.mjs`
(`npm run build:release`), `localPrefixes`/`localPathIn` and the
`--local-paths` mode in `scripts/check-release-binary.mjs` (also run by
Tauri's `beforeBundleCommand`), `.github/workflows/release-build.yml`.
How to build: [release.md](../development/release.md#building-a-release).

- **Context:** the release-candidate audit found 601 absolute paths such as
  `C:\Users\<name>\.cargo\registry\src\…` in the normal Windows release
  binary (746 `/home/<name>/.cargo/…` in the Linux one). They are panic
  locations (`file!()`, `#[track_caller]`) of crates.io dependencies,
  which rustc records with absolute paths; `strip = true` removes symbols,
  not these strings. They give away the builder's user name and folder
  layout, and differ between machines, so two builds of one commit differ.
- **Decision:** release builds go through `npm run build:release`, which
  runs `tauri build` with rustc's `--remap-path-prefix` for the home
  directory (`~`), RUSTUP_HOME (`rustup`), CARGO_HOME (`cargo`) and the
  checkout (`sotyware`), shortest first because rustc applies the last
  matching remap. The flags travel in `CARGO_ENCODED_RUSTFLAGS`, appended
  to what the caller set. After the build the binary is checked for those
  directories (either separator, any case). The bundling guard refuses to
  package a binary that names them, so a plain `tauri build` can no longer
  produce an installer with them. The wrapper refuses `--features`,
  `--target`, `--target-dir`, `--debug` and `--config`: it is for the
  normal release only.
- **Rejected:** Cargo's `trim-paths` profile option (what this wants, but
  still unstable in Cargo 1.99.1: "feature `trim-paths` is required"); a
  committed `.cargo/config.toml` with `[build] rustflags` (it cannot name a
  machine's own paths, and would change every debug and test build's
  flags and fingerprints); `RUSTFLAGS` documented as a manual step (one
  forgotten variable ships the paths again).
- **Consequences:** panic messages in release builds keep the crate,
  version and file (`cargo/registry/src/index.crates.io-…/tokio-1.54.1/src/…`)
  but no machine path. Debug, test, test-profile and recorder builds are
  unchanged and keep full paths. The remapped release build has its own
  Cargo fingerprints: switching between `npm run build:release` and
  `tauri build --no-bundle` in one target directory recompiles. A
  `[build] rustflags` in a Cargo config file is overridden while
  `build:release` runs. Strings that a dependency builds from
  `env!("CARGO_MANIFEST_DIR")` would not be remapped; the check after the
  build would refuse them (none found).

## ADR-029: Provider behaviour is a capability model with evidence levels

Status: Accepted (2026-11-15). Code: `src-tauri/src/cloud/provider.rs`
(`Capabilities`, `Support`, `Evidence`).

- **Context:** sotyware talks to every provider through one S3 adapter
  (ADR-016), but S3-compatible services differ in exactly the places where
  data safety depends on them: conditional writes (`If-None-Match: *`),
  copy preconditions (`x-amz-copy-source-if-match`), whether a delete
  reports a delete marker, version listing, `ListMultipartUploads`, whether
  user metadata survives a copy, presigned URLs, Object Lock. These
  assumptions were spread over code comments and documentation, and some
  were only ever seen against a local mock server.
- **Decision:** `ProviderProfile::capabilities` lists each behaviour as a
  `Support` value: `Yes(Evidence)`, `No(Evidence)` or `Unknown`, where
  `Evidence` is `Documented` (the provider's documentation says so),
  `VerifiedAtProvider` (an opt-in test against the real provider showed it;
  the date and test are in status.md), or `AssumedFromS3` (Amazon's
  documented behaviour, not checked at this provider). Code that depends on
  a behaviour asks the capability; a safety mechanism that can misbehave on
  an unsupported provider (sending `If-None-Match: *`) is used only on
  `Yes(_)`. Checks that are harmless when ignored (the copy precondition,
  verified afterwards by `HeadObject` anyway) keep being sent. A local mock
  server never counts as evidence for a real provider. The capabilities are
  returned by `list_cloud_providers` so the dashboard can say what a bucket
  does and does not guarantee.
- **Rejected:** probing providers at runtime (a failed probe write is itself
  a write, and "accepted the header" does not prove "enforced it");
  per-account capability overrides (users cannot know; wrong answers would
  remove safety checks).
- **Consequences:** behaviour differences are visible and testable in one
  place. Backblaze B2 and Hetzner keep conditional writes off until a run
  at the provider shows them enforced; the HEAD-then-write race stays
  documented for them.

## ADR-030: Remote and local side effects are journaled before they happen, and recovery inspects instead of repeating

Status: Accepted (2026-11-15). Code: `src-tauri/src/journal.rs`,
`storage.rs` (`JOURNAL_FILE_NAME`), hooks in `cloud/upload.rs`,
`cloud/download.rs`, `cloud/bulk_download.rs`, `cloud/operations.rs`;
recovery commands in `cloud/recovery.rs`; page `src/CloudRecovery.svelte`.

- **Context:** uploads, downloads and file operations keep their state in
  memory. A crash, power loss or kill leaves orphaned multipart parts
  (billed, invisible), unexplained copy-plus-original pairs from a move,
  partly applied deletes and hidden temporary files, and the app cannot say
  afterwards what happened.
- **Decision:** a separate SQLite database, `operations-journal.sqlite3`
  (own append-only migrations, same open/refuse rules as the others), gets
  an entry **before** each step that can create, change or remove remote
  objects or local files: an upload's intent (account, bucket, key, size,
  SHA-256 once hashed, local path — the database is local and never
  synced), its multipart upload id as soon as `CreateMultipartUpload`
  returns, its final outcome; a file operation's planned steps and each
  step's progress (copied, verified, source deleted, delete marker added);
  each download's temporary file and the several-item download's staging
  folder. Entries that reach a confirmed final state are removed. At
  startup nothing is sent: entries left over are shown in a **Recovery**
  view, classified from the journal alone, and each is resolved by an
  explicit, reviewed action that first **inspects** the remote state:
  an upload is checked by `HeadObject` (stored with the expected SHA-256 →
  confirmed complete; absent → nothing stored; other content → needs
  attention, never deleted); an open multipart id can be aborted (that
  exact id only); a move with copied-but-undeleted originals is explained
  with the kept originals' keys (finishing it is a normal reviewed delete in
  the browser); a partly applied delete is reported with what was deleted
  (delete markers are restorable). The only
  automatic cleanup is sotyware's own temporary files and staging folders
  recorded in the journal, matched by sotyware's partial-name pattern.
  Incomplete multipart uploads not in the journal (another device, an
  earlier version) can be listed (`ListMultipartUploads`) and aborted only
  one by one on explicit confirmation, because another device may still be
  uploading.
- **Rejected:** repeating interrupted operations automatically (could
  duplicate data or overwrite something written meanwhile); writing journal
  entries after the step (a crash between the step and the write would hide
  it); keeping the journal inside an existing database (separate schemas
  evolve independently, as for accounts and places).
- **Consequences:** one small write per upload, download and staging
  folder, and per phase of each operation item (SQLite on the blocking
  pool); a journal write failure on an open journal stops the transfer or
  operation before the step, because an unjournaled step could not be
  recovered. A journal that cannot be opened (damaged, newer schema) is left
  untouched like every other database: work goes on unrecorded and the page
  says so, so a damaged journal never blocks the user's storage. Recovery is
  evidence-based; a journal entry is a hint of what to inspect, not proof.
  Commands: `list_cloud_recovery`, `inspect_cloud_recovery`,
  `dismiss_cloud_recovery`, `list_cloud_incomplete_uploads`,
  `abort_cloud_incomplete_upload` (five, 43 in all).

## ADR-031: Backups are content-addressed blobs plus plain JSON snapshot manifests in an independent destination

Status: Accepted (2026-11-15), implemented the same day. Code:
`src-tauri/src/backup/`, `features/backup.rs`; page `src/Backup.svelte`;
tool `scripts/recovery/sotyware-restore.py`; design: [backup.md](backup.md);
recovery without sotyware: [docs/user/recovery.md](../user/recovery.md).

- **Context:** provider versioning is not a backup: one account, one key,
  one provider, one lifecycle policy. The user wants to keep files only in
  sotyware's storage and still survive deletion, corruption, credential
  compromise, provider failure and the loss of the computer.
- **Decision:** a backup job copies a source (account, bucket, prefix) into
  a destination (another account, ideally another provider; bucket;
  prefix) as **content-addressed blobs** (`blobs/sha256/<2 hex>/<sha256>`,
  each stored once, never overwritten, carrying `sotyware-sha256`) and one
  **snapshot manifest** per run (`snapshots/<UTC time>-<id>.json`: every
  key with size, SHA-256, source ETag, version id, modification time and
  content type). Unchanged objects (same size, ETag and modification time as
  in the previous snapshot) are not transferred again; identical content is
  stored once. Objects are streamed through a local spool file, hashed, and
  uploaded through `S3Store::upload` (verified by `HeadObject`). The
  manifest is the recovery manifest: plain JSON, no secrets, readable with
  any S3 tool. **Verification** (quick: every blob of the snapshot exists
  with its size and SHA-256 metadata; full: every blob downloaded and
  hashed) gates health: an unverified backup is never shown as healthy.
  **Retention** (keep last N, daily D, weekly W) runs only after the newest
  snapshot is verified, re-reads every manifest from the destination before
  deleting a blob, deletes only blobs no remaining manifest references, and
  is skipped when the newest snapshot is **suspicious** (its change or
  deletion ratio exceeds the job's threshold, default 30 %; counted by
  content after the transfers). Restores go to
  a local folder or a bucket prefix, never overwrite, and verify every file
  against the manifest's SHA-256. Schedules run from the `backup-scheduler`
  built-in feature; one backup task at a time, in its own slot; quitting
  cancels. Manifests live under `snapshots/<job>/`; retention keeps
  unreferenced blobs younger than the oldest remaining manifest minus 24 h
  (another device may be running the same job). Nine commands (52 in all).
  A standalone, stdlib-only Python tool restores a snapshot without
  sotyware and is tested against the test server in CI. The
  destination's Object Lock configuration is read and shown as it is;
  "immutable" is said only for default retention in compliance mode.
- **Rejected:** restic (an extra binary and repository format the user
  cannot read without it; its encryption makes recovery depend on the
  restic password); mirroring keys one-to-one (no history, a deletion or
  corruption propagates); provider-side replication (single provider,
  not available across providers); encrypting blobs (recovery would depend
  on a key; the destination's access control protects them instead, see
  the threat model).
- **Consequences:** history and deduplication at the cost of a spool file
  as large as the largest object; restores need the manifest plus blobs
  (both plain objects). A key without delete permission works for backups
  (retention then reports what it could not prune).

## ADR-032: sotyware accounts are a separate service; synced configuration is end-to-end encrypted with a recovery key and device keys

Status: Accepted (2026-11-15). Code: `server/` (account service),
`src-tauri/src/account/` (client, vault crypto, sync); page
`src/Account.svelte`; design and threat model: [accounts.md](accounts.md),
[threat-model.md](threat-model.md).

- **Context:** the user wants to sign in on another computer and find their
  storage accounts there, without re-entering keys, and without the service
  being able to read those keys. No backend exists.
- **Decision:**
  - **Service:** a small Rust HTTP service (axum, SQLite with append-only
    migrations, versioned API `/v1`), independent of any cloud provider
    credential; it never has bucket access. Passwords hashed with Argon2id
    (memory-hard parameters recorded in the code); opaque random tokens
    (access 15 min, refresh 30 days, rotated on every use, reuse revokes the
    whole session family), stored only as SHA-256 hashes; rate limits per IP
    and per account with progressive lockout; responses that do not reveal
    whether an email is registered; audit events without secrets; online
    database backups with a tested restore. Email delivery (verification,
    reset) goes through a mailer interface; the development mailer writes to
    a local outbox; SMTP is an external gate.
  - **Vault:** the client generates a random 256-bit **account key**. The
    synced configuration (storage accounts with their credentials,
    favorites, backup jobs, preferences, the trusted device list) is one
    JSON document encrypted with XChaCha20-Poly1305 under it (versioned
    envelope; AAD binds account id, key id and format version). The account
    key is stored at the server only **wrapped**: under a key derived with
    HKDF-SHA256 from a 256-bit **recovery key** shown once to the user and
    verified by retyping, and as an X25519 sealed box to each enrolled
    device's public key. Device private keys stay in the OS credential
    store. The password never derives any vault key.
  - **Enrollment:** a newly signed-in device has no vault access until the
    user enters the recovery key on it or approves it from an enrolled
    device after comparing a fingerprint of the new device's public key
    shown on both screens. Revoking a device ends its sessions, deletes its
    wrap and prompts key rotation (new account key, vault re-encrypted,
    re-wrapped to the remaining trusted devices listed inside the vault and
    to the recovery key).
  - **Sync:** optimistic concurrency on the vault revision (`If-Match`);
    a stale write gets 409 and the client merges three-way per record (base
    = the last synced vault, kept locally as ciphertext); a record changed on
    both sides keeps the remote value and stores the local one as a visible
    conflict copy; deletions are tombstones. Devices refuse a vault older
    than the newest revision they have seen. Local paths, temporary state,
    SQLite internals and running operations are never synced.
  - **Optional:** without an account nothing changes for local-only users.
  - **Implementation (2026-11-15):** a key proof (one-way HKDF of the
    account key, stored hashed) gates every vault write, device wrap,
    approval and recovery-wrap change; key rotation is one atomic
    service operation (`/v1/keys/rotate`); the client uses `reqwest` with
    rustls on ring and the OS roots, no redirects, HTTPS only (loopback
    HTTP for development); account secrets live in the OS credential
    store under their own entry prefix; no production service is
    configured (`PRODUCTION_SERVER = None`). 20 commands (72 in all; 77
    with the update commands of ADR-033).
- **Rejected:** password-derived vault keys (a weak or reused password would
  expose every credential to whoever obtains the server data); server-side
  key escrow (a backdoor); a third-party identity provider (adds an external
  dependency and data processor; can be added later in front of the same
  API); JWT access tokens (opaque hashed tokens are revocable and need no
  signing key management).
- **Consequences:** losing the recovery key and every enrolled device makes
  the vault unrecoverable by design; storage data is unaffected and keys can
  be re-entered. Production use needs hosting, TLS, SMTP and server backups
  (external gates); the service is tested locally only.

## ADR-033: Updates use tauri-plugin-updater behind a separately signed manifest, installed only on the user's click

Status: Accepted (2026-11-15), implemented the same day. Its update source
(a hosted `sotyware-update.json` served without redirects, `PRODUCTION`)
and the "no restart from inside" rule are superseded by
[ADR-036](#adr-036-updates-come-from-the-latest-github-release-latestjson-signed-and-verified-as-in-adr-033);
everything else stands. Code:
`src-tauri/src/updates.rs`, feature `src-tauri/src/features/updates.rs`,
page `src/Updates.svelte`, release tooling `scripts/update-manifest.mjs`;
design: [updates.md](updates.md).

- **Context:** releases should reach users without manual reinstalls, but an
  update channel is the most powerful attack path into every installation.
- **Decision:** use the maintained `tauri-plugin-updater` (Tauri 2), driven
  from Rust only (no frontend `updater:` permission; the page uses
  sotyware's own commands). Its artifacts are minisign-signed by the release
  process; the public key is compiled in. In addition, sotyware fetches a
  **signed manifest** (`sotyware-update.json` + `.sig`, signed with the same
  minisign key) and checks, before involving the updater: the signature,
  the channel, a version strictly newer than the running one and not lower
  than the highest version this installation was ever offered (stored
  locally), and that the updater's own view of the release (version, URL,
  signature) equals the signed manifest. After download, the artifact's
  SHA-256 must equal the manifest's before `install`. Windows updates use the
  NSIS artifact for NSIS installations and the MSI for MSI installations.
  Checks run at startup and every 24 h (one timer), and on request;
  installing needs the user's click and is refused while a transfer, file
  operation or backup runs. Release builds without a configured production
  public key and endpoint report updates as not configured.
- **Rejected:** a custom installer replacement (re-implements what Tauri
  maintains); trusting the plugin's unsigned `latest.json` alone (a
  tampered manifest could point a higher version number at an older signed
  artifact); silent automatic installation (restarts during transfers).
- **Consequences:** two signatures per release (artifact and manifest),
  both made by the official `tauri signer sign --app-version` (it handles
  Tauri's encrypted key format and binds the version) and checked by
  `scripts/update-manifest.mjs`, which writes the manifest only from
  verified installers. No production key exists yet: the mechanism is
  tested with keys generated in the tests and a fixture signed once by the
  real Tauri CLI with a throwaway key (its private half deleted). New
  dependencies: `tauri-plugin-updater` (rustls without a provider: sotyware
  passes its own ring-based TLS configuration), `minisign-verify` (the
  manifest signature; already used by the plugin), `semver`.
- **Implementation notes (2026-11-15):** the verified manifest is handed to
  the plugin with `Updater::restore_update` (no second fetch; it forces the
  signed-version check); `UpdaterBuilder::build` insists on an endpoint, so
  a reserved `https://plugin-endpoint.invalid/` is configured and never
  fetched. The automatic checks are the built-in feature `update-check`
  (default on), so a user can stop sotyware from contacting the update
  server by itself. On Windows the plugin ends the process after starting
  the installer; sotyware runs `release_resources` (the same cleanup as
  `RunEvent::Exit`) in the plugin's `on_before_exit` hook. Off Windows the
  plugin returns after installing and sotyware does not restart itself (a
  restart would meet its own single-instance lock). Manifests carry no
  generic `windows-x86_64` entry, so a build without an installer marker is
  offered nothing.

## ADR-034: On Windows the linker embeds the Common Controls manifest into every binary, test binaries included

Status: Accepted (2026-11-15). Code: `src-tauri/build.rs`
(`embed_windows_manifest`), `src-tauri/windows-app-manifest.xml`.

- **Context:** a Windows program that imports `TaskDialogIndirect` or the
  window-subclass functions needs Common Controls v6, which Windows loads
  only for a program whose manifest asks for it; without one the loader
  stops it with `STATUS_ENTRYPOINT_NOT_FOUND` before `main`. `tauri-build`
  puts that manifest into the app binary as a resource, but not into test
  binaries. Since the update tests build a Tauri app on the mock runtime
  (ADR-033), the unit-test binary links that code too, and on native
  Windows 11 it did not start (found 2026-11-15, before anything was
  pushed).
- **Decision:** for Windows MSVC targets `build.rs` gives `tauri-build`
  `WindowsAttributes::new_without_app_manifest()` and passes the linker
  `/MANIFEST:EMBED /MANIFESTINPUT:windows-app-manifest.xml`, a copy of
  `tauri-build`'s own default manifest. This is the approach Tauri
  documents for testing on Windows.
- **Rejected:** skipping the mock-runtime tests on Windows (Windows is the
  release platform; those tests matter most there); moving them to an
  integration test (they need crate-private items).
- **Consequences:** the app binary's manifest is unchanged in content (one
  manifest, resource ID 1, checked on a native Windows debug build; icons
  and version resource as before). A change of `tauri-build`'s default
  manifest must be copied by hand.

## ADR-035: The dashboard is a desktop shell with a sidebar on one token-based design system

Status: Accepted (2026-11-15). Code: `src/App.svelte`, `src/Sidebar.svelte`,
`src/Overview.svelte`, `src/Settings.svelte`, `src/styles/tokens.css`,
`src/styles/base.css`, `src/theme.ts`, `src/Icon.svelte`,
`src/updatesModel.svelte.ts`; design: [frontend.md](frontend.md).

- **Context:** the dashboard grew from one section to five (Overview, Cloud
  Storage, Backups, Account, update settings) inside a 36rem centred column
  with tab-like links. Every component styled itself with its own
  hard-coded colours and its own `prefers-color-scheme` block, so the same
  meaning (an error, a notice, a primary action) looked different on each
  page, and a theme chosen by the user could not override the system's.
  The file browser, the densest page, had the same narrow column.
- **Decision:** one shell: a sidebar (sections, the storage accounts as
  direct links to their buckets, Account and Settings at the bottom) and a
  main area whose pages share a header pattern; list and form pages keep a
  56rem reading width, the bucket browser uses the full width. A narrow
  window turns the sidebar into an overlay. All colours, sizes, radii,
  shadows and durations are CSS custom properties in `tokens.css`, with a
  light and a dark palette; `base.css` styles the elements and a small
  shared vocabulary (`.callout`, `.badge`, `.card`, `.empty`, button
  variants, …). Components keep layout-only styles and never a colour
  literal. The theme is System, Light or Dark, chosen in Settings ›
  Appearance: stored in the webview's `localStorage` (a per-device display
  preference, read synchronously before the first paint), applied as
  `data-theme` on `<html>`, and mirrored to the native window with
  `setTheme` (title bar), which needs the one new permission
  `core:window:allow-set-theme`. Features and About & updates move into
  Settings; one update notification appears in the shell. Icons are an
  in-repo set of line drawings (`Icon.svelte`), no icon font or package.
  The update state is one `UpdatesModel` per window (`.svelte.ts` runes),
  shared by the notification and the settings panel. Keyboard: Ctrl/⌘+1–3
  switch sections, Ctrl/⌘+, opens Settings, Ctrl/⌘+F searches a bucket,
  ⌘⌫ deletes on a Mac; page titles take focus on navigation.
- **Rejected:** a component library or CSS framework (a dependency and a
  second design language for a small app); a router (navigation stays
  local state, ADR-001); storing the theme in SQLite through a new command
  (an asynchronous read flashes the wrong theme at startup, and the choice
  is a per-device convenience with no security meaning); a custom title
  bar (native decorations keep each platform's own window controls).
- **Consequences:** every visual change is a token or a vocabulary class.
  Accessible names the desktop drivers use stayed (Open bucket, Check
  connection, Upload files…), with deliberate exceptions recorded in
  [frontend.md](frontend.md#names-the-tests-rely-on): "← Back to accounts"
  became "Back to accounts", folder rows lost their 📁 emoji (an icon now),
  and the features moved to Settings › Features. A theme saved in
  `localStorage` lives in the webview data folder (the test profile's, in
  test builds) and is lost with it; then the system theme is used.

## ADR-036: Updates come from the latest GitHub release (`latest.json`), signed and verified as in ADR-033

Status: Accepted (2026-11-15). Supersedes ADR-033's update source. Where
the releases live: [ADR-038](#adr-038-releases-are-published-to-a-separate-public-repository-casualsotysotyware-releases)
moved them to a separate public repository, and
[ADR-039](#adr-039-releases-are-published-in-the-public-source-repository-with-github_token-signing-credentials-live-in-protected-environments)
brought them back to this repository once it was made public. Code:
`src-tauri/src/updates.rs`, `src-tauri/update-key.pub`,
`scripts/update-manifest.mjs`, `.github/workflows/release.yml`; design:
[updates.md](updates.md), procedure:
[release.md](../development/release.md#publishing-a-release).

- **Context:** ADR-033 needed a hosted update server (a domain, hosting,
  TLS) that does not exist. GitHub Releases already stores the installers;
  its `releases/latest/download/<asset>` URL always redirects to the asset
  of the release marked latest (never a draft or a pre-release).
- **Decision:** every release carries `latest.json`, the manifest of
  ADR-033 (the updater plugin's static format plus `format`, `channel`
  and per-platform `sha256`), and `latest.json.sig`, its minisign signature
  by the release key. sotyware fetches both from
  `https://github.com/casualsoty/sotyware/releases/latest/download/`
  (`MANIFEST_URL`), following at most five redirects and each only to
  HTTPS (loopback HTTP only in test builds); a signature that does not
  match is fetched once more (a release published between the two
  requests). All of ADR-033's checks stay: signature first, channel,
  version floor against replays, an HTTPS URL, a signature and a SHA-256
  per package, the plugin handed exactly the verified JSON, the package's
  signature and signed version, the SHA-256 before installing, installation
  only on the user's click and only while nothing runs. Platform entries
  now cover `windows-x86_64-{nsis,msi}`, `darwin-{aarch64,x86_64}-app` and
  `linux-x86_64-{appimage,deb,rpm}`. The public key is a file in the
  repository, `src-tauri/update-key.pub`, compiled in; while it holds no
  key, release builds report updates as not set up. On macOS and Linux the
  plugin replaces the app in place; sotyware then offers **Restart now**,
  which uses `AppHandle::request_restart` (the normal exit, so
  `release_resources` runs and the single-instance lock is released before
  the new process starts); Windows' installer still restarts sotyware
  itself. New command `restart_after_update` (refused unless this process
  installed an update and nothing runs). Status and failures carry a kind
  (`network`, `signature`, `integrity`, `replay`, `platform`, `malformed`,
  `installer`, `busy`) so the page words them without parsing text.
  Releases are made only by `release.yml`: version and tag checked, the
  signing secret checked against the committed public key, CI on the
  tagged commit, Windows, Linux and both macOS builds, then one job, the
  only one with the signing key, signs every updater artifact and
  `latest.json` with `tauri signer sign --app-version`, verifies all of it
  with the public key alone, uploads to a draft, publishes it as a
  pre-release, downloads every file through its public URL and verifies it
  again, and only then marks it latest. A manifest names exactly the
  platforms built in that run (macOS only when signed and notarized).
- **Rejected:** trusting GitHub as the authority (a compromised account or
  token could publish anything; the release key is the authority, kept out
  of every build job); the plugin's own endpoint fetch with an unsigned
  `latest.json` (ADR-033's reasons stand); a pinned host allow-list for the
  redirects (GitHub has changed its asset host before; the signature, not
  the host, decides); universal macOS binaries (ADR-037).
- **Consequences:** **the repository's releases must be public**: GitHub
  serves release assets of a private repository only to signed-in clients,
  so with the repository private today no installation could download
  `latest.json`, and `release.yml` refuses to publish (a dry run still
  works). No installation exists yet (0.1.0 was never released and had
  updates switched off), so nothing migrates; the first release with a key
  in `update-key.pub` is the first that can be updated, and every later
  key change needs the rotation procedure in
  [release.md](../development/release.md#update-signing-key). Losing the
  private key strands every installation on its version.

## ADR-037: macOS is a supported build target: Keychain, a menu bar, separate Apple Silicon and Intel builds

Status: Accepted (2026-11-15). Code: `src-tauri/Cargo.toml`
(`apple-native-keyring-store`, `security-framework`),
`src-tauri/src/cloud/keyring.rs`, `src-tauri/src/app_menu.rs`,
`src-tauri/src/lib.rs` (`RunEvent::Reopen`), `src-tauri/tauri.conf.json`
(`bundle.macOS`), `.github/workflows/macos.yml`; guide:
[macos.md](../development/macos.md).

- **Context:** sotyware should run on Macs too. Its code already avoided
  OS-specific paths except the credential store (Windows and Linux only),
  and a Mac app without a menu bar gets no ⌘C/⌘V in its webview and no
  standard Quit.
- **Decision:** credentials go to the user's login keychain through
  `apple-native-keyring-store` (feature `keychain`: the legacy keychain,
  which needs neither a provisioning profile nor an entitlement), with the
  same service and account names as on the other systems; the Security
  framework's `OSStatus` codes sort errors (a dismissed prompt or a locked
  keychain is "refused", no keychain is "unavailable"), as the Secret
  Service's are sorted on Linux. A menu bar (`app_menu.rs`): the
  application menu (About, Services, Hide, Hide Others, Show All, **Quit
  sotyware** ⌘Q through `AppControl::quit`, the same path as the tray's
  Quit), Edit (undo, redo, cut, copy, paste, select all) and Window
  (minimize, zoom, close ⌘W, which hides as every close does). A click on
  the Dock icon shows the dashboard (`RunEvent::Reopen`). macOS is always
  tray mode (a menu bar icon); terminations from outside (Dock Quit,
  logout) go straight to `RunEvent::Exit`, so cleanup runs and nothing
  blocks the logout. Builds: one per architecture, each on its own hosted
  runner (`macos-15` for Apple Silicon, `macos-15-intel` for Intel), so
  each is tested natively; minimum macOS 10.15. Signing (Developer ID,
  hardened runtime) and notarization happen in CI only when the Apple
  secrets exist; without them the build is an unsigned development build
  and is left out of releases.
- **Rejected:** a universal binary (twice the download for every user and
  only the native half tested on a runner); the data-protection keychain
  (needs an entitlement and a provisioning profile); Tauri's default macOS
  menu (its Quit would bypass `AppControl::quit`); hiding the Dock icon
  (an `Accessory` app has no menu bar while its window is open).
- **Consequences:** macOS code is compiled and tested only on GitHub's
  macOS runners: the development machine has no Mac, so no build has run
  on real Mac hardware or been seen on screen; signing and notarization
  are untested until a Developer ID certificate exists. An unsigned build's
  keychain items ask for permission again after every rebuild (the
  keychain ties access to the binary's signature). The menu bar icon is the
  coloured app icon, not a monochrome template image. New dependencies:
  `apple-native-keyring-store` 1.0.2 (macOS only); `security-framework`
  3.7.0 was already in the tree.

## ADR-038: Releases are published to a separate public repository (`casualsoty/sotyware-releases`)

Status: Superseded by
[ADR-039](#adr-039-releases-are-published-in-the-public-source-repository-with-github_token-signing-credentials-live-in-protected-environments)
(2026-11-15): the maintainer made the source repository public, so releases are published
there and the release repository, its token and its checks were removed.
`check-key` and the release-key guards stay. Its statement that environment
secrets can never reach a reusable workflow was wrong: a caller cannot pass
them, but a called job that names an environment receives that
environment's secrets. Was: Accepted (2026-11-15), superseding ADR-036 in
one point: where the releases live.
Code (then): `src-tauri/src/updates.rs` (`release_repository!`,
`RELEASE_REPOSITORY`, `MANIFEST_URL`), `.github/workflows/release.yml`,
`scripts/update-manifest.mjs` (`release-repository`, `check-key`);
procedure: the "public release repository" section of release.md, removed
with ADR-039.

- **Context:** ADR-036 takes updates from the latest GitHub release, which
  installations must download without signing in. The source repository
  is private and stays private, and GitHub serves a private repository's
  release assets only to signed-in clients.
- **Decision:** releases are published to the public repository
  `casualsoty/sotyware-releases`, which holds distribution files and
  release metadata only: per release the installers, their minisign
  signatures, `latest.json`, `latest.json.sig` and `SHA256SUMS`, and on its
  default branch nothing but a README, a licence or similar notes. The
  installed app fetches
  `https://github.com/casualsoty/sotyware-releases/releases/latest/download/latest.json`
  (+ `.sig`) anonymously; the manifest's package URLs point into that
  repository's tag folder. The repository is written in one place, the
  `release_repository!` macro in `updates.rs`, from which `RELEASE_REPOSITORY`
  and `MANIFEST_URL` are built; `release.yml` reads it from there
  (`update-manifest.mjs release-repository`) and refuses it if it is the
  source repository itself, is not public, or holds anything besides those
  notes on its default branch (a release also carries GitHub's "Source
  code" archives of that branch). Builds, tests and signing stay in the
  private source repository; only the publish job writes to the release
  repository, with a fine-grained token (`SOTYWARE_RELEASES_TOKEN`:
  Contents read and write on that repository only) given to exactly the
  steps that call `gh release`; `GITHUB_TOKEN` stays read-only. The release
  tag is created in the release repository, on its default branch, when the
  draft is published; the source tag keeps its signature in the source
  repository. Signature verification is unchanged (ADR-033, ADR-036): the
  release key, not the repository, is the authority. The release key file
  is checked so the signing fixtures' throwaway keys (test-only) and a
  pasted private key can never become the release key
  (`update-manifest.mjs check-key` in `test:headless` and `release.yml`, and
  a Rust test).
- **Rejected:** making the source repository public (decided against by
  the maintainer); another host for the files (a domain, TLS and hosting
  again, ADR-036's reason); GitHub Pages in the release repository (one
  more publishing path and a branch with content; release assets suffice);
  `GITHUB_TOKEN` (it cannot write to another repository); a classic
  personal token (it would reach every repository of the account).
- **Consequences:** the maintainer creates the release repository once
  (public, initialised with a README) and a fine-grained token for it,
  which expires and must be renewed. A compromised token can publish to
  the release repository but cannot sign: installations refuse anything
  not signed by the release key, and the version floor refuses replays. The
  release notes (the source tag's message) become public. Moving releases
  to another repository later means a release built with the new
  `release_repository!` published at the old place first, since
  installations follow only the URL compiled into them.

## ADR-039: Releases are published in the public source repository with `GITHUB_TOKEN`; signing credentials live in protected environments

Status: Accepted (2026-11-15). Supersedes
[ADR-038](#adr-038-releases-are-published-to-a-separate-public-repository-casualsotysotyware-releases).
Code: `src-tauri/src/updates.rs` (`release_repository!`,
`RELEASE_REPOSITORY`, `MANIFEST_URL`), `.github/workflows/release.yml`,
`.github/workflows/macos.yml`, `scripts/release-assemble.sh`,
`scripts/update-manifest.mjs` (`release-repository --expect`,
`check-key`); procedure:
[release.md](../development/release.md#github-releases).

- **Context:** the maintainer made `casualsoty/sotyware` public on
  2026-11-15. That removes ADR-038's reason for a second repository: GitHub
  serves a public repository's release assets to anyone, without signing
  in. Being public also changes what GitHub offers the repository (standard
  hosted runners, macOS included, cost nothing; environments with
  deployment restrictions and required reviewers exist on GitHub Free) and
  who can start workflows (anyone can open a pull request from a fork).
- **Decision:**
  - Releases are published on GitHub Releases of `casualsoty/sotyware`.
    Installations fetch
    `https://github.com/casualsoty/sotyware/releases/latest/download/latest.json`
    (+ `.sig`); package URLs point at the release's tag folder.
    `release_repository!` names this repository, and `release.yml` refuses
    to publish unless it equals `GITHUB_REPOSITORY` (a fork's or a renamed
    repository's release would reach no installation).
  - Publishing uses the workflow's own `GITHUB_TOKEN`: `contents: write`
    only in the publish job, and only in the steps that call the API;
    read-only everywhere else. No personal token or app. The draft is
    created with `--verify-tag` on the pushed tag. Before it is published
    the job checks that the tag still names the commit every job built,
    compares the uploaded names and sizes and downloads the files back with
    the token; published as a pre-release, every file is downloaded again
    anonymously over HTTPS and verified with the committed public key; only
    then is the release marked latest (`prerelease=false`,
    `make_latest=true`; the pre-release is published with
    `make_latest=false`). After a failed or cancelled run the draft this
    run created is deleted (never one the job found when it started; not
    after a force-cancel or a lost runner, which skip that step); a
    published pre-release is left for the maintainer, and a failure after
    the promotion request leaves a full release that GitHub was asked to
    make the latest. Every step finds the release by its tag name, so a
    second release of the tag made during the run, which takes write
    access, can be the one a later step acts on
    ([limitations](../status/limitations.md#updates-adr-033-adr-036-adr-039)).
  - The update signing key (`TAURI_SIGNING_PRIVATE_KEY`, `…_PASSWORD`) is a
    secret of the environment `release`, which admits only `v*` tags
    (recommended: a required reviewer, administrators not bypassing). Only
    the publish job names that environment and only its signing step
    receives the key; that job installs dependencies with
    `--ignore-scripts` and restores no cache. `prepare` refuses a real
    release unless the environment exists and admits exactly the tag rule
    `v*`, and while
    the key (or an Apple credential) is a repository or organisation
    secret, which every workflow run could read.
  - The Apple credentials move from repository secrets to the environment
    `macos-signing` (`main` and `v*` tags), named only by `macos.yml`'s new
    `package` job; its `test` job never has a secret.
  - A dry run (the default for a manual run, now allowed on any branch)
    never touches the release key or the `release` environment: the job
    `rehearse` signs with a throwaway key made in the run, so every
    platform and the whole manifest chain can be rehearsed before the key,
    the `release` environment or a tag exist. `macos-signing` must exist
    first: the macOS package job runs in it, and GitHub creates a missing
    environment that a running job names, without protection (a skipped
    job, such as `publish` in a dry run, creates none). Signing and
    checking are one script, `scripts/release-assemble.sh`, shared by
    `rehearse` and `publish` and tested with the real Tauri CLI.
  - `macos.yml` runs like `ci.yml`, on every pull request to and push to
    `main`, on both architectures natively (`macos-15`, `macos-15-intel`;
    each job checks it is not translated by Rosetta and that rustc's host
    is the expected one). Release builds there restore no cache.
  - Unchanged from ADR-036 and ADR-038: signature verification and the
    version floor, `check-key` refusing fixture and private keys,
    draft → pre-release → latest, one release at a time, unsigned Windows
    installers only with `SOTYWARE_ALLOW_UNSIGNED_WINDOWS=true`, macOS only
    when both builds are signed and notarized.
- **Rejected:** keeping the separate release repository (a second
  repository and a long-lived token that expires and can leak, for a reason
  that no longer exists); a personal access token or a GitHub App for
  publishing (`GITHUB_TOKEN` can publish the repository's own releases);
  repository secrets for the signing key (readable by every workflow run,
  including one on any branch a collaborator pushes); the real key in dry
  runs (more exposure, and the publish job checks the key against
  `update-key.pub` before signing anyway); `pull_request_target` or
  `workflow_run` triggers (they run with secrets or a write token next to
  code from a fork); an environment chosen by an expression (whether an
  empty name means "no environment" is not documented, and a placeholder
  environment would be created unprotected); building the Intel Mac
  package on Apple Silicon (its tests would not run natively).
- **Consequences:** the source code and the release notes are public, and
  every release also carries GitHub's "Source code" archives of the tagged
  commit. The maintainer configures, once, the two environments, a tag
  ruleset for `v*` and the repository's Actions settings
  ([release.md](../development/release.md#repository-settings)); all of
  them exist since 2026-11-15, and `release` holds the signing key. A
  publish job's token can publish files, but installations
  refuse anything the release key did not sign, and the version floor
  refuses replays. Whoever can push a `v*` tag can start a release; the tag
  ruleset and the environment's reviewer decide whether it reaches the
  key. macOS CI time is free but slow (two Mac jobs per pull request). Intel
  Mac builds end when GitHub retires its last Intel image (announced for
  2027).

## ADR-040: On Windows the dashboard draws its own title bar; the window opens at half the primary work area and remembers its place

Status: Accepted (2026-11-15). Code: `src-tauri/src/title_bar.rs`,
`src-tauri/src/window_state.rs`, `storage.rs` (`window-state.sqlite3`),
`lib.rs` (`setup`, `on_window_event`, `show_dashboard`,
`release_resources`), `src/TitleBar.svelte`, `src/App.svelte`;
details: [frontend.md](frontend.md#title-bar-windows),
[lifecycle.md](lifecycle.md#placement-and-chrome).

- **Context:** on Windows the system title bar sat above the dashboard as a
  separate strip in the system's colours. The window opened at a fixed
  1120 × 760, wherever Windows put it, and forgot any size or place the
  user chose. Tauri 2.12 supports an undecorated window with a page-drawn
  title bar (`decorations: false`, `data-tauri-drag-region`, the window
  API) and keeps native behaviour for it on Windows: `tauri-runtime-wry`
  attaches a resize border (a child window on the top edge, the DWM frame
  elsewhere), the DWM shadow gives Windows 11's rounded corners, and tao
  keeps `WS_CAPTION`, `WS_SYSMENU`, `WS_SIZEBOX` and the maximize and
  minimize boxes, hiding only the frame. There is no native "window
  controls overlay" in Tauri for Windows.
- **Decision:**
  - The dashboard is created hidden (`visible: false`). In `setup`, after
    the mode is known (never after a fatal tray error), Windows first gets
    the runtime capability `window-controls` (start dragging, the drag
    region's double-click toggle, minimize, toggle maximize, is maximized,
    close), and only then loses its decorations, so an undecorated window
    always has controls; a failure keeps the native title bar. The page
    asks `isDecorated()` (a read-only permission in the static capability)
    and draws `TitleBar.svelte` only for an undecorated window: the name,
    a drag region over everything that is not a control (double click
    maximizes or restores), and Windows 11-style Minimize, Maximize/Restore
    and Close buttons in the design system's tokens (Close in the system's
    red). Close sends the same close request as Alt+F4, which Rust turns
    into hiding in tray mode (ADR-002). macOS and Linux keep their native
    title bars.
  - Placement (`window_state`): with nothing saved, half the width and
    half the height of the primary monitor's work area, at least 900 × 540
    logical pixels (`DEFAULT_MIN`: half of a 1366 or 1280 px wide screen
    would fall below the 760 px breakpoint and collapse the sidebar into an
    overlay; a first run of the virtual-display test showed exactly that)
    and at most the work area, centred; on 1920 × 1080 that is 960 × 540.
    The window can still be resized down to 640 × 480. Otherwise the
    saved normal bounds (outer position, inner size, physical pixels) and
    maximized state, restored on the monitor they overlap most, rescaled
    when that monitor's scaling changed, and moved (and shrunk if needed)
    to lie fully inside its work area; bounds on no connected monitor give
    the default. Showing a hidden dashboard first moves it back onto a
    connected monitor if it lies on none. The geometry is pure functions,
    unit-tested.
  - The bounds are tracked in memory from the window's Moved and Resized
    events (not while minimized, hidden or maximized) and written to
    `window-state.sqlite3` when the window is hidden or loses focus (off
    the event loop) and at exit. Its own database file, so an older
    sotyware, which never opens it, meets no unknown schema.
- **Consequences:** one cohesive window on Windows with native moving,
  snapping, resizing and keyboard behaviour. The Snap Layouts flyout that
  Windows 11 shows when hovering a native maximize button does not appear
  (Win+Z still opens it). On Windows 10 an undecorated window with a shadow
  has a 1 px border (Windows 10 is not supported). The page's title bar is
  covered by unit tests and by checks in `tests/desktop/windows.ps1` that
  have not run yet (no native Windows build in this session); a person's
  visual check on Windows 11 is still to do. A placement change during a
  drag is written only once the window is hidden, loses focus or sotyware
  quits: a crash loses it. Wayland compositors do not let a client choose
  its position.

## ADR-041: Storage usage is measured by listing; costs are estimates from versioned price lists; "snapshots" of a bucket are sotyware's backup snapshots

Status: Accepted (2026-11-15). Code: `src-tauri/src/cloud/usage.rs`,
`src-tauri/src/cloud/pricing.rs`, `src-tauri/pricing/*.json`,
`cloud/s3/manage.rs` (`usage_versions_page`, `list_parts_page`),
`storage.rs` (`storage-usage.sqlite3`), `backup/engine.rs`
(`delete_snapshot`, the shared `delete_chosen`), `backup/mod.rs`
(`start_delete_snapshot`, `backup_snapshot_details`), `src/CloudUsage.svelte`,
`src/CloudSnapshots.svelte`, `src/CloudBrowser.svelte` (tabs); details:
[cloud-storage.md](cloud-storage.md#storage-usage-and-cost-estimates),
[cloud-storage.md](cloud-storage.md#snapshots-and-backups-of-a-bucket),
[backup.md](backup.md#deleting-a-snapshot).

- **Context:** users want to see how much each bucket stores and costs, and
  to manage "Backblaze B2 snapshots" from sotyware. Read on 2026-11-15 from
  Backblaze's and Hetzner's official pages: no API reports a bucket's size
  or the account's bill (`b2_list_buckets` has no size field; Hetzner shows
  bucket statistics in its console only); Backblaze snapshots are ZIP files
  created, downloaded and deleted only in the web console, kept in a
  read-only `b2-snapshot-…` bucket, with statuses shown only there; no API
  creates or lists them. The 2026-11-11 proposal for a cost estimator
  ([cloud-storage.md](cloud-storage.md#storage-usage-and-cost-estimates))
  required prices as data with their source and date.
- **Decision:**
  - **Usage** is measured on request, read-only: every version and delete
    marker (`ListObjectVersions`, falling back to the current objects with
    `ListObjectsV2` where versions cannot be listed, and saying so), and
    unfinished multipart uploads with their stored parts (`ListParts`, up
    to 100 uploads). One measurement at a time in its own slot, progress on
    a per-call channel, cancellable, at most 20,000 pages (then marked
    incomplete). Results go to `storage-usage.sqlite3` (newest 100 per
    account): the history is sotyware's own measurements.
  - **Prices** are one JSON file per provider in `src-tauri/pricing/`
    (format `sotyware-pricing` v1: currency, taxes excluded, `checkedOn`,
    https sources, the provider's own units), compiled in and validated by
    tests. A provider without a file gets usage and no estimate.
  - **Estimates** are computed in Rust for 30 days per provider account
    (Backblaze: buckets of one region; Hetzner: all buckets, one base fee),
    as components (base price, storage, downloads at the amount the user
    states, API calls) with the assumptions written out, in the provider's
    currency, never summed across currencies, and labelled an estimate,
    not a bill. No value is presented as billing data.
  - **Snapshots:** the bucket view's "Snapshots & backups" tab explains
    Backblaze's console snapshots without pretending to manage them, and
    manages sotyware's backup snapshots (ADR-031) of the bucket: list,
    details from the manifest, Back up now, Verify, and a new deletion of
    one chosen snapshot behind a typed confirmation, run as the backup
    task `delete-snapshot`. It reuses retention's deletion: every manifest
    is read first, the chosen manifest is deleted, then only blobs no
    remaining manifest references and older than a day before the oldest
    remaining one. Health ignores a deleted snapshot.
- **Consequences:** five new commands (83 in all), two new database files
  in the test-profile handshake. A measurement of a large bucket takes
  minutes and one listing request per 1,000 entries (free at both
  providers today). The estimate misses what sotyware cannot see:
  downloads by other programs (stated by the user), Hetzner's billed
  metadata, prices of contracts, VAT. Price files must be updated by hand
  when providers change prices (the tests pin the figures read on
  2026-11-15). Deleting a snapshot is destructive at the destination; a
  versioned destination keeps the data until its lifecycle rules remove
  it.

## ADR-042: An enrolled device syncs its vault automatically on events, never on a timer

Status: Accepted (2026-11-15). Code: `src/accountSync.svelte.ts`,
`src/App.svelte`, `src/Sidebar.svelte`, `src/CloudStorage.svelte`
(`onlocalchange`, `generation`); details:
[accounts.md](accounts.md#when-sync-runs).

- **Context:** with ADR-032, a computer synced only after account actions
  and on "Sync now": a storage account added on one computer reached the
  others only when someone clicked there, and the shell showed nothing
  about sync.
- **Decision:** a device that is signed in and enrolled (the account key is
  on it) syncs when sotyware starts, when the window is focused again (at
  most every 15 minutes) and after a storage account was added, changed or
  removed on it. The page drives it (one run at a time); Rust is
  unchanged and records the outcome (`lastSyncMs`, `lastSyncError`). After
  a sync the shell reads the storage accounts again. The sidebar's Account
  item shows the state (synced, not synced and why, waiting for unlock,
  conflicts). No timer or background loop: a window that stays unfocused
  does not sync.
- **Consequences:** offline, a sync fails quietly, the reason is shown, and
  every storage account keeps working locally. Builds without an account
  service (release builds today) only read the local status. A change made
  on one computer reaches another when that one starts, is focused, or
  changes something itself.

## ADR-043: Folder sync keeps a chosen folder and a bucket in step by a three-way comparison, with guarded deletions and one background engine per sync

Status: Accepted (2026-11-16). Code: `src-tauri/src/folder_sync/`,
`src-tauri/src/storage/folder_sync.rs`, `src-tauri/src/features/folder_sync.rs`,
`S3Store::upload_with` (`cloud/s3/upload.rs`), `src/CloudSync.svelte`,
`src/folderSync.ts`, `src/folderSyncModel.svelte.ts`; details:
[folder-sync.md](folder-sync.md).

- **Context:** users want to work with a bucket's files as ordinary files on
  their computer, in a folder they choose, in both directions and in the
  background, as with Google Drive for desktop, Dropbox or OneDrive. Until
  now sotyware transferred only files the user picked, never replaced an
  object, ran no background work against a provider on its own, and its page
  never received a local path.
- **Decision:**
  - **One sync** is one storage account's configured bucket (all of it; the
    configuration has a prefix field, `""`, for a later selective sync) and
    one folder the user chose in the native folder dialog Rust opens. Each
    path has three states: the **synced state** (what both sides last agreed
    on; persisted), the **local scan** and the **remote listing** (per
    session). `folder_sync::model::decide` turns them into one decision per
    path (the table is in [folder-sync.md](folder-sync.md#the-decision-for-one-path)).
    Equal content (SHA-256) is recorded without a transfer; a change wins
    over a deletion; a file changed on both sides is **kept twice**: this
    computer's version keeps the name, the bucket's becomes
    `name (conflicted copy YYYY-MM-DD HHMM).ext` on both sides, and the user
    settles it later (keep this computer's, keep the other one, keep both).
  - **Safety rules.** Every action checks the live file and the live object
    right before it changes anything, so a decision taken on an older scan or
    listing can only do less. A modified file replaces only the version that
    was synced: `S3Store::upload_with` with `Destination::Replace { e_tag }`
    refuses another object or none (the HEAD-then-write race of every
    sotyware upload stays, ADR-021, ADR-029). Files deleted in the bucket
    are moved to the **sync trash** (`.sotyware-sync/trash/<day>/` in the
    sync folder, kept 30 days), never unlinked; deletions in the bucket add
    delete markers where the bucket keeps versions. Nothing below a folder
    that could not be read, and nothing from a listing that did not finish,
    counts as deleted. The **deletion guard** holds the deletions of a pass
    that are at least 1,000, or at least 50 and more than half of the synced
    files, until the user answers (delete them, or restore them). A
    **marker** (`.sotyware-sync/root.json`, naming the sync) must be in the
    folder: a folder that is gone (a disconnected drive) makes the sync wait
    and look again every 30 s; a folder without the marker (another disk at
    the same place, an unmounted mount point) stops it until the user
    chooses the folder again (which then syncs from scratch). A sync never
    follows its account to another bucket or endpoint.
  - **Background.** One async task per sync (`engine.rs`) runs while the new
    built-in feature `folder-sync` runs (on by default). A pass runs at
    start, after changes the file watcher reports (`notify`:
    ReadDirectoryChangesW, inotify, FSEvents; 1.5 s after the last event, at
    most 10 s), when the remote check is due (every 5, 15, 30 or 60 minutes,
    15 by default: S3 has no change feed, so the bucket is listed), on "Sync
    now", when a failed item's retry is due (30 s doubling to an hour), after
    sotyware itself changed the bucket (an upload batch, a file operation or
    a new folder of the same account nudges its sync), and every 30 s while
    the folder is unavailable. These are sotyware's first timers against
    remote state; each exists because nothing notifies, and each is bounded.
    Full local scans happen at start, after a watcher overflow and when the
    folder comes back; watcher events otherwise lead to scans of the named
    paths only.
  - **Paths reach the page.** The page receives the chosen folder's path, to
    show it before confirming and on the Sync tab: an exception to "no
    command returns a local path" (ADR-020, ADR-022), limited to the folder
    the user chose for sync. The page still never sends a path: a choice is
    a token kept in Rust, a sync an id. "Open folder" calls
    `tauri_plugin_opener::open_path` from Rust (the official plugin's
    function; the plugin is not registered and the page gets no `opener:`
    permission).
  - **Moving a sync folder:** "move" (into an empty folder only: renamed on
    one drive; between drives every file copied, read back and compared by
    SHA-256 before any original is removed; the synced state stays valid)
    or "start fresh" (the old folder is left as it is; the new one is merged
    with the bucket; nothing is deleted on either side because of the
    switch). Turning sync off forgets the configuration and the synced
    state and removes the marker; no file or object is deleted.
  - **Persistence:** `folder-sync.sqlite3` (configurations as JSON, the
    synced state, per-path issues, the last 500 activity items per sync),
    SQL in `storage/folder_sync.rs` only. The local scan and the remote
    listing live in TEMP tables, so planning streams through one SQL query in
    path order and a large bucket is never held in memory.
  - **Dependencies:** `notify` 8 (desktop targets; new in the lock:
    `notify`, `notify-types`, `inotify`, `inotify-sys`, `kqueue`,
    `kqueue-sys`, `objc2-core-services`), `tauri-plugin-opener` 2 (new:
    `tauri-plugin-opener`, `open`, `is-wsl`, `is-docker`), and direct
    declarations of `rustix` (Unix: `statvfs` for free space, safe) and
    `windows-sys` 0.61 (Windows: `GetDiskFreeSpaceExW`, and
    `SetFileAttributesW` to hide `.sotyware-sync`), both already in the tree.
    The two Windows calls are sotyware's first `unsafe` blocks, each with a
    `SAFETY` comment.
  - **16 commands** (99 in all), one `allow-<command>` permission each. The
    live status reaches the page over a `Channel` per subscription
    (`watch_folder_syncs`), not as an event.
- **Alternatives considered:** a virtual file system or the platforms' cloud
  file APIs (files online until opened): not for this first version, which
  keeps every file available offline; the design leaves room for it (the
  synced state could mark a file online-only). A two-way mirror without a
  synced state: it cannot tell a deletion from a creation, so it either
  loses deletions or brings deleted files back. Local scans without a
  watcher: kept as the safety net, not the only source. "The bucket keeps
  the name" for conflicts (Dropbox's way): it renames the user's file, which
  fails on Windows while an editor holds it open and moves the work they are
  doing.
- **Consequences:** each remote check costs one listing request per 1,000
  objects (for a large bucket a real request count; the interval is the
  user's); a provider whose `HeadObject` sends no ETag cannot sync changed
  files (the replacement is refused, the file reported); on a bucket without
  versioning a file deleted or changed in the folder is deleted or replaced
  in the bucket for good (said before the sync starts and on the tab); a
  sync upload interrupted by a crash is listed under Interrupted work like
  any upload (ADR-030), while the sync redoes it; a file system that ignores
  case cannot hold two keys that differ only in case, and Windows cannot
  hold names it refuses: those objects are reported as not synced. Tested
  only against the in-process servers and the WSL2 file system; never at a
  real provider, on native Windows or macOS, or on a real Linux desktop.

## ADR-044: The account service delivers its outbox through any SMTP relay, with lettre

Status: Accepted (2026-11-16). Code: `server/src/mail.rs`, migration 2 in
`server/src/db.rs`, the wake-ups in `server/src/api.rs`, `send-test-mail`
in `server/src/main.rs`; operations: [server.md](../development/server.md#email).
Replaces ADR-032's "SMTP is an external gate" for the code; choosing and
configuring a relay stays one.

- **Context:** the service queued verification codes, reset codes and
  notices in an `outbox` table that nothing sent, so no real person could
  verify an address or reset a password. The maintainer chose a free email
  plan with SMTP (Brevo suggested) beside a small VM.
- **Decision:**
  - **SMTP, not a provider's HTTP API.** Every transactional email
    provider accepts SMTP submission, so changing provider is a change of
    environment variables (`SOTYWARE_SMTP_HOST`, `_PORT`, `_SECURITY`,
    `_USERNAME`, `_PASSWORD`, `SOTYWARE_MAIL_FROM`), not of code.
  - **`lettre` 0.11** (no default features; `smtp-transport`, `builder`,
    `hostname`, `tokio1-rustls`, `ring`, `rustls-native-certs`): the
    established Rust SMTP client, on the tokio runtime the service already
    uses, with rustls on ring as in the desktop app (no OpenSSL, no
    aws-lc-rs) and the system's root certificates. Writing an SMTP client
    with STARTTLS and AUTH ourselves would be more code to get wrong.
  - **The outbox stays the queue.** Handlers still only insert into it,
    inside the transaction that decided to send, and then wake one
    delivery task (`tokio::sync::Notify`; a sweep once a minute as a
    fallback). A sent mail is deleted; a temporary failure is retried after
    30 s, doubling up to 15 minutes; a permanent refusal (5xx, unusable
    address) or a day without success drops it. Delivery is therefore at
    least once: a mail in flight when the service stops is sent again.
  - **Encryption is required to any remote relay:** `starttls` (default;
    the upgrade is required, nothing is sent without it) or `tls`;
    `none` only to a loopback host. A setting error stops `serve` at
    startup.
  - **Logs carry no personal data:** the row id and the kind of failure
    (SMTP code, timeout, TLS, connection), never the address, subject, body
    or the relay's reply text. Only the operator's `send-test-mail` prints
    the relay's full answer.
- **Consequences:** 62 more packages in the service's lock file (81 →
  143; many are Windows-only platform crates that a Linux build does not
  compile: `lettre`, `rustls` and its tree, `hostname`, email encoding);
  none in the desktop app. The relay's password lives on the host, in a root-only
  environment file. A queued mail holds its code in plaintext until sent.
  Tested only against an SMTP server in the test process, without TLS or
  authentication; the first real check is `send-test-mail` against the
  chosen provider.

## ADR-045: The account service runs on one free Google Cloud VM, and builds from `main` use it

Status: Accepted (2026-11-17). Code: `PRODUCTION_SERVER` and
`configured_server` in `src-tauri/src/account/mod.rs`; operations:
[server-setup.md](../development/server-setup.md).

- **Context:** the service (ADR-032) could send mail since ADR-044 but ran
  nowhere, and release builds had no service (`PRODUCTION_SERVER = None`).
  The maintainer wanted it running at no recurring cost besides the
  domain. Oracle Cloud's Always Free tier, the first choice, had no
  capacity for either free shape in its EU home region on 2026-11-17.
- **Decision:**
  - **Hosting:** one Google Cloud free-tier e2-micro (2 shared vCPUs,
    1 GB, 30 GB standard disk) in `us-east1`, Ubuntu 24.04, the service
    under systemd as an unprivileged user, Caddy in front for HTTPS with
    Let's Encrypt and HSTS, Brevo's free plan as the SMTP relay, the domain
    `sotyware.com`. The e2-micro cannot compile the service, so it is
    built in WSL (the same Ubuntu and architecture) and copied.
  - **`PRODUCTION_SERVER` is `https://accounts.sotyware.com`.** Release
    and development builds use it; development builds can still point
    `SOTYWARE_ACCOUNT_SERVER` elsewhere.
  - **Test-profile builds never use it.** Without `SOTYWARE_ACCOUNT_SERVER`
    a test-profile build has no service, so a desktop test cannot reach
    the users' accounts (the rule of ADR-027 for their data).
- **Consequences:** account data (email, device names, encrypted vaults)
  are stored in the US; the privacy notice must say so. One VM, no
  redundancy or monitoring; its backups stay on it until off-host copies
  exist. The VM's address is ephemeral: recreating the VM means a new DNS
  record and a new authorised IP at Brevo. Free-plan limits: 300 mails a
  day, about 1 GB of outgoing traffic a month. `v0.1.0` and `v0.1.1` have
  no service; the next release will be the first to offer accounts, so a
  privacy notice and an external review remain gates before it.
