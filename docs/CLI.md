# Command-line reference

The [durable model catalog](REGISTRY.md#durable-model-catalog) documents SQLite inventory initialization, revision-fenced edits and the optional `--model-catalog <PATH>` flag.

Build with `cargo build --release --locked -p vespera-app`. On Windows use
`.\target\release\vespera.exe`; on Linux use `./target/release/vespera`.
`cargo run --locked -p vespera-app -- <command>` builds/runs the same CLI.

## Commands and common behavior

| Command | Behavior | Provider/network activity |
| --- | --- | --- |
| `catalog` | Initialize, list and edit durable model metadata with revision fencing | None |
| `validate-config` | Validate strict schema, references and declared resource limits | None |
| `doctor` | Read host evidence, preserving unknown measurements | None |
| `models` | List declared model metadata | None |
| `agents` | List declared identities, status, policy and permissions | None |
| `provider-health` | Poll once or watch configured adapters and report expiring health evidence | Explicit HTTP health requests; no model load |
| `serve` | Host the external worker task broker | Loopback by default, opt-in remote TLS |
| `world` | Initialize, inspect and advance a persistent simulation or request a model decision | Local SQLite access. Decisions also require a configured agent and provider. |
| `run-agent` | Execute one bounded task through the configured agent/model/provider | Health, supported load, inference; optional unload |
| `agent-task-history` | Read a durable agent task checkpoint and owner | None |
| `run-workflow` | Execute an authored or automatically decomposed workflow through authorized agents | Selected health, bounded inference and optional cleanup |
| `workflow-history` | Read a durable workflow plan and latest checkpoint | None |
| `memory` | Explicit local SQLite memory operations for a configured owner | None; file/database I/O only |
| `events` | Initialize, replay or follow a local journal and recover agent, workflow, world or memory events | None. Local SQLite access. |

Every command supports `--help`. The root supports `--version`. Provider,
inventory and inference commands use `--config <PATH>` (default
`config/local.toml`). See each command's help for its output and input options.
Offline inventory does not resolve credential environment variables,
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
Add `--event-journal <PATH>` to record the same provider UUIDs, effective states
and ages in an existing local journal. Each watch round publishes its own
observations. A journal write failure reports the affected round and exits
nonzero after printing the health report. Health observations describe the
runtime's view at sampling time and do not change model residency.
Health failure categories are intentionally aggregated into unreachable; inspect
endpoint policy, daemon status and credential references when troubleshooting.
This command checks all configured providers and initializes their adapters;
`custom` currently has no installed adapter. It registers no executing agents.

## Run one agent task

Prepare an operator configuration with an installed provider-native model
identifier, advertised capabilities, known RAM/VRAM estimates and appropriate
budgets. Estimates must include retained context allocation and headroom.
Configure temporary per-request memory in the optional
[`[inference_overhead]` table](SCHEDULER.md#per-inference-memory-allowance).
A separate [`[scheduling]` table](SCHEDULER.md#bounded-queue-and-shutdown)
controls admission queue bounds, model priorities, native keep-alive and
pressure eviction. It can also opt into [host-constrained limits for loopback
providers](SCHEDULER.md#local-host-limits).
For a single native loopback Ollama provider, the
[`ollama_gpu_uuid` setting](SCHEDULER.md#nvidia-vram-limit-for-local-ollama)
can constrain VRAM to the observed free memory of an assigned NVIDIA device.
Set `prefer_observed_latency = true` in the same table to use
[recent provider call timings](SCHEDULER.md#observed-latency-preference)
when a managed request can compare all eligible models.
A zero
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
| `--task-store <PATH>` | Create or reopen private task ownership and outcome history |
| `--task-id <UUID>` | Explicit durable task ID. Requires `--task-store` |
| `--prompt-file <PATH>` | Required bounded UTF-8 task input |
| `--resume` | Explicitly activate a suspended/failed agent for this invocation |
| `--max-output-tokens <N>` | 1–131,072; default 256, constrained by admitted context |
| `--context-tokens <N>` | Optional 64–1,048,576 ceiling, also limited by model/budget |
| `--temperature <FLOAT>` | Optional finite value in 0–2 |
| `--timeout-seconds <N>` | 1–3,600; default 60 across health and task execution |
| `--reconcile-residency` | Observe external residency at startup and before each admission |
| `--residency-journal <PATH>` | Create or reopen a private local SQLite journal. Requires `--reconcile-residency` |
| `--acknowledge-residency-recovery` | Confirm earlier provider work has stopped. Clear recovered records verified absent from scoped providers |
| `--unload-after` | Request explicit unload of this runtime's reservations after work |
| `--json` | Structured task state, result/error and cleanup evidence |

`--resume` does not rewrite the file, grant permissions or persist status.
A completed or interrupted durable task can be inspected after exit.

```powershell
.\target\release\vespera.exe agent-task-history --database data/agent-tasks.sqlite3 --task 77777777-7777-4777-8777-777777777777
```

Use the task ID returned by the execution command or supplied with
`--task-id`. [Durable task ownership](AGENTS.md#durable-task-ownership)
A fixed-agent invocation scopes composition to its provider/model; unrelated
credentials and adapters are not initialized. Capability-based policies include
matching candidate inventory. Health refresh targets eligible provider IDs.
Generated output remains untrusted data: this command runs no tools and does not
automatically read/write memories, change identity, or execute model instructions.

## Operator suspension

```powershell
$agent = '33333333-3333-4333-8333-333333333333'
$stateArgs = @('--config', 'config/local.toml', '--database', 'data/agent-tasks.sqlite3', '--agent', $agent)
.\target\release\vespera.exe agent-state @stateArgs suspend
.\target\release\vespera.exe agent-state @stateArgs inspect
.\target\release\vespera.exe agent-state @stateArgs release
```

Use `--model-catalog <PATH>` if the configuration relies on catalog entries.
`suspend` creates or updates the private task database. `inspect` and
`release` require an existing database. All three commands work offline and
return JSON with the configured status, explicit hold and effective status.
Pass the same file to execution with `--task-store`. An operator hold takes
precedence over `--resume`. Releasing it clears the hold, so a configured
suspension still requires `--resume` for execution.

## Agent inbox

```powershell
.\target\release\vespera.exe agent-inbox --config config/local.private.toml --database data/agent-tasks.sqlite3 --agent $sender send --to $recipient --message-file data/message.txt
.\target\release\vespera.exe agent-inbox --config config/local.private.toml --database data/agent-tasks.sqlite3 --agent $recipient list
```

Set `$sender` and `$recipient` to configured agent UUIDs with the
`communicate` grant. `--config` defaults to `config/local.toml`.
Use `--model-catalog <PATH>` when the configuration relies on catalog entries.
`send` accepts nonblank UTF-8 text and creates the database if needed.
Optional `send --task <UUID>` links a task owned by one of the participants in
the same database. `list` requires an existing database and can migrate older
schema versions. Both commands return JSON and work offline. Use a private
local path and issue the commands between invocations using that database.

Add `send --event-journal <PATH>` to publish sanitized message metadata to an
existing journal. If publication fails, the message remains committed and
the command returns its logical tick. See [inbox event recovery](#recover-agent-inbox-events).

## Cleanup and provider sharing

`run-agent` and `run-workflow` accept `--reconcile-residency`. The probes share
the command deadline and require residency telemetry from every provider scoped
to that invocation. The native Ollama adapter supports it. Compatible endpoints
return an unsupported operation before execution. JSON reports
`startup_residency` and, when a later probe completes, `admission_residency`.
Each includes `providers_observed`, `external_models`, `ram_bytes` and
`vram_bytes`. The latest complete admission probe is reported even when its
observed pressure causes rejection. See [scheduler accounting and scope](SCHEDULER.md#startup-and-admission-residency-reconciliation).

Use the same `--residency-journal` path when restarting a runtime with
unresolved model lifecycle work. An acknowledged unload retires its record.
An unresolved record fences its model name until operator-confirmed recovery.
See [durable restart recovery](SCHEDULER.md#durable-restart-recovery).

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

A compatible task admitted at 512 tokens resulted in provider-reported context allocation of 65,536 tokens. Compatible chat has no portable allocation setting. Scheduler context bounds the local request. Native Ollama preload sends the admitted context, and compatible allocations still require independent provider evidence.

## Execute a workflow

```powershell
.\target\release\vespera.exe run-workflow --config config/local.private.toml --workflow-file config/workflow.example.json --resume --timeout-seconds 120 --unload-after --json
```

The shipped [workflow example](../config/workflow.example.json) validates structurally and references the sample agent UUID. To execute it, configure a real installed model, known estimates/budgets and `permissions = ["delegate"]` for its coordinator in the private configuration. The default sample intentionally grants no delegation. The example uses the same agent for two dependent steps; distinct specialized candidates are supported and tested separately. Eight-token output bounds are deliberately small and can produce length-limited text, whose finish reason remains recorded.

| `run-workflow` option | Requirement/default |
| --- | --- |
| `--config <PATH>` | Default `config/local.toml` |
| `--workflow-file <PATH>` | Authored workflow in strict UTF-8 JSON, at most one MiB. Supply either this or `--decomposition-file` |
| `--decomposition-file <PATH>` | Strict JSON task, coordinator, authorized worker roster and planning limits |
| `--workflow-store <PATH>` | Create or reopen a private SQLite checkpoint database |
| `--task-store <PATH>` | Retain worker and planning task history and enforce operator holds |
| `--resume-workflow` | Continue an interrupted workflow from a committed step boundary. Requires the original plan and an existing `--workflow-store` |
| `--timeout-seconds <N>` | Overall health/execution bound 1–3,600; default 60 |
| `--resume` | Activate only named coordinator/candidate agents for this invocation; no new grants |
| `--reconcile-residency` | Observe external residency at startup and before each admission |
| `--residency-journal <PATH>` | Create or reopen a private local SQLite journal. Requires `--reconcile-residency` |
| `--acknowledge-residency-recovery` | Confirm earlier provider work has stopped. Clear recovered records verified absent from scoped providers |
| `--unload-after` | Explicit supported unload after local execution, including failure |
| `--json` | Root/child history, accepted contributions/errors and cleanup evidence |

The file supplies whole-workflow and per-step deadlines; the command deadline additionally bounds the entire operation. Cleanup has up to five extra seconds. Unknown fields, invalid bounds/cycles, missing configured agent IDs and denied coordinator permissions fail before provider requests. A suspended coordinator requires explicit `--resume`; suspended candidates otherwise remain ineligible. Source configuration is unchanged.

Composition initializes only providers/models eligible for named idle (or explicitly resumed) step candidates. A delegation-only coordinator needs no model activation or adapter/credential initialization. Each attempt refreshes relevant health, so a long prior step does not leave the next step dependent on expired observations. No unrelated inventory is activated.

JSON fields are `task_id`, optional `workflow`, optional sanitized `error`, and `cleanup`. A failed/cancelled/partial workflow returns nonzero while retaining accepted attempt evidence. Command cancellation/expiry recovers any workflow history created before interruption; failure before workflow acceptance may leave it null. Early file/configuration/permission/credential errors can leave stdout empty. Human output prints a selected contribution only after workflow evaluation succeeds. Cleanup behavior and compatible unload limitations are the same as `run-agent`.

After the workflow exits, inspect its durable history by root task UUID.

```powershell
.\target\release\vespera.exe workflow-history --database data/workflow.sqlite3 --task 77777777-7777-4777-8777-777777777777
```

This command returns JSON with `plan`, `report` and `interrupted`. Use the
same path originally passed to `--workflow-store`. See [workflow history
semantics](ORCHESTRATION.md#durable-workflow-history).

To continue an eligible interrupted workflow, add `--resume-workflow`
to the original invocation. Completed steps retain their committed results.
The original workflow deadline still applies.

To decompose and execute a task, configure the coordinator with the
`delegate` grant and a model that supports `text-generation`.

```powershell
.\target\release\vespera.exe run-workflow --config config/local.private.toml --decomposition-file config/decomposition.example.json --resume --unload-after --json
```

Use the request's worker roster to select configured agents. The JSON
response includes the validated plan when planning succeeds. [Decomposition
and authorization](ORCHESTRATION.md#automatic-decomposition)

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
| `remember-task` | Required `--task-store <PATH>`, `--task <UUID>` and `--id <UUID>`. Copies the owned, completed task's exact response into episodic memory. `memory_write`. |
| `get` | Required `--id <UUID>`. `memory_read`; returns record and lineage, or JSON `record: null` and nonzero status when absent. |
| `list` | Optional structured query flags below. `memory_read`. |
| `search` | Required `--query-file <PATH>` plus the structured query flags. `memory_read`; literal phrase search. |
| `usage` | No action options. `memory_read`; only this namespace's logical record and byte counts. |
| `consolidate` | Required `--content-file <PATH>`, repeatable `--source <UUID>` (1–32 distinct owned records), `--method <LABEL>` (1–128 UTF-8 bytes); optional `--id`, `--layer` (default `long_term`), `--created-at-unix-ms`. Both grants. Stores supplied text and lineage; runs no summarization model. |
| `delete` | Required `--id <UUID>`. Both grants. Removes an unreferenced record and FTS entry atomically; returns `memory_absent` for an already absent record too. Referenced sources cannot be deleted before their derived records. |

Content files must contain nonempty UTF-8 up to 64 KiB. Query files must contain a nonempty UTF-8 phrase up to 256 bytes and at most 32 whitespace-separated terms. Layers include `long_term`, `episodic`, `working`, `relationship`, `goal` and `observation`. Timestamps must fit a nonnegative SQLite signed integer. `add` accepts observation and import origins. `remember-task` verifies generated provenance against the task database. A derived record is created through `consolidate`.

```powershell
# Set $taskId to a completed task UUID and keep $memoryId if you retry.
$memoryId = [guid]::NewGuid()
.\target\release\vespera.exe @memoryArgs remember-task --task-store data/agent-tasks.sqlite3 --task $taskId --id $memoryId --json
```

Use the same owner UUID and private task database used for execution. The
memory database must already exist. Use `memory get --id $memoryId` to verify
the record after a conflict or interrupted command. Reusing the memory UUID
leaves the prior record intact.

Place `--event-journal <PATH>` before `add`, `remember-task`,
`consolidate` or `delete` to publish the committed mutation to an existing
local journal. A journal failure leaves the mutation committed and returns
nonzero. See [memory event recovery](#recover-memory-mutation-events).

| `list` / `search` filter | Bound/default |
| --- | --- |
| `--layer <LAYER>` | Optional layer |
| `--after-sequence <N>` | Owner-local cursor; default 0; use `next_after_sequence` from the preceding page |
| `--created-since-unix-ms <N>` | Optional inclusive timestamp |
| `--created-before-unix-ms <N>` | Optional exclusive timestamp |
| `--limit <N>` | 1–100 records; default 100 |
| `--max-content-bytes <N>` | 65,536–4,194,304 bytes; default 1,048,576; metadata is separate |

Only `init` can create storage. Read operations open SQLite read-only; existing-file write operations do not create or migrate a missing or empty database. Denied permissions are checked before content/database access. JSON output carries `agent_id` and the relevant `record`, `page`, `usage`, `database_ready`/`sqlite_version`, or `memory_absent` fields. Ordinary failures are nonzero with sanitized stderr; no private path, content excerpt or raw SQL is included. File/database work uses a blocking worker and is outside inference timeout flags. See [MEMORY.md](MEMORY.md) for schema, quotas and recovery limits.

## Memory backup and restore

```powershell
.\target\release\vespera.exe memory-admin backup --database data/vespera.sqlite3 --output data/vespera-backup.sqlite3
.\target\release\vespera.exe memory-admin restore --backup data/vespera-backup.sqlite3 --database data/vespera-restored.sqlite3
```

Both commands report verified completion in JSON. The destination must be a
new file in an existing private directory on a filesystem that supports hard
links. Existing files and SQLite sidecars are rejected. To use the restored
copy, point the usual memory commands or inference memory options at the new
database path.

These commands copy every namespace. They are intended for a trusted filesystem
operator and use no agent configuration or provider. Keep backup files private.
Back up the separate agent task and workflow databases independently when
preserving their provenance. [Snapshot behavior and limits](MEMORY.md#backup-and-restore)

## Opt-in memory during inference

Use these options on `run-agent` after initializing a database with `memory init`.

| Option | Behavior |
| --- | --- |
| `--memory-query-file <PATH>` | Read a nonempty UTF-8 literal phrase of at most 256 bytes. Requires `memory_read`. |
| `--remember-result` | Store accepted output as generated episodic memory. Requires `memory_write`. |
| `--memory-database <PATH>` | Existing database used by either option. Defaults to `data/vespera.sqlite3`. |

Retrieval examines the first eight matches in namespace order. Whole records that fit within an 8 KiB serialized JSON message are sent as untrusted user content before the original task prompt. This is a byte limit. Provider tokenization and context handling still apply. The JSON execution report lists included IDs in `memory_context_ids` when at least one record is attached. The stored task snapshot keeps the original prompt.

A retained response carries the accepted task ID and appears as `stored_memory_id` in the JSON report. Storage failure returns nonzero with `memory_error` and preserves the completed execution evidence. File and database preparation and post-inference persistence run on blocking workers outside the inference deadline.

## Persistent world simulation

Each command takes `world --database <path>` before its subcommand. Create the database's parent directory first. Initialization rejects a database that already contains a world.

```powershell
New-Item -ItemType Directory -Force data | Out-Null
cargo run --locked -p vespera-app -- world --database data/world.sqlite3 init --spec config/world.example.json
cargo run --locked -p vespera-app -- world --database data/world.sqlite3 advance --ticks 60
cargo run --locked -p vespera-app -- world --database data/world.sqlite3 inspect
```

| Subcommand | Inputs and behavior |
| --- | --- |
| `init` | `--spec <path>` reads a strict JSON world up to 64 KiB |
| `inspect` | Returns committed public world state as JSON |
| `advance` | `--ticks <1..1000>` defaults to one. All requested ticks commit together. Optional `--event-journal <PATH>` publishes the advance |
| `set-active` | `--resident <UUID> --active <true/false> --revision <n>` suspends or reactivates a resident |
| `move` | `--resident <UUID> --x <n> --z <n> --revision <n>` moves an active resident to an adjacent cell |
| `memory` | `--resident <UUID>` reads that resident's last 24 recorded actions or encounters as the local database operator |

The specification sets a seed, grid dimensions of 2 to 64 cells per axis, one depot, 1 to 32 residents and up to 64 resource sites. Roles are `forager` and `builder`. Seed and tick determine a rotating resident turn order. Foragers gather from the nearest stocked site, carry up to four units and deliver to the depot. Builders spend six depot units per shelter. Active residents within one cell of each other accumulate encounter counts. Sites regenerate per tick. [Example world specification](../config/world.example.json)

Logical ticks are independent of wall time. A control increments the revision without advancing the tick. Controls reject stale revisions, and an inactive resident stays in place while the rest of the world advances. Public snapshots omit resident memories. The local database is operator-owned and its filesystem permissions govern access.

### Graphical world controls

Launch `cargo run --locked -p vespera-view --features graphics -- --world data/world.sqlite3` after initialization. The window is read-only by default and reads committed state from the same database used by the headless CLI. Changes from another local process appear on refresh. Append `--allow-world-control` to enable writes. Space commits a tick and P toggles one requested tick every half second while the database keeps up. A failed storage operation stops playback. The UI uses the last observed revision for writes and reports stale controls.

Left-click a capsule to inspect the resident's role, goal, activity, position, inventory and encounter counts. Green cylinders show resource sites. The depot block reflects the number of shelters, with its display height capped at 20 shelters. Residents on occupied cells use display offsets to remain visible. The inspector reports their actual grid coordinates and counts. Encounter links appear for the selected resident. Press Enter to toggle its activity or use IJKL for a one-cell move. The graphical view omits resident memories.

### Model-assisted resident decisions

Set up a model with known scheduler resource estimates and a configured agent whose UUID matches the world resident. Grant that agent `simulation_act` in its configuration. The resident must be active in the world. Read the current revision with `world inspect` or the 3D view.

```powershell
$world = 'data/world.sqlite3'
$state = cargo run --quiet --locked -p vespera-app -- world --database $world inspect | ConvertFrom-Json
$resident = $state.residents[0].id
cargo run --locked -p vespera-app -- world --database $world decide --resident $resident --revision $state.revision --config config/local.toml
```

The command previews by default. Add `--apply` to commit the choice from that invocation. If an inference preview is followed by a separate apply invocation, the model runs again. `--resume` activates a suspended or failed runtime agent for this invocation. `--timeout-seconds` accepts 1–120 and defaults to 30. `--unload-after` requests cleanup of idle provider reservations after execution. Review `committed_revision` in the JSON report if cleanup fails after a committed action.

The model receives a small set of simulation facts and a legal action menu. It returns one zero-based JSON choice. The engine validates that choice and commits it against the same world revision. An intervening tick, movement or activity change makes the choice stale. Successful application advances the revision by one and preserves the tick. The world records the latest choice with task, model and provider provenance. Raw generated text and owned memories stay out of this report and the 3D inspector. Deterministic ticks can continue independently. Older Vespera builds can read unchanged worlds and require an update to read worlds containing committed model decisions.

### Run a bounded reasoning schedule

Use `world run` to advance a fixed number of ticks and request one model choice after each group of ticks. Repeat `--resident` to rotate through an approved roster in the supplied order. Each UUID must match an active world resident and a configured agent with `simulation_act`. All selected agents share the run's scheduler.

```powershell
$state = cargo run --quiet --locked -p vespera-app -- world --database $world inspect | ConvertFrom-Json
cargo run --locked -p vespera-app -- world --database $world run --revision $state.revision --ticks 12 --reason-every 4 --resident $($state.residents[0].id) --resident $($state.residents[1].id) --config config/local.toml --resume
```

`--ticks` and `--reason-every` accept 1–1000. Choose values that schedule 1–32 model requests per run. `--tick-ms` adds a delay of 0–5000 ms before each tick and defaults to 200. `--timeout-seconds` applies to each model request, accepts 1–120 and defaults to 10. `--resume` activates configured agents once at startup. `--unload-after` requests cleanup of idle model reservations at the end.

This command commits ticks and legal choices. At each reasoning boundary it waits up to the request deadline before resuming ticks. Invalid choices, provider failures and request deadlines appear in the attempt report, and the run continues with deterministic ticks. A competing world writer can make a pending action or next tick stale. The run then stops, preserving commits already made. Ctrl+C also stops the run and reports its committed progress. Use the read-only 3D window to observe changes from the running CLI.

Add `--event-journal <PATH>` to publish each committed tick to an existing
local journal. A publication failure stops the run and reports the last
committed revision. See [world event recovery](#recover-a-world-advance-event).

In World Mode, an amber outline marks a resident preparing a model request, which can include health checks, queueing and loading. A cyan outline marks an observed active local inference lease. Select the resident to inspect the phase and task. This is runtime evidence alongside simulated state. It describes local activity, and provider-side progress is unavailable. Short phases can fall between samples. The status leaves the view after completion, a revision change or two seconds without a heartbeat. The last committed choice stays in the resident inspector. This live display applies to scheduled runs.

### Replay a committed world revision

```powershell
cargo run --locked -p vespera-app -- world --database data/world.sqlite3 replay --revision 3
cargo run --locked -p vespera-view --features graphics -- --world data/world.sqlite3 --world-revision 3
```

Replace 3 with a revision committed in your world. Omit `--revision` on the CLI to replay the latest revision. The response includes the reconstructed world, its basis revision and the earliest and latest known revisions. The 3D window opens on the requested revision and labels it historical. Press T to freeze the displayed revision during live observation or return to live from history. Press [ for the previous retained commit and ] for the next retained commit while browsing history. The inspector shows the retained range and identifies jumps across unavailable revisions. Selection and camera controls carry across steps. Historical mode disables world controls and pauses playback initiated by the window. An external writer can continue, and T refreshes the display from the latest committed state.

Tick batches, resident activity and movement controls, and committed model choices are recorded in the same SQLite transaction as the world update. Replay applies those recorded changes to a retained checkpoint without inference or network access. It can select a tick batch's final revision. Intermediate revisions inside that batch were never committed separately. Failed changes and transient reasoning heartbeats do not enter replay.

History keeps a rolling checkpoint and up to 512 commits within a span of 4,096 revisions. An older database starts with its current revision as the first available checkpoint. A writer that predates recording can also leave gaps. An unavailable revision returns an explicit error.

### Reproduce the world scale check

```sh
cargo test --locked --release -p vespera-world --test scale_replay -- --nocapture
```

The test generates a seeded 64×64 world with 32 residents and 64 resource sites. It compares 5,120 direct engine ticks against 40 separate SQLite commits and checks replay before and after the retention boundary. Output reports direct engine time, SQLite commit time including tick execution, latest replay time and database bytes. The check requires no model or graphics device.

### Compare two world policies

```powershell
cargo run --locked -p vespera-app -- experiment-world --spec config/world.example.json --evidence-dir target/world-policy-evidence --ticks 60 --seeds 3 --suspend-role forager --target-shelters 5 > target/world-policy-report.json
```

`experiment-world` compares an all-active baseline with a run where selected residents of one role are suspended from tick zero. `--suspend-role` accepts `forager` or `builder`. `--suspend-count` defaults to one, accepts 1–31 and selects matching residents in specification order. At least one resident must remain active. `--ticks` accepts 1–1,000 and `--target-shelters` accepts 1–256. `--seeds` defaults to one and accepts up to the number of residents, with a maximum of 32. Each paired run uses the same world and seed in both arms. Successive seeds rotate the starting resident. The runner alternates which arm executes first to reduce timing order effects.

The command writes JSON to stdout and creates a new evidence directory containing one SQLite world per arm per seed. The parent directory must exist. Existing evidence directories are refused. The report includes the full experiment configuration with canonical resident names and IDs, each trial’s final simulated resources and shelter count, target hits and misses, and the observed range of paired shelter differences. Each arm names its database, committed revision and advance event UUID. Use `world replay --revision` with the indicated database and revision to inspect a committed outcome.

Export `configuration.world` as JSON and supply the recorded parameters with a fresh evidence directory to repeat the deterministic metrics. Execution timings, process memory readings and event UUIDs vary between runs. Timing includes the policy and tick commits, with the initial database creation outside the measurement. Process memory is a single end-of-run reading. The report records no inference latency or GPU measurements for this offline experiment. The seed range describes the configured simulation, and it does not support a population-level statistical claim. If a trial encounters a storage error, the command exits unsuccessfully and completed evidence databases remain available.

### Compare two live model bindings

Register two models on native loopback Ollama and select one configured agent. Save a private UTF-8 JSON workload with 1–16 cases. Each case has exactly two string fields.

```json
[
  { "prompt": "What is 3 plus 4? Reply with the number only.", "expected": "7" }
]
```

```powershell
cargo run --locked -p vespera-app -- experiment-models --config config/local.private.toml --agent $agent --baseline-model $baseline --candidate-model $candidate --workload data/model-cases.json --evidence-dir target/model-comparison --resume > target/model-comparison.json
```

The workload is limited to 128 KiB. A prompt can be up to 16 KiB and expected text up to 4 KiB. Each case runs once per arm in a fresh runtime with the same agent profile, with the order alternating by case. The runner fixes the model binding for that trial, checks provider residency before admission, and requests unload of locally owned reservations after execution. Existing evidence directories are refused. Use `--model-catalog <PATH>` to load registrations from a catalog. The defaults are 512 context tokens, 32 output tokens and 30 seconds per trial. `--context-tokens` accepts 64–32,768, `--max-output-tokens` accepts 1–1,024 and `--timeout-seconds` accepts 1–120. `--resume` activates a suspended or failed agent within each trial.

Scoring is case-sensitive exact text comparison after trimming outer whitespace. The report keeps separate counts for planned trials, attempts, matches, mismatches, failures, deadlines and interruptions. A timeout, interruption or unconfirmed cleanup stops the comparison and returns a nonzero exit code with the observed report. End-to-end timing includes startup residency checks, admission and inference. Provider execution counters and post-task model residency are reported separately when available. Reported model digests and provider versions are observed around each trial. They do not pin the backend or make inference deterministic. An unavailable measurement is null.

Keep the workload and configuration to rerun the comparison. The JSON and the journal omit prompts, expected answers, model output, provider-native names, endpoints and credentials. The journal retains terminal observations with task IDs and cursors, and a trial that never reached a terminal task can have a null cursor. Use `events --journal target/model-comparison/events.sqlite3 replay` to inspect the observed events. Compare results within the stated cases and resource bounds.

### Compare live retrieval with a fixed model

Use `--memory-database <PATH>` in place of `--candidate-model`. Both arms use `--baseline-model` and the same agent. Give the agent `memory_read` and add a private `memory_query` phrase to each workload case. Each phrase can contain up to 256 UTF-8 bytes and 32 whitespace-separated terms.

```json
[
  {
    "prompt": "What is the recorded laboratory code? Reply with only the code.",
    "expected": "K84Q",
    "memory_query": "laboratory code"
  }
]
```

```powershell
cargo run --locked -p vespera-app -- experiment-models --config config/local.private.toml --agent $agent --baseline-model $baseline --memory-database data/vespera.sqlite3 --workload data/memory-cases.json --evidence-dir target/memory-comparison --resume > target/memory-comparison.json
```

The baseline runs without memory. The candidate adds bounded, untrusted context from literal phrase retrieval within the agent’s own namespace. Retrieval completes before provider work and reads the database without writing. Use a stable private copy of the database if other processes may change the source during preparation. The report includes retrieval duration separately from execution time and counts records actually included in successful requests. A miss can include zero records. The report and journal omit memory text, queries, record IDs and the database path. The candidate sends the authorized context to the configured local Ollama provider.

### Compare two live workflow plans

```powershell
cargo run --locked -p vespera-app -- experiment-workflows --config config/local.private.toml --baseline-plan data/baseline-workflow.json --candidate-plan data/candidate-workflow.json --workload data/model-cases.json --evidence-dir target/workflow-comparison --resume > target/workflow-comparison.json
```

Use two authored workflow JSON files and the same private `prompt` and `expected` workload format as [the live model comparison](#compare-two-live-model-bindings). Each case replaces the root prompt of both plans and receives a fresh root task ID per arm. The plans retain their own coordinator, candidates, dependencies, output policy and limits. The normal delegation and scheduling rules apply. Each arm runs in a fresh runtime, and arm order alternates by case. Models in scope must use native loopback Ollama. `--config` defaults to `config/local.toml`. Optional `--model-catalog <PATH>` supplies registrations. `--resume` enables activation for the trial. The outer timeout defaults to 60 seconds and accepts 1–120 seconds through `--timeout-seconds`.

The report scores the output selected by each plan using case-sensitive exact comparison after trimming outer whitespace. It reports planned and attempted trials, failures, timings, actual attempts and provider observations. Plan shape is exported using numbered steps. Prompts, expected answers, output text, step names and endpoint details stay private. Post-workflow residency is a point sample, and a model unloaded earlier in the workflow can have no residency observation. Backend execution remains nondeterministic. Use a fresh evidence directory. Its `events.sqlite3` stores observed attempt, dependency and terminal events and can be opened in Swarm. A deadline, interruption or unconfirmed cleanup stops subsequent trials and returns the partial report with a nonzero exit code.

## External worker service

`vespera serve --access-file <path> [--listen <address:port>] [--broker-db <path>]` starts the broker until Ctrl+C or process exit. It prints the bound URL to stdout. The default address is `127.0.0.1:8765`. Use brackets for IPv6, for example `[::1]:8765`. Port `0` asks the OS to select a port. Workers connect with their own credentials and execute claimed tasks in their own process.

Save this strict JSON as `data/worker-access.json`. The policy references environment variables and grants one worker the `text-generation` capability.

```json
{
  "operator_token_env": "VESPERA_OPERATOR_TOKEN",
  "workers": [
    {
      "id": "73c7d9eb-eddf-418a-b604-ff0cc751ee67",
      "capabilities": ["text-generation"],
      "token_env": "VESPERA_WORKER_TOKEN"
    }
  ]
}
```

This Windows PowerShell 7.2 or later example uses one shell to exercise both roles. In a deployment, give each worker process only its own credential. Generate a distinct signing key from 32 cryptographically random bytes for each principal.

```powershell
$env:VESPERA_OPERATOR_TOKEN = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
$env:VESPERA_WORKER_TOKEN = [Convert]::ToHexString(
    [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
cargo build -p vespera-app --locked
$server = Start-Process .\target\debug\vespera.exe -ArgumentList 'serve', '--access-file', 'data/worker-access.json' -PassThru -WindowStyle Hidden
```

With the service running, submit one task, claim it, post a result, inspect the outcome and read the worker's attempt events.

```powershell
$worker = '73c7d9eb-eddf-418a-b604-ff0cc751ee67'
$task = [guid]::NewGuid().ToString()
$base = 'http://127.0.0.1:8765'
function Invoke-VesperaAction($path, $principal, $key, $action) {
    $session = Invoke-RestMethod "$base/v2/session"
    $issued = $session.elapsed_ms.ToString([Globalization.CultureInfo]::InvariantCulture)
    $nonce = [guid]::NewGuid().ToString()
    $scheme = 'Vespera-HMAC-SHA256'
    $json = ConvertTo-Json -InputObject $action -Depth 4 -Compress
    $prefix = @($scheme, 'POST', $path, $principal, $session.session_id, $issued, $nonce, '') -join [char]10
    $mac = [Security.Cryptography.HMACSHA256]::HashData(
        [Convert]::FromHexString($key), [Text.Encoding]::UTF8.GetBytes($prefix + $json))
    $proof = @($principal, $session.session_id, $issued, $nonce, [Convert]::ToHexString($mac).ToLowerInvariant()) -join ':'
    $request = @{
        Uri = "$base$path"
        Method = 'Post'
        ContentType = 'application/json'
        Headers = @{ Authorization = "$scheme $proof" }
        Body = [Text.Encoding]::UTF8.GetBytes($json)
    }
    Invoke-RestMethod @request
}
$submit = @{
    action = 'submit'
    task_id = $task
    prompt = 'Return a short acknowledgement.'
    required_capabilities = @('text-generation')
    eligible_workers = @($worker)
    lifetime_seconds = 60
    max_attempts = 2
}
Invoke-VesperaAction '/v2/operator' 'operator' $env:VESPERA_OPERATOR_TOKEN $submit
$lease = Invoke-VesperaAction '/v2/worker' $worker $env:VESPERA_WORKER_TOKEN @{ action = 'claim' }
$result = @{ action = 'complete'; fence = $lease.fence; content = 'Acknowledged.' }
Invoke-VesperaAction '/v2/worker' $worker $env:VESPERA_WORKER_TOKEN $result
Invoke-VesperaAction '/v2/operator' 'operator' $env:VESPERA_OPERATOR_TOKEN @{ action = 'inspect'; task_id = $task }
$events = Invoke-VesperaAction '/v2/worker' $worker $env:VESPERA_WORKER_TOKEN @{ action = 'replay'; limit = 16 }
$events.entries.event
Stop-Process -InputObject $server
Remove-Item Env:VESPERA_OPERATOR_TOKEN, Env:VESPERA_WORKER_TOKEN
```

See the [protocol reference](PROTOCOL.md#implemented-loopback-worker-api) for operations, statuses, credentials and bounds.
An optional `message_targets` array on a worker entry grants outbound messaging to those approved worker IDs. [Worker message contract](PROTOCOL.md#worker-messages)

An optional top-level `"operator_mailbox_receipts": true` publishes acknowledged-message metadata to the operator feed. Enable it once operator clients can read event schema 8. [Receipt compatibility](PROTOCOL.md#worker-messages)
For live events, use the Rust SDK or a client that implements the signed WebSocket handshake. [Live subscription contract](PROTOCOL.md#live-websocket-subscriptions)

### Open the native desktop

Launch `vespera-view.exe` without arguments, or run the graphics crate from source.

```powershell
cargo run --locked -p vespera-view --features graphics
```

Home shows OS CPU and available RAM measurements and the renderer name reported by wgpu. Select **Open Hardware** or press **F1** to inspect the host in 3D. Select **Create or resume world** or press **Enter** to open the local simulation. Vespera creates its SQLite database only when you first open the world and reads the saved state on later launches. In World, **Space** commits one tick and **P** toggles playback. **F5** returns to Home, where playback is paused.

Select **Reduced motion** on Home or press **M** to toggle the preference. It applies to camera transitions and Living World poses in the desktop and is saved in the selected desktop data directory. The `--reduced-motion` launch flag enables it for that session independently of the saved preference.

The desktop saves its world in `%LOCALAPPDATA%\Vespera\world.sqlite3` on Windows. On Linux the path is `$XDG_DATA_HOME/Vespera/world.sqlite3` or `~/.local/share/Vespera/world.sqlite3`. On macOS it is `~/Library/Application Support/Vespera/world.sqlite3`. Use a separate directory for a fresh workspace.

```powershell
cargo run --locked -p vespera-view --features graphics -- --desktop-data ./data/desktop
```

The Home world uses deterministic simulation rules. Provider-backed decisions, signed broker observation, and inspection of another world database use the explicit CLI or view options described below. The native Home walkthrough has been validated on Windows.

### Discover local models in the desktop

From Home, select **Models & Providers** or press **F6**. Select **Ollama** for the native adapter at `http://127.0.0.1:11434`, or **OpenAI-compatible** for the compatible adapter at `http://127.0.0.1:1234/v1`, the usual LM Studio endpoint. The server must already be running. For a different loopback address or port, select **Edit endpoint**, type the URL and press **Enter** to save it for the session. **Delete** clears the field while editing and **Esc** cancels the edit.

Select **Test & discover** or press **Enter** to check health and fetch the model inventory. Press **Esc** or select **Cancel work** to stop the local request and return to the discovery controls. The panel shows the observed health, round-trip time, provider model IDs and adapter support for metadata, load, unload and residency. Use **Previous page** and **Next page** to browse results. **F5** returns Home.

Closing the desktop window requests cancellation of active discovery, tasks, or workflows and waits for the provider panel's worker result. If work is still pending, the window title shows **Finishing local work**. Forced process termination still relies on durable restart recovery.

The panel connects to loopback endpoints. For a server that requires authentication, set the secret in the desktop's launch environment, then choose **Credential reference** and enter its variable name. Desktop references start with `VESPERA_` and use uppercase ASCII letters, numbers and underscores. Enter the name alone. The adapter reads its value from the process environment. Changing the endpoint or adapter clears the discovery reference. To save a reference for tasks, register the discovered model or rebind the selected agent. The saved profile contains the variable name and requires the value again on later launches.

On Windows, select **Windows credential** to display the exact target for the chosen adapter and endpoint. In Windows Credential Manager, open **Windows Credentials** and add a **generic credential**. Use the displayed target as the network address, `vespera` as the username, and the provider token as the password. Then run **Test & discover** and register or rebind the agent to retain the source choice. Rotate or remove the token in Windows Credential Manager. Vespera reads the token when it creates the adapter. A missing or unreadable credential stops adapter initialization.

For the Windows CLI, set `credential_windows = true` in the provider declaration and leave `credential_env` unset. The target includes the adapter and a SHA-256 digest of the exact endpoint text. This keeps paths with different letter case distinct in Windows Credential Manager. Use PowerShell 7 to print the target.

```powershell
$adapter = 'ollama' # or 'openai-compatible'
$endpoint = 'http://127.0.0.1:11434' # use the exact configured text
$digest = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($endpoint))
$target = "Vespera/provider/$adapter/$([Convert]::ToHexString($digest).ToLowerInvariant())"
$target
```

This source is limited to loopback endpoints. Profile backups retain the source choice and endpoint. A restored profile needs its credential provisioned in the Windows account where it will run.

In PowerShell 7.2 or later, launch the app from the same shell where you read the token.

```powershell
$env:VESPERA_LOCAL_TOKEN = Read-Host -MaskInput 'Provider token'
cargo run --locked -p vespera-view --features graphics
Remove-Item Env:VESPERA_LOCAL_TOKEN
```

```bash
read -rsp 'Provider token: ' VESPERA_LOCAL_TOKEN
printf '\n'
export VESPERA_LOCAL_TOKEN
cargo run --locked -p vespera-view --features graphics
unset VESPERA_LOCAL_TOKEN
```

Discovery results and unregistered endpoint or credential-reference edits live in the current app session. Clear the credential field and press **Enter** for an unauthenticated server.

### Register a model and run a desktop agent

Choose **New agent**, then press the number beside a text model in the discovery results. Enter an agent name, role, positive RAM estimate in MiB, and VRAM estimate in MiB. Press **Enter** after each field. Use zero VRAM for a CPU-only estimate. These estimates set the task's scheduler memory ceilings, with available host RAM applied as an additional limit. Agents sharing one model registration share its estimates. For Ollama, registration checks the model's reported text-generation capability. On the compatible interface, choosing a model declares that it supports text generation.

Select **Run agent task** or press **P** while the panel has no active editor. Enter a prompt and press **Enter** to submit one task with a 128-output-token limit and a 50-second execution deadline. **Esc** and **Cancel work** request local cancellation. The panel shows the result or failure and reports local cleanup. Native Ollama checks existing residency before admission and requests cleanup of models loaded by the runtime. The compatible path leaves residency and explicit unloading to the provider.

The desktop saves up to 16 agents in `desktop.sqlite3` under the desktop data directory. Use **Previous agent** and **Next agent** to select the agent that will run the task. To keep its identity and bind it to another model, select **Rebind selected**, then choose the model. Tasks use the selected agent's saved endpoint even if discovery is pointed elsewhere. New agents start without optional grants.

Select **Task history** to see the selected agent's retained tasks, newest accepted first. Use the page controls to browse and press a task number for a prompt and response preview. The task database `desktop-tasks.sqlite3` in the same directory contains private prompts and outcomes. Pass that database path and the task ID to `agent-task-history` to inspect the full checkpoint. [Durable task ownership](AGENTS.md#durable-task-ownership)

Use **Suspend** and **Resume** to set or clear the selected agent's durable operator hold. A held agent cannot start a task. Use **Allow delegation** or **Revoke delegation** to change the agent's delegation permission. The grant is saved with the agent profile and is separate from its hold.

### Run a delegated desktop workflow

Select the drafting agent, choose **Allow delegation**, and open **Draft + review** or press **G** while the panel has no active editor. Enter the number of a different saved agent on the same provider, then enter the draft prompt and review instruction. Press **Enter** after each field. Both agents must be available for work.

The orchestrator runs the draft first and sends its accepted output to the reviewer as untrusted input. Each step gets one attempt and at most 128 output tokens. The workflow has a 100-second execution budget. Press **Esc** or select **Cancel work** to request local cancellation. The panel reports the workflow ID, step owners, observed task outcomes, and the selected review response. The desktop stores private workflow checkpoints in `desktop-workflows.sqlite3`. Use `workflow-history` with that database and the displayed ID to reopen the full report after exiting. Individual steps also appear in their owners' desktop task histories.

Press **F3** during the run to follow committed desktop workflow observations in Swarm, or use **Open Swarm** from Home. The desktop projects the retained audit into `desktop-events.sqlite3` without copying prompts or responses. On startup it restores missing observations from the workflow store. **T** freezes the display or returns live. While frozen, **[** and **]** step through captured observations. The workflow store remains the recovery source if the journal cannot be written.

<a id="author-a-sequential-desktop-workflow"></a>

### Author a desktop workflow

In **Models & Providers**, select a saved coordinator, grant it **Allow delegation**, and choose **Build workflow** or press **C**. Enter a count from two to six. The selected coordinator owns the first step. For every later step, select a saved agent on the same provider and enter its prompt. The same agent can own more than one step.

Press **Enter** after each field, then review the assignments and prompt previews. Press **Enter** again to run the plan or **Esc** to discard it. After each later prompt, enter comma-separated earlier step numbers as inputs. Blank selects the immediately preceding step; `0` makes the step independent. For a four-step branch and join, use `1` for steps 2 and 3, then `2,3` for step 4. Forward references and duplicates are rejected. Review shows each input list. Selected results are supplied as untrusted input; dependency branches do not imply concurrent inference. The last step supplies the final result. Each step has one attempt, a 128-output-token limit and a 50-second deadline. The desktop records the run in the same workflow history and Swarm journal as **Draft + review**.

Branch selection is implemented and covered by state and orchestration tests. The desktop execution path also completed a four-step live Ollama branch/join and reopened its checkpoint; a native graphical branch-authoring walkthrough remains an acceptance gate.

For an opt-in execution rehearsal, set `VESPERA_TEST_BRANCH_DATA` to an isolated desktop data directory with a saved roster, a delegation-enabled coordinator and a running configured model provider. The test creates tasks, workflow history and event evidence in that directory; it does not drive the graphical editor.

```sh
cargo test --locked -p vespera-view --features graphics --bin vespera-view live_desktop_branch_join_reopens_committed_plan_and_evidence -- --ignored --nocapture
```

Use **Ctrl+V** in a task or workflow prompt to append text from the system clipboard. The editor flattens line breaks and tabs into spaces and applies the same 1,200-character limit as typing. Pasting keeps the prompt in the editor until you submit it.

### Browse retained desktop workflows

Open **Workflow history** from Home or press **F7**. Use **Newer workflow** and **Older workflow**, or **Up** and **Down**, to inspect retained checkpoints in acceptance order. The panel shows the root ID, status, coordinator, step owners, and a preview of the selected response. Nonterminal records are marked interrupted after the writer exits. If the database is busy, use **Refresh** or **R** after the writer finishes. The history reader does not start providers.

Select **Open in Swarm** or press **Enter** to focus the journal view on the selected root workflow. **Backspace** clears the focus. Use **F7** to return to Workflow history. The CLI's `workflow-history` command prints the full private checkpoint.

### Switch layers in Atlas

Open Atlas without credentials to inspect the host in Machine. Add the sources you want available in the same window.

```powershell
cargo run --locked -p vespera-view --features graphics -- --atlas
cargo run --locked -p vespera-view --features graphics -- --atlas --broker 127.0.0.1:8765 --world data/world.sqlite3
```

| Key | Layer | Source |
| --- | --- | --- |
| F1 | Machine | Local host and optional `--scheduler-telemetry <DIRECTORY>` |
| F2 | Intelligence Network | `--broker <IP:PORT>` and the operator signing key |
| F3 | Swarm | `--swarm <JOURNAL>` for local workflow evidence, or the configured broker for accepted messages |
| F4 | Living World | `--world <DATABASE>` for an initialized world |

The footer marks the active and unavailable layers. Each configured layer keeps its camera, filter and replay position. A selected worker, Swarm participant or world resident transfers to the destination when the same UUID is visible there. Each layer keeps its selection when the destination has no matching identity.

For a local responsiveness sample, add `--measure-frames <60..3600>` to Atlas. After 120 warm-up updates at the current layer and physical window size, it writes one JSON record to stdout. Changing layers, resizing or losing focus restarts the sample. The report includes the adapter, backend, resolution, requested present mode, update interval percentiles and process resident memory when the OS exposes it. Update intervals measure the time between Bevy application updates. Process resident memory covers the whole view process. GPU frame time, input-to-display latency and GPU memory require separate measurements.

```powershell
cargo run --locked -p vespera-view --features graphics -- --atlas --world data/world.sqlite3 --measure-frames 600
```

Machine shares the selected task's agent UUID with other layers. Switching back from another layer selects a Machine task when exactly one displayed lease belongs to that agent.

The window observes configured sources in the background. Leaving World pauses window-driven playback, and leaving Intelligence Network dismisses any cancellation confirmation that has yet to be sent. Controls are read-only by default. Use `--allow-cancel` with `--broker` or `--allow-world-control` with `--world` to enable the corresponding live controls. `--world-revision` opens the World layer at a retained revision with read-only controls.

### Intelligence Network controls

Run `cargo run --locked -p vespera-view --features graphics -- --broker 127.0.0.1:8765` with the operator key in `VESPERA_OPERATOR_TOKEN`. The view groups the latest observed states into queued, running, complete, failed and cancelled lanes. Violet spheres represent workers that appear in lease events. Cyan links identify running attempts. Selecting a task or worker can reveal its dimmed links to the latest recorded attempts. The activity list shows recent observations and their feed sequences. Counts reflect the loaded observations.

Press Tab to cycle all tasks, queued and running work, and terminal outcomes. N cycles visible tasks and B cycles workers. Left-click picks a visible object. Use right-drag, wheel and WASD to navigate, F to focus, and R to reset. `--reduced-motion` skips animated camera focus and task lane transitions.

Press T to freeze the Intelligence Network or return live. Press [ to freeze and step back, then use [ and ] to move across captured task transitions. The header shows the viewed sequence, captured range and live position. The window can replay up to 256 locally observed task transitions from a rolling baseline. The capture survives a feed reset. Returning live switches to the new feed.

The view is read-only by default. In live mode, `--allow-cancel` enables the C confirmation prompt for queued or running tasks. Press Y to send the signed request or Esc to dismiss. Freezing dismisses any confirmation that has yet to be sent. The inspector displays pending, returned or uncertain outcomes for requests already sent. The task lane changes when the event feed reports the new state. Confirmation and the queued request are bound to the observed broker session. A restart dismisses confirmation or reports that cancellation was not sent; reconfirm against the current broker. Authorization rejection or invalid event data stops the subscription and disables cancellation until the connection is reconfigured and the application is relaunched. Closing the view drops pending broker I/O and reconnect waits. An interrupted cancellation has an unknown remote outcome; inspect the broker before acting again.

### TLS worker transport

Atlas, Intelligence Network and broker-backed Swarm can use the same verified
TLS transport. Supply an explicit numeric broker address and a PEM trust bundle:

```powershell
cargo run --locked -p vespera-view --features graphics -- --atlas --broker "${interfaceIp}:8765" --broker-tls-ca $certificatePath
```

In a POSIX shell, use `"${interfaceIp}:8765"` and `"$certificatePath"` after setting
those variables. Keep the operator signing key in `VESPERA_OPERATOR_TOKEN`.
The bundle is limited to 128 KiB and 16 certificates. Certificate identity must
match the broker IP. Omitting `--broker-tls-ca` retains loopback-only HTTP;
TLS failures never fall back to plaintext. The view remains read-only unless
`--allow-cancel` is supplied. `--swarm-broker` selects the standalone message view.

Use `--tls-cert <chain.pem> --tls-key <key.pem>` together to enable HTTPS and secure WebSockets. Use a PEM chain in leaf-first order and protect the corresponding private key file. A non-loopback address additionally requires `--allow-remote`. For example, with a certificate valid for the configured interface address, run the following command using that address and the paths to your PEM files.

```powershell
cargo run --locked -p vespera-app -- serve --access-file data/worker-access.json --listen "${interfaceIp}:8765" --tls-cert $certificatePath --tls-key $privateKeyPath --allow-remote
```

For loopback TLS, keep the default address and omit `--allow-remote`. Certificate verification and request authentication both apply. [Transport and SDK trust configuration](PROTOCOL.md#protected-transport)

### Durable broker mode

With the access file and environment variables configured, supply a path in a private directory on local storage. The parent directory must already exist.

```powershell
cargo run --locked -p vespera-app -- serve --access-file data/worker-access.json --broker-db data/worker-broker.sqlite3
```

Use the same database on each restart to recover queued tasks, terminal outcomes, generations, revocations, recent event feeds and authorized worker inboxes. An authorized client can resume with its saved event cursor. The broker creates the database when needed and uses it exclusively. A second server using the same database exits with a startup error. Protect the database and its SQLite journal with filesystem permissions because they contain prompts, results and pending messages. If an authenticated action returns `storage_unavailable`, resolve the database error and restart with the same file before reconciling the result. The default without `--broker-db` uses process memory. [Recovery contract](PROTOCOL.md#durable-broker-recovery)

## Machine view

Run `cargo run --locked -p vespera-view --features graphics -- --machine` without a broker, provider or key. Click a component or press 1 for CPU, 2 for memory, or 3 for the renderer adapter. Right-drag to orbit, scroll to zoom, use WASD to pan, F to focus and R to reset. Use `--reduced-motion` to skip animated focus and reset.

The window samples host CPU and RAM roughly once per second. The CPU inspector shows process-available logical threads and host-wide CPU utilization. The CPU meter starts after a utilization baseline. The memory meter shows the fraction of physical RAM currently unavailable to applications according to the OS. Host RAM can exceed container limits. Missing or stale samples are labeled unavailable and their meters are hidden.

The adapter inspector displays the renderer selected by wgpu. On Windows and Linux with NVIDIA NVML available at launch, it separately lists accessible NVIDIA GPUs by PCI address and displays device-wide used, total and free VRAM for the selected device. Press G in the adapter inspector to cycle through the accessible devices. The sampler checks up to 16 NVML devices and reports the driver's count alongside the accessible count. An unavailable driver or memory query is labeled unavailable. These values can include driver reservations and allocations from other processes. Rendering identity, inference ownership and scheduler estimates are separate from the NVML measurements. [NVML memory counters](https://docs.nvidia.com/deploy/nvml-api/latest/api/structnvmlMemory__t.html)

### Observe a local scheduler

Supply `--scheduler-telemetry <DIRECTORY>` to `run-agent` or `run-workflow`. Choose a new directory under an existing private local parent. In a separate terminal, run the view with the same directory. Open Machine before starting a short run to see live samples.

```powershell
cargo run --locked -p vespera-view --features graphics -- --machine --scheduler-telemetry data/observation-001
# In the existing run-agent or run-workflow command, append:
# --scheduler-telemetry data/observation-001
```

Select the amber component or press 4. The meter shows the fraction of granted local admission permits. The inspector separates queued requests, granted permits, estimated RAM and VRAM reservations, uncertain model reservations, and cached provider health from host measurements. Health observations expire according to the runtime's registry policy. Sampling adds no provider requests. The local samples are approximately 250 ms apart, and the window refreshes approximately once a second. Brief transitions can fall between observations.

The inspector also shows the latest scheduler admission that returned for a model. It distinguishes a successful preparation, queue rejection, cancellation, a resource limit and other preparation failures. A resource rejection shows the requested amount and configured limit from that decision. The value describes one local runtime and persists in its final sample. Decisions replaced between samples can be missed. Work rejected before reaching scheduler preparation has no admission record.

Click a model cube or press 5 to cycle through model reservations. Click a violet provider sphere or press 6 to cycle providers represented by those reservations. The model inspector shows its UUID, provider UUID, scheduler lifecycle state, issued leases, charged slots and estimated memory reservation. Cyan links indicate at least one issued local lease in the displayed sample. A slot can be charged while preparation is still in progress. Provider totals cover the displayed reservations. They do not measure provider-side inference.

Click a yellow task capsule or press 7 to cycle through displayed local task leases. The yellow link connects a task to its reserved model. The inspector shows the task, owning agent, lease, model and provider UUIDs. The publisher records ownership only when the agent runtime holds the task and the scheduler still accounts for the same issued lease. Queued tasks and tasks preparing a model have no capsule. A completed task disappears from this live graph.

When `run-agent` includes authorized retrieved memory in a validated inference request, Machine shows a green memory node linked to the task. Click it or press 8 to inspect the count of included records. The task inspector distinguishes zero included records from unavailable request evidence. The count describes local request assembly. Memory text, queries and record UUIDs stay out of the sample.

The CLI replaces one bounded `scheduler.json` in the newly created directory and leaves a final sample after its cleanup attempt. Version 5 adds the latest admission decision. The reader also accepts version 4 request memory counts, version 3 task ownership, version 2 model reservations and version 1 aggregate counts. The window shows up to 16 model reservations, with charged slots prioritized, and up to 16 task owners whose models appear in the sample. A final sample may still show reservations when unload was unrequested or unconfirmed. If the publisher stops before a final sample, Machine marks the last running sample stale. Keep the parent directory private and use the same local filesystem and clock for both processes. Samples carry local accounting and task, agent, lease, model and provider UUIDs. Prompts, responses, provider endpoints, provider-native model names and credentials stay private. Use a different directory for each invocation.

## Local task outcome journal

```powershell
New-Item -ItemType Directory -Force data | Out-Null
.\target\release\vespera.exe events --journal data/events.sqlite3 init --retention 10000
.\target\release\vespera.exe events --journal data/events.sqlite3 replay --limit 100
```

`events` uses `data/events.sqlite3` by default and prints JSON. `init` requires an existing parent directory. Retention is fixed at initialization and accepts 1 to 100,000 events. The default is 10,000. `replay` opens the existing file read-only and returns 1 to 256 records per page. The default is 100. Pass `--after <journal-UUID:sequence>` using the fields of `next_cursor` to continue. Check `has_more` for another page and `discarded_through` for the retention boundary. An expired, foreign or future cursor returns nonzero.

Pass `--event-journal <PATH>` to `run-agent` or `run-workflow` to record an observed terminal outcome in an initialized journal. Successful agent runs emit `inference_completed` and successful workflow roots emit `task_completed`. An observed failed or cancelled agent task or workflow root emits `task_failed` or `task_cancelled` in schema version 2. Early rejections before task creation have no recorded task state. Reports include the committed cursor or an append error in `event_publication`. Append failure returns nonzero while preserving the execution report. Publication is a separate post-execution transaction, so a process exit between execution and publication can leave a gap.

Journal records contain versioned typed metadata, including the event and task IDs. Local access uses filesystem permissions. Consumers may replay from the same journal after restart. Duplicate event IDs are idempotent while retained if the envelope matches. Use `follow` for local streaming and `replay` for discrete pages.

### Runtime lifecycle journal

Initialize an event journal, then add `--lifecycle-journal <PATH>` to
`run-agent` or `run-workflow`. This live feed records acknowledged explicit
model loads and unloads, managed inference entry, and budget rejections at
the scheduler's resource admission gate. The initial events reflect the
accepted model registrations and effective agent statuses scoped to that
invocation. Subsequent status changes are local runtime observations.
Inference entry carries task,
model and agent IDs and records when the validated request enters the local
provider call. Provider-managed models can emit inference entry without an
explicit load or unload event.

Replay or follow the lifecycle journal using the usual event commands.
Reports include `lifecycle_publication` with the last committed cursor,
omission count and any writer error. The producer queues at most 256 events
and continues execution if the consumer is slow or unavailable. A partial
publication returns nonzero with the task and cleanup results. Lifecycle
telemetry is separate from the recoverable task and workflow outcome audit.

### Swarm view

Initialize a private local event journal with the `events init` command above. Add `--record-delegations --event-journal data/events.sqlite3` to a configured `run-workflow` command, then open the view in the same workspace. The view can also be open when the workflow runs.

```powershell
cargo run --locked -p vespera-view --features graphics -- --swarm data/events.sqlite3
```

The workflow publishes only delegations that the runtime accepted. Each record carries the attempt and agent IDs and the root workflow correlation. The delegation records and root terminal event commit in one batch after execution and cleanup. Choose retention large enough to retain the whole batch if you need all its records. The journal stores no workflow prompts or generated outputs in this batch.

For live evidence, append `--observe-workflow --event-journal data/events.sqlite3` to the workflow command. Use one of `--observe-workflow` and `--record-delegations`. Live mode records acceptance and known terminal task state separately. Without a workflow store, it records the root outcome after cleanup. Accepted attempts carry the IDs of predecessor results included in their input. These records establish prompt assembly and delegation, while the attempt outcome describes local task execution. Select the Dependencies filter to isolate green conduits. Interrupted publication can leave an accepted attempt without terminal evidence. Use a unique root task ID for each run.

Add `--workflow-store <PATH>` to commit the original audit envelopes with
the workflow state. Live mode then publishes the committed envelopes as
they become available, including the root outcome once it commits. If
publication stops, use [workflow event recovery](#recover-workflow-events).

Pass the same journal path to `--lifecycle-journal` to show registered
agents and their latest observed runtime status in Swarm. The inspector
shows the status sequence. Frozen replay shows only statuses and outcomes
through the selected sequence. A shared journal displays observations in
append order, including when separate runs observe the same agent UUID.

Click an agent or press N to cycle observed agents. Use F to focus, R to reset, right-drag to orbit, WASD to pan and scroll to zoom. Tab filters links by evidence type. Cyan conduits represent delegations. A local ring represents a same-agent observation. Repeated observations between the same agents share a conduit of their evidence type. The inspector shows the latest matching sequence, peer, attempt and workflow outcome when available. Journal mode follows new committed records without a broker key.

Press G in live mode to focus the selected agent's latest linked workflow.
Press G again to cycle through that agent's other linked workflows in reverse
observation order. Backspace returns to the full journal. Workflow focus keeps
the selected agent when it is part of the filtered evidence.

The Swarm Foundry uses a fixed architectural gallery and faceted agent constructs.
Construct silhouettes and accents derive from agent UUIDs. In the standalone
journal view, append **`--swarm-capture <new-file.png>`** to save a native window
capture after 120 updates following the committed journal. Choose a new file
path in an existing directory.

The short upper beacon shows observed agent runtime status. The wide lower
beacon shows the latest assigned attempt. Amber means acceptance without a
terminal record, green means completed, red means failed, and violet means
cancelled. Runtime suspension also uses violet on the upper beacon. The
inspector lists the corresponding runtime or acceptance sequence. New live
delegations, message enqueues and receipts can send a brief marker along their
conduit. Frozen replay and reduced motion use the static evidence display.

For a synthetic three-agent visual test, generate a new scratch journal and
open it in the native view:

```powershell
cargo run --locked -p vespera-view --features graphics --example swarm-fixture -- target/swarm-fixture.sqlite3
cargo run --locked -p vespera-view --features graphics -- --swarm target/swarm-fixture.sqlite3 --swarm-capture target/swarm-fixture.png
```

This journal contains synthetic event metadata, including a four-step branch
and join and separate failed and open attempts. Use a fresh fixture path.

To inspect a new message in motion, run the fixture writer in one terminal
and open its journal in a second terminal:

```powershell
# Terminal 1
cargo run --locked -p vespera-view --features graphics --example swarm-fixture -- target/swarm-live.sqlite3 --live
# Terminal 2
cargo run --locked -p vespera-view --features graphics -- --swarm target/swarm-live.sqlite3 --swarm-capture target/swarm-transit.png --swarm-capture-transit
```

Once the scene shows the baseline journal, press Enter in terminal 1. The
writer commits a synthetic message, and the app captures the first observed
transit around its midpoint. Transit capture requires motion to be enabled.

For a repeatable close-up and update-interval sample, select a fixture agent
by UUID when starting standalone Swarm:

```powershell
cargo run --locked -p vespera-view --features graphics -- --swarm target/swarm-fixture.sqlite3 --swarm-agent 10000000-0000-4000-8000-000000000003 --reduced-motion --swarm-capture target/swarm-close.png --swarm-measure-frames 600
```

Keep the window focused for the sample. The report describes application
update intervals, adapter and backend, and process resident memory.

To observe live worker messages, use broker mode with the operator signing key in `VESPERA_OPERATOR_TOKEN` and a running loopback broker.

```powershell
cargo run --locked -p vespera-view --features graphics -- --swarm-broker --broker 127.0.0.1:8765
```

Amber conduits show an accepted inbox enqueue from the authenticated sender to an approved recipient. With broker receipts enabled, pink conduits run from the acknowledging recipient back to the sender. The inspector shows the peer, observed operation and signed feed sequence. Journal and broker modes use separate sources. Contention evidence remains roadmap work. [Worker message contract](PROTOCOL.md#worker-messages)

In either Swarm mode, press T to freeze the current evidence window or return live. Press [ to freeze and step back, then use [ and ] to move between retained observations and the captured range boundaries. The inspector displays the viewed sequence and the live source position separately. New events continue to arrive while frozen. An attempt's terminal state appears only once the viewed sequence includes its terminal event.

Both modes are read-only. A window keeps up to 128 relationships, 128 terminal observations and 128 agent status observations, with up to 24 agents visible. The inspector labels the cutoff when source retention or the window limit removes earlier evidence. Replay stays within the captured window, and returning live refreshes it from the current feed.

### Follow committed events

```powershell
.\target\release\vespera.exe events --journal data/events.sqlite3 follow --poll-ms 250
```

Each stdout line contains one JSON entry with its event and cursor. The follower flushes each entry and reads at most one page ahead. It waits between reads when caught up, without holding a read transaction while writing to stdout. Save the last fully consumed cursor and pass `--after <journal-UUID:sequence>` to resume. Without a cursor it starts from the oldest retained entry and reports any earlier discarded history on stderr. If a saved cursor expires while following, the command exits nonzero and reports the retention boundary.

`--poll-ms` defaults to 250 and accepts 10 to 60,000. The command continues until interrupted or until it reaches an optional `--max-events <N>` or `--idle-timeout-ms <N>` bound. The idle timeout accepts 10 to 3,600,000 milliseconds and applies while waiting for new entries. A closed stdout pipe ends delivery. Consumers control how quickly the command can write to stdout, so an idle timeout does not bound writes to a stalled consumer.

### Recover a durable agent terminal event

When `run-agent` uses both `--task-store` and `--event-journal`, it publishes
the exact envelope committed with the task's terminal state. If the journal
write fails, the task and its original event identity remain in the task store.
After the writer exits and the journal is available, publish the stored event.

```powershell
.\target\release\vespera.exe events --journal data/events.sqlite3 recover-agent-task --task-store data/agent-tasks.sqlite3 --task $taskId
```

Use the actual agent task UUID. Recovery works offline and returns the
journal cursor and event UUID. Retrying returns the original cursor while
the entry is retained. If retention has discarded it, explicit recovery can
publish the same event UUID at a new cursor. Replay order is publication
order, so a recovered event can carry an earlier occurrence timestamp.
Interrupted tasks and tasks from older schemas without an envelope cannot
produce a terminal event through this command.

### Recover a world advance event

```powershell
$world = 'data/world.sqlite3'
$journal = 'data/events.sqlite3'
.\target\release\vespera.exe world --database $world advance --ticks 3 --event-journal $journal
.\target\release\vespera.exe events --journal $journal recover-world-advance --database $world --revision $revision
```

Set `$revision` to the committed world revision reported by the advance or
scheduled run. The world database retains its last 512 tick-advancing commits
as metadata-only envelopes. A batched advance records one event with the
final logical tick. Movement, activity changes and model decisions can advance
the revision without advancing the tick. Recover by world revision, which
can differ from the tick shown in the event.

The envelope includes a stable simulation UUID, the logical tick and the
commit timestamp. Recovery keeps the original event UUID. If the journal
already retains the event, the command returns its existing cursor. Older
world revisions written before event storage was added have no envelope.

### Recover agent inbox events

```powershell
$journal = 'data/events.sqlite3'
$tasks = 'data/agent-tasks.sqlite3'
$page = .\target\release\vespera.exe events --journal $journal recover-agent-messages --task-store $tasks --limit 100 | ConvertFrom-Json
.\target\release\vespera.exe events --journal $journal recover-agent-messages --task-store $tasks --after $page.next_source_cursor --limit 100
```

Save `next_source_cursor` when present and continue while `has_more` is
true. Each page has at most 256 envelopes. The cursor is scoped to the task
database. A retry keeps the original event IDs and reuses their journal
cursors while retained. The envelopes contain sender and recipient UUIDs
and an optional task correlation. The message text stays in the private inbox.
Recovery requires trusted filesystem access and runs after the task store
writer exits. Older messages remain readable in the inbox.

### Recover workflow events

```powershell
.\target\release\vespera.exe events --journal data/events.sqlite3 recover-workflow --workflow-store data/workflows.sqlite3 --task $workflowId
```

Use the workflow root UUID after the writer exits. The command publishes the
retained attempt acceptance, known terminal attempt and root outcome events
in their original checkpoint order. It returns the journal cursor and the
number submitted. Retries reuse event IDs while the journal retains them.
An interrupted workflow can have a partial audit. Resuming it preserves the
recorded events and appends new evidence. Journals populated with
`--record-delegations` can also contain their separate summary delegation
events. Workflow rows created before audit support remain readable and have
no recoverable audit.

### Recover memory mutation events

```powershell
$journal = 'data/events.sqlite3'
$page = .\target\release\vespera.exe events --journal $journal recover-memory --database data/vespera.sqlite3 --limit 100 | ConvertFrom-Json
.\target\release\vespera.exe events --journal $journal recover-memory --database data/vespera.sqlite3 --after $page.next_source_cursor --limit 100
```

The first command starts with the oldest retained mutation. When
`next_source_cursor` is present, save it after successful publication and
pass it as `--after` to continue. Each call reads at most 256 events and
reports `has_more` and `discarded_through`. A stale source cursor fails
explicitly. A retry reuses event identities while the destination journal
retains them. Choose a page size within the destination journal's retention.
Journal ordering follows publication order. Recovery can add older
occurrence timestamps after newer ones.

This is a trusted filesystem operation across all namespaces. Envelopes
carry agent and memory UUIDs, and record writes also carry the memory layer.
They omit memory text, lineage and provider data. A deletion uses event schema
version 5. Use a version 5 reader for journals containing these events.
