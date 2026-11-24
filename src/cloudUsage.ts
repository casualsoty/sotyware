// Typed access to the storage usage commands (ADR-041;
// src-tauri/src/cloud/usage.rs). A measurement lists the bucket in Rust
// (reads only) and reports on a per-call channel; the page never computes a
// price itself: Rust returns the estimate with its components and
// assumptions. Every response is checked so a Rust / TypeScript mismatch
// fails loudly.
import { Channel } from "@tauri-apps/api/core";
import { call, CANCEL_OUTCOMES, type CancelOutcome } from "./cloud";
import { LOCALE } from "./format";

export const STORAGE_USAGE_OVERVIEW = "storage_usage_overview";
export const MEASURE_STORAGE_USAGE = "measure_storage_usage";
export const CANCEL_STORAGE_USAGE = "cancel_storage_usage";

/** `Measurement` in usage.rs. */
export interface Measurement {
  account: string;
  bucket: string;
  measuredMs: number;
  durationMs: number;
  objects: number;
  bytes: number;
  versionsListed: boolean;
  noncurrentVersions: number;
  noncurrentBytes: number;
  deleteMarkers: number;
  incompleteListed: boolean;
  incompleteUploads: number;
  incompleteBytes: number;
  incompleteUnmeasured: number;
  smallObjects: number;
  smallBytes: number;
  complete: boolean;
  requests: number;
  notes: string[];
}

export interface HistoryPoint {
  measuredMs: number;
  storedBytes: number;
  objects: number;
  complete: boolean;
}

export interface AccountUsage {
  account: string;
  label: string;
  provider: string;
  providerName: string | null;
  region: string;
  bucket: string | null;
  group: string;
  latest: Measurement | null;
  history: HistoryPoint[];
}

export interface CostComponent {
  id: string;
  label: string;
  amount: number;
  detail: string;
}

export interface Estimate {
  currency: string;
  period: string;
  total: number;
  components: CostComponent[];
  assumptions: string[];
}

export interface PricingInfo {
  name: string;
  currency: string;
  checkedOn: string;
  sources: string[];
  notes: string[];
  /** Storage included in a base price (Hetzner: 1 TB). */
  includedStorageBytes: number | null;
  /** Storage free of charge (Backblaze: 10 GB). */
  freeStorageBytes: number;
}

export interface ProviderGroup {
  key: string;
  provider: string;
  providerName: string | null;
  region: string | null;
  accounts: string[];
  measured: number;
  storedBytes: number;
  objects: number;
  downloadsGb: number;
  estimate: Estimate | null;
  pricing: PricingInfo | null;
  noEstimate: string | null;
  scopeNote: string;
}

export interface UsageOverview {
  accounts: AccountUsage[];
  groups: ProviderGroup[];
  storedBytes: number;
  objects: number;
  measured: number;
  costs: { currency: string; total: number }[];
  running: { task: string; account: string; startedMs: number } | null;
  storageError: string | null;
}

export interface UsageProgress {
  phase: string;
  entries: number;
  bytes: number;
  requests: number;
}

export type UsageEvent =
  | { task: string; seq: number; type: "progress"; progress: UsageProgress }
  | {
      task: string;
      seq: number;
      type: "finished";
      measurement: Measurement | null;
      error: { kind: string; message: string } | null;
      cancelled: boolean;
      saveError: string | null;
    };

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

