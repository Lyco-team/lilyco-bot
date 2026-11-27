//! `lilyco-bot` — Multi-platform IM Bot adapter for [`lilyco_core::Registry`].
//!
//! Define your command once with `lilyco-core` (`#[derive(App)]` or `RegisteredCommand`),
//! then expose it to Telegram (`feature = "tg"`), QQ OneBot v11 (`feature = "qq"`),
//! or WeChat iLink (`feature = "wx"`) without duplicating business logic.

pub mod dispatcher;

#[cfg(feature = "tg")]
pub mod tg;

#[cfg(feature = "qq")]
pub mod qq;

#[cfg(feature = "wx")]
pub mod wx;

pub use dispatcher::{BotDispatcher, IncomingMessage, OutgoingReply};
