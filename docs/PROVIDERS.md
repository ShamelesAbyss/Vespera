# Providers

## Current: metadata and offline configuration only

Milestone 1 defines provider/model identities, capability and model metadata, model-selection policy, and configuration references. There is **no provider trait implementation, discovery process, connection, health polling, inference, streaming, credential resolution, loading, or unloading** in the current build. `validate-config` verifies declarations, not endpoint availability. `doctor` observes this host, not a provider's machines or memory.

A provider ID identifies a configured engine/endpoint; a model ID identifies a registered model independently of its display name or provider-native identifier. Agent identity remains separate. Endpoints belong to provider configuration; a model references that provider rather than supplying an independent credential-bearing connection. Declared capabilities can inform selection, but are not a health check, permission grant, or verified performance claim.

## Future provider port — draft for Milestone 2

Use a shared Rust contract with provider-specific adapters for enumeration, metadata, availability, bounded inference, streaming, cancellation, health, typed errors, and optional resource observations. Negotiate or explicitly report support rather than assuming uniform behavior. Separate provider-operation support from model capabilities: a model may support structured output while the server cannot cancel a request or expose residency.

The contract should represent supported, unsupported, and unknown states for lifecycle and telemetry operations. Optional operations include load, unload, keep-alive/residency hints, and device-specific utilization. A no-op must not be reported as successful unloading. Provider-owned automatic loading may be observable but not controllable. Capabilities can become stale when the server or model changes; cache them with observation/version information and bounded freshness.

Normalize errors into actionable categories such as authentication failure, denied endpoint, missing model, unsupported operation, malformed response, timeout, cancellation requested/confirmed, resource exhaustion, and transport failure. Preserve safe provider diagnostic context without credentials. Retry only classified transient failures within a bounded policy; an interrupted request may already have consumed resources or produced a result.

Streaming requires explicit framing and bounds on chunks, total response size, idle timeout, and completion. Transport closure is not successful completion. Client-side cancellation can stop consumption even when a provider cannot stop compute; report that distinction. Unknown remote completion must not become an invented cancellation acknowledgement.

## Target adapters and documentation gates

**Ollama** is the first implementation target at Milestone 2. Confirm the selected product/version against its official API documentation before coding inference, streaming, or lifecycle behavior. The present documents assert no tested Ollama support.

**Bionic** is an ambiguous product/API name. Obtain the exact product, version, deployment type, and authoritative API documentation from the user/product before Milestone 3. Do not assume OpenAI compatibility, invent a base URL, or route it through another adapter merely because the name appears in the directive. Missing documentation is an explicit blocker to that adapter's acceptance.

**OpenAI-compatible local endpoints** are a separate Milestone 3 target. “Compatible” is a deployment claim to test, not proof that every operation, streaming format, model enumeration, tool call, lifecycle control, or error matches. Record the verified subset and limitations for each tested engine.

**llama.cpp, LM Studio, custom inference servers, and future local/remote providers** remain targets, not implemented adapters. Add them only with product/version evidence and adapter contract tests. A custom server must implement a documented interface; VESPERA cannot infer an arbitrary protocol.

## Registry is not residency

Registering an arbitrary number of models stores metadata and does not reserve RAM or VRAM. Distinguish registry presence, discovered availability, provider health, load state, and currently executing requests. Provider availability does not guarantee that a model fits or that its endpoint supports a given request. Future health observations need timestamps, stale states, bounded polling, and recovery without repeatedly activating models.

Model metadata can record parameter size, quantization, context limit, responsibilities, capabilities, and estimated requirements when known. Missing data stays unknown. Later performance history must distinguish measured samples from generated claims. A provider's total allocation may include other clients and models outside VESPERA; tracked requests alone cannot prove free capacity.

Future admission must include weight memory, context-dependent KV cache, temporary buffers, batching/concurrency, provider-global overhead, system headroom, and separate RAM/per-device VRAM limits. Estimates vary by engine and offloading strategy. No exposed GPU on this sandbox is not a measured zero-VRAM claim about the user or a remote endpoint. The default single-inference configuration is a safe starting limit, not a guarantee of model fit. Only future measured admission and lifecycle implementations can govern residency.

## Security and acceptance

Current configuration permits loopback by default; remote endpoints need explicit opt-in. URL validation rejects embedded credentials, and configuration holds only environment-variable references for secrets. Offline validation must not resolve DNS, contact endpoints, or print environment values. A valid reference does not establish that its variable exists or that a credential works.

Future adapters must enforce network policy again when connecting, including redirects, resolved destinations, transport security, timeouts, and response-size limits. Loopback is not an authentication mechanism. Do not let provider responses, model output, or tool instructions change credential sources, endpoint policy, or permissions. Logs and events must redact authorization headers, secret values, and sensitive query material.

Milestone 2/3 acceptance requires repeatable adapter tests for success, unsupported operations, malformed/partial streams, missing models, auth errors, timeouts, cancellation limits, and reconnect behavior. Live integration tests require explicit opt-in; deterministic local fixtures must keep ordinary CI offline. This proposal derives from [INITIAL_PLAN.md](INITIAL_PLAN.md) and [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt), not independently verified provider APIs.

## Implemented configuration details

Schema version 1 currently accepts declaration categories `ollama`, `open_ai_compatible`, and `custom`; these are enum values, **not installed adapters**. There is no `bionic` category or compatibility alias. The input file is bounded to 1 MiB; that safety bound is not a hardcoded model-count limit.

Endpoints must use HTTP/HTTPS with a host and nonzero explicit port when supplied. URL user information, query strings, and fragments are rejected. Literal IPv4/IPv6 loopback addresses and the exact hostname `localhost` count as loopback; arbitrary DNS names, LAN addresses, wildcard addresses, and names merely containing `localhost` do not. `network.allow_remote` permits remote HTTPS declarations; remote plaintext HTTP additionally requires `network.allow_insecure_http`. Both flags default to false. Offline parsing performs no DNS resolution; future connections must recheck destinations and redirects.

`credential_env` is only a variable-name reference, with an ASCII letter/underscore first and ASCII letters/digits/underscores thereafter, at most 128 bytes. Parsing neither reads its value nor proves that it is set. Unknown fields are rejected and parse errors suppress input excerpts. This reduces accidental leakage; it cannot determine whether an arbitrary allowed text field was deliberately misused to hide a secret.
