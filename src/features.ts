// Typed access to the built-in features (src-tauri/src/features/). The page
// reaches them only through these two commands; it never sees Rust state
// directly. Responses are checked, so a Rust/TypeScript mismatch fails loudly.
// Saving the user's choices happens on the Rust side; the page only learns
// whether it worked (`storageError`, `saveError`).
import { invoke } from "@tauri-apps/api/core";

export const LIST_FEATURES = "list_features";
export const SET_FEATURE_ENABLED = "set_feature_enabled";

// `FeatureState` in src-tauri/src/features/mod.rs, serialized in kebab-case.
export const FEATURE_STATES = [
  "registered",
  "disabled",
  "initializing",
  "running",
  "failed",
  "stopping",
] as const;

export type FeatureState = (typeof FEATURE_STATES)[number];

// `FeatureInfo` in src-tauri/src/features/mod.rs.
export interface FeatureInfo {
  id: string;
  name: string;
  description: string;
  version: string;
  state: FeatureState;
  /** The user's intent: true while initializing, running or failed. */
  enabled: boolean;
  /** The last start or stop error, if any. */
  error: string | null;
}

function isFeatureState(value: unknown): value is FeatureState {
  return (FEATURE_STATES as readonly unknown[]).includes(value);
}

export function parseFeature(value: unknown): FeatureInfo {
  const invalid = (): Error => new Error(`invalid feature: ${JSON.stringify(value)}`);
  if (typeof value !== "object" || value === null) {
    throw invalid();
  }
  const v = value as Record<string, unknown>;
  const { id, name, description, version, state, enabled, error } = v;
  if (
    typeof id !== "string" ||
    typeof name !== "string" ||
    typeof description !== "string" ||
    typeof version !== "string" ||
    !isFeatureState(state) ||
    typeof enabled !== "boolean" ||
    (error !== null && typeof error !== "string")
  ) {
    throw invalid();
  }
  return { id, name, description, version, state, enabled, error };
}

// `FeatureList` in src-tauri/src/features/mod.rs.
export interface FeatureList {
  features: FeatureInfo[];
  /** Why the saved choices could not be loaded: then every feature uses its
   * default and no change is saved. `null` when they were loaded. */
  storageError: string | null;
}

// `FeatureChange` in src-tauri/src/features/mod.rs.
export interface FeatureChange {
  /** The feature's new state; the switch happened even if saving failed. */
  feature: FeatureInfo;
  /** Why the choice could not be saved: it then lasts only until sotyware quits. */
  saveError: string | null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

export function parseFeatureList(value: unknown): FeatureList {
  if (!isRecord(value) || !Array.isArray(value["features"]) || !isNullableString(value["storageError"])) {
    throw new Error(`invalid feature list: ${JSON.stringify(value)}`);
  }
  return { features: value["features"].map(parseFeature), storageError: value["storageError"] };
}

export function parseFeatureChange(value: unknown): FeatureChange {
  if (!isRecord(value) || !isNullableString(value["saveError"])) {
    throw new Error(`invalid feature change: ${JSON.stringify(value)}`);
  }
  return { feature: parseFeature(value["feature"]), saveError: value["saveError"] };
}

/** Every built-in feature with its current state, in registration order, and
 * whether the saved choices could be loaded. */
export async function listFeatures(): Promise<FeatureList> {
  return parseFeatureList(await invoke<unknown>(LIST_FEATURES));
}

/**
 * Starts or stops one feature, saves the choice, and returns the feature's new
 * state. A feature that fails to start resolves with state "failed"; a choice
 * that could not be saved resolves with `saveError` set. The promise rejects
 * only for an unknown id or a transition its current state does not allow,
 * and then nothing changed.
 */
export async function setFeatureEnabled(id: string, enabled: boolean): Promise<FeatureChange> {
  return parseFeatureChange(await invoke<unknown>(SET_FEATURE_ENABLED, { id, enabled }));
}
