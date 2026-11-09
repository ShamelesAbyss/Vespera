use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use vespera_agents::{AgentTaskStore, ExecutionOptions, MemoryContext};
use vespera_app::memory::{authorize_remembered_task, configured_authorizer};
use vespera_app::runtime::{VesperaRuntime, configured_registry};
use vespera_config::{AppConfig, CONFIG_SCHEMA_VERSION, ProviderConfig};
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, Capability, MemoryLayer, MemoryRecord,
    ModelAvailability, ModelDescriptor, ModelId, ModelSelectionPolicy, Permission, ProviderId,
    ProviderKind, ResourceBudget, ResourceEstimate, Task, TaskId, TaskState,
};
use vespera_memory::{MemoryOrigin, MemoryStore, StorageError, StorageLimits};
use vespera_providers::{CancellationToken, DiscoveredModel};

const APPLICATION_ID: i32 = 0x5644_534b;
const MAX_CONFIG: usize = 65_536;
const MIB: u64 = 1024 * 1024;
const MAX_AGENTS: usize = 16;

#[derive(Clone)]
pub(super) struct Roster {
    pub revision: i64,
    config: AppConfig,
}

impl Roster {
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn len(&self) -> usize {
        self.config.agents.len()
    }

    pub fn selection(
        &self,
        index: usize,
    ) -> Option<(&AgentDefinition, &ModelDescriptor, &ProviderConfig)> {
        let agent = self.config.agents.get(index)?;
        let ModelSelectionPolicy::Fixed { model_id } = agent.model_policy else {
            return None;
        };
        let model = self
            .config
            .models
            .iter()
            .find(|model| model.id == model_id)?;
        let provider = self
            .config
            .providers
            .iter()
            .find(|provider| provider.id == model.provider_id)?;
        Some((agent, model, provider))
    }
}

pub(super) struct Registration {
    pub provider: ProviderConfig,
    pub model: DiscoveredModel,
    pub agent_name: String,
    pub role: String,
    pub rebind: Option<AgentId>,
    pub ram_mib: u32,
    pub vram_mib: u32,
}

pub(super) struct FinishedTask {
    pub id: TaskId,
    pub response: String,
    pub memory_records: Option<usize>,
    pub cleanup_confirmed: bool,
}

type Failure = &'static str;

const STORAGE: Failure = "Could not access the desktop profile database.";
const FORMAT: Failure = "The desktop profile database has an unrecognized format.";

pub(super) fn load(dir: &Path) -> Result<Option<Roster>, Failure> {
    let path = dir.join("desktop.sqlite3");
    match path.symlink_metadata() {
        Ok(info) if info.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(FORMAT),
    }
    let db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| STORAGE)?;
    db.busy_timeout(Duration::ZERO).map_err(|_| STORAGE)?;
    if identity(&db)? != (APPLICATION_ID, 1) {
        return Err(FORMAT);
    }
    read(&db)
}

fn identity(db: &Connection) -> Result<(i32, u32), Failure> {
    Ok((
        db.pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|_| FORMAT)?,
        db.pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| FORMAT)?,
    ))
}

fn read(db: &Connection) -> Result<Option<Roster>, Failure> {
    let record: Option<(i64, Vec<u8>)> = db
        .query_row(
            "SELECT revision, payload FROM desktop_profile WHERE singleton = 1 AND length(payload) <= ?",
            [MAX_CONFIG as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| FORMAT)?;
    let (revision, payload) = record.ok_or(FORMAT)?;
    let config: AppConfig = serde_json::from_slice(&payload).map_err(|_| FORMAT)?;
    if revision < 0
        || !(1..=MAX_AGENTS).contains(&config.agents.len())
        || config.providers.len() > MAX_AGENTS
        || config.models.len() > MAX_AGENTS
        || config.network.allow_remote
        || config
            .providers
            .iter()
            .filter_map(|provider| provider.credential_env.as_deref())
            .any(|name| !credential_name(name))
        || config
            .agents
            .iter()
            .any(|agent| !matches!(agent.model_policy, ModelSelectionPolicy::Fixed { .. }))
    {
        return Err(FORMAT);
    }
    config.validate().map_err(|_| FORMAT)?;
    Ok(Some(Roster { revision, config }))
}

pub(super) fn provider(kind: ProviderKind, endpoint: String) -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::new(),
        name: "Local provider".into(),
        kind,
        endpoint,
        credential_env: None,
        credential_windows: false,
        request_timeout_seconds: 45,
    }
}

