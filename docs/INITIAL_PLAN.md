# VESPERA: repository assessment and initial development plan

Assessment date: 2026-11-04. Scope: Milestone 0 and Milestone 1 of the supplied master directive.

## Repository assessment

No existing VESPERA directory or Rust workspace was found in accessible source locations under `/home/ubuntu`, `/opt`, and `/tmp`, or in the initially checked `/workspace` and `/app` locations. This is not a claim about the user's local machine or any unconnected repository. The only project input was the supplied master directive, preserved as `docs/MASTER_DIRECTIVE.txt`. Create a new project at `/home/ubuntu/vespera`; do not modify unrelated files or repositories.

The sandbox reports six logical CPU threads, approximately 7.8 GiB of RAM, roughly 39 GiB of free disk space, and no exposed NVIDIA or DRM GPU device. These observations do not establish the user's hardware. Rust was absent initially. The official rustup installer installed stable Rust 1.99.1 and Cargo 1.99.1 with rustfmt and Clippy. This is the initial tested toolchain; no lower minimum supported Rust version is claimed yet.

## Architecture proposal

Use an acyclic Rust workspace. A small domain crate owns model-independent identities, capability vocabulary, model metadata, agent definitions, memory namespaces, task lifecycle rules, resource-budget types, and a versioned event envelope. Keep side effects out of this crate. Configuration parsing and boundary validation belong in a separate crate. A native Rust CLI composes both crates and provides real configuration checks and read-only environment diagnostics. It must not pretend to connect to providers or execute agents.

For Milestone 1 the compile-time dependency graph is:

```text
vespera-app -> vespera-config -> vespera-core
vespera-app -----------------> vespera-core
```

Future adapters, scheduling, storage, and simulation depend on domain contracts rather than the application. Introduce crates only when they own working functionality. The provider interface is deferred to Milestone 2, where streaming, cancellation, health, capability negotiation, and provider-specific lifecycle support can be tested together. Inference and simulation clocks must remain independent. Agent IDs and memory namespaces must not depend on a model or provider ID.

The future control plane will use Tokio, with Axum and WebSockets at the API boundary. SQLite is the first persistent-memory candidate. A Bevy/wgpu client is an optional presentation layer, not an orchestrator dependency. These are architectural candidates, not installed runtime requirements in Milestone 1.

## Initial workspace structure

```text
vespera/
  Cargo.toml, Cargo.lock, rust-toolchain.toml, README.md, LICENSE
  crates/
    vespera-core/     # validated domain types and deterministic rules
    vespera-config/   # strict TOML parsing and referential validation
    vespera-app/      # Rust CLI composition and read-only diagnostics
  config/local.toml
  docs/
    MASTER_DIRECTIVE.txt, INITIAL_PLAN.md, ARCHITECTURE.md,
    ROADMAP.md, DEPENDENCIES.md, PROVIDERS.md, AGENTS.md,
    MEMORY.md, PROTOCOL.md, DEVELOPMENT_LOG.md, IMPLEMENTATION_REPORT.md
  .github/workflows/ci.yml
  artifacts/         # actual validation logs; no inference data or credentials
```

No empty provider, rendering, or orchestration crate will be added. The license will reserve rights pending an explicit owner licensing decision; publication remains disabled.

## Milestone 1 implementation plan and completion criteria

Implement typed IDs, extensible capabilities, replaceable model selection, explicit permissions, agent-scoped memory addressing, constrained task transitions, versioned event serialization, and checked resource arithmetic. Define strict configuration with unique IDs, resolved model/provider/agent references, loopback-only networking by default, no embedded credentials, and a one-inference default. Provide a Rust CLI that validates a supplied configuration, emits a truthful summary, and inspects available CPU/RAM/GPU evidence without loading models or making network requests.

Completion requires all workspace crates to compile on the recorded stable toolchain; positive and negative domain/configuration tests and CLI integration tests to pass; rustfmt and strict Clippy to pass; documentation generation to pass; and the sample configuration to validate. Tests must cover independent agent/model identity, memory namespaces, terminal task states, checked arithmetic, unknown-field rejection, duplicates, broken references, credentials in URLs, and remote endpoints denied by default. Record actual results and limitations in the implementation report. A lockfile and reproducible CI commands are required. No simulated success responses or unimplemented command stubs will be shipped.

## Hardware-aware design considerations

Registering models must never reserve their estimated memory. Unknown VRAM or RAM estimates must remain unknown rather than become zero. Resource planning will be pure and conservative; only future adapters can report or control residency. A default maximum of one active inference permits time-sharing on modest hardware. A CPU-only host remains valid, and optional GPU telemetry and rendering must report unsupported features rather than invent measurements. Context limits, KV-cache overhead, memory headroom, and provider-global consumption must be considered before claiming a model can fit.

Queue capacity, cancellation, fair scheduling, and measured memory admission will be built in later milestones. Rendering will be separately disableable and may eventually fall back to a native text/2D view. The foundation will not start a service or expose a port.

## Dependency rationale and sources

Use Serde and serde_json for typed configuration and event serialization, UUID for collision-resistant independent IDs, thiserror for structured errors, TOML for readable configuration, URL for proper endpoint validation, and Clap for a native CLI. Use a shared Cargo lockfile and workspace dependency definitions. Cargo documents these workspace facilities [1]. TOML documents its Serde-compatible configuration support [2]. The Rust toolchain was installed using the official Cargo installation guidance [3]. Exact resolved dependency versions and licenses will be recorded after Cargo resolves the workspace.

**References**

[1]: https://doc.rust-lang.org/cargo/reference/workspaces.html "Cargo workspaces reference"
[2]: https://docs.rs/toml "Rust TOML crate documentation"
[3]: https://doc.rust-lang.org/cargo/getting-started/installation.html "Official Rust and Cargo installation guidance"
