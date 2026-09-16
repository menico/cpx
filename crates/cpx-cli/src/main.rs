//! `cpx` — Claude Code and Codex profile manager.
//!
//! This crate is deliberately thin: argument parsing and rendering. Every
//! decision lives in `cpx-core`, so the desktop app can make the same ones.

mod render;
mod session;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cpx_core::binding::{self, Bindings};
use cpx_core::config::Provider;
use cpx_core::config_edit;
use cpx_core::credentials;
use cpx_core::doctor::{self, Severity};
use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::materialize::plan_apply;
use serde_json::json;
use session::Session;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "cpx",
    version,
    about = "Run several Claude Code and Codex accounts side by side",
    long_about = "cpx gives each Claude Code or Codex account its own config directory, \
                  wrapper command, and per-directory binding, while sharing whatever you \
                  want shared."
)]
struct Cli {
    /// Emit machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ProviderArg {
    Claude,
    Codex,
}

impl From<ProviderArg> for Provider {
    fn from(value: ProviderArg) -> Provider {
        match value {
            ProviderArg::Claude => Provider::Claude,
            ProviderArg::Codex => Provider::Codex,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Create a starter config.
    Init {
        /// Seed a profile. Repeatable: --profile work --profile personal
        #[arg(long = "profile", value_name = "NAME")]
        profiles: Vec<String>,
    },
    /// Make the filesystem match the config.
    Apply {
        /// Show what would happen and stop.
        #[arg(long)]
        dry_run: bool,
        /// Refresh `copy` resources from source.
        #[arg(long)]
        sync: bool,
        /// Back up and replace files cpx did not write.
        #[arg(long)]
        force: bool,
    },
    /// List profiles with their login status.
    List,
    /// Show one profile's resolved configuration.
    Show { profile: String },
    /// Show what `apply` would change.
    Status,
    /// Diagnose problems.
    Doctor {
        /// Include checks that passed.
        #[arg(long, short)]
        verbose: bool,
    },
    /// Bind a directory to a profile via its .envrc.
    Bind {
        profile: String,
        /// Defaults to the current directory.
        dir: Option<PathBuf>,
    },
    /// Remove a directory's binding.
    Unbind {
        dir: Option<PathBuf>,
        /// Remove only this provider's binding, keeping the other's.
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
    },
    /// List bound directories.
    Bindings,
    /// Show which profile applies here.
    Which,
    /// Run Claude or Codex once under a profile.
    Run {
        profile: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Copy a profile's configuration under a new name, without credentials.
    Clone { from: String, to: String },
    /// Manage an existing Claude or Codex directory in place, without moving it.
    Adopt {
        /// The directory to adopt. Omit to list what could be adopted.
        dir: Option<PathBuf>,
        /// Profile name. Defaults to the directory's own name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Add, remove or edit profiles in the config.
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Show or change which statusline each Claude profile uses.
    #[command(subcommand)]
    Statusline(StatuslineCommand),
    /// List a profile's skills, and turn them on or off.
    #[command(subcommand)]
    Skills(SkillsCommand),
    /// Show how much of each account's rate limits is used, and when they reset.
    Usage {
        /// Profile names. `@claude` and `@codex` name the default sessions.
        /// Omit for every account.
        accounts: Vec<String>,
        /// Never show a Keychain dialog; report logins cpx may not read yet.
        #[arg(long)]
        no_prompt: bool,
    },
    /// Estimate what each account's local transcripts cost.
    Cost {
        /// Profile names. `@claude` and `@codex` name the default sessions.
        accounts: Vec<String>,
        /// How many days back to report.
        #[arg(long, default_value = "30")]
        days: u32,
        /// Re-read every transcript instead of trusting the cache.
        #[arg(long)]
        no_cache: bool,
    },
    /// Where cpx keeps its configuration.
    Paths,
}

#[derive(Subcommand)]
enum SkillsCommand {
    /// List the skills a profile has, from its own directory and its plugins.
    List {
        profile: String,
        /// Show every skill a plugin provides, not just the count.
        #[arg(long)]
        all: bool,
    },
    /// Switch a skill of your own back on.
    Enable { profile: String, skill: String },
    /// Switch a skill of your own off, keeping it in the profile.
    Disable { profile: String, skill: String },
    /// Move a skill out of the profile, keeping a copy.
    Remove { profile: String, skill: String },
    /// Turn a whole plugin on or off for a profile.
    Plugin {
        profile: String,
        /// The `plugin@marketplace` key, as `cpx skills list` shows it.
        key: String,
        #[arg(long, conflicts_with = "on")]
        off: bool,
        #[arg(long)]
        on: bool,
    },
}

#[derive(Subcommand)]
enum StatuslineCommand {
    /// Show the statusline each profile is using.
    Show,
    /// Put a profile badge in front of the statusline already configured.
    Set {
        /// The profile to change. Omit with --base for the default session.
        profile: Option<String>,
        /// Change the default session's statusline (~/.claude/settings.json).
        #[arg(long)]
        base: bool,
        /// Text for the badge. Defaults to the profile name.
        #[arg(long)]
        label: Option<String>,
        /// Seconds between refreshes.
        #[arg(long)]
        refresh: Option<u64>,
    },
    /// Remove the badge, restoring the statusline that was there before.
    Clear {
        profile: Option<String>,
        #[arg(long)]
        base: bool,
    },
    /// Show, save or fork the script a statusline runs.
    Script {
        profile: Option<String>,
        #[arg(long)]
        base: bool,
        /// Replace the script with this file's contents (`-` for stdin).
        #[arg(long, value_name = "FILE", conflicts_with = "fork")]
        save: Option<PathBuf>,
        /// Copy a shared script into the profile and run the copy instead.
        #[arg(long)]
        fork: bool,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Add a profile to the config.
    Add {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        /// Which tool the profile is for.
        #[arg(long, value_enum, default_value = "claude")]
        provider: ProviderArg,
    },
    /// Remove a profile from the config. Its directory is left in place.
    Rm { name: String },
    /// Set a profile's description, model or colour; omit the value to clear it.
    Set {
        name: String,
        /// One of: description, model, color
        field: String,
        value: Option<String>,
    },
    /// Set how a resource reaches a profile.
    Resource {
        name: String,
        /// A resource key, as `cpx show` lists them.
        resource: String,
        /// One of: link, copy, own, merge, ignore
        mode: String,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("cpx: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let as_json = cli.json;
    match cli.command {
        Command::Init { profiles } => cmd_init(&profiles, as_json),
        Command::Apply {
            dry_run,
            sync,
            force,
        } => cmd_apply(dry_run, sync, force, as_json),
        Command::List => cmd_list(as_json),
        Command::Show { profile } => cmd_show(&profile, as_json),
        Command::Status => cmd_status(as_json),
        Command::Doctor { verbose } => cmd_doctor(verbose, as_json),
        Command::Bind { profile, dir } => cmd_bind(&profile, dir, as_json),
        Command::Unbind { dir, provider } => cmd_unbind(dir, provider.map(Into::into), as_json),
        Command::Bindings => cmd_bindings(as_json),
        Command::Which => cmd_which(as_json),
        Command::Run { profile, args } => cmd_run(&profile, &args),
        Command::Clone { from, to } => cmd_clone(&from, &to, as_json),
        Command::Adopt { dir, name } => cmd_adopt(dir, name.as_deref(), as_json),
        Command::Skills(SkillsCommand::List { profile, all }) => {
            cmd_skills_list(&profile, all, as_json)
        }
        Command::Skills(SkillsCommand::Enable { profile, skill }) => {
            cmd_skills_set(&profile, &skill, true, as_json)
        }
        Command::Skills(SkillsCommand::Disable { profile, skill }) => {
            cmd_skills_set(&profile, &skill, false, as_json)
        }
        Command::Skills(SkillsCommand::Remove { profile, skill }) => {
            cmd_skills_remove(&profile, &skill, as_json)
        }
        Command::Skills(SkillsCommand::Plugin {
            profile,
            key,
            off,
            on,
        }) => cmd_skills_plugin(&profile, &key, on || !off, as_json),
        Command::Usage { accounts, no_prompt } => cmd_usage(&accounts, !no_prompt, as_json),
        Command::Cost {
            accounts,
            days,
            no_cache,
        } => cmd_cost(&accounts, days, no_cache, as_json),
        Command::Paths => cmd_paths(as_json),
        Command::Statusline(StatuslineCommand::Show) => cmd_statusline_show(as_json),
        Command::Statusline(StatuslineCommand::Set {
            profile,
            base,
            label,
            refresh,
        }) => cmd_statusline_set(profile.as_deref(), base, label.as_deref(), refresh, as_json),
        Command::Statusline(StatuslineCommand::Clear { profile, base }) => {
            cmd_statusline_clear(profile.as_deref(), base, as_json)
        }
        Command::Statusline(StatuslineCommand::Script {
            profile,
            base,
            save,
            fork,
        }) => cmd_statusline_script(profile.as_deref(), base, save, fork, as_json),
        Command::Profile(ProfileCommand::Add {
            name,
            description,
            provider,
        }) => {
            let provider: Provider = provider.into();
            edit_config(|text| Ok(config_edit::add_profile(text, &name, &description, provider)?))?;
            done(
                as_json,
                json!({ "added": name, "provider": provider }),
                &format!("Added {} profile `{name}`. Run `cpx apply` to create it.", provider.display_name()),
            )
        }
        Command::Profile(ProfileCommand::Rm { name }) => {
            edit_config(|text| Ok(config_edit::remove_profile(text, &name)?))?;
            done(
                as_json,
                json!({ "removed": name }),
                &format!(
                    "Removed profile `{name}` from the config.\nIts directory is still there; delete it by hand if you want it gone."
                ),
            )
        }
        Command::Profile(ProfileCommand::Set { name, field, value }) => {
            edit_config(|text| {
                Ok(config_edit::set_profile_field(text, &name, &field, value.as_deref())?)
            })?;
            done(
                as_json,
                json!({ "profile": name, "field": field, "value": value }),
                &match &value {
                    Some(value) => format!("{name}: {field} = {value}"),
                    None => format!("{name}: {field} cleared"),
                },
            )
        }
        Command::Profile(ProfileCommand::Resource {
            name,
            resource,
            mode,
        }) => {
            edit_config(|text| Ok(config_edit::set_resource_mode(text, &name, &resource, &mode)?))?;
            done(
                as_json,
                json!({ "profile": name, "resource": resource, "mode": mode }),
                &format!("{name}: {resource} is now `{mode}`. Run `cpx apply` to rebuild it."),
            )
        }
    }
}

/// Finish a mutating command: a JSON object for scripts and the app, a
/// sentence for people.
fn done(as_json: bool, value: serde_json::Value, text: &str) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{text}");
    }
    Ok(())
}

/// Rewrite `config.toml` through `edit`, preserving comments and key order.
fn edit_config(edit: impl FnOnce(&str) -> Result<String>) -> Result<()> {
    let path = session::layout()?.config_file();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("no config at {}. Run `cpx init` first.", path.display()))?;
    std::fs::write(&path, edit(&text)?)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

fn cmd_init(profiles: &[String], as_json: bool) -> Result<()> {
    let layout = session::layout()?;
    let path = layout.config_file();
    if path.exists() {
        bail!("{} already exists; edit it directly", path.display());
    }

    let seeds: Vec<(String, String)> = profiles
        .iter()
        .map(|name| (name.clone(), String::new()))
        .collect();
    let text = config_edit::starter_config(&seeds);
    // Validate before writing: a config that does not load is worse than none.
    cpx_core::config::Config::parse(&text, &layout.home)?;

    std::fs::create_dir_all(&layout.root)?;
    std::fs::write(&path, text)?;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&json!({ "config": path }))?);
        return Ok(());
    }
    println!("Wrote {}", path.display());

    report_existing_config_dirs(&layout.home);

    if profiles.is_empty() {
        println!("\nAdd a profile:  cpx profile add work --description 'Company account'");
        println!("                cpx profile add work --provider codex");
    }
    println!("Then:           cpx apply");
    Ok(())
}

/// Point out hand-rolled `~/.claude-*` and `~/.codex-*` directories. They can
/// be adopted in place: a login is keyed to its directory's path, so moving
/// one would mean signing in again, and `cpx adopt` never moves anything.
fn report_existing_config_dirs(home: &Path) {
    let Ok(entries) = std::fs::read_dir(home) else {
        return;
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| {
            Provider::ALL.iter().any(|p| name.starts_with(p.dir_prefix()))
                && name != ".claude-profiles"
        })
        .collect();
    found.sort();
    if found.is_empty() {
        return;
    }

    println!("\nFound existing config directories:");
    for name in &found {
        println!("  ~/{name}");
    }
    println!("They are left alone. `cpx adopt` manages one where it sits, so its login");
    println!("keeps working without signing in again.");
}

fn cmd_apply(dry_run: bool, sync: bool, force: bool, as_json: bool) -> Result<()> {
    let mut session = Session::load()?;
    let options = session.apply_options(sync);
    let plan = plan_apply(&session.config, &session.layout, &session.state, &options)?;

    if as_json {
        if dry_run {
            println!("{}", serde_json::to_string_pretty(&render::plan_json(&plan))?);
            return Ok(());
        }
    } else {
        render::print_plan(&plan);
        if dry_run || plan.is_empty() {
            return Ok(());
        }
        println!();
    }

    let sources = session.config.source_dirs();
    let report = execute(&plan, &mut session.state, &sources, &ExecuteOptions { force })?;
    session.save_state()?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "performed": plan.actions.len(),
                "backups": report.backups,
                "notes": plan.notes,
            }))?
        );
        return Ok(());
    }
    for (from, to) in &report.backups {
        println!("backed up {} -> {}", from.display(), to.display());
    }
    println!("Applied {} action(s).", plan.actions.len());
    Ok(())
}

