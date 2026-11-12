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

export const INTEGRITY_CHECKS = ["sha256-verified", "size-only"] as const;
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

async function call(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
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
