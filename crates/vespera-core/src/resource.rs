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
