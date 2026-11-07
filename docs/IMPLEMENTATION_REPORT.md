# VESPERA implementation report

## Current recovery implementation — 2026-11-06

The eight-crate workspace now integrates real provider adapters, effective-health
registry operations, conservative model reservations/lifecycle, bounded priority
admission, owned inference leases, bounded independent agents and actual task
inference. The application library composes these services. The CLI implements
`validate-config`, `doctor`, `models`, `agents`, `provider-health` and `run-agent`;
the complete command contract is in [CLI.md](CLI.md).

Current Windows validation passes formatting, all-target compilation, the full
151-test offline suite (one live test ignored), strict Clippy, rustdoc, optimized
workspace build, and all 22 optimized application/CLI tests. HTTP end-to-end
fixtures exercise both adapters, load/infer/opt-in unload, deadline cleanup,
unknown-resource rejection, credential isolation and supported command behavior.
GitHub Ubuntu runs for the preceding `aee7e24`, `566f6d7` and `0eb16a7` checkpoints
were independently checked and succeeded. Current-checkpoint remote CI is tracked
in the development log after pushing; it is not inferred from local validation.

The bounded workflow library is implemented and tested; see [ORCHESTRATION.md](ORCHESTRATION.md). Application workflow submission, automatic decomposition and durable workflow history remain. This remains an incomplete platform: no durable
memory/event bus, authenticated external-agent server, simulation engine, 3D
client, experiment engine or release package is claimed. Provider allocation
reconciliation and per-request overhead accounting also remain. See the current
[architecture](ARCHITECTURE.md), [roadmap](ROADMAP.md) and
[checkpoint log](DEVELOPMENT_LOG.md) for exact scope and live-validation evidence.

## Archived initial foundation report — 2026-11-04

The remainder describes the original foundation delivery and its historical
environment. It is preserved as provenance, not a statement of current scope.

**Milestones 0 and 1 are complete within the supplied initial execution scope.** A new, entirely Rust workspace compiles and passes its recorded checks. This is the tested foundation for VESPERA, not a completed orchestration platform or a production release.

Implementation date: 2026-11-04. Workspace: `/home/ubuntu/vespera`. Tested platform: Linux x86-64. Toolchain: stable Rust 1.99.1, Cargo 1.99.1. No Python runtime or Python-based implementation was introduced.

## Repository assessment and requested planning deliverables

No existing VESPERA source tree was found in the accessible locations inspected. No existing application repository was overwritten. The assessment applies only to this sandbox, not the user's local machine or unconnected repositories. A fresh local Git repository was initialized on branch `main`; no commit, push, or publication was made.

[INITIAL_PLAN.md](INITIAL_PLAN.md) records the repository assessment, architecture proposal, initial dependency graph, workspace structure, first implementation plan and acceptance criteria, and hardware-aware design considerations. It was written before implementation began. [ARCHITECTURE.md](ARCHITECTURE.md) expands the current and future dependency graphs and trust boundaries. [ROADMAP.md](ROADMAP.md) defines measurable completion criteria for all Milestones 0–14.

Only three implemented crates were added. The compile-time graph is acyclic:

```text
vespera-app -> vespera-config -> vespera-core
vespera-app -----------------> vespera-core
```

## Implemented functionality

### Domain contracts

`vespera-core` provides distinct nonnil UUID types for providers, models, agents, tasks, memory records, and events. Capabilities are extensible lowercase ASCII identifiers with validated length and grammar. Model metadata includes provider association, capabilities, responsibilities, availability, and optional context/parameter/quantization/resource information. Unknown estimates are not converted to zero.

Agent definitions separate identity from fixed-model or capability-based model selection. The default status is suspended and permissions default to an empty set. Model rebinding preserves the agent ID, its permission declarations, and its memory namespace. Namespaces are derived only from an agent ID. Memory record types cover long-term, episodic, working, relationship, goal, and observation layers; protected identity is not an agent-writable memory-record layer. These are data contracts, not a persistent store.

Task-state transitions allow queued to running or cancelled, and running to completed, failed, or cancelled. Self-transitions and all exits from terminal states are rejected without changing state. This does not implement task execution or retry attempts.

Resource budget admission is a checked pure calculation. It rejects arithmetic overflow and RAM, VRAM, concurrent-inference, and per-request context-limit violations. CPU-only requests can fit a zero-VRAM budget. It returns prospective usage without mutating state, reserving memory, loading models, or proving hardware fit. Future admission must atomically reserve costs and account for provider overhead, KV cache, and headroom.

Version-1 event envelopes support all thirteen event examples requested in the directive with typed payloads. Unsupported versions, malformed IDs, unknown fields, and invalid envelope contents are rejected. Logical simulation ticks are distinct from Unix timestamps. These are serialization contracts, not a running event bus, journal, or telemetry pipeline.

