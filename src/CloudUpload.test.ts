// Headless tests of the upload panel (CloudUpload.svelte) inside the bucket
// browser, opened from the Cloud Storage page as a user would. Tauri's IPC
// mock plays `upload_cloud_files`, `cancel_cloud_upload` and
// `retry_cloud_upload` (src-tauri/src/cloud/upload.rs) with the same shapes,
// and the tests send the batch events through the channel the command was
// given. This checks what the page shows and asks for; it does not open the
// native file dialog or upload anything (that is tested in Rust).
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import CloudStorage from "./CloudStorage.svelte";
import {
  CANCEL_CLOUD_DOWNLOAD,
  CANCEL_CLOUD_UPLOAD,
  DOWNLOAD_CLOUD_FILE,
  LIST_CLOUD_ACCOUNTS,
  LIST_CLOUD_FOLDER,
  LIST_CLOUD_PROVIDERS,
  RETRY_CLOUD_UPLOAD,
  UPLOAD_CLOUD_FILES,
  parseUploadEvent,
  parseUploadSelection,
  type AccountInfo,
  type FileState,
  type RemoteState,
  type SkipReason,
  type UploadStage,
} from "./cloud";

interface FakeChannel {
  onmessage: (message: unknown) => void;
}

interface UploadCall {
  id: string;
  bucket: string;
  prefix: string;
  channel: FakeChannel;
}

type Raw = Record<string, unknown>;

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

let accounts: AccountInfo[];
let folderCalls: { id: string; prefix: string }[];
let uploadCalls: UploadCall[];
let retryCalls: { batch: string; channel: FakeChannel }[];
let cancelCalls: string[];
let downloadCancels: string[];
let onUpload: (call: UploadCall) => unknown;
let onRetry: (batch: string, channel: FakeChannel) => unknown;
let onCancel: (batch: string) => unknown;
let app: ReturnType<typeof mount> | undefined;

/** A local path the fake "dialog" picked; it must never reach the page. */
const LOCAL_PATH = "/home/soty/Pictures/secret-folder";
const WINDOWS_PATH = "C:\\Users\\soty\\Pictures";

function listing(id: string, prefix: string): Raw {
  const acct = accounts.find((a) => a.id === id);
  const folders = prefix === "" ? [{ name: "2024", prefix: "2024/" }] : [];
  const crumbs = prefix === "" ? [] : [{ name: prefix.slice(0, -1), prefix }];
  return {
    accountId: id,
    bucket: acct?.bucket ?? "",
    prefix,
    parent: prefix === "" ? null : "",
    breadcrumbs: crumbs,
    folders,
    files: [{ name: "readme.txt", key: `${prefix}readme.txt`, size: 12, lastModified: null }],
    nextPage: null,
  };
}

beforeEach(() => {
  accounts = [account()];
  folderCalls = [];
  uploadCalls = [];
  retryCalls = [];
  cancelCalls = [];
  downloadCancels = [];
  onUpload = () => null;
  onRetry = () => Promise.reject({ kind: "invalid-input", message: "nothing in this upload is left to retry" });
  onCancel = () => "cancelling";
  mockWindows("main");
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case LIST_CLOUD_PROVIDERS:
        return [];
      case LIST_CLOUD_ACCOUNTS:
        return { accounts, storageError: null };
      case LIST_CLOUD_FOLDER: {
        const call = { id: args["id"] as string, prefix: args["prefix"] as string };
        folderCalls.push(call);
        return listing(call.id, call.prefix);
      }
      case UPLOAD_CLOUD_FILES: {
        const call = {
          id: args["id"] as string,
          bucket: args["bucket"] as string,
          prefix: args["prefix"] as string,
          channel: args["onEvent"] as FakeChannel,
        };
        uploadCalls.push(call);
        return onUpload(call);
      }
      case RETRY_CLOUD_UPLOAD: {
        const call = { batch: args["batch"] as string, channel: args["onEvent"] as FakeChannel };
        retryCalls.push(call);
        return onRetry(call.batch, call.channel);
      }
      case CANCEL_CLOUD_UPLOAD: {
        const batch = args["batch"] as string;
        cancelCalls.push(batch);
        return onCancel(batch);
      }
      case DOWNLOAD_CLOUD_FILE:
        return { transfer: "download-1", fileName: "readme.txt" };
      case CANCEL_CLOUD_DOWNLOAD:
        downloadCancels.push(args["transfer"] as string);
        return "cancelling";
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

function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function buttons(root: ParentNode): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>("button")];
}

function maybeButton(root: ParentNode, name: string): HTMLButtonElement | undefined {
  return buttons(root).find((b) => text(b) === name);
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = maybeButton(root, name);
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

async function render(): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(CloudStorage, { target });
  flushSync();
  await settle();
  return target;
}

async function opened(id = "photos"): Promise<{ root: HTMLElement; b: HTMLElement }> {
  const root = await render();
  const li = root.querySelector<HTMLElement>(`li[data-account="${id}"]`);
  if (!li) {
    throw new Error(`no account ${id}`);
  }
  await click(li, "Open bucket");
  const b = root.querySelector<HTMLElement>("section.browser");
  if (!b) {
    throw new Error("browser not open");
  }
  return { root, b };
}

