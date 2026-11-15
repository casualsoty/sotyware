<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import {
    addAccount,
    CAPABILITY_NAMES,
    CloudCommandError,
    updateAccount,
    type AccountInfo,
    type AccountInput,
    type CredentialInput,
    type ProviderInfo,
  } from "./cloud";
  import { describeError } from "./cloudMessages";

  // Adds an account (`account` null) or edits one. Validation happens in Rust;
  // the form only collects what the backend needs and shows its answer.
  //
  // The key ID and secret live in this component's state only while the form
  // is open: they are cleared after a successful save, on cancel and when the
  // form goes away, and they are never written anywhere else.

  interface Props {
    providers: ProviderInfo[];
    account: AccountInfo | null;
    /** The account was saved; the form will be closed by the parent. */
    onsaved: (account: AccountInfo) => void;
    oncancel: () => void;
    /** Part of an edit was saved: the parent should reload the list. */
    onchanged: () => void;
  }

  let { providers, account, onsaved, oncancel, onchanged }: Props = $props();

  // The form edits a copy: the values it starts with are read once.
  const initial = untrack(() => account);
  const editing = initial !== null;
  const firstProvider = untrack(() => providers[0]);

  let provider = $state(initial?.provider ?? firstProvider?.id ?? "");
  let label = $state(initial?.label ?? "");
  let bucket = $state(initial?.bucket ?? "");
  let region = $state(initial?.region ?? firstProvider?.defaultRegion ?? "");
  let endpoint = $state(initial?.endpoint ?? "");
  let replaceKey = $state(!editing);
  let accessKeyId = $state("");
  let secretAccessKey = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  const profile = $derived(providers.find((p) => p.id === provider));
  const providerName = $derived(profile?.name ?? initial?.providerName ?? provider);
  const idPrefix = editing ? `edit-${initial?.id ?? ""}` : "add";

  function clearCredentials(): void {
    accessKeyId = "";
    secretAccessKey = "";
  }

  function chooseProvider(id: string): void {
    provider = id;
    const p = providers.find((x) => x.id === id);
    region = p?.defaultRegion ?? "";
    endpoint = "";
  }

  function cancel(): void {
    clearCredentials();
    oncancel();
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    error = null;
    saving = true;
    const input: AccountInput = {
      label,
      provider,
      region,
      endpoint: endpoint.trim() === "" ? null : endpoint,
      bucket,
    };
    const credentials: CredentialInput | null = replaceKey ? { accessKeyId, secretAccessKey } : null;
    try {
      let saved: AccountInfo;
      if (initial === null) {
        if (credentials === null) {
          throw new Error("an access key is required");
        }
        saved = await addAccount(input, credentials);
      } else {
        saved = await updateAccount(initial.id, input, credentials);
      }
      clearCredentials();
      onsaved(saved);
    } catch (e) {
      error = describeError(e);
      if (e instanceof CloudCommandError && e.kind === "partial") {
        // The new key is stored; only the other changes are not.
        clearCredentials();
        replaceKey = false;
        onchanged();
      }
    } finally {
      saving = false;
    }
  }

  onDestroy(clearCredentials);
</script>

