<script lang="ts">
  import type { PlaybackSettings, Positioning } from "./automation";
  import { numberOf } from "./automationFormat";

  // How a macro plays (ADR-050): repetitions, speed, delays, random
  // variation, where positions go, a time limit, and the part that
  // repeats. Used by the macro editor and for a profile's own settings.
  // Delays are shown in seconds and kept in milliseconds.

  interface Props {
    value: PlaybackSettings;
    /** The macro's number of actions, to offer a repeated part. */
    actions?: number | null;
    idPrefix: string;
    disabled?: boolean;
    onchange: (value: PlaybackSettings) => void;
  }

  let { value, actions = null, idPrefix, disabled = false, onchange }: Props = $props();

  const mode = $derived(value.repeat === null ? "forever" : value.repeat === 1 ? "once" : "times");

  function set(patch: Partial<PlaybackSettings>): void {
    onchange({ ...value, ...patch });
  }

  function number(e: Event): number | null {
    return numberOf(e.currentTarget as HTMLInputElement);
  }

  function seconds(e: Event): number | null {
    const v = number(e);
    return v === null ? null : Math.round(Math.max(0, v) * 1000);
  }

  function setMode(e: Event): void {
    const m = (e.currentTarget as HTMLSelectElement).value;
    set({ repeat: m === "once" ? 1 : m === "forever" ? null : Math.max(2, value.repeat ?? 2) });
  }

  const SPEEDS = [0.5, 1, 2, 4];
</script>

