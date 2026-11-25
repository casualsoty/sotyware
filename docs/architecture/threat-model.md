# Threat model

What sotyware protects, against whom, how, and what remains. It covers the
desktop app (Tauri process, the dashboard's and the notes overlay's
webviews, local databases, OS credential store), the cloud storage
providers it talks to, the backup engine ([backup.md](backup.md)), the
sotyware account service and its end-to-end encrypted vault
([accounts.md](accounts.md)), and application updates
([updates.md](updates.md)). Each section names the assets at stake, the
controls, and the residual risk. Controls that depend on external
configuration (a signing key, a hosted service) say so. Written 2026-11-15
as the design target of the production-readiness work; whether each control
is implemented and tested is recorded in [status.md](../status/status.md)
and the [journal](../status/journal.md), not here.

## Assets

| Asset | Where it lives | Why it matters |
| --- | --- | --- |
| User files (objects) | Provider buckets (primary), backup destination | The data itself |
| Cloud provider credentials (key id + secret) | OS credential store (Windows Credential Manager, Linux Secret Service), per account id; inside the encrypted vault when sync is on | Full access to the buckets they cover, including deletion |
| Account key (vault key) | OS credential store of each enrolled device (wrapped), wrapped copies on the server | Decrypts the vault, hence every synced credential |
| Recovery key | Only where the user keeps it (printed, password manager) | Unlocks the vault on a device that has no enrolled key |
| sotyware account password | User's memory/password manager; Argon2id hash on the server | Signs in; never decrypts the vault |
| Session tokens | Refresh token in the OS credential store; access token in memory | Act as the user towards the account service |
| Update signing key | Release maintainer's secret storage / CI secret (not in the repository) | Whoever holds it can ship code to every installation |
| Local databases | App local data folder (`%LOCALAPPDATA%\com.sotyware.desktop`, `~/.local/share/com.sotyware.desktop`) | Configuration, favorites, journal, backup history; no secrets |
| Recorded macros (ADR-050) | `automation.sqlite3` in the same folder; export files where the user saves them | Whatever the user typed while recording, in plain text (a password typed then is in the macro) |
| Input automation (ADR-050) | The `autoclicker` feature's native backend, while it runs | Sends clicks and keys to any program the user's session can reach, and records the user's input while a recording runs |
| Notes (ADR-052) | `notes.sqlite3` in the same folder | Whatever the user writes in the notes overlay, in plain text, kept nowhere else: not synced to the sotyware account (the vault carries storage accounts and backup jobs only), not in sotyware's backups (they copy buckets), never in a sync folder (folder sync refuses the data folder) |

## Trust boundaries

1. **Dashboard webview ↔ Rust.** The dashboard's page is treated as less
   trusted than Rust: it reaches Rust only through the declared commands,
   each with its own permission in `capabilities/default.json`; it never
   receives a credential, a token or a key, never sends a local path
   (dialogs are opened by Rust), and receives one kind of local path only:
   the folder the user chose for folder sync, to show it (ADR-043; a
   compromised page learns where that folder is, and still cannot name
   another). CSP allows no remote content.
2. **Overlay webview ↔ Rust (ADR-052).** The notes overlay's page
   (`overlay.html`) runs in a second webview window, label `overlay`, that
   Rust creates at runtime the first time the overlay is shown and
   destroys when the `notes-overlay` feature stops (`overlay/host.rs`);
   while the feature is off there is none. The same CSP applies, and the
   page is treated like the dashboard's: it receives no credential, token,
   key or path and sends no path. Its own capability, `notes-overlay`
   (`capabilities/overlay.json`, window `overlay` only), grants its 16
   commands (its status; showing, hiding, the mode, collapsing and the
   opacity of its window; asking the dashboard for its settings; listing,
   creating, saving, deleting, duplicating, pinning, moving and selecting
   notes; `open_note_link`) and dragging and resizing its own window, and
   nothing of the dashboard's: no storage account, backup, account,
   update, sync, automation or feature command, no setting but its
   opacity, no `dialog:`, `fs:` or `opener:` permission. The dashboard's
   capability holds the 7 commands that watch, configure, reset, show,
   hide and switch the overlay, none that reads a note; the capabilities
   granted at runtime (`tray-mode`, `window-controls`) name only the
   dashboard.
