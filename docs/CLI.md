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
| `run-workflow` | Execute a strict bounded workflow through authorized agents | Selected health before each attempt; supported load/inference; optional cleanup |
| `memory` | Explicit local SQLite memory operations for a configured owner | None; file/database I/O only |
| `events` | Initialize, replay or follow a local event journal | None. Local SQLite access. |

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

A compatible task admitted at 512 tokens resulted in provider-reported context allocation of 65,536 tokens. Compatible chat has no portable allocation setting; scheduler context is a logical constraint rather than proof of actual backend allocation. At the recorded source checkpoint native preload could initially allocate the backend default; scheduler-driven native preload now sends the admitted context too. Estimates must account for backend behavior; external/startup allocations and per-request overhead remain unreconciled.

## Execute a workflow

```powershell
.\target\release\vespera.exe run-workflow --config config/local.private.toml --workflow-file config/workflow.example.json --resume --timeout-seconds 120 --unload-after --json
```

The shipped [workflow example](../config/workflow.example.json) validates structurally and references the sample agent UUID. To execute it, configure a real installed model, known estimates/budgets and `permissions = ["delegate"]` for its coordinator in the private configuration. The default sample intentionally grants no delegation. The example uses the same agent for two dependent steps; distinct specialized candidates are supported and tested separately. Eight-token output bounds are deliberately small and can produce length-limited text, whose finish reason remains recorded.

| `run-workflow` option | Requirement/default |
| --- | --- |
| `--config <PATH>` | Default `config/local.toml` |
| `--workflow-file <PATH>` | Required strict UTF-8 JSON, at most one MiB |
| `--timeout-seconds <N>` | Overall health/execution bound 1–3,600; default 60 |
| `--resume` | Activate only named coordinator/candidate agents for this invocation; no new grants |
| `--unload-after` | Explicit supported unload after local execution, including failure |
| `--json` | Root/child history, accepted contributions/errors and cleanup evidence |

The file supplies whole-workflow and per-step deadlines; the command deadline additionally bounds the entire operation. Cleanup has up to five extra seconds. Unknown fields, invalid bounds/cycles, missing configured agent IDs and denied coordinator permissions fail before provider requests. A suspended coordinator requires explicit `--resume`; suspended candidates otherwise remain ineligible. Source configuration is unchanged.

Composition initializes only providers/models eligible for named idle (or explicitly resumed) step candidates. A delegation-only coordinator needs no model activation or adapter/credential initialization. Each attempt refreshes relevant health, so a long prior step does not leave the next step dependent on expired observations. No unrelated inventory is activated.

JSON fields are `task_id`, optional `workflow`, optional sanitized `error`, and `cleanup`. A failed/cancelled/partial workflow returns nonzero while retaining accepted attempt evidence. Command cancellation/expiry recovers any workflow history created before interruption; failure before workflow acceptance may leave it null. Early file/configuration/permission/credential errors can leave stdout empty. Human output prints a selected contribution only after workflow evaluation succeeds. Cleanup behavior and compatible unload limitations are the same as `run-agent`.

See [ORCHESTRATION.md](ORCHESTRATION.md) for strict plan fields, bounds, retry/selection/conflict policies and in-process retention limits. [Live evidence](../artifacts/live-workflow-2026-11-06.json) records one native two-step sample, not broad interoperability or factual validation.

## Offline durable memory

`memory` validates configuration and the selected agent's grants without resolving provider credentials, starting adapters, activating an agent or contacting a backend. Grant `memory_read` and/or `memory_write` explicitly in the agent's private TOML `permissions` list, preserving any other grants. Memory ownership follows the configured UUID across model changes.

```powershell
$memoryArgs = @('memory', '--config', 'config/local.private.toml', '--agent', '33333333-3333-4333-8333-333333333333')
.\target\release\vespera.exe @memoryArgs init --json
Set-Content -LiteralPath data/observation.txt -Encoding utf8 -Value 'One operator observation about the amber experiment.'
.\target\release\vespera.exe @memoryArgs add --content-file data/observation.txt --layer observation --json
.\target\release\vespera.exe @memoryArgs list --layer observation --limit 10 --json
Set-Content -LiteralPath data/query.txt -Encoding utf8 -Value 'amber experiment'
.\target\release\vespera.exe @memoryArgs search --query-file data/query.txt --json
```

Common options precede the subcommand: required `--agent <UUID>`, `--config <PATH>` (default `config/local.toml`), and `--database <PATH>` (default `data/vespera.sqlite3`). `--json` may also follow the subcommand. Each operation supports `--help`. POSIX shells can pass the same flags directly to `./target/release/vespera`.

| Subcommand | Options and permissions |
| --- | --- |
| `init` | No action options. `memory_write`; creates parent directories and initializes or validates an existing database. |
| `add` | Required `--content-file <PATH>`; optional `--id <UUID>` (fresh UUID by default), `--layer <LAYER>` (default `observation`), `--origin observation\|import` (default `observation`), `--created-at-unix-ms <N>` (current local observation by default). `memory_write`. |
| `get` | Required `--id <UUID>`. `memory_read`; returns record and lineage, or JSON `record: null` and nonzero status when absent. |
| `list` | Optional structured query flags below. `memory_read`. |
| `search` | Required `--query-file <PATH>` plus the structured query flags. `memory_read`; literal phrase search. |
| `usage` | No action options. `memory_read`; only this namespace's logical record and byte counts. |
| `consolidate` | Required `--content-file <PATH>`, repeatable `--source <UUID>` (1–32 distinct owned records), `--method <LABEL>` (1–128 UTF-8 bytes); optional `--id`, `--layer` (default `long_term`), `--created-at-unix-ms`. Both grants. Stores supplied text and lineage; runs no summarization model. |
| `delete` | Required `--id <UUID>`. Both grants. Removes an unreferenced record and FTS entry atomically; returns `memory_absent` for an already absent record too. Referenced sources cannot be deleted before their derived records. |

