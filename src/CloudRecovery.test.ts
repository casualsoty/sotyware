// Headless tests of the recovery view (ADR-030) and its typed wrappers. The
// Rust side is Tauri's IPC mock, playing src-tauri/src/cloud/recovery.rs for
// the cases used here; the commands themselves are tested against the
// in-process S3 servers in recovery/tests.rs.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import CloudRecovery from "./CloudRecovery.svelte";
import type { AccountInfo } from "./cloud";
import {
  ABORT_CLOUD_INCOMPLETE_UPLOAD,
  DISMISS_CLOUD_RECOVERY,
  INSPECT_CLOUD_RECOVERY,
  LIST_CLOUD_INCOMPLETE_UPLOADS,
  LIST_CLOUD_RECOVERY,
  parseInspection,
  parseRecoveryList,
  type Inspection,
  type RecoveryItem,
} from "./cloudRecovery";

interface IpcCall {
  cmd: string;
  payload: Record<string, unknown>;
}

const ACCOUNT: AccountInfo = {
  id: "photos",
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

function entry(overrides: Partial<RecoveryItem> = {}): RecoveryItem {
  return {
    id: 1,
    kind: "upload",
    state: "multipart-open",
    accountId: "photos",
    accountLabel: "Photos",
    bucket: "soty-photos",
    key: "2026/big.mov",
    fileName: "big.mov",
    size: 123,
    operation: null,
    steps: 0,
    multipart: true,
    startedMs: Date.UTC(2026, 10, 15, 9, 0),
    updatedMs: Date.UTC(2026, 10, 15, 9, 1),
    summary: "sotyware stopped while this file was being uploaded in parts.",
    ...overrides,
  };
}

let calls: IpcCall[];
let items: RecoveryItem[];
let unavailable: string | null;
let inspection: Inspection;
let incomplete: { key: string; uploadId: string; initiated: string | null; recorded: boolean }[];
let app: ReturnType<typeof mount> | undefined;

function backend(cmd: string, payload: Record<string, unknown>): unknown {
  switch (cmd) {
    case LIST_CLOUD_RECOVERY:
      return { unavailable, items };
    case INSPECT_CLOUD_RECOVERY:
      return inspection;
    case DISMISS_CLOUD_RECOVERY:
      items = items.filter((i) => i.id !== payload["id"]);
      return null;
    case LIST_CLOUD_INCOMPLETE_UPLOADS:
      return { uploads: incomplete, more: false };
    case ABORT_CLOUD_INCOMPLETE_UPLOAD:
      incomplete = incomplete.filter((u) => u.uploadId !== payload["uploadId"]);
      items = items.filter((i) => i.kind !== "upload");
      return "aborted";
    default:
      return null;
  }
}

beforeEach(() => {
  calls = [];
  items = [];
  unavailable = null;
  incomplete = [];
  inspection = {
    id: 1,
    verdict: "needs-attention",
    summary: "1 incomplete multipart upload(s) for this name remain at the provider.",
    counts: { absent: 1 },
    keys: { absent: ["2026/big.mov"] },
    notInspected: 0,
    openUploads: [{ key: "2026/big.mov", uploadId: "up-1", initiated: "2026-11-15T09:00:01Z", recorded: true }],
  };
  mockWindows("main");
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload: (payload ?? {}) as Record<string, unknown> });
    return backend(cmd, (payload ?? {}) as Record<string, unknown>);
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
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

async function render(accounts: AccountInfo[] = [ACCOUNT]): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudRecovery, { target, props: { accounts } });
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

