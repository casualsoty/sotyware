# Privacy notice

> **Draft of 2026-11-17, not published.** Written from the service's code
> (`server/src/db.rs`, `server/src/api.rs`, `server/src/mail.rs`) and its
> deployment (ADR-045, ADR-047). It is a gate before any release offers
> sotyware accounts ([roadmap](../status/roadmap.md)). Before publishing,
> the maintainer has to fill in every point marked **[open]** and decide
> whether a lawyer should read it. This draft is not legal advice.

This notice explains what sotyware, the desktop app, and the optional
sotyware account service at `https://accounts.sotyware.com` do with
information about you.

**Who is responsible:** **[open: the maintainer's name or legal entity,
postal address, and a contact email address for privacy questions]**.

## In short

- The app has **no telemetry, analytics or advertising**. Using sotyware
  without an account sends nothing about you to us.
- A sotyware account is **optional**. You only need one to sync your
  storage accounts and backup jobs between your computers.
- What you sync is **end-to-end encrypted** on your computer before it
  leaves it. We cannot read your storage keys, bucket names or backup
  jobs, and neither can anyone who gets a copy of our database.
- What we can read: your **email address**, the **names and operating
  systems you give your computers**, and **when** things happen in your
  account.
- The service runs in the **United States** (Google Cloud), sends email
  through **Brevo**, and keeps encrypted backups at **Backblaze**.
- You can **delete your account** from the app at any time.

## The app without an account

sotyware runs on your computer. Your storage accounts' settings, the
operation journal, backup jobs, folder syncs, preferences and window
position are stored in files on your computer. Your storage keys are
stored in your operating system's credential store (Windows Credential
Manager, the macOS login keychain or the Linux Secret Service). None of
this is sent to us.

The app connects to other parties' servers only to do what you asked:

| Connection | When | What the other party sees |
| --- | --- | --- |
| Your storage provider (Backblaze B2, Hetzner or another S3-compatible service) | when you browse, transfer, back up or sync files, and once at startup for each storage account, to check that it still connects | your IP address, your storage key's ID and the requests, under that provider's own privacy policy. sotyware sends these requests directly; they never pass through us |
| GitHub (`github.com` and its download servers) | checks for updates, if the "update-check" feature is on, and downloads of an update you chose to install | your IP address and an ordinary download request, under GitHub's privacy policy. No identifier of your installation is sent |

## The sotyware account service

### What we store, and why

| Information | Why | Can we read it? |
| --- | --- | --- |
| Your email address | to identify your account, send verification and password reset codes, and notify you of security events | yes |
| Your password | to sign you in | no: stored only as an Argon2id hash. It is sent to the service over HTTPS when you sign in |
| The account's creation time, failed sign-in count and any temporary lockout | to protect the account against password guessing | yes |
| For each computer you sign in on: the name you gave it (default "This computer"), its operating system (Windows, macOS or Linux), a public key, its state (waiting, enrolled, revoked), when it was added and last seen | to show you your devices, let you approve or revoke them, and deliver the encrypted account key to the right one | yes |
| Sign-in sessions: random tokens, stored only as hashes, with their expiry times | to keep you signed in | no (hashes only) |
| Your **vault**: your synced storage accounts (with their keys) and backup jobs | to sync them between your computers | **no**: encrypted on your computer with a random account key we never receive. We see only its size, its revision number, when it changed and which of your devices changed it |
| Copies of the account key, encrypted under your recovery key and under each enrolled device's key, and a check value of the account key | so that a new computer can unlock the vault with your recovery key or with another device's approval | no. The recovery key is shown once on your computer and never sent to us |
| Requests to add a new computer (approval requests) | so that another of your devices can approve it | the request, yes; the key it carries is encrypted |
| Verification and reset codes | to confirm your address and reset your password | no: stored only as hashes, and they expire after 15 minutes |
| A security log of your account: event names (for example "sign-in", "sign-in-failed", "password-changed", "vault-written"), the time and the device ID | to detect misuse and investigate incidents | yes. It contains no IP address and no email address |
| Emails waiting to be sent: address, subject and text | to deliver them | yes, until the email provider accepts them |

