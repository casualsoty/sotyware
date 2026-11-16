// The live state of every folder sync (ADR-043), for the shell's sidebar
// and the bucket view's Sync tab: one subscription to `watch_folder_syncs`,
// whose channel brings the overview whenever a sync changes (Rust sends at
// most four a second). The engines run in Rust whether or not anything
// here listens.
import { listFolderSyncs, unwatchFolderSyncs, watchFolderSyncs, type SyncOverview, type SyncStatus } from "./folderSync";

export class FolderSyncModel {
  overview = $state<SyncOverview | null>(null);
  error = $state<string | null>(null);
  private subscription: number | null = null;
  private stopped = false;

  /** Subscribes; the overview is read once first so the page fills at
   * once even if the subscription fails. */
  async start(): Promise<void> {
    this.stopped = false;
    try {
      this.overview = await listFolderSyncs();
      const id = await watchFolderSyncs((o) => {
        if (!this.stopped && (this.overview === null || o.seq >= this.overview.seq)) {
          this.overview = o;
        }
      });
      if (this.stopped) {
        void unwatchFolderSyncs(id).catch(() => undefined);
      } else {
        this.subscription = id;
      }
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  stop(): void {
    this.stopped = true;
    if (this.subscription !== null) {
      void unwatchFolderSyncs(this.subscription).catch(() => undefined);
      this.subscription = null;
    }
  }

  /** Reads the overview again (after a command changed something). */
  async refresh(): Promise<void> {
    try {
      const o = await listFolderSyncs();
      if (this.overview === null || o.seq >= this.overview.seq) {
        this.overview = o;
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  /** The sync of `account`, if it has one. */
  of(account: string): SyncStatus | null {
    return this.overview?.syncs.find((s) => s.account === account) ?? null;
  }
}
