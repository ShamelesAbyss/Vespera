//! OpenAI-compatible provider adapter for VESPERA.
//!
//! Supports local engines exposing standard OpenAI v1 endpoints:
//! - `GET /v1/models` (Model enumeration)
//! - `POST /v1/chat/completions` (Non-streaming and SSE streaming chat generation)

use std::time::Duration;

use bytes::BytesMut;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::contract::{
    BoundedInferenceRequest, BoxFuture, CancellationToken, ChatMessage, DiscoveredModel,
    DiscoveredModelMetadata, InferenceFinishReason, InferenceMetrics, InferenceProvider,
    InferenceResponse, InferenceStream, LifecycleCapabilityState, ModelLifecycleOutcome,
    ProviderHealthReport, ProviderHealthStatus, ProviderKind, ProviderLifecycleSupport,
    ProviderTelemetrySnapshot, StreamChunk,
};
use crate::core_bridge::ProviderId;
use crate::error::ProviderError;
use crate::policy::{EndpointPolicy, SecretRef, SecretString, ValidatedEndpoint};

/// Maximum size in bytes of a single SSE line frame (256 KiB).
const MAX_SSE_LINE_BYTES: usize = 256 * 1024;

/// Configuration for initializing an [`OpenAiCompatibleProvider`].
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// Unique VESPERA provider identifier.
    pub provider_id: ProviderId,
    /// Validated endpoint base URL (e.g., `http://127.0.0.1:8080` or `http://127.0.0.1:1234/v1`).
    pub endpoint: ValidatedEndpoint,
    /// Optional environment variable reference for bearer authentication.
    pub auth_secret_ref: Option<SecretRef>,
    /// Optional pre-resolved secret for testing.
    pub pre_resolved_secret: Option<SecretString>,
    /// Connection establishment timeout.
    pub connect_timeout: Duration,
    /// Total request timeout for non-streaming calls.
    pub request_timeout: Duration,
    /// Maximum idle gap between consecutive SSE chunks before aborting with `Timeout`.
    pub stream_idle_timeout: Duration,
    /// Total lifetime limit for streaming transport, including connection and consumption.
    pub stream_total_timeout: Duration,
}

impl OpenAiConfig {
    /// Constructs a loopback OpenAI-compatible configuration with conservative timeouts.
    pub fn loopback_default(provider_id: ProviderId, raw_url: &str) -> Result<Self, ProviderError> {
        let endpoint = ValidatedEndpoint::parse(provider_id, raw_url, EndpointPolicy::default())?;
        Ok(Self {
            provider_id,
            endpoint,
            auth_secret_ref: None,
            pre_resolved_secret: None,
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(60),
            stream_idle_timeout: Duration::from_secs(30),
            stream_total_timeout: Duration::from_secs(300),
        })
    }
}

/// OpenAI-compatible adapter implementing [`InferenceProvider`].
pub struct OpenAiCompatibleProvider {
    provider_id: ProviderId,
    endpoint: ValidatedEndpoint,
    client: reqwest::Client,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    stream_total_timeout: Duration,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("provider_id", &self.provider_id)
            .field("endpoint", &self.endpoint)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    /// Initializes a new [`OpenAiCompatibleProvider`].
    pub fn new(config: OpenAiConfig) -> Result<Self, ProviderError> {
        crate::http::validate_timeouts(&[
            config.connect_timeout,
            config.request_timeout,
            config.stream_idle_timeout,
            config.stream_total_timeout,
        ])?;
        let mut default_headers = HeaderMap::new();
        default_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let resolved_secret = match (config.pre_resolved_secret, config.auth_secret_ref) {
            (Some(secret), _) => Some(secret),
            (None, Some(secret_ref)) => Some(secret_ref.resolve(config.provider_id)?),
            (None, None) => None,
        };

        if let Some(secret) = resolved_secret {
            let formatted = format!("Bearer {}", secret.expose_secret());
            let mut header_val = HeaderValue::from_str(&formatted).map_err(|_| {
                ProviderError::SecretResolutionFailed {
                    provider_id: config.provider_id,
                    env_var: "<resolved>".to_string(),
                    reason: "secret contains invalid HTTP header bytes".to_string(),
                }
            })?;
            header_val.set_sensitive(true);
            default_headers.insert(AUTHORIZATION, header_val);
        }

        let client = config
            .endpoint
            .configure_client(reqwest::Client::builder())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .default_headers(default_headers)
            .build()
            .map_err(|e| ProviderError::ConnectionFailed {
                provider_id: config.provider_id,
                sanitized_message: ProviderError::sanitize_diagnostic(&e.to_string()),
            })?;

        Ok(Self {
            provider_id: config.provider_id,
            endpoint: config.endpoint,
            client,
            request_timeout: config.request_timeout,
            stream_idle_timeout: config.stream_idle_timeout,
            stream_total_timeout: config.stream_total_timeout,
        })
    }

