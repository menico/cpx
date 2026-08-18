// The typed surface of the Rust side. Every shape here mirrors a struct in
// crates/cpx-app/src/view.rs; nothing is re-derived on this side.
import { invoke } from "@tauri-apps/api/core";
import { fixtures } from "./fixtures";

export type CredentialSource = "keychain" | "file" | "none";

export interface ProfileRow {
  name: string;
  /** Managed in place, in a directory that existed before cpx did. */
  adopted: boolean;
  description: string;
  color: string | null;
  command: string;
  model: string | null;
  directory: string;
  applied: boolean;
  signedIn: boolean;
  account: string | null;
  credentialSource: CredentialSource;
}

export type ResourceMode = "link" | "copy" | "own" | "merge";

export interface ResourceRow {
  resource: string;
  mode: ResourceMode;
  isDirectory: boolean;
  supportsMerge: boolean;
  hasPatch: boolean;
}

export interface ProfileDetail {
  row: ProfileRow;
  addDirs: string[];
  env: [string, string][];
  resources: ResourceRow[];
  keychainService: string;
}

export type Risk = "safe" | "generated" | "foreign";

export interface PlanLine {
  risk: Risk;
  verb: string;
  target: string;
  description: string;
}

export interface PlanView {
  lines: PlanLine[];
  notes: string[];
  requiresForce: boolean;
}

export type BindingHealth =
  | "healthy"
  | "directoryMissing"
  | "profileMissing"
  | "blockAbsent"
  | "blockEdited"
  | "notAllowed";

export interface BindingRow {
  path: string;
  profile: string;
  color: string | null;
  health: BindingHealth;
  healthy: boolean;
}

export type Severity = "ok" | "warning" | "error";

export interface CheckView {
  name: string;
  severity: Severity;
  detail: string;
  remedy: string | null;
}

export interface AdoptionRow {
  name: string;
  dir: string;
  /** What the directory already holds, and keeps untouched. */
  keeps: string[];
  /** A profile of this name is already configured. */
  taken: boolean;
}

export interface ApplyView {
  performed: number;
  backups: [string, string][];
}

/** Whether the Tauri bridge is present. Absent when the UI is opened in a
 *  plain browser, which is how the interface is developed and reviewed. */
const inDesktopApp = "__TAURI_INTERNALS__" in window;

const real = {
  isInitialised: () => invoke<boolean>("is_initialised"),
  initialise: (profiles: string[]) => invoke<void>("initialise", { profiles }),
  profiles: () => invoke<ProfileRow[]>("profiles"),
  profile: (name: string) => invoke<ProfileDetail>("profile", { name }),
  plan: () => invoke<PlanView>("plan"),
  apply: (force: boolean, sync: boolean) => invoke<ApplyView>("apply", { force, sync }),
  bindings: () => invoke<BindingRow[]>("bindings"),
  bind: (profile: string, dir: string) => invoke<void>("bind", { profile, dir }),
  unbind: (dir: string) => invoke<void>("unbind", { dir }),
  checks: () => invoke<CheckView[]>("checks"),
  addProfile: (name: string, description: string) =>
    invoke<void>("add_profile", { name, description }),
  removeProfile: (name: string) => invoke<void>("remove_profile", { name }),
  cloneProfile: (from: string, to: string) => invoke<void>("clone_profile", { from, to }),
  setField: (profile: string, field: string, value: string | null) =>
    invoke<void>("set_field", { profile, field, value }),
  setResource: (profile: string, resource: string, mode: ResourceMode) =>
    invoke<void>("set_resource", { profile, resource, mode }),
  adoptionCandidates: () => invoke<AdoptionRow[]>("adoption_candidates"),
  adopt: (dir: string, name: string | null) => invoke<void>("adopt", { dir, name }),
  configPath: () => invoke<string>("config_path"),
  auth: (profile: string, action: "login" | "logout" | "status") =>
    invoke<void>("auth", { profile, action }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  pickDirectory: () => invoke<string | null>("pick_directory"),
  hideWindow: () => invoke<void>("hide_window"),
};

export const api = inDesktopApp ? real : (fixtures as unknown as typeof real);

/// Tauri rejects with a string; keep that as the message the UI shows.
export function message(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}
