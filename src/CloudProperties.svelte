<script lang="ts">
  import { onMount } from "svelte";
  import { type AccountInfo } from "./cloud";
  import {
    LINK_LIFETIMES,
    PREVIEW_IMAGE_TYPES,
    objectProperties,
    previewFile,
    shareFile,
    type Preview,
    type Properties,
    type ShareLink,
  } from "./cloudFiles";
  import { describeOperationError } from "./cloudMessages";
  import { formatSize, shownName } from "./format";
  import Modal from "./Modal.svelte";

  // A file's properties, a size-limited preview and a temporary share link
  // (ADR-026). The preview is Rust's: at most 256 KiB of text, shown as text
  // (never as HTML or markup), or a raster image of at most 10 MiB that Rust
  // recognised by its first bytes (never SVG), shown as an image only if its
  // type is one of PREVIEW_IMAGE_TYPES. Nothing is executed.

  interface Props {
    account: AccountInfo;
    bucket: string;
    fileKey: string;
    favorite: boolean;
    ondownload: () => void;
    onshowinfolder: (() => void) | null;
    ontogglefavorite: () => void;
    onclose: () => void;
  }

  let { account, bucket, fileKey, favorite, ondownload, onshowinfolder, ontogglefavorite, onclose }: Props = $props();

  let info = $state<Properties | null>(null);
  let propsError = $state<unknown>(null);
  let preview = $state<Preview | null>(null);
  let previewState = $state<"idle" | "loading" | "failed">("idle");
  let previewError = $state<unknown>(null);
  let lifetime = $state<number>(3600);
  let link = $state<ShareLink | null>(null);
  let linkError = $state<unknown>(null);
  let linking = $state(false);
  let copied = $state<"no" | "yes" | "failed">("no");
  let linkInput: HTMLInputElement | undefined = $state();

  const json = $derived.by(() => {
    if (preview?.kind !== "text" || preview.truncated || preview.text === null) {
      return null;
    }
    const name = fileKey.toLowerCase();
    const looks = name.endsWith(".json") || (info?.contentType ?? "").includes("json");
    if (!looks) {
      return null;
    }
    try {
      return JSON.stringify(JSON.parse(preview.text), null, 2);
    } catch {
      return null;
    }
  });

  const imageSource = $derived(
    preview?.kind === "image" &&
      preview.data !== null &&
      (PREVIEW_IMAGE_TYPES as readonly (string | null)[]).includes(preview.mime)
      ? `data:${preview.mime};base64,${preview.data}`
      : null,
  );

  async function loadPreview(): Promise<void> {
    previewState = "loading";
    previewError = null;
    try {
      preview = await previewFile(account.id, bucket, fileKey);
      previewState = "idle";
    } catch (e) {
      previewError = e;
      previewState = "failed";
    }
  }

  async function makeLink(): Promise<void> {
    linking = true;
    linkError = null;
    copied = "no";
    try {
      link = await shareFile(account.id, bucket, fileKey, lifetime);
    } catch (e) {
      linkError = e;
    } finally {
      linking = false;
    }
  }

  async function copyLink(): Promise<void> {
    if (link === null) {
      return;
    }
    try {
      await navigator.clipboard.writeText(link.url);
      copied = "yes";
    } catch {
      copied = "failed";
      linkInput?.select();
    }
  }

  function formatDate(rfc3339: string | null): string {
    if (rfc3339 === null) {
      return "—";
    }
    const d = new Date(rfc3339);
    return Number.isNaN(d.getTime()) ? rfc3339 : d.toLocaleString();
  }

  onMount(async () => {
    try {
      info = await objectProperties(account.id, bucket, fileKey);
      // Rust applies the size limits; it never reads more than allowed.
      void loadPreview();
    } catch (e) {
      propsError = e;
    }
  });
</script>

