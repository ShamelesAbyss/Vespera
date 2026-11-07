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
    /// Suppress untrusted diagnostic text instead of attempting incomplete token redaction.
    /// Typed variants retain provider, phase, HTTP status and error category. Arbitrary
    /// upstream messages may echo secrets without recognizable markers and are not logged.
    #[must_use]
    pub fn sanitize_diagnostic(_input: &str) -> String {
        "upstream diagnostic omitted to protect endpoint and credential data".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_diagnostics_are_suppressed_without_utf8_truncation_panics() {
        for input in [
            "Bearer fixture-token".to_string(),
            "https://user:fixture-password@example.invalid/private".to_string(),
            format!("{}é", "x".repeat(239)),
            "unmarked-fixture-credential".to_string(),
        ] {
            let result = ProviderError::sanitize_diagnostic(&input);
            assert!(!result.contains(&input));
            assert!(!result.contains("fixture"));
        }
    }
}