function panel(root: ParentNode): HTMLElement {
  const el = root.querySelector<HTMLElement>("div.upload");
  if (!el) {
    throw new Error("no upload panel");
  }
  return el;
}

function alerts(root: ParentNode): string[] {
  return [...root.querySelectorAll('[role="alert"]')].map((e) => text(e));
}

/** Each listed file as "name | size | status | details…". */
function fileRows(root: ParentNode): string[] {
  return [...panel(root).querySelectorAll("ol.upload-files li")].map((li) =>
    [...li.children].map((c) => text(c)).join(" | "),
  );
}

function row(root: ParentNode, index: number): HTMLElement {
  const li = panel(root).querySelector<HTMLElement>(`ol.upload-files li[data-index="${index}"]`);
  if (!li) {
    throw new Error(`no file ${index}`);
  }
  return li;
}

function status(root: ParentNode, index: number): string {
  return text(row(root, index).querySelector(".file-status"));
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

// Shapes as Rust serializes them (upload.rs).

function uf(index: number, name: string, overrides: Raw = {}): Raw {
  return {
    index,
    name,
    key: name,
    size: 1000,
    state: "pending",
    stage: null,
    bytes: 0,
    total: 1000,
    started: false,
    skip: null,
    error: null,
    remote: null,
    retryable: false,
    ...overrides,
  };
}

function skipped(index: number, name: string, reason: SkipReason, message: string, extra: Raw = {}): Raw {
  return uf(index, name, { state: "skipped", skip: { reason, message }, ...extra });
}

function progressing(index: number, name: string, stage: UploadStage, bytes: number, total = 1000): Raw {
  return uf(index, name, { state: stage, stage, bytes, total, started: true });
}

function done(index: number, name: string, size = 1000): Raw {
  return uf(index, name, { state: "completed", stage: "verifying", bytes: size, total: size, size, started: true });
}

function failedFile(index: number, name: string, kind: string, message: string, remote: RemoteState): Raw {
  return uf(index, name, {
    state: "failed",
    stage: "sending",
    bytes: 500,
    started: true,
    error: { kind, message },
    remote,
    retryable: remote === "nothing-stored",
  });
}

function cancelledFile(index: number, name: string, started: boolean, remote: RemoteState = "nothing-stored"): Raw {
  return uf(index, name, {
    state: "cancelled",
    stage: started ? "sending" : null,
    bytes: started ? 300 : 0,
    started,
    remote,
    retryable: remote === "nothing-stored",
  });
}

function selection(batch: string | null, files: Raw[], prefix = ""): Raw {
  return { batch, prefix, files };
}

function summary(state: string, files: Raw[], error: Raw | null = null): Raw {
  const count = (s: FileState): number => files.filter((f) => f["state"] === s).length;
  return {
    state,
    count: files.length,
    completed: count("completed"),
    skipped: count("skipped"),
    failed: count("failed"),
    cancelled: count("cancelled"),
    notStarted: files.filter((f) => f["state"] === "cancelled" && f["started"] === false).length,
    retryable: files.filter((f) => f["retryable"] === true).length,
    needsAttention: files.filter((f) => f["remote"] === "incomplete-parts" || f["remote"] === "needs-inspection")
      .length,
    error,
    files,
  };
}

async function send(channel: FakeChannel, message: Raw): Promise<void> {
  channel.onmessage(message);
  flushSync();
  await settle();
}

function fileEvent(batch: string, seq: number, count: number, file: Raw): Raw {
  return { batch, seq, kind: "file", count, file };
}

function finishedEvent(batch: string, seq: number, s: Raw): Raw {
  return { batch, seq, kind: "finished", summary: s };
}

function cleanupEvent(batch: string, seq: number, key: string, outcome: "removed" | "failed"): Raw {
  return { batch, seq, kind: "cleanup", key, outcome };
}

/** Opens the browser and starts a batch of `files` (all pending) as upload-1. */
async function started(
  files: Raw[] = [uf(0, "a.jpg"), uf(1, "b.jpg")],
): Promise<{ root: HTMLElement; b: HTMLElement; channel: FakeChannel }> {
  onUpload = () => selection("upload-1", files);
  const { root, b } = await opened();
  await click(b, "Upload files…");
  return { root, b, channel: uploadCalls[0]!.channel };
}

describe("upload action", () => {
  it("is offered beside Up and Refresh and sends the browsed account, bucket and folder", async () => {
    const { b } = await opened();
    const actions = b.querySelector(".actions");
    expect(buttons(actions!).map((x) => text(x))).toEqual(["Up one folder", "Refresh", "Upload files…"]);
    expect(button(b, "Upload files…").disabled).toBe(false);
    await click(b, "Upload files…");
    expect(uploadCalls.map(({ id, bucket, prefix }) => ({ id, bucket, prefix }))).toEqual([
      { id: "photos", bucket: "soty-photos", prefix: "" },
    ]);
    // Only these arguments: no path, no file list from the page.
    expect(Object.keys(uploadCalls[0]!)).toEqual(["id", "bucket", "prefix", "channel"]);
  });

  it("uses the folder shown as the destination, exactly as Rust listed it", async () => {
    const { b } = await opened();
    b.querySelector<HTMLButtonElement>('tr[data-prefix="2024/"] button')!.click();
    flushSync();
    await settle();
    await click(b, "Upload files…");
    expect(uploadCalls[0]!.prefix).toBe("2024/");
  });

  it("shows nothing and no error when the dialog is cancelled or returns no file", async () => {
    const { b } = await opened();
    const dialog = deferred<unknown>();
    onUpload = () => dialog.promise;
    await click(b, "Upload files…");
    expect(text(panel(b))).toContain("Choose the files to upload…");
    expect(button(b, "Upload files…").disabled).toBe(true);
    dialog.resolve(null);
    await settle();
    expect(b.querySelector("div.upload")).toBeNull();
    expect(alerts(b)).toEqual([]);
    expect(button(b, "Upload files…").disabled).toBe(false);
  });

  it("keeps the previous result when the next dialog is cancelled", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    await send(channel, finishedEvent("upload-1", 1, summary("completed", [done(0, "a.jpg")])));
    onUpload = () => null;
    await click(b, "Upload files…");
    expect(text(panel(b))).toContain("Upload finished. 1 file: 1 uploaded.");
  });

  it("blocks a second batch while one is active", async () => {
    const { b } = await started();
    expect(button(b, "Upload files…").disabled).toBe(true);
    button(b, "Upload files…").click();
    await settle();
    expect(uploadCalls).toHaveLength(1);
  });

  it("shows a refused start with its reason and offers Edit account where that fixes it", async () => {
    const { root, b } = await opened();
    onUpload = () => Promise.reject({ kind: "credentials-missing", message: "no key is stored for 'photos'" });
    await click(b, "Upload files…");
    expect(panel(b).dataset["uploadPhase"]).toBe("refused");
    expect(alerts(panel(b)).join(" ")).toContain("No key is stored for this account on this computer.");
    await click(panel(b), "Edit account");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(root.querySelector("form")).not.toBeNull();
  });

  it("explains too many files and another running upload", async () => {
    const { b } = await opened();
    onUpload = () => Promise.reject({ kind: "invalid-input", message: "101 files were chosen; an upload takes at most 100" });
    await click(b, "Upload files…");
    expect(alerts(panel(b)).join(" ")).toContain("101 files were chosen; an upload takes at most 100");
    await click(panel(b), "Dismiss");
    onUpload = () => Promise.reject({ kind: "busy", message: "another upload is running; wait for it to finish or cancel it" });
    await click(b, "Upload files…");
    expect(alerts(panel(b)).join(" ")).toContain("Another upload is running.");
  });

  it("leaves a removed account's bucket and says why", async () => {
    const { root, b } = await opened();
    onUpload = () => Promise.reject({ kind: "unknown-account", message: "no account 'photos'" });
    await click(b, "Upload files…");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(alerts(root).join(" ")).toContain("Photos no longer exists");
  });
});

