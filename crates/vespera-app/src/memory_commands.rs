//! Explicit offline memory commands. Blocking database work never occupies the async reactor.
use crate::AppError;
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use vespera_app::memory::configured_authorizer;
use vespera_config::AppConfig;
use vespera_core::{AgentId, MemoryId, MemoryLayer, MemoryRecord, Permission};
use vespera_memory::{MemoryOrigin, MemoryQuery, MemoryStore, StorageLimits};

#[derive(Debug, Args)]
pub(crate) struct MemoryArgs {
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    /// Explicit database path; only init can create it.
    #[arg(long, default_value = "data/vespera.sqlite3")]
    database: PathBuf,
    /// Configured owner/actor UUID; grants are checked before file/database access.
    #[arg(long)]
    agent: AgentId,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    action: MemoryCommand,
}
#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// Explicitly create/validate a database and parent directories; requires memory_write.
    Init,
    /// Insert nonempty UTF-8 content from a file (at most 64 KiB).
    Add {
        #[arg(long)]
        content_file: PathBuf,
        #[arg(long)]
        id: Option<MemoryId>,
        #[arg(long, value_enum, default_value = "observation")]
        layer: LayerArg,
        #[arg(long, value_enum, default_value = "observation")]
        origin: OriginArg,
        /// Supplied timestamp; defaults to this process's Unix-millisecond observation.
        #[arg(long)]
        created_at_unix_ms: Option<u64>,
    },
    /// Inspect one record and its lineage; missing records return nonzero.
    Get {
        #[arg(long)]
        id: MemoryId,
    },
    /// List a bounded page with structured filters.
    List(QueryArgs),
    /// Search a literal UTF-8 phrase from a file (at most 256 bytes).
    Search {
        #[arg(long)]
        query_file: PathBuf,
        #[command(flatten)]
        query: QueryArgs,
    },
    /// Inspect this namespace's authoritative record/content counts.
    Usage,
    /// Store a supplied derived summary with existing same-owner source links.
    Consolidate {
        #[arg(long)]
        content_file: PathBuf,
        #[arg(long)]
        id: Option<MemoryId>,
        #[arg(long, value_enum, default_value = "long_term")]
        layer: LayerArg,
        #[arg(long = "source", required = true)]
        sources: Vec<MemoryId>,
        #[arg(long)]
        method: String,
        #[arg(long)]
        created_at_unix_ms: Option<u64>,
    },
    /// Explicitly remove one unreferenced record and its search entry; requires read/write.
    Delete {
        #[arg(long)]
        id: MemoryId,
    },
}
#[derive(Debug, Args)]
struct QueryArgs {
    #[arg(long, value_enum)]
    layer: Option<LayerArg>,
    #[arg(long, default_value_t = 0)]
    after_sequence: u64,
    #[arg(long)]
    created_since_unix_ms: Option<u64>,
    #[arg(long)]
    created_before_unix_ms: Option<u64>,
    #[arg(long, default_value_t=100, value_parser=clap::value_parser!(u32).range(1..=100))]
    limit: u32,
    #[arg(long, default_value_t=1_048_576, value_parser=clap::value_parser!(u64).range(65536..=4194304))]
    max_content_bytes: u64,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum LayerArg {
    LongTerm,
    Episodic,
    Working,
    Relationship,
    Goal,
    Observation,
}
impl From<LayerArg> for MemoryLayer {
    fn from(value: LayerArg) -> Self {
        match value {
            LayerArg::LongTerm => Self::LongTerm,
            LayerArg::Episodic => Self::Episodic,
            LayerArg::Working => Self::Working,
            LayerArg::Relationship => Self::Relationship,
            LayerArg::Goal => Self::Goal,
            LayerArg::Observation => Self::Observation,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OriginArg {
    Observation,
    Import,
}
impl From<OriginArg> for MemoryOrigin {
    fn from(value: OriginArg) -> Self {
        match value {
            OriginArg::Observation => Self::Observation,
            OriginArg::Import => Self::Import,
        }
    }
}
impl QueryArgs {
    fn query(self) -> MemoryQuery {
        MemoryQuery {
            layer: self.layer.map(Into::into),
            after_sequence: self.after_sequence,
            created_since_unix_ms: self.created_since_unix_ms,
            created_before_unix_ms: self.created_before_unix_ms,
            limit: self.limit,
            max_content_bytes: self.max_content_bytes as usize,
        }
    }
}
struct Output {
    value: Value,
    text: String,
    missing: bool,
}
pub(crate) async fn run(args: MemoryArgs) -> Result<(), AppError> {
    let json = args.json;
    let output = tokio::task::spawn_blocking(move || execute(args))
        .await
        .map_err(|_| AppError::MemoryWorker)??;
    if json {
        println!("{}", serde_json::to_string_pretty(&output.value)?);
    } else {
        println!("{}", output.text);
    }
    if output.missing {
        return Err(AppError::MemoryNotFound);
    }
    Ok(())
}
pub(crate) fn read_text(
    path: &Path,
    limit: usize,
    field: &'static str,
) -> Result<String, AppError> {
    let mut text = String::new();
    File::open(path)
        .map_err(|_| AppError::MemoryInput(field))?
        .take((limit + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| AppError::MemoryInput(field))?;
    if text.len() > limit || text.trim().is_empty() {
        return Err(AppError::MemoryInput(field));
    }
    Ok(text)
}
pub(crate) fn current_timestamp() -> Result<u64, AppError> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AppError::MemoryInput("clock"))?
            .as_millis(),
    )
    .map_err(|_| AppError::MemoryInput("clock"))
}

fn record(
    actor: AgentId,
    id: Option<MemoryId>,
    layer: LayerArg,
    content_file: &Path,
    timestamp: Option<u64>,
) -> Result<MemoryRecord, AppError> {
    let content = read_text(content_file, 64 * 1024, "content_file")?;
    let timestamp = match timestamp {
        Some(value) => value,
        None => current_timestamp()?,
    };
    if timestamp > i64::MAX as u64 {
        return Err(AppError::MemoryInput("created_at_unix_ms"));
    }
    Ok(MemoryRecord {
        id: id.unwrap_or_default(),
        agent_id: actor,
        layer: layer.into(),
        content,
        created_at_unix_ms: timestamp,
        source_task_id: None,
    })
}
fn execute(args: MemoryArgs) -> Result<Output, AppError> {
    let config = AppConfig::load(&args.config)?;
    let required = match &args.action {
        MemoryCommand::Init | MemoryCommand::Add { .. } => vec![Permission::MemoryWrite],
        MemoryCommand::Consolidate { .. } | MemoryCommand::Delete { .. } => {
            vec![Permission::MemoryRead, Permission::MemoryWrite]
        }
        _ => vec![Permission::MemoryRead],
    };
    let authorizer = configured_authorizer(&config, args.agent, &required)?;
    let actor = args.agent;
    let value = match args.action {
        MemoryCommand::Init => {
            if let Some(parent) = args.database.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .map_err(|_| AppError::MemoryInput("database_parent"))?;
            }
            let _store = MemoryStore::open(&args.database, StorageLimits::default(), authorizer)?;
            json!({"agent_id":actor,"database_ready":true,"sqlite_version":vespera_memory::sqlite_version()})
        }
        MemoryCommand::Add {
            content_file,
            id,
            layer,
            origin,
            created_at_unix_ms,
        } => {
            let record = record(actor, id, layer, &content_file, created_at_unix_ms)?;
            let store =
                MemoryStore::open_existing(&args.database, StorageLimits::default(), authorizer)?;
            json!({"agent_id":actor,"record":store.insert(actor,record,origin.into())?})
        }
        MemoryCommand::Consolidate {
            content_file,
            id,
            layer,
            sources,
            method,
            created_at_unix_ms,
        } => {
            let record = record(actor, id, layer, &content_file, created_at_unix_ms)?;
            let store =
                MemoryStore::open_existing(&args.database, StorageLimits::default(), authorizer)?;
            json!({"agent_id":actor,"record":store.consolidate(actor,record,&sources,&method)?})
        }
        MemoryCommand::Delete { id } => {
            let store =
                MemoryStore::open_existing(&args.database, StorageLimits::default(), authorizer)?;
            store.delete(actor, actor, id)?;
            json!({"agent_id":actor,"memory_absent":id})
        }
        action => {
            // Inspect/search never create a missing database or migrate an empty file.
            let phrase = if let MemoryCommand::Search { query_file, .. } = &action {
                Some(read_text(query_file, 256, "query_file")?)
            } else {
                None
            };
            let store =
                MemoryStore::open_read_only(&args.database, StorageLimits::default(), authorizer)?;
            match action {
                MemoryCommand::Get { id } => {
                    json!({"agent_id":actor,"record":store.get(actor,actor,id)?})
                }
                MemoryCommand::List(query) => {
                    json!({"agent_id":actor,"page":store.list(actor,actor,&query.query())?})
                }
                MemoryCommand::Search { query, .. } => {
                    json!({"agent_id":actor,"page":store.search(actor,actor,phrase.as_deref().expect("search phrase prepared"),&query.query())?})
                }
                MemoryCommand::Usage => {
                    json!({"agent_id":actor,"usage":store.namespace_usage(actor,actor)?})
                }
                _ => unreachable!("writes handled above"),
            }
        }
    };
    let missing = value.get("record").is_some_and(Value::is_null);
    let text = if let Some(record) = value.get("record").filter(|v| !v.is_null()) {
        format!(
            "{}\nMemory {} in namespace {}",
            record["record"]["content"].as_str().unwrap_or_default(),
            record["record"]["id"].as_str().unwrap_or_default(),
            actor
        )
    } else if let Some(page) = value.get("page") {
        serde_json::to_string_pretty(page)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    Ok(Output {
        value,
        text,
        missing,
    })
}
