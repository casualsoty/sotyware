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
  import CloudBulkDownload from "./CloudBulkDownload.svelte";
  import CloudDeleted from "./CloudDeleted.svelte";
  import CloudFolderPicker from "./CloudFolderPicker.svelte";
  import CloudOperation from "./CloudOperation.svelte";
  import CloudProperties from "./CloudProperties.svelte";
  import CloudUpload from "./CloudUpload.svelte";
  import {
    createFolder,
    listPlaces,
    recordRecent,
    searchFolder,
    setFavorite,
    unwatchDrops,
    watchDrops,
    type DropNotice,
    type ItemRef,
    type OperationRequest,
    type Place,
    type Places,
    type RestoreRef,
    type SearchHit,
    type SearchMode,
  } from "./cloudFiles";
  import { FIXED_BY_EDITING, describeError, describeOperationError } from "./cloudMessages";
  import {
    CATEGORIES,
    EMPTY_SELECTION,
    NO_FILTER,
    entryId,
    filterEntries,
    isFiltering,
    modeOf,
    moveFocus,
    parseExtensions,
    pruneSelection,
    select,
    selectAll,
    shortcutOf,
    sortEntries,
    typeLabel,
    type Category,
    type Entry,
    type Filter,
    type Selection,
    type SortDirection,
    type SortKey,
  } from "./fileList";
  import { formatSize, shownName } from "./format";
  import Modal from "./Modal.svelte";

  // Browser of one account's bucket, one folder level and one page at a
  // time. Folder prefixes are only ever ones Rust returned (or "" for the
  // root): the page never joins names into a prefix, so keys are kept
  // exactly. Files can be downloaded one at a time: Rust opens the save
  // dialog, so the page never names a local path. Files can be uploaded
  // into the folder shown (CloudUpload.svelte: Rust opens the file dialog
  // and keeps the paths).
  //
  // File management (ADR-026): selection (click, Ctrl/Shift-click, select
  // all, keyboard), a context menu and a bar for the selected items, new
  // folders, rename, copy and move (with a destination picker), delete and
  // restore, all through reviewed plans in Rust (CloudOperation.svelte);
  // downloads of several items (CloudBulkDownload.svelte); search, sorting
  // and filters; properties, previews and share links
  // (CloudProperties.svelte); favorites and recent items. Sorting and
  // filtering only reorder or hide what is loaded; the provider lists in key
  // order and cannot filter by size, date or type.

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
  let uploader: CloudUpload | undefined = $state();
  let uploadBusy = $state(false);

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

  // ------------------------------------------------------------ file manager

  let view = $state<"folder" | "search" | "deleted">("folder");
  let sortBy = $state<SortKey>("listed");
  let sortDirection = $state<SortDirection>("ascending");
  let filter = $state<Filter>({ ...NO_FILTER });
  let filterOpen = $state(false);
  let extensionsText = $state("");
  let minSizeText = $state("");
  let maxSizeText = $state("");
  let fromText = $state("");
  let toText = $state("");
  let selection = $state<Selection>(EMPTY_SELECTION);
  let focusId = $state<string | null>(null);
  let clipboard = $state<{ mode: "copy" | "cut"; items: ItemRef[]; names: string[] } | null>(null);
  let notice = $state<{ text: string; error: boolean } | null>(null);
  let operation: CloudOperation | undefined = $state();
  let bulk: CloudBulkDownload | undefined = $state();
  let opBusy = $state(false);
  let bulkBusy = $state(false);
  let deletedReload = $state(0);

  type Dialog =
    | { type: "new-folder"; error: unknown; saving: boolean }
    | { type: "rename"; entry: Entry }
    | { type: "picker"; mode: "copy" | "move"; items: ItemRef[]; moving: string[] }
    | { type: "properties"; key: string; fromSearch: boolean };
  let dialog = $state<Dialog | null>(null);
  /** The name typed in the new-folder or rename dialog. */
  let typedName = $state("");
  let menu = $state<{ x: number; y: number } | null>(null);

  // Search: results replace the folder's entries until cleared.
  let query = $state("");
  let searchScope = $state<"folder" | "bucket">("folder");
  let searchMode = $state<SearchMode>("name");
  let hits = $state<SearchHit[]>([]);
  let searchNext = $state<string | null>(null);
  let searching = $state(false);
  let searchError = $state<unknown>(null);
  let scanned = $state(0);
  let searched = $state<{ prefix: string; query: string; mode: SearchMode }>({ prefix: "", query: "", mode: "name" });
  let searchRequest = 0;

  let dropped = $state<DropNotice | null>(null);
  let places = $state<Places>({ favorites: [], recent: [] });
  let placesError = $state<unknown>(null);

  const entries = $derived<Entry[]>(
    view === "search"
      ? hits.map((h): Entry =>
          h.folder
            ? { kind: "folder", name: h.name, prefix: h.path }
            : { kind: "file", name: h.name, key: h.path, size: h.size ?? 0, lastModified: h.lastModified },
        )
      : [
          ...folders.map((f): Entry => ({ kind: "folder", name: f.name, prefix: f.prefix })),
          ...files.map((f): Entry => ({ kind: "file", name: f.name, key: f.key, size: f.size, lastModified: f.lastModified })),
        ],
  );
  const visible = $derived(sortEntries(filterEntries(entries, filter), sortBy, sortDirection));
  const order = $derived(visible.map(entryId));
  const selected = $derived(visible.filter((e) => selection.ids.has(entryId(e))));
  const favoriteSet = $derived(new Set(places.favorites.map((f) => f.path)));
  const hitParents = $derived(new Map(hits.map((h) => [h.path, h.parent])));

  function refOf(e: Entry): ItemRef {
    return e.kind === "folder" ? { type: "folder", prefix: e.prefix } : { type: "file", key: e.key };
  }

  function say(text: string, isError = false): void {
    notice = { text, error: isError };
  }

  function choose(e: Entry, event: MouseEvent | KeyboardEvent | null): void {
    const id = entryId(e);
    const mode = event === null ? "replace" : modeOf(event);
    selection = select(selection, id, mode, order);
    focusId = id;
  }

  function toggleOne(e: Entry): void {
    selection = select(selection, entryId(e), "toggle", order);
    focusId = entryId(e);
  }

  function toggleAll(): void {
    selection = selected.length === visible.length && visible.length > 0 ? EMPTY_SELECTION : selectAll(order);
  }

  function clearSelection(): void {
    selection = EMPTY_SELECTION;
  }

  /** Deleted items replace the folder's entries: what was selected there
   * is no longer shown, so it is no longer selected. */
  function toggleDeleted(): void {
    if (view === "deleted") {
      view = "folder";
    } else {
      view = "deleted";
      selection = EMPTY_SELECTION;
      focusId = null;
      menu = null;
    }
  }

  function activate(e: Entry): void {
    if (e.kind === "folder") {
      if (view === "search") {
        void open(e.prefix, []);
      } else {
        openFolder({ name: e.name, prefix: e.prefix });
      }
    } else {
      showProperties(e.key);
    }
  }

  function rowClick(e: Entry, event: MouseEvent): void {
    const target = event.target;
    if (target instanceof HTMLElement && target.closest("button, input, a, select")) {
      return;
    }
    choose(e, event);
  }

  function rowMenu(e: Entry, event: MouseEvent): void {
    event.preventDefault();
    if (!selection.ids.has(entryId(e))) {
      choose(e, null);
    }
    menu = { x: event.clientX, y: event.clientY };
  }

  function menuFor(e: Entry, event: MouseEvent): void {
    if (!selection.ids.has(entryId(e))) {
      choose(e, null);
    }
    const box = event.currentTarget instanceof HTMLElement ? event.currentTarget.getBoundingClientRect() : null;
    menu = { x: box?.left ?? 0, y: box?.bottom ?? 0 };
  }

  function closeMenu(): void {
    menu = null;
  }

  // ------------------------------------------------------------ actions

  async function loadPlaces(): Promise<void> {
    try {
      places = await listPlaces(account.id);
      placesError = null;
    } catch (e) {
      placesError = e;
    }
  }

  function remember(path: string): void {
    if (path === "") {
      return;
    }
    void recordRecent(account.id, bucket, path)
      .then(() => loadPlaces())
      .catch(() => undefined);
  }

  async function toggleFavorite(path: string): Promise<void> {
    try {
      places = await setFavorite(account.id, bucket, path, !favoriteSet.has(path));
    } catch (e) {
      say(describeOperationError(e), true);
    }
  }

  function openPlace(p: Place): void {
    if (p.folder) {
      void open(p.path, []);
      remember(p.path);
    } else {
      showProperties(p.path, true);
    }
  }

  function showProperties(key: string, fromSearch = view === "search"): void {
    closeMenu();
    dialog = { type: "properties", key, fromSearch };
    remember(key);
  }

  function showInFolder(key: string): void {
    const parent = hitParents.get(key) ?? key.slice(0, key.lastIndexOf("/") + 1);
    dialog = null;
    void open(parent, []).then(() => {
      selection = { ids: new Set([key]), anchor: key };
      focusId = key;
    });
  }

  function downloadSelected(): void {
    closeMenu();
    const items = selected;
    const only = items[0];
    if (items.length === 1 && only !== undefined && only.kind === "file") {
      const file = files.find((f) => f.key === only.key) ?? { name: only.name, key: only.key, size: only.size, lastModified: only.lastModified };
      void startDownload(file);
      remember(only.key);
      return;
    }
    void bulk?.start(items.map(refOf));
  }

  function copyOrMoveTo(mode: "copy" | "move"): void {
    closeMenu();
    if (selected.length === 0) {
      return;
    }
    dialog = {
      type: "picker",
      mode,
      items: selected.map(refOf),
      moving: mode === "move" ? selected.filter((e) => e.kind === "folder").map(entryId) : [],
    };
  }

  function picked(prefixChosen: string): void {
    if (dialog?.type !== "picker") {
      return;
    }
    const { mode, items } = dialog;
    dialog = null;
    void operation?.run({ kind: mode, items, destination: prefixChosen });
  }

  function rechoose(request: OperationRequest): void {
    if (request.kind === "copy" || request.kind === "move") {
      dialog = {
        type: "picker",
        mode: request.kind,
        items: request.items,
        moving: request.kind === "move" ? request.items.flatMap((i) => (i.type === "folder" ? [i.prefix] : [])) : [],
      };
    }
  }

  function deleteSelected(): void {
    closeMenu();
    if (selected.length > 0) {
      void operation?.run({ kind: "delete", items: selected.map(refOf) });
    }
  }

  function renameSelected(): void {
    closeMenu();
    const only = selected[0];
    if (selected.length === 1 && only !== undefined) {
      typedName = only.name;
      dialog = { type: "rename", entry: only };
    }
  }

  function submitRename(): void {
    if (dialog?.type !== "rename") {
      return;
    }
    const { entry } = dialog;
    const name = typedName;
    dialog = null;
    if (name !== entry.name) {
      void operation?.run({ kind: "rename", item: refOf(entry), newName: name });
    }
  }

  function restore(items: RestoreRef[]): void {
    void operation?.run({ kind: "restore", items });
  }

  function clip(mode: "copy" | "cut"): void {
    closeMenu();
    if (selected.length === 0) {
      return;
    }
    clipboard = { mode, items: selected.map(refOf), names: selected.map((e) => e.name) };
    say(
      `${selected.length} item${selected.length === 1 ? "" : "s"} ${mode === "copy" ? "copied" : "marked to move"}; open a folder and paste (Ctrl+V).`,
    );
  }

  function paste(): void {
    if (clipboard === null || view !== "folder") {
      return;
    }
    const c = clipboard;
    void operation?.run({ kind: c.mode === "copy" ? "copy" : "move", items: c.items, destination: prefix });
    if (c.mode === "cut") {
      clipboard = null;
    }
  }

  async function submitNewFolder(): Promise<void> {
    if (dialog?.type !== "new-folder" || dialog.saving) {
      return;
    }
    const d = dialog;
    d.saving = true;
    d.error = null;
    try {
      const made = await createFolder(account.id, bucket, prefix, typedName);
      dialog = null;
      await open(prefix, crumbs);
      selection = { ids: new Set([made.prefix]), anchor: made.prefix };
      focusId = made.prefix;
      say(`Folder “${shownName(made.name)}” created.`);
    } catch (e) {
      if (dialog?.type === "new-folder") {
        dialog.saving = false;
        dialog.error = e;
      }
    }
  }

  function changed(): void {
    if (view === "search") {
      void runSearch(false);
    } else if (view === "deleted") {
      deletedReload++;
      refreshQuiet();
    } else {
      refresh();
    }
  }

  // ------------------------------------------------------------ search

  async function runSearch(more: boolean): Promise<void> {
    const text = more ? searched.query : query.trim();
    if (text === "") {
      return;
    }
    const mine = ++searchRequest;
    if (!more) {
      searched = { prefix: searchScope === "bucket" ? "" : prefix, query: text, mode: searchMode };
      hits = [];
      searchNext = null;
      scanned = 0;
      selection = EMPTY_SELECTION;
    }
    view = "search";
    searching = true;
    searchError = null;
    try {
      const r = await searchFolder(account.id, bucket, searched.prefix, searched.query, searched.mode, more ? searchNext : null);
      if (mine !== searchRequest || destroyed) {
        return;
      }
      const known = new Set(hits.map((h) => h.path));
      hits = [...hits, ...r.hits.filter((h) => !known.has(h.path))];
      searchNext = r.next;
      scanned += r.scanned;
      selection = pruneSelection(selection, hits.map((h) => h.path));
    } catch (e) {
      if (mine === searchRequest && !destroyed) {
        searchError = e;
      }
    } finally {
      if (mine === searchRequest && !destroyed) {
        searching = false;
      }
    }
  }

  function clearSearch(): void {
    searchRequest++;
    query = "";
    hits = [];
    searchNext = null;
    searchError = null;
    searching = false;
    if (view === "search") {
      view = "folder";
      selection = pruneSelection(selection, [...folders.map((f) => f.prefix), ...files.map((f) => f.key)]);
    }
  }

  // ------------------------------------------------------------ filters

  function sizeOf(text: string): number | null {
    const t = text.trim().toLowerCase();
    if (t === "") {
      return null;
    }
    const m = /^(\d+(?:[.,]\d+)?)\s*(b|kb|kib|mb|mib|gb|gib)?$/.exec(t);
    if (m === null || m[1] === undefined) {
      return null;
    }
    const n = Number.parseFloat(m[1].replace(",", "."));
    const unit = m[2] ?? "b";
    const factor = unit.startsWith("k") ? 1024 : unit.startsWith("m") ? 1024 ** 2 : unit.startsWith("g") ? 1024 ** 3 : 1;
    return Math.round(n * factor);
  }

  function dayOf(text: string, end: boolean): number | null {
    if (text === "") {
      return null;
    }
    const t = Date.parse(`${text}T${end ? "23:59:59.999" : "00:00:00"}`);
    return Number.isNaN(t) ? null : t;
  }

  function applyFilter(): void {
    filter = {
      ...filter,
      extensions: parseExtensions(extensionsText),
      minSize: sizeOf(minSizeText),
      maxSize: sizeOf(maxSizeText),
      modifiedFrom: dayOf(fromText, false),
      modifiedTo: dayOf(toText, true),
    };
    selection = pruneSelection(selection, order);
  }

  function toggleCategory(c: Category): void {
    const has = filter.categories.includes(c);
    filter = { ...filter, categories: has ? filter.categories.filter((x) => x !== c) : [...filter.categories, c] };
  }

  function resetFilter(): void {
    filter = { ...NO_FILTER };
    extensionsText = "";
    minSizeText = "";
    maxSizeText = "";
    fromText = "";
    toText = "";
  }

  // ------------------------------------------------------------ keyboard

  function onkeydown(event: KeyboardEvent): void {
    // A dialog (rename, picker, review, properties) owns the keyboard. In
    // Deleted items the folder's entries are not shown, so no shortcut may
    // act on them (Delete, F2, Ctrl+A, Ctrl+X …).
    if (dialog !== null || document.querySelector('[role="dialog"]') !== null || view === "deleted") {
      return;
    }
    const action = shortcutOf(event);
    if (action === null) {
      return;
    }
    const focused = visible.find((e) => entryId(e) === focusId) ?? null;
    switch (action) {
      case "escape":
        if (menu !== null) {
          closeMenu();
        } else if (selection.ids.size > 0) {
          clearSelection();
        } else {
          return;
        }
        break;
      case "select-all":
        selection = selectAll(order);
        break;
      case "copy":
        clip("copy");
        break;
      case "cut":
        clip("cut");
        break;
      case "paste":
        paste();
        break;
      case "delete":
        deleteSelected();
        break;
      case "rename":
        renameSelected();
        break;
      case "refresh":
        changed();
        break;
      case "open":
        if (focused !== null) {
          activate(focused);
        } else {
          return;
        }
        break;
      case "up":
      case "down": {
        const id = moveFocus(order, focusId, action === "up" ? -1 : 1);
        if (id !== null) {
          selection = select(selection, id, "replace", order);
          focusId = id;
          focusRow(id);
        }
        break;
      }
      case "extend-up":
      case "extend-down": {
        const id = moveFocus(order, focusId, action === "extend-up" ? -1 : 1);
        if (id !== null) {
          selection = select(selection, id, selection.anchor === null ? "replace" : "range", order);
          focusId = id;
          focusRow(id);
        }
        break;
      }
    }
    event.preventDefault();
  }

  function focusRow(id: string): void {
    const row = [...document.querySelectorAll<HTMLElement>("tr[data-id]")].find((r) => r.dataset["id"] === id);
    row?.focus();
  }

  function onwindowclick(event: MouseEvent): void {
    // Only a primary click elsewhere closes the menu: some engines
    // (WebKitGTK) also send "click" for the right button that opened it.
    if (menu !== null && event.button === 0) {
      closeMenu();
    }
  }

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
    if (target !== prefix) {
      selection = EMPTY_SELECTION;
      focusId = null;
    }
    if (view !== "deleted") {
      view = "folder";
    }
    searchRequest++;
    hits = [];
    searchNext = null;
    searching = false;
    searchError = null;
    menu = null;
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
      selection = pruneSelection(selection, [...listing.folders.map((f) => f.prefix), ...listing.files.map((f) => f.key)]);
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

  /** Reloads the folder without leaving the deleted-items view. */
  function refreshQuiet(): void {
    void open(prefix, crumbs);
  }

  function openFolder(folder: FolderEntry): void {
    void open(folder.prefix, [...crumbs, { name: folder.name, prefix: folder.prefix }]);
    remember(folder.prefix);
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
    switch (check) {
      case "sha256-verified":
        return "The SHA-256 checksum stored with the file matched.";
      case "size-only":
        return "The size matched. This file has no SHA-256 checksum stored by sotyware, so its content could not be verified.";
      default:
        return "The transfer completed, but the provider announced neither a size nor a checksum, so nothing could be verified.";
    }
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
    void loadPlaces();
    void watchDrops((notice) => {
      if (!destroyed) {
        dropped = notice;
      }
    }).catch(() => undefined);
  });

  onDestroy(() => {
    destroyed = true;
    void unwatchDrops().catch(() => undefined);
    // Leaving the browser cancels a running download: no page would show it.
    const d = download;
    if (d !== null && d.state === "active" && d.transfer !== null) {
      void cancelDownload(d.transfer).catch(() => undefined);
    }
  });
