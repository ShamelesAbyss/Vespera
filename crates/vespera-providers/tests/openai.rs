//! Offline OpenAI-compatible protocol fixtures. No external model server is used.
use futures_util::StreamExt;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vespera_core::ProviderId;
use vespera_providers::{
    BoundedInferenceRequest, CancellationToken, ChatMessage, ChatRole, InferenceFinishReason,
    InferenceProvider, OpenAiCompatibleProvider, OpenAiConfig, ProviderError, SecretString,
};

async fn fixture(
    body: String,
    content_type: &'static str,
    status: &'static str,
) -> (String, tokio::sync::oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let count = socket.read(&mut buffer).await.unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|length| length.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
            assert!(bytes.len() < 1024 * 1024);
        }
        let _ = sender.send(String::from_utf8(bytes).unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        // Small writes exercise splitting in both JSON and UTF-8 text.
        for fragment in response.as_bytes().chunks(997) {
            if socket.write_all(fragment).await.is_err() {
                return;
            }
        }
    });
    (format!("http://{address}"), receiver)
}
fn request() -> BoundedInferenceRequest {
    BoundedInferenceRequest::new(
        "fixture-model",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "fixture prompt".into(),
        }],
        32,
        512,
        Some(0.1),
        None,
    )
    .unwrap()
}
fn provider(url: &str) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(OpenAiConfig::loopback_default(ProviderId::new(), url).unwrap())
        .unwrap()
}

#[tokio::test]
async fn endpoint_v1_suffix_is_not_duplicated_and_bounded_request_is_sent() {
    let (url, received) = fixture(r#"{"choices":[{"message":{"role":"assistant","content":"answer"},"finish_reason":"length"}],"usage":{"prompt_tokens":9,"completion_tokens":32}}"#.into(), "application/json", "200 OK").await;
    let mut config =
        OpenAiConfig::loopback_default(ProviderId::new(), &format!("{url}/v1/")).unwrap();
    config.pre_resolved_secret = Some(SecretString::new_in_memory("fixture-token".into()));
    let p = OpenAiCompatibleProvider::new(config).unwrap();
    let response = p.infer(request(), CancellationToken::new()).await.unwrap();
    assert_eq!(response.content, "answer");
    assert_eq!(response.finish_reason, InferenceFinishReason::Length);
    assert_eq!(response.metrics.completion_tokens, Some(32));
    let received = received.await.unwrap();
    assert!(
        received.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"),
        "API prefix must occur once"
    );
    assert!(
        received
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-token")
    );
    let (_, body) = received.split_once("\r\n\r\n").unwrap();
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["model"], "fixture-model");
    assert_eq!(json["max_tokens"], 32);
    assert_eq!(json["stream"], false);
}

#[tokio::test]
async fn eof_without_terminal_event_is_an_error() {
    let (url, _) = fixture(
        "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n"
            .into(),
        "text/event-stream",
        "200 OK",
    )
    .await;
    let p = provider(&url);
    let mut stream = p
        .infer_stream(request(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "partial");
    assert!(matches!(
        stream.next().await.unwrap(),
        Err(ProviderError::StreamInterrupted { .. })
    ));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn sse_events_preserve_finish_reason_and_trailing_usage() {
    let body = ": keepalive\r\nevent: message\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"héllo\"},\"finish_reason\":null}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":32}}\n\ndata: [DONE]\n\n";
    let (url, _) = fixture(body.into(), "text/event-stream", "200 OK").await;
    let p = provider(&url);
    let mut stream = p
        .infer_stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut content = String::new();
    let mut terminal = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        content.push_str(&chunk.delta);
        if chunk.done {
            terminal = Some(chunk);
        }
    }
    assert_eq!(content, "héllo");
    let terminal = terminal.unwrap();
    assert_eq!(terminal.finish_reason, Some(InferenceFinishReason::Length));
    assert_eq!(terminal.metrics.unwrap().completion_tokens, Some(32));
}

#[tokio::test]
async fn multiline_data_event_is_parsed_as_one_json_payload() {
    let body = "data: {\"choices\":\n".to_string()
        + "data: [{\"delta\":{\"content\":\"joined\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
    let (url, _) = fixture(body, "text/event-stream", "200 OK").await;
    let p = provider(&url);
    let mut stream = p
        .infer_stream(request(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap().delta, "joined");
    let terminal = stream.next().await.unwrap().unwrap();
    assert!(terminal.done);
    assert_eq!(terminal.finish_reason, Some(InferenceFinishReason::Unknown));
}

#[tokio::test]
async fn malformed_or_oversized_frames_fail_and_stream_terminates() {
    for body in [
        "data: {broken}\n\n".to_string(),
        format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}},\"finish_reason\":null}}]}}\n\n",
            "x".repeat(256 * 1024)
        ),
    ] {
        let (url, _) = fixture(body, "text/event-stream", "200 OK").await;
        let p = provider(&url);
        let mut stream = p
            .infer_stream(request(), CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Err(ProviderError::MalformedPayload { .. })
        ));
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn authorization_and_redirect_failures_do_not_echo_response_bodies() {
    for (status, auth) in [
        ("401 Unauthorized", true),
        ("403 Forbidden", true),
        ("302 Found", false),
    ] {
        let (url, _) = fixture("fixture-secret-error-body".into(), "text/plain", status).await;
        let error = provider(&url)
            .health_check(CancellationToken::new())
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("fixture-secret-error-body"));
        if auth {
            assert!(matches!(error, ProviderError::AuthenticationFailed { .. }));
        } else {
            assert!(matches!(
                error,
                ProviderError::TransportPolicyViolation { .. }
            ));
        }
    }
}

#[tokio::test]
async fn cancellation_is_observed_before_starting_transport() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let p = provider("http://127.0.0.1:1");
    assert!(matches!(
        p.infer(request(), cancel.clone()).await,
        Err(ProviderError::Cancelled { .. })
    ));
    assert!(matches!(
        p.infer_stream(request(), cancel).await,
        Err(ProviderError::Cancelled { .. })
    ));
    assert!(
        p.load_model(
            "fixture-model",
            Some(Duration::from_secs(1)),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn aggregate_sse_event_size_is_bounded_across_small_data_lines() {
    let body = format!(
        "data: {{\"choices\":\n{}data: []}}\n\n",
        format!("data: {}\n", " ".repeat(80 * 1024)).repeat(4)
    );
    let (url, _) = fixture(body, "text/event-stream", "200 OK").await;
    let p = provider(&url);
    let mut stream = p
        .infer_stream(request(), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await.unwrap(),
        Err(ProviderError::MalformedPayload { .. })
    ));
}

#[tokio::test]
async fn unsupported_metadata_and_residency_are_not_fabricated() {
    let p = provider("http://127.0.0.1:1");
    assert!(matches!(
        p.model_metadata("fixture", CancellationToken::new()).await,
        Err(ProviderError::UnsupportedOperation { .. })
    ));
    assert!(matches!(
        p.query_residency(CancellationToken::new()).await,
        Err(ProviderError::UnsupportedOperation { .. })
    ));
    assert_eq!(
        p.lifecycle_support().model_metadata,
        vespera_providers::LifecycleCapabilityState::Unsupported
    );
}
