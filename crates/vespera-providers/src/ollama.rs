//! Ollama provider adapter implementing the universal `InferenceProvider` contract.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use bytes::BytesMut;
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;

use crate::contract::{
    BoundedInferenceRequest, BoxFuture, CancellationToken, ChatMessage, DiscoveredModel,
    DiscoveredModelMetadata, InferenceFinishReason, InferenceMetrics, InferenceProvider,
    InferenceResponse, InferenceStream, LifecycleCapabilityState, ModelLifecycleOutcome,
    ModelResidencyRecord, ProviderHealthReport, ProviderHealthStatus, ProviderKind,
    ProviderLifecycleSupport, ProviderTelemetrySnapshot, StreamChunk,
};
use crate::core_bridge::{Capability, ProviderId, ResourceEstimate};
use crate::error::ProviderError;
use crate::policy::{EndpointPolicy, SecretRef, SecretString, ValidatedEndpoint};

/// Maximum size in bytes of a single NDJSON line frame during streaming (256 KiB).
const MAX_NDJSON_LINE_BYTES: usize = 256 * 1024;

/// Configuration for initializing an [`OllamaProvider`].
#[derive(Debug, Clone)]
pub struct OllamaConfig {
    /// Unique VESPERA provider identifier.
    pub provider_id: ProviderId,
    /// Validated endpoint URL (e.g., `http://127.0.0.1:11434`).
    pub endpoint: ValidatedEndpoint,
    /// Optional environment variable reference for bearer authentication behind a reverse proxy.
    pub auth_secret_ref: Option<SecretRef>,
    /// Optional pre-resolved secret (used for deterministic tests without mutating process env).
    pub pre_resolved_secret: Option<SecretString>,
    /// Connection establishment timeout.
    pub connect_timeout: Duration,
    /// Total request timeout for non-streaming calls and metadata queries.
    pub request_timeout: Duration,
    /// Maximum idle gap between consecutive streaming chunks before aborting with `Timeout`.
    pub stream_idle_timeout: Duration,
    /// Total lifetime limit for streaming transport, including connection and consumption.
    pub stream_total_timeout: Duration,
}

impl OllamaConfig {
    /// Constructs a loopback Ollama configuration with conservative timeouts.
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

/// Ollama adapter implementing [`InferenceProvider`].
pub struct OllamaProvider {
    provider_id: ProviderId,
    endpoint: ValidatedEndpoint,
    client: reqwest::Client,
    request_timeout: Duration,
    stream_idle_timeout: Duration,
    stream_total_timeout: Duration,
}

impl std::fmt::Debug for OllamaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OllamaProvider")
            .field("provider_id", &self.provider_id)
            .field("endpoint", &self.endpoint)
            .field("request_timeout", &self.request_timeout)
            .field("stream_idle_timeout", &self.stream_idle_timeout)
            .finish()
    }
}

impl OllamaProvider {
    /// Initializes a new [`OllamaProvider`], enforcing redirect prohibition and resolving any configured secret.
    pub fn new(config: OllamaConfig) -> Result<Self, ProviderError> {
        crate::http::validate_timeouts(&[
            config.connect_timeout,
            config.request_timeout,
            config.stream_idle_timeout,
            config.stream_total_timeout,
        ])?;
        let mut default_headers = HeaderMap::new();

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

        // Security invariant: redirects are strictly disabled to prevent SSRF and credential leakage.
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

impl InferenceProvider for OllamaProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    fn lifecycle_support(&self) -> ProviderLifecycleSupport {
        ProviderLifecycleSupport {
            model_enumeration: LifecycleCapabilityState::Supported,
            model_metadata: LifecycleCapabilityState::Supported,
            explicit_model_load: LifecycleCapabilityState::Supported,
            explicit_model_unload: LifecycleCapabilityState::Supported,
            residency_telemetry: LifecycleCapabilityState::Supported,
        }
    }

    fn health_check<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderHealthReport, ProviderError>> {
        Box::pin(async move {
            let url = self
                .endpoint
                .join_api_path(self.provider_id, "api/version")?;
            let req = self.client.get(url);
            let (resp, latency_ms): (OllamaVersionResponse, u64) = self
                .execute_json_request(req, "health_check", "api/version", None, &cancel)
                .await?;

            Ok(ProviderHealthReport {
                provider_id: self.provider_id,
                status: ProviderHealthStatus::Healthy,
                version: Some(resp.version),
                latency_ms,
            })
        })
    }

    fn enumerate_models<'a>(
        &'a self,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<Vec<DiscoveredModel>, ProviderError>> {
        Box::pin(async move {
            let url = self.endpoint.join_api_path(self.provider_id, "api/tags")?;
            let req = self.client.get(url);
            let (resp, _): (OllamaTagsResponse, u64) = self
                .execute_json_request(req, "enumerate_models", "api/tags", None, &cancel)
                .await?;

            let models = resp
                .models
                .into_iter()
                .map(|m| DiscoveredModel {
                    provider_model_name: m.name,
                    digest: m.digest,
                    size_bytes: m.size,
                    parameter_size: m.details.as_ref().and_then(|d| d.parameter_size.clone()),
                    quantization_level: m
                        .details
                        .as_ref()
                        .and_then(|d| d.quantization_level.clone()),
                    family: m.details.and_then(|d| d.family),
                })
                .collect();

            Ok(models)
        })
    }

