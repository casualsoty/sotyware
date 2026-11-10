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

let calls: IpcCall[];
let respond: (cmd: string) => unknown;
let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  calls = [];
  respond = (): null => null;
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

function render(): HTMLElement {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(App, { target });
  flushSync();
  return target;
}

function hideButton(root: HTMLElement): HTMLButtonElement {
  const button = [...root.querySelectorAll("button")].find((b) => b.textContent === "Hide dashboard");
  if (!button) {
    throw new Error(`no "Hide dashboard" button in: ${root.innerHTML}`);
  }
  return button;
}

/** Runs every pending promise callback (the mocked IPC round trip), then Svelte's updates. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

describe("dashboard", () => {
  it("renders the heading and a plain button, no error", () => {
    const root = render();
    expect(root.querySelector("h1")?.textContent).toBe("sotyware");
    // type="button" so it never submits a form if the page grows one.
    expect(hideButton(root).type).toBe("button");
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(calls, "rendering must not call Tauri").toEqual([]);
  });

  it("hides its own window when the button is clicked", async () => {
    const root = render();
    hideButton(root).click();
    await settle();
    expect(calls).toEqual([{ cmd: "plugin:window|hide", payload: { label: DASHBOARD_LABEL } }]);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("calls hide once per click", async () => {
    const root = render();
    hideButton(root).click();
    hideButton(root).click();
    await settle();
    expect(calls.map((c) => c.cmd)).toEqual(["plugin:window|hide", "plugin:window|hide"]);
  });

  it("shows the error when hiding fails, and keeps the button usable", async () => {
    respond = (): never => {
      throw new Error("window.hide not allowed");
    };
    const root = render();
    hideButton(root).click();
    await settle();
    const alert = root.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe("Could not hide the dashboard: Error: window.hide not allowed");
    expect(hideButton(root).disabled).toBe(false);
  });

  it("shows a rejected non-Error value too", async () => {
    // Tauri rejects with a plain string when the capability denies a command.
    respond = (): Promise<never> => Promise.reject("window.hide not allowed. Permissions associated with this command: core:window:allow-hide");
    const root = render();
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')?.textContent).toMatch(
      /^Could not hide the dashboard: window\.hide not allowed\./,
    );
  });

  it("clears an earlier error when a retry succeeds", async () => {
    let fail = true;
    respond = (): null => {
      if (fail) {
        throw new Error("busy");
      }
      return null;
    };
    const root = render();
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')).not.toBeNull();

    fail = false;
    hideButton(root).click();
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    expect(calls).toHaveLength(2);
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
    const { default: mounted } = await import("./main");
    app = mounted;
    flushSync();
    expect(target.querySelector("h1")?.textContent).toBe("sotyware");
  });
});
