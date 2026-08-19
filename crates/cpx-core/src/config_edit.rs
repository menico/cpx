//! Editing `config.toml` in place.
//!
//! Configuration edits deliberately sit outside the plan/execute system:
//! they touch exactly one file, and mixing them into materialization would
//! make both harder to reason about. Comments and key order are preserved,
//! because this is a file the user writes by hand.

use crate::config::Config;
use crate::error::ConfigError;
use toml_edit::{DocumentMut, Item, Table};

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("config is not valid TOML: {0}")]
    Toml(#[from] toml_edit::TomlError),

    #[error("a profile named `{0}` already exists")]
    ProfileExists(String),

    #[error("no profile named `{0}`")]
    UnknownProfile(String),

    #[error("`{0}` is not a field the app may set")]
    UnwritableField(String),

    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Reject names that cannot safely become a directory and a wrapper
/// filename. Checked here so an edit never writes a config that will not
/// load afterwards.
fn check_name(name: &str) -> Result<(), EditError> {
    let text = format!("version = 1\n[profiles.\"{}\"]\n", name.replace('"', "\\\""));
    Config::parse(&text, std::path::Path::new("/")).map_err(EditError::Config)?;
    Ok(())
}

/// A starter config, with `~/.claude` as the source.
pub fn starter_config(profiles: &[(String, String)]) -> String {
    let mut out = String::from(
        "# cpx — Claude profile manager\n\
         # Run `cpx apply` after editing. `cpx apply --dry-run` shows what would change.\n\
         version = 1\n\
         source_dir = \"~/.claude\"\n\
         bin_dir = \"~/.local/bin\"\n\
         \n\
         # How each resource reaches a profile:\n\
         #   link  — symlink to source_dir (edit once, every profile sees it)\n\
         #   copy  — seeded once, then yours to diverge\n\
         #   own   — profile-private, source never consulted\n\
         #   merge — regenerated from source JSON plus this profile's patch\n\
         [defaults.resources]\n\
         settings       = \"merge\"\n\
         settings_local = \"merge\"\n\
         \"CLAUDE.md\"    = \"copy\"\n\
         commands       = \"link\"\n\
         skills         = \"link\"\n\
         agents         = \"link\"\n\
         plugins        = \"link\"\n\
         hooks          = \"link\"\n\
         projects       = \"own\"\n",
    );
    for (name, description) in profiles {
        out.push_str(&format!(
            "\n[profiles.{name}]\ndescription = \"{description}\"\n"
        ));
    }
    out
}

fn profiles_table(doc: &mut DocumentMut) -> &mut Table {
    if !doc.contains_key("profiles") {
        let mut table = Table::new();
        table.set_implicit(true);
        doc["profiles"] = Item::Table(table);
    }
    doc["profiles"]
        .as_table_mut()
        .expect("profiles is a table")
}

/// Add an empty profile.
pub fn add_profile(text: &str, name: &str, description: &str) -> Result<String, EditError> {
    check_name(name)?;
    let mut doc: DocumentMut = text.parse()?;
    let profiles = profiles_table(&mut doc);
    if profiles.contains_key(name) {
        return Err(EditError::ProfileExists(name.to_string()));
    }
    let mut table = Table::new();
    table["description"] = toml_edit::value(description);
    profiles.insert(name, Item::Table(table));
    Ok(doc.to_string())
}

/// Remove a profile and everything under it.
pub fn remove_profile(text: &str, name: &str) -> Result<String, EditError> {
    let mut doc: DocumentMut = text.parse()?;
    let profiles = profiles_table(&mut doc);
    if profiles.remove(name).is_none() {
        return Err(EditError::UnknownProfile(name.to_string()));
    }
    Ok(doc.to_string())
}

/// Duplicate a profile's configuration under a new name. Credentials are not
/// copied: they live in the Keychain, keyed to the profile's own directory.
pub fn clone_profile(text: &str, from: &str, to: &str) -> Result<String, EditError> {
    check_name(to)?;
    let mut doc: DocumentMut = text.parse()?;
    let profiles = profiles_table(&mut doc);

    if profiles.contains_key(to) {
        return Err(EditError::ProfileExists(to.to_string()));
    }
    let source = profiles
        .get(from)
        .ok_or_else(|| EditError::UnknownProfile(from.to_string()))?
        .clone();

    profiles.insert(to, source);
    Ok(doc.to_string())
}
/// Fields the app may write as plain strings. Anything structured has its
/// own function, so a typo cannot turn a table into a string.
const WRITABLE_FIELDS: [&str; 3] = ["description", "model", "color"];

/// Re-parse an edited document, so every rule in `Config::parse` — unknown
/// resource keys, impossible modes, invalid names — applies to edits too.
fn validated(doc: DocumentMut) -> Result<String, EditError> {
    let text = doc.to_string();
    Config::parse(&text, std::path::Path::new("/")).map_err(EditError::Config)?;
    Ok(text)
}

fn profile_mut<'a>(doc: &'a mut DocumentMut, name: &str) -> Result<&'a mut Table, EditError> {
    let profiles = profiles_table(doc);
    if !profiles.contains_key(name) {
        return Err(EditError::UnknownProfile(name.to_string()));
    }
    profiles
        .get_mut(name)
        .and_then(|item| item.as_table_mut())
        .ok_or_else(|| EditError::UnknownProfile(name.to_string()))
}

