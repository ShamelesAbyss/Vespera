use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use vespera_core::{EventEnvelope, TaskId};
use vespera_events::{Journal, JournalError};
use vespera_orchestrator::WorkflowStore;

const RETENTION: u32 = 4096;

pub(super) fn journal_path(dir: &Path) -> PathBuf {
    dir.join("desktop-events.sqlite3")
}

pub(super) fn workflow_path(dir: &Path) -> PathBuf {
    dir.join("desktop-workflows.sqlite3")
}

/// Rebuild a missing projection from the committed, payload-free workflow audit.
pub(super) fn restore(dir: &Path) -> Result<(), String> {
    let path = workflow_path(dir);
    if !path.try_exists().map_err(|error| error.to_string())? {
        return Ok(());
    }
    let events = WorkflowStore::inspect_audit_history(&path).map_err(|error| error.to_string())?;
    if events.is_empty() {
        return Ok(());
    }
    let mut journal = open(dir).map_err(|error| error.to_string())?;
    publish(&mut journal, &events).map_err(|error| error.to_string())
}

fn open(dir: &Path) -> Result<Journal, JournalError> {
    let path = journal_path(dir);
    match path.symlink_metadata() {
        Ok(info) if info.is_file() => Journal::open_writer(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Journal::initialize(&path, RETENTION)
        }
        _ => Err(JournalError::WrongDatabase),
    }
}

fn publish(journal: &mut Journal, events: &[EventEnvelope]) -> Result<(), JournalError> {
    for batch in events.chunks(vespera_events::MAX_PAGE as usize) {
        journal.append_batch(batch)?;
    }
    Ok(())
}

pub(super) struct Projection {
    writer: tokio::task::JoinHandle<Result<(), JournalError>>,
}

pub(super) fn replay(dir: &Path, store: &WorkflowStore) -> Result<(), String> {
    let events = store.audit_history().map_err(|error| error.to_string())?;
    let mut journal = open(dir).map_err(|error| error.to_string())?;
    publish(&mut journal, &events).map_err(|error| error.to_string())
}

impl Projection {
    pub fn start(dir: &Path, store: &WorkflowStore, task: TaskId) -> Result<Self, String> {
        let mut journal = open(dir).map_err(|error| error.to_string())?;
        let retained = store.audit_history().map_err(|error| error.to_string())?;
        publish(&mut journal, &retained).map_err(|error| error.to_string())?;
        let (sender, mut events) = mpsc::unbounded_channel();
        store.observe_events(Some((task, sender)));
        // A validated workflow emits at most 97 audit entries.
        let writer = tokio::task::spawn_blocking(move || {
            while let Some(event) = events.blocking_recv() {
                journal.append(&event)?;
            }
            Ok(())
        });
        Ok(Self { writer })
    }

    pub async fn finish(
        self,
        dir: &Path,
        store: &WorkflowStore,
        task: TaskId,
    ) -> Result<(), String> {
        store.observe_events(None);
        if matches!(self.writer.await, Ok(Ok(()))) {
            return Ok(());
        }
        let events = store
            .events(task)
            .map_err(|error| error.to_string())?
            .ok_or("No retained workflow audit was available.")?;
        let mut journal = open(dir).map_err(|error| error.to_string())?;
        publish(&mut journal, &events).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::EventKind;

    #[test]
    fn replay_keeps_the_same_evidence_identity_across_restarts() {
        let dir = std::env::temp_dir().join(format!("vespera-evidence-{}", TaskId::new()));
        std::fs::create_dir(&dir).unwrap();
        restore(&dir).unwrap();
        assert!(!journal_path(&dir).exists());
        let task_id = TaskId::new();
        let event =
            EventEnvelope::new("workflow-store", 1, EventKind::TaskCompleted { task_id }).unwrap();
        publish(&mut open(&dir).unwrap(), std::slice::from_ref(&event)).unwrap();
        publish(&mut open(&dir).unwrap(), std::slice::from_ref(&event)).unwrap();
        let path = journal_path(&dir);
        let page = Journal::open_reader(&path)
            .unwrap()
            .replay(None, 8)
            .unwrap();
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].event.event_id, event.event_id);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