describe("selection", () => {
  it("lists skipped files with their reasons right away and uploads the rest", async () => {
    const { b } = await started([
      uf(0, "ok.jpg"),
      skipped(1, "folder", "not-a-file", "this is not a file"),
      skipped(2, "x.bin", "too-large", "the file is larger than the provider accepts (5497558138880 bytes)"),
      skipped(3, "dup.txt", "duplicate-key", "another selected file has the same name; none of them is uploaded"),
      skipped(4, "bad\u0007name", "control-characters", "the file name contains control characters", { key: null }),
      skipped(5, "long.txt", "key-too-long", "folder and file name together are longer than the 1,024 bytes"),
      skipped(6, "gone.txt", "unreadable", "the file could not be read", { size: null }),
      skipped(7, "nonutf8", "invalid-name", "the file name is not valid Unicode", { key: null }),
    ]);
    const rows = fileRows(b);
    expect(rows[0]).toBe("ok.jpg | 1000 B | Waiting");
    expect(rows[1]).toBe("folder | 1000 B | Skipped: not a file | This is not a file.");
    expect(rows[2]).toContain("Skipped: too large for this provider | The file is larger than the provider accepts");
    expect(rows[3]).toContain("Skipped: another selected file has the same name");
    expect(rows[4]).toContain("Skipped: name contains control characters");
    expect(rows[5]).toContain("Skipped: name too long for this folder");
    expect(rows[6]).toBe("gone.txt |  | Skipped: could not be read | The file could not be read.");
    expect(rows[7]).toContain("Skipped: name cannot be an object key");
    expect(text(panel(b).querySelector(".upload-progress"))).toContain("7 of 8 files finished.");
  });

  it("says nothing was sent when no file can be uploaded", async () => {
    onUpload = () => selection(null, [skipped(0, "dir", "not-a-file", "this is not a file")]);
    const { b } = await opened();
    await click(b, "Upload files…");
    expect(panel(b).dataset["uploadPhase"]).toBe("nothing");
    expect(text(panel(b))).toContain("None of the chosen files can be uploaded, so nothing was sent.");
    expect(maybeButton(panel(b), "Cancel upload")).toBeUndefined();
    expect(button(b, "Upload files…").disabled).toBe(false);
    expect(alerts(b)).toEqual([]);
  });
});

