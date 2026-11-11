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
