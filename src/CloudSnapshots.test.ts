// The bucket view's "Snapshots & backups" tab (ADR-041), headless with
// Tauri's IPC mocks: what it explains about the provider's own snapshots,
// and what it asks Rust to do with sotyware's backup snapshots.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AccountInfo } from "./cloud";
import CloudSnapshots from "./CloudSnapshots.svelte";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let channels: FakeChannel[];
let jobs: Record<string, unknown>[];
let snapshots: Record<string, unknown>[];
let opened: number;
let app: ReturnType<typeof mount> | undefined;

function account(over: Partial<AccountInfo> = {}): AccountInfo {
  return {
    id: "b2-photos",
    label: "Photos",
    provider: "backblaze-b2",
    providerName: "Backblaze B2",
    region: "eu-central-003",
    endpoint: null,
    endpointUrl: "https://s3.eu-central-003.backblazeb2.com",
    bucket: "soty-photos",
    configError: null,
    lastCheck: null,
    ...over,
  };
}

function job(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    config: {
      id: "photos",
      name: "Photos backup",
      source: { account: "b2-photos", bucket: "soty-photos", prefix: "" },
      destination: { account: "vault", bucket: "vault-bucket", prefix: "sotyware/" },
      intervalHours: 24,
      retention: { keepLast: 0, keepDaily: 0, keepWeekly: 0 },
      suspiciousPercent: 30,
    },
    health: "healthy",
    sameAccount: false,
    lastBackup: null,
    lastVerification: null,
    lastPrune: null,
    lastSnapshot: "sotyware/snapshots/photos/20261115T103000Z-b.json",
    nextDueMs: null,
    ...over,
  };
}

const OLD = "sotyware/snapshots/photos/20261114T103000Z-a.json";
const NEW = "sotyware/snapshots/photos/20261115T103000Z-b.json";

beforeEach(() => {
  calls = [];
  channels = [];
  opened = 0;
  jobs = [job()];
  snapshots = [
    { key: OLD, created: "2026-11-14T10:30:00Z", size: 900 },
    { key: NEW, created: "2026-11-15T10:30:00Z", size: 950 },
  ];
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    switch (cmd) {
      case "list_backups":
        return { storageError: null, jobs, active: null };
      case "list_cloud_accounts":
        return {
          accounts: [
            { ...account(), lastCheck: null },
            { ...account({ id: "vault", label: "Vault", provider: "hetzner", providerName: "Hetzner Object Storage", bucket: "vault-bucket" }) },
          ],
          storageError: null,
        };
      case "list_backup_snapshots":
        return snapshots;
      case "backup_snapshot_details":
        return {
          key: p["snapshot"],
          id: "b",
          created: "2026-11-15T10:30:00Z",
          sourceProvider: "backblaze-b2",
          sourceBucket: "soty-photos",
          sourcePrefix: "",
          stats: { objects: 120, bytes: 5_000_000, new: 3, changed: 1, deleted: 2, transferred: 4, transferredBytes: 40_000 },
          suspicious: false,
          verified: true,
          verifiedMs: Date.UTC(2026, 10, 15, 11),
          verifiedFull: false,
        };
      case "start_backup_task":
      case "delete_backup_snapshot":
        channels.push(p["onEvent"] as FakeChannel);
        return "backup-1-1";
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

async function render(a: AccountInfo = account()): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudSnapshots, { target, props: { account: a, onopenbackups: () => (opened += 1) } });
  flushSync();
  await settle();
  return target;
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === name);
  if (!found) {
    throw new Error(`no button "${name}"`);
  }
  return found;
}

function summary(over: Record<string, unknown>): Record<string, unknown> {
  return {
    task: "backup-1-1",
    job: "photos",
    kind: "backup",
    state: "completed",
    snapshot: null,
    stats: null,
    suspicious: false,
    verification: null,
    prune: null,
    restore: null,
    pruneSkipped: null,
    objectErrors: [],
    error: null,
    ...over,
  };
}

describe("the provider's own snapshots", () => {
  it("explains Backblaze's console-only snapshots without offering to manage them", async () => {
    const root = await render();
    const card = root.querySelector('[data-provider="backblaze-b2"]')!;
    expect(text(card)).toContain("Backblaze B2 snapshots Web console only");
    expect(text(card)).toContain("Backblaze offers no API for them, so sotyware cannot create or list them or show their progress.");
    expect(text(card)).toContain("b2-snapshot-");
    expect(card.querySelectorAll("button")).toHaveLength(0);
  });

  it("says plainly when sotyware knows of no snapshot feature", async () => {
    const root = await render(account({ provider: "hetzner", providerName: "Hetzner Object Storage" }));
    expect(text(root.querySelector('[data-provider="hetzner"]'))).toContain("sotyware knows of no snapshot feature in Hetzner Object Storage's API.");
  });
});

