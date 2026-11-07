# Development log

## 2026-11-06 — Live signed mailbox evidence in Swarm

Verified implementation checkpoint `2060d53ecf0284f7a58b9f9d7cce497dca19e6ec` was pushed to `main`.

The native Swarm checkpoint `5b402c6e627725bb92a492ea900452563306c57c` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42346866960). The broker now adds a sender-to-recipient observation to the signed operator feed after a newly authorized inbox enqueue. Worker feeds remain scoped to task attempts. The record contains no message ID, content or hash. Swarm can follow that signed feed in read-only broker mode and render the observed direction.

Windows validation passed 218 offline workspace tests, the SDK doctest, graphics tests, strict workspace and graphics Clippy, and the graphics build. The HTTP fixture verified the live signed frame, worker feed isolation, retained retry and rejection behavior, and durable replay after restart. A separate desktop fixture showed zero agents on connect and then one amber link and its sequence after a signed enqueue and identical retry.

The operator's message observations share task-feed retention. Inbox contents remain process-local, and broker versions that predate message evidence reject databases containing these entries. Swarm still has separate broker and journal modes. In-flight workflow progress, dependency edges and receipt evidence remain open.

## 2026-11-06 — Native Swarm delegation view

Verified implementation checkpoint `5b402c6e627725bb92a492ea900452563306c57c` was pushed to `main`.

The Machine scheduler checkpoint `5a22eeaefd43f7e10e8d17bedde2d63ff873d070` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42346207413). Workflows can opt into recording accepted delegations with their attempt IDs, participating agents and root correlation. The journal commits the batch with the terminal outcome in one transaction. Swarm follows that journal read-only and renders independent agents, directional links and inspection context.

Windows validation passed 218 offline workspace tests, the SDK doctest, graphics tests, strict workspace and graphics Clippy, and the graphics build. The two-agent CLI fixture checked delegation evidence against the actual workflow report. The journal conflict test checked that a rejected batch leaves no partial write. On the desktop, an empty journal showed zero agents. A separate CLI workflow populated the window after commit, and mouse picking, keyboard traversal, filtering and focus exposed the matching attempt and completed outcome.

Workflow evidence is published after cleanup. Swarm does not currently show in-flight attempts, dependency edges or a first-party publisher for authorized message metadata. The next Swarm checkpoint needs those observations from the runtime and broker.

## 2026-11-06 — Machine scheduler and provider evidence

Verified implementation checkpoint `5a22eeaefd43f7e10e8d17bedde2d63ff873d070` was pushed to `main`.

The host hardware checkpoint `14d0aba58fad681fe8be5163265ad600089e2239` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42345384973). Agent and workflow commands can publish bounded aggregate snapshots from the scheduler and registry they use. Machine reads those snapshots through an opt-in local directory. Its scheduler tile distinguishes granted admission permits, estimated memory reservations and cached health from host measurements. The producer leaves a final snapshot after the cleanup attempt and refuses to reuse an existing run directory.

Windows validation passed 217 offline workspace tests, the SDK doctest, graphics tests, strict workspace and graphics Clippy, and the graphics build. The CLI subprocess fixture observed a real lease and reservation during delayed inference, confirmed acknowledged unload and checked directory reuse rejection. The workflow fixture also verified final publication. A Windows desktop run showed the live permit meter, the final snapshot with zero reservations after unload, and expiry after interrupting a separate fixture publisher.

Each directory describes one sampled local runtime. Sampling can miss short transitions. Per-model scheduling decisions, measured VRAM and device inventory remain open. Swarm still needs a projection of observed delegation and communication.

## 2026-11-06 — Native Machine hardware view

Verified implementation checkpoint `14d0aba58fad681fe8be5163265ad600089e2239` was pushed to `main`.

The Intelligence Atlas checkpoint `9af4c56df09cb03813af604198ea17ff4e5bb794` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42344484673). The native client adds a read-only `--machine` view that opens without a broker or operator key. It samples host CPU utilization and physical memory off the render thread, displays logical parallelism available to the process, and shows the wgpu render adapter. CPU and RAM meters follow the observed values. Expired and missing samples are labeled unavailable.

