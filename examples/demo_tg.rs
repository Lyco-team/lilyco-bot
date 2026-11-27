//! Minimal runnable Telegram Bot example backed by `lilyco_core::Registry`.
//!
//! Usage:
//!   # Verify token & proxy connectivity only:
//!   TELOXIDE_TOKEN="123456:ABC..." cargo run --example demo_tg -- --check
//!   # Start live long-polling Telegram bot:
//!   TELOXIDE_TOKEN="123456:ABC..." cargo run --example demo_tg

use std::sync::Arc;

use lilyco_bot::tg::{run_tg_bot, verify_tg_token, TgBotConfig};
use lilyco_bot::BotDispatcher;
use lilyco_core::registry::{RegisteredCommand, Registry};
use lilyco_core::safety::SafetyTier;
use lilyco_core::schema::{ArgKind, ArgSchema, CommandSchema};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut reg = Registry::new();
    let echo_schema = CommandSchema {
        name: "echo".into(),
        about: "Echo back a message with optional repeat count".into(),
        args: vec![
            ArgSchema {
                name: "text".into(),
                kind: ArgKind::Text,
                required: true,
                about: "Message text".into(),
                default: None,
            },
            ArgSchema {
                name: "repeat".into(),
                kind: ArgKind::Number {
                    min: Some(1.0),
                    max: Some(5.0),
                },
                required: false,
                about: "Repeat count (1-5)".into(),
                default: Some(serde_json::json!(1)),
            },
        ],
        subcommands: vec![],
        safety: SafetyTier::default(),
    };

    reg.register(
        RegisteredCommand::new("echo", echo_schema).with_handler(Arc::new(|_ctx, args| {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let repeat = args
                .get("repeat")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .clamp(1, 5) as usize;
            Ok(serde_json::json!({
                "output": std::iter::repeat_n(text, repeat).collect::<Vec<_>>().join(" "),
            }))
        })),
    )
    .expect("register echo command");

    let cfg = TgBotConfig::from_env().expect("set TELOXIDE_TOKEN");
    if std::env::args().any(|a| a == "--check") {
        let info = verify_tg_token(&cfg).await.expect("getMe failed");
        println!("Telegram Bot OK: {info}");
        return;
    }
    let dispatcher = BotDispatcher::new("LilycoBot", reg);
    run_tg_bot(dispatcher, cfg).await.expect("tg bot error");
}
