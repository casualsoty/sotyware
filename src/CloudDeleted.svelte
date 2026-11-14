<script lang="ts">
  import { onMount } from "svelte";
  import { type AccountInfo } from "./cloud";
  import { listDeleted, type DeletedEntry, type RestoreRef } from "./cloudFiles";
  import { describeError } from "./cloudMessages";
  import { formatSize, shownName } from "./format";

  // Deleted items under the folder shown (ADR-026): keys whose latest
  // version is a delete marker, read from the provider's version listing.
  // Only those with an earlier version can be restored; restoring removes
  // the delete markers (through a reviewed operation). sotyware has no
  // trash of its own: what is listed here is what the provider keeps.

  interface Props {
    account: AccountInfo;
    bucket: string;
    prefix: string;
    /** Bumped by the browser after an operation: reload. */
    reload: number;
    onrestore: (items: RestoreRef[]) => void;
    onclose: () => void;
  }

  let { account, bucket, prefix, reload, onrestore, onclose }: Props = $props();

  let entries = $state<DeletedEntry[]>([]);
  let truncated = $state(false);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let chosen = $state<Set<string>>(new Set());
  let request = 0;

  async function load(): Promise<void> {
    const mine = ++request;
    loading = true;
    error = null;
    try {
      const listing = await listDeleted(account.id, bucket, prefix);
      if (mine !== request) {
        return;
      }
      entries = listing.entries;
      truncated = listing.truncated;
      chosen = new Set([...chosen].filter((k) => listing.entries.some((e) => e.key === k && e.restorable)));
    } catch (e) {
      if (mine === request) {
        error = e;
      }
    } finally {
      if (mine === request) {
        loading = false;
      }
    }
  }

  function toggle(key: string): void {
    const next = new Set(chosen);
    if (next.has(key)) {
      next.delete(key);
    } else {
      next.add(key);
    }
    chosen = next;
  }

  function restore(list: DeletedEntry[]): void {
    onrestore(list.map((e) => ({ key: e.key, markerVersion: e.markerVersion })));
  }

  let lastReload = -1;
  $effect(() => {
    if (reload !== lastReload) {
      lastReload = reload;
      void load();
    }
  });

  onMount(() => {
    lastReload = reload;
    void load();
  });

  function formatDate(rfc3339: string | null): string {
    if (rfc3339 === null) {
      return "—";
    }
    const d = new Date(rfc3339);
    return Number.isNaN(d.getTime()) ? rfc3339 : d.toLocaleString();
  }
</script>

<section class="deleted" aria-labelledby="deleted-heading">
  <h4 id="deleted-heading">Deleted items in {prefix === "" ? `the whole bucket ${bucket}` : prefix}</h4>
  <p class="note">
    Files deleted on a bucket that keeps versions are hidden behind a delete marker; their earlier versions stay (and
    are billed) until removed at the provider. sotyware can restore those. A file without an earlier version, or on a
    bucket without versions, cannot be restored.
  </p>
  <div class="actions">
    <button type="button" onclick={onclose}>← Back to the folder</button>
    <button type="button" onclick={() => void load()}>Refresh</button>
    <button
      type="button"
      disabled={chosen.size === 0}
      onclick={() => restore(entries.filter((e) => chosen.has(e.key)))}>Restore {chosen.size === 0 ? "selected" : chosen.size}</button
    >
  </div>
  {#if loading}
    <p role="status">Loading deleted items…</p>
  {:else if error !== null}
    <div role="alert" class="problem">
      <p>Deleted items cannot be shown.</p>
      <p>{describeError(error)}</p>
    </div>
  {:else if entries.length === 0}
    <p role="status">No deleted items here.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col"><span class="visually-hidden">Select</span></th>
          <th scope="col">Name</th>
          <th scope="col">Folder</th>
          <th scope="col">Deleted</th>
          <th scope="col" class="num">Size</th>
          <th scope="col">Can be restored</th>
          <th scope="col"><span class="visually-hidden">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        {#each entries as e (e.key)}
          <tr data-key={e.key}>
            <td>
              <input
                type="checkbox"
                aria-label={`Select ${shownName(e.name)}`}
                disabled={!e.restorable}
                checked={chosen.has(e.key)}
                onchange={() => toggle(e.key)}
              />
            </td>
            <td class="key">{shownName(e.name)}</td>
            <td class="key">{e.parent === "" ? "(root)" : e.parent}</td>
            <td>{formatDate(e.deletedAt)}</td>
            <td class="num">{e.size === null ? "—" : formatSize(e.size)}</td>
            <td>
              {#if e.restorable}
                Yes ({e.versions} version{e.versions === 1 ? "" : "s"} kept)
              {:else}
                No: no earlier version is kept
              {/if}
            </td>
            <td>
              {#if e.restorable}
                <button type="button" aria-label={`Restore ${shownName(e.name)}`} onclick={() => restore([e])}>Restore</button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    {#if truncated}
      <p class="note">Only the first 5,000 versions were read; open a subfolder to see the rest.</p>
    {/if}
  {/if}
</section>

<style>
  .deleted {
    margin-top: 0.75rem;
  }

  h4 {
    margin: 0.5rem 0;
    overflow-wrap: anywhere;
  }

  .note {
    color: #4a4a52;
    font-size: 0.9em;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin: 0.5rem 0;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.95em;
  }

  th,
  td {
    text-align: left;
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid rgb(0 0 0 / 10%);
    vertical-align: top;
  }

  .num {
    text-align: right;
  }

  .key {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .problem {
    border-left: 3px solid #c62828;
    padding: 0.5rem 1rem;
    background: rgb(0 0 0 / 4%);
  }

  button {
    font: inherit;
    padding: 0.35rem 0.8rem;
    cursor: pointer;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (prefers-color-scheme: dark) {
    .note {
      color: #b8b8c0;
    }

    th,
    td {
      border-bottom-color: rgb(255 255 255 / 12%);
    }
  }
</style>
