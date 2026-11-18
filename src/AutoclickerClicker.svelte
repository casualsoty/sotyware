<script lang="ts">
  import {
    captureCursorPosition,
    saveProfile,
    type ClickerConfig,
    type MouseButton,
    type Point,
    type RandomMode,
    type TargetMode,
  } from "./automation";
  import {
    clickerProblem,
    cpsToInterval,
    formatRate,
    intervalToCps,
    numberOf,
  } from "./automationFormat";
  import type { AutomationModel } from "./automationModel.svelte";
  import Icon from "./Icon.svelte";

  // The Clicker tab (ADR-050): the active profile's autoclicker, simple or
  // advanced. Every change is checked here first and saved to the profile
  // a moment later; Rust checks it again and runs it.

  interface Props {
    model: AutomationModel;
    /** Automation can start (feature on, nothing else runs). */
    canStart: boolean;
    onstart: () => void;
    onstop: () => void;
  }

  let { model, canStart, onstart, onstop }: Props = $props();

  const profile = $derived(model.activeProfile);
  const clicking = $derived(model.status?.session?.kind === "clicker");
  const countdown = $derived(model.config?.settings.dashboardCountdownMs ?? 0);

  let draft = $state<ClickerConfig | null>(null);
  let draftFor = $state<string | null>(null);
  let saveState = $state<"idle" | "saving" | "saved" | "error">("idle");
  let saveError = $state<string | null>(null);
  let picking = $state<string | null>(null);
  let pickLeft = $state(0);
  let pickNote = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    const p = profile;
    if (p !== null && p.id !== draftFor) {
      draft = $state.snapshot(p.clicker) as ClickerConfig;
      draftFor = p.id;
      saveState = "idle";
      saveError = null;
    }
  });

  const problem = $derived(draft === null ? null : clickerProblem(draft));

  function changed(): void {
    if (timer !== null) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => void save(), 400);
  }

  async function save(): Promise<void> {
    timer = null;
    const p = profile;
    if (p === null || draft === null || clickerProblem(draft) !== null) {
      return;
    }
    saveState = "saving";
    try {
      const saved = await saveProfile({
        ...($state.snapshot(p) as typeof p),
        clicker: $state.snapshot(draft) as ClickerConfig,
      });
      model.putProfile(saved);
      saveState = "saved";
      saveError = null;
    } catch (e) {
      saveState = "error";
      saveError = e instanceof Error ? e.message : String(e);
    }
  }

  function edit(f: (c: ClickerConfig) => void): void {
    if (draft !== null) {
      f(draft);
      changed();
    }
  }

  function num(e: Event): number | null {
    return numberOf(e.currentTarget as HTMLInputElement);
  }

  const rateValue = $derived(
    draft === null
      ? 0
      : draft.rateUnit === "cps"
        ? Number(formatRate(intervalToCps(draft.intervalMs)))
        : Math.round(draft.intervalMs * 1000) / 1000,
  );

  function setRate(e: Event): void {
    const v = num(e);
    if (v === null || v <= 0) {
      return;
    }
    edit((c) => {
      c.intervalMs = c.rateUnit === "cps" ? cpsToInterval(v) : v;
    });
  }

  function setTargetMode(mode: TargetMode, fixed = false): void {
    edit((c) => {
      c.target.mode = mode;
      if (mode === "points" && c.target.points.length === 0) {
        c.target.points = [{ x: 0, y: 0 }];
      }
      if (mode === "points" && fixed) {
        c.target.points = c.target.points.slice(0, 1);
      }
      if (mode === "area" && c.target.area === null) {
        c.target.area = { x: 0, y: 0, width: 200, height: 200 };
      }
    });
  }

  /** Reads the pointer after a countdown, so it can be moved first. */
  async function pick(what: string, use: (p: Point) => void): Promise<void> {
    if (picking !== null) {
      return;
    }
    picking = what;
    pickNote = null;
    pickLeft = 3;
    const tick = setInterval(() => {
      pickLeft = Math.max(0, pickLeft - 1);
    }, 1000);
    try {
      const got = await captureCursorPosition(3000);
      if (got.onSotyware) {
        pickNote = "The pointer was on sotyware's own window; move it where the clicks should go during the countdown.";
      } else {
        use(got.point);
      }
    } catch (e) {
      pickNote = e instanceof Error ? e.message : String(e);
    } finally {
      clearInterval(tick);
      picking = null;
    }
  }

  const fixedMode = $derived(
    draft === null
      ? "cursor"
      : draft.target.mode === "cursor"
        ? "cursor"
        : draft.target.mode === "points" && draft.target.points.length <= 1
          ? "fixed"
          : draft.target.mode === "points"
            ? "list"
            : "area",
  );
