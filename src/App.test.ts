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
const ACCOUNTS_CMD = "list_cloud_accounts"; // the sidebar's storage accounts; tested in CloudStorage.test.ts
const BACKUPS_CMD = "list_backups"; // the Overview's backup summary; tested in Backup.test.ts
const DECORATED_CMD = "plugin:window|is_decorated"; // whether the page draws its own title bar (ADR-040)
const ACCOUNT_CMD = "account_status"; // automatic sync reads the account status (local); tested in accountSync.test.ts

let calls: IpcCall[];
let mode: unknown;
let respond: (cmd: string) => unknown;
let app: ReturnType<typeof mount> | undefined;

/** The update status `update_status` answers (Updates.test.ts covers the rest). */
let update: Record<string, unknown>;

function updateStatus(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    configured: false,
    current: "0.1.0",
    channel: "stable",
    lastCheckMs: null,
    available: null,
    postponed: false,
    error: null,
    errorKind: null,
    blockedBy: [],
    installed: null,
    usesInstaller: true,
    ...over,
  };
}

/** The storage accounts the sidebar lists. */
let accountList: Record<string, unknown>[];

/** What the window says about its native decorations (null: the default
 * answer of the mock, which keeps the native title bar). */
let decorated: boolean | null;

/** The Rust side: answers the mode command with `mode`, the feature list with no
 * features (Features.test.ts covers them), the accounts, backups and update
 * status the shell reads, everything else with null. */
function backend(cmd: string): unknown {
  switch (cmd) {
    case LIST_CMD:
      return { features: [], storageError: null };
    case UPDATE_CMD:
      return update;
    case ACCOUNTS_CMD:
      return { accounts: accountList, storageError: null };
    case BACKUPS_CMD:
      return { storageError: null, jobs: [], active: null };
    case "list_cloud_providers":
      return [];
    case "list_cloud_recovery":
      return { entries: [], storageError: null };
    case MODE_CMD:
      return mode;
    case DECORATED_CMD:
      return decorated;
    default:
      return null;
  }
}

/** The commands sent so far, in a stable order (the page and its Features
 * section ask on mount independently). */
function sentCommands(): string[] {
  return calls.map((c) => c.cmd).sort();
}

beforeEach(() => {
  calls = [];
  update = updateStatus();
  accountList = [];
  decorated = null;
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
  it("asks Rust for the mode, the accounts, the backups, the update and account status once each, and nothing else", async () => {
    await render();
    expect(sentCommands()).toEqual([ACCOUNT_CMD, BACKUPS_CMD, ACCOUNTS_CMD, MODE_CMD, DECORATED_CMD, UPDATE_CMD]);
  });

  it("renders the heading, the tray hint and a plain button, no error", async () => {
    const root = await render();
    expect(root.querySelector("main h1")?.textContent).toBe("Overview");
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
    // The page header holds the Hide button in tray mode, and nothing here.
    expect(root.querySelectorAll("main .page-header button")).toHaveLength(0);
    expect(root.textContent).not.toContain("from the system tray");
    expect(sentCommands()).toEqual([ACCOUNT_CMD, BACKUPS_CMD, ACCOUNTS_CMD, MODE_CMD, DECORATED_CMD, UPDATE_CMD]);
    expect(hideCalls()).toHaveLength(0);
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

describe("update notification", () => {
  const AVAILABLE = { version: "0.2.0", notes: "Faster uploads.", pubDate: "2026-12-01T12:00:00Z" };

  function banner(root: HTMLElement): HTMLElement | null {
    return root.querySelector<HTMLElement>('[role="region"][aria-label="Update notification"]');
  }

  function bannerButton(root: HTMLElement, name: string): HTMLButtonElement {
    const b = [...(banner(root)?.querySelectorAll("button") ?? [])].find(
      (x) => x.textContent?.trim() === name || x.getAttribute("aria-label") === name,
    );
    if (!b) {
      throw new Error(`no "${name}" in the notification: ${banner(root)?.innerHTML ?? "(none)"}`);
    }
    return b;
  }

  it("is not shown when sotyware is up to date or updates are not set up", async () => {
    const root = await render();
    expect(banner(root)).toBeNull();
    unmount(app!);
    app = undefined;
    update = updateStatus({ configured: true, lastCheckMs: 5 });
    const again = await render();
    expect(banner(again)).toBeNull();
  });

  it("shows an available version once, and installs only on its Update button", async () => {
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE });
    const root = await render();
    expect(banner(root)?.textContent).toContain("sotyware 0.2.0 is available.");
    expect(root.querySelectorAll('[aria-label="Update notification"]')).toHaveLength(1);
    expect(calls.map((c) => c.cmd)).not.toContain("install_update");
    bannerButton(root, "Update").click();
    await settle();
    expect(calls.map((c) => c.cmd)).toContain("install_update");
  });

  it("does not offer Update while a transfer runs", async () => {
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE, blockedBy: ["an upload"] });
    const root = await render();
    expect(bannerButton(root, "Update").disabled).toBe(true);
    expect(banner(root)?.textContent).toContain("Update when an upload has finished.");
  });

  it("goes away for that version with Later, and for this session when closed", async () => {
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE });
    respond = (cmd): unknown => {
      if (cmd === "postpone_update") {
        update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE, postponed: true });
        return null;
      }
      return backend(cmd);
    };
    const root = await render();
    bannerButton(root, "Later").click();
    await settle();
    expect(calls.map((c) => c.cmd)).toContain("postpone_update");
    expect(banner(root)).toBeNull();

    unmount(app!);
    app = undefined;
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE });
    respond = backend;
    const again = await render();
    bannerButton(again, "Close the update notification").click();
    await settle();
    expect(banner(again)).toBeNull();
    expect(calls.filter((c) => c.cmd === "postpone_update")).toHaveLength(1);
  });

  it("appears when the window is shown after an automatic check found a version", async () => {
    update = updateStatus({ configured: true, lastCheckMs: null });
    const root = await render();
    expect(banner(root)).toBeNull();
    // The update-check feature found 0.2.0 in Rust; no event is sent.
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE });
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(banner(root)?.textContent).toContain("sotyware 0.2.0 is available.");
    expect(calls.filter((c) => c.cmd === UPDATE_CMD)).toHaveLength(2);
  });

  it("offers a restart once an update is installed", async () => {
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE, installed: "0.2.0", usesInstaller: false });
    const root = await render();
    expect(banner(root)?.textContent).toContain("sotyware 0.2.0 is installed.");
    bannerButton(root, "Restart now").click();
    await settle();
    expect(calls.map((c) => c.cmd)).toContain("restart_after_update");
  });

  it("leads to Settings › About & updates with Details", async () => {
    update = updateStatus({ configured: true, lastCheckMs: 5, available: AVAILABLE });
    const root = await render();
    bannerButton(root, "Details").click();
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Settings");
    const tab = root.querySelector('[role="tab"][aria-selected="true"]');
    expect(tab?.textContent?.trim()).toBe("About & updates");
    expect(root.querySelector("#updates-heading")).not.toBeNull();
    // The panel shows the same offer; the banner is not repeated above it.
    expect(banner(root)).toBeNull();
  });
});

