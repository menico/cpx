// Development fallback.
//
// Opened in a plain browser there is no Tauri bridge, so the app would fail on
// its first call. These fixtures let the interface be built and reviewed
// without launching the desktop shell. They are used only when the bridge is
// absent, so the packaged app never reaches them.
import type {
  AdoptionRow,
  DefaultSession,
  Statusline,
  ApplyView,
  BindingRow,
  CheckView,
  PlanView,
  ProfileDetail,
  ProfileRow,
  ResourceMode,
} from "./api";

const RESOURCES: { resource: string; mode: ResourceMode; dir: boolean; json: boolean }[] = [
  { resource: "settings", mode: "merge", dir: false, json: true },
  { resource: "settings_local", mode: "merge", dir: false, json: true },
  { resource: "CLAUDE.md", mode: "copy", dir: false, json: false },
  { resource: "commands", mode: "link", dir: true, json: false },
  { resource: "skills", mode: "link", dir: true, json: false },
  { resource: "agents", mode: "link", dir: true, json: false },
  { resource: "plugins", mode: "link", dir: true, json: false },
  { resource: "hooks", mode: "link", dir: true, json: false },
  { resource: "projects", mode: "own", dir: true, json: false },
];

const STATUSLINE: Statusline = {
  badge: false,
  delegate: "node ~/.claude/statusline.mjs",
  script: "/Users/you/.claude-profiles/work/statusline.sh",
  needsApply: true,
};

const DEFAULT_SESSION: DefaultSession = {
  dir: "/Users/you/.claude",
  account: "you@company.com",
  signedIn: true,
  isSource: true,
  claimedBy: null,
};

const ADOPTABLE: AdoptionRow[] = [
  {
    // Not among the profiles below, so it is offered.
    name: "archive",
    dir: "/Users/you/.claude-archive",
    keeps: ["plugins", "projects", "settings.json"],
    taken: false,
  },
  {
    // Already a profile, so the interface hides it.
    name: "personal",
    dir: "/Users/you/.claude-personal",
    keeps: ["plugins", "projects", "settings.json"],
    taken: true,
  },
];

const PROFILES: ProfileRow[] = [
  {
    name: "work",
    adopted: true,
    description: "Company work account",
    color: "#5c8dff",
    command: "claude-work",
    model: "sonnet",
    directory: "/Users/you/.claude-profiles/work",
    applied: true,
    signedIn: true,
    account: "you@company.com",
    credentialSource: "keychain",
  },
  {
    name: "personal",
    adopted: false,
    description: "Personal subscription",
    color: "#5dc794",
    command: "claude-personal",
    model: null,
    directory: "/Users/you/.claude-profiles/personal",
    applied: true,
    signedIn: true,
    account: "me@example.com",
    credentialSource: "keychain",
  },
  {
    name: "client",
    adopted: false,
    description: "Client work",
    color: "#d69552",
    command: "claude-client",
    model: "opus",
    directory: "/Users/you/.claude-profiles/client",
    applied: true,
    signedIn: false,
    account: null,
    credentialSource: "none",
  },
  {
    name: "vertex",
    adopted: false,
    description: "Company via Vertex AI",
    color: "#c96ec9",
    command: "claude-vertex",
    model: null,
    directory: "/Users/you/.claude-profiles/vertex",
    applied: false,
    signedIn: false,
    account: null,
    credentialSource: "none",
  },
];

const PLAN: PlanView = {
  lines: [
    { risk: "safe", verb: "create", target: "/Users/you/.claude-profiles/vertex", description: "" },
    { risk: "safe", verb: "create", target: "/Users/you/.claude-profiles/vertex/bin", description: "" },
    { risk: "safe", verb: "write", target: "/Users/you/.claude-profiles/vertex/settings.json", description: "" },
    { risk: "safe", verb: "link", target: "/Users/you/.claude-profiles/vertex/commands", description: "" },
    { risk: "generated", verb: "write", target: "/Users/you/.claude-profiles/hd/settings.json", description: "" },
    { risk: "foreign", verb: "write", target: "/Users/you/.local/bin/claude-vertex", description: "" },
  ],
  notes: ["vertex: skipping `agents` — /Users/you/.claude/agents does not exist"],
  requiresForce: true,
};

