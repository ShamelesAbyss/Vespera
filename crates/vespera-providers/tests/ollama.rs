//! Deterministic integration and chaos fixtures for `vespera-providers`.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use vespera_providers::core_bridge::ProviderId;
use vespera_providers::{
    BoundedInferenceRequest, CancellationToken, ChatMessage, ChatRole, EndpointPolicy,
    InferenceFinishReason, InferenceProvider, LifecycleCapabilityState, OllamaConfig,
    OllamaProvider, ProviderError, ProviderHealthStatus, SecretRef, SecretString,
    ValidatedEndpoint,
};

fn sample_provider_id() -> ProviderId {
    "0195f000-0000-7000-8000-000000000001"
        .parse()
        .expect("valid non-nil ProviderId")
}

/// Spawns a single-request raw HTTP fixture server on `127.0.0.1:0` to allow exact TCP packet fragmentation.
async fn spawn_raw_http_fixture<F, Fut>(handler: F) -> String
where
    F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut req_buf = [0u8; 2048];
            let _ = socket.read(&mut req_buf).await;
            handler(socket).await;
        }
    });
    format!("http://127.0.0.1:{}", addr.port())
}

#[tokio::test]
async fn object_safety_and_lifecycle_negotiation_work() {
    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, "http://127.0.0.1:11434")
        .expect("valid loopback config");
    let provider: Arc<dyn InferenceProvider> =
        Arc::new(OllamaProvider::new(cfg).expect("provider init"));

    let support = provider.lifecycle_support();
    assert_eq!(
        support.model_enumeration,
        LifecycleCapabilityState::Supported
    );
    assert_eq!(
        support.residency_telemetry,
        LifecycleCapabilityState::Supported
    );
    assert_eq!(
        support.explicit_model_unload,
        LifecycleCapabilityState::Supported
    );
}

#[tokio::test]
async fn vram_residency_telemetry_parses_accurately() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let body = r#"{
            "models": [
                {
                    "name": "qwen2.5-coder:7b",
                    "size": 5368709120,
                    "size_vram": 4294967296,
                    "context_length": 8192
                }
            ]
        }"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(resp.as_bytes()).await;
    })
    .await;

    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    let provider = OllamaProvider::new(cfg).expect("provider");

    let snapshot = provider
        .query_residency(CancellationToken::new())
        .await
        .expect("residency snapshot");

    assert_eq!(snapshot.resident_models.len(), 1);
    let model = &snapshot.resident_models[0];
    assert_eq!(model.provider_model_name, "qwen2.5-coder:7b");
    assert_eq!(model.total_resident_bytes, 5_368_709_120);
    assert_eq!(model.vram_resident_bytes, 4_294_967_296);
    assert_eq!(model.ram_resident_bytes, 1_073_741_824);
    assert_eq!(model.allocated_context_tokens, Some(8192));
}

#[tokio::test]
async fn fragmented_ndjson_stream_reassembles_across_tcp_splits() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n";
        let _ = socket.write_all(headers.as_bytes()).await;

        // Split the first JSON line across three separate TCP writes to test framing resilience.
        let part1 = br#"{"message":{"role":"assistant","con"#;
        let part2 = br#"tent":"Hello "},"done":false}"#;
        let part3 = b"\n{\"message\":{\"role\":\"assistant\",\"content\":\"world\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":12,\"eval_count\":4}\n";

        let _ = socket.write_all(part1).await;
        let _ = socket.flush().await;
        tokio::time::sleep(Duration::from_millis(15)).await;

        let _ = socket.write_all(part2).await;
        let _ = socket.flush().await;
        tokio::time::sleep(Duration::from_millis(15)).await;

        let _ = socket.write_all(part3).await;
        let _ = socket.flush().await;
    })
    .await;

    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    let provider = OllamaProvider::new(cfg).expect("provider");

    let req = BoundedInferenceRequest::new(
        "llama3.2:3b",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Greet me.".to_string(),
        }],
        64,
        4096,
        Some(0.2),
        None,
    )
    .expect("valid bounded request");

    let mut stream = provider
        .infer_stream(req, CancellationToken::new())
        .await
        .expect("stream started");

    let first = stream
        .next()
        .await
        .expect("chunk 1")
        .expect("valid chunk 1");
    assert_eq!(first.delta, "Hello ");
    assert!(!first.done);

    let second = stream
        .next()
        .await
        .expect("chunk 2")
        .expect("valid chunk 2");
    assert_eq!(second.delta, "world");
    assert!(second.done);
    assert_eq!(second.finish_reason, Some(InferenceFinishReason::Stop));
    assert_eq!(second.metrics.expect("metrics").completion_tokens, Some(4));
}