    async fn execute_json_request<Resp: for<'de> Deserialize<'de>>(
        &self,
        request_builder: reqwest::RequestBuilder,
        phase: &'static str,
        context: &'static str,
        model_hint: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<(Resp, u64), ProviderError> {
        crate::http::execute_json(
            self.provider_id,
            request_builder,
            self.request_timeout,
            phase,
            context,
            model_hint,
            cancel,
        )
        .await
    }
    fn map_reqwest_error(
        &self,
        err: &reqwest::Error,
        phase: &'static str,
        started: Instant,
    ) -> ProviderError {
        if err.is_timeout() {
            return ProviderError::Timeout {
                provider_id: self.provider_id,
                phase,
                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            };
        }
        if err.is_redirect() {
            return ProviderError::TransportPolicyViolation {
                provider_id: self.provider_id,
                reason: "HTTP redirect blocked by transport policy".to_string(),
            };
        }
        ProviderError::ConnectionFailed {
            provider_id: self.provider_id,
            sanitized_message: ProviderError::sanitize_diagnostic(&err.to_string()),
        }
    }
}

impl InferenceProvider for OpenAiCompatibleProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn lifecycle_support(&self) -> ProviderLifecycleSupport {
        ProviderLifecycleSupport {
            model_enumeration: LifecycleCapabilityState::Supported,
            model_metadata: LifecycleCapabilityState::Unsupported,
            explicit_model_load: LifecycleCapabilityState::Unsupported,
            explicit_model_unload: LifecycleCapabilityState::Unsupported,
            residency_telemetry: LifecycleCapabilityState::Unsupported,
        }
    }

    fn health_check<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderHealthReport, ProviderError>> {
        Box::pin(async move {
            let url = self
                .endpoint
                .join_versioned_api_path(self.provider_id, "v1", "models")?;
            let req = self.client.get(url);
            let (_, latency_ms): (OpenAiModelsResponse, u64) = self
                .execute_json_request(req, "health_check", "v1/models", None, &cancel)
                .await?;

            Ok(ProviderHealthReport {
                provider_id: self.provider_id,
                status: ProviderHealthStatus::Healthy,
                version: Some("openai-v1".to_string()),
                latency_ms,
            })
        })
    }

    fn enumerate_models<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<Vec<DiscoveredModel>, ProviderError>> {
        Box::pin(async move {
            let url = self
                .endpoint
                .join_versioned_api_path(self.provider_id, "v1", "models")?;
            let req = self.client.get(url);
            let (resp, _): (OpenAiModelsResponse, u64) = self
                .execute_json_request(req, "enumerate_models", "v1/models", None, &cancel)
                .await?;

            let models = resp
                .data
                .into_iter()
                .map(|m| DiscoveredModel {
                    provider_model_name: m.id,
                    digest: None,
                    size_bytes: None,
                    parameter_size: None,
                    quantization_level: None,
                    family: None,
                })
                .collect();

            Ok(models)
        })
    }

