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
