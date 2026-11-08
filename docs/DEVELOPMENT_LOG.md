# Development log

## 2026-11-07 — Desktop provider discovery

Verified implementation checkpoint `b0645b12aadd70949354b9053fc8983985e94ea8`, pushed to `main`. The desktop Models & Providers screen uses validated application configuration and the same Ollama and OpenAI-compatible adapters as the CLI. It supports endpoint editing, bounded background health and inventory requests, and paginated provider-reported model IDs. The adapter capability readout distinguishes lifecycle support from observed residency.

Formatting, locked offline workspace and graphics suites, strict workspace and graphics Clippy, and the Windows graphics build passed. A compatible fixture checked both `/v1/models` requests and verified unsupported residency. A separate test rejected a non-loopback address and URL-embedded credentials. The native Windows walkthrough listed 12 models from Ollama 0.40.13 and paged through both result pages. Editing the compatible endpoint to the same server's `/v1` API produced the same model count and showed metadata, load, unload and residency as unsupported through that adapter. The graphical walkthrough used Ollama for both protocol paths. Previous Bionic daemon evidence is recorded in the provider guide.

Desktop provider discovery currently covers unauthenticated loopback endpoints and session-only settings. Durable provider and model registration, graphical authentication, assigning a model to an agent, and executing a bounded task remain open.

## 2026-11-07 — Native Home and local world access

Verified implementation checkpoint `f63963de8b8e6bc325a2205a341011a2d86fc238`, pushed to `main`. Launching the Bevy/wgpu client without flags now opens Home. OS CPU and available RAM observations are separate from the wgpu renderer identity. Home opens the existing Machine and Living World scenes in one window. Explicit Atlas, broker, journal and world flags retain their entry points. The desktop creates the starter database only after an explicit action and uses the existing revision-fenced world store for mutations.

Windows formatting, locked offline workspace tests, strict workspace and graphics Clippy, graphics build, and graphics tests passed. The opt-in live tests were skipped by the offline suites. On the RTX 4070 Laptop GPU, mouse navigation opened Hardware, the RAM object showed its live inspector, F5 returned Home, and a fresh desktop world opened at revision 0. Space committed revision 1, which Home reopened after the process exited. Regression tests cover preserving unrecognized files, reopening committed state and selecting explicit launch modes.

Home currently connects only to the local hardware and deterministic simulation backends. The next desktop acceptance slice is graphical provider connectivity and inventory using the existing adapters, followed by model registration and a bounded agent task. Public packaging and additional platform validation remain open.

## 2026-11-07 — Windows release rehearsal and desktop transition

Verified packaging checkpoint `8066f9541a2e370ccfec9dc56a76dd3d38fcd766`, pushed to `main`. The packager passed formatting, strict Clippy and its Cargo artifact-selection test. It built the optimized Windows CLI and Atlas, then staged 27 checksummed files. The local ZIP was 32,084,857 bytes with SHA-256 `09a1bc1583c80ba0d7d0b002483a07673d1add6f7bbb99bf5fcabd719c4dae0c`. A fresh extraction verified every size and hash, ran CLI configuration and host diagnostics, and initialized, advanced and replayed a world at revision 8.

The extracted Atlas showed the same committed world at revision 8. On the RTX 4070 Laptop GPU with Vulkan at 1200×800, the 60-update optimized samples measured p95 7.28 ms in Machine and 7.44 ms in Living World. The Machine inspector showed live device-wide NVIDIA VRAM and a separate saved scheduler rejection of 13 GiB requested against 8 GiB. These readouts verify the Windows interactive path. Multi-GPU and other graphics environments remain unverified.

The upgrade rehearsal built prior commit `c935e7be1232d1f23207e4f435cf070dcea49214` separately. Its schema 2 broker committed a result and three events. The packaged binary migrated a stopped-state copy, preserved that result and feed, and reopened a new pending message across restart. The prior binary refused the upgraded database and successfully recovered the original result and feed from a restored pre-upgrade copy. Hashes confirmed the backup set was unchanged.

The native desktop directive extends the work into a no-CLI application shell and graphical operation while preserving backend authority. WSL2 Ubuntu is available for a separate-OS transport check on this machine. Independent-machine TLS, multi-GPU validation and the owner's licensing decision remain open. The local archive was retained under `target` for rehearsal.

## 2026-11-07 — Live authored workflow comparisons

Verified implementation checkpoint `899db706eca09542b001a0bf7c62451f8f13214f`, pushed to `main`. The memory comparison checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42442180179).

`experiment-workflows` runs two authored plans on paired private cases. It exports numbered plan structure, exact-match results, actual attempts, timings, provider observations and journal cursors. Fixture coverage verified delegation denial before provider I/O, redaction and replay of an accepted dependency. Windows formatting, offline workspace and graphics tests, and both strict Clippy passes succeeded. Swarm projection tests verified historical outcomes from the experiment journal source.

Live Ollama with `hermes3:8b` compared one direct step with a two-step answer-and-check plan. Both plans matched both arithmetic cases. The runner observed two baseline attempts and four candidate attempts. Journal replay found six accepted attempts, six attempt outcomes and four root outcomes. It confirmed the exact predecessor for each of the two dependent attempts. Every runtime confirmed cleanup and the provider reported zero resident models. Milestone 13 is verified within the deterministic simulation and native Ollama scope. The next milestone is local release package and fresh-workspace validation, followed by upgrade and recovery evidence.

## 2026-11-07 — Live paired memory experiments

Verified implementation checkpoint `dcff7bb27f7778f2685c1099fffe659c10cfc2d9`, pushed to `main`.

`experiment-models --memory-database` holds model and agent fixed and compares no retrieval with authorized literal phrase retrieval. Preflight loads candidate snapshots read-only before contacting a provider. Reports carry retrieval duration and the count of records included in successful requests. Formatting, strict Clippy and offline workspace tests passed. The fixture verified permission denial before provider I/O, isolation from a second owner, a zero-match request, redaction and unchanged database bytes.

Live Ollama with `hermes3:8b` completed all four trials. With one owned observation, the retrieval arm matched the synthetic code case and the baseline missed it. Both arms matched the arithmetic case, for which retrieval found no record. The comparison reported 1/2 and 2/2 exact matches, four terminal journal observations and confirmed cleanup. A separate residency check returned zero models and the memory database hash matched before and after. These scores describe two cases. Live orchestration policy comparisons remain open.

## 2026-11-07 — Live paired model experiments

Verified implementation checkpoint `aaf324b8310efd510b52b7242deef8abad52b1c3`, pushed to `main`. The offline world experiment checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42441280150).

`experiment-models` compares two fixed model bindings on the same private cases with alternating arm order. It reports attempts and exact-match outcomes, bounded timings, provider and model version observations, residency and terminal journal cursors. Formatting, strict workspace Clippy and offline workspace tests passed. The subprocess fixture verified pairing, deadline stops, redaction and journal correlation.

On Ollama 0.40.13, `hermes3:8b` matched 1/2 test cases and the local Gemma 4 12B registration matched 0/2. All four calls completed and produced four journal records. The provider reported distinct stable digests across the two cases per arm. End-to-end times were 3.82–3.94 seconds for Hermes and 8.16–9.59 seconds for Gemma. Observed post-task VRAM was 4,614,782,975 and 6,228,740,669 bytes respectively. All cleanup was confirmed, and Ollama reported zero resident models at the end. These results describe one machine and two exact-match cases. Live orchestration and memory policy comparisons remain open.

## 2026-11-07 — Reproducible world policy experiments

Verified implementation checkpoint `a07d129c9408abd6c1cff7eac811107161627c6f`, pushed to `main`. The scheduler admission checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42440745533).

