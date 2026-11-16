<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    HEALTH_SHORT,
    HEALTH_TEXT,
    HEALTH_TONES,
    backupSnapshotDetails,
    cancelBackupTask,
    deleteBackupSnapshot,
    listBackupSnapshots,
    listBackups,
    startBackupTask,
    type BackupEvent,
    type BackupList,
    type JobStatus,
    type Progress,
    type SnapshotDetails,
    type SnapshotRef,
    type TaskSummary,
  } from "./backup";
  import { listAccounts, type AccountInfo } from "./cloud";
  import { describeError } from "./cloudMessages";
  import { formatDecimalSize } from "./format";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // The "Snapshots & backups" tab of the bucket view (ADR-041,
  // docs/architecture/cloud-storage.md#snapshots-and-backups-of-a-bucket).
  // Two different things, never mixed up:
  // - the provider's own snapshots (Backblaze B2: ZIP files made in its web
  //   console). Backblaze has no API for them, so this tab only explains
  //   them;
  // - sotyware's backup snapshots (ADR-031): manifests written by backup
  //   jobs of this bucket into another destination. Listed, inspected,
  //   created ("Back up now"), verified and deleted here.

  interface Props {
    account: AccountInfo;
    onopenbackups?: (() => void) | undefined;
  }

  let { account, onopenbackups }: Props = $props();

  type Loaded<T> = { state: "loading" } | { state: "ready"; value: T } | { state: "error"; message: string };

  let list = $state<BackupList | null>(null);
  let labels = $state<Record<string, string>>({});
  let loadError = $state<string | null>(null);
  let snapshots = $state<Record<string, Loaded<SnapshotRef[]>>>({});
  let details = $state<Record<string, Loaded<SnapshotDetails>>>({});
  let expanded = $state<string | null>(null);
  let running = $state<{ job: string; kind: string; task: string | null; progress: Progress | null; cancelling: boolean } | null>(null);
  let notice = $state<{ tone: "success" | "warning" | "danger"; text: string } | null>(null);
  let confirm = $state<{ job: JobStatus; snapshot: SnapshotRef; newest: boolean; only: boolean; typed: string; error: string | null } | null>(
    null,
  );
  let destroyed = false;
  let lastSeq = 0;

  const protecting = $derived(list?.jobs.filter((j) => j.config.source.account === account.id) ?? []);
  const holding = $derived(list?.jobs.filter((j) => j.config.destination.account === account.id) ?? []);
  const otherTask = $derived(list?.active !== null && list?.active !== undefined && running === null);
  const name = $derived(account.providerName ?? account.provider);

  function label(id: string): string {
    return labels[id] ?? id;
  }

  async function load(): Promise<void> {
    try {
      const [backups, accounts] = await Promise.all([listBackups(), listAccounts()]);
      if (destroyed) {
        return;
      }
      list = backups;
      labels = Object.fromEntries(accounts.accounts.map((a) => [a.id, a.label]));
      loadError = backups.storageError === null ? null : `The backup jobs could not be read: ${backups.storageError}`;
      for (const j of backups.jobs.filter((x) => x.config.source.account === account.id)) {
        void loadSnapshots(j.config.id);
      }
    } catch (e) {
      loadError = `Could not read the backup jobs: ${describeError(e)}`;
    }
  }

  async function loadSnapshots(job: string): Promise<void> {
    snapshots[job] = { state: "loading" };
    try {
      const value = await listBackupSnapshots(job);
      if (!destroyed) {
        snapshots[job] = { state: "ready", value: [...value].reverse() };
      }
    } catch (e) {
      snapshots[job] = { state: "error", message: describeError(e) };
    }
  }

  async function toggleDetails(job: string, snap: SnapshotRef): Promise<void> {
    if (expanded === snap.key) {
      expanded = null;
      return;
    }
    expanded = snap.key;
    if (details[snap.key]?.state === "ready") {
      return;
    }
    details[snap.key] = { state: "loading" };
    try {
      details[snap.key] = { state: "ready", value: await backupSnapshotDetails(job, snap.key) };
    } catch (e) {
      details[snap.key] = { state: "error", message: describeError(e) };
    }
  }

  function onEvent(e: BackupEvent): void {
    if (destroyed || running === null || (running.task !== null && e.task !== running.task) || e.seq <= lastSeq) {
      return;
    }
    lastSeq = e.seq;
    if (e.type === "progress") {
      running.progress = e.progress;
      return;
    }
    finish(e.summary);
  }

  function finish(s: TaskSummary): void {
    const job = running?.job ?? s.job;
    running = null;
    if (s.kind === "delete-snapshot") {
      confirm = null;
    }
    if (s.state === "cancelled") {
      notice = { tone: "warning", text: "Cancelled." };
    } else if (s.error !== null) {
      notice = { tone: "danger", text: `${KIND_DONE[s.kind] ?? "The task"} failed: ${s.error.message}` };
    } else if (s.kind === "backup") {
      const st = s.stats;
      notice = {
        tone: s.state === "completed" ? "success" : "warning",
        text:
          s.state === "completed" && st !== null
            ? `New snapshot written: ${st.objects.toLocaleString()} files, ${formatDecimalSize(st.bytes)}; ${st.transferred.toLocaleString()} transferred.${s.verification?.passed ? " Verified." : ""}`
            : "The backup did not complete; no new snapshot was written.",
      };
    } else if (s.kind === "verify" || s.kind === "verify-full") {
      notice = s.verification?.passed
        ? { tone: "success", text: "The newest snapshot passed verification." }
        : { tone: "danger", text: "The newest snapshot did not pass verification: backup data is missing or damaged." };
    } else if (s.kind === "delete-snapshot") {
      const p = s.prune;
      notice = {
        tone: "success",
        text:
          p === null
            ? "Snapshot deleted."
            : `Snapshot deleted. ${p.blobsDeleted.toLocaleString()} piece${p.blobsDeleted === 1 ? "" : "s"} of data no other snapshot used ${p.blobsDeleted === 1 ? "was" : "were"} removed (${formatDecimalSize(p.bytesDeleted)}); newer unused data is removed by a later deletion or retention, once it is a day old.`,
      };
    }
    void load().then(() => loadSnapshots(job));
  }

  const KIND_DONE: Record<string, string> = {
    backup: "The backup",
    verify: "The verification",
    "verify-full": "The verification",
    "delete-snapshot": "Deleting the snapshot",
  };

  async function start(job: JobStatus, kind: "backup" | "verify"): Promise<void> {
    notice = null;
    lastSeq = 0;
    running = { job: job.config.id, kind, task: null, progress: null, cancelling: false };
    try {
      const task = await startBackupTask(job.config.id, kind, onEvent);
      if (running !== null) {
        running.task = task;
      }
    } catch (e) {
      running = null;
      notice = { tone: "danger", text: `${KIND_DONE[kind]} could not start: ${describeError(e)}` };
    }
  }

  async function cancel(): Promise<void> {
    if (running?.task) {
      running.cancelling = true;
      await cancelBackupTask(running.task).catch(() => undefined);
    }
  }

  function askDelete(job: JobStatus, snap: SnapshotRef, all: SnapshotRef[]): void {
    notice = null;
    confirm = { job, snapshot: snap, newest: all[0]?.key === snap.key, only: all.length === 1, typed: "", error: null };
  }

  async function confirmDelete(): Promise<void> {
    if (confirm === null || confirm.typed.trim().toLowerCase() !== "delete") {
      return;
    }
    const { job, snapshot } = confirm;
    confirm.error = null;
    lastSeq = 0;
    running = { job: job.config.id, kind: "delete-snapshot", task: null, progress: null, cancelling: false };
    try {
      const task = await deleteBackupSnapshot(job.config.id, snapshot.key, onEvent);
      if (running !== null) {
        running.task = task;
      }
    } catch (e) {
      running = null;
      if (confirm !== null) {
        confirm.error = `The snapshot could not be deleted: ${describeError(e)}`;
      }
    }
  }

  function when(rfc3339: string): string {
    const d = new Date(rfc3339);
    return Number.isNaN(d.getTime()) ? rfc3339 : d.toLocaleString();
  }

  function whenMs(ms: number | null): string {
    if (ms === null) {
      return "";
    }
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleString();
  }

  function destination(j: JobStatus): string {
    const d = j.config.destination;
    return `${label(d.account)}, bucket ${d.bucket}${d.prefix === "" ? "" : `, folder ${d.prefix}`}`;
  }

  function schedule(j: JobStatus): string {
    return j.config.intervalHours === 0 ? "manual only" : `every ${j.config.intervalHours} h`;
  }

  onMount(() => {
    void load();
  });

  onDestroy(() => {
    destroyed = true;
  });
