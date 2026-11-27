use std::path::Path;
use std::sync::Arc;

use lilyco_core::context::Context;
use lilyco_core::progress::Progress;
use lilyco_core::registry::{RegisteredCommand, Registry};
use lilyco_core::schema::{ArgKind, CommandSchema};
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;

/// Platform-agnostic incoming chat message.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub platform: &'static str,
    pub chat_id: String,
    pub user_id: String,
    pub language_code: Option<String>,
    pub text: String,
    /// Optional local file path downloaded from an IM attachment (e.g. TG photo/document).
    pub attachment_path: Option<String>,
}

/// Platform-agnostic reply returned by [`BotDispatcher`].
#[derive(Debug, Clone)]
pub struct OutgoingReply {
    pub text: String,
    pub is_error: bool,
    /// Optional local file path extracted from command output for IM file upload (e.g. `sendDocument`).
    pub output_file: Option<String>,
}

#[derive(Debug, Error)]
pub enum BotError {
    #[error("unknown command: /{0}")]
    UnknownCommand(String),
    #[error("command /{0} has no executable handler")]
    MissingHandler(String),
    #[error("argument error: {0}")]
    InvalidArgs(String),
    #[error("execution failed: {0}")]
    Execution(String),
}

/// Shared command dispatcher that bridges any IM platform to `lilyco_core::Registry`.
#[derive(Clone)]
pub struct BotDispatcher {
    app_name: String,
    registry: Arc<Registry>,
}

impl BotDispatcher {
    pub fn new(app_name: impl Into<String>, registry: Registry) -> Self {
        Self {
            app_name: app_name.into(),
            registry: Arc::new(registry),
        }
    }

    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// List visible `(command_name, description)` pairs for platform menu registration
    /// (e.g., Telegram `setMyCommands`).
    pub fn menu_commands(&self, zh: bool) -> Vec<(String, String)> {
        let help_desc = if zh {
            "查看可用命令列表"
        } else {
            "Show available commands"
        };
        let mut items = vec![("help".to_string(), help_desc.to_string())];
        for cmd in self.registry.visible() {
            let desc = localized_about(cmd, zh);
            let norm = cmd.name.replace('-', "_");
            items.push((norm, desc));
        }
        items
    }

    /// Render a concise help text listing all visible commands and their arguments.
    pub fn format_help(&self, zh: bool) -> String {
        let mut lines = Vec::new();
        if zh {
            lines.push(format!("🤖 {} 可用命令：", self.app_name));
        } else {
            lines.push(format!("🤖 {} commands:", self.app_name));
        }
        for cmd in self.registry.visible() {
            let args_hint: Vec<String> = cmd
                .schema
                .args
                .iter()
                .map(|a| {
                    if a.required {
                        format!("{}=<val>", a.name)
                    } else {
                        format!("[{}=<val>]", a.name)
                    }
                })
                .collect();
            let usage = if args_hint.is_empty() {
                format!("/{}", cmd.name)
            } else {
                format!("/{} {}", cmd.name, args_hint.join(" "))
            };
            let about = localized_about(cmd, zh);
            lines.push(format!("• {} — {}", usage, about));
        }
        lines.join("\n")
    }

    /// Dispatch an [`IncomingMessage`] against the underlying `lilyco_core::Registry`.
    pub async fn handle(&self, msg: IncomingMessage) -> OutgoingReply {
        self.handle_with_progress(msg, None).await
    }

    /// Dispatch an [`IncomingMessage`] and optionally stream human-readable progress status lines.
    pub async fn handle_with_progress(
        &self,
        msg: IncomingMessage,
        progress_tx: Option<UnboundedSender<String>>,
    ) -> OutgoingReply {
        let zh = msg
            .language_code
            .as_deref()
            .map(|l| l.to_ascii_lowercase().starts_with("zh"))
            .unwrap_or(false);

        let trimmed = msg.text.trim();
        let Some((raw_cmd, rest)) = split_command(trimmed) else {
            return OutgoingReply {
                text: self.format_help(zh),
                is_error: false,
                output_file: None,
            };
        };

        if raw_cmd == "start" || raw_cmd == "help" {
            return OutgoingReply {
                text: self.format_help(zh),
                is_error: false,
                output_file: None,
            };
        }

        match self
            .execute_command(&raw_cmd, rest, msg.attachment_path.as_deref(), progress_tx)
            .await
        {
            Ok(val) => {
                let output_file = detect_output_file(&val);
                let pretty = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
                OutgoingReply {
                    text: format!("✅ /{}\n{}", raw_cmd, pretty),
                    is_error: false,
                    output_file,
                }
            }
            Err(err) => OutgoingReply {
                text: format!("❌ {}", err),
                is_error: true,
                output_file: None,
            },
        }
    }