`experiment-world` compares paired roster policies, alternates trial order and exports redacted configuration, outcome denominators, timings, resource readings and durable advance event references. Formatting, strict workspace Clippy and the offline workspace suite passed. The integration test independently reopened each trial at its reported revision, matched its event ID and replayed the exported configuration. It also verified resident identity redaction and refusal of a preexisting evidence directory.

In the bundled 60-tick world on seeds 42–44, the baseline built six shelters in every trial. Suspending one forager left one active forager and one builder and yielded three shelters per trial. With a five-shelter target, the baseline met 3/3 and the intervention met 0/3. A separate invocation reproduced the metrics from the exported configuration, and the CLI replayed the intervention at revision 61. One run sampled 13,250,560 resident process bytes after all trials. Timings include policy and tick commits and vary by run. This checkpoint covers offline simulation comparisons. Live model and orchestration policy comparisons remain open.

## 2026-11-07 — Scheduler admission evidence in Machine

Verified implementation checkpoint `8d830750c9e23478e969e2c9213db004e4c26d2f`, pushed to `main`. The NVIDIA telemetry checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42440299752).

Scheduler sample version 5 preserves the latest returned preparation result with a model identity and a bounded reason. Earlier sample versions remain readable. Fixtures verified RAM and VRAM rejections, admitted reuse, final CLI snapshots and legacy decoding. Windows formatting, strict workspace and graphics Clippy, and offline workspace and graphics tests passed.

Live Ollama 0.40.13 with `hermes3:8b` produced an admitted sample with acknowledged explicit loading and a separate RAM budget rejection of 13 GiB against 8 GiB. The latter came from a deliberately increased estimate. Both final snapshots were present, and Ollama reported no resident models afterward. The new native readouts still need an interactive inspection. The experiment runner and release gates remain open.

## 2026-11-07 — Measured NVIDIA VRAM in Machine

Verified implementation checkpoint `9d07027463237725f2ac9ea451d26074d159b518`, pushed to `main`. The Swarm receipt checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42439913893).

Machine samples device-wide NVIDIA VRAM from NVML and lets users cycle accessible devices by PCI address. The wgpu renderer and scheduler reservations remain separate. Windows formatting, headless workspace tests, graphics tests, both strict Clippy passes and the graphics build passed. Fixtures verified device cycling, removal and unavailable readings. The live probe read the RTX 4070 Laptop GPU at PCI `00000000:01:00.0` with 1,692,897,280 bytes used, 6,641,184,768 free and 8,585,740,288 total. The subsequent NVIDIA utility reading matched at whole MiB resolution. The native window launched. The Windows automation helper returned `foreground window did not report a process id` twice, so interactive validation of the new panel remains open. There was one accessible NVIDIA GPU.

## 2026-11-07 — Native Swarm acknowledgement evidence

Verified implementation checkpoint `f77d2841c0ac69a69945623071ab30a78467dfee`, pushed to `main`. The Atlas timing checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42439252028).

The opt-in broker policy emits a schema 8 operator receipt only when the signed recipient acknowledges a pending entry. SDK tests verify live delivery, concurrent and restarted retries, worker-feed isolation, metadata redaction and atomic rollback of the inbox removal and receipt. Windows formatting, offline workspace tests, both strict Clippy passes and the graphics tests and build passed.

In the native Windows view, Swarm froze at sequence 1 with an amber enqueue link while the live feed advanced to sequence 2. Returning live displayed a pink reverse link and identified the acknowledging recipient. An independent signed replay contained one acceptance and one receipt after two acknowledgement requests. The event describes a route-level acknowledgement. Deploy schema 8 readers before enabling it. The next graphics gate is measured device VRAM and multi-device visibility in Machine.

## 2026-11-07 — Live Network and Swarm hardware validation

Validated the graphics checkpoint `2b29b4c2df8ce9ae25d02dc0187b0ca4b3a23ccc` with two signed fixture workers and a live SQLite broker. The workers exercised real broker submit, claim, completion, mailbox send and acknowledgement operations. Worker outputs were deterministic fixture content.

The Windows Vulkan debug build on the RTX 4070 Laptop GPU measured 600 updates per layer at 1200×800 after 120 warm-up updates. Intelligence Network measured p50 8.61 ms, p95 9.65 ms and 513,736,704 process resident bytes. Swarm measured p50 8.68 ms, p95 9.47 ms and 535,744,512 process resident bytes. Three Windows samples per layer reported 261,419,008 dedicated and 127,860,736 shared GPU bytes. The timing describes Bevy updates and the memory describes the whole Atlas process.

Network showed the held lease, asked for confirmation and showed the cancellation at sequence 91. A separate signed inspection returned `cancelled`. Swarm showed the sender, recipient and accepted-message sequence. Freezing at sequence 179 held the same 44 observations while the live sequence advanced to 183. This verifies rendering and controls against broker state. Receipt visualization and broader resource visibility remain open.

## 2026-11-07 — Native Atlas timing and process memory

Verified checkpoint `2b29b4c2df8ce9ae25d02dc0187b0ca4b3a23ccc`, pushed to `main`. The world-scale checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42438908986).

Atlas now emits opt-in bounded update interval samples after 120 warm-up updates. Formatting, strict graphics Clippy, eight executable tests, one library test and the Windows graphics build passed. A debug build rendered at 1200×800 on an RTX 4070 Laptop GPU using Vulkan, NVIDIA driver 617.42 and requested FIFO presentation. The 600-update Machine sample measured p50 7.11 ms, p95 7.86 ms and 500,219,904 resident process bytes. The Living World sample measured p50 8.33 ms, p95 9.00 ms and 515,915,776 resident process bytes. Three Windows GPU Process Memory samples per layer each reported 261,414,912 dedicated and 126,812,160 shared bytes for the Atlas process. These timings measure app updates. The memory counters cover the whole view process with both layers loaded.

The same window rendered the live run's three residents at tick 24 and revision 30. Picking Rowan displayed his two relationships and committed model choice at revision 25. Freezing and stepping backward reached revision 29 while retaining Rowan's selection. The next graphics gate is measuring Network and Swarm with active sources and closing the remaining observability and control gaps.

## 2026-11-07 — World scale and independent resident replay

Verified checkpoint `07413f24bf0dd8a68855d39de4929e2c82f3eea4`, pushed to `main`. The external mailbox implementation also passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42438622628).

Windows formatting, strict workspace Clippy and the offline workspace suite passed. The new maximum-size fixture checked all 40 SQLite commits against independent engine state, including owned memories and relationships. Replay matched at revisions 2,560 and 5,120, and revision 128 returned the retention gap with earliest revision 1,024. One release run on Windows 11 and an Intel Core i9-14900HX measured 247 ms for direct ticks, 813 ms for SQLite commits including tick execution and 217 ms for latest replay. The database occupied 393,216 bytes. These timings describe one fixture with no inference or rendering.

Live Ollama 0.40.13 with `hermes3:8b` completed six scheduled moves across Aster, Rowan and Lark. Each resident took two turns under its own `simulation_act` grant. The CLI reopened and replayed the same public snapshot at tick 24, revision 30. Local operator reads recovered 24, 19 and 24 resident action records respectively. Each resident had two encounter peers. Public snapshots omitted owned histories. The provider acknowledged unload and reported zero resident models afterward. The next gate is native command-center observability and renderer hardware validation in Milestone 12.

## 2026-11-07 — Durable external worker inboxes

Verified implementation checkpoint `92b59939917bd0643414c991edc797bb182f310a`, pushed to `main`.

