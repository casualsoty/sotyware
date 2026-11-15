// Headless tests of the Cloud Storage page, its form and its typed command
// wrappers. The Rust side is Tauri's IPC mock playing the commands of
// src-tauri/src/cloud/dashboard.rs with the same shapes; the real validation,
// credential store and connection check are tested in Rust.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "./App.svelte";
import CloudStorage from "./CloudStorage.svelte";
import {
  ADD_CLOUD_ACCOUNT,
  CHECK_CLOUD_ACCOUNT,
  CHECK_STATUSES,
  CloudCommandError,
  LIST_CLOUD_ACCOUNTS,
  LIST_CLOUD_PROVIDERS,
  REMOVE_CLOUD_ACCOUNT,
  UPDATE_CLOUD_ACCOUNT,
  parseAccount,
  parseAccountList,
  parseProvider,
  toCommandError,
  type AccountInfo,
  type CheckStatus,
  type ConnectionCheck,
  type ProviderInfo,
} from "./cloud";
import { CHECK_LABELS } from "./cloudMessages";

const SECRET = "K003-very-secret-value";
const KEY_ID = "003a1b2c3d4e5f60000000001";

const PROVIDERS: ProviderInfo[] = [
  {
    id: "backblaze-b2",
    name: "Backblaze B2",
    endpointRequired: false,
    defaultRegion: "eu-central-003",
    knownRegions: [
      { id: "eu-central-003", location: "Amsterdam, NL" },
      { id: "us-west-004", location: "US West" },
    ],
    versioning: "always",
    capabilities: [
      { name: "conditional-put", support: "unknown", evidence: null },
      { name: "multipart-abort", support: "yes", evidence: "verified at the provider" },
      { name: "copy-if-match", support: "assumed", evidence: "Amazon S3 behaviour, not checked here" },
    ],
  },
  {
    id: "hetzner",
    name: "Hetzner Object Storage",
    endpointRequired: false,
    defaultRegion: "fsn1",
    knownRegions: [{ id: "fsn1", location: "Falkenstein, DE" }],
    versioning: "optional",
    capabilities: [],
  },
  {
    id: "s3-compatible",
    name: "S3-compatible service",
    endpointRequired: true,
    defaultRegion: "us-east-1",
    knownRegions: [],
    versioning: "unknown",
    capabilities: [],
  },
];

function account(overrides: Partial<AccountInfo> = {}): AccountInfo {
  return {
    id: "photos",
    label: "Photos",
    provider: "backblaze-b2",
    providerName: "Backblaze B2",
    region: "eu-central-003",
    endpoint: null,
    endpointUrl: "https://s3.eu-central-003.backblazeb2.com",
    bucket: "soty-photos",
    configError: null,
    lastCheck: null,
    ...overrides,
  };
}

interface IpcCall {
  cmd: string;
  payload: unknown;
}

type Handler = (payload: Record<string, unknown>) => unknown;

let calls: IpcCall[];
let accounts: AccountInfo[];
let storageError: string | null;
/** Per-command overrides of the fake backend. */
let overrides: Record<string, Handler>;
let app: ReturnType<typeof mount> | undefined;

function check(status: CheckStatus, message: string | null = null): ConnectionCheck {
  return { status, message, bucket: "soty-photos", checkedAt: Date.UTC(2026, 10, 11, 12, 30) };
}

