//! `~/.claude-profiles/config.toml` parsing and resolution.

use crate::error::ConfigError;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// How a resource is materialized into a profile directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMode {
    /// Symlink to the resource under `source_dir`.
    Link,
    /// Copied once from source; refreshed by `apply --sync`.
    Copy,
    /// Profile-private; source is never consulted.
    Own,
    /// Regenerated every apply as source JSON deep-merged with `patch`.
    Merge,
}

/// The closed set of configurable resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKey {
    Settings,
    SettingsLocal,
    ClaudeMd,
    Commands,
    Skills,
    Agents,
    Plugins,
    Hooks,
    Projects,
}

/// A resource's mode plus, for `merge`, the patch applied over the source.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSpec {
    pub mode: ResourceMode,
    pub patch: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub description: String,
    pub model: Option<String>,
    pub add_dirs: Vec<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub resources: BTreeMap<ResourceKey, ResourceSpec>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub version: u32,
    pub source_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub wrapper_prefix: String,
    pub profiles: BTreeMap<String, Profile>,
}

impl ResourceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceMode::Link => "link",
            ResourceMode::Copy => "copy",
            ResourceMode::Own => "own",
            ResourceMode::Merge => "merge",
        }
    }

    fn parse(raw: &str) -> Option<ResourceMode> {
        match raw {
            "link" => Some(ResourceMode::Link),
            "copy" => Some(ResourceMode::Copy),
            "own" => Some(ResourceMode::Own),
            "merge" => Some(ResourceMode::Merge),
            _ => None,
        }
    }
}

impl ResourceKey {
    /// Every configurable key, in declaration order.
    pub const ALL: [ResourceKey; 9] = [
        ResourceKey::Settings,
        ResourceKey::SettingsLocal,
        ResourceKey::ClaudeMd,
        ResourceKey::Commands,
        ResourceKey::Skills,
        ResourceKey::Agents,
        ResourceKey::Plugins,
        ResourceKey::Hooks,
        ResourceKey::Projects,
    ];

    /// The key as written in `config.toml`.
    pub fn config_name(self) -> &'static str {
        match self {
            ResourceKey::Settings => "settings",
            ResourceKey::SettingsLocal => "settings_local",
            ResourceKey::ClaudeMd => "CLAUDE.md",
            ResourceKey::Commands => "commands",
            ResourceKey::Skills => "skills",
            ResourceKey::Agents => "agents",
            ResourceKey::Plugins => "plugins",
            ResourceKey::Hooks => "hooks",
            ResourceKey::Projects => "projects",
        }
    }

    /// The file or directory name this key materializes to.
    pub fn target_name(self) -> &'static str {
        match self {
            ResourceKey::Settings => "settings.json",
            ResourceKey::SettingsLocal => "settings.local.json",
            ResourceKey::ClaudeMd => "CLAUDE.md",
            other => other.config_name(),
        }
    }

    /// Whether the resource is a directory rather than a file.
    pub fn is_dir(self) -> bool {
        matches!(
            self,
            ResourceKey::Commands
                | ResourceKey::Skills
                | ResourceKey::Agents
                | ResourceKey::Plugins
                | ResourceKey::Hooks
                | ResourceKey::Projects
        )
    }

    /// Whether the resource is JSON, and therefore eligible for `merge`.
    pub fn is_json(self) -> bool {
        matches!(self, ResourceKey::Settings | ResourceKey::SettingsLocal)
    }

    pub fn parse(name: &str) -> Option<ResourceKey> {
        ResourceKey::ALL.into_iter().find(|k| k.config_name() == name)
    }

    fn known_names() -> String {
        ResourceKey::ALL
            .iter()
            .map(|k| k.config_name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The built-in resource modes, used for any key the config does not set.
pub fn builtin_defaults() -> BTreeMap<ResourceKey, ResourceSpec> {
    use ResourceKey::*;
    use ResourceMode::*;
    [
        (Settings, Merge),
        (SettingsLocal, Merge),
        (ClaudeMd, Copy),
        (Commands, Link),
        (Skills, Link),
        (Agents, Link),
        (Plugins, Link),
        (Hooks, Link),
        (Projects, Own),
    ]
    .into_iter()
    .map(|(key, mode)| (key, ResourceSpec { mode, patch: None }))
    .collect()
}

/// Expand a leading `~` against `home`. Other paths are returned unchanged.
pub fn expand_path(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) => match rest.strip_prefix('/') {
            Some(rest) => home.join(rest),
            None => PathBuf::from(raw),
        },
        None => PathBuf::from(raw),
    }
}

