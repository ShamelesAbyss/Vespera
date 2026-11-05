//! Universal inference provider abstraction layer and protocol adapters for VESPERA.
//!
//! This crate implements Milestone 2 and Milestone 3 of the VESPERA roadmap:
//! - Object-safe asynchronous `InferenceProvider` trait (`dyn InferenceProvider`).
//! - Explicit lifecycle capability negotiation (`Supported`, `Unsupported`, `Unknown`).
//! - Connection-time transport security (loopback enforcement, redirect prohibition, secret redaction).
//! - Production Ollama adapter supporting residency telemetry and NDJSON streaming.
//! - Production OpenAI-compatible adapter supporting SSE chat streaming and model discovery.

pub mod contract;
pub mod error;
pub mod ollama;
pub mod openai;
pub mod policy;

pub use contract::{
    BoundedInferenceRequest, BoxFuture, BoxStream, CancellationToken, ChatMessage, ChatRole,
    DiscoveredModel, DiscoveredModelMetadata, InferenceFinishReason, InferenceMetrics,
    InferenceProvider, InferenceResponse, InferenceStream, LifecycleCapabilityState,
    ModelLifecycleOutcome, ModelResidencyRecord, ProviderHealthReport, ProviderHealthStatus,
    ProviderKind, ProviderLifecycleSupport, ProviderTelemetrySnapshot, StreamChunk,
};
pub use error::ProviderError;
pub use ollama::{OllamaConfig, OllamaProvider};
pub use openai::{OpenAiCompatibleProvider, OpenAiConfig};
pub use policy::{EndpointPolicy, SecretRef, SecretString, ValidatedEndpoint};

/// Core domain bindings re-exported from `vespera-core`.
pub mod core_bridge {
    pub use vespera_core::{Capability, ModelId, ProviderId, ResourceEstimate};
}
