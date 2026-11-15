// What the Cloud Storage page says about check results and command errors.
// Chosen by the structured `status` / `kind` from Rust, never by parsing
// message text; Rust's message is shown as the detail.
import {
  CloudCommandError,
  type CheckStatus,
  type ErrorKind,
  type RemoteState,
  type SkipReason,
  type TransferError,
} from "./cloud";

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
  "credentials-missing": "No key is stored for this account on this computer. Edit the account and enter its key again. Details:",
  "invalid-credentials":
    "The provider does not accept this account's key ID or secret. Edit the account and enter the key again. Details:",
  "access-denied":
    "The key is valid, but the provider refused this request. A key limited to a file name prefix can only list inside that prefix; check the key's bucket, prefix and permissions at your provider. Details:",
  "bucket-unavailable":
    "The bucket could not be found with this account's settings, or this key may not see it. Check the bucket name and region. Details:",
  network: "The provider could not be reached. Check your internet connection and the endpoint, then try again. Details:",
  timeout: "The provider did not answer in time. Try again. Details:",
  "invalid-configuration": "The account's settings cannot be used. Edit the account to fix them. Details:",
  provider: "The provider could not complete the request:",
  "outdated-page": "This list is out of date. Refresh to load it again. Details:",
  busy: "Another download is running. Wait for it to finish or cancel it first. Details:",
  "object-not-found": "The file could not be found. It may have been deleted or renamed; refresh the folder. Details:",
  integrity: "The downloaded data did not pass the integrity check, so it was not saved. Try again. Details:",
  "disk-full": "The destination drive is full. Free some space or choose another location. Details:",
  "local-permission": "sotyware may not write to this location. Choose another folder. Details:",
  "destination-unavailable": "The destination folder is not available (removed, or a drive was disconnected). Details:",
  "destination-exists":
    "A file with this name appeared while downloading. It was not replaced; download again and choose to replace it. Details:",
  "local-file": "The file could not be written. Details:",
  "file-changed": "The file changed while it was being uploaded, so it was not stored. Try again once it is saved. Details:",
  "remote-changed": "The object changed in the bucket since it was listed, so it was left as it is. Refresh and try again. Details:",
  "too-large": "The object is too large for this action. Details:",
  "outdated-plan": "This action was reviewed too long ago or replaced by another one. Start it again to review it. Details:",
  journal:
    "sotyware could not record this step in its operation journal, so it did not start it (an unrecorded step could not be recovered after a crash). Check the free space of your system drive. Details:",
  other: "Something went wrong:",
};

/** Errors the user fixes by editing the account (re-entering its key). */
export const FIXED_BY_EDITING: ReadonlySet<ErrorKind> = new Set<ErrorKind>([
  "credentials-missing",
  "invalid-credentials",
  "invalid-configuration",
]);

