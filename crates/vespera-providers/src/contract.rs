//! Object-safe asynchronous provider contract and normalized inference domain types.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_core::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::core_bridge::{Capability, ProviderId, ResourceEstimate};
use crate::error::ProviderError;

/// Object-safe pinned future alias.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe pinned stream alias.
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;

/// Stream of incremental inference tokens/chunks.
pub type InferenceStream<'a> = BoxStream<'a, Result<StreamChunk, ProviderError>>;

/// Cooperative, thread-safe cancellation token for aborting in-flight requests and streams.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl CancellationToken {
    /// Creates a new uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Signals cancellation to all tasks observing this token.
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::SeqCst) {
            self.notify.notify_waiters();
        }
    }

    /// Returns `true` if cancellation has already been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Completes immediately if already cancelled, or awaits the cancellation signal.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

/// Supported inference provider families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Native Ollama HTTP/NDJSON API.
    Ollama,
    /// OpenAI-compatible v1 API (LM Studio, llama.cpp server, vLLM, etc.).
    OpenAiCompatible,
    /// Bionic provider family (pending authoritative protocol specification in Milestone 3).
    Bionic,
    /// Custom user-defined adapter.
    Custom,
}

/// Tri-state support declaration for optional provider lifecycle capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCapabilityState {
    /// Provider natively supports and enforces this operation.
    Supported,
    /// Provider protocol does not support this operation.
    Unsupported,
    /// Support depends on backend configuration and cannot be statically guaranteed.
    Unknown,
}

/// Explicit negotiation matrix of lifecycle features supported by a provider adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderLifecycleSupport {
    /// Ability to list available models (`/api/tags` or `/v1/models`).
    pub model_enumeration: LifecycleCapabilityState,
    /// Ability to inspect model parameter count, quantization, and context length (`/api/show`).
    pub model_metadata: LifecycleCapabilityState,
    /// Ability to explicitly preload/warm a model into RAM/VRAM before inference.
    pub explicit_model_load: LifecycleCapabilityState,
    /// Ability to explicitly evict/unload a model from RAM/VRAM without stopping the daemon.
    pub explicit_model_unload: LifecycleCapabilityState,
    /// Ability to query currently loaded models and their measured RAM/VRAM footprint (`/api/ps`).
    pub residency_telemetry: LifecycleCapabilityState,
}

/// Operational health state of a provider endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthStatus {
    /// Provider responded within timeout and passed protocol validation.
    Healthy,
    /// Provider responded with degraded status or partial capability.
    Degraded,
    /// Provider is unreachable or failing health checks.
    Unreachable,
}

/// Result of a provider health check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthReport {
    /// Provider identifier.
    pub provider_id: ProviderId,
    /// Health status.
    pub status: ProviderHealthStatus,
    /// Reported server version string when available.
    pub version: Option<String>,
    /// Monotonic round-trip latency in milliseconds.
    pub latency_ms: u64,
}

/// Summary of a model discovered during provider enumeration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    /// Provider-specific model identifier (e.g., `"llama3.2:3b"`).
    pub provider_model_name: String,
    /// Content digest when reported by the provider.
    pub digest: Option<String>,
    /// Total model disk/weight size in bytes when known.
    pub size_bytes: Option<u64>,
    /// Parameter count string when reported (e.g., `"3.2B"`).
    pub parameter_size: Option<String>,
    /// Quantization level when reported (e.g., `"Q4_K_M"`).
    pub quantization_level: Option<String>,
    /// Model family when reported (e.g., `"llama"`).
    pub family: Option<String>,
}

/// Detailed model metadata retrieved from deep inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredModelMetadata {
    /// Provider-specific model identifier.
    pub provider_model_name: String,
    /// Model family or architecture.
    pub architecture: Option<String>,
    /// Parameter size description.
    pub parameter_size: Option<String>,
    /// Quantization scheme.
    pub quantization_level: Option<String>,
    /// Maximum context window in tokens when reported by model metadata.
    pub context_window_tokens: Option<u32>,
    /// Normalized VESPERA capabilities inferred or reported by the provider.
    pub capabilities: BTreeSet<Capability>,
    /// Estimated resource footprint when determinable from weight metadata.
    pub estimated_resources: ResourceEstimate,
}

/// Live residency measurement for a single currently loaded model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelResidencyRecord {
    /// Provider-specific model identifier.
    pub provider_model_name: String,
    /// Total memory (RAM + VRAM) currently occupied in bytes.
    pub total_resident_bytes: u64,
    /// Measured GPU VRAM currently occupied in bytes.
    pub vram_resident_bytes: u64,
    /// Computed host system RAM occupied in bytes (`total - vram`).
    pub ram_resident_bytes: u64,
    /// Context length currently allocated for the resident instance, if reported.
    pub allocated_context_tokens: Option<u32>,
}

