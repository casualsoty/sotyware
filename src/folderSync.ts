// Typed access to folder sync (ADR-043; src-tauri/src/folder_sync/mod.rs):
// keeping a folder on this computer and an account's bucket in step. The
// page chooses a folder only through the native dialog Rust opens; it gets
// the chosen path to show and confirm, and never sends a path back: later
// calls name the sync by its id, or the choice by a token kept in Rust.
// The live status arrives on a channel (`watchFolderSyncs`) whenever it
// changes. Every response is checked so a Rust / TypeScript mismatch fails
// loudly.
import { Channel } from "@tauri-apps/api/core";
import { call, ERROR_KINDS, type ErrorKind } from "./cloud";
import { formatCount } from "./format";

export const LIST_FOLDER_SYNCS = "list_folder_syncs";
export const WATCH_FOLDER_SYNCS = "watch_folder_syncs";
export const UNWATCH_FOLDER_SYNCS = "unwatch_folder_syncs";
export const CHOOSE_SYNC_FOLDER = "choose_sync_folder";
export const DISCARD_SYNC_FOLDER_CHOICE = "discard_sync_folder_choice";
export const ENABLE_FOLDER_SYNC = "enable_folder_sync";
export const SET_FOLDER_SYNC_PAUSED = "set_folder_sync_paused";
export const SYNC_FOLDER_NOW = "sync_folder_now";
export const RETRY_FOLDER_SYNC = "retry_folder_sync";
export const SET_FOLDER_SYNC_INTERVAL = "set_folder_sync_interval";
export const DISABLE_FOLDER_SYNC = "disable_folder_sync";
export const CHANGE_SYNC_FOLDER = "change_sync_folder";
export const OPEN_SYNC_FOLDER = "open_sync_folder";
export const FOLDER_SYNC_DETAILS = "folder_sync_details";
export const RESOLVE_FOLDER_SYNC_CONFLICT = "resolve_folder_sync_conflict";
export const ANSWER_FOLDER_SYNC_DELETIONS = "answer_folder_sync_deletions";

/** How often the bucket can be checked, in minutes (`model::Interval`). */
export const INTERVALS = [5, 15, 30, 60] as const;
export const DEFAULT_INTERVAL = 15;

/** `SyncState` in mod.rs. */
export const SYNC_STATES = [
  "starting",
  "scanning",
  "syncing",
  "synced",
  "attention",
  "paused",
  "offline",
  "unavailable",
  "error",
  "moving",
  "stopped",
] as const;
export type SyncState = (typeof SYNC_STATES)[number];

export interface HeldDeletions {
  /** Files that would be removed here (deleted in the bucket). */
  local: number;
  /** Objects that would be deleted in the bucket (deleted here). */
  remote: number;
  synced: number;
  askedMs: number;
}

export interface TransferInfo {
  path: string;
  kind: "upload" | "download";
  bytes: number;
  total: number;
  startedMs: number;
  speed: number | null;
}

export interface MigrationInfo {
  to: string;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  copying: boolean;
}

export interface SyncStatus {
  id: string;
  account: string;
  bucket: string;
  /** The sync folder, to show. */
  path: string;
  state: SyncState;
  message: string | null;
  error: { kind: ErrorKind; message: string } | null;
  paused: boolean;
  intervalMinutes: number;
  versioning: string | null;
  watching: boolean;
  files: number;
  bytes: number;
  folders: number;
  remoteFiles: number;
  remoteBytes: number;
  pendingUploads: number;
  pendingUploadBytes: number;
  pendingDownloads: number;
  pendingDownloadBytes: number;
  failed: number;
  skipped: number;
  conflicts: number;
  heldDeletions: HeldDeletions | null;
  trashFiles: number;
  trashBytes: number;
  freeBytes: number | null;
  diskFull: boolean;
  lastSyncedMs: number | null;
  lastCheckedMs: number | null;
  nextCheckMs: number | null;
  transfers: TransferInfo[];
  migration: MigrationInfo | null;
  activitySeq: number;
}

