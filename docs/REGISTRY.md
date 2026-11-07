# Model registry and provider health

## Durable model catalog

The optional SQLite catalog stores declared model metadata across CLI invocations. Providers, credential references, network policy and agents continue to come from the supplied TOML. Health evidence and residency are observed at runtime.

```powershell
$db = 'data/models.sqlite3'
$config = 'config/local.toml'
cargo run --locked -p vespera-app -- catalog --database $db --config $config init
cargo run --locked -p vespera-app -- catalog --database $db list
cargo run --locked -p vespera-app -- models --config $config --model-catalog $db --json
```

Create the parent directory first. `init` seeds revision 0 from the TOML and rejects an existing initialized catalog. `list` returns the current revision and complete ordered descriptors. Save a descriptor from the `models` array as a JSON object to register or edit one model. Use `put --model-file <PATH> --revision <N>` or `remove --model <UUID> --revision <N>`, with the revision last read.

Edits preserve UUID bindings, including after removal. Changes that break configured agent policies or provider references fail before commit. A stale revision returns an error and preserves the winning writer's changes. SQLite stores model descriptors and retired UUID bindings. Provider endpoints, credentials, health results and reservations stay outside the catalog.

Pass `--model-catalog <PATH>` with `models`, `agents`, `validate-config`, `provider-health`, `run-agent`, `run-workflow`, `memory`, `world decide` or `world run` to use the catalog in place of the TOML model list. A missing or invalid catalog fails explicitly. A command reads one committed snapshot at startup. Running commands keep their loaded inventory.

## Runtime registry

The registry is an in-process, thread-safe inventory. Registering providers or
models performs no model loading, inference, or health I/O. Models and providers
have independent UUID identities; model inventory size has no hardcoded cap.

## Identity and controlled changes

- `register_provider` rejects duplicate provider IDs.
- `register_model` rejects duplicate model IDs and duplicate provider/model-name
  bindings. Aliases for the same binding are not supported.
- `update_model` changes metadata and operator enablement while preserving the
  provider ID and exact provider model identifier. A new binding needs a new
  model ID and explicit retirement of the old entry.
- `remove_model` removes routing metadata, not provider memory.
- `remove_provider` rejects removal while any registered model references it.

All mutations share one registry lock, so identity and referential checks are
atomic. Empty names/identifiers and known zero context windows are rejected.
Unknown resource estimates remain unknown.

## Effective availability

`get_registration` returns original operator metadata. Its `available` flag means
operator enablement. `get_model`, `list_models`, and `find_by_capability` return
routing snapshots whose availability additionally requires a fresh healthy
provider observation. Use original metadata when preparing an operator update;
copying an effective availability flag would confuse health with enablement.

`provider_health` distinguishes unknown, healthy, degraded, unreachable, and
stale states, with an optional monotonic completion timestamp. Unknown, stale,
degraded, and unreachable providers are excluded from routing. Recovery restores
only operator-enabled entries; it never re-enables an operator-disabled model.

Defaults are 30 seconds of health freshness and a five-second per-provider poll
deadline. `with_health_policy` accepts positive custom durations.
`health_check_all` explicitly polls a snapshot sequentially;
`health_check_providers` restricts polling to specified provider IDs without
fencing unrelated in-flight polls. Neither is a background monitor. Application
commands explicitly schedule refreshes. Each poll is cancellable
and bounded. Cancellation preserves previous evidence until it expires; a
failed/timed-out poll or mismatched response identity records unreachable.

No registry lock spans provider I/O. Newer poll generations and replacement
provider instances fence out delayed results. Timestamps are in-process Tokio
monotonic instants, not restart-persistent audit timestamps.

## Scheduler relationship

Scheduler reservations retain the provider instance used for loading. Inventory
removal/re-registration therefore cannot redirect a later unload. Reusing a
reservation whose provider instance, binding, or resource estimates changed is
rejected until explicit eviction. A new context request must fit both the model's
known context ceiling and the operator's budget.

Registration and health do not prove residency, model-specific readiness, hardware
fit, or inference quality. External provider/model changes and restart
reconciliation remain separate work.

## Verification

```powershell
cargo test -p vespera-model-registry -p vespera-scheduler --locked
```

Deterministic fixtures cover duplicate races, aliases, update/removal invariants,
health expiry/recovery, operator disablement, cancellation/timeouts, stalled and
outdated polls, provider replacement, and unloading after inventory removal.
Tokio's test-only virtual clock makes expiry and timeout tests deterministic.