3. **sotyware ↔ storage provider.** HTTPS (rustls with ring and the OS root
   store); SigV4 request signing; the provider is trusted to store bytes but
   its answers are verified (sizes, SHA-256, ETags where meaningful).
4. **sotyware ↔ account service.** HTTPS only in release builds; the service
   is **not trusted with plaintext**: it stores ciphertext and wrapped keys
   only and cannot decrypt them.
5. **sotyware ↔ update server (GitHub Releases, ADR-036).** Not trusted:
   everything it serves is checked against signatures made with a key that
   neither GitHub nor the build jobs hold. A compromised GitHub account or
   token can publish a release, but not a validly signed one.
6. **Process ↔ other local processes.** Same-user processes are outside
   sotyware's control (see "malicious local process").

## Threats and controls

### Stolen computer

- *Exposure:* local databases (no credential, token or key; macros and
  notes hold whatever the user typed or wrote, in plain text), OS
  credential store entries, cached vault ciphertext.
- *Controls:* credentials only in the OS credential store, which is
  protected by the user's login (Windows DPAPI-backed Credential Manager;
  Secret Service keyrings locked with the login password). No credential,
  token or key is written to SQLite, logs or files. The vault on disk is
  ciphertext.
- *Residual:* a thief who can log in as the user (or a running, unlocked
  session) can use the stored provider keys and the enrolled device key. The
  user must revoke the device in the account settings (which stops vault
  access for that device and triggers key rotation) and **rotate the
  provider keys at the provider**: revoking a device cannot make it forget
  secrets it already decrypted. Full-disk encryption (BitLocker) is the
  user's control and is recommended in the user documentation.

### Compromised sotyware account (password known to an attacker)

- *Controls:* the password signs in but **does not decrypt the vault**. A
  new device needs the recovery key or an approval from an already enrolled
  device, with a fingerprint the user compares on both screens. Rate limits
  and progressive lockout on sign-in; refresh-token rotation with reuse
  detection revokes a stolen session family; password change revokes other
  sessions.
- *Residual:* the attacker can delete the account (and so the server copy of
  the vault), list device names, and create pending enrollment requests
  (which an inattentive user might approve — the approval screen shows the
  requesting device's name, platform and fingerprint and says that approving
  gives access to every synced credential). Deleting the account does not
  delete local configuration on enrolled devices, from which the vault can be
  re-created.

### Compromised account service (or its operator)

- *Controls:* it stores only ciphertext, wrapped keys (sealed boxes,
  AEAD-wrapped account key) and non-secret metadata. It never receives the
  account key, the recovery key or any provider credential in plaintext. The
  list of trusted device keys lives **inside** the encrypted vault, so the
  server cannot add its own key to the set that receives a re-wrapped account
  key. The vault payload carries a monotonically increasing revision checked
  against the highest revision each device has seen, so a rollback to an old
  vault is detected by existing devices.
- *Residual:* the service can deny service, delete data, serve an old vault
  to a **fresh** device that has never seen a newer revision (rollback not
  detectable there), and learn metadata (email, device names and platforms,
  sync times, vault size). It could also present a malicious new-device
  enrollment request; the fingerprint comparison is the control.

### Compromised cloud provider account or key

- *Controls:* backups go to a separate destination, ideally another provider
  account with its own key that has no delete permission, and with Object
  Lock where the provider and bucket support it. The app reports the
  destination's actual lock configuration and never calls a backup immutable
  unless the bucket enforces default retention in compliance mode.
- *Residual:* a key with delete rights on the primary bucket can destroy the
  primary data; a key with delete rights on the backup bucket (and no Object
  Lock) can destroy backups. Governance-mode retention can be bypassed by a
  key with the bypass permission.

