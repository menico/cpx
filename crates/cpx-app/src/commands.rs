//! Tauri commands. Each one loads the installation fresh, so a config edited
//! in an editor shows up without restarting the app.

use crate::view::*;
use cpx_core::binding;
use cpx_core::config_edit;
use cpx_core::doctor;
use cpx_core::execute::{execute, ExecuteOptions};
use cpx_core::install::{layout_from_env, Install};
use cpx_core::materialize::{plan_apply, ApplyOptions};
use std::path::PathBuf;

/// Errors reach the UI as text, because that is what the UI shows. The
/// `{:#}`-style chain keeps the cause rather than just the outer message.
type Answer<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn install() -> Answer<Install> {
    Install::from_env().map_err(err)
}

/// Reload after a config write, so the next step sees the new state.
fn install_fresh() -> Answer<Install> {
    Install::from_env().map_err(err)
}

#[tauri::command]
pub fn is_initialised() -> bool {
    layout_from_env()
        .map(|layout| Install::is_initialised(&layout))
        .unwrap_or(false)
}

#[tauri::command]
pub fn initialise(profiles: Vec<String>) -> Answer<()> {
    let layout = layout_from_env().map_err(err)?;
    let path = layout.config_file();
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    let seeds: Vec<(String, String)> = profiles.into_iter().map(|n| (n, String::new())).collect();
    let text = config_edit::starter_config(&seeds);
    cpx_core::config::Config::parse(&text, &layout.home).map_err(err)?;

    std::fs::create_dir_all(&layout.root).map_err(err)?;
    std::fs::write(&path, text).map_err(err)
}

#[tauri::command]
pub fn profiles() -> Answer<Vec<ProfileRow>> {
    let install = install()?;
    Ok(install
        .config
        .profiles
        .keys()
        .map(|name| profile_row(&install.config, &install.layout, name))
        .collect())
}

#[tauri::command]
pub fn profile(name: String) -> Answer<ProfileDetail> {
    let install = install()?;
    if !install.config.profiles.contains_key(&name) {
        return Err(format!("no profile named `{name}`"));
    }
    Ok(profile_detail(&install.config, &install.layout, &name))
}

#[tauri::command]
pub fn plan() -> Answer<PlanView> {
    let install = install()?;
    let options = ApplyOptions {
        sync: false,
        claude_binary: install.claude_binary(),
    };
    let plan = plan_apply(&install.config, &install.layout, &install.state, &options).map_err(err)?;
    Ok(plan_view(&plan))
}

#[tauri::command]
pub fn apply(force: bool, sync: bool) -> Answer<ApplyView> {
    let mut install = install()?;
    let options = ApplyOptions {
        sync,
        claude_binary: install.claude_binary(),
    };
    let plan = plan_apply(&install.config, &install.layout, &install.state, &options).map_err(err)?;
    let report = execute(
        &plan,
        &mut install.state,
        &install.config.source_dir,
        &ExecuteOptions { force },
    )
    .map_err(err)?;
    install.save_state().map_err(err)?;

    Ok(ApplyView {
        performed: plan.actions.len(),
        backups: report
            .backups
            .iter()
            .map(|(from, to)| (from.display().to_string(), to.display().to_string()))
            .collect(),
    })
}

#[tauri::command]
pub fn bindings() -> Answer<Vec<BindingRow>> {
    let install = install()?;
    Ok(binding_rows(&install.bindings, &install.config))
}

#[tauri::command]
pub fn bind(profile: String, dir: String) -> Answer<()> {
    let mut install = install()?;
    let dir = PathBuf::from(dir).canonicalize().map_err(err)?;
    let planned =
        binding::plan_bind(&install.config, &install.layout, &profile, &dir).map_err(err)?;

    execute(
        &planned.plan,
        &mut install.state,
        &install.config.source_dir,
        &ExecuteOptions::default(),
    )
    .map_err(err)?;

    install.bindings.upsert(planned.binding);
    install.save_bindings().map_err(err)?;
    install.save_state().map_err(err)
}

#[tauri::command]
pub fn unbind(dir: String) -> Answer<()> {
    let mut install = install()?;
    let dir = PathBuf::from(dir);
    let plan = binding::plan_unbind(&dir).map_err(err)?;
    execute(
        &plan,
        &mut install.state,
        &install.config.source_dir,
        &ExecuteOptions::default(),
    )
    .map_err(err)?;
    install.bindings.remove(&dir);
    install.save_bindings().map_err(err)
}

#[tauri::command]
pub fn checks() -> Answer<Vec<CheckView>> {
    let install = install()?;
    Ok(check_views(&doctor::diagnose(
        &install.config,
        &install.layout,
        &install.state,
        &install.bindings,
        &install.ambient(),
    )))
}

