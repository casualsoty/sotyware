# Built-in features

How built-in features are defined, registered, started, stopped and shown to
the frontend. The code is in `src-tauri/src/features/`, and why it is built
this way is in
[ADR-014](decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry).
Only **built-in** features exist: native Rust modules compiled into sotyware.
The user's enable/disable choice is saved and survives restarts
([Persistence](#persistence), [ADR-015](decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait)).
Third-party plugins, WASM and a management UI do not exist
([what is deferred](#deferred)).

Today there are three features: `example`, which does nothing visible (it
demonstrates the lifecycle and gives the dashboard and the desktop tests a
feature to list and switch), `backup-scheduler` ("Scheduled backups",
on by default, 2026-11-15, ADR-031), which starts and stops the backup
scheduler of `crate::backup` (see [backup.md](backup.md#scheduling)), and
`update-check` ("Automatic update checks", on by default, 2026-11-15,
ADR-033), which starts and stops the automatic update checks of
`crate::updates` (one minute after startup, then daily; nothing when no
update source is configured; see [updates.md](updates.md#flow)).

## Pieces

| Piece | File | Role |
| --- | --- | --- |
| `Feature` trait | `features/mod.rs` | What a feature implements: `metadata()`, `start()`, `stop()` |
| `FeatureMetadata` | `features/mod.rs` | `id`, `name`, `description`, `version`, `enabled_by_default`; all `&'static str`/`bool`, fixed at compile time |
| `builtin()` | `features/mod.rs` | The one list of built-in features, in registration order |
| `FeatureRegistry` | `features/registry.rs` | Owns the features, validates registration, drives the lifecycle, isolates errors. Knows nothing about storage |
| `FeaturePreferences`, `PreferenceStore` | `features/preferences.rs` | The stored choices: decides at startup whether a feature runs (`enabled_at_startup`: stored choice, else the default) and saves each switch. `PreferenceStore` is the two-method storage trait |
| `SqliteStore` | `storage.rs` | The `PreferenceStore` implementation: `preferences.sqlite3`, schema and migrations |
| `list_features`, `set_feature_enabled` | `features/mod.rs` | The frontend's only access to features |
| `Example` | `features/example.rs` | The example feature |
| `BackupScheduler` | `features/backup.rs` | Starts and stops the backup scheduler through `backup::SCHEDULER_LINK`, set in `run()` before the event loop (without it, as in unit tests, start and stop do nothing) |
| `src/features.ts`, `src/Features.svelte` | frontend | Typed command wrappers that validate responses; the minimal Features section on the dashboard |

## Registration

`run()` builds the registry once, before the app starts:
`FeatureRegistry::with_features(features::builtin())`, managed as Tauri state.
Registration:

- **Deterministic:** features are registered, started and listed in the order of
  `builtin()`. They are stopped in reverse order.
- **Validated:** the ID must be lowercase kebab-case (`example`,
  `clipboard-history`), and name, description and version must not be empty.
  A **duplicate ID is rejected** and the first feature with that ID is kept.
  A rejected feature is logged (`sotyware: feature not registered: …`) and
  skipped. The others are registered as usual. The unit test
  `builtin_features_register_without_errors` makes a bad built-in list fail
  `cargo test`, so this never happens in a release.
- **Fixed afterwards:** `register` needs `&mut self`. Once the registry is Tauri
  state it is shared as `&self`, so no feature can be added or removed while the
  app runs. There is no global or static registry.

## Lifecycle

| State | Meaning | `enabled` |
| --- | --- | --- |
| `registered` | Registered; startup has not decided yet | false |
| `disabled` | Not running, by choice or default | false |
| `initializing` | `start()` is running | true |
| `running` | `start()` succeeded | true |
| `failed` | `start()` returned an error; nothing of the feature runs | true |
| `stopping` | `stop()` is running | false |

`enabled` is the user's intent ("on"), so a failed feature counts as enabled.
Every transition goes through one table, `plan()` in `registry.rs`:

| From | Enable | Disable |
| --- | --- | --- |
| `registered` | `initializing` → `start()` → `running` / `failed` | `disabled` (no call) |
| `disabled` | `initializing` → `start()` → `running` / `failed` | invalid |
| `failed` | retry: `initializing` → `start()` → `running` / `failed` | `disabled` (no call, error cleared) |
| `running` | invalid | `stopping` → `stop()` → `disabled` |
| `initializing`, `stopping` | invalid (busy) | invalid (busy) |

An invalid transition calls nothing and returns
`ControlError::InvalidTransition` (to the frontend:
`cannot enable feature 'x' while it is Running`). An unknown ID returns
`unknown feature 'x'`.

When it happens:

- **Startup:** the stored preferences are loaded after `build()`, before the
  event loop runs ([Persistence](#persistence)). In `setup`, after the operating
  mode is chosen (in both modes),
  `start_all(|m| prefs.enabled_at_startup(m))` enables or disables every feature
  that is still `registered`, in order: the stored choice, or
  `enabled_by_default` if there is none. A disabled feature is never started. If
  the tray error is fatal (exit 1), no feature is started.
- **At runtime:** only `set_feature_enabled` from the frontend, which saves the
  choice afterwards.
- **Exit:** `RunEvent::Exit` (after an explicit `exit(code)`: tray Quit, close
  in dashboard-only mode, fatal tray error) calls `stop_all()`, which stops
  every `running` feature in reverse order without changing any stored choice,
  then `FeaturePreferences::close()`. Tauri 2.12.5 delivers `Exit` to the
  `app.run` callback before its own cleanup (`App::run`, `RuntimeRunEvent::Exit`).

## Errors and isolation

- **`start()` fails:** the feature becomes `failed` and the message is stored as
  its `error` and logged (`sotyware: feature 'x' failed to start: …`). By
  contract, a failing `start()` has already released what it acquired, so
  `stop()` is not called for it. The next feature is still started.
- **`stop()` fails:** the feature becomes `disabled` with
  `error: "stop failed: …"`, which is also logged. The registry treats it as
  stopped. A failing `stop()` during `stop_all()` does not keep the others from
  stopping.
- The next successful transition clears `error`.
- **One feature never blocks another.** Each feature has its own two locks: a
  status lock, held only to check and claim a transition, and a feature lock,
  held while `start()`/`stop()` runs. A slow `start()` keeps only that feature
  busy (`initializing`). The list stays readable and other features can still
  be switched. A second request for a busy feature is rejected, not queued.
- **Panics are not isolated.** Release builds use `panic = "abort"`, so a
  panicking feature ends the process. Return an error instead. (In debug builds
  a poisoned lock is recovered, so tests can still read the state.)
- Registry errors never touch the operating mode, the tray or the window.

## Threading

- `start()` and `stop()` are called synchronously: at startup and exit on the
  main thread, and from `set_feature_enabled` on Tauri's blocking thread pool
  (`#[tauri::command(async)]`), never on the event loop. They must return
  promptly. The preference save runs on that same pool thread, after the
  switch. Long-running work goes on a thread or task that the feature spawns
  in `start()` and joins in `stop()`.
- No polling or timers. A feature waits on events, channels or OS
  notifications.
- `Feature: Send` is required because the registry is shared state.

## Frontend

| Command | Permission | Returns |
| --- | --- | --- |
| `list_features` | `allow-list-features` | `{ features, storageError }`: every `FeatureInfo`, in registration order, and why the stored choices could not be loaded (`null` if they were) |
| `set_feature_enabled { id, enabled }` | `allow-set-feature-enabled` | `{ feature, saveError }`: the feature's new `FeatureInfo`, and why the choice could not be saved (`null` if it was); rejects only for an unknown ID or invalid transition, and then nothing changed or was saved |

`FeatureInfo` is `{ id, name, description, version, state, enabled, error }`,
with `state` in kebab-case and `error` a string or `null`. A failing start is a
`failed` state, not a rejection. A failed save is a `saveError`, not a
rejection: the switch did happen. Both commands are declared in `build.rs` and
granted to window `main` by the static capability `default`. There are no
events: the page reads the list on mount and replaces a feature with the
response of each switch. A change made elsewhere (only startup, today) is not
pushed. `src/features.ts` validates every response and throws on a mismatch.
`src/Features.svelte` shows each feature's name, version, state, description and
last error, with a checkbox to switch it. While a feature is busy or a request is
in flight, the checkbox is disabled. A `storageError` is shown as an alert
("Could not load the saved feature choices: …. Every feature uses its default,
and changes are not saved."), and so is a `saveError` ("Could not save the
choice for Example: …. It stays disabled until sotyware quits; the next start
uses the last saved choice or the default."), with the checkbox showing the
feature's real, new state.

A feature that needs its own frontend call adds a `#[tauri::command]` in its
module, registers it in `generate_handler!` in `run()`, declares it in
`build.rs` and grants `allow-<command>` in the capability. This is a new
permission, so it gets an ADR. Such a command must check that the feature is
`running`. No feature has a command yet, so there is no helper for that.

## Persistence

The user's choice for each feature survives restarts. Why it is built this
way:
[ADR-015](decisions.md#adr-015-feature-preferences-are-stored-in-sqlite-behind-a-narrow-store-trait).

### Layers

| Layer | Knows | Does not know |
| --- | --- | --- |
| `FeatureRegistry` (`registry.rs`) | features and their states | that anything is stored |
| `FeaturePreferences` (`preferences.rs`) | the policy below; reads the registry's state to save it | SQL, files, paths |
| `PreferenceStore` trait | `load() -> [(id, enabled)]`, `save(id, enabled)` | |
| `SqliteStore` (`storage.rs`) | the database file, schema, migrations | what a value means |

`run()` (`lib.rs`) wires them: it resolves the folder, opens the store, loads
the preferences and manages `FeaturePreferences` as Tauri state.

### Database

- **File:** `preferences.sqlite3` in Tauri's `app_local_data_dir()`:
  - Linux: `$XDG_DATA_HOME/com.sotyware.desktop/`, normally
    `~/.local/share/com.sotyware.desktop/`.
  - Windows: `%LOCALAPPDATA%\com.sotyware.desktop\` (local, not roaming). The
    webview's own data (`EBWebView`) is in the same folder.
  The folder is created if needed. Default rollback journal (no WAL), so the
  only other file is a short-lived `preferences.sqlite3-journal` during a write.
- **Schema (version 1):**

  ```sql
  CREATE TABLE feature_preferences (
      feature_id TEXT PRIMARY KEY NOT NULL,
      enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
  ) STRICT;
  ```

  `feature_id` is the feature's stable `FeatureMetadata::id`, which is why an ID
  must never change once released. A row exists only for a feature the user has
  switched; there is no row for a feature left at its default.
- **Versioning:** `PRAGMA user_version` is the schema version (0 for a new,
  empty file). `storage::MIGRATIONS[n]` takes the schema from version `n` to
  `n + 1`.
- **Migration:** on open, if the version is behind, every pending step and the
  new version run in one `BEGIN IMMEDIATE` transaction: all or nothing. A
  version newer than this sotyware knows is refused and the file is not
  changed (no downgrade). To change the schema, append a step (never edit a
  released one) and add a test that migrates a database of the previous
  version with rows in it.
- **Checks on open:** `PRAGMA quick_check` must answer `ok`, so a file that is
  not a database, or has damaged pages, is refused.

### When it is read and written

- **Read once,** after `build()` and before `run()`: open, check, migrate,
  load. That is on the main thread before the event loop starts, so no database
  I/O runs on the event loop. A second launch hands over and exits inside
  `build()`, so only the primary instance opens the file.
- **Written** by `set_feature_enabled`, on Tauri's blocking thread pool, after
  a switch the registry accepted: one `INSERT … ON CONFLICT DO UPDATE`, which
  SQLite runs as its own transaction and which is durable when it returns. A
  rejected switch (unknown ID, invalid transition) writes nothing. Startup and
  exit never write.
- **What is written** is the feature's `enabled` in the registry at save time,
  read under the same lock as the write. So whichever of two concurrent saves
  runs last stores the state after the last change.

### Behaviour

| Situation | Result |
| --- | --- |
| First launch, no database | File and table created; every feature uses `enabled_by_default` |
| Stored choice exists | Used instead of the default, both ways (a default-on feature can stay off, a default-off feature can stay on) |
| New feature, no stored choice | Its `enabled_by_default` |
| Stored ID that no built-in feature has | Ignored, logged (`ignoring stored preference for unknown feature 'x'`), kept in the file |
| Feature fails to start | Its intent is on, so "enabled" is saved and the next start retries; the user disables it to stop that |
| Database older schema | Migrated in one transaction, rows kept |
| Database newer schema, not a database, damaged, cannot be opened, migration fails | File left untouched; features use their defaults; logged (`feature preferences unavailable, using defaults: …`); `storageError` shown on the dashboard; every save fails with that reason until a restart. Recovery is manual: quit and move the file away |
| Data folder cannot be created | Tauri cannot create the webview data folder either and fails its own setup: sotyware does not start (this was already so before persistence) |
| Save fails (disk full, read-only file, …) | The switch stays for this session; the stored value is the old one; `saveError` shown; logged (`failed to save the preference for feature 'x': …`). After a restart: the last saved choice or the default |
| Exit (tray Quit, close in dashboard-only mode, exit 1) | Features stop; no choice changes; a save in progress finishes first, later ones fail with "sotyware is quitting" |
| Process killed mid-save | SQLite rolls the unfinished write back on the next open; earlier saves are intact |
| Two processes (single instance off without a D-Bus session bus) | SQLite's file locking serializes them; no corruption, last write wins. A save that cannot get the lock within the 2 s busy timeout fails with "database is locked" and changes nothing, handled like any failed save above |

## Adding a built-in feature

1. Create `src-tauri/src/features/<name>.rs` with a type that implements
   `Feature`. Pick a lowercase kebab-case `id` and never change it once
   released. Make `start()` clean up after itself on error, and `stop()` release
   everything `start()` acquired (join threads, close handles).
2. Add `mod <name>;` and one `Box::new(...)` line to `builtin()` in
   `features/mod.rs`. Position in the list is start order.
3. Unit-test the feature in its own file: start, stop, and its error paths. The
   existing `builtin_features_*` tests check registration and a clean
   start/stop of every built-in feature automatically.
4. If the frontend needs more than enable/disable, add a command as described
   in [Frontend](#frontend), with an ADR.
5. Update [status.md](../status/status.md) and, if the feature changes what the
   desktop tests see, the tests.

Nothing else changes: no edits to `lib.rs`, the capability or the frontend for
a feature without its own command.

## Deferred

These are intentionally not built: events from Rust to the page (state changes are seen only by the page that made
them); a way for a running feature to report a later failure (it stays
`running`); feature dependencies or ordering beyond the list; a feature context
(for example an `AppHandle`) passed to `start()`, to be added when a feature
needs one; a per-feature command guard; a feature-management UI; third-party
plugins, WASM and hot reloading; automatic recovery from a corrupt preferences
database; sync of preferences between devices. See
[roadmap.md](../status/roadmap.md).
