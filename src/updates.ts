// Typed access to the update commands (ADR-033; src-tauri/src/updates.rs).
// The page never sees a key, a URL or a file: Rust fetches and verifies the
// signed manifest, downloads, checks and installs; the page shows the state
// and sends the user's clicks.
import { Channel } from "@tauri-apps/api/core";
import { call } from "./cloud";

export const UPDATE_STATUS = "update_status";
export const CHECK_FOR_UPDATE = "check_for_update";
export const INSTALL_UPDATE = "install_update";
export const CANCEL_UPDATE = "cancel_update";
export const POSTPONE_UPDATE = "postpone_update";

export interface AvailableUpdate {
  version: string;
  notes: string | null;
  pubDate: string | null;
}

export interface UpdateStatus {
  /** Whether this build has an update source (release key and server). */
  configured: boolean;
  current: string;
  channel: string;
  lastCheckMs: number | null;
  available: AvailableUpdate | null;
  /** The user chose "Later" for the version available. */
  postponed: boolean;
  /** The last check's failure, if it failed. */
  error: string | null;
  /** What installing now would interrupt (an upload, a backup task…). */
  blockedBy: string[];
}

export type InstallEvent =
  | { type: "progress"; downloaded: number; total: number | null }
  | { type: "verified" }
  | { type: "installing" }
  | { type: "installed" }
  | { type: "failed"; message: string }
  | { type: "cancelled" };

type Rec = Record<string, unknown>;

function invalid(what: string, value: unknown): Error {
  return new Error(`invalid ${what}: ${JSON.stringify(value)}`);
}

function rec(what: string, v: unknown): Rec {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    throw invalid(what, v);
  }
  return v as Rec;
}

function optString(v: unknown): v is string | null {
  return v === null || typeof v === "string";
}

export function parseUpdateStatus(value: unknown): UpdateStatus {
  const w = "update status";
  const v = rec(w, value);
  const available = v["available"];
  const blockedBy = v["blockedBy"];
  const lastCheckMs = v["lastCheckMs"];
  const error = v["error"];
  if (
    typeof v["configured"] !== "boolean" ||
    typeof v["current"] !== "string" ||
    typeof v["channel"] !== "string" ||
    typeof v["postponed"] !== "boolean" ||
    !(lastCheckMs === null || typeof lastCheckMs === "number") ||
    !optString(error) ||
    !Array.isArray(blockedBy) ||
    !blockedBy.every((b) => typeof b === "string")
  ) {
    throw invalid(w, value);
  }
  let parsed: AvailableUpdate | null = null;
  if (available !== null) {
    const a = rec(w, available);
    const notes = a["notes"];
    const pubDate = a["pubDate"];
    if (typeof a["version"] !== "string" || !optString(notes) || !optString(pubDate)) {
      throw invalid(w, value);
    }
    parsed = { version: a["version"], notes, pubDate };
  }
  return {
    configured: v["configured"],
    current: v["current"],
    channel: v["channel"],
    lastCheckMs,
    available: parsed,
    postponed: v["postponed"],
    error,
    blockedBy: blockedBy as string[],
  };
}

export function parseInstallEvent(value: unknown): InstallEvent {
  const w = "update event";
  const v = rec(w, value);
  switch (v["type"]) {
    case "progress": {
      const total = v["total"];
      if (typeof v["downloaded"] !== "number" || !(total === null || typeof total === "number")) {
        throw invalid(w, value);
      }
      return { type: "progress", downloaded: v["downloaded"], total };
    }
    case "verified":
    case "installing":
    case "installed":
    case "cancelled":
      return { type: v["type"] };
    case "failed":
      if (typeof v["message"] !== "string") {
        throw invalid(w, value);
      }
      return { type: "failed", message: v["message"] };
    default:
      throw invalid(w, value);
  }
}

export async function getUpdateStatus(): Promise<UpdateStatus> {
  return parseUpdateStatus(await call(UPDATE_STATUS));
}

/** Asks the update server now (also when automatic checks are off). */
export async function checkForUpdate(): Promise<UpdateStatus> {
  return parseUpdateStatus(await call(CHECK_FOR_UPDATE));
}

/** Downloads, verifies and installs the update found by the last check.
 * Refused (kind `busy`) while a transfer, file operation or backup runs. */
export async function installUpdate(onEvent: (e: InstallEvent) => void): Promise<void> {
  const ch = new Channel<unknown>();
  ch.onmessage = (m) => onEvent(parseInstallEvent(m));
  await call(INSTALL_UPDATE, { onEvent: ch });
}

/** Stops a download in progress; `false` when none was running. */
export async function cancelUpdate(): Promise<boolean> {
  return (await call(CANCEL_UPDATE)) === true;
}

export async function postponeUpdate(): Promise<void> {
  await call(POSTPONE_UPDATE);
}