/// Every config mutation funnels through here, so validation and writing
/// happen in exactly one place.
fn edit(f: impl FnOnce(&str) -> Result<String, config_edit::EditError>) -> Answer<()> {
    let install = install()?;
    let text = install.config_text().map_err(err)?;
    let edited = f(&text).map_err(err)?;
    install.write_config(&edited).map_err(err)
}

#[tauri::command]
pub fn add_profile(name: String, description: String) -> Answer<()> {
    edit(|text| config_edit::add_profile(text, &name, &description))
}

#[tauri::command]
pub fn remove_profile(name: String) -> Answer<()> {
    edit(|text| config_edit::remove_profile(text, &name))
}

#[tauri::command]
pub fn clone_profile(from: String, to: String) -> Answer<()> {
    edit(|text| config_edit::clone_profile(text, &from, &to))
}

#[tauri::command]
pub fn set_field(profile: String, field: String, value: Option<String>) -> Answer<()> {
    edit(|text| config_edit::set_profile_field(text, &profile, &field, value.as_deref()))
}

#[tauri::command]
pub fn set_resource(profile: String, resource: String, mode: String) -> Answer<()> {
    edit(|text| config_edit::set_resource_mode(text, &profile, &resource, &mode))
}

/// Resolve an optional profile name into a statusline target.
fn statusline_target(profile: Option<String>) -> cpx_core::statusline::Target {
    match profile {
        Some(name) => cpx_core::statusline::Target::Profile(name),
        None => cpx_core::statusline::Target::Base,
    }
}

/// Whether a badge is installed, and what it sits in front of.
#[tauri::command]
pub fn statusline(profile: Option<String>) -> Answer<StatuslineView> {
    let install = install()?;
    let target = statusline_target(profile);
    let plan = cpx_core::statusline::plan_install(&install.config, &install.layout, &target, None)
        .map_err(err)?;
    Ok(StatuslineView {
        badge: plan.replacing,
        delegate: plan.delegate.as_ref().map(|d| d.command.clone()),
        script: plan.script_path.display().to_string(),
        needs_apply: matches!(
            plan.write,
            cpx_core::statusline::SettingsWrite::ConfigPatch { .. }
        ),
    })
}

/// Put a badge in front of whatever statusline is configured.
#[tauri::command]
pub fn set_statusline(
    profile: Option<String>,
    label: Option<String>,
    refresh: Option<u64>,
) -> Answer<()> {
    let install = install()?;
    let target = statusline_target(profile);
    let plan = cpx_core::statusline::plan_install(
        &install.config,
        &install.layout,
        &target,
        label.as_deref(),
    )
    .map_err(err)?;

    let text = install.config_text().map_err(err)?;
    let applied = cpx_core::statusline::install(&plan, &text, refresh).map_err(err)?;
    if let Some(config_text) = applied.config_text {
        install.write_config(&config_text).map_err(err)?;
    }
    Ok(())
}

/// Remove the badge, restoring what was there before.
#[tauri::command]
pub fn clear_statusline(profile: Option<String>) -> Answer<()> {
    let install = install()?;
    let target = statusline_target(profile);
    let plan = cpx_core::statusline::plan_install(&install.config, &install.layout, &target, None)
        .map_err(err)?;
    if !plan.replacing {
        return Ok(());
    }

    let text = install.config_text().map_err(err)?;
    let applied = cpx_core::statusline::remove(&plan, &text).map_err(err)?;
    if let Some(config_text) = applied.config_text {
        install.write_config(&config_text).map_err(err)?;
    }
    Ok(())
}

/// The script behind a target's statusline, for editing.
#[tauri::command]
pub fn statusline_script(profile: Option<String>) -> Answer<Option<ScriptView>> {
    let install = install()?;
    let target = statusline_target(profile);
    let found = cpx_core::statusline::script_of(&install.config, &install.layout, &target)
        .map_err(err)?;
    Ok(found.map(|script| ScriptView {
        path: script.path.display().to_string(),
        contents: script.contents,
        owned: script.owned,
        managed_by: script.managed_by,
    }))
}

/// Save an edited statusline script.
///
/// The path is resolved here rather than taken from the caller, so this can
/// only ever write the file that target's statusline actually runs.
#[tauri::command]
pub fn save_statusline_script(profile: Option<String>, contents: String) -> Answer<()> {
    let install = install()?;
    let target = statusline_target(profile);
    let script = cpx_core::statusline::script_of(&install.config, &install.layout, &target)
        .map_err(err)?
        .ok_or("there is no script behind this statusline to edit")?;
    cpx_core::statusline::save_script(&script.path, &contents).map_err(err)?;
    Ok(())
}

