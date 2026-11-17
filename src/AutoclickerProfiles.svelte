<script lang="ts">
  import {
    activateProfile,
    createProfile,
    deleteProfile,
    exportAutomation,
    importAutomation,
    saveProfile,
    setDefaultProfile,
    type PlaybackSettings,
    type Profile,
  } from "./automation";
  import { clickerSummary, playbackSummary } from "./automationFormat";
  import type { AutomationModel } from "./automationModel.svelte";
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import PlaybackForm from "./PlaybackForm.svelte";

  // The Profiles tab (ADR-050): sets of autoclicker settings, shortcuts and
  // a macro to switch between. Switching applies at once (its shortcuts
  // replace the others); what runs goes on with the settings it started
  // with. The default profile is the one sotyware starts with.

  interface Props {
    model: AutomationModel;
  }

  let { model }: Props = $props();

  const config = $derived(model.config);
  const active = $derived(model.activeProfile);
  const macros = $derived(model.macros ?? []);

  let error = $state<string | null>(null);
  let message = $state<string | null>(null);
  let newName = $state("");
  let copyActive = $state(true);
  let renaming = $state<string | null>(null);
  let renameTo = $state("");
  let confirmDelete = $state<Profile | null>(null);
  let busy = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function fail(e: unknown): void {
    error = e instanceof Error ? e.message : String(e);
    message = null;
  }

  async function run(f: () => Promise<void>): Promise<void> {
    busy = true;
    try {
      await f();
      error = null;
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }

  async function create(): Promise<void> {
    await run(async () => {
      const p = await createProfile(newName.trim() === "" ? "Profile" : newName, copyActive ? (active?.id ?? null) : null);
      model.putProfile(p);
      newName = "";
      message = `Created “${p.name}”. Use it to make it the active profile.`;
    });
  }

  async function use(p: Profile): Promise<void> {
    await run(async () => {
      await activateProfile(p.id);
      await model.loadConfig();
      await model.refreshStatus();
      message = `“${p.name}” is the active profile; its shortcuts are in use.`;
    });
  }

  async function makeDefault(p: Profile): Promise<void> {
    await run(async () => {
      await setDefaultProfile(p.id);
      await model.loadConfig();
      message = `sotyware starts with “${p.name}” from now on.`;
    });
  }

  async function rename(p: Profile): Promise<void> {
    await run(async () => {
      model.putProfile(await saveProfile({ ...($state.snapshot(p) as Profile), name: renameTo }));
      renaming = null;
    });
  }

  async function duplicate(p: Profile): Promise<void> {
    await run(async () => {
      const copy = await createProfile(p.name, p.id);
      model.putProfile(copy);
      message = `Copied as “${copy.name}”.`;
    });
  }

  async function remove(p: Profile): Promise<void> {
    await run(async () => {
      await deleteProfile(p.id);
      confirmDelete = null;
      await model.loadConfig();
      await model.refreshStatus();
      message = `Deleted “${p.name}”.`;
    });
    confirmDelete = null;
  }

  async function exportOne(p: Profile): Promise<void> {
    await run(async () => {
      const ids = p.macroId === null ? [] : [p.macroId];
      const file = await exportAutomation(ids, [p.id]);
      if (file !== null) {
        message = `Exported “${p.name}”${ids.length > 0 ? " with its macro" : ""} to ${file}.`;
      }
    });
  }

  async function importFile(): Promise<void> {
    await run(async () => {
      const r = await importAutomation();
      if (r !== null) {
        await model.loadConfig();
        await model.loadMacros();
        message = `Imported ${r.profiles.length} profile${r.profiles.length === 1 ? "" : "s"} and ${r.macros.length} macro${r.macros.length === 1 ? "" : "s"}.${r.problems.length > 0 ? ` Not imported: ${r.problems.join(" ")}` : ""}`;
      }
    });
  }

  /** The active profile's macro and playback settings, saved a moment
   * after each change. */
  function setActive(patch: Partial<Profile>): void {
    if (active === null) {
      return;
    }
    const next = { ...($state.snapshot(active) as Profile), ...patch };
    model.putProfile(next);
    if (timer !== null) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = null;
      void run(async () => {
        const cur = model.activeProfile;
        if (cur !== null) {
          model.putProfile(await saveProfile($state.snapshot(cur) as Profile));
        }
      });
    }, 400);
  }

  function shortcuts(p: Profile): number {
    return Object.values(p.hotkeys).filter((b) => b.enabled && b.chord !== null).length;
  }

  const DEFAULT_PLAYBACK: PlaybackSettings = {
    repeat: 1,
    stopAfterMs: null,
    speed: 1,
    startDelayMs: 0,
    repeatDelayMs: 0,
    jitterPercent: 0,
    positioning: "absolute",
    scaleToScreen: true,
    section: null,
  };
</script>

