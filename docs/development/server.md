# Operating the account service

`server/` is sotyware's account and end-to-end encrypted sync service
(ADR-032; design and API: [accounts.md](../architecture/accounts.md)). It is a
separate Rust crate with its own `Cargo.lock`, not part of the desktop app's
build. **No production instance exists**: hosting, a domain, a TLS
certificate, an email provider and backup storage are external decisions
(see [External gates](#external-gates)). Everything below is tested locally.

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

The service has no secret of its own: no signing key, no encryption key.
Tokens are random and stored as SHA-256 hashes; passwords as Argon2id
(19 MiB, 2 passes, 1 lane).

## Deploying (outline; not done)

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
Schedule it (hourly suggested), copy the files off the host (another
provider; encrypted at rest there), and keep them for 30 days.

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
| The same in service backups | the backups age out (30 days suggested) |
| Audit events (event name, account id, device id, time) | kept after deletion with the account id only (pseudonymous); purge them with the backups' retention |
| Expired or revoked sessions | pruned a day after their refresh token expires |
| Verification and reset codes (hashed) | deleted on use or expiry |
| Development outbox (mails with codes) | development only; production would send mail instead |

Nothing in the service can decrypt the vault or the wraps: deleting an
account removes ciphertext only. Devices keep their local configuration.

## Email

The service queues verification codes, reset codes and notices through one
function (`db::queue_mail`) into an outbox table. `sotyware-server outbox`
prints it (development only: it contains codes). **Production email
delivery is not implemented**: it needs an SMTP server or a transactional
email provider (an external gate), connected where `queue_mail` is called.

## External gates

- Hosting (a VM or container host), a domain and its TLS certificate.
- An email provider (SMTP or API) for verification, reset and notices.
- Off-host storage for the service's backups, and their schedule.
- An external security review of the service and of the vault design.
- A privacy notice (what the service stores; see the table above).
