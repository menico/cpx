//! Statusline wrapping.
//!
//! Claude Code runs the `statusLine` command once per refresh, handing it the
//! session as JSON on stdin and using its stdout as the line. A statusline
//! script written for the default config directory therefore has no idea which
//! profile it is running under — a common one reads `~/.claude.json` directly
//! and so reports the default account in every profile.
//!
//! cpx does not edit those scripts: they belong to whoever installed them and
//! would be overwritten on update. Instead it generates a wrapper that prints
//! a profile badge and then delegates, forwarding stdin unchanged.

use crate::config::{Config, ResourceKey, ResourceMode};
use crate::layout::Layout;
use crate::wrapper::sh_quote;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A marker naming what the wrapper delegates to, so installing twice wraps
/// the original rather than nesting wrapper inside wrapper.
const DELEGATE_MARKER: &str = "# cpx-delegate: ";

/// The badge printed ahead of the delegated statusline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub label: String,
    /// `#rrggbb`; rendered as a truecolor escape. `None` prints unstyled.
    pub color: Option<String>,
    pub glyph: String,
    /// Printed between the badge and the delegated output.
    pub separator: String,
}

impl Badge {
    pub fn new(label: impl Into<String>, color: Option<String>) -> Badge {
        Badge {
            label: label.into(),
            color,
            glyph: "●".to_string(),
            separator: " │ ".to_string(),
        }
    }
}

/// The statusline a profile already had, which the wrapper delegates to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegate {
    /// The command as it appears in `settings.json`, run through a shell.
    pub command: String,
}

/// Turn `#rrggbb` into a truecolor foreground escape.
pub fn ansi_color(hex: &str) -> Option<String> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(format!(
        "\x1b[38;2;{};{};{}m",
        (value >> 16) & 0xff,
        (value >> 8) & 0xff,
        value & 0xff
    ))
}

/// The wrapper script: badge, then whatever was there before.
///
/// The label and colour are resolved at run time from `CLAUDE_PROFILE` and
/// `CPX_PROFILE_COLOR`, falling back to the values baked in. That way a single
/// wrapper — including one installed on the base settings, which every merged
/// profile inherits — shows whichever profile is actually running rather than
/// the name it was generated for.
pub fn render_wrapper(badge: &Badge, delegate: Option<&Delegate>) -> String {
    use std::fmt::Write;
    let mut s = String::new();

    writeln!(s, "#!/usr/bin/env bash").unwrap();
    writeln!(s, "{}", crate::state::MARKER).unwrap();
    writeln!(s, "# Statusline badge, generated for profile: {}", badge.label).unwrap();
    writeln!(s, "#").unwrap();
    writeln!(
        s,
        "# Prints a badge for whichever profile is running, then hands the\n         # session on stdin to whatever statusline was configured before.\n         # That script is never modified."
    )
    .unwrap();
    // Deliberately no `set -e`: a failing delegate must not take the badge
    // down with it. The line is more useful with the profile than without.
    writeln!(s, "\n# The session JSON arrives on stdin, and the delegate needs it too.").unwrap();
    writeln!(s, "__cpx_session=\"$(cat)\"").unwrap();

    writeln!(
        s,
        "\n# Whoever is actually running wins over the name baked in here."
    )
    .unwrap();
    // The fallbacks are assigned as single-quoted literals first. Writing them
    // straight into `${VAR:-default}` would leave them subject to expansion,
    // which turns a profile name containing $(...) into a command.
    writeln!(s, "__cpx_fallback_label={}", sh_quote(&badge.label)).unwrap();
    writeln!(
        s,
        "__cpx_fallback_color={}",
        sh_quote(badge.color.as_deref().unwrap_or(""))
    )
    .unwrap();
    writeln!(s, "__cpx_label=\"${{CLAUDE_PROFILE:-$__cpx_fallback_label}}\"").unwrap();
    writeln!(s, "__cpx_color=\"${{CPX_PROFILE_COLOR:-$__cpx_fallback_color}}\"").unwrap();

    writeln!(s, "__cpx_pre=\"\" __cpx_post=\"\"").unwrap();
    writeln!(s, "if [[ \"$__cpx_color\" =~ ^#([0-9a-fA-F]{{6}})$ ]]; then").unwrap();
    writeln!(s, "  __cpx_hex=\"${{BASH_REMATCH[1]}}\"").unwrap();
    writeln!(
        s,
        "  __cpx_pre=$'\\e'\"[38;2;$((16#${{__cpx_hex:0:2}}));$((16#${{__cpx_hex:2:2}}));$((16#${{__cpx_hex:4:2}}))m\""
    )
    .unwrap();
    writeln!(s, "  __cpx_post=$'\\e[0m'").unwrap();
    writeln!(s, "fi\n").unwrap();

    // The label is an argument, never part of the format, so its contents can
    // never be read as a format specifier or as shell syntax.
    writeln!(
        s,
        "printf '%s%s %s%s%s' \"$__cpx_pre\" {} \"$__cpx_label\" \"$__cpx_post\" {}",
        sh_quote(&badge.glyph),
        sh_quote(&badge.separator)
    )
    .unwrap();

    match delegate {
        None => {
            writeln!(s, "printf '\\n'").unwrap();
        }
        Some(delegate) => {
            writeln!(s, "\n# Delegate, keeping the whole thing to one line.").unwrap();
            writeln!(
                s,
                "__cpx_inner=\"$(printf '%s' \"$__cpx_session\" | {} 2>/dev/null || true)\"",
                delegate.command
            )
            .unwrap();
            writeln!(s, "printf '%s\\n' \"${{__cpx_inner//$'\\n'/ }}\"").unwrap();
        }
    }

    s
}


