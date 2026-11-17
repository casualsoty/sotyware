# Setting up the account service, step by step

A deployment of `server/` (ADR-032) for a maintainer starting without a
domain, with free plans where they exist: a domain from a registrar,
**Brevo** for mail (SMTP, ADR-044), a **Google Cloud free-tier e2-micro**
VM, **Caddy** for HTTPS (ADR-045). Background and the reasons for each
setting: [server.md](server.md).

The maintainer followed this guide on 2026-11-17 for
`https://accounts.sotyware.com`; the notes below record what differed from
the first draft. Provider menus change: where a step names a menu, look
for the closest match and record here what differed.

Throughout, `example.com` stands for your domain, `accounts.example.com`
for the service's address, `you@example.org` for your own mailbox and
`<VM IP>` for the VM's external address. **Replace every placeholder,**
including in the Caddyfile (step 9): a leftover `example.com` makes Caddy
request a certificate for someone else's domain.

## What you end up with

| Piece | Where | Cost |
| --- | --- | --- |
| Domain `example.com` | a registrar (step 1) | about €10–15 a year |
| Mail from `no-reply@example.com` | Brevo free plan (300 mails a day) | free |
| Service at `https://accounts.example.com` | Google Cloud e2-micro VM in a US free-tier region | free within the free tier |
| HTTPS certificate | Let's Encrypt, through Caddy | free |

The free tier's VM is in the US, so the account data (email, device
names, encrypted vaults) are stored there; the privacy notice must say so.

## 1. Buy a domain

1. Pick a registrar that sells at or near cost and includes DNS:
   **Cloudflare Registrar** (at cost; DNS at Cloudflare; Brevo can set its
   records there automatically) or **Porkbun**. A `.com` is about
   $10–11 a year; avoid "first year" discounts that renew expensively.
2. Buy the domain, turn on auto-renewal and the registrar's WHOIS privacy.
3. Find the DNS page for the domain; you will add records there in steps
   2 and 5. At Cloudflare, every record you add for this guide must be
   **DNS only** (grey cloud), not proxied.

## 2. Set up Brevo for sending mail

1. Create a free account at brevo.com. Brevo may ask for details about
   your use and review the account before it can send; answer that it
   sends account verification codes and security notices for an app.
2. **Authenticate the domain:** in Brevo's settings, under *Senders,
   Domains & Dedicated IPs* → *Domains*, add `example.com` and choose to
   authenticate it. Brevo shows DNS records (a `brevo-code` TXT record,
   DKIM records and a DMARC record). Add each exactly as shown on your
   registrar's DNS page (at Cloudflare, Brevo can add them itself), then
   press *Verify* in Brevo. DNS can take minutes to an hour. In the
   *Name* field, type only the part before your domain (`brevo1._domainkey`,
   not `brevo1._domainkey.example.com`); keep one SPF and one DMARC record.
3. **Add the sender** `no-reply@example.com` under *Senders* (name:
   `sotyware`).
4. **Create an SMTP key:** under *SMTP & API* → *SMTP*, note the server
   (`smtp-relay.brevo.com`), the port (587) and your **login** (an address
   Brevo shows there, not your account email), then generate an **SMTP
   key**. Copy it once into your password manager: it is the
   `SOTYWARE_SMTP_PASSWORD`. Never commit it or paste it into a chat.

**Authorised IPs:** Brevo accepts SMTP only from IP addresses you have
authorised. A new address is refused with `525 5.7.1 Unauthorized IP
address`, and Brevo mails you a link to authorise it; or add it under your
account name → *Security* → *Authorised IPs*. Authorise your own address
for step 3 and the VM's for step 7.

## 3. Check mail from this computer first

Before any server exists, check that Brevo accepts mail from sotyware.
In WSL, from the repository:

```bash
cd server && cargo build --release --locked
```

```bash
export SOTYWARE_SMTP_HOST=smtp-relay.brevo.com SOTYWARE_SMTP_USERNAME='<login from step 2.4>' SOTYWARE_MAIL_FROM='sotyware <no-reply@example.com>'
```

```bash
read -rs SOTYWARE_SMTP_PASSWORD && export SOTYWARE_SMTP_PASSWORD
```

(type or paste the SMTP key, then Enter; `read -s` keeps it out of the
screen and the shell history)

```bash
./target/release/sotyware-server send-test-mail you@example.org
```

Expected: `the relay smtp-relay.brevo.com:587 accepted the test mail`, and
a mail "sotyware test mail" in your inbox. If it went to spam, the domain
authentication of step 2.2 is not complete yet. If the relay refused, the
command prints its answer (wrong login or key, unverified sender, an IP
not yet authorised, account not yet activated).

## 4. Create the VM at Google Cloud

The first draft of this guide used Oracle Cloud's Always Free tier; on
2026-11-17 its EU region had no capacity for either free shape (`Out of
capacity for shape VM.Standard.A1.Flex` and `…E2.1.Micro`), so the
maintainer used Google Cloud's free tier instead.