fn cmd_status(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let options = session.apply_options(false);
    let plan = plan_apply(&session.config, &session.layout, &session.state, &options)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&render::plan_json(&plan))?);
    } else {
        render::print_plan(&plan);
    }
    Ok(())
}

/// One profile as the app and `list --json` see it.
fn profile_json(session: &Session, name: &str) -> serde_json::Value {
    let profile = &session.config.profiles[name];
    let dir = session.config.config_dir(&session.layout, name);
    let status = credentials::status_for(profile.provider, &dir, &session.layout.home);
    json!({
        "name": name,
        "provider": profile.provider,
        "description": profile.description,
        "color": profile.color,
        "adopted": profile.dir.is_some(),
        "directory": dir,
        "applied": dir.is_dir(),
        "authenticated": status.authenticated,
        "account": status.account,
        "organization": status.organization,
        "plan": status.plan,
        "expired": status.expired,
        "source": format!("{:?}", status.source),
        "model": profile.model,
        "command": session.config.command_name(name),
    })
}

fn default_sessions_json(session: &Session) -> Vec<serde_json::Value> {
    cpx_core::default_session::default_sessions(&session.layout, &session.config)
        .iter()
        .map(|d| {
            json!({
                "provider": d.provider,
                "directory": d.dir,
                "account": d.account,
                "signedIn": d.signed_in,
                "isSource": d.is_source,
                "claimedBy": d.claimed_by,
            })
        })
        .collect()
}