<Modal title={`Properties of “${shownName(fileKey.split("/").at(-1) ?? fileKey)}”`} {onclose} wide>
  <div class="properties">
    {#if propsError !== null}
      <p role="alert">{describeOperationError(propsError)}</p>
    {:else if info === null}
      <p role="status">Loading properties…</p>
    {:else}
      <dl>
        <dt>Name</dt>
        <dd class="key">{shownName(info.name)}</dd>
        <dt>Location</dt>
        <dd class="key">{bucket} / {info.folder === "" ? "(root)" : info.folder}</dd>
        <dt>Object key</dt>
        <dd class="key">{info.key}</dd>
        <dt>Size</dt>
        <dd>{formatSize(info.size)} ({info.size.toLocaleString()} bytes)</dd>
        <dt>Last modified</dt>
        <dd>{formatDate(info.lastModified)}</dd>
        <dt>Content type</dt>
        <dd>{info.contentType ?? "—"}</dd>
        <dt>SHA-256 (stored by sotyware)</dt>
        <dd class="key">{info.sha256 ?? "None: not uploaded by sotyware; downloads check the size only"}</dd>
        <dt>ETag</dt>
        <dd class="key">{info.eTag ?? "—"}</dd>
        {#if info.versionId !== null}
          <dt>Version</dt>
          <dd class="key">{info.versionId}</dd>
        {/if}
        {#if info.storageClass !== null}
          <dt>Storage class</dt>
          <dd>{info.storageClass}</dd>
        {/if}
        {#if info.cacheControl !== null}
          <dt>Cache-Control</dt>
          <dd class="key">{info.cacheControl}</dd>
        {/if}
        {#if info.contentEncoding !== null}
          <dt>Content-Encoding</dt>
          <dd class="key">{info.contentEncoding}</dd>
        {/if}
        {#if info.contentDisposition !== null}
          <dt>Content-Disposition</dt>
          <dd class="key">{info.contentDisposition}</dd>
        {/if}
        {#each info.metadata.filter(([k]) => k !== "sotyware-sha256") as [k, v] (k)}
          <dt class="key">Metadata: {k}</dt>
          <dd class="key">{v}</dd>
        {/each}
      </dl>
    {/if}

    <section class="preview" aria-label="Preview">
      <h5>Preview</h5>
      {#if previewState === "loading"}
        <p role="status">Loading preview…</p>
      {:else if previewState === "failed"}
        <p role="alert">No preview: {describeOperationError(previewError)}</p>
      {:else if preview !== null}
        {#if imageSource !== null}
          <img src={imageSource} alt={`Preview of ${shownName(info?.name ?? "")}`} />
        {:else if preview.kind === "text"}
          <pre class="text">{json ?? preview.text}</pre>
          {#if preview.truncated}
            <p class="note">Only the first 256 KiB are shown. Download the file to see all of it.</p>
          {/if}
        {:else if preview.kind === "too-large"}
          <p class="note">This image is larger than 10 MiB, so it is not previewed. Download it to view it.</p>
        {:else}
          <p class="note">No preview for this kind of file. Download it to open it with another program.</p>
        {/if}
      {/if}
    </section>

    <section class="share" aria-label="Share link">
      <h5>Temporary download link</h5>
      <p class="note">
        Anyone who has the link can download this file until it expires; it cannot be withdrawn earlier (except by
        removing this key at the provider). The link contains this key's ID and a signature, never the secret key.
      </p>
      <div class="row">
        <label>
          Valid for
          <select value={lifetime} onchange={(e) => (lifetime = Number(e.currentTarget.value))}>
            {#each LINK_LIFETIMES as l (l.seconds)}
              <option value={l.seconds}>{l.label}</option>
            {/each}
          </select>
        </label>
        <button type="button" disabled={linking} onclick={() => void makeLink()}>{link === null ? "Create link" : "Create a new link"}</button>
      </div>
      {#if linkError !== null}
        <p role="alert">{describeOperationError(linkError)}</p>
      {/if}
      {#if link !== null}
        <div class="row">
          <input class="url" type="text" readonly value={link.url} aria-label="Download link" bind:this={linkInput} />
          <button type="button" onclick={() => void copyLink()}>Copy link</button>
        </div>
        <p role="status">
          Expires {formatDate(link.expiresAt)}.
          {#if copied === "yes"}Copied to the clipboard.{:else if copied === "failed"}The clipboard is not available; the link is selected, press Ctrl+C.{/if}
        </p>
      {/if}
    </section>
  </div>
  {#snippet footer()}
    <button type="button" onclick={ontogglefavorite}>{favorite ? "Remove from favorites" : "Add to favorites"}</button>
    {#if onshowinfolder !== null}
      <button type="button" onclick={onshowinfolder}>Show in folder</button>
    {/if}
    <button type="button" onclick={ondownload}>Download…</button>
    <button type="button" data-autofocus onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.2rem 0.75rem;
    margin: 0 0 0.75rem;
    font-size: 0.93em;
  }

  dt {
    color: #6b6b73;
  }

  dd {
    margin: 0;
  }

  .key {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  h5 {
    margin: 0.75rem 0 0.25rem;
    font-size: 0.95em;
  }

  pre.text {
    max-height: 18rem;
    overflow: auto;
    background: rgb(0 0 0 / 5%);
    padding: 0.5rem;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: 0.85em;
  }

  img {
    max-width: 100%;
    max-height: 22rem;
    object-fit: contain;
    display: block;
    background: repeating-conic-gradient(#ddd 0% 25%, #fff 0% 50%) 50% / 16px 16px;
  }

  .note {
    color: #4a4a52;
    font-size: 0.9em;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin: 0.4rem 0;
  }

  .url {
    flex: 1;
    min-width: 12rem;
    font: inherit;
    font-size: 0.85em;
  }

  button,
  select {
    font: inherit;
    padding: 0.35rem 0.8rem;
    cursor: pointer;
  }

  @media (prefers-color-scheme: dark) {
    dt,
    .note {
      color: #b8b8c0;
    }

    pre.text {
      background: rgb(255 255 255 / 6%);
    }
  }
</style>