### Malicious update server, compromised GitHub account or update artifact

- *Controls (implemented 2026-11-15, [updates.md](updates.md)):* sotyware
  verifies a separately signed manifest with the compiled-in public key
  before reading it; installs only a version newer than the running one and
  refuses one lower than the highest version a verified manifest ever
  offered this installation (replay/rollback); hands exactly that manifest
  to `tauri-plugin-updater` (no second, unsigned fetch), whose package
  signature must name the same version; checks the package's SHA-256
  against the signed manifest; HTTPS only, and at most five redirects,
  each to HTTPS (GitHub's `releases/latest/download/` redirects twice). The
  signing key is a secret of the `release` environment, which admits only
  `v*` tags (and, configured as documented, waits for a reviewer); only
  the signing step of `release.yml`'s publish job receives it, never a
  build job whose dependencies' build scripts could read it, a pull
  request (fork pull requests get no secret and a read-only token) or a
  dry run (a throwaway key). That job installs its dependencies without
  install scripts and restores no cache, checks that the tag still names
  the built commit and that the secret belongs to the committed public
  key, and the committed key can never be a signing fixture's throwaway
  key or a private key (`check-key`). `prepare` refuses a release while
  the key or an Apple credential is a repository secret, or while the
  `release` environment admits a branch (ADR-039). Releases are published
  in the public source repository with the workflow's own `GITHUB_TOKEN`
  (`contents: write` only in that job): a leaked token can publish files
  installations refuse, not updates. The Apple credentials live in the
  `macos-signing` environment (`main` and `v*` tags), used only by the
  macOS package job.
  The plugin's own commands are unreachable from the page (no `updater:`
  permission). Nothing installs without the user's click, and not while a
  transfer, file operation or backup runs. Tested with test keys and a
  fixture signed by the real Tauri CLI (`updates/tests.rs`,
  `scripts/update-manifest.test.mjs`).
