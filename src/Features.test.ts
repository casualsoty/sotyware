// Headless tests of the Features section and its typed command wrapper. The Rust
// side is Tauri's IPC mock, playing a registry with the same rules as
// src-tauri/src/features/registry.rs for the cases used here. Whether the real
// capability allows the commands is checked by the desktop tests.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "./App.svelte";
import Features from "./Features.svelte";
import {
  LIST_FEATURES,
  SET_FEATURE_ENABLED,
  type FeatureInfo,
  parseFeature,
  parseFeatureChange,
  parseFeatureList,
} from "./features";

interface IpcCall {
  cmd: string;
  payload: unknown;
}

function feature(overrides: Partial<FeatureInfo> = {}): FeatureInfo {
  return {
    id: "backup-scheduler",
    name: "Scheduled backups",
    description: "Runs backup jobs that have an interval when they are due, while sotyware is running.",
    version: "0.1.0",
    state: "running",
    enabled: true,
    error: null,
    ...overrides,
  };
}

let calls: IpcCall[];
let features: FeatureInfo[];
let storageError: string | null;
let saveError: string | null;
let respond: (cmd: string, payload: unknown) => unknown;
/** When set, each switch waits here until `answer()` answers it, oldest
 * first; otherwise the registry answers at once. */
let held: ((error?: unknown) => void)[] | null;
let app: ReturnType<typeof mount> | undefined;

/** Answers like the registry: a switch returns the new state of that feature,
 * and `saveError` as the preference store's answer; a feature starting or
 * stopping is not switched. */
function backend(cmd: string, payload: unknown): unknown {
  if (cmd === LIST_FEATURES) {
    return { features, storageError };
  }
  if (cmd === SET_FEATURE_ENABLED) {
    const { id, enabled } = payload as { id: string; enabled: boolean };
    const current = features.find((f) => f.id === id);
    if (!current) {
      return Promise.reject(`unknown feature '${id}'`);
    }
    if (current.state === "initializing" || current.state === "stopping") {
      const state = current.state === "initializing" ? "Initializing" : "Stopping";
      return Promise.reject(`cannot ${enabled ? "enable" : "disable"} feature '${id}' while it is ${state}`);
    }
    const next = enabled
      ? feature({ ...current, state: "running", enabled: true, error: null })
      : feature({ ...current, state: "disabled", enabled: false, error: null });
    features = features.map((f) => (f.id === id ? next : f));
    return { feature: next, saveError };
  }
  if (cmd === "operating_mode") {
    return "tray";
  }
  return null;
}

beforeEach(() => {
  calls = [];
  features = [feature(), feature({ id: "other", name: "Other", state: "disabled", enabled: false })];
  storageError = null;
  saveError = null;
  respond = backend;
  held = null;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload });
    const waiting = held;
    if (cmd === SET_FEATURE_ENABLED && waiting !== null) {
      return new Promise((resolve, reject) => {
        waiting.push((error) => (error === undefined ? resolve(respond(cmd, payload)) : reject(error)));
      });
    }
    return respond(cmd, payload);
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
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function render(component: typeof Features | typeof App = Features): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(component, { target });
  flushSync();
  await settle();
  return target;
}

function item(root: HTMLElement, id: string): HTMLElement {
  const li = root.querySelector<HTMLElement>(`li[data-feature="${id}"]`);
  if (!li) {
    throw new Error(`no feature ${id} in: ${root.innerHTML}`);
  }
  return li;
}

function checkbox(root: HTMLElement, id: string): HTMLInputElement {
  const input = item(root, id).querySelector<HTMLInputElement>('input[type="checkbox"]');
  if (!input) {
    throw new Error(`no checkbox for ${id}`);
  }
  return input;
}

function stateText(root: HTMLElement, id: string): string | null | undefined {
  return item(root, id).querySelector(".state")?.textContent;
}

function switchCalls(): IpcCall[] {
  return calls.filter((c) => c.cmd === SET_FEATURE_ENABLED);
}

/** Changes a checkbox the way a click does: toggles it, then fires a bubbling
 * `change` (Svelte delegates the event to the document root). */
