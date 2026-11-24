// The bucket view's "Sync" tab (ADR-043) and its helpers, headless with
// Tauri's IPC mocks: what the page asks Rust, that it never sends a local
// path, that it asks before anything destructive, and that it shows the
// live status exactly as Rust reports it.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AccountInfo } from "./cloud";
import CloudSync from "./CloudSync.svelte";
import { overallSummary, parseChoice, parseOverview, parseStatus, stateLabel, type SyncOverview } from "./folderSync";
import { withDefaultLocale } from "./testLocale";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

let calls: { cmd: string; payload: Record<string, unknown> }[];
let overview: Record<string, unknown>;
let choice: Record<string, unknown> | null;
let details: Record<string, unknown>;
let channel: FakeChannel | null;
let failNext: string | null;
let app: ReturnType<typeof mount> | undefined;

const ACCOUNT: AccountInfo = {
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
};

const PATH = "D:\\CloudStorage\\Photos";

function status(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: "b2-photos-1a2b3c4d",
    account: "b2-photos",
    bucket: "soty-photos",
    path: PATH,
    state: "synced",
    message: null,
    error: null,
    paused: false,
    intervalMinutes: 15,
    versioning: "enabled",
    watching: true,
    files: 1234,
    bytes: 5_000_000_000,
    folders: 12,
    remoteFiles: 1234,
    remoteBytes: 5_000_000_000,
    pendingUploads: 0,
    pendingUploadBytes: 0,
    pendingDownloads: 0,
    pendingDownloadBytes: 0,
    failed: 0,
    skipped: 0,
    conflicts: 0,
    heldDeletions: null,
    trashFiles: 0,
    trashBytes: 0,
    freeBytes: 120_000_000_000,
    diskFull: false,
    lastSyncedMs: Date.now() - 2 * 60_000,
    lastCheckedMs: Date.now() - 2 * 60_000,
    nextCheckMs: Date.now() + 13 * 60_000,
    transfers: [],
    migration: null,
    activitySeq: 0,
    ...over,
  };
}

function withSync(s: Record<string, unknown> | null, over: Record<string, unknown> = {}): Record<string, unknown> {
  return { seq: 1, running: true, storageError: null, syncs: s === null ? [] : [s], ...over };
}

function chosen(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    token: "choice-abc",
    account: "b2-photos",
    sync: null,
    path: PATH,
    files: 0,
    bytes: 0,
    more: false,
    freeBytes: 120_000_000_000,
    bucketBytes: 5_000_000_000,
    bucketObjects: 1234,
    bucketMeasuredMs: Date.now() - 60 * 60_000,
    wasSyncFolder: false,
    versioning: "enabled",
    problems: [],
    warnings: [],
    canMove: false,
    ...over,
  };
}

function noDetails(): Record<string, unknown> {
  return { activity: [], failed: [], skipped: [], conflicts: [], heldDeletions: null };
}

beforeEach(() => {
  calls = [];
  overview = withSync(null);
  choice = chosen();
  details = noDetails();
  channel = null;
  failNext = null;
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const p = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, payload: p });
    if (failNext === cmd) {
      failNext = null;
      throw { kind: "busy", message: "this sync is paused; resume it first" };
    }
    switch (cmd) {
      case "list_folder_syncs":
        return overview;
      case "watch_folder_syncs":
        channel = p["onEvent"] as FakeChannel;
        return 7;
      case "choose_sync_folder":
        return choice;
      case "enable_folder_sync":
        overview = withSync(status({ state: "starting", files: 0, bytes: 0, lastSyncedMs: null }));
        return (overview["syncs"] as unknown[])[0];
      case "folder_sync_details":
        return details;
      case "disable_folder_sync":
        overview = withSync(null);
        return { path: PATH, trashKept: true };
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
  for (let i = 0; i < 6; i++) {
    await new Promise((r) => setTimeout(r, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudSync, { target, props: { account: ACCOUNT, onedit: () => calls.push({ cmd: "edit", payload: {} }) } });
  flushSync();
  await settle();
  return target;
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => text(b) === name);
  if (!found) {
    throw new Error(`no button "${name}" in ${text(root)}`);
  }
  return found;
}

function sent(cmd: string): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.payload);
}

