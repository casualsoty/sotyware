<script lang="ts">
  import { onMount } from "svelte";
  import type { AccountInfo } from "./cloud";
  import { describeOperationError } from "./cloudMessages";
  import {
    abortIncompleteUpload,
    dismissRecovery,
    inspectRecovery,
    listIncompleteUploads,
    listRecovery,
    type IncompleteInfo,
    type Inspection,
    type RecoveryItem,
  } from "./cloudRecovery";

  // The recovery view (ADR-030): work the operation journal recorded and
  // that did not finish normally — uploads, downloads and file operations
  // interrupted by a crash, a power loss or a killed process, and partial
  // moves. Nothing here repeats an operation. Inspect reads the provider;
  // the only changes are dismissing an entry and aborting an incomplete
  // multipart upload the user picked.

  interface Props {
    accounts: AccountInfo[];
  }

  let { accounts }: Props = $props();

  let unavailable = $state<string | null>(null);
  let items = $state<RecoveryItem[]>([]);
  let loadError = $state<string | null>(null);
  let inspections = $state<Record<number, Inspection>>({});
  let busy = $state<Record<string, boolean>>({});
  let errors = $state<Record<string, string>>({});
  let confirming = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let scanAccount = $state<string>("");
  let scan = $state<{ account: AccountInfo; uploads: IncompleteInfo[]; more: boolean } | null>(null);

  async function load(): Promise<void> {
    try {
      const list = await listRecovery();
      unavailable = list.unavailable;
      items = list.items;
      loadError = null;
    } catch (e) {
      loadError = `Could not read the operation journal: ${describeOperationError(e)}`;
    }
  }

  function title(item: RecoveryItem): string {
    const where = [item.accountLabel ?? item.accountId, item.bucket].filter((x) => x !== null).join(" / ");
    switch (item.kind) {
      case "upload":
        return `Upload of ${item.fileName ?? item.key ?? "a file"}${where ? ` to ${where}` : ""}`;
      case "operation":
        return `${(item.operation ?? "Operation").replace(/^./, (c) => c.toUpperCase())} of ${item.steps} object(s)${where ? ` in ${where}` : ""}`;
      case "download":
      case "staging":
        return "Temporary download file";
      default:
        return "Unknown entry";
    }
  }

  function when(ms: number): string {
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleString();
  }

  async function run(key: string, f: () => Promise<void>): Promise<void> {
    busy = { ...busy, [key]: true };
    const { [key]: _, ...rest } = errors;
    errors = rest;
    try {
      await f();
    } catch (e) {
      errors = { ...errors, [key]: describeOperationError(e) };
    } finally {
      const { [key]: _b, ...others } = busy;
      busy = others;
    }
  }

  function inspect(item: RecoveryItem): Promise<void> {
    return run(`inspect-${item.id}`, async () => {
      const result = await inspectRecovery(item.id);
      inspections = { ...inspections, [item.id]: result };
    });
  }

  function dismiss(item: RecoveryItem): Promise<void> {
    return run(`dismiss-${item.id}`, async () => {
      await dismissRecovery(item.id);
      confirming = null;
      notice = "The entry was removed from the operation journal.";
      await load();
    });
  }

  function needsWarning(item: RecoveryItem): boolean {
    return item.state === "multipart-open" || item.state === "incomplete-parts" || (item.multipart && item.state === "started");
  }

  function abortFor(item: RecoveryItem, u: IncompleteInfo): Promise<void> {
    return run(`abort-${u.uploadId}`, async () => {
      if (item.accountId === null || item.bucket === null) {
        return;
      }
      const result = await abortIncompleteUpload(item.accountId, item.bucket, u.key, u.uploadId);
      confirming = null;
      notice = abortNotice(result);
      await load();
      if (items.some((i) => i.id === item.id)) {
        await inspect(item);
      }
    });
  }

  function abortNotice(result: string): string {
    return result === "aborted"
      ? "The incomplete upload was aborted; its parts are removed."
      : result === "gone"
        ? "The provider no longer knows this upload (it was completed or aborted before)."
        : "The abort failed or did not answer in time; the parts may remain. Try again later.";
  }

  function scanNow(): Promise<void> {
    const account = accounts.find((a) => a.id === scanAccount);
    if (!account || account.bucket === null) {
      return Promise.resolve();
    }
    return run("scan", async () => {
      const r = await listIncompleteUploads(account.id, account.bucket!);
      scan = { account, uploads: r.uploads, more: r.more };
    });
  }

  function abortScanned(u: IncompleteInfo): Promise<void> {
    const s = scan;
    if (!s || s.account.bucket === null) {
      return Promise.resolve();
    }
    return run(`abort-${u.uploadId}`, async () => {
      const result = await abortIncompleteUpload(s.account.id, s.account.bucket!, u.key, u.uploadId);
      confirming = null;
      notice = abortNotice(result);
      if (result !== "failed") {
        scan = { ...s, uploads: s.uploads.filter((x) => x.uploadId !== u.uploadId) };
      }
      await load();
    });
  }

  onMount(() => {
    void load();
  });