export interface SyncOverview {
  seq: number;
  /** The `folder-sync` feature runs. */
  running: boolean;
  storageError: string | null;
  syncs: SyncStatus[];
}

export interface FolderChoice {
  token: string;
  account: string;
  sync: string | null;
  path: string;
  files: number;
  bytes: number;
  more: boolean;
  freeBytes: number | null;
  bucketBytes: number | null;
  bucketObjects: number | null;
  bucketMeasuredMs: number | null;
  wasSyncFolder: boolean;
  versioning: string | null;
  problems: string[];
  warnings: string[];
  canMove: boolean;
}

export interface ActivityItem {
  atMs: number;
  path: string;
  op: string;
  bytes: number;
  message: string | null;
}

export interface FailedEntry {
  path: string;
  op: string;
  kind: ErrorKind;
  message: string;
  attempts: number;
  nextMs: number;
  firstMs: number;
}

export interface SkippedEntry {
  path: string;
  reason: string;
  side: "local" | "remote";
}

export interface ConflictEntry {
  path: string;
  copy: string;
  atMs: number;
}

export interface SyncDetails {
  activity: ActivityItem[];
  failed: FailedEntry[];
  skipped: SkippedEntry[];
  conflicts: ConflictEntry[];
  heldDeletions: HeldDeletions | null;
}

export interface DisableReport {
  path: string;
  trashKept: boolean;
}

export type ConflictChoice = "keep-local" | "keep-remote" | "keep-both";
export type MoveMode = "move" | "fresh";
export type Place = "folder" | "trash";

// ---------------------------------------------------------------- parsing

type Rec = Record<string, unknown>;

function invalid(what: string, value: unknown): Error {
  return new Error(`invalid ${what}: ${JSON.stringify(value)}`);
}

function rec(what: string, value: unknown): Rec {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value as Rec;
}

function num(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (typeof x !== "number" || !Number.isFinite(x)) {
    throw invalid(what, v);
  }
  return x;
}

function optNum(what: string, v: Rec, k: string): number | null {
  return v[k] === null ? null : num(what, v, k);
}

