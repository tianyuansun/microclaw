use async_trait::async_trait;
use serde_json::json;
use teloxide::prelude::*;

use super::{schema_object, Tool, ToolResult};
use crate::claude::ToolDefinition;

pub struct SendMessageTool {
    bot: Bot,
}

impl SendMessageTool {
    pub fn new(bot: Bot) -> Self {
        SendMessageTool { bot }
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".into(),
            description: "Send a message to a Telegram chat mid-conversation. Use this when you want to send intermediate updates, progress reports, or multiple messages before your final response.".into(),
            input_schema: schema_object(
                json!({
                    "chat_id": {
                        "type": "integer",
                        "description": "The chat ID to send the message to (use the current chat_id from system prompt)"
                    },
                    "text": {
                        "type": "string",
                        "description": "The message text to send"
                    }
                }),
                &["chat_id", "text"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let chat_id = match input.get("chat_id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return ToolResult::error("Missing required parameter: chat_id".into()),
        };
        let text = match input.get("text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::error("Missing required parameter: text".into()),
        };

        match self
            .bot
            .send_message(ChatId(chat_id), text)
            .await
        {
            Ok(_) => ToolResult::success("Message sent successfully.".into()),
            Err(e) => ToolResult::error(format!("Failed to send message: {e}")),
        }
    }
}
