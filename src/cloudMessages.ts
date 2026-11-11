// What the Cloud Storage page says about check results and command errors.
// Chosen by the structured `status` / `kind` from Rust, never by parsing
// message text; Rust's message is shown as the detail.
import { CloudCommandError, type CheckStatus, type ErrorKind } from "./cloud";

export const CHECK_LABELS: Record<CheckStatus, string> = {
  connected: "Connected",
  "invalid-credentials": "Invalid credentials",
  "access-denied": "Access denied",
  "bucket-unavailable": "Bucket unavailable",
  network: "Network error",
  "credential-store-unavailable": "Credential store unavailable",
  "credential-store-locked": "Credential store locked",
  "credentials-missing": "Credentials missing",
  "invalid-configuration": "Configuration problem",
  error: "Check failed",
};

/** What the user can do about each result. */
export const CHECK_HINTS: Record<CheckStatus, string> = {
  connected: "The provider accepted the key for this bucket.",
  "invalid-credentials":
    "The provider does not accept this key ID or secret. Edit the account and enter the key again.",
  "access-denied":
    "The key is valid, but it is not allowed to use this bucket. Check the key's bucket and permissions at your provider.",
  "bucket-unavailable":
    "This bucket does not exist at this region and endpoint. Check the bucket name and region.",
  network: "The provider could not be reached. Check your internet connection and the endpoint, then try again.",
  "credential-store-unavailable":
    "Your system's credential store is not available, so the key cannot be read. On Linux this needs a Secret Service such as GNOME Keyring or KWallet.",
  "credential-store-locked": "The credential store is locked or the unlock prompt was dismissed. Unlock it and check again.",
  "credentials-missing": "No key is stored for this account on this computer. Edit the account and enter its key.",
  "invalid-configuration": "The account's settings cannot be used. Edit the account to fix them.",
  error: "Something else went wrong; the details are below.",
};

const ERROR_SUMMARIES: Record<ErrorKind, string> = {
  "invalid-input": "Check the form:",
  "unknown-account": "This account no longer exists:",
  storage: "The account database could not be used:",
  "credential-store-unavailable":
    "Your system's credential store is not available, so the key was not stored. sotyware never stores keys anywhere else. Details:",
  "credential-store-locked": "The credential store refused access. Unlock it (or accept its prompt) and try again. Details:",
  "credential-store": "The credential store failed:",
  partial: "Only part of the change was made:",
  other: "Something went wrong:",
};

/** A sentence for any error a cloud command rejected with. */
export function describeError(e: unknown): string {
  if (e instanceof CloudCommandError) {
    return `${ERROR_SUMMARIES[e.kind]} ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}
