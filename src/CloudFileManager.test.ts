// Headless tests of the file manager in the bucket browser (ADR-026):
// selection, the context menu and the selection bar, reviewed operations
// (plan, review, start, events, retry, cancel), the destination picker,
// rename and new folders, search, sorting and filters, deleted items,
// properties with a safe preview and share links, downloads of several
// items, favorites and keyboard shortcuts. Tauri's IPC mock plays the Rust
// commands with their shapes; the real behaviour is tested in Rust against
// a local S3 server.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import CloudStorage from "./CloudStorage.svelte";
import { LIST_CLOUD_ACCOUNTS, LIST_CLOUD_FOLDER, LIST_CLOUD_PROVIDERS, type AccountInfo, type FileEntry, type FolderEntry } from "./cloud";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

const MODIFIED = "2026-11-11T12:30:00Z";

function account(): AccountInfo {
  return {
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
}

const f = (key: string, size = 1, lastModified = MODIFIED): FileEntry => ({
  name: key.slice(key.lastIndexOf("/") + 1),
  key,
  size,
  lastModified,
});
const d = (prefix: string): FolderEntry => ({ name: prefix.slice(0, -1).split("/").pop() ?? "", prefix });

let tree: Record<string, { folders: FolderEntry[]; files: FileEntry[] }>;
let calls: { cmd: string; args: Record<string, unknown> }[];
let handlers: Record<string, (args: Record<string, unknown>) => unknown>;
let app: ReturnType<typeof mount> | undefined;

function listing(prefix: string): unknown {
  const page = tree[prefix] ?? { folders: [], files: [] };
  const crumbs: { name: string; prefix: string }[] = [];
  let start = 0;
  for (let i = prefix.indexOf("/"); i !== -1; i = prefix.indexOf("/", i + 1)) {
    crumbs.push({ name: prefix.slice(start, i), prefix: prefix.slice(0, i + 1) });
    start = i + 1;
  }
  return {
    accountId: "photos",
    bucket: "soty-photos",
    prefix,
    parent: crumbs.length === 0 ? null : (crumbs.at(-2)?.prefix ?? ""),
    breadcrumbs: crumbs,
    folders: page.folders,
    files: page.files,
    nextPage: null,
  };
}

function plan(kind: string, items: Partial<Record<string, unknown>>[], extra: Record<string, unknown> = {}): unknown {
  const full = items.map((i, index) => ({
    index,
    name: "x",
    folder: false,
    source: "x",
    destination: null,
    objects: 1,
    bytes: 1,
    conflict: null,
    problem: null,
    ...i,
  }));
  return {
    plan: "plan-1",
    kind,
    accountId: "photos",
    bucket: "soty-photos",
    destination: null,
    items: full,
    objects: full.reduce((n, i) => n + (i.problem === null ? (i.objects as number) : 0), 0),
    bytes: 10,
    conflicts: full.filter((i) => i.conflict !== null).length,
    versioning: "enabled",
    ...extra,
  };
}

function report(index: number, name: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    index,
    name,
    folder: false,
    source: name,
    destination: null,
    state: "pending",
    objectsTotal: 1,
    objectsDone: 0,
    objectsFailed: 0,
    bytesTotal: 1,
    bytesDone: 0,
    hidden: 0,
    removed: 0,
    note: null,
    failures: [],
    ...extra,
  };
}

function summary(kind: string, state: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    operation: "op-1",
    kind,
    state,
    items: { completed: 1, skipped: 0, failed: 0, partial: 0, cancelled: 0 },
    objectsDone: 1,
    objectsFailed: 0,
    hidden: 0,
    removed: 0,
    retryable: false,
    error: null,
    ...extra,
  };
}

