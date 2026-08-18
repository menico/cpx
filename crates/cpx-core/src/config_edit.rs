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