pub(super) fn config(provider: ProviderConfig) -> AppConfig {
    AppConfig {
        schema_version: CONFIG_SCHEMA_VERSION,
        network: Default::default(),
        resources: Default::default(),
        inference_overhead: Default::default(),
        scheduling: Default::default(),
        providers: vec![provider],
        models: vec![],
        agents: vec![],
    }
}

pub(super) fn credential_name(name: &str) -> bool {
    name.len() <= 128
        && name.strip_prefix("VESPERA_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

pub(super) async fn register(
    dir: &Path,
    previous: Option<&Roster>,
    registration: Registration,
) -> Result<(Roster, usize), Failure> {
    let Registration {
        mut provider,
        model,
        agent_name,
        role,
        rebind,
        ram_mib,
        vram_mib,
    } = registration;
    if ram_mib == 0 {
        return Err("Enter a positive model RAM estimate.");
    }
    if provider
        .credential_env
        .as_deref()
        .is_some_and(|name| !credential_name(name))
    {
        return Err("Invalid Vespera credential environment variable name.");
    }
    let native = provider.kind == ProviderKind::Ollama;
    let mut inventory = previous.map_or_else(|| config(provider.clone()), |old| old.config.clone());
    if let Some(existing) = inventory.providers.iter().find(|existing| {
        existing.kind == provider.kind
            && existing.endpoint == provider.endpoint
            && existing.credential_env == provider.credential_env
            && existing.credential_windows == provider.credential_windows
    }) {
        provider.id = existing.id;
    } else {
        inventory.providers.push(provider.clone());
    }
    let id = provider.id;
    let index = match rebind {
        Some(agent) => inventory
            .agents
            .iter()
            .position(|entry| entry.identity.id == agent)
            .ok_or("The selected agent no longer exists.")?,
        None if inventory.agents.len() < MAX_AGENTS => inventory.agents.len(),
        None => return Err("This desktop profile already has 16 agents."),
    };
    let text = Capability::new("text-generation").expect("static capability is valid");
    let mut context = None;
    let mut quantization = model.quantization_level;
    if native {
        let mut probe = provider;
        probe.request_timeout_seconds = 5;
        let registry = configured_registry(&config(probe))
            .await
            .map_err(|_| "Invalid local provider configuration.")?;
        let adapter = registry
            .get_provider(id)
            .await
            .map_err(|_| "Provider registration failed.")?;
        let metadata = adapter
            .model_metadata(&model.provider_model_name, CancellationToken::new())
            .await
            .map_err(|_| "Could not read model metadata. Verify the model is still installed.")?;
        if !metadata.capabilities.contains(&text) {
            return Err("The model did not report text generation support.");
        }
        context = metadata.context_window_tokens;
        quantization = metadata.quantization_level.or(quantization);
    }
    let existing_model = inventory
        .models
        .iter()
        .position(|entry| entry.identifier == model.provider_model_name && entry.provider_id == id);
    let model_id = existing_model.map_or_else(ModelId::new, |index| inventory.models[index].id);
    let ram_bytes = u64::from(ram_mib) * MIB;
    let vram_bytes = u64::from(vram_mib) * MIB;
    let descriptor = ModelDescriptor {
        id: model_id,
        provider_id: id,
        name: model.provider_model_name.clone(),
        identifier: model.provider_model_name,
        capabilities: [text].into(),
        context_window_tokens: context,
        parameter_count: None,
        quantization,
        resources: ResourceEstimate {
            ram_bytes: Some(ram_bytes),
            vram_bytes: Some(vram_bytes),
        },
        availability: ModelAvailability::Available,
        responsibilities: Default::default(),
    };
    if let Some(existing) = existing_model {
        let shared = inventory
            .agents
            .iter()
            .enumerate()
            .any(|(agent_index, agent)| {
                agent_index != index
                    && agent.model_policy == ModelSelectionPolicy::Fixed { model_id }
            });
        if shared && inventory.models[existing].resources != descriptor.resources {
            return Err(
                "Another agent uses this model. Enter the model's saved RAM and VRAM estimates.",
            );
        }
        inventory.models[existing] = descriptor;
    } else {
        inventory.models.push(descriptor);
    }
    let definition = AgentDefinition {
        identity: AgentIdentity {
            id: rebind.unwrap_or_else(AgentId::new),
            name: agent_name,
            description: String::new(),
            role,
            personality: String::new(),
            goals: vec![],
        },
        model_policy: ModelSelectionPolicy::Fixed { model_id },
        permissions: Default::default(),
        status: AgentStatus::Idle,
    };
    if let Some(agent) = inventory.agents.get_mut(index) {
        agent.identity.name = definition.identity.name;
        agent.identity.role = definition.identity.role;
        agent.model_policy = definition.model_policy;
    } else {
        inventory.agents.push(definition);
    }
    inventory.models.retain(|model| {
        inventory
            .agents
            .iter()
            .any(|agent| agent.model_policy == ModelSelectionPolicy::Fixed { model_id: model.id })
    });
    inventory.providers.retain(|provider| {
        inventory
            .models
            .iter()
            .any(|model| model.provider_id == provider.id)
    });
    inventory
        .validate()
        .map_err(|_| "Invalid agent name, model, or resource declaration.")?;
    Ok((
        save(dir, previous.map(|roster| roster.revision), inventory)?,
        index,
    ))
}

fn save(dir: &Path, expected: Option<i64>, config: AppConfig) -> Result<Roster, Failure> {
    let payload = serde_json::to_vec(&config).map_err(|_| FORMAT)?;
    if payload.len() > MAX_CONFIG {
        return Err(FORMAT);
    }
    fs::create_dir_all(dir).map_err(|_| STORAGE)?;
    let path = dir.join("desktop.sqlite3");
    let created = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(_) => return Err(STORAGE),
    };
    let mut db = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| STORAGE)?;
    db.busy_timeout(Duration::ZERO).map_err(|_| STORAGE)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| STORAGE)?;
    let actual = match identity(&tx)? {
        (APPLICATION_ID, 1) => read(&tx)?.map(|binding| binding.revision),
        (0, 0) if created => {
            let objects: i64 = tx
                .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
                .map_err(|_| FORMAT)?;
            if objects != 0 {
                return Err(FORMAT);
            }
            tx.execute_batch(
                "CREATE TABLE desktop_profile (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                revision INTEGER NOT NULL,
                payload BLOB NOT NULL CHECK(length(payload) <= 65536)
            );",
            )
            .map_err(|_| STORAGE)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(|_| STORAGE)?;
            tx.pragma_update(None, "user_version", 1)
                .map_err(|_| STORAGE)?;
            None
        }
        _ => return Err(FORMAT),
    };
    if actual != expected {
        return Err("The saved binding changed in another window. Reopen the application.");
    }
    let revision = actual
        .map_or(Some(0), |revision| revision.checked_add(1))
        .ok_or(FORMAT)?;
    tx.execute(
        "INSERT INTO desktop_profile VALUES (1, ?1, ?2)
         ON CONFLICT(singleton) DO UPDATE SET revision = excluded.revision, payload = excluded.payload",
        (revision, payload),
    ).map_err(|_| STORAGE)?;
    tx.commit().map_err(|_| STORAGE)?;
    Ok(Roster { revision, config })
}