function str(what: string, v: Rec, k: string): string {
  const x = v[k];
  if (typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

function optStr(what: string, v: Rec, k: string): string | null {
  return v[k] === null ? null : str(what, v, k);
}

function bool(what: string, v: Rec, k: string): boolean {
  const x = v[k];
  if (typeof x !== "boolean") {
    throw invalid(what, v);
  }
  return x;
}

function list<T>(what: string, value: unknown, item: (x: unknown) => T): T[] {
  if (!Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value.map(item);
}

function strings(what: string, value: unknown): string[] {
  return list(what, value, (x) => {
    if (typeof x !== "string") {
      throw invalid(what, value);
    }
    return x;
  });
}

function kind(what: string, value: unknown): ErrorKind {
  if (!(ERROR_KINDS as readonly unknown[]).includes(value)) {
    throw invalid(what, value);
  }
  return value as ErrorKind;
}

function parseHeld(value: unknown): HeldDeletions | null {
  if (value === null) {
    return null;
  }
  const w = "held deletions";
  const v = rec(w, value);
  return {
    local: num(w, v, "local"),
    remote: num(w, v, "remote"),
    synced: num(w, v, "synced"),
    askedMs: num(w, v, "askedMs"),
  };
}

function parseTransfer(value: unknown): TransferInfo {
  const w = "transfer";
  const v = rec(w, value);
  const k = v["kind"];
  if (k !== "upload" && k !== "download") {
    throw invalid(w, value);
  }
  return {
    path: str(w, v, "path"),
    kind: k,
    bytes: num(w, v, "bytes"),
    total: num(w, v, "total"),
    startedMs: num(w, v, "startedMs"),
    speed: optNum(w, v, "speed"),
  };
}

function parseMigration(value: unknown): MigrationInfo | null {
  if (value === null) {
    return null;
  }
  const w = "migration";
  const v = rec(w, value);
  return {
    to: str(w, v, "to"),
    filesDone: num(w, v, "filesDone"),
    filesTotal: num(w, v, "filesTotal"),
    bytesDone: num(w, v, "bytesDone"),
    bytesTotal: num(w, v, "bytesTotal"),
    copying: bool(w, v, "copying"),
  };
}

export function parseStatus(value: unknown): SyncStatus {
  const w = "sync status";
  const v = rec(w, value);
  const state = v["state"];
  if (!(SYNC_STATES as readonly unknown[]).includes(state)) {
    throw invalid(w, value);
  }
  const error = v["error"] === null ? null : rec(w, v["error"]);
  return {
    id: str(w, v, "id"),
    account: str(w, v, "account"),
    bucket: str(w, v, "bucket"),
    path: str(w, v, "path"),
    state: state as SyncState,
    message: optStr(w, v, "message"),
    error: error === null ? null : { kind: kind(w, error["kind"]), message: str(w, error, "message") },
    paused: bool(w, v, "paused"),
    intervalMinutes: num(w, v, "intervalMinutes"),
    versioning: optStr(w, v, "versioning"),
    watching: bool(w, v, "watching"),
    files: num(w, v, "files"),
    bytes: num(w, v, "bytes"),
    folders: num(w, v, "folders"),
    remoteFiles: num(w, v, "remoteFiles"),
    remoteBytes: num(w, v, "remoteBytes"),
    pendingUploads: num(w, v, "pendingUploads"),
    pendingUploadBytes: num(w, v, "pendingUploadBytes"),
    pendingDownloads: num(w, v, "pendingDownloads"),
    pendingDownloadBytes: num(w, v, "pendingDownloadBytes"),
    failed: num(w, v, "failed"),
    skipped: num(w, v, "skipped"),
    conflicts: num(w, v, "conflicts"),
    heldDeletions: parseHeld(v["heldDeletions"]),
    trashFiles: num(w, v, "trashFiles"),
    trashBytes: num(w, v, "trashBytes"),
    freeBytes: optNum(w, v, "freeBytes"),
    diskFull: bool(w, v, "diskFull"),
    lastSyncedMs: optNum(w, v, "lastSyncedMs"),
    lastCheckedMs: optNum(w, v, "lastCheckedMs"),
    nextCheckMs: optNum(w, v, "nextCheckMs"),
    transfers: list(w, v["transfers"], parseTransfer),
    migration: parseMigration(v["migration"]),
    activitySeq: num(w, v, "activitySeq"),
  };
}

export function parseOverview(value: unknown): SyncOverview {
  const w = "sync overview";
  const v = rec(w, value);
  return {
    seq: num(w, v, "seq"),
    running: bool(w, v, "running"),
    storageError: optStr(w, v, "storageError"),
    syncs: list(w, v["syncs"], parseStatus),
  };
}

export function parseChoice(value: unknown): FolderChoice {
  const w = "folder choice";
  const v = rec(w, value);
  return {
    token: str(w, v, "token"),
    account: str(w, v, "account"),
    sync: optStr(w, v, "sync"),
    path: str(w, v, "path"),
    files: num(w, v, "files"),
    bytes: num(w, v, "bytes"),
    more: bool(w, v, "more"),
    freeBytes: optNum(w, v, "freeBytes"),
    bucketBytes: optNum(w, v, "bucketBytes"),
    bucketObjects: optNum(w, v, "bucketObjects"),
    bucketMeasuredMs: optNum(w, v, "bucketMeasuredMs"),
    wasSyncFolder: bool(w, v, "wasSyncFolder"),
    versioning: optStr(w, v, "versioning"),
    problems: strings(w, v["problems"]),
    warnings: strings(w, v["warnings"]),
    canMove: bool(w, v, "canMove"),
  };
}

export function parseDetails(value: unknown): SyncDetails {
  const w = "sync details";
  const v = rec(w, value);
  return {
    activity: list(w, v["activity"], (x) => {
      const a = rec("activity item", x);
      return {
        atMs: num("activity item", a, "atMs"),
        path: str("activity item", a, "path"),
        op: str("activity item", a, "op"),
        bytes: num("activity item", a, "bytes"),
        message: optStr("activity item", a, "message"),
      };
    }),
    failed: list(w, v["failed"], (x) => {
      const f = rec("failed item", x);
      return {
        path: str("failed item", f, "path"),
        op: str("failed item", f, "op"),
        kind: kind("failed item", f["kind"]),
        message: str("failed item", f, "message"),
        attempts: num("failed item", f, "attempts"),
        nextMs: num("failed item", f, "nextMs"),
        firstMs: num("failed item", f, "firstMs"),
      };
    }),
    skipped: list(w, v["skipped"], (x) => {
      const s = rec("skipped item", x);
      const side = s["side"];
      if (side !== "local" && side !== "remote") {
        throw invalid("skipped item", x);
      }
      return { path: str("skipped item", s, "path"), reason: str("skipped item", s, "reason"), side };
    }),
    conflicts: list(w, v["conflicts"], (x) => {
      const c = rec("conflict", x);
      return { path: str("conflict", c, "path"), copy: str("conflict", c, "copy"), atMs: num("conflict", c, "atMs") };
    }),
    heldDeletions: parseHeld(v["heldDeletions"]),
  };
}

// --------------------------------------------------------------- commands

export async function listFolderSyncs(): Promise<SyncOverview> {
  return parseOverview(await call(LIST_FOLDER_SYNCS));
}

/** Calls `onOverview` now and whenever a sync changes; returns the
 * subscription to end with `unwatchFolderSyncs`. */
export async function watchFolderSyncs(onOverview: (o: SyncOverview) => void): Promise<number> {
  const ch = new Channel<unknown>();
  ch.onmessage = (m) => onOverview(parseOverview(m));
  const id = await call(WATCH_FOLDER_SYNCS, { onEvent: ch });
  if (typeof id !== "number") {
    throw invalid("subscription", id);
  }
  return id;
}

export async function unwatchFolderSyncs(subscription: number): Promise<void> {
  await call(UNWATCH_FOLDER_SYNCS, { subscription });
}

/** Opens the native folder dialog (for a new sync of `account`, or a new
 * folder for `sync`); `null` if it was cancelled. */
export async function chooseSyncFolder(account: string, sync: string | null): Promise<FolderChoice | null> {
  const v = await call(CHOOSE_SYNC_FOLDER, { account, sync });
  return v === null ? null : parseChoice(v);
}

export async function discardSyncFolderChoice(token: string): Promise<void> {
  await call(DISCARD_SYNC_FOLDER_CHOICE, { token });
}

export async function enableFolderSync(token: string, intervalMinutes: number): Promise<SyncStatus> {
  return parseStatus(await call(ENABLE_FOLDER_SYNC, { token, intervalMinutes }));
}

export async function setFolderSyncPaused(id: string, paused: boolean): Promise<void> {
  await call(SET_FOLDER_SYNC_PAUSED, { id, paused });
}

export async function syncFolderNow(id: string): Promise<void> {
  await call(SYNC_FOLDER_NOW, { id });
}

export async function retryFolderSync(id: string): Promise<void> {
  await call(RETRY_FOLDER_SYNC, { id });
}

export async function setFolderSyncInterval(id: string, minutes: number): Promise<void> {
  await call(SET_FOLDER_SYNC_INTERVAL, { id, minutes });
}

export async function disableFolderSync(id: string): Promise<DisableReport> {
  const w = "disable report";
  const v = rec(w, await call(DISABLE_FOLDER_SYNC, { id }));
  return { path: str(w, v, "path"), trashKept: bool(w, v, "trashKept") };
}

export async function changeSyncFolder(token: string, mode: MoveMode): Promise<void> {
  await call(CHANGE_SYNC_FOLDER, { token, mode });
}

export async function openSyncFolder(id: string, place: Place): Promise<void> {
  await call(OPEN_SYNC_FOLDER, { id, place });
}

export async function folderSyncDetails(id: string): Promise<SyncDetails> {
  return parseDetails(await call(FOLDER_SYNC_DETAILS, { id }));
}

export async function resolveFolderSyncConflict(id: string, path: string, choice: ConflictChoice): Promise<void> {
  await call(RESOLVE_FOLDER_SYNC_CONFLICT, { id, path, choice });
}

export async function answerFolderSyncDeletions(id: string, allow: boolean): Promise<void> {
  await call(ANSWER_FOLDER_SYNC_DELETIONS, { id, allow });
}

// ------------------------------------------------------------- words

export type Tone = "success" | "warning" | "danger" | "info" | "neutral";

/** The state as the page names it, with its tone. */
export function stateLabel(s: SyncStatus): { label: string; tone: Tone } {
  switch (s.state) {
    case "synced":
      return { label: "Synced", tone: "success" };
    case "attention":
      return { label: s.heldDeletions !== null ? "Waiting for you" : "Synced, with problems", tone: "warning" };
    case "starting":
    case "scanning":
    case "syncing":
      return { label: "Syncing", tone: "info" };
    case "moving":
      return { label: "Moving", tone: "info" };
    case "paused":
      return { label: "Paused", tone: "neutral" };
    case "stopped":
      return { label: "Off", tone: "neutral" };
    case "offline":
      return { label: "Offline", tone: "warning" };
    case "unavailable":
      return { label: "Folder unavailable", tone: "warning" };
    case "error":
      return { label: "Error", tone: "danger" };
  }
}

/** What the sidebar says about every sync together, or null without any. */
export function overallSummary(o: SyncOverview | null): { tone: "success" | "warning" | "danger" | "info"; text: string } | null {
  if (o === null || o.syncs.length === 0) {
    return null;
  }
  if (!o.running) {
    return { tone: "warning", text: "Folder sync is switched off in Settings" };
  }
  const n = o.syncs.length;
  const errors = o.syncs.filter((s) => s.state === "error").length;
  if (errors > 0) {
    return { tone: "danger", text: errors === 1 ? "A folder sync needs attention" : `${errors} folder syncs need attention` };
  }
  const waiting = o.syncs.filter((s) => s.heldDeletions !== null || s.conflicts > 0 || s.failed > 0 || s.diskFull).length;
  const busy = o.syncs.filter((s) => ["starting", "scanning", "syncing", "moving"].includes(s.state));
  if (busy.length > 0) {
    const files = busy.reduce((t, s) => t + s.pendingUploads + s.pendingDownloads, 0);
    return { tone: "info", text: files > 0 ? `Syncing ${formatCount(files)} file${files === 1 ? "" : "s"}…` : "Syncing…" };
  }
  if (waiting > 0) {
    return { tone: "warning", text: waiting === 1 ? "A folder sync waits for you" : `${waiting} folder syncs wait for you` };
  }
  const away = o.syncs.filter((s) => s.state === "offline" || s.state === "unavailable").length;
  if (away > 0) {
    return { tone: "warning", text: away === n ? "Folder sync is offline" : `${away} of ${n} folder syncs offline` };
  }
  const paused = o.syncs.filter((s) => s.state === "paused").length;
  if (paused === n) {
    return { tone: "warning", text: n === 1 ? "Folder sync is paused" : "Every folder sync is paused" };
  }
  return { tone: "success", text: n === 1 ? "Folder up to date" : `${n} folders up to date` };
}

/** An activity operation in words. */
export const OP_LABELS: Record<string, string> = {
  upload: "Uploaded",
  download: "Downloaded",
  "delete-remote": "Deleted in the bucket",
  "delete-local": "Moved to the sync trash",
  "move-remote": "Moved in the bucket",
  "move-local": "Moved here",
  conflict: "Kept both versions",
  "conflict-copy": "Saved the bucket's version",
  "create-folder": "Created a folder",
  compare: "Compared",
  sync: "Synced",
};