- *Residual:* whoever holds the update signing key can ship arbitrary code.
  The key must live outside the repository (CI secret or offline). A
  production key does **not** exist yet (`src-tauri/update-key.pub` is
  empty); release builds have updates switched off. A fresh installation has
  no version floor yet, so the first manifest it sees can be any validly
  signed one, newer than the installed version. A withheld update (the
  server answering an old manifest, or nothing) is not detected beyond the
  floor: there is no expiry on manifests. Whoever controls the repository
  (the account, or a publish job's token) can delete releases or mark an
  older one latest, which installations see as "up to date" (the floor
  refuses anything lower than what they were offered). Whoever can push a
  `v*` tag can start a release: the tag ruleset and the `release`
  environment's reviewer are the gates, and both are settings the
  maintainer must make ([release.md](../development/release.md#repository-settings);
  none exists yet). A dependency or action compromised upstream could act
  inside the signing step; actions are pinned to commit SHAs and npm
  packages to the lockfile's hashes.

### Malicious local process (same user)

- *Residual, by design of desktop operating systems:* a process running as
  the same user can read the user's files, query the OS credential store
  (Linux Secret Service grants any same-session client once unlocked;
  Windows Credential Manager entries are readable by the same user), attach a
  debugger, or inject input. sotyware cannot defend against this; it keeps no
  secrets in files and keeps the IPC surface narrow (no local HTTP server,
  no named pipes beyond the single-instance plugin, which only shows the
  dashboard).

### Input automation misused or running away (ADR-050)

- *Threats:* a macro or the autoclicker clicks or types where the user did
  not mean (a wrong window, a runaway loop); a stopped session leaves a key
  or button held; an imported macro file carries something harmful; the
  recorder captures a password.
- *Controls:* off by default; nothing starts unless the user starts it (a
  click on the page, a shortcut while the feature runs), never after a
  restart; one session at a time; the emergency stop shortcut, the
  fail-safe corners and the page's Stop (sotyware's own window is never
  clicked or typed into, so Stop always works); an endless run started
  from the page without a registered emergency stop needs confirming;
  every held key and button is released on every exit path, including a
  panic; a macro's target program can stop playback before keys reach
  another program. Macro files are data only: eight action kinds with
  bounded values, unknown fields refused, imported shortcuts switched off;
  nothing is ever executed. Power, Sleep and Wake keys cannot be recorded
  or replayed. The recorder skips sotyware's own window and the shortcut
  keys, and the Recorder tab warns that typed text is stored in plain
  text. The page never sends or records input itself: only Rust does,
  through the commands listed in the capability.
- *Residual:* a macro does what it was recorded or edited to do, in
  whatever program is in front (without a target program); macros are not
  encrypted at rest; a process killed from outside cannot release a held
  key (the user's next press of it does); on Windows, input to programs
  running as administrator is blocked by the system, not by sotyware.

### Hostile text in a note (ADR-052)

- *Threats:* text pasted into a note (from a web page or a guide) carries
  HTML or script that would run in the overlay's page, which can edit
  every note; a link in a note opens a file, a program or another scheme
  instead of a web page, or an address the note does not hold.
- *Controls:* `noteMarkdown.ts` parses a note into a tree of headings,
  paragraphs, lists, checklists, quotes, code and rules, with text,
  emphasis, code and links inside, which `NoteView.svelte` renders with
  Svelte's own elements, the note's text in text nodes. No `{@html}`,
  `innerHTML` or other HTML sink exists in the frontend, so a `<script>`
  or `<img>` in a note stays visible text; the subset has no images, and
  the CSP would load nothing remote. A link's target is only ever an
  `http` or `https` address (every link form the parser reads starts so),
  and a click on one is cancelled in the page and handed to Rust.
  `open_note_link(id, url)` opens only what `model::link_allowed`
  accepts: an `http://` or `https://` address with a host, without
  whitespace or control characters, at most 2,048 characters long,
  contained in note `id`'s text as Rust reads it from `notes.sqlite3`.
  Rust opens it in the default browser with `tauri-plugin-opener`'s
  `open_url` (the plugin is not registered and the page has no `opener:`
  permission), so nothing but a web address is ever opened, never a
  `file:`, `javascript:` or other scheme; test-profile builds record the
  address instead.
- *Residual:* Rust checks the note's text, not what the page showed as a
  link, and the overlay's page is the notes' editor: a page that ran a
  script it should not (through a webview or Svelte defect; a note cannot
  bring one) could rewrite or delete every note and open any web address
  it first wrote into one, though only in the browser.

### The notes overlay's shortcuts and passive mode (ADR-052)

- *Threats:* the overlay's global shortcuts take keys away from the game
  or another program, or read what the user types; the overlay takes the
  game's keyboard, or stays click-through with no way back.
- *Controls:* off by default; its shortcuts are registered only while the
  `notes-overlay` feature runs, and stopping it unregisters them. They are
  system shortcuts (`RegisterHotKey` on Windows, `XGrabKey` on X11)
  through a second instance of the autoclicker's backend, of which the
  overlay uses only registering, unregistering and naming keys: it
  installs no input hook, opens no RECORD context (both exist only while
  an autoclicker recording runs) and sends no input; it acts only on
  presses of its own chords and stores nothing. The autoclicker's rules
  apply (`Chord::problem`: a typing key needs Ctrl, Alt or the Windows
  key; F12 alone, Alt+F4, Alt+Tab, Ctrl+Alt+Delete, Ctrl or Alt with Esc
  and Win+L are refused), and `automation::claims` keeps the two features
  from holding one chord: each refuses a chord the other holds, naming
  it. Showing never takes the keyboard: on Windows the window is shown
  with `SWP_NOACTIVATE`, and the keyboard is given straight back should
  Windows activate it anyway; elsewhere it is unfocusable for the show.
  Only the mode or new-note shortcut, or a click, gives it the keyboard
  (on Windows `SetForegroundWindow`, never a synthesised key press).
  Passive mode is the window system's (on Windows
  `WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE`, on X11 an input
  shape without the window and no input focus), so the page gets no click
  and holds no keyboard. The mode is never stored, and the ways back never
  depend on the overlay's page: the mode shortcut, the dashboard's Make
  interactive and Hide overlay, turning the feature off, and a restart,
  which begins in the default mode (interactive unless the user chose
  passive). The passive badge names the shortcut, or says to switch back
  from sotyware. The background is never below 20 % opacity and the text
  stays opaque, so the overlay never takes clicks unseen.
- *Residual:* while the feature runs, its chords no longer reach the
  program in front. A chord the overlay holds is refused to the
  autoclicker, the emergency stop's too: the autoclicker then shows its
  stop as not registered and an endless run from the page needs
  confirming, but a combination of the stop with other modifiers, which
  the autoclicker adds while a session runs (ADR-050), stays the
  overlay's without being shown; the fail-safe corners and Stop still
  work. A game that reads the keyboard below Windows' shortcut handling,
  or anti-cheat software, can keep the shortcuts from arriving; without
  the mode shortcut (Wayland, macOS, a chord another program holds) only
  sotyware's window switches the overlay back. On X11 the window manager
  decides which window gets the keyboard back.

### Another user on the same computer

- *Controls (2026-11-15):* the data folder is private to the user: on Linux
  `storage::prepare_data_dir` creates it with mode `0700` and narrows an
  existing one from an older version (only that folder; a link in its place
  is not followed); on Windows `%LOCALAPPDATA%` is per-user by its ACL.
  Credentials are in the user's OS credential store, never in files.
- *Residual:* an administrator or root can read everything; the databases
  are not encrypted at rest (full-disk encryption is the user's or the
  organisation's choice).

### Accidental user deletion

- *Controls:* every delete is a reviewed plan; on versioned buckets a delete
  adds a delete marker and the file can be restored from Deleted items;
  sotyware never deletes versions permanently; backups keep historical
  snapshots under a retention policy and are restorable byte for byte; the
  recovery manifest makes restoration possible without sotyware.
- *Residual:* deletes on unversioned buckets are permanent at the primary
  (the review says so); recovery then depends on a backup.

### Ransomware-like mass modification / incorrect synchronization / app bug

- *Controls:* backups are content-addressed and append-only (a changed file
  is a new blob; the old one stays referenced by older snapshots). A snapshot
  whose change ratio exceeds the configured threshold is marked suspicious:
  retention pruning stops and the backup health warns. Sync never deletes
  remote objects (it carries configuration only) and never lets a stale device
  overwrite a newer vault (optimistic concurrency with revisions).
- *Residual:* a slow, below-threshold corruption is captured into new
  snapshots; older snapshots keep the good versions until retention removes
  them.

### Provider lifecycle policies / backup destination deletion

- *Controls:* backup verification lists and checks every blob of the latest
  snapshot; missing blobs mark the backup unhealthy. Object Lock (where
  configured) prevents deletion before the retention date.
- *Residual:* a lifecycle rule configured at the provider can remove backup
  data between verifications.

### Application database corruption

- *Controls:* SQLite databases are integrity-checked at open; a damaged or
  newer database is never repaired or replaced automatically, the app runs
  with defaults and shows the error. Losing local databases loses no data
  kept elsewhere: objects stay at the provider, credentials in the OS store,
  the vault on the server, backups describe themselves.
- *Residual:* notes (ADR-052) and macros (ADR-050) are kept nowhere else
  (macros at most in files the user exported): losing `notes.sqlite3` or
  `automation.sqlite3` loses them.

### Provider outage / sotyware backend outage

- *Controls:* storage works without the account service (accounts are
  optional, local-only installations keep working). A backup on another
  provider keeps the data reachable during a primary-provider outage, with
  standard S3 tools if needed. The service's own database is backed up and
  its restore is tested ([server operations](../development/server.md)).
- *Residual:* during a service outage, new devices cannot enroll and sync
  pauses; existing devices keep their local configuration.

## Privacy

- No telemetry, analytics or crash reporting exists. stderr diagnostics may
  contain object keys (file names) and backup job ids; they are never
  persisted or sent by sotyware. Reviewed 2026-11-15: no log line in the
  app or the service contains a credential, token, password, account key,
  recovery key or update key (errors that could carry one are typed and
  tested, e.g. `errors_never_contain_a_secret`). The service's `outbox`
  command prints development mail, verification codes included, by design;
  production sends it through an SMTP relay (ADR-044), whose log lines name
  only the mail's row id and the kind of failure, never the address or the
  relay's reply text. A queued mail holds its code in plaintext in the
  `outbox` table until it is sent (seconds, at most a day while the relay
  refuses), so a database backup taken then contains it; codes expire after
  15 minutes.
- The autoclicker (ADR-050) records input only during a recording the user
  started, keeps macros and profiles on this computer and sends nothing
  anywhere; macros leave the computer only in files the user exports.
- The notes overlay (ADR-052) keeps its notes, settings and state on this
  computer and sends nothing anywhere: notes are neither synced nor backed
  up, and its shortcuts record no input. An address in a note reaches the
  default browser only when the user opens the link.
- An update check sends a plain GET with `User-Agent: sotyware/<version>`;
  GitHub (and its asset store) learns the IP address, the time and the
  version.
- The account service learns: email, password hash, device names/platforms,
  IP addresses in its rate limiter (memory only), sync timestamps and vault
  sizes. Its audit log records event types, account and device ids and times,
  never content or secrets.

## Dependency review (2026-11-15)

`cargo audit` (RustSec database, 1,509 advisories) and `npm audit` on the
lock files of this commit:

- **Account service** (`server/Cargo.lock`): no advisory.
- **Frontend runtime** (`npm audit --omit=dev`): no advisory. Dev only:
  `deepmerge` (low, GHSA-gcqx-w48m-w2h4, build time, no fix).
- **Desktop app** (`src-tauri/Cargo.lock`): two vulnerabilities in
  `quick-xml` 0.37.5 (RUSTSEC-2026-0194, -0195: denial of service on
  hostile XML), reached only through `s3s`, the in-process S3 server of the
  tests (a dev-dependency, never in a build). Unsound: `lru` 0.12.5
  (RUSTSEC-2026-0002, -0253) through `aws-sdk-s3`, which uses only
  `get_or_insert_mut` in its S3 Express session cache (not the affected
  `IterMut` or `pop`, and S3 Express is AWS-only); a fixed `lru` needs a
  newer `aws-sdk-s3` than Rust 1.90 allows. `glib` 0.18.5
  (RUSTSEC-2024-0429, `VariantStrIter`, Linux GTK 3 through Tauri; no GTK 3
  release with the fix). Unmaintained: `zbus_names`, `zvariant`,
  `zvariant_derive` (Linux D-Bus), `pin-utils`, `proc-macro-error` (build
  time). None is reachable with attacker-controlled input in sotyware's use;
  re-run before every release.

## Residual risks summary

1. Same-user malware can read stored credentials.
2. Revoked devices keep what they already decrypted: rotate provider keys.
3. Losing both the recovery key and every enrolled device makes the vault
   unrecoverable; there is no backdoor. Storage data itself is unaffected
   (credentials can be re-entered from the provider console).
4. A fresh device can be served an old vault by a malicious service.
5. The update signing key holder can ship code; production key not yet
   created, so updates are off in release builds. Manifests do not expire,
   so a server can withhold updates (never roll back below the floor).
6. Check-then-write races remain where providers lack conditional writes.
7. Backups are stored as plaintext objects (readable by anyone with read
   access to the backup bucket), chosen so they stay restorable with
   standard tools; use a private bucket with a dedicated key.
8. Recorded macros keep typed text in plain text on this computer, and a
   running macro acts on whatever program is in front unless it has a
   target program (ADR-050).
9. Notes are kept in plain text on this computer only, outside sync and
   backups, and a chord the overlay holds is unavailable to every other
   program, the autoclicker's emergency stop included (ADR-052).
