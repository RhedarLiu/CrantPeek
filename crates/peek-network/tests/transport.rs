use peek_core::{Message, Provider};
use peek_network::{Client, Event};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn streams_from_local_http_service() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 8192];
        let n = socket.read(&mut buffer).await.unwrap();
        let request = String::from_utf8_lossy(&buffer[..n]);
        assert!(request.starts_with("POST /v1/chat/completions"));
        assert!(request.contains("Bearer test-secret"));
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\ndata: [DONE]\n\n";
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    let provider = Provider {
        base_url: format!("http://{address}/v1"),
        model: "test".into(),
        ..Provider::default()
    };
    let (tx, mut rx) = mpsc::channel(8);
    Client::default()
        .stream(
            &provider,
            "test-secret",
            &[Message {
                role: "user".into(),
                content: "hello".into(),
            }],
            tx,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(rx.recv().await, Some(Event::Text("你好".into())));
    assert_eq!(rx.recv().await, Some(Event::Done));
    server.await.unwrap();
}

#[tokio::test]
async fn responses_and_anthropic_complete_over_http() {
    use peek_core::Protocol;
    for (protocol, path, body) in [
        (
            Protocol::Responses,
            "/v1/responses",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\ndata: {\"type\":\"response.completed\"}\n\n",
        ),
        (
            Protocol::Anthropic,
            "/v1/messages",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 8192];
            let n = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]).to_lowercase();
            assert!(request.starts_with(&format!("post {path}")));
            if protocol == Protocol::Anthropic {
                assert!(request.contains("x-api-key: test-secret"));
                assert!(request.contains("anthropic-version: 2023-06-01"));
                assert!(!request.contains("authorization:"));
            } else {
                assert!(request.contains("authorization: bearer test-secret"));
            }
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let provider = Provider {
            protocol,
            base_url: format!("http://{address}/v1"),
            model: "test".into(),
            ..Provider::default()
        };
        let (tx, mut rx) = mpsc::channel(8);
        Client::default()
            .stream(
                &provider,
                "test-secret",
                &[Message {
                    role: "user".into(),
                    content: "hello".into(),
                }],
                tx,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(rx.recv().await, Some(Event::Text("hello".into())));
        assert_eq!(rx.recv().await, Some(Event::Done));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn errors_and_redirects_do_not_look_like_success() {
    for (status, extra, body, expected) in [
        ("429 Too Many Requests", "", "", "HTTP 429"),
        (
            "302 Found",
            "Location: http://127.0.0.1:1/private\r\n",
            "",
            "HTTP 302",
        ),
        (
            "200 OK",
            "Content-Type: text/event-stream\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "error-stream-ended",
        ),
        (
            "200 OK",
            "Content-Type: text/html\r\n",
            "<html>login</html>",
            "error-sse-type",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 8192];
            let _ = socket.read(&mut buffer).await.unwrap();
            socket.write_all(format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let provider = Provider {
            base_url: format!("http://{address}/v1"),
            model: "test".into(),
            ..Provider::default()
        };
        let (tx, mut rx) = mpsc::channel(8);
        let error = Client::default()
            .stream(&provider, "secret", &[], tx, CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        while let Some(event) = rx.recv().await {
            assert_ne!(event, Event::Done);
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn decision_accepts_direct_and_cloudflare_envelopes() {
    for wrapped in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&request[..header_end]);
                    let length: usize = header
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= header_end + 4 + length {
                        break;
                    }
                }
            }
            let header_end = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end + 4..]).unwrap();
            assert_eq!(body["state"], "error: failure");
            assert_eq!(body["questions"]["task"]["type"], "choice");
            let response = serde_json::json!({"answers":{"task":{"type":"choice","choice":"explain_error","confidence":0.9,"probabilities":{"explain_error":0.92,"translate":0.08}}}});
            let response = if wrapped {
                serde_json::json!({"success":true,"result":response})
            } else {
                response
            };
            let text = response.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len()).as_bytes()).await.unwrap();
        });
        let result = Client::default()
            .decide(
                &format!("http://{address}/decision"),
                "test",
                "test",
                "error: failure",
                std::time::Duration::from_secs(5),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.task, peek_core::Task::ExplainError);
        server.await.unwrap();
    }
}
