<script lang="ts">
  import { onMount } from "svelte";
  import { listFeatures, setFeatureEnabled, type FeatureInfo, type FeatureState } from "./features";

  // Minimal view of the built-in features: enough to see their state and switch
  // them on or off. Not a feature-management UI. The list is read on mount and
  // updated from each switch's response; there are no events from Rust.
  // Rust saves each switch; when it cannot, the page says so (storageError,
  // saveError) instead of pretending the choice will be remembered.

  const STATE_LABELS: Record<FeatureState, string> = {
    registered: "Not started yet",
    disabled: "Disabled",
    initializing: "Starting…",
    running: "Running",
    failed: "Failed",
    stopping: "Stopping…",
  };

  const STATE_TONES: Record<FeatureState, string> = {
    registered: "",
    disabled: "",
    initializing: "info",
    running: "success",
    failed: "danger",
    stopping: "info",
  };

  let features = $state<FeatureInfo[] | null>(null);
  let storageError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let pending = $state<string | null>(null);

  async function load(): Promise<void> {
    try {
      const list = await listFeatures();
      features = list.features;
      storageError = list.storageError;
    } catch (e) {
      error = `Could not list features: ${String(e)}`;
    }
  }

  function busy(feature: FeatureInfo): boolean {
    return pending !== null || feature.state === "initializing" || feature.state === "stopping";
  }

  async function toggle(feature: FeatureInfo, input: HTMLInputElement): Promise<void> {
    const enabled = input.checked;
    error = null;
    pending = feature.id;
    try {
      const { feature: updated, saveError } = await setFeatureEnabled(feature.id, enabled);
      features = (features ?? []).map((f) => (f.id === updated.id ? updated : f));
      if (saveError !== null) {
        // The switch happened; only remembering it failed.
        const now = updated.enabled ? "enabled" : "disabled";
        error =
          `Could not save the choice for ${feature.name}: ${saveError}. ` +
          `It stays ${now} until sotyware quits; the next start uses the last saved choice or the default.`;
      }
    } catch (e) {
      // Nothing changed on the Rust side: put the checkbox back.
      input.checked = feature.enabled;
      error = `Could not ${enabled ? "enable" : "disable"} ${feature.name}: ${String(e)}`;
    } finally {
      pending = null;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<section aria-labelledby="features-heading">
  <h2 id="features-heading">Features</h2>
  <p class="muted small-text">Built-in parts of sotyware that run in the background. Your choice is remembered.</p>
  {#if features}
    {#if features.length === 0}
      <p>No built-in features.</p>
    {/if}
    <ul>
      {#each features as feature (feature.id)}
        <li data-feature={feature.id}>
          <label>
            <input
              type="checkbox"
              checked={feature.enabled}
              disabled={busy(feature)}
              onchange={(e) => toggle(feature, e.currentTarget)}
            />
            {feature.name}
          </label>
          <span class="meta">{feature.version} · <span class="state badge {STATE_TONES[feature.state]}">{STATE_LABELS[feature.state]}</span></span>
          <p class="description">{feature.description}</p>
          {#if feature.error}
            <p class="feature-error">{feature.error}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if storageError}
    <p class="error" role="alert">
      Could not load the saved feature choices: {storageError}. Every feature uses its default, and
      changes are not saved.
    </p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</section>

<style>
  h2 {
    margin-bottom: var(--space-1);
  }

  ul {
    list-style: none;
    padding: 0;
    margin: var(--space-4) 0 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) max-content;
    gap: var(--space-1) var(--space-4);
    align-items: center;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
  }

  li:last-child {
    border-bottom: none;
  }

  label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    font-weight: var(--weight-medium);
  }

  .meta {
    color: var(--text-muted);
    font-size: var(--text-xs);
    justify-self: end;
  }

  .description {
    grid-column: 1 / -1;
    margin: 0 0 0 calc(15px + var(--space-3));
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .feature-error {
    grid-column: 1 / -1;
    margin: var(--space-1) 0 0 calc(15px + var(--space-3));
    font-size: var(--text-sm);
    color: var(--danger);
  }
</style>
