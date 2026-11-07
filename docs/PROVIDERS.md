# Provider adapters

Vespera includes an object-safe asynchronous Rust `InferenceProvider` contract and
Ollama/OpenAI-compatible HTTP adapters, composed through the Rust application
library and explicit `provider-health`/`run-agent` CLI operations. Offline
configuration/model/agent inspection remains separate. Health is refreshed during
an invocation; no persistent background polling service is claimed.

## Implemented protocol subset

| Operation | Ollama | OpenAI-compatible |
| --- | --- | --- |
| Health | `GET /api/version` | `GET /v1/models` |
| Enumeration | `GET /api/tags` | `GET /v1/models` |
| Model metadata | `POST /api/show` | Explicitly unsupported |
| Non-stream chat | `POST /api/chat` | `POST /v1/chat/completions` |
| Streaming chat | NDJSON, terminal `done` required | SSE, terminal `[DONE]` required |
| Explicit load/unload | Empty-prompt `/api/generate`, keep-alive control | Explicitly unsupported |
| Residency | `GET /api/ps` | Explicitly unsupported |

The OpenAI-compatible base may end at the server root, a proxy prefix, or a `v1`
suffix; the adapter adds that version segment once. The current payload uses
`max_tokens`, text messages and optional temperature. This is a tested subset,
not universal compatibility with every engine or model family. No tool executor,
multimodal payload support, embeddings operation, or Responses API is supplied.

Ollama capabilities are derived only from reported metadata: `completion` maps to
`text-generation`, `tools` to `tool-use`, and `embedding` to `embeddings`. Valid
other capability slugs are preserved. An embedding-only record does not acquire
text generation implicitly. Unsupported OpenAI metadata is not synthesized into
a successful result or a verified capability declaration.

## Bounded request and stream behavior

`BoundedInferenceRequest::new` validates nonempty model/messages, output-token
limits, a context ceiling, finite optional temperature, a 512-byte model name,
at most 256 messages, and at most one MiB of combined message content.
Deserialization uses the same validation and rejects unknown request/message
fields. Ingress transports must still bound serialized input before decoding it.
Ollama receives its
native context option. The OpenAI-compatible protocol has no portable context
allocation field; the scheduler must enforce its own declared context ceiling.
The provider is still responsible for actually honoring generation limits.

Both adapters expose connection/request/stream-idle/stream-total timeouts and
cooperative cancellation. All durations must be positive and at most one hour.
The JSON deadline covers connection, headers and body together (default 60 seconds).
Streaming has a separate absolute lifetime (default five minutes) in addition to
the idle bound (default 30 seconds). Cancellation stops local I/O/consumption; it does not prove remote
compute stopped. Non-streaming Ollama responses must be terminal. OpenAI SSE
supports CRLF, comments/ignored event fields, multiline data, trailing usage and
TCP fragmentation. Finish reasons are retained until `[DONE]`; EOF without it is
an interruption. Missing finish reason stays unknown rather than becoming an
invented natural stop.

NDJSON lines and SSE lines/events have 256-KiB bounds, including already complete
newline-terminated lines. JSON response bodies are capped at eight MiB, checked
both against declared content length and incrementally while reading. Streams
have a sixteen-MiB aggregate wire-body limit, including ignored keep-alives.
Periodic small chunks cannot extend the absolute stream deadline. Unrelated JSON
objects cannot become healthy model lists or empty residency measurements.

## Resource observations and lifecycle

Ollama residency separates provider-reported total/VRAM and computed host RAM.
Inconsistent measurements (VRAM larger than total) and aggregate overflow are
rejected; values are not clamped into plausible-looking telemetry. These
observations are not automatically reconciled into scheduler reservations yet.
Model metadata resource estimates remain unknown unless separately supplied.

A lifecycle acknowledgement is not measured memory. The scheduler retains
reservations after uncertain lifecycle outcomes and checks provider/model
identity on acknowledgements. See [SCHEDULER.md](SCHEDULER.md) and
[REGISTRY.md](REGISTRY.md). Provider health does not prove model-specific readiness
or hardware fit, and other provider clients can consume resources outside this
process's accounting.

## Endpoints and credentials

`ValidatedEndpoint` defaults to loopback-only HTTP/HTTPS. URL credentials, query
strings and fragments are rejected. Remote HTTPS requires explicit
`EndpointPolicy.allow_remote_https`; remote plaintext HTTP additionally requires
`allow_insecure_remote_http`. Redirects are prohibited in both clients. Clients
disable inherited proxies and pin `localhost` resolution to literal IPv4/IPv6
loopback addresses, preserving the configured port and HTTP/TLS hostname.
Literal loopback endpoints need no DNS. Other DNS names require remote opt-in;
proxy transport is not currently configurable.

Library configs accept a case-preserving environment credential reference
(`[A-Za-z_][A-Za-z0-9_]*`, at most 128 bytes) or a pre-resolved
`SecretString`. Secret wrappers redact formatting and authorization headers are
marked sensitive. Arbitrary upstream diagnostic text is suppressed rather than
passed through incomplete token heuristics; typed errors retain provider ID,
operation/category and HTTP status. Never put actual credentials in TOML,
provider/model names, command arguments, Git, memory or event records.