/// Copy a shared script into this profile's own space and run the copy.
#[tauri::command]
pub fn fork_statusline_script(profile: Option<String>) -> Answer<String> {
    let install = install()?;
    let target = statusline_target(profile.clone());

    let plan = cpx_core::statusline::plan_install(&install.config, &install.layout, &target, None)
        .map_err(err)?;
    let had_badge = plan.replacing;

    let fork = cpx_core::statusline::fork_script(&install.config, &install.layout, &target)
        .map_err(err)?
        .ok_or("this script is already yours to edit")?;

    // Take the badge off first so the delegate it records is rewritten, then
    // point the statusline at the copy, then put the badge back in front.
    let mut text = install.config_text().map_err(err)?;
    if had_badge {
        let applied = cpx_core::statusline::remove(&plan, &text).map_err(err)?;
        if let Some(updated) = applied.config_text {
            install.write_config(&updated).map_err(err)?;
            text = updated;
        }
    }

    let install = install_fresh()?;
    let applied = cpx_core::statusline::set_command(
        &install.config,
        &install.layout,
        &target,
        &fork.command,
        &text,
    )
    .map_err(err)?;
    if let Some(updated) = applied.config_text {
        install.write_config(&updated).map_err(err)?;
    }

    if had_badge {
        set_statusline(profile, None, None)?;
    }
    Ok(fork.path.display().to_string())
}

/// The directory a plain `claude` uses. Reported, never managed.
#[tauri::command]
pub fn default_session() -> Answer<DefaultSessionView> {
    let install = install()?;
    Ok(default_session_view(
        &cpx_core::default_session::default_session(&install.layout, &install.config),
    ))
}

/// Directories that could be managed in place, with what each would keep.
#[tauri::command]
pub fn adoption_candidates() -> Answer<Vec<AdoptionRow>> {
    let install = install()?;
    let found = cpx_core::adopt::candidates(
        &install.layout.home,
        &install.config.source_dir,
        &install.layout.root,
    );
    Ok(adoption_rows(&found, &install.config))
}

/// Adopt a directory: register it where it is, changing nothing inside it.
#[tauri::command]
pub fn adopt(dir: String, name: Option<String>) -> Answer<()> {
    let install = install()?;
    let adoption = cpx_core::adopt::inspect(
        std::path::Path::new(&dir),
        &install.config.source_dir,
        name.as_deref(),
    )
    .map_err(err)?;
    edit(|text| config_edit::add_adopted_profile(text, &adoption))
}

#[tauri::command]
pub fn config_path() -> Answer<String> {
    Ok(layout_from_env()
        .map_err(err)?
        .config_file()
        .display()
        .to_string())
}

/// Run a profile's own wrapper in Terminal, so signing in happens where the
/// user can see the browser hand-off and the result.
#[tauri::command]
pub fn auth(profile: String, action: String) -> Answer<()> {
    if !["login", "logout", "status"].contains(&action.as_str()) {
        return Err(format!("unknown auth action `{action}`"));
    }
    let install = install()?;
    let wrapper = install.config.wrapper_path(&profile);
    if !wrapper.exists() {
        return Err(format!(
            "{} does not exist yet — apply your changes first",
            wrapper.display()
        ));
    }
    open_in_terminal(&format!("{} auth {action}", shell_quote(&wrapper.to_string_lossy())))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// AppleScript string literals escape with backslashes, which is a different
/// set of rules from the shell quoting inside the command itself.
fn applescript_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn open_in_terminal(command: &str) -> Answer<()> {
    let script = format!(
        "tell application \"Terminal\"\n  activate\n  do script {}\nend tell",
        applescript_quote(command)
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(err)?;
    if status.success() {
        Ok(())
    } else {
        Err("could not open Terminal".to_string())
    }
}

/// Reveal a path in Finder.
#[tauri::command]
pub fn reveal(path: String) -> Answer<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .status()
        .map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting_neutralises_a_single_quote() {
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
    }

    #[test]
    fn shell_quoting_neutralises_command_substitution() {
        assert_eq!(shell_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn applescript_quoting_escapes_quotes_and_backslashes() {
        assert_eq!(applescript_quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(applescript_quote(r"a\b"), r#""a\\b""#);
    }

    #[test]
    fn a_path_with_a_quote_survives_both_quoting_layers() {
        // A profile directory could contain an apostrophe; the command must
        // still reach Terminal as one argument.
        let quoted = applescript_quote(&format!("{} auth login", shell_quote("/Users/o'brien/bin/claude-work")));
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        assert!(quoted.contains(r"o'"), "{quoted}");
        assert!(!quoted.contains(r#"" auth"#), "quote broke out of the literal: {quoted}");
    }

    #[test]
    fn an_unknown_auth_action_is_refused_before_anything_is_launched() {
        let result = auth("work".into(), "delete-everything".into());
        assert!(result.unwrap_err().contains("unknown auth action"));
    }
}
