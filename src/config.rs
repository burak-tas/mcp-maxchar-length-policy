//! Typed, validated view over the policy configuration.

use crate::generated::config::{Config, FieldLimits0Config};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("defaultFieldMaxChars must be >= 1 (got {0})")]
    InvalidDefaultMax(i64),

    #[error("fieldLimits[{idx}]: maxChars must be >= 1 (got {value})")]
    InvalidFieldMax { idx: usize, value: i64 },

    #[error("fieldLimits[{idx}]: field name must not be empty")]
    EmptyFieldName { idx: usize },

    #[error("maxTotalArgumentChars must be >= 1 (got {0})")]
    InvalidTotalMax(i64),
}

/// Fully validated runtime configuration for the policy.
#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub mcp_endpoint: String,
    pub strict_mode: bool,

    /// Maximum chars per string field (default applied to all fields not
    /// covered by a specific entry in `field_limits`). `None` = no default.
    pub default_field_max_chars: Option<usize>,

    /// Per-field character limits. Tuple is (field_name, max_chars).
    /// `field_name == "*"` acts as a wildcard (checked after exact matches).
    pub field_limits: Vec<(String, usize)>,

    /// Maximum total character count across all string values in the
    /// arguments object. `None` = not enforced.
    pub max_total_argument_chars: Option<usize>,

    /// Maximum size (bytes) of the incoming MCP request body.
    pub max_request_bytes: usize,
}

impl PolicyConfig {
    pub fn from_config(raw: &Config) -> Result<Self, ConfigError> {
        let default_field_max_chars = match raw.default_field_max_chars {
            None => Some(4096usize), // schema default
            Some(v) if v < 1 => return Err(ConfigError::InvalidDefaultMax(v)),
            Some(v) => Some(v as usize),
        };

        let field_limits = parse_field_limits(raw.field_limits.as_deref())?;

        let max_total_argument_chars = match raw.max_total_argument_chars {
            None => None,
            Some(v) if v < 1 => return Err(ConfigError::InvalidTotalMax(v)),
            Some(v) => Some(v as usize),
        };

        let max_request_bytes =
            clamp_usize(raw.max_request_bytes, 1_024, 104_857_600, 1_048_576);

        Ok(Self {
            mcp_endpoint: normalize_mcp_path(raw.mcp_endpoint.as_deref().unwrap_or("/mcp")),
            strict_mode: raw.strict_mode.unwrap_or(true),
            default_field_max_chars,
            field_limits,
            max_total_argument_chars,
            max_request_bytes,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_field_limits(
    raw: Option<&[FieldLimits0Config]>,
) -> Result<Vec<(String, usize)>, ConfigError> {
    let Some(rows) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(rows.len());
    for (idx, row) in rows.iter().enumerate() {
        if row.field.trim().is_empty() {
            return Err(ConfigError::EmptyFieldName { idx });
        }
        if row.max_chars < 1 {
            return Err(ConfigError::InvalidFieldMax {
                idx,
                value: row.max_chars,
            });
        }
        out.push((row.field.trim().to_string(), row.max_chars as usize));
    }
    Ok(out)
}

fn normalize_mcp_path(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return "/mcp".into();
    }
    if !s.starts_with('/') {
        return format!("/{s}");
    }
    s.to_string()
}

fn clamp_usize(v: Option<i64>, min: i64, max: i64, default: i64) -> usize {
    v.unwrap_or(default).clamp(min, max) as usize
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::config::{Config, FieldLimits0Config};

    fn bare_config() -> Config {
        Config {
            default_field_max_chars: None,
            field_limits: None,
            max_request_bytes: None,
            max_total_argument_chars: None,
            mcp_endpoint: None,
            strict_mode: None,
        }
    }

    #[test]
    fn defaults_apply_when_nothing_configured() {
        let cfg = PolicyConfig::from_config(&bare_config()).unwrap();
        assert_eq!(cfg.mcp_endpoint, "/mcp");
        assert_eq!(cfg.strict_mode, true);
        assert_eq!(cfg.default_field_max_chars, Some(4096));
        assert!(cfg.field_limits.is_empty());
        assert!(cfg.max_total_argument_chars.is_none());
        assert_eq!(cfg.max_request_bytes, 1_048_576);
    }

    #[test]
    fn field_limits_parsed_correctly() {
        let raw = Config {
            field_limits: Some(vec![
                FieldLimits0Config {
                    field: "description".into(),
                    max_chars: 512,
                },
                FieldLimits0Config {
                    field: "*".into(),
                    max_chars: 2048,
                },
            ]),
            ..bare_config()
        };
        let cfg = PolicyConfig::from_config(&raw).unwrap();
        assert_eq!(cfg.field_limits.len(), 2);
        assert_eq!(cfg.field_limits[0], ("description".into(), 512));
        assert_eq!(cfg.field_limits[1], ("*".into(), 2048));
    }

    #[test]
    fn zero_default_max_is_rejected() {
        let raw = Config {
            default_field_max_chars: Some(0),
            ..bare_config()
        };
        assert!(matches!(
            PolicyConfig::from_config(&raw),
            Err(ConfigError::InvalidDefaultMax(0))
        ));
    }

    #[test]
    fn zero_field_max_is_rejected() {
        let raw = Config {
            field_limits: Some(vec![FieldLimits0Config {
                field: "desc".into(),
                max_chars: 0,
            }]),
            ..bare_config()
        };
        assert!(matches!(
            PolicyConfig::from_config(&raw),
            Err(ConfigError::InvalidFieldMax { idx: 0, value: 0 })
        ));
    }

    #[test]
    fn empty_field_name_is_rejected() {
        let raw = Config {
            field_limits: Some(vec![FieldLimits0Config {
                field: "  ".into(),
                max_chars: 100,
            }]),
            ..bare_config()
        };
        assert!(matches!(
            PolicyConfig::from_config(&raw),
            Err(ConfigError::EmptyFieldName { idx: 0 })
        ));
    }

    #[test]
    fn total_max_zero_is_rejected() {
        let raw = Config {
            max_total_argument_chars: Some(0),
            ..bare_config()
        };
        assert!(matches!(
            PolicyConfig::from_config(&raw),
            Err(ConfigError::InvalidTotalMax(0))
        ));
    }

    #[test]
    fn mcp_endpoint_gets_leading_slash() {
        let raw = Config {
            mcp_endpoint: Some("mcp".into()),
            ..bare_config()
        };
        let cfg = PolicyConfig::from_config(&raw).unwrap();
        assert_eq!(cfg.mcp_endpoint, "/mcp");
    }
}
