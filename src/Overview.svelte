<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { listBackups, type Health, type JobStatus } from "./backup";
  import type { AccountInfo } from "./cloud";
  import { CHECK_LABELS, checkTone } from "./cloudMessages";
  import Icon from "./Icon.svelte";
  import type { Section } from "./navigation";
  import type { OverlayModel } from "./overlayModel.svelte";
  import type { UpdatesModel } from "./updatesModel.svelte";

  // The dashboard's first page: how sotyware runs (tray or dashboard-only),
  // and one card each for storage accounts, backups, the gaming notes
  // overlay and updates, each leading to its page. It reads only; every
  // action is on those pages, except showing or hiding the overlay.

  type OperatingMode = "tray" | "dashboard-only";

  interface Props {
    mode: OperatingMode | null;
    modeError: string | null;
    accounts: AccountInfo[] | null;
    accountsError: string | null;
    updates: UpdatesModel;
    overlay: OverlayModel;
    onnavigate: (section: Section) => void;
    onopenaccount: (id: string) => void;
    onaddaccount: () => void;
    onopenupdates: () => void;
    onopenoverlay: () => void;
  }

  let {
    mode,
    modeError,
    accounts,
    accountsError,
    updates,
    overlay,
    onnavigate,
    onopenaccount,
    onaddaccount,
    onopenupdates,
    onopenoverlay,
  }: Props = $props();

  const mac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
  const trayName = mac ? "menu bar" : "system tray";

  let error = $state<string | null>(null);
  let jobs = $state<JobStatus[] | null>(null);
  let backupsError = $state(false);

  const GOOD: readonly Health[] = ["healthy"];
  const attention = $derived(jobs === null ? 0 : jobs.filter((j) => !GOOD.includes(j.health) && j.health !== "never-run").length);
  const connected = $derived(accounts === null ? 0 : accounts.filter((a) => a.lastCheck?.status === "connected").length);
  const updateStatus = $derived(updates.status);
  const overlayStatus = $derived(overlay.status);
  const overlayKeys = $derived(overlayStatus?.hotkeys.find((h) => h.action === "toggle-visible" && h.registered)?.keys ?? null);

  async function showOverlay(visible: boolean): Promise<void> {
    error = null;
    try {
      await overlay.setVisible(visible);
    } catch (e) {
      error = `Could not ${visible ? "show" : "hide"} the notes overlay: ${e instanceof Error ? e.message : String(e)}`;
    }
  }

  // Hiding only affects the window; the Rust process and tray icon keep running.
  async function hideDashboard(): Promise<void> {
    error = null;
    try {
      await getCurrentWindow().hide();
    } catch (e) {
      error = `Could not hide the dashboard: ${String(e)}`;
    }
  }

  async function loadBackups(): Promise<void> {
    try {
      jobs = (await listBackups()).jobs;
    } catch {
      // The Backups page shows the reason; this summary only says it is unknown.
      backupsError = true;
    }
  }

  onMount(() => {
    void loadBackups();
  });
</script>