beforeEach(() => {
  tree = {
    "": {
      folders: [d("photos/"), d("docs/")],
      files: [f("b.txt", 300), f("a.jpg", 5000, "2026-01-01T00:00:00Z"), f("c.pdf", 20)],
    },
    "docs/": { folders: [d("docs/old/")], files: [f("docs/x.md")] },
    "photos/": { folders: [], files: [] },
  };
  calls = [];
  handlers = {};
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    const h = handlers[cmd];
    if (h) {
      return h(args);
    }
    switch (cmd) {
      case LIST_CLOUD_PROVIDERS:
        return [];
      case LIST_CLOUD_ACCOUNTS:
        return { accounts: [account()], storageError: null };
      case LIST_CLOUD_FOLDER:
        return listing(args["prefix"] as string);
      case "list_cloud_places":
        return { favorites: [], recent: [] };
      case "record_cloud_recent":
        return null;
      case "discard_cloud_operation_plan":
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
  for (let i = 0; i < 4; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function buttons(root: ParentNode): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>("button")];
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = buttons(root).find((b) => text(b) === name || b.getAttribute("aria-label") === name);
  if (!found) {
    throw new Error(`no button "${name}" in: ${buttons(root).map((b) => text(b) || b.getAttribute("aria-label")).join(", ")}`);
  }
  return found;
}

async function click(root: ParentNode, name: string, init: MouseEventInit = {}): Promise<void> {
  const b = button(root, name);
  b.dispatchEvent(new MouseEvent("click", { bubbles: true, ...init }));
  flushSync();
  await settle();
}

async function opened(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudStorage, { target });
  flushSync();
  await settle();
  const li = target.querySelector<HTMLElement>('li[data-account="photos"]');
  await click(li!, "Open bucket");
  return target;
}

function row(root: ParentNode, id: string): HTMLElement {
  const r = root.querySelector<HTMLElement>(`tr[data-id="${CSS.escape(id)}"]`);
  if (!r) {
    throw new Error(`no row ${id}`);
  }
  return r;
}

async function clickRow(root: ParentNode, id: string, init: MouseEventInit = {}): Promise<void> {
  row(root, id).querySelector("td:nth-child(3)")!.dispatchEvent(new MouseEvent("click", { bubbles: true, ...init }));
  flushSync();
  await settle();
}

function names(root: ParentNode): string[] {
  return [...root.querySelectorAll("section.browser tbody tr")].map((tr) => (tr as HTMLElement).dataset["id"] ?? "");
}

function selectedIds(root: ParentNode): string[] {
  return [...root.querySelectorAll("section.browser tbody tr.selected")].map((tr) => (tr as HTMLElement).dataset["id"] ?? "");
}

async function key(k: string, init: KeyboardEventInit = {}, target: EventTarget = document.body): Promise<void> {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  flushSync();
  await settle();
}

function dialog(root: ParentNode): HTMLElement {
  const el = root.querySelector<HTMLElement>('[role="dialog"]');
  if (!el) {
    throw new Error("no dialog open");
  }
  return el;
}

function argsOf(cmd: string): Record<string, unknown>[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.args);
}

