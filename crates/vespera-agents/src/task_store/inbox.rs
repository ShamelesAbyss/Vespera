//! Durable inbox storage behind the agent runtime's authorization boundary.
use rusqlite::{Connection, params};
use vespera_core::AgentId;

use super::{AgentTaskStore, AgentTaskStoreError, db};
use crate::{AgentMessage, RuntimeLimits};

const MAX_MESSAGES: i64 = 4096;
const MAX_PER_INBOX: usize = 256;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_INBOX_BYTES: usize = 64 * 1024 * 1024;

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
    ) -> Result<(), AgentTaskStoreError> {
        if message.content.len() > MAX_MESSAGE_BYTES {
            return Err(AgentTaskStoreError::Capacity);
        }
        let tick =
            i64::try_from(message.logical_tick).map_err(|_| AgentTaskStoreError::Capacity)?;
        let written = self
            .0
            .lock()
            .map_err(|_| AgentTaskStoreError::Database)?
            .execute(
                "INSERT INTO agent_messages (tick, sender, recipient, task_id, content)
             SELECT ?1, ?2, ?3, ?4, ?5
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
                ],
            )
            .map_err(db)?;
        if written == 0 {
            return Err(AgentTaskStoreError::Capacity);
        }
        Ok(())
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
