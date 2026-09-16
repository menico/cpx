//! `~/.claude-profiles/config.toml` parsing and resolution.

use crate::error::ConfigError;
use crate::layout::Layout;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which tool a profile is for.
///
/// Both tools keep everything about one login in a single directory, chosen
/// by an environment variable, which is what makes a profile possible at
/// all. Everything that differs between them — the variable, the binary, the
/// files worth managing — hangs off this enum, so nothing else in cpx needs
/// to know which one it is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }

    pub fn parse(raw: &str) -> Option<Provider> {
        match raw {
            "claude" => Some(Provider::Claude),
            "codex" => Some(Provider::Codex),
            _ => None,
        }
    }

    /// The short product name, for labels: `Claude`, `Codex`.
    pub fn short_name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex",
        }
    }

    /// The product name, for people rather than files.
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
        }
    }

    /// The executable a profile's wrapper runs.
    pub fn binary_name(self) -> &'static str {
        self.as_str()
    }

    /// The variable that points the tool at a profile's directory.
    pub fn home_env_var(self) -> &'static str {
        match self {
            Provider::Claude => "CLAUDE_CONFIG_DIR",
            Provider::Codex => "CODEX_HOME",
        }
    }

    /// The variable naming the running profile, read by statusline badges
    /// and by `cpx which`.
    pub fn profile_env_var(self) -> &'static str {
        match self {
            Provider::Claude => "CLAUDE_PROFILE",
            Provider::Codex => "CODEX_PROFILE",
        }
    }

    /// Where the tool keeps its own state when no profile is in play, and
    /// therefore what profiles are built from by default.
    pub fn default_source_dir(self) -> &'static str {
        match self {
            Provider::Claude => "~/.claude",
            Provider::Codex => "~/.codex",
        }
    }

    pub fn default_wrapper_prefix(self) -> &'static str {
        match self {
            Provider::Claude => "claude-",
            Provider::Codex => "codex-",
        }
    }

    /// Inherited variables a wrapper drops, so a parent shell cannot point a
    /// profile at a different account.
    pub fn scrubbed_env_prefixes(self) -> &'static [&'static str] {
        match self {
            Provider::Claude => &["CLAUDE_", "ANTHROPIC_"],
            Provider::Codex => &["CODEX_", "OPENAI_"],
        }
    }

    /// The hand-rolled directory name prefix people use: `~/.claude-work`.
    pub fn dir_prefix(self) -> &'static str {
        match self {
            Provider::Claude => ".claude-",
            Provider::Codex => ".codex-",
        }
    }
}

/// How a resource is materialized into a profile directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceMode {
    /// cpx does not manage this resource at all: it plans nothing, creates
    /// nothing, and reports nothing. The mode adoption uses for resources an
    /// existing directory does not have, so cpx never litters it.
    Ignore,
    /// Symlink to the resource under `source_dir`.
    Link,
    /// Copied once from source; refreshed by `apply --sync`.
    Copy,
    /// Profile-private; source is never consulted.
    Own,
    /// Regenerated every apply as source JSON deep-merged with `patch`.
    Merge,
}

/// The closed set of configurable resources, across both providers.
///
/// A key shared by both tools (`skills`, `hooks`, `plugins`) is one variant:
/// it names the same directory in either home, and the provider decides
/// whether it applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKey {
    // Claude Code.
    Settings,
    SettingsLocal,
    ClaudeMd,
    Commands,
    Agents,
    Projects,
    // Codex.
    CodexConfig,
    AgentsMd,
    Prompts,
    Rules,
    HooksJson,
    Memories,
    Sessions,
    // Both.
    Skills,
    Plugins,
    Hooks,
}

/// A resource's mode plus, for `merge`, the patch applied over the source.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSpec {
    pub mode: ResourceMode,
    pub patch: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub provider: Provider,
    pub description: String,
    /// An existing config directory to manage in place, rather than one under
    /// the cpx root. A login is keyed to its directory path, so adopting a
    /// directory where it already sits avoids signing in again.
    pub dir: Option<PathBuf>,
    /// A `#rrggbb` identity colour. Profiles are told apart at a glance by
    /// colour in the app and in a statusline, so it is part of the config
    /// rather than app-local preference.
    pub color: Option<String>,
    pub model: Option<String>,
    pub add_dirs: Vec<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub resources: BTreeMap<ResourceKey, ResourceSpec>,
}

