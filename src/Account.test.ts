// Headless tests of the Account page (ADR-032) with Tauri's IPC mock playing
// src-tauri/src/account/mod.rs. The real flows (crypto, service, two
// devices) are tested end to end in src-tauri/src/account/tests.rs.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Account from "./Account.svelte";
import { parseStatus, type AccountStatus } from "./account";

const KEY = "ABCD-EFGH-JKMN-PQRS-TVWX-YZ01-2345-6789-ABCD-EFGH-JKMN-PQRS-TVWX-YZ01";

function status(over: Partial<AccountStatus> = {}): AccountStatus {
  return {
    configured: true,
    service: "https://accounts.example.org",
    signedIn: false,
    email: null,
    deviceId: null,
    vaultAccess: "unknown",
    lastSyncMs: null,
    lastSyncError: null,
    conflicts: [],
    storageError: null,
    ...over,
  };
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let current: AccountStatus;
let answers: Record<string, (p: Record<string, unknown>) => unknown>;
let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  calls = [];
  current = status();
  answers = {};
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    if (answers[cmd]) {
      return answers[cmd]!(p);
    }
    switch (cmd) {
      case "account_status":
        return current;
      case "account_devices":
        return [
          { id: "d1", name: "Desktop", platform: "windows", state: "enrolled", createdMs: 1, lastSeenMs: 2, current: true },
          { id: "d2", name: "Old laptop", platform: "windows", state: "enrolled", createdMs: 1, lastSeenMs: 2, current: false },
        ];
      case "account_enrollments":
        return [{ id: "r1", deviceName: "New PC", platform: "windows", createdMs: 3, fingerprint: "12345 67890 12345" }];
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
  app = mount(Account, { target });
  flushSync();
  await settle();
  return target;
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ");
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const all = [...root.querySelectorAll<HTMLButtonElement>("button")];
  const found = all.find((b) => b.textContent?.trim() === name);
  if (!found) {
    throw new Error(`no button "${name}" in: ${all.map((b) => b.textContent?.trim()).join(", ")}`);
  }
  return found;
}

async function click(root: ParentNode, name: string): Promise<void> {
  button(root, name).click();
  flushSync();
  await settle();
}

function type(root: ParentNode, id: string, value: string): void {
  const el = root.querySelector<HTMLInputElement>(`#${id}`)!;
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

async function submit(root: ParentNode): Promise<void> {
  root.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  flushSync();
  await settle();
}

describe("the Account page", () => {
  it("says when this build has no account service, and offers nothing to send", async () => {
    current = status({ configured: false, service: null });
    const root = await render();
    expect(text(root.querySelector("[data-account=not-configured]"))).toContain("not available in this build");
    expect(root.querySelector("form")).toBeNull();
    expect(text(root)).toContain("it never unlocks your keys");
  });

  it("creates an account, verifies the address and signs in", async () => {
    const root = await render();
    await click(root, "Create account");
    type(root, "ac-email", "me@example.org");
    type(root, "ac-password", "a long enough passphrase");
    await submit(root);
    expect(calls.find((c) => c.cmd === "account_register")!.payload).toEqual({ email: "me@example.org", password: "a long enough passphrase" });
    expect(text(root.querySelector("[role=status]"))).toContain("If this address can be registered");
    type(root, "ac-code", "12345678");
    answers["account_sign_in"] = () => {
      current = status({ signedIn: true, email: "me@example.org", vaultAccess: "none" });
      return "none";
    };
    await submit(root);
    expect(calls.find((c) => c.cmd === "account_verify")!.payload).toEqual({ email: "me@example.org", code: "12345678", password: "a long enough passphrase" });
    expect(calls.find((c) => c.cmd === "account_sign_in")!.payload).toMatchObject({ email: "me@example.org", deviceName: "This computer" });
    expect(root.querySelector<HTMLElement>("[data-vault]")!.dataset["vault"]).toBe("none");
    expect(button(root, "Set up sync")).toBeTruthy();
  });

  it("shows the recovery key once and keeps it until it is typed back correctly", async () => {
    current = status({ signedIn: true, email: "me@example.org", vaultAccess: "none" });
    answers["account_set_up_sync"] = () => {
      current = status({ signedIn: true, email: "me@example.org", vaultAccess: "enrolled" });
      return KEY;
    };
    answers["account_check_recovery_key"] = (p) => p["key"] === KEY.replace(/-/g, "");
    const root = await render();
    await click(root, "Set up sync");
    expect(text(root.querySelector("[data-recovery-key]"))).toBe(KEY);
    expect(text(root.querySelector(".recovery-key"))).toContain("shown only this once");
    type(root, "ac-retype", "ABCD-WRONG");
    await click(root, "I saved it");
    expect(text(root.querySelector("[role=alert]"))).toContain("does not match");
    expect(root.querySelector("[data-recovery-key]")).not.toBeNull();
    type(root, "ac-retype", KEY.toLowerCase().replace(/-/g, " "));
    await click(root, "I saved it");
    expect(root.querySelector("[data-recovery-key]")).toBeNull();
    expect(text(root.querySelector("[role=status]"))).toContain("recovery key is confirmed");
  });

  it("unlocks a new computer with the recovery key or by approval with a fingerprint", async () => {
    current = status({ signedIn: true, email: "me@example.org", vaultAccess: "pending" });
    answers["account_request_approval"] = () => ({ id: "r9", fingerprint: "11111 22222 33333" });
    answers["account_approval_status"] = () => "pending";
    const root = await render();
    expect(text(root)).toContain("cannot read your synced keys yet");
    type(root, "ac-unlock", KEY);
    await click(root, "Unlock");
    expect(calls.find((c) => c.cmd === "account_unlock")!.payload).toEqual({ key: KEY });
    await click(root, "Ask another computer");
    expect(text(root.querySelector("[data-fingerprint]"))).toBe("11111 22222 33333");
    await click(root, "I approved it");
    expect(calls.find((c) => c.cmd === "account_approval_status")!.payload).toEqual({ id: "r9" });
    expect(text(root.querySelector("[role=status]"))).toContain("Not approved yet");
  });

  it("lists computers and requests, and approves only after the digits warning", async () => {
    current = status({ signedIn: true, email: "me@example.org", vaultAccess: "enrolled", lastSyncMs: Date.UTC(2026, 10, 15) });
    const root = await render();
    const devices = [...root.querySelectorAll<HTMLElement>("li[data-device]")];
    expect(devices.map((d) => d.dataset["device"])).toEqual(["d1", "d2"]);
    expect(text(devices[0])).toContain("this computer");
    const req = root.querySelector<HTMLElement>("li[data-request=r1]")!;
    expect(text(req)).toContain("12345 67890 12345");
    await click(req, "Approve…");
    expect(text(root.querySelector("li[data-request=r1] .confirm"))).toContain("only if the new computer shows exactly these digits");
    await click(root.querySelector("li[data-request=r1]")!, "Approve");
    expect(calls.find((c) => c.cmd === "account_answer_enrollment")!.payload).toEqual({ id: "r1", approve: true });
    // Revoking asks first and then suggests a key rotation.
    await click(root.querySelector("li[data-device=d2]")!, "Revoke…");
    expect(text(root.querySelector("li[data-device=d2] .confirm"))).toContain("keys it already read stay on it");
    await click(root.querySelector("li[data-device=d2]")!, "Revoke");
    expect(calls.find((c) => c.cmd === "account_revoke_device")!.payload).toEqual({ id: "d2" });
    expect(text(root.querySelector("[role=status]"))).toContain("rotate the keys now");
    expect(button(root, "Rotate keys now")).toBeTruthy();
  });

  it("lets the user choose between two versions changed on two computers", async () => {
    current = status({
      signedIn: true,
      email: "me@example.org",
      vaultAccess: "enrolled",
      conflicts: [{ key: "conflict:storage-account:photos:dev2", of: "storage-account:photos", summary: 'Storage account "Photos (laptop)"' }],
    });
    const root = await render();
    const c = root.querySelector<HTMLElement>("li[data-conflict]")!;
    expect(text(c)).toContain('Storage account "Photos (laptop)" was also changed elsewhere');
    await click(c, "Use this version");
    expect(calls.find((x) => x.cmd === "account_resolve_conflict")!.payload).toEqual({ key: "conflict:storage-account:photos:dev2", keepLocal: true });
  });

  it("deletes the account only after a confirmation with the password", async () => {
    current = status({ signedIn: true, email: "me@example.org", vaultAccess: "enrolled" });
    const root = await render();
    await click(root, "Delete account…");
    expect(text(root.querySelector(".actions .confirm"))).toContain("storage accounts on your computers stay");
    expect(button(root, "Delete account").disabled).toBe(true);
    type(root, "ac-delete", "a long enough passphrase");
    await click(root, "Delete account");
    expect(calls.find((x) => x.cmd === "account_delete")!.payload).toEqual({ password: "a long enough passphrase" });
  });
});

describe("the account wrappers", () => {
  it("parse the status and refuse anything else", () => {
    expect(parseStatus(status()).configured).toBe(true);
    expect(() => parseStatus({ ...status(), vaultAccess: "maybe" })).toThrow("invalid account status");
    expect(() => parseStatus({ ...status(), conflicts: [{ key: 1 }] })).toThrow("invalid conflict");
  });
});