describe("selection", () => {
  it("selects with click, Ctrl-click, Shift-click, the checkboxes and Ctrl+A, and clears with Escape", async () => {
    const root = await opened();
    expect(names(root)).toEqual(["photos/", "docs/", "b.txt", "a.jpg", "c.pdf"]);
    await clickRow(root, "b.txt");
    expect(selectedIds(root)).toEqual(["b.txt"]);
    await clickRow(root, "c.pdf", { ctrlKey: true });
    expect(selectedIds(root)).toEqual(["b.txt", "c.pdf"]);
    // The anchor is the last clicked row (c.pdf): the range runs from it.
    await clickRow(root, "docs/", { shiftKey: true });
    expect(selectedIds(root)).toEqual(["docs/", "b.txt", "a.jpg", "c.pdf"]);
    expect(text(root.querySelector('[aria-label="Selected items"]'))).toContain("4 selected");
    await key("a", { ctrlKey: true });
    expect(selectedIds(root)).toHaveLength(5);
    await key("Escape");
    expect(selectedIds(root)).toEqual([]);
    const box = root.querySelector<HTMLInputElement>('input[aria-label="Select all shown"]')!;
    box.click();
    flushSync();
    expect(selectedIds(root)).toHaveLength(5);
    root.querySelector<HTMLInputElement>('input[aria-label="Select a.jpg"]')!.click();
    flushSync();
    expect(selectedIds(root)).not.toContain("a.jpg");
  });

  it("leaves the keyboard to an open dialog", async () => {
    const root = await opened();
    await click(root, "New folder…");
    await key("a", { ctrlKey: true });
    expect(selectedIds(root)).toEqual([]);
  });

  it("copies with Ctrl+C and pastes with Ctrl+V only into a reviewed plan; a second Ctrl+V under the review does nothing", async () => {
    handlers["plan_cloud_operation"] = () => plan("copy", [{ name: "b.txt", source: "b.txt", destination: "docs/b.txt" }], { destination: "docs/" });
    const root = await opened();
    await clickRow(root, "b.txt");
    await key("c", { ctrlKey: true });
    expect(text(root)).toContain("1 item copied");
    expect([...root.querySelectorAll("button")].map((b) => b.textContent?.trim())).toContain("Paste copy (1)");
    // Nothing is sent by a copy to the clipboard.
    expect(argsOf("plan_cloud_operation")).toHaveLength(0);
    row(root, "docs/").querySelector("button")!.click();
    await settle();
    await key("v", { ctrlKey: true });
    expect(argsOf("plan_cloud_operation")).toHaveLength(1);
    expect(argsOf("plan_cloud_operation")[0]!["request"]).toEqual({ kind: "copy", items: [{ type: "file", key: "b.txt" }], destination: "docs/" });
    expect(text(dialog(root))).toContain("b.txt");
    await key("v", { ctrlKey: true });
    expect(argsOf("plan_cloud_operation")).toHaveLength(1);
    expect(argsOf("start_cloud_operation")).toHaveLength(0);
    await click(dialog(root), "Cancel");
    expect(argsOf("start_cloud_operation")).toHaveLength(0);
  });

  it("takes shortcuts again once a reviewed operation has finished and its panel is dismissed", async () => {
    handlers["plan_cloud_operation"] = () => plan("copy", [{ name: "b.txt", source: "b.txt", destination: "docs/b.txt" }], { destination: "docs/" });
    let channel: FakeChannel | undefined;
    handlers["start_cloud_operation"] = (args) => {
      channel = args["onEvent"] as FakeChannel;
      return { operation: "op-1", kind: "copy", items: [report(0, "b.txt")] };
    };
    const root = await opened();
    await clickRow(root, "b.txt");
    await key("c", { ctrlKey: true });
    row(root, "docs/").querySelector("button")!.click();
    await settle();
    await key("v", { ctrlKey: true });
    await click(dialog(root), "Copy");
    channel!.onmessage({ operation: "op-1", seq: 1, body: { type: "item", item: report(0, "b.txt", { state: "completed", objectsDone: 1 }) } });
    channel!.onmessage({ operation: "op-1", seq: 2, body: { type: "finished", summary: summary("copy", "completed") } });
    await settle();
    expect(root.querySelector('[role="dialog"]')).toBeNull();
    await click(root.querySelector<HTMLElement>('[aria-label="File operation"]')!, "Dismiss");
    await click(root, "Up one folder");
    await clickRow(root, "b.txt");
    await key("x", { ctrlKey: true });
    expect(text(root)).toContain("marked to move");
  });

  it("does not take shortcuts while typing in the search field", async () => {
    const root = await opened();
    const input = root.querySelector<HTMLInputElement>('input[aria-label="Search"]')!;
    await key("a", { ctrlKey: true }, input);
    expect(selectedIds(root)).toEqual([]);
  });

  it("keeps only what is still there after navigating", async () => {
    const root = await opened();
    await clickRow(root, "b.txt");
    row(root, "docs/").querySelector("button")!.click();
    await settle();
    expect(selectedIds(root)).toEqual([]);
    expect(names(root)).toEqual(["docs/old/", "docs/x.md"]);
  });
});

