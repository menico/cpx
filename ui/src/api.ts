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

export interface Statusline {
  /** A cpx badge is installed for this target. */
  badge: boolean;
  /** What the badge sits in front of, or what is configured now. */
  delegate: string | null;
  script: string;
  /** The change is recorded in config.toml and takes effect on the next apply. */
  needsApply: boolean;
}

export interface Skill {
  name: string;
  description: string | null;
  enabled: boolean;
  path: string;
}

export interface PluginSkills {
  /** The `plugin@marketplace` key used in settings. */
  key: string;
  plugin: string;
  marketplace: string;
  enabled: boolean;
  skills: number;
  names: string[];
}

export interface SkillInventory {
  /** Skills the profile owns, enabled and disabled. */
  own: Skill[];
  plugins: PluginSkills[];
  /** The skills directory is shared, so turning one off affects every profile. */
  shared: boolean;
}

export interface StatuslineScript {
  path: string;
  contents: string;
  /** cpx's own copy, so edits are safe from installers. */
  owned: boolean;
  /** Set when something else will overwrite this file later. */
  managedBy: string | null;
}

export interface DefaultSession {
  dir: string;
  account: string | null;
  signedIn: boolean;
  /** Also the directory profiles inherit from. */
  isSource: boolean;
  /** Set when a profile already manages it, in which case it is listed above. */
  claimedBy: string | null;
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
  defaultSession: () => invoke<DefaultSession>("default_session"),
  statusline: (profile: string | null) => invoke<Statusline>("statusline", { profile }),
  setStatusline: (profile: string | null, label: string | null, refresh: number | null) =>
    invoke<void>("set_statusline", { profile, label, refresh }),
  clearStatusline: (profile: string | null) => invoke<void>("clear_statusline", { profile }),
  skills: (profile: string) => invoke<SkillInventory>("skills", { profile }),
  setSkillEnabled: (profile: string, skill: string, enabled: boolean) =>
    invoke<void>("set_skill_enabled", { profile, skill, enabled }),
  removeSkill: (profile: string, skill: string) =>
    invoke<string>("remove_skill", { profile, skill }),
  setPluginEnabled: (profile: string, key: string, enabled: boolean) =>
    invoke<boolean>("set_plugin_enabled", { profile, key, enabled }),
  statuslineScript: (profile: string | null) =>
    invoke<StatuslineScript | null>("statusline_script", { profile }),
  saveStatuslineScript: (profile: string | null, contents: string) =>
    invoke<void>("save_statusline_script", { profile, contents }),
  forkStatuslineScript: (profile: string | null) =>
    invoke<string>("fork_statusline_script", { profile }),
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
