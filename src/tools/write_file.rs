use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::claude::ToolDefinition;

use super::{schema_object, Tool, ToolResult};

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file. Creates the file and any parent directories if they don't exist. Overwrites existing content.".into(),
            input_schema: schema_object(
                json!({
                    "path": {
                        "type": "string",
                        "description": "The file path to write to"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write"
                    }
                }),
                &["path", "content"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("Missing 'path' parameter".into()),
        };

        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolResult::error("Missing 'content' parameter".into()),
        };

        info!("Writing file: {}", path);

        if let Some(parent) = std::path::Path::new(path).parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::error(format!("Failed to create directories: {e}"));
            }
        }

        match tokio::fs::write(path, content).await {
            Ok(()) => ToolResult::success(format!("Successfully wrote to {path}")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_write_file_success() {
        let dir = std::env::temp_dir().join(format!("microclaw_wf_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("out.txt");

        let tool = WriteFileTool;
        let result = tool
            .execute(json!({"path": file.to_str().unwrap(), "content": "hello world"}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("Successfully wrote"));

        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "hello world");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!("microclaw_wf2_{}", uuid::Uuid::new_v4()));
        let file = dir.join("sub").join("dir").join("file.txt");

        let tool = WriteFileTool;
        let result = tool
            .execute(json!({"path": file.to_str().unwrap(), "content": "nested"}))
            .await;
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "nested");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_write_file_missing_params() {
        let tool = WriteFileTool;

        let result = tool.execute(json!({"content": "hello"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'path'"));

        let result = tool.execute(json!({"path": "/tmp/x"})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'content'"));
    }
}
