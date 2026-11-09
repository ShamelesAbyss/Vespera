mod atlas;
mod branding;
mod desktop_agent;
mod desktop_evidence;
mod desktop_workflow;
mod gpu_telemetry;
mod home_scene;
mod machine_scene;
mod provider_scene;
mod scene;
mod scene_measurement;
mod swarm_art;
mod swarm_scene;
mod task_timeline;
mod workflow_history_scene;
mod world_art;
mod world_scene;

use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use bevy::prelude::Color;
use clap::{CommandFactory, Parser, error::ErrorKind};
use tokio::sync::mpsc;
use vespera_sdk::{
    BrokerEventEntry, BrokerSessionId, BrokerStreamEnd, BrokerStreamFrame, ClientError,
    OperatorClient, SigningKey, TaskId, TaskState,
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
    /// Trust this PEM certificate bundle and require TLS for the explicit broker.
    #[arg(long, requires = "broker")]
    broker_tls_ca: Option<PathBuf>,
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
    /// Save a PNG from the standalone Swarm view after the journal settles.
    #[arg(long, requires = "swarm", conflicts_with = "atlas")]
    swarm_capture: Option<PathBuf>,
    /// Capture the first newly observed Swarm transit midway along its route.
    #[arg(long, requires = "swarm_capture", conflicts_with = "reduced_motion")]
    swarm_capture_transit: bool,
    /// Select and focus an observed agent on standalone Swarm startup.
    #[arg(long, requires = "swarm", conflicts_with = "atlas")]
    swarm_agent: Option<vespera_core::AgentId>,
    /// Print a bounded native update-interval sample in standalone Swarm.
    #[arg(long, requires = "swarm", conflicts_with = "atlas", value_parser = clap::value_parser!(u32).range(60..=3600))]
    swarm_measure_frames: Option<u32>,
    /// Inspect accepted mailbox messages on the signed operator feed.
    #[arg(long, conflicts_with_all = ["swarm", "allow_cancel", "world", "machine"])]
    swarm_broker: bool,
    /// Allow the World Mode window to commit simulation controls.
    #[arg(long, requires = "world")]
    allow_world_control: bool,
}

