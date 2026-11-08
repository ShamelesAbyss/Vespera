use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use vespera_agents::{AgentTaskStore, ExecutionOptions};
use vespera_app::runtime::{VesperaRuntime, configured_registry};
use vespera_config::{AppConfig, CONFIG_SCHEMA_VERSION, ProviderConfig};
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, Capability, ModelAvailability,
    ModelDescriptor, ModelId, ModelSelectionPolicy, ProviderId, ProviderKind, ResourceBudget,
    ResourceEstimate, Task, TaskId, TaskState,
};
use vespera_providers::{CancellationToken, DiscoveredModel};

const APPLICATION_ID: i32 = 0x5644_534b;
const MAX_CONFIG: usize = 65_536;
const MIB: u64 = 1024 * 1024;

#[derive(Clone)]
pub(super) struct Binding {
    pub revision: i64,
    config: AppConfig,
}

impl Binding {
    pub fn agent(&self) -> &AgentDefinition {
        &self.config.agents[0]
    }

    pub fn model(&self) -> &ModelDescriptor {
        &self.config.models[0]
    }

    pub fn provider(&self) -> &ProviderConfig {
        &self.config.providers[0]
    }
}

pub(super) struct Registration {
    pub provider: ProviderConfig,
    pub model: DiscoveredModel,
    pub agent_name: String,
    pub ram_mib: u32,
    pub vram_mib: u32,
}

pub(super) struct FinishedTask {
    pub id: TaskId,
    pub response: String,
    pub cleanup_confirmed: bool,
}

type Failure = &'static str;

const STORAGE: Failure = "Could not access the desktop profile database.";
const FORMAT: Failure = "The desktop profile database has an unrecognized format.";

pub(super) fn load(dir: &Path) -> Result<Option<Binding>, Failure> {
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

fn read(db: &Connection) -> Result<Option<Binding>, Failure> {
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
        || config.providers.len() != 1
        || config.models.len() != 1
        || config.agents.len() != 1
        || config.network.allow_remote
        || config.providers[0].credential_env.is_some()
    {
        return Err(FORMAT);
    }
    config.validate().map_err(|_| FORMAT)?;
    Ok(Some(Binding { revision, config }))
}

pub(super) fn provider(kind: ProviderKind, endpoint: String) -> ProviderConfig {
    ProviderConfig {
        id: ProviderId::new(),
        name: "Local provider".into(),
        kind,
        endpoint,
        credential_env: None,
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

pub(super) async fn register(
    dir: &Path,
    previous: Option<&Binding>,
    registration: Registration,
) -> Result<Binding, Failure> {
    let Registration {
        mut provider,
        model,
        agent_name,
        ram_mib,
        vram_mib,
    } = registration;
    if let Some(previous) = previous
        && previous.provider().kind == provider.kind
        && previous.provider().endpoint == provider.endpoint
    {
        provider.id = previous.provider().id;
    }
    let native = provider.kind == ProviderKind::Ollama;
    let mut config = config(provider);
    config.providers[0].request_timeout_seconds = 5;
    let id = config.providers[0].id;
    let text = Capability::new("text-generation").expect("static capability is valid");
    let mut context = None;
    let mut quantization = model.quantization_level;
    if native {
        let registry = configured_registry(&config)
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
    config.providers[0].request_timeout_seconds = 45;
    let model_id = previous
        .filter(|old| {
            old.model().identifier == model.provider_model_name && old.provider().id == id
        })
        .map_or_else(ModelId::new, |old| old.model().id);
    let ram_bytes = u64::from(ram_mib) * MIB;
    let vram_bytes = u64::from(vram_mib) * MIB;
    config.resources = ResourceBudget {
        ram_bytes,
        vram_bytes,
        ..ResourceBudget::default()
    };
    config.scheduling.local_host_limits = true;
    config.scheduling.keep_alive_seconds = native.then_some(60);
    config.models.push(ModelDescriptor {
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
    });
    config.agents.push(AgentDefinition {
        identity: AgentIdentity {
            id: previous.map_or_else(AgentId::new, |old| old.agent().identity.id),
            name: agent_name,
            description: String::new(),
            role: "Desktop text assistant".into(),
            personality: String::new(),
            goals: vec![],
        },
        model_policy: ModelSelectionPolicy::Fixed { model_id },
        permissions: Default::default(),
        status: AgentStatus::Idle,
    });
    config
        .validate()
        .map_err(|_| "Invalid agent name, model, or resource declaration.")?;
    save(dir, previous.map(|binding| binding.revision), config)
}

fn save(dir: &Path, expected: Option<i64>, config: AppConfig) -> Result<Binding, Failure> {
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
    Ok(Binding { revision, config })
}

pub(super) async fn run(
    dir: &Path,
    binding: Binding,
    prompt: String,
    cancel: CancellationToken,
) -> Result<FinishedTask, String> {
    let store = AgentTaskStore::open(&dir.join("desktop-tasks.sqlite3"))
        .map_err(|_| "Could not acquire the desktop task store.")?;
    if load(dir)?.is_none_or(|current| current.revision != binding.revision) {
        return Err("The saved binding changed. Reopen the application.".into());
    }
    let agent = binding.agent().identity.id;
    let native = binding.provider().kind == ProviderKind::Ollama;
    let runtime = VesperaRuntime::from_config_for_agent(&binding.config, agent)
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
            .run_agent(
                agent,
                task,
                ExecutionOptions {
                    max_output_tokens: 128,
                    timeout: deadline,
                    ..ExecutionOptions::default()
                },
                false,
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
            ram_mib: 1024,
            vram_mib: 0,
        }
    }

    #[tokio::test]
    async fn a_rebinding_preserves_identity_and_fences_an_old_window() {
        let dir = std::env::temp_dir().join(format!("vespera-desktop-agent-{}", TaskId::new()));
        assert!(load(&dir).unwrap().is_none());
        let first = register(&dir, None, registration("first")).await.unwrap();
        let second = register(&dir, Some(&first), registration("second"))
            .await
            .unwrap();
        assert!(
            register(&dir, Some(&first), registration("stale"))
                .await
                .is_err()
        );
        let reopened = load(&dir).unwrap().unwrap();
        assert_eq!(reopened.revision, 1);
        assert_eq!(reopened.model().id, second.model().id);
        assert_ne!(reopened.model().id, first.model().id);
        assert_eq!(reopened.provider().id, first.provider().id);
        assert_eq!(reopened.agent().identity.id, first.agent().identity.id);
        assert!(reopened.agent().permissions.is_empty());
        assert_eq!(reopened.model().identifier, "second");
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
