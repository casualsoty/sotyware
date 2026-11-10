# Development environment setup

How to get a machine ready to build and run sotyware. For the commands you run
afterwards, see [workflow.md](workflow.md).

## Toolchain

| Tool    | Required                                    | Used for current development (WSL2, Ubuntu 24.04) |
| ------- | ------------------------------------------- | ------------------------------------------------- |
| Rust    | ≥ 1.90 (`rust-version` in `src-tauri/Cargo.toml`), stable, via [rustup](https://rustup.rs/) | 1.99.1 |
| Node.js | `^20.19 \|\| ^22.12 \|\| >=24` (the strictest `engines` range among the dev dependencies, from `@sveltejs/vite-plugin-svelte` and `vite`) | 24.22.0 (via nvm) |
| npm     | the one bundled with Node; `package-lock.json` is lockfile v3 | 11.19.1 |

npm is the only supported package manager (`package-lock.json` is committed).
There is no `.nvmrc` or `rust-toolchain.toml`.

## Linux system packages

Debian / Ubuntu (this exact list was used on Ubuntu 24.04):

```bash
sudo apt update
sudo apt install -y build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev
```

For Fedora, Arch, openSUSE and others, use the package lists in the official
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/#linux). They have
not been tested for sotyware.

The simulated desktop test ([testing.md](testing.md)) also needs:

```bash
sudo apt install -y python3-dbus python3-gi dbus-x11
```

At runtime, end users need WebKitGTK 4.1, GTK 3 and libayatana-appindicator3. The
`.deb` declares these as dependencies (verified). The `.rpm` dependencies have not
been inspected. The AppImage bundles the libraries.

## Windows 10 / 11

Not yet exercised for sotyware. Requirements according to Tauri:

- **Microsoft C++ Build Tools** (or Visual Studio) with the *Desktop development
  with C++* workload.
- **Rust** with the MSVC toolchain (`rustup default stable-msvc`).
- **WebView2 runtime.** It ships with Windows 11 and is installed on up-to-date
  Windows 10 systems. The installers Tauri produces download it if missing
  (Tauri's default `downloadBootstrapper` mode).

Build Windows binaries on Windows itself, from a normal Windows path. Do not build
from a WSL path (`\\wsl.localhost\...`).

## WSL2 notes

These apply to the machine sotyware is currently developed on. They are
environment issues, not sotyware bugs.

- **Windows `npm`/`node` shadow the Linux ones.** WSL appends Windows `PATH`
  entries (`/mnt/c/...`). In a shell that does not load nvm (non-interactive shells,
  scripts, AI agent tool calls), `npm` resolves to `/mnt/c/Program Files/nodejs/npm`,
  and `node`/`cargo` may not be found at all. Check with `which -a npm node cargo`.
  Put the Linux toolchains first and drop the Windows entries:

  ```bash
  export PATH="$HOME/.nvm/versions/node/<version>/bin:$HOME/.cargo/bin:$(echo "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd:)"
  ```

  Interactive zsh/bash sessions that source nvm and `~/.cargo/env` are not affected.
- **AppImage bundling fails** when `/mnt/...` entries are on `PATH`, because
  `linuxdeploy` crashes on the Windows directories. The `PATH` filter above fixes
  it; on its own:

  ```bash
  PATH=$(echo "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd:) npm run tauri build
  ```
- **`$XDG_RUNTIME_DIR` may point to a directory that does not exist** (seen here:
  `XDG_RUNTIME_DIR=/run/user/1000` with no such directory). The tray icon image
  is written there, so tray creation fails with
  `sotyware: cannot create system tray icon: tray icon error: Permission denied`
  and sotyware exits with code 1. With the variable unset, the tray is created
  normally. Fix: point it at a private `chmod 700` directory you own, as
  `tests/desktop/run.sh` does.
- **WSLg's window manager ignores focus and minimize requests.** Focus and
  un-minimize behaviour cannot be verified under WSLg (see
  [testing.md](testing.md)).
- A `libEGL warning: DRI3 error` on startup under WSLg is harmless.
