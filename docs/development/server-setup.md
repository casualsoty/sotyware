# Setting up the account service, step by step

A first deployment of `server/` (ADR-032) for a maintainer starting
without a domain, with free plans where they exist: a domain from a
registrar, **Brevo** for mail (SMTP, ADR-044), an **Oracle Cloud Always
Free** VM, **Caddy** for HTTPS. Background and the reasons for each
setting: [server.md](server.md). Written 2026-11-16 and **not yet
followed end to end**: provider menus change, so where a step names a
menu, look for the closest match; record what differed here.

Throughout, `example.com` stands for your domain, `accounts.example.com`
for the service's address and `you@example.org` for your own mailbox.

## What you end up with

| Piece | Where | Cost |
| --- | --- | --- |
| Domain `example.com` | a registrar (step 1) | about €10–15 a year |
| Mail from `no-reply@example.com` | Brevo free plan (300 mails a day) | free |
| Service at `https://accounts.example.com` | Oracle Cloud Always Free Arm VM, EU region | free within the Always Free limits |
| HTTPS certificate | Let's Encrypt, through Caddy | free |

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
   press *Verify* in Brevo. DNS can take minutes to an hour.
3. **Add the sender** `no-reply@example.com` under *Senders* (name:
   `sotyware`).
4. **Create an SMTP key:** under *SMTP & API* → *SMTP*, note the server
   (`smtp-relay.brevo.com`), the port (587) and your **login** (an address
   Brevo shows there, not your account email), then generate an **SMTP
   key**. Copy it once into your password manager: it is the
   `SOTYWARE_SMTP_PASSWORD`. Never commit it or paste it into a chat.

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
command prints its answer (wrong login or key, unverified sender, account
not yet activated).

## 4. Create the VM at Oracle Cloud

1. Sign up at oracle.com/cloud/free. A card is asked for verification.
   Choose an **EU home region** (Frankfurt, Amsterdam, Paris, …): it
   cannot be changed later, and Always Free resources exist only there.
2. **Upgrade the account to Pay As You Go** (*Billing* → *Upgrade*). Usage
   within the Always Free limits stays free, and Oracle stops reclaiming
   idle Always Free VMs, which this service would otherwise be. Add a
   budget alert (*Billing* → *Budgets*, e.g. €1) so any charge is noticed.
3. *Compute* → *Instances* → *Create instance*:
   - Image: **Canonical Ubuntu 24.04**.
   - Shape: **Ampere, VM.Standard.A1.Flex**, 2 OCPUs and 12 GB memory
     (within the free 4 OCPUs / 24 GB; 2 OCPUs make the build faster).
   - Networking: create a new VCN with a public subnet, and assign a
     public IPv4 address.
   - SSH keys: upload your public key (in WSL, `ssh-keygen -t ed25519` if
     you have none, then the contents of `~/.ssh/id_ed25519.pub`).
   If Oracle reports no capacity for this shape, try again later or in
   another availability domain.
4. Note the instance's **public IP address**.
5. **Open ports 80 and 443 in Oracle's firewall:** *Networking* → *Virtual
   cloud networks* → your VCN → the public subnet → its *Security List* →
   *Add Ingress Rules*: source `0.0.0.0/0`, protocol TCP, destination port
   `80`; again for `443`. Leave 22 as it is.
6. Log in: `ssh ubuntu@<public IP>`.
7. **Open the same ports in the VM's own firewall** (Oracle's Ubuntu
   images allow only SSH):

   ```bash
   sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 80 -j ACCEPT
   ```

   ```bash
   sudo iptables -I INPUT 6 -m state --state NEW -p tcp --dport 443 -j ACCEPT
   ```

   ```bash
   sudo netfilter-persistent save
   ```

8. Keep the system updated automatically:

   ```bash
   sudo apt update && sudo apt full-upgrade -y && sudo apt install -y unattended-upgrades
   ```

## 5. Point the name at the VM

On the registrar's DNS page add an **A record**: name `accounts`, value
the VM's public IP (at Cloudflare: DNS only). Check from WSL after a few
minutes:

```bash
getent hosts accounts.example.com
```

## 6. Build and install the service on the VM

On the VM:

```bash
sudo apt install -y build-essential git curl pkg-config
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal && . "$HOME/.cargo/env"
```

```bash
git clone https://github.com/casualsoty/sotyware.git && cd sotyware/server && cargo build --release --locked
```

(several minutes on the Arm VM). Then a system user, its data folder and
the binary:

```bash
sudo useradd --system --home-dir /var/lib/sotyware --shell /usr/sbin/nologin sotyware
```

```bash
sudo install -d -o sotyware -g sotyware -m 700 /var/lib/sotyware /var/lib/sotyware/backups
```

```bash
sudo install -m 755 target/release/sotyware-server /usr/local/bin/sotyware-server
```

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

Check mail from the VM with exactly these settings:

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
settings file was not read.

## 9. Put HTTPS in front with Caddy

```bash
sudo apt install -y caddy
```

```bash
sudo nano /etc/caddy/Caddyfile
```

Replace its contents with:

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
DNS record and both firewalls of step 4 open on port 80 and 443.

## 10. Check it end to end

From WSL:

```bash
curl -sS https://accounts.example.com/healthz
```

Expected: `{"status":"ok","database":"ok","api":"v1"}`. Then register
with your own address:

```bash
curl -sS https://accounts.example.com/v1/accounts -H 'content-type: application/json' -d '{"email":"you@example.org","password":"a long test passphrase"}'
```

Expected: `{"status":"check-your-email"}`, and within seconds a mail "Your
sotyware verification code" with an 8-digit code. Confirm it:

```bash
curl -sS https://accounts.example.com/v1/accounts/verify -H 'content-type: application/json' -d '{"email":"you@example.org","code":"<code>","password":"a long test passphrase"}'
```

To try the desktop app against it before any release, run a development
build pointed at it (debug builds only read this variable):

```bash
SOTYWARE_ACCOUNT_SERVER=https://accounts.example.com npm run tauri dev
```

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
sudo systemctl daemon-reload && sudo systemctl enable --now sotyware-backup.timer
```

These copies are on the same VM; a lost VM loses them too. Copying them
off the host (for example to a separate Backblaze bucket with a
write-only key) is the next step, described in
[server.md](server.md#backups-of-the-service).

## 12. Updating the service

```bash
cd ~/sotyware && git pull && cd server && cargo build --release --locked && sudo install -m 755 target/release/sotyware-server /usr/local/bin/sotyware-server && sudo systemctl restart sotyware-server
```

## 13. Connect release builds

Release builds talk only to the address compiled into them
(`PRODUCTION_SERVER` in `src-tauri/src/account/mod.rs`). Once steps 1–10
work, that constant becomes `https://accounts.example.com` in a reviewed
change, with its test updated. Before a release enables accounts for
other people, [server.md](server.md#external-gates) still asks for a
privacy notice and an external review.