Windows validation passed the headless workspace suite, the SDK doctest, graphics tests, strict graphics Clippy and the graphics build. The desktop fixture verified mouse picking, keyboard selection, reduced-motion focus and reset, and changing CPU readings. The memory inspector matched the OS at 31.78 GiB total, and wgpu reported the RTX 4070 Laptop GPU and Vulkan backend. The parser rejected an explicit broker address combined with Machine mode.

Machine currently identifies the active render adapter. Inference-device inventory, VRAM, scheduler reservations, provider health and renderer performance measurements remain open. The next Machine checkpoint needs a telemetry source attached to a real scheduler instance.

## 2026-11-06 — Intelligence Network scene refresh

Verified implementation checkpoint `9af4c56df09cb03813af604198ea17ff4e5bb794` was pushed to `main`.

The native world checkpoint `33497ce968c335329b8c6616e7e6cf920ba6faa5` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42343790468). The Atlas now groups observed task states into color-coded lanes, shows current attempt links and recent event sequences, and separates connection and filter status from the inspector. Picking remains available alongside keyboard traversal and filtering. Lane and focus motion interpolate real changes and can be disabled. The read-only default remains. Opt-in cancellation requires a confirmation bound to the selected task, shows the broker's response, and leaves the graph to update from events.

Windows checks passed the 216-test headless workspace suite, the SDK doctest, graphics tests, strict Clippy and the graphics development build. The desktop fixture showed actual claim and expiry events and verified filter, keyboard selection, confirmation and updated lanes. Separate signed inspection verified the task was queued before confirmation and cancelled afterward.

The graph still represents broker tasks and workers only. Counts and links describe retained observations. Machine telemetry, wider entity coverage, Swarm and timeline reconstruction remain open. The next Atlas checkpoint adds measured hardware evidence.

## 2026-11-06 — Native World Simulation Mode

Verified implementation checkpoint `33497ce968c335329b8c6616e7e6cf920ba6faa5` was pushed to `main`.

The deterministic engine checkpoint `7298a07726e2175639d6f4a674b06c81d6932fb8` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42342843092). Added `vespera-view --world` with committed resident, site and depot meshes, encounter links, a contextual inspector and shared camera controls. A background observer refreshes SQLite state. The window is read-only by default. Explicit opt-in enables revision-fenced movement, activity, step and playback controls. The graphical dependency stays optional.

Windows validation passed 216 offline workspace tests, the SDK doctest, strict Clippy and rustdoc, the graphics build and the graphics control test. Desktop checks verified persistence after reopening, resident picking, movement, activation, playback and pause. Read-only input left the revision unchanged, and a separate CLI commit appeared in the running window. The scene labels world state as simulated.

Residents still use deterministic policies. Per the Living Intelligence Atlas directive, the next checkpoint evolves the existing Orchestration scene before Machine telemetry, Swarm relationships and model-driven resident decisions. Hardware benchmarks, richer orchestration evidence and layer transitions remain open.

## 2026-11-06 — Persistent deterministic world engine

Verified implementation checkpoint `7298a07726e2175639d6f4a674b06c81d6932fb8` was pushed to `main`.

The native orchestration view checkpoint `8609bbb5704080372d9164c183550bd8f52266fa` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42342384565). Added `vespera-world` and native `world` commands. The configurable simulation persists independently of providers and graphics. Residents follow role goals, gather and deliver resources, build shelters, record encounters and retain bounded action memories. The SQLite store serializes writers and commits each batch of ticks together. Movement and activation controls require an observed revision. Public observations omit memories.

Windows validation passed 216 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, the optimized workspace build and the optimized world subprocess test. The test compared two independently initialized worlds after 60 ticks and across different commit batch sizes. It also checked persistent memories, relationship evidence, stale movement rejection and continued simulation while a resident is suspended.

Residents currently use deterministic role policies. The native 3D World Simulation Mode and model-driven resident actions remain required work. Next wire the Bevy view to committed world snapshots and revision-checked controls.

## 2026-11-06 — Native 3D orchestration client

Verified implementation checkpoint `8609bbb5704080372d9164c183550bd8f52266fa` was pushed to `main`.

The worker mailbox checkpoint `0c0e57ac26503679190726e6c2a93a0862066e15` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42341391818). Added the optional Bevy/wgpu executable `vespera-view`. Signed broker subscriptions populate task meshes, observed worker meshes and attempt links. Picking exposes observed state and provenance. Camera controls provide orbit, pan, zoom, focus and reset. Cancellation is disabled unless explicitly enabled and uses the signed operator API. The default workspace stays headless.

