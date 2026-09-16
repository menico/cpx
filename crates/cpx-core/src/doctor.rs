//! Diagnostics.

use crate::binding::{self, Bindings};
use crate::config::{Config, Provider};
use crate::layout::Layout;
use crate::state::State;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Ok,
    /// Works, but something will bite later.
    Warning,
    /// Broken now.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: String,
    pub severity: Severity,
    pub detail: String,
    /// What to do about it, when there is something to do.
    pub remedy: Option<String>,
}

/// The ambient facts diagnostics depend on, passed in rather than read from
/// the process so the checks are testable.
#[derive(Debug, Clone, Default)]
pub struct Ambient {
    /// `PATH`, as the user's shell has it.
    pub path: String,
    /// `CLAUDE_CONFIG_DIR` in the invoking environment, if set.
    pub claude_config_dir: Option<String>,
    /// `CODEX_HOME` in the invoking environment, if set.
    pub codex_home: Option<String>,
    /// Whether `direnv` was found.
    pub direnv_present: bool,
    /// The resolved Claude binary, if one was found.
    pub claude_binary: Option<PathBuf>,
    /// The resolved Codex binary, if one was found.
    pub codex_binary: Option<PathBuf>,
}

impl Ambient {
    fn binary(&self, provider: Provider) -> Option<&PathBuf> {
        match provider {
            Provider::Claude => self.claude_binary.as_ref(),
            Provider::Codex => self.codex_binary.as_ref(),
        }
    }

    fn home_override(&self, provider: Provider) -> Option<&String> {
        match provider {
            Provider::Claude => self.claude_config_dir.as_ref(),
            Provider::Codex => self.codex_home.as_ref(),
        }
    }
}

fn ok(name: impl Into<String>, detail: impl Into<String>) -> Check {
    Check {
        name: name.into(),
        severity: Severity::Ok,
        detail: detail.into(),
        remedy: None,
    }
}

fn finding(
    severity: Severity,
    name: impl Into<String>,
    detail: impl Into<String>,
    remedy: impl Into<String>,
) -> Check {
    Check {
        name: name.into(),
        severity,
        detail: detail.into(),
        remedy: Some(remedy.into()),
    }
}

/// Symlinks under a profile directory whose target no longer resolves.
fn broken_links(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut broken: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            std::fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()) && !p.exists()
        })
        .collect();
    broken.sort();
    broken
}

fn check_environment(config: &Config, ambient: &Ambient, checks: &mut Vec<Check>) {
    for provider in config.providers_in_use() {
        let source = config.source_dir(provider);
        let label = provider.short_name();
        if source.is_dir() {
            checks.push(ok(format!("{label} source directory"), source.display().to_string()));
        } else {
            checks.push(finding(
                Severity::Error,
                format!("{label} source directory"),
                format!("{} does not exist", source.display()),
                format!(
                    "point `source_dir` at your real {label} directory in config.toml"
                ),
            ));
        }

        match ambient.binary(provider) {
            Some(path) => checks.push(ok(format!("{label} binary"), path.display().to_string())),
            None => checks.push(finding(
                Severity::Error,
                format!("{label} binary"),
                format!("no `{}` executable found on PATH", provider.binary_name()),
                format!("install {label}, or put it on PATH; wrappers exec it by absolute path"),
            )),
        }

        let var = provider.home_env_var();
        if let Some(dir) = ambient.home_override(provider) {
            checks.push(finding(
                Severity::Warning,
                var,
                format!("{var} is set to {dir} in this shell, which overrides whichever profile you think you are using"),
                format!("unset {var}, or leave the directory whose .envrc sets it"),
            ));
        } else {
            checks.push(ok(var, "not set in this shell"));
        }
    }

    let on_path = ambient
        .path
        .split(':')
        .any(|entry| std::path::Path::new(entry) == config.bin_dir);
    if on_path {
        checks.push(ok("PATH", format!("{} is on PATH", config.bin_dir.display())));
    } else {
        checks.push(finding(
            Severity::Warning,
            "PATH",
            format!("{} is not on PATH, so the wrappers are not runnable by name", config.bin_dir.display()),
            format!("add {} to PATH in your shell profile", config.bin_dir.display()),
        ));
    }

    if ambient.direnv_present {
        checks.push(ok("direnv", "installed"));
    } else {
        checks.push(finding(
            Severity::Warning,
            "direnv",
            "direnv is not installed, so bound directories will not switch profiles automatically",
            "install direnv and hook it into your shell",
        ));
    }
}

