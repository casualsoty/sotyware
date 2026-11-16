# sotyware

sotyware is a desktop application meant to run continuously in the
background while staying lightweight. It lives in the system tray and opens a
dashboard window on demand.

**Platforms.** The releases target **Windows 11** only, and only once their
installers have been validated (not done yet; see
[docs/development/release.md](docs/development/release.md)). **Windows 10 is not
validated** and is not claimed as supported. The code also builds and runs on
**Linux**, but it has been exercised only in simulated environments (WSL2,
virtual displays), never on a real Linux desktop, so Linux is not a supported
release target yet. On Linux without a tray host sotyware runs as a plain
window instead (dashboard-only mode: no hiding, closing quits). **macOS**
(Apple Silicon and Intel) is a build target with a menu bar and the
Keychain, built and tested only on GitHub's macOS runners, not yet on a Mac
anyone used, and not signed ([docs/development/macos.md](docs/development/macos.md)).

**What it does today:**

- The desktop runtime: one dashboard window, a tray icon with "Open Dashboard"
  and "Quit", hide-on-close, single-instance handling.
- A desktop dashboard: a sidebar with the sections and your storage
  accounts, an Overview, Settings (light, dark or system theme; features;
  about and updates), keyboard shortcuts. On Windows it draws its own title
  bar (native moving, snapping and resizing kept); it opens at half your
  primary screen (at least 900 × 540), centred, then where you left it.
- Built-in features: native Rust modules that can be enabled and disabled from
  the dashboard (scheduled backups, automatic update checks); each
  choice is saved locally (SQLite, in the app's data folder, private to the
  user).
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
  - favorites and recent items per account;
  - after a crash, a list of interrupted work, inspected at the provider
    before you decide what to do, and a list of unfinished large uploads;
  - **usage and costs** per bucket: measured by listing the bucket (files,
    older versions, unfinished uploads; no provider reports this through
    its API), with a history of sotyware's measurements and a 30-day cost
    estimate from the providers' published prices (Backblaze B2, Hetzner),
    always labelled as an estimate, never a bill;
  - **snapshots and backups** per bucket: what Backblaze's own snapshots are
    (made only in its web console; sotyware cannot manage them), and
    sotyware's backup snapshots of the bucket: details, back up now,
    verify, delete one (after typing `delete`).
- **Backups**: jobs that keep an independent, verified, versioned copy of a
  bucket (or folder) in a second account, ideally at another provider:
  scheduled or manual runs that transfer only changes, verification, a
  retention policy that never prunes after a suspicious mass change,
  restores that never overwrite, a recovery file, and a restore tool that
  needs only Python ([docs/user/recovery.md](docs/user/recovery.md)).
- **Optional sotyware account** with end-to-end encrypted sync of storage
  accounts (keys included) and backup jobs between computers: the service
  only ever stores ciphertext; a recovery key shown once, device approval,
  revocation and key rotation; an enrolled computer syncs at startup, when
  focused and after you change a storage account, and shows its sync state.
  **Not available to users yet:** the service
  (`server/`) is not deployed and release builds have none configured.
- **Signed updates from GitHub Releases** of this repository,
  installed only when you click "Update" and never during a transfer or
  backup. `v0.1.0` was published on 2026-11-15 and `v0.1.1` on
  2026-11-16; no update has been installed yet (moving from 0.1.0 to
  0.1.1 will be the first).

It does not search file contents, download as ZIP, keep a trash of its own or
permanently delete old object versions, and it does not copy objects larger
than 5 GiB. Providers differ (versioning, link limits, key restrictions), so
not every action behaves the same at every provider; file management,
recovery and backups have so far been exercised only against local test
servers, not at a real provider, and accounts and updates only in tests
with an in-process service and local update server.
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
| [docs/development/](docs/development/workflow.md) | [Setup](docs/development/setup.md), [commands](docs/development/workflow.md), [testing](docs/development/testing.md), [cross-platform rules](docs/development/cross-platform.md), [release builds, signing and installer testing](docs/development/release.md), [application icons](docs/development/icons.md) |
| [docs/status/](docs/status/status.md) | [Status and validation matrix](docs/status/status.md), [known limitations](docs/status/limitations.md), [roadmap](docs/status/roadmap.md), [production-readiness journal](docs/status/journal.md) |
| [docs/user/recovery.md](docs/user/recovery.md) | Recovering files from a backup, with or without sotyware |
| [AGENTS.md](AGENTS.md) | Conventions and rules for contributors and coding agents |
