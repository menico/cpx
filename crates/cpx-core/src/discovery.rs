//! Finding the real Claude and Codex binaries.
//!
//! Wrappers exec the tool by absolute path, so resolving that path must never
//! land on one of cpx's own wrappers or shims — that is exactly the recursion
//! the absolute path exists to prevent.

use crate::config::Provider;
use crate::layout::Layout;
use crate::state::has_marker;
use std::path::{Path, PathBuf};

/// The first entry on `path` named after the provider's binary that cpx did
/// not generate.
pub fn resolve_binary(path_var: &str, layout: &Layout, provider: Provider) -> Option<PathBuf> {
    let name = provider.binary_name();
    path_var
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(|entry| Path::new(entry).join(name))
        .find(|candidate| is_executable(candidate) && !is_cpx_script(candidate, layout))
}

/// The first entry on `path` named `claude` that cpx did not generate.
pub fn resolve_claude_binary(path_var: &str, layout: &Layout) -> Option<PathBuf> {
    resolve_binary(path_var, layout, Provider::Claude)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Whether `candidate` is one of cpx's own scripts.
pub fn is_cpx_script(candidate: &Path, layout: &Layout) -> bool {
    // Anything under the profiles root is ours by construction, marker or
    // not — a shim is generated from the same template but may predate a
    // change to the marker.
    candidate.starts_with(&layout.root) || has_marker(candidate)
}
