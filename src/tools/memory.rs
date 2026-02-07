use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use tracing::info;

use crate::claude::ToolDefinition;

use super::{schema_object, Tool, ToolResult};

pub struct ReadMemoryTool {
    groups_dir: PathBuf,
}

impl ReadMemoryTool {
    pub fn new(data_dir: &str) -> Self {
        ReadMemoryTool {
            groups_dir: PathBuf::from(data_dir).join("groups"),
        }
    }
}

#[async_trait]
impl Tool for ReadMemoryTool {
    fn name(&self) -> &str {
        "read_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_memory".into(),
            description: "Read the CLAUDE.md memory file. Use scope 'global' for memories shared across all chats, or 'chat' for chat-specific memories.".into(),
            input_schema: schema_object(
                json!({
                    "scope": {
                        "type": "string",
                        "description": "Memory scope: 'global' or 'chat'",
                        "enum": ["global", "chat"]
                    },
                    "chat_id": {
                        "type": "integer",
                        "description": "Chat ID (required for scope 'chat')"
                    }
                }),
                &["scope"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let scope = match input.get("scope").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("Missing 'scope' parameter".into()),
        };

        let path = match scope {
            "global" => self.groups_dir.join("CLAUDE.md"),
            "chat" => {
                let chat_id = match input.get("chat_id").and_then(|v| v.as_i64()) {
                    Some(id) => id,
                    None => return ToolResult::error("Missing 'chat_id' for chat scope".into()),
                };
                self.groups_dir.join(chat_id.to_string()).join("CLAUDE.md")
            }
            _ => return ToolResult::error("scope must be 'global' or 'chat'".into()),
        };

        info!("Reading memory: {}", path.display());

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if content.trim().is_empty() {
                    ToolResult::success("Memory file is empty.".into())
                } else {
                    ToolResult::success(content)
                }
            }
            Err(_) => ToolResult::success("No memory file found (not yet created).".into()),
        }
    }
}

pub struct WriteMemoryTool {
    groups_dir: PathBuf,
}

impl WriteMemoryTool {
    pub fn new(data_dir: &str) -> Self {
        WriteMemoryTool {
            groups_dir: PathBuf::from(data_dir).join("groups"),
        }
    }
}

#[async_trait]
impl Tool for WriteMemoryTool {
    fn name(&self) -> &str {
        "write_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_memory".into(),
            description: "Write to the CLAUDE.md memory file. Use this to remember important information about the user or conversation. Use scope 'global' for memories shared across all chats, or 'chat' for chat-specific memories.".into(),
            input_schema: schema_object(
                json!({
                    "scope": {
                        "type": "string",
                        "description": "Memory scope: 'global' or 'chat'",
                        "enum": ["global", "chat"]
                    },
                    "chat_id": {
                        "type": "integer",
                        "description": "Chat ID (required for scope 'chat')"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the memory file (replaces existing content)"
                    }
                }),
                &["scope", "content"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let scope = match input.get("scope").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("Missing 'scope' parameter".into()),
        };
        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("Missing 'content' parameter".into()),
        };

        let path = match scope {
            "global" => self.groups_dir.join("CLAUDE.md"),
            "chat" => {
                let chat_id = match input.get("chat_id").and_then(|v| v.as_i64()) {
                    Some(id) => id,
                    None => return ToolResult::error("Missing 'chat_id' for chat scope".into()),
                };
                self.groups_dir.join(chat_id.to_string()).join("CLAUDE.md")
            }
            _ => return ToolResult::error("scope must be 'global' or 'chat'".into()),
        };

        info!("Writing memory: {}", path.display());

        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolResult::error(format!("Failed to create directory: {e}"));
            }
        }

        match std::fs::write(&path, content) {
            Ok(()) => ToolResult::success(format!("Memory saved to {} scope.", scope)),
            Err(e) => ToolResult::error(format!("Failed to write memory: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("microclaw_memtool_{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn test_read_memory_global_not_exists() {
        let dir = test_dir();
        let tool = ReadMemoryTool::new(dir.to_str().unwrap());
        let result = tool.execute(json!({"scope": "global"})).await;
        assert!(!result.is_error);
        assert!(result.content.contains("No memory file found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_and_read_memory_global() {
        let dir = test_dir();
        let write_tool = WriteMemoryTool::new(dir.to_str().unwrap());
        let read_tool = ReadMemoryTool::new(dir.to_str().unwrap());

        let result = write_tool
            .execute(json!({"scope": "global", "content": "user prefers Rust"}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("Memory saved"));

        let result = read_tool.execute(json!({"scope": "global"})).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "user prefers Rust");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_and_read_memory_chat() {
        let dir = test_dir();
        let write_tool = WriteMemoryTool::new(dir.to_str().unwrap());
        let read_tool = ReadMemoryTool::new(dir.to_str().unwrap());

        let result = write_tool
            .execute(json!({"scope": "chat", "chat_id": 42, "content": "chat 42 notes"}))
            .await;
        assert!(!result.is_error);

        let result = read_tool
            .execute(json!({"scope": "chat", "chat_id": 42}))
            .await;
        assert!(!result.is_error);
        assert_eq!(result.content, "chat 42 notes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_memory_chat_missing_chat_id() {
        let dir = test_dir();
        let tool = ReadMemoryTool::new(dir.to_str().unwrap());
        let result = tool.execute(json!({"scope": "chat"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'chat_id'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_memory_missing_scope() {
        let dir = test_dir();
        let tool = WriteMemoryTool::new(dir.to_str().unwrap());
        let result = tool.execute(json!({"content": "data"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'scope'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_memory_invalid_scope() {
        let dir = test_dir();
        let tool = ReadMemoryTool::new(dir.to_str().unwrap());
        let result = tool.execute(json!({"scope": "invalid"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("must be 'global' or 'chat'"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_read_memory_empty_file() {
        let dir = test_dir();
        let write_tool = WriteMemoryTool::new(dir.to_str().unwrap());
        let read_tool = ReadMemoryTool::new(dir.to_str().unwrap());

        write_tool
            .execute(json!({"scope": "global", "content": "   "}))
            .await;

        let result = read_tool.execute(json!({"scope": "global"})).await;
        assert!(!result.is_error);
        assert!(result.content.contains("empty"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
