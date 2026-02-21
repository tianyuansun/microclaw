use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use tracing::{error, info, warn};

use crate::agent_engine::archive_conversation;
use crate::agent_engine::process_with_agent_with_events;
use crate::agent_engine::AgentEvent;
use crate::agent_engine::AgentRequestContext;
use crate::runtime::AppState;
use microclaw_channels::channel::ConversationKind;
use microclaw_channels::channel_adapter::ChannelAdapter;
use microclaw_core::llm_types::Message as LlmMessage;
use microclaw_core::text::split_text;
use microclaw_storage::db::call_blocking;
use microclaw_storage::db::StoredMessage;
use microclaw_storage::usage::build_usage_report;

fn default_enabled() -> bool {
    true
}

fn default_matrix_mention_required() -> bool {
    true
}

fn default_matrix_sync_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixAccountConfig {
    pub access_token: String,
    pub homeserver_url: String,
    pub bot_user_id: String,
    #[serde(default)]
    pub allowed_room_ids: Vec<String>,
    #[serde(default)]
    pub bot_username: String,
    #[serde(default = "default_matrix_mention_required")]
    pub mention_required: bool,
    #[serde(default = "default_matrix_sync_timeout_ms")]
    pub sync_timeout_ms: u64,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MatrixChannelConfig {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub homeserver_url: String,
    #[serde(default)]
    pub bot_user_id: String,
    #[serde(default)]
    pub allowed_room_ids: Vec<String>,
    #[serde(default)]
    pub bot_username: String,
    #[serde(default = "default_matrix_mention_required")]
    pub mention_required: bool,
    #[serde(default = "default_matrix_sync_timeout_ms")]
    pub sync_timeout_ms: u64,
    #[serde(default)]
    pub accounts: HashMap<String, MatrixAccountConfig>,
    #[serde(default)]
    pub default_account: Option<String>,
}

fn pick_default_account_id(
    configured: Option<&str>,
    accounts: &HashMap<String, MatrixAccountConfig>,
) -> Option<String> {
    let explicit = configured
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);
    if explicit.is_some() {
        return explicit;
    }
    if accounts.contains_key("default") {
        return Some("default".to_string());
    }
    let mut keys: Vec<String> = accounts.keys().cloned().collect();
    keys.sort();
    keys.first().cloned()
}

#[derive(Clone)]
pub struct MatrixRuntimeContext {
    pub channel_name: String,
    pub access_token: String,
    pub homeserver_url: String,
    pub bot_user_id: String,
    pub bot_username: String,
    pub allowed_room_ids: Vec<String>,
    pub mention_required: bool,
    pub sync_timeout_ms: u64,
}

impl MatrixRuntimeContext {
    fn normalized_homeserver_url(&self) -> String {
        self.homeserver_url.trim_end_matches('/').to_string()
    }

    fn sync_timeout_ms_or_default(&self) -> u64 {
        if self.sync_timeout_ms == 0 {
            default_matrix_sync_timeout_ms()
        } else {
            self.sync_timeout_ms
        }
    }

    fn should_process_room(&self, room_id: &str) -> bool {
        self.allowed_room_ids.is_empty() || self.allowed_room_ids.iter().any(|v| v == room_id)
    }

    fn bot_localpart(&self) -> String {
        let user = self.bot_user_id.trim();
        if let Some(rest) = user.strip_prefix('@') {
            return rest.split(':').next().unwrap_or(rest).to_string();
        }
        user.to_string()
    }

    fn should_respond(&self, text: &str) -> bool {
        if !self.mention_required {
            return true;
        }

        let text_lower = text.to_lowercase();
        let user_lower = self.bot_user_id.to_lowercase();
        if !user_lower.is_empty() && text_lower.contains(&user_lower) {
            return true;
        }

        let localpart = self.bot_localpart().to_lowercase();
        !localpart.is_empty() && text_lower.contains(&localpart)
    }
}

