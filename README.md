# sotyware

sotyware is a cross-platform desktop application for Windows 10/11 and Linux,
meant to run continuously in the background while staying lightweight. It lives
in the system tray and opens a dashboard window on demand.

**Current state:** the minimal desktop runtime exists: one dashboard window,
a system tray icon with "Open Dashboard" and "Quit", hide-on-close, and
single-instance handling. On Linux without a tray host, sotyware runs as a plain
window instead (dashboard-only mode: no hiding, closing quits). The foundation
for built-in features (native Rust modules that can be enabled and disabled
from the dashboard) exists with one example feature, and each enable/disable
choice is saved locally (SQLite, in the app's data folder) and survives
restarts; no product features exist yet. It has been validated
natively on Windows 11 (automated desktop test) and in a simulated Linux
environment (WSL2); Windows 10 and real Linux desktops are not yet validated.
See [docs/status/status.md](docs/status/status.md).

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

Build a release with `npm run tauri build`. All commands and checks:
[docs/development/workflow.md](docs/development/workflow.md).

## Documentation

| Document | Contents |
| --- | --- |
| [docs/CONTEXT.md](docs/CONTEXT.md) | One-page summary of the project's current state |
| [docs/architecture/](docs/architecture/overview.md) | Architecture, [repository map](docs/architecture/repository-map.md), [lifecycle and tray](docs/architecture/lifecycle.md), [built-in features](docs/architecture/features.md), [single instance](docs/architecture/single-instance.md), [decisions](docs/architecture/decisions.md) |
| [docs/development/](docs/development/workflow.md) | [Setup](docs/development/setup.md), [commands](docs/development/workflow.md), [testing](docs/development/testing.md), [cross-platform rules](docs/development/cross-platform.md) |
| [docs/status/](docs/status/status.md) | [Status and validation matrix](docs/status/status.md), [known limitations](docs/status/limitations.md), [roadmap](docs/status/roadmap.md) |
| [AGENTS.md](AGENTS.md) | Conventions and rules for contributors and coding agents |
