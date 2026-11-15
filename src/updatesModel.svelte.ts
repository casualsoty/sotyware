// The update state the dashboard shows in two places: the notification
// banner of the shell (App.svelte) and Settings › About & updates
// (Updates.svelte). One instance per window, created by App and passed down,
// so a download keeps its progress while the user moves between pages.
// Only state and the user's actions live here; every check, download,
// verification and installation happens in Rust (src-tauri/src/updates.rs).
import {
  cancelUpdate,
  checkForUpdate,
  getUpdateStatus,
  installUpdate,
  postponeUpdate,
  restartAfterUpdate,
  type InstallEvent,
  type UpdateErrorKind,
  type UpdateStatus,
} from "./updates";

export type InstallState =
  | { stage: "downloading"; downloaded: number; total: number | null }
  | { stage: "verified" }
  | { stage: "installing" }
  | { stage: "installed" }
  | { stage: "failed"; kind: UpdateErrorKind; message: string }
  | { stage: "cancelled" };

/** What the user is told about a failure, by its kind. */
export const FAILURE_TITLES: Record<UpdateErrorKind, string> = {
  network: "The update server could not be reached",
  signature: "The update is not signed by sotyware's release key",
  integrity: "The downloaded update failed verification",
  replay: "The update server offered an older release",
  platform: "No update package for this installation",
  malformed: "The update information is not valid",
  installer: "The update could not be installed",
  busy: "Something is still running",
};

function kindOf(e: unknown): UpdateErrorKind | null {
  if (typeof e === "object" && e !== null && "kind" in e) {
    const k = (e as { kind: unknown }).kind;
    if (k === "network" || k === "integrity" || k === "busy") {
      return k;
    }
  }
  return null;
}

export class UpdatesModel {
  status = $state<UpdateStatus | null>(null);
  /** A command failed (not the check itself, which Rust reports in status). */
  error = $state<string | null>(null);
  errorKind = $state<UpdateErrorKind | null>(null);
  checking = $state(false);
  restarting = $state(false);
  install = $state<InstallState | null>(null);
  /** The banner was closed for this session (not "Later": that is saved). */
  bannerClosed = $state(false);

  running = $derived(
    this.install !== null &&
      (this.install.stage === "downloading" || this.install.stage === "verified" || this.install.stage === "installing"),
  );

  /** Whether the shell's banner should show: a version is available and was
   * neither postponed nor closed, or one was installed and waits for a
   * restart, or a download runs (the banner then shows its progress). */
  bannerVisible = $derived(
    this.status !== null &&
      !this.bannerClosed &&
      ((this.status.available !== null && !this.status.postponed) || this.status.installed !== null || this.running),
  );

  async load(): Promise<void> {
    try {
      this.status = await getUpdateStatus();
    } catch (e) {
      this.error = `Could not read the update status: ${String(e)}`;
    }
  }

  async check(): Promise<void> {
    this.error = null;
    this.errorKind = null;
    this.checking = true;
    try {
      this.status = await checkForUpdate();
      this.install = null;
      this.bannerClosed = false;
    } catch (e) {
      this.error = `Could not check for updates: ${messageOf(e)}`;
      this.errorKind = kindOf(e);
      await this.load();
    } finally {
      this.checking = false;
    }
  }

  private onEvent(e: InstallEvent): void {
    switch (e.type) {
      case "progress":
        this.install = { stage: "downloading", downloaded: e.downloaded, total: e.total };
        break;
      case "failed":
        this.install = { stage: "failed", kind: e.kind, message: e.message };
        void this.load();
        break;
      case "installed":
        this.install = { stage: "installed" };
        void this.load();
        break;
      default:
        this.install = { stage: e.type };
    }
  }

  async updateNow(): Promise<void> {
    this.error = null;
    this.errorKind = null;
    this.install = { stage: "downloading", downloaded: 0, total: null };
    try {
      await installUpdate((e) => this.onEvent(e));
    } catch (e) {
      this.install = null;
      this.error = `Could not start the update: ${messageOf(e)}`;
      this.errorKind = kindOf(e);
      await this.load();
    }
  }

  async cancel(): Promise<void> {
    try {
      await cancelUpdate();
    } catch (e) {
      this.error = `Could not cancel the update: ${messageOf(e)}`;
    }
  }

  async later(): Promise<void> {
    this.error = null;
    try {
      await postponeUpdate();
      await this.load();
    } catch (e) {
      this.error = `Could not postpone the update: ${messageOf(e)}`;
    }
  }

  async restart(): Promise<void> {
    this.error = null;
    this.restarting = true;
    try {
      await restartAfterUpdate();
    } catch (e) {
      this.restarting = false;
      this.error = `Could not restart sotyware: ${messageOf(e)}`;
      this.errorKind = kindOf(e);
    }
  }

  closeBanner(): void {
    this.bannerClosed = true;
  }
}

function messageOf(e: unknown): string {
  if (typeof e === "object" && e !== null && "message" in e && typeof (e as { message: unknown }).message === "string") {
    return (e as { message: string }).message;
  }
  return String(e);
}
