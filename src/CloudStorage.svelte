<script lang="ts">
  import { onMount } from "svelte";
  import CloudAccountForm from "./CloudAccountForm.svelte";
  import CloudBrowser from "./CloudBrowser.svelte";
  import {
    checkAccount,
    CloudCommandError,
    listAccounts,
    listProviders,
    removeAccount,
    type AccountInfo,
    type ProviderInfo,
  } from "./cloud";
  import { CHECK_HINTS, CHECK_LABELS, describeError } from "./cloudMessages";

  // Cloud storage accounts: list, add, edit, check and remove, and a
  // read-only browser of each account's bucket (CloudBrowser.svelte). Nothing
  // here uploads, downloads or changes objects. Every operation is a Rust
  // command; the page never sees a stored key.

  type FormState = { mode: "add" } | { mode: "edit"; id: string };

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
  /** The account whose bucket is open in the browser, if any. */
  let browsing = $state<string | null>(null);
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

  function checkedAt(ms: number): string {
    return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  onMount(() => {
    void load();
  });
</script>

<section aria-labelledby="cloud-storage-heading">
  <h2 id="cloud-storage-heading">Cloud Storage</h2>
  <p>
    Connect sotyware to your cloud storage: Backblaze B2, Hetzner Object Storage or another S3-compatible service.
    Each account uses one bucket. Access keys are kept in your system's credential store (Windows Credential Manager
    or the desktop keyring), never in sotyware's own files.
  </p>

  {#if loadError}
    <p class="error" role="alert">{loadError}</p>
  {/if}
  {#if storageError}
    <p class="error" role="alert">
      The account database could not be opened: {storageError}. Accounts cannot be shown or added until this is fixed.
    </p>
  {/if}

  {#if browsed}
    {#key browsed.id}
      <CloudBrowser account={browsed} onclose={closeBrowser} onedit={() => editFromBrowser(browsed.id)} />
    {/key}
  {:else if providers && accounts && !storageError}
    {#if form?.mode === "add"}
      <CloudAccountForm
        {providers}
        account={null}
        onsaved={(info) => saved(info, true)}
        oncancel={() => (form = null)}
        onchanged={() => void load()}
      />
    {:else}
      <button type="button" class="primary" onclick={() => openForm({ mode: "add" })}>Add account</button>
    {/if}

    {#if notice}
      <p class="notice" role="status">{notice}</p>
    {/if}
    {#if actionError}
      <p class="error" role="alert">{actionError}</p>
    {/if}

    {#if accounts.length === 0 && form === null}
      <div class="empty">
        <p><strong>No storage accounts yet.</strong></p>
        <p>
          Add one with a bucket and an application key from your provider's console. A key restricted to that one
          bucket is enough.
        </p>
      </div>
    {/if}

    <ul class="accounts">
      {#each accounts as account (account.id)}
        <li data-account={account.id}>
          {#if form?.mode === "edit" && form.id === account.id}
            <CloudAccountForm
              {providers}
              {account}
              onsaved={(info) => saved(info, false)}
              oncancel={() => (form = null)}
              onchanged={() => void load()}
            />
          {:else}
            <div class="title">
              <strong class="label">{account.label}</strong>
              <span class="provider">{account.providerName ?? account.provider}</span>
            </div>
            <dl class="meta">
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

            <div class="status-line">
              {#if checking[account.id]}
                <span class="status checking" role="status">Checking connection…</span>
              {:else if account.lastCheck}
                <span
                  class="status"
                  class:ok={account.lastCheck.status === "connected"}
                  class:bad={account.lastCheck.status !== "connected"}
                  data-status={account.lastCheck.status}
                >
                  {CHECK_LABELS[account.lastCheck.status]}
                </span>
                <span class="when">checked {checkedAt(account.lastCheck.checkedAt)}</span>
              {:else}
                <span class="status unknown">Not checked yet</span>
              {/if}
            </div>
            {#if account.lastCheck && !checking[account.id] && account.lastCheck.status !== "connected"}
              <p class="check-detail">
                {CHECK_HINTS[account.lastCheck.status]}
                {#if account.lastCheck.message}<span class="detail">Details: {account.lastCheck.message}</span>{/if}
              </p>
            {/if}

            {#if confirmingRemove === account.id}
              <div class="confirm" role="group" aria-label="Confirm removal of {account.label}">
                <p>
                  Remove <strong>{account.label}</strong> from sotyware? Its access key is deleted from this computer's
                  credential store. The bucket <strong>{account.bucket ?? ""}</strong> and its files at
                  {account.providerName ?? account.provider} are <strong>not</strong> deleted or changed.
                </p>
                <div class="actions">
                  <button type="button" class="danger" disabled={removing === account.id} onclick={() => remove(account)}>
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
                  disabled={account.bucket === null || account.configError !== null}
                  onclick={() => openBrowser(account)}
                >
                  Open bucket
                </button>
                <button type="button" disabled={checking[account.id] === true} onclick={() => check(account)}>
                  Check connection
                </button>
                <button type="button" onclick={() => openForm({ mode: "edit", id: account.id })}>Edit</button>
                <button
                  type="button"
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
    <p>Loading…</p>
  {/if}
</section>

<style>
  section {
    margin-top: 1.5rem;
  }

  h2 {
    font-size: 1.1em;
  }

  button {
    font: inherit;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }

  .accounts {
    list-style: none;
    padding: 0;
  }

  .accounts > li {
    padding: 0.75rem 0;
    border-top: 1px solid rgb(0 0 0 / 10%);
  }

  .title {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    flex-wrap: wrap;
  }

  .provider,
  .when,
  .meta,
  .detail {
    color: #6b6b73;
    font-size: 0.9em;
  }

  .meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.1rem 0.75rem;
    margin: 0.4rem 0;
  }

  .meta dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .status-line {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    margin: 0.3rem 0;
  }

  .status {
    font-weight: 600;
    font-size: 0.9em;
  }

  .status.ok {
    color: #2e7d32;
  }

  .status.bad {
    color: #c62828;
  }

  .status.unknown,
  .status.checking {
    color: #6b6b73;
    font-weight: normal;
  }

  .check-detail {
    margin: 0.2rem 0 0.5rem;
    font-size: 0.9em;
  }

  .detail {
    display: block;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .confirm {
    padding: 0.75rem 1rem;
    border-left: 3px solid #c62828;
    background: rgb(0 0 0 / 4%);
    font-size: 0.95em;
  }

  .confirm p {
    margin-top: 0;
  }

  .danger {
    color: #fff;
    background: #c62828;
    border: 1px solid #c62828;
    border-radius: 3px;
  }

  .empty {
    margin: 1rem 0;
    padding: 0.75rem 1rem;
    border-left: 3px solid #8a8a93;
    background: rgb(0 0 0 / 4%);
  }

  .empty p {
    margin: 0.25rem 0;
  }

  .notice {
    padding: 0.5rem 0.75rem;
    border-left: 3px solid #2e7d32;
    background: rgb(0 0 0 / 4%);
    font-size: 0.95em;
  }

  .error {
    color: #c62828;
  }

  @media (prefers-color-scheme: dark) {
    .accounts > li {
      border-top-color: rgb(255 255 255 / 12%);
    }

    .provider,
    .when,
    .meta,
    .detail,
    .status.unknown,
    .status.checking {
      color: #a0a0a8;
    }

    .status.ok {
      color: #81c784;
    }

    .status.bad,
    .error {
      color: #ef7070;
    }

    .confirm,
    .empty,
    .notice {
      background: rgb(255 255 255 / 6%);
    }
  }
</style>
