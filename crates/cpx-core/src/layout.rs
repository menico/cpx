//! Every path in the system derives from a `Layout`, so tests can build a
//! complete installation inside a temporary directory.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Layout {
    pub home: PathBuf,
    pub root: PathBuf,
}

impl Layout {
    /// The standard layout: `~/.claude-profiles`.
    pub fn new(home: impl Into<PathBuf>) -> Layout {
        let home = home.into();
        let root = home.join(".claude-profiles");
        Layout { home, root }
    }

    /// A layout with an explicit root, used by tests and `CPX_ROOT`.
    pub fn with_root(home: impl Into<PathBuf>, root: impl Into<PathBuf>) -> Layout {
        Layout {
            home: home.into(),
            root: root.into(),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn bindings_file(&self) -> PathBuf {
        self.root.join("bindings.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The per-profile shim directory placed on `PATH` by a bound `.envrc`.
    pub fn profile_bin_dir(&self, name: &str) -> PathBuf {
        self.profile_dir(name).join("bin")
    }

    /// The shim that makes a plain `claude` run under this profile.
    pub fn shim_path(&self, name: &str) -> PathBuf {
        self.profile_bin_dir(name).join("claude")
    }

    /// Whether `path` lies inside `dir`, used to protect `source_dir`.
    pub fn is_inside(path: &Path, dir: &Path) -> bool {
        path.starts_with(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new("/Users/tester")
    }

    #[test]
    fn root_defaults_to_dot_claude_profiles_under_home() {
        assert_eq!(layout().root, PathBuf::from("/Users/tester/.claude-profiles"));
    }

    #[test]
    fn profile_paths_hang_off_the_root() {
        let l = layout();
        assert_eq!(l.profile_dir("work"), l.root.join("work"));
        assert_eq!(l.shim_path("work"), l.root.join("work/bin/claude"));
        assert_eq!(l.config_file(), l.root.join("config.toml"));
        assert_eq!(l.bindings_file(), l.root.join("bindings.toml"));
        assert_eq!(l.state_file(), l.root.join("state.json"));
    }

    #[test]
    fn an_explicit_root_overrides_the_default() {
        let l = Layout::with_root("/Users/tester", "/tmp/cpx-test");
        assert_eq!(l.profile_dir("work"), PathBuf::from("/tmp/cpx-test/work"));
        assert_eq!(l.home, PathBuf::from("/Users/tester"));
    }

    #[test]
    fn is_inside_detects_containment() {
        assert!(Layout::is_inside(
            Path::new("/a/b/c"),
            Path::new("/a/b")
        ));
        assert!(!Layout::is_inside(Path::new("/a/bc"), Path::new("/a/b")));
        assert!(!Layout::is_inside(Path::new("/a"), Path::new("/a/b")));
    }
}
