# Roadmap

The order comes from [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt) and the owner's full-platform directive. The historical [INITIAL_PLAN.md](INITIAL_PLAN.md) describes the initial foundation, not a reduction of the current product scope. Status was re-audited on 2026-11-06; source and checkpoint evidence in [DEVELOPMENT_LOG.md](DEVELOPMENT_LOG.md) are authoritative. Gates below remain acceptance criteria, not blanket completion claims.

| Milestone | Current status and remaining evidence |
| --- | --- |
| 0–1: inspection and foundation | Implemented; Windows workspace and CLI validation pass on Rust 1.99.1. |
| 2: provider contract and Ollama | Adapter and discovered offline protocol fixtures pass; framing, terminal completion, metadata and residency arithmetic hardened. Whole-transport bounds/policy and live-version interoperability remain. |
| 3: additional providers | OpenAI-compatible SSE/payload/prefix/auth/error fixtures pass. Live engine compatibility remains unverified. Bionic blocked on exact product/version and authoritative protocol. |
| 4: registry | In-process identity/update/removal and effective-health behavior implemented with deterministic expiry, recovery, timeout, cancellation and stale-result fencing tests. Live provider interoperability and continuous application polling remain unverified/unwired. See [REGISTRY.md](REGISTRY.md). |
| 5: scheduler | Partial: conservative estimates, coordinated load, transactional unload, owned leases, bounded priority/fairness queue, cancellation, bounded shutdown drain tested. Per-request overhead, telemetry reconciliation, external activity and restart recovery remain. |
| 6: master orchestration | Pending; no bounded multi-step orchestrator yet. |
| 7: agents | Partial: identities, permissions, in-memory records/messages, delegated task preparation, ownership and execution-state guards tested. Actual task inference, bounded histories and persistence remain. |
| 8–10: memory/events/interoperability | Domain contracts and draft documents only; durable services/server/SDK pending. |
| 11: simulation | Pending; independent deterministic world engine required. |
| 12: 3D command center | Required product destination; native observed graph and world mode pending. Headless operation remains supported architecturally. |
| 13–14: experiments/integration/release | Pending; no production-readiness or benchmark claims. |

## Milestone 0 — Repository inspection and architecture

Record the accessible repository/environment assessment before creating the workspace. Establish the three-crate foundation, acyclic current/future graphs, trust boundaries, and hardware uncertainty.

- Preserve the directive and record inspected locations and limitations.
- Record stable Rust 1.99.1 and current-host CPU/RAM/GPU evidence without attributing it to the user's hardware.
- Explicitly separate Milestone 1 functions from future services.

## Milestone 1 — Rust workspace foundation and core types

Implement only `vespera-core`, `vespera-config`, and `vespera-app`; ship offline validation, not simulated inference.

- Workspace compilation, tests, rustfmt, strict Clippy, and documentation generation succeed on the recorded toolchain; retain actual command evidence and a shared lockfile.
- Domain tests cover typed UUIDs, extensible capability validation, independent agent/model identity, replaceable policy, default-deny permissions, namespaces, legal/illegal task edges and terminal states, versioned envelope serialization, and checked budget overflow/boundaries/per-request context. Arithmetic neither mutates usage nor reserves resources.
- Strict TOML tests reject unknown fields, duplicate IDs, unresolved references, unsatisfied declared capability requirements, embedded credentials, and remote URLs without required opt-ins. The sample validates; CLI success/failure paths and truthful `doctor` observations are tested without provider connections, services, tools, or model loading.

## Milestone 2 — Provider abstraction and Ollama integration

Build one working Rust adapter against identified official product/version documentation.

- Exercise enumeration, metadata, bounded inference, streaming completion, timeout, cancellation, health, and typed errors in a documented setup.
- Test unsupported lifecycle/telemetry, malformed/interrupted streams, and honest cancellation limitations with local fixtures.
- Redact secrets and enforce endpoint/redirect policy. Live tests are explicit opt-in; ordinary CI stays offline and CPU-only execution remains valid.

## Milestone 3 — Bionic and OpenAI-compatible adapters

**Identify Bionic's exact product/version and obtain authoritative API docs before this milestone.** Never infer its endpoint or OpenAI compatibility. A separately tested OpenAI-compatible adapter cannot imply Bionic completion.

- Record Bionic identification and operations; missing documentation remains an explicit blocker, not a completed adapter.
- Test each implemented adapter's auth, framing, cancellation limits, missing capabilities, and errors against its documented API subset.
- Demonstrate interchangeable provider ports without vendor branches in orchestration. llama.cpp, LM Studio, and custom servers remain additional targets, not assumed compatibility.

## Milestone 4 — Model registry and health monitoring

Maintain metadata independently of loading/execution.

