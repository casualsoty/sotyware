<script lang="ts">
  import type { AccountInfo } from "./cloud";
  import Icon, { type IconName } from "./Icon.svelte";
  import Logo from "./Logo.svelte";
  import { checkTone } from "./cloudMessages";
  import type { Section } from "./navigation";

  // The shell's left column: sotyware's name, the sections, the storage
  // accounts (each opens its bucket), and the sotyware account and Settings
  // at the bottom. Navigation only: every action is the page's.

  interface Props {
    section: Section;
    /** The account whose bucket is open, if any. */
    browsing: string | null;
    accounts: AccountInfo[] | null;
    /** Why the list could not be loaded (the Cloud Storage page says more). */
    accountsError?: string | null;
    onnavigate: (section: Section) => void;
    onopenaccount: (id: string) => void;
    onaddaccount: () => void;
    /** Narrow windows: the sidebar is an overlay that closes after a choice. */
    onclose?: () => void;
    /** False when the page's own title bar shows the name (Windows). */
    showBrand?: boolean;
    /** The sotyware account's sync state, when there is one to tell. */
    syncStatus?: { tone: "success" | "warning"; text: string } | null;
  }

  let {
    section,
    browsing,
    accounts,
    accountsError = null,
    onnavigate,
    onopenaccount,
    onaddaccount,
    onclose,
    showBrand = true,
    syncStatus = null,
  }: Props = $props();

  const MAIN: { id: Section; title: string; icon: IconName; key: string }[] = [
    { id: "overview", title: "Overview", icon: "overview", key: "1" },
    { id: "cloud-storage", title: "Cloud Storage", icon: "cloud", key: "2" },
    { id: "backups", title: "Backups", icon: "backup", key: "3" },
  ];

  const mac = typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
  const mod = mac ? "⌘" : "Ctrl+";

  function go(event: MouseEvent, next: Section): void {
    event.preventDefault();
    onnavigate(next);
    onclose?.();
  }

  function openAccount(event: MouseEvent, id: string): void {
    event.preventDefault();
    onopenaccount(id);
    onclose?.();
  }
</script>

<div class="sidebar-inner">
  {#if showBrand}
    <div class="brand">
      <Logo size={22} />
      <span class="name">sotyware</span>
    </div>
  {/if}

  <!-- Links, not buttons: they switch sections and keep each page's
       buttons those of the section shown. -->
  <nav aria-label="Sections" class="nav">
    {#each MAIN as s (s.id)}
      <a
        href="#{s.id}"
        class="nav-item"
        aria-current={section === s.id && (s.id !== "cloud-storage" || browsing === null) ? "page" : undefined}
        title="{s.title} ({mod}{s.key})"
        onclick={(e) => go(e, s.id)}><Icon name={s.icon} />{s.title}</a
      >
    {/each}
  </nav>

  <div class="group">
    <div class="group-head">
      <span id="sidebar-accounts-heading">Storage accounts</span>
    </div>
    {#if accountsError !== null && (accounts === null || accounts.length === 0)}
      <p class="none">Could not load the accounts.</p>
    {:else if accounts === null}
      <p class="none">Loading…</p>
    {:else if accounts.length === 0}
      <p class="none">No accounts yet.</p>
    {:else}
      <nav aria-labelledby="sidebar-accounts-heading" class="accounts">
        {#each accounts as a (a.id)}
          {@const tone = a.configError !== null ? "danger" : checkTone(a.lastCheck?.status ?? null)}
          <a
            href="#cloud-storage/{a.id}"
            class="nav-item account"
            data-sidebar-account={a.id}
            aria-current={section === "cloud-storage" && browsing === a.id ? "page" : undefined}
            title={a.bucket === null ? a.label : `${a.label}: ${a.bucket}`}
            onclick={(e) => openAccount(e, a.id)}
            ><span class="dot {tone}" aria-hidden="true"></span><span class="label">{a.label}</span><span
              class="visually-hidden">, {a.providerName ?? a.provider}</span
            ></a
          >
        {/each}
      </nav>
    {/if}
    <button
      type="button"
      class="ghost add"
      onclick={() => {
        onaddaccount();
        onclose?.();
      }}><Icon name="plus" size={14} />Add storage account</button>
  </div>

  <div class="bottom">
    <a
      href="#account"
      class="nav-item"
      aria-current={section === "account" ? "page" : undefined}
      title={syncStatus?.text}
      onclick={(e) => go(e, "account")}
      ><Icon name="account" /><span class="grow">Account</span>{#if syncStatus}<span
          class="dot {syncStatus.tone}"
          data-sync={syncStatus.tone}
          aria-hidden="true"
        ></span><span class="visually-hidden">, {syncStatus.text}</span>{/if}</a
    >
    <a
      href="#settings"
      class="nav-item"
      aria-current={section === "settings" ? "page" : undefined}
      title="Settings ({mod},)"
      onclick={(e) => go(e, "settings")}><Icon name="settings" />Settings</a
    >
  </div>
</div>

<style>
  .sidebar-inner {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: var(--space-3) var(--space-2) var(--space-3);
    gap: var(--space-1);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2) var(--space-4);
  }

  .name {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.01em;
  }

  .nav,
  .accounts {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 32px;
    padding: 0 var(--space-2);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text-muted);
    text-decoration: none;
    border-radius: var(--radius-md);
    transition:
      background-color var(--dur-fast) var(--ease),
      color var(--dur-fast) var(--ease);
  }

  .nav-item:hover {
    color: var(--text);
    background: var(--surface-3);
  }

  .nav-item[aria-current="page"] {
    color: var(--text);
    background: var(--surface-3);
  }

  .group {
    margin-top: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    flex: 1;
    overflow: auto;
  }

  .group-head {
    padding: 0 var(--space-2) var(--space-1);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-subtle);
  }

  .account {
    gap: var(--space-2);
    padding-left: var(--space-3);
  }

  .account .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--weight-regular);
  }

  .none {
    margin: 0;
    padding: var(--space-1) var(--space-3);
    font-size: var(--text-sm);
    color: var(--text-subtle);
  }

  .add {
    justify-content: flex-start;
    gap: var(--space-2);
    margin-top: var(--space-1);
    padding: 0 var(--space-3);
    font-weight: var(--weight-regular);
    color: var(--text-muted);
  }

  .add:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .bottom {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }
</style>
