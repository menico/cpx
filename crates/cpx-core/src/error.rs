//! Error types for cpx-core.
//!
//! Planning errors and execution errors are deliberately distinct: a plan
//! that cannot be computed is a configuration problem, while a plan that
//! fails part-way through reports which actions completed.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("unknown resource key `{key}` (known keys: {known})")]
    UnknownResourceKey { key: String, known: String },

    #[error("unknown resource mode `{mode}` for `{key}` (known modes: link, copy, own, merge)")]
    UnknownResourceMode { key: String, mode: String },

    #[error("resource `{key}` sets a patch but its mode is `{mode}`; only `merge` accepts a patch")]
    PatchOnNonMergeMode { key: String, mode: String },

    #[error("resource `{key}` cannot use mode `{mode}`: {reason}")]
    IncompatibleMode {
        key: String,
        mode: String,
        reason: String,
    },

    #[error("profile name `{name}` is invalid: {reason}")]
    InvalidProfileName { name: String, reason: String },

    #[error("no profile named `{0}`")]
    UnknownProfile(String),

    #[error("config file not found at {0}")]
    NotFound(PathBuf),

    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