fn cmd_list(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let rows: Vec<serde_json::Value> = session
        .config
        .profiles
        .keys()
        .map(|name| profile_json(&session, name))
        .collect();
    let defaults = cpx_core::default_session::default_sessions(&session.layout, &session.config);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profiles": rows,
                "defaultSessions": default_sessions_json(&session),
            }))?
        );
        return Ok(());
    }

    if rows.is_empty() {
        println!("No profiles configured. Add one with `cpx profile add <name>`.");
        print_default_sessions(&defaults);
        return Ok(());
    }

    println!(
        "{:<14} {:<8} {:<24} {:<10} COMMAND",
        "PROFILE", "TOOL", "ACCOUNT", "STATE"
    );
    for row in &rows {
        let account = row["account"].as_str().unwrap_or("—");
        let state = match (row["applied"].as_bool(), row["authenticated"].as_bool()) {
            (Some(false), _) => "not built",
            (_, Some(false)) => "logged out",
            _ => "ready",
        };
        println!(
            "{:<14} {:<8} {:<24} {:<10} {}",
            row["name"].as_str().unwrap_or(""),
            row["provider"].as_str().unwrap_or(""),
            account,
            state,
            row["command"].as_str().unwrap_or(""),
        );
    }

    print_default_sessions(&defaults);
    Ok(())
}

/// Report the directories a plain `claude` and a plain `codex` use. cpx
/// treats them as the sources profiles inherit from rather than as
/// profiles, but each is usually a working account, and leaving them out
/// would show fewer accounts than the machine has.
fn print_default_sessions(defaults: &[cpx_core::default_session::DefaultSession]) {
    let unclaimed: Vec<_> = defaults.iter().filter(|d| d.claimed_by.is_none()).collect();
    if unclaimed.is_empty() {
        return;
    }

    println!();
    println!("Not managed by cpx:");
    for default in unclaimed {
        let who = match (&default.signed_in, &default.account) {
            (true, Some(account)) => account.clone(),
            (true, None) => "signed in".to_string(),
            (false, _) => "not signed in".to_string(),
        };
        println!(
            "  {:<14} {:<8} {:<24} {}",
            default.provider.binary_name(),
            default.provider.as_str(),
            who,
            default.dir.display()
        );
        if default.is_source {
            println!("  This is also the directory your {} profiles inherit from.", default.provider.display_name());
        }
    }
}

