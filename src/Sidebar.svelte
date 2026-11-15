<script lang="ts">
  import type { AccountInfo } from "./cloud";
  import Icon, { type IconName } from "./Icon.svelte";
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
  }

  let { section, browsing, accounts, accountsError = null, onnavigate, onopenaccount, onaddaccount, onclose }: Props = $props();

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
  <div class="brand">
    <svg width="22" height="22" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient id="brand-tile" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stop-color="#2b6cf0" />
          <stop offset="1" stop-color="#0f8b8d" />
        </linearGradient>
      </defs>
      <rect x="1" y="1" width="30" height="30" rx="7" fill="url(#brand-tile)" />
      <path d="M16 5 L26 10 L16 15 L6 10 Z" fill="#fff" />
      <path d="M6 10 L16 15 V27 L6 22 Z" fill="#fff" fill-opacity=".78" />
      <path d="M26 10 L16 15 V27 L26 22 Z" fill="#fff" fill-opacity=".54" />
    </svg>
    <span class="name">sotyware</span>
  </div>

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
    <button type="button" class="ghost add" onclick={onaddaccount}><Icon name="plus" size={14} />Add storage account</button>
  </div>

  <div class="bottom">
    <a
      href="#account"
      class="nav-item"
      aria-current={section === "account" ? "page" : undefined}
      onclick={(e) => go(e, "account")}><Icon name="account" />Account</a
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

  .bottom {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }
</style>
