use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Dimension of a resource limit, used by errors and event payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Ram,
    Vram,
    ConcurrentInferences,
    ContextTokens,
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ram => "RAM bytes",
            Self::Vram => "VRAM bytes",
            Self::ConcurrentInferences => "concurrent inferences",
            Self::ContextTokens => "context tokens",
        })
    }
}

/// Optional estimates: `None` means unknown, never zero or an admission guarantee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceEstimate {
    #[serde(default)]
    pub ram_bytes: Option<u64>,
    #[serde(default)]
    pub vram_bytes: Option<u64>,
}

impl ResourceEstimate {
    /// Zero is a valid known estimate; both unknown dimensions are valid.
    pub fn validate(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

/// User-specified arithmetic limits, not measured available hardware.
///
/// Context is a per-request limit, not an aggregate across active requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceBudget {
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub max_concurrent_inferences: u32,
    pub max_context_tokens: u32,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            ram_bytes: 4 * 1024 * 1024 * 1024,
            vram_bytes: 0,
            max_concurrent_inferences: 1,
            max_context_tokens: 4096,
        }
    }
}

impl ResourceBudget {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.ram_bytes == 0 {
            return Err(DomainError::ZeroValue {
                field: "resource_budget.ram_bytes",
            });
        }
        if self.max_concurrent_inferences == 0 {
            return Err(DomainError::ZeroValue {
                field: "resource_budget.max_concurrent_inferences",
            });
        }
        if self.max_context_tokens == 0 {
            return Err(DomainError::ZeroValue {
                field: "resource_budget.max_context_tokens",
            });
        }
        Ok(())
    }

    /// Check one inference request and return the prospective aggregate usage.
    ///
    /// This is a pure calculation: it neither mutates the supplied usage nor
    /// reserves memory, schedules work, loads a model, or controls hardware.
    /// Callers must supply independently established, complete resource costs
    /// (including provider overhead and headroom). Unknown model estimates must
    /// not be converted to zero. Future schedulers must make admission atomic.
    ///
    /// All additions are checked. Usage that already exceeds a budget is also
    /// rejected; a zero-VRAM request is valid for a CPU-only budget.
    pub fn admit(
        &self,
        usage: &ResourceUsage,
        request: &ResourceRequest,
    ) -> Result<ResourceUsage, DomainError> {
        self.validate()?;
        usage.validate()?;
        request.validate()?;

        let ram_bytes = usage.ram_bytes.checked_add(request.ram_bytes).ok_or(
            DomainError::ResourceOverflow {
                resource: ResourceKind::Ram,
            },
        )?;
        let vram_bytes = usage.vram_bytes.checked_add(request.vram_bytes).ok_or(
            DomainError::ResourceOverflow {
                resource: ResourceKind::Vram,
            },
        )?;
        let active_inferences =
            usage
                .active_inferences
                .checked_add(1)
                .ok_or(DomainError::ResourceOverflow {
                    resource: ResourceKind::ConcurrentInferences,
                })?;

        for (resource, requested, limit) in [
            (ResourceKind::Ram, ram_bytes, self.ram_bytes),
            (ResourceKind::Vram, vram_bytes, self.vram_bytes),
            (
                ResourceKind::ConcurrentInferences,
                u64::from(active_inferences),
                u64::from(self.max_concurrent_inferences),
            ),
            (
                ResourceKind::ContextTokens,
                u64::from(request.context_tokens),
                u64::from(self.max_context_tokens),
            ),
        ] {
            if requested > limit {
                return Err(DomainError::ResourceLimitExceeded {
                    resource,
                    requested,
                    limit,
                });
            }
        }

        Ok(ResourceUsage {
            ram_bytes,
            vram_bytes,
            active_inferences,
        })
    }
}

/// Supplied current usage, not telemetry collected by this crate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceUsage {
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub active_inferences: u32,
}

impl ResourceUsage {
    /// All nonnegative values are representable; budgets are checked by `admit`.
    pub fn validate(&self) -> Result<(), DomainError> {
        Ok(())
    }
}