<div class="page">
  <header class="page-header">
    <div class="titles">
      <h1>Overview</h1>
      {#if mode === "tray"}
        <p class="lede">Welcome. sotyware keeps running in the background — reopen it any time from the {trayName}.</p>
      {:else if mode === "dashboard-only"}
        <p class="lede">Welcome.</p>
      {/if}
    </div>
    {#if mode === "tray"}
      <button type="button" onclick={hideDashboard}>Hide dashboard</button>
    {/if}
  </header>

  {#if mode === "dashboard-only"}
    <p class="callout warning" role="status">
      No compatible system tray is available, so sotyware cannot run in the background and the dashboard cannot be
      hidden. Keep this window open while you use sotyware. Closing it quits sotyware.
    </p>
  {/if}
  {#if modeError}
    <p class="error" role="alert">{modeError}</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="cards">
    <section class="card summary" aria-labelledby="overview-storage">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true"><Icon name="cloud" size={18} /></span>
        <h2 id="overview-storage">Storage</h2>
      </div>
      {#if accountsError !== null}
        <p class="muted small-text">The storage accounts could not be loaded. Open Cloud Storage for details.</p>
      {:else if accounts === null}
        <p class="muted small-text">Loading…</p>
      {:else if accounts.length === 0}
        <p class="muted small-text">No storage accounts yet. Connect a bucket at Backblaze B2, Hetzner or another S3-compatible service.</p>
        <div class="card-foot">
          <button type="button" class="primary" onclick={onaddaccount}>Connect a bucket</button>
        </div>
      {:else}
        <p class="figure">
          {accounts.length} account{accounts.length === 1 ? "" : "s"}
          <span class="muted">· {connected} connected</span>
        </p>
        <ul class="rows">
          {#each accounts.slice(0, 5) as a (a.id)}
            {@const tone = a.configError !== null ? "danger" : checkTone(a.lastCheck?.status ?? null)}
            <li>
              <span class="dot {tone}" aria-hidden="true"></span>
              <button type="button" class="link row-link" onclick={() => onopenaccount(a.id)}>{a.label}</button>
              <span class="muted row-meta">{a.lastCheck ? CHECK_LABELS[a.lastCheck.status] : "Not checked yet"}</span>
            </li>
          {/each}
        </ul>
        <div class="card-foot">
          <button type="button" class="ghost" onclick={() => onnavigate("cloud-storage")}>Manage accounts<Icon name="chevron-right" size={14} /></button>
        </div>
      {/if}
    </section>

    <section class="card summary" aria-labelledby="overview-backups">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true"><Icon name="backup" size={18} /></span>
        <h2 id="overview-backups">Backups</h2>
      </div>
      {#if backupsError}
        <p class="muted small-text">The backup jobs could not be loaded. Open Backups for details.</p>
      {:else if jobs === null}
        <p class="muted small-text">Loading…</p>
      {:else if jobs.length === 0}
        <p class="muted small-text">No backup jobs yet. A job keeps a verified copy of a bucket in a second, independent place.</p>
      {:else}
        <p class="figure">
          {jobs.length} job{jobs.length === 1 ? "" : "s"}
          {#if attention > 0}
            <span class="badge warning">{attention} need{attention === 1 ? "s" : ""} attention</span>
          {:else}
            <span class="badge success">All verified or not run yet</span>
          {/if}
        </p>
      {/if}
      <div class="card-foot">
        <button type="button" class="ghost" onclick={() => onnavigate("backups")}>Open Backups<Icon name="chevron-right" size={14} /></button>
      </div>
    </section>

    <section class="card summary" aria-labelledby="overview-overlay">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true"><Icon name="note" size={18} /></span>
        <h2 id="overview-overlay">Notes overlay</h2>
      </div>
      {#if overlayStatus === null}
        <p class="muted small-text">Loading…</p>
      {:else if !overlayStatus.running}
        <p class="muted small-text">Notes and checklists in a small window above your game. Off until you turn it on.</p>
      {:else}
        <p class="figure">
          {overlayStatus.visible ? "Shown" : "Hidden"}
          <span class="muted">· {overlayStatus.noteCount} note{overlayStatus.noteCount === 1 ? "" : "s"}</span>
          {#if overlayStatus.mode === "passive"}<span class="badge">Passive</span>{/if}
        </p>
        {#if overlayKeys !== null}<p class="muted small-text">{overlayKeys} shows and hides it.</p>{/if}
      {/if}
      <div class="card-foot foot-row">
        {#if overlayStatus?.running}
          <button type="button" class="small" onclick={() => void showOverlay(!overlayStatus.visible)}
            >{overlayStatus.visible ? "Hide overlay" : "Show overlay"}</button
          >
        {/if}
        <button type="button" class="ghost" onclick={onopenoverlay}
          >{overlayStatus?.running ? "Settings" : "Set up"}<Icon name="chevron-right" size={14} /></button
        >
      </div>
    </section>

    <section class="card summary" aria-labelledby="overview-updates">
      <div class="card-head">
        <span class="card-icon" aria-hidden="true"><Icon name="refresh" size={18} /></span>
        <h2 id="overview-updates">sotyware</h2>
      </div>
      {#if updateStatus === null}
        <p class="muted small-text">Loading…</p>
      {:else}
        <p class="figure">Version {updateStatus.current}</p>
        <p class="muted small-text">
          {#if !updateStatus.configured}
            Updates are not set up in this build.
          {:else if updateStatus.installed !== null}
            Version {updateStatus.installed} is installed; restart to use it.
          {:else if updateStatus.available}
            Version {updateStatus.available.version} is available.
          {:else if updateStatus.error}
            The last update check failed.
          {:else if updateStatus.lastCheckMs !== null}
            Up to date.
          {:else}
            Not checked for updates yet.
          {/if}
        </p>
      {/if}
      <div class="card-foot">
        <button type="button" class="ghost" onclick={onopenupdates}>About and updates<Icon name="chevron-right" size={14} /></button>
      </div>
    </section>
  </div>
</div>

<style>
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15.5rem, 1fr));
    gap: var(--space-4);
    margin-top: var(--space-2);
  }

  .summary {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    min-height: 11rem;
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
  }

  .card-head h2 {
    font-size: var(--text-md);
  }

  .card-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: var(--radius-md);
  }

  .figure {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--weight-medium);
  }

  .figure .muted {
    font-weight: var(--weight-regular);
  }

  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .rows li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--text-sm);
  }

  .row-link {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }

  .row-meta {
    margin-left: auto;
    white-space: nowrap;
    font-size: var(--text-xs);
  }

  .card-foot {
    margin-top: auto;
    padding-top: var(--space-2);
  }

  .card-foot .ghost {
    margin-left: calc(-1 * var(--space-3));
    color: var(--text-muted);
  }

  .foot-row {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .foot-row .small + .ghost {
    margin-left: 0;
  }
</style>