The 215-test workspace suite and SDK doctest passed. Strict Clippy passed with and without graphics. The Windows graphics development build rendered on a machine with an RTX 4070 Laptop GPU. Desktop checks verified picking, inspector details, camera focus and reset, and cancellation of a queued fixture task. Independent broker inspection confirmed the cancelled state.

The view currently covers external broker tasks and workers on loopback. Unavailable measurements are labeled. Providers, models, native workflows, memory, communication telemetry and persistent World Simulation Mode remain required work. GPU memory and responsiveness benchmarks and other operating systems remain unverified.

## 2026-11-06 — Authorized worker mailboxes

Verified implementation checkpoint `0c0e57ac26503679190726e6c2a93a0862066e15` was pushed to `main`.

Added optional directional `message_targets` grants and signed SDK actions for sending, reading and acknowledging text. The authenticated principal determines sender and inbox ownership. Pending messages and recent acknowledgements are bounded, and revocation purges messages to and from that worker. Mailboxes are process-local, including in durable task mode.

Windows validation passed the 214-test workspace suite plus the SDK doctest, strict Clippy and rustdoc, and release worker tests. The subprocess test covers directional authorization, private inboxes, acknowledgements, exact retries, conflicting content, capacity and revocation.

The user's updated visualization directive makes an integrated executable 3D application a completion gate. Begin the native Orchestration Mode using observed broker events, and develop World Simulation Mode alongside the authoritative simulation backend. Durable inboxes and cross-host deployment remain open.

## 2026-11-06 — Opt-in TLS worker transport

Verified implementation checkpoint `6ebf89c06b6972ab08f679d66b4df1af1fe1cb48` was pushed to the default branch, `main`.

The WebSocket checkpoint `3fedaa938018d3ab77da5dde76efee445d88c1e6` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42340588226). The worker listener now serves HTTPS and secure WebSockets with TLS 1.3 when supplied a PEM identity. Remote binding also requires `--allow-remote`. The Rust SDK uses the same explicit trust roots and IP identity checks for both transports. Request proofs and worker grants continue to apply over TLS. The broker reuses locked Rustls packages, and the CLI fixture generates temporary identities with dev-only Rcgen.

Windows validation passed 213 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, the optimized workspace build and release worker subprocess tests. A loopback client verified HTTPS task actions and secure WebSocket events on an explicitly enabled wildcard listener. Tests rejected missing remote opt-in, plaintext on the TLS port, untrusted roots and a certificate with the wrong IP identity.

Cross-host deployment remains untested. The SDK uses numeric endpoints and explicit PEM trust roots. The operator supplies and rotates the certificates. Broader authorized messaging remains open.

## 2026-11-06 — Signed WebSocket event subscriptions

Verified implementation checkpoint `3fedaa938018d3ab77da5dde76efee445d88c1e6` was pushed to the default branch, `main`.

The durable event feed checkpoint `2fb290c2853825e7a43fb473a268cff72bd14bc1` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42339791343). Added signed WebSocket upgrade routes for operator and worker events, using the same scoped cursors as HTTP replay. The proof covers the exact GET target and consumes a fresh nonce. The broker reads one authorized event at a time and releases its state lock before sending. Each connection has bounded frame sizes, write time and lifetime. A separate eight-slot quota leaves room for HTTP actions. The Rust SDK exposes typed live frames and reconnect from a processed cursor.

Windows validation passed 212 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, the workspace release build and optimized broker and worker HTTP tests. Subprocess tests cover operator and worker delivery, scope isolation, reconnect, revocation, signed-query tampering, reused upgrade proofs and quota release. Hyper now closes ordinary HTTP responses explicitly, preserving the upgrade headers on switching-protocol responses. Tokio Tungstenite adds ten locked packages without changing existing versions.

WebSockets remain plaintext and loopback-only. Clients handle reconnection and retention gaps. Protected remote transport and broader interoperability remain open.

## 2026-11-06 — Durable broker event feeds

Verified implementation checkpoint `2fb290c2853825e7a43fb473a268cff72bd14bc1` was pushed to the default branch, `main`.

