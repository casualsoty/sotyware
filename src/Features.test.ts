// Headless tests of the Features section and its typed command wrapper. The Rust
// side is Tauri's IPC mock, playing a registry with the same rules as
// src-tauri/src/features/registry.rs for the cases used here. Whether the real
// capability allows the commands is checked by the desktop tests.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "./App.svelte";
import Features from "./Features.svelte";
import { LIST_FEATURES, SET_FEATURE_ENABLED, type FeatureInfo, parseFeature } from "./features";

interface IpcCall {
  cmd: string;
  payload: unknown;
}

function feature(overrides: Partial<FeatureInfo> = {}): FeatureInfo {
  return {
    id: "example",
    name: "Example",
    description: "Demonstrates the built-in feature lifecycle.",
    version: "0.1.0",
    state: "running",
    enabled: true,
    error: null,
    ...overrides,
  };
}

let calls: IpcCall[];
let features: FeatureInfo[];
let respond: (cmd: string, payload: unknown) => unknown;
let app: ReturnType<typeof mount> | undefined;

/** Answers like the registry: a switch returns the new state of that feature. */
function backend(cmd: string, payload: unknown): unknown {
  if (cmd === LIST_FEATURES) {
    return features;
  }
  if (cmd === SET_FEATURE_ENABLED) {
    const { id, enabled } = payload as { id: string; enabled: boolean };
    const current = features.find((f) => f.id === id);
    if (!current) {
      return Promise.reject(`unknown feature '${id}'`);
    }
    const next = enabled
      ? feature({ ...current, state: "running", enabled: true, error: null })
      : feature({ ...current, state: "disabled", enabled: false, error: null });
    features = features.map((f) => (f.id === id ? next : f));
    return next;
  }
  if (cmd === "operating_mode") {
    return "tray";
  }
  return null;
}

beforeEach(() => {
  calls = [];
  features = [feature(), feature({ id: "other", name: "Other", state: "disabled", enabled: false })];
  respond = backend;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload });
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

describe("feature list", () => {
  it("asks for the list once on mount and changes nothing", async () => {
    await render();
    expect(calls.map((c) => c.cmd)).toEqual([LIST_FEATURES]);
  });

  it("shows each feature's name, version, state and description in order", async () => {
    const root = await render();
    expect(root.querySelector("h2")?.textContent).toBe("Features");
    expect([...root.querySelectorAll("li")].map((li) => li.dataset["feature"])).toEqual(["example", "other"]);
    expect(item(root, "example").querySelector("label")?.textContent?.trim()).toBe("Example");
    expect(item(root, "example").textContent).toContain("0.1.0");
    expect(item(root, "example").textContent).toContain("Demonstrates the built-in feature lifecycle.");
    expect(stateText(root, "example")).toBe("Running");
    expect(stateText(root, "other")).toBe("Disabled");
    expect(checkbox(root, "example").checked).toBe(true);
    expect(checkbox(root, "other").checked).toBe(false);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("shows a failed feature as enabled, with its error", async () => {
    features = [feature({ state: "failed", enabled: true, error: "port in use" })];
    const root = await render();
    expect(stateText(root, "example")).toBe("Failed");
    expect(checkbox(root, "example").checked).toBe(true);
    expect(item(root, "example").querySelector(".feature-error")?.textContent).toBe("port in use");
    // A feature's own failure is its state, not an alert about the page.
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("does not offer a switch while a feature is starting or stopping", async () => {
    features = [
      feature({ id: "a", state: "initializing", enabled: true }),
      feature({ id: "b", state: "stopping", enabled: false }),
      feature({ id: "c", state: "registered", enabled: false }),
    ];
    const root = await render();
    expect(checkbox(root, "a").disabled).toBe(true);
    expect(checkbox(root, "b").disabled).toBe(true);
    expect(checkbox(root, "c").disabled).toBe(false);
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
      cmd === LIST_FEATURES ? [{ ...feature(), state: "sleeping" }] : backend(cmd, payload);
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent).toMatch(/^Could not list features: Error: invalid feature: /);
  });
});

describe("switching a feature", () => {
  it("disables a running feature", async () => {
    const root = await render();
    toggle(checkbox(root, "example"));
    await settle();
    expect(switchCalls()).toEqual([{ cmd: SET_FEATURE_ENABLED, payload: { id: "example", enabled: false } }]);
    expect(stateText(root, "example")).toBe("Disabled");
    expect(checkbox(root, "example").checked).toBe(false);
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
      cmd === SET_FEATURE_ENABLED ? feature({ id: "other", name: "Other", state: "failed", enabled: true, error: "boom" }) : backend(cmd, payload);
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
        ? Promise.reject("cannot disable feature 'example' while it is Stopping")
        : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "example"));
    await settle();
    expect(checkbox(root, "example").checked).toBe(true);
    expect(stateText(root, "example")).toBe("Running");
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not disable Example: cannot disable feature 'example' while it is Stopping",
    );
  });

  it("clears an earlier error after a successful switch", async () => {
    let fail = true;
    respond = (cmd, payload): unknown =>
      cmd === SET_FEATURE_ENABLED && fail ? Promise.reject("busy") : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "example"));
    await settle();
    expect(root.querySelector('[role="alert"]')).not.toBeNull();
    fail = false;
    toggle(checkbox(root, "example"));
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(stateText(root, "example")).toBe("Disabled");
  });

  it("offers no switch while a request is in flight", async () => {
    let finish: (value: unknown) => void = () => {};
    respond = (cmd, payload): unknown =>
      cmd === SET_FEATURE_ENABLED
        ? new Promise((resolve) => {
            finish = (): void => resolve(backend(cmd, payload));
          })
        : backend(cmd, payload);
    const root = await render();
    toggle(checkbox(root, "example"));
    await settle();
    expect(checkbox(root, "example").disabled).toBe(true);
    expect(checkbox(root, "other").disabled).toBe(true);
    finish(null);
    await settle();
    expect(checkbox(root, "example").disabled).toBe(false);
    expect(switchCalls()).toHaveLength(1);
  });
});

describe("features on the dashboard", () => {
  it("lists features below the dashboard content", async () => {
    const root = await render(App);
    expect(root.querySelector("main h1")?.textContent).toBe("sotyware");
    expect(stateText(root, "example")).toBe("Running");
    // Checkboxes, not buttons: the page's only button stays "Hide dashboard".
    expect([...root.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Hide dashboard"]);
  });
});

describe("parseFeature", () => {
  it("accepts the shape Rust sends", () => {
    expect(parseFeature(feature({ error: "x" }))).toEqual(feature({ error: "x" }));
  });

  it("rejects anything else", () => {
    for (const bad of [
      null,
      "example",
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