describe("progress", () => {
  it("shows each file's stage, bytes and position, one file after another", async () => {
    const { b, channel } = await started();
    const p = panel(b);
    expect(p.dataset["batch"]).toBe("upload-1");
    expect(text(p.querySelector(".upload-destination"))).toBe("Upload to soty-photos (root of the bucket)");
    await send(channel, fileEvent("upload-1", 1, 2, progressing(0, "a.jpg", "hashing", 250)));
    expect(text(p.querySelector(".upload-current"))).toBe(
      "File 1 of 2: a.jpg Computing the SHA-256 checksum · 25% · 250 B of 1000 B",
    );
    await send(channel, fileEvent("upload-1", 2, 2, progressing(0, "a.jpg", "sending", 500)));
    expect(text(p.querySelector(".upload-current"))).toContain("Sending · 50% · 500 B of 1000 B");
    expect(status(b, 0)).toBe("Sending · 50%");
    expect(status(b, 1)).toBe("Waiting");
    await send(channel, fileEvent("upload-1", 3, 2, progressing(0, "a.jpg", "verifying", 1000)));
    expect(status(b, 0)).toBe("Verifying the stored object · 100%");
    // Not complete until Rust says so.
    expect(text(p.querySelector(".upload-progress"))).toContain("0 of 2 files finished.");
    await send(channel, fileEvent("upload-1", 4, 2, done(0, "a.jpg")));
    expect(status(b, 0)).toBe("Uploaded and verified");
    expect(text(p.querySelector(".upload-progress"))).toContain("1 of 2 files finished.");
    await send(channel, fileEvent("upload-1", 5, 2, progressing(1, "b.jpg", "sending", 10)));
    expect(text(p.querySelector(".upload-current"))).toContain("File 2 of 2: b.jpg");
    expect(p.querySelector<HTMLProgressElement>("progress")!.value).toBe(1);
    expect(p.querySelector<HTMLProgressElement>("progress")!.max).toBe(2);
  });

  it("never goes back to an older event and shows no percentage without a total", async () => {
    const { b, channel } = await started([uf(0, "empty.txt", { size: 0, total: 0 })]);
    await send(channel, fileEvent("upload-1", 3, 1, progressing(0, "empty.txt", "sending", 0, 0)));
    expect(text(panel(b).querySelector(".upload-current"))).toBe("File 1 of 1: empty.txt Sending · 0 B of 0 B");
    expect(text(panel(b).querySelector(".upload-current"))).not.toContain("%");
    // An older event (lower seq) arriving late changes nothing.
    await send(channel, fileEvent("upload-1", 2, 1, progressing(0, "empty.txt", "hashing", 0, 0)));
    expect(status(b, 0)).toBe("Sending");
  });

  it("keeps the larger of two progress events whatever order they arrive in", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    await send(channel, fileEvent("upload-1", 5, 1, progressing(0, "a.jpg", "sending", 800)));
    await send(channel, fileEvent("upload-1", 4, 1, progressing(0, "a.jpg", "sending", 300)));
    expect(status(b, 0)).toBe("Sending · 80%");
  });

  it("accepts events that arrive before the command's reply", async () => {
    const reply = deferred<unknown>();
    onUpload = () => reply.promise;
    const { b } = await opened();
    await click(b, "Upload files…");
    const channel = uploadCalls[0]!.channel;
    await send(channel, fileEvent("upload-1", 1, 2, progressing(0, "a.jpg", "hashing", 100)));
    expect(panel(b).dataset["batch"]).toBe("upload-1");
    reply.resolve(selection("upload-1", [uf(0, "a.jpg"), uf(1, "b.jpg")]));
    await settle();
    // The reply's "pending" does not overwrite the newer event.
    expect(status(b, 0)).toBe("Computing the SHA-256 checksum · 10%");
    expect(status(b, 1)).toBe("Waiting");
  });

  it("finishes with a summary from the backend's final state", async () => {
    const { b, channel } = await started([uf(0, "a.jpg"), uf(1, "b.jpg"), uf(2, "c.jpg")]);
    const files = [
      done(0, "a.jpg"),
      skipped(1, "b.jpg", "destination-exists", "an object with this name is already in the folder; it was left as it is"),
      failedFile(2, "c.jpg", "file-changed", "the file changed while it was being uploaded", "nothing-stored"),
    ];
    await send(channel, finishedEvent("upload-1", 9, summary("completed", files)));
    const p = panel(b);
    expect(p.dataset["uploadPhase"]).toBe("finished");
    expect(text(p.querySelector(".upload-result"))).toBe("Upload finished. 3 files: 1 uploaded, 1 skipped, 1 failed.");
    expect(fileRows(b)[1]).toBe(
      "b.jpg | 1000 B | Skipped: already in this folder | An object with this name is already in the folder; it was left as it is.",
    );
    expect(fileRows(b)[2]).toContain("Failed | The file changed while it was being uploaded");
    expect(fileRows(b)[2]).toContain("Nothing was stored for this file.");
    expect(maybeButton(p, "Cancel upload")).toBeUndefined();
    expect(button(b, "Upload files…").disabled).toBe(false);
    // Events after the summary change nothing.
    await send(channel, fileEvent("upload-1", 10, 3, progressing(0, "a.jpg", "sending", 1)));
    expect(status(b, 0)).toBe("Uploaded and verified");
  });

  it("opens the destination folder after files were uploaded, also from another folder", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    b.querySelector<HTMLButtonElement>('tr[data-prefix="2024/"] button')!.click();
    flushSync();
    await settle();
    await send(channel, finishedEvent("upload-1", 2, summary("completed", [done(0, "a.jpg")])));
    folderCalls = [];
    await click(panel(b), "Show destination folder");
    expect(folderCalls).toEqual([{ id: "photos", prefix: "" }]);
  });

  it("explains a batch stopped by an error and the files it never reached", async () => {
    const { b, channel } = await started([uf(0, "a.jpg"), uf(1, "b.jpg")]);
    const error = { kind: "access-denied", message: "the provider refused to store files with this key" };
    const files = [
      failedFile(0, "a.jpg", "access-denied", "the provider refused to store files with this key", "nothing-stored"),
      cancelledFile(1, "b.jpg", false),
    ];
    await send(channel, finishedEvent("upload-1", 3, summary("failed", files, error)));
    const p = panel(b);
    expect(p.classList.contains("failed")).toBe(true);
    const result = alerts(p).join(" ");
    expect(result).toContain("Upload stopped after an error. 2 files: 0 uploaded, 1 failed, 1 not started.");
    expect(result).toContain("refused to store files with it");
    expect(status(b, 1)).toBe("Not started: the upload stopped after an error");
  });

  it("shows network, timeout and integrity failures with their own explanation", async () => {
    const { b, channel } = await started([uf(0, "a"), uf(1, "b"), uf(2, "c")]);
    const files = [
      failedFile(0, "a", "network", "the connection to the provider failed or broke off", "nothing-stored"),
      failedFile(1, "b", "timeout", "the provider stopped taking data or did not answer in time", "nothing-stored"),
      failedFile(2, "c", "integrity", "the stored object does not match", "needs-inspection"),
    ];
    await send(channel, finishedEvent("upload-1", 4, summary("completed", files)));
    const rows = fileRows(b);
    expect(rows[0]).toContain("The provider could not be reached, or the connection broke off.");
    expect(rows[1]).toContain("The provider stopped taking data or did not answer in time.");
    expect(rows[2]).toContain("did not match the file's size or SHA-256 checksum");
  });

  it("keeps a long batch in a scrolling list", async () => {
    const files = Array.from({ length: 100 }, (_, i) => uf(i, `file-${i}.bin`));
    const { b } = await started(files);
    const list = panel(b).querySelector("ol.upload-files");
    expect(list?.querySelectorAll("li")).toHaveLength(100);
    expect(text(panel(b).querySelector(".upload-progress"))).toContain("0 of 100 files finished.");
  });

  it("reports an event it cannot read without showing its content", async () => {
    const { b, channel } = await started();
    await send(channel, { batch: "upload-1", seq: 1, kind: "file", count: 2, file: { path: LOCAL_PATH } });
    expect(alerts(panel(b)).join(" ")).toContain("does not understand");
    expect(text(b)).not.toContain(LOCAL_PATH);
  });
});

