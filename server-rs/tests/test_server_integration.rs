use futures_util::{SinkExt, StreamExt};
use hyper::{Body, Method, Request, StatusCode};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use yello_server::server::{handle_http_request, AppState};
use yello_server::*;

fn static_dir() -> PathBuf {
    if PathBuf::from("server-rs/static").exists() {
        PathBuf::from("server-rs/static")
    } else if PathBuf::from("../server-rs/static").exists() {
        PathBuf::from("../server-rs/static")
    } else {
        PathBuf::from("static")
    }
}

#[tokio::test]
async fn test_api_status_and_studio() {
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio.json"),
        PathBuf::from("/tmp/test_secrets.json"),
        static_dir(),
    ));

    // Test GET /api/status
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/status")
        .body(Body::empty())
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["deviceConnected"], false);
    assert_eq!(val["connectedScreens"], 0);
    assert_eq!(val["knownScreens"], 0);
    assert_eq!(val["homeAssistantConfigured"], false);

    // Test GET /api/studio
    let req = Request::builder()
        .method(Method::GET)
        .uri("/api/studio")
        .body(Body::empty())
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["designs"].as_array().unwrap().len(), 1);
    assert_eq!(val["screens"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_api_layout_validation_and_preview() {
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio.json"),
        PathBuf::from("/tmp/test_secrets.json"),
        static_dir(),
    ));

    // Test POST /api/layout/validate
    let valid_layout = default_layout();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/layout/validate")
        .body(Body::from(serde_json::to_vec(&valid_layout).unwrap()))
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Test POST /api/preview
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/preview")
        .body(Body::from(serde_json::to_vec(&valid_layout).unwrap()))
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("Content-Type").unwrap(), "image/png");
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    assert!(!bytes.is_empty());
    // PNG signature: 0x89, 'P', 'N', 'G'
    assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
}

#[tokio::test]
async fn test_api_design_management() {
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio.json"),
        PathBuf::from("/tmp/test_secrets.json"),
        static_dir(),
    ));

    // Test POST /api/designs
    let new_design_req = json!({
        "name": "Custom Dashboard",
        "orientation": "landscape"
    });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/designs")
        .body(Body::from(serde_json::to_vec(&new_design_req).unwrap()))
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let design_id = val["id"].as_str().unwrap();
    assert_eq!(val["name"], "Custom Dashboard");
    assert_eq!(val["layout"]["orientation"], "landscape");

    // Test GET /api/designs/{design_id}/preview
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/designs/{}/preview", design_id))
        .body(Body::empty())
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("Content-Type").unwrap(), "image/png");
}

#[tokio::test]
async fn test_static_index_serving() {
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio.json"),
        PathBuf::from("/tmp/test_secrets.json"),
        static_dir(),
    ));

    let req = Request::builder()
        .method(Method::GET)
        .uri("/")
        .body(Body::empty())
        .unwrap();

    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("Content-Type").unwrap(), "text/html");
}

#[tokio::test]
async fn test_websocket_stream_and_zones() {
    use tokio::io::duplex;

    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio.json"),
        PathBuf::from("/tmp/test_secrets.json"),
        static_dir(),
    ));

    let (client_io, server_io) = duplex(64 * 1024);

    // Run server WS handler on server_io
    let st = state.clone();
    tokio::spawn(async move {
        let ws_stream = tokio_tungstenite::accept_async(server_io).await.unwrap();
        let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<tokio_tungstenite::tungstenite::Message>();

        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let _ = ws_sink.send(msg).await;
            }
        });

        {
            let mut devices = st.devices.write().await;
            devices.insert(
                "test-device".to_string(),
                DeviceSession {
                    device_id: "test-device".to_string(),
                    remote: "127.0.0.1:12345".to_string(),
                    last_frame: None,
                    last_full_push: std::time::Instant::now() - std::time::Duration::from_secs(400),
                    pressed: std::collections::HashSet::new(),
                    dots: Vec::new(),
                    connected_at: std::time::Instant::now(),
                    connected_since: chrono::Utc::now().to_rfc3339(),
                    stream_samples: std::collections::VecDeque::new(),
                    total_pixels: 0,
                    total_wire_bytes: 0,
                    total_zone_messages: 0,
                    frames_sent: 0,
                    last_push_ms: 0.0,
                    last_frame_at: chrono::Utc::now().to_rfc3339(),
                    orientation: "portrait".to_string(),
                    last_ping_at: chrono::Utc::now().to_rfc3339(),
                    wifi_rssi_dbm: None,
                    uptime_ms: 0,
                    active_design_id: DEFAULT_DESIGN_ID.to_string(),
                    ws_sender: Some(tx),
                    push_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                    last_tick: std::time::Instant::now(),
                },
            );
        }

        // Push initial frame
        let _ = push_frame(&st, "test-device", true).await;

        while let Some(Ok(_msg)) = ws_stream_rx.next().await {}
    });

    // Client connects
    let client_ws = tokio_tungstenite::client_async("ws://localhost/ws", client_io).await.unwrap().0;
    let (_client_tx, mut client_rx) = client_ws.split();

    // Client receives zone update messages
    let mut received_zones = 0;
    while let Some(Ok(msg)) = client_rx.next().await {
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg {
            if bytes.len() >= 16 && bytes[0] == PROTO_MAGIC && bytes[1] == PROTO_VERSION && bytes[2] == MsgType::ZoneUpdate as u8 {
                received_zones += 1;
                if received_zones >= 5 {
                    break;
                }
            }
        }
    }

    assert!(received_zones >= 5);
}