describe("sotyware's backup snapshots of the bucket", () => {
  it("lists the jobs that back up this bucket and their snapshots, newest first", async () => {
    const root = await render();
    const card = root.querySelector('[data-job="photos"]')!;
    expect(text(card)).toContain("Photos backup Healthy");
    expect(text(card)).toContain("Destination Vault, bucket vault-bucket, folder sotyware/");
    expect([...card.querySelectorAll("[data-snapshot]")].map((li) => li.getAttribute("data-snapshot"))).toEqual([NEW, OLD]);
    expect(text(card.querySelector(`[data-snapshot="${NEW}"]`))).toContain("newest");
    expect(calls.filter((c) => c.cmd === "list_backup_snapshots").map((c) => c.payload["job"])).toEqual(["photos"]);
    // It names them as sotyware's, never as the provider's.
    expect(text(root)).toContain("These are sotyware's own snapshots, not Backblaze B2's.");
  });

  it("shows a snapshot's details read from its manifest", async () => {
    const root = await render();
    const li = root.querySelector(`[data-snapshot="${OLD}"]`)!;
    button(li, "Details").click();
    await settle();
    expect(calls.find((c) => c.cmd === "backup_snapshot_details")!.payload).toMatchObject({ job: "photos", snapshot: OLD });
    const details = text(li.querySelector(".details"));
    expect(details).toContain("Files 120, 5.00 MB");
    expect(details).toContain("3 new, 1 changed, 2 gone; 4 transferred (40.0 kB)");
    expect(details).toContain("Verified passed");
    expect(details).toContain("sizes and checksums");
  });

  it("writes a new snapshot with Back up now and reports it", async () => {
    const root = await render();
    button(root, "Back up now").click();
    await settle();
    expect(calls.find((c) => c.cmd === "start_backup_task")!.payload).toMatchObject({ job: "photos", kind: "backup" });
    expect(text(root.querySelector('[data-job="photos"] .progress'))).toContain("Backing up");
    channels[0]!.onmessage({
      task: "backup-1-1",
      seq: Number.MAX_SAFE_INTEGER,
      type: "finished",
      summary: summary({
        snapshot: "sotyware/snapshots/photos/20261116T000000Z-c.json",
        stats: { objects: 121, bytes: 5_100_000, new: 1, changed: 0, deleted: 0, transferred: 1, transferredBytes: 100_000 },
        verification: { snapshot: "x", blobs: 1, missing: 0, damaged: 0, passed: true },
      }),
    });
    await settle();
    expect(text(root.querySelector('[role="status"]'))).toBe("New snapshot written: 121 files, 5.10 MB; 1 transferred. Verified.");
    // The list is read again.
    expect(calls.filter((c) => c.cmd === "list_backup_snapshots").length).toBeGreaterThanOrEqual(2);
  });

  it("deletes a snapshot only after the word delete is typed, and says what goes", async () => {
    const root = await render();
    button(root.querySelector(`[data-snapshot="${NEW}"]`)!, "Delete…").click();
    await settle();
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(text(dialog)).toContain("The files in Photos are not touched.");
    expect(text(dialog)).toContain("It is the newest snapshot: the backup's health will rest on the previous one.");
    const confirm = button(dialog, "Delete snapshot");
    expect(confirm.disabled).toBe(true);
    const input = dialog.querySelector<HTMLInputElement>("input")!;
    input.value = "remove";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(confirm.disabled).toBe(true);
    input.value = "Delete";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(confirm.disabled).toBe(false);
    confirm.click();
    await settle();
    expect(calls.find((c) => c.cmd === "delete_backup_snapshot")!.payload).toMatchObject({ job: "photos", snapshot: NEW });
    expect(button(document.body, "Deleting…").disabled).toBe(true);
    channels[0]!.onmessage({
      task: "backup-1-1",
      seq: Number.MAX_SAFE_INTEGER,
      type: "finished",
      summary: summary({
        kind: "delete-snapshot",
        snapshot: NEW,
        prune: { snapshotsKept: 1, snapshotsDeleted: 1, blobsDeleted: 2, bytesDeleted: 3_000_000, refused: 0 },
      }),
    });
    await settle();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(text(root.querySelector('[role="status"]'))).toContain("Snapshot deleted. 2 pieces of data no other snapshot used were removed (3.00 MB)");
  });

  it("warns before deleting the only snapshot, and shows a refusal", async () => {
    snapshots = [{ key: OLD, created: "2026-11-14T10:30:00Z", size: 900 }];
    const root = await render();
    button(root, "Delete…").click();
    await settle();
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(text(dialog)).toContain("It is the only snapshot of this backup: nothing will be left to restore from.");
    const input = dialog.querySelector<HTMLInputElement>("input")!;
    input.value = "delete";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    button(dialog, "Delete snapshot").click();
    await settle();
    channels[0]!.onmessage({
      task: "backup-1-1",
      seq: Number.MAX_SAFE_INTEGER,
      type: "finished",
      summary: summary({
        kind: "delete-snapshot",
        state: "failed",
        error: { kind: "access-denied", message: "the destination refused to delete the snapshot" },
      }),
    });
    await settle();
    expect(text(root.querySelector('[role="alert"]'))).toBe("Deleting the snapshot failed: the destination refused to delete the snapshot");
  });

  it("offers to set up a backup when there is none, and names backups kept here", async () => {
    jobs = [
      job({
        config: {
          ...(job()["config"] as object),
          id: "other",
          name: "Docs backup",
          source: { account: "docs", bucket: "d", prefix: "" },
          destination: { account: "b2-photos", bucket: "soty-photos", prefix: "bk/" },
        },
      }),
    ];
    const root = await render();
    expect(text(root)).toContain("No backup of this bucket yet.");
    button(root, "Set up a backup").click();
    expect(opened).toBe(1);
    expect(text(root)).toContain("“Docs backup”, a backup of docs, keeps its snapshots here under bk/.");
    expect(calls.some((c) => c.cmd === "list_backup_snapshots")).toBe(false);
  });
});
