use vespera_core::{
    DomainError, ResourceBudget, ResourceEstimate, ResourceKind, ResourceRequest, ResourceUsage,
};

fn budget() -> ResourceBudget {
    ResourceBudget {
        ram_bytes: 100,
        vram_bytes: 50,
        max_concurrent_inferences: 2,
        max_context_tokens: 4096,
    }
}

fn request() -> ResourceRequest {
    ResourceRequest {
        ram_bytes: 20,
        vram_bytes: 10,
        context_tokens: 4096,
    }
}

#[test]
fn defaults_are_conservative_and_estimates_stay_unknown() {
    let default = ResourceBudget::default();
    assert_eq!(default.ram_bytes, 4 * 1024 * 1024 * 1024);
    assert_eq!(default.vram_bytes, 0);
    assert_eq!(default.max_concurrent_inferences, 1);
    assert_eq!(default.max_context_tokens, 4096);
    default.validate().unwrap();
    let estimate: ResourceEstimate = serde_json::from_str("{}").unwrap();
    assert_eq!(estimate, ResourceEstimate::default());
    assert_eq!(estimate.ram_bytes, None);
    assert_eq!(estimate.vram_bytes, None);
    let encoded = serde_json::to_value(estimate).unwrap();
    assert!(encoded["ram_bytes"].is_null());
    assert!(encoded["vram_bytes"].is_null());
    let known_zero: ResourceEstimate = serde_json::from_str("{\"vram_bytes\":0}").unwrap();
    assert_eq!(known_zero.vram_bytes, Some(0));
    known_zero.validate().unwrap();
}

#[test]
fn admission_accepts_exact_boundary_and_does_not_mutate_inputs() {
    let usage = ResourceUsage {
        ram_bytes: 80,
        vram_bytes: 40,
        active_inferences: 1,
    };
    let before = usage;
    let admitted = budget().admit(&usage, &request()).unwrap();
    assert_eq!(
        admitted,
        ResourceUsage {
            ram_bytes: 100,
            vram_bytes: 50,
            active_inferences: 2
        }
    );
    assert_eq!(usage, before);
}

#[test]
fn zero_gpu_budget_allows_cpu_inference_but_not_gpu_cost() {
    let mut request = ResourceRequest {
        ram_bytes: 1024,
        vram_bytes: 0,
        context_tokens: 1,
    };
    let usage = ResourceUsage::default();
    let admitted = ResourceBudget::default().admit(&usage, &request).unwrap();
    assert_eq!(admitted.vram_bytes, 0);
    request.vram_bytes = 1;
    assert_eq!(
        ResourceBudget::default().admit(&usage, &request),
        Err(DomainError::ResourceLimitExceeded {
            resource: ResourceKind::Vram,
            requested: 1,
            limit: 0,
        })
    );
}

#[test]
fn each_resource_limit_has_a_typed_diagnostic() {
    let usage = ResourceUsage::default();
    for (request, resource, requested, limit) in [
        (
            ResourceRequest {
                ram_bytes: 101,
                ..request()
            },
            ResourceKind::Ram,
            101,
            100,
        ),
        (
            ResourceRequest {
                vram_bytes: 51,
                ..request()
            },
            ResourceKind::Vram,
            51,
            50,
        ),
        (
            ResourceRequest {
                context_tokens: 4097,
                ..request()
            },
            ResourceKind::ContextTokens,
            4097,
            4096,
        ),
    ] {
        assert_eq!(
            budget().admit(&usage, &request),
            Err(DomainError::ResourceLimitExceeded {
                resource,
                requested,
                limit
            })
        );
    }
    let busy = ResourceUsage {
        active_inferences: 2,
        ..usage
    };
    assert_eq!(
        budget().admit(&busy, &request()),
        Err(DomainError::ResourceLimitExceeded {
            resource: ResourceKind::ConcurrentInferences,
            requested: 3,
            limit: 2,
        })
    );
}

#[test]
fn all_aggregate_arithmetic_is_checked() {
    let budget = ResourceBudget {
        ram_bytes: u64::MAX,
        vram_bytes: u64::MAX,
        max_concurrent_inferences: u32::MAX,
        max_context_tokens: u32::MAX,
    };
    let request = ResourceRequest {
        ram_bytes: 1,
        vram_bytes: 1,
        context_tokens: 1,
    };
    for (usage, resource) in [
        (
            ResourceUsage {
                ram_bytes: u64::MAX,
                ..ResourceUsage::default()
            },
            ResourceKind::Ram,
        ),
        (
            ResourceUsage {
                vram_bytes: u64::MAX,
                ..ResourceUsage::default()
            },
            ResourceKind::Vram,
        ),
        (
            ResourceUsage {
                active_inferences: u32::MAX,
                ..ResourceUsage::default()
            },
            ResourceKind::ConcurrentInferences,
        ),
    ] {
        assert_eq!(
            budget.admit(&usage, &request),
            Err(DomainError::ResourceOverflow { resource })
        );
    }
}

#[test]
fn invalid_limits_and_zero_context_are_rejected() {
    for invalid in [
        ResourceBudget {
            ram_bytes: 0,
            ..budget()
        },
        ResourceBudget {
            max_concurrent_inferences: 0,
            ..budget()
        },
        ResourceBudget {
            max_context_tokens: 0,
            ..budget()
        },
    ] {
        assert!(invalid.validate().is_err());
        assert!(
            invalid
                .admit(&ResourceUsage::default(), &request())
                .is_err()
        );
    }
    assert!(
        budget()
            .admit(
                &ResourceUsage::default(),
                &ResourceRequest {
                    context_tokens: 0,
                    ..request()
                }
            )
            .is_err()
    );
}

#[test]
fn already_over_budget_usage_is_rejected_even_with_zero_memory_increment() {
    let request = ResourceRequest {
        ram_bytes: 0,
        vram_bytes: 0,
        context_tokens: 1,
    };
    let usage = ResourceUsage {
        ram_bytes: 101,
        vram_bytes: 0,
        active_inferences: 0,
    };
    assert_eq!(
        budget().admit(&usage, &request),
        Err(DomainError::ResourceLimitExceeded {
            resource: ResourceKind::Ram,
            requested: 101,
            limit: 100,
        })
    );
}
