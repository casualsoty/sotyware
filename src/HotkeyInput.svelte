<script lang="ts">
  import type { KeyView } from "./automation";
  import { chordFromEvent, chordProblem, formatChord, META_NAME } from "./automationFormat";

  // A shortcut field: click it, press the keys, done. Escape cancels,
  // Backspace or Delete clears. The keys are read only while it listens,
  // and never reach the shell's own shortcuts meanwhile. Whether the
  // system accepts the shortcut is Rust's to say (the status shows it).

  interface Props {
    value: string | null;
    keys: readonly KeyView[];
    /** What the shortcut does, for its accessible name. */
    label: string;
    onchange: (chord: string | null) => void;
  }

  let { value, keys, label, onchange }: Props = $props();

  let listening = $state(false);
  let held = $state("");
  let problem = $state<string | null>(null);

  function start(): void {
    listening = true;
    held = "";
    problem = null;
  }

  function stop(): void {
    listening = false;
    held = "";
  }

  function onkeydown(e: KeyboardEvent): void {
    if (!listening) {
      if (e.key === "Enter" || e.key === " ") {
        return;
      }
      if ((e.key === "Backspace" || e.key === "Delete") && value !== null) {
        e.preventDefault();
        onchange(null);
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
    if (plain && e.key === "Escape") {
      stop();
      return;
    }
    if (plain && (e.key === "Backspace" || e.key === "Delete")) {
      stop();
      onchange(null);
      return;
    }
    const chord = chordFromEvent(e, keys);
    if (chord === null) {
      held = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && META_NAME]
        .filter(Boolean)
        .join(" + ");
      return;
    }
    const why = chordProblem(chord, keys);
    if (why !== null) {
      problem = why;
      held = "";
      return;
    }
    stop();
    onchange(chord);
  }

  function onkeyup(e: KeyboardEvent): void {
    if (listening) {
      e.preventDefault();
      e.stopPropagation();
      held = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && META_NAME]
        .filter(Boolean)
        .join(" + ");
    }
  }
</script>

<span class="hotkey">
  <button
    type="button"
    class="field-button"
    class:listening
    aria-label="{label}: {listening ? 'press the keys, Escape cancels' : formatChord(value, keys)}"
    onclick={() => (listening ? stop() : start())}
    {onkeydown}
    {onkeyup}
    onblur={stop}
  >
    {#if listening}
      <span class="prompt">{held === "" ? "Press keys…" : `${held} + …`}</span>
    {:else if value === null}
      <span class="none">Not set</span>
    {:else}
      <kbd>{formatChord(value, keys)}</kbd>
    {/if}
  </button>
  {#if problem !== null}
    <!-- Named by its own text: inside a table cell (the Settings tab, the
         editor) WebView2 gives an alert no name from its content, and
         Narrator reads a live region's name (frontend.md). -->
    <span class="problem-text" role="alert" aria-label={problem}>{problem}</span>
  {/if}
</span>

<style>
  .hotkey {
    display: inline-flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .field-button {
    justify-content: flex-start;
    min-width: 11rem;
    font-weight: var(--weight-regular);
  }

  .field-button.listening {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }

  .prompt {
    color: var(--accent-text);
  }

  .none {
    color: var(--text-subtle);
  }

  .problem-text {
    font-size: var(--text-xs);
    color: var(--danger);
    max-width: 22rem;
  }
</style>