describe("cancelling", () => {
  it("cancels the batch by its id, once, and waits for the backend's final state", async () => {
    const { b, channel } = await started();
    await send(channel, fileEvent("upload-1", 1, 2, progressing(0, "a.jpg", "sending", 300)));
    await click(panel(b), "Cancel upload");
    expect(cancelCalls).toEqual(["upload-1"]);
    expect(text(panel(b).querySelector(".upload-progress"))).toContain("Cancelling…");
    expect(button(panel(b), "Cancel upload").disabled).toBe(true);
    button(panel(b), "Cancel upload").click();
    await settle();
    expect(cancelCalls).toHaveLength(1);
    // Not cancelled until Rust says so.
    expect(panel(b).dataset["uploadPhase"]).toBe("active");
    const files = [cancelledFile(0, "a.jpg", true), cancelledFile(1, "b.jpg", false)];
    await send(channel, finishedEvent("upload-1", 2, summary("cancelled", files)));
    expect(text(panel(b).querySelector(".upload-result"))).toBe(
      "Upload cancelled. 2 files: 0 uploaded, 1 cancelled, 1 not started.",
    );
    // Started and cancelled, versus never started.
    expect(status(b, 0)).toBe("Cancelled during its upload");
    expect(fileRows(b)[0]).toContain("Nothing was stored for this file.");
    expect(status(b, 1)).toBe("Not started: the upload was cancelled");
    expect(fileRows(b)[1]).not.toContain("Nothing was stored");
  });

  it("explains that cancelling may let the current file finish", async () => {
    const { b } = await started();
    expect(text(panel(b))).toContain(
      "Cancelling stops the current file unless it is already being completed at the provider",
    );
  });

  it("says when the current file is past its commit point and will finish", async () => {
    onCancel = () => "finishing";
    const { b, channel } = await started();
    await send(channel, fileEvent("upload-1", 1, 2, progressing(0, "a.jpg", "verifying", 100)));
    await click(panel(b), "Cancel upload");
    expect(text(panel(b).querySelector(".upload-progress"))).toContain(
      "The current file is already being completed at the provider and will finish; no further file starts.",
    );
    expect(button(panel(b), "Cancel upload").disabled).toBe(true);
    const files = [done(0, "a.jpg"), cancelledFile(1, "b.jpg", false)];
    await send(channel, finishedEvent("upload-1", 3, summary("cancelled", files)));
    expect(status(b, 0)).toBe("Uploaded and verified");
    expect(status(b, 1)).toBe("Not started: the upload was cancelled");
  });

  it("cancels between files: the next files never start", async () => {
    const { b, channel } = await started([uf(0, "a.jpg"), uf(1, "b.jpg"), uf(2, "c.jpg")]);
    await send(channel, fileEvent("upload-1", 4, 3, done(0, "a.jpg")));
    await click(panel(b), "Cancel upload");
    const files = [done(0, "a.jpg"), cancelledFile(1, "b.jpg", false), cancelledFile(2, "c.jpg", false)];
    await send(channel, finishedEvent("upload-1", 5, summary("cancelled", files)));
    expect(text(panel(b).querySelector(".upload-result"))).toBe(
      "Upload cancelled. 3 files: 1 uploaded, 2 not started.",
    );
    expect(button(panel(b), "Try 2 files again")).toBeTruthy();
  });

  it("handles a batch that already ended when the cancel arrived", async () => {
    onCancel = () => "not-active";
    const { b, channel } = await started([uf(0, "a.jpg")]);
    await click(panel(b), "Cancel upload");
    expect(text(panel(b).querySelector(".upload-progress"))).toContain("The upload already ended; its result follows.");
    await send(channel, finishedEvent("upload-1", 2, summary("completed", [done(0, "a.jpg")])));
    expect(text(panel(b).querySelector(".upload-result"))).toBe("Upload finished. 1 file: 1 uploaded.");
  });

  it("shows a failed cancel request and allows another", async () => {
    onCancel = () => Promise.reject(new Error("ipc broke"));
    const { b } = await started();
    await click(panel(b), "Cancel upload");
    expect(alerts(panel(b)).join(" ")).toContain("Could not cancel: ipc broke");
    expect(button(panel(b), "Cancel upload").disabled).toBe(false);
  });
});