/// Scheduler knowledge of a reservation, distinct from physical residency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationState {
    /// Load in flight; no usable inference lease has been issued.
    Loading,
    /// Provider acknowledged explicit load; memory amounts remain estimates.
    Loaded,
    /// Provider controls loading; physical residency is unverified.
    ProviderManaged,
    /// Unload in flight; memory remains charged.
    Unloading,
    /// Error, cancellation or dropped operation left provider state uncertain.
    Unknown,
}

/// Identity and accounting from one local scheduler reservation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelReservationSample {
    pub model_id: crate::ModelId,
    pub provider_id: crate::ProviderId,
    pub state: ReservationState,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    /// Charged slots include preparation before a lease has been issued.
    pub charged_slots: u32,
    pub issued_leases: u32,
}

/// Payload-free provenance of an agent's running local task and issued lease.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLeaseSample {
    pub task_id: crate::TaskId,
    pub agent_id: crate::AgentId,
    pub lease_id: uuid::Uuid,
    pub model_id: crate::ModelId,
    pub provider_id: crate::ProviderId,
}

/// Bounded local snapshot published by one CLI runtime. Reservations are estimates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerSample {
    pub schema_version: u32,
    pub observed_unix_ms: u64,
    /// The command completed its execution and cleanup attempt.
    pub finished: bool,
    pub waiting: usize,
    /// Granted permits include work still in preparation.
    pub admitted: usize,
    pub queue_closed: bool,
    pub budget: ResourceBudget,
    pub reserved: ResourceUsage,
    pub model_reservations: usize,
    pub uncertain_reservations: usize,
    pub provider_health: ProviderHealthCounts,
    /// Version 1 carried only aggregate counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelReservationSample>>,
    /// Version 3 adds bounded ownership for leases whose model is displayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<TaskLeaseSample>>,
}

impl SchedulerSample {
    pub const SCHEMA_VERSION: u32 = 3;
    pub const FILE_NAME: &str = "scheduler.json";
    pub const MAX_FILE_BYTES: u64 = 16384;
    pub const MAX_MODELS: usize = 16;
    pub const MAX_TASKS: usize = 16;

    pub fn is_supported(&self) -> bool {
        if self.budget.validate().is_err() {
            return false;
        }
        if self.schema_version == 1 {
            return self.models.is_none() && self.tasks.is_none();
        }
        let Some(models) = &self.models else {
            return false;
        };
        let mut issued = std::collections::BTreeMap::new();
        if models.len() != self.model_reservations.min(Self::MAX_MODELS)
            || !models.iter().all(|model| {
                model.model_id.validate().is_ok()
                    && model.provider_id.validate().is_ok()
                    && model.issued_leases <= model.charged_slots
                    && issued
                        .insert(model.model_id, (model.provider_id, model.issued_leases))
                        .is_none()
            })
        {
            return false;
        }
        match (self.schema_version, &self.tasks) {
            (2, None) => true,
            (Self::SCHEMA_VERSION, Some(tasks)) if tasks.len() <= Self::MAX_TASKS => {
                let mut ids = std::collections::BTreeSet::new();
                let mut leases = std::collections::BTreeSet::new();
                tasks.iter().all(|task| {
                    task.task_id.validate().is_ok()
                        && task.agent_id.validate().is_ok()
                        && !task.lease_id.is_nil()
                        && ids.insert(task.task_id)
                        && leases.insert(task.lease_id)
                        && issued
                            .get_mut(&task.model_id)
                            .is_some_and(|(provider, count)| {
                                if *provider != task.provider_id || *count == 0 {
                                    return false;
                                }
                                *count -= 1;
                                true
                            })
                })
            }
            _ => false,
        }
    }
}

/// Effective observations already held by this runtime's provider registry.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderHealthCounts {
    pub unknown: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub unreachable: usize,
    pub stale: usize,
}

/// Explicit resource costs for one prospective inference; no unknown dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequest {
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub context_tokens: u32,
}

impl ResourceRequest {
    /// Memory increments may be zero, e.g. for an already-resident CPU model.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.context_tokens == 0 {
            Err(DomainError::ZeroValue {
                field: "resource_request.context_tokens",
            })
        } else {
            Ok(())
        }
    }
}
