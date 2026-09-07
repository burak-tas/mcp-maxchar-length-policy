//! Argument character-limit enforcement.
//!
//! Two independent checks run in order:
//!   1. Per-field check: each string field (recursively) is tested against its
//!      field-specific limit (from `fieldLimits`) or the `defaultFieldMaxChars`
//!      fallback.  A field that has no applicable limit is skipped.
//!   2. Total check: the sum of all string character counts in the arguments
//!      object is tested against `maxTotalArgumentChars` when configured.
//!
//! Error messages report the field path and the limit value — never the
//! offending string content, which may be prompt-injection material or PII.

use serde_json::Value;

use crate::config::PolicyConfig;

/// Result of the character-limit check.  `Ok(())` = all limits satisfied.
/// `Err(msg)` = first violated limit; the message is safe to surface to the
/// MCP client (no caller-supplied content).
pub fn check_argument_chars(args: &Value, config: &PolicyConfig) -> Result<(), String> {
    // Walk every string in the arguments tree.
    // Per-field limits apply to the top-level key of the field that exceeds the
    // limit; we carry the top-level key name down through the recursion.
    check_value(args, args, "<root>", None, config)?;

    // Total character count check.
    if let Some(max_total) = config.max_total_argument_chars {
        let total = count_total_chars(args);
        if total > max_total {
            return Err(format!(
                "total argument characters ({total}) exceed the configured limit of {max_total}"
            ));
        }
    }

    Ok(())
}

/// Recursively check `value` (which lives at `path` inside the root args
/// object).  `top_key` is the key name one level below `<root>` — i.e. the
/// name used to look up per-field limits.  When `top_key` is `None` we are
/// at the root object itself.
fn check_value(
    root: &Value,
    value: &Value,
    path: &str,
    top_key: Option<&str>,
    config: &PolicyConfig,
) -> Result<(), String> {
    match value {
        Value::String(s) => {
            let len = s.chars().count();
            // Determine the effective limit for this field.
            let effective_limit = effective_field_limit(top_key, config);
            if let Some(limit) = effective_limit {
                if len > limit {
                    return Err(format!(
                        "argument field '{path}' has {len} character(s), which exceeds the limit of {limit}"
                    ));
                }
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = child_path(path, key);
                // The top_key is the first segment below <root>.
                let child_top_key = if path == "<root>" {
                    Some(key.as_str())
                } else {
                    top_key
                };
                check_value(root, child, &child_path, child_top_key, config)?;
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let item_path = format!("{path}[{i}]");
                check_value(root, item, &item_path, top_key, config)?;
            }
        }
        // Non-string scalars and null impose no character limit.
        _ => {}
    }
    Ok(())
}

/// Determine the per-field character limit for `top_key`.
///
/// Lookup order:
///   1. An exact match in `fieldLimits` for this key.
///   2. A wildcard `"*"` entry in `fieldLimits`.
///   3. `defaultFieldMaxChars` (when non-zero).
///   4. `None` — no limit applies.
fn effective_field_limit(top_key: Option<&str>, config: &PolicyConfig) -> Option<usize> {
    let key = match top_key {
        Some(k) => k,
        None => return config.default_field_max_chars,
    };

    // Exact match first.
    if let Some(lim) = config.field_limits.iter().find(|(f, _)| f == key) {
        return Some(lim.1);
    }
    // Wildcard.
    if let Some(lim) = config.field_limits.iter().find(|(f, _)| f == "*") {
        return Some(lim.1);
    }
    // Fall back to the default.
    config.default_field_max_chars
}

/// Sum the `chars().count()` of every `Value::String` in the tree.
fn count_total_chars(value: &Value) -> usize {
    match value {
        Value::String(s) => s.chars().count(),
        Value::Object(map) => map.values().map(count_total_chars).sum(),
        Value::Array(items) => items.iter().map(count_total_chars).sum(),
        _ => 0,
    }
}

