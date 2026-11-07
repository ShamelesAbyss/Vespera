use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hyper::StatusCode;
use ring::digest::{SHA256, digest};
use serde::Serialize;
use uuid::Uuid;
use vespera_core::AgentId;

use super::{HttpError, ServerError, UNAUTHORIZED};

const MAX_PENDING: usize = 32;
const MAX_ACKNOWLEDGED: usize = 64;
const MAX_CONTENT: usize = 2048;

#[derive(Serialize)]
pub(super) struct Message {
    message_id: Uuid,
    sender: AgentId,
    content: String,
}

struct Acknowledged {
    sender: AgentId,
    id: Uuid,
    hash: ring::digest::Digest,
}

struct Mailbox {
    targets: BTreeSet<AgentId>,
    pending: VecDeque<Message>,
    acknowledged: VecDeque<Acknowledged>,
}

#[derive(Default)]
pub(super) struct Mailboxes(BTreeMap<AgentId, Mailbox>);

impl Mailboxes {
    pub(super) fn approve(&mut self, worker: AgentId, targets: BTreeSet<AgentId>) {
        self.0.insert(
            worker,
            Mailbox {
                targets,
                pending: VecDeque::new(),
                acknowledged: VecDeque::new(),
            },
        );
    }

    pub(super) fn validate_routes(&self) -> Result<(), ServerError> {
        for (worker, mailbox) in &self.0 {
            if mailbox
                .targets
                .iter()
                .any(|target| target == worker || !self.0.contains_key(target))
            {
                return Err(ServerError::AccessFormat);
            }
        }
        Ok(())
    }

    pub(super) fn revoke(&mut self, worker: AgentId) {
        self.0.remove(&worker);
        for mailbox in self.0.values_mut() {
            mailbox.targets.remove(&worker);
            mailbox.pending.retain(|message| message.sender != worker);
            mailbox
                .acknowledged
                .retain(|message| message.sender != worker);
        }
    }

    pub(super) fn send(
        &mut self,
        sender: AgentId,
        recipient: AgentId,
        id: Uuid,
        content: String,
    ) -> Result<bool, HttpError> {
        if !self
            .0
            .get(&sender)
            .is_some_and(|mailbox| mailbox.targets.contains(&recipient))
        {
            return Err(UNAUTHORIZED);
        }
        if id.is_nil() || content.len() > MAX_CONTENT || content.trim().is_empty() {
            return Err(HttpError(StatusCode::BAD_REQUEST, "invalid_input"));
        }
        let mailbox = self.0.get_mut(&recipient).ok_or(UNAUTHORIZED)?;
        let conflict = HttpError(StatusCode::CONFLICT, "message_conflict");
        if let Some(previous) = mailbox
            .pending
            .iter()
            .find(|message| message.sender == sender && message.message_id == id)
        {
            return (previous.content == content)
                .then_some(false)
                .ok_or(conflict);
        }
        if let Some(previous) = mailbox
            .acknowledged
            .iter()
            .find(|message| message.sender == sender && message.id == id)
        {
            return (previous.hash.as_ref() == digest(&SHA256, content.as_bytes()).as_ref())
                .then_some(false)
                .ok_or(conflict);
        }
        if mailbox.pending.len() == MAX_PENDING {
            return Err(HttpError(StatusCode::SERVICE_UNAVAILABLE, "inbox_capacity"));
        }
        mailbox.pending.push_back(Message {
            message_id: id,
            sender,
            content,
        });
        Ok(true)
    }

    pub(super) fn inbox(&self, worker: AgentId) -> Result<&VecDeque<Message>, HttpError> {
        Ok(&self.0.get(&worker).ok_or(UNAUTHORIZED)?.pending)
    }

    pub(super) fn acknowledge(
        &mut self,
        worker: AgentId,
        sender: AgentId,
        id: Uuid,
    ) -> Result<(), HttpError> {
        let mailbox = self.0.get_mut(&worker).ok_or(UNAUTHORIZED)?;
        if let Some(index) = mailbox
            .pending
            .iter()
            .position(|message| message.sender == sender && message.message_id == id)
        {
            let message = mailbox
                .pending
                .remove(index)
                .expect("located pending message");
            if mailbox.acknowledged.len() == MAX_ACKNOWLEDGED {
                mailbox.acknowledged.pop_front();
            }
            mailbox.acknowledged.push_back(Acknowledged {
                sender,
                id,
                hash: digest(&SHA256, message.content.as_bytes()),
            });
        }
        Ok(())
    }
}
