//! Deep-merge semantics for `merge`-mode resources.

use serde_json::Value;

/// Deep-merge `patch` into `base`, returning the result.
///
/// Objects merge key-by-key with the patch winning. Arrays and scalars are
/// replaced wholesale. A `null` in the patch deletes the key from the base.
pub fn deep_merge(base: &Value, patch: &Value) -> Value {
    let (Value::Object(base_map), Value::Object(patch_map)) = (base, patch) else {
        return patch.clone();
    };

    let mut out = base_map.clone();
    for (key, patch_value) in patch_map {
        if patch_value.is_null() {
            out.remove(key);
        } else if let Some(base_value) = out.get(key) {
            let merged = deep_merge(base_value, patch_value);
            out.insert(key.clone(), merged);
        } else {
            out.insert(key.clone(), patch_value.clone());
        }
    }
    Value::Object(out)
}

#[derive(Debug, thiserror::Error)]
pub enum TomlMergeError {
    #[error("not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("the merged document cannot be written as TOML: {0}")]
    Render(#[from] toml::ser::Error),

    #[error("the patch contains a value TOML cannot hold")]
    Unrepresentable,
}

/// Deep-merge a JSON patch into TOML text, with the same rules as
/// [`deep_merge`], and render the result as TOML.
///
/// Comments and formatting in the source are not preserved: the output is a
/// generated file, regenerated on every apply from the source and the patch,
/// exactly like a merged `settings.json`.
pub fn merge_toml(base_text: &str, patch: &Value) -> Result<String, TomlMergeError> {
    let base: toml::Value = if base_text.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(base_text)?
    };
    let base_json = serde_json::to_value(&base).map_err(|_| TomlMergeError::Unrepresentable)?;
    let merged = deep_merge(&base_json, patch);
    let merged: toml::Value =
        serde_json::from_value(merged).map_err(|_| TomlMergeError::Unrepresentable)?;
    Ok(toml::to_string_pretty(&merged)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_wins_on_conflicting_scalar() {
        let base = json!({"model": "opus", "theme": "dark"});
        let patch = json!({"model": "sonnet"});
        assert_eq!(
            deep_merge(&base, &patch),
            json!({"model": "sonnet", "theme": "dark"})
        );
    }

    #[test]
    fn nested_objects_merge_key_by_key() {
        let base = json!({"permissions": {"allow": ["a"], "deny": ["b"]}});
        let patch = json!({"permissions": {"deny": ["c"]}});
        assert_eq!(
            deep_merge(&base, &patch),
            json!({"permissions": {"allow": ["a"], "deny": ["c"]}})
        );
    }

    #[test]
    fn arrays_are_replaced_not_concatenated() {
        let base = json!({"allow": ["one", "two"]});
        let patch = json!({"allow": ["three"]});
        assert_eq!(deep_merge(&base, &patch), json!({"allow": ["three"]}));
    }

    #[test]
    fn null_in_patch_deletes_the_key() {
        let base = json!({"model": "opus", "statusLine": {"type": "command"}});
        let patch = json!({"statusLine": null});
        assert_eq!(deep_merge(&base, &patch), json!({"model": "opus"}));
    }

    #[test]
    fn null_for_absent_key_is_not_reintroduced() {
        let base = json!({"model": "opus"});
        let patch = json!({"statusLine": null});
        assert_eq!(deep_merge(&base, &patch), json!({"model": "opus"}));
    }

    #[test]
    fn patch_key_absent_from_base_is_added() {
        let base = json!({"model": "opus"});
        let patch = json!({"statusLine": {"type": "command", "command": "cpx prompt"}});
        assert_eq!(
            deep_merge(&base, &patch),
            json!({"model": "opus", "statusLine": {"type": "command", "command": "cpx prompt"}})
        );
    }

    #[test]
    fn non_object_base_is_replaced_by_object_patch() {
        assert_eq!(deep_merge(&json!("scalar"), &json!({"a": 1})), json!({"a": 1}));
    }

    #[test]
    fn toml_merge_overrides_a_scalar_and_keeps_tables() {
        let base = "model = \"gpt-5\"\n\n[mcp_servers.figma]\ncommand = \"figma-mcp\"\n";
        let out = merge_toml(base, &json!({"model": "gpt-5.5"})).unwrap();
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(parsed["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(parsed["mcp_servers"]["figma"]["command"].as_str(), Some("figma-mcp"));
    }

    #[test]
    fn toml_merge_adds_nested_tables_from_the_patch() {
        let out = merge_toml("model = \"gpt-5\"\n", &json!({"projects": {"/srv/app": {"trust_level": "trusted"}}})).unwrap();
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(parsed["projects"]["/srv/app"]["trust_level"].as_str(), Some("trusted"));
        assert_eq!(parsed["model"].as_str(), Some("gpt-5"));
    }

    #[test]
    fn toml_merge_deletes_on_null() {
        let out = merge_toml("model = \"gpt-5\"\nnotify = [\"x\"]\n", &json!({"notify": null})).unwrap();
        let parsed: toml::Value = toml::from_str(&out).unwrap();
        assert!(parsed.get("notify").is_none(), "{out}");
    }

    #[test]
    fn toml_merge_of_an_empty_source_is_just_the_patch() {
        let out = merge_toml("", &json!({"model": "gpt-5.5"})).unwrap();
        assert_eq!(out.trim(), "model = \"gpt-5.5\"");
    }

    #[test]
    fn toml_merge_reports_a_broken_source() {
        assert!(matches!(merge_toml("= = =", &json!({})), Err(TomlMergeError::Parse(_))));
    }
}