/// Reject profile names that would escape the profiles directory or collide
/// with a path component. The name becomes both a directory name and part of
/// a wrapper filename, so it must be a single safe path segment.
fn validate_profile_name(name: &str) -> Result<(), ConfigError> {
    let reject = |reason: &str| {
        Err(ConfigError::InvalidProfileName {
            name: name.to_string(),
            reason: reason.to_string(),
        })
    };
    if name.is_empty() {
        return reject("it is empty");
    }
    if name == "." || name == ".." {
        return reject("it refers to a directory rather than naming one");
    }
    if name.contains('/') || name.contains('\\') {
        return reject("it contains a path separator");
    }
    if name.starts_with('.') {
        return reject("it starts with a dot");
    }
    if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return reject("it contains whitespace or control characters");
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum RawResource {
    Bare(String),
    Table {
        mode: String,
        patch: Option<toml::Value>,
    },
}

#[derive(serde::Deserialize, Default)]
struct RawDefaults {
    #[serde(default)]
    resources: BTreeMap<String, RawResource>,
}

#[derive(serde::Deserialize, Default)]
struct RawProfile {
    #[serde(default)]
    description: String,
    model: Option<String>,
    #[serde(default)]
    add_dirs: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    resources: BTreeMap<String, RawResource>,
}

/// Unknown top-level and per-profile keys are tolerated on purpose: a `cpm`
/// config carries `[cloud]` and `[profiles.x.attribution]` sections that this
/// phase does not consume but must not choke on.
#[derive(serde::Deserialize)]
struct RawConfig {
    #[serde(default = "default_version")]
    version: u32,
    source_dir: Option<String>,
    bin_dir: Option<String>,
    wrapper_prefix: Option<String>,
    #[serde(default)]
    defaults: RawDefaults,
    #[serde(default)]
    profiles: BTreeMap<String, RawProfile>,
}

fn default_version() -> u32 {
    1
}

/// Overlay one layer of raw resource declarations onto a resolved map.
fn overlay(
    into: &mut BTreeMap<ResourceKey, ResourceSpec>,
    raw: &BTreeMap<String, RawResource>,
) -> Result<(), ConfigError> {
    for (name, value) in raw {
        let key = ResourceKey::parse(name).ok_or_else(|| ConfigError::UnknownResourceKey {
            key: name.clone(),
            known: ResourceKey::known_names(),
        })?;

        let (mode_str, patch) = match value {
            RawResource::Bare(mode) => (mode.clone(), None),
            RawResource::Table { mode, patch } => (mode.clone(), patch.clone()),
        };

        let mode = ResourceMode::parse(&mode_str).ok_or_else(|| ConfigError::UnknownResourceMode {
            key: name.clone(),
            mode: mode_str.clone(),
        })?;

        if patch.is_some() && mode != ResourceMode::Merge {
            return Err(ConfigError::PatchOnNonMergeMode {
                key: name.clone(),
                mode: mode_str,
            });
        }

        if mode == ResourceMode::Merge && !key.is_json() {
            return Err(ConfigError::IncompatibleMode {
                key: name.clone(),
                mode: mode_str,
                reason: if key.is_dir() {
                    "merge applies to JSON files, and this resource is a directory".to_string()
                } else {
                    "merge applies to JSON files, and this resource is not JSON".to_string()
                },
            });
        }

        let patch = patch.map(|value| serde_json::to_value(value).expect("TOML values are representable as JSON"));

        into.insert(key, ResourceSpec { mode, patch });
    }
    Ok(())
}

impl Config {
    /// Parse `config.toml` text, expanding `~` against `home`.
    pub fn parse(text: &str, home: &Path) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(text)?;

        let mut defaults = builtin_defaults();
        overlay(&mut defaults, &raw.defaults.resources)?;

        let mut profiles = BTreeMap::new();
        for (name, raw_profile) in &raw.profiles {
            validate_profile_name(name)?;

            let mut resources = defaults.clone();
            overlay(&mut resources, &raw_profile.resources)?;

            profiles.insert(
                name.clone(),
                Profile {
                    description: raw_profile.description.clone(),
                    model: raw_profile.model.clone(),
                    add_dirs: raw_profile
                        .add_dirs
                        .iter()
                        .map(|d| expand_path(d, home))
                        .collect(),
                    env: raw_profile.env.clone(),
                    resources,
                },
            );
        }

        Ok(Config {
            version: raw.version,
            source_dir: expand_path(
                raw.source_dir.as_deref().unwrap_or("~/.claude"),
                home,
            ),
            bin_dir: expand_path(raw.bin_dir.as_deref().unwrap_or("~/.local/bin"), home),
            wrapper_prefix: raw.wrapper_prefix.unwrap_or_else(|| "claude-".to_string()),
            profiles,
        })
    }

    /// The directory a profile is materialized into.
    pub fn profile_dir(&self, profiles_dir: &Path, name: &str) -> PathBuf {
        profiles_dir.join(name)
    }

    /// The wrapper script path for a profile.
    pub fn wrapper_path(&self, name: &str) -> PathBuf {
        self.bin_dir.join(format!("{}{}", self.wrapper_prefix, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn home() -> PathBuf {
        PathBuf::from("/Users/tester")
    }

    fn parse(text: &str) -> Config {
        Config::parse(text, &home()).expect("config should parse")
    }

    const MINIMAL: &str = r#"
version = 1

[profiles.work]
description = "Company account"
"#;

    #[test]
    fn parses_a_minimal_config() {
        let cfg = parse(MINIMAL);
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.profiles["work"].description, "Company account");
    }

    #[test]
    fn source_and_bin_dirs_default_and_expand_tilde() {
        let cfg = parse(MINIMAL);
        assert_eq!(cfg.source_dir, home().join(".claude"));
        assert_eq!(cfg.bin_dir, home().join(".local/bin"));
        assert_eq!(cfg.wrapper_prefix, "claude-");
    }

    #[test]
    fn explicit_tilde_paths_expand_against_home() {
        let cfg = parse(
            r#"
version = 1
source_dir = "~/dotfiles/claude"
bin_dir = "~/bin"
[profiles.work]
"#,
        );
        assert_eq!(cfg.source_dir, home().join("dotfiles/claude"));
        assert_eq!(cfg.bin_dir, home().join("bin"));
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        let cfg = parse("version = 1\nsource_dir = \"/etc/claude\"\n[profiles.a]\n");
        assert_eq!(cfg.source_dir, PathBuf::from("/etc/claude"));
    }

    #[test]
    fn profile_without_resources_gets_builtin_defaults() {
        let cfg = parse(MINIMAL);
        let res = &cfg.profiles["work"].resources;
        assert_eq!(res[&ResourceKey::Settings].mode, ResourceMode::Merge);
        assert_eq!(res[&ResourceKey::Commands].mode, ResourceMode::Link);
        assert_eq!(res[&ResourceKey::Projects].mode, ResourceMode::Own);
        assert_eq!(res[&ResourceKey::ClaudeMd].mode, ResourceMode::Copy);
        assert_eq!(res.len(), ResourceKey::ALL.len());
    }

    #[test]
    fn config_defaults_override_builtin_defaults() {
        let cfg = parse(
            r#"
version = 1
[defaults.resources]
projects = "link"
[profiles.work]
"#,
        );
        assert_eq!(
            cfg.profiles["work"].resources[&ResourceKey::Projects].mode,
            ResourceMode::Link
        );
    }

    #[test]
    fn profile_resources_override_config_defaults() {
        let cfg = parse(
            r#"
version = 1
[defaults.resources]
projects = "own"
[profiles.work.resources]
projects = "link"
[profiles.personal]
"#,
        );
        assert_eq!(
            cfg.profiles["work"].resources[&ResourceKey::Projects].mode,
            ResourceMode::Link
        );
        assert_eq!(
            cfg.profiles["personal"].resources[&ResourceKey::Projects].mode,
            ResourceMode::Own
        );
    }

    #[test]
    fn table_form_carries_a_merge_patch() {
        let cfg = parse(
            r#"
version = 1
[profiles.work.resources.settings]
mode = "merge"
patch = { model = "sonnet" }
"#,
        );
        let spec = &cfg.profiles["work"].resources[&ResourceKey::Settings];
        assert_eq!(spec.mode, ResourceMode::Merge);
        assert_eq!(spec.patch, Some(json!({"model": "sonnet"})));
    }

    #[test]
    fn unknown_resource_key_is_rejected() {
        let err = Config::parse(
            "version = 1\n[profiles.work.resources]\nsettingz = \"copy\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(
            matches!(err, ConfigError::UnknownResourceKey { ref key, .. } if key == "settingz"),
            "got {err:?}"
        );
    }

    #[test]
    fn unknown_resource_mode_is_rejected() {
        let err = Config::parse(
            "version = 1\n[profiles.work.resources]\nsettings = \"symlink\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::UnknownResourceMode { .. }), "got {err:?}");
    }

    #[test]
    fn patch_on_a_non_merge_mode_is_rejected() {
        let err = Config::parse(
            r#"
version = 1
[profiles.work.resources.settings]
mode = "copy"
patch = { model = "sonnet" }
"#,
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::PatchOnNonMergeMode { .. }), "got {err:?}");
    }

    #[test]
    fn merge_mode_on_a_directory_resource_is_rejected() {
        let err = Config::parse(
            "version = 1\n[profiles.work.resources]\ncommands = \"merge\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::IncompatibleMode { .. }), "got {err:?}");
    }

    #[test]
    fn merge_mode_on_a_non_json_file_is_rejected() {
        let err = Config::parse(
            "version = 1\n[profiles.work.resources]\n\"CLAUDE.md\" = \"merge\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::IncompatibleMode { .. }), "got {err:?}");
    }

    #[test]
    fn profile_name_with_a_path_separator_is_rejected() {
        let err = Config::parse("version = 1\n[profiles.\"a/b\"]\n", &home()).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfileName { .. }), "got {err:?}");
    }

    #[test]
    fn profile_name_that_is_a_parent_reference_is_rejected() {
        let err = Config::parse("version = 1\n[profiles.\"..\"]\n", &home()).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidProfileName { .. }), "got {err:?}");
    }

    #[test]
    fn profile_env_and_add_dirs_parse_and_expand() {
        let cfg = parse(
            r#"
version = 1
[profiles.work]
model = "sonnet"
add_dirs = ["~/Work/company", "/srv/shared"]
env = { ANTHROPIC_LOG = "debug" }
"#,
        );
        let p = &cfg.profiles["work"];
        assert_eq!(p.model.as_deref(), Some("sonnet"));
        assert_eq!(
            p.add_dirs,
            vec![home().join("Work/company"), PathBuf::from("/srv/shared")]
        );
        assert_eq!(p.env["ANTHROPIC_LOG"], "debug");
    }

    #[test]
    fn wrapper_path_uses_the_configured_prefix() {
        let cfg = parse("version = 1\nwrapper_prefix = \"cc-\"\n[profiles.work]\n");
        assert_eq!(cfg.wrapper_path("work"), home().join(".local/bin/cc-work"));
    }

    #[test]
    fn resource_keys_map_to_their_target_names() {
        assert_eq!(ResourceKey::Settings.target_name(), "settings.json");
        assert_eq!(ResourceKey::SettingsLocal.target_name(), "settings.local.json");
        assert_eq!(ResourceKey::ClaudeMd.target_name(), "CLAUDE.md");
        assert_eq!(ResourceKey::Commands.target_name(), "commands");
        assert!(ResourceKey::Commands.is_dir());
        assert!(!ResourceKey::Settings.is_dir());
        assert!(ResourceKey::Settings.is_json());
        assert!(!ResourceKey::ClaudeMd.is_json());
    }

    #[test]
    fn every_resource_key_round_trips_through_its_config_name() {
        for key in ResourceKey::ALL {
            assert_eq!(ResourceKey::parse(key.config_name()), Some(key));
        }
    }
}