describe("delete", () => {
  it("reviews the plan with the object count and what deleting means, then runs it and refreshes", async () => {
    handlers["plan_cloud_operation"] = () =>
      plan("delete", [{ name: "docs", folder: true, source: "docs/", objects: 3 }, { name: "b.txt", source: "b.txt" }]);
    let channel: FakeChannel | undefined;
    handlers["start_cloud_operation"] = (args) => {
      channel = args["onEvent"] as FakeChannel;
      return { operation: "op-1", kind: "delete", items: [report(0, "docs", { folder: true, objectsTotal: 3 }), report(1, "b.txt")] };
    };
    const root = await opened();
    await clickRow(root, "docs/");
    await clickRow(root, "b.txt", { ctrlKey: true });
    await key("Delete");
    const request = argsOf("plan_cloud_operation")[0]!;
    expect(request["bucket"]).toBe("soty-photos");
    expect(request["request"]).toEqual({
      kind: "delete",
      items: [
        { type: "folder", prefix: "docs/" },
        { type: "file", key: "b.txt" },
      ],
    });
    const dlg = dialog(root);
    expect(text(dlg.querySelector("h4"))).toBe("Delete 2 items?");
    expect(text(dlg)).toContain("4 objects");
    expect(text(dlg)).toContain("can be restored from Deleted items");
    expect(argsOf("start_cloud_operation")).toHaveLength(0);
    await click(dlg, "Delete");
    expect(argsOf("start_cloud_operation")[0]).toMatchObject({ plan: "plan-1", conflicts: "skip" });
    const before = argsOf(LIST_CLOUD_FOLDER).length;
    channel!.onmessage({ operation: "op-1", seq: 1, body: { type: "progress", objectsDone: 2, objectsTotal: 4, bytesDone: 0, bytesTotal: 0 } });
    channel!.onmessage({ operation: "op-other", seq: 2, body: { type: "finished", summary: summary("delete", "failed") } });
    flushSync();
    const panel = root.querySelector<HTMLElement>('[aria-label="File operation"]')!;
    expect(text(panel)).toContain("2 of 4 objects");
    channel!.onmessage({
      operation: "op-1",
      seq: 3,
      body: { type: "item", item: report(0, "docs", { folder: true, state: "completed", objectsTotal: 3, objectsDone: 3, hidden: 3 }) },
    });
    channel!.onmessage({
      operation: "op-1",
      seq: 4,
      body: { type: "item", item: report(1, "b.txt", { state: "completed", objectsDone: 1, removed: 1 }) },
    });
    channel!.onmessage({ operation: "op-1", seq: 5, body: { type: "finished", summary: summary("delete", "completed", { hidden: 3, removed: 1, items: { completed: 2, skipped: 0, failed: 0, partial: 0, cancelled: 0 } }) } });
    // A stale (older) event after the end changes nothing.
    channel!.onmessage({ operation: "op-1", seq: 2, body: { type: "item", item: report(1, "b.txt", { state: "running" }) } });
    await settle();
    expect(text(panel)).toContain("Finished: 2 done.");
    expect(text(panel)).toContain("3 objects can be restored from Deleted items.");
    expect(text(panel)).toContain("1 object is gone for good.");
    expect(text(panel)).toContain("Deleted for good");
    expect(argsOf(LIST_CLOUD_FOLDER).length).toBeGreaterThan(before);
  });

  it("warns that deleting is final on a bucket without versions, and Cancel discards the plan", async () => {
    handlers["plan_cloud_operation"] = () => plan("delete", [{ name: "b.txt", source: "b.txt" }], { versioning: "disabled" });
    const root = await opened();
    await clickRow(root, "b.txt");
    await click(root.querySelector('[aria-label="Selected items"]')!, "Delete");
    expect(text(dialog(root))).toContain("gone for good and cannot be restored");
    await click(dialog(root), "Cancel");
    expect(root.querySelector('[role="dialog"]')).toBeNull();
    expect(argsOf("discard_cloud_operation_plan")).toEqual([{ plan: "plan-1" }]);
    expect(argsOf("start_cloud_operation")).toHaveLength(0);
  });

  it("shows a partial failure with each failed object and offers Try again only when Rust says so", async () => {
    handlers["plan_cloud_operation"] = () => plan("move", [{ name: "b.txt", source: "b.txt", destination: "docs/b.txt" }], { destination: "docs/" });
    let channel: FakeChannel | undefined;
    handlers["start_cloud_operation"] = (args) => {
      channel = args["onEvent"] as FakeChannel;
      return { operation: "op-1", kind: "move", items: [report(0, "b.txt")] };
    };
    handlers["retry_cloud_operation"] = () => ({ operation: "op-2", kind: "move", items: [report(0, "b.txt")] });
    const root = await opened();
    await clickRow(root, "b.txt");
    await key("x", { ctrlKey: true });
    row(root, "docs/").querySelector("button")!.click();
    await settle();
    await click(root, "Move here (1)");
    expect(argsOf("plan_cloud_operation")[0]!["request"]).toEqual({ kind: "move", items: [{ type: "file", key: "b.txt" }], destination: "docs/" });
    expect(text(dialog(root))).toContain("deleted only once all copies are verified");
    await click(dialog(root), "Move");
    channel!.onmessage({
      operation: "op-1",
      seq: 1,
      body: {
        type: "item",
        item: report(0, "b.txt", {
          state: "partial",
          note: "copied-original-kept",
          objectsFailed: 1,
          failures: [{ key: "b.txt", error: { kind: "access-denied", message: "the provider refused this change with this key" } }],
        }),
      },
    });
    channel!.onmessage({
      operation: "op-1",
      seq: 2,
      body: { type: "finished", summary: summary("move", "failed", { retryable: true, items: { completed: 0, skipped: 0, failed: 0, partial: 1, cancelled: 0 }, error: { kind: "access-denied", message: "refused" } }) },
    });
    await settle();
    const panel = root.querySelector<HTMLElement>('[aria-label="File operation"]')!;
    expect(text(panel)).toContain("Copied and verified, but the original could not be deleted; it is still there");
    expect(text(panel)).toContain("Stopped after an error");
    await click(panel, "Try again");
    expect(argsOf("retry_cloud_operation")[0]).toMatchObject({ operation: "op-1" });
  });
});

