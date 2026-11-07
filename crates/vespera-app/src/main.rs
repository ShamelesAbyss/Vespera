//! Native VESPERA CLI: offline inspection and explicit provider/agent operations.

mod commands;
mod doctor;
mod event_commands;
mod memory_commands;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use thiserror::Error;
use vespera_app::runtime::RuntimeError;
use vespera_config::{AppConfig, ConfigError};
use vespera_core::{AgentId, ResourceBudget};

#[derive(Debug, Parser)]
#[command(
    name = "vespera",
    version,
    about = "Local-first AI orchestration: inspect configuration and execute bounded agent inference"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List declared model metadata offline; registration does not imply residency.
    Models {
        #[arg(long, default_value = "config/local.toml")]
        config: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List configured agent identities and policies offline.
    Agents {
        #[arg(long, default_value = "config/local.toml")]
        config: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Check configured provider health through real HTTP requests without loading models.
    ProviderHealth {
        #[arg(long, default_value = "config/local.toml")]
        config: PathBuf,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_seconds: u64,
        #[arg(long)]
        json: bool,
    },
    /// Execute one bounded agent task using configured real provider adapters.
    RunAgent(RunAgentArgs),
    /// Execute a strict bounded workflow file through authorized agent delegation.
    RunWorkflow(RunWorkflowArgs),
    /// Explicit offline durable memory operations; never start model providers.
    Memory(memory_commands::MemoryArgs),
    /// Initialize, replay or follow a local event journal.
    Events(event_commands::EventArgs),
    /// Serve the external worker task broker over loopback HTTP or TLS.
    Serve {
        /// Strict JSON access policy with environment variable references.
        #[arg(long)]
        access_file: PathBuf,
        /// Optional SQLite broker database for task and revocation recovery.
        #[arg(long)]
        broker_db: Option<PathBuf>,
        /// PEM certificate chain, leaf first. Enables TLS with --tls-key.
        #[arg(long, requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        /// PEM private key for the TLS certificate.
        #[arg(long, requires = "tls_cert")]
        tls_key: Option<PathBuf>,
        /// Permit a TLS listener on a non-loopback interface.
        #[arg(long, requires_all = ["tls_cert", "tls_key"])]
        allow_remote: bool,
        /// Numeric IPv4 or IPv6 address and port.
        #[arg(long, default_value = "127.0.0.1:8765")]
        listen: SocketAddr,
    },
    /// Validate configuration and all references without connecting to providers.
    ValidateConfig {
        #[arg(long, default_value = "config/local.toml")]
        config: PathBuf,
        /// Emit machine-readable JSON without provider endpoints or credentials.
        #[arg(long)]
        json: bool,
    },
    /// Inspect host evidence without loading models or starting any service.
    Doctor {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
struct RunAgentArgs {
    /// Record the observed terminal outcome in an existing local journal.
    #[arg(long)]
    event_journal: Option<PathBuf>,
    /// Retrieve a literal phrase from a UTF-8 file and attach matching owned memories.
    #[arg(long)]
    memory_query_file: Option<PathBuf>,
    /// Save the accepted output as an episodic generated memory.
    #[arg(long)]
    remember_result: bool,
    /// Existing database for explicit retrieval or result retention.
    #[arg(long, default_value = "data/vespera.sqlite3")]
    memory_database: PathBuf,
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    /// Configured agent UUID.
    #[arg(long)]
    agent: AgentId,
    /// UTF-8 task text, bounded to 64 KiB; contents are not command-line arguments.
    #[arg(long)]
    prompt_file: PathBuf,
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(1..=131072))]
    max_output_tokens: u32,
    #[arg(long, value_parser = clap::value_parser!(u32).range(64..=1048576))]
    context_tokens: Option<u32>,
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout_seconds: u64,
    #[arg(long)]
    temperature: Option<f32>,
    /// Activate a suspended/failed agent in this invocation without changing its configuration file.
    #[arg(long)]
    resume: bool,
    /// Explicitly request unload after local work, including failure; providers may be shared.
    #[arg(long)]
    unload_after: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct RunWorkflowArgs {
    /// Record the observed root outcome in an existing local journal.
    #[arg(long)]
    event_journal: Option<PathBuf>,
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    /// Strict UTF-8 JSON workflow, at most one MiB.
    #[arg(long)]
    workflow_file: PathBuf,
    /// Overall command bound, including provider health and workflow execution.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout_seconds: u64,
    /// Activate only named workflow agents for this invocation; grants remain unchanged.
    #[arg(long)]
    resume: bool,
    #[arg(long)]
    unload_after: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
struct ConfigReport {
    valid: bool,
    schema_version: u32,
    providers_declared: usize,
    models_registered: usize,
    agents_declared: usize,
    allow_remote: bool,
    resource_budget: ResourceBudget,
    notice: &'static str,
}

#[derive(Debug, Error)]
enum AppError {
    #[error(transparent)]
    BrokerServer(#[from] vespera_api::http::ServerError),
    #[error("could not monitor shutdown signal: {0}")]
    ShutdownSignal(std::io::Error),
    #[error(transparent)]
    Journal(#[from] vespera_events::JournalError),
    #[error("cannot write event output: {0}")]
    EventOutput(std::io::Error),
    #[error("{0}")]
    Storage(#[from] vespera_memory::StorageError),
    #[error("invalid bounded memory input: {0}")]
    MemoryInput(&'static str),
    #[error("memory operation worker did not complete")]
    MemoryWorker,
    #[error("memory record is absent from the authorized namespace")]
    MemoryNotFound,
    #[error("could not read bounded UTF-8 workflow file")]
    WorkflowIo,
    #[error("workflow must be valid strict JSON and fit within one MiB")]
    InvalidWorkflow,
    #[error("{0}")]
    WorkflowPlan(#[from] vespera_orchestrator::PlanError),
    #[error("workflow did not complete successfully; inspect its report")]
    WorkflowFailed,
    #[error("{0}")]
    Runtime(#[from] RuntimeError),
    #[error("could not read UTF-8 prompt file: {0}")]
    PromptIo(std::io::Error),
    #[error("prompt must contain nonempty UTF-8 text and fit within 64 KiB")]
    InvalidPrompt,
    #[error("one or more configured providers are not freshly healthy")]
    UnhealthyProviders,
    #[error("inference completed but requested cleanup was not fully confirmed")]
    CleanupFailed,
    #[error("{0}")]
    Config(#[from] ConfigError),
    #[error("could not serialize diagnostic output: {0}")]
    Json(#[from] serde_json::Error),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Models { config, json } => {
            let config = AppConfig::load(config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&config.models)?);
            } else {
                for model in config.models {
                    println!(
                        "{}: {} ({:?}; declared, residency unverified)",
                        model.id, model.name, model.availability
                    );
                }
            }
        }
        Command::Agents { config, json } => {
            let config = AppConfig::load(config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&config.agents)?);
            } else {
                for agent in config.agents {
                    println!(
                        "{}: {} ({:?})",
                        agent.identity.id, agent.identity.name, agent.status
                    );
                }
            }
        }
        Command::ProviderHealth {
            config,
            timeout_seconds,
            json,
        } => {
            commands::provider_health(
                AppConfig::load(config)?,
                std::time::Duration::from_secs(timeout_seconds),
                json,
            )
            .await?
        }
        Command::RunAgent(args) => commands::run_agent(args).await?,
        Command::RunWorkflow(args) => commands::run_workflow(args).await?,
        Command::Memory(args) => memory_commands::run(args).await?,
        Command::Events(args) => event_commands::run(args).await?,
        Command::Serve {
            access_file,
            listen,
            broker_db,
            tls_cert,
            tls_key,
            allow_remote,
        } => {
            let tls =
                tls_cert
                    .as_deref()
                    .zip(tls_key.as_deref())
                    .map(|(certificate, private_key)| vespera_api::http::TlsOptions {
                        certificate,
                        private_key,
                        allow_remote,
                    });
            let server = vespera_api::http::Server::bind_with_transport(
                listen,
                access_file,
                broker_db.as_deref(),
                tls,
            )
            .await?;
            println!("{}://{}", server.scheme(), server.address());
            tokio::select! {
                result = server.serve() => result?,
                result = tokio::signal::ctrl_c() => result.map_err(AppError::ShutdownSignal)?,
            }
        }
        Command::ValidateConfig { config, json } => {
            let config = AppConfig::load(config)?;
            let report = ConfigReport {
                valid: true,
                schema_version: config.schema_version,
                providers_declared: config.providers.len(),
                models_registered: config.models.len(),
                agents_declared: config.agents.len(),
                allow_remote: config.network.allow_remote,
                resource_budget: config.resources,
                notice: "Configuration is internally valid. Provider availability, model fit, inference, and persistent memory have not been tested or started.",
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "VESPERA configuration valid (schema v{})",
                    report.schema_version
                );
                println!(
                    "Declared inventory: {} providers, {} models, {} agents",
                    report.providers_declared, report.models_registered, report.agents_declared
                );
                println!(
                    "Budget: {} RAM bytes, {} VRAM bytes; at most {} inference(s); context limit {} tokens",
                    report.resource_budget.ram_bytes,
                    report.resource_budget.vram_bytes,
                    report.resource_budget.max_concurrent_inferences,
                    report.resource_budget.max_context_tokens
                );
                println!("Remote networking allowed: {}", report.allow_remote);
                println!("{}", report.notice);
            }
        }
        Command::Doctor { json } => {
            let report = doctor::SystemReport::collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "VESPERA host diagnostics: {} / {}",
                    report.os, report.architecture
                );
                println!(
                    "CPU threads available to process: {:?}",
                    report.logical_cpu_threads_available
                );
                println!(
                    "Linux host RAM total bytes: {:?}; available bytes: {:?}",
                    report.linux_host_ram_total_bytes, report.linux_host_ram_available_bytes
                );
                println!(
                    "Root cgroup v2 memory: {:?}; CPU: {:?}",
                    report.cgroup_v2_memory, report.cgroup_v2_cpu
                );
                println!(
                    "Exposed GPU device paths: {:?}",
                    report.exposed_gpu_device_paths
                );
                println!("GPU telemetry: {}", report.gpu_telemetry_status);
                println!("{}", report.notice);
            }
        }
    }
    Ok(())
}
