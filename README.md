# sotyware

sotyware is a desktop application meant to run continuously in the
background while staying lightweight. It lives in the system tray and opens a
dashboard window on demand.

**Platforms.** The first release targets **Windows 11** only, and only once its
installers have been validated (not done yet; see
[docs/development/release.md](docs/development/release.md)). **Windows 10 is not
validated** and is not claimed as supported. The code also builds and runs on
**Linux**, but it has been exercised only in simulated environments (WSL2,
virtual displays), never on a real Linux desktop, so Linux is not a supported
release target yet. On Linux without a tray host sotyware runs as a plain
window instead (dashboard-only mode: no hiding, closing quits).

**What it does today:**

- The desktop runtime: one dashboard window, a tray icon with "Open Dashboard"
  and "Quit", hide-on-close, single-instance handling.
- Built-in features: native Rust modules that can be enabled and disabled from
  the dashboard (one example feature); each choice is saved locally (SQLite,
  in the app's data folder).
- **Cloud Storage** for S3-compatible buckets (Backblaze B2, Hetzner or any
  S3-compatible service; one bucket per account, keys only in the OS
  credential store):
  - manage accounts (add, edit, check the connection, remove);
  - browse folders and objects;
  - upload files and folders (also by drag and drop), never overwriting an
    existing object;
  - download one file, or several files and folders into a chosen folder;
  - create folders; rename, copy, move and delete files and folders, each
    shown as a plan for review before anything changes;
  - search by name, sort, filter and select;
  - restore deleted files from "Deleted items", only on buckets with
    versioning (a delete there adds a delete marker; on other buckets it is
    final);
  - preview small text files and common raster images;
  - create temporary share links (1 hour, 1 day or 7 days);
  - favorites and recent items per account.

It does not search file contents, download as ZIP, keep a trash of its own or
permanently delete old object versions, and it does not copy objects larger
than 5 GiB. Providers differ (versioning, link limits, key restrictions), so
not every action behaves the same at every provider; file management has so
far been exercised only against local test servers, not at a real provider.
Details and evidence: [docs/status/status.md](docs/status/status.md) and
[docs/status/limitations.md](docs/status/limitations.md).

Built with [Tauri 2](https://v2.tauri.app/) (Rust) and Svelte 5 + TypeScript
(Vite).

## Quick start

Install the prerequisites in [docs/development/setup.md](docs/development/setup.md)
(Rust ≥ 1.90, Node.js 20.19+/22.12+/24+, and on Linux the WebKitGTK and
AppIndicator development packages), then:

```bash
npm ci
npm run tauri dev
```

Build a release with `npm run build:release` (not plain `tauri build`, which
leaves the build machine's paths in the binary and is refused when bundling;
[release.md](docs/development/release.md)). All commands and checks:
[docs/development/workflow.md](docs/development/workflow.md).

## Documentation

| Document | Contents |
| --- | --- |
| [docs/CONTEXT.md](docs/CONTEXT.md) | One-page summary of the project's current state |
| [docs/architecture/](docs/architecture/overview.md) | Architecture, [repository map](docs/architecture/repository-map.md), [lifecycle and tray](docs/architecture/lifecycle.md), [built-in features](docs/architecture/features.md), [single instance](docs/architecture/single-instance.md), [decisions](docs/architecture/decisions.md) |
| [docs/development/](docs/development/workflow.md) | [Setup](docs/development/setup.md), [commands](docs/development/workflow.md), [testing](docs/development/testing.md), [cross-platform rules](docs/development/cross-platform.md), [release builds, signing and installer testing](docs/development/release.md) |
| [docs/status/](docs/status/status.md) | [Status and validation matrix](docs/status/status.md), [known limitations](docs/status/limitations.md), [roadmap](docs/status/roadmap.md) |
| [AGENTS.md](AGENTS.md) | Conventions and rules for contributors and coding agents |
