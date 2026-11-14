<script lang="ts">
  import { onMount } from "svelte";
  import { listFolder, type AccountInfo, type Breadcrumb, type FolderEntry } from "./cloud";
  import { createFolder } from "./cloudFiles";
  import { describeError, describeOperationError } from "./cloudMessages";
  import { shownName } from "./format";
  import Modal from "./Modal.svelte";

  // Chooses a destination folder for a copy or a move (ADR-026): browses
  // the account's bucket one folder level at a time (only folders shown),
  // and can create a folder (its marker) on the way. Prefixes are only ever
  // ones Rust returned, never joined from names.

  interface Props {
    account: AccountInfo;
    bucket: string;
    title: string;
    confirm: string;
    start: string;
    startCrumbs: Breadcrumb[];
    /** Folders being moved: the destination may not be inside them. */
    moving: string[];
    onchoose: (prefix: string) => void;
    onclose: () => void;
  }

  let { account, bucket, title, confirm, start, startCrumbs, moving, onchoose, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let prefix = $state(start);
  // svelte-ignore state_referenced_locally
  let crumbs = $state<Breadcrumb[]>(startCrumbs);
  let folders = $state<FolderEntry[]>([]);
  let next = $state<string | null>(null);
  let loading = $state(false);
  let error = $state<unknown>(null);
  let creating = $state(false);
  let newName = $state("");
  let createError = $state<unknown>(null);
  let request = 0;

  const inside = $derived(moving.some((m) => prefix.startsWith(m)));

  async function open(target: string, path: Breadcrumb[], page: string | null = null): Promise<void> {
    const mine = ++request;
    if (page === null) {
      prefix = target;
      crumbs = path;
      folders = [];
    }
    loading = true;
    error = null;
    try {
      const listing = await listFolder(account.id, target, page);
      if (mine !== request) {
        return;
      }
      crumbs = listing.breadcrumbs;
      folders = page === null ? listing.folders : [...folders, ...listing.folders];
      next = listing.nextPage;
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

  async function create(): Promise<void> {
    const name = newName;
    createError = null;
    try {
      const made = await createFolder(account.id, bucket, prefix, name);
      newName = "";
      creating = false;
      await open(made.prefix, [...crumbs, { name: made.name, prefix: made.prefix }]);
    } catch (e) {
      createError = e;
    }
  }

  onMount(() => {
    void open(start, startCrumbs);
  });
</script>

<Modal {title} {onclose} wide>
  <div class="picker">
    <nav aria-label="Destination path" class="crumbs">
      <ol>
        <li>
          {#if crumbs.length === 0}
            <span aria-current="location">{bucket}</span>
          {:else}
            <button type="button" class="link" onclick={() => void open("", [])}>{bucket}</button>
          {/if}
        </li>
        {#each crumbs as crumb, i (crumb.prefix)}
          <li>
            {#if i === crumbs.length - 1}
              <span aria-current="location">{shownName(crumb.name)}</span>
            {:else}
              <button type="button" class="link" onclick={() => void open(crumb.prefix, crumbs.slice(0, i + 1))}>{shownName(crumb.name)}</button>
            {/if}
          </li>
        {/each}
      </ol>
    </nav>
    {#if loading && folders.length === 0}
      <p role="status">Loading folders…</p>
    {:else if error !== null}
      <p role="alert">{describeError(error)}</p>
      <button type="button" onclick={() => void open(prefix, crumbs)}>Try again</button>
    {:else}
      <ul class="folders" aria-label="Folders">
        {#each folders as folder (folder.prefix)}
          <li>
            <button type="button" class="link" onclick={() => void open(folder.prefix, [...crumbs, { name: folder.name, prefix: folder.prefix }])}>
              <span aria-hidden="true">📁</span> {shownName(folder.name)}
            </button>
          </li>
        {:else}
          <li class="none">No folders here.</li>
        {/each}
      </ul>
      {#if next !== null}
        <button type="button" disabled={loading} onclick={() => void open(prefix, crumbs, next)}>Load more folders</button>
      {/if}
    {/if}
    {#if creating}
      <form
        class="create"
        onsubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <label>Name of the new folder <input type="text" bind:value={newName} data-autofocus /></label>
        <button type="submit" disabled={newName === ""}>Create</button>
        <button type="button" onclick={() => ((creating = false), (createError = null))}>Cancel</button>
      </form>
      {#if createError !== null}
        <p role="alert">{describeOperationError(createError)}</p>
      {/if}
    {/if}
    {#if inside}
      <p class="warning" role="note">A folder cannot be moved into itself; choose a folder outside it.</p>
    {/if}
  </div>
  {#snippet footer()}
    {#if !creating}
      <button type="button" onclick={() => (creating = true)}>New folder…</button>
    {/if}
    <button type="button" onclick={onclose}>Cancel</button>
    <button type="button" class="primary" disabled={inside || loading} onclick={() => onchoose(prefix)}>
      {confirm} {prefix === "" ? `the root of ${bucket}` : `“${shownName(crumbs.at(-1)?.name ?? prefix)}”`}
    </button>
  {/snippet}
</Modal>

<style>
  .crumbs ol {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    list-style: none;
    padding: 0;
    margin: 0 0 0.5rem;
  }

  .crumbs li + li::before {
    content: "/";
    margin-right: 0.25rem;
    color: #6b6b73;
  }

  .folders {
    list-style: none;
    padding: 0;
    margin: 0;
    max-height: 18rem;
    overflow: auto;
    border: 1px solid rgb(0 0 0 / 12%);
    border-radius: 4px;
  }

  .folders li {
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid rgb(0 0 0 / 6%);
  }

  .folders .none {
    color: #6b6b73;
  }

  button {
    font: inherit;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }

  button.link {
    padding: 0;
    border: none;
    background: none;
    color: #1565c0;
    text-decoration: underline;
    text-align: left;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  button.primary {
    font-weight: 600;
  }

  .create {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: end;
    margin-top: 0.5rem;
  }

  .create label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .warning {
    color: #b71c1c;
  }

  @media (prefers-color-scheme: dark) {
    button.link {
      color: #90caf9;
    }

    .warning {
      color: #ef9a9a;
    }
  }
</style>
