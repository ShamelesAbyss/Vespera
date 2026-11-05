//! VESPERA Milestone 1 native application entry point.

mod doctor;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde::Serialize;
use thiserror::Error;
use vespera_config::{AppConfig, ConfigError};
use vespera_core::ResourceBudget;

#[derive(Debug, Parser)]
#[command(
    name = "vespera",
    version,
    about = "Local-first AI infrastructure: validated Rust foundation"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
    #[error("{0}")]
    Config(#[from] ConfigError),
    #[error("could not serialize diagnostic output: {0}")]
    Json(#[from] serde_json::Error),
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
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
