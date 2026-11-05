# Individual memory

## Current: addressing and record types, no storage

Milestone 1 supplies `MemoryNamespace`, `MemoryLayer`, and `MemoryRecord`. The namespace derives solely from an agent UUID and exposes a logical relative address `agents/<agent-uuid>`; it does not create directories or access files. **There is no database, migration, retrieval engine, consolidation worker, or embedding service in this build.** Model/provider identifiers and display names never own a namespace.

Current agent-writable categories are `long_term`, `episodic`, `working`, `relationship`, `goal`, and `observation`. Identity is intentionally absent: identity/configuration remain control-plane owned. A record carries a memory UUID, owner agent UUID, layer, nonempty content, caller-supplied Unix-millisecond creation time, and optional source task UUID. This records supplied data, not verified provenance or time.

Model-policy replacement leaves the namespace unchanged. A typed address is not authorization; future storage must check principal and owner/scope on every operation. Two agents with the same name remain different owners. Untrusted model text or a caller-provided filesystem path must not select another namespace.

## Future structured storage — Milestone 8

SQLite is the first persistence target, behind Rust interfaces. Use structured records and transactional migrations rather than an unbounded prompt file. JSON/Markdown can be bounded content or inspectable exports, not the sole ownership/indexing model. Storage must not require an orchestrator or inference engine to be alive.

Extend record metadata with source episodes/attempts, update times, content format, provenance, and retention/access policy. Timestamp ordering is not sufficient provenance; preserve stable IDs and relationships. Models/providers can be recorded as sources without becoming owners. Schema, encryption decisions, and migration/backup procedures are future implementation work, not current guarantees.

The intended layers include identity, long-term knowledge, episodes, working context, relationships, goals, and learned observations. Identity and permissions stay in protected control-plane records, not the agent-writable layer enum. Agents may propose authorized goal/fact updates but may not silently overwrite identity, permissions, or global settings. Working memory can be bounded/ephemeral; durable layers must survive restart, suspension, and model/provider changes.

Separate original user observations, imports, tool results, and generated claims. Preserve uncertainty so retrieval does not turn repetition into truth. Relationship records may concern several agents, but knowing another ID never grants access to its namespace.

## Retrieval before embeddings

Start with structured filters and SQLite full-text search (FTS), including owner/category filters, bounded payloads/results, stable ordering, and source links. Apply authorization before exposing excerpts, existence, counts, or aggregates. Cross-agent sharing creates an explicitly authorized reference or copied record with provenance; it does not reassign ownership.

Optional embeddings come later, after useful structured/FTS retrieval. They must not require Python. Track embedding model/version, provenance, rebuild state, and deletion propagation. A vector index is derived, not authoritative; ordinary retrieval stays functional when it is absent. Retrieved content is untrusted prompt input and cannot change identity, network/tool access, or system instructions.

## Consolidation preserves evidence

Consolidation creates a derived summary/knowledge record linked to source episode IDs, method/model/version, task/attempt, and time. **Preserve original episodes and their provenance** rather than destructively replacing them with a model summary. Corrected summaries retain lineage and contradictory evidence stays inspectable.

Commit summary and source links atomically. Interrupted consolidation must not delete sources or falsely claim complete coverage. Idempotency identifiers prevent duplicate consolidation on retry. Normal namespace permissions and resource limits still apply; consolidation cannot rewrite identity.

Preserving sources during consolidation does not mean retaining every interaction forever. Explicit user-controlled retention, redaction, and deletion are separate audited operations. Address summaries, FTS, optional vector indexes, exports, backups, and provenance links; disclose when a retained summary's source is no longer available. Event replay is not a substitute for authoritative memory storage.

## Durability, privacy, and limits

Future tests cover transaction rollback, interruption, schema upgrade/recovery, locking, disk-full errors, backup/restore, and namespace isolation. A failed write must not produce a successful memory-update outcome. Storage/event coordination needs a transactional outbox or equivalent documented reconciliation to prevent committed data with missing events, or events for rolled-back data.

Bound growth, record sizes, per-agent retention, retrieval budgets, and consolidation work. Unknown capacity is not unlimited storage. Inspection/export require authorization. Secrets and private hidden chain-of-thought are not normal memory content; store user-visible outputs or explicit summaries only under policy and distinguish them from measured facts.

The user should eventually inspect sources, summaries, provenance, and retention decisions without inference. Those controls and restart persistence are Milestone 8 gates in [ROADMAP.md](ROADMAP.md). The basis is [INITIAL_PLAN.md](INITIAL_PLAN.md) and [MASTER_DIRECTIVE.txt](MASTER_DIRECTIVE.txt).
