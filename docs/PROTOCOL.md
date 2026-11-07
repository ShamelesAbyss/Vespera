# Protocol and events

## Implemented loopback worker API

Launch the broker with `vespera serve --access-file <path>`. The access file approves worker UUIDs and their capability sets at startup. Workers authenticate with a key assigned to their identity. Setup and a working PowerShell example are in the [CLI reference](CLI.md#external-worker-service).

### Request authentication

1. Fetch `GET /v2/session` without credentials. The response has `session_id` (a UUID unique to the running process) and `elapsed_ms` (milliseconds on the server's monotonic clock).
2. Set `principal` to `operator` or the approved worker UUID. Copy `session_id` and `elapsed_ms` into `session` and `issued_ms`. Generate a fresh random UUID for `nonce`. Use lowercase hyphenated UUIDs and a decimal integer without leading zeros for `issued_ms`.
3. Compute HMAC-SHA256 with the 32-byte key decoded from its 64-character lowercase hexadecimal representation. The signed message is the following UTF-8 prefix, with a single LF after every line, followed immediately by the exact JSON body bytes. Use the full literal path, without a query string.

```text
Vespera-HMAC-SHA256
POST
<path>
<principal>
<session>
<issued_ms>
<nonce>
```

4. Send the signed body with `Content-Type: application/json` and `Authorization: Vespera-HMAC-SHA256 <principal>:<session>:<issued_ms>:<nonce>:<mac>`. Encode `mac` as 64 lowercase hex characters. Sign the serialized body once and send the same bytes, including any whitespace.

The server accepts proofs issued in its current session with an age below 30 seconds. It consumes each verified nonce under the same lock that serializes task operations. Replays receive 401. Each principal has 128 slots for unexpired proofs. A full cache returns 429 without evicting a live nonce. Each application retry needs a fresh proof even when its task ID or completion fence stays the same.

The original loopback bearer endpoints under `/v1` have been retired. Use fresh keys when migrating from the bearer checkpoint. The access file retains its original `operator_token_env` and `token_env` field names.

### Operator and worker actions

| Path | `action` | Additional JSON fields | Success |
| --- | --- | --- | --- |
| `/v2/operator` | `submit` | `task_id`, `prompt`, `required_capabilities`, `eligible_workers`, `lifetime_seconds`, `max_attempts` | 201 with `task_id` and `state` |
| `/v2/operator` | `inspect` | `task_id` | 200 with `state` and `result` (nullable) |
| `/v2/operator` | `cancel` | `task_id` | 200 with resulting `state` |
| `/v2/operator` | `revoke` | `worker_id` | 204 |
| `/v2/worker` | `claim` | None | 200 with `task`, `fence` and `expires_in_ms`, or 204 when no work is eligible |
| `/v2/worker` | `renew` | `fence` | 200 with `expires_in_ms` |
| `/v2/worker` | `complete` | `fence`, `content` | 200 with `worker`, `fence` and `content` |

IDs are UUID strings. Capabilities are strings and capability sets and worker allowlists are JSON arrays. The claimed `task` contains `id`, `prompt`, `required_capabilities` and `state`. Task states are `queued`, `running`, `completed`, `failed` and `cancelled`. A `fence` contains `task_id`, `attempt_id` and integer `generation`. Send the entire returned fence unchanged on renewal or completion. After an uncertain submit, inspect by task ID. After an uncertain completion, retry with the same fence and content while the credential remains valid. After an uncertain claim, the lease can expire and become eligible for reassignment.

Application errors carry `{"error":"code"}`.

| Status | Codes |
| --- | --- |
| 400 | `invalid_json`, `invalid_input`, `invalid_body`, `unexpected_query` |
| 401 | `unauthorized` |
| 404 | `unknown_route`, `unknown_task`, `unknown_worker` |
| 405 | `expected_get`, `expected_post` |
| 409 | `worker_busy`, `duplicate_task`, `stale_lease`, `result_conflict` |
| 413 | `body_limit` |
| 415 | `expected_json` |
| 429 | `nonce_capacity` |
| 503 | `capacity` |

The service accepts up to 32 connections concurrently, reads headers for up to five seconds and gives each connection up to 15 seconds in total. It closes each connection after a response and accepts JSON bodies up to 512 KiB. Broker limits below apply after decoding. The host can reject malformed HTTP or close an expired connection before returning an application error.

Treat loopback as a deployment boundary. Keep each signing key private and use an isolated local account where appropriate. The service uses plaintext HTTP, so request proofs do not provide confidentiality or authenticate the server's responses. Keys remain valid for the lifetime of the process. Rotate them by replacing the environment values and restarting. Revocation takes effect in the running process. Remove a revoked worker from the access file before restarting to keep it revoked. Protected remote transport, network events and durable restart recovery remain open.

The CLI also supports a separate SQLite outcome journal with bounded replay and local streaming. See the [local journal CLI](CLI.md#local-task-outcome-journal).

## Rust SDK

Use `vespera-sdk` as a Cargo path dependency from a checkout. It depends on shared domain types, independent of the broker and CLI crates. Run its async methods on a Tokio runtime with I/O and timers enabled. Construct either client with a numeric loopback `SocketAddr` and `SigningKey::from_hex`. Worker clients also take an approved `AgentId`.

| Client | Methods |
| --- | --- |
| `OperatorClient` | `submit(&Submission)`, `inspect(TaskId)`, `cancel(TaskId)`, `revoke(AgentId)` |
| `WorkerClient` | `claim()`, `renew(Fence)`, `complete(Fence, &str)` |

Every call fetches the server's session and signs the request with a fresh nonce. The client permits 15 seconds for each HTTP exchange, caps JSON responses at 512 KiB, and disables proxies, redirects and automatic retries. `claim()` returns `Ok(None)` for 204. `ClientError::Rejected` carries the HTTP status and a typed `ErrorCode` when the server sends a known code.

After an uncertain submit, inspect the original task ID. For an uncertain completion, retry with the same fence and content. If a claim's response was lost, allow the lease to expire before expecting another claim. The caller owns worker polling, execution deadlines and retry timing.

Build the Rust API reference with `cargo doc -p vespera-sdk --no-deps --locked`. The crate documentation includes a worker example and `cargo test -p vespera-sdk --locked` compile-checks it. [SDK source and example](../crates/vespera-sdk/src/lib.rs)

## Future interoperability draft

The draft sections below describe remaining Milestone 9 and 10 work. A resource-oriented HTTP API and WebSocket event stream are planned. Remote deployments require protected transport and explicit opt-in. Future SDK extensions cover authorized event replay and reconnect behavior.

## Authentication and registration — draft

Registration is an authorized control-plane operation, not anonymous self-enrollment. The server assigns or approves an independent agent ID, binds it to an authenticated principal, and records user-approved capabilities, namespace, quotas, and permission scopes. A client cannot choose another agent's identity, claim ownership by display name, or increase privileges through later registration updates. The user owns identity/permission changes.

Prefer scoped credentials delivered through an approved secret mechanism, with rotation and revocation. Never put credentials in URLs, event payloads, logs, or memory. Authenticate REST requests and WebSocket handshakes; authorize each operation and event subscription independently. Long-lived sessions must notice expiry/revocation and drop access. Loopback deployment still requires a considered authentication policy: local malicious processes are part of the threat model.

Any future remote transport needs application replay protection with authenticated principal, operation, scope and an accepted time window. The local HTTP protocol applies these checks using a session and request nonce. Task lease fences convey authority for an attempt when presented by its authenticated owner.

## REST operation groups — draft, not installed routes

The API needs authorized agent registration/inspection, task submission/claim/renewal/result/cancellation, scoped messaging, and event subscription/replay metadata. Read operations must not leak other agents' namespaces. Write operations need explicit request schemas, bounded payloads, structured errors, and request correlation. Endpoint paths and an OpenAPI schema will be fixed with the implementation, not invented here.

Version the public API independently from event payload schemas and domain storage. Negotiate supported versions explicitly; reject incompatible major versions. Use structured machine-readable codes for unauthenticated, forbidden, invalid schema/state, conflict, stale lease, quota exhausted, unavailable, and replay gap. Responses must never turn a rejected request into a success-shaped placeholder. Retry advice is advisory and bounded; authentication and permission failures are not ordinary transient retries.

## Implemented in-process lease broker

`vespera-api::LeaseBroker` accepts operator-approved worker IDs and capability sets. A host must authenticate the worker before calling claim, renew or complete. Each submitted task has an explicit worker allowlist, a lifetime of up to one hour and an attempt budget of 1 to 8. A worker can hold one lease at a time. Claims select the earliest submitted eligible task.

Leases last up to 30 seconds and are capped by the task deadline. Renewal uses the host's monotonic clock. Every claim assigns a fresh attempt UUID and increments the task generation. Expiry or revocation fences the attempt and queues another attempt if time and budget remain. Cancellation and completion leave one terminal state. A result retry with the same worker, fence and content returns the accepted result. A different result under that fence is rejected.

The broker retains up to 64 workers and 256 tasks per process, including terminal tasks. Prompts and results are capped at 64 KiB each. The HTTP host serializes mutations and owns operator authorization. Restart drops broker state.

## Remote task lease and result semantics — draft

A submitted task has a stable task ID, required capability/scope, deadline, and bounded execution budget. Each execution attempt has a distinct attempt ID and monotonically increasing fencing generation. Claiming a task atomically grants a single active lease bound to the authenticated agent, task, attempt, and generation. The server supplies authoritative expiry and renewal rules. A lease conveys only the task's approved subset of permissions and namespace access.

Lease duration and renewal use server monotonic elapsed time while running. Clients use durations and server-provided expiry information, not comparisons of unrelated monotonic clocks. On restart, durable generations and conservative invalidation/reconciliation prevent stale leases from gaining authority; wall-clock rollback must not revive one. A disconnected agent cannot assume its task still belongs to it.

Renewals, progress, results, and cancellation acknowledgements carry task/attempt/generation and lease authority. Reject expired/revoked leases and stale generations, including a late result from an earlier attempt after reassignment. Expiry may queue a new bounded attempt; it does not prove that the earlier process or provider stopped. Fencing protects accepted state, not physical exactly-once computation. Non-idempotent tools need additional explicit authorization and reconciliation before a retry.

Accepting a result is a single checked state transition tied to the active attempt, with result provenance and an auditable outcome. Terminal tasks cannot be revived by renewal or duplicate completion. Invalid/conflicting results do not overwrite the accepted result. Keep task history separate from transient stream delivery.

## Idempotency, replay, and quotas — draft

Mutating REST operations use a caller-supplied idempotency key scoped to authenticated principal and operation. Store the canonical request digest and outcome for a documented bounded retention interval. An exact retry returns the recorded outcome without repeating the mutation; reuse with different input is a conflict. Transactionally bind the key and mutation. An in-flight key returns a defined pending/conflict outcome rather than duplicate execution. Keys must not be accepted across unrelated principals.

Document idempotency expiry: after the retention interval, a client must reconcile by task/request identity instead of blindly replaying an old write. Idempotency does not defeat stolen credentials or guarantee exactly-once external side effects. Authentication replay protection, task fencing, and operation idempotency solve different problems.

Enforce per-principal and per-agent limits for concurrent leases, pending tasks, message rate, payload size, connections, response bytes, and resource/time budgets, with bounded global limits. A capability claim cannot override quotas. Rejection should include safe retry information where appropriate. Delegation must not amplify authority or bypass a parent's quotas through unlimited child identities.

## Cancellation and backpressure — draft

Cancellation is an authorized idempotent request. Stop further delegation, revoke or fence the applicable lease, signal the agent/provider, and record separately whether stopping compute was confirmed. Use checked transitions to resolve races: if completion already committed, return that terminal outcome; if cancellation committed first, reject late completion. Define deadline/acknowledgement limits so an unresponsive peer cannot retain a task forever. Cancellation does not imply that previously committed tool effects have been undone.

Use bounded queues for task admission and bounded buffers for REST/WebSocket input/output. A slow consumer must not block orchestration or grow memory without limit. Reject new work or signal backpressure when queues fill. Noncritical telemetry may be coalesced/dropped with explicit gap counters; audit-critical task outcomes require a different durability policy. If a client cannot keep up, disconnect with a recoverable cursor/gap indication, not silent loss masquerading as complete history.

## Event envelope, versions, and time

The current Rust envelope is the source of truth for implemented field names and version checks. Conceptually it links an event ID, schema version, event kind/payload, source, wall-clock timestamp, and optional task/agent/correlation references. These types do not yet assign replay cursors, provide durable sequence numbers, enforce subscriber permissions, or coordinate storage transactions.

The future journal needs distinct event identity and replay position. Generate an event ID once and preserve it on retries. Assign durable sequence/cursor values within a documented scope; do not infer global order from UUIDs or wall timestamps. Correlation links a workflow, and causation links its triggering event/operation. Simulation-related events include logical tick/world version separately from human timestamps and measured inference elapsed time. Do not serialize host monotonic values as cross-host timestamps.

Payload compatibility requires explicit schema/version rules and fixtures. Additive changes may be allowed only within a documented reader policy; strict current deserializers must not be promised forward compatibility. Unknown or unsupported versions need an explicit reject/skip/quarantine outcome. Never silently interpret a changed field's semantics as an old version. Public API versions, envelope versions, payload versions, and database migrations are related but not interchangeable.

## Delivery, retention, and failure — draft

Define future delivery as at-least-once within retained history, not exactly-once. Subscribers deduplicate by event ID and resume with an authorized cursor. Apply subscription filtering and redaction during replay as well as live delivery; replay must not re-execute tools or task mutations. A client reconnecting after revocation cannot regain access through an old cursor.

Retention is bounded by configured time/size and privacy policy. Keep audit-critical history separate from high-volume telemetry where needed. If a requested cursor precedes retained data or a journal gap exists, return an explicit replay-gap outcome and a consistent authorized snapshot/resynchronization path. Never present an incomplete stream as a complete task history. Consumer offsets and retention races need test coverage.

In-memory publication cannot establish durability. Persist critical state/event intent atomically through an outbox or equivalent reconciliation strategy, then publish and mark delivery progress. Crash after commit but before publication causes a retry, not loss; crash after publication can cause a duplicate. A state transition and event stored in different systems without reconciliation have a documented consistency risk. On disk-full/journal failure, fail the critical mutation or enter an explicit degraded mode; do not acknowledge durable audit success that did not occur.

Separate user-visible final outputs, authorized messages, task/routing/tool events, and measured telemetry from private model hidden chain-of-thought. Do not publish chain-of-thought or credentials as a debugging stream. Generated assertions are not measured events. Logs, event exports, and retained payloads need bounded content, access control, and redaction consistent with [MEMORY.md](MEMORY.md).

## Acceptance and basis

Before the API/SDK becomes stable, test duplicate and conflicting requests, authentication expiry/revocation, stale attempts, lease renewal/expiry/restart, cancellation races, quota exhaustion, slow consumers, malformed frames, cursor expiry, incompatible versions, journal failures, and unauthorized replay. Document every delivery limitation and provider cancellation boundary. Gates are in [ROADMAP.md](ROADMAP.md); the basis is [INITIAL_PLAN.md](INITIAL_PLAN.md) and [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt), not an existing server specification.

## Implemented event wire contract

The current `EventEnvelope` fields are `schema_version`, `event_id`, `occurred_at_unix_ms`, optional `correlation_task_id`, nonempty `source` and typed `kind`. Versions 1 and 2 are supported. `task_failed` and `task_cancelled` require version 2 and carry only a task ID. Existing event kinds still emit version 1. Readers of mixed journals need version 2 support. Agent, model and provider references live in their applicable payloads. The `kind` discriminator uses snake_case. Payloads carry IDs and bounded metadata. `simulation_advanced` carries a logical tick distinct from envelope wall time.

`EventEnvelope::new` generates an event UUID, selects the earliest compatible schema and validates the supplied source, time and payload. Unix time zero is valid. Deserialization rejects unknown fields and incompatible schema versions. Public-field changes need explicit revalidation. Journal entries carry the cursor alongside the envelope. The proposed remote API has its own versioning and authorization requirements.
