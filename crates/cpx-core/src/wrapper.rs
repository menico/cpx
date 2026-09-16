//! Generation of the `claude-<profile>` / `codex-<profile>` wrapper and the
//! per-profile shim.

use crate::config::{Profile, Provider};
use std::path::Path;

/// Claude Code subcommands that manage the installation itself. These exec
/// straight through: a `--model` default or an `--add-dir` would be a syntax
/// error for them.
pub const PASSTHROUGH_SUBCOMMANDS: &[&str] = &[
    "mcp",
    "auth",
    "doctor",
    "install",
    "setup-token",
    "update",
    "upgrade",
    "agents",
    "auto-mode",
    "plugin",
    "plugins",
];

/// The same for Codex: subcommands that take no session flags.
pub const CODEX_PASSTHROUGH_SUBCOMMANDS: &[&str] = &[
    "login",
    "logout",
    "mcp",
    "mcp-server",
    "app-server",
    "apply",
    "completion",
    "features",
    "debug",
    "cloud",
    "sandbox",
    "help",
];

pub fn passthrough_subcommands(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => PASSTHROUGH_SUBCOMMANDS,
        Provider::Codex => CODEX_PASSTHROUGH_SUBCOMMANDS,
    }
}

pub struct WrapperContext<'a> {
    pub name: &'a str,
    pub profile: &'a Profile,
    pub profile_dir: &'a Path,
    /// The real Claude or Codex binary, resolved at plan time and exec'd by
    /// absolute path so a wrapper in `bin_dir` can never recurse into itself.
    pub binary: &'a Path,
}

/// Quote a value for safe interpolation into shell source.
///
/// Everything cpx interpolates — profile names, paths, env values — comes
/// from a config file the user edits by hand, so a stray `$(...)` must end up
/// as literal text rather than a command substitution.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn quote_path(path: &Path) -> String {
    sh_quote(&path.to_string_lossy())
}