    fn model_metadata<'a>(
        &'a self,
        _model_name: &'a str,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<DiscoveredModelMetadata, ProviderError>> {
        Box::pin(async move {
            Err(ProviderError::UnsupportedOperation {
                provider_id: self.provider_id,
                operation: "model_metadata",
            })
        })
    }

    fn query_residency<'a>(
        &'a self,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderTelemetrySnapshot, ProviderError>> {
        Box::pin(async move {
            Err(ProviderError::UnsupportedOperation {
                provider_id: self.provider_id,
                operation: "query_residency",
            })
        })
    }

    fn load_model<'a>(
        &'a self,
        _model_name: &'a str,
        _keep_alive: Option<Duration>,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>> {
        Box::pin(async move {
            Err(ProviderError::UnsupportedOperation {
                provider_id: self.provider_id,
                operation: "load_model",
            })
        })
    }

    fn unload_model<'a>(
        &'a self,
        _model_name: &'a str,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>> {
        Box::pin(async move {
            Err(ProviderError::UnsupportedOperation {
                provider_id: self.provider_id,
                operation: "unload_model",
            })
        })
    }

    fn infer<'a>(
        &'a self,
        request: BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InferenceResponse, ProviderError>> {
        Box::pin(async move {
            let url = self.endpoint.join_versioned_api_path(
                self.provider_id,
                "v1",
                "chat/completions",
            )?;
            let payload = OpenAiChatRequest::from_bounded(&request, false);
            let req = self.client.post(url).json(&payload);

            let (resp, elapsed_ms): (OpenAiChatResponse, u64) = self
                .execute_json_request(
                    req,
                    "infer",
                    "v1/chat/completions",
                    Some(request.model_name()),
                    &cancel,
                )
                .await?;

            let choice =
                resp.choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| ProviderError::MalformedPayload {
                        provider_id: self.provider_id,
                        context: "v1/chat/completions",
                        reason: "response contained zero choices".to_string(),
                    })?;

            let content = choice.message.content.unwrap_or_default();
            let finish_reason =
                map_finish_reason(choice.finish_reason.as_deref().unwrap_or("unknown"));
            let metrics = InferenceMetrics {
                client_elapsed_ms: elapsed_ms,
                prompt_tokens: resp.usage.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: resp.usage.as_ref().map(|u| u.completion_tokens),
                provider_load_duration_ms: None,
                provider_eval_duration_ms: None,
            };

            Ok(InferenceResponse {
                provider_id: self.provider_id,
                model_name: request.model_name().to_string(),
                content,
                finish_reason,
                metrics,
            })
        })
    }

    fn infer_stream<'a>(
        &'a self,
        request: BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InferenceStream<'a>, ProviderError>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ProviderError::Cancelled {
                    provider_id: self.provider_id,
                    phase: "infer_stream_init",
                });
            }

            let url = self.endpoint.join_versioned_api_path(
                self.provider_id,
                "v1",
                "chat/completions",
            )?;
            let payload = OpenAiChatRequest::from_bounded(&request, true);
            let req = self.client.post(url).json(&payload);

            let started = Instant::now();
            let response = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    return Err(ProviderError::Cancelled {
                        provider_id: self.provider_id,
                        phase: "infer_stream_connect",
                    });
                }
                res = tokio::time::timeout(self.request_timeout.min(self.stream_total_timeout), req.send()) => {
                    match res {
                        Ok(Ok(resp)) => resp,
                        Ok(Err(err)) => return Err(self.map_reqwest_error(&err, "infer_stream_connect", started)),
                        Err(_) => {
                            return Err(ProviderError::Timeout {
                                provider_id: self.provider_id,
                                phase: "infer_stream_connect",
                                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                            });
                        }
                    }
                }
            };

            crate::http::check_status(self.provider_id, &response, Some(request.model_name()))?;

            let byte_stream = response.bytes_stream();
            let provider_id = self.provider_id;
            let idle_timeout = self.stream_idle_timeout;

            let stream = futures_util::stream::unfold(
                SseStreamState {
                    byte_stream,
                    buffer: BytesMut::with_capacity(4096),
                    provider_id,
                    cancel,
                    idle_timeout,
                    started,
                    finished: false,
                    deadline: started + self.stream_total_timeout,
                    received_bytes: 0,
                    event_data: String::new(),
                    finish_reason: None,
                    usage: None,
                },
                |mut state| async move {
                    if state.finished {
                        return None;
                    }
                    match state.next_chunk().await {
                        Ok(Some(chunk)) => {
                            if chunk.done {
                                state.finished = true;
                            }
                            Some((Ok(chunk), state))
                        }
                        Ok(None) => None,
                        Err(err) => {
                            state.finished = true;
                            Some((Err(err), state))
                        }
                    }
                },
            );

            Ok(Box::pin(stream) as InferenceStream<'a>)
        })
    }
}

struct SseStreamState<S> {
    byte_stream: S,
    buffer: BytesMut,
    event_data: String,
    provider_id: ProviderId,
    cancel: CancellationToken,
    idle_timeout: Duration,
    started: Instant,
    finished: bool,
    deadline: Instant,
    received_bytes: usize,
    finish_reason: Option<InferenceFinishReason>,
    usage: Option<OpenAiUsage>,
}

