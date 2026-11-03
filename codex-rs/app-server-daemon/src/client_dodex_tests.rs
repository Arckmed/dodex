//! Exercise the actual daemon websocket handshake without a model or user state.
use super::*;
use serde_json::json;

#[tokio::test]
async fn capabilities_accept_matching_protocol_and_reject_incompatible_servers() {
    for scenario in ["matching", "official", "different", "malformed", "closed"] {
        let home = tempfile::TempDir::new().unwrap();
        let directory = home.path().join("control");
        codex_uds::prepare_private_socket_directory(&directory).await.unwrap();
        let path = directory.join("server.sock");
        let mut listener = codex_uds::UnixListener::bind(&path).await.unwrap();
        let server_home = home.path().to_path_buf();
        let server = tokio::spawn(async move {
            let mut socket = tokio_tungstenite::accept_async(listener.accept().await.unwrap()).await.unwrap();
            let JSONRPCMessage::Request(initialize) = read_message(&mut socket).await.unwrap() else { panic!("initialize expected") };
            let response = json!({"id": initialize.id, "result": {
                "userAgent": "daemon-test/1.0", "codexHome": server_home,
                "platformFamily": std::env::consts::FAMILY, "platformOs": std::env::consts::OS,
            }});
            socket.send(Message::Text(response.to_string().into())).await.unwrap();
            assert!(matches!(read_message(&mut socket).await.unwrap(), JSONRPCMessage::Notification(_)));
            let JSONRPCMessage::Request(request) = read_message(&mut socket).await.unwrap() else { panic!("capabilities expected") };
            assert_eq!(request.method, "dodex/capabilities");
            let response = match scenario {
                "matching" => json!({"id": request.id, "result": {"protocolVersion": codex_app_server_protocol::DODEX_PROTOCOL_VERSION}}),
                "different" => json!({"id": request.id, "result": {"protocolVersion": 999}}),
                "malformed" => json!({"id": request.id, "result": {}}),
                "official" => json!({"id": request.id, "error": {"code": -32600, "message": "unknown method"}}),
                "closed" => return,
                _ => unreachable!(),
            };
            socket.send(Message::Text(response.to_string().into())).await.unwrap();
            // Complete a successful client's websocket close handshake.
            if scenario == "matching" { let _ = socket.next().await; }
        });
        let result = verify_dodex_protocol(&path).await;
        assert_eq!(result.is_ok(), scenario == "matching", "{scenario}: {result:?}");
        server.await.unwrap();
    }
}
