// Typed access to cloud storage accounts (src-tauri/src/cloud/dashboard.rs).
// The page reaches them only through these commands. Validation, ids,
// persistence and credentials are handled in Rust; the page sends what the
// user typed and shows what comes back. Responses are checked, so a
// Rust/TypeScript mismatch fails loudly.
//
// Credentials only ever travel from the page to Rust (`addAccount`,
// `updateAccount`); no command returns them, and nothing here keeps them.
import { invoke } from "@tauri-apps/api/core";

export const LIST_CLOUD_PROVIDERS = "list_cloud_providers";
export const LIST_CLOUD_ACCOUNTS = "list_cloud_accounts";
export const ADD_CLOUD_ACCOUNT = "add_cloud_account";
export const UPDATE_CLOUD_ACCOUNT = "update_cloud_account";
export const REMOVE_CLOUD_ACCOUNT = "remove_cloud_account";
export const CHECK_CLOUD_ACCOUNT = "check_cloud_account";

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