<section aria-labelledby="profiles-heading">
  <div class="head">
    <div>
      <h2 id="profiles-heading">Profiles</h2>
      <p class="muted small-text">Each profile keeps its own autoclicker settings, shortcuts and macro.</p>
    </div>
    <button type="button" onclick={() => void importFile()}><Icon name="upload" size={14} />Import…</button>
  </div>

  {#if error !== null}<p class="callout danger" role="alert">{error}</p>{/if}
  {#if message !== null}<p class="callout success" role="status">{message}</p>{/if}

  {#if config === null}
    <p class="muted">Loading…</p>
  {:else}
    <ul class="profiles">
      {#each config.profiles as p (p.id)}
        {@const isActive = p.id === config.activeProfile}
        {@const isDefault = p.id === config.settings.defaultProfile}
        <li class="card profile" class:active={isActive} data-profile={p.id}>
          <div class="title-row">
            {#if renaming === p.id}
              <form class="rename" onsubmit={(e) => { e.preventDefault(); void rename(p); }}>
                <input type="text" maxlength="120" aria-label="New name for {p.name}" bind:value={renameTo} />
                <button type="submit" class="small primary" disabled={busy || renameTo.trim() === ""}>Rename</button>
                <button type="button" class="small" onclick={() => (renaming = null)}>Cancel</button>
              </form>
            {:else}
              <h3>{p.name}</h3>
            {/if}
            {#if isActive}<span class="badge success">Active</span>{/if}
            {#if isDefault}<span class="badge">Default</span>{/if}
          </div>
          <p class="small-text">{clickerSummary(p.clicker)}</p>
          <p class="small-text muted">
            Macro: {p.macroId === null ? "none" : (macros.find((m) => m.id === p.macroId)?.name ?? "…")}{p.playback !== null
              ? ` (${playbackSummary(p.playback)})`
              : ""} · {shortcuts(p)} shortcut{shortcuts(p) === 1 ? "" : "s"}
          </p>
          <div class="actions">
            {#if !isActive}
              <button type="button" class="small primary" disabled={busy} onclick={() => void use(p)}>Use this profile</button>
            {/if}
            {#if !isDefault}
              <button type="button" class="small" disabled={busy} onclick={() => void makeDefault(p)}>Start with it</button>
            {/if}
            <button type="button" class="small" disabled={busy} onclick={() => { renaming = p.id; renameTo = p.name; }}>Rename</button>
            <button type="button" class="small" disabled={busy} onclick={() => void duplicate(p)}>Duplicate</button>
            <button type="button" class="small" disabled={busy} onclick={() => void exportOne(p)}>Export…</button>
            <button type="button" class="small danger" disabled={busy || config.profiles.length === 1} onclick={() => (confirmDelete = p)}>Delete</button>
          </div>
        </li>
      {/each}
    </ul>

    <form class="card create" onsubmit={(e) => { e.preventDefault(); void create(); }}>
      <label class="field">New profile <input type="text" aria-label="New profile" maxlength="120" placeholder="Gaming" bind:value={newName} /></label>
      <label class="check"><input type="checkbox" bind:checked={copyActive} />Start from the active profile's settings</label>
      <button type="submit" disabled={busy}><Icon name="plus" size={14} />Create profile</button>
    </form>

    {#if active !== null}
      {@const a = active}
      <div class="card">
        <h3 class="section-title">The macro of “{a.name}”</h3>
        <p class="muted small-text">What “Play the profile's macro” (and its shortcut) plays.</p>
        <div class="field">
          <label for="profile-macro">Macro</label>
          <select id="profile-macro" value={a.macroId ?? ""} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; setActive({ macroId: v === "" ? null : v }); }}>
            <option value="">None</option>
            {#each macros as m (m.id)}<option value={m.id}>{m.name}</option>{/each}
          </select>
        </div>
        {#if a.macroId !== null}
          <label class="check own">
            <input type="checkbox" checked={a.playback === null} onchange={(e) => {
              const own = (e.currentTarget as HTMLInputElement).checked;
              setActive({ playback: own ? null : DEFAULT_PLAYBACK });
            }} />
            Use the macro's own playback settings
          </label>
          {#if a.playback !== null}
            <PlaybackForm value={a.playback} idPrefix="profile-playback" onchange={(v) => setActive({ playback: v })} />
          {/if}
        {/if}
      </div>
    {/if}
  {/if}
</section>

{#if confirmDelete !== null}
  {@const d = confirmDelete}
  <Modal title="Delete the profile “{d.name}”?" onclose={() => (confirmDelete = null)} locked={busy}>
    <p>Its autoclicker settings and shortcuts are deleted. Its macro stays in the library.</p>
    {#snippet footer()}
      <button type="button" class="primary danger" disabled={busy} onclick={() => void remove(d)}>Delete profile</button>
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

  .profiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr));
    gap: var(--space-3);
    margin: 0 0 var(--space-4);
    padding: 0;
    list-style: none;
  }

  .profile.active {
    border-color: var(--success-border);
  }

  .title-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .title-row h3 {
    overflow-wrap: anywhere;
  }

  .rename {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .profile p {
    margin: 0 0 var(--space-1);
  }

  .create {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .create .field {
    flex: 1 1 14rem;
  }

  .own {
    margin: var(--space-3) 0;
  }
</style>