The durable task recovery checkpoint `fa521556b4e62e38e7789c174ce50e053f249d7d` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42339348902). Schema 2 adds bounded per-principal feed snapshots to the broker database. Task changes, event pages and revocations commit in one transaction. The server restores each feed's sequence and identity before reconciling interrupted attempts. Revoked or removed workers lose their saved feed. Opening a schema 1 database preserves its tasks and revocations while creating empty feeds.

Windows validation passed 210 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, the release workspace build and optimized broker and worker HTTP tests. Forced-exit recovery verified both operator and worker cursor continuity, stable event identities, restart fencing and cross-feed isolation. Storage fixtures covered migration, policy removal and rollback after a feed write failure.

Feeds retain 256 events per principal. Clients still poll HTTP and must handle retention gaps. The original process-only mode remains available. Live subscriptions and protected remote transport remain open.

## 2026-11-06 — Durable broker recovery

Verified implementation checkpoint `fa521556b4e62e38e7789c174ce50e053f249d7d` was pushed to the default branch, `main`.

The worker event replay checkpoint `74d2392e7487fdb8b0f95283d3fc687f91692c3f` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42338686957). Added opt-in `serve --broker-db`. A single exclusive SQLite connection commits task state, accepted outcomes, attempt generations and revocations before HTTP responses. Startup recovers queue order, checks deadlines and fences interrupted attempts. A failed write leaves the last committed database state available for recovery and disables service operations until restart. The existing locked Rusqlite build is reused.

Windows validation passed 208 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, the release workspace build and optimized broker and worker HTTP tests. The subprocess fixture verifies forced exit and reopening, exclusive ownership, preserved outcomes and revocations, restart fencing, generation advancement, retry exhaustion and queue order. The storage tests inject a failed write batch and simulate a wall-clock rollback.

The database contains prompts and results and relies on local filesystem permissions. Broker event feeds remain process-local. Writes are synchronous, so storage latency can delay other requests. Durable feeds, live subscriptions and protected remote transport remain open.

## 2026-11-06 — Worker-scoped event replay

Verified implementation checkpoint `74d2392e7487fdb8b0f95283d3fc687f91692c3f` was pushed to the default branch, `main`.

The Rust SDK checkpoint `f3d5f5f59bd7ce14119f26a525f82c1519c4c504` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42338173619). Added a signed `replay` action to both v2 roles and typed SDK support. The broker records task lifecycle metadata with version 3 envelopes. Operators see every recorded transition and each worker sees its own attempts. Independent feeds retain 256 events and return pages of up to 64. Feed IDs fence cursors across principals and restarts, and a retention gap returns an explicit error. Versions 1 and 2 remain readable.

Windows validation passed 205 offline tests plus the SDK doctest, strict Clippy and rustdoc, the workspace release build and optimized envelope, broker and CLI tests. Coverage includes reconnect, cursor isolation, revocation, retention gaps, expiry, active cancellation and completion retries. The documented PowerShell flow returned the claim and completion events.

Broker feeds remain process-local and are separate from the SQLite CLI journal. Clients poll for updates. Durable recovery, live subscriptions and protected remote transport remain open.

## 2026-11-06 — Standalone Rust worker SDK

Verified implementation checkpoint `f3d5f5f59bd7ce14119f26a525f82c1519c4c504` was pushed to the default branch, `main`.

The signed HTTP checkpoint `7f2eea23dcb8e3d3d6be0b7de6dd8e99ef89fd37` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42337663813). Added `vespera-sdk` with separate typed operator and worker clients for every v2 task operation. The client fetches the session for each action, signs the exact bytes and maps known server errors to typed codes. It uses numeric loopback HTTP/1 with no proxy, redirects or automatic retries. Requests and JSON responses have byte limits and each HTTP exchange has a deadline. Its normal dependency graph contains `vespera-core` and existing third-party packages, independent of `vespera-api` and the CLI.

Windows validation passed 204 offline tests plus the SDK doctest, formatting, strict Clippy and rustdoc, and the release workspace build. The SDK subprocess test exercised submit, inspect, cancel, revoke, claim, renew, stale fencing and exact completion retries through the real CLI. Transport fixtures rejected a redirect and an oversized chunked response. The SDK and worker subprocess tests passed in optimized builds.