1. Sign up at cloud.google.com/free. A card is asked for verification.
   New accounts get a 90-day trial credit; when it ends, **activate the
   full account** when asked, or the VM is stopped. The free tier stays
   free on a full account.
2. **Budget alert:** *Billing* → *Budgets & alerts* → *Create budget*,
   amount 1, alerts at 50, 90 and 100 %.
3. *Compute Engine* → *VM instances* (enable the Compute Engine API the
   first time) → *Create instance*. The free tier covers only these exact
   choices:

   | Setting | Choose |
   | --- | --- |
   | Region | **us-central1**, **us-east1** or **us-west1** (no other) |
   | Machine | series **E2**, type **e2-micro** (2 shared vCPUs, 1 GB) |
   | Boot disk | **Ubuntu 24.04 LTS (x86/64, amd64)**, type **Standard persistent disk** (the default *Balanced* is billed), **30 GB** |
   | Snapshot schedule | **none** (snapshot storage is billed; step 11 backs up the database itself) |
   | Firewall | ✔ **Allow HTTP traffic**, ✔ **Allow HTTPS traffic** |
   | Networking | as offered (an ephemeral external IPv4) |

   The form's monthly estimate shows the e2-micro's list price (about
   $6): **it does not subtract the free tier**, which appears on the bill
   as a matching credit. The disk line must show $0.00. Two days later,
   check *Billing* → *Reports*: the net cost must be 0.
4. **SSH key:** in WSL, `ssh-keygen -t ed25519` if `~/.ssh/id_ed25519.pub`
   does not exist, then paste the contents of that `.pub` file under the
   VM's *Edit* → *Security and access* → *SSH keys* → *Add item*. The
   username is the end of the key (`<user>@…`).
5. Note the VM's **External IP**, then log in: `ssh <user>@<VM IP>` (answer
   `yes` to the fingerprint question the first time).
6. Keep the system updated automatically. Right after the first boot the
   VM runs its own package updates; a `Waiting for cache lock` message
   means waiting for them (minutes on an e2-micro), not interrupting:

   ```bash
   sudo apt update && sudo apt full-upgrade -y && sudo apt install -y unattended-upgrades
   ```

7. Add 1 GB of swap as a margin for the 1 GB of memory:

   ```bash
   sudo fallocate -l 1G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile && echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
   ```

Google's Ubuntu image has no firewall on the VM itself; the two firewall
boxes of step 4.3 open ports 80 and 443.

## 5. Point the name at the VM

On the registrar's DNS page add an **A record**: name `accounts`, value
the VM's external IP (at Cloudflare: DNS only). Check from WSL after a few
minutes:

```bash
getent hosts accounts.example.com
```

## 6. Build in WSL and install on the VM

The e2-micro is too small to compile the service, so build it in WSL
(Ubuntu 24.04 on x86-64, like the VM; the binary needs glibc 2.34 or
later):

```bash
cd server && cargo build --release --locked
```

```bash
scp target/release/sotyware-server <user>@<VM IP>:
```

On the VM, a system user, its data folder and the binary:

```bash
sudo useradd --system --home-dir /var/lib/sotyware --shell /usr/sbin/nologin sotyware
```

```bash
sudo install -d -o sotyware -g sotyware -m 700 /var/lib/sotyware /var/lib/sotyware/backups
```

```bash
sudo install -m 755 ~/sotyware-server /usr/local/bin/sotyware-server && rm ~/sotyware-server
```

Compare `sha256sum /usr/local/bin/sotyware-server` with the same command
on the file in WSL.

## 7. Configure it

Create the settings file, readable by root only (systemd reads it before
starting the service as `sotyware`):

```bash
sudo install -d -m 700 /etc/sotyware && sudo install -m 600 /dev/null /etc/sotyware/server.env && sudo nano /etc/sotyware/server.env
```

Contents (your values in place of the placeholders):

```ini
SOTYWARE_SERVER_DB=/var/lib/sotyware/server.sqlite3
SOTYWARE_SERVER_ADDR=127.0.0.1:8787
SOTYWARE_SERVER_TRUST_PROXY=1
SOTYWARE_SMTP_HOST=smtp-relay.brevo.com
SOTYWARE_SMTP_PORT=587
SOTYWARE_SMTP_SECURITY=starttls
SOTYWARE_SMTP_USERNAME=<login from step 2.4>
SOTYWARE_SMTP_PASSWORD=<SMTP key from step 2.4>
SOTYWARE_MAIL_FROM="sotyware <no-reply@example.com>"
```

Keep the double quotes on the last line (`<` and `>` would otherwise break
it). Authorise the VM's IP at Brevo (step 2), then check mail from the VM
with exactly these settings:

```bash
sudo sh -c 'set -a; . /etc/sotyware/server.env; exec /usr/local/bin/sotyware-server send-test-mail you@example.org'
```

