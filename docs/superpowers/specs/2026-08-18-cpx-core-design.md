# cpx — Claude Profile Manager: Core + CLI (Phase 1)

**Date:** 2026-08-18
**Status:** Approved design, ready for implementation planning
**Scope:** Phase 1 of 4 — the Rust core library and its CLI. No GUI.

## Problem

Running several Claude Code accounts on one machine means re-logging in,
mixing credentials, or losing context on every switch. The existing tool in
this space, `JakubKontra/claude-profile-manager` (`cpm`, Go), solves the
basic isolation problem: each profile gets its own `CLAUDE_CONFIG_DIR`, a
`claude-<name>` wrapper script, and symlinks back to `~/.claude` for shared
resources.

Four things are missing or thin in `cpm`:

1. **No GUI.** Everything is CLI, so the state of the system — which profile
   is logged into which account, which directories are bound — is only
   visible by running commands.
2. **`.envrc` support is a stub.** `cpm direnv` prints four lines to stdout
   and leaves the rest to the user. There is no registry of bound
   directories, no idempotent write, no `direnv allow`.
3. **Per-profile control is coarse.** What is shared versus copied is
   hardcoded. A profile can override `model`, `env`, and `add_dirs`, and
   nothing else — in particular there is no way to give a profile its own
   statusline or its own MCP servers.
4. **Mutations are opaque.** Operations write to the filesystem as they go.
   There is no way to see what a command will do before it does it, which
   matters for a tool that manipulates the Keychain, credentials, and
   symlinks under `~/.claude`.

## Goals

Phase 1 delivers a Rust library, `cpx-core`, that owns all profile logic, and
a CLI, `cpx`, that is a thin renderer over it. The library is designed so
that the Phase 2 Tauri application is a second consumer of the same API with
no logic of its own.

Non-goals for Phase 1: the desktop application, the statusline builder,
per-profile MCP management, git-backed settings sync, usage statistics,
migration from `cpm`, and Windows support. Each is a later phase.

## Phase plan

| Phase | Deliverable |
|---|---|
| 1 (this spec) | `cpx-core` + `cpx` CLI: profile model, materialization, credentials, wrappers, directory binding |
| 2 | Tauri menu-bar app + React UI over the same core |
| 3 | Statusline builder and per-profile MCP server management |
| 4 | Git-backed settings sync and per-profile usage/cost statistics |

## Repository layout

```
claude-profiles/
├── Cargo.toml              # workspace
├── crates/
│   ├── cpx-core/           # all logic; no stdout, no CLI concerns
│   └── cpx-cli/            # argument parsing and rendering only
└── docs/superpowers/specs/
```

Phase 2 adds `crates/cpx-app/` and `ui/`. Nothing in `cpx-core` may assume a
terminal exists.

## Configuration

`~/.claude-profiles/config.toml`. The format is a superset of `cpm`'s, so an
existing `cpm` config parses without modification.

```toml
version        = 1
source_dir     = "~/.claude"
bin_dir        = "~/.local/bin"
wrapper_prefix = "claude-"

[defaults.resources]
settings       = "merge"
settings_local = "merge"
"CLAUDE.md"    = "copy"
commands    = "link"
skills      = "link"
agents      = "link"
plugins     = "link"
hooks       = "link"
projects    = "own"

[profiles.work]
description = "Company account"
model       = "sonnet"
add_dirs    = ["~/Work/company"]
env         = { ANTHROPIC_LOG = "debug" }

[profiles.work.resources]
projects = "link"
settings = { mode = "merge", patch = { statusLine = { type = "command", command = "..." } } }
```

A profile's effective resource map is `defaults.resources` overlaid with
`profiles.<name>.resources`. A resource value is either a bare mode string or
a table with a `mode` key plus mode-specific fields.

Resource keys are a fixed, closed set. Unknown keys are a configuration
error rather than being ignored, so a typo is caught at parse time.

