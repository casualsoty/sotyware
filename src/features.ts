// Typed access to the built-in features (src-tauri/src/features/). The page
// reaches them only through these two commands; it never sees Rust state
// directly. Responses are checked, so a Rust/TypeScript mismatch fails loudly.
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

/** Every built-in feature with its current state, in registration order. */
export async function listFeatures(): Promise<FeatureInfo[]> {
  const value = await invoke<unknown>(LIST_FEATURES);
  if (!Array.isArray(value)) {
    throw new Error(`invalid feature list: ${JSON.stringify(value)}`);
  }
  return value.map(parseFeature);
}

/**
 * Starts or stops one feature and returns its new state. A feature that fails
 * to start resolves with state "failed"; the promise rejects only for an
 * unknown id or a transition its current state does not allow.
 */
export async function setFeatureEnabled(id: string, enabled: boolean): Promise<FeatureInfo> {
  return parseFeature(await invoke<unknown>(SET_FEATURE_ENABLED, { id, enabled }));
}