describe("retrying", () => {
  async function finishedWith(files: Raw[], state = "completed"): Promise<{ b: HTMLElement; channel: FakeChannel }> {
    const s = await started(files.map((f) => uf(f["index"] as number, f["name"] as string)));
    await send(s.channel, finishedEvent("upload-1", 50, summary(state, files)));
    return { b: s.b, channel: s.channel };
  }

  it("retries only what the backend marked retryable, as a new batch", async () => {
    const { b } = await finishedWith([
      done(0, "a.jpg"),
      skipped(1, "b.jpg", "destination-exists", "an object with this name is already in the folder"),
      failedFile(2, "c.jpg", "network", "the connection broke off", "nothing-stored"),
    ]);
    expect(text(panel(b))).toContain("Try again uploads only the files with nothing stored (1)");
    onRetry = () => selection("upload-2", [uf(0, "c.jpg")]);
    await click(panel(b), "Try 1 file again");
    expect(retryCalls.map((c) => c.batch)).toEqual(["upload-1"]);
    expect(uploadCalls).toHaveLength(1); // no new dialog
    const p = panel(b);
    expect(p.dataset["batch"]).toBe("upload-2");
    expect(text(p.querySelector(".upload-destination"))).toContain("Retrying upload to soty-photos");
    expect(fileRows(b)).toEqual(["c.jpg | 1000 B | Waiting"]);
  });

  it("ignores the old batch's events once the retry runs", async () => {
    const { b, channel: old } = await finishedWith([
      failedFile(0, "a.jpg", "network", "the connection broke off", "nothing-stored"),
    ]);
    onRetry = () => selection("upload-2", [uf(0, "a.jpg")]);
    await click(panel(b), "Try 1 file again");
    const fresh = retryCalls[0]!.channel;
    await send(old, fileEvent("upload-1", 99, 1, done(0, "a.jpg")));
    await send(old, finishedEvent("upload-1", 100, summary("completed", [done(0, "a.jpg")])));
    expect(status(b, 0)).toBe("Waiting");
    expect(panel(b).dataset["uploadPhase"]).toBe("active");
    // An event naming another batch on the new channel is ignored too.
    await send(fresh, fileEvent("upload-7", 1, 1, done(0, "a.jpg")));
    expect(status(b, 0)).toBe("Waiting");
    await send(fresh, fileEvent("upload-2", 1, 1, progressing(0, "a.jpg", "sending", 100)));
    expect(status(b, 0)).toBe("Sending · 10%");
  });

  it("ignores the old batch's events while the retry's reply is still pending", async () => {
    const { b, channel: old } = await finishedWith([
      failedFile(0, "a.jpg", "network", "the connection broke off", "nothing-stored"),
    ]);
    const reply = deferred<unknown>();
    onRetry = () => reply.promise;
    await click(panel(b), "Try 1 file again");
    expect(panel(b).dataset["uploadPhase"]).toBe("starting");
    // The new batch id is not known yet; the old channel must still not count.
    await send(old, fileEvent("upload-1", 99, 1, done(0, "a.jpg")));
    expect(panel(b).dataset["batch"]).toBe("");
    expect(b.querySelectorAll("ol.upload-files li")).toHaveLength(0);
    reply.resolve(selection("upload-2", [uf(0, "a.jpg")]));
    await settle();
    expect(panel(b).dataset["batch"]).toBe("upload-2");
    expect(status(b, 0)).toBe("Waiting");
  });

  it("never offers a retry for files that need inspection or left parts", async () => {
    const { b } = await finishedWith([
      failedFile(0, "a.jpg", "integrity", "the stored object does not match", "needs-inspection"),
      failedFile(1, "b.jpg", "network", "the connection broke off", "incomplete-parts"),
      done(2, "c.jpg"),
    ]);
    const p = panel(b);
    expect(buttons(p).map((x) => text(x))).toEqual(["Show destination folder", "Dismiss"]);
    expect(text(p)).toContain("2 files need attention");
    const rows = fileRows(b);
    expect(rows[0]).toContain("The provider may have stored this file, but sotyware could not verify it.");
    expect(rows[0]).toContain("sotyware does not retry or delete it");
    expect(rows[0]).not.toMatch(/try again/i);
    expect(rows[1]).toContain("parts of it may remain at the provider");
    expect(rows[1]).not.toMatch(/try again/i);
  });

  it("shows cleanup of earlier parts apart from the files, without claiming the upload completed", async () => {
    const { b } = await finishedWith([
      failedFile(0, "a.jpg", "network", "the connection broke off", "incomplete-parts"),
      failedFile(1, "b.jpg", "network", "the connection broke off", "nothing-stored"),
    ]);
    let fresh: FakeChannel | undefined;
    onRetry = (_batch, channel) => {
      fresh = channel;
      channel.onmessage(cleanupEvent("upload-2", 1, "a.jpg", "removed"));
      return selection("upload-2", [uf(0, "b.jpg")]);
    };
    await click(panel(b), "Try 1 file again");
    await send(fresh!, cleanupEvent("upload-2", 2, "z.bin", "failed"));
    const p = panel(b);
    const cleanup = [...p.querySelectorAll("ul.upload-cleanup li")].map((li) => text(li));
    expect(cleanup).toEqual([
      "The incomplete parts left by an earlier failed upload of a.jpg were removed. That earlier upload was not completed.",
      "The incomplete parts left by an earlier failed upload of z.bin could still not be removed; they may remain at the provider.",
    ]);
    // Not a file of this batch and not "uploaded".
    expect(fileRows(b)).toEqual(["b.jpg | 1000 B | Waiting"]);
    // A repeated cleanup event (same seq) is not listed twice.
    await send(fresh!, cleanupEvent("upload-2", 2, "z.bin", "failed"));
    expect(p.querySelectorAll("ul.upload-cleanup li")).toHaveLength(2);
  });

  it("keeps the earlier result when a retry is refused, and closes it when Rust says it is gone", async () => {
    const { b } = await finishedWith([failedFile(0, "a.jpg", "network", "the connection broke off", "nothing-stored")]);
    onRetry = () => Promise.reject({ kind: "credential-store-locked", message: "the keyring is locked" });
    await click(panel(b), "Try 1 file again");
    expect(panel(b).dataset["batch"]).toBe("upload-1");
    expect(alerts(panel(b)).join(" ")).toContain("The retry did not start.");
    expect(button(panel(b), "Try 1 file again").disabled).toBe(false);
    onRetry = () =>
      Promise.reject({ kind: "invalid-input", message: "this upload can no longer be retried; choose the files again" });
    await click(panel(b), "Try 1 file again");
    expect(alerts(panel(b)).join(" ")).toContain("choose the files again");
    expect(maybeButton(panel(b), "Try 1 file again")).toBeUndefined();
  });

  it("shows a retry whose files are all skipped now", async () => {
    const { b } = await finishedWith([failedFile(0, "a.jpg", "network", "the connection broke off", "nothing-stored")]);
    onRetry = () =>
      selection(null, [skipped(0, "a.jpg", "unreadable", "the file could not be read", { size: null })]);
    await click(panel(b), "Try 1 file again");
    expect(panel(b).dataset["uploadPhase"]).toBe("nothing");
    expect(status(b, 0)).toBe("Skipped: could not be read");
  });
});