describe("copy, move and rename", () => {
  it("picks a destination in the folder picker, shows conflicts and keeps both on request", async () => {
    handlers["plan_cloud_operation"] = () =>
      plan("copy", [{ name: "b.txt", source: "b.txt", destination: "docs/b.txt", conflict: { existing: "docs/b.txt", alternative: "docs/b (1).txt" } }], {
        destination: "docs/",
      });
    handlers["start_cloud_operation"] = () => ({ operation: "op-1", kind: "copy", items: [report(0, "b.txt")] });
    const root = await opened();
    await clickRow(root, "b.txt");
    await click(root.querySelector('[aria-label="Selected items"]')!, "Copy to…");
    const picker = dialog(root);
    expect(text(picker.querySelector("h4"))).toBe("Copy 1 item to…");
    await click(picker, "📁 docs");
    expect(text(dialog(root).querySelector('[aria-current="location"]'))).toBe("docs");
    await click(dialog(root), "Copy to “docs”");
    expect(argsOf("plan_cloud_operation")[0]!["request"]).toEqual({ kind: "copy", items: [{ type: "file", key: "b.txt" }], destination: "docs/" });
    const review = dialog(root);
    expect(text(review)).toContain("Already exists at docs/b.txt");
    expect(text(review)).toContain("“Keep both” saves it as docs/b (1).txt");
    expect(button(review, "Copy").disabled).toBe(true);
    review.querySelector<HTMLInputElement>('input[value="rename"]')!.click();
    flushSync();
    await click(review, "Copy");
    expect(argsOf("start_cloud_operation")[0]).toMatchObject({ plan: "plan-1", conflicts: "rename" });
  });

  it("creates a folder in the picker", async () => {
    handlers["create_cloud_folder"] = (args) => {
      tree[`${args["prefix"] as string}${args["name"] as string}/`] = { folders: [], files: [] };
      return { name: args["name"], prefix: `${args["prefix"] as string}${args["name"] as string}/` };
    };
    const root = await opened();
    await clickRow(root, "a.jpg");
    await click(root.querySelector('[aria-label="Selected items"]')!, "Move to…");
    await click(dialog(root), "New folder…");
    const input = dialog(root).querySelector<HTMLInputElement>(".create input")!;
    input.value = "Neu";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    await click(dialog(root), "Create");
    expect(argsOf("create_cloud_folder")[0]).toEqual({ id: "photos", bucket: "soty-photos", prefix: "", name: "Neu" });
    expect(text(dialog(root).querySelector('[aria-current="location"]'))).toBe("Neu");
  });

  it("renames with F2 through a reviewed plan", async () => {
    handlers["plan_cloud_operation"] = () => plan("rename", [{ name: "c.pdf", source: "c.pdf", destination: "d.pdf" }]);
    const root = await opened();
    await clickRow(root, "c.pdf");
    await key("F2");
    const input = dialog(root).querySelector<HTMLInputElement>("input")!;
    expect(input.value).toBe("c.pdf");
    input.value = "d.pdf";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    await click(dialog(root), "Rename");
    expect(argsOf("plan_cloud_operation")[0]!["request"]).toEqual({ kind: "rename", item: { type: "file", key: "c.pdf" }, newName: "d.pdf" });
    expect(text(dialog(root))).toContain("From c.pdf to d.pdf");
  });

  it("shows a plan error such as an existing name", async () => {
    handlers["plan_cloud_operation"] = () => Promise.reject({ kind: "destination-exists", message: "'b.txt' already exists in this folder; choose another name" });
    const root = await opened();
    await clickRow(root, "c.pdf");
    await key("F2");
    const input = dialog(root).querySelector<HTMLInputElement>("input")!;
    input.value = "b.txt";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    await click(dialog(root), "Rename");
    expect(text(root.querySelector('[aria-label="File operation"]'))).toContain("already exists in this folder");
  });
});

describe("new folder", () => {
  it("creates the marker in the folder shown and selects it", async () => {
    handlers["create_cloud_folder"] = (args) => {
      tree[""]!.folders.push(d("Neu/"));
      return { name: args["name"], prefix: "Neu/" };
    };
    const root = await opened();
    await click(root, "New folder…");
    const input = dialog(root).querySelector<HTMLInputElement>("input")!;
    input.value = "Neu";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    await click(dialog(root), "Create folder");
    expect(argsOf("create_cloud_folder")[0]).toEqual({ id: "photos", bucket: "soty-photos", prefix: "", name: "Neu" });
    expect(selectedIds(root)).toEqual(["Neu/"]);
  });
});

