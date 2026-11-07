use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use serde::Serialize;
use vespera_core::{EventEnvelope, EventKind, TaskId, TaskState};
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
    /// Follow committed events as newline-delimited JSON.
    Follow {
        /// Resume after this journal-UUID:sequence cursor.
        #[arg(long)]
        after: Option<Cursor>,
        /// Wait between reads when caught up.
        #[arg(long, default_value_t = 250, value_parser = clap::value_parser!(u64).range(10..=60_000))]
        poll_ms: u64,
        /// Stop successfully after printing this many events.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        max_events: Option<u64>,
        /// Stop successfully after this long without emitting an event.
        #[arg(long, value_parser = clap::value_parser!(u64).range(10..=3_600_000))]
        idle_timeout_ms: Option<u64>,
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
        EventCommand::Follow {
            after,
            poll_ms,
            max_events,
            idle_timeout_ms,
        } => {
            follow(args.journal, after, poll_ms, max_events, idle_timeout_ms)?;
            Ok(serde_json::Value::Null)
        }
    })
    .await
    .map_err(|_| JournalError::Database)??;
    if !output.is_null() {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn follow(
    path: PathBuf,
    mut cursor: Option<Cursor>,
    poll_ms: u64,
    max_events: Option<u64>,
    idle_timeout_ms: Option<u64>,
) -> Result<(), AppError> {
    let mut journal = Journal::open_reader(&path)?;
    let mut output = io::stdout().lock();
    let poll = Duration::from_millis(poll_ms);
    let idle = idle_timeout_ms.map(Duration::from_millis);
    let mut last_emission = Instant::now();
    let mut remaining = max_events.unwrap_or(u64::MAX);
    while remaining > 0 {
        let page = journal.replay(
            cursor,
            remaining.min(vespera_events::MAX_PAGE.into()) as u32,
        )?;
        if cursor.is_none() && page.discarded_through > 0 {
            eprintln!(
                "Journal history was discarded through sequence {}",
                page.discarded_through
            );
        }
        for entry in page.entries {
            let line = serde_json::to_string(&entry)?;
            let written = writeln!(output, "{line}").and_then(|()| output.flush());
            match written {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
                Err(error) => return Err(AppError::EventOutput(error)),
            }
            remaining -= 1;
            last_emission = Instant::now();
        }
        cursor = Some(page.next_cursor);
        if remaining == 0 {
            break;
        }
        if !page.has_more {
            let wait = idle.map_or(poll, |timeout| {
                poll.min(timeout.saturating_sub(last_emission.elapsed()))
            });
            if wait.is_zero() {
                break;
            }
            std::thread::sleep(wait);
        }
    }
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

    pub(crate) fn report_failure(&self) {
        if let Some(error) = &self.error {
            eprintln!("Event publication failed: {error}");
        }
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

pub(crate) fn terminal_event(task_id: TaskId, state: TaskState) -> Option<EventKind> {
    match state {
        TaskState::Completed => Some(EventKind::TaskCompleted { task_id }),
        TaskState::Failed => Some(EventKind::TaskFailed { task_id }),
        TaskState::Cancelled => Some(EventKind::TaskCancelled { task_id }),
        TaskState::Queued | TaskState::Running => None,
    }
}
