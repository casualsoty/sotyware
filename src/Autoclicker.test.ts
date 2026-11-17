// The Autoclicker page (ADR-050), headless with Tauri's IPC mocks: what it
// asks Rust, what it shows of the live status, that it asks before an
// endless run without an emergency stop, and what it sends when the user
// edits, records, saves, plays and switches. No input is ever sent here:
// that is Rust's, tested in src-tauri/src/automation.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Autoclicker from "./Autoclicker.svelte";
import { indicatorOf, AutomationModel } from "./automationModel.svelte";
import Sidebar from "./Sidebar.svelte";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let current: Record<string, unknown>;
let cfg: Record<string, unknown>;
let macros: Record<string, unknown>[];
let doc: Record<string, unknown>;
let channel: FakeChannel | null;
let failNext: { cmd: string; error: unknown } | null;
let app: ReturnType<typeof mount> | undefined;

const KEYS = [
  { code: "KeyA", label: "A", kind: "layout" },
  { code: "KeyP", label: "P", kind: "layout" },
  { code: "KeyS", label: "S", kind: "layout" },
  { code: "F6", label: "F6", kind: "function" },
  { code: "F7", label: "F7", kind: "function" },
  { code: "F8", label: "F8", kind: "function" },
  { code: "F9", label: "F9", kind: "function" },
  { code: "ControlLeft", label: "Left Ctrl", kind: "modifier" },
];

function clicker(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    button: "left",
    clickType: "single",
    intervalMs: 100,
    rateUnit: "cps",
    random: { mode: "off", minMs: 80, maxMs: 120, variationPercent: 20 },
    maxClicks: null,
    maxDurationMs: null,
    startDelayMs: 0,
    batch: null,
    target: { mode: "cursor", points: [], order: "sequential", area: null, spread: 0 },
    holdMs: 0,
    doubleGapMs: 50,
    advanced: false,
    ...over,
  };
}

function profile(id: string, name: string, over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id,
    name,
    clicker: clicker(),
    hotkeys: {
      "toggle-clicker": { chord: "F6", enabled: true },
      "pause-resume": { chord: "F7", enabled: true },
      "start-recording": { chord: "F9", enabled: true },
      "start-clicker": { chord: null, enabled: false },
    },
    macroId: null,
    playback: null,
    createdMs: 1,
    modifiedMs: 1,
    ...over,
  };
}

function config(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    settings: {
      defaultProfile: "profile-1",
      emergencyStop: { chord: "F8", enabled: true },
      failsafeCorners: true,
      dashboardCountdownMs: 3000,
      recording: {
        mouseMoves: true,
        mouseButtons: true,
        keyboard: true,
        scroll: true,
        coordinates: "absolute",
        moveRateHz: 60,
        minMovePx: 3,
        ignoredKeys: [],
        ignoredButtons: [],
      },
    },
    profiles: [profile("profile-1", "Default"), profile("profile-2", "Gaming")],
    activeProfile: "profile-1",
    keys: KEYS,
    suggestions: { "toggle-clicker": "F6", "stop-all": "F8" },
    ...over,
  };
}

function hotkey(owner: Record<string, unknown>, label: string, chord: string): Record<string, unknown> {
  return { owner, label, chord, keys: chord, registered: true, error: null };
}

function status(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    seq: 1,
    running: true,
    support: { available: true, backend: "windows", note: null },
    session: null,
    last: null,
    draft: null,
    hotkeys: [
      hotkey({ kind: "settings" }, "Emergency stop", "F8"),
      hotkey({ kind: "profile", action: "toggle-clicker" }, "Start or stop the autoclicker", "F6"),
    ],
    activeProfile: { id: "profile-1", name: "Default" },
    activeWindow: null,
    notice: null,
    storageError: null,
    ...over,
  };
}

function summary(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: "macro-1",
    name: "Daily report",
    description: "Opens the report",
    folder: "Work",
    favorite: false,
    shortcut: { chord: null, enabled: false },
    actions: 3,
    durationMs: 1500,
    createdMs: 1,
    modifiedMs: 2,
    target: null,
    hasPause: false,
    ...over,
  };
}

