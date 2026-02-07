use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::claude::{
    ContentBlock, ImageSource, Message, MessageContent, MessagesRequest, MessagesResponse,
    ResponseContentBlock, ToolDefinition, Usage,
};
use crate::config::Config;
use crate::error::MicroClawError;

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn send_message(
        &self,
        system: &str,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<MessagesResponse, MicroClawError>;
}

pub fn create_provider(config: &Config) -> Box<dyn LlmProvider> {
    match config.llm_provider.as_str() {
        "openai" => Box::new(OpenAiProvider::new(config)),
        _ => Box::new(AnthropicProvider::new(config)),
    }
}

// ---------------------------------------------------------------------------
// Anthropic provider
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    http: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(config: &Config) -> Self {
        AnthropicProvider {
            http: reqwest::Client::new(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            base_url: config
                .llm_base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com/v1/messages".into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicApiError {
    error: AnthropicApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicApiErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn send_message(
        &self,
        system: &str,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<MessagesResponse, MicroClawError> {
        let request = MessagesRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            system: system.to_string(),
            messages,
            tools,
        };

        let mut retries = 0u32;
        let max_retries = 3;

        loop {
            let response = self
                .http
                .post(&self.base_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&request)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let body = response.text().await?;
                let parsed: MessagesResponse = serde_json::from_str(&body).map_err(|e| {
                    MicroClawError::AnthropicApi(format!(
                        "Failed to parse response: {e}\nBody: {body}"
                    ))
                })?;
                return Ok(parsed);
            }

            if status.as_u16() == 429 && retries < max_retries {
                retries += 1;
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                warn!(
                    "Rate limited, retrying in {:?} (attempt {retries}/{max_retries})",
                    delay
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            let body = response.text().await.unwrap_or_default();
            if let Ok(api_err) = serde_json::from_str::<AnthropicApiError>(&body) {
                return Err(MicroClawError::AnthropicApi(format!(
                    "{}: {}",
                    api_err.error.error_type, api_err.error.message
                )));
            }
            return Err(MicroClawError::AnthropicApi(format!(
                "HTTP {status}: {body}"
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible provider  (OpenAI, OpenRouter, DeepSeek, Groq, Ollama …)
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    http: reqwest::Client,
    api_key: String,
    model: String,
    max_tokens: u32,
    chat_url: String,
}

impl OpenAiProvider {
    pub fn new(config: &Config) -> Self {
        let base = config
            .llm_base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");
        let chat_url = format!("{}/chat/completions", base.trim_end_matches('/'));

        OpenAiProvider {
            http: reqwest::Client::new(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            max_tokens: config.max_tokens,
            chat_url,
        }
    }
}

// --- OpenAI response types ---

#[derive(Debug, Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OaiToolCall {
    id: String,
    function: OaiFunction,
}

#[derive(Debug, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct OaiErrorResponse {
    error: OaiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct OaiErrorDetail {
    message: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn send_message(
        &self,
        system: &str,
        messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<MessagesResponse, MicroClawError> {
        let oai_messages = translate_messages_to_oai(system, &messages);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "messages": oai_messages,
        });

        if let Some(ref tool_defs) = tools {
            if !tool_defs.is_empty() {
                body["tools"] = json!(translate_tools_to_oai(tool_defs));
            }
        }

        let mut retries = 0u32;
        let max_retries = 3;

        loop {
            let response = self
                .http
                .post(&self.chat_url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = response.status();

            if status.is_success() {
                let text = response.text().await?;
                let oai: OaiResponse = serde_json::from_str(&text).map_err(|e| {
                    MicroClawError::AnthropicApi(format!(
                        "Failed to parse OpenAI response: {e}\nBody: {text}"
                    ))
                })?;
                return Ok(translate_oai_response(oai));
            }

            if status.as_u16() == 429 && retries < max_retries {
                retries += 1;
                let delay = std::time::Duration::from_secs(2u64.pow(retries));
                warn!(
                    "Rate limited, retrying in {:?} (attempt {retries}/{max_retries})",
                    delay
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            let text = response.text().await.unwrap_or_default();
            if let Ok(err) = serde_json::from_str::<OaiErrorResponse>(&text) {
                return Err(MicroClawError::AnthropicApi(err.error.message));
            }
            return Err(MicroClawError::AnthropicApi(format!(
                "HTTP {status}: {text}"
            )));
        }
    }
}

// ---------------------------------------------------------------------------
// Format translation helpers  (internal Anthropic-style ↔ OpenAI)
// ---------------------------------------------------------------------------

fn translate_messages_to_oai(system: &str, messages: &[Message]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();

    // System message
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }

    for msg in messages {
        match &msg.content {
            MessageContent::Text(text) => {
                out.push(json!({"role": msg.role, "content": text}));
            }
            MessageContent::Blocks(blocks) => {
                if msg.role == "assistant" {
                    // Collect text and tool_calls
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");

                    let tool_calls: Vec<serde_json::Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => Some(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input).unwrap_or_default()
                                }
                            })),
                            _ => None,
                        })
                        .collect();

                    let mut m = json!({"role": "assistant"});
                    if !text.is_empty() || tool_calls.is_empty() {
                        m["content"] = json!(text);
                    }
                    if !tool_calls.is_empty() {
                        m["tool_calls"] = json!(tool_calls);
                    }
                    out.push(m);
                } else {
                    // User role — tool_results, images, or text
                    let has_tool_results = blocks
                        .iter()
                        .any(|b| matches!(b, ContentBlock::ToolResult { .. }));

                    if has_tool_results {
                        // Each tool result → separate "tool" message
                        for block in blocks {
                            if let ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            } = block
                            {
                                let c = if is_error == &Some(true) {
                                    format!("[Error] {content}")
                                } else {
                                    content.clone()
                                };
                                out.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": c,
                                }));
                            }
                        }
                    } else {
                        // Images + text → multipart content array
                        let has_images = blocks
                            .iter()
                            .any(|b| matches!(b, ContentBlock::Image { .. }));
                        if has_images {
                            let parts: Vec<serde_json::Value> = blocks
                                .iter()
                                .filter_map(|b| match b {
                                    ContentBlock::Text { text } => {
                                        Some(json!({"type": "text", "text": text}))
                                    }
                                    ContentBlock::Image {
                                        source:
                                            ImageSource {
                                                media_type, data, ..
                                            },
                                    } => {
                                        let url = format!("data:{media_type};base64,{data}");
                                        Some(json!({
                                            "type": "image_url",
                                            "image_url": {"url": url}
                                        }))
                                    }
                                    _ => None,
                                })
                                .collect();
                            out.push(json!({"role": "user", "content": parts}));
                        } else {
                            let text: String = blocks
                                .iter()
                                .filter_map(|b| match b {
                                    ContentBlock::Text { text } => Some(text.as_str()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            out.push(json!({"role": "user", "content": text}));
                        }
                    }
                }
            }
        }
    }

    out
}

fn translate_tools_to_oai(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}

fn translate_oai_response(oai: OaiResponse) -> MessagesResponse {
    let choice = match oai.choices.into_iter().next() {
        Some(c) => c,
        None => {
            return MessagesResponse {
                content: vec![ResponseContentBlock::Text {
                    text: "(empty response)".into(),
                }],
                stop_reason: Some("end_turn".into()),
                usage: None,
            };
        }
    };

    let mut content = Vec::new();

    if let Some(text) = choice.message.content {
        if !text.is_empty() {
            content.push(ResponseContentBlock::Text { text });
        }
    }

    if let Some(tool_calls) = choice.message.tool_calls {
        for tc in tool_calls {
            let input: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or_default();
            content.push(ResponseContentBlock::ToolUse {
                id: tc.id,
                name: tc.function.name,
                input,
            });
        }
    }

    if content.is_empty() {
        content.push(ResponseContentBlock::Text {
            text: String::new(),
        });
    }

    let stop_reason = match choice.finish_reason.as_deref() {
        Some("tool_calls") => Some("tool_use".into()),
        Some("length") => Some("max_tokens".into()),
        _ => Some("end_turn".into()),
    };

    let usage = oai.usage.map(|u| Usage {
        input_tokens: u.prompt_tokens,
        output_tokens: u.completion_tokens,
    });

    MessagesResponse {
        content,
        stop_reason,
        usage,
    }
}