    async fn execute_command(
        &self,
        raw_cmd: &str,
        rest: &str,
        attachment_path: Option<&str>,
        progress_tx: Option<UnboundedSender<String>>,
    ) -> Result<Value, BotError> {
        let canonical = raw_cmd.replace('_', "-");
        let cmd = self
            .registry
            .get(raw_cmd)
            .or_else(|| self.registry.get(&canonical))
            .ok_or_else(|| BotError::UnknownCommand(raw_cmd.to_string()))?;

        let handler = cmd
            .handler
            .clone()
            .ok_or_else(|| BotError::MissingHandler(cmd.name.clone()))?;

        let args = parse_kv_args(&cmd.schema, rest, attachment_path)?;
        cmd.schema
            .validate_args(&args)
            .map_err(|e| BotError::InvalidArgs(e.to_string()))?;

        let (std_tx, std_rx) = std::sync::mpsc::channel::<Progress>();

        // Forward lilyco_core::Progress events to async progress_tx if supplied.
        let forwarder = progress_tx.map(|async_tx| {
            std::thread::spawn(move || {
                for ev in std_rx {
                    if let Some(line) = format_progress(&ev) {
                        let _ = async_tx.send(line);
                    }
                }
            })
        });

        let res = tokio::task::spawn_blocking(move || {
            let ctx = Context::from_sender(std_tx);
            (handler)(&ctx, &args).map_err(|e| BotError::Execution(e.to_string()))
        })
        .await
        .map_err(|e| BotError::Execution(format!("worker panicked: {e}")))?;

        if let Some(handle) = forwarder {
            let _ = handle.join();
        }
        res
    }
}

/// Extract `[zh-CN].about` from `cmd.ui_messages` TOML when `zh == true`, falling back to `schema.about`.
fn localized_about(cmd: &RegisteredCommand, zh: bool) -> String {
    if zh {
        if let Some(raw_toml) = cmd.ui_messages {
            if let Ok(doc) = raw_toml.parse::<toml::Table>() {
                for key in ["zh-CN", "zh_CN", "zh"] {
                    if let Some(about) = doc
                        .get(key)
                        .and_then(|v| v.as_table())
                        .and_then(|t| t.get("about"))
                        .and_then(|v| v.as_str())
                    {
                        if !about.trim().is_empty() {
                            return about.to_string();
                        }
                    }
                }
            }
        }
    }
    if cmd.schema.about.is_empty() {
        format!("Run {}", cmd.name)
    } else {
        cmd.schema.about.clone()
    }
}

fn format_progress(ev: &Progress) -> Option<String> {
    match ev {
        Progress::Tick {
            current,
            total: Some(total),
            message,
            ..
        } if *total > 0 => {
            let pct = ((*current as f64 / *total as f64) * 100.0).min(100.0);
            let msg = message.as_deref().unwrap_or("");
            Some(format!("⏳ {:.0}% ({}/{}) {}", pct, current, total, msg).trim_end().to_string())
        }
        Progress::Tick {
            current,
            total: None,
            message,
            ..
        } => {
            let msg = message.as_deref().unwrap_or("");
            Some(format!("⏳ #{} {}", current, msg).trim_end().to_string())
        }
        Progress::Log { message, .. } if !message.trim().is_empty() => {
            Some(format!("📝 {}", message))
        }
        _ => None,
    }
}

/// If the handler result JSON contains an existing local file path, return it for IM upload.
fn detect_output_file(val: &Value) -> Option<String> {
    let obj = val.as_object()?;
    for key in ["output_file", "output_path", "output", "file"] {
        if let Some(path_str) = obj.get(key).and_then(|v| v.as_str()) {
            let p = Path::new(path_str);
            if p.is_file() {
                return Some(path_str.to_string());
            }
        }
    }
    None
}

/// Split `/cmd@MyBot arg1=val1 arg2=val2` into `("cmd", "arg1=val1 arg2=val2")`.
fn split_command(input: &str) -> Option<(String, &str)> {
    let s = input.strip_prefix('/')?;
    let mut parts = s.splitn(2, char::is_whitespace);
    let head = parts.next()?.trim();
    if head.is_empty() {
        return None;
    }
    let cmd = head.split('@').next().unwrap_or(head).to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim();
    Some((cmd, rest))
}

/// Parse `key=value` / `--flag` tokens and coerce them according to `CommandSchema`.
fn parse_kv_args(
    schema: &CommandSchema,
    rest: &str,
    attachment_path: Option<&str>,
) -> Result<Value, BotError> {
    let mut map = Map::new();

    if let Some(path) = attachment_path {
        if let Some(file_arg) = schema
            .args
            .iter()
            .find(|a| matches!(a.kind, ArgKind::Path { .. }))
        {
            map.insert(file_arg.name.clone(), Value::String(path.to_string()));
        }
    }

    for tok in shell_split(rest) {
        if let Some(flag) = tok.strip_prefix("--") {
            if let Some((k, v)) = flag.split_once('=') {
                insert_typed(&mut map, schema, k, v)?;
            } else {
                map.insert(flag.to_string(), Value::Bool(true));
            }
        } else if let Some((k, v)) = tok.split_once('=') {
            insert_typed(&mut map, schema, k, v)?;
        } else if let Some(first_req) = schema
            .args
            .iter()
            .find(|a| a.required && !map.contains_key(&a.name))
        {
            insert_typed(&mut map, schema, &first_req.name, &tok)?;
        }
    }

    Ok(Value::Object(map))
}