describe("the recovery view", () => {
  it("shows nothing but the unfinished-uploads tool when the journal is empty", async () => {
    const root = await render();
    expect(root.querySelector("section.recovery")).toBeNull();
    expect(root.querySelector("[role=alert]")).toBeNull();
    expect(text(root.querySelector("details.scan summary"))).toBe("Unfinished large uploads");
    expect(calls.map((c) => c.cmd)).toEqual([LIST_CLOUD_RECOVERY]);
  });

  it("says when the journal cannot be used, without blocking anything", async () => {
    unavailable = "the database is damaged";
    const root = await render();
    const warning = root.querySelector("[data-recovery=unavailable]");
    expect(text(warning)).toContain("The operation journal cannot be used (the database is damaged)");
    expect(text(warning)).toContain("still work");
  });

  it("lists interrupted work with what is known and inspects an entry at the provider", async () => {
    items = [
      entry(),
      entry({ id: 2, kind: "operation", state: "partial", operation: "move", steps: 12, key: null, fileName: null, multipart: false, summary: "A move of 12 object(s) ended with only part of it applied." }),
    ];
    const root = await render();
    const section = root.querySelector("section.recovery")!;
    expect(text(section.querySelector("h3"))).toBe("Interrupted work (2)");
    const rows = [...section.querySelectorAll<HTMLElement>("li[data-recovery-item]")];
    expect(text(rows[0]!.querySelector("strong"))).toBe("Upload of big.mov to Photos / soty-photos");
    expect(text(rows[1]!.querySelector("strong"))).toBe("Move of 12 object(s) in Photos / soty-photos");
    expect(text(rows[1])).toContain("ended with only part of it applied");

    inspection = {
      id: 2,
      verdict: "needs-attention",
      summary: "3 moved; 2 copied with the original still in place.",
      counts: { moved: 3, "original-kept": 2 },
      keys: { moved: ["a", "b", "c"], "original-kept": ["d.txt", "e.txt"] },
      notInspected: 0,
      openUploads: [],
    };
    await click(rows[1]!, "Inspect");
    expect(calls.at(-1)).toEqual({ cmd: INSPECT_CLOUD_RECOVERY, payload: { id: 2 } });
    const insp = rows[1]!.querySelector<HTMLElement>(".inspection")!;
    expect(insp.dataset["verdict"]).toBe("needs-attention");
    expect(text(insp)).toContain("2 copied with the original still in place");
    // The kept originals are named so the user can find them.
    expect(text(insp.querySelector("details summary"))).toBe("original kept: 2");
    expect([...insp.querySelectorAll(".keys li")].map((li) => li.textContent)).toEqual(["d.txt", "e.txt"]);
  });

  it("aborts an incomplete upload only after confirmation, and warns when sotyware did not record it", async () => {
    items = [entry()];
    inspection = {
      ...inspection,
      openUploads: [
        { key: "2026/big.mov", uploadId: "up-1", initiated: "2026-11-15T09:00:01Z", recorded: true },
        { key: "2026/big.mov", uploadId: "up-other", initiated: "2026-11-15T09:30:00Z", recorded: false },
      ],
    };
    const root = await render();
    await click(root, "Inspect");
    const other = root.querySelector<HTMLElement>('li[data-upload="up-other"]')!;
    expect(text(other)).toContain("possibly another device");
    await click(other, "Abort…");
    expect(text(other)).toContain("Another device may still be uploading this file.");
    await click(other, "Keep");
    expect(calls.some((c) => c.cmd === ABORT_CLOUD_INCOMPLETE_UPLOAD)).toBe(false);

    const ours = root.querySelector<HTMLElement>('li[data-upload="up-1"]')!;
    await click(ours, "Abort…");
    expect(text(ours)).not.toContain("Another device");
    await click(ours, "Abort upload");
    expect(calls.find((c) => c.cmd === ABORT_CLOUD_INCOMPLETE_UPLOAD)?.payload).toEqual({
      id: "photos",
      bucket: "soty-photos",
      key: "2026/big.mov",
      uploadId: "up-1",
    });
    expect(text(root.querySelector("[role=status]"))).toContain("The incomplete upload was aborted");
  });

  it("dismisses an entry after confirmation, warning that parts may remain", async () => {
    items = [entry()];
    const root = await render();
    await click(root, "Dismiss");
    expect(text(root.querySelector(".confirm"))).toContain("may remain at the provider and be billed");
    await click(root, "Remove entry");
    expect(calls.find((c) => c.cmd === DISMISS_CLOUD_RECOVERY)?.payload).toEqual({ id: 1 });
    expect(root.querySelector("section.recovery")).toBeNull();
  });

  it("lists an account's unfinished uploads and aborts the one chosen", async () => {
    incomplete = [
      { key: "a.bin", uploadId: "u-a", initiated: null, recorded: false },
      { key: "b.bin", uploadId: "u-b", initiated: "2026-11-01T00:00:00Z", recorded: true },
    ];
    const root = await render();
    const select = root.querySelector<HTMLSelectElement>("#recovery-scan-account")!;
    expect(button(root, "List unfinished uploads").disabled).toBe(true);
    select.value = "photos";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(button(root, "List unfinished uploads").disabled, `select value ${select.value}`).toBe(false);
    await click(root, "List unfinished uploads");
    expect(calls.find((c) => c.cmd === LIST_CLOUD_INCOMPLETE_UPLOADS)?.payload).toEqual({ id: "photos", bucket: "soty-photos" });
    const rows = [...root.querySelectorAll<HTMLElement>("details.scan li[data-upload]")];
    expect(rows.map((r) => r.dataset["upload"])).toEqual(["u-a", "u-b"]);
    await click(rows[1]!, "Abort…");
    await click(rows[1]!, "Abort upload");
    expect([...root.querySelectorAll<HTMLElement>("details.scan li[data-upload]")].map((r) => r.dataset["upload"])).toEqual(["u-a"]);
  });
});

describe("the recovery wrappers", () => {
  it("parse the Rust shapes and refuse anything else", () => {
    expect(parseRecoveryList({ unavailable: null, items: [entry()] }).items[0]!.fileName).toBe("big.mov");
    expect(() => parseRecoveryList({ unavailable: null, items: [{ ...entry(), kind: "other" }] })).toThrow("invalid recovery item");
    expect(() => parseRecoveryList({ unavailable: null, items: [{ ...entry(), size: -1 }] })).toThrow("invalid recovery item");
    expect(parseInspection(inspection).openUploads[0]!.recorded).toBe(true);
    expect(() => parseInspection({ ...inspection, verdict: "fine" })).toThrow("invalid inspection");
    expect(() => parseInspection({ ...inspection, counts: { absent: "1" } })).toThrow("invalid inspection");
  });
});
