// Typed access to the backup commands (ADR-031; src-tauri/src/backup/mod.rs).
// The page never sends or receives a local path: restores into a folder and
// recovery exports go through dialogs Rust opens. Every response is checked
// so a Rust / TypeScript mismatch fails loudly.
import { Channel } from "@tauri-apps/api/core";
import { call, type CancelOutcome } from "./cloud";

export const LIST_BACKUPS = "list_backups";
export const SAVE_BACKUP_JOB = "save_backup_job";
export const REMOVE_BACKUP_JOB = "remove_backup_job";
export const START_BACKUP_TASK = "start_backup_task";
export const CANCEL_BACKUP_TASK = "cancel_backup_task";
export const LIST_BACKUP_SNAPSHOTS = "list_backup_snapshots";
export const RESTORE_BACKUP = "restore_backup";
export const EXPORT_BACKUP_MANIFEST = "export_backup_manifest";
export const CHECK_BACKUP_PROTECTION = "check_backup_protection";
export const BACKUP_SNAPSHOT_DETAILS = "backup_snapshot_details";
export const DELETE_BACKUP_SNAPSHOT = "delete_backup_snapshot";

export const HEALTHS = [
  "healthy",
  "never-run",
  "not-verified",
  "verification-failed",
  "partial",
  "failed",
  "out-of-date",
  "large-change",
] as const;
export type Health = (typeof HEALTHS)[number];

// How the pages name a backup's health (Backups page, Cloud Storage's
// Snapshots & backups tab).
export const HEALTH_TEXT: Record<Health, string> = {
  healthy: "Healthy: the newest backup was verified.",
  "never-run": "Never run.",
  "not-verified": "Not verified: the newest backup has not passed verification yet.",
  "verification-failed": "Verification failed: backup data is missing or damaged. Run a backup again and check the destination.",
  partial: "Partial: the last run did not complete, so no new snapshot was written. Older snapshots are still there.",
  failed: "Failed: the last run could not back up anything.",
  "out-of-date": "Out of date: no verified backup within twice the schedule interval.",
  "large-change":
    "Large change detected: many files changed or disappeared since the previous snapshot. Older snapshots are kept until you review it and run the next backup.",
};

export const HEALTH_SHORT: Record<Health, string> = {
  healthy: "Healthy",
  "never-run": "Never run",
  "not-verified": "Not verified",
  "verification-failed": "Verification failed",
  partial: "Partial",
  failed: "Failed",
  "out-of-date": "Out of date",
  "large-change": "Large change",
};

export const HEALTH_TONES: Record<Health, string> = {
  healthy: "success",
  "never-run": "",
  "not-verified": "warning",
  "verification-failed": "danger",
  partial: "warning",
  failed: "danger",
  "out-of-date": "warning",
  "large-change": "warning",
};

export const TASK_KINDS = ["backup", "verify", "verify-full", "prune", "restore", "delete-snapshot"] as const;
export type TaskKind = (typeof TASK_KINDS)[number];

export interface Location {
  account: string;
  bucket: string;
  prefix: string;
}

export interface Retention {
  keepLast: number;
  keepDaily: number;
  keepWeekly: number;
}

export interface JobConfig {
  id: string;
  name: string;
  source: Location;
  destination: Location;
  intervalHours: number;
  retention: Retention;
  suspiciousPercent: number;
}

export interface RunInfo {
  kind: string;
  state: string;
  startedMs: number;
  finishedMs: number | null;
  snapshot: string | null;
  suspicious: boolean;
  passed: boolean | null;
  error: string | null;
  summary: string | null;
}

export interface JobStatus {
  config: JobConfig;
  health: Health;
  sameAccount: boolean;
  lastBackup: RunInfo | null;
  lastVerification: RunInfo | null;
  lastPrune: RunInfo | null;
  lastSnapshot: string | null;
  nextDueMs: number | null;
}

export interface ActiveTask {
  task: string;
  job: string;
  kind: TaskKind;
}

export interface BackupList {
  storageError: string | null;
  jobs: JobStatus[];
  active: ActiveTask | null;
}

export interface JobInput {
  id: string | null;
  name: string;
  sourceAccount: string;
  sourcePrefix: string;
  destinationAccount: string;
  destinationPrefix: string;
  intervalHours: number;
  retention: Retention;
  suspiciousPercent: number;
}

