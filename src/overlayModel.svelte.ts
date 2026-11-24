// The gaming notes overlay as the dashboard sees it (ADR-052): one
// subscription to `watch_overlay`, whose channel brings the status
// whenever it changes, and the configuration, read when the settings are
// shown. The overlay itself runs in Rust whether or not anything here
// listens. When the overlay asks for its settings, `onsettings` runs (the
// shell opens Settings › Notes overlay).
import {
  overlayConfig,
  resetOverlay,
  saveOverlaySettings,
  setOverlayMode,
  setOverlayVisible,
  unwatchOverlay,
  watchOverlay,
  type Mode,
  type OverlayConfig,
  type OverlaySettings,
  type OverlayStatus,
} from "./notesOverlay";

export class OverlayModel {
  status = $state<OverlayStatus | null>(null);
  config = $state<OverlayConfig | null>(null);
  error = $state<string | null>(null);
  /** The overlay asked for its settings. */
  onsettings: (() => void) | null = null;
  private subscription: number | null = null;
  private stopped = false;
  private settingsSeq: number | null = null;

  async start(): Promise<void> {
    this.stopped = false;
    try {
      const id = await watchOverlay((s) => this.put(s));
      if (this.stopped) {
        void unwatchOverlay(id).catch(() => undefined);
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
      void unwatchOverlay(this.subscription).catch(() => undefined);
      this.subscription = null;
    }
  }

  /** A newer status (from the channel or a command's answer). */
  put(s: OverlayStatus): void {
    if (this.stopped || (this.status !== null && s.seq < this.status.seq)) {
      return;
    }
    const before = this.settingsSeq;
    this.status = s;
    this.settingsSeq = s.settingsSeq;
    if (before !== null && s.settingsSeq !== before) {
      this.onsettings?.();
    }
  }

  async loadConfig(): Promise<void> {
    try {
      this.config = await overlayConfig();
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  /** Saves the settings; throws Rust's refusal. */
  async save(settings: OverlaySettings): Promise<void> {
    const saved = await saveOverlaySettings(settings);
    if (this.config !== null) {
      this.config.settings = saved;
    }
  }

  async reset(): Promise<void> {
    const saved = await resetOverlay();
    if (this.config !== null) {
      this.config.settings = saved;
    }
  }

  async setVisible(visible: boolean): Promise<void> {
    this.put(await setOverlayVisible(visible));
  }

  async setMode(mode: Mode): Promise<void> {
    this.put(await setOverlayMode(mode));
  }
}
