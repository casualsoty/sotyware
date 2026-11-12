// Headless tests of the read-only bucket browser (CloudBrowser.svelte), opened
// from the Cloud Storage page as a user would. Tauri's IPC mock plays
// `list_cloud_folder` (src-tauri/src/cloud/browser.rs) with the same shapes;
// the real listing, key rules and pagination are tested in Rust.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import CloudStorage from "./CloudStorage.svelte";
import {
  CANCEL_CLOUD_DOWNLOAD,
  DOWNLOAD_CLOUD_FILE,
  LIST_CLOUD_ACCOUNTS,
  LIST_CLOUD_FOLDER,
  RETRY_CLOUD_DOWNLOAD,
  LIST_CLOUD_PROVIDERS,
  parseFolderListing,
  type AccountInfo,
  type FileEntry,
  type FolderEntry,
  type FolderListing,
} from "./cloud";

function account(overrides: Partial<AccountInfo> = {}): AccountInfo {
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
    ...overrides,
  };
}

const MODIFIED = "2026-11-11T12:30:00Z";

function file(key: string, size = 1, name = key.slice(key.lastIndexOf("/") + 1)): FileEntry {
  return { name, key, size, lastModified: MODIFIED };
}

function folder(prefix: string, name: string): FolderEntry {
  return { name, prefix };
}

/** The pages of each `account/prefix`, as the fake Rust side serves them. */
type Tree = Record<string, Record<string, { folders: FolderEntry[]; files: FileEntry[] }[]>>;

interface FolderCall {
  id: string;
  prefix: string;
  page: string | null;
}

let accounts: AccountInfo[];
let tree: Tree;
let folderCalls: FolderCall[];
let listCalls: number;
/** Replaces the fake for the next `list_cloud_folder` calls, in order. */
let queued: ((call: FolderCall) => unknown)[];
let app: ReturnType<typeof mount> | undefined;

/** The channel a download command was given (a real `Channel`; the mock
 * passes arguments through, so tests call its handler directly). */
interface FakeChannel {
  onmessage: (message: unknown) => void;
}

interface DownloadCall {
  id: string;
  bucket: string;
  key: string;
  channel: FakeChannel;
}

let downloadCalls: DownloadCall[];
let retryCalls: { transfer: string; channel: FakeChannel }[];
let cancelCalls: string[];
/** What `download_cloud_file` answers (default: started as download-N). */
let onDownload: (call: DownloadCall) => unknown;
let onRetry: (transfer: string) => unknown;
let onCancel: (transfer: string) => unknown;

function crumbsOf(prefix: string): { name: string; prefix: string }[] {
  // Test data only: the component never splits prefixes itself.
  const crumbs = [];
  let start = 0;
  for (let i = prefix.indexOf("/"); i !== -1; i = prefix.indexOf("/", i + 1)) {
    crumbs.push({ name: prefix.slice(start, i), prefix: prefix.slice(0, i + 1) });
    start = i + 1;
  }
  return crumbs;
}

function serve(call: FolderCall): FolderListing | Promise<never> {
  const acct = accounts.find((a) => a.id === call.id);
  if (!acct) {
    return Promise.reject({ kind: "unknown-account", message: `no account '${call.id}'` });
  }
  const pages = tree[call.id]?.[call.prefix] ?? [{ folders: [], files: [] }];
  const index = call.page === null ? 0 : Number(call.page.slice(`${call.id}:${call.prefix}:`.length));
  const page = pages[index];
  if (call.page !== null && (!call.page.startsWith(`${call.id}:${call.prefix}:`) || !page)) {
    return Promise.reject({ kind: "outdated-page", message: "this page of the folder is no longer available" });
  }
  const crumbs = crumbsOf(call.prefix);
  return {
    accountId: call.id,
    bucket: acct.bucket ?? "",
    prefix: call.prefix,
    parent: crumbs.length === 0 ? null : (crumbs.at(-2)?.prefix ?? ""),
    breadcrumbs: crumbs,
    folders: page?.folders ?? [],
    files: page?.files ?? [],
    nextPage: index + 1 < pages.length ? `${call.id}:${call.prefix}:${index + 1}` : null,
  };
}

