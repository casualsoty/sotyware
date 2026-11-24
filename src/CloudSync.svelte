<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type { AccountInfo } from "./cloud";
  import { describeError } from "./cloudMessages";
  import {
    DEFAULT_INTERVAL,
    INTERVALS,
    OP_LABELS,
    answerFolderSyncDeletions,
    changeSyncFolder,
    chooseSyncFolder,
    disableFolderSync,
    discardSyncFolderChoice,
    enableFolderSync,
    folderSyncDetails,
    openSyncFolder,
    resolveFolderSyncConflict,
    retryFolderSync,
    setFolderSyncInterval,
    setFolderSyncPaused,
    stateLabel,
    syncFolderNow,
    type ConflictChoice,
    type FolderChoice,
    type MoveMode,
    type SyncDetails,
    type SyncStatus,
  } from "./folderSync";
  import { FolderSyncModel } from "./folderSyncModel.svelte";
  import { formatAgo, formatCount, formatDateTime, formatDecimalSize } from "./format";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // The bucket view's "Sync" tab (ADR-043, docs/architecture/folder-sync.md):
  // keeps a folder on this computer and this account's bucket in step, in
  // both directions, in the background. The folder is chosen in the native
  // dialog Rust opens; the page shows the chosen path and never sends one.
  // Everything else (watching, listing, transfers, conflicts, the deletion
  // guard) is Rust's; this page shows the live status and sends the user's
  // decisions.

  interface Props {
    account: AccountInfo;
    /** The shell's live sync state; without one the tab keeps its own. */
    model?: FolderSyncModel | undefined;
    /** Opens this account's edit form (a key the provider refuses). */
    onedit?: (() => void) | undefined;
  }

  let { account, model, onedit }: Props = $props();

  // svelte-ignore state_referenced_locally
  const ownModel = model === undefined;
  // svelte-ignore state_referenced_locally
  const live: FolderSyncModel = model ?? new FolderSyncModel();

  const status = $derived<SyncStatus | null>(live.of(account.id));
  const running = $derived(live.overview?.running ?? true);

  let choice = $state<FolderChoice | null>(null);
  let choosing = $state(false);
  let interval = $state<number>(DEFAULT_INTERVAL);
  let moveMode = $state<MoveMode>("move");
  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let details = $state<SyncDetails | null>(null);
  let detailsTab = $state<"activity" | "problems" | "skipped">("activity");
  let confirmOff = $state(false);
  let destroyed = false;
  let detailsKey = "";
  let detailsTimer: ReturnType<typeof setTimeout> | undefined;

  const label = $derived(status === null ? null : stateLabel(status));

  async function act(name: string, f: () => Promise<void>): Promise<void> {
    busy = name;
    error = null;
    notice = null;
    try {
      await f();
    } catch (e) {
      error = describeError(e);
    } finally {
      busy = null;
      void live.refresh();
    }
  }

  async function choose(): Promise<void> {
    choosing = true;
    error = null;
    notice = null;
    try {
      if (choice !== null) {
        void discardSyncFolderChoice(choice.token).catch(() => undefined);
        choice = null;
      }
      const c = await chooseSyncFolder(account.id, status?.id ?? null);
      if (destroyed) {
        if (c !== null) {
          void discardSyncFolderChoice(c.token).catch(() => undefined);
        }
        return;
      }
      choice = c;
      if (c !== null) {
        moveMode = c.canMove ? "move" : "fresh";
      }
    } catch (e) {
      error = describeError(e);
    } finally {
      choosing = false;
    }
  }

  function cancelChoice(): void {
    if (choice !== null) {
      void discardSyncFolderChoice(choice.token).catch(() => undefined);
    }
    choice = null;
  }

  async function confirmChoice(): Promise<void> {
    const c = choice;
    if (c === null) {
      return;
    }
    await act("confirm", async () => {
      if (c.sync === null) {
        await enableFolderSync(c.token, interval);
        notice = `Sync started. Files in ${c.path} and the bucket ${account.bucket ?? ""} are being compared.`;
      } else {
        await changeSyncFolder(c.token, moveMode);
        notice =
          moveMode === "move"
            ? `The files were moved to ${c.path}; sync goes on there.`
            : `Sync goes on in ${c.path}. The previous folder was left as it is.`;
      }
      choice = null;
    });
  }

  async function turnOff(): Promise<void> {
    const s = status;
    if (s === null) {
      return;
    }
    await act("off", async () => {
      const report = await disableFolderSync(s.id);
      confirmOff = false;
      notice =
        `Sync is off. The files stay in ${report.path}; the bucket was not changed.` +
        (report.trashKept ? " Files sync had removed are still in the hidden folder .sotyware-sync/trash there." : "");
    });
  }

  async function resolve(path: string, choiceOf: ConflictChoice): Promise<void> {
    const s = status;
    if (s === null) {
      return;
    }
    await act(`conflict:${path}`, () => resolveFolderSyncConflict(s.id, path, choiceOf));
    await loadDetails(true);
  }

  async function loadDetails(force = false): Promise<void> {
    const s = status;
    if (s === null) {
      details = null;
      return;
    }
    const key = `${s.id}:${s.activitySeq}:${s.failed}:${s.conflicts}:${s.skipped}:${s.heldDeletions?.askedMs ?? 0}`;
    if (!force && key === detailsKey) {
      return;
    }
    detailsKey = key;
    try {
      const d = await folderSyncDetails(s.id);
      if (!destroyed) {
        details = d;
      }
    } catch (e) {
      if (!destroyed) {
        error = describeError(e);
      }
    }
  }

  $effect(() => {
    // Reload the lists when what they show changed (debounced: the status
    // changes several times a second while files transfer).
    const s = status;
    void s?.activitySeq;
    void s?.failed;
    void s?.conflicts;
    void s?.skipped;
    void s?.heldDeletions;
    clearTimeout(detailsTimer);
    detailsTimer = setTimeout(() => void loadDetails(), details === null ? 0 : 400);
  });

  function percent(done: number, total: number): number {
    return total <= 0 ? 100 : Math.min(100, Math.floor((done / total) * 100));
  }

  function when(ms: number): string {
    return formatDateTime(new Date(ms));
  }

  function minutesUntil(ms: number | null): string | null {
    if (ms === null) {
      return null;
    }
    const m = Math.round((ms - Date.now()) / 60_000);
    return m <= 0 ? "soon" : `in ${m} minute${m === 1 ? "" : "s"}`;
  }

  function versioningText(v: string | null): string {
    switch (v) {
      case "enabled":
        return "Kept: a file deleted or replaced through sync stays in the bucket as an earlier version.";
      case "suspended":
      case "disabled":
        return "Not kept: deleting or changing a file in the folder deletes or replaces it in the bucket for good.";
      default:
        return "Unknown (the bucket's versioning could not be read).";
    }
  }

  const needsAccount = $derived(
    status?.error !== null &&
      status?.error !== undefined &&
      ["invalid-credentials", "credentials-missing", "credential-store-locked", "credential-store-unavailable", "access-denied"].includes(
        status.error.kind,
      ),
  );

  onMount(() => {
    if (ownModel) {
      void live.start();
    }
  });

  onDestroy(() => {
    destroyed = true;
    clearTimeout(detailsTimer);
    if (choice !== null) {
      void discardSyncFolderChoice(choice.token).catch(() => undefined);
    }
    if (ownModel) {
      live.stop();
    }
  });