pub fn build_matrix_runtime_contexts(config: &crate::config::Config) -> Vec<MatrixRuntimeContext> {
    let Some(matrix_cfg) = config.channel_config::<MatrixChannelConfig>("matrix") else {
        return Vec::new();
    };

    let default_account =
        pick_default_account_id(matrix_cfg.default_account.as_deref(), &matrix_cfg.accounts);

    let mut runtimes = Vec::new();

    let mut account_ids: Vec<String> = matrix_cfg.accounts.keys().cloned().collect();
    account_ids.sort();
    for account_id in account_ids {
        let Some(account_cfg) = matrix_cfg.accounts.get(&account_id) else {
            continue;
        };
        if !account_cfg.enabled
            || account_cfg.access_token.trim().is_empty()
            || account_cfg.homeserver_url.trim().is_empty()
            || account_cfg.bot_user_id.trim().is_empty()
        {
            continue;
        }

        let is_default = default_account
            .as_deref()
            .map(|v| v == account_id.as_str())
            .unwrap_or(false);
        let channel_name = if is_default {
            "matrix".to_string()
        } else {
            format!("matrix.{account_id}")
        };

        let bot_username = if account_cfg.bot_username.trim().is_empty() {
            config.bot_username_for_channel(&channel_name)
        } else {
            account_cfg.bot_username.trim().to_string()
        };

        runtimes.push(MatrixRuntimeContext {
            channel_name,
            access_token: account_cfg.access_token.clone(),
            homeserver_url: account_cfg.homeserver_url.clone(),
            bot_user_id: account_cfg.bot_user_id.clone(),
            bot_username,
            allowed_room_ids: account_cfg.allowed_room_ids.clone(),
            mention_required: account_cfg.mention_required,
            sync_timeout_ms: account_cfg.sync_timeout_ms,
        });
    }

    if runtimes.is_empty()
        && !matrix_cfg.access_token.trim().is_empty()
        && !matrix_cfg.homeserver_url.trim().is_empty()
        && !matrix_cfg.bot_user_id.trim().is_empty()
    {
        runtimes.push(MatrixRuntimeContext {
            channel_name: "matrix".to_string(),
            access_token: matrix_cfg.access_token,
            homeserver_url: matrix_cfg.homeserver_url,
            bot_user_id: matrix_cfg.bot_user_id,
            bot_username: if matrix_cfg.bot_username.trim().is_empty() {
                config.bot_username_for_channel("matrix")
            } else {
                matrix_cfg.bot_username.trim().to_string()
            },
            allowed_room_ids: matrix_cfg.allowed_room_ids,
            mention_required: matrix_cfg.mention_required,
            sync_timeout_ms: matrix_cfg.sync_timeout_ms,
        });
    }

    runtimes
}

pub struct MatrixAdapter {
    name: String,
    homeserver_url: String,
    access_token: String,
    http_client: reqwest::Client,
}