/** A sentence for any error a cloud command rejected with. */
export function describeError(e: unknown): string {
  if (e instanceof CloudCommandError) {
    return `${ERROR_SUMMARIES[e.kind]} ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}

// Uploads (src-tauri/src/cloud/upload.rs). Chosen by the structured skip
// reason, error kind and remote state; Rust's sentence is the detail. None of
// these says "try again" by itself: whether a retry is possible is decided by
// the backend (`retryable`) and said next to the Try again button.

/** Why a file was not uploaded, by `SkipReason`. */
export const SKIP_LABELS: Record<SkipReason, string> = {
  "not-a-file": "Skipped: not a file",
  unreadable: "Skipped: could not be read",
  "invalid-name": "Skipped: name cannot be an object key",
  "control-characters": "Skipped: name contains control characters",
  "key-too-long": "Skipped: name too long for this folder",
  "too-large": "Skipped: too large for this provider",
  "duplicate-key": "Skipped: another selected file has the same name",
  "destination-exists": "Skipped: already in this folder",
};

/** What a failed or cancelled upload may have left at the provider. */
export const REMOTE_NOTES: Record<RemoteState, string> = {
  "nothing-stored": "Nothing was stored for this file.",
  "incomplete-parts":
    "The incomplete upload could not be removed, so parts of it may remain at the provider (and may be billed). sotyware tries to remove them again when this account's upload is retried and when sotyware quits; otherwise only a lifecycle rule on the bucket removes them.",
  "needs-inspection":
    "The provider may have stored this file, but sotyware could not verify it. Check the object in the bucket before uploading it again; sotyware does not retry or delete it.",
};

const UPLOAD_ERROR_SUMMARIES: Record<ErrorKind, string> = {
  "invalid-input": "The upload could not be started:",
  "unknown-account": "This account no longer exists:",
  storage: "The account database could not be used:",
  "credential-store-unavailable":
    "Your system's credential store is not available, so the account's key could not be read. Details:",
  "credential-store-locked": "The credential store refused access. Unlock it (or accept its prompt). Details:",
  "credential-store": "The credential store failed:",
  partial: "Only part of the change was made:",
  "credentials-missing": "No key is stored for this account on this computer. Edit the account and enter its key. Details:",
  "invalid-credentials":
    "The provider does not accept this account's key ID or secret. Edit the account and enter the key again. Details:",
  "access-denied":
    "The key is valid, but the provider refused to store files with it. Check that the key may write to this bucket (and to this folder, if it is limited to a file name prefix). Details:",
  "bucket-unavailable":
    "The bucket could not be found with this account's settings, or this key may not use it. Details:",
  network: "The provider could not be reached, or the connection broke off. Check your internet connection. Details:",
  timeout: "The provider stopped taking data or did not answer in time. Details:",
  "invalid-configuration": "The account's settings cannot be used for uploading. Edit the account to fix them. Details:",
  provider: "The provider refused the upload:",
  "outdated-page": "This account's bucket changed since the folder was listed. Refresh, then choose the files again. Details:",
  busy: "Another upload is running. Wait for it to finish or cancel it first. Details:",
  "object-not-found": "The object could not be found:",
  integrity: "The stored object did not match the file's size or SHA-256 checksum. Details:",
  "disk-full": "The local drive is full:",
  "local-permission": "sotyware may not read this file:",
  "destination-unavailable": "The file's folder is not available:",
  "destination-exists": "An object with this name is already in the folder; it was left as it is. Details:",
  "local-file": "The file could not be read on this computer. Details:",
  "file-changed": "The file changed while it was being uploaded, so its upload was stopped. Details:",
  "remote-changed": "The object changed in the bucket meanwhile. Details:",
  "too-large": "The file is too large. Details:",
  "outdated-plan": "This action is out of date. Details:",
  journal:
    "sotyware could not record this upload in its operation journal, so it did not start it. Check the free space of your system drive. Details:",
  other: "Something went wrong:",
};

/** A sentence for an upload command's rejection or a file's error. */
export function describeUploadError(e: unknown): string {
  if (e instanceof CloudCommandError) {
    return `${UPLOAD_ERROR_SUMMARIES[e.kind]} ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}

/** The same for an error that came inside an upload event. */
export function describeUploadFailure(error: TransferError): string {
  return `${UPLOAD_ERROR_SUMMARIES[error.kind]} ${error.message}`;
}

// File operations, downloads of several items and the other file-manager
// commands (ADR-026; src-tauri/src/cloud/operations.rs, manage.rs,
// bulk_download.rs, places.rs).

const OPERATION_ERROR_SUMMARIES: Record<ErrorKind, string> = {
  ...ERROR_SUMMARIES,
  "invalid-input": "This cannot be done:",
  "access-denied":
    "The key is valid, but the provider refused this change. Check that the key may write and delete in this bucket. Details:",
  "object-not-found": "The object could not be found. It may have been deleted or renamed meanwhile; refresh the folder. Details:",
  "destination-exists": "Something with this name already exists there; nothing was replaced. Details:",
  integrity: "A copy did not match the original, so the original was kept. Details:",
  busy: "Another action of this kind is running. Wait for it to finish or cancel it first. Details:",
  "outdated-page": "The account's bucket changed since this folder was listed. Refresh and try again. Details:",
};

/** A sentence for a file-manager command's rejection or an item's error. */
export function describeOperationError(e: unknown): string {
  if (e instanceof CloudCommandError) {
    return `${OPERATION_ERROR_SUMMARIES[e.kind]} ${e.message}`;
  }
  return e instanceof Error ? e.message : String(e);
}

/** The same for an error that came inside an event. */
export function describeOperationFailure(error: TransferError): string {
  return `${OPERATION_ERROR_SUMMARIES[error.kind]} ${error.message}`;
}

/** The colour meaning of a check result: green when connected, amber when
 * the user must act locally (unlock, re-enter a key), red when the provider
 * refused, grey when never checked. */
export function checkTone(status: CheckStatus | null): "success" | "warning" | "danger" | "neutral" {
  switch (status) {
    case null:
      return "neutral";
    case "connected":
      return "success";
    case "credential-store-locked":
    case "credentials-missing":
    case "credential-store-unavailable":
    case "network":
      return "warning";
    default:
      return "danger";
  }
}
