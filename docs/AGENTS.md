# Agents

## Current foundation

Milestone 1 provides `AgentIdentity`, `AgentDefinition`, replaceable `ModelSelectionPolicy`, default-deny permission declarations, agent-owned memory addressing, and legal task-state transitions. These are domain/configuration contracts, **not an agent executor**. There is no autonomous loop, delegation service, tool runner, persistence, or external-agent session in this build.

An agent is not a model. `AgentIdentity` contains an independent agent UUID, name, description, role, personality, and goals. `AgentDefinition` adds model policy, declared permissions, and status. Status defaults to `suspended`; other declarations are `idle`, `running`, and `failed`. Declaring `running` does not start execution. Persistent relationships and execution history are future runtime/storage features, not fields or services already supplied by this definition.

Model policy is replaceable: `fixed` references a registered model UUID; `capability` requires a nonempty capability set. Changing the policy does not change identity, memory ownership, or permissions. Configuration checks fixed references and requires at least one registered model whose declared capabilities contain the required set. This is a metadata match, not a promise of current health, affordability, or runtime selection.

Capability strings are extensible validated slugs: 1–64 ASCII bytes, beginning with `a-z`, followed by `a-z0-9._-`. Values are not silently trimmed or lowercased. Capabilities express claimed/required abilities, not authority to act. Selection must eventually combine them with availability, permissions, measured limits, and routing policy. Agents do not currently have a separate advertised-capability field beyond their model policy.

## Identity and authority ownership

The user/control plane creates and approves identity and permissions. Agents, external peers, model outputs, retrieved memories, and provider responses cannot silently replace an ID, move into another namespace, grant access, or modify system configuration. A model-policy change is a controlled configuration operation, not a generated instruction.

Current permission variants are `communicate`, `memory_read`, `memory_write`, `delegate`, and `simulation_act`; the default set is empty. These are declarations, not runtime enforcement. Tool and network grants are future boundary design, not current permission variants. Future checks must cover tool execution, network destinations, delegation, messaging, memory access, and resource use. A parent's authority is not automatically transferred to a child. Effective delegated authority should be the intersection of user approval, delegator authority, task scope, and recipient limits.

Local model text, tool results, and retrieved episodes are untrusted data. Embedded instructions cannot override configuration or the user's control plane. Approval/audit paths must be distinct from generated messages. Secret values must never enter routine history, events, or memory.

## Tasks: rules now, execution later

The current `Task` carries a task UUID, prompt, required capabilities, and state. Legal edges are `queued → running`, `queued → cancelled`, and `running → completed | failed | cancelled`. All self-transitions and exits from terminal states are rejected without changing the state. Callers must use `TaskState::transition` for lifecycle changes; public snapshot fields are not a security boundary.

Transition validation does not enqueue work, implement retries, or execute cancellation. Future execution must distinguish the stable task ID from attempts and leases. Bounded retries need a separate attempt lifecycle; they must not silently revive terminal tasks or reset the current state machine. Cancellation request, local acknowledgement, provider stop confirmation, and terminal state are different observations. Results after cancellation, expiry, or reassignment must not overwrite accepted state.

Future tasks need bounded inputs/outputs, deadlines, iteration/delegation limits, and provenance. Missing models, denied permissions, budget exhaustion, and partial failure must be reported truthfully. A capability-selection miss must not trigger every registered model.

## Future runtime — Milestones 6 and 7

A runtime will receive authorized tasks, route requests through the orchestrator/scheduler, apply permitted actions, record history, and support bounded communication/suspension. Persistence follows at Milestone 8. No unrestricted autonomous loop is enabled by default; concurrency, queues, retries, and assistance depth need explicit bounds.

Agents request actions through ports rather than directly editing world state, provider lifecycle, permissions, or storage files. Simulation residents use world-scoped observations and authorized intents. Simulation ticks do not wait for inference; late decisions carry their originating world version and require validation. Once persistence exists, individual memory survives model/provider replacement and restart. Learned summaries cannot become authoritative identity or permission records.

External agents may use other languages while VESPERA remains Rust. Authentication, leases, idempotency, quotas, cancellation, and namespace restrictions are **draft** in [PROTOCOL.md](PROTOCOL.md), not current connectivity.

## What is observable

User-visible outputs include authorized answers, messages, explicit concise decision summaries, permitted tool activity, task transitions, routing metadata, and measured observations. They do not include a model's private hidden chain-of-thought. “Observe reasoning workflows” means accountable decisions/effects, not disclosure or invention of internal reasoning.

Agent statements are generated claims unless independently verified. Record model/provider and attempt provenance without conflating them with agent identity. Future inspection must authorize private memory/messages and redact secrets. Acceptance gates are in [ROADMAP.md](ROADMAP.md); the basis is [INITIAL_PLAN.md](INITIAL_PLAN.md) and [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt).