/// Snapshot of active model residency across a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTelemetrySnapshot {
    /// Provider identifier.
    pub provider_id: ProviderId,
    /// Models currently resident in memory on this provider.
    pub resident_models: Vec<ModelResidencyRecord>,
    /// Sum of `vram_resident_bytes` across all resident models on this provider.
    pub total_provider_vram_bytes: u64,
    /// Sum of `ram_resident_bytes` across all resident models on this provider.
    pub total_provider_ram_bytes: u64,
}

/// Outcome of an explicit model load or unload lifecycle command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLifecycleOutcome {
    /// Provider identifier.
    pub provider_id: ProviderId,
    /// Target model name.
    pub provider_model_name: String,
    /// Whether the provider acknowledged the state transition.
    pub acknowledged: bool,
    /// Monotonic duration of the lifecycle operation in milliseconds.
    pub elapsed_ms: u64,
}

/// Role of a participant in a structured chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// System instruction.
    System,
    /// User or orchestrator prompt.
    User,
    /// Assistant response.
    Assistant,
    /// Tool execution output.
    Tool,
}

/// Single message in an inference conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    /// Participant role.
    pub role: ChatRole,
    /// Text content of the message.
    pub content: String,
}

/// Validated, resource-bounded inference request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawBoundedInferenceRequest")]
pub struct BoundedInferenceRequest {
    /// Target provider-specific model name.
    model_name: String,
    /// Ordered conversation messages.
    messages: Vec<ChatMessage>,
    /// Maximum tokens to generate (`1..=131_072`).
    max_output_tokens: u32,
    /// Context window ceiling for this request (`64..=1_048_576`).
    context_window_limit: u32,
    /// Optional sampling temperature (`0.0..=2.0`).
    temperature: Option<f32>,
    /// Optional keep-alive duration in seconds after completion.
    keep_alive_secs: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBoundedInferenceRequest {
    model_name: String,
    messages: Vec<ChatMessage>,
    max_output_tokens: u32,
    context_window_limit: u32,
    temperature: Option<f32>,
    keep_alive_secs: Option<u64>,
}
impl TryFrom<RawBoundedInferenceRequest> for BoundedInferenceRequest {
    type Error = ProviderError;
    fn try_from(raw: RawBoundedInferenceRequest) -> Result<Self, Self::Error> {
        Self::new(
            raw.model_name,
            raw.messages,
            raw.max_output_tokens,
            raw.context_window_limit,
            raw.temperature,
            raw.keep_alive_secs,
        )
    }
}

impl BoundedInferenceRequest {
    /// Constructs and validates a bounded inference request.
    pub fn new(
        model_name: impl Into<String>,
        messages: Vec<ChatMessage>,
        max_output_tokens: u32,
        context_window_limit: u32,
        temperature: Option<f32>,
        keep_alive_secs: Option<u64>,
    ) -> Result<Self, ProviderError> {
        let model_name = model_name.into();
        if model_name.len() > 512 || messages.len() > 256 {
            return Err(ProviderError::RequestValidation {
                reason: "model name exceeds 512 bytes or conversation exceeds 256 messages".into(),
            });
        }
        let mut input_bytes = 0usize;
        for message in &messages {
            input_bytes = input_bytes
                .checked_add(message.content.len())
                .ok_or_else(|| ProviderError::RequestValidation {
                    reason: "conversation byte count overflow".into(),
                })?;
            if input_bytes > 1024 * 1024 {
                return Err(ProviderError::RequestValidation {
                    reason: "conversation exceeds one MiB of content".into(),
                });
            }
        }
        if model_name.trim().is_empty() {
            return Err(ProviderError::RequestValidation {
                reason: "model_name must not be empty".to_string(),
            });
        }
        if messages.is_empty() {
            return Err(ProviderError::RequestValidation {
                reason: "messages list must not be empty".to_string(),
            });
        }
        if messages.iter().all(|m| m.content.trim().is_empty()) {
            return Err(ProviderError::RequestValidation {
                reason: "at least one message must contain non-empty content".to_string(),
            });
        }
        if max_output_tokens == 0 || max_output_tokens > 131_072 {
            return Err(ProviderError::RequestValidation {
                reason: "max_output_tokens must be between 1 and 131072".to_string(),
            });
        }
        if !(64..=1_048_576).contains(&context_window_limit) {
            return Err(ProviderError::RequestValidation {
                reason: "context_window_limit must be between 64 and 1048576".to_string(),
            });
        }
        if max_output_tokens > context_window_limit {
            return Err(ProviderError::RequestValidation {
                reason: "max_output_tokens cannot exceed context_window_limit".to_string(),
            });
        }
        if let Some(temp) = temperature
            && (!temp.is_finite() || !(0.0..=2.0).contains(&temp))
        {
            return Err(ProviderError::RequestValidation {
                reason: "temperature must be a finite value between 0.0 and 2.0".to_string(),
            });
        }

        Ok(Self {
            model_name,
            messages,
            max_output_tokens,
            context_window_limit,
            temperature,
            keep_alive_secs,
        })
    }

