use crate::clawhub::service::{ClawHubGateway, RegistryClawHubGateway};
use crate::config::Config;
use crate::llm_types::ToolDefinition;
use crate::tools::{schema_object, Tool, ToolResult};
use async_trait::async_trait;
use microclaw_clawhub::install::InstallOptions;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Runtime;

pub struct ClawHubSearchTool {
    gateway: Arc<dyn ClawHubGateway>,
}

pub struct ClawHubInstallTool {
    gateway: Arc<dyn ClawHubGateway>,
    skills_dir: PathBuf,
    lockfile_path: PathBuf,
    skip_security: bool,
}

impl ClawHubSearchTool {
    pub fn new(config: &Config) -> Self {
        let gateway: Arc<dyn ClawHubGateway> =
            Arc::new(RegistryClawHubGateway::from_config(config));
        Self { gateway }
    }
}

#[async_trait]
impl Tool for ClawHubSearchTool {
    fn name(&self) -> &str {
        "clawhub_search"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "clawhub_search".to_string(),
            description: "Search the ClawHub registry for skills matching a query.".to_string(),
            input_schema: schema_object(
                serde_json::json!({
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results (default 10, max 50)"
                    },
                    "sort": {
                        "type": "string",
                        "description": "Sort order: trending, installs, latest"
                    }
                }),
                &["query"],
            ),
        }
    }

    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => return ToolResult::error("Missing required parameter: query".into()),
        };
        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let sort = params
            .get("sort")
            .and_then(|v| v.as_str())
            .unwrap_or("trending");

        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return ToolResult::error(e.to_string()),
        };
        let gateway = self.gateway.clone();
        let results = rt.block_on(async move { gateway.search(query, limit.min(50), sort).await });

        match results {
            Ok(results) => {
                let output = results
                    .iter()
                    .map(|r| {
                        let vt_info = r
                            .virustotal
                            .as_ref()
                            .map(|v| {
                                format!(" | VirusTotal: {} ({} reports)", v.status, v.report_count)
                            })
                            .unwrap_or_default();
                        format!(
                            "• {} — {} ({} installs){}",
                            r.slug, r.description, r.install_count, vt_info
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolResult::success(output)
            }
            Err(e) => ToolResult::error(e.to_string()),
        }
    }
}

impl ClawHubInstallTool {
    pub fn new(config: &Config) -> Self {
        let gateway: Arc<dyn ClawHubGateway> =
            Arc::new(RegistryClawHubGateway::from_config(config));
        let skills_dir = PathBuf::from(config.skills_data_dir());
        let lockfile_path = config.clawhub_lockfile_path();
        Self {
            gateway,
            skills_dir,
            lockfile_path,
            skip_security: config.clawhub.skip_security_warnings,
        }
    }
}

#[async_trait]
impl Tool for ClawHubInstallTool {
    fn name(&self) -> &str {
        "clawhub_install"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "clawhub_install".to_string(),
            description:
                "Download and install a skill from ClawHub into ~/.microclaw/skills/ (or configured skills dir)."
                    .to_string(),
            input_schema: schema_object(
                serde_json::json!({
                    "slug": {
                        "type": "string",
                        "description": "The ClawHub skill slug to install"
                    },
                    "version": {
                        "type": "string",
                        "description": "Specific version to install (default: latest)"
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Overwrite if already installed"
                    }
                }),
                &["slug"],
            ),
        }
    }

    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let slug = match params.get("slug").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("Missing required parameter: slug".into()),
        };
        let version = params.get("version").and_then(|v| v.as_str());
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return ToolResult::error(e.to_string()),
        };
        let gateway = self.gateway.clone();
        let result = rt.block_on(async {
            let options = InstallOptions {
                force,
                skip_gates: false,
                skip_security: self.skip_security,
            };
            gateway
                .install(
                    slug,
                    version,
                    &self.skills_dir,
                    &self.lockfile_path,
                    &options,
                )
                .await
        });

        match result {
            Ok(install_result) => {
                if install_result.success {
                    let mut msg = install_result.message;
                    if install_result.requires_restart {
                        msg.push_str("\nRestart MicroClaw or run /reload-skills to activate.");
                    }
                    ToolResult::success(msg)
                } else {
                    ToolResult::error(install_result.message)
                }
            }
            Err(e) => ToolResult::error(e.to_string()),
        }
    }
}
