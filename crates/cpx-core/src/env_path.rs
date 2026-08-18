//! Recovering the user's `PATH`.
//!
//! A macOS app launched from Finder inherits a stub environment, not the one
//! a terminal has. For cpx that is not cosmetic: without the real `PATH` it
//! cannot find the Claude binary, and a wrapper written without an absolute
//! path loses the protection against a wrapper directory shadowing the real
//! binary. So the app asks the login shell what `PATH` should be.

/// Combine two `PATH` values, keeping `first`'s ordering and appending
/// anything from `second` that is not already there.
pub fn merge_paths(first: &str, second: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    first
        .split(':')
        .chain(second.split(':'))
        .filter(|entry| !entry.is_empty())
        .filter(|entry| seen.insert(entry.to_string()))
        .collect::<Vec<_>>()
        .join(":")
}

/// Ask the user's login shell for its `PATH`.
///
/// Returns `None` when there is no usable shell or it does not answer; the
/// caller keeps whatever it already had.
pub fn login_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // `-l` so profile files are read, `-i` because many people set PATH in
    // .zshrc rather than .zprofile. Output is printed without a newline so
    // there is nothing to trim but stray shell chatter.
    let output = std::process::Command::new(&shell)
        .args(["-lic", "printf %s \"$PATH\""])
        .env_remove("PATH")
        .output()
        .ok()?;

    let path = String::from_utf8_lossy(&output.stdout);
    // An interactive shell may print banners; the PATH is the last line.
    let path = path.lines().last().unwrap_or("").trim();
    if path.contains('/') {
        Some(path.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_keeps_the_first_paths_order() {
        assert_eq!(merge_paths("/a:/b", "/c"), "/a:/b:/c");
    }

    #[test]
    fn merging_does_not_repeat_an_entry() {
        assert_eq!(merge_paths("/a:/b", "/b:/c"), "/a:/b:/c");
    }

    #[test]
    fn merging_drops_repeats_within_one_side_too() {
        assert_eq!(merge_paths("/a:/a:/b", "/b"), "/a:/b");
    }

    #[test]
    fn merging_ignores_empty_entries() {
        assert_eq!(merge_paths("/a::/b:", ":/c"), "/a:/b:/c");
    }

    #[test]
    fn merging_with_an_empty_side_returns_the_other() {
        assert_eq!(merge_paths("", "/a:/b"), "/a:/b");
        assert_eq!(merge_paths("/a:/b", ""), "/a:/b");
    }

    #[test]
    fn the_login_shell_reports_a_path_containing_the_usual_places() {
        // Runs the real shell: the point of this code is that it works on a
        // real machine, and a mocked shell would prove nothing.
        let Some(path) = login_path() else {
            return; // no shell available (CI); nothing to assert
        };
        assert!(path.contains('/'), "got {path:?}");
        assert!(!path.trim().is_empty());
        assert!(
            path.split(':').any(|p| p == "/usr/bin"),
            "a login shell PATH should include /usr/bin: {path}"
        );
    }
}
