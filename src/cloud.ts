// Typed access to cloud storage accounts (src-tauri/src/cloud/dashboard.rs).
// The page reaches them only through these commands. Validation, ids,
// persistence and credentials are handled in Rust; the page sends what the
// user typed and shows what comes back. Responses are checked, so a
// Rust/TypeScript mismatch fails loudly.
//
// Credentials only ever travel from the page to Rust (`addAccount`,
// `updateAccount`); no command returns them, and nothing here keeps them.
//
// `listFolder` is read-only browsing (src-tauri/src/cloud/browser.rs): one
// page of one folder of the account's configured bucket. Folder prefixes and
// object keys are passed through exactly as Rust returns them; the page never
// builds a prefix from names.
//
// `downloadFile` downloads one file (src-tauri/src/cloud/download.rs): Rust
// opens the native save dialog, so the page never sends a local path, only
// the account, the bucket it browsed and the object key exactly as listed.
// Progress arrives on a Tauri channel created for that one download.
//
// `uploadFiles` uploads a batch (src-tauri/src/cloud/upload.rs): Rust opens
// the native multi-file dialog and keeps the paths; the page sends only the
// account, the bucket it browsed and the folder prefix it shows, and gets
// back file names, keys, sizes and states. Each batch's events arrive on a
// channel created for that one call (also for a retry, which is a new batch).
// Validation, conflicts, hashing, verification and cleanup are Rust's; the
// page shows what the events say.
import { Channel, invoke } from "@tauri-apps/api/core";

export const LIST_CLOUD_PROVIDERS = "list_cloud_providers";
export const LIST_CLOUD_ACCOUNTS = "list_cloud_accounts";
export const ADD_CLOUD_ACCOUNT = "add_cloud_account";
export const UPDATE_CLOUD_ACCOUNT = "update_cloud_account";
export const REMOVE_CLOUD_ACCOUNT = "remove_cloud_account";
export const CHECK_CLOUD_ACCOUNT = "check_cloud_account";
export const LIST_CLOUD_FOLDER = "list_cloud_folder";
export const DOWNLOAD_CLOUD_FILE = "download_cloud_file";
export const CANCEL_CLOUD_DOWNLOAD = "cancel_cloud_download";
export const RETRY_CLOUD_DOWNLOAD = "retry_cloud_download";
export const UPLOAD_CLOUD_FILES = "upload_cloud_files";
export const CANCEL_CLOUD_UPLOAD = "cancel_cloud_upload";
export const RETRY_CLOUD_UPLOAD = "retry_cloud_upload";

// `CheckStatus` in dashboard.rs, serialized in kebab-case.
export const CHECK_STATUSES = [
  "connected",
  "invalid-credentials",
  "access-denied",
  "bucket-unavailable",
  "network",
  "credential-store-unavailable",
  "credential-store-locked",
  "credentials-missing",
  "invalid-configuration",
  "error",
] as const;

export type CheckStatus = (typeof CHECK_STATUSES)[number];

// `ErrorKind` in dashboard.rs, serialized in kebab-case.
export const ERROR_KINDS = [
  "invalid-input",
  "unknown-account",
  "storage",
  "credential-store-unavailable",
  "credential-store-locked",
  "credential-store",
  "partial",
  "credentials-missing",
  "invalid-credentials",
  "access-denied",
  "bucket-unavailable",
  "network",
  "timeout",
  "invalid-configuration",
  "provider",
  "outdated-page",
  "busy",
  "object-not-found",
  "integrity",
  "disk-full",
  "local-permission",
  "destination-unavailable",
  "destination-exists",
  "local-file",
  "file-changed",
  "remote-changed",
  "too-large",
  "outdated-plan",
  "other",
] as const;

export type ErrorKind = (typeof ERROR_KINDS)[number];

// `RegionInfo` and `ProviderInfo` in dashboard.rs.
export interface RegionInfo {
  id: string;
  location: string;
}

