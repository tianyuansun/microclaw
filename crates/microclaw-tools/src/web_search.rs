use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::web_html::SearchItem;
use serde::{Deserialize, Serialize};

fn http_client(timeout_secs: u64) -> reqwest::Client {
    static CLIENTS: OnceLock<Mutex<HashMap<u64, reqwest::Client>>> = OnceLock::new();
    let cache = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(client) = cache.get(&timeout_secs) {
        return client.clone();
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("MicroClaw/1.0")
        .build()
        .expect("failed to build HTTP client");
    cache.insert(timeout_secs, client.clone());
    client
}

/// Default maximum number of search results
const DEFAULT_MAX_RESULTS: usize = 8;
/// Default timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 15;

fn default_max_results() -> usize {
    DEFAULT_MAX_RESULTS
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// Configuration for web search providers
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// SearXNG instance URL (e.g., "https://search.example.com")
    pub searxng_endpoint: Option<String>,
    /// Tavily API key for fallback search
    pub tavily_api_key: Option<String>,
    /// Maximum number of search results (default: 8)
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    /// Timeout in seconds for search requests (default: 15)
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            searxng_endpoint: None,
            tavily_api_key: None,
            max_results: DEFAULT_MAX_RESULTS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

/// Search result from any provider
#[derive(Debug, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

impl From<SearchItem> for WebSearchResult {
    fn from(item: SearchItem) -> Self {
        WebSearchResult {
            title: item.title,
            url: item.url,
            snippet: item.snippet,
        }
    }
}

/// Search the web using configured providers.
/// Priority: SearXNG -> Tavily (fallback)
pub async fn search(query: &str, config: &WebSearchConfig) -> Result<Vec<WebSearchResult>, String> {
    let timeout_secs = config.timeout_secs.max(1);
    let max_results = config.max_results.max(1);

    // Try SearXNG first if configured
    if let Some(ref endpoint) = config.searxng_endpoint {
        match search_searxng(endpoint, query, timeout_secs, max_results).await {
            Ok(results) => return Ok(results),
            Err(e) => {
                tracing::warn!("SearXNG search failed: {e}, trying fallback");
            }
        }
    }

    // Fallback to Tavily if configured
    if let Some(ref api_key) = config.tavily_api_key {
        return search_tavily(api_key, query, timeout_secs, max_results).await;
    }

    Err("No web search provider configured. Set searxng_endpoint or tavily_api_key.".to_string())
}

/// Search using SearXNG JSON API
pub async fn search_searxng(endpoint: &str, query: &str, timeout_secs: u64, max_results: usize) -> Result<Vec<WebSearchResult>, String> {
    let client = http_client(timeout_secs);

    // Build URL with query parameters
    let base = endpoint.trim_end_matches('/');
    let url = format!("{}/search?q={}&format=json", base, urlencoding::encode(query));

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let results = json
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let title = item.get("title")?.as_str()?.to_string();
                    let url = item.get("url")?.as_str()?.to_string();
                    let snippet = item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(WebSearchResult { title, url, snippet })
                })
                .take(max_results)
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}

/// Search using Tavily API
pub async fn search_tavily(api_key: &str, query: &str, timeout_secs: u64, max_results: usize) -> Result<Vec<WebSearchResult>, String> {
    let client = http_client(timeout_secs);

    let body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "search_depth": "basic",
        "max_results": max_results
    });

    let resp = client
        .post("https://api.tavily.com/search")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let results = json
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let title = item.get("title")?.as_str()?.to_string();
                    let url = item.get("url")?.as_str()?.to_string();
                    let snippet = item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(WebSearchResult { title, url, snippet })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}

/// Format search results as human-readable text
pub fn format_results(results: &[WebSearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }

    let mut output = String::new();
    for (i, item) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   {}\n   {}\n\n",
            i + 1,
            item.title,
            item.url,
            item.snippet
        ));
    }
    output
}
