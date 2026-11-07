mod scene;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;
use vespera_sdk::{
    BrokerEventEntry, BrokerStreamEnd, BrokerStreamFrame, ClientError, OperatorClient, SigningKey,
    TaskId, TaskState,
};

#[derive(Parser)]
#[command(about = "Vespera native 3D orchestration view")]
struct Options {
    #[arg(long, default_value = "127.0.0.1:8765")]
    broker: SocketAddr,
    #[arg(long)]
    allow_cancel: bool,
}

enum Update {
    Status(&'static str),
    Reset,
    Observation(BrokerEventEntry),
    Cancelled(TaskId, TaskState),
}

fn main() -> ExitCode {
    let options = Options::parse();
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
    scene::run(receiver, options.allow_cancel.then_some(control));
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
                    action = actions.recv(), if !actions.is_closed() => {
                        if let Some(task) = action {
                            let update = match client.cancel(task).await {
                                Ok(state) => Update::Cancelled(task, state),
                                Err(_) => Update::Status("Cancellation failed. Inspect the broker before retrying."),
                            };
                            if updates.send(update).await.is_err() { return; }
                        }
                    }
                    frame = stream.next() => match frame {
                        Ok(Some(BrokerStreamFrame::Ready { discarded_through, .. })) => {
                            let status = if discarded_through == 0 { "Connected - live broker" } else { "Connected - earlier history was pruned" };
                            if updates.send(Update::Status(status)).await.is_err() { return; }
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
