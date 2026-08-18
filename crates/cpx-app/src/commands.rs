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
