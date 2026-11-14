# Commands and workflow

Every command below was run successfully in the current repository (WSL2,
Ubuntu 24.04) unless marked otherwise. Run npm commands from the repository root
and Cargo commands from `src-tauri/`. Under WSL, check the `PATH` note in
[setup.md](setup.md#wsl2-notes) first.

## Install dependencies

```bash
npm ci
```

`npm ci` installs exactly what `package-lock.json` pins; use `npm install` only
when you intend to change dependencies. Rust crates are fetched by Cargo on the
first build (`Cargo.lock` is committed).

## Run in development

```bash
npm run tauri dev
```

Tauri runs `npm run dev` (Vite on `http://127.0.0.1:1420`, `strictPort`), then
builds and starts the debug binary. Frontend changes hot-reload; Rust changes
trigger a rebuild. The port is fixed in both `vite.config.ts` and
`src-tauri/tauri.conf.json` (`devUrl`); change both together.

Because sotyware is single-instance, a dev build and a release build of sotyware
cannot run side by side in the same session: the second one hands over to the
first and exits.

## Checks (run before every commit that touches code)

```bash
npm run check                               # svelte-check + TypeScript, strict, fails on warnings
npm run build                               # frontend production build into dist/
cd src-tauri && cargo fmt --check
cd src-tauri && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo test
npm run test:frontend                       # Vitest: dashboard UI with Tauri IPC mocks
```

`npm run test:headless` first runs `npm run check:rfd-patch` (below), the
release-binary, desktop-isolation and icon checks
([icons.md](icons.md#regenerating)), then the frontend tests and `cargo test`; it
needs no display, window manager, tray host or D-Bus session
([testing.md](testing.md#headless-tests)).

A change to `src-tauri/patches/` (the locally patched rfd, ADR-023), or a
dependency update that touches `tauri-plugin-dialog` or rfd, also needs the
patch tests on native Windows, from `src-tauri`: `cargo test --manifest-path
patches/rfd/Cargo.toml --lib --locked`, and the guard's own tests:

```bash
npm run check:rfd-patch                     # fails unless Cargo resolves rfd 0.17.2 from src-tauri/patches/rfd
npm run test:rfd-patch-guard                # the guard against broken temporary copies of src-tauri
```

`check:rfd-patch` reads Cargo's resolved graph (`cargo metadata --locked`,
never changes `Cargo.lock`), so it needs `cargo` on `PATH` and the crates
fetched or fetchable; it runs from any directory, on Windows and Linux.
Cargo only warns when a `[patch]` entry is not used and then builds the
unpatched rfd from crates.io, which brings back the Windows abort: treat
that warning, and any guard failure, as a failed check. How to update or
remove the patch: [patches/README.md](../../src-tauri/patches/README.md#updating-or-removing-the-patch).
Builds print rfd's nine upstream dead-code warnings; they are expected.

There is no JavaScript linter or formatter (ESLint/Prettier) and no `rustfmt.toml`
or `clippy.toml`; defaults apply. GitHub Actions runs the static checks and the
headless tests on Linux and Windows for every pull request to `main` and push to
`main`; it does not replace running them locally before a commit
([testing.md](testing.md#ci)).

## Production build

```bash
npm run build:release                       # binary + all bundles for the host OS (ADR-028)
npm run build:release -- --no-bundle        # release binary only
```

`build:release` keeps the build machine's paths out of the binary; a plain
`npm run tauri build` compiles but its bundling is refused for that reason.
Release scope, signing and the installer test plan:
[release.md](release.md). Desktop tests use a test-profile build instead
(`npm run build:test-profile`, ADR-027).

Outputs under `src-tauri/target/release/`:

- Linux: `sotyware`, `bundle/deb/*.deb`, `bundle/rpm/*.rpm`, `bundle/appimage/*.AppImage`
- Windows: `sotyware.exe`, `bundle/msi/*.msi`, `bundle/nsis/*-setup.exe`

Under WSL the AppImage step needs the `PATH` filter from [setup.md](setup.md#wsl2-notes).

## Desktop test (Linux, simulated)

```bash
npm run build:test-profile                # desktop tests run a test-profile build (ADR-027)
tests/desktop/run.sh                        # optional argument: path to another binary
```

Needs an X11 display. Details, prerequisites and how to read the results:
[testing.md](testing.md).

## Desktop test (Linux, virtual display)

```bash
npm run build:test-profile                # desktop tests run a test-profile build (ADR-027)
npm run test:desktop:virtual                # optional argument: path to another binary
```

Same test on a private Xvfb display with openbox; needs no monitor or desktop
session and leaves your display alone. Details:
[testing.md](testing.md#virtual-display-test).

## Desktop test (Windows, real desktop)

```powershell
npm run build:test-profile                # desktop tests run a test-profile build (ADR-027)
npm run test:desktop:windows                # = powershell ... -File tests\desktop\windows.ps1 [-Exe <path>]
```

It moves the mouse and clicks on your desktop; leave it alone while it runs. It
needs an unlocked, logged-in desktop with a taskbar and exits 3 without starting
sotyware if there is none.
Details: [testing.md](testing.md#windows-desktop-test).

## Desktop test (Windows, private desktop)

```powershell
npm run build:test-profile                # desktop tests run a test-profile build (ADR-027)
npm run test:desktop:windows:isolated       # = powershell ... -File tests\desktop\windows-isolated.ps1 [-Exe <path>]
```

Runs sotyware on a desktop that is never shown and uses no mouse or keyboard, so
you can keep working. It needs a logged-in session but no visible screen; it does
not cover the tray. Run it from a non-elevated PowerShell: elevated, it reports
NOT RUN (exit 4). Details:
[testing.md](testing.md#windows-isolated-desktop-test).

## Generated and ignored paths

- `dist/`: Vite output, consumed by Tauri as `frontendDist`. Ignored.
- `node_modules/`: ignored.
- `src-tauri/target/`: Cargo output. Ignored.
- `src-tauri/patches/rfd/target/`: only if the patch tests run without
  `--target-dir`; ignored by the crate's own `.gitignore`.
- `src-tauri/gen/schemas/`: capability JSON schemas generated by `tauri-build`.
  Ignored; `capabilities/default.json` references `../gen/schemas/desktop-schema.json`,
  which appears after the first build.
- `src-tauri/permissions/autogenerated/`: the `allow-`/`deny-operating-mode`
  permissions that `tauri-build` generates from the app manifest in `build.rs`.
  Rebuilt by every build. Ignored.
