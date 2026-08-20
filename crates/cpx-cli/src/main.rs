//! `cpx` — Claude profile manager.
//!
//! This crate is deliberately thin: argument parsing and rendering. Every
//! decision lives in `cpx-core`, so the desktop app can make the same ones.

mod render;
mod session;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use cpx_core::binding::{self, Bindings};
use cpx_core::config_edit;
use cpx_core::credentials;
use cpx_core::doctor::{self, Severity};
use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::materialize::{plan_apply, ApplyOptions};
use serde_json::json;
use session::Session;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "cpx",
    version,
    about = "Run several Claude Code accounts side by side",
    long_about = "cpx gives each Claude account its own config directory, wrapper command, \
                  and per-directory binding, while sharing whatever you want shared."
)]
struct Cli {
    /// Emit machine-readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
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
    Unbind { dir: Option<PathBuf> },
    /// List bound directories.
    Bindings,
    /// Show which profile applies here.
    Which,
    /// Run Claude once under a profile.
    Run {
        profile: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Copy a profile's configuration under a new name, without credentials.
    Clone { from: String, to: String },
    /// Manage an existing Claude config directory in place, without moving it.
    Adopt {
        /// The directory to adopt. Omit to list what could be adopted.
        dir: Option<PathBuf>,
        /// Profile name. Defaults to the directory's own name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Add or remove profiles in the config.
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Show or change which statusline each profile uses.
    #[command(subcommand)]
    Statusline(StatuslineCommand),
    /// List a profile's skills, and turn them on or off.
    #[command(subcommand)]
    Skills(SkillsCommand),
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
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Add a profile to the config.
    Add {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Remove a profile from the config. Its directory is left in place.
    Rm { name: String },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("cpx: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { profiles } => cmd_init(&profiles),
        Command::Apply {
            dry_run,
            sync,
            force,
        } => cmd_apply(dry_run, sync, force, cli.json),
        Command::List => cmd_list(cli.json),
        Command::Show { profile } => cmd_show(&profile, cli.json),
        Command::Status => cmd_status(cli.json),
        Command::Doctor { verbose } => cmd_doctor(verbose, cli.json),
        Command::Bind { profile, dir } => cmd_bind(&profile, dir),
        Command::Unbind { dir } => cmd_unbind(dir),
        Command::Bindings => cmd_bindings(cli.json),
        Command::Which => cmd_which(cli.json),
        Command::Run { profile, args } => cmd_run(&profile, &args),
        Command::Clone { from, to } => cmd_clone(&from, &to),
        Command::Adopt { dir, name } => cmd_adopt(dir, name.as_deref(), cli.json),
        Command::Skills(SkillsCommand::List { profile, all }) => {
            cmd_skills_list(&profile, all, cli.json)
        }
        Command::Skills(SkillsCommand::Enable { profile, skill }) => {
            cmd_skills_set(&profile, &skill, true)
        }
        Command::Skills(SkillsCommand::Disable { profile, skill }) => {
            cmd_skills_set(&profile, &skill, false)
        }
        Command::Skills(SkillsCommand::Remove { profile, skill }) => {
            cmd_skills_remove(&profile, &skill)
        }
        Command::Skills(SkillsCommand::Plugin {
            profile,
            key,
            off,
            on,
        }) => cmd_skills_plugin(&profile, &key, on || !off),
        Command::Statusline(StatuslineCommand::Show) => cmd_statusline_show(cli.json),
        Command::Statusline(StatuslineCommand::Set {
            profile,
            base,
            label,
            refresh,
        }) => cmd_statusline_set(profile.as_deref(), base, label.as_deref(), refresh),
        Command::Statusline(StatuslineCommand::Clear { profile, base }) => {
            cmd_statusline_clear(profile.as_deref(), base)
        }
        Command::Profile(ProfileCommand::Add { name, description }) => {
            edit_config(|text| Ok(config_edit::add_profile(text, &name, &description)?))?;
            println!("Added profile `{name}`. Run `cpx apply` to create it.");
            Ok(())
        }
        Command::Profile(ProfileCommand::Rm { name }) => {
            edit_config(|text| Ok(config_edit::remove_profile(text, &name)?))?;
            println!("Removed profile `{name}` from the config.");
            println!("Its directory is still there; delete it by hand if you want it gone.");
            Ok(())
        }
    }
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

fn cmd_init(profiles: &[String]) -> Result<()> {
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
    println!("Wrote {}", path.display());

    report_existing_config_dirs(&layout.home);

    if profiles.is_empty() {
        println!("\nAdd a profile:  cpx profile add work --description 'Company account'");
    }
    println!("Then:           cpx apply");
    Ok(())
}

/// Point out hand-rolled `~/.claude-*` directories. cpx cannot adopt them in
/// place: the Keychain entry for a login is keyed to the config directory's
/// path, so moving one means logging in again.
fn report_existing_config_dirs(home: &Path) {
    let Ok(entries) = std::fs::read_dir(home) else {
        return;
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with(".claude-") && name != ".claude-profiles")
        .collect();
    found.sort();
    if found.is_empty() {
        return;
    }

    println!("\nFound existing Claude config directories:");
    for name in &found {
        println!("  ~/{name}");
    }
    println!("cpx keeps its profiles under ~/.claude-profiles/. Those directories are");
    println!("left alone — a login is tied to its directory path, so moving one would");
    println!("mean signing in again.");
}

fn cmd_apply(dry_run: bool, sync: bool, force: bool, as_json: bool) -> Result<()> {
    let mut session = Session::load()?;
    let options = ApplyOptions {
        sync,
        claude_binary: session.claude_binary(),
    };
    let plan = plan_apply(&session.config, &session.layout, &session.state, &options)?;

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "actions": plan.actions.iter().map(|a| json!({
                    "description": a.description,
                    "target": a.action.target().display().to_string(),
                    "risk": format!("{:?}", a.risk),
                })).collect::<Vec<_>>(),
                "notes": plan.notes,
                "requiresForce": plan.requires_force(),
            }))?
        );
        if dry_run {
            return Ok(());
        }
    } else {
        render::print_plan(&plan);
        if dry_run {
            return Ok(());
        }
        if plan.is_empty() {
            return Ok(());
        }
        println!();
    }

    let report = execute(
        &plan,
        &mut session.state,
        &session.config.source_dir,
        &ExecuteOptions { force },
    )?;
    session.save_state()?;

    if !as_json {
        for (from, to) in &report.backups {
            println!("backed up {} -> {}", from.display(), to.display());
        }
        println!("Applied {} action(s).", plan.actions.len());
    }
    Ok(())
}

