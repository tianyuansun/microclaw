use async_trait::async_trait;
use serde_json::json;

use super::{schema_object, Tool, ToolResult};
use crate::config::WebSearchConfig;
use microclaw_core::llm_types::ToolDefinition;
use microclaw_tools::web_search;

pub struct WebSearchTool {
    config: WebSearchConfig,
}

impl WebSearchTool {
    pub fn new(config: WebSearchConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn definition(&self) -> ToolDefinition {
        let description = if self.config.searxng_endpoint.is_some() {
            "Search the web using SearXNG (with Tavily fallback if configured). Returns titles, URLs, and snippets."
        } else if self.config.tavily_api_key.is_some() {
            "Search the web using Tavily. Returns titles, URLs, and snippets."
        } else {
            "Search the web. Requires searxng_endpoint or tavily_api_key in config."
        };

        ToolDefinition {
            name: "web_search".into(),
            description: description.into(),
            input_schema: schema_object(
                json!({
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    }
                }),
                &["query"],
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let query = match parse_query(&input) {
            Ok(q) => q,
            Err(msg) => return ToolResult::error(msg),
        };

        match web_search::search(&query, &self.config).await {
            Ok(results) => {
                if results.is_empty() {
                    ToolResult::success("No results found.".into())
                } else {
                    ToolResult::success(web_search::format_results(&results))
                }
            }
            Err(e) => ToolResult::error(format!("Search failed: {e}")),
        }
    }
}

fn parse_query(input: &serde_json::Value) -> Result<String, String> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("Missing required parameter: query".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_web_search_definition() {
        let config = WebSearchConfig {
            searxng_endpoint: Some("https://search.example.com".into()),
            tavily_api_key: None,
            max_results: 8,
            timeout_secs: 15,
        };
        let tool = WebSearchTool::new(config);
        assert_eq!(tool.name(), "web_search");
        let def = tool.definition();
        assert_eq!(def.name, "web_search");
        assert!(def.description.contains("SearXNG"));
        assert!(def.input_schema["properties"]["query"].is_object());
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[tokio::test]
    async fn test_web_search_missing_query() {
        let tool = WebSearchTool::new(WebSearchConfig::default());
        let result = tool.execute(json!({})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required parameter: query"));
    }

    #[tokio::test]
    async fn test_web_search_null_query() {
        let tool = WebSearchTool::new(WebSearchConfig::default());
        let result = tool.execute(json!({"query": null})).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required parameter: query"));
    }

    #[tokio::test]
    async fn test_web_search_empty_query() {
        let tool = WebSearchTool::new(WebSearchConfig::default());
        let result = tool.execute(json!({"query": "   " })).await;
        assert!(result.is_error);
        assert!(result.content.contains("Missing required parameter: query"));
    }
}
