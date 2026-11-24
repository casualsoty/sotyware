// Settings › Notes overlay (ADR-052), headless with Tauri's IPC mocks:
// what it shows of the overlay's status and settings, what each control
// saves, the shortcut table's states, and the reset. The overlay's own
// page is tested in NotesOverlay.test.ts, the rules in src-tauri.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import NotesOverlaySettings from "./NotesOverlaySettings.svelte";
import { OverlayModel } from "./overlayModel.svelte";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let current: Record<string, unknown>;
let cfg: Record<string, unknown>;
let channel: FakeChannel | null;
let failNext: { cmd: string; error: unknown } | null;
let app: ReturnType<typeof mount> | undefined;
let model: OverlayModel;
let root: HTMLElement;

const KEYS = [
  { code: "F1", label: "F1", kind: "function" },
  { code: "F2", label: "F2", kind: "function" },
  { code: "F3", label: "F3", kind: "function" },
  { code: "F4", label: "F4", kind: "function" },
  { code: "F5", label: "F5", kind: "function" },
  { code: "F9", label: "F9", kind: "function" },
  { code: "KeyN", label: "N", kind: "layout" },
  { code: "ControlLeft", label: "Left Ctrl", kind: "modifier" },
];

function settings(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    formatVersion: 1,
    opacity: 88,
    textSize: "medium",
    width: 360,
    height: 440,
    anchor: "top-right",
    rememberPlacement: true,
    defaultMode: "interactive",
    showAtStartup: false,
    newNote: "blank",
    openNote: "last",
    shortcuts: {
      "toggle-visible": { chord: "Ctrl+Shift+F1", enabled: true },
      "toggle-mode": { chord: "Ctrl+Shift+F2", enabled: true },
      "new-note": { chord: "Ctrl+Shift+F3", enabled: true },
      "previous-note": { chord: "Ctrl+Shift+F4", enabled: false },
      "next-note": { chord: null, enabled: false },
    },
    ...over,
  };
}

function config(): Record<string, unknown> {
  return {
    settings: settings(),
    suggestions: {
      "toggle-visible": "Ctrl+Shift+F1",
      "toggle-mode": "Ctrl+Shift+F2",
      "new-note": "Ctrl+Shift+F3",
      "previous-note": "Ctrl+Shift+F4",
      "next-note": "Ctrl+Shift+F5",
    },
    keys: KEYS,
    limits: {
      minWidth: 240,
      minHeight: 160,
      maxWidth: 1600,
      maxHeight: 1600,
      minOpacity: 20,
      maxOpacity: 100,
      maxNotes: 500,
      maxTitleChars: 120,
      maxBodyChars: 50000,
    },
  };
}

function hotkey(action: string, chord: string, registered = true, error: string | null = null): Record<string, unknown> {
  return { action, label: action, chord, keys: chord.replaceAll("+", " + "), registered, error };
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
    activeNote: null,
    activeSeq: 0,
    notesSeq: 0,
    focusSeq: 0,
    focusTarget: "content",
    settingsSeq: 0,
    noteCount: 3,
    hotkeys: [
      hotkey("toggle-visible", "Ctrl+Shift+F1"),
      hotkey("toggle-mode", "Ctrl+Shift+F2"),
      hotkey("new-note", "Ctrl+Shift+F3", false, "Not registered: another program uses this shortcut"),
    ],
    shortcutsNote: null,
    notice: null,
    storageError: null,
    ...over,
  };
}

function send(over: Record<string, unknown>): void {
  current = { ...current, ...over, seq: (current["seq"] as number) + 1 };
  channel?.onmessage(current);
  flushSync();
}

