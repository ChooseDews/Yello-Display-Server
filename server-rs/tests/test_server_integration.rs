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
async fn test_button_and_touch_zone_ha_actions_call_mock_ha() {
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Mock Home Assistant: records raw requests, answers 200 with [].
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ha_addr = listener.local_addr().unwrap();
    let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
    let captured_task = captured.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { break };
            let cap = captured_task.clone();
            tokio::spawn(async move {
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut header_end: Option<usize> = None;
                let mut content_length = 0usize;
                loop {
                    if let Some(end) = header_end {
                        if buf.len() >= end + content_length {
                            break;
                        }
                    }
                    match socket.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if header_end.is_none() {
                                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    header_end = Some(pos + 4);
                                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                                    for line in head.lines() {
                                        if let Some(v) = line.strip_prefix("content-length:") {
                                            content_length = v.trim().parse().unwrap_or(0);
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => return,
                    }
                }
                if !buf.is_empty() {
                    cap.lock().unwrap().push(String::from_utf8_lossy(&buf).to_string());
                }
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n[]")
                    .await;
            });
        }
    });

    let state = Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio_blocks.json"),
        PathBuf::from("/tmp/test_secrets_blocks.json"),
        static_dir(),
    ));
    {
        let mut client = state.ha_client.write().await;
        *client = yello_server::HaClient::new(&format!("http://{ha_addr}"), "test-token");
    }

    let touch = |state: Arc<AppState>, layout: serde_json::Value| async move {
        let body = json!({ "layout": layout, "x": 120, "y": 160, "touchType": "down" });
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/live-touch?design=default")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = handle_http_request(req, state).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    };

    // A button whose actionKind is "ha" fires the mapped service:
    // scene preset -> scene.turn_on.
    let mut layout = default_design();
    layout["pages"][0]["elements"] = json!([{
        "id": "btn1", "type": "button", "name": "Movie", "visible": true, "locked": false,
        "frame": { "x": 0, "y": 0, "w": 240, "h": 320 },
        "style": { "color": "#2255aa", "fontSize": 16, "radius": 8 },
        "props": { "label": "Movie", "actionId": "", "actionKind": "ha",
                   "haEntityId": "scene.movie_night", "haService": "scene" }
    }]);
    touch(state.clone(), layout).await;

    // A touch-zone with turn_on -> homeassistant.turn_on.
    let mut layout = default_design();
    layout["pages"][0]["elements"] = json!([{
        "id": "zone1", "type": "touch-zone", "name": "Zone", "visible": true, "locked": false,
        "frame": { "x": 0, "y": 0, "w": 240, "h": 320 },
        "style": {},
        "props": { "actionId": "", "actionKind": "ha",
                   "haEntityId": "light.kitchen", "haService": "turn_on" }
    }]);
    touch(state.clone(), layout).await;

    // The service calls are spawned; wait for both to land.
    let mut requests = Vec::new();
    for _ in 0..50 {
        {
            let cap = captured.lock().unwrap();
            requests = cap.clone();
        }
        if requests.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(requests.len(), 2, "expected two HA service calls, got: {requests:?}");
    assert!(
        requests.iter().any(|r| r.starts_with("POST /api/services/scene/turn_on") && r.contains("scene.movie_night")),
        "scene call missing: {requests:?}"
    );
    assert!(
        requests.iter().any(|r| r.starts_with("POST /api/services/homeassistant/turn_on") && r.contains("light.kitchen")),
        "turn_on call missing: {requests:?}"
    );
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

// ---------------------------------------------------------------------------
// Script ha() fetch coverage: entities referenced only from script
// code must enter the HA fetch specs, and ha_count domains must be
// maintained by the slow full-states sweep.
// ---------------------------------------------------------------------------

#[test]
fn ha_fetch_specs_include_script_literals() {
    let layout = json!({
        "version": 3,
        "pages": [{
            "id": "main",
            "elements": [
                { "id": "t1", "type": "ha-toggle",
                  "props": { "entityId": "light.kitchen", "refreshInterval": 10 } },
                { "id": "g1", "type": "gauge",
                  "props": { "sourceKind": "ha", "entityId": "sensor.power", "refreshInterval": 7 } },
                { "id": "g2", "type": "gauge",
                  "props": { "sourceKind": "external", "entityId": "sensor.ignored" } },
                { "id": "s1", "type": "script",
                  "props": { "refreshInterval": 9, "code": "text(ha(\"binary_sensor.front_door\"))" } },
                { "id": "s2", "type": "script",
                  "props": { "code": "a = ha('sensor.attic_temp')\nb = ha(some_var)\nc = ha(\"not an entity\")\nn = ha_count(\"light\")\nm = ha_count('switch', \"on\")\ntext(str(ha(\"light.kitchen\")))" } }
            ]
        }],
        "sharedElements": []
    });
    let specs = yello_server::server::collect_ha_fetch_specs(&[&layout]);
    // Element-declared entities keep their own intervals.
    assert_eq!(specs.entities.get("sensor.power"), Some(&7));
    // Script-only entities are fetched at the script's interval
    // (explicit 9 on s1, the default 5 on s2).
    assert_eq!(specs.entities.get("binary_sensor.front_door"), Some(&9));
    assert_eq!(specs.entities.get("sensor.attic_temp"), Some(&5));
    // Shared entity: the most frequent refresh wins (s2's default 5
    // beats the toggle's 10).
    assert_eq!(specs.entities.get("light.kitchen"), Some(&5));
    // Non-HA gauges, dynamic ha(variable), and malformed literals
    // contribute nothing.
    assert!(!specs.entities.contains_key("sensor.ignored"));
    assert_eq!(specs.entities.len(), 4);
    // ha_count domains from both quote styles.
    assert!(specs.count_domains.contains("light"));
    assert!(specs.count_domains.contains("switch"));
    assert_eq!(specs.count_domains.len(), 2);
}

#[tokio::test]
async fn script_ha_entities_are_fetched_and_counted() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Mock HA: a per-entity endpoint plus the full state list.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ha_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match socket.read(&mut chunk).await {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(_) => return,
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let body = if path == "/api/states" {
                    json!([
                        {"entity_id": "binary_sensor.front_door", "state": "on", "attributes": {}},
                        {"entity_id": "light.kitchen", "state": "on", "attributes": {}},
                        {"entity_id": "light.desk", "state": "off", "attributes": {}},
                        {"entity_id": "switch.fan", "state": "on", "attributes": {}}
                    ])
                    .to_string()
                } else if let Some(eid) = path.strip_prefix("/api/states/") {
                    let st = if eid == "light.desk" { "off" } else { "on" };
                    json!({"entity_id": eid, "state": st, "attributes": {}}).to_string()
                } else {
                    "[]".to_string()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    let studio_path = PathBuf::from("/tmp/test_studio_script_ha.json");
    let secrets_path = PathBuf::from("/tmp/test_secrets_script_ha.json");
    let _ = std::fs::remove_file(&studio_path);
    let _ = std::fs::remove_file(&secrets_path);
    let state = Arc::new(AppState::new(studio_path, secrets_path, static_dir()));
    {
        let mut client = state.ha_client.write().await;
        *client = yello_server::HaClient::new(&format!("http://{ha_addr}"), "test-token");
    }
    // The design's ONLY HA references live in script code: one ha()
    // literal and one ha_count() domain. No element declares them.
    let layout = json!({
        "version": 3,
        "name": "Script HA",
        "pages": [{
            "id": "main",
            "elements": [
                { "id": "s1", "type": "script",
                  "props": { "refreshInterval": 1, "code": "text(ha(\"binary_sensor.front_door\"))" } },
                { "id": "s2", "type": "script",
                  "props": { "refreshInterval": 1, "code": "text(str(ha_count(\"light\")))" } }
            ]
        }],
        "sharedElements": []
    });
    {
        let mut studio = state.studio.write().await;
        studio.designs.insert(
            "script-ha".to_string(),
            yello_server::server::Design {
                id: "script-ha".to_string(),
                name: "Script HA".to_string(),
                revision: 1,
                layout,
            },
        );
    }
    yello_server::server::start_background_tasks(state.clone());

    // Wait for the poll loop to fetch the script-only entity and for
    // the ha_count sweep to land the light domain's membership.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        {
            let vals = state.ha_values.read().await;
            if vals.contains_key("binary_sensor.front_door") && vals.contains_key("light.desk") {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "HA loop never fetched script-referenced entities"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let ha_vals = state.ha_values.read().await.clone();
    // Domain filtering: switch.fan exists in HA but no script counts
    // the switch domain and nothing names the entity.
    assert!(!ha_vals.contains_key("switch.fan"));

    let ctx = yello_server::server::build_render_context(
        &std::collections::HashMap::new(),
        "script-ha",
        &ha_vals,
        &std::collections::HashSet::new(),
        &[],
    );
    // ha() returns the mocked state, not "unavailable".
    let out = yello_server::script_runtime::execute_script(
        "text(ha(\"binary_sensor.front_door\"))",
        200,
        60,
        &ctx,
    )
    .unwrap();
    assert_eq!(out.text.as_deref(), Some("on"));
    // ha_count sees the whole light domain, not just placed elements.
    let out = yello_server::script_runtime::execute_script(
        "text(str(ha_count(\"light\")))",
        200,
        60,
        &ctx,
    )
    .unwrap();
    assert_eq!(out.text.as_deref(), Some("2"));
    let out = yello_server::script_runtime::execute_script(
        "text(str(ha_count(\"light\", \"on\")))",
        200,
        60,
        &ctx,
    )
    .unwrap();
    assert_eq!(out.text.as_deref(), Some("1"));
}
