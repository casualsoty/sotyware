// Typed access to the recovery view's commands (ADR-030;
// src-tauri/src/cloud/recovery.rs): the operation journal's entries from a
// session that ended unexpectedly, an inspection of each at the provider
// (reads only), dismissing an entry, and listing and aborting incomplete
// multipart uploads. As everywhere: the page never receives a local path,
// and every response is checked so a Rust / TypeScript mismatch fails
// loudly.
import { call } from "./cloud";

export const LIST_CLOUD_RECOVERY = "list_cloud_recovery";
export const INSPECT_CLOUD_RECOVERY = "inspect_cloud_recovery";
export const DISMISS_CLOUD_RECOVERY = "dismiss_cloud_recovery";
export const LIST_CLOUD_INCOMPLETE_UPLOADS = "list_cloud_incomplete_uploads";
export const ABORT_CLOUD_INCOMPLETE_UPLOAD = "abort_cloud_incomplete_upload";

export const RECOVERY_KINDS = ["upload", "download", "staging", "operation", "unknown"] as const;
export type RecoveryKind = (typeof RECOVERY_KINDS)[number];

export const VERDICTS = ["complete", "nothing-stored", "needs-attention", "local-only"] as const;
export type Verdict = (typeof VERDICTS)[number];

export const ABORT_RESULTS = ["aborted", "gone", "failed"] as const;
export type AbortResult = (typeof ABORT_RESULTS)[number];

export interface RecoveryItem {
  id: number;
  kind: RecoveryKind;
  state: string;
  accountId: string | null;
  accountLabel: string | null;
  bucket: string | null;
  key: string | null;
  fileName: string | null;
  size: number | null;
  operation: string | null;
  steps: number;
  multipart: boolean;
  startedMs: number;
  updatedMs: number;
  summary: string;
}

export interface RecoveryList {
  unavailable: string | null;
  items: RecoveryItem[];
}

export interface IncompleteInfo {
  key: string;
  uploadId: string;
  initiated: string | null;
  recorded: boolean;
}

export interface Inspection {
  id: number;
  verdict: Verdict;
  summary: string;
  counts: Record<string, number>;
  keys: Record<string, string[]>;
  notInspected: number;
  openUploads: IncompleteInfo[];
}

export interface IncompleteList {
  uploads: IncompleteInfo[];
  more: boolean;
}

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

function int(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (typeof x !== "number" || !Number.isSafeInteger(x)) {
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

function list<T>(what: string, value: unknown, each: (x: unknown) => T): T[] {
  if (!Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value.map(each);
}

export function parseRecoveryItem(value: unknown): RecoveryItem {
  const w = "recovery item";
  const v = rec(w, value);
  const kind = v["kind"];
  if (!oneOf(RECOVERY_KINDS, kind)) {
    throw invalid(w, value);
  }
  const size = v["size"];
  if (size !== null && !isCount(size)) {
    throw invalid(w, value);
  }
  return {
    id: int(w, v, "id"),
    kind,
    state: str(w, v, "state"),
    accountId: optStr(w, v, "accountId"),
    accountLabel: optStr(w, v, "accountLabel"),
    bucket: optStr(w, v, "bucket"),
    key: optStr(w, v, "key"),
    fileName: optStr(w, v, "fileName"),
    size,
    operation: optStr(w, v, "operation"),
    steps: count(w, v, "steps"),
    multipart: bool(w, v, "multipart"),
    startedMs: int(w, v, "startedMs"),
    updatedMs: int(w, v, "updatedMs"),
    summary: str(w, v, "summary"),
  };
}

export function parseRecoveryList(value: unknown): RecoveryList {
  const v = rec("recovery list", value);
  return {
    unavailable: optStr("recovery list", v, "unavailable"),
    items: list("recovery items", v["items"], parseRecoveryItem),
  };
}

export function parseIncomplete(value: unknown): IncompleteInfo {
  const w = "incomplete upload";
  const v = rec(w, value);
  return {
    key: str(w, v, "key"),
    uploadId: str(w, v, "uploadId"),
    initiated: optStr(w, v, "initiated"),
    recorded: bool(w, v, "recorded"),
  };
}

export function parseInspection(value: unknown): Inspection {
  const w = "inspection";
  const v = rec(w, value);
  const verdict = v["verdict"];
  if (!oneOf(VERDICTS, verdict)) {
    throw invalid(w, value);
  }
  const counts: Record<string, number> = {};
  for (const [k, n] of Object.entries(rec(w, v["counts"]))) {
    if (!isCount(n)) {
      throw invalid(w, value);
    }
    counts[k] = n;
  }
  const keys: Record<string, string[]> = {};
  for (const [k, l] of Object.entries(rec(w, v["keys"]))) {
    keys[k] = list(w, l, (x) => {
      if (typeof x !== "string") {
        throw invalid(w, value);
      }
      return x;
    });
  }
  return {
    id: int(w, v, "id"),
    verdict,
    summary: str(w, v, "summary"),
    counts,
    keys,
    notInspected: count(w, v, "notInspected"),
    openUploads: list(w, v["openUploads"], parseIncomplete),
  };
}

export async function listRecovery(): Promise<RecoveryList> {
  return parseRecoveryList(await call(LIST_CLOUD_RECOVERY));
}

export async function inspectRecovery(id: number): Promise<Inspection> {
  return parseInspection(await call(INSPECT_CLOUD_RECOVERY, { id }));
}

export async function dismissRecovery(id: number): Promise<void> {
  await call(DISMISS_CLOUD_RECOVERY, { id });
}

export async function listIncompleteUploads(id: string, bucket: string): Promise<IncompleteList> {
  const v = rec("incomplete list", await call(LIST_CLOUD_INCOMPLETE_UPLOADS, { id, bucket }));
  return {
    uploads: list("incomplete uploads", v["uploads"], parseIncomplete),
    more: bool("incomplete list", v, "more"),
  };
}

export async function abortIncompleteUpload(
  id: string,
  bucket: string,
  key: string,
  uploadId: string,
): Promise<AbortResult> {
  const v = await call(ABORT_CLOUD_INCOMPLETE_UPLOAD, { id, bucket, key, uploadId });
  if (!oneOf(ABORT_RESULTS, v)) {
    throw invalid("abort result", v);
  }
  return v;
}