describe("search, sorting and filters", () => {
  it("searches recursively, shows where each hit is and returns to the folder", async () => {
    handlers["search_cloud_folder"] = () => ({
      hits: [{ folder: false, name: "x.md", path: "docs/x.md", parent: "docs/", size: 9, lastModified: MODIFIED }],
      scanned: 120,
      next: null,
    });
    const root = await opened();
    const input = root.querySelector<HTMLInputElement>('input[aria-label="Search"]')!;
    input.value = "x.m";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    await click(root, "Search");
    expect(argsOf("search_cloud_folder")[0]).toEqual({ id: "photos", bucket: "soty-photos", prefix: "", query: "x.m", mode: "name", cursor: null });
    expect(names(root)).toEqual(["docs/x.md"]);
    expect(text(row(root, "docs/x.md"))).toContain("in docs/");
    expect(text(root)).toContain("120 objects looked at");
    expect(text(root)).toContain("S3 cannot search file contents");
    await click(root, "Clear search");
    expect(names(root)).toEqual(["photos/", "docs/", "b.txt", "a.jpg", "c.pdf"]);
  });

  it("sorts on the client and keeps folders first; filters hide what does not match", async () => {
    const root = await opened();
    const sort = root.querySelector<HTMLSelectElement>(".listbar select")!;
    sort.value = "size";
    sort.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    // Folders have no size: they tie and fall back to their names.
    expect(names(root)).toEqual(["docs/", "photos/", "c.pdf", "b.txt", "a.jpg"]);
    await click(root, "↑ Ascending");
    expect(names(root)).toEqual(["docs/", "photos/", "a.jpg", "b.txt", "c.pdf"]);
    await click(root, "Filter");
    root.querySelector<HTMLInputElement>('input[name="show"][value="files"]')!.click();
    flushSync();
    expect(names(root)).toEqual(["a.jpg", "b.txt", "c.pdf"]);
    const image = [...root.querySelectorAll<HTMLLabelElement>(".cats label")].find((l) => text(l) === "image")!;
    image.querySelector("input")!.click();
    flushSync();
    expect(names(root)).toEqual(["a.jpg"]);
    await click(root, "Show everything");
    expect(names(root)).toHaveLength(5);
  });
});

describe("deleted items", () => {
  it("lists what can and cannot be restored and restores through a reviewed plan", async () => {
    handlers["list_cloud_deleted"] = () => ({
      entries: [
        { key: "old.txt", name: "old.txt", parent: "", deletedAt: MODIFIED, markerVersion: "m1", restorable: true, size: 4, versions: 2 },
        { key: "gone.txt", name: "gone.txt", parent: "", deletedAt: MODIFIED, markerVersion: "m2", restorable: false, size: null, versions: 0 },
      ],
      truncated: false,
    });
    handlers["plan_cloud_operation"] = () => plan("restore", [{ name: "old.txt", source: "old.txt" }]);
    const root = await opened();
    await click(root, "Deleted items");
    expect(text(root)).toContain("Yes (2 versions kept)");
    expect(text(root)).toContain("No: no earlier version is kept");
    expect(root.querySelector<HTMLInputElement>('input[aria-label="Select gone.txt"]')!.disabled).toBe(true);
    await click(root, "Restore old.txt");
    expect(argsOf("plan_cloud_operation")[0]!["request"]).toEqual({ kind: "restore", items: [{ key: "old.txt", markerVersion: "m1" }] });
    expect(text(dialog(root))).toContain("removes the delete markers");
  });

  // Audit fix (2026-11-14): the folder's selection stayed active behind the
  // Deleted items view, so Delete or Ctrl+A acted on entries not shown.
  it("drops the folder's selection and ignores file shortcuts while Deleted items are shown", async () => {
    handlers["list_cloud_deleted"] = () => ({ entries: [], truncated: false });
    const root = await opened();
    await clickRow(root, "b.txt");
    expect(selectedIds(root)).toEqual(["b.txt"]);
    await click(root, "Deleted items");
    await key("Delete");
    await key("F2");
    await key("a", { ctrlKey: true });
    await key("x", { ctrlKey: true });
    expect(argsOf("plan_cloud_operation")).toEqual([]);
    expect(root.querySelector('[role="dialog"]')).toBeNull();
    expect(text(root)).not.toContain("marked to move");
    await click(root, "Deleted items");
    expect(selectedIds(root)).toEqual([]);
  });

  it("says when versions cannot be listed with this key", async () => {
    handlers["list_cloud_deleted"] = () => Promise.reject({ kind: "access-denied", message: "the provider refused to list this folder with this key" });
    const root = await opened();
    await click(root, "Deleted items");
    expect(text(root.querySelector(".deleted [role=alert]"))).toContain("Deleted items cannot be shown");
  });
});