### Configuration boundary

`vespera-config` parses at most one MiB of TOML, rejects unknown fields, validates records, enforces unique IDs, and checks every provider/model/agent reference. Capability-bound agents must have a matching registered capability set; availability and competence are not inferred. An empty inventory is valid, and a test validates 101 registered models while leaving the single-inference budget unchanged. There is no hardcoded model-count limit, although input size is bounded for safety.

Provider declarations are loopback-only by default. Remote HTTPS requires explicit remote-network opt-in; remote HTTP requires an additional insecure-network opt-in. Embedded credentials, query strings, fragments, zero ports, invalid schemes, and malformed environment-variable references are rejected. TOML errors suppress input excerpts and endpoint values. Credentials are never resolved or used by current commands.

### Native application

`vespera validate-config` checks a real file and reports inventory counts and configured limits, with optional JSON output. It does not check connectivity or activate models. `vespera doctor` reports current OS/architecture, available CPU threads, Linux RAM and root-cgroup evidence where readable, and exposed GPU device paths. Missing measurements remain unknown; VRAM telemetry is explicitly unimplemented. Unsupported runtime commands fail rather than return fabricated success.

## Files created and modified

Every project file below is newly created. In-pass formatting and the targeted diagnostics lint correction modified only these newly created files. No pre-existing application source was changed. The original uploaded directive was copied, not modified.