export interface ProviderInfo {
  id: string;
  name: string;
  /** No regional endpoint: the account must give one. */
  endpointRequired: boolean;
  defaultRegion: string;
  knownRegions: RegionInfo[];
}

// `ConnectionCheck` in dashboard.rs.
export interface ConnectionCheck {
  status: CheckStatus;
  message: string | null;
  bucket: string | null;
  /** Milliseconds since the Unix epoch. */
  checkedAt: number;
}

// `AccountInfo` in dashboard.rs. Holds no credentials.
export interface AccountInfo {
  id: string;
  label: string;
  provider: string;
  providerName: string | null;
  region: string;
  endpoint: string | null;
  endpointUrl: string | null;
  bucket: string | null;
  configError: string | null;
  lastCheck: ConnectionCheck | null;
}

// `AccountList` in dashboard.rs.
export interface AccountList {
  accounts: AccountInfo[];
  storageError: string | null;
}

// `Breadcrumb`, `FolderEntry`, `FileEntry` and `FolderListing` in browser.rs.
export interface Breadcrumb {
  /** May be empty: S3 allows a key such as `a//b`. */
  name: string;
  prefix: string;
}

export interface FolderEntry {
  /** Without the trailing `/`; may be empty. */
  name: string;
  /** The full prefix, ending in `/`, exactly as the provider returned it. */
  prefix: string;
}

export interface FileEntry {
  name: string;
  /** The full object key, unchanged. */
  key: string;
  size: number;
  /** RFC 3339, if the provider gave one. */
  lastModified: string | null;
}

export interface FolderListing {
  accountId: string;
  bucket: string;
  /** Empty for the bucket's root, else ending in `/`. */
  prefix: string;
  parent: string | null;
  /** From the first folder below the root to this one; empty at the root. */
  breadcrumbs: Breadcrumb[];
  folders: FolderEntry[];
  files: FileEntry[];
  /** Pass to `listFolder` for the next page of the same folder. */
  nextPage: string | null;
}

// `TransferState`, `Integrity`, `CancelOutcome` in download.rs (kebab-case).
export const TRANSFER_STATES = ["active", "completed", "cancelled", "failed"] as const;
export type TransferState = (typeof TRANSFER_STATES)[number];

export const INTEGRITY_CHECKS = ["sha256-verified", "size-only", "transfer-only"] as const;
export type IntegrityCheck = (typeof INTEGRITY_CHECKS)[number];

export const CANCEL_OUTCOMES = ["cancelling", "finishing", "not-active"] as const;
export type CancelOutcome = (typeof CANCEL_OUTCOMES)[number];

/** A command error as it travels inside a download message. */
export interface TransferError {
  kind: ErrorKind;
  message: string;
}

// `DownloadEvent` in download.rs.
export interface DownloadEvent {
  transfer: string;
  state: TransferState;
  bytes: number;
  /** `null` when the provider did not announce a size. */
  total: number | null;
  /** Only for "completed". */
  integrity: IntegrityCheck | null;
  /** Only for "failed". */
  error: TransferError | null;
}

// `DownloadStarted` in download.rs.
export interface DownloadStarted {
  transfer: string;
  /** The chosen file's name (not its folder). */
  fileName: string;
}

// `BatchState`, `FileState`, `Stage`, `SkipReason`, `RemoteState` and
// `CleanupOutcome` in upload.rs, serialized in kebab-case.
export const BATCH_STATES = ["active", "completed", "cancelled", "failed"] as const;
export type BatchState = (typeof BATCH_STATES)[number];

export const FILE_STATES = [
  "pending",
  "hashing",
  "sending",
  "verifying",
  "completed",
  "skipped",
  "failed",
  "cancelled",
] as const;
export type FileState = (typeof FILE_STATES)[number];

/** The states after which a file never changes again. */
export const FINAL_FILE_STATES: ReadonlySet<FileState> = new Set<FileState>([
  "completed",
  "skipped",
  "failed",
  "cancelled",
]);

