//! Durable inbox storage behind the agent runtime's authorization boundary.
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use vespera_core::{AgentId, EventEnvelope, EventId, EventKind};

use super::{AgentTaskStore, AgentTaskStoreError, db};
use crate::{AgentMessage, RuntimeLimits};

const MAX_MESSAGES: i64 = 4096;
const MAX_PER_INBOX: usize = 256;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_INBOX_BYTES: usize = 64 * 1024 * 1024;
const SOURCE: &str = "agent-inbox";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxAuditCursor {
    pub source_id: EventId,
    pub tick: i64,
}

impl std::str::FromStr for InboxAuditCursor {
    type Err = AgentTaskStoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, tick) = value
            .split_once(':')
            .ok_or(AgentTaskStoreError::InvalidInboxCursor)?;
        Ok(Self {
            source_id: id
                .parse()
                .map_err(|_| AgentTaskStoreError::InvalidInboxCursor)?,
            tick: tick
                .parse()
                .map_err(|_| AgentTaskStoreError::InvalidInboxCursor)?,
        })
    }
}

impl std::fmt::Display for InboxAuditCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.source_id, self.tick)
    }
}

pub struct InboxAuditPage {
    pub events: Vec<EventEnvelope>,
    pub next_cursor: Option<InboxAuditCursor>,
    pub has_more: bool,
}

pub(super) fn migrate(connection: &Connection) -> Result<(), AgentTaskStoreError> {
    connection
        .execute_batch(
            "CREATE TABLE agent_messages (
            tick INTEGER PRIMARY KEY CHECK (tick > 0),
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            task_id TEXT REFERENCES agent_tasks(id),
            content TEXT NOT NULL
        ) STRICT;
        CREATE INDEX agent_messages_inbox ON agent_messages(recipient, tick);",
        )
        .map_err(db)
}

pub(super) fn migrate_audit(connection: &Connection) -> Result<(), AgentTaskStoreError> {
    connection
        .execute_batch(
            "ALTER TABLE agent_messages ADD COLUMN event BLOB CHECK(length(event)<=4096);
         CREATE TABLE agent_inbox_source (
             singleton INTEGER PRIMARY KEY CHECK(singleton=1),
             id TEXT NOT NULL
         ) STRICT;",
        )
        .map_err(db)?;
    connection
        .execute(
            "INSERT INTO agent_inbox_source VALUES (1,?1)",
            [EventId::new().to_string()],
        )
        .map_err(db)?;
    Ok(())
}

