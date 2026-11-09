//! Shared deterministic paired-world runner for the CLI and desktop.
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::json;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use vespera_core::{AgentId, EventId};
use vespera_world::{Role, World, WorldChange, WorldError, WorldFence, WorldSpec, WorldStore};

/// Bounded inputs and fresh output directory for a paired world experiment.
pub struct Settings {
    pub evidence_dir: PathBuf,
    pub ticks: u32,
    pub seeds: u32,
    pub suspend_role: Role,
    pub suspend_count: u32,
    pub target_shelters: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    World(#[from] WorldError),
    #[error("cannot create a fresh experiment evidence directory: {0}")]
    Directory(std::io::Error),
}

#[derive(Serialize)]
struct Trial {
    tick: u64,
    active_residents: usize,
    shelters: u16,
    shared_stock: u16,
    site_stock: u32,
    target_met: bool,
    execution_elapsed_ns: u128,
    evidence: Evidence,
}

#[derive(Serialize)]
struct Evidence {
    database: String,
    database_bytes: u64,
    revision: u64,
    advance_event_id: EventId,
}

#[derive(Serialize)]
struct Pair {
    seed: u64,
    first_arm: &'static str,
    baseline: Trial,
    intervention: Trial,
    shelter_delta: i32,
}

/// Run paired worlds and return the versioned report.
pub fn run(mut spec: WorldSpec, args: &Settings) -> Result<serde_json::Value, Error> {
    if !(1..=1000).contains(&args.ticks)
        || !(1..=32).contains(&args.seeds)
        || !(1..=31).contains(&args.suspend_count)
        || !(1..=256).contains(&args.target_shelters)
    {
        return Err(
            WorldError::Invalid("world experiment settings exceed supported bounds").into(),
        );
    }
    World::new(spec.clone())?;
    let role = args.suspend_role;
    let affected = spec
        .residents
        .iter()
        .filter(|resident| resident.role == role)
        .count();
    let suspend_count = args.suspend_count as usize;
    if suspend_count > affected || suspend_count >= spec.residents.len() {
        return Err(WorldError::Invalid(
            "intervention must match the requested role count and leave at least one resident active",
        )
        .into());
    }
    if args.seeds as usize > spec.residents.len()
        || spec
            .seed
            .checked_add(u64::from(args.seeds) + u64::from(args.ticks) - 2)
            .is_none()
    {
        return Err(WorldError::Invalid(
            "seed rotations must fit the roster and cannot wrap during the run",
        )
        .into());
    }
    for (index, resident) in spec.residents.iter_mut().enumerate() {
        resident.id = format!("10000000-0000-4000-8000-{:012x}", index + 1)
            .parse()
            .expect("canonical fixture identity");
        resident.name = format!("Resident {}", index + 1);
    }
    let suspended: Vec<_> = spec
        .residents
        .iter()
        .filter(|resident| resident.role == role)
        .take(suspend_count)
        .map(|resident| resident.id)
        .collect();
    std::fs::create_dir(&args.evidence_dir).map_err(Error::Directory)?;
    let pairs = (0..args.seeds)
        .map(|index| {
            let mut world = spec.clone();
            world.seed += u64::from(index);
            let run = |arm, suspended: &[AgentId]| {
                trial(
                    &world,
                    suspended,
                    args.ticks,
                    args.target_shelters,
                    &args.evidence_dir,
                    format!("{arm}-{index:02}.sqlite3"),
                )
            };
            let (baseline, intervention, first_arm) = if index % 2 == 0 {
                (
                    run("baseline", &[])?,
                    run("intervention", suspended.as_slice())?,
                    "baseline",
                )
            } else {
                let intervention = run("intervention", suspended.as_slice())?;
                (run("baseline", &[])?, intervention, "intervention")
            };
            Ok(Pair {
                seed: world.seed,
                first_arm,
                shelter_delta: i32::from(intervention.shelters) - i32::from(baseline.shelters),
                baseline,
                intervention,
            })
        })
        .collect::<Result<Vec<_>, WorldError>>()?;
    let baseline_met = pairs.iter().filter(|pair| pair.baseline.target_met).count();
    let intervention_met = pairs
        .iter()
        .filter(|pair| pair.intervention.target_met)
        .count();
    let delta_range = pairs.iter().fold(None, |range: Option<(i32, i32)>, pair| {
        let delta = pair.shelter_delta;
        Some(range.map_or((delta, delta), |(min, max)| {
            (min.min(delta), max.max(delta))
        }))
    });
    let report = json!({
        "schema_version": 1,
        "experiment": "world_roster",
        "vespera_version": env!("CARGO_PKG_VERSION"),
        "configuration": {
            "world": spec,
            "ticks": args.ticks,
            "seed_rotations": args.seeds,
            "baseline": { "policy": "all_active" },
            "intervention": { "policy": "suspend_role", "role": role, "count": suspend_count, "residents": suspended },
            "target_shelters": args.target_shelters
        },
        "summary": {
            "pairs": pairs.len(),
            "baseline_target_met": baseline_met,
            "baseline_target_missed": pairs.len() - baseline_met,
            "intervention_target_met": intervention_met,
            "intervention_target_missed": pairs.len() - intervention_met,
            "observed_shelter_delta_range": delta_range
        },
        "measurements": {
            "process_resident_bytes_after_trials": process_resident_bytes(),
            "inference_latency_ns": null,
            "gpu_memory_bytes": null,
        },
        "inference": { "used": false, "model_version": null, "provider_version": null },
        "pairs": pairs,
    });

    Ok(report)
}

fn trial(
    spec: &WorldSpec,
    suspended: &[AgentId],
    ticks: u32,
    target: u16,
    directory: &Path,
    database: String,
) -> Result<Trial, WorldError> {
    let path = directory.join(&database);
    WorldStore::initialize(&path, &World::new(spec.clone())?)?;
    let start = Instant::now();
    let mut revision = 0;
    for &resident in suspended {
        revision = WorldStore::commit(
            &path,
            WorldFence::Revision(revision),
            WorldChange::SetActive {
                resident,
                active: false,
            },
        )?
        .revision();
    }
    let (world, event) = WorldStore::commit_observed(
        &path,
        WorldFence::Revision(revision),
        WorldChange::Advance { ticks },
    )?;
    let execution_elapsed_ns = start.elapsed().as_nanos();
    let snapshot = world.snapshot();
    Ok(Trial {
        tick: snapshot.tick,
        active_residents: snapshot
            .residents
            .iter()
            .filter(|resident| resident.active)
            .count(),
        shelters: snapshot.shelters,
        shared_stock: snapshot.shared_stock,
        site_stock: snapshot
            .sites
            .iter()
            .map(|site| u32::from(site.stock))
            .sum(),
        target_met: snapshot.shelters >= target,
        execution_elapsed_ns,
        evidence: Evidence {
            database,
            database_bytes: path.metadata().map_err(|_| WorldError::Storage)?.len(),
            revision: snapshot.revision,
            advance_event_id: event.ok_or(WorldError::Format)?.event_id,
        },
    })
}

fn process_resident_bytes() -> Option<u64> {
    let pid = sysinfo::get_current_pid().ok()?;
    let mut host = System::new();
    host.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_memory(),
    );
    host.process(pid)
        .map(|process| process.memory())
        .filter(|&bytes| bytes > 0)
}
