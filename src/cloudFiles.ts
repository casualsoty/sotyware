// Typed access to the file manager's commands (ADR-026): file operations
// as reviewed plans (src-tauri/src/cloud/operations.rs), the single-request
// commands (manage.rs: folders, properties, preview, search, deleted items,
// share links), downloads of several files and folders (bulk_download.rs),
// folder uploads (upload.rs) and favorites / recent items (places.rs).
//
// As in cloud.ts: the page sends keys and prefixes exactly as Rust returned
// them, never a local path, and every response is checked so a Rust /
// TypeScript mismatch fails loudly. Plans, provider tokens, version ids of
// copies and local folders stay in Rust.
import { Channel } from "@tauri-apps/api/core";
import {
  CANCEL_OUTCOMES,
  ERROR_KINDS,
  call,
  parseUploadEvent,
  parseUploadSelection,
  type CancelOutcome,
  type ErrorKind,
  type UploadEvent,
  type UploadSelection,
} from "./cloud";

export const PLAN_CLOUD_OPERATION = "plan_cloud_operation";
export const DISCARD_CLOUD_OPERATION_PLAN = "discard_cloud_operation_plan";
export const START_CLOUD_OPERATION = "start_cloud_operation";
export const CANCEL_CLOUD_OPERATION = "cancel_cloud_operation";
export const RETRY_CLOUD_OPERATION = "retry_cloud_operation";
export const CREATE_CLOUD_FOLDER = "create_cloud_folder";
export const CLOUD_OBJECT_PROPERTIES = "cloud_object_properties";
export const PREVIEW_CLOUD_FILE = "preview_cloud_file";
export const SEARCH_CLOUD_FOLDER = "search_cloud_folder";
export const LIST_CLOUD_DELETED = "list_cloud_deleted";
export const SHARE_CLOUD_FILE = "share_cloud_file";
export const UPLOAD_CLOUD_FOLDER = "upload_cloud_folder";
export const DOWNLOAD_CLOUD_ITEMS = "download_cloud_items";
export const CANCEL_CLOUD_ITEMS_DOWNLOAD = "cancel_cloud_items_download";
export const RETRY_CLOUD_ITEMS_DOWNLOAD = "retry_cloud_items_download";
export const LIST_CLOUD_PLACES = "list_cloud_places";
export const SET_CLOUD_FAVORITE = "set_cloud_favorite";
export const RECORD_CLOUD_RECENT = "record_cloud_recent";
export const CLEAR_CLOUD_RECENT = "clear_cloud_recent";
export const WATCH_CLOUD_DROPS = "watch_cloud_drops";
export const UNWATCH_CLOUD_DROPS = "unwatch_cloud_drops";
export const UPLOAD_CLOUD_DROP = "upload_cloud_drop";

// ------------------------------------------------------------------ checks

type Rec = Record<string, unknown>;

function isRecord(value: unknown): value is Rec {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function invalid(what: string, value: unknown): Error {
  return new Error(`invalid ${what}: ${JSON.stringify(value)}`);
}

function oneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return (values as readonly unknown[]).includes(value);
}

function isCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function str(what: string, v: Rec, k: string): string {
  const x = v[k];
  if (typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

function optStr(what: string, v: Rec, k: string): string | null {
  const x = v[k];
  if (x !== null && typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

function count(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (!isCount(x)) {
    throw invalid(what, v);
  }
  return x;
}

function optCount(what: string, v: Rec, k: string): number | null {
  const x = v[k];
  if (x !== null && !isCount(x)) {
    throw invalid(what, v);
  }
  return x;
}

function bool(what: string, v: Rec, k: string): boolean {
  const x = v[k];
  if (typeof x !== "boolean") {
    throw invalid(what, v);
  }
  return x;
}

function rec(what: string, value: unknown): Rec {
  if (!isRecord(value)) {
    throw invalid(what, value);
  }
  return value;
}

function list<T>(what: string, value: unknown, each: (x: unknown) => T): T[] {
  if (!Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value.map(each);
}

export interface CommandErrorInfo {
  kind: ErrorKind;
  message: string;
}

function parseErrorInfo(what: string, value: unknown): CommandErrorInfo | null {
  if (value === null) {
    return null;
  }
  const v = rec(what, value);
  if (!oneOf(ERROR_KINDS, v["kind"])) {
    throw invalid(what, value);
  }
  return { kind: v["kind"], message: str(what, v, "message") };
}

function enumOf<T extends string>(what: string, values: readonly T[], v: Rec, k: string): T {
  const x = v[k];
  if (!oneOf(values, x)) {
    throw invalid(what, v);
  }
  return x;
}

function optEnumOf<T extends string>(what: string, values: readonly T[], v: Rec, k: string): T | null {
  return v[k] === null ? null : enumOf(what, values, v, k);
}

// --------------------------------------------------------------- operations

/** A selected file or folder, exactly as a listing named it. */
export type ItemRef = { type: "file"; key: string } | { type: "folder"; prefix: string };

export interface RestoreRef {
  key: string;
  markerVersion: string;
}

export type OperationRequest =
  | { kind: "copy"; items: ItemRef[]; destination: string }
  | { kind: "move"; items: ItemRef[]; destination: string }
  | { kind: "rename"; item: ItemRef; newName: string }
  | { kind: "delete"; items: ItemRef[] }
  | { kind: "restore"; items: RestoreRef[] };

export const OPERATION_KINDS = ["copy", "move", "rename", "delete", "restore"] as const;
export type OperationKind = (typeof OPERATION_KINDS)[number];

export const VERSIONING = ["enabled", "suspended", "disabled", "unknown"] as const;
export type VersioningInfo = (typeof VERSIONING)[number];

export interface ConflictInfo {
  existing: string;
  alternative: string | null;
}

export interface PlanItem {
  index: number;
  name: string;
  folder: boolean;
  source: string;
  destination: string | null;
  objects: number;
  bytes: number;
  conflict: ConflictInfo | null;
  problem: string | null;
}

export interface PlanSummary {
  plan: string;
  kind: OperationKind;
  accountId: string;
  bucket: string;
  destination: string | null;
  items: PlanItem[];
  objects: number;
  bytes: number;
  conflicts: number;
  versioning: VersioningInfo;
}

export const ITEM_STATES = ["pending", "running", "completed", "skipped", "failed", "partial", "cancelled"] as const;
export type ItemState = (typeof ITEM_STATES)[number];

export const ITEM_NOTES = [
  "destination-exists",
  "original-kept-copy-incomplete",
  "copied-original-kept",
  "incomplete",
  "source-not-empty",
] as const;
export type ItemNote = (typeof ITEM_NOTES)[number];

export interface ObjectFailure {
  key: string;
  error: CommandErrorInfo;
}

export interface ItemReport {
  index: number;
  name: string;
  folder: boolean;
  source: string;
  destination: string | null;
  state: ItemState;
  objectsTotal: number;
  objectsDone: number;
  objectsFailed: number;
  bytesTotal: number;
  bytesDone: number;
  hidden: number;
  removed: number;
  note: ItemNote | null;
  failures: ObjectFailure[];
}

export const OPERATION_STATES = ["completed", "partial", "cancelled", "failed"] as const;
export type OperationState = (typeof OPERATION_STATES)[number];

export interface ItemCounts {
  completed: number;
  skipped: number;
  failed: number;
  partial: number;
  cancelled: number;
}

export interface OperationSummary {
  operation: string;
  kind: OperationKind;
  state: OperationState;
  items: ItemCounts;
  objectsDone: number;
  objectsFailed: number;
  hidden: number;
  removed: number;
  retryable: boolean;
  error: CommandErrorInfo | null;
}

export type OperationEvent =
  | { operation: string; seq: number; type: "item"; item: ItemReport }
  | {
      operation: string;
      seq: number;
      type: "progress";
      objectsDone: number;
      objectsTotal: number;
      bytesDone: number;
      bytesTotal: number;
    }
  | { operation: string; seq: number; type: "finished"; summary: OperationSummary };

export interface OperationStarted {
  operation: string;
  kind: OperationKind;
  items: ItemReport[];
}

function parsePlanItem(value: unknown): PlanItem {
  const w = "plan item";
  const v = rec(w, value);
  const c = v["conflict"];
  let conflict: ConflictInfo | null = null;
  if (c !== null) {
    const cv = rec(w, c);
    conflict = { existing: str(w, cv, "existing"), alternative: optStr(w, cv, "alternative") };
  }
  return {
    index: count(w, v, "index"),
    name: str(w, v, "name"),
    folder: bool(w, v, "folder"),
    source: str(w, v, "source"),
    destination: optStr(w, v, "destination"),
    objects: count(w, v, "objects"),
    bytes: count(w, v, "bytes"),
    conflict,
    problem: optStr(w, v, "problem"),
  };
}

export function parsePlan(value: unknown): PlanSummary {
  const w = "plan";
  const v = rec(w, value);
  return {
    plan: str(w, v, "plan"),
    kind: enumOf(w, OPERATION_KINDS, v, "kind"),
    accountId: str(w, v, "accountId"),
    bucket: str(w, v, "bucket"),
    destination: optStr(w, v, "destination"),
    items: list(w, v["items"], parsePlanItem),
    objects: count(w, v, "objects"),
    bytes: count(w, v, "bytes"),
    conflicts: count(w, v, "conflicts"),
    versioning: enumOf(w, VERSIONING, v, "versioning"),
  };
}

export function parseItemReport(value: unknown): ItemReport {
  const w = "operation item";
  const v = rec(w, value);
  return {
    index: count(w, v, "index"),
    name: str(w, v, "name"),
    folder: bool(w, v, "folder"),
    source: str(w, v, "source"),
    destination: optStr(w, v, "destination"),
    state: enumOf(w, ITEM_STATES, v, "state"),
    objectsTotal: count(w, v, "objectsTotal"),
    objectsDone: count(w, v, "objectsDone"),
    objectsFailed: count(w, v, "objectsFailed"),
    bytesTotal: count(w, v, "bytesTotal"),
    bytesDone: count(w, v, "bytesDone"),
    hidden: count(w, v, "hidden"),
    removed: count(w, v, "removed"),
    note: optEnumOf(w, ITEM_NOTES, v, "note"),
    failures: list(w, v["failures"], (f) => {
      const fv = rec(w, f);
      const error = parseErrorInfo(w, fv["error"]);
      if (error === null) {
        throw invalid(w, f);
      }
      return { key: str(w, fv, "key"), error };
    }),
  };
}

export function parseOperationSummary(value: unknown): OperationSummary {
  const w = "operation summary";
  const v = rec(w, value);
  const items = rec(w, v["items"]);
  return {
    operation: str(w, v, "operation"),
    kind: enumOf(w, OPERATION_KINDS, v, "kind"),
    state: enumOf(w, OPERATION_STATES, v, "state"),
    items: {
      completed: count(w, items, "completed"),
      skipped: count(w, items, "skipped"),
      failed: count(w, items, "failed"),
      partial: count(w, items, "partial"),
      cancelled: count(w, items, "cancelled"),
    },
    objectsDone: count(w, v, "objectsDone"),
    objectsFailed: count(w, v, "objectsFailed"),
    hidden: count(w, v, "hidden"),
    removed: count(w, v, "removed"),
    retryable: bool(w, v, "retryable"),
    error: parseErrorInfo(w, v["error"]),
  };
}

export function parseOperationEvent(value: unknown): OperationEvent {
  const w = "operation event";
  const v = rec(w, value);
  const operation = str(w, v, "operation");
  const seq = count(w, v, "seq");
  const body = rec(w, v["body"]);
  switch (body["type"]) {
    case "item":
      return { operation, seq, type: "item", item: parseItemReport(body["item"]) };
    case "progress":
      return {
        operation,
        seq,
        type: "progress",
        objectsDone: count(w, body, "objectsDone"),
        objectsTotal: count(w, body, "objectsTotal"),
        bytesDone: count(w, body, "bytesDone"),
        bytesTotal: count(w, body, "bytesTotal"),
      };
    case "finished":
      return { operation, seq, type: "finished", summary: parseOperationSummary(body["summary"]) };
    default:
      throw invalid(w, value);
  }
}

export function parseOperationStarted(value: unknown): OperationStarted {
  const w = "operation start";
  const v = rec(w, value);
  return {
    operation: str(w, v, "operation"),
    kind: enumOf(w, OPERATION_KINDS, v, "kind"),
    items: list(w, v["items"], parseItemReport),
  };
}

function eventChannel<T>(
  parse: (v: unknown) => T,
  onEvent: (event: T) => void,
  onInvalid: (e: Error) => void,
): Channel<unknown> {
  const channel = new Channel<unknown>();
  channel.onmessage = (message: unknown): void => {
    let event: T;
    try {
      event = parse(message);
    } catch (e) {
      onInvalid(e instanceof Error ? e : new Error(String(e)));
      return;
    }
    onEvent(event);
  };
  return channel;
}

function parseCancel(value: unknown): CancelOutcome {
  if (!oneOf(CANCEL_OUTCOMES, value)) {
    throw invalid("cancel outcome", value);
  }
  return value;
}

/** Plans an operation (reads only); nothing changes until it is started. */
export async function planOperation(id: string, bucket: string, request: OperationRequest): Promise<PlanSummary> {
  return parsePlan(await call(PLAN_CLOUD_OPERATION, { id, bucket, request }));
}

/** Forgets a plan the user did not start. */
export async function discardPlan(plan: string): Promise<void> {
  await call(DISCARD_CLOUD_OPERATION_PLAN, { plan });
}

/** Starts a reviewed plan; events go to `onEvent`. */
export async function startOperation(
  plan: string,
  conflicts: "skip" | "rename",
  onEvent: (event: OperationEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<OperationStarted> {
  return parseOperationStarted(
    await call(START_CLOUD_OPERATION, { plan, conflicts, onEvent: eventChannel(parseOperationEvent, onEvent, onInvalid) }),
  );
}

export async function cancelOperation(operation: string): Promise<CancelOutcome> {
  return parseCancel(await call(CANCEL_CLOUD_OPERATION, { operation }));
}

export async function retryOperation(
  operation: string,
  onEvent: (event: OperationEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<OperationStarted> {
  return parseOperationStarted(
    await call(RETRY_CLOUD_OPERATION, { operation, onEvent: eventChannel(parseOperationEvent, onEvent, onInvalid) }),
  );
}

// ------------------------------------------------------- single requests

export interface CreatedFolder {
  name: string;
  prefix: string;
}

export async function createFolder(id: string, bucket: string, prefix: string, name: string): Promise<CreatedFolder> {
  const v = rec("folder", await call(CREATE_CLOUD_FOLDER, { id, bucket, prefix, name }));
  return { name: str("folder", v, "name"), prefix: str("folder", v, "prefix") };
}

export interface Properties {
  key: string;
  name: string;
  folder: string;
  bucket: string;
  size: number;
  lastModified: string | null;
  contentType: string | null;
  eTag: string | null;
  versionId: string | null;
  storageClass: string | null;
  cacheControl: string | null;
  contentEncoding: string | null;
  contentDisposition: string | null;
  sha256: string | null;
  metadata: [string, string][];
}

export function parseProperties(value: unknown): Properties {
  const w = "properties";
  const v = rec(w, value);
  return {
    key: str(w, v, "key"),
    name: str(w, v, "name"),
    folder: str(w, v, "folder"),
    bucket: str(w, v, "bucket"),
    size: count(w, v, "size"),
    lastModified: optStr(w, v, "lastModified"),
    contentType: optStr(w, v, "contentType"),
    eTag: optStr(w, v, "eTag"),
    versionId: optStr(w, v, "versionId"),
    storageClass: optStr(w, v, "storageClass"),
    cacheControl: optStr(w, v, "cacheControl"),
    contentEncoding: optStr(w, v, "contentEncoding"),
    contentDisposition: optStr(w, v, "contentDisposition"),
    sha256: optStr(w, v, "sha256"),
    metadata: list(w, v["metadata"], (pair) => {
      if (!Array.isArray(pair) || pair.length !== 2 || typeof pair[0] !== "string" || typeof pair[1] !== "string") {
        throw invalid(w, v);
      }
      return [pair[0], pair[1]] as [string, string];
    }),
  };
}

export async function objectProperties(id: string, bucket: string, key: string): Promise<Properties> {
  return parseProperties(await call(CLOUD_OBJECT_PROPERTIES, { id, bucket, key }));
}

export const PREVIEW_KINDS = ["text", "image", "unsupported", "too-large"] as const;
export type PreviewKind = (typeof PREVIEW_KINDS)[number];
/** The only image types a preview may show (never SVG). */
export const PREVIEW_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp"] as const;

export interface Preview {
  kind: PreviewKind;
  size: number;
  mime: string | null;
  text: string | null;
  data: string | null;
  truncated: boolean;
}

export function parsePreview(value: unknown): Preview {
  const w = "preview";
  const v = rec(w, value);
  const p: Preview = {
    kind: enumOf(w, PREVIEW_KINDS, v, "kind"),
    size: count(w, v, "size"),
    mime: optStr(w, v, "mime"),
    text: optStr(w, v, "text"),
    data: optStr(w, v, "data"),
    truncated: bool(w, v, "truncated"),
  };
  // An image of another type would not be shown anyway; refuse it here.
  if (p.kind === "image" && !oneOf(PREVIEW_IMAGE_TYPES, p.mime)) {
    throw invalid(w, value);
  }
  return p;
}

export async function previewFile(id: string, bucket: string, key: string): Promise<Preview> {
  return parsePreview(await call(PREVIEW_CLOUD_FILE, { id, bucket, key }));
}

export type SearchMode = "name" | "prefix";

export interface SearchHit {
  folder: boolean;
  name: string;
  path: string;
  parent: string;
  size: number | null;
  lastModified: string | null;
}

export interface SearchResults {
  hits: SearchHit[];
  scanned: number;
  next: string | null;
}

export function parseSearchResults(value: unknown): SearchResults {
  const w = "search results";
  const v = rec(w, value);
  return {
    hits: list(w, v["hits"], (h) => {
      const hv = rec(w, h);
      return {
        folder: bool(w, hv, "folder"),
        name: str(w, hv, "name"),
        path: str(w, hv, "path"),
        parent: str(w, hv, "parent"),
        size: optCount(w, hv, "size"),
        lastModified: optStr(w, hv, "lastModified"),
      };
    }),
    scanned: count(w, v, "scanned"),
    next: optStr(w, v, "next"),
  };
}

export async function searchFolder(
  id: string,
  bucket: string,
  prefix: string,
  query: string,
  mode: SearchMode,
  cursor: string | null,
): Promise<SearchResults> {
  return parseSearchResults(await call(SEARCH_CLOUD_FOLDER, { id, bucket, prefix, query, mode, cursor }));
}

export interface DeletedEntry {
  key: string;
  name: string;
  parent: string;
  deletedAt: string | null;
  markerVersion: string;
  restorable: boolean;
  size: number | null;
  versions: number;
}

export interface DeletedListing {
  entries: DeletedEntry[];
  truncated: boolean;
}

export function parseDeleted(value: unknown): DeletedListing {
  const w = "deleted items";
  const v = rec(w, value);
  return {
    entries: list(w, v["entries"], (e) => {
      const ev = rec(w, e);
      return {
        key: str(w, ev, "key"),
        name: str(w, ev, "name"),
        parent: str(w, ev, "parent"),
        deletedAt: optStr(w, ev, "deletedAt"),
        markerVersion: str(w, ev, "markerVersion"),
        restorable: bool(w, ev, "restorable"),
        size: optCount(w, ev, "size"),
        versions: count(w, ev, "versions"),
      };
    }),
    truncated: bool(w, v, "truncated"),
  };
}

export async function listDeleted(id: string, bucket: string, prefix: string): Promise<DeletedListing> {
  return parseDeleted(await call(LIST_CLOUD_DELETED, { id, bucket, prefix }));
}

export const LINK_LIFETIMES = [
  { seconds: 3600, label: "1 hour" },
  { seconds: 86400, label: "1 day" },
  { seconds: 604800, label: "7 days" },
] as const;

export interface ShareLink {
  url: string;
  expiresAt: string;
  expiresIn: number;
}

export async function shareFile(id: string, bucket: string, key: string, expiresIn: number): Promise<ShareLink> {
  const w = "share link";
  const v = rec(w, await call(SHARE_CLOUD_FILE, { id, bucket, key, expiresIn }));
  const url = str(w, v, "url");
  if (!/^https?:\/\//.test(url)) {
    throw invalid(w, v);
  }
  return { url, expiresAt: str(w, v, "expiresAt"), expiresIn: count(w, v, "expiresIn") };
}

// --------------------------------------------------------- folder upload

/** Asks for a folder in the native folder dialog (opened by Rust) and
 * uploads it into `prefix`, as an upload batch (same events as
 * `uploadFiles`). `null` if the dialog was cancelled. */
export async function uploadFolder(
  id: string,
  bucket: string,
  prefix: string,
  onEvent: (event: UploadEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<UploadSelection | null> {
  const value = await call(UPLOAD_CLOUD_FOLDER, {
    id,
    bucket,
    prefix,
    onEvent: eventChannel(parseUploadEvent, onEvent, onInvalid),
  });
  return value === null ? null : parseUploadSelection(value);
}

// ------------------------------------------------------------ bulk download

export const BULK_FILE_STATES = ["pending", "downloading", "completed", "failed", "cancelled"] as const;
export type BulkFileState = (typeof BULK_FILE_STATES)[number];
export const BULK_ITEM_STATES = ["pending", "downloading", "saved", "failed", "cancelled"] as const;
export type BulkItemState = (typeof BULK_ITEM_STATES)[number];
export const BULK_STATES = ["completed", "partial", "cancelled", "failed"] as const;
export type BulkState = (typeof BULK_STATES)[number];

export interface BulkFile {
  index: number;
  item: number;
  name: string;
  key: string;
  size: number;
  state: BulkFileState;
  bytes: number;
  integrity: "sha256-verified" | "size-only" | "transfer-only" | null;
  error: CommandErrorInfo | null;
}

export interface BulkItem {
  index: number;
  name: string;
  folder: boolean;
  files: number;
  bytes: number;
  state: BulkItemState;
  savedAs: string | null;
  error: CommandErrorInfo | null;
}

export interface BulkSummary {
  batch: string;
  state: BulkState;
  itemsSaved: number;
  itemsFailed: number;
  itemsCancelled: number;
  filesCompleted: number;
  bytes: number;
  retryable: boolean;
  error: CommandErrorInfo | null;
}

export type BulkEvent =
  | { batch: string; seq: number; type: "file"; file: BulkFile }
  | { batch: string; seq: number; type: "item"; item: BulkItem }
  | { batch: string; seq: number; type: "finished"; summary: BulkSummary };

export interface BulkStarted {
  batch: string;
  folderName: string;
  items: BulkItem[];
  files: BulkFile[];
}

function parseBulkFile(value: unknown): BulkFile {
  const w = "download file";
  const v = rec(w, value);
  const integrity = v["integrity"];
  if (!(integrity === null || integrity === "sha256-verified" || integrity === "size-only" || integrity === "transfer-only")) {
    throw invalid(w, value);
  }
  return {
    index: count(w, v, "index"),
    item: count(w, v, "item"),
    name: str(w, v, "name"),
    key: str(w, v, "key"),
    size: count(w, v, "size"),
    state: enumOf(w, BULK_FILE_STATES, v, "state"),
    bytes: count(w, v, "bytes"),
    integrity,
    error: parseErrorInfo(w, v["error"]),
  };
}

function parseBulkItem(value: unknown): BulkItem {
  const w = "download item";
  const v = rec(w, value);
  return {
    index: count(w, v, "index"),
    name: str(w, v, "name"),
    folder: bool(w, v, "folder"),
    files: count(w, v, "files"),
    bytes: count(w, v, "bytes"),
    state: enumOf(w, BULK_ITEM_STATES, v, "state"),
    savedAs: optStr(w, v, "savedAs"),
    error: parseErrorInfo(w, v["error"]),
  };
}

export function parseBulkEvent(value: unknown): BulkEvent {
  const w = "download event";
  const v = rec(w, value);
  const batch = str(w, v, "batch");
  const seq = count(w, v, "seq");
  const body = rec(w, v["body"]);
  switch (body["type"]) {
    case "file":
      return { batch, seq, type: "file", file: parseBulkFile(body["file"]) };
    case "item":
      return { batch, seq, type: "item", item: parseBulkItem(body["item"]) };
    case "finished": {
      const s = rec(w, body["summary"]);
      return {
        batch,
        seq,
        type: "finished",
        summary: {
          batch: str(w, s, "batch"),
          state: enumOf(w, BULK_STATES, s, "state"),
          itemsSaved: count(w, s, "itemsSaved"),
          itemsFailed: count(w, s, "itemsFailed"),
          itemsCancelled: count(w, s, "itemsCancelled"),
          filesCompleted: count(w, s, "filesCompleted"),
          bytes: count(w, s, "bytes"),
          retryable: bool(w, s, "retryable"),
          error: parseErrorInfo(w, s["error"]),
        },
      };
    }
    default:
      throw invalid(w, value);
  }
}

export function parseBulkStarted(value: unknown): BulkStarted {
  const w = "download start";
  const v = rec(w, value);
  return {
    batch: str(w, v, "batch"),
    folderName: str(w, v, "folderName"),
    items: list(w, v["items"], parseBulkItem),
    files: list(w, v["files"], parseBulkFile),
  };
}

/** Asks for a folder in the native folder dialog (opened by Rust) and
 * downloads the items into it. `null` if the dialog was cancelled. */
export async function downloadItems(
  id: string,
  bucket: string,
  items: ItemRef[],
  onEvent: (event: BulkEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<BulkStarted | null> {
  const value = await call(DOWNLOAD_CLOUD_ITEMS, {
    id,
    bucket,
    items,
    onEvent: eventChannel(parseBulkEvent, onEvent, onInvalid),
  });
  return value === null ? null : parseBulkStarted(value);
}

export async function cancelItemsDownload(batch: string): Promise<CancelOutcome> {
  return parseCancel(await call(CANCEL_CLOUD_ITEMS_DOWNLOAD, { batch }));
}

export async function retryItemsDownload(
  batch: string,
  onEvent: (event: BulkEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<BulkStarted> {
  return parseBulkStarted(
    await call(RETRY_CLOUD_ITEMS_DOWNLOAD, { batch, onEvent: eventChannel(parseBulkEvent, onEvent, onInvalid) }),
  );
}

// ------------------------------------------------------------------ places

export interface Place {
  path: string;
  bucket: string;
  folder: boolean;
  name: string;
  parent: string;
  touched: number;
}

export interface Places {
  favorites: Place[];
  recent: Place[];
}

export function parsePlaces(value: unknown): Places {
  const w = "places";
  const v = rec(w, value);
  const place = (p: unknown): Place => {
    const pv = rec(w, p);
    return {
      path: str(w, pv, "path"),
      bucket: str(w, pv, "bucket"),
      folder: bool(w, pv, "folder"),
      name: str(w, pv, "name"),
      parent: str(w, pv, "parent"),
      touched: count(w, pv, "touched"),
    };
  };
  return { favorites: list(w, v["favorites"], place), recent: list(w, v["recent"], place) };
}

export async function listPlaces(id: string): Promise<Places> {
  return parsePlaces(await call(LIST_CLOUD_PLACES, { id }));
}

export async function setFavorite(id: string, bucket: string, path: string, favorite: boolean): Promise<Places> {
  return parsePlaces(await call(SET_CLOUD_FAVORITE, { id, bucket, path, favorite }));
}

export async function recordRecent(id: string, bucket: string, path: string): Promise<void> {
  await call(RECORD_CLOUD_RECENT, { id, bucket, path });
}

export async function clearRecent(id: string): Promise<Places> {
  return parsePlaces(await call(CLEAR_CLOUD_RECENT, { id }));
}

// ------------------------------------------------------------ drag and drop

/** Files and folders dropped on the window: names and counts only. */
export interface DropNotice {
  drop: number;
  files: number;
  folders: number;
  names: string[];
}

export function parseDropNotice(value: unknown): DropNotice {
  const w = "drop notice";
  const v = rec(w, value);
  return {
    drop: count(w, v, "drop"),
    files: count(w, v, "files"),
    folders: count(w, v, "folders"),
    names: list(w, v["names"], (n) => {
      if (typeof n !== "string") {
        throw invalid(w, value);
      }
      return n;
    }),
  };
}

/** Tells Rust that this page wants to hear about drops (while it shows a
 * bucket). Invalid notices are ignored. */
export async function watchDrops(onDrop: (notice: DropNotice) => void): Promise<void> {
  await call(WATCH_CLOUD_DROPS, { onDrop: eventChannel(parseDropNotice, onDrop, () => undefined) });
}

export async function unwatchDrops(): Promise<void> {
  await call(UNWATCH_CLOUD_DROPS);
}

/** Uploads drop `drop` into `prefix`, as an upload batch. */
export async function uploadDrop(
  id: string,
  bucket: string,
  prefix: string,
  drop: number,
  onEvent: (event: UploadEvent) => void,
  onInvalid: (e: Error) => void,
): Promise<UploadSelection> {
  return parseUploadSelection(
    await call(UPLOAD_CLOUD_DROP, { id, bucket, prefix, drop, onEvent: eventChannel(parseUploadEvent, onEvent, onInvalid) }),
  );
}
