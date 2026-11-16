// Automatic sync of the optional sotyware account's vault (ADR-032; the
// automatic part: docs/architecture/accounts.md#when-sync-runs). A device
// that is signed in and enrolled syncs by itself:
// - when sotyware starts (so a computer picks up what the others changed),
// - when the window is focused again, at most every FOCUS_INTERVAL_MS,
// - after a storage account was added, changed or removed on this computer.
// Event-driven: no timer runs in the background. Nothing here blocks local
// work: offline, the sync fails, Rust records why (`lastSyncError`), the
// shell shows it, and every storage account keeps working as configured.
// Builds without an account service (`configured: false`) never sync.
import { accountStatus, syncNow, type AccountStatus } from "./account";

/** The shortest gap between two syncs started by focusing the window. */
export const FOCUS_INTERVAL_MS = 15 * 60_000;

export type SyncReason = "startup" | "focus" | "change";

/** Whether this device may sync on its own: an account service exists, the
 * user is signed in, and this computer holds the account key. */
export function canAutoSync(s: AccountStatus | null): boolean {
  return s !== null && s.configured && s.signedIn && s.vaultAccess === "enrolled";
}

/** What the shell says about sync, or null when there is nothing to say
 * (no account service, or not signed in). */
export function syncSummary(s: AccountStatus | null, now: number = Date.now()): { tone: "success" | "warning"; text: string } | null {
  if (s === null || !s.configured || !s.signedIn) {
    return null;
  }
  if (s.vaultAccess === "pending") {
    return { tone: "warning", text: "Sync is waiting: unlock this computer on the Account page." };
  }
  if (s.vaultAccess !== "enrolled") {
    return { tone: "warning", text: "Sync is not set up on this computer." };
  }
  if (s.conflicts.length > 0) {
    return { tone: "warning", text: `${s.conflicts.length} sync conflict${s.conflicts.length === 1 ? "" : "s"} to review on the Account page.` };
  }
  if (s.lastSyncError !== null) {
    return { tone: "warning", text: `Not synced: ${s.lastSyncError}. Everything keeps working on this computer.` };
  }
  if (s.lastSyncMs === null) {
    return { tone: "warning", text: "Not synced yet." };
  }
  const minutes = Math.max(0, Math.round((now - s.lastSyncMs) / 60_000));
  return { tone: "success", text: minutes < 1 ? "Synced just now." : `Synced ${minutes} minute${minutes === 1 ? "" : "s"} ago.` };
}

export class AutoSync {
  /** The account status as last read. */
  status = $state<AccountStatus | null>(null);
  private inFlight: Promise<boolean> | null = null;
  private lastFocusSync = 0;

  constructor(private readonly now: () => number = Date.now) {}

  /** Syncs if this device may; true when a sync ran (storage accounts may
   * have changed). Concurrent calls share one run. */
  run(reason: SyncReason): Promise<boolean> {
    if (this.inFlight !== null) {
      return this.inFlight;
    }
    if (reason === "focus" && this.now() - this.lastFocusSync < FOCUS_INTERVAL_MS) {
      return Promise.resolve(false);
    }
    this.inFlight = this.attempt(reason).finally(() => {
      this.inFlight = null;
    });
    return this.inFlight;
  }

  private async attempt(reason: SyncReason): Promise<boolean> {
    try {
      this.status = await accountStatus();
    } catch {
      // No status (the account state could not be read): no sync.
      return false;
    }
    if (!canAutoSync(this.status)) {
      return false;
    }
    if (reason !== "change") {
      this.lastFocusSync = this.now();
    }
    try {
      await syncNow();
    } catch {
      // Rust recorded why; the status read below carries it.
    }
    try {
      this.status = await accountStatus();
    } catch {
      // Keep the status read before.
    }
    return true;
  }
}
