# Operating the account service

`server/` is sotyware's account and end-to-end encrypted sync service
(ADR-032; design and API: [accounts.md](../architecture/accounts.md)). It is a
separate Rust crate with its own `Cargo.lock`, not part of the desktop app's
build. **No production instance exists**: hosting, a domain, a TLS
certificate, an email provider and backup storage are external decisions
(see [External gates](#external-gates)). Everything below is tested locally.
A step-by-step guide to a first deployment (domain, Brevo, Oracle Cloud,
Caddy): [server-setup.md](server-setup.md).

## Build and test

```bash
cd server
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked      # target/release/sotyware-server
```

The tests run the router in process with a fresh SQLite database and a
controllable clock (`tests/api.rs`): registration, verification, sign-in,
lockout, token rotation and reuse detection, sync setup, enrollment by
recovery key and by approval, optimistic concurrency, revocation, password
change and reset, account deletion, rate limits, body limits, no plaintext
secrets in the database file, and an online backup restored after the live
database was deleted. CI runs them in the `server` job of `ci.yml`.

## Configuration

Environment variables only (nothing secret is passed on the command line):

| Variable | Default | Meaning |
| --- | --- | --- |
| `SOTYWARE_SERVER_DB` | `sotyware-server.sqlite3` | The database file (WAL mode, `secure_delete` on) |
| `SOTYWARE_SERVER_ADDR` | `127.0.0.1:8787` | Listen address |
| `SOTYWARE_SERVER_TRUST_PROXY` | unset | `1`: take the client address from `X-Forwarded-For` (only behind a proxy that overwrites that header) |
| `SOTYWARE_SMTP_HOST` | unset | The mail relay ([Email](#email)); unset: no mail is sent, mails stay in the outbox (development) |
| `SOTYWARE_SMTP_SECURITY` | `starttls` | `starttls` (required upgrade, nothing is sent if it fails), `tls` (TLS from the first byte) or `none` (only to `localhost` or a loopback address) |
| `SOTYWARE_SMTP_PORT` | 587 / 465 / 25 | By the security above |
| `SOTYWARE_SMTP_USERNAME`, `SOTYWARE_SMTP_PASSWORD` | unset | The relay's login; both or neither. The password is a secret: put it in a root-only environment file, never on the command line or in git |
| `SOTYWARE_MAIL_FROM` | — | Required with a host: the sender, e.g. `sotyware <no-reply@example.com>`, at a domain the relay is allowed to send for |

An invalid mail setting stops `serve` at startup with the reason.

The service has no secret of its own: no signing key, no encryption key.
Tokens are random and stored as SHA-256 hashes; passwords as Argon2id
(19 MiB, 2 passes, 1 lane).

## Deploying (outline)

Done once, on 2026-11-17, by following [server-setup.md](server-setup.md)
(ADR-045); the outline is the reasoning behind its steps.

1. Run `sotyware-server serve` as an unprivileged user on `127.0.0.1`, with
   the database on a disk that is backed up (below).
2. Put a TLS-terminating reverse proxy in front (Caddy, nginx) on the public
   name, HTTPS only, HSTS, forwarding to `127.0.0.1:8787` and overwriting
   `X-Forwarded-For`; set `SOTYWARE_SERVER_TRUST_PROXY=1`.
3. Monitor `GET /healthz` (`{"status":"ok","database":"ok","api":"v1"}`).
4. Logs are stderr lines (`sotyware-server: …`): startup, internal errors;
   never an email address, password, token, code or vault content. The
   audit table records event names with account and device ids and times.
5. Capacity: one SQLite connection; password hashing happens while it is
   held, so about 20 sign-ins per second per instance. Enough for a small
   service; a larger one needs a connection pool and hashing outside the
   lock (a known limit).

## Backups of the service

The database is the only state. Back it up while it runs (SQLite online
backup, consistent):

```bash
SOTYWARE_SERVER_DB=/var/lib/sotyware/server.sqlite3 sotyware-server backup /backups/server-$(date -u +%Y%m%dT%H%M%SZ).sqlite3
```

The command refuses to overwrite a file and checks the copy's integrity.

**Off the host (ADR-047):** `server/ops/sotyware-backup`, run hourly by
`sotyware-backup.timer`, takes that backup, encrypts it with age to a
public key whose private half is only with the maintainer, uploads it to a
Backblaze B2 bucket with a Write Only key (one SigV4 `PUT` through curl,
ETag checked against the MD5), keeps local copies 7 days and pings a
healthchecks.io monitor (`/fail` on any error). The bucket's Object Lock
(Compliance, 30 days) and lifecycle rule (31 days) keep every copy for 30
days whatever happens to the VM. Setup, the monthly restore check and the
procedure for a lost VM: [server-setup.md](server-setup.md#11-backups).

Restore (service stopped):

```bash
SOTYWARE_SERVER_DB=/var/lib/sotyware/server.sqlite3 sotyware-server restore /backups/server-20261115T100000Z.sqlite3
```

The backup is integrity-checked first; a damaged or newer-schema backup is
refused. The current database is kept aside as
`<db>.before-restore-<ms>`. Tested by
`the_database_is_backed_up_online_and_restored`.

**What a restore means for users:** vault writes after the backup are lost
at the server, but every enrolled device keeps its synced configuration
locally and re-uploads it on its next sync (its revision is newer than the
restored one). Accounts registered after the backup must register again.

## Data retention and deletion

| Data | Kept until |
| --- | --- |
| Account (email, password hash), devices, sessions, enrollments, recovery wrap, device wraps, vault | the account is deleted (`DELETE /v1/account`): removed immediately from the live database (`secure_delete` overwrites the pages) |
| The same in service backups | the backups age out: 7 days on the VM, 31 days in the backup bucket (ADR-047) |
| Audit events (event name, account id, device id, time) | kept after deletion with the account id only (pseudonymous); purge them with the backups' retention |
| Expired or revoked sessions | pruned a day after their refresh token expires |
| Verification and reset codes (hashed) | deleted on use or expiry |
| Outbox (queued mails, with codes) | sent: deleted at once; a temporary failure: retried for at most a day; a permanent refusal or a day without success: deleted. Without a relay (development) they stay |

Nothing in the service can decrypt the vault or the wraps: deleting an
account removes ciphertext only. Devices keep their local configuration.

## Email

Handlers queue verification codes, reset codes and notices through one
function (`db::queue_mail`) into the `outbox` table, in the same
transaction that decided to send them, and wake the delivery task
(`server/src/mail.rs`, ADR-044). The task sends each mail through the
configured SMTP relay (`lettre`, rustls with ring, the system's root
certificates), deletes it once the relay accepted it, retries a temporary
failure (4xx reply, network, TLS, timeout) after 30 s, doubling up to
15 minutes, and drops a mail after a permanent refusal (5xx, an unusable
address) or a day without success. Besides the wake-up it looks at the
outbox once a minute. Each mail opens its own connection.

Log lines name the mail's row id and the kind of failure only
(`sotyware-server: mail 12 not sent, attempt 1: temporary failure (smtp 451); trying again in 30 s`):
never the address, subject, body or the relay's reply text, which often
repeats the address.

Check the settings before starting the service:

```bash
sotyware-server send-test-mail you@example.com   # with the SOTYWARE_SMTP_* variables set
```

It sends one test mail directly (not through the outbox) and prints the
relay's full answer if it refuses. Without `SOTYWARE_SMTP_HOST`, `serve`
says that delivery is off and mails stay in the outbox;
`sotyware-server outbox` prints them (development only: they contain
codes).

Any relay that accepts SMTP submission works: a transactional email
provider (Brevo, Resend, Mailgun, Amazon SES, …) with a sender domain
authenticated by SPF, DKIM and DMARC, or a mail server on the same machine.

## External gates

Done on 2026-11-17 (ADR-045): hosting (a Google Cloud free-tier VM), the
domain `sotyware.com` with a Let's Encrypt certificate for
`accounts.sotyware.com`, an email provider (Brevo, SMTP), and the same
day encrypted off-host backups in Backblaze B2 with a monitor (ADR-047;
first upload and restore check that day). Still open:

- A rebuild on a scratch VM by
  [server-setup.md step 14](server-setup.md#14-if-the-vm-is-lost).
- An external security review of the service and of the vault design.
- A privacy notice (what the service stores; see the table above).

## Production deployment proposal (2026-11-15)

What it would take to run the service for real, written before the
deployment. The service was deployed on 2026-11-17 by following
[server-setup.md](server-setup.md), on Google Cloud's free tier instead
of a paid VM (ADR-045), with off-host backups in Backblaze B2 (ADR-047);
the privacy notice and the external review below are still open. Prices are rough orders of
magnitude to check when choosing; none was verified for this proposal.

### Is a separate database needed?

No. The service embeds SQLite (one file, WAL, `secure_delete`, online
backups with `sotyware-server backup`). For one instance this is the
simplest reliable choice: nothing to operate besides the file, and the
data are small (per account: a password hash, a few devices and sessions,
and one encrypted vault of at most 1 MiB, typically a few kB). PostgreSQL
would only pay off for several service instances behind a load balancer
(high availability) or above the documented limit of one connection
(about 20 sign-ins per second). Moving to it later means porting `db.rs`
(SQL is plain; the migrations are append-only) and the backup commands;
the API and the clients would not change. **Recommendation: SQLite now;
no database credentials are needed from you.**

### Services

| Service | Purpose | Choice to make |
| --- | --- | --- |
| One small Linux VM (or container host with a persistent volume) | runs `sotyware-server serve` as an unprivileged user on `127.0.0.1:8787` | provider and region (an EU region keeps the data in the EU) |
| Reverse proxy with automatic TLS | HTTPS only, HSTS, `X-Forwarded-For` overwritten (`SOTYWARE_SERVER_TRUST_PROXY=1`) | Caddy (automatic Let's Encrypt) is the smallest setup |
| Domain / subdomain and DNS | e.g. `accounts.<your domain>` pointing at the VM | the name; it is compiled into release builds |
| Transactional email | verification and reset codes, notices | an SMTP relay or an email API; a sender domain with SPF, DKIM and DMARC |
| Off-host backup storage | hourly `sotyware-server backup` copies, kept 30 days | a bucket at another provider (or another account), encrypted at rest there |
| Uptime monitoring | `GET /healthz` | any HTTP monitor |

**Code:** everything the service needs is implemented and tested in
process, mail delivery included since 2026-11-16 ([Email](#email);
tested against an SMTP server in the test process only, never through a
real provider).

### Schema (SQLite, `server/src/db.rs`, migrations 1 and 2)

`accounts` (id, email, Argon2id password hash, verified, lockout counters,
recovery wrap, key-proof hash, vault ciphertext with revision and
updater), `codes` (hashed verification and reset codes with expiry and
attempts), `devices` (name, platform, X25519 public key, state, sealed
account key), `sessions` (hashed access and refresh tokens, family for
reuse detection, expiry), `enrollments` (approval requests with the
sealed key), `audit` (event names with account and device ids), `outbox`
(mails to send, with failed attempts and the next attempt's time, migration 2). No column holds a provider credential, the account key
or the recovery key in plaintext.

### Endpoints (`/v1`, JSON, bearer tokens)

Registration and verification (`POST /v1/accounts`,
`/v1/accounts/verify`), sessions (`POST /v1/sessions`,
`/v1/sessions/refresh`, `DELETE /v1/sessions/current`), the account
(`GET`/`DELETE /v1/account`, `POST /v1/account/password`,
`GET /v1/account/events`), password reset (`/v1/password-reset/request`,
`/confirm`), the recovery wrap (`GET`/`PUT /v1/recovery`), the vault
(`GET`/`PUT /v1/vault` with optimistic concurrency), key rotation
(`POST /v1/keys/rotate`, one transaction), devices (`GET /v1/devices`,
`DELETE /v1/devices/{id}`, wraps), enrollment by approval
(`/v1/enrollments…`), and `GET /healthz`. Details:
[accounts.md](../architecture/accounts.md).

### Authentication and encryption

Email and password, Argon2id at the service; opaque random tokens stored as
SHA-256 (access 15 minutes, refresh 30 days, single use, reuse revokes the
family); verification codes by email; per-IP and per-account rate limits
with lockout. Storage credentials are synced only inside the vault, which
each device encrypts itself (XChaCha20-Poly1305 under a random account key
that the service never receives); the account key is wrapped by the user's
recovery key and sealed to each enrolled device's X25519 key (ADR-032). The
service operator, or anyone who steals the database or its backups, sees
ciphertext only. The trade-off chosen in ADR-032: a password alone never
unlocks the vault, so a new computer needs the recovery key or an approval
from an enrolled computer. A password-derived key (as password managers
do) would let the password alone unlock a new computer, at the price that
a weak or reused password, combined with a stolen database, exposes every
synced credential, and a password reset would lose the vault unless the
recovery key is kept anyway. Not implemented; a decision for the
maintainer.

### Deployment steps

1. Create the VM; a non-root user `sotyware`; firewall: 22 (restricted),
   80 and 443 only.
2. `cargo build --release --locked` in `server/` (or in CI) and install
   `target/release/sotyware-server` as `/usr/local/bin/sotyware-server`.
3. A systemd unit running `sotyware-server serve` with
   `SOTYWARE_SERVER_DB=/var/lib/sotyware/server.sqlite3`,
   `SOTYWARE_SERVER_ADDR=127.0.0.1:8787`, `SOTYWARE_SERVER_TRUST_PROXY=1`,
   `Restart=on-failure`, `NoNewPrivileges`, `ProtectSystem=strict`,
   `ReadWritePaths=/var/lib/sotyware`.
4. Caddy: `accounts.<domain> { reverse_proxy 127.0.0.1:8787 }` (TLS and
   HSTS automatic; set `header_up X-Forwarded-For {remote_host}`).
5. A systemd timer (hourly) running `sotyware-server backup` and copying
   the file off the host; a monthly restore test (`restore` on a scratch
   VM).
6. Set the `SOTYWARE_SMTP_*` and `SOTYWARE_MAIL_FROM` variables in a
   root-only environment file of the unit, check them with
   `send-test-mail`, and test registration end to end with a real address.
7. Set `PRODUCTION_SERVER` to `https://accounts.<domain>` in
   `src-tauri/src/account/mod.rs`, publish a privacy notice (what is
   stored: [Data retention](#data-retention-and-deletion)), have the
   service and the vault design reviewed externally, then release.

### Expected running costs (rough, to verify)

| Item | Order of magnitude |
| --- | --- |
| VM (2 vCPU, 2–4 GB RAM, 40 GB disk) | €4–10 per month |
| Domain | €10–20 per year (nothing if a subdomain of an existing one) |
| Email | free tiers cover low volumes (codes and notices only); paid plans start around $10–15 per month |
| Off-host backups | cents per month (30 hourly copies of a database of a few MB to a few hundred MB) |
| TLS certificates, monitoring | free (Let's Encrypt; a free uptime monitor) |

Roughly **€5–15 per month** for a small user base.

### What the maintainer would provide

1. A domain or subdomain and access to its DNS.
2. A VM (or the choice of a provider, and SSH access for the deployment).
3. An email-sending account with SMTP access and a verified sender domain
   (SPF, DKIM, DMARC records); any provider that offers SMTP works.
4. An off-host bucket and a write-only key for the backups.
5. The decisions above: SQLite (recommended) or PostgreSQL; keep the
   recovery-key model or add a password-derived unlock.
6. A privacy notice and an external review before any user signs up.

No secret is needed in this repository: the service has no signing or
encryption key of its own, and the email and backup credentials belong on
the host (environment or a root-only file), never in git.
