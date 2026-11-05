//! Provider error taxonomy with strict credential and payload redaction.

use crate::core_bridge::ProviderId;
use thiserror::Error;

/// Standardized error taxonomy for all VESPERA inference providers.
///
/// Security invariant: Variants never embed raw secret tokens, authorization headers,
/// or unredacted sensitive URLs.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Endpoint violated loopback, scheme, credential, or redirect security policies.
    #[error("provider {provider_id} transport policy violation: {reason}")]
    TransportPolicyViolation {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Sanitized explanation of the policy failure.
        reason: String,
    },

    /// Configured environment variable for authentication could not be resolved.
    #[error("provider {provider_id} secret resolution failed for env var '{env_var}': {reason}")]
    SecretResolutionFailed {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Name of the environment variable (never its value).
        env_var: String,
        /// Reason resolution failed.
        reason: String,
    },

    /// Provider rejected credentials or blocked access (HTTP 401/403).
    #[error("provider {provider_id} authentication or authorization failed (status {status_code})")]
    AuthenticationFailed {
        /// Provider identifier.
        provider_id: ProviderId,
        /// HTTP status code.
        status_code: u16,
    },

    /// Request failed validation before reaching the transport layer.
    #[error("invalid inference request: {reason}")]
    RequestValidation {
        /// Validation failure description.
        reason: String,
    },

    /// Requested model does not exist on the target provider.
    #[error("provider {provider_id} reported model '{model_name}' not found")]
    ModelNotFound {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Provider-specific model identifier.
        model_name: String,
    },

    /// Provider does not support the requested capability or lifecycle operation.
    #[error("provider {provider_id} does not support operation: {operation}")]
    UnsupportedOperation {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Name of the unsupported operation.
        operation: &'static str,
    },

    /// Network connection failed before a response could be established.
    #[error("provider {provider_id} connection failed: {sanitized_message}")]
    ConnectionFailed {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Sanitized transport error description.
        sanitized_message: String,
    },

    /// Operation exceeded its monotonic deadline.
    ///
    /// Architectural note: A local timeout does not prove that remote inference computation stopped.
    #[error(
        "provider {provider_id} timed out after {elapsed_ms}ms during {phase} (remote execution state unverified)"
    )]
    Timeout {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Phase where timeout occurred (`connect`, `request`, or `stream_chunk`).
        phase: &'static str,
        /// Monotonic elapsed milliseconds.
        elapsed_ms: u64,
    },

    /// Operation was cooperatively cancelled by the caller.
    ///
    /// Architectural note: Cancellation aborts local stream consumption and drops the socket,
    /// which may not immediately halt remote provider GPU kernels.
    #[error("provider {provider_id} operation cancelled during {phase}")]
    Cancelled {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Phase where cancellation was triggered.
        phase: &'static str,
    },

    /// Provider returned an unexpected HTTP status code.
    #[error("provider {provider_id} returned HTTP {status_code}: {sanitized_summary}")]
    HttpStatus {
        /// Provider identifier.
        provider_id: ProviderId,
        /// HTTP status code.
        status_code: u16,
        /// Truncated, sanitized summary of the provider error message.
        sanitized_summary: String,
    },

    /// Provider returned malformed JSON, invalid UTF-8, or an oversized stream frame.
    #[error("provider {provider_id} returned malformed payload during {context}: {reason}")]
    MalformedPayload {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Operation context.
        context: &'static str,
        /// Diagnostic reason.
        reason: String,
    },

    /// Streaming response terminated prematurely without a completion frame.
    #[error("provider {provider_id} stream interrupted before completion: {reason}")]
    StreamInterrupted {
        /// Provider identifier.
        provider_id: ProviderId,
        /// Reason for interruption.
        reason: String,
    },
}

impl ProviderError {
    /// Sanitizes an arbitrary error string to ensure URLs, userinfo, and bearer tokens are stripped.
    #[must_use]
    pub fn sanitize_diagnostic(input: &str) -> String {
        const MAX_LEN: usize = 240;
        let mut cleaned = String::with_capacity(input.len().min(MAX_LEN));
        for token in input.split_whitespace() {
            if !cleaned.is_empty() {
                cleaned.push(' ');
            }
            if token.starts_with("http://")
                || token.starts_with("https://")
                || token.to_ascii_lowercase().starts_with("bearer")
                || token.contains('@')
            {
                cleaned.push_str("[REDACTED]");
            } else {
                cleaned.push_str(token);
            }
            if cleaned.len() >= MAX_LEN {
                cleaned.truncate(MAX_LEN);
                cleaned.push_str("...");
                break;
            }
        }
        cleaned
    }
}