/// Set a resource's mode for one profile.
pub fn set_resource_mode(
    text: &str,
    profile: &str,
    resource: &str,
    mode: &str,
) -> Result<String, EditError> {
    let mut doc: DocumentMut = text.parse()?;
    let table = profile_mut(&mut doc, profile)?;

    if !table.contains_key("resources") {
        let mut resources = Table::new();
        resources.set_implicit(false);
        table["resources"] = Item::Table(resources);
    }
    let resources = table["resources"]
        .as_table_mut()
        .ok_or_else(|| EditError::UnknownProfile(profile.to_string()))?;

    match resources.get(resource).and_then(|i| i.as_table_like()) {
        // An existing table form carries a patch worth keeping.
        Some(_) => {
            resources[resource]["mode"] = toml_edit::value(mode);
        }
        None => {
            resources[resource] = toml_edit::value(mode);
        }
    }

    validated(doc)
}

/// Set or clear a profile's `description` or `model`.
pub fn set_profile_field(
    text: &str,
    profile: &str,
    field: &str,
    value: Option<&str>,
) -> Result<String, EditError> {
    if !WRITABLE_FIELDS.contains(&field) {
        return Err(EditError::UnwritableField(field.to_string()));
    }
    let mut doc: DocumentMut = text.parse()?;
    let table = profile_mut(&mut doc, profile)?;

    match value {
        Some(value) => table[field] = toml_edit::value(value),
        None => {
            table.remove(field);
        }
    }
    validated(doc)
}

/// Write an adopted directory into the config as a new profile.
///
/// The resource modes come from what the directory already contains, so the
/// profile is inert on the next apply beyond its wrapper and shim.
pub fn add_adopted_profile(
    text: &str,
    adoption: &crate::adopt::Adoption,
) -> Result<String, EditError> {
    check_name(&adoption.name)?;
    let mut doc: DocumentMut = text.parse()?;
    let profiles = profiles_table(&mut doc);
    if profiles.contains_key(&adoption.name) {
        return Err(EditError::ProfileExists(adoption.name.clone()));
    }

    let mut table = Table::new();
    table["dir"] = toml_edit::value(adoption.dir.to_string_lossy().as_ref());

    let mut resources = Table::new();
    resources.set_implicit(false);
    for (key, mode) in &adoption.resources {
        resources[key.config_name()] = toml_edit::value(mode.as_str());
    }
    table["resources"] = Item::Table(resources);

    profiles.insert(&adoption.name, Item::Table(table));
    validated(doc)
}

/// Set (or clear) the merge patch for a resource.
///
/// A patch only means anything in `merge` mode, so setting one selects that
/// mode too rather than leaving a patch that nothing reads.
pub fn set_resource_patch(
    text: &str,
    profile: &str,
    resource: &str,
    patch: Option<&serde_json::Value>,
) -> Result<String, EditError> {
    let mut doc: DocumentMut = text.parse()?;
    let table = profile_mut(&mut doc, profile)?;

    if !table.contains_key("resources") {
        let mut resources = Table::new();
        resources.set_implicit(false);
        table["resources"] = Item::Table(resources);
    }
    let resources = table["resources"]
        .as_table_mut()
        .ok_or_else(|| EditError::UnknownProfile(profile.to_string()))?;

    match patch {
        None => {
            if let Some(entry) = resources.get_mut(resource) {
                if let Some(entry) = entry.as_table_like_mut() {
                    entry.remove("patch");
                }
            }
        }
        Some(patch) => {
            let mut entry = Table::new();
            entry["mode"] = toml_edit::value("merge");
            entry["patch"] = json_to_toml(patch)?;
            resources.insert(resource, Item::Table(entry));
        }
    }

    validated(doc)
}

/// Convert JSON into an inline TOML value, so a patch reads as one line.
fn json_to_toml(value: &serde_json::Value) -> Result<Item, EditError> {
    use serde_json::Value as J;
    use toml_edit::{Array, InlineTable, Value as T};

    fn convert(value: &serde_json::Value) -> Option<T> {
        Some(match value {
            J::Null => return None,
            J::Bool(b) => T::from(*b),
            J::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => T::from(i),
                (None, Some(f)) => T::from(f),
                _ => return None,
            },
            J::String(s) => T::from(s.as_str()),
            J::Array(items) => {
                let mut array = Array::new();
                for item in items {
                    array.push(convert(item)?);
                }
                T::Array(array)
            }
            J::Object(map) => {
                let mut table = InlineTable::new();
                for (key, item) in map {
                    if let Some(converted) = convert(item) {
                        table.insert(key, converted);
                    }
                }
                T::InlineTable(table)
            }
        })
    }

    convert(value)
        .map(Item::Value)
        .ok_or_else(|| EditError::UnwritableField("patch".to_string()))
}
