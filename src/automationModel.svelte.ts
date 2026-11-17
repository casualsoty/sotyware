// The autoclicker's live state for the shell (the sidebar's indicator)
// and the Autoclicker page (ADR-050): one subscription to
// `watch_automation`, whose channel brings the status whenever it changes
// (at most ten times a second, once a second while something runs), plus
// the configuration and the macro library, read when needed. Automation
// itself runs in Rust whether or not anything here listens.
import {
  automationConfig,
  automationStatus,
  listMacros,
  unwatchAutomation,
  watchAutomation,
  type AutomationConfig,
  type AutomationStatus,
  type MacroSummary,
  type Profile,
  type Settings,
} from "./automation";

/** What the sidebar shows next to "Autoclicker". */
export type Indicator = "recording" | "running" | "paused" | null;

export function indicatorOf(status: AutomationStatus | null): Indicator {
  const s = status?.session;
  if (!s) {
    return null;
  }
  const state = s.kind === "recording" ? s.state : s.progress.state;
  if (state === "paused") {
    return "paused";
  }
  return s.kind === "recording" ? "recording" : "running";
}

export class AutomationModel {
  status = $state<AutomationStatus | null>(null);
  config = $state<AutomationConfig | null>(null);
  macros = $state<MacroSummary[] | null>(null);
  error = $state<string | null>(null);
  private subscription: number | null = null;
  private stopped = false;

  /** Subscribes; the status is read once first so the page fills at once
   * even if the subscription fails. */
  async start(): Promise<void> {
    this.stopped = false;
    try {
      this.status = await automationStatus();
      const id = await watchAutomation((s) => {
        if (!this.stopped && (this.status === null || s.seq >= this.status.seq)) {
          const profileChanged = this.status?.activeProfile.id !== s.activeProfile.id;
          this.status = s;
          if (profileChanged && this.config !== null) {
            void this.loadConfig();
          }
        }
      });
      if (this.stopped) {
        void unwatchAutomation(id).catch(() => undefined);
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
      void unwatchAutomation(this.subscription).catch(() => undefined);
      this.subscription = null;
    }
  }

  async refreshStatus(): Promise<void> {
    try {
      const s = await automationStatus();
      if (this.status === null || s.seq >= this.status.seq) {
        this.status = s;
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async loadConfig(): Promise<void> {
    try {
      this.config = await automationConfig();
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async loadMacros(): Promise<void> {
    try {
      this.macros = await listMacros();
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  get activeProfile(): Profile | null {
    const c = this.config;
    return c?.profiles.find((p) => p.id === c.activeProfile) ?? null;
  }

  get indicator(): Indicator {
    return indicatorOf(this.status);
  }

  /** A profile Rust saved: replaces the page's copy. */
  putProfile(p: Profile): void {
    if (this.config) {
      const i = this.config.profiles.findIndex((x) => x.id === p.id);
      this.config.profiles = i < 0 ? [...this.config.profiles, p] : this.config.profiles.map((x, j) => (j === i ? p : x));
    }
  }

  putSettings(s: Settings): void {
    if (this.config) {
      this.config.settings = s;
    }
  }

  putMacro(m: MacroSummary): void {
    const list = this.macros ?? [];
    const i = list.findIndex((x) => x.id === m.id);
    this.macros = i < 0 ? [...list, m] : list.map((x, j) => (j === i ? m : x));
  }

  dropMacro(id: string): void {
    this.macros = (this.macros ?? []).filter((m) => m.id !== id);
  }
}
