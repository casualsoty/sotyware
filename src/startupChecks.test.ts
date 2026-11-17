// Unit tests of the startup connection check (startupChecks.svelte.ts). The
// page-level behaviour, through the shell and the Cloud Storage page, is in
// CloudStorage.test.ts.
import { describe, expect, it } from "vitest";
import type { AccountInfo, CheckStatus } from "./cloud";
import { mergeCheck, needsStartupCheck, StartupChecks } from "./startupChecks.svelte";

function account(id: string, over: Partial<AccountInfo> = {}): AccountInfo {
  return {
    id,
    label: id,
    provider: "backblaze-b2",
    providerName: "Backblaze B2",
    region: "eu-central-003",
    endpoint: null,
    endpointUrl: null,
    bucket: `bucket-${id}`,
    configError: null,
    lastCheck: null,
    ...over,
  };
}

function checked(id: string, status: CheckStatus, checkedAt = 100): AccountInfo {
  return account(id, { lastCheck: { status, message: null, bucket: `bucket-${id}`, checkedAt } });
}

describe("needsStartupCheck", () => {
  it("checks only unchecked accounts with a bucket and usable settings", () => {
    expect(needsStartupCheck(account("a"))).toBe(true);
    expect(needsStartupCheck(checked("a", "connected"))).toBe(false);
    expect(needsStartupCheck(account("a", { bucket: null }))).toBe(false);
    expect(needsStartupCheck(account("a", { configError: "bad" }))).toBe(false);
  });
});

describe("mergeCheck", () => {
  it("puts a result in place and leaves the other accounts alone", () => {
    const list = [account("a"), account("b")];
    const next = mergeCheck(list, checked("a", "connected"));
    expect(next[0]?.lastCheck?.status).toBe("connected");
    expect(next[1]).toBe(list[1]);
  });

  it("keeps a newer check, ignores a dropped one and a vanished account", () => {
    const newer = checked("a", "network", 200);
    expect(mergeCheck([newer], checked("a", "connected", 100))[0]).toBe(newer);
    const plain = account("a");
    expect(mergeCheck([plain], account("a"))[0]).toBe(plain);
    expect(mergeCheck([plain], checked("gone", "connected"))).toEqual([plain]);
  });
});

describe("StartupChecks", () => {
  it("checks one account after another, once per start", async () => {
    const order: string[] = [];
    let running = 0;
    const checks = new StartupChecks(async (id) => {
      running += 1;
      expect(running).toBe(1);
      expect(checks.current).toBe(id);
      order.push(id);
      await Promise.resolve();
      running -= 1;
      return checked(id, "connected");
    });
    const got: string[] = [];
    await checks.run([account("a"), checked("b", "connected"), account("c")], (i) => got.push(i.id));
    expect(order).toEqual(["a", "c"]);
    expect(got).toEqual(["a", "c"]);
    expect(Object.keys(checks.results)).toEqual(["a", "c"]);
    expect(checks.current).toBeNull();

    await checks.run([account("d")], (i) => got.push(i.id));
    expect(order).toEqual(["a", "c"]);
  });

  it("stops after the credential store is locked or unavailable", async () => {
    for (const status of ["credential-store-locked", "credential-store-unavailable"] as const) {
      const order: string[] = [];
      const checks = new StartupChecks((id) => {
        order.push(id);
        return Promise.resolve(checked(id, status));
      });
      await checks.run([account("a"), account("b")], () => {});
      expect(order).toEqual(["a"]);
    }
  });

  it("goes on after other failures and after a check that threw", async () => {
    const order: string[] = [];
    const checks = new StartupChecks((id) => {
      order.push(id);
      return id === "a" ? Promise.reject(new Error("gone")) : Promise.resolve(checked(id, "invalid-credentials"));
    });
    await checks.run([account("a"), account("b"), account("c")], () => {});
    expect(order).toEqual(["a", "b", "c"]);
    expect(Object.keys(checks.results)).toEqual(["b", "c"]);
    expect(checks.current).toBeNull();
  });
});