beforeEach(() => {
  accounts = [account()];
  tree = {};
  folderCalls = [];
  listCalls = 0;
  queued = [];
  downloadCalls = [];
  retryCalls = [];
  cancelCalls = [];
  onDownload = (call) => ({ transfer: `download-${downloadCalls.length}`, fileName: call.key.split("/").pop() ?? "" });
  onRetry = () => ({ transfer: `download-retry-${retryCalls.length}`, fileName: "chosen.bin" });
  onCancel = () => "cancelling";
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case LIST_CLOUD_PROVIDERS:
        return [];
      case LIST_CLOUD_ACCOUNTS:
        listCalls++;
        return { accounts, storageError: null };
      case LIST_CLOUD_FOLDER: {
        const call = { id: args["id"] as string, prefix: args["prefix"] as string, page: args["page"] as string | null };
        folderCalls.push(call);
        const next = queued.shift();
        return next ? next(call) : serve(call);
      }
      case DOWNLOAD_CLOUD_FILE: {
        const call = {
          id: args["id"] as string,
          bucket: args["bucket"] as string,
          key: args["key"] as string,
          channel: args["onEvent"] as FakeChannel,
        };
        downloadCalls.push(call);
        return onDownload(call);
      }
      case RETRY_CLOUD_DOWNLOAD: {
        const transfer = args["transfer"] as string;
        retryCalls.push({ transfer, channel: args["onEvent"] as FakeChannel });
        return onRetry(transfer);
      }
      case CANCEL_CLOUD_DOWNLOAD: {
        const transfer = args["transfer"] as string;
        cancelCalls.push(transfer);
        return onCancel(transfer);
      }
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
  for (let i = 0; i < 3; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudStorage, { target });
  flushSync();
  await settle();
  return target;
}

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function buttons(root: ParentNode): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>("button")];
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = buttons(root).find((b) => text(b) === name);
  if (!found) {
    throw new Error(`no button "${name}" in: ${buttons(root).map((b) => text(b)).join(", ")}`);
  }
  return found;
}

async function click(root: ParentNode, name: string): Promise<void> {
  button(root, name).click();
  flushSync();
  await settle();
}

function browser(root: HTMLElement): HTMLElement {
  const el = root.querySelector<HTMLElement>("section.browser");
  if (!el) {
    throw new Error(`browser not open: ${root.innerHTML}`);
  }
  return el;
}

async function openBucket(root: HTMLElement, id = "photos"): Promise<HTMLElement> {
  const li = root.querySelector<HTMLElement>(`li[data-account="${id}"]`);
  if (!li) {
    throw new Error(`no account ${id}`);
  }
  await click(li, "Open bucket");
  return browser(root);
}

/** Name, size and date of each row (the action column is checked by the
 * download tests). */
function rows(root: ParentNode): string[] {
  return [...root.querySelectorAll("tbody tr")].map((tr) =>
    [...tr.querySelectorAll("td:not(.row-actions):not(:last-child)")].map((td) => text(td)).join(" | "),
  );
}

function crumbs(root: ParentNode): string[] {
  return [...root.querySelectorAll("nav.crumbs li")].map((li) => text(li));
}

function current(root: ParentNode): string {
  return text(root.querySelector('nav.crumbs [aria-current="location"]'));
}

async function openFolder(root: ParentNode, prefix: string): Promise<void> {
  const row = root.querySelector<HTMLElement>(`tr[data-prefix="${CSS.escape(prefix)}"]`);
  const b = row?.querySelector("button");
  if (!b) {
    throw new Error(`no folder ${prefix}`);
  }
  b.click();
  flushSync();
  await settle();
}