describe("shell navigation", () => {
  function key(init: KeyboardEventInit): void {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  }

  it("switches sections with Ctrl or Cmd and a number, and opens Settings with comma", async () => {
    const root = await render();
    key({ key: "3", ctrlKey: true });
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Backups");
    key({ key: ",", metaKey: true });
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Settings");
    key({ key: "2", ctrlKey: true });
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Cloud Storage");
    // Unmodified keys belong to the page.
    key({ key: "1" });
    await settle();
    expect(root.querySelector("main h1")?.textContent).toBe("Cloud Storage");
  });

  it("moves focus to the new page's title", async () => {
    const root = await render();
    [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.trim() === "Backups")!.click();
    await settle();
    await settle();
    expect(document.activeElement).toBe(root.querySelector("main h1"));
  });

  it("applies and remembers the theme chosen in Settings", async () => {
    const root = await render();
    [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.trim() === "Settings")!.click();
    await settle();
    const dark = root.querySelector<HTMLInputElement>('input[name="theme"][value="dark"]')!;
    dark.checked = true;
    dark.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(document.documentElement.dataset["theme"]).toBe("dark");
    expect(localStorage.getItem("sotyware.theme.v1")).toBe("dark");
    expect(calls.some((c) => c.cmd === "plugin:window|set_theme")).toBe(true);
    const system = root.querySelector<HTMLInputElement>('input[name="theme"][value="system"]')!;
    system.checked = true;
    system.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(document.documentElement.dataset["theme"]).toBeUndefined();
    expect(localStorage.getItem("sotyware.theme.v1")).toBeNull();
  });

  // Narrow windows: the sidebar is an overlay; every choice in it closes it
  // (review of 2e3972e: "Add storage account" left it open).
  it("closes the sidebar overlay after a link and after Add storage account", async () => {
    const root = await render();
    const shell = root.querySelector<HTMLElement>(".shell")!;
    const toggle = (): HTMLButtonElement => root.querySelector<HTMLButtonElement>('button[aria-label$="navigation"]')!;
    const open = async (): Promise<void> => {
      toggle().click();
      await settle();
      expect(shell.classList.contains("sidebar-open")).toBe(true);
      expect(toggle().getAttribute("aria-expanded")).toBe("true");
      expect(root.querySelector(".scrim")).not.toBeNull();
    };
    await open();
    [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.trim() === "Backups")!.click();
    await settle();
    expect(shell.classList.contains("sidebar-open")).toBe(false);
    expect(root.querySelector("main h1")?.textContent).toBe("Backups");
    await open();
    [...root.querySelectorAll<HTMLButtonElement>("aside button")].find((b) => b.textContent?.trim() === "Add storage account")!.click();
    await settle();
    await settle();
    expect(shell.classList.contains("sidebar-open")).toBe(false);
    expect(toggle().getAttribute("aria-expanded")).toBe("false");
    expect(root.querySelector(".scrim")).toBeNull();
    expect(root.querySelector("main h1")?.textContent).toBe("Cloud Storage");
  });

  it("walks the Settings tabs with the arrow keys", async () => {
    const root = await render();
    [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.trim() === "Settings")!.click();
    await settle();
    const list = root.querySelector<HTMLElement>('[role="tablist"]')!;
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await settle();
    expect(root.querySelector('[role="tab"][aria-selected="true"]')?.textContent?.trim()).toBe("Features");
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await settle();
    expect(root.querySelector('[role="tab"][aria-selected="true"]')?.textContent?.trim()).toBe("About & updates");
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await settle();
    expect(root.querySelector('[role="tab"][aria-selected="true"]')?.textContent?.trim()).toBe("General");
  });
});