/** Every payload the page sent, flattened: no local path may be in one
 * (the page only shows the path Rust chose). */
function everySentValue(): string {
  return JSON.stringify(calls.filter((c) => c.cmd !== "edit").map((c) => c.payload));
}

describe("the Sync tab without a sync", () => {
  it("explains what sync does and offers to choose a folder", async () => {
    const root = await render();
    expect(sent("list_folder_syncs")).toHaveLength(1);
    expect(sent("watch_folder_syncs")).toHaveLength(1);
    expect(text(root)).toContain("Sync soty-photos with a folder on this computer");
    expect(text(root)).toContain("a file changed on both sides is kept twice, never overwritten");
    expect(button(root, "Choose folder…").disabled).toBe(false);
  });

  it("changes nothing when the dialog is cancelled", async () => {
    choice = null;
    const root = await render();
    button(root, "Choose folder…").click();
    await settle();
    expect(sent("choose_sync_folder")).toEqual([{ account: "b2-photos", sync: null }]);
    expect(root.querySelector("[data-sync-choice]")).toBeNull();
    expect(sent("enable_folder_sync")).toHaveLength(0);
  });

  it("shows the chosen folder and what was found before anything starts", async () => {
    choice = chosen({
      files: 42,
      bytes: 9_000_000,
      wasSyncFolder: true,
      warnings: ["The folder already holds 42 files (9.0 MB). They are merged with the bucket."],
    });
    const root = await render();
    button(root, "Choose folder…").click();
    await settle();
    const card = root.querySelector("[data-sync-choice]")!;
    expect(text(card.querySelector("[data-choice-path]"))).toBe(PATH);
    expect(text(card)).toContain("42 files, 9.00 MB");
    expect(text(card)).toContain("120 GB");
    expect(text(card)).toContain("5.00 GB in 1,234 files");
    expect(text(card)).toContain("They are merged with the bucket.");
    expect(text(card)).toContain("stays in the bucket as an earlier version");
    expect(sent("enable_folder_sync")).toHaveLength(0);
    // Every 30 minutes, then start.
    const select = card.querySelector<HTMLSelectElement>("select")!;
    select.value = "30";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    button(card, "Start sync").click();
    await settle();
    expect(sent("enable_folder_sync")).toEqual([{ token: "choice-abc", intervalMinutes: 30 }]);
    expect(root.querySelector("[data-sync-choice]")).toBeNull();
    expect(text(root.querySelector('[role="status"]'))).toContain(`Files in ${PATH} and the bucket soty-photos are being compared`);
    expect(everySentValue()).not.toContain("CloudStorage");
  });

  it("cannot start with a problem, and Cancel forgets the choice in Rust", async () => {
    choice = chosen({ problems: ["sotyware's own data folder cannot be synced."] });
    const root = await render();
    button(root, "Choose folder…").click();
    await settle();
    expect(text(root.querySelector('[role="alert"]'))).toContain("own data folder");
    expect(button(root, "Start sync").disabled).toBe(true);
    button(root, "Cancel").click();
    await settle();
    expect(sent("discard_sync_folder_choice")).toEqual([{ token: "choice-abc" }]);
    expect(root.querySelector("[data-sync-choice]")).toBeNull();
  });

  it("says so when folder sync is switched off", async () => {
    overview = withSync(null, { running: false });
    const root = await render();
    expect(text(root)).toContain("Folder sync is switched off in Settings › Features");
  });
});

