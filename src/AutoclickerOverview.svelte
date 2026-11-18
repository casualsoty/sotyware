<script lang="ts">
  import { clickerSummary, formatClock, formatCount, formatDuration, formatRate, playbackSummary } from "./automationFormat";
  import type { AutomationModel } from "./automationModel.svelte";
  import Icon from "./Icon.svelte";

  // The Autoclicker's dashboard (ADR-050): what runs and how far it got,
  // quick controls, the active profile, the shortcuts in use and the
  // program in front while something runs.

  interface Props {
    model: AutomationModel;
    canStart: boolean;
    onstartclicker: () => void;
    onplay: (id: string | null) => void;
    onrecord: () => void;
    onpause: (paused: boolean) => void;
    onstop: () => void;
    ontab: (tab: "clicker" | "recorder" | "macros" | "profiles" | "settings") => void;
  }

  let { model, canStart, onstartclicker, onplay, onrecord, onpause, onstop, ontab }: Props = $props();

  const status = $derived(model.status);
  const session = $derived(status?.session ?? null);
  const profile = $derived(model.activeProfile);
  const profileMacro = $derived(
    profile?.macroId ? ((model.macros ?? []).find((m) => m.id === profile.macroId) ?? null) : null,
  );
  const state = $derived(session === null ? null : session.kind === "recording" ? session.state : session.progress.state);
  const OUTCOME_TONE = { completed: "success", stopped: "neutral", failsafe: "warning", failed: "danger" } as const;
</script>