Broker schema 3 persists pending messages and bounded acknowledgement fingerprints alongside existing work, revocations and event feeds. Accepted enqueues and their operator events commit atomically. The current access file governs recovery. Startup drops entries and fingerprints whose directed routes have been removed.

Windows formatting, strict workspace Clippy and the offline workspace suite passed. A signed Rust SDK subprocess test verified version 2 migration, FIFO recovery, idempotent retries, conflicting payload rejection and permanent route removal. With a SQLite trigger rejecting a mailbox update, the broker returned `storage_unavailable` and fenced further reads. Restart recovered the previous inbox and event feed together. A separate test verified durable revocation despite a policy that still listed the revoked worker. These runs used local processes. Milestone 10 still needs an independent TLS host and explicitly provisioned trust root. The next locally actionable gate is Milestone 11 simulation reproducibility and scale.

## 2026-11-07 — Swarm status and durable outcome replay

Verified implementation checkpoint `9c66452fac73c941af9fae7245042842945c7709`, pushed to `main`.

Swarm now projects agent registrations and status changes and recognizes attempt and root outcomes from the durable workflow store. Historical projection filters all three kinds of evidence by the selected sequence. The Windows graphics suite passed seven executable tests, strict graphics Clippy and the graphics build. The headless workspace suite passed at the preceding registration checkpoint.

In the native Swarm window, two live Ollama workflows fed the same journal. The selected agent showed Running in frozen sequence 11 while the live journal advanced to 30. Returning live showed Idle at sequence 27 and the second workflow's Completed outcome. At sequence 13, the first attempt completion was visible and the workflow outcome appeared one step later. Ollama acknowledged both unloads. This verifies the bounded Milestone 9 event gates. The next gate is durable external inbox and cross-host interoperability in Milestone 10.

## 2026-11-07 — Runtime registration and agent status evidence

Verified implementation checkpoint `4ef3aa2996bdd41ccbf7c273bf53c0ae8ff7c5f3`, pushed to `main`.

Lifecycle journals now include the accepted scoped model inventory and effective agent registrations. Runtime status transitions include previous and new status and the task UUID when applicable. Schema 7 validates actual status changes. Windows formatting, strict Clippy and offline workspace tests passed. The scheduler lifecycle checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42437410255).

Live Ollama `hermes3:8b` completed a two-step workflow with ten lifecycle entries and five durable workflow events in the same journal. The lifecycle entries captured registration, both Running/Idle cycles and acknowledged unload. Final residency was empty. These are runtime-scoped observations. Next is verifying their graphical subscription and historical projection.

## 2026-11-07 — Scheduler lifecycle journal

Verified implementation checkpoint `d0349c0f10a812173ed81b3b752ac2e860815a84`, pushed to `main`.

`run-agent` and `run-workflow` can publish acknowledged explicit load and unload, managed inference entry and resource admission rejection events. Fixtures observed inference entry before completion, verified provider-managed inference without explicit residency claims, and withheld success events on failed load or unload. The 256-entry transport reported overflow. A journal failure reported omissions while the task completed and unloaded. Windows formatting, strict Clippy and offline workspace tests passed. The provider health checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42436776470).

Live Ollama `hermes3:8b` completed a two-step workflow. Replay showed one acknowledged activation, two inference entries correlated to their attempts, and one acknowledged deactivation. Final residency was empty. Lifecycle telemetry can report gaps and is separate from recoverable outcomes. Remaining Milestone 9 work includes registration and agent lifecycle events and their graphical subscriptions.

## 2026-11-07 — Provider health journal observations

Verified implementation checkpoint `9bdd8535c6369b26902bcd9297dcdefde159e98c`, pushed to `main`.

`provider-health --event-journal` publishes the provider UUID, effective state and age from each reported sample using event schema 6. The watch fixture observed unknown after a deadline, then unreachable and healthy after recovery. It checked replay against each report and verified redaction. An injected journal failure reported the affected round and preserved the printed healthy observation. Windows formatting, strict Clippy and offline workspace tests passed. The inbox checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42436460440).

A live Ollama poll produced one healthy journal entry and left residency at zero. These samples are expiring telemetry. A failed append leaves an explicit gap in journal history. Scheduler activation, unload and active inference events remain open under Milestone 9.

## 2026-11-07 — Recoverable agent inbox events

Verified implementation checkpoint `d9269f87ca2e7b45837e9ee77aba037c482d4e69`, pushed to `main`.

Durable inbox acceptance commits a metadata-only envelope with the message. CLI sends can publish it immediately, and scoped recovery pages can retry the original events. Fixtures injected a journal failure, reopened the accepted message, verified redaction, rejected a foreign cursor and preserved legacy inbox contents through migration. Four concurrent journal writers retried 128 unique events with no duplicate retained entries, then a slow reader detected retention loss. Windows formatting, strict Clippy and offline workspace tests passed. The workflow audit checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42435997113).

A live Ollama task was used as the correlation for a separately authorized local inbox send. Recovery left one journal entry and the message text stayed in the inbox. Acknowledged unload left zero residents. Provider observation coverage remains open in Milestone 9.

## 2026-11-07 — Recoverable workflow audit

Verified implementation checkpoint `6e12ccfcbb0992049c41e7486cb18ca90639f59b`, pushed to `main`.

Workflow checkpoints now commit accepted attempt, dependency, terminal attempt and root outcome envelopes alongside the report. Live publication uses the original envelopes. A journal insert failure left the audit recoverable, and a source checkpoint failure withheld the uncommitted root outcome. The killed-process fixture resumed a completed dependency with its original event IDs. Windows formatting, strict Clippy and offline workspace tests passed. The memory mutation checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42435462221).

Live Ollama `hermes3:8b` completed a two-step workflow. Live publication and recovery returned the same cursor, and replay contained five unique events with the recorded dependency. Acknowledged unload left zero residents. Legacy workflow rows remain readable without a recoverable audit. Agent communication, provider observations and concurrent recovery remain open under Milestone 9.

## 2026-11-07 — Durable memory mutation audit

Verified implementation checkpoint `221be81e798f0ae5b23e1a4f2f1d7af074f520ce`, pushed to `main`.

Memory writes, consolidation and actual deletes commit metadata-only events with the record transaction. The CLI can publish them immediately or recover up to 256 at a time using a scoped source cursor. Tests injected journal and source-audit write failures, verified rollback and duplicate-free retry while retained, and checked cursor expiry and restore scoping. Windows formatting, strict Clippy and offline workspace tests passed. The simulation tick checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42434876635).

Live Ollama generated a saved episode. Independent CLI invocations recovered the save event, published deletion, and replayed exactly those two entries after a recovery retry. The model unloaded and final residency was empty. Memory audit retains the latest 4,096 mutations, and restores preserve event UUIDs while assigning new cursor scope. Deletion envelopes require schema version 5. Broader workflow, agent communication and provider audit remain open.

## 2026-11-07 — Recoverable simulation tick events

Verified implementation checkpoint `082c921c3540a38548acf3215cfe6b3397ede7b4`, pushed to `main`.

Tick-advancing world commits retain the original simulation envelope with a stable simulation UUID. `world advance` and `world run` can publish to an existing event journal. An injected journal failure left a committed tick and recoverable envelope. An injected world update failure rolled both back. Tests also checked batching, ordered cadence, idempotent recovery and the 512-advance retention limit. Windows formatting, strict Clippy and offline workspace tests passed. The agent terminal checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42434453232).

Live Ollama `hermes3:8b` selected a legal action after two ticks. The resulting world revision was 3, the journal contained exactly two simulation events, and replay after recovering revision 2 still contained the original two entries. Unload completed and final residency was empty. The simulation audit covers tick advancement. Non-tick controls and model decisions retain their separate world history evidence. Broader lifecycle coverage and recovery under load remain open.