</script>

<section class="sync" aria-labelledby="sync-heading" data-sync-tab={account.id}>
  <header class="sync-head">
    <div>
      <h2 id="sync-heading">Sync with this computer</h2>
      <p class="muted small-text">
        {#if status}
          Files in the folder below and in the bucket {status.bucket} are kept in step, in both directions, while sotyware runs.
        {:else}
          Work with this bucket's files in your file manager, as ordinary files on this computer.
        {/if}
      </p>
    </div>
    {#if status && choice === null}
      <div class="actions">
        <button type="button" disabled={busy !== null || status.paused || !running} onclick={() => void act("now", () => syncFolderNow(status.id))}
          ><Icon name="refresh" size={14} />Sync now</button
        >
        {#if status.paused}
          <button type="button" class="primary" disabled={busy !== null} onclick={() => void act("resume", () => setFolderSyncPaused(status.id, false))}
            >Resume sync</button
          >
        {:else}
          <button type="button" disabled={busy !== null || status.state === "moving"} onclick={() => void act("pause", () => setFolderSyncPaused(status.id, true))}
            >Pause sync</button
          >
        {/if}
        <button type="button" disabled={busy !== null} onclick={() => void act("open", () => openSyncFolder(status.id, "folder"))}
          ><Icon name="external" size={14} />Open folder</button
        >
      </div>
    {/if}
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if notice}
    <p class="notice" role="status">{notice}</p>
  {/if}
  {#if live.error}
    <p class="error" role="alert">The sync status cannot be read: {live.error}</p>
  {/if}
  {#if live.overview?.storageError}
    <p class="notice error">Folder sync cannot keep its state on this computer: {live.overview.storageError}</p>
  {/if}
  {#if live.overview && !running}
    <p class="notice warning">
      Folder sync is switched off in Settings › Features. Syncs stay set up and go on when it is switched on again.
    </p>
  {/if}

  {#if choice}
    {@render confirm(choice)}
  {:else if status === null}
    {#if live.overview === null && live.error === null}
      <p class="muted" role="status">Loading…</p>
    {:else}
      <div class="empty intro">
        <span class="intro-icon" aria-hidden="true"><Icon name="sync" size={22} /></span>
        <p><strong>Sync {account.bucket ?? "this bucket"} with a folder on this computer</strong></p>
        <p>
          Choose any folder, on any drive. Its files appear in the bucket and the bucket's files appear in it, as ordinary files
          you open, copy, rename and delete in your file manager. Changes go both ways while sotyware runs; a file changed on
          both sides is kept twice, never overwritten.
        </p>
        <button type="button" class="primary" disabled={choosing} onclick={() => void choose()}
          ><Icon name="folder" size={14} />{choosing ? "Choosing…" : "Choose folder…"}</button
        >
      </div>
    {/if}
  {:else}
    <div class="card status-card" data-sync-state={status.state}>
      <div class="status-line">
        <span class="badge {label?.tone}" data-sync-badge>{label?.label}</span>
        <p class="status-text">
          {#if status.message}
            {status.message}
          {:else if status.state === "synced"}
            Everything is in step.
          {:else if status.state === "paused"}
            Paused. Changes on either side wait until sync is resumed.
          {:else if status.state === "stopped"}
            Not running.
          {:else if status.state === "scanning" || status.state === "starting"}
            Looking for changes…
          {:else if status.transfers.length > 0 || status.pendingUploads + status.pendingDownloads > 0}
            Passing changes on: {formatCount(status.pendingUploads + status.pendingDownloads)} file{status.pendingUploads +
              status.pendingDownloads ===
            1
              ? ""
              : "s"} to transfer.
          {:else}
            Applying changes…
          {/if}
        </p>
      </div>
      {#if needsAccount && onedit}
        <p class="status-hint">
          <button type="button" class="small" onclick={() => onedit?.()}><Icon name="edit" size={13} />Edit account</button>
        </p>
      {/if}
      {#if status.state === "error" || status.state === "unavailable"}
        <p class="status-hint small-text muted">
          Nothing was changed while this lasts. Choose the folder again (or another one) to sync from scratch, or turn sync off.
          <button type="button" class="small" disabled={choosing} onclick={() => void choose()}>Choose folder…</button>
        </p>
      {/if}
      {#if status.migration}
        {@const m = status.migration}
        <div class="migration" role="status">
          <p class="small-text">
            {m.copying ? "Copying and checking" : "Moving"} files to {m.to}: {formatCount(m.filesDone)} of {formatCount(m.filesTotal)}
            ({formatDecimalSize(m.bytesDone)} of {formatDecimalSize(m.bytesTotal)})
          </p>
          <progress max={Math.max(1, m.bytesTotal)} value={m.bytesDone} aria-label="Moving the sync folder"></progress>
        </div>
      {/if}
    </div>

    {#if status.heldDeletions}
      {@const h = status.heldDeletions}
      <div class="callout warning held" role="alert">
        <p><strong>Many files would be deleted at once, so sync asks first.</strong></p>
        <p>
          {#if h.local > 0}{formatCount(h.local)} file{h.local === 1 ? "" : "s"} deleted in the bucket would be removed from this folder (into the sync trash).{/if}
          {#if h.remote > 0}{formatCount(h.remote)} file{h.remote === 1 ? "" : "s"} deleted in this folder would be deleted in the bucket.{/if}
          That is more than half of the {formatCount(h.synced)} synced files. If you did not mean it (a disconnected drive,
          another program, a mistake), restore them.
        </p>
        <div class="actions">
          <button type="button" class="primary" disabled={busy !== null} onclick={() => void act("restore", () => answerFolderSyncDeletions(status.id, false))}
            >Restore them</button
          >
          <button type="button" class="danger" disabled={busy !== null} onclick={() => void act("delete", () => answerFolderSyncDeletions(status.id, true))}
            >Delete them</button
          >
        </div>
      </div>
    {/if}

    {#if status.diskFull}
      <p class="notice warning">
        The drive of the sync folder is full: downloads wait. Free some space; sync tries again by itself.
      </p>
    {/if}

    <div class="card folder-card">
      <h3 class="section-title">Folder on this computer</h3>
      <div class="folder-row">
        <span class="folder-icon" aria-hidden="true"><Icon name="folder" size={18} /></span>
        <p class="mono path" data-sync-path>{status.path}</p>
        <button type="button" class="small" disabled={choosing || busy !== null || status.state === "moving"} onclick={() => void choose()}
          >{choosing ? "Choosing…" : "Change folder…"}</button
        >
      </div>
      <p class="small-text muted">
        {status.watching ? "Changes in the folder are noticed as they happen." : "Changes in the folder are found at each check."}
        The bucket is checked for changes made elsewhere every {status.intervalMinutes} minutes{status.nextCheckMs !== null &&
        status.state !== "paused"
          ? ` (next ${minutesUntil(status.nextCheckMs)})`
          : ""}.
      </p>
    </div>

    <div class="tiles">
      <div class="tile">
        <p class="tile-label">Files in step</p>
        <p class="tile-value">{formatCount(status.files)}</p>
        <p class="tile-sub">{formatCount(status.folders)} folder{status.folders === 1 ? "" : "s"}</p>
      </div>
      <div class="tile">
        <p class="tile-label">On this computer</p>
        <p class="tile-value">{formatDecimalSize(status.bytes)}</p>
        <p class="tile-sub">{status.freeBytes === null ? "" : `${formatDecimalSize(status.freeBytes)} free on the drive`}</p>
      </div>
      <div class="tile">
        <p class="tile-label">In the bucket</p>
        <p class="tile-value">{status.lastCheckedMs === null ? "—" : formatDecimalSize(status.remoteBytes)}</p>
        <p class="tile-sub">
          {status.lastCheckedMs === null ? "not listed yet" : `${formatCount(status.remoteFiles)} files, current versions`}
        </p>
      </div>
      <div class="tile">
        <p class="tile-label">Last in step</p>
        <p class="tile-value">{status.lastSyncedMs === null ? "Not yet" : formatAgo(status.lastSyncedMs)}</p>
        <p class="tile-sub">{status.lastSyncedMs === null ? "" : when(status.lastSyncedMs)}</p>
      </div>
    </div>

    <p class="pending small-text" data-sync-pending>
      {#if status.pendingUploads + status.pendingDownloads === 0}
        Nothing waits to be transferred.
      {:else}
        To upload: {formatCount(status.pendingUploads)} ({formatDecimalSize(status.pendingUploadBytes)}). To download:
        {formatCount(status.pendingDownloads)} ({formatDecimalSize(status.pendingDownloadBytes)}).
      {/if}
    </p>

    {#if status.transfers.length > 0}
      <div class="card">
        <h3 class="section-title">Transferring now</h3>
        <ul class="transfers">
          {#each status.transfers as t (t.path + t.kind + t.startedMs)}
            <li>
              <span class="dir" aria-hidden="true"><Icon name={t.kind === "upload" ? "upload" : "download"} size={14} /></span>
              <p class="t-name" title={t.path}>{t.path}</p>
              <progress max={Math.max(1, t.total)} value={t.bytes} aria-label="{t.kind === 'upload' ? 'Uploading' : 'Downloading'} {t.path}"></progress>
              <p class="t-figure">
                {percent(t.bytes, t.total)}%{t.speed === null ? "" : ` · ${formatDecimalSize(t.speed)}/s`}
              </p>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if details && details.conflicts.length > 0}
      <div class="card conflicts" aria-labelledby="conflicts-heading">
        <h3 class="section-title" id="conflicts-heading">Changed on both sides</h3>
        <p class="small-text muted">
          These files were changed here and elsewhere before sync could pass the change on. Both versions were kept: this
          computer's under the name, the other one as a conflicted copy next to it.
        </p>
        <ul class="conflict-list">
          {#each details.conflicts as c (c.path)}
            <li>
              <p class="c-name"><span class="mono">{c.path}</span> <span class="muted small-text">and</span> <span class="mono">{c.copy}</span></p>
              <div class="actions">
                <button type="button" class="small" disabled={busy !== null} onclick={() => void resolve(c.path, "keep-local")}
                  >Keep this computer's</button
                >
                <button type="button" class="small" disabled={busy !== null} onclick={() => void resolve(c.path, "keep-remote")}
                  >Keep the other one</button
                >
                <button type="button" class="small ghost" disabled={busy !== null} onclick={() => void resolve(c.path, "keep-both")}
                  >Keep both</button
                >
              </div>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="card details" aria-labelledby="activity-heading">
      <div class="details-head">
        <h3 class="section-title" id="activity-heading">Activity</h3>
        <div class="segmented-tabs" role="tablist" aria-label="Activity views">
          <button type="button" role="tab" aria-selected={detailsTab === "activity"} onclick={() => (detailsTab = "activity")}>Recent</button>
          <button type="button" role="tab" aria-selected={detailsTab === "problems"} onclick={() => (detailsTab = "problems")}
            >Problems{status.failed > 0 ? ` (${status.failed})` : ""}</button
          >
          <button type="button" role="tab" aria-selected={detailsTab === "skipped"} onclick={() => (detailsTab = "skipped")}
            >Not synced{status.skipped > 0 ? ` (${status.skipped})` : ""}</button
          >
        </div>
      </div>
      {#if details === null}
        <p class="muted small-text">Loading…</p>
      {:else if detailsTab === "activity"}
        {#if details.activity.length === 0}
          <p class="muted small-text">Nothing transferred yet.</p>
        {:else}
          <ul class="activity">
            {#each details.activity as a, i (i)}
              <li>
                <p class="a-time" title={when(a.atMs)}>{formatAgo(a.atMs)}</p>
                <p class="a-op">{OP_LABELS[a.op] ?? a.op}</p>
                <p class="a-path mono" title={a.path}>{a.path}</p>
                <p class="a-note small-text muted">{a.message ?? (a.bytes > 0 ? formatDecimalSize(a.bytes) : "")}</p>
              </li>
            {/each}
          </ul>
        {/if}
      {:else if detailsTab === "problems"}
        {#if details.failed.length === 0}
          <p class="muted small-text">No problems.</p>
        {:else}
          <div class="actions">
            <button type="button" class="small" disabled={busy !== null} onclick={() => void act("retry", () => retryFolderSync(status.id))}
              ><Icon name="refresh" size={13} />Try again now</button
            >
          </div>
          <ul class="activity">
            {#each details.failed as f (f.path)}
              <li>
                <p class="a-time">{f.attempts}×</p>
                <p class="a-op">{OP_LABELS[f.op] ?? f.op}</p>
                <p class="a-path mono" title={f.path}>{f.path}</p>
                <p class="a-note small-text">{f.message}. <span class="muted">Next try {formatAgo(f.nextMs).replace(" ago", "")}.</span></p>
              </li>
            {/each}
          </ul>
        {/if}
      {:else if details.skipped.length === 0}
        <p class="muted small-text">Every file is synced.</p>
      {:else}
        <ul class="activity">
          {#each details.skipped as s (s.side + s.path)}
            <li>
              <p class="a-time">{s.side === "local" ? "Here" : "Bucket"}</p>
              <p class="a-op">Not synced</p>
              <p class="a-path mono" title={s.path}>{s.path}</p>
              <p class="a-note small-text muted">{s.reason}</p>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="card settings" aria-labelledby="sync-settings-heading">
      <h3 class="section-title" id="sync-settings-heading">Settings</h3>
      <label class="field">
        <span>Check the bucket for changes made elsewhere</span>
        <select
          value={status.intervalMinutes}
          disabled={busy !== null}
          onchange={(e) => {
            const minutes = Number(e.currentTarget.value);
            void act("interval", () => setFolderSyncInterval(status.id, minutes));
          }}
        >
          {#each INTERVALS as m (m)}
            <option value={m}>Every {m} minutes</option>
          {/each}
        </select>
        <span class="hint">Each check lists the whole bucket: about one request per 1,000 files.</span>
      </label>
      <p class="small-text">
        <strong>Earlier versions in the bucket:</strong>
        {versioningText(status.versioning)}
      </p>
      <p class="small-text">
        <strong>Sync trash:</strong>
        {#if status.trashFiles === 0}
          empty. Files deleted in the bucket are moved there from this folder and kept 30 days.
        {:else}
          {formatCount(status.trashFiles)} file{status.trashFiles === 1 ? "" : "s"} ({formatDecimalSize(status.trashBytes)}) deleted in the bucket
          and removed from this folder, kept 30 days.
          <button type="button" class="link" onclick={() => void act("trash", () => openSyncFolder(status.id, "trash"))}>Open the sync trash</button>
        {/if}
      </p>
      <div class="actions">
        <button type="button" class="danger" disabled={busy !== null || status.state === "moving"} onclick={() => (confirmOff = true)}
          >Turn sync off…</button
        >
      </div>
    </div>
  {/if}
</section>

{#if confirmOff && status}
  <Modal title="Turn off sync for {status.bucket}?" onclose={() => (confirmOff = false)} locked={busy === "off"}>
    <p>
      The files stay in <span class="mono">{status.path}</span> and the bucket is not changed. Nothing is deleted on either side.
    </p>
    <p class="small-text muted">
      If you turn sync on again later, the folder and the bucket are compared from scratch: files only on one side are copied to
      the other, and nothing is deleted because of this earlier sync.
    </p>
    {#snippet footer()}
      <button type="button" class="primary danger" disabled={busy !== null} onclick={() => void turnOff()}
        >{busy === "off" ? "Turning off…" : "Turn sync off"}</button
      >
      <button type="button" disabled={busy === "off"} onclick={() => (confirmOff = false)}>Cancel</button>
    {/snippet}
  </Modal>
{/if}

{#snippet confirm(c: FolderChoice)}
  <div class="card confirm" aria-labelledby="confirm-heading" data-sync-choice>
    <h3 class="section-title" id="confirm-heading">{c.sync === null ? "Sync this folder?" : "Use this folder for sync?"}</h3>
    <dl class="meta-list facts">
      <dt>Folder</dt>
      <dd class="mono path" data-choice-path>{c.path}</dd>
      <dt>In it now</dt>
      <dd>{c.files === 0 && !c.more ? "Empty" : `${formatCount(c.files)}${c.more ? "+" : ""} files, ${formatDecimalSize(c.bytes)}`}</dd>
      <dt>Free on its drive</dt>
      <dd>{c.freeBytes === null ? "Unknown" : formatDecimalSize(c.freeBytes)}</dd>
      <dt>The bucket</dt>
      <dd>
        {c.bucketBytes === null || c.bucketMeasuredMs === null
          ? "Not measured yet (see Usage & costs)"
          : `${formatDecimalSize(c.bucketBytes)} in ${formatCount(c.bucketObjects ?? 0)} files, measured ${formatAgo(c.bucketMeasuredMs)}`}
      </dd>
      <dt>Earlier versions</dt>
      <dd>{versioningText(c.versioning)}</dd>
    </dl>
    {#each c.problems as p (p)}
      <p class="error" role="alert">{p}</p>
    {/each}
    {#each c.warnings as w (w)}
      <p class="notice warning">{w}</p>
    {/each}
    {#if c.sync === null}
      <label class="field">
        <span>Check the bucket for changes made elsewhere</span>
        <select value={interval} onchange={(e) => (interval = Number(e.currentTarget.value))}>
          {#each INTERVALS as m (m)}
            <option value={m}>Every {m} minutes</option>
          {/each}
        </select>
      </label>
    {:else if c.problems.length === 0}
      <fieldset class="modes">
        <legend>What happens to the files in the current folder</legend>
        <label class="check">
          <input type="radio" name="move-mode" value="move" bind:group={moveMode} disabled={!c.canMove} />
          <span>
            <strong>Move them to the new folder</strong> and go on syncing there.
            {#if c.canMove}
              On the same drive they are moved at once; across drives each file is copied and checked before the original is removed.
            {:else}
              Only into an empty folder.
            {/if}
          </span>
        </label>
        <label class="check">
          <input type="radio" name="move-mode" value="fresh" bind:group={moveMode} />
          <span>
            <strong>Leave them where they are</strong> (no longer synced) and sync the new folder from scratch: its files and the
            bucket are merged; nothing is deleted.
          </span>
        </label>
      </fieldset>
    {/if}
    <div class="actions">
      <button type="button" class="primary" disabled={c.problems.length > 0 || busy !== null} onclick={() => void confirmChoice()}
        >{busy === "confirm" ? "Working…" : c.sync === null ? "Start sync" : moveMode === "move" ? "Move files" : "Switch folder"}</button
      >
      <button type="button" disabled={choosing || busy !== null} onclick={() => void choose()}>Choose another folder…</button>
      <button type="button" class="ghost" disabled={busy !== null} onclick={cancelChoice}>Cancel</button>
    </div>
  </div>
{/snippet}

<style>
  .sync {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: var(--content-w);
  }

  .sync-head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .sync-head h2 {
    margin-bottom: var(--space-1);
  }

  .intro .intro-icon {
    display: inline-flex;
    margin-bottom: var(--space-2);
    color: var(--accent);
  }

  .status-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  /* Paragraphs, not spans, for text in flex rows: WebKitGTK leaves a flex
     item span's text out of the accessibility tree (frontend.md). */
  .status-text,
  .status-hint {
    margin: 0;
  }

  .status-hint {
    margin-top: var(--space-2);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .migration {
    margin-top: var(--space-3);
  }

  .migration p {
    margin: 0 0 var(--space-1);
  }

  progress {
    width: 100%;
    height: 8px;
    accent-color: var(--accent);
  }

  .folder-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .folder-icon {
    color: var(--text-muted);
    display: inline-flex;
  }

  .path {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: var(--space-3);
  }

  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    min-width: 0;
  }

  .tile-label,
  .tile-value,
  .tile-sub {
    margin: 0;
  }

  .tile-label {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .tile-value {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    line-height: var(--leading-tight);
    overflow-wrap: anywhere;
  }

  .tile-sub {
    font-size: var(--text-xs);
    color: var(--text-subtle);
  }

  .pending {
    margin: 0;
    color: var(--text-muted);
  }

  .transfers,
  .conflict-list,
  .activity {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }

  .transfers li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) minmax(4rem, 10rem) auto;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .t-name,
  .t-figure {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .t-figure {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }

  .dir {
    display: inline-flex;
    color: var(--text-muted);
  }

  .held p {
    margin: 0 0 var(--space-2);
  }

  .conflict-list li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }

  .c-name {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--text-sm);
  }

  .details-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .segmented-tabs {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    background: var(--surface-2);
    border-radius: var(--radius-md);
  }

  .segmented-tabs button {
    border: none;
    background: transparent;
    min-height: 26px;
    font-size: var(--text-xs);
  }

  .segmented-tabs button[aria-selected="true"] {
    background: var(--surface);
    box-shadow: var(--shadow-sm);
  }

  .activity li {
    display: grid;
    grid-template-columns: 7rem 10rem minmax(0, 1fr);
    grid-template-areas:
      "time op path"
      "time op note";
    column-gap: var(--space-2);
    font-size: var(--text-sm);
    padding-bottom: var(--space-2);
    border-bottom: 1px solid var(--border);
  }

  .activity li > p {
    margin: 0;
    min-width: 0;
  }

  .a-time {
    grid-area: time;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .a-op {
    grid-area: op;
  }

  .a-path {
    grid-area: path;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .a-note {
    grid-area: note;
    overflow-wrap: anywhere;
  }

  .settings .field {
    max-width: 26rem;
  }

  .settings p {
    margin: var(--space-2) 0;
  }

  .facts {
    margin: 0 0 var(--space-3);
  }

  .modes {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    margin: 0 0 var(--space-3);
    display: grid;
    gap: var(--space-2);
  }

  .modes legend {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    padding: 0 var(--space-1);
  }

  .modes .check {
    align-items: flex-start;
    font-size: var(--text-sm);
  }

  @media (max-width: 640px) {
    .activity li {
      grid-template-columns: 4.5rem minmax(0, 1fr);
      grid-template-areas:
        "time op"
        "time path"
        "time note";
    }

    .transfers li {
      grid-template-columns: auto minmax(0, 1fr) auto;
    }

    .transfers progress {
      grid-column: 1 / -1;
    }
  }
</style>