</script>

<section class="snapshots" aria-labelledby="snapshots-heading" data-snapshots={account.id}>
  <h2 id="snapshots-heading">Snapshots and backups</h2>
  <p class="muted small-text">Ways to get back the files of {account.label} as they were at an earlier moment.</p>

  {#if notice}
    <p class="callout {notice.tone}" role={notice.tone === "danger" ? "alert" : "status"}>{notice.text}</p>
  {/if}
  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {/if}

  <div class="card provider" data-provider={account.provider}>
    {#if account.provider === "backblaze-b2"}
      <div class="card-head">
        <h3 class="section-title">Backblaze B2 snapshots</h3>
        <span class="badge">Web console only</span>
      </div>
      <p class="small-text">
        Backblaze can make a snapshot of a bucket: a ZIP file of its files at one moment, to download or to receive on a
        drive. Snapshots are created, downloaded and deleted only in the Backblaze web console (Browse Files, then
        Snapshots). Backblaze offers no API for them, so sotyware cannot create or list them or show their progress.
      </p>
      <p class="small-text muted">
        Backblaze keeps them, and bills their storage, in a separate read-only bucket of your account whose name starts with
        <code>b2-snapshot-</code>. To browse or download one with sotyware, add that bucket as a storage account with a key
        that may read it.
      </p>
    {:else if account.provider === "hetzner"}
      <h3 class="section-title">Provider snapshots</h3>
      <p class="small-text">
        sotyware knows of no snapshot feature in Hetzner Object Storage's API. A bucket with versioning switched on keeps
        older versions of changed and deleted files; for an independent copy, use a backup below.
      </p>
    {:else}
      <h3 class="section-title">Provider snapshots</h3>
      <p class="small-text">
        sotyware does not manage snapshots of {name}. For an independent copy of this bucket, use a backup below.
      </p>
    {/if}
  </div>

  <div class="section-head">
    <div>
      <h3 class="section-title">Backups of this bucket</h3>
      <p class="muted small-text">
        A sotyware backup copies this bucket into another one, ideally at another provider. Each run writes a
        <strong>sotyware snapshot</strong>: a list of every file and where its content is kept, with unchanged content stored
        once. These are sotyware's own snapshots, not {name}'s.
      </p>
    </div>
    {#if onopenbackups && protecting.length > 0}
      <button type="button" class="ghost" onclick={onopenbackups}><Icon name="backup" size={14} />Backups page</button>
    {/if}
  </div>

  {#if list === null && loadError === null}
    <p class="muted small-text" role="status"><span class="spinner" aria-hidden="true"></span> Loading…</p>
  {:else if list !== null && protecting.length === 0}
    <div class="empty">
      <span aria-hidden="true"><Icon name="backup" size={24} /></span>
      <p><strong>No backup of this bucket yet.</strong></p>
      <p>Set up a backup job on the Backups page: choose this account as the source and another account as the destination.</p>
      {#if onopenbackups}
        <button type="button" class="primary" onclick={onopenbackups}>Set up a backup</button>
      {/if}
    </div>
  {/if}

  {#each protecting as j (j.config.id)}
    {@const snaps = snapshots[j.config.id]}
    {@const busyHere = running !== null && running.job === j.config.id}
    <article class="card job" data-job={j.config.id} aria-labelledby="job-{j.config.id}">
      <div class="card-head">
        <h4 id="job-{j.config.id}">{j.config.name}</h4>
        <span class="badge {HEALTH_TONES[j.health]}">{HEALTH_SHORT[j.health]}</span>
      </div>
      <p class="small-text">{HEALTH_TEXT[j.health]}</p>
      <dl class="meta-list">
        <dt>Destination</dt>
        <dd>{destination(j)}</dd>
        <dt>Schedule</dt>
        <dd>{schedule(j)}{j.nextDueMs !== null ? `, next ${whenMs(j.nextDueMs)}` : ""}</dd>
        {#if j.sameAccount}
          <dt>Note</dt>
          <dd class="text-danger">Same account as the source: a compromised key or a closed account affects both.</dd>
        {/if}
      </dl>

      <div class="actions">
        {#if busyHere && running}
          <span class="progress" role="status"
            ><span class="spinner" aria-hidden="true"></span>
            {running.kind === "backup" ? "Backing up" : running.kind === "verify" ? "Verifying" : "Deleting the snapshot"}{running.progress &&
            running.progress.total > 0
              ? `: ${running.progress.done.toLocaleString()} of ${running.progress.total.toLocaleString()}`
              : "…"}</span
          >
          {#if running.kind !== "delete-snapshot"}
            <button type="button" disabled={running.cancelling || running.task === null} onclick={() => void cancel()}
              >{running.cancelling ? "Cancelling…" : "Cancel"}</button
            >
          {/if}
        {:else}
          <button type="button" class="primary" disabled={running !== null || otherTask} onclick={() => void start(j, "backup")}
            >Back up now</button
          >
          <button
            type="button"
            disabled={running !== null || otherTask || j.lastSnapshot === null}
            onclick={() => void start(j, "verify")}>Verify newest</button
          >
        {/if}
      </div>
      {#if otherTask && list?.active && running === null}
        <p class="muted small-text">Another backup task is running; one runs at a time.</p>
      {/if}

      <h5 class="list-title">Snapshots</h5>
      {#if snaps === undefined || snaps.state === "loading"}
        <p class="muted small-text"><span class="spinner" aria-hidden="true"></span> Reading the snapshots at the destination…</p>
      {:else if snaps.state === "error"}
        <p class="error small-text">The snapshots could not be listed: {snaps.message}</p>
      {:else if snaps.value.length === 0}
        <p class="muted small-text">None yet. "Back up now" writes the first.</p>
      {:else}
        <ul class="snapshot-list">
          {#each snaps.value as snap, i (snap.key)}
            {@const d = details[snap.key]}
            <li data-snapshot={snap.key}>
              <div class="snapshot-row">
                <span class="when">{when(snap.created)}{#if i === 0}<span class="badge info">newest</span>{/if}</span>
                <span class="row-actions">
                  <button
                    type="button"
                    class="ghost small"
                    aria-expanded={expanded === snap.key}
                    onclick={() => void toggleDetails(j.config.id, snap)}>{expanded === snap.key ? "Hide details" : "Details"}</button
                  >
                  <button
                    type="button"
                    class="ghost small danger"
                    disabled={running !== null || otherTask}
                    onclick={() => askDelete(j, snap, snaps.value)}>Delete…</button
                  >
                </span>
              </div>
              {#if expanded === snap.key}
                {#if d === undefined || d.state === "loading"}
                  <p class="muted small-text"><span class="spinner" aria-hidden="true"></span> Reading the snapshot…</p>
                {:else if d.state === "error"}
                  <p class="error small-text">{d.message}</p>
                {:else}
                  <dl class="meta-list details">
                    <dt>Files</dt>
                    <dd>{d.value.stats.objects.toLocaleString()}, {formatDecimalSize(d.value.stats.bytes)}</dd>
                    <dt>Since the previous</dt>
                    <dd>
                      {d.value.stats.new.toLocaleString()} new, {d.value.stats.changed.toLocaleString()} changed, {d.value.stats.deleted.toLocaleString()}
                      gone; {d.value.stats.transferred.toLocaleString()} transferred ({formatDecimalSize(d.value.stats.transferredBytes)})
                    </dd>
                    <dt>Source</dt>
                    <dd>bucket {d.value.sourceBucket}{d.value.sourcePrefix === "" ? "" : `, folder ${d.value.sourcePrefix}`}</dd>
                    <dt>Verified</dt>
                    <dd>
                      {#if d.value.verified === null}
                        not on this computer
                      {:else if d.value.verified}
                        <span class="text-success">passed</span> {whenMs(d.value.verifiedMs)}{d.value.verifiedFull ? ", every byte read" : ", sizes and checksums"}
                      {:else}
                        <span class="text-danger">failed</span> {whenMs(d.value.verifiedMs)}
                      {/if}
                    </dd>
                    {#if d.value.suspicious}
                      <dt>Note</dt>
                      <dd class="text-danger">Many files changed or disappeared since the previous snapshot.</dd>
                    {/if}
                  </dl>
                {/if}
              {/if}
            </li>
          {/each}
        </ul>
        <p class="muted small-text">Restore a snapshot, or save its recovery file, on the Backups page.</p>
      {/if}
    </article>
  {/each}

  {#if holding.length > 0}
    <div class="card">
      <h3 class="section-title">Backups kept in this bucket</h3>
      <ul class="holding small-text">
        {#each holding as j (j.config.id)}
          <li>
            “{j.config.name}”, a backup of {label(j.config.source.account)}, keeps its snapshots here{j.config.destination.prefix ===
            ""
              ? ""
              : ` under ${j.config.destination.prefix}`}. Changing or deleting those objects in Files damages the backup.
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <p class="muted small-text">
    {#if account.provider === "backblaze-b2"}
      Backblaze also keeps every older version of a changed or deleted file until a lifecycle rule removes it: see Files,
      then Deleted items.
    {:else}
      If this bucket keeps versions, older versions of deleted files are under Files, then Deleted items.
    {/if}
  </p>
</section>

{#if confirm}
  {@const c = confirm}
  <Modal title="Delete this snapshot?" locked={running?.kind === "delete-snapshot"} onclose={() => (confirm = null)}>
    <form
      id="delete-snapshot-form"
      onsubmit={(e) => {
        e.preventDefault();
        void confirmDelete();
      }}
    >
      <p>
        The snapshot of <strong>{when(c.snapshot.created)}</strong> of the backup “{c.job.config.name}” is deleted at
        {destination(c.job)}.
      </p>
      <ul class="small-text consequences">
        <li>Its list of files is deleted at once. This cannot be undone in sotyware.</li>
        <li>
          Content that no other snapshot uses is deleted too, once it is more than a day older than the oldest snapshot left;
          newer content stays until a later deletion or retention.
        </li>
        <li>The files in {account.label} are not touched.</li>
        <li>If the destination keeps versions, its provider keeps the deleted data until a lifecycle rule removes it.</li>
        {#if c.only}
          <li class="text-danger">It is the only snapshot of this backup: nothing will be left to restore from.</li>
        {:else if c.newest}
          <li class="text-danger">It is the newest snapshot: the backup's health will rest on the previous one.</li>
        {/if}
      </ul>
      <label class="field">
        <span>Type <strong>delete</strong> to confirm</span>
        <input
          type="text"
          autocomplete="off"
          spellcheck="false"
          bind:value={c.typed}
          disabled={running?.kind === "delete-snapshot"}
        />
      </label>
      {#if c.error}
        <p class="error" role="alert">{c.error}</p>
      {/if}
    </form>
    {#snippet footer()}
      <button type="button" disabled={running?.kind === "delete-snapshot"} onclick={() => (confirm = null)}>Cancel</button>
      <button
        type="submit"
        form="delete-snapshot-form"
        class="primary danger"
        disabled={c.typed.trim().toLowerCase() !== "delete" || running?.kind === "delete-snapshot"}
        >{running?.kind === "delete-snapshot" ? "Deleting…" : "Delete snapshot"}</button
      >
    {/snippet}
  </Modal>
{/if}

<style>
  .snapshots {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: var(--content-w);
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .card-head .section-title,
  .card-head h4 {
    margin: 0;
  }

  .card-head h4 {
    font-size: var(--text-md);
  }

  .section-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }

  .job {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .job .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .progress {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .list-title {
    margin: var(--space-2) 0 0;
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
  }

  .snapshot-list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .snapshot-list li + li {
    border-top: 1px solid var(--border);
  }

  .snapshot-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2) var(--space-1) var(--space-3);
    font-size: var(--text-sm);
  }

  .when {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-variant-numeric: tabular-nums;
  }

  .row-actions {
    display: inline-flex;
    gap: var(--space-1);
  }

  .details {
    padding: 0 var(--space-3) var(--space-3);
  }

  .holding {
    margin: 0;
    padding-left: var(--space-5);
  }

  .consequences {
    padding-left: var(--space-5);
    margin: var(--space-2) 0 var(--space-3);
  }

  .consequences li + li {
    margin-top: var(--space-1);
  }
</style>
