// The page's own title bar (ADR-040), headless: happy-dom and Tauri's IPC
// mocks, so these tests check what the bar asks the window to do, not that a
// real window moves. The native behaviour (dragging, snapping, resizing) is
// in the manual procedure in docs/development/testing.md.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import TitleBar from "./TitleBar.svelte";

interface IpcCall {
  cmd: string;
  payload: unknown;
}

let calls: IpcCall[];
let maximized: boolean;
let fail: string | null;
let toggled: number;
let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  calls = [];
  maximized = false;
  fail = null;
  toggled = 0;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload });
    if (fail !== null && cmd === fail) {
      return Promise.reject("not allowed");
    }
    switch (cmd) {
      case "plugin:window|is_maximized":
        return maximized;
      case "plugin:window|toggle_maximize":
        maximized = !maximized;
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
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

async function render(props: Partial<{ title: string; mode: "tray" | "dashboard-only" | null; sidebarOpen: boolean }> = {}): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(TitleBar, {
    target,
    props: {
      title: props.title ?? "Cloud Storage",
      mode: props.mode === undefined ? "tray" : props.mode,
      sidebarOpen: props.sidebarOpen ?? false,
      ontogglesidebar: () => (toggled += 1),
    },
  });
  flushSync();
  await settle();
  return target;
}

function control(root: HTMLElement, name: string): HTMLButtonElement {
  const b = root.querySelector<HTMLButtonElement>(`[role="group"][aria-label="Window"] button[aria-label="${name}"]`);
  if (!b) {
    throw new Error(`no ${name} button in ${root.innerHTML}`);
  }
  return b;
}

function sent(cmd: string): number {
  return calls.filter((c) => c.cmd === cmd).length;
}

describe("the title bar", () => {
  it("shows sotyware's name and the three window controls, out of the tab order", async () => {
    const root = await render();
    const bar = root.querySelector<HTMLElement>("[data-titlebar]")!;
    expect(bar.textContent).toContain("sotyware");
    const names = [...bar.querySelectorAll('[role="group"] button')].map((b) => b.getAttribute("aria-label"));
    expect(names).toEqual(["Minimize", "Maximize", "Close"]);
    for (const b of bar.querySelectorAll('[role="group"] button')) {
      expect(b.getAttribute("tabindex")).toBe("-1");
      expect(b.getAttribute("type")).toBe("button");
    }
    // It asked once whether the window is maximized, for the middle button.
    expect(sent("plugin:window|is_maximized")).toBe(1);
  });

  it("is a drag region everywhere except on its buttons", async () => {
    const root = await render();
    const bar = root.querySelector<HTMLElement>("[data-titlebar]")!;
    // Tauri's drag script: "deep" makes the whole subtree draggable; its
    // buttons block dragging because they carry no drag attribute.
    expect(bar.getAttribute("data-tauri-drag-region")).toBe("deep");
    expect(bar.querySelectorAll("button[data-tauri-drag-region], a[data-tauri-drag-region]")).toHaveLength(0);
  });

  it("minimizes, maximizes, restores and closes through the window API", async () => {
    const root = await render();
    control(root, "Minimize").click();
    await settle();
    expect(sent("plugin:window|minimize")).toBe(1);

    control(root, "Maximize").click();
    await settle();
    expect(sent("plugin:window|toggle_maximize")).toBe(1);
    // The middle button now restores.
    expect(control(root, "Restore").title).toBe("Restore Down");
    control(root, "Restore").click();
    await settle();
    expect(sent("plugin:window|toggle_maximize")).toBe(2);
    expect(control(root, "Maximize").title).toBe("Maximize");

    control(root, "Close").click();
    await settle();
    expect(sent("plugin:window|close")).toBe(1);
    // Never anything that would move, resize or restyle the window itself.
    expect(calls.map((c) => c.cmd).filter((c) => /set_|start_dragging/.test(c))).toEqual([]);
    expect(root.querySelector('[role="alert"]')).toBeNull();
  });

  it("follows a maximize that happened elsewhere (double click, Win+Up)", async () => {
    const root = await render();
    expect(control(root, "Maximize")).toBeTruthy();
    maximized = true;
    window.dispatchEvent(new Event("resize"));
    await settle();
    expect(control(root, "Restore")).toBeTruthy();
  });

  it("says that closing keeps sotyware in the tray, only in tray mode", async () => {
    let root = await render({ mode: "tray" });
    expect(control(root, "Close").title).toBe("Close (sotyware keeps running in the system tray)");
    unmount(app!);
    app = undefined;
    document.body.innerHTML = "";
    root = await render({ mode: "dashboard-only" });
    expect(control(root, "Close").title).toBe("Close");
  });

  it("shows a refused window action instead of failing silently", async () => {
    const root = await render();
    fail = "plugin:window|minimize";
    control(root, "Minimize").click();
    await settle();
    expect(root.querySelector('[role="alert"]')?.textContent).toBe("Could not minimize the window: not allowed");
    fail = null;
    control(root, "Minimize").click();
    await settle();
    expect(root.querySelector('[role="alert"]')).toBeNull();
    fail = "plugin:window|close";
    control(root, "Close").click();
    await settle();
    expect(root.querySelector('[role="alert"]')?.textContent).toBe("Could not close the window: not allowed");
  });

  it("names the page and toggles the navigation for narrow windows", async () => {
    const root = await render({ title: "Backups", sidebarOpen: false });
    expect(root.querySelector(".page-title")?.textContent).toBe("Backups");
    const nav = root.querySelector<HTMLButtonElement>(".nav-toggle")!;
    expect(nav.getAttribute("aria-label")).toBe("Show navigation");
    expect(nav.getAttribute("aria-expanded")).toBe("false");
    nav.click();
    expect(toggled).toBe(1);
  });

  it("looks inactive while another window has the focus", async () => {
    const root = await render();
    const bar = root.querySelector<HTMLElement>("[data-titlebar]")!;
    window.dispatchEvent(new Event("blur"));
    flushSync();
    expect(bar.classList.contains("inactive")).toBe(true);
    window.dispatchEvent(new Event("focus"));
    flushSync();
    expect(bar.classList.contains("inactive")).toBe(false);
  });
});
