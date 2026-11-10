# AGENTS.md

Canonical instructions for every coding agent (and human) working on sotyware.
Tool-specific files such as `CLAUDE.md` point here and must not add competing
rules. Start each task by reading [docs/CONTEXT.md](docs/CONTEXT.md).

## Project identity

- **Name:** `sotyware`, always lowercase, also at the start of a sentence and in UI
  text. Bundle identifier `com.sotyware.desktop`; Cargo package and binary
  `sotyware`; Rust library `sotyware_lib` ([why](docs/architecture/decisions.md#adr-007-library-crate-named-sotyware_lib)).
- **Purpose:** a cross-platform desktop app for Windows 10/11 and Linux that runs
  continuously in the background, stays lightweight, lives in the system tray and
  opens a dashboard window on demand.
- **Current scope:** only the desktop runtime (window, tray, lifecycle, single
  instance). No product features exist yet. Current milestone and status:
  [docs/status/status.md](docs/status/status.md).

### Terminology

| Term | Meaning |
| --- | --- |
| dashboard | the one webview window, label `main` (`DASHBOARD_LABEL`) |
| tray | the system tray icon and its menu |
| hide | make the dashboard invisible; process keeps running |
| close | the window manager's close (✕, Alt+F4); sotyware turns it into hide |
| quit | end the process; only the tray "Quit" does this |
| open / show | `show_dashboard()`: un-minimize, show, focus the existing window |
| primary instance / second launch | the running process / a later start that hands over and exits |

Use these words consistently in code, UI, commits and docs. Docs use British
spelling ("behaviour").

## Technology stack

Declared ranges are in the manifests; resolved versions are from the lockfiles
(`package-lock.json`, `src-tauri/Cargo.lock`) as of 2026-11-10.

| Component | Declared | Resolved |
| --- | --- | --- |
| Tauri (Rust crate, feature `tray-icon`) | `2` | 2.12.5 |
| tauri-build | `2` | 2.7.4 |
| tauri-plugin-single-instance (desktop targets only) | `2` | 2.5.5 |
| Rust | edition 2024, `rust-version = "1.90"` | developed with 1.99.1 |
| `@tauri-apps/api` (only runtime JS dependency) | `^2.12.4` | 2.12.4 |
| `@tauri-apps/cli` | `^2.12.4` | 2.12.4 |
| Svelte | `^5.57.3` | 5.57.3 |
| `@sveltejs/vite-plugin-svelte` | `^7.3.1` | 7.3.1 |
| Vitest (frontend tests) | `^4.1.12` | 4.1.12 |
| happy-dom (DOM for Vitest) | `^21.0.4` | 21.0.4 |
| svelte-check | `^4.7.8` | 4.7.8 |
| TypeScript | `~6.0.3` | 6.0.3 |
| Vite | `^8.3.7` | 8.3.7 |
| Node.js | `^20.19 \|\| ^22.12 \|\| >=24` (dependency `engines`) | developed with 24.22.0 |
| Package manager | npm (lockfile v3) | developed with 11.19.1 |
| `tray-icon` (through Tauri; Linux-only direct dependency adds feature `ksni`) | `0.25` | 0.25.1 |
| `ksni` (Linux tray backend, pure Rust over `zbus` 5.19.0) | through `tray-icon` | 0.3.7 |
| Linux webview / tray | WebKitGTK 4.1 / StatusNotifierItem via `ksni` (libayatana-appindicator3 is not loaded) | system packages / crate |
| Windows webview | WebView2 | system |

Re-check this table against the lockfiles when you change dependencies.

## Architecture

Implemented today (details in [docs/architecture/](docs/architecture/overview.md)):

- **Entry point:** `src-tauri/src/main.rs` → `sotyware_lib::run()` in
  `src-tauri/src/lib.rs`.
- **Runtime:** `run()` builds the Tauri app: single-instance plugin (first, desktop
  only) → window `main` from `tauri.conf.json` → `setup` creates the tray → event
  loop.
- **Window lifecycle:** close is turned into hide; implicit exits
  (`ExitRequested { code: None }`) are blocked; only `exit(code)` ends the process.
- **Tray:** `src-tauri/src/tray.rs`, "Open Dashboard" → `show_dashboard()`,
  "Quit" → `exit(0)`. Tray creation failure → `exit(1)`.
- **Single instance:** second launch hands over to the primary, which calls
  `show_dashboard()`; the new process exits 0.
- **Frontend:** `src/App.svelte`, a static page. Its only access to Rust is
  `getCurrentWindow().hide()`, allowed by `core:window:allow-hide` in
  `src-tauri/capabilities/default.json`. No custom Tauri commands or events.

**Not implemented (planned or only discussed):** everything in
[docs/status/roadmap.md](docs/status/roadmap.md). In particular there is no
persistence, settings, autostart, feature/plugin system, sync, backend service,
custom command or custom event. Never describe these as existing.

## Coding conventions

Follow what the code already does. When in doubt, read `lib.rs` and `tray.rs`.

### Rust

- Default `rustfmt`; `cargo clippy --all-targets -- -D warnings` must pass.
- Module docs with `//!`, public items with `///`. Comments explain *why*
  (platform quirks, ordering constraints), not *what*.
- Identifiers that must match elsewhere (window label, menu ids, tray id) are
  `const`s, never repeated string literals. `DASHBOARD_LABEL` must match the label
  in `tauri.conf.json`.
- Keep testable logic in plain functions/types (like `TrayAction::from_menu_id`)
  and unit-test them in a `#[cfg(test)] mod tests` in the same file.
- Keep lifecycle policy in `lib.rs`; other modules call `show_dashboard()` or
  `AppHandle::exit` instead of re-implementing it.
- Register handlers once, on the builder or in `setup`. No polling loops or timers.

### Error handling

- Fallible setup code returns `tauri::Result` and uses `?`.
- A failing window operation that leaves the app usable is logged and the flow
  continues (`if let Err(e) = window.show() { eprintln!(...) }`).
- A failure that would leave the app unreachable is fatal: log it and
  `app.handle().exit(1)` (as for the tray).
- `expect` only where the app cannot start at all (`Builder::build`). No `unwrap`
  in runtime paths.
- Frontend: catch errors from Tauri API calls and show them in the UI (see
  `hideDashboard` in `App.svelte`).

### Logging

- `eprintln!` with the prefix `sotyware: ` and a lowercase message, e.g.
  `eprintln!("sotyware: failed to show dashboard: {e}")`.
- There is no logging crate or `tracing` subscriber. Adding one is an
  architectural decision (record it in
  [decisions.md](docs/architecture/decisions.md)).

### TypeScript

- Strict settings in `tsconfig.json` (`strict`, `noUncheckedIndexedAccess`,
  `exactOptionalPropertyTypes`, no unused locals/parameters). Do not relax them.
- Explicit return types on functions (`async function hideDashboard(): Promise<void>`).
- Fail loudly on broken invariants (`main.ts` throws if `#app` is missing).
- `npm run check` must report 0 errors and 0 warnings.
- Frontend tests live next to the code as `src/**/*.test.ts` (Vitest, happy-dom)
  and mock Tauri with `@tauri-apps/api/mocks`, never by stubbing `App.svelte`.

### Svelte

- Svelte 5 runes (`$state`, …) and event attributes (`onclick={…}`), not Svelte 4
  syntax (`on:click`, `export let`, stores for local state).
- `<script lang="ts">`; component-scoped `<style>`; `:global(...)` only for
  document-level styles.
- Support dark mode via `prefers-color-scheme`. Use semantic HTML (`type="button"`,
  `role="alert"` for errors).

### Dependencies

- Add a dependency only when the platform or existing code cannot reasonably do
  the job. Prefer official Tauri plugins over custom platform code. Record new
  runtime dependencies in [decisions.md](docs/architecture/decisions.md).
- Desktop-only Rust crates go in the
  `[target.'cfg(any(target_os = "macos", windows, target_os = "linux"))'.dependencies]`
  section and their use behind `#[cfg(desktop)]`.
- A new Tauri plugin or frontend API call needs the narrowest capability
  permission in `src-tauri/capabilities/default.json`.
- Commit `package-lock.json` and `Cargo.lock` changes together with the manifest
  change. Install with `npm ci` unless you are changing dependencies.
- Keep versions in `package.json`, `src-tauri/Cargo.toml` and
  `src-tauri/tauri.conf.json` identical.

### Naming

- Rust: `snake_case` functions/modules, `CamelCase` types, `SCREAMING_SNAKE_CASE`
  consts. TypeScript: `camelCase`; Svelte components `PascalCase.svelte`.
- Tauri ids and labels: lowercase kebab-case (`open-dashboard`, `sotyware-tray`),
  window labels lowercase (`main`).
- UI labels: Title Case for menu items ("Open Dashboard"), sentence case for
  buttons ("Hide dashboard").

### Testing

See [docs/development/testing.md](docs/development/testing.md). Pure logic gets
unit tests; runtime behaviour changes need the desktop test and the manual
checklist updated.

### Cross-platform code

See [docs/development/cross-platform.md](docs/development/cross-platform.md).
Windows and Linux behaviour must both be considered for every change. State in the
commit message on which platforms a change was actually run.

## Agent behaviour

Every coding agent must:

1. Inspect the relevant code before modifying it. Read the code, not only the docs.
2. Understand existing abstractions (`show_dashboard()`, `AppControl`,
   `TrayAction`, the capability file) before introducing new ones.
3. Avoid unnecessary dependencies.
4. Avoid speculative abstractions and premature architecture. Build what the task
   needs, nothing for hypothetical features.
5. Preserve cross-platform behaviour.
6. Never weaken, skip or delete tests or checks to make a change pass.
7. Update documentation in the same change when behaviour or architectural
   decisions change (see [Documentation rules](#documentation-rules)).
8. Run the relevant checks before committing
   ([commands](docs/development/workflow.md#checks-run-before-every-commit-that-touches-code)).
9. Commit every logically complete step.
10. Never push without explicit approval.
11. Never overwrite, discard or commit unrelated user changes. Run `git status`
    and `git diff` before starting and before committing.
12. Report what was actually tested, on which environment, versus what remains
    unverified.

## Git workflow

- [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`,
  `docs:`, `test:`, `build:`, `refactor:`, `chore:`. Subject in the imperative,
  lowercase after the type. The body explains why and what was verified.
- One logical change per commit; validate it before committing.
- Never amend, reset, rebase, force-push or otherwise rewrite existing commits
  without explicit authorization.
- Never push to a remote without explicit approval.
- Stage files explicitly; never commit build output (`dist/`, `src-tauri/target/`,
  `src-tauri/gen/`) or `node_modules/`.

## Documentation rules

- **The repository is the only source of truth.** No Notion or other external
  documentation dependency
  ([ADR-008](docs/architecture/decisions.md#adr-008-the-repository-is-the-only-source-of-truth-for-documentation)).
- **One home per fact.** Write details once and link to them. Where things belong:

  | Change | Update |
  | --- | --- |
  | Behaviour, lifecycle, tray, single instance | `docs/architecture/` (and `docs/CONTEXT.md` if the summary changes) |
  | Significant design decision | new entry in `docs/architecture/decisions.md`, with rationale |
  | Files moved, added or renamed | `docs/architecture/repository-map.md` |
  | Commands, setup, tooling | `docs/development/` |
  | Something implemented or validated | `docs/status/status.md` |
  | Limitation found or fixed | `docs/status/limitations.md` |
  | Plan or next step | `docs/status/roadmap.md` |
  | Dependency versions | the stack table above |
  | Agent rules | this file only |

- **Planned is not implemented.** Roadmap items stay in `roadmap.md` until they
  are done. Validation is recorded only for environments where it actually ran,
  with the date.
- **Remove outdated documentation only after checking** that anything still true
  is preserved elsewhere.
- Check that relative links resolve after editing docs.