fn cmd_status(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let options = ApplyOptions {
        sync: false,
        claude_binary: session.claude_binary(),
    };
    let plan = plan_apply(&session.config, &session.layout, &session.state, &options)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "pendingActions": plan.actions.len(),
                "requiresForce": plan.requires_force(),
                "notes": plan.notes,
            }))?
        );
    } else {
        render::print_plan(&plan);
    }
    Ok(())
}

fn cmd_list(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let mut rows = Vec::new();

    for (name, profile) in &session.config.profiles {
        let dir = session.config.config_dir(&session.layout, name);
        let status = credentials::status(&dir, &session.layout.home);
        rows.push(json!({
            "name": name,
            "description": profile.description,
            "applied": dir.is_dir(),
            "authenticated": status.authenticated,
            "account": status.account,
            "source": format!("{:?}", status.source),
            "model": profile.model,
            "command": format!("{}{name}", session.config.wrapper_prefix),
        }));
    }

    let default = cpx_core::default_session::default_session(&session.layout, &session.config);

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profiles": rows,
                "defaultSession": {
                    "directory": default.dir,
                    "account": default.account,
                    "signedIn": default.signed_in,
                    "isSource": default.is_source,
                    "claimedBy": default.claimed_by,
                },
            }))?
        );
        return Ok(());
    }

    if rows.is_empty() {
        println!("No profiles configured. Add one with `cpx profile add <name>`.");
        print_default_session(&default);
        return Ok(());
    }

    println!("{:<14} {:<24} {:<10} COMMAND", "PROFILE", "ACCOUNT", "STATE");
    for row in &rows {
        let account = row["account"].as_str().unwrap_or("—");
        let state = match (row["applied"].as_bool(), row["authenticated"].as_bool()) {
            (Some(false), _) => "not built",
            (_, Some(false)) => "logged out",
            _ => "ready",
        };
        println!(
            "{:<14} {:<24} {:<10} {}",
            row["name"].as_str().unwrap_or(""),
            account,
            state,
            row["command"].as_str().unwrap_or(""),
        );
    }

    print_default_session(&default);
    Ok(())
}