## 2026-11-07 — Recoverable agent terminal events

Verified implementation checkpoint `06eb99408e908f493a95c9ed3444905de6d10dcc`, pushed to `main`.

New terminal task checkpoints commit the sanitized event envelope, outcome and task state in one SQLite update. An injected journal insert failure left a completed task and recoverable event. Independent CLI invocations published the exact envelope, and retries reused its cursor while retained. A separate injected task checkpoint failure left no terminal envelope. Windows formatting, strict Clippy and offline workspace tests passed. The previous backup checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42434059504).

Live Ollama completed and unloaded a task. Two recovery invocations produced one replay entry with the original event UUID and cursor. The source event is stored for new terminal checkpoints only. Journal retention can remove an entry, and explicit recovery can then append the same event UUID at a new cursor. Broader lifecycle audit and replay remain open in Milestone 9.

## 2026-11-07 — Verified SQLite memory backup and restore

Verified implementation checkpoint `a5617d48dabb82b9f14023e94a051eca7f525d78`, pushed to `main`.

`memory-admin backup` and `restore` stage, verify and publish a full database at a new path. Fixtures restored committed WAL data for two namespaces, preserved FTS, lineage and owner counters, and rejected a damaged index, existing destination and stale sidecar. Windows formatting, strict Clippy and offline workspace tests passed. The previous task recovery checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42433564636).

Live Ollama generated a task response that was saved with durable provenance. Independent backup, restore and read commands reopened the exact response and source UUID. Another live task included an authorized record from the restored database. Both inferences acknowledged unload and the final residency sample was empty. This verifies the original local SQLite Milestone 8 gates. Whole-database operations require a trusted filesystem operator and a private hard-link-capable destination directory. Agent task and workflow histories remain separate databases. Next is Milestone 9 durable event coordination.

## 2026-11-07 — Durable task result recovery into memory

Verified implementation checkpoint `72090611aedc0ef870db7c7b8e45cdad8666453c`, pushed to `main`.

`memory remember-task` uses the configured `memory_write` grant and private task ledger to copy the exact response of an owned, completed task into generated episodic memory. CLI fixtures checked current permissions, wrong owner, absent source, duplicate memory identity, exact content and independent reopen without provider traffic. Windows formatting, strict Clippy and the offline workspace suite passed. The planning checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42433155431).

Live Ollama `hermes3:8b` completed and unloaded a task. Independent memory commands reopened the retained result and wrote and read the linked episode with a configuration whose custom provider could not run inference. Final Ollama residency was empty. The two databases are written separately, and a caller can retry with the same memory UUID without replacing a committed record. Milestone 8 still requires verified backup and restore.

## 2026-11-07 — Integrated agent recovery acceptance

Verified against `921301e` on `main`. Independent Windows CLI processes recorded relationship memory, completed a task on Ollama `hermes3:8b`, and linked an incoming message to the task. A durable hold remained in effect after the configuration switched the same agent UUID to `igorls/gemma-4-12B-it-heretic-GGUF:latest`. After release, a new process used the retained relationship record in a successful inference on the replacement model. Separate history and inbox reads returned the original owner, original model and task-linked message. Both models acknowledged unload and final residency was empty.

The original bounded Milestone 7 gates are verified. Configuration remains the authority for model binding and grants. Task and inbox durability require the private task store, and relationship observations use the separate memory store. Next is Milestone 8 backup, restore and durable memory lineage.

## 2026-11-07 — Bounded agent planning and durable provenance

Verified implementation checkpoint `dba89a21de6e142c1ddd5d7f0eb3061e5152c48a`, pushed to `main`.

Bounded decomposition now sends the configured planner profile as structured input. Proposal validation and operator limits still govern execution. Workflow history retains the planning task, agent, model and provider identifiers from the first accepted plan checkpoint. Existing reports without planning provenance remain readable. Windows formatting, strict Clippy and the offline workspace suite passed. Tests inspected the actual planner request and reopened both stores. The inbox checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42432684113).

In live Ollama, a durable hold prevented planning. After release, the configured `hermes3:8b` agent planned and executed two dependent steps as sole worker. Independent history commands reopened its plan, planning provenance, prompt and owner. Acknowledged unload left zero residents. The profile goes to the selected planning provider and is retained in the optional private agent task store. A process interruption before acceptance can leave a planning task without a workflow checkpoint. Integrated relationship and task recovery after model replacement remains open.

## 2026-11-07 — Permission-gated durable agent inboxes

Verified implementation checkpoint `1182d0499124f4c1cb42b8e8b2093c7e9b574bcd`, pushed to `main`.

The task store now retains ordered, permission-gated agent messages. A subprocess test linked messages to accepted task ownership, reopened the recipient's inbox, checked namespace isolation, and rejected an unrelated task reference, a held sender and revoked grants. Inbox operations generated no provider requests. Runtime coverage also verified retention after model rebinding, enforcement of the inbox limit after restart and migration from versions 1 and 2. Windows formatting, strict workspace Clippy and offline workspace tests passed. The previous operator suspension checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42432256462).

Live Ollama `hermes3:8b` completed and unloaded a task. Separate CLI processes sent a message linked to the accepted task and reopened the recipient's inbox after its model selection policy changed. The final Ollama residency sample was empty.

Inboxes are bounded, append-only and scoped to a private local task database. The offline operator supplies the current configuration and agent identity. Milestone 7 still needs integrated recovery and verification of identity and objectives in bounded planning.

## 2026-11-07 — Durable agent operator suspension

Verified implementation checkpoint `3b61707120f85133a71e86ec5041a9c9188c0c49`, pushed to `main`.

`agent-state suspend`, `inspect` and `release` work offline against the private task database. Migration preserves version 1 task history. Independent processes verified that a hold defeats `--resume` for direct execution and coordinator activation before provider traffic. An authored workflow completed using an eligible alternate when another candidate was held. The runtime independently enforces the hold after attachment. Windows formatting, strict workspace Clippy and offline workspace tests passed. The preceding task ownership checkpoint also passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42431748791).

With live Ollama and `hermes3:8b`, the held agent was denied, then completed after release and an explicit invocation resume. Acknowledged unload left zero resident models. Holds are scoped to the shared private task database, and control changes are issued between runs. Configuration still supplies identity, grants and model policy. Durable authorized inboxes and bounded autonomous planning remain open for Milestone 7.

## 2026-11-07 — Durable agent task ownership

Verified implementation checkpoint `ad458955821fae77629c8e01307668a893991dcf` was pushed to `main`. Automatic decomposition passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42431100527).

`run-agent` and `run-workflow` can keep task ownership and managed outcomes in a private SQLite database. A killed subprocess left a running checkpoint with its original owner, and replay of the ID was rejected before provider traffic. The delegated workflow fixture retained the worker separately from the coordinator. An injected outcome write failure released the owned lease and left the prior durable checkpoint uncertain. Formatting, strict workspace Clippy and Windows offline workspace tests passed.

On Ollama 0.40.13, a `hermes3:8b` task completed and unloaded. An independent `agent-task-history` invocation returned the same owner and exact result, and the task ID was fenced on reuse. Final residency was empty. Agent status and inboxes still live in the process. Persistent operator status is the next Milestone 7 checkpoint.

## 2026-11-07 — Bounded automatic workflow decomposition

Verified implementation checkpoint `73c68698289f2f62e63fe59393efbae8c5ac4706` was pushed to `main`. Workflow resume passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42430453474).