- Register/update/remove stable entries with resolved providers; registration produces no load requests and has no hardcoded model-count cap.
- Test stale/unavailable/unknown health, bounded polling, timestamps, and recovery.
- Unknown estimates and unsupported telemetry stay unknown; registry presence never implies residency.

## Milestone 5 — Resource-aware scheduling and lifecycle

Add a bounded queue, conservative admission, and provider-aware lifecycle handling; pure budget arithmetic alone is insufficient.

- Deterministic fake-provider tests demonstrate single-inference default, queue rejection, priority/fairness, cancellation, and shutdown recovery without oversubscription.
- Reject known over-budget work with headroom/KV-cache allowances; test explicit unknown-estimate policy and separate RAM/device budgets.
- Demonstrate no duplicate loads, supported reuse, and truthful inability to unload or stop compute.

## Milestone 6 — Master orchestration engine

Route bounded authorized workflows by capabilities rather than activating every model.

- Complete one single-step and one bounded multi-step fixture with recorded model choice, delegation, results, and legal task transitions.
- Test capability misses, exhausted budgets, timeout, partial failure, bounded retry, cancellation, and conflicting outputs without fabricated success.
- Maintain attempt/correlation/provenance; separate measured outcomes from generated assertions.

## Milestone 7 — Independent agent framework

Execute agents independently of model/provider identity.

- Replace a model policy while retaining UUID, namespace, relationships, and history.
- Enforce default-deny tool/network/delegation/memory access at actual boundaries; reject self-modification of identity or permissions.
- Demonstrate bounded communication and suspend/resume/cancel; no unrestricted autonomous loop by default.

## Milestone 8 — Persistent memory and retrieval

Implement structured SQLite storage with transactional migration and inspection.

- Reopen two distinct namespaces after restart/model replacement; deny cross-agent reads/writes and test isolation.
- Implement structured queries and FTS before optional embeddings; test retention/deletion and migration rollback or documented recovery.
- Consolidation preserves original episodes/provenance and links derived summaries to sources; identity/permissions stay control-plane owned.

## Milestone 9 — Event bus and real-time observability

Add authorized bounded delivery and durable history where required; envelope types alone are insufficient.

- Test schema compatibility, event IDs, ordering scope, restart, duplicate handling, and cursor expiry.
- Exercise slow/disconnected subscribers, buffer limits, redaction, retention, and journal failure; explicitly report gaps.
- Distinguish critical audit transitions from expendable telemetry and expose no credentials or private hidden chain-of-thought.

## Milestone 10 — External agent interoperability

Implement draft REST/WebSocket semantics and a small Rust SDK; no server/SDK exists now.

- An authenticated test agent registers with approved authority, receives a task lease, submits a result, and reads authorized events.
- Reject invalid credentials, expired/revoked leases, stale attempts, duplicate conflicts, auth replay, and unauthorized identity/namespace/permission changes.
- Test quotas, backpressure, cancellation races, reconnect replay, lease expiry/reassignment. Publish versioned examples and compatibility rules; no gRPC now.

## Milestone 11 — Simulation and artificial civilization

Build a small deterministic world on a reusable contract.

- Reproduce a fixed number of ticks with a recorded seed/inputs, without inference access.
- Pause/stall inference while world state stays valid; reject or explicitly reconcile late results by world version.
- Demonstrate a few independent residents, goals, relationships, and authorized memories under bounded resources.

## Milestone 12 — 3D intelligence command center

Implement the central native Rust 3D orchestration and world views, evaluating Bevy/wgpu. Rendering remains separate from the authoritative control plane and can be disabled for headless deployments; the 3D product destination is required.

- Show a small observed graph and real task/resource state with useful selection and authorized controls.
- Disable rendering on CPU-only/headless hosts; document native text/2D fallback and unsupported GPU features.
- Measure responsiveness and renderer memory on stated hardware; meaningful controls work without decorative animation.

## Milestone 13 — Experimentation and analytics

Record reproducible bounded experiments, not model opinions presented as research results.

- Export configuration, seed, model/provider versions, event references, measured latency/resources, and known unknowns.
- Reproduce a deterministic fixture's metrics; mark live-provider nondeterminism and missing measurements.
- Compare two bounded policies with stated denominators, failures, and uncertainty; redact unauthorized/private data and secrets.

## Milestone 14 — Integration, optimization, documentation, release

Release only after integrated behavior and limitations are verified.

- Run offline tests plus an explicit live-provider matrix; record CPU-only operation, optional GPUs, permission checks, and interruption recovery.
- Validate packaging, owner licensing decision, dependencies, migration/backup recovery, protocol compatibility, and a fresh-user walkthrough.
- Publish measured performance, resource limits, unresolved issues, upgrade/rollback instructions, and support scope; optimize against recorded baselines.