function macroDoc(): Record<string, unknown> {
  return {
    formatVersion: 1,
    appVersion: "0.1.2",
    id: "macro-1",
    name: "Daily report",
    description: "Opens the report",
    folder: "Work",
    favorite: false,
    shortcut: { chord: null, enabled: false },
    createdMs: 1,
    modifiedMs: 2,
    recordedOn: null,
    target: null,
    playback: {
      repeat: 1,
      stopAfterMs: null,
      speed: 1,
      startDelayMs: 0,
      repeatDelayMs: 0,
      jitterPercent: 0,
      positioning: "absolute",
      scaleToScreen: true,
      section: null,
    },
    actions: [
      { delayMs: 0, type: "move", x: 10, y: 20 },
      { delayMs: 500, type: "click", button: "left", count: 1, x: 10, y: 20 },
      { delayMs: 1000, type: "wait" },
    ],
  };
}

beforeEach(() => {
  calls = [];
  current = status();
  cfg = config();
  macros = [summary()];
  doc = macroDoc();
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
      case "automation_status":
        return current;
      case "watch_automation":
        channel = p["onEvent"] as FakeChannel;
        return 5;
      case "automation_config":
        return cfg;
      case "list_macros":
        return macros;
      case "get_macro":
        return doc;
      case "save_macro":
        doc = p["document"] as Record<string, unknown>;
        return { summary: summary({ actions: (doc["actions"] as unknown[]).length }), warnings: ["This macro presses F6, the shortcut of \"Start or stop the autoclicker\": playing it may trigger that shortcut"] };
      case "save_profile":
        return p["profile"];
      case "save_automation_settings":
        return p["settings"];
      case "save_recording":
        return summary({ id: "macro-2", name: p["name"], folder: p["folder"] });
      case "set_feature_enabled":
        current = status();
        return {
          feature: { id: "autoclicker", name: "Autoclicker", description: "", version: "0.1.0", state: "running", enabled: true, error: null },
          saveError: null,
        };
      case "activate_profile":
        cfg = { ...cfg, activeProfile: p["id"] };
        current = status({ activeProfile: { id: p["id"], name: "Gaming" } });
        return null;
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
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) {
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
  }
}

async function render(): Promise<{ root: HTMLElement; model: AutomationModel }> {
  const target = document.createElement("div");
  document.body.append(target);
  const model = new AutomationModel();
  await model.start();
  app = mount(Autoclicker, { target, props: { model } });
  flushSync();
  await settle();
  return { root: target, model };
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => text(b) === name);
  if (!found) {
    throw new Error(`no button "${name}" in ${text(root).slice(0, 2000)}`);
  }
  return found;
}

function tab(root: ParentNode, name: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((b) => text(b) === name);
  if (!found) {
    throw new Error(`no tab ${name}`);
  }
  return found;
}

function sent(cmd: string): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.payload);
}

function push(over: Record<string, unknown>): void {
  current = status({ seq: 10, ...over });
  channel?.onmessage(current);
  flushSync();
}

describe("availability", () => {
  it("explains a feature that is off and turns it on when asked", async () => {
    current = status({ running: false });
    const { root } = await render();
    expect(text(root)).toContain("The autoclicker is off.");
    expect(text(root)).toContain("Nothing starts by itself.");
    expect(button(root, "Start autoclicker").disabled).toBe(true);
    button(root, "Turn on").click();
    await settle();
    expect(sent("set_feature_enabled")).toEqual([{ id: "autoclicker", enabled: true }]);
    expect(text(root)).not.toContain("The autoclicker is off.");
  });

  it("says why a system has no input automation and offers nothing to start", async () => {
    current = status({
      running: false,
      support: { available: false, backend: "none", note: "This desktop session uses Wayland." },
    });
    const { root } = await render();
    expect(text(root)).toContain("Not available on this computer.");
    expect(text(root)).toContain("This desktop session uses Wayland.");
    expect([...root.querySelectorAll("button")].some((b) => text(b) === "Turn on")).toBe(false);
  });
});