export interface Progress {
  phase: string;
  done: number;
  total: number;
  bytes: number;
}

export interface Stats {
  objects: number;
  bytes: number;
  new: number;
  changed: number;
  deleted: number;
  transferred: number;
  transferredBytes: number;
}

export interface ObjectError {
  key: string;
  message: string;
}

export interface VerifyReport {
  snapshot: string;
  deep: boolean;
  blobs: number;
  bytes: number;
  missing: string[];
  damaged: string[];
  passed: boolean;
}

export interface PruneReport {
  snapshotsKept: number;
  snapshotsDeleted: number;
  blobsDeleted: number;
  bytesDeleted: number;
  refused: number;
}

export interface RestoreReport {
  restored: number;
  bytes: number;
  skipped: string[];
  renamed: [string, string][];
  failed: ObjectError[];
}

export interface TaskSummary {
  task: string;
  job: string;
  kind: TaskKind;
  state: string;
  snapshot: string | null;
  stats: Stats | null;
  suspicious: boolean;
  verification: VerifyReport | null;
  prune: PruneReport | null;
  restore: RestoreReport | null;
  pruneSkipped: string | null;
  objectErrors: ObjectError[];
  error: { kind: string; message: string } | null;
}

export type BackupEvent =
  | { task: string; seq: number; type: "progress"; progress: Progress }
  | { task: string; seq: number; type: "finished"; summary: TaskSummary };

export interface SnapshotRef {
  key: string;
  created: string;
  size: number;
}

export interface Protection {
  sameAccount: boolean;
  versioning: string;
  objectLock: boolean;
  defaultMode: string | null;
  defaultDays: number | null;
  immutable: boolean;
  summary: string;
}

export type RestoreTarget = { kind: "local" } | { kind: "bucket"; account: string; prefix: string };

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

function oneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return (values as readonly unknown[]).includes(value);
}

function n(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (typeof x !== "number" || !Number.isFinite(x)) {
    throw invalid(what, v);
  }
  return x;
}

function optN(what: string, v: Rec, k: string): number | null {
  return v[k] === null ? null : n(what, v, k);
}

