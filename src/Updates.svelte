<script lang="ts">
  import { onMount } from "svelte";
  import { formatSize } from "./format";
  import Icon from "./Icon.svelte";
  import { FAILURE_TITLES, UpdatesModel } from "./updatesModel.svelte";

  // Settings › About & updates (ADR-033, ADR-036). Rust checks the signed
  // manifest of the latest GitHub release and verifies the package; this
  // panel only shows what Rust reports and sends the user's clicks. Nothing
  // is downloaded or installed without "Update now".

  interface Props {
    /** The window's shared update state; a panel mounted on its own makes
     * its own. */
    model?: UpdatesModel;
  }

  let { model: given }: Props = $props();
  // svelte-ignore state_referenced_locally
  const model = given ?? new UpdatesModel();

  const status = $derived(model.status);
  const install = $derived(model.install);

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  function released(rfc3339: string | null): string | null {
    if (rfc3339 === null) {
      return null;
    }
    const d = new Date(rfc3339);
    return Number.isNaN(d.getTime()) ? null : d.toLocaleDateString([], { year: "numeric", month: "long", day: "numeric" });
  }

  function progressText(downloaded: number, total: number | null): string {
    return total === null
      ? `Downloading… ${formatSize(downloaded)}`
      : `Downloading… ${formatSize(downloaded)} of ${formatSize(total)}`;
  }

  function percent(downloaded: number, total: number | null): number | null {
    return total === null || total === 0 ? null : Math.min(100, Math.floor((downloaded / total) * 100));
  }

  onMount(() => {
    void model.load();
  });
</script>