fn cmd_show(name: &str, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let profile = session
        .config
        .profiles
        .get(name)
        .with_context(|| format!("no profile named `{name}`"))?;
    let dir = session.config.config_dir(&session.layout, name);
    let status = credentials::status_for(profile.provider, &dir, &session.layout.home);

    let resources: Vec<_> = profile
        .resources
        .iter()
        .map(|(key, spec)| {
            json!({
                "resource": key.config_name(),
                "mode": spec.mode.as_str(),
                "patch": spec.patch,
                "isDirectory": key.is_dir(),
                "supportsMerge": key.supports_merge(),
            })
        })
        .collect();

    if as_json {
        let mut value = profile_json(&session, name);
        let extra = json!({
            "addDirs": profile.add_dirs,
            "env": profile.env,
            "resources": resources,
            "credentials": status,
            "credentialLocation": credentials::location(profile.provider, &dir, &session.layout.home),
            "sourceDir": session.config.source_dir(profile.provider),
        });
        for (key, item) in extra.as_object().unwrap() {
            value[key] = item.clone();
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    println!("{name}  {}", profile.description);
    println!("  tool        {}", profile.provider.display_name());
    println!("  directory   {}", dir.display());
    println!("  command     {}", session.config.command_name(name));
    println!(
        "  login       {}",
        match (&status.authenticated, &status.account) {
            (true, Some(account)) => format!("{account} ({:?})", status.source),
            (true, None) => format!("signed in ({:?})", status.source),
            (false, _) => "not signed in".to_string(),
        }
    );
    if let Some(plan) = &status.plan {
        println!("  plan        {plan}");
    }
    if let Some(model) = &profile.model {
        println!("  model       {model}");
    }
    for dir in &profile.add_dirs {
        println!("  add-dir     {}", dir.display());
    }
    for (key, value) in &profile.env {
        println!("  env         {key}={value}");
    }
    println!("  resources");
    for (key, spec) in &profile.resources {
        let patch = if spec.patch.is_some() { "  (+patch)" } else { "" };
        println!("    {:<15} {}{patch}", key.config_name(), spec.mode.as_str());
    }
    Ok(())
}

fn cmd_doctor(verbose: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let checks = doctor::diagnose(
        &session.config,
        &session.layout,
        &session.state,
        &session.bindings,
        &session.ambient(),
    );

    if as_json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        render::print_checks(&checks, verbose);
        if doctor::worst(&checks) == Severity::Ok {
            println!("Everything checks out.");
        }
    }

    if doctor::worst(&checks) == Severity::Error {
        std::process::exit(1);
    }
    Ok(())
}

fn here(dir: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match dir {
        Some(dir) => dir,
        None => std::env::current_dir().context("could not read the current directory")?,
    };
    dir.canonicalize()
        .with_context(|| format!("{} does not exist", dir.display()))
}

fn cmd_bind(profile: &str, dir: Option<PathBuf>, as_json: bool) -> Result<()> {
    let mut session = Session::load()?;
    let dir = here(dir)?;
    let planned = binding::plan_bind(
        &session.config,
        &session.layout,
        &session.bindings,
        profile,
        &dir,
    )?;

    if !as_json {
        render::print_plan(&planned.plan);
    }
    let sources = session.config.source_dirs();
    let report = execute(
        &planned.plan,
        &mut session.state,
        &sources,
        &ExecuteOptions::default(),
    )?;
    let warnings: Vec<&String> = report
        .performed
        .iter()
        .filter(|line| {
            line.contains("direnv") && line.contains("failed") || line.contains("not installed")
        })
        .collect();

    for binding in planned.bindings {
        session.bindings.upsert(binding);
    }
    session.save_bindings()?;
    session.save_state()?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "bound": dir, "profile": profile, "warnings": warnings }))?
        );
        return Ok(());
    }
    for line in warnings {
        println!("{line}");
    }
    println!("{} now uses profile `{profile}`.", dir.display());
    Ok(())
}

fn cmd_unbind(dir: Option<PathBuf>, provider: Option<Provider>, as_json: bool) -> Result<()> {
    let mut session = Session::load()?;
    let dir = here(dir)?;
    let planned = binding::plan_unbind(
        &session.config,
        &session.layout,
        &session.bindings,
        &dir,
        provider,
    )?;

    if planned.plan.is_empty() && !session.bindings.is_bound(&dir) {
        return done(
            as_json,
            json!({ "unbound": dir, "changed": false }),
            &format!("{} is not bound.", dir.display()),
        );
    }

    let sources = session.config.source_dirs();
    execute(
        &planned.plan,
        &mut session.state,
        &sources,
        &ExecuteOptions::default(),
    )?;
    match provider {
        Some(provider) => {
            session.bindings.remove_provider(&dir, provider);
        }
        None => {
            session.bindings.remove(&dir);
        }
    }
    for binding in planned.remaining {
        session.bindings.upsert(binding);
    }
    session.save_bindings()?;
    session.save_state()?;

    done(
        as_json,
        json!({ "unbound": dir, "changed": true, "provider": provider }),
        &format!("{} is no longer bound.", dir.display()),
    )
}