/// What one provider's profiles are built from and named after.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// The directory profiles inherit from. Never written to.
    pub source_dir: PathBuf,
    /// `claude-` gives `claude-work`; `codex-` gives `codex-work`.
    pub wrapper_prefix: String,
    /// The resource modes a profile of this provider starts from.
    pub defaults: BTreeMap<ResourceKey, ResourceSpec>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub version: u32,
    pub bin_dir: PathBuf,
    pub claude: ProviderConfig,
    pub codex: ProviderConfig,
    pub profiles: BTreeMap<String, Profile>,
}

impl ResourceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceMode::Ignore => "ignore",
            ResourceMode::Link => "link",
            ResourceMode::Copy => "copy",
            ResourceMode::Own => "own",
            ResourceMode::Merge => "merge",
        }
    }

    fn parse(raw: &str) -> Option<ResourceMode> {
        match raw {
            "ignore" => Some(ResourceMode::Ignore),
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
    pub const ALL: [ResourceKey; 16] = [
        ResourceKey::Settings,
        ResourceKey::SettingsLocal,
        ResourceKey::ClaudeMd,
        ResourceKey::Commands,
        ResourceKey::Agents,
        ResourceKey::Projects,
        ResourceKey::CodexConfig,
        ResourceKey::AgentsMd,
        ResourceKey::Prompts,
        ResourceKey::Rules,
        ResourceKey::HooksJson,
        ResourceKey::Memories,
        ResourceKey::Sessions,
        ResourceKey::Skills,
        ResourceKey::Plugins,
        ResourceKey::Hooks,
    ];

    /// The keys that mean something in a given provider's home, in the order
    /// a person would want to read them.
    pub fn for_provider(provider: Provider) -> &'static [ResourceKey] {
        use ResourceKey::*;
        match provider {
            Provider::Claude => &[
                Settings,
                SettingsLocal,
                ClaudeMd,
                Commands,
                Skills,
                Agents,
                Plugins,
                Hooks,
                Projects,
            ],
            Provider::Codex => &[
                CodexConfig,
                AgentsMd,
                Skills,
                Prompts,
                Rules,
                Hooks,
                HooksJson,
                Plugins,
                Memories,
                Sessions,
            ],
        }
    }

    pub fn applies_to(self, provider: Provider) -> bool {
        ResourceKey::for_provider(provider).contains(&self)
    }

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
            ResourceKey::CodexConfig => "config",
            ResourceKey::AgentsMd => "AGENTS.md",
            ResourceKey::Prompts => "prompts",
            ResourceKey::Rules => "rules",
            ResourceKey::HooksJson => "hooks_json",
            ResourceKey::Memories => "memories",
            ResourceKey::Sessions => "sessions",
        }
    }

    /// The file or directory name this key materializes to.
    pub fn target_name(self) -> &'static str {
        match self {
            ResourceKey::Settings => "settings.json",
            ResourceKey::SettingsLocal => "settings.local.json",
            ResourceKey::ClaudeMd => "CLAUDE.md",
            ResourceKey::CodexConfig => "config.toml",
            ResourceKey::HooksJson => "hooks.json",
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
                | ResourceKey::Prompts
                | ResourceKey::Rules
                | ResourceKey::Memories
                | ResourceKey::Sessions
        )
    }

    /// Whether the resource is JSON.
    pub fn is_json(self) -> bool {
        matches!(
            self,
            ResourceKey::Settings | ResourceKey::SettingsLocal | ResourceKey::HooksJson
        )
    }

    /// Whether the resource is TOML.
    pub fn is_toml(self) -> bool {
        matches!(self, ResourceKey::CodexConfig)
    }

    /// Whether the resource is structured data, and therefore eligible for
    /// `merge`.
    pub fn supports_merge(self) -> bool {
        self.is_json() || self.is_toml()
    }

    pub fn parse(name: &str) -> Option<ResourceKey> {
        ResourceKey::ALL.into_iter().find(|k| k.config_name() == name)
    }

    fn known_names(provider: Provider) -> String {
        ResourceKey::for_provider(provider)
            .iter()
            .map(|k| k.config_name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The built-in resource modes for a provider, used for any key the config
/// does not set.
pub fn builtin_defaults(provider: Provider) -> BTreeMap<ResourceKey, ResourceSpec> {
    use ResourceKey::*;
    use ResourceMode::*;
    let modes: &[(ResourceKey, ResourceMode)] = match provider {
        Provider::Claude => &[
            (Settings, Merge),
            (SettingsLocal, Merge),
            (ClaudeMd, Copy),
            (Commands, Link),
            (Skills, Link),
            (Agents, Link),
            (Plugins, Link),
            (Hooks, Link),
            (Projects, Own),
        ],
        // Codex keeps its login next to its config, so `config.toml` is the
        // only file worth merging: MCP servers and trust levels come from the
        // source, and a profile overrides the model on top.
        Provider::Codex => &[
            (CodexConfig, Merge),
            (AgentsMd, Copy),
            (Skills, Link),
            (Prompts, Link),
            (Rules, Link),
            (Hooks, Link),
            (HooksJson, Copy),
            (Plugins, Link),
            (Memories, Own),
            (Sessions, Own),
        ],
    };
    modes
        .iter()
        .map(|(key, mode)| (*key, ResourceSpec { mode: *mode, patch: None }))
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

/// Colours are written into generated files, so only a plain `#rrggbb` is
/// accepted — never an arbitrary string.
fn validate_color(profile: &str, raw: &str) -> Result<String, ConfigError> {
    let valid = raw.len() == 7
        && raw.starts_with('#')
        && raw[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(raw.to_ascii_lowercase())
    } else {
        Err(ConfigError::InvalidColor {
            profile: profile.to_string(),
            color: raw.to_string(),
        })
    }
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

/// A `[claude]` or `[codex]` section. Every key is optional: the section
/// exists to override, and an absent one means the built-in defaults.
#[derive(serde::Deserialize, Default)]
struct RawProviderSection {
    source_dir: Option<String>,
    wrapper_prefix: Option<String>,
    #[serde(default)]
    defaults: RawDefaults,
}

#[derive(serde::Deserialize, Default)]
struct RawProfile {
    provider: Option<String>,
    #[serde(default)]
    description: String,
    dir: Option<String>,
    color: Option<String>,
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
///
/// The top-level `source_dir`, `wrapper_prefix` and `[defaults]` are the
/// Claude settings: they predate Codex support and every existing config
/// uses them. `[claude]` is the explicit spelling of the same thing.
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
    claude: RawProviderSection,
    #[serde(default)]
    codex: RawProviderSection,
    #[serde(default)]
    profiles: BTreeMap<String, RawProfile>,
}

fn default_version() -> u32 {
    1
}

/// Overlay one layer of raw resource declarations onto a resolved map.
///
/// Keys that belong to the other provider are refused: a Codex profile
/// declaring `settings` is a mistake worth catching, not a no-op.
fn overlay(
    into: &mut BTreeMap<ResourceKey, ResourceSpec>,
    raw: &BTreeMap<String, RawResource>,
    provider: Provider,
) -> Result<(), ConfigError> {
    for (name, value) in raw {
        let key = ResourceKey::parse(name).ok_or_else(|| ConfigError::UnknownResourceKey {
            key: name.clone(),
            known: ResourceKey::known_names(provider),
        })?;
        if !key.applies_to(provider) {
            return Err(ConfigError::ResourceNotForProvider {
                key: name.clone(),
                provider: provider.as_str().to_string(),
                known: ResourceKey::known_names(provider),
            });
        }

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

        if mode == ResourceMode::Merge && !key.supports_merge() {
            return Err(ConfigError::IncompatibleMode {
                key: name.clone(),
                mode: mode_str,
                reason: if key.is_dir() {
                    "merge applies to JSON and TOML files, and this resource is a directory"
                        .to_string()
                } else {
                    "merge applies to JSON and TOML files, and this resource is neither"
                        .to_string()
                },
            });
        }

        let patch = patch.map(|value| serde_json::to_value(value).expect("TOML values are representable as JSON"));

        into.insert(key, ResourceSpec { mode, patch });
    }
    Ok(())
}

fn provider_config(
    provider: Provider,
    section: &RawProviderSection,
    legacy_source_dir: Option<&str>,
    legacy_prefix: Option<&str>,
    legacy_defaults: Option<&RawDefaults>,
    home: &Path,
) -> Result<ProviderConfig, ConfigError> {
    let mut defaults = builtin_defaults(provider);
    if let Some(legacy) = legacy_defaults {
        overlay(&mut defaults, &legacy.resources, provider)?;
    }
    overlay(&mut defaults, &section.defaults.resources, provider)?;

    let source_dir = section
        .source_dir
        .as_deref()
        .or(legacy_source_dir)
        .unwrap_or(provider.default_source_dir());
    let wrapper_prefix = section
        .wrapper_prefix
        .as_deref()
        .or(legacy_prefix)
        .unwrap_or(provider.default_wrapper_prefix());

    Ok(ProviderConfig {
        source_dir: expand_path(source_dir, home),
        wrapper_prefix: wrapper_prefix.to_string(),
        defaults,
    })
}

impl Config {
    /// Parse `config.toml` text, expanding `~` against `home`.
    pub fn parse(text: &str, home: &Path) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(text)?;

        let claude = provider_config(
            Provider::Claude,
            &raw.claude,
            raw.source_dir.as_deref(),
            raw.wrapper_prefix.as_deref(),
            Some(&raw.defaults),
            home,
        )?;
        let codex = provider_config(Provider::Codex, &raw.codex, None, None, None, home)?;

        let mut profiles = BTreeMap::new();
        for (name, raw_profile) in &raw.profiles {
            validate_profile_name(name)?;

            let provider = match raw_profile.provider.as_deref() {
                None => Provider::Claude,
                Some(raw) => Provider::parse(raw).ok_or_else(|| ConfigError::UnknownProvider {
                    profile: name.clone(),
                    provider: raw.to_string(),
                })?,
            };

            let mut resources = match provider {
                Provider::Claude => claude.defaults.clone(),
                Provider::Codex => codex.defaults.clone(),
            };
            overlay(&mut resources, &raw_profile.resources, provider)?;

            profiles.insert(
                name.clone(),
                Profile {
                    provider,
                    description: raw_profile.description.clone(),
                    dir: raw_profile.dir.as_deref().map(|d| expand_path(d, home)),
                    color: match &raw_profile.color {
                        Some(color) => Some(validate_color(name, color)?),
                        None => None,
                    },
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
            bin_dir: expand_path(raw.bin_dir.as_deref().unwrap_or("~/.local/bin"), home),
            claude,
            codex,
            profiles,
        })
    }

    pub fn provider(&self, provider: Provider) -> &ProviderConfig {
        match provider {
            Provider::Claude => &self.claude,
            Provider::Codex => &self.codex,
        }
    }

    /// The directory a provider's profiles inherit from.
    pub fn source_dir(&self, provider: Provider) -> &Path {
        &self.provider(provider).source_dir
    }

    /// Every source directory, whichever providers are in use. Nothing may
    /// ever write inside any of them.
    pub fn source_dirs(&self) -> Vec<PathBuf> {
        Provider::ALL
            .iter()
            .map(|p| self.source_dir(*p).to_path_buf())
            .collect()
    }

    pub fn wrapper_prefix(&self, provider: Provider) -> &str {
        &self.provider(provider).wrapper_prefix
    }

    /// The providers that at least one profile uses. Claude when there are
    /// no profiles at all, since that is what a fresh config is for.
    pub fn providers_in_use(&self) -> Vec<Provider> {
        let mut used: Vec<Provider> = self.profiles.values().map(|p| p.provider).collect();
        used.sort();
        used.dedup();
        if used.is_empty() {
            used.push(Provider::Claude);
        }
        used
    }

    /// A profile's provider. Callers that already hold the profile should
    /// read the field; this is for the many places that only have a name.
    pub fn provider_of(&self, name: &str) -> Result<Provider, ConfigError> {
        self.profiles
            .get(name)
            .map(|p| p.provider)
            .ok_or_else(|| ConfigError::UnknownProfile(name.to_string()))
    }

    /// What the tool's home variable points at for this profile: an adopted
    /// directory when one is configured, otherwise one under the cpx root.
    pub fn config_dir(&self, layout: &Layout, name: &str) -> PathBuf {
        match self.profiles.get(name).and_then(|p| p.dir.clone()) {
            Some(dir) => dir,
            None => layout.profile_dir(name),
        }
    }

    /// Where cpx keeps its own artifacts for this profile. Always under the
    /// cpx root, so adopting a directory never scatters cpx files through it.
    pub fn support_dir(&self, layout: &Layout, name: &str) -> PathBuf {
        layout.profile_dir(name)
    }

    /// The shim that makes a plain `claude` (or `codex`) run under this
    /// profile.
    pub fn shim_path(&self, layout: &Layout, name: &str) -> PathBuf {
        let binary = self
            .profiles
            .get(name)
            .map(|p| p.provider)
            .unwrap_or(Provider::Claude)
            .binary_name();
        layout.profile_bin_dir(name).join(binary)
    }

    /// The name of a profile's command: `claude-work`, `codex-work`.
    pub fn command_name(&self, name: &str) -> String {
        let provider = self
            .profiles
            .get(name)
            .map(|p| p.provider)
            .unwrap_or(Provider::Claude);
        format!("{}{name}", self.wrapper_prefix(provider))
    }

    /// The wrapper script path for a profile.
    pub fn wrapper_path(&self, name: &str) -> PathBuf {
        self.bin_dir.join(self.command_name(name))
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
        assert_eq!(cfg.profiles["work"].provider, Provider::Claude);
    }

    #[test]
    fn source_and_bin_dirs_default_and_expand_tilde() {
        let cfg = parse(MINIMAL);
        assert_eq!(cfg.source_dir(Provider::Claude), home().join(".claude"));
        assert_eq!(cfg.source_dir(Provider::Codex), home().join(".codex"));
        assert_eq!(cfg.bin_dir, home().join(".local/bin"));
        assert_eq!(cfg.wrapper_prefix(Provider::Claude), "claude-");
        assert_eq!(cfg.wrapper_prefix(Provider::Codex), "codex-");
    }

    #[test]
    fn explicit_tilde_paths_expand_against_home() {
        let cfg = parse(
            r#"
version = 1
source_dir = "~/dotfiles/claude"
bin_dir = "~/bin"
[codex]
source_dir = "~/dotfiles/codex"
[profiles.work]
"#,
        );
        assert_eq!(cfg.source_dir(Provider::Claude), home().join("dotfiles/claude"));
        assert_eq!(cfg.source_dir(Provider::Codex), home().join("dotfiles/codex"));
        assert_eq!(cfg.bin_dir, home().join("bin"));
    }

    #[test]
    fn the_claude_section_is_another_spelling_of_the_top_level_keys() {
        let cfg = parse(
            "version = 1\n[claude]\nsource_dir = \"/etc/claude\"\nwrapper_prefix = \"cc-\"\n[profiles.a]\n",
        );
        assert_eq!(cfg.source_dir(Provider::Claude), PathBuf::from("/etc/claude"));
        assert_eq!(cfg.wrapper_prefix(Provider::Claude), "cc-");
    }

    #[test]
    fn absolute_paths_are_left_alone() {
        let cfg = parse("version = 1\nsource_dir = \"/etc/claude\"\n[profiles.a]\n");
        assert_eq!(cfg.source_dir(Provider::Claude), PathBuf::from("/etc/claude"));
    }

    #[test]
    fn profile_without_resources_gets_builtin_defaults() {
        let cfg = parse(MINIMAL);
        let res = &cfg.profiles["work"].resources;
        assert_eq!(res[&ResourceKey::Settings].mode, ResourceMode::Merge);
        assert_eq!(res[&ResourceKey::Commands].mode, ResourceMode::Link);
        assert_eq!(res[&ResourceKey::Projects].mode, ResourceMode::Own);
        assert_eq!(res[&ResourceKey::ClaudeMd].mode, ResourceMode::Copy);
        assert_eq!(res.len(), ResourceKey::for_provider(Provider::Claude).len());
        assert!(!res.contains_key(&ResourceKey::CodexConfig));
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
        assert_eq!(ResourceKey::CodexConfig.target_name(), "config.toml");
        assert_eq!(ResourceKey::HooksJson.target_name(), "hooks.json");
        assert!(ResourceKey::Commands.is_dir());
        assert!(!ResourceKey::Settings.is_dir());
        assert!(ResourceKey::Settings.is_json());
        assert!(!ResourceKey::ClaudeMd.is_json());
        assert!(ResourceKey::CodexConfig.is_toml());
        assert!(ResourceKey::CodexConfig.supports_merge());
    }

    #[test]
    fn every_resource_key_round_trips_through_its_config_name() {
        for key in ResourceKey::ALL {
            assert_eq!(ResourceKey::parse(key.config_name()), Some(key));
        }
    }

    #[test]
    fn every_key_belongs_to_at_least_one_provider() {
        for key in ResourceKey::ALL {
            assert!(
                Provider::ALL.iter().any(|p| key.applies_to(*p)),
                "{key:?} belongs to nobody"
            );
        }
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/Users/tester")
    }

    fn parse(text: &str) -> Config {
        Config::parse(text, &home()).expect("config should parse")
    }

    #[test]
    fn a_profile_can_declare_itself_a_codex_profile() {
        let cfg = parse("version = 1\n[profiles.work]\nprovider = \"codex\"\n");
        let p = &cfg.profiles["work"];
        assert_eq!(p.provider, Provider::Codex);
        assert_eq!(p.resources[&ResourceKey::CodexConfig].mode, ResourceMode::Merge);
        assert_eq!(p.resources[&ResourceKey::Sessions].mode, ResourceMode::Own);
        assert_eq!(p.resources.len(), ResourceKey::for_provider(Provider::Codex).len());
        assert!(!p.resources.contains_key(&ResourceKey::Settings));
    }

    #[test]
    fn an_unknown_provider_is_refused() {
        let err = Config::parse("version = 1\n[profiles.work]\nprovider = \"gemini\"\n", &home())
            .unwrap_err();
        assert!(matches!(err, ConfigError::UnknownProvider { .. }), "got {err:?}");
        assert!(err.to_string().contains("gemini"));
    }

    #[test]
    fn a_codex_profile_cannot_declare_a_claude_resource() {
        let err = Config::parse(
            "version = 1\n[profiles.work]\nprovider = \"codex\"\n[profiles.work.resources]\nsettings = \"own\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::ResourceNotForProvider { .. }), "got {err:?}");
        assert!(err.to_string().contains("codex"), "{err}");
    }

    #[test]
    fn a_claude_profile_cannot_declare_a_codex_resource() {
        let err = Config::parse(
            "version = 1\n[profiles.work.resources]\nconfig = \"own\"\n",
            &home(),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::ResourceNotForProvider { .. }), "got {err:?}");
    }

    #[test]
    fn shared_keys_apply_to_both_providers() {
        let cfg = parse(
            "version = 1\n[profiles.c]\n[profiles.c.resources]\nskills = \"own\"\n[profiles.x]\nprovider = \"codex\"\n[profiles.x.resources]\nskills = \"own\"\n",
        );
        assert_eq!(cfg.profiles["c"].resources[&ResourceKey::Skills].mode, ResourceMode::Own);
        assert_eq!(cfg.profiles["x"].resources[&ResourceKey::Skills].mode, ResourceMode::Own);
    }

    #[test]
    fn codex_defaults_come_from_their_own_section() {
        let cfg = parse(
            "version = 1\n[defaults.resources]\nskills = \"own\"\n[codex.defaults.resources]\nsessions = \"link\"\n[profiles.x]\nprovider = \"codex\"\n[profiles.c]\n",
        );
        // The top-level defaults are Claude's; Codex is not affected by them.
        assert_eq!(cfg.profiles["x"].resources[&ResourceKey::Skills].mode, ResourceMode::Link);
        assert_eq!(cfg.profiles["x"].resources[&ResourceKey::Sessions].mode, ResourceMode::Link);
        assert_eq!(cfg.profiles["c"].resources[&ResourceKey::Skills].mode, ResourceMode::Own);
    }

    #[test]
    fn claude_defaults_refuse_codex_keys() {
        let err = Config::parse("version = 1\n[defaults.resources]\nprompts = \"link\"\n", &home())
            .unwrap_err();
        assert!(matches!(err, ConfigError::ResourceNotForProvider { .. }), "got {err:?}");
    }

    #[test]
    fn config_toml_may_be_merged_with_a_patch() {
        let cfg = parse(
            "version = 1\n[profiles.x]\nprovider = \"codex\"\n[profiles.x.resources.config]\nmode = \"merge\"\npatch = { model = \"gpt-5.5\" }\n",
        );
        let spec = &cfg.profiles["x"].resources[&ResourceKey::CodexConfig];
        assert_eq!(spec.mode, ResourceMode::Merge);
        assert_eq!(spec.patch, Some(serde_json::json!({"model": "gpt-5.5"})));
    }

    #[test]
    fn codex_commands_and_shims_carry_the_codex_name() {
        let cfg = parse("version = 1\n[profiles.work]\nprovider = \"codex\"\n[profiles.also]\n");
        let l = Layout::new(home());
        assert_eq!(cfg.command_name("work"), "codex-work");
        assert_eq!(cfg.wrapper_path("work"), home().join(".local/bin/codex-work"));
        assert_eq!(cfg.shim_path(&l, "work"), home().join(".claude-profiles/work/bin/codex"));
        assert_eq!(cfg.command_name("also"), "claude-also");
        assert_eq!(cfg.shim_path(&l, "also"), home().join(".claude-profiles/also/bin/claude"));
    }

    #[test]
    fn the_codex_wrapper_prefix_is_configurable() {
        let cfg = parse("version = 1\n[codex]\nwrapper_prefix = \"cx-\"\n[profiles.work]\nprovider = \"codex\"\n");
        assert_eq!(cfg.command_name("work"), "cx-work");
    }

    #[test]
    fn providers_in_use_reports_what_the_profiles_need() {
        assert_eq!(parse("version = 1\n").providers_in_use(), vec![Provider::Claude]);
        assert_eq!(
            parse("version = 1\n[profiles.x]\nprovider = \"codex\"\n").providers_in_use(),
            vec![Provider::Codex]
        );
        assert_eq!(
            parse("version = 1\n[profiles.x]\nprovider = \"codex\"\n[profiles.c]\n").providers_in_use(),
            vec![Provider::Claude, Provider::Codex]
        );
    }

    #[test]
    fn provider_facts_are_what_the_tools_expect() {
        assert_eq!(Provider::Claude.home_env_var(), "CLAUDE_CONFIG_DIR");
        assert_eq!(Provider::Codex.home_env_var(), "CODEX_HOME");
        assert_eq!(Provider::Codex.binary_name(), "codex");
        assert_eq!(Provider::parse("codex"), Some(Provider::Codex));
        assert_eq!(Provider::parse("Codex"), None);
        for p in Provider::ALL {
            assert_eq!(Provider::parse(p.as_str()), Some(p));
        }
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, Path::new("/Users/tester"))
    }

    #[test]
    fn a_profile_colour_is_parsed_and_normalised() {
        let cfg = parse("version = 1\n[profiles.work]\ncolor = \"#5C8DFF\"\n").unwrap();
        assert_eq!(cfg.profiles["work"].color.as_deref(), Some("#5c8dff"));
    }

    #[test]
    fn a_profile_without_a_colour_has_none() {
        let cfg = parse("version = 1\n[profiles.work]\n").unwrap();
        assert_eq!(cfg.profiles["work"].color, None);
    }

    #[test]
    fn a_colour_that_is_not_a_hex_triplet_is_refused() {
        for bad in ["red", "#abc", "#12345g", "5c8dff", "#5c8dff;"] {
            let text = format!("version = 1\n[profiles.work]\ncolor = \"{bad}\"\n");
            assert!(
                matches!(parse(&text), Err(ConfigError::InvalidColor { .. })),
                "accepted {bad:?}"
            );
        }
    }
}

#[cfg(test)]
mod adoption_tests {
    use super::*;
    use crate::layout::Layout;

    fn home() -> PathBuf {
        PathBuf::from("/Users/tester")
    }

    fn parse(text: &str) -> Config {
        Config::parse(text, &home()).expect("should parse")
    }

    fn layout() -> Layout {
        Layout::new(home())
    }

    #[test]
    fn a_profile_without_dir_lives_under_the_cpx_root() {
        let cfg = parse("version = 1\n[profiles.work]\n");
        assert_eq!(
            cfg.config_dir(&layout(), "work"),
            home().join(".claude-profiles/work")
        );
    }

    #[test]
    fn dir_points_the_config_directory_somewhere_else() {
        let cfg = parse("version = 1\n[profiles.hd]\ndir = \"~/.claude-hd\"\n");
        assert_eq!(cfg.config_dir(&layout(), "hd"), home().join(".claude-hd"));
    }

    #[test]
    fn cpx_keeps_its_own_artifacts_out_of_an_adopted_directory() {
        // The shim is cpx's, not the user's: it belongs under the cpx root
        // even when the config directory is somewhere they already own.
        let cfg = parse("version = 1\n[profiles.hd]\ndir = \"~/.claude-hd\"\n");
        let l = layout();
        assert_eq!(cfg.support_dir(&l, "hd"), home().join(".claude-profiles/hd"));
        assert_eq!(
            cfg.shim_path(&l, "hd"),
            home().join(".claude-profiles/hd/bin/claude")
        );
        assert!(
            !cfg.shim_path(&l, "hd").starts_with(home().join(".claude-hd")),
            "adoption must not write cpx scaffolding into the adopted directory"
        );
    }

    #[test]
    fn an_absolute_dir_is_used_as_given() {
        let cfg = parse("version = 1\n[profiles.hd]\ndir = \"/opt/claude-hd\"\n");
        assert_eq!(cfg.config_dir(&layout(), "hd"), PathBuf::from("/opt/claude-hd"));
    }

    #[test]
    fn ignore_is_a_resource_mode() {
        let cfg = parse("version = 1\n[profiles.hd.resources]\ncommands = \"ignore\"\n");
        assert_eq!(
            cfg.profiles["hd"].resources[&ResourceKey::Commands].mode,
            ResourceMode::Ignore
        );
        assert_eq!(ResourceMode::Ignore.as_str(), "ignore");
    }

    #[test]
    fn ignore_is_allowed_for_every_resource() {
        for provider in Provider::ALL {
            for key in ResourceKey::for_provider(provider) {
                let text = format!(
                    "version = 1\n[profiles.hd]\nprovider = \"{}\"\n[profiles.hd.resources]\n\"{}\" = \"ignore\"\n",
                    provider.as_str(),
                    key.config_name()
                );
                assert!(
                    Config::parse(&text, &home()).is_ok(),
                    "ignore rejected for {}",
                    key.config_name()
                );
            }
        }
    }

    #[test]
    fn ignore_takes_no_patch() {
        let text = r#"
version = 1
[profiles.hd.resources.settings]
mode = "ignore"
patch = { model = "sonnet" }
"#;
        assert!(matches!(
            Config::parse(text, &home()),
            Err(ConfigError::PatchOnNonMergeMode { .. })
        ));
    }

    #[test]
    fn a_dir_that_is_not_a_path_is_still_just_a_path() {
        // No validation beyond expansion: the directory may not exist yet,
        // and `adopt` is where existence is checked.
        let cfg = parse("version = 1\n[profiles.hd]\ndir = \"~/nowhere\"\n");
        assert_eq!(cfg.config_dir(&layout(), "hd"), home().join("nowhere"));
    }
}
