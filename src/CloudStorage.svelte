<script lang="ts">
  import { onMount, untrack } from "svelte";
  import CloudAccountForm from "./CloudAccountForm.svelte";
  import CloudBrowser from "./CloudBrowser.svelte";
  import CloudRecovery from "./CloudRecovery.svelte";
  import type { FolderSyncModel } from "./folderSyncModel.svelte";
  import { mergeCheck, type StartupChecks } from "./startupChecks.svelte";
  import Icon from "./Icon.svelte";
  import {
    checkAccount,
    CloudCommandError,
    listAccounts,
    listProviders,
    removeAccount,
    type AccountInfo,
    type ProviderInfo,
  } from "./cloud";
  import { CHECK_HINTS, CHECK_LABELS, checkTone, describeError } from "./cloudMessages";
  import { formatTime } from "./format";

  // Cloud storage accounts: list, add, edit, check and remove, and the
  // browser of each account's bucket (CloudBrowser.svelte). Every operation
  // is a Rust command; the page never sees a stored key. The shell's sidebar
  // opens an account's bucket through `browsing` and the add form through
  // `requestAdd`, and hears about list changes through `onaccountschange`.

  type FormState = { mode: "add" } | { mode: "edit"; id: string };

  interface Props {
    /** The account whose bucket is open (bound by the shell). */
    browsing?: string | null;
    /** The shell asks for the add form; answered with `onaddrequested`. */
    requestAdd?: boolean;
    onaddrequested?: () => void;
    /** The list changed (loaded, added, edited, checked, removed). */
    onaccountschange?: (accounts: AccountInfo[]) => void;
    /** Opens the Backups page (from a bucket's Snapshots & backups tab). */
    onopenbackups?: (() => void) | undefined;
    /** An account was added, changed or removed here (the shell syncs). */
    onlocalchange?: (() => void) | undefined;
    /** Changes when the accounts may have changed elsewhere (a sync):
     * the list is read again. */
    generation?: number;
    /** The live state of folder sync (ADR-043), for the bucket's Sync tab. */
    folderSyncs?: FolderSyncModel | undefined;
    /** The connection checks the shell runs at startup: their results are
     * merged into the list and the account being checked shows so. */
    startupChecks?: StartupChecks | undefined;
  }

  let {
    browsing = $bindable(null),
    requestAdd = false,
    onaddrequested,
    onaccountschange,
    onopenbackups,
    onlocalchange,
    generation = 0,
    folderSyncs,
    startupChecks,
  }: Props = $props();

  // A sync applied other devices' changes: read the list again (not on
  // the first value, which `onMount` loads).
  let seenGeneration: number | null = null;
  $effect(() => {
    const g = generation;
    if (seenGeneration !== null && g !== seenGeneration) {
      void load();
    }
    seenGeneration = g;
  });

  let providers = $state<ProviderInfo[] | null>(null);
  let accounts = $state<AccountInfo[] | null>(null);
  let storageError = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let form = $state<FormState | null>(null);
  let checking = $state<Record<string, boolean>>({});
  let confirmingRemove = $state<string | null>(null);
  let removing = $state<string | null>(null);
  /** Whether `id` is being checked, by its button or at startup. */
  function isChecking(id: string): boolean {
    return checking[id] === true || startupChecks?.current === id;
  }

  // Startup checks that finished: their results go into the list (one the
  // user started meanwhile and that finished later wins).
  $effect(() => {
    const results = startupChecks?.results ?? {};
    const list = untrack(() => accounts);
    if (list === null) {
      return;
    }
    let next = list;
    for (const info of Object.values(results)) {
      next = mergeCheck(next, info);
    }
    if (next.some((a, i) => a !== list[i])) {
      accounts = next;
    }
  });

  const browsed = $derived(browsing === null ? null : (accounts?.find((a) => a.id === browsing) ?? null));

  function openBrowser(account: AccountInfo): void {
    actionError = null;
    notice = null;
    confirmingRemove = null;
    form = null;
    browsing = account.id;
  }

  function closeBrowser(message: string | null): void {
    browsing = null;
    if (message !== null) {
      actionError = message;
      void load();
    }
  }

  function editFromBrowser(id: string): void {
    browsing = null;
    openForm({ mode: "edit", id });
  }

  // Every change of the list reaches the shell's sidebar.
  $effect(() => {
    if (accounts !== null) {
      onaccountschange?.(accounts);
    }
  });

  // The sidebar's "Add storage account": the list must be loaded first.
  $effect(() => {
    if (requestAdd && providers !== null && accounts !== null) {
      openForm({ mode: "add" });
      onaddrequested?.();
    }
  });

  async function load(): Promise<void> {
    try {
      const [p, list] = await Promise.all([listProviders(), listAccounts()]);
      providers = p;
      accounts = list.accounts;
      storageError = list.storageError;
      loadError = null;
    } catch (e) {
      loadError = `Could not load the storage accounts: ${describeError(e)}`;
    }
  }

  /** Puts `info` in the list in Rust's order (by id). */
  function upsert(info: AccountInfo): void {
    const rest = (accounts ?? []).filter((a) => a.id !== info.id);
    accounts = [...rest, info].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  }

  function openForm(next: FormState): void {
    actionError = null;
    notice = null;
    confirmingRemove = null;
    form = next;
  }

  function saved(info: AccountInfo, wasNew: boolean): void {
    upsert(info);
    form = null;
    onlocalchange?.();
    notice = wasNew ? `Added ${info.label}. Check its connection to make sure the key works.` : `Saved ${info.label}.`;
  }

  async function check(account: AccountInfo): Promise<void> {
    actionError = null;
    notice = null;
    checking = { ...checking, [account.id]: true };
    try {
      upsert(await checkAccount(account.id));
    } catch (e) {
      actionError = `Could not check ${account.label}: ${describeError(e)}`;
      if (e instanceof CloudCommandError && e.kind === "unknown-account") {
        void load();
      }
    } finally {
      const { [account.id]: _done, ...rest } = checking;
      checking = rest;
    }
  }

  async function remove(account: AccountInfo): Promise<void> {
    actionError = null;
    removing = account.id;
    try {
      await removeAccount(account.id);
      accounts = (accounts ?? []).filter((a) => a.id !== account.id);
      confirmingRemove = null;
      onlocalchange?.();
      notice = `Removed ${account.label} from sotyware. Its bucket and files at the provider were not changed.`;
    } catch (e) {
      actionError = `Could not remove ${account.label}: ${describeError(e)} The account is still listed.`;
      if (e instanceof CloudCommandError && (e.kind === "partial" || e.kind === "unknown-account")) {
        void load();
      }
    } finally {
      removing = null;
    }
  }

  // The credential store's name on this system, as its users know it.
  const UA = typeof navigator === "undefined" ? "" : navigator.userAgent;
  const KEYCHAIN = /Mac/.test(UA)
    ? "the macOS Keychain"
    : /Windows/.test(UA)
      ? "Windows Credential Manager"
      : /Linux|X11/.test(UA)
        ? "the desktop keyring"
        : "Windows Credential Manager, the macOS Keychain or the desktop keyring";

  function checkedAt(ms: number): string {
    return formatTime(new Date(ms), { hour: "2-digit", minute: "2-digit" });
  }

  onMount(() => {
    void load();
  });
