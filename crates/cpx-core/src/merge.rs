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
}
