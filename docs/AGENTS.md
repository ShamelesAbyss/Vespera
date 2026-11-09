# Agent runtime

An agent has an independent UUID, identity, role, personality, goals, replaceable
model policy, permissions and operational state. The Rust runtime registers and
inspects agents, routes by fixed model or declared capabilities, coordinates
owned task execution, and provides permission-gated in-process memory/messages.
It is not yet a persistent autonomous ecosystem or an external-agent server.

## Fixed model requirements

An agent can require capabilities while keeping a fixed model binding. In
configuration, set the policy's `kind` to `fixed_with_capabilities` and provide
the existing `model_id` and a nonempty `required` list. The same binding and
capability check applies when scoping providers, planning workflow candidates,
and resolving a model for execution. Task requirements still apply in addition
to the agent policy. [Desktop capability editor](CLI.md#register-a-model-and-run-a-desktop-agent)

Existing fixed policies continue to load. Earlier Vespera builds cannot read a
profile that contains the new policy. Keep a pre-upgrade profile backup when
qualifying a downgrade.

## Durable task ownership

Pass `--task-store <PATH>` to `run-agent` or `run-workflow` to retain accepted
tasks in a private local SQLite database. The runtime commits the owner before
model preparation. Delegation records both the sender and the assigned worker.
Managed completion commits the outcome, selected model and provider provenance.
The database holds up to 256 tasks and 128 MiB of serialized payloads.
Use a separate path from the workflow store, memory database and event journal.

`run-agent --task-id <UUID>` selects a stable task identity and requires
`--task-store`. Reusing an existing ID fails before provider access. The
runtime generates an ID when the option is omitted.

After the writer exits, use `agent-task-history --database <PATH> --task <UUID>`
to read the stored owner, task and result. A nonterminal checkpoint is reported
as interrupted and retains its last observed state. The store fences replay
of that task ID. The database contains task prompts and generated results.

New terminal checkpoints also store one sanitized event envelope in the
same SQLite update as the task outcome. Use the [event recovery command](CLI.md#recover-a-durable-agent-terminal-event)
to publish that envelope after a journal outage. Prompts and generated
responses stay in the private task database.

## Desktop agent memory

Open the Providers panel with **F6** and select a saved agent. The panel
shows separate read and write grants. Grant write access before opening
**Task history**, select a completed task by number, and choose **Remember
task** to copy the committed response into that agent's episodic memory.
Each retained task can produce one desktop memory record. Its source task
and owner are checked again when the record is saved.

Grant read access and choose **Run with memory** to enter a phrase and a
separate prompt. The phrase is limited to 256 bytes and 32 words. The runtime
passes up to eight matching records, within an 8 KiB context budget, to the
bound provider as untrusted user input. The task checkpoint records the IDs
of the records actually included. Regular tasks and workflows do not
retrieve memory automatically. Rebinding preserves the same agent namespace.

The desktop profile stores these records in `desktop-memory.sqlite3`. Treat
that file and any SQLite sidecars as private data. Changing a grant affects
subsequent operations and leaves retained records available if access is
granted again. See [memory storage and recovery](MEMORY.md).

## Durable operator suspension

`agent-state` manages an operator hold for a configured agent UUID. Use the
same task database for the control command and later `run-agent` or
`run-workflow` invocations. An invocation's `--resume` cannot bypass a hold.
A held coordinator or decomposition roster member prevents execution.
Authored workflows can select another eligible worker when a candidate is held.

The store supports up to 256 holds. Opening a legacy task database for
execution or suspension adds missing tables while preserving task history.
Releasing a hold restores the configured status. The database is exclusive
while an invocation uses it, so issue operator state commands between runs.
Configuration remains the authority for agent definitions, grants and model
policy. Transient runtime status still lives in memory.
See the [CLI reference](CLI.md#operator-suspension).

## Durable inbox

With a task store attached, `AgentRuntime::send_message` commits a message
before returning. `inbox_messages` reopens the recipient's messages in logical
tick order. The sender and reader each need a current `communicate` grant.
The recipient must exist in the current configuration. A held sender cannot
send. A message may reference a stored task when either participant owns it.
Rebinding a model policy preserves messages under the same agent UUID.
New durable messages commit a sanitized envelope with the message. The event
records inbox acceptance and can be [recovered after a journal outage](CLI.md#recover-agent-inbox-events).

The store keeps up to 256 messages per recipient, 4,096 messages total,
64 KiB per message and 64 MiB of message text across the database. Lower runtime
limits also apply to new writes. At capacity, sends fail and retained messages
stay available. Inboxes are local, append-only and private. The offline CLI
treats the operator as trusted and uses the supplied configuration to authorize
the claimed agent UUID. See [CLI inbox operations](CLI.md#agent-inbox).

In the desktop Providers panel, **Toggle communication** manages the selected
agent's grant. **Send message** saves a local message to another saved agent.
**Inbox** opens the selected agent's messages and supports paging. Desktop
messages and their audit share `desktop-tasks.sqlite3`. Swarm receives a
sanitized envelope with the sender and recipient, and can recover a missing
observation from the committed inbox audit on startup. Message text remains
in the private task database.

## Actual task inference

A configured agent with the `delegate` grant can plan and execute its own
bounded workflow when named as both coordinator and sole worker. The planner
receives its configured objectives and can propose only steps within the
operator's capability, roster and execution limits. See
[automatic decomposition](ORCHESTRATION.md#automatic-decomposition).

`AgentRuntime::execute_task(agent_id, task, options, cancel)` executes one bounded
provider inference through the selected model and scheduler lease. It returns a
`TaskExecutionResult` containing the terminal task snapshot, agent/model/provider
IDs, unique lease identity, response/finish reason/provider metrics, and
runtime-observed elapsed time including queueing and loading.

The lifecycle is:

```mermaid
flowchart LR
    A[Validate task and bounds] --> B[Reserve agent and task identity]
    B --> C[Select healthy eligible model]
    C --> D[Bounded scheduler queue and preparation]
    D --> E[Construct bounded provider request]
    E --> F[Run provider inference]
    F --> G[Validate response identity and bounds]
    G --> H[Record terminal task and release owned lease]
```

An agent can prepare/run only one task at a time. Accepted task IDs cannot be
overwritten or revived. Delegation records a queued task for a specific recipient;
that recipient can execute the unchanged task once. Provider-native model names
are captured at admission, so a later registry edit cannot silently retarget an
accepted request.

`ExecutionOptions` defaults to 256 output tokens, model/scheduler-limited context,
a 60-second total task budget, no explicit temperature and a 256-KiB accepted
response content limit. Limits are validated before execution. The task deadline
includes queue, preparation and inference; it is separate from transport limits.
Provider/model identity mismatches, excessive content/reported output tokens and
unsupported tool/lifecycle responses fail the task. Unknown finish reasons remain
unknown. A completed inference does not certify that generated claims are correct
or that a broader objective was achieved. Length-limited output retains its
reported finish reason.

By default, execution supplies the task prompt as a user message. Personality
and goals are not automatically injected. Explicit authorized memory context
can precede the prompt, and opt-in CLI memory options can retain an accepted
response with task provenance. Durable task history preserves accepted results
when a task store is configured. See [inference memory options](CLI.md#opt-in-memory-during-inference)
and [task history](#durable-task-ownership). Bounded multi-step planning is
implemented through the orchestrator; arbitrary tool execution and an open-ended
autonomous loop are not implemented.

## Cancellation, failure and ownership

Caller cancellation and `cancel_task(task_id)` request local cancellation.
`cancel_task` returns false when the task exists but no managed invocation remains.
It signals the owned invocation rather than releasing its lease prematurely.
`finish_task_execution` cannot finish a managed task while provider work is active.

A synchronous execution guard records cancellation and releases the local slot
when a running future is dropped/aborted, including an uncooperative provider.
The total task deadline also cancels local execution. Provider errors and invalid
responses record failure. Agents return to idle after completion/cancellation;
failure requires an operator status change before retrying with a new task ID.
Cancellation does not prove remote compute stopped; provider memory reservations
remain charged until the scheduler has acknowledged unload evidence.

The lower-level `start_task_execution`/`finish_task_execution` pair remains for
trusted composition. A caller choosing that API must finish its accepted task;
the managed `execute_task` method owns cancellation/abort cleanup for the whole
invocation. Completion validates the recorded model and unique lease before
releasing anything. Duplicate/wrong-model completion cannot release another task.

## Operator authority and permissions

Identity, permissions, model policy and status are control-plane owned. Model
output cannot modify them. Model-policy rebinding retains identity and memory;
rebinding/status changes cannot bypass active execution, and registration cannot
fabricate a running agent.

The default permission set is empty. Current variants are `communicate`,
`memory_read`, `memory_write`, `delegate`, and `simulation_act`. Memory reads and
writes, message sends and inbox reads, delegation, and simulation authorization
check the corresponding grant. The `world decide` command requires
`simulation_act` before asking the resident's configured model to choose a
revision-fenced world action. There is no tool executor or OS sandbox in this runtime.

These Rust library methods assume a trusted caller supplies the correct actor ID.
They are not a remotely authenticated principal boundary. The separate [worker broker](PROTOCOL.md#implemented-loopback-worker-api)
derives authority from signed operator or worker requests and fenced leases;
an arbitrary agent UUID is not proof of identity. Provider network authorization
is operator configuration, not authority conferred by model text.

## Memory and communication limits

Without a durable store, memory and inbox records live in per-agent in-process
maps for the lifetime of the runtime. `RuntimeLimits`
defaults to 64 agents, 1,024 accepted task records (including terminal history),
256 memories per agent, 256 messages per inbox, 64 KiB of text per record and
64 MiB of aggregate retained textual content. `with_limits` accepts positive
operator bounds; `usage_snapshot` exposes counts and logical content accounting.
These are storage-policy bounds, not measured host RAM: allocations, IDs,
provider state and caller-owned snapshot clones are separate overhead.

Writes and policy replacement check capacity before changing state. Full stores
reject new records rather than silently deleting provenance. Accepted delegated
tasks are charged once when later dispatched. Memory IDs cannot be reused across
namespaces. A memory task source must be a known task assigned to that owner;
a message task correlation must belong to its sender or recipient. A valid
reference does not certify the generated content or imply task completion.
No automatic pruning worker is implemented. Explicit durable isolated storage,
retrieval, consolidation and deletion are available through the separate
[memory store](MEMORY.md); these in-process buffers are not automatically migrated.

## Verification

```powershell
cargo test -p vespera-agents --locked
```

Deterministic fake providers verify actual invocation, transmitted bounds,
provenance, invalid/failing responses, task ownership, model-policy independence,
permission checks, duplicate delegation, concurrent preparation, managed/manual
completion fencing, queue/load/inference cancellation, absolute deadlines and
future abort. CLI integration additionally verifies the executable-to-HTTP path
against both adapter fixtures. This is not live inference evidence for a
particular installed model/server. See [CLI.md](CLI.md) for actual operation.

Managed execution validates the selected model name, input and output/context bounds before scheduler admission can load a model. Accepted invalid requests become failed history records with no load/reservation. Scheduler request admission fences the validated native name and pins context; current inventory and resource limits are rechecked before lifecycle I/O. Manual preparation remains a lower-level trusted API.

The [orchestrator library](ORCHESTRATION.md) now drives actual authorized dependent workflows. It preserves failure states, never implicitly resumes failed/suspended candidates and uses a synchronous owner-checked queued-task cancellation operation for drop cleanup. CLI workflow submission and [bounded automatic decomposition](ORCHESTRATION.md#automatic-decomposition) are integrated. A generated plan must pass validation against the authorized roster and bounds before execution.

Explicit durable stores can use `memory_authorizer()` to consult current agent grants and accepted task assignments. The storage adapter never assigns ownership to a model/provider; a regression rebinds a model and reopens both namespaces under a fresh runtime. Existing `write_memory`/`read_memory` buffers remain separate and are not automatically persisted. See [MEMORY.md](MEMORY.md).