    fn model_metadata<'a>(
        &'a self,
        model_name: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<DiscoveredModelMetadata, ProviderError>> {
        Box::pin(async move {
            if model_name.trim().is_empty() {
                return Err(ProviderError::RequestValidation {
                    reason: "model_name must not be empty".to_string(),
                });
            }
            let url = self.endpoint.join_api_path(self.provider_id, "api/show")?;
            let req = self
                .client
                .post(url)
                .json(&OllamaShowRequest { model: model_name });
            let (resp, _): (OllamaShowResponse, u64) = self
                .execute_json_request(req, "model_metadata", "api/show", Some(model_name), &cancel)
                .await?;

            let architecture = resp
                .details
                .as_ref()
                .and_then(|d| d.family.clone())
                .or_else(|| {
                    resp.model_info
                        .get("general.architecture")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                });

            let context_window_tokens = extract_context_length(&resp.model_info);

            let mut capabilities = BTreeSet::new();
            for raw_cap in resp.capabilities {
                let normalized = match raw_cap.as_str() {
                    "completion" => "text-generation",
                    "tools" => "tool-use",
                    "vision" => "vision",
                    "embedding" => "embeddings",
                    other => other,
                };
                if let Ok(cap) = normalized.parse::<Capability>() {
                    capabilities.insert(cap);
                }
            }

            Ok(DiscoveredModelMetadata {
                provider_model_name: model_name.to_string(),
                architecture,
                parameter_size: resp.details.as_ref().and_then(|d| d.parameter_size.clone()),
                quantization_level: resp
                    .details
                    .as_ref()
                    .and_then(|d| d.quantization_level.clone()),
                context_window_tokens,
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
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ProviderTelemetrySnapshot, ProviderError>> {
        Box::pin(async move {
            let url = self.endpoint.join_api_path(self.provider_id, "api/ps")?;
            let req = self.client.get(url);
            let (resp, _): (OllamaPsResponse, u64) = self
                .execute_json_request(req, "query_residency", "api/ps", None, &cancel)
                .await?;

            let mut resident_models = Vec::with_capacity(resp.models.len());
            let mut total_vram: u64 = 0;
            let mut total_ram: u64 = 0;

            for entry in resp.models {
                let total_bytes = entry.size;
                let vram_bytes = entry.size_vram;
                let invalid = || ProviderError::MalformedPayload {
                    provider_id: self.provider_id,
                    context: "api/ps",
                    reason: "inconsistent or overflowing residency telemetry".into(),
                };
                let ram_bytes = total_bytes.checked_sub(vram_bytes).ok_or_else(invalid)?;
                total_vram = total_vram.checked_add(vram_bytes).ok_or_else(invalid)?;
                total_ram = total_ram.checked_add(ram_bytes).ok_or_else(invalid)?;

                resident_models.push(ModelResidencyRecord {
                    provider_model_name: entry.name,
                    total_resident_bytes: total_bytes,
                    vram_resident_bytes: vram_bytes,
                    ram_resident_bytes: ram_bytes,
                    allocated_context_tokens: entry.context_length,
                });
            }

            Ok(ProviderTelemetrySnapshot {
                provider_id: self.provider_id,
                resident_models,
                total_provider_vram_bytes: total_vram,
                total_provider_ram_bytes: total_ram,
            })
        })
    }

    fn load_model<'a>(
        &'a self,
        model_name: &'a str,
        keep_alive: Option<Duration>,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>> {
        Box::pin(async move {
            let secs = keep_alive.map_or(300, |d| d.as_secs().max(1));
            let keep_alive_str = format!("{secs}s");
            self.execute_lifecycle_generate(
                model_name,
                Value::String(keep_alive_str),
                None,
                &cancel,
            )
            .await
        })
    }

    fn load_model_with_context<'a>(
        &'a self,
        model_name: &'a str,
        context_tokens: u32,
        keep_alive: Option<Duration>,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>> {
        Box::pin(async move {
            if !(64..=1_048_576).contains(&context_tokens) {
                return Err(ProviderError::RequestValidation {
                    reason: "context must be between 64 and 1048576 tokens".into(),
                });
            }
            let secs = keep_alive.map_or(300, |d| d.as_secs().max(1));
            self.execute_lifecycle_generate(
                model_name,
                Value::String(format!("{secs}s")),
                Some(context_tokens),
                &cancel,
            )
            .await
        })
    }

    fn unload_model<'a>(
        &'a self,
        model_name: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ModelLifecycleOutcome, ProviderError>> {
        Box::pin(async move {
            self.execute_lifecycle_generate(model_name, Value::Number(0.into()), None, &cancel)
                .await
        })
    }

    fn infer<'a>(
        &'a self,
        request: BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<InferenceResponse, ProviderError>> {
        Box::pin(async move {
            let url = self.endpoint.join_api_path(self.provider_id, "api/chat")?;
            let payload = OllamaChatRequest::from_bounded(&request, false);
            let req = self.client.post(url).json(&payload);

            let (resp, elapsed_ms): (OllamaChatFrame, u64) = self
                .execute_json_request(
                    req,
                    "infer",
                    "api/chat",
                    Some(request.model_name()),
                    &cancel,
                )
                .await?;

            if let Some(err_msg) = resp.error {
                return Err(ProviderError::HttpStatus {
                    provider_id: self.provider_id,
                    status_code: 500,
                    sanitized_summary: ProviderError::sanitize_diagnostic(&err_msg),
                });
            }

            if !resp.done {
                return Err(ProviderError::StreamInterrupted {
                    provider_id: self.provider_id,
                    reason: "non-streaming response was not terminal".into(),
                });
            }

            let finish_reason = map_done_reason(resp.done_reason.as_deref());
            let metrics = build_metrics(elapsed_ms, &resp);
            let content = resp.message.and_then(|m| m.content).unwrap_or_default();

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

            let url = self.endpoint.join_api_path(self.provider_id, "api/chat")?;
            let payload = OllamaChatRequest::from_bounded(&request, true);
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
                NdjsonStreamState {
                    byte_stream,
                    buffer: BytesMut::with_capacity(4096),
                    provider_id,
                    cancel,
                    idle_timeout,
                    started,
                    finished: false,
                    deadline: started + self.stream_total_timeout,
                    received_bytes: 0,
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

impl OllamaProvider {
    async fn execute_lifecycle_generate(
        &self,
        model_name: &str,
        keep_alive: Value,
        context_tokens: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<ModelLifecycleOutcome, ProviderError> {
        if model_name.trim().is_empty() {
            return Err(ProviderError::RequestValidation {
                reason: "model_name must not be empty".to_string(),
            });
        }
        let url = self
            .endpoint
            .join_api_path(self.provider_id, "api/generate")?;
        let req = self.client.post(url).json(&OllamaLifecycleRequest {
            model: model_name,
            prompt: "",
            stream: false,
            keep_alive,
            options: context_tokens.map(|tokens| serde_json::json!({ "num_ctx": tokens })),
        });

        let (resp, elapsed_ms): (OllamaLifecycleResponse, u64) = self
            .execute_json_request(
                req,
                "model_lifecycle",
                "api/generate",
                Some(model_name),
                cancel,
            )
            .await?;

        Ok(ModelLifecycleOutcome {
            provider_id: self.provider_id,
            provider_model_name: model_name.to_string(),
            acknowledged: resp.done,
            elapsed_ms,
        })
    }
}

struct NdjsonStreamState<S> {
    byte_stream: S,
    buffer: BytesMut,
    provider_id: ProviderId,
    cancel: CancellationToken,
    idle_timeout: Duration,
    started: Instant,
    finished: bool,
    deadline: Instant,
    received_bytes: usize,
}

impl<S> NdjsonStreamState<S>
where
    S: futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
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
                    phase: "stream_chunk",
                });
            }

            if let Some(newline_idx) = self.buffer.iter().position(|&b| b == b'\n') {
                if newline_idx > MAX_NDJSON_LINE_BYTES {
                    return Err(ProviderError::MalformedPayload {
                        provider_id: self.provider_id,
                        context: "ndjson_stream",
                        reason: "stream frame exceeded 256 KiB line ceiling".into(),
                    });
                }
                let line_bytes = self.buffer.split_to(newline_idx + 1);
                let trimmed = trim_ascii_whitespace(&line_bytes);
                if trimmed.is_empty() {
                    continue;
                }
                return self.parse_frame(trimmed).map(Some);
            }

            if self.buffer.len() > MAX_NDJSON_LINE_BYTES {
                return Err(ProviderError::MalformedPayload {
                    provider_id: self.provider_id,
                    context: "ndjson_stream",
                    reason: "stream frame exceeded 256 KiB line ceiling".to_string(),
                });
            }

            let chunk_start = Instant::now();
            let next_bytes = tokio::select! {
                biased;
                () = self.cancel.cancelled() => {
                    return Err(ProviderError::Cancelled {
                        provider_id: self.provider_id,
                        phase: "stream_chunk",
                    });
                }
                res = tokio::time::timeout_at(self.deadline.min(chunk_start + self.idle_timeout), self.byte_stream.next()) => {
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
                                phase: if Instant::now() >= self.deadline { "stream_total" } else { "stream_chunk" },
                                elapsed_ms: u64::try_from(chunk_start.elapsed().as_millis()).unwrap_or(u64::MAX),
                            });
                        }
                    }
                }
            };

            match next_bytes {
                Some(bytes) => {
                    if bytes.len() > crate::http::MAX_STREAM_BYTES - self.received_bytes {
                        return Err(ProviderError::MalformedPayload {
                            provider_id: self.provider_id,
                            context: "ndjson_stream",
                            reason: "stream exceeded sixteen MiB aggregate limit".into(),
                        });
                    }
                    self.received_bytes += bytes.len();
                    self.buffer.extend_from_slice(&bytes);
                }
                None => {
                    let remaining = trim_ascii_whitespace(&self.buffer);
                    if !remaining.is_empty() {
                        let owned = remaining.to_vec();
                        self.buffer.clear();
                        let chunk = self.parse_frame(&owned)?;
                        if !chunk.done {
                            return Err(ProviderError::StreamInterrupted {
                                provider_id: self.provider_id,
                                reason: "EOF reached after partial non-terminal frame".to_string(),
                            });
                        }
                        return Ok(Some(chunk));
                    }
                    return Err(ProviderError::StreamInterrupted {
                        provider_id: self.provider_id,
                        reason: "stream closed by peer before terminal 'done: true' frame"
                            .to_string(),
                    });
                }
            }
        }
    }

    fn parse_frame(&self, frame_bytes: &[u8]) -> Result<StreamChunk, ProviderError> {
        let frame: OllamaChatFrame =
            serde_json::from_slice(frame_bytes).map_err(|e| ProviderError::MalformedPayload {
                provider_id: self.provider_id,
                context: "ndjson_stream",
                reason: ProviderError::sanitize_diagnostic(&e.to_string()),
            })?;

        if let Some(err_msg) = frame.error {
            return Err(ProviderError::HttpStatus {
                provider_id: self.provider_id,
                status_code: 500,
                sanitized_summary: ProviderError::sanitize_diagnostic(&err_msg),
            });
        }

        let finish_reason = if frame.done {
            Some(map_done_reason(frame.done_reason.as_deref()))
        } else {
            None
        };
        let metrics = if frame.done {
            let elapsed_ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
            Some(build_metrics(elapsed_ms, &frame))
        } else {
            None
        };
        let delta = frame.message.and_then(|m| m.content).unwrap_or_default();

        Ok(StreamChunk {
            delta,
            done: frame.done,
            finish_reason,
            metrics,
        })
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

fn extract_context_length(model_info: &BTreeMap<String, Value>) -> Option<u32> {
    for (k, v) in model_info {
        if k.ends_with(".context_length")
            && let Some(n) = v.as_u64()
            && let Ok(tokens) = u32::try_from(n)
        {
            return Some(tokens);
        }
    }
    None
}

fn map_done_reason(reason: Option<&str>) -> InferenceFinishReason {
    match reason {
        Some("stop") => InferenceFinishReason::Stop,
        Some("length") => InferenceFinishReason::Length,
        Some("tool_calls") => InferenceFinishReason::ToolCalls,
        Some("unload" | "load") => InferenceFinishReason::Unload,
        _ => InferenceFinishReason::Unknown,
    }
}

fn build_metrics(client_elapsed_ms: u64, frame: &OllamaChatFrame) -> InferenceMetrics {
    InferenceMetrics {
        client_elapsed_ms,
        prompt_tokens: frame.prompt_eval_count,
        completion_tokens: frame.eval_count,
        provider_load_duration_ms: frame.load_duration.map(|ns| ns / 1_000_000),
        provider_eval_duration_ms: frame.eval_duration.map(|ns| ns / 1_000_000),
    }
}

// --- Ollama wire protocol DTOs ---

#[derive(Deserialize)]
struct OllamaVersionResponse {
    version: String,
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaTagModel>,
}

#[derive(Deserialize)]
struct OllamaTagModel {
    name: String,
    digest: Option<String>,
    size: Option<u64>,
    details: Option<OllamaModelDetails>,
}

#[derive(Deserialize)]
struct OllamaModelDetails {
    family: Option<String>,
    parameter_size: Option<String>,
    quantization_level: Option<String>,
}

#[derive(Serialize)]
struct OllamaShowRequest<'a> {
    model: &'a str,
}

#[derive(Deserialize)]
struct OllamaShowResponse {
    details: Option<OllamaModelDetails>,
    #[serde(default)]
    model_info: BTreeMap<String, Value>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Deserialize)]
struct OllamaPsResponse {
    models: Vec<OllamaPsModel>,
}

#[derive(Deserialize)]
struct OllamaPsModel {
    name: String,
    size: u64,
    #[serde(default)]
    size_vram: u64,
    context_length: Option<u32>,
}

#[derive(Serialize)]
struct OllamaLifecycleRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    keep_alive: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<Value>,
}