/// Report the directory a plain `claude` uses. cpx treats it as the source
/// profiles inherit from rather than as a profile, but it is usually a
/// working account, and leaving it out would show fewer accounts than the
/// machine has.
fn print_default_session(default: &cpx_core::default_session::DefaultSession) {
    // Already listed above under its own name.
    if default.claimed_by.is_some() {
        return;
    }

    println!();
    println!("Not managed by cpx:");
    let who = match (&default.signed_in, &default.account) {
        (true, Some(account)) => account.clone(),
        (true, None) => "signed in".to_string(),
        (false, _) => "not signed in".to_string(),
    };
    println!("  {:<14} {:<24} {}", "claude", who, default.dir.display());
    if default.is_source {
        println!("  This is also the directory your profiles inherit from.");
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
    let status = credentials::status(&dir, &session.layout.home);

    let resources: Vec<_> = profile
        .resources
        .iter()
        .map(|(key, spec)| {
            json!({
                "resource": key.config_name(),
                "mode": spec.mode.as_str(),
                "patch": spec.patch,
            })
        })
        .collect();

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": name,
                "description": profile.description,
                "directory": dir,
                "model": profile.model,
                "addDirs": profile.add_dirs,
                "env": profile.env,
                "resources": resources,
                "credentials": status,
                "keychainService": credentials::keychain_service_for(&dir, &session.layout.home),
            }))?
        );
        return Ok(());
    }

    println!("{name}  {}", profile.description);
    println!("  directory   {}", dir.display());
    println!("  command     {}{name}", session.config.wrapper_prefix);
    println!(
        "  login       {}",
        match (&status.authenticated, &status.account) {
            (true, Some(account)) => format!("{account} ({:?})", status.source),
            (true, None) => format!("signed in ({:?})", status.source),
            (false, _) => "not signed in".to_string(),
        }
    );
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

fn cmd_bind(profile: &str, dir: Option<PathBuf>) -> Result<()> {
    let mut session = Session::load()?;
    let dir = here(dir)?;
    let planned = binding::plan_bind(&session.config, &session.layout, profile, &dir)?;

    render::print_plan(&planned.plan);
    let report = execute(
        &planned.plan,
        &mut session.state,
        &session.config.source_dir,
        &ExecuteOptions::default(),
    )?;
    for line in &report.performed {
        if line.contains("direnv") && line.contains("failed") || line.contains("not installed") {
            println!("{line}");
        }
    }

    session.bindings.upsert(planned.binding);
    session.save_bindings()?;
    session.save_state()?;

    println!("{} now uses profile `{profile}`.", dir.display());
    Ok(())
}