**IP addresses** are used only in the service's memory to limit how often
one address may register, verify, sign in or refresh a session. They are
not written to the database or to the service's logs. The web server in
front of the service (Caddy) does not keep access logs.

**Legal basis** **[open: confirm]**: performing the service you signed up
for (account, sync, the emails it needs); our legitimate interest in
keeping accounts secure (the security log, rate limits, lockouts).

### Emails we send

Only these, and never marketing:

- your verification code when you register;
- a notice when someone tries to register with an address that already
  has an account;
- a notice when a new computer signs in to your account;
- your password reset code, when you ask for one;
- a notice when your password is changed;
- a notice when your account is deleted.

### Who processes the information for us

| Service provider | What for | Where |
| --- | --- | --- |
| Google Cloud | the server that runs the service and its database | United States (`us-east1`, South Carolina) |
| Brevo | delivering the emails above: it receives your address and the email's text, including any code | **[open: Brevo's processing location and data processing agreement]** |
| Backblaze | storing hourly backups of the database, encrypted with a key that is not on the server | **[open: the backup bucket's region]** |
| Let's Encrypt | the service's HTTPS certificate (receives no information about you) | — |
| healthchecks.io | a monitor that the hourly backup ran (receives no information about you) | — |

**Transfers outside your country** **[open]**: the service runs in the
United States. If you live in the European Economic Area, the United
Kingdom or Switzerland, name the legal mechanism the transfer relies on
(for example the EU–US Data Privacy Framework certification of Google
and Backblaze, or standard contractual clauses).

We do not sell or share information about you with anyone else.

### How long we keep it

| Information | Kept until |
| --- | --- |
| Your account, devices, sessions, vault and the encrypted key copies | you delete your account; then removed from the live database at once (its pages are overwritten) |
| The same in backups | the backups expire: within 7 days on the server, and 31 days in the backup storage, where a lock prevents deletion for 30 days |
| Expired or revoked sessions | a day after they expire |
| Verification and reset codes | used, or 15 minutes |
| Emails waiting to be sent | accepted by Brevo, or at most one day of failed attempts |
| The security log | **[open: today it is kept with no time limit, also after your account is deleted, under the account's random ID only. Decide a period and implement the purge, or state that it is kept.]** |
| An account whose address was never verified | **[open: today it is kept until someone deletes it. Decide a period and implement the purge, or state that it is kept.]** |

Deleting your account does not change anything on your computers: your
storage accounts and backup jobs keep working there.

### Your choices and rights

- **See** your devices and remove (revoke) any of them on the Account
  page.
- **Change** your password, or reset it with a code sent to your email.
- **Delete** your account on the Account page. This removes your account,
  devices and vault at once; backups expire as described above.
- **[open: the rights that apply where the maintainer and users are,
  for example access, rectification, erasure, restriction, portability
  and objection under the GDPR, how to exercise them through the contact
  address above, and the right to complain to a data protection
  authority.]**

### Security

All connections to the service use HTTPS. Passwords, codes and session
tokens are stored as hashes; repeated failed sign-ins lock the account
for up to 15 minutes; the database backups are encrypted before they
leave the server. Your vault is encrypted with XChaCha20-Poly1305 under a
key that only your devices and your recovery key can unlock. If you lose
every enrolled device and your recovery key, we cannot recover your
vault. Details: [accounts.md](../architecture/accounts.md).

## Children

sotyware accounts are not meant for children **[open: minimum age, for
example 16 in the EEA]**.

## Changes to this notice

We will publish changes in this file in the sotyware repository
(`casualsoty/sotyware`) and mention important changes in the release
notes. **[open: say whether account holders will be emailed about
material changes.]**

Last changed: **[open: the date it is published]**.
