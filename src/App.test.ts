// Headless tests of the dashboard UI. They run in happy-dom (no browser, no
// display) with Tauri's official IPC mocks standing in for the Rust side, so they
// check what the frontend asks Tauri to do, not that the OS window really hides.
// That is covered by tests/desktop/windows.ps1 (real WebView2 window).
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App.svelte";

const DASHBOARD_LABEL = "main"; // DASHBOARD_LABEL in src-tauri/src/lib.rs

interface IpcCall {
  cmd: string;
  payload: unknown;
}

const MODE_CMD = "operating_mode"; // #[tauri::command] in src-tauri/src/lib.rs
const LIST_CMD = "list_features"; // #[tauri::command] in src-tauri/src/features/mod.rs; tested in Features.test.ts
const HIDE_CMD = "plugin:window|hide";
const UPDATE_CMD = "update_status"; // #[tauri::command] in src-tauri/src/updates.rs; tested in Updates.test.ts

let calls: IpcCall[];
let mode: unknown;
let respond: (cmd: string) => unknown;
let app: ReturnType<typeof mount> | undefined;

/** The Rust side: answers the mode command with `mode`, the feature list with no
 * features (Features.test.ts covers them), everything else with null. */
function backend(cmd: string): unknown {
  if (cmd === LIST_CMD) {
    return { features: [], storageError: null };
  }
  if (cmd === UPDATE_CMD) {
    return {
      configured: false,
      current: "0.1.0",
      channel: "stable",
      lastCheckMs: null,
      available: null,
      postponed: false,
      error: null,
      blockedBy: [],
    };
  }
  return cmd === MODE_CMD ? mode : null;
}

/** The commands sent so far, in a stable order (the page and its Features
 * section ask on mount independently). */
function sentCommands(): string[] {
  return calls.map((c) => c.cmd).sort();
}

beforeEach(() => {
  calls = [];
  mode = "tray";
  respond = backend;
  mockWindows(DASHBOARD_LABEL);
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload });
    return respond(cmd);
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

/** Runs every pending promise callback (the mocked IPC round trip), then Svelte's updates. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

/** Mounts the dashboard and waits until it has its operating mode from Rust. */
async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(App, { target });
  flushSync();
  await settle();
  return target;
}

function findHideButton(root: HTMLElement): HTMLButtonElement | undefined {
  return [...root.querySelectorAll("button")].find((b) => b.textContent === "Hide dashboard");
}

function hideButton(root: HTMLElement): HTMLButtonElement {
  const button = findHideButton(root);
  if (!button) {
    throw new Error(`no "Hide dashboard" button in: ${root.innerHTML}`);
  }
  return button;
}

function hideCalls(): IpcCall[] {
  return calls.filter((c) => c.cmd === HIDE_CMD);
}

describe("dashboard in tray mode", () => {
  it("asks Rust for the mode and the feature list once each, and nothing else, when it renders", async () => {
    await render();
    expect(sentCommands()).toEqual([LIST_CMD, MODE_CMD, UPDATE_CMD]);
  });

  it("renders the heading, the tray hint and a plain button, no error", async () => {
    const root = await render();
    expect(root.querySelector("h1")?.textContent).toBe("sotyware");
    expect(root.textContent).toContain("reopen it any time from the system tray");
    // type="button" so it never submits a form if the page grows one.
    expect(hideButton(root).type).toBe("button");
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(root.querySelector('[role="status"]')).toBeNull();
  });

  it("hides its own window when the button is clicked", async () => {
    const root = await render();
    hideButton(root).click();
    await settle();
    expect(hideCalls()).toEqual([{ cmd: HIDE_CMD, payload: { label: DASHBOARD_LABEL } }]);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("calls hide once per click", async () => {
    const root = await render();
    hideButton(root).click();
    hideButton(root).click();
    await settle();
    expect(hideCalls()).toHaveLength(2);
  });

  it("shows the error when hiding fails, and keeps the button usable", async () => {
    respond = (cmd): unknown => {
      if (cmd === HIDE_CMD) {
        throw new Error("window.hide not allowed");
      }
      return backend(cmd);
    };
    const root = await render();
    hideButton(root).click();
    await settle();
    const alert = root.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe("Could not hide the dashboard: Error: window.hide not allowed");
    expect(hideButton(root).disabled).toBe(false);
  });

  it("shows a rejected non-Error value too", async () => {
    // Tauri rejects with a plain string when the capability denies a command.
    respond = (cmd): unknown =>
      cmd === HIDE_CMD
        ? Promise.reject("window.hide not allowed. Permissions associated with this command: core:window:allow-hide")
        : backend(cmd);
    const root = await render();
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')?.textContent).toMatch(
      /^Could not hide the dashboard: window\.hide not allowed\./,
    );
  });

  it("clears an earlier error when a retry succeeds", async () => {
    let fail = true;
    respond = (cmd): unknown => {
      if (cmd === HIDE_CMD && fail) {
        throw new Error("busy");
      }
      return backend(cmd);
    };
    const root = await render();
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')).not.toBeNull();

    fail = false;
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(hideCalls()).toHaveLength(2);
  });
});

describe("dashboard in dashboard-only mode", () => {
  beforeEach(() => {
    mode = "dashboard-only";
  });

  it("explains that there is no tray and that closing quits", async () => {
    const root = await render();
    // Whitespace as a reader sees it: the template wraps the text over lines.
    const status = root.querySelector('[role="status"]')?.textContent?.replace(/\s+/g, " ");
    expect(status).toContain("No compatible system tray is available");
    expect(status).toContain("the dashboard cannot be hidden");
    expect(status).toContain("Keep this window open while you use sotyware.");
    expect(status).toContain("Closing it quits sotyware.");
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("offers no Hide action and never asks Tauri to hide", async () => {
    const root = await render();
    expect(findHideButton(root)).toBeUndefined();
    expect(root.querySelectorAll("button")).toHaveLength(0);
    expect(root.textContent).not.toContain("from the system tray");
    expect(sentCommands()).toEqual([LIST_CMD, MODE_CMD, UPDATE_CMD]);
  });
});

describe("dashboard when the mode cannot be read", () => {
  it("shows the error and offers no Hide action when the command fails", async () => {
    respond = (cmd): unknown => (cmd === MODE_CMD ? Promise.reject("operating_mode not allowed") : backend(cmd));
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      "Could not check whether a system tray is available: operating_mode not allowed",
    );
    expect(findHideButton(root)).toBeUndefined();
    expect(root.querySelector('[role="status"]')).toBeNull();
  });

  it("treats an unknown mode as an error, not as tray mode", async () => {
    mode = "background";
    const root = await render();
    expect(root.querySelector('[role="alert"]')?.textContent).toBe(
      'Could not check whether a system tray is available: Error: unknown operating mode: "background"',
    );
    expect(findHideButton(root)).toBeUndefined();
  });
});

describe("main.ts", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  it("fails loudly when #app is missing", async () => {
    await expect(import("./main")).rejects.toThrow("Missing #app mount element in index.html");
  });

  it("mounts the dashboard into #app", async () => {
    const target = document.createElement("div");
    target.id = "app";
    document.body.append(target);
    // After vi.resetModules() main.ts gets its own copy of svelte; only that copy
    // knows the component it mounted, so flush and unmount with it.
    const svelte = await import("svelte");
    const { default: mounted } = await import("./main");
    try {
      svelte.flushSync();
      expect(target.querySelector("h1")?.textContent).toBe("sotyware");
    } finally {
      svelte.unmount(mounted);
    }
  });
});
