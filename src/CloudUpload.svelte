<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    CloudCommandError,
    FINAL_FILE_STATES,
    cancelUpload,
    retryUpload,
    uploadFiles,
    type AccountInfo,
    type Breadcrumb,
    type CleanupOutcome,
    type TransferError,
    type UploadBatchId,
    type UploadEvent,
    type UploadFile,
    type UploadSelection,
    type UploadStage,
    type UploadSummary,
  } from "./cloud";
  import { uploadDrop, uploadFolder } from "./cloudFiles";
  import {
    FIXED_BY_EDITING,
    REMOTE_NOTES,
    SKIP_LABELS,
    describeUploadError,
    describeUploadFailure,
  } from "./cloudMessages";
  import { formatCount, formatSize, shownName } from "./format";

  // The upload panel of the cloud browser. It shows one batch (the latest)
  // exactly as Rust reports it (src-tauri/src/cloud/upload.rs): Rust opens
  // the file dialog, keeps the paths, checks the files, skips existing
  // objects, uploads one file after another, verifies and cleans up. This
  // component only starts, cancels and retries batches and renders their
  // events; it never decides a file's state, progress or retry eligibility.
  //
  // Stale events: every start or retry gets a new `seq` and a new channel,
  // and events are applied only while that `seq` is the one shown and only
  // for its batch id. Within a batch, a file event older than the last one
  // applied to that file (by the event's `seq`) is dropped, and nothing is
  // applied after `finished`. After unmount nothing is applied at all.

  interface Props {
    account: AccountInfo;
    /** Whether a batch is being chosen, started or run (read by the parent). */
    busy?: boolean;
    /** Back to the account list; `message` says why. */
    onclose: (message: string | null) => void;
    /** Back to the account list with this account's edit form open. */
    onedit: () => void;
    /** Shows a folder of the bucket (the batch's destination). */
    onshowfolder: (prefix: string, crumbs: Breadcrumb[]) => void;
  }

  let { account, busy = $bindable(false), onclose, onedit, onshowfolder }: Props = $props();

  /** Where the batch uploads to, fixed when it starts. */
  interface Destination {
    bucket: string;
    prefix: string;
    crumbs: Breadcrumb[];
  }

  interface Batch {
    /** Which start this is; events and replies of an older one are ignored. */
    seq: number;
    /** Known from the command's reply or the batch's first event. */
    batch: UploadBatchId | null;
    destination: Destination;
    /**
     * choosing: the file dialog is open; starting: a retry was requested;
     * active: the batch runs; finished: its summary arrived; nothing: no
     * file could be uploaded (no batch); refused: the command failed.
     */
    phase: "choosing" | "starting" | "active" | "finished" | "nothing" | "refused";
    /** Every file of the batch, by `index`. */
    files: UploadFile[];
    /** Per file index, the `seq` of the last event applied (0: the reply). */
    fileSeq: Record<number, number>;
    count: number;
    cleanup: { seq: number; key: string; outcome: CleanupOutcome }[];
    summary: UploadSummary | null;
    cancel: "requested" | "finishing" | "ended" | null;
    cancelError: unknown;
    /** Why the command was refused (phase "refused"). */
    error: unknown;
    /** Why a retry of this batch was refused; it stays shown. */
    retryError: unknown;
    /** The backend said this batch can no longer be retried. */
    retryClosed: boolean;
    /** Set when an event did not have the Rust shape. */
    protocolError: boolean;
    /** Whether this batch is a retry of an earlier one. */
    retry: boolean;
    /** How many files are in a final state, kept as events arrive. */
    finished: number;
    /** The file being uploaded, by `index`, kept as events arrive. */
    runningIndex: number | null;
    /** The page of rows shown when the list is longer than `ROW_LIMIT`. */
    page: number;
    /** Whether the list shows only the files that were not uploaded. */
    onlyNotUploaded: boolean;
  }

  /**
   * Rows shown at once. A folder batch can hold 50,000 files: every event
   * updates one file in O(1) and redraws at most one page, so a long batch
   * costs the same per event as a short one.
   */
  const ROW_LIMIT = 1000;

  let upload = $state<Batch | null>(null);
  let uploadSeq = 0;
  let destroyed = false;

  const active = $derived(
    upload !== null && (upload.phase === "choosing" || upload.phase === "starting" || upload.phase === "active"),
  );

  $effect(() => {
    busy = active;
  });

  const finishedCount = $derived(upload === null ? 0 : upload.finished);
  const running = $derived.by((): UploadFile | null => {
    if (upload === null || upload.summary !== null || upload.runningIndex === null) {
      return null;
    }
    const at = position(upload.files, upload.runningIndex);
    return at < 0 ? null : (upload.files[at] ?? null);
  });

  /** The files the list can show: all, or only those not uploaded. */
  const listed = $derived.by((): UploadFile[] => {
    if (upload === null) {
      return [];
    }
    return upload.onlyNotUploaded && upload.summary !== null
      ? upload.files.filter((f) => NOT_UPLOADED.has(f.state))
      : upload.files;
  });
  const pageCount = $derived(Math.max(1, Math.ceil(listed.length / ROW_LIMIT)));
  const shownPage = $derived(upload === null ? 0 : Math.min(upload.page, pageCount - 1));
  const shown = $derived(
    listed.length <= ROW_LIMIT ? listed : listed.slice(shownPage * ROW_LIMIT, (shownPage + 1) * ROW_LIMIT),
  );

  const STAGE_LABELS: Record<UploadStage, string> = {
    hashing: "Computing the SHA-256 checksum",
    sending: "Sending",
    verifying: "Verifying the stored object",
  };

  function percentOf(f: UploadFile): number | null {
    return f.total > 0 ? Math.min(100, Math.floor((f.bytes / f.total) * 100)) : null;
  }

  function current(seq: number): Batch | null {
    return !destroyed && upload !== null && upload.seq === seq ? upload : null;
  }

  function fresh(seq: number, destination: Destination, phase: Batch["phase"], retry: boolean): Batch {
    return {
      seq,
      batch: null,
      destination,
      phase,
      files: [],
      fileSeq: {},
      count: 0,
      cleanup: [],
      summary: null,
      cancel: null,
      cancelError: null,
      error: null,
      retryError: null,
      retryClosed: false,
      protocolError: false,
      retry,
      finished: 0,
      runningIndex: null,
      page: 0,
      onlyNotUploaded: false,
    };
  }

  const NOT_UPLOADED: ReadonlySet<UploadFile["state"]> = new Set(["skipped", "failed", "cancelled"]);

  /**
   * Where the file `index` is in `files` (sorted by index), or -1. Rust
   * numbers a batch's files 0, 1, 2…, so once the reply is in, a file sits
   * at its own index; before that, a binary search finds it.
   */
  function position(files: readonly UploadFile[], index: number): number {
    if (files[index]?.index === index) {
      return index;
    }
    let lo = 0;
    let hi = files.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const at = files[mid]!.index;
      if (at === index) {
        return mid;
      }
      if (at < index) {
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    return -1;
  }

  function isRunning(f: UploadFile): boolean {
    return !FINAL_FILE_STATES.has(f.state) && (f.started || f.state !== "pending");
  }

  function putFile(b: Batch, file: UploadFile): void {
    const at = position(b.files, file.index);
    const before = at === -1 ? null : b.files[at]!;
    if (at === -1) {
      // Only before the reply: at most the few files its events reached.
      b.files = [...b.files, file].sort((x, y) => x.index - y.index);
    } else {
      b.files[at] = file;
    }
    b.finished +=
      (FINAL_FILE_STATES.has(file.state) ? 1 : 0) -
      (before !== null && FINAL_FILE_STATES.has(before.state) ? 1 : 0);
    if (isRunning(file)) {
      b.runningIndex = file.index;
    } else if (b.runningIndex === file.index) {
      b.runningIndex = null;
    }
  }

  function countFinished(files: readonly UploadFile[]): number {
    let n = 0;
    for (const f of files) {
      if (FINAL_FILE_STATES.has(f.state)) {
        n += 1;
      }
    }
    return n;
  }

  function onEvent(seq: number, event: UploadEvent): void {
    const b = current(seq);
    // Another start's event, another batch's, or one after the summary.
    if (b === null || (b.batch !== null && b.batch !== event.batch) || b.summary !== null) {
      return;
    }
    if (b.phase === "refused" || b.phase === "nothing") {
      return;
    }
    b.batch = event.batch;
    if (b.phase === "choosing" || b.phase === "starting") {
      b.phase = "active";
    }
    switch (event.kind) {
      case "file": {
        // Never go back to an older state of a file.
        if (event.seq <= (b.fileSeq[event.file.index] ?? -1)) {
          return;
        }
        b.fileSeq[event.file.index] = event.seq;
        b.count = Math.max(b.count, event.count);
        putFile(b, event.file);
        return;
      }
      case "cleanup":
        if (!b.cleanup.some((c) => c.seq === event.seq)) {
          b.cleanup = [...b.cleanup, { seq: event.seq, key: event.key, outcome: event.outcome }].sort(
            (x, y) => x.seq - y.seq,
          );
        }
        return;
      case "finished":
        b.summary = event.summary;
        b.files = [...event.summary.files].sort((x, y) => x.index - y.index);
        b.finished = countFinished(b.files);
        b.runningIndex = null;
        b.count = event.summary.count;
        b.phase = "finished";
        b.cancel = null;
        return;
    }
  }

  function onInvalid(seq: number): void {
    const b = current(seq);
    if (b !== null && b.summary === null) {
      b.protocolError = true;
    }
  }

  /** Takes the command's reply; files already reported by events stay. */
  function adopt(seq: number, selection: UploadSelection): void {
    const b = current(seq);
    if (b === null) {
      // The browser went away meanwhile: nobody can see or cancel it.
      if (destroyed && selection.batch !== null) {
        void cancelUpload(selection.batch).catch(() => undefined);
      }
      return;
    }
    if (b.batch !== null && selection.batch !== null && b.batch !== selection.batch) {
      return;
    }
    b.destination.prefix = selection.prefix;
    // One merge, not one insert per file: a folder batch can be large.
    // Files that events already reported keep their newer state.
    const reported = new Map(b.files.map((f) => [f.index, f]));
    const merged: UploadFile[] = [];
    for (const file of selection.files) {
      const known = reported.get(file.index);
      if (known === undefined) {
        b.fileSeq[file.index] = 0;
        merged.push(file);
      } else {
        reported.delete(file.index);
        merged.push(known);
      }
    }
    merged.push(...reported.values());
    merged.sort((x, y) => x.index - y.index);
    b.files = merged;
    b.finished = countFinished(merged);
    b.count = Math.max(b.count, selection.files.length);
    if (selection.batch === null) {
      b.phase = "nothing";
      return;
    }
    b.batch = selection.batch;
    if (b.phase === "choosing" || b.phase === "starting") {
      b.phase = "active";
    }
  }

  /** Leaves the browser if the account is gone; true if it did. */
  function closedForMissingAccount(e: unknown): boolean {
    if (e instanceof CloudCommandError && e.kind === "unknown-account") {
      onclose(`${account.label} no longer exists; it was removed or changed elsewhere.`);
      return true;
    }
    return false;
  }

  /** Opens the file dialog for an upload into `prefix` of `bucket` (the
   * folder shown now); that folder stays the destination. */
  export async function start(bucket: string, prefix: string, crumbs: Breadcrumb[]): Promise<void> {
    await begin(bucket, prefix, crumbs, uploadFiles);
  }

  /** The same for a whole folder: Rust opens the folder dialog and walks it
   * (ADR-026); the batch, its events and its rules are the same. */
  export async function startFolder(bucket: string, prefix: string, crumbs: Breadcrumb[]): Promise<void> {
    await begin(bucket, prefix, crumbs, uploadFolder);
  }

  /** Uploads what was dropped on the window (drop `drop`, announced by
   * Rust with names only) into the folder shown (ADR-026). */
  export async function startDrop(bucket: string, prefix: string, crumbs: Breadcrumb[], drop: number): Promise<void> {
    await begin(bucket, prefix, crumbs, (id, b, p, onEvent, onInvalid) => uploadDrop(id, b, p, drop, onEvent, onInvalid));
  }

  /** Whether the dialog now open is the folder dialog. */
  let folderPick = $state(false);

  async function begin(
    bucket: string,
    prefix: string,
    crumbs: Breadcrumb[],
    pick: typeof uploadFiles,
  ): Promise<void> {
    if (active || destroyed) {
      return;
    }
    const previous = upload;
    const seq = ++uploadSeq;
    folderPick = pick === uploadFolder;
    upload = fresh(seq, { bucket, prefix, crumbs: [...crumbs] }, "choosing", false);
    try {
      const selection = await pick(
        account.id,
        bucket,
        prefix,
        (e) => onEvent(seq, e),
        () => onInvalid(seq),
      );
      if (selection === null) {
        // Dialog cancelled or empty: nothing started; show what was there.
        if (current(seq)?.phase === "choosing") {
          upload = previous;
        }
        return;
      }
      adopt(seq, selection);
    } catch (e) {
      if (closedForMissingAccount(e)) {
        return;
      }
      const b = current(seq);
      if (b !== null && b.phase === "choosing") {
        b.phase = "refused";
        b.error = e;
      }
    }
  }

  async function retry(): Promise<void> {
    const old = upload;
    if (
      active ||
      old === null ||
      old.phase !== "finished" ||
      old.batch === null ||
      old.summary === null ||
      old.summary.retryable === 0 ||
      old.retryClosed
    ) {
      return;
    }
    old.retryError = null;
    const previous = old.batch;
    const seq = ++uploadSeq;
    upload = fresh(seq, { ...old.destination, crumbs: [...old.destination.crumbs] }, "starting", true);
    try {
      adopt(
        seq,
        await retryUpload(
          previous,
          (e) => onEvent(seq, e),
          () => onInvalid(seq),
        ),
      );
    } catch (e) {
      if (closedForMissingAccount(e)) {
        return;
      }
      if (current(seq)?.phase === "starting") {
        // Nothing started: show the earlier batch again, with the reason.
        old.retryError = e;
        // "invalid-input": that batch can no longer be retried (Rust dropped
        // or never kept it); other refusals leave it retryable.
        old.retryClosed = e instanceof CloudCommandError && e.kind === "invalid-input";
        upload = old;
      }
    }
  }

  async function cancel(): Promise<void> {
    const b = upload;
    if (b === null || b.phase !== "active" || b.batch === null || b.cancel !== null) {
      return;
    }
    const seq = b.seq;
    b.cancel = "requested";
    b.cancelError = null;
    try {
      const outcome = await cancelUpload(b.batch);
      const now = current(seq);
      if (now !== null && now.phase === "active") {
        now.cancel = outcome === "finishing" ? "finishing" : outcome === "not-active" ? "ended" : "requested";
      }
    } catch (e) {
      const now = current(seq);
      if (now !== null && now.phase === "active") {
        now.cancel = null;
        now.cancelError = e;
      }
    }
  }

  function dismiss(): void {
    if (!active) {
      upload = null;
    }
  }

  function fixedByEditing(e: unknown): boolean {
    return e instanceof CloudCommandError && FIXED_BY_EDITING.has(e.kind);
  }

  function fixedByEditingKind(error: TransferError | null): boolean {
    return error !== null && FIXED_BY_EDITING.has(error.kind);
  }

  function plural(n: number, one: string, many = `${one}s`): string {
    return `${n} ${n === 1 ? one : many}`;
  }

  function sentence(text: string): string {
    return text === "" ? text : `${text.charAt(0).toUpperCase()}${text.slice(1)}${/[.!?]$/.test(text) ? "" : "."}`;
  }

  function statusText(f: UploadFile, summary: UploadSummary | null): string {
    switch (f.state) {
      case "pending":
        return f.started ? "Starting…" : "Waiting";
      case "hashing":
      case "sending":
      case "verifying": {
        const p = percentOf(f);
        return p === null ? STAGE_LABELS[f.state] : `${STAGE_LABELS[f.state]} · ${p}%`;
      }
      case "completed":
        return "Uploaded and verified";
      case "skipped":
        return f.skip === null ? "Skipped" : SKIP_LABELS[f.skip.reason];
      case "failed":
        return "Failed";
      case "cancelled":
        if (f.started) {
          return "Cancelled during its upload";
        }
        return summary?.state === "failed"
          ? "Not started: the upload stopped after an error"
          : "Not started: the upload was cancelled";
    }
  }

  function details(f: UploadFile): string[] {
    const lines: string[] = [];
    if (f.state === "skipped" && f.skip !== null) {
      lines.push(sentence(f.skip.message));
    }
    if (f.state === "failed" && f.error !== null) {
      lines.push(describeUploadFailure(f.error));
    }
    if ((f.state === "failed" || (f.state === "cancelled" && f.started)) && f.remote !== null) {
      lines.push(REMOTE_NOTES[f.remote]);
    }
    return lines;
  }

  function summaryText(s: UploadSummary): string {
    const parts = [`${s.completed} uploaded`];
    if (s.skipped > 0) {
      parts.push(`${s.skipped} skipped`);
    }
    if (s.failed > 0) {
      parts.push(`${s.failed} failed`);
    }
    if (s.cancelled - s.notStarted > 0) {
      parts.push(`${s.cancelled - s.notStarted} cancelled`);
    }
    if (s.notStarted > 0) {
      parts.push(`${s.notStarted} not started`);
    }
    const counts = `${plural(s.count, "file")}: ${parts.join(", ")}.`;
    switch (s.state) {
      case "completed":
        return `Upload finished. ${counts}`;
      case "cancelled":
        return `Upload cancelled. ${counts}`;
      case "failed":
        return `Upload stopped after an error. ${counts}`;
      case "active":
        // Never in a summary; shown as is rather than guessed.
        return `Upload still running. ${counts}`;
    }
  }

  function destinationText(d: Destination): string {
    return d.prefix === "" ? `${d.bucket} (root of the bucket)` : `${d.bucket} / ${d.prefix}`;
  }

  onDestroy(() => {
    destroyed = true;
    // Leaving the browser cancels a running batch: no page would show it.
    // A batch whose reply has not arrived yet is cancelled in `adopt`.
    const b = upload;
    if (b !== null && b.phase === "active" && b.batch !== null) {
      void cancelUpload(b.batch).catch(() => undefined);
    }
  });
</script>

{#if upload !== null}
  <div
    class="upload"
    class:failed={upload.phase === "refused" || upload.summary?.state === "failed"}
    data-upload-phase={upload.phase}
    data-batch={upload.batch ?? ""}
    aria-label="Upload"
    role="region"
  >
    <p class="upload-destination">
      <strong>{upload.retry ? "Retrying upload" : "Upload"}</strong> to
      <span class="dest">{destinationText(upload.destination)}</span>
    </p>

    {#if upload.phase === "choosing"}
      <p role="status">{folderPick ? "Choose the folder to upload…" : "Choose the files to upload…"}</p>
    {:else if upload.phase === "starting"}
      <p role="status">Starting the retry…</p>
    {:else if upload.phase === "refused"}
      <div role="alert" class="upload-result">
        <p><strong>The upload did not start. Nothing was uploaded.</strong></p>
        <p>{describeUploadError(upload.error)}</p>
      </div>
      <div class="actions">
        {#if fixedByEditing(upload.error)}
          <button type="button" onclick={onedit}>Edit account</button>
        {/if}
        <button type="button" onclick={dismiss}>Dismiss</button>
      </div>
    {:else}
      {#if upload.phase === "nothing"}
        <p role="status" class="upload-result">None of the chosen files can be uploaded, so nothing was sent.</p>
      {:else if upload.phase === "active"}
        <progress
          max={Math.max(1, upload.count)}
          value={finishedCount}
          aria-label="Files finished">{finishedCount} of {upload.count}</progress
        >
        <p role="status" class="upload-progress">
          {finishedCount} of {plural(upload.count, "file")} finished.
          {#if upload.cancel === "requested"}
            Cancelling…
          {:else if upload.cancel === "finishing"}
            The current file is already being completed at the provider and will finish; no further file starts.
          {:else if upload.cancel === "ended"}
            The upload already ended; its result follows.
          {/if}
        </p>
        {#if running !== null}
          <p class="upload-current" data-current={running.index}>
            File {running.index + 1} of {upload.count}: <strong class="name">{shownName(running.name)}</strong><br />
            {#if running.stage === null || running.state === "pending"}
              Starting…
            {:else}
              {STAGE_LABELS[running.stage]}{#if percentOf(running) !== null}&nbsp;· {percentOf(running)}%{/if}
              · {formatSize(running.bytes)} of {formatSize(running.total)}
            {/if}
          </p>
        {/if}
        {#if upload.cancelError !== null}
          <p role="alert">Could not cancel: {describeUploadError(upload.cancelError)}</p>
        {/if}
        <div class="actions">
          <button
            type="button"
            disabled={upload.batch === null || upload.cancel !== null}
            onclick={() => void cancel()}>Cancel upload</button
          >
        </div>
        <p class="hint">
          Cancelling stops the current file unless it is already being completed at the provider; files after it are not
          started.
        </p>
      {:else if upload.summary !== null}
        {@const s = upload.summary}
        <div role={s.state === "failed" ? "alert" : "status"} class="upload-result">
          <p><strong>{summaryText(s)}</strong></p>
          {#if s.error !== null}
            <p>{describeUploadFailure(s.error)}</p>
          {/if}
          {#if s.needsAttention > 0}
            <p class="attention">
              {plural(s.needsAttention, "file needs", "files need")} attention; see the notes in the list below.
            </p>
          {/if}
        </div>
      {/if}

      {#if upload.protocolError}
        <p role="alert">
          sotyware received an upload message it does not understand, so this list may be out of date.
        </p>
      {/if}

      {#if upload.cleanup.length > 0}
        <ul class="upload-cleanup" aria-label="Cleanup of earlier uploads">
          {#each upload.cleanup as c (c.seq)}
            <li data-cleanup={c.outcome}>
              {#if c.outcome === "removed"}
                The incomplete parts left by an earlier failed upload of <span class="name">{c.key}</span> were removed.
                That earlier upload was not completed.
              {:else}
                The incomplete parts left by an earlier failed upload of <span class="name">{c.key}</span> could still not
                be removed; they may remain at the provider.
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if upload.files.length > ROW_LIMIT}
        <div class="list-controls">
          {#if upload.summary !== null}
            <label>
              <input
                type="checkbox"
                checked={upload.onlyNotUploaded}
                onchange={(e) => {
                  if (upload !== null) {
                    upload.onlyNotUploaded = e.currentTarget.checked;
                    upload.page = 0;
                  }
                }}
              />
              Only files that were not uploaded
            </label>
          {/if}
          {#if listed.length > ROW_LIMIT}
            <p class="hint" role="status">
              Files {formatCount(shownPage * ROW_LIMIT + 1)}–{formatCount(Math.min(
                (shownPage + 1) * ROW_LIMIT,
                listed.length,
              ))} of {formatCount(listed.length)} listed.
            </p>
            <div class="actions">
              <button
                type="button"
                disabled={shownPage === 0}
                onclick={() => upload !== null && (upload.page = shownPage - 1)}>Previous files</button
              >
              <button
                type="button"
                disabled={shownPage >= pageCount - 1}
                onclick={() => upload !== null && (upload.page = shownPage + 1)}>Next files</button
              >
            </div>
          {/if}
        </div>
      {/if}

      {#if shown.length > 0}
        <ol class="upload-files" aria-label="Files in this upload">
          {#each shown as f (f.index)}
            <li data-index={f.index} data-state={f.state} data-started={f.started}>
              <span class="file-name name">{shownName(f.name)}</span>
              <span class="file-size">{f.size === null ? "" : formatSize(f.size)}</span>
              <span class="file-status">{statusText(f, upload.summary)}</span>
              {#each details(f) as line, i (i)}
                <span class="file-detail">{line}</span>
              {/each}
            </li>
          {/each}
        </ol>
      {/if}

      {#if upload.phase === "finished" || upload.phase === "nothing"}
        {#if upload.retryError !== null}
          <p role="alert">
            The retry did not start. {describeUploadError(upload.retryError)}
          </p>
        {/if}
        {#if upload.summary !== null && upload.summary.retryable > 0 && !upload.retryClosed}
          <p class="hint">
            Try again uploads only the files with nothing stored ({upload.summary.retryable}); each is checked again and
            skipped if an object with its name now exists.
          </p>
        {/if}
        <div class="actions">
          {#if upload.summary !== null && upload.summary.retryable > 0 && !upload.retryClosed}
            <button type="button" onclick={() => void retry()}>
              Try {plural(upload.summary.retryable, "file")} again
            </button>
          {/if}
          {#if upload.summary !== null && fixedByEditingKind(upload.summary.error)}
            <button type="button" onclick={onedit}>Edit account</button>
          {/if}
          {#if upload.summary !== null && upload.summary.completed > 0}
            <button
              type="button"
              onclick={() => upload && onshowfolder(upload.destination.prefix, upload.destination.crumbs)}
              >Show destination folder</button
            >
          {/if}
          <button type="button" onclick={dismiss}>Dismiss</button>
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .upload {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .upload.failed {
    border-color: var(--danger-border);
  }

  .upload p {
    margin: var(--space-1) 0;
    overflow-wrap: anywhere;
  }

  .upload-destination {
    font-size: var(--text-sm);
  }

  .upload-progress,
  .upload-current {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .upload progress {
    margin: var(--space-2) 0;
  }

  .dest,
  .name {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .upload .actions {
    margin-top: var(--space-2);
  }

  .hint,
  .file-size,
  .file-detail {
    color: var(--text-muted);
    font-size: var(--text-xs);
  }

  .attention {
    font-weight: var(--weight-semibold);
  }

  .list-controls {
    margin: var(--space-2) 0;
    font-size: var(--text-sm);
  }

  .upload-cleanup {
    margin: var(--space-2) 0;
    padding-left: var(--space-5);
    font-size: var(--text-sm);
  }

  /* A batch shows up to 1,000 rows at a time (more as pages): the list
     scrolls instead of pushing the folder out of view. */
  .upload-files {
    list-style: none;
    margin: var(--space-2) 0;
    padding: 0;
    max-height: 15rem;
    overflow-y: auto;
    font-size: var(--text-sm);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .upload-files li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) max-content;
    gap: 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border);
  }

  .upload-files li:last-child {
    border-bottom: none;
  }

  .file-size {
    text-align: right;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .file-status,
  .file-detail {
    grid-column: 1 / -1;
    overflow-wrap: anywhere;
  }

  .file-status {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  li[data-state="failed"] .file-status {
    color: var(--danger);
    font-weight: var(--weight-semibold);
  }

  li[data-state="completed"] .file-status {
    color: var(--success);
  }
</style>
