//! Offline, explicit durable memory composition. No provider adapters or credentials.
use std::collections::BTreeSet;
use std::sync::Arc;
use vespera_config::AppConfig;
use vespera_core::{AgentId, Permission, TaskId};
use vespera_memory::{MemoryAuthorizer, StorageError};

struct ConfigAuthority {
    actor: AgentId,
    grants: BTreeSet<Permission>,
}
impl MemoryAuthorizer for ConfigAuthority {
    fn allowed(&self, actor: AgentId, permission: Permission) -> bool {
        actor == self.actor
            && matches!(permission, Permission::MemoryRead | Permission::MemoryWrite)
            && self.grants.contains(&permission)
    }
    fn owns_task(&self, _actor: AgentId, _task: TaskId) -> bool {
        false
    }
}

/// Validate declarations/grants before database I/O. A fresh CLI has no durable
/// accepted-task ledger and therefore cannot authorize generated-origin writes.
pub fn configured_authorizer(
    config: &AppConfig,
    actor: AgentId,
    required: &[Permission],
) -> Result<Arc<dyn MemoryAuthorizer>, StorageError> {
    config
        .validate()
        .map_err(|_| StorageError::Invalid("configuration"))?;
    let agent = config
        .agents
        .iter()
        .find(|agent| agent.identity.id == actor)
        .ok_or(StorageError::Denied)?;
    if !required.iter().all(|permission| {
        matches!(permission, Permission::MemoryRead | Permission::MemoryWrite)
            && agent.permissions.contains(permission)
    }) {
        return Err(StorageError::Denied);
    }
    Ok(Arc::new(ConfigAuthority {
        actor,
        grants: agent.permissions.clone(),
    }))
}
