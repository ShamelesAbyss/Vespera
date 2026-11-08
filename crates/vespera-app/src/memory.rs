//! Offline, explicit durable memory composition. No provider adapters or credentials.
use std::collections::BTreeSet;
use std::sync::Arc;
use vespera_config::AppConfig;
use vespera_core::{AgentId, Permission, TaskId};
use vespera_memory::{MemoryAuthorizer, StorageError};

struct ConfigAuthority {
    actor: AgentId,
    grants: BTreeSet<Permission>,
    completed_task: Option<TaskId>,
}
impl MemoryAuthorizer for ConfigAuthority {
    fn allowed(&self, actor: AgentId, permission: Permission) -> bool {
        actor == self.actor
            && matches!(permission, Permission::MemoryRead | Permission::MemoryWrite)
            && self.grants.contains(&permission)
    }
    fn owns_task(&self, actor: AgentId, task: TaskId) -> bool {
        actor == self.actor && self.completed_task == Some(task)
    }
}

/// Validate declarations and grants before database I/O.
pub fn configured_authorizer(
    config: &AppConfig,
    actor: AgentId,
    required: &[Permission],
) -> Result<Arc<dyn MemoryAuthorizer>, StorageError> {
    Ok(Arc::new(config_authority(config, actor, required)?))
}

/// Reopen a completed result, authorizing only its original owner and source task.
pub fn authorize_remembered_task(
    config: &AppConfig,
    actor: AgentId,
    database: &std::path::Path,
    task: TaskId,
) -> Result<
    (
        Arc<dyn MemoryAuthorizer>,
        vespera_agents::TaskExecutionResult,
    ),
    StorageError,
> {
    let mut authority = config_authority(config, actor, &[Permission::MemoryWrite])?;
    let stored = vespera_agents::AgentTaskStore::inspect(database, task).map_err(|error| {
        use vespera_agents::AgentTaskStoreError;
        match error {
            AgentTaskStoreError::Busy => StorageError::Busy,
            AgentTaskStoreError::NotFound => StorageError::SourceUnavailable,
            _ => StorageError::Database,
        }
    })?;
    let result = stored
        .result
        .filter(|_| stored.owner == actor)
        .ok_or(StorageError::SourceUnavailable)?;
    authority.completed_task = Some(task);
    Ok((Arc::new(authority), result))
}

fn config_authority(
    config: &AppConfig,
    actor: AgentId,
    required: &[Permission],
) -> Result<ConfigAuthority, StorageError> {
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
    Ok(ConfigAuthority {
        actor,
        grants: agent.permissions.clone(),
        completed_task: None,
    })
}