describe("properties, preview and share links", () => {
  const props = {
    key: "b.txt",
    name: "b.txt",
    folder: "",
    bucket: "soty-photos",
    size: 300,
    lastModified: MODIFIED,
    contentType: "text/html",
    eTag: "abc",
    versionId: "v1",
    storageClass: null,
    cacheControl: null,
    contentEncoding: null,
    contentDisposition: null,
    sha256: null,
    metadata: [["owner", "me"]],
  };

  it("shows properties and a text preview as text, never as markup", async () => {
    handlers["cloud_object_properties"] = () => props;
    handlers["preview_cloud_file"] = () => ({ kind: "text", size: 300, mime: null, text: "<b>bold?</b><script>alert(1)</script>", data: null, truncated: false });
    const root = await opened();
    row(root, "b.txt").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    const dlg = dialog(root);
    expect(text(dlg)).toContain("text/html");
    expect(text(dlg)).toContain("Metadata: owner");
    expect(text(dlg)).toContain("None: not uploaded by sotyware");
    const pre = dlg.querySelector("pre")!;
    expect(pre.textContent).toBe("<b>bold?</b><script>alert(1)</script>");
    expect(dlg.querySelector("b, script")).toBeNull();
    expect(argsOf("record_cloud_recent")[0]).toMatchObject({ path: "b.txt" });
  });

  it("refuses an image preview of a type that is not allowed", async () => {
    handlers["cloud_object_properties"] = () => ({ ...props, key: "a.jpg", name: "a.jpg" });
    handlers["preview_cloud_file"] = () => ({ kind: "image", size: 3, mime: "image/svg+xml", text: null, data: "PHN2Zz4=", truncated: false });
    const root = await opened();
    row(root, "a.jpg").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    expect(dialog(root).querySelector("img")).toBeNull();
    expect(text(dialog(root))).toContain("No preview");
  });

  it("shows an allowed image from its data", async () => {
    handlers["cloud_object_properties"] = () => ({ ...props, key: "a.jpg", name: "a.jpg" });
    handlers["preview_cloud_file"] = () => ({ kind: "image", size: 3, mime: "image/png", text: null, data: "iVBORw==", truncated: false });
    const root = await opened();
    row(root, "a.jpg").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    expect(dialog(root).querySelector("img")!.getAttribute("src")).toBe("data:image/png;base64,iVBORw==");
  });

  it("creates a share link with the chosen expiry and says who can use it", async () => {
    handlers["cloud_object_properties"] = () => props;
    handlers["preview_cloud_file"] = () => ({ kind: "unsupported", size: 300, mime: null, text: null, data: null, truncated: false });
    handlers["share_cloud_file"] = (args) => ({
      url: "https://s3.example/soty-photos/b.txt?X-Amz-Signature=x",
      expiresAt: "2026-11-12T12:00:00Z",
      expiresIn: args["expiresIn"],
    });
    const root = await opened();
    row(root, "b.txt").dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await settle();
    const dlg = dialog(root);
    expect(text(dlg)).toContain("Anyone who has the link can download this file until it expires");
    const sel = dlg.querySelector<HTMLSelectElement>(".share select")!;
    sel.value = "86400";
    sel.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    await click(dlg, "Create link");
    expect(argsOf("share_cloud_file")[0]).toEqual({ id: "photos", bucket: "soty-photos", key: "b.txt", expiresIn: 86400 });
    expect(dlg.querySelector<HTMLInputElement>('input[aria-label="Download link"]')!.value).toContain("X-Amz-Signature");
    expect(text(dlg)).toContain("Expires");
  });
});

