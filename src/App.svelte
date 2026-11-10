<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let error = $state<string | null>(null);

  // Hiding only affects the window; the Rust process and tray icon keep running.
  async function hideDashboard(): Promise<void> {
    error = null;
    try {
      await getCurrentWindow().hide();
    } catch (e) {
      error = `Could not hide the dashboard: ${String(e)}`;
    }
  }
</script>

<main>
  <h1>sotyware</h1>
  <p>Welcome. sotyware keeps running in the background — reopen it any time from the system tray.</p>
  <button type="button" onclick={hideDashboard}>Hide dashboard</button>
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

  .error {
    color: #c62828;
  }
</style>