fn check_profiles(config: &Config, layout: &Layout, checks: &mut Vec<Check>) {
    for (name, profile) in &config.profiles {
        let dir = config.config_dir(layout, name);
        if !dir.is_dir() {
            checks.push(finding(
                Severity::Error,
                format!("profile {name}"),
                format!("{} does not exist", dir.display()),
                "run `cpx apply`",
            ));
            continue;
        }
        checks.push(ok(format!("profile {name}"), dir.display().to_string()));

        for link in broken_links(&dir) {
            checks.push(finding(
                Severity::Error,
                format!("profile {name} links"),
                format!("{} points at something that no longer exists", link.display()),
                "run `cpx apply`, or restore the resource under source_dir",
            ));
        }

        let wrapper = config.wrapper_path(name);
        if wrapper.exists() {
            checks.push(ok(format!("profile {name} wrapper"), wrapper.display().to_string()));
        } else {
            checks.push(finding(
                Severity::Error,
                format!("profile {name} wrapper"),
                format!("{} is missing", wrapper.display()),
                "run `cpx apply`",
            ));
        }

        let status = crate::credentials::status_for(profile.provider, &dir, &layout.home);
        if status.authenticated {
            let who = status.account.unwrap_or_else(|| "signed in".to_string());
            checks.push(ok(format!("profile {name} login"), who));
        } else {
            let login = match profile.provider {
                Provider::Claude => "auth login",
                Provider::Codex => "login",
            };
            checks.push(finding(
                Severity::Warning,
                format!("profile {name} login"),
                format!("{name} has no credentials yet"),
                format!("run `{} {login}`", config.command_name(name)),
            ));
        }
    }
}

fn check_bin_dir(config: &Config, state: &State, checks: &mut Vec<Check>) {
    let Ok(entries) = std::fs::read_dir(&config.bin_dir) else {
        return;
    };
    let live: Vec<String> = config.profiles.keys().map(|n| config.command_name(n)).collect();
    let prefixes: Vec<&str> = Provider::ALL.iter().map(|p| config.wrapper_prefix(*p)).collect();

    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
        .filter(|(name, _)| {
            prefixes.iter().any(|p| name.starts_with(p)) && !live.contains(name)
        })
        .collect();
    found.sort();

    for (name, path) in found {
        let ours = state
            .classify(&path)
            .is_ok_and(|o| o == crate::state::Ownership::Ours);
        if ours {
            checks.push(finding(
                Severity::Warning,
                format!("stale wrapper {name}"),
                format!("{} was generated for a profile that no longer exists", path.display()),
                "run `cpx apply` to remove it",
            ));
        } else {
            let bare = prefixes
                .iter()
                .find_map(|p| name.strip_prefix(p))
                .unwrap_or(&name);
            checks.push(finding(
                Severity::Warning,
                format!("foreign wrapper {name}"),
                format!("{} was not generated by cpx and will never be touched", path.display()),
                format!("nothing to do, unless you want a cpx profile named `{bare}` — that name is taken"),
            ));
        }
    }
}

fn check_bindings(config: &Config, bindings: &Bindings, checks: &mut Vec<Check>) {
    for entry in &bindings.entries {
        let name = format!("binding {}", entry.path.display());
        let (severity, detail, remedy) = match binding::health(entry, config) {
            binding::BindingHealth::Healthy => {
                checks.push(ok(name, format!("bound to {}", entry.profile)));
                continue;
            }
            binding::BindingHealth::DirectoryMissing => (
                Severity::Warning,
                format!("{} no longer exists", entry.path.display()),
                "run `cpx unbind` for it, or restore the directory",
            ),
            binding::BindingHealth::ProfileMissing => (
                Severity::Error,
                format!("{} is bound to profile `{}`, which is not in the config", entry.path.display(), entry.profile),
                "re-bind it to a profile that exists, or add that profile back",
            ),
            binding::BindingHealth::BlockAbsent => (
                Severity::Error,
                format!("{}/.envrc no longer contains its cpx block", entry.path.display()),
                "run `cpx bind` again for that directory",
            ),
            binding::BindingHealth::BlockEdited => (
                Severity::Warning,
                format!("the cpx block in {}/.envrc has been edited by hand", entry.path.display()),
                "keep the edit and it will be replaced on the next bind; run `cpx bind` to regenerate",
            ),
            binding::BindingHealth::NotAllowed => (
                Severity::Warning,
                format!("direnv has not been allowed for {}", entry.path.display()),
                "run `direnv allow` in that directory",
            ),
        };
        checks.push(finding(severity, name, detail, remedy));
    }
}

pub fn diagnose(
    config: &Config,
    layout: &Layout,
    state: &State,
    bindings: &Bindings,
    ambient: &Ambient,
) -> Vec<Check> {
    let mut checks = Vec::new();
    check_environment(config, ambient, &mut checks);
    check_profiles(config, layout, &mut checks);
    check_bin_dir(config, state, &mut checks);
    check_bindings(config, bindings, &mut checks);

    // Worst first, so the thing that is actually broken is the thing the
    // user reads. `sort_by` is stable, so related checks stay together.
    checks.sort_by_key(|c| std::cmp::Reverse(c.severity));
    checks
}

/// The worst severity in a report.
pub fn worst(checks: &[Check]) -> Severity {
    checks.iter().map(|c| c.severity).max().unwrap_or(Severity::Ok)
}
