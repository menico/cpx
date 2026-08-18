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
