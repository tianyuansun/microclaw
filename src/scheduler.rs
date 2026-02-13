use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use tracing::{error, info};

use crate::agent_engine::process_with_agent;
use crate::agent_engine::AgentRequestContext;
use crate::channel::{
    deliver_and_store_bot_message, get_chat_routing, ChatChannel, ChatRouting, ConversationKind,
};
use crate::db::call_blocking;
use crate::llm_types::{Message, MessageContent, ResponseContentBlock};
use crate::runtime::AppState;
use crate::text::floor_char_boundary;

pub fn spawn_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        info!("Scheduler started");
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            run_due_tasks(&state).await;
        }
    });
}

async fn run_due_tasks(state: &Arc<AppState>) {
    let now = Utc::now().to_rfc3339();
    let tasks = match call_blocking(state.db.clone(), move |db| db.get_due_tasks(&now)).await {
        Ok(t) => t,
        Err(e) => {
            error!("Scheduler: failed to query due tasks: {e}");
            return;
        }
    };

    for task in tasks {
        info!(
            "Scheduler: executing task #{} for chat {}",
            task.id, task.chat_id
        );

        let started_at = Utc::now();
        let started_at_str = started_at.to_rfc3339();
        let routing = get_chat_routing(state.db.clone(), task.chat_id)
            .await
            .ok()
            .flatten()
            .unwrap_or(ChatRouting {
                channel: ChatChannel::Telegram,
                conversation: ConversationKind::Private,
            });

        // Run agent loop with the task prompt
        let (success, result_summary) = match process_with_agent(
            state,
            AgentRequestContext {
                caller_channel: routing.channel.as_caller_channel(),
                chat_id: task.chat_id,
                chat_type: routing.conversation.as_agent_chat_type(),
            },
            Some(&task.prompt),
            None,
        )
        .await
        {
            Ok(response) => {
                if !response.is_empty() {
                    let _ = deliver_and_store_bot_message(
                        state.telegram_bot.as_ref(),
                        Some(&state.config),
                        state.db.clone(),
                        &state.config.bot_username,
                        task.chat_id,
                        &response,
                    )
                    .await;
                }
                let summary = if response.len() > 200 {
                    format!("{}...", &response[..floor_char_boundary(&response, 200)])
                } else {
                    response
                };
                (true, Some(summary))
            }
            Err(e) => {
                error!("Scheduler: task #{} failed: {e}", task.id);
                let err_text = format!("Scheduled task #{} failed: {e}", task.id);
                let _ = deliver_and_store_bot_message(
                    state.telegram_bot.as_ref(),
                    Some(&state.config),
                    state.db.clone(),
                    &state.config.bot_username,
                    task.chat_id,
                    &err_text,
                )
                .await;
                (false, Some(format!("Error: {e}")))
            }
        };

        let finished_at = Utc::now();
        let finished_at_str = finished_at.to_rfc3339();
        let duration_ms = (finished_at - started_at).num_milliseconds();

        // Log the task run
        let log_summary = result_summary.clone();
        let started_for_log = started_at_str.clone();
        let finished_for_log = finished_at_str.clone();
        if let Err(e) = call_blocking(state.db.clone(), move |db| {
            db.log_task_run(
                task.id,
                task.chat_id,
                &started_for_log,
                &finished_for_log,
                duration_ms,
                success,
                log_summary.as_deref(),
            )?;
            Ok(())
        })
        .await
        {
            error!("Scheduler: failed to log task run for #{}: {e}", task.id);
        }

        // Compute next run
        let tz: chrono_tz::Tz = state.config.timezone.parse().unwrap_or(chrono_tz::Tz::UTC);
        let next_run = if task.schedule_type == "cron" {
            match cron::Schedule::from_str(&task.schedule_value) {
                Ok(schedule) => schedule.upcoming(tz).next().map(|t| t.to_rfc3339()),
                Err(e) => {
                    error!("Scheduler: invalid cron for task #{}: {e}", task.id);
                    None
                }
            }
        } else {
            None // one-shot
        };

        let started_for_update = started_at_str.clone();
        if let Err(e) = call_blocking(state.db.clone(), move |db| {
            db.update_task_after_run(task.id, &started_for_update, next_run.as_deref())?;
            Ok(())
        })
        .await
        {
            error!("Scheduler: failed to update task #{}: {e}", task.id);
        }
    }
}

const REFLECTOR_SYSTEM_PROMPT: &str = r#"You are a memory extraction specialist. Extract durable, factual information from conversations.

Rules:
- Extract ONLY concrete facts, preferences, expertise, or notable events
- IGNORE: greetings, small talk, unanswered questions, transient requests
- Each memory < 150 characters, specific and concrete
- Category must be exactly one of: PROFILE (user attributes/preferences), KNOWLEDGE (facts/expertise), EVENT (significant things that happened)
- Output ONLY valid JSON array: [{"content":"...","category":"PROFILE"}]
- If nothing worth remembering: []"#;

