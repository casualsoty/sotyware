// The gaming notes overlay's own page (ADR-052), headless with Tauri's
// IPC mocks: what it shows of the notes and of the status Rust sends,
// what it asks Rust when the user edits, ticks, switches, creates,
// deletes and moves notes, and that passive mode leaves nothing to click
// or type into. The window itself (click-through, always on top) is
// Rust's, tested in src-tauri/src/overlay.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import NotesOverlay from "./NotesOverlay.svelte";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let notes: Record<string, unknown>[];
let activeNote: string | null;
let channel: FakeChannel | null;
let current: Record<string, unknown>;
let fail: Record<string, unknown> | null;
let app: ReturnType<typeof mount> | undefined;
let root: HTMLElement;

function note(id: string, title: string, body: string, over: Record<string, unknown> = {}): Record<string, unknown> {
  return { id, title, body, pinned: false, position: 0, createdMs: 1, updatedMs: 1, revision: 1, ...over };
}

function hotkey(action: string, keys: string, registered = true): Record<string, unknown> {
  return { action, label: action, chord: keys.replaceAll(" ", ""), keys, registered, error: registered ? null : "Not registered" };
}

function status(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    seq: 1,
    running: true,
    visible: true,
    mode: "interactive",
    collapsed: false,
    opacity: 88,
    textSize: "medium",
    activeNote: "boss",
    activeSeq: 0,
    notesSeq: 0,
    focusSeq: 0,
    focusTarget: "content",
    settingsSeq: 0,
    noteCount: notes.length,
    hotkeys: [hotkey("toggle-visible", "Ctrl + Shift + F1"), hotkey("toggle-mode", "Ctrl + Shift + F2")],
    shortcutsNote: null,
    notice: null,
    storageError: null,
    ...over,
  };
}

/** The next status from Rust, with a higher seq. */
function send(over: Record<string, unknown>): void {
  current = { ...current, ...over, seq: (current["seq"] as number) + 1 };
  channel?.onmessage(current);
  flushSync();
}

function orderOf(ids: string[], pinned: string[] = []): Record<string, unknown>[] {
  return ids.map((id, position) => ({ id, pinned: pinned.includes(id), position }));
}

beforeEach(() => {
  calls = [];
  notes = [
    note("route", "Route", "1. Bridge\n2. Cave", { pinned: true }),
    note("boss", "Boss strategy", "## Phase 1\n- [ ] Prepare healing items\n- [x] Equip gear\n\nSee [wiki](https://wiki.example/boss)"),
    note("loot", "Loot", "Legendary sword"),
  ];
  activeNote = "boss";
  channel = null;
  fail = null;
  current = status();
  mockWindows("overlay");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    if (fail !== null && fail["cmd"] === cmd) {
      const e = fail["error"];
      fail = null;
      throw e;
    }
    switch (cmd) {
      case "list_notes":
        return { notes, activeNote };
      case "watch_overlay":
        channel = p["onEvent"] as FakeChannel;
        channel.onmessage(current);
        return 9;
      case "save_note": {
        const n = notes.find((x) => x["id"] === p["id"]);
        if (n) {
          Object.assign(n, { title: p["title"], body: p["body"], revision: (p["revision"] as number) + 1 });
        }
        return { revision: (p["revision"] as number) + 1, updatedMs: 2 };
      }
      case "create_note": {
        const n = note("new-1", "New note", "");
        notes.push(n);
        return n;
      }
      case "delete_note":
        notes = notes.filter((x) => x["id"] !== p["id"]);
        return { activeNote: "loot" };
      case "duplicate_note":
        return { note: note("boss-2", "Boss strategy (2)", "copied"), order: orderOf(["route", "boss", "boss-2", "loot"], ["route"]) };
      case "set_note_pinned":
        return orderOf(["route", "boss", "loot"], ["route", "boss"]);
      case "move_note":
        return orderOf(["route", "loot", "boss"], ["route"]);
      case "set_overlay_mode":
        current = { ...current, mode: p["mode"], seq: (current["seq"] as number) + 1 };
        return current;
      case "set_overlay_collapsed":
        current = { ...current, collapsed: p["collapsed"], seq: (current["seq"] as number) + 1 };
        return current;
      case "set_overlay_opacity":
        current = { ...current, opacity: p["opacity"], seq: (current["seq"] as number) + 1 };
        return current;
      case "set_overlay_visible":
        current = { ...current, visible: p["visible"], seq: (current["seq"] as number) + 1 };
        return current;
      default:
        return null;
    }
  });
});

