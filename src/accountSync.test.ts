// Automatic sync of the sotyware account (accountSync.svelte.ts), headless
// with Tauri's IPC mocks: when it syncs, when it does not, and what the shell
// says. The sync itself (merge, encryption) is tested in Rust.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AccountStatus } from "./account";
import { AutoSync, FOCUS_INTERVAL_MS, canAutoSync, syncSummary } from "./accountSync.svelte";

let calls: string[];
let status: AccountStatus;
let syncFails: boolean;

function make(over: Partial<AccountStatus> = {}): AccountStatus {
  return {
    configured: true,
    service: "https://accounts.example.org",
    signedIn: true,
    email: "me@example.org",
    deviceId: "dev-1",
    vaultAccess: "enrolled",
    lastSyncMs: null,
    lastSyncError: null,
    conflicts: [],
    storageError: null,
    ...over,
  };
}

beforeEach(() => {
  calls = [];
  status = make();
  syncFails = false;
  mockWindows("main");
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "account_status") {
      return status;
    }
    if (cmd === "account_sync") {
      if (syncFails) {
        status = { ...status, lastSyncMs: 1, lastSyncError: "the account service could not be reached" };
        return Promise.reject({ kind: "network", message: "the account service could not be reached" });
      }
      status = { ...status, lastSyncMs: 1, lastSyncError: null };
      return null;
    }
    return null;
  });
});

afterEach(() => {
  clearMocks();
});

describe("automatic sync", () => {
  it("syncs an enrolled, signed-in device and reads the status again", async () => {
    const auto = new AutoSync();
    expect(await auto.run("startup")).toBe(true);
    expect(calls).toEqual(["account_status", "account_sync", "account_status"]);
    expect(auto.status?.lastSyncMs).toBe(1);
  });

  it("never syncs without a service, a sign-in or the account key", async () => {
    for (const s of [make({ configured: false }), make({ signedIn: false }), make({ vaultAccess: "pending" }), make({ vaultAccess: "none" })]) {
      status = s;
      calls = [];
      expect(await new AutoSync().run("startup")).toBe(false);
      expect(calls).toEqual(["account_status"]);
      expect(canAutoSync(s)).toBe(false);
    }
  });

  it("syncs on focus at most every 15 minutes, and after every local change", async () => {
    let now = 1_000_000;
    const auto = new AutoSync(() => now);
    await auto.run("startup");
    calls = [];
    now += FOCUS_INTERVAL_MS - 1;
    expect(await auto.run("focus")).toBe(false);
    expect(calls).toEqual([]);
    expect(await auto.run("change")).toBe(true);
    expect(calls).toEqual(["account_status", "account_sync", "account_status"]);
    now += 2;
    calls = [];
    expect(await auto.run("focus")).toBe(true);
    expect(calls).toContain("account_sync");
  });

  it("shares one run between overlapping requests", async () => {
    const auto = new AutoSync();
    const [a, b] = await Promise.all([auto.run("startup"), auto.run("change")]);
    expect([a, b]).toEqual([true, true]);
    expect(calls.filter((c) => c === "account_sync")).toHaveLength(1);
  });

  it("offline: the failure is recorded and shown, nothing throws", async () => {
    syncFails = true;
    const auto = new AutoSync();
    expect(await auto.run("startup")).toBe(true);
    expect(syncSummary(auto.status)).toEqual({
      tone: "warning",
      text: "Not synced: the account service could not be reached. Everything keeps working on this computer.",
    });
  });
});

describe("what the shell says about sync", () => {
  it("names each state, and nothing without an account", () => {
    const now = 10 * 60_000;
    expect(syncSummary(null)).toBeNull();
    expect(syncSummary(make({ configured: false }))).toBeNull();
    expect(syncSummary(make({ signedIn: false }))).toBeNull();
    expect(syncSummary(make({ vaultAccess: "pending" }))?.text).toBe("Sync is waiting: unlock this computer on the Account page.");
    expect(syncSummary(make({ vaultAccess: "none" }))?.text).toBe("Sync is not set up on this computer.");
    expect(syncSummary(make({ conflicts: [{ key: "conflict:x:d", of: "x", summary: "" }] }))?.text).toBe(
      "1 sync conflict to review on the Account page.",
    );
    expect(syncSummary(make({ lastSyncMs: now - 3 * 60_000 }), now)).toEqual({ tone: "success", text: "Synced 3 minutes ago." });
    expect(syncSummary(make({ lastSyncMs: now }), now)?.text).toBe("Synced just now.");
    expect(syncSummary(make())?.text).toBe("Not synced yet.");
  });
});
