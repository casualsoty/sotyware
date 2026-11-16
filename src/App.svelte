<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";
  import Account from "./Account.svelte";
  import Backup from "./Backup.svelte";
  import { AutoSync, syncSummary } from "./accountSync.svelte";
  import { overallSummary, stateLabel } from "./folderSync";
  import { FolderSyncModel } from "./folderSyncModel.svelte";
  import { listAccounts, type AccountInfo } from "./cloud";
  import CloudStorage from "./CloudStorage.svelte";
  import { formatSize } from "./format";
  import Icon from "./Icon.svelte";
  import { SECTION_TITLES, shellShortcut, type Section, type SettingsTab } from "./navigation";
  import Overview from "./Overview.svelte";
  import Settings from "./Settings.svelte";
  import Sidebar from "./Sidebar.svelte";
  import TitleBar from "./TitleBar.svelte";
  import { UpdatesModel } from "./updatesModel.svelte";

  // The dashboard shell (docs/architecture/frontend.md): a sidebar with the
  // sections and storage accounts, and the main area with the page shown and,
  // when a new version is available, one update notification. Navigation is
  // local state: the page is never reloaded.

  // Decided once by the Rust side (`OperatingMode` in src-tauri/src/lib.rs):
  // "tray" when the tray icon exists, "dashboard-only" when no tray host is available.
  type OperatingMode = "tray" | "dashboard-only";

  let section = $state<Section>("overview");
  let settingsTab = $state<SettingsTab>("general");
  /** The account whose bucket the Cloud Storage page shows, if any. */
  let browsing = $state<string | null>(null);
  /** The sidebar asked Cloud Storage to open its add-account form. */
  let requestAdd = $state(false);
  let mode = $state<OperatingMode | null>(null);
  let modeError = $state<string | null>(null);
  let accounts = $state<AccountInfo[] | null>(null);
  let accountsError = $state<string | null>(null);
  /** Narrow windows: the sidebar is an overlay, open or closed. */
  let sidebarOpen = $state(false);
  let main: HTMLElement | undefined = $state();
  /** The window has no native title bar (Windows, ADR-040): the page draws
   * its own. Guessed from the platform for the first paint, then confirmed
   * by asking the window; anything but a plain `false` keeps the native one. */
  let customChrome = $state(typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent));

  const updates = new UpdatesModel();
  /** Syncs the optional sotyware account on its own (accountSync.svelte.ts). */
  const autoSync = new AutoSync();
  /** Bumped after a sync, so Cloud Storage reads its list again. */
  let accountsGeneration = $state(0);
  const syncStatus = $derived(syncSummary(autoSync.status));

  /** Every folder sync's live state (ADR-043): the sidebar's indicator and
   * the bucket view's Sync tab. The engines run in Rust either way. */
  const folderSyncs = new FolderSyncModel();
  const folderSync = $derived(overallSummary(folderSyncs.overview));
  const syncedAccounts = $derived(
    Object.fromEntries(
      (folderSyncs.overview?.syncs ?? []).map((s) => {
        const l = stateLabel(s);
        return [s.account, { tone: l.tone, label: l.label }];
      }),
    ),
  );
  const ustatus = $derived(updates.status);
  const uinstall = $derived(updates.install);

  async function loadMode(): Promise<void> {
    try {
      const value = await invoke<unknown>("operating_mode");
      if (value !== "tray" && value !== "dashboard-only") {
        throw new Error(`unknown operating mode: ${JSON.stringify(value)}`);
      }
      mode = value;
    } catch (e) {
      // Without the mode it is unknown whether a tray could bring the window back,
      // so the Hide button stays unavailable.
      modeError = `Could not check whether a system tray is available: ${String(e)}`;
    }
  }

  async function loadAccounts(): Promise<void> {
    try {
      const list = await listAccounts();
      accounts = list.accounts;
      accountsError = list.storageError;
    } catch (e) {
      accountsError = String(e);
    }
  }

  async function navigate(next: Section): Promise<void> {
    const changed = next !== section || (next === "cloud-storage" && browsing !== null);
    section = next;
    if (next === "cloud-storage") {
      browsing = null;
    }
    if (changed) {
      await focusPage();
    }
  }

  function openAccount(id: string): void {
    section = "cloud-storage";
    browsing = id;
  }

  function addAccount(): void {
    browsing = null;
    section = "cloud-storage";
    requestAdd = true;
  }

  function openUpdates(): void {
    settingsTab = "about";
    void navigate("settings");
  }

  /** Moves focus to the new page's title, so keyboard and screen-reader
   * users start reading where the page starts. */
  async function focusPage(): Promise<void> {
    await tick();
    main?.scrollTo?.({ top: 0 });
    const title = main?.querySelector<HTMLElement>("h1");
    if (title) {
      title.tabIndex = -1;
      title.focus({ preventScroll: true });
    }
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && sidebarOpen) {
      sidebarOpen = false;
      return;
    }
    // A dialog owns the keyboard.
    if (document.querySelector('[role="dialog"]') !== null) {
      return;
    }
    const target = shellShortcut(event);
    if (target !== null) {
      event.preventDefault();
      void navigate(target);
    }
  }

  function percent(downloaded: number, total: number | null): number | null {
    return total === null || total === 0 ? null : Math.min(100, Math.floor((downloaded / total) * 100));
  }

  /** The automatic update check runs in Rust (the `update-check` feature)
   * and sends no event; the status is a local read, so it is read again
   * whenever the window is shown or focused instead of on a timer. */
  function onshown(): void {
    if (!document.hidden && !updates.running && !updates.checking) {
      void updates.load();
    }
    if (!document.hidden) {
      void sync("focus");
    }
  }

  /** A sync may add, change or remove storage accounts: read them again. */
  async function sync(reason: "startup" | "focus" | "change"): Promise<void> {
    if (await autoSync.run(reason)) {
      accountsGeneration += 1;
      await loadAccounts();
    }
  }

  async function loadChrome(): Promise<void> {
    try {
      customChrome = (await getCurrentWindow().isDecorated()) === false;
    } catch {
      // No window to ask (the browser preview): the native title bar.
      customChrome = false;
    }
  }

  onMount(() => {
    void loadChrome();
    void loadMode();
    void loadAccounts();
    void updates.load();
    void sync("startup");
    void folderSyncs.start();
    return () => folderSyncs.stop();
  });
