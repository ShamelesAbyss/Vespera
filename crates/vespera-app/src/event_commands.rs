use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use serde::Serialize;
use tokio::sync::mpsc;
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
    /// Publish a task's durable terminal envelope to an existing journal.
    RecoverAgentTask {
        #[arg(long)]
        task_store: PathBuf,
        #[arg(long)]
        task: TaskId,
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
        EventCommand::RecoverAgentTask { task_store, task } => {
            let event = vespera_agents::AgentTaskStore::inspect_terminal_event(&task_store, task)?
                .ok_or(JournalError::Invalid("durable terminal event unavailable"))?;
            let cursor = Journal::open_writer(&args.journal)?.append(&event)?;
            Ok(serde_json::json!({"cursor": cursor, "event_id": event.event_id}))
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

    fn from_result(result: Result<Cursor, AppError>) -> Self {
        match result {
            Ok(cursor) => Self {
                cursor: Some(cursor),
                error: None,
            },
            Err(error) => Self {
                cursor: None,
                error: Some(error.to_string()),
            },
        }
    }
}

pub(crate) async fn record_durable(
    mut journal: Journal,
    event: Result<Option<EventEnvelope>, vespera_agents::AgentTaskStoreError>,
) -> Option<Publication> {
    let event = match event {
        Ok(Some(event)) => event,
        Ok(None) => return None,
        Err(error) => return Some(Publication::from_result(Err(error.into()))),
    };
    let result = tokio::task::spawn_blocking(move || journal.append(&event))
        .await
        .unwrap_or(Err(JournalError::Database));
    Some(Publication::from_result(result.map_err(AppError::from)))
}

pub(crate) async fn record(
    journal: Journal,
    source: &'static str,
    task: TaskId,
    kind: EventKind,
) -> Publication {
    record_batch(journal, source, task, vec![kind]).await
}

pub(crate) async fn record_batch(
    mut journal: Journal,
    source: &'static str,
    task: TaskId,
    kinds: Vec<EventKind>,
) -> Publication {
    let result = tokio::task::spawn_blocking(move || {
        let timestamp = crate::memory_commands::current_timestamp()?;
        let events = kinds
            .into_iter()
            .map(|kind| {
                let mut event = EventEnvelope::new(source, timestamp, kind)
                    .map_err(|_| JournalError::Invalid("envelope"))?;
                event.correlation_task_id = Some(task);
                Ok(event)
            })
            .collect::<Result<Vec<_>, JournalError>>()?;
        Ok::<_, AppError>(journal.append_batch(&events)?)
    })
    .await
    .unwrap_or(Err(JournalError::Database.into()));
    Publication::from_result(result)
}

pub(crate) struct WorkflowJournal {
    pub(crate) sender: mpsc::UnboundedSender<EventKind>,
    writer: tokio::task::JoinHandle<Option<Publication>>,
}

impl WorkflowJournal {
    pub(crate) fn start(mut journal: Journal, root: TaskId) -> Self {
        // One validated workflow emits at most 96 attempt events and one root outcome.
        let (sender, mut events) = mpsc::unbounded_channel();
        let writer = tokio::task::spawn_blocking(move || {
            let mut cursor = None;
            while let Some(kind) = events.blocking_recv() {
                let result = (|| {
                    let timestamp = crate::memory_commands::current_timestamp()?;
                    let mut event = EventEnvelope::new("run-workflow", timestamp, kind)
                        .map_err(|_| JournalError::Invalid("envelope"))?;
                    event.correlation_task_id = Some(root);
                    Ok::<_, AppError>(journal.append(&event)?)
                })();
                match result {
                    Ok(committed) => cursor = Some(committed),
                    Err(error) => {
                        return Some(Publication {
                            cursor,
                            error: Some(error.to_string()),
                        });
                    }
                }
            }
            cursor.map(|cursor| Publication {
                cursor: Some(cursor),
                error: None,
            })
        });
        Self { sender, writer }
    }

    pub(crate) async fn finish(self, terminal: Option<EventKind>) -> Option<Publication> {
        if let Some(event) = terminal {
            let _ = self.sender.send(event);
        }
        drop(self.sender);
        self.writer.await.unwrap_or_else(|_| {
            Some(Publication {
                cursor: None,
                error: Some(JournalError::Database.to_string()),
            })
        })
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