function s(what: string, v: Rec, k: string): string {
  const x = v[k];
  if (typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

function optS(what: string, v: Rec, k: string): string | null {
  return v[k] === null ? null : s(what, v, k);
}

function b(what: string, v: Rec, k: string): boolean {
  const x = v[k];
  if (typeof x !== "boolean") {
    throw invalid(what, v);
  }
  return x;
}

function strings(what: string, value: unknown): string[] {
  if (!Array.isArray(value) || !value.every((x) => typeof x === "string")) {
    throw invalid(what, value);
  }
  return value as string[];
}

function location(value: unknown): Location {
  const v = rec("location", value);
  return { account: s("location", v, "account"), bucket: s("location", v, "bucket"), prefix: s("location", v, "prefix") };
}

function retention(value: unknown): Retention {
  const v = rec("retention", value);
  return { keepLast: n("retention", v, "keepLast"), keepDaily: n("retention", v, "keepDaily"), keepWeekly: n("retention", v, "keepWeekly") };
}

export function parseJob(value: unknown): JobConfig {
  const v = rec("backup job", value);
  return {
    id: s("backup job", v, "id"),
    name: s("backup job", v, "name"),
    source: location(v["source"]),
    destination: location(v["destination"]),
    intervalHours: n("backup job", v, "intervalHours"),
    retention: retention(v["retention"]),
    suspiciousPercent: n("backup job", v, "suspiciousPercent"),
  };
}

function runInfo(value: unknown): RunInfo | null {
  if (value === null) {
    return null;
  }
  const w = "backup run";
  const v = rec(w, value);
  const passed = v["passed"];
  if (passed !== null && typeof passed !== "boolean") {
    throw invalid(w, value);
  }
  return {
    kind: s(w, v, "kind"),
    state: s(w, v, "state"),
    startedMs: n(w, v, "startedMs"),
    finishedMs: optN(w, v, "finishedMs"),
    snapshot: optS(w, v, "snapshot"),
    suspicious: b(w, v, "suspicious"),
    passed,
    error: optS(w, v, "error"),
    summary: optS(w, v, "summary"),
  };
}

export function parseBackupList(value: unknown): BackupList {
  const v = rec("backup list", value);
  if (!Array.isArray(v["jobs"])) {
    throw invalid("backup list", value);
  }
  const jobs = v["jobs"].map((j: unknown): JobStatus => {
    const x = rec("job status", j);
    const health = x["health"];
    if (!oneOf(HEALTHS, health)) {
      throw invalid("job status", j);
    }
    return {
      config: parseJob(x["config"]),
      health,
      sameAccount: b("job status", x, "sameAccount"),
      lastBackup: runInfo(x["lastBackup"]),
      lastVerification: runInfo(x["lastVerification"]),
      lastPrune: runInfo(x["lastPrune"]),
      lastSnapshot: optS("job status", x, "lastSnapshot"),
      nextDueMs: optN("job status", x, "nextDueMs"),
    };
  });
  let active: ActiveTask | null = null;
  if (v["active"] !== null) {
    const a = rec("active task", v["active"]);
    const kind = a["kind"];
    if (!oneOf(TASK_KINDS, kind)) {
      throw invalid("active task", v["active"]);
    }
    active = { task: s("active task", a, "task"), job: s("active task", a, "job"), kind };
  }
  return { storageError: optS("backup list", v, "storageError"), jobs, active };
}

function parseSummary(value: unknown): TaskSummary {
  const w = "task summary";
  const v = rec(w, value);
  const kind = v["kind"];
  if (!oneOf(TASK_KINDS, kind) || !Array.isArray(v["objectErrors"])) {
    throw invalid(w, value);
  }
  const error = v["error"] === null ? null : rec(w, v["error"]);
  return {
    task: s(w, v, "task"),
    job: s(w, v, "job"),
    kind,
    state: s(w, v, "state"),
    snapshot: optS(w, v, "snapshot"),
    stats: v["stats"] === null ? null : (rec(w, v["stats"]) as unknown as Stats),
    suspicious: b(w, v, "suspicious"),
    verification: v["verification"] === null ? null : (rec(w, v["verification"]) as unknown as VerifyReport),
    prune: v["prune"] === null ? null : (rec(w, v["prune"]) as unknown as PruneReport),
    restore: v["restore"] === null ? null : (rec(w, v["restore"]) as unknown as RestoreReport),
    pruneSkipped: optS(w, v, "pruneSkipped"),
    objectErrors: v["objectErrors"].map((e: unknown) => {
      const x = rec(w, e);
      return { key: s(w, x, "key"), message: s(w, x, "message") };
    }),
    error: error === null ? null : { kind: s(w, error, "kind"), message: s(w, error, "message") },
  };
}

export function parseBackupEvent(value: unknown): BackupEvent {
  const v = rec("backup event", value);
  const task = s("backup event", v, "task");
  const seq = n("backup event", v, "seq");
  if (v["type"] === "progress") {
    const p = rec("progress", v["progress"]);
    return {
      task,
      seq,
      type: "progress",
      progress: { phase: s("progress", p, "phase"), done: n("progress", p, "done"), total: n("progress", p, "total"), bytes: n("progress", p, "bytes") },
    };
  }
  if (v["type"] === "finished") {
    return { task, seq, type: "finished", summary: parseSummary(v["summary"]) };
  }
  throw invalid("backup event", value);
}

export async function listBackups(): Promise<BackupList> {
  return parseBackupList(await call(LIST_BACKUPS));
}

export async function saveBackupJob(job: JobInput): Promise<JobConfig> {
  return parseJob(await call(SAVE_BACKUP_JOB, { job }));
}

export async function removeBackupJob(id: string): Promise<void> {
  await call(REMOVE_BACKUP_JOB, { id });
}

function channel(onEvent: (e: BackupEvent) => void): Channel<unknown> {
  const ch = new Channel<unknown>();
  ch.onmessage = (m) => onEvent(parseBackupEvent(m));
  return ch;
}

export async function startBackupTask(job: string, kind: Exclude<TaskKind, "restore">, onEvent: (e: BackupEvent) => void): Promise<string> {
  const v = await call(START_BACKUP_TASK, { job, kind, onEvent: channel(onEvent) });
  if (typeof v !== "string") {
    throw invalid("task id", v);
  }
  return v;
}

export async function cancelBackupTask(task: string): Promise<CancelOutcome> {
  return (await call(CANCEL_BACKUP_TASK, { task })) as CancelOutcome;
}

export async function listBackupSnapshots(job: string): Promise<SnapshotRef[]> {
  const v = await call(LIST_BACKUP_SNAPSHOTS, { job });
  if (!Array.isArray(v)) {
    throw invalid("snapshot list", v);
  }
  return v.map((x: unknown) => {
    const r = rec("snapshot", x);
    return { key: s("snapshot", r, "key"), created: s("snapshot", r, "created"), size: n("snapshot", r, "size") };
  });
}

/** Starts a restore; `null` when the user cancelled the folder dialog. */
export async function restoreBackup(
  job: string,
  snapshot: string,
  filter: string,
  target: RestoreTarget,
  onEvent: (e: BackupEvent) => void,
): Promise<string | null> {
  const v = await call(RESTORE_BACKUP, { job, snapshot, filter, target, onEvent: channel(onEvent) });
  if (v !== null && typeof v !== "string") {
    throw invalid("task id", v);
  }
  return v;
}

/** Saves a recovery file; its name, or `null` when the dialog was cancelled. */
export async function exportBackupManifest(job: string, snapshot: string): Promise<string | null> {
  const v = await call(EXPORT_BACKUP_MANIFEST, { job, snapshot });
  if (v !== null && typeof v !== "string") {
    throw invalid("file name", v);
  }
  return v;
}

export async function checkBackupProtection(job: string): Promise<Protection> {
  const v = rec("protection", await call(CHECK_BACKUP_PROTECTION, { job }));
  return {
    sameAccount: b("protection", v, "sameAccount"),
    versioning: s("protection", v, "versioning"),
    objectLock: b("protection", v, "objectLock"),
    defaultMode: optS("protection", v, "defaultMode"),
    defaultDays: optN("protection", v, "defaultDays"),
    immutable: b("protection", v, "immutable"),
    summary: s("protection", v, "summary"),
  };
}

/** What a snapshot's manifest says (`SnapshotDetails` in backup/mod.rs). */
export interface SnapshotDetails {
  key: string;
  id: string;
  created: string;
  sourceProvider: string;
  sourceBucket: string;
  sourcePrefix: string;
  stats: Stats;
  suspicious: boolean;
  /** The newest verification on this computer: passed, when, every byte. */
  verified: boolean | null;
  verifiedMs: number | null;
  verifiedFull: boolean;
}

export async function backupSnapshotDetails(job: string, snapshot: string): Promise<SnapshotDetails> {
  const w = "snapshot details";
  const v = rec(w, await call(BACKUP_SNAPSHOT_DETAILS, { job, snapshot }));
  const st = rec(w, v["stats"]);
  const verified = v["verified"];
  if (verified !== null && typeof verified !== "boolean") {
    throw invalid(w, v);
  }
  return {
    key: s(w, v, "key"),
    id: s(w, v, "id"),
    created: s(w, v, "created"),
    sourceProvider: s(w, v, "sourceProvider"),
    sourceBucket: s(w, v, "sourceBucket"),
    sourcePrefix: s(w, v, "sourcePrefix"),
    stats: {
      objects: n(w, st, "objects"),
      bytes: n(w, st, "bytes"),
      new: n(w, st, "new"),
      changed: n(w, st, "changed"),
      deleted: n(w, st, "deleted"),
      transferred: n(w, st, "transferred"),
      transferredBytes: n(w, st, "transferredBytes"),
    },
    suspicious: b(w, v, "suspicious"),
    verified,
    verifiedMs: optN(w, v, "verifiedMs"),
    verifiedFull: b(w, v, "verifiedFull"),
  };
}

/** Deletes one snapshot (its manifest, then data no other snapshot uses);
 * the result comes as a task's events. */
export async function deleteBackupSnapshot(job: string, snapshot: string, onEvent: (e: BackupEvent) => void): Promise<string> {
  const v = await call(DELETE_BACKUP_SNAPSHOT, { job, snapshot, onEvent: channel(onEvent) });
  if (typeof v !== "string") {
    throw invalid("task id", v);
  }
  return v;
}

export { strings as parseStrings };