fn cmd_bindings(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let rows: Vec<_> = session
        .bindings
        .entries
        .iter()
        .map(|entry| {
            let health = binding::health(entry, &session.config);
            json!({
                "path": entry.path,
                "profile": entry.profile,
                "provider": entry.provider,
                "color": session.config.profiles.get(&entry.profile).and_then(|p| p.color.clone()),
                "health": format!("{health:?}"),
                "healthy": health == binding::BindingHealth::Healthy,
            })
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No directories bound. Bind one with `cpx bind <profile>`.");
        return Ok(());
    }
    println!("{:<14} {:<8} {:<16} DIRECTORY", "PROFILE", "TOOL", "HEALTH");
    for row in &rows {
        println!(
            "{:<14} {:<8} {:<16} {}",
            row["profile"].as_str().unwrap_or(""),
            row["provider"].as_str().unwrap_or(""),
            row["health"].as_str().unwrap_or(""),
            row["path"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

fn cmd_which(as_json: bool) -> Result<()> {
    let layout = session::layout()?;
    let bindings = Bindings::load(&layout.bindings_file())?;
    let cwd = std::env::current_dir().unwrap_or_default();

    // The environment wins: it is what the tool will actually see.
    let mut found: Vec<serde_json::Value> = Vec::new();
    for provider in Provider::ALL {
        let (profile, reason) = match std::env::var(provider.profile_env_var()) {
            Ok(name) if !name.is_empty() => (Some(name), "environment"),
            _ => (
                cwd.ancestors()
                    .find_map(|dir| bindings.find(dir, provider))
                    .map(|b| b.profile.clone()),
                "binding",
            ),
        };
        if let Some(profile) = profile {
            found.push(json!({ "provider": provider, "profile": profile, "reason": reason }));
        }
    }

    if as_json {
        // `profile` keeps the pre-Codex shape: the Claude answer, if any.
        let claude = found
            .iter()
            .find(|f| f["provider"] == "claude")
            .cloned()
            .unwrap_or(json!({ "profile": null, "reason": null }));
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile": claude["profile"],
                "reason": claude["reason"],
                "profiles": found,
            }))?
        );
        return Ok(());
    }

    if found.is_empty() {
        println!("no profile here");
    }
    for entry in &found {
        println!(
            "{:<8} {}  (from {})",
            entry["provider"].as_str().unwrap_or(""),
            entry["profile"].as_str().unwrap_or(""),
            entry["reason"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

fn cmd_run(profile: &str, args: &[String]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let session = Session::load()?;
    if !session.config.profiles.contains_key(profile) {
        bail!("no profile named `{profile}`");
    }
    let wrapper = session.config.wrapper_path(profile);
    if !wrapper.exists() {
        bail!(
            "{} does not exist yet. Run `cpx apply` first.",
            wrapper.display()
        );
    }
    // exec, so signals and the exit status belong to the tool, not to cpx.
    Err(std::process::Command::new(&wrapper).args(args).exec().into())
}

fn cmd_skills_list(profile: &str, all: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let inventory = cpx_core::skills::inventory(&session.config, &session.layout, profile)?;

    if as_json {
        println!("{}", serde_json::to_string_pretty(&inventory)?);
        return Ok(());
    }

    if inventory.own.is_empty() {
        println!("{profile} has no skills of its own.");
    } else {
        println!("Skills in this profile:");
        for skill in &inventory.own {
            let mark = if skill.enabled { " " } else { "off" };
            let description = skill.description.clone().unwrap_or_default();
            println!("  {mark:<4} {:<28} {}", skill.name, clip(&description, 48));
        }
    }

    if inventory.shared {
        println!();
        println!("This profile shares its skills directory with the others, so turning one");
        println!("off here turns it off everywhere. Give it its own copy first if that");
        println!("is not what you want.");
    }

    let with_skills: Vec<_> = inventory.plugins.iter().filter(|p| p.skills > 0).collect();
    if !with_skills.is_empty() {
        let total: usize = with_skills.iter().map(|p| p.skills).sum();
        println!();
        println!("From plugins ({total} skills):");
        for plugin in with_skills {
            let mark = if plugin.enabled { " " } else { "off" };
            let noun = if plugin.skills == 1 { "skill" } else { "skills" };
            println!("  {mark:<4} {:<28} {} {noun}", plugin.key, plugin.skills);
            if all {
                for name in &plugin.names {
                    println!("       {name}");
                }
            }
        }
        if !all {
            println!();
            println!("`cpx skills list {profile} --all` lists what each plugin provides.");
        }
    }
    Ok(())
}

fn cmd_skills_set(profile: &str, skill: &str, enabled: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let moved =
        cpx_core::skills::set_enabled(&session.config, &session.layout, profile, skill, enabled)?;
    done(
        as_json,
        json!({ "skill": skill, "enabled": enabled, "path": moved }),
        &if enabled {
            format!("{skill} is on again.")
        } else {
            format!("{skill} is off. It is still here, at {}", moved.display())
        },
    )
}

fn cmd_skills_remove(profile: &str, skill: &str, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let moved = cpx_core::skills::remove(&session.config, &session.layout, profile, skill)?;
    done(
        as_json,
        json!({ "skill": skill, "keptAt": moved }),
        &format!(
            "Removed {skill} from {profile}.\nKept at {} — delete it yourself when you are sure.",
            moved.display()
        ),
    )
}

fn cmd_skills_plugin(profile: &str, key: &str, enabled: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let text = std::fs::read_to_string(session.layout.config_file())?;
    let change = cpx_core::skills::set_plugin_enabled(
        &session.config,
        &session.layout,
        profile,
        key,
        enabled,
        &text,
    )?;
    if let Some(config_text) = &change.config_text {
        std::fs::write(session.layout.config_file(), config_text)?;
    }
    let mut text = format!(
        "{key} is {} for {profile}.",
        if enabled { "on" } else { "off" }
    );
    if change.needs_apply {
        text.push_str("\nRun `cpx apply` to regenerate the profile's settings.");
    }
    done(
        as_json,
        json!({ "plugin": key, "enabled": enabled, "needsApply": change.needs_apply }),
        &text,
    )
}

/// Resolve the profile/--base pair into one target.
fn statusline_target(profile: Option<&str>, base: bool) -> Result<cpx_core::statusline::Target> {
    use cpx_core::statusline::Target;
    match (profile, base) {
        (Some(_), true) => bail!("give a profile or --base, not both"),
        (Some(name), false) => Ok(Target::Profile(name.to_string())),
        (None, true) => Ok(Target::Base),
        (None, false) => bail!("which statusline? name a profile, or pass --base"),
    }
}

/// Shorten prose, keeping the start. `brief` keeps the tail, which is right
/// for a path and wrong for a sentence.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let head: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// Shorten a command for display without hiding which script it runs.
fn brief(command: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = if home.is_empty() {
        command.to_string()
    } else {
        command.replace(&home, "~")
    };
    if shown.chars().count() > 54 {
        let tail: String = shown.chars().skip(shown.chars().count() - 51).collect();
        format!("...{tail}")
    } else {
        shown
    }
}

/// What a statusline target looks like right now, for `show` and the app.
fn statusline_row(
    session: &Session,
    target: &cpx_core::statusline::Target,
    label: &str,
) -> Result<serde_json::Value> {
    use cpx_core::statusline::{command_in, SettingsWrite, Target};
    let plan = cpx_core::statusline::plan_install(&session.config, &session.layout, target, None)?;
    let settings = match target {
        Target::Base => session.config.source_dir(Provider::Claude).join("settings.json"),
        Target::Profile(name) => session.config.config_dir(&session.layout, name).join("settings.json"),
    };
    Ok(json!({
        "target": label,
        "badge": plan.replacing,
        "command": command_in(&settings)?,
        "delegate": plan.delegate.as_ref().map(|d| d.command.clone()),
        "script": plan.script_path,
        "needsApply": matches!(plan.write, SettingsWrite::ConfigPatch { .. }),
    }))
}

fn cmd_statusline_show(as_json: bool) -> Result<()> {
    use cpx_core::statusline::Target;

    let session = Session::load()?;
    let mut rows = Vec::new();
    for (name, profile) in &session.config.profiles {
        if profile.provider != Provider::Claude {
            continue;
        }
        rows.push(statusline_row(&session, &Target::Profile(name.clone()), name)?);
    }
    rows.push(statusline_row(&session, &Target::Base, "(base)")?);

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("{:<14} {:<6} STATUSLINE", "TARGET", "BADGE");
    for row in &rows {
        let command = row["command"].as_str().map(brief).unwrap_or_else(|| "—".into());
        let badge = if row["badge"].as_bool() == Some(true) { "yes" } else { "no" };
        println!("{:<14} {:<6} {}", row["target"].as_str().unwrap_or(""), badge, command);
    }

    let wrapped: Vec<&serde_json::Value> = rows.iter().filter(|r| r["badge"] == true).collect();
    if wrapped.is_empty() {
        println!();
        println!("Add a badge with `cpx statusline set <profile>`. Whatever statusline is");
        println!("already configured keeps working — the badge goes in front of it.");
    } else {
        println!();
        for row in wrapped {
            if let Some(delegate) = row["delegate"].as_str() {
                println!(
                    "{}: badge, then {}",
                    row["target"].as_str().unwrap_or(""),
                    brief(delegate)
                );
            }
        }
    }
    Ok(())
}

fn cmd_statusline_set(
    profile: Option<&str>,
    base: bool,
    label: Option<&str>,
    refresh: Option<u64>,
    as_json: bool,
) -> Result<()> {
    let session = Session::load()?;
    let target = statusline_target(profile, base)?;

    let plan =
        cpx_core::statusline::plan_install(&session.config, &session.layout, &target, label)?;
    let text = std::fs::read_to_string(session.layout.config_file())?;
    let applied = cpx_core::statusline::install(&plan, &text, refresh)?;

    if let Some(config_text) = &applied.config_text {
        std::fs::write(session.layout.config_file(), config_text)?;
    }

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "script": plan.script_path,
                "delegate": plan.delegate.as_ref().map(|d| d.command.clone()),
                "backup": applied.backup,
                "needsApply": applied.config_text.is_some(),
            }))?
        );
        return Ok(());
    }
    println!("Statusline badge installed.");
    println!("  script   {}", plan.script_path.display());
    match &plan.delegate {
        Some(delegate) => println!("  then     {}", brief(&delegate.command)),
        None => println!("  then     nothing else — the badge is the whole line"),
    }
    if let Some(backup) = &applied.backup {
        println!("  backup   {}", backup.display());
    }
    if applied.config_text.is_some() {
        println!("  recorded in config.toml as a settings patch; run `cpx apply`");
    }
    Ok(())
}

