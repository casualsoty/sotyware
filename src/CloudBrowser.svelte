<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import {
    CloudCommandError,
    cancelDownload,
    downloadFile,
    listFolder,
    retryDownload,
    type AccountInfo,
    type Breadcrumb,
    type DownloadEvent,
    type DownloadStarted,
    type FileEntry,
    type FolderEntry,
    type IntegrityCheck,
  } from "./cloud";
  import { FIXED_BY_EDITING, describeError } from "./cloudMessages";

  // Browser of one account's bucket, one folder level and one page at a
  // time. Folder prefixes are only ever ones Rust returned (or "" for the
  // root): the page never joins names into a prefix, so keys are kept
  // exactly. Files can be downloaded one at a time: Rust opens the save
  // dialog, so the page never names a local path. Nothing here uploads,
  // renames or deletes.

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

  /** The one download this browser shows (none, or the latest). */
  interface Download {
    /** Which start this is; messages of an older one are ignored. */
    seq: number;
    /** Known once Rust started it (from its reply or its first message). */
    transfer: string | null;
    file: FileEntry;
    /** The chosen file's name, else the remote name until it is known. */
    fileName: string;
    state: "choosing" | "active" | "completed" | "cancelled" | "failed";
    bytes: number;
    total: number | null;
    integrity: IntegrityCheck | null;
    error: unknown;
    cancel: "requested" | "finishing" | null;
  }

  let download = $state<Download | null>(null);
  let downloadSeq = 0;
  const downloadBusy = $derived(download !== null && (download.state === "choosing" || download.state === "active"));
  const percent = $derived(
    download === null || download.total === null
      ? null
      : download.total === 0
        ? 100
        : Math.min(100, Math.floor((download.bytes / download.total) * 100)),
  );

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

  /** The current download, if `seq` is still it. */
  function current(seq: number): Download | null {
    return !destroyed && download !== null && download.seq === seq ? download : null;
  }

  function onDownloadEvent(seq: number, event: DownloadEvent): void {
    const d = current(seq);
    // Another transfer's message (an older start, or one this page no longer
    // shows): ignore it.
    if (d === null || (d.transfer !== null && d.transfer !== event.transfer)) {
      return;
    }
    if (d.state === "completed" || d.state === "cancelled" || d.state === "failed") {
      return;
    }
    d.transfer = event.transfer;
    d.bytes = event.bytes;
    d.total = event.total;
    if (event.state === "active") {
      d.state = "active";
      return;
    }
    d.state = event.state;
    d.integrity = event.integrity;
    d.error = event.error === null ? null : new CloudCommandError(event.error.kind, event.error.message);
    d.cancel = null;
  }

  function adopt(seq: number, started: DownloadStarted): void {
    const d = current(seq);
    if (d === null) {
      // The page went away meanwhile: nobody can see or cancel it.
      if (destroyed) {
        void cancelDownload(started.transfer).catch(() => undefined);
      }
      return;
    }
    if (d.transfer !== null && d.transfer !== started.transfer) {
      return;
    }
    d.transfer = started.transfer;
    d.fileName = started.fileName;
    if (d.state === "choosing") {
      d.state = "active";
    }
  }

  function begin(file: FileEntry): number {
    const seq = ++downloadSeq;
    download = {
      seq,
      transfer: null,
      file,
      fileName: shownName(file.name),
      state: "choosing",
      bytes: 0,
      total: null,
      integrity: null,
      error: null,
      cancel: null,
    };
    return seq;
  }

  function failStart(seq: number, e: unknown): void {
    const d = current(seq);
    if (d !== null && d.state !== "completed" && d.state !== "cancelled" && d.state !== "failed") {
      d.state = "failed";
      d.error = e;
    }
  }

  async function startDownload(file: FileEntry): Promise<void> {
    if (downloadBusy) {
      return;
    }
    const seq = begin(file);
    try {
      const started = await downloadFile(account.id, bucket, file.key, (e) => onDownloadEvent(seq, e));
      if (started === null) {
        // The save dialog was cancelled: nothing started.
        if (current(seq)?.state === "choosing") {
          download = null;
        }
        return;
      }
      adopt(seq, started);
    } catch (e) {
      if (errorKind(e) === "unknown-account") {
        onclose(`${account.label} no longer exists; it was removed or changed elsewhere.`);
        return;
      }
      failStart(seq, e);
    }
  }

  async function retryFailed(): Promise<void> {
    const failed = download;
    if (failed === null || failed.state !== "failed") {
      return;
    }
    if (failed.transfer === null) {
      // It never started (for example the account changed): ask again.
      await startDownload(failed.file);
      return;
    }
    const previous = failed.transfer;
    const seq = begin(failed.file);
    const d = current(seq);
    if (d !== null) {
      d.fileName = failed.fileName;
      d.state = "active";
    }
    try {
      adopt(seq, await retryDownload(previous, (e) => onDownloadEvent(seq, e)));
    } catch (e) {
      failStart(seq, e);
    }
  }

  async function cancelActive(): Promise<void> {
    const d = download;
    if (d === null || d.state !== "active" || d.transfer === null || d.cancel !== null) {
      return;
    }
    const seq = d.seq;
    d.cancel = "requested";
    try {
      const outcome = await cancelDownload(d.transfer);
      const now = current(seq);
      if (now !== null && now.state === "active") {
        now.cancel = outcome === "finishing" ? "finishing" : "requested";
      }
    } catch (e) {
      const now = current(seq);
      if (now !== null && now.state === "active") {
        now.cancel = null;
        now.error = e;
      }
    }
  }

  function dismissDownload(): void {
    if (!downloadBusy) {
      download = null;
    }
  }

  function integrityText(check: IntegrityCheck | null): string {
    return check === "sha256-verified"
      ? "The SHA-256 checksum stored with the file matched."
      : "The size matched. This file has no SHA-256 checksum stored by sotyware, so its content could not be verified.";
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
    // Leaving the browser cancels a running download: no page would show it.
    const d = download;
    if (d !== null && d.state === "active" && d.transfer !== null) {
      void cancelDownload(d.transfer).catch(() => undefined);
    }
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

  {#if download !== null}
    <div
      class="download"
      class:failed={download.state === "failed"}
      data-download-state={download.state}
      data-transfer={download.transfer ?? ""}
      aria-label="Download"
      role="region"
    >
      <p class="download-name"><strong>{download.fileName}</strong></p>
      {#if download.state === "choosing"}
        <p role="status">Choose where to save the file…</p>
      {:else if download.state === "active"}
        {#if percent !== null}
          <progress max="100" value={percent} aria-label={`Downloading ${download.fileName}`}>{percent}%</progress>
        {/if}
        <p role="status" class="download-progress">
          {#if download.cancel === "requested"}
            Cancelling…
          {:else if download.cancel === "finishing"}
            Almost done; the file is being saved and cannot be cancelled any more.
          {:else if download.total !== null}
            {percent}% · {formatSize(download.bytes)} of {formatSize(download.total)}
          {:else}
            {formatSize(download.bytes)} downloaded (total size unknown)
          {/if}
        </p>
        {#if download.error !== null}
          <p role="alert">Could not cancel: {describeError(download.error)}</p>
        {/if}
        <div class="actions">
          <button type="button" disabled={download.transfer === null || download.cancel !== null} onclick={() => void cancelActive()}>
            Cancel download
          </button>
        </div>
      {:else if download.state === "completed"}
        <p role="status" class="download-result">
          Downloaded {formatSize(download.bytes)}. {integrityText(download.integrity)}
        </p>
        <div class="actions">
          <button type="button" onclick={dismissDownload}>Dismiss</button>
        </div>
      {:else if download.state === "cancelled"}
        <p role="status" class="download-result">Download cancelled. Nothing was saved.</p>
        <div class="actions">
          <button type="button" onclick={dismissDownload}>Dismiss</button>
        </div>
      {:else}
        <div role="alert" class="download-result">
          <p><strong>Download failed. Nothing was saved.</strong></p>
          <p>{describeError(download.error)}</p>
        </div>
        <div class="actions">
          <button type="button" onclick={() => void retryFailed()}>Try download again</button>
          {#if fixedByEditing(download.error)}
            <button type="button" onclick={onedit}>Edit account</button>
          {/if}
          <button type="button" onclick={dismissDownload}>Dismiss</button>
        </div>
      {/if}
    </div>
  {/if}

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
          <th scope="col"><span class="visually-hidden">Actions</span></th>
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
            <td></td>
          </tr>
        {/each}
        {#each files as file (file.key)}
          <tr class="file" data-key={file.key}>
            <td class="name">{shownName(file.name)}</td>
            <td class="num" title={`${file.size.toLocaleString()} bytes`}>{formatSize(file.size)}</td>
            <td>{formatDate(file.lastModified)}</td>
            <td class="row-actions">
              <button
                type="button"
                class="download-action"
                disabled={downloadBusy}
                aria-label={`Download ${shownName(file.name)}`}
                title={downloadBusy ? "Another download is running" : "Download this file"}
                onclick={() => void startDownload(file)}>Download</button
              >
            </td>
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

  .row-actions {
    text-align: right;
    white-space: nowrap;
  }

  .row-actions button {
    padding: 0.15rem 0.6rem;
    font-size: 0.9em;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .download {
    margin: 0.75rem 0;
    padding: 0.5rem 1rem;
    border-left: 3px solid #1565c0;
    background: rgb(0 0 0 / 4%);
  }

  .download.failed {
    border-left-color: #c62828;
  }

  .download p {
    margin: 0.25rem 0;
    overflow-wrap: anywhere;
  }

  .download-name {
    white-space: pre-wrap;
  }

  .download progress {
    width: 100%;
    max-width: 28rem;
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
    .problem,
    .download {
      background: rgb(255 255 255 / 6%);
    }

    .download {
      border-left-color: #90caf9;
    }
  }
</style>
