//! Transport security policy, endpoint validation, and secret redaction primitives.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::Url;

use crate::core_bridge::ProviderId;
use crate::error::ProviderError;

/// Transport policy governing which network endpoints a provider adapter may contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointPolicy {
    /// Permit connections to non-loopback HTTPS hosts.
    pub allow_remote_https: bool,
    /// Permit connections to non-loopback plaintext HTTP hosts (requires `allow_remote_https` too).
    pub allow_insecure_remote_http: bool,
}

impl Default for EndpointPolicy {
    /// Defaults to strict loopback-only operation.
    fn default() -> Self {
        Self {
            allow_remote_https: false,
            allow_insecure_remote_http: false,
        }
    }
}

/// Validated, canonicalized provider base endpoint guaranteed to obey [`EndpointPolicy`].
#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedEndpoint {
    url: Url,
    is_loopback: bool,
}

impl fmt::Debug for ValidatedEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidatedEndpoint")
            .field("scheme", &self.url.scheme())
            .field("is_loopback", &self.is_loopback)
            .field("port", &self.url.port_or_known_default())
            .finish()
    }
}

impl ValidatedEndpoint {
    /// Parses and validates an endpoint string against the supplied [`EndpointPolicy`].
    ///
    /// Rejects non-HTTP(S) schemes, embedded credentials, query strings, fragments, zero ports,
    /// and remote hosts unless explicitly permitted by policy.
    pub fn parse(
        provider_id: ProviderId,
        raw_url: &str,
        policy: EndpointPolicy,
    ) -> Result<Self, ProviderError> {
        let parsed = Url::parse(raw_url).map_err(|_| ProviderError::TransportPolicyViolation {
            provider_id,
            reason: "malformed endpoint URL".to_string(),
        })?;

        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id,
                reason: "endpoint scheme must be 'http' or 'https'".to_string(),
            });
        }

        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id,
                reason: "embedded URL credentials are forbidden".to_string(),
            });
        }

        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id,
                reason: "endpoint URL must not contain query parameters or fragments".to_string(),
            });
        }

        if parsed.port() == Some(0) {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id,
                reason: "port 0 is forbidden".to_string(),
            });
        }

        let host_str =
            parsed
                .host_str()
                .ok_or_else(|| ProviderError::TransportPolicyViolation {
                    provider_id,
                    reason: "endpoint URL is missing a host".to_string(),
                })?;

        let is_loopback = is_loopback_host(host_str);

        if !is_loopback {
            if !policy.allow_remote_https {
                return Err(ProviderError::TransportPolicyViolation {
                    provider_id,
                    reason: "remote host rejected by loopback-only policy".to_string(),
                });
            }
            if scheme == "http" && !policy.allow_insecure_remote_http {
                return Err(ProviderError::TransportPolicyViolation {
                    provider_id,
                    reason:
                        "plaintext remote HTTP rejected without allow_insecure_remote_http opt-in"
                            .to_string(),
                });
            }
        }

        Ok(Self {
            url: parsed,
            is_loopback,
        })
    }

    /// Joins a relative API path onto the validated base endpoint without allowing origin escape.
    pub fn join_api_path(
        &self,
        provider_id: ProviderId,
        relative_path: &str,
    ) -> Result<Url, ProviderError> {
        let clean_relative = relative_path.trim_start_matches('/');
        let mut base = self.url.clone();
        if !base.path().ends_with('/') {
            let mut p = base.path().to_string();
            p.push('/');
            base.set_path(&p);
        }
        let joined =
            base.join(clean_relative)
                .map_err(|_| ProviderError::TransportPolicyViolation {
                    provider_id,
                    reason: "failed to join relative API path".to_string(),
                })?;

        if joined.origin() != self.url.origin() {
            return Err(ProviderError::TransportPolicyViolation {
                provider_id,
                reason: "joined API path attempted to escape base origin".to_string(),
            });
        }

        Ok(joined)
    }

    /// Join a versioned API route without duplicating a version suffix in the configured base.
    pub(crate) fn join_versioned_api_path(
        &self,
        provider_id: ProviderId,
        version: &str,
        path: &str,
    ) -> Result<Url, ProviderError> {
        let suffix = self.url.path().trim_end_matches('/').rsplit('/').next();
        if suffix == Some(version) {
            self.join_api_path(provider_id, path)
        } else {
            self.join_api_path(provider_id, &format!("{version}/{path}"))
        }
    }

    /// Returns whether this endpoint points to a loopback address.
    #[must_use]
    pub const fn is_loopback(&self) -> bool {
        self.is_loopback
    }

    /// Enforce direct transport and pin the only permitted loopback hostname.
    /// Literal loopback IPs need no DNS; remote DNS is allowed only by parse-time opt-in.
    pub(crate) fn configure_client(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> reqwest::ClientBuilder {
        let builder = builder.no_proxy();
        if self
            .url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("localhost"))
        {
            builder.resolve_to_addrs(
                "localhost",
                &[
                    SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                    SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
                ],
            )
        } else {
            builder
        }
    }
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let trimmed = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return ip.is_loopback();
    }
    false
}

/// Environment-variable reference for provider credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    env_var_name: String,
}

impl SecretRef {
    /// Validates an environment reference (`[A-Za-z_][A-Za-z0-9_]*`), preserving case.
    pub fn new(
        provider_id: ProviderId,
        env_var_name: impl Into<String>,
    ) -> Result<Self, ProviderError> {
        let name = env_var_name.into();
        let valid = !name.is_empty()
            && name.len() <= 128
            && name.bytes().enumerate().all(|(i, b)| {
                b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit())
            });

        if !valid {
            return Err(ProviderError::SecretResolutionFailed {
                provider_id,
                env_var: "<invalid-identifier>".to_string(),
                reason: "environment variable name must match [A-Za-z_][A-Za-z0-9_]*".to_string(),
            });
        }
        Ok(Self { env_var_name: name })
    }

    /// Resolves the secret value from the process environment at connection initialization time.
    pub fn resolve(&self, provider_id: ProviderId) -> Result<SecretString, ProviderError> {
        let val = std::env::var(&self.env_var_name).map_err(|_| {
            ProviderError::SecretResolutionFailed {
                provider_id,
                env_var: self.env_var_name.clone(),
                reason: "environment variable is not set".to_string(),
            }
        })?;
        if val.trim().is_empty() {
            return Err(ProviderError::SecretResolutionFailed {
                provider_id,
                env_var: self.env_var_name.clone(),
                reason: "environment variable is empty or whitespace".to_string(),
            });
        }
        Ok(SecretString { inner: val })
    }

    /// Returns the environment variable name.
    #[must_use]
    pub fn env_var_name(&self) -> &str {
        &self.env_var_name
    }
}

/// Opaque wrapper around a resolved credential that refuses to print its contents in `Debug` or `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    /// Creates a new `SecretString` directly in memory for testing or runtime injection.
    #[must_use]
    pub fn new_in_memory(secret: String) -> Self {
        Self { inner: secret }
    }

    /// Exposes the secret value exclusively for attaching an HTTP authorization header.
    #[must_use]
    pub(crate) fn expose_secret(&self) -> &str {
        &self.inner
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
