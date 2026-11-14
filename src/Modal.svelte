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
    children: Snippet;
    footer?: Snippet;
  }

  let { title, onclose, locked = false, wide = false, children, footer }: Props = $props();
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
    const target = box?.querySelector<HTMLElement>("[data-autofocus]") ?? focusables()[0] ?? box;
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
<div class="backdrop" onclick={() => (locked ? undefined : onclose())}>
  <div
    class="modal"
    class:wide
    role="dialog"
    aria-modal="true"
    aria-labelledby={id}
    tabindex="-1"
    bind:this={box}
    {onkeydown}
    onclick={(e) => e.stopPropagation()}
  >
    <h4 {id}>{title}</h4>
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
    background: rgb(0 0 0 / 35%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 20;
    padding: 1rem;
  }

  .modal {
    background: #fff;
    color: inherit;
    border-radius: 6px;
    box-shadow: 0 8px 30px rgb(0 0 0 / 30%);
    padding: 1rem 1.25rem;
    width: min(32rem, 100%);
    max-height: calc(100vh - 2rem);
    display: flex;
    flex-direction: column;
  }

  .modal.wide {
    width: min(48rem, 100%);
  }

  .modal:focus {
    outline: none;
  }

  h4 {
    margin: 0 0 0.5rem;
    font-size: 1.05em;
    overflow-wrap: anywhere;
  }

  .content {
    overflow: auto;
    flex: 1;
  }

  .footer {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    justify-content: flex-end;
    margin-top: 0.75rem;
  }

  @media (prefers-color-scheme: dark) {
    .modal {
      background: #2a2a30;
    }
  }
</style>