fn insert_typed(
    map: &mut Map<String, Value>,
    schema: &CommandSchema,
    key: &str,
    raw: &str,
) -> Result<(), BotError> {
    let spec = schema
        .args
        .iter()
        .find(|a| a.name == key || a.name.replace('-', "_") == key);
    let canonical_key = spec.map(|s| s.name.as_str()).unwrap_or(key);

    let val = match spec.map(|s| &s.kind) {
        Some(ArgKind::Flag) => {
            let b = matches!(
                raw.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
            Value::Bool(b)
        }
        Some(ArgKind::Number { .. }) => {
            if let Ok(n) = raw.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(f) = raw.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .ok_or_else(|| {
                        BotError::InvalidArgs(format!("{canonical_key} invalid number '{raw}'"))
                    })?
            } else {
                return Err(BotError::InvalidArgs(format!(
                    "{canonical_key} expects number, got '{raw}'"
                )));
            }
        }
        Some(ArgKind::List { .. }) => {
            let items: Vec<Value> = raw
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.to_string()))
                .collect();
            Value::Array(items)
        }
        _ => Value::String(raw.to_string()),
    };

    map.insert(canonical_key.to_string(), val);
    Ok(())
}

/// Minimal quote-aware tokenizer (`key="hello world"` -> `key=hello world`).
fn shell_split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;

    for ch in s.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lilyco_core::registry::RegisteredCommand;
    use lilyco_core::safety::SafetyTier;
    use lilyco_core::schema::{ArgKind, ArgSchema, CommandSchema};

    fn sample_registry() -> Registry {
        let mut reg = Registry::new();
        let schema = CommandSchema {
            name: "ping-node".into(),
            about: "Check node latency".into(),
            args: vec![
                ArgSchema {
                    name: "host".into(),
                    kind: ArgKind::Text,
                    required: true,
                    about: "Target hostname".into(),
                    default: None,
                },
                ArgSchema {
                    name: "count".into(),
                    kind: ArgKind::Number {
                        min: Some(1.0),
                        max: Some(10.0),
                    },
                    required: false,
                    about: "Ping count".into(),
                    default: Some(serde_json::json!(3)),
                },
            ],
            subcommands: vec![],
            safety: SafetyTier::default(),
        };
        let mut cmd =
            RegisteredCommand::new("ping-node", schema).with_handler(Arc::new(|ctx, args| {
                ctx.tick(1, Some(2), "probing");
                Ok(serde_json::json!({
                    "host": args.get("host"),
                    "count": args.get("count").cloned().unwrap_or_else(|| serde_json::json!(3)),
                    "ok": true
                }))
            }));
        cmd.ui_messages = Some("[zh-CN]\nabout = \"检测节点延迟\"\n");
        reg.register(cmd).unwrap();
        reg
    }

    #[tokio::test]
    async fn help_uses_ui_messages_zh_cn() {
        let disp = BotDispatcher::new("LilycoDemo", sample_registry());
        let reply = disp
            .handle(IncomingMessage {
                platform: "test",
                chat_id: "1".into(),
                user_id: "1".into(),
                language_code: Some("zh-hans".into()),
                text: "/help".into(),
                attachment_path: None,
            })
            .await;
        assert!(!reply.is_error);
        assert!(reply.text.contains("/ping-node"));
        assert!(reply.text.contains("检测节点延迟"));
    }

    #[tokio::test]
    async fn executes_kebab_or_underscore_command_with_progress() {
        let disp = BotDispatcher::new("LilycoDemo", sample_registry());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let reply = disp
            .handle_with_progress(
                IncomingMessage {
                    platform: "tg",
                    chat_id: "42".into(),
                    user_id: "7".into(),
                    language_code: Some("en".into()),
                    text: "/ping_node@MyBot host=example.com count=5".into(),
                    attachment_path: None,
                },
                Some(tx),
            )
            .await;
        assert!(!reply.is_error, "unexpected error: {}", reply.text);
        assert!(reply.text.contains("\"host\": \"example.com\""));
        assert!(reply.text.contains("\"count\": 5"));
        let tick = rx.try_recv().expect("should emit progress tick");
        assert!(tick.contains("50%"));
    }

    #[tokio::test]
    async fn rejects_invalid_number_arg() {
        let disp = BotDispatcher::new("LilycoDemo", sample_registry());
        let reply = disp
            .handle(IncomingMessage {
                platform: "tg",
                chat_id: "42".into(),
                user_id: "7".into(),
                language_code: None,
                text: "/ping-node host=example.com count=abc".into(),
                attachment_path: None,
            })
            .await;
        assert!(reply.is_error);
        assert!(reply.text.contains("expects number"));
    }
}
