<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    CloudCommandError,
    listFolder,
    type AccountInfo,
    type Breadcrumb,
    type FileEntry,
    type FolderEntry,
  } from "./cloud";
  import { FIXED_BY_EDITING, describeError } from "./cloudMessages";

  // Read-only browser of one account's bucket, one folder level and one page
  // at a time. Folder prefixes are only ever ones Rust returned (or "" for the
  // root): the page never joins names into a prefix, so keys are kept
  // exactly. Nothing here uploads, downloads, renames or deletes.

  interface Props {
    account: AccountInfo;
    /** Back to the account list; `message` says why if it was not the user. */
    onclose: (message: string | null) => void;
    /** Back to the account list with this account's edit form open. */
    onedit: () => void;
  }

  let { account, onclose, onedit }: Props = $props();

  let prefix = $state("");
  let crumbs = $state<Breadcrumb[]>([]);
  // Not $derived: the bucket comes from the props until Rust names it.
  // svelte-ignore state_referenced_locally
  let bucket = $state(account.bucket ?? "");
  let folders = $state<FolderEntry[]>([]);
  let files = $state<FileEntry[]>([]);
  let nextPage = $state<string | null>(null);
  let loading = $state<"folder" | "more" | null>(null);
  let error = $state<{ during: "folder" | "more"; cause: unknown } | null>(null);
  let heading: HTMLHeadingElement | undefined = $state();

  // Bumped by every navigation and refresh: a response for an older one is
  // dropped, so a slow folder can never overwrite a newer one.
  let request = 0;
  let destroyed = false;

  const parentCrumbs = $derived(crumbs.length === 0 ? null : crumbs.slice(0, -1));
  const empty = $derived(loading === null && error === null && folders.length === 0 && files.length === 0);

  function errorKind(e: unknown): string | null {
    return e instanceof CloudCommandError ? e.kind : null;
  }

  function fixedByEditing(e: unknown): boolean {
    return e instanceof CloudCommandError && FIXED_BY_EDITING.has(e.kind);
  }

  /** Lists `target`; `path` is the breadcrumb trail to show until Rust's
   * own arrives (built from existing entries, never by splitting strings). */
  async function open(target: string, path: Breadcrumb[]): Promise<void> {
    const mine = ++request;
    prefix = target;
    crumbs = path;
    folders = [];
    files = [];
    nextPage = null;
    error = null;
    loading = "folder";
    try {
      const listing = await listFolder(account.id, target, null);
      if (mine !== request || destroyed) {
        return;
      }
      bucket = listing.bucket;
      crumbs = listing.breadcrumbs;
      folders = listing.folders;
      files = listing.files;
      nextPage = listing.nextPage;
    } catch (e) {
      if (mine !== request || destroyed) {
        return;
      }
      if (errorKind(e) === "unknown-account") {
        onclose(`${account.label} no longer exists; it was removed or changed elsewhere.`);
        return;
      }
      error = { during: "folder", cause: e };
    } finally {
      if (mine === request && !destroyed) {
        loading = null;
      }
    }
  }

  async function loadMore(): Promise<void> {
    const page = nextPage;
    if (page === null || loading !== null) {
      return;
    }
    const mine = request;
    error = null;
    loading = "more";
    try {
      const listing = await listFolder(account.id, prefix, page);
      if (mine !== request || destroyed) {
        return;
      }
      // A page never repeats an entry; this keeps it so even if a provider did.
      const knownFolders = new Set(folders.map((f) => f.prefix));
      const knownFiles = new Set(files.map((f) => f.key));
      folders = [...folders, ...listing.folders.filter((f) => !knownFolders.has(f.prefix))];
      files = [...files, ...listing.files.filter((f) => !knownFiles.has(f.key))];
      nextPage = listing.nextPage;
    } catch (e) {
      if (mine !== request || destroyed) {
        return;
      }
      if (errorKind(e) === "unknown-account") {
        onclose(`${account.label} no longer exists; it was removed or changed elsewhere.`);
        return;
      }
      error = { during: "more", cause: e };
    } finally {
      if (mine === request && !destroyed) {
        loading = null;
      }
    }
  }

  function refresh(): void {
    void open(prefix, crumbs);
  }

  function openFolder(folder: FolderEntry): void {
    void open(folder.prefix, [...crumbs, { name: folder.name, prefix: folder.prefix }]);
  }

  function openCrumb(index: number): void {
    const crumb = crumbs[index];
    if (crumb) {
      void open(crumb.prefix, crumbs.slice(0, index + 1));
    }
  }

  function goUp(): void {
    if (parentCrumbs !== null) {
      void open(parentCrumbs.at(-1)?.prefix ?? "", parentCrumbs);
    }
  }

  function retry(): void {
    if (error?.during === "more" && errorKind(error.cause) !== "outdated-page") {
      void loadMore();
    } else {
      refresh();
    }
  }

  function shownName(name: string): string {
    return name === "" ? "(no name)" : name;
  }

  const UNITS = ["KiB", "MiB", "GiB", "TiB", "PiB"];

  function formatSize(bytes: number): string {
    if (bytes < 1024) {
      return `${bytes} B`;
    }
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < UNITS.length - 1) {
      value /= 1024;
      unit++;
    }
    return `${value.toFixed(value < 10 ? 1 : 0)} ${UNITS[unit]}`;
  }

  function formatDate(rfc3339: string | null): string {
    if (rfc3339 === null) {
      return "—";
    }
    const date = new Date(rfc3339);
    return Number.isNaN(date.getTime())
      ? rfc3339
      : date.toLocaleString([], { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  }

  onMount(() => {
    heading?.focus();
    void open("", []);
  });

  onDestroy(() => {
    destroyed = true;
  });
</script>

<section class="browser" aria-labelledby="browser-heading" data-browsing={account.id}>
  <button type="button" class="back" onclick={() => onclose(null)}>← Back to accounts</button>
  <h3 id="browser-heading" tabindex="-1" bind:this={heading}>{account.label}</h3>
  <dl class="meta">
    <dt>Bucket</dt>
    <dd class="bucket">{bucket}</dd>
    <dt>Folder</dt>
    <dd class="path">{prefix === "" ? "Root of the bucket" : prefix}</dd>
  </dl>

  <nav aria-label="Folder path" class="crumbs">
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
            <button type="button" class="link" onclick={() => openCrumb(i)}>{shownName(crumb.name)}</button>
          {/if}
        </li>
      {/each}
    </ol>
  </nav>

  <div class="actions">
    <button type="button" disabled={parentCrumbs === null} onclick={goUp}>Up one folder</button>
    <button type="button" onclick={refresh}>Refresh</button>
  </div>

  {#if loading === "folder"}
    <p class="loading" role="status">Loading folder…</p>
  {:else if error?.during === "folder"}
    <div class="problem" role="alert">
      <p><strong>Could not load this folder.</strong></p>
      <p>{describeError(error.cause)}</p>
      <div class="actions">
        <button type="button" onclick={retry}>Try again</button>
        {#if fixedByEditing(error.cause)}
          <button type="button" onclick={onedit}>Edit account</button>
        {/if}
      </div>
    </div>
  {:else if empty}
    <div class="empty" role="status">
      <p><strong>{prefix === "" ? "This bucket is empty." : "This folder is empty."}</strong></p>
      <p>
        Folders exist only as part of file names in cloud storage, so a folder without files shows up only if a folder
        marker was created for it.
      </p>
    </div>
  {:else}
    <table>
      <thead>
        <tr>
          <th scope="col">Name</th>
          <th scope="col" class="num">Size</th>
          <th scope="col">Last modified</th>
        </tr>
      </thead>
      <tbody>
        {#each folders as folder (folder.prefix)}
          <tr class="folder" data-prefix={folder.prefix}>
            <td>
              <button type="button" class="link name" onclick={() => openFolder(folder)}>
                <span aria-hidden="true">📁</span>
                {shownName(folder.name)}
              </button>
            </td>
            <td class="num">Folder</td>
            <td>—</td>
          </tr>
        {/each}
        {#each files as file (file.key)}
          <tr class="file" data-key={file.key}>
            <td class="name">{shownName(file.name)}</td>
            <td class="num" title={`${file.size.toLocaleString()} bytes`}>{formatSize(file.size)}</td>
            <td>{formatDate(file.lastModified)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="count">
      {folders.length + files.length} item{folders.length + files.length === 1 ? "" : "s"} shown{nextPage !== null
        ? "; more are available"
        : ""}.
    </p>
    {#if loading === "more"}
      <p class="loading" role="status">Loading more…</p>
    {:else if nextPage !== null && error === null}
      <button type="button" onclick={() => void loadMore()}>Load more</button>
    {/if}
    {#if error?.during === "more"}
      <div class="problem" role="alert">
        <p>Could not load more of this folder. {describeError(error.cause)}</p>
        <button type="button" onclick={retry}>
          {errorKind(error.cause) === "outdated-page" ? "Refresh" : "Try again"}
        </button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .browser {
    margin-top: 0.5rem;
  }

  h3 {
    font-size: 1.05em;
    margin: 0.75rem 0 0.25rem;
  }

  h3:focus {
    outline: none;
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
  }

  .back {
    margin-bottom: 0.25rem;
  }

  .meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.1rem 0.75rem;
    margin: 0.4rem 0;
    color: #6b6b73;
    font-size: 0.9em;
  }

  .meta dd {
    margin: 0;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .crumbs ol {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    list-style: none;
    padding: 0;
    margin: 0.5rem 0;
  }

  .crumbs li + li::before {
    content: "/";
    margin-right: 0.25rem;
    color: #6b6b73;
  }

  .crumbs span,
  .crumbs button {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
    margin-top: 0.5rem;
    font-size: 0.95em;
  }

  th,
  td {
    text-align: left;
    padding: 0.3rem 0.5rem;
    border-bottom: 1px solid rgb(0 0 0 / 10%);
    vertical-align: top;
  }

  th {
    font-weight: 600;
  }

  td.name,
  .name {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .num {
    text-align: right;
    white-space: nowrap;
  }

  .count,
  .loading {
    color: #6b6b73;
    font-size: 0.9em;
  }

  .empty,
  .problem {
    margin: 1rem 0;
    padding: 0.75rem 1rem;
    border-left: 3px solid #8a8a93;
    background: rgb(0 0 0 / 4%);
  }

  .problem {
    border-left-color: #c62828;
  }

  .empty p,
  .problem p {
    margin: 0.25rem 0;
    overflow-wrap: anywhere;
  }

  @media (prefers-color-scheme: dark) {
    button.link {
      color: #90caf9;
    }

    .meta,
    .count,
    .loading,
    .crumbs li + li::before {
      color: #a0a0a8;
    }

    th,
    td {
      border-bottom-color: rgb(255 255 255 / 12%);
    }

    .empty,
    .problem {
      background: rgb(255 255 255 / 6%);
    }
  }
</style>