</script>

<svelte:window {onkeydown} onclick={onwindowclick} />

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

  <div class="actions toolbar" role="toolbar" aria-label="Folder actions">
    <button type="button" disabled={parentCrumbs === null} onclick={goUp}>Up one folder</button>
    <button type="button" onclick={refresh} title="Refresh (Ctrl+R)">Refresh</button>
    <button
      type="button"
      class="upload-action"
      disabled={uploadBusy || uploader === undefined}
      title={uploadBusy ? "An upload is running" : "Upload files into this folder"}
      onclick={() => uploader?.start(bucket, prefix, crumbs)}>Upload files…</button
    >
    <button
      type="button"
      disabled={uploadBusy || uploader === undefined}
      title={uploadBusy ? "An upload is running" : "Upload a folder and everything in it into this folder"}
      onclick={() => uploader?.startFolder(bucket, prefix, crumbs)}>Upload folder…</button
    >
    <button type="button" onclick={() => ((typedName = ""), (dialog = { type: "new-folder", error: null, saving: false }))}>New folder…</button>
    {#if clipboard !== null}
      <button type="button" disabled={opBusy || view !== "folder"} title="Paste (Ctrl+V)" onclick={paste}>
        {clipboard.mode === "copy" ? "Paste copy" : "Move here"} ({clipboard.items.length})
      </button>
      <button type="button" onclick={() => (clipboard = null)}>Clear clipboard</button>
    {/if}
    <button type="button" aria-pressed={view === "deleted"} onclick={toggleDeleted}>
      Deleted items
    </button>
  </div>

  <form
    class="search"
    role="search"
    onsubmit={(e) => {
      e.preventDefault();
      void runSearch(false);
    }}
  >
    <input type="search" aria-label="Search" placeholder="Search names…" bind:value={query} />
    <select aria-label="Search where" value={searchScope} onchange={(e) => (searchScope = e.currentTarget.value === "bucket" ? "bucket" : "folder")}>
      <option value="folder">In this folder and below</option>
      <option value="bucket">In the whole bucket</option>
    </select>
    <select aria-label="Search how" value={searchMode} onchange={(e) => (searchMode = e.currentTarget.value === "prefix" ? "prefix" : "name")}>
      <option value="name">Name contains</option>
      <option value="prefix">Key starts with</option>
    </select>
    <button type="submit" disabled={query.trim() === ""}>Search</button>
    {#if view === "search"}
      <button type="button" onclick={clearSearch}>Clear search</button>
    {/if}
  </form>

  <details class="places">
    <summary>Favorites ({places.favorites.length}) and recent ({places.recent.length})</summary>
    {#if placesError !== null}
      <p role="alert">{describeOperationError(placesError)}</p>
    {/if}
    <div class="place-lists">
      <div>
        <h4>Favorites</h4>
        <ul>
          {#each places.favorites as p (p.path)}
            <li>
              <button type="button" class="link" onclick={() => openPlace(p)}>
                <span aria-hidden="true">{p.folder ? "📁" : "📄"}</span> {shownName(p.name)}
              </button>
              <span class="where">{p.parent === "" ? bucket : p.parent}</span>
              <button type="button" class="small" aria-label={`Remove ${shownName(p.name)} from favorites`} onclick={() => void toggleFavorite(p.path)}>✕</button>
            </li>
          {:else}
            <li class="none">No favorites yet. Use “Add to favorites” in a file's or folder's menu.</li>
          {/each}
        </ul>
      </div>
      <div>
        <h4>Recent</h4>
        <ul>
          {#each places.recent as p (p.path)}
            <li>
              <button type="button" class="link" onclick={() => openPlace(p)}>
                <span aria-hidden="true">{p.folder ? "📁" : "📄"}</span> {shownName(p.name)}
              </button>
              <span class="where">{p.parent === "" ? bucket : p.parent}</span>
            </li>
          {:else}
            <li class="none">Nothing opened yet.</li>
          {/each}
        </ul>
      </div>
    </div>
  </details>

  <CloudUpload
    bind:this={uploader}
    bind:busy={uploadBusy}
    {account}
    {onclose}
    {onedit}
    onshowfolder={(target, path) => void open(target, path)}
  />

  <CloudOperation bind:this={operation} bind:busy={opBusy} {account} {bucket} onchanged={changed} onrechoose={rechoose} {onedit} />

  <CloudBulkDownload bind:this={bulk} bind:busy={bulkBusy} {account} {bucket} />

  {#if dropped !== null}
    {@const d = dropped ?? { drop: 0, files: 0, folders: 0, names: [] }}
    <div class="notice drop" role="region" aria-label="Dropped files">
      <p>
        Dropped: {d.files} file{d.files === 1 ? "" : "s"}{d.folders > 0 ? ` and ${d.folders} folder${d.folders === 1 ? "" : "s"} (with everything in them)` : ""}
        — {d.names.slice(0, 5).map((n) => shownName(n)).join(", ")}{d.names.length > 5 ? ", …" : ""}.
        Upload into {prefix === "" ? `the root of ${bucket}` : prefix}? Nothing that already exists is replaced.
      </p>
      <div class="actions">
        <button
          type="button"
          disabled={uploadBusy || view !== "folder"}
          onclick={() => {
            const id = dropped?.drop;
            dropped = null;
            if (id !== undefined) {
              void uploader?.startDrop(bucket, prefix, crumbs, id);
            }
          }}>Upload here</button
        >
        <button type="button" onclick={() => (dropped = null)}>Ignore</button>
      </div>
    </div>
  {/if}

  {#if notice !== null}
    <p class="notice" class:error={notice.error} role={notice.error ? "alert" : "status"}>
      {notice.text}
      <button type="button" class="small" aria-label="Dismiss message" onclick={() => (notice = null)}>✕</button>
    </p>
  {/if}

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

  {#if view === "deleted"}
    <CloudDeleted {account} {bucket} {prefix} reload={deletedReload} onrestore={restore} onclose={() => (view = "folder")} />
  {:else}
  {#if view === "search"}
    <p class="search-status" role="status">
      {#if searching}
        Searching {searched.prefix === "" ? "the whole bucket" : searched.prefix}…
      {:else}
        {hits.length} result{hits.length === 1 ? "" : "s"} for “{searched.query}” ({searched.mode === "name" ? "names containing it" : "keys starting with it"})
        in {searched.prefix === "" ? "the whole bucket" : searched.prefix}; {scanned.toLocaleString()} objects looked at{searchNext !== null ? ", more to search" : ""}.
        S3 cannot search file contents.
      {/if}
    </p>
    {#if searchError !== null}
      <div class="problem" role="alert"><p>{describeError(searchError)}</p></div>
    {/if}
  {/if}

  <div class="listbar">
    <label>
      Sort by
      <select value={sortBy} onchange={(e) => (sortBy = e.currentTarget.value as SortKey)}>
        <option value="listed">Bucket order</option>
        <option value="name">Name</option>
        <option value="size">Size</option>
        <option value="modified">Last modified</option>
        <option value="type">Type</option>
      </select>
    </label>
    <button
      type="button"
      aria-label={sortDirection === "ascending" ? "Ascending; switch to descending" : "Descending; switch to ascending"}
      onclick={() => (sortDirection = sortDirection === "ascending" ? "descending" : "ascending")}
      >{sortDirection === "ascending" ? "↑ Ascending" : "↓ Descending"}</button
    >
    <button type="button" aria-expanded={filterOpen} onclick={() => (filterOpen = !filterOpen)}>
      Filter{isFiltering(filter) ? " (on)" : ""}
    </button>
    {#if isFiltering(filter)}
      <button type="button" onclick={resetFilter}>Show everything</button>
    {/if}
    <span class="hint">Sorting and filtering apply to the {entries.length.toLocaleString()} items loaded.</span>
  </div>

  {#if filterOpen}
    <fieldset class="filters">
      <legend>Show only</legend>
      <label><input type="radio" name="show" value="all" bind:group={filter.show} /> Files and folders</label>
      <label><input type="radio" name="show" value="files" bind:group={filter.show} /> Files</label>
      <label><input type="radio" name="show" value="folders" bind:group={filter.show} /> Folders</label>
      <div class="cats">
        {#each CATEGORIES as c (c)}
          <label><input type="checkbox" checked={filter.categories.includes(c)} onchange={() => toggleCategory(c)} /> {c}</label>
        {/each}
      </div>
      <form
        class="ranges"
        onsubmit={(e) => {
          e.preventDefault();
          applyFilter();
        }}
      >
        <label>Extensions <input type="text" placeholder="pdf, jpg" bind:value={extensionsText} /></label>
        <label>Min size <input type="text" placeholder="e.g. 10 MB" bind:value={minSizeText} /></label>
        <label>Max size <input type="text" placeholder="e.g. 1 GB" bind:value={maxSizeText} /></label>
        <label>Modified from <input type="date" bind:value={fromText} /></label>
        <label>to <input type="date" bind:value={toText} /></label>
        <button type="submit">Apply</button>
      </form>
    </fieldset>
  {/if}

  {#if selected.length > 0}
    <div class="selectionbar" role="toolbar" aria-label="Selected items">
      <span>{selected.length} selected</span>
      <button type="button" disabled={bulkBusy || downloadBusy} onclick={downloadSelected}>Download</button>
      <button type="button" disabled={opBusy} onclick={() => copyOrMoveTo("copy")}>Copy to…</button>
      <button type="button" disabled={opBusy} onclick={() => copyOrMoveTo("move")}>Move to…</button>
      <button type="button" disabled={opBusy || selected.length !== 1} onclick={renameSelected}>Rename</button>
      <button type="button" class="danger" disabled={opBusy} onclick={deleteSelected}>Delete</button>
      {#if selected.length === 1 && selected[0]?.kind === "file"}
        <button type="button" onclick={() => selected[0]?.kind === "file" && showProperties(selected[0].key)}>Properties</button>
      {/if}
      {#if selected.length === 1 && selected[0] !== undefined}
        {@const only = selected[0]}
        <button type="button" onclick={() => void toggleFavorite(entryId(only))}>
          {favoriteSet.has(entryId(only)) ? "Remove from favorites" : "Add to favorites"}
        </button>
      {/if}
      <button type="button" onclick={clearSelection}>Clear selection</button>
    </div>
  {/if}

  {#if view === "search"}
    {#if visible.length > 0}
      {@render table()}
    {:else if !searching && searchError === null}
      <div class="empty" role="status"><p><strong>Nothing found.</strong></p></div>
    {/if}
    {#if searchNext !== null && !searching}
      <button type="button" onclick={() => void runSearch(true)}>Search further</button>
    {/if}
  {:else if loading === "folder"}
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
  {:else if visible.length === 0}
    <div class="empty" role="status">
      <p><strong>No items match the filter.</strong></p>
      <button type="button" onclick={resetFilter}>Show everything</button>
    </div>
  {:else}
    {@render table()}
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
  {/if}
</section>

{#snippet table()}
  <table>
    <thead>
      <tr>
        <th scope="col">Name</th>
        <th scope="col" class="num">Size</th>
        <th scope="col">Last modified</th>
        <th scope="col">Type</th>
        <th scope="col">
          <input
            type="checkbox"
            aria-label="Select all shown"
            checked={selected.length > 0 && selected.length === visible.length}
            indeterminate={selected.length > 0 && selected.length < visible.length}
            onchange={toggleAll}
          />
        </th>
        <th scope="col"><span class="visually-hidden">Actions</span></th>
      </tr>
    </thead>
    <tbody>
      {#each visible as entry (entryId(entry))}
        {@const id = entryId(entry)}
        {@const isSelected = selection.ids.has(id)}
        {#if entry.kind === "folder"}
          <tr
            class="folder"
            class:selected={isSelected}
            class:cut={clipboard?.mode === "cut" && clipboard.items.some((i) => i.type === "folder" && i.prefix === id)}
            data-prefix={entry.prefix}
            data-id={id}
            tabindex={focusId === id ? 0 : -1}
            onclick={(e) => rowClick(entry, e)}
            ondblclick={() => activate(entry)}
            oncontextmenu={(e) => rowMenu(entry, e)}
          >
            <td>
              <button type="button" class="link name" onclick={() => activate(entry)}>
                <span aria-hidden="true">📁</span>
                {shownName(entry.name)}
              </button>
              {#if view === "search"}<span class="where">in {(hitParents.get(id) ?? "") === "" ? bucket : hitParents.get(id)}</span>{/if}
            </td>
            <td class="num">Folder</td>
            <td>—</td>
            <td>{typeLabel(entry)}</td>
            <td><input type="checkbox" aria-label={`Select ${shownName(entry.name)}`} checked={isSelected} onchange={() => toggleOne(entry)} /></td>
            <td class="row-actions">
              <button type="button" class="menu-action" aria-label={`More actions for ${shownName(entry.name)}`} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); menuFor(entry, e); }}>⋯</button>
            </td>
          </tr>
        {:else}
          <tr
            class="file"
            class:selected={isSelected}
            class:cut={clipboard?.mode === "cut" && clipboard.items.some((i) => i.type === "file" && i.key === id)}
            data-key={entry.key}
            data-id={id}
            tabindex={focusId === id ? 0 : -1}
            onclick={(e) => rowClick(entry, e)}
            ondblclick={() => activate(entry)}
            oncontextmenu={(e) => rowMenu(entry, e)}
          >
            <td class="name">{shownName(entry.name)}{#if view === "search"}<span class="where">in {(hitParents.get(id) ?? "") === "" ? bucket : hitParents.get(id)}</span>{/if}</td>
            <td class="num" title={`${entry.size.toLocaleString()} bytes`}>{formatSize(entry.size)}</td>
            <td>{formatDate(entry.lastModified)}</td>
            <td>{typeLabel(entry)}</td>
            <td><input type="checkbox" aria-label={`Select ${shownName(entry.name)}`} checked={isSelected} onchange={() => toggleOne(entry)} /></td>
            <td class="row-actions">
              <button
                type="button"
                class="download-action"
                disabled={downloadBusy}
                aria-label={`Download ${shownName(entry.name)}`}
                title={downloadBusy ? "Another download is running" : "Download this file"}
                onclick={() => {
                  const f = files.find((x) => x.key === entry.key) ?? { name: entry.name, key: entry.key, size: entry.size, lastModified: entry.lastModified };
                  remember(entry.key);
                  void startDownload(f);
                }}>Download</button
              >
              <button type="button" class="menu-action" aria-label={`More actions for ${shownName(entry.name)}`} aria-haspopup="menu" onclick={(e) => { e.stopPropagation(); menuFor(entry, e); }}>⋯</button>
            </td>
          </tr>
        {/if}
      {/each}
    </tbody>
  </table>
{/snippet}

{#if menu !== null && selected.length > 0}
  {@const only = selected.length === 1 ? selected[0] : undefined}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="context-menu" role="menu" aria-label="Actions" tabindex="-1" style={`left:${menu.x}px;top:${menu.y}px`} onclick={(e) => e.stopPropagation()}>
    {#if only !== undefined}
      <button type="button" role="menuitem" onclick={() => { closeMenu(); activate(only); }}>{only.kind === "folder" ? "Open" : "Properties and preview"}</button>
    {/if}
    <button type="button" role="menuitem" disabled={bulkBusy || downloadBusy} onclick={downloadSelected}>Download{selected.length > 1 ? ` ${selected.length} items` : ""}…</button>
    <button type="button" role="menuitem" disabled={opBusy} onclick={() => copyOrMoveTo("copy")}>Copy to…</button>
    <button type="button" role="menuitem" disabled={opBusy} onclick={() => copyOrMoveTo("move")}>Move to…</button>
    <button type="button" role="menuitem" onclick={() => clip("copy")}>Copy (Ctrl+C)</button>
    <button type="button" role="menuitem" onclick={() => clip("cut")}>Cut (Ctrl+X)</button>
    {#if only !== undefined}
      <button type="button" role="menuitem" disabled={opBusy} onclick={renameSelected}>Rename (F2)</button>
      <button type="button" role="menuitem" onclick={() => { closeMenu(); void toggleFavorite(entryId(only)); }}>
        {favoriteSet.has(entryId(only)) ? "Remove from favorites" : "Add to favorites"}
      </button>
      {#if view === "search"}
        <button type="button" role="menuitem" onclick={() => { closeMenu(); showInFolder(entryId(only)); }}>Show in folder</button>
      {/if}
    {/if}
    <button type="button" role="menuitem" class="danger" disabled={opBusy} onclick={deleteSelected}>Delete (Del)</button>
  </div>
{/if}

{#if dialog?.type === "new-folder"}
  <Modal title="New folder" onclose={() => (dialog = null)} locked={dialog?.type === "new-folder" && dialog.saving}>
    <form
      id="new-folder-form"
      onsubmit={(e) => {
        e.preventDefault();
        void submitNewFolder();
      }}
    >
      <label class="field">Name <input type="text" bind:value={typedName} data-autofocus /></label>
      <p class="hint">Created in {prefix === "" ? `the root of ${bucket}` : prefix}. Folders are part of object keys; sotyware stores an empty marker object so the folder shows while it is empty.</p>
      {#if dialog?.type === "new-folder" && dialog.error !== null}
        <p role="alert">{describeOperationError(dialog.error)}</p>
      {/if}
    </form>
    {#snippet footer()}
      <button type="button" onclick={() => (dialog = null)}>Cancel</button>
      <button type="submit" form="new-folder-form" disabled={dialog?.type !== "new-folder" || typedName === "" || dialog.saving}>Create folder</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "rename"}
  <Modal title={dialog?.type === "rename" ? `Rename “${shownName(dialog.entry.name)}”` : "Rename"} onclose={() => (dialog = null)}>
    <form
      id="rename-form"
      onsubmit={(e) => {
        e.preventDefault();
        submitRename();
      }}
    >
      <label class="field">New name <input type="text" bind:value={typedName} data-autofocus /></label>
      {#if dialog?.type === "rename" && dialog.entry.kind === "folder"}
        <p class="hint">Renaming a folder copies every object in it to the new name and then deletes the originals; you will see how many first.</p>
      {/if}
    </form>
    {#snippet footer()}
      <button type="button" onclick={() => (dialog = null)}>Cancel</button>
      <button type="submit" form="rename-form" disabled={dialog?.type !== "rename" || typedName === "" || typedName === dialog.entry.name}>Rename</button>
    {/snippet}
  </Modal>
{:else if dialog?.type === "picker"}
  <CloudFolderPicker
    {account}
    {bucket}
    title={dialog?.type !== "picker"
      ? ""
      : `${dialog.mode === "copy" ? "Copy" : "Move"} ${dialog.items.length} item${dialog.items.length === 1 ? "" : "s"} to…`}
    confirm={dialog?.type === "picker" && dialog.mode === "move" ? "Move to" : "Copy to"}
    start={view === "search" ? "" : prefix}
    startCrumbs={view === "search" ? [] : crumbs}
    moving={dialog?.type === "picker" ? dialog.moving : []}
    onchoose={picked}
    onclose={() => (dialog = null)}
  />
{:else if dialog?.type === "properties"}
  {@const key = dialog?.type === "properties" ? dialog.key : ""}
  <CloudProperties
    {account}
    {bucket}
    fileKey={key}
    favorite={favoriteSet.has(key)}
    ondownload={() => {
      dialog = null;
      void startDownload(files.find((f) => f.key === key) ?? { name: key.split("/").at(-1) ?? key, key, size: 0, lastModified: null });
    }}
    onshowinfolder={dialog?.type === "properties" && dialog.fromSearch ? () => showInFolder(key) : null}
    ontogglefavorite={() => void toggleFavorite(key)}
    onclose={() => (dialog = null)}
  />
{/if}

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

  .toolbar,
  .listbar,
  .selectionbar,
  .search {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    align-items: center;
    margin: 0.5rem 0;
  }

  .search input[type="search"] {
    flex: 1;
    min-width: 12rem;
    font: inherit;
    padding: 0.35rem 0.5rem;
  }

  select,
  input[type="text"],
  input[type="date"] {
    font: inherit;
    padding: 0.3rem 0.4rem;
  }

  .listbar .hint,
  .hint,
  .where,
  .search-status {
    color: #6b6b73;
    font-size: 0.88em;
  }

  .where {
    display: block;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .selectionbar {
    position: sticky;
    top: 0;
    z-index: 2;
    padding: 0.4rem 0.6rem;
    background: #e3f2fd;
    border-radius: 4px;
  }

  .selectionbar span {
    font-weight: 600;
    margin-right: 0.5rem;
  }

  button.danger {
    color: #b71c1c;
  }

  button.small {
    padding: 0 0.4rem;
    font-size: 0.85em;
  }

  .filters {
    border: 1px solid rgb(0 0 0 / 15%);
    border-radius: 4px;
    margin: 0.5rem 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
  }

  .filters .cats,
  .filters .ranges {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.8rem;
    width: 100%;
    align-items: end;
  }

  .filters .ranges label {
    display: flex;
    flex-direction: column;
    font-size: 0.9em;
  }

  .places {
    margin: 0.5rem 0;
  }

  .places summary {
    cursor: pointer;
    color: #1565c0;
  }

  .place-lists {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.5rem 1.5rem;
  }

  .place-lists h4 {
    margin: 0.5rem 0 0.25rem;
    font-size: 0.95em;
  }

  .place-lists ul {
    list-style: none;
    padding: 0;
    margin: 0;
  }

  .place-lists li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    align-items: baseline;
    padding: 0.15rem 0;
  }

  .place-lists .none {
    color: #6b6b73;
    font-size: 0.9em;
  }

  tbody tr {
    cursor: default;
  }

  tbody tr:hover {
    background: rgb(0 0 0 / 3%);
  }

  tbody tr.selected {
    background: #e3f2fd;
  }

  tbody tr.cut {
    opacity: 0.55;
  }

  tbody tr:focus {
    outline: 2px solid #1565c0;
    outline-offset: -2px;
  }

  button:focus-visible,
  input:focus-visible,
  select:focus-visible,
  summary:focus-visible {
    outline: 2px solid #1565c0;
    outline-offset: 2px;
  }

  .menu-action {
    margin-left: 0.25rem;
  }

  .context-menu {
    position: fixed;
    z-index: 15;
    display: flex;
    flex-direction: column;
    min-width: 13rem;
    padding: 0.25rem 0;
    background: #fff;
    border: 1px solid rgb(0 0 0 / 15%);
    border-radius: 4px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 20%);
  }

  .context-menu button {
    text-align: left;
    border: none;
    background: none;
    padding: 0.35rem 0.9rem;
  }

  .context-menu button:hover:not(:disabled),
  .context-menu button:focus-visible {
    background: #e3f2fd;
  }

  .notice {
    margin: 0.5rem 0;
    padding: 0.35rem 0.75rem;
    border-left: 3px solid #1565c0;
    background: rgb(0 0 0 / 4%);
    overflow-wrap: anywhere;
  }

  .notice.error {
    border-left-color: #c62828;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  @media (prefers-color-scheme: dark) {
    button.link {
      color: #90caf9;
    }

    .selectionbar,
    tbody tr.selected,
    .context-menu button:hover:not(:disabled),
    .context-menu button:focus-visible {
      background: rgb(144 202 249 / 18%);
    }

    tbody tr:hover {
      background: rgb(255 255 255 / 4%);
    }

    .context-menu {
      background: #2a2a30;
      border-color: rgb(255 255 255 / 15%);
    }

    .places summary {
      color: #90caf9;
    }

    button.danger {
      color: #ef9a9a;
    }

    .hint,
    .where,
    .search-status,
    .place-lists .none {
      color: #a0a0a8;
    }

    .notice {
      background: rgb(255 255 255 / 6%);
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
