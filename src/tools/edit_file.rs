use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::claude::ToolDefinition;

use super::{schema_object, Tool, ToolResult};

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit_file".into(),
            description: "Edit a file by replacing an exact string match with new content. The old_string must be unique in the file.".into(),
            input_schema: schema_object(
                json!({
                    "path": {
                        "type": "string",
                        "description": "The file path to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact string to find and replace (must be unique in the file)"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The string to replace with"
                    }
                }),
                &["path", "old_string", "new_string"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("Missing 'path' parameter".into()),
        };
        let old_string = match input.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("Missing 'old_string' parameter".into()),
        };
        let new_string = match input.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("Missing 'new_string' parameter".into()),
        };

        info!("Editing file: {}", path);

        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read file: {e}")),
        };

        let count = content.matches(old_string).count();
        if count == 0 {
            return ToolResult::error(
                "old_string not found in file. Make sure the string matches exactly.".into(),
            );
        }
        if count > 1 {
            return ToolResult::error(format!(
                "old_string found {count} times in file. It must be unique. Provide more context to make it unique."
            ));
        }

        let new_content = content.replacen(old_string, new_string, 1);
        match tokio::fs::write(path, new_content).await {
            Ok(()) => ToolResult::success(format!("Successfully edited {path}")),
            Err(e) => ToolResult::error(format!("Failed to write file: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup_file(content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("microclaw_ef_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("edit_me.txt");
        std::fs::write(&file, content).unwrap();
        (dir, file)
    }

    #[tokio::test]
    async fn test_edit_file_success() {
        let (dir, file) = setup_file("hello world");
        let tool = EditFileTool;
        let result = tool
            .execute(json!({
                "path": file.to_str().unwrap(),
                "old_string": "world",
                "new_string": "rust"
            }))
            .await;
        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello rust");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_file_not_found() {
        let tool = EditFileTool;
        let result = tool
            .execute(json!({
                "path": "/nonexistent/file.txt",
                "old_string": "a",
                "new_string": "b"
            }))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read file"));
    }

    #[tokio::test]
    async fn test_edit_file_old_string_not_found() {
        let (dir, file) = setup_file("hello world");
        let tool = EditFileTool;
        let result = tool
            .execute(json!({
                "path": file.to_str().unwrap(),
                "old_string": "xyz",
                "new_string": "abc"
            }))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found in file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_file_multiple_matches() {
        let (dir, file) = setup_file("aaa bbb aaa");
        let tool = EditFileTool;
        let result = tool
            .execute(json!({
                "path": file.to_str().unwrap(),
                "old_string": "aaa",
                "new_string": "ccc"
            }))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("2 times"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_edit_file_missing_params() {
        let tool = EditFileTool;
        let result = tool.execute(json!({})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'path'"));

        let result = tool
            .execute(json!({"path": "/tmp/x"}))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'old_string'"));
    }
}