/** Plays the Rust commands: ids from labels, accounts without credentials. */
function backend(cmd: string, payload: Record<string, unknown>): unknown {
  const override = overrides[cmd];
  if (override) {
    return override(payload);
  }
  switch (cmd) {
    case LIST_CLOUD_PROVIDERS:
      return PROVIDERS;
    case LIST_CLOUD_ACCOUNTS:
      return { accounts, storageError };
    case ADD_CLOUD_ACCOUNT: {
      const input = payload["account"] as { label: string; provider: string; region: string; endpoint: string | null; bucket: string };
      const base = input.label.toLowerCase().replace(/[^a-z0-9]+/g, "-");
      let id = base;
      for (let n = 2; accounts.some((a) => a.id === id); n++) {
        id = `${base}-${n}`;
      }
      const added = account({
        id,
        label: input.label,
        provider: input.provider,
        providerName: PROVIDERS.find((p) => p.id === input.provider)?.name ?? null,
        region: input.region,
        endpoint: input.endpoint,
        bucket: input.bucket,
      });
      accounts = [...accounts, added];
      return added;
    }
    case UPDATE_CLOUD_ACCOUNT: {
      const id = payload["id"] as string;
      const input = payload["account"] as { label: string; region: string; endpoint: string | null; bucket: string };
      const current = accounts.find((a) => a.id === id);
      if (!current) {
        return Promise.reject({ kind: "unknown-account", message: `no account '${id}'` });
      }
      const next = { ...current, label: input.label, region: input.region, endpoint: input.endpoint, bucket: input.bucket, lastCheck: null };
      accounts = accounts.map((a) => (a.id === id ? next : a));
      return next;
    }
    case REMOVE_CLOUD_ACCOUNT:
      accounts = accounts.filter((a) => a.id !== payload["id"]);
      return null;
    case CHECK_CLOUD_ACCOUNT: {
      const current = accounts.find((a) => a.id === payload["id"]);
      return { ...current, lastCheck: check("connected") };
    }
    case "operating_mode":
      return "tray";
    case "list_features":
      return { features: [], storageError: null };
    default:
      return null;
  }
}

beforeEach(() => {
  calls = [];
  accounts = [];
  storageError = null;
  overrides = {};
  mockWindows("main");
  mockIPC((cmd, payload) => {
    calls.push({ cmd, payload });
    return backend(cmd, (payload ?? {}) as Record<string, unknown>);
  });
});

afterEach(() => {
  if (app) {
    unmount(app);
    app = undefined;
  }
  clearMocks();
  document.body.innerHTML = "";
});

async function settle(): Promise<void> {
  for (let i = 0; i < 3; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }
}

async function render(component: typeof CloudStorage | typeof App = CloudStorage): Promise<HTMLElement> {
  const target = document.createElement("div");
  document.body.append(target);
  app = mount(component, { target });
  flushSync();
  await settle();
  return target;
}

/** The element's text with runs of whitespace (source line breaks) as one space. */
function text(el: Node | null | undefined): string {
  return (el?.textContent ?? "").replace(/\s+/g, " ");
}

function buttons(root: ParentNode): HTMLButtonElement[] {
  return [...root.querySelectorAll<HTMLButtonElement>("button")];
}

function button(root: ParentNode, name: string): HTMLButtonElement {
  const found = buttons(root).find((b) => b.textContent?.trim() === name);
  if (!found) {
    throw new Error(`no button "${name}" in: ${buttons(root).map((b) => b.textContent?.trim()).join(", ")}`);
  }
  return found;
}

async function click(root: ParentNode, name: string): Promise<void> {
  button(root, name).click();
  flushSync();
  await settle();
}

function item(root: HTMLElement, id: string): HTMLElement {
  const li = root.querySelector<HTMLElement>(`li[data-account="${id}"]`);
  if (!li) {
    throw new Error(`no account ${id} in: ${root.innerHTML}`);
  }
  return li;
}

/** The form field whose label starts with `name`. */
function field(root: ParentNode, name: string): HTMLInputElement | HTMLSelectElement {
  for (const label of root.querySelectorAll("label")) {
    if (label.textContent?.trim().startsWith(name)) {
      const control = label.htmlFor
        ? document.getElementById(label.htmlFor)
        : label.querySelector<HTMLInputElement | HTMLSelectElement>("input, select");
      if (control instanceof HTMLInputElement || control instanceof HTMLSelectElement) {
        return control;
      }
    }
  }
  throw new Error(`no field "${name}" in: ${(root as HTMLElement).innerHTML}`);
}

