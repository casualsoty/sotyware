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
          <span class="meta">{feature.version} · <span class="state">{STATE_LABELS[feature.state]}</span></span>
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
  section {
    margin-top: 2rem;
  }

  h2 {
    font-size: 1.1em;
  }

  ul {
    list-style: none;
    padding: 0;
  }

  li {
    padding: 0.5rem 0;
  }

  .meta {
    margin-left: 0.5rem;
    color: #6b6b73;
    font-size: 0.9em;
  }

  .description {
    margin: 0.25rem 0 0 1.6rem;
    font-size: 0.9em;
  }

  .feature-error,
  .error {
    color: #c62828;
  }

  .feature-error {
    margin: 0.25rem 0 0 1.6rem;
    font-size: 0.9em;
  }

  @media (prefers-color-scheme: dark) {
    .meta {
      color: #a0a0a8;
    }

    .feature-error,
    .error {
      color: #ef7070;
    }
  }
</style>
