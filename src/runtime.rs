use std::collections::HashMap;
use std::sync::Arc;

use anyhow::anyhow;
use tracing::info;
#[cfg(feature = "sqlite-vec")]
use tracing::warn;

use crate::channels::dingtalk::{build_dingtalk_runtime_contexts, DingTalkRuntimeContext};
use crate::channels::discord::{build_discord_runtime_contexts, DiscordRuntimeContext};
use crate::channels::email::{build_email_runtime_contexts, EmailRuntimeContext};
use crate::channels::feishu::{build_feishu_runtime_contexts, FeishuRuntimeContext};
use crate::channels::imessage::{build_imessage_runtime_contexts, IMessageRuntimeContext};
use crate::channels::matrix::{build_matrix_runtime_contexts, MatrixRuntimeContext};
use crate::channels::nostr::{build_nostr_runtime_contexts, NostrRuntimeContext};
use crate::channels::qq::{build_qq_runtime_contexts, QQRuntimeContext};
use crate::channels::signal::{build_signal_runtime_contexts, SignalRuntimeContext};
use crate::channels::slack::{build_slack_runtime_contexts, SlackRuntimeContext};
use crate::channels::telegram::{
    build_telegram_runtime_contexts, TelegramChannelConfig, TelegramRuntimeContext,
};
use crate::channels::whatsapp::{build_whatsapp_runtime_contexts, WhatsAppRuntimeContext};
use crate::channels::{
    DingTalkAdapter, DiscordAdapter, EmailAdapter, FeishuAdapter, IMessageAdapter, IrcAdapter,
    MatrixAdapter, NostrAdapter, QQAdapter, SignalAdapter, SlackAdapter, TelegramAdapter,
    WhatsAppAdapter,
};
use crate::config::Config;
use crate::embedding::EmbeddingProvider;
use crate::hooks::HookManager;
use crate::llm::LlmProvider;
use crate::memory::MemoryManager;
use crate::memory_backend::MemoryBackend;
use crate::skills::SkillManager;
use crate::tools::ToolRegistry;
use crate::web::WebAdapter;
use microclaw_channels::channel_adapter::ChannelRegistry;
use microclaw_storage::db::Database;

pub struct AppState {
    pub config: Config,
    pub channel_registry: Arc<ChannelRegistry>,
    pub db: Arc<Database>,
    pub memory: MemoryManager,
    pub skills: SkillManager,
    pub hooks: Arc<HookManager>,
    pub llm: Box<dyn LlmProvider>,
    pub llm_model_overrides: HashMap<String, String>,
    pub embedding: Option<Arc<dyn EmbeddingProvider>>,
    pub memory_backend: Arc<MemoryBackend>,
    pub tools: ToolRegistry,
}

