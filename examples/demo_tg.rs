//! Minimal runnable Telegram Bot example backed by `lilyco_core::Registry` and `lilyco-bot.toml`.
//!
//! Usage:
//!   cp lilyco-bot.example.toml lilyco-bot.toml
//!   # Verify token & proxy connectivity:
//!   cargo run --example demo_tg -- --check
//!   # Start live long-polling Telegram bot (custom config path optional):
//!   cargo run --example demo_tg -- --config lilyco-bot.toml

use std::process::ExitCode;
use std::sync::Arc;

use lilyco_bot::tg::{run_tg_bot, verify_tg_token};
use lilyco_bot::{BotConfig, BotDispatcher, DEFAULT_CONFIG_PATH};
use lilyco_core::registry::{RegisteredCommand, Registry};
use lilyco_core::safety::SafetyTier;
use lilyco_core::schema::{ArgKind, ArgSchema, CommandSchema};

fn parse_cli_args() -> (String, bool) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config_path = DEFAULT_CONFIG_PATH.to_string();
    let mut check_only = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check_only = true,
            "--config" if i + 1 < args.len() => {
                config_path = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    (config_path, check_only)
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt::init();

    let (config_path, check_only) = parse_cli_args();
    let cfg = match BotConfig::from_file(&config_path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Config error: {err}");
            return ExitCode::FAILURE;
        }
    };
    let tg_cfg = match cfg.require_telegram() {
        Ok(t) => t.clone(),
        Err(err) => {
            eprintln!("Config error: {err}");
            return ExitCode::FAILURE;
        }
    };

    if check_only {
        match verify_tg_token(&tg_cfg).await {
            Ok(info) => {
                println!("Telegram Bot OK: {info}");
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                eprintln!("Telegram check failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

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

    let mut cmd =
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
        }));
    cmd.ui_messages = Some("[zh-CN]\nabout = \"复读文本消息（支持重复次数）\"\n");

    if let Err(err) = reg.register(cmd) {
        eprintln!("Failed to register command: {err}");
        return ExitCode::FAILURE;
    }

    let dispatcher = BotDispatcher::new(&cfg.app_name, reg);
    if let Err(err) = run_tg_bot(dispatcher, tg_cfg).await {
        eprintln!("Telegram bot exited with error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
