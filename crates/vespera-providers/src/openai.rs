//! Production OpenAI-compatible provider adapter for VESPERA.
//!
//! Supports local engines exposing standard OpenAI v1 endpoints:
//! - `GET /v1/models` (Model enumeration)
//! - `POST /v1/chat/completions` (Non-streaming and SSE streaming chat generation)

use std::collections::BTreeSet;
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
use crate::core_bridge::{Capability, ProviderId, ResourceEstimate};
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

        let client = reqwest::Client::builder()
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
        if cancel.is_cancelled() {
            return Err(ProviderError::Cancelled {
                provider_id: self.provider_id,
                phase,
            });
        }

        let started = Instant::now();
        let timeout_dur = self.request_timeout;

        let response = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Err(ProviderError::Cancelled {
                    provider_id: self.provider_id,
                    phase,
                });
            }
            res = tokio::time::timeout(timeout_dur, request_builder.send()) => {
                match res {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(err)) => return Err(self.map_reqwest_error(&err, phase, started)),
                    Err(_) => {
                        return Err(ProviderError::Timeout {
                            provider_id: self.provider_id,
                            phase,
                            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        });
                    }
                }
            }
        };

        self.check_http_status(&response, model_hint).await?;

        let body_bytes = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                return Err(ProviderError::Cancelled {
                    provider_id: self.provider_id,
                    phase,
                });
            }
            res = tokio::time::timeout(timeout_dur, response.bytes()) => {
                match res {
                    Ok(Ok(bytes)) => bytes,
                    Ok(Err(err)) => return Err(self.map_reqwest_error(&err, phase, started)),
                    Err(_) => {
                        return Err(ProviderError::Timeout {
                            provider_id: self.provider_id,
                            phase,
                            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                        });
                    }
                }
            }
        };

        let parsed: Resp =
            serde_json::from_slice(&body_bytes).map_err(|e| ProviderError::MalformedPayload {
                provider_id: self.provider_id,
                context,
                reason: ProviderError::sanitize_diagnostic(&e.to_string()),
            })?;

        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok((parsed, elapsed_ms))
    }

    async fn check_http_status(
        &self,
        response: &reqwest::Response,
        model_hint: Option<&str>,
    ) -> Result<(), ProviderError> {
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }

        let code = status.as_u16();
        if status.is_redirection() {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id: self.provider_id,
                reason: format!("HTTP redirect (status {code}) is forbidden by transport policy"),
            });
        }

        if code == 401 || code == 403 {
            return Err(ProviderError::AuthenticationFailed {
                provider_id: self.provider_id,
                status_code: code,
            });
        }

        if code == 404
            && let Some(model_name) = model_hint
        {
            return Err(ProviderError::ModelNotFound {
                provider_id: self.provider_id,
                model_name: model_name.to_string(),
            });
        }

        Err(ProviderError::HttpStatus {
            provider_id: self.provider_id,
            status_code: code,
            sanitized_summary: format!("unexpected HTTP status {code}"),
        })
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
            model_metadata: LifecycleCapabilityState::Unknown,
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
            let url = self.endpoint.join_api_path(self.provider_id, "v1/models")?;
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
            let url = self.endpoint.join_api_path(self.provider_id, "v1/models")?;
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
        model_name: &'a str,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<DiscoveredModelMetadata, ProviderError>> {
        Box::pin(async move {
            if model_name.trim().is_empty() {
                return Err(ProviderError::RequestValidation {
                    reason: "model_name must not be empty".to_string(),
                });
            }
            let mut capabilities = BTreeSet::new();
            if let Ok(cap) = "text-generation".parse::<Capability>() {
                capabilities.insert(cap);
            }

            Ok(DiscoveredModelMetadata {
                provider_model_name: model_name.to_string(),
                architecture: None,
                parameter_size: None,
                quantization_level: None,
                context_window_tokens: None,
                capabilities,
                estimated_resources: ResourceEstimate {
                    ram_bytes: None,
                    vram_bytes: None,
                },
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
            let url = self
                .endpoint
                .join_api_path(self.provider_id, "v1/chat/completions")?;
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

            let content = choice.message.content;
            let finish_reason =
                map_finish_reason(choice.finish_reason.as_deref().unwrap_or("stop"));
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

            let url = self
                .endpoint
                .join_api_path(self.provider_id, "v1/chat/completions")?;
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
                res = tokio::time::timeout(self.request_timeout, req.send()) => {
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

            self.check_http_status(&response, Some(request.model_name()))
                .await?;

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
    provider_id: ProviderId,
    cancel: CancellationToken,
    idle_timeout: Duration,
    started: Instant,
    finished: bool,
}

impl<S> SseStreamState<S>
where
    S: futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, ProviderError> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(ProviderError::Cancelled {
                    provider_id: self.provider_id,
                    phase: "sse_stream_chunk",
                });
            }

            if let Some(newline_idx) = self.buffer.iter().position(|&b| b == b'\n') {
                let line_bytes = self.buffer.split_to(newline_idx + 1);
                let trimmed = trim_ascii_whitespace(&line_bytes);
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(chunk) = self.parse_sse_line(trimmed)? {
                    return Ok(Some(chunk));
                }
                continue;
            }

            if self.buffer.len() > MAX_SSE_LINE_BYTES {
                return Err(ProviderError::MalformedPayload {
                    provider_id: self.provider_id,
                    context: "sse_stream",
                    reason: "SSE line exceeded 256 KiB limit".to_string(),
                });
            }

            let chunk_start = Instant::now();
            let next_bytes = tokio::select! {
                biased;
                () = self.cancel.cancelled() => {
                    return Err(ProviderError::Cancelled {
                        provider_id: self.provider_id,
                        phase: "sse_stream_chunk",
                    });
                }
                res = tokio::time::timeout(self.idle_timeout, self.byte_stream.next()) => {
                    match res {
                        Ok(Some(Ok(bytes))) => Some(bytes),
                        Ok(Some(Err(err))) => {
                            return Err(ProviderError::ConnectionFailed {
                                provider_id: self.provider_id,
                                sanitized_message: ProviderError::sanitize_diagnostic(&err.to_string()),
                            });
                        }
                        Ok(None) => None,
                        Err(_) => {
                            return Err(ProviderError::Timeout {
                                provider_id: self.provider_id,
                                phase: "sse_stream_chunk",
                                elapsed_ms: u64::try_from(chunk_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                            });
                        }
                    }
                }
            };

            match next_bytes {
                Some(bytes) => {
                    self.buffer.extend_from_slice(&bytes);
                }
                None => {
                    return Ok(Some(StreamChunk {
                        delta: String::new(),
                        done: true,
                        finish_reason: Some(InferenceFinishReason::Stop),
                        metrics: Some(InferenceMetrics {
                            client_elapsed_ms: u64::try_from(self.started.elapsed().as_millis())
                                .unwrap_or(u64::MAX),
                            prompt_tokens: None,
                            completion_tokens: None,
                            provider_load_duration_ms: None,
                            provider_eval_duration_ms: None,
                        }),
                    }));
                }
            }
        }
    }

    fn parse_sse_line(&self, line_bytes: &[u8]) -> Result<Option<StreamChunk>, ProviderError> {
        let line_str =
            std::str::from_utf8(line_bytes).map_err(|_| ProviderError::MalformedPayload {
                provider_id: self.provider_id,
                context: "sse_stream",
                reason: "invalid UTF-8 in SSE stream".to_string(),
            })?;

        let data_payload = if let Some(stripped) = line_str.strip_prefix("data:") {
            stripped.trim()
        } else {
            return Ok(None);
        };

        if data_payload == "[DONE]" {
            let elapsed_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            return Ok(Some(StreamChunk {
                delta: String::new(),
                done: true,
                finish_reason: Some(InferenceFinishReason::Stop),
                metrics: Some(InferenceMetrics {
                    client_elapsed_ms: elapsed_ms,
                    prompt_tokens: None,
                    completion_tokens: None,
                    provider_load_duration_ms: None,
                    provider_eval_duration_ms: None,
                }),
            }));
        }

        let chunk_frame: OpenAiStreamChunkFrame =
            serde_json::from_str(data_payload).map_err(|e| ProviderError::MalformedPayload {
                provider_id: self.provider_id,
                context: "sse_stream_json",
                reason: ProviderError::sanitize_diagnostic(&e.to_string()),
            })?;

        let choice = match chunk_frame.choices.into_iter().next() {
            Some(c) => c,
            None => return Ok(None),
        };

        let delta = choice.delta.content.unwrap_or_default();
        let finish_reason = choice.finish_reason.as_deref().map(map_finish_reason);
        let done = finish_reason.is_some()
            || (delta.is_empty() && finish_reason == Some(InferenceFinishReason::Stop));

        let metrics = if done {
            Some(InferenceMetrics {
                client_elapsed_ms: u64::try_from(self.started.elapsed().as_millis())
                    .unwrap_or(u64::MAX),
                prompt_tokens: None,
                completion_tokens: None,
                provider_load_duration_ms: None,
                provider_eval_duration_ms: None,
            })
        } else {
            None
        };

        Ok(Some(StreamChunk {
            delta,
            done,
            finish_reason,
            metrics,
        }))
    }
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |idx| idx + 1);
    &bytes[start..end]
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
    #[serde(default)]
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
    message: ChatMessage,
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