fn cmd_statusline_clear(profile: Option<&str>, base: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let target = statusline_target(profile, base)?;

    let plan = cpx_core::statusline::plan_install(&session.config, &session.layout, &target, None)?;
    if !plan.replacing {
        return done(
            as_json,
            json!({ "changed": false }),
            "No cpx badge is installed there.",
        );
    }

    let text = std::fs::read_to_string(session.layout.config_file())?;
    let applied = cpx_core::statusline::remove(&plan, &text)?;
    if let Some(config_text) = &applied.config_text {
        std::fs::write(session.layout.config_file(), config_text)?;
    }

    let mut text = match &plan.delegate {
        Some(delegate) => format!("Badge removed; restored {}", brief(&delegate.command)),
        None => "Badge removed; no statusline is configured now.".to_string(),
    };
    if applied.config_text.is_some() {
        text.push_str("\nRun `cpx apply` to regenerate the profile's settings.");
    }
    done(
        as_json,
        json!({ "changed": true, "needsApply": applied.config_text.is_some() }),
        &text,
    )
}

/// Show the script behind a statusline, save a new version of it, or fork a
/// shared one into the profile.
fn cmd_statusline_script(
    profile: Option<&str>,
    base: bool,
    save: Option<PathBuf>,
    fork: bool,
    as_json: bool,
) -> Result<()> {
    use cpx_core::statusline;

    let session = Session::load()?;
    let target = statusline_target(profile, base)?;

    if fork {
        let plan = statusline::plan_install(&session.config, &session.layout, &target, None)?;
        let had_badge = plan.replacing;

        let fork = statusline::fork_script(&session.config, &session.layout, &target)?
            .context("this script is already yours to edit")?;

        // Take the badge off first so the delegate it records is rewritten,
        // then point the statusline at the copy, then put the badge back.
        let mut text = std::fs::read_to_string(session.layout.config_file())?;
        if had_badge {
            let applied = statusline::remove(&plan, &text)?;
            if let Some(updated) = applied.config_text {
                std::fs::write(session.layout.config_file(), &updated)?;
                text = updated;
            }
        }
        let session = Session::load()?;
        let applied =
            statusline::set_command(&session.config, &session.layout, &target, &fork.command, &text)?;
        if let Some(updated) = applied.config_text {
            std::fs::write(session.layout.config_file(), updated)?;
        }
        if had_badge {
            cmd_statusline_set(profile, base, None, None, true).map(|_| ())?;
        }
        return done(
            as_json,
            json!({ "forked": fork.path, "command": fork.command }),
            &format!("Copied the script to {}; the statusline now runs the copy.", fork.path.display()),
        );
    }

    let found = statusline::script_of(&session.config, &session.layout, &target)?;

    if let Some(source) = save {
        let script = found.context("there is no script behind this statusline to edit")?;
        let contents = if source == Path::new("-") {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            text
        } else {
            std::fs::read_to_string(&source)
                .with_context(|| format!("could not read {}", source.display()))?
        };
        let backup = statusline::save_script(&script.path, &contents)?;
        return done(
            as_json,
            json!({ "saved": script.path, "backup": backup }),
            &match backup {
                Some(backup) => format!("Saved {}; previous version kept at {}", script.path.display(), backup.display()),
                None => format!("Saved {}", script.path.display()),
            },
        );
    }

    if as_json {
        let value = found.map(|s| {
            json!({
                "path": s.path,
                "contents": s.contents,
                "owned": s.owned,
                "managedBy": s.managed_by,
            })
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    match found {
        None => println!("There is no script file behind this statusline."),
        Some(script) => {
            println!("# {}", script.path.display());
            if let Some(by) = &script.managed_by {
                println!("# Installed by {by}; edits here may be overwritten. Use --fork first.");
            }
            print!("{}", script.contents);
        }
    }
    Ok(())
}

fn cmd_adopt(dir: Option<PathBuf>, name: Option<&str>, as_json: bool) -> Result<()> {
    use cpx_core::adopt;

    let session = Session::load()?;

    let Some(dir) = dir else {
        let found = adopt::candidates(&session.layout.home, &session.config, &session.layout.root);
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!(found
                    .iter()
                    .map(|a| json!({
                        "name": a.name,
                        "provider": a.provider,
                        "dir": a.dir,
                        "found": a.found,
                        "taken": session.config.profiles.contains_key(&a.name),
                    }))
                    .collect::<Vec<_>>()))?
            );
            return Ok(());
        }
        if found.is_empty() {
            println!("Nothing to adopt: no Claude or Codex directories found outside the sources.");
            return Ok(());
        }
        println!("These directories can be managed where they are:\n");
        for adoption in &found {
            let known = session.config.profiles.contains_key(&adoption.name);
            println!(
                "  {:<12} {:<8} {}{}",
                adoption.name,
                adoption.provider.as_str(),
                adoption.dir.display(),
                if known { "   (already a profile)" } else { "" }
            );
            println!("                        keeps: {}", adoption.found.join(", "));
        }
        println!("\nAdopt one with:  cpx adopt {}", found[0].dir.display());
        return Ok(());
    };

    let adoption = adopt::inspect(&dir, &session.config, name)?;
    let text = std::fs::read_to_string(session.layout.config_file())?;
    let edited = cpx_core::config_edit::add_adopted_profile(&text, &adoption)?;
    std::fs::write(session.layout.config_file(), edited)?;

    // The login is keyed to the directory path, which has not moved.
    let status = credentials::status_for(adoption.provider, &adoption.dir, &session.layout.home);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "adopted": adoption.dir,
                "name": adoption.name,
                "provider": adoption.provider,
                "keeps": adoption.found,
                "signedIn": status.authenticated,
                "account": status.account,
            }))?
        );
        return Ok(());
    }

    println!(
        "Adopted {} as {} profile `{}`.",
        adoption.dir.display(),
        adoption.provider.display_name(),
        adoption.name
    );
    println!("Left exactly as it is: {}", adoption.found.join(", "));
    match (status.authenticated, status.account.as_deref()) {
        (true, Some(account)) => println!("Still signed in as {account} — no need to log in again."),
        (true, None) => println!("Still signed in — no need to log in again."),
        (false, _) => println!("No login found for this directory yet."),
    }

    println!("\nRun `cpx apply` to add its command. Nothing inside the directory will change;");
    println!("check with `cpx apply --dry-run` first if you want to see it.");
    Ok(())
}