function type(root: ParentNode, name: string, value: string): void {
  const control = field(root, name);
  control.value = value;
  control.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function form(root: ParentNode): HTMLFormElement {
  const f = root.querySelector<HTMLFormElement>("form");
  if (!f) {
    throw new Error("no form open");
  }
  return f;
}

async function submit(root: ParentNode): Promise<void> {
  form(root).dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  flushSync();
  await settle();
}

function credentialFields(root: ParentNode): HTMLInputElement[] {
  return [...root.querySelectorAll<HTMLInputElement>("input[data-credential]")];
}

function sent(cmd: string): unknown[] {
  return calls.filter((c) => c.cmd === cmd).map((c) => c.payload);
}

function alerts(root: ParentNode): string[] {
  return [...root.querySelectorAll('[role="alert"]')].map((e) => e.textContent?.trim() ?? "");
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

describe("navigation", () => {
  it("opens on the overview and switches to Cloud Storage and back through links", async () => {
    const root = await render(App);
    expect(root.querySelector("#cloud-storage-heading")).toBeNull();
    expect(sent(LIST_CLOUD_ACCOUNTS)).toHaveLength(0);
    expect(buttons(root).map((b) => b.textContent?.trim())).toEqual(["Hide dashboard"]);
    const links = [...root.querySelectorAll<HTMLAnchorElement>("nav a")];
    expect(links.map((a) => a.textContent?.trim())).toEqual(["Overview", "Cloud Storage"]);
    expect(links[0]?.getAttribute("aria-current")).toBe("page");

    links[1]?.click();
    flushSync();
    await settle();
    expect(root.querySelector("#cloud-storage-heading")?.textContent).toBe("Cloud Storage");
    expect(links[1]?.getAttribute("aria-current")).toBe("page");
    expect(root.querySelector("#features-heading")).toBeNull();
    expect(sent(LIST_CLOUD_ACCOUNTS)).toHaveLength(1);

    links[0]?.click();
    flushSync();
    await settle();
    expect(root.querySelector("#features-heading")).not.toBeNull();
    expect(buttons(root).map((b) => b.textContent?.trim())).toEqual(["Hide dashboard"]);
  });
});

describe("account list", () => {
  it("shows a useful empty state with an Add account action", async () => {
    const root = await render();
    expect(text(root)).toContain("No storage accounts yet.");
    expect(text(root)).toContain("A key restricted to that one bucket is enough.");
    expect(button(root, "Add account").disabled).toBe(false);
    expect(root.querySelectorAll("li[data-account]")).toHaveLength(0);
  });

  it("lists every account with its non-sensitive details, also two at the same provider", async () => {
    accounts = [
      account(),
      account({ id: "work", label: "Work", bucket: "soty-work", lastCheck: check("connected") }),
    ];
    const root = await render();
    expect(text(root)).not.toContain("No storage accounts yet");
    const photos = item(root, "photos");
    const work = item(root, "work");
    expect(photos.querySelector(".label")?.textContent).toBe("Photos");
    expect(photos.querySelector(".provider")?.textContent).toBe("Backblaze B2");
    expect(photos.querySelector(".bucket")?.textContent).toBe("soty-photos");
    expect(text(photos)).toContain("eu-central-003");
    expect(text(photos)).toContain("https://s3.eu-central-003.backblazeb2.com");
    expect(text(photos)).toContain("Not checked yet");
    expect(work.querySelector(".provider")?.textContent).toBe("Backblaze B2");
    expect(work.querySelector(".bucket")?.textContent).toBe("soty-work");
    expect(work.querySelector("[data-status]")?.textContent?.trim()).toBe("Connected");
    // Each account has its own actions.
    for (const li of [photos, work]) {
      expect(buttons(li).map((b) => b.textContent?.trim())).toEqual(["Open bucket", "Check connection", "Edit", "Remove"]);
    }
  });

  it("shows an account whose settings no longer validate", async () => {
    accounts = [account({ provider: "retired", providerName: null, endpointUrl: null, configError: "unknown provider 'retired'" })];
    const root = await render();
    expect(text(item(root, "photos"))).toContain("This account's settings cannot be used: unknown provider 'retired'");
    expect(item(root, "photos").querySelector(".provider")?.textContent).toBe("retired");
  });

  it("reports an account database that cannot be opened, without an add action", async () => {
    storageError = "the database is damaged";
    const root = await render();
    expect(alerts(root).join(" ")).toContain("The account database could not be opened: the database is damaged");
    expect(buttons(root)).toHaveLength(0);
  });

  it("reports a failing list command", async () => {
    overrides[LIST_CLOUD_ACCOUNTS] = () => Promise.reject({ kind: "other", message: "boom" });
    const root = await render();
    expect(alerts(root)).toEqual(["Could not load the storage accounts: Something went wrong: boom"]);
  });
});

describe("adding an account", () => {
  it("starts with the first provider's presets and changes them with the provider", async () => {
    const root = await render();
    await click(root, "Add account");
    expect(field(root, "Provider").value).toBe("backblaze-b2");
    expect(field(root, "Region").value).toBe("eu-central-003");
    const regions = [...root.querySelectorAll("datalist option")].map((o) => (o as HTMLOptionElement).value);
    expect(regions).toEqual(["eu-central-003", "us-west-004"]);
    expect(field(root, "Custom endpoint (optional)").required).toBe(false);

    const select = field(root, "Provider");
    select.value = "s3-compatible";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(field(root, "Region").value).toBe("us-east-1");
    const endpoint = field(root, "Endpoint");
    expect(endpoint.required).toBe(true);
    expect(root.querySelector("datalist")).toBeNull();
    // Secure inputs for both parts of the key.
    expect(credentialFields(root).map((i) => i.type)).toEqual(["password", "password"]);
    expect(credentialFields(root).every((i) => i.autocomplete === "off")).toBe(true);
  });

  it("says what the chosen provider guarantees, with the evidence", async () => {
    const root = await render();
    await click(root, "Add account");
    const details = root.querySelector("details.capabilities");
    expect(details).not.toBeNull();
    expect(text(details!.querySelector("summary"))).toBe("What Backblaze B2 guarantees");
    expect(text(details)).toContain("Versioning: always on");
    const item = (name: string): string => text(details!.querySelector(`[data-capability="${name}"]`));
    expect(item("conditional-put")).toContain("Refuses to overwrite during upload (conditional writes): unknown");
    expect(item("multipart-abort")).toContain("yes (verified at the provider)");
    // An assumption is labelled as one, not as a guarantee.
    expect(item("copy-if-match")).toContain("Copies only an unchanged source: assumed");
    // A provider with nothing listed shows no section.
    const select = field(root, "Provider");
    select.value = "hetzner";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    expect(root.querySelector("details.capabilities")).toBeNull();
  });

  it("sends the form to Rust, lists the new account and keeps no credentials", async () => {
    const root = await render();
    await click(root, "Add account");
    type(root, "Name", "Photos");
    type(root, "Bucket", "soty-photos");
    type(root, "Access key ID", KEY_ID);
    type(root, "Secret access key", SECRET);
    await submit(root);

    expect(sent(ADD_CLOUD_ACCOUNT)).toEqual([
      {
        account: { label: "Photos", provider: "backblaze-b2", region: "eu-central-003", endpoint: null, bucket: "soty-photos" },
        credentials: { accessKeyId: KEY_ID, secretAccessKey: SECRET },
      },
    ]);
    expect(root.querySelector("form")).toBeNull();
    expect(item(root, "photos").querySelector(".bucket")?.textContent).toBe("soty-photos");
    expect(text(root)).toContain("Added Photos. Check its connection");
    expect(document.body.innerHTML).not.toContain(SECRET);
    expect(document.body.innerHTML).not.toContain(KEY_ID);
    expect(localStorage.length + sessionStorage.length).toBe(0);

    // Opening the form again starts with empty credential fields.
    await click(root, "Add account");
    expect(credentialFields(root).map((i) => i.value)).toEqual(["", ""]);
  });

  it("adds a second account at the same provider as its own entry", async () => {
    accounts = [account()];
    const root = await render();
    await click(root, "Add account");
    type(root, "Name", "Photos");
    type(root, "Bucket", "soty-photos-archive");
    type(root, "Access key ID", "k2");
    type(root, "Secret access key", "s2");
    await submit(root);
    expect(item(root, "photos").querySelector(".bucket")?.textContent).toBe("soty-photos");
    expect(item(root, "photos-2").querySelector(".bucket")?.textContent).toBe("soty-photos-archive");
  });

  it("keeps the form open with the error when the credential store is unavailable", async () => {
    overrides[ADD_CLOUD_ACCOUNT] = () =>
      Promise.reject({ kind: "credential-store-unavailable", message: "the Secret Service (desktop keyring) is not available" });
    const root = await render();
    await click(root, "Add account");
    type(root, "Name", "Photos");
    type(root, "Bucket", "soty-photos");
    type(root, "Access key ID", KEY_ID);
    type(root, "Secret access key", SECRET);
    await submit(root);
    const shown = alerts(root).join(" ");
    expect(shown).toContain("Your system's credential store is not available, so the key was not stored.");
    expect(shown).toContain("sotyware never stores keys anywhere else");
    expect(shown).not.toContain(SECRET);
    expect(root.querySelectorAll("li[data-account]")).toHaveLength(0);
    // Still open for a retry; cancelling clears the key.
    expect(field(root, "Name").value).toBe("Photos");
    await click(root, "Cancel");
    expect(root.querySelector("form")).toBeNull();
    await click(root, "Add account");
    expect(credentialFields(root).map((i) => i.value)).toEqual(["", ""]);
    expect(field(root, "Name").value).toBe("");
  });

  it("shows invalid input from Rust and a locked keyring as their own messages", async () => {
    const root = await render();
    await click(root, "Add account");
    for (const [kind, message, expected] of [
      ["invalid-input", "region 'EU' must be lowercase letters, digits and '-'", "Check the form: region 'EU'"],
      ["credential-store-locked", "the Secret Service refused access: prompt dismissed", "Unlock it (or accept its prompt) and try again"],
    ] as const) {
      overrides[ADD_CLOUD_ACCOUNT] = () => Promise.reject({ kind, message });
      type(root, "Name", "Photos");
      type(root, "Bucket", "soty-photos");
      type(root, "Access key ID", "k");
      type(root, "Secret access key", "s");
      await submit(root);
      expect(alerts(root).join(" ")).toContain(expected);
    }
  });

  it("disables the form while saving", async () => {
    const pending = deferred<unknown>();
    overrides[ADD_CLOUD_ACCOUNT] = () => pending.promise;
    const root = await render();
    await click(root, "Add account");
    type(root, "Name", "Photos");
    type(root, "Bucket", "soty-photos");
    type(root, "Access key ID", "k");
    type(root, "Secret access key", "s");
    await submit(root);
    expect(root.querySelector("fieldset")?.disabled).toBe(true);
    expect(button(root, "Saving…")).toBeDefined();
    pending.resolve(account());
    await settle();
    expect(root.querySelector("form")).toBeNull();
  });
});

describe("editing an account", () => {
  it("changes the name without asking for the key again", async () => {
    accounts = [account()];
    const root = await render();
    await click(item(root, "photos"), "Edit");
    expect(field(root, "Provider").value).toBe("Backblaze B2");
    expect((field(root, "Provider") as HTMLInputElement).readOnly).toBe(true);
    expect(field(root, "Name").value).toBe("Photos");
    expect(field(root, "Bucket").value).toBe("soty-photos");
    expect(credentialFields(root)).toHaveLength(0);
    type(root, "Name", "Family photos");
    await submit(root);
    expect(sent(UPDATE_CLOUD_ACCOUNT)).toEqual([
      {
        id: "photos",
        account: { label: "Family photos", provider: "backblaze-b2", region: "eu-central-003", endpoint: null, bucket: "soty-photos" },
        credentials: null,
      },
    ]);
    expect(item(root, "photos").querySelector(".label")?.textContent).toBe("Family photos");
    expect(text(root)).toContain("Saved Family photos.");
  });

  it("replaces the key when asked and clears the fields afterwards", async () => {
    accounts = [account()];
    const root = await render();
    await click(item(root, "photos"), "Edit");
    const replace = field(root, "Replace access key") as HTMLInputElement;
    replace.click();
    flushSync();
    expect(text(root)).toContain("The current key stays in use until the new one is stored.");
    type(root, "Access key ID", KEY_ID);
    type(root, "Secret access key", SECRET);
    await submit(root);
    expect(sent(UPDATE_CLOUD_ACCOUNT)).toEqual([
      {
        id: "photos",
        account: { label: "Photos", provider: "backblaze-b2", region: "eu-central-003", endpoint: null, bucket: "soty-photos" },
        credentials: { accessKeyId: KEY_ID, secretAccessKey: SECRET },
      },
    ]);
    expect(document.body.innerHTML).not.toContain(SECRET);
    await click(item(root, "photos"), "Edit");
    (field(root, "Replace access key") as HTMLInputElement).click();
    flushSync();
    expect(credentialFields(root).map((i) => i.value)).toEqual(["", ""]);
  });

  it("clears typed credentials when Replace access key is unticked", async () => {
    accounts = [account()];
    const root = await render();
    await click(item(root, "photos"), "Edit");
    const replace = field(root, "Replace access key") as HTMLInputElement;
    replace.click();
    flushSync();
    type(root, "Secret access key", SECRET);
    replace.click();
    flushSync();
    replace.click();
    flushSync();
    expect(credentialFields(root).map((i) => i.value)).toEqual(["", ""]);
  });

  it("keeps the old key when storing the new one fails", async () => {
    accounts = [account()];
    overrides[UPDATE_CLOUD_ACCOUNT] = () =>
      Promise.reject({ kind: "credential-store-locked", message: "Windows Credential Manager refused access" });
    const root = await render();
    await click(item(root, "photos"), "Edit");
    (field(root, "Replace access key") as HTMLInputElement).click();
    flushSync();
    type(root, "Access key ID", "k");
    type(root, "Secret access key", "s");
    await submit(root);
    expect(alerts(root).join(" ")).toContain("The credential store refused access.");
    expect(root.querySelector("form")).not.toBeNull();
  });

  it("reloads after a partial save and drops the stored key from the form", async () => {
    accounts = [account()];
    overrides[UPDATE_CLOUD_ACCOUNT] = () =>
      Promise.reject({ kind: "partial", message: "the new access key was saved, but the other changes were not: disk full" });
    const root = await render();
    await click(item(root, "photos"), "Edit");
    (field(root, "Replace access key") as HTMLInputElement).click();
    flushSync();
    type(root, "Name", "Renamed");
    type(root, "Access key ID", "k");
    type(root, "Secret access key", "s");
    const listsBefore = sent(LIST_CLOUD_ACCOUNTS).length;
    await submit(root);
    expect(alerts(root).join(" ")).toContain("Only part of the change was made: the new access key was saved");
    expect(credentialFields(root)).toHaveLength(0);
    expect((field(root, "Replace access key") as HTMLInputElement).checked).toBe(false);
    expect(sent(LIST_CLOUD_ACCOUNTS).length).toBe(listsBefore + 1);
  });
});

describe("removing an account", () => {
  it("asks first, explains that remote data stays, and removes on confirmation", async () => {
    accounts = [account(), account({ id: "work", label: "Work", bucket: "soty-work" })];
    const root = await render();
    await click(item(root, "photos"), "Remove");
    const confirm = item(root, "photos").querySelector('[role="group"]');
    expect(text(confirm)).toContain("Remove Photos from sotyware?");
    expect(text(confirm)).toContain("The bucket soty-photos and its files at Backblaze B2 are not deleted or changed.");
    expect(sent(REMOVE_CLOUD_ACCOUNT)).toHaveLength(0);

    await click(item(root, "photos"), "Cancel");
    expect(item(root, "photos").querySelector('[role="group"]')).toBeNull();
    expect(sent(REMOVE_CLOUD_ACCOUNT)).toHaveLength(0);

    await click(item(root, "photos"), "Remove");
    await click(item(root, "photos"), "Remove account");
    expect(sent(REMOVE_CLOUD_ACCOUNT)).toEqual([{ id: "photos" }]);
    expect(root.querySelector('li[data-account="photos"]')).toBeNull();
    expect(item(root, "work")).toBeDefined();
    expect(text(root)).toContain("Its bucket and files at the provider were not changed.");
  });

  it("keeps the account and reports the error when its key cannot be deleted", async () => {
    accounts = [account()];
    overrides[REMOVE_CLOUD_ACCOUNT] = () =>
      Promise.reject({ kind: "credential-store-unavailable", message: "the Secret Service (desktop keyring) is not available" });
    const root = await render();
    await click(item(root, "photos"), "Remove");
    await click(item(root, "photos"), "Remove account");
    expect(item(root, "photos")).toBeDefined();
    const shown = alerts(root).join(" ");
    expect(shown).toContain("Could not remove Photos");
    expect(shown).toContain("The account is still listed.");
    expect(text(root)).not.toContain("Removed Photos");
  });
});

describe("connection check", () => {
  it("shows progress, disables the button, then shows the result", async () => {
    accounts = [account(), account({ id: "work", label: "Work" })];
    const pending = deferred<unknown>();
    overrides[CHECK_CLOUD_ACCOUNT] = () => pending.promise;
    const root = await render();
    await click(item(root, "photos"), "Check connection");
    expect(sent(CHECK_CLOUD_ACCOUNT)).toEqual([{ id: "photos" }]);
    expect(item(root, "photos").querySelector('[role="status"]')?.textContent).toBe("Checking connection…");
    expect(button(item(root, "photos"), "Check connection").disabled).toBe(true);
    // The other account is not affected.
    expect(button(item(root, "work"), "Check connection").disabled).toBe(false);
    expect(text(item(root, "work"))).toContain("Not checked yet");

    pending.resolve({ ...account(), lastCheck: check("connected") });
    await settle();
    expect(item(root, "photos").querySelector("[data-status]")?.textContent?.trim()).toBe("Connected");
    expect(button(item(root, "photos"), "Check connection").disabled).toBe(false);
    expect(text(item(root, "work"))).toContain("Not checked yet");
  });

  it("shows every result with its own label and, for failures, what to do", async () => {
    for (const status of CHECK_STATUSES) {
      accounts = [account()];
      overrides[CHECK_CLOUD_ACCOUNT] = () => ({ ...account(), lastCheck: check(status, status === "connected" ? null : `detail of ${status}`) });
      const root = await render();
      await click(item(root, "photos"), "Check connection");
      const li = item(root, "photos");
      expect(li.querySelector("[data-status]")?.textContent?.trim()).toBe(CHECK_LABELS[status]);
      if (status === "connected") {
        expect(li.querySelector(".check-detail")).toBeNull();
      } else {
        expect(text(li.querySelector(".check-detail"))).toContain(`Details: detail of ${status}`);
      }
      unmount(app!);
      app = undefined;
      document.body.innerHTML = "";
    }
  });

  it("tells access denied apart from invalid credentials", async () => {
    accounts = [account()];
    overrides[CHECK_CLOUD_ACCOUNT] = () => ({ ...account(), lastCheck: check("access-denied", "access denied: check bucket: not entitled") });
    const root = await render();
    await click(item(root, "photos"), "Check connection");
    const shown = text(item(root, "photos"));
    expect(shown).toContain("Access denied");
    expect(shown).toContain("The key is valid, but it is not allowed to use this bucket.");
    expect(shown).not.toContain("Invalid credentials");
  });

  it("reports a check that could not run and reloads a vanished account", async () => {
    accounts = [account()];
    overrides[CHECK_CLOUD_ACCOUNT] = () => Promise.reject({ kind: "unknown-account", message: "no account 'photos'" });
    const root = await render();
    accounts = [];
    await click(item(root, "photos"), "Check connection");
    expect(alerts(root).join(" ")).toContain("Could not check Photos: This account no longer exists: no account 'photos'");
    expect(root.querySelector('li[data-account="photos"]')).toBeNull();
  });
});

describe("cloud command wrappers", () => {
  it("parse the Rust shapes and refuse anything else", () => {
    expect(parseProvider(PROVIDERS[0])).toEqual(PROVIDERS[0]);
    expect(parseAccount(account({ lastCheck: check("network", "timed out") }))).toEqual(
      account({ lastCheck: check("network", "timed out") }),
    );
    expect(parseAccountList({ accounts: [account()], storageError: null }).accounts).toHaveLength(1);
    expect(() => parseAccount({ ...account(), bucket: 3 })).toThrow("invalid account");
    expect(() => parseAccount({ ...account(), lastCheck: { ...check("connected"), status: "fine" } })).toThrow(
      "invalid connection check",
    );
    expect(() => parseProvider({ ...PROVIDERS[0], knownRegions: [{ id: 1 }] })).toThrow("invalid region");
    expect(() => parseProvider({ ...PROVIDERS[0], capabilities: [{ name: "x", support: "maybe", evidence: null }] })).toThrow(
      "invalid capability",
    );
    expect(() => parseProvider({ ...PROVIDERS[0], capabilities: undefined })).toThrow("invalid provider");
  });

  it("turn structured rejections into typed errors and nothing else", () => {
    const e = toCommandError({ kind: "partial", message: "half done" });
    expect(e).toBeInstanceOf(CloudCommandError);
    expect((e as CloudCommandError).kind).toBe("partial");
    expect(toCommandError({ kind: "made-up", message: "x" })).not.toBeInstanceOf(CloudCommandError);
    expect(toCommandError("Command add_cloud_account not allowed").message).toBe("Command add_cloud_account not allowed");
  });
});