Workspace/build/configuration files:

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
.gitignore
.github/workflows/ci.yml
LICENSE
README.md
config/local.toml
```

Core crate:

```text
crates/vespera-core/Cargo.toml
crates/vespera-core/src/lib.rs
crates/vespera-core/src/error.rs
crates/vespera-core/src/id.rs
crates/vespera-core/src/capability.rs
crates/vespera-core/src/resource.rs
crates/vespera-core/src/model.rs
crates/vespera-core/src/agent.rs
crates/vespera-core/src/memory.rs
crates/vespera-core/src/task.rs
crates/vespera-core/src/event.rs
crates/vespera-core/tests/identities.rs
crates/vespera-core/tests/resources.rs
crates/vespera-core/tests/domain.rs
crates/vespera-core/tests/tasks.rs
crates/vespera-core/tests/events.rs
```

Configuration and application crates:

```text
crates/vespera-config/Cargo.toml
crates/vespera-config/src/lib.rs
crates/vespera-app/Cargo.toml
crates/vespera-app/src/main.rs
crates/vespera-app/src/doctor.rs
crates/vespera-app/tests/cli.rs
```

Documentation and evidence:

```text
docs/MASTER_DIRECTIVE.txt
docs/INITIAL_PLAN.md
docs/ARCHITECTURE.md
docs/ROADMAP.md
docs/DEPENDENCIES.md
docs/PROVIDERS.md
docs/AGENTS.md
docs/MEMORY.md
docs/PROTOCOL.md
docs/DEVELOPMENT_LOG.md
docs/IMPLEMENTATION_REPORT.md
artifacts/validation-initial.log
artifacts/validation.log
artifacts/config-report.json
artifacts/doctor-report.json
artifacts/cargo-metadata.json
artifacts/dependency-tree.txt
artifacts/dependency-licenses.tsv
```

Build output under `target/` and local Git metadata are not part of the source bundle. Rustup tooling was installed in the sandbox to enable compilation; it is not an application runtime dependency. No runtime `data/` directory or empty future crates were created.

## Architectural and dependency decisions

The core stays free of transport, database, asynchronous execution, and rendering. Boundary validation and CLI composition are separated from domain types. Records with public mutable fields require validation after input or mutation; configuration entry points do this automatically. Typed permission declarations are not claimed to be runtime authorization.

Exact direct third-party versions resolved: Serde 1.0.229, serde_json 1.0.154, UUID 1.29.0, thiserror 2.0.21, TOML 0.9.12+spec-1.1.0, URL 2.5.8, and Clap 4.6.7. Their declared licenses are MIT/Apache-2.0 alternatives. [DEPENDENCIES.md](DEPENDENCIES.md) records feature decisions, rationale, documentation sources, and limitations of this review. The full declared license inventory includes target-specific transitive packages and is not a completed legal or security audit.

Tokio, HTTP transport, Axum, SQLite, Bevy, wgpu, gRPC, model weights, and provider SDKs were not introduced. The optional native visualization must not own orchestration. Project publication remains disabled and rights remain reserved until the owner chooses a license.

## Tests and validation actually performed

Final evidence is [artifacts/validation.log](../artifacts/validation.log). **57 tests passed, zero failed**: 28 core integration tests, 16 configuration unit tests, three diagnostics unit tests, and ten CLI integration tests. Both library doctest harnesses completed successfully with zero doctest examples; they are not included in the 57-test count.

The corrected final validation ran every command below and ended with an explicit all-passed marker:

```sh
rustc --version
cargo --version
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
cargo build --release --locked -p vespera-app
./target/release/vespera validate-config --config config/local.toml --json
./target/release/vespera doctor --json
```

Coverage includes malformed/nil IDs, exact capability grammar, metadata defaults, unknown fields, default-deny declarations, model replacement and namespaces, the full task transition matrix, resource overflow/limits, event round trips/version rejection, duplicates, missing references, network opt-ins, bounded input, secret redaction, missing files, human/JSON CLI output, and rejection of unsupported runtime commands. Tests are deterministic fixtures except intentionally nondeterministic UUID generation and read-only host observations; they do not depend on inference or a GPU.

The first workspace check passed compilation/tests but failed strict Clippy on three nested conditionals in device-path inspection. Those findings were corrected and the complete final run passed. The first log is preserved rather than described as successful. Initial dependency resolution required a normal build before a lockfile existed. No compiler/linter failure was silently ignored.

The GitHub Actions workflow contains reproducible checks but has not run on GitHub. Only Linux x86-64 was tested. No live provider test, vulnerability scan, fuzzing, performance benchmark, migration test, or hardware-fit test is claimed.

## Hardware-aware observations

Initial inspection recorded approximately 7.8 GiB host RAM and six CPU threads. The sandbox scaled its memory during development; final `doctor` output recorded 25,604,427,776 host RAM bytes, six available CPU threads, unknown root-cgroup limits, and no exposed GPU device paths. These observations are time-specific sandbox evidence, not user-machine specifications or effective inference capacity. No measured VRAM was available.

Configuration defaults are a 4 GiB RAM budget, zero configured VRAM, one inference, and a 4096-token context limit. A configured zero-VRAM budget is different from unknown physical VRAM. Inventory registration causes no allocations. The eventual scheduler must separate per-device limits, measured usage, residency, inference costs, and unknown estimates; graphical operation must remain optional.

## Known limitations

No operational provider adapters, provider discovery, model registry service, health monitor, load/unload control, scheduler, orchestrator, agent executor, tool sandbox, persistent memory, event bus/history, authenticated API/SDK, simulation engine, 3D interface, or experimentation system exists yet. Core types related to these capabilities are foundations, not implementations of those later milestones.

Offline endpoint checks do not provide connection-time redirect/DNS protection. Loopback is not authentication. Permission declarations do not isolate processes or enforce tool execution. Public configuration fields can be changed by trusted host code and must be revalidated. No agent/model execution exists that could grant itself permissions.

UUID namespaces do not demonstrate persistence across restarts; that requires the later database implementation and recovery tests. Current diagnostics do not compute true container-effective memory or inspect all nested cgroups. They do not establish GPU usability. No broad cross-platform support or production-readiness claim is made.

Bionic's exact product/version/API is unresolved. It has not been equated with an OpenAI-compatible server, and its adapter cannot be considered complete without authoritative documentation.

## Next recommended milestone

Begin **Milestone 2: provider abstraction and Ollama integration**. Design an object-safe asynchronous Rust provider contract with explicit enumeration, metadata, bounded requests, streaming, cancellation, health, error taxonomy, optional telemetry, and lifecycle support negotiation. Implement one real Ollama adapter, enforcing transport policy and secret redaction. Use local fixtures for fragmented/malformed streams, cancellation races, timeouts, and unsupported capabilities; keep live-provider tests explicit opt-in.

Only after those contracts are tested should scheduling, orchestration, persistent execution, and optional 3D visualization be added. No next-milestone functionality is falsely advertised by the current CLI.

## Live execution evidence (2026-11-06)

Two bounded release CLI tasks against Ollama 0.40.13 / hermes3:8b returned `OK`: native and compatible `/v1`. See the [sanitized artifact](../artifacts/live-ollama-2026-11-06.json). This is one backend/model version and nonstreaming execution, not universal compatibility or a benchmark. Native unload was acknowledged. Compatible residency was cleaned up by a separate explicit native operator request, not by the compatible adapter.

A compatible task admitted at 512 tokens resulted in provider-reported context allocation of 65,536 tokens. Compatible chat has no portable allocation setting; scheduler context is a logical constraint rather than proof of actual backend allocation. At the recorded source checkpoint native preload could initially allocate the backend default; scheduler-driven native preload now sends the admitted context too. Estimates must account for backend behavior; external/startup allocations and per-request overhead remain unreconciled.
