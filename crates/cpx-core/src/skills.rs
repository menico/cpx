//! Which skills a profile has, and turning them off.
//!
//! Skills reach a profile two ways, and the two have different levers.
//! A skill of your own is a directory under `<config>/skills/`, and Claude
//! loads whatever it finds there — so the way to switch one off is to move it
//! out, which is reversible and loses nothing. Plugin skills come with their
//! plugin, and `enabledPlugins` in settings only toggles a whole plugin, so
//! that is the granularity offered rather than pretending otherwise.

use crate::config::Config;
use crate::layout::Layout;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a disabled skill is kept. Claude only scans `skills/`, so anything
/// here is inert while staying exactly where the user can find it.
pub const DISABLED_DIR: &str = "skills.disabled";

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("no profile named `{0}`")]
    UnknownProfile(String),

    #[error("no skill named `{0}` in this profile")]
    UnknownSkill(String),

    #[error("a skill named `{0}` is already there")]
    AlreadyExists(String),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    /// The first line of the skill's description, when it has one.
    pub description: Option<String>,
    pub enabled: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSkills {
    /// The `plugin@marketplace` key used in settings.
    pub key: String,
    pub plugin: String,
    pub marketplace: String,
    pub enabled: bool,
    /// How many skills this plugin provides.
    pub skills: usize,
    /// Their names, for showing what turning it off would take away.
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    /// Skills the profile owns, enabled and disabled.
    pub own: Vec<Skill>,
    pub plugins: Vec<PluginSkills>,
    /// True when `skills/` is shared with other profiles, so disabling one
    /// would disable it everywhere.
    pub shared: bool,
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> SkillError + '_ {
    move |source| SkillError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The `description:` from a skill's front matter.
///
/// Handles both an inline value and a folded block (`>-`), which most skills
/// use once the description runs past one line. Only the first sentence is
/// kept: these descriptions are written for matching, not for reading, and run
/// to whole paragraphs.
fn description_of(skill_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(skill_dir.join("SKILL.md")).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut collecting = false;
    let mut folded = String::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }

        if collecting {
            // The block ends at the next key at column zero.
            let is_key = !line.starts_with(char::is_whitespace) && line.contains(':');
            if is_key || trimmed.is_empty() {
                break;
            }
            if !folded.is_empty() {
                folded.push(' ');
            }
            folded.push_str(trimmed);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("description:") {
            let value = rest.trim().trim_matches('"').trim_matches('\'');
            if value.is_empty() || value == ">-" || value == ">" || value == "|" || value == "|-" {
                collecting = true;
                continue;
            }
            return Some(first_sentence(value));
        }
    }

    (!folded.is_empty()).then(|| first_sentence(&folded))
}

/// The first sentence, so a list stays readable.
fn first_sentence(text: &str) -> String {
    match text.find(". ") {
        Some(end) => text[..=end].trim().to_string(),
        None => text.trim().to_string(),
    }
}

/// Skill directories directly under `dir`.
fn skills_in(dir: &Path, enabled: bool) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Skill> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("SKILL.md").is_file())
        .map(|path| Skill {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            description: description_of(&path),
            enabled,
            path,
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// Plugins listed in a settings file, with how many skills each provides.
fn plugins_in(config_dir: &Path, enabled_plugins: &BTreeMap<String, bool>) -> Vec<PluginSkills> {
    let cache = config_dir.join("plugins").join("cache");

    let mut found: Vec<PluginSkills> = enabled_plugins
        .iter()
        .map(|(key, enabled)| {
            let (plugin, marketplace) = key.split_once('@').unwrap_or((key.as_str(), ""));
            let names = plugin_skill_names(&cache, marketplace, plugin);
            PluginSkills {
                key: key.clone(),
                plugin: plugin.to_string(),
                marketplace: marketplace.to_string(),
                enabled: *enabled,
                skills: names.len(),
                names,
            }
        })
        .collect();
    found.sort_by(|a, b| a.plugin.cmp(&b.plugin));
    found
}

/// Skill names a plugin provides, taking its newest installed version.
fn plugin_skill_names(cache: &Path, marketplace: &str, plugin: &str) -> Vec<String> {
    let plugin_dir = cache.join(marketplace).join(plugin);
    let Ok(versions) = std::fs::read_dir(&plugin_dir) else {
        return Vec::new();
    };
    // Several versions can sit side by side; only the latest is in play.
    let mut versions: Vec<PathBuf> = versions.flatten().map(|e| e.path()).collect();
    versions.sort();
    let Some(newest) = versions.last() else {
        return Vec::new();
    };

    let mut names: Vec<String> = std::fs::read_dir(newest.join("skills"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("SKILL.md").is_file())
        .filter_map(|path| path.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    names.sort();
    names
}

/// Everything a profile has, from both sources.
pub fn inventory(
    config: &Config,
    layout: &Layout,
    profile: &str,
) -> Result<Inventory, SkillError> {
    if !config.profiles.contains_key(profile) {
        return Err(SkillError::UnknownProfile(profile.to_string()));
    }
    let config_dir = config.config_dir(layout, profile);
    let skills_dir = config_dir.join("skills");

    let mut own = skills_in(&skills_dir, true);
    own.extend(skills_in(&config_dir.join(DISABLED_DIR), false));
    own.sort_by(|a, b| a.name.cmp(&b.name));

    let enabled_plugins = enabled_plugins_of(&config_dir);

    Ok(Inventory {
        // A symlinked skills directory is someone else's too, so switching a
        // skill off here would switch it off everywhere.
        shared: std::fs::symlink_metadata(&skills_dir)
            .map(|m| m.is_symlink())
            .unwrap_or(false),
        own,
        plugins: plugins_in(&config_dir, &enabled_plugins),
    })
}

/// The `enabledPlugins` map from a profile's settings.
pub fn enabled_plugins_of(config_dir: &Path) -> BTreeMap<String, bool> {
    let Ok(text) = std::fs::read_to_string(config_dir.join("settings.json")) else {
        return BTreeMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BTreeMap::new();
    };
    value
        .get("enabledPlugins")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(false)))
                .collect()
        })
        .unwrap_or_default()
}

