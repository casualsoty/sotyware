<script lang="ts">
  import {
    discardRecording,
    saveAutomationSettings,
    saveRecording,
    type Coordinates,
    type MouseButton,
    type RecordingOptions,
  } from "./automation";
  import { BUTTON_LABELS, describeAction, formatClock, formatDuration, keyLabel, numberOf } from "./automationFormat";
  import { formatCount } from "./format";
  import type { AutomationModel } from "./automationModel.svelte";
  import Icon from "./Icon.svelte";

  // The Recorder tab (ADR-050): records mouse and keyboard input into a
  // macro. Rust captures the input (never sotyware's own window, never the
  // stop shortcut); this page starts and stops it, shows what arrives, and
  // saves or discards the result.

  interface Props {
    model: AutomationModel;
    canStart: boolean;
    onrecord: () => void;
    onstop: () => void;
    onpause: (paused: boolean) => void;
    onopenmacro: (id: string) => void;
  }

  let { model, canStart, onrecord, onstop, onpause, onopenmacro }: Props = $props();

  const status = $derived(model.status);
  const keys = $derived(model.config?.keys ?? []);
  const session = $derived(status?.session?.kind === "recording" ? status.session : null);
  const draft = $derived(status?.draft ?? null);
  const startKeys = $derived(
    status?.hotkeys.find((h) => h.owner.kind === "profile" && h.owner.action === "start-recording" && h.registered)?.keys ?? null,
  );
  const stopKeys = $derived(
    status?.hotkeys.find((h) => h.owner.kind === "profile" && h.owner.action === "stop-recording" && h.registered)?.keys ?? null,
  );
  const folders = $derived([...new Set((model.macros ?? []).map((m) => m.folder).filter((f) => f !== ""))].sort());

  let name = $state("");
  let description = $state("");
  let folder = $state("");
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let saved = $state<{ id: string; name: string } | null>(null);
  let optionsError = $state<string | null>(null);
  let addKey = $state("");
  let timer: ReturnType<typeof setTimeout> | null = null;

  const options = $derived(model.config?.settings.recording ?? null);

  function setOptions(patch: Partial<RecordingOptions>): void {
    const c = model.config;
    if (c === null) {
      return;
    }
    const next = { ...c.settings, recording: { ...c.settings.recording, ...patch } };
    model.putSettings(next);
    if (timer !== null) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = null;
      void (async () => {
        try {
          model.putSettings(await saveAutomationSettings($state.snapshot(model.config?.settings ?? next) as typeof next));
          optionsError = null;
        } catch (e) {
          optionsError = e instanceof Error ? e.message : String(e);
        }
      })();
    }, 300);
  }

  async function save(): Promise<void> {
    if (name.trim() === "") {
      saveError = "Give the macro a name.";
      return;
    }
    saving = true;
    try {
      const s = await saveRecording(name, description, folder);
      model.putMacro(s);
      saved = { id: s.id, name: s.name };
      name = "";
      description = "";
      folder = "";
      saveError = null;
      await model.refreshStatus();
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function discard(): Promise<void> {
    try {
      await discardRecording();
      saveError = null;
      await model.refreshStatus();
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    }
  }
</script>

<section class="recorder" aria-labelledby="recorder-heading">
  <h2 id="recorder-heading">Recorder</h2>
  <p class="muted small-text">
    Records your clicks, pointer movements, scrolling and key presses with their timing, to replay them later as a
    macro.
  </p>

  {#if draft !== null && session === null}
    <div class="card draft">
      <h3>Recording ready</h3>
      <p class="muted small-text">
        {formatCount(draft.actions)} actions, {formatDuration(draft.durationMs)}{draft.window
          ? `, first input in ${draft.window.process || draft.window.title}`
          : ""}.
        {draft.skipped > 0 ? `${draft.skipped} inputs were left out (sotyware's window, ignored keys, keys it cannot replay).` : ""}
      </p>
      <ol class="events compact" aria-label="Last recorded actions">
        {#each draft.recent.slice(-8) as a, i (i)}
          <li><span class="delay">+{formatDuration(a.delayMs)}</span>{describeAction(a, keys)}</li>
        {/each}
      </ol>
      <form
        class="save-form"
        onsubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <label class="field">
          Name
          <input type="text" aria-label="Name" bind:value={name} maxlength="120" required placeholder="Daily report" />
        </label>
        <label class="field">
          <span>Folder <span class="hint">(optional)</span></span>
          <input type="text" aria-label="Folder (optional)" bind:value={folder} maxlength="80" list="recorder-folders" />
          <datalist id="recorder-folders">
            {#each folders as f (f)}<option value={f}></option>{/each}
          </datalist>
        </label>
        <label class="field wide">
          <span>Description <span class="hint">(optional)</span></span>
          <textarea rows="2" aria-label="Description (optional)" bind:value={description} maxlength="2000"></textarea>
        </label>
        {#if saveError !== null}
          <p class="callout danger wide" role="alert">{saveError}</p>
        {/if}
        <div class="actions wide">
          <button type="submit" class="primary" disabled={saving}>Save macro</button>
          <button type="button" class="danger" disabled={saving} onclick={() => void discard()}>Discard</button>
        </div>
      </form>
    </div>
  {:else if session !== null}
    <div class="card live" class:paused={session.state === "paused"}>
      <div class="live-head">
        <span class="rec-dot" class:paused={session.state !== "running"} aria-hidden="true"></span>
        <p class="live-state" role="status">
          {#if session.state === "countdown"}
            Recording starts in {Math.ceil((session.countdownMs ?? 0) / 1000)} s…
          {:else if session.state === "paused"}
            Recording paused
          {:else}
            Recording
          {/if}
        </p>
        <span class="clock">{formatClock(session.elapsedMs)}</span>
      </div>
      <p class="muted small-text">
        {formatCount(session.actions)} actions{session.skipped > 0 ? `, ${session.skipped} left out` : ""}.
        {stopKeys ? `Press ${stopKeys} to stop from any program.` : ""}
      </p>
      <div class="actions">
        {#if session.state === "paused"}
          <button type="button" onclick={() => onpause(false)}><Icon name="play" />Resume</button>
        {:else}
          <button type="button" disabled={session.state === "countdown"} onclick={() => onpause(true)}><Icon name="pause" />Pause</button>
        {/if}
        <button type="button" class="primary danger" onclick={onstop}><Icon name="stop" />Stop recording</button>
      </div>
      <ol class="events" aria-label="Recorded actions so far">
        {#each session.recent as a, i (i)}
          <li><span class="delay">+{formatDuration(a.delayMs)}</span>{describeAction(a, keys)}</li>
        {:else}
          <li class="muted">Nothing yet. Switch to the program you want to automate.</li>
        {/each}
      </ol>
    </div>
  {:else}
    <div class="card idle">
      <button type="button" class="record" disabled={!canStart} onclick={onrecord}>
        <span class="rec-dot big" aria-hidden="true"></span>Start recording
      </button>
      <p class="muted small-text">
        {(model.config?.settings.dashboardCountdownMs ?? 0) > 0
          ? `Recording starts after ${(model.config?.settings.dashboardCountdownMs ?? 0) / 1000} s, so you can switch to the program first. `
          : ""}
        {startKeys ? `Or press ${startKeys} in any program to start${stopKeys ? `, ${stopKeys} to stop` : ""}.` : ""}
      </p>
      {#if saved !== null}
        {@const s = saved}
        <p class="callout success" role="status">
          Saved “{s.name}”. <button type="button" class="link" onclick={() => onopenmacro(s.id)}>Open it in the editor</button>
        </p>
      {/if}
    </div>
  {/if}

  <p class="callout warning privacy">
    <strong>Everything you type while recording is saved in the macro</strong>, as plain text on this computer, including
    passwords. Pause the recording before typing anything secret.
  </p>

  {#if options !== null}
    {@const o = options}
    <details class="options" open={session === null && draft === null}>
      <summary>Recording options</summary>
      <fieldset class="opts" disabled={session !== null}>
        <legend class="visually-hidden">Recording options</legend>
        <fieldset class="field">
          <legend>Record</legend>
          <label class="check"><input type="checkbox" checked={o.mouseButtons} onchange={(e) => setOptions({ mouseButtons: (e.currentTarget as HTMLInputElement).checked })} />Clicks</label>
          <label class="check"><input type="checkbox" checked={o.mouseMoves} onchange={(e) => setOptions({ mouseMoves: (e.currentTarget as HTMLInputElement).checked })} />Pointer movements</label>
          <label class="check"><input type="checkbox" checked={o.scroll} onchange={(e) => setOptions({ scroll: (e.currentTarget as HTMLInputElement).checked })} />Scrolling</label>
          <label class="check"><input type="checkbox" checked={o.keyboard} onchange={(e) => setOptions({ keyboard: (e.currentTarget as HTMLInputElement).checked })} />Keyboard</label>
        </fieldset>
        <div class="field">
          <label for="rec-coords">Replay positions</label>
          <select id="rec-coords" value={o.coordinates} onchange={(e) => setOptions({ coordinates: (e.currentTarget as HTMLSelectElement).value as Coordinates })}>
            <option value="absolute">At the same place on the screen</option>
            <option value="relative">Relative to where the pointer starts</option>
            <option value="window">Relative to the program's window</option>
          </select>
          <p class="hint">Can be changed for each macro later.</p>
        </div>
        <div class="field">
          <label for="rec-rate">Pointer movements kept</label>
          <select id="rec-rate" value={o.moveRateHz} onchange={(e) => setOptions({ moveRateHz: Number((e.currentTarget as HTMLSelectElement).value) })}>
            {#each [10, 30, 60, 125, 250, 1000] as hz (hz)}
              <option value={hz}>{hz === 1000 ? "All of them" : `Up to ${hz} per second`}</option>
            {/each}
          </select>
        </div>
        <div class="field">
          <label for="rec-min">Ignore movements under</label>
          <div class="inline">
            <input id="rec-min" type="number" class="short" min="0" max="500" value={o.minMovePx} onchange={(e) => { const v = numberOf(e.currentTarget as HTMLInputElement); if (v !== null && v >= 0) { setOptions({ minMovePx: Math.round(v) }); } }} />
            <p class="muted">px</p>
          </div>
          <p class="hint">The position just before a click or key is always kept.</p>
        </div>
        <fieldset class="field">
          <legend>Never record these buttons</legend>
          {#each ["left", "right", "middle", "back", "forward"] as MouseButton[] as b (b)}
            <label class="check">
              <input
                type="checkbox"
                checked={o.ignoredButtons.includes(b)}
                onchange={(e) =>
                  setOptions({
                    ignoredButtons: (e.currentTarget as HTMLInputElement).checked
                      ? [...o.ignoredButtons, b]
                      : o.ignoredButtons.filter((x) => x !== b),
                  })}
              />{BUTTON_LABELS[b]}
            </label>
          {/each}
        </fieldset>
        <div class="field">
          <label for="rec-ignore">Never record these keys</label>
          <div class="chips">
            {#each o.ignoredKeys as k (k)}
              <span class="chip">
                {keyLabel(k, keys)}
                <button type="button" class="ghost icon small" aria-label="Record {keyLabel(k, keys)} again" onclick={() => setOptions({ ignoredKeys: o.ignoredKeys.filter((x) => x !== k) })}><Icon name="close" size={12} /></button>
              </span>
            {:else}
              <span class="muted small-text">None</span>
            {/each}
          </div>
          <div class="inline">
            <select id="rec-ignore" bind:value={addKey}>
              <option value="">Choose a key…</option>
              {#each keys.filter((k) => !o.ignoredKeys.includes(k.code)) as k (k.code)}
                <option value={k.code}>{k.label}</option>
              {/each}
            </select>
            <button
              type="button"
              class="small"
              disabled={addKey === ""}
              onclick={() => {
                setOptions({ ignoredKeys: [...o.ignoredKeys, addKey] });
                addKey = "";
              }}>Add</button
            >
          </div>
        </div>
      </fieldset>
      {#if optionsError !== null}
        <p class="callout danger" role="alert">Not saved: {optionsError}</p>
      {/if}
    </details>
  {/if}
</section>

<style>
  .recorder > h2 {
    margin-bottom: var(--space-1);
  }

  .card {
    margin-top: var(--space-4);
  }

  .idle {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .record {
    min-height: 48px;
    padding: 0 var(--space-5);
    font-size: var(--text-md);
    gap: var(--space-3);
    border-radius: var(--radius-full);
  }

  .rec-dot {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--danger-solid);
    flex: none;
  }

  .rec-dot.big {
    width: 14px;
    height: 14px;
  }

  .live .rec-dot:not(.paused) {
    animation: pulse 1.2s ease-in-out infinite;
  }

  .rec-dot.paused {
    background: var(--warning);
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .live .rec-dot:not(.paused) {
      animation: none;
    }
  }

  .live {
    border-color: var(--danger-border);
  }

  .live.paused {
    border-color: var(--warning-border);
  }

  .live-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .live-state {
    margin: 0;
    flex: 1;
    font-weight: var(--weight-semibold);
  }

  .clock {
    font-variant-numeric: tabular-nums;
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
  }

  .events {
    max-height: 16rem;
    overflow: auto;
    margin: var(--space-3) 0 0;
    padding: var(--space-2) var(--space-3);
    list-style: none;
    font-size: var(--text-sm);
    background: var(--surface-2);
    border-radius: var(--radius-md);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .events.compact {
    max-height: 10rem;
  }

  .delay {
    display: inline-block;
    min-width: 5rem;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .save-form {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: var(--space-3) var(--space-4);
    margin-top: var(--space-4);
  }

  .wide {
    grid-column: 1 / -1;
  }

  .privacy {
    margin-top: var(--space-4);
  }

  .options {
    margin-top: var(--space-4);
  }

  .options summary {
    font-weight: var(--weight-semibold);
    padding: var(--space-1) 0;
  }

  .opts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
    gap: var(--space-4) var(--space-5);
    margin-top: var(--space-3);
  }

  fieldset.field legend {
    margin-bottom: var(--space-1);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }

  .inline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .short {
    width: 6rem;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    margin-bottom: var(--space-2);
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-left: var(--space-2);
    font-size: var(--text-xs);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
  }

  /* Text in a flex box is a <p>: WebKitGTK leaves a <span> flex item's
     text out of the accessibility tree (frontend.md). No margin, as the
     <span> had none. */
  .field > p.hint,
  .inline > p {
    margin: 0;
  }
</style>