describe("downloads of several items", () => {
  it("downloads a folder and files into a folder Rust asks for, with progress and a summary", async () => {
    let channel: FakeChannel | undefined;
    handlers["download_cloud_items"] = (args) => {
      channel = args["onEvent"] as FakeChannel;
      return {
        batch: "bulk-1",
        folderName: "Downloads",
        items: [
          { index: 0, name: "docs", folder: true, files: 2, bytes: 10, state: "pending", savedAs: null, error: null },
          { index: 1, name: "b.txt", folder: false, files: 1, bytes: 300, state: "pending", savedAs: null, error: null },
        ],
        files: [],
      };
    };
    const root = await opened();
    await clickRow(root, "docs/");
    await clickRow(root, "b.txt", { ctrlKey: true });
    await click(root.querySelector('[aria-label="Selected items"]')!, "Download");
    expect(argsOf("download_cloud_items")[0]!["items"]).toEqual([
      { type: "folder", prefix: "docs/" },
      { type: "file", key: "b.txt" },
    ]);
    const panel = root.querySelector<HTMLElement>('[aria-label="Download of several items"]')!;
    expect(text(panel)).toContain("into the folder “Downloads”");
    channel!.onmessage({ batch: "bulk-1", seq: 1, body: { type: "item", item: { index: 0, name: "docs", folder: true, files: 2, bytes: 10, state: "saved", savedAs: "docs (1)", error: null } } });
    channel!.onmessage({ batch: "bulk-1", seq: 2, body: { type: "finished", summary: { batch: "bulk-1", state: "completed", itemsSaved: 2, itemsFailed: 0, itemsCancelled: 0, filesCompleted: 3, bytes: 310, retryable: false, error: null } } });
    await settle();
    expect(text(panel)).toContain("Saved as “docs (1)” (the name was taken; nothing was replaced)");
    expect(text(panel)).toContain("Download finished: 2 saved");
  });

  it("keeps the single-file save dialog for one file", async () => {
    handlers["download_cloud_file"] = () => null;
    const root = await opened();
    await clickRow(root, "c.pdf");
    await click(root.querySelector('[aria-label="Selected items"]')!, "Download");
    expect(argsOf("download_cloud_file")[0]).toMatchObject({ key: "c.pdf" });
    expect(argsOf("download_cloud_items")).toHaveLength(0);
  });
});

describe("context menu and favorites", () => {
  it("opens on right-click with the actions for the selection and marks favorites", async () => {
    handlers["set_cloud_favorite"] = (args) => ({
      favorites: [{ path: args["path"], bucket: "soty-photos", folder: true, name: "docs", parent: "", touched: 1 }],
      recent: [],
    });
    const root = await opened();
    row(root, "docs/").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 10, clientY: 10 }));
    flushSync();
    const menu = document.querySelector<HTMLElement>('[role="menu"]')!;
    expect(buttons(menu).map((b) => text(b))).toContain("Open");
    expect(buttons(menu).map((b) => text(b))).toContain("Delete (Del)");
    await click(menu, "Add to favorites");
    expect(argsOf("set_cloud_favorite")[0]).toEqual({ id: "photos", bucket: "soty-photos", path: "docs/", favorite: true });
    expect(text(root.querySelector(".places summary"))).toBe("Favorites (1) and recent (0)");
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  // Found by the Linux desktop driver (2026-11-14): WebKitGTK also sends a
  // "click" for the right button that opened the menu, which closed it.
  it("stays open after the right button's click and closes on a primary click elsewhere", async () => {
    const root = await opened();
    row(root, "docs/").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 10, clientY: 10 }));
    flushSync();
    window.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 2 }));
    flushSync();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true, button: 0 }));
    flushSync();
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });
});

describe("folder uploads and drag and drop", () => {
  it("asks Rust for a folder upload into the folder shown", async () => {
    handlers["upload_cloud_folder"] = () => null;
    const root = await opened();
    row(root, "docs/").querySelector("button")!.click();
    await settle();
    await click(root, "Upload folder…");
    expect(argsOf("upload_cloud_folder")[0]).toMatchObject({ id: "photos", bucket: "soty-photos", prefix: "docs/" });
  });

  it("announces a drop by names only and uploads it on request", async () => {
    let watcher: FakeChannel | undefined;
    handlers["watch_cloud_drops"] = (args) => {
      watcher = args["onDrop"] as FakeChannel;
      return null;
    };
    handlers["upload_cloud_drop"] = () => ({ batch: "batch-1", files: [] });
    const root = await opened();
    expect(watcher).toBeDefined();
    watcher!.onmessage({ drop: 7, files: 2, folders: 1, names: ["a.txt", "b.txt", "Album"] });
    flushSync();
    const banner = root.querySelector<HTMLElement>('[aria-label="Dropped files"]')!;
    expect(text(banner)).toContain("2 files and 1 folder");
    expect(text(banner)).toContain("a.txt, b.txt, Album");
    await click(banner, "Upload here");
    expect(argsOf("upload_cloud_drop")[0]).toMatchObject({ id: "photos", bucket: "soty-photos", prefix: "", drop: 7 });
    expect(root.querySelector('[aria-label="Dropped files"]')).toBeNull();
    // Leaving the browser stops listening.
    await click(root, "← Back to accounts");
    expect(argsOf("unwatch_cloud_drops")).toHaveLength(1);
  });
});
