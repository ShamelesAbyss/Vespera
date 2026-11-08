mod atlas;
mod gpu_telemetry;
mod home_scene;
mod machine_scene;
mod scene;
mod scene_measurement;
mod swarm_scene;
mod task_timeline;
mod world_scene;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use bevy::prelude::Color;
use clap::{CommandFactory, Parser, error::ErrorKind};
use tokio::sync::mpsc;
use vespera_sdk::{
    BrokerEventEntry, BrokerStreamEnd, BrokerStreamFrame, ClientError, OperatorClient, SigningKey,
    TaskId, TaskState,
};

#[derive(Parser)]
#[command(about = "Vespera native 3D orchestration and simulation")]
struct Options {
    /// Use this directory for the Home screen's local world.
    #[arg(long, conflicts_with_all = ["atlas", "machine", "world", "swarm", "swarm_broker", "broker", "allow_cancel"])]
    desktop_data: Option<PathBuf>,
    /// Signed broker address. Standalone broker views default to 127.0.0.1:8765.
    #[arg(long)]
    broker: Option<SocketAddr>,
    #[arg(long)]
    allow_cancel: bool,
    /// Disable animated camera focus and task lane transitions.
    #[arg(long)]
    reduced_motion: bool,
    /// Switch between configured layers in a single desktop window.
    #[arg(long, conflicts_with_all = ["machine", "swarm_broker"])]
    atlas: bool,
    /// Print a bounded frame-interval sample for each focused Atlas layer.
    #[arg(long, requires = "atlas", value_parser = clap::value_parser!(u32).range(60..=3600))]
    measure_frames: Option<u32>,
    /// Open the local hardware view without a broker or credentials.
    #[arg(long, conflicts_with_all = ["broker", "allow_cancel", "world"])]
    machine: bool,
    /// Read scheduler telemetry in Machine or Atlas mode.
    #[arg(long)]
    scheduler_telemetry: Option<PathBuf>,
    /// Open a persistent simulation world, standalone or as an Atlas layer.
    #[arg(long)]
    world: Option<PathBuf>,
    /// Inspect a retained world revision in a read-only 3D window.
    #[arg(long, requires = "world", conflicts_with = "allow_world_control")]
    world_revision: Option<u64>,
    /// Inspect observed agent relationships from an existing local event journal.
    #[arg(long, conflicts_with = "machine")]
    swarm: Option<PathBuf>,
    /// Inspect accepted mailbox messages on the signed operator feed.
    #[arg(long, conflicts_with_all = ["swarm", "allow_cancel", "world", "machine"])]
    swarm_broker: bool,
    /// Allow the World Mode window to commit simulation controls.
    #[arg(long, requires = "world")]
    allow_world_control: bool,
}

