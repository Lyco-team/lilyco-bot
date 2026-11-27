//! Strict TOML configuration loader for `lilyco-bot`.
//!
//! Follows `lyco_dev` rules:
//! - All runtime settings live in `.toml` (no environment-variable sprawl).
//! - Unknown keys are rejected at parse time (`#[serde(deny_unknown_fields)]`).
//! - Every struct stays within Rule of 4 (<= 4 fields).

use std::path::Path;

use serde::Deserialize;
use thiserror::Error;

/// Default configuration filename searched in the current working directory.
pub const DEFAULT_CONFIG_PATH: &str = "lilyco-bot.toml";

/// Structured configuration errors with actionable recovery hints.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config '{path}': {source} (hint: copy lilyco-bot.example.toml to {path})")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML in '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("config validation failed: {0}")]
    Validation(String),
}

/// Top-level `lilyco-bot.toml` schema (<= 4 fields).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    /// Display name shown in `/help` and logs.
    #[serde(default = "default_app_name")]
    pub app_name: String,
    /// Fallback locale when an IM platform does not report user language (`"zh-CN"` or `"en"`).
    #[serde(default = "default_locale")]
    pub default_locale: String,
    /// Telegram Bot settings (`[telegram]` section).
    pub telegram: Option<TgBotConfig>,
}

/// `[telegram]` section in `lilyco-bot.toml` (<= 4 fields).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TgBotConfig {
    /// Bot token issued by `@BotFather`.
    pub token: String,
    /// Optional HTTP or SOCKS5 proxy URL (e.g. `http://127.0.0.1:7890`).
    pub proxy: Option<String>,
    /// Optional allow-list of Telegram numeric user IDs. Empty allows all users.
    #[serde(default)]
    pub allowed_users: Vec<u64>,
    /// Minimum interval (ms) between Telegram `editMessageText` progress updates.
    #[serde(default = "default_progress_throttle_ms")]
    pub progress_throttle_ms: u64,
}

fn default_app_name() -> String {
    "LilycoBot".to_string()
}

fn default_locale() -> String {
    "zh-CN".to_string()
}

fn default_progress_throttle_ms() -> u64 {
    1500
}

impl BotConfig {
    /// Load and validate a `lilyco-bot.toml` file from disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let path_str = path_ref.display().to_string();
        let raw = std::fs::read_to_string(path_ref).map_err(|source| ConfigError::Io {
            path: path_str.clone(),
            source,
        })?;
        Self::from_toml_str(&raw, &path_str)
    }

    /// Load from [`DEFAULT_CONFIG_PATH`] (`./lilyco-bot.toml`).
    pub fn load_default() -> Result<Self, ConfigError> {
        Self::from_file(DEFAULT_CONFIG_PATH)
    }

    /// Parse and validate TOML text directly (also used in tests).
    pub fn from_toml_str(raw: &str, source_label: &str) -> Result<Self, ConfigError> {
        let cfg: Self = toml::from_str(raw).map_err(|source| ConfigError::Parse {
            path: source_label.to_string(),
            source,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Extract the `[telegram]` config or return a clear validation error.
    pub fn require_telegram(&self) -> Result<&TgBotConfig, ConfigError> {
        self.telegram.as_ref().ok_or_else(|| {
            ConfigError::Validation("missing [telegram] section in lilyco-bot.toml".to_string())
        })
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.app_name.trim().is_empty() {
            return Err(ConfigError::Validation("app_name must not be empty".into()));
        }
        if let Some(tg) = &self.telegram {
            if tg.token.trim().is_empty() || tg.token.contains("REPLACE_WITH") {
                return Err(ConfigError::Validation(
                    "[telegram].token is empty or still set to placeholder".into(),
                ));
            }
            if tg.progress_throttle_ms < 250 {
                return Err(ConfigError::Validation(
                    "[telegram].progress_throttle_ms must be >= 250 ms to avoid Telegram 429"
                        .into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_toml_config() {
        let raw = r#"
app_name = "MyLilycoBot"
default_locale = "zh-CN"

[telegram]
token = "123456:real-token"
proxy = "http://127.0.0.1:7890"
allowed_users = [10001, 10002]
progress_throttle_ms = 1200
"#;
        let cfg = BotConfig::from_toml_str(raw, "mem.toml").expect("valid toml");
        assert_eq!(cfg.app_name, "MyLilycoBot");
        let tg = cfg.require_telegram().expect("telegram present");
        assert_eq!(tg.token, "123456:real-token");
        assert_eq!(tg.allowed_users, vec![10001, 10002]);
        assert_eq!(tg.progress_throttle_ms, 1200);
    }

    #[test]
    fn rejects_unknown_toml_fields() {
        let raw = r#"
app_name = "Bot"
typo_field = true
"#;
        let err = BotConfig::from_toml_str(raw, "mem.toml").unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn rejects_placeholder_or_empty_token() {
        let raw = r#"
[telegram]
token = "REPLACE_WITH_YOUR_BOT_TOKEN"
"#;
        let err = BotConfig::from_toml_str(raw, "mem.toml").unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn rejects_aggressive_progress_throttle() {
        let raw = r#"
[telegram]
token = "123456:ok"
progress_throttle_ms = 50
"#;
        let err = BotConfig::from_toml_str(raw, "mem.toml").unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }
}
