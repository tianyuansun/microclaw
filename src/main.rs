mod claude;
mod config;
mod db;
mod error;
mod llm;
mod mcp;
mod memory;
mod scheduler;
mod setup;
mod skills;
mod telegram;
mod tools;
mod transcribe;
mod whatsapp;

use config::Config;
use error::MicroClawError;
use tracing::info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!(
        r#"MicroClaw v{VERSION} — Agentic AI assistant for Telegram

USAGE:
    microclaw <COMMAND>

COMMANDS:
    start       Start the Telegram bot
    setup       Run interactive setup wizard
    help        Show this help message

FEATURES:
    - Agentic tool use (bash, files, search, memory)
    - Web search and page fetching
    - Image/photo understanding (Claude Vision)
    - Voice message transcription (OpenAI Whisper)
    - Scheduled/recurring tasks with timezone support
    - Task execution history/run logs
    - Chat export to markdown
    - Mid-conversation message sending
    - Group chat catch-up (reads all messages since last reply)
    - Group allowlist (restrict which groups can use the bot)
    - Continuous typing indicator
    - MCP (Model Context Protocol) server integration
    - WhatsApp Cloud API support

SETUP:
    1. Run: microclaw setup
       (or run microclaw start and follow auto-setup on first launch)
    2. Required values:

       TELEGRAM_BOT_TOKEN   Bot token from @BotFather
       LLM_API_KEY          API key (also accepts ANTHROPIC_API_KEY)
       BOT_USERNAME         Your bot's username (without @)

    3. Run: microclaw start

LLM PROVIDER ENV VARS:
    LLM_PROVIDER             Provider: "anthropic" or "openai" (default: anthropic)
    LLM_API_KEY              API key (falls back to ANTHROPIC_API_KEY)
    LLM_MODEL                Model name (falls back to CLAUDE_MODEL)
    LLM_BASE_URL             Custom base URL for the provider (optional)
                             Supports OpenRouter, DeepSeek, Groq, Ollama, etc.

OPTIONAL ENV VARS:
    DATA_DIR                 Data directory (default: ./data)
    MAX_TOKENS               Max tokens per response (default: 8192)
    MAX_TOOL_ITERATIONS      Max tool loop iterations (default: 25)
    MAX_HISTORY_MESSAGES     Chat history context size (default: 50)
    OPENAI_API_KEY           OpenAI API key for voice transcription (optional)
    TIMEZONE                 IANA timezone for scheduling (default: UTC)
    ALLOWED_GROUPS           Comma-separated chat IDs to allow (empty = all)
    RUST_LOG                 Log level, e.g. debug, info (default: info)

WHATSAPP (optional):
    WHATSAPP_ACCESS_TOKEN    Meta API access token
    WHATSAPP_PHONE_NUMBER_ID Phone number ID from Meta dashboard
    WHATSAPP_VERIFY_TOKEN    Webhook verification token (you choose)
    WHATSAPP_WEBHOOK_PORT    Webhook server port (default: 8080)

MCP (optional):
    Place a mcp.json file in DATA_DIR to connect MCP servers.
    See https://modelcontextprotocol.io for details.

EXAMPLES:
    microclaw start          Start the bot
    microclaw help           Show this message

ABOUT:
    https://microclaw.ai"#
    );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str());

    match command {
        Some("start") => {}
        Some("setup") => {
            let saved = setup::run_setup_wizard()?;
            if saved {
                println!("Setup saved to .env");
            } else {
                println!("Setup canceled");
            }
            return Ok(());
        }
        Some("help" | "--help" | "-h") | None => {
            print_help();
            return Ok(());
        }
        Some(unknown) => {
            eprintln!("Unknown command: {unknown}\n");
            print_help();
            std::process::exit(1);
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(MicroClawError::Config(e)) => {
            eprintln!("Config missing/invalid: {e}");
            eprintln!("Launching setup wizard...");
            let saved = setup::run_setup_wizard()?;
            if !saved {
                return Err(anyhow::anyhow!(
                    "setup canceled and config is still incomplete"
                ));
            }
            Config::from_env()?
        }
        Err(e) => return Err(e.into()),
    };
    info!("Starting MicroClaw bot...");

    let db = db::Database::new(&config.data_dir)?;
    info!("Database initialized");

    let memory_manager = memory::MemoryManager::new(&config.data_dir);
    info!("Memory manager initialized");

    let skill_manager = skills::SkillManager::new(&config.data_dir);
    let discovered = skill_manager.discover_skills();
    info!(
        "Skill manager initialized ({} skills discovered)",
        discovered.len()
    );

    // Initialize MCP servers (optional, configured via data_dir/mcp.json)
    let mcp_config_path = std::path::Path::new(&config.data_dir)
        .join("mcp.json")
        .to_string_lossy()
        .to_string();
    let mcp_manager = mcp::McpManager::from_config_file(&mcp_config_path).await;
    let mcp_tool_count: usize = mcp_manager.all_tools().len();
    if mcp_tool_count > 0 {
        info!("MCP initialized: {} tools available", mcp_tool_count);
    }

    telegram::run_bot(config, db, memory_manager, skill_manager, mcp_manager).await?;

    Ok(())
}
