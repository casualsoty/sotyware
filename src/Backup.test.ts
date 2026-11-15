// Headless tests of the Backup page (ADR-031) and its typed wrappers. The
// Rust side is Tauri's IPC mock, playing src-tauri/src/backup/mod.rs for the
// cases used here; the engine itself is tested against in-process S3
// servers in backup/tests.rs.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import Backup from "./Backup.svelte";
import {
  CANCEL_BACKUP_TASK,
  CHECK_BACKUP_PROTECTION,
  EXPORT_BACKUP_MANIFEST,
  LIST_BACKUP_SNAPSHOTS,
  LIST_BACKUPS,
  REMOVE_BACKUP_JOB,
  RESTORE_BACKUP,
  SAVE_BACKUP_JOB,
  START_BACKUP_TASK,
  parseBackupEvent,
  parseBackupList,
  type JobStatus,
  type TaskSummary,
} from "./backup";
import { LIST_CLOUD_ACCOUNTS, type AccountInfo } from "./cloud";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

function account(id: string, label: string, bucket: string): AccountInfo {
  return {
    id,
    label,
    provider: "s3-compatible",
    providerName: "S3-compatible service",
    region: "us-east-1",
    endpoint: "https://s3.example.com",
    endpointUrl: "https://s3.example.com",
    bucket,
    configError: null,
    lastCheck: null,
  };
}

function job(overrides: Partial<JobStatus> = {}): JobStatus {
  return {
    config: {
      id: "photos",
      name: "Photos",
      source: { account: "b2", bucket: "photos", prefix: "" },
      destination: { account: "hetzner", bucket: "vault", prefix: "sotyware/" },
      intervalHours: 24,
      retention: { keepLast: 7, keepDaily: 14, keepWeekly: 8 },
      suspiciousPercent: 30,
    },
    health: "healthy",
    sameAccount: false,
    lastBackup: { kind: "backup", state: "completed", startedMs: Date.UTC(2026, 10, 15, 3), finishedMs: Date.UTC(2026, 10, 15, 3, 5), snapshot: "s1", suspicious: false, passed: null, error: null, summary: null },
    lastVerification: { kind: "verify", state: "completed", startedMs: Date.UTC(2026, 10, 15, 3, 5), finishedMs: Date.UTC(2026, 10, 15, 3, 6), snapshot: "s1", suspicious: false, passed: true, error: null, summary: null },
    lastPrune: null,
    lastSnapshot: "s1",
    nextDueMs: Date.UTC(2026, 10, 16, 3),
    ...overrides,
  };
}