| Key | Target inside the profile directory |
|---|---|
| `settings` | `settings.json` |
| `settings_local` | `settings.local.json` |
| `CLAUDE.md` | `CLAUDE.md` |
| `commands` | `commands/` |
| `skills` | `skills/` |
| `agents` | `agents/` |
| `plugins` | `plugins/` |
| `hooks` | `hooks/` |
| `projects` | `projects/` |

`.claude.json` and `.credentials.json` are always `own` and are not
configurable: Claude Code creates and owns them, and sharing either between
profiles would defeat the isolation the tool exists to provide.

### Resource modes

| Mode | Behaviour | Applies to | Default users |
|---|---|---|---|
| `link` | Symlink to `<source_dir>/<resource>` | directories | `commands`, `skills`, `agents`, `plugins`, `hooks` |
| `copy` | Copied once from source; refreshed by `apply --sync` | files, directories | `CLAUDE.md` |
| `own` | Profile-private; source is never consulted | files, directories | `projects`, `.claude.json`, `.credentials.json` |
| `merge` | Regenerated on every apply as source JSON deep-merged with the profile's `patch` | JSON files | `settings.json`, `settings.local.json` |

`merge` is the mechanism that later phases build on. Because the file is
regenerated from source on every apply, changes to `~/.claude/settings.json`
keep flowing into every profile, while each profile's overrides stay
declarative in one place. This replaces `cpm`'s copy-then-detect-drift
approach: there is no drift to detect, because the file is derived.

Deep-merge semantics: objects merge key-by-key with the patch winning;
arrays and scalars are replaced wholesale by the patch. A `null` in the patch
deletes the key.

### Two defaults worth stating explicitly

- **`projects` defaults to `own`.** Session history is per profile, so
  `--resume` in a personal terminal cannot surface a work session. The cost
  is that history is siloed; the mode is a one-line flip to `link`.
- **`hooks` defaults to `link`.** Hook commands in `settings.json` reference
  absolute paths under `~/.claude/hooks/`, so they execute correctly from any
  profile regardless of this setting; linking the directory keeps the two
  consistent.

## Core architecture: Plan then Apply

`cpx-core` never mutates the filesystem from an operation function. Every
operation computes a `Plan` — an ordered list of typed actions — and a
separate `execute` function runs it.

```rust
struct Plan { actions: Vec<PlannedAction> }
struct PlannedAction { action: Action, risk: Risk, description: String }

enum Action {
    CreateDir      { path: PathBuf },
    Symlink        { link: PathBuf, target: PathBuf },
    CopyFile       { src: PathBuf, dst: PathBuf },
    CopyTree       { src: PathBuf, dst: PathBuf },
    WriteMerged    { dst: PathBuf, source: Option<PathBuf>, patch: serde_json::Value },
    WriteWrapper   { path: PathBuf, profile: String },
    WriteShim      { path: PathBuf, profile: String },
    WriteEnvrcBlock{ envrc: PathBuf, profile: String },
    RemoveEnvrcBlock { envrc: PathBuf },
    GitInfoExclude { repo: PathBuf, line: String },
    RunDirenvAllow { dir: PathBuf },
    Backup         { path: PathBuf, to: PathBuf },
    RemoveGenerated{ path: PathBuf },
}

enum Risk { Safe, OverwritesGenerated, OverwritesForeign }
```

`Risk` classifies each action by what it would displace. `Safe` means the
target is absent or already exactly correct. `OverwritesGenerated` means the
target is ours — a symlink we created or a file carrying our marker — and is
replaced without ceremony. `OverwritesForeign` means the target is something
we did not create, and requires `--force`. A plan's risk is the maximum of
its actions'.

This shape serves three purposes simultaneously: `cpx apply --dry-run`
renders the plan; the Phase 2 UI renders the same plan as a confirmation
sheet with no additional logic; and tests assert on `Plan` values rather than
inspecting temporary directories, which makes exhaustive coverage of
mode × prior-state combinations cheap.

`Plan`/`execute` governs filesystem materialization only. Commands that edit
`config.toml` itself — `init`, `clone` — rewrite that one file directly,
preserving comments and key order, and then report what `apply` would do
next. Mixing configuration editing into the materialization plan would make
both harder to reason about.

