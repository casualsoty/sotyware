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
- **Tests:** `cargo test -p rfd --lib` on Windows (on Linux the tests are not
  compiled: the module is Windows-only).
- **Remove** the copy, the patch file and the `[patch.crates-io]` entry once
  an rfd release that `tauri-plugin-dialog` accepts no longer unwraps the
  conversion (not the case in 0.17.2, 0.18.0 or `master` on 2026-11-13), then
  re-run the Windows upload desktop test (its unpaired-surrogate batch).