function toggle(input: HTMLInputElement): void {
  input.checked = !input.checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

/** Answers the oldest switch still waiting: made, or refused with `error`. */
async function answer(error?: unknown): Promise<void> {
  const next = held?.shift();
  if (next === undefined) {
    throw new Error("no switch is waiting");
  }
  next(error);
  await settle();
}

/** The features whose checkbox is disabled. */
function disabledBoxes(root: HTMLElement): string[] {
  return [...root.querySelectorAll<HTMLInputElement>('li[data-feature] input[type="checkbox"]')]
    .filter((input) => input.disabled)
    .map((input) => input.closest("li")?.dataset["feature"] ?? "?");
}

describe("feature list", () => {
  it("asks for the list once on mount and changes nothing", async () => {
    await render();
    expect(calls.map((c) => c.cmd)).toEqual([LIST_FEATURES]);
  });

  it("shows each feature's name, version, state and description in order", async () => {
    const root = await render();
    expect(root.querySelector("h2")?.textContent).toBe("Features");
    expect([...root.querySelectorAll("li")].map((li) => li.dataset["feature"])).toEqual(["backup-scheduler", "other"]);
    expect(item(root, "backup-scheduler").querySelector("label")?.textContent?.trim()).toBe("Scheduled backups");
    expect(item(root, "backup-scheduler").textContent).toContain("0.1.0");
    expect(item(root, "backup-scheduler").textContent).toContain("Runs backup jobs that have an interval when they are due");
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    expect(stateText(root, "other")).toBe("Disabled");
    expect(checkbox(root, "backup-scheduler").checked).toBe(true);
    expect(checkbox(root, "other").checked).toBe(false);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("shows a failed feature as enabled, with its error", async () => {
    features = [feature({ state: "failed", enabled: true, error: "port in use" })];
    const root = await render();
    expect(stateText(root, "backup-scheduler")).toBe("Failed");
    expect(checkbox(root, "backup-scheduler").checked).toBe(true);
    expect(item(root, "backup-scheduler").querySelector(".feature-error")?.textContent).toBe("port in use");
    // A feature's own failure is its state, not an alert about the page.
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  // A feature switched elsewhere (the notes overlay's settings, the
  // Autoclicker page) can still be starting or stopping when the list is
  // read. Its checkbox stays enabled (until 2026-11-25 it was disabled until
  // the page was shown again), but no switch is sent while the registry
  // would refuse it: the page reads the list again first.
  it("reads the list again before switching a feature it saw starting or stopping, and sends nothing while it still is", async () => {
    features = [
      feature({ id: "a", name: "A", state: "initializing", enabled: true }),
      feature({ id: "b", name: "B", state: "stopping", enabled: false }),
      feature({ id: "c", name: "C", state: "registered", enabled: false }),
    ];
    const root = await render();
    expect(disabledBoxes(root)).toEqual([]);
    expect(stateText(root, "a")).toBe("Starting…");
    expect(stateText(root, "b")).toBe("Stopping…");
    toggle(checkbox(root, "a"));
    await settle();
    expect(calls.map((c) => c.cmd)).toEqual([LIST_FEATURES, LIST_FEATURES]);
    expect(root.querySelector('[role="alert"]')?.textContent).toBe("Could not disable A: it is still starting. Try again in a moment.");
    expect(checkbox(root, "a").checked).toBe(true);
    expect(stateText(root, "a")).toBe("Starting…");
    // Both have finished meanwhile: the list read again says so, and the
    // switch is sent.
    features = features.map((f) =>
      f.id === "a" ? { ...f, state: "running" } : f.id === "b" ? { ...f, state: "disabled" } : f,
    );
    toggle(checkbox(root, "a"));
    toggle(checkbox(root, "b"));
    await settle();
    expect(switchCalls().map((c) => c.payload)).toEqual([
      { id: "a", enabled: false },
      { id: "b", enabled: true },
    ]);
    expect(calls.filter((c) => c.cmd === LIST_FEATURES)).toHaveLength(3);
    expect([stateText(root, "a"), stateText(root, "b"), stateText(root, "c")]).toEqual(["Disabled", "Running", "Not started yet"]);
    expect([checkbox(root, "a").checked, checkbox(root, "b").checked]).toEqual([false, true]);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("sends nothing for a feature it saw stopping that is now where the switch asks", async () => {
    features = [feature({ id: "a", name: "A", state: "stopping", enabled: false })];
    const root = await render();
    toggle(checkbox(root, "a"));
    // Switched on again elsewhere meanwhile.
    features = [feature({ id: "a", name: "A", state: "running", enabled: true })];
    await settle();
    expect(switchCalls()).toEqual([]);
    expect(stateText(root, "a")).toBe("Running");
    expect(checkbox(root, "a").checked).toBe(true);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("says why and sends nothing when the list cannot be read again", async () => {
    features = [feature({ id: "a", name: "A", state: "stopping", enabled: false })];
    const root = await render();
    respond = (cmd, payload): unknown => (cmd === LIST_FEATURES ? Promise.reject("list_features not allowed") : backend(cmd, payload));
    toggle(checkbox(root, "a"));
    await settle();
    expect(switchCalls()).toEqual([]);
    expect(root.querySelector('[role="alert"]')?.textContent).toBe("Could not list features: list_features not allowed");
    expect(checkbox(root, "a").checked).toBe(false);
    expect(stateText(root, "a")).toBe("Stopping…");
  });

  it("says so when there are no features", async () => {
    features = [];
    const root = await render();
    expect(root.textContent).toContain("No built-in features.");
  });

  it("shows an error when the list cannot be read", async () => {
    respond = (cmd, payload): unknown =>
      cmd === LIST_FEATURES ? Promise.reject("list_features not allowed") : backend(cmd, payload);
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not list features: list_features not allowed",
    );
    expect(root.querySelectorAll("li")).toHaveLength(0);
  });

  it("treats a malformed answer as an error", async () => {
    respond = (cmd, payload): unknown =>
      cmd === LIST_FEATURES ? { features: [{ ...feature(), state: "sleeping" }], storageError: null } : backend(cmd, payload);
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent).toMatch(/^Could not list features: Error: invalid feature: /);
  });
});

describe("switching a feature", () => {
  it("disables a running feature", async () => {
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(switchCalls()).toEqual([{ cmd: SET_FEATURE_ENABLED, payload: { id: "backup-scheduler", enabled: false } }]);
    expect(stateText(root, "backup-scheduler")).toBe("Disabled");
    expect(checkbox(root, "backup-scheduler").checked).toBe(false);
    expect(stateText(root, "other")).toBe("Disabled");
  });

  it("enables a disabled feature", async () => {
    const root = await render();
    toggle(checkbox(root, "other"));
    await settle();
    expect(switchCalls()).toEqual([{ cmd: SET_FEATURE_ENABLED, payload: { id: "other", enabled: true } }]);
    expect(stateText(root, "other")).toBe("Running");
    expect(checkbox(root, "other").checked).toBe(true);
  });

  it("shows a feature that fails to start as failed, without an alert", async () => {
    respond = (cmd, payload): unknown =>
      cmd === SET_FEATURE_ENABLED
        ? { feature: feature({ id: "other", name: "Other", state: "failed", enabled: true, error: "boom" }), saveError: null }
        : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "other"));
    await settle();
    expect(stateText(root, "other")).toBe("Failed");
    expect(item(root, "other").querySelector(".feature-error")?.textContent).toBe("boom");
    expect(checkbox(root, "other").checked).toBe(true);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("puts the checkbox back and shows the error when Rust refuses", async () => {
    respond = (cmd, payload): unknown =>
      cmd === SET_FEATURE_ENABLED
        ? Promise.reject("cannot disable feature 'backup-scheduler' while it is Stopping")
        : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(checkbox(root, "backup-scheduler").checked).toBe(true);
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not disable Scheduled backups: cannot disable feature 'backup-scheduler' while it is Stopping",
    );
  });

  it("clears an earlier error after a successful switch", async () => {
    let fail = true;
    respond = (cmd, payload): unknown =>
      cmd === SET_FEATURE_ENABLED && fail ? Promise.reject("busy") : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(root.querySelector('[role="alert"]')).not.toBeNull();
    fail = false;
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(stateText(root, "backup-scheduler")).toBe("Disabled");
  });

});

describe("while a switch runs", () => {
  // WebKitGTK takes the keyboard focus from a control that becomes
  // disabled: while the page disabled every checkbox for each switch, Space
  // pressed again changed nothing. They now stay enabled, and the switches
  // are sent one after another, as before.
  it("keeps the checkboxes enabled and focused through quick switches, sent one after another", async () => {
    const root = await render();
    held = [];
    const box = checkbox(root, "backup-scheduler");
    box.focus();
    toggle(box);
    await settle();
    expect(switchCalls().map((c) => c.payload)).toEqual([{ id: "backup-scheduler", enabled: false }]);
    expect(disabledBoxes(root)).toEqual([]);
    expect(document.activeElement).toBe(box);
    expect(stateText(root, "backup-scheduler")).toBe("Stopping…");
    // Space again, and another feature, before the first switch is answered.
    toggle(box);
    toggle(checkbox(root, "other"));
    await settle();
    expect(switchCalls()).toHaveLength(1);
    expect([box.checked, checkbox(root, "other").checked]).toEqual([true, true]);
    expect([stateText(root, "backup-scheduler"), stateText(root, "other")]).toEqual(["Starting…", "Starting…"]);
    await answer();
    // The first answer (disabled) does not take the checkbox back off.
    expect(box.checked).toBe(true);
    expect(stateText(root, "backup-scheduler")).toBe("Starting…");
    expect(disabledBoxes(root)).toEqual([]);
    expect(switchCalls()).toHaveLength(2);
    await answer();
    await answer();
    expect(switchCalls().map((c) => c.payload)).toEqual([
      { id: "backup-scheduler", enabled: false },
      { id: "backup-scheduler", enabled: true },
      { id: "other", enabled: true },
    ]);
    expect([stateText(root, "backup-scheduler"), stateText(root, "other")]).toEqual(["Running", "Running"]);
    expect([box.checked, checkbox(root, "other").checked]).toEqual([true, true]);
    expect(document.activeElement).toBe(box);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("puts a refused switch back once no later switch of that feature waits, and says why", async () => {
    const root = await render();
    held = [];
    const box = checkbox(root, "backup-scheduler");
    toggle(box);
    toggle(box);
    await settle();
    await answer("cannot save: the registry is busy");
    // The later switch still waits: the checkbox stays on it.
    expect(box.checked).toBe(true);
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not disable Scheduled backups: cannot save: the registry is busy",
    );
    // It asks for what is there already (running): nothing more is sent.
    expect(switchCalls()).toHaveLength(1);
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    toggle(box);
    await settle();
    expect(box.checked).toBe(false);
    await answer("cannot save: the registry is busy");
    expect(box.checked).toBe(true);
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    expect(switchCalls()).toHaveLength(2);
  });
});

describe("saved choices", () => {
  it("shows nothing about storage when the saved choices loaded", async () => {
    const root = await render();
    expect(root.textContent).not.toContain("saved feature choices");
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("says so when the saved choices could not be loaded, and still lists the features", async () => {
    storageError = "/data/preferences.sqlite3: cannot read: file is not a database";
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent?.replace(/\s+/g, " ").trim()).toBe(
      "Could not load the saved feature choices: /data/preferences.sqlite3: cannot read: file is not a database. " +
        "Every feature uses its default, and changes are not saved.",
    );
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    expect(checkbox(root, "backup-scheduler").disabled).toBe(false);
  });

  it("keeps the storage warning after a switch", async () => {
    storageError = "no app data folder";
    saveError = "preferences are unavailable: no app data folder";
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    const alerts = [...root.querySelectorAll('[role="alert"]')].map((a) => a.textContent ?? "");
    expect(alerts).toHaveLength(2);
    expect(alerts[0]).toContain("Could not load the saved feature choices: no app data folder.");
    expect(alerts[1]).toContain("Could not save the choice for Scheduled backups");
  });

  it("shows the new state and an alert when a disable could not be saved", async () => {
    saveError = "cannot save: disk I/O error";
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    // The switch happened: the page shows the real state, not the old one.
    expect(stateText(root, "backup-scheduler")).toBe("Disabled");
    expect(checkbox(root, "backup-scheduler").checked).toBe(false);
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not save the choice for Scheduled backups: cannot save: disk I/O error. " +
        "It stays disabled until sotyware quits; the next start uses the last saved choice or the default.",
    );
  });

  it("shows the new state and an alert when an enable could not be saved", async () => {
    saveError = "sotyware is quitting";
    const root = await render();
    toggle(checkbox(root, "other"));
    await settle();
    expect(stateText(root, "other")).toBe("Running");
    expect(checkbox(root, "other").checked).toBe(true);
    expect(root.querySelector('[role="alert"]')?.textContent).toContain(
      "Could not save the choice for Other: sotyware is quitting. It stays enabled until sotyware quits;",
    );
  });

  it("clears the save alert after a switch that was saved", async () => {
    saveError = "disk full";
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(root.querySelector('[role="alert"]')).not.toBeNull();
    saveError = null;
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(stateText(root, "backup-scheduler")).toBe("Running");
  });

  it("treats a malformed switch answer as an error and puts the checkbox back", async () => {
    respond = (cmd, payload): unknown => (cmd === SET_FEATURE_ENABLED ? feature({ state: "disabled", enabled: false }) : backend(cmd, payload));
    const root = await render();
    toggle(checkbox(root, "backup-scheduler"));
    await settle();
    expect(checkbox(root, "backup-scheduler").checked).toBe(true);
    expect(root.querySelector('[role="alert"]')?.textContent).toMatch(/^Could not disable Scheduled backups: Error: invalid feature change: /);
  });
});

describe("features in Settings", () => {
  it("lists features under Settings › Features, not on the Overview", async () => {
    const root = await render(App);
    expect(root.querySelector("main h1")?.textContent).toBe("Overview");
    expect(root.querySelector("li[data-feature]")).toBeNull();
    expect(calls.map((c) => c.cmd)).not.toContain(LIST_FEATURES);

    [...root.querySelectorAll<HTMLAnchorElement>("nav a, a")].find((a) => a.textContent?.trim() === "Settings")!.click();
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Settings");
    [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((t) => t.textContent?.trim() === "Features")!.click();
    await settle();
    expect(stateText(root, "backup-scheduler")).toBe("Running");
    expect(calls.filter((c) => c.cmd === LIST_FEATURES)).toHaveLength(1);
    // Checkboxes, not buttons: the features panel adds no button of its own.
    const panel = root.querySelector<HTMLElement>('[role="tabpanel"]')!;
    expect(panel.querySelectorAll("button")).toHaveLength(0);
    expect(panel.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
  });
});

describe("parseFeatureList and parseFeatureChange", () => {
  it("accept the shapes Rust sends", () => {
    expect(parseFeatureList({ features: [feature()], storageError: null })).toEqual({ features: [feature()], storageError: null });
    expect(parseFeatureList({ features: [], storageError: "x" })).toEqual({ features: [], storageError: "x" });
    expect(parseFeatureChange({ feature: feature(), saveError: "disk full" })).toEqual({ feature: feature(), saveError: "disk full" });
  });

  it("reject anything else", () => {
    for (const bad of [null, [], [feature()], { features: [feature()] }, { features: {}, storageError: null }, { features: [], storageError: 1 }]) {
      expect(() => parseFeatureList(bad), JSON.stringify(bad)).toThrow(/^invalid feature list: /);
    }
    expect(() => parseFeatureList({ features: [{ id: 1 }], storageError: null })).toThrow(/^invalid feature: /);
    for (const bad of [null, feature(), { feature: feature() }, { feature: feature(), saveError: false }]) {
      expect(() => parseFeatureChange(bad), JSON.stringify(bad)).toThrow(/^invalid feature change: /);
    }
    expect(() => parseFeatureChange({ feature: null, saveError: null })).toThrow(/^invalid feature: /);
  });
});

describe("parseFeature", () => {
  it("accepts the shape Rust sends", () => {
    expect(parseFeature(feature({ error: "x" }))).toEqual(feature({ error: "x" }));
  });

  it("rejects anything else", () => {
    for (const bad of [
      null,
      "backup-scheduler",
      [],
      { ...feature(), id: 1 },
      { ...feature(), enabled: "yes" },
      { ...feature(), error: undefined },
      { ...feature(), state: "Running" },
    ]) {
      expect(() => parseFeature(bad), JSON.stringify(bad)).toThrow(/^invalid feature: /);
    }
  });
});
