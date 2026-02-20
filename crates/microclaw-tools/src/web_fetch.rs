use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use microclaw_core::text::floor_char_boundary;

use crate::web_html::{extract_primary_html, html_to_text};

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

pub async fn fetch_url_with_timeout(url: &str, timeout_secs: u64) -> Result<String, String> {
    let client = http_client(timeout_secs.max(1));
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body = resp.text().await.map_err(|e| e.to_string())?;
    let primary = extract_primary_html(&body);
    let text = html_to_text(primary);

    const MAX_BYTES: usize = 20_000;
    if text.len() > MAX_BYTES {
        let truncated = &text[..floor_char_boundary(&text, MAX_BYTES)];
        Ok(format!("{truncated}\n\n[Truncated at 20KB]"))
    } else {
        Ok(text)
    }
}

pub async fn fetch_url(url: &str) -> Result<String, String> {
    fetch_url_with_timeout(url, 15).await
}