#[derive(Deserialize)]
struct OllamaLifecycleResponse {
    #[serde(default)]
    done: bool,
}

#[derive(Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    options: OllamaChatOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<String>,
}

#[derive(Serialize)]
struct OllamaChatOptions {
    num_predict: u32,
    num_ctx: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

impl<'a> OllamaChatRequest<'a> {
    fn from_bounded(req: &'a BoundedInferenceRequest, stream: bool) -> Self {
        Self {
            model: req.model_name(),
            messages: req.messages(),
            stream,
            options: OllamaChatOptions {
                num_predict: req.max_output_tokens(),
                num_ctx: req.context_window_limit(),
                temperature: req.temperature(),
            },
            keep_alive: req.keep_alive_secs().map(|s| format!("{s}s")),
        }
    }
}

#[derive(Deserialize)]
struct OllamaChatFrame {
    #[serde(default)]
    message: Option<crate::http::ResponseMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    load_duration: Option<u64>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    eval_duration: Option<u64>,
}

#[cfg(test)]
mod stream_limits {
    use super::*;
    #[tokio::test]
    async fn complete_oversized_ndjson_line_is_rejected_before_parsing() {
        let body = format!(
            "{{\"message\":{{\"role\":\"assistant\",\"content\":\"{}\"}},\"done\":true}}\n",
            "x".repeat(MAX_NDJSON_LINE_BYTES)
        );
        let byte_stream =
            futures_util::stream::iter([Ok::<_, reqwest::Error>(bytes::Bytes::from(body))]);
        let mut state = NdjsonStreamState {
            byte_stream,
            buffer: BytesMut::new(),
            provider_id: ProviderId::new(),
            cancel: CancellationToken::new(),
            idle_timeout: Duration::from_secs(1),
            started: Instant::now(),
            finished: false,
            deadline: Instant::now() + Duration::from_secs(1),
            received_bytes: 0,
        };
        assert!(matches!(
            state.next_chunk().await,
            Err(ProviderError::MalformedPayload { .. })
        ));
    }
}
