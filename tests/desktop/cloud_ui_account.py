# The account phase of cloud_ui.py (ADR-032, ADR-049), run by it with exec()
# in its own namespace (pid, check, press, text, nodes, by_role, buttons,
# fill, wait_for, link, PROFILE, ...). Kept in its own file only for length.
#
# Drives the release binary (a test-profile build, ADR-027) against a
# local account service: the `sotyware-server` binary of this repository,
# started by cloud-ui.sh on 127.0.0.1 with its own database and no mail
# relay, so its mails stay in the outbox, where this phase reads the
# verification code (`sotyware-server outbox`). The app reaches it through
# SOTYWARE_ACCOUNT_SERVER, which only debug and test-profile builds read.
#
# First the GitHub pages (ADR-049): Settings › About & updates' Send
# feedback, Report a bug and Privacy notice, and the account form's
# Privacy notice. A test-profile build records the address it would open
# in `opened-pages.txt` in the profile instead of starting a browser, so
# the exact URLs are checked and nothing reaches GitHub. Then sign-up as a
# person would: the age line, register, the code from the mail, verify and
# sign in, set up sync with the recovery key shown once and typed again,
# and delete the account; after each step the service's own database says
# what it stored.
import re
import sqlite3

SERVER_BIN = os.environ["SOTYWARE_UI_ACCOUNT_SERVER_BIN"]
SERVER_DB = os.environ["SOTYWARE_UI_ACCOUNT_SERVER_DB"]
OPENED = os.path.join(PROFILE.root, "opened-pages.txt")
REPO_URL = "https://github.com/casualsoty/sotyware"
EMAIL = f"e2e-{RUN}@example.org"
PASSWORD = "e2e passphrase long enough"


def opened():
    try:
        with open(OPENED, encoding="utf-8") as fh:
            return [line.strip() for line in fh if line.strip()]
    except FileNotFoundError:
        return []


def page_tab(name):
    for n, node in by_role(pid, Atspi.Role.PAGE_TAB):
        if n.strip() == name:
            return Atspi.Action.do_action(node, 0)
    print(f"  no tab {name!r}; tabs: {[n for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)]}", flush=True)
    return False


def mode_buttons():
    """The account form's mode buttons (Sign in, Create account, Forgot
    password): toggle buttons to AT-SPI, as they carry aria-pressed."""
    return [n.strip() for n, _ in by_role(pid, Atspi.Role.TOGGLE_BUTTON)]


def press_mode(name):
    return act(pid, (Atspi.Role.TOGGLE_BUTTON,), name)


def press_last(name):
    """Presses the last button named `name` (the account form has a mode
    button and a submit button both called "Create account")."""
    found = [node for n, node in by_role(pid, Atspi.Role.PUSH_BUTTON) if n.strip() == name]
    return bool(found) and Atspi.Action.do_action(found[-1], 0)


def server_rows():
    """(email, verified, has recovery wrap, has vault) of every account."""
    with sqlite3.connect(f"file:{SERVER_DB}?mode=ro", uri=True) as db:
        return db.execute("SELECT email, verified, recovery_wrap IS NOT NULL, vault IS NOT NULL "
                          "FROM accounts ORDER BY email").fetchall()


def outbox_code():
    """The newest verification code mailed to EMAIL, read from the
    service's outbox (no relay is configured)."""
    out = subprocess.run([SERVER_BIN, "outbox"], env={**os.environ, "SOTYWARE_SERVER_DB": SERVER_DB},
                         capture_output=True, text=True, timeout=30)
    codes = []
    for mail in out.stdout.split("\n---"):
        if f"To: {EMAIL}" in mail:
            codes += re.findall(r"Your verification code is (\d+)\.", mail)
    return codes[-1] if codes else None


def shown_recovery_key():
    for role, _, t, _ in nodes(pid):
        m = re.fullmatch(r"\s*([0-9A-Z]{4}(?:-[0-9A-Z]{4}){7,})\s*", t or "")
        if m:
            return m.group(1)
    return None


# ---- the GitHub pages ----
ok = link(pid, "Settings") and wait_for(lambda: any(n.strip() == "About & updates"
                                                   for n, _ in by_role(pid, Atspi.Role.PAGE_TAB)), 10)
ok = ok and page_tab("About & updates") and wait_for(lambda: "Send feedback" in [b.strip() for b in buttons(pid)], 10)
check("Settings › About & updates shows the feedback buttons", bool(ok), [b.strip() for b in buttons(pid)][:30])
check("no page was opened before a click", opened() == [], opened())
ok = press(pid, "Send feedback") and wait_for(lambda: len(opened()) == 1, 10)
ok = ok and press(pid, "Report a bug") and wait_for(lambda: len(opened()) == 2, 10)
ok = ok and press(pid, "Privacy notice") and wait_for(lambda: len(opened()) == 3, 10)
pages = opened()
check("Send feedback opens the feedback form", bool(ok) and pages[:1] == [
    f"{REPO_URL}/issues/new?template=feedback.yml"], pages)