    /// Target model identifier.
    #[must_use]
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Conversation messages.
    #[must_use]
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Maximum tokens to generate.
    #[must_use]
    pub const fn max_output_tokens(&self) -> u32 {
        self.max_output_tokens
    }

    /// Context window ceiling in tokens.
    #[must_use]
    pub const fn context_window_limit(&self) -> u32 {
        self.context_window_limit
    }

    /// Sampling temperature if specified.
    #[must_use]
    pub const fn temperature(&self) -> Option<f32> {
        self.temperature
    }

    /// Keep-alive duration in seconds if specified.
    #[must_use]
    pub const fn keep_alive_secs(&self) -> Option<u64> {
        self.keep_alive_secs
    }
}

/// Reason an inference generation terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceFinishReason {
    /// Model reached a natural stop token.
    Stop,
    /// Generation hit the `max_output_tokens` or context ceiling.
    Length,
    /// Model requested a tool invocation.
    ToolCalls,
    /// Provider unloaded the model or completed a lifecycle request.
    Unload,
    /// Reason was not specified by the provider.
    Unknown,
}

/// Measured execution telemetry separated from model-generated content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceMetrics {
    /// Client-observed monotonic wall duration in milliseconds.
    pub client_elapsed_ms: u64,
    /// Prompt token count reported by the provider, if available.
    pub prompt_tokens: Option<u32>,
    /// Generated completion token count reported by the provider, if available.
    pub completion_tokens: Option<u32>,
    /// Provider-reported model load duration in milliseconds, if available.
    pub provider_load_duration_ms: Option<u64>,
    /// Provider-reported token evaluation duration in milliseconds, if available.
    pub provider_eval_duration_ms: Option<u64>,
}

/// Complete response from a non-streaming inference call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Provider that executed the inference.
    pub provider_id: ProviderId,
    /// Model that executed the inference.
    pub model_name: String,
    /// Assistant output text.
    pub content: String,
    /// Termination reason.
    pub finish_reason: InferenceFinishReason,
    /// Measured execution metrics.
    pub metrics: InferenceMetrics,
}

/// Single incremental delta emitted during streaming inference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamChunk {
    /// Incremental text delta.
    pub delta: String,
    /// Whether this chunk marks the final completion frame of the stream.
    pub done: bool,
    /// Finish reason, populated on the final chunk (`done == true`).
    pub finish_reason: Option<InferenceFinishReason>,
    /// Final execution metrics, populated on the final chunk (`done == true`).
    pub metrics: Option<InferenceMetrics>,
}

/// Object-safe universal interface for all local and remote inference engines in VESPERA.
pub trait InferenceProvider: Send + Sync + 'static {
    /// Returns the unique VESPERA `ProviderId` assigned to this instance.
    fn provider_id(&self) -> ProviderId;

    /// Returns the provider family (`Ollama`, `OpenAiCompatible`, `Bionic`, `Custom`).
    fn provider_kind(&self) -> ProviderKind;

    /// Returns the static and negotiated lifecycle capability matrix for this provider.
    fn lifecycle_support(&self) -> ProviderLifecycleSupport;

    /// Checks provider reachability, protocol responsiveness, and round-trip latency.
    fn health_check<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderHealthReport, ProviderError>>;

    /// Enumerates all models currently installed or exposed by the provider.
    fn enumerate_models<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<Vec<DiscoveredModel>, ProviderError>>;

    /// Retrieves deep metadata (architecture, parameter count, quantization, context length, capabilities)
    /// for a specific model.
    fn model_metadata<'a>(
        &'a self,
        model_name: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<DiscoveredModelMetadata, ProviderError>>;

    /// Queries live RAM and VRAM residency for currently loaded models when supported.
    fn query_residency<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderTelemetrySnapshot, ProviderError>>;

    /// Explicitly preloads/warms a model into memory with an optional keep-alive lease.
    fn load_model<'a>(
        &'a self,
        model_name: &'a str,
        keep_alive: Option<Duration>,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>>;

    /// Explicitly unloads/evicts a model from VRAM/RAM immediately.
    fn unload_model<'a>(
        &'a self,
        model_name: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>>;

    /// Executes a bounded, non-streaming inference request.
    fn infer<'a>(
        &'a self,
        request: BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InferenceResponse, ProviderError>>;

    /// Executes a bounded, streaming inference request yielding incremental [`StreamChunk`] frames.
    fn infer_stream<'a>(
        &'a self,
        request: BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InferenceStream<'a>, ProviderError>>;
}