<section class="updates" aria-labelledby="updates-heading">
  <div class="about card">
    <svg class="mark" width="40" height="40" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient id="about-tile" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#2b6cf0" />
          <stop offset="1" stop-color="#0f8b8d" />
        </linearGradient>
      </defs>
      <rect x="1" y="1" width="30" height="30" rx="7" fill="url(#about-tile)" />
      <path d="M16 5 L26 10 L16 15 L6 10 Z" fill="#fff" />
      <path d="M6 10 L16 15 V27 L6 22 Z" fill="#fff" fill-opacity=".78" />
      <path d="M26 10 L16 15 V27 L26 22 Z" fill="#fff" fill-opacity=".54" />
    </svg>
    <div>
      {#if status}
        <p class="version">sotyware {status.current}</p>
        <p class="muted small-text">Cloud storage, backups and sync for your desktop · {status.channel} channel</p>
      {:else}
        <p class="version">sotyware</p>
      {/if}
    </div>
  </div>

  <h2 id="updates-heading">Updates</h2>
  {#if status}
    {#if !status.configured}
      <!-- A fixed note, not a live region. -->
      <p class="callout neutral">
        Updates are not set up in this build: it does not trust a release signing key, so it cannot verify new
        versions. New versions have to be installed by hand.
      </p>
    {:else}
      <div class="status-row">
        <!-- Inline content in a paragraph, not flex items in a div:
             WebKitGTK leaves a flex item span's text out of the
             accessibility tree. -->
        <p class="status-text">
          {#if status.installed !== null}
            <span class="badge info"><Icon name="check" size={12} />Restart required</span>
          {:else if status.available}
            <span class="badge info">Update available</span>
          {:else if status.error}
            <span class="badge danger">Last check failed</span>
          {:else if status.lastCheckMs !== null}
            <span class="badge success"><Icon name="check" size={12} />Up to date</span>
          {/if}
          <span class="meta">
            {status.lastCheckMs === null ? "Not checked yet." : `Last checked ${when(status.lastCheckMs)}.`}
          </span>
        </p>
        <button type="button" disabled={model.checking || model.running} onclick={() => void model.check()}>
          {#if model.checking}<span class="spinner" aria-hidden="true"></span>{/if}{model.checking ? "Checking…" : "Check for updates"}
        </button>
      </div>

      {#if status.error}
        <div class="callout danger" role="alert">
          <strong>{status.errorKind ? FAILURE_TITLES[status.errorKind] : "The last check failed"}.</strong>
          The last check failed: {status.error}
        </div>
      {/if}

      {#if status.installed !== null}
        <div class="available card">
          <p><strong>sotyware {status.installed} is installed.</strong></p>
          <p class="muted small-text">Restart sotyware to use the new version. Nothing that is running is interrupted.</p>
          {#if status.blockedBy.length > 0}
            <p class="meta">Wait until {status.blockedBy.join(", ")} has finished, then restart.</p>
          {/if}
          <div class="actions">
            <button
              type="button"
              class="primary"
              disabled={model.restarting || status.blockedBy.length > 0}
              onclick={() => void model.restart()}>{model.restarting ? "Restarting…" : "Restart now"}</button
            >
          </div>
        </div>
      {:else if status.available}
        <div class="available card">
          <div class="available-head">
            <p>
              <strong>sotyware {status.available.version} is available.</strong>
              {#if status.postponed}<span class="meta">(postponed)</span>{/if}
            </p>
            {#if released(status.available.pubDate)}
              <p class="meta">Released {released(status.available.pubDate)}</p>
            {/if}
          </div>
          {#if status.available.notes}
            <div class="notes">
              <h3 class="visually-hidden">Release notes</h3>
              <p>{status.available.notes}</p>
            </div>
          {/if}
          {#if install?.stage === "downloading"}
            {@const p = percent(install.downloaded, install.total)}
            {#if p !== null}
              <progress max="100" value={p} aria-label="Downloading the update">{p}%</progress>
            {/if}
            <p role="status" class="progress-text">{progressText(install.downloaded, install.total)}</p>
            <div class="actions">
              <button type="button" onclick={() => void model.cancel()}>Cancel</button>
            </div>
          {:else if install?.stage === "verified"}
            <p role="status">The update was downloaded and verified.</p>
          {:else if install?.stage === "installing"}
            <p role="status">
              <span class="spinner" aria-hidden="true"></span>
              {status.usesInstaller
                ? "Installing… sotyware closes and the installer starts the new version."
                : "Installing the new version…"}
            </p>
          {:else}
            {#if install?.stage === "cancelled"}
              <p role="status">The download was cancelled; nothing was installed.</p>
            {:else if install?.stage === "failed"}
              <div class="callout danger" role="alert">
                <strong>{FAILURE_TITLES[install.kind]}.</strong>
                The update failed; nothing was installed: {install.message}
              </div>
            {/if}
            {#if status.blockedBy.length > 0}
              <p class="meta">Updating restarts sotyware. Wait until {status.blockedBy.join(", ")} has finished.</p>
            {/if}
            <div class="actions">
              <button
                type="button"
                class="primary"
                disabled={status.blockedBy.length > 0}
                onclick={() => void model.updateNow()}
              >
                {install?.stage === "failed" ? "Try again" : "Update now"}
              </button>
              {#if !status.postponed}
                <button type="button" onclick={() => void model.later()}>Later</button>
              {/if}
            </div>
          {/if}
        </div>
      {:else if status.lastCheckMs !== null && !status.error}
        <p class="muted">sotyware is up to date.</p>
      {/if}
      <p class="hint about-checks">
        sotyware checks the latest release on GitHub once a day while the “Automatic update checks” feature is on, and
        installs only after you click Update now. Every update must carry a valid signature from sotyware's release key.
      </p>
    {/if}
  {/if}
  {#if model.error}
    <p class="error" role="alert">{model.error}</p>
  {/if}
</section>

<style>
  .about {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }

  .about p {
    margin: 0;
  }

  .mark {
    flex: none;
  }

  .version {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
  }

  h2 {
    margin-bottom: var(--space-3);
  }

  .status-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .status-text {
    margin: 0;
    line-height: 1.8;
  }

  .status-text :global(.badge) {
    margin-right: var(--space-2);
  }

  .meta {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .available {
    margin-top: var(--space-3);
  }

  .available-head {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: baseline;
    gap: var(--space-2);
  }

  .available-head p {
    margin: 0;
  }

  .notes {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-sm);
    white-space: pre-line;
    background: var(--surface-2);
    border-radius: var(--radius-md);
    max-height: 14rem;
    overflow: auto;
  }

  .notes p {
    margin: 0;
  }

  .progress-text {
    margin-top: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .about-checks {
    margin-top: var(--space-4);
  }
</style>
