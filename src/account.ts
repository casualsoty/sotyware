// Typed access to the sotyware account commands (ADR-032;
// src-tauri/src/account/mod.rs). Tokens and keys never reach the page; the
// recovery key arrives exactly once (set up, rotation) to be shown, and a
// typed recovery key is sent to unlock or to check it.
import { call } from "./cloud";

export interface ConflictInfo {
  key: string;
  of: string;
  summary: string;
}

export interface AccountStatus {
  configured: boolean;
  service: string | null;
  signedIn: boolean;
  email: string | null;
  deviceId: string | null;
  vaultAccess: "enrolled" | "pending" | "none" | "unknown";
  lastSyncMs: number | null;
  lastSyncError: string | null;
  conflicts: ConflictInfo[];
  storageError: string | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  platform: string;
  state: "pending" | "enrolled" | "revoked";
  createdMs: number;
  lastSeenMs: number;
  current: boolean;
}

export interface EnrollmentInfo {
  id: string;
  deviceName: string;
  platform: string;
  createdMs: number;
  fingerprint: string;
}

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

function optS(what: string, v: Rec, k: string): string | null {
  const x = v[k];
  if (x !== null && typeof x !== "string") {
    throw invalid(what, v);
  }
  return x;
}

export function parseStatus(value: unknown): AccountStatus {
  const w = "account status";
  const v = rec(w, value);
  const access = v["vaultAccess"];
  if (
    typeof v["configured"] !== "boolean" ||
    typeof v["signedIn"] !== "boolean" ||
    !(access === "enrolled" || access === "pending" || access === "none" || access === "unknown") ||
    !Array.isArray(v["conflicts"]) ||
    !(v["lastSyncMs"] === null || typeof v["lastSyncMs"] === "number")
  ) {
    throw invalid(w, value);
  }
  return {
    configured: v["configured"],
    service: optS(w, v, "service"),
    signedIn: v["signedIn"],
    email: optS(w, v, "email"),
    deviceId: optS(w, v, "deviceId"),
    vaultAccess: access,
    lastSyncMs: v["lastSyncMs"],
    lastSyncError: optS(w, v, "lastSyncError"),
    conflicts: v["conflicts"].map((c: unknown) => {
      const x = rec("conflict", c);
      if (typeof x["key"] !== "string" || typeof x["of"] !== "string" || typeof x["summary"] !== "string") {
        throw invalid("conflict", c);
      }
      return { key: x["key"], of: x["of"], summary: x["summary"] };
    }),
    storageError: optS(w, v, "storageError"),
  };
}

export async function accountStatus(): Promise<AccountStatus> {
  return parseStatus(await call("account_status"));
}

export async function register(email: string, password: string): Promise<void> {
  await call("account_register", { email, password });
}

export async function verifyEmail(email: string, code: string, password: string): Promise<void> {
  await call("account_verify", { email, code, password });
}

/** Returns this computer's vault access after signing in. */
export async function signIn(email: string, password: string, deviceName: string): Promise<string> {
  const v = await call("account_sign_in", { email, password, deviceName });
  if (typeof v !== "string") {
    throw invalid("vault access", v);
  }
  return v;
}

export async function signOut(): Promise<void> {
  await call("account_sign_out");
}

/** Sets up sync; returns the recovery key, to show once. */
export async function setUpSync(): Promise<string> {
  const v = await call("account_set_up_sync");
  if (typeof v !== "string") {
    throw invalid("recovery key", v);
  }
  return v;
}

export async function checkRecoveryKey(key: string): Promise<boolean> {
  return (await call("account_check_recovery_key", { key })) === true;
}

export async function unlock(key: string): Promise<void> {
  await call("account_unlock", { key });
}

export async function requestApproval(): Promise<{ id: string; fingerprint: string }> {
  const v = rec("approval request", await call("account_request_approval"));
  if (typeof v["id"] !== "string" || typeof v["fingerprint"] !== "string") {
    throw invalid("approval request", v);
  }
  return { id: v["id"], fingerprint: v["fingerprint"] };
}

export async function approvalStatus(id: string): Promise<string> {
  const v = await call("account_approval_status", { id });
  if (typeof v !== "string") {
    throw invalid("approval state", v);
  }
  return v;
}

export async function enrollments(): Promise<EnrollmentInfo[]> {
  const v = await call("account_enrollments");
  if (!Array.isArray(v)) {
    throw invalid("enrollments", v);
  }
  return v.map((e: unknown) => {
    const x = rec("enrollment", e);
    return {
      id: String(x["id"]),
      deviceName: String(x["deviceName"]),
      platform: String(x["platform"]),
      createdMs: Number(x["createdMs"]),
      fingerprint: String(x["fingerprint"]),
    };
  });
}

export async function answerEnrollment(id: string, approve: boolean): Promise<void> {
  await call("account_answer_enrollment", { id, approve });
}

export async function devices(): Promise<DeviceInfo[]> {
  const v = await call("account_devices");
  if (!Array.isArray(v)) {
    throw invalid("devices", v);
  }
  return v.map((d: unknown) => {
    const x = rec("device", d);
    const state = x["state"];
    if (!(state === "pending" || state === "enrolled" || state === "revoked")) {
      throw invalid("device", d);
    }
    return {
      id: String(x["id"]),
      name: String(x["name"]),
      platform: String(x["platform"]),
      state,
      createdMs: Number(x["createdMs"]),
      lastSeenMs: Number(x["lastSeenMs"]),
      current: x["current"] === true,
    };
  });
}

export async function revokeDevice(id: string): Promise<void> {
  await call("account_revoke_device", { id });
}

/** Rotates the account key; returns a new recovery key when none was given. */
export async function rotateKeys(recoveryKey: string | null): Promise<string | null> {
  const v = await call("account_rotate_keys", { recoveryKey });
  if (v !== null && typeof v !== "string") {
    throw invalid("recovery key", v);
  }
  return v;
}

export async function syncNow(): Promise<void> {
  await call("account_sync");
}

export async function resolveConflict(key: string, keepLocal: boolean): Promise<void> {
  await call("account_resolve_conflict", { key, keepLocal });
}

export async function changePassword(current: string, next: string): Promise<void> {
  await call("account_change_password", { current, new: next });
}

export async function requestReset(email: string): Promise<void> {
  await call("account_reset_password", { step: { stage: "request", email } });
}

export async function confirmReset(email: string, code: string, newPassword: string): Promise<void> {
  await call("account_reset_password", { step: { stage: "confirm", email, code, newPassword } });
}

export async function deleteAccount(password: string): Promise<void> {
  await call("account_delete", { password });
}
