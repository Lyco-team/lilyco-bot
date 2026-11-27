//! Telegram Bot backend powered by `teloxide` (MIT).
//!
//! Reads `TELOXIDE_TOKEN` (or an explicit token) and optional `TELOXIDE_PROXY` / `HTTPS_PROXY`,
//! auto-registers all visible `lilyco_core::Registry` commands via `setMyCommands`,
//! and dispatches incoming messages through [`BotDispatcher`].

use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::BotCommand;

use crate::dispatcher::{BotDispatcher, IncomingMessage};

/// Configuration for running a Lilyco Registry as a Telegram Bot.
#[derive(Debug, Clone)]
pub struct TgBotConfig {
    pub token: String,
    /// Optional HTTP or SOCKS5 proxy URL (e.g. `http://127.0.0.1:7890` or `socks5://127.0.0.1:1080`).
    pub proxy: Option<String>,
    /// Optional allow-list of Telegram user IDs. Empty means allow all users.
    pub allowed_users: Vec<u64>,
}

impl TgBotConfig {
    /// Load from `TELOXIDE_TOKEN` (required) and `TELOXIDE_PROXY` / `HTTPS_PROXY` (optional).
    pub fn from_env() -> Result<Self, String> {
        let token = std::env::var("TELOXIDE_TOKEN")
            .or_else(|_| std::env::var("TG_BOT_TOKEN"))
            .map_err(|_| "Missing TELOXIDE_TOKEN (or TG_BOT_TOKEN) environment variable".to_string())?;
        let proxy = std::env::var("TELOXIDE_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("https_proxy"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        Ok(Self {
            token,
            proxy,
            allowed_users: Vec::new(),
        })
    }
}

/// Start a long-polling Telegram Bot backed by a [`BotDispatcher`].
pub async fn run_tg_bot(dispatcher: BotDispatcher, cfg: TgBotConfig) -> Result<(), String> {
    let mut client_builder = reqwest::Client::builder();
    if let Some(proxy_url) = &cfg.proxy {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| format!("Invalid proxy URL '{proxy_url}': {e}"))?;
        client_builder = client_builder.proxy(proxy);
    }
    let client = client_builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let bot = Bot::with_client(cfg.token, client);

    // Sync visible Registry commands to Telegram's `/` command menu.
    let commands: Vec<BotCommand> = dispatcher
        .menu_commands()
        .into_iter()
        .map(|(name, desc)| BotCommand::new(name, desc))
        .collect();
    if let Err(e) = bot.set_my_commands(commands).await {
        tracing::warn!("setMyCommands failed (bot will still run): {e}");
    }

    let disp = Arc::new(dispatcher);
    let allowed = Arc::new(cfg.allowed_users);
    tracing::info!("Lilyco Telegram Bot '{}' started (long polling)", disp.app_name());

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let disp = Arc::clone(&disp);
        let allowed = Arc::clone(&allowed);
        async move {
            let user_id = msg.from.as_ref().map(|u| u.id.0).unwrap_or(0);
            if !allowed.is_empty() && !allowed.contains(&user_id) {
                return Ok(());
            }
            let Some(text) = msg.text().or_else(|| msg.caption()) else {
                return Ok(());
            };
            let incoming = IncomingMessage {
                platform: "telegram",
                chat_id: msg.chat.id.0.to_string(),
                user_id: user_id.to_string(),
                language_code: msg.from.as_ref().and_then(|u| u.language_code.clone()),
                text: text.to_string(),
                attachment_path: None,
            };
            let reply = disp.handle(incoming).await;
            bot.send_message(msg.chat.id, reply.text).await?;
            Ok(())
        }
    })
    .await;

    Ok(())
}
