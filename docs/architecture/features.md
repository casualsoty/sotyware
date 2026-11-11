# Built-in features

How built-in features are defined, registered, started, stopped and shown to
the frontend. The code is in `src-tauri/src/features/`, and why it is built
this way is in
[ADR-014](decisions.md#adr-014-built-in-features-are-rust-modules-in-a-fixed-registry).
Only **built-in** features exist: native Rust modules compiled into sotyware.
Third-party plugins, WASM, persistence of the user's choices and a management UI
do not exist ([what is deferred](#deferred)).

Today there is one feature, `example`, which does nothing visible. It is there
to demonstrate the lifecycle and to give the dashboard and the desktop tests a
feature to list and switch.

## Pieces

| Piece | File | Role |
| --- | --- | --- |
| `Feature` trait | `features/mod.rs` | What a feature implements: `metadata()`, `start()`, `stop()` |
| `FeatureMetadata` | `features/mod.rs` | `id`, `name`, `description`, `version`, `enabled_by_default`; all `&'static str`/`bool`, fixed at compile time |
| `builtin()` | `features/mod.rs` | The one list of built-in features, in registration order |
| `enabled_at_startup()` | `features/mod.rs` | Decides at startup whether a feature runs. Today it returns the default. Persistence plugs in here |
| `FeatureRegistry` | `features/registry.rs` | Owns the features, validates registration, drives the lifecycle, isolates errors |
| `list_features`, `set_feature_enabled` | `features/mod.rs` | The frontend's only access to features |
| `Example` | `features/example.rs` | The example feature |
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

- **Startup:** in `setup`, after the operating mode is chosen (in both modes),
  `start_all(enabled_at_startup)` enables or disables every feature that is
  still `registered`, in order. A disabled feature is never started. If the tray
  error is fatal (exit 1), no feature is started.
- **At runtime:** only `set_feature_enabled` from the frontend.
- **Exit:** `RunEvent::Exit` (after an explicit `exit(code)`: tray Quit, close
  in dashboard-only mode, fatal tray error) calls `stop_all()`, which stops
  every `running` feature in reverse order. Tauri 2.12.5 delivers `Exit` to the
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
  promptly. Long-running work goes on a thread or task that the feature spawns
  in `start()` and joins in `stop()`.
- No polling or timers. A feature waits on events, channels or OS
  notifications.
- `Feature: Send` is required because the registry is shared state.

## Frontend

| Command | Permission | Returns |
| --- | --- | --- |
| `list_features` | `allow-list-features` | every `FeatureInfo`, in registration order |
| `set_feature_enabled { id, enabled }` | `allow-set-feature-enabled` | the feature's new `FeatureInfo`; rejects only for an unknown ID or invalid transition |

`FeatureInfo` is `{ id, name, description, version, state, enabled, error }`,
with `state` in kebab-case and `error` a string or `null`. A failing start is a
`failed` state, not a rejection. Both commands are declared in `build.rs` and
granted to window `main` by the static capability `default`. There are no
events: the page reads the list on mount and replaces a feature with the
response of each switch. A change made elsewhere (only startup, today) is not
pushed. `src/features.ts` validates every response and throws on a mismatch.
`src/Features.svelte` shows each feature's name, version, state, description and
last error, with a checkbox to switch it. While a feature is busy or a request is
in flight, the checkbox is disabled.

A feature that needs its own frontend call adds a `#[tauri::command]` in its
module, registers it in `generate_handler!` in `run()`, declares it in
`build.rs` and grants `allow-<command>` in the capability. This is a new
permission, so it gets an ADR. Such a command must check that the feature is
`running`. No feature has a command yet, so there is no helper for that.

## Persistence (not implemented)

Nothing is stored. **On every start, each feature is enabled or disabled
according to its `enabled_by_default`**, so a feature the user disabled runs
again after a restart.

The registry was designed so storage can be added without changing it:

- At startup it asks only `enabled(&FeatureMetadata) -> bool`
  (`enabled_at_startup`). A future store answers with the saved choice and
  falls back to the default.
- After a successful `set_feature_enabled`, the command layer (not the registry)
  saves the new intent (`FeatureInfo::enabled`), keyed by the stable feature ID.
- The registry stays free of SQLite, files and settings types. What to store,
  where, and how to handle a stored ID that no longer exists is decided when
  persistence is designed.

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

These are intentionally not built: persistence of the enabled/disabled choice;
events from Rust to the page (state changes are seen only by the page that made
them); a way for a running feature to report a later failure (it stays
`running`); feature dependencies or ordering beyond the list; a feature context
(for example an `AppHandle`) passed to `start()`, to be added when a feature
needs one; a per-feature command guard; a feature-management UI; third-party
plugins, WASM and hot reloading. See [roadmap.md](../status/roadmap.md).
