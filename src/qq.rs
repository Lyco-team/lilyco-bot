//! QQ / OneBot v11 adapter (`feature = "qq"`, powered by `onebot_v11`).
//!
//! Converts incoming OneBot v11 text events (NapCat / LLOneBot) into [`IncomingMessage`]
//! and executes them through the shared [`BotDispatcher`].

use crate::dispatcher::{BotDispatcher, IncomingMessage, OutgoingReply};

/// Handle a raw OneBot v11 text message payload via the shared [`BotDispatcher`].
pub async fn handle_onebot_text(
    dispatcher: &BotDispatcher,
    chat_id: impl Into<String>,
    user_id: impl Into<String>,
    text: impl Into<String>,
) -> OutgoingReply {
    dispatcher
        .handle(IncomingMessage {
            platform: "qq-onebot11",
            chat_id: chat_id.into(),
            user_id: user_id.into(),
            language_code: Some("zh-CN".into()),
            text: text.into(),
            attachment_path: None,
        })
        .await
}