</script>

{#if draft === null || profile === null}
  <p class="muted">Loading…</p>
{:else}
  {@const c = draft}
  <section class="clicker" aria-labelledby="clicker-heading">
    <div class="head">
      <div>
        <h2 id="clicker-heading">Autoclicker</h2>
        <p class="muted small-text">Profile “{profile.name}”. Changes are saved as you make them.</p>
      </div>
      <div class="view-switch" role="group" aria-label="Settings shown">
        <button type="button" class="small" aria-pressed={!c.advanced} onclick={() => edit((x) => (x.advanced = false))}>Simple</button>
        <button type="button" class="small" aria-pressed={c.advanced} onclick={() => edit((x) => (x.advanced = true))}>Advanced</button>
      </div>
    </div>

    <div class="grid">
      <fieldset class="field">
        <legend>Mouse button</legend>
        <div class="segmented" role="radiogroup" aria-label="Mouse button">
          {#each (c.advanced ? ["left", "right", "middle", "back", "forward"] : ["left", "right", "middle"]) as MouseButton[] as b (b)}
            <label class="seg" class:on={c.button === b}>
              <input type="radio" name="clicker-button" value={b} checked={c.button === b} onchange={() => edit((x) => (x.button = b))} />
              {b[0]?.toUpperCase()}{b.slice(1)}
            </label>
          {/each}
        </div>
      </fieldset>

      <fieldset class="field">
        <legend>Click</legend>
        <div class="segmented" role="radiogroup" aria-label="Click type">
          <label class="seg" class:on={c.clickType === "single"}>
            <input type="radio" name="clicker-type" checked={c.clickType === "single"} onchange={() => edit((x) => (x.clickType = "single"))} />Single
          </label>
          <label class="seg" class:on={c.clickType === "double"}>
            <input type="radio" name="clicker-type" checked={c.clickType === "double"} onchange={() => edit((x) => (x.clickType = "double"))} />Double
          </label>
        </div>
      </fieldset>

      <div class="field">
        <label for="clicker-rate">Speed</label>
        <div class="inline">
          <input id="clicker-rate" type="number" class="short" min="0.001" step="any" value={rateValue} onchange={setRate} />
          <select
            aria-label="Speed unit"
            value={c.rateUnit}
            onchange={(e) => edit((x) => (x.rateUnit = (e.currentTarget as HTMLSelectElement).value as "cps" | "interval"))}
          >
            <option value="cps">clicks per second</option>
            <option value="interval">ms between clicks</option>
          </select>
        </div>
        <p class="hint">
          {c.rateUnit === "cps"
            ? `One click every ${Math.round(c.intervalMs * 100) / 100} ms.`
            : `${formatRate(intervalToCps(c.intervalMs))} clicks per second.`}
          {intervalToCps(c.intervalMs) > 100 ? " Many programs miss clicks this fast." : ""}
        </p>
      </div>

      <fieldset class="field">
        <legend>Repeat</legend>
        <label class="check">
          <input type="radio" name="clicker-repeat" checked={c.maxClicks === null} onchange={() => edit((x) => (x.maxClicks = null))} />
          Until stopped
        </label>
        <label class="check">
          <input type="radio" name="clicker-repeat" checked={c.maxClicks !== null} onchange={() => edit((x) => (x.maxClicks = x.maxClicks ?? 100))} />
          <span>Stop after</span>
          <input
            type="number"
            class="short"
            min="1"
            max="100000000"
            aria-label="Number of clicks"
            disabled={c.maxClicks === null}
            value={c.maxClicks ?? 100}
            onchange={(e) => {
              const n = num(e);
              if (n !== null && n >= 1) {
                edit((x) => (x.maxClicks = Math.round(n)));
              }
            }}
          />
          <span>clicks</span>
        </label>
      </fieldset>

      <fieldset class="field span">
        <legend>Where</legend>
        <div class="where">
          <label class="check">
            <input type="radio" name="clicker-where" checked={fixedMode === "cursor"} onchange={() => setTargetMode("cursor")} />
            Where the pointer is
          </label>
          <label class="check">
            <input type="radio" name="clicker-where" checked={fixedMode === "fixed"} onchange={() => setTargetMode("points", true)} />
            At a fixed position
          </label>
          {#if c.advanced}
            <label class="check">
              <input type="radio" name="clicker-where" checked={fixedMode === "list"} onchange={() => {
                setTargetMode("points");
                edit((x) => {
                  if (x.target.points.length < 2) {
                    x.target.points = [...x.target.points, { x: 0, y: 0 }];
                  }
                });
              }} />
              At several positions in turn
            </label>
            <label class="check">
              <input type="radio" name="clicker-where" checked={fixedMode === "area"} onchange={() => setTargetMode("area")} />
              At random points in an area
            </label>
          {/if}
        </div>

        {#if c.target.mode === "points"}
          <ol class="points">
            {#each c.target.points as p, i (i)}
              <li>
                <p class="muted">{c.target.points.length > 1 ? `${i + 1}.` : "Position"}</p>
                <label class="xy">X <input type="number" class="coord" aria-label={c.target.points.length > 1 ? `X of position ${i + 1}` : "X"} value={p.x} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => { const q = x.target.points[i]; if (q) { q.x = Math.round(n); } }); } }} /></label>
                <label class="xy">Y <input type="number" class="coord" aria-label={c.target.points.length > 1 ? `Y of position ${i + 1}` : "Y"} value={p.y} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => { const q = x.target.points[i]; if (q) { q.y = Math.round(n); } }); } }} /></label>
                <button type="button" class="small" disabled={picking !== null} onclick={() => void pick(`point-${i}`, (pt) => edit((x) => { x.target.points[i] = pt; }))}>
                  <Icon name="target" size={14} />{picking === `point-${i}` ? `Point… ${pickLeft}` : "Pick…"}
                </button>
                {#if c.target.points.length > 1}
                  <button type="button" class="ghost icon small" aria-label="Move position {i + 1} up" disabled={i === 0} onclick={() => edit((x) => { const [m] = x.target.points.splice(i, 1); if (m) { x.target.points.splice(i - 1, 0, m); } })}><Icon name="arrow-up" size={14} /></button>
                  <button type="button" class="ghost icon small" aria-label="Remove position {i + 1}" onclick={() => edit((x) => { x.target.points.splice(i, 1); })}><Icon name="trash" size={14} /></button>
                {/if}
              </li>
            {/each}
          </ol>
          {#if c.advanced && fixedMode === "list"}
            <div class="actions">
              <button type="button" class="small" disabled={c.target.points.length >= 100} onclick={() => edit((x) => { x.target.points = [...x.target.points, { x: 0, y: 0 }]; })}><Icon name="plus" size={14} />Add position</button>
              <label class="check">
                <input type="checkbox" checked={c.target.order === "random"} onchange={(e) => edit((x) => (x.target.order = (e.currentTarget as HTMLInputElement).checked ? "random" : "sequential"))} />
                In random order
              </label>
            </div>
          {/if}
        {:else if c.target.mode === "area" && c.target.area !== null}
          {@const a = c.target.area}
          <div class="area">
            <label class="xy">X <input type="number" class="coord" aria-label="X" value={a.x} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => { if (x.target.area) { x.target.area.x = Math.round(n); } }); } }} /></label>
            <label class="xy">Y <input type="number" class="coord" aria-label="Y" value={a.y} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => { if (x.target.area) { x.target.area.y = Math.round(n); } }); } }} /></label>
            <label class="xy">Width <input type="number" class="coord" min="1" aria-label="Width" value={a.width} onchange={(e) => { const n = num(e); if (n !== null && n >= 1) { edit((x) => { if (x.target.area) { x.target.area.width = Math.round(n); } }); } }} /></label>
            <label class="xy">Height <input type="number" class="coord" min="1" aria-label="Height" value={a.height} onchange={(e) => { const n = num(e); if (n !== null && n >= 1) { edit((x) => { if (x.target.area) { x.target.area.height = Math.round(n); } }); } }} /></label>
            <button type="button" class="small" disabled={picking !== null} onclick={() => void pick("corner-1", (pt) => edit((x) => { if (x.target.area) { const r = x.target.area.x + x.target.area.width; const b = x.target.area.y + x.target.area.height; x.target.area = { x: pt.x, y: pt.y, width: Math.max(1, r - pt.x), height: Math.max(1, b - pt.y) }; } }))}>
              <Icon name="target" size={14} />{picking === "corner-1" ? `Point… ${pickLeft}` : "Pick top-left…"}
            </button>
            <button type="button" class="small" disabled={picking !== null} onclick={() => void pick("corner-2", (pt) => edit((x) => { if (x.target.area) { x.target.area.width = Math.max(1, pt.x - x.target.area.x + 1); x.target.area.height = Math.max(1, pt.y - x.target.area.y + 1); } }))}>
              <Icon name="target" size={14} />{picking === "corner-2" ? `Point… ${pickLeft}` : "Pick bottom-right…"}
            </button>
          </div>
        {/if}
        {#if picking !== null}
          <p class="hint" role="status">Move the pointer to the spot; it is read in {pickLeft} s.</p>
        {/if}
        {#if pickNote !== null}
          <p class="callout warning" role="alert">{pickNote}</p>
        {/if}
      </fieldset>

      {#if c.advanced}
        <fieldset class="field">
          <legend>Random interval</legend>
          <select
            aria-label="Random interval"
            value={c.random.mode}
            onchange={(e) => edit((x) => (x.random.mode = (e.currentTarget as HTMLSelectElement).value as RandomMode))}
          >
            <option value="off">Off: always the same interval</option>
            <option value="range">Between two values</option>
            <option value="variation">Vary by a percentage</option>
          </select>
          {#if c.random.mode === "range"}
            <div class="inline">
              <input type="number" class="short" min="1" aria-label="Shortest interval in ms" value={c.random.minMs} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => (x.random.minMs = n)); } }} />
              <p class="unit">to</p>
              <input type="number" class="short" min="1" aria-label="Longest interval in ms" value={c.random.maxMs} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => (x.random.maxMs = n)); } }} />
              <p class="unit">ms</p>
            </div>
          {:else if c.random.mode === "variation"}
            <div class="inline">
              <p class="unit">±</p>
              <input type="number" class="short" min="1" max="90" aria-label="Variation in percent" value={c.random.variationPercent} onchange={(e) => { const n = num(e); if (n !== null) { edit((x) => (x.random.variationPercent = Math.round(n))); } }} />
              <p class="unit">%</p>
            </div>
          {/if}
        </fieldset>

        <div class="field">
          <label class="check">
            <input type="checkbox" checked={c.maxDurationMs !== null} onchange={(e) => edit((x) => (x.maxDurationMs = (e.currentTarget as HTMLInputElement).checked ? 60_000 : null))} />
            Stop after a time
          </label>
          {#if c.maxDurationMs !== null}
            <div class="inline">
              <input type="number" class="short" min="0.001" step="any" aria-label="Time limit in seconds" value={c.maxDurationMs / 1000} onchange={(e) => { const n = num(e); if (n !== null && n > 0) { edit((x) => (x.maxDurationMs = Math.round(n * 1000))); } }} />
              <p class="unit">s</p>
            </div>
          {/if}
        </div>

        <div class="field">
          <label for="clicker-delay">Start delay</label>
          <div class="inline">
            <input id="clicker-delay" type="number" class="short" min="0" max="86400" step="0.1" value={c.startDelayMs / 1000} onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { edit((x) => (x.startDelayMs = Math.round(n * 1000))); } }} />
            <p class="unit">s</p>
          </div>
        </div>

        <div class="field">
          <label class="check">
            <input type="checkbox" checked={c.batch !== null} onchange={(e) => edit((x) => (x.batch = (e.currentTarget as HTMLInputElement).checked ? { clicks: 10, pauseMs: 1000 } : null))} />
            Pause between batches
          </label>
          {#if c.batch !== null}
            {@const b = c.batch}
            <div class="inline">
              <p class="unit">every</p>
              <input type="number" class="short" min="1" aria-label="Clicks per batch" value={b.clicks} onchange={(e) => { const n = num(e); if (n !== null && n >= 1) { edit((x) => { if (x.batch) { x.batch.clicks = Math.round(n); } }); } }} />
              <p class="unit">clicks, for</p>
              <input type="number" class="short" min="0" aria-label="Pause in ms" value={b.pauseMs} onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { edit((x) => { if (x.batch) { x.batch.pauseMs = Math.round(n); } }); } }} />
              <p class="unit">ms</p>
            </div>
          {/if}
        </div>

        <div class="field">
          <label for="clicker-hold">Hold each click</label>
          <div class="inline">
            <input id="clicker-hold" type="number" class="short" min="0" max="10000" value={c.holdMs} onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { edit((x) => (x.holdMs = Math.round(n))); } }} />
            <p class="unit">ms</p>
          </div>
        </div>

        {#if c.clickType === "double"}
          <div class="field">
            <label for="clicker-gap">Between the two clicks</label>
            <div class="inline">
              <input id="clicker-gap" type="number" class="short" min="0" max="10000" value={c.doubleGapMs} onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { edit((x) => (x.doubleGapMs = Math.round(n))); } }} />
              <p class="unit">ms</p>
            </div>
          </div>
        {/if}

        {#if c.target.mode !== "cursor"}
          <div class="field">
            <label for="clicker-spread">Spread around each position</label>
            <div class="inline">
              <input id="clicker-spread" type="number" class="short" min="0" max="10000" value={c.target.spread} onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { edit((x) => (x.target.spread = Math.round(n))); } }} />
              <p class="unit">px</p>
            </div>
          </div>
        {/if}
      {/if}
    </div>

    {#if problem !== null}
      <p class="callout warning" role="alert">{problem} Not saved yet.</p>
    {:else if saveError !== null}
      <p class="callout danger" role="alert">Not saved: {saveError}</p>
    {/if}

    <div class="run">
      {#if clicking}
        <button type="button" class="primary danger" onclick={onstop}><Icon name="stop" />Stop</button>
      {:else}
        <button type="button" class="primary" disabled={!canStart || problem !== null} onclick={onstart}><Icon name="play" />Start autoclicker</button>
      {/if}
      <p class="hint" aria-live="polite">
        {#if saveState === "saving"}Saving…{:else if saveState === "saved"}Saved.{/if}
        {countdown > 0 ? `Starts after ${countdown / 1000} s, so you can point where it should click. ` : ""}It never clicks sotyware's own window.
      </p>
    </div>
  </section>
{/if}

<style>
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .head p {
    margin: var(--space-1) 0 0;
  }

  .view-switch {
    display: inline-flex;
    gap: var(--space-1);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(17rem, 1fr));
    gap: var(--space-5) var(--space-6);
  }

  .span {
    grid-column: 1 / -1;
  }

  fieldset.field legend {
    margin-bottom: var(--space-1);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }

  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    align-self: flex-start;
    gap: 2px;
    padding: 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .seg {
    display: inline-flex;
    align-items: center;
    min-height: 28px;
    padding: 0 var(--space-3);
    font-size: var(--text-sm);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .seg input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  /* The chosen one looks like a pressed button (base.css). */
  .seg.on {
    color: var(--accent-text);
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent-soft-strong);
    font-weight: var(--weight-medium);
  }

  .seg:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }

  .inline,
  .where,
  .area {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
  }

  .where {
    margin-bottom: var(--space-2);
  }

  .short {
    width: 7rem;
  }

  .coord {
    width: 6rem;
  }

  .unit {
    margin: 0;
    color: var(--text-muted);
    font-weight: var(--weight-regular);
  }

  /* Text in a flex box is a <p>: WebKitGTK leaves a <span> flex item's
     text out of the accessibility tree (frontend.md). No margin, as the
     <span> had none. */
  .field > p.hint,
  .points p {
    margin: 0;
  }

  .xy {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-weight: var(--weight-regular);
    color: var(--text-muted);
  }

  .points {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .points li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .run {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-6);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border);
  }

  .run .hint {
    margin: 0;
    flex: 1 1 18rem;
  }
</style>
