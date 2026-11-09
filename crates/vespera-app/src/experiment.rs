use std::path::PathBuf;

use clap::ValueEnum;
use vespera_app::world_experiment::{self, Settings};
use vespera_world::Role;

use crate::{AppError, world_commands};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ResidentRole {
    Forager,
    Builder,
}

impl From<ResidentRole> for Role {
    fn from(value: ResidentRole) -> Self {
        match value {
            ResidentRole::Forager => Self::Forager,
            ResidentRole::Builder => Self::Builder,
        }
    }
}

#[derive(Debug, clap::Args)]
pub(super) struct Args {
    /// Strict JSON world specification. Resident names and IDs are replaced in the experiment.
    #[arg(long)]
    spec: PathBuf,
    /// A new directory for the paired SQLite world databases.
    #[arg(long)]
    evidence_dir: PathBuf,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000))]
    ticks: u32,
    /// Number of consecutive seed rotations starting at the seed in the specification.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=32))]
    seeds: u32,
    /// Suspend residents of this role in the intervention arm from tick zero.
    #[arg(long, value_enum)]
    suspend_role: ResidentRole,
    /// Number to suspend in specification order.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=31))]
    suspend_count: u32,
    /// A trial meets its target when it ends with at least this many shelters.
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..=256))]
    target_shelters: u16,
}

pub(super) fn run(args: Args) -> Result<(), AppError> {
    let spec = world_commands::load_spec(&args.spec)?;
    let report = world_experiment::run(
        spec,
        &Settings {
            evidence_dir: args.evidence_dir,
            ticks: args.ticks,
            seeds: args.seeds,
            suspend_role: args.suspend_role.into(),
            suspend_count: args.suspend_count,
            target_shelters: args.target_shelters,
        },
    )
    .map_err(|error| match error {
        world_experiment::Error::World(error) => AppError::World(error),
        world_experiment::Error::Directory(error) => AppError::ExperimentDirectory(error),
    })?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
