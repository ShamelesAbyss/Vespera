use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hyper::StatusCode;
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vespera_core::AgentId;

use super::{HttpError, ServerError, UNAUTHORIZED};

const MAX_PENDING: usize = 32;
const MAX_ACKNOWLEDGED: usize = 64;
const MAX_CONTENT: usize = 2048;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Message {
    message_id: Uuid,
    sender: AgentId,
    content: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Acknowledged {
    sender: AgentId,
    id: Uuid,
    hash: [u8; 32],
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inbox {
    pending: VecDeque<Message>,
    acknowledged: VecDeque<Acknowledged>,
}

struct Mailbox {
    targets: BTreeSet<AgentId>,
    inbox: Inbox,
}

#[derive(Default)]
pub(crate) struct Mailboxes {
    approved: BTreeMap<AgentId, Mailbox>,
    pub(crate) dirty: BTreeSet<AgentId>,
}

fn content_hash(content: &str) -> [u8; 32] {
    digest(&SHA256, content.as_bytes())
        .as_ref()
        .try_into()
        .expect("SHA-256 digest size")
}

impl Mailboxes {
    pub(super) fn approve(&mut self, worker: AgentId, targets: BTreeSet<AgentId>) {
        self.approved.insert(
            worker,
            Mailbox {
                targets,
                inbox: Inbox::default(),
            },
        );
    }

    pub(super) fn validate_routes(&self) -> Result<(), ServerError> {
        for (worker, mailbox) in &self.approved {
            if mailbox
                .targets
                .iter()
                .any(|target| target == worker || !self.approved.contains_key(target))
            {
                return Err(ServerError::AccessFormat);
            }
        }
        Ok(())
    }

    pub(crate) fn revoke(&mut self, worker: AgentId) {
        self.approved.remove(&worker);
        self.dirty.insert(worker);
        for (recipient, mailbox) in &mut self.approved {
            mailbox.targets.remove(&worker);
            let inbox = &mut mailbox.inbox;
            let before = inbox.pending.len() + inbox.acknowledged.len();
            inbox.pending.retain(|message| message.sender != worker);
            inbox
                .acknowledged
                .retain(|message| message.sender != worker);
            if inbox.pending.len() + inbox.acknowledged.len() != before {
                self.dirty.insert(*recipient);
            }
        }
    }

    /// Decode a bounded snapshot, then apply the current access policy.
    pub(crate) fn restore(&mut self, owner: AgentId, payload: &[u8]) -> Result<(), ()> {
        let mut inbox: Inbox = serde_json::from_slice(payload).map_err(|_| ())?;
        let mut ids = BTreeSet::new();
        if inbox.pending.len() > MAX_PENDING
            || inbox.acknowledged.len() > MAX_ACKNOWLEDGED
            || inbox.pending.iter().any(|message| {
                message.content.len() > MAX_CONTENT
                    || message.content.trim().is_empty()
                    || message.message_id.is_nil()
                    || message.sender == owner
                    || !ids.insert((message.sender, message.message_id))
            })
            || inbox.acknowledged.iter().any(|message| {
                message.id.is_nil()
                    || message.sender == owner
                    || !ids.insert((message.sender, message.id))
            })
        {
            return Err(());
        }
        let routed = |sender| {
            self.approved
                .get(&sender)
                .is_some_and(|mailbox| mailbox.targets.contains(&owner))
        };
        inbox.pending.retain(|message| routed(message.sender));
        inbox.acknowledged.retain(|message| routed(message.sender));
        if let Some(mailbox) = self.approved.get_mut(&owner) {
            mailbox.inbox = inbox;
        }
        Ok(())
    }

    pub(crate) fn owners(&self) -> impl Iterator<Item = AgentId> + '_ {
        self.approved.keys().copied()
    }

    pub(crate) fn snapshot(&self, owner: AgentId) -> Result<Option<Vec<u8>>, serde_json::Error> {
        self.approved
            .get(&owner)
            .map(|mailbox| &mailbox.inbox)
            .filter(|inbox| !inbox.pending.is_empty() || !inbox.acknowledged.is_empty())
            .map(serde_json::to_vec)
            .transpose()
    }

    pub(super) fn send(
        &mut self,
        sender: AgentId,
        recipient: AgentId,
        id: Uuid,
        content: String,
    ) -> Result<bool, HttpError> {
        if !self
            .approved
            .get(&sender)
            .is_some_and(|mailbox| mailbox.targets.contains(&recipient))
        {
            return Err(UNAUTHORIZED);
        }
        if id.is_nil() || content.len() > MAX_CONTENT || content.trim().is_empty() {
            return Err(HttpError(StatusCode::BAD_REQUEST, "invalid_input"));
        }
        let routed_senders = self
            .approved
            .values()
            .filter(|mailbox| mailbox.targets.contains(&recipient))
            .count();
        let sender_limit = (MAX_PENDING / routed_senders).max(1);
        let inbox = &mut self.approved.get_mut(&recipient).ok_or(UNAUTHORIZED)?.inbox;
        let conflict = HttpError(StatusCode::CONFLICT, "message_conflict");
        if let Some(previous) = inbox
            .pending
            .iter()
            .find(|message| message.sender == sender && message.message_id == id)
        {
            return (previous.content == content)
                .then_some(false)
                .ok_or(conflict);
        }
        if let Some(previous) = inbox
            .acknowledged
            .iter()
            .find(|message| message.sender == sender && message.id == id)
        {
            return (previous.hash == content_hash(&content))
                .then_some(false)
                .ok_or(conflict);
        }
        if inbox.pending.len() == MAX_PENDING
            || inbox
                .pending
                .iter()
                .filter(|message| message.sender == sender)
                .count()
                >= sender_limit
        {
            return Err(HttpError(StatusCode::SERVICE_UNAVAILABLE, "inbox_capacity"));
        }
        inbox.pending.push_back(Message {
            message_id: id,
            sender,
            content,
        });
        self.dirty.insert(recipient);
        Ok(true)
    }

    pub(super) fn inbox(&self, worker: AgentId) -> Result<&VecDeque<Message>, HttpError> {
        Ok(&self
            .approved
            .get(&worker)
            .ok_or(UNAUTHORIZED)?
            .inbox
            .pending)
    }

    pub(super) fn acknowledge(
        &mut self,
        worker: AgentId,
        sender: AgentId,
        id: Uuid,
    ) -> Result<bool, HttpError> {
        let inbox = &mut self.approved.get_mut(&worker).ok_or(UNAUTHORIZED)?.inbox;
        if let Some(index) = inbox
            .pending
            .iter()
            .position(|message| message.sender == sender && message.message_id == id)
        {
            let message = inbox
                .pending
                .remove(index)
                .expect("located pending message");
            if inbox.acknowledged.len() == MAX_ACKNOWLEDGED {
                inbox.acknowledged.pop_front();
            }
            inbox.acknowledged.push_back(Acknowledged {
                sender,
                id,
                hash: content_hash(&message.content),
            });
            self.dirty.insert(worker);
            return Ok(true);
        }
        Ok(false)
    }
}