function alerts(root: ParentNode): string[] {
  return [...root.querySelectorAll('[role="alert"]')].map((e) => text(e));
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const PHOTOS: Tree[string] = {
  "": [
    {
      folders: [folder("2024/", "2024"), folder("a//", "a/"), folder("Ferien 2024 ü/", "Ferien 2024 ü")],
      files: [file("readme.txt", 12), file("big.bin", 3 * 1024 * 1024)],
    },
  ],
  "2024/": [{ folders: [folder("2024/beach/", "beach")], files: [file("2024/cat.jpg", 2048)] }],
  "2024/beach/": [{ folders: [], files: [file("2024/beach/sun.jpg", 500)] }],
  "Ferien 2024 ü/": [{ folders: [], files: [file("Ferien 2024 ü/<b>not bold</b> & ok.txt", 3, "<b>not bold</b> & ok.txt")] }],
  "empty/": [{ folders: [], files: [] }],
};

describe("opening a bucket", () => {
  it("offers Open bucket per account, disabled when the settings cannot be used", async () => {
    accounts = [account(), account({ id: "old", label: "Old", configError: "unknown provider 'x'" })];
    const root = await render();
    const photos = root.querySelector<HTMLElement>('li[data-account="photos"]')!;
    expect(buttons(photos).map((b) => text(b))).toEqual(["Open bucket", "Check connection", "Edit", "Remove"]);
    expect(button(photos, "Open bucket").disabled).toBe(false);
    expect(button(root.querySelector<HTMLElement>('li[data-account="old"]')!, "Open bucket").disabled).toBe(true);
    expect(folderCalls).toHaveLength(0);
  });

  it("lists the root with folders first, sizes and dates, in place of the account list", async () => {
    tree = { photos: PHOTOS };
    const root = await render();
    const b = await openBucket(root);
    expect(folderCalls).toEqual([{ id: "photos", prefix: "", page: null }]);
    expect(root.querySelector("li[data-account]")).toBeNull();
    expect(root.querySelector("button.primary")).toBeNull(); // no Add account while browsing
    expect(text(b.querySelector("h3"))).toBe("Photos");
    expect(text(b.querySelector(".bucket"))).toBe("soty-photos");
    expect(text(b.querySelector(".path"))).toBe("Root of the bucket");
    expect(crumbs(b)).toEqual(["soty-photos"]);
    expect(current(b)).toBe("soty-photos");
    expect(button(b, "Up one folder").disabled).toBe(true);
    const shown = rows(b);
    expect(shown.slice(0, 3)).toEqual(["📁 2024 | Folder | —", "📁 a/ | Folder | —", "📁 Ferien 2024 ü | Folder | —"]);
    expect(shown[3]).toMatch(/^readme\.txt \| 12 B \| .*2026/);
    expect(shown[4]).toMatch(/^big\.bin \| 3\.0 MiB \| /);
    expect(b.querySelector('tr[data-key="big.bin"] td.num')?.getAttribute("title")).toBe(
      `${(3 * 1024 * 1024).toLocaleString()} bytes`,
    );
    expect(text(b.querySelector(".count"))).toBe("5 items shown.");
    expect(buttons(b).map((x) => text(x))).not.toContain("Load more");
  });

  it("shows loading while the folder is listed", async () => {
    const pending = deferred<unknown>();
    queued.push(() => pending.promise);
    const root = await render();
    const b = await openBucket(root);
    expect(text(b.querySelector('[role="status"]'))).toBe("Loading folder…");
    expect(b.querySelector("table")).toBeNull();
    pending.resolve(serve({ id: "photos", prefix: "", page: null }));
    await settle();
    expect(b.querySelector('[role="status"].loading')).toBeNull();
    expect(text(b)).toContain("This bucket is empty.");
  });

  it("shows a clear empty state for an empty bucket and an empty folder", async () => {
    tree = { photos: { "": [{ folders: [folder("empty/", "empty")], files: [] }], "empty/": [{ folders: [], files: [] }] } };
    const root = await render();
    const b = await openBucket(root);
    expect(text(b)).not.toContain("is empty");
    await openFolder(b, "empty/");
    expect(text(b.querySelector(".empty"))).toContain("This folder is empty.");
    expect(text(b.querySelector(".empty"))).toContain("folder marker");
    expect(b.querySelector("table")).toBeNull();
  });
});

describe("navigation", () => {
  it("opens folders by the prefix Rust returned and goes back through breadcrumbs and Up", async () => {
    tree = { photos: PHOTOS };
    const root = await render();
    const b = await openBucket(root);
    await openFolder(b, "2024/");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "2024/", page: null });
    expect(text(b.querySelector(".path"))).toBe("2024/");
    expect(rows(b)[0]).toBe("📁 beach | Folder | —");
    expect(rows(b)[1]).toMatch(/^cat\.jpg \| 2\.0 KiB/);
    await openFolder(b, "2024/beach/");
    expect(crumbs(b)).toEqual(["soty-photos", "2024", "beach"]);
    expect(current(b)).toBe("beach");
    expect(rows(b)[0]).toMatch(/^sun\.jpg/);

    // Breadcrumb to a middle folder.
    await click(b.querySelector("nav.crumbs")!, "2024");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "2024/", page: null });
    expect(crumbs(b)).toEqual(["soty-photos", "2024"]);
    // Up to the root, then the root breadcrumb is not a button.
    await click(b, "Up one folder");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: null });
    expect(crumbs(b)).toEqual(["soty-photos"]);
    expect(b.querySelector("nav.crumbs button")).toBeNull();
    // The bucket breadcrumb goes to the root from anywhere.
    await openFolder(b, "2024/");
    await click(b.querySelector("nav.crumbs")!, "soty-photos");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: null });
  });

  it("keeps odd keys exactly and shows them as text", async () => {
    tree = { photos: { ...PHOTOS, "a//": [{ folders: [folder("a///", "")], files: [] }] } };
    const root = await render();
    const b = await openBucket(root);
    await openFolder(b, "a//");
    expect(folderCalls.at(-1)?.prefix).toBe("a//");
    expect(rows(b)).toEqual(["📁 (no name) | Folder | —"]);
    await openFolder(b, "a///");
    expect(folderCalls.at(-1)?.prefix).toBe("a///");
    expect(crumbs(b)).toEqual(["soty-photos", "a", "(no name)", "(no name)"]);

    await click(b.querySelector("nav.crumbs")!, "soty-photos");
    await openFolder(b, "Ferien 2024 ü/");
    expect(folderCalls.at(-1)?.prefix).toBe("Ferien 2024 ü/");
    const name = b.querySelector("tr.file td.name");
    expect(name?.textContent).toBe("<b>not bold</b> & ok.txt");
    expect(name?.querySelector("b")).toBeNull();
    expect(name?.closest("tr")?.getAttribute("data-key")).toBe("Ferien 2024 ü/<b>not bold</b> & ok.txt");
  });

  it("returns to the account list without another request", async () => {
    tree = { photos: PHOTOS };
    const root = await render();
    const b = await openBucket(root);
    const calls = folderCalls.length;
    await click(b, "← Back to accounts");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(root.querySelector('li[data-account="photos"]')).not.toBeNull();
    expect(folderCalls).toHaveLength(calls);
  });

  it("drops a slow folder's answer once another folder was opened", async () => {
    tree = { photos: PHOTOS };
    const root = await render();
    const b = await openBucket(root);
    const slow = deferred<unknown>();
    queued.push(() => slow.promise);
    await openFolder(b, "2024/");
    expect(text(b.querySelector(".loading"))).toBe("Loading folder…");
    // While 2024/ loads, the user goes back to the root through the breadcrumb.
    await click(b.querySelector("nav.crumbs")!, "soty-photos");
    expect(rows(b)).toHaveLength(5);
    slow.resolve(serve({ id: "photos", prefix: "2024/", page: null }));
    await settle();
    expect(crumbs(b)).toEqual(["soty-photos"]);
    expect(rows(b)).toHaveLength(5);
    expect(b.querySelector(".loading")).toBeNull();
  });
});