fn cmd_clone(from: &str, to: &str, as_json: bool) -> Result<()> {
    edit_config(|text| Ok(config_edit::clone_profile(text, from, to)?))?;
    let session = Session::load()?;
    let command = session.config.command_name(to);
    let login = match session.config.provider_of(to)? {
        Provider::Claude => "auth login",
        Provider::Codex => "login",
    };
    done(
        as_json,
        json!({ "cloned": from, "to": to }),
        &format!("Cloned `{from}` to `{to}`. Credentials are not copied.\nRun `cpx apply`, then `{command} {login}`."),
    )
}

fn cmd_paths(as_json: bool) -> Result<()> {
    let layout = session::layout()?;
    let config = layout.config_file();
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "home": layout.home,
                "root": layout.root,
                "config": config,
                "initialised": config.is_file(),
                "bindings": layout.bindings_file(),
                "state": layout.state_file(),
            }))?
        );
        return Ok(());
    }
    println!("config    {}{}", config.display(), if config.is_file() { "" } else { "  (not created yet: run `cpx init`)" });
    println!("root      {}", layout.root.display());
    println!("bindings  {}", layout.bindings_file().display());
    println!("state     {}", layout.state_file().display());
    Ok(())
}

/// The accounts a `usage` or `cost` command is about: everything, or the
/// ones named. `@claude` and `@codex` are the tools' own default sessions.
fn usage_targets(session: &Session, names: &[String]) -> Result<Vec<cpx_core::usage::Target>> {
    use cpx_core::usage::Target;
    if names.is_empty() {
        return Ok(Target::all(&session.config, &session.layout));
    }
    names
        .iter()
        .map(|name| match name.as_str() {
            "@claude" => Ok(Target::default_session(&session.layout, Provider::Claude)),
            "@codex" => Ok(Target::default_session(&session.layout, Provider::Codex)),
            _ => Target::for_profile(&session.config, &session.layout, name)
                .with_context(|| format!("no profile named `{name}` (use @claude or @codex for a default session)")),
        })
        .collect()
}

