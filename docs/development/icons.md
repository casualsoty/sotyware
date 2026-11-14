# Application icons

sotyware's icon is a **stored object**: an isometric cube in three panels,
white at three strengths (top face solid, left about 78 %, right about 54 %),
on a rounded tile with a diagonal gradient from blue `#2b6cf0` (top left)
to teal `#0f8b8d` (bottom right). It stands for object storage without
using a generic cloud shape, and it has no text. The white cube on the
mid-tone tile keeps its contrast on light and dark taskbars, title bars
and tray areas, because the tile carries its own background.

## Sources

Three hand-made SVG files in `src-tauri/icons/source/` are the only
sources. Every other file in `src-tauri/icons/` is generated from them:

| Source | Used for | Difference |
| --- | --- | --- |
| `sotyware.svg` (1024 × 1024) | 48 px and up: installers, Start menu, Explorer, Linux packages | tile with a margin; the panels are separated by seams and have rounded outer corners |
| `sotyware-32.svg` (32 grid) | 24 and 32 px | no seams, nearly full-bleed tile, vertical edges on whole pixels |
| `sotyware-16.svg` (16 grid) | 16 px | the same, fitted to the 16 px grid |

The small sources exist because the master's seams and margins blur at
16–32 px. Keep the three in step: a change to colour or shape goes into all
three. The colours are repeated in each file, so a colour change has to be
made three times.

## Generated files

| File | Size | Content | Used by |
| --- | --- | --- | --- |
| `icon.ico` | 16, 24, 32, 48, 64, 256 px | one PNG-compressed entry each (16 from `sotyware-16.svg`, 24 and 32 from `sotyware-32.svg`, the rest from the master) | Windows: the executable's icon resource (Explorer, Start menu, taskbar), the window and tray icon, the MSI's `ProductIcon`, and the NSIS setup program and uninstaller (`bundle.windows.nsis`) |
| `32x32.png` | 32 px | from `sotyware-32.svg`; byte-equal to the ICO's 32 px entry | Linux: the window and tray icon (Tauri takes the first PNG in `bundle.icon`) and the packages' 32 px icon |
| `128x128.png` | 128 px | master | Linux packages |
| `128x128@2x.png` | 256 px | master | Linux packages (256 px) |
| `icon.png` | 512 px | master | Tauri's fallback when no PNG is configured; not listed in `bundle.icon` |
| `icon.icns` | ICNS | master, every size | macOS only (not a target); kept so the configuration stays complete |

`bundle.icon` in `src-tauri/tauri.conf.json` lists `32x32.png`,
`128x128.png`, `128x128@2x.png`, `icon.icns` and `icon.ico`; `32x32.png`
must stay the first PNG. `bundle.windows.nsis.installerIcon` and
`uninstallerIcon` both name `icons/icon.ico`: without them Tauri leaves
NSIS's stock icon on the setup program and the uninstaller (seen in the
first build of 2026-11-14). The MSI needs no setting: Tauri puts
`icon.ico` into its Icon table as `ProductIcon`. Add/Remove Programs shows
the icon of the installed `sotyware.exe` for NSIS and `ProductIcon` for the
MSI.

**Window and tray icon.** `tray.rs` uses the app's default window icon.
On Windows Tauri loads it from the executable's icon resource at the
system's large-icon size (`SM_CXICON`: 32 px at 100 % scaling, 48 at
150 %, 64 at 200 %), so at 100 % the tray shows the simplified 32 px entry
scaled to 16 px. On Linux it is `32x32.png`. There is no separate tray
image.