fn jaccard_similar(a: &str, b: &str, threshold: f64) -> bool {
    use std::collections::HashSet;
    let a_words: HashSet<&str> = a.split_whitespace().collect();
    let b_words: HashSet<&str> = b.split_whitespace().collect();
    let intersection = a_words.intersection(&b_words).count();
    let union = a_words.len() + b_words.len() - intersection;
    if union == 0 {
        return true;
    }
    intersection as f64 / union as f64 >= threshold
}

pub fn spawn_reflector(state: Arc<AppState>) {
    if !state.config.reflector_enabled {
        info!("Reflector disabled by config");
        return;
    }
    let interval_secs = state.config.reflector_interval_mins * 60;
    tokio::spawn(async move {
        info!(
            "Reflector started (interval: {}min)",
            state.config.reflector_interval_mins
        );
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            run_reflector(&state).await;
        }
    });
}

async fn run_reflector(state: &Arc<AppState>) {
    let lookback_secs = (state.config.reflector_interval_mins * 2 * 60) as i64;
    let since = (Utc::now() - chrono::Duration::seconds(lookback_secs)).to_rfc3339();

    let chat_ids = match call_blocking(state.db.clone(), move |db| {
        db.get_active_chat_ids_since(&since)
    })
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            error!("Reflector: failed to get active chats: {e}");
            return;
        }
    };

    for chat_id in chat_ids {
        reflect_for_chat(state, chat_id).await;
    }
}

async fn reflect_for_chat(state: &Arc<AppState>, chat_id: i64) {
    // 1. Get recent messages
    let messages = match call_blocking(state.db.clone(), move |db| {
        db.get_recent_messages(chat_id, 30)
    })
    .await
    {
        Ok(m) => m,
        Err(_) => return,
    };

    if messages.is_empty() {
        return;
    }

    // 2. Format conversation for the LLM
    let conversation = messages
        .iter()
        .map(|m| format!("[{}]: {}", m.sender_name, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Call LLM directly (no tools, no session)
    let user_msg = Message {
        role: "user".into(),
        content: MessageContent::Text(format!(
            "Extract memories from this conversation (chat_id={chat_id}):\n\n{conversation}"
        )),
    };
    let response = match state
        .llm
        .send_message(REFLECTOR_SYSTEM_PROMPT, vec![user_msg], None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Reflector: LLM call failed for chat {chat_id}: {e}");
            return;
        }
    };

    // 4. Extract text from response
    let text = response
        .content
        .iter()
        .filter_map(|b| {
            if let ResponseContentBlock::Text { text } = b {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");

    // 5. Parse JSON array
    let extracted: Vec<serde_json::Value> = match serde_json::from_str(text.trim()) {
        Ok(v) => v,
        Err(_) => {
            let start = text.find('[').unwrap_or(0);
            let end = text.rfind(']').map(|i| i + 1).unwrap_or(text.len());
            if start >= end {
                error!("Reflector: parse failed for chat {chat_id}: no JSON array found");
                return;
            }
            match serde_json::from_str(&text[start..end]) {
                Ok(v) => v,
                Err(e) => {
                    error!("Reflector: parse failed for chat {chat_id}: {e}");
                    return;
                }
            }
        }
    };

    if extracted.is_empty() {
        return;
    }

    // 6. Load existing memories for dedup
    let existing = match call_blocking(state.db.clone(), move |db| {
        db.get_all_memories_for_chat(Some(chat_id))
    })
    .await
    {
        Ok(m) => m,
        Err(_) => return,
    };

    // 7. Insert non-duplicate memories
    let mut inserted = 0usize;
    for item in &extracted {
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let category = item
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("KNOWLEDGE");
        let content = content.trim();
        if content.is_empty() || content.len() > 300 {
            continue;
        }

        if existing
            .iter()
            .any(|m| jaccard_similar(&m.content, content, 0.5))
        {
            continue;
        }

        let content = content.to_string();
        let category = category.to_string();
        if call_blocking(state.db.clone(), move |db| {
            db.insert_memory(Some(chat_id), &content, &category)
        })
        .await
        .is_ok()
        {
            inserted += 1;
        }
    }

    if inserted > 0 {
        info!("Reflector: chat {chat_id} → {inserted} new memories");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_similar_identical() {
        assert!(jaccard_similar("hello world", "hello world", 0.5));
    }

    #[test]
    fn test_jaccard_similar_no_overlap() {
        assert!(!jaccard_similar("hello world", "foo bar", 0.5));
    }

    #[test]
    fn test_jaccard_similar_partial_overlap() {
        // "a b c" vs "a b d" => intersection=2, union=4 => 0.5 >= 0.5
        assert!(jaccard_similar("a b c", "a b d", 0.5));
        // "a b c" vs "a d e" => intersection=1, union=5 => 0.2 < 0.5
        assert!(!jaccard_similar("a b c", "a d e", 0.5));
    }

    #[test]
    fn test_jaccard_similar_empty_strings() {
        // Both empty => union=0 => returns true
        assert!(jaccard_similar("", "", 0.5));
        // One empty => intersection=0, union=1 => 0.0 < 0.5
        assert!(!jaccard_similar("hello", "", 0.5));
    }
}
