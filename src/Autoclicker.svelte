<script lang="ts">
  import { onMount } from "svelte";
  import {
    activateProfile,
    AUTOCLICKER_FEATURE,
    AutomationError,
    playMacro,
    setAutomationPaused,
    startClicker,
    startRecording,
    stopAutomation,
  } from "./automation";
  import { formatClock, formatRate } from "./automationFormat";
  import { formatCount } from "./format";
  import type { AutomationModel } from "./automationModel.svelte";
  import AutoclickerClicker from "./AutoclickerClicker.svelte";
  import AutoclickerMacros from "./AutoclickerMacros.svelte";
  import AutoclickerOverview from "./AutoclickerOverview.svelte";
  import AutoclickerProfiles from "./AutoclickerProfiles.svelte";
  import AutoclickerRecorder from "./AutoclickerRecorder.svelte";
  import AutoclickerSettings from "./AutoclickerSettings.svelte";
  import { setFeatureEnabled } from "./features";
  import Icon, { type IconName } from "./Icon.svelte";
  import Modal from "./Modal.svelte";

  // The Autoclicker page (ADR-050): automates clicks, records and replays
  // mouse and keyboard input. The page shows and asks; Rust sends and
  // records input, and only while the built-in feature `autoclicker` runs
  // (off until the user turns it on here or in Settings › Features).

  type Tab = "overview" | "clicker" | "recorder" | "macros" | "profiles" | "settings";

  interface Props {
    model: AutomationModel;
  }

  let { model }: Props = $props();

  const TABS: { id: Tab; title: string; icon: IconName }[] = [
    { id: "overview", title: "Overview", icon: "overview" },
    { id: "clicker", title: "Clicker", icon: "pointer" },
    { id: "recorder", title: "Recorder", icon: "record" },
    { id: "macros", title: "Macros", icon: "macro" },
    { id: "profiles", title: "Profiles", icon: "profile" },
    { id: "settings", title: "Settings", icon: "settings" },
  ];

  let tab = $state<Tab>("overview");
  let editing = $state<string | null>(null);
  let error = $state<string | null>(null);
  let turningOn = $state(false);
  let confirm = $state<{ message: string; retry: () => Promise<void> } | null>(null);
  let stopping = $state(false);

  const status = $derived(model.status);
  const config = $derived(model.config);
  const session = $derived(status?.session ?? null);
  const available = $derived(status?.support.available ?? false);
  const running = $derived(status?.running ?? false);
  const canStart = $derived(available && running && session === null);
  const sessionState = $derived(session === null ? null : session.kind === "recording" ? session.state : session.progress.state);

  onMount(() => {
    if (model.config === null) {
      void model.loadConfig();
    }
    if (model.macros === null) {
      void model.loadMacros();
    }
  });

  async function act(f: () => Promise<void>, retry?: () => Promise<void>): Promise<void> {
    try {
      await f();
      error = null;
      await model.refreshStatus();
    } catch (e) {
      if (e instanceof AutomationError && e.kind === "confirm" && retry !== undefined) {
        confirm = { message: e.message, retry };
        return;
      }
      error = e instanceof Error ? e.message : String(e);
    }
  }

  const onstartclicker = (): void => void act(() => startClicker(false), () => startClicker(true));
  const onplay = (id: string | null): void => void act(() => playMacro(id, false), () => playMacro(id, true));
  const onrecord = (): void => void act(() => startRecording());
  const onpause = (paused: boolean): void => void act(() => setAutomationPaused(paused));

  async function stop(): Promise<void> {
    stopping = true;
    await act(() => stopAutomation());
    stopping = false;
  }

  async function turnOn(): Promise<void> {
    turningOn = true;
    try {
      const change = await setFeatureEnabled(AUTOCLICKER_FEATURE, true);
      error = change.feature.error;
      await model.refreshStatus();
      await model.loadConfig();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      turningOn = false;
    }
  }

  async function switchProfile(id: string): Promise<void> {
    await act(async () => {
      await activateProfile(id);
      await model.loadConfig();
    });
  }

  function openMacro(id: string | null): void {
    editing = id;
    tab = "macros";
  }

  function onTabKey(event: KeyboardEvent): void {
    const i = TABS.findIndex((t) => t.id === tab);
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (step === 0 && event.key !== "Home" && event.key !== "End") {
      return;
    }
    event.preventDefault();
    const next = event.key === "Home" ? TABS[0] : event.key === "End" ? TABS.at(-1) : TABS[(i + step + TABS.length) % TABS.length];
    if (next) {
      tab = next.id;
      queueMicrotask(() => document.getElementById(`automation-tab-${next.id}`)?.focus());
    }
  }