describe("pages", () => {
  const BIG: Tree[string] = {
    "": [
      { folders: [folder("d1/", "d1")], files: [file("f1"), file("f2")] },
      { folders: [folder("d2/", "d2")], files: [file("f3"), file("f2")] }, // f2 again: shown once
      { folders: [], files: [file("f4")] },
    ],
    "d1/": [{ folders: [], files: [file("d1/x")] }, { folders: [], files: [file("d1/y")] }],
  };

  it("loads one page first, then more on request, without duplicates and without a total", async () => {
    tree = { photos: BIG };
    const root = await render();
    const b = await openBucket(root);
    expect(rows(b)).toHaveLength(3);
    expect(text(b.querySelector(".count"))).toBe("3 items shown; more are available.");
    await click(b, "Load more");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: "photos::1" });
    expect(rows(b).map((r) => r.split(" | ")[0])).toEqual(["📁 d1", "📁 d2", "f1", "f2", "f3"]);
    await click(b, "Load more");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: "photos::2" });
    expect(rows(b)).toHaveLength(6);
    expect(text(b.querySelector(".count"))).toBe("6 items shown.");
    expect(buttons(b).map((x) => text(x))).not.toContain("Load more");
  });

  it("starts again from the first page on refresh and on changing folders", async () => {
    tree = { photos: BIG };
    const root = await render();
    const b = await openBucket(root);
    await click(b, "Load more");
    expect(rows(b)).toHaveLength(5);
    await click(b, "Refresh");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: null });
    expect(rows(b)).toHaveLength(3);
    await openFolder(b, "d1/");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "d1/", page: null });
    expect(rows(b)).toHaveLength(1);
    await click(b, "Load more");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "d1/", page: "photos:d1/:1" });
  });

  it("shows a loading note while more is loading and drops it after leaving the folder", async () => {
    tree = { photos: BIG };
    const root = await render();
    const b = await openBucket(root);
    const slow = deferred<unknown>();
    queued.push(() => slow.promise);
    await click(b, "Load more");
    expect(text(b.querySelector(".loading"))).toBe("Loading more…");
    expect(buttons(b).map((x) => text(x))).not.toContain("Load more");
    await openFolder(b, "d1/");
    slow.resolve(serve({ id: "photos", prefix: "", page: "photos::1" }));
    await settle();
    expect(rows(b).map((r) => r.split(" | ")[0])).toEqual(["x"]);
  });

  it("retries a failed page with the same cursor and refreshes an outdated one", async () => {
    tree = { photos: BIG };
    const root = await render();
    const b = await openBucket(root);
    queued.push(() => Promise.reject({ kind: "timeout", message: "the provider did not answer in time" }));
    await click(b, "Load more");
    expect(alerts(b).join(" ")).toContain("Could not load more of this folder. The provider did not answer in time.");
    expect(rows(b)).toHaveLength(3); // what was loaded stays
    await click(b, "Try again");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: "photos::1" });
    expect(rows(b)).toHaveLength(5);

    queued.push(() => Promise.reject({ kind: "outdated-page", message: "this page of the folder is no longer available" }));
    await click(b, "Load more");
    expect(alerts(b).join(" ")).toContain("This list is out of date. Refresh to load it again.");
    await click(b.querySelector('[role="alert"]')!, "Refresh");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: null });
    expect(rows(b)).toHaveLength(3);
    expect(alerts(b)).toEqual([]);
  });
});