beforeEach(() => {
  calls = [];
  current = status();
  cfg = config();
  channel = null;
  failNext = null;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    if (failNext !== null && failNext.cmd === cmd) {
      const e = failNext.error;
      failNext = null;
      throw e;
    }
    switch (cmd) {
      case "watch_overlay":
        channel = p["onEvent"] as FakeChannel;
        channel.onmessage(current);
        return 4;
      case "overlay_config":
        return cfg;
      case "save_overlay_settings":
        cfg = { ...cfg, settings: p["settings"] };
        return p["settings"];
      case "reset_overlay":
        cfg = config();
        return settings();
      case "set_overlay_visible":
        current = { ...current, visible: p["visible"], seq: (current["seq"] as number) + 1 };
        return current;
      case "set_overlay_mode":
        current = { ...current, mode: p["mode"], seq: (current["seq"] as number) + 1 };
        return current;
      case "set_feature_enabled":
        // As Rust does: turned on by the user, the overlay shows at once.
        current = { ...current, running: p["enabled"], visible: p["enabled"], seq: (current["seq"] as number) + 1 };
        channel?.onmessage(current);
        return {
          feature: {
            id: "notes-overlay",
            name: "Gaming notes overlay",
            description: "",
            version: "0.1.0",
            state: p["enabled"] ? "running" : "disabled",
            enabled: p["enabled"],
            error: null,
          },
          saveError: null,
        };
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
  model?.stop();
  clearMocks();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  for (let i = 0; i < 4; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  model = new OverlayModel();
  await model.start();
  root = document.createElement("div");
  document.body.append(root);
  app = mount(NotesOverlaySettings, { target: root, props: { model } });
  flushSync();
  await settle();
  return root;
}

function button(label: string): HTMLButtonElement {
  const b = [...document.querySelectorAll("button")].find(
    (x) => x.textContent?.trim() === label || x.getAttribute("aria-label") === label,
  );
  if (!b) {
    throw new Error(`no button "${label}"`);
  }
  return b;
}

function saved(): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === "save_overlay_settings").map((c) => c.payload["settings"] as Record<string, unknown>);
}

function choose(id: string, value: string): void {
  const select = root.querySelector<HTMLSelectElement>(`#${id}`);
  if (!select) {
    throw new Error(`no ${id}`);
  }
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
}

function rows(): string[][] {
  return [...root.querySelectorAll("table.shortcuts tr")].map((tr) => [
    tr.querySelector("th")?.textContent?.trim() ?? "",
    tr.querySelector("td.state")?.textContent?.trim() ?? "",
  ]);
}

describe("turning it on and off", () => {
  it("off, it says so and turns the feature on", async () => {
    current = status({ running: false, visible: false, hotkeys: [] });
    await render();
    expect(root.querySelector("[role='status']")?.textContent).toContain("Off.");
    button("Turn on").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "set_feature_enabled").map((c) => c.payload)).toEqual([
      { id: "notes-overlay", enabled: true },
    ]);
    expect(root.querySelector("[role='status']")?.textContent).toContain("Shown");
  });

  it("on, it shows and hides the overlay, switches the mode and turns it off", async () => {
    await render();
    expect(root.querySelector("[role='status']")?.textContent).toMatch(/Shown\s*, interactive\s*· 3 notes/);
    button("Hide overlay").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "set_overlay_visible").map((c) => c.payload)).toEqual([{ visible: false }]);
    button("Show overlay").click();
    await settle();
    button("Make passive").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "set_overlay_mode").map((c) => c.payload)).toEqual([{ mode: "passive" }]);
    expect(root.textContent).toContain("Switch it back with Ctrl + Shift + F2, or here.");
    button("Make interactive").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "set_overlay_mode").at(-1)?.payload).toEqual({ mode: "interactive" });
    button("Turn off").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "set_feature_enabled").map((c) => c.payload)).toEqual([
      { id: "notes-overlay", enabled: false },
    ]);
  });

  it("passive without a mode shortcut, it says to switch back here", async () => {
    current = status({ mode: "passive", hotkeys: [] });
    await render();
    expect(root.textContent).toContain("Switch it back here.");
  });

  it("says when no global shortcut can work on this system", async () => {
    current = status({ shortcutsNote: "this desktop session uses Wayland, which does not let applications register global shortcuts" });
    await render();
    expect(root.querySelector(".callout.warning")?.textContent).toContain("No global shortcut can work here: this desktop session uses Wayland");
  });
});

