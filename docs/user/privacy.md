# Privacy notice

> **Draft of 2026-11-17, not published.** Written from the service's code
> (`server/src/db.rs`, `server/src/api.rs`, `server/src/mail.rs`) and its
> deployment (ADR-045, ADR-047). It is a gate before any release offers
> sotyware accounts ([roadmap](../status/roadmap.md)). The maintainer's
> answers of 2026-11-17 filled the points that were open; still to do
> before publishing: create the address `privacy@sotyware.com` and check
> that mail to it arrives, set the date below, and decide whether a lawyer
> should read it. The GDPR (Article 13) asks for the controller's
> identity; the notice names the maintainer only by the name sotyware is
> published under, which a lawyer should confirm is enough. Brevo's and
> Backblaze's locations and certifications were taken from their own
> pages on 2026-11-17. This draft is not legal advice.

This notice explains what sotyware, the desktop app, and the optional
sotyware account service at `https://accounts.sotyware.com` do with
information about you.

**Who is responsible:** sotyware's maintainer, an individual who
publishes sotyware as `casualsoty` on GitHub
([casualsoty/sotyware](https://github.com/casualsoty/sotyware)). For any
question about your information, or to use your rights, write to
**privacy@sotyware.com**.

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
  through **Brevo** (EU), and keeps encrypted backups at **Backblaze**
  (EU, Netherlands).
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
| GitHub, in your web browser | when you click **Send feedback** or **Report a bug** in Settings › About & updates | the app only opens a page in your browser to write a GitHub issue, with sotyware's version and your operating system (for example "Windows, x86_64") filled in for a bug. Nothing is sent until you submit the issue yourself, with your GitHub account; **issues are public**, so do not put personal information, storage keys or file names you want to keep private in them |

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

**Legal basis** (for the GDPR, Article 6(1)): performing the service you
signed up for (point b: your account, sync and the emails it needs); our
legitimate interest in keeping accounts secure and recoverable (point f:
the security log, rate limits, lockouts and the database backups).

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
| Brevo (Sendinblue SAS, France) | delivering the emails above: it receives your address and the email's text, including any code | the European Union, under Brevo's data processing agreement, which is part of its terms of use |
| Backblaze (Backblaze, Inc., United States) | storing hourly backups of the database, encrypted with a key that is not on the server | the European Union (region `eu-central-003`, Amsterdam, Netherlands) |
| Let's Encrypt | the service's HTTPS certificate (receives no information about you) | — |
| healthchecks.io | a monitor that the hourly backup ran (receives no information about you) | — |

**Transfers outside the European Economic Area:** the service's server
is in the United States, so your information is transferred there. The
transfer relies on Google's certification under the EU–US Data Privacy
Framework (with its UK Extension and the Swiss–US framework) and, should
that end, on the standard contractual clauses in Google Cloud's data
processing terms. The backups are stored in the European Union, but by
Backblaze, a United States company that is certified under the same
frameworks. Brevo processes your information in the European Union.

We do not sell or share information about you with anyone else.

### How long we keep it

| Information | Kept until |
| --- | --- |
| Your account, devices, sessions, vault and the encrypted key copies | you delete your account; then removed from the live database at once (its pages are overwritten) |
| The same in backups | the backups expire: within 7 days on the server, and 31 days in the backup storage, where a lock prevents deletion for 30 days |
| Expired or revoked sessions | a day after they expire |
| Verification and reset codes | used, or 15 minutes |
| Emails waiting to be sent | accepted by Brevo, or at most one day of failed attempts |
| The security log | 365 days. After you delete your account, its events stay under the account's random ID only, without your email address, until the last one is 31 days old, then they are removed |
| An account whose address was never verified | 7 days after registration (longer only while a verification code we just sent is still valid) |

Deleting your account does not change anything on your computers: your
storage accounts and backup jobs keep working there.

### Your choices and rights

- **See** your devices and remove (revoke) any of them on the Account
  page.
- **Change** your password, or reset it with a code sent to your email.
- **Delete** your account on the Account page. This removes your account,
  devices and vault at once; backups expire as described above.
- Under the GDPR, and similar laws where you live, you also have the
  right to **access** the information we hold about you and get a copy
  of it, to have it **corrected** or **erased**, to **restrict** or
  **object to** its use, and to receive it in a portable form. Write to
  **privacy@sotyware.com** from your account's email address (so that we
  know the request is yours); we answer within one month. We cannot give
  you your vault's contents: we cannot decrypt them, and your enrolled
  devices already hold them.
- You can **complain** to the data protection authority where you live
  or work.

### Security

All connections to the service use HTTPS. Passwords, codes and session
tokens are stored as hashes; repeated failed sign-ins lock the account
for up to 15 minutes; the database backups are encrypted before they
leave the server. Your vault is encrypted with XChaCha20-Poly1305 under a
key that only your devices and your recovery key can unlock. If you lose
every enrolled device and your recovery key, we cannot recover your
vault. Details: [accounts.md](../architecture/accounts.md).

## Children

You must be at least **16 years old** to create a sotyware account. If
we learn that an account belongs to someone younger, we delete it. The
app itself can be used without an account at any age.

## Changes to this notice

We publish every change in this file in the sotyware repository
(`casualsoty/sotyware`), where its history shows what changed. Changes
that affect what we collect, why, or who receives it are announced in the
release notes at least 30 days before they apply.

Last changed: **[open: the date it is published]**.