#[derive(Debug, thiserror::Error)]
pub enum StatusLineError {
    #[error("no profile named `{0}`")]
    UnknownProfile(String),

    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not valid JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// Whose statusline to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Profile(String),
    /// The default session's statusline, in `source_dir/settings.json`.
    ///
    /// Materialization is forbidden from writing there, and stays forbidden;
    /// this is a separate, deliberate edit of one key.
    Base,
}

/// How the target's `settings.json` gets pointed at the wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsWrite {
    /// The profile's settings are generated by merging, so the statusline is
    /// recorded as a patch in `config.toml` — a direct edit would be undone by
    /// the next apply.
    ConfigPatch { profile: String },
    /// The file is the profile's own, or the base one; edit it in place.
    File { path: PathBuf },
}

/// Everything needed to install a wrapper, computed before anything is written.
#[derive(Debug, Clone)]
pub struct Installation {
    pub script_path: PathBuf,
    pub script: String,
    pub write: SettingsWrite,
    /// What the wrapper will delegate to, if anything.
    pub delegate: Option<Delegate>,
    /// True when a cpx wrapper is already installed and is being replaced.
    pub replacing: bool,
}

/// Read the `statusLine.command` out of a settings file.
pub fn command_in(path: &Path) -> Result<Option<String>, StatusLineError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StatusLineError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let value: Value = serde_json::from_str(&text).map_err(|source| StatusLineError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(value
        .pointer("/statusLine/command")
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

/// The delegate recorded inside a wrapper we generated earlier.
pub fn delegate_of_wrapper(script: &Path) -> Option<Delegate> {
    let text = std::fs::read_to_string(script).ok()?;
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix(DELEGATE_MARKER))?;
    let command = line.trim();
    (!command.is_empty()).then(|| Delegate {
        command: command.to_string(),
    })
}

/// Whether a configured command is one of our wrappers.
fn is_our_wrapper(command: &str, script_path: &Path) -> bool {
    command.contains(&script_path.to_string_lossy().to_string())
}

/// Work out what installing a badge on `target` would involve.
pub fn plan_install(
    config: &Config,
    layout: &Layout,
    target: &Target,
    badge_label: Option<&str>,
) -> Result<Installation, StatusLineError> {
    let (script_path, settings_path, write, label, color) = match target {
        Target::Base => {
            let settings = config.source_dir.join("settings.json");
            (
                layout.root.join("statusline-base.sh"),
                settings.clone(),
                SettingsWrite::File { path: settings },
                badge_label.unwrap_or("default").to_string(),
                None,
            )
        }
        Target::Profile(name) => {
            let profile = config
                .profiles
                .get(name)
                .ok_or_else(|| StatusLineError::UnknownProfile(name.clone()))?;

            let uses_merge = profile
                .resources
                .get(&ResourceKey::Settings)
                .map(|spec| spec.mode == ResourceMode::Merge)
                .unwrap_or(false);

            let settings = config.config_dir(layout, name).join("settings.json");
            let write = if uses_merge {
                SettingsWrite::ConfigPatch {
                    profile: name.clone(),
                }
            } else {
                SettingsWrite::File {
                    path: settings.clone(),
                }
            };
            (
                config.support_dir(layout, name).join("statusline.sh"),
                settings,
                write,
                badge_label.unwrap_or(name).to_string(),
                profile.color.clone(),
            )
        }
    };

    // What counts as "configured now" depends on where the statusline lives.
    // For a merged profile that is the patch in config.toml — the file itself
    // is regenerated from it and lags until the next apply, so reading the file
    // would report no badge immediately after installing one.
    let configured = match (&write, target) {
        (SettingsWrite::ConfigPatch { .. }, Target::Profile(name)) => {
            let patched = config.profiles[name]
                .resources
                .get(&ResourceKey::Settings)
                .and_then(|spec| spec.patch.as_ref())
                .and_then(|patch| patch.pointer("/statusLine/command"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            match patched {
                Some(command) => Some(command),
                // Nothing of ours yet: the merged file inherits the base one.
                None => command_in(&config.source_dir.join("settings.json"))?,
            }
        }
        _ => command_in(&settings_path)?,
    };

    let (delegate, replacing) = match configured {
        Some(command) if is_our_wrapper(&command, &script_path) => {
            (delegate_of_wrapper(&script_path), true)
        }
        Some(command) => (Some(Delegate { command }), false),
        None => (None, false),
    };

    // A profile whose own settings name no statusline still inherits whatever
    // the base configures, so that is what a first install should wrap.
    let delegate = match (delegate, target) {
        (None, Target::Profile(_)) if !replacing => command_in(
            &config.source_dir.join("settings.json"),
        )?
        .map(|command| Delegate { command }),
        (delegate, _) => delegate,
    };

    Ok(Installation {
        script: render_wrapper_with_delegate_record(
            &Badge::new(label, color),
            delegate.as_ref(),
        ),
        script_path,
        write,
        delegate,
        replacing,
    })
}

/// The wrapper, plus a machine-readable record of what it delegates to.
pub fn render_wrapper_with_delegate_record(
    badge: &Badge,
    delegate: Option<&Delegate>,
) -> String {
    let mut script = render_wrapper(badge, delegate);
    let record = format!(
        "{DELEGATE_MARKER}{}\n",
        delegate.map(|d| d.command.as_str()).unwrap_or("")
    );
    // Placed after the shebang so it survives being read back.
    let insert_at = script.find('\n').map(|i| i + 1).unwrap_or(0);
    script.insert_str(insert_at, &record);
    script
}

/// The command to put in `settings.json` for a script at `path`.
pub fn command_for(script_path: &Path) -> String {
    format!("bash {}", sh_quote(&script_path.to_string_lossy()))
}

/// The outcome of installing or removing a statusline.
#[derive(Debug, Clone)]
pub struct Applied {
    pub script_path: Option<PathBuf>,
    /// The settings file that changed, when one did.
    pub settings_path: Option<PathBuf>,
    /// Where the previous settings file was kept.
    pub backup: Option<PathBuf>,
    /// The config text to write back, when the change was declarative.
    pub config_text: Option<String>,
}

/// Merge a `statusLine` value into a settings file, preserving everything else.
///
/// The file is backed up first: for the base settings this is a file the user
/// maintains by hand, and cpx does not get to lose it.
fn write_status_line(
    path: &Path,
    status_line: Option<Value>,
) -> Result<Option<PathBuf>, StatusLineError> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(StatusLineError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    let backup = match &existing {
        Some(text) => {
            let backup = path.with_extension("json.cpx.bak");
            std::fs::write(&backup, text).map_err(|source| StatusLineError::Read {
                path: backup.clone(),
                source,
            })?;
            Some(backup)
        }
        None => None,
    };

    let mut value: Value = match &existing {
        Some(text) => serde_json::from_str(text).map_err(|source| StatusLineError::Json {
            path: path.to_path_buf(),
            source,
        })?,
        None => Value::Object(Default::default()),
    };

    let object = value.as_object_mut().ok_or_else(|| StatusLineError::Json {
        path: path.to_path_buf(),
        source: serde::de::Error::custom("settings.json is not an object"),
    })?;
    match status_line {
        Some(status_line) => {
            object.insert("statusLine".to_string(), status_line);
        }
        None => {
            object.remove("statusLine");
        }
    }

    let mut text = serde_json::to_string_pretty(&value).expect("settings are serializable");
    text.push('\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, text).map_err(|source| StatusLineError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(backup)
}

/// The `statusLine` value pointing at a wrapper.
fn status_line_value(script_path: &Path, refresh: Option<u64>) -> Value {
    let mut value = serde_json::json!({
        "type": "command",
        "command": command_for(script_path),
    });
    if let Some(refresh) = refresh {
        value["refreshInterval"] = serde_json::json!(refresh);
    }
    value
}

/// Install the wrapper described by `plan`.
///
/// Returns the config text to persist when the change is declarative; the
/// caller writes it, keeping all config writing in one place.
pub fn install(
    plan: &Installation,
    config_text: &str,
    refresh: Option<u64>,
) -> Result<Applied, StatusLineError> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = plan.script_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| StatusLineError::Read {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&plan.script_path, &plan.script).map_err(|source| StatusLineError::Read {
        path: plan.script_path.clone(),
        source,
    })?;
    std::fs::set_permissions(&plan.script_path, std::fs::Permissions::from_mode(0o755)).ok();

    match &plan.write {
        SettingsWrite::File { path } => {
            let backup = write_status_line(path, Some(status_line_value(&plan.script_path, refresh)))?;
            Ok(Applied {
                script_path: Some(plan.script_path.clone()),
                settings_path: Some(path.clone()),
                backup,
                config_text: None,
            })
        }
        SettingsWrite::ConfigPatch { profile } => {
            let patch = serde_json::json!({
                "statusLine": status_line_value(&plan.script_path, refresh),
            });
            let edited = crate::config_edit::set_resource_patch(
                config_text,
                profile,
                ResourceKey::Settings.config_name(),
                Some(&patch),
            )
            .map_err(|e| StatusLineError::Json {
                path: PathBuf::from("config.toml"),
                source: serde::de::Error::custom(e.to_string()),
            })?;
            Ok(Applied {
                script_path: Some(plan.script_path.clone()),
                settings_path: None,
                backup: None,
                config_text: Some(edited),
            })
        }
    }
}

/// Put back whatever the wrapper was delegating to, and remove the wrapper.
pub fn remove(plan: &Installation, config_text: &str) -> Result<Applied, StatusLineError> {
    let restored = plan.delegate.as_ref().map(|delegate| {
        serde_json::json!({ "type": "command", "command": delegate.command })
    });

    let applied = match &plan.write {
        SettingsWrite::File { path } => {
            let backup = write_status_line(path, restored)?;
            Applied {
                script_path: None,
                settings_path: Some(path.clone()),
                backup,
                config_text: None,
            }
        }
        SettingsWrite::ConfigPatch { profile } => {
            let edited = crate::config_edit::set_resource_patch(
                config_text,
                profile,
                ResourceKey::Settings.config_name(),
                None,
            )
            .map_err(|e| StatusLineError::Json {
                path: PathBuf::from("config.toml"),
                source: serde::de::Error::custom(e.to_string()),
            })?;
            Applied {
                script_path: None,
                settings_path: None,
                backup: None,
                config_text: Some(edited),
            }
        }
    };

    // The wrapper is ours and nothing points at it any more.
    let _ = std::fs::remove_file(&plan.script_path);
    Ok(applied)
}

/// Interpreters that take a script path as their first non-flag argument.
const INTERPRETERS: &[&str] = &["node", "bash", "sh", "zsh", "python", "python3", "deno", "bun", "ruby", "perl"];

/// Split a command the way a shell would, honouring quotes.
fn split_command(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;

    for c in command.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => current.push(c),
        }
    }
    if started || !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The script a statusline command runs, when it runs one.
///
/// Returns `None` for commands with no file to edit — `npx something`, a bare
/// binary on PATH, a shell pipeline — because there is nothing to open.
pub fn script_path_of(command: &str, home: &Path) -> Option<PathBuf> {
    let tokens = split_command(command);
    let mut candidates = tokens.iter();

    let first = candidates.next()?;
    let first_name = Path::new(first).file_name()?.to_str()?;

    let raw = if INTERPRETERS.contains(&first_name) {
        // The script is the first argument that is not a flag.
        candidates.find(|token| !token.starts_with('-'))?
    } else if first_name == "npx" || first_name == "npm" || first_name == "pnpm" {
        return None;
    } else {
        first
    };

    let expanded = crate::config::expand_path(raw, home);
    // A bare name on PATH is not a file to edit.
    if !expanded.is_absolute() {
        return None;
    }
    Some(expanded)
}

/// The script behind a target's statusline, and whether editing it is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFile {
    pub path: PathBuf,
    pub contents: String,
    /// True when the file is one cpx made for this profile, so edits are safe.
    pub owned: bool,
    /// Set when the file looks installed by a package manager that will
    /// overwrite it — editing in place would lose the change.
    pub managed_by: Option<String>,
}

/// A copy made so a shared script can be edited safely.
#[derive(Debug, Clone)]
pub struct Fork {
    pub path: PathBuf,
    pub command: String,
}

/// Who, if anyone, will overwrite this file later.
fn managed_by(path: &Path, contents: &str) -> Option<String> {
    let text = contents.to_ascii_lowercase();
    let looks_installed = ["npx ", "npm i", "install with", "generated by"]
        .iter()
        .any(|needle| text.contains(needle));
    let in_package_dir = path
        .components()
        .any(|c| c.as_os_str() == "node_modules" || c.as_os_str() == ".npm");

    if in_package_dir {
        return Some("a package in node_modules".to_string());
    }
    if looks_installed {
        // Take the first line that says how it is installed; it is the most
        // useful thing to show someone about to edit it.
        let hint = contents
            .lines()
            .take(30)
            .find(|line| line.to_ascii_lowercase().contains("npx "))
            .map(|line| line.trim_start_matches(['#', '/', ' ']).trim().to_string());
        return Some(hint.unwrap_or_else(|| "an installer".to_string()));
    }
    None
}

/// Read the script a target's statusline runs.
pub fn script_of(
    config: &Config,
    layout: &Layout,
    target: &Target,
) -> Result<Option<ScriptFile>, StatusLineError> {
    let plan = plan_install(config, layout, target, None)?;

    // With a badge installed the interesting file is what it delegates to;
    // without one it is whatever is configured.
    let command = match &plan.delegate {
        Some(delegate) => delegate.command.clone(),
        None => return Ok(None),
    };

    let Some(path) = script_path_of(&command, &layout.home) else {
        return Ok(None);
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StatusLineError::Read { path, source }),
    };

    let owned = path.starts_with(&layout.root);
    Ok(Some(ScriptFile {
        managed_by: if owned { None } else { managed_by(&path, &contents) },
        owned,
        path,
        contents,
    }))
}