describe("errors", () => {
  it("shows a failed folder with what to do, and loads it on Try again", async () => {
    tree = { photos: PHOTOS };
    queued.push(() => Promise.reject({ kind: "network", message: "the provider could not be reached" }));
    const root = await render();
    const b = await openBucket(root);
    const shown = alerts(b).join(" ");
    expect(shown).toContain("Could not load this folder.");
    expect(shown).toContain("The provider could not be reached. Check your internet connection");
    expect(buttons(b.querySelector('[role="alert"]')!).map((x) => text(x))).toEqual(["Try again"]);
    await click(b, "Try again");
    expect(alerts(b)).toEqual([]);
    expect(rows(b)).toHaveLength(5);
  });

  it("gives every kind its own explanation and offers editing where that fixes it", async () => {
    const cases: [string, string, boolean][] = [
      ["credentials-missing", "No key is stored for this account on this computer.", true],
      ["invalid-credentials", "does not accept this account's key ID or secret", true],
      ["access-denied", "A key limited to a file name prefix can only list inside that prefix", false],
      ["bucket-unavailable", "or this key may not see it", false],
      ["timeout", "did not answer in time", false],
      ["credential-store-unavailable", "credential store is not available", false],
      ["credential-store-locked", "Unlock it (or accept its prompt)", false],
      ["invalid-configuration", "settings cannot be used", true],
      ["provider", "The provider could not complete the request: HTTP 503 (SlowDown)", false],
    ];
    for (const [kind, expected, editable] of cases) {
      queued.push(() => Promise.reject({ kind, message: kind === "provider" ? "HTTP 503 (SlowDown)" : "detail" }));
      const root = await render();
      const b = await openBucket(root);
      const alert = b.querySelector('[role="alert"]')!;
      expect(text(alert), kind).toContain(expected);
      expect(buttons(alert).some((x) => text(x) === "Edit account"), kind).toBe(editable);
      unmount(app!);
      app = undefined;
      document.body.innerHTML = "";
    }
  });

  it("opens the account's edit form from a missing key", async () => {
    queued.push(() => Promise.reject({ kind: "credentials-missing", message: "no credentials stored for account 'photos'" }));
    const root = await render();
    const b = await openBucket(root);
    await click(b, "Edit account");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(root.querySelector('li[data-account="photos"] form')).not.toBeNull();
  });

  it("leaves a removed account's bucket and says why", async () => {
    tree = { photos: PHOTOS };
    const root = await render();
    const b = await openBucket(root);
    accounts = []; // removed elsewhere meanwhile
    const lists = listCalls;
    await click(b, "Refresh");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(alerts(root).join(" ")).toContain("Photos no longer exists");
    expect(listCalls).toBe(lists + 1);
    expect(root.querySelector('li[data-account="photos"]')).toBeNull();
  });
});