The SDK leaves polling, authorized task execution and retry timing to callers. The transport remains plaintext loopback and responses are unsigned. Network event replay, durable broker recovery and protected remote transport remain open.

## 2026-11-06 — Replay-resistant worker HTTP authentication

Verified implementation checkpoint `7f2eea23dcb8e3d3d6be0b7de6dd8e99ef89fd37` was pushed to the default branch, `main`.

The loopback service checkpoint `58837796c27b7e59f013f4ffaaa6800629dc93e5` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42337036931). HTTP v2 replaces the original bearer endpoints with HMAC-SHA256 proofs bound to the method, route, principal, server session, monotonic issue time, nonce and exact body bytes. Each verified nonce is consumed before broker execution under the same lock. The 128-slot cache per principal preserves live nonce entries and applies backpressure. A new server session invalidates old proofs. Ring was already present in the lockfile.

The 201-test offline Windows suite, formatting, strict Clippy and rustdoc, optimized workspace build and release auth/broker/CLI tests passed. Tests cover concurrent duplicate delivery, body tampering, key/identity mismatch, retired bearer routes, expiration, server sessions and saturated-cache behavior. The updated PowerShell guide completed submit, claim, complete and inspect using .NET HMAC.

Existing policy file field names still work. Clients must move to v2 and use fresh keys if those keys were previously used as bearer credentials. The server still binds only to loopback and uses plaintext HTTP with unsigned responses. Keys and broker state are process-local. Next is the native client SDK, followed by authorized network events, durable reconciliation and protected remote transport.

## 2026-11-06 — Authenticated loopback worker HTTP service

Verified implementation checkpoint `58837796c27b7e59f013f4ffaaa6800629dc93e5` is on `origin/main` and is the repository's default branch tip at publication. [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42337036931) was in progress when this checkpoint was recorded.

Previous checkpoint `ba0cf36581074fe196b584820c6dd70436a22f80` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42335796654). Added `vespera serve` and two versioned HTTP/1 endpoints for operator and worker actions. The access file approves identities and capabilities and references distinct environment credentials. The authenticated credential determines the worker identity. Revocation removes its credential and fences the attempt in one serialized operation. The listener requires numeric loopback and caps connections, header time, total connection time and body bytes. HTTP/1 server support adds one transitive package, `httpdate`.

Windows validation passed all 199 offline tests, formatting, strict Clippy and rustdoc, and the optimized workspace build. Two subprocess tests passed in debug and release, covering credential scope, strict decoding, body limits, non-loopback rejection, revocation/reassignment, fenced renewal and result retries. The documented PowerShell submit, claim, complete and inspect example returned the expected result. See the [wire contract](PROTOCOL.md#implemented-loopback-worker-api).

The process holds task state in memory. Bearer credentials are reusable, and revoked workers can return after restart if their grants remain in the file. The broker does not yet integrate with native workflows or the event journal. Next work includes replay-resistant authentication, a client SDK, durable reconciliation and authorized event delivery.

## 2026-11-06 — External worker lease broker

Previous checkpoint `8527f95d007e432c5a8b346f84527bdedbb6737a` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42335390456). Added `vespera-api::LeaseBroker` with operator-approved workers, capability matching and explicit task allowlists. Each lease carries a fresh attempt ID, a per-task fencing generation and a monotonic expiry capped by the task deadline. Expiry and revocation permit bounded reassignment. Completion accepts one result and returns the same record on exact retry. Cancellation fences late work and preserves an already accepted result.

The broker holds at most 64 workers and 256 task records, including terminal outcomes. It retains state for one host process and takes the authenticated identity from its caller. Network authentication, transport, durable restart reconciliation and SDK remain open. No new third-party packages were added.

All 197 offline tests passed. Strict Clippy and rustdoc, the optimized workspace build and four optimized broker tests passed. Broker tests cover worker scope, attempt fencing and reassignment, revocation, duplicate results, cancellation, deadline-capped renewal and capacity.

## 2026-11-06 — Versioned task failure and cancellation events

Previous checkpoint `11fdcd74c8949fb6ac0be20a089c36c7c65eb559` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42334971707). The CLI now journals observed failed and cancelled agent tasks and workflow roots. Existing event kinds still emit version 1. New `task_failed` and `task_cancelled` kinds require version 2, and current readers decode both versions. Records contain task IDs and omit provider errors and user payloads. Publication follows observed terminal state and remains separate from execution. JSON and human reports expose journal errors alongside execution evidence.

