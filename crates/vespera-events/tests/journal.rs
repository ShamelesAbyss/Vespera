use std::path::PathBuf;
use vespera_core::{EventEnvelope, EventKind, TaskId};
use vespera_events::{Journal, JournalError};

struct File(PathBuf);
impl File {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vespera-journal-{}.sqlite3", uuid::Uuid::new_v4())))
    }
}
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn event() -> EventEnvelope {
    EventEnvelope::new(
        "fixture",
        100,
        EventKind::TaskCompleted {
            task_id: TaskId::new(),
        },
    )
    .unwrap()
}

#[test]
fn restart_replays_in_append_order_and_retention_reports_expired_cursors() {
    let file = File::new();
    let mut writer = Journal::initialize(&file.0, 2).unwrap();
    let zero = writer.replay(None, 1).unwrap().next_cursor;
    let first = writer.append(&event()).unwrap();
    let second = writer.append(&event()).unwrap();
    let third = writer.append(&event()).unwrap();
    drop(writer);
    let before = std::fs::read(&file.0).unwrap();
    let mut reader = Journal::open_reader(&file.0).unwrap();
    assert!(matches!(
        reader.replay(Some(zero), 1),
        Err(JournalError::Expired {
            discarded_through: 1
        })
    ));
    let page = reader.replay(None, 1).unwrap();
    assert_eq!(page.discarded_through, first.sequence);
    assert_eq!(page.entries[0].cursor, second);
    assert_eq!(page.next_cursor, second);
    assert!(page.has_more);
    let page = reader.replay(Some(second), 1).unwrap();
    assert_eq!(page.entries[0].cursor, third);
    assert!(!page.has_more);
    assert!(reader.replay(Some(third), 1).unwrap().entries.is_empty());
    assert_eq!(std::fs::read(&file.0).unwrap(), before);
}

#[test]
fn retained_duplicates_are_idempotent_and_invalid_writes_leave_history_intact() {
    let file = File::new();
    let mut journal = Journal::initialize(&file.0, 2).unwrap();
    let mut first = event();
    let cursor = journal.append(&first).unwrap();
    assert_eq!(journal.append(&first).unwrap(), cursor);
    first.occurred_at_unix_ms += 1;
    assert!(matches!(
        journal.append(&first),
        Err(JournalError::Conflict)
    ));
    let mut invalid = event();
    invalid.source = "private prompt text".to_owned();
    assert!(matches!(
        journal.append(&invalid),
        Err(JournalError::Invalid(_))
    ));
    invalid = event();
    invalid.schema_version = vespera_core::EVENT_SCHEMA_VERSION + 1;
    assert!(matches!(
        journal.append(&invalid),
        Err(JournalError::Invalid(_))
    ));
    let mut reader = Journal::open_reader(&file.0).unwrap();
    assert!(reader.append(&event()).is_err());
    let page = reader.replay(None, 2).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.next_cursor, cursor);
    assert!(matches!(
        reader.replay(
            Some(vespera_events::Cursor {
                sequence: 2,
                ..cursor
            }),
            1
        ),
        Err(JournalError::FutureCursor)
    ));
}

#[test]
fn journal_identity_fences_cursors_and_foreign_database_initialization() {
    let first = File::new();
    let second = File::new();
    let foreign = File::new();
    assert!(Journal::open_reader(&first.0).is_err());
    assert!(!first.0.exists());
    let cursor = Journal::initialize(&first.0, 1)
        .unwrap()
        .append(&event())
        .unwrap();
    let mut other = Journal::initialize(&second.0, 1).unwrap();
    assert!(matches!(
        other.replay(Some(cursor), 1),
        Err(JournalError::ForeignCursor)
    ));
    assert!(matches!(
        Journal::initialize(&first.0, 2),
        Err(JournalError::Invalid(_))
    ));
    let connection = rusqlite::Connection::open(&foreign.0).unwrap();
    connection
        .execute("CREATE TABLE unrelated(value TEXT)", [])
        .unwrap();
    drop(connection);
    let before = std::fs::read(&foreign.0).unwrap();
    assert!(matches!(
        Journal::initialize(&foreign.0, 1),
        Err(JournalError::WrongDatabase)
    ));
    assert_eq!(std::fs::read(&foreign.0).unwrap(), before);
}

#[test]
fn trimming_failure_rolls_back_the_append_and_cursor_floor() {
    let file = File::new();
    let mut journal = Journal::initialize(&file.0, 1).unwrap();
    let first = event();
    let cursor = journal.append(&first).unwrap();
    let connection = rusqlite::Connection::open(&file.0).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_trim BEFORE DELETE ON journal_events
         BEGIN SELECT RAISE(ABORT, 'fixture'); END;",
        )
        .unwrap();
    assert!(matches!(
        journal.append(&event()),
        Err(JournalError::Database)
    ));
    let page = journal.replay(None, 1).unwrap();
    assert_eq!(page.discarded_through, 0);
    assert_eq!(page.next_cursor, cursor);
    assert_eq!(page.entries[0].event, first);
}
