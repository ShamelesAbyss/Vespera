# Development log

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