afterEach(() => {
  if (app) {
    unmount(app);
    app = undefined;
  }
  clearMocks();
  vi.useRealTimers();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  for (let i = 0; i < 4; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  root = document.createElement("div");
  document.body.append(root);
  app = mount(NotesOverlay, { target: root });
  flushSync();
  await settle();
  return root;
}

function sent(cmd: string): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.payload);
}

function button(label: string): HTMLButtonElement {
  const b = [...document.querySelectorAll("button")].find(
    (x) => x.getAttribute("aria-label") === label || x.textContent?.trim() === label,
  );
  if (!b) {
    throw new Error(`no button "${label}" in ${document.body.innerHTML}`);
  }
  return b;
}

function tabs(): string[] {
  return [...root.querySelectorAll<HTMLElement>("[role='tab']")].map((t) => t.textContent?.trim() ?? "");
}

function selectedTab(): string {
  return root.querySelector<HTMLElement>("[role='tab'][aria-selected='true']")?.textContent?.trim() ?? "";
}

function key(target: EventTarget, k: string, over: KeyboardEventInit = {}): void {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...over }));
  flushSync();
}

function type(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

async function edit(): Promise<HTMLTextAreaElement> {
  button("Edit the note").click();
  await settle();
  const t = root.querySelector<HTMLTextAreaElement>("textarea.editor");
  if (!t) {
    throw new Error("no editor");
  }
  return t;
}

describe("the notes", () => {
  it("shows the tabs, pinned first, and the note shown last with its checklist progress", async () => {
    await render();
    expect(tabs()).toEqual(["Pinned: Route", "Boss strategy", "Loot"]);
    expect(selectedTab()).toBe("Boss strategy");
    expect(root.querySelector("h1")?.textContent).toBe("Boss strategy");
    expect(root.querySelector(".progress")?.textContent).toBe("1/2");
    const view = root.querySelector(".note-view");
    expect(view?.querySelector("h3")?.textContent).toBe("Phase 1");
    const boxes = [...(view?.querySelectorAll<HTMLInputElement>("input[type='checkbox']") ?? [])];
    expect(boxes.map((b) => [b.getAttribute("aria-label"), b.checked, b.disabled])).toEqual([
      ["Prepare healing items", false, false],
      ["Equip gear", true, false],
    ]);
    // No raw HTML anywhere: a link is an <a> drawn by the page.
    expect(view?.querySelector("a")?.getAttribute("href")).toBe("https://wiki.example/boss");
  });

  it("ticks a checklist item in the text and saves it at once", async () => {
    await render();
    const box = root.querySelector<HTMLInputElement>(".note-view input[type='checkbox']");
    box?.click();
    flushSync();
    await settle();
    expect(sent("save_note")).toEqual([
      {
        id: "boss",
        title: "Boss strategy",
        body: "## Phase 1\n- [x] Prepare healing items\n- [x] Equip gear\n\nSee [wiki](https://wiki.example/boss)",
        revision: 1,
      },
    ]);
    expect(root.querySelector(".progress")?.textContent).toBe("2/2");
  });

  it("autosaves the title and the text half a second after typing stops", async () => {
    await render();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const title = root.querySelector<HTMLInputElement>("input.note-title");
    if (!title) {
      throw new Error("no title field");
    }
    type(title, "Boss: phase 2");
    await vi.advanceTimersByTimeAsync(300);
    expect(sent("save_note")).toEqual([]);
    await vi.advanceTimersByTimeAsync(300);
    expect(sent("save_note")).toEqual([
      expect.objectContaining({ id: "boss", title: "Boss: phase 2", revision: 1 }),
    ]);
    expect(selectedTab()).toBe("Boss: phase 2");
    vi.useRealTimers();
    const editor = await edit();
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    type(editor, "Kill the adds first");
    await vi.advanceTimersByTimeAsync(600);
    expect(sent("save_note").at(-1)).toEqual({ id: "boss", title: "Boss: phase 2", body: "Kill the adds first", revision: 2 });
    expect(root.querySelector(".save-state")?.textContent).toBe("Saved");
  });

  it("continues a checklist on Enter and ticks the line with Ctrl+Enter", async () => {
    await render();
    const editor = await edit();
    editor.setSelectionRange(editor.value.indexOf("items") + 5, editor.value.indexOf("items") + 5);
    key(editor, "Enter");
    expect(editor.value.split("\n")[2]).toBe("- [ ] ");
    key(editor, "Enter", { ctrlKey: true });
    expect(editor.value.split("\n")[2]).toBe("- [x] ");
    // Escape leaves the editor; the note is rendered again.
    key(editor, "Escape");
    await settle();
    expect(root.querySelector("textarea")).toBeNull();
    expect(root.querySelector(".note-view")).not.toBeNull();
  });

  it("formats the selection from the formatting bar", async () => {
    await render();
    const editor = await edit();
    const at = editor.value.indexOf("Equip gear");
    editor.setSelectionRange(at, at + "Equip".length);
    button("Bold (Ctrl+B)").click();
    flushSync();
    expect(editor.value).toContain("- [x] **Equip** gear");
  });

  it("starts editing at the line clicked", async () => {
    await render();
    root.querySelector<HTMLElement>(".note-view [data-line='2'] .task-text")?.click();
    flushSync();
    await settle();
    const editor = root.querySelector<HTMLTextAreaElement>("textarea.editor");
    expect(editor).not.toBeNull();
    const lineEnd = "## Phase 1\n- [ ] Prepare healing items\n- [x] Equip gear".length;
    expect(editor?.selectionStart).toBe(lineEnd);
  });

  it("creates a note and puts the cursor in its title", async () => {
    await render();
    button("New note (Ctrl+N)").click();
    await settle();
    expect(sent("create_note")).toEqual([{ title: null }]);
    expect(tabs().at(-1)).toBe("New note");
    expect(selectedTab()).toBe("New note");
    expect(document.activeElement).toBe(root.querySelector("input.note-title"));
  });

  it("deletes a note only after asking", async () => {
    await render();
    button("More actions for this note").click();
    flushSync();
    button("Delete…").click();
    flushSync();
    const dialog = document.querySelector("[role='dialog']");
    expect(dialog?.textContent).toContain("“Boss strategy” and its text are deleted");
    expect(sent("delete_note")).toEqual([]);
    button("Delete").click();
    await settle();
    expect(sent("delete_note")).toEqual([{ id: "boss" }]);
    expect(tabs()).toEqual(["Pinned: Route", "Loot"]);
    expect(selectedTab()).toBe("Loot");
  });

  it("saves what was typed before duplicating, and shows the copy", async () => {
    await render();
    const title = root.querySelector<HTMLInputElement>("input.note-title");
    if (title) {
      type(title, "Boss v2");
    }
    button("More actions for this note").click();
    flushSync();
    button("Duplicate").click();
    await settle();
    const order = calls.map((c) => c.cmd).filter((c) => c === "save_note" || c === "duplicate_note");
    expect(order).toEqual(["save_note", "duplicate_note"]);
    expect(tabs()).toEqual(["Pinned: Route", "Boss v2", "Boss strategy (2)", "Loot"]);
    expect(selectedTab()).toBe("Boss strategy (2)");
  });

  it("pins and moves notes in the order Rust returns", async () => {
    await render();
    button("More actions for this note").click();
    flushSync();
    button("Pin to the front").click();
    await settle();
    expect(sent("set_note_pinned")).toEqual([{ id: "boss", pinned: true }]);
    expect(tabs()).toEqual(["Pinned: Route", "Pinned: Boss strategy", "Loot"]);
    button("More actions for this note").click();
    flushSync();
    button("Move right").click();
    await settle();
    expect(sent("move_note")).toEqual([{ id: "boss", index: 2 }]);
    expect(tabs()).toEqual(["Pinned: Route", "Loot", "Boss strategy"]);
  });

  it("switches notes from the tabs and the keyboard, and tells Rust", async () => {
    await render();
    button("Loot").click();
    flushSync();
    expect(selectedTab()).toBe("Loot");
    key(window, "Tab", { ctrlKey: true });
    expect(selectedTab()).toBe("Pinned: Route");
    key(window, "PageUp", { ctrlKey: true });
    expect(selectedTab()).toBe("Loot");
    key(window, "2", { ctrlKey: true });
    expect(selectedTab()).toBe("Boss strategy");
    const tab = root.querySelector<HTMLElement>("[role='tab'][aria-selected='true']");
    if (tab) {
      key(tab, "ArrowRight");
    }
    expect(selectedTab()).toBe("Loot");
    await settle();
    expect(sent("select_note").map((p) => p["id"])).toEqual(["loot", "route", "loot", "boss", "loot"]);
  });

  it("searches titles and text and opens a match", async () => {
    await render();
    key(window, "f", { ctrlKey: true });
    await settle();
    const field = root.querySelector<HTMLInputElement>("input[type='search']");
    expect(document.activeElement).toBe(field);
    if (field) {
      type(field, "sword");
    }
    const hits = [...root.querySelectorAll(".hits button")].map((b) => b.querySelector(".hit-title")?.textContent);
    expect(hits).toEqual(["Loot"]);
    if (field) {
      key(field, "Enter");
    }
    await settle();
    expect(selectedTab()).toBe("Loot");
    expect(root.querySelector("input[type='search']")).toBeNull();
  });

  it("opens a link of the note through Rust", async () => {
    await render();
    root.querySelector<HTMLAnchorElement>(".note-view a")?.click();
    await settle();
    expect(sent("open_note_link")).toEqual([{ id: "boss", url: "https://wiki.example/boss" }]);
  });
});

describe("saving problems", () => {
  it("after a conflict saves the user's text again with Rust's revision", async () => {
    await render();
    fail = { cmd: "save_note", error: { kind: "conflict", message: "this note changed meanwhile (revision 4)" } };
    notes[1] = { ...notes[1], revision: 4 };
    const title = root.querySelector<HTMLInputElement>("input.note-title");
    if (title) {
      type(title, "Mine");
    }
    window.dispatchEvent(new Event("blur"));
    await settle();
    await settle();
    expect(sent("save_note").map((p) => [p["title"], p["revision"]])).toEqual([
      ["Mine", 1],
      ["Mine", 4],
    ]);
  });

  it("says when a note could not be saved", async () => {
    await render();
    fail = { cmd: "save_note", error: { kind: "storage", message: "the notes database cannot be used: disk I/O error" } };
    const title = root.querySelector<HTMLInputElement>("input.note-title");
    if (title) {
      type(title, "Mine");
    }
    window.dispatchEvent(new Event("blur"));
    await settle();
    expect(root.querySelector("[role='alert']")?.textContent).toContain("Not saved: the notes database cannot be used: disk I/O error");
  });
});

describe("the status from Rust", () => {
  it("shows the note a shortcut chose", async () => {
    await render();
    send({ activeNote: "loot", activeSeq: 1 });
    await settle();
    expect(selectedTab()).toBe("Loot");
    expect(sent("select_note")).toEqual([]);
  });

  it("reads the notes again after a note was created by shortcut, then focuses its title", async () => {
    await render();
    notes.push(note("new-2", "New note", ""));
    send({ activeNote: "new-2", activeSeq: 1, notesSeq: 1, focusSeq: 1, focusTarget: "title" });
    await settle();
    expect(sent("list_notes")).toHaveLength(2);
    expect(tabs().at(-1)).toBe("New note");
    expect(selectedTab()).toBe("New note");
    expect(document.activeElement).toBe(root.querySelector("input.note-title"));
  });

  it("passive, it shows the note and nothing to click or type into", async () => {
    await render();
    await edit();
    send({ mode: "passive" });
    await settle();
    expect(root.querySelector(".overlay")?.classList.contains("passive")).toBe(true);
    expect(root.querySelector("textarea")).toBeNull();
    expect(root.querySelector("[role='tablist']")).toBeNull();
    expect(root.querySelector("input.note-title")).toBeNull();
    expect(root.querySelectorAll(".bar button")).toHaveLength(0);
    expect(root.querySelector(".passive-badge")?.textContent).toContain("Click-through · Ctrl + Shift + F2");
    const boxes = [...root.querySelectorAll<HTMLInputElement>("input[type='checkbox']")];
    expect(boxes.length).toBe(2);
    expect(boxes.every((b) => b.disabled)).toBe(true);
    expect(root.querySelector(".note-view")?.textContent).toContain("Prepare healing items");
    // Without a registered mode shortcut it says where to switch back.
    send({ hotkeys: [hotkey("toggle-mode", "Ctrl + Shift + F2", false)] });
    expect(root.querySelector(".passive-badge")?.textContent).toContain("switch back from sotyware");
  });

  it("collapsed, it shows only its title bar and the next item to do", async () => {
    await render();
    send({ collapsed: true });
    await settle();
    expect(root.querySelector(".content")).toBeNull();
    expect(root.querySelector(".next")?.textContent).toBe("Prepare healing items");
  });

  it("follows the opacity and the text size, and keeps text readable when see-through", async () => {
    await render();
    const panel = root.querySelector<HTMLElement>(".overlay");
    expect(panel?.getAttribute("style")).toContain("--panel-alpha: 0.88");
    expect(panel?.classList.contains("see-through")).toBe(false);
    send({ opacity: 40, textSize: "large" });
    expect(panel?.getAttribute("style")).toContain("--panel-alpha: 0.4");
    expect(panel?.dataset["size"]).toBe("large");
    expect(panel?.classList.contains("see-through")).toBe(true);
  });

  it("shows a shortcut's failure until dismissed", async () => {
    await render();
    send({ notice: "Ctrl + Shift + F3: There can be at most 500 notes; delete one first" });
    expect(root.querySelector("[role='alert']")?.textContent).toContain("at most 500 notes");
    button("Dismiss").click();
    flushSync();
    expect(root.querySelector("[role='alert']")).toBeNull();
  });
});

describe("the window's controls", () => {
  it("asks Rust to go passive, collapse, open the settings and hide", async () => {
    await render();
    button("Passive: let clicks through to the game").click();
    await settle();
    expect(sent("set_overlay_mode")).toEqual([{ mode: "passive" }]);
    send({ mode: "interactive" });
    button("Collapse to the title bar").click();
    await settle();
    expect(sent("set_overlay_collapsed")).toEqual([{ collapsed: true }]);
    button("Expand").click();
    await settle();
    button("Overlay settings").click();
    await settle();
    expect(sent("open_overlay_settings")).toHaveLength(1);
    button("Hide the overlay (notes are kept)").click();
    await settle();
    expect(sent("set_overlay_visible")).toEqual([{ visible: false }]);
  });

  it("previews the opacity while dragging and saves it when let go", async () => {
    await render();
    button("Background opacity, 88%").click();
    flushSync();
    const range = root.querySelector<HTMLInputElement>(".popover input[type='range']");
    if (!range) {
      throw new Error("no opacity slider");
    }
    range.value = "50";
    range.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(root.querySelector<HTMLElement>(".overlay")?.getAttribute("style")).toContain("--panel-alpha: 0.5");
    expect(sent("set_overlay_opacity")).toEqual([]);
    range.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(sent("set_overlay_opacity")).toEqual([{ opacity: 50 }]);
  });

  it("moves the window from its title bar and collapses it on a double click", async () => {
    await render();
    const bar = root.querySelector<HTMLElement>("header.bar");
    bar?.querySelector("h1")?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 1 }));
    await settle();
    expect(calls.some((c) => c.cmd === "plugin:window|start_dragging")).toBe(true);
    bar?.querySelector("h1")?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, detail: 2 }));
    await settle();
    expect(sent("set_overlay_collapsed")).toEqual([{ collapsed: true }]);
  });

  it("starts with an empty state and a New note button when there is no note", async () => {
    notes = [];
    activeNote = null;
    current = status({ activeNote: null, noteCount: 0 });
    await render();
    expect(root.textContent).toContain("No notes yet.");
    button("New note").click();
    await settle();
    expect(sent("create_note")).toHaveLength(1);
  });
});
