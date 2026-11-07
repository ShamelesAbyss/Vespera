use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use vespera_config::AppConfig;
use vespera_core::{ModelDescriptor, ModelId};
use vespera_model_registry::{Catalog, CatalogError};

use crate::AppError;

#[derive(Debug, Args)]
pub(crate) struct CatalogArgs {
    #[arg(long)]
    database: PathBuf,
    #[arg(long, default_value = "config/local.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: CatalogCommand,
}

#[derive(Debug, Subcommand)]
enum CatalogCommand {
    /// Seed model metadata from a validated TOML configuration.
    Init,
    /// Print the committed revision and declared models as JSON.
    List,
    /// Register or update one model from a strict JSON descriptor.
    Put {
        #[arg(long)]
        model_file: PathBuf,
        #[arg(long)]
        revision: u64,
    },
    /// Remove an unreferenced model.
    Remove {
        #[arg(long)]
        model: ModelId,
        #[arg(long)]
        revision: u64,
    },
}

pub(crate) fn load(config: &Path, catalog: Option<&Path>) -> Result<AppConfig, AppError> {
    if let Some(path) = catalog {
        Ok(AppConfig::load_with_models(
            config,
            Catalog::read(path)?.models,
        )?)
    } else {
        Ok(AppConfig::load(config)?)
    }
}

pub(crate) fn run(args: CatalogArgs) -> Result<(), AppError> {
    let path = &args.database;
    let revision = match args.command {
        CatalogCommand::Init => {
            Catalog::initialize(path, &AppConfig::load(&args.config)?.models)?;
            0
        }
        CatalogCommand::List => {
            println!("{}", serde_json::to_string_pretty(&Catalog::read(path)?)?);
            return Ok(());
        }
        CatalogCommand::Put {
            model_file,
            revision,
        } => edit(path, &args.config, revision, |models| {
            let mut bytes = Vec::new();
            File::open(model_file)
                .and_then(|file| file.take(65537).read_to_end(&mut bytes))
                .map_err(|_| AppError::CatalogInput)?;
            if bytes.len() > 65536 {
                return Err(AppError::CatalogInput);
            }
            let descriptor: ModelDescriptor =
                serde_json::from_slice(&bytes).map_err(|_| AppError::CatalogInput)?;
            if let Some(existing) = models.iter_mut().find(|model| model.id == descriptor.id) {
                *existing = descriptor;
            } else {
                models.push(descriptor);
            }
            Ok(())
        })?,
        CatalogCommand::Remove { model, revision } => {
            edit(path, &args.config, revision, |models| {
                let index = models
                    .iter()
                    .position(|entry| entry.id == model)
                    .ok_or(AppError::CatalogModelAbsent)?;
                models.remove(index);
                Ok(())
            })?
        }
    };
    println!("{}", serde_json::json!({ "revision": revision }));
    Ok(())
}

fn edit(
    database: &Path,
    config: &Path,
    expected: u64,
    change: impl FnOnce(&mut Vec<ModelDescriptor>) -> Result<(), AppError>,
) -> Result<u64, AppError> {
    let mut snapshot = Catalog::read(database)?;
    if snapshot.revision != expected {
        return Err(CatalogError::StaleRevision {
            expected,
            actual: snapshot.revision,
        }
        .into());
    }
    change(&mut snapshot.models)?;
    let config = AppConfig::load_with_models(config, snapshot.models)?;
    Ok(Catalog::replace(database, expected, &config.models)?)
}