</script>

<div class="page autoclicker">
  <header class="page-header">
    <div class="titles">
      <h1>Autoclicker</h1>
      <p class="lede">Clicks for you, records your mouse and keyboard, and replays it, from shortcuts that work in every program.</p>
    </div>
    {#if config !== null && config.profiles.length > 1}
      <label class="profile-switch">
        <span class="small-text muted">Profile</span>
        <select value={config.activeProfile} onchange={(e) => void switchProfile((e.currentTarget as HTMLSelectElement).value)}>
          {#each config.profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
        </select>
      </label>
    {/if}
  </header>

  {#if model.error !== null}
    <p class="callout danger" role="alert">The autoclicker cannot be reached: {model.error}</p>
  {/if}
  {#if status !== null && !available}
    <div class="callout warning" role="status">
      <strong>Not available on this computer.</strong>
      {status.support.note ?? ""} Macros and profiles can still be edited, imported and exported.
    </div>
  {:else if status !== null && !running}
    <div class="callout turn-on" role="status">
      <p>
        <strong>The autoclicker is off.</strong> Turning it on lets sotyware send and record mouse and keyboard input and
        registers its shortcuts{status.hotkeys.length > 0 ? ` (${status.hotkeys.map((h) => h.keys).join(", ")})` : ""} in every
        program while sotyware runs. Nothing starts by itself.
      </p>
      <button type="button" class="primary" disabled={turningOn} onclick={() => void turnOn()}><Icon name="power" />{turningOn ? "Turning on…" : "Turn on"}</button>
    </div>
  {/if}
  {#if status?.storageError}
    <p class="callout danger" role="alert">Settings and macros cannot be saved: {status.storageError}</p>
  {/if}
  {#if status?.notice}
    <p class="callout warning" role="status">{status.notice}</p>
  {/if}
  {#if error !== null}
    <p class="callout danger" role="alert">{error}</p>
  {/if}

  {#if session !== null}
    <div class="session-bar" class:paused={sessionState === "paused"} class:recording={session.kind === "recording"} role="region" aria-label="Running automation">
      <span class="pulse" aria-hidden="true"></span>
      <p class="session-text">
        {#if session.kind === "clicker"}
          <strong>{sessionState === "countdown" ? "Autoclicker starting" : sessionState === "paused" ? "Autoclicker paused" : "Clicking"}</strong>
          {formatCount(session.progress.clicks)} clicks · {formatRate(session.progress.cps)}/s · {formatClock(session.progress.elapsedMs)}
        {:else if session.kind === "playback"}
          <strong>{sessionState === "paused" ? "Paused:" : "Playing"} “{session.macroName}”</strong>
          {session.progress.repetitions === null ? `repetition ${session.progress.repetition}` : `${session.progress.repetition} of ${session.progress.repetitions}`} · {formatClock(session.progress.elapsedMs)}
        {:else}
          <strong>{sessionState === "paused" ? "Recording paused" : sessionState === "countdown" ? "Recording starts…" : "Recording"}</strong>
          {formatCount(session.actions)} actions · {formatClock(session.elapsedMs)}
        {/if}
      </p>
      {#if sessionState === "paused"}
        <button type="button" class="small" onclick={() => onpause(false)}><Icon name="play" size={14} />Resume</button>
      {:else}
        <button type="button" class="small" disabled={sessionState === "countdown"} onclick={() => onpause(true)}><Icon name="pause" size={14} />Pause</button>
      {/if}
      <button type="button" class="small primary danger" disabled={stopping} onclick={() => void stop()}><Icon name="stop" size={14} />Stop</button>
    </div>
  {/if}

  <div class="automation-tabs" role="tablist" aria-label="Autoclicker" tabindex="-1" onkeydown={onTabKey}>
    {#each TABS as t (t.id)}
      <button
        type="button"
        role="tab"
        id="automation-tab-{t.id}"
        class="automation-tab"
        aria-selected={tab === t.id}
        aria-controls="automation-panel"
        tabindex={tab === t.id ? 0 : -1}
        onclick={() => {
          tab = t.id;
          if (t.id !== "macros") {
            editing = null;
          }
        }}><Icon name={t.icon} size={15} />{t.title}</button
      >
    {/each}
  </div>

  <div id="automation-panel" role="tabpanel" aria-labelledby="automation-tab-{tab}" class="panel">
    {#if tab === "overview"}
      <AutoclickerOverview {model} {canStart} {onstartclicker} {onplay} {onrecord} {onpause} onstop={() => void stop()} ontab={(t) => (tab = t)} />
    {:else if tab === "clicker"}
      <AutoclickerClicker {model} {canStart} onstart={onstartclicker} onstop={() => void stop()} />
    {:else if tab === "recorder"}
      <AutoclickerRecorder {model} {canStart} {onrecord} onstop={() => void stop()} {onpause} onopenmacro={(id) => openMacro(id)} />
    {:else if tab === "macros"}
      <AutoclickerMacros {model} canPlay={canStart} {editing} onedit={(id) => (editing = id)} onplay={(id) => onplay(id)} />
    {:else if tab === "profiles"}
      <AutoclickerProfiles {model} />
    {:else}
      <AutoclickerSettings {model} />
    {/if}
  </div>
</div>

{#if confirm !== null}
  {@const c = confirm}
  <Modal title="Run until you stop it?" onclose={() => (confirm = null)}>
    <p>{c.message}</p>
    <p class="small-text muted">Set an emergency stop shortcut on the Settings tab to stop it from any program.</p>
    {#snippet footer()}
      <button
        type="button"
        class="primary"
        onclick={() => {
          const retry = c.retry;
          confirm = null;
          void act(retry);
        }}>Start anyway</button
      >
      <button type="button" onclick={() => (confirm = null)}>Cancel</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .autoclicker {
    max-width: 72rem;
  }

  .profile-switch {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .turn-on {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .turn-on p {
    flex: 1 1 24rem;
    margin: 0;
  }

  .session-bar {
    position: sticky;
    top: 0;
    z-index: var(--z-sticky);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin: var(--space-3) 0;
    padding: var(--space-2) var(--space-3);
    background: var(--surface);
    border: 1px solid var(--success-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .session-bar.recording {
    border-color: var(--danger-border);
  }

  .session-bar.paused {
    border-color: var(--warning-border);
  }

  .pulse {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--success);
    flex: none;
  }

  .recording .pulse {
    background: var(--danger-solid);
  }

  .paused .pulse {
    background: var(--warning);
  }

  .session-text {
    flex: 1 1 16rem;
    margin: 0;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
  }

  .session-text strong {
    margin-right: var(--space-2);
  }

  .automation-tabs {
    display: flex;
    gap: var(--space-1);
    margin: var(--space-4) 0 var(--space-4);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    overflow-y: hidden;
  }

  .automation-tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 34px;
    padding: 0 var(--space-3);
    border: 0;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    white-space: nowrap;
  }

  .automation-tab:hover:not(:disabled) {
    color: var(--text);
    background: transparent;
  }

  .automation-tab[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }

  .automation-tab:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }

  @media (max-width: 760px) {
    .session-bar {
      top: var(--topbar-h);
    }
  }
</style>