</script>

<svelte:window {onkeydown} onfocus={onshown} />
<svelte:document onvisibilitychange={onshown} />

<div class="shell" class:sidebar-open={sidebarOpen} class:custom-chrome={customChrome}>
  {#if customChrome}
    <TitleBar
      title={SECTION_TITLES[section]}
      {mode}
      {sidebarOpen}
      ontogglesidebar={() => (sidebarOpen = !sidebarOpen)}
    />
  {/if}
  <aside class="sidebar" aria-label="Navigation">
    <Sidebar
      showBrand={!customChrome}
      {syncStatus}
      {folderSync}
      {syncedAccounts}
      {section}
      {browsing}
      {accounts}
      {accountsError}
      onnavigate={(s) => void navigate(s)}
      onopenaccount={openAccount}
      onaddaccount={addAccount}
      onclose={() => (sidebarOpen = false)}
    />
  </aside>
  {#if sidebarOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scrim" onclick={() => (sidebarOpen = false)}></div>
  {/if}

  <main class="main" bind:this={main}>
    <div class="topbar">
      <button
        type="button"
        class="ghost icon"
        aria-label={sidebarOpen ? "Hide navigation" : "Show navigation"}
        aria-expanded={sidebarOpen}
        onclick={() => (sidebarOpen = !sidebarOpen)}><Icon name="sidebar" /></button
      >
      <span class="topbar-title">{SECTION_TITLES[section]}</span>
    </div>

    {#if updates.bannerVisible && ustatus !== null && !(section === "settings" && settingsTab === "about")}
      <div class="banner-wrap" class:wide={section === "cloud-storage" && browsing !== null}>
      <div class="update-banner" role="region" aria-label="Update notification">
        <span class="banner-icon" aria-hidden="true"><Icon name="refresh" size={16} /></span>
        <!-- A paragraph with inline content: WebKitGTK leaves a flex item
             span's text out of the accessibility tree. -->
        <p class="banner-text">
          {#if ustatus.installed !== null}
            <strong>sotyware {ustatus.installed} is installed.</strong>
            <span class="muted">Restart to start using it.</span>
          {:else if uinstall?.stage === "downloading"}
            {@const p = percent(uinstall.downloaded, uinstall.total)}
            <strong>Downloading sotyware {ustatus.available?.version ?? ""}…</strong>
            <span class="muted">{p === null ? formatSize(uinstall.downloaded) : `${p}%`}</span>
          {:else if uinstall?.stage === "verified" || uinstall?.stage === "installing"}
            <strong>Installing sotyware {ustatus.available?.version ?? ""}…</strong>
          {:else if ustatus.available}
            <strong>sotyware {ustatus.available.version} is available.</strong>
            {#if uinstall?.stage === "failed"}
              <span class="text-danger">The update failed; nothing was installed.</span>
            {:else if ustatus.blockedBy.length > 0}
              <span class="muted">Update when {ustatus.blockedBy.join(", ")} has finished.</span>
            {/if}
          {/if}
        </p>
        <div class="banner-actions">
          {#if ustatus.installed !== null}
            <button
              type="button"
              class="primary small"
              disabled={updates.restarting || ustatus.blockedBy.length > 0}
              onclick={() => void updates.restart()}>Restart now</button
            >
          {:else if !updates.running && ustatus.available}
            <button
              type="button"
              class="primary small"
              disabled={ustatus.blockedBy.length > 0}
              onclick={() => void updates.updateNow()}>Update</button
            >
            <button type="button" class="small" onclick={openUpdates}>Details</button>
            {#if !ustatus.postponed}
              <button type="button" class="small ghost" onclick={() => void updates.later()}>Later</button>
            {/if}
          {:else}
            <button type="button" class="small" onclick={openUpdates}>Details</button>
          {/if}
          <button type="button" class="ghost icon small" aria-label="Close the update notification" onclick={() => updates.closeBanner()}
            ><Icon name="close" size={14} /></button
          >
        </div>
      </div>
      </div>
    {/if}

    {#if section === "overview"}
      <Overview
        {mode}
        {modeError}
        {accounts}
        {accountsError}
        {updates}
        onnavigate={(s) => void navigate(s)}
        onopenaccount={openAccount}
        onaddaccount={addAccount}
        onopenupdates={openUpdates}
      />
    {:else if section === "cloud-storage"}
      <CloudStorage
        bind:browsing
        {requestAdd}
        onaddrequested={() => (requestAdd = false)}
        onaccountschange={(list) => {
          accounts = list;
          accountsError = null;
        }}
        onopenbackups={() => void navigate("backups")}
        onlocalchange={() => void sync("change")}
        generation={accountsGeneration}
        {folderSyncs}
      />
    {:else if section === "backups"}
      <Backup />
    {:else if section === "account"}
      <Account />
    {:else}
      <Settings tab={settingsTab} ontab={(t) => (settingsTab = t)} {updates} {mode} />
    {/if}
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    height: 100vh;
    overflow: hidden;
  }

  /* The page's own title bar across the top (Windows, ADR-040). */
  .shell.custom-chrome {
    grid-template-rows: var(--titlebar-h) minmax(0, 1fr);
    grid-template-areas:
      "titlebar titlebar"
      "sidebar main";
  }

  .custom-chrome .sidebar {
    grid-area: sidebar;
  }

  .custom-chrome .main {
    grid-area: main;
  }

  .sidebar {
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    min-height: 0;
    overflow: hidden;
  }

  .main {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    background: var(--bg);
  }

  .topbar {
    display: none;
  }

  .banner-wrap {
    max-width: var(--content-w);
    margin: var(--space-4) auto 0;
    padding: 0 var(--space-8);
  }

  .banner-wrap.wide {
    max-width: none;
  }

  .update-banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    font-size: var(--text-sm);
    background: var(--surface);
    border: 1px solid var(--info-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-sm);
  }

  .banner-icon {
    display: inline-flex;
    color: var(--accent-text);
  }

  .banner-text {
    flex: 1 1 14rem;
    min-width: 0;
    margin: 0;
  }

  .banner-text > :global(* + *) {
    margin-left: var(--space-2);
  }

  .banner-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .scrim {
    display: none;
  }

  @media (max-width: 760px) {
    .shell {
      grid-template-columns: minmax(0, 1fr);
    }

    .sidebar {
      position: fixed;
      z-index: var(--z-dialog);
      top: 0;
      bottom: 0;
      left: 0;
      width: min(var(--sidebar-w), 85vw);
      transform: translateX(-100%);
      transition: transform var(--dur) var(--ease);
      box-shadow: var(--shadow-lg);
      visibility: hidden;
    }

    .sidebar-open .sidebar {
      transform: none;
      visibility: visible;
    }

    .scrim {
      display: block;
      position: fixed;
      inset: 0;
      z-index: calc(var(--z-dialog) - 1);
      background: var(--overlay);
    }

    .topbar {
      position: sticky;
      top: 0;
      z-index: var(--z-sticky);
      display: flex;
      align-items: center;
      gap: var(--space-2);
      height: var(--topbar-h);
      padding: 0 var(--space-3);
      background: var(--bg);
      border-bottom: 1px solid var(--border);
    }

    .topbar-title {
      font-weight: var(--weight-semibold);
    }

    /* With the page's own title bar, it holds the navigation toggle and
       the page's name; the overlay sidebar opens below it, so the window
       controls stay reachable. */
    .shell.custom-chrome {
      grid-template-areas:
        "titlebar"
        "main";
    }

    .custom-chrome .topbar {
      display: none;
    }

    .custom-chrome .sidebar,
    .custom-chrome .scrim {
      top: var(--titlebar-h);
    }

    .custom-chrome .main :global(th),
    .custom-chrome .main :global(.selectionbar) {
      top: 0;
    }

    /* Sticky table headers and the browser's selection bar stop below the
       sticky top bar instead of sliding under it. */
    .main :global(th),
    .main :global(.selectionbar) {
      top: var(--topbar-h);
    }

    .banner-wrap {
      margin-top: var(--space-3);
      padding: 0 var(--space-4);
    }
  }
</style>
