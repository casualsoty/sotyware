// The connection check of every storage account, run once when the
// dashboard starts, so the user sees whether each account works without
// clicking "Check connection". It is the same command as the button
// (`check_cloud_account`: one `HeadBucket`, a one-key listing only after a
// refused HEAD, never a write). Rust keeps check results in memory only, so
// after a restart every account starts unchecked.
//
// Accounts are checked one after another: each check loads the key from the
// OS credential store, and on Linux a locked keyring opens an unlock prompt.
// If the store turns out to be locked or unavailable, the remaining accounts
// are left unchecked instead of prompting once per account; their "Check
// connection" button still works.
import { checkAccount, type AccountInfo, type CheckStatus } from "./cloud";

/** Results after which the other accounts are not checked: they would need
 * the same credential store. */
const STOP_AFTER: readonly CheckStatus[] = ["credential-store-locked", "credential-store-unavailable"];

/** Whether the startup check should check `account`: never checked, and
 * checkable at all (a bucket and usable settings). */
export function needsStartupCheck(account: AccountInfo): boolean {
  return account.lastCheck === null && account.bucket !== null && account.configError === null;
}

/** `list` with `info`'s check in place of its account's, unless that
 * account is gone, has changed since (its check was dropped), or already
 * shows a newer check (the user pressed "Check connection" meanwhile). */
export function mergeCheck(list: readonly AccountInfo[], info: AccountInfo): AccountInfo[] {
  return list.map((a) => {
    if (a.id !== info.id || info.lastCheck === null) {
      return a;
    }
    if (a.lastCheck !== null && a.lastCheck.checkedAt >= info.lastCheck.checkedAt) {
      return a;
    }
    return info;
  });
}

export class StartupChecks {
  /** The account being checked now, if any. */
  current = $state<string | null>(null);
  /** Every result so far, by account id (the Cloud Storage page merges them
   * into its own list). */
  results = $state<Record<string, AccountInfo>>({});
  private started = false;

  constructor(private readonly check: (id: string) => Promise<AccountInfo> = checkAccount) {}

  /** Checks each account that needs it, once per dashboard start; later
   * calls do nothing. `onresult` gets every account as the check returns it. */
  async run(accounts: readonly AccountInfo[], onresult: (info: AccountInfo) => void): Promise<void> {
    if (this.started) {
      return;
    }
    this.started = true;
    for (const account of accounts.filter(needsStartupCheck)) {
      this.current = account.id;
      let info: AccountInfo;
      try {
        info = await this.check(account.id);
      } catch {
        // Removed meanwhile, or the account database failed: the page
        // reports those itself when the user acts; the account stays
        // "Not checked yet".
        continue;
      } finally {
        this.current = null;
      }
      this.results = { ...this.results, [info.id]: info };
      onresult(info);
      if (info.lastCheck !== null && STOP_AFTER.includes(info.lastCheck.status)) {
        return;
      }
    }
  }
}