Content files must contain nonempty UTF-8 up to 64 KiB. Query files must contain a nonempty UTF-8 phrase up to 256 bytes and at most 32 whitespace-separated terms. Layers: `long_term`, `episodic`, `working`, `relationship`, `goal`, `observation`. Timestamps must fit a nonnegative SQLite signed integer; no task source or `generated` origin can be declared by this offline CLI because it has no accepted task ledger. A derived record is created only through `consolidate`.

| `list` / `search` filter | Bound/default |
| --- | --- |
| `--layer <LAYER>` | Optional layer |
| `--after-sequence <N>` | Owner-local cursor; default 0; use `next_after_sequence` from the preceding page |
| `--created-since-unix-ms <N>` | Optional inclusive timestamp |
| `--created-before-unix-ms <N>` | Optional exclusive timestamp |
| `--limit <N>` | 1–100 records; default 100 |
| `--max-content-bytes <N>` | 65,536–4,194,304 bytes; default 1,048,576; metadata is separate |

Only `init` can create storage. Read operations open SQLite read-only; existing-file write operations do not create or migrate a missing or empty database. Denied permissions are checked before content/database access. JSON output carries `agent_id` and the relevant `record`, `page`, `usage`, `database_ready`/`sqlite_version`, or `memory_absent` fields. Ordinary failures are nonzero with sanitized stderr; no private path, content excerpt or raw SQL is included. File/database work uses a blocking worker and is outside inference timeout flags. See [MEMORY.md](MEMORY.md) for schema, quotas and recovery limits.

## Opt-in memory during inference

Use these options on `run-agent` after initializing a database with `memory init`.

| Option | Behavior |
| --- | --- |
| `--memory-query-file <PATH>` | Read a nonempty UTF-8 literal phrase of at most 256 bytes. Requires `memory_read`. |
| `--remember-result` | Store accepted output as generated episodic memory. Requires `memory_write`. |
| `--memory-database <PATH>` | Existing database used by either option. Defaults to `data/vespera.sqlite3`. |

Retrieval examines the first eight matches in namespace order. Whole records that fit within an 8 KiB serialized JSON message are sent as untrusted user content before the original task prompt. This is a byte limit. Provider tokenization and context handling still apply. The JSON execution report lists included IDs in `memory_context_ids` when at least one record is attached. The stored task snapshot keeps the original prompt.

A retained response carries the accepted task ID and appears as `stored_memory_id` in the JSON report. Storage failure returns nonzero with `memory_error` and preserves the completed execution evidence. File and database preparation and post-inference persistence run on blocking workers outside the inference deadline.

## Local completion journal

```powershell
New-Item -ItemType Directory -Force data | Out-Null
.\target\release\vespera.exe events --journal data/events.sqlite3 init --retention 10000
.\target\release\vespera.exe events --journal data/events.sqlite3 replay --limit 100
```

`events` uses `data/events.sqlite3` by default and prints JSON. `init` requires an existing parent directory. Retention is fixed at initialization and accepts 1 to 100,000 events. The default is 10,000. `replay` opens the existing file read-only and returns 1 to 256 records per page. The default is 100. Pass `--after <journal-UUID:sequence>` using the fields of `next_cursor` to continue. Check `has_more` for another page and `discarded_through` for the retention boundary. An expired, foreign or future cursor returns nonzero.

Pass `--event-journal <PATH>` to `run-agent` or `run-workflow` to append a successful completion to an initialized journal. Single-agent runs emit `inference_completed`. Workflows emit `task_completed` for the root. Reports include the committed cursor or an append error in `event_publication`. Append failure returns nonzero while preserving the execution report. Publication is a separate post-execution transaction, so a process exit between execution and publication can leave a gap.

Journal records contain versioned typed metadata, including the event and task IDs. Local access uses filesystem permissions. Consumers may replay from the same journal after restart. Duplicate event IDs are idempotent while retained if the envelope matches. Use `follow` for local streaming and `replay` for discrete pages. Network subscriptions and broader lifecycle events remain roadmap work.

### Follow committed events

```powershell
.\target\release\vespera.exe events --journal data/events.sqlite3 follow --poll-ms 250
```

Each stdout line contains one JSON entry with its event and cursor. The follower flushes each entry and reads at most one page ahead. It waits between reads when caught up, without holding a read transaction while writing to stdout. Save the last fully consumed cursor and pass `--after <journal-UUID:sequence>` to resume. Without a cursor it starts from the oldest retained entry and reports any earlier discarded history on stderr. If a saved cursor expires while following, the command exits nonzero and reports the retention boundary.

`--poll-ms` defaults to 250 and accepts 10 to 60,000. The command continues until interrupted or until it reaches an optional `--max-events <N>` or `--idle-timeout-ms <N>` bound. The idle timeout accepts 10 to 3,600,000 milliseconds and applies while waiting for new entries. A closed stdout pipe ends delivery. Consumers control how quickly the command can write to stdout, so an idle timeout does not bound writes to a stalled consumer.