#[derive(Clone)]
enum Update {
    Ready {
        floor: u64,
        session: BrokerSessionId,
    },
    Status(&'static str),
    Reset,
    Observation(BrokerEventEntry),
    Cancelled(TaskId, TaskState),
    CancelFailed,
    CancelNotSent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CancelRequest {
    task: TaskId,
    session: BrokerSessionId,
}

fn main() -> ExitCode {
    let mut options = Options::parse();
    if let Some(error) = options.conflict() {
        Options::command()
            .error(ErrorKind::ArgumentConflict, error)
            .exit();
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = linux_graphics_prerequisites() {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
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
        return match swarm_scene::run(
            path,
            options.reduced_motion,
            options.swarm_capture,
            options.swarm_capture_transit,
            options.swarm_agent,
            options.swarm_measure_frames,
        ) {
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
    let (receiver, control) = match start_broker(address, options.broker_tls_ca.as_deref(), None) {
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

#[cfg(target_os = "linux")]
fn linux_graphics_prerequisites() -> Result<(), &'static str> {
    if std::env::var_os("DISPLAY").is_none_or(|display| display.is_empty()) {
        return Err(
            "An X11 display is required for graphics. Use the vespera CLI for headless operation.",
        );
    }
    xkbcommon_dl::x11::xkbcommon_x11_option()
        .ok_or("The X11 keyboard runtime libxkbcommon-x11.so.0 could not be loaded. Install your distribution's X11 keyboard runtime.")?;
    Ok(())
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
        let (updates, control) = start_broker(address, options.broker_tls_ca.as_deref(), mirror)?;
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
    roots: Option<&Path>,
    mirror: Option<mpsc::Sender<Update>>,
) -> Result<(mpsc::Receiver<Update>, mpsc::Sender<CancelRequest>), Box<dyn std::error::Error>> {
    let key = std::env::var("VESPERA_OPERATOR_TOKEN")
        .map_err(|_| "Set VESPERA_OPERATOR_TOKEN to the broker operator signing key.")?;
    let client = broker_client(address, SigningKey::from_hex(&key)?, roots)?;
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

fn broker_client(
    address: SocketAddr,
    key: SigningKey,
    roots: Option<&Path>,
) -> Result<OperatorClient, Box<dyn std::error::Error>> {
    match roots {
        Some(path) => {
            let mut pem = Vec::new();
            std::fs::File::open(path)?
                .take(128 * 1024 + 1)
                .read_to_end(&mut pem)?;
            Ok(OperatorClient::new_tls(address, key, &pem)?)
        }
        None => Ok(OperatorClient::new(address, key)?),
    }
}

async fn observe(client: OperatorClient, updates: Feeds, actions: mpsc::Receiver<CancelRequest>) {
    let primary = updates.primary.clone();
    tokio::select! {
        () = primary.closed() => {},
        () = observe_connected(client, updates, actions) => {},
    }
}

async fn observe_connected(
    client: OperatorClient,
    updates: Feeds,
    mut actions: mpsc::Receiver<CancelRequest>,
) {
    const DENIED: Update = Update::Status("Broker authorization denied - check the operator key");
    const INVALID: Update = Update::Status("Broker sent invalid event data - subscription stopped");
    let mut cursor = None;
    while !updates.primary.is_closed() {
        match client.subscribe(cursor).await {
            Ok(mut stream) => {
                let session = stream.session_id();
                let mut ready = false;
                loop {
                    tokio::select! {
                        () = updates.primary.closed() => return,
                        action = actions.recv(), if ready && !actions.is_closed() => {
                            if let Some(request) = action {
                                let update = if request.session != session {
                                    Update::CancelNotSent
                                } else {
                                    match client.cancel_in_session(request.task, request.session).await {
                                        Ok(state) => Update::Cancelled(request.task, state),
                                        Err(ClientError::SessionChanged) => Update::CancelNotSent,
                                        Err(_) => Update::CancelFailed,
                                    }
                                };
                                if updates.send(update).await.is_err() { return; }
                            }
                        }
                        frame = stream.next() => match frame {
                            Ok(Some(BrokerStreamFrame::Ready { discarded_through, .. })) => {
                                if updates.send(Update::Ready { floor: discarded_through, session }).await.is_err() { return; }
                                ready = true;
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
                            Ok(Some(BrokerStreamFrame::End { reason: BrokerStreamEnd::Unauthorized })) => {
                                let _ = updates.send(DENIED).await;
                                return;
                            }
                            Err(ClientError::InvalidEventFrame) => {
                                let _ = updates.send(INVALID).await;
                                return;
                            }
                            _ => break,
                        }
                    }
                }
            }
            Err(ClientError::Rejected {
                status: 409 | 410, ..
            }) => {
                cursor = None;
                if updates.send(Update::Reset).await.is_err() {
                    return;
                }
            }
            Err(ClientError::Rejected {
                status: 401 | 403, ..
            }) => {
                let _ = updates.send(DENIED).await;
                return;
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

    #[tokio::test]
    async fn closing_the_view_cancels_a_stalled_broker_handshake() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = broker_client(
            listener.local_addr().unwrap(),
            SigningKey::from_hex(&"42".repeat(32)).unwrap(),
            None,
        )
        .unwrap();
        let (primary, updates) = mpsc::channel(1);
        let (_actions, receiver) = mpsc::channel(1);
        let observer = tokio::spawn(observe(
            client,
            Feeds {
                primary,
                mirror: None,
            },
            receiver,
        ));
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = [0; 4096];
        assert!(
            tokio::time::timeout(Duration::from_secs(5), socket.read(&mut request))
                .await
                .unwrap()
                .unwrap()
                > 0
        );
        // Keep the accepted socket open without answering the session request.
        drop(updates);
        tokio::time::timeout(Duration::from_secs(1), observer)
            .await
            .unwrap()
            .unwrap();
        loop {
            let count = tokio::time::timeout(Duration::from_secs(1), socket.read(&mut request))
                .await
                .unwrap()
                .unwrap();
            if count == 0 {
                break;
            }
        }
    }

    #[tokio::test]
    async fn closing_the_view_interrupts_the_reconnect_delay() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = broker_client(
            address,
            SigningKey::from_hex(&"42".repeat(32)).unwrap(),
            None,
        )
        .unwrap();
        let (primary, mut updates) = mpsc::channel(1);
        let (_actions, receiver) = mpsc::channel(1);
        let observer = tokio::spawn(observe(
            client,
            Feeds {
                primary,
                mirror: None,
            },
            receiver,
        ));
        let status = tokio::time::timeout(Duration::from_secs(5), updates.recv())
            .await
            .unwrap();
        assert!(matches!(
            status,
            Some(Update::Status("Disconnected - reconnecting"))
        ));
        drop(updates);
        // Shutdown must not wait for the two-second reconnect sleep.
        tokio::time::timeout(Duration::from_secs(1), observer)
            .await
            .unwrap()
            .unwrap();
    }
    #[test]
    fn broker_tls_requires_an_explicit_broker_and_preserves_mode_conflicts() {
        assert!(Options::try_parse_from(["vespera-view", "--broker-tls-ca", "ca.pem"]).is_err());
        for mode in ["--atlas", "--swarm-broker"] {
            let options = Options::try_parse_from([
                "vespera-view",
                mode,
                "--broker",
                "192.0.2.1:8765",
                "--broker-tls-ca",
                "ca.pem",
            ])
            .unwrap();
            assert!(!options.opens_home());
            assert!(options.conflict().is_none());
        }
        assert!(
            Options::try_parse_from([
                "vespera-view",
                "--machine",
                "--broker",
                "192.0.2.1:8765",
                "--broker-tls-ca",
                "ca.pem",
            ])
            .is_err()
        );
        let key = SigningKey::from_hex(&"42".repeat(32)).unwrap();
        assert!(broker_client("192.0.2.1:8765".parse().unwrap(), key, None).is_err());
    }

    #[tokio::test]
    #[ignore = "requires an isolated TLS broker and VESPERA_REHEARSAL_* environment"]
    async fn desktop_tls_feed_observes_and_cancels_its_own_fixture_task() {
        let address = std::env::var("VESPERA_REHEARSAL_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let roots = PathBuf::from(std::env::var_os("VESPERA_REHEARSAL_CA").unwrap());
        let key = std::env::var("VESPERA_REHEARSAL_OPERATOR_KEY").unwrap();
        let client =
            || broker_client(address, SigningKey::from_hex(&key).unwrap(), Some(&roots)).unwrap();
        let operator = client();
        let (primary, mut updates) = mpsc::channel(64);
        let (actions, receiver) = mpsc::channel(4);
        let observer = tokio::spawn(observe(
            client(),
            Feeds {
                primary,
                mirror: None,
            },
            receiver,
        ));
        tokio::time::timeout(Duration::from_secs(30), async {
            let Some(Update::Ready { session, .. }) = updates.recv().await else { panic!("TLS feed did not become ready"); };
            let task = TaskId::new();
            operator.submit(&vespera_sdk::Submission {
                task_id: task,
                prompt: "Desktop TLS transport fixture".into(),
                required_capabilities: ["text-generation".parse().unwrap()].into(),
                eligible_workers: [std::env::var("VESPERA_REHEARSAL_FIRST").unwrap().parse().unwrap()].into(),
                lifetime_seconds: 60,
                max_attempts: 1,
            }).await.unwrap();
            loop {
                match updates.recv().await {
                    Some(Update::Observation(entry)) if matches!(entry.event.kind, vespera_core::EventKind::ExternalTaskChanged { task_id, .. } if task_id == task) => break,
                    Some(Update::Observation(_)) => {},
                    _ => panic!("feed interrupted before observing the submitted task"),
                }
            }
            actions.send(CancelRequest { task, session }).await.unwrap();
            loop {
                match updates.recv().await {
                    Some(Update::Cancelled(id, TaskState::Cancelled)) if id == task => break,
                    Some(Update::Observation(_)) => {},
                    _ => panic!("fixture cancellation failed"),
                }
            }
            assert_eq!(operator.inspect(task).await.unwrap().state, TaskState::Cancelled);
            drop(updates);
            observer.await.unwrap();
        }).await.unwrap();
    }

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