describe("running automation", () => {
  it("starts the autoclicker from the dashboard and asks before an endless run without an emergency stop", async () => {
    const { root } = await render();
    failNext = {
      cmd: "start_clicker",
      error: { kind: "confirm", message: "The autoclicker will click until you stop it, and no emergency stop shortcut is active." },
    };
    button(root, "Start autoclicker").click();
    await settle();
    expect(sent("start_clicker")).toEqual([{ fromDashboard: true, confirmed: false }]);
    const dialog = document.querySelector('[role="dialog"]');
    expect(text(dialog)).toContain("no emergency stop shortcut is active");
    button(document.body, "Start anyway").click();
    await settle();
    expect(sent("start_clicker")).toEqual([
      { fromDashboard: true, confirmed: false },
      { fromDashboard: true, confirmed: true },
    ]);
  });

  it("shows the live session from the channel on every tab and stops it", async () => {
    const { root } = await render();
    push({
      session: {
        kind: "clicker",
        progress: { state: "running", clicks: 1234, elapsedMs: 65_000, countdownMs: null, cps: 19.5, skipped: 0, remaining: null },
        targetCps: 20,
        startedMs: 1,
        profile: "Default",
      },
    });
    const bar = root.querySelector('[aria-label="Running automation"]');
    expect(text(bar)).toContain("Clicking 1,234 clicks · 19.5/s · 1:05");
    expect(text(root.querySelector("#ov-status"))).toBe("Clicking");
    tab(root, "Macros").click();
    await settle();
    expect(text(root.querySelector('[aria-label="Running automation"]'))).toContain("1,234 clicks");
    button(root.querySelector('[aria-label="Running automation"]')!, "Stop").click();
    await settle();
    expect(sent("stop_automation")).toHaveLength(1);
  });

  it("shows a playback's progress and the program in front", async () => {
    const { root } = await render();
    push({
      session: {
        kind: "playback",
        progress: {
          state: "paused",
          repetition: 2,
          repetitions: 5,
          action: 3,
          actions: 10,
          label: "Left click",
          progress: 0.4,
          elapsedMs: 4000,
          remainingMs: 6000,
          countdownMs: null,
          skipped: 0,
          warning: "Playing although browser.exe is active, not notepad.exe",
          atPausePoint: true,
        },
        macroId: "macro-1",
        macroName: "Daily report",
        startedMs: 1,
      },
      activeWindow: { title: "Report", process: "browser.exe", client: { x: 0, y: 0, width: 800, height: 600 } },
    });
    expect(text(root)).toContain("Paused: “Daily report” 2 of 5");
    expect(text(root)).toContain("Paused at a pause point: resume to go on.");
    expect(text(root)).toContain("Playing although browser.exe is active, not notepad.exe");
    expect(text(root)).toContain("browser.exe · 800 × 600 at 0, 0");
    button(root.querySelector('[aria-label="Running automation"]')!, "Resume").click();
    await settle();
    expect(sent("set_automation_paused")).toEqual([{ paused: false }]);
  });
});

describe("the Clicker tab", () => {
  it("saves a changed speed to the active profile a moment later", async () => {
    const { root } = await render();
    tab(root, "Clicker").click();
    await settle();
    const rate = root.querySelector<HTMLInputElement>("#clicker-rate")!;
    expect(rate.value).toBe("10");
    rate.value = "20";
    rate.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 500));
    await settle();
    const saves = sent("save_profile");
    expect(saves).toHaveLength(1);
    const saved = saves[0]?.["profile"] as Record<string, unknown>;
    expect(saved["id"]).toBe("profile-1");
    expect((saved["clicker"] as Record<string, unknown>)["intervalMs"]).toBe(50);
  });

  it("saves a fixed position typed into its X and Y fields", async () => {
    const { root } = await render();
    tab(root, "Clicker").click();
    await settle();
    root.querySelectorAll<HTMLInputElement>('input[name="clicker-where"]')[1]!.click();
    await settle();
    // Named on their own, without their values (WebKitGTK adds those to a
    // wrapping label's text).
    const x = root.querySelector<HTMLInputElement>('input[aria-label="X"]')!;
    const y = root.querySelector<HTMLInputElement>('input[aria-label="Y"]')!;
    x.value = "640";
    x.dispatchEvent(new Event("change", { bubbles: true }));
    y.value = "360";
    y.dispatchEvent(new Event("change", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 500));
    await settle();
    const saved = sent("save_profile").at(-1)?.["profile"] as { clicker: { target: unknown } };
    expect(saved.clicker.target).toMatchObject({ mode: "points", points: [{ x: 640, y: 360 }] });
  });

  it("does not save a configuration Rust would refuse", async () => {
    const { root } = await render();
    tab(root, "Clicker").click();
    await settle();
    root.querySelector<HTMLInputElement>('input[type="radio"][name="clicker-type"]:not(:checked)')!.click();
    await settle();
    root.querySelector<HTMLInputElement>("#clicker-rate")!.value = "100";
    root.querySelector<HTMLInputElement>("#clicker-rate")!.dispatchEvent(new Event("change", { bubbles: true }));
    // Double clicks with a 50 ms gap cannot happen every 10 ms.
    await new Promise((r) => setTimeout(r, 500));
    await settle();
    expect(text(root)).toContain("longer than the shortest interval");
    expect(sent("save_profile")).toHaveLength(0);
    expect(button(root, "Start autoclicker").disabled).toBe(true);
  });
});

