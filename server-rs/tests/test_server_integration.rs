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
    let valid_layout = default_design();
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
async fn test_websocket_stream_and_patch() {
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
                    active_page_id: DEFAULT_PAGE_ID.to_string(),
                    nav_stack: Vec::new(),
                    last_interaction: std::time::Instant::now(),
                    last_click: None,
                    ws_sender: Some(tx),
                    frame_seq: 0,
                    // This harness acts as a current v2 device that has
                    // completed the DeviceHello handshake with DEFLATE.
                    firmware_version: Some("2.0.0".to_string()),
                    device_proto_version: Some(2),
                    capabilities: yello_server::CAPS_CURRENT,
                    supports_compression: true,
                    hello_received: true,
                    push_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                    last_tick: std::time::Instant::now(),
                    ota: yello_server::OtaSessionState::default(),
                    ota_auto_attempted: false,
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

    // Client receives ONE FramePatch message for the initial full push
    // (protocol v2: bundled + compressed, not per-zone messages).
    let mut patch: Option<(yello_server::FramePatch, bool)> = None;
    let mut patch_wire_len = 0usize;
    while let Some(Ok(msg)) = client_rx.next().await {
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg {
            if bytes.len() >= yello_server::FRAME_PATCH_HEADER_BYTES
                && bytes[0] == PROTO_MAGIC
                && bytes[1] == PROTO_VERSION
                && bytes[2] == MsgType::FramePatch as u8
            {
                patch_wire_len = bytes.len();
                patch = Some(yello_server::FramePatch::unpack(&bytes).expect("patch must decode"));
                break;
            }
        }
    }

    let (patch, _compressed) = patch.expect("a FramePatch must arrive");
    assert!(patch.full_frame, "initial push is a full frame");
    assert_eq!(patch.frame_seq, 0);
    let total_pixels: usize = patch.zones.iter().map(|z| z.w as usize * z.h as usize).sum();
    assert_eq!(total_pixels, 240 * 320, "full frame covers the whole panel");
    // The wire packet must be far smaller than the old raw per-zone total
    // (240*320*2 pixel bytes + 160 zone headers = 156,160 bytes).
    assert!(
        patch_wire_len < 156_160 / 2,
        "patch wire len {patch_wire_len} should show the compression win"
    );

    // Session accounting used the actual packet length and advanced seq.
    {
        let devices = state.devices.read().await;
        let s = devices.get("test-device").unwrap();
        assert_eq!(s.total_wire_bytes, patch_wire_len as u64);
        assert_eq!(s.frame_seq, 1);
        assert_eq!(s.frames_sent, 1);
    }
}

#[tokio::test]
async fn test_push_frame_raw_for_device_without_deflate_capability() {
    // A device whose DeviceHello did NOT set CAP_DEFLATE (here: an old
    // v2 build that only identified its firmware version) must receive
    // the raw bundle — compression is negotiated, never assumed.
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio_raw.json"),
        PathBuf::from("/tmp/test_secrets_raw.json"),
        static_dir(),
    ));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<tokio_tungstenite::tungstenite::Message>();
    {
        let mut devices = state.devices.write().await;
        devices.insert(
            "raw-device".to_string(),
            DeviceSession {
                device_id: "raw-device".to_string(),
                remote: "127.0.0.1:12346".to_string(),
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
                active_page_id: DEFAULT_PAGE_ID.to_string(),
                nav_stack: Vec::new(),
                last_interaction: std::time::Instant::now(),
                    last_click: None,
                ws_sender: Some(tx),
                frame_seq: 0,
                firmware_version: Some("2.0.0".to_string()),
                device_proto_version: Some(2),
                capabilities: yello_server::CAP_RAW_BUNDLE | yello_server::CAP_FRAME_PATCH,
                supports_compression: false,
                hello_received: true,
                push_lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                last_tick: std::time::Instant::now(),
                    ota: yello_server::OtaSessionState::default(),
                    ota_auto_attempted: false,
            },
        );
    }

    push_frame(&state, "raw-device", true).await.unwrap();

    // First binary message could be a DeviceConfig if orientation
    // differed; the FramePatch is the one that matters here.
    let mut patch_bytes: Option<Vec<u8>> = None;
    while let Ok(Some(msg)) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = msg {
            if bytes.len() >= 4 && bytes[2] == MsgType::FramePatch as u8 {
                patch_bytes = Some(bytes.to_vec());
                break;
            }
        }
    }
    let bytes = patch_bytes.expect("a FramePatch must arrive");
    assert_eq!(bytes[3] & PATCH_FLAG_COMPRESSED, 0, "no-DEFLATE device must get a raw patch");
    assert!(bytes.len() > 100_000, "raw full frame should be ~156 KB, got {}", bytes.len());
    let (_patch, was_compressed) = FramePatch::unpack(&bytes).expect("raw patch must decode");
    assert!(!was_compressed);

    // The session's version info is visible through the Studio API.
    let req = Request::builder().method(Method::GET).uri("/api/studio").body(Body::empty()).unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    let body = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let dev = &val["devices"][0];
    assert_eq!(dev["firmwareVersion"], "2.0.0");
    assert_eq!(dev["protoVersion"], 2);
    assert_eq!(dev["supportsCompression"], false);
    assert_eq!(dev["helloReceived"], true);
}

#[tokio::test]
async fn test_write_atomic_replaces_content() {
    let dir = std::env::temp_dir().join(format!("yello-atomic-{}", std::process::id()));
    let path = dir.join("studio.json");
    write_atomic(&path, b"{\"v\":1}").await.unwrap();
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "{\"v\":1}");
    write_atomic(&path, b"{\"v\":2}").await.unwrap();
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "{\"v\":2}");
    // No temp file is left behind after a successful save.
    assert!(!dir.join("studio.json.tmp").exists());
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn test_live_touch_endpoint() {
    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio_live.json"),
        PathBuf::from("/tmp/test_secrets_live.json"),
        static_dir(),
    ));

    // Tap on the empty background of the default layout: accepted, recorded
    // in the design's live preview state, and no navigation is reported.
    let body = json!({ "layout": default_design(), "x": 4, "y": 4, "touchType": "down" });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/live-touch?design=default")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["status"], "ok");
    assert!(val.get("navigateTo").is_none());
    {
        let live_states = state.live_states.read().await;
        let ls = live_states.get("default").expect("live state recorded");
        assert_eq!(ls.dots.len(), 1);
    }

    // Tap on a full-screen design-link: the Studio is told where to go.
    let mut layout = default_design();
    layout["pages"][0]["elements"] = json!([{
        "id": "link1", "type": "design-link", "name": "Open", "visible": true, "locked": false,
        "frame": { "x": 0, "y": 0, "w": 240, "h": 320 },
        "style": { "color": "#6b9cff", "fontSize": 16, "radius": 8 },
        "props": { "label": "Open screen", "targetDesignId": "default" }
    }]);
    let body = json!({ "layout": layout, "x": 120, "y": 160, "touchType": "down" });
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/live-touch?design=default")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["navigateTo"], "default");

    // A body without a layout is a 400, not a panic.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/live-touch?design=default")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "x": 1, "y": 1 }).to_string()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