### Safety invariants

Each invariant is enforced in `execute` and covered by a test.

1. **`source_dir` is read-only.** No action may target a path inside
   `source_dir`. `execute` validates the whole plan before running any
   action and rejects the entire plan on violation.
2. **Foreign files are never overwritten.** A target that exists and is
   neither a symlink we created nor a file carrying the generated-by marker
   in its first five lines is classified `OverwritesForeign`. Such a plan is
   refused unless `--force`, and `--force` inserts a `Backup` action before
   the write.
3. **Nothing is deleted.** Removal is a rename to `<path>.cpx.bak.<n>`. The
   sole exception is `RemoveGenerated`, which requires the marker.
4. **Profile directories are mode `0700`.** No action reads a credential
   value; credential handling is existence-and-metadata only.

## Wrappers and shims

For each profile, `apply` writes `<bin_dir>/<wrapper_prefix><name>`. The
script unsets inherited `CLAUDE_*` and `ANTHROPIC_*` variables, exports
`CLAUDE_CONFIG_DIR`, `CLAUDE_PROFILE`, and the profile's `env`, passes
`--add-dir` for each entry in `add_dirs`, and applies the profile's `model`
unless `--model` appears in the arguments. Subcommands that manage Claude
itself (`mcp`, `auth`, `doctor`, `install`, `setup-token`, `update`,
`upgrade`, `agents`, `auto-mode`, `plugin`, `plugins`) exec through directly.

Three changes relative to `cpm`'s wrapper:

- **The real Claude binary is resolved at apply time and exec'd by absolute
  path.** `cpm` emits `exec claude "$@"`, which recurses infinitely if
  `bin_dir` precedes the real binary on `PATH`. The resolved path is recorded
  in the plan so `--dry-run` shows which binary a wrapper will call.
- **Collision detection.** A wrapper path that exists without our marker is
  `OverwritesForeign` and stops the apply. This matters on this machine:
  `~/.local/bin/claude-company` already exists, generated by an unrelated
  "ai-primitives" system. `cpx doctor` reports foreign `claude-*` executables
  as information, not as errors.
- **A per-profile shim.** Each profile directory gets
  `<profile>/bin/claude`, execing the resolved binary with that profile's
  environment. Combined with `PATH_add` in the generated `.envrc`, the plain
  `claude` command inside a bound directory runs the correct profile, so the
  `claude-<name>` wrappers become optional rather than the primary interface.

## Credentials

Status only. No token value is ever read.

On macOS, Claude Code stores the OAuth token in the Keychain under service
`Claude Code-credentials-<first 8 hex chars of sha256(config_dir)>`, with the
default `~/.claude` login using the bare `Claude Code-credentials`. The
account name is `$USER`, falling back to `claude-code-user` when `$USER` does
not match `^[a-zA-Z0-9._-]+$`. Existence is checked with
`security find-generic-password -a <account> -s <service>`, deliberately
without `-w`, so no secret is returned and macOS shows no access prompt.

On Linux and in CI the token lives in `<profile>/.credentials.json`, which is
checked as a fallback.

The logged-in account's email address and organization UUID are read from the
profile's `.claude.json` (`oauthAccount.emailAddress`,
`oauthAccount.organizationUuid`), which Claude Code writes regardless of
where the token itself is stored.

The Keychain lookup is an injectable function so tests never touch the real
Keychain.

## Directory binding

The registry `~/.claude-profiles/bindings.toml` is the index; the managed
block inside each directory's `.envrc` is the mechanism.

```toml
[[bindings]]
path       = "/Users/me/Work/company-project"
profile    = "work"
envrc_hash = "sha256:..."
```

`envrc_hash` is the hash of the managed block as written, which lets
`cpx bindings` detect hand-edits without re-deriving the whole file.

### Generated block