describe("the Recorder tab", () => {
  it("records from the dashboard and saves the stopped recording with its name", async () => {
    const { root } = await render();
    tab(root, "Recorder").click();
    await settle();
    expect(text(root)).toContain("Everything you type while recording is saved in the macro");
    button(root, "Start recording").click();
    await settle();
    expect(sent("start_recording")).toEqual([{ fromDashboard: true }]);
    push({
      draft: {
        actions: 12,
        durationMs: 4200,
        skipped: 1,
        recent: [{ delayMs: 0, type: "key", key: "KeyA", state: "down" }],
        window: { title: "Notes", process: "notes.exe", client: { x: 0, y: 0, width: 10, height: 10 } },
      },
    });
    await settle();
    expect(text(root)).toContain("Recording ready");
    expect(text(root)).toContain("12 actions, 4.2 s, first input in notes.exe");
    // Named on their own: WebKitGTK names an input inside its label with
    // the label's text plus the input's value or placeholder.
    const name = root.querySelector<HTMLInputElement>('.save-form input[aria-label="Name"]')!;
    name.value = "Notes macro";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    const folder = root.querySelector<HTMLInputElement>('.save-form input[aria-label="Folder (optional)"]')!;
    folder.value = "Work";
    folder.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    button(root, "Save macro").click();
    await settle();
    expect(sent("save_recording")).toEqual([{ name: "Notes macro", description: "", folder: "Work" }]);
  });
});

describe("the Macros tab", () => {
  it("lists, filters, plays and deletes macros after asking", async () => {
    macros = [summary(), summary({ id: "macro-9", name: "Game loop", folder: "", shortcut: { chord: "F6", enabled: false } })];
    const { root } = await render();
    tab(root, "Macros").click();
    await settle();
    const rows = (): string[] => [...root.querySelectorAll("tr[data-macro]")].map((r) => r.getAttribute("data-macro") ?? "");
    expect(rows()).toEqual(["macro-1", "macro-9"]);
    const search = root.querySelector<HTMLInputElement>('input[type="search"]')!;
    search.value = "game";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(rows()).toEqual(["macro-9"]);
    button(root.querySelector('tr[data-macro="macro-9"]')!, "Play").click();
    await settle();
    expect(sent("play_macro")).toEqual([{ id: "macro-9", fromDashboard: true, confirmed: false }]);
    root.querySelector<HTMLButtonElement>('[aria-label="Delete Game loop"]')!.click();
    await settle();
    expect(text(document.querySelector('[role="dialog"]'))).toContain("Delete “Game loop”?");
    expect(sent("delete_macro")).toHaveLength(0);
    button(document.body, "Delete macro").click();
    await settle();
    expect(sent("delete_macro")).toEqual([{ id: "macro-9" }]);
  });

  it("edits a macro and saves the whole macro, showing Rust's warnings", async () => {
    const { root } = await render();
    tab(root, "Macros").click();
    await settle();
    root.querySelector<HTMLButtonElement>('[aria-label="Edit Daily report"]')!.click();
    await settle();
    expect(sent("get_macro")).toEqual([{ id: "macro-1" }]);
    expect(text(root)).toContain("3 actions, 1.5 s at normal speed");
    const delay = root.querySelector<HTMLInputElement>('[aria-label="Action 2: delay before it in ms"]')!;
    delay.value = "250";
    delay.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(text(root)).toContain("Unsaved changes");
    button(root, "Save").click();
    await settle();
    const saved = sent("save_macro")[0]?.["document"] as Record<string, unknown>;
    expect((saved["actions"] as Record<string, unknown>[])[1]).toEqual({ delayMs: 250, type: "click", button: "left", count: 1, x: 10, y: 20 });
    expect(text(root)).toContain("playing it may trigger that shortcut");
  });

  it("removes pointer movements and keeps their time", async () => {
    const { root } = await render();
    tab(root, "Macros").click();
    await settle();
    root.querySelector<HTMLButtonElement>('[aria-label="Edit Daily report"]')!.click();
    await settle();
    button(root, "Remove pointer movements").click();
    await settle();
    expect(text(root)).toContain("1 pointer movements removed; their time was kept.");
    expect(text(root)).toContain("2 actions, 1.5 s");
  });
});