describe("accounts", () => {
  it("never shows one account's folder in another's browser", async () => {
    accounts = [account(), account({ id: "work", label: "Work", bucket: "soty-work" })];
    tree = {
      photos: { "": [{ folders: [], files: [file("photo-only.jpg")] }] },
      work: { "": [{ folders: [], files: [file("work-only.pdf")] }] },
    };
    const root = await render();
    // Photos' listing is slow; the user goes back and opens Work.
    const slow = deferred<unknown>();
    queued.push(() => slow.promise);
    let b = await openBucket(root, "photos");
    await click(b, "← Back to accounts");
    b = await openBucket(root, "work");
    expect(folderCalls.at(-1)).toEqual({ id: "work", prefix: "", page: null });
    expect(text(b.querySelector("h3"))).toBe("Work");
    expect(text(b.querySelector(".bucket"))).toBe("soty-work");
    expect(rows(b).map((r) => r.split(" | ")[0])).toEqual(["work-only.pdf"]);
    slow.resolve(serve({ id: "photos", prefix: "", page: null }));
    await settle();
    expect(rows(b).map((r) => r.split(" | ")[0])).toEqual(["work-only.pdf"]);
    expect(text(root)).not.toContain("photo-only.jpg");

    // And back to Photos: its own listing, from the root.
    await click(b, "← Back to accounts");
    b = await openBucket(root, "photos");
    expect(folderCalls.at(-1)).toEqual({ id: "photos", prefix: "", page: null });
    expect(rows(b).map((r) => r.split(" | ")[0])).toEqual(["photo-only.jpg"]);
    expect(text(root)).not.toContain("work-only.pdf");
  });
});

describe("listing wrapper", () => {
  it("parses the Rust shape and refuses anything else", () => {
    const listing = serve({ id: "photos", prefix: "a/", page: null }) as FolderListing;
    expect(parseFolderListing(listing)).toEqual(listing);
    expect(() => parseFolderListing({ ...listing, nextPage: 3 })).toThrow("invalid folder listing");
    expect(() => parseFolderListing({ ...listing, files: [{ ...file("x"), size: "1" }] })).toThrow("invalid file");
    expect(() => parseFolderListing({ ...listing, folders: [{ name: "x" }] })).toThrow("invalid folder");
    expect(() => parseFolderListing({ ...listing, breadcrumbs: [{ prefix: 1 }] })).toThrow("invalid breadcrumb");
  });
});