`run-workflow --decomposition-file` makes one bounded coordinator planning call and validates the resulting steps against the operator roster, configured capability policies and supplied bounds. The subprocess fixture verified two dependent steps on an authorized worker and no worker traffic for proposals with an extra candidate field, an unsupported capability or a disconnected step. Formatting, strict workspace Clippy and Windows offline workspace tests passed.

On Ollama 0.40.13, `hermes3:8b` proposed a description step and a dependent summary step. Both completed, `workflow-history` reopened two attempts, and acknowledged unload left residency empty. Execution JSON records the planning call identity. The durable store keeps the accepted plan and worker lineage. The bounded native Milestone 6 gates are verified. Milestone 7 proceeds with persistent agent state and task ownership.

## 2026-11-07 — Resume from a committed workflow step

Verified implementation checkpoint `1dea00ddb0a6e6e48be84ceaf6485314f3e4f9df` was pushed to `main`. Durable workflow lineage passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42429802294).

`--resume-workflow` continues an interrupted workflow only with its original validated plan, an intact completed step prefix, and time remaining on its original deadline. The subprocess fixture killed the first invocation after its first commit. The second invocation reused that dependency and its task ID and issued only the pending inference. Another killed-process fixture rejected resume with an unresolved attempt. The tests also covered a changed plan, expired and rolled-back deadlines, and version 1 history inspection without resume eligibility.

The Windows offline workspace suite, strict Clippy and formatting passed. A live Ollama 0.40.13 workflow completed and rejected a terminal resume, with empty final residency. The execution policy rejected the proposed live proxy interruption harness, so successful interruption recovery is verified by the subprocess fixture. Automatic decomposition is the next Milestone 6 gate.

## 2026-11-07 — Durable workflow lineage

Verified implementation checkpoint `7ba48173d74725d657977f7b56079de3e34be8c5` was pushed to `main`. The NVIDIA admission checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42429199750).

`run-workflow --workflow-store` retains the plan, root state, delegation intent, accepted attempts and outcomes in a bounded private SQLite database. A subprocess fixture killed the workflow during its second provider call. Reopen retained the first result and second delegation without claiming a second outcome. The same root was fenced before provider requests, and inspection during the active invocation returned a busy error. An injected result checkpoint failure prevented the dependent call.

On Windows, a live two-step Ollama 0.40.13 workflow using `hermes3:8b` completed and acknowledged unload. `workflow-history` reopened the database and returned the exact terminal report from execution. Formatting, strict workspace Clippy and offline workspace tests passed. The store distinguishes inspection after interruption from re-execution. Safe resume from committed boundaries and automatic decomposition remain Milestone 6 work.

## 2026-11-07 — NVIDIA VRAM admission limit

Verified implementation checkpoint `69ddd91f1ec9279fdb30756c7d367a00fb236d0c` was pushed to `main`. Observed-latency routing passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42428793122).

Opt-in `[scheduling].ollama_gpu_uuid` uses NVML free VRAM to tighten the startup ceiling for a single native loopback Ollama provider. A fixture verified that an unobservable device and an incompatible provider scope fail before provider traffic. Unit tests covered a fully occupied device, a smaller measured ceiling and preservation of a smaller declared ceiling. Windows formatting, strict workspace Clippy and offline workspace tests passed.

On the Windows RTX 4070 Laptop GPU, NVML reported 8,585,740,288 total bytes and 6,756,495,360 free bytes. Ollama 0.40.13 rejected an 8 GiB scheduler estimate before loading `hermes3:8b`. With a 5 GiB estimate the same task completed and acknowledged unload. Residency was empty afterward. The operator supplies device attribution, and the measurements describe startup headroom. The Milestone 5 gates are verified in the documented native scope. Milestone 6 proceeds with durable workflow lineage and interruption recovery.

## 2026-11-07 — Observed-latency model routing

Verified implementation checkpoint `78dacbd8703addc0c132d630ed64175076fa57c2` was pushed to `main`. Local host admission limits passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42428151048).

Opt-in `[scheduling].prefer_observed_latency` ranks eligible capability-based models from recent successful provider call timings. The workflow fixture verified stable cold ordering, preference after both models had observations, and fallback when output bounds changed. Fixed assignments and the authored agent order retain their existing behavior. Windows formatting, strict workspace Clippy and offline workspace tests passed.

On Ollama 0.40.13, a workflow sampled `codestral:22b-v0.1-q2_K` at 12,039 ms and `hermes3:8b` at 116 ms of client-observed provider call time. The third step selected Hermes and completed in a 76 ms provider call. Final residency was empty. A prior Gemma sample exhausted 128 output tokens before visible text and was excluded. The timings describe these calls under their observed conditions. Provider GPU capacity and device attribution remain open.

## 2026-11-07 — Local host admission limits

Verified implementation checkpoint `005cff016a2e6ea19f5f27640451c8328e652592` was pushed to `main`. The configurable scheduler policy passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42427577547).

Opt-in `[scheduling].local_host_limits` tightens the startup RAM and concurrency ceilings for loopback provider scopes. The CLI exposes the observations and effective budget, and scheduler telemetry uses the effective ceilings. A fixture demonstrated rejection of a RAM cost within the declaration but outside measured headroom, without activation. A separate case rejected a remote provider scope. Unit tests cover optional process cgroup headroom, zero capacity and preservation of smaller declared limits.

On Windows, Ollama 0.40.13 and `hermes3:8b` completed a request under a sampled 18,130,427,904-byte RAM ceiling and 32 inference slots. The configured VRAM ceiling stayed at 6 GiB. Acknowledged unload left residency empty. Windows formatting, strict workspace Clippy and offline workspace tests passed. Linux cgroup discovery awaits Ubuntu execution. Startup measurements can become stale, and the local sample does not identify provider GPU capacity. Latency-aware routing remains the next actionable scheduler gate.

## 2026-11-07 — Configurable scheduler policy

Verified implementation checkpoint `742fc1137320c026eaa15d5f3f757f4c36130bad` was pushed to `main`. The durable residency journal passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42427116568).

The optional TOML scheduling table configures queue capacity, priority bypasses, per-model priority, native keep-alive and pressure eviction. Runtime fixtures exercised configured queue rejection, priority overtaking, lower-priority eviction, and budget rejection with pressure eviction disabled. The CLI fixture verified that scoped configuration drops unrelated priorities and that native preload and inference both carry the configured retention.

On Windows, Ollama 0.40.13 and `hermes3:8b` completed a request with a 12-second retention hint. The first residency sample reported 11.94 seconds remaining, and the model expired without an unload request. Formatting, strict workspace Clippy and the offline workspace suite passed. The configured policy still depends on declared budgets and estimates. Hardware and latency evidence are the next scheduler gates.

## 2026-11-07 — Durable scheduler residency intent

Verified implementation checkpoint `5ca35981273f683d73450acf2ac129925c457403` was pushed to `main`. The admission residency checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42426565208).

`--residency-journal` commits lifecycle intent before activation and retires it after acknowledged unload. A CLI fixture killed a process with a load request in flight. It verified exclusive access while that process lived, a durable fence after restart, continued memory pressure when the old model was absent, and no automatic unload of a reported resident. An acknowledgement cleared only an observed-absent record, and a subsequent acknowledged unload retired the new intent. The fixture also verified rejection of an unrelated file without modifying it or contacting the provider.

With Ollama 0.40.13 and `hermes3:8b`, a separate invocation left a model resident. Reopen refused to take ownership, including with recovery acknowledgement while the model was still present. Operator unload left the durable fence intact until acknowledgement. The recovery invocation and an ordinary reopen then completed, with final residency empty. Windows formatting, strict workspace Clippy and offline workspace tests passed. Journals coordinate only their own invocations, and operator acknowledgement is required for unresolved intent. The next scheduler work is exposing configurable model priorities and lifecycle policies.