## 8. Run it as a service

```bash
sudo nano /etc/systemd/system/sotyware-server.service
```

```ini
[Unit]
Description=sotyware account service
After=network-online.target
Wants=network-online.target

[Service]
User=sotyware
Group=sotyware
EnvironmentFile=/etc/sotyware/server.env
ExecStart=/usr/local/bin/sotyware-server serve
Restart=on-failure
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/sotyware

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now sotyware-server
```

```bash
journalctl -u sotyware-server -n 20
```

Expected: `mail delivery through smtp-relay.brevo.com:587 (StartTls)` and
`listening on 127.0.0.1:8787`. If it says mail delivery is off, the
settings file was not read. `curl -sS http://127.0.0.1:8787/healthz` on
the VM answers `{"status":"ok",…}`.

## 9. Put HTTPS in front with Caddy

```bash
sudo apt install -y caddy
```

```bash
sudo nano /etc/caddy/Caddyfile
```

Replace its contents with (your name on the first line):

```text
accounts.example.com {
    reverse_proxy 127.0.0.1:8787 {
        header_up X-Forwarded-For {remote_host}
    }
    header Strict-Transport-Security "max-age=31536000"
}
```

```bash
sudo systemctl reload caddy
```

Caddy gets a Let's Encrypt certificate on its own; this needs step 5's
DNS record and ports 80 and 443 open. Until it has one, HTTPS fails with
`tlsv1 alert internal error`; `journalctl -u caddy -n 40` shows why
(look for the name it requested: it must be yours).

## 10. Check it end to end

From WSL:

```bash
curl -sS https://accounts.example.com/healthz
```

Expected: `{"api":"v1","database":"ok","status":"ok"}`. Then register
with your own address, either in the app (below) or with `curl`, the
password read without echo:

```bash
read -rs PW && export PW
```

```bash
curl -sS https://accounts.example.com/v1/accounts -H 'content-type: application/json' -d "{\"email\":\"you@example.org\",\"password\":\"$PW\"}"
```

Expected: `{"status":"check-your-email"}`, and within seconds a mail "Your
sotyware verification code" with an 8-digit code. Confirm it:

```bash
curl -sS https://accounts.example.com/v1/accounts/verify -H 'content-type: application/json' -d "{\"email\":\"you@example.org\",\"code\":\"<code>\",\"password\":\"$PW\"}"
```

To try the desktop app against it, a development build uses
`PRODUCTION_SERVER` (step 13) or `SOTYWARE_ACCOUNT_SERVER`:

```bash
SOTYWARE_ACCOUNT_SERVER=https://accounts.example.com npm run tauri dev
```

On Linux the app needs a Secret Service (GNOME Keyring or KWallet) to keep
the session; the WSL2 used on 2026-11-17 had none running.

## 11. Backups

Hourly online backups kept 30 days on the VM:

```bash
sudo nano /etc/systemd/system/sotyware-backup.service
```

```ini
[Unit]
Description=sotyware account service backup

[Service]
Type=oneshot
User=sotyware
Environment=SOTYWARE_SERVER_DB=/var/lib/sotyware/server.sqlite3
ExecStart=/bin/sh -c '/usr/local/bin/sotyware-server backup /var/lib/sotyware/backups/server-$(date -u +%%Y%%m%%dT%%H%%M%%SZ).sqlite3 && find /var/lib/sotyware/backups -name "server-*.sqlite3" -mtime +30 -delete'
```

```bash
sudo nano /etc/systemd/system/sotyware-backup.timer
```

```ini
[Unit]
Description=Hourly sotyware account service backup

[Timer]
OnCalendar=hourly
Persistent=true

[Install]
WantedBy=timers.target
```

```bash
sudo systemctl daemon-reload && sudo systemctl enable --now sotyware-backup.timer && sudo systemctl start sotyware-backup.service && sudo ls -l /var/lib/sotyware/backups
```

These copies are on the same VM; a lost VM loses them too. Copying them
off the host (for example to a separate Backblaze bucket with a
write-only key) is the next step, described in
[server.md](server.md#backups-of-the-service).

## 12. Updating the service

In WSL:

```bash
git pull && cd server && cargo build --release --locked && scp target/release/sotyware-server <user>@<VM IP>:
```

On the VM:

```bash
sudo install -m 755 ~/sotyware-server /usr/local/bin/sotyware-server && rm ~/sotyware-server && sudo systemctl restart sotyware-server
```

## 13. Connect release builds

Release builds talk only to the address compiled into them
(`PRODUCTION_SERVER` in `src-tauri/src/account/mod.rs`); since 2026-11-17
it is `https://accounts.sotyware.com` (ADR-045). Test-profile builds never
use it: a desktop test reaches only a service its run names in
`SOTYWARE_ACCOUNT_SERVER`. Before a release enables accounts for other
people, [server.md](server.md#external-gates) still asks for a privacy
notice and an external review.