**Seen at runtime** (2026-11-14, Windows 11 25H2 at 100 %, a test-profile
build): the title bar, the running taskbar button and the tray icon in the
hidden-icons flyout show the cube; the window's live small icon is the
32 px entry. Not yet seen: other scalings, Explorer, the Start menu, Linux
([status](../status/status.md#application-icons-in-the-running-app-on-native-windows-11-2026-11-14)).
A taskbar pin made from an older build keeps its old icon: it points at
another executable.

The frontend uses no icon: the dashboard has no logo, and the webview has
no favicon.

## Installer icons

`scripts/check-installer-icons.mjs` checks that the Windows installers
carry `icon.ico`. It reads the files and parses them in memory (Node
built-ins only, no 7-Zip, WiX or Windows Installer API); it never runs,
installs or unpacks an installer to disk and writes nothing. What it
compares:

| Installer | Resource | Compared how |
| --- | --- | --- |
| MSI | `Property` `ARPPRODUCTICON` (the Add/Remove Programs icon) → its `Icon` table row (`ProductIcon`) → the stream `Icon.ProductIcon` | byte-equal to `icon.ico` |
| MSI | every `Shortcut` row: `Icon_` (the Start menu shortcut names `ProductIcon`) | that `Icon` stream byte-equal to `icon.ico`; at least one shortcut must name it |
| MSI | a shortcut without `Icon_` (the desktop shortcut) | must target the `File` that is `sotyware.exe`, whose icon is then checked in the administrative image (next row); a shortcut to something outside the package (Tauri's "Uninstall sotyware" → `msiexec.exe`) is listed and not checked |
| MSI administrative image (`msiexec /a`) | the `.msi` copy in it, as above; its `sotyware.exe`'s icon group | as above; as for an executable |
| NSIS setup program | its PE icon group (`RT_GROUP_ICON`) and icons (`RT_ICON`) | as for an executable |
| NSIS uninstaller | not a file in the setup program: NSIS writes it at install time from the setup program's header, patched with the uninstaller icon data stored in the compressed data. The check decompresses that data (solid LZMA, Tauri's default; its own decoder), finds the one entry that is icon patch data, applies it in memory and checks the result's icon group. Every icon resource must have been written by that patch, so the setup program's own icon cannot pass for the uninstaller's | as for an executable |
| `sotyware.exe` (built, and the copy in the MSI image) | PE icon group | as for an executable |

"As for an executable": exactly one icon group; its entries are, in order,
`icon.ico`'s six entries with the same width, height, colour count, planes,
bit count and size; each points at an `RT_ICON` resource byte-equal to that
entry's PNG; no other `RT_ICON` resource exists. A missing, malformed,
truncated or different icon fails, as does a file or image that is missing
or cannot be parsed (each with its own message); the command exits 1, or 2
for a wrong command line.

Run it on a Windows release build (after `npm run build:release`):

```powershell
# msiexec returns at once unless waited for
Start-Process msiexec.exe -Wait -ArgumentList '/a', 'src-tauri\target\release\bundle\msi\sotyware_0.1.0_x64_en-US.msi', '/qn', "TARGETDIR=`"$env:TEMP\sotyware-msi-image`""
node scripts/check-installer-icons.mjs `
  --msi src-tauri\target\release\bundle\msi\sotyware_0.1.0_x64_en-US.msi `
  --msi-image "$env:TEMP\sotyware-msi-image" `
  --nsis src-tauri\target\release\bundle\nsis\sotyware_0.1.0_x64-setup.exe `
  --exe src-tauri\target\release\sotyware.exe
npm run test:installer-icons   # its tests: in-memory fixtures, any platform
```

`--msi`, `--nsis` and `--exe` also work on Linux on copied files;
`--msi-image` needs the image `msiexec /a` writes (it installs nothing, but
Windows logs MsiInstaller "installed the product" events for it).
`--ico` checks against another ICO (default `src-tauri/icons/icon.ico`).

**In CI:** `release-build.yml` runs the check after unpacking both
installers, on every artifact in the table, and fails the job on any
problem; `ci.yml` and `release-build.yml` run its tests
(`npm run test:installer-icons`): PE files, MSI databases and NSIS
setup programs built in memory, with correct, missing, stale, incomplete
and malformed icons, an LZMA vector made by liblzma, and checks that the
files are left unchanged and that the script imports no process, network
or write API.

**Not covered:** these checks prove which bytes the installers carry, not
what Windows shows. Explorer, the Start menu, Add/Remove Programs, the
installed application and other display scalings remain part of the
[installation test](release.md#installation-test-plan-disposable-windows-11)
on a disposable Windows 11 VM.

## Regenerating

```bash
npm ci
npm run icons          # writes every generated file from the three sources
npm run check:icons    # also part of npm run test:headless
npm run test:icons     # the check's own tests (CI runs them)
```

`npm run icons` (`scripts/icons.mjs`) renders with `tauri icon` from
`@tauri-apps/cli`, which is already a dev dependency (resvg; no Inkscape,
ImageMagick or other tool, nothing paid). The script asks it for the sizes
it needs and writes `icon.ico` itself, so the small entries come from the
small sources. It discards everything else `tauri icon` creates: Android and
iOS sets, Windows Store logos, `64x64.png`. It uses Node built-ins only and
runs on Linux and Windows.

The PNG and ICO output was identical across two runs on WSL2; the
`icon.icns` bytes differed between runs (`tauri icon` output, not
checked).

`npm run check:icons` fails when a file in `bundle.icon` is missing, the
NSIS installer or uninstaller icon is not `icons/icon.ico`, a PNG
is not 8-bit RGBA at the size its name says, `icon.ico` does not hold
exactly 16/24/32/48/64/256 px as PNG entries, its 32 px entry differs from
`32x32.png`, `icon.icns` is not an ICNS file, a source is missing, or
another file lies in `src-tauri/icons/`. It does not compare the images
with a fresh render: check them by eye after `npm run icons`.

Changing the icon changes no identifier, so an installed copy is upgraded
as before. Windows caches icons, so Explorer may show the old icon until
its icon cache is refreshed.