function summary(overrides: Partial<TaskSummary> = {}): TaskSummary {
  return {
    task: "backup-1",
    job: "photos",
    kind: "backup",
    state: "completed",
    snapshot: "sotyware/snapshots/photos/20261115T030000Z-a.json",
    stats: { objects: 10, bytes: 2048, new: 2, changed: 1, deleted: 0, transferred: 3, transferredBytes: 1024 },
    suspicious: false,
    verification: { snapshot: "s", deep: false, blobs: 9, bytes: 2048, missing: [], damaged: [], passed: true },
    prune: { snapshotsKept: 7, snapshotsDeleted: 1, blobsDeleted: 2, bytesDeleted: 512, refused: 0 },
    restore: null,
    pruneSkipped: null,
    objectErrors: [],
    error: null,
    ...overrides,
  };
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let jobs: JobStatus[];
let accounts: AccountInfo[];
let channels: FakeChannel[];
let app: ReturnType<typeof mount> | undefined;

function backend(cmd: string, p: Record<string, unknown>): unknown {
  switch (cmd) {
    case LIST_BACKUPS:
      return { storageError: null, jobs, active: null };
    case LIST_CLOUD_ACCOUNTS:
      return { accounts, storageError: null };
    case SAVE_BACKUP_JOB: {
      const input = p["job"] as { name: string };
      const saved = { ...job().config, id: "new-job", name: input.name };
      jobs = [...jobs, job({ config: saved, health: "never-run", lastBackup: null, lastVerification: null, lastSnapshot: null })];
      return saved;
    }
    case REMOVE_BACKUP_JOB:
      jobs = jobs.filter((j) => j.config.id !== p["id"]);
      return null;
    case START_BACKUP_TASK:
    case RESTORE_BACKUP:
      channels.push(p["onEvent"] as FakeChannel);
      return `backup-${channels.length}`;
    case CANCEL_BACKUP_TASK:
      return "cancelling";
    case LIST_BACKUP_SNAPSHOTS:
      return [
        { key: "sotyware/snapshots/photos/20261114T030000Z-a.json", created: "2026-11-14T03:00:00Z", size: 100 },
        { key: "sotyware/snapshots/photos/20261115T030000Z-b.json", created: "2026-11-15T03:00:00Z", size: 120 },
      ];
    case EXPORT_BACKUP_MANIFEST:
      return "sotyware-recovery-photos-20261115T030000Z.json";
    case CHECK_BACKUP_PROTECTION:
      return {
        sameAccount: false,
        versioning: "enabled",
        objectLock: true,
        defaultMode: "COMPLIANCE",
        defaultDays: 30,
        immutable: true,
        summary: "Object Lock in compliance mode keeps every new backup object for 30 days: nobody can delete or shorten it, not even you.",
      };
    default:
      return null;
  }
}

beforeEach(() => {
  calls = [];
  channels = [];
  jobs = [job()];
  accounts = [account("b2", "Backblaze", "photos"), account("hetzner", "Hetzner", "vault")];
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    return backend(cmd, p);
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
  for (let i = 0; i < 4; i++) {
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(Backup, { target });
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

async function send(ch: FakeChannel, message: unknown): Promise<void> {
  ch.onmessage(message);
  flushSync();
  await settle();
}

describe("the Backup page", () => {
  it("shows each job with its health, route and history", async () => {
    jobs = [job(), job({ config: { ...job().config, id: "docs", name: "Docs", destination: { account: "b2", bucket: "photos", prefix: "bk/" } }, health: "large-change", sameAccount: true })];
    const root = await render();
    const items = [...root.querySelectorAll<HTMLElement>("li[data-job]")];
    expect(items.map((i) => i.dataset["health"])).toEqual(["healthy", "large-change"]);
    expect(text(items[0]!.querySelector(".route"))).toContain("Backblaze / photos/ → Hetzner / vault/sotyware/");
    expect(text(items[0]!.querySelector(".health"))).toBe("Healthy: the newest backup was verified.");
    expect(text(items[0])).toContain("passed");
    expect(text(items[1]!.querySelector(".health"))).toContain("Older snapshots are kept until you review it");
    expect(text(items[1])).toContain("Same account as the source: not an independent copy.");
    expect(items[0]!.querySelector(".warning")).toBeNull();
  });

  it("never calls an unverified backup healthy", async () => {
    jobs = [job({ health: "not-verified", lastVerification: null })];
    const root = await render();
    expect(text(root.querySelector(".health"))).toBe("Not verified: the newest backup has not passed verification yet.");
    expect(text(root.querySelector("li[data-job]"))).not.toContain("Healthy");
  });

  it("asks for an account first when there is none", async () => {
    accounts = [];
    jobs = [];
    const root = await render();
    expect(text(root)).toContain("Add a storage account in Cloud Storage first");
    expect(root.querySelector("form")).toBeNull();
  });

  it("creates a job with two different accounts by default and warns about the same account", async () => {
    const root = await render();
    await click(root, "New backup job");
    const src = root.querySelector<HTMLSelectElement>("#bk-source")!;
    const dst = root.querySelector<HTMLSelectElement>("#bk-dest")!;
    expect([src.value, dst.value]).toEqual(["b2", "hetzner"]);
    expect(root.querySelector("form .warning")).toBeNull();
    dst.value = "b2";
    dst.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(text(root.querySelector("form .warning"))).toContain("not against a compromised key");
    dst.value = "hetzner";
    dst.dispatchEvent(new Event("change", { bubbles: true }));
    const name = root.querySelector<HTMLInputElement>("#bk-name")!;
    name.value = "Everything";
    name.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    root.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    flushSync();
    await settle();
    const sent = calls.find((c) => c.cmd === SAVE_BACKUP_JOB)!.payload["job"];
    expect(sent).toEqual({
      id: null,
      name: "Everything",
      sourceAccount: "b2",
      sourcePrefix: "",
      destinationAccount: "hetzner",
      destinationPrefix: "sotyware-backup/",
      intervalHours: 24,
      retention: { keepLast: 7, keepDaily: 14, keepWeekly: 8 },
      suspiciousPercent: 30,
    });
    expect(root.querySelector("form")).toBeNull();
    expect(text(root.querySelector("[role=status]"))).toContain('Saved backup job "Everything"');
    expect(root.querySelectorAll("li[data-job]")).toHaveLength(2);
  });

  it("runs a backup with progress and reports what was verified and pruned", async () => {
    const root = await render();
    await click(root, "Back up now");
    expect(calls.find((c) => c.cmd === START_BACKUP_TASK)!.payload).toMatchObject({ job: "photos", kind: "backup" });
    expect(button(root, "Verify").disabled).toBe(true);
    await send(channels[0]!, { task: "backup-1", seq: 1, type: "progress", progress: { phase: "transferring", done: 3, total: 10, bytes: 0 } });
    expect(text(root.querySelector(".task"))).toContain("Backing up 3 of 10");
    expect(root.querySelector<HTMLProgressElement>(".task progress")!.value).toBe(3);
    await click(root, "Cancel");
    expect(calls.find((c) => c.cmd === CANCEL_BACKUP_TASK)!.payload).toEqual({ task: "backup-1" });
    await send(channels[0]!, { task: "backup-1", seq: 2, type: "finished", summary: summary() });
    expect(root.querySelector(".task")).toBeNull();
    const r = text(root.querySelector(".result"));
    expect(r).toContain("Backup: completed.");
    expect(r).toContain("2 new, 1 changed, 0 gone; 3 transferred");
    expect(r).toContain("Verified 9 stored object(s).");
    expect(r).toContain("Retention removed 1 snapshot(s)");
  });

  it("says when retention was skipped after a large change", async () => {
    const root = await render();
    await click(root, "Back up now");
    await send(channels[0]!, {
      task: "backup-1",
      seq: 1,
      type: "finished",
      summary: summary({ suspicious: true, prune: null, pruneSkipped: "many objects changed" }),
    });
    const r = text(root.querySelector(".result"));
    expect(r).toContain("older snapshots are kept");
    expect(r).toContain("Retention did not run: many objects changed.");
  });

  it("lists snapshots and restores to a folder or a bucket, and saves a recovery file", async () => {
    const root = await render();
    await click(root, "Snapshots…");
    const snaps = [...root.querySelectorAll<HTMLElement>("li[data-snapshot]")];
    expect(snaps.map((s) => s.dataset["snapshot"])).toEqual([
      "sotyware/snapshots/photos/20261115T030000Z-b.json",
      "sotyware/snapshots/photos/20261114T030000Z-a.json",
    ]);
    await click(snaps[0]!, "Restore to a folder…");
    expect(calls.find((c) => c.cmd === RESTORE_BACKUP)!.payload).toMatchObject({
      job: "photos",
      snapshot: "sotyware/snapshots/photos/20261115T030000Z-b.json",
      filter: "",
      target: { kind: "local" },
    });
    await send(channels[0]!, {
      task: "backup-1",
      seq: 1,
      type: "finished",
      summary: summary({ kind: "restore", stats: null, verification: null, prune: null, restore: { restored: 5, bytes: 100, skipped: [], renamed: [["a", "a (1)"]], failed: [] } }),
    });
    expect(text(root.querySelector(".result"))).toContain("Restored and verified 5 file(s)");
    expect(text(root.querySelector(".result"))).toContain("1 saved under another name");

    await click(root.querySelector<HTMLElement>("li[data-snapshot]")!, "Restore to the bucket");
    expect(calls.filter((c) => c.cmd === RESTORE_BACKUP)[1]!.payload).toMatchObject({
      target: { kind: "bucket", account: "b2", prefix: "restored/" },
    });
    await send(channels[1]!, { task: "backup-2", seq: 1, type: "finished", summary: summary({ kind: "restore" }) });

    await click(root.querySelector<HTMLElement>("li[data-snapshot]")!, "Save recovery file…");
    expect(calls.find((c) => c.cmd === EXPORT_BACKUP_MANIFEST)!.payload).toEqual({
      job: "photos",
      snapshot: "sotyware/snapshots/photos/20261115T030000Z-b.json",
    });
    expect(text(root.querySelector("[role=status]"))).toContain("it contains no key");
  });

  it("shows the destination's protection as the provider reports it", async () => {
    const root = await render();
    await click(root, "Protection…");
    const p = root.querySelector<HTMLElement>(".protection")!;
    expect(p.dataset["immutable"]).toBe("true");
    expect(text(p)).toContain("not even you");
  });

  it("removes a job only after saying the backup data stays", async () => {
    const root = await render();
    await click(root, "Remove…");
    expect(text(root.querySelector(".confirm"))).toContain("Its backup data at the destination stays.");
    await click(root, "Remove job");
    expect(calls.find((c) => c.cmd === REMOVE_BACKUP_JOB)!.payload).toEqual({ id: "photos" });
    expect(root.querySelectorAll("li[data-job]")).toHaveLength(0);
  });
});

describe("the backup wrappers", () => {
  it("parse the Rust shapes and refuse anything else", () => {
    expect(parseBackupList({ storageError: null, jobs: [job()], active: null }).jobs[0]!.health).toBe("healthy");
    expect(() => parseBackupList({ storageError: null, jobs: [{ ...job(), health: "fine" }], active: null })).toThrow("invalid job status");
    expect(() => parseBackupList({ storageError: null, jobs: [], active: { task: "t", job: "j", kind: "dance" } })).toThrow("invalid active task");
    expect(parseBackupEvent({ task: "t", seq: 1, type: "progress", progress: { phase: "listing", done: 0, total: 0, bytes: 0 } }).type).toBe("progress");
    expect(() => parseBackupEvent({ task: "t", seq: 1, type: "other" })).toThrow("invalid backup event");
  });
});
