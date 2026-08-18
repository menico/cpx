# cpx — Claude profile manager

Run several Claude Code accounts on one machine. Each profile gets its own
config directory, its own login, its own command, and can be bound to a
directory so the right account is used automatically.

Per-profile statuslines and per-profile MCP servers need nothing extra: both
live in the config directory, so each profile already has its own. Settings
sync across machines is deliberately not included.

## Install

```bash
brew tap menico/tap
brew install cpx           # the command
brew install --cask cpx-app  # the menu-bar app
```

Homebrew asks you to trust a third-party cask the first time:
`brew trust menico/tap`.

Updating:

```bash
brew update
brew upgrade cpx           # the command
brew upgrade --cask cpx-app
```

The app lives in the menu bar, not the Dock. To start it at login, add it
under System Settings → General → Login Items.

The app is ad-hoc signed rather than notarised. The cask clears the quarantine
flag for you, so it opens without a right-click.

### From source

```bash
make install
```

Installs the command into `~/.local/bin` and the app into `/Applications`.
If you have also installed through Homebrew, remove one of them — whichever
comes first on `PATH` wins, which is rarely the one you meant.

## Quick start

```bash
cpx init --profile work --profile personal
cpx apply

claude-work auth login        # sign each profile in once
claude-personal auth login

cd ~/Work/some-project
cpx bind work                 # this directory now uses `work`
```

Inside a bound directory, the plain `claude` command *is* that profile —
there is no separate command to remember.

## How it works

Each profile is a directory under `~/.claude-profiles/<name>/` used as
`CLAUDE_CONFIG_DIR`. What goes into it is declared per resource:

| Mode | Behaviour |
|---|---|
| `link` | symlink to `~/.claude/<resource>` — edit once, every profile sees it |
| `copy` | seeded once from source, then yours to diverge (`apply --sync` refreshes) |
| `own` | profile-private; source is never consulted |
| `ignore` | cpx does not manage this resource at all |
| `merge` | regenerated each apply as the source JSON deep-merged with this profile's patch |

`merge` is what lets a profile override part of `settings.json` — a model, a
statusline, permissions — while the rest of your base settings keep flowing
through. There is no drift to reconcile, because the file is derived.

```toml
# ~/.claude-profiles/config.toml
version = 1
source_dir = "~/.claude"
bin_dir    = "~/.local/bin"

[defaults.resources]
settings = "merge"
commands = "link"
skills   = "link"
projects = "own"

[profiles.work]
description = "Company account"
model       = "sonnet"
add_dirs    = ["~/Work/company"]
env         = { ANTHROPIC_LOG = "debug" }

[profiles.work.resources.settings]
mode  = "merge"
patch = { statusLine = { type = "command", command = "echo work" } }
```

## Nothing happens without a plan

Every filesystem change is planned first and executed second. `cpx apply
--dry-run` prints the exact list; the plan is also what the desktop app will
show for confirmation.

```
     mkdir   ~/.claude-profiles/work
     write   ~/.claude-profiles/work/settings.json
     link    ~/.claude-profiles/work/commands -> ~/.claude/commands
  !  write   ~/.local/bin/claude-work
```

A `!` means the target is something cpx did not write — or wrote and you have
since edited. Those are refused unless you pass `--force`, and `--force` backs
each one up before replacing it.

Four invariants, each covered by a test:

1. **`source_dir` is read-only.** No action may write inside `~/.claude`. The
   whole plan is rejected if one tries.
2. **Foreign files are never overwritten** without `--force`.
3. **Nothing is deleted.** Removal is a rename to `<path>.cpx.bak`. The only
   exception is removing a file cpx generated, re-verified at execution time.
4. **Profile directories are `0700`** — credentials live there.

Ownership is tracked by content hash in `~/.claude-profiles/state.json`, so a
file you hand-edit is recognised as yours and protected, and applying twice
converges to nothing.

## Adopting what you already have

If you already run Claude accounts by hand out of `~/.claude-work`,
`~/.claude-personal` and so on, cpx manages them where they are:

```bash
cpx adopt                      # lists what it found, and what each would keep
cpx adopt ~/.claude-work       # registers it; nothing inside it changes
cpx apply                      # adds its command, and only that
```

Adoption is deliberately inert. Every resource the directory already has is
set to `own` and every one it lacks is set to `ignore`, so `apply` writes the
wrapper and the shim and touches nothing else — not the plugins, not the
sessions, not `settings.json`. The shim lives under the cpx root rather than
inside your directory. Because the path does not move, the Keychain entry
still matches and the profile stays signed in.

