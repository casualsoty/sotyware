<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    captureActiveWindow,
    getMacro,
    saveMacro,
    type Action,
    type ActionType,
    type Macro,
    type Mismatch,
    type MouseButton,
    type Press,
  } from "./automation";
  import {
    ACTION_TYPE_LABELS,
    BUTTON_LABELS,
    actionTimes,
    describeAction,
    formatDuration,
    newAction,
    numberOf,
    positionBefore,
    totalDuration,
  } from "./automationFormat";
  import type { AutomationModel } from "./automationModel.svelte";
  import HotkeyInput from "./HotkeyInput.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import PlaybackForm from "./PlaybackForm.svelte";

  // The macro editor (ADR-050): a recorded or written macro's details,
  // shortcut, target program, playback settings and actions. Everything is
  // edited in this copy; Save sends the whole macro, which Rust checks
  // again. Preview moves along the timeline only: it sends no input.

  interface Props {
    model: AutomationModel;
    id: string;
    canPlay: boolean;
    onclose: () => void;
    onplay: (id: string) => void;
  }

  let { model, id, canPlay, onclose, onplay }: Props = $props();

  const PAGE = 100;
  const keys = $derived(model.config?.keys ?? []);

  let saved = $state<Macro | null>(null);
  let m = $state<Macro | null>(null);
  let loadError = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let warnings = $state<string[]>([]);
  let saving = $state(false);
  let notice = $state<string | null>(null);
  let page = $state(0);
  let selected = $state<number[]>([]);
  let addType = $state<ActionType>("click");
  let confirmLeave = $state(false);
  let picking = $state(false);
  let pickLeft = $state(0);
  let bulkDelay = $state(100);
  let bulkFactor = $state(0.5);

  // Preview: a playhead over the timeline, no input sent.
  let previewing = $state(false);
  let playhead = $state(0);
  let frame: number | null = null;

  async function load(): Promise<void> {
    try {
      const got = await getMacro(id);
      saved = got;
      m = structuredClone(got);
      loadError = null;
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    void id;
    void load();
  });

  onDestroy(() => stopPreview());

  const dirty = $derived(m !== null && saved !== null && JSON.stringify(m) !== JSON.stringify(saved));
  const times = $derived(m === null ? [] : actionTimes(m.actions));
  const total = $derived(m === null ? 0 : totalDuration(m.actions));
  const pages = $derived(m === null ? 1 : Math.max(1, Math.ceil(m.actions.length / PAGE)));
  const current = $derived.by(() => {
    if (!previewing) {
      return -1;
    }
    let found = -1;
    for (let i = 0; i < times.length && (times[i] ?? 0) <= playhead; i++) {
      found = i;
    }
    return found;
  });

  /** At most ~1,500 marks on the timeline: a long macro is drawn in
   * buckets, a click or key winning over a movement. */
  const marks = $derived.by(() => {
    if (m === null || m.actions.length === 0) {
      return [] as { left: number; type: ActionType; index: number }[];
    }
    const span = Math.max(1, total);
    const rank: Record<ActionType, number> = { button: 4, click: 4, key: 4, keys: 4, pause: 5, scroll: 3, wait: 2, move: 1 };
    const out: { left: number; type: ActionType; index: number }[] = [];
    const slots = new Map<number, number>();
    m.actions.forEach((a, i) => {
      const left = ((times[i] ?? 0) / span) * 100;
      const slot = Math.round(left * 15);
      const prev = slots.get(slot);
      const pa = prev === undefined ? undefined : out[prev];
      if (pa === undefined) {
        slots.set(slot, out.length);
        out.push({ left, type: a.type, index: i });
      } else if (rank[a.type] > rank[pa.type]) {
        out[prev ?? 0] = { left, type: a.type, index: i };
      }
    });
    return out;
  });

  function changed(): void {
    saveError = null;
    notice = null;
  }

  async function save(): Promise<void> {
    if (m === null) {
      return;
    }
    saving = true;
    try {
      const r = await saveMacro($state.snapshot(m) as Macro);
      model.putMacro(r.summary);
      warnings = r.warnings;
      const fresh = await getMacro(id);
      saved = fresh;
      m = structuredClone(fresh);
      saveError = null;
      notice = "Saved.";
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  function revert(): void {
    if (saved !== null) {
      m = structuredClone($state.snapshot(saved) as Macro);
      selected = [];
      changed();
    }
  }

  function leave(): void {
    if (dirty) {
      confirmLeave = true;
    } else {
      onclose();
    }
  }

  function setAction(i: number, a: Action): void {
    if (m !== null) {
      m.actions[i] = a;
      changed();
    }
  }

  function insert(): void {
    if (m === null) {
      return;
    }
    const after = selected.length > 0 ? Math.max(...selected) + 1 : m.actions.length;
    const a = newAction(addType, positionBefore(m.actions, after));
    m.actions.splice(after, 0, a);
    selected = [after];
    page = Math.floor(after / PAGE);
    changed();
  }

  function move(i: number, by: number): void {
    if (m === null) {
      return;
    }
    const j = i + by;
    if (j < 0 || j >= m.actions.length) {
      return;
    }
    const [a] = m.actions.splice(i, 1);
    if (a) {
      m.actions.splice(j, 0, a);
    }
    selected = [];
    changed();
  }

  function duplicate(indices: number[]): void {
    if (m === null || indices.length === 0) {
      return;
    }
    const sorted = [...indices].sort((a, b) => a - b);
    const copies = sorted.map((i) => structuredClone($state.snapshot(m?.actions[i]) as Action));
    const at = (sorted.at(-1) ?? 0) + 1;
    m.actions.splice(at, 0, ...copies);
    selected = copies.map((_, k) => at + k);
    changed();
  }

  function remove(indices: number[]): void {
    if (m === null) {
      return;
    }
    const drop = new Set(indices);
    m.actions = m.actions.filter((_, i) => !drop.has(i));
    if (m.playback.section !== null && m.playback.section.last >= m.actions.length) {
      m.playback.section = null;
    }
    selected = [];
    page = Math.min(page, Math.max(0, Math.ceil(m.actions.length / PAGE) - 1));
    changed();
  }

  function removeMoves(): void {
    if (m === null) {
      return;
    }
    const keep: Action[] = [];
    let carry = 0;
    for (const a of m.actions) {
      if (a.type === "move") {
        carry += a.delayMs;
      } else {
        keep.push({ ...a, delayMs: a.delayMs + carry });
        carry = 0;
      }
    }
    const removed = m.actions.length - keep.length;
    m.actions = keep;
    m.playback.section = null;
    selected = [];
    page = 0;
    changed();
    notice = `${removed.toLocaleString("en-US")} pointer movements removed; their time was kept.`;
  }

  function setDelays(indices: number[], f: (d: number) => number): void {
    if (m === null) {
      return;
    }
    for (const i of indices) {
      const a = m.actions[i];
      if (a) {
        a.delayMs = Math.max(0, Math.min(86_400_000, Math.round(f(a.delayMs))));
      }
    }
    changed();
  }

  function toggle(i: number, on: boolean): void {
    selected = on ? [...selected, i] : selected.filter((x) => x !== i);
  }

  function preview(): void {
    if (m === null || m.actions.length === 0) {
      return;
    }
    stopPreview();
    previewing = true;
    playhead = 0;
    const speed = m.playback.speed;
    const startedAt = performance.now();
    const step = (now: number): void => {
      playhead = (now - startedAt) * speed;
      if (playhead >= total) {
        playhead = total;
        frame = null;
        previewing = false;
        return;
      }
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
  }

  function stopPreview(): void {
    if (frame !== null) {
      cancelAnimationFrame(frame);
      frame = null;
    }
    previewing = false;
  }

  async function pickWindow(): Promise<void> {
    if (m === null || picking) {
      return;
    }
    picking = true;
    pickLeft = 3;
    const tick = setInterval(() => (pickLeft = Math.max(0, pickLeft - 1)), 1000);
    try {
      const w = await captureActiveWindow(3000);
      if (m !== null) {
        m.target = { process: w.process, title: null, onMismatch: m.target?.onMismatch ?? "warn" };
        changed();
      }
    } catch (e) {
      saveError = e instanceof Error ? e.message : String(e);
    } finally {
      clearInterval(tick);
      picking = false;
    }
  }

  function num(e: Event): number | null {
    return numberOf(e.currentTarget as HTMLInputElement);
  }
</script>

{#snippet details(a: Action, i: number)}
  {#if a.type === "move"}
    <span class="xy">
      <input type="number" class="coord" aria-label="Action {i + 1}: X" value={a.x} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, x: Math.round(n) }); } }} />
      <input type="number" class="coord" aria-label="Action {i + 1}: Y" value={a.y} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, y: Math.round(n) }); } }} />
    </span>
  {:else if a.type === "button"}
    <span class="xy">
      <select aria-label="Action {i + 1}: button" value={a.button} onchange={(e) => setAction(i, { ...a, button: (e.currentTarget as HTMLSelectElement).value as MouseButton })}>
        {#each Object.entries(BUTTON_LABELS) as [b, label] (b)}<option value={b}>{label}</option>{/each}
      </select>
      <select aria-label="Action {i + 1}: press or release" value={a.state} onchange={(e) => setAction(i, { ...a, state: (e.currentTarget as HTMLSelectElement).value as Press })}>
        <option value="down">down</option>
        <option value="up">up</option>
      </select>
      <input type="number" class="coord" aria-label="Action {i + 1}: X" value={a.x} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, x: Math.round(n) }); } }} />
      <input type="number" class="coord" aria-label="Action {i + 1}: Y" value={a.y} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, y: Math.round(n) }); } }} />
    </span>
  {:else if a.type === "click"}
    <span class="xy">
      <select aria-label="Action {i + 1}: button" value={a.button} onchange={(e) => setAction(i, { ...a, button: (e.currentTarget as HTMLSelectElement).value as MouseButton })}>
        {#each Object.entries(BUTTON_LABELS) as [b, label] (b)}<option value={b}>{label}</option>{/each}
      </select>
      <select aria-label="Action {i + 1}: single or double" value={a.count} onchange={(e) => setAction(i, { ...a, count: Number((e.currentTarget as HTMLSelectElement).value) })}>
        <option value={1}>click</option>
        <option value={2}>double-click</option>
      </select>
      <label class="check small-text">
        <input type="checkbox" checked={a.x === null} onchange={(e) => { const here = (e.currentTarget as HTMLInputElement).checked; setAction(i, here ? { ...a, x: null, y: null } : { ...a, ...positionBefore(m?.actions ?? [], i) }); }} />
        where the pointer is
      </label>
      {#if a.x !== null && a.y !== null}
        <input type="number" class="coord" aria-label="Action {i + 1}: X" value={a.x} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, x: Math.round(n) }); } }} />
        <input type="number" class="coord" aria-label="Action {i + 1}: Y" value={a.y} onchange={(e) => { const n = num(e); if (n !== null) { setAction(i, { ...a, y: Math.round(n) }); } }} />
      {/if}
    </span>
  {:else if a.type === "scroll"}
    {@const dir = a.dy > 0 ? "up" : a.dy < 0 ? "down" : a.dx > 0 ? "right" : "left"}
    {@const notches = Math.max(1, Math.round(Math.abs(a.dy !== 0 ? a.dy : a.dx) / 120))}
    <span class="xy">
      <select aria-label="Action {i + 1}: direction" value={dir} onchange={(e) => { const d = (e.currentTarget as HTMLSelectElement).value; const amount = notches * 120; setAction(i, { ...a, dx: d === "right" ? amount : d === "left" ? -amount : 0, dy: d === "up" ? amount : d === "down" ? -amount : 0 }); }}>
        <option value="up">up</option>
        <option value="down">down</option>
        <option value="left">left</option>
        <option value="right">right</option>
      </select>
      <input type="number" class="coord" min="1" max="100" aria-label="Action {i + 1}: notches" value={notches} onchange={(e) => { const n = num(e); if (n !== null && n >= 1) { const amount = Math.round(Math.min(100, n)) * 120; setAction(i, { ...a, dx: dir === "right" ? amount : dir === "left" ? -amount : 0, dy: dir === "up" ? amount : dir === "down" ? -amount : 0 }); } }} />
      <span class="muted small-text">notches</span>
    </span>
  {:else if a.type === "key"}
    <span class="xy">
      <select aria-label="Action {i + 1}: key" value={a.key} onchange={(e) => setAction(i, { ...a, key: (e.currentTarget as HTMLSelectElement).value })}>
        {#each keys as k (k.code)}<option value={k.code}>{k.label}</option>{/each}
      </select>
      <select aria-label="Action {i + 1}: press or release" value={a.state} onchange={(e) => setAction(i, { ...a, state: (e.currentTarget as HTMLSelectElement).value as Press })}>
        <option value="down">down</option>
        <option value="up">up</option>
      </select>
    </span>
  {:else if a.type === "keys"}
    <HotkeyInput value={a.chord} {keys} label="Action {i + 1}: shortcut" onchange={(c) => { if (c !== null) { setAction(i, { ...a, chord: c }); } }} />
  {:else if a.type === "wait"}
    <span class="muted small-text">Waits {formatDuration(a.delayMs)}</span>
  {:else}
    <span class="muted small-text">Playback pauses here until you resume it.</span>
  {/if}
{/snippet}

<section class="editor" aria-labelledby="editor-heading">
  <div class="editor-head">
    <button type="button" class="ghost" onclick={leave}><Icon name="arrow-left" />Back to macros</button>
    <h2 id="editor-heading" tabindex="-1">{m?.name ?? "Macro"}</h2>
    <span class="spacer"></span>
    {#if dirty}<span class="badge warning">Unsaved changes</span>{/if}
    <button type="button" disabled={!dirty || saving} onclick={revert}>Revert</button>
    <button type="button" class="primary" disabled={!dirty || saving || m === null} onclick={() => void save()}>{saving ? "Saving…" : "Save"}</button>
    <button type="button" disabled={!canPlay || dirty || m === null || m.actions.length === 0} title={dirty ? "Save first" : undefined} onclick={() => onplay(id)}><Icon name="play" />Play</button>
  </div>

  {#if loadError !== null}
    <p class="callout danger" role="alert">The macro could not be opened: {loadError}</p>
  {:else if m === null}
    <p class="muted">Loading…</p>
  {:else}
    {#if saveError !== null}
      <p class="callout danger" role="alert">{saveError}</p>
    {/if}
    {#each warnings as w (w)}
      <p class="callout warning">{w}</p>
    {/each}
    {#if notice !== null}
      <p class="callout success" role="status">{notice}</p>
    {/if}

    <div class="card">
      <h3 class="section-title">Details</h3>
      <div class="form-grid">
        <label class="field">Name <input type="text" aria-label="Name" maxlength="120" bind:value={m.name} oninput={changed} /></label>
        <label class="field">Folder <input type="text" aria-label="Folder" maxlength="80" bind:value={m.folder} oninput={changed} placeholder="No folder" /></label>
        <label class="field wide">Description <textarea rows="2" aria-label="Description" maxlength="2000" bind:value={m.description} oninput={changed}></textarea></label>
        <div class="field">
          <span id="editor-shortcut-label">Shortcut that plays it</span>
          <div class="inline">
            <HotkeyInput
              value={m.shortcut.chord}
              {keys}
              label="Shortcut that plays {m.name}"
              onchange={(c) => {
                if (m !== null) {
                  m.shortcut = { chord: c, enabled: c !== null };
                  changed();
                }
              }}
            />
            {#if m.shortcut.chord !== null}
              <label class="check"><input type="checkbox" bind:checked={m.shortcut.enabled} onchange={changed} />On</label>
            {/if}
          </div>
          <span class="hint">Pressed again while the macro plays, it stops it.</span>
        </div>
        <label class="check"><input type="checkbox" bind:checked={m.favorite} onchange={changed} />Favorite</label>
      </div>
    </div>

    <div class="card">
      <h3 class="section-title">Program</h3>
      {#if m.target === null}
        <p class="muted small-text">Plays in whatever program is active.</p>
        <div class="actions">
          <button type="button" disabled={picking} onclick={() => void pickWindow()}><Icon name="target" size={14} />{picking ? `Switch to the program… ${pickLeft}` : "Use the active program…"}</button>
          <button type="button" onclick={() => { if (m !== null) { m.target = { process: "", title: null, onMismatch: "warn" }; changed(); } }}>Name a program</button>
        </div>
      {:else}
        {@const t = m.target}
        <div class="form-grid">
          <label class="field">Program file <input type="text" aria-label="Program file" maxlength="260" value={t.process} placeholder="notepad.exe" oninput={(e) => { if (m?.target) { m.target.process = (e.currentTarget as HTMLInputElement).value; changed(); } }} /></label>
          <label class="field"><span>Window title contains <span class="hint">(optional)</span></span><input type="text" aria-label="Window title contains (optional)" maxlength="260" value={t.title ?? ""} oninput={(e) => { if (m?.target) { const v = (e.currentTarget as HTMLInputElement).value; m.target.title = v === "" ? null : v; changed(); } }} /></label>
          <label class="field">When another program is active
            <select aria-label="When another program is active" value={t.onMismatch} onchange={(e) => { if (m?.target) { m.target.onMismatch = (e.currentTarget as HTMLSelectElement).value as Mismatch; changed(); } }}>
              <option value="warn">Play anyway and say so</option>
              <option value="stop">Stop before typing into it</option>
            </select>
          </label>
        </div>
        <div class="actions">
          <button type="button" class="small" disabled={picking} onclick={() => void pickWindow()}>{picking ? `Switch to the program… ${pickLeft}` : "Use the active program…"}</button>
          <button type="button" class="small" onclick={() => { if (m !== null) { m.target = null; changed(); } }}>Any program</button>
        </div>
      {/if}
    </div>

    <div class="card">
      <h3 class="section-title">Playback</h3>
      <PlaybackForm
        value={m.playback}
        actions={m.actions.length}
        idPrefix="editor-playback"
        onchange={(v) => {
          if (m !== null) {
            m.playback = v;
            changed();
          }
        }}
      />
    </div>

    <div class="card">
      <div class="actions-head">
        <h3 class="section-title">Actions</h3>
        <span class="muted small-text">{m.actions.length.toLocaleString("en-US")} actions, {formatDuration(total)} at normal speed</span>
      </div>

      <div class="timeline" role="img" aria-label="Timeline of {m.actions.length} actions over {formatDuration(total)}">
        {#each marks as k (k.index)}
          <span
            class="mark {k.type}"
            class:sel={selected.includes(k.index)}
            class:now={k.index === current}
            style="left: {k.left}%"
          ></span>
        {/each}
        {#if previewing}
          <span class="playhead" style="left: {total === 0 ? 0 : (playhead / total) * 100}%"></span>
        {/if}
      </div>
      <div class="legend small-text">
        <span><span class="swatch click"></span>Clicks</span>
        <span><span class="swatch key"></span>Keys</span>
        <span><span class="swatch move"></span>Movements</span>
        <span><span class="swatch scroll"></span>Scrolling</span>
        <span><span class="swatch wait"></span>Waits and pauses</span>
      </div>

      <div class="toolbar">
        <select aria-label="Kind of action to add" bind:value={addType}>
          {#each Object.entries(ACTION_TYPE_LABELS) as [t, label] (t)}<option value={t}>{label}</option>{/each}
        </select>
        <button type="button" onclick={insert}><Icon name="plus" size={14} />{selected.length > 0 ? "Add after selection" : "Add at the end"}</button>
        {#if previewing}
          <button type="button" onclick={stopPreview}><Icon name="stop" size={14} />Stop preview</button>
        {:else}
          <button type="button" disabled={m.actions.length === 0} onclick={preview}><Icon name="play" size={14} />Preview</button>
        {/if}
        <button type="button" disabled={!m.actions.some((a) => a.type === "move")} onclick={removeMoves}>Remove pointer movements</button>
      </div>
      {#if previewing && current >= 0}
        <p class="hint" role="status">Preview (no input is sent): action {current + 1}, {describeAction(m.actions[current] ?? { type: "wait", delayMs: 0 }, keys)}</p>
      {/if}

      {#if selected.length > 0}
        <div class="selectionbar" role="region" aria-label="Selected actions">
          <span>{selected.length} selected</span>
          <button type="button" class="small" onclick={() => duplicate(selected)}>Duplicate</button>
          <button type="button" class="small danger" onclick={() => remove(selected)}>Delete</button>
          <label class="inline small-text">Delay <input type="number" class="coord" min="0" aria-label="Delay of the selected actions, in ms" bind:value={bulkDelay} /> ms
            <button type="button" class="small" onclick={() => setDelays(selected, () => bulkDelay)}>Set</button></label>
          <label class="inline small-text">× <input type="number" class="coord" min="0" step="0.1" aria-label="Scale the delays of the selected actions by" bind:value={bulkFactor} />
            <button type="button" class="small" onclick={() => setDelays(selected, (d) => d * bulkFactor)}>Scale delays</button></label>
          <button type="button" class="small" onclick={() => { if (m !== null) { m.playback.section = { first: Math.min(...selected), last: Math.max(...selected) }; changed(); } }}>Repeat only these</button>
          <button type="button" class="ghost small" onclick={() => (selected = [])}>Clear</button>
        </div>
      {/if}

      {#if m.actions.length === 0}
        <div class="empty">
          <strong>No actions yet</strong>
          <p>Record them on the Recorder tab, or add them here one by one.</p>
        </div>
      {:else}
        <div class="table-wrap">
          <table class="actions-table">
            <thead>
              <tr>
                <th scope="col"><span class="visually-hidden">Select</span></th>
                <th scope="col">#</th>
                <th scope="col">At</th>
                <th scope="col">Delay (ms)</th>
                <th scope="col">Action</th>
                <th scope="col">Details</th>
                <th scope="col"><span class="visually-hidden">Order</span></th>
              </tr>
            </thead>
            <tbody>
              {#each m.actions.slice(page * PAGE, page * PAGE + PAGE) as a, k (page * PAGE + k)}
                {@const i = page * PAGE + k}
                <tr class:sel={selected.includes(i)} class:now={i === current} class:section={m.playback.section !== null && i >= m.playback.section.first && i <= m.playback.section.last}>
                  <td><input type="checkbox" aria-label="Select action {i + 1}" checked={selected.includes(i)} onchange={(e) => toggle(i, (e.currentTarget as HTMLInputElement).checked)} /></td>
                  <td class="num">{i + 1}</td>
                  <td class="num muted">{formatDuration(times[i] ?? 0)}</td>
                  <td>
                    <input type="number" class="coord" min="0" max="86400000" aria-label="Action {i + 1}: delay before it in ms" value={a.delayMs}
                      onchange={(e) => { const n = num(e); if (n !== null && n >= 0) { setAction(i, { ...a, delayMs: Math.round(n) }); } }} />
                  </td>
                  <td><span class="type {a.type}">{ACTION_TYPE_LABELS[a.type]}</span></td>
                  <td class="details">{@render details(a, i)}</td>
                  <td class="order">
                    <button type="button" class="ghost icon small" aria-label="Move action {i + 1} up" disabled={i === 0} onclick={() => move(i, -1)}><Icon name="arrow-up" size={14} /></button>
                    <button type="button" class="ghost icon small" aria-label="Move action {i + 1} down" disabled={i === m.actions.length - 1} onclick={() => move(i, 1)}><Icon name="arrow-down" size={14} /></button>
                    <button type="button" class="ghost icon small" aria-label="Duplicate action {i + 1}" onclick={() => duplicate([i])}><Icon name="copy" size={14} /></button>
                    <button type="button" class="ghost icon small" aria-label="Delete action {i + 1}" onclick={() => remove([i])}><Icon name="trash" size={14} /></button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if pages > 1}
          <div class="pager">
            <button type="button" class="small" disabled={page === 0} onclick={() => (page -= 1)}>Previous</button>
            <span class="small-text muted">Actions {page * PAGE + 1}–{Math.min(m.actions.length, (page + 1) * PAGE)} of {m.actions.length.toLocaleString("en-US")}</span>
            <button type="button" class="small" disabled={page >= pages - 1} onclick={() => (page += 1)}>Next</button>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</section>

{#if confirmLeave}
  <Modal title="Leave without saving?" onclose={() => (confirmLeave = false)}>
    <p>The changes to “{m?.name ?? ""}” are not saved and will be lost.</p>
    {#snippet footer()}
      <button type="button" class="primary danger" onclick={() => { confirmLeave = false; onclose(); }}>Discard changes</button>
      <button type="button" onclick={() => (confirmLeave = false)}>Keep editing</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .editor-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .editor-head h2 {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
    gap: var(--space-3) var(--space-4);
    margin-bottom: var(--space-2);
  }

  .wide {
    grid-column: 1 / -1;
  }

  .inline,
  .xy {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .coord {
    width: 5.5rem;
  }

  .actions-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .timeline {
    position: relative;
    height: 28px;
    margin: var(--space-2) 0;
    background: var(--surface-2);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .mark {
    position: absolute;
    top: 6px;
    width: 2px;
    height: 16px;
    margin-left: -1px;
    border-radius: 1px;
    background: var(--text-subtle);
  }

  .mark.move,
  .swatch.move {
    background: var(--chart-3);
    opacity: 0.6;
  }

  .mark.click,
  .mark.button,
  .swatch.click {
    background: var(--chart-1);
  }

  .mark.key,
  .mark.keys,
  .swatch.key {
    background: var(--chart-2);
  }

  .mark.scroll,
  .swatch.scroll {
    background: var(--warning);
  }

  .mark.wait,
  .mark.pause,
  .swatch.wait {
    background: var(--text-muted);
  }

  .mark.sel {
    top: 2px;
    height: 24px;
    outline: 1px solid var(--accent);
  }

  .mark.now {
    top: 0;
    height: 28px;
    width: 3px;
  }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--danger-solid);
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
    color: var(--text-muted);
  }

  .swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    margin-right: var(--space-1);
    border-radius: 2px;
    vertical-align: -1px;
  }

  .selectionbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin: var(--space-3) 0;
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    background: var(--accent-soft);
    border-radius: var(--radius-md);
  }

  .table-wrap {
    margin-top: var(--space-3);
    overflow-x: auto;
  }

  .actions-table th {
    position: static;
    background: var(--surface);
  }

  .actions-table td {
    padding: var(--space-1) var(--space-2);
  }

  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  tr.sel td {
    background: var(--accent-soft);
  }

  tr.now td {
    background: var(--warning-soft);
  }

  tr.section td:first-child {
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .type {
    display: inline-block;
    padding: 1px var(--space-2);
    font-size: var(--text-xs);
    border-radius: var(--radius-full);
    background: var(--surface-2);
    white-space: nowrap;
  }

  .order {
    white-space: nowrap;
  }

  .pager {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
</style>