export const UPLOAD_STAGES = ["hashing", "sending", "verifying"] as const;
export type UploadStage = (typeof UPLOAD_STAGES)[number];

export const SKIP_REASONS = [
  "not-a-file",
  "unreadable",
  "invalid-name",
  "control-characters",
  "key-too-long",
  "too-large",
  "duplicate-key",
  "destination-exists",
] as const;
export type SkipReason = (typeof SKIP_REASONS)[number];

export const REMOTE_STATES = ["nothing-stored", "incomplete-parts", "needs-inspection"] as const;
export type RemoteState = (typeof REMOTE_STATES)[number];

export const CLEANUP_OUTCOMES = ["removed", "failed"] as const;
export type CleanupOutcome = (typeof CLEANUP_OUTCOMES)[number];

/** A batch id (`upload-<n>`), never reused. */
export type UploadBatchId = string;

// `Skip` in upload.rs.
export interface UploadSkip {
  reason: SkipReason;
  /** sotyware's own sentence. */
  message: string;
}

// `UploadFile` in upload.rs. Never carries a local path.
export interface UploadFile {
  /** Position in the batch, from 0. */
  index: number;
  /** The file's name, not its folder. */
  name: string;
  /** The object key; `null` if the name cannot become one. */
  key: string | null;
  /** The size when the file was picked, if it could be read. */
  size: number | null;
  state: FileState;
  /** The last stage the upload reached. */
  stage: UploadStage | null;
  /** Hashed, sent or verified bytes of `stage`; never decreases within it. */
  bytes: number;
  total: number;
  /**
   * Whether its upload began: true from the moment the batch takes the file
   * (also while its destination is checked, though no event is sent then:
   * a file's first event is its first progress or its final state); false
   * for skipped files (also when the key turned out to exist) and
   * never-reached files.
   */
  started: boolean;
  /** Only for "skipped". */
  skip: UploadSkip | null;
  /** Only for "failed". */
  error: TransferError | null;
  /** For "failed" and "cancelled". */
  remote: RemoteState | null;
  /** Whether `retryUpload` would upload it again. */
  retryable: boolean;
}

// `BatchSummary` in upload.rs.
export interface UploadSummary {
  state: BatchState;
  count: number;
  completed: number;
  skipped: number;
  failed: number;
  /** Cancelled while running or never started. */
  cancelled: number;
  /** Of those, never started. */
  notStarted: number;
  retryable: number;
  /** Files whose remote outcome needs inspection or cleanup. */
  needsAttention: number;
  /** The error that stopped the batch (only for "failed"). */
  error: TransferError | null;
  files: UploadFile[];
}

// `UploadEvent` (with its flattened `EventBody`) in upload.rs.
export type UploadEvent =
  | { batch: UploadBatchId; seq: number; kind: "file"; count: number; file: UploadFile }
  | { batch: UploadBatchId; seq: number; kind: "cleanup"; key: string; outcome: CleanupOutcome }
  | { batch: UploadBatchId; seq: number; kind: "finished"; summary: UploadSummary };

// `UploadSelection` in upload.rs.
export interface UploadSelection {
  /** `null` if no file can be uploaded: no batch started. */
  batch: UploadBatchId | null;
  /** The destination folder ("" for the top level). */
  prefix: string;
  /** Every selected file: pending, or skipped with its reason. */
  files: UploadFile[];
}

// `AccountInput` in dashboard.rs: the non-secret part of the form.
export interface AccountInput {
  label: string;
  provider: string;
  region: string;
  endpoint: string | null;
  bucket: string;
}

// `CredentialInput` in dashboard.rs.
export interface CredentialInput {
  accessKeyId: string;
  secretAccessKey: string;
}

/** A command's structured failure (`CommandError` in dashboard.rs). */
export class CloudCommandError extends Error {
  readonly kind: ErrorKind;