describe("the page's own title bar (ADR-040)", () => {
  it("is not drawn while the window has its native title bar", async () => {
    decorated = true;
    const root = await render();
    expect(root.querySelector("[data-titlebar]")).toBeNull();
    expect(root.querySelector(".shell")?.classList.contains("custom-chrome")).toBe(false);
    // The name stays at the top of the sidebar.
    expect(root.querySelector("aside .brand")?.textContent?.trim()).toBe("sotyware");
  });

  it("is not drawn when the window cannot be asked", async () => {
    respond = (cmd) => (cmd === DECORATED_CMD ? Promise.reject("no window") : backend(cmd));
    const root = await render();
    expect(root.querySelector("[data-titlebar]")).toBeNull();
  });

  it("is drawn across the top for an undecorated window, with the name moved into it", async () => {
    decorated = false;
    const root = await render();
    const shell = root.querySelector<HTMLElement>(".shell")!;
    expect(shell.classList.contains("custom-chrome")).toBe(true);
    const bar = shell.querySelector<HTMLElement>(":scope > [data-titlebar]")!;
    expect(bar).not.toBeNull();
    // First in the window, before the navigation and the page.
    expect(shell.firstElementChild).toBe(bar);
    expect(bar.textContent).toContain("sotyware");
    expect(root.querySelector("aside .brand")).toBeNull();
    expect([...bar.querySelectorAll('[role="group"][aria-label="Window"] button')].map((b) => b.getAttribute("aria-label"))).toEqual([
      "Minimize",
      "Maximize",
      "Close",
    ]);
    // The page's name for narrow windows follows the section.
    expect(bar.querySelector(".page-title")?.textContent).toBe("Overview");
    [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.trim() === "Backups")!.click();
    await settle();
    expect(bar.querySelector(".page-title")?.textContent).toBe("Backups");
  });

  it("opens and closes the navigation overlay from its toggle", async () => {
    decorated = false;
    const root = await render();
    const shell = root.querySelector<HTMLElement>(".shell")!;
    const toggle = root.querySelector<HTMLButtonElement>("[data-titlebar] .nav-toggle")!;
    toggle.click();
    await settle();
    expect(shell.classList.contains("sidebar-open")).toBe(true);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    toggle.click();
    await settle();
    expect(shell.classList.contains("sidebar-open")).toBe(false);
  });

  it("closes the window through the same close request as Alt+F4 (Rust hides it in tray mode)", async () => {
    decorated = false;
    const root = await render();
    root.querySelector<HTMLButtonElement>('[data-titlebar] button[aria-label="Close"]')!.click();
    await settle();
    expect(calls.filter((c) => c.cmd === "plugin:window|close")).toHaveLength(1);
    expect(hideCalls()).toHaveLength(0);
  });
});

describe("automatic sync of the sotyware account", () => {
  it("syncs an enrolled device at startup, reads the accounts again and shows the state by Account", async () => {
    let synced = false;
    const status = (lastSyncMs: number | null) => ({
      configured: true,
      service: "https://accounts.example.org",
      signedIn: true,
      email: "me@example.org",
      deviceId: "d1",
      vaultAccess: "enrolled",
      lastSyncMs,
      lastSyncError: null,
      conflicts: [],
      storageError: null,
    });
    respond = (cmd) => {
      if (cmd === ACCOUNT_CMD) {
        return status(synced ? Date.now() : null);
      }
      if (cmd === "account_sync") {
        synced = true;
        return null;
      }
      return backend(cmd);
    };
    const root = await render();
    await settle();
    expect(calls.filter((c) => c.cmd === "account_sync")).toHaveLength(1);
    // The sidebar's accounts were read again after the sync.
    expect(calls.filter((c) => c.cmd === ACCOUNTS_CMD).length).toBeGreaterThanOrEqual(2);
    const link = [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.includes("Account"))!;
    expect(link.querySelector("[data-sync]")?.getAttribute("data-sync")).toBe("success");
    expect(link.title).toBe("Synced just now.");
    expect(link.textContent).toContain("Synced just now.");
  });

  it("shows nothing by Account without a sotyware account", async () => {
    const root = await render();
    const link = [...root.querySelectorAll<HTMLAnchorElement>("aside a")].find((a) => a.textContent?.includes("Account"))!;
    expect(link.querySelector("[data-sync]")).toBeNull();
    expect(calls.some((c) => c.cmd === "account_sync")).toBe(false);
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
      expect(target.querySelector("main h1")?.textContent).toBe("Overview");
    } finally {
      svelte.unmount(mounted);
    }
  });
});
