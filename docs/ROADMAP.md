# Roadmap

Use the [milestone acceptance ledger](MILESTONE_LEDGER.md) for requirement status, implementation evidence, blockers and next actions in dependency order.

The order comes from [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt) and the owner's full-platform directive. The historical [INITIAL_PLAN.md](INITIAL_PLAN.md) describes the starting workspace. The [development log](DEVELOPMENT_LOG.md) records the source and evidence from the November 8 stabilization audit. The gates below define the remaining scope.

| Milestone | Current status and remaining evidence |
| --- | --- |
| 0–1: inspection and foundation | Implemented; Windows workspace and CLI validation pass on Rust 1.99.1. |
| 2: provider contract and Ollama | Original gates verified on Ollama 0.40.13 / hermes3:8b. Live checks include nonstreaming inference, streamed completion, local cancellation, discovery, metadata and acknowledged unload. Offline fixtures cover transport policy and failure paths. |
| 3: additional providers | Verified the shared OpenAI-compatible adapter against Ollama 0.40.13 and LM Studio Bionic v1.2.0+2. Bionic loopback evidence covers discovery, bounded chat, streaming and client cancellation. Portable lifecycle and residency remain unsupported. |
| 4: registry | Durable SQLite model metadata, revision-fenced edits and live inference from a reopened catalog are verified. The headless health monitor handles bounded rounds, failure and recovery. Opt-in startup lifecycle reconciliation is verified on native Ollama. See [REGISTRY.md](REGISTRY.md). |
| 5: scheduler | Verified in the native Ollama scope. Admission uses conservative estimates, optional host RAM and CPU ceilings and an operator-mapped NVIDIA VRAM ceiling. Fixtures and live runs cover lifecycle, contention, latency routing, external pressure and durable restart recovery. Other GPU backends use declared ceilings. |
| 6: master orchestration | Verified for bounded native Ollama workflows. The coordinator can decompose a task within an authorized roster and explicit budgets. Durable history supports resume at a committed step boundary. Fixtures verify routing, retries, cancellation and conflict policies. See [ORCHESTRATION.md](ORCHESTRATION.md). |
| 7: agents | Bounded execution, permission-checked delegation and operator-controlled planning are verified. Durable task ownership, operator holds and inboxes survive restart. See [AGENTS.md](AGENTS.md). |
| 8: memory | Verified local SQLite namespaces, FTS, lineage, backup and restore. Live inference retrieved memory from a restored database. See [MEMORY.md](MEMORY.md). |
| 9: events | Verified local SQLite replay, recoverable audit, signed broker delivery and native Windows Swarm live and historical observation. See the [event journal CLI](CLI.md#local-task-outcome-journal). |
| 10: interoperability | Signed leases, directed messages, WebSocket resume and broker restart recovery passed over non-loopback TLS between Windows and an Ubuntu WSL2 guest. Rehearse between separate physical hosts to complete the deployment gate. |
| 11: simulation | Persistent deterministic world, native CLI and graphical inspection are integrated. Explicit decisions and bounded resident reasoning use revision fences. World Mode shows preparation and active leases. Retained commits replay without a provider in the CLI and fixed-revision 3D view. |
| 12: 3D command center | All four Atlas modes and Living World persistence and replay were exercised in the Windows x64 candidate. Living World also rendered and replayed through software Vulkan on Ubuntu WSLg. Additional hardware, desktop environments, workflow authoring and OS-backed credentials remain open. |
| 13: experiments | Deterministic world experiments and bounded native Ollama comparisons are verified. Additional providers and broader statistical benchmarking remain outside that scope. |
| 14: release readiness | Windows x64 has an internal `0.1.1` candidate with portable package verification, a hosted per-user MSI lifecycle rehearsal and local rollback evidence. Public distribution still requires signing, notice and licensing review, and a supported production installer toolchain. See [RELEASE.md](RELEASE.md). |

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

The qualified Bionic target is the installed LM Studio Bionic v1.2.0+2 daemon and its documented compatibility API. The [provider guide](PROVIDERS.md#lm-studio-bionic) records the verified operations. Keep the acceptance gates below for future backends.

- Record Bionic identification and operations; missing documentation remains an explicit blocker, not a completed adapter.
- Test each implemented adapter's auth, framing, cancellation limits, missing capabilities, and errors against its documented API subset.
- Demonstrate interchangeable provider ports without vendor branches in orchestration. Standalone llama.cpp, other LM Studio versions and custom servers remain separate qualification targets.

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
- Verified in the bounded local runtime. Operator suspension, task ownership and permission-gated inboxes survive restart. Live Ollama verified bounded planning and retained identity, relationship memory, inbox and history across two model bindings.

## Milestone 8 — Persistent memory and retrieval

Implement structured SQLite storage with transactional migration and inspection.

- Reopen two distinct namespaces after restart/model replacement; deny cross-agent reads/writes and test isolation.
- Implement structured queries and FTS before optional embeddings; test retention/deletion and migration rollback or documented recovery.
- Consolidation preserves original episodes/provenance and links derived summaries to sources; identity/permissions stay control-plane owned.
- Verified in the local SQLite scope. Completed task results can be recovered into episodic memory. Backup and restore preserve namespace isolation, FTS and lineage. Live inference retrieved memory from a restored database.

## Milestone 9 — Event bus and real-time observability

Add authorized bounded delivery and durable history where required; envelope types alone are insufficient.

- Test schema compatibility, event IDs, ordering scope, restart, duplicate handling, and cursor expiry.
- Exercise slow/disconnected subscribers, buffer limits, redaction, retention, and journal failure; explicitly report gaps.
- Distinguish critical audit transitions from expendable telemetry and expose no credentials or private hidden chain-of-thought.
- Verified in the local journal and authenticated broker feed. Native Swarm followed real workflow events and replayed agent statuses at their observed sequence.

## Milestone 10 — External agent interoperability

The [worker service](PROTOCOL.md#implemented-loopback-worker-api) and [Rust SDK](PROTOCOL.md#rust-sdk) support directed, operator-granted external messaging. Signed loopback tests cover revocation, route changes and recovery. The [non-loopback TLS rehearsal](PROTOCOL.md#rehearse-a-non-loopback-tls-deployment) passed between the Windows SDK and an Ubuntu WSL2 guest, including broker restart and event resume. Deployment between separate physical hosts remains open.

- An authenticated test agent registers with approved authority, receives a task lease, submits a result, and reads authorized events.
- Reject invalid credentials, expired/revoked leases, stale attempts, duplicate conflicts, auth replay, and unauthorized identity/namespace/permission changes.
- Test quotas, backpressure, cancellation races, reconnect replay, lease expiry/reassignment. Publish versioned examples and compatibility rules; no gRPC now.

## Milestone 11 — Simulation and artificial civilization

Verified within the bounded [world engine](CLI.md#persistent-world-simulation). Three residents committed two live Ollama choices each and retained their own action histories and encounter relationships. The maximum supported world passed deterministic replay through 5,120 ticks and a history checkpoint rollover. [Reproduce the scale check](CLI.md#reproduce-the-world-scale-check).

- Reproduce a fixed number of ticks with a recorded seed/inputs, without inference access.
- Pause/stall inference while world state stays valid; reject or explicitly reconcile late results by world version.
- Demonstrate a few independent residents, goals, relationships, and authorized memories under bounded resources.

## Milestone 12 — 3D intelligence command center

Implement the central native Rust 3D orchestration and world views, evaluating Bevy/wgpu. Rendering remains separate from the authoritative control plane and can be disabled for headless deployments; the 3D product destination is required.

- Develop the graphical client alongside the orchestration and simulation backends. A runnable, integrated 3D application is a release gate.
- Orchestration Mode shows observed providers, models, agents, task pipelines, communication, memory and resource use. Provide camera navigation, picking, inspection and authorized controls. Label missing measurements.
- World Simulation Mode renders persistent world state with residents, interactions, goals, relationships and shared changes. Identify simulated state separately from real measurements.

- Show a small observed graph and real task/resource state with useful selection and authorized controls.
- Disable rendering on CPU-only/headless hosts; document native text/2D fallback and unsupported GPU features.
- Measure responsiveness and renderer memory on stated hardware; meaningful controls work without decorative animation.

### Living Intelligence Atlas sequence

1. Continue evolving the Intelligence Network. State lanes, scoped attempt links, recent observations, filters, camera transitions and confirmed cancellation are implemented. Search and broader typed entity coverage remain.
2. Machine displays local CPU and RAM observations, wgpu renderer identity, device-wide NVIDIA VRAM from NVML and a bounded graph connecting active task leases, reserved models and providers from an opted-in local run. The adapter inspector cycles accessible NVIDIA devices by PCI address. Request memory nodes show counts of included records. The scheduler inspector shows the latest returned admission decision and its reason. Telemetry for other GPU vendors remains open.
3. Swarm follows accepted workflow attempts, observed dependency inputs and terminal task states from the local journal. The signed operator feed shows worker message acceptance and opt-in recipient acknowledgements. Add combined source navigation and contention evidence.
4. Living World supports individual choices and bounded resident reasoning through the scheduler. The view distinguishes preparation, active local inference and committed actions. Retained world commits replay without inference.
5. World Mode can freeze, step through retained revisions with explicit gaps, and return live. Swarm and the Intelligence Network can replay captured observations while their feeds advance. Atlas can switch among configured layers, preserve view context and transfer exact observed agent identities. Add drill-downs through model, provider, hardware and task relationships as the corresponding evidence becomes available.
6. Reconstruct the Living World as an interactive miniature settlement. Keep authored scenery and character poses separate from the simulation state. Use native Windows captures and frame timing to review each graphics checkpoint. See [Living World visuals](WORLD_VISUALS.md).

Keep read-only operation as the default, require explicit authorization for controls, and show pending and committed outcomes. Preserve headless operation. Validate interactive graphics and performance on the RTX 4070 Laptop GPU, and label measurements and simulation state separately.

## Milestone 13 — Experimentation and analytics

Record reproducible bounded experiments, not model opinions presented as research results.

The [world policy runner](CLI.md#compare-two-world-policies) compares roster policies with paired seed rotations and durable replay. Live experiments compare [fixed local Ollama models](CLI.md#compare-two-live-model-bindings), [owned phrase retrieval](CLI.md#compare-live-retrieval-with-a-fixed-model) and [authored workflow plans](CLI.md#compare-two-live-workflow-plans) on paired private cases. Other provider families remain outside the live experiment runners.

- Export configuration, seed, model/provider versions, event references, measured latency/resources, and known unknowns.
- Reproduce a deterministic fixture's metrics; mark live-provider nondeterminism and missing measurements.
- Compare two bounded policies with stated denominators, failures, and uncertainty; redact unauthorized/private data and secrets.

## Milestone 14 — Integration, optimization, documentation, release

Release only after integrated behavior and limitations are verified.

The `0.1.1` Windows x64 internal candidate passed optimized archive and MSI payload verification, portable backup recovery, and the per-user installer lifecycle on Windows CI. Remaining release qualification includes signing, distribution notices, the owner's licensing decision, and other platforms. [Candidate validation](RELEASE.md#validated-windows-x64-candidate)

- Run offline tests plus an explicit live-provider matrix; record CPU-only operation, optional GPUs, permission checks, and interruption recovery.
- Validate packaging, owner licensing decision, dependencies, migration/backup recovery, protocol compatibility, and a fresh-user walkthrough.
- Publish measured performance, resource limits, unresolved issues, upgrade/rollback instructions, and support scope; optimize against recorded baselines.

### Native application identity

- Keep the supplied Vespera artwork as the canonical icon. Derive the Windows ICO, desktop PNG, and macOS ICNS from the whole square image without redesigning it.
- Display the icon alongside the VESPERA name on the first rendered Home frame and in the shared header during Atlas and desktop navigation.
- Use the same artwork for Windows window chrome, taskbar, application and launcher executables, installer, and installed Start Menu and desktop shortcuts.
- Verify a fresh portable launch and installed launch on Windows. Verify native macOS packaging when that release target is available. [Release validation](RELEASE.md)
