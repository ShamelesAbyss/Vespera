use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{ArgAction, Args, Subcommand};
use vespera_core::AgentId;
use vespera_world::{Position, World, WorldError, WorldStore};

use crate::AppError;

#[derive(Debug, Args)]
pub(super) struct WorldArgs {
    /// SQLite world database.
    #[arg(long)]
    database: PathBuf,
    #[command(subcommand)]
    command: WorldCommand,
}

#[derive(Debug, Subcommand)]
enum WorldCommand {
    /// Initialize a world from strict JSON.
    Init {
        #[arg(long)]
        spec: PathBuf,
    },
    /// Read the current public state as JSON.
    Inspect,
    /// Advance deterministic ticks and commit them together.
    Advance {
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=1000))]
        ticks: u32,
    },
    /// Activate or suspend one resident at an observed world revision.
    SetActive {
        #[arg(long)]
        resident: AgentId,
        #[arg(long, action = ArgAction::Set)]
        active: bool,
        #[arg(long)]
        revision: u64,
    },
    /// Move an active resident to an adjacent cell at an observed revision.
    Move {
        #[arg(long)]
        resident: AgentId,
        #[arg(long)]
        x: u16,
        #[arg(long)]
        z: u16,
        #[arg(long)]
        revision: u64,
    },
    /// Read one resident's memories as the local database operator.
    Memory {
        #[arg(long)]
        resident: AgentId,
    },
}

pub(super) fn run(args: WorldArgs) -> Result<(), AppError> {
    let database = &args.database;
    let world = match args.command {
        WorldCommand::Init { spec } => {
            let world = load_spec(&spec)?;
            WorldStore::initialize(database, &world)?;
            world
        }
        WorldCommand::Inspect => WorldStore::read(database)?,
        WorldCommand::Advance { ticks } => WorldStore::update(database, |world| {
            for _ in 0..ticks {
                world.advance()?;
            }
            Ok(())
        })?,
        WorldCommand::SetActive {
            resident,
            active,
            revision,
        } => WorldStore::update(database, |world| {
            world.set_active(resident, active, revision)
        })?,
        WorldCommand::Move {
            resident,
            x,
            z,
            revision,
        } => WorldStore::update(database, |world| {
            world.move_resident(resident, Position { x, z }, revision)
        })?,
        WorldCommand::Memory { resident } => {
            let world = WorldStore::read(database)?;
            println!("{}", serde_json::to_string_pretty(world.memory(resident)?)?);
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&world.snapshot())?);
    Ok(())
}

fn load_spec(path: &Path) -> Result<World, WorldError> {
    const LIMIT: u64 = 64 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|_| WorldError::Invalid("could not read world specification"))?;
    if bytes.len() as u64 > LIMIT {
        return Err(WorldError::Invalid("world specification exceeds 64 KiB"));
    }
    let spec = serde_json::from_slice(&bytes)
        .map_err(|_| WorldError::Invalid("world specification JSON"))?;
    World::new(spec)
}
