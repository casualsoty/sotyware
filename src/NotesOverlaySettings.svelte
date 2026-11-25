<script lang="ts">
  import { onMount } from "svelte";
  import type { Binding } from "./automation";
  import { formatChord } from "./automationFormat";
  import { setFeatureEnabled } from "./features";
  import HotkeyInput from "./HotkeyInput.svelte";
  import Modal from "./Modal.svelte";
  import {
    ACTION_LABELS,
    OVERLAY_ACTIONS,
    OVERLAY_FEATURE,
    type Anchor,
    type Mode,
    type NewNoteTemplate,
    type OpenNote,
    type OverlayAction,
    type OverlaySettings,
    type TextSize,
  } from "./notesOverlay";
  import type { OverlayModel } from "./overlayModel.svelte";

  // Settings › Notes overlay (ADR-052): turning the gaming notes overlay on
  // and off, showing it and taking it out of passive mode from here, its
  // look, size and place, what new notes start with, its global shortcuts
  // (registered by Rust while it is on; the state column says whether the
  // system accepted each one) and a reset. Every change is saved at once.

  interface Props {
    model: OverlayModel;
  }

  let { model }: Props = $props();

  const status = $derived(model.status);
  const config = $derived(model.config);
  const keys = $derived(config?.keys ?? []);
  const limits = $derived(config?.limits ?? null);
  const running = $derived(status?.running ?? false);
  const modeKeys = $derived(status?.hotkeys.find((h) => h.action === "toggle-mode" && h.registered)?.keys ?? null);
  const showKeys = $derived(status?.hotkeys.find((h) => h.action === "toggle-visible" && h.registered)?.keys ?? null);

  let error = $state<string | null>(null);
  let saving = $state(false);
  let switching = $state(false);
  let confirmReset = $state(false);
  let sizeError = $state<string | null>(null);

  // As they lie on the screen: the top row, the middle row from left to
  // right, the bottom row.
  const ANCHOR_LABELS: Record<Anchor, string> = {
    "top-left": "Top left",
    "top-right": "Top right",
    "center-left": "Centre left",
    "mid-center-left": "Mid centre left",
    center: "Centre",
    "mid-center-right": "Mid centre right",
    "center-right": "Centre right",
    "bottom-left": "Bottom left",
    "bottom-right": "Bottom right",
  };

  function message(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  /** Saves run one after another, each computed from what the one
   * before saved, so two quick changes never undo each other. */
  let queue: Promise<void> = Promise.resolve();

  function change(next: (current: OverlaySettings) => OverlaySettings): Promise<void> {
    queue = queue.then(async () => {
      if (config === null) {
        return;
      }
      saving = true;
      try {
        await model.save(next($state.snapshot(config.settings) as OverlaySettings));
        error = null;
      } catch (e) {
        error = message(e);
      } finally {
        saving = false;
      }
    });
    return queue;
  }

  function set(patch: Partial<OverlaySettings>): Promise<void> {
    return change((s) => ({ ...s, ...patch }));
  }

  function setBinding(action: OverlayAction, b: Binding): void {
    void change((s) => ({ ...s, shortcuts: { ...s.shortcuts, [action]: b } }));
  }

  function setSize(which: "width" | "height", input: HTMLInputElement): void {
    if (limits === null) {
      return;
    }
    const value = Number(input.value);
    const [min, max] = which === "width" ? [limits.minWidth, limits.maxWidth] : [limits.minHeight, limits.maxHeight];
    if (!Number.isInteger(value) || value < min || value > max) {
      sizeError = `The ${which} must be a whole number from ${min} to ${max} pixels.`;
      return;
    }
    sizeError = null;
    void set({ [which]: value });
  }

  function stateOf(action: OverlayAction, b: Binding): { text: string; tone: string } {
    if (b.chord === null) {
      return { text: "Not set", tone: "" };
    }
    if (!b.enabled) {
      return { text: "Off", tone: "" };
    }
    if (!running) {
      return { text: "Used while the overlay is on", tone: "" };
    }
    const h = status?.hotkeys.find((x) => x.action === action && x.chord === b.chord);
    if (h === undefined) {
      return { text: "…", tone: "" };
    }
    return h.registered ? { text: "Ready", tone: "success" } : { text: h.error ?? "Not registered", tone: "warning" };
  }

  async function turn(on: boolean): Promise<void> {
    switching = true;
    try {
      const change = await setFeatureEnabled(OVERLAY_FEATURE, on);
      error = change.feature.error ?? change.saveError;
      await model.loadConfig();
    } catch (e) {
      error = message(e);
    } finally {
      switching = false;
    }
  }

  async function act(f: () => Promise<void>): Promise<void> {
    try {
      await f();
      error = null;
    } catch (e) {
      error = message(e);
    }
  }

  async function reset(): Promise<void> {
    confirmReset = false;
    saving = true;
    try {
      await model.reset();
      error = null;
      sizeError = null;
    } catch (e) {
      error = message(e);
    } finally {
      saving = false;
    }
  }

  onMount(() => {
    void model.loadConfig();
  });
</script>

<section aria-labelledby="overlay-settings-heading" class="overlay-settings">
  <h2 id="overlay-settings-heading">Gaming notes overlay</h2>
  <p class="muted small-text">
    A small window of notes and checklists that stays above your game or any other program, with its own shortcuts.
    Notes are edited in the overlay itself.
  </p>
  {#if error !== null}<p class="callout danger" role="alert">{error}</p>{/if}
  {#if model.error !== null}<p class="callout danger" role="alert">{model.error}</p>{/if}

  <div class="card state-card">
    <div class="state-line">
      <span class="dot {running ? (status?.visible ? 'success' : 'neutral') : ''}" aria-hidden="true"></span>
      <p class="state-text" role="status">
        {#if status === null}
          Loading…
        {:else if !running}
          <strong>Off.</strong> No overlay window and no shortcut until you turn it on.
        {:else}
          <strong>{status.visible ? "Shown" : "Hidden"}</strong>, {status.mode === "passive" ? "passive (clicks go through it)" : "interactive"}
          · {status.noteCount} note{status.noteCount === 1 ? "" : "s"}
        {/if}
      </p>
    </div>
    <div class="actions">
      {#if !running}
        <button type="button" class="primary" disabled={switching || status === null} onclick={() => void turn(true)}>Turn on</button>
      {:else}
        {#if status?.visible}
          <button type="button" onclick={() => void act(() => model.setVisible(false))}>Hide overlay</button>
        {:else}
          <button type="button" class="primary" onclick={() => void act(() => model.setVisible(true))}>Show overlay</button>
        {/if}
        {#if status?.mode === "passive"}
          <button type="button" onclick={() => void act(() => model.setMode("interactive"))}>Make interactive</button>
        {:else}
          <button type="button" onclick={() => void act(() => model.setMode("passive"))}>Make passive</button>
        {/if}
        <button type="button" class="ghost" disabled={switching} onclick={() => void turn(false)}>Turn off</button>
      {/if}
    </div>
    {#if running && status?.mode === "passive"}
      <p class="hint">
        Passive: the overlay lets every click through and never takes the keyboard. {modeKeys === null
          ? "Switch it back here."
          : `Switch it back with ${modeKeys}, or here.`}
      </p>
    {:else if running && showKeys !== null}
      <p class="hint">{showKeys} shows and hides it from any program.</p>
    {/if}
    {#if status?.shortcutsNote}
      <p class="callout warning small-text">No global shortcut can work here: {status.shortcutsNote}.</p>
    {/if}
  </div>

  {#if config !== null && limits !== null}
    {@const s = config.settings}
    <div class="card">
      <h3 class="section-title">Overlay</h3>
      <div class="grid">
        <div class="field">
          <label for="overlay-opacity">Background opacity</label>
          <div class="range">
            <input
              id="overlay-opacity"
              type="range"
              min={limits.minOpacity}
              max={limits.maxOpacity}
              step="5"
              value={s.opacity}
              disabled={saving}
              aria-valuetext="{s.opacity}%"
              onchange={(e) => void set({ opacity: Number((e.currentTarget as HTMLInputElement).value) })}
            />
            <span class="value">{s.opacity}%</span>
          </div>
          <p class="hint">The text stays opaque. Also on the overlay's own title bar.</p>
        </div>
        <div class="field">
          <label for="overlay-text-size">Text size</label>
          <select
            id="overlay-text-size"
            value={s.textSize}
            disabled={saving}
            onchange={(e) => void set({ textSize: (e.currentTarget as HTMLSelectElement).value as TextSize })}
          >
            <option value="small">Small</option>
            <option value="medium">Medium</option>
            <option value="large">Large</option>
          </select>
        </div>
        <div class="field">
          <label for="overlay-mode">Mode when it starts</label>
          <select
            id="overlay-mode"
            value={s.defaultMode}
            disabled={saving}
            onchange={(e) => void set({ defaultMode: (e.currentTarget as HTMLSelectElement).value as Mode })}
          >
            <option value="interactive">Interactive: clicks and typing go to the overlay</option>
            <option value="passive">Passive: clicks go through to the game</option>
          </select>
        </div>
      </div>
      <label class="check top">
        <input
          type="checkbox"
          checked={s.showAtStartup}
          disabled={saving}
          onchange={(e) => void set({ showAtStartup: (e.currentTarget as HTMLInputElement).checked })}
        />
        Show the overlay when sotyware starts
      </label>
    </div>

    <div class="card">
      <h3 class="section-title">Size and place</h3>
      <div class="grid">
        <div class="field">
          <label for="overlay-width">Width</label>
          <input
            id="overlay-width"
            type="number"
            min={limits.minWidth}
            max={limits.maxWidth}
            step="10"
            value={s.width}
            disabled={saving}
            onchange={(e) => setSize("width", e.currentTarget as HTMLInputElement)}
          />
        </div>
        <div class="field">
          <label for="overlay-height">Height</label>
          <input
            id="overlay-height"
            type="number"
            min={limits.minHeight}
            max={limits.maxHeight}
            step="10"
            value={s.height}
            disabled={saving}
            onchange={(e) => setSize("height", e.currentTarget as HTMLInputElement)}
          />
        </div>
        <div class="field">
          <label for="overlay-anchor">Opens at</label>
          <select
            id="overlay-anchor"
            value={s.anchor}
            disabled={saving}
            onchange={(e) => void set({ anchor: (e.currentTarget as HTMLSelectElement).value as Anchor })}
          >
            {#each Object.entries(ANCHOR_LABELS) as [value, label] (value)}
              <option {value}>{label}</option>
            {/each}
          </select>
        </div>
      </div>
      {#if sizeError !== null}<p class="text-danger small-text" role="alert">{sizeError}</p>{/if}
      <p class="hint">
        In pixels at 100 % scaling, on the screen the pointer is on. Changing them moves an open overlay there now. You
        can also drag and resize the overlay itself. Mid centre left and right centre it a third of the way across from
        that side.
      </p>
      <label class="check top">
        <input
          type="checkbox"
          checked={s.rememberPlacement}
          disabled={saving}
          onchange={(e) => void set({ rememberPlacement: (e.currentTarget as HTMLInputElement).checked })}
        />
        Open where it was last, at the size it had
      </label>
    </div>

    <div class="card">
      <h3 class="section-title">Notes</h3>
      <div class="grid">
        <div class="field">
          <label for="overlay-new-note">A new note starts</label>
          <select
            id="overlay-new-note"
            value={s.newNote}
            disabled={saving}
            onchange={(e) => void set({ newNote: (e.currentTarget as HTMLSelectElement).value as NewNoteTemplate })}
          >
            <option value="blank">Empty</option>
            <option value="checklist">With a checklist item</option>
          </select>
        </div>
        <div class="field">
          <label for="overlay-open-note">When the overlay starts, show</label>
          <select
            id="overlay-open-note"
            value={s.openNote}
            disabled={saving}
            onchange={(e) => void set({ openNote: (e.currentTarget as HTMLSelectElement).value as OpenNote })}
          >
            <option value="last">The note shown last</option>
            <option value="first-pinned">The first pinned note</option>
          </select>
        </div>
      </div>
      <p class="hint">
        Notes are kept on this computer only, in <code>notes.sqlite3</code> in sotyware's data folder, and saved as you
        type. They support Markdown: headings, lists, checklists, bold, italic and links.
      </p>
    </div>

    <div class="card">
      <h3 class="section-title">Shortcuts</h3>
      <p class="muted small-text">
        They work in every program while the overlay is on. Click a field and press the keys; Backspace clears it.
      </p>
      <div class="table-wrap">
        <table class="shortcuts">
          <tbody>
            {#each OVERLAY_ACTIONS as action (action)}
              {@const b = s.shortcuts[action] ?? { chord: null, enabled: false }}
              {@const st = stateOf(action, b)}
              {@const suggestion = config.suggestions[action]}
              <tr>
                <th scope="row">{ACTION_LABELS[action]}</th>
                <td>
                  <HotkeyInput
                    value={b.chord}
                    {keys}
                    label={ACTION_LABELS[action]}
                    disabled={saving}
                    onchange={(c) => setBinding(action, { chord: c, enabled: c !== null })}
                  />
                  {#if b.chord === null && suggestion !== undefined}
                    <button type="button" class="small link suggest" disabled={saving} onclick={() => setBinding(action, { chord: suggestion, enabled: true })}
                      >Use {formatChord(suggestion, keys)}</button
                    >
                  {/if}
                </td>
                <td>
                  {#if b.chord !== null}
                    <label class="check"
                      ><input
                        type="checkbox"
                        checked={b.enabled}
                        disabled={saving}
                        aria-label="{ACTION_LABELS[action]}: on"
                        onchange={(e) => setBinding(action, { chord: b.chord, enabled: (e.currentTarget as HTMLInputElement).checked })}
                      />On</label
                    >
                  {/if}
                </td>
                <td class="state {st.tone}">{st.text}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="hint">
        The suggestions use Ctrl+Shift with a function key, which games rarely use; a key another program (or the
        autoclicker) already uses cannot be taken. Some games that read the keyboard directly can keep shortcuts from
        reaching sotyware.
      </p>
    </div>

    <div class="card">
      <h3 class="section-title">Games in fullscreen</h3>
      <ul class="facts small-text">
        <li>Windowed and borderless (“fullscreen windowed”) games show the overlay above them.</li>
        <li>
          A game in exclusive fullscreen takes the whole screen for itself: no other window shows above it. Switch the
          game to borderless or windowed to see the overlay.
        </li>
        <li>While the overlay is shown above a fullscreen game, Windows composes the screen; hide it when you do not need it.</li>
      </ul>
    </div>

    <div class="card">
      <h3 class="section-title">Reset</h3>
      <p class="small-text">
        Puts every setting above back to its default, forgets where the overlay was and makes it interactive. Your
        notes are kept.
      </p>
      <button type="button" class="danger" disabled={saving} onclick={() => (confirmReset = true)}>Reset overlay settings…</button>
    </div>
  {:else if model.error === null}
    <p class="muted" role="status">Loading…</p>
  {/if}
</section>

{#if confirmReset}
  <Modal title="Reset the overlay's settings?" onclose={() => (confirmReset = false)}>
    <p>The overlay's look, size, place, shortcuts and note options go back to their defaults. Your notes are kept.</p>
    {#snippet footer()}
      <button type="button" onclick={() => (confirmReset = false)}>Cancel</button>
      <button type="button" class="primary danger" data-autofocus onclick={() => void reset()}>Reset</button>
    {/snippet}
  </Modal>
{/if}

<style>
  section > h2 {
    margin-bottom: var(--space-1);
  }

  .overlay-settings > .muted {
    margin-bottom: var(--space-4);
  }

  .card + .card {
    margin-top: var(--space-4);
  }

  .state-line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .state-text {
    margin: 0;
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    background: var(--text-subtle);
  }

  .dot.success {
    background: var(--success);
  }

  .dot.neutral {
    background: var(--accent);
  }

  .state-card .actions {
    margin-top: var(--space-3);
  }

  .state-card .hint,
  .state-card .callout {
    margin: var(--space-3) 0 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
    gap: var(--space-3) var(--space-4);
  }

  .range {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .range input {
    flex: 1;
    accent-color: var(--accent);
  }

  .range .value {
    min-width: 3em;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    font-weight: var(--weight-regular);
  }

  .field > p.hint {
    margin: 0;
  }

  .check.top {
    margin-top: var(--space-3);
  }

  .table-wrap {
    overflow-x: auto;
  }

  .shortcuts th {
    position: static;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text);
    background: none;
    white-space: normal;
    min-width: 12rem;
  }

  .suggest {
    margin-left: var(--space-2);
  }

  .state {
    font-size: var(--text-xs);
    color: var(--text-muted);
    max-width: 18rem;
  }

  .state.success {
    color: var(--success);
  }

  .state.warning {
    color: var(--warning);
  }

  .facts {
    margin: 0;
    padding-left: var(--space-5);
    color: var(--text-muted);
  }

  .card .hint {
    margin: var(--space-2) 0 0;
  }
</style>
