//! Bounded, deadline-aware JSON transport shared by provider adapters.
use crate::core_bridge::ProviderId;
use crate::{CancellationToken, ProviderError};
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

pub(crate) const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;

/// Provider responses may contain extra fields; only authorized public text is retained.
#[derive(Deserialize)]
pub(crate) struct ResponseMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
}

pub(crate) fn validate_timeouts(timeouts: &[Duration]) -> Result<(), ProviderError> {
    if timeouts
        .iter()
        .any(|value| value.is_zero() || *value > Duration::from_secs(3600))
    {
        return Err(ProviderError::RequestValidation {
            reason: "timeouts must be positive and at most one hour".into(),
        });
    }
    Ok(())
}

pub(crate) async fn execute_json<Resp: for<'de> Deserialize<'de>>(
    provider_id: ProviderId,
    request: reqwest::RequestBuilder,
    timeout: Duration,
    phase: &'static str,
    context: &'static str,
    model_hint: Option<&str>,
    cancel: &CancellationToken,
) -> Result<(Resp, u64), ProviderError> {
    let started = Instant::now();
    let deadline =
        started
            .checked_add(timeout)
            .ok_or_else(|| ProviderError::RequestValidation {
                reason: "request deadline overflow".into(),
            })?;
    let work = async {
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                ProviderError::Timeout {
                    provider_id,
                    phase,
                    elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                }
            } else {
                ProviderError::ConnectionFailed {
                    provider_id,
                    sanitized_message: "HTTP request failed".into(),
                }
            }
        })?;
        check_status(provider_id, &response, model_hint)?;
        let malformed = || ProviderError::MalformedPayload {
            provider_id,
            context,
            reason: "JSON response exceeded eight MiB limit".into(),
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_BYTES as u64)
        {
            return Err(malformed());
        }
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| ProviderError::ConnectionFailed {
                provider_id,
                sanitized_message: "HTTP response body failed".into(),
            })?;
            if chunk.len() > MAX_JSON_BYTES - body.len() {
                return Err(malformed());
            }
            body.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice(&body).map_err(|_| ProviderError::MalformedPayload {
            provider_id,
            context,
            reason: "invalid provider JSON response".into(),
        })?;
        Ok((
            value,
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ))
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(ProviderError::Cancelled { provider_id, phase }),
        result = tokio::time::timeout_at(deadline, work) => result.unwrap_or_else(|_| Err(ProviderError::Timeout { provider_id, phase, elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX) })),
    }
}

pub(crate) fn check_status(
    provider_id: ProviderId,
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
            provider_id,
            reason: "HTTP redirects are forbidden".into(),
        });
    }
    if code == 401 || code == 403 {
        return Err(ProviderError::AuthenticationFailed {
            provider_id,
            status_code: code,
        });
    }
    if code == 404
        && let Some(model_name) = model_hint
    {
        return Err(ProviderError::ModelNotFound {
            provider_id,
            model_name: model_name.into(),
        });
    }
    Err(ProviderError::HttpStatus {
        provider_id,
        status_code: code,
        sanitized_summary: "unexpected HTTP status".into(),
    })
}
