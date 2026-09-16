# cpx — Claude Code and Codex profile manager

Run several Claude Code and Codex accounts on one machine. Each profile gets
its own config directory, its own login, its own command, and can be bound to
a directory so the right account is used automatically. A menu bar app shows
every account's rate limits, when they reset, and what its transcripts have
cost.

Per-profile MCP servers need nothing extra: they live in the config directory,
so each profile already has its own. Settings sync across machines is
deliberately not included.

## Install

```bash
brew tap menico/tap
brew trust menico/tap        # Homebrew requires this for any third-party tap
brew install cpx             # the command
brew install --cask cpx-app  # the menu-bar app
```

Without the `brew trust` line, Homebrew refuses with *"Refusing to load formula
menico/tap/cpx from untrusted tap"*. It is asked once per machine and covers
both the formula and the cask.

Updating:

```bash
brew update
brew upgrade cpx             # the command
brew upgrade --cask cpx-app  # the app
```

The app lives in the menu bar, not the Dock. Its settings have a switch to
start it at login.

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
cpx profile add gpt --provider codex
cpx apply

claude-work auth login        # sign each profile in once
claude-personal auth login
codex-gpt login

cd ~/Work/some-project
cpx bind work                 # this directory now uses `work` for Claude
cpx bind gpt                  # and `gpt` for Codex
```

Inside a bound directory, the plain `claude` and `codex` commands *are* those
profiles — there is no separate command to remember.

## How it works

Each profile is a directory under `~/.claude-profiles/<name>/`, used as
`CLAUDE_CONFIG_DIR` for a Claude profile or `CODEX_HOME` for a Codex one.
What goes into it is declared per resource:

| Mode | Behaviour |
|---|---|
| `link` | symlink to `~/.claude/<resource>` — edit once, every profile sees it |
| `copy` | seeded once from source, then yours to diverge (`apply --sync` refreshes) |
| `own` | profile-private; source is never consulted |
| `ignore` | cpx does not manage this resource at all |
| `merge` | regenerated each apply as the source JSON deep-merged with this profile's patch |

`merge` is what lets a profile override part of `settings.json` — a model, a
statusline, permissions — while the rest of your base settings keep flowing
through. There is no drift to reconcile, because the file is derived. For a
Codex profile the same applies to `config.toml`: the MCP servers and trusted
projects come from `~/.codex`, and the profile's patch sits on top.

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

[profiles.gpt]
provider = "codex"
model    = "gpt-5.5"

[profiles.gpt.resources.config]
mode  = "merge"
patch = { model_reasoning_effort = "high" }
```

### Codex profiles

`provider = "codex"` makes a profile a Codex home. Its command is
`codex-<name>`, its shim is `codex`, and it is built from `~/.codex` (set
`[codex] source_dir` to change that). The resources it knows are the ones a
Codex home has:

| Key | File | Default |
|---|---|---|
| `config` | `config.toml` | `merge` |
| `AGENTS.md` | `AGENTS.md` | `copy` |
| `skills`, `prompts`, `rules`, `hooks`, `plugins` | directories | `link` |
| `hooks_json` | `hooks.json` | `copy` |
| `memories`, `sessions` | directories | `own` |

The top-level `source_dir` and `[defaults.resources]` are Claude's, so a
config written before Codex support reads exactly as it did. Codex defaults
live under `[codex.defaults.resources]`.

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

If you already run Claude or Codex accounts by hand out of `~/.claude-work`,
`~/.codex-work` and so on, cpx manages them where they are:

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

## Statusline badges

A statusline script written for the default config directory has no idea which
profile it is running under. A common one reads `~/.claude.json` directly, so
it reports the *default* account in every profile — the statusline says one
thing while the session is signed in as someone else.

```bash
cpx statusline show            # what each profile uses, and whether it has a badge
cpx statusline set work        # put a badge in front of work's statusline
cpx statusline set --base      # and in front of the default session's
cpx statusline clear work      # put back exactly what was there
```

The badge is a generated wrapper that prints the profile, then hands the
session on stdin to whatever statusline was already configured:

```
● work │ <your existing statusline, untouched>
```

Your script is never edited, so reinstalling it — `npx` and friends — cannot
undo this, and installing a badge twice wraps the original rather than nesting.
The name and colour come from `CLAUDE_PROFILE` and `CPX_PROFILE_COLOR` at run
time, so one badge installed on the base settings still shows the right profile
in every profile that inherits it.

