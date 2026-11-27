//! WeChat iLink Bot adapter (`feature = "wx"`, powered by `weixin-agent` MIT SDK).
//!
//! Bridges WeChat iLink text messages into the shared [`BotDispatcher`].

use crate::dispatcher::{BotDispatcher, IncomingMessage, OutgoingReply};

/// Handle a WeChat iLink text message via the shared [`BotDispatcher`].
pub async fn handle_weixin_text(
    dispatcher: &BotDispatcher,
    chat_id: impl Into<String>,
    user_id: impl Into<String>,
    text: impl Into<String>,
) -> OutgoingReply {
    dispatcher
        .handle(IncomingMessage {
            platform: "wechat-ilink",
            chat_id: chat_id.into(),
            user_id: user_id.into(),
            language_code: Some("zh-CN".into()),
            text: text.into(),
            attachment_path: None,
        })
        .await
}