describe("the Sync tab with a sync", () => {
  it("shows the state, the folder, the figures and what waits", async () => {
    overview = withSync(status({ pendingUploads: 3, pendingUploadBytes: 3_000_000, pendingDownloads: 1, pendingDownloadBytes: 1_000 }));
    const root = await render();
    expect(text(root.querySelector("[data-sync-badge]"))).toBe("Synced");
    expect(text(root.querySelector("[data-sync-path]"))).toBe(PATH);
    const tiles = [...root.querySelectorAll(".tile")].map(text);
    expect(tiles[0]).toBe("Files in step 1,234 12 folders");
    expect(tiles[1]).toBe("On this computer 5.00 GB 120 GB free on the drive");
    expect(tiles[2]).toBe("In the bucket 5.00 GB 1,234 files, current versions");
    expect(tiles[3]).toContain("Last in step 2 minutes ago");
    expect(text(root.querySelector("[data-sync-pending]"))).toBe(
      "To upload: 3 (3.00 MB). To download: 1 (1.00 kB).",
    );
    expect(text(root)).toContain("Changes in the folder are noticed as they happen.");
    expect(sent("folder_sync_details")).toEqual([{ id: "b2-photos-1a2b3c4d" }]);
  });

  it("writes its figures in en-US whatever the webview's language", async () => {
    // Until 2026-11-24 the counts followed the webview's language and the
    // sizes did not: "5.00 GB in 1 234 files" in fr-FR, "5.00 GB in 1.234
    // files" in de-DE.
    for (const language of ["fr-FR", "de-DE"]) {
      await withDefaultLocale(language, async () => {
        overview = withSync(status({ pendingUploads: 1234, pendingUploadBytes: 3_000_000 }));
        const root = await render();
        const tiles = [...root.querySelectorAll(".tile")].map(text);
        expect(tiles[0], language).toBe("Files in step 1,234 12 folders");
        expect(tiles[2], language).toBe("In the bucket 5.00 GB 1,234 files, current versions");
        expect(text(root.querySelector("[data-sync-pending]")), language).toBe("To upload: 1,234 (3.00 MB). To download: 0 (0 bytes).");
        unmount(app!);
        document.body.innerHTML = "";

        overview = withSync(null);
        const choosing = await render();
        button(choosing, "Choose folder…").click();
        await settle();
        expect(text(choosing.querySelector("[data-sync-choice]")), language).toContain("5.00 GB in 1,234 files");
        unmount(app!);
        app = undefined;
        document.body.innerHTML = "";
      });
    }
  });

  it("follows the live status from the channel, with running transfers", async () => {
    overview = withSync(status());
    const root = await render();
    channel!.onmessage(
      withSync(
        status({
          state: "syncing",
          transfers: [{ path: "2024/video.mp4", kind: "upload", bytes: 50, total: 200, startedMs: Date.now() - 5000, speed: 10 }],
        }),
        { seq: 5 },
      ),
    );
    await settle();
    expect(text(root.querySelector("[data-sync-badge]"))).toBe("Syncing");
    const t = root.querySelector(".transfers li")!;
    expect(text(t)).toContain("2024/video.mp4");
    expect(text(t)).toContain("25% · 10 bytes/s");
    expect(t.querySelector("progress")?.getAttribute("aria-label")).toBe("Uploading 2024/video.mp4");
  });

  it("pauses, resumes, checks now and opens the folder by the sync's id only", async () => {
    overview = withSync(status());
    const root = await render();
    button(root, "Sync now").click();
    await settle();
    button(root, "Pause sync").click();
    await settle();
    button(root, "Open folder").click();
    await settle();
    expect(sent("sync_folder_now")).toEqual([{ id: "b2-photos-1a2b3c4d" }]);
    expect(sent("set_folder_sync_paused")).toEqual([{ id: "b2-photos-1a2b3c4d", paused: true }]);
    expect(sent("open_sync_folder")).toEqual([{ id: "b2-photos-1a2b3c4d", place: "folder" }]);
    overview = withSync(status({ state: "paused", paused: true }), { seq: 9 });
    channel!.onmessage(overview);
    await settle();
    expect(text(root.querySelector("[data-sync-badge]"))).toBe("Paused");
    expect(button(root, "Sync now").disabled).toBe(true);
    button(root, "Resume sync").click();
    await settle();
    expect(sent("set_folder_sync_paused")[1]).toEqual({ id: "b2-photos-1a2b3c4d", paused: false });
    expect(everySentValue()).not.toContain("CloudStorage");
  });

  it("shows a refusal from Rust", async () => {
    overview = withSync(status());
    failNext = "sync_folder_now";
    const root = await render();
    button(root, "Sync now").click();
    await settle();
    expect(text(root.querySelector('[role="alert"]'))).toContain("this sync is paused; resume it first");
  });

  it("asks before held deletions run, and answers with Restore or Delete", async () => {
    overview = withSync(
      status({ state: "attention", heldDeletions: { local: 60, remote: 0, synced: 60, askedMs: 1 }, message: "Many files would be deleted at once; sync waits for your answer." }),
    );
    const root = await render();
    expect(text(root.querySelector("[data-sync-badge]"))).toBe("Waiting for you");
    const held = root.querySelector(".held")!;
    expect(text(held)).toContain("60 files deleted in the bucket would be removed from this folder (into the sync trash).");
    expect(text(held)).toContain("more than half of the 60 synced files");
    button(held, "Restore them").click();
    await settle();
    expect(sent("answer_folder_sync_deletions")).toEqual([{ id: "b2-photos-1a2b3c4d", allow: false }]);
    button(held, "Delete them").click();
    await settle();
    expect(sent("answer_folder_sync_deletions")[1]).toEqual({ id: "b2-photos-1a2b3c4d", allow: true });
  });

  it("settles conflicts three ways", async () => {
    overview = withSync(status({ conflicts: 1 }));
    details = {
      ...noDetails(),
      conflicts: [{ path: "docs/report.docx", copy: "docs/report (conflicted copy 2026-11-16 1403).docx", atMs: 1 }],
    };
    const root = await render();
    const card = root.querySelector(".conflicts")!;
    expect(text(card)).toContain("docs/report.docx and docs/report (conflicted copy 2026-11-16 1403).docx");
    button(card, "Keep this computer's").click();
    await settle();
    button(card, "Keep the other one").click();
    await settle();
    button(card, "Keep both").click();
    await settle();
    expect(sent("resolve_folder_sync_conflict").map((p) => p["choice"])).toEqual(["keep-local", "keep-remote", "keep-both"]);
    expect(sent("resolve_folder_sync_conflict")[0]).toEqual({ id: "b2-photos-1a2b3c4d", path: "docs/report.docx", choice: "keep-local" });
  });

  it("lists recent activity, problems with a retry, and what is not synced", async () => {
    overview = withSync(status({ failed: 1, skipped: 1, activitySeq: 3 }));
    details = {
      activity: [{ atMs: Date.now() - 60_000, path: "a.txt", op: "upload", bytes: 1000, message: null }],
      failed: [{ path: "locked.pst", op: "upload", kind: "local-file", message: "the file is open in another program", attempts: 2, nextMs: Date.now() + 60_000, firstMs: 1 }],
      skipped: [{ path: "con.txt", reason: "the name is reserved for a device on Windows", side: "remote" }],
      conflicts: [],
      heldDeletions: null,
    };
    const root = await render();
    const card = root.querySelector(".details")!;
    expect(text(card)).toContain("Uploaded a.txt");
    button(card, "Problems (1)").click();
    flushSync();
    expect(text(card)).toContain("locked.pst");
    expect(text(card)).toContain("the file is open in another program");
    button(card, "Try again now").click();
    await settle();
    expect(sent("retry_folder_sync")).toEqual([{ id: "b2-photos-1a2b3c4d" }]);
    button(card, "Not synced (1)").click();
    flushSync();
    expect(text(card)).toContain("con.txt");
    expect(text(card)).toContain("the name is reserved for a device on Windows");
  });

  it("changes the interval", async () => {
    overview = withSync(status());
    const root = await render();
    const select = root.querySelector<HTMLSelectElement>(".settings select")!;
    expect(select.value).toBe("15");
    select.value = "60";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await settle();
    expect(sent("set_folder_sync_interval")).toEqual([{ id: "b2-photos-1a2b3c4d", minutes: 60 }]);
  });

  it("asks before turning off, and says the files stay", async () => {
    overview = withSync(status());
    const root = await render();
    button(root, "Turn sync off…").click();
    flushSync();
    const dialog = document.querySelector('[role="dialog"]')!;
    expect(text(dialog)).toContain(`The files stay in ${PATH} and the bucket is not changed. Nothing is deleted on either side.`);
    expect(sent("disable_folder_sync")).toHaveLength(0);
    button(dialog, "Turn sync off").click();
    await settle();
    expect(sent("disable_folder_sync")).toEqual([{ id: "b2-photos-1a2b3c4d" }]);
    expect(text(root.querySelector('[role="status"]'))).toContain(`Sync is off. The files stay in ${PATH}; the bucket was not changed.`);
    expect(text(root.querySelector('[role="status"]'))).toContain(".sotyware-sync/trash");
  });

  it("moves the folder only into an empty one, else starts over", async () => {
    overview = withSync(status());
    choice = chosen({ sync: "b2-photos-1a2b3c4d", path: "E:\\Photos", canMove: false, files: 3, bytes: 30 });
    const root = await render();
    button(root, "Change folder…").click();
    await settle();
    expect(sent("choose_sync_folder")).toEqual([{ account: "b2-photos", sync: "b2-photos-1a2b3c4d" }]);
    const card = root.querySelector("[data-sync-choice]")!;
    const [move, fresh] = [...card.querySelectorAll<HTMLInputElement>('input[type="radio"]')];
    expect(move!.disabled).toBe(true);
    expect(fresh!.checked).toBe(true);
    button(card, "Switch folder").click();
    await settle();
    expect(sent("change_sync_folder")).toEqual([{ token: "choice-abc", mode: "fresh" }]);

    choice = chosen({ sync: "b2-photos-1a2b3c4d", path: "E:\\Empty", canMove: true, token: "choice-2" });
    button(root, "Change folder…").click();
    await settle();
    const card2 = root.querySelector("[data-sync-choice]")!;
    expect(card2.querySelector<HTMLInputElement>('input[value="move"]')!.checked).toBe(true);
    button(card2, "Move files").click();
    await settle();
    expect(sent("change_sync_folder")[1]).toEqual({ token: "choice-2", mode: "move" });
  });

  it("offers to edit the account when the provider refuses its key", async () => {
    overview = withSync(
      status({ state: "error", message: "the provider does not accept this account's key any more", error: { kind: "invalid-credentials", message: "x" } }),
    );
    const root = await render();
    expect(text(root.querySelector("[data-sync-badge]"))).toBe("Error");
    button(root, "Edit account").click();
    expect(calls.some((c) => c.cmd === "edit")).toBe(true);
  });
});

