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

The [virtual-display test](testing.md#virtual-display-test) additionally needs:

```bash
sudo apt install -y xvfb openbox
```

The [headless tests](testing.md#headless-tests) need nothing beyond the build
packages above.

At runtime, end users need WebKitGTK 4.1, GTK 3 and libayatana-appindicator3. The
`.deb` declares these as dependencies (verified). The `.rpm` dependencies have not
been inspected. The AppImage bundles the libraries.

## Windows 10 / 11

Exercised on Windows 11 only (25H2, build 26200.9711, 2026-11-10); Windows 10 has
not been tried. Requirements according to Tauri, all confirmed on that machine:

- **Microsoft C++ Build Tools** (or Visual Studio) with the *Desktop development
  with C++* workload.
- **Rust** with the MSVC toolchain (`rustup default stable-msvc`).
- **WebView2 runtime.** It ships with Windows 11 and is installed on up-to-date
  Windows 10 systems. The installers Tauri produces download it if missing
  (Tauri's default `downloadBootstrapper` mode).

Build Windows binaries on Windows itself, from a normal Windows path. Do not build
from a WSL path (`\\wsl.localhost\...`).

Used on the Windows 11 machine: Rust 1.99.1 (`stable-x86_64-pc-windows-msvc`, from
`rustup-init.exe`), Visual Studio Community 2022 C++ tools, Node.js 24.14.1, npm
11.12.1, WebView2 Runtime 157.0.4341.42. Checkout steps that worked there, keeping
the WSL repository untouched (a separate clone from `origin`, so only pushed commits
are in it):

```powershell
git clone https://github.com/casualsoty/sotyware.git C:\dev\sotyware
cd C:\dev\sotyware
npm ci
npm run tauri build
```

The first `npm run tauri build` downloads WiX 3.14 and NSIS 3.13 for the bundles.

### Testing unpushed commits on Windows

To test commits that exist only in the WSL repository, move them with a Git
bundle instead of pushing or copying files; the Windows checkout then has the
same commit hashes. Used on 2026-11-10 to move `f8174da..1f5853b`. In WSL, with
`<base>` a commit the Windows checkout already has:

```bash
git bundle create /mnt/c/Users/<you>/AppData/Local/Temp/sotyware.bundle <base>..main
```

In Windows PowerShell:

```powershell
cd C:\dev\sotyware
git bundle verify $env:TEMP\sotyware.bundle
git fetch $env:TEMP\sotyware.bundle main:refs/remotes/wsl-bundle/main
git merge --ff-only wsl-bundle/main
```

`--ff-only` refuses rather than creating a merge if the checkout has diverged.
Afterwards delete the bundle and, if you like, the `wsl-bundle/main` ref
(`git update-ref -d refs/remotes/wsl-bundle/main`).

A PowerShell started from WSL (`powershell.exe`) inherits WSL's `PATH`, which may
lack `%USERPROFILE%\.cargo\bin`. Reload the Windows `PATH` first so `cargo` is
the native one:

```powershell
$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User')
```

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
  un-minimize behaviour cannot be verified under WSLg; the
  [virtual-display test](testing.md#virtual-display-test) checks them under openbox.
- **`/tmp/.X11-unix` is a read-only mount under WSLg**, so an X server cannot create
  its socket file there. `tests/desktop/virtual.sh` makes Xvfb listen on the
  abstract socket only, which works.
- A `libEGL warning: DRI3 error` on startup under WSLg is harmless.