impl MatrixAdapter {
    pub fn new(name: String, homeserver_url: String, access_token: String) -> Self {
        Self {
            name,
            homeserver_url: homeserver_url.trim_end_matches('/').to_string(),
            access_token,
            http_client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat_type_routes(&self) -> Vec<(&str, ConversationKind)> {
        vec![
            ("matrix", ConversationKind::Group),
            ("matrix_dm", ConversationKind::Private),
        ]
    }

    async fn send_text(&self, external_chat_id: &str, text: &str) -> Result<(), String> {
        send_matrix_text(
            &self.http_client,
            &self.homeserver_url,
            &self.access_token,
            external_chat_id,
            text,
        )
        .await
    }

    async fn send_attachment(
        &self,
        _external_chat_id: &str,
        _file_path: &Path,
        _caption: Option<&str>,
    ) -> Result<String, String> {
        Err("attachments not supported for matrix".to_string())
    }
}

struct MatrixIncomingMessage {
    room_id: String,
    sender: String,
    event_id: String,
    body: String,
}

pub async fn start_matrix_bot(app_state: Arc<AppState>, runtime: MatrixRuntimeContext) {
    let mut since: Option<String> = None;
    let mut bootstrapped = false;

    loop {
        match sync_matrix_messages(&runtime, since.as_deref()).await {
            Ok((next_batch, messages)) => {
                since = Some(next_batch);

                if !bootstrapped {
                    bootstrapped = true;
                    continue;
                }

                for msg in messages {
                    let state = app_state.clone();
                    let runtime_ctx = runtime.clone();
                    tokio::spawn(async move {
                        handle_matrix_message(state, runtime_ctx, msg).await;
                    });
                }
            }
            Err(e) => {
                warn!(
                    "Matrix adapter '{}' sync error: {e}",
                    runtime.channel_name.as_str()
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn sync_matrix_messages(
    runtime: &MatrixRuntimeContext,
    since: Option<&str>,
) -> Result<(String, Vec<MatrixIncomingMessage>), String> {
    let homeserver_url = runtime.normalized_homeserver_url();
    let url = format!("{homeserver_url}/_matrix/client/v3/sync");

    let timeout_ms = if since.is_some() {
        runtime.sync_timeout_ms_or_default()
    } else {
        0
    };

    let client = reqwest::Client::new();
    let mut request = client
        .get(&url)
        .bearer_auth(runtime.access_token.trim())
        .query(&[("timeout", timeout_ms)]);

    if let Some(since_token) = since {
        request = request.query(&[("since", since_token)]);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Matrix /sync request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Matrix /sync failed: HTTP {status} {}",
            body.chars().take(300).collect::<String>()
        ));
    }

    let payload: Value = response
        .json()
        .await
        .map_err(|e| format!("Matrix /sync response parse failed: {e}"))?;

    let next_batch = payload
        .get("next_batch")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Matrix /sync response missing next_batch".to_string())?;

    let mut incoming = Vec::new();

    let joined_rooms = payload
        .pointer("/rooms/join")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    for (room_id, room_data) in joined_rooms {
        if !runtime.should_process_room(&room_id) {
            continue;
        }

        let Some(events) = room_data
            .pointer("/timeline/events")
            .and_then(|v| v.as_array())
        else {
            continue;
        };

        for event in events {
            if event.get("type").and_then(|v| v.as_str()) != Some("m.room.message") {
                continue;
            }

            let sender = event
                .get("sender")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if sender.trim().is_empty() || sender == runtime.bot_user_id {
                continue;
            }

            let body = event
                .pointer("/content/body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if body.trim().is_empty() {
                continue;
            }

            let event_id = event
                .get("event_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            incoming.push(MatrixIncomingMessage {
                room_id: room_id.clone(),
                sender,
                event_id,
                body,
            });
        }
    }

    Ok((next_batch, incoming))
}

async fn send_matrix_text(
    client: &reqwest::Client,
    homeserver_url: &str,
    access_token: &str,
    room_id: &str,
    text: &str,
) -> Result<(), String> {
    let homeserver = homeserver_url.trim_end_matches('/');
    for chunk in split_text(text, 3800) {
        let txn_id = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{homeserver}/_matrix/client/v3/rooms/{}/send/m.room.message/{txn_id}",
            urlencoding::encode(room_id)
        );

        let body = serde_json::json!({
            "msgtype": "m.text",
            "body": chunk,
        });

        let response = client
            .put(&url)
            .bearer_auth(access_token.trim())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Matrix send request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!(
                "Matrix send failed: HTTP {status} {}",
                body.chars().take(300).collect::<String>()
            ));
        }
    }

    Ok(())
}

async fn handle_matrix_message(
    app_state: Arc<AppState>,
    runtime: MatrixRuntimeContext,
    msg: MatrixIncomingMessage,
) {
    if !runtime.should_respond(&msg.body) {
        return;
    }

    let chat_id = call_blocking(app_state.db.clone(), {
        let room_id = msg.room_id.clone();
        let title = format!("matrix-{}", room_id);
        let chat_type = "matrix".to_string();
        let channel_name = runtime.channel_name.clone();
        move |db| db.resolve_or_create_chat_id(&channel_name, &room_id, Some(&title), &chat_type)
    })
    .await
    .unwrap_or(0);

    if chat_id == 0 {
        error!("Matrix: failed to resolve chat ID for room {}", msg.room_id);
        return;
    }

    let incoming = StoredMessage {
        id: if msg.event_id.trim().is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            msg.event_id.clone()
        },
        chat_id,
        sender_name: msg.sender.clone(),
        content: msg.body.clone(),
        is_from_bot: false,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let _ = call_blocking(app_state.db.clone(), move |db| db.store_message(&incoming)).await;

    let trimmed = msg.body.trim();
    if trimmed == "/reset" {
        let _ = call_blocking(app_state.db.clone(), move |db| {
            db.clear_chat_context(chat_id)
        })
        .await;
        let _ = send_matrix_text(
            &reqwest::Client::new(),
            &runtime.homeserver_url,
            &runtime.access_token,
            &msg.room_id,
            "Context cleared (session + chat history).",
        )
        .await;
        return;
    }

    if trimmed == "/skills" {
        let formatted = app_state.skills.list_skills_formatted();
        let _ = send_matrix_text(
            &reqwest::Client::new(),
            &runtime.homeserver_url,
            &runtime.access_token,
            &msg.room_id,
            &formatted,
        )
        .await;
        return;
    }

    if trimmed == "/reload-skills" {
        let reloaded = app_state.skills.reload();
        let text = format!("Reloaded {} skills from disk.", reloaded.len());
        let _ = send_matrix_text(
            &reqwest::Client::new(),
            &runtime.homeserver_url,
            &runtime.access_token,
            &msg.room_id,
            &text,
        )
        .await;
        return;
    }

    if trimmed == "/archive" {
        if let Ok(Some((json, _))) =
            call_blocking(app_state.db.clone(), move |db| db.load_session(chat_id)).await
        {
            let messages: Vec<LlmMessage> = serde_json::from_str(&json).unwrap_or_default();
            if messages.is_empty() {
                let _ = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    "No session to archive.",
                )
                .await;
            } else {
                archive_conversation(
                    &app_state.config.data_dir,
                    &runtime.channel_name,
                    chat_id,
                    &messages,
                );
                let _ = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    &format!("Archived {} messages.", messages.len()),
                )
                .await;
            }
        } else {
            let _ = send_matrix_text(
                &reqwest::Client::new(),
                &runtime.homeserver_url,
                &runtime.access_token,
                &msg.room_id,
                "No session to archive.",
            )
            .await;
        }
        return;
    }

    if trimmed == "/usage" {
        match build_usage_report(app_state.db.clone(), chat_id).await {
            Ok(report) => {
                let _ = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    &report,
                )
                .await;
            }
            Err(e) => {
                let _ = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    &format!("Failed to query usage statistics: {e}"),
                )
                .await;
            }
        }
        return;
    }

    info!(
        "Matrix message from {} in {}: {}",
        msg.sender,
        msg.room_id,
        msg.body.chars().take(100).collect::<String>()
    );

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    match process_with_agent_with_events(
        &app_state,
        AgentRequestContext {
            caller_channel: &runtime.channel_name,
            chat_id,
            chat_type: "group",
        },
        None,
        None,
        Some(&event_tx),
    )
    .await
    {
        Ok(response) => {
            drop(event_tx);
            let mut used_send_message_tool = false;
            while let Some(event) = event_rx.recv().await {
                if let AgentEvent::ToolStart { name } = event {
                    if name == "send_message" {
                        used_send_message_tool = true;
                    }
                }
            }

            if !response.is_empty() {
                if let Err(e) = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    &response,
                )
                .await
                {
                    error!("Matrix: failed to send response: {e}");
                }

                let bot_msg = StoredMessage {
                    id: uuid::Uuid::new_v4().to_string(),
                    chat_id,
                    sender_name: runtime.bot_username.clone(),
                    content: response,
                    is_from_bot: true,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                let _ =
                    call_blocking(app_state.db.clone(), move |db| db.store_message(&bot_msg)).await;
            } else if !used_send_message_tool {
                let fallback =
                    "I couldn't produce a visible reply after an automatic retry. Please try again.";
                let _ = send_matrix_text(
                    &reqwest::Client::new(),
                    &runtime.homeserver_url,
                    &runtime.access_token,
                    &msg.room_id,
                    fallback,
                )
                .await;

                let bot_msg = StoredMessage {
                    id: uuid::Uuid::new_v4().to_string(),
                    chat_id,
                    sender_name: runtime.bot_username.clone(),
                    content: fallback.to_string(),
                    is_from_bot: true,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                let _ =
                    call_blocking(app_state.db.clone(), move |db| db.store_message(&bot_msg)).await;
            }
        }
        Err(e) => {
            error!("Error processing Matrix message: {e}");
            let _ = send_matrix_text(
                &reqwest::Client::new(),
                &runtime.homeserver_url,
                &runtime.access_token,
                &msg.room_id,
                &format!("Error: {e}"),
            )
            .await;
        }
    }
}
