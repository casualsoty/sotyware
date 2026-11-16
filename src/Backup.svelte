<script lang="ts">
  import { onMount } from "svelte";
  import {
    HEALTH_SHORT,
    HEALTH_TEXT,
    HEALTH_TONES,
    cancelBackupTask,
    checkBackupProtection,
    exportBackupManifest,
    listBackupSnapshots,
    listBackups,
    removeBackupJob,
    restoreBackup,
    saveBackupJob,
    startBackupTask,
    type BackupEvent,
    type JobInput,
    type JobStatus,
    type Progress,
    type Protection,
    type SnapshotRef,
    type TaskKind,
    type TaskSummary,
  } from "./backup";
  import { listAccounts, type AccountInfo } from "./cloud";
  import { describeOperationError } from "./cloudMessages";
  import { formatSize } from "./format";
  import Icon from "./Icon.svelte";

  // Backups (ADR-031): jobs that keep an independent, verified copy of an
  // account's bucket in a second destination. Everything remote happens in
  // Rust; the page never sends or receives a local path (restoring into a
  // folder and saving a recovery file use dialogs Rust opens).

  const INTERVALS: { hours: number; label: string }[] = [
    { hours: 0, label: "Manual only" },
    { hours: 6, label: "Every 6 hours" },
    { hours: 12, label: "Every 12 hours" },
    { hours: 24, label: "Daily" },
    { hours: 168, label: "Weekly" },
  ];

  let jobs = $state<JobStatus[]>([]);
  let accounts = $state<AccountInfo[]>([]);
  let storageError = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let active = $state<{ task: string; job: string; kind: TaskKind; progress: Progress | null; cancelling: boolean } | null>(null);
  let scheduled = $state<{ job: string; kind: TaskKind } | null>(null);
  let result = $state<TaskSummary | null>(null);
  let actionError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let form = $state<JobInput | null>(null);
  let saving = $state(false);
  let confirmRemove = $state<string | null>(null);
  let snapshots = $state<{ job: string; list: SnapshotRef[] } | null>(null);
  let protection = $state<{ job: string; value: Protection } | null>(null);
  let restoreFilter = $state("");
  let restoreAccount = $state("");
  let restorePrefix = $state("restored/");

  async function load(): Promise<void> {
    try {
      const [list, acc] = await Promise.all([listBackups(), listAccounts()]);
      jobs = list.jobs;
      storageError = list.storageError;
      accounts = acc.accounts.filter((a) => a.bucket !== null);
      scheduled = list.active && list.active.task !== active?.task ? { job: list.active.job, kind: list.active.kind } : null;
      loadError = null;
    } catch (e) {
      loadError = `Could not load the backups: ${describeOperationError(e)}`;
    }
  }

  function label(account: string): string {
    return accounts.find((a) => a.id === account)?.label ?? account;
  }

  function when(ms: number | null | undefined): string {
    if (ms === null || ms === undefined) {
      return "—";
    }
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? "—" : d.toLocaleString();
  }

  function newJob(): void {
    const first = accounts[0]?.id ?? "";
    const second = accounts.find((a) => a.id !== first)?.id ?? first;
    form = {
      id: null,
      name: "",
      sourceAccount: first,
      sourcePrefix: "",
      destinationAccount: second,
      destinationPrefix: "sotyware-backup/",
      intervalHours: 24,
      retention: { keepLast: 7, keepDaily: 14, keepWeekly: 8 },
      suspiciousPercent: 30,
    };
    actionError = null;
  }

  function editJob(j: JobStatus): void {
    form = {
      id: j.config.id,
      name: j.config.name,
      sourceAccount: j.config.source.account,
      sourcePrefix: j.config.source.prefix,
      destinationAccount: j.config.destination.account,
      destinationPrefix: j.config.destination.prefix,
      intervalHours: j.config.intervalHours,
      retention: { ...j.config.retention },
      suspiciousPercent: j.config.suspiciousPercent,
    };
    actionError = null;
  }

  async function save(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!form) {
      return;
    }
    saving = true;
    actionError = null;
    try {
      const saved = await saveBackupJob(form);
      notice = `Saved backup job "${saved.name}".`;
      form = null;
      await load();
    } catch (e) {
      actionError = describeOperationError(e);
    } finally {
      saving = false;
    }
  }

  async function remove(id: string): Promise<void> {
    actionError = null;
    try {
      await removeBackupJob(id);
      confirmRemove = null;
      notice = "The job was removed from this computer. Its backup data at the destination was not touched.";
      await load();
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  function onEvent(event: BackupEvent): void {
    if (!active || event.task !== active.task) {
      return;
    }
    if (event.type === "progress") {
      active.progress = event.progress;
    } else {
      result = event.summary;
      active = null;
      void load();
    }
  }

  async function start(job: string, kind: Exclude<TaskKind, "restore">): Promise<void> {
    actionError = null;
    notice = null;
    result = null;
    try {
      const task = await startBackupTask(job, kind, onEvent);
      active = { task, job, kind, progress: null, cancelling: false };
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function cancel(): Promise<void> {
    if (!active) {
      return;
    }
    active.cancelling = true;
    try {
      await cancelBackupTask(active.task);
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function showSnapshots(job: string): Promise<void> {
    actionError = null;
    try {
      snapshots = { job, list: (await listBackupSnapshots(job)).reverse() };
      restoreAccount = jobs.find((j) => j.config.id === job)?.config.source.account ?? "";
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function restoreLocal(job: string, snapshot: string): Promise<void> {
    actionError = null;
    result = null;
    try {
      const task = await restoreBackup(job, snapshot, restoreFilter, { kind: "local" }, onEvent);
      if (task !== null) {
        active = { task, job, kind: "restore", progress: null, cancelling: false };
      }
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function restoreBucket(job: string, snapshot: string): Promise<void> {
    actionError = null;
    result = null;
    try {
      const task = await restoreBackup(job, snapshot, restoreFilter, { kind: "bucket", account: restoreAccount, prefix: restorePrefix }, onEvent);
      if (task !== null) {
        active = { task, job, kind: "restore", progress: null, cancelling: false };
      }
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function exportFile(job: string, snapshot: string): Promise<void> {
    actionError = null;
    try {
      const name = await exportBackupManifest(job, snapshot);
      if (name !== null) {
        notice = `Saved the recovery file ${name}. Keep it with your recovery notes; it contains no key.`;
      }
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  async function showProtection(job: string): Promise<void> {
    actionError = null;
    try {
      protection = { job, value: await checkBackupProtection(job) };
    } catch (e) {
      actionError = describeOperationError(e);
    }
  }

  function phaseText(p: Progress | null): string {
    if (!p) {
      return "Starting…";
    }
    const counts = p.total > 0 ? ` ${p.done} of ${p.total}` : "";
    const names: Record<string, string> = {
      listing: "Listing the source…",
      transferring: `Backing up${counts}`,
      writing: "Writing the snapshot…",
      verifying: `Verifying${counts}`,
      pruning: "Applying retention…",
      deleting: "Deleting the snapshot…",
      restoring: `Restoring${counts}`,
    };
    return names[p.phase] ?? p.phase;
  }

  function resultText(r: TaskSummary): string {
    const parts: string[] = [];
    const what = {
      backup: "Backup",
      verify: "Verification",
      "verify-full": "Full verification",
      prune: "Retention",
      restore: "Restore",
      "delete-snapshot": "Snapshot deletion",
    }[r.kind];
    parts.push(`${what}: ${r.state}.`);
    if (r.stats) {
      parts.push(
        `${r.stats.objects} file(s), ${formatSize(r.stats.bytes)}; ${r.stats.new} new, ${r.stats.changed} changed, ${r.stats.deleted} gone; ${r.stats.transferred} transferred (${formatSize(r.stats.transferredBytes)}).`,
      );
    }
    if (r.suspicious) {
      parts.push("Many files changed or disappeared: older snapshots are kept until you review this.");
    }
    if (r.verification) {
      parts.push(
        r.verification.passed
          ? `Verified ${r.verification.blobs} stored object(s)${r.verification.deep ? " by reading their content" : ""}.`
          : `Verification failed: ${r.verification.missing.length} missing, ${r.verification.damaged.length} damaged.`,
      );
    }
    if (r.prune) {
      parts.push(`Retention removed ${r.prune.snapshotsDeleted} snapshot(s) and ${r.prune.blobsDeleted} unreferenced object(s) (${formatSize(r.prune.bytesDeleted)}); kept ${r.prune.snapshotsKept}.${r.prune.refused > 0 ? ` ${r.prune.refused} delete(s) were refused by the provider.` : ""}`);
    }
    if (r.pruneSkipped) {
      parts.push(`Retention did not run: ${r.pruneSkipped}.`);
    }
    if (r.restore) {
      parts.push(
        `Restored and verified ${r.restore.restored} file(s) (${formatSize(r.restore.bytes)}); ${r.restore.skipped.length} left alone because something was already there; ${r.restore.renamed.length} saved under another name; ${r.restore.failed.length} failed.`,
      );
    }
    if (r.error) {
      parts.push(`Error: ${r.error.message}`);
    }
    return parts.join(" ");
  }

  onMount(() => {
    void load();
  });
</script>

<div class="page">
<section aria-labelledby="backup-heading">
  <header class="page-header">
    <div class="titles">
      <h1 id="backup-heading">Backups</h1>
      <p class="lede">
        A backup keeps an independent copy of a bucket (or a folder of it) in a second place, ideally another account at
        another provider. Every run writes a snapshot you can restore from, also without sotyware: the snapshots are plain
        JSON files next to the backed-up content.
      </p>
    </div>
    {#if !form && accounts.length > 0}
      <button type="button" class="primary" onclick={newJob}><Icon name="plus" size={15} />New backup job</button>
    {/if}
  </header>

  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {/if}
  {#if storageError}
    <p class="error" role="alert">The backup database cannot be used: {storageError}</p>
  {/if}
  {#if actionError}
    <p class="error" role="alert">{actionError}</p>
  {/if}
  {#if notice}
    <p class="notice" role="status">{notice}</p>
  {/if}

  {#if scheduled}
    <p class="status" role="status">A scheduled {scheduled.kind} of "{scheduled.job}" is running. <button type="button" onclick={() => void load()}>Refresh</button></p>
  {/if}

  {#if active}
    <div class="task panel" data-task={active.task}>
      <p role="status">{phaseText(active.progress)}</p>
      {#if active.progress && active.progress.total > 0}
        <progress max={active.progress.total} value={active.progress.done}></progress>
      {/if}
      <button type="button" disabled={active.cancelling} onclick={() => void cancel()}>{active.cancelling ? "Cancelling…" : "Cancel"}</button>
    </div>
  {/if}

  {#if result}
    <div class="result panel" data-state={result.state} data-kind={result.kind}>
      <p>{resultText(result)}</p>
      {#if result.objectErrors.length > 0}
        <details>
          <summary>{result.objectErrors.length} file(s) with problems</summary>
          <ul>
            {#each result.objectErrors.slice(0, 50) as o (o.key)}
              <li><span class="key">{o.key}</span>: {o.message}</li>
            {/each}
          </ul>
        </details>
      {/if}
      <button type="button" onclick={() => (result = null)}>Dismiss</button>
    </div>
  {/if}

  {#if form}
    <form class="job-form card" onsubmit={save} aria-label={form.id ? "Edit backup job" : "New backup job"}>
      <h3>{form.id ? "Edit backup job" : "New backup job"}</h3>
      <fieldset disabled={saving}>
        <label for="bk-name">Name</label>
        <input id="bk-name" type="text" bind:value={form.name} required placeholder="Photos" />
        <label for="bk-source">Back up from</label>
        <select id="bk-source" value={form.sourceAccount} onchange={(e) => form && (form.sourceAccount = e.currentTarget.value)}>
          {#each accounts as a (a.id)}<option value={a.id}>{a.label} ({a.bucket})</option>{/each}
        </select>
        <label for="bk-source-prefix">Folder to back up (empty for the whole bucket)</label>
        <input id="bk-source-prefix" type="text" bind:value={form.sourcePrefix} placeholder="photos/" spellcheck="false" />
        <label for="bk-dest">Back up to</label>
        <select id="bk-dest" value={form.destinationAccount} onchange={(e) => form && (form.destinationAccount = e.currentTarget.value)}>
          {#each accounts as a (a.id)}<option value={a.id}>{a.label} ({a.bucket})</option>{/each}
        </select>
        <label for="bk-dest-prefix">Folder for the backup</label>
        <input id="bk-dest-prefix" type="text" bind:value={form.destinationPrefix} spellcheck="false" />
        {#if form.sourceAccount === form.destinationAccount}
          <p class="warning callout" role="note">
            Source and destination are the same account: this protects against deleting files by mistake, but not against
            a compromised key, a closed account or a provider outage. Prefer an account at another provider.
          </p>
        {/if}
        <label for="bk-interval">Schedule</label>
        <select id="bk-interval" value={String(form.intervalHours)} onchange={(e) => form && (form.intervalHours = Number(e.currentTarget.value))}>
          {#each INTERVALS as i (i.hours)}<option value={String(i.hours)}>{i.label}</option>{/each}
        </select>
        <fieldset class="retention">
          <legend>Keep snapshots (0 everywhere keeps all)</legend>
          <label for="bk-last">Last</label>
          <input id="bk-last" type="number" min="0" max="10000" bind:value={form.retention.keepLast} />
          <label for="bk-daily">Daily</label>
          <input id="bk-daily" type="number" min="0" max="10000" bind:value={form.retention.keepDaily} />
          <label for="bk-weekly">Weekly</label>
          <input id="bk-weekly" type="number" min="0" max="10000" bind:value={form.retention.keepWeekly} />
        </fieldset>
        <label for="bk-threshold">Warn and keep older snapshots when more than this share of files changed or disappeared (%)</label>
        <input id="bk-threshold" type="number" min="1" max="100" bind:value={form.suspiciousPercent} />
        <div class="actions">
          <button type="submit" class="primary">{saving ? "Saving…" : "Save"}</button>
          <button type="button" onclick={() => (form = null)}>Cancel</button>
        </div>
      </fieldset>
    </form>
  {:else if accounts.length === 0}
    <p class="hint">Add a storage account in Cloud Storage first; a backup needs a source and a destination account.</p>
  {/if}

  {#if jobs.length === 0 && !form}
    <div class="empty">
      <span class="empty-icon" aria-hidden="true"><Icon name="backup" size={28} /></span>
      <p><strong>No backup job yet.</strong></p>
      <p>A job copies a bucket into a second, independent destination on a schedule and verifies every snapshot.</p>
    </div>
  {/if}

  <ul class="jobs">
    {#each jobs as j (j.config.id)}
      <li data-job={j.config.id} data-health={j.health}>
        <div class="job-head">
          <h3>{j.config.name}</h3>
          <span class="badge {HEALTH_TONES[j.health]}">{HEALTH_SHORT[j.health]}</span>
        </div>
        <p class="route">
          {label(j.config.source.account)} / {j.config.source.bucket}/{j.config.source.prefix} → {label(j.config.destination.account)} /
          {j.config.destination.bucket}/{j.config.destination.prefix}
        </p>
        <p class="health" data-health={j.health}>{HEALTH_TEXT[j.health]}</p>
        {#if j.sameAccount}
          <p class="warning">Same account as the source: not an independent copy.</p>
        {/if}
        <dl>
          <dt>Last backup</dt>
          <dd>{j.lastBackup ? `${when(j.lastBackup.startedMs)} (${j.lastBackup.state})` : "never"}</dd>
          <dt>Last verified</dt>
          <dd>{j.lastVerification ? `${when(j.lastVerification.finishedMs)} (${j.lastVerification.passed ? "passed" : "failed"})` : "never"}</dd>
          <dt>Next run</dt>
          <dd>{j.config.intervalHours === 0 ? "manual" : when(j.nextDueMs)}</dd>
        </dl>
        <div class="actions">
          <button type="button" disabled={active !== null} onclick={() => void start(j.config.id, "backup")}>Back up now</button>
          <button type="button" disabled={active !== null} onclick={() => void start(j.config.id, "verify")}>Verify</button>
          <button type="button" disabled={active !== null} onclick={() => void start(j.config.id, "verify-full")}>Verify content</button>
          <button type="button" disabled={active !== null} onclick={() => void start(j.config.id, "prune")}>Apply retention</button>
          <button type="button" onclick={() => void showSnapshots(j.config.id)}>Snapshots…</button>
          <button type="button" onclick={() => void showProtection(j.config.id)}>Protection…</button>
          <button type="button" onclick={() => editJob(j)}>Edit</button>
          {#if confirmRemove === j.config.id}
            <span class="confirm"
              >Remove this job from sotyware? Its backup data at the destination stays.
              <button type="button" class="danger" onclick={() => void remove(j.config.id)}>Remove job</button>
              <button type="button" onclick={() => (confirmRemove = null)}>Keep</button></span
            >
          {:else}
            <button type="button" onclick={() => (confirmRemove = j.config.id)}>Remove…</button>
          {/if}
        </div>
        {#if protection?.job === j.config.id}
          <p class="protection" data-immutable={protection.value.immutable}>{protection.value.summary}</p>
        {/if}
        {#if snapshots?.job === j.config.id}
          <div class="snapshots">
            {#if snapshots.list.length === 0}
              <p class="hint">No snapshot at the destination yet.</p>
            {:else}
              <label for="bk-filter-{j.config.id}">Restore only keys starting with (empty for all)</label>
              <input id="bk-filter-{j.config.id}" type="text" bind:value={restoreFilter} spellcheck="false" />
              <label for="bk-target-{j.config.id}">Restore into account</label>
              <select id="bk-target-{j.config.id}" value={restoreAccount} onchange={(e) => (restoreAccount = e.currentTarget.value)}>
                {#each accounts as a (a.id)}<option value={a.id}>{a.label} ({a.bucket})</option>{/each}
              </select>
              <label for="bk-target-prefix-{j.config.id}">into folder</label>
              <input id="bk-target-prefix-{j.config.id}" type="text" bind:value={restorePrefix} spellcheck="false" />
              <ul>
                {#each snapshots.list as s (s.key)}
                  <li data-snapshot={s.key}>
                    {new Date(s.created).toLocaleString()}
                    <button type="button" disabled={active !== null} onclick={() => void restoreLocal(j.config.id, s.key)}>Restore to a folder…</button>
                    <button type="button" disabled={active !== null || restoreAccount === ""} onclick={() => void restoreBucket(j.config.id, s.key)}
                      >Restore to the bucket</button
                    >
                    <button type="button" onclick={() => void exportFile(j.config.id, s.key)}>Save recovery file…</button>
                  </li>
                {/each}
              </ul>
              <p class="hint">Restores never replace existing files or objects: a name that is taken gets " (n)", an object that exists is left alone.</p>
            {/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</section>
</div>

<style>
  .jobs {
    list-style: none;
    padding: 0;
    margin: var(--space-4) 0 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .jobs > li {
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    font-size: var(--text-sm);
  }

  .job-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin-bottom: var(--space-1);
  }

  .jobs h3 {
    margin: 0;
    font-size: var(--text-md);
  }

  .route,
  .hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
    word-break: break-all;
  }

  .health {
    margin: var(--space-2) 0;
  }

  .health[data-health="healthy"] {
    color: var(--success);
  }

  .health:not([data-health="healthy"]) {
    color: var(--warning);
  }

  .health[data-health="failed"],
  .health[data-health="verification-failed"] {
    color: var(--danger);
  }

  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--space-1) var(--space-4);
    font-size: var(--text-sm);
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
  }

  .actions {
    margin-top: var(--space-3);
  }

  .job-form {
    margin: var(--space-4) 0;
  }

  .job-form h3 {
    margin: 0 0 var(--space-3);
    font-size: var(--text-lg);
  }

  .job-form label,
  .snapshots label {
    display: block;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    margin-top: var(--space-3);
    margin-bottom: var(--space-1);
  }

  .job-form input,
  .job-form select,
  .snapshots input,
  .snapshots select {
    display: block;
    width: 100%;
  }

  .job-form .retention {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0 var(--space-3);
  }

  .job-form .retention legend {
    margin-top: var(--space-3);
    font-weight: var(--weight-medium);
  }

  .snapshots {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }

  .panel {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-sm);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .panel progress {
    margin: var(--space-2) 0;
  }

  .result[data-state="failed"],
  .result[data-state="partial"] {
    border-color: var(--danger-border);
  }

  .warning:not(.callout) {
    color: var(--warning);
  }

  .key {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .empty-icon {
    display: inline-flex;
    margin-bottom: var(--space-2);
    color: var(--text-subtle);
  }

  .protection {
    color: var(--text-muted);
  }
</style>