pub(super) fn set_permission(
    dir: &Path,
    mut roster: Roster,
    selected: usize,
    permission: Permission,
    allow: bool,
) -> Result<(Roster, usize), Failure> {
    let agent = roster
        .config
        .agents
        .get_mut(selected)
        .ok_or("No saved agent selected.")?;
    if allow {
        agent.permissions.insert(permission);
    } else {
        agent.permissions.remove(&permission);
    }
    Ok((save(dir, Some(roster.revision), roster.config)?, selected))
}

const MEMORY_DB: &str = "desktop-memory.sqlite3";

pub(super) fn remember(
    dir: &Path,
    revision: i64,
    owner: AgentId,
    task: TaskId,
) -> Result<(), String> {
    let roster = current_roster(dir, revision)?;
    let (authority, result) = authorize_remembered_task(
        roster.config(),
        owner,
        &dir.join("desktop-tasks.sqlite3"),
        task,
    )
    .map_err(|error| error.to_string())?;
    let created_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .ok_or("Could not determine the memory timestamp.")?;
    let record = MemoryRecord {
        id: task
            .to_string()
            .parse()
            .map_err(|_| "Invalid source task ID.")?,
        agent_id: owner,
        layer: MemoryLayer::Episodic,
        content: result.response.content,
        created_at_unix_ms,
        source_task_id: Some(task),
    };
    let store = MemoryStore::open(dir.join(MEMORY_DB), StorageLimits::default(), authority)
        .map_err(|error| error.to_string())?;
    match store.insert(owner, record, MemoryOrigin::Generated) {
        Ok(_) => Ok(()),
        Err(StorageError::Conflict) => Err("A memory already uses this task's ID.".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn current_roster(dir: &Path, revision: i64) -> Result<Roster, Failure> {
    load(dir)?
        .filter(|roster| roster.revision == revision)
        .ok_or("The saved binding changed. Reopen the application.")
}

fn retrieve_memory(
    dir: &Path,
    config: &AppConfig,
    owner: AgentId,
    phrase: &str,
) -> Result<MemoryContext, String> {
    let authority = configured_authorizer(config, owner, &[Permission::MemoryRead])
        .map_err(|error| error.to_string())?;
    let path = dir.join(MEMORY_DB);
    if !path
        .try_exists()
        .map_err(|_| "Could not locate desktop memory.")?
    {
        return Err(
            "Save a completed task with Remember task to initialize desktop memory.".into(),
        );
    }
    let store = MemoryStore::open_read_only(path, StorageLimits::default(), authority)
        .map_err(|error| error.to_string())?;
    MemoryContext::retrieve(&store, owner, phrase).map_err(|error| error.to_string())
}

pub(super) fn set_identity(
    dir: &Path,
    mut roster: Roster,
    selected: usize,
    name: String,
    role: String,
) -> Result<(Roster, usize), Failure> {
    let agent = roster
        .config
        .agents
        .get_mut(selected)
        .ok_or("No saved agent selected.")?;
    agent.identity.name = name;
    agent.identity.role = role;
    agent
        .identity
        .validate()
        .map_err(|_| "Enter a valid agent name and role.")?;
    Ok((save(dir, Some(roster.revision), roster.config)?, selected))
}

pub(super) fn set_hold(
    dir: &Path,
    agent: AgentId,
    hold: bool,
) -> Result<(), vespera_agents::AgentTaskStoreError> {
    let path = dir.join("desktop-tasks.sqlite3");
    if hold {
        AgentTaskStore::open(&path)?.set_suspended(agent, true)
    } else if path
        .try_exists()
        .map_err(|_| vespera_agents::AgentTaskStoreError::Database)?
    {
        AgentTaskStore::release_suspension(&path, agent)
    } else {
        Ok(())
    }
}

pub(super) async fn run(
    dir: &Path,
    mut roster: Roster,
    selected: usize,
    prompt: String,
    memory_phrase: Option<String>,
    cancel: CancellationToken,
) -> Result<FinishedTask, String> {
    current_roster(dir, roster.revision)?;
    let (agent, model, provider) = roster
        .selection(selected)
        .ok_or("No saved agent selected.")?;
    let agent = agent.identity.id;
    let context = memory_phrase
        .as_deref()
        .map(|phrase| retrieve_memory(dir, roster.config(), agent, phrase))
        .transpose()
        .map_err(|error| format!("Could not retrieve agent memory. {error}"))?;
    let used_memory = context.is_some();
    let store = AgentTaskStore::open(&dir.join("desktop-tasks.sqlite3"))
        .map_err(|_| "Could not acquire the desktop task store.")?;
    if store
        .is_suspended(agent)
        .map_err(|_| "Could not read the agent's operator hold.")?
    {
        return Err("Agent is suspended. Resume the selected agent before running a task.".into());
    }
    let native = provider.kind == ProviderKind::Ollama;
    roster.config.resources = ResourceBudget {
        ram_bytes: model
            .resources
            .ram_bytes
            .ok_or("Unknown model RAM estimate.")?,
        vram_bytes: model
            .resources
            .vram_bytes
            .ok_or("Unknown model VRAM estimate.")?,
        ..ResourceBudget::default()
    };
    roster.config.scheduling.local_host_limits = true;
    roster.config.scheduling.keep_alive_seconds = native.then_some(60);
    let runtime = VesperaRuntime::from_config_for_agent(&roster.config, agent)
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .agents()
        .attach_task_store(store)
        .map_err(|_| "Could not attach durable task ownership.")?;
    let id = TaskId::new();
    let task = Task {
        id,
        prompt,
        required_capabilities: Default::default(),
        state: TaskState::Queued,
    };
    let deadline = Duration::from_secs(50);
    let outcome = tokio::time::timeout(deadline, async {
        if native {
            runtime.reconcile_startup(cancel.clone()).await?;
        }
        runtime
            .run_agent_with_context(
                agent,
                task,
                ExecutionOptions {
                    max_output_tokens: 128,
                    timeout: deadline,
                    ..ExecutionOptions::default()
                },
                false,
                context,
                cancel.clone(),
            )
            .await
    })
    .await;
    let deadline_elapsed = outcome.is_err();
    let outcome = match outcome {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => {
            cancel.cancel();
            Err("Deadline exceeded. Remote computation stop is unverified.".into())
        }
    };
    let cleanup = runtime.cleanup(native, Duration::from_secs(5)).await;
    let cleanup_confirmed = cleanup.drained && cleanup.unload_errors.is_empty();
    match outcome {
        Ok(result) => Ok(FinishedTask {
            id,
            memory_records: used_memory.then_some(result.memory_context_ids.len()),
            response: result.response.content,
            cleanup_confirmed,
        }),
        Err(error) => {
            let state = runtime
                .agents()
                .get_task(id)
                .await
                .ok()
                .map(|task| task.state);
            let summary = match state {
                Some(_) if deadline_elapsed => format!("Task {id} timed out."),
                Some(TaskState::Cancelled) => format!("Task {id} cancelled locally."),
                Some(_) => format!("Task {id} failed."),
                None => "Task did not start.".into(),
            };
            Err(format!(
                "{summary}\n{error}{}",
                if cleanup_confirmed {
                    ""
                } else {
                    "\nRuntime cleanup was not confirmed."
                }
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(name: &str) -> Registration {
        Registration {
            provider: provider(
                ProviderKind::OpenAiCompatible,
                "http://127.0.0.1:1234/v1".into(),
            ),
            model: DiscoveredModel {
                provider_model_name: name.into(),
                digest: None,
                size_bytes: None,
                parameter_size: None,
                quantization_level: None,
                family: None,
            },
            agent_name: "Fixture".into(),
            role: "Test role".into(),
            rebind: None,
            ram_mib: 1024,
            vram_mib: 0,
        }
    }

    #[tokio::test]
    async fn a_rebinding_preserves_identity_and_fences_an_old_window() {
        let dir = std::env::temp_dir().join(format!("vespera-desktop-agent-{}", TaskId::new()));
        assert!(load(&dir).unwrap().is_none());
        let (first, _) = register(&dir, None, registration("first")).await.unwrap();
        let first_agent = first.selection(0).unwrap().0.identity.id;
        let (two, index) = register(&dir, Some(&first), registration("first"))
            .await
            .unwrap();
        assert_eq!(index, 1);
        let second_agent = two.selection(1).unwrap().0.identity.id;
        assert_ne!(first_agent, second_agent);
        assert_eq!(
            two.selection(0).unwrap().1.id,
            two.selection(1).unwrap().1.id
        );
        let mut rebind = registration("second");
        rebind.rebind = Some(first_agent);
        let (changed, index) = register(&dir, Some(&two), rebind).await.unwrap();
        assert_eq!(index, 0);
        assert!(
            register(&dir, Some(&first), registration("stale"))
                .await
                .is_err()
        );
        let reopened = load(&dir).unwrap().unwrap();
        assert_eq!(reopened.revision, 2);
        let (agent, model, provider) = reopened.selection(0).unwrap();
        assert_eq!(agent.identity.id, first_agent);
        assert_eq!(model.id, changed.selection(0).unwrap().1.id);
        assert_eq!(provider.id, first.selection(0).unwrap().2.id);
        assert!(agent.permissions.is_empty());
        assert_eq!(model.identifier, "second");
        assert_eq!(reopened.selection(1).unwrap().0.identity.id, second_agent);
        assert_eq!(reopened.selection(1).unwrap().1.identifier, "first");
        fs::remove_file(dir.join("desktop.sqlite3")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn identity_edit_preserves_binding_and_rejects_a_stale_roster() {
        let dir = std::env::temp_dir().join(format!("vespera-desktop-agent-{}", TaskId::new()));
        let (original, _) = register(&dir, None, registration("fixture")).await.unwrap();
        let (granted, _) = set_permission(&dir, original, 0, Permission::Delegate, true).unwrap();
        let (agent, model, provider) = granted.selection(0).unwrap();
        let (id, model_id, provider_id) = (agent.identity.id, model.id, provider.id);
        assert!(set_identity(&dir, granted.clone(), 0, String::new(), "Reviewer".into()).is_err());
        set_identity(
            &dir,
            granted.clone(),
            0,
            "Updated".into(),
            "Reviewer".into(),
        )
        .unwrap();
        assert!(set_identity(&dir, granted, 0, "Stale".into(), "Coordinator".into()).is_err());

        let reopened = load(&dir).unwrap().unwrap();
        let (agent, model, provider) = reopened.selection(0).unwrap();
        assert_eq!(agent.identity.id, id);
        assert_eq!(agent.identity.name, "Updated");
        assert_eq!(agent.identity.role, "Reviewer");
        assert!(agent.permissions.contains(&Permission::Delegate));
        assert_eq!(model.id, model_id);
        assert_eq!(provider.id, provider_id);
        fs::remove_file(dir.join("desktop.sqlite3")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn delegation_and_operator_hold_are_persistent_and_owner_scoped() {
        let dir = std::env::temp_dir().join(format!("vespera-desktop-agent-{}", TaskId::new()));
        let (first, _) = register(&dir, None, registration("fixture")).await.unwrap();
        let (roster, _) = register(&dir, Some(&first), registration("fixture"))
            .await
            .unwrap();
        let owner = roster.selection(0).unwrap().0.identity.id;
        let other = roster.selection(1).unwrap().0.identity.id;
        let (granted, _) = set_permission(&dir, roster, 0, Permission::Delegate, true).unwrap();
        let reopened = load(&dir).unwrap().unwrap();
        assert!(
            reopened
                .selection(0)
                .unwrap()
                .0
                .permissions
                .contains(&Permission::Delegate)
        );
        assert!(reopened.selection(1).unwrap().0.permissions.is_empty());
        assert!(set_permission(&dir, first, 0, Permission::Delegate, false).is_err());
        set_hold(&dir, owner, true).unwrap();
        let tasks = dir.join("desktop-tasks.sqlite3");
        assert!(AgentTaskStore::inspect_suspension(&tasks, owner).unwrap());
        assert!(!AgentTaskStore::inspect_suspension(&tasks, other).unwrap());
        let blocked = run(
            &dir,
            granted.clone(),
            0,
            "test".into(),
            None,
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(blocked, Err(error) if error.contains("Agent is suspended")));
        set_hold(&dir, owner, false).unwrap();
        assert!(!AgentTaskStore::inspect_suspension(&tasks, owner).unwrap());
        let (revoked, _) = set_permission(&dir, granted, 0, Permission::Delegate, false).unwrap();
        assert!(revoked.selection(0).unwrap().0.permissions.is_empty());
        fs::remove_file(tasks).unwrap();
        fs::remove_file(dir.join("desktop.sqlite3")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn credential_references_are_saved_without_resolving_secret_values() {
        let dir = std::env::temp_dir().join(format!("vespera-credential-{}", TaskId::new()));
        let mut configured = registration("fixture");
        configured.provider.credential_env = Some("VESPERA_TEST_PROVIDER_TOKEN".into());
        register(&dir, None, configured).await.unwrap();
        let roster = load(&dir).unwrap().unwrap();
        assert_eq!(
            roster.selection(0).unwrap().2.credential_env.as_deref(),
            Some("VESPERA_TEST_PROVIDER_TOKEN")
        );
        assert!(!credential_name("PATH"));
        assert!(!credential_name("VESPERA_"));
        assert!(!credential_name("VESPERA_invalid"));
        fs::remove_file(dir.join("desktop.sqlite3")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn windows_credential_binding_survives_reopening_the_profile() {
        let dir = std::env::temp_dir().join(format!("vespera-credential-{}", TaskId::new()));
        let mut configured = registration("fixture");
        configured.provider.credential_windows = true;
        let endpoint = configured.provider.endpoint.clone();
        register(&dir, None, configured).await.unwrap();
        let roster = load(&dir).unwrap().unwrap();
        let (_, _, provider) = roster.selection(0).unwrap();
        assert!(provider.credential_windows);
        assert!(provider.credential_env.is_none());
        assert_eq!(provider.endpoint, endpoint);
        fs::remove_file(dir.join("desktop.sqlite3")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn registration_preserves_an_existing_foreign_file() {
        let dir = std::env::temp_dir().join(format!("vespera-desktop-agent-{}", TaskId::new()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("desktop.sqlite3");
        fs::write(&path, b"").unwrap();
        assert!(register(&dir, None, registration("fixture")).await.is_err());
        assert!(fs::read(&path).unwrap().is_empty());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