## 2026-11-07 — External residency checks before admission

Verified implementation checkpoint `7f75c0b14b56701aa608bedf800cd3e7ec957634` was pushed to `main`. The per-inference allowance checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42426014923).

`--reconcile-residency` captures startup observations and refreshes external memory pressure before each admission. JSON keeps the baseline and latest complete admission observation separate. A two-step subprocess fixture introduced an external model after the first step started. With 10 reported external RAM bytes and a 100-byte local reservation under a 120-byte budget, both steps completed. With 90 external bytes the second step was rejected before inference. Only the local model received lifecycle requests. A scheduler test also verified that an unacknowledged load retains its charge and cannot be automatically unloaded in reconciliation mode.

On Windows, a live two-step workflow against Ollama 0.40.13 and `hermes3:8b` completed with zero external models in both reports. Acknowledged unload left the provider residency list empty. Changing external pressure was exercised by the fixture. Formatting, strict workspace Clippy and the offline workspace suite passed. The probe cannot prove provider-wide ownership or observe changes during active inference. Durable lifecycle intent and restart recovery are the next scheduler gates.

## 2026-11-07 — Per-inference memory allowances

Verified implementation checkpoint `ce1cf5c58847efbc454276b8d96e84d4bb8730b4` was pushed to `main`. The context-aware scheduler checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42425514575).

The optional `[inference_overhead]` configuration adds fixed and per-context-token RAM and VRAM allowances to each local inference from preparation until lease release. Reused models share one resident reservation and still charge each lease. A failed or interrupted load returns the transient charge while retaining uncertain resident memory. The scheduler fixtures exercised separate RAM and VRAM budget failures, idempotent release, drop and abort. The CLI telemetry fixture observed the allowance during both loading and active execution. Windows formatting, strict workspace Clippy and offline workspace tests passed.

With Ollama 0.40.13 and `hermes3:8b`, a 512-token request was rejected at 7 GiB of configured VRAM costs against a 6 GiB budget, leaving residency empty. With the configured transient allowance reduced to 64 MiB, the request completed and explicit unload left residency empty again. These allowances are operator estimates. Automatic estimation and provider-side memory verification remain open. The next Milestone 5 work is refreshing external pressure between admissions and preserving uncertainty across process restart.

## 2026-11-07 — Context-aware scheduler reuse

Verified implementation checkpoint `680f9d839726a88c43038c08ac70e32a90ddf225` was pushed to `main`. The startup residency checkpoint passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42425220762).

An explicitly loaded model now retains its requested context with its reservation. An active lease fences a context change. An idle change requires an acknowledged unload before reloading. An impossible request fails before any pressure eviction. The scheduler regression exercised active rejection, idle replacement and an unacknowledged unload that retained memory without loading a replacement. Formatting, strict workspace Clippy and the Windows offline workspace suite passed.

On Windows with Ollama 0.40.13 and `hermes3:8b`, a single workflow completed both steps at requested contexts of 512 and 1,024 tokens. A provider query after the workflow reported a 1,024-token allocation and 4,614,782,975 VRAM bytes. Operator cleanup left the residency list empty. This sample verifies the mixed-context path and final allocation. Automatic KV-cache costing and provider-managed context verification remain open.

## 2026-11-07 — Startup provider residency reconciliation

Verified implementation checkpoint `aa38cf6a09ebc033781c4637d35b6bdc7cdd4d1d` was pushed to `main`. The preceding provider health watch passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42424520669).

`run-agent` and `run-workflow` can now opt into a startup residency observation. The scheduler charges already resident models against its headroom, fences matching model names, and keeps observed models outside its owned cleanup set. Windows formatting, strict workspace Clippy and the offline workspace suite passed. A CLI fixture verified a RAM pressure rejection, an Ollama alias ownership rejection and successful cold execution with acknowledged cleanup.

Live Windows Ollama 0.40.13 reported one externally preloaded `hermes3:8b` model at 4,547,149,823 VRAM bytes. The CLI rejected admission, reported no cleanup actions and left that model resident. After operator unload, a cold invocation completed successfully and acknowledged its own unload. The final provider list was empty.

The observation is scoped, opt-in and taken once per invocation. Continuous pressure and durable ownership reconciliation remain open. Next assess context changes across scheduler reuse and the assumptions behind per-request memory admission.

## 2026-11-07 — Continuous provider health monitoring

Verified implementation checkpoint `064971760a3689f1684ab2c24e100e6675f5d891` was pushed to `main`.

The durable catalog checkpoint `963533e2aa604f126a0c3a39b136633b0307b0ee` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42424232656). The CLI can now keep one registry alive across health rounds, bound each round by a deadline, and stream JSON observations until Ctrl+C or a sample limit. A timeout marks the round incomplete and does not replace unfinished observations with a fabricated failure.

The Windows subprocess regression observed a timed-out round with unknown health, a subsequent unreachable state, and recovery to healthy in the same process. The fixture received exactly three health requests. A separate live Ollama check reported three healthy rounds, with zero resident models both before and after. Formatting, strict workspace Clippy and offline workspace tests passed.

The monitor reads its configuration at startup and keeps its health observations local to the process. Startup lifecycle reconciliation remains separate from health monitoring.

## 2026-11-07 — Durable model catalog

Verified implementation checkpoint `963533e2aa604f126a0c3a39b136633b0307b0ee` was pushed to `main`. The Windows offline workspace suite, formatting, strict Clippy and Rustdoc with warnings denied passed.

The Bionic compatibility checkpoint `f186bbc350c54797d054760e820bbd62a6c46bef` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42423469770). Added an opt-in SQLite catalog for model metadata. CLI edits validate the effective provider and agent references and commit behind a revision check. Retired UUIDs keep their original provider binding. Runtime commands load a committed catalog snapshot before building adapters.

The Windows CLI regression covered restart, concurrent edits, rejection of a foreign database, references that prevent removal, stale revisions and failed rebinding after removal. It also ran a local HTTP inference fixture using resource metadata supplied by the reopened catalog. In a separate live test, Ollama 0.40.13 and `hermes3:8b` returned `OK` using catalog revision 1 and the same model UUID. Acknowledged unload left zero resident models.

Providers and agent policies continue to come from TOML. The catalog persists declarations and stable model bindings. Background health monitoring and startup residency reconciliation remain open.

## 2026-11-07 — LM Studio Bionic compatibility

Verified checkpoint `f186bbc350c54797d054760e820bbd62a6c46bef` was pushed to `main`.

The native Ollama and milestone audit checkpoint `62974b0297eb04aac4811cd67422f83188cfc111` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42422837461). The installed LM Studio daemon identified itself as Bionic v1.2.0+2. Its documented loopback OpenAI API passed the opt-in Rust adapter check with a preloaded Gemma 4 12B Q4_K_S model. Health and model enumeration succeeded. Bounded chat reported 56 completion tokens, and streaming emitted two visible deltas before its terminal event. A separate stream returned typed local cancellation after a visible delta.

The first 32-token run exhausted its budget before visible text. The final test used 128 tokens for completion and 256 for the cancellation probe. The streaming response carried no terminal token count. The run used an unauthenticated loopback server. Auth, unsupported operations and malformed transport remain covered by offline fixtures. LM Studio Bionic uses the existing compatible adapter. The record does not qualify BionicGPT or assert that cancelled server compute stopped.

The same CLI agent then ran through Bionic with no code changes to routing or scheduling. It completed with visible text and 62 reported output tokens. The temporary load was released through the vendor CLI, and the loopback server was stopped. Offline workspace tests, strict Clippy and formatting passed.

## 2026-11-07 — Milestone audit and native Ollama streaming