<form class="account-form" onsubmit={submit} aria-label={editing ? `Edit ${initial?.label ?? ""}` : "Add account"}>
  <h3>{editing ? `Edit ${initial?.label ?? ""}` : "Add account"}</h3>
  <fieldset disabled={saving}>
    <!-- Explicit for/id pairs: a label that wraps its input would give the
         input an accessible name with its placeholder or value appended. -->
    <div class="field">
      <label for="{idPrefix}-provider">Provider</label>
      {#if editing}
        <input id="{idPrefix}-provider" type="text" value={providerName} readonly />
      {:else}
        <select id="{idPrefix}-provider" value={provider} onchange={(e) => chooseProvider(e.currentTarget.value)}>
          {#each providers as p (p.id)}
            <option value={p.id}>{p.name}</option>
          {/each}
        </select>
      {/if}
    </div>
    {#if profile && profile.capabilities.length > 0}
      <details class="capabilities">
        <summary>What {profile.name} guarantees</summary>
        <p class="hint">
          Versioning: {profile.versioning === "always"
            ? "always on"
            : profile.versioning === "optional"
              ? "optional, per bucket"
              : "unknown"}. "Assumed" means Amazon S3's behaviour, not checked at this provider.
        </p>
        <ul>
          {#each profile.capabilities as c (c.name)}
            <li data-capability={c.name}>
              <span>{CAPABILITY_NAMES[c.name] ?? c.name}</span>:
              <strong>{c.support}</strong>{#if c.evidence !== null && c.support !== "assumed"}<span class="evidence"
                  >{` (${c.evidence})`}</span
                >{/if}
            </li>
          {/each}
        </ul>
      </details>
    {/if}
    <div class="field">
      <label for="{idPrefix}-label">Name</label>
      <input id="{idPrefix}-label" type="text" bind:value={label} required placeholder="Photos" autocomplete="off" />
    </div>
    <div class="field">
      <label for="{idPrefix}-bucket">Bucket</label>
      <input id="{idPrefix}-bucket" type="text" bind:value={bucket} required spellcheck="false" autocomplete="off" />
    </div>
    <div class="field">
      <label for="{idPrefix}-region">Region</label>
      <input
        id="{idPrefix}-region"
        type="text"
        bind:value={region}
        required
        spellcheck="false"
        autocomplete="off"
        list={profile && profile.knownRegions.length > 0 ? `${idPrefix}-regions` : undefined}
      />
    </div>
    {#if profile && profile.knownRegions.length > 0}
      <datalist id="{idPrefix}-regions">
        {#each profile.knownRegions as r (r.id)}
          <option value={r.id}>{r.location}</option>
        {/each}
      </datalist>
    {/if}
    <div class="field">
      <label for="{idPrefix}-endpoint">{profile?.endpointRequired ? "Endpoint" : "Custom endpoint (optional)"}</label>
      <input
        id="{idPrefix}-endpoint"
        type="url"
        bind:value={endpoint}
        required={profile?.endpointRequired ?? false}
        placeholder={profile?.endpointRequired ? "https://s3.example.com" : "Default for the region"}
        spellcheck="false"
        autocomplete="off"
      />
    </div>

    {#if editing}
      <label class="inline">
        <input type="checkbox" bind:checked={replaceKey} onchange={() => replaceKey || clearCredentials()} />
        Replace access key
      </label>
    {/if}
    {#if replaceKey}
      <div class="field">
        <label for="{idPrefix}-access-key-id">Access key ID</label>
        <input
          id="{idPrefix}-access-key-id"
          type="password"
          bind:value={accessKeyId}
          required
          autocomplete="off"
          spellcheck="false"
          data-credential="access-key-id"
        />
      </div>
      <div class="field">
        <label for="{idPrefix}-secret-access-key">Secret access key</label>
        <input
          id="{idPrefix}-secret-access-key"
          type="password"
          bind:value={secretAccessKey}
          required
          autocomplete="off"
          spellcheck="false"
          data-credential="secret-access-key"
        />
      </div>
      <p class="hint">
        The key is stored in your system's credential store, not in sotyware's files, and is never shown again.
        {#if editing}The current key stays in use until the new one is stored.{/if}
      </p>
    {/if}

    <div class="actions">
      <button type="submit">{saving ? "Saving…" : editing ? "Save changes" : "Add account"}</button>
      <button type="button" onclick={cancel}>Cancel</button>
    </div>
  </fieldset>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</form>

<style>
  .account-form {
    margin: 1rem 0;
    padding: 1rem;
    border: 1px solid rgb(0 0 0 / 15%);
    border-radius: 6px;
  }

  h3 {
    margin: 0 0 0.75rem;
    font-size: 1em;
  }

  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
  }

  .field {
    margin-bottom: 0.6rem;
    font-size: 0.9em;
  }

  .field label {
    display: block;
  }

  label.inline {
    margin-bottom: 0.6rem;
    font-size: 0.9em;
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }

  input:not([type="checkbox"]),
  select {
    display: block;
    box-sizing: border-box;
    width: 100%;
    margin-top: 0.2rem;
    padding: 0.35rem 0.5rem;
    font: inherit;
  }

  input[readonly] {
    color: #6b6b73;
  }

  .hint {
    font-size: 0.85em;
    color: #6b6b73;
  }

  .capabilities {
    margin-bottom: 0.6rem;
    font-size: 0.85em;
  }

  .capabilities ul {
    margin: 0.3rem 0 0;
    padding-left: 1.2rem;
  }

  .actions {
    display: flex;
    gap: 0.5rem;
  }

  button {
    font: inherit;
    padding: 0.4rem 0.9rem;
    cursor: pointer;
  }

  .error {
    color: #c62828;
  }

  @media (prefers-color-scheme: dark) {
    .account-form {
      border-color: rgb(255 255 255 / 18%);
    }

    input[readonly],
    .hint {
      color: #a0a0a8;
    }

    .error {
      color: #ef7070;
    }
  }
</style>