/// Move a skill between `skills/` and `skills.disabled/`.
///
/// Nothing is deleted: Claude only scans `skills/`, so moving a directory out
/// of it is enough to switch a skill off, and moving it back restores it.
pub fn set_enabled(
    config: &Config,
    layout: &Layout,
    profile: &str,
    skill: &str,
    enabled: bool,
) -> Result<PathBuf, SkillError> {
    let config_dir = config.config_dir(layout, profile);
    let active = config_dir.join("skills").join(skill);
    let parked = config_dir.join(DISABLED_DIR).join(skill);

    let (from, to) = if enabled {
        (parked, active)
    } else {
        (active, parked)
    };

    if !from.is_dir() {
        return Err(SkillError::UnknownSkill(skill.to_string()));
    }
    if to.exists() {
        return Err(SkillError::AlreadyExists(skill.to_string()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    std::fs::rename(&from, &to).map_err(io(&from))?;
    Ok(to)
}

/// Remove a skill, keeping a copy alongside the profile.
///
/// cpx does not delete: the skill is moved out of the way, and where it came
/// from is the user's business to clean up when they are sure.
pub fn remove(
    config: &Config,
    layout: &Layout,
    profile: &str,
    skill: &str,
) -> Result<PathBuf, SkillError> {
    let config_dir = config.config_dir(layout, profile);
    for candidate in [
        config_dir.join("skills").join(skill),
        config_dir.join(DISABLED_DIR).join(skill),
    ] {
        if candidate.is_dir() {
            let removed = config_dir.join("skills.removed").join(skill);
            if removed.exists() {
                return Err(SkillError::AlreadyExists(skill.to_string()));
            }
            if let Some(parent) = removed.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::rename(&candidate, &removed).map_err(io(&candidate))?;
            return Ok(removed);
        }
    }
    Err(SkillError::UnknownSkill(skill.to_string()))
}

/// The result of toggling a plugin.
#[derive(Debug, Clone)]
pub struct PluginChange {
    /// The settings file that changed, when one did.
    pub settings_path: Option<PathBuf>,
    /// The config text to persist, when the change was declarative.
    pub config_text: Option<String>,
    /// True when the change lands on the next `cpx apply`.
    pub needs_apply: bool,
}

/// Enable or disable a whole plugin for one profile.
///
/// `enabledPlugins` is the only lever Claude offers here, and it works per
/// plugin rather than per skill, so this does what it can rather than
/// pretending individual plugin skills can be switched off.
pub fn set_plugin_enabled(
    config: &Config,
    layout: &Layout,
    profile: &str,
    key: &str,
    enabled: bool,
    config_text: &str,
) -> Result<PluginChange, SkillError> {
    use crate::config::{ResourceKey, ResourceMode};

    let profile_config = config
        .profiles
        .get(profile)
        .ok_or_else(|| SkillError::UnknownProfile(profile.to_string()))?;

    let merged = profile_config
        .resources
        .get(&ResourceKey::Settings)
        .map(|spec| spec.mode == ResourceMode::Merge)
        .unwrap_or(false);

    if merged {
        // Editing the generated file directly would be undone by the next
        // apply, so the change belongs in the patch it is generated from.
        let mut patch = profile_config
            .resources
            .get(&ResourceKey::Settings)
            .and_then(|spec| spec.patch.clone())
            .unwrap_or_else(|| serde_json::json!({}));

        patch
            .as_object_mut()
            .expect("a patch is an object")
            .entry("enabledPlugins")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| SkillError::UnknownSkill(key.to_string()))?
            .insert(key.to_string(), serde_json::json!(enabled));

        let edited = crate::config_edit::set_resource_patch(
            config_text,
            profile,
            ResourceKey::Settings.config_name(),
            Some(&patch),
        )
        .map_err(|e| SkillError::Io {
            path: PathBuf::from("config.toml"),
            source: std::io::Error::other(e.to_string()),
        })?;

        return Ok(PluginChange {
            settings_path: None,
            config_text: Some(edited),
            needs_apply: true,
        });
    }

    let path = config.config_dir(layout, profile).join("settings.json");
    let mut value: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| SkillError::Io {
            path: path.clone(),
            source: std::io::Error::other(e.to_string()),
        })?,
        Err(_) => serde_json::json!({}),
    };

    value
        .as_object_mut()
        .ok_or_else(|| SkillError::Io {
            path: path.clone(),
            source: std::io::Error::other("settings.json is not an object"),
        })?
        .entry("enabledPlugins")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| SkillError::Io {
            path: path.clone(),
            source: std::io::Error::other("enabledPlugins is not an object"),
        })?
        .insert(key.to_string(), serde_json::json!(enabled));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let mut text = serde_json::to_string_pretty(&value).expect("settings are serializable");
    text.push('\n');
    std::fs::write(&path, text).map_err(io(&path))?;

    Ok(PluginChange {
        settings_path: Some(path),
        config_text: None,
        needs_apply: false,
    })
}
