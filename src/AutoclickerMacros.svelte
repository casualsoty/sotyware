<script lang="ts">
  import {
    createMacro,
    deleteMacro,
    duplicateMacro,
    exportAutomation,
    importAutomation,
    updateMacroInfo,
    type ImportReport,
    type MacroSummary,
  } from "./automation";
  import { formatChord, formatDuration } from "./automationFormat";
  import { formatCount, formatDate } from "./format";
  import type { AutomationModel } from "./automationModel.svelte";
  import AutoclickerEditor from "./AutoclickerEditor.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // The Macros tab (ADR-050): the macro library (search, folders,
  // favourites, play, edit, duplicate, export, delete, import) and, when a
  // macro is opened, its editor.

  interface Props {
    model: AutomationModel;
    canPlay: boolean;
    /** The macro to open in the editor (set by other tabs). */
    editing: string | null;
    onedit: (id: string | null) => void;
    onplay: (id: string) => void;
  }

  let { model, canPlay, editing, onedit, onplay }: Props = $props();

  const keys = $derived(model.config?.keys ?? []);
  const playingId = $derived(model.status?.session?.kind === "playback" ? model.status.session.macroId : null);

  let query = $state("");
  let folder = $state("");
  let favorites = $state(false);
  let sort = $state<"name" | "modified" | "duration">("name");
  let chosen = $state<string[]>([]);
  let creating = $state(false);
  let newName = $state("");
  let error = $state<string | null>(null);
  let message = $state<string | null>(null);
  let report = $state<ImportReport | null>(null);
  let confirmDelete = $state<MacroSummary | null>(null);
  let busy = $state(false);

  const all = $derived(model.macros ?? []);
  const folders = $derived([...new Set(all.map((m) => m.folder).filter((f) => f !== ""))].sort((a, b) => a.localeCompare(b)));
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = all.filter(
      (m) =>
        (folder === "" || m.folder === folder) &&
        (!favorites || m.favorite) &&
        (q === "" || `${m.name} ${m.description} ${m.folder}`.toLowerCase().includes(q)),
    );
    return list.sort((a, b) =>
      sort === "modified"
        ? b.modifiedMs - a.modifiedMs
        : sort === "duration"
          ? b.durationMs - a.durationMs
          : a.name.localeCompare(b.name),
    );
  });

  function fail(e: unknown): void {
    error = e instanceof Error ? e.message : String(e);
    message = null;
  }

  async function create(): Promise<void> {
    if (newName.trim() === "") {
      return;
    }
    busy = true;
    try {
      const m = await createMacro(newName, "", folder);
      model.putMacro({
        id: m.id,
        name: m.name,
        description: m.description,
        folder: m.folder,
        favorite: m.favorite,
        shortcut: m.shortcut,
        actions: 0,
        durationMs: 0,
        createdMs: m.createdMs,
        modifiedMs: m.modifiedMs,
        target: null,
        hasPause: false,
      });
      creating = false;
      newName = "";
      error = null;
      onedit(m.id);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function favorite(m: MacroSummary): Promise<void> {
    try {
      model.putMacro(await updateMacroInfo(m.id, { favorite: !m.favorite }));
    } catch (e) {
      fail(e);
    }
  }

  async function duplicate(m: MacroSummary): Promise<void> {
    try {
      const copy = await duplicateMacro(m.id);
      model.putMacro(copy);
      message = `Copied as “${copy.name}”.`;
      error = null;
    } catch (e) {
      fail(e);
    }
  }

  async function remove(m: MacroSummary): Promise<void> {
    busy = true;
    try {
      await deleteMacro(m.id);
      model.dropMacro(m.id);
      chosen = chosen.filter((x) => x !== m.id);
      confirmDelete = null;
      message = `Deleted “${m.name}”.`;
      error = null;
      await model.loadConfig();
    } catch (e) {
      fail(e);
      confirmDelete = null;
    } finally {
      busy = false;
    }
  }

  async function exportSome(ids: string[]): Promise<void> {
    try {
      const file = await exportAutomation(ids, []);
      if (file !== null) {
        message = `Exported ${ids.length === 1 ? "the macro" : `${ids.length} macros`} to ${file}.`;
        error = null;
      }
    } catch (e) {
      fail(e);
    }
  }

  async function importFile(): Promise<void> {
    try {
      const r = await importAutomation();
      if (r !== null) {
        report = r;
        for (const m of r.macros) {
          model.putMacro(m);
        }
        if (r.profiles.length > 0) {
          await model.loadConfig();
        }
        error = null;
        message = null;
      }
    } catch (e) {
      fail(e);
    }
  }
</script>

{#if editing !== null}
  <AutoclickerEditor {model} id={editing} {canPlay} onclose={() => onedit(null)} {onplay} />
{:else}
  <section aria-labelledby="macros-heading">
    <div class="head">
      <div>
        <h2 id="macros-heading">Macros</h2>
        <p class="muted small-text">Recorded or written sequences of clicks and keys.</p>
      </div>
      <div class="toolbar">
        <button type="button" onclick={() => (creating = !creating)}><Icon name="plus" size={14} />New macro</button>
        <button type="button" onclick={() => void importFile()}><Icon name="upload" size={14} />Import…</button>
        <button type="button" disabled={chosen.length === 0} onclick={() => void exportSome(chosen)}><Icon name="download" size={14} />Export{chosen.length > 0 ? ` ${chosen.length}` : ""}…</button>
      </div>
    </div>

    {#if creating}
      <form class="card create" onsubmit={(e) => { e.preventDefault(); void create(); }}>
        <label class="field">Name <input type="text" aria-label="Name" maxlength="120" bind:value={newName} required placeholder="Open the daily report" /></label>
        <div class="actions">
          <button type="submit" class="primary" disabled={busy || newName.trim() === ""}>Create and edit</button>
          <button type="button" onclick={() => (creating = false)}>Cancel</button>
        </div>
      </form>
    {/if}

    {#if error !== null}<p class="callout danger" role="alert">{error}</p>{/if}
    {#if message !== null}<p class="callout success" role="status">{message}</p>{/if}
    {#if report !== null}
      {@const r = report}
      <div class="callout {r.problems.length > 0 ? 'warning' : 'success'}" role="status">
        <p>
          Imported {r.macros.length} macro{r.macros.length === 1 ? "" : "s"}{r.profiles.length > 0
            ? ` and ${r.profiles.length} profile${r.profiles.length === 1 ? "" : "s"}`
            : ""}. Imported shortcuts are switched off until you turn them on.
        </p>
        {#each r.problems as p (p)}<p class="text-danger">{p}</p>{/each}
        <button type="button" class="link" onclick={() => (report = null)}>Close</button>
      </div>
    {/if}

    <div class="filters">
      <input type="search" aria-label="Search macros" placeholder="Search" bind:value={query} />
      <select aria-label="Folder" bind:value={folder}>
        <option value="">All folders</option>
        {#each folders as f (f)}<option value={f}>{f}</option>{/each}
      </select>
      <label class="check"><input type="checkbox" bind:checked={favorites} />Favorites only</label>
      <select aria-label="Sort by" bind:value={sort}>
        <option value="name">By name</option>
        <option value="modified">Recently changed first</option>
        <option value="duration">Longest first</option>
      </select>
    </div>

    {#if model.macros === null}
      <p class="muted">Loading…</p>
    {:else if all.length === 0}
      <div class="empty">
        <strong>No macros yet</strong>
        <p>Record one on the Recorder tab, write one with New macro, or import a file.</p>
      </div>
    {:else if shown.length === 0}
      <p class="muted">No macro matches.</p>
    {:else}
      <div class="table-wrap">
        <table class="library">
          <thead>
            <tr>
              <th scope="col"><span class="visually-hidden">Choose for export</span></th>
              <th scope="col"><span class="visually-hidden">Favorite</span></th>
              <th scope="col">Name</th>
              <th scope="col">Actions</th>
              <th scope="col">Duration</th>
              <th scope="col">Changed</th>
              <th scope="col">Shortcut</th>
              <th scope="col"><span class="visually-hidden">Commands</span></th>
            </tr>
          </thead>
          <tbody>
            {#each shown as m (m.id)}
              <tr data-macro={m.id} class:playing={playingId === m.id}>
                <td>
                  <input type="checkbox" aria-label="Choose {m.name}" checked={chosen.includes(m.id)}
                    onchange={(e) => (chosen = (e.currentTarget as HTMLInputElement).checked ? [...chosen, m.id] : chosen.filter((x) => x !== m.id))} />
                </td>
                <td>
                  <button type="button" class="ghost icon small star" class:on={m.favorite} aria-pressed={m.favorite} aria-label="Favorite: {m.name}" onclick={() => void favorite(m)}><Icon name="star" size={14} filled={m.favorite} /></button>
                </td>
                <td class="name-cell">
                  <button type="button" class="link name" onclick={() => onedit(m.id)}>{m.name}</button>
                  {#if m.folder !== ""}<span class="badge">{m.folder}</span>{/if}
                  {#if m.target !== null}<span class="badge info" title="Plays in {m.target}">{m.target}</span>{/if}
                  {#if m.description !== ""}<p class="desc">{m.description}</p>{/if}
                </td>
                <td class="num">{formatCount(m.actions)}</td>
                <td class="num">{formatDuration(m.durationMs)}</td>
                <td class="num">{formatDate(new Date(m.modifiedMs))}</td>
                <td>
                  {#if m.shortcut.chord !== null}
                    <kbd class:off={!m.shortcut.enabled}>{formatChord(m.shortcut.chord, keys)}</kbd>{#if !m.shortcut.enabled}<span class="visually-hidden">(off)</span>{/if}
                  {:else}
                    <span class="subtle">–</span>
                  {/if}
                </td>
                <td class="commands">
                  <button type="button" class="small" disabled={!canPlay || m.actions === 0} onclick={() => onplay(m.id)}><Icon name="play" size={14} />Play</button>
                  <button type="button" class="ghost icon small" aria-label="Edit {m.name}" onclick={() => onedit(m.id)}><Icon name="edit" size={14} /></button>
                  <button type="button" class="ghost icon small" aria-label="Duplicate {m.name}" onclick={() => void duplicate(m)}><Icon name="copy" size={14} /></button>
                  <button type="button" class="ghost icon small" aria-label="Export {m.name}" onclick={() => void exportSome([m.id])}><Icon name="download" size={14} /></button>
                  <button type="button" class="ghost icon small danger" aria-label="Delete {m.name}" disabled={playingId === m.id} onclick={() => (confirmDelete = m)}><Icon name="trash" size={14} /></button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </section>
{/if}

{#if confirmDelete !== null}
  {@const d = confirmDelete}
  <Modal title="Delete “{d.name}”?" onclose={() => (confirmDelete = null)} locked={busy}>
    <p>Its {formatCount(d.actions)} actions are deleted from this computer. Profiles that play it will play nothing.</p>
    <p class="small-text muted">Export it first if you might want it back.</p>
    {#snippet footer()}
      <button type="button" class="primary danger" disabled={busy} onclick={() => void remove(d)}>Delete macro</button>
      <button type="button" disabled={busy} onclick={() => (confirmDelete = null)}>Cancel</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }

  .head p {
    margin: var(--space-1) 0 0;
  }

  .create {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }

  .create .field {
    flex: 1 1 18rem;
  }

  .create .actions {
    margin: 0;
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin: var(--space-3) 0;
  }

  .filters input[type="search"] {
    flex: 1 1 14rem;
  }

  .table-wrap {
    overflow-x: auto;
  }

  .library th {
    position: static;
  }

  .name-cell {
    min-width: 14rem;
  }

  .name {
    font-weight: var(--weight-medium);
    margin-right: var(--space-2);
  }

  .desc {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .commands {
    white-space: nowrap;
  }

  .star {
    color: var(--text-subtle);
  }

  .star.on {
    color: var(--warning);
  }

  kbd.off {
    opacity: 0.55;
    text-decoration: line-through;
  }

  tr.playing td {
    background: var(--success-soft);
  }
</style>
