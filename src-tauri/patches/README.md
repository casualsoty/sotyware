# Patched dependencies

sotyware builds one crate from a local copy instead of crates.io, through
`[patch.crates-io]` in [`../Cargo.toml`](../Cargo.toml). Why:
[ADR-023](../../docs/architecture/decisions.md#adr-023-rfd-is-patched-locally-so-windows-dialog-paths-are-read-as-osstring).

## `rfd/`: rfd 0.17.2 with Windows dialog paths read as `OsString`

- **Upstream:** rfd 0.17.2 as published on crates.io (checksum
  `20dafead71c16a34e1ff357ddefc8afc11e7d51d6d2b9fbd07eaa48e3e540220`,
  upstream commit `cca0cfa5fc38c2752c9c98f9f5168904a515fd55`), reached
  through `tauri-plugin-dialog` 2.8.4, which requires `rfd = "0.17"`.
- **Change:** only `src/backend/win_cid/file_dialog/com.rs`, the full diff
  is [`rfd-0.17.2-osstring.patch`](rfd-0.17.2-osstring.patch).
  `read_to_string` (`String::from_utf16(..).unwrap()`) becomes
  `read_to_os_string` (`OsString::from_wide`), so `IShellItem::get_path`
  returns a path with exactly the UTF-16 code units Windows gave it. Every
  Windows dialog result goes through that function: the open-file, folder
  and save dialogs (`get_result`) and multi-selection (`get_results`).
  Windows-only regression tests are added at the end of the same file. No
  other file is changed (the copy has no `.cargo-ok`, which Cargo writes when
  it unpacks a crate). Linux and macOS code is untouched.
- **Check that the copy is still exactly upstream plus the patch** (from
  `src-tauri/`, after `cargo fetch` put rfd 0.17.2 in the registry cache):

  ```bash
  diff -r -x .cargo-ok ~/.cargo/registry/src/index.crates.io-*/rfd-0.17.2 patches/rfd
  ```

  The only difference must be `com.rs`, and `patch -p1 -R --dry-run -d
  patches/rfd < patches/rfd-0.17.2-osstring.patch` must apply cleanly.
- **Guard: Cargo must actually build this copy.** `npm run check:rfd-patch`
  ([`scripts/check-rfd-patch.mjs`](../../scripts/check-rfd-patch.mjs)) fails
  unless the resolved graph from `cargo metadata --locked` contains rfd
  exactly once, at 0.17.2, as a path package (no registry source) whose
  manifest is `patches/rfd/Cargo.toml`, and `com.rs` still contains
  `OsString::from_wide(slice)` and not `String::from_utf16(slice).unwrap()`.
  It also fails on a `[[patch.unused]]` rfd entry in `Cargo.lock` or a
  "was not used" warning from Cargo. It runs at the start of
  `npm run test:headless`, in CI right after `cargo fetch --locked`
  (before anything compiles) and before the release build of both manual
  desktop workflows. Its own tests (`npm run test:rfd-patch-guard`) break
  temporary copies of `src-tauri` and expect it to fail.
- **Why an unused-patch warning is a failure:** a `[patch]` entry that no
  longer matches the requirement (for example a `tauri-plugin-dialog`
  release that needs rfd 0.18) is ignored with a warning, and Cargo builds
  the unpatched rfd from crates.io. The build succeeds and every headless
  test passes, but on Windows a dialog path with an unpaired surrogate
  aborts sotyware again. Only a check of what Cargo resolved catches that.
- **Tests:** `cargo test -p rfd --lib` on Windows (on Linux the tests are not
  compiled: the module is Windows-only).

## Updating or removing the patch

When `npm run check:rfd-patch` fails, or a dependency update touches
`tauri-plugin-dialog` or rfd:

1. Find the rfd version the new graph requires (`cargo tree -i rfd` from
   `src-tauri/`; the guard's message names the resolved version).
2. If that rfd release no longer unwraps the conversion (look at
   `read_to_string`/`get_path` in `src/backend/win_cid/file_dialog/com.rs`),
   **remove** the patch: the copy, the `.patch` file, the
   `[patch.crates-io]` entry, `scripts/check-rfd-patch.mjs`, its test, the
   two npm scripts and the CI steps that call them. Then re-run the Windows
   upload desktop test (its unpaired-surrogate batch).
3. Otherwise **re-make** the patch on the new release: replace `rfd/` with
   the unpacked crate (without `.cargo-ok`), apply the change again (start
   from `rfd-0.17.2-osstring.patch`), regenerate the `.patch` file, rename it
   for the new version and update `EXPECTED_VERSION` in
   `scripts/check-rfd-patch.mjs` together with this README and ADR-023.
4. Run `npm run check:rfd-patch`, `npm run test:rfd-patch-guard`, the patch
   tests on Windows (below) and the Windows upload desktop test.

Never silence the guard by changing only `EXPECTED_VERSION`: the version
belongs to the patched copy, not to whatever Cargo resolves.

## Removal condition

- **Remove** the copy, the patch file and the `[patch.crates-io]` entry once
  an rfd release that `tauri-plugin-dialog` accepts no longer unwraps the
  conversion (not the case in 0.17.2, 0.18.0 or `master` on 2026-11-13), then
  re-run the Windows upload desktop test (its unpaired-surrogate batch).