describe("folder sync words and checks", () => {
  it("names every state", () => {
    const base = parseStatus(status());
    expect(stateLabel(base)).toEqual({ label: "Synced", tone: "success" });
    expect(stateLabel({ ...base, state: "offline" })).toEqual({ label: "Offline", tone: "warning" });
    expect(stateLabel({ ...base, state: "unavailable" }).label).toBe("Folder unavailable");
    expect(stateLabel({ ...base, state: "stopped" }).label).toBe("Off");
  });

  it("sums every sync up for the sidebar", () => {
    const one = (over: Record<string, unknown>): SyncOverview => parseOverview(withSync(status(over)));
    expect(overallSummary(null)).toBeNull();
    expect(overallSummary(parseOverview(withSync(null)))).toBeNull();
    expect(overallSummary(one({}))?.text).toBe("Folder up to date");
    expect(overallSummary(one({ state: "syncing", pendingUploads: 2, pendingDownloads: 1 }))?.text).toBe("Syncing 3 files…");
    expect(overallSummary(one({ state: "attention", failed: 1 }))?.tone).toBe("warning");
    expect(overallSummary(one({ state: "error" }))?.tone).toBe("danger");
    expect(overallSummary(one({ state: "paused", paused: true }))?.text).toBe("Folder sync is paused");
    expect(overallSummary(parseOverview(withSync(status(), { running: false })))?.text).toBe("Folder sync is switched off in Settings");
  });

  it("refuses responses that do not fit", () => {
    expect(() => parseStatus({ ...status(), state: "dancing" })).toThrow(/invalid sync status/);
    expect(() => parseStatus({ ...status(), files: "many" })).toThrow();
    expect(() => parseChoice({ ...chosen(), problems: [1] })).toThrow();
    expect(() => parseOverview({ seq: 1, running: true, storageError: null })).toThrow();
  });
});
