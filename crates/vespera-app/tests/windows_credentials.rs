#![cfg(target_os = "windows")]

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use vespera_app::runtime::{RuntimeError, configured_registry, windows_credential_target};
use vespera_config::AppConfig;
use vespera_core::{ProviderKind, TaskId};
use vespera_providers::CancellationToken;

struct FixtureCredential(keyring::Entry);

impl Drop for FixtureCredential {
    fn drop(&mut self) {
        let _ = self.0.delete_credential();
    }
}

#[tokio::test]
#[ignore = "writes a temporary synthetic credential to the current Windows user store"]
async fn reads_the_endpoint_bound_windows_credential_for_provider_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = AppConfig::parse(include_str!("../../../config/local.toml")).unwrap();
    let provider = &mut config.providers[0];
    provider.kind = ProviderKind::OpenAiCompatible;
    provider.credential_windows = true;
    provider.endpoint = format!(
        "http://{}/fixture-{}/v1",
        listener.local_addr().unwrap(),
        TaskId::new()
    );
    let provider_id = provider.id;
    let target = windows_credential_target(provider);
    let token = format!("fixture-{}", TaskId::new());
    let entry =
        FixtureCredential(keyring::Entry::new_with_target(&target, "Vespera", "provider").unwrap());
    entry.0.set_password(&token).unwrap();
    let authorization = format!("Bearer {token}");
    assert!(!format!("{config:?}").contains(&token));

    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(socket);
        let mut found = false;
        loop {
            let mut line = String::new();
            assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
            if line.trim_end().is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':')
                && key.eq_ignore_ascii_case("authorization")
            {
                found = value.trim() == authorization;
            }
        }
        assert!(found, "fixture did not receive the Windows credential");
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"data\":[]}").await.unwrap();
    });
    let registry = configured_registry(&config).await.unwrap();
    let provider = registry.get_provider(provider_id).await.unwrap();
    tokio::time::timeout(
        Duration::from_secs(5),
        provider.health_check(CancellationToken::new()),
    )
    .await
    .unwrap()
    .unwrap();
    server.await.unwrap();

    config.providers[0].endpoint = config.providers[0]
        .endpoint
        .replace("/fixture-", "/Fixture-");
    assert!(matches!(
        configured_registry(&config).await,
        Err(RuntimeError::WindowsCredentialUnavailable { .. })
    ));
    entry.0.delete_credential().unwrap();
}
