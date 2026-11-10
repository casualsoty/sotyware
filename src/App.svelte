<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  // Decided once by the Rust side (`OperatingMode` in src-tauri/src/lib.rs):
  // "tray" when the tray icon exists, "dashboard-only" when no tray host is available.
  type OperatingMode = "tray" | "dashboard-only";

  let mode = $state<OperatingMode | null>(null);
  let modeError = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function loadMode(): Promise<void> {
    try {
      const value = await invoke<unknown>("operating_mode");
      if (value !== "tray" && value !== "dashboard-only") {
        throw new Error(`unknown operating mode: ${JSON.stringify(value)}`);
      }
      mode = value;
    } catch (e) {
      // Without the mode it is unknown whether a tray could bring the window back,
      // so the Hide button stays unavailable.
      modeError = `Could not check whether a system tray is available: ${String(e)}`;
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

  onMount(() => {
    void loadMode();
  });
</script>

<main>
  <h1>sotyware</h1>
  {#if mode === "tray"}
    <p>Welcome. sotyware keeps running in the background — reopen it any time from the system tray.</p>
    <button type="button" onclick={hideDashboard}>Hide dashboard</button>
  {:else if mode === "dashboard-only"}
    <p>Welcome.</p>
    <p class="notice" role="status">
      No compatible system tray is available, so sotyware cannot run in the background and
      the dashboard cannot be hidden. Keep this window open while you use sotyware. Closing it
      quits sotyware.
    </p>
  {/if}
  {#if modeError}
    <p class="error" role="alert">{modeError}</p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: system-ui, sans-serif;
    color: #1d1d1f;
    background: #f6f6f7;
  }

  @media (prefers-color-scheme: dark) {
    :global(body) {
      color: #ececef;
      background: #1c1c1f;
    }
  }

  main {
    max-width: 32rem;
    margin: 4rem auto;
    padding: 0 1.5rem;
  }

  button {
    font: inherit;
    padding: 0.5rem 1rem;
    cursor: pointer;
  }

  .notice {
    padding: 0.75rem 1rem;
    border-left: 3px solid #8a8a93;
    background: rgb(0 0 0 / 4%);
    font-size: 0.95em;
  }

  @media (prefers-color-scheme: dark) {
    .notice {
      background: rgb(255 255 255 / 6%);
    }
  }

  .error {
    color: #c62828;
  }
</style>