For a profile whose settings are `merge`d, the badge is recorded as a patch in
`config.toml` and applied on the next `cpx apply`; otherwise the profile's own
`settings.json` is edited, and the previous file is kept alongside it.

### Editing the script

The app opens the script a profile's statusline actually runs, resolved from
the configured command, and saves it with the previous contents kept
alongside. Where that file belongs to an installer — a `statusline.mjs` put
there by `npx`, say — it offers to copy it into the profile first, because
editing the original would be undone on its next update. The copy lives under
the cpx root and the profile is repointed at it.

There is nothing to open when the statusline is a command with no file behind
it, such as `npx something` or a binary on `PATH`; the app says so rather than
guessing.

## Skills

Skills reach a profile two ways, and the two have different switches.

```bash
cpx skills list work           # your own, plus what each plugin brings
cpx skills list work --all     # naming every skill a plugin provides
cpx skills disable work noisy  # move it out of skills/, keeping it
cpx skills enable work noisy
cpx skills remove work noisy   # move it to skills.removed/, still not deleted
cpx skills plugin work posthog@claude-plugins-official --off
```

Skills of your own are directories under `<config>/skills/`, and Claude loads
whatever it finds there — so switching one off means moving it to
`skills.disabled/` in the same profile. Nothing is deleted, and enabling moves
it back.

Plugin skills come with their plugin, and `enabledPlugins` only toggles a whole
plugin. That is the granularity offered rather than pretending otherwise: one
plugin can easily bring a hundred skills.

Where `skills/` is shared between profiles — the `link` mode — turning a skill
off turns it off everywhere, so that is called out before you do it.

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

A directory can hold one profile per tool. Binding a Codex profile beside a
Claude one adds it to the same block:

```sh
# >>> cpx: work, gpt >>>
# Claude Code: profile work
export CLAUDE_CONFIG_DIR='/Users/you/.claude-profiles/work'
export CLAUDE_PROFILE='work'
PATH_add '/Users/you/.claude-profiles/work/bin'
# Codex: profile gpt
export CODEX_HOME='/Users/you/.claude-profiles/gpt'
export CODEX_PROFILE='gpt'
PATH_add '/Users/you/.claude-profiles/gpt/bin'
# <<< cpx <<<
```

`cpx unbind --provider codex` takes one of them out; `cpx unbind` takes the
block out. `cpx bindings` lists every bound directory and flags ones that have
gone stale: directory deleted, profile removed, block hand-edited, direnv not
allowed.

## Credentials

Everything but `cpx usage` works from the *existence* of a login, never its
value. On macOS a Claude login lives in the Keychain under
`Claude Code-credentials-<sha256(config_dir)[..8]>`, so profiles cannot
collide with each other or with your default `~/.claude` session; the
existence probe deliberately omits `security -w`, so nothing is returned and
macOS shows no access prompt. A Codex login is the `auth.json` in its home,
which is read for the account name and plan and nothing else.

Switching an account always goes through that profile's own command:

```bash
claude-work auth status
claude-work auth logout    # touches only the work entry
claude-work auth login
codex-gpt login
```

## Usage and spend

```bash
cpx usage                  # every account's rate limits, and when they reset
cpx usage work @codex      # named profiles; @claude and @codex are the default sessions
cpx cost                   # what the local transcripts would cost at list prices
cpx cost --days 90 --json
```

`cpx usage` is the one command that reads a token: it asks the same endpoint
each tool asks for its own `/usage` screen, and forgets the token when the
request is done. For a Claude profile that means reading its Keychain item,
so the first time macOS asks whether cpx may; *Always Allow* makes later
reads silent. Nothing is written back, and nothing is refreshed — a Claude
token that has expired is reported with the fix (run the profile once), and a
stale Codex token is refreshed by `codex app-server` itself, which cpx asks
instead. Set `CPX_NO_KEYCHAIN=1` to never be asked.

```
work           claude   me@example.com (Max 5x)
  Session                 12.5% used   resets in 3h 12m
  Weekly                  40.0% used   resets in 4d 2h
  Opus weekly              3.2% used   resets in 4d 2h
```