/// Write a script back, keeping the previous contents alongside it.
pub fn save_script(path: &Path, contents: &str) -> Result<Option<PathBuf>, StatusLineError> {
    use std::os::unix::fs::PermissionsExt;

    let backup = match std::fs::read_to_string(path) {
        Ok(previous) => {
            let backup = path.with_extension(format!(
                "{}.cpx.bak",
                path.extension().and_then(|e| e.to_str()).unwrap_or("bak")
            ));
            std::fs::write(&backup, previous).map_err(|source| StatusLineError::Read {
                path: backup.clone(),
                source,
            })?;
            Some(backup)
        }
        Err(_) => None,
    };

    let mode = std::fs::metadata(path).ok().map(|m| m.permissions().mode());
    std::fs::write(path, contents).map_err(|source| StatusLineError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(mode) = mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).ok();
    }
    Ok(backup)
}

/// Copy a shared script into the profile's own space so it can be edited
/// without an installer overwriting the changes.
///
/// Returns the new path and the command that should now run it; the caller
/// points the statusline at it the same way it installs a badge.
pub fn fork_script(
    config: &Config,
    layout: &Layout,
    target: &Target,
) -> Result<Option<Fork>, StatusLineError> {
    use std::os::unix::fs::PermissionsExt;

    let Some(script) = script_of(config, layout, target)? else {
        return Ok(None);
    };
    if script.owned {
        return Ok(None);
    }

    let dir = match target {
        Target::Base => layout.root.clone(),
        Target::Profile(name) => config.support_dir(layout, name),
    };
    let file_name = script
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "statusline".to_string());
    let destination = dir.join(format!("custom-{file_name}"));

    std::fs::create_dir_all(&dir).map_err(|source| StatusLineError::Read {
        path: dir.clone(),
        source,
    })?;
    std::fs::write(&destination, &script.contents).map_err(|source| StatusLineError::Read {
        path: destination.clone(),
        source,
    })?;
    if let Ok(meta) = std::fs::metadata(&script.path) {
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(meta.permissions().mode())).ok();
    }

    // Keep whatever interpreter the original used.
    let original = split_command(&plan_command_of(config, layout, target)?);
    let interpreter = original
        .first()
        .filter(|first| {
            Path::new(first)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| INTERPRETERS.contains(&n))
                .unwrap_or(false)
        })
        .cloned();

    let command = match interpreter {
        Some(interpreter) => format!("{interpreter} {}", sh_quote(&destination.to_string_lossy())),
        None => sh_quote(&destination.to_string_lossy()),
    };

    Ok(Some(Fork {
        path: destination,
        command,
    }))
}

