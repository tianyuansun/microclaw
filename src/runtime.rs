use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use anyhow::anyhow;
use futures_util::FutureExt;
use tracing::{info, warn};

use crate::channels::dingtalk::{build_dingtalk_runtime_contexts, DingTalkRuntimeContext};
use crate::channels::email::{build_email_runtime_contexts, EmailRuntimeContext};
use crate::channels::feishu::{build_feishu_runtime_contexts, FeishuRuntimeContext};
use crate::channels::nostr::{build_nostr_runtime_contexts, NostrRuntimeContext};
use crate::channels::signal::{build_signal_runtime_contexts, SignalRuntimeContext};
use crate::channels::{
    DingTalkAdapter, EmailAdapter, FeishuAdapter, NostrAdapter, SignalAdapter,
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

fn prepare_channel_runtimes<T, Build, Register, ModelOverride>(
    config: &Config,
    channel_key: &str,
    registry: &mut ChannelRegistry,
    llm_model_overrides: &mut HashMap<String, String>,
    build: Build,
    register: Register,
    model_override: ModelOverride,
) -> Vec<T>
where
    Build: Fn(&Config) -> Vec<T>,
    Register: Fn(&T, &mut ChannelRegistry),
    ModelOverride: Fn(&T) -> Option<(String, String)>,
{
    if !config.channel_enabled(channel_key) {
        return Vec::new();
    }

    let runtimes = build(config);
    for runtime in &runtimes {
        if let Some((channel_name, model)) = model_override(runtime) {
            llm_model_overrides.insert(channel_name, model);
        }
        register(runtime, registry);
    }
    runtimes
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(msg) = payload.downcast_ref::<&str>() {
        return (*msg).to_string();
    }
    if let Some(msg) = payload.downcast_ref::<String>() {
        return msg.clone();
    }
    "unknown panic payload".to_string()
}

fn spawn_guarded<F>(task_name: String, future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        if let Err(payload) = AssertUnwindSafe(future).catch_unwind().await {
            warn!(
                "Task '{}' panicked; this channel task is skipped and other channels keep running. reason={}",
                task_name,
                panic_message(&*payload)
            );
        }
    });
}

