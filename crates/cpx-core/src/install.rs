//! Loading an installation: layout, config, ownership state, bindings.
//!
//! Both the CLI and the desktop app start here, so neither has to know how
//! the pieces fit together.

use crate::binding::{BindError, Bindings};
use crate::config::Config;
use crate::discovery::resolve_claude_binary;
use crate::doctor::Ambient;
use crate::error::ConfigError;
use crate::layout::Layout;
use crate::state::State;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("no config at {path}. Run `cpx init` to create one.")]
    NotInitialised { path: PathBuf },

    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("HOME is not set, so cpx cannot find your configuration")]
    NoHome,

    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Binding(#[from] BindError),
}

#[derive(Debug)]
pub struct Install {
    pub layout: Layout,
    pub config: Config,
    pub state: State,
    pub bindings: Bindings,
}

/// The layout implied by the environment. `CPX_HOME` and `CPX_ROOT` exist so
/// the tools can be driven against a throwaway installation.
pub fn layout_from_env() -> Result<Layout, InstallError> {
    let home = match std::env::var("CPX_HOME") {
        Ok(home) => PathBuf::from(home),
        Err(_) => PathBuf::from(std::env::var("HOME").map_err(|_| InstallError::NoHome)?),
    };
    Ok(match std::env::var("CPX_ROOT") {
        Ok(root) => Layout::with_root(home, root),
        Err(_) => Layout::new(home),
    })
}

impl Install {
    /// Load everything. Reloaded per operation rather than cached, so an edit
    /// made in an editor is picked up without restarting anything.
    pub fn load(layout: Layout) -> Result<Install, InstallError> {
        let path = layout.config_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(InstallError::NotInitialised { path })
            }
            Err(source) => return Err(InstallError::Io { path, source }),
        };

        let config = Config::parse(&text, &layout.home)?;
        let state = State::load(&layout.state_file()).map_err(|source| InstallError::Io {
            path: layout.state_file(),
            source,
        })?;
        let bindings = Bindings::load(&layout.bindings_file())?;

        Ok(Install {
            layout,
            config,
            state,
            bindings,
        })
    }

    pub fn from_env() -> Result<Install, InstallError> {
        Install::load(layout_from_env()?)
    }

    /// Whether an installation exists at all.
    pub fn is_initialised(layout: &Layout) -> bool {
        layout.config_file().is_file()
    }

    pub fn config_text(&self) -> Result<String, InstallError> {
        let path = self.layout.config_file();
        std::fs::read_to_string(&path).map_err(|source| InstallError::Io { path, source })
    }

    /// Replace `config.toml` wholesale. Callers produce the text through
    /// `config_edit`, which validates before returning it.
    pub fn write_config(&self, text: &str) -> Result<(), InstallError> {
        let path = self.layout.config_file();
        std::fs::write(&path, text).map_err(|source| InstallError::Io { path, source })
    }

    pub fn save_state(&self) -> Result<(), InstallError> {
        self.state
            .save(&self.layout.state_file())
            .map_err(|source| InstallError::Io {
                path: self.layout.state_file(),
                source,
            })
    }

    pub fn save_bindings(&self) -> Result<(), InstallError> {
        Ok(self.bindings.save(&self.layout.bindings_file())?)
    }

    /// The real Claude binary, falling back to a bare `claude` so a wrapper is
    /// still written and `doctor` can report the problem.
    pub fn claude_binary(&self) -> PathBuf {
        resolve_claude_binary(&path_var(), &self.layout).unwrap_or_else(|| PathBuf::from("claude"))
    }

    pub fn ambient(&self) -> Ambient {
        Ambient {
            path: path_var(),
            claude_config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            direnv_present: which("direnv").is_some(),
            claude_binary: resolve_claude_binary(&path_var(), &self.layout),
        }
    }
}

fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// The first executable named `program` on `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var("PATH").ok()?.split(':').find_map(|dir| {
        let candidate = PathBuf::from(dir).join(program);
        std::fs::metadata(&candidate)
            .ok()
            .filter(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .map(|_| candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn loading_without_a_config_says_to_initialise() {
        let d = TempDir::new().unwrap();
        let err = Install::load(Layout::new(d.path())).unwrap_err();
        assert!(matches!(err, InstallError::NotInitialised { .. }));
        assert!(err.to_string().contains("cpx init"), "{err}");
    }

    #[test]
    fn a_broken_config_reports_the_parse_error_not_a_missing_file() {
        let d = TempDir::new().unwrap();
        let layout = Layout::new(d.path());
        std::fs::create_dir_all(&layout.root).unwrap();
        std::fs::write(layout.config_file(), "this is not toml = = =").unwrap();
        assert!(matches!(
            Install::load(layout).unwrap_err(),
            InstallError::Config(_)
        ));
    }

    #[test]
    fn is_initialised_follows_the_config_file() {
        let d = TempDir::new().unwrap();
        let layout = Layout::new(d.path());
        assert!(!Install::is_initialised(&layout));
        std::fs::create_dir_all(&layout.root).unwrap();
        std::fs::write(layout.config_file(), "version = 1\n").unwrap();
        assert!(Install::is_initialised(&layout));
    }

    #[test]
    fn state_and_bindings_start_empty_on_a_fresh_install() {
        let d = TempDir::new().unwrap();
        let layout = Layout::new(d.path());
        std::fs::create_dir_all(&layout.root).unwrap();
        std::fs::write(layout.config_file(), "version = 1\n[profiles.work]\n").unwrap();

        let install = Install::load(layout).unwrap();
        assert!(install.state.artifacts.is_empty());
        assert!(install.bindings.entries.is_empty());
        assert_eq!(install.config.profiles.len(), 1);
    }
}
