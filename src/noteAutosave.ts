// The notes overlay's autosave (ADR-052): what the user types is saved
// half a second after they stop, and at least every two seconds while
// they keep typing, one save at a time per note, each naming the revision
// it edited. A flush saves at once (switching notes, hiding, the window
// losing the focus). Typing during a save is saved after it. A save that
// failed stays pending and is tried again: at once after a conflict (with
// the revision Rust holds), after a pause otherwise.

/** The latest text of a note, as the page holds it. */
export interface Draft {
  title: string;
  body: string;
  revision: number;
}

export type SaveState = "saved" | "pending" | "saving" | "failed";

/** What to do after a failed save: try again now (after a conflict was
 * resolved), later, at the next change (the text itself was refused), or
 * never (the note is gone). */
export type Recovery = "retry" | "later" | "wait" | "drop";

export interface AutosaveOptions {
  /** The note's latest text, or `null` when it no longer exists. */
  read: (id: string) => Draft | null;
  send: (id: string, title: string, body: string, revision: number) => Promise<{ revision: number }>;
  /** The save of `title` and `body` succeeded as `revision`. */
  saved: (id: string, revision: number, title: string, body: string) => void;
  failed: (id: string, error: unknown) => Recovery | Promise<Recovery>;
  /** The state of a note's text changed. */
  onstate?: (id: string, state: SaveState) => void;
  delayMs?: number;
  maxWaitMs?: number;
  retryMs?: number;
  now?: () => number;
}

interface NoteState {
  timer: ReturnType<typeof setTimeout> | null;
  /** When the oldest unsaved change was made (0: none). */
  since: number;
  dirty: boolean;
  inFlight: Promise<void> | null;
  failed: boolean;
}

export class Autosave {
  private readonly states = new Map<string, NoteState>();
  private readonly delayMs: number;
  private readonly maxWaitMs: number;
  private readonly retryMs: number;
  private readonly now: () => number;

  constructor(private readonly opts: AutosaveOptions) {
    this.delayMs = opts.delayMs ?? 500;
    this.maxWaitMs = opts.maxWaitMs ?? 2000;
    this.retryMs = opts.retryMs ?? 5000;
    this.now = opts.now ?? ((): number => Date.now());
  }

  private state(id: string): NoteState {
    let s = this.states.get(id);
    if (s === undefined) {
      s = { timer: null, since: 0, dirty: false, inFlight: null, failed: false };
      this.states.set(id, s);
    }
    return s;
  }

  private schedule(id: string, s: NoteState, wait: number): void {
    if (s.timer !== null) {
      clearTimeout(s.timer);
    }
    s.timer = setTimeout(() => {
      s.timer = null;
      void this.flush(id);
    }, wait);
  }

  /** The note's text changed: a save is due soon. */
  changed(id: string): void {
    const s = this.state(id);
    s.dirty = true;
    if (s.since === 0) {
      s.since = this.now();
    }
    const waited = this.now() - s.since;
    this.schedule(id, s, waited >= this.maxWaitMs ? 0 : Math.min(this.delayMs, this.maxWaitMs - waited));
    if (s.inFlight === null) {
      this.opts.onstate?.(id, "pending");
    }
  }

  /** Saves the note's pending text now (after a save in flight). */
  async flush(id: string): Promise<void> {
    const s = this.states.get(id);
    if (s === undefined) {
      return;
    }
    if (s.timer !== null) {
      clearTimeout(s.timer);
      s.timer = null;
    }
    while (s.inFlight !== null) {
      await s.inFlight;
    }
    if (!s.dirty) {
      return;
    }
    const draft = this.opts.read(id);
    if (draft === null) {
      this.forget(id);
      return;
    }
    s.dirty = false;
    s.since = 0;
    this.opts.onstate?.(id, "saving");
    let recovery: Recovery | null = null;
    const run = (async (): Promise<void> => {
      try {
        const r = await this.opts.send(id, draft.title, draft.body, draft.revision);
        s.failed = false;
        this.opts.saved(id, r.revision, draft.title, draft.body);
      } catch (e) {
        // Still unsaved, unless typing since made it dirty anyway.
        s.dirty = true;
        s.failed = true;
        recovery = await this.opts.failed(id, e);
      }
    })();
    s.inFlight = run;
    try {
      await run;
    } finally {
      s.inFlight = null;
    }
    const r = recovery as Recovery | null;
    if (r === "drop") {
      this.forget(id);
      return;
    }
    if (s.dirty) {
      if (r === "retry") {
        this.schedule(id, s, 0);
      } else if (r === "later") {
        this.schedule(id, s, this.retryMs);
      } else if (r === "wait") {
        // The next change schedules the next save.
      } else if (s.timer === null) {
        // Typed during the save, and its timer already fired.
        this.schedule(id, s, 0);
      }
      this.opts.onstate?.(id, s.failed ? "failed" : "pending");
    } else {
      this.opts.onstate?.(id, "saved");
    }
  }

  /** Saves every pending note now. */
  async flushAll(): Promise<void> {
    await Promise.all([...this.states.keys()].map((id) => this.flush(id)));
  }

  /** Whether the note has text not saved yet (or being saved). */
  pending(id: string): boolean {
    const s = this.states.get(id);
    return s !== undefined && (s.dirty || s.inFlight !== null);
  }

  /** The note was deleted: nothing more to save. */
  forget(id: string): void {
    const s = this.states.get(id);
    if (s?.timer) {
      clearTimeout(s.timer);
    }
    this.states.delete(id);
  }
}