fn spawn_channel_runtimes<T, StartFn, Fut>(state: Arc<AppState>, runtimes: Vec<T>, start: StartFn)
where
    T: Send + 'static,
    StartFn: Fn(Arc<AppState>, T) -> Fut + Copy + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    for runtime_ctx in runtimes {
        let channel_state = state.clone();
        let task_name = std::any::type_name::<T>().to_string();
        spawn_guarded(task_name, start(channel_state, runtime_ctx));
    }
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
    let mut llm_model_overrides: HashMap<String, String> = HashMap::new();

    let feishu_runtimes: Vec<FeishuRuntimeContext> = prepare_channel_runtimes(
        &config,
        "feishu",
        &mut registry,
        &mut llm_model_overrides,
        build_feishu_runtime_contexts,
        |runtime, reg| {
            reg.register(Arc::new(FeishuAdapter::new(
                runtime.channel_name.clone(),
                runtime.config.app_id.clone(),
                runtime.config.app_secret.clone(),
                runtime.config.domain.clone(),
            )));
        },
        |runtime| {
            runtime
                .model
                .clone()
                .map(|model| (runtime.channel_name.clone(), model))
        },
    );
    let email_runtimes: Vec<EmailRuntimeContext> = prepare_channel_runtimes(
        &config,
        "email",
        &mut registry,
        &mut llm_model_overrides,
        build_email_runtime_contexts,
        |runtime, reg| {
            reg.register(Arc::new(EmailAdapter::new(
                runtime.channel_name.clone(),
                runtime.from_address.clone(),
                runtime.sendmail_path.clone(),
            )));
        },
        |runtime| {
            runtime
                .model
                .clone()
                .map(|model| (runtime.channel_name.clone(), model))
        },
    );
    let nostr_runtimes: Vec<NostrRuntimeContext> = prepare_channel_runtimes(
        &config,
        "nostr",
        &mut registry,
        &mut llm_model_overrides,
        build_nostr_runtime_contexts,
        |runtime, reg| {
            reg.register(Arc::new(NostrAdapter::new(
                runtime.channel_name.clone(),
                runtime.publish_command.clone(),
            )));
        },
        |runtime| {
            runtime
                .model
                .clone()
                .map(|model| (runtime.channel_name.clone(), model))
        },
    );
    let signal_runtimes: Vec<SignalRuntimeContext> = prepare_channel_runtimes(
        &config,
        "signal",
        &mut registry,
        &mut llm_model_overrides,
        build_signal_runtime_contexts,
        |runtime, reg| {
            reg.register(Arc::new(SignalAdapter::new(
                runtime.channel_name.clone(),
                runtime.send_command.clone(),
            )));
        },
        |runtime| {
            runtime
                .model
                .clone()
                .map(|model| (runtime.channel_name.clone(), model))
        },
    );
    let dingtalk_runtimes: Vec<DingTalkRuntimeContext> = prepare_channel_runtimes(
        &config,
        "dingtalk",
        &mut registry,
        &mut llm_model_overrides,
        build_dingtalk_runtime_contexts,
        |runtime, reg| {
            reg.register(Arc::new(DingTalkAdapter::new(
                runtime.channel_name.clone(),
                runtime.robot_webhook_url.clone(),
            )));
        },
        |runtime| {
            runtime
                .model
                .clone()
                .map(|model| (runtime.channel_name.clone(), model))
        },
    );

    let mut has_web = false;

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

    let has_feishu = !feishu_runtimes.is_empty();
    if has_feishu {
        spawn_channel_runtimes(
            state.clone(),
            feishu_runtimes,
            |channel_state, runtime_ctx| async move {
                info!(
                    "Starting Feishu bot adapter '{}' as @{}",
                    runtime_ctx.channel_name, runtime_ctx.bot_username
                );
                crate::channels::feishu::start_feishu_bot(channel_state, runtime_ctx).await;
            },
        );
    }

    let has_email = !email_runtimes.is_empty();
    if has_email {
        spawn_channel_runtimes(
            state.clone(),
            email_runtimes,
            |channel_state, runtime_ctx| async move {
                info!(
                    "Starting Email adapter '{}' (from={})",
                    runtime_ctx.channel_name, runtime_ctx.from_address
                );
                crate::channels::email::start_email_bot(channel_state, runtime_ctx).await;
            },
        );
    }

    let has_nostr = !nostr_runtimes.is_empty();
    if has_nostr {
        spawn_channel_runtimes(
            state.clone(),
            nostr_runtimes,
            |channel_state, runtime_ctx| async move {
                info!("Starting Nostr adapter '{}'", runtime_ctx.channel_name);
                crate::channels::nostr::start_nostr_bot(channel_state, runtime_ctx).await;
            },
        );
    }

    let has_signal = !signal_runtimes.is_empty();
    if has_signal {
        spawn_channel_runtimes(
            state.clone(),
            signal_runtimes,
            |channel_state, runtime_ctx| async move {
                info!("Starting Signal adapter '{}'", runtime_ctx.channel_name);
                crate::channels::signal::start_signal_bot(channel_state, runtime_ctx).await;
            },
        );
    }

    let has_dingtalk = !dingtalk_runtimes.is_empty();
    if has_dingtalk {
        spawn_channel_runtimes(
            state.clone(),
            dingtalk_runtimes,
            |channel_state, runtime_ctx| async move {
                info!("Starting DingTalk adapter '{}'", runtime_ctx.channel_name);
                crate::channels::dingtalk::start_dingtalk_bot(channel_state, runtime_ctx).await;
            },
        );
    }

    if has_web {
        let web_state = state.clone();
        info!(
            "Starting Web UI server on {}:{}",
            state.config.web_host, state.config.web_port
        );
        spawn_guarded("web".to_string(), async move {
            crate::web::start_web_server(web_state).await;
        });
    }

    let has_active_channels = [
        has_web,
        has_feishu,
        has_email,
        has_nostr,
        has_signal,
        has_dingtalk,
    ]
    .into_iter()
    .any(|v| v);

    if has_active_channels {
        info!("Runtime active; waiting for Ctrl-C");
        tokio::signal::ctrl_c()
            .await
            .map_err(|e| anyhow!("Failed to listen for Ctrl-C: {e}"))?;
        Ok(())
    } else {
        Err(anyhow!(
            "No channel is enabled. Configure channels.<name>.enabled for Feishu, Email, Nostr, Signal, DingTalk, or web."
        ))
    }
}