# Roadmap

Nothing on this page is implemented. Implemented features are listed only in
[status.md](status.md). Move an item there when it is done, and record any
architectural decision it needs in
[decisions.md](../architecture/decisions.md).

## Next: finish Milestone 0 (validation)

In priority order:

1. **Validate on Windows 10 and 11.** Build on Windows (not from a WSL path), run the
   [manual checklist](../development/testing.md#manual-check-real-desktop-linux-or-windows),
   including left-click-to-open and focus on second launch. Record results in the
   [validation matrix](status.md#validation-matrix).
2. **Validate on real Linux desktops:** GNOME (with and without the AppIndicator
   extension), KDE Plasma, XFCE, and a real Wayland session.
3. **Install and run the bundles** (`.deb`, `.rpm`, AppImage, Windows installers).

Fixes found during validation come before anything below.

## Known follow-ups (not started, no design yet)

- Replace the placeholder icons.
- Decide whether and how to handle a missing tray host on Linux (see
  [limitations.md](limitations.md#linux-tray)).
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
