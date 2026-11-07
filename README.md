# VESPERA

A local-first, hardware-aware AI orchestration and simulation environment, built entirely in Rust.

**Current implementation:** the seven-crate workspace now provides Ollama/OpenAI-compatible adapters, a health-aware registry, conservative scheduling with owned leases and bounded admission, independent bounded agent execution, and explicit CLI operations. See the current [CLI reference](docs/CLI.md), [architecture](docs/ARCHITECTURE.md), [roadmap](docs/ROADMAP.md) and [verified checkpoint log](docs/DEVELOPMENT_LOG.md). Durable memory/events, multi-step orchestration, interoperability, simulation, native 3D and experiments remain required roadmap work.

The foundation sections below are historical onboarding material and are being replaced during the comprehensive documentation pass; use the linked current references for runtime commands and validation scope.

## What works now

`vespera-core` provides distinct UUID identities for providers, models, agents, tasks, memory records, and events. Capabilities are validated extensible strings. Agent identity and memory addressing remain unchanged when a model binding changes. Task states follow explicit transitions; terminal states cannot be reopened. Versioned event envelopes have typed payloads. Resource admission arithmetic rejects overflow and RAM, VRAM, concurrency, and per-request context-limit violations without loading or reserving anything.

`vespera-config` loads size-bounded TOML, rejects unknown fields, validates domain records, detects duplicate IDs and broken references, and requires registered capability coverage for capability-bound agents. Providers are loopback-only unless the user explicitly opts into remote endpoints. Plaintext remote HTTP requires an additional opt-in. Embedded URL credentials, query strings, and fragments are rejected. Credentials can be named by environment-variable reference, but are not resolved or used in this milestone. Errors suppress raw input and endpoint strings.

`vespera-app` supplies two real commands: `validate-config` and `doctor`. Neither connects to a provider, activates a model, starts an agent, creates runtime data, or starts a network listener. Diagnostics report missing evidence as unknown and never fabricate VRAM measurements.

## Build and run

Install Rust through [rustup](https://rustup.rs/). The workspace pins stable Rust **1.99.1** and uses edition 2024. Cargo downloads dependencies during the first build; the application itself does not require network access for its current commands. No Python, Node.js, GPU, or database runtime is required.

From the extracted repository root:

```sh
cargo build --release --locked -p vespera-app
cargo run --locked -p vespera-app -- validate-config --config config/local.toml
cargo run --locked -p vespera-app -- validate-config --config config/local.toml --json
cargo run --locked -p vespera-app -- doctor --json
```

After building, use `./target/release/vespera` directly on Linux/macOS or `target\release\vespera.exe` on Windows. Only Linux x86-64 was built and tested during this implementation; Windows and macOS support has not been validated.

The sample contains an **example inventory entry**, not a claim that `example-local-model` exists or is available. Change its provider-specific identifier to an installed model when the Ollama adapter is implemented. The sample agent is suspended and has no permission grants. No activation command is advertised yet.

The defaults allow one inference in resource planning, a 4 GiB RAM budget, zero VRAM budget, and a 4096-token context ceiling. These are user limits, not measured hardware-fit guarantees. Declaring 100 models does not allocate memory or activate any of them.

Configuration format version is `schema_version = 1`. Provider kinds currently supported as declarations are `ollama`, `open_ai_compatible`, and `custom`; they are not operational adapters. Model selection policies are `{ kind = "fixed", model_id = "..." }` or `{ kind = "capability", required = ["text-generation"] }`. See `config/local.toml` for the complete nested TOML layout.

## Validate the workspace

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
```

The RUSTDOCFLAGS shell form above is for POSIX shells. CI uses the equivalent environment setting. Test output, release smoke-test results, and actual host observations are preserved under `artifacts/`. See the implementation report for counts and limitations. The GitHub Actions workflow is supplied but has not been executed by GitHub in this session.

## Workspace and architecture

```text
crates/vespera-app   -> crates/vespera-config -> crates/vespera-core
crates/vespera-app   -----------------------> crates/vespera-core
```

Domain code has no transport, database, async-runtime, or rendering dependency. The future provider interface will make health, streaming, cancellation, enumeration, and model-lifecycle support explicit. Ollama is next; Bionic's exact product/API must be identified before its adapter is designed. Native Bevy/wgpu visualization will remain optional and independent of the control plane.

Documentation:

- [Repository assessment and initial plan](docs/INITIAL_PLAN.md)
- [Architecture and dependency graph](docs/ARCHITECTURE.md)
- [Milestone roadmap and acceptance criteria](docs/ROADMAP.md)
- [Dependency decisions and licenses](docs/DEPENDENCIES.md)
- [Provider contracts](docs/PROVIDERS.md)
- [Agents and ownership](docs/AGENTS.md)
- [Memory design](docs/MEMORY.md)
- [Draft external protocol](docs/PROTOCOL.md)
- [Development log](docs/DEVELOPMENT_LOG.md)
- [Implementation report](docs/IMPLEMENTATION_REPORT.md)
- [Original master directive](docs/MASTER_DIRECTIVE.txt)

## Security and license

The foundation processes operator-owned local configuration. Its permission enums are declarations, not an implemented execution sandbox. Public-field domain records must be validated after deserialization and mutation; `AppConfig::parse` and `AppConfig::load` already perform the required validation. Agent model outputs must never become trusted permission or identity changes in later milestones. Endpoint validation alone is not future transport security: DNS resolution, redirects, network allowlists, credentials, and authentication must be enforced by the provider/API implementation.

Runtime data and private configuration paths are ignored by Git. No credentials are bundled. Package publication is disabled. [LICENSE](LICENSE) reserves rights pending the project owner's licensing decision; third-party dependencies retain their own licenses.
