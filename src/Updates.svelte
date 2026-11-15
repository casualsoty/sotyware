<script lang="ts">
  import { onMount } from "svelte";
  import { formatSize } from "./format";
  import {
    cancelUpdate,
    checkForUpdate,
    getUpdateStatus,
    installUpdate,
    postponeUpdate,
    type InstallEvent,
    type UpdateStatus,
  } from "./updates";

  // Updates (ADR-033). Rust checks a signed manifest and verifies the
  // package; this page only shows what Rust reports and sends the user's
  // clicks. Nothing is installed without "Update now".

  type Install =
    | { stage: "downloading"; downloaded: number; total: number | null }
    | { stage: "verified" }
    | { stage: "installing" }
    | { stage: "installed" }
    | { stage: "failed"; message: string }
    | { stage: "cancelled" };

  let status = $state<UpdateStatus | null>(null);
  let error = $state<string | null>(null);
  let checking = $state(false);
  let install = $state<Install | null>(null);

  const running = $derived(
    install !== null && (install.stage === "downloading" || install.stage === "verified" || install.stage === "installing"),
  );

  async function load(): Promise<void> {
    try {
      status = await getUpdateStatus();
    } catch (e) {
      error = `Could not read the update status: ${String(e)}`;
    }
  }

  async function check(): Promise<void> {
    error = null;
    checking = true;
    try {
      status = await checkForUpdate();
      install = null;
    } catch (e) {
      error = `Could not check for updates: ${String(e)}`;
      await load();
    } finally {
      checking = false;
    }
  }

  function onEvent(e: InstallEvent): void {
    switch (e.type) {
      case "progress":
        install = { stage: "downloading", downloaded: e.downloaded, total: e.total };
        break;
      case "failed":
        install = { stage: "failed", message: e.message };
        void load();
        break;
      default:
        install = { stage: e.type };
    }
  }

  async function updateNow(): Promise<void> {
    error = null;
    install = { stage: "downloading", downloaded: 0, total: null };
    try {
      await installUpdate(onEvent);
    } catch (e) {
      install = null;
      error = `Could not start the update: ${String(e)}`;
      await load();
    }
  }

  async function cancel(): Promise<void> {
    try {
      await cancelUpdate();
    } catch (e) {
      error = `Could not cancel the update: ${String(e)}`;
    }
  }

  async function later(): Promise<void> {
    error = null;
    try {
      await postponeUpdate();
      await load();
    } catch (e) {
      error = `Could not postpone the update: ${String(e)}`;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  function progressText(downloaded: number, total: number | null): string {
    return total === null
      ? `Downloading… ${formatSize(downloaded)}`
      : `Downloading… ${formatSize(downloaded)} of ${formatSize(total)}`;
  }

  onMount(() => {
    void load();
  });
</script>

<section aria-labelledby="updates-heading">
  <h2 id="updates-heading">Updates</h2>
  {#if status}
    <p>sotyware {status.current}</p>
    {#if !status.configured}
      <!-- A fixed note, not a live region. -->
      <p class="notice">
        Updates are not set up in this build: no release signing key and update server exist yet.
        New versions have to be installed by hand.
      </p>
    {:else}
      <p class="meta">
        {status.lastCheckMs === null ? "Not checked yet." : `Last checked ${when(status.lastCheckMs)}.`}
      </p>
      {#if status.error}
        <p class="error" role="alert">The last check failed: {status.error}</p>
      {/if}
      {#if status.available}
        <div class="available">
          <p>
            <strong>sotyware {status.available.version} is available.</strong>
            {#if status.postponed}<span class="meta">(postponed)</span>{/if}
          </p>
          {#if status.available.notes}
            <p class="notes">{status.available.notes}</p>
          {/if}
          {#if install?.stage === "downloading"}
            <p role="status">{progressText(install.downloaded, install.total)}</p>
            <button type="button" onclick={cancel}>Cancel</button>
          {:else if install?.stage === "verified"}
            <p role="status">The update was downloaded and verified.</p>
          {:else if install?.stage === "installing"}
            <p role="status">Installing… sotyware closes and the installer starts the new version.</p>
          {:else if install?.stage === "installed"}
            <p role="status">The update is installed. It takes effect when sotyware starts again.</p>
          {:else}
            {#if install?.stage === "cancelled"}
              <p role="status">The download was cancelled; nothing was installed.</p>
            {:else if install?.stage === "failed"}
              <p class="error" role="alert">The update failed; nothing was installed: {install.message}</p>
            {/if}
            {#if status.blockedBy.length > 0}
              <p class="meta">Updating restarts sotyware. Wait until {status.blockedBy.join(", ")} has finished.</p>
            {/if}
            <div class="actions">
              <button type="button" disabled={status.blockedBy.length > 0} onclick={updateNow}>
                {install?.stage === "failed" ? "Try again" : "Update now"}
              </button>
              {#if !status.postponed}
                <button type="button" onclick={later}>Later</button>
              {/if}
            </div>
          {/if}
        </div>
      {:else if status.lastCheckMs !== null && !status.error}
        <p>sotyware is up to date.</p>
      {/if}
      <button type="button" disabled={checking || running} onclick={check}>
        {checking ? "Checking…" : "Check for updates"}
      </button>
    {/if}
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</section>

<style>
  section {
    margin-top: 2rem;
  }

  h2 {
    font-size: 1.1em;
  }

  button {
    font: inherit;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }

  .meta {
    color: #6b6b73;
    font-size: 0.9em;
  }

  .notes {
    white-space: pre-line;
    font-size: 0.9em;
  }

  .notice {
    font-size: 0.9em;
  }

  .error {
    color: #c62828;
  }

  @media (prefers-color-scheme: dark) {
    .meta {
      color: #a1a1aa;
    }

    .error {
      color: #ef9a9a;
    }
  }
</style>