pub async fn run(
    config: Config,
    db: Database,
    memory: MemoryManager,
    skills: SkillManager,
    mcp_manager: crate::mcp::McpManager,
) -> anyhow::Result<()> {
    let db = Arc::new(db);
    let llm = crate::llm::create_provider(&config);
    let embedding = crate::embedding::create_provider(&config);
    #[cfg(feature = "sqlite-vec")]
    {
        let dim = embedding
            .as_ref()
            .map(|e| e.dimension())
            .or(config.embedding_dim)
            .unwrap_or(1536);
        if let Err(e) = db.prepare_vector_index(dim) {
            warn!("Failed to initialize sqlite-vec index: {e}");
        }
    }

    // Build channel registry from config
    let mut registry = ChannelRegistry::new();
    let mut telegram_runtimes: Vec<(teloxide::Bot, TelegramRuntimeContext)> = Vec::new();
    let mut discord_runtimes: Vec<(String, DiscordRuntimeContext)> = Vec::new();
    let mut slack_runtimes: Vec<SlackRuntimeContext> = Vec::new();
    let mut feishu_runtimes: Vec<FeishuRuntimeContext> = Vec::new();
    let mut matrix_runtimes: Vec<MatrixRuntimeContext> = Vec::new();
    let mut whatsapp_runtimes: Vec<WhatsAppRuntimeContext> = Vec::new();
    let mut imessage_runtimes: Vec<IMessageRuntimeContext> = Vec::new();
    let mut email_runtimes: Vec<EmailRuntimeContext> = Vec::new();
    let mut nostr_runtimes: Vec<NostrRuntimeContext> = Vec::new();
    let mut signal_runtimes: Vec<SignalRuntimeContext> = Vec::new();
    let mut dingtalk_runtimes: Vec<DingTalkRuntimeContext> = Vec::new();
    let mut qq_runtimes: Vec<QQRuntimeContext> = Vec::new();
    let mut has_irc = false;
    let mut has_web = false;
    let mut llm_model_overrides: HashMap<String, String> = HashMap::new();

    if config.channel_enabled("telegram") {
        if let Some(tg_cfg) = config.channel_config::<TelegramChannelConfig>("telegram") {
            for (token, runtime_ctx) in build_telegram_runtime_contexts(&config) {
                if let Some(model) = runtime_ctx.model.clone() {
                    llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
                }
                let bot = teloxide::Bot::new(&token);
                registry.register(Arc::new(TelegramAdapter::new(
                    runtime_ctx.channel_name.clone(),
                    bot.clone(),
                    tg_cfg.clone(),
                )));
                telegram_runtimes.push((bot, runtime_ctx));
            }
        }
    }

    if config.channel_enabled("discord") {
        discord_runtimes = build_discord_runtime_contexts(&config);
        for (token, runtime_ctx) in &discord_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(DiscordAdapter::new(
                runtime_ctx.channel_name.clone(),
                token.clone(),
            )));
        }
    }

    if config.channel_enabled("slack") {
        slack_runtimes = build_slack_runtime_contexts(&config);
        for runtime_ctx in &slack_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(SlackAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.bot_token.clone(),
            )));
        }
    }

    if config.channel_enabled("feishu") {
        feishu_runtimes = build_feishu_runtime_contexts(&config);
        for runtime_ctx in &feishu_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(FeishuAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.config.app_id.clone(),
                runtime_ctx.config.app_secret.clone(),
                runtime_ctx.config.domain.clone(),
            )));
        }
    }

    if config.channel_enabled("matrix") {
        matrix_runtimes = build_matrix_runtime_contexts(&config);
        for runtime_ctx in &matrix_runtimes {
            registry.register(Arc::new(MatrixAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.homeserver_url.clone(),
                runtime_ctx.access_token.clone(),
            )));
        }
    }

    if config.channel_enabled("whatsapp") {
        whatsapp_runtimes = build_whatsapp_runtime_contexts(&config);
        for runtime_ctx in &whatsapp_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(WhatsAppAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.access_token.clone(),
                runtime_ctx.phone_number_id.clone(),
                runtime_ctx.api_version.clone(),
            )));
        }
    }

    if config.channel_enabled("imessage") {
        imessage_runtimes = build_imessage_runtime_contexts(&config);
        for runtime_ctx in &imessage_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(IMessageAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.service.clone(),
            )));
        }
    }

    if config.channel_enabled("email") {
        email_runtimes = build_email_runtime_contexts(&config);
        for runtime_ctx in &email_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(EmailAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.from_address.clone(),
                runtime_ctx.sendmail_path.clone(),
            )));
        }
    }

    if config.channel_enabled("nostr") {
        nostr_runtimes = build_nostr_runtime_contexts(&config);
        for runtime_ctx in &nostr_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(NostrAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.publish_command.clone(),
            )));
        }
    }
    if config.channel_enabled("signal") {
        signal_runtimes = build_signal_runtime_contexts(&config);
        for runtime_ctx in &signal_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(SignalAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.send_command.clone(),
            )));
        }
    }
    if config.channel_enabled("dingtalk") {
        dingtalk_runtimes = build_dingtalk_runtime_contexts(&config);
        for runtime_ctx in &dingtalk_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(DingTalkAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.robot_webhook_url.clone(),
            )));
        }
    }
    if config.channel_enabled("qq") {
        qq_runtimes = build_qq_runtime_contexts(&config);
        for runtime_ctx in &qq_runtimes {
            if let Some(model) = runtime_ctx.model.clone() {
                llm_model_overrides.insert(runtime_ctx.channel_name.clone(), model);
            }
            registry.register(Arc::new(QQAdapter::new(
                runtime_ctx.channel_name.clone(),
                runtime_ctx.send_command.clone(),
            )));
        }
    }

    let mut irc_adapter: Option<Arc<IrcAdapter>> = None;
    if config.channel_enabled("irc") {
        if let Some(irc_cfg) =
            config.channel_config::<crate::channels::irc::IrcChannelConfig>("irc")
        {
            if !irc_cfg.server.trim().is_empty() && !irc_cfg.nick.trim().is_empty() {
                if let Some(model) = irc_cfg
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(ToOwned::to_owned)
                {
                    llm_model_overrides.insert("irc".to_string(), model);
                }
                has_irc = true;
                let adapter = Arc::new(IrcAdapter::new(380));
                registry.register(adapter.clone());
                irc_adapter = Some(adapter);
            }
        }
    }

    if config.channel_enabled("web") {
        has_web = true;
        registry.register(Arc::new(WebAdapter));
    }

    let channel_registry = Arc::new(registry);

    let memory_backend = Arc::new(MemoryBackend::new(
        db.clone(),
        crate::memory_backend::MemoryMcpClient::discover(&mcp_manager),
    ));
    let mut tools = ToolRegistry::new(
        &config,
        channel_registry.clone(),
        db.clone(),
        memory_backend.clone(),
    );

    for (server, tool_info) in mcp_manager.all_tools() {
        tools.add_tool(Box::new(crate::tools::mcp::McpTool::new(server, tool_info)));
    }

    let hooks = Arc::new(HookManager::from_config(&config).with_db(db.clone()));

    let state = Arc::new(AppState {
        config,
        channel_registry,
        db,
        memory,
        skills,
        hooks,
        llm,
        llm_model_overrides,
        embedding,
        memory_backend,
        tools,
    });

    crate::scheduler::spawn_scheduler(state.clone());
    crate::scheduler::spawn_reflector(state.clone());

    let has_discord = !discord_runtimes.is_empty();
    if has_discord {
        for (token, runtime_ctx) in discord_runtimes {
            let discord_state = state.clone();
            info!(
                "Starting Discord bot adapter '{}' as @{}",
                runtime_ctx.channel_name, runtime_ctx.bot_username
            );
            tokio::spawn(async move {
                crate::discord::start_discord_bot(discord_state, runtime_ctx, &token).await;
            });
        }
    }

    let has_slack = !slack_runtimes.is_empty();
    if has_slack {
        for runtime_ctx in slack_runtimes {
            let slack_state = state.clone();
            info!(
                "Starting Slack bot adapter '{}' as @{} (Socket Mode)",
                runtime_ctx.channel_name, runtime_ctx.bot_username
            );
            tokio::spawn(async move {
                crate::channels::slack::start_slack_bot(slack_state, runtime_ctx).await;
            });
        }
    }

    let has_feishu = !feishu_runtimes.is_empty();
    if has_feishu {
        for runtime_ctx in feishu_runtimes {
            let feishu_state = state.clone();
            info!(
                "Starting Feishu bot adapter '{}' as @{}",
                runtime_ctx.channel_name, runtime_ctx.bot_username
            );
            tokio::spawn(async move {
                crate::channels::feishu::start_feishu_bot(feishu_state, runtime_ctx).await;
            });
        }
    }

    let has_matrix = !matrix_runtimes.is_empty();
    if has_matrix {
        for runtime_ctx in matrix_runtimes {
            let matrix_state = state.clone();
            info!(
                "Starting Matrix bot adapter '{}' as {}",
                runtime_ctx.channel_name, runtime_ctx.bot_user_id
            );
            tokio::spawn(async move {
                crate::channels::matrix::start_matrix_bot(matrix_state, runtime_ctx).await;
            });
        }
    }

    let has_whatsapp = !whatsapp_runtimes.is_empty();
    if has_whatsapp {
        for runtime_ctx in whatsapp_runtimes {
            let whatsapp_state = state.clone();
            info!(
                "Starting WhatsApp adapter '{}' (webhook mode, phone_number_id={})",
                runtime_ctx.channel_name, runtime_ctx.phone_number_id
            );
            tokio::spawn(async move {
                crate::channels::whatsapp::start_whatsapp_bot(whatsapp_state, runtime_ctx).await;
            });
        }
    }

    let has_imessage = !imessage_runtimes.is_empty();
    if has_imessage {
        for runtime_ctx in imessage_runtimes {
            let imessage_state = state.clone();
            info!(
                "Starting iMessage adapter '{}' (service={})",
                runtime_ctx.channel_name, runtime_ctx.service
            );
            tokio::spawn(async move {
                crate::channels::imessage::start_imessage_bot(imessage_state, runtime_ctx).await;
            });
        }
    }

    let has_email = !email_runtimes.is_empty();
    if has_email {
        for runtime_ctx in email_runtimes {
            let email_state = state.clone();
            info!(
                "Starting Email adapter '{}' (from={})",
                runtime_ctx.channel_name, runtime_ctx.from_address
            );
            tokio::spawn(async move {
                crate::channels::email::start_email_bot(email_state, runtime_ctx).await;
            });
        }
    }

    let has_nostr = !nostr_runtimes.is_empty();
    if has_nostr {
        for runtime_ctx in nostr_runtimes {
            let nostr_state = state.clone();
            info!("Starting Nostr adapter '{}'", runtime_ctx.channel_name);
            tokio::spawn(async move {
                crate::channels::nostr::start_nostr_bot(nostr_state, runtime_ctx).await;
            });
        }
    }
    let has_signal = !signal_runtimes.is_empty();
    if has_signal {
        for runtime_ctx in signal_runtimes {
            let signal_state = state.clone();
            info!("Starting Signal adapter '{}'", runtime_ctx.channel_name);
            tokio::spawn(async move {
                crate::channels::signal::start_signal_bot(signal_state, runtime_ctx).await;
            });
        }
    }
    let has_dingtalk = !dingtalk_runtimes.is_empty();
    if has_dingtalk {
        for runtime_ctx in dingtalk_runtimes {
            let dingtalk_state = state.clone();
            info!("Starting DingTalk adapter '{}'", runtime_ctx.channel_name);
            tokio::spawn(async move {
                crate::channels::dingtalk::start_dingtalk_bot(dingtalk_state, runtime_ctx).await;
            });
        }
    }
    let has_qq = !qq_runtimes.is_empty();
    if has_qq {
        for runtime_ctx in qq_runtimes {
            let qq_state = state.clone();
            info!("Starting QQ adapter '{}'", runtime_ctx.channel_name);
            tokio::spawn(async move {
                crate::channels::qq::start_qq_bot(qq_state, runtime_ctx).await;
            });
        }
    }

    if has_web {
        let web_state = state.clone();
        info!(
            "Starting Web UI server on {}:{}",
            state.config.web_host, state.config.web_port
        );
        tokio::spawn(async move {
            crate::web::start_web_server(web_state).await;
        });
    }

    let has_telegram = !telegram_runtimes.is_empty();
    if has_telegram {
        for (bot, tg_ctx) in telegram_runtimes {
            let telegram_state = state.clone();
            info!(
                "Starting Telegram bot adapter '{}' as @{}",
                tg_ctx.channel_name, tg_ctx.bot_username
            );
            tokio::spawn(async move {
                let _ = crate::telegram::start_telegram_bot(telegram_state, bot, tg_ctx).await;
            });
        }
    }

    if has_irc {
        let irc_state = state.clone();
        let Some(irc_adapter) = irc_adapter else {
            return Err(anyhow!("IRC adapter state is missing"));
        };
        info!("Starting IRC bot");
        tokio::spawn(async move {
            crate::channels::irc::start_irc_bot(irc_state, irc_adapter).await;
        });
    }

    if has_telegram
        || has_web
        || has_discord
        || has_slack
        || has_feishu
        || has_matrix
        || has_irc
        || has_whatsapp
        || has_imessage
        || has_email
        || has_nostr
        || has_signal
        || has_dingtalk
        || has_qq
    {
        info!("Runtime active; waiting for Ctrl-C");
        tokio::signal::ctrl_c()
            .await
            .map_err(|e| anyhow!("Failed to listen for Ctrl-C: {e}"))?;
        Ok(())
    } else {
        Err(anyhow!(
            "No channel is enabled. Configure channels.<name>.enabled (or legacy channel settings) for Telegram, Discord, Slack, Feishu, Matrix, WhatsApp, iMessage, Email, Nostr, Signal, DingTalk, QQ, IRC, or web."
        ))
    }
}