Configuration schema version 1 permits provider categories `ollama`,
`open_ai_compatible` and `custom`; `custom` is a declaration, not an installed
adapter. `credential_env` is a variable-name reference, not a value. Parsing
performs no credential resolution or provider I/O. `network.allow_remote` and
`network.allow_insecure_http` govern offline declarations; future composition
must map them into the runtime endpoint policy.

## Verification and live scope

The opt-in native Ollama test uses an already installed text model and an idle loopback backend on port 11434. It explicitly loads and unloads that model. Run it while other clients are idle.

```powershell
$env:VESPERA_LIVE_OLLAMA_MODEL = 'hermes3:8b'
cargo test --locked -p vespera-providers --test ollama live_ollama_smoke_test -- --exact --ignored --nocapture
Remove-Item Env:VESPERA_LIVE_OLLAMA_MODEL
```

On Windows, Ollama 0.40.13 with `hermes3:8b` passed live health, enumeration, metadata, explicit load, bounded stream completion, local cancellation after a text delta, and acknowledged unload. The completion reported 10 output tokens over 9 nonempty deltas. Residency reported 512 allocated context tokens and 4,547,149,823 VRAM bytes after load, with the target absent after unload. Cancellation verified the client result. Remote compute cessation was unmeasured. [Ollama chat](https://docs.ollama.com/api/chat) · [Ollama streaming](https://docs.ollama.com/api/streaming) · [Ollama lifecycle request](https://docs.ollama.com/api/generate)

```powershell
cargo test -p vespera-providers --locked
# Opt-in health check against a running local Ollama daemon:
cargo test -p vespera-providers --test ollama --locked -- --ignored
```

Ordinary tests use ephemeral loopback fixture servers and require no model weights, GPU or external service.

Fixture coverage includes enumeration/lifecycle, advertised metadata, telemetry
arithmetic, bounded payload fields, endpoint prefixes, streaming framing and
completion, malformed/oversized data, cancellation, idle timeout, redirects,
auth errors, diagnostic suppression, request deserialization, aggregate bounds,
whole deadlines and proxy isolation using subprocess-local environment fixtures.
### LM Studio Bionic

The installed daemon identified itself as **LM Studio Bionic v1.2.0+2**. Its loopback OpenAI compatibility server passed model discovery, bounded chat, streamed text with a terminal event, and typed client cancellation after visible text using a preloaded Gemma 4 12B Q4_K_S model. The bounded chat reported 56 completion tokens. The stream emitted two nonempty deltas and provided no terminal usage count. The health report's `openai-v1` string identifies the protocol path. The daemon supplied the product version separately. [LM Studio compatibility API](https://lmstudio.ai/docs/developer/openai-compat) · [LM Studio Bionic product documentation](https://lmstudio.ai/docs/bionic)

Use `kind = "open_ai_compatible"` with the configured loopback endpoint and the exact ID returned by model discovery. Load the model in the Bionic daemon. Vespera's compatible adapter leaves load, unload, residency and model metadata unsupported. The live run used the loopback server without an auth requirement. Authentication and failure paths remain fixture evidence for this backend. [Loopback server CLI](https://lmstudio.ai/docs/cli/serve/server-start)

A separate `run-agent` invocation kept the configured agent UUID and used the Bionic endpoint through the scheduler and agent runtime. The task completed with visible text and 62 reported completion tokens. The temporary model was unloaded using `lms unload` and the loopback server was stopped.

```powershell
$env:VESPERA_LIVE_OPENAI_ENDPOINT = 'http://127.0.0.1:1234/v1'
$env:VESPERA_LIVE_OPENAI_MODEL = 'vespera-live-check' # Use the loaded model's actual ID
cargo test --locked -p vespera-providers --test openai live_openai_compatible_smoke_test -- --exact --ignored --nocapture
Remove-Item Env:VESPERA_LIVE_OPENAI_ENDPOINT, Env:VESPERA_LIVE_OPENAI_MODEL
```

BionicGPT is a separate product. Its deployment and API were outside this qualification. [BionicGPT model configuration](https://bionic-gpt.com/docs/guides/managing-models/)

## Protocol references

These official references were consulted on 2026-11-06 for the tested wire subset;
they are not records of a live server version:

- [OpenAI Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
  and [streaming events](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events).
- Ollama [chat](https://docs.ollama.com/api/chat),
  [generation/keep-alive](https://docs.ollama.com/api/generate),
  [model details](https://docs.ollama.com/api-reference/show-model-details), and
  [running models](https://docs.ollama.com/api/ps).

## Live execution evidence (2026-11-06)

Two bounded release CLI tasks against Ollama 0.40.13 / hermes3:8b returned `OK`: native and compatible `/v1`. See the [sanitized artifact](../artifacts/live-ollama-2026-11-06.json). This is one backend/model version and nonstreaming execution, not universal compatibility or a benchmark. Native unload was acknowledged. Compatible residency was cleaned up by a separate explicit native operator request, not by the compatible adapter.

A compatible task admitted at 512 tokens resulted in provider-reported context allocation of 65,536 tokens. Compatible chat has no portable allocation setting; scheduler context is a logical constraint rather than proof of actual backend allocation. At the recorded source checkpoint native preload could initially allocate the backend default; scheduler-driven native preload now sends the admitted context too. Estimates must account for backend behavior; external/startup allocations and per-request overhead remain unreconciled.
