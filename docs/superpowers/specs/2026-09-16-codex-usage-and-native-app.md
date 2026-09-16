# Codex profiles, usage, spend, and the native app

Date: 2026-09-16. Status: implemented.

## What changed

cpx grew from a Claude Code profile manager into one for Claude Code *and*
Codex, learned to report each account's rate limits and local spend, and
had its Tauri/React app replaced by a native Swift menu bar app in the
style of [CodexBar](https://github.com/steipete/CodexBar). The Rust core
stayed; the app drives the `cpx` command and makes no decision of its own.

## Providers

`Provider { Claude, Codex }` in `cpx-core::config` carries everything that
differs between the tools: the home variable (`CLAUDE_CONFIG_DIR` /
`CODEX_HOME`), the binary, the default source (`~/.claude` / `~/.codex`),
the wrapper prefix (`claude-` / `codex-`), the inherited variables a
wrapper scrubs, and the resource keys that exist in that home.

A profile declares `provider = "codex"`; absent means Claude, so every
existing config reads unchanged. The top-level `source_dir`,
`wrapper_prefix` and `[defaults.resources]` remain Claude's (with `[claude]`
as the explicit spelling); Codex has `[codex]` and
`[codex.defaults.resources]`.

Codex resource keys: `config` (config.toml, mergeable), `AGENTS.md`,
`skills`, `prompts`, `rules`, `hooks`, `hooks_json`, `plugins`, `memories`,
`sessions`. `merge` now works for TOML as well as JSON: the source is
converted to JSON, deep-merged with the patch, and rendered back. Comments
in the source are not preserved, exactly as with a merged `settings.json`.

Everything provider-specific dispatches on the enum: wrappers, shims,
bindings, credentials (`auth.json` with the identity token's `email` and
`chatgpt_plan_type` claims), adoption (`~/.codex-*` detected by `auth.json`
or two of config.toml / sessions / history.jsonl / version.json /
installation_id / log), doctor, default sessions. The statusline and plugin
toggles are Claude-only and say so.

## Bindings hold one profile per provider

A directory can be bound to a Claude profile and a Codex profile at once.
Both live in the single managed `.envrc` block, rendered in provider order
so the block is identical however it was built up. Registry entries carry
`provider` (defaulting to Claude for old registries) and share the block
hash. `cpx unbind --provider codex` rewrites the block without that
provider; a plain `cpx unbind` removes it.

## Usage

`cpx usage` is the one command that reads a token. Claude: the Keychain
item Claude Code wrote (via the Security framework, so macOS attributes
the prompt to cpx and "Always Allow" sticks), then
`GET https://api.anthropic.com/api/oauth/usage` with
`anthropic-beta: oauth-2025-04-20`. Codex: `auth.json`, then
`GET <chatgpt_base_url>/wham/usage` with `ChatGPT-Account-Id`. Windows are
normalised to `{id, title, usedPercent, windowMinutes, resetsAt}`.

Tokens are never refreshed by cpx. Redeeming a refresh token from a second
process risks invalidating the one the tool holds. An expired Claude token
is reported with the remedy of running the profile's command once; a
stale Codex token routes the question through `codex app-server`
(newline-delimited JSON-RPC: `initialize`, `initialized`,
`account/rateLimits/read`), which refreshes `auth.json` itself.

`CPX_NO_KEYCHAIN=1` makes the Claude path report "denied" without asking,
for tests and for anyone who would rather cpx did not.

## Spend

`cpx cost` sums the local transcripts: Claude `projects/**/*.jsonl`
assistant records (last record per message id wins, buckets bill
additively, 1-hour cache writes at twice the input rate) and Codex
`sessions/**/rollout-*.jsonl` `token_count` events (deltas of the running
total, cached reads a subset of input). Prices are a bundled table in USD
per token; unknown models are reported as unpriced, never as free. Per-file
rows are cached under `<root>/cache/cost/` by size and mtime, so a refresh
re-reads only what changed.

## The app

`app/` is a Swift package: `CpxKit` (the JSON contract with the command,
the CLI runner, the adaptive refresh policy, quota-alert logic, status
feeds, formatting) and `CpxApp` (AppKit status items, an NSPopover hosting
SwiftUI, a settings window, notifications). `app/build.sh` assembles
`cpx.app` with the `cpx` binary inside `Contents/MacOS`, which the app
prefers over one on PATH so the two never disagree.

Refresh cadence follows CodexBar's table: 2 minutes within 5 minutes of a
menu open, 5 within an hour, 15 within four, 30 otherwise, 30 under low
power or thermal pressure. Alerts fire once per threshold of percent
remaining and re-arm on recovery; a depleted session and its return are
announced. Provider status comes from the Statuspage `status.json` feeds.

The icon is an 18-point template meter: session lane above, weekly below,
filled by what remains; stale readings dim it, an incident adds a dot. One
merged icon (pinned or busiest account) or one per account.
