// The feedback buttons (ADR-049) with Tauri's IPC mock playing
// src-tauri/src/feedback.rs, which builds the URL and is tested there.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Feedback from "./Feedback.svelte";

let sent: { cmd: string; payload: unknown }[];
let fail: string | null;
let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  sent = [];
  fail = null;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    sent.push({ cmd, payload });
    if (fail !== null) {
      throw fail;
    }
    return null;
  });
  app = mount(Feedback, { target: document.body });
  flushSync();
});

afterEach(() => {
  if (app) {
    void unmount(app);
  }
  app = undefined;
  clearMocks();
  document.body.innerHTML = "";
});

function button(name: string): HTMLButtonElement {
  const b = [...document.querySelectorAll("button")].find((x) => x.textContent?.trim() === name);
  if (!b) {
    throw new Error(`no button ${name}`);
  }
  return b;
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
  flushSync();
}

describe("feedback buttons", () => {
  it("send only the kind of form, never an address", async () => {
    button("Send feedback").click();
    button("Report a bug").click();
    await settle();
    expect(sent).toEqual([
      { cmd: "open_feedback_page", payload: { kind: "feedback" } },
      { cmd: "open_feedback_page", payload: { kind: "bug" } },
    ]);
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  it("show why the browser could not be opened", async () => {
    fail = "The browser could not be opened. The page is https://github.com/casualsoty/sotyware/issues/new";
    button("Report a bug").click();
    await settle();
    expect(document.querySelector('[role="alert"]')?.textContent).toBe(fail);
  });
});