#[tokio::test]
async fn malformed_ndjson_frame_is_rejected_cleanly() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let payload =
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n{corrupted_json_frame\n";
        let _ = socket.write_all(payload.as_bytes()).await;
    })
    .await;

    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    let provider = OllamaProvider::new(cfg).expect("provider");

    let req = BoundedInferenceRequest::new(
        "llama3.2:3b",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Test".to_string(),
        }],
        32,
        2048,
        None,
        None,
    )
    .expect("request");

    let mut stream = provider
        .infer_stream(req, CancellationToken::new())
        .await
        .expect("stream");
    let err = stream
        .next()
        .await
        .expect("item")
        .expect_err("must fail on malformed JSON");
    assert!(matches!(
        err,
        ProviderError::MalformedPayload {
            context: "ndjson_stream",
            ..
        }
    ));
}

#[tokio::test]
async fn cancellation_race_aborts_in_flight_stream_immediately() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n";
        let _ = socket.write_all(headers.as_bytes()).await;
        let first_line =
            b"{\"message\":{\"role\":\"assistant\",\"content\":\"Part 1\"},\"done\":false}\n";
        let _ = socket.write_all(first_line).await;
        let _ = socket.flush().await;
        // Stall long enough for the client to trigger cancellation.
        tokio::time::sleep(Duration::from_secs(10)).await;
    })
    .await;

    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    let provider = OllamaProvider::new(cfg).expect("provider");

    let req = BoundedInferenceRequest::new(
        "llama3.2:3b",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Long task".to_string(),
        }],
        128,
        4096,
        None,
        None,
    )
    .expect("request");

    let cancel = CancellationToken::new();
    let mut stream = provider
        .infer_stream(req, cancel.clone())
        .await
        .expect("stream");

    let first = stream.next().await.expect("first").expect("ok");
    assert_eq!(first.delta, "Part 1");

    // Trigger cancellation while waiting for the second chunk.
    cancel.cancel();

    let second = stream
        .next()
        .await
        .expect("cancelled item")
        .expect_err("must be cancelled");
    assert_eq!(
        second,
        ProviderError::Cancelled {
            provider_id,
            phase: "stream_chunk",
        }
    );
}

#[tokio::test]
async fn stream_idle_timeout_triggers_when_server_stalls() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\r\n";
        let _ = socket.write_all(headers.as_bytes()).await;
        let _ = socket.flush().await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    })
    .await;

    let provider_id = sample_provider_id();
    let mut cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    cfg.stream_idle_timeout = Duration::from_millis(80);
    let provider = OllamaProvider::new(cfg).expect("provider");

    let req = BoundedInferenceRequest::new(
        "llama3.2:3b",
        vec![ChatMessage {
            role: ChatRole::User,
            content: "Stall test".to_string(),
        }],
        64,
        2048,
        None,
        None,
    )
    .expect("request");

    let mut stream = provider
        .infer_stream(req, CancellationToken::new())
        .await
        .expect("stream");
    let err = stream
        .next()
        .await
        .expect("timeout item")
        .expect_err("must timeout");
    assert!(matches!(
        err,
        ProviderError::Timeout {
            phase: "stream_chunk",
            ..
        }
    ));
}

#[tokio::test]
async fn transport_policy_blocks_http_redirects_and_redacts_secrets() {
    let base_url = spawn_raw_http_fixture(|mut socket| async move {
        let redirect =
            "HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data\r\nContent-Length: 0\r\n\r\n";
        let _ = socket.write_all(redirect.as_bytes()).await;
    })
    .await;

    let provider_id = sample_provider_id();
    let mut cfg = OllamaConfig::loopback_default(provider_id, &base_url).expect("config");
    let secret = SecretString::new_in_memory("super-secret-token-value-999".to_string());
    assert!(!format!("{secret:?}").contains("super-secret-token-value-999"));
    assert!(!format!("{secret}").contains("super-secret-token-value-999"));
    cfg.pre_resolved_secret = Some(secret);

    let provider = OllamaProvider::new(cfg).expect("provider");
    let err = provider
        .health_check(CancellationToken::new())
        .await
        .expect_err("redirect must be blocked");

    assert!(matches!(
        err,
        ProviderError::TransportPolicyViolation { .. }
    ));
    assert!(!err.to_string().contains("super-secret-token-value-999"));

    // Verify non-loopback endpoint is rejected by default policy.
    let remote_err = ValidatedEndpoint::parse(
        provider_id,
        "https://api.example.com:11434",
        EndpointPolicy::default(),
    )
    .expect_err("remote blocked by default");
    assert!(matches!(
        remote_err,
        ProviderError::TransportPolicyViolation { .. }
    ));

    // Verify invalid env var identifier is rejected without leaking anything.
    let bad_env = SecretRef::new(provider_id, "bad-env-name!");
    assert!(bad_env.is_err());
}