</script>

{#if loadError}
  <p class="error" role="alert">{loadError}</p>
{/if}
{#if unavailable}
  <p class="warning" role="alert" data-recovery="unavailable">
    The operation journal cannot be used ({unavailable}). Uploads, downloads and file operations still work, but if
    sotyware stops unexpectedly it cannot tell afterwards what they did.
  </p>
{/if}

{#if notice}
  <p class="notice" role="status">{notice}</p>
{/if}

{#if items.length > 0}
  <section class="recovery" aria-labelledby="recovery-heading">
    <h3 id="recovery-heading">Interrupted work ({items.length})</h3>
    <p class="hint">
      sotyware recorded this work before it started and it did not finish normally. Nothing is repeated
      automatically. Inspect each entry to see what the provider holds now, then dismiss it once you have dealt with
      it.
    </p>
    <ul>
      {#each items as item (item.id)}
        {@const insp = inspections[item.id]}
        <li data-recovery-item={item.id} data-kind={item.kind} data-state={item.state}>
          <strong>{title(item)}</strong>
          <span class="time">started {when(item.startedMs)}</span>
          <p>{item.summary}</p>
          <div class="actions">
            {#if item.kind === "upload" || item.kind === "operation"}
              <button type="button" disabled={busy[`inspect-${item.id}`]} onclick={() => void inspect(item)}>
                {busy[`inspect-${item.id}`] ? "Inspecting…" : "Inspect"}
              </button>
            {/if}
            {#if confirming === `dismiss-${item.id}`}
              <span class="confirm">
                {#if needsWarning(item)}
                  Parts of an incomplete upload may remain at the provider and be billed until aborted.
                {/if}
                Remove this entry?
                <button type="button" class="danger" onclick={() => void dismiss(item)}>Remove entry</button>
                <button type="button" onclick={() => (confirming = null)}>Keep</button>
              </span>
            {:else}
              <button type="button" onclick={() => (confirming = `dismiss-${item.id}`)}>Dismiss</button>
            {/if}
          </div>
          {#if errors[`inspect-${item.id}`]}
            <p class="error" role="alert">{errors[`inspect-${item.id}`]}</p>
          {/if}
          {#if errors[`dismiss-${item.id}`]}
            <p class="error" role="alert">{errors[`dismiss-${item.id}`]}</p>
          {/if}
          {#if insp}
            <div class="inspection" data-verdict={insp.verdict}>
              <p>{insp.summary}</p>
              {#each Object.entries(insp.keys) as [category, keys] (category)}
                {#if category === "original-kept" || category === "different" || category === "destination-differs" || category === "missing"}
                  <details>
                    <summary>{category.replace(/-/g, " ")}: {insp.counts[category] ?? keys.length}</summary>
                    <ul class="keys">
                      {#each keys as k (k)}<li>{k}</li>{/each}
                    </ul>
                  </details>
                {/if}
              {/each}
              {#if insp.openUploads.length > 0}
                <ul class="uploads">
                  {#each insp.openUploads as u (u.uploadId)}
                    <li data-upload={u.uploadId}>
                      Incomplete upload started {u.initiated ?? "at an unknown time"}
                      {u.recorded ? "(started by sotyware on this computer)" : "(not recorded by sotyware: possibly another device)"}
                      {#if confirming === `abort-${u.uploadId}`}
                        <span class="confirm">
                          {#if !u.recorded}Another device may still be uploading this file.{/if}
                          Abort it and remove its parts?
                          <button type="button" class="danger" disabled={busy[`abort-${u.uploadId}`]} onclick={() => void abortFor(item, u)}
                            >Abort upload</button
                          >
                          <button type="button" onclick={() => (confirming = null)}>Keep</button>
                        </span>
                      {:else}
                        <button type="button" onclick={() => (confirming = `abort-${u.uploadId}`)}>Abort…</button>
                      {/if}
                      {#if errors[`abort-${u.uploadId}`]}
                        <span class="error" role="alert">{errors[`abort-${u.uploadId}`]}</span>
                      {/if}
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}

{#if accounts.length > 0}
  <details class="scan">
    <summary>Unfinished large uploads</summary>
    <p class="hint">
      Large files are uploaded in parts. An upload that was never finished or aborted keeps its parts at the provider,
      where they may be billed. List them for one account's bucket; sotyware aborts only the ones you choose.
    </p>
    <label for="recovery-scan-account">Account</label>
    <select id="recovery-scan-account" value={scanAccount} onchange={(e) => (scanAccount = e.currentTarget.value)}>
      <option value="">Choose an account</option>
      {#each accounts.filter((a) => a.bucket !== null) as a (a.id)}
        <option value={a.id}>{a.label}</option>
      {/each}
    </select>
    <button type="button" disabled={scanAccount === "" || busy["scan"]} onclick={() => void scanNow()}>
      {busy["scan"] ? "Listing…" : "List unfinished uploads"}
    </button>
    {#if errors["scan"]}
      <p class="error" role="alert">{errors["scan"]}</p>
    {/if}
    {#if scan}
      {#if scan.uploads.length === 0}
        <p class="notice" role="status">No unfinished uploads in {scan.account.label}.</p>
      {:else}
        <ul class="uploads">
          {#each scan.uploads as u (u.uploadId)}
            <li data-upload={u.uploadId}>
              <span class="key">{u.key}</span>, started {u.initiated ?? "at an unknown time"}
              {u.recorded ? "(started by sotyware on this computer)" : "(not recorded by sotyware: possibly another device)"}
              {#if confirming === `abort-${u.uploadId}`}
                <span class="confirm">
                  {#if !u.recorded}Another device may still be uploading this file.{/if}
                  Abort it and remove its parts?
                  <button type="button" class="danger" disabled={busy[`abort-${u.uploadId}`]} onclick={() => void abortScanned(u)}
                    >Abort upload</button
                  >
                  <button type="button" onclick={() => (confirming = null)}>Keep</button>
                </span>
              {:else}
                <button type="button" onclick={() => (confirming = `abort-${u.uploadId}`)}>Abort…</button>
              {/if}
              {#if errors[`abort-${u.uploadId}`]}
                <span class="error" role="alert">{errors[`abort-${u.uploadId}`]}</span>
              {/if}
            </li>
          {/each}
        </ul>
        {#if scan.more}
          <p class="hint">More unfinished uploads exist than were listed.</p>
        {/if}
      {/if}
    {/if}
  </details>
{/if}

<style>
  .recovery {
    border: 1px solid #e0a000;
    border-radius: 6px;
    padding: 0.6rem 0.9rem;
    margin: 0.8rem 0;
  }

  .recovery h3 {
    margin: 0 0 0.3rem;
  }

  .recovery > ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .recovery > ul > li {
    border-top: 1px solid rgb(0 0 0 / 10%);
    padding: 0.5rem 0;
  }

  .time,
  .hint {
    font-size: 0.85em;
    color: #6b6b73;
  }

  .time {
    margin-left: 0.5rem;
  }

  .actions {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    align-items: center;
  }

  .inspection {
    margin-top: 0.4rem;
    padding: 0.4rem 0.6rem;
    border-left: 3px solid #8a8a93;
  }

  .inspection[data-verdict="complete"],
  .inspection[data-verdict="nothing-stored"] {
    border-left-color: #2e7d32;
  }

  .inspection[data-verdict="needs-attention"] {
    border-left-color: #e0a000;
  }

  .keys {
    font-family: ui-monospace, monospace;
    font-size: 0.85em;
  }

  .scan {
    margin: 0.8rem 0;
  }

  button {
    font: inherit;
    padding: 0.25rem 0.7rem;
    cursor: pointer;
  }

  .danger {
    color: #c62828;
  }

  .warning {
    color: #8a5a00;
  }

  .error {
    color: #c62828;
  }

  .notice {
    color: #2e7d32;
  }

  @media (prefers-color-scheme: dark) {
    .time,
    .hint {
      color: #a0a0a8;
    }

    .recovery > ul > li {
      border-top-color: rgb(255 255 255 / 12%);
    }

    .warning {
      color: #f0c060;
    }

    .error,
    .danger {
      color: #ef7070;
    }

    .notice {
      color: #7bc67e;
    }
  }
</style>
