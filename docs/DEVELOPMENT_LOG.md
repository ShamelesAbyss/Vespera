# Development log

## 2026-11-06 — Bounded authorized master workflow engine

Previous verified checkpoint: `596fc5c60f88213cb4d1ba08fbe59bfce37e9c33`, pushed; Ubuntu CI [42325700985](https://github.com/ShamelesAbyss/Vespera/actions/runs/42325700985) passed. Added the eighth first-party crate, `vespera-orchestrator`, without changing third-party package versions.

The engine executes actual bounded single/dependent workflows through capability-based candidate routing, permission-checked delegation, managed agent inference and scheduler lifecycle. Plans carry explicit requirements, candidate allowlists, prior-step dependencies, deadlines, attempt/token bounds and output policies. Reports retain root/child states, attempt numbers, candidate and actual model/provider/lease provenance, partial results, failures and conflict evidence. Exact agreement and explicit operator precedence evaluate protocol-level results without claiming factual truth or enabling generated control-plane changes. Only 429/503 rejections qualify for capped retry through remaining idle candidates; failed/suspended identities are never silently resumed and uncertain timeouts are not retried.

Drop guards cover running work and accepted queued delegation; queued cancellation checks the assigned owner and preserves terminal states. Eight retained/running workflows, sixteen steps, forty-eight attempts, bounded prompt expansion and one MiB aggregate accepted generated output cap logical retention. Explicit terminal forgetting cannot delete running work or agent history. Histories remain in-process.

Validation passed: all 151 offline workspace tests (one opt-in live health test ignored), formatting, all-target check, strict Clippy, strict rustdoc, optimized workspace build and eleven optimized orchestration integration tests. Tests use actual native HTTP adapters through the complete library stack, covering dependent payload/provenance, selected-only model activation, permission/capability/resource rejection, partial failure, budgets, capped retry, agreement/conflicts, empty output, caller/global/step cancellation and abort, ownership and retention. A test-only Rust borrow error in invalid-plan fixture construction was corrected before validation. No live multi-model inference claim is made.

Updated current architecture, README, roadmap, dependency/agent/implementation records and added `docs/ORCHESTRATION.md`. This is an operational library checkpoint, not yet application/CLI workflow submission or automatic task decomposition. Next: compose and expose bounded workflow execution in the CLI with real executable fixtures, then durable isolated memory/events and the remaining platform roadmap. Candidate ranking/resource reconciliation and durable workflow history remain explicit limitations.

## 2026-11-06 — Validate managed requests before model loading

Previous verified checkpoint: `31fb2eecf07e34c650947225805ac62970ca9522`, pushed. Ubuntu CI [42325403037](https://github.com/ShamelesAbyss/Vespera/actions/runs/42325403037) succeeded; documentation checkpoint `fd6596e82b8cdb52d20038965addca2c622baac7` also passed [42325094343](https://github.com/ShamelesAbyss/Vespera/actions/runs/42325094343).

Managed task preparation now constructs and validates the bounded provider request after selecting the model but before scheduler admission/loading. Default context is derived from model and scheduler ceilings, then pinned explicitly for admission. Invalid output/context/input/model-name bounds record a failed accepted task without a load or retained reservation. Scheduler `admit_request` verifies the validated native name still matches inventory before lifecycle I/O and rechecks current context/budget limits. Existing manual admission remains available for trusted library users; this does not claim that they supplied a valid inference request.

Regressions verify zero loads/reservations for output above context, executable rejection of unusable model context after health only, and pre-load rejection of changed request binding or excessive model context. All 140 offline tests pass (one opt-in live health test ignored), as do formatting, all-target check, strict Clippy, strict rustdoc, release workspace build and 22 optimized app/CLI tests. No extra live model run was needed for this preflight-only behavior. Remaining work includes richer selection, resource overhead/reconciliation, bounded master orchestration, durable memory/events, external agents, simulation, native 3D and experiments.

## 2026-11-06 — Context-aware native model preparation

Previous verified checkpoint: `fd6596e` (current onboarding and live evidence), pushed. Added provider-port `load_model_with_context` with backward-compatible default behavior and an Ollama override validating the context before HTTP I/O. Scheduler-driven initial loads now transmit the admitted context as `options.num_ctx`; inference already transmitted that same value. Direct legacy preload and unload remain unchanged, and no allocation support is invented for compatible providers.

The executable HTTP fixture checks preload and inference both carry 512 tokens and unload omits allocation options. Invalid contextual preload bounds are rejected before a connection attempt. All 138 offline tests pass (one live health test ignored), along with formatting, all-target check, strict Clippy, strict rustdoc and release workspace build.

A fresh live native release task returned `OK` in 3,719 ms. Provider residency reported exactly 512 allocated context tokens and 4,547,149,823 VRAM bytes. A separate explicit native unload was acknowledged and final residency was empty. Evidence is in `artifacts/live-ollama-context-2026-11-06.json`; this is a single bounded sample, not a benchmark or universal guarantee. Compatible allocation control remains unsupported; reused model context changes, external activity and overhead accounting remain unresolved. Next priority: reject invalid inference requests before any model load, then advance orchestration and durable services.

## 2026-11-06 — Live execution and current onboarding

Verified source checkpoint: `136e1b3214b2b910683d869fe74d054c0dcffb24`, pushed. Ubuntu CI [42323808303](https://github.com/ShamelesAbyss/Vespera/actions/runs/42323808303) passed. Windows release CLI executed two bounded nonstreaming tasks against Ollama 0.40.13 / hermes3:8b: native and compatible `/v1` both returned exactly `OK`, with 20 prompt tokens and two output tokens. Native execution took 18,505 ms overall; the separate compatible sample took 7,582 ms. Different residency/context conditions prevent comparative benchmark claims.

Native unload was acknowledged and the next residency list was empty. Compatible cleanup retained a ProviderManaged reservation; separate explicit native operator cleanup was acknowledged and final residency was empty. No weights were downloaded. The inventory command started the installed Windows daemon during discovery; it remains idle. The sanitized artifact excludes private paths, complete inventory and credentials.

A compatible task admitted at 512 tokens produced provider-reported context allocation of 65,536 tokens. Reported VRAM stayed below the six-GiB estimate in this observation only. Compatible APIs have no portable allocation setting; native preload currently uses backend defaults before contextual inference. Request preflight and contextual preparation are the next priorities. External/startup allocations and per-request overhead remain unreconciled.

Replaced stale root onboarding with actual architecture, commands, setup, provider/agent semantics, limitations and licensing. Updated supporting docs. Validation: JSON parsing and provenance/state/content/cleanup assertions, README local links, executable help agreement, sanitized artifact inspection and Git diff/whitespace review. Source is unchanged from the fully validated checkpoint. Full-platform work remains ongoing.

## 2026-11-06 — Operational CLI composition and executable HTTP integration

Previous verified checkpoint: `aee7e24a7caddbe0e38dc951f7493fa237e581da`, pushed to the recovery branch. Public GitHub Actions evidence independently confirms successful Ubuntu runs for `aee7e24` ([42321278020](https://github.com/ShamelesAbyss/Vespera/actions/runs/42321278020)), `566f6d7` ([42320678469](https://github.com/ShamelesAbyss/Vespera/actions/runs/42320678469)) and `0eb16a7` ([42319792636](https://github.com/ShamelesAbyss/Vespera/actions/runs/42319792636)).

Added the application composition library and actual `models`, `agents`, `provider-health` and `run-agent` commands. Offline inventory/validation/diagnostics remain separate from credential resolution and provider I/O. Agent execution takes a bounded UTF-8 prompt file, explicit identity, token/context/time bounds, optional invocation-only resume, and optional unload. Fixed-agent startup scopes providers/models to the selected agent, so unrelated missing credentials or unsupported adapters do not prevent execution. Selected health polling leaves unrelated in-flight poll generations intact.

The executable now drives the complete real HTTP adapter path. JSON reports include accepted result/provenance or sanitized failure, terminal task observation and cleanup evidence. Local permits drain after work; unload is explicit because providers can be shared. Unsupported/failed unload preserves reservations and returns a nonzero status rather than fabricating release. Command deadlines cover health and execution, with a separate bounded cleanup phase. Tokio OS-signal support routes Ctrl+C into cancellation; actual interactive console signal delivery is not yet manually verified. The CLI uses a current-thread runtime. The lockfile adds only the two transitive signal dependencies documented in `DEPENDENCIES.md`.

Validation passed on Windows: formatting, all-target workspace check, all 137 offline tests (one live test ignored), strict Clippy, rustdoc, optimized workspace build and 21 optimized application/CLI tests. New executable fixtures cover both adapters, unknown estimates/suspension without load, preserved configuration, unrelated provider isolation, credentials supplied only to child processes, prompt limits and deadlines. Initial fixture failures exposed inherited nonblocking sockets on Windows; accepted fixture sockets now explicitly enter blocking mode before bounded request reads. No production diagnostic redaction was weakened to investigate it.

Rewrote architecture and added the complete CLI reference, updated implementation/provider/agent/registry/dependency/roadmap records, and marked the old root README foundation material as historical pending the comprehensive onboarding pass. Local Ollama discovery found client/server version 0.40.13. Its inventory command automatically started the Windows app/server; the initial residency list was empty. No model was loaded by discovery. The release CLI `provider-health` and explicit ignored live Ollama health smoke test passed against that daemon. Separate live execution evidence will be recorded only after an actual bounded task succeeds.

Remaining priorities: request preflight before provider load, richer capability/context/resource-aware selection, per-inference overhead and external/startup allocation reconciliation; bounded multi-step orchestration; durable isolated memory/events; authenticated interoperability; independent simulation; native 3D; experiments; comprehensive final documentation and release validation. No full-platform completion claim is made.

## 2026-11-06 — Bounded retained runtime data and source ownership

Previous verified checkpoint: `566f6d715e0dfb23e9204e4d72acc7b63910f2bb`, pushed to the recovery branch.

Added operator-configurable positive `RuntimeLimits` for registered agents, task history, per-agent memory/inboxes, record text and aggregate retained text. Defaults are 64 agents, 1,024 tasks, 256 memory/inbox records per agent, 64 KiB per textual record and 64 MiB of retained text. This is logical storage accounting, not measured RAM; allocation/provider/caller-clone overhead is explicitly separate. Inspection exposes counts and retained content. Rebinding replaces the correct text charge transactionally, and dispatch of an accepted delegated task does not charge a second history record.

Writes validate all applicable ownership/duplicate/capacity checks before mutation. Memory UUIDs cannot be duplicated across namespaces; task provenance must reference accepted history assigned to the memory owner. Message correlations must reference a task assigned to the sender or recipient. Old tests used dangling advisory IDs only to exercise inbox permission checks; they now use uncorrelated messages, with dedicated new tests covering authorized and rejected task links. A validated reference is not a claim that content is true or the task completed.

New regressions cover record/count limits, aggregate quota rollback, memory-ID preservation after rejected writes, logical ordering, policy-rebinding accounting, source ownership and dispatch at a full task-history bound. Validation passed: formatting, all-target workspace check, all 128 tests with one live test ignored, strict Clippy, rustdoc and optimized workspace build. No automatic deletion/pruning or restart durability is claimed. Next work: compose the validated provider/registry/scheduler/agent path into executable CLI operations, followed by orchestration and durable storage/events.

## 2026-11-06 — Actual bounded agent inference and managed execution ownership

Previous verified checkpoint: `0eb16a7cd8c72f5698787e3bec71f7fba8534b2c`, pushed to the recovery branch.

Added `AgentRuntime::execute_task` in a dedicated execution module. An accepted task resolves the agent's model, acquires the bounded scheduler lease, constructs a validated provider request and actually invokes inference. Results include independent agent/model/provider identities, the unique admission ID, terminal task, generated response/finish reason, provider observations and runtime-observed elapsed time. Provider-native names are captured at admission. Response identity/content/reported output bounds are enforced; tool/lifecycle results cannot silently execute unsupported actions.

The managed invocation owns its whole queue/load/inference lifecycle. Its absolute budget covers preparation and inference. Caller/operator cancellation, deadline and dropped futures record local cancellation and return owned slots; provider/response failures record failure. Synchronous guards handle task abort without async cleanup. Cancellation is explicitly local, with no claim that remote compute stopped or provider memory was freed. Manual task completion is fenced while managed inference is active, closing a premature-release race before provider return. Low-level preparation/completion remains an explicitly caller-owned composition API.

Fifteen agent tests pass, including actual fake-provider invocation/bounds/provenance, invalid results, failed requests, manual completion fencing, uncooperative-provider deadline, abort, and cancellation during queueing/loading/inference. Queued cancellation preserves all other lease owners; interrupted load retains uncertain memory accounting. Enabled Tokio `test-util` only for deterministic agent deadline tests. Strict Clippy initially identified an explicit-drop test scope it could not prove ended before awaits; a lexical scope resolved it. Full checkpoint validation passed: formatting, all-target workspace check, all 123 tests with one live test ignored, strict Clippy, rustdoc and optimized workspace build.

Rewrote `AGENTS.md` to describe actual behavior and remaining boundaries. No tools, automatic memory injection/writes, persistent results, autonomous loop, or CLI execution command are claimed. Next prerequisites are bounded runtime histories/memory provenance and application composition; orchestration, persistence/events and the remaining full-platform roadmap remain active.

## 2026-11-06 — Bounded transport and validated request deserialization

Previous verified checkpoint: `497c924b6130b39880c2ed26df6ef7c5f7164a1e`, pushed to the recovery branch.

Provider JSON operations now share a transport helper with an eight-MiB response cap and one absolute request deadline across connection, headers and body. Declared and undeclared body lengths are bounded incrementally. Streaming has a sixteen-MiB aggregate wire-body cap and a separate absolute lifetime (default five minutes), so keep-alive traffic cannot evade the existing idle timeout. Timeout settings are positive and at most one hour. A provider response message DTO accepts extra server fields while retaining only public content; request messages remain strict. Required enumeration/residency arrays prevent unrelated JSON from fabricating health or empty measurements.

Both clients disable inherited proxy settings and pin `localhost` to literal IPv4/IPv6 loopback addresses while preserving the configured port/hostname. The validated local-only endpoint cannot escape through system DNS or proxy inheritance; remote DNS remains explicit opt-in and redirects remain disabled. Port-zero validation now checks the parsed port rather than matching arbitrary URL path text. Credential references preserve valid mixed/lower-case names in agreement with configuration parsing.

`BoundedInferenceRequest` deserialization now calls its validated constructor. Unknown fields, invalid generation/context/temperature settings, model names over 512 bytes, conversations over 256 messages or one MiB of combined content are rejected before transport. Ingress must still bound serialized data before decoding it; these limits do not supply a tokenizer or prove prompt fit.

New tests exercise constructor/deserialization parity, declared/streamed response caps, shared header/body deadlines, continuous keep-alives, aggregate small-frame limits, unrelated JSON and proxy isolation. Proxy tests set environment variables only on child processes, with no unsafe process-global mutation. Validation passed: 31 offline provider tests and 115 full-workspace tests, with the single live Ollama health test ignored; formatting, all-target check, strict workspace Clippy, rustdoc and optimized workspace build also passed.

Remaining limitations: no live provider/version matrix, no vendor-specific context/tokenizer guarantee for compatible endpoints, no automatic telemetry reconciliation or OS credential store. The next integration task is bounded actual inference through owned agent task execution.

## 2026-11-06 — Discovered provider fixtures and truthful protocol completion

Previous verified checkpoint: `f98c8d6f1a392898669b5f1146ece916b2ed4cc5`, pushed to the recovery branch.

Moved the dormant Ollama fixture file from `src` into Cargo's integration-test directory. Its seven offline fixtures passed immediately; the live daemon health test remains ignored and unrun. Added OpenAI-compatible fixtures, first reproducing failures for duplicated `v1` prefixes, EOF falsely becoming success, multiline SSE data, discarded trailing usage and oversized frames. SSE parsing now dispatches complete bounded events, retains finish reasons and reported usage until `[DONE]`, and returns an interruption when transport closes without that terminal event. A missing reason remains unknown. Versioned bases are joined without duplicating their suffix.

Additional failing Ollama regressions exposed invented text-generation capability on embedding-only models, clamped/overflowing residency telemetry, and non-terminal non-stream responses being accepted. Fixed all three. Complete newline-terminated NDJSON lines are checked before parsing. OpenAI metadata now reports explicit unsupported operation instead of synthesizing success/capabilities.

Arbitrary upstream error diagnostics are suppressed rather than incompletely redacted. This also removes a UTF-8 truncation panic path and separated-Bearer-token leakage from the old token heuristic. Typed errors retain safe operation/category/status information. Official OpenAI/Ollama protocol references and exact implemented/live-unverified scope are recorded in the rewritten `PROVIDERS.md`.

Validation passed: provider-only suite has 22 offline tests with one live test ignored; full workspace suite has 106 passing tests and that same one ignored live test. Formatting, all-target check, strict Clippy, rustdoc and optimized workspace build passed. Remaining provider work: bounded aggregate bodies/streams, single request deadlines, DNS/proxy connection enforcement, live provider/version matrix and per-engine interoperability. No live inference claim is made.

## 2026-11-06 — Atomic registry lifecycle and effective provider health

Previous verified checkpoint: `d37c4aac6cc78035e4e8a0d9e01f9d75afe87a3e`, pushed to the recovery branch.

Registry mutations now use a single state lock and reject duplicate provider IDs, model IDs and provider/model-name aliases. Controlled metadata updates preserve immutable provider bindings; model removal does not claim unload and provider removal rejects live model references. Input validation rejects empty names/identifiers and known zero context windows without echoing payloads.

Separated operator metadata (`get_registration`) from effective routing availability. Routing requires a fresh healthy provider poll; unknown, degraded, unreachable and stale observations remain inspectable but unavailable. Defaults are a 30-second freshness window and a bounded five-second per-provider poll. Polls snapshot handles and release all registry locks before I/O; poll generations and provider-instance checks fence out delayed results. Cancellation preserves previous observations until expiry, while timeout, failed poll or mismatched provider identity records unreachable. Operator-disabled models remain disabled after recovery.

Scheduler reservations now retain their original provider handles. Removing/re-registering registry entries cannot redirect unload or silently reuse a changed provider/binding/resource estimate. Known model context ceilings are enforced independently of the operator budget; unspecified context defaults to the smaller applicable ceiling. Existing leased work is not silently revoked by a registry health change.

Added Tokio's test-only `test-util` feature for deterministic health time tests, with no new runtime library. Seven registry tests and 13 scheduler tests pass, including duplicate races, health expiry/recovery/operator disablement, timeout/cancellation, stalled/newer/replacement polls and unload after inventory removal. Full workspace validation passed: formatting, all-target check, all 84 tests, strict Clippy, rustdoc and optimized workspace build. See `REGISTRY.md` and the updated dependency/roadmap records.

Limitations: explicit polling is a library operation, not yet an application background monitor; health is provider-level rather than model-specific readiness; observations and registry state are not persisted. Next work is enabling the dormant provider fixtures, then actual bounded agent inference and application integration.

## 2026-11-06 — Bounded priority admission and shutdown drain

Previous verified checkpoint: `46b9e5c2f50bba844be410e05c5e51cad33b4fcd`, pushed to the recovery branch.

Added a bounded admission queue whose granted permits follow owned inference leases. The default policy allows 64 waiters and a maximum of eight priority bypasses. Equal priorities are FIFO; after the bypass bound the oldest waiter takes precedence. Full/closed/invalid queue states are typed errors. Cancellation and future abort remove waiters synchronously; preparation failure and final lease drop return permits without detached cleanup tasks. Per-model gate metadata now uses weak handles and prunes expired entries rather than retaining every requested identifier.

`queue_snapshot()` exposes waiting/granted counts independently of memory accounting. `shutdown(timeout)` closes new/queued admission and reports whether existing permits drained within the bound. It neither claims provider cancellation nor frees model reservations. Active leases remain visible after timeout and idle reservations can still be explicitly unloaded. There is no automatic restart/telemetry reconciliation yet.

Validation passed: formatting, all-target workspace check, all 78 tests (including queue unit regressions and scheduler-integrated shutdown), strict Clippy, rustdoc, and optimized workspace build. The roadmap status table now reflects the actual seven-crate implementation while preserving all original acceptance gates and the required 3D destination.

Next prerequisites: registry duplicate/update/removal and effective-health behavior; discover and run dormant provider fixtures; bounded real agent inference; per-inference resource overhead/telemetry; persistent memory/events and orchestration. Milestone 5 is materially hardened, not certified complete: hardware estimates, external provider activity, and restart recovery remain limitations.

## 2026-11-06 — Owned inference leases and exclusive agent execution

Previous verified checkpoint: `60db6742343831be7d47f588b524c338205e3314`, pushed to the recovery branch.

Replaced scheduler release-by-model with private UUID-backed inference leases. Clones share one reservation; explicit release is idempotent, and the final handle's drop releases the local slot synchronously. Provider memory stays charged. Runtime task completion validates the recorded model/lease before any release, preventing wrong-model, unknown-task, or duplicate completion from releasing another inference.

Agent/task reservation is now atomic before asynchronous model resolution and loading. A synchronous preparation guard records cancellation on future abort, or failure on model/provider errors, and restores a usable agent state. Runtime state locks never span I/O. Duplicate IDs cannot overwrite accepted tasks, delegated tasks can only start unchanged for the assigned recipient, and arbitrary status/model-policy changes cannot bypass an active execution. Registration cannot fabricate a running agent. Inbox reads now enforce the communication grant; the previous test expecting unauthorized reads was corrected.

Added task snapshots and deterministic regressions for lease clone/drop/idempotence, wrong/duplicate completion, exclusive preparation, aborted/failed preparation, duplicate delegation, recipient ownership, and inbox permission checks. Full validation passed: formatting, all-target workspace check, all 74 tests, strict Clippy, rustdoc, and optimized workspace build. No claim of actual inference execution is made yet: `start_task_execution` prepares an owned execution, and provider invocation is the next integration task.

API migration: callers use `ScheduledInference::release()` or owned-handle drop instead of `release_inference(model_id)`. `finish_task_execution` keeps its signature but verifies its model argument against runtime-owned state. Queueing, shutdown, bounded memory/message history, persistence, and task inference remain outstanding.

## 2026-11-06 — Coordinated and conservative provider lifecycle

Previous verified checkpoint: `541fec8` (`fix: reject unknown scheduler resource estimates`), pushed to `origin/codex/scheduler-recovery`.

Reworked scheduler lifecycle coordination around a per-model asynchronous gate and a short synchronous accounting mutex. No global scheduler lock spans provider I/O. Other admissions for the same model cannot observe a loading reservation as usable. Reservation snapshots explicitly distinguish `Loading`, provider-acknowledged `Loaded`, `ProviderManaged` (unverified physical residency), `Unloading`, and `Unknown`.

Memory remains charged until unload returns a matching provider/model acknowledgement. Unsupported unload is an explicit error. Failed, cancelled, unacknowledged, and dropped lifecycle futures retain conservative memory reservations; interrupted loads release their local inference slot. A synchronous transition guard handles task abort without spawning asynchronous cleanup. Unknown entries must be unloaded before reuse. This deliberately favors accounting safety over automatic recovery after uncertain provider errors.

Seven deterministic scheduler regressions pass, covering incomplete estimates, successful load/reuse/pressure eviction, failed and unsupported unload, unacknowledged lifecycle responses, simultaneous same-model admissions, failed load, and task abort during load/unload. Full checkpoint validation passed: formatting, workspace all-target check, all 68 tests, strict workspace Clippy, rustdoc, and optimized workspace build. The CLI is unchanged; its integration smoke tests passed in the workspace suite. See `SCHEDULER.md` for lifecycle invariants and limitations.

Remaining scheduler limitations: model-ID-based release still lacks lease ownership; admission is immediate with no priority queue; shutdown orchestration is absent; reservation amounts are supplied estimates with no telemetry reconciliation or per-inference KV-cache estimate. The gate map currently retains model identifiers. These remain active engineering work, not completed Milestone 5 criteria.

## 2026-11-06 — Recovery baseline and conservative scheduler admission

Resumed from clean `main` at `adb163a8750a87fb8bfa6f8dff4e3afcdc10a2a7`, matching the remote HEAD. Created `codex/scheduler-recovery` without altering main or existing history. Rust 1.99.1 is installed on this Windows host. The baseline workspace suite passed 61 tests; no provider or scheduler tests were discovered. The Ollama fixture file currently lives under `src` without a module declaration or test target and therefore supplies no baseline evidence. The GitHub connector returned no combined commit statuses; GitHub Actions job success has not been independently verified in this session.

Admission now returns typed `UnknownResources` when either RAM or VRAM estimates are missing. Explicit zero VRAM remains the representation for a known CPU-only estimate. A deterministic fake-provider regression covers both missing dimensions and each independently missing dimension, checking unchanged accounting and no resident records. Estimates remain estimates; this patch does not establish actual model fit or measured residency.

Validation: targeted scheduler regression passed. Full workspace formatting, all-target check, 62 tests, strict Clippy, rustdoc, and optimized workspace build passed after removing an unused test import found by Clippy. Release executable `validate-config --config config/local.toml --json` and `doctor --json` passed. No live inference or hardware-capacity claim is made.

Priority continuation: transactional unloading and per-model loading coordination; uniquely owned inference leases; bounded queue/cancellation/shutdown; registry duplicate/update/remove/health semantics; agent execution races and ownership; actual bounded inference; orchestration; durable memory/events; interoperability; independent simulation; native 3D; experiments; integration and comprehensive documentation. Existing README, roadmap, and architecture claims restricted to Milestones 0–1 are stale. No later milestone is certified complete by this recovery patch.

Checkpoint identity: the commit containing this entry is discoverable with `git log --all --grep="reject unknown scheduler resource estimates"`; its SHA will be recorded in the next log update (a commit cannot embed its own SHA).

## 2026-11-04 — Milestone 0: inspect and plan

Read the supplied master directive before making project changes. Inspected accessible source locations and found no existing VESPERA project. The assessment is limited to this sandbox and does not cover the user's unconnected computer or repositories. Initially observed six CPU threads, approximately 7.8 GiB RAM, and no exposed GPU devices. Rust was not installed.

Installed official stable Rust/Cargo 1.99.1 with rustfmt and Clippy. Created a new `/home/ubuntu/vespera` directory and preserved the directive. Recorded the repository assessment, current and future architecture direction, initial dependency graph, workspace structure, hardware considerations, and measurable implementation criteria in `INITIAL_PLAN.md` before writing implementation code.

Selected three functional crates rather than creating empty crates for all future milestones. Reserved licensing pending the owner's decision and disabled package publication. No remote repository was connected or changed.

## 2026-11-04 — Milestone 1: implement the foundation

Implemented side-effect-free Rust domain contracts in `vespera-core`, strict TOML and reference validation in `vespera-config`, and a native CLI in `vespera-app`. Agent identity and memory addressing remain independent of model selection. Unknown model estimates remain optional values. Checked budget arithmetic is deliberately not a resource scheduler or hardware controller.

The core crate's first locked build could not run before a lockfile existed. A normal Cargo build resolved dependencies and generated the shared `Cargo.lock`; subsequent checks used `--locked`. The core's own build, 28 tests, formatting, strict Clippy, and documentation checks passed.

Implemented size-bounded configuration loading, unknown-field rejection, duplicate-ID and referential checks, registered capability coverage, conservative resource defaults, and a loopback-first URL policy. Config errors suppress raw TOML and endpoints. Embedded URL credentials are denied, and environment references are not resolved in this milestone. Added end-to-end CLI tests and read-only diagnostic parsing tests.

## 2026-11-04 — Integration, correction, and documentation

The first full-workspace run compiled and passed all tests, but strict Clippy rejected three nested conditionals in GPU-device-path inspection. Corrected the conditionals with Rust let chains without changing behavior. The first run's evidence remains in `artifacts/validation-initial.log`; it is not treated as an all-green validation.

The corrected final run passed formatting, all-target compilation, all 57 tests, strict Clippy, rustdoc with warnings denied, and the optimized release build. The release executable validated `config/local.toml` and produced truthful `doctor --json` output. The final log ends with `Validation complete: all commands passed`.

The sandbox's RAM increased during execution: the final diagnostic recorded 25,604,427,776 host RAM bytes, six available CPU threads, unknown root-cgroup limits, and no exposed GPU device paths. Initial and final memory observations are time-specific, not a fixed user-hardware claim. Neither report establishes effective inference capacity or measured VRAM.

Completed architecture, all milestone acceptance criteria, provider, agent, memory, draft external-protocol, dependency, and implementation documentation. Provider/API/scheduler/storage/simulation/3D behavior is explicitly future work. Bionic integration is gated on identifying the exact product/version/API rather than assuming compatibility.

Initialized a fresh local Git repository on branch `main` after validation. No commit, remote push, destructive Git operation, GitHub publication, service deployment, or model inference was performed. The supplied GitHub Actions file has not been executed on GitHub. Source packaging excludes `.git` and `target`; the lockfile, sample configuration, documentation, and validation artifacts are included.

## Next work

Proceed to Milestone 2: provider abstraction and one working Ollama adapter. First establish bounded streaming, timeout, cancellation semantics, explicit unsupported capabilities, safe transport policy, and offline protocol fixtures. Do not add 3D or complex autonomous execution before those contracts support them.