describe("downloading a file", () => {
  const TOTAL = 4 * 1024 * 1024;

  function event(
    transfer: string,
    state: string,
    bytes: number,
    total: number | null = TOTAL,
    extra: Record<string, unknown> = {},
  ): Record<string, unknown> {
    return { transfer, state, bytes, total, integrity: null, error: null, ...extra };
  }

  async function send(channel: FakeChannel, message: Record<string, unknown>): Promise<void> {
    channel.onmessage(message);
    flushSync();
    await settle();
  }

  function panel(root: ParentNode): HTMLElement {
    const el = root.querySelector<HTMLElement>("div.download");
    if (!el) {
      throw new Error("no download panel");
    }
    return el;
  }

  function downloadButton(root: ParentNode, key: string): HTMLButtonElement {
    const b = root.querySelector<HTMLButtonElement>(`tr[data-key="${CSS.escape(key)}"] button.download-action`);
    if (!b) {
      throw new Error(`no download action for ${key}`);
    }
    return b;
  }

  async function clickDownload(root: ParentNode, key: string): Promise<void> {
    downloadButton(root, key).click();
    flushSync();
    await settle();
  }

  async function opened(): Promise<HTMLElement> {
    tree = { photos: PHOTOS };
    const root = await render();
    return openBucket(root);
  }

  it("offers a download action per file, not per folder, and sends the exact key", async () => {
    const b = await opened();
    expect(b.querySelectorAll("tr.folder button.download-action")).toHaveLength(0);
    expect(downloadButton(b, "readme.txt").getAttribute("aria-label")).toBe("Download readme.txt");
    await openFolder(b, "Ferien 2024 ü/");
    const odd = "Ferien 2024 ü/<b>not bold</b> & ok.txt";
    const pending = deferred<unknown>();
    onDownload = () => pending.promise;
    await clickDownload(b, odd);
    expect(downloadCalls.map(({ id, bucket, key }) => ({ id, bucket, key }))).toEqual([
      { id: "photos", bucket: "soty-photos", key: odd },
    ]);
    // No local path ever leaves the page: Rust opens the dialog.
    expect(Object.keys(downloadCalls[0]!)).toEqual(["id", "bucket", "key", "channel"]);
    expect(panel(b).dataset["downloadState"]).toBe("choosing");
    expect(text(panel(b))).toContain("<b>not bold</b> & ok.txt");
    expect(panel(b).querySelector("b")).toBeNull();
    expect(text(panel(b))).toContain("Choose where to save the file");
    pending.resolve({ transfer: "download-1", fileName: "saved copy.txt" });
    await settle();
    expect(panel(b).dataset["downloadState"]).toBe("active");
    expect(panel(b).dataset["transfer"]).toBe("download-1");
    expect(text(panel(b).querySelector(".download-name"))).toBe("saved copy.txt");
  });

  it("starts nothing when the save dialog is cancelled", async () => {
    const b = await opened();
    onDownload = () => null;
    await clickDownload(b, "readme.txt");
    expect(downloadCalls).toHaveLength(1);
    expect(b.querySelector("div.download")).toBeNull();
    expect(downloadButton(b, "readme.txt").disabled).toBe(false);
    expect(alerts(b)).toEqual([]);
  });

  it("shows progress and blocks a second download while one is active", async () => {
    const b = await opened();
    await clickDownload(b, "big.bin");
    const ch = downloadCalls[0]!.channel;
    await send(ch, event("download-1", "active", 0));
    await send(ch, event("download-1", "active", TOTAL / 2));
    const progress = panel(b).querySelector("progress");
    expect(progress?.value).toBe(50);
    expect(text(panel(b).querySelector(".download-progress"))).toBe("50% · 2.0 MiB of 4.0 MiB");
    for (const key of ["big.bin", "readme.txt"]) {
      expect(downloadButton(b, key).disabled).toBe(true);
    }
    // The browser stays usable: folders still open.
    await openFolder(b, "2024/");
    expect(current(b)).toBe("2024");
    expect(downloadButton(b, "2024/cat.jpg").disabled).toBe(true);
    downloadButton(b, "2024/cat.jpg").click();
    await settle();
    expect(downloadCalls).toHaveLength(1);
    expect(panel(b).dataset["transfer"]).toBe("download-1");
  });

  it("shows bytes without a percentage when the size is unknown", async () => {
    const b = await opened();
    await clickDownload(b, "big.bin");
    await send(downloadCalls[0]!.channel, event("download-1", "active", 1536, null));
    expect(panel(b).querySelector("progress")).toBeNull();
    expect(text(panel(b).querySelector(".download-progress"))).toBe("1.5 KiB downloaded (total size unknown)");
  });

  it("reports completion with what was verified and frees the action", async () => {
    const b = await opened();
    await clickDownload(b, "big.bin");
    const ch = downloadCalls[0]!.channel;
    await send(ch, event("download-1", "active", TOTAL / 4));
    await send(ch, event("download-1", "completed", TOTAL, TOTAL, { integrity: "sha256-verified" }));
    expect(panel(b).dataset["downloadState"]).toBe("completed");
    expect(text(panel(b))).toContain("Downloaded 4.0 MiB. The SHA-256 checksum stored with the file matched.");
    expect(downloadButton(b, "readme.txt").disabled).toBe(false);
    // A late message of the finished transfer changes nothing.
    await send(ch, event("download-1", "active", 1));
    expect(panel(b).dataset["downloadState"]).toBe("completed");
    await click(b, "Dismiss");
    expect(b.querySelector("div.download")).toBeNull();

    await clickDownload(b, "readme.txt");
    await send(downloadCalls[1]!.channel, event("download-2", "completed", 12, 12, { integrity: "size-only" }));
    expect(text(panel(b))).toContain("no SHA-256 checksum stored by sotyware, so its content could not be verified");
    expect(text(panel(b))).not.toContain("SHA-256 checksum stored with the file matched");
  });

  it("cancels the running transfer by its id", async () => {
    const b = await opened();
    await clickDownload(b, "big.bin");
    const ch = downloadCalls[0]!.channel;
    await send(ch, event("download-1", "active", 100));
    await click(b, "Cancel download");
    expect(cancelCalls).toEqual(["download-1"]);
    expect(text(panel(b).querySelector(".download-progress"))).toBe("Cancelling…");
    expect(button(b, "Cancel download").disabled).toBe(true);
    await send(ch, event("download-1", "cancelled", 100));
    expect(panel(b).dataset["downloadState"]).toBe("cancelled");
    expect(text(panel(b))).toContain("Download cancelled. Nothing was saved.");
    expect(downloadButton(b, "big.bin").disabled).toBe(false);
  });

  it("says when cancelling is too late and the file is completed", async () => {
    const b = await opened();
    onCancel = () => "finishing";
    await clickDownload(b, "big.bin");
    const ch = downloadCalls[0]!.channel;
    await send(ch, event("download-1", "active", TOTAL));
    await click(b, "Cancel download");
    expect(text(panel(b))).toContain("cannot be cancelled any more");
    await send(ch, event("download-1", "completed", TOTAL, TOTAL, { integrity: "sha256-verified" }));
    expect(panel(b).dataset["downloadState"]).toBe("completed");
  });

  it("shows a failure and retries it as a new transfer, ignoring the old one", async () => {
    const b = await opened();
    await clickDownload(b, "big.bin");
    const first = downloadCalls[0]!.channel;
    await send(first, event("download-1", "active", 10));
    await send(
      first,
      event("download-1", "failed", 10, TOTAL, {
        error: { kind: "network", message: "the connection to the provider failed or broke off" },
      }),
    );
    expect(panel(b).dataset["downloadState"]).toBe("failed");
    expect(alerts(b).join(" ")).toContain("Download failed. Nothing was saved.");
    expect(alerts(b).join(" ")).toContain("could not be reached");
    expect(downloadButton(b, "readme.txt").disabled).toBe(false);

    await click(b, "Try download again");
    expect(retryCalls.map((r) => r.transfer)).toEqual(["download-1"]);
    expect(downloadCalls).toHaveLength(1); // no second dialog
    expect(panel(b).dataset["transfer"]).toBe("download-retry-1");
    expect(panel(b).dataset["downloadState"]).toBe("active");
    // Messages of the old transfer, on either channel, are ignored.
    await send(first, event("download-1", "failed", 0, TOTAL, { error: { kind: "other", message: "old" } }));
    await send(retryCalls[0]!.channel, event("download-1", "completed", TOTAL));
    expect(panel(b).dataset["downloadState"]).toBe("active");
    await send(retryCalls[0]!.channel, event("download-retry-1", "completed", TOTAL, TOTAL, { integrity: "size-only" }));
    expect(panel(b).dataset["downloadState"]).toBe("completed");
  });

  it("ignores messages of other transfers and accepts its own before the reply", async () => {
    const b = await opened();
    const pending = deferred<unknown>();
    onDownload = () => pending.promise;
    await clickDownload(b, "big.bin");
    const ch = downloadCalls[0]!.channel;
    await send(ch, event("download-7", "active", TOTAL / 4));
    expect(panel(b).dataset["transfer"]).toBe("download-7");
    expect(panel(b).dataset["downloadState"]).toBe("active");
    await send(ch, event("download-8", "completed", TOTAL));
    expect(panel(b).dataset["downloadState"]).toBe("active");
    pending.resolve({ transfer: "download-7", fileName: "big.bin" });
    await settle();
    await send(ch, event("download-7", "active", TOTAL / 2));
    expect(panel(b).querySelector("progress")?.value).toBe(50);
  });

  it("shows a refused start and asks again through the dialog", async () => {
    const b = await opened();
    onDownload = () => Promise.reject({ kind: "busy", message: "another download is running" });
    await clickDownload(b, "readme.txt");
    expect(panel(b).dataset["downloadState"]).toBe("failed");
    expect(alerts(b).join(" ")).toContain("Another download is running");
    onDownload = (call) => ({ transfer: "download-2", fileName: call.key });
    await click(b, "Try download again");
    expect(retryCalls).toHaveLength(0);
    expect(downloadCalls.map((c) => c.key)).toEqual(["readme.txt", "readme.txt"]);
    expect(panel(b).dataset["downloadState"]).toBe("active");
  });

  it("offers Edit account when the key must be fixed", async () => {
    const b = await opened();
    await clickDownload(b, "readme.txt");
    await send(
      downloadCalls[0]!.channel,
      event("download-1", "failed", 0, null, { error: { kind: "credentials-missing", message: "no key" } }),
    );
    expect(button(b, "Edit account")).toBeTruthy();
  });

  it("cancels a running download when the browser is closed", async () => {
    const root = await render();
    tree = { photos: PHOTOS };
    const b = await openBucket(root);
    await clickDownload(b, "big.bin");
    await send(downloadCalls[0]!.channel, event("download-1", "active", 5));
    await click(b, "← Back to accounts");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(cancelCalls).toEqual(["download-1"]);
  });
});
