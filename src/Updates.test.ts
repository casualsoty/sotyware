// Headless tests of the Updates panel (ADR-033) with Tauri's IPC mock
// playing src-tauri/src/updates.rs. The verification itself (signed
// manifest, version floor, package signature and SHA-256) is tested in
// src-tauri/src/updates/tests.rs.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Updates from "./Updates.svelte";
import { parseInstallEvent, parseUpdateStatus, type UpdateStatus } from "./updates";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

function status(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return {
    configured: true,
    current: "0.1.0",
    channel: "stable",
    lastCheckMs: null,
    available: null,
    postponed: false,
    error: null,
    blockedBy: [],
    ...over,
  };
}

const AVAILABLE = { version: "0.2.0", notes: "Faster uploads.", pubDate: "2026-11-15T12:00:00Z" };

let calls: string[];
let current: UpdateStatus;
let answers: Record<string, (p: Record<string, unknown>) => unknown>;
let channels: FakeChannel[];
let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  calls = [];
  channels = [];
  current = status();
  answers = {};
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push(cmd);
    if (answers[cmd]) {
      return answers[cmd]!(p);
    }
    switch (cmd) {
      case "update_status":
        return current;
      case "install_update":
        channels.push(p["onEvent"] as FakeChannel);
        return null;
      case "cancel_update":
        return true;
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
  for (let i = 0; i < 5; i++) {
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(Updates, { target });
  flushSync();
  await settle();
  return target;
}

function text(root: HTMLElement): string {
  return (root.textContent ?? "").replace(/\s+/g, " ");
}

function button(root: HTMLElement, label: string): HTMLButtonElement {
  const b = [...root.querySelectorAll("button")].find((x) => x.textContent?.trim() === label);
  if (!b) {
    throw new Error(`no "${label}" button in: ${root.innerHTML}`);
  }
  return b;
}

function hasButton(root: HTMLElement, label: string): boolean {
  return [...root.querySelectorAll("button")].some((x) => x.textContent?.trim() === label);
}

async function send(ch: FakeChannel, message: unknown): Promise<void> {
  ch.onmessage(message);
  await settle();
}

describe("Updates", () => {
  it("says honestly when this build has no update source", async () => {
    current = status({ configured: false });
    const root = await render();
    expect(text(root)).toContain("sotyware 0.1.0");
    expect(text(root)).toContain("Updates are not set up in this build");
    expect(hasButton(root, "Check for updates")).toBe(false);
    expect(calls).toEqual(["update_status"]);
  });

  it("checks on request and reports an up-to-date installation", async () => {
    const root = await render();
    expect(text(root)).toContain("Not checked yet.");
    answers["check_for_update"] = () => status({ lastCheckMs: 1_700_000_000_000 });
    button(root, "Check for updates").click();
    await settle();
    expect(calls).toContain("check_for_update");
    expect(text(root)).toContain("sotyware is up to date.");
    expect(text(root)).toContain("Last checked");
  });

  it("shows a failed check as an error without offering anything", async () => {
    const root = await render();
    answers["check_for_update"] = () => {
      throw { kind: "integrity", message: "the update information is not signed by sotyware's release key" };
    };
    current = status({ lastCheckMs: 5, error: "the update information is not signed by sotyware's release key" });
    button(root, "Check for updates").click();
    await settle();
    const alerts = [...root.querySelectorAll('[role="alert"]')].map((a) => a.textContent ?? "");
    expect(alerts.some((a) => a.includes("not signed by sotyware's release key"))).toBe(true);
    expect(hasButton(root, "Update now")).toBe(false);
  });

  it("installs only on click, with progress, and can be cancelled", async () => {
    current = status({ lastCheckMs: 5, available: AVAILABLE });
    const root = await render();
    expect(text(root)).toContain("sotyware 0.2.0 is available.");
    expect(text(root)).toContain("Faster uploads.");
    // Nothing was started by showing it.
    expect(calls).toEqual(["update_status"]);
    button(root, "Update now").click();
    await settle();
    expect(calls).toContain("install_update");
    const ch = channels[0]!;
    await send(ch, { type: "progress", downloaded: 1048576, total: 4194304 });
    expect(text(root)).toContain("Downloading… 1.0 MiB of 4.0 MiB");
    expect(button(root, "Check for updates").disabled).toBe(true);
    button(root, "Cancel").click();
    await settle();
    expect(calls).toContain("cancel_update");
    await send(ch, { type: "cancelled" });
    expect(text(root)).toContain("The download was cancelled; nothing was installed.");
    expect(hasButton(root, "Update now")).toBe(true);
  });

  it("reports verification, installation and failures from Rust", async () => {
    current = status({ lastCheckMs: 5, available: AVAILABLE });
    const root = await render();
    button(root, "Update now").click();
    await settle();
    const ch = channels[0]!;
    await send(ch, { type: "failed", message: "the downloaded update does not match its signed description: SHA-256 x, expected y" });
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("nothing was installed");
    button(root, "Try again").click();
    await settle();
    const again = channels[1]!;
    await send(again, { type: "verified" });
    expect(text(root)).toContain("downloaded and verified");
    await send(again, { type: "installing" });
    expect(text(root)).toContain("sotyware closes and the installer starts the new version");
    await send(again, { type: "installed" });
    expect(text(root)).toContain("takes effect when sotyware starts again");
  });

  it("does not offer to update while a transfer or backup runs", async () => {
    current = status({ lastCheckMs: 5, available: AVAILABLE, blockedBy: ["an upload", "a backup task"] });
    const root = await render();
    expect(button(root, "Update now").disabled).toBe(true);
    expect(text(root)).toContain("Wait until an upload, a backup task has finished.");
    // Refused by Rust too (a transfer started after the status was read).
    unmount(app!);
    app = undefined;
    document.body.innerHTML = "";
    current = status({ lastCheckMs: 5, available: AVAILABLE });
    const fresh = await render();
    answers["install_update"] = () => {
      throw { kind: "busy", message: "wait until an upload has finished, or cancel it, then update" };
    };
    button(fresh, "Update now").click();
    await settle();
    expect(fresh.querySelector('[role="alert"]')?.textContent).toContain("wait until an upload has finished");
  });

  it("postpones a version on request", async () => {
    current = status({ lastCheckMs: 5, available: AVAILABLE });
    const root = await render();
    answers["postpone_update"] = () => {
      current = status({ lastCheckMs: 5, available: AVAILABLE, postponed: true });
      return null;
    };
    button(root, "Later").click();
    await settle();
    expect(calls).toContain("postpone_update");
    expect(text(root)).toContain("(postponed)");
    expect(hasButton(root, "Later")).toBe(false);
    expect(hasButton(root, "Update now")).toBe(true);
  });

  it("rejects malformed answers", () => {
    expect(() => parseUpdateStatus(null)).toThrow();
    expect(() => parseUpdateStatus({ ...status(), blockedBy: [1] })).toThrow();
    expect(() => parseUpdateStatus({ ...status(), available: { version: 2 } })).toThrow();
    expect(parseUpdateStatus(status({ available: AVAILABLE })).available).toEqual(AVAILABLE);
    expect(() => parseInstallEvent({ type: "progress", downloaded: "x", total: null })).toThrow();
    expect(() => parseInstallEvent({ type: "other" })).toThrow();
    expect(parseInstallEvent({ type: "failed", message: "m" })).toEqual({ type: "failed", message: "m" });
  });
});