```sh
# >>> cpx: work >>>
export CLAUDE_CONFIG_DIR="$HOME/.claude-profiles/work"
export CLAUDE_PROFILE="work"
export ANTHROPIC_LOG="debug"
PATH_add "$HOME/.claude-profiles/work/bin"
# <<< cpx <<<
```

### Behaviour

- **Idempotent.** Re-binding replaces the content between the markers and
  preserves the rest of the file byte-for-byte. Binding a directory already
  bound to a different profile rewrites the single block rather than
  appending a second one.
- **`.git/info/exclude`, not `.gitignore`.** When the directory is a git
  repository and `.envrc` is not already tracked, the ignore line goes in
  `.git/info/exclude`, which is local to the clone. `cpm` appends to
  `.gitignore`, which commits personal tooling into a shared repository.
- **`direnv allow` runs automatically** when `direnv` is on `PATH`. When it
  is absent the block is still written and the CLI says so.
- **`cpx unbind`** removes only the managed block, deletes the `.envrc` only
  if nothing else remains in it, and removes the `.git/info/exclude` line if
  we added it.
- **`cpx bindings`** lists each binding with a health verdict: healthy,
  directory missing, profile deleted, block absent, block hand-edited (hash
  mismatch), or direnv not allowed.

## CLI surface

| Command | Behaviour |
|---|---|
| `cpx init` | Interactive first run: detect `~/.claude`, scaffold `config.toml` |
| `cpx apply [--dry-run] [--sync] [--force]` | Materialize all profiles; `--dry-run` prints the plan and exits; `--sync` refreshes `copy` resources; `--force` permits backed-up overwrites |
| `cpx list` | Profiles with authentication status and health |
| `cpx show <profile>` | One profile's fully resolved configuration |
| `cpx status` | Summary of what `apply` would change |
| `cpx doctor` | Full diagnostics |
| `cpx bind <profile> [dir]` | Bind a directory (defaults to cwd) |
| `cpx unbind [dir]` | Remove a binding |
| `cpx bindings` | List bindings with health |
| `cpx which` | Active profile here: environment, then binding, then none |
| `cpx run <profile> -- <args>` | One-shot invocation under a profile |
| `cpx clone <src> <dst>` | Duplicate a profile's configuration without credentials |

Every read command accepts a global `--json` flag emitting a stable
machine-readable form, so the Phase 2 UI and user scripts share one contract.

### `cpx doctor` checks

`source_dir` exists · broken symlinks inside profile directories · stale
wrappers carrying our marker for profiles that no longer exist · foreign
`claude-*` executables in `bin_dir` · `bin_dir` present on `PATH` · `direnv`
installed · binding health for every registry entry · credential status per
profile · `CLAUDE_CONFIG_DIR` set in the invoking environment, which silently
routes sessions to an unintended account.

## Error handling

`cpx-core` returns typed errors via `thiserror`, one variant per failure
class (config parse, resource conflict, foreign overwrite, missing source,
keychain unavailable, direnv failure). `cpx-cli` converts them at the
boundary with `anyhow` and renders remediation text. Planning errors and
execution errors are distinct types: a plan that cannot be computed is a
configuration problem, while a plan that fails mid-execution reports which
actions completed.

## Testing

Test-driven, per the project's development workflow.

Every path in the system derives from a `Layout { source_dir, profiles_dir,
bin_dir }`. Tests construct a real `Layout` inside a `tempfile::TempDir` and
use the real filesystem — there is no filesystem abstraction to mock and
therefore none to drift from reality. The only injected seams are the
Keychain lookup and the `direnv` invocation.

Two test tiers:

- **Plan tests** assert on `Plan` values. These are fast enough to cover
  every resource mode against every prior filesystem state (absent, correct,
  our symlink pointing elsewhere, our generated file, a foreign file, a
  directory where a file is expected).
- **Execute tests** assert on the resulting directory tree for a smaller,
  representative set of plans, plus one test per safety invariant asserting
  that a violating plan is rejected in full.

## Open questions

None. Decisions deferred to later phases are listed under Goals as
non-goals, not as unknowns.
