//! `lilyco-bot` — Multi-platform IM Bot adapter for [`lilyco_core::Registry`].
//!
//! Define your command once with `lilyco-core` (`#[derive(App)]` or `RegisteredCommand`),
//! configure your IM credentials in `lilyco-bot.toml`, and run on Telegram (`feature = "tg"`),
//! QQ OneBot v11 (`feature = "qq"`), or WeChat iLink (`feature = "wx"`).

pub mod config;
pub mod dispatcher;

#[cfg(feature = "tg")]
pub mod tg;

#[cfg(feature = "qq")]
pub mod qq;

#[cfg(feature = "wx")]
pub mod wx;

pub use config::{BotConfig, ConfigError, TgBotConfig, DEFAULT_CONFIG_PATH};
pub use dispatcher::{BotDispatcher, IncomingMessage, OutgoingReply};