You can then opt any resource into `link` or `merge` deliberately, seeing the
plan first.

## Per-directory profiles

`cpx bind <profile>` writes a marker-delimited block into that directory's
`.envrc` and runs `direnv allow`:

```sh
# >>> cpx: work >>>
export CLAUDE_CONFIG_DIR='/Users/you/.claude-profiles/work'
export CLAUDE_PROFILE='work'
PATH_add '/Users/you/.claude-profiles/work/bin'
# <<< cpx <<<
```

Everything outside the markers is preserved byte-for-byte. `cpx unbind`
restores the original file exactly, and removes the `.envrc` if it only ever
held our block. The `.envrc` is ignored through `.git/info/exclude`, which is
local to your clone — never `.gitignore`, which is shared with everyone else.

`cpx bindings` lists every bound directory and flags ones that have gone
stale: directory deleted, profile removed, block hand-edited, direnv not
allowed.

## Credentials

cpx never reads a token. On macOS the login lives in the Keychain under
`Claude Code-credentials-<sha256(config_dir)[..8]>`, so profiles cannot
collide with each other or with your default `~/.claude` session. The
existence probe deliberately omits `security -w`, so nothing is returned and
macOS shows no access prompt.

Switching an account always goes through that profile's own command:

```bash
claude-work auth status
claude-work auth logout    # touches only the work entry
claude-work auth login
```

## Commands

| Command | |
|---|---|
| `cpx init [--profile NAME]` | write a starter config |
| `cpx apply [--dry-run] [--sync] [--force]` | make the filesystem match the config |
| `cpx list` / `cpx show <profile>` | profiles and logins / one profile in full |
| `cpx status` / `cpx doctor [-v]` | what apply would change / diagnostics |
| `cpx bind <profile> [dir]` / `cpx unbind [dir]` / `cpx bindings` | directory binding |
| `cpx which` | which profile applies here |
| `cpx run <profile> -- <args>` | one-shot under a profile |
| `cpx clone <from> <to>` | duplicate a profile's config, without credentials |
| `cpx adopt [dir]` | manage a config directory you already have, in place |
| `cpx --version` | the installed version |
| `cpx profile add\|rm <name>` | edit the config, preserving comments |

Every read command takes `--json`.

`cpx doctor` exits non-zero when something is genuinely broken. It also
catches the quiet one: a `CLAUDE_CONFIG_DIR` left set in your shell, which
overrides whichever profile you think you are using.

## Notes

- **`~/.claude` is shown but never managed.** It is the directory profiles
  inherit from, and usually a working account in its own right, so `cpx list`
  and the app report which account it is signed into and leave it alone.
- **Existing `~/.claude-*` directories are adopted in place, not moved.** A
  login is keyed to its config directory's path, so `cpx adopt` manages the
  directory where it already sits and you never sign in again.
- **Wrapper name collisions are refused, not resolved.** If
  `~/.local/bin/claude-<name>` exists and cpx did not write it, apply stops.
- Wrappers exec Claude by absolute path, so a wrapper directory earlier on
  `PATH` than the real binary cannot cause recursion.

## The menu-bar app

```bash
make dev     # runs the app against a live UI
make app     # builds cpx.app and a .dmg
```

It lives in the menu bar: click the icon for a popover listing every profile,
which account it is signed into, and whether it is built. Each profile carries
an identity colour, and that colour is the through-line — it marks the profile
in the list and every directory bound to it, so colour always answers "which
account".

Pending changes never apply silently. The footer shows a count; expanding it
shows the real plan, with the same three-state gutter the CLI prints, so the
app and the terminal never describe a change differently.

The app is a second consumer of `cpx-core`, not a reimplementation: it makes
no decision the CLI does not.

## Development

```bash
make test    # 311 Rust tests, clippy with warnings denied, and a UI typecheck
make dev     # the app against a live UI
```

Releases are cut locally rather than in CI, so they can be rehearsed:

```bash
make release-dry VERSION=0.2.0   # build everything, publish nothing
make release VERSION=0.2.0       # publish, and point the tap at it
```

Opening `ui` in a plain browser (`pnpm --dir ui dev`) runs the interface
against fixtures, so the frontend can be built and reviewed without launching
the desktop shell.

`cpx-core` holds every decision and knows nothing about terminals;
`cpx-cli` is argument parsing and rendering. The desktop app will be a second
consumer of the same API.

Drive the CLI against a throwaway installation with `CPX_HOME` and
`CPX_ROOT`.

## License

MIT