describe("navigation and lifecycle", () => {
  it("cancels a running batch when the browser is left", async () => {
    const { root, b, channel } = await started();
    await send(channel, fileEvent("upload-1", 1, 2, progressing(0, "a.jpg", "sending", 1)));
    await click(b, "← Back to accounts");
    expect(root.querySelector("section.browser")).toBeNull();
    expect(cancelCalls).toEqual(["upload-1"]);
  });

  it("cancels a batch whose reply arrives after the browser was left", async () => {
    const reply = deferred<unknown>();
    onUpload = () => reply.promise;
    const { b } = await opened();
    await click(b, "Upload files…");
    await click(b, "← Back to accounts");
    expect(cancelCalls).toEqual([]);
    reply.resolve(selection("upload-3", [uf(0, "a.jpg")]));
    await settle();
    expect(cancelCalls).toEqual(["upload-3"]);
  });

  it("does not cancel a finished batch when leaving, nor on hiding the page", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("blur"));
    await settle();
    expect(cancelCalls).toEqual([]);
    await send(channel, finishedEvent("upload-1", 2, summary("completed", [done(0, "a.jpg")])));
    await click(b, "← Back to accounts");
    expect(cancelCalls).toEqual([]);
  });

  it("keeps the captured destination while the user browses elsewhere", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    b.querySelector<HTMLButtonElement>('tr[data-prefix="2024/"] button')!.click();
    flushSync();
    await settle();
    expect(text(b.querySelector(".path"))).toBe("2024/");
    expect(text(panel(b).querySelector(".upload-destination"))).toBe("Upload to soty-photos (root of the bucket)");
    await send(channel, fileEvent("upload-1", 1, 1, progressing(0, "a.jpg", "sending", 1)));
    expect(cancelCalls).toEqual([]);
    expect(uploadCalls).toHaveLength(1);
  });

  it("starts fresh in another account's browser and drops the old batch's events", async () => {
    accounts = [account(), account({ id: "work", label: "Work", bucket: "work-bucket" })];
    const { root, b, channel } = await started([uf(0, "a.jpg")]);
    await click(b, "← Back to accounts");
    expect(cancelCalls).toEqual(["upload-1"]);
    const work = root.querySelector<HTMLElement>('li[data-account="work"]')!;
    await click(work, "Open bucket");
    const w = root.querySelector<HTMLElement>("section.browser")!;
    expect(w.querySelector("div.upload")).toBeNull();
    await send(channel, fileEvent("upload-1", 2, 1, progressing(0, "a.jpg", "sending", 9)));
    await send(channel, finishedEvent("upload-1", 3, summary("cancelled", [cancelledFile(0, "a.jpg", true)])));
    expect(w.querySelector("div.upload")).toBeNull();
    onUpload = () => selection("upload-2", [uf(0, "w.txt")]);
    await click(w, "Upload files…");
    expect(uploadCalls[1]!.id).toBe("work");
    expect(uploadCalls[1]!.bucket).toBe("work-bucket");
    expect(text(panel(w).querySelector(".upload-destination"))).toBe("Upload to work-bucket (root of the bucket)");
  });

  it("applies nothing after the whole page is unmounted", async () => {
    const { channel } = await started([uf(0, "a.jpg")]);
    unmount(app!);
    app = undefined;
    expect(cancelCalls).toEqual(["upload-1"]);
    expect(() => channel.onmessage(fileEvent("upload-1", 2, 1, done(0, "a.jpg")))).not.toThrow();
    expect(document.querySelector("div.upload")).toBeNull();
  });

  it("runs beside a download without either affecting the other", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    b.querySelector<HTMLButtonElement>('tr[data-key="readme.txt"] button.download-action')!.click();
    flushSync();
    await settle();
    expect(b.querySelector("div.download")).not.toBeNull();
    await send(channel, finishedEvent("upload-1", 2, summary("completed", [done(0, "a.jpg")])));
    expect(b.querySelector("div.download")?.getAttribute("data-download-state")).toBe("active");
    expect(downloadCancels).toEqual([]);
  });
});

