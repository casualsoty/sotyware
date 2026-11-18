<script lang="ts">
  import {
    PROFILE_ACTIONS,
    saveAutomationSettings,
    saveProfile,
    type Binding,
    type HotkeyAction,
    type HotkeyView,
    type Profile,
    type Settings,
  } from "./automation";
  import { formatChord, HOTKEY_LABELS } from "./automationFormat";
  import type { AutomationModel } from "./automationModel.svelte";
  import HotkeyInput from "./HotkeyInput.svelte";

  // The Settings tab (ADR-050): the active profile's shortcuts, the
  // emergency stop every profile shares, the safety options and what this
  // system allows. Shortcuts are registered by Rust while the autoclicker
  // is on; the status says whether the system accepted each one.

  interface Props {
    model: AutomationModel;
  }

  let { model }: Props = $props();

  const config = $derived(model.config);
  const status = $derived(model.status);
  const profile = $derived(model.activeProfile);
  const keys = $derived(config?.keys ?? []);

  let error = $state<string | null>(null);
  let saving = $state(false);

  function stateOf(owner: (h: HotkeyView) => boolean, b: Binding): { text: string; tone: string } {
    if (b.chord === null) {
      return { text: "Not set", tone: "" };
    }
    if (!b.enabled) {
      return { text: "Off", tone: "" };
    }
    const h = status?.hotkeys.find((x) => owner(x) && x.chord === b.chord);
    if (!status?.running) {
      return { text: "Used while the autoclicker is on", tone: "" };
    }
    if (h === undefined) {
      return { text: "…", tone: "" };
    }
    return h.registered ? { text: "Ready", tone: "success" } : { text: h.error ?? "Not registered", tone: "warning" };
  }

  async function setProfileBinding(p: Profile, action: HotkeyAction, b: Binding): Promise<void> {
    saving = true;
    try {
      const next = $state.snapshot(p) as Profile;
      next.hotkeys = { ...next.hotkeys, [action]: b };
      model.putProfile(await saveProfile(next));
      await model.refreshStatus();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  async function setSettings(patch: Partial<Settings>): Promise<void> {
    if (config === null) {
      return;
    }
    saving = true;
    try {
      model.putSettings(await saveAutomationSettings({ ...($state.snapshot(config.settings) as Settings), ...patch }));
      await model.refreshStatus();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }
</script>

{#snippet row(label: string, b: Binding, suggestion: string | undefined, isOwner: (h: HotkeyView) => boolean, save: (b: Binding) => void, shared: boolean)}
  {@const st = stateOf(isOwner, b)}
  <tr>
    <th scope="row">
      {label}
      {#if shared}<span class="hint">All profiles</span>{/if}
    </th>
    <td>
      <HotkeyInput value={b.chord} {keys} {label} disabled={saving} onchange={(c) => save({ chord: c, enabled: c !== null })} />
      {#if b.chord === null && suggestion !== undefined}
        <button type="button" class="small link suggest" disabled={saving} onclick={() => save({ chord: suggestion, enabled: true })}>Use {formatChord(suggestion, keys)}</button>
      {/if}
    </td>
    <td>
      {#if b.chord !== null}
        <label class="check"><input type="checkbox" checked={b.enabled} disabled={saving} onchange={(e) => save({ chord: b.chord, enabled: (e.currentTarget as HTMLInputElement).checked })} />On</label>
      {/if}
    </td>
    <td class="state {st.tone}">{st.text}</td>
  </tr>
{/snippet}

<section aria-labelledby="automation-settings-heading">
  <h2 id="automation-settings-heading">Settings</h2>
  {#if error !== null}<p class="callout danger" role="alert">{error}</p>{/if}

  {#if config !== null && profile !== null}
    {@const p = profile}
    <div class="card">
      <h3 class="section-title">Shortcuts</h3>
      <p class="muted small-text">
        They work in every program while the autoclicker is on. Those below belong to the profile “{p.name}”, except the
        emergency stop. Click a field and press the keys; Backspace clears it.
      </p>
      <div class="table-wrap">
        <table class="shortcuts">
          <tbody>
            {@render row(
              HOTKEY_LABELS["stop-all"],
              config.settings.emergencyStop,
              config.suggestions["stop-all"],
              (h) => h.owner.kind === "settings",
              (b) => void setSettings({ emergencyStop: b }),
              true,
            )}
            {#each PROFILE_ACTIONS as action (action)}
              {@render row(
                HOTKEY_LABELS[action],
                p.hotkeys[action] ?? { chord: null, enabled: false },
                config.suggestions[action],
                (h) => h.owner.kind === "profile" && h.owner.action === action,
                (b) => void setProfileBinding(p, action, b),
                false,
              )}
            {/each}
          </tbody>
        </table>
      </div>
      <p class="hint">
        A macro's own shortcut is set in its editor. A key that another program already uses cannot be taken; choose
        another one.
      </p>
    </div>

    <div class="card">
      <h3 class="section-title">Safety</h3>
      <label class="check">
        <input type="checkbox" checked={config.settings.failsafeCorners} disabled={saving} onchange={(e) => void setSettings({ failsafeCorners: (e.currentTarget as HTMLInputElement).checked })} />
        Stop everything when the pointer is pushed into a corner of the screen
      </label>
      <div class="field countdown">
        <label for="automation-countdown">Wait before starting from this window</label>
        <select id="automation-countdown" value={config.settings.dashboardCountdownMs} disabled={saving} onchange={(e) => void setSettings({ dashboardCountdownMs: Number((e.currentTarget as HTMLSelectElement).value) })}>
          {#each [0, 1000, 2000, 3000, 5000, 10000] as ms (ms)}
            <option value={ms}>{ms === 0 ? "No wait" : `${ms / 1000} s`}</option>
          {/each}
        </select>
        <p class="hint">Time to move the pointer, or switch to another program, after clicking Start here.</p>
      </div>
      <ul class="facts small-text">
        <li>Stopping, pausing or quitting releases every key and mouse button that automation held.</li>
        <li>The autoclicker never clicks sotyware's own window, so this window's Stop button always works.</li>
        <li>Only one thing runs at a time: the autoclicker, a recording or a macro.</li>
        <li>Nothing starts again by itself after sotyware restarts.</li>
      </ul>
    </div>
  {/if}

  {#if status !== null}
    <div class="card">
      <h3 class="section-title">This computer</h3>
      {#if status.support.available}
        <p class="small-text">
          Input goes through {status.support.backend === "windows" ? "Windows' SendInput, recorded with low-level hooks" : "the X11 XTEST and RECORD extensions"}.
        </p>
        <ul class="facts small-text">
          {#if status.support.backend === "windows"}
            <li>Programs running as administrator, the sign-in screen and UAC prompts do not accept input from sotyware unless it runs as administrator too.</li>
          {:else}
            <li>Only programs on this X11 display can be automated.</li>
          {/if}
          <li>Positions are in physical pixels of all monitors together; macros can adapt them when the screens change.</li>
        </ul>
      {:else}
        <p class="callout warning">{status.support.note ?? "Input automation is not available on this system."}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  section > h2 {
    margin-bottom: var(--space-3);
  }

  .card + .card {
    margin-top: var(--space-4);
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

  .shortcuts th .hint {
    display: block;
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

  .countdown {
    margin-top: var(--space-3);
    max-width: 24rem;
  }

  .facts {
    margin: var(--space-3) 0 0;
    padding-left: var(--space-5);
    color: var(--text-muted);
  }

  /* Text in a flex box is a <p>: WebKitGTK leaves a <span> flex item's
     text out of the accessibility tree (frontend.md). No margin, as the
     <span> had none. */
  .field > p.hint {
    margin: 0;
  }
</style>
