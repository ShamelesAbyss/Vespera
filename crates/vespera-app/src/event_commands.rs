use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::Serialize;
use vespera_core::{EventEnvelope, EventKind, TaskId};
use vespera_events::{Cursor, Journal, JournalError};

use crate::AppError;

#[derive(Debug, Args)]
pub(crate) struct EventArgs {
    /// Local journal file in an existing directory.
    #[arg(long, default_value = "data/events.sqlite3")]
    journal: PathBuf,
    #[command(subcommand)]
    command: EventCommand,
}

#[derive(Debug, Subcommand)]
enum EventCommand {
    /// Initialize the journal with a fixed retained event count.
    Init {
        #[arg(long, default_value_t = 10_000, value_parser = clap::value_parser!(u32).range(1..=100_000))]
        retention: u32,
    },
    /// Read events after a cursor and print one JSON page.
    Replay {
        /// Cursor in journal-UUID:sequence form. Omit for the oldest retained event.
        #[arg(long)]
        after: Option<Cursor>,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=256))]
        limit: u32,
    },
}

pub(crate) async fn run(args: EventArgs) -> Result<(), AppError> {
    let output = tokio::task::spawn_blocking(move || match args.command {
        EventCommand::Init { retention } => {
            Journal::initialize(&args.journal, retention)?;
            Ok::<_, AppError>(serde_json::json!({ "initialized": true, "retention": retention }))
        }
        EventCommand::Replay { after, limit } => {
            let page = Journal::open_reader(&args.journal)?.replay(after, limit)?;
            Ok(serde_json::to_value(page)?)
        }
    })
    .await
    .map_err(|_| JournalError::Database)??;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(crate) async fn prepare(path: Option<PathBuf>) -> Result<Option<Journal>, AppError> {
    match path {
        Some(path) => Ok(Some(
            tokio::task::spawn_blocking(move || Journal::open_writer(&path))
                .await
                .map_err(|_| JournalError::Database)??,
        )),
        None => Ok(None),
    }
}

#[derive(Serialize)]
pub(crate) struct Publication {
    cursor: Option<Cursor>,
    error: Option<String>,
}

impl Publication {
    pub(crate) fn failed(&self) -> bool {
        self.error.is_some()
    }
}

pub(crate) async fn record(
    mut journal: Journal,
    source: &'static str,
    task: TaskId,
    kind: EventKind,
) -> Publication {
    let result = tokio::task::spawn_blocking(move || {
        let timestamp = crate::memory_commands::current_timestamp()?;
        let mut event = EventEnvelope::new(source, timestamp, kind)
            .map_err(|_| JournalError::Invalid("envelope"))?;
        event.correlation_task_id = Some(task);
        Ok::<_, AppError>(journal.append(&event)?)
    })
    .await
    .unwrap_or(Err(JournalError::Database.into()));
    match result {
        Ok(cursor) => Publication {
            cursor: Some(cursor),
            error: None,
        },
        Err(error) => Publication {
            cursor: None,
            error: Some(error.to_string()),
        },
    }
}