`cpx cost` reads the transcripts each tool already writes — Claude's
`projects/*.jsonl`, Codex's `sessions/**/rollout-*.jsonl` — and prices the
tokens at published list prices. A subscription is not billed per token, so
this is what the usage would have cost on the API rather than a bill; it is
still the most honest measure of how much each account is doing. Models the
price table does not know are reported as unpriced, never as free. Per-file
results are cached under `~/.claude-profiles/cache/`, so only changed
transcripts are re-read.

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
| `cpx statusline show\|set\|clear\|script` | profile badge in front of an existing statusline, and the script behind it |
| `cpx skills list\|enable\|disable\|remove\|plugin` | what a profile can call on |
| `cpx usage [account…]` | rate limits and resets, live from each provider |
| `cpx cost [account…] [--days N]` | spend estimated from local transcripts |
| `cpx paths` | where the configuration lives |
| `cpx --version` | the installed version |
| `cpx profile add [--provider codex]\|rm\|set\|resource` | edit the config, preserving comments |

Every command takes `--json`; that is the interface the app uses.

`cpx doctor` exits non-zero when something is genuinely broken. It also
catches the quiet one: a `CLAUDE_CONFIG_DIR` left set in your shell, which
overrides whichever profile you think you are using.

## Notes

- **`~/.claude` and `~/.codex` are shown but never managed.** They are the
  directories profiles inherit from, and usually working accounts in their
  own right, so `cpx list` and the app report which account each is signed
  into and leave them alone.
- **Existing `~/.claude-*` and `~/.codex-*` directories are adopted in
  place, not moved.** A login is keyed to its directory's path, so
  `cpx adopt` manages the directory where it already sits and you never sign
  in again.
- **Wrapper name collisions are refused, not resolved.** If
  `~/.local/bin/claude-<name>` exists and cpx did not write it, apply stops.
- Wrappers exec Claude by absolute path, so a wrapper directory earlier on
  `PATH` than the real binary cannot cause recursion.

## The menu-bar app

```bash
make app     # builds target/app/cpx.app
make dev     # runs it with the popover open
```

A native Swift app, in the spirit of [CodexBar](https://github.com/steipete/CodexBar)
but built around profiles. The menu bar icon is a small meter — session lane
above, weekly below, filled by what remains — for the account closest to a
limit, or the one you pin, or one icon per account. Click it for the popover:

- **Accounts.** Every profile and each tool's default session, with its
  session and weekly bars, reset countdowns, plan, and today / 7-day / 30-day
  spend. Open one for every limit, extra usage, credits, spend by model and by
  day, and the profile's configuration: name, colour, model, sign in and out,
  statusline badge, skills, what it inherits and how, duplicate, remove.
- **Directories.** What is bound where, and to which profile of each tool.
- **Health.** The same checks as `cpx doctor`.

Pending changes never apply silently. The footer shows a count; expanding it
shows the real plan, with the same gutter the CLI prints, so the app and the
terminal never describe a change differently.

Usage refreshes on an adaptive cadence — every two minutes while you are
looking, every half hour when you are not, slower on battery — or on a fixed
interval, or by hand. Notifications say when a window's remaining share
crosses a threshold (50% and 20% by default, once each, re-armed on
recovery), when a session runs out and when it is back, and optionally when a
week resets. Provider incidents from the Anthropic and OpenAI status pages
show as a dot on the icon.

The app makes no decision the command does not: everything it shows is a
`cpx … --json` call, and everything it changes is the matching subcommand.
The command ships inside the app bundle so the two can never disagree.

## Development

```bash
make test    # the Rust suite, clippy with warnings denied, and the Swift tests
make dev     # the app with its popover open
```

`cpx-core` holds every decision and knows nothing about terminals or
windows; `cpx-cli` is argument parsing and rendering; `app/` is a Swift
package whose `CpxKit` library holds the contract with the command, the
refresh policy and the alert logic, and whose `CpxApp` target is the menu bar
app.

Releases are cut locally rather than in CI, so they can be rehearsed:

```bash
make release-dry VERSION=0.2.0   # build everything, publish nothing
make release VERSION=0.2.0       # publish, and point the tap at it
```

Drive the CLI against a throwaway installation with `CPX_HOME` and
`CPX_ROOT`. Point the app at a different command with `CPX_CLI`.

## License

MIT