#[tokio::test]
#[ignore = "requires live local Ollama instance on 127.0.0.1:11434; run with --ignored"]
async fn live_ollama_smoke_test() {
    let provider_id = sample_provider_id();
    let cfg = OllamaConfig::loopback_default(provider_id, "http://127.0.0.1:11434")
        .expect("loopback config");
    let provider = OllamaProvider::new(cfg).expect("provider");
    let report = provider
        .health_check(CancellationToken::new())
        .await
        .expect("live health check");
    assert_eq!(report.status, ProviderHealthStatus::Healthy);
}

async fn json_fixture(body: &str) -> String {
    let body = body.to_owned();
    spawn_raw_http_fixture(|mut socket| async move {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
    })
    .await
}

#[tokio::test]
async fn metadata_does_not_invent_text_capability_for_embedding_only_models() {
    let url = json_fixture(r#"{"details":{"family":"embed","parameter_size":"1B","quantization_level":"F16"},"model_info":{"embed.context_length":512},"capabilities":["embedding"]}"#).await;
    let p =
        OllamaProvider::new(OllamaConfig::loopback_default(sample_provider_id(), &url).unwrap())
            .unwrap();
    let metadata = p
        .model_metadata("fixture", CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(metadata.context_window_tokens, Some(512));
    assert!(
        metadata
            .capabilities
            .contains(&"embeddings".parse().unwrap())
    );
    assert!(
        !metadata
            .capabilities
            .contains(&"text-generation".parse().unwrap())
    );
    assert_eq!(metadata.estimated_resources.ram_bytes, None);
}

#[tokio::test]
async fn invalid_or_overflowing_residency_is_rejected_instead_of_clamped() {
    for body in [
        r#"{"models":[{"name":"bad","size":1,"size_vram":2}]}"#.to_string(),
        format!(
            "{{\"models\":[{{\"name\":\"a\",\"size\":{},\"size_vram\":0}},{{\"name\":\"b\",\"size\":1,\"size_vram\":0}}]}}",
            u64::MAX
        ),
    ] {
        let url = json_fixture(&body).await;
        let p = OllamaProvider::new(
            OllamaConfig::loopback_default(sample_provider_id(), &url).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            p.query_residency(CancellationToken::new()).await,
            Err(ProviderError::MalformedPayload { .. })
        ));
    }
}

#[tokio::test]
async fn nonstreaming_inference_requires_a_terminal_done_response() {
    let url =
        json_fixture(r#"{"message":{"role":"assistant","content":"partial"},"done":false}"#).await;
    let p =
        OllamaProvider::new(OllamaConfig::loopback_default(sample_provider_id(), &url).unwrap())
            .unwrap();
    let request = BoundedInferenceRequest::new(
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
    .unwrap();
    assert!(matches!(
        p.infer(request, CancellationToken::new()).await,
        Err(ProviderError::StreamInterrupted { .. })
    ));
}

#[tokio::test]
async fn enumeration_and_explicit_lifecycle_parse_provider_responses() {
    let url = json_fixture(r#"{"models":[{"name":"fixture","digest":"fixture-digest","size":100,"details":{"parameter_size":"1B","quantization_level":"Q4"}}]}"#).await;
    let p =
        OllamaProvider::new(OllamaConfig::loopback_default(sample_provider_id(), &url).unwrap())
            .unwrap();
    let models = p.enumerate_models(CancellationToken::new()).await.unwrap();
    assert_eq!(models[0].provider_model_name, "fixture");
    assert_eq!(models[0].size_bytes, Some(100));
    for load in [true, false] {
        let url = json_fixture(r#"{"done":true}"#).await;
        let p = OllamaProvider::new(
            OllamaConfig::loopback_default(sample_provider_id(), &url).unwrap(),
        )
        .unwrap();
        let outcome = if load {
            p.load_model(
                "fixture",
                Some(Duration::from_secs(10)),
                CancellationToken::new(),
            )
            .await
        } else {
            p.unload_model("fixture", CancellationToken::new()).await
        }
        .unwrap();
        assert!(outcome.acknowledged);
    }
}

#[tokio::test]
async fn contextual_preload_rejects_invalid_context_before_network_io() {
    let p = OllamaProvider::new(
        OllamaConfig::loopback_default(sample_provider_id(), "http://127.0.0.1:1").unwrap(),
    )
    .unwrap();
    for tokens in [0, 63, 1_048_577, u32::MAX] {
        assert!(matches!(
            p.load_model_with_context("fixture", tokens, None, CancellationToken::new())
                .await,
            Err(ProviderError::RequestValidation { .. })
        ));
    }
}
