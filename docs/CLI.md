# Command-line reference

Build with `cargo build --release --locked -p vespera-app`. On Windows use
`.\target\release\vespera.exe`; on Linux use `./target/release/vespera`.
`cargo run --locked -p vespera-app -- <command>` builds/runs the same CLI.

## Commands and common behavior

| Command | Behavior | Provider/network activity |
| --- | --- | --- |
| `validate-config` | Validate strict schema, references and declared resource limits | None |
| `doctor` | Read host evidence, preserving unknown measurements | None |
| `models` | List declared model metadata | None |
| `agents` | List declared identities, status, policy and permissions | None |
| `provider-health` | Poll configured adapters and report expiring health evidence | Explicit HTTP health requests; no model load |
| `run-agent` | Execute one bounded task through the configured agent/model/provider | Health, supported load, inference; optional unload |

Every command supports `--help`; the root supports `--version`. All except
`doctor` take `--config <PATH>` (default `config/local.toml`). Each command supports
`--json`. Offline inventory does not resolve credential environment variables,
verify model existence, resume agents, or activate models.

Exit status is zero on success and nonzero on failure, unhealthy/stale provider
checks, cancellation/deadline, or requested cleanup that cannot be confirmed.
Errors go to stderr without configuration excerpts or credential values.
`run-agent --json` emits an execution/error/cleanup report once composition
succeeds; early argument/configuration/prompt/credential errors can leave stdout
empty. JSON contains generated content only when an execution result was accepted.

## Offline inspection

```powershell
cargo run --locked -p vespera-app -- validate-config --config config/local.toml --json
cargo run --locked -p vespera-app -- doctor --json
cargo run --locked -p vespera-app -- models --config config/local.toml --json
cargo run --locked -p vespera-app -- agents --config config/local.toml --json
```

Model inventory resource values are declarations, not measurements. Unknown
estimates remain null/omitted and cannot become free scheduler admission. The
sample model identifier is a placeholder; the sample agent is suspended.

## Provider health

```powershell
cargo run --locked -p vespera-app -- provider-health --config config/local.toml --timeout-seconds 30 --json
```

`--timeout-seconds` is an overall 1–3,600-second bound (default 30), including all
provider polls. Individual registry polls additionally have a five-second bound.
Output identifies providers, effective state (`unknown`, `healthy`, `degraded`,
`unreachable`, `stale`) and optional observation age in monotonic milliseconds.
Health failure categories are intentionally aggregated into unreachable; inspect
endpoint policy, daemon status and credential references when troubleshooting.
This command checks all configured providers and initializes their adapters;
`custom` currently has no installed adapter. It registers no executing agents.

## Run one agent task

Prepare an operator configuration with an installed provider-native model
identifier, advertised capabilities, known RAM/VRAM estimates and appropriate
budgets. Estimates must include context-dependent overhead and headroom. A zero
VRAM estimate means an explicit CPU-only estimate; it does not force a provider
onto the CPU. Provider setup must match the estimate. Do not infer RAM residency
from model disk size. The shipped unknown estimates intentionally block loading.

Use a private configuration path such as `config/local.private.toml`, which is
ignored by Git. Keep credentials in referenced environment variables. Create a
UTF-8 prompt file, for example under the ignored `data/` directory.

```powershell
cargo run --locked -p vespera-app -- run-agent --config config/local.private.toml --agent 33333333-3333-4333-8333-333333333333 --prompt-file data/task.txt --resume --max-output-tokens 256 --context-tokens 4096 --timeout-seconds 60 --json
```

The UUID is the configured agent identity, not a model/provider UUID. The example
assumes the sample identity was retained in the operator's private configuration.
The prompt file must contain nonempty UTF-8 text and fit within 64 KiB. Its content
is not a process argument. Configuration/model-policy creation and edits currently
happen in TOML; no agent/model administrative subcommand is supplied yet.

| `run-agent` option | Requirement/default |
| --- | --- |
| `--agent <UUID>` | Required configured agent |
| `--prompt-file <PATH>` | Required bounded UTF-8 task input |
| `--resume` | Explicitly activate a suspended/failed agent for this invocation |
| `--max-output-tokens <N>` | 1–131,072; default 256, constrained by admitted context |
| `--context-tokens <N>` | Optional 64–1,048,576 ceiling, also limited by model/budget |
| `--temperature <FLOAT>` | Optional finite value in 0–2 |
| `--timeout-seconds <N>` | 1–3,600; default 60 across health and task execution |
| `--unload-after` | Request explicit unload of this runtime's reservations after work |
| `--json` | Structured task state, result/error and cleanup evidence |

`--resume` does not rewrite the file, grant permissions or persist status.
A fixed-agent invocation scopes composition to its provider/model; unrelated
credentials and adapters are not initialized. Capability-based policies include
matching candidate inventory. Health refresh targets eligible provider IDs.
Generated output remains untrusted data: this command runs no tools and does not
automatically read/write memories, change identity, or execute model instructions.

## Cleanup and provider sharing

New admissions are closed and local permits drained after the invocation.
Cleanup has an additional five-second bound. Default cleanup does not unload
provider memory because providers may be shared with other applications.
Add `--unload-after` only when explicit unload is intended. A confirmed Ollama
unload removes the charged reservation. Standard OpenAI-compatible unload is
unsupported: inference can succeed while requested cleanup returns a nonzero
exit status and reports the retained `ProviderManaged` reservation.

`cleanup.drained` describes local permits, not GPU inactivity. Remaining models
are reservations without confirmed unload; their RAM/VRAM values are estimates,
not fresh measured residency. Cancellation drops local work and does not prove
remote compute stopped. The process does not yet persist task history or
reconcile pre-existing/external provider allocations at startup. Budget settings
therefore cannot be interpreted as a measured whole-provider capacity guarantee.

Ctrl+C is connected through Tokio OS signal handling; deadline/cancellation
cleanup is tested through the runtime and CLI deadline fixtures. Interactive
console signal delivery has not yet been manually verified on every OS.

## Verification scope

CLI integration tests launch the actual executable against ephemeral loopback
HTTP servers for both adapters. They cover payload bounds/provenance, health,
load/infer/opt-in unload, unsupported unload, suspended/unknown-cost rejection,
offline inventory, child-process credentials, prompt limits and command deadlines.
These fixtures are not live model inference or a product/version compatibility
matrix. See [PROVIDERS.md](PROVIDERS.md), [AGENTS.md](AGENTS.md),
[REGISTRY.md](REGISTRY.md) and [DEVELOPMENT_LOG.md](DEVELOPMENT_LOG.md).

## Live execution evidence (2026-11-06)

Two bounded release CLI tasks against Ollama 0.40.13 / hermes3:8b returned `OK`: native and compatible `/v1`. See the [sanitized artifact](../artifacts/live-ollama-2026-11-06.json). This is one backend/model version and nonstreaming execution, not universal compatibility or a benchmark. Native unload was acknowledged. Compatible residency was cleaned up by a separate explicit native operator request, not by the compatible adapter.

A compatible task admitted at 512 tokens resulted in provider-reported context allocation of 65,536 tokens. Compatible chat has no portable allocation setting; scheduler context is a logical constraint rather than proof of actual backend allocation. Native inference sends its context setting, but current native preload may initially allocate the backend default. Estimates must account for backend behavior; external/startup allocations and per-request overhead remain unreconciled.
