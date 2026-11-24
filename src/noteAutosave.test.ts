// The notes overlay's autosave (ADR-052) on fake timers: when saves go
// out, that they never overlap for one note, and what happens after a
// failure.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Autosave, type Draft, type Recovery } from "./noteAutosave";

interface Sent {
  id: string;
  title: string;
  body: string;
  revision: number;
}

let notes: Map<string, Draft>;
let sent: Sent[];
let pendingSends: { resolve: (r: { revision: number }) => void; reject: (e: unknown) => void }[];
let manual: boolean;
let failNext: unknown;
let recovery: Recovery;
let onFailed: ((id: string) => void) | null;
let states: string[];

function autosave(): Autosave {
  return new Autosave({
    read: (id) => notes.get(id) ?? null,
    send: (id, title, body, revision) => {
      sent.push({ id, title, body, revision });
      if (failNext !== undefined) {
        const e = failNext;
        failNext = undefined;
        return Promise.reject(e);
      }
      if (manual) {
        return new Promise((resolve, reject) => pendingSends.push({ resolve, reject }));
      }
      return Promise.resolve({ revision: revision + 1 });
    },
    saved: (id, revision) => {
      const n = notes.get(id);
      if (n) {
        n.revision = revision;
      }
    },
    failed: (id) => {
      onFailed?.(id);
      return recovery;
    },
    onstate: (_id, s) => states.push(s),
    now: () => Date.now(),
  });
}

function type(id: string, body: string): void {
  const n = notes.get(id);
  if (n) {
    n.body = body;
  }
}

beforeEach(() => {
  vi.useFakeTimers();
  notes = new Map([
    ["a", { title: "A", body: "", revision: 1 }],
    ["b", { title: "B", body: "", revision: 1 }],
  ]);
  sent = [];
  pendingSends = [];
  manual = false;
  failNext = undefined;
  recovery = "later";
  onFailed = null;
  states = [];
});

afterEach(() => {
  vi.useRealTimers();
});

describe("autosave", () => {
  it("saves half a second after typing stops", async () => {
    const s = autosave();
    type("a", "h");
    s.changed("a");
    await vi.advanceTimersByTimeAsync(300);
    type("a", "he");
    s.changed("a");
    await vi.advanceTimersByTimeAsync(400);
    expect(sent).toEqual([]);
    await vi.advanceTimersByTimeAsync(200);
    expect(sent).toEqual([{ id: "a", title: "A", body: "he", revision: 1 }]);
    expect(notes.get("a")?.revision).toBe(2);
    expect(s.pending("a")).toBe(false);
    expect(states).toEqual(["pending", "pending", "saving", "saved"]);
  });

  it("saves at least every two seconds while typing goes on", async () => {
    const s = autosave();
    for (let i = 1; i <= 12; i += 1) {
      type("a", "x".repeat(i));
      s.changed("a");
      await vi.advanceTimersByTimeAsync(250);
    }
    // 3 s of typing every 250 ms: saved once at the 2 s mark, then the
    // rest after typing stops.
    expect(sent.map((x) => x.body.length)).toEqual([8]);
    await vi.advanceTimersByTimeAsync(600);
    expect(sent.map((x) => [x.body.length, x.revision])).toEqual([
      [8, 1],
      [12, 2],
    ]);
  });

  it("flushes at once and never sends two saves of one note together", async () => {
    manual = true;
    const s = autosave();
    type("a", "one");
    s.changed("a");
    const first = s.flush("a");
    expect(sent).toHaveLength(1);
    // Typing and flushing again while the first save is in flight waits
    // for it, then saves with the new revision.
    type("a", "one two");
    s.changed("a");
    const second = s.flush("a");
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toHaveLength(1);
    pendingSends.shift()?.resolve({ revision: 2 });
    await first;
    await vi.advanceTimersByTimeAsync(0);
    expect(sent).toHaveLength(2);
    expect(sent[1]).toEqual({ id: "a", title: "A", body: "one two", revision: 2 });
    pendingSends.shift()?.resolve({ revision: 3 });
    await second;
    expect(notes.get("a")?.revision).toBe(3);
    expect(s.pending("a")).toBe(false);
  });

  it("keeps notes apart and flushes them all", async () => {
    const s = autosave();
    type("a", "alpha");
    s.changed("a");
    type("b", "beta");
    s.changed("b");
    await s.flushAll();
    expect(sent.map((x) => [x.id, x.body])).toEqual([
      ["a", "alpha"],
      ["b", "beta"],
    ]);
    await vi.advanceTimersByTimeAsync(5000);
    // Nothing more to save.
    expect(sent).toHaveLength(2);
  });

  it("tries a failed save again later and keeps the text pending", async () => {
    const s = autosave();
    failNext = new Error("database is locked");
    type("a", "keep me");
    s.changed("a");
    await vi.advanceTimersByTimeAsync(600);
    expect(sent).toHaveLength(1);
    expect(s.pending("a")).toBe(true);
    expect(states.at(-1)).toBe("failed");
    await vi.advanceTimersByTimeAsync(4000);
    expect(sent).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1100);
    expect(sent).toHaveLength(2);
    expect(sent[1]?.body).toBe("keep me");
    expect(s.pending("a")).toBe(false);
    expect(states.at(-1)).toBe("saved");
  });

  it("retries at once after a conflict was resolved, and drops a deleted note", async () => {
    const s = autosave();
    failNext = new Error("conflict");
    recovery = "retry";
    type("a", "mine");
    s.changed("a");
    // On the conflict the page reads the revision Rust holds.
    onFailed = (id) => {
      const n = notes.get(id);
      if (n) {
        n.revision = 7;
      }
    };
    await vi.advanceTimersByTimeAsync(600);
    expect(sent.map((x) => x.revision)).toEqual([1, 7]);

    failNext = new Error("not found");
    recovery = "drop";
    type("b", "gone");
    s.changed("b");
    await vi.advanceTimersByTimeAsync(600);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(sent.filter((x) => x.id === "b")).toHaveLength(1);
    expect(s.pending("b")).toBe(false);
  });

  it("waits for the next change after the text itself was refused", async () => {
    const s = autosave();
    failNext = new Error("A note can have at most 50000 characters");
    recovery = "wait";
    type("a", "too long");
    s.changed("a");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(sent).toHaveLength(1);
    expect(s.pending("a")).toBe(true);
    type("a", "shorter");
    s.changed("a");
    await vi.advanceTimersByTimeAsync(600);
    expect(sent.map((x) => x.body)).toEqual(["too long", "shorter"]);
    expect(s.pending("a")).toBe(false);
  });

  it("forgets a deleted note and does not save what no longer exists", async () => {
    const s = autosave();
    type("a", "x");
    s.changed("a");
    s.forget("a");
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toEqual([]);
    type("b", "y");
    s.changed("b");
    notes.delete("b");
    await s.flush("b");
    expect(sent).toEqual([]);
  });
});
