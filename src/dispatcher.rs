use std::sync::Arc;

use lilyco_core::context::Context;
use lilyco_core::progress::Progress;
use lilyco_core::registry::Registry;
use lilyco_core::schema::{ArgKind, CommandSchema};
use serde_json::{Map, Value};
use thiserror::Error;

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
    pub fn menu_commands(&self) -> Vec<(String, String)> {
        let mut items = vec![("help".to_string(), "Show available commands".to_string())];
        for cmd in self.registry.visible() {
            let desc = if cmd.schema.about.is_empty() {
                format!("Run {}", cmd.name)
            } else {
                cmd.schema.about.clone()
            };
            // Telegram command names allow [a-z0-9_], max 32 chars.
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
            if cmd.schema.about.is_empty() {
                lines.push(format!("• {}", usage));
            } else {
                lines.push(format!("• {} — {}", usage, cmd.schema.about));
            }
        }
        lines.join("\n")
    }

    /// Dispatch an [`IncomingMessage`] against the underlying `lilyco_core::Registry`.
    pub async fn handle(&self, msg: IncomingMessage) -> OutgoingReply {
        let zh = msg
            .language_code
            .as_deref()
            .map(|l| l.to_ascii_lowercase().starts_with("zh"))
            .unwrap_or(false);

        let trimmed = msg.text.trim();
        if trimmed.is_empty() {
            return OutgoingReply {
                text: self.format_help(zh),
                is_error: false,
            };
        }

        let Some((raw_cmd, rest)) = split_command(trimmed) else {
            return OutgoingReply {
                text: self.format_help(zh),
                is_error: false,
            };
        };

        if raw_cmd == "start" || raw_cmd == "help" {
            return OutgoingReply {
                text: self.format_help(zh),
                is_error: false,
            };
        }

        match self
            .execute_command(&raw_cmd, rest, msg.attachment_path.as_deref())
            .await
        {
            Ok(val) => {
                let pretty = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
                OutgoingReply {
                    text: format!("✅ /{}\n{}", raw_cmd, pretty),
                    is_error: false,
                }
            }
            Err(err) => OutgoingReply {
                text: format!("❌ {}", err),
                is_error: true,
            },
        }
    }

    async fn execute_command(
        &self,
        raw_cmd: &str,
        rest: &str,
        attachment_path: Option<&str>,
    ) -> Result<Value, BotError> {
        // Support both `/img_press` (TG menu style) and `/img-press` (canonical kebab-case).
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

        tokio::task::spawn_blocking(move || {
            let (tx, _rx) = std::sync::mpsc::channel::<Progress>();
            let ctx = Context::from_sender(tx);
            (handler)(&ctx, &args).map_err(|e| BotError::Execution(e.to_string()))
        })
        .await
        .map_err(|e| BotError::Execution(format!("worker panicked: {e}")))?
    }
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

    // Auto-bind attachment to the first Path argument if omitted by user.
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
            // Positional convenience: `/echo hello` fills the first unset required arg.
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
            let b = matches!(raw.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
            Value::Bool(b)
        }
        Some(ArgKind::Number { .. }) => {
            if let Ok(n) = raw.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(f) = raw.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .ok_or_else(|| BotError::InvalidArgs(format!("{canonical_key} invalid number '{raw}'")))?
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
        let cmd = RegisteredCommand::new("ping-node", schema).with_handler(Arc::new(|_ctx, args| {
            Ok(serde_json::json!({
                "host": args.get("host"),
                "count": args.get("count").cloned().unwrap_or_else(|| serde_json::json!(3)),
                "ok": true
            }))
        }));
        reg.register(cmd).unwrap();
        reg
    }

    #[tokio::test]
    async fn help_lists_visible_commands() {
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
        assert!(reply.text.contains("可用命令"));
    }

    #[tokio::test]
    async fn executes_kebab_or_underscore_command_with_type_coercion() {
        let disp = BotDispatcher::new("LilycoDemo", sample_registry());
        let reply = disp
            .handle(IncomingMessage {
                platform: "tg",
                chat_id: "42".into(),
                user_id: "7".into(),
                language_code: Some("en".into()),
                text: "/ping_node@MyBot host=example.com count=5".into(),
                attachment_path: None,
            })
            .await;
        assert!(!reply.is_error, "unexpected error: {}", reply.text);
        assert!(reply.text.contains("\"host\": \"example.com\""));
        assert!(reply.text.contains("\"count\": 5"));
    }

    #[tokio::test]
    async fn rejects_invalid_int_arg() {
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
