# Roadmap

Nothing on this page is implemented. Implemented features are listed only in
[status.md](status.md). Move an item there when it is done, and record any
architectural decision it needs in
[decisions.md](../architecture/decisions.md).

## Next: finish Milestone 0 (validation)

In priority order:

1. **Finish Windows validation.** Windows 11 is built and passes the headless
   suite and the automated
   [Windows desktop test](../development/testing.md#windows-desktop-test),
   including the NSIS-installed copy, at `e8834fd` (with the test fix `71442d3`).
   Still to do: a person running the
   [manual checklist](../development/testing.md#manual-check-real-desktop-linux-or-windows)
   on Windows 11, the MSI installer (needs an elevated install), and everything on
   Windows 10. Record results in the [validation matrix](status.md#validation-matrix).
2. **Validate on real Linux desktops:** GNOME (with and without the AppIndicator
   extension), KDE Plasma, XFCE, and a real Wayland session. Blocked on access:
   the development machine has none
   ([survey](status.md#real-linux-desktop-availability-2026-11-10)); it needs a
   VM (for example VirtualBox with Ubuntu Desktop), a live USB session or a
   Linux machine, which is the user's decision. With the `ksni`
   backend, check in particular that the icon shows and that a left click opens the
   dashboard. Without a tray host (vanilla GNOME, an XEmbed-only panel), run the
   dashboard-only steps of the manual checklist.
3. **Install and run the bundles** (`.deb`, `.rpm`, AppImage, the Windows MSI). The
   NSIS installer is done on Windows 11 (silent mode).

Fixes found during validation come before anything below.

## Known follow-ups (not started, no design yet)

- Add CI as described in
  [testing.md](../development/testing.md#ci): headless jobs on Linux and
  Windows, the virtual-display job on Linux, a trial of the Windows
  private-desktop test on a hosted runner (exit 3 = no usable session), and the
  real-desktop Windows test only on a self-hosted runner in a logged-on session.
- Replace the placeholder icons.
- Drop `libayatana-appindicator3-1` from the `.deb` dependencies: the binary
  never loads it since `12a6185` ([limitations.md](limitations.md#linux-tray)).
  Separate packaging task, unrelated to dashboard-only mode.
- Optionally report the libayatana-appindicator registration race upstream. It no
  longer affects sotyware
  ([limitations.md](limitations.md#historical-libayatana-appindicator-registration-race)).
- With every Tauri upgrade, check that the Linux-only `tray-icon` declaration
  still unifies with Tauri's copy and still selects `ksni`, and drop it once Tauri
  lets apps choose the backend
  ([ADR-011](../architecture/decisions.md#adr-011-linux-tray-uses-tray-icons-ksni-backend-not-libayatana-appindicator)).
- Make the D-Bus fallback warning visible (for example by installing a logging
  subscriber), if that is wanted.

## Ideas (not decided)

These have been mentioned as possible future directions. There is no design, no
decision and no code for any of them. Do not implement them without an explicit
request and a recorded decision.

- Autostart on login.
- Local persistence (SQLite has been mentioned).
- A feature manager / plugin architecture.
- Synchronization.
