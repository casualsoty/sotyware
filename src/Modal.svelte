<script lang="ts">
  import { onDestroy, onMount, type Snippet } from "svelte";

  // A modal dialog over the browser: focus moves into it when it opens and
  // back to where it was when it closes; Escape and the backdrop close it
  // (unless `locked`, while something runs); Tab stays inside.

  interface Props {
    title: string;
    onclose: () => void;
    locked?: boolean;
    wide?: boolean;
    /** A details sheet along the right edge (properties) instead of a
     * centred dialog; the same dialog semantics. */
    side?: boolean;
    children: Snippet;
    footer?: Snippet;
  }

  let { title, onclose, locked = false, wide = false, side = false, children, footer }: Props = $props();
  let box: HTMLDivElement | undefined = $state();
  let before: Element | null = null;
  const id = `modal-${Math.random().toString(36).slice(2)}`;

  function focusables(): HTMLElement[] {
    return box === undefined
      ? []
      : [...box.querySelectorAll<HTMLElement>("button, [href], input, select, textarea, [tabindex]:not([tabindex='-1'])")].filter(
          (el) => !el.hasAttribute("disabled"),
        );
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.stopPropagation();
      e.preventDefault();
      if (!locked) {
        onclose();
      }
      return;
    }
    if (e.key === "Tab") {
      const all = focusables();
      const first = all[0];
      const last = all.at(-1);
      if (first === undefined || last === undefined) {
        return;
      }
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  onMount(() => {
    before = document.activeElement;
    // The first control of the content (or footer), not the header's Close.
    const inner = focusables().filter((el) => !el.classList.contains("close"));
    const target = box?.querySelector<HTMLElement>("[data-autofocus]") ?? inner[0] ?? focusables()[0] ?? box;
    target?.focus();
  });

  onDestroy(() => {
    if (before instanceof HTMLElement && before.isConnected) {
      before.focus();
    }
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="backdrop" class:side onclick={() => (locked ? undefined : onclose())}>
  <div
    class="modal"
    class:wide
    class:side
    role="dialog"
    aria-modal="true"
    aria-labelledby={id}
    tabindex="-1"
    bind:this={box}
    {onkeydown}
    onclick={(e) => e.stopPropagation()}
  >
    <div class="head">
      <h4 {id}>{title}</h4>
      {#if !locked}
        <button type="button" class="ghost icon small close" aria-label="Close" onclick={onclose}
          ><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"
            ><path d="M6.5 6.5l11 11M17.5 6.5l-11 11" /></svg
          ></button
        >
      {/if}
    </div>
    <div class="content">{@render children()}</div>
    {#if footer}
      <div class="footer">{@render footer()}</div>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: var(--overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-dialog);
    padding: var(--space-4);
    animation: fade-in var(--dur) var(--ease);
  }

  .backdrop.side {
    justify-content: flex-end;
    align-items: stretch;
    padding: var(--space-2);
  }

  .modal {
    background: var(--surface-raised);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-lg);
    padding: var(--space-5) var(--space-6) var(--space-5);
    width: min(32rem, 100%);
    max-height: calc(100vh - 2rem);
    display: flex;
    flex-direction: column;
    animation: pop-in var(--dur) var(--ease);
  }

  .modal.wide {
    width: min(48rem, 100%);
  }

  .modal.side {
    width: min(30rem, 100%);
    max-height: none;
    border-radius: var(--radius-lg);
    animation: slide-in var(--dur) var(--ease);
  }

  .modal:focus {
    outline: none;
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }

  h4 {
    flex: 1;
    margin: 0;
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    overflow-wrap: anywhere;
  }

  .close {
    margin: -4px -8px 0 0;
    color: var(--text-muted);
  }

  .content {
    overflow: auto;
    flex: 1;
    font-size: var(--text-md);
    margin: 0 calc(-1 * var(--space-2));
    padding: 0 var(--space-2);
  }

  .footer {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: flex-end;
    margin-top: var(--space-5);
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }

  @keyframes pop-in {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.985);
    }
  }

  @keyframes slide-in {
    from {
      opacity: 0;
      transform: translateX(12px);
    }
  }
</style>
