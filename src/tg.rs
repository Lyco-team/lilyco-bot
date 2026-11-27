//! Telegram Bot backend powered by `teloxide` (MIT).
//!
//! Features:
//! - Auto-registers all visible `lilyco_core::Registry` commands via `setMyCommands`
//! - Downloads incoming photo/document attachments into temp files and binds them to `ArgKind::Path`
//! - Streams `lilyco_core::Progress` updates via throttled `editMessageText`
//! - Automatically uploads output files back to the user via `sendDocument`
//! - Supports HTTP/SOCKS5 proxy via `TELOXIDE_PROXY` / `HTTPS_PROXY`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, InputFile};

use crate::config::TgBotConfig;
use crate::dispatcher::{BotDispatcher, IncomingMessage};

fn build_bot(cfg: &TgBotConfig) -> Result<Bot, String> {
    let mut client_builder = reqwest::Client::builder();
    if let Some(proxy_url) = &cfg.proxy {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| format!("Invalid proxy URL '{proxy_url}': {e}"))?;
        client_builder = client_builder.proxy(proxy);
    }
    let client = client_builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
    Ok(Bot::with_client(cfg.token.clone(), client))
}

/// Verify bot token connectivity via Telegram `getMe` and return `@username`.
pub async fn verify_tg_token(cfg: &TgBotConfig) -> Result<String, String> {
    let bot = build_bot(cfg)?;
    let me = bot
        .get_me()
        .await
        .map_err(|e| format!("Telegram getMe failed: {e}"))?;
    let uname = me
        .username
        .clone()
        .unwrap_or_else(|| me.first_name.clone());
    Ok(format!("@{uname} (id={})", me.id))
}

/// Download a photo or document from an incoming Telegram message to a temporary path.
async fn download_attachment(bot: &Bot, msg: &Message) -> Option<PathBuf> {
    let (file_id, ext) = if let Some(doc) = msg.document() {
        let ext = doc
            .file_name
            .as_deref()
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_string())
            .unwrap_or_else(|| "bin".to_string());
        (doc.file.id.clone(), ext)
    } else if let Some(photos) = msg.photo() {
        let largest = photos.last()?;
        (largest.file.id.clone(), "jpg".to_string())
    } else {
        return None;
    };

    let file = bot.get_file(file_id).await.ok()?;
    let tmp_dir = std::env::temp_dir().join("lilyco-bot-tg");
    let _ = tokio::fs::create_dir_all(&tmp_dir).await;
    let dst_path = tmp_dir.join(format!("tg-{}-{}.{}", msg.chat.id.0, msg.id.0, ext));
    let mut dst = tokio::fs::File::create(&dst_path).await.ok()?;
    bot.download_file(&file.path, &mut dst).await.ok()?;
    Some(dst_path)
}

/// Start a long-polling Telegram Bot backed by a [`BotDispatcher`].
pub async fn run_tg_bot(dispatcher: BotDispatcher, cfg: TgBotConfig) -> Result<(), String> {
    let bot = build_bot(&cfg)?;

    let me = bot
        .get_me()
        .await
        .map_err(|e| format!("Telegram getMe failed: {e}"))?;
    tracing::info!(
        "Connected to Telegram as @{} (id={})",
        me.username.as_deref().unwrap_or("unknown"),
        me.id
    );

    // Sync visible Registry commands to Telegram's `/` command menu (zh & default).
    let commands: Vec<BotCommand> = dispatcher
        .menu_commands(true)
        .into_iter()
        .map(|(name, desc)| BotCommand::new(name, desc))
        .collect();
    if let Err(e) = bot.set_my_commands(commands).await {
        tracing::warn!("setMyCommands failed (bot will still run): {e}");
    }

    let disp = Arc::new(dispatcher);
    let allowed = Arc::new(cfg.allowed_users.clone());
    let throttle_ms = cfg.progress_throttle_ms;
    tracing::info!(
        "Lilyco Telegram Bot '{}' started (long polling, throttle={}ms)",
        disp.app_name(),
        throttle_ms
    );

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let disp = Arc::clone(&disp);
        let allowed = Arc::clone(&allowed);
        async move {
            let user_id = msg.from.as_ref().map(|u| u.id.0).unwrap_or(0);
            if !allowed.is_empty() && !allowed.contains(&user_id) {
                return Ok(());
            }

            let text = msg
                .text()
                .or_else(|| msg.caption())
                .unwrap_or("")
                .to_string();
            let attachment = download_attachment(&bot, &msg).await;
            if text.trim().is_empty() && attachment.is_none() {
                return Ok(());
            }

            let incoming = IncomingMessage {
                platform: "telegram",
                chat_id: msg.chat.id.0.to_string(),
                user_id: user_id.to_string(),
                language_code: msg.from.as_ref().and_then(|u| u.language_code.clone()),
                text,
                attachment_path: attachment.as_ref().map(|p| p.to_string_lossy().to_string()),
            };

            let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let bot_prog = bot.clone();
            let chat_id = msg.chat.id;
            let status_msg_id = Arc::new(tokio::sync::Mutex::new(None));
            let status_msg_id_bg = Arc::clone(&status_msg_id);

            let progress_task = tokio::spawn(async move {
                let mut last_edit = Instant::now() - Duration::from_secs(10);
                while let Some(line) = prx.recv().await {
                    if last_edit.elapsed() < Duration::from_millis(throttle_ms) {
                        continue;
                    }
                    last_edit = Instant::now();
                    let mut guard = status_msg_id_bg.lock().await;
                    if let Some(mid) = *guard {
                        let _ = bot_prog.edit_message_text(chat_id, mid, &line).await;
                    } else if let Ok(sent) = bot_prog.send_message(chat_id, &line).await {
                        *guard = Some(sent.id);
                    }
                }
            });

            let reply = disp.handle_with_progress(incoming, Some(ptx)).await;
            let _ = progress_task.await;

            if let Some(mid) = *status_msg_id.lock().await {
                let _ = bot.edit_message_text(chat_id, mid, &reply.text).await;
            } else {
                bot.send_message(chat_id, &reply.text).await?;
            }

            if let Some(out_file) = reply.output_file {
                let _ = bot
                    .send_document(chat_id, InputFile::file(PathBuf::from(out_file)))
                    .await;
            }

            if let Some(tmp_path) = attachment {
                let _ = tokio::fs::remove_file(tmp_path).await;
            }

            Ok(())
        }
    })
    .await;

    Ok(())
}
