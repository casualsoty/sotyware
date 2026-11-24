<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "./Icon.svelte";
  import { type AccountInfo } from "./cloud";
  import {
    cancelItemsDownload,
    downloadItems,
    retryItemsDownload,
    type BulkEvent,
    type BulkFile,
    type BulkItem,
    type BulkStarted,
    type BulkSummary,
    type ItemRef,
  } from "./cloudFiles";
  import { describeOperationError, describeOperationFailure } from "./cloudMessages";
  import { formatCount, formatSize, shownName } from "./format";

  // Downloads several files and whole folders (ADR-026): Rust opens the
  // folder dialog, keeps the path, downloads into a hidden staging folder
  // and moves each item into place once all its files are verified, never
  // replacing anything. This panel shows what the events say.

  interface Props {
    account: AccountInfo;
    bucket: string;
    busy?: boolean;
  }

  let { account, bucket, busy = $bindable(false) }: Props = $props();

  interface Batch {
    id: string | null;
    folderName: string;
    items: BulkItem[];
    files: BulkFile[];
    summary: BulkSummary | null;
    choosing: boolean;
    cancel: boolean;
    error: unknown;
  }

  let batch = $state<Batch | null>(null);
  let seen = new Map<string, number>();
  let early: BulkEvent[] = [];
  let destroyed = false;

  $effect(() => {
    busy = batch !== null && batch.summary === null && batch.error === null;
  });

  const totals = $derived.by(() => {
    if (batch === null) {
      return { done: 0, total: 0, files: 0, filesDone: 0 };
    }
    let done = 0;
    let total = 0;
    let filesDone = 0;
    for (const f of batch.files) {
      total += f.size;
      done += f.state === "completed" ? f.size : f.bytes;
      if (f.state === "completed") filesDone++;
    }
    return { done, total, files: batch.files.length, filesDone };
  });

  const current = $derived(batch?.files.find((f) => f.state === "downloading") ?? null);

  function onEvent(e: BulkEvent): void {
    if (destroyed || batch === null) {
      return;
    }
    if (batch.id === null) {
      early.push(e);
      return;
    }
    if (e.batch !== batch.id) {
      return;
    }
    if (e.seq <= (seen.get(e.batch) ?? 0)) {
      return;
    }
    seen.set(e.batch, e.seq);
    if (e.type === "file") {
      batch.files[e.file.index] = e.file;
    } else if (e.type === "item") {
      batch.items[e.item.index] = e.item;
    } else {
      batch.summary = e.summary;
      batch.cancel = false;
    }
  }

  function adopt(started: BulkStarted): void {
    if (batch === null) {
      return;
    }
    batch.id = started.batch;
    batch.folderName = started.folderName;
    batch.items = started.items;
    batch.files = started.files;
    batch.choosing = false;
    const pending = early;
    early = [];
    for (const e of pending) {
      onEvent(e);
    }
  }

  function onInvalid(e: Error): void {
    if (batch !== null) {
      batch.error = e;
    }
  }

  /** Asks for a folder and downloads `items` into it. */
  export async function start(items: ItemRef[]): Promise<void> {
    if (busy || items.length === 0) {
      return;
    }
    const previous = batch;
    batch = { id: null, folderName: "", items: [], files: [], summary: null, choosing: true, cancel: false, error: null };
    early = [];
    try {
      const started = await downloadItems(account.id, bucket, items, onEvent, onInvalid);
      if (started === null) {
        batch = previous;
        return;
      }
      if (destroyed) {
        void cancelItemsDownload(started.batch).catch(() => undefined);
        return;
      }
      adopt(started);
    } catch (e) {
      if (batch !== null) {
        batch.choosing = false;
        batch.error = e;
      }
    }
  }

  async function cancel(): Promise<void> {
    if (batch?.id == null || batch.cancel) {
      return;
    }
    batch.cancel = true;
    try {
      await cancelItemsDownload(batch.id);
    } catch (e) {
      if (batch !== null) {
        batch.cancel = false;
        batch.error = e;
      }
    }
  }

  async function retry(): Promise<void> {
    const old = batch;
    if (old?.id == null || old.summary === null || !old.summary.retryable) {
      return;
    }
    batch = { id: null, folderName: old.folderName, items: [], files: [], summary: null, choosing: false, cancel: false, error: null };
    early = [];
    try {
      adopt(await retryItemsDownload(old.id, onEvent, onInvalid));
    } catch (e) {
      if (batch !== null) {
        batch.error = e;
      }
    }
  }

  function dismiss(): void {
    if (!busy) {
      batch = null;
    }
  }

  onDestroy(() => {
    destroyed = true;
    if (batch?.id != null && batch.summary === null) {
      void cancelItemsDownload(batch.id).catch(() => undefined);
    }
  });

  function summaryText(s: BulkSummary): string {
    const parts = [`${s.itemsSaved} saved`];
    if (s.itemsFailed > 0) parts.push(`${s.itemsFailed} failed`);
    if (s.itemsCancelled > 0) parts.push(`${s.itemsCancelled} not saved (cancelled)`);
    const head =
      s.state === "completed"
        ? "Download finished"
        : s.state === "partial"
          ? "Download finished with problems"
          : s.state === "cancelled"
            ? "Download cancelled"
            : "Download stopped after an error";
    return `${head}: ${parts.join(", ")} (${formatCount(s.filesCompleted)} files, ${formatSize(s.bytes)}).`;
  }

  function itemLine(i: BulkItem): string {
    switch (i.state) {
      case "pending":
        return "Waiting";
      case "downloading":
        return "Downloading";
      case "saved":
        return i.savedAs !== null && i.savedAs !== i.name
          ? `Saved as “${i.savedAs}” (the name was taken; nothing was replaced)`
          : "Saved";
      case "failed":
        return "Failed; nothing of it was kept";
      case "cancelled":
        return "Not saved (cancelled)";
    }
  }