fn cmd_unbind(dir: Option<PathBuf>) -> Result<()> {
    let mut session = Session::load()?;
    let dir = here(dir)?;
    let plan = binding::plan_unbind(&dir)?;

    if plan.is_empty() && session.bindings.get(&dir).is_none() {
        println!("{} is not bound.", dir.display());
        return Ok(());
    }

    execute(
        &plan,
        &mut session.state,
        &session.config.source_dir,
        &ExecuteOptions::default(),
    )?;
    session.bindings.remove(&dir);
    session.save_bindings()?;
    session.save_state()?;

    println!("{} is no longer bound.", dir.display());
    Ok(())
}

fn cmd_bindings(as_json: bool) -> Result<()> {
    let session = Session::load()?;
    let rows: Vec<_> = session
        .bindings
        .entries
        .iter()
        .map(|entry| {
            json!({
                "path": entry.path,
                "profile": entry.profile,
                "health": format!("{:?}", binding::health(entry, &session.config)),
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
    println!("{:<14} {:<16} DIRECTORY", "PROFILE", "HEALTH");
    for row in &rows {
        println!(
            "{:<14} {:<16} {}",
            row["profile"].as_str().unwrap_or(""),
            row["health"].as_str().unwrap_or(""),
            row["path"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

fn cmd_which(as_json: bool) -> Result<()> {
    let layout = session::layout()?;
    let bindings = Bindings::load(&layout.bindings_file())?;

    // The environment wins: it is what Claude will actually see.
    let (profile, reason) = match std::env::var("CLAUDE_PROFILE") {
        Ok(name) if !name.is_empty() => (Some(name), "environment"),
        _ => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let found = cwd
                .ancestors()
                .find_map(|dir| bindings.get(dir))
                .map(|b| b.profile.clone());
            (found, "binding")
        }
    };

    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "profile": profile, "reason": profile.as_ref().map(|_| reason) }))?
        );
        return Ok(());
    }

    match profile {
        Some(name) => println!("{name}  (from {reason})"),
        None => println!("no profile here"),
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
    // exec, so signals and the exit status belong to Claude, not to cpx.
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

fn cmd_skills_set(profile: &str, skill: &str, enabled: bool) -> Result<()> {
    let session = Session::load()?;
    let moved =
        cpx_core::skills::set_enabled(&session.config, &session.layout, profile, skill, enabled)?;
    if enabled {
        println!("{skill} is on again.");
    } else {
        println!("{skill} is off. It is still here, at {}", moved.display());
    }
    Ok(())
}

fn cmd_skills_remove(profile: &str, skill: &str) -> Result<()> {
    let session = Session::load()?;
    let moved = cpx_core::skills::remove(&session.config, &session.layout, profile, skill)?;
    println!("Removed {skill} from {profile}.");
    println!("Kept at {} — delete it yourself when you are sure.", moved.display());
    Ok(())
}

fn cmd_skills_plugin(profile: &str, key: &str, enabled: bool) -> Result<()> {
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
    println!(
        "{key} is {} for {profile}.",
        if enabled { "on" } else { "off" }
    );
    if change.needs_apply {
        println!("Run `cpx apply` to regenerate the profile's settings.");
    }
    Ok(())
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

fn cmd_statusline_show(as_json: bool) -> Result<()> {
    use cpx_core::statusline::{command_in, delegate_of_wrapper, Target};

    let session = Session::load()?;
    let layout = &session.layout;

    let mut rows = Vec::new();
    for name in session.config.profiles.keys() {
        let plan = cpx_core::statusline::plan_install(
            &session.config,
            layout,
            &Target::Profile(name.clone()),
            None,
        )?;
        let configured = command_in(&session.config.config_dir(layout, name).join("settings.json"))?;
        rows.push(json!({
            "target": name,
            "badge": plan.replacing,
            "command": configured,
            "delegate": plan.delegate.as_ref().map(|d| d.command.clone()),
        }));
    }

    let base_settings = session.config.source_dir.join("settings.json");
    let base_plan =
        cpx_core::statusline::plan_install(&session.config, layout, &Target::Base, None)?;
    rows.push(json!({
        "target": "(base)",
        "badge": base_plan.replacing,
        "command": command_in(&base_settings)?,
        "delegate": base_plan.delegate.as_ref().map(|d| d.command.clone()),
    }));

    if as_json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!("{:<14} {:<6} {}", "TARGET", "BADGE", "STATUSLINE");
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
    let _ = delegate_of_wrapper;
    Ok(())
}

fn cmd_statusline_set(
    profile: Option<&str>,
    base: bool,
    label: Option<&str>,
    refresh: Option<u64>,
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

fn cmd_statusline_clear(profile: Option<&str>, base: bool) -> Result<()> {
    let session = Session::load()?;
    let target = statusline_target(profile, base)?;

    let plan = cpx_core::statusline::plan_install(&session.config, &session.layout, &target, None)?;
    if !plan.replacing {
        println!("No cpx badge is installed there.");
        return Ok(());
    }

    let text = std::fs::read_to_string(session.layout.config_file())?;
    let applied = cpx_core::statusline::remove(&plan, &text)?;
    if let Some(config_text) = &applied.config_text {
        std::fs::write(session.layout.config_file(), config_text)?;
    }

    match &plan.delegate {
        Some(delegate) => println!("Badge removed; restored {}", brief(&delegate.command)),
        None => println!("Badge removed; no statusline is configured now."),
    }
    if applied.config_text.is_some() {
        println!("Run `cpx apply` to regenerate the profile's settings.");
    }
    Ok(())
}

fn cmd_adopt(dir: Option<PathBuf>, name: Option<&str>, as_json: bool) -> Result<()> {
    use cpx_core::adopt;

    let session = Session::load()?;
    let source = &session.config.source_dir;

    let Some(dir) = dir else {
        let found = adopt::candidates(&session.layout.home, source, &session.layout.root);
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!(found
                    .iter()
                    .map(|a| json!({ "name": a.name, "dir": a.dir, "found": a.found }))
                    .collect::<Vec<_>>()))?
            );
            return Ok(());
        }
        if found.is_empty() {
            println!("Nothing to adopt: no Claude config directories found outside ~/.claude.");
            return Ok(());
        }
        println!("These directories can be managed where they are:\n");
        for adoption in &found {
            let known = session.config.profiles.contains_key(&adoption.name);
            println!(
                "  {:<12} {}{}",
                adoption.name,
                adoption.dir.display(),
                if known { "   (already a profile)" } else { "" }
            );
            println!("               keeps: {}", adoption.found.join(", "));
        }
        println!("\nAdopt one with:  cpx adopt {}", found[0].dir.display());
        return Ok(());
    };

    let adoption = adopt::inspect(&dir, source, name)?;
    let text = std::fs::read_to_string(session.layout.config_file())?;
    let edited = cpx_core::config_edit::add_adopted_profile(&text, &adoption)?;
    std::fs::write(session.layout.config_file(), edited)?;

    println!("Adopted {} as profile `{}`.", adoption.dir.display(), adoption.name);
    println!("Left exactly as it is: {}", adoption.found.join(", "));

    // The login is keyed to the directory path, which has not moved.
    let status = cpx_core::credentials::status(&adoption.dir, &session.layout.home);
    match (status.authenticated, status.account.as_deref()) {
        (true, Some(account)) => println!("Still signed in as {account} — no need to log in again."),
        (true, None) => println!("Still signed in — no need to log in again."),
        (false, _) => println!("No login found for this directory yet."),
    }

    println!("\nRun `cpx apply` to add its command. Nothing inside the directory will change;");
    println!("check with `cpx apply --dry-run` first if you want to see it.");
    Ok(())
}

fn cmd_clone(from: &str, to: &str) -> Result<()> {
    edit_config(|text| Ok(config_edit::clone_profile(text, from, to)?))?;
    println!("Cloned `{from}` to `{to}`. Credentials are not copied.");
    println!("Run `cpx apply`, then `claude-{to} auth login`.");
    Ok(())
}