<fieldset class="playback" {disabled}>
  <legend class="visually-hidden">Playback</legend>
  <div class="field">
    <label for="{idPrefix}-repeat">Repeat</label>
    <div class="inline">
      <select id="{idPrefix}-repeat" value={mode} onchange={setMode}>
        <option value="once">Once</option>
        <option value="times">Several times</option>
        <option value="forever">Until stopped</option>
      </select>
      {#if mode === "times"}
        <input
          type="number"
          class="short"
          min="2"
          max="1000000"
          aria-label="Number of repetitions"
          value={value.repeat ?? 2}
          onchange={(e) => {
            const n = number(e);
            if (n !== null && n >= 1) {
              set({ repeat: Math.round(n) });
            }
          }}
        />
        <span class="unit">times</span>
      {/if}
    </div>
  </div>

  <div class="field">
    <label for="{idPrefix}-speed">Speed</label>
    <div class="inline">
      <input
        id="{idPrefix}-speed"
        type="number"
        class="short"
        min="0.1"
        max="20"
        step="0.1"
        value={value.speed}
        onchange={(e) => {
          const n = number(e);
          if (n !== null && n > 0) {
            set({ speed: n });
          }
        }}
      />
      <span class="unit">×</span>
      <span class="presets" role="group" aria-label="Speed presets">
        {#each SPEEDS as s (s)}
          <button type="button" class="small" aria-pressed={value.speed === s} onclick={() => set({ speed: s })}>{s}×</button>
        {/each}
      </span>
    </div>
    <span class="hint">Delays are divided by the speed; the delays below are not.</span>
  </div>

  <div class="field">
    <label for="{idPrefix}-start">Start delay</label>
    <div class="inline">
      <input
        id="{idPrefix}-start"
        type="number"
        class="short"
        min="0"
        max="86400"
        step="0.1"
        value={value.startDelayMs / 1000}
        onchange={(e) => {
          const ms = seconds(e);
          if (ms !== null) {
            set({ startDelayMs: ms });
          }
        }}
      />
      <span class="unit">s</span>
    </div>
  </div>

  {#if mode !== "once"}
    <div class="field">
      <label for="{idPrefix}-between">Between repetitions</label>
      <div class="inline">
        <input
          id="{idPrefix}-between"
          type="number"
          class="short"
          min="0"
          max="86400"
          step="0.1"
          value={value.repeatDelayMs / 1000}
          onchange={(e) => {
            const ms = seconds(e);
            if (ms !== null) {
              set({ repeatDelayMs: ms });
            }
          }}
        />
        <span class="unit">s</span>
      </div>
    </div>
  {/if}

  <div class="field">
    <label for="{idPrefix}-jitter">Random delay variation</label>
    <div class="inline">
      <input
        id="{idPrefix}-jitter"
        type="number"
        class="short"
        min="0"
        max="50"
        step="1"
        value={value.jitterPercent}
        onchange={(e) => {
          const n = number(e);
          if (n !== null) {
            set({ jitterPercent: Math.round(Math.min(50, Math.max(0, n))) });
          }
        }}
      />
      <span class="unit">±%</span>
    </div>
  </div>

  <div class="field">
    <label for="{idPrefix}-positions">Positions</label>
    <select
      id="{idPrefix}-positions"
      value={value.positioning}
      onchange={(e) => set({ positioning: (e.currentTarget as HTMLSelectElement).value as Positioning })}
    >
      <option value="absolute">Where they were recorded</option>
      <option value="cursor">Relative to the pointer at the start</option>
      <option value="window">Relative to the active window</option>
    </select>
    <label class="check">
      <input
        type="checkbox"
        checked={value.scaleToScreen}
        onchange={(e) => set({ scaleToScreen: (e.currentTarget as HTMLInputElement).checked })}
      />
      {value.positioning === "window" ? "Scale with the window's size" : "Adapt to changed screens"}
    </label>
  </div>

  <div class="field">
    <label class="check">
      <input
        type="checkbox"
        checked={value.stopAfterMs !== null}
        onchange={(e) => set({ stopAfterMs: (e.currentTarget as HTMLInputElement).checked ? 60_000 : null })}
      />
      Stop after
    </label>
    {#if value.stopAfterMs !== null}
      <div class="inline">
        <input
          type="number"
          class="short"
          min="0.001"
          max="604800"
          step="1"
          aria-label="Stop after, in seconds"
          value={value.stopAfterMs / 1000}
          onchange={(e) => {
            const ms = seconds(e);
            if (ms !== null && ms > 0) {
              set({ stopAfterMs: ms });
            }
          }}
        />
        <span class="unit">s</span>
      </div>
    {/if}
  </div>

  {#if actions !== null && actions > 0}
    <div class="field wide">
      <label class="check">
        <input
          type="checkbox"
          checked={value.section !== null}
          onchange={(e) =>
            set({ section: (e.currentTarget as HTMLInputElement).checked ? { first: 0, last: actions - 1 } : null })}
        />
        Repeat only part of the macro
      </label>
      {#if value.section !== null}
        {@const sec = value.section}
        <div class="inline">
          <span class="unit">actions</span>
          <input
            type="number"
            class="short"
            min="1"
            max={actions}
            aria-label="First repeated action"
            value={sec.first + 1}
            onchange={(e) => {
              const n = number(e);
              if (n !== null) {
                set({ section: { first: Math.min(Math.max(0, Math.round(n) - 1), sec.last), last: sec.last } });
              }
            }}
          />
          <span class="unit">to</span>
          <input
            type="number"
            class="short"
            min="1"
            max={actions}
            aria-label="Last repeated action"
            value={sec.last + 1}
            onchange={(e) => {
              const n = number(e);
              if (n !== null) {
                set({ section: { first: sec.first, last: Math.max(Math.min(actions - 1, Math.round(n) - 1), sec.first) } });
              }
            }}
          />
        </div>
        <span class="hint">The actions before play once at the start, the ones after once at the end.</span>
      {/if}
    </div>
  {/if}
</fieldset>

<style>
  .playback {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: var(--space-4) var(--space-5);
  }

  .wide {
    grid-column: 1 / -1;
  }

  .inline {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .short {
    width: 6.5rem;
  }

  .unit {
    color: var(--text-muted);
    font-weight: var(--weight-regular);
  }

  .presets {
    display: inline-flex;
    gap: var(--space-1);
  }
</style>
