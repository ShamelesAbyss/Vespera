use std::path::PathBuf;
use std::time::{Duration, Instant};

use vespera_core::TaskId;
use vespera_world::{
    Position, ResidentSpec, Role, Site, World, WorldChange, WorldError, WorldFence, WorldSpec,
    WorldStore,
};

struct Database(PathBuf);

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn maximum_roster_replays_retained_commits_after_checkpoint_rollover() {
    const BATCH: u32 = 128;
    const TICKS: u64 = 5120;
    let spec = WorldSpec {
        seed: 42,
        width: 64,
        depth: 64,
        depot: Position { x: 4, z: 4 },
        residents: (0..32)
            .map(|index| ResidentSpec {
                id: format!("a0000000-0000-4000-8000-{:012x}", index + 1)
                    .parse()
                    .unwrap(),
                name: format!("Resident {index}"),
                role: if index % 4 == 0 {
                    Role::Builder
                } else {
                    Role::Forager
                },
                position: Position {
                    x: index % 8,
                    z: index / 8,
                },
            })
            .collect(),
        sites: (0..64)
            .map(|index| Site {
                position: Position {
                    x: index % 8,
                    z: index / 8,
                },
                stock: 10,
                capacity: 12,
                regeneration: 1,
            })
            .collect(),
    };
    let mut expected = World::new(spec.clone()).unwrap();
    let database = Database(
        std::env::temp_dir().join(format!("vespera-world-scale-{}.sqlite3", TaskId::new())),
    );
    WorldStore::initialize(&database.0, &World::new(spec).unwrap()).unwrap();

    let mut engine_time = Duration::ZERO;
    let mut commit_time = Duration::ZERO;
    let mut midpoint = None;
    for revision in (0..TICKS).step_by(BATCH as usize) {
        let start = Instant::now();
        for _ in 0..BATCH {
            expected.advance().unwrap();
        }
        engine_time += start.elapsed();
        let start = Instant::now();
        let actual = WorldStore::commit(
            &database.0,
            WorldFence::Revision(revision),
            WorldChange::Advance { ticks: BATCH },
        )
        .unwrap();
        commit_time += start.elapsed();
        assert_eq!(actual, expected);
        if revision + u64::from(BATCH) == TICKS / 2 {
            midpoint = Some(expected.clone());
        }
    }

    let start = Instant::now();
    let latest = WorldStore::replay(&database.0, None).unwrap();
    let latest_replay_time = start.elapsed();
    assert_eq!(latest.latest_revision, TICKS);
    assert_eq!(latest.earliest_revision, 1024);
    assert_eq!(latest.world, expected);
    assert_eq!(WorldStore::read(&database.0).unwrap(), expected);
    let replayed = WorldStore::replay(&database.0, Some(TICKS / 2)).unwrap();
    assert_eq!(replayed.world, midpoint.unwrap());
    assert!(matches!(
        WorldStore::replay(&database.0, Some(128)),
        Err(WorldError::ReplayUnavailable {
            revision: 128,
            earliest: 1024,
            latest: TICKS
        })
    ));

    let snapshot = expected.snapshot();
    assert_eq!(snapshot.residents.len(), 32);
    assert_eq!(snapshot.sites.len(), 64);
    assert!(snapshot.shelters > 0);
    assert!(snapshot.residents.iter().all(|resident| {
        !resident.relationships.is_empty() && !expected.memory(resident.id).unwrap().is_empty()
    }));
    let bytes = std::fs::metadata(&database.0).unwrap().len();
    eprintln!(
        "{TICKS} ticks, 32 residents, 64 sites: engine {engine_time:?}, SQLite commits {commit_time:?}, latest replay {latest_replay_time:?}, database {bytes} bytes"
    );
}