Verified checkpoint `62974b0297eb04aac4811cd67422f83188cfc111` was pushed to `main`.

Audited Milestones 0–14 from the clean `7a7edba` baseline against the original gates. The [acceptance ledger](MILESTONE_LEDGER.md) records evidence, gaps and dependency-ordered actions. The Atlas benchmark was at inspection stage and was deferred without source changes.

The opt-in Rust test passed on Windows against Ollama 0.40.13 and installed `hermes3:8b`. It verified live health, model listing, metadata, explicit load, 9 text deltas and a terminal event reporting 10 tokens, typed local cancellation after a text delta, and acknowledged unload. The backend reported 512 allocated context tokens and 4,547,149,823 VRAM bytes after load, and the model was absent after unload. Twelve Ollama offline fixtures passed. Server compute cessation and other model/backend versions were outside this run.

## 2026-11-07 — Request memory in Atlas

Verified implementation checkpoint `1c8e6eab3aa9c42451c9cb17065631e71ab2b86c` was pushed to `main` and passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42422206226).

Machine shows a green node when a validated local inference request includes retrieved memory. The count is attached to the exact running lease after request construction. The task inspector distinguishes zero included records from unavailable evidence. Version 4 carries counts, and the viewer still accepts versions 1 through 3. Memory queries, content and record IDs stay out of telemetry.

Windows validation included the offline workspace suite, the SDK doctest, seven graphics tests, strict workspace and graphics Clippy, and both builds. In a native Atlas session, a held provider request produced one memory node on the active task. The CLI result recorded one included memory ID, while the live telemetry carried only its count. The final sample had no task leases. The regression compared the live count with the provider request and verified that an oversized retrieved record did not enter either.

The previous [Ubuntu run](https://github.com/ShamelesAbyss/Vespera/actions/runs/42421611995) failed the World reasoning heartbeat test when it missed the active phase. The fixture now gates its health and inference responses on observation of the corresponding phase and reports subprocess errors before asserting missing evidence. Memory nodes cover the current local request. Memory lineage and historical visualization remain open.

## 2026-11-07 — Local task ownership in Atlas

Verified implementation checkpoint `df39d84d72a02b267fbd800ceee9546b940d3782` was pushed to `main`.

The Machine reservation checkpoint `c8b0f5db949231b63a8ece0ea278d452a47c6e2d` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42420881117). The opt-in scheduler sample now pairs running task and agent IDs with their confirmed local lease, model and provider IDs. Machine renders a task node only when the scheduler still accounts for that exact lease. The sample is bounded, and the viewer still accepts versions 1 and 2.

Windows validation covered the offline workspace suite, the SDK doctest, seven graphics tests, strict workspace and graphics Clippy, and the graphics and CLI builds. The CLI regression compared the sampled lease and task IDs against the execution report, exercised both older formats, and rejected mismatched providers and duplicate ownership. In a native Atlas session driven by a held workflow fixture, Machine and Swarm showed the same attempt UUID. The selected owner transferred to the matching Living World resident, and returning from Swarm selected the corresponding live Machine task. The final sample contained zero tasks and reservations after cleanup.

This graph covers live ownership for one opted-in local runtime. A task can finish between samples, and ownership rows are retained only for reservations visible in the sample. It omits queued identities, provider-side execution progress and historical local task ownership.

## 2026-11-07 — Model and provider reservations in Machine

Verified implementation checkpoint `c8b0f5db949231b63a8ece0ea278d452a47c6e2d` was pushed to `main`.

The unified Atlas checkpoint `fbb47a8f9ab30b0ba4209fceae0e75c92c01ac1c` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42419896754). Machine now connects the scheduler to providers and model reservations observed in one opted-in runtime. Each model shows its lifecycle state, charged slots, issued leases and estimated memory. Version 2 telemetry includes up to 16 model rows, with charged reservations first. Version 1 still displays aggregate counts.

Windows validation included the offline workspace suite, the SDK doctest, seven graphics tests, strict workspace and graphics Clippy, and the graphics build. A held Ollama-compatible HTTP fixture let the native Atlas window observe an amber loading model with one charged slot and zero issued leases. After load acknowledgement, the same model and provider showed one issued lease and a cyan link. Acknowledged unload removed both nodes and the final sample reported zero reservations. The CLI regression matched both UUIDs, lifecycle and accounting values, verified the legacy schema, and checked final cleanup.

The graph covers one local scheduler and the providers represented by its displayed reservations. Physical VRAM, provider-side progress and local task ownership are outside this sample. Task-to-model links need an ownership projection from the agent runtime.

## 2026-11-07 — Unified native Atlas navigation

Verified implementation checkpoint `fbb47a8f9ab30b0ba4209fceae0e75c92c01ac1c` was pushed to `main`.

The signed task replay checkpoint `b6988a6866a5c9b4d259cb86000674a9b06dd327` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42419091459). Added a single-window Atlas mode for Machine, Intelligence Network, Swarm and Living World. Layer switches retain cameras, filters and replay state. A selected agent UUID transfers when it appears in the destination. The active scene handles input. Broker observation continues in the background, and switching away from World pauses window-driven ticks.

Windows validation passed the offline workspace suite, the SDK doctest, seven graphics tests, strict workspace and graphics Clippy, and the graphics build. In the native window, a worker selected from a signed lease appeared as the selected Swarm sender and the corresponding World resident. The broker accepted a cancellation while Network was hidden, and the world database stayed paused at revision 8 while another layer was open. The World inspector retained its replay revision on return. Signed Swarm replay hid the message at sequence 0 and restored its arrow at the accepted sequence 3.

Atlas opens only the configured layers and transfers only identities already observed by the destination. The session keeps layer context in memory. Model, provider and hardware drill-downs require more cross-source evidence.

## 2026-11-07 — Intelligence Network task replay

Verified implementation checkpoint `b6988a6866a5c9b4d259cb86000674a9b06dd327` was pushed to `main`.

The Swarm timeline checkpoint `47ba366f0d652276ab4ecf48d08e26e59b501bda` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42418445608). The signed Intelligence Network now captures up to 256 task transitions from a rolling baseline. The live feed continues during replay. The inspector preserves task selection even before that task appears. Historical controls suppress new cancellation requests and dismiss confirmations that have yet to be sent.

Windows validation passed the offline workspace suite, the SDK doctest, six graphics tests, strict workspace and graphics Clippy, and the graphics build. The regression covered rollover, a live source reset during capture and cancellation confirmation across a replay boundary. In a native Windows session connected to a signed loopback broker, the selected task stayed queued at sequence 1 while the live source reported sequence 2. The broker confirmed that pressing Y during and after replay did not send the old confirmation. Stepping back to sequence 0 removed the task from the scene while retaining its identity in the inspector. Stepping to 2 restored the selected task in the cancelled lane.

Replay covers task transitions observed in the current viewer session. It can retain an already captured window through a feed reset. Shared navigation across the Atlas layers remains open.

## 2026-11-07 — Interactive Swarm evidence timeline

Verified implementation checkpoint `47ba366f0d652276ab4ecf48d08e26e59b501bda` was pushed to `main`.

The World Mode timeline checkpoint `034a7030a0ad851d3b96f4c02617322a3060abf7` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42350667278). Swarm now captures a bounded view of accepted relations and terminal observations. The brackets step across retained evidence, and the live source keeps advancing while the inspector displays an earlier sequence. Compound acceptance events retain their delegation and dependency edges together. A source reset starts a separate live view.