Validation passed 193 offline tests, strict Clippy and rustdoc, the optimized workspace build and all 30 release CLI subprocess tests. The schema compatibility regression covers legacy decoding and rejection of new kinds in version 1. Existing admission and deadline tests now verify replay of correlated failure and cancellation records for agent and workflow execution. Broader lifecycle coverage, atomic audit coordination and external-agent interoperability remain open.

## 2026-11-06 — Local event streaming and cursor resume

Previous checkpoint `1fc484b6663d7717c9ee10704ec696a60848659f` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42334621248). Added `events follow` for newline-delimited JSON delivery from the existing read-only journal. The follower flushes one entry at a time, buffers at most one replay page and holds no database transaction while writing. Clients can resume from a consumed cursor. Expiry exits nonzero, and a cursorless start reports earlier discarded history on stderr. Optional idle and event-count bounds support unattended consumers. A blocked stdout write can outlast the idle bound.

Validation passed all 192 offline workspace tests, strict Clippy and rustdoc, and the optimized workspace build. Two subprocess regressions passed in debug and release mode. They cover delivery of a commit made after following begins, disconnect and resume, idle exit and retention gaps. Broader lifecycle events, durable audit coordination and authenticated network delivery remain roadmap work.

## 2026-11-06 — Durable local completion journal

Previous checkpoint `692bd44b7404997df94995ca5f992dbd070a7cd5` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42333919135). Added the `vespera-events` crate and `events init/replay` commands. SQLite transactions append events and advance retention together. Replay uses one read-only snapshot with journal-specific cursors, ordered bounded pages and explicit expiry. Duplicate IDs are idempotent for matching retained envelopes. The new crate reuses locked dependencies.

Agent and workflow CLI execution can opt into recording successful completions in an existing journal. Publication preserves typed correlation IDs and uses static source labels. Journal failure preserves the execution report and returns nonzero. Completion publication is separate from inference and memory persistence, leaving a potential process-exit gap. Local journal access is governed by filesystem permissions. Live subscriptions, additional lifecycle events and durable audit coordination remain open.

Validation passed 190 offline workspace tests, strict Clippy and rustdoc, and the optimized workspace build. Four journal tests cover reopening, ordering, retention, cursor fencing, duplicate conflicts, foreign files, read-only access and injected trim failure. The subprocess test passed in debug and release builds. It exercises agent and workflow completion, metadata-only replay across invocations and missing-journal preflight.

## 2026-11-06 — Opt-in durable memory during agent inference

Previous checkpoint `b490c0ec62fe8a4f76e39c1797fc8c6a9f4e1539` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42333437601). Single-agent execution now accepts an explicitly retrieved memory snapshot and separately offers generated result retention. The CLI checks grants and opens the requested existing database before provider requests. The agent runtime checks the snapshot owner and read grant again. Provider preflight validates the same messages sent during inference.

Retrieval considers eight namespace-ordered matches and includes whole records within an 8 KiB serialized user message. Execution reports list the included IDs while task history keeps the original prompt. Result retention writes accepted output as generated episodic memory through the live task authority. Persistence errors preserve the inference result and return nonzero. Database work runs outside the inference deadline. Input tokenization remains provider-specific.

Validation passed all 185 offline workspace tests, strict Clippy and rustdoc, and the optimized workspace build. Two subprocess regressions also passed in release mode. They verify the captured provider payload, oversized record exclusion, original task prompt, persisted task provenance and absence of provider I/O on denied grants or missing storage. The remaining roadmap includes durable events and replay, durable task history, interoperability, simulation, 3D and experiments.

## 2026-11-06 — Explicit offline durable memory CLI

