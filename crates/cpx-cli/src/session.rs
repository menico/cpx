//! Loading everything a command needs, from one place.
//!
//! The CLI and the desktop app both start from `cpx_core::install::Install`;
//! this is that, with the CLI's error wording.

use anyhow::{Context, Result};
use cpx_core::install::{layout_from_env, Install};
use cpx_core::layout::Layout;
use std::ops::{Deref, DerefMut};

pub struct Session(Install);

pub fn layout() -> Result<Layout> {
    Ok(layout_from_env()?)
}

impl Session {
    pub fn load() -> Result<Session> {
        let layout = layout()?;
        let config_path = layout.config_file();
        let install = Install::load(layout).with_context(|| {
            format!("could not load the installation at {}", config_path.display())
        })?;
        Ok(Session(install))
    }
}

impl Deref for Session {
    type Target = Install;
    fn deref(&self) -> &Install {
        &self.0
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Install {
        &mut self.0
    }
}
