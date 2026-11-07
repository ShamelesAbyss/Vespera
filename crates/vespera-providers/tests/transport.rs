//! Loopback-only transport boundary regressions, without process-global env mutation.
use futures_util::StreamExt;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vespera_core::ProviderId;
use vespera_providers::{
    BoundedInferenceRequest, CancellationToken, ChatMessage, ChatRole, InferenceProvider,
    OllamaConfig, OllamaProvider, OpenAiCompatibleProvider, OpenAiConfig, ProviderError,
};

fn provider(
    kind: &str,
    url: &str,
    timeout: Duration,
    stream_total: Duration,
) -> Box<dyn InferenceProvider> {
    if kind == "ollama" {
        let mut config = OllamaConfig::loopback_default(ProviderId::new(), url).unwrap();
        config.request_timeout = timeout;
        config.stream_total_timeout = stream_total;
        config.stream_idle_timeout = Duration::from_secs(1);
        Box::new(OllamaProvider::new(config).unwrap())
    } else {
        let mut config = OpenAiConfig::loopback_default(ProviderId::new(), url).unwrap();
        config.request_timeout = timeout;
        config.stream_total_timeout = stream_total;
        config.stream_idle_timeout = Duration::from_secs(1);
        Box::new(OpenAiCompatibleProvider::new(config).unwrap())
    }
}
async fn server<F, Fut>(handler: F) -> String
where
    F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await;
        handler(socket).await;
    });
    format!("http://{address}")
}
fn request() -> BoundedInferenceRequest {
    BoundedInferenceRequest::new(
        "fixture",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "fixture".into(),
        }],
        32,
        512,
        None,
        None,
    )
    .unwrap()
}

#[tokio::test]
async fn json_body_limit_covers_declared_and_undeclared_lengths() {
    for kind in ["ollama", "openai"] {
        for declared in [true, false] {
            let url = server(move |mut socket| async move {
                let header = if declared {
                    "HTTP/1.1 200 OK\r\nContent-Length: 9437184\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n"
                };
                if socket.write_all(header.as_bytes()).await.is_err() {
                    return;
                }
                let block = vec![b' '; 1024 * 1024];
                for _ in 0..9 {
                    if socket.write_all(&block).await.is_err() {
                        return;
                    }
                }
            })
            .await;
            let p = provider(kind, &url, Duration::from_secs(2), Duration::from_secs(2));
            let error = p.health_check(CancellationToken::new()).await.unwrap_err();
            assert!(
                matches!(error, ProviderError::MalformedPayload { ref reason, .. } if reason.contains("limit"))
            );
        }
    }
}

#[tokio::test]
async fn request_deadline_is_shared_between_headers_and_body() {
    for kind in ["ollama", "openai"] {
        let body = if kind == "ollama" {
            r#"{"version":"fixture"}"#
        } else {
            r#"{"data":[]}"#
        };
        let url = server(move |mut socket| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let headers = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            if socket.write_all(headers.as_bytes()).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = socket.write_all(body.as_bytes()).await;
        })
        .await;
        let p = provider(
            kind,
            &url,
            Duration::from_millis(300),
            Duration::from_secs(2),
        );
        assert!(matches!(
            p.health_check(CancellationToken::new()).await,
            Err(ProviderError::Timeout { .. })
        ));
    }
}

#[tokio::test]
async fn continuous_keepalive_traffic_cannot_extend_total_stream_lifetime() {
    for kind in ["ollama", "openai"] {
        let line: &'static [u8] = if kind == "ollama" {
            b"\n"
        } else {
            b": keepalive\n\n"
        };
        let url = server(move |mut socket| async move {
            if socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
            for _ in 0..100 {
                if socket.write_all(line).await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let p = provider(
            kind,
            &url,
            Duration::from_secs(2),
            Duration::from_millis(100),
        );
        let mut stream = p
            .infer_stream(request(), CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(ProviderError::Timeout {
                phase: "stream_total",
                ..
            })
        ));
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn aggregate_stream_limit_covers_small_valid_frames() {
    for kind in ["ollama", "openai"] {
        let line = if kind == "ollama" {
            format!(
                "{{\"message\":{{\"content\":\"{}\"}},\"done\":false}}\n",
                "x".repeat(32 * 1024)
            )
        } else {
            format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}},\"finish_reason\":null}}]}}\n\n",
                "x".repeat(32 * 1024)
            )
        };
        let url = server(move |mut socket| async move {
            if socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
            for _ in 0..600 {
                if socket.write_all(line.as_bytes()).await.is_err() {
                    return;
                }
            }
        })
        .await;
        let p = provider(kind, &url, Duration::from_secs(5), Duration::from_secs(10));
        let mut stream = p
            .infer_stream(request(), CancellationToken::new())
            .await
            .unwrap();
        let mut bounded = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => assert!(!chunk.done),
                Err(ProviderError::MalformedPayload { reason, .. })
                    if reason.contains("aggregate") =>
                {
                    bounded = true;
                    break;
                }
                Err(error) => panic!("unexpected fixture error: {error}"),
            }
        }
        assert!(bounded);
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn local_connections_ignore_proxy_environment_and_use_loopback_hostname() {
    const FIXTURE_URL: &str = "VESPERA_TRANSPORT_FIXTURE_URL";
    const FIXTURE_KIND: &str = "VESPERA_TRANSPORT_FIXTURE_KIND";
    if let Ok(url) = std::env::var(FIXTURE_URL) {
        let kind = std::env::var(FIXTURE_KIND).unwrap();
        provider(&kind, &url, Duration::from_secs(1), Duration::from_secs(1))
            .health_check(CancellationToken::new())
            .await
            .unwrap();
        return;
    }
    for kind in ["ollama", "openai"] {
        let body = if kind == "ollama" {
            r#"{"version":"fixture"}"#
        } else {
            r#"{"data":[]}"#
        };
        let url = server(move |mut socket| async move {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        })
        .await
        .replace("127.0.0.1", "localhost");
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "local_connections_ignore_proxy_environment_and_use_loopback_hostname",
            ])
            .env(FIXTURE_URL, url)
            .env(FIXTURE_KIND, kind);
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(name, &proxy_url);
        }
        command.env("NO_PROXY", "").env("no_proxy", "");
        let output = tokio::task::spawn_blocking(move || command.output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            output.status.success(),
            "proxy isolation subprocess failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), proxy.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn unrelated_json_is_not_successful_health_or_empty_residency() {
    for operation in ["openai-health", "ollama-residency", "ollama-enumeration"] {
        let url = server(|mut socket| async move {
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .await;
        })
        .await;
        let kind = if operation == "openai-health" {
            "openai"
        } else {
            "ollama"
        };
        let p = provider(kind, &url, Duration::from_secs(1), Duration::from_secs(1));
        let malformed = match operation {
            "openai-health" => matches!(
                p.health_check(CancellationToken::new()).await,
                Err(ProviderError::MalformedPayload { .. })
            ),
            "ollama-residency" => matches!(
                p.query_residency(CancellationToken::new()).await,
                Err(ProviderError::MalformedPayload { .. })
            ),
            _ => matches!(
                p.enumerate_models(CancellationToken::new()).await,
                Err(ProviderError::MalformedPayload { .. })
            ),
        };
        assert!(malformed);
    }
}
