//! Loading everything a command needs, from one place.

use anyhow::{Context, Result};
use cpx_core::binding::Bindings;
use cpx_core::config::Config;
use cpx_core::discovery::resolve_claude_binary;
use cpx_core::doctor::Ambient;
use cpx_core::layout::Layout;
use cpx_core::state::State;
use std::path::PathBuf;

pub struct Session {
    pub layout: Layout,
    pub config: Config,
    pub state: State,
    pub bindings: Bindings,
}

/// `CPX_HOME` and `CPX_ROOT` exist so the CLI can be driven against a
/// throwaway installation, in tests and when trying things out.
pub fn home() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CPX_HOME") {
        return Ok(PathBuf::from(dir));
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME is not set, so cpx cannot find your configuration")
}

pub fn layout() -> Result<Layout> {
    let home = home()?;
    Ok(match std::env::var("CPX_ROOT") {
        Ok(root) => Layout::with_root(home, root),
        Err(_) => Layout::new(home),
    })
}

impl Session {
    pub fn load() -> Result<Session> {
        let layout = layout()?;
        let config_path = layout.config_file();
        let text = std::fs::read_to_string(&config_path).with_context(|| {
            format!(
                "no config at {}. Run `cpx init` to create one.",
                config_path.display()
            )
        })?;
        let config = Config::parse(&text, &layout.home)
            .with_context(|| format!("could not read {}", config_path.display()))?;

        Ok(Session {
            state: State::load(&layout.state_file())?,
            bindings: Bindings::load(&layout.bindings_file())?,
            config,
            layout,
        })
    }

    pub fn save_state(&self) -> Result<()> {
        self.state.save(&self.layout.state_file())?;
        Ok(())
    }

    pub fn save_bindings(&self) -> Result<()> {
        self.bindings.save(&self.layout.bindings_file())?;
        Ok(())
    }

    /// The real Claude binary, or a bare `claude` if none was found — the
    /// wrapper is still written, and `cpx doctor` reports the problem.
    pub fn claude_binary(&self) -> PathBuf {
        resolve_claude_binary(&std::env::var("PATH").unwrap_or_default(), &self.layout)
            .unwrap_or_else(|| PathBuf::from("claude"))
    }

    pub fn ambient(&self) -> Ambient {
        Ambient {
            path: std::env::var("PATH").unwrap_or_default(),
            claude_config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
            direnv_present: which("direnv").is_some(),
            claude_binary: resolve_claude_binary(
                &std::env::var("PATH").unwrap_or_default(),
                &self.layout,
            ),
        }
    }
}

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
