//! Opt-in local observation of the CLI runtime that actually owns the work.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use vespera_app::runtime::VesperaRuntime;
use vespera_core::SchedulerSample;

use crate::AppError;

pub(crate) struct Publisher {
    finish: oneshot::Sender<()>,
    worker: JoinHandle<io::Result<()>>,
}

impl Publisher {
    pub(crate) async fn start(
        directory: Option<PathBuf>,
        runtime: &VesperaRuntime,
    ) -> Result<Option<Self>, AppError> {
        let Some(directory) = directory else {
            return Ok(None);
        };
        let directory = tokio::task::spawn_blocking(move || {
            fs::create_dir(&directory)?;
            Ok::<_, io::Error>(directory)
        })
        .await
        .map_err(|error| AppError::SchedulerTelemetry(io::Error::other(error)))?
        .map_err(AppError::SchedulerTelemetry)?;

        let scheduler = runtime.scheduler().clone();
        let registry = runtime.registry().clone();
        let (finish, mut finished) = oneshot::channel();
        let worker = tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(250));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let finished = tokio::select! {
                    biased;
                    signal = &mut finished => {
                        if signal.is_err() {
                            return Ok(());
                        }
                        true
                    }
                    _ = timer.tick() => false,
                };
                let provider_health = registry.provider_health_counts().await;
                let queue = scheduler.queue_snapshot();
                let (budget, reserved, model_reservations, uncertain_reservations) =
                    scheduler.reservation_snapshot();
                let observed_unix_ms = SystemTime::UNIX_EPOCH
                    .elapsed()
                    .map_err(io::Error::other)?
                    .as_millis()
                    .try_into()
                    .map_err(io::Error::other)?;
                let sample = SchedulerSample {
                    schema_version: SchedulerSample::SCHEMA_VERSION,
                    observed_unix_ms,
                    finished,
                    waiting: queue.waiting,
                    admitted: queue.active,
                    queue_closed: queue.closed,
                    budget,
                    reserved,
                    model_reservations,
                    uncertain_reservations,
                    provider_health,
                };
                let directory = directory.clone();
                tokio::task::spawn_blocking(move || publish(directory, sample))
                    .await
                    .map_err(io::Error::other)??;
                if finished {
                    return Ok(());
                }
            }
        });
        Ok(Some(Self { finish, worker }))
    }

    pub(crate) async fn finish(publisher: Option<Self>) -> Result<(), AppError> {
        if let Some(Self { finish, worker }) = publisher {
            let _ = finish.send(());
            worker
                .await
                .map_err(|error| AppError::SchedulerTelemetry(io::Error::other(error)))?
                .map_err(AppError::SchedulerTelemetry)?;
        }
        Ok(())
    }
}

fn publish(directory: PathBuf, sample: SchedulerSample) -> io::Result<()> {
    let pending = directory.join("scheduler.pending");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let result = serde_json::to_writer(&mut output, &sample)
        .map_err(io::Error::other)
        .and_then(|()| output.flush());
    drop(output);
    result?;
    fs::rename(pending, directory.join(SchedulerSample::FILE_NAME))
}
