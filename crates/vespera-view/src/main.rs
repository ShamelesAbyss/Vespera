mod machine_scene;
mod scene;
mod swarm_scene;
mod world_scene;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;
use vespera_sdk::{
    BrokerEventEntry, BrokerStreamEnd, BrokerStreamFrame, ClientError, OperatorClient, SigningKey,
    TaskId, TaskState,
};

#[derive(Parser)]
#[command(about = "Vespera native 3D orchestration and simulation")]
struct Options {
    #[arg(long, default_value = "127.0.0.1:8765")]
    broker: SocketAddr,
    #[arg(long)]
    allow_cancel: bool,
    /// Disable animated camera focus and task lane transitions.
    #[arg(long)]
    reduced_motion: bool,
    /// Open the local hardware view without a broker or credentials.
    #[arg(long, conflicts_with_all = ["broker", "allow_cancel", "world"])]
    machine: bool,
    /// Read scheduler telemetry from this local run directory.
    #[arg(long, requires = "machine")]
    scheduler_telemetry: Option<PathBuf>,
    /// Open a persistent simulation world instead of the broker view.
    #[arg(long, conflicts_with_all = ["broker", "allow_cancel"])]
    world: Option<PathBuf>,
    /// Inspect a retained world revision in a read-only 3D window.
    #[arg(long, requires = "world", conflicts_with = "allow_world_control")]
    world_revision: Option<u64>,
    /// Inspect observed agent relationships from an existing local event journal.
    #[arg(long, conflicts_with_all = ["broker", "allow_cancel", "world", "machine"])]
    swarm: Option<PathBuf>,
    /// Inspect accepted mailbox messages on the signed operator feed.
    #[arg(long, conflicts_with_all = ["swarm", "allow_cancel", "world", "machine"])]
    swarm_broker: bool,
    /// Allow the World Mode window to commit simulation controls.
    #[arg(long, requires = "world")]
    allow_world_control: bool,
}

enum Update {
    Ready(u64),
    Status(&'static str),
    Reset,
    Observation(BrokerEventEntry),
    Cancelled(TaskId, TaskState),
    CancelFailed,
}

fn main() -> ExitCode {
    let options = Options::parse();
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
    let Ok(key) = std::env::var("VESPERA_OPERATOR_TOKEN") else {
        eprintln!("Set VESPERA_OPERATOR_TOKEN to the broker operator signing key.");
        return ExitCode::FAILURE;
    };
    let client =
        SigningKey::from_hex(&key).and_then(|key| OperatorClient::new(options.broker, key));
    let client = match client {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let (observations, receiver) = mpsc::channel(64);
    let (control, actions) = mpsc::channel(4);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Could not start broker client: {error}");
            return ExitCode::FAILURE;
        }
    };
    std::thread::spawn(move || runtime.block_on(observe(client, observations, actions)));
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

async fn observe(
    client: OperatorClient,
    updates: mpsc::Sender<Update>,
    mut actions: mpsc::Receiver<TaskId>,
) {
    let mut cursor = None;
    while !updates.is_closed() {
        match client.subscribe(cursor).await {
            Ok(mut stream) => loop {
                tokio::select! {
                    () = updates.closed() => return,
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