<div class="overview">
  <section class="card status" class:running={state === "running"} class:paused={state === "paused"} class:recording={session?.kind === "recording"} aria-labelledby="ov-status">
    <div class="status-head">
      <span class="dot-big" aria-hidden="true"></span>
      <h2 id="ov-status">
        {#if session === null}
          Idle
        {:else if session.kind === "clicker"}
          Clicking
        {:else if session.kind === "playback"}
          Playing “{session.macroName}”
        {:else}
          Recording
        {/if}
      </h2>
      {#if state !== null}
        <span class="badge {state === 'paused' ? 'warning' : state === 'countdown' ? 'info' : session?.kind === 'recording' ? 'danger' : 'success'}">
          {state === "countdown" ? "Starting" : state === "paused" ? "Paused" : "Running"}
        </span>
      {/if}
    </div>

    {#if session !== null && session.kind === "clicker"}
      {@const p = session.progress}
      <dl class="metrics">
        <div><dt>Clicks</dt><dd>{formatCount(p.clicks)}</dd></div>
        <div><dt>Per second</dt><dd>{formatRate(p.cps)}<span class="of">{` / ${formatRate(session.targetCps)}`}</span></dd></div>
        <div><dt>Time</dt><dd>{formatClock(p.elapsedMs)}</dd></div>
        <div><dt>Left</dt><dd>{p.remaining === null ? "Until stopped" : formatCount(p.remaining)}</dd></div>
      </dl>
      {#if p.state === "countdown" && p.countdownMs !== null}
        <p class="small-text" role="status">Starts in {Math.ceil(p.countdownMs / 1000)} s: point where it should click.</p>
      {/if}
      {#if p.skipped > 0}
        <p class="small-text muted">{formatCount(p.skipped)} clicks skipped over sotyware's window.</p>
      {/if}
    {:else if session !== null && session.kind === "playback"}
      {@const p = session.progress}
      <dl class="metrics">
        <div><dt>Repetition</dt><dd>{formatCount(p.repetition)}<span class="of">{p.repetitions === null ? " of ∞" : ` of ${formatCount(p.repetitions)}`}</span></dd></div>
        <div><dt>Action</dt><dd>{formatCount(Math.min(p.action + 1, p.actions))}<span class="of">{` of ${formatCount(p.actions)}`}</span></dd></div>
        <div><dt>Time</dt><dd>{formatClock(p.elapsedMs)}</dd></div>
        <div><dt>Left</dt><dd>{p.remainingMs === null ? "Until stopped" : formatDuration(p.remainingMs)}</dd></div>
      </dl>
      {#if p.progress !== null}
        <progress max="1" value={p.progress} aria-label="Playback progress"></progress>
      {/if}
      <p class="small-text muted now">{p.atPausePoint ? "Paused at a pause point: resume to go on." : p.label}</p>
      {#if p.warning !== null}<p class="callout warning">{p.warning}</p>{/if}
    {:else if session !== null && session.kind === "recording"}
      <dl class="metrics">
        <div><dt>Time</dt><dd>{formatClock(session.elapsedMs)}</dd></div>
        <div><dt>Actions</dt><dd>{formatCount(session.actions)}</dd></div>
      </dl>
    {:else if status?.last}
      {@const l = status.last}
      <p class="callout {OUTCOME_TONE[l.outcome]} last">
        {l.kind === "clicker" ? "Autoclicker" : l.kind === "playback" ? "Playback" : "Recording"}: {l.message}
        <span class="muted">({new Date(l.atMs).toLocaleTimeString()})</span>
      </p>
    {/if}

    <div class="actions">
      {#if session === null}
        <button type="button" class="primary" disabled={!canStart} onclick={onstartclicker}><Icon name="pointer" />Start autoclicker</button>
        <button type="button" disabled={!canStart || profileMacro === null} title={profileMacro === null ? "Choose the profile's macro on the Profiles tab" : undefined} onclick={() => onplay(null)}><Icon name="play" />Play {profileMacro ? `“${profileMacro.name}”` : "the profile's macro"}</button>
        <button type="button" disabled={!canStart || status?.draft !== null} onclick={onrecord}><Icon name="record" />Record</button>
      {:else}
        {#if state === "paused"}
          <button type="button" onclick={() => onpause(false)}><Icon name="play" />Resume</button>
        {:else}
          <button type="button" disabled={state === "countdown"} onclick={() => onpause(true)}><Icon name="pause" />Pause</button>
        {/if}
        <button type="button" class="primary danger" onclick={onstop}><Icon name="stop" />Stop</button>
      {/if}
    </div>
  </section>

  <section class="card" aria-labelledby="ov-profile">
    <h2 id="ov-profile" class="section-title">Profile “{profile?.name ?? "…"}”</h2>
    {#if profile !== null}
      <p class="small-text">{clickerSummary(profile.clicker)}</p>
      <p class="small-text muted">
        Macro: {profileMacro === null ? "none" : `“${profileMacro.name}”, ${playbackSummary(profile.playback ?? { repeat: 1, stopAfterMs: null, speed: 1, startDelayMs: 0, repeatDelayMs: 0, jitterPercent: 0, positioning: "absolute", scaleToScreen: true, section: null }).toLowerCase()}`}
      </p>
      <div class="actions">
        <button type="button" class="small" onclick={() => ontab("clicker")}>Autoclicker settings</button>
        <button type="button" class="small" onclick={() => ontab("profiles")}>Profiles</button>
      </div>
    {/if}
  </section>

  <section class="card" aria-labelledby="ov-keys">
    <h2 id="ov-keys" class="section-title">Shortcuts</h2>
    {#if status === null || status.hotkeys.length === 0}
      <p class="small-text muted">None set. <button type="button" class="link" onclick={() => ontab("settings")}>Set shortcuts</button></p>
    {:else}
      <ul class="keys">
        {#each status.hotkeys as h (h.label + h.chord)}
          <li>
            <kbd>{h.keys}</kbd>
            <p class="what">{h.label}</p>
            {#if !status.running}
              <p class="subtle small-text">off</p>
            {:else if h.registered}
              <span class="ok" title="Registered"><Icon name="check" size={14} /><span class="visually-hidden">registered</span></span>
            {:else}
              <span class="warn" title={h.error ?? ""}><Icon name="alert" size={14} /><span class="visually-hidden">{h.error}</span></span>
            {/if}
          </li>
        {/each}
      </ul>
      {#if status.running && status.hotkeys.some((h) => !h.registered)}
        <p class="small-text text-danger">Some shortcuts could not be registered. <button type="button" class="link" onclick={() => ontab("settings")}>See why</button></p>
      {/if}
    {/if}
  </section>

  {#if session !== null}
    <section class="card" aria-labelledby="ov-window">
      <h2 id="ov-window" class="section-title">Program in front</h2>
      {#if status?.activeWindow}
        {@const w = status.activeWindow}
        <p class="small-text"><strong>{w.title || "Untitled window"}</strong></p>
        <p class="small-text muted">{w.process || "Unknown program"} · {w.client.width} × {w.client.height} at {w.client.x}, {w.client.y}</p>
      {:else}
        <p class="small-text muted">Unknown.</p>
      {/if}
    </section>
  {/if}
</div>

<style>
  .overview {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(19rem, 1fr));
    gap: var(--space-4);
    align-items: start;
  }

  .status {
    grid-column: 1 / -1;
  }

  .status.running {
    border-color: var(--success-border);
  }

  .status.recording {
    border-color: var(--danger-border);
  }

  .status.paused {
    border-color: var(--warning-border);
  }

  .status-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .status-head h2 {
    overflow-wrap: anywhere;
  }

  .dot-big {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-subtle);
    flex: none;
  }

  .running .dot-big {
    background: var(--success);
  }

  .recording .dot-big {
    background: var(--danger-solid);
  }

  .paused .dot-big {
    background: var(--warning);
  }

  .metrics {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: var(--space-3);
    margin: var(--space-4) 0 var(--space-3);
  }

  .metrics div {
    padding: var(--space-2) var(--space-3);
    background: var(--surface-2);
    border-radius: var(--radius-md);
  }

  .metrics dt {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .metrics dd {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    font-variant-numeric: tabular-nums;
  }

  .of {
    font-size: var(--text-sm);
    font-weight: var(--weight-regular);
    color: var(--text-muted);
  }

  .now {
    margin-top: var(--space-2);
  }

  .last {
    margin-top: var(--space-3);
  }

  .keys {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .keys li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }

  .what {
    flex: 1;
    min-width: 0;
  }

  .ok {
    color: var(--success);
    display: inline-flex;
  }

  .warn {
    color: var(--warning);
    display: inline-flex;
  }

  /* Text in a flex box is a <p>: WebKitGTK leaves a <span> flex item's
     text out of the accessibility tree (frontend.md). No margin, as the
     <span> had none. */
  .keys li > p {
    margin: 0;
  }
</style>