  constructor(kind: ErrorKind, message: string) {
    super(message);
    this.name = "CloudCommandError";
    this.kind = kind;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function oneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return (values as readonly unknown[]).includes(value);
}

function invalid(what: string, value: unknown): Error {
  return new Error(`invalid ${what}: ${JSON.stringify(value)}`);
}

export function parseProvider(value: unknown): ProviderInfo {
  if (
    !isRecord(value) ||
    typeof value["id"] !== "string" ||
    typeof value["name"] !== "string" ||
    typeof value["endpointRequired"] !== "boolean" ||
    typeof value["defaultRegion"] !== "string" ||
    !Array.isArray(value["knownRegions"])
  ) {
    throw invalid("provider", value);
  }
  const knownRegions = value["knownRegions"].map((r: unknown) => {
    if (!isRecord(r) || typeof r["id"] !== "string" || typeof r["location"] !== "string") {
      throw invalid("region", r);
    }
    return { id: r["id"], location: r["location"] };
  });
  return {
    id: value["id"],
    name: value["name"],
    endpointRequired: value["endpointRequired"],
    defaultRegion: value["defaultRegion"],
    knownRegions,
  };
}

export function parseCheck(value: unknown): ConnectionCheck {
  if (
    !isRecord(value) ||
    !oneOf(CHECK_STATUSES, value["status"]) ||
    !isNullableString(value["message"]) ||
    !isNullableString(value["bucket"]) ||
    typeof value["checkedAt"] !== "number"
  ) {
    throw invalid("connection check", value);
  }
  return {
    status: value["status"],
    message: value["message"],
    bucket: value["bucket"],
    checkedAt: value["checkedAt"],
  };
}

export function parseAccount(value: unknown): AccountInfo {
  if (!isRecord(value)) {
    throw invalid("account", value);
  }
  const { id, label, provider, providerName, region, endpoint, endpointUrl, bucket, configError, lastCheck } =
    value;
  if (
    typeof id !== "string" ||
    typeof label !== "string" ||
    typeof provider !== "string" ||
    !isNullableString(providerName) ||
    typeof region !== "string" ||
    !isNullableString(endpoint) ||
    !isNullableString(endpointUrl) ||
    !isNullableString(bucket) ||
    !isNullableString(configError)
  ) {
    throw invalid("account", value);
  }
  return {
    id,
    label,
    provider,
    providerName,
    region,
    endpoint,
    endpointUrl,
    bucket,
    configError,
    lastCheck: lastCheck === null ? null : parseCheck(lastCheck),
  };
}

export function parseAccountList(value: unknown): AccountList {
  if (!isRecord(value) || !Array.isArray(value["accounts"]) || !isNullableString(value["storageError"])) {
    throw invalid("account list", value);
  }
  return { accounts: value["accounts"].map(parseAccount), storageError: value["storageError"] };
}

function parseNamed(what: string, value: unknown): { name: string; prefix: string } {
  if (!isRecord(value) || typeof value["name"] !== "string" || typeof value["prefix"] !== "string") {
    throw invalid(what, value);
  }
  return { name: value["name"], prefix: value["prefix"] };
}

function parseFile(value: unknown): FileEntry {
  if (
    !isRecord(value) ||
    typeof value["name"] !== "string" ||
    typeof value["key"] !== "string" ||
    typeof value["size"] !== "number" ||
    !isNullableString(value["lastModified"])
  ) {
    throw invalid("file", value);
  }
  return { name: value["name"], key: value["key"], size: value["size"], lastModified: value["lastModified"] };
}

export function parseFolderListing(value: unknown): FolderListing {
  if (
    !isRecord(value) ||
    typeof value["accountId"] !== "string" ||
    typeof value["bucket"] !== "string" ||
    typeof value["prefix"] !== "string" ||
    !isNullableString(value["parent"]) ||
    !Array.isArray(value["breadcrumbs"]) ||
    !Array.isArray(value["folders"]) ||
    !Array.isArray(value["files"]) ||
    !isNullableString(value["nextPage"])
  ) {
    throw invalid("folder listing", value);
  }
  return {
    accountId: value["accountId"],
    bucket: value["bucket"],
    prefix: value["prefix"],
    parent: value["parent"],
    breadcrumbs: value["breadcrumbs"].map((b: unknown) => parseNamed("breadcrumb", b)),
    folders: value["folders"].map((f: unknown) => parseNamed("folder", f)),
    files: value["files"].map(parseFile),
    nextPage: value["nextPage"],
  };
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || typeof value === "number";
}

export function parseDownloadEvent(value: unknown): DownloadEvent {
  if (
    !isRecord(value) ||
    typeof value["transfer"] !== "string" ||
    !oneOf(TRANSFER_STATES, value["state"]) ||
    typeof value["bytes"] !== "number" ||
    !isNullableNumber(value["total"]) ||
    !(value["integrity"] === null || oneOf(INTEGRITY_CHECKS, value["integrity"]))
  ) {
    throw invalid("download event", value);
  }
  const error = value["error"];
  let parsedError: TransferError | null = null;
  if (error !== null) {
    if (!isRecord(error) || !oneOf(ERROR_KINDS, error["kind"]) || typeof error["message"] !== "string") {
      throw invalid("download event", value);
    }
    parsedError = { kind: error["kind"], message: error["message"] };
  }
  return {
    transfer: value["transfer"],
    state: value["state"],
    bytes: value["bytes"],
    total: value["total"],
    integrity: value["integrity"],
    error: parsedError,
  };
}

function parseError(what: string, value: unknown): TransferError | null {
  if (value === null) {
    return null;
  }
  if (!isRecord(value) || !oneOf(ERROR_KINDS, value["kind"]) || typeof value["message"] !== "string") {
    throw invalid(what, value);
  }
  return { kind: value["kind"], message: value["message"] };
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

export function parseUploadFile(value: unknown): UploadFile {
  if (!isRecord(value)) {
    throw invalid("upload file", value);
  }
  const { index, name, key, size, state, stage, bytes, total, started, skip, error, remote, retryable } = value;
  if (
    !isCount(index) ||
    typeof name !== "string" ||
    !isNullableString(key) ||
    !(size === null || isCount(size)) ||
    !oneOf(FILE_STATES, state) ||
    !(stage === null || oneOf(UPLOAD_STAGES, stage)) ||
    !isCount(bytes) ||
    !isCount(total) ||
    typeof started !== "boolean" ||
    !(remote === null || oneOf(REMOTE_STATES, remote)) ||
    typeof retryable !== "boolean"
  ) {
    throw invalid("upload file", value);
  }
  let parsedSkip: UploadSkip | null = null;
  if (skip !== null) {
    if (!isRecord(skip) || !oneOf(SKIP_REASONS, skip["reason"]) || typeof skip["message"] !== "string") {
      throw invalid("upload file", value);
    }
    parsedSkip = { reason: skip["reason"], message: skip["message"] };
  }
  return {
    index,
    name,
    key,
    size,
    state,
    stage,
    bytes,
    total,
    started,
    skip: parsedSkip,
    error: parseError("upload file", error),
    remote,
    retryable,
  };
}

export function parseUploadSummary(value: unknown): UploadSummary {
  if (!isRecord(value)) {
    throw invalid("upload summary", value);
  }
  const { state, count, completed, skipped, failed, cancelled, notStarted, retryable, needsAttention, error, files } =
    value;
  if (
    !oneOf(BATCH_STATES, state) ||
    !isCount(count) ||
    !isCount(completed) ||
    !isCount(skipped) ||
    !isCount(failed) ||
    !isCount(cancelled) ||
    !isCount(notStarted) ||
    !isCount(retryable) ||
    !isCount(needsAttention) ||
    !Array.isArray(files)
  ) {
    throw invalid("upload summary", value);
  }
  return {
    state,
    count,
    completed,
    skipped,
    failed,
    cancelled,
    notStarted,
    retryable,
    needsAttention,
    error: parseError("upload summary", error),
    files: files.map(parseUploadFile),
  };
}

export function parseUploadEvent(value: unknown): UploadEvent {
  if (!isRecord(value) || typeof value["batch"] !== "string" || !isCount(value["seq"])) {
    throw invalid("upload event", value);
  }
  const batch = value["batch"];
  const seq = value["seq"];
  switch (value["kind"]) {
    case "file":
      if (!isCount(value["count"])) {
        throw invalid("upload event", value);
      }
      return { batch, seq, kind: "file", count: value["count"], file: parseUploadFile(value["file"]) };
    case "cleanup":
      if (typeof value["key"] !== "string" || !oneOf(CLEANUP_OUTCOMES, value["outcome"])) {
        throw invalid("upload event", value);
      }
      return { batch, seq, kind: "cleanup", key: value["key"], outcome: value["outcome"] };
    case "finished":
      return { batch, seq, kind: "finished", summary: parseUploadSummary(value["summary"]) };
    default:
      throw invalid("upload event", value);
  }
}

export function parseUploadSelection(value: unknown): UploadSelection {
  if (
    !isRecord(value) ||
    !isNullableString(value["batch"]) ||
    typeof value["prefix"] !== "string" ||
    !Array.isArray(value["files"])
  ) {
    throw invalid("upload selection", value);
  }
  return { batch: value["batch"], prefix: value["prefix"], files: value["files"].map(parseUploadFile) };
}

export function parseDownloadStarted(value: unknown): DownloadStarted {
  if (!isRecord(value) || typeof value["transfer"] !== "string" || typeof value["fileName"] !== "string") {
    throw invalid("download start", value);
  }
  return { transfer: value["transfer"], fileName: value["fileName"] };
}

/** Turns a command's rejection into a `CloudCommandError` when it has the
 * structured shape; anything else (an IPC or permission failure) becomes an
 * `Error` with its text. */
export function toCommandError(rejection: unknown): Error {
  if (isRecord(rejection) && oneOf(ERROR_KINDS, rejection["kind"]) && typeof rejection["message"] === "string") {
    return new CloudCommandError(rejection["kind"], rejection["message"]);
  }
  return rejection instanceof Error ? rejection : new Error(String(rejection));
}

/** Invokes `cmd`; a rejection becomes a `CloudCommandError` when it has
 * Rust's `CommandError` shape. */
export async function call(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  try {
    return await invoke<unknown>(cmd, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

/** The supported providers and their presets. */
export async function listProviders(): Promise<ProviderInfo[]> {
  const value = await call(LIST_CLOUD_PROVIDERS);
  if (!Array.isArray(value)) {
    throw invalid("provider list", value);
  }
  return value.map(parseProvider);
}

/** Every configured account with its last connection check. */
export async function listAccounts(): Promise<AccountList> {
  return parseAccountList(await call(LIST_CLOUD_ACCOUNTS));
}

/** Adds an account; Rust stores the credentials in the OS credential store. */
export async function addAccount(account: AccountInput, credentials: CredentialInput): Promise<AccountInfo> {
  return parseAccount(await call(ADD_CLOUD_ACCOUNT, { account, credentials }));
}

/** Changes an account; `credentials` replaces its key, `null` keeps it. */
export async function updateAccount(
  id: string,
  account: AccountInput,
  credentials: CredentialInput | null,
): Promise<AccountInfo> {
  return parseAccount(await call(UPDATE_CLOUD_ACCOUNT, { id, account, credentials }));
}

/** Removes the account from sotyware and deletes its stored key. Never
 * touches the bucket or its objects. */
export async function removeAccount(id: string): Promise<void> {
  await call(REMOVE_CLOUD_ACCOUNT, { id });
}

/** Checks the connection to the account's bucket; resolves with the account
 * and the result, also when the check failed. */
export async function checkAccount(id: string): Promise<AccountInfo> {
  return parseAccount(await call(CHECK_CLOUD_ACCOUNT, { id }));
}

/** One page of one folder of the account's bucket (read-only). `prefix` is
 * `""` for the root or a prefix Rust returned; `page` is the previous page's
 * `nextPage`, or `null` for the first page. */
export async function listFolder(id: string, prefix: string, page: string | null): Promise<FolderListing> {
  return parseFolderListing(await call(LIST_CLOUD_FOLDER, { id, prefix, page }));
}

function progressChannel(onEvent: (event: DownloadEvent) => void): Channel<unknown> {
  const channel = new Channel<unknown>();
  channel.onmessage = (message: unknown): void => {
    onEvent(parseDownloadEvent(message));
  };
  return channel;
}

/** Asks for a destination in the native save dialog (opened by Rust) and
 * starts downloading `key` from the account's bucket, which must still be
 * `bucket`. Resolves with `null` if the dialog was cancelled (nothing
 * started). Every message about this download goes to `onEvent`, possibly
 * also before this resolves. */
export async function downloadFile(
  id: string,
  bucket: string,
  key: string,
  onEvent: (event: DownloadEvent) => void,
): Promise<DownloadStarted | null> {
  const value = await call(DOWNLOAD_CLOUD_FILE, { id, bucket, key, onEvent: progressChannel(onEvent) });
  return value === null ? null : parseDownloadStarted(value);
}

/** Repeats the failed download `transfer` to the same destination, without
 * a dialog, as a new transfer. */
export async function retryDownload(
  transfer: string,
  onEvent: (event: DownloadEvent) => void,
): Promise<DownloadStarted> {
  return parseDownloadStarted(await call(RETRY_CLOUD_DOWNLOAD, { transfer, onEvent: progressChannel(onEvent) }));
}

/** Cancels the running download `transfer`; never affects another one. */
export async function cancelDownload(transfer: string): Promise<CancelOutcome> {
  const value = await call(CANCEL_CLOUD_DOWNLOAD, { transfer });
  if (!oneOf(CANCEL_OUTCOMES, value)) {
    throw invalid("cancel outcome", value);
  }
  return value;
}

function uploadChannel(onEvent: (event: UploadEvent) => void, onInvalid: (e: Error) => void): Channel<unknown> {
  const channel = new Channel<unknown>();
  channel.onmessage = (message: unknown): void => {
    let event: UploadEvent;
    try {
      event = parseUploadEvent(message);
    } catch (e) {
      onInvalid(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    onEvent(event);
  };
  return channel;
}

/** Asks for files in the native file dialog (opened by Rust) and uploads
 * them one after another into folder `prefix` of the account's bucket,
 * which must still be `bucket`. Resolves with `null` if the dialog was
 * cancelled or returned nothing (nothing started). Every event of the batch
 * goes to `onEvent`, possibly also before this resolves; an event that does
 * not have the Rust shape goes to `onInvalid` instead. */
export async function uploadFiles(
  id: string,
  bucket: string,
  prefix: string,
  onEvent: (event: UploadEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<UploadSelection | null> {
  const value = await call(UPLOAD_CLOUD_FILES, { id, bucket, prefix, onEvent: uploadChannel(onEvent, onInvalid) });
  return value === null ? null : parseUploadSelection(value);
}

/** Starts the retryable files of the finished batch `batch` again, without
 * a dialog, as a new batch with a new id. */
export async function retryUpload(
  batch: UploadBatchId,
  onEvent: (event: UploadEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<UploadSelection> {
  return parseUploadSelection(await call(RETRY_CLOUD_UPLOAD, { batch, onEvent: uploadChannel(onEvent, onInvalid) }));
}

/** Cancels the running batch `batch`; never affects another one. */
export async function cancelUpload(batch: UploadBatchId): Promise<CancelOutcome> {
  const value = await call(CANCEL_CLOUD_UPLOAD, { batch });
  if (!oneOf(CANCEL_OUTCOMES, value)) {
    throw invalid("cancel outcome", value);
  }
  return value;
}