Windows validation passed 224 offline workspace tests, the SDK doctest, five graphics tests, strict workspace and graphics Clippy, and the graphics build. In the native journal-backed view, a held local provider fixture left the Swarm inspector frozen on sequence 1 while the backend advanced to sequence 5. Returning live and stepping between sequences 4 and 5 preserved the selected agent and showed the workflow outcome only at 5. The graphics regression also covered the shared message projection and bounded eviction.

The signed broker and journal windows still use separate sources. The native timeline was exercised with the journal. The signed broker timeline lacks a dedicated desktop smoke result for this checkpoint. The broker task view and cross-layer navigation remain open.

## 2026-11-06 — Interactive World Mode timeline

Verified implementation checkpoint `034a7030a0ad851d3b96f4c02617322a3060abf7` was pushed to `main`.

The world replay checkpoint `5b77ab43cbfda85997055208a08a9cdea1ecf71a` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42350207043). World Mode can freeze the current display, step to the nearest retained commit and return to live observation. The inspector reports the replay range and skipped revisions. Historical controls are fenced in both the window and its worker. Resident selection and camera state stay in the same scene.

Windows validation passed 224 offline workspace tests, the SDK doctest, four graphics tests, strict workspace and graphics Clippy, and the graphics build. The retention test exercised neighbors around a pruned boundary. The graphics worker test verified skipped batch interiors, rejected historical writes and return to a newer live revision. In the native window, Aster remained selected while stepping from revision 3 to 1. A separate CLI commit advanced the live world to 4 while the window stayed at 1. The next step displayed 3 and T returned to 4.

The timeline covers World Mode commits. Graphical timelines for broker and workflow evidence and unified layer navigation remain open.

## 2026-11-06 — Deterministic replay of committed world revisions

Verified implementation checkpoint `5b77ab43cbfda85997055208a08a9cdea1ecf71a` was pushed to `main`.

The live reasoning checkpoint `15c9f8366e0e21ffa30c115150d12682d151d245` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42349595201). World updates now commit typed changes and state together. A bounded history rolls forward a replay checkpoint as older records expire. The CLI reconstructs a retained commit without a provider. World Mode can open a fixed historical revision for read-only inspection.

Windows validation passed 224 offline workspace tests, the SDK doctest, strict workspace and graphics Clippy, graphics tests and the graphics build. Tests compared reconstructed and current state including committed model provenance, verified retention rollover, and rejected revisions between grouped tick commits. A real Ollama choice was committed at revision 1 and the world advanced to revision 3. The historical 3D view showed Aster's revision 1 action at tick 0. The database remained byte-identical after inspection and attempted control input.

Replay starts from an available checkpoint and covers retained commit boundaries. Older worlds cannot reconstruct changes from before recording, and runtime heartbeat data stays outside history. The historical graphical view is pinned at launch. Interactive timeline navigation and the shared Atlas entry point remain open.

## 2026-11-06 — Live resident reasoning evidence

Verified implementation checkpoint `15c9f8366e0e21ffa30c115150d12682d151d245` was pushed to `main`.

The cadence checkpoint `ccdb4c4328354732fb8b8e0afe7e3c7506dfe6d0` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42348974830). Scheduled runs publish a short-lived heartbeat in the world database. The marker describes preparation or a sampled admitted local inference lease. It leaves the tick and revision unchanged. World Mode reads it alongside simulation state and distinguishes it from a committed model choice.

Windows validation passed 223 offline workspace tests, the SDK doctest, strict workspace and graphics Clippy, graphics tests and the graphics build. The CLI fixture observed both phases for the same task, verified heartbeat expiry and matched the final task to the persisted choice. A controlled provider fixture showed the amber preparation marker at revision 1, the cyan lease marker during the held response and the committed choice at revision 2. The tick stayed at 1 during all three observations.

The heartbeat describes local runtime activity from `world run`. Short phases can fall between samples, and provider-side progress is unavailable. Deterministic replay of recorded decisions and unified Atlas navigation remain open.

## 2026-11-06 — Bounded resident reasoning cadence

Verified implementation checkpoint `ccdb4c4328354732fb8b8e0afe7e3c7506dfe6d0` was pushed to `main`.

The individual model-decision checkpoint `c034dd7c33f54ba7bdbd3836968379f9ba62bc75` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42348529835). Added `world run` to commit a finite series of deterministic ticks and rotate sparse model choices among explicitly approved residents. One scheduler and provider registry serve the roster. Failed or invalid responses leave the world unchanged at that boundary and deterministic ticks resume. Every write is fenced against the observed world.

Windows validation passed 222 offline workspace tests, the SDK doctest, formatting and strict workspace Clippy. The HTTP integration exercised two residents with one shared model load and unload. It compared a run with two invalid choices to a deterministic-only world and found identical persisted state. A real Ollama 0.40.13 / hermes3:8b run committed five ticks, with Aster's and Lark's choices at revisions 3 and 6. The open 3D view showed tick 5, revision 7 and each resident's committed provenance. Unload was acknowledged and the backend reported zero resident models afterward.

Reasoning boundaries wait up to their individual deadlines. The CLI reports completed progress and each attempt, while the world stores each resident's latest committed decision. Live queued and active reasoning states and deterministic decision replay remain open.

## 2026-11-06 — Revision-fenced model decisions in Living World

Verified implementation checkpoint `c034dd7c33f54ba7bdbd3836968379f9ba62bc75` was pushed to `main`.

The in-flight Swarm checkpoint `8834c8ad0662414359f5977eefd24e60a4b3d734` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42347919796). Added `world decide` with explicit resident identity, `simulation_act` authorization and bounded scheduler inference. The world offers a legal action menu and accepts a strict choice index. Preview is read-only. An applied choice commits against the original world revision, records provenance and refreshes the 3D inspector. Deterministic ticks still operate independently.

Windows validation passed 221 offline workspace tests, the SDK doctest, graphics tests, strict workspace and graphics Clippy, and the graphics build. Tests covered role and resource accounting, missing permissions before provider calls, preview, malformed output, committed provenance and a concurrent tick that rejected stale inference. A real Ollama 0.40.13 / hermes3:8b run moved Aster from (4, 5) to (5, 5) at revision 1, tick 0. The open read-only Windows view displayed the move and model ID. Native unload was acknowledged, and the backend reported zero resident models afterward.

The model choice is operator-triggered. Autonomous cadence, queued and active reasoning evidence, replay of committed decisions, and unified layer navigation remain open. Older builds cannot read a world after it contains model-decision provenance.

## 2026-11-06 — In-flight workflow evidence in Swarm

Verified implementation checkpoint `8834c8ad0662414359f5977eefd24e60a4b3d734` was pushed to `main`.

The signed mailbox checkpoint `2060d53ecf0284f7a58b9f9d7cce497dca19e6ec` passed [Ubuntu CI](https://github.com/ShamelesAbyss/Vespera/actions/runs/42347328245). Opt-in workflow observation writes acceptance and known terminal task states to the local journal as execution progresses. An acceptance includes the actual predecessor attempt and agent IDs used to assemble its prompt. Swarm shows green dependency links and inspects attempt and root outcomes independently. The post-run delegation batch remains available.

Windows validation passed 219 offline workspace tests, the SDK doctest, graphics tests, strict workspace and graphics Clippy, and the graphics build. CLI subprocess coverage verified acceptance before completion, predecessor identity, payload exclusion and cancellation without a second attempt. A desktop workflow with a held fixture response showed accepted work before completion. Releasing the response produced the green dependency link, both attempt IDs and the final completed states in the same window.

Live evidence is local task state and prompt provenance. It does not certify remote computation or prove a result correct. Interrupted publication can leave gaps, and old readers cannot decode the opt-in version 4 records. Swarm still lacks combined sources and contention evidence. The next Atlas milestone extends Living World decisions through the existing scheduler with world revision fencing.

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