fn account_label(target: &cpx_core::usage::Target) -> String {
    match &target.profile {
        Some(name) => name.clone(),
        None => format!("@{}", target.provider.as_str()),
    }
}

/// Minutes and hours until an epoch second, for people.
fn until(epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let seconds = epoch - now;
    if seconds <= 0 {
        return "now".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("{hours}h {}m", minutes % 60);
    }
    format!("{}d {}h", hours / 24, hours % 24)
}

fn cmd_usage(names: &[String], prompt: bool, as_json: bool) -> Result<()> {
    use cpx_core::usage::{http::UreqHttp, Fetcher, SystemCredentialReader};

    let session = Session::load()?;
    let targets = usage_targets(&session, names)?;
    let http = UreqHttp::default();
    let fetcher = Fetcher {
        http: &http,
        claude_credentials: &SystemCredentialReader,
        codex_binary: cpx_core::discovery::resolve_binary(
            &std::env::var("PATH").unwrap_or_default(),
            &session.layout,
            Provider::Codex,
        ),
        home: session.layout.home.clone(),
        keychain_prompt: prompt,
    };

    let mut rows = Vec::new();
    for target in &targets {
        let result = fetcher.fetch(target);
        rows.push((target, result));
    }

    if as_json {
        let accounts: Vec<serde_json::Value> = rows
            .iter()
            .map(|(target, result)| {
                json!({
                    "profile": target.profile,
                    "provider": target.provider,
                    "directory": target.dir,
                    "command": target.command,
                    "usage": result.as_ref().ok(),
                    "error": result.as_ref().err(),
                    "message": result.as_ref().err().map(|e| e.to_string()),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json!({ "accounts": accounts }))?);
        return Ok(());
    }

    for (target, result) in &rows {
        let label = account_label(target);
        match result {
            Err(error) => println!("{label:<14} {:<8} {error}", target.provider.as_str()),
            Ok(snapshot) => {
                let who = snapshot.account.clone().unwrap_or_default();
                let plan = snapshot.plan.clone().map(|p| format!(" ({p})")).unwrap_or_default();
                println!("{label:<14} {:<8} {who}{plan}", target.provider.as_str());
                for window in &snapshot.windows {
                    let reset = window
                        .resets_at_epoch
                        .map(|at| format!("resets in {}", until(at)))
                        .unwrap_or_default();
                    println!(
                        "  {:<22} {:>5.1}% used   {reset}",
                        window.title,
                        window.used_percent
                    );
                }
                if let Some(extra) = &snapshot.extra_usage {
                    if extra.enabled {
                        println!(
                            "  {:<22} ${:.2} of ${:.2}",
                            "Extra usage",
                            extra.used_usd.unwrap_or(0.0),
                            extra.limit_usd.unwrap_or(0.0)
                        );
                    }
                }
                if let Some(credits) = &snapshot.credits {
                    if credits.unlimited {
                        println!("  {:<22} unlimited", "Credits");
                    } else if let Some(balance) = credits.balance {
                        println!("  {:<22} {balance:.2}", "Credits");
                    }
                }
            }
        }
    }
    Ok(())
}

fn cmd_cost(names: &[String], days: u32, no_cache: bool, as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let targets = usage_targets(&session, names)?;

    let reports: Vec<(&cpx_core::usage::Target, cpx_core::cost::CostReport)> = targets
        .iter()
        .map(|target| {
            let cache = cpx_core::cost::cache_path(&session.layout.root, &target.dir);
            let report = cpx_core::cost::scan(
                target.provider,
                &target.dir,
                if no_cache { None } else { Some(cache.as_path()) },
                days,
            );
            (target, report)
        })
        .collect();

    if as_json {
        let accounts: Vec<serde_json::Value> = reports
            .iter()
            .map(|(target, report)| {
                json!({
                    "profile": target.profile,
                    "provider": target.provider,
                    "directory": target.dir,
                    "cost": report,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json!({ "accounts": accounts, "days": days }))?);
        return Ok(());
    }

    println!("{:<14} {:<8} {:>10} {:>10} {:>10}", "ACCOUNT", "TOOL", "TODAY", "7 DAYS", "30 DAYS");
    for (target, report) in &reports {
        let money = |spend: &cpx_core::cost::Spend| {
            format!("${:.2}{}", spend.cost_usd, if spend.partly_unpriced { "+" } else { "" })
        };
        println!(
            "{:<14} {:<8} {:>10} {:>10} {:>10}",
            account_label(target),
            target.provider.as_str(),
            money(&report.today),
            money(&report.last_7_days),
            money(&report.last_30_days),
        );
        if !report.unpriced_models.is_empty() {
            println!("               no price known for: {}", report.unpriced_models.join(", "));
        }
    }
    println!();
    println!("Estimates from local transcripts at list prices; a subscription is not billed per token.");
    Ok(())
}