describe("privacy", () => {
  it("never shows a local path, even if one were sent", async () => {
    const { b, channel } = await started([uf(0, "a.jpg")]);
    // Rust never sends paths; this checks the page has nowhere to show one.
    await send(
      channel,
      finishedEvent(
        "upload-1",
        2,
        summary("completed", [failedFile(0, "a.jpg", "local-file", "the file could not be read", "nothing-stored")]),
      ),
    );
    const shown = `${text(b)} ${alerts(b).join(" ")} ${[...b.querySelectorAll("[aria-label],[title]")]
      .map((e) => `${e.getAttribute("aria-label") ?? ""} ${e.getAttribute("title") ?? ""}`)
      .join(" ")}`;
    expect(shown).not.toContain(LOCAL_PATH);
    expect(shown).not.toContain(WINDOWS_PATH);
    expect(shown).not.toContain("/home/");
    expect(shown).toContain("The file could not be read on this computer.");
  });
});

describe("upload wrappers", () => {
  it("parse the Rust shapes and refuse anything else", () => {
    const sel = parseUploadSelection(selection("upload-1", [uf(0, "a.jpg"), skipped(1, "b", "not-a-file", "x")]));
    expect(sel.batch).toBe("upload-1");
    expect(sel.files[1]?.skip?.reason).toBe("not-a-file");
    expect(() => parseUploadSelection({ batch: 1, prefix: "", files: [] })).toThrow();
    expect(() => parseUploadSelection(selection("u", [uf(0, "a", { state: "uploading" })]))).toThrow();
    expect(() => parseUploadSelection(selection("u", [uf(0, "a", { remote: "maybe" })]))).toThrow();
    expect(() => parseUploadSelection(selection("u", [uf(0, "a", { bytes: -1 })]))).toThrow();

    const ev = parseUploadEvent(finishedEvent("upload-1", 3, summary("failed", [done(0, "a")], {
      kind: "network",
      message: "broke",
    })));
    expect(ev.kind).toBe("finished");
    if (ev.kind === "finished") {
      expect(ev.summary.error).toEqual({ kind: "network", message: "broke" });
      expect(ev.summary.completed).toBe(1);
    }
    expect(parseUploadEvent(cleanupEvent("upload-1", 1, "k", "removed"))).toEqual({
      batch: "upload-1",
      seq: 1,
      kind: "cleanup",
      key: "k",
      outcome: "removed",
    });
    expect(() => parseUploadEvent({ batch: "u", seq: 1, kind: "progress" })).toThrow();
    expect(() => parseUploadEvent(cleanupEvent("u", 1, "k", "gone" as "removed"))).toThrow();
    expect(() => parseUploadEvent({ ...fileEvent("u", 1, 1, uf(0, "a")), seq: "1" })).toThrow();
    expect(() =>
      parseUploadEvent(fileEvent("u", 1, 1, uf(0, "a", { error: { kind: "nope", message: "x" } }))),
    ).toThrow();
  });
});
