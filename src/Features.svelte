<script lang="ts">
  import { onMount } from "svelte";
  import { listFeatures, setFeatureEnabled, type FeatureInfo, type FeatureState } from "./features";

  // Minimal view of the built-in features: enough to see their state and switch
  // them on or off. Not a feature-management UI. The list is read on mount and
  // updated from each switch's response; there are no events from Rust.
  // Rust saves each switch; when it cannot, the page says so (storageError,
  // saveError) instead of pretending the choice will be remembered.
  //
  // The checkboxes stay enabled while a switch runs: a disabled control
  // ignores the keys it gets, and WebKitGTK can take the keyboard focus from
  // it, so Space pressed again on a checkbox changed nothing while, and
  // after, its switch ran (until 2026-11-25). What disabling them protected
  // is kept another way: switches are sent one after another, in the order
  // asked, and none is sent for a feature still starting or stopping.

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

  /** Every feature as Rust last said: the list, then each switch's answer. */
  let features = $state<FeatureInfo[] | null>(null);
  let storageError = $state<string | null>(null);
  let error = $state<string | null>(null);

  /** One feature switched on or off by the user. */
  interface Switch {
    id: string;
    enabled: boolean;
  }

  /** Switches run one after another, in the order asked, as they did when
   * the page disabled every checkbox while one ran. */
  let queue: Promise<void> = Promise.resolve();
  /** Switches waiting for Rust's answer, oldest first. */
  let pending = $state.raw<Switch[]>([]);
  /** What the list shows: what Rust said, with the waiting switches applied
   * (Starting… or Stopping… until they are answered). Svelte sets a
   * checkbox whenever the value it is given changes, so a feature switched
   * again before an earlier switch was answered would otherwise jump back
   * when it is; and a switch Rust refused goes back to what Rust said. */
  const shown = $derived(
    features?.map((f) => {
      const last = pending.filter((s) => s.id === f.id).at(-1);
      return last === undefined ? f : { ...f, enabled: last.enabled, state: last.enabled ? ("initializing" as const) : ("stopping" as const) };
    }) ?? null,
  );

  /** Reads the list; false (and says why) if it cannot be read. */
  async function load(): Promise<boolean> {
    try {
      const list = await listFeatures();
      features = list.features;
      storageError = list.storageError;
      return true;
    } catch (e) {
      error = `Could not list features: ${String(e)}`;
      return false;
    }
  }

  function transient(state: FeatureState): boolean {
    return state === "initializing" || state === "stopping";
  }

  function toggle(feature: FeatureInfo, input: HTMLInputElement): void {
    const s: Switch = { id: feature.id, enabled: input.checked };
    error = null;
    pending = [...pending, s];
    queue = queue.then(async () => {
      try {
        await send(s);
      } catch (e) {
        // Never left rejected: the switches after it would not run.
        error = String(e);
      } finally {
        pending = pending.filter((p) => p !== s);
      }
    });
  }

  async function send(s: Switch): Promise<void> {
    let known = features?.find((f) => f.id === s.id);
    if (known !== undefined && transient(known.state)) {
      // Started or stopped elsewhere (the notes overlay's settings, the
      // Autoclicker page) and still starting or stopping when the list was
      // read: this page hears of the end only by reading the list again.
      if (!(await load())) {
        return;
      }
      known = features?.find((f) => f.id === s.id);
    }
    if (known === undefined || known.enabled === s.enabled) {
      return;
    }
    const verb = s.enabled ? "enable" : "disable";
    if (transient(known.state)) {
      // Rust would refuse it: the registry switches a feature only between
      // its transitions.
      const doing = known.state === "initializing" ? "starting" : "stopping";
      error = `Could not ${verb} ${known.name}: it is still ${doing}. Try again in a moment.`;
      return;
    }
    try {
      const { feature: updated, saveError } = await setFeatureEnabled(s.id, s.enabled);
      features = (features ?? []).map((f) => (f.id === updated.id ? updated : f));
      if (saveError !== null) {
        // The switch happened; only remembering it failed.
        const now = updated.enabled ? "enabled" : "disabled";
        error =
          `Could not save the choice for ${known.name}: ${saveError}. ` +
          `It stays ${now} until sotyware quits; the next start uses the last saved choice or the default.`;
      }
    } catch (e) {
      // Nothing changed on the Rust side: the checkbox goes back once no
      // later switch of this feature waits.
      error = `Could not ${verb} ${known.name}: ${String(e)}`;
    }
  }

  onMount(() => {
    void load();
  });
</script>

<section aria-labelledby="features-heading">
  <h2 id="features-heading">Features</h2>
  <p class="muted small-text">Built-in parts of sotyware that run in the background. Your choice is remembered.</p>
  {#if shown}
    {#if shown.length === 0}
      <p>No built-in features.</p>
    {/if}
    <ul>
      {#each shown as feature (feature.id)}
        <li data-feature={feature.id}>
          <label>
            <input type="checkbox" checked={feature.enabled} onchange={(e) => toggle(feature, e.currentTarget)} />
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
