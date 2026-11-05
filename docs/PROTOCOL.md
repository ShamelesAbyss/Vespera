# Protocol and events

## Current surface versus future draft

Milestone 1 provides serializable versioned event envelope types, independent UUID identities, and checked task-state rules. **There is no event bus, durable journal, HTTP/WebSocket server, external-agent authentication, task lease implementation, or SDK.** The current CLI is offline. Data-type serialization must not be mistaken for a deployed wire API or a delivery guarantee.

The following protocol is a **draft proposal for Milestones 9 and 10**, requiring implementation, security review, and contract tests before it is stable. REST over HTTP and a WebSocket event stream are the selected future transports. Remote deployments require protected transport and explicit opt-in. Do not add gRPC now. A future native Rust SDK should wrap authentication, typed requests/results, retries, leases, cancellation, and replay without depending on server implementation crates. External clients in other languages are permitted; they add no JavaScript/Python runtime to VESPERA.

## Authentication and registration — draft

Registration is an authorized control-plane operation, not anonymous self-enrollment. The server assigns or approves an independent agent ID, binds it to an authenticated principal, and records user-approved capabilities, namespace, quotas, and permission scopes. A client cannot choose another agent's identity, claim ownership by display name, or increase privileges through later registration updates. The user owns identity/permission changes.

Prefer scoped credentials delivered through an approved secret mechanism, with rotation and revocation. Never put credentials in URLs, event payloads, logs, or memory. Authenticate REST requests and WebSocket handshakes; authorize each operation and event subscription independently. Long-lived sessions must notice expiry/revocation and drop access. Loopback deployment still requires a considered authentication policy: local malicious processes are part of the threat model.

Transport protection does not alone prevent application replay. Bind authority to principal, method/operation, scope, and an accepted time window; any future signed-request scheme must include a nonce/request ID and reject reused authentication material. The exact credential/signature scheme remains undecided; do not claim one exists. Task lease tokens are scoped authority for an attempt, not reusable general agent credentials.

## REST operation groups — draft, not installed routes

The API needs authorized agent registration/inspection, task submission/claim/renewal/result/cancellation, scoped messaging, and event subscription/replay metadata. Read operations must not leak other agents' namespaces. Write operations need explicit request schemas, bounded payloads, structured errors, and request correlation. Endpoint paths and an OpenAPI schema will be fixed with the implementation, not invented here.

Version the public API independently from event payload schemas and domain storage. Negotiate supported versions explicitly; reject incompatible major versions. Use structured machine-readable codes for unauthenticated, forbidden, invalid schema/state, conflict, stale lease, quota exhausted, unavailable, and replay gap. Responses must never turn a rejected request into a success-shaped placeholder. Retry advice is advisory and bounded; authentication and permission failures are not ordinary transient retries.

## Task lease and result semantics — draft

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

The current `EventEnvelope` fields are `schema_version` (only `1` supported), `event_id`, `occurred_at_unix_ms`, optional `correlation_task_id`, nonempty `source`, and typed `kind`. Agent/model/provider references live in the applicable kind payload, not in a generic top-level agent field. `kind` uses a snake_case discriminator. Payloads contain domain IDs and bounded-purpose metadata, not prompts, responses, messages, credentials, or private reasoning. `simulation_advanced` carries a logical tick distinct from envelope wall time.

`EventEnvelope::new` creates an event UUID from caller-supplied source, time, and payload; it does not consult a clock or observe an occurrence. Unix time zero is valid. Deserialization rejects unknown envelope/payload fields and unsupported versions and validates the envelope. Public-field changes need explicit revalidation. There are no cursors, causation IDs, lease tokens, durable sequence numbers, subscribers, or publication side effects in this type. The future proposal above must version any additions rather than silently assuming they already fit the strict current wire shape.
