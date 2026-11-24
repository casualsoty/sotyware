<script lang="ts">
  import { parseBlocks, plainText, type Block, type Inline } from "./noteMarkdown";

  // A note as read in the overlay (ADR-052): its Markdown drawn with
  // Svelte's own elements, never as HTML. Checkboxes toggle the item in
  // the text; links open in the browser through Rust; a click anywhere
  // else starts editing at the line clicked. Passive, it only shows.

  interface Props {
    body: string;
    interactive: boolean;
    ontoggle: (line: number) => void;
    /** Edit at the end of `line`, or at the end of the note (`null`). */
    onedit: (line: number | null) => void;
    onlink: (href: string) => void;
  }

  let { body, interactive, ontoggle, onedit, onlink }: Props = $props();

  const blocks = $derived(parseBlocks(body));

  function clicked(e: MouseEvent): void {
    if (!interactive || e.button !== 0) {
      return;
    }
    const target = e.target instanceof Element ? e.target : null;
    if (target?.closest("a, input, button") || (window.getSelection?.()?.toString() ?? "") !== "") {
      // A link, a checkbox, or text being selected to copy.
      return;
    }
    const at = target?.closest<HTMLElement>("[data-line]");
    onedit(at ? Number(at.dataset["line"]) : null);
  }

  function follow(e: MouseEvent, href: string): void {
    e.preventDefault();
    if (interactive) {
      onlink(href);
    }
  }
</script>

{#snippet inline(nodes: Inline[])}{#each nodes as n, i (i)}{#if n.t === "text"}{n.v}{:else if n.t === "strong"}<strong>{@render inline(n.c)}</strong>{:else if n.t === "em"}<em>{@render inline(n.c)}</em>{:else if n.t === "del"}<del>{@render inline(n.c)}</del>{:else if n.t === "code"}<code>{n.v}</code>{:else if n.t === "link"}<a href={n.href} title={n.href} tabindex={interactive ? 0 : -1} onclick={(e) => follow(e, n.href)}>{@render inline(n.c)}</a>{:else}<br />{/if}{/each}{/snippet}

{#snippet block(b: Block)}
  {#if b.t === "heading"}
    {#if b.level === 1}
      <h2 data-line={b.line}>{@render inline(b.content)}</h2>
    {:else if b.level === 2}
      <h3 data-line={b.line}>{@render inline(b.content)}</h3>
    {:else}
      <h4 data-line={b.line}>{@render inline(b.content)}</h4>
    {/if}
  {:else if b.t === "paragraph"}
    <p data-line={b.line}>{@render inline(b.content)}</p>
  {:else if b.t === "list"}
    {#if b.ordered}
      <ol start={b.start} data-line={b.line}>{@render items(b)}</ol>
    {:else}
      <ul data-line={b.line} class:tasks={b.items.some((item) => item.task !== null)}>{@render items(b)}</ul>
    {/if}
  {:else if b.t === "quote"}
    <blockquote data-line={b.line}>{@render inline(b.content)}</blockquote>
  {:else if b.t === "code"}
    <pre data-line={b.line}><code>{b.text}</code></pre>
  {:else}
    <hr data-line={b.line} />
  {/if}
{/snippet}

{#snippet items(list: Extract<Block, { t: "list" }>)}
  {#each list.items as item (item.line)}
    <li data-line={item.line} class:task={item.task !== null} class:done={item.task === true}>
      {#if item.task !== null}
        <input
          type="checkbox"
          checked={item.task}
          disabled={!interactive}
          aria-label={plainText(item.content) || "Checklist item"}
          onchange={() => ontoggle(item.line)}
        /><p class="task-text">{@render inline(item.content)}</p>
      {:else}
        {@render inline(item.content)}
      {/if}
      {#each item.children as child, c (c)}
        {@render block(child)}
      {/each}
    </li>
  {/each}
{/snippet}

<!-- A click is a shortcut to the Edit button the note header has; the
     keyboard reaches that button and every checkbox and link. A checklist
     item's text is a paragraph: WebKitGTK leaves a grid or flex item
     span's text out of the accessibility tree (frontend.md). -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="note-view" class:interactive onclick={clicked}>
  {#each blocks as b, i (i)}
    {@render block(b)}
  {:else}
    <p class="empty">{interactive ? "Empty note. Click here to write." : "Empty note."}</p>
  {/each}
</div>

<style>
  .note-view {
    min-height: 100%;
    padding: var(--space-2) var(--space-3) var(--space-4);
    overflow-wrap: anywhere;
    line-height: 1.45;
  }

  .note-view.interactive {
    cursor: text;
  }

  .note-view :global(:is(h2, h3, h4)) {
    margin: var(--space-3) 0 var(--space-1);
    line-height: var(--leading-tight);
    letter-spacing: -0.005em;
  }

  .note-view > :global(:first-child) {
    margin-top: 0;
  }

  h2 {
    font-size: 1.2em;
  }

  h3 {
    font-size: 1.08em;
  }

  h4 {
    color: var(--ov-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 0.82em;
  }

  p,
  blockquote,
  pre {
    margin: 0 0 var(--space-2);
  }

  ul,
  ol {
    margin: 0 0 var(--space-2);
    padding-left: 1.35em;
  }

  ul ul,
  ul ol,
  ol ul,
  ol ol {
    margin-bottom: 0;
  }

  li {
    margin: 0.12em 0;
  }

  li::marker {
    color: var(--ov-subtle);
  }

  ul.tasks {
    padding-left: 0.2em;
    list-style: none;
  }

  li.task {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    column-gap: 0.5em;
    align-items: start;
  }

  li.task > :global(:is(ul, ol)) {
    grid-column: 2;
  }

  li.task input {
    width: 1.05em;
    height: 1.05em;
    margin: 0.2em 0 0;
    accent-color: var(--ov-accent);
    cursor: pointer;
  }

  li.task input:disabled {
    cursor: default;
    opacity: 1;
  }

  .task-text {
    margin: 0;
  }

  li.done > .task-text {
    color: var(--ov-subtle);
    text-decoration: line-through;
    text-decoration-color: rgb(255 255 255 / 35%);
  }

  blockquote {
    padding: 0.1em 0 0.1em 0.7em;
    border-left: 3px solid var(--ov-accent);
    color: var(--ov-muted);
  }

  pre {
    padding: 0.4em 0.6em;
    border-radius: var(--radius-sm);
    background: rgb(0 0 0 / 30%);
    white-space: pre-wrap;
  }

  code {
    padding: 0.05em 0.3em;
    border-radius: 4px;
    background: rgb(255 255 255 / 9%);
    font-size: 0.9em;
  }

  pre code {
    padding: 0;
    background: none;
  }

  hr {
    margin: var(--space-3) 0;
    border-top-color: var(--ov-border);
  }

  a {
    color: var(--ov-accent);
    text-underline-offset: 2px;
  }

  del {
    color: var(--ov-subtle);
  }

  .empty {
    color: var(--ov-subtle);
    font-style: italic;
  }
</style>
