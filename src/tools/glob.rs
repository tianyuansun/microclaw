use async_trait::async_trait;
use serde_json::json;
use tracing::info;

use crate::claude::ToolDefinition;

use super::{schema_object, Tool, ToolResult};

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "glob".into(),
            description: "Find files matching a glob pattern. Returns matching file paths.".into(),
            input_schema: schema_object(
                json!({
                    "pattern": {
                        "type": "string",
                        "description": "The glob pattern to match (e.g., '**/*.rs', 'src/**/*.ts')"
                    },
                    "path": {
                        "type": "string",
                        "description": "Base directory to search from (default: current directory)"
                    }
                }),
                &["pattern"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("Missing 'pattern' parameter".into()),
        };
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        info!("Glob: {} in {}", pattern, base);

        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{base}/{pattern}")
        };

        match glob::glob(&full_pattern) {
            Ok(paths) => {
                let mut matches: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.display().to_string())
                    .collect();
                matches.sort();

                if matches.is_empty() {
                    ToolResult::success("No files found matching pattern.".into())
                } else {
                    let count = matches.len();
                    if count > 500 {
                        matches.truncate(500);
                        matches.push(format!("... and {} more files", count - 500));
                    }
                    ToolResult::success(matches.join("\n"))
                }
            }
            Err(e) => ToolResult::error(format!("Invalid glob pattern: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_glob_finds_files() {
        let dir = std::env::temp_dir().join(format!("microclaw_glob_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("c.rs"), "").unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(json!({"pattern": "*.txt", "path": dir.to_str().unwrap()}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("b.txt"));
        assert!(!result.content.contains("c.rs"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = std::env::temp_dir().join(format!("microclaw_glob2_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let tool = GlobTool;
        let result = tool
            .execute(json!({"pattern": "*.xyz", "path": dir.to_str().unwrap()}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("No files found"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_glob_missing_pattern() {
        let tool = GlobTool;
        let result = tool.execute(json!({})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing 'pattern'"));
    }
}