const BINDINGS: BindingRow[] = [
  { path: "/Users/you/Work/company/platform", profile: "work", color: "#5c8dff", health: "healthy", healthy: true },
  { path: "/Users/you/Work/client/api", profile: "client", color: "#d69552", health: "notAllowed", healthy: false },
  { path: "/Users/you/side/notes", profile: "personal", color: "#5dc794", health: "blockEdited", healthy: false },
];

const CHECKS: CheckView[] = [
  {
    name: "CLAUDE_CONFIG_DIR",
    severity: "warning",
    detail:
      "CLAUDE_CONFIG_DIR is set to /Users/you/.claude-hd in this shell, which overrides whichever profile you think you are using",
    remedy: "unset CLAUDE_CONFIG_DIR, or leave the directory whose .envrc sets it",
  },
  {
    name: "profile ol login",
    severity: "warning",
    detail: "ol has no credentials yet",
    remedy: "run `claude-ol auth login`",
  },
  {
    name: "foreign wrapper claude-company",
    severity: "warning",
    detail: "/Users/you/.local/bin/claude-company was not generated by cpx and will never be touched",
    remedy: "nothing to do, unless you want a cpx profile named `company` — that name is taken",
  },
  { name: "source directory", severity: "ok", detail: "/Users/you/.claude", remedy: null },
  { name: "direnv", severity: "ok", detail: "installed", remedy: null },
];

function detail(name: string): ProfileDetail {
  const row = PROFILES.find((p) => p.name === name) ?? PROFILES[0];
  return {
    row,
    addDirs: name === "work" ? ["/Users/you/Work/company"] : [],
    env: name === "vertex" ? [["CLAUDE_CODE_USE_VERTEX", "1"], ["CLOUD_ML_REGION", "europe-west1"]] : [],
    resources: RESOURCES.map((r) => ({
      resource: r.resource,
      mode: r.mode,
      isDirectory: r.dir,
      supportsMerge: r.json,
      hasPatch: r.resource === "settings" && name === "work",
    })),
    keychainService: "Claude Code-credentials-a81467e4",
  };
}

const unsupported = () => Promise.reject("Not available outside the desktop app");

export const fixtures = {
  isInitialised: () => Promise.resolve(true),
  initialise: unsupported,
  profiles: () => Promise.resolve(PROFILES),
  profile: (name: string) => Promise.resolve(detail(name)),
  plan: () => Promise.resolve(PLAN),
  apply: (): Promise<ApplyView> => Promise.resolve({ performed: PLAN.lines.length, backups: [] }),
  bindings: () => Promise.resolve(BINDINGS),
  bind: unsupported,
  unbind: unsupported,
  checks: () => Promise.resolve(CHECKS),
  addProfile: unsupported,
  removeProfile: unsupported,
  cloneProfile: unsupported,
  setField: () => Promise.resolve(),
  setResource: () => Promise.resolve(),
  defaultSession: () => Promise.resolve(DEFAULT_SESSION),
  statusline: () => Promise.resolve(STATUSLINE),
  setStatusline: () => Promise.resolve(),
  clearStatusline: () => Promise.resolve(),
  adoptionCandidates: () => Promise.resolve(ADOPTABLE),
  adopt: unsupported,
  configPath: () => Promise.resolve("/Users/you/.claude-profiles/config.toml"),
  auth: unsupported,
  reveal: () => Promise.resolve(),
  pickDirectory: () => Promise.resolve("/Users/you/Work/new-project"),
  hideWindow: () => Promise.resolve(),
};