function n(what: string, v: Rec, k: string): number {
  const x = v[k];
  if (typeof x !== "number" || !Number.isFinite(x)) {
    throw invalid(what, v);
  }
  return x;
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

function list<T>(what: string, value: unknown, item: (x: unknown) => T): T[] {
  if (!Array.isArray(value)) {
    throw invalid(what, value);
  }
  return value.map(item);
}

export function parseMeasurement(value: unknown): Measurement {
  const w = "measurement";
  const v = rec(w, value);
  return {
    account: s(w, v, "account"),
    bucket: s(w, v, "bucket"),
    measuredMs: n(w, v, "measuredMs"),
    durationMs: n(w, v, "durationMs"),
    objects: n(w, v, "objects"),
    bytes: n(w, v, "bytes"),
    versionsListed: b(w, v, "versionsListed"),
    noncurrentVersions: n(w, v, "noncurrentVersions"),
    noncurrentBytes: n(w, v, "noncurrentBytes"),
    deleteMarkers: n(w, v, "deleteMarkers"),
    incompleteListed: b(w, v, "incompleteListed"),
    incompleteUploads: n(w, v, "incompleteUploads"),
    incompleteBytes: n(w, v, "incompleteBytes"),
    incompleteUnmeasured: n(w, v, "incompleteUnmeasured"),
    smallObjects: n(w, v, "smallObjects"),
    smallBytes: n(w, v, "smallBytes"),
    complete: b(w, v, "complete"),
    requests: n(w, v, "requests"),
    notes: strings(w, v["notes"]),
  };
}

function parseEstimate(value: unknown): Estimate | null {
  if (value === null) {
    return null;
  }
  const w = "estimate";
  const v = rec(w, value);
  return {
    currency: s(w, v, "currency"),
    period: s(w, v, "period"),
    total: n(w, v, "total"),
    components: list(w, v["components"], (c) => {
      const x = rec("cost component", c);
      return {
        id: s("cost component", x, "id"),
        label: s("cost component", x, "label"),
        amount: n("cost component", x, "amount"),
        detail: s("cost component", x, "detail"),
      };
    }),
    assumptions: strings(w, v["assumptions"]),
  };
}

function parsePricing(value: unknown): PricingInfo | null {
  if (value === null) {
    return null;
  }
  const w = "pricing";
  const v = rec(w, value);
  return {
    name: s(w, v, "name"),
    currency: s(w, v, "currency"),
    checkedOn: s(w, v, "checkedOn"),
    sources: strings(w, v["sources"]),
    notes: strings(w, v["notes"]),
    includedStorageBytes: v["includedStorageBytes"] === null ? null : n(w, v, "includedStorageBytes"),
    freeStorageBytes: n(w, v, "freeStorageBytes"),
  };
}

export function parseUsageOverview(value: unknown): UsageOverview {
  const w = "usage overview";
  const v = rec(w, value);
  const running = v["running"] === null ? null : rec(w, v["running"]);
  return {
    accounts: list(w, v["accounts"], (a) => {
      const x = rec("account usage", a);
      return {
        account: s("account usage", x, "account"),
        label: s("account usage", x, "label"),
        provider: s("account usage", x, "provider"),
        providerName: optS("account usage", x, "providerName"),
        region: s("account usage", x, "region"),
        bucket: optS("account usage", x, "bucket"),
        group: s("account usage", x, "group"),
        latest: x["latest"] === null ? null : parseMeasurement(x["latest"]),
        history: list("history", x["history"], (h) => {
          const y = rec("history point", h);
          return {
            measuredMs: n("history point", y, "measuredMs"),
            storedBytes: n("history point", y, "storedBytes"),
            objects: n("history point", y, "objects"),
            complete: b("history point", y, "complete"),
          };
        }),
      };
    }),
    groups: list(w, v["groups"], (g) => {
      const x = rec("provider group", g);
      return {
        key: s("provider group", x, "key"),
        provider: s("provider group", x, "provider"),
        providerName: optS("provider group", x, "providerName"),
        region: optS("provider group", x, "region"),
        accounts: strings("provider group", x["accounts"]),
        measured: n("provider group", x, "measured"),
        storedBytes: n("provider group", x, "storedBytes"),
        objects: n("provider group", x, "objects"),
        downloadsGb: n("provider group", x, "downloadsGb"),
        estimate: parseEstimate(x["estimate"]),
        pricing: parsePricing(x["pricing"]),
        noEstimate: optS("provider group", x, "noEstimate"),
        scopeNote: s("provider group", x, "scopeNote"),
      };
    }),
    storedBytes: n(w, v, "storedBytes"),
    objects: n(w, v, "objects"),
    measured: n(w, v, "measured"),
    costs: list(w, v["costs"], (c) => {
      const x = rec("cost total", c);
      return { currency: s("cost total", x, "currency"), total: n("cost total", x, "total") };
    }),
    running:
      running === null
        ? null
        : { task: s(w, running, "task"), account: s(w, running, "account"), startedMs: n(w, running, "startedMs") },
    storageError: optS(w, v, "storageError"),
  };
}

export function parseUsageEvent(value: unknown): UsageEvent {
  const w = "usage event";
  const v = rec(w, value);
  const task = s(w, v, "task");
  const seq = n(w, v, "seq");
  if (v["type"] === "progress") {
    const p = rec("usage progress", v["progress"]);
    return {
      task,
      seq,
      type: "progress",
      progress: {
        phase: s("usage progress", p, "phase"),
        entries: n("usage progress", p, "entries"),
        bytes: n("usage progress", p, "bytes"),
        requests: n("usage progress", p, "requests"),
      },
    };
  }
  if (v["type"] === "finished") {
    const error = v["error"] === null ? null : rec(w, v["error"]);
    return {
      task,
      seq,
      type: "finished",
      measurement: v["measurement"] === null ? null : parseMeasurement(v["measurement"]),
      error: error === null ? null : { kind: s(w, error, "kind"), message: s(w, error, "message") },
      cancelled: b(w, v, "cancelled"),
      saveError: optS(w, v, "saveError"),
    };
  }
  throw invalid(w, value);
}

/** Every account's usage and the estimates; `downloads` are the downloads
 * per 30 days assumed for each provider account group, in GB. */
export async function usageOverview(downloads: Record<string, number> = {}): Promise<UsageOverview> {
  return parseUsageOverview(await call(STORAGE_USAGE_OVERVIEW, { downloads }));
}

/** Starts measuring an account's bucket; returns the task id. */
export async function measureUsage(account: string, onEvent: (e: UsageEvent) => void): Promise<string> {
  const ch = new Channel<unknown>();
  ch.onmessage = (m) => onEvent(parseUsageEvent(m));
  const v = await call(MEASURE_STORAGE_USAGE, { account, onEvent: ch });
  if (typeof v !== "string") {
    throw invalid("task id", v);
  }
  return v;
}

export async function cancelUsage(task: string): Promise<CancelOutcome> {
  const v = await call(CANCEL_STORAGE_USAGE, { task });
  if (!(CANCEL_OUTCOMES as readonly unknown[]).includes(v)) {
    throw invalid("cancel outcome", v);
  }
  return v as CancelOutcome;
}

/** Everything a measurement found stored. */
export function storedBytes(m: Measurement): number {
  return m.bytes + m.noncurrentBytes + m.incompleteBytes;
}

/** An amount in its currency, as the page writes it ("$1.39", "€6.49"). */
export function formatMoney(amount: number, currency: string): string {
  try {
    return new Intl.NumberFormat(LOCALE, { style: "currency", currency, minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(amount);
  } catch {
    return `${currency} ${amount.toFixed(2)}`;
  }
}