describe("the Settings tab", () => {
  it("records a shortcut from a key press and saves it on the profile", async () => {
    const { root } = await render();
    tab(root, "Settings").click();
    await settle();
    const field = root.querySelector<HTMLButtonElement>('[aria-label^="Start the autoclicker:"]')!;
    field.click();
    flushSync();
    // A letter alone would take the key from typing: refused at once.
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "a", code: "KeyA", bubbles: true }));
    flushSync();
    expect(text(root)).toContain("alone would stop that key from working everywhere");
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "p", code: "KeyP", ctrlKey: true, altKey: true, bubbles: true }));
    await settle();
    const saved = sent("save_profile")[0]?.["profile"] as Record<string, Record<string, unknown>>;
    expect(saved["hotkeys"]?.["start-clicker"]).toEqual({ chord: "Ctrl+Alt+KeyP", enabled: true });
  });

  it("shows why Rust refused a shortcut", async () => {
    const { root } = await render();
    tab(root, "Settings").click();
    await settle();
    failNext = { cmd: "save_profile", error: { kind: "conflict", message: "F8 is already the shortcut of \"Emergency stop\"" } };
    const field = root.querySelector<HTMLButtonElement>('[aria-label^="Start the autoclicker:"]')!;
    field.click();
    flushSync();
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "F8", code: "F8", bubbles: true }));
    await settle();
    expect(text(root.querySelector('[role="alert"]'))).toContain("already the shortcut of \"Emergency stop\"");
  });
});

describe("profiles", () => {
  it("switch from the page header and take effect at once", async () => {
    const { root } = await render();
    const select = root.querySelector<HTMLSelectElement>(".profile-switch select")!;
    select.value = "profile-2";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(sent("activate_profile")).toEqual([{ id: "profile-2" }]);
    expect(sent("automation_config").length).toBeGreaterThanOrEqual(2);
  });
});

describe("the sidebar's indicator", () => {
  it("tells what runs, in words as well as colour", () => {
    expect(indicatorOf(null)).toBeNull();
    const rec = status({
      session: { kind: "recording", state: "running", elapsedMs: 0, countdownMs: null, actions: 0, recent: [], skipped: 0, startedMs: 1 },
    });
    // The parsed shape is what the model holds; the raw one has the same fields.
    expect(indicatorOf(rec as never)).toBe("recording");
    const target = document.createElement("div");
    document.body.append(target);
    app = mount(Sidebar, {
      target,
      props: {
        section: "overview",
        browsing: null,
        accounts: [],
        automation: "recording",
        onnavigate: () => undefined,
        onopenaccount: () => undefined,
        onaddaccount: () => undefined,
      },
    });
    flushSync();
    const link = [...target.querySelectorAll("a")].find((a) => text(a).startsWith("Autoclicker"));
    expect(text(link)).toBe("Autoclicker, recording");
    expect(link?.querySelector("[data-sidebar-automation]")?.getAttribute("data-sidebar-automation")).toBe("recording");
  });
});
