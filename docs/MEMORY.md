# Individual memory

## Implemented durable storage and agent ownership

`vespera-memory` provides a blocking SQLite library independent of inference and orchestration. It stores isolated agent-owned records, performs structured and FTS5 literal-phrase retrieval, and commits consolidation lineage with derived records. Data survives close/reopen and agent model-policy changes. The agent runtime supplies a current-grant/accepted-task authorization adapter through `memory_authorizer()`.

The older `AgentRuntime::write_memory` / `read_memory` methods remain separate bounded in-process buffers. They are not silently persisted or migrated. The explicit offline [memory CLI](CLI.md#offline-durable-memory) can initialize, add, inspect, search, consolidate and delete authorized records. Single-agent inference can explicitly retrieve matching owned records and retain an accepted response with its task ID. The [inference memory options](CLI.md#opt-in-memory-during-inference) document grants, bounds and failure reporting.

An agent UUID owns a namespace; model/provider IDs and display names never do. Every store operation checks the trusted actor, requested owner and current `MemoryRead`/`MemoryWrite` grant before looking up content, excerpts or namespace statistics. Cross-owner access is denied even when the actor has memory grants for its own namespace. The caller must authenticate actor identity outside this library; UUIDs are not credentials. No remote authorization boundary or cross-agent sharing API is implied.

## Store API and records

Open a file explicitly with `MemoryStore::open(path, StorageLimits, authorizer)`. Parent directories must exist for the library opener; CLI `memory init` creates them. `open_existing` and `open_read_only` require a valid existing database and never initialize empty files. A trusted `MemoryAuthorizer` supplies current permission and accepted-task ownership checks; `AgentRuntime::memory_authorizer()` implements that port. Calls block on SQLite/filesystem I/O and should run on a blocking worker in async applications. Errors expose bounded categories, not raw SQL, private paths or content.

| API | Behavior and permission |
| --- | --- |
| `insert(actor, record, origin)` | Append immutable record; same owner and `MemoryWrite` required |
| `get(actor, owner, id)` | Authorized record plus lineage, or none; `MemoryRead` |
| `list(actor, owner, query)` | Structured, insertion-ordered bounded page; `MemoryRead` |
| `search(actor, owner, phrase, query)` | FTS5 literal phrase plus the same filters/bounds; `MemoryRead` |
| `namespace_usage(actor, owner)` | Only that namespace's authoritative record/content counts; `MemoryRead` |
| `consolidate(actor, summary, sources, method)` | Atomic derived record and lineage; both read/write |
| `delete(actor, owner, id)` | Atomic authoritative/index deletion; both read/write; referenced originals protected |
| `sqlite_version()` | Report the linked database library version |

A `MemoryRecord` contains its memory UUID, owner UUID, agent-writable layer, nonempty content, caller-supplied Unix-millisecond timestamp and optional source task. Layers are `long_term`, `episodic`, `working`, `relationship`, `goal` and `observation`. Identity and permission records are deliberately absent. Timestamps must fit SQLite's nonnegative signed integer range and are not independently verified clocks.

Origins distinguish `observation`, `import`, `generated` and `derived`. They describe provenance. `generated` requires a source task accepted and assigned to the owner according to the authorizer at write time.

`memory remember-task` authorizes an offline write from an owned, completed result in the private agent task store. It copies the recorded response verbatim into episodic memory. The caller supplies a new memory UUID and needs the current `memory_write` grant. The memory row retains the task UUID. Keep the task database available to inspect the original execution. The two databases are written separately. See [offline durable memory](CLI.md#offline-durable-memory).

Stored results include a monotonically increasing **namespace-local** sequence. Other agents' inserts do not create observable cursor gaps. Namespace counters survive deletion, so old cursors do not silently restart. Memory UUIDs must be unique among live records; explicit deletion can free an unreferenced UUID, while sequence numbers are never reset/reused. Stable identity/lineage should therefore use newly generated memory UUIDs for new observations.

## Bounds, retrieval and consolidation

| Default limit | Value |
| --- | --- |
| Ever-created namespace counters per database | 64; configurable 1–1,024 |
| Record content | 64 KiB maximum |
| Records per namespace / total | 1,024 / 8,192 |
| Content bytes per namespace / total | 8 MiB / 64 MiB |
| Records per query | 1–100 |
| Query content budget | Default 1 MiB; between configured maximum record size and 4 MiB |
| Search phrase | 1–256 UTF-8 bytes, at most 32 whitespace-separated terms |
| Consolidation sources / method | 1–32 distinct IDs / 1–128 bytes |
| Database busy timeout | One second |

Write transactions derive current counts from the database and reject limits before mutation, including across independently opened connections. No automatic pruning occurs. These are logical content/count quotas, not measured RAM or physical file/index limits. Indexes, metadata, journals, free pages and caller clones require additional capacity. Reopening with tighter limits does not delete existing data; existing oversized records can require restoring a suitable read limit. Namespace-counter capacity includes deleted namespaces to preserve cursor history.

`MemoryQuery` filters layer, cursor, inclusive `created_since_unix_ms`, exclusive `created_before_unix_ms`, result count and content-byte budget. The returned page reports accepted content bytes and an optional continuation sequence. Phrase text is bound as a quoted literal; FTS operators are not a user query language. Results expose insertion order and complete authorized records, not cross-namespace relevance scores. Retrieved content remains untrusted prompt data.

Consolidation preserves all source records and records the caller-supplied method plus sorted source UUIDs. Sources must exist in the same namespace. Record, FTS entry, quota/counter effects and lineage commit together or roll back. Repeating an existing summary ID conflicts without rewriting evidence. A source referenced by a derived record cannot be deleted until dependent derived records are explicitly removed. There is no automatic summarization model, retention worker, embedding service or vector index.

## Schema, recovery and privacy

The database identifies itself with application ID `0x5653504d` and `PRAGMA user_version = 1`. Initial schema/FTS creation and identity changes occur in one transaction. Initialization rechecks identity under a writer lock so concurrent openers cannot label unrelated data. Unrelated nonempty databases and unsupported future versions are rejected; missing expected tables/columns fail rather than being silently recreated. No migration of an older deployed storage schema has been needed yet.

Foreign keys are enabled, synchronous mode is `FULL`, and trusted schema execution is disabled. Extension loading is not enabled. FTS maintenance is explicit inside the same transactions as records. Reads of a record and its lineage use a consistent read transaction. This is a trusted local database, not a sandbox for attacker-edited SQLite files. No application-level encryption or automatic filesystem ACL provisioning is implemented.

Tests inject migration failure after schema creation and lineage failure after record/index insertion, verify transaction rollback, and abruptly exit a child writer without running transaction destructors. Reopening recovers previously committed content and removes uncommitted record/FTS writes. This verifies process-exit recovery on the tested filesystem; it does not establish resilience to every power-loss, disk-full, filesystem or hardware failure. Lock contention returns a bounded busy error; synchronous OS I/O is not subject to a hard operation deadline.

Logical deletion does not guarantee removal from free pages, journals, OS snapshots, exports or earlier backups. Durable event coordination and workflow/task history use separate storage. Keep credentials and private hidden reasoning out of ordinary memory.

## Backup and restore

`memory-admin backup` and `memory-admin restore` use the same SQLite
snapshot operation. It reads a consistent view that includes committed WAL
transactions. It stages the copy beside the destination and verifies
SQLite structure, foreign keys, owner counters, lineage and the FTS index
against the content table. It syncs the staged file before publishing it
with a hard link. [SQLite snapshot semantics](https://sqlite.org/lang_vacuum.html)

Use existing private directories and a filesystem with hard-link support.
The operation rejects an existing destination or SQLite sidecar. To adopt a
restore, point the application at the newly published database. A process
interruption can leave a private staging directory with the prefix
`.vespera-memory-copy-`. The application uses the destination only after
publication. A snapshot contains all memory namespaces but covers only this
database, so preserve the separate task ledger when keeping source results.
See [CLI examples](CLI.md#memory-backup-and-restore).

## Verification

```powershell
cargo test -p vespera-memory --locked
cargo test -p vespera-agents --test agents --locked
cargo test -p vespera-memory -p vespera-agents --release --locked
```

Storage tests exercise namespace/reopen isolation, literal/structured queries, byte/count pagination, source validation, current grants, consolidation/deletion, rollback, concurrent initialization/admission, foreign/future file preservation, lock failure and abrupt-writer recovery. Snapshot tests restore two namespaces with FTS, lineage and owner sequence continuity, and reject a corrupt search index. The agent adapter test rebinds a model and reopens the same two namespaces under a new runtime.

See [agents](AGENTS.md), [architecture](ARCHITECTURE.md), [dependency decisions](DEPENDENCIES.md), [roadmap](ROADMAP.md) and [development log](DEVELOPMENT_LOG.md).