bug = pages[1] if len(pages) > 1 else ""
check("Report a bug opens the bug form with the version and 'Linux, <arch>' filled in, nothing else",
      re.fullmatch(re.escape(f"{REPO_URL}/issues/new?template=bug_report.yml&version=")
                   + r"\d+\.\d+\.\d+&os=Linux%2C%20[a-z0-9_]+", bug) is not None, bug)
check("Privacy notice opens the notice on main", pages[2:3] == [f"{REPO_URL}/blob/main/docs/user/privacy.md"], pages)
check("a GitHub page records no browser error", "could not be" not in text(pid), text(pid)[-300:])

# ---- sign-up ----
ok = link(pid, "Account") and wait_for(lambda: "Create account" in mode_buttons(), 15)
check("the Account page offers Create account (the service is configured)", bool(ok),
      text(pid)[-400:])
check("the age line is shown only when creating an account", "at least 16" not in text(pid))
ok = press_mode("Create account") and wait_for(lambda: "at least 16" in text(pid), 10)
check("Create account names the minimum age of 16 and links the privacy notice",
      bool(ok) and "Privacy notice" in [b.strip() for b in buttons(pid)], text(pid)[-400:])
ok = press(pid, "Privacy notice") and wait_for(lambda: len(opened()) == 4, 10)
check("the sign-up form's Privacy notice opens the notice, and sends nothing to the service",
      bool(ok) and opened()[3:] == [f"{REPO_URL}/blob/main/docs/user/privacy.md"] and server_rows() == [],
      (opened(), server_rows()))
ok = fill(pid, "Email", EMAIL) and fill(pid, "Password", PASSWORD) and press_last("Create account")
ok = ok and wait_for(lambda: "If this address can be registered" in text(pid), 30)
check("register: the page asks for the code from the email", bool(ok), text(pid)[-400:])
check("the service stored the account, not yet verified", server_rows() == [(EMAIL, 0, 0, 0)], server_rows())
code = wait_for(outbox_code, 15)
check("the verification code is in the service's outbox", code is not None)
ok = bool(code) and fill(pid, "Code from the email", code) and press(pid, "Verify and sign in")
ok = ok and wait_for(lambda: "Signed in as" in text(pid) and "Set up sync" in [b.strip() for b in buttons(pid)], 60)
check("verify and sign in: signed in, sync not set up yet", bool(ok), text(pid)[-400:])
check("the service marks the address verified", server_rows() == [(EMAIL, 1, 0, 0)], server_rows())
ok = press(pid, "Set up sync") and wait_for(shown_recovery_key, 60)
key_shown = shown_recovery_key()
check("Set up sync shows the recovery key once", key_shown is not None, text(pid)[-400:])
ok = bool(key_shown) and fill(pid, "Type it again to confirm you saved it", key_shown) and press(pid, "I saved it")
ok = ok and wait_for(lambda: shown_recovery_key() is None and "Last sync" in text(pid), 60)
check("typing the key again hides it; the page shows the synced state", bool(ok), text(pid)[-400:])
rows_now = server_rows()
check("the service holds the recovery wrap and an encrypted vault", rows_now == [(EMAIL, 1, 1, 1)], rows_now)
with sqlite3.connect(f"file:{SERVER_DB}?mode=ro", uri=True) as sdb:
    vault = sdb.execute("SELECT vault FROM accounts WHERE email = ?", (EMAIL,)).fetchone()[0] or ""
check("the vault at the service holds no recovery key, password or address in clear",
      all(s not in vault for s in (key_shown or "-", PASSWORD, EMAIL)))

# ---- delete ----
ok = press(pid, "Delete account…") and wait_for(lambda: "Password" in fields(pid), 10)
ok = ok and fill(pid, "Password", PASSWORD) and press(pid, "Delete account")
ok = ok and wait_for(lambda: "The sotyware account was deleted" in text(pid), 60)
check("Delete account with the password: the page says it was deleted", bool(ok), text(pid)[-400:])
check("the service no longer has the account", server_rows() == [], server_rows())
ok = wait_for(lambda: "Create account" in mode_buttons(), 15)
check("the page offers sign-in and Create account again", bool(ok))
logtext_now = open(log, errors="replace").read()
check("neither the password nor the recovery key reached sotyware's output",
      PASSWORD not in logtext_now and (key_shown is None or key_shown not in logtext_now), "(output withheld)")