</script>

{#if browsed}
  {#key browsed.id}
    <CloudBrowser account={browsed} onclose={closeBrowser} onedit={() => editFromBrowser(browsed.id)} {onopenbackups} {folderSyncs} />
  {/key}
{:else}
  <div class="page">
    <section aria-labelledby="cloud-storage-heading">
      <header class="page-header">
        <div class="titles">
          <h1 id="cloud-storage-heading">Cloud Storage</h1>
          <p class="lede">
            Connect sotyware to your cloud storage: Backblaze B2, Hetzner Object Storage or another S3-compatible
            service. Each account uses one bucket. Access keys are kept in your system's credential store ({KEYCHAIN}),
            never in sotyware's own files.
          </p>
        </div>
        {#if providers && accounts && !storageError && form?.mode !== "add"}
          <button type="button" class="primary" onclick={() => openForm({ mode: "add" })}><Icon name="plus" size={15} />Add account</button>
        {/if}
      </header>

      {#if loadError}
        <p class="error" role="alert">{loadError}</p>
      {/if}
      {#if storageError}
        <p class="error" role="alert">
          The account database could not be opened: {storageError}. Accounts cannot be shown or added until this is fixed.
        </p>
      {/if}

      {#if providers && accounts && !storageError}
        <CloudRecovery {accounts} />
        {#if form?.mode === "add"}
          <div class="card form-card">
            <CloudAccountForm
              {providers}
              account={null}
              onsaved={(info) => saved(info, true)}
              oncancel={() => (form = null)}
              onchanged={() => void load()}
            />
          </div>
        {/if}

        {#if notice}
          <p class="notice success" role="status">{notice}</p>
        {/if}
        {#if actionError}
          <p class="error" role="alert">{actionError}</p>
        {/if}

        {#if accounts.length === 0 && form === null}
          <div class="empty">
            <span class="empty-icon" aria-hidden="true"><Icon name="cloud" size={28} /></span>
            <p><strong>No storage accounts yet.</strong></p>
            <p>
              Add one with a bucket and an application key from your provider's console. A key restricted to that one
              bucket is enough.
            </p>
          </div>
        {/if}

        <ul class="accounts">
          {#each accounts as account (account.id)}
            <li data-account={account.id} class="account" class:editing={form?.mode === "edit" && form.id === account.id}>
              {#if form?.mode === "edit" && form.id === account.id}
                <CloudAccountForm
                  {providers}
                  {account}
                  onsaved={(info) => saved(info, false)}
                  oncancel={() => (form = null)}
                  onchanged={() => void load()}
                />
              {:else}
                <div class="head">
                  <span class="tile" aria-hidden="true"><Icon name="bucket" size={18} /></span>
                  <div class="title">
                    <strong class="label">{account.label}</strong>
                    <span class="provider">{account.providerName ?? account.provider}</span>
                  </div>
                  <div class="status-line">
                    {#if isChecking(account.id)}
                      <span class="badge" role="status"><span class="spinner" aria-hidden="true"></span>Checking connection…</span>
                    {:else if account.lastCheck}
                      <span class="status badge {checkTone(account.lastCheck.status)}" data-status={account.lastCheck.status}>
                        {CHECK_LABELS[account.lastCheck.status]}
                      </span>
                      <span class="when">checked {checkedAt(account.lastCheck.checkedAt)}</span>
                    {:else}
                      <span class="status badge unknown">Not checked yet</span>
                    {/if}
                  </div>
                </div>
                <dl class="meta meta-list">
                  <dt>Bucket</dt>
                  <dd class="bucket">{account.bucket ?? "none"}</dd>
                  <dt>Region</dt>
                  <dd>{account.region}</dd>
                  {#if account.endpointUrl}
                    <dt>Endpoint</dt>
                    <dd class="endpoint">{account.endpointUrl}</dd>
                  {/if}
                </dl>
                {#if account.configError}
                  <p class="error">This account's settings cannot be used: {account.configError}</p>
                {/if}
                {#if account.lastCheck && !isChecking(account.id) && account.lastCheck.status !== "connected"}
                  <p class="check-detail callout {checkTone(account.lastCheck.status)}">
                    {CHECK_HINTS[account.lastCheck.status]}
                    {#if account.lastCheck.message}<span class="detail">Details: {account.lastCheck.message}</span>{/if}
                  </p>
                {/if}

                {#if confirmingRemove === account.id}
                  <div class="confirm callout danger" role="group" aria-label="Confirm removal of {account.label}">
                    <p>
                      Remove <strong>{account.label}</strong> from sotyware? Its access key is deleted from this computer's
                      credential store. The bucket <strong>{account.bucket ?? ""}</strong> and its files at
                      {account.providerName ?? account.provider} are <strong>not</strong> deleted or changed.
                    </p>
                    <div class="actions">
                      <button
                        type="button"
                        class="primary danger"
                        disabled={removing === account.id}
                        onclick={() => remove(account)}
                      >
                        {removing === account.id ? "Removing…" : "Remove account"}
                      </button>
                      <button type="button" disabled={removing === account.id} onclick={() => (confirmingRemove = null)}>
                        Cancel
                      </button>
                    </div>
                  </div>
                {:else}
                  <div class="actions">
                    <button
                      type="button"
                      class="primary"
                      disabled={account.bucket === null || account.configError !== null}
                      onclick={() => openBrowser(account)}
                    >
                      Open bucket
                    </button>
                    <button type="button" disabled={isChecking(account.id)} onclick={() => check(account)}>
                      Check connection
                    </button>
                    <button type="button" class="ghost" onclick={() => openForm({ mode: "edit", id: account.id })}>Edit</button>
                    <button
                      type="button"
                      class="ghost danger"
                      onclick={() => {
                        actionError = null;
                        notice = null;
                        confirmingRemove = account.id;
                      }}
                    >
                      Remove
                    </button>
                  </div>
                {/if}
              {/if}
            </li>
          {/each}
        </ul>
      {:else if !loadError && !storageError}
        <p class="loading muted" role="status"><span class="spinner" aria-hidden="true"></span> Loading…</p>
      {/if}
    </section>
  </div>
{/if}

<style>
  .accounts {
    list-style: none;
    margin: var(--space-4) 0 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(22rem, 1fr));
    gap: var(--space-4);
  }

  .account {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: var(--space-4) var(--space-5);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    transition: border-color var(--dur-fast) var(--ease);
  }

  .account:hover {
    border-color: var(--border-strong);
  }

  .account.editing {
    grid-column: 1 / -1;
  }

  .account > .actions {
    margin-top: auto;
    padding-top: var(--space-4);
    gap: 6px;
  }

  .account > .actions button {
    padding: 0 10px;
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-bottom: var(--space-3);
  }

  .tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    flex: none;
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: var(--radius-md);
  }

  .title {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }

  .label {
    font-size: var(--text-md);
    overflow-wrap: anywhere;
  }

  .provider,
  .when {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .status-line {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
    flex: none;
  }

  .when {
    font-size: var(--text-xs);
  }

  .meta {
    margin: 0 0 var(--space-1);
  }

  .meta dd {
    white-space: pre-wrap;
  }

  .check-detail {
    margin: var(--space-3) 0 0;
  }

  .detail {
    display: block;
    margin-top: var(--space-1);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .confirm {
    margin: var(--space-3) 0 0;
  }

  .confirm .actions {
    margin-top: var(--space-2);
  }

  .form-card {
    margin: var(--space-4) 0;
  }

  .empty-icon {
    display: inline-flex;
    margin-bottom: var(--space-2);
    color: var(--text-subtle);
  }

  .loading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
</style>