describe("the settings", () => {
  it("saves each change at once, keeping the rest", async () => {
    await render();
    choose("overlay-text-size", "large");
    await settle();
    expect(saved()).toEqual([settings({ textSize: "large" })]);
    choose("overlay-mode", "passive");
    choose("overlay-anchor", "bottom-left");
    choose("overlay-new-note", "checklist");
    choose("overlay-open-note", "first-pinned");
    await settle();
    expect(saved().at(-1)).toEqual(
      settings({ textSize: "large", defaultMode: "passive", anchor: "bottom-left", newNote: "checklist", openNote: "first-pinned" }),
    );
    const opacity = root.querySelector<HTMLInputElement>("#overlay-opacity");
    if (opacity) {
      opacity.value = "45";
      opacity.dispatchEvent(new Event("change", { bubbles: true }));
    }
    await settle();
    expect(saved().at(-1)?.["opacity"]).toBe(45);
    const boxes = [...root.querySelectorAll<HTMLInputElement>("label.check.top input")];
    boxes[0]?.click();
    await settle();
    expect(saved().at(-1)?.["showAtStartup"]).toBe(true);
    boxes[1]?.click();
    await settle();
    expect(saved().at(-1)?.["rememberPlacement"]).toBe(false);
  });

  it("refuses a size outside the limits before asking Rust", async () => {
    await render();
    const width = root.querySelector<HTMLInputElement>("#overlay-width");
    if (!width) {
      throw new Error("no width");
    }
    width.value = "100";
    width.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(root.querySelector("[role='alert']")?.textContent).toBe("The width must be a whole number from 240 to 1600 pixels.");
    expect(saved()).toEqual([]);
    width.value = "420";
    width.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(saved()).toEqual([settings({ width: 420 })]);
    expect(root.querySelector("[role='alert']")).toBeNull();
  });

  it("shows Rust's refusal", async () => {
    await render();
    failNext = { cmd: "save_overlay_settings", error: { kind: "invalid-input", message: "\"New note\" and \"Next note\" have the same shortcut" } };
    choose("overlay-text-size", "small");
    await settle();
    expect(root.querySelector(".callout.danger")?.textContent).toContain("have the same shortcut");
  });

  it("resets after asking, keeping the notes", async () => {
    cfg = { ...config(), settings: settings({ opacity: 40 }) };
    await render();
    button("Reset overlay settings…").click();
    flushSync();
    expect(document.querySelector("[role='dialog']")?.textContent).toContain("Your notes are kept.");
    button("Reset").click();
    await settle();
    expect(calls.filter((c) => c.cmd === "reset_overlay")).toHaveLength(1);
    expect(root.querySelector<HTMLInputElement>("#overlay-opacity")?.value).toBe("88");
  });
});

describe("the shortcuts", () => {
  it("say whether the system accepted each one", async () => {
    await render();
    expect(rows()).toEqual([
      ["Show or hide the overlay", "Ready"],
      ["Switch between interactive and passive", "Ready"],
      ["New note", "Not registered: another program uses this shortcut"],
      ["Previous note", "Off"],
      ["Next note", "Not set"],
    ]);
    send({ running: false, hotkeys: [] });
    expect(rows()[0]?.[1]).toBe("Used while the overlay is on");
  });

  it("are changed by pressing keys, switched off and offered their suggestion", async () => {
    await render();
    const field = [...root.querySelectorAll<HTMLButtonElement>("table.shortcuts .field-button")][0];
    if (!field) {
      throw new Error("no shortcut field");
    }
    field.click();
    flushSync();
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "F9", code: "F9", ctrlKey: true, altKey: true, bubbles: true, cancelable: true }));
    await settle();
    expect((saved().at(-1)?.["shortcuts"] as Record<string, unknown>)["toggle-visible"]).toEqual({ chord: "Ctrl+Alt+F9", enabled: true });
    const toggle = root.querySelector<HTMLInputElement>("input[aria-label='Show or hide the overlay: on']");
    toggle?.click();
    await settle();
    expect((saved().at(-1)?.["shortcuts"] as Record<string, unknown>)["toggle-visible"]).toEqual({ chord: "Ctrl+Alt+F9", enabled: false });
    button("Use Ctrl + Shift + F5").click();
    await settle();
    expect((saved().at(-1)?.["shortcuts"] as Record<string, unknown>)["next-note"]).toEqual({ chord: "Ctrl+Shift+F5", enabled: true });
  });
});