/// The `<prefix><profile>` wrapper installed into `bin_dir`.
pub fn wrapper_script(ctx: &WrapperContext) -> String {
    use std::fmt::Write;
    let provider = ctx.profile.provider;
    let mut s = String::new();

    writeln!(s, "#!/usr/bin/env bash").unwrap();
    writeln!(s, "{}", crate::state::MARKER).unwrap();
    if ctx.profile.description.is_empty() {
        writeln!(s, "# Profile: {} ({})", ctx.name, provider.display_name()).unwrap();
    } else {
        writeln!(
            s,
            "# Profile: {} ({}) — {}",
            ctx.name,
            provider.display_name(),
            ctx.profile.description
        )
        .unwrap();
    }
    writeln!(s, "#\n# Edit ~/.claude-profiles/config.toml and run `cpx apply`;").unwrap();
    writeln!(s, "# changes made here are overwritten.").unwrap();
    writeln!(s, "set -euo pipefail\n").unwrap();

    let prefixes = provider.scrubbed_env_prefixes();
    writeln!(
        s,
        "# Drop inherited {} variables so a parent shell cannot\n# redirect this profile at a different account.",
        prefixes
            .iter()
            .map(|p| format!("{p}*"))
            .collect::<Vec<_>>()
            .join("/")
    )
    .unwrap();
    writeln!(s, "while IFS= read -r __cpx_var; do").unwrap();
    writeln!(s, "  unset \"$__cpx_var\"").unwrap();
    writeln!(
        s,
        "done < <(compgen -v | grep -E '^({})' || true)\n",
        prefixes.join("|")
    )
    .unwrap();

    writeln!(
        s,
        "export {}={}",
        provider.home_env_var(),
        quote_path(ctx.profile_dir)
    )
    .unwrap();
    writeln!(s, "export {}={}", provider.profile_env_var(), sh_quote(ctx.name)).unwrap();
    // Read by a generated statusline badge, so one badge script can colour
    // itself for whichever profile is running.
    if let Some(color) = &ctx.profile.color {
        writeln!(s, "export CPX_PROFILE_COLOR={}", sh_quote(color)).unwrap();
    }
    for (key, value) in &ctx.profile.env {
        writeln!(s, "export {}={}", key, sh_quote(value)).unwrap();
    }
    writeln!(s).unwrap();

    writeln!(s, "__cpx_bin={}\n", quote_path(ctx.binary)).unwrap();

    writeln!(
        s,
        "# Subcommands that manage the installation take no session flags."
    )
    .unwrap();
    writeln!(s, "case \"${{1:-}}\" in").unwrap();
    writeln!(
        s,
        "  {}) exec \"$__cpx_bin\" \"$@\" ;;",
        passthrough_subcommands(provider).join("|")
    )
    .unwrap();
    writeln!(s, "esac\n").unwrap();

    let mut flags = String::new();
    for dir in &ctx.profile.add_dirs {
        flags.push_str(&format!(" --add-dir {}", quote_path(dir)));
    }

    // Codex spells the flag `-m` as well as `--model`; Claude only the latter.
    let model_patterns = match provider {
        Provider::Claude => "--model|--model=*",
        Provider::Codex => "-m|--model|--model=*",
    };

    match &ctx.profile.model {
        None => {
            writeln!(s, "exec \"$__cpx_bin\"{flags} \"$@\"").unwrap();
        }
        Some(model) => {
            // Arrays are avoided on purpose: macOS still ships bash 3.2,
            // where expanding an empty array under `set -u` is an error.
            writeln!(s, "# A --model on the command line wins over the profile default.").unwrap();
            writeln!(s, "__cpx_has_model=false").unwrap();
            writeln!(s, "for __cpx_arg in \"$@\"; do").unwrap();
            writeln!(s, "  case \"$__cpx_arg\" in").unwrap();
            writeln!(s, "    {model_patterns}) __cpx_has_model=true; break ;;").unwrap();
            writeln!(s, "  esac").unwrap();
            writeln!(s, "done\n").unwrap();
            writeln!(s, "if [ \"$__cpx_has_model\" = true ]; then").unwrap();
            writeln!(s, "  exec \"$__cpx_bin\"{flags} \"$@\"").unwrap();
            writeln!(s, "else").unwrap();
            writeln!(
                s,
                "  exec \"$__cpx_bin\"{flags} --model {} \"$@\"",
                sh_quote(model)
            )
            .unwrap();
            writeln!(s, "fi").unwrap();
        }
    }

    s
}