/// The command whose script a target's statusline currently runs.
fn plan_command_of(
    config: &Config,
    layout: &Layout,
    target: &Target,
) -> Result<String, StatusLineError> {
    Ok(plan_install(config, layout, target, None)?
        .delegate
        .map(|d| d.command)
        .unwrap_or_default())
}

/// Point a target's statusline at `command`, wherever that target records it.
///
/// Used after forking a script: the copy is what should run from then on.
pub fn set_command(
    config: &Config,
    layout: &Layout,
    target: &Target,
    command: &str,
    config_text: &str,
) -> Result<Applied, StatusLineError> {
    let plan = plan_install(config, layout, target, None)?;
    let status_line = serde_json::json!({ "type": "command", "command": command });

    match &plan.write {
        SettingsWrite::File { path } => {
            let backup = write_status_line(path, Some(status_line))?;
            Ok(Applied {
                script_path: None,
                settings_path: Some(path.clone()),
                backup,
                config_text: None,
            })
        }
        SettingsWrite::ConfigPatch { profile } => {
            let patch = serde_json::json!({ "statusLine": status_line });
            let edited = crate::config_edit::set_resource_patch(
                config_text,
                profile,
                ResourceKey::Settings.config_name(),
                Some(&patch),
            )
            .map_err(|e| StatusLineError::Json {
                path: PathBuf::from("config.toml"),
                source: serde::de::Error::custom(e.to_string()),
            })?;
            Ok(Applied {
                script_path: None,
                settings_path: None,
                backup: None,
                config_text: Some(edited),
            })
        }
    }
}