</script>

{#if batch !== null}
  <div
    class="bulk"
    class:failed={batch.error !== null || (batch.summary !== null && batch.summary.state !== "completed")}
    role="region"
    aria-label="Download of several items"
    data-bulk-state={batch.summary?.state ?? (batch.choosing ? "choosing" : "active")}
  >
    {#if batch.choosing}
      <p role="status">Choose the folder to download into…</p>
    {:else if batch.error !== null && batch.id === null}
      <p role="alert">The download did not start. {describeOperationError(batch.error)}</p>
    {:else}
      <p>
        <strong>Downloading {batch.items.length} item{batch.items.length === 1 ? "" : "s"}</strong>
        into the folder “{batch.folderName}”
      </p>
      {#if batch.summary === null}
        <progress max={Math.max(1, totals.total)} value={totals.done} aria-label="Download progress"></progress>
        <p role="status">
          {#if batch.cancel}
            Cancelling…
          {:else}
            {formatCount(totals.filesDone)} of {formatCount(totals.files)} files · {formatSize(totals.done)} of {formatSize(totals.total)}
            {#if current !== null}· {shownName(current.name)}{/if}
          {/if}
        </p>
      {:else}
        <p role="status" class="result">{summaryText(batch.summary)}</p>
        {#if batch.summary.error !== null}
          <p role="alert">{describeOperationFailure(batch.summary.error)}</p>
        {/if}
      {/if}
      <ul class="items">
        {#each batch.items as item (item.index)}
          <li data-item-state={item.state}>
            <span class="item-icon" aria-hidden="true"><Icon name={item.folder ? "folder" : "file"} size={14} /></span>
            <span class="key">{shownName(item.name)}</span>: {itemLine(item)}
            {#if item.error !== null}<span class="error"> {describeOperationFailure(item.error)}</span>{/if}
          </li>
        {/each}
      </ul>
      {#if batch.files.some((f) => f.state === "failed")}
        <ul class="failures">
          {#each batch.files.filter((f) => f.state === "failed").slice(0, 20) as f (f.index)}
            <li><span class="key">{f.name}</span>: {f.error !== null ? describeOperationFailure(f.error) : "failed"}</li>
          {/each}
        </ul>
      {/if}
      {#if batch.error !== null}
        <p role="alert">{describeOperationError(batch.error)}</p>
      {/if}
    {/if}
    <div class="actions">
      {#if batch.summary === null && batch.id !== null}
        <button type="button" disabled={batch.cancel} onclick={() => void cancel()}>Cancel download</button>
      {:else if !batch.choosing}
        {#if batch.summary?.retryable}
          <button type="button" onclick={() => void retry()}>Download the rest again</button>
        {/if}
        <button type="button" onclick={dismiss}>Dismiss</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .bulk {
    margin: var(--space-3) 0;
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-sm);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .bulk.failed {
    border-color: var(--danger-border);
  }

  .bulk p {
    margin: var(--space-1) 0;
    overflow-wrap: anywhere;
  }

  .bulk progress {
    margin: var(--space-2) 0;
  }

  .items,
  .failures {
    margin: var(--space-2) 0;
    padding-left: var(--space-5);
    max-height: 12rem;
    overflow: auto;
  }

  .failures {
    color: var(--danger);
    font-size: var(--text-xs);
  }

  .key {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .bulk .actions {
    margin-top: var(--space-2);
  }

  .item-icon {
    display: inline-flex;
    vertical-align: -2px;
    color: var(--text-subtle);
  }
</style>
