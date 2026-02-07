use crate::error::MicroClawError;

#[derive(Clone, Debug)]
pub struct Config {
    pub telegram_bot_token: String,
    pub bot_username: String,
    // LLM
    pub llm_provider: String,
    pub api_key: String,
    pub model: String,
    pub llm_base_url: Option<String>,
    pub max_tokens: u32,
    pub max_tool_iterations: usize,
    pub max_history_messages: usize,
    pub data_dir: String,
    pub openai_api_key: Option<String>,
    pub timezone: String,
    pub allowed_groups: Vec<i64>,
    pub max_session_messages: usize,
    pub compact_keep_recent: usize,
    pub whatsapp_access_token: Option<String>,
    pub whatsapp_phone_number_id: Option<String>,
    pub whatsapp_verify_token: Option<String>,
    pub whatsapp_webhook_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, MicroClawError> {
        dotenvy::dotenv().ok();

        let telegram_bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
            .map_err(|_| MicroClawError::Config("TELEGRAM_BOT_TOKEN not set".into()))?;
        let bot_username = std::env::var("BOT_USERNAME")
            .map_err(|_| MicroClawError::Config("BOT_USERNAME not set".into()))?;

        // LLM provider config with backward-compatible env var fallbacks
        let llm_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "anthropic".into());

        let api_key = std::env::var("LLM_API_KEY")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
            .map_err(|_| {
                MicroClawError::Config("LLM_API_KEY (or ANTHROPIC_API_KEY) not set".into())
            })?;

        let default_model = match llm_provider.as_str() {
            "openai" => "gpt-4o",
            _ => "claude-sonnet-4-20250514",
        };
        let model = std::env::var("LLM_MODEL")
            .or_else(|_| std::env::var("CLAUDE_MODEL"))
            .unwrap_or_else(|_| default_model.into());

        let llm_base_url = std::env::var("LLM_BASE_URL").ok().filter(|s| !s.is_empty());

        let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into());
        let max_tokens = std::env::var("MAX_TOKENS")
            .unwrap_or_else(|_| "8192".into())
            .parse::<u32>()
            .map_err(|e| MicroClawError::Config(format!("Invalid MAX_TOKENS: {e}")))?;
        let max_tool_iterations = std::env::var("MAX_TOOL_ITERATIONS")
            .unwrap_or_else(|_| "25".into())
            .parse::<usize>()
            .map_err(|e| MicroClawError::Config(format!("Invalid MAX_TOOL_ITERATIONS: {e}")))?;
        let max_history_messages = std::env::var("MAX_HISTORY_MESSAGES")
            .unwrap_or_else(|_| "50".into())
            .parse::<usize>()
            .map_err(|e| MicroClawError::Config(format!("Invalid MAX_HISTORY_MESSAGES: {e}")))?;

        let openai_api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());

        let timezone = std::env::var("TIMEZONE").unwrap_or_else(|_| "UTC".into());
        timezone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| MicroClawError::Config(format!("Invalid TIMEZONE: {timezone}")))?;

        let max_session_messages = std::env::var("MAX_SESSION_MESSAGES")
            .unwrap_or_else(|_| "40".into())
            .parse::<usize>()
            .map_err(|e| MicroClawError::Config(format!("Invalid MAX_SESSION_MESSAGES: {e}")))?;
        let compact_keep_recent = std::env::var("COMPACT_KEEP_RECENT")
            .unwrap_or_else(|_| "20".into())
            .parse::<usize>()
            .map_err(|e| MicroClawError::Config(format!("Invalid COMPACT_KEEP_RECENT: {e}")))?;

        let whatsapp_access_token = std::env::var("WHATSAPP_ACCESS_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let whatsapp_phone_number_id = std::env::var("WHATSAPP_PHONE_NUMBER_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let whatsapp_verify_token = std::env::var("WHATSAPP_VERIFY_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let whatsapp_webhook_port = std::env::var("WHATSAPP_WEBHOOK_PORT")
            .unwrap_or_else(|_| "8080".into())
            .parse::<u16>()
            .map_err(|e| MicroClawError::Config(format!("Invalid WHATSAPP_WEBHOOK_PORT: {e}")))?;

        let allowed_groups = std::env::var("ALLOWED_GROUPS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                s.trim().parse::<i64>().map_err(|e| {
                    MicroClawError::Config(format!("Invalid ALLOWED_GROUPS entry '{s}': {e}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Config {
            telegram_bot_token,
            bot_username,
            llm_provider,
            api_key,
            model,
            llm_base_url,
            max_tokens,
            max_tool_iterations,
            max_history_messages,
            data_dir,
            openai_api_key,
            timezone,
            allowed_groups,
            max_session_messages,
            compact_keep_recent,
            whatsapp_access_token,
            whatsapp_phone_number_id,
            whatsapp_verify_token,
            whatsapp_webhook_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn test_config() -> Config {
        Config {
            telegram_bot_token: "tok".into(),
            bot_username: "bot".into(),
            llm_provider: "anthropic".into(),
            api_key: "key".into(),
            model: "claude-sonnet-4-20250514".into(),
            llm_base_url: None,
            max_tokens: 8192,
            max_tool_iterations: 25,
            max_history_messages: 50,
            data_dir: "./data".into(),
            openai_api_key: None,
            timezone: "UTC".into(),
            allowed_groups: vec![],
            max_session_messages: 40,
            compact_keep_recent: 20,
            whatsapp_access_token: None,
            whatsapp_phone_number_id: None,
            whatsapp_verify_token: None,
            whatsapp_webhook_port: 8080,
        }
    }

    #[test]
    fn test_config_struct_clone_and_debug() {
        let config = test_config();
        let cloned = config.clone();
        assert_eq!(cloned.telegram_bot_token, "tok");
        assert_eq!(cloned.max_tokens, 8192);
        assert_eq!(cloned.max_tool_iterations, 25);
        assert_eq!(cloned.max_history_messages, 50);
        assert!(cloned.openai_api_key.is_none());
        assert_eq!(cloned.timezone, "UTC");
        assert!(cloned.allowed_groups.is_empty());
        assert_eq!(cloned.max_session_messages, 40);
        assert_eq!(cloned.compact_keep_recent, 20);
        let _ = format!("{:?}", config);
    }

    #[test]
    fn test_config_default_values() {
        let mut config = test_config();
        config.openai_api_key = Some("sk-test".into());
        config.timezone = "US/Eastern".into();
        config.allowed_groups = vec![123, 456];
        assert_eq!(config.model, "claude-sonnet-4-20250514");
        assert_eq!(config.data_dir, "./data");
        assert_eq!(config.openai_api_key.as_deref(), Some("sk-test"));
        assert_eq!(config.timezone, "US/Eastern");
        assert_eq!(config.allowed_groups, vec![123, 456]);
    }
}
