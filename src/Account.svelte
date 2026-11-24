<script lang="ts">
  import { onMount } from "svelte";
  import * as acc from "./account";
  import type { AccountStatus, DeviceInfo, EnrollmentInfo } from "./account";
  import { describeError } from "./cloudMessages";
  import { formatDateTime } from "./format";
  import { openGithubPage } from "./githubPage";

  // The optional sotyware account (ADR-032): sign in on several computers
  // and find the same storage accounts and backup jobs everywhere. The
  // storage keys travel only inside a vault encrypted on this computer;
  // the service cannot read them. Without an account nothing changes.

  let status = $state<AccountStatus | null>(null);
  let loadError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);
  let mode = $state<"sign-in" | "register" | "verify" | "reset">("sign-in");
  let email = $state("");
  let password = $state("");
  let code = $state("");
  let newPassword = $state("");
  let deviceName = $state("This computer");
  /** The recovery key, shown until the user retyped it correctly. */
  let shownKey = $state<string | null>(null);
  let retyped = $state("");
  let typedKey = $state("");
  let approval = $state<{ id: string; fingerprint: string } | null>(null);
  let deviceList = $state<DeviceInfo[]>([]);
  let requests = $state<EnrollmentInfo[]>([]);
  let confirm = $state<string | null>(null);
  let rotateWith = $state("");
  let pwCurrent = $state("");
  let pwNew = $state("");
  let deletePassword = $state("");

  async function load(): Promise<void> {
    try {
      status = await acc.accountStatus();
      loadError = null;
      if (status.signedIn && status.vaultAccess === "enrolled") {
        [deviceList, requests] = await Promise.all([acc.devices(), acc.enrollments()]);
      }
    } catch (e) {
      loadError = `Could not read the account status: ${describeError(e)}`;
    }
  }

  async function run(f: () => Promise<void>): Promise<void> {
    busy = true;
    error = null;
    notice = null;
    try {
      await f();
    } catch (e) {
      error = describeError(e);
    } finally {
      busy = false;
    }
  }

  function clearSecrets(): void {
    password = "";
    newPassword = "";
    code = "";
    typedKey = "";
    retyped = "";
    pwCurrent = "";
    pwNew = "";
    deletePassword = "";
  }

  /** Opens the privacy notice in the browser (ADR-049); a failure is shown
   * like any other, without touching the form. */
  async function openPrivacyNotice(): Promise<void> {
    try {
      await openGithubPage("privacy-notice");
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  const doRegister = (): Promise<void> =>
    run(async () => {
      await acc.register(email, password);
      mode = "verify";
      notice = "If this address can be registered, a verification code was sent to it.";
    });

  const doVerify = (): Promise<void> =>
    run(async () => {
      await acc.verifyEmail(email, code, password);
      await acc.signIn(email, password, deviceName);
      clearSecrets();
      await load();
    });

  const doSignIn = (): Promise<void> =>
    run(async () => {
      await acc.signIn(email, password, deviceName);
      clearSecrets();
      await load();
    });

  const doReset = (): Promise<void> =>
    run(async () => {
      if (code === "") {
        await acc.requestReset(email);
        notice = "If this address has an account, a reset code was sent to it.";
      } else {
        await acc.confirmReset(email, code, newPassword);
        notice = "The password was changed. Every computer was signed out and must be unlocked again with the recovery key.";
        mode = "sign-in";
        clearSecrets();
      }
    });

  const doSetUp = (): Promise<void> =>
    run(async () => {
      shownKey = await acc.setUpSync();
      await load();
    });

  const doConfirmKey = (): Promise<void> =>
    run(async () => {
      const typed = retyped.replace(/[\s-]/g, "").toUpperCase();
      if (shownKey !== null && typed === shownKey.replace(/-/g, "") && (await acc.checkRecoveryKey(typed))) {
        shownKey = null;
        retyped = "";
        notice = "Your recovery key is confirmed. Keep it somewhere safe that is not this computer.";
      } else {
        error = "That does not match the recovery key shown. Check each group of characters.";
      }
    });

  const doUnlock = (): Promise<void> =>
    run(async () => {
      await acc.unlock(typedKey);
      typedKey = "";
      notice = "This computer is unlocked and synced.";
      await load();
    });

  const doRequestApproval = (): Promise<void> =>
    run(async () => {
      approval = await acc.requestApproval();
    });

  const doCheckApproval = (): Promise<void> =>
    run(async () => {
      if (!approval) {
        return;
      }
      const s = await acc.approvalStatus(approval.id);
      if (s === "approved") {
        approval = null;
        notice = "This computer was approved and synced.";
        await load();
      } else {
        notice = s === "pending" ? "Not approved yet." : `The request was ${s}. Ask again.`;
      }
    });

  const doAnswer = (id: string, approve: boolean): Promise<void> =>
    run(async () => {
      await acc.answerEnrollment(id, approve);
      confirm = null;
      notice = approve ? "The computer was approved." : "The request was rejected.";
      await load();
    });

  const doRevoke = (id: string): Promise<void> =>
    run(async () => {
      await acc.revokeDevice(id);
      confirm = "rotate";
      notice = "The computer was revoked. It may still hold your keys: rotate the keys now, and change the storage keys at your providers if it was stolen.";
      await load();
    });

  const doRotate = (): Promise<void> =>
    run(async () => {
      const fresh = await acc.rotateKeys(rotateWith.trim() === "" ? null : rotateWith);
      rotateWith = "";
      confirm = null;
      if (fresh !== null) {
        shownKey = fresh;
      }
      notice = "The account key was replaced; revoked computers can no longer read new changes.";
      await load();
    });

  const doSync = (): Promise<void> =>
    run(async () => {
      await acc.syncNow();
      notice = "Synced.";
      await load();
    });

  const doResolve = (key: string, keepLocal: boolean): Promise<void> =>
    run(async () => {
      await acc.resolveConflict(key, keepLocal);
      await load();
    });

  const doChangePassword = (): Promise<void> =>
    run(async () => {
      await acc.changePassword(pwCurrent, pwNew);
      clearSecrets();
      notice = "The password was changed; every other computer was signed out.";
    });

  const doSignOut = (): Promise<void> =>
    run(async () => {
      await acc.signOut();
      notice = "Signed out. Your storage accounts on this computer stay.";
      await load();
    });

  const doDelete = (): Promise<void> =>
    run(async () => {
      await acc.deleteAccount(deletePassword);
      clearSecrets();
      confirm = null;
      notice = "The sotyware account was deleted. Your storage accounts on this computer stay.";
      await load();
    });

  function when(ms: number | null): string {
    return ms === null ? "never" : formatDateTime(new Date(ms));
  }

  onMount(() => {
    void load();
  });
</script>

<div class="page">
<section aria-labelledby="account-heading">
  <header class="page-header">
    <div class="titles">
      <h1 id="account-heading">Account</h1>
      <p class="lede">
        A sotyware account is optional. With one, the storage accounts and backup jobs you set up here appear on your other
        computers. Their keys are encrypted on this computer before anything is sent: the sotyware service stores only
        ciphertext it cannot read. Your password signs you in; it never unlocks your keys. A new computer needs your
        <strong>recovery key</strong> or an approval from a computer that is already set up.
      </p>
    </div>
  </header>

  {#if loadError}<p class="error" role="alert">{loadError}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}

  {#if shownKey}
    <div class="recovery-key" role="region" aria-label="Your recovery key">
      <h3>Your recovery key</h3>
      <p>
        Write it down or save it in a password manager now, somewhere other than this computer. It is shown only this
        once. Without it and without another set-up computer, nobody can unlock your synced keys again, not even
        sotyware.
      </p>
      <p class="key" data-recovery-key>{shownKey}</p>
      <label for="ac-retype">Type it again to confirm you saved it</label>
      <input id="ac-retype" type="text" bind:value={retyped} autocomplete="off" spellcheck="false" />
      <button type="button" class="primary" disabled={busy || retyped === ""} onclick={() => void doConfirmKey()}>I saved it</button>
    </div>
  {/if}

  {#if status && !status.configured}
    <p class="callout neutral" data-account="not-configured">
      Accounts are not available in this build: no sotyware account service is configured. Everything else works
      without one.{#if status.storageError} ({status.storageError}){/if}
    </p>
  {:else if status && !status.signedIn}
    <div class="auth card">
    <nav class="modes" aria-label="Account actions">
      <button type="button" aria-pressed={mode === "sign-in"} onclick={() => (mode = "sign-in")}>Sign in</button>
      <button type="button" aria-pressed={mode === "register" || mode === "verify"} onclick={() => (mode = "register")}>Create account</button>
      <button type="button" aria-pressed={mode === "reset"} onclick={() => (mode = "reset")}>Forgot password</button>
    </nav>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void (mode === "sign-in" ? doSignIn() : mode === "register" ? doRegister() : mode === "verify" ? doVerify() : doReset());
      }}
    >
      <fieldset disabled={busy}>
        <label for="ac-email">Email</label>
        <input id="ac-email" type="email" bind:value={email} required autocomplete="username" />
        {#if mode !== "reset"}
          <label for="ac-password">Password</label>
          <input id="ac-password" type="password" bind:value={password} required autocomplete={mode === "sign-in" ? "current-password" : "new-password"} />
          {#if mode === "register"}<p class="hint">At least 12 characters. A passphrase of several words works well.</p>{/if}
        {/if}
        {#if mode === "verify" || (mode === "reset" && notice)}
          <label for="ac-code">Code from the email</label>
          <input id="ac-code" type="text" inputmode="numeric" bind:value={code} required autocomplete="one-time-code" />
        {/if}
        {#if mode === "reset" && code !== ""}
          <label for="ac-new">New password</label>
          <input id="ac-new" type="password" bind:value={newPassword} required autocomplete="new-password" />
        {/if}
        {#if mode === "sign-in" || mode === "verify"}
          <label for="ac-device">Name of this computer</label>
          <input id="ac-device" type="text" bind:value={deviceName} required maxlength="64" />
        {/if}
        {#if mode === "register"}
          <p class="hint" data-account="privacy">
            You must be at least 16 to create an account. What the service stores and why:
            <button type="button" class="link" onclick={() => void openPrivacyNotice()}>Privacy notice</button>
          </p>
        {/if}
        <button type="submit" class="primary">
          {mode === "sign-in" ? "Sign in" : mode === "register" ? "Create account" : mode === "verify" ? "Verify and sign in" : code === "" ? "Send reset code" : "Set new password"}
        </button>
      </fieldset>
    </form>
    </div>
  {:else if status}
    <div class="signed-in" data-vault={status.vaultAccess}>
      <p>Signed in as <strong>{status.email}</strong>.</p>

      {#if status.vaultAccess === "none"}
        <h3>Set up sync</h3>
        <p>
          Synced: your storage accounts (with their keys, encrypted) and backup jobs. Not synced: files, local folders,
          recent items and anything running.
        </p>
        <button type="button" class="primary" disabled={busy} onclick={() => void doSetUp()}>Set up sync</button>
      {:else if status.vaultAccess === "pending"}
        <h3>Unlock this computer</h3>
        <p>This computer is signed in but cannot read your synced keys yet.</p>
        <label for="ac-unlock">Recovery key</label>
        <input id="ac-unlock" type="text" bind:value={typedKey} autocomplete="off" spellcheck="false" />
        <button type="button" class="primary" disabled={busy || typedKey === ""} onclick={() => void doUnlock()}>Unlock</button>
        <p>Or ask a computer that is already set up:</p>
        {#if approval}
          <p class="fingerprint">Approval code: <strong data-fingerprint>{approval.fingerprint}</strong></p>
          <p class="hint">On the other computer, open Account and approve the request only if it shows exactly these digits.</p>
          <button type="button" disabled={busy} onclick={() => void doCheckApproval()}>I approved it</button>
        {:else}
          <button type="button" disabled={busy} onclick={() => void doRequestApproval()}>Ask another computer</button>
        {/if}
      {:else}
        <h3>Sync</h3>
        <p>
          Last sync: {when(status.lastSyncMs)}{#if status.lastSyncError}<span class="error"> — {status.lastSyncError}</span>{/if}
        </p>
        <button type="button" disabled={busy} onclick={() => void doSync()}>Sync now</button>

        {#if status.conflicts.length > 0}
          <h3>Changed on two computers</h3>
          <ul class="conflicts">
            {#each status.conflicts as c (c.key)}
              <li data-conflict={c.key}>
                {c.summary} was also changed elsewhere; the other computer's version is in use.
                <button type="button" disabled={busy} onclick={() => void doResolve(c.key, true)}>Use this version</button>
                <button type="button" disabled={busy} onclick={() => void doResolve(c.key, false)}>Keep the other</button>
              </li>
            {/each}
          </ul>
        {/if}

        {#if requests.length > 0}
          <h3>Computers asking to join</h3>
          <ul class="requests">
            {#each requests as r (r.id)}
              <li data-request={r.id}>
                <strong>{r.deviceName}</strong> ({r.platform}) shows <strong>{r.fingerprint}</strong>
                {#if confirm === `approve-${r.id}`}
                  <span class="confirm"
                    >Approve only if the new computer shows exactly these digits: it will be able to read every synced key.
                    <button type="button" class="primary" onclick={() => void doAnswer(r.id, true)}>Approve</button>
                    <button type="button" onclick={() => (confirm = null)}>Cancel</button></span
                  >
                {:else}
                  <button type="button" onclick={() => (confirm = `approve-${r.id}`)}>Approve…</button>
                  <button type="button" onclick={() => void doAnswer(r.id, false)}>Reject</button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}

        <h3>Computers</h3>
        <ul class="devices">
          {#each deviceList as d (d.id)}
            <li data-device={d.id} data-state={d.state}>
              {d.name} ({d.platform}) — {d.state}{d.current ? ", this computer" : ""}, last seen {when(d.lastSeenMs)}
              {#if d.state !== "revoked" && !d.current}
                {#if confirm === `revoke-${d.id}`}
                  <span class="confirm"
                    >Revoke it? It is signed out at once; keys it already read stay on it.
                    <button type="button" class="danger" onclick={() => void doRevoke(d.id)}>Revoke</button>
                    <button type="button" onclick={() => (confirm = null)}>Cancel</button></span
                  >
                {:else}
                  <button type="button" onclick={() => (confirm = `revoke-${d.id}`)}>Revoke…</button>
                {/if}
              {/if}
            </li>
          {/each}
        </ul>

        <h3>Keys</h3>
        <p class="hint">
          Rotating replaces the account key. Enter your recovery key to keep it, or leave the field empty to get a new
          one (shown once).
        </p>
        <label for="ac-rotate">Recovery key (optional)</label>
        <input id="ac-rotate" type="text" bind:value={rotateWith} autocomplete="off" spellcheck="false" />
        <button type="button" disabled={busy} onclick={() => void doRotate()}>{confirm === "rotate" ? "Rotate keys now" : "Rotate keys"}</button>
      {/if}

      <h3>Password</h3>
      <label for="ac-pw-current">Current password</label>
      <input id="ac-pw-current" type="password" bind:value={pwCurrent} autocomplete="current-password" />
      <label for="ac-pw-new">New password</label>
      <input id="ac-pw-new" type="password" bind:value={pwNew} autocomplete="new-password" />
      <button type="button" disabled={busy || pwCurrent === "" || pwNew === ""} onclick={() => void doChangePassword()}>Change password</button>

      <div class="actions">
        <button type="button" disabled={busy} onclick={() => void doSignOut()}>Sign out</button>
        {#if confirm === "delete"}
          <span class="confirm"
            >Delete the sotyware account? Your synced vault is deleted at the service; storage accounts on your computers stay.
            <label for="ac-delete">Password</label>
            <input id="ac-delete" type="password" bind:value={deletePassword} autocomplete="current-password" />
            <button type="button" class="danger" disabled={busy || deletePassword === ""} onclick={() => void doDelete()}>Delete account</button>
            <button type="button" onclick={() => (confirm = null)}>Cancel</button></span
          >
        {:else}
          <button type="button" onclick={() => (confirm = "delete")}>Delete account…</button>
        {/if}
      </div>
    </div>
  {/if}
</section>
</div>

<style>
  label {
    display: block;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    margin-top: var(--space-3);
    margin-bottom: var(--space-1);
  }

  input {
    display: block;
    width: 100%;
  }

  button {
    margin-top: var(--space-3);
  }

  .auth {
    max-width: 28rem;
  }

  .modes {
    display: flex;
    gap: var(--space-1);
    padding: 3px;
    background: var(--surface-2);
    border-radius: var(--radius-md);
  }

  .modes button {
    flex: 1;
    margin: 0;
    min-height: 28px;
    background: transparent;
    border-color: transparent;
    color: var(--text-muted);
  }

  .modes button[aria-pressed="true"] {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
    box-shadow: var(--shadow-sm);
  }

  .recovery-key {
    margin: var(--space-4) 0;
    padding: var(--space-4) var(--space-5);
    background: var(--warning-soft);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-lg);
  }

  .recovery-key h3 {
    margin-bottom: var(--space-2);
  }

  .key {
    font-family: var(--font-mono);
    font-size: var(--text-lg);
    word-break: break-all;
    user-select: all;
    padding: var(--space-3);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .signed-in :global(h3) {
    margin: var(--space-6) 0 var(--space-2);
  }

  .hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .actions {
    margin-top: var(--space-4);
  }

  .actions button {
    margin-top: 0;
  }

  .danger:not(button) {
    color: var(--danger);
  }
</style>