#[derive(Clone)]
enum Update {
    Ready(u64),
    Status(&'static str),
    Reset,
    Observation(BrokerEventEntry),
    Cancelled(TaskId, TaskState),
    CancelFailed,
}

fn main() -> ExitCode {
    let mut options = Options::parse();
    if let Some(error) = options.conflict() {
        Options::command()
            .error(ErrorKind::ArgumentConflict, error)
            .exit();
    }
    if options.opens_home() {
        home_scene::run(options.desktop_data, options.reduced_motion);
        return ExitCode::SUCCESS;
    }
    if options.atlas {
        return match run_atlas(&mut options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(path) = options.swarm {
        return match swarm_scene::run(path, options.reduced_motion) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if options.machine {
        machine_scene::run(options.scheduler_telemetry, options.reduced_motion);
        return ExitCode::SUCCESS;
    }
    if let Some(path) = options.world {
        return match world_scene::run(
            path,
            options.world_revision,
            options.allow_world_control,
            options.reduced_motion,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    let address = options
        .broker
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8765)));
    let (receiver, control) = match start_broker(address, None) {
        Ok(channels) => channels,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if options.swarm_broker {
        drop(control);
        swarm_scene::run_broker(receiver, options.reduced_motion);
    } else {
        scene::run(
            receiver,
            options.allow_cancel.then_some(control),
            options.reduced_motion,
        );
    }
    ExitCode::SUCCESS
}

impl Options {
    fn opens_home(&self) -> bool {
        !(self.atlas
            || self.machine
            || self.world.is_some()
            || self.swarm.is_some()
            || self.swarm_broker
            || self.broker.is_some()
            || self.allow_cancel)
    }

    fn conflict(&self) -> Option<&'static str> {
        if self.scheduler_telemetry.is_some() && !(self.atlas || self.machine) {
            return Some("--scheduler-telemetry requires --machine or --atlas");
        }
        if self.atlas {
            return (self.allow_cancel && self.broker.is_none())
                .then_some("--atlas --allow-cancel requires --broker");
        }
        if (self.world.is_some() || self.swarm.is_some())
            && (self.broker.is_some() || self.allow_cancel)
        {
            return Some("combining a broker with --world or --swarm requires --atlas");
        }
        if self.world.is_some() && self.swarm.is_some() {
            return Some("combining --world and --swarm requires --atlas");
        }
        None
    }
}

fn run_atlas(options: &mut Options) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = scene::desktop(
        "Vespera | Atlas",
        Color::srgb(0.017, 0.023, 0.043),
        atlas::Layer::Machine,
    );
    let reduced = options.reduced_motion;
    machine_scene::install(&mut app, options.scheduler_telemetry.take(), reduced);
    if let Some(path) = options.world.take() {
        world_scene::install(
            &mut app,
            path,
            options.world_revision,
            options.allow_world_control,
            reduced,
        )?;
    }
    let use_broker_swarm = options.swarm.is_none();
    if let Some(path) = options.swarm.take() {
        swarm_scene::install_journal(&mut app, path, reduced)?;
    }
    if let Some(address) = options.broker {
        let (mirror, swarm_updates) = if use_broker_swarm {
            let (send, recv) = mpsc::channel(64);
            (Some(send), Some(recv))
        } else {
            (None, None)
        };
        let (updates, control) = start_broker(address, mirror)?;
        scene::install(
            &mut app,
            updates,
            options.allow_cancel.then_some(control),
            reduced,
        );
        if let Some(updates) = swarm_updates {
            swarm_scene::install_broker(&mut app, updates, reduced);
        }
    }
    atlas::navigation(&mut app);
    if let Some(frames) = options.measure_frames {
        scene_measurement::install(&mut app, frames as usize);
    }
    app.run();
    Ok(())
}

struct Feeds {
    primary: mpsc::Sender<Update>,
    mirror: Option<mpsc::Sender<Update>>,
}

impl Feeds {
    async fn send(&self, update: Update) -> Result<(), ()> {
        if let Some(mirror) = &self.mirror {
            let _ = mirror.send(update.clone()).await;
        }
        self.primary.send(update).await.map_err(|_| ())
    }
}

fn start_broker(
    address: SocketAddr,
    mirror: Option<mpsc::Sender<Update>>,
) -> Result<(mpsc::Receiver<Update>, mpsc::Sender<TaskId>), Box<dyn std::error::Error>> {
    let key = std::env::var("VESPERA_OPERATOR_TOKEN")
        .map_err(|_| "Set VESPERA_OPERATOR_TOKEN to the broker operator signing key.")?;
    let client = OperatorClient::new(address, SigningKey::from_hex(&key)?)?;
    let (primary, observations) = mpsc::channel(64);
    let (control, actions) = mpsc::channel(4);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    std::thread::spawn(move || {
        runtime.block_on(observe(client, Feeds { primary, mirror }, actions))
    });
    Ok((observations, control))
}

async fn observe(client: OperatorClient, updates: Feeds, mut actions: mpsc::Receiver<TaskId>) {
    let mut cursor = None;
    while !updates.primary.is_closed() {
        match client.subscribe(cursor).await {
            Ok(mut stream) => loop {
                tokio::select! {
                    () = updates.primary.closed() => return,
                    action = actions.recv(), if !actions.is_closed() => {
                        if let Some(task) = action {
                            let update = match client.cancel(task).await {
                                Ok(state) => Update::Cancelled(task, state),
                                Err(_) => Update::CancelFailed,
                            };
                            if updates.send(update).await.is_err() { return; }
                        }
                    }
                    frame = stream.next() => match frame {
                        Ok(Some(BrokerStreamFrame::Ready { discarded_through, .. })) => {
                            if updates.send(Update::Ready(discarded_through)).await.is_err() { return; }
                        }
                        Ok(Some(BrokerStreamFrame::Event { entry })) => {
                            cursor = Some(entry.cursor);
                            if updates.send(Update::Observation(entry)).await.is_err() { return; }
                        }
                        Ok(Some(BrokerStreamFrame::End { reason: BrokerStreamEnd::CursorExpired })) => {
                            cursor = None;
                            if updates.send(Update::Reset).await.is_err() { return; }
                            break;
                        }
                        _ => break,
                    }
                }
            },
            Err(ClientError::Rejected {
                status: 409 | 410, ..
            }) => {
                cursor = None;
                if updates.send(Update::Reset).await.is_err() {
                    return;
                }
            }
            Err(_) => {}
        }
        if updates
            .send(Update::Status("Disconnected - reconnecting"))
            .await
            .is_err()
        {
            return;
        }
        while actions.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_the_default_and_explicit_modes_still_select_their_views() {
        assert!(
            Options::try_parse_from(["vespera-view"])
                .unwrap()
                .opens_home()
        );
        for args in [
            vec!["--atlas"],
            vec!["--machine"],
            vec!["--world", "world.sqlite3"],
            vec!["--broker", "127.0.0.1:8765"],
            vec!["--swarm", "events.sqlite3"],
            vec!["--swarm-broker"],
        ] {
            let options =
                Options::try_parse_from(std::iter::once("vespera-view").chain(args)).unwrap();
            assert!(!options.opens_home());
            assert!(options.conflict().is_none());
        }
    }
}