impl<S> SseStreamState<S>
where
    S: futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    fn malformed(&self, reason: &'static str) -> ProviderError {
        ProviderError::MalformedPayload {
            provider_id: self.provider_id,
            context: "sse_stream",
            reason: reason.into(),
        }
    }
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, ProviderError> {
        loop {
            if Instant::now() >= self.deadline {
                return Err(ProviderError::Timeout {
                    provider_id: self.provider_id,
                    phase: "stream_total",
                    elapsed_ms: u64::try_from(self.started.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                });
            }
            if self.cancel.is_cancelled() {
                return Err(ProviderError::Cancelled {
                    provider_id: self.provider_id,
                    phase: "sse_stream_chunk",
                });
            }
            if let Some(newline_idx) = self.buffer.iter().position(|&b| b == b'\n') {
                if newline_idx > MAX_SSE_LINE_BYTES {
                    return Err(self.malformed("SSE line exceeded 256 KiB limit"));
                }
                let line_bytes = self.buffer.split_to(newline_idx + 1);
                let line = std::str::from_utf8(&line_bytes[..newline_idx])
                    .map_err(|_| self.malformed("invalid UTF-8 in SSE stream"))?;
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.is_empty() {
                    if !self.event_data.is_empty() {
                        let data = std::mem::take(&mut self.event_data);
                        if let Some(chunk) = self.parse_event(data.trim_end_matches('\n'))? {
                            return Ok(Some(chunk));
                        }
                    }
                } else if let Some(data) = line.strip_prefix("data:") {
                    let data = data.strip_prefix(' ').unwrap_or(data);
                    if self
                        .event_data
                        .len()
                        .saturating_add(data.len())
                        .saturating_add(1)
                        > MAX_SSE_LINE_BYTES
                    {
                        return Err(self.malformed("SSE event exceeded 256 KiB limit"));
                    }
                    self.event_data.push_str(data);
                    self.event_data.push('\n');
                }
                continue;
            }
            if self.buffer.len() > MAX_SSE_LINE_BYTES {
                return Err(self.malformed("SSE line exceeded 256 KiB limit"));
            }
            let chunk_start = Instant::now();
            let next_bytes = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Err(ProviderError::Cancelled { provider_id: self.provider_id, phase: "sse_stream_chunk" }),
                result = tokio::time::timeout_at(self.deadline.min(chunk_start + self.idle_timeout), self.byte_stream.next()) => match result {
                    Ok(Some(Ok(bytes))) => bytes,
                    Ok(Some(Err(error))) => return Err(ProviderError::ConnectionFailed { provider_id: self.provider_id, sanitized_message: ProviderError::sanitize_diagnostic(&error.to_string()) }),
                    Ok(None) => return Err(ProviderError::StreamInterrupted { provider_id: self.provider_id, reason: "SSE stream ended before [DONE] event".into() }),
                    Err(_) => return Err(ProviderError::Timeout { provider_id: self.provider_id, phase: if Instant::now() >= self.deadline { "stream_total" } else { "sse_stream_chunk" }, elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX) }),
                },
            };
            if next_bytes.len() > crate::http::MAX_STREAM_BYTES - self.received_bytes {
                return Err(self.malformed("stream exceeded sixteen MiB aggregate limit"));
            }
            self.received_bytes += next_bytes.len();
            self.buffer.extend_from_slice(&next_bytes);
        }
    }
    fn parse_event(&mut self, data: &str) -> Result<Option<StreamChunk>, ProviderError> {
        if data == "[DONE]" {
            return Ok(Some(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(self.finish_reason.unwrap_or(InferenceFinishReason::Unknown)),
                metrics: Some(InferenceMetrics {
                    client_elapsed_ms: u64::try_from(self.started.elapsed().as_millis())
                        .unwrap_or(u64::MAX),
                    prompt_tokens: self.usage.as_ref().map(|usage| usage.prompt_tokens),
                    completion_tokens: self.usage.as_ref().map(|usage| usage.completion_tokens),
                    provider_load_duration_ms: None,
                    provider_eval_duration_ms: None,
                }),
            }));
        }
        let frame: OpenAiStreamChunkFrame = serde_json::from_str(data)
            .map_err(|_| self.malformed("invalid chat completion event JSON"))?;
        if let Some(usage) = frame.usage {
            self.usage = Some(usage);
        }
        let Some(choice) = frame.choices.into_iter().next() else {
            return Ok(None);
        };
        let finish_reason = choice.finish_reason.as_deref().map(map_finish_reason);
        if finish_reason.is_some() {
            self.finish_reason = finish_reason;
        }
        Ok(Some(StreamChunk {
            delta: choice.delta.content.unwrap_or_default(),
            done: false,
            finish_reason,
            metrics: None,
        }))
    }
}
fn map_finish_reason(reason: &str) -> InferenceFinishReason {
    match reason {
        "stop" => InferenceFinishReason::Stop,
        "length" => InferenceFinishReason::Length,
        "tool_calls" => InferenceFinishReason::ToolCalls,
        _ => InferenceFinishReason::Unknown,
    }
}

// --- OpenAI wire protocol DTOs ---

#[derive(Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(Deserialize)]
struct OpenAiModelItem {
    id: String,
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

impl<'a> OpenAiChatRequest<'a> {
    fn from_bounded(req: &'a BoundedInferenceRequest, stream: bool) -> Self {
        Self {
            model: req.model_name(),
            messages: req.messages(),
            stream,
            max_tokens: Some(req.max_output_tokens()),
            temperature: req.temperature(),
        }
    }
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: crate::http::ResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAiStreamChunkFrame {
    #[serde(default)]
    usage: Option<OpenAiUsage>,
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Deserialize)]
struct OpenAiStreamChoice {
    #[serde(default)]
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
}