Previous verified checkpoint: `c59ecb7f5ddeba801c83bc526f95256d509916d9`, pushed to `main`; Ubuntu CI [42332452032](https://github.com/ShamelesAbyss/Vespera/actions/runs/42332452032) passed. Added `memory init/add/get/list/search/usage/consolidate/delete`, all scoped to a configured agent UUID and explicit read/write grants. The application authority uses validated local declarations without resolving provider credentials or creating adapters. It never invents accepted task ownership; offline writes support observation/import and explicit derived lineage only.

The storage library now distinguishes create/initialize, existing-file write, and true SQLite read-only opens. Only CLI `init` creates parents/storage. Inspection and ordinary writes reject missing/empty/unrelated files without migration. Denied grants fail before content/database access. File payloads, query text, result pages, timestamps and sources are bounded; file/database calls run on a blocking worker. Explicit delete reports absence without claiming an already missing record was removed, and referenced originals remain protected. No new third-party packages were added.

Validation passed: all 183 offline workspace tests, two separately ignored helpers (live Ollama health and parent-invoked crash writer), formatting, all-target compilation, strict Clippy/rustdoc, optimized workspace build, 36 optimized application/CLI tests and 16 optimized memory tests. Eight actual subprocess tests cover persistence/filter/search/usage, owner isolation, model replacement, grants, missing/foreign files, invalid input, lineage/deletion and help. Read-only storage tests verify byte-for-byte unchanged database contents and denied writes. A separate release PowerShell smoke run used a new ignored private database, exercised default UUID/time assignment, listed and found the same record via phrase search, observed SQLite 3.54.1 and confirmed unchanged file hash after inspection. No user database or configuration was changed.

Updated root onboarding and full CLI/memory/architecture/roadmap/implementation documentation. The `memory` command does not start inference or auto-attach memory to prompts; retained buffers and durable storage remain distinct. Next: bounded opt-in memory context and owned generated-result persistence during real agent execution, then durable events/replay and the remaining platform roadmap. Dedicated backup/restore, durable task/source history, encryption and disk/power failure validation remain open.

## 2026-11-06 — Durable authorized SQLite memory and retrieval

Previous verified checkpoint: `c19d0b625c28f71ddc84526b314955a8043518e7`, pushed to the new `main`. Ubuntu CI succeeded for that promotion checkpoint ([42331095059](https://github.com/ShamelesAbyss/Vespera/actions/runs/42331095059)) and workflow source `339b9f8` ([42331021906](https://github.com/ShamelesAbyss/Vespera/actions/runs/42331021906)). The former baseline remains preserved on `old-dead-code`.

Added the ninth first-party crate, `vespera-memory`, with explicit file opening, transactional schema identity/initialization, immutable owner-scoped records, structured/FTS5 literal-phrase retrieval, content/count quotas across connections, namespace-local persistent cursors, accepted-task provenance checks, atomic consolidation lineage and protected source deletion. Foreign/nonempty and future-schema databases fail without migration. Initialization reads identity/schema in a single snapshot and rechecks under the writer lock, addressing concurrent-opener races. Database/index/counter/lineage mutations commit or roll back together.

The real agent authorization port consults current grants and accepted task assignments rather than copying permissions or assigning ownership to model names. A regression rebinds a model and reopens two namespaces under a fresh runtime, preserving content and denying cross-owner access. Original runtime memory buffers remain separate; no implicit migration, prompt injection or CLI memory operation is claimed. Origins/timestamps/methods are supplied provenance classifications, not independently verified facts. Generated-origin writes require an owned accepted task.

Locked `rusqlite 0.41.1` with defaults disabled, native `libsqlite3-sys` and bundled SQLite (source reports 3.54.1). Six new third-party packages are documented; no existing package version/unrelated lock edge changes. FTS5 is exercised by actual queries. First-party code remains Rust; bundled third-party SQLite needs a C compiler. Added ignore rules for SQLite files/sidecars, and corrected onboarding to name the actual private config ignore rule.

Windows validation: all 174 offline tests pass; the ordinary suite separately ignores the live Ollama health test and a child-only crash helper that its parent explicitly launches. Formatting, all-target check, strict Clippy, strict rustdoc, optimized workspace build, 15 optimized storage tests, 21 optimized agent tests and 28 optimized app tests pass. Coverage includes rollback after staged migration/FTS/lineage writes, abrupt child exit leaving a real rollback journal, reopen recovery, owner-local pagination/phrase safety, concurrent initialization/admission, quotas, current grants, source ownership, unchanged foreign/future files and bounded lock failure. Test fixture/helper and documentation escaping mistakes were repaired before final validation; no failing checkpoint is represented as verified.

Rewrote memory documentation and updated architecture, README, agent/dependency/implementation/roadmap records. Limitations: blocking OS/database I/O has no hard deadline; quotas count logical content, not full file/index/heap size; no encryption, backup/restore, secure erase, event outbox or durable workflow/task service yet. Process-exit recovery is verified, not universal power-loss/disk-full behavior. Next: explicit application/CLI memory operations and bounded context integration, followed by events/replay and the remaining platform roadmap. Full completion remains unproven.

## 2026-11-06 — Promote development branch to the public default

At the owner's explicit request, renamed the former GitHub `main` to `old-dead-code` (spaces are invalid in Git branch names), preserving commit `adb163a8750a87fb8bfa6f8dff4e3afcdc10a2a7`. Renamed `codex/scheduler-recovery` to `main` at verified development checkpoint `339b9f8b1297161ab1dc460b8b3b90bbcf96f866` and set repository `default_branch` to `main`. No history was rewritten, branch content overwritten or force push performed. The initial second rename did not complete during the first API sequence; remote refs were inspected before resuming the remaining operation successfully.

Verified GitHub repository metadata and remote symbolic HEAD both identify the new `main`, and both remote branch SHAs match the preserved source tips. Local branch names/upstreams and `origin/HEAD` now match. Future checkpoints target `origin/main`; `old-dead-code` is the historical baseline. The repository homepage now opens the current implementation/README by default. CI triggers accept all push branches and required no change.

The SQLite memory work started after `339b9f8` remains unfinished local work and is deliberately excluded from this documentation-only checkpoint. Its dependency probe introduced only the new memory crate and six new locked packages; it is not advertised as working storage. Continue implementing/validating durable namespace storage from the current worktree on `main`.

## 2026-11-06 — Workflow CLI integration and live dependent execution

Previous verified checkpoint: `020bdeb4f576b6e17de63cca5d85bc5ec834f5bc`, pushed; Ubuntu CI [42326680634](https://github.com/ShamelesAbyss/Vespera/actions/runs/42326680634) passed. The native CLI now exposes `run-workflow --workflow-file` with strict bounded UTF-8 JSON, explicit invocation-only resume, overall deadline, opt-in unload and JSON/human reports. Added a structurally validated example. Composition validates the complete config/plan/references/authority before adapters and scopes initialization to eligible named worker models. A delegation-only coordinator does not initialize its unrelated model adapter or credentials.

Integration review found that a long step could expire the next step's health evidence. The engine now explicitly polls only eligible candidate providers before each attempt, within the workflow/command deadline. A regression forces observation expiry between steps and proves subsequent routing recovers. No model activation is performed by those health checks; no failed/suspended identity is silently resumed by the engine. Configuration stays unchanged.

Windows validation passed: all 158 offline workspace tests (one explicit live health test ignored), formatting, all-target compilation, strict Clippy, strict rustdoc, optimized workspace build, 28 optimized application/CLI tests and twelve optimized orchestration tests. Executable fixtures cover both HTTP adapters, dependencies/provenance/cleanup, denied activation/permissions, unknown agents, invalid/oversized files before credential resolution, coordinator-only inventory isolation and cancellation preventing later steps. Validation handles disappeared after an environment continuation; authoritative checks were re-run and observed terminal success rather than assuming missing results were successful.

The optimized CLI then executed a real native two-step workflow against Ollama 0.40.13 / hermes3:8b. Both contributions returned exactly `OK`, dependency data reached the second request, root/child states completed and runtime elapsed was 4,001 ms. Explicit native unload was acknowledged and a fresh residency query reported zero models. Evidence with executable SHA-256 and sanitized full report is in `artifacts/live-workflow-2026-11-06.json`; inputs/raw output remain in ignored private data files. This uses one agent/model and is not a live multi-model or performance claim. No weights were downloaded or existing config rewritten.

Updated CLI/architecture/agent/orchestration/roadmap/implementation documentation and root onboarding. Remaining work includes durable isolated SQLite memory/retrieval and parent/child histories, events/replay, external-agent authentication/connectivity, automatic decomposition/evaluation, resource/context-aware ranking and allocation reconciliation, simulation, native 3D and experiments. Next priority is the durable memory foundation with namespace/provenance/migration/reopen tests; platform completion is not claimed.

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