impl AgentTaskStore {
    pub(crate) fn last_message_tick(&self) -> Result<u64, AgentTaskStoreError> {
        let tick: i64 = self
            .0
            .lock()
            .map_err(|_| AgentTaskStoreError::Database)?
            .query_row(
                "SELECT coalesce(max(tick), 0) FROM agent_messages",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        u64::try_from(tick).map_err(|_| AgentTaskStoreError::InvalidCheckpoint)
    }

    pub(crate) fn insert_message(
        &self,
        message: &AgentMessage,
        limits: RuntimeLimits,
    ) -> Result<EventEnvelope, AgentTaskStoreError> {
        if message.content.len() > MAX_MESSAGE_BYTES {
            return Err(AgentTaskStoreError::Capacity);
        }
        let tick =
            i64::try_from(message.logical_tick).map_err(|_| AgentTaskStoreError::Capacity)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
            .ok_or(AgentTaskStoreError::InvalidCheckpoint)?;
        let mut event = EventEnvelope::new(
            SOURCE,
            timestamp,
            EventKind::AgentMessageSent {
                from_agent_id: message.from_agent_id,
                to_agent_id: message.to_agent_id,
            },
        )
        .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        event.correlation_task_id = message.task_id;
        let payload =
            serde_json::to_vec(&event).map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        let written = self
            .0
            .lock()
            .map_err(|_| AgentTaskStoreError::Database)?
            .execute(
                "INSERT INTO agent_messages (tick, sender, recipient, task_id, content, event)
             SELECT ?1, ?2, ?3, ?4, ?5, ?10
             WHERE (SELECT count(*) FROM agent_messages WHERE recipient=?3) < ?6
             AND (SELECT count(*) < ?7
                 AND coalesce(sum(length(CAST(content AS BLOB))), 0) + ?8 <= ?9
                 FROM agent_messages)",
                params![
                    tick,
                    message.from_agent_id.to_string(),
                    message.to_agent_id.to_string(),
                    message.task_id.map(|id| id.to_string()),
                    message.content,
                    MAX_PER_INBOX.min(limits.max_messages_per_agent) as i64,
                    MAX_MESSAGES,
                    message.content.len() as i64,
                    MAX_INBOX_BYTES.min(limits.max_retained_content_bytes) as i64,
                    payload,
                ],
            )
            .map_err(db)?;
        if written == 0 {
            return Err(AgentTaskStoreError::Capacity);
        }
        Ok(event)
    }

    /// Read sanitized envelopes across inboxes as a trusted filesystem operator.
    pub fn inspect_inbox_audit(
        path: &Path,
        after: Option<InboxAuditCursor>,
        limit: u32,
    ) -> Result<InboxAuditPage, AgentTaskStoreError> {
        if !(1..=256).contains(&limit) || after.is_some_and(|cursor| cursor.tick < 0) {
            return Err(AgentTaskStoreError::InvalidInboxCursor);
        }
        let store = Self::connect(path, false, false)?;
        let connection = store.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(db)?;
        if version < 5 {
            if after.is_some() {
                return Err(AgentTaskStoreError::InvalidInboxCursor);
            }
            return Ok(InboxAuditPage {
                events: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        }
        let source: String = connection
            .query_row(
                "SELECT id FROM agent_inbox_source WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        let source_id = source
            .parse()
            .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        let latest: i64 = connection
            .query_row(
                "SELECT coalesce(max(tick),0) FROM agent_messages WHERE event IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        let mut tick = match after {
            Some(cursor) if cursor.source_id != source_id || cursor.tick > latest => {
                return Err(AgentTaskStoreError::InvalidInboxCursor);
            }
            Some(cursor) => cursor.tick,
            None => 0,
        };
        let mut query = connection
            .prepare(
                "SELECT tick,sender,recipient,task_id,CASE WHEN length(event)<=4096 THEN event END
             FROM agent_messages WHERE tick>?1 AND event IS NOT NULL ORDER BY tick LIMIT ?2",
            )
            .map_err(db)?;
        let mut rows = query.query(params![tick, limit]).map_err(db)?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(db)? {
            tick = row.get(0).map_err(db)?;
            let sender: String = row.get(1).map_err(db)?;
            let recipient: String = row.get(2).map_err(db)?;
            let task: Option<String> = row.get(3).map_err(db)?;
            let bytes: Vec<u8> = row.get(4).map_err(db)?;
            let event: EventEnvelope = serde_json::from_slice(&bytes)
                .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
            if event.source != SOURCE
                || event.correlation_task_id.map(|id| id.to_string()) != task
                || !matches!(event.kind, EventKind::AgentMessageSent { from_agent_id, to_agent_id }
                    if from_agent_id.to_string() == sender && to_agent_id.to_string() == recipient)
            {
                return Err(AgentTaskStoreError::InvalidCheckpoint);
            }
            events.push(event);
        }
        Ok(InboxAuditPage {
            events,
            next_cursor: Some(InboxAuditCursor { source_id, tick }),
            has_more: tick < latest,
        })
    }

    pub(crate) fn inbox(
        &self,
        recipient: AgentId,
    ) -> Result<Vec<AgentMessage>, AgentTaskStoreError> {
        let connection = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        let mut query = connection
            .prepare(
                "SELECT tick, sender, task_id,
                CASE WHEN length(CAST(content AS BLOB)) <= ?2 THEN content END
             FROM agent_messages WHERE recipient=?1 ORDER BY tick LIMIT ?3",
            )
            .map_err(db)?;
        let mut rows = query
            .query(params![
                recipient.to_string(),
                MAX_MESSAGE_BYTES as i64,
                MAX_PER_INBOX as i64 + 1
            ])
            .map_err(db)?;
        let mut messages = Vec::new();
        while let Some(row) = rows.next().map_err(db)? {
            let sender: String = row.get(1).map_err(db)?;
            let task: Option<String> = row.get(2).map_err(db)?;
            let content: Option<String> = row.get(3).map_err(db)?;
            let content = content
                .filter(|text| !text.trim().is_empty())
                .ok_or(AgentTaskStoreError::InvalidCheckpoint)?;
            if messages.len() == MAX_PER_INBOX {
                return Err(AgentTaskStoreError::InvalidCheckpoint);
            }
            messages.push(AgentMessage {
                from_agent_id: sender
                    .parse()
                    .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?,
                to_agent_id: recipient,
                task_id: task
                    .map(|id| id.parse())
                    .transpose()
                    .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?,
                content,
                logical_tick: u64::try_from(row.get::<_, i64>(0).map_err(db)?)
                    .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?,
            });
        }
        Ok(messages)
    }
}