fn child_path(parent: &str, key: &str) -> String {
    if parent == "<root>" {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PolicyConfig;
    use serde_json::json;

    fn config_default_only(default_max: usize) -> PolicyConfig {
        PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(default_max),
            field_limits: vec![],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        }
    }

    fn config_field_limit(field: &str, max: usize) -> PolicyConfig {
        PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(4096),
            field_limits: vec![(field.to_string(), max)],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        }
    }

    fn config_total_only(max_total: usize) -> PolicyConfig {
        PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: None,
            field_limits: vec![],
            max_total_argument_chars: Some(max_total),
            max_request_bytes: 1_048_576,
        }
    }

    // -- per-field: default limit ------------------------------------------

    #[test]
    fn short_string_passes_default_limit() {
        let cfg = config_default_only(10);
        assert!(check_argument_chars(&json!({"a": "hello"}), &cfg).is_ok());
    }

    #[test]
    fn string_at_exact_limit_passes() {
        let cfg = config_default_only(5);
        assert!(check_argument_chars(&json!({"a": "12345"}), &cfg).is_ok());
    }

    #[test]
    fn string_one_over_default_limit_fails() {
        let cfg = config_default_only(5);
        let err = check_argument_chars(&json!({"a": "123456"}), &cfg).unwrap_err();
        assert!(err.contains("'a'"), "error must name the field, got: {err}");
        assert!(err.contains("6"), "error must report the length, got: {err}");
        assert!(err.contains("5"), "error must report the limit, got: {err}");
        // Must never echo the offending value.
        assert!(!err.contains("123456"), "error must not echo value, got: {err}");
    }

    #[test]
    fn no_limits_configured_allows_any_string() {
        let cfg = PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: None,
            field_limits: vec![],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        };
        let huge = "x".repeat(1_000_000);
        assert!(check_argument_chars(&json!({"a": huge}), &cfg).is_ok());
    }

    // -- per-field: specific field limit overrides default -----------------

    #[test]
    fn specific_field_limit_overrides_default() {
        let cfg = PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(1000),
            field_limits: vec![("description".to_string(), 10)],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        };
        // description exceeds its field-specific limit (10), not the default (1000).
        let err = check_argument_chars(
            &json!({"description": "this is way too long for the field limit"}),
            &cfg,
        )
        .unwrap_err();
        assert!(err.contains("'description'"), "got: {err}");
        assert!(err.contains("10"), "limit should be 10, got: {err}");
    }

    #[test]
    fn other_field_falls_back_to_default_when_specific_limit_set() {
        let cfg = config_field_limit("description", 10);
        // "name" is not in fieldLimits → falls back to default (4096).
        assert!(
            check_argument_chars(&json!({"name": "a".repeat(100)}), &cfg).is_ok()
        );
    }

    // -- per-field: wildcard "*" -------------------------------------------

    #[test]
    fn wildcard_applies_when_no_exact_match() {
        let cfg = PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(10000),
            field_limits: vec![("*".to_string(), 5)],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        };
        let err = check_argument_chars(&json!({"anything": "123456"}), &cfg).unwrap_err();
        assert!(err.contains("'anything'"), "got: {err}");
        assert!(err.contains("5"), "wildcard limit 5, got: {err}");
    }

    #[test]
    fn exact_match_takes_priority_over_wildcard() {
        let cfg = PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(10000),
            field_limits: vec![
                ("description".to_string(), 20),
                ("*".to_string(), 5),
            ],
            max_total_argument_chars: None,
            max_request_bytes: 1_048_576,
        };
        // "description" has a 20-char specific limit; 15-char value passes.
        assert!(
            check_argument_chars(&json!({"description": "fifteen chars!!"}), &cfg).is_ok()
        );
        // "other" has no specific limit → falls to wildcard (5); 6 chars fails.
        let err = check_argument_chars(&json!({"other": "123456"}), &cfg).unwrap_err();
        assert!(err.contains("'other'"), "got: {err}");
    }

    // -- recursion: nested objects and arrays ------------------------------

    #[test]
    fn nested_string_inside_object_is_checked() {
        let cfg = config_default_only(5);
        // The nested string "toolong" is 7 chars > 5.  The top-level key is
        // "address" so the error must name "address.city".
        let err = check_argument_chars(
            &json!({"address": {"city": "toolong"}}),
            &cfg,
        )
        .unwrap_err();
        assert!(err.contains("address.city"), "got: {err}");
    }

    #[test]
    fn string_inside_array_is_checked() {
        let cfg = config_default_only(3);
        let err = check_argument_chars(
            &json!({"tags": ["ok", "too_long_for_limit"]}),
            &cfg,
        )
        .unwrap_err();
        assert!(err.contains("tags[1]"), "got: {err}");
    }

    // -- total limit -------------------------------------------------------

    #[test]
    fn total_limit_sums_all_strings() {
        let cfg = config_total_only(10);
        // "hello" (5) + "world" (5) = 10 → passes (at exact limit).
        assert!(
            check_argument_chars(&json!({"a": "hello", "b": "world"}), &cfg).is_ok()
        );
        // "hello" (5) + "world!" (6) = 11 → fails.
        let err =
            check_argument_chars(&json!({"a": "hello", "b": "world!"}), &cfg).unwrap_err();
        assert!(err.contains("11"), "total should be 11, got: {err}");
        assert!(err.contains("10"), "limit should be 10, got: {err}");
    }

    #[test]
    fn total_check_runs_after_per_field_passes() {
        let cfg = PolicyConfig {
            mcp_endpoint: "/mcp".into(),
            strict_mode: true,
            default_field_max_chars: Some(100),
            field_limits: vec![],
            max_total_argument_chars: Some(5),
            max_request_bytes: 1_048_576,
        };
        // Each field individually ok (< 100) but combined > 5.
        let err =
            check_argument_chars(&json!({"a": "abc", "b": "defg"}), &cfg).unwrap_err();
        assert!(err.contains("total"), "should be a total-chars error, got: {err}");
    }

    // -- unicode: chars() not bytes() --------------------------------------

    #[test]
    fn unicode_chars_counted_not_bytes() {
        let cfg = config_default_only(3);
        // "São" is 3 Unicode chars but 5 UTF-8 bytes; limit=3 → passes.
        assert!(check_argument_chars(&json!({"city": "São"}), &cfg).is_ok());
        // "SãoP" is 4 chars → fails.
        let err = check_argument_chars(&json!({"city": "SãoP"}), &cfg).unwrap_err();
        assert!(err.contains("'city'"), "got: {err}");
        assert!(err.contains("4"), "should report 4 chars, got: {err}");
    }

    // -- error sanitisation ------------------------------------------------

    #[test]
    fn error_never_echoes_the_offending_string_value() {
        let cfg = config_default_only(3);
        let secret = "TOP_SECRET_PROMPT_INJECTION_PAYLOAD";
        let err = check_argument_chars(&json!({"cmd": secret}), &cfg).unwrap_err();
        assert!(!err.contains(secret), "error must not echo value, got: {err}");
        assert!(err.contains("'cmd'"), "error must name field, got: {err}");
    }
}