/// The `<profile>/bin/claude` (or `/codex`) shim placed on PATH by a bound
/// `.envrc`.
///
/// Deliberately the same script as the wrapper: a plain `claude` inside a
/// bound directory must not quietly behave differently from `claude-<name>`.
pub fn shim_script(ctx: &WrapperContext) -> String {
    wrapper_script(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::state::MARKER;
    use std::path::PathBuf;

    fn ctx_for(toml: &str, name: &str) -> (Config, PathBuf, PathBuf) {
        let cfg = Config::parse(toml, Path::new("/Users/tester")).unwrap();
        (
            cfg,
            PathBuf::from("/Users/tester/.claude-profiles").join(name),
            PathBuf::from("/Users/tester/.local/bin/claude"),
        )
    }

    fn script(toml: &str, name: &str) -> String {
        let (cfg, dir, bin) = ctx_for(toml, name);
        wrapper_script(&WrapperContext {
            name,
            profile: &cfg.profiles[name],
            profile_dir: &dir,
            binary: &bin,
        })
    }

    const PLAIN: &str = "version = 1\n[profiles.work]\ndescription = \"Company\"\n";

    #[test]
    fn quoting_wraps_in_single_quotes() {
        assert_eq!(sh_quote("plain"), "'plain'");
    }

    #[test]
    fn quoting_neutralises_an_embedded_single_quote() {
        assert_eq!(sh_quote("it's"), r#"'it'\''s'"#);
    }

    #[test]
    fn quoting_neutralises_command_substitution() {
        assert_eq!(sh_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn wrapper_opens_with_a_shebang_and_the_marker() {
        let s = script(PLAIN, "work");
        let mut lines = s.lines();
        assert_eq!(lines.next().unwrap(), "#!/usr/bin/env bash");
        assert_eq!(lines.next().unwrap(), MARKER);
    }

    #[test]
    fn wrapper_exports_the_config_dir_and_profile_name() {
        let s = script(PLAIN, "work");
        assert!(
            s.contains("export CLAUDE_CONFIG_DIR='/Users/tester/.claude-profiles/work'"),
            "{s}"
        );
        assert!(s.contains("export CLAUDE_PROFILE='work'"), "{s}");
    }

    #[test]
    fn wrapper_execs_the_resolved_binary_never_a_bare_claude() {
        let s = script(PLAIN, "work");
        assert!(s.contains("'/Users/tester/.local/bin/claude'"), "{s}");
        assert!(
            !s.lines().any(|l| l.trim_start().starts_with("exec claude")),
            "a bare `exec claude` would recurse through bin_dir: {s}"
        );
    }

    #[test]
    fn wrapper_unsets_inherited_claude_and_anthropic_variables() {
        let s = script(PLAIN, "work");
        assert!(s.contains("CLAUDE_"), "{s}");
        assert!(s.contains("ANTHROPIC_"), "{s}");
        assert!(s.contains("unset"), "{s}");
        assert!(!s.contains("CODEX_|"), "{s}");
    }

    #[test]
    fn wrapper_passes_through_installation_subcommands() {
        let s = script(PLAIN, "work");
        for sub in PASSTHROUGH_SUBCOMMANDS {
            assert!(s.contains(sub), "missing passthrough for {sub}: {s}");
        }
    }

    #[test]
    fn add_dirs_become_add_dir_arguments() {
        let s = script(
            "version = 1\n[profiles.work]\nadd_dirs = [\"~/Work/a\", \"/srv/b\"]\n",
            "work",
        );
        assert!(s.contains("--add-dir '/Users/tester/Work/a'"), "{s}");
        assert!(s.contains("--add-dir '/srv/b'"), "{s}");
    }

    #[test]
    fn a_configured_model_becomes_a_conditional_default() {
        let s = script("version = 1\n[profiles.work]\nmodel = \"sonnet\"\n", "work");
        assert!(s.contains("sonnet"), "{s}");
        assert!(s.contains("--model"), "{s}");
    }

    #[test]
    fn no_model_configured_means_no_model_flag() {
        let s = script(PLAIN, "work");
        assert!(!s.contains("--model"), "{s}");
    }

    #[test]
    fn env_vars_are_exported_in_a_stable_order() {
        let s = script(
            "version = 1\n[profiles.work]\nenv = { ZED = \"1\", ALPHA = \"2\" }\n",
            "work",
        );
        let alpha = s.find("export ALPHA=").expect("ALPHA exported");
        let zed = s.find("export ZED=").expect("ZED exported");
        assert!(alpha < zed, "env vars should be sorted: {s}");
    }

    #[test]
    fn env_values_are_quoted_against_injection() {
        let s = script(
            "version = 1\n[profiles.work]\nenv = { EVIL = \"$(touch /tmp/pwned)\" }\n",
            "work",
        );
        assert!(s.contains("export EVIL='$(touch /tmp/pwned)'"), "{s}");
    }

    #[test]
    fn shim_execs_the_resolved_binary_with_the_profile_environment() {
        let (cfg, dir, bin) = ctx_for(PLAIN, "work");
        let s = shim_script(&WrapperContext {
            name: "work",
            profile: &cfg.profiles["work"],
            profile_dir: &dir,
            binary: &bin,
        });
        assert!(s.starts_with("#!/usr/bin/env bash"), "{s}");
        assert!(s.contains(MARKER), "{s}");
        assert!(s.contains("export CLAUDE_CONFIG_DIR='/Users/tester/.claude-profiles/work'"), "{s}");
        assert!(s.contains("'/Users/tester/.local/bin/claude'"), "{s}");
    }

    #[test]
    fn shim_behaves_identically_to_the_wrapper() {
        let (cfg, dir, bin) = ctx_for(
            "version = 1\n[profiles.work]\nmodel = \"sonnet\"\nadd_dirs = [\"/srv/b\"]\n",
            "work",
        );
        let ctx = WrapperContext {
            name: "work",
            profile: &cfg.profiles["work"],
            profile_dir: &dir,
            binary: &bin,
        };
        assert_eq!(
            shim_script(&ctx),
            wrapper_script(&ctx),
            "`claude` inside a bound directory must behave exactly like `claude-work`"
        );
    }
}

#[cfg(test)]
mod codex_tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    const CODEX: &str = "version = 1\n[profiles.work]\nprovider = \"codex\"\nmodel = \"gpt-5.5\"\nadd_dirs = [\"/srv/b\"]\nenv = { OPENAI_BASE_URL = \"https://proxy\" }\n";

    fn script() -> String {
        let cfg = Config::parse(CODEX, Path::new("/Users/tester")).unwrap();
        wrapper_script(&WrapperContext {
            name: "work",
            profile: &cfg.profiles["work"],
            profile_dir: &PathBuf::from("/Users/tester/.claude-profiles/work"),
            binary: &PathBuf::from("/opt/homebrew/bin/codex"),
        })
    }

    #[test]
    fn a_codex_wrapper_points_codex_home_at_the_profile() {
        let s = script();
        assert!(s.contains("export CODEX_HOME='/Users/tester/.claude-profiles/work'"), "{s}");
        assert!(s.contains("export CODEX_PROFILE='work'"), "{s}");
        assert!(!s.contains("CLAUDE_CONFIG_DIR"), "{s}");
    }

    #[test]
    fn a_codex_wrapper_scrubs_codex_and_openai_variables_but_keeps_its_own_env() {
        let s = script();
        assert!(s.contains("grep -E '^(CODEX_|OPENAI_)'"), "{s}");
        assert!(!s.contains("ANTHROPIC_"), "{s}");
        // The profile's own variables are exported after the scrub.
        let scrub = s.find("grep -E '^(CODEX_|OPENAI_)'").unwrap();
        let export = s.find("export OPENAI_BASE_URL='https://proxy'").unwrap();
        assert!(scrub < export, "{s}");
    }

    #[test]
    fn a_codex_wrapper_execs_the_codex_binary_and_passes_through_login() {
        let s = script();
        assert!(s.contains("__cpx_bin='/opt/homebrew/bin/codex'"), "{s}");
        assert!(s.contains("login|logout|"), "{s}");
        assert!(!s.contains("|auth|"), "{s}");
    }

    #[test]
    fn a_codex_wrapper_honours_the_short_model_flag() {
        let s = script();
        assert!(s.contains("-m|--model|--model=*"), "{s}");
        assert!(s.contains("--model 'gpt-5.5'"), "{s}");
        assert!(s.contains("--add-dir '/srv/b'"), "{s}");
    }

    #[test]
    fn a_codex_wrapper_names_the_product_in_its_header() {
        let s = script();
        assert!(s.contains("# Profile: work (Codex)"), "{s}");
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;

    fn script(toml: &str) -> String {
        let cfg = Config::parse(toml, Path::new("/Users/tester")).unwrap();
        wrapper_script(&WrapperContext {
            name: "work",
            profile: &cfg.profiles["work"],
            profile_dir: &PathBuf::from("/p/work"),
            binary: &PathBuf::from("/usr/bin/claude"),
        })
    }

    #[test]
    fn the_wrapper_exports_the_identity_colour_for_the_statusline_badge() {
        let s = script("version = 1\n[profiles.work]\ncolor = \"#5c8dff\"\n");
        assert!(s.contains("export CPX_PROFILE_COLOR='#5c8dff'"), "{s}");
    }

    #[test]
    fn a_profile_without_a_colour_exports_nothing_extra() {
        let s = script("version = 1\n[profiles.work]\n");
        assert!(!s.contains("CPX_PROFILE_COLOR"), "{s}");
    }
}
